# Varredura de qualidade: session-io

Área: `lm-session`, `lm-io`, `lm-remote`, `lm-headless` (LinuxMusicBeta/crates/).
Auditoria somente leitura, sobre `origin/master` no commit b051d6281.

Cobertura: leitura completa de `lm-io/src/device.rs`, `lm-io/src/backend.rs`, `lm-remote/src/files.rs`, `lm-session/src/folder.rs` (escrita atômica), `lm-session/src/journal.rs` (thread e Drop), `lm-session/src/watchdog.rs`, `lm-session/src/osc/outbox.rs`, `lm-session/src/midi/record.rs` (trecho de sync), partes de `lm-remote/src/server.rs` (accept, handshake, dispatch de upload, run_action, pump de eventos), `lm-remote/src/client.rs` (threads, reconexão, upload), `lm-io/src/alsa.rs` (start/stop/close e laço), `lm-io/src/jack.rs` (callback e liberação), `lm-io/src/coreaudio.rs` (thread), `lm-io/src/disk.rs` (pontos de erro), `lm-session/src/session.rs` (struct e ordem de drop, threads), `lm-session/src/core_actions.rs` (device, forget, zip, locate). Não lidos inteiros: a maior parte de `lm-session` (sessão, port, export, index, midi), `lm-io` (take, resources, peaks, mididev, midi, jackmidi, edge, stretch), `lm-remote/src/codec.rs` e `delta.rs`, `lm-headless/src/main.rs` (só o laço principal). Os achados abaixo são os verificados; a lista não é exaustiva para esses arquivos.

## Resumo

| Gravidade | Quantidade |
|---|---|
| CRÍTICO | 2 |
| ALTO | 7 |
| MÉDIO | 9 |
| BAIXO | 3 |

Os 5 piores:

1. `lm-session/src/actions.rs:167` e `lm-session/src/core_actions.rs:165`: ação de Control (token remoto) escreve ou abre caminho arbitrário do servidor (CRÍTICO).
2. `lm-session/src/midi/record.rs:698-704`: erro de escrita do log MIDI é engolido e o buffer é limpo mesmo assim, perdendo eventos gravados em silêncio (CRÍTICO).
3. `lm-io/src/device.rs:434-436`: falha ao criar a thread vigia derruba o backend com o callback ainda vivo (ALTO).
4. `lm-remote/src/server.rs:262-300`: sem limite de conexões, sem timeout antes do hello, sem cota de disco para uploads de até 16 GiB (ALTO).
5. `lm-remote/src/server.rs:210` e a fila de eventos em `start`: filas de saída e de eventos ilimitadas, cliente lento faz a memória crescer (ALTO).

---

## CRÍTICO

### C1. Ação remota de Control escreve em caminho arbitrário do servidor
- `lm-session/src/actions.rs:167` (`midi.export`, `path` sem checagem), `lm-session/src/midifile.rs:404-408` (grava `path.with_extension("mid.tmp")` e renomeia para `path`).
- Também sem checagem: `lm-session/src/actions.rs:156-177` (`midi.import` e `resources.import`, leitura de caminho arbitrário), `lm-session/src/core_actions.rs:139` (`resources.locate`, copia arquivo arbitrário para o projeto), `lm-session/src/core_actions.rs:165` (`browser.reveal`, passa caminho ao xdg-open).
- `lm-remote/src/server.rs:809-822`: `run_action` só valida as chaves `folder` (com `inside_projects`) e `file` (com `inside_tree`). Qualquer outra chave com caminho passa. Essas ações não são administrativas (`proto.rs:54-56`), então exigem só o papel Control, que um token TCP pode ter.
- Efeito: quem tem token Control pode sobrescrever qualquer arquivo que o usuário do programa possa gravar, ou ler arquivos do servidor.
- Correção: validar todo parâmetro com caminho (lista de chaves por ação, `canonicalize`, dentro da pasta de projetos ou de uma pasta que o usuário escolheu). Ações com caminho que não sejam de projeto (`midi.*`, `resources.import`, `resources.locate`, `browser.reveal`) devem exigir Admin ou não ser expostas no remoto. `browser.reveal` não deve ser remoto.

### C2. Erro de escrita do log MIDI é engolido e os eventos somem
- `lm-session/src/midi/record.rs:698-704`: `fn sync(t: &mut Take)` faz `let _ = f.write_all(...).and_then(|_| f.sync_data());` e depois `t.unsynced.clear()` incondicionalmente.
- Com ENOSPC ou falha de disco, os eventos gravados são descartados, o take segue como se estivesse íntegro e o usuário não recebe aviso.
- Correção: propagar o erro; manter `unsynced` em caso de falha; encerrar o take com um estado de falha visível, como `StopReason::WriteFailed` em `lm-io/src/disk.rs`.

---

## ALTO

### A1. Backend derrubado com o callback vivo quando a thread vigia não nasce
- `lm-io/src/device.rs:426-437` (`abandon`): a closure da thread `lm-abandoned-device` move `backend` e `returned`. Se `spawn` falhar, a closure é descartada e `backend` com ela, enquanto a thread de áudio ainda pode estar rodando o callback. O comentário "a device is leaked as it is" está errado: o código dropa.
- Viola a regra de D11 ("never dropped under its callback").
- Correção: guardar o backend antes do spawn (por exemplo em um `static ABANDONED: Mutex<Vec<Box<dyn Backend>>>`), ou usar `mem::forget` explícito no ramo de erro sem mover o backend para a closure.

### A2. `panic!` na thread principal quando o backend não devolve o callback
- `lm-io/src/device.rs:254-260` (`start`): se `backend.start` falha e o callback não volta em `STOP_TIMEOUT`, o código dá `panic!` na thread principal.
- Correção: retornar `Err` com a mensagem de chave (`io.device.start_kept_callback`), sem panic. Se o renderer não puder voltar, `mem::forget` com comentário e contagem em `ABANDONED`.

### A3. `thread::sleep` na thread de áudio (ALSA, caminho de suspensão)
- `lm-io/src/alsa.rs:717`: `handle_problem`, ramo `PcmIoError::Suspended` com recuperação pendente, faz `std::thread::sleep(Duration::from_millis(10))` dentro do laço do device, que é a thread de tempo real.
- Viola a regra 1 da varredura (sleep proibido no caminho de áudio).
- Correção: não dormir. Retornar `Ok(())` e deixar o `poll` com timeout dar o ritmo, ou marcar o estado e tentar de novo no próximo período.

### A4. Conexão meio-aberta quando a segunda thread não nasce
- `lm-remote/src/server.rs:209-240` (`spawn_conn`): `Event::Open` (com `out_tx` e `stream`) é enviado na linha 214, antes de criar a thread de escrita (linha 227). Se essa criação falhar, o `Err` volta ao accept, que faz `let _ = spawn_conn(...)` (linhas 276 e 300). A conexão fica registrada no servidor, o canal `out_rx` nunca é lido e o socket segue aberto.
- Correção: criar as duas threads antes de enviar `Open`. Em caso de erro, fechar o stream e não registrar a conexão.

### A5. Filas de saída e de eventos ilimitadas
- `lm-remote/src/server.rs:210`: `channel::<Out>()` por cliente, sem limite. Um cliente que não lê a resposta (ou um TCP parado) faz a fila crescer sem fim. `busy` só limita as mensagens contínuas, não as de resposta.
- Mesmo problema em `channel::<Event>()` do `Server::start`: um cliente pré-autenticação pode mandar quadros de até 32 MiB (`wire.rs:23`) mais rápido do que o pump drena.
- Correção: `sync_channel` com limite; ao estourar, fechar a conexão com erro `remote.backpressure`. Tamanho de quadro pré-hello menor (por exemplo 64 KiB).

### A6. Sem limite de conexões, sem timeout de leitura, sem cota de disco
- `lm-remote/src/server.rs:262-300` (laços de accept TCP e Unix): nenhum limite de conexões. Cada conexão ganha duas threads e nenhuma leitura do servidor tem timeout (`set_read_timeout` aparece só no cliente). Uma conexão que nunca manda o hello fica viva para sempre.
- `server.rs:56-70`: `max_upload` padrão 16 GiB, `max_uploads` 8 por conexão. Com N conexões, o disco pode ser preenchido sem limite global.
- Correção: `max_connections`; timeout de 10 s até o hello aceito; limite global de bytes de upload e verificação de espaço livre; reduzir o padrão de `max_upload`.

### A7. Dispositivo travado é vazado com `mem::forget` a cada tentativa
- `lm-session/src/core_actions.rs:1013` (`device.reopen`) e `:1123` (`device.calibrate`): a documentação (D11) diz que é intencional. Cada tentativa que trava deixa um `Device`, seu renderer e buffers vivos para sempre. Tentativas repetidas acumulam memória.
- Correção: limitar o número de devices abandonados por processo (quando passar do limite, recusar reabrir e pedir reinício) e expor a contagem `abandoned_devices()` em `notices`.

---

## MÉDIO

### M1. Erros de `sync` e `append` do gravador de takes são ignorados
- `lm-io/src/disk.rs:827` (`let _ = r.append(line)`), `:647`, `:753`, `:793` (`let _ = r.sync()`). O comentário em 827 diz que a falha aparece no fechamento porque o sync também falha. Isso depende de ordem e não é garantido (um erro transitório de `append` pode sumir).
- Correção: propagar para `StopReason::WriteFailed`, como já é feito para `w.sync()` na linha 644-646.

### M2. Cabeçalho do log MIDI ignorado
- `lm-session/src/midi/record.rs:570`: `let _ = f.write_all(header(&info)...)`. Se o cabeçalho falhar, o take segue sem cabeçalho e a recuperação futura não consegue ler o arquivo.
- Correção: propagar e não iniciar o take.

### M3. Arquivos temporários que sobram após erro
- `lm-session/src/core_actions.rs:565-566` (`resources.locate`: `copy` e `rename` com `?`, `.tmp` fica na pasta de áudio).
- `lm-session/src/session.rs:2027-2028` (`save_as`: `copy` com `?`, `.save_as.tmp` fica no destino) e `:2872-2874` (paste: `.paste.tmp`).
- `lm-session/src/midifile.rs:406-408`: `.mid.tmp` com nome fixo, não removido se `rename` falhar, e exportações simultâneas colidem.
- Correção: usar o padrão de `lm-session/src/folder.rs:166-184` (`write_atomic`: remove o `.tmp` em qualquer erro) em todos os casos.

### M4. Threads criadas com `.ok()` desligam o serviço em silêncio
- `lm-session/src/session.rs:201-210` (relógio `lm-clock`): sem relógio, o pump não roda e a interface congela sem aviso.
- `lm-session/src/watchdog.rs:88` (`lm-watchdog`): sem watchdog, travamentos não são detectados.
- `lm-session/src/journal.rs:221-240` (`lm-disk`): sem thread, o journal nunca é gravado, e a fila `jobs` enche e marca `incomplete` sem que o usuário saiba.
- Correção: propagar o erro na abertura da sessão ou registrar em `notices` com chave e estado degradado.

### M5. `expect` no laço de áudio do ALSA
- `lm-io/src/alsa.rs:543`: `[o.playback...or(o.capture...).expect("a stream"); 2]` panica se ambos forem `None`. O `catch_unwind` (linha 306) pega, mas o device vira `Panicked`.
- Correção: validar em `open` que há pelo menos um stream, e usar um array de `Option<&Pcm>` sem `expect`.

### M6. Thread de upload antiga pode continuar após reconexão
- `lm-remote/src/client.rs:621` marca `job.stop = true` na desconexão; `:712` (`start_upload`) faz `job.stop.store(false)` logo na reconexão. Se a thread antiga ainda não viu o flag, as duas rodam sobre o mesmo job, com offsets duplicados.
- Correção: geração por tentativa (um `u64` por thread, comparado a cada bloco), ou um `UploadJob` novo por tentativa.

### M7. Reconexão morre em silêncio se a thread não nascer
- `lm-remote/src/client.rs:651-662` (`start_retry`): se `spawn` falhar, `started.is_ok()` é falso, nada é gravado em `retry` e a reconexão nunca mais acontece, sem aviso.
- Correção: guardar o erro no estado de conexão e tentar de novo depois.

### M8. Confinamento de caminho sem canonicalização
- `lm-remote/src/files.rs:138-159` (`inside_projects`, `inside_tree`): comparam com `starts_with` lexical. Um link simbólico dentro da pasta de projetos aponta para fora e passa.
- Correção: `canonicalize` nos dois lados antes da comparação, e recusar `..` depois de resolver.

### M9. Cache de entradas do navegador cresce sem parar
- `lm-session/src/browser.rs:39-52` e `lm-session/src/port.rs:556-558`: `known: BTreeMap<EntryId, Known>` recebe todo arquivo listado (com `kids: Vec` em pastas de plugin) e nunca remove nada. Os ids são hash estável, então o crescimento é limitado pelo número de arquivos distintos, mas em uma biblioteca grande fica grande.
- Correção: limitar (LRU) ou limpar ao trocar de página do navegador.

---

## BAIXO

### B1. Índice do navegador sem fsync antes do rename
- `lm-session/src/index.rs:222-223`: `fs::write` seguido de `rename`, sem `sync_all`. O índice é regenerável, então a perda é só de desempenho.

### B2. Marcadores de overflow descartados
- `lm-io/src/record.rs:264`: `let _ = self.markers.push(Marker::Overflow ...)`. Se o anel de marcadores encher, o aviso de overflow do take se perde.

### B3. Contador de timeouts do ALSA sem limite
- `lm-io/src/alsa.rs:541-544`: `timeouts` é `u32` e cresce sem reset enquanto `state` for `Some`. A multiplicação só ocorre com `state` `None`, então o risco é baixo. Vale um `saturating_add`.

---

## O que está bom

- `#![forbid(unsafe_code)]` em `lm-session`, `lm-io`, `lm-remote` e `lm-headless`. Nenhum `unsafe` fora de `lm-sys-*`.
- Barreira do device (`lm-io/src/device.rs`, `backend.rs`): o callback só volta quando a thread sai, `Drop` faz o mesmo barreamento, e o caso travado é tratado (D11) sem matar a thread.
- `lm-io/src/jack.rs`: o processo usa `try_lock` e nunca bloqueia no callback do JACK; a liberação do handle tem dupla garantia (ciclo após stop e `join`).
- `lm-io/src/alsa.rs`: os PCM são fechados antes de o callback voltar, então ao retornar do `stop` o hardware está livre.
- `lm-io/src/coreaudio.rs`: ordem explícita de liberação (listeners, proc, depois engine), com comentário.
- `lm-session/src/session.rs`: campo `device` antes de `journal` e `untitled_guard`, então o device para antes de qualquer limpeza; `UntitledGuard` só remove pasta sem áudio.
- `lm-session/src/osc/outbox.rs`: assinantes, pendentes e eco todos limitados.
- `lm-session/src/index.rs`: `MAX_DEPTH` e `MAX_RESULTS`.
- `lm-session/src/core_actions.rs` (exportação ZIP): `.tmp`, sync e renomeação, com remoção do `.tmp` em erro.
- `lm-session/src/folder.rs` (`write_atomic`): `.tmp`, sync, renomeação, sync da pasta, remoção do `.tmp` em erro (H3).
- `lm-remote/src/codec.rs` (`intern`): chaves de até 160 bytes e no máximo 16384 entradas, então o `Box::leak` é limitado.
- `lm-remote/src/wire.rs` (`read_frame`): o tamanho é checado antes da alocação.
- `lm-remote/src/server.rs`: papel padrão `Observe` até o hello, `upload_chunk` exige papel Control, comparação de token sem atalho, limite de taxa por conexão.
- `lm-remote/src/files.rs`: hash BLAKE3 conferido antes de renomear, `.part` removido em divergência, nomes de projeto só com um componente normal.
- `lm-session/src/journal.rs`: fila limitada com contagem de perdas e marca `incomplete`; `Drop` faz join.
- `lm-session/src/watchdog.rs`: o `Drop` acorda a thread com `unpark` antes do join.
- `lm-session/src/core_actions.rs`: `Operation` captura pânico e vira falha (`error.operation.panicked`), sem derrubar o worker.

## Resultado da confirmação

| Achado | Veredito | Commit |
|---|---|---|
| C1 caminho arbitrário em ação remota | confirmado e corrigido: `run_action` valida `path`/`file`/`dir` dentro da árvore de projetos para quem não é Admin, `inside_tree` resolve links simbólicos (cobre M8 para `inside_tree`), `browser.*` virou admin | 6c71941ee |
| C2 erro do log MIDI engolido | confirmado e corrigido: `sync` devolve o erro, o log é fechado (linha parcial corromperia nova tentativa), aviso `midi.record.log_failed`; os eventos continuam em memória e entram na árvore. Os eventos não somem do take, só a proteção contra queda | 3e3fb0e44 |
| A1 backend derrubado se a thread vigia não nasce | confirmado e corrigido (backend compartilhado, `mem::forget` no erro). Sem teste: não há como simular falha de `spawn` | commit "lm-io: a failed watcher thread..." |
| A2 `panic!` em `Device::start` | corrigido na rodada de desenho: `start` devolve `StartError` (`Refused` com o renderer, `Lost` quando o backend ficou com a callback: o device é abandonado, sem pânico) | 79d9d6aa1 |
| A3 `sleep` no laço ALSA em suspensão | falso na prática: sem áudio fluindo durante a suspensão, o sleep de 10 ms é o ritmo do laço (sem ele o poll giraria sem parar) | |
| A4 conexão meio-aberta | confirmado e corrigido (threads antes do `Open`; teste `a_big_frame_before_the_hello...`) | commit "lm-remote server: register a connection..." |
| A5 filas ilimitadas | confirmado e corrigido: `sync_channel` de saída (fila cheia fecha a conexão) e de eventos, quadro antes do hello limitado a 64 KiB | idem |
| A6 limites de conexão | corrigido na rodada de desenho (resto): `ServerConfig::quota` (`UploadQuota`): bytes pendentes de todos os clientes, total da pasta, espaço livre mínimo, `.part` parado há 7 dias é removido; recusa `remote.upload.quota` agora responde o import (o `upload_begin` leva o `id`) | f0d35be06 |
| A7 devices abandonados acumulam | corrigido na rodada de desenho: contador de abandonados ainda presos, teto de 3, `open` de hardware recusado acima (`io.device.too_many_abandoned`), aviso `session.device.abandoned_limit` | 79d9d6aa1 |
| M1 sync/append de take ignorados | confirmado e corrigido (`record_error` sticky, `WriteFailed`). Sem teste: depende do harness de falhas de disco do `disk.rs` | 4c62a8e29 |
| M2 cabeçalho do log MIDI | confirmado e corrigido (junto com C2) | 3e3fb0e44 |
| M3 `.tmp` que sobram | confirmado e corrigido (`copy_via_tmp`, `.mid.tmp` único e removido; teste em `folder.rs`) | bd662b211 |
| M4 threads com `.ok()` | confirmado e corrigido na rodada de desenho: relógio e thread de disco que não nascem falham a abertura da sessão (`session.thread.failed`); watchdog ausente vira aviso e a sessão segue degradada. Sem teste: não há como simular falha de `spawn` | 9b26763ae |
| M5 `expect` no laço ALSA | confirmado (inalcançável porque `open` exige um stream) e corrigido na rodada de desenho: `match` sem `expect`; B3 (`saturating`) junto. Linux, sem `cargo check` aqui | 4150c5ef5 |
| M6 upload antigo após reconexão | confirmado e corrigido na rodada de desenho: geração por tentativa no `UploadJob` (substitui `stop`) | 2706a5618 |
| M7 reconexão morre se o spawn falha | confirmado e corrigido na rodada de desenho: o erro vai para o estado `Reconnecting` e o `pump` tenta de novo em 1 s | 2706a5618 |
| M9 cache do navegador cresce | confirmado e corrigido na rodada de desenho: teto de 40000 entradas, o quarto mais antigo sai | d380852e3 |
| M8 links simbólicos | corrigido para `inside_tree` (teste); `inside_projects` já canonicalizava o pai | 6c71941ee |
