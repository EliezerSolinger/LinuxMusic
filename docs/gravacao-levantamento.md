# Gravação multitrack: levantamento

Pedido do usuário (12/11/2026): preparar o terreno para gravação multitrack de verdade, como no REAPER, em etapas:
1. investigar, limpar e corrigir os problemas estruturais do LMMS no núcleo e na interface;
2. levantar todos os requisitos e modelos desejados;
3. arquitetar um plano técnico;
4. implementar um sistema de gravação e faixas de gravação limpos.

Este documento é o levantamento que abre a etapa 1. Ele junta cinco pesquisas, que estão nos anexos A a E com citações
`arquivo:linha` e URLs.

**Decisões já tomadas:**
- **Drivers:** ALSA e JACK, com o ALSA primeiro. "Se conseguirmos o buffer ALSA puro exclusivo para gravação já
  está ótimo."
- **Entrada, armar e monitorar** são da faixa, nunca do canal da mesa (passo 26). A mesa só mistura.
- **A gravação fica na página Composição.** A mixagem é uma página separada.
- **Do plano da etapa 3** (`etapa-3-gravacao.md`):
  - um tipo novo de faixa, a faixa de gravação;
  - gravação direto em arquivo no disco, numa thread de disco;
  - nada de alocação, trava ou disco na thread de áudio;
  - a captura só abre quando há faixa armada ou monitorando.

---

## 1. Como o sistema é hoje

### Motor de áudio (anexo A)
- **Quem renderiza é o driver:** a thread do callback chama `renderNextBuffer`. O período interno é de no máximo 256
  frames, desacoplado do buffer do driver.
- **O motor soma 1 período de latência:** a saída é o período renderizado na chamada anterior.
- **`renderNextPeriod` trava `m_changeMutex` o período inteiro**, o mesmo mutex que a interface usa em
  `requestChangeInModel`. Editar na interface bloqueia o callback de áudio.
- **A entrada é um único fluxo estéreo:**
  - os drivers empurram para um ring de 25.600 frames;
  - a cada período o motor drena até 512 frames, sem carimbo de tempo;
  - **hoje ninguém lê a entrada**: a gravação herdada foi removida, o ring transborda e os avisos saem com
    `fprintf` direto da thread de tempo real.
- **Drivers:**

  | Driver | Captura | Observação |
  |---|---|---|
  | JACK | sim | 2 portas, entrada um ciclo atrasada |
  | SDL | sim | dois relógios independentes |
  | PortAudio | não | abre a entrada e descarta |
  | **ALSA**, PulseAudio, sndio | **não** | só tocam |

- **Nenhum driver informa latência**, e `AudioDevice` não tem API para isso.
- **Não existe contador de frames do transporte:** a posição é tick (192 por compasso) mais um offset `float`.
- **O PDC da mesa existe** (`Mixer::addLatencySource`, `LatencyDelay`). Ele alinha os canais entre si, mas
  `Mixer::latency()` não é usado em lugar nenhum.

### Modelo de faixas, clipes e áudio (anexo B)
- **Tudo em ticks:** posição, comprimento, offset, fades e loop são ticks (~10 ms a 120 BPM). A reprodução só
  começa em fronteira de tick.
- **Sem mapa de tempo:** um BPM global inteiro, e a fórmula de compasso global.
- **O `SampleClip` fica preso ao BPM:** mudar o andamento corta o áudio ou cria silêncio.
- **O `SampleBuffer` guarda o arquivo inteiro na RAM** em float estéreo. Mono vira estéreo, e mais canais são
  descartados. Uma hora mono ocupa ~3,2 GB, mais a miniatura (~1,33× o áudio).
- **Reprodução:** um `new SamplePlayHandle` por clipe, na thread de áudio. Clipes sobrepostos são somados. Toda
  edição reinicia a reprodução da faixa.
- **Desfazer é um snapshot XML do objeto inteiro**, às vezes com o áudio em Base64. Desfazer numa faixa recria e
  redecodifica os clipes com o áudio travado.
- **Arquivos:**
  - não há pasta de mídia do projeto;
  - um arquivo ausente é apagado do projeto no próximo salvamento;
  - clipe sem arquivo embute o áudio em Base64.
- **O que existe para gravar:** a entrada, armar e monitorar do `SampleTrack`, só como dados.

### Interface (anexo C)
- **Nenhuma interface de gravação de áudio.** Os botões de gravar da barra só gravam MIDI no clipe aberto no Piano
  Roll e, sem clipe aberto, não fazem nada nem avisam. O Song Editor não tem botão de gravar.
- **Não há modelo de transporte global:** a barra chama ações dos editores. O núcleo lê `PianoRoll::m_recording`
  na thread de áudio.
- **Cabeçalho da faixa:** uma linha de 32 px, com 78 px fixos à esquerda (`TRACK_OP_WIDTH`).
- **Redesenho em massa:** cada rolagem recria a grade de cada faixa e redesenha a forma de onda inteira de cada
  clipe, sem recortar o que está visível. Com a música tocando, isso ocorre ~96 vezes por segundo, sem o limite de
  60 Hz.
- **A miniatura da forma de onda é imutável e enorme** (8 bytes por frame), e é calculada travando o motor. Não
  serve para mostrar a gravação crescendo.
- **Configurações de áudio:** não têm latência nem compensação, toda mudança exige reiniciar, e o ALSA fica preso
  em 2 canais.
- **Duplicação:**
  - o menu de canal está repetido em `SampleTrackView` e `InstrumentTrackView`;
  - o cabeçalho é montado à mão em três views;
  - há CSS paralelo nos dois temas.

### Histórico (anexo D)
- **Upstream:** o LMMS nunca teve gravação de áudio. Foram cinco PRs abandonados desde 2018; o último está parado
  desde o fim de 2025. O bloqueio era guardar a gravação em Base64 dentro do projeto.
- **Consenso do upstream:** armar por faixa, entrada por faixa, um clipe novo por take, e uma camada de E/S
  (`IOManager`) entre os drivers e as faixas.
- **Neste fork:**
  - a branch `sample-track-recording-jack` está obsoleta;
  - da `feature/recording-stage-one` sobraram a captura do JACK e o ring;
  - a gravação herdada foi removida no `d95c6ba49`.
- **Para reaproveitar:**
  - a captura do JACK, refeita para N canais e aberta sob demanda;
  - a escolha de portas do JACK (#7919);
  - a ideia do ring (#7903), com carimbo de tempo e multicanal;
  - o PDC da mesa;
  - os fades de clipe (#7900);
  - o `AudioInput` e os modelos de armar e monitorar.

---

## 2. Problemas estruturais (etapa 1: limpar antes de construir)

Em ordem de prioridade para a gravação. Cada item está documentado nos anexos.

### Núcleo, tempo real
1. **Mutex global no período** (`m_changeMutex`): a interface bloqueia o áudio. A gravação não pode perder
   amostras porque alguém editou na tela.
2. **Alocações e E/S na thread de áudio:**
   - `new SamplePlayHandle`;
   - `LatencyDelay::setDelay`;
   - o `fprintf` do overflow;
   - o resampler criado no play;
   - o evento Qt `nextAudioBuffer`;
   - as conexões diretas que rodam lógica de tempo.
3. **Flags de transporte `volatile bool` escritas pela interface**, mais o bug `Song::stop()` → `m_recording =
   true`.
4. **Faltam um contador absoluto de frames do transporte e posições em amostra.** Sem eles não há gravação
   alinhada nem compensação de latência.

### Núcleo, entrada e drivers
5. **ALSA não captura.** É prioridade, e precisa de acesso exclusivo e captura sincronizada com a reprodução: o
   mesmo período, o mesmo relógio, e de preferência o mesmo dispositivo em duplex.
6. **A entrada é só estéreo**, sem carimbo de tempo, drenada fora de sincronia, e sempre aberta.
7. **Nenhum driver informa latência**, e não há API para abrir e fechar a captura nem para listar canais.
8. **Bugs:** o `tlength` do PulseAudio, o dispositivo configurado ignorado no PulseAudio, o código morto
   `AUDIO_BUS_HANDLE_SUPPORT` e a porta de entrada do JACK não conferida.

### Modelo
9. **Áudio inteiro na RAM, sem streaming.**
10. **Clipes de áudio presos ao BPM, e nenhum mapa de tempo.**
11. **Desfazer por snapshot XML** (e redecodificação com o áudio travado).
12. **Arquivos:** nenhuma pasta de mídia, áudio embutido em Base64, e referências apagadas quando o arquivo some.

### Interface
13. **Redesenho em massa** na rolagem e na reprodução, e a forma de onda sem recorte.
14. **Miniatura imutável e enorme**, calculada travando o motor.
15. **Não há modelo de transporte**, e o estado de gravação fica dentro de uma view (`PianoRoll::m_recording`).
16. **Duplicação nos cabeçalhos de faixa**, constantes fixas e sinais no estilo antigo.

---

## 3. O que o REAPER faz (anexo E), e o que propomos

### Na faixa
- **Armar.** No REAPER também dá para armar uma faixa com a gravação já rodando.
- **Entrada:** nenhuma, mono, estéreo ou multicanal a partir de um canal, ou MIDI.
- **Modo de gravação:** a entrada; a saída da faixa (antes ou depois dos efeitos, antes ou depois do fader, com ou
  sem compensação de latência); overdub ou replace de MIDI; ou desligado ("só monitorar").
- **Monitorar:** desligado, ligado ou "tape auto", em que a entrada só é ouvida parado ou gravando.
- **Efeitos de entrada**, gravados de vez no arquivo, e **efeitos de monitoração**, que nunca são gravados.
- **Medidor de entrada.**

### Gravação global
- **Modos:** normal, auto-punch por seleção de tempo e auto-punch pelos itens selecionados.
- **Ao gravar por cima de algo:** criar takes, cortar o antigo (modo fita), criar lanes ou misturar.
- **Loop recording:** cada volta vira um take. Há um limite de 90% para descartar voltas incompletas.
- **Lanes fixas e comping** (REAPER 7): uma lane de comp é montada copiando pedaços das outras.
- **Pré-roll e count-in**, separados, e o metrônomo.
- **Parar:** pelo Record, salva; pelo Stop, pergunta se salva ou apaga.

### Arquivos
- **Caminhos:** um primário e um secundário, com backup invisível.
- **Formato e profundidade de bits** por faixa.
- **Nomes com variáveis** (`$recpass`, `$tracknumber`…), e troca de arquivo a cada X MB sem buraco.
- **Parar se o disco encher ou falhar.**
- **Ardour, para comparar:** um arquivo mono por canal, em float de 32 bits, e um buffer de disco em segundos
  esvaziado por uma thread de disco.

### Latência
- **A latência informada pelo driver mais offsets manuais** de entrada e de saída, calibrados com um teste de
  loopback (ping).
- **Ardour:** a primeira amostra gravada = início da captura + latência de entrada + offset da reprodução.
- **Com PDC, a monitoração fica atrasada.** Para isso há o bypass automático de plugins com latência ao armar.

### Recomendação de modelo (dos anexos B e E)
- **Gravar sempre a passada inteira em arquivo.** Punch, takes de loop e lanes são janelas não destrutivas sobre
  esse arquivo.
- **Um item de áudio com tempo absoluto, em amostras**, desacoplado do BPM, com o modo "musical" opcional.
- **Um pool de mídia do projeto:** a pasta `media/`, caminhos relativos, estado offline e leitura em streaming do
  disco com cache de picos em vários níveis.
- **Reprodução por snapshot imutável:** a thread de áudio lê uma estrutura que a interface troca atomicamente, sem
  mutex e sem alocar.

---

## 4. Modelos de gravação possíveis

| Modelo | O que é | Prós | Contras |
|---|---|---|---|
| **A. Faixa de gravação nova** | Um tipo novo, com itens em amostras, lanes e takes. O `SampleTrack` continua para loops | Limpa, sem amarras do legado; é o que o plano da etapa 3 diz | Dois tipos de faixa de áudio por um tempo |
| **B. Evoluir o `SampleTrack`** | Itens em amostras, lanes e streaming no próprio `SampleTrack` | Um tipo só | Carrega o legado (ticks, RAM, desfazer); quebra loops existentes |
| **C. Faixa de gravação que vira a faixa de áudio** | Começa como A; quando cobrir loops e samples, o `SampleTrack` sai e os projetos convertem | Limpa e termina com um tipo só | Mais trabalho no fim |

**Dentro de qualquer um, por take:**
- **Arquivo:** um por faixa e passada (intercalado, como o REAPER, por inferência) ou um mono por canal (Ardour).
- **Formato:** WAV float de 32 bits (Ardour), WAV 24 bits ou FLAC.
- **Takes:** lanes empilhadas, cada passada uma lane, como no REAPER 7; ou takes dentro do item, como no REAPER
  antigo.

---

## 5. Caminho do ALSA puro exclusivo (prioridade do usuário)

O que o ALSA puro precisa, a partir do anexo A:
- **Abrir o dispositivo em duplex** (`hw:`), reprodução e captura no mesmo dispositivo, com o mesmo período e o
  mesmo relógio, com `snd_pcm_link` para começarem juntos.
- **Acesso exclusivo:** `hw:` direto, sem `dmix`/`dsnoop` e sem o PulseAudio ou o PipeWire no meio. Isso dá a menor
  latência e a latência mais previsível.
- **Formato nativo do dispositivo:** 24 bits ou 32 bits, intercalado ou não, convertido para float no período.
- **Latência conhecida:** `snd_pcm_delay` e o tamanho de período e buffer dão a latência de entrada e de saída que
  hoje nenhum driver informa.
- **Multicanal de verdade:** os N canais de entrada da interface entregues ao motor por período, cada faixa lendo
  os seus (`AudioInput.firstChannel`/`channelCount`).
- **Recuperação de xrun** (`snd_pcm_recover`), contada e mostrada, e marcada no take.

O JACK vem depois, pelo mesmo caminho: N portas de entrada, a latência das portas (`jack_port_get_latency_range`),
e a captura no mesmo ciclo.

---

## Anexos
- **A:** motor de áudio e caminho da entrada
- **B:** modelo de faixas, clipes e áudio
- **C:** interface
- **D:** histórico, branches e upstream
- **E:** gravação no REAPER (e no Ardour)

---

## Anexo A: 01 — Motor de áudio e caminho de entrada (levantamento somente leitura)

Repositório: `/Users/castrot1/Desktop/LinuxMusic`, branch `master` (HEAD `2395a2c0d`). Todas as referências são
`arquivo:linha` desse estado. O que é suposição está marcado como **[suposição]**.

---

### 1. AudioEngine

#### 1.1 Modelo de threads: quem chama o quê

O motor é **puxado (pull) pelo driver**. Não existe thread própria do motor: quem renderiza é a thread do callback
do driver (JACK, SDL, PortAudio), uma `QThread` do próprio backend (ALSA, PulseAudio, sndio, Dummy) ou a thread de
exportação (`ProjectRenderer`).

- `AudioEngine::renderNextBuffer(view)` (`include/AudioEngine.h:296-299`, implementação template em `:326-357`)
  copia frames de `m_outputBufferRead` para o buffer do driver; quando o índice de leitura volta a 0 chama
  `renderNextPeriod()` (`:330-331`). Ou seja, o tamanho do buffer do driver é desacoplado do período interno.
- `AudioEngine::renderNextPeriod()` (`src/core/AudioEngine.cpp:414-440`):
  1. desliga denormais por thread (`:421`);
  2. **trava `m_changeMutex` (std::recursive_mutex) por todo o período** (`:424`);
  3. marca a thread como de áudio (`ThreadCheck::AudioThreadScope`, `:425`) e `s_renderingThread = true` (`:428`);
  4. estágios: `renderStageNoteSetup` (`:274-333`), `renderStageInstruments` (`:337-354`),
     `renderStageEffects` (`:358-391`), `renderStageMix` (`:395-410`);
  5. retorna `{m_outputBufferRead, m_framesPerPeriod}` (`:439`).
- Worker threads: `QThread::idealThreadCount() - 1` workers (`AudioEngine.cpp:93`), criados com
  `TimeCriticalPriority` (`:114-122`); o último "worker" não é iniciado e roda inline na thread que renderiza
  (`src/core/AudioEngineWorkerThread.cpp:147-155`). Fila global estática `globalJobQueue` (`:40`), acordada por
  `QWaitCondition::wakeAll()` (`:149`); cada worker espera numa `QMutex` local (`:165-171`). A espera do fim dos jobs é
  *busy-wait* (`:98-101`).
  - Estágio 1: play handles (notas, samples, instrumentos) em paralelo (`AudioEngine.cpp:352-353`).
  - Estágio 2: `AudioBusHandle`s (efeitos por faixa) em paralelo (`:363-364`).
  - Estágio 3: `Mixer::masterMix` monta a fila com dependências entre canais (`src/core/Mixer.cpp:1147-1183`).
- `Song::processNextBuffer()` roda **dentro** do estágio 0, na thread de áudio (`AudioEngine.cpp:323`), e é quem
  avança a posição da música e chama `Track::play()` de cada faixa (`src/core/Song.cpp:207-352`).
- Exportação: `ProjectRenderer::run()` troca o device pelo de arquivo e chama `renderNextPeriod()` em loop numa
  `QThread` própria (`src/core/ProjectRenderer.cpp:136-176`).

#### 1.2 Período e buffer

- `m_framesPerAudioBuffer` = config `audioengine/framesperaudiobuffer`, limitado a [32, 4096]
  (`AudioEngine.cpp:80-83`, constantes em `include/AudioEngine.h:55-57`).
- `m_framesPerPeriod = min(framesPerAudioBuffer, 256)` (`AudioEngine.cpp:84`). **O período interno nunca passa de
  256 frames**; buffers maiores do driver são atendidos com vários períodos por callback.
- Ambos são fixos após a construção (`BufferManager::init`, `:108`); mudar exige reiniciar (aviso de restart em
  `src/gui/modals/SetupDialog.cpp:595`, `:618`).
- **Latência extra de 1 período dentro do motor:** `swapBuffers()` troca read/write no começo do período
  (`AudioEngine.cpp:316`, `:442-450`), o mix é escrito em `m_outputBufferWrite` (`:400`), e o que se devolve/toca é
  `m_outputBufferRead`, ou seja, **o período renderizado na chamada anterior** (`:439`). A exportação confirma isso
  descartando o primeiro buffer vazio (`ProjectRenderer.cpp:150-151`). Isso precisa entrar na conta da compensação de
  latência de gravação.
- Se o buffer do driver não é múltiplo de 256 (ex.: JACK 64, 128, 1000), o `renderNextBuffer` guarda o resto do
  período em `m_outputBufferRead` entre callbacks (`include/AudioEngine.h:330-355`): a latência efetiva depende da
  relação entre os dois tamanhos.

#### 1.3 Sample rate

- `m_baseSampleRate` vem da config, mínimo 44100 (`AudioEngine.cpp:85-86`). A taxa usada de fato é a do device:
  `outputSampleRate()` e `inputSampleRate()` retornam `m_audioDev->sampleRate()` (`include/AudioEngine.h:195-204`) —
  **entrada e saída são supostas sempre na mesma taxa**.
- O device pode mudar a taxa (`AudioDevice::setSampleRate`, `include/AudioDevice.h:86`): o JACK adota a taxa do
  servidor (`src/core/audio/AudioJack.cpp:210`). Depois de criar o device o motor emite `sampleRateChanged()`
  (`AudioEngine.cpp:165`, `:515`, `:542`), que vários objetos escutam (Song, LFOs, plugins, `RingBuffer`...).
- **Não há reamostragem entre motor e driver.** O motor roda na taxa do driver; ALSA exige taxa exata
  (`AudioAlsa.cpp:277-285`), SDL converte internamente porque abre sem `allowed_changes` (`AudioSdl.cpp:73`, `:99`),
  PortAudio usa `baseSampleRate` e falha se não suportado (`AudioPortAudio.cpp:126`, `:151-158`).
  `AudioResampler` existe só para tocar samples (`include/Sample.h:30`, `src/core/Sample.cpp:107-119`).
- Mudança de taxa em tempo de execução pelo servidor (JACK) **não é tratada**: não há
  `jack_set_sample_rate_callback` em `AudioJack.cpp`.

#### 1.4 Entrega da saída

- Cada backend chama `renderNextBuffer` (interleaved ou planar) ou `renderNextPeriod` direto (Dummy, Pulse sem
  conexão). Mono = média L/R; >2 canais: L/R nos dois primeiros e zero no resto (`include/AudioEngine.h:333-353`).
  O motor só produz estéreo na saída (`SampleFrame` é fixo em 2 canais).
- `renderStageMix` também emite `nextAudioBuffer(m_outputBufferRead)` **da thread de áudio** (`AudioEngine.cpp:404`);
  o osciloscópio da GUI conecta com conexão automática (enfileirada) e lê o ponteiro depois
  (`src/gui/widgets/Oscilloscope.cpp:96-98`): alocação de evento Qt por período e leitura sem sincronização de um
  buffer que o motor reescreve.
- O suporte a "uma porta JACK por faixa" (`AUDIO_BUS_HANDLE_SUPPORT`) está todo atrás de um `#ifdef` que nenhum
  CMake define (`AudioJack.cpp:117`, `:323`, `:395`; `grep` não acha definição) — código morto.

#### 1.5 O modelo `requestChangeInModel`

- `requestChangeInModel()` trava `m_changeMutex`; `doneChangeInModel()` destrava; ambos são no-op na thread que está
  renderizando (`s_renderingThread`, thread_local) (`AudioEngine.cpp:676-686`). Há também o guard RAII
  `RequestChangesGuard` (`include/AudioEngine.h:76-110`, `:305-308`).
- `renderNextPeriod` segura o mesmo mutex durante todo o período (`AudioEngine.cpp:424`). Logo, **qualquer mudança
  de modelo na GUI bloqueia o callback de áudio** até a GUI terminar (e vice-versa). Usado em ~18 arquivos
  (Song, Track, Clip, Mixer, EffectChain, TrackContainer, InstrumentTrack, views da GUI, Lv2...). Exemplo:
  `Song::stop()` (`src/core/Song.cpp:649`), `Mixer::addLatencySource` (`Mixer.cpp:997-999`).
- Em JACK, bloquear o callback = xrun, e se demorar o servidor pode expulsar o cliente (há tratamento de
  "zombified", `AudioJack.cpp:136-157`).
- `s_renderingThread` só é verdadeiro na thread que chamou `renderNextPeriod`, não nos workers. Um worker que chame
  `requestChangeInModel()` travaria esperando o mutex que a thread de render segura enquanto espera o worker
  (deadlock). **[suposição]** Não encontrei caminho atual que faça isso, mas é uma armadilha para código novo de
  gravação que rode em jobs.

#### 1.6 O que roda na thread de áudio (resumo)

Leitura do ring buffer de entrada, remoção/criação de play handles (inclusive `delete` e `new SamplePlayHandle`,
`src/tracks/SampleTrack.cpp:149`), `Song::processNextBuffer` (automação, metrônomo, `Track::play`), instrumentos,
efeitos, mixer com PDC (`Mixer::compensateLatency`, que pode alocar — ver §4), monitoração do master, LFOs/
controladores, `emit nextAudioBuffer`. No JACK também MIDI (`AudioJack.cpp:384-388`).

---

### 2. Backends (`src/core/audio/*`)

Interface comum: `AudioDevice` (`include/AudioDevice.h:43-101`). Para entrada só existe `bool m_supportsCapture`
(`:57-60`, `:91`), consultada por `AudioEngine::captureDeviceAvailable()` (`AudioEngine.cpp:550-553`). **Não há
API de latência, nem número de canais de entrada, nem abrir/fechar captura sob demanda.** Ordem de tentativa
automática: SDL, ALSA, PulseAudio, sndio, JACK, PortAudio, Dummy (`AudioEngine.cpp:789-904`). Não existe backend
CoreAudio, WASAPI, ASIO nativo nem PipeWire nativo neste código; no macOS só SDL/PortAudio/JACK
(`CMakeLists.txt:188-199`).

| Backend | Captura? | Canais de entrada | Como entrega | Latência informada | Linux |
|---|---|---|---|---|---|
| JACK | sim (`AudioJack.cpp:109`) | 2 portas fixas "master in L1/R1" (`:96-98`, `:212-219`) | `pushInputFrames` no fim do callback (`:424-433`) | não | sim |
| SDL | sim, se abrir device de captura (`AudioSdl.cpp:91-112`) | 2 (força `SampleFrame`, `:184-185`) | `pushInputFrames` no callback de captura (`:183-188`) | não | sim |
| PortAudio | **não** (`m_supportsCapture` fica false) mas **abre** o stream de entrada se configurado (`AudioPortAudio.cpp:109-160`) | configurável, mas descartado | callback ignora o ponteiro de entrada (`:191-206`) | não (ignora `PaStreamCallbackTimeInfo`, `:192`) | sim |
| ALSA | não, só `SND_PCM_STREAM_PLAYBACK` (`AudioAlsa.cpp:54`) | — | — | não (`snd_pcm_delay` não usado) | sim |
| PulseAudio | não, só `pa_stream_connect_playback` (`AudioPulseAudio.cpp:147`) | — | — | não | sim |
| sndio | não, só `SIO_PLAY` (`AudioSndio.cpp:55`, `:59`) | — | — | não | sim (mais BSD) |
| Dummy | não | — | — | — | sim |

Detalhes por backend:

#### JACK (`src/core/audio/AudioJack.cpp`)
- Callback RT em `processCallback` (`:380-435`): MIDI, saída (`renderNextBuffer` planar direto nos buffers das
  portas, `:421`), **depois** copia as 2 portas de entrada para `m_inputFrameBuffer` e chama `pushInputFrames`
  (`:424-433`). Como o render vem antes, **a entrada deste ciclo só é consumida no ciclo seguinte**: +1 buffer JACK
  de atraso na entrada, além de o motor ser duplex "no mesmo clock" (o único backend realmente síncrono).
- Buffer de entrada redimensionado com `std::vector::resize` no callback de buffer size (`:197-202`, `:234-237`) —
  alocação; no JACK2 esse callback roda fora do process, **[suposição]** mas sem garantia de exclusão com o process.
- Conexões automáticas das entradas vêm da config `audiojack/input1..2` (`:308-311`); a GUI só oferece 2 entradas
  (`:512-520`).
- Sem `jack_port_get_latency_range`/`jack_set_latency_callback`, sem callback de sample rate, `stopProcessingImpl`
  vazio (`:317-319`, o callback continua rodando e zera a saída via `isRunning()`, `:412-418`).
- Mantido: 22 commits desde 2023, último 2026-11-11.

#### SDL (`src/core/audio/AudioSdl.cpp`)
- Dois devices SDL independentes (saída `:73`, captura `:99`), com **dois clocks e dois callbacks/threads**
  diferentes. Entrada e saída nunca estão sincronizadas: deriva de clock entre os devices acaba enchendo ou
  esvaziando o ring buffer **[suposição forte: é o caso típico com placas diferentes ou com o servidor de som
  reamostrando]**.
- A captura é aberta no construtor e despausada em `startProcessingImpl` (`:131-135`) **sempre**, mesmo sem nada
  armado. No macOS isso dispara o pedido de permissão de microfone; no Linux ocupa o device.
- `frames = _len / sizeof(SampleFrame)` assume float estéreo (`:184-185`); funciona porque o SDL converte para o
  formato pedido.
- Mantido: 16 commits desde 2023.

#### PortAudio (`src/core/audio/AudioPortAudio.cpp`)
- Abre stream duplex com o device de entrada configurado (`:133-160`), mas o callback descarta a entrada (`:191`
  parâmetro sem nome) e não marca `m_supportsCapture`. Ou seja: **abre o microfone e joga fora**. Ao contrário do
  que diz `docs/etapa-3-gravacao.md` ("Entrada só quando necessária"), **o PortAudio não gera os avisos de overflow**
  porque nunca chama `pushInputFrames`; quem gera é o SDL (e o JACK, se o ring encher).
- É o melhor candidato a duplex síncrono genérico (um único callback com input e output). Oferece
  `Pa_GetStreamInfo()->inputLatency/outputLatency` e `timeInfo->inputBufferAdcTime`, não usados.
- `setChannels(outputDeviceChannels)` com valor da config (`:172`); se a config estiver vazia, canais = 0 e
  `Pa_IsFormatSupported` falha **[suposição a partir do código, `:110-113`]**.

#### ALSA (`src/core/audio/AudioAlsa.cpp`)
- Só playback, thread própria `run()` com `snd_pcm_writei` bloqueante (`:223-237`), buffer = 8 períodos (`:288`):
  latência alta. Canais fixos em 2 (`:38-41`). Sem captura, sem `snd_pcm_link` para duplex.

#### PulseAudio (`src/core/audio/AudioPulseAudio.cpp`)
- Só playback (`:147`), mainloop numa `QThread` (`:168-216`). O device da config é ignorado (connect com `nullptr`,
  `:147`). `tlength` calculado como `m_latency * PA_USEC_PER_MSEC` com `m_latency` em **segundos** (`:51`, `:145`) —
  pede ~5 µs em vez de ~5 ms (bug de unidade; o servidor ajusta para o mínimo possível).
- No Linux moderno costuma ser PipeWire por baixo (pipewire-pulse); **[suposição]** para gravação multicanal com
  baixa latência a rota sensata é JACK sobre PipeWire (`pw-jack`).

#### sndio / Dummy
- sndio: só `SIO_PLAY`, conversão para int16 (`AudioSndio.cpp:131-154`). Dummy: thread que chama
  `renderNextPeriod` e dorme o tempo do período (`include/AudioDummy.h:91-105`).

---

### 3. Entrada e gravação existentes

#### 3.1 Caminho de entrada atual (do driver até o motor)

1. Driver → `AudioEngine::pushInputFrames(SampleFrame*, frames)` (`AudioEngine.cpp:176-203`): escreve num
   `LocklessRingBuffer<SampleFrame>` de capacidade `FIXED_INPUT_BUFFER_CAPACITY = 256*100 = 25600` frames
   (`:75`, `:124-125`), single-writer (`include/LocklessRingBuffer.h`, wrapper de `ringbuffer_t`).
   - Se cheio: **`fprintf(stderr, ...)` direto da thread RT** ("Critical input ring buffer overflow",
     "Partial input ring buffer overflow", "Ring buffer write issue") (`:182-202`). Esses são os avisos que inundam o
     log.
2. No começo de cada período, `processBufferedInputFrames()` (`:207-268`, chamado em `:278`) lê **no máximo 512
   frames** (`DEFAULT_BUFFER_SIZE * 2`, `:214`) do ring e copia (com `resize` dentro de capacidade reservada,
   `:246`, `:259`) para `m_inputBuffer[m_inputBufferWrite]` (dois `std::vector<SampleFrame>` reservados com 25600,
   `:103-106`). Chama `requestChangeInModel()` (`:223`, `:267`) que é no-op ali (thread de render).
3. `swapBuffers()` (`:442-450`) troca os índices: o que acabou de ser lido vira `inputBuffer()` /
   `inputBufferFrames()` do período (`include/AudioEngine.h:242-250`).
4. **Ninguém consome `inputBuffer()` hoje** (grep: só a declaração). O consumidor era o `SampleRecordHandle`,
   removido no commit `d95c6ba49`. Portanto, a entrada é capturada, drenada e descartada a cada período.

Consequências:
- O número de frames de entrada por período **varia** (0 a 512) e não está alinhado ao período de 256: com JACK a
  1024 frames, um callback faz 4 períodos; os dois primeiros pegam 512 cada (da entrada do ciclo anterior) e os dois
  últimos pegam 0. Continuidade de amostras preservada, mas a relação "frame de entrada ↔ posição da música" tem
  jitter de até um buffer do driver.
- Se o driver entrega mais de 512 frames por período de 256 de forma sustentada (clocks diferentes no SDL), o ring
  enche → overflow e perda.
- Estéreo fixo (`SampleFrame`): não há como representar entrada 3..N de uma interface multicanal.

#### 3.2 Gravação herdada (removida) — `SampleRecordHandle` / `AudioSampleRecorder`

Removidos em `d95c6ba49` ("Remove the inherited sample clip recording"; ver `git show d95c6ba49`). Como funcionavam
(para não repetir os erros):
- Era um `PlayHandle` do tipo `SamplePlayHandle` rodando nos workers; `play()` lia `inputBuffer()` e fazia
  `new SampleFrame[frames]` **por período na thread de áudio**, guardando em `QList<QPair<SampleFrame*,f_cnt_t>>`
  (memória cresce sem limite, tudo em RAM).
- No destrutor juntava tudo num `std::vector` gigante e criava um `SampleBuffer` na `inputSampleRate()`; nada era
  escrito em disco (só ao salvar o projeto, embutido).
- Alinhamento: só `m_startRecordTimeOffset` em ticks; nenhuma compensação de latência; a duração era convertida em
  ticks com `Engine::framesPerTick()` (sensível a tempo). Gravava um clipe marcado com "Set/clear record".

#### 3.3 O que existe agora para a etapa 3

- `AudioInput` (`include/AudioInput.h:33-46`): `Source {None, Hardware}`, `firstChannel`, `channelCount` (1 ou 2).
- `SampleTrack` tem `m_input`, `m_recordArmModel`, `m_monitorModel` (Off/On/Auto) (`include/SampleTrack.h:75-84`,
  `:120-122`; `src/tracks/SampleTrack.cpp:50-52`), salvos/carregados (`:186-195`, `:229-238`). **Só modelo e
  persistência: nenhum motor os usa.** O mixer não tem mais entrada própria (`src/core/Mixer.cpp:1380`, comentário).
- `firstChannel` pode apontar para qualquer canal, mas o motor só tem 2 canais de entrada e nenhum roteamento por
  canal.

#### 3.4 Modos de gravação da Song

- `Song::m_recording` é `volatile bool` (`include/Song.h:429`), ligado em `playAndRecord()` (`Song.cpp:528-532`).
  `playSong()` zera (`:506`) e **`stop()` liga (`:653`)** — parece bug herdado (lógica invertida). Usado hoje só para
  gravação de automação/MIDI; não existe "record mode" de áudio (punch, loop/takes).
- O botão de gravar do transporte grava MIDI no Piano Roll (`src/gui/TransportBar.cpp:159-167`, `:249-260`).
- Flags de transporte (`m_playing`, `m_paused`, `m_exporting`) são `volatile bool` escritas pela GUI e lidas pela
  thread de áudio (`include/Song.h:429-434`), algumas sem mutex (`playSong`, `:513-515`). Não é atômico em C++.

#### 3.5 Posição da música (importante para alinhar a gravação)

- A posição é `TimePos` em ticks (192 por compasso, `include/TimePos.h:38`) + `float m_frameOffset` dentro do tick
  (`include/Timeline.h:71-72`, `:109`), avançada em `Song::processNextBuffer` (`Song.cpp:273-349`).
- `Engine::framesPerTick()` é um `float` global (`src/core/Engine.cpp:143-146`), muda com o tempo (BPM).
- Faixas só começam um clipe de sample no início de um tick (`SampleTrack.cpp:111-127` com `Song.cpp:331-339`), com
  conversão truncada `framesPerTick * ticks` (`SampleTrack.cpp:116-117`).
- **Não há contador absoluto de frames do transporte.** Para gravar alinhado a amostra é preciso criar um (frame
  da música no início de cada período), porque tick + float acumula erro e não sobrevive a mudanças de tempo.

---

### 4. Latência

#### 4.1 Latência de saída/entrada conhecida hoje
- Nenhum backend informa latência ao motor; `AudioDevice` não tem método para isso (`include/AudioDevice.h`).
  Só se sabe `framesPerAudioBuffer()`/`framesPerPeriod()` e, no Pulse, um valor pedido (não o obtido).
- Componentes reais (que a compensação vai precisar somar):
  1. latência de captura do driver/hardware (JACK: `jack_port_get_latency_range(input, JackCaptureLatency)`;
     PortAudio: `inputLatency`);
  2. +1 ciclo JACK pela ordem render-depois-captura (`AudioJack.cpp:421` vs `:424-433`);
  3. o jitter de 0–512 frames da drenagem do ring (`AudioEngine.cpp:214`);
  4. +1 período interno de 256 na saída (`AudioEngine.cpp:316`, `:439`);
  5. resto do período no `renderNextBuffer` quando o buffer do driver não é múltiplo de 256;
  6. latência de playback do driver/hardware;
  7. PDC do mixer (abaixo), se o que o músico ouve passa por plugins latentes.

#### 4.2 PDC já presente
- `AudioProcessor::latency()` (`include/AudioProcessor.h:228-235`), `EffectChain::latency()` soma os efeitos
  (`src/core/EffectChain.cpp:336-342`), `Effect` atrasa o dry com `LatencyDelay` (`include/Effect.h:199`, `:224`;
  `src/core/Effect.cpp:128`). Lv2 mede a latência pela porta de latência (`include/Lv2Proc.h:145-176`).
- `Mixer::addLatencySource(const IntModel* channel, const std::atomic<f_cnt_t>* latency)` /
  `removeLatencySource` (`include/Mixer.h:277-282`, `src/core/Mixer.cpp:995-1010`) registram fontes de áudio
  atrasadas (hoje: instrumento latente, `include/InstrumentTrack.h:352-353`, `src/tracks/InstrumentTrack.cpp:111`,
  `:628`).
- `Mixer::compensateLatency()` roda a cada `masterMix` (`Mixer.cpp:1089-1136`, chamada em `:1151`): por canal,
  atrasa as entradas mais adiantadas até a mais atrasada (`m_inputDelay`, atraso por send em
  `MixerRoute::latencyDelay`) e calcula `m_latency = maior entrada + efeitos` (`include/Mixer.h:145-148`, `:224-233`).
- `LatencyDelay` (`include/LatencyDelay.h:38-95`): linhas de atraso por canal; **`setDelay` aloca
  (`m_lines.assign`) quando o atraso muda** (`:45-51`) e é chamado da thread de áudio dentro de
  `compensateLatency` (`Mixer.cpp:1124-1128`). `m_latencyPass.assign(...)` (`:1101`) também pode alocar.
- O PDC alinha canais entre si, mas **não adianta a timeline**: `Mixer::latency()` (`Mixer.cpp:1139-1142`) não é
  usado em lugar nenhum (o master sai atrasado em relação à posição mostrada/à `Song`).

#### 4.3 Onde caberia a compensação de latência de entrada
- **Na gravação, não no mixer:** o arquivo gravado deve ser deslocado (o início do clipe recuado) por
  `latência_captura + latência_saída + latência_interna_do_motor (+ PDC do master se o músico ouve através dele)`,
  mais um ajuste manual. Isso é só um offset no posicionamento do clipe/sample gravado, calculado a partir do
  contador absoluto de frames do transporte (§3.5). Não exige mexer em `compensateLatency`.
- O `addLatencySource` serve para outra coisa: alinhar a **monitoração** de uma entrada (sem latência) com faixas
  latentes no mesmo canal. Para monitoração de entrada a regra do REAPER é não atrasar a entrada (menor latência
  possível), então provavelmente a faixa monitorando não deve se registrar como fonte latente **[suposição de
  projeto]**.
- Requisito de API: `AudioDevice` precisa ganhar algo como `inputLatency()`/`outputLatency()` em frames, preenchido
  por backend (JACK pelo callback de latência; PortAudio por `Pa_GetStreamInfo`; ALSA por `snd_pcm_delay`/
  parâmetros; SDL não informa de forma confiável).

---

### 5. Problemas estruturais que bloqueiam ou complicam a gravação multipista

1. **Entrada fixa em estéreo e sem roteamento por canal.** `SampleFrame` de 2 canais, JACK com 2 portas, SDL
   estéreo, ring buffer de `SampleFrame` (`AudioEngine.h:385`, `:427`; `AudioJack.cpp:212-219`). `AudioInput`
   com `firstChannel` não tem de onde ler. É preciso um buffer de entrada planar com N canais (`AudioBuffer`
   planar já existe, `include/AudioBuffer.h`) e N portas/canais por backend.
2. **Entrada não sincronizada ao período e com jitter.** Drenagem limitada a 512 frames/período
   (`AudioEngine.cpp:214`), JACK empurra a entrada depois do render (`AudioJack.cpp:421-433`), SDL com dois clocks.
   Para gravação séria: no JACK e no PortAudio (duplex), a entrada do callback deve ser consumida **no mesmo
   callback, antes do render**, período a período, sem ring intermediário; ring só para backends realmente
   assíncronos.
3. **Mutex na thread de áudio.** `renderNextPeriod` trava `m_changeMutex` o período inteiro e a GUI trava o mesmo
   mutex para editar (`AudioEngine.cpp:424`, `:676-686`); mais `MixerChannel::m_lock` em `mixToChannel`
   (`Mixer.cpp:954-961`, `:974-983`), `AudioBusHandle::m_playHandleLock` (`src/core/AudioBusHandle.cpp:256-263`),
   `NotePlayHandle` locks, `QWaitCondition::wakeAll` a cada estágio. Qualquer edição na GUI durante a gravação pode
   causar xrun → buraco no arquivo. A gravação em disco **não pode** depender desse caminho: o lado RT só deve
   copiar para um ring por faixa armada.
4. **Alocações na thread de áudio.** `new SamplePlayHandle` (`SampleTrack.cpp:149`), `delete` de play handles
   (`AudioEngine.cpp:301`, `:383`), `LatencyDelay::setDelay` e `m_latencyPass.assign` (`Mixer.cpp:1101`, `:1124`),
   evento Qt de `nextAudioBuffer` (`AudioEngine.cpp:404`), `resize` do buffer JACK (`AudioJack.cpp:236`).
5. **I/O de log na thread RT.** `fprintf(stderr)` em `pushInputFrames` (`AudioEngine.cpp:184-201`) e `printf` em
   handlers de erro (ALSA). Trocar por contadores atômicos lidos pela GUI.
6. **Captura sempre aberta.** SDL abre e despausa a captura sempre (`AudioSdl.cpp:99`, `:134`); PortAudio abre e
   descarta (`AudioPortAudio.cpp:149-160`, `:191`); JACK registra portas de entrada sempre (`AudioJack.cpp:218-219`,
   barato). Não há API para abrir/fechar captura sob demanda (`AudioDevice.h`).
7. **Nenhuma latência informada pelos drivers** e +1 período interno não documentado (§4.1).
8. **Backends Linux sem captura:** ALSA, PulseAudio e sndio são só playback. No Linux, hoje, só JACK (e SDL, com
   clocks separados) capturam. Para cumprir "ALSA e PulseAudio também precisam funcionar" é preciso escrever a
   captura neles (ALSA: abrir `SND_PCM_STREAM_CAPTURE` e `snd_pcm_link` com o playback; Pulse: stream de record) ou
   aceitar PortAudio (host API ALSA) como rota "ALSA" duplex.
9. **Sem contador absoluto de frames do transporte** e posição em ticks + `float` (§3.5); clipes posicionados em
   ticks (192/compasso). O alinhamento sub-tick do áudio gravado tem que ficar no offset do sample dentro do clipe.
10. **Estado global e singletons.** `Engine::audioEngine()`, `Engine::getSong()`, `Engine::mixer()`,
    `Engine::s_framesPerTick` (`Engine.cpp:44`, `:143-146`), fila global de jobs estática
    (`AudioEngineWorkerThread.cpp:40-42`), `volatile bool` de transporte (`Song.h:429-434`). Início/fim de gravação
    precisa de handshake atômico GUI→RT com o frame exato de início, não de `volatile`.
11. **Sample rate:** entrada e saída supostas iguais (`AudioEngine.h:195-204`); sem callback de mudança de taxa do
    JACK; trocar device/taxa exige reiniciar. Arquivos gravados precisam guardar a taxa e o projeto precisa
    reamostrar na reprodução se a taxa mudar (o `Sample` já reamostra, `Sample.cpp:107`).
12. **Reprodução de áudio longo toda em RAM.** `SampleBuffer` é um `std::vector<SampleFrame>` inteiro
    (`include/SampleBuffer.h:52`, `:62`). Para várias faixas longas gravadas, tocar exige streaming de disco ou muita
    memória — não bloqueia a primeira versão, mas limita.
13. **`Song::stop()` liga `m_recording`** (`Song.cpp:653`) — corrigir antes de reutilizar a flag para áudio.
14. **Pequenos bugs nos backends que afetam confiabilidade:** unidade errada do `tlength` no Pulse
    (`AudioPulseAudio.cpp:145`), device configurado ignorado no Pulse (`:147`), ALSA com buffer de 8 períodos
    (`AudioAlsa.cpp:288`), `AUDIO_BUS_HANDLE_SUPPORT` morto no JACK, osciloscópio lendo buffer do motor sem
    sincronização (`Oscilloscope.cpp:96-98`).

---

### 6. Recomendação mínima de arquitetura (derivada do levantamento)

- Estender `AudioDevice` com: número de canais de entrada, `inputLatency()/outputLatency()`, `setCaptureEnabled()`.
- Um buffer de entrada planar de N canais **por período**, preenchido na mesma chamada que renderiza (backends
  duplex: JACK, PortAudio), exposto ao período como `AudioBufferView` somente leitura.
- Por faixa armada: copiar os canais escolhidos (`AudioInput`) num ring SPSC pré-alocado; uma thread de disco
  (não RT) esvazia e escreve WAV/FLAC (`AudioFileWave`/`FlacExporter` existentes podem servir de base,
  `src/core/audio/AudioFileWave.cpp`, `FlacExporter.cpp`).
- Registrar, no período em que a gravação começa, o frame absoluto do transporte e a soma das latências; o clipe é
  criado na GUI depois, deslocado por essa compensação.
- Monitoração: mixar a entrada no buffer da faixa (antes dos efeitos da faixa) no estágio de play handles, sem
  passar pelo ring de disco.

---

## Anexo B: 02 — Modelo de dados de faixas, clipes e áudio (levantamento para gravação multipista)

Repositório: `/Users/castrot1/Desktop/LinuxMusic` (branch `master`, HEAD `2395a2c0d`). Levantamento **somente leitura**.
Citações no formato `arquivo:linha`. O que é inferência ou palpite está marcado como **[inferência]** ou **[palpite]**.

---

### 0. Resumo executivo

- O tempo do projeto é medido em **ticks inteiros** (`tick_t = int32`), com **192 ticks por compasso 4/4**
  (`include/TimePos.h:38`), ou seja, 48 ticks por semínima. A 48 kHz e 120 BPM, 1 tick = 500 frames ≈ 10,4 ms
  (`src/core/Engine.cpp:134-146`). **Posição, comprimento e offset de clipe são todos em ticks**: nada no modelo é
  preciso em nível de amostra.
- **Não existe mapa de tempo.** O tempo é um único `IntModel` inteiro (`include/Song.h:414`, `bpm_t = uint16`), que pode
  ser automatizado. `Engine::framesPerTick()` é um `float` global recalculado a cada mudança (`src/core/Engine.cpp:143`).
  A fórmula de compasso também é global (`TimePos::s_ticksPerBar` estático, `include/TimePos.h:144`).
- **O áudio fica inteiro na RAM, decodificado para float estéreo** (`std::vector<SampleFrame>`,
  `include/SampleBuffer.h:91`), mesmo quando o arquivo é mono. A miniatura da forma de onda guarda **mais uma cópia do
  mesmo tamanho** (1 pico min/max por frame, `src/core/SampleThumbnailCache.cpp:39-44`). Não há leitura em streaming do
  disco.
- `SampleTrack`/`SampleClip` já aceitam **vários clipes sobrepostos**: todos tocam ao mesmo tempo e são **somados**
  (`src/tracks/SampleTrack.cpp:107-156`). Não há prioridade "o de cima ganha", nem lanes, nem takes.
- A gravação herdada (flag de gravação no clipe, `SampleRecordHandle`, `AudioSampleRecorder`) **foi removida** no
  commit `d95c6ba49`. No modelo sobraram `AudioInput` + `recordArmModel` + `monitorModel` na `SampleTrack`, que por
  enquanto só são salvos e carregados (`src/tracks/SampleTrack.cpp:182-239`), e o caminho de captura do
  `AudioEngine`, que não tem consumidor.
- O undo é **por snapshot XML** do objeto inteiro (`src/core/ProjectJournal.cpp:121-136`). Desfazer algo numa faixa
  serializa **todos os clipes da faixa**. Um clipe sem arquivo embute o áudio inteiro em Base64
  (`src/core/SampleClip.cpp:312-316`).

---

### 1. Linha do tempo: `TimePos`, `Song`, `Timeline`, tempo

#### 1.1 `TimePos` (`include/TimePos.h`, `src/core/TimePos.cpp`)

- Guarda um único `tick_t m_ticks` (`include/TimePos.h:142`), e `tick_t` é `std::int32_t`
  (`include/LmmsTypes.h:35`).
- `DefaultTicksPerBar = 192` (`include/TimePos.h:38`). `s_ticksPerBar` é **estático global** e muda com a fórmula de
  compasso (`include/TimePos.h:131,144`; `src/core/Song.cpp:185-194`).
- Conversões tick ↔ frame truncam: `frames(float framesPerTick)` faz `static_cast<f_cnt_t>(m_ticks * framesPerTick)`
  (`include/TimePos.h:111-118`), e `fromFrames` arredonda para baixo (`include/TimePos.h:122-125`).
- `operator int()` implícito (`include/TimePos.h:97`): TimePos se mistura livremente com `int` no código todo.

#### 1.2 Frames por tick e tempo

- `Engine::framesPerTick()` devolve um `float s_framesPerTick` (`include/Engine.h:102-104,138`):
  `sampleRate * 60 * 4 / 192 / tempo` (`src/core/Engine.cpp:143-146`). A variante `framesPerTick(sampleRate)`
  (`src/core/Engine.cpp:134-137`) é usada para converter ticks em frames **na taxa do arquivo**.
- Tempo: `IntModel m_tempoModel` (`include/Song.h:69,414`), inteiro de 10 a 999 (`include/Song.h:60-62`). Não existe
  tempo fracionário.
- `Song::setTempo()` (`src/core/Song.cpp:158-180`) é chamado por conexão **DirectConnection** a partir de
  `dataChanged` do modelo (`src/core/Song.cpp:99-102`). Ele redimensiona as NotePlayHandles, chama
  `Engine::updateFramesPerTick()` e emite `tempoChanged`.
- **Não existe mapa de tempo.** Mudanças de tempo no meio da música só acontecem automatizando `m_tempoModel` com um
  clipe de automação, aplicado em `Song::processAutomations` (`src/core/Song.cpp:360-447`), que roda na thread de
  áudio (chamado por `processNextBuffer`, `src/core/Song.cpp:335`). **[inferência]** Por causa da DirectConnection,
  tempo automatizado dispara `setTempo()` e `SampleClip::tempoChanged()` → `updateLength()` → `Song::updateLength()`
  (com `m_tracksMutex.lockForRead`, `src/core/Song.cpp:588`) **dentro da thread de áudio**.
- `Timeline::m_elapsedSeconds` acumula segundos usando o `framesPerTick` vigente (`include/Timeline.h:64-68`). Em
  `setTicks` ele é recalculado como se o tempo fosse constante (`include/Timeline.h:52-61`). O próprio comentário
  admite que ignora mudanças de tempo.

#### 1.3 Posição de reprodução, loop e `Timeline`

- `Song` tem um `Timeline` por `PlayMode` (`include/Song.h:227-230,444`). Cada `Timeline` (`include/Timeline.h:36-115`)
  guarda `m_pos` (TimePos), `m_frameOffset` (float, frames dentro do tick atual), os pontos de loop
  `m_loopBegin`/`m_loopEnd` (TimePos, em ticks), `m_loopEnabled`, `m_playStartPosition` e o comportamento ao parar.
- A posição absoluta em frames é **derivada**: `ticks * framesPerTick + frameOffset` (`include/Song.h:389-392`). Com
  tempo automatizado, esse número não corresponde ao tempo real decorrido.
- Os loops são em ticks: `enforceLoop` (`src/core/Song.cpp:253-261`) só é verificado **quando um tick inteiro passa**
  (`src/core/Song.cpp:278-314`). O ponto de loop, portanto, cai numa fronteira de tick.

#### 1.4 Como `Song::processNextBuffer` aciona as faixas

`src/core/Song.cpp:207-352`. Roda na thread de áudio, a partir de `AudioEngine::renderStageNoteSetup`
(`src/core/AudioEngine.cpp:323`), com `m_changeMutex` travado durante o período inteiro
(`src/core/AudioEngine.cpp:424`).

1. Escolhe as faixas pelo modo: Song = todas, Pattern = a `PatternTrack` do padrão atual, MidiClip = a faixa do clipe
   (`src/core/Song.cpp:222-246`).
2. Percorre o período em **pedaços que terminam em fronteiras de tick** (`src/core/Song.cpp:273-348`):
   `framesToPlay = min(até o fim do período, até o próximo tick)` (`:316-320`).
3. **Só no primeiro frame de cada tick** (`if (frameOffsetInTick == 0)`, `:332`) processa automações, metrônomo e
   chama `track->play(getPlayPos(), framesToPlay, frameOffsetInPeriod, clipNum)` (`:338-341`).
   **Consequência:** uma faixa só pode iniciar algo numa fronteira de tick. O `frameOffsetInPeriod` vira o `offset`
   da PlayHandle.
4. As PlayHandles criadas entram em `m_newPlayHandles` e são transferidas para `m_playHandles` depois de
   `processNextBuffer` (`src/core/AudioEngine.cpp:326-332`). São renderizadas em paralelo pelos workers no estágio 1
   (`src/core/AudioEngine.cpp:337-353`).

#### 1.5 Estado `m_recording` do Song

- `volatile bool m_recording` (`include/Song.h:429`), ligado por `playAndRecord()` (`src/core/Song.cpp:528-532`).
- **Bug/resíduo:** `Song::stop()` faz `m_recording = true` (`src/core/Song.cpp:653`).
- Hoje ninguém chama `playAndRecord()` nem lê `Song::isRecording()`. O grep só encontra `AutomationClip::isRecording`
  e `PianoRoll::isRecording`. É código morto.

#### 1.6 Modelo de lock

- `AudioEngine::requestChangeInModel()`/`doneChangeInModel()` (`src/core/AudioEngine.cpp:676-686`) são um **mutex
  global** que a thread de áudio segura durante cada período (`:424`). A GUI o trava para editar
  (`Clip::movePosition`, `src/core/Clip.cpp:103-105`; `SampleClip::setSampleBuffer`, `src/core/SampleClip.cpp:149`).
  Na thread de renderização vira no-op (`s_renderingThread`).
- `Track::create(const QDomElement&)` segura esse mutex durante o `restoreState` inteiro
  (`src/core/Track.cpp:146-171`). Isso inclui a decodificação síncrona de todos os arquivos de áudio dos clipes, ou
  seja, **o áudio para enquanto arquivos são decodificados** (clonar faixa, colar faixa, undo de faixa).
- `Track::addClip`/`removeClip` mexem em `m_clips` (`std::vector`) sem lock próprio (`src/core/Track.cpp:328-352`).
  **[inferência]** Dependem de quem chama já ter travado o mutex global, e isso nem sempre acontece. O construtor de
  `Clip` chama `addClip` sem lock (`src/core/Clip.cpp:53-56`).

---

### 2. `Track`, `TrackContainer`, `PatternStore`/`PatternTrack`

#### 2.1 `Track` (`include/Track.h`, `src/core/Track.cpp`)

- Herda de `Model` e `JournallingObject` (`include/Track.h:67`). Tipos em `include/Track.h:75-86`:
  Instrument, Pattern, Sample, Event, Video, Automation, HiddenAutomation, Folder. Event e Video não são
  implementados (`src/core/Track.cpp:110-111`).
- A interface virtual central é `play(start, frames, frameBase, clipNum)` (`include/Track.h:121-122`), mais
  `createClip`, `saveTrackSpecificSettings`/`loadTrackSpecificSettings`.
- Clipes: `std::vector<Clip*> m_clips` (`include/Track.h:339`), **sem ordem garantida** (`push_back` em
  `src/core/Track.cpp:330`). `getClipsInRange` devolve ordenado por início (`src/core/Track.cpp:400-415`).
  `getClip(n)` fora do intervalo **cria um clipe novo** (`src/core/Track.cpp:366-376`).
- Silêncio: `isSilenced()` combina mute com solo e pasta, via atômico (`include/Track.h:272-274`).
- Serialização (`src/core/Track.cpp:189-227`): um `<track type= name= muted= solo= ...>` que contém
  `<sampletrack .../>` (configurações específicas) seguido de **um elemento por clipe** (`clip->saveState`).
- Carga (`src/core/Track.cpp:230-301`): apaga todos os clipes (`:270-273`) e recria cada um com `createClip(0)` +
  `restoreState` (`:286-291`).
- `Track::create` dá um canal de mixer novo às faixas de instrumento e sample (`src/core/Track.cpp:129-135`), uma
  regra do fork.

#### 2.2 `TrackContainer` (`include/TrackContainer.h`)

- `TrackList m_tracks` (vetor), protegido por `QReadWriteLock m_tracksMutex` (`include/TrackContainer.h:53,167,192`).
- Dois tipos: `Pattern` e `Song` (`:54-58`). Pastas no estilo REAPER são feitas por profundidade (`folderDepth`), com
  um `FolderLayout` que é um JournallingObject para undo do layout (`:180-196`).
- `Song` e `PatternStore` herdam de `TrackContainer` (`include/Song.h:66`, `include/PatternStore.h`).

#### 2.3 `PatternStore`/`PatternTrack`

- O `PatternStore` é um container paralelo cujas faixas têm **um clipe por padrão**, indexado pelo número do clipe.
  `PatternStore::play` faz `start % comprimento do padrão` e chama `t->play(start, frames, offset, clipNum)` em cada
  faixa (`src/core/PatternStore.cpp:47-68`).
- `PatternTrack::play` (`src/tracks/PatternTrack.cpp:89-134`) procura na faixa do Song o `PatternClip` ativo e
  repassa para o `PatternStore` com o índice do padrão.
- `SampleTrack::play` tem um ramo inteiro só para o caso de padrão (`clipNum >= 0`,
  `src/tracks/SampleTrack.cpp:81-103`). Também `SampleClip::updateLength` trata padrão à parte
  (`src/core/SampleClip.cpp:242-246`). **Para uma faixa de gravação nova, esse caminho é bagagem.** O ideal é ela não
  existir no `PatternStore`.

---

### 3. `Clip` (base) e `ClipView`

#### 3.1 Modelo (`include/Clip.h`, `src/core/Clip.cpp`)

| Campo | Tipo | Significado |
|---|---|---|
| `m_startPosition` | TimePos (ticks) | início na linha do tempo (`include/Clip.h:201`) |
| `m_length` | TimePos (ticks) | comprimento (`:202`) |
| `m_startTimeOffset` | TimePos (ticks) | deslocamento do conteúdo em relação ao início do clipe (`:203`) |
| `m_start/endCrossfadeLength` | TimePos | fade in/out (`:205-206`) |
| `m_start/endCrossfadeTension` | float | curva do fade (`:207-208`) |
| `m_mutedModel`, `m_soloModel` | BoolModel | (`:210-211`) |
| `m_autoResize` | bool | se o comprimento acompanha o conteúdo (`:212`) |
| `m_color` | optional<QColor> | (`:216`) |

- **Semântica do `startTimeOffset`** (deduzida de `src/gui/clips/ClipView.cpp:984-1005` e `:1585`): é a posição, em
  ticks, do início do conteúdo em relação ao início do clipe. Aparar a borda esquerda para a direita deixa o valor
  **mais negativo**. Ao cortar, o clipe da direita recebe `offset_esq - comprimento_esq`. Valor positivo significa
  silêncio antes do conteúdo.
- `movePosition` trava o mutex do engine e chama `Song::updateLength()` (`src/core/Clip.cpp:98-109`).
  `changeLength` **não trava** (`:112-121`). O comentário do header diz que essas funções "adicionam uma entrada no
  journal" (`include/Clip.h:120-130`), mas o código não faz isso: quem faz é a GUI.
- O fade (crossfade) por clipe é do PR #7900 do LMMS. Ele **não faz crossfade automático** entre clipes sobrepostos:
  é só um fade in/out em cada clipe, aplicado pela `SamplePlayHandle` (ver 5.4). `isCrossfadeable()` só é verdadeiro
  para `SampleClip` (`include/SampleClip.h:88`).
- `clone()` usa o construtor de cópia (`include/SampleClip.h:90-93`). `copyStateTo` faz cópia via XML
  (`src/core/Clip.cpp:130-145`).

#### 3.2 Seleção, cópia/cola e undo na GUI

- Copiar e arrastar: `ClipView::createClipDataFiles` serializa cada clipe com `saveState` dentro de um `DataFile`
  `DragNDropData`, junto com `trackIndex`/`trackType` e os metadados `initialTrackIndex` e `grabbedClipPos`
  (`src/gui/clips/ClipView.cpp:378-423`). O conteúdo vai para o clipboard como texto MIME
  (`src/core/Clipboard.cpp:51-75`).
- Colar: `TrackContentWidget::pasteSelection` desserializa, calcula o offset quantizado e faz `createClip(pos)`,
  `restoreState` e depois `movePosition(pos)` (`src/gui/tracks/TrackContentWidget.cpp:381-470`).
  `SampleClip::saveSettings` grava `pos=-1` quando o pai é `clipboard` (`src/core/SampleClip.cpp:295-302`).
  **Colar um SampleClip relê o arquivo pelo caminho.** Quando há cache, compartilha o buffer.
- Undo: `addJournalCheckPoint` no clipe (alteração de 1 clipe) ou **na faixa inteira** (vários clipes, corte, colagem)
  (`src/gui/clips/ClipView.cpp:255-274`, `:592`, `:622`, `:1397`, `:1575`;
  `src/gui/tracks/TrackContentWidget.cpp:395,524`).

#### 3.3 Journal (`src/core/JournallingObject.cpp`, `src/core/ProjectJournal.cpp`)

- Um checkpoint é `jo->saveState(...)` num `DataFile` (XML DOM) (`src/core/ProjectJournal.cpp:121-136`), com limite
  de `MAX_UNDO_STATES = 100` (`:42`).
- Undo: salva o estado atual para o redo e depois `restoreState` do snapshot (`src/core/ProjectJournal.cpp:55-84`).
- Para uma faixa, `restoreState` passa por `Track::loadTrack`, que **apaga e recria todos os clipes**
  (`src/core/Track.cpp:270-291`). **[inferência]** Como o `SampleCache` guarda só `weak_ptr`
  (`include/FileResourceCache.h:83`), se nenhum outro objeto segurar o buffer, ele é liberado ao apagar os clipes e o
  arquivo é **decodificado de novo do disco** ao recriar, com o mutex de áudio possivelmente travado (ver 1.6).
- `SampleClipView::reverseSample` não grava checkpoint (`src/gui/clips/SampleClipView.cpp:341-347`).
  `SampleClip::loadSettings` só **liga** o reverso quando o atributo existe e nunca desliga
  (`src/core/SampleClip.cpp:370-374`). **[inferência]** Um undo que deveria desfazer o reverso não o desfaz.

---

### 4. `SampleTrack`

`include/SampleTrack.h`, `src/tracks/SampleTrack.cpp`.

- Modelos: `m_volumeModel`, `m_panningModel`, `m_mixerChannelModel` (`MixerChannelLcdModel`), **`AudioInput m_input`**,
  **`BoolModel m_recordArmModel`** ("Record arm"), **`IntModel m_monitorModel`** (Off, On, Auto; padrão Auto),
  `AudioBusHandle m_audioBusHandle` e `m_isPlaying` (`include/SampleTrack.h:117-124`;
  `src/tracks/SampleTrack.cpp:46-60`).
- `AudioInput` (`include/AudioInput.h`): `{Source None|Hardware, firstChannel, channelCount (1|2)}`.
- Persistência: `recordarm` e `monitor` são atributos. `<input source="hardware" channel= channels=>` só é escrito
  quando a fonte é Hardware (`src/tracks/SampleTrack.cpp:182-198,213-239`).
- **Por enquanto esses três modelos não têm efeito no áudio.** Não há consumidor de `inputBuffer()` no núcleo. O grep
  por `inputBuffer()` e `inputBufferFrames()` só encontra a declaração (`include/AudioEngine.h:242-250`).
- `play()` (`src/tracks/SampleTrack.cpp:73-159`), modo Song:
  - Percorre **todos** os clipes a cada tick (`:107`). Para cada clipe que cobre `_start` e ainda não está tocando
    (`:112-114`) calcula:
    - `sampleStart = framesPerTick(srArquivo) * (_start - início - offset)` (`:116-117`);
    - `clipFrameLength = framesPerTick(srArquivo) * (fim - início - offset)` (`:118`), que é na prática o **índice
      absoluto** do frame do arquivo onde o clipe termina. O comentário chama de "length";
    - `samplePlayLength = min(clipFrameLength, tamanho do buffer)` (`:122`).
    Grava esses valores **no `Sample` do próprio clipe** (`setSampleStartFrame`/`setSamplePlayLength` →
    `m_startFrame`/`m_endFrame`) e marca `isPlaying` (`:127-131`).
  - Clipes fora de `_start` são marcados como não tocando (`:135-138`).
  - Cria uma `SamplePlayHandle` nova (`new`, na thread de áudio) para cada clipe não mutado e define
    `offset = frameOffsetInPeriod` (`:144-156`).
- `updateClips()` e `SampleClip::playbackPositionChanged()` removem todas as `SamplePlayHandle` da faixa e zeram
  `isPlaying` (`src/tracks/SampleTrack.cpp:244-248`; `src/core/SampleClip.cpp:199-204`). Isso acontece em
  play/stop, salto do cursor, mute de clipe ou faixa e movimento de clipe. **Qualquer edição durante a reprodução
  corta o som da faixa**, que só volta a soar no próximo tick em que um clipe "começa": o clipe é reativado no tick
  seguinte com `sampleStart` recalculado.

#### 4.1 Clipes sobrepostos hoje

- Cada clipe ativo ganha sua própria PlayHandle, e todas somam no `AudioBusHandle` da faixa
  (`src/tracks/SampleTrack.cpp:144-156`). **Sobreposição = mixagem**. Não há regra de "último/de cima ganha", nem
  crossfade automático na região sobreposta, nem noção de camada.
- Desenho: os clipes de uma faixa ocupam a mesma faixa visual e se sobrepõem graficamente. Não há lanes.

---

### 5. `SampleClip`, `Sample`, `SampleBuffer`, cache, decodificação, `SamplePlayHandle`

#### 5.1 `SampleClip` (`include/SampleClip.h`, `src/core/SampleClip.cpp`)

- Membros: `Sample m_sample` (cópia por valor, que compartilha o `SampleBuffer`), `m_sampleThumbnailCache`,
  `bool m_isPlaying` (não atômico, escrito pela thread de áudio e pela GUI) e `int m_startFrameOffset`
  (`include/SampleClip.h:107-110`).
- Cada clipe conecta **5 sinais do Song** (tempo, compasso, playbackStateChanged, positionJumped, mute)
  (`src/core/SampleClip.cpp:52-69`). Com milhares de clipes, cada mudança de tempo faz um trabalho O(N) em cascata.
  **[inferência]**
- Arquivo: `setSampleFile(path)` (`src/core/SampleClip.cpp:159-190`) → `SampleCache::fetchBuffer` +
  `fetchThumbnailCache` → `Sample{buffer}` → `updateLength()`. Também zera `startTimeOffset`.
- Buffer direto: `setSampleBuffer(shared_ptr)` (`src/core/SampleClip.cpp:144-157`) troca o buffer sob o mutex do
  engine e **recalcula a miniatura sem cache** (`:151`). Duplo clique no clipe usa `SampleBuffer::fromFile` direto,
  **sem passar pelo cache** (`src/gui/clips/SampleClipView.cpp:184-197`). Existem dois caminhos de carga
  inconsistentes.
- **Comprimento dependente de tempo:**
  - `sampleLength()` = `frames do arquivo / framesPerTick(srArquivo)` em ticks, truncado
    (`src/core/SampleClip.cpp:269-272`). Depende do BPM atual.
  - `updateLength()`: se `autoResize` está ligado, `changeLength(sampleLength())` e `offset = 0`
    (`src/core/SampleClip.cpp:237-252`).
  - `tempoChanged()`: recalcula o offset em ticks a partir de `m_startFrameOffset` (que guarda o offset em **frames
    da taxa de saída**) e chama `updateLength()` (`src/core/SampleClip.cpp:255-260`). Assim, o offset é preservado
    em tempo real, mas **o comprimento só acompanha se `autoResize` estiver ligado**. Basta um redimensionamento
    manual ou um corte (`setAutoResize(false)`, `src/gui/clips/ClipView.cpp:913,926,941,1006,1581,1586`) para o
    clipe ficar com comprimento fixo em ticks. Depois disso, mudar o BPM corta o áudio ou cria silêncio no fim. A
    **posição** do clipe também é em ticks: mudar o BPM move o áudio no tempo absoluto. Para áudio gravado, que é
    tempo absoluto, isso está errado por padrão.
  - `m_startFrameOffset` não é salvo. Só `off` em ticks vai para o arquivo (`src/core/SampleClip.cpp:306`), então a
    precisão abaixo de tick se perde ao salvar.
- Serialização (`src/core/SampleClip.cpp:293-328`): `<sampleclip pos len muted src off autoresize startfadelength
  endfadelength startfadetension endfadetension sample_rate [color] [reversed] [data=base64]>`.
  - `src` é o caminho do arquivo (ver 6).
  - **Sem arquivo, o áudio inteiro vai em Base64 no XML** (`:312-316`). Carregar faz o caminho inverso
    (`:349-354`).
  - O comentário `// TODO: start- and end-frame` (`:327`) mostra que não existe "início no arquivo" independente do
    offset em ticks.
  - `sample_rate` é salvo, mas na carga com `src` a taxa vem do arquivo decodificado.
- Carga com arquivo ausente: chama `collectError("Sample not found")` e o clipe fica vazio, **perdendo a referência
  ao caminho** (`src/core/SampleClip.cpp:340-347`). Ao salvar de novo, `src` sai vazio. **[inferência]** Um arquivo
  offline faz a referência ser destruída no próximo salvamento.

#### 5.2 `Sample` (`include/Sample.h`, `src/core/Sample.cpp`)

- Wrapper leve: `shared_ptr<const SampleBuffer> m_buffer` mais pontos atômicos `m_startFrame`, `m_endFrame`,
  `m_loopStart/EndFrame`, `m_amplification`, `m_frequency`, `m_reversed` (`include/Sample.h:113-120`). Os índices são
  `int` (limite de cerca de 2³¹ frames ≈ 12,4 h a 48 kHz).
- `PlaybackState` (`include/Sample.h:46-68`): `AudioResampler`, buffer intermediário de `DEFAULT_BUFFER_SIZE` frames,
  `m_frameIndex` e `m_backwards`.
- `Sample::play` (`src/core/Sample.cpp:99-134`):
  - força `frameIndex >= m_startFrame`;
  - razão = `srSaída / srArquivo * freq/DefaultBaseFreq * ratio` (`:105-107`);
  - em laço: `render()` copia do buffer em RAM para o buffer intermediário, aplicando reverso
    (`m_buffer->data()[size - idx - 1]`) e ganho (`:136-176`), e o resampler converte esse buffer para o destino;
  - **lê direto da memória**. Não há acesso a disco nem pré-carga.
- `SampleBuffer::emptyBuffer()` é um singleton estático compartilhado (`src/core/SampleBuffer.cpp:67-71`).

#### 5.3 `SampleBuffer`, `SampleDecoder`, `SampleCache`

- `SampleBuffer` = `std::vector<SampleFrame> m_data`, `QString m_audioFile` e `m_sampleRate`
  (`include/SampleBuffer.h:90-93`). `SampleFrame` = 2 floats (8 bytes). **Tudo em RAM, sempre estéreo float32.**
- `SampleBuffer::fromFile` (`src/core/SampleBuffer.cpp:73-103`): resolve o caminho absoluto, decodifica **inteiro e
  de forma síncrona** e guarda `m_audioFile = toShortestRelative(path)`. Em caso de erro mostra um `QMessageBox`
  (de dentro do modelo) e devolve `emptyBuffer()`, não `nullptr`.
- `SampleDecoder::decode` (`src/core/SampleDecoder.cpp:223-233`) tenta libsndfile, depois Ogg Vorbis, depois DrumSynth.
  - libsndfile (`:59-95`): lê **o arquivo inteiro** num vetor interleaved (`sf_read_float`) e copia para
    `vector<SampleFrame>`. Mono é duplicado para estéreo. Com mais de 2 canais, **descarta tudo além de L/R**
    (`:85-91`). **Pico de memória transitório ≈ 2× o tamanho final.**
  - **Não há conversão de taxa na carga**: a taxa original fica guardada e a conversão acontece na reprodução.
- `SampleCache` (`include/SampleCache.h:37-65`) é um singleton com dois `FileResourceCache`, um de buffers e um de
  miniaturas, indexados pelo caminho absoluto.
  - `FileResourceCache` (`include/FileResourceCache.h:43-88`) **não é dono**: guarda `weak_ptr` mais
    `last_write_time`. Se o arquivo mudou no disco, recarrega. Não tem política de despejo (`:38-40`), não tem lock
    (só pode ser usado na thread da GUI) e chama `std::filesystem::last_write_time`, que **lança exceção** se o
    arquivo não existir.
  - **[inferência]** A fábrica do cache faz `std::make_shared<SampleBuffer>(std::move(*buffer))` sobre um
    `shared_ptr<const SampleBuffer>` (`include/SampleCache.h:43-44`). `std::move` de um `const&` vira **cópia**:
    mais uma cópia completa e transitória do áudio.
  - Para um arquivo que está **crescendo** (gravação), o cache refaria a decodificação completa a cada consulta,
    porque `last_write_time` muda o tempo todo.
- `SampleThumbnailCache` (`src/core/SampleThumbnailCache.cpp:34-55`): o nível mais detalhado tem **1 `Peak{min,max}`
  (8 bytes) por frame**, ou seja, o mesmo tamanho do áudio. Os níveis seguintes dividem por 4 (`:29`). Total ≈
  **1,33× o tamanho do áudio a mais**, calculado de forma síncrona na carga.

**Conta de memória (estimativa a partir do código):** uma gravação mono de 1 h a 48 kHz ocupa cerca de 518 MB em
disco (24 bits). Na RAM, vira 48000 × 3600 × 8 B ≈ **1,38 GB** de áudio mais ≈ **1,84 GB** de miniaturas, ≈ 3,2 GB
por take, e ainda há picos transitórios na carga. Com 8 faixas e vários takes, isso fica inviável.

#### 5.4 `SamplePlayHandle` (`include/SamplePlayHandle.h`, `src/core/SamplePlayHandle.cpp`)

- Para clipes, é construída com um ponteiro para `clip->sample()`, ou seja, **compartilha o `Sample` do clipe**,
  incluindo `m_startFrame`/`m_endFrame` (`src/core/SamplePlayHandle.cpp:59-65`). Usa o `AudioBusHandle` da faixa.
- `PlaybackState m_state` é construído por padrão: **interpolação Linear** do libsamplerate (`include/Sample.h:49`).
  `AudioResampler` chama `src_new` no construtor (`src/core/AudioResampler.cpp:55`), e isso acontece **na thread de
  áudio**, porque a handle é criada em `SampleTrack::play`. É alocação em tempo real. **[inferência]**
- `play(buffer)` (`src/core/SamplePlayHandle.cpp:82-154`): é executada pelos workers do AudioEngine
  (`affinityMatters() = true`, `include/SamplePlayHandle.h:49-52`). No primeiro período zera `offset()` frames
  (`:94-100`), respeita `isSilenced()` da faixa e do PatternTrack (`:104-105`), chama `Sample::play` e depois aplica
  o fade por clipe (`:117-150`).
- **[inferência] Imprecisões do fade:**
  1. usa `framesPerTick` da **taxa de saída**, mas compara com `frameIndex`, que está na **taxa do arquivo**
     (`:120-132`). Se as taxas diferem, o fade fica no lugar errado;
  2. `initialFrameIndex = m_state.frameIndex()` (`:102`) é o índice de *render-ahead* do buffer intermediário, que
     avança em blocos de `DEFAULT_BUFFER_SIZE` (`src/core/Sample.cpp:113-117`), não a posição do frame de saída. O
     envelope pode ficar deslocado em até um bloco;
  3. o fade é calculado pelo comprimento do clipe em ticks.
- Término: `totalFrames = (endFrame - startFrame) * srSaída/srArquivo` (`src/core/SamplePlayHandle.cpp:175-179`). A
  handle termina quando `m_frame >= totalFrames` (`:159-162`).

---

### 6. Projeto em disco: `DataFile`, caminhos, bundles

- Formato: XML DOM (`.mmp`) ou XML compactado com `qCompress` (`.mmpz`) (`src/core/DataFile.cpp:373-384`).
  `FILE_VERSION = 31` (`include/DataFile.h:48`). **O fork não converte projetos antigos** e só emite um aviso
  (`src/core/DataFile.cpp:684-697`). Isso dá liberdade para mudar o formato.
- Caminhos (`src/core/PathUtil.cpp`): prefixos de base `userprojects:`, `factorysample:`, `usersample:`, `local:`
  (pasta do projeto) e outros (`:104-122`). `toShortestRelative(path, allowLocal=false)` escolhe a forma mais curta
  e, **por padrão, nunca usa `local:`** (`:229-253`). `SampleBuffer::fromFile` chama esse método sem `allowLocal`
  (`src/core/SampleBuffer.cpp:78`). Um arquivo dentro da pasta do projeto, portanto, é salvo como caminho absoluto ou
  relativo a uma base global, **não como relativo ao projeto**.
- Bundle (`Song::saveProjectFile(..., withResources)`, `src/core/Song.cpp:1304`; `DataFile::writeFile`/
  `copyResources`, `src/core/DataFile.cpp:302-503`): cria `<nome>/` e `<nome>/resources/`, **copia** cada `src` de
  `sampleclip` e `audiofileprocessor` (`:66-69`), renomeando com sufixo `-N` quando há nomes repetidos, e reescreve
  o atributo como `local:resources/<arquivo>` (`:492-494`). Recusa sobrescrever um bundle que já existe (`:325-333`).
  Na carga, `hasLocalPlugins()` bloqueia `local:` fora dos elementos permitidos (`:508-550`;
  `src/core/Song.cpp:1040-1050`).
  **[inferência]** Ao reabrir um bundle, `fromFile` grava de novo o caminho com `toShortestRelative(..., false)`, o
  que transforma `local:` em caminho absoluto. Um novo salvamento normal fica preso à localização atual do bundle.
- **Não há pasta de mídia do projeto, nem gerência de arquivos gravados** (nomes, limpeza de não usados,
  consolidação).

---

### 7. Código de gravação existente no modelo

#### 7.1 Áudio

- Removido no commit `d95c6ba49` (11/11/2026): flag `m_recordModel`/`toggleRecord` do `SampleClip`,
  `SampleRecordHandle`, `AudioSampleRecorder` e o botão de gravar do Song Editor. Antes, `SampleTrack::play` criava
  `SampleRecordHandle` para clipes com `isRecord()` (ver o diff `git show d95c6ba49 -- src/tracks/SampleTrack.cpp`).
- O que sobrou:
  - **Captura no AudioEngine:**
    - `pushInputFrames` escreve num `LocklessRingBuffer` (`src/core/AudioEngine.cpp:176-203`) e chama `fprintf`
      na thread do driver quando há overflow;
    - `processBufferedInputFrames` copia para um `std::array<std::vector<SampleFrame>,2> m_inputBuffer` com
      double-buffer, no início de cada período (`src/core/AudioEngine.cpp:207-271,278`;
      `include/AudioEngine.h:242-250,385-387`);
    - é **um único fluxo estéreo**. `AudioInput::firstChannel`/`channelCount` não têm onde se apoiar: não há
      captura multicanal;
    - só **JACK** (`src/core/audio/AudioJack.cpp:433`) e **SDL** (`src/core/audio/AudioSdl.cpp:187`) chamam
      `pushInputFrames`. **[inferência]** Pelo grep, PortAudio, ALSA e PulseAudio não alimentam a entrada, embora
      `docs/etapa-3-gravacao.md` diga que PortAudio captura o tempo todo. Vale verificar.
  - `SampleTrack`: `AudioInput`, `recordArmModel` e `monitorModel`, só salvos e carregados.
  - `Song::m_recording`/`playAndRecord()`: mortos (ver 1.5).
- `ProjectJournal` e `AudioEngine` não sabem nada sobre gravação.

#### 7.2 MIDI, para comparação

- O `PianoRoll` conecta `InstrumentTrack::midiNoteOn/Off` a `startRecordNote`/`finishRecordNote`
  (`src/gui/editors/PianoRoll.cpp:917-918`). `NotePlayHandle` emite esses sinais (`src/core/NotePlayHandle.cpp:109,424`).
- `record()` e `recordAccompany()` (`src/gui/editors/PianoRoll.cpp:4341-4385`) gravam um checkpoint **no MidiClip
  aberto**, tocam o clipe ou a música e acumulam notas em `m_recordingNotes`. No note-off, inserem a nota com posição
  = `getPlayPos()` (ticks), quantização opcional e `addNote` (`:4442-4512`).
- Ou seja: a gravação MIDI é **guiada pela GUI**, grava num clipe que já existe, com resolução de tick, sem takes e
  com um único checkpoint de undo antes de começar. Ela não serve de modelo para áudio.
- Automação: `AutomationClip::isRecording` é processado em `Song::processAutomations`
  (`src/core/Song.cpp:396-416`), na thread de áudio e a cada tick.

---

### 8. Problemas estruturais para uma faixa de gravação limpa (no estilo REAPER)

Em ordem aproximada de gravidade.

1. **Memória: tudo em RAM, estéreo float32, e mais uma miniatura do mesmo tamanho.**
   `SampleBuffer` = `vector<SampleFrame>` (`include/SampleBuffer.h:91`). Mono vira estéreo
   (`src/core/SampleDecoder.cpp:80-84`), a miniatura tem resolução de 1 frame (`src/core/SampleThumbnailCache.cpp:39`)
   e a carga é síncrona com picos de 2–3×. Para gravação multipista longa, é preciso um **leitor de disco em
   streaming** (thread de leitura e ring buffers por clipe ativo) e **picos pré-calculados em arquivo** (como os
   `.reapeaks` do REAPER), com resolução grossa.

2. **Tempo em ticks, não em amostras.** Posição, comprimento, offset, fades e pontos de loop são todos `int32` ticks
   (`include/Clip.h:201-206`; `include/Timeline.h:104-105`). As faixas só iniciam reprodução em fronteiras de tick
   (`src/core/Song.cpp:332-341`). Isso não permite alinhar a gravação com precisão de amostra, nem compensar
   latência, nem fazer punch exato, nem edição fina. Um clipe de áudio gravado precisa de posição em
   **tempo absoluto (amostras ou segundos em double)**, com conversão para a régua musical apenas na interface.

3. **Clipes de áudio presos ao tempo musical.** `sampleLength()` e `tempoChanged()` amarram comprimento e offset ao
   BPM corrente (`src/core/SampleClip.cpp:255-272`). A posição em ticks move o áudio quando o BPM muda. Com
   `autoResize` desligado (após qualquer corte), o clipe é cortado ou fica com silêncio. Não existe opção "base de
   tempo: tempo/beats" por item.

4. **Sem mapa de tempo.** Um só `IntModel` inteiro, alterado por automação na thread de áudio, e
   `Engine::framesPerTick` global em `float` (`src/core/Engine.cpp:143`). As conversões tick ↔ frame acumulam erro.
   `Timeline::m_elapsedSeconds` é aproximado. Converter tempo absoluto em posição musical exige um **mapa de tempo**
   (lista de mudanças de tempo e de compasso) com integração exata.

5. **Sem takes nem lanes.** O `Clip` não tem conceito de take, de grupo de takes ou de lane, e o `Track` guarda uma
   lista plana (`include/Track.h:339`). Sobreposição soma (`src/tracks/SampleTrack.cpp:107-156`). É preciso um
   modelo novo: item → takes (cada um com fonte + offset na fonte), take ativo, lanes de comping e regra de
   sobreposição ("o mais recente esconde o de baixo" ou crossfade automático).

6. **A reprodução depende de estado mutável compartilhado e é reiniciada a cada edição.** A `SamplePlayHandle`
   aponta para o `Sample` do clipe e `SampleTrack::play` escreve `start/endFrame` nele
   (`src/tracks/SampleTrack.cpp:127-128`). Qualquer edição, salto ou mute derruba todas as handles da faixa
   (`src/core/SampleClip.cpp:199-204`). Criar handles e resamplers (`new` + `src_new`) acontece na thread de áudio.
   Uma faixa de gravação deveria ter um **"playlist" imutável** (snapshot por período, trocado de forma atômica) e
   leitores pré-alocados.

7. **Undo baseado em snapshots XML da faixa inteira.** `ProjectJournal::addJournalCheckPoint`
   (`src/core/ProjectJournal.cpp:121-136`). Um clipe sem arquivo embute Base64 do áudio inteiro
   (`src/core/SampleClip.cpp:312-316`), e 100 checkpoints disso estouram a memória. Desfazer numa faixa recria todos
   os clipes e pode redecodificar arquivos com o mutex de áudio travado (`src/core/Track.cpp:146-171,270-291`). Para
   áudio, o undo deve guardar **apenas metadados** (referências imutáveis a arquivos de mídia, nunca as amostras) e,
   de preferência, comandos ou diferenças, não a faixa inteira.

8. **Gerência de arquivos inexistente.**
   - Não há pasta de mídia do projeto, convenção de nomes, limpeza de não usados ou consolidação.
   - O caminho vem de `toShortestRelative` sem `local:` (`src/core/SampleBuffer.cpp:78`; `src/core/PathUtil.cpp:243`).
   - O bundle copia arquivos inteiros (`src/core/DataFile.cpp:486`).
   - Um arquivo ausente apaga a referência no próximo salvamento (`src/core/SampleClip.cpp:340-347`).
   - Arquivos gravados precisam de caminhos **relativos ao projeto**, de um estado "offline" que preserva a
     referência e de recuperação de arquivos órfãos (gravação interrompida).

9. **Cache e decodificação inadequados para arquivos que crescem ou são grandes.** O `FileResourceCache` invalida por
   `last_write_time`, não tem lock, lança exceção se o arquivo não existir e não é dono do recurso
   (`include/FileResourceCache.h:60-78`). Há dois caminhos de carga, com e sem cache
   (`src/gui/clips/SampleClipView.cpp:190` vs `src/core/SampleClip.cpp:173`). Um `QMessageBox` sai de dentro do
   modelo (`src/core/SampleBuffer.cpp:86-90`). Multicanal é descartado (`src/core/SampleDecoder.cpp:85-91`).

10. **Captura de entrada insuficiente.** Um único ring buffer estéreo (`src/core/AudioEngine.cpp:176-271`),
    alimentado só por JACK e SDL. `processBufferedInputFrames` não garante que o bloco lido corresponda ao período
    que está sendo renderizado. **[inferência]** Não existe carimbo de tempo (frame do engine) por bloco capturado,
    e isso é indispensável para alinhar a gravação e compensar latência.

11. **Concorrência frágil.** O mutex global trava o período inteiro (`src/core/AudioEngine.cpp:424`), `m_clips` é
    alterado sem lock garantido (`src/core/Track.cpp:328-352`), `SampleClip::m_isPlaying` é um `bool` comum
    compartilhado entre threads (`include/SampleClip.h:109`) e as conexões DirectConnection de tempo fazem lógica de
    GUI e modelo rodar na thread de áudio (`src/core/SampleClip.cpp:54`; `src/core/Song.cpp:99-102`).

12. **Bagagem de padrões.** `SampleTrack` e `SampleClip` têm ramos para o `PatternStore` (`src/tracks/SampleTrack.cpp:81-103`;
    `src/core/SampleClip.cpp:242-246`) que uma faixa de gravação não deveria herdar.

13. **Detalhes menores encontrados:**
    - `Song::stop()` liga `m_recording = true` (`src/core/Song.cpp:653`);
    - o reverso não entra no journal e não é desligado pela carga (`src/gui/clips/SampleClipView.cpp:341-347`;
      `src/core/SampleClip.cpp:370-374`);
    - `m_startFrameOffset` não é salvo (`src/core/SampleClip.cpp:306`);
    - o comentário de `Clip::movePosition/changeLength` promete journal que não existe (`include/Clip.h:118-130`);
    - o fade da `SamplePlayHandle` mistura taxas e usa o índice de render-ahead
      (`src/core/SamplePlayHandle.cpp:102,120-132`).

---

### 9. Implicações para o desenho (sugestões, não decisões)

- **Tipo de faixa novo** (`AudioTrack`/"faixa de gravação"), fora do `PatternStore`, que herda de `SampleTrack`
  apenas os modelos de entrada, arme e monitoração (ou os move para uma base comum).
- **Item de áudio novo** (sem herdar a mecânica de `SampleClip`), com:
  - posição e comprimento em **frames do projeto** (int64) ou segundos (double), com base de tempo por item;
  - `sourceOffset` em frames da fonte, ganho, fades em frames, mute e travamento;
  - lista de **takes**, cada um com uma referência a uma `MediaSource` imutável (arquivo + taxa + canais + duração)
    e um índice de take ativo ou lane.
- Uma **`MediaSource`/`MediaPool`** no nível do projeto, dona dos arquivos na pasta de mídia, com caminhos
  relativos, estado offline, picos em arquivo e leitura em streaming por uma thread de disco.
- **Reprodução** a partir de um snapshot imutável por faixa (lista de segmentos visíveis já resolvida, com
  sobreposição e crossfade), sem `new` nem `src_new` na thread de áudio, com início em qualquer frame do período e
  não só em ticks.
- **Gravação**: a captura com carimbo de frame vai para um ring buffer por faixa armada, a thread de disco escreve
  WAV/FLAC, e ao parar cria um item com take novo. O undo guarda só metadados.
- Um **mapa de tempo** separado, se a régua musical continuar mandando na edição de MIDI. O áudio fica em tempo
  absoluto.

---

## Anexo C: 03 — Interface (UI) relevante para a gravação multipista

Levantamento só de leitura do código em `master` (commit `2395a2c0d`, 12/11/2026). Todas as referências são
`arquivo:linha` no repositório `/Users/castrot1/Desktop/LinuxMusic`. O que é suposição está marcado **(suposição)**.

---

### 1. Resumo executivo

- **Não existe nenhuma UI de gravação de áudio hoje.** O núcleo já tem os modelos por faixa (`SampleTrack`:
  `recordArmModel`, `monitorModel`, `AudioInput`), mas **nenhum widget os usa**: `grep` por `recordArmModel`,
  `monitorModel`, `setInput` ou `AudioInput` em `src/gui` não retorna nada. A gravação herdada
  (`SampleRecordHandle`, "Set/clear record" em clipes de sample) já foi removida.
- Os botões de gravar da barra de transporte **só gravam MIDI** no clipe aberto no Piano Roll
  (`src/gui/TransportBar.cpp:161-163`, `249-260`).
- O cabeçalho de faixa é estreito (32 px de altura padrão, mínimo 32) e já está cheio. Arm, monitor, seletor de
  entrada e medidor de entrada não cabem sem rever o layout.
- **O maior problema estrutural** é o desenho dos clipes de áudio. Cada mudança de posição horizontal invalida
  **todos** os clipes de **todas** as faixas. Cada `SampleClipView` então redesenha a forma de onda na largura
  **inteira** do clipe, mesmo a parte fora da tela. A miniatura (`SampleThumbnailCache`) é imutável, calculada de uma
  vez, mono, com cerca de 8 bytes por frame, e é construída com o motor de áudio travado. Nada disso aguenta tomadas
  longas nem forma de onda ao vivo.
- Posições e comprimentos de clipe são em **ticks** (192 por compasso). O zoom máximo é 400 px/compasso. O snap é só
  musical. Isso não serve para editar áudio com precisão de amostra **(impacto deduzido do código)**.

---

### 2. Estrutura da janela principal hoje

#### 2.1 MainWindow: páginas, docks e janelas MDI

- O espaço de trabalho é um `QMdiArea` customizado (`MovableQMdiArea`), em `src/gui/MainWindow.cpp:164` e
  `1763-1834`.
- **Páginas:** um `QStackedWidget m_pages` é criado como **filho do viewport do MDI**
  (`src/gui/MainWindow.cpp:189`) e é empurrado para o fundo com `m_pages->lower()` (`:209`). Assim as janelas MDI
  (instrumentos, efeitos, `SampleTrackWindow`) flutuam por cima das páginas.
  - O tamanho é mantido à mão por um event filter no `Resize` do viewport (`:1419-1423`). Na prática é um hack de
    layout: as páginas não estão num layout Qt.
  - **Página 0, "Composição":** `m_dockSplitter`, um `QSplitter` vertical com dois `QTabWidget`.
    - `m_topDock` (`:192-194`) recebe o Song Editor e o Pattern Editor.
    - `m_bottomDock` recebe o Piano Roll e o Automation Editor (`:504-510`).
  - **Página 1, "Mixagem":** `m_mixingPage`, que contém só o `MixerView` (`:202-207`, `:506`).
  - **Dock direito:** `m_rightDock`, um `QTabWidget` no splitter principal, com Controladores e Notas
    (`:213-215`, `:509-510`). Começa recolhido (`:519`).
- **Troca de página:** botões "COMPOSITION" e "MIXING" na direita da toolbar (`:430-451`), `showPage()`
  (`:535-546`) e Ctrl+5 (`:1154`).
- **Encaixe:** `dockWidget()` (`:523-531`) embrulha cada editor num `DockPanel` (`include/DockPanel.h`). O
  `revealDock()` (`:565-...`) troca para a página de composição quando um editor docado pede para aparecer.
- **Janelas de plugin e de faixa:** `addWindowedWidget()` (`:644-667`) cria uma `SubWindow` (subclasse de
  `QMdiSubWindow`) dentro do viewport. `SampleTrackWindow` e as janelas de instrumento/efeito usam esse caminho.
- **Timer de UI:** `m_updateTimer.start(1000 / 60, this)`, a 60 fps (`:240`). `timerEvent` emite
  `periodicUpdate()` (`:1499-1502`). Medidores, CPU, `TimeDisplayWidget`, mixer etc. se atualizam por esse sinal
  (veja o `grep` em `include/MainWindow.h:311`).

#### 2.2 Toolbar e TransportBar

- **Toolbar:** um `QWidget` com altura fixa de 64 px (`src/gui/MainWindow.cpp:226`) e um `QGridLayout`:
  salvar e ajuda à esquerda, `TransportBar` no centro e o seletor de páginas à direita (`:453-460`).
- **Transporte duplicado oculto:** os botões de transporte próprios do Song Editor e do Pattern Editor são
  escondidos (`:475-476`, `Editor::hideTransportButtons` em `src/gui/editors/Editor.cpp:136-142`).
- **`TransportBar`** (`src/gui/TransportBar.cpp`, 279 linhas), da esquerda para a direita:
  - metrônomo (`:129-135`);
  - "Part" e "Song", o modo do Play (`:137-156`);
  - **Play, Record, Record-while-playing, Stop** (`:159-172`);
  - `TimeDisplayWidget` (`:177`), tempo, compasso, volume e pitch (`:183-202`);
  - `TransportMeters`, uma barra de CPU e uma de RMS do master (`:206`, `:50-110`).
- **O que os botões de gravar fazem:**
  - `record()` → `getGUI()->pianoRoll()->recordAction()->trigger()` (`:249-252`).
  - `recordWhilePlaying()` → `pianoRoll()->recordAccompanyAction()->trigger()` (`:257-260`).
  - Os tooltips dizem explicitamente "Record MIDI into the clip open in the Piano Roll" (`:161-163`).
  - Os botões **não são checkable** e **não refletem estado de gravação**. `updateState()` só troca o ícone de
    play/pause e o modo Part/Song (`:264-277`).
  - Sem clipe MIDI aberto no Piano Roll, os dois botões não fazem nada, sem nenhum aviso: `PianoRoll::record()`
    retorna se `!hasValidMidiClip()` (`src/gui/editors/PianoRoll.cpp:4347-4350`).
  - `TransportBar::stop()` dispara o stop do Song Editor (`:237-240`). Esse stop chama
    `pianoRoll()->stopRecording()` e limpa a cor de gravação da timeline e da linha de posição
    (`src/gui/editors/SongEditor.cpp:910-916`).

#### 2.3 SongEditor e sua toolbar

- `SongEditorWindow` é construído com `Editor(false, false, false)`, portanto **sem nenhum botão de gravação**
  (`src/gui/editors/SongEditor.cpp:729-730`). Só o Piano Roll usa `Editor(true, true, true)`
  (`src/gui/editors/PianoRoll.cpp:5179`).
- **Toolbars do Song Editor** (`SongEditor.cpp:749-844`):
  - "Track actions": adicionar faixa de pattern, de sample, de automação e pasta (`:749-770`). **Não há "adicionar
    faixa de áudio/gravação".**
  - "Edit actions": Draw, Knife e Select (`:774-788`).
  - "Bar insert controls": inserir/remover compasso (`:793-799`).
  - Zoom: um slider (`:804-818`).
  - Snap: combo, botão de snap proporcional e rótulo (`:822-844`).
- **Zoom:** de 4 a 400 px por compasso, em 200 passos exponenciais (`SongEditor.cpp:66-68`, `628-654`). O padrão
  é 128 px/compasso (`include/TrackContainerView.h:182`). O zoom fica salvo em config (`:181`).
- **Snap:** `SNAP_SIZES` vai de 8 compassos a 1/16 de compasso. O snap proporcional vai de 64 a 1/64
  (`:70-71`, `:201-216`). Só existe grade musical.
  - Não há snap a segundos ou amostras, a bordas de outros clipes nem a transientes.
  - Para não quantizar, segura-se Ctrl ou Alt (`src/gui/clips/ClipView.cpp:1439-1442`).
- **Rolagem horizontal:** o `QScrollBar m_leftRightScroll` é próprio (`SongEditor.cpp:133-139`); o
  `QScrollArea` só rola na vertical. `scrolled()` emite `positionChanged` (`:237-242`). Cada
  `TrackContentWidget::changePosition` **move todos os ClipViews** (`TrackContentWidget.cpp:222-238`). A rolagem
  horizontal é simulada movendo widgets.
- **Seguir a reprodução:** `updatePosition()` (`SongEditor.cpp:578-605`) tem o modo Stepped (pula de página) e o
  modo Continuous (`setValue` a cada mudança de posição, `:596-599`).

#### 2.4 Timeline e linha de posição

- **`TimeLineWidget`:** altura 32 (`SongEditor.cpp:94`). A régua mostra **só números de compasso**
  (`src/gui/editors/TimeLineWidget.cpp:134-...`). Não há régua em tempo (min:s) nem em amostras. O
  `TimeDisplayWidget` da toolbar alterna entre min:s e compassos (`src/gui/widgets/TimeDisplayWidget.cpp:62-130`).
  - O widget tem `setRecording(bool)`, que é só uma flag (`include/TimeLineWidget.h:161-162`), e o pixmap
    `recording_playpos_marker` (`:201`).
- **`PositionLine`:** um widget filho do SongEditor com a altura total das faixas (`SongEditor.cpp:609-623`).
  `setRecording()` troca a cor para vermelho (`src/gui/editors/PositionLine.cpp:38`, `46-58`).
- **Origem da atualização:** `Timeline::incrementTicks()` emite `positionChanged` a cada tick inteiro
  (`include/Timeline.h:63-68`). Ele é chamado **da thread de áudio**, dentro de `Song::processNextBuffer`
  (`src/core/Song.cpp:274-284`), então o sinal chega à GUI por conexão enfileirada.
  - **(Suposição, pela aritmética):** a 120 BPM em 4/4 são cerca de 96 ticks/s, ou seja, ~96 eventos/s
    enfileirados na GUI. Cada um chama `SongEditor::updatePosition` e `TimeLineWidget::update`. A frequência não
    é limitada a 60 Hz como o resto da UI.

---

### 3. Faixas: TrackContainerView, TrackView e cabeçalho

#### 3.1 TrackContainerView

- É um `QScrollArea` com um `QVBoxLayout` que contém **um `TrackView` (QWidget) por faixa**
  (`src/gui/editors/TrackContainerView.cpp:97-112`). Não há virtualização: todas as faixas e todos os clipes
  existem como widgets.
- `addTrackView()` liga `positionChanged` → `TrackContentWidget::changePosition` (`:160-169`).
- **Largura do cabeçalho** (`m_trackHeadWidth`): padrão 338 px ou compacto 214 px (`:90`,
  `include/TrackView.h:51-57`). O usuário pode arrastar a borda (`TrackView.cpp:309`).

#### 3.2 TrackView (`src/gui/tracks/TrackView.cpp`)

- O layout é um `QHBoxLayout` com três widgets (`:69-74`): `TrackOperationsWidget` (largura fixa
  `TRACK_OP_WIDTH = 78`), `m_trackSettingsWidget` (largura do cabeçalho − 78) e `TrackContentWidget` (o resto).
  - O resize força tamanhos fixos (`:120-126`).
- **Altura:** `setFixedHeight(m_track->getHeight())` (`:75`, `:177`).
  - `MINIMAL_TRACK_HEIGHT = DEFAULT_TRACK_HEIGHT = 32` (`include/Track.h:60-61`).
  - Para redimensionar, arrasta-se a borda inferior ou usa-se Shift (`:237`, `:419-424`; veio do PR #7878).
  - Os construtores das views também chamam `setFixedHeight(32)` (`SampleTrackView.cpp:59`,
    `InstrumentTrackView.cpp:72`, `FolderTrackView.cpp:43`).
- **Recuo de pasta:** `FolderIndent * folderDepth` nas margens do settings widget (`:100`).

#### 3.3 TrackOperationsWidget (`src/gui/tracks/TrackOperationsWidget.cpp`)

- Contém o grip, o botão de engrenagem (menu) e os botões **M** e **S** (`AutomatableButton`), em
  `:61-106`. O CSS fixa os botões em 14×14 px (`data/themes/linuxmusic/style.css:435-445`).
- **Menu da engrenagem** (`updateMenu`, `:~200-313`): clonar, limpar, remover, menu do canal do mixer
  (`:259-262`), mover para dentro ou fora de pasta (`:265-281`), cor da faixa.
  - Para faixas de instrumento há o **menu MIDI** (entrada e saída), o pin connector e "um canal por saída"
    (`:283-297`). É o análogo natural para um futuro **menu "Entrada de áudio"** nas faixas de sample/gravação.
  - "Turn all recording on/off" existe **só para faixas de automação**: liga a gravação de automação nos clipes
    (`:298-302`, `:317-341`).

#### 3.4 TrackLabelButton (`src/gui/tracks/TrackLabelButton.cpp`)

- É um `QToolButton` com ícone de 24 px e o nome da faixa, com **altura fixa de 29** (`:58-61`). Em modo compacto,
  renomeia via `RenameDialog`; senão, via um line edit sobreposto (`:72-100`).
- Na faixa de sample, o botão é checkable e abre e fecha a `SampleTrackWindow` (`SampleTrackView.cpp:61-64`,
  `:179-182`).

#### 3.5 SampleTrackView: o cabeçalho atual da faixa de áudio (`src/gui/tracks/SampleTrackView.cpp`)

O layout é uma linha só, num `QHBoxLayout` dentro de um `QVBoxLayout` (`:92-104`):

```
[grip][⚙][M][S] | [ícone + nome (expande)] [LCD canal do mixer] [LED atividade 8×28] [VOL 17px] [PAN 17px] +4px
   78 px fixos  |  ← largura do cabeçalho − 78 (260 px no padrão, 136 px no compacto) →
```

- **LCD do canal do mixer:** `MixerChannelLcdSpinBox` de 2 dígitos (`:66`). Fica **oculto** quando o cabeçalho
  é menor que `COMPACT_TRACK_WIDTH` (`:67-73`).
- **VOL e PAN:** `KnobType::Small17` com `LegacyFixedFontSize` (`:75-83`).
- **LED de atividade:** um `FadeButton` 8×28 (`:85-87`), que também mostra "áudio corrompido"
  (`corruptStateUpdate`, `:257-270`, a 60 Hz via `periodicUpdate`, `:107`).
- **`TrackChannelControls`:** liga os knobs e o M/S ao modelo certo (faixa ou canal) e segue as mudanças de canal
  (`:189-192`; `src/gui/tracks/TrackChannelControls.cpp:42-166`).
- **Arrastar arquivo:** cria um clipe na posição quantizada pelo snap do **Song Editor global**
  (`SampleTrackView.cpp:207-229`).

**Onde caberiam os controles de gravação (proposta, não implementado):**

| Controle | Opção A (no próprio cabeçalho) | Opção B (fora dele) |
|---|---|---|
| **Armar (●)** | No `TrackOperationsWidget`, ao lado de M/S, do mesmo tamanho de 14 px. Exige aumentar `TRACK_OP_WIDTH` (78), que é constante global usada por todas as faixas, ou tirar o grip ou a engrenagem da linha | — |
| **Monitor (3 estados: off, on, auto)** | Botão ao lado do armar, em `NStateButton` (`src/gui/widgets/NStateButton.cpp`, já existe) | Menu de contexto |
| **Seletor de entrada** | Pequeno combo/tool button no settings widget, no lugar do LCD do canal quando a faixa estiver armada | Submenu "Entrada de áudio" no menu da engrenagem, como o menu MIDI do instrumento (`TrackOperationsWidget.cpp:283-287`); e na `SampleTrackWindow` |
| **Medidor de entrada** | Barra vertical fina no lugar do `FadeButton` 8×28 (mesmo slot, `SampleTrackView.cpp:85-87`), ou barra horizontal sob o nome numa segunda linha (exige altura > 32) | — |

Problema de espaço: a 32 px de altura e 136 px de settings no modo compacto, não cabe nada sem esconder algo. Um
layout tipo REAPER (duas linhas: nome e botões em cima, medidor e entrada embaixo) precisa de altura mínima de
~48–56 px para essas faixas **(estimativa)**.

#### 3.6 SampleTrackWindow (`src/gui/SampleTrackWindow.cpp`)

- É uma janela MDI (`addWindowedWidget`, `:155`) com nome, M/S, VOL e PAN (`Bright26`), o LCD "CHAN" e um
  `EffectRackView` de tamanho fixo `DEFAULT_WIDTH × 242` (`:146-147`). Tem `SetFixedSize` (`:162`).
- **Não tem entrada, armar nem monitor.** É o lugar mais simples para um primeiro seletor de entrada (mono ou estéreo
  e primeiro canal, correspondendo a `AudioInput{source, firstChannel, channelCount}`, `include/AudioInput.h:33-45`)
  e para o modo de monitoração.
- Duplica os controles do cabeçalho (M/S, VOL, PAN, canal), ligados pelo mesmo `TrackChannelControls::replace`
  (`:199-200`).

---

### 4. Clipes de áudio: SampleClipView e ClipView

#### 4.1 Desenho (`src/gui/clips/SampleClipView.cpp:177-339`)

- **Cache em pixmap:** se `!needsUpdate()`, só copia `m_paintPixmap` (`:181-185`). Senão, cria ou reaproveita um
  pixmap de largura **2 × largura do TrackView**, posicionado em `pe->rect().x() − trackViewWidth` (`:189-201`).
- **Forma de onda:** `SampleThumbnailRenderer::visualize` com `rect = (offsetStart, …, sampleLength, …)` (`:285-294`),
  onde `sampleLength` é o comprimento **inteiro** da amostra em pixels.
  - O renderer percorre `for (out = 0; out < rect.width(); ++out)` com `std::accumulate` por pixel
    (`src/gui/SampleThumbnailRenderer.cpp:63-79`). Não recorta pela área visível nem pela largura do pixmap; o
    `QPainter` só descarta o que cai fora.
  - Antialiasing ligado (`:61`).
  - **Consequência (deduzida):** num clipe de 10 min a 400 px/compasso e 120 BPM (≈ 300 compassos ≈ 120 000 px),
    cada repaint faz ~120 000 iterações de `drawLine` e `accumulate`, mesmo com o pixmap tendo só ~2× a largura da
    tela.
- **Fades e crossfades:** `drawCrossfade()` (`src/gui/clips/ClipView.cpp:458-550`) desenha curvas e alças. Veio do
  PR #7900.
- **Inconsistência:** `offsetStart` usa `pixelsPerBar()` mas `sampleLength` usa `ppb`, que difere em `fixedClips()`.
  E `ticksPerBar` é recalculado a partir do compasso (`SampleClipView.cpp:272-279`).

#### 4.2 SampleThumbnailCache (`src/core/SampleThumbnailCache.cpp`, `include/SampleThumbnailCache.h`)

- **Mono:** cada frame vira `average()` de L/R (`:39-42`). Sinais em contrafase somem da miniatura, e não há como
  mostrar os dois canais de uma tomada estéreo.
- **Memória:** o primeiro nível tem **um `Peak` (2 floats) por frame** (`:38-46`), e os níveis seguintes são
  divididos por 4 (`:49-53`). **(Conta):** 10 min a 48 kHz = 28,8 M frames × 8 B ≈ 230 MB, mais ~33% ≈ 300 MB **só
  para a miniatura** de uma tomada. Nada é guardado em disco (não há arquivo de peaks como o `.reapeaks`).
- **Imutável:** é `shared_ptr<const SampleThumbnailCache>`, criado de uma vez a partir do buffer inteiro. Não há
  como acrescentar frames, o que é necessário para a forma de onda ao vivo durante a gravação.
- **Construída com o motor de áudio travado:** `SampleClip::setSampleBuffer` monta a miniatura **dentro** de
  `requestChangesGuard()` (`src/core/SampleClip.cpp:148-152`). Para um buffer longo, isso para o áudio pelo tempo
  do cálculo. Por arquivo, a miniatura vem de `SampleCache::fetchThumbnailCache` (`include/SampleCache.h:48-52`),
  também síncrona na thread da GUI **(suposição: não vi nenhuma thread de trabalho nesse caminho)**.
- Reaproveitada em `AutomationEditor` (ghost), AudioFileProcessor e SlicerT. Mudar a API afeta esses três.

#### 4.3 Edição disponível hoje

| Operação | Onde | Observação |
|---|---|---|
| Mover | `ClipView::mousePressEvent/MoveEvent`, `Action::Move` (`ClipView.cpp:676-680`) | quantizado pelo snap; Ctrl/Alt livre |
| Aparar direita/esquerda | `Action::Resize` / `ResizeLeft` (`:659-667`, `:895-1060`) | ResizeLeft ajusta `startTimeOffset` |
| Deslocar o conteúdo (slip) | `setStartTimeOffset` (`:999-1004`) | (suposição: o atalho exato não foi verificado) |
| Cortar (knife) | modo faca, `splitClip()` (`:1566-1590`); com Shift, `destructiveSplitClip` (`:1177-1187`) | o corte normal clona o clipe e ajusta o offset |
| Fade in/out e tensão | alças de crossfade (`:594-611`, `:639-657`) | PR #7900 |
| Inverter | menu "Reverse sample" → `Sample::setReversed` (`SampleClipView.cpp:342-348`) | não destrutivo |
| Exportar buffer | "Export sample buffer", FLAC (`:359-367`) | PR #7627 |
| Trocar o sample | duplo clique abre arquivo (`:160-174`); arrastar e soltar arquivo (`:117-140`) | |
| Ganho por clipe | **não existe** na UI | `sample.amplification()` só é lido no desenho |
| Takes e lanes | **não existe** | |

- **Resolução:** `TimePos` em ticks, `DefaultTicksPerBar = 192` (`include/TimePos.h:38`). Posição, comprimento,
  `startTimeOffset` e fades são `TimePos` (`include/Clip.h:82-154`). A 120 BPM, 1 tick ≈ 10,4 ms
  **(conta)**: nenhuma edição chega à precisão de amostra.
- **Zoom máximo de 400 px/compasso:** a 120 BPM são ~200 px/s, então não é possível ver nem editar no nível da
  amostra.

---

### 5. Gravação na UI hoje: inventário

| Elemento | Onde | O que faz de fato |
|---|---|---|
| Record (toolbar) | `TransportBar.cpp:161`, `249-252` | Dispara `PianoRoll::record()`: toca **só o clipe MIDI aberto** (`playMidiClip`) e grava notas MIDI nele (`PianoRoll.cpp:4341-4359`). Sem clipe aberto, não faz nada |
| Record while playing (toolbar) | `TransportBar.cpp:162`, `257-260` | `PianoRoll::recordAccompany()`: toca a música ou o pattern e grava MIDI no clipe aberto (`PianoRoll.cpp:4364-4391`). Pinta de vermelho a timeline e a linha do Song Editor |
| Record, accompany e step (toolbar do Piano Roll) | `Editor.cpp:122-124`, `PianoRoll.cpp:5179` | Os mesmos, mais a gravação passo a passo (`toggleStepRecording`, `:4395-4420`) |
| Notas gravadas | `PianoRoll::startRecordNote/finishRecordNote` (`PianoRoll.cpp:4441-4510`) | Juntam notas em `m_recordingNotes` e, opcionalmente, quantizam (configuração "Behavior when recording", `SetupDialog.cpp:757-772`) |
| "Set/clear record" | menu do clipe de **automação** (`AutomationClipView.cpp:179-181`) | Grava automação (ícone em `:371-372`; processamento em `src/core/Song.cpp:402`) |
| "Turn all recording on/off" | menu da faixa de automação (`TrackOperationsWidget.cpp:298-302`) | Idem, em todos os clipes da faixa |
| Página "Composição e gravação" | tooltip em `MainWindow.cpp:437` | Só um nome; não há nada de gravação de áudio nela |
| Arm, monitor e entrada de áudio | `SampleTrack` (`include/SampleTrack.h:71-122`, `src/tracks/SampleTrack.cpp:50-52`, `188-238`) | **Só modelo e serialização. Sem UI e sem motor.** |

**Restos herdados (não são UI, mas confundem):** `Song::playAndRecord()` (`src/core/Song.cpp:528-532`) não é
chamado por ninguém. `Song::stop()` liga `m_recording = true` (`:653`), o que parece um bug herdado
**(suposição sobre a intenção)**.

---

### 6. Configurações de áudio (SetupDialog e widgets dos backends)

`src/gui/modals/SetupDialog.cpp:481-653`. A aba "Audio" tem:

- um combo da interface (backend), `:490-563`;
- um widget específico do backend, `:497-561`;
- a taxa de amostragem, com slider sobre `SUPPORTED_SAMPLERATES` e aviso de reinício, `:565-601`;
- o tamanho do buffer em passos de 32, com aviso de reinício, `:603-637`;
- "mix sanitization", `:639-644`.

**Não há** latência de entrada nem compensação manual, nem a lista de canais de entrada para as faixas.
Toda mudança exige **reiniciar** o programa (`restartWarningLbl`, `:907-923`).

| Backend | Arquivo | Escolha de entrada no diálogo | Captura no driver |
|---|---|---|---|
| JACK | `src/core/audio/AudioJack.cpp:455-520` | **2 entradas fixas** (`numberOfInputChannels = 2`, `:512`), cada uma ligada por um menu a uma porta JACK | `m_supportsCapture = true` (`:109`); copia `channels()` portas para um `SampleFrame` estéreo (`:425-433`) |
| ALSA | `src/gui/AudioAlsaSetupWidget.cpp` | só o dispositivo de **saída** e "Channels" travado em 2 (`setRange(DEFAULT_CHANNELS, DEFAULT_CHANNELS)`) | não captura (não há `pushInputFrames` em `AudioAlsa.cpp`) |
| PulseAudio | `src/core/audio/AudioPulseAudio.cpp:255-292` | dispositivo como texto e canais travados em 2 | não captura |
| SDL | `src/core/audio/AudioSdl.cpp:192-240` | **combo "Input device"** (`:204-208`) | abre a captura sempre que possível (`:99-111`) e empurra os frames (`:187`) |
| PortAudio | `src/core/audio/AudioPortAudio.cpp:211-311` | "Input device" com **número de canais** até o máximo do dispositivo (`:211-269`) | **o callback ignora a entrada** (o primeiro parâmetro de `processCallback` não tem nome, `:191-206`) e `m_supportsCapture` nunca vira true. O documento `docs/etapa-3-gravacao.md` diz que o PortAudio captura sempre; o código atual não mostra isso |
| sndio, Dummy | — | — | não |

- O motor só tem **um buffer de entrada estéreo** (`std::array<std::vector<SampleFrame>, 2> m_inputBuffer`,
  `include/AudioEngine.h:242-250`, `385-387`). Hoje um `AudioInput{firstChannel = 3}` não teria de onde ler, e um
  seletor de entrada na faixa só poderia oferecer "1", "2" e "1+2" até o motor suportar N canais.
- **Não existe** uma API para a GUI listar os canais de entrada disponíveis (nome e quantidade) para preencher um
  seletor por faixa. Só existe `AudioEngine::captureDeviceAvailable()` (`src/core/AudioEngine.cpp:550-553`).

---

### 7. Problemas estruturais que vão complicar a UI de gravação

#### 7.1 Desempenho de desenho e repaint

1. **Invalidação total a cada rolagem horizontal:**
   - `SongEditor::scrolled` → `positionChanged` → `TrackContentWidget::changePosition`, que move **todos** os
     clipes e chama `updateBackground()` (`TrackContentWidget.cpp:222-240`).
   - `updateBackground()` **realoca e redesenha um `QPixmap`** de grade por faixa (`:90-151`) e termina em
     `update()`. Esse `update()` é o `TrackContentWidget::update()` que **esconde** o `QWidget::update` não virtual e
     chama `clipView->update()` em **todos** os clipes (`:185-193`).
   - O `ClipView::update()` liga `m_needsUpdate = true` (`ClipView.cpp:152-160`). Com isso **todos os
     `SampleClipView` refazem a forma de onda inteira** (§4.1).
   - No modo de autoscroll Continuous isso acontece a cada evento de posição (§2.4). Com várias faixas gravando ou
     tomadas longas, é o gargalo principal **(dedução pelo código, não medida)**.
2. **Waveform sem recorte de área visível** (§4.1) e **miniatura sem LOD em disco** (§4.2).
3. **Um widget por clipe com largura em pixels igual ao comprimento** (`ClipView::updateLength`,
   `ClipView.cpp:206-219`), posicionado com x negativo quando rolado. **(Suposição):** clipes de centenas de
   milhares de pixels podem esbarrar em limites de coordenadas de 16 bits em alguns backends de pintura e de
   janela; ainda não foi testado.
4. **Posição via sinal enfileirado da thread de áudio** a cada tick (§2.4), sem limitar a 60 Hz. Durante a
   gravação, o desenho ao vivo deveria usar o timer de 60 Hz (`periodicUpdate`) e ler a posição atômica.
5. **Medidores consumíveis:** `ChannelMeter::takePeaks()` **consome** o pico desde a última leitura
   (`include/ChannelMeter.h`). O comentário em `TransportBar.cpp:76` admite que o mixer "consome" os picos do
   master e por isso a toolbar mostra RMS. Um medidor de entrada mostrado em dois lugares (cabeçalho da faixa e
   mixer) precisa de um leitor por consumidor ou de picos com decaimento, não "take".

#### 7.2 Acoplamento modelo/visão e globais

- **O estado de gravação MIDI mora na view.** `PianoRoll::m_recording` (`PianoRoll.cpp:171`) é consultado pelo
  **núcleo, na thread de áudio**: `InstrumentTrack.cpp:797` chama `gui::getGUI()->pianoRoll()->isRecording()`.
  Uma gravação de áudio não deve repetir esse padrão; o estado "gravando" precisa estar no `Song` ou num
  controlador de gravação no core.
- **A TransportBar chama ações de editores específicos** (`TransportBar.cpp:226-260`), não um modelo de
  transporte. Um "Gravar" global (armar → gravar todas as faixas armadas) não tem onde se ligar hoje.
- **As views dependem do Song Editor global:**
  - `ClipView` conecta a `getGUI()->songEditor()->m_editor` (`ClipView.cpp:120`);
  - `TrackContentWidget::updateBackground` lê o snap do Song Editor (`:93`);
  - `SampleTrackView::dropEvent` também (`:217`);
  - `TrackContentWidget` testa `== getGUI()->patternEditor()->m_editor` (`:198`, `:550`).
- **O modelo de cor de gravação é duplicado:** a timeline e a linha de posição têm flags `setRecording` separadas,
  ligadas e desligadas manualmente em três lugares (`PianoRoll.cpp:4357-4358`, `4389-4390`, `4433-4436`;
  `SongEditor.cpp:914-915`).
- **`SampleClip` com buffer imutável em RAM:** `shared_ptr<const SampleBuffer>`, e trocar o buffer trava o motor
  (`SampleClip.cpp:144-157`). Gravar direto em disco com forma de onda crescente pede um tipo de clipe e uma view
  novos (o que já está planejado em `docs/etapa-3-gravacao.md`, "faixa de gravação").

#### 7.3 Tamanhos fixos e widgets legados

- **Constantes de cabeçalho:** `TRACK_OP_WIDTH = 78`, `DEFAULT_TRACK_WIDTH = 338`, `COMPACT_TRACK_WIDTH = 214`,
  `MINIMUM_TRACK_WIDTH = 168` (`include/TrackView.h:51-57`). Altura mínima e padrão: 32 (`include/Track.h:60-61`).
- **Tamanhos fixos nos widgets:**
  - `TrackLabelButton`: altura 29 (`TrackLabelButton.cpp:58`);
  - `FadeButton`: 8×28;
  - knobs `Small17` com `LegacyFixedFontSize`;
  - toolbar principal: 64 px;
  - `TimeLineWidget`: 32 px;
  - rack da `SampleTrackWindow`: 242 px;
  - botões M/S: 14 px fixos no CSS.
- **Dois temas com CSS paralelo** (`data/themes/default/style.css` e `data/themes/linuxmusic/style.css`; o commit
  `d426dc4db` alterou os dois). Todo widget novo de gravação precisa de estilo duplicado.
- **Widgets legados misturados:** `LcdSpinBox` para o canal, `AutomatableButton` e `PixmapButton`, sinais e slots
  no formato `SIGNAL()`/`SLOT()` em boa parte do código (por exemplo `SongEditor.cpp:760-799`,
  `TrackOperationsWidget.cpp`). E o hack das páginas dentro do viewport do MDI (§2.1).

#### 7.4 Código duplicado

- **Mixer:** `createMixerMenu`, `createMixerLine` e `assignMixerLine` idênticos em `SampleTrackView`
  (`:140-245`) e `InstrumentTrackView` (`:266-440`). Os próprios arquivos têm `FIXME` dizendo isso
  (`SampleTrackView.cpp:140`, `InstrumentTrackView.cpp:413`).
- **Layout do cabeçalho:** montado à mão e repetido em `SampleTrackView` (`:58-104`), `InstrumentTrackView`
  (`:72-…`) e `FolderTrackView` (`:43-82`). Acrescentar arm, monitor e medidor exige mexer em cada um ou criar uma
  base comum.
- **Controles da faixa:** M/S, VOL, PAN e canal são repetidos entre o cabeçalho e a `SampleTrackWindow`.

---

### 8. Recomendações para a UI de gravação (para planejar a etapa 3)

1. **Transporte:** separar "Gravar MIDI no Piano Roll" de um **Gravar global** que grava as faixas armadas. O botão
   deve ser checkable e refletir o estado. O estado deve morar no core (`Song` ou um `RecordingController`), nunca
   numa view.
2. **Cabeçalho:** criar uma base comum de cabeçalho com slots (arm, monitor, entrada, medidor) e permitir altura
   maior para faixas de gravação. Colocar arm e monitor junto a M/S e o medidor no lugar do `FadeButton`.
3. **Entrada:** menu "Entrada de áudio" na engrenagem (como o menu MIDI) e na janela da faixa, preenchido por uma
   API nova do motor que liste os canais de entrada.
4. **Forma de onda:** nova miniatura incremental, por canal, com LOD calculado fora da thread de áudio e salvo em
   disco. O renderer deve recortar a área visível, e a pintura não deve ser invalidada em massa a cada rolagem.
   Corrigir `TrackContentWidget::update()` e `updateBackground()` antes de pôr tomadas longas na tela.
5. **Tempo:** posições de clipe em frames (ou ticks + offset em frames), zoom além de 400 px/compasso, régua em
   tempo e snap opcional a segundos e a bordas.
6. **Configurações:** latência e compensação manual, quantidade de entradas por backend (ALSA e Pulse sem captura,
   PortAudio ignorando a entrada) e alguma forma de mudar sem reiniciar.

---

## Anexo D: 04 – Histórico: o que já existe sobre gravação de áudio (LinuxMusic e LMMS)

Levantamento só de leitura, feito em 12/11/2026 sobre o repositório `/Users/castrot1/Desktop/LinuxMusic`
(branch `master`, HEAD `2395a2c0d`; `origin/master` aponta para o mesmo commit). Nada foi editado, commitado ou
enviado.

---

### 1. O que a documentação do projeto já decidiu

#### 1.1 Roteiro geral (`docs/README.md`)

- Três etapas, nesta ordem: 1) limpeza, 2) interface e mixer, 3) gravação estilo REAPER. A ordem é proposital: a
  gravação é construída **sobre** a base nova de canais, "em vez de ser encaixada à força na estrutura antiga".
- Referências de produto: **mixer/plugins = FL Studio**; **gravação = REAPER** (armar faixas, escolher entrada por
  faixa, monitorar, gravar direto em disco).
- Regras: Linux é o destino (macOS só para desenvolver); **formato de projeto pode mudar livremente**; GPL-2.0+;
  depois da etapa 1 as correções do upstream vêm por `cherry-pick`, não por merge.

#### 1.2 Ponto de partida (`docs/estado-atual.md`)

- Base: branch `feature/recording-stage-one` do LMMS ([LMMS#7786](https://github.com/LMMS/lmms/pull/7786)),
  sincronizado com o master do LMMS de set/2026 (1.3.0-alpha.2).
- PRs incorporados: #7903 (ring buffer lock-free da entrada) e #7627 (exportar buffer de clipe).
- **A gravação herdada não é a base**: grava um clipe marcado com "Set/clear record", guarda na memória e embute no
  projeto; não tem entrada por faixa, monitoração nem gravação em disco.

#### 1.3 Etapa 1 (`docs/etapa-1-limpeza.md`)

- Decisão de 11/11/2026: a gravação herdada (`SampleRecordHandle`, "Set/clear record") é **removida** e será
  substituída na etapa 3. Gravação MIDI em tempo real fica. Todos os backends de áudio ficam (a tabela "avaliar
  remover OSS, sndio, SoundIO, SDL e PortAudio" continua sem decisão).
- Executado no commit `d95c6ba49` (ver §3.1).

#### 1.4 Etapa 2 (`docs/etapa-2-interface-e-mixer.md`) e preparação do mixer (`docs/preparacao-mixer.md`)

Partes ligadas à gravação:

- **Passo 4 (feito, `f17200328`)**: o canal do mixer ganhou `ChannelInput`, `m_recordArmModel` e `m_monitorModel`
  (desligada/ligada/automática, padrão automática como no REAPER), salvos no projeto, sem motor.
- **Passo 26 (feito, `2395a2c0d`, 12/11/2026)** – decisão do usuário: *"o input de áudio vai ser somente via
  faixa"*, *"não vamos armar canais da mesa e sim armar faixas"*. Tudo saiu do canal da mesa e foi para a
  `SampleTrack`:
  - `include/AudioInput.h`: `struct AudioInput { Source {None, Hardware}; firstChannel; channelCount (1|2) }`;
  - `SampleTrack::recordArmModel()`, `monitorModel()`, `setInput()`/`inputChanged`; salvo como `recordarm`,
    `monitor` e elemento `<input source="hardware" channel=".." channels=".."/>`.
  - **Ainda não há interface** (nenhum uso de `recordArmModel`/`monitorModel` em `src/gui`) **nem motor**.
- **Passo 7 – PDC (feito)**: `AudioProcessor::latency()`, `EffectChain::latency()`, `Mixer::compensateLatency()`,
  linhas `LatencyDelay`. O doc diz explicitamente: *"`Mixer::latency()` é a latência da mixagem toda. A gravação
  (etapa 3) vai usá-la para alinhar o que for gravado."* Na reprodução, a mixagem sai atrasada pela latência do
  master (não adianta a música), como no REAPER. VST, Carla e ZynAddSubFX ainda informam latência 0.
- **Hierarquia (passo 9)**: pastas/grupos **não gravam** (sem clipes, sem entrada) – commit `ea5e25e37`.
- **Passo 25**: páginas Composição/Mixagem; *"A gravação fica aqui"* (na Composição).
- Qualidade: tudo é compilado e testado só no macOS; **testes no Linux (ALSA/JACK/PulseAudio) ficaram para depois**
  – risco registrado no próprio doc.

#### 1.5 Multicanal (`docs/multicanal.md`)

- Base multicanal pronta (fases 1–3b): `AudioBuffer` planar com grupos de canais, pares auxiliares (3/4, 5/6…),
  pin connector, envios com pares e sidechain. Relevante para gravação: o **mixer** já trata N canais, mas a
  **entrada de áudio do motor continua estéreo** (ver §4).
- Fases 4–7 (nova API de plugins #8275, CLAP/VST3, plugins remotos, medição de desempenho) pendentes.

#### 1.6 Etapa 3 (`docs/etapa-3-gravacao.md`) – decisões tomadas

- Sistema **construído do zero** sobre os canais da etapa 2; **não** reaproveita o stage-one.
- **Tipo de faixa novo: "faixa de gravação" (áudio)**: vários clipes em qualquer posição (inclusive sobrepostos),
  crossfade automático com curva, fade in/out (base possível: LMMS #7900, já mesclado), edição não destrutiva
  (cortar, aparar, slip, ganho por clipe), takes em lanes e comping.
- Entrada, armar e monitorar **só na faixa**; a `SampleTrack` segura esses ajustes até a faixa de gravação existir;
  a `SampleTrack` continua para loops.
- Primeira versão: armar várias faixas; entrada por faixa (mono/estéreo); monitoração off/on/auto; **gravação direta
  em disco (WAV ou FLAC na pasta de mídia do projeto) por thread de disco separada**; forma de onda ao vivo;
  compensação de latência (latência do driver + ajuste manual); medidores de entrada; metrônomo e count-in.
- Depois: punch in/out, loop recording com takes, modos de gravação (entrada, saída/resample, MIDI), recuperação
  após falha.
- Requisitos: JACK/PipeWire principal, ALSA e PulseAudio também; tempo real seguro (sem alocação, mutex ou disco na
  thread de áudio; ring buffers lock-free até a thread de disco); **abrir a captura só quando houver faixa armada
  ou monitorando** (hoje SDL/PortAudio abrem o microfone sempre e o log enche de "input ring buffer overflow"
  impresso da thread de áudio).
- Critério de conclusão: várias faixas ao mesmo tempo no Linux, monitoração, áudio alinhado, arquivos em disco e
  projeto reabrindo com tudo no lugar.

#### 1.7 Outras notas

- `docs/ideias-futuras.md`: "modo sem conversões" (uma taxa de amostragem no projeto inteiro; dispositivo abre na
  taxa do projeto; gravação já sai no formato do projeto, sem conversão).
- `docs/toolbar-nova.md`: o botão Gravar/Gravar tocando da barra nova é **só MIDI** (Piano Roll); áudio vem na etapa 3.
- `docs/pesquisa-ui-existente.md`: #7900 (fades) marcado como "essencial para editar áudio gravado"; #7459 (pin
  connector) útil para entradas de gravação.

#### 1.8 Pendências em aberto (resumo)

1. Criar a faixa de gravação (tipo novo) e migrar entrada/armar/monitorar da `SampleTrack` para ela.
2. Interface para entrada, armar e monitorar (não existe).
3. Motor: captura multicanal por backend, roteamento entrada→faixa, monitoração, escrita em disco, latência.
4. Decidir backends (tabela da etapa 1 em aberto) e testar no Linux de verdade (nunca foi feito neste fork).
5. Formato de projeto para arquivos gravados, takes e pasta de mídia.

---

### 2. Branches relacionados à gravação

Estado medido com `git rev-list --count` e `git merge-base --is-ancestor` contra `master` (`2395a2c0d`).

| Branch | Tip | À frente / atrás do master | Mesclado? | Conteúdo |
|---|---|---|---|---|
| `feature/recording-stage-one` (local, `origin/`, `upstream/`) | `1067fdab1` (05/09/2026, Hyunjin Song, "Use `requestChangesGuard`") | 0 / 429 | **Sim** (é ancestral direto do master) | Ver §2.1 |
| `origin/sample-track-recording-jack` = `upstream/…` | `e09763080` (01/05/2025, Johannes Lorenz) | 6 / 677 | **Não** (mas o conteúdo entrou por squash, ver §2.2) | JACK com entrada |
| `linuxmusic` | `4179863cc` (11/11/2026) | 0 / 236 | Sim | Branch de trabalho antigo; tudo no master |
| `multicanal` | `7429e6d1d` (12/11/2026) | 0 / 2 | Sim | Núcleo multicanal + mixer; tudo no master |
| `pr-7903` | `95bbfd69e` | 0 / 585 | Sim no fork (merge `b0e145268`); aberto upstream | Ring buffer lock-free da entrada |
| `pr-7627` | `f8d5c56a5` | 0 / 643 | Sim no fork (merge `2d8c54fcd`); aberto upstream | Exportar buffer de clipe de sample |
| `pr-7900` | `840b2ad06` | 0 / 666 | Sim no fork (merge `ad11c9290`); aberto upstream | Fades em clipes de sample |
| `pr-7459`, `pr-8118`, `pr-8317`, `pr-7878` | — | 0 / … | Sim | Multicanal/pin connector, Qt6, desacoplar faixas do mixer, redimensionar faixas (não são de gravação) |
| `pr-2944`, `pr-5261`, `pr-6407` | — | 35, 17, 25 à frente | Não | UI (controllers, toolbar, auto-link faixa→canal), sem relação com gravação |

Os demais remotos (`origin/groove`, `feature/bezier`, `new-i18n`, `refac/memory`, `stable-1.2`, `fix-memleak-help` e
os `messmerd/*` de pin connector/planar buffers) não tratam de gravação.

#### 2.1 `feature/recording-stage-one` (LMMS#7786)

O master do LinuxMusic **é** este branch continuado (a primeira linha de pais do master passa por
`4486e1b2c Merge branch 'master' into feature/recording-stage-one`). Commits próprios de gravação (sem os merges do
master do LMMS):

- 2021 – Shmuel Hazan (Reflexe), a origem: `79b709251` "SampleTrack: Uncomment disabled recording-related code",
  `50cc41a5a` PlayHandle sem `audioPort`, `b8938fd7b`/`ed7ff82a4` tocar (gravar) clipe vazio marcado como
  `isRecord`, `7184ce6e5` offset do take por parâmetro em `SampleRecordHandle`, `1b39d39b2` desabilitar botões de
  gravar quando não há captura.
- 2022 – Hyunjin Song: formatação (`99d97fcc3`, `dfed374c0`, `17efb1387`).
- 2024 – Michael Gregorius: entrada configurável no SDL (`33139b9f4`, `29c43c2bb`, `1ab45e499`), opção de gravação
  mais visível (`48c4dce96`, `549553074`), remoção de botão (`d8c271650`); saker: revisões.
- 2025 – mealynn/Johannes Lorenz/Michael Gregorius: **gravação com JACK** (#7567, squash `352ef8c25`, 31/05/2025);
  `2d2c26d50` remove `m_inBuf` (#7922); `997764a0d` seleção de entradas/saídas no JACK (#7919); `63e386707`
  `m_startRecordTimeOffset` const.
- Nov/2025 – Steven Jaro: ring buffer (`5a9c81157`, `c6f804731`, `52868fdaa`) – o PR #7903, mesclado no fork em
  `b0e145268` (11/11/2026).
- 30/04/2026 – `3f1902365` remove o primeiro botão de gravar do Song Editor (#7899).

**O que fazia:** clipe de sample marcado com "Set/clear record"; ao tocar com gravação, um `SampleRecordHandle`
(um *PlayHandle*) lia `AudioEngine::inputBuffer()` a cada período, copiava para blocos `new SampleFrame[]` (alocação
na thread de áudio) e, no destrutor, juntava tudo num `SampleBuffer` em memória que ia para o clipe (salvo em
base64 dentro do projeto). Havia também `AudioSampleRecorder` (um `AudioDevice` que acumulava em memória).

**Situação no fork:** a parte de clipe/handle/UI foi **removida** em `d95c6ba49` (11/11/2026, −503 linhas:
`SampleRecordHandle`, `AudioSampleRecorder`, flag de record no `SampleClip`, botão do Song Editor). Ficou o caminho
de captura dos drivers e o ring buffer.

**Vale reaproveitar?** Só a captura nos drivers (JACK e SDL), a seleção de portas do JACK (#7919) e o ring buffer
(#7903), e mesmo assim com reformas (§4). A lógica de clipe/handle foi corretamente descartada.

#### 2.2 `sample-track-recording-jack`

- 6 commits sobre `6c3d05ca9` (merge de março/2025 no stage-one): `75ea7099d` mealynn "enable recording for the
  jack backend", `3e1a6781c`, `25ddb1f34`/`ecd6a63a0` (buffer de entrada vira `std::vector<SampleFrame>`),
  `3ab8cc5a4` (Michael Gregorius, remove configuração de número de canais do JACK), `e09763080`.
- Diff: `include/AudioJack.h` e `src/core/audio/AudioJack.cpp` (+41/−36): registra portas `master in L/R`,
  `m_supportsCapture = true`, callback de mudança de buffer que redimensiona o vetor de entrada, copia as portas de
  entrada para `SampleFrame` e chama `pushInputFrames`, e fixa o JACK em 2 canais (`DEFAULT_CHANNELS`).
- **Não está mesclado como branch**, mas é obsoleto: foi mesclado por squash no stage-one como **#7567**
  (`352ef8c25`) e está no master. **Nada a reaproveitar** além do que já está no master.

---

### 3. Estado atual do código no master (o que existe de fato)

#### 3.1 Captura

- `AudioDevice::m_supportsCapture` / `AudioEngine::captureDeviceAvailable()`.
- **JACK** (`src/core/audio/AudioJack.cpp`): 2 portas de entrada registradas sempre; o `processCallback` copia
  `nframes` quadros para `m_inputFrameBuffer` e chama `pushInputFrames` a cada ciclo, armado ou não. Seleção de
  portas de entrada/saída na configuração (#7919).
- **SDL** (`AudioSdl.cpp`): abre um dispositivo de captura sempre que consegue; o callback reinterpreta o buffer como
  `SampleFrame` e empurra.
- **PortAudio** (`AudioPortAudio.cpp`): abre stream com entrada se houver dispositivo de entrada configurado, mas o
  callback **ignora** o ponteiro de entrada (`processCallback(const void*, …)`), e não marca `m_supportsCapture`.
  Abre o microfone sem usar.
- **ALSA, PulseAudio, sndio**: sem captura.
- `AudioEngine::pushInputFrames()` (thread do driver) escreve num `LocklessRingBuffer<SampleFrame>` de
  `DEFAULT_BUFFER_SIZE*100` quadros; em overflow faz `fprintf(stderr, …)` da thread de tempo real.
  `processBufferedInputFrames()` (chamado em `renderStageNoteSetup`) copia até `2*DEFAULT_BUFFER_SIZE` quadros
  para um buffer duplo `m_inputBuffer[2]` trocado em `swapBuffers()`.
- **Ninguém consome `inputBuffer()`** desde `d95c6ba49`: o anel enche sem leitor efetivo de gravação – a causa do
  "input ring buffer overflow" citado na etapa 3.

#### 3.2 Modelo de dados

- `AudioInput` + armar + monitorar na `SampleTrack` (`2395a2c0d`), salvos no projeto, sem GUI e sem motor.

#### 3.3 Ligado à gravação, já pronto

- PDC no mixer (`Mixer::latency()`), fades de clipe (#7900), exportar buffer de clipe (#7627), núcleo multicanal.

---

### 4. Problemas estruturais conhecidos (código, comentários e docs)

Encontrados no código do master (citados por arquivo):

1. **Entrada só estéreo e intercalada.** Todo o caminho de entrada usa `SampleFrame` (2 canais): o anel, o
   `m_inputBuffer`, o JACK (`DEFAULT_CHANNELS` portas) e o SDL. `AudioInput::firstChannel` acima de 1 não tem de onde
   vir. Gravação multitrack estilo REAPER exige captura de N canais planar, por dispositivo.
2. **Captura sempre ligada** (JACK, SDL; PortAudio abre e descarta), sem leitor → overflow e `fprintf` na thread de
   áudio (`AudioEngine::pushInputFrames`). Já registrado em `etapa-3-gravacao.md`.
3. **Sem relação temporal entre entrada e saída.** O anel não carrega carimbo de tempo/posição; o JACK empurra
   `nframes` do ciclo do JACK enquanto o motor renderiza em `framesPerPeriod` próprios; `processBufferedInputFrames`
   limita a `2*DEFAULT_BUFFER_SIZE` por período. Não dá para saber a que quadro do projeto corresponde cada amostra
   capturada.
4. **Nenhuma API de latência do dispositivo.** `AudioDevice` não informa latência de entrada/saída (nenhum uso de
   `jack_port_get_latency_range`; o PortAudio calcula `defaultLowInputLatency` só para abrir o stream). A
   compensação da etapa 3 depende disso.
5. **JACK:** a porta de entrada registrada não é checada contra `nullptr` (só a de saída); o vetor de entrada é
   redimensionado no callback de mudança de buffer. `// TODO` em `AudioJack.cpp:461` (trocar backend sem reiniciar).
6. **Taxa de entrada = taxa do dispositivo** (`AudioEngine::inputSampleRate()`), sem tratar dispositivos de entrada
   e saída diferentes (SDL/PortAudio podem abrir dispositivos distintos).
7. **Clipes de áudio em memória.** `SampleBuffer` é um `std::vector<SampleFrame>` com o arquivo inteiro decodificado
   (via `SampleCache`); não há leitura em streaming do disco. Takes longos em várias faixas custam muita RAM. O
   `SampleClip` salva o áudio como base64 dentro do projeto quando não há arquivo (`SampleClip.cpp:315`), e há
   `// TODO: start- and end-frame` no `saveSettings` (`SampleClip.cpp:327`) e dois `TODO` de tratamento de erro ao
   carregar (`:172`, `:176`).
8. **Posição em ticks, não em amostras.** `DefaultTicksPerBar = 192` (`include/TimePos.h:38`); clipes e o início da
   leitura (`SampleTrack::play`) são calculados com `framesPerTick` – não há posicionamento com precisão de amostra,
   o que atrapalha alinhar gravações e compensar latência.
9. **Reprodução de sample aloca na thread de áudio.** `SampleTrack::play` faz `new SamplePlayHandle` por clipe e
   `SamplePlayHandle` cria um `AudioBusHandle` com `new` (`SamplePlayHandle.cpp:44`). A `SampleTrack` só sabe tocar
   um clipe começando no seu início (`isPlaying` por clipe), base fraca para clipes sobrepostos/crossfade.
10. `Song.cpp:780/795`: `FIXME journal batch of tracks` (histórico por faixa, não em lote – afeta criar várias
    faixas de gravação de uma vez). `Sample.cpp:109`: TODO sobre pipelines de reprodução repetidos.
11. Plataforma: o fork nunca foi testado no Linux (ALSA/JACK/PulseAudio), registrado em `preparacao-mixer.md`.

---

### 5. Trabalho do LMMS (upstream) sobre gravação de áudio

Pesquisa feita pela API pública do GitHub (sem `gh` autenticado) em 12/11/2026. Itens não confirmados estão
marcados como **não verificado**.

#### 5.1 Linha do tempo dos PRs de gravação

| PR/issue | Autor, datas | Estado | O que fez / o que se discutiu |
|---|---|---|---|
| [#3943](https://github.com/LMMS/lmms/issues/3943) (issue) "Sample track standalone recorder" | Reflexe, 2017 | aberta | `SampleRecordHandle` já existia, mas a captura estava desativada; JACK foi "trivial" de habilitar. |
| [#3947](https://github.com/LMMS/lmms/pull/3947) "Enable sample track recording" | Reflexe, 2017–2018 | fechado sem merge | Captura em JACK, PulseAudio e SDL; grava em clipe novo nas faixas marcadas. Problemas: JACK não conectava sozinho à entrada, SDL perdia os últimos segundos, canal global de gravação instável; pediram ALSA. PhysSong: usar a taxa de processamento, `inputSampleRate` para o gravado; `SampleBuffer` reamostrava a cada mudança de taxa (degradação). |
| [#4531](https://github.com/LMMS/lmms/pull/4531) "Add sample track recording support" | PhysSong, 2018–2019 | fechado | Continuação no branch upstream `feature/recording`. Pendente: ALSA, CoreAudio, sndio, libsoundio; "decidir quando reamostrar". Reflexe: "ALSA é muito diferente de Pulse e das APIs modernas". |
| [#4994](https://github.com/LMMS/lmms/pull/4994) "Recording after rebase to master" | Reflexe, 2019–2021 | fechado, "rework required" | +3352/−2232 em 76 arquivos: captura nos backends, `SampleBuffer` refatorado, threads, UI. PhysSong: "instável e cheio de bugs"; atualizar a forma de onda ao vivo "exige muitas mudanças na estrutura interna e na segurança de threads". Depois Reflexe listou o que valia cherry-pick: armar por faixa, JACK, PulseAudio, PortAudio, forma de onda ao vivo, gravação em loop. |
| [#5990](https://github.com/LMMS/lmms/pull/5990) "Sample Track Recording Stage One" | Reflexe, 2021–2025 (129 comentários) | fechado em 15/03/2025, substituído pelo #7786 | Recorte mínimo do #4994, só SDL: clipe armado por "Set/clear record". Michael Gregorius (2024) acrescentou escolha de dispositivo SDL e `captureDeviceAvailable`. Ver §5.2. |
| [#7786](https://github.com/LMMS/lmms/pull/7786) "Sample Track Recording Stage One" | JohannesLorenz, 15/03/2025 | **aberto** (branch `LMMS:feature/recording-stage-one`, a base deste fork) | Pendências no próprio PR: 1) botão de gravar sem efeito (resolvido pelo #7899); 2) salvar os samples direito (talvez via #7627); 3) #7567 (mesclado); 4) `pushInputFrames` não é tempo real seguro (#7903). Sem atividade de mantenedores desde ~fim de 2025. |
| [#7567](https://github.com/LMMS/lmms/pull/7567) "Sample Track Recording with Jack backend" | mmeeaallyynn, 2024–2025 | **mesclado no master do LMMS** em 31/05/2025 | Portas de entrada JACK → `pushInputFrames`. Problemas apontados: `SampleFrame` é par estéreo, outro número de canais quebra; configuração de canais do JACK era morta e foi removida; entradas precisam ser ligadas à mão (qpwgraph/Helvum). É o branch `sample-track-recording-jack`. |
| [#7919](https://github.com/LMMS/lmms/pull/7919) "Input and output combo boxes for JACK driver" | michaelgregorius | mesclado no master em 16/07/2025 | Seleção de portas; o #7920 (só entrada) foi fechado/superado. |
| [#7899](https://github.com/LMMS/lmms/pull/7899) | steven-jaro | mesclado no `feature/recording-stage-one` em 30/04/2026 | Remove `Song::record()`/`SongEditor::record()`; Song Editor fica só com "gravar tocando". |
| [#7903](https://github.com/LMMS/lmms/pull/7903) "Real time safe recording with ring buffer stage one" | steven-jaro, 2025 | **aberto** (base `feature/recording-stage-one`); mesclado no fork em `b0e145268` | Antes: o callback do driver escrevia num buffer que dobrava de tamanho (alocação e lock no callback). Depois: `LocklessRingBuffer` de `DEFAULT_BUFFER_SIZE*100`, drenado por `processBufferedInputFrames()`; acrescenta `AudioDevice::isProcessing()`. Versões iniciais estalavam; testador relatou corrigido depois. |
| [#7627](https://github.com/LMMS/lmms/pull/7627) "adding sample exporter" | szeli1, 2024 | aberto upstream; mesclado no fork (`2d8c54fcd`) | Exportar o buffer do clipe; "permite terminar o #7786". |
| [#7444](https://github.com/LMMS/lmms/pull/7444) "Refactor PortAudio backend" | sakertooth | mesclado em 01/02/2026 | Na revisão, o stream de entrada foi considerado "inútil por enquanto". No fork, o PortAudio abre a entrada mas descarta (§3.1); se o upstream final mantém captura: **não verificado**. |
| [#6070](https://github.com/LMMS/lmms/pull/6070) | firewall1110 | fechado sem merge | Captura SDL ligada/desligada sob demanda (só durante a gravação) – exatamente o problema da captura sempre aberta. |
| [#8476](https://github.com/LMMS/lmms/pull/8476) | Bobjack18 (fork externo) | fechado sem merge em 05/09/2026 | Tentativa de mesclar o stage-one no master. |
| [#8631](https://github.com/LMMS/lmms/pull/8631) driver PipeWire nativo | michaelgregorius, 03/11/2026 | aberto | Página de entradas no diálogo e taxas diferentes da do sistema. Muito relevante para "Linux primeiro". |
| [#8588](https://github.com/LMMS/lmms/pull/8588) "Refactor SampleBuffer" | sakertooth, out/2026 | rascunho | Mexe no carregamento de samples e no Base64; pode conflitar com gravação. |

Pedidos de funcionalidade abertos: [#1297](https://github.com/LMMS/lmms/issues/1297) (gravação de áudio, 2014),
[#4603](https://github.com/LMMS/lmms/issues/4603) (multitrack com armar por faixa, estilo Ardour),
[#8027](https://github.com/LMMS/lmms/issues/8027) (várias entradas e monitoração pela cadeia de efeitos, estilo FL),
[#6022](https://github.com/LMMS/lmms/issues/6022) (salvar gravações como arquivos separados),
[#5108](https://github.com/LMMS/lmms/issues/5108) (mais de 2 canais). Duplicatas fechadas: #3394, #5346, #5520,
#7033, #7629, #4827.

#### 5.2 Discussões de desenho mais importantes (no #5990)

- **Armazenamento – o bloqueio principal.** tresf (17/07/2024,
  [comentário](https://github.com/LMMS/lmms/pull/5990#issuecomment-2233885139)): salvar o gravado como blob base64
  `data=` dentro do projeto "não é uma implementação apropriada". Alternativas: perguntar onde salvar, salvar numa
  pasta `samples`, ou projeto "portátil" (#5735, #3981, #7538 "Sample Folder"). Michael Gregorius descreveu o REAPER
  (pasta global para projetos não salvos, nomes com data) e o Bitwig (`projeto/recordings/` separado de `samples/`)
  e defendeu **gravar em streaming no disco** para não perder takes num crash; JohannesLorenz preferia memória e
  gravar ao salvar/autossalvar.
- **Armar por faixa, não por clipe** (consenso); cada faixa armada escolhe a sua entrada; cada passada cria clipe
  novo, não destrutivo; gravar a partir do cursor; um controle "INPUT" na faixa de sample, como o de MIDI (tresf).
  O botão de gravar desenhado no clipe foi revertido (`d8c271650`) por "poluir os clipes".
- **`IOManager`** proposto por Michael Gregorius (16/02/2025,
  [comentário](https://github.com/LMMS/lmms/pull/5990#issuecomment-2661425152)): os drivers empurram todos os buffers
  de entrada e registram/removem portas por ele; as faixas leem os buffers de entrada do período; ciclo por período =
  empurrar entradas → renderizar o grafo → copiar saídas. Ele também notou que o suporte a várias portas por faixa
  (`AudioPort`/`AUDIO_PORT_SUPPORT`, `AudioJack::registerPort`) existe mas não é usado.
- **Latência:** messmerd (23/06/2024) notou atraso perceptível ao tocar junto do metrônomo e gravação **só no canal
  esquerdo** (sem upmix de mono para estéreo). Ninguém resolveu. Não há trabalho upstream de compensação de latência
  de gravação nem de monitoração de entrada.
- **Tempo real:** JohannesLorenz apontou que `pushInputFrames` pegava mutex (`requestChangeInModel`) – motivo do #7903.
  As mudanças de threads foram uma das razões do colapso do #4994.

#### 5.3 Situação upstream em resumo (nov/2026)

A gravação de áudio **não está no master do LMMS**. O ramo vivo é o `feature/recording-stage-one` (#7786), parado;
o que entrou no master upstream foram só pedaços de driver (JACK #7567, seleção de portas #7919; SDL captura desde o
#5990/#4994 – data exata de chegada ao master **não verificada**). Bloqueios abertos: onde guardar o gravado
(#7627, #7538) e buffer de entrada tempo real seguro (#7903). PulseAudio só teve captura nos PRs abandonados
(#3947/#4994); ALSA, CoreAudio, sndio, OSS e libsoundio nunca tiveram.

#### 5.4 Comparação com o fork

- O fork já está **à frente** do upstream nesse tema: tem #7903, #7627 e #7900 mesclados (abertos upstream), PDC no
  mixer e entrada/armar/monitorar por faixa (o consenso do #5990), e já descartou a gravação em memória + base64
  (o bloqueio do tresf).
- Observação sobre o #7903 no fork: o `pushInputFrames` agora não trava, mas ainda imprime com `fprintf` na thread do
  driver; `processBufferedInputFrames` chama `requestChangeInModel()`, que vira no-op na thread de renderização
  (`s_renderingThread`), e usa `resize` num vetor com capacidade reservada.
- O desenho decidido na etapa 3 (gravação em disco por thread separada, pasta de mídia, armar por faixa, entrada por
  faixa) coincide com as posições de Michael Gregorius e tresf no #5990, e o `IOManager` dele é um bom ponto de
  partida para o caminho de entrada multicanal.

---

### 6. Conclusões para o plano

- O que **reaproveitar**: o caminho de captura do JACK (com reforma para N canais e portas sob demanda), a seleção
  de portas do JACK (#7919), a ideia do anel lock-free (#7903, mas com carimbo de tempo, multicanal e sem `fprintf`),
  o PDC do mixer, os fades (#7900) e o modelo `AudioInput`/armar/monitorar já na `SampleTrack`.
- O que **não reaproveitar**: `SampleRecordHandle`/`AudioSampleRecorder` (já removidos), o branch
  `sample-track-recording-jack` (obsoleto, já no master), a gravação em memória + base64.
- **Lacunas estruturais a resolver antes/junto da etapa 3:** captura multicanal planar por backend (ALSA e PulseAudio
  não capturam), abertura sob demanda, carimbo de tempo/latência do dispositivo, thread de disco, streaming de clipes
  do disco, posição com precisão de amostra e uma faixa de áudio própria com clipes sobrepostos.

---

## Anexo E: 05 — Como o REAPER grava multipista (com o Ardour como comparação)

Pesquisa para desenhar a gravação do LinuxMusic "como o REAPER". Data: 2026-11-12.

### Convenções e fontes

- **[Doc]** = afirmação documentada numa fonte primária (citada logo ao lado).
- **[Código]** = lida diretamente no código-fonte (Ardour) ou na referência de API (REAPER).
- **[Inferência]** = minha dedução a partir das fontes ou do comportamento conhecido do programa, sem documento que a afirme literalmente. Antes de copiar um comportamento marcado assim, confirme no programa.

Fontes primárias usadas (baixadas e lidas por inteiro nas partes relevantes):

| Sigla | Fonte | URL |
|---|---|---|
| UG | *Up and Running: A REAPER User Guide v7.85* (cap. 3 "Recording Audio and MIDI", cap. 6.13–6.15, cap. 8.12, cap. 22) | https://www.reaper.fm/userguide/ReaperUserGuide785c.pdf |
| WN7 | *REAPER 7.0: What's New* (resumo) | https://www.reaper.fm/userguide/WhatsNewReaper7Summary_r2.pdf |
| RP | *Multiple Disk Recording Paths in REAPER* | https://www.reaper.fm/guides/reaper_recpath_guide.pdf |
| CL | Changelog oficial completo do REAPER (até v7.85) | https://www.reaper.fm/whatsnew.txt |
| API | ReaScript API (parâmetros de track `I_REC*`, `C_LANE*` etc.) | https://www.reaper.fm/sdk/reascript/reascripthelp.html |
| BLOG | reaper.blog, "rec latency offset" (Jon Tidey, REAPER Mania) | https://reaper.blog/2018/11/rec_latency_offset/ |
| ARD | Manual do Ardour (Recorder, Monitoring, Latency, Audio/MIDI Setup, Preferences, Playlists) | https://manual.ardour.org/ (páginas citadas em cada seção) |
| ARDSRC | Código do Ardour (master): `route.cc`, `track.cc`, `disk_writer.cc`, `session.cc` | https://github.com/Ardour/ardour/tree/master/libs/ardour |

Observação: a wiki da Cockos (wiki.cockos.com) agora redireciona para cockos.com e não estava acessível, e o archive.org bloqueou o acesso automatizado. Por isso, as páginas antigas da wiki (por exemplo "Preferences Audio Recording") aparecem aqui só pelo resumo do mecanismo de busca, e o que vem delas está marcado.

---

### 1. Controles de gravação por track

#### 1.1 Record arm

- **[Doc]** Armar é por track. Clicar no botão de arm de qualquer track selecionada arma toda a seleção. Com o tema padrão do REAPER 7, armar também faz aparecer o botão de seleção de entrada. (UG §3.5 passo 2, §3.6 passo 2)
- **[Doc]** O WN7 confirma que "Record input" e "Input source" só ficam visíveis no TCP quando o arm está ligado. (WN7, p. 4)
- **[Doc]** "On the fly recording": dá para armar tracks **depois** de a gravação ter começado, e elas gravam a partir do momento em que são armadas. Desarmar uma track durante a gravação encerra a gravação só dela e as demais continuam. (UG §3.5 Nota 2)
- **[Doc]** Há **arm automático quando a track é selecionada** (por track, ou global com "Set all tracks to automatic record arm when selected"). (UG §3.7)
- **[Doc]** Há uma preferência para **impedir o início da gravação se nenhuma track estiver armada**. (UG §3.37)
- **[Doc]** Há uma preferência para **armar automaticamente tracks novas** e uma "Record Config" padrão para tracks novas (monitoramento, entrada etc.). (UG §3.38)
- **[Doc/API]** Estado exposto na API: `I_RECARM` (0/1) e `I_AUTORECARM`. (API)
- **Comparação com o Ardour [Doc]:** o Ardour tem rec-arm por track, um **Rec-safe** (clique direito, protege a track contra gravação acidental) e um "Global Arm". (https://manual.ardour.org/recording/the-recorder/) Há também uma preferência "Disable per-track record disarm while rolling", que impede desarmar uma track no meio de um take. (manual do Ardour, Preferences > Transport, fonte em https://raw.githubusercontent.com/Ardour/manual/master/include/preferences.html)

#### 1.2 Seleção de entrada (mono, estéreo, multicanal, MIDI)

- **[Doc]** A track não tem tipo "mono" ou "estéreo". O que é mono ou estéreo é a **entrada** escolhida: "You do not need to specify whether a track is to be stereo or mono." (UG §3.4 Nota)
- **[Doc]** O menu de entrada traz `Input: Mono`, `Input: Stereo`, `Input: MIDI` e **`Input: None`** (útil para impedir gravação acidental em buses e pastas). (UG §3.5 Dica)
- **[Doc]** Há uma opção para esconder pares estéreo "não padrão" (por exemplo 2/3), para simplificar o menu. (UG §3.9)
- **[Doc]** Gravação **multicanal**: se a track tiver N canais (definidos no routing), o menu ganha uma entrada multicanal. Ela grava N entradas simultâneas, cada uma num canal da mesma track, até 128 canais, por exemplo para ambisonics. O meter tem o modo "Multichannel peaks". (UG §3.10)
- **[Doc]** O dispositivo pode expor até 512 entradas e 512 saídas ("Input Range", "Output Range"), e é preciso ligar "Enable inputs". (UG §1.15.1)
- **[Doc]** "Assign inputs sequentially": seleciona N tracks e atribui as entradas 1..N em sequência, para áudio ou para canais MIDI. Também dá para atribuir entradas pela **Routing Matrix** (Alt+R). (UG §3.6, §3.31 Nota)
- **[Doc]** MIDI: escolhe-se dispositivo e canal de origem (ou todos), e opcionalmente "Map output to" um canal. São até 62 portas MIDI de entrada e 64 de saída. Há também o teclado virtual (VKB). (UG §3.31)
- **[API]** A codificação de `I_RECINPUT` mostra o modelo interno: `<0` = sem entrada. Com o bit 4096 ligado, a entrada é MIDI (5 bits de canal, 0 = todos; 6 bits de dispositivo, 63 = todos, 62 = VKB). Sem ele, os 10 bits baixos são o canal inicial (ReaRoute/Loopback a partir de 512), o bit 1024 indica estéreo, o bit 2048 indica multicanal (com o número de canais da track) e o resto é mono. (API)
  - **[Inferência] Lição de design:** a entrada é **um canal inicial mais uma largura** (1, 2 ou N), não uma lista arbitrária de portas. É simples e cobre quase tudo. No JACK/PipeWire o equivalente seria "porta inicial de captura mais largura", ou portas de entrada próprias da track.

#### 1.3 Modos de monitoramento de gravação

- **[API]** `I_RECMON`: `0 = off`, `1 = normal`, `2 = not when playing (tape style)`. À parte, `I_RECMONITEMS`: "monitor items while recording" (0/1). (API)
- **[Doc]** **Monitor Input (normal):** escuta a entrada sempre que a track está armada. Desde a v6.0 vem ligado por padrão em tracks novas. (UG §3.5 passo 5; CL v6.0 "enable track record monitoring on new tracks by default")
- **[Doc]** **Monitor Input (Tape Auto Style):** "monitors input only when the transport has stopped or when you are recording. During playback you will hear only the timeline audio." (UG §3.14 Nota 1)
- **[Doc]** **Monitor track media when recording:** decide se o material que já está na track toca durante a gravação. Em punch, com Monitor Input normal, ouve-se o material antigo e o novo até o punch e durante ele. Com tape auto, ouve-se só o material antigo até o punch e os dois durante o punch. (UG §3.16 Nota) Os modos de record output também obedecem a essa opção. (CL v2.99)
- **[Doc]** O guia recomenda desligar o monitoramento do REAPER quando a interface tem *direct monitoring*. (UG §3.5, §3.14 Dica)
- **[Doc]** O REAPER **não** tem um "auto-input" global como o do Ardour. A política de monitoramento é por track (os três estados acima). Para MIDI em overdub/replace, "auto input monitoring controls whether notes are monitored outside of punch regions". (CL v4.11)
- **Comparação com o Ardour [Código]:** `Route::monitoring_state()` implementa a tabela-verdade de https://manual.ardour.org/appendix/monitor-modes/ . Escolha explícita por track (botões **In** / **Disk**, que podem ser combinados) > modo de sessão (Disk / Input / Cue) > modo automático. No automático, com a track armada:
  - transporte parado → entrada;
  - rodando sem gravar e com `auto-input` → disco;
  - rodando sem gravar e sem `auto-input` → entrada;
  - gravando → entrada (durante o pre-roll: disco + entrada).

  Com a track não armada, sempre disco, exceto no modo "auto-input does talkback" (com o transporte parado, monitora a entrada). O modo "tape machine" significa que tracks não armadas nunca monitoram a entrada. (ARDSRC `route.cc`, `Route::monitoring_state`; preferências em https://raw.githubusercontent.com/Ardour/manual/master/include/preferences.html, seção Monitoring)
  - **[Inferência]** O "tape auto style" do REAPER equivale ao `auto-input` do Ardour, mas **por track** em vez de por sessão.
- **[Doc, Ardour]** O Ardour distingue monitoramento externo, por hardware (o Ardour pode controlar o mixer da placa, se o driver permitir) e por software. ("Record monitoring handled by", https://manual.ardour.org/recording/monitoring/monitor-signal-flow/)

#### 1.4 Medição de entrada (input metering)

- **[Doc]** "Show track input when record armed": quando a track está armada, o meter mostra o **sinal de entrada**, não a saída. Há escalas em dB no meter armado, indicadores de clip persistentes, "Reset peak indicators on play/seek" e uma opção global para meters pré-fader. (UG §3.38)
- **[Doc]** O volume do fader **não** altera o nível gravado: "the recording level cannot be controlled by adjusting the Track's volume fader: that controls only the playback level." (UG §3.5 passo 6)
- **[Doc]** Opção de mostrar o TCP inteiro (com o VU) ao armar. (UG §3.37)
- **[Doc]** Mostrar os picos (waveform) **durante** a gravação, com taxa de atualização configurável, serve de confirmação visual de que algo está sendo gravado. (UG §3.37)
- **[API]** O `I_VUMODE` inclui peaks estéreo, peaks multicanal, RMS e LUFS-M/S. (API)
- **Ardour [Doc]:** o Recorder tem um painel global de entradas com meter por porta física, "Peak Hold", a waveform dos últimos 5 s e atividade MIDI (os 4 últimos eventos). (https://manual.ardour.org/recording/the-recorder/) **[Código]** Ao armar, o Ardour muda o ponto de medição da track para a entrada (e restaura depois), exceto quando o transporte roda sem gravar com auto-input ligado. (ARDSRC `track.cc`, `Track::update_input_meter`)

#### 1.5 Input FX e Monitor FX

- **[Doc]** **Input FX chain** (por track, no menu do arm): é processado **antes** da gravação e fica gravado no arquivo, de forma destrutiva. Só consome CPU quando a track está armada. (UG §3.26) A latência do input FX é compensada na gravação. (CL v5.75 "latency-compensate input-FX delay")
- **[Doc]** **Monitoring FX** (Shift+clique no FX do master, ou View > Monitoring FX): ficam nas saídas de hardware. **Não entram no render, não afetam o metrônomo e não são salvos no projeto.** Servem para correção de sala e analisadores. (UG §6.13)
- **[Doc]** "Record disable (input monitoring only)": a track fica armada e monitorando (com input FX), mas não grava. Serve para ensaio ou ajuste de timbre. (UG §3.27)
- **[Doc]** Preferências: "Autobypass plug-ins whose PDC exceeds a threshold when record-armed" (com a opção "only while actually recording") e "Temporarily bypass oversampling on record arm affected tracks". (UG §22.6)
- **Ardour [Doc]:** o equivalente às Monitoring FX é a **Monitor section**, e os mixes de fone ficam no **Foldback**. (https://manual.ardour.org/recording/monitoring/)

#### 1.6 Modos de gravação por track (o que é gravado)

Lista canônica (**[API]** `I_RECMODE`): `0 = input`, `1 = stereo out`, `2 = none`, `3 = stereo out w/ latency compensation`, `4 = midi output`, `5 = mono out`, `6 = mono out w/ latency compensation`, `7 = midi overdub`, `8 = midi replace`. A API lista só esses, mas o menu tem mais modos: touch-replace e latch-replace no MIDI, saída multicanal e entradas forçadas. (API; UG §3.32; CL v7.18 "force multichannel input, force MIDI input, and MIDI latch-replace as default track record modes")

- **[Doc]** **Record: input (audio or MIDI):** grava a entrada como chega (depois do input FX). (UG §3.14, §3.31)
- **[Doc]** **Record: input (force format):** força mono, estéreo, multicanal ou MIDI. (CL v7.18; CL v4.0alpha4 "Record input: force mono")
- **[Doc]** **Record: output:** mono, estéreo ou multicanal, "with or without latency compensation". O ponto de captura é **post-fader (padrão), post-FX/pre-fader ou pre-FX** (`I_RECMODE_FLAGS & 3`). Serve para gravar um synth externo, buses de FX, receives e hardware via ReaInsert. (UG §3.29, §3.30, §3.36; CL v6.60)
- **[Doc]** **MIDI:** *overdub* (soma notas aos itens existentes), *replace* (apaga tudo do início ao fim da passada), *touch-replace* (substitui só onde há nota nova, no mesmo canal) e *latch-replace* (substitui a partir da primeira nota tocada). Há também **output (MIDI)**, que grava o MIDI na saída da cadeia de FX. Em loop, esses modos **editam o take existente** em vez de criar takes novos. (UG §3.32, §3.35)
- **[Doc]** **Record: disable (input monitoring only):** ver 1.5. (UG §3.27)
- **[Doc]** **Formato por track:** cada track pode ter seu próprio formato (WAV, AIFF, FLAC, MP3, OGG, WavPack etc.), sobrepondo o do projeto, em "Track recording settings". (UG §3.39)
- **[Doc]** **Caminho por track:** primário, secundário ou "primary + secondary as invisible backup". (RP)
- **Ardour [Inferência]:** o Ardour não tem "record output" por track. Para isso, cria-se outra track cuja entrada é a saída/bus de origem. O Ardour 8 também tem *I/O plugins* (https://manual.ardour.org/recording/io-plugins/).

---

### 2. Comportamento global de gravação

#### 2.1 Semântica do botão Record

- **[Doc]** Record (Ctrl+R) **inicia o transporte e grava** nas tracks armadas. Para parar: Stop, Espaço, Record de novo ou Ctrl+R. (UG §3.5 passos 7–8)
- **[Doc]** Há uma diferença importante na forma de parar:
  - parar com **Record/Ctrl+R** salva os arquivos automaticamente;
  - parar com **Stop/Espaço** abre um diálogo para **salvar, apagar ou renomear** os arquivos novos (configurável; há prompts separados para "on stop" e "on punch out").

  (UG §3.5 Nota 1, §3.37)
- **[Doc]** Há ações para iniciar e parar a gravação no próximo compasso, no próximo beat, num marcador ou no cursor de edição. (CL v4.0alpha37)
- **[Doc]** Há ações para "start new files during recording", "add recorded media" e "remove recorded media" (dividir a gravação em arquivos sem parar). (CL v2.202)
- **[Inferência]** **Gravar com o transporte rodando:** apertar Record durante o playback entra em gravação a partir da posição atual, sem reiniciar (punch manual). O guia não descreve isso explicitamente, mas é coerente com o "on the fly recording" e com os prompts de "punch out" (UG §3.5, §3.37). Para o LinuxMusic, recomendo suportar as duas transições, parado→gravando e tocando→gravando, sem glitch.
- **[Doc]** Há uma opção que mantém o **dispositivo de áudio aberto quando há tracks armadas**, mesmo com o app inativo ("Close audio device when inactive and tracks are record armed" é configurável). (UG §22.6)
- **[Doc]** Gravação **retroativa de MIDI**: o REAPER guarda o MIDI tocado mesmo sem gravar, e ações o inserem depois. (UG §13.52; CL v7.08/v7.48)
- **Ardour [Doc]:** o record global é um "arm" da sessão, e o take começa quando o transporte roda. Há "Keep record-enable engaged on stop" e "Stop transport using the current grid". (preferences.html, Transport)

#### 2.2 Modos de gravação globais (Options > Record mode)

- **[Doc]** **Normal:** grava do ponto de play até o stop. (UG §3.5 passo 1)
- **[Doc]** **Time selection auto-punch:** só grava dentro da seleção de tempo. Fora dela, ouve-se o material existente. É **não destrutivo** por padrão: cria um take novo. (UG §3.16)
- **[Doc]** **Auto-punch selected items:** grava só dentro dos itens selecionados, que podem ser "empty items" criados com Insert > Empty item. Os itens vazios são substituídos pelo material gravado. (UG §3.25) Quando a gravação não cruza nenhum item, cai para gravação normal naquela track. (CL v5.17)
- **[Doc]** Gravar durante o **pre-roll** é opcional ("record audio during pre-roll"): o item fica com o começo escondido e basta arrastar a borda esquerda para revelar. (UG §3.8; CL v5.33)
- **[Inferência] Implementação:** o REAPER grava a passada inteira em disco e aplica o punch como *trim* do item, e por isso consegue revelar o pre-roll. Isso é confirmado indiretamente pela opção do REAPER 7 "add the entire new recording, but comp only within the time selection". (CL v7.0) É um padrão valioso: **gravar sempre a passada inteira e recortar o punch de forma não destrutiva**.

#### 2.3 Gravação sobre material existente (Options > "New recording that overlaps existing media")

- **[Doc]** **Split existing items and add takes** (padrão): o item existente é dividido e a gravação vira um **take** novo dentro do item. (UG §3.16, §3.19, §3.23)
- **[Doc]** **Trim existing items (tape mode):** destrutivo para o *item*, porque corta o item antigo. O arquivo antigo continua no disco. (UG §3.17)
- **[Doc]** **Add lanes:** a gravação vai para uma **lane fixa** nova e só a nova toca. **Add lanes (layer new lanes):** a lane nova toca junto com as outras. **Do not add lanes:** grava numa lane existente. (UG §3.19, §3.23; CL v7.0, v7.17)
- **[Doc]** **Add media items in layers:** replica a opção da v6, em que os itens sobrepostos são mixados. (CL v7.03)
- **[Doc]** Há crossfade automático opcional quando a gravação nova se sobrepõe. (CL v7.0) Os takes não dividem um item se começarem ou terminarem dentro do fade do item existente. (CL v7.70)
- **[Doc]** Takes de várias tracks gravadas juntas podem receber **a mesma cor por passada** (opção de aparência) para identificar o conjunto. (UG §3.19 Nota 1)
- **[Doc]** Com *free item positioning* (FIPM), cada gravação sobreposta vira um **item novo** e os itens podem ser empilhados livremente. (UG §3.24)

#### 2.4 Loop recording e takes

- **[Doc]** Há três ingredientes: modo *time selection auto-punch*, **repeat** ligado e "loop points linked to time selection" **desligado**. Cada volta grava um take novo. Com MIDI overdub/replace, cada volta edita o mesmo take. (UG §3.35)
- **[Doc]** Preferências (Audio > Loop/Lane Recording, UG §22.6.7):
  - "Discard incomplete first or last takes if at least one full loop was recorded", com limiar padrão de **90%** da duração do loop;
  - quando adicionar a mídia ao projeto: **On stop** (recomendado, com a opção "Creating new files on loop") ou **At each loop**;
  - "MIDI overdub/replace always creates selection-length media item";
  - para tracks com lanes fixas: "always record into an existing lane if space" e "add whole recording whether or not comping".
- **[Doc]** Durante a gravação aparece um **contador de loops**. (CL v7.0)
- **[Doc]** No loop recording, a mídia não é dividida mesmo que uma volta completa não tenha sido gravada. (CL v7.0)
- **[Doc]** "Ignore loop points if recording is entirely outside the loop". (CL v7.20)
- **[Inferência]** Com "On stop" e sem "new files on loop", cada take é uma **janela (offset e duração) num único arquivo contínuo** gravado durante todas as voltas. Daí a correção "take offset off-by-1 sample when using looped recording". (CL v7.67) Isso é barato e seguro, porque nunca se interrompe o stream de disco. Recomendo o mesmo para o LinuxMusic.

#### 2.5 Takes "clássicos" e take lanes

- **[Doc]** Um *media item* pode ter N **takes**: só um toca (o ativo), "Take n/N" aparece no nome, T e Shift+T alternam entre eles e Ctrl+L mostra todos ("Show all takes (when room)"). Há também "Display empty takes", que mantém as lanes alinhadas por passada, e "Explode all takes to new tracks". (UG §3.20–3.22)
- **[Doc]** O comping clássico é feito dividindo o item (S) e escolhendo o take ativo em cada pedaço. Há **comp sets** com nome ("Save as new comp", "Move active comp to top") e **take markers** ranqueados como auxílio. (UG §8.6, §8.11)
- **[Doc]** "Take lanes remain aligned by recording pass, creating empty lanes as needed." (CL v4.0)

#### 2.6 Fixed item lanes e comping (REAPER 7)

- **[Doc]** As lanes fixas são "tracks dentro da track": até 256 lanes segundo o UG v7.85. O WN7 e o changelog falam em 128 ("if recording exceeds maximum number of lanes (currently 128), place recording in last lane", CL v7.06). Por padrão só uma lane toca, mas há "Play only lane", "Toggle playing lane" e "Play all lanes", e Ctrl+clique seleciona várias. (UG §8.12; WN7 p. 9–10)
- **[Doc]** Recursos de gravação por lane: "Record into lane" (marca a lane-alvo com um ponto vermelho), "Record into first available lane" e "Layer new lanes". Se "record into first available lane" estiver desligado, **cada gravação cria uma lane nova**. Tracks agrupadas gravam **na mesma lane**. (UG §8.12.2; CL v7.0)
- **[Doc]** **Comping:** uma *comp lane* (C1, C2…) é criada com "Comp into new empty lane". Arrastar sobre uma lane de origem cria **comp areas**, e os trechos escolhidos são **copiados como itens novos** na comp lane, sem mexer nas lanes de origem. Há várias comp lanes (alterna-se a ativa com Shift+Ctrl+clique), "auto-comp" da gravação nova enquanto se faz comping e a opção de que lanes de número maior mascarem as de número menor. (UG §8.12.3–8.12.4; CL v7.0 "automatically create comp areas for new recording while comping")
- **[API]** Configuração por track (`C_LANESETTINGS`): auto-remover lanes vazias, não auto-compor gravação nova, lanes novas tocam exclusivas ou em camadas, lanes grandes/pequenas, nova gravação no fundo ou na primeira lane livre, esconder botões. Por item, `C_LANEPLAYS` (0 = não toca, 1 = exclusiva, 2 = toca junto). (API)
- **[Doc]** Há conversão de takes em lanes. (WN7 p. 13)
- **Ardour [Doc]:** o modelo é de **playlists**. Cada take pode ir para uma playlist nova (nome padrão "Take.#n", criada para todas as tracks armadas de uma vez, o que mantém os takes correlacionados entre tracks). A alternativa é gravar em **camadas** sobre a mesma playlist, com modos "later is higher" ou "most recently added is higher", e fazer o comp com as ferramentas de layer. (https://manual.ardour.org/recording/the-recorder/ ; https://manual.ardour.org/working-with-playlists/playlist_usecases/ ; doc de layering em https://git.ardour.org/ardour/ardour/src/branch/master/doc/layering/layering.tex) O Recorder do Ardour tem **"Discard Last Take"**. (Recorder)

#### 2.7 Pre-roll, count-in e metrônomo

- **[Doc]** O metrônomo tem:
  - saída roteável para qualquer saída de hardware (por exemplo, só nos fones);
  - "Run during playback" e "Run during recording" separados;
  - **count-in antes do playback e/ou da gravação**, com N compassos e "start count-in at start of measure";
  - **pre-roll** antes da gravação e/ou do playback, em compassos (padrão 2), com "start pre-roll at start of measure";
  - padrão de clique 0.5x/1x/2x/4x e grade de acentos por tempo;
  - até 4 samples (A–D) ou síntese (seno, triângulo, quadrada, seno+quadrada);
  - volume e ganho relativo dos clicks B–D, e presets.

  (UG §3.8)
- **[Doc]** Gravar durante o pre-roll é opcional (ver 2.2). Tracks armadas durante o pre-roll ficam com o offset correto. (CL v5.96)
- **[Doc]** Notas MIDI tocadas durante o count-in são tratadas. (CL v6.53)
- **Ardour [Doc]:** pre-roll em compassos ou segundos (preferences.html, Transport). O Ardour monitora disco + entrada durante o pre-roll de gravação. (ARDSRC `route.cc`)

---

### 3. Arquivos

#### 3.1 Caminho de gravação

- **[Doc]** Em Project Settings > Audio Settings há caminho **primário** e **secundário**. O primário vazio significa a pasta do .RPP. O caminho pode ser absoluto ou relativo ao projeto. Por track: primário, secundário ou "primary + secondary as invisible backup" (grava nos dois e só o primário entra no projeto). Usos citados: dobrar a banda de disco, fail-safe contra falha de disco, show ao vivo com vibração. (RP)
- **[API]** `RECORD_PATH` (pode ser relativo), `RECORD_PATH_SECONDARY` e `RECORD_FORMAT` (configuração do *sink*). (API)
- **[Doc]** O REAPER pode **criar pastas automaticamente**, inclusive com wildcards. (CL v6.13)
- **[Doc]** Opções para verificar o espaço livre antes de gravar e mostrar espaço livre e caminho na barra de menu. (UG §3.37) No Ardour, o Recorder mostra uma **estimativa de tempo restante** com base em bit depth, taxa e tracks armadas. (Recorder)

#### 3.2 Formatos

- **[Doc]** Formato do projeto (Project Settings > Media): WAV (recomendado 24 bits para gravar), AIFF, FLAC, MP3, OGG Vorbis, WavPack e outros. O formato também pode ser definido por track. Pode-se gravar marcadores e regiões como *cues* dentro do arquivo (todos, nenhum ou só os que começam com `#`). (UG §3.2, §3.39)
- **[Doc]** FLAC e WavPack aceitam bit depth arbitrário entre 16 e 24 bits. (CL v5.12)
- **[Doc]** O render e a importação tratam .WAV/.W64/.BWF (UG p. 5041 do texto extraído). **[Inferência]** O sink WAV do REAPER tem opções para arquivos grandes (W64/RF64) e chunks BWF; a lista exata não aparece no UG.
- **Ardour [Doc]:** o formato é definido na sessão. **Sample format** padrão em **32-bit float** (24 e 16 bits disponíveis). **File type** WAVE (padrão), Broadcast WAVE, CAF ou RF64, este último para passar de 4 GB. (manual do Ardour, Session Properties, https://raw.githubusercontent.com/Ardour/manual/master/include/session-properties.html)

#### 3.3 Nomes

- **[Doc]** Nomeie a track antes de gravar: o arquivo herda o nome dela. Há wildcards configuráveis em Preferences > Recording (número e nome da track, nome do projeto, data e hora…). O padrão inclui número da track, ano, mês, dia, hora e minuto. (UG §3.4, §3.37)
- **[Doc]** Wildcards adicionais:
  - `$recpass`: contador global de passadas, que dá para zerar ou ajustar por ação (CL v5.974, v5.979);
  - `$recnumber[N]`: 1, 2… para nomes repetidos (CL v7.09);
  - `$rectag`: tag do projeto, usada no padrão desde a v7.36 (CL v7.36; API `RECTAG`);
  - timestamp no início do nome (CL v3.76).
- **[Doc]** Nomes longos são encurtados automaticamente para caber nos limites do sistema de arquivos. (CL v7.28)
- **Ardour [Código]:** `Session::format_audio_source_name`: `base[-N]` + `%L`/`%R` (estéreo) ou `%a`, `%b`… (multicanal) + extensão. O Ardour grava **um arquivo mono por canal**. (ARDSRC `session.cc`)

#### 3.4 Arquivo por track e por passada; relação take ↔ arquivo

- **[Doc]** Cada track armada gera **um** item por passada. Entrada estéreo vira "a single stereo item on a single track" e entrada multicanal grava "each to a separate channel on the one track". (UG §3.9, §3.10)
- **[Inferência]** O REAPER grava **um arquivo intercalado por track por passada**, com 1, 2 ou N canais, ao contrário do Ardour, que grava um mono por canal. O UG não diz "intercalado" literalmente, mas o item é um único *source* com N canais.
- **[Doc]** Opção de **começar um arquivo novo a cada X MB**, para segurança em gravações longas. A troca é **gapless**: sem fades padrão e com correção de arredondamento. (UG §3.37; CL v5.11, v7.17)
- **[Doc]** No loop recording, há a opção de "new files on loop". Sem ela, ver a inferência em 2.4.
- **[Doc]** Um take do REAPER é um *source* (arquivo) mais um offset de início e um play rate dentro de um item. **[Doc]** "preserve PDC monitoring mode adjusts media start offset rather than item position". (CL v4.33)
- **[Doc]** Os peaks (.reapeaks) podem ser gerados durante a gravação ou depois, conforme preferência. (UG §3.37)

#### 3.5 Escrita em disco

- **[Doc]** Há "Advanced Disk I/O Options" em Audio > Buffering (modo de escrita, tamanhos de buffer). As mudanças só valem depois de reabrir o projeto. (UG §22.6.2)
- **[Doc]** Histórico: "Recording: optimized disk writing buffers"; "changed OSX default write mode to unbuffered, reduced buffer size". (CL v4.73)
- **[Doc]** "Improve realtime performance when initializing recording". (CL v7.23)
- **[Doc]** Preferência: **"Stop processing audio in the event of disk failure/disk full"**. (UG §22.6)
- **[Inferência]** A arquitetura implícita é o padrão de toda DAW séria: a thread de áudio copia blocos para um ring buffer sem bloquear, e uma thread de disco esvazia o ring em blocos grandes. O UG não descreve a thread.
- **Ardour [Doc/Código]:** "Disk I/O Buffering", com **segundos** de buffer separados para playback e gravação e presets por tamanho de sessão. (preferences.html, Performance) A thread **butler** esvazia os buffers de captura em *chunks* (`DiskWriter::_chunk_samples`). O ring buffer de captura tem no mínimo `2 × chunk` ou `audio_capture_buffer_size` da butler. (ARDSRC `disk_writer.cc`) A mensagem "disk system not able to keep up" indica overflow desse ring (https://discourse.ardour.org/t/the-disk-system-on-your-computer-was-not-able-to-keep-up-with-ardour/106664).

#### 3.6 O que acontece ao parar

- **[Doc]** O diálogo **Save all / Delete all / Rename / escolha individual** aparece ao parar com Stop/Espaço ou no punch out (configurável). Parar com Record salva tudo direto. (UG §3.5, §3.37)
- **[Doc]** Gravação desfazível com Ctrl+Z. Itens indesejados podem ser apagados. (UG §3.16 Dica)
- **[Doc]** Ações "mark last recording pass as favorite / for deletion" e take markers adicionados **durante** a gravação. (CL v7.06, v6.09)
- **[Doc]** O autosave não salva durante playback ou gravação. (CL v4.401)
- **[Doc]** Ao desarmar uma track com MIDI, são enviados note-offs. (UG §22.6.4; CL v5.1)
- **Ardour [Doc]:** "Discard Last Take" apaga os arquivos da última passada. Não há histórico de descartes. (Recorder)

---

### 4. Latência

#### 4.1 Offset de gravação e compensação da latência do driver

- **[Doc]** Preferences > Recording: **"use audio driver reported latency"** ajusta automaticamente a posição da mídia gravada pela latência informada pelo driver. Há também **offsets manuais separados de entrada e de saída**, em ms e em samples. (UG §3.37; CL "separate offsets for input/output latency in prefs/recording, now specified in both ms and samples")
- **[Doc, wiki via busca]** "If auto adjust is enabled, this offset is added to the automatic offset." O offset manual soma-se ao automático e pode ser negativo. (https://www.cockos.com/wiki/index.php/Preferences_Audio_Recording, não acessível no momento)
- **[Doc]** O procedimento de calibração é um **teste de loopback** com cabo da saída para a entrada, medindo com o "ping detect" do ReaInsert. O valor medido entra no offset de entrada até o ping dar 0 samples. O offset muda se a taxa ou o block size mudarem. "this only applies to signals coming through the inputs": o roteamento entre tracks não precisa disso. (BLOG)
- **[API]** `GetInputOutputLatency()` dá as latências de entrada e saída do dispositivo **em samples**, e `GetOutputLatency()` em segundos. (API)
- **[Doc]** Loopback/ReaRoute **não** recebe compensação da latência do dispositivo. (CL v7.0)
- **[Doc]** O record output tem as variantes "latency compensated" e não compensada. (API `I_RECMODE` 3/6; UG §3.29)
- **[Inferência] Fórmula:** a posição gravada = posição do transporte no início do bloco − (latência de entrada + latência de saída reportadas) − offsets manuais. O ouvido do músico está **atrasado em `output latency`** em relação ao transporte e o sinal dele chega **atrasado em `input latency`**. Por isso o REAPER compensa a ida e volta ("round trip"). O blog confirma: sem essa opção "direct monitoring would never line up with the project".
- **Ardour [Código]:** cada processor recebe `set_capture_offset(in_latency)` (latência de captura a montante) e `set_playback_offset(signal_latency + output_latency)`. O `DiskWriter` define `first_recordable_sample = capture_start + capture_offset + playback_offset`. Ou seja, a mesma compensação de ida e volta, calculada **por porta e por rota** a partir da API de latência do JACK e do backend. (ARDSRC `route.cc` ~l.4850–4890, `disk_writer.cc` ~l.170–215)
- **Ardour [Doc]:** a latência sistêmica (conversores, USB) não é detectável e precisa ser informada ("Hardware Input/Output Latency" em samples no Audio/MIDI Setup, ou `-I`/`-O` no jackd). Há um botão "Calibrate Audio" (loopback guiado) e ferramentas `jack_delay`/`jack_iodelay`. Interfaces USB no Linux podem variar: "recalibrate at the start of each session and each time an xrun occurs". (https://manual.ardour.org/sessions-tracks/sessions/audiomidi-setup/ ; https://manual.ardour.org/synchronization/latency-and-latency-compensation/)

#### 4.2 PDC e monitoramento

- **[Doc]** Com PDC, "The track media is read in advance, so as to be heard at the correct time, but **input monitoring is delayed**." O sinal monitorado sai atrasado pela PDC da cadeia. (UG §6.15)
- **[Doc]** O modo PDC da cadeia pode ser por cadeia, por FX ou "ignore plug-in delay". No master há **"Hardware output compensation"**: o PDC do master é ignorado e compensado **na gravação como se fosse latência da interface**, e as outras saídas de hardware são atrasadas na mesma medida. (UG §6.15)
- **[Doc]** **"Preserve PDC delayed monitoring in recorded items"** (por track) serve para quem toca "adiantado" para compensar o que ouve: o item gravado toca depois **como foi ouvido**. Isso é feito ajustando o *start offset* do take, não a posição do item, e considera também a latência de gravação. A PDC fica marcada no item com dois triângulos. (UG §3.28; CL v3.alpha.1, v4.33, v4.5)
- **[Doc]** Mitigação ao armar: auto-bypass de plug-ins com PDC acima de um limiar (opcionalmente só durante a gravação) e bypass temporário de oversampling. (UG §22.6)
- **[Doc]** A PDC fica desativada quando há feedback routing. (UG §6.15)
- **[Inferência] Lição:** o caminho de monitoramento da track armada deve ser o **mais curto possível**, ignorando a PDC das outras tracks. É isso que o auto-bypass e o "hardware output compensation" fazem. O Ardour faz o equivalente: o input monitorado passa pelos plugins da própria track, e as outras tracks são alinhadas por *read-ahead*. (https://manual.ardour.org/synchronization/latency-and-latency-compensation/)

---

### 5. Desempenho e robustez

#### 5.1 Anticipative FX e buffering

- **[Doc]** **Anticipative FX processing** processa FX "à frente" do tempo real em várias threads. "On multiprocessor/multicore systems it also allows for very significant multiprocessor usage." Ajuda com plug-ins que trabalham em blocos grandes (ReaVerb, ReaFir) e permite latências de interface menores. Há opções para permitir em tracks sem FX, com o editor MIDI aberto ou com automação em touch/latch/write. (UG §22.6.2, §6.14)
- **[Inferência importante]** Tracks **armadas e monitorando entrada** não podem ser antecipadas, porque a entrada ainda não existe. Elas são processadas no tempo do bloco do dispositivo. Por isso o REAPER aguenta muitas tracks em mix com pouca CPU, mas uma track armada com FX pesado continua exigindo tempo real. É consistente com o "Allow live FX multiprocessing" separado (UG §6.14), mas o UG não descreve isso dessa forma.
- **[Doc]** **Media buffering** é a leitura antecipada de disco, que pode ser desativada por track ("Track performance options": media buffering, anticipative FX, metering, spectral peaks) ou para tracks selecionadas. (UG §6.14, §22.6.2)
- **[Doc]** Block size: comece com 44.1 kHz/512 e reduza para 256 ou 128 se houver eco no monitoramento. Blocos menores carregam mais a CPU. "low levels of latency are only really needed for recording". (UG §1.15.1, §3.14 Nota, §22.4)
- **[Doc]** "Audio thread priority" é configurável. (UG §1.15.1) No Linux, há override manual das configurações de realtime do JACK para usuários de pw-jack. (CL v7.73)

#### 5.2 Xruns

- **[Doc]** O REAPER separa dois tipos de xrun:
  - **audio xrun**, do dispositivo, mostrado com um "yellow-flash", opcional ("flash transport status yellow on possible audio device underrun", CL);
  - **media xrun**, de buffering de disco ou FX, mostrado com um "red-flash" (o fundo do time display pisca em vermelho).

  A API `GetUnderrunTime()` devolve os timestamps dos dois. O medidor de desempenho mostra contadores de xrun. (API; CL "Performance meter: optionally show xrun counters"; CL "time display background flashes red when there are underruns")
- **[Inferência]** O REAPER **não para a gravação** num xrun. Ele sinaliza e continua. Não encontrei documento de opção "stop on xrun" ou de marcadores de xrun no REAPER.
- **Ardour [Doc]:** "Stop recording when an xrun occurs" (opcional), "Show xrun markers in regions" (marca o ponto na região gravada) e um contador de xruns no Recorder ("each one is a red flag for the recording quality"). (preferences.html, Transport > Dropout (xrun) Handling e Appearance; https://manual.ardour.org/recording/the-recorder/) **[Código]** O `DiskWriter` mantém um vetor `_xruns` com as posições de xrun da passada. (ARDSRC `disk_writer.cc`)
- **[Doc]** O REAPER tem tratamento de underrun para ALSA no Linux. (CL v5.961)

#### 5.3 Segurança da gravação

- **[Doc]** Disco espelhado (gravação simultânea no secundário como backup invisível). (RP)
- **[Doc]** Arquivos novos a cada X MB (limita a perda se algo corromper). (UG §3.37)
- **[Doc]** "Stop processing audio in the event of disk failure/disk full". (UG §22.6)
- **[Doc]** Checagem de espaço livre antes de gravar. (UG §3.37)
- **[Doc]** Autosave e backups temporizados configuráveis ("whether to apply this when not recording, when stopped, or at any time"). (UG §3.1)
- **[Doc]** Prompt de salvar/apagar ao parar. (UG §3.37)
- **[Doc]** O REAPER lida com tracks apagadas ou reordenadas durante a gravação. (CL v7.06)
- **[Doc]** O dispositivo fica aberto com tracks armadas. (UG §22.6)
- **Ardour:** Rec-safe por track, "Disable per-track record disarm while rolling", stop-on-xrun e marcadores de xrun (ver acima).
- **[Inferência]** No REAPER, o que já foi gravado sobrevive a um crash: os WAV ficam na pasta, porque cabeçalho e tamanho são atualizados ao fechar ou periodicamente. Não achei documento oficial sobre recuperação de WAV após crash. Para o LinuxMusic, recomendo **atualizar o cabeçalho WAV/RF64 periodicamente** ou usar um formato que se recupere sozinho, e mostrar os arquivos órfãos ao reabrir a sessão.

---

### 6. Especificidades de Linux

#### 6.1 REAPER no Linux

- **[Doc]** O REAPER tem build nativo para Linux x86_64, i686, aarch64 e armv7l. (https://www.reaper.fm/download.php) O **UG v7.85 não tem seção de configuração de áudio para Linux**: só Windows (§1.15.1) e macOS (§1.15.2). A única menção específica a Linux é a varredura de LV2.
- **[Doc]** Backends documentados no changelog: **JACK** e **ALSA**. Exemplos:
  - "fix crashes/incorrect priority when using JACK/PipeWire" (v7.18);
  - "allow configuring JACK MIDI port count (62/64 by default, 0–128)" (v7.18);
  - "allow using JACK with ALSA-MIDI devices by configuring JACK with 0 MIDI inputs/outputs" (v7.34);
  - "automatically try ALSA subdevices when the default subdevice fails" e "only try suspending PulseAudio when ALSA device returns busy status" (v7.44);
  - "allow manually overriding JACK realtime settings, may help some pw-jack users" (v7.73);
  - "add ALSA MIDI support" (v5.94).

  (CL)
- **[Secundária]** Não há backend PipeWire nativo. Usa-se o PipeWire pela camada JACK (`pipewire-jack`), ou o ALSA direto, que é exclusivo e bloqueia o dispositivo para outros apps. (https://www.linux-magazine.com/Issues/2022/257/Reaper ; https://linuxdj-project.pages.dev/notes/reaper-on-pipewire-in-2026-low-latency-setup/ , blog não oficial)
- **[Inferência]** No modo JACK, o REAPER registra N portas de captura e playback, e o "Input: Mono 1" do track aponta para a porta 1 do *próprio* REAPER. A conexão com o hardware é feita fora (qjackctl/qpwgraph) ou pelas opções de auto-conexão. A latência reportada vem de `jack_port_get_latency_range` (inferência sobre a API JACK, não documentado pelo REAPER).

#### 6.2 Ardour no Linux

- **[Doc]** Backends: **ALSA, PulseAudio e JACK** (mais NetJack). MIDI: ALSA sequencer ou ALSA raw. O PipeWire entra pela camada JACK. (https://manual.ardour.org/sessions-tracks/sessions/audiomidi-setup/) O backend PulseAudio é só de playback (tutorial oficial, segundo o resumo de busca).
- **[Secundária]** Os desenvolvedores do Ardour recomendam o backend **ALSA** para produção. No JACK, a latência sistêmica só se altera reiniciando o jackd. No PipeWire, recomenda-se medir e configurar o `ProcessLatency` do nó. (https://discourse.ardour.org/t/linux-use-alsa-not-jack/106514 ; https://discourse.ardour.org/t/trouble-with-latency-compensation-and-pipewire/108177)

#### 6.3 Modelo de gravação do Ardour (resumo comparativo)

| Aspecto | REAPER | Ardour |
|---|---|---|
| Arm | Por track, "on the fly", auto-arm por seleção | Por track + Global Arm + Rec-safe + bloqueio de desarme rodando |
| Monitoramento | Por track: off / normal / tape auto + "monitor media when recording" | Por track In/Disk explícito, ou automático por tabela-verdade (auto-input de sessão, tape-machine mode global) |
| Unidade de take | Take dentro de um item, ou lane fixa (v7) | Região numa playlist. Takes = playlists novas ("Take.#n") ou camadas |
| Comping | Takes + split, ou comp lanes (cópia de trechos) | Camadas + ferramentas de layer, ou "cherry-pick" de playlists para outra track |
| Arquivo | 1 arquivo (N canais) por track por passada | 1 arquivo **mono por canal** por passada (`%L`/`%R`/`%a`…) |
| Formato | Por projeto e **por track**; WAV/FLAC/WavPack/OGG/MP3… | Por sessão: WAV/BWF/CAF/RF64; float 32 (padrão)/24/16 |
| Alinhamento | Latência do driver + offsets manuais (ent./saída) | Latência por porta (API JACK/backend) + latência sistêmica calibrada |
| Xrun | Sinaliza (amarelo = dispositivo, vermelho = mídia), contador | Contador, marcadores na região, stop opcional |
| Buffer de disco | "Advanced disk I/O" (opaco) | Segundos de buffer configuráveis + butler em chunks |

---

### 7. Implicações para o LinuxMusic (recomendações, todas **[Inferência]**)

1. **Modelo de track:** a entrada é `{nenhuma | áudio(porta inicial, largura 1/2/N) | MIDI(dispositivo, canal)}`. O modo de gravação é `{input, input forçado, output(pre-FX | post-FX | post-fader, com ou sem compensação), MIDI overdub/replace/touch/latch, disable/só monitorar}`, e o monitoramento é `{off, on, tape-auto}` mais "tocar mídia ao gravar". Isso replica o `I_RECINPUT`/`I_RECMODE`/`I_RECMON` do REAPER e cabe num enum pequeno.
2. **Pipeline em tempo real:** a thread JACK/PipeWire copia os buffers de captura das tracks armadas para ring buffers lock-free (alguns segundos, configurável como no Ardour). Uma thread de disco escreve em blocos grandes. Nunca fazer I/O nem alocação na thread de áudio.
3. **Gravar sempre a passada inteira** (incluindo pre-roll e loops), num arquivo por track (ou por canal, ver o item 8), e representar punch, takes de loop e lanes como **janelas não destrutivas** sobre esse arquivo. Os modos de punch, loop e lanes viram regras de criação de clips depois do stop, não lógica no caminho de áudio.
4. **Alinhamento:** posição gravada = posição do transporte − (latência de captura da porta + latência de playback) + offset manual (ent./saída, em samples). No JACK/PipeWire, pegar as latências com `jack_port_get_latency_range` nas portas da própria app. Oferecer calibração por loopback, como o ReaInsert ping e o "Calibrate Audio" do Ardour.
5. **Monitoramento sem PDC:** o caminho de monitoramento da track armada não deve ser atrasado pela PDC das outras. Opcionalmente, auto-bypass de plugins latentes ao armar e "preservar PDC monitorada" no take.
6. **Xruns:** contar e marcar na região as posições de xrun, com opção de parar a gravação (como no Ardour), e piscar o transporte (como no REAPER).
7. **Segurança:** caminho secundário de backup, arquivo novo a cada X MB/minutos, checagem de espaço, atualização periódica do cabeçalho e "Discard last take" / prompt ao parar.
8. **Formato:** WAV/RF64 (ou W64) e FLAC. No LinuxMusic, a escolha entre arquivo intercalado (REAPER) e um mono por canal (Ardour) é uma decisão de design: o mono por canal facilita a edição de canais individuais, e o intercalado reduz o número de arquivos e de seeks.
