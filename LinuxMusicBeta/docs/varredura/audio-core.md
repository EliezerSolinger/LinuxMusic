# Varredura de qualidade: audio-core

Área: `LinuxMusicBeta/crates/lm-render` e `LinuxMusicBeta/crates/lm-compile`, em `origin/master` (commit b051d6281 no momento da leitura).
Método: leitura de todos os arquivos de `src/` das duas crates, sem compilar nem executar. Os achados foram conferidos relendo o trecho do código. Também foram lidas as funções de outras crates que rodam dentro do bloco de áudio (`lm-midimap` `MapTable::feed` e `inherit`, `lm-time` `relocate`, `sample_at`, `segment_at_sample`): não alocam.

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO | 1 |
| ALTO | 7 |
| MÉDIO | 4 |
| BAIXO | 6 |
| Total | 18 |

Por crate: `lm-render` tem 0 CRÍTICO, 5 ALTO, 4 MÉDIO, 2 BAIXO. `lm-compile` tem 1 CRÍTICO, 2 ALTO, 0 MÉDIO, 4 BAIXO.

### As 5 piores

1. **CRÍTICO** `lm-compile/src/items.rs:345` (laço em 359 e 390): `reps = (span + l - 1) / l` sem limite. Com `loop` pequeno (1 tick, valor aceito pelo filtro `>= 1`) num item longo, o compilador gera milhões de repetições e as listas de eventos crescem sem teto. Isso trava a thread de compilação e pode esgotar a memória do processo.
2. **ALTO** `lm-compile/src/core.rs:911-916`: o `expect("a tail is planned for every replacement")` dispara quando um efeito de nota (`NoteEffect`) é trocado por outro build ou recriado (`renew`). `decide` (linha 1135) não planeja cauda para efeito de nota, mas o commit espera uma. O `catch_unwind` converte o pânico em `compile.panicked`, a troca nunca é aplicada e o núcleo pronto é perdido em `from_table`.
3. **ALTO** `lm-render/src/mpe.rs:149-156` (`MpeOut::start`): quando não há canal livre, o canal mais antigo é sobrescrito sem enviar note-off da nota anterior. Depois, o `end` dessa nota não encontra nada e o hardware externo fica com a nota presa.
4. **ALTO** `lm-compile/src/core.rs:1907` (e `556-559`, `thread.rs:219-225` e `259`): a sobrevivência de uma cauda (U32) é decidida pelo relógio de parede (`t.created.elapsed() < TAIL_LIFETIME`), e não pelo tempo de áudio. A saída da compilação depende de quando ela roda. Numa exportação mais rápida ou mais lenta que o tempo real, a cauda pode ser cortada antes ou depois de 2 s de áudio.
5. **ALTO** `lm-render/src/renderer.rs:1936` e `2540`: o seguidor de pico (`ModKind::Peak`) mede o pico de cada passagem (`peak_measure` sobre o bloco ou o trecho entre bordas de loop e de tempo) e avança o seguidor por passos de grade. O resultado depende do tamanho do bloco do dispositivo. Exportação e reprodução podem sair diferentes com o mesmo projeto, o que quebra a regra de determinismo.

## lm-render

### ALTO

**R-1. `renderer.rs:1936` e `2540`: seguidor de pico depende do tamanho do bloco** (determinismo)
- O que está errado: `post()` define `m.follower.target` com o pico da passagem atual (`peak_measure` sobre `n` quadros). `modulation_events()` avança o seguidor com `advance(&p, rate, len)` por passo de grade (32 amostras). Como as passagens são cortadas em `max_block`, em bordas de loop e em trechos de tempo, a janela de medição muda com o tamanho do bloco. Duas renderizações do mesmo projeto com blocos diferentes dão moduladores diferentes.
- Correção: medir o pico por janela de grade que atravessa as passagens. Guardar um acumulador (soma de quadrados ou pico) no nó e fechar a janela no mesmo quadro da grade, de modo que a janela dependa só do número do quadro, como o LFO depende só do quadro. Assim o alvo depende só do número do quadro, não da partição do bloco.

**R-2. `renderer.rs:808-819` (`swap_program`): alocação no thread de áudio** (regra 5)
- O que está errado: as closures `effects_of` e `chain_of` devolvem `Vec<NodeIndex>` e são chamadas dentro de um laço por cadeia. A comparação `effects_of(old, oi) != effects_of(program, ci)` aloca duas vezes por cadeia a cada instalação. `chain_of` faz uma busca linear, então o custo é quadrático no número de cadeias. A regra 5 proíbe alocação no bloco de áudio, e a instalação roda nele.
- Correção: comparar as sequências por iterador, sem coletar (`a.effects.iter().map(..).eq(b.effects.iter().map(..))`). Calcular o mapa `target -> chain` uma vez, fora do áudio (no `build` do programa), ou com um vetor indexado pelo slot do nó.

**R-3. `renderer.rs:926`: ordenação estável pode alocar no thread de áudio**
- O que está errado: `self.live.sort_by_key` é a ordenação estável do Rust. Para slices acima de cerca de 340 elementos (`LiveMidi` tem 12 bytes e o buffer de pilha da ordenação cabe 4 KiB), ela aloca memória no heap. O limite padrão `max_live_midi_per_block` é 1024, então uma rajada de MIDI com mais de ~340 mensagens num bloco dispara a alocação.
- Correção: ordenar in-place sem alocação e com desempate estável. Por exemplo, guardar a ordem de chegada em `LiveMidi` e usar `sort_unstable_by_key(|m| (m.offset, m.arrival))`. Alternativa: ordenação por inserção, que é estável e não aloca.

**R-4. `panic.rs:47`: o hook silencia todo pânico em thread de render**
- O que está errado: `install_panic_hook` não imprime nada quando `is_render_thread()` é verdadeiro. Isso vale também para pânicos fora de `catch_unwind`: em `apply`, `take_commands`, `prepare`, `post`, `swap_program` e `mix_feed`. Um índice fora do limite nesses pontos sobe pela `render()` sem mensagem nenhuma. Não verificado nesta área: se `lm-io` envolve `render()` em `catch_unwind`. Se não envolve, o processo pode abortar sem aviso.
- Correção: silenciar só dentro dos sites de `catch_unwind`, com um `Cell` de "dentro de catch" ligado ao redor de cada `catch_unwind`. Pânicos fora dele devem ser reportados (por exemplo, um flag atômico lido pelo thread principal, ou um registro de pânico sem alocação).

**R-5. `mpe.rs:149-156` (`MpeOut::start`): nota presa no hardware**
- O que está errado: sem canal livre, `start` sobrescreve `slots[index]` com a nova nota e devolve o canal. Nenhum note-off é gerado para a nota antiga. Quando a nota antiga termina, `end(note)` não a encontra (`index_of` devolve `None`) e nada é enviado. O dispositivo externo mantém a nota soando até o canal receber outro note-off.
- Correção: `start` deve devolver também a nota removida (canal e chave). O chamador (`send_mpe`, `renderer.rs:1505`) emite `0x80 | ch, key, 0` antes do `0x90` da nova nota.

### MÉDIO

**R-6. `renderer.rs:1603-1604`, `1625`, `1629`: eventos descartados sem contagem**
- O que está errado: `chain.cur.push(...)`, `chain.merge.push(...)` ignoram o `bool` de retorno. Um evento de note-off recusado (por exemplo, acima da capacidade dura) some sem ser contado em `dropped`, e a nota pode ficar presa no instrumento.
- Correção: verificar o retorno e incrementar `dropped[1]` (como `deliver` faz). Revisar se a capacidade dura de `merge` (`CHAIN_EVENTS + 256`) cobre o pior caso de `io.events`.

**R-7. `renderer.rs:541-548`: gravação perde trechos sem marcador**
- O que está errado: se `report.spans` está cheio, o trecho gravado não é registrado, `take_frames` não avança e nenhum evento é emitido. Pela análise dos limites (`span_capacity`), isso não deve acontecer, mas a falha é silenciosa.
- Correção: quando estiver cheio, emitir `TransportEvent` de falha (como `CaptureGap`) ou um `Telemetry`, e parar a gravação de forma explícita, em vez de descartar o trecho.

**R-8. `renderer.rs:2306-2309` (`run_level`): espera ocupada sem limite**
- O que está errado: enquanto um worker está ocupado, o thread de áudio gira em `spin_loop` até ele devolver o job. Não há prazo. Um processador travado num worker congela o callback do dispositivo. Com `Limits::default().workers == 0` o problema não aparece, mas a sessão pode configurar workers.
- Correção: prazo com limite (por exemplo, por contagem de iterações ou pelo `Watch`), que marca o nó como morto e devolve silêncio para ele. Documentar que processadores nativos precisam de tempo limitado.

**R-9. `renderer.rs:2339-2340` (`job`): `expect` no thread de áudio, fora de `catch_unwind`**
- O que está errado: `expect("runnable node has a processor")` e `expect("node memory is home between calls")` rodam no áudio. Se o invariante quebrar, o pânico não é capturado.
- Correção: devolver `Option`. Quando faltar núcleo ou memória, pular o nó (silêncio) e reportar `NodeNonFinite` ou um telemetry próprio.

### BAIXO

**R-10. `renderer.rs:2800-2801`: requisições de preview descartadas sem contagem**
- O que está errado: `for _ in s.outbox.drain() {}` joga fora as requisições do processador de preview, e o resultado de `take_dropped()` é ignorado com `let _`. Requisições como `GestureBegin` e `ParamChanged` se perdem sem aviso.
- Correção: encaminhar as requisições como `Telemetry::PluginRequest` (como os nós fazem) e somar `take_dropped` em `dropped[4]`.

**R-11. `exec.rs:202`: `expect` em `Workers::spawn`** (thread principal)
- Falha ao criar um worker derruba o processo. Correção: devolver `Result` e propagar o erro para `engine_with`.

## lm-compile

### CRÍTICO

**C-1. `items.rs:345` (laços em `359` e `390`): `loop` pequeno gera repetições sem limite**
- O que está errado: `reps = ((span + l - 1) / l).max(1)`. Com `l = 1` tick e um item de, por exemplo, uma hora (~3,6 milhões de ticks a 960 PPQ), `reps` chega a milhões. Cada repetição gera eventos para as notas que começam antes de `loop`. A lista cresce linearmente e a memória pode estourar. O cálculo `span + l - 1` também pode estourar `i64` com valores extremos.
- Correção: rejeitar `loop` abaixo de um mínimo (por exemplo, 1/64 de semibreve, 60 ticks a 960 PPQ) com erro `compile.item.loop_too_short`. Limitar o número total de eventos gerados (orçamento por item) e usar aritmética com `checked_*` ou `saturating_*`.

### ALTO

**C-2. `core.rs:911-916` e `1135-1139`: pânico ao trocar um efeito de nota**
- O que está errado: em `decide`, quando o build pronto (`use_ready`) substitui um processador instalado que é efeito de nota, nenhuma cauda é planejada (`filter(|o| !o.shape.note_effect)`). No commit, `proc.installed.replace` devolve `Some(old)`, e o ramo `Some(old)` faz `expect` numa cauda que não existe. Acontece na troca de plugin de um efeito de nota e no `renew` (recriação após falha de helper). A mudança nunca é aplicada, a cada tentativa a compilação cai em `compile.panicked` e o núcleo pronto se perde em `from_table`.
- Correção: no commit, tratar `Some(old)` com cauda opcional. Se o `old` for efeito de nota, ou se não houver cauda planejada, usar `install.replace(idx, r.core, None)` e devolver o antigo. Trocar o `expect` por esse caminho. Teste sugerido: trocar o plugin de um efeito de nota e verificar que a compilação termina sem pânico.

**C-3. `core.rs:1907` (e `556-559`, `thread.rs:219-225`, `259`): cauda decidida pelo relógio de parede**
- O que está errado: `live` filtra as caudas com `t.created.elapsed() < TAIL_LIFETIME`. Isso usa `Instant`, não o tempo de áudio. O renderer silencia a cauda por quadros (`TAIL_SECONDS` em `renderer.rs`). Se a compilação demora, ou se o áudio anda mais devagar que o tempo real (exportação, xruns), o compilador remove a cauda antes que o renderer a silencie. Com a mesma entrada, duas execuções dão saídas diferentes. A remoção também causa um corte abrupto no áudio.
- Correção: a vida da cauda deve ser decidida só pelo `TailEnded` que vem do renderer (que já existe). Remover o filtro por `created.elapsed()`. O deadline de `TAIL_LIFETIME` pode continuar só como despertador do thread de compilação, sem influenciar o resultado.

### BAIXO

**C-4. `core.rs:1382`: `plan.index[&NodeKey::ChannelIn(MASTER)]`**
- Panica se `mixer.master` não existir. A validação de `lm-doc` (`check_master`, `rules.rs:138`) impede isso, então é só uma defesa. Correção: trocar o índice por `get` e devolver um erro `compile.plan.no_master`.

**C-5. `map.rs:218-239` (`search`): recursão sem limite de profundidade**
- A busca em árvore recursiva segue a profundidade do documento. Um documento muito aninhado estoura a pilha. Só é usada por testes e ferramentas (o próprio comentário diz isso). Correção: versão iterativa com pilha explícita, ou limite de profundidade.

**C-6. `keys.rs:84`, `95`, `115` e vizinhos: `as i32` em valores vindos de arquivo**
- Os graus do arquivo de afinação são `i64` e são convertidos com `as i32` sem checagem. Valores grandes dão resultado errado silenciosamente. Não há pânico. Correção: validar a faixa dos graus na leitura do arquivo (`lm-doc::tuning`) e recusar valores fora de faixa.

**C-7. `thread.rs:59` (`Shared.results`): fila sem limite**
- Se a sessão nunca chama `poll`, cada resultado (que contém um `Install` com o programa inteiro) se acumula. Correção: limitar a fila (por exemplo, guardar só o mais recente quando a fila passa de 2) ou descartar resultados obsoletos por `seq`.

## O que está bom

- `lm-render` tem `#![forbid(unsafe_code)]`. Nenhum `unsafe` na área.
- Instalações e devoluções: `EngineControl::install` reserva o espaço de volta (`reserve_returns`), e `apply` só empurra em vetores reservados. Os objetos que saem do áudio vão pelo anel de retorno. `HandoffReceiver::give_back` usa `mem::forget` como último recurso, em vez de liberar no áudio.
- Pânicos de processadores: payloads vão para o thread principal em `PanicReport`, e o `push` cheio faz `forget` em vez de `drop` (`renderer.rs:2885-2894`).
- Saídas de telemetria (`Telemetry`, `TransportEvent`, `Touched`, `MapEvent`, `RecordedMidi`, `MidiOutEvent`) são `Copy`, então o `drop` na recusa do anel não libera memória.
- Filas com capacidade (`pending`, `live`, `held`, `map_hits`, `midi_active`, `ActiveNotes`) checam `len() < capacity()` antes de empurrar, então não realocam.
- `EventList` e `NoteOut` usam uma chave de ordenação total (offset, rank, sequência). O resultado não depende da ordem de chegada nem do thread que roda o nó.
- O RNG de cada nó é semeado por `seed_for(projeto, id)`, sem relógio.
- Canais de parâmetros: a escrita grava o valor com `Relaxed` e depois faz `fetch_or(Release)` do bit sujo. A leitura faz `swap(Acquire)`. O par de ordens está correto (testado em loom).
- Fluxo de disco (`stream.rs`): seqlock com tag antes e depois da cópia e `fence` nos dois lados. Leitura e escrita não bloqueiam e não alocam.
- Atribuição de índices e slots (`alloc.rs`): menor livre primeiro, geração que cresce, e índice pendente até a devolução. Não há reuso antes do retorno.
- Tabelas do compilador (`BTreeMap`, `BTreeSet`) são ordenadas. Os `HashMap` e `HashSet` são usados só para consulta e pertinência, e os conjuntos de `channel_solo` são resultados de fecho transitivo, que não dependem da ordem. A saída do compilador não depende da ordem de iteração de hash.
- Divisão do bloco no renderer: o laço de automação e de modulação de `Rate::Block` usa a grade global de 32 quadros, então automação e LFO não dependem do tamanho do bloco.
- A compilação roda num thread próprio com `catch_unwind`, e recomeça da última tabela boa (H14).
- `lm-compile` valida as entradas que chegam ao renderer (`ProgramBuilder::build`): limites de nó, slot, porta, canal de feed, ciclos e tamanho de linha de atraso. Por isso os índices que o áudio usa depois são seguros.

## Resultado da confirmação

| Achado | Veredito | Commit |
|---|---|---|
| C-1 laço de 1 tique gera repetições sem limite | Confirmado e corrigido: o número de repetições é calculado sem estouro e a compilação recusa o item (`compile.item.too_many_events`) quando repetições x notas/pontos de controle passa de 2^20 eventos, antes de gerar qualquer um (mais uma checagem durante a geração). O esquema do documento não mudou. | 32634cf82 |
| C-2 pânico ao trocar efeito de nota | Confirmado e corrigido: sem cauda planejada, `install.replace` recebe `None`. Teste `replacing_a_note_effect_installs_it_without_a_tail` (com plugin de teste `nfx`). | 559682b74 |
| C-3 cauda decidida pelo relógio de parede | Corrigido na rodada de desenho: a rede de segurança conta quadros renderizados (`TelemetryFrame::frames` → `Compiler::observe_frames`, `TAIL_NET_SECONDS`) e o corte duro da cauda cai no quadro exato. Testes `tail_hard_cap_is_the_same_for_any_block_size` e `a_tail_without_its_end_event_leaves_by_the_audio_clock`. | 4d404e3f7 |
| R-1 seguidor de pico depende do bloco | Corrigido na rodada de desenho: janela da grade com acumulador no nó (`PeakTracker`), leitura com atraso fixo de 4096 quadros. Teste `a_peak_follower_is_the_same_for_any_block_size`. | COMMIT |
| R-2 alocação em `swap_program` | Confirmado e corrigido: comparação por iteradores, sem `collect` (a busca linear da cadeia continua, sem alocar). | 1df90833d |
| R-3 ordenação estável aloca | Confirmado e corrigido: `seq` de chegada em `LiveMidi` e `sort_unstable_by_key((offset, seq))`; teste com rajada de 1000. | ff81ef2f1 |
| R-4 hook silencia pânico fora do `catch_unwind` | Corrigido na rodada de desenho: `catching` nos pontos de captura, registro fixo de mensagens alimentado pelo gancho fora deles, `take_render_panics` na sessão (`session.audio.panicked`), `note_caught_panic` em `lm-io` (dummy e alsa). Teste `a_panic_outside_a_catch_site_is_logged_and_one_inside_is_not`. | COMMIT |
| R-5 MPE nota presa no hardware | Confirmado e corrigido: `MpeOut::start` devolve a tecla da nota tomada e `send_mpe` envia o note-off antes; teste atualizado em `mpe.rs`. | ff81ef2f1 |
| R-6 eventos descartados sem contagem | Confirmado e corrigido: falhas de `push` em `run_chains` somam em `dropped[1]`. | 1df90833d |
| R-7 gravação perde trecho sem marcador | Corrigido na rodada de desenho: `TransportEvent::RecordFailed` e parada explícita da gravação (`push_span`). Teste `push_span_joins_contiguous_passes_and_reports_a_full_table`. | COMMIT |
| R-8 espera ocupada sem prazo | Corrigido na rodada de desenho: prazo de 1 s, worker abandonado, nó morto e `Telemetry::NodeStuck`. Teste `a_hung_worker_is_given_up_after_the_deadline`. | COMMIT |
| R-9 `expect` em `job` | Confirmado e corrigido: `job` devolve `Option` e o nó fica em silêncio. | 1df90833d |
| C-6 `as i32` em graus do arquivo de afinação (`lm-doc::tuning`) | Verificado: a leitura do `.kbm` já recusa graus fora de 0..=1023 (`parse_kbm`); a razão de intervalo infinita foi corrigida junto (M-12). Os demais itens de MÉDIO e BAIXO desta tabela são de `lm-compile` e `lm-render` | rodada de desenho (31f5ae9ba) |
