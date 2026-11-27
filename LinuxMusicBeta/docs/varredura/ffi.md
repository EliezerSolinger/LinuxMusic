# Varredura de qualidade: área `ffi` (crates `lm-sys-*`)

Auditoria somente leitura do código das crates `lm-sys-*` (única área autorizada a usar `unsafe`), incluindo os shims C/C++.
Base: `origin/master` no momento da varredura. Nada foi compilado nem executado.

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO (pode travar, corromper dados ou falhar áudio) | 3 |
| ALTO (vazamento, resultado errado, ou regra de tempo real quebrada, mas improvável) | 6 |
| MÉDIO (fragilidade) | 18 |
| BAIXO (limpeza) | 19 |
| **Total** | **46** |

Contagem feita sobre as entradas marcadas com severidade no corpo do texto (cada uma aparece uma vez).

### Os 5 piores

1. **CRÍTICO** `lm-sys-jack/src/client.rs:337-373` (`Client::activate`): nos caminhos de erro (callbacks não registrados, L365; `jack_activate` falhou, L370), o `Box<State>` é liberado ao sair da função, mas o JACK continua com ponteiro para ele nos callbacks já registrados (xrun, registro de porta, conexão, shutdown). O `Client` devolvido ao chamador segue vivo e pode receber esses callbacks na thread de notificação: uso após liberação.
2. **CRÍTICO** `lm-sys-coreaudio/src/mac.rs:802-810` (`Io::drop`): `self.stop()` (L804) e `AudioDeviceDestroyIOProcID` (L807) têm o resultado descartado com `let _`, e em seguida `Box::from_raw(self.state)` libera o estado do callback. Se a parada falhar, o IO proc pode continuar rodando na thread de tempo real e usar memória liberada.
3. **CRÍTICO** `lm-sys-guitarix/src/lib.rs:408` e `:418` (`process_mono` / `process_stereo`): a asserção compara o tamanho com `MAX_BLOCK` (256), e não com o bloco com que a unidade foi iniciada (`self.block`, definido em `start`). As unidades C (`GuitarixDetune.cpp` usa `numSampsToProcess`, `GuitarixConvolution.cpp` usa `m_block`, ambos fixados no start) podem escrever além de seus buffers se receberem mais quadros do que o bloco iniciado.
4. **ALTO** `lm-sys-linux/src/crash.rs:136-160` (`install_handlers`): se `sigaction` falhar no meio (i ≥ 1), os handlers já trocados não são desfeitos e `PREVIOUS` não é gravado. Numa nova tentativa, o "handler anterior" gravado passa a ser o próprio `on_signal`; num crash, `chain` chama `on_signal` de novo, que chama `chain` de novo: recursão sem fim no tratador de sinal.
5. **ALTO** `lm-sys-sndfile/src/lib.rs:371` (`Reader::open`): se `Info::from_raw(&raw)?` falha (arquivo malformado ou sem canais/taxa), o retorno acontece antes de a struct `Reader` existir, e o handle `SNDFILE` aberto por `sf_open_fd` nunca é fechado (vazamento por arquivo rejeitado).

---

## Por crate

### lm-sys-jack

**[CRÍTICO] client.rs:337-373 — `Client::activate`: uso após liberação nos caminhos de erro.**
O `state` (Box com `process`, `notify`, `ports`, `client`, `panicked`) é criado em L342 e seu ponteiro é passado como `arg` aos callbacks (L355-363). Se `codes` tiver erro (L364-366) ou `jack_activate` falhar (L368-371), a função retorna `Err((_, self))` e o `state` é liberado. Mas `self` (o `Client`) continua vivo e com os callbacks registrados; `Drop` do `Client` chama `jack_client_close`, e até lá os callbacks de registro/conexão/xrun/shutdown podem disparar na thread de notificação do JACK e ler `State` liberado.
*Correção:* em todo caminho de erro, zerar os callbacks (`jack_set_*_callback(raw, None, null)`) antes de liberar o `state`, ou mover o `state` para dentro do `Client` (que só o libera depois de `jack_client_close`).

**[MÉDIO] client.rs:240-241 e :324 (`port_name`, `latency`): índice de `Port` sem verificação.**
`self.ports[port.0]` entra em pânico se um `Port` de outro cliente (com mais portas) for usado. *Correção:* `get()` com `Option`, ou guardar o id do cliente dentro de `Port`.

**[BAIXO] client.rs:139 e :151-152:** `silent` é instalado como função de erro global em cada `open` (`jack_set_error_function`), o que é aceitável mas silencia mensagens de todos os clientes do processo. Registrar uma vez.

**O que está bom (jack):** todos os trampolins (`process`, `xrun`, `shutdown`, `buffer_size`, registro, conexão) usam `catch_unwind` e nunca deixam pânico cruzar para C; `process_trampoline` zera as saídas antes de chamar o handler e silencia após um pânico; `Drop` de `ActiveClient` fecha o cliente antes de liberar o `State`; áudio é copiado (sem empréstimo simultâneo de buffers).

### lm-sys-coreaudio

**[CRÍTICO] mac.rs:802-810 — `Io::drop`: estado liberado com o IO proc possivelmente ativo.**
`let _ = self.stop();` e `let _ = AudioDeviceDestroyIOProcID(...)` ignoram erros, e logo depois `drop(Box::from_raw(self.state))`. Se `AudioDeviceStop` falhar (`stop()` devolve `Result`, L775), o callback continua podendo chamar `io_trampoline` com `state` já liberado.
*Correção:* liberar `state` somente se `stop` e `DestroyIOProcID` tiverem retornado sucesso; caso contrário, vazar o Box de propósito (ou marcar um flag de "não liberar").

**[ALTO] mac.rs:516-528 — `Watch::drop`: a closure é liberada mesmo se a remoção do listener falhou.**
`AudioObjectRemovePropertyListener` tem o resultado descartado (L521) e `Box::from_raw(self.client)` roda em seguida. Se a remoção falhar por outro motivo que não "dispositivo sumiu", o CoreAudio pode continuar chamando `listener_trampoline` com ponteiro liberado.
*Correção:* liberar só com sucesso; senão vazar.

**[BAIXO] mac.rs (`get_vec`, L178-192):** `Vec::with_capacity(n)` com `n` calculado do tamanho informado pelo sistema; `set_len` usa o `min(n)` correto, mas a conversão `(n * size_of::<T>()) as u32` pode truncar para tamanhos grandes. Baixo risco na prática; usar `u32::try_from`.

**O que está bom (coreaudio):** `io_trampoline` e `listener_trampoline` usam `catch_unwind`; `Cycle::input/output` tratam `mData` nulo sem criar slice a partir de ponteiro nulo; `Aggregate::create` destrói o dispositivo no caminho de erro (drop); `Io::new` libera o estado se o registro falhar.

### lm-sys-alsa

Revisão completa de `src/` (pcm, poll, seq, midi, enumerate, format, pcm_types, error, ffi). As declarações FFI batem com os cabeçalhos (valores de constantes e layouts de `snd_seq_event_t`, 28 bytes, e `snd_pcm_channel_area_t` conferidos). O caminho de áudio (`write_src`/`read_dst`) não aloca: o `scratch` e os ponteiros de plano são criados em `configure` e os limites de cada transferência respeitam `capacity`, `frames` e `n`.

**[MÉDIO] midi.rs:45-47 — `MidiMessage::as_slice` entra em pânico se `len > 3`.** `bytes` e `len` são campos públicos, então um chamador pode montar `len = 5` e `&self.bytes[..5]` falha. *Correção:* tornar os campos privados (com construtores) ou limitar `len` a 3.

**[MÉDIO] midi.rs:160 — `(f.value + 8192)` pode estourar `i32`.** Um cliente pode enviar `value` arbitrário (`data[2]` como `i32`); em debug isso é pânico no thread de MIDI. *Correção:* `saturating_add`.

**[MÉDIO] linux/enumerate.rs:304-316 — `pcm_hints`: vetor de hints não verificado.** `snd_device_name_hint` com sucesso e vetor nulo levaria a `*hints.0.add(i)` desreferenciando nulo (UB). *Correção:* checar `raw.is_null()` antes do laço.

**[MÉDIO] linux/poll.rs:104-131 — `PcmPoll`:** `new` confia que `snd_pcm_poll_descriptors` devolve no máximo `count` (L117-119: `truncate` não estende se `got > count`, e `spans` fica inconsistente com `fds`). `wait` só confere que `pcms` é o mesmo conjunto em `debug_assert_eq!` (L131), então em release um conjunto trocado usa descritores de outra PCM. *Correção:* checar `got <= count` e comparar o tamanho de `pcms` em `wait` com verificação de release (retornar erro).

**[MÉDIO] linux/seq.rs:1006 — `Instant::now() - REMEASURE * 2` pode entrar em pânico** se o sistema acabou de subir (relógio monotônico menor que 2 s). *Correção:* `checked_sub(...)` e, se falhar, `Instant::now()` com `measured` forçando remedição.

**[BAIXO] linux/ffi.rs e pcm.rs:** `write_src`/`read_dst` no caminho mmap usam `?` em `mmap_commit`; um erro deixa a área de `mmap_begin` sem commit. Aceitável (o PCM entra em xrun e é recuperado), mas vale registrar.

### lm-sys-alsa (sem `unsafe` fora de linux/)

Nenhum achado adicional em `format.rs`, `pcm_types.rs`, `error.rs`, `midi.rs` (demais partes). Os testes de conversão cobrem NaN, saturação e ordem de bytes.

### lm-sys-ipc

**[ALTO] lib.rs:96-115 (`SharedMemory::open`) — o processo par pode encolher o objeto e causar SIGBUS no núcleo.** A checagem `st_size >= bytes` (L107) acontece uma única vez. O objeto é aberto com `O_RDWR` por um processo de host de plugin, que é código de terceiros e roda com o mesmo usuário; se ele fizer `ftruncate` para um tamanho menor depois da checagem, acessos às palavras além do fim do arquivo geram SIGBUS no núcleo. *Correção:* o núcleo deve tratar o objeto como não confiável: não aceitar `open` de um objeto que não ele mesmo criou, ou criar o objeto com o núcleo como dono do `ftruncate` e abrir somente leitura o lado do par quando possível; no mínimo, documentar que o par não pode reduzir o objeto (e verificar de novo o tamanho antes de cada ciclo, o que não é suficiente sem um mecanismo do kernel). Registrar o risco no safety sheet.

**Limites de offsets e tamanhos (exigência da varredura):** o crate não usa nenhum offset vindo do outro processo: `words()` devolve sempre a fatia inteira de `words` (validada em `create`/`open`, `checked_mul`, `filter(>0)`), e o acesso é só por `AtomicU32`. Não há leitura de estruturas do par. Os tamanhos de `create` são limitados por `checked_mul`; o nome é validado (`check_name`).

**[BAIXO] lib.rs:161-167 (`unique_name`)** usa o relógio de parede só para nomear; sem efeito em áudio ou dados gravados. Aceitável.

**O que está bom (ipc):** o segmento só é visto por `AtomicU32` (sem `&mut` nem `&[u32]` compartilhado); `unsafe impl Send/Sync` estão justificados pela ausência de estado não atômico; `wait`/`wake` do futex usam o flag compartilhado (não `_PRIVATE`), com timeout limitado; a asserção de alocação no teste cobre as chamadas de espera e de aviso.

### lm-sys-linux

**[ALTO] crash.rs:136-160 — `install_handlers` com falha parcial gera recursão no tratador de sinal.** Veja o resumo (item 4). *Correção:* em caso de falha no laço, restaurar os handlers já trocados a partir de `previous[0..i]` antes de retornar, ou gravar `PREVIOUS` parcial e nunca reinstalar; além disso, `chain` deve ignorar um "handler anterior" que seja o próprio `on_signal`.

**[MÉDIO] crash.rs:29, 46-47, 326 — `Probe` (`Box<dyn Fn>`) é chamado dentro do tratador de sinal.** O requisito "só chamadas async-signal-safe" é só uma convenção: o tipo não o garante, e a closure pode fazer qualquer coisa. *Correção:* a closure deve ser substituída por uma tabela de `AtomicU32` de tamanho fixo, lida diretamente; ou documentar e testar que a closure só lê atômicos.

**[MÉDIO] crash.rs:233-246 (`write_all`):** se `write` devolver 0 para um buffer não vazio, o laço gira para sempre dentro do tratador de sinal. *Correção:* tratar `n == 0` como erro e sair.

**[BAIXO] crash.rs:193-196 e 206-208 (`arm`):** bloco `if !old.is_null() { }` vazio (código morto). *Correção:* remover. `arm` também não valida `name` (um `/` escaparia de `dir`).

**[MÉDIO] linux/mem.rs:173-184 — `prefault_stack` recursivo sem limite.** `touch(bytes.div_ceil(4096))` recursa uma vez por página; um `bytes` grande estoura a pilha. A documentação pede "bem dentro da pilha", mas nada impõe. *Correção:* limitar `bytes` (por exemplo, a 1 MiB) ou fazer a preparação com um laço de `black_box` em um único quadro.

**[MÉDIO] linux/rt.rs:205-215 (`RealtimeGuard::drop`):** o retorno de `sched_setscheduler` só é checado por `debug_assert_eq!`. Em release, uma falha deixa a thread em tempo real sem aviso. *Correção:* registrar o erro (log) e tentar `SCHED_OTHER` pelo caminho de rtkit, se houver.

**[MÉDIO] linux/rt.rs:256-262 (`make_current_thread_realtime`):** se rtkit concede e a política lida não é FIFO/RR, a função devolve `Err` mas a thread já foi promovida pelo rtkit e não é revertida. *Correção:* restaurar a política anterior nesse caminho.

**[BAIXO] linux/bus.rs:419-424 (`Worker::release`):** `self.reservations.remove` acontece antes de `session.get`; se a reconexão falhar, a reserva sai do mapa e o nome D-Bus fica com o cliente até o fim da conexão, sem nova tentativa. *Correção:* devolver a entrada ao mapa em caso de falha.

**Verificado sem achado — bus.rs:259 e 333-362 (`zbus::block_on` dentro da thread `lm-dbus`):** correto e limitado por `method_timeout`.

**[BAIXO] reserve.rs:39 e 97:** `let _ = priority;` (parâmetro não usado no `RequestRelease`) e o retorno de `object_server().at(...)` (um `bool` que indica se já existia) é ignorado. *Correção:* tratar `false` como erro ou remover o registro antigo.

**O que está bom (linux):** a regra de `mlockall` (só com `RLIMIT_MEMLOCK` ilimitado) está implementada; `LockedBuffer` tem páginas próprias e `Drop` faz `munmap`; `StackLock` e `RealtimeGuard` não são `Send`; o D-Bus fica numa thread própria com canal, e `Reply` nunca bloqueia a thread principal; `SleepInhibitor` libera ao soltar o descritor; o tratador de sinal só usa `write`, `fsync`, `rename`, `sigaction`, `raise` e atômicos (exceto o `Probe`, acima), e preserva o tratador anterior.

### lm-sys-lilv

**[ALTO] urid.rs:95-106 e :163-170 — `urid:map` obtém um `Mutex` e pode alocar no thread de áudio.** A função `map_trampoline` é chamada pelo próprio plugin, inclusive dentro de `run` (LV2 não proíbe). Dentro de `run`, isso bloqueia o thread de áudio (o `save`/`unmap` de outro thread pode segurar o mesmo mutex) e aloca quando a URI é nova. *Correção:* pré-mapear todas as URIs no `instantiate` (o `map` passa a ser só leitura), ou usar uma tabela imutável/`RwLock` com leitura sem alocação; documentar que `map` em `run` é proibido e, se o plugin chamar, devolver 0 sem bloquear (`try_lock`).

**[MÉDIO] worker.rs:299-312 (`wait_idle`):** espera até `done >= scheduled` sem limite. Se a thread do worker tiver morrido (o `join` de `stop_and_join` só acontece depois), a restauração trava para sempre. *Correção:* limite de tempo e erro (`lilv.worker_failed`) quando a thread não estiver mais viva.

**[BAIXO] worker.rs:203-224 (`schedule_work`):** `scheduled.fetch_add` acontece depois de `push_message`. Um `work` rápido pode incrementar `done` antes de `scheduled`, e `wait_idle` pode retornar cedo. *Correção:* incrementar `scheduled` antes do push (e desfazer se o push falhar).

**[MÉDIO] state.rs:50-75 (`store`):** o tamanho de cada propriedade vem do plugin e é copiado sem limite (`to_vec`). Um plugin pode fazer o processo host crescer sem limite durante `save`. *Correção:* limite por propriedade e total (por exemplo, 64 MiB), com erro `lilv.state_too_large`.

**[BAIXO] instance.rs:544 (`RawBuf::new(max_block, value)` para portas CV):** `max_block` pode chegar a `i32::MAX` (validado só como `i32`), alocando até ~8 GB de `f32` por porta. *Correção:* limite de `max_block` no `InstanceConfig::check` (por exemplo, 1 << 20).

**[BAIXO] instance.rs (`run`):** os buffers de áudio do chamador ficam ligados às portas depois do `run` (só são religados no próximo `run`). O LV2 proíbe o plugin de guardar o ponteiro, então não há problema hoje; registrar no safety sheet.

**Verificado sem achado:** a ordem de `Drop` de `Instance` (worker parado e unido, depois `deactivate`, depois `lilv_instance_free` sob o lock do mundo, depois `host`/`schedule`) está correta; `start_worker` falhando pelo `?` descarta a instância inteira pelo `Drop`; o índice das portas (`info.ports`) é `0..num_ports`, então `controls` tem o tamanho certo; `Reader` do `state` valida os limites (`checked_add`); `Core` é `Send` só sob o `Mutex`.

### lm-sys-clap

Lidos: `Library` (abertura, `descriptors`, `Drop`), `Instance::new`, `process`, callbacks de eventos (`in_size`, `in_get`, `out_push`), streams de estado e `Drop`. A camada de host usa `guard` (`catch_unwind`) em todos os callbacks lidos. Não li em detalhe `ffi.rs` (declarações; conferidas pelos testes de layout) nem os blocos de parâmetros e portas (L300-520, L830-1000 parcial, L1150-1340). Esses trechos ficam como "não verificados por completo".

**[MÉDIO] lib.rs:1352-1362 (`stream_write`):** cada chamada aceita até 1 GiB do plugin e o `Vec` cresce sem limite total. *Correção:* limite do total (por exemplo, 64 MiB) e retorno de erro (-1) ao exceder.

**[BAIXO] lib.rs (`Library::drop`):** o `deinit` da entrada CLAP é chamado no thread que solta o último `Arc` (pode ser o thread de áudio, se a última instância for destruída lá). A especificação CLAP exige a thread principal para `deinit`. *Correção:* garantir a liberação na thread principal ou documentar.

**[BAIXO] lib.rs (MIDI → controlador, `ch * MIDI_CONTROLLERS + cc`, vst3 e clap):** com `cc >= 128` o índice cai na faixa do canal seguinte (vst3 usa `.get()`, então sem pânico). *Correção:* rejeitar `cc >= 128` antes da busca.

**O que está bom (clap):** `Instance::process` não aloca (`slots`, `out_events.params` e `changes` pré-alocados; `pending.drain` sem realocação); `out_push` respeita a capacidade; `in_get` confere `index < len`; `stream_read` limita a cópia ao tamanho restante; `Drop` de `Instance` chama `destroy` antes de liberar o host (campo `host`, `Box` estável).

### lm-sys-vst3

Lidos: `Module` (abertura, `classes`, `create`, `Drop`), `save_state`/`load_state`, `process` (parte de MIDI e transporte), `host.rs` (atributos e `Stream`). `testing.rs` (825 linhas, só com a feature `testing`) foi percorrido por grep de `unsafe`: são callbacks COM de teste, sem achado relevante. `lib.rs` parcial (editor e parâmetros não lidos por completo).

**[ALTO] host.rs:552-565 (`Stream::write`) e :572-586 (`Stream::seek`) — o plugin controla a posição e o tamanho.** `seek` faz `(base + pos).max(0)`, que estoura `i64` com `pos` próximo de `i64::MAX` (pânico em debug, envolvimento em release). `write` então faz `d.resize(at + bytes.len(), 0)` para qualquer `at`: uma posição enorme vira alocação enorme (aborta o processo host). Além disso, os métodos COM de `host.rs` não usam `catch_unwind`: um pânico dentro de `extern "C"` aborta. *Correção:* `saturating_add`/`checked_add`, limite máximo para a posição (por exemplo, o tamanho do estado mais 1 MiB) e erro `kResultFalse` acima disso; envolver os métodos COM em `catch_unwind`.

**[BAIXO] host.rs:202-211 (`setString`):** laço fixo de 65 536 caracteres (limitado, sem problema), mas o `Vec` cresce conforme a string do plugin. Sem limite total por atributo. *Correção:* limite de tamanho.

**O que está bom (vst3):** `Module::drop` libera a fábrica antes de chamar `exit` e depois faz `dlclose` (ordem correta); `load_state` valida o tamanho com `chunk.get(8..8+n)`; `Stream::read` limita a cópia; `ComRef::from_raw` é checado contra nulo.

### lm-sys-ladspa

Lidos: `Library` (abertura, `Drop`), `Instance::new`, `process`, `Drop`.

Sem achado de memória. Os buffers de controle são um `Box<[f32]>` de `max(ports, 1)` elementos, liberado em `Drop` depois do `cleanup`. As entradas e saídas são ligadas a buffers do chamador com comprimento verificado (`b.len() >= frames`), e as ausentes apontam para `silence`/`sink`, do tamanho de `max_block`. `process` limita `frames` a `max_block`.

**[BAIXO] lib.rs:296-341 (`Instance::new`):** o `controls` é alocado antes de `instantiate`; se `instantiate` falhar, ele é liberado (correto), mas o `Descriptor` lido de `info.controls()` acessa `values[p.index]` sem checagem explícita (depende de `index < ports`, que vale pela construção). Registrar a invariante.

### lm-sys-samplerate

Lido por completo.

**[BAIXO] lib.rs:201-204 e :192-193 (`convert`):** `offset += done.input_frames` confia que libsamplerate nunca consome mais do que o bloco; se consumir, `&block[offset*channels..]` entra em pânico. *Correção:* checar `done.input_frames <= restante`.

**[BAIXO] lib.rs:276-277 (`measure_latency`):** `vec![0.0; (PROBE*ratio)*channels]` sem limite de `channels` (aceita qualquer `c_int > 0`). *Correção:* limite superior de canais (por exemplo, 64).

**O que está bom (samplerate):** `Resampler` não é `Sync`; `Drop` chama `src_delete` uma vez; `run` monta `SRC_DATA` com contagens de quadros convertidas com `try_from`; `process` não aloca; `convert` checa cancelamento a cada bloco e o tamanho de saída é exato.

### lm-sys-rubberband

Lidos: `Handle` (Drop com `LIFECYCLE`), `Offline` (`study`, `process`, `drain`, `finish`), `Realtime` (`process`, `retrieve`, `available`).

Sem achado de memória: `drain` limita `keep` a `output_frames - emitted` (sem subtração que estoure, pois `emitted <= output_frames` por invariante); os ponteiros de plano têm `OFFLINE_BLOCK` elementos; `Realtime::process` confere `frames <= max_block` e forma iguais; `retrieve` limita a `available()`.

**[BAIXO] lib.rs:530-535 e :549-552:** o código confia que `rubberband_retrieve` devolve no máximo `want`; se devolver mais, `plane[f]` entra em pânico (não é UB). *Correção:* `got.min(want)`.

### lm-sys-sndfile

Lidos: `Reader` (abertura, leitura, seek, Drop), `Writer` (criação, escrita, `close` com `ManuallyDrop`, `Drop`), `fix_ogg_serial`, preallocação para Linux e macOS.

**[ALTO] lib.rs:371 — vazamento do `SNDFILE` quando `Info::from_raw` falha.** `handle` é `NonNull` (sem `Drop`), e o `?` sai antes de `Reader` ser montado. O descritor `File` é fechado, mas o handle da libsndfile e sua memória não são liberados. *Correção:* montar `Info` antes de `open_fd` (com o `raw` preenchido) ou chamar `sf_close(handle)` no caminho de erro.

**Verificado sem achado — lib.rs (`fix_ogg_serial`, L~740-750):** o `Vec` `page` é redimensionado a `size` que vem do próprio arquivo, limitado a ~65 KB por página pelo formato Ogg.

**O que está bom (sndfile):** `Info::from_raw` valida taxa e canais (divisão por zero impossível em `check_buffer`); `close` usa `ManuallyDrop` e não fecha duas vezes; `open_fd` usa `close_desc = 0` (o `File` dono do descritor); a preallocação confere `fst_*` (layout de `fstore_t` correto em macOS) e trata `EOPNOTSUPP`/`ENOSYS`.

### lm-sys-fpu

Lido por completo. As instruções `stmxcsr`/`ldmxcsr` (x86-64) e `mrs`/`msr fpcr` (aarch64) mexem só nos bits FTZ/DAZ/FZ, preservando os demais, e declaram as opções corretas (`nostack`, `preserves_flags`, `readonly` em `ldmxcsr`). Sem achado.

### lm-sys-gui

Revisão por grep e leitura de `x11.rs` (conexão, `pump`, `Drop`).

**[MÉDIO] x11.rs:51 — `unsafe impl Sync for Connection`.** A justificativa é "só a thread principal usa" (convenção, não garantia). Como o `static CONNECTION` exige `Sync`, o tipo do chamador não impede o uso a partir de outra thread. *Correção:* guardar a conexão em `thread_local!` ou em um `Mutex` com o `Display` (que não é `Send` por natureza), ou restringir a API ao thread principal com verificação em tempo de execução.

**[BAIXO] x11.rs:55 e :216-226:** `CLOSED` (`Mutex<Vec<Window>>`) só remove entradas em `take_closed`. Janelas fechadas e nunca consultadas ficam no vetor, e o id de uma janela destruída pode ser reaproveitado pelo servidor X e marcar uma janela nova como fechada. *Correção:* remover a entrada no `Drop` de `Native` e limitar o tamanho.

**Não verificado por completo:** `cocoa.rs` (chamadas `objc_msgSend` via `transmute`, 232 linhas). Os tipos de assinatura batem com os usados (`Rect`, `Size`, `f64`, `u64`), mas não foram checados um a um.

### lm-sys-nam

Lidos: `shim.cpp` por completo e `lib.rs` (`Model::reset`, `process`, `Drop`).

Sem achado. `lm_nam_process` é `noexcept` e captura tudo; `lm_nam_load` e `lm_nam_reset` capturam exceções e copiam a mensagem com limite; `Model::process` divide em pedaços de `max_frames` (que coincide com o `reset`), e `frames` é convertido com `c_int` no mesmo limite. Pequeno ponto:

**[BAIXO] shim.cpp:43 (`nlohmann::json::parse(json, json + len)`):** com `json == NULL` e `len == 0`, a aritmética de ponteiro é tecnicamente indefinida (o `parse` lança erro depois, então o efeito é nulo). *Correção:* tratar `json == nullptr` antes.

### lm-sys-resid

Lidos: `shim.cc` por completo e `lib.rs` por grep.

Sem achado de memória no caminho de áudio: `lm_resid_clock` escreve no máximo `n` amostras e `Sid::clock` passa `out.len()` limitado a `c_int::MAX`. `copyFrom` refaz os ponteiros de sincronização de voz dentro do próprio chip.

**[MÉDIO] lib.rs:162 (`Clone` da cópia de `Sid`):** `Self::new(...).unwrap_or_else(|e| panic!(...))`. Se a cópia for feita no thread de áudio (ou se `new` falhar por falta de memória), o pânico atravessa o código do chamador. *Correção:* devolver `Result` na cópia e não entrar em pânico.

### lm-sys-guitarix

Lidos: `src/lib.rs` (`make`, `new`, `start`, `update`, `set`, `output`, `process_mono`, `process_stereo`, `clear`, `Drop`), `glue/shim.cpp` por completo, `GuitarixUnits.h` e `GuitarixUnits.cpp` (bloco por thread: `thread_local t_blockFrames`, correto para `start`/`update`, que rodam na mesma thread). Os demais `glue/*.cpp` (~1.800 linhas de DSP) foram lidos por grep de alocação (`new`, `vector`, `resize`), sem achado novo.

**[CRÍTICO] src/lib.rs:408 e :418 — o tamanho do bloco não é o iniciado.** Veja o resumo (item 3). `process_mono` afirma "at most the block it was started with" na documentação, mas a asserção usa `MAX_BLOCK`. Os processadores C (Detune, Convolution) fixam tamanhos no `start` (`numSampsToProcess`, `m_block`). *Correção:* `assert!(input.len() <= self.block as usize)` (e exigir que `start` tenha sido chamado); idem `process_stereo`. Se `start` nunca foi chamado, `block == 0` e a asserção deve falhar.

**[MÉDIO] src/lib.rs:303-316 (`Instance::make`):** `panic!` quando `lmgx_make` devolve nulo, em vez de `Result`. *Correção:* devolver `Result` e propagar com `?` em `new`.

**[MÉDIO] glue/shim.cpp (`lmgx_register`, L117-119; globais `t_register`/`t_context`):** o registro usa variáveis globais sem sincronização, e `lmgx_make` pode lançar `std::bad_alloc` dentro de `extern "C"` (que termina o processo). Hoje o registro é só da thread principal, mas a garantia não é imposta. *Correção:* `thread_local` para `t_register`/`t_context`, e `try`/`catch` em `lmgx_make` e nas funções de registro.

**Nota (guitarix `set`/`output`, não é achado):** escrevem/leem as variáveis C; a segurança depende de a engine não usar a mesma instância em dois threads ao mesmo tempo. `Instance` é `Send`, não `Sync`, o que não impede isso entre instâncias diferentes. Registrar essa dependência no safety sheet.

**Verificado sem achado:** `Drop` chama `lmgx_delete` uma vez; `lmgx_delete` chama `activate_plugin(false)` antes de `delete_instance`; `register_param` está em `catch_unwind`; `clamp` do bloco ao `MAX_BLOCK`; `value.round() as c_int` é saturante (NaN vira 0).

### lm-sys-lilv (complemento)

Ver a seção acima. O `world.rs` (527 linhas) foi lido nos trechos de `Core`, `Drop`, índices de porta e `Shared`; a parte de leitura de metadados (`PluginInfo`) não foi lida linha a linha.

### lm-sys-vst3-test

Lido: 6 linhas, apenas reexporta. Sem achado.

---

## Índice rápido dos BAIXO

Cada item abaixo já está descrito, com correção, no corpo por crate (não somar de novo):
crash.rs (código morto em `arm`), reserve.rs (parâmetro e retorno ignorados), bus.rs (`release`), worker.rs (`scheduled`), instance.rs (CV sem limite, buffers ligados após `run`), clap (`deinit` no último `Arc`; MIDI `cc >= 128`), vst3 (`setString`; MIDI `cc >= 128`), samplerate (contagens e canais), rubberband (`got` > `want`), nam (`json` nulo), x11 (lista `CLOSED`), alsa (`mmap_commit`), ipc (`unique_name`), ladspa (invariante de `controls`), coreaudio (`get_vec`), jack (`silent`).

---

## O que está bom (padrões corretos, em geral)

- **Trampolins de C:** todos os callbacks lidos (jack, clap, vst3 host, lilv `map`/`unmap`/`schedule_work`/`respond`/`store`/`retrieve`, coreaudio, nam, resid) capturam pânico com `catch_unwind` ou `guard`. Nenhum pânico cruza para C nos trechos lidos.
- **Ordem de liberação:** `lilv::Instance::drop` (worker unido, desativação, `lilv_instance_free`, só então `host`/`schedule`); `clap::Instance::drop` (`destroy` antes de liberar o host); `jack::ActiveClient::drop` (cliente fechado antes do estado); `sndfile::Writer::close` (`ManuallyDrop`, sem dupla liberação).
- **Tipos `Send`/`Sync`:** os handles de ALSA, JACK, sndfile, samplerate e rubberband são `Send` e não `Sync`, com uso exclusivo por `&mut self`; os tipos de sinal (`Waker`, `MidiStopper`, `MidiWaker`) são `Send + Sync` e contêm só atômicos e um eventfd.
- **Áudio em tempo real (ALSA, JACK, nam, resid, rubberband, samplerate):** os buffers de transferência são alocados fora do caminho de áudio (`configure`, `Resampler::new`, `Offline`), e as chamadas de processamento não alocam.
- **Limites de comprimento em C:** nos caminhos lidos, `lengths` passados a C vêm de `min(...)` ou de checagens explícitas (ALSA `write_src`/`read_dst`, JACK `read_audio`/`write_audio`, samplerate `run`, nam `process`, resid `clock`).
- **Strings:** nomes passados a C são `CString` com o tempo de vida cobrindo a chamada; strings de C copiadas antes de liberar o objeto dono (JACK `copy_str`, ALSA `text`, vst3 `text`).
- **Fundo do fpu e do ipc:** as instruções de `fpu` mexem só nos bits necessários; `ipc` só expõe `AtomicU32`.

## Cobertura desta varredura

- **Lidos por completo:** `lm-sys-ipc`, `lm-sys-linux` (todos os arquivos de `src/`), `lm-sys-alsa` (`src/`, exceto a lista de declarações de `ffi.rs`, conferida por amostragem), `lm-sys-jack` (`src/`), `lm-sys-samplerate`, `lm-sys-fpu`, `lm-sys-nam` (`shim.cpp` e `lib.rs` relevante), `lm-sys-resid` (`shim.cc`).
- **Lidos em parte (trechos de risco):** `lm-sys-lilv`, `lm-sys-clap`, `lm-sys-vst3`, `lm-sys-ladspa`, `lm-sys-sndfile`, `lm-sys-rubberband`, `lm-sys-coreaudio` (`mac.rs`), `lm-sys-gui` (`x11.rs`), `lm-sys-guitarix` (`lib.rs` e `shim.cpp` completos; DSP em grep).
- **Não lidos:** `lm-sys-clap/src/ffi.rs` (declarações, cobertas pelo `tests/layout.rs`), `lm-sys-lilv/src/ffi.rs`, `lm-sys-jack/src/ffi.rs`, `lm-sys-coreaudio/src/ffi.rs`, `lm-sys-gui/src/cocoa.rs`, `lm-sys-vst3/src/testing.rs` (grep apenas), e os blocos de parâmetros/portas de `clap` e `vst3`. Os arquivos de teste (`tests/`) e os `vendor/` de terceiros não entraram na varredura.
- **Safety sheets (`lib.rs`):** no geral batem com o código lido. Divergências: `lm-sys-jack` afirma que o estado nunca é usado após o erro de `activate` (contradito por C1); `lm-sys-guitarix` afirma que os processos aceitam "at most the block it was started with" (não imposto, C3); `lm-sys-ipc` não menciona o risco de SIGBUS por encolhimento do objeto (A6).
- **Fora desta área:** `unsafe` em crates que não são `lm-sys-*` não foi verificado (a regra do projeto proíbe, e isso cabe à varredura das outras áreas).

## Resultado da confirmação

Só `cargo check` (macOS). Os trechos só de Linux (`lm-sys-linux/src/crash.rs`, `lm-sys-alsa/src/linux/*`) ficaram SEM `cargo check` (o alvo Linux instalado não tem a `core`); foram revisados à mão.

| Achado | Veredito | Commit |
|---|---|---|
| CRÍTICO jack `Client::activate` (uso após liberação) | confirmado e corrigido: os callbacks são desregistrados (permitido com o cliente inativo) antes de liberar o `State`; se algum não puder, o estado é vazado. Sem teste (exige servidor JACK) | ver `git log` (lm-sys-jack) |
| CRÍTICO coreaudio `Io::drop` | confirmado e corrigido: o estado só é liberado se a parada e a destruição do IO proc deram certo; senão é vazado. Sem teste (exige dispositivo) | lm-sys-coreaudio |
| CRÍTICO guitarix `process_mono`/`process_stereo` | confirmado e corrigido: a asserção compara com o bloco iniciado em `start`; teste `a_block_larger_than_the_started_one_is_refused` | lm-sys-guitarix |
| ALTO coreaudio `Watch::drop` | confirmado e corrigido: a closure só é liberada se a remoção deu certo ou o dispositivo sumiu | lm-sys-coreaudio |
| ALTO linux `install_handlers` (recursão) | confirmado e corrigido (sem `cargo check`): desfaz os handlers já trocados; `chain` ignora o próprio `on_signal`; `write_all` sai com `n == 0`. Sem teste (sinais reais) | lm-sys-linux |
| ALTO sndfile `Reader::open` (vazamento) | confirmado e corrigido: `sf_close` no caminho de erro. Sem teste (libsndfile não aceita arquivo sem canais) | lm-sys-sndfile |
| ALTO vst3 `Stream::write`/`seek` | confirmado e corrigido: limite de 256 MiB, `checked_add`, erro acima disso; teste em `host.rs`. O `catch_unwind` nos métodos COM não foi feito | lm-sys-vst3 |
| ALTO ipc `SharedMemory::open` (par encolhe o objeto) | confirmado e não corrigido: o par é do mesmo usuário e o macOS não tem selos de `memfd`; a correção certa é mudar o desenho do canal (selos no Linux) | - |
| ALTO lilv `urid:map` com mutex | falso: a especificação LV2 diz que `map` não é de tempo real e plugins mapeiam na instanciação; `try_lock` devolvendo 0 quebraria instanciações simultâneas | - |
| MÉDIO midi `as_slice`, `value + 8192` | confirmado e corrigido, com teste | lm-sys-alsa |
| MÉDIO `pcm_hints` nulo, `PcmPoll::new` (`got > count`), `Instant` em seq | confirmado e corrigido (sem `cargo check`; sem teste, exige ALSA) | lm-sys-alsa |
| MÉDIO clap `stream_write` | confirmado e corrigido (64 MiB), com teste | lm-sys-clap |
| MÉDIO lilv `state::store` (limite) e `wait_idle` (worker morto) | confirmado e corrigido; sem teste (exige plugin LV2) | lm-sys-lilv |
| MÉDIO jack `port_name`/`latency` com índice | corrigido na rodada de desenho: `Port` carrega o id do cliente; porta alheia dá vazio/zero | rodada de desenho (lm-sys-jack) |
| MÉDIO guitarix `make` (`panic!`), shim.cpp (exceções) | corrigido na rodada de desenho: `make`/`start`/`update` fallíveis, `catch (...)` no shim | rodada de desenho (lm-sys-guitarix) |
| MÉDIO sid `Clone` (`panic!`) | corrigido na rodada de desenho: `Sid::try_clone`, `Clone` aborta como falta de memória | rodada de desenho (lm-sys-resid) |
| MÉDIO linux `prefault_stack`, `RealtimeGuard::drop`, rtkit sem reverter | corrigido na rodada de desenho (sem `cargo check`, só Linux) | rodada de desenho (lm-sys-linux) |
| MÉDIO crash `Probe`, `write_all` | `Probe` corrigido na rodada de desenho: tabela de atômicos (`write_all` já na primeira) | rodada de desenho (lm-sys-linux, lm-render, lm-session) |
| MÉDIO x11 `Sync` | corrigido na rodada de desenho: só a thread dona recebe a conexão (sem `cargo check`, só Linux) | rodada de desenho (lm-sys-gui) |
| MÉDIO `PcmPoll::wait` | corrigido na rodada de desenho: conjunto diferente é erro em release (sem `cargo check`, só Linux) | rodada de desenho (lm-sys-alsa) |
| ALTO vst3 `catch_unwind` nos métodos COM | corrigido na rodada de desenho: `guard` em todos os 40 métodos de `host.rs`; jack e coreaudio passaram a cobrir o corpo inteiro | rodada de desenho (lm-sys-vst3, jack, coreaudio) |
| BAIXOS (jack hooks, samplerate, rubberband, nam, crash `arm`, lilv `schedule_work`/`max_block`, reserve, vst3 CC, x11 `CLOSED`, clap `deinit`) | corrigidos na rodada de desenho; o resto (bus `release`, mmap `?`, `unique_name`, `setString`) mantido com motivo em `decisoes-autonomas.md` | rodada de desenho |
