# Varredura de qualidade: plugin-host

Escopo: `lm-plugin-host` (helper), `lm-plugin-proxy` (lado do core), `lm-doc`, `lm-state`, `lm-base`, `lm-time`.
Auditoria somente leitura: nenhum build, nenhum teste, nenhuma edição de código. Lidos por inteiro: todo o `lm-plugin-proxy`, todo o `lm-plugin-host` (main, backend, dual, expressions, clap, vst3, lv2, ladspa), todo o `lm-base`, todo o `lm-time`, `lm-state` (json, value, id, resource, frozen, schema na parte de validação e upgrade). Em `lm-doc`: `document.rs` (quase todo), `migrate.rs`, `tuning.rs`, `diff.rs` (parte), e `edit.rs`, `index.rs`, `rules.rs`, `refs.rs`, `ops.rs`, `schemas.rs` por busca de padrões e leitura dos trechos de laço, recursão e conversão. Os trechos não lidos linha a linha estão indicados na seção de limites.

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO | 1 |
| ALTO | 7 |
| MÉDIO | 12 |
| BAIXO | 14 |
| **Total** | **34** |

Os 5 piores:

1. **CRÍTICO** `lm-plugin-proxy/src/hosted.rs:374-375` (latência reportada pelo helper) chega a `lm-render/src/program.rs:1248-1253`, onde `vec![0.0; channels * length]` aloca na montagem. Um helper que reporte latência perto de 4·10⁹ quadros faz o core tentar alocar dezenas de GB e abortar, com o trabalho não salvo perdido.
2. **ALTO** `lm-plugin-host/src/main.rs:222` e `:400`: a thread de áudio e a thread de controle disputam o mesmo `Mutex` do plugin. Abrir a janela do plugin (`window()`, `:400`) segura o lock por segundos; o bloco perde o prazo 16 vezes seguidas e o core mata um plugin saudável.
3. **ALTO** `lm-plugin-proxy/src/proxy.rs:217-224` e `:237-244`: as saídas e os valores de parâmetro vindos do helper entram sem checagem de finitude. Um NaN/inf no mix se propaga por realimentações (delay, reverb) e permanece até reiniciar.
4. **ALTO** `lm-doc/src/document.rs:357` e `:430`: `max_id(..) + 1` com ids lidos de arquivo. Um id `t:18446744073709551615` faz o contador dar a volta para 0 em release (ids novos colidem) e panic em debug.
5. **ALTO** `lm-plugin-proxy/src/hosted.rs:263` (`write_message` bloqueante sob o mutex do stdin) e `:523` (`Hosted::drop` envia `quit` por essa mesma escrita): com o helper parado de ler e o pipe cheio, a thread que chama (inclusive a principal, ao fechar um plugin) trava sem prazo.

---

## lm-plugin-proxy

### CRÍTICO

**C-1. Latência do helper sem limite até a alocação do delay line.**
- `lm-plugin-proxy/src/hosted.rs:374-375`: `ready.latency` é só limitado a `u32::MAX`. `proxy.rs:226-231` aceita o `LATENCY` de memória compartilhada sem limite e repassa com `cx.host.request(LatencyChanged)`.
- `lm-compile/src/core.rs:2099-2102`: a latência entra na soma de atrasos; `f.delay = u32::try_from(...).unwrap_or(u32::MAX)`.
- `lm-render/src/program.rs:1241-1253`: `length = f.feed.delay as usize` e `memory: vec![0.0; channels * length]`.
- Efeito: um plugin (malicioso ou com bug) que reporte latência de bilhões de quadros faz a montagem alocar GBs e abortar o processo do core.
- Correção: no core, limitar a latência reportada (por exemplo, 1 s de quadros, ou `max_block * N`), recusar com notice `plugin_host.latency` acima do limite; em `program.rs`, limitar o comprimento do delay e usar alocação falível.

### ALTO

**A-1. Saídas e parâmetros do helper sem checagem de finitude.**
- `proxy.rs:217-224`: `get(words, l.output(ch), ..)` copia f32 bruto (NaN/inf) para o buffer do mix. O render só filtra não-finitos na medição (`lm-render/src/renderer.rs:517`), não no áudio: um NaN em um delay ou reverb com realimentação fica para sempre.
- `proxy.rs:237-244`: `ParamChanged { value }` sem checagem. Em `lm-session/src/external.rs:312-322` o valor vai a `lm-doc/src/edit.rs:651` (`Value::real(v)?`), que falha; o lote inteiro de parâmetros do bloco é descartado e o erro se repete a cada bloco (spam de notices).
- Correção: em `proxy.rs`, trocar valor não finito por 0.0 (e limitar a faixa) após `get()`; rejeitar `value` não finito em `ParamChanged`.

**A-2. Modo `Deadline::Wait` nunca desiste e não é cancelável.**
- `lm-plugin-proxy/src/proxy.rs:203-213`: com `Wait`, o laço só sai com `DONE` ou com helper morto; `HANG_BLOCKS` nunca é contado.
- `lm-session/src/export.rs:178`: a exportação usa `Wait`. Um plugin travado (helper vivo) trava a exportação para sempre, e o cancelamento só é checado entre blocos.
- Correção: prazo generoso por bloco no export (por exemplo, 10 s) que vira erro `plugin_host.hung`, ou um flag de cancelamento lido dentro do laço.

**A-3. Escrita bloqueante no stdin do helper, sem prazo e sob mutex.**
- `lm-plugin-proxy/src/hosted.rs:259-266` (`send`): `write_message` faz `write_all` bloqueante segurando `stdin` (mutex). O prazo de `request()` cobre só a espera da resposta (`:427-428`), não a escrita.
- `hosted.rs:515-528` (`Hosted::drop`): envia `quit` pela mesma escrita. Um helper parado com um `restore` grande (até 16 MB em hex) no pipe trava a thread que solta o último `Arc`. Se for a thread principal, a interface congela.
- Correção: escrever em thread própria ou com timeout; em `drop`, pular o `quit` quando o helper já estiver morto ou usar `kill` direto após um prazo.

**A-4. Fila de mensagens do helper sem limite.**
- `hosted.rs:198` e `:339`: `std::sync::mpsc::channel()` (ilimitado). O leitor (`:217`) empurra toda mensagem de uma instância; elas só são drenadas em `request()` (`:426`). Um helper que emita mensagens não solicitadas cresce a memória do core sem limite.
- Correção: `sync_channel` com capacidade fixa e política de descarte com contagem, ou drenagem periódica pelo supervisor.

**A-5. Mensagens da varredura sem limite.**
- `lm-plugin-proxy/src/scan.rs:155-162`: a thread leitora acumula `Vec<Value>` de todo o stdout do helper durante até 60 s (`:223`, `:252`) ou `probe_timeout`. Um helper que escreva sem parar consome memória do core até o prazo.
- Correção: limite de número e de bytes de mensagens; ao passar, matar o helper e marcar como bloqueado.

**A-6. A thread de áudio do helper disputa o lock com o controle; plugin saudável é morto.**
- `lm-plugin-host/src/main.rs:222`: `audio_loop` faz `plugin.lock()` a cada bloco.
- `main.rs:366`, `:381`, `:400`, `:471`: `save()`, `restore()`, `window()` e `idle()` seguram o mesmo lock na thread de controle. Abrir a janela (CLAP `gui_open`, VST3 `editor_open`) leva vários segundos.
- Efeito: o bloco não é processado, `DONE` não sobe, o proxy conta o bloco perdido; em 16 seguidos (`proxy.rs:19`, `:114`) marca `hung` e o supervisor mata o helper.
- Correção: a thread de áudio deve ter posse exclusiva do plugin; comandos de controle vão por mensagem para a thread de áudio (ou o trabalho de janela/estado roda com o plugin fora do caminho de áudio).

**A-7. Soma `+ 1` em ids vindos do arquivo de projeto.**
- `lm-doc/src/migrate.rs:17-31` (`max_id`): lê todos os números de id. `NodeId::parse` aceita até `u64::MAX` (`lm-state/src/id.rs:47`, `parse_canonical_u64`).
- `lm-doc/src/document.rs:357`: `max_id(&root) + 1` e `:430`: `max_id.max(max_note) + 1`. Com `u64::MAX`, a soma dá a volta para 0 em release (sem `overflow-checks` no `[profile.release]`) e panic em debug. O contador volta a 0 e novos ids podem colidir com existentes.
- Correção: `checked_add` e erro `doc.id.exhausted` (como já é feito em `document.rs:658`), ou recusar ids/notas acima de `MAX_SAFE_INT` ao abrir.

### MÉDIO

**M-1. Leitor do controle morre com linha ruim, helper continua vivo.**
- `hosted.rs:201-221`: ao primeiro erro de `read_message` (JSON ruim, linha > 16 MB, UTF-8 inválido) o leitor sai e limpa as rotas. O supervisor (`:395-410`) só observa a saída do processo: a instância fica "meio morta" (áudio segue, janela e estado não respondem) e nunca é morta.
- Correção: ao sair o leitor, marcar `dead` e matar o filho.

**M-2. `try_wait` com erro abandona o helper.**
- `lm-plugin-proxy/src/scan.rs:165`: `child.try_wait().map_err(..)?` retorna sem `kill`/`wait` e sem `join` da leitora. Resultado: helper órfão rodando, thread e fd presos.
- Correção: matar e recolher o filho em todos os caminhos de saída.

**M-3. Helpers órfãos se o core morrer por panic/abort.**
- `hosted.rs:181-193` (`spawn`): sem `PR_SET_PDEATHSIG`. O helper só sai quando o stdin fecha; um helper preso em código de plugin ou em janela não lê o stdin e segue vivo.
- Correção: `pre_exec` com `prctl(PR_SET_PDEATHSIG, SIGKILL)` no Linux; no helper, checar o pid do pai.

**M-4. Eventos e mudanças descartados em silêncio, inclusive note-off.**
- `lm-plugin-host/src/clap.rs:135-139` e `vst3.rs:121-125` (`push`): descarte silencioso quando a capacidade acaba. Um note-off perdido deixa nota presa.
- `clap.rs:358-364`, `vst3.rs:330-337`, `dual.rs:199-201`: mudanças além da capacidade (`out.len() < out.capacity()`) somem.
- `lm-plugin-proxy/src/proxy.rs:157-164`: `cx.events.iter().take(l.max_events)` descarta eventos excedentes do bloco (note-off incluso).
- Correção: nunca descartar note-off (reservar espaço ou dividir o bloco); contar e reportar descartes.

**M-5. Restauração pela metade no dual mono.**
- `lm-plugin-host/src/dual.rs:170-173`: `self.left.restore(chunk)?; self.right.restore(chunk)`. Se a direita falhar, a esquerda já mudou.
- Correção: restaurar a direita primeiro em cópia ou desfazer a esquerda no erro.

**M-6. Chaves de parâmetro de plugin internadas sem limite.**
- `lm-plugin-proxy/src/external.rs:49` (`plugin_type(&id)`) e `:55` (`intern(&p.key)`): textos vindos do helper/plugin vão ao `lm_base::intern` (`lm-base/src/symbol.rs:23-32`), que não tem teto. O próprio `symbol.rs:14-15` diz que texto externo deve passar por interner com teto antes. Cada chave nova de cada varredura fica para sempre.
- Correção: interner com teto (como `protocol.rs:intern`) ou guardar `String` no `ParamSpec`.

**M-7. Lista de bloqueados permanente por timeout de máquina lenta.**
- `lm-plugin-proxy/src/scan.rs:260-262` e `:275-276`: qualquer timeout (`probe_timeout`, 60 s para listagem) vira bloqueio definitivo, guardado em `lm-session/src/external.rs:209-224` sem caminho para desbloquear.
- Correção: bloquear só por crash/sinal; timeout expira; criar ação "desbloquear".

**M-8. Fila de operações com mutex envenenado: job some em silêncio.**
- `lm-base/src/operation.rs:255-257`: `if let Ok(mut jobs) = ... { push }`. Com o mutex envenenado o job nunca entra, a alça fica `Queued` para sempre e a interface espera sem fim.
- `operation.rs:282-285`: o worker sai silenciosamente se o `wait` envenenar; as operações seguintes também não rodam.
- Correção: propagar o erro (`Phase::Failed`) em vez de descartar.

**M-9. `Workers::drop` roda trabalho enfileirado e bloqueia a thread principal.**
- `lm-base/src/operation.rs:295-301` e `worker_loop` (`:271-293`): o drop fecha a fila e junta as threads; `worker_loop` só sai depois de esvaziar a fila. Trabalho enfileirado de alças ainda vivas roda inteiro na thread que solta o pool.
- Correção: no drop, cancelar os jobs pendentes (sinal de cancelamento por job) antes de juntar.

**M-10. Escritor JSON não impõe a profundidade que o leitor impõe.**
- `lm-state/src/json/read.rs:16` (`MAX_DEPTH = 128`) e `json/write.rs:70-150`: o escritor não verifica profundidade. Uma árvore em memória com mais de 128 níveis salva sem erro e depois não abre (`state.json.too_deep`): perda de dados no reabrir. Não alcançável hoje pelos esquemas, mas quebra o contrato de ida e volta.
- Correção: checar profundidade no escritor (`state.json.too_deep`) antes de escrever.

**M-11. Contador do projeto acima de 2⁵³ impede salvar.**
- `lm-doc/src/document.rs:441` (`Value::int(needed as i64)`) e `:366` (`up.next_id as i64`): com ids do arquivo acima de `MAX_SAFE_INT` (2⁵³−1), o contador vira inteiro não representável. O escritor recusa (`lm-state/src/json/write.rs:76`) e o projeto não salva mais. `as i64` com valores acima de `i64::MAX` vira negativo.
- Correção: validar ids/notas ao abrir (mesmo limite do item A-7) e converter com `try_from`, como em `document.rs:658`.

**M-12. Intervalo de escala em razão aceita inf/NaN.**
- `lm-doc/src/tuning.rs:35-38`: o ramo de centavos checa `is_finite`, o de razão (`(a > 0.0 && b > 0.0).then(|| 1200.0 * (a / b).log2())`, linha 38) não. Entrada `1e400/1` ou `inf/1` vira intervalo infinito; `inf/inf` vira NaN. Isso vai para `lm-compile` (`keys.rs`), que calcula frequências.
- Correção: filtrar `is_finite` também no resultado da razão.

### BAIXO

**B-1.** `lm-plugin-host/src/main.rs:288-304` e `:492`: `Slot` não tem `Drop` que pare a thread de áudio. Recarregar um `id` de grupo (`slots.insert(id, ..)`) descarta o slot antigo sem `stop()`; a thread de áudio antiga fica rodando para sempre. Correção: `impl Drop` chamando `stop()`, ou recusar `load` com `id` existente.

**B-2.** `lm-plugin-proxy/src/proxy.rs:184`: `Duration::from_secs_f64(...)` entra em pânico com NaN ou negativo. Hoje o `Deadline::Live` é a constante 0.6 (`proxy.rs:32`), então não dispara. Correção: `clamp` e `is_finite`.

**B-3.** `lm-plugin-host/src/test_plugins.rs` (`crash`, `hang`, `bomb`, `stall`) entra no binário de produção e é escolhido por `format = "test"` (`main.rs:86`, `:104`), que um projeto ou a varredura pode citar. Correção: `cfg(feature = "testing")`.

**B-4.** `lm-base/src/range.rs:49` e `:58`: `f32::clamp(self.min, self.max)` entra em pânico se `min > max`. Hoje é inalcançável: `lm-state/src/schema.rs:916-922` rejeita `min > max` e `external.rs:54` garante `max > min`. Correção: usar `lo`/`hi` como em `to_units` (`range.rs:42`).

**B-5.** `lm-base/src/operation.rs:216`: `.expect(...)` no `spawn` de thread; sistema sem recursos derruba a thread principal. Correção: propagar `Err`.

**B-6.** `lm-time/src/time.rs:31`: `as u64` no resultado de `nanos_to_samples` trunca para valores absurdos (taxa e nanos muito grandes). Inalcançável com os limites atuais. Correção: `try_from` com saturação.

**B-7.** `lm-time/src/kernel.rs:101-102`: `Ramp::new` só tem `debug_assert!(length > 0 && from != to)`. Em release, `length == 0` leva a divisão por zero em `c1 / g`. Hoje o `map.rs` garante ticks estritamente crescentes. Correção: `assert!` ou erro.

**B-8.** `lm-doc/src/document.rs:1027`: `depth > 256` devolve o nó sem upgrade e sem checagem. Inalcançável com o leitor (máximo 128). Correção: erro explícito.

**B-9.** `lm-doc/src/document.rs:665-667`, `:752-753`, `:770-771`: o diário (`journal`) cresce a cada edição e desfazer/refazer até `take_journal`. O `lm-session` esvazia em vários pontos (`session.rs:1909`, `:1944`, `:2776`), então é só um risco se o salvamento automático parar. Correção: limite e aviso.

**B-10.** `lm-plugin-proxy/src/hosted.rs:195-196`: se `child.stdin.take()` falhar, o `child` é descartado sem `kill`/`wait` (helper órfão). Inalcançável com `Stdio::piped()`. Correção: matar e recolher.

**B-11.** `hosted.rs:269-286` (`Process::drop`): dorme até 500 ms na thread que solta o último `Arc`. Se for a thread de áudio, quebra a regra; hoje os processadores aposentados voltam à thread principal (`lm-render/src/renderer.rs:603`). Correção: documentar a exigência e afirmar em teste.

**B-12.** `lm-plugin-host/src/lv2.rs:47`: `default.clamp(min, max)` entra em pânico se `min` vier NaN do arquivo do plugin. Só derruba o helper (que vira "bloqueado" na varredura). Correção: `is_finite` antes.

**B-13.** `lm-plugin-host/src/dual.rs:152-154`: com saídas faltando, o `process` retorna sem preencher as saídas, que ficam com o conteúdo antigo. Correção: zerar as saídas.

**B-14.** `lm-plugin-proxy/src/proxy.rs:258-260`: `reset()` é vazio (só comentário). Pelo comentário, o pânico chega como evento na próxima janela, então é intencional, mas vale um teste.

---

## lm-base

Achados que não estão no bloco do proxy: M-6 (`symbol.rs`, a tabela sem teto), M-8, M-9 (`operation.rs`), B-4, B-5 (`range.rs`, `operation.rs`).

## lm-time

Nenhum achado ALTO ou CRÍTICO. Os limites (`MAX_POSITION = 2⁶²`, `MAX_SAMPLE_RATE = 4·10⁶`, `Ppq ≤ 10⁵`, `Bpm` em 1–1000) são checados na construção (`map.rs:204-239`, `meter.rs:18`, `bpm.rs:32`) e mantêm as contas de `i128`/`I256` dentro da faixa. Os `panic!` de `wide.rs` (overflow de 256 bits, divisão por zero) só disparam se algo fugir desses limites. Ver B-6 e B-7.

## lm-state

Achados: M-10 (`json/write.rs`), M-11 (contador, ver também A-7), e a parte de upgrade/validação de `schema.rs` (sem achados; ver abaixo).

## lm-doc

Achados: A-7, M-11, M-12, B-8, B-9.

---

## O que está bom

- **Helper sem alocação no caminho do bloco do proxy**: `proxy.rs` não aloca nem trava (atômicos, `futex`, espera limitada pelo prazo); a saída de eventos do helper usa `Vec` de capacidade fixa (`layout.rs`, `expressions.rs`).
- **Limites lidos da memória compartilhada são checados**: `Layout::read_header` (`layout.rs:122-141`) valida magia, versão e tamanho contra `words.len()`; o core limita `frames` a `max_block`, o número de eventos a `max_events`, o de saídas de parâmetro a `max_out` (`proxy.rs:137`, `:157`, `:231`); o helper limita `frames`, `n` de eventos e `m` de saídas (`main.rs:187`, `:192`, `:246`, `:252`).
- **`DONE` é só comparado por igualdade**: o core nunca usa o valor de `DONE` como índice ou tamanho, e um bloco atrasado vira silêncio, sem fila (`proxy.rs:133`).
- **Morte, pendura e cancelamento têm caminho definido**: `dead` e `hung` são observados a cada espera; o supervisor mata o helper travado e acorda o `DONE` (`hosted.rs:397-405`).
- **Nada de shm vaza**: `SharedMemory::drop` faz `shm_unlink` e `munmap` (`lm-sys-ipc/src/lib.rs:152-158`); o nome é desfeito logo após o `ready` (`hosted.rs:373`), e os caminhos de erro de `start_in` soltam a memória.
- **Processos são recolhidos**: `Process::drop` faz `try_wait` e mata após 500 ms (`hosted.rs:269-286`); os caminhos de erro de `spawn` matam e recolhem (`:243`, `:223-224`); a varredura mata e recolhe no timeout e no cancelamento (`scan.rs:169-171`).
- **Protocolo**: linha limitada a 16 MB (`protocol.rs:25`), `hex`/`unhex` validados, `intern` do protocolo com teto de 4096 chaves e 160 bytes (`protocol.rs:90-110`), versão do protocolo verificada no `hello` (`hosted.rs:234-240`).
- **Leitor JSON**: profundidade ≤ 128, sem pânico, chave duplicada é erro, inteiros limitados a ±(2⁵³−1), reais finitos, `OrdMap` (chaves ordenadas): a saída é estável byte a byte. Nenhum `HashMap`/`HashSet` nos crates auditados.
- **lm-doc**: só `BTreeMap`/`BTreeSet`; nenhum `unwrap`/`expect` em código de produção (só em testes); histórico limitado (`history_limit` = 1000 passos, `document.rs:720-724`), com índices de desfazer/refazer conferidos (`:710`, `:749`, `:767`); transação tudo-ou-nada (`:640-675`); laço de remoção de referências termina (`edit.rs:818-860`); detecção de ciclo de pasta e de roteamento (`rules.rs:383-470`); upgrades de esquema são funções puras sem pânico (`schemas.rs:714-734`).
- **lm-time**: aritmética exata, sem float nas conversões; `saturate` nas bordas; `div_euclid`/`rem_euclid` no sentido certo; `relocate` (`map.rs:521-537`) testa os casos degenerados antes de dividir.
- **lm-base**: `Main` é `!Send` e prova a thread principal; `Cancel` e `Progress` são atômicos; `catch_unwind` transforma pânico de operação em falha (`operation.rs:244-248`); `Workers` junta as threads no drop.
- **Ids**: `NodeId::parse` aceita só a forma canônica (`id.rs:41-49`); `parse_canonical_u64` rejeita zeros à esquerda e sinal.

## Limites da auditoria

- `lm-doc/src/edit.rs`, `index.rs`, `rules.rs`, `refs.rs`, `ops.rs` e `schemas.rs` foram lidos por busca de padrões (laços, `as`, indexação, `unwrap`) e por leitura dos trechos relevantes, não linha a linha. Os trechos de `schemas.rs` que são só declarações não foram revisados.
- `lm-state/src/value.rs` (partes de acesso e edição de listas) e `schema.rs` (checagem de valores) foram lidos por padrão; o contrato de cada função de edição não foi testado.
- Nenhum build ou teste foi executado, então os achados são de leitura: confirmar C-1 e A-6 com um helper de teste (`test` format com latência e janela) antes de corrigir.

## Resultado da confirmação

Commits: `f56190a83` (proxy), `bdc71e9f8` e `1d1952299` (host), o do documento está listado abaixo. Nada foi executado (só `cargo check`).

| Achado | Veredito | Commit |
|---|---|---|
| C-1 latência do helper sem limite | Confirmado e corrigido: `clamp_latency` (teto de 2^18 quadros) aplicado uma vez em `start_in`, em `Proxy::new` e na leitura do `LATENCY` da memória compartilhada; teste de integração em `tests/host.rs` | f56190a83 |
| A-1 NaN/inf nas saídas e nos parâmetros | Confirmado e corrigido: `scrub` zera não finitos após `get()`; `ParamChanged` não finito é descartado; `Saved` descarta valores que viram inf em f32 | f56190a83 |
| A-2 `Deadline::Wait` sem fim | Confirmado e corrigido: `WAIT_LIMIT` de 10 s por bloco conta como bloco perdido (após 16, o plugin é dado como travado) | f56190a83 |
| A-3 escrita bloqueante no stdin sob mutex | Confirmado e corrigido: thread escritora com fila limitada (`try_send`); `send` e `Hosted::drop` nunca bloqueiam | f56190a83 |
| A-4 fila de mensagens sem limite | Confirmado e corrigido: `sync_channel` de 256, o leitor descarta com `try_send` | f56190a83 |
| A-5 mensagens da varredura sem limite | Confirmado e corrigido: teto de 100 000 mensagens, o helper é morto (`plugin_host.flood`); campos de texto, chaves (160 B), parâmetros (2048) e faixas (±1e9, finitas) limitados em `scanned_from` | f56190a83 |
| A-6 thread de áudio e controle disputam o `Mutex` do plugin | corrigido na rodada de desenho: a thread de áudio só tenta pegar o plugin (sem esperar; ocupado, silêncio) e a principal espera a vez e segura o plugin; ver decisoes-autonomas.md | 177be238e |
| A-7 `max_id + 1` com ids do arquivo | Confirmado e corrigido: `first_free` recusa o arquivo com `doc.id.exhausted` se o contador não couber em 2^53-1 (cobre também M-11) | 362f80836 |
| M-1 leitor do controle morre e o helper segue vivo | Confirmado e corrigido: ao sair o leitor, o helper é morto | f56190a83 |
| M-2 `try_wait` com erro abandona o helper | Confirmado e corrigido: mata, recolhe e junta a leitora | f56190a83 |
| M-3 helper órfão sem `PDEATHSIG` | corrigido na rodada de desenho, sem `PDEATHSIG` (dispara com a morte da thread que criou o filho): o helper sai com o fim do stdin (e `_exit` após 3 s) e vigia o pai por `getppid` | 177be238e |
| M-4 eventos descartados, inclusive note-off | corrigido na rodada de desenho: o proxy divide o bloco em pedaços e nenhum evento se perde (além da reserva de note-off do helper, de antes) | 177be238e |
| M-5 restore pela metade no dual mono | Confirmado e corrigido: a esquerda volta ao estado anterior se a direita recusar; teste | bdc71e9f8 |
| M-6 chaves internadas sem limite | Confirmado e corrigido na fronteira: teto de comprimento e de quantidade por plugin em `scanned_from` (o interner em si é de `lm-base`, não alterado) | f56190a83 |
| M-7 bloqueio permanente por timeout | corrigido na rodada de desenho: queda bloqueia na hora; timeout conta (2 para bloquear), é tentado de novo, e `plugin.unblock` desfaz | 2a1efecbe |
| M-8, M-9 (`lm-base` operation) | Corrigido na rodada de desenho: recuperação do envenenamento; o `Drop` do pool termina a fila como cancelada sem rodar; B-5 junto (pool sem threads falha a operação) | rodada de desenho (31f5ae9ba) |
| M-10 escritor JSON sem profundidade | Corrigido na rodada de desenho: o escritor recusa `state.json.too_deep` na profundidade do leitor | rodada de desenho (31f5ae9ba) |
| M-11 contador acima de 2^53 | Confirmado e corrigido junto com A-7 | 362f80836 |
| M-12 razão de escala inf/NaN | Corrigido na rodada de desenho: resultado da razão filtrado por `is_finite`; B-8 (erro explícito de profundidade no upgrade) junto; B-9 mantido (ver decisoes-autonomas) | rodada de desenho (31f5ae9ba) |
| Extras do relatório: B-1 (Slot sem Drop), B-2 (`from_secs_f64`), B-12 (clamp com NaN no LV2), B-13 (saídas velhas no dual) | Confirmados e corrigidos | bdc71e9f8, f56190a83 |
