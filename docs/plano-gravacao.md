# Plano técnico: estabilidade e gravação multitrack

Base:
- `gravacao-levantamento.md`: o levantamento do sistema e as decisões do usuário (§6, segunda rodada e etapa 0);
- `etapa-3-gravacao.md`: o objetivo.

Este documento diz **como** fazer, em que ordem, e como verificar cada passo. Ele é atualizado conforme o trabalho
avança, com cada passo marcado como feito.

Regras para todo o trabalho:
- Nenhum atalho: cada problema é corrigido na causa.
- Cada passo termina com testes passando, a bateria A/B (que não pode mudar o som dos projetos sem razão) e um
  commit.
- Primeiro no Mac (CoreAudio). O ALSA fica escrito e compilando, e é testado no Linux depois.

---

## Arquitetura

### 1. Drivers (API nova de `AudioDevice`)

Hoje o driver só pede saída estéreo ao motor, e a entrada chega por um ring separado, sem sincronia. Passa a ser
**duplex e síncrono**:
- **O callback entrega entrada e saída juntas:** o driver chama `AudioEngine::renderNextBuffer(input, output)`,
  sendo `input` uma view planar de N canais de entrada para os mesmos frames da saída. O motor fatia os dois em
  períodos internos e, antes de renderizar cada período, publica a fatia de entrada daquele período
  (`AudioEngine::capture()`). Assim a entrada fica **no mesmo período e no mesmo relógio**: não há ring nem deriva.
- **Os drivers informam:**
  - `inputChannelCount()` e `outputChannelCount()`;
  - os nomes dos canais de entrada;
  - `inputLatency()` e `outputLatency()` em frames (a latência do hardware e dos buffers);
  - a taxa efetiva.
- **Captura sob demanda:** `setCaptureEnabled(bool)`. Ela abre (ou liga a leitura da entrada) só quando há faixa
  armada ou monitorando.
- **Reabrir sem reiniciar:** `AudioEngine::changeAudioDevice()` para o transporte, fecha o driver, cria o novo com os
  ajustes novos e retoma, sem reiniciar o app. Durante a gravação, fica bloqueado.
- **A taxa é a do projeto:** o driver abre o dispositivo na taxa do projeto; se não conseguir, abre na mais próxima
  e o motor converte (`AudioResampler`), avisando.
- **Drivers:**
  - **CoreAudio**, novo: duplex num AudioUnit HAL (`kAudioUnitSubType_HALOutput`) com a entrada ligada,
    multicanal, com a latência das propriedades `kAudioDevicePropertyLatency`, `SafetyOffset` e `BufferFrameSize`;
  - **ALSA**, reescrito: `hw:` exclusivo, duplex com `snd_pcm_link`, o formato nativo (S32/S24_3/S16, intercalado
    ou não) convertido para float, latência por `snd_pcm_delay` e pelos tamanhos de período e buffer, xrun por
    `snd_pcm_recover` (contado), e a placa pedida ao PipeWire/PulseAudio por `org.freedesktop.ReserveDevice1`
    (DBus);
  - **JACK:** N portas de entrada, a entrada lida no mesmo ciclo, latência por `jack_port_get_latency_range`;
  - os outros (PortAudio, SDL, Pulse, sndio, Dummy) se adaptam à API: sem entrada (0 canais) onde não capturam.
- **O ring antigo (`pushInputFrames`, `inputBuffer`) sai.**

### 2. Thread de áudio limpa (etapa 1)
- **Nada de alocação, E/S ou trava** no caminho do período:
  - os play handles vêm de pools pré-alocados;
  - o `LatencyDelay` aloca fora (com capacidade reservada);
  - os resamplers são criados fora;
  - não há `fprintf`: um contador atômico de eventos é lido pela interface;
  - não há eventos Qt emitidos do áudio (a interface consulta por timer).
- **`m_changeMutex`:** o objetivo é o áudio nunca esperar pela interface. Em passos:
  1. listas de objetos que o áudio percorre (play handles, bus handles, faixas, clipes) passam a ser publicadas
     como **snapshots imutáveis** (`std::shared_ptr<const T>` trocados atomicamente, com descarte adiado fora do
     áudio);
  2. as estruturas restantes ganham travas finas tentadas (`try_lock`) pelo áudio: se a interface está mexendo,
     o período usa o snapshot anterior;
  3. o mutex do período inteiro sai.

  Cada passo roda com TSan e a bateria A/B.
- **Transporte** (`Transport`): estado atômico (parado, tocando, gravando, pré-roll, count-in), o **contador
  absoluto de frames da linha do tempo** (`int64`) e as ações globais Tocar, Parar, Gravar, que a barra chama
  (hoje ela chama ações dos editores).
- **O estado de gravação sai das views** (`PianoRoll::m_recording`).

### 3. Mapa de tempo
- **`TempoMap`:** uma lista ordenada de marcadores `{tick, bpm, salto|rampa, fórmula de compasso}`.
- **Conversões exatas** ticks ↔ frames, com rampa linear de BPM. Na rampa, o tempo é a integral, calculada em forma
  fechada com o logaritmo.
- **O motor usa o mapa** em vez de `Engine::framesPerTick()` global; o `framesPerTick` passa a depender da posição.
- **A automação de BPM de projetos antigos vira marcadores** ao abrir: cada ponto vira um salto, e as rampas
  lineares viram rampas.
- **Interface:** uma faixa de tempo acima da régua; duplo clique cria um marcador, arrastar move, e um menu edita
  BPM, compasso e salto ou rampa.

### 4. Mídia do projeto
- **`MediaPool`:** a pasta `<projeto>_media/`; caminhos relativos no projeto; uma pasta temporária
  (`~/.cache/linuxmusic/unsaved-<id>/`) para projeto não salvo, movida ao salvar; estado offline com a referência
  mantida; limpeza de órfãos ao salvar ou fechar.
- **Leitura em streaming** (`DiskReader`): uma thread de disco lê adiantado (alguns segundos) os trechos que vão
  tocar, para buffers pré-alocados que o áudio consome sem trava. Os arquivos ficam abertos em leitura com
  `libsndfile`.
- **Cache de picos** (`PeakCache`): níveis de min/máx por bloco (por exemplo 256, 4096 e 65536 frames), em arquivo
  `.peaks` ao lado da mídia. Ele cresce **ao vivo** durante a gravação, alimentado pelo gravador, para a forma de
  onda aparecer enquanto grava.

### 5. Faixa de gravação (`AudioTrack`)
- **Um tipo novo de faixa.** O `SampleTrack` continua até a faixa nova cobrir loops e samples.
- **Itens de áudio** (`AudioItem`): posição e comprimento em **ticks**, porque são musicais. Cada item tem:
  - a referência à mídia (original e renderizada);
  - o BPM de quando foi gravado;
  - o offset no arquivo em frames;
  - fades de entrada e saída automáticos de ~5 ms;
  - ganho em dB.
- **Lanes** (`AudioLane`): cada passada é uma lane. A lane de cima toca; as outras ficam guardadas.
- **Comping:** uma lane de comp é feita de pedaços das outras lanes, e é a que toca quando existe.
- **Crossfade automático** onde itens da lane que toca se sobrepõem.
- **Entrada, armar e monitorar:** `AudioInput`, armar e monitorar (desligado, ligado, auto), com a mesma semântica
  do `SampleTrack`.
- **Reprodução:** a faixa publica um snapshot imutável do que toca (itens resolvidos em frames pelo mapa de tempo);
  o áudio lê da mídia em streaming e soma, com fades e ganho. Sem `new` no áudio.
- **Desfazer por comandos** (ver 7).

### 6. Gravação
- **Captura:** a cada período, para cada faixa armada, o motor copia os canais de entrada dela (mono ou estéreo,
  `AudioInput.firstChannel/channelCount`) da fatia de captura para um **ring SPSC pré-alocado** por faixa. A mesma
  entrada pode ir para várias faixas.
- **`DiskWriter`:** uma thread esvazia os rings e escreve **WAV float de 32 bits** intercalado
  (`<faixa>_<passada>.wav`). O cabeçalho é atualizado a cada ~1 s, para o arquivo ser válido até o último bloco
  escrito se o programa cair. Ela também alimenta o cache de picos ao vivo. Disco cheio ou falhando: para a gravação
  e avisa, sem perder o já gravado.
- **Alinhamento:** a posição do take = o frame da linha do tempo em que a captura começou − (latência de entrada +
  latência de saída + 1 período do motor + o offset de calibração + o ajuste manual). Fica salvo no projeto.
- **Modos:**
  - **gravar parado:** a gravação começa do cursor, com pré-roll e count-in se ligados;
  - **gravar tocando:** punch manual na hora; apertar de novo sai;
  - **auto-punch:** grava a passada inteira e o item fica recortado à região de punch, sem destruir o arquivo;
  - **loop:** a passada inteira vai para um arquivo, e cada volta vira uma lane com uma janela no arquivo. Uma volta
    com menos de 90% é descartada;
  - **sem faixa armada:** arma a selecionada, se ela grava; se não, avisa;
  - **ao parar:** os takes viram itens na hora, numa entrada só do desfazer. Desfazer os remove, e os arquivos ficam
    como órfãos limpos ao salvar ou fechar.
- **Monitoração por software:** a entrada da faixa armada entra como fonte da faixa, passa pelos efeitos dela e pela
  mesa. Os modos são desligado, ligado e auto (tape auto: ouve parado ou gravando, não tocando).
  - Enquanto armada, os plugins com latência da faixa são contornados.
  - O metrônomo sai só no monitor, nunca na exportação nem na mesa.
- **Xrun:** a gravação continua com silêncio no trecho perdido, o ponto fica marcado no item e um contador avisa.
- **Calibração por loopback:** o botão "calibrar" toca um pulso na saída, grava pela entrada (com um cabo ligando as
  duas), acha o pico por correlação e salva o offset.

### 7. Desfazer por comandos
- **`UndoStack`:** comandos (`apply`/`revert`) que guardam só a diferença.
- **O que usa comandos:** a faixa de gravação, os itens, as lanes, o comping, o ganho, os fades, os marcadores de
  tempo, armar e o fim de uma gravação.
- **Convivência:** o `ProjectJournal` antigo continua para os objetos de antes. Os dois ficam numa linha do tempo
  única de desfazer, com o journal antigo encapsulado como um comando.

### 8. Gravação MIDI
- **As faixas de instrumento armadas gravam na linha do tempo**, com o mesmo transporte, punch e loop.
- **Modo:** padrão overdub; uma chave na faixa alterna entre overdub, replace e take em lane.
- **Quantização na entrada:** opcional, desligada por padrão.
- **Entrada MIDI por faixa:** dispositivo e canal, padrão todos/omni.
- **As notas** recebem o tempo em frames do evento (com a latência de entrada MIDI) e são convertidas para ticks pelo
  mapa de tempo.
- **A gravação presa ao Piano Roll sai**, e o Piano Roll mostra o clipe sendo gravado.

### 9. Time-stretch renderizado
- **Ao mudar o BPM ou o mapa de tempo, quando há itens fora do BPM em que foram gravados:**
  1. um modal mostra "vai renderizar N itens, ~X s", com Renderizar e Cancelar; Cancelar volta o BPM;
  2. uma thread renderiza com o **Rubber Band**, a partir do **original**, um arquivo `st<bpm>_<nome>.wav`, guardado
     ao lado;
  3. o modal mostra o progresso;
  4. o item passa a tocar o renderizado.
- **Com mapa de tempo com rampas,** o item é esticado com o mapa de tempo do Rubber Band.
- **Sem a biblioteca disponível no build,** cai para reamostragem (muda o tom) e avisa.

### 10. Estabilidade (etapa 0)
- **Sanitizers:**
  - `build-asan` (AddressSanitizer + UndefinedBehaviorSanitizer) e `build-tsan` (ThreadSanitizer), configurados
    pelo CMake (`-DLMMS_SANITIZERS`);
  - rodam os testes, a bateria A/B e um roteiro de abrir, tocar e fechar projetos;
  - cada achado é corrigido na causa e documentado.
- **Carga robusta:**
  - projetos e presets truncados, XML inválido e valores fora de faixa não derrubam o programa;
  - um sample que sumiu fica offline e mantém a referência, em vez de ser apagado;
  - um plugin que não carrega vira "não carregado" e guarda os ajustes;
  - uma **lista de problemas** mostra tudo isso depois de abrir;
  - testes com arquivos corrompidos gerados.
- **Autosave contínuo:**
  - um snapshot do projeto é serializado na thread da interface depois de cada mudança, com debounce de ~2 s, e
    escrito em disco por uma thread, de forma atômica (arquivo temporário e `rename`);
  - ao reabrir depois de uma queda, um diálogo oferece recuperar;
  - os takes que estavam sendo gravados voltam a partir dos arquivos, válidos até o último bloco.
- **Relatório de crash:** um handler de sinal (SIGSEGV, SIGABRT…) grava pilha e contexto num arquivo, com
  `backtrace` e só funções seguras em sinal, e tenta salvar o projeto em recuperação.
- **Plugins:**
  - um plugin que estava ativo numa queda é registrado na lista de suspeitos;
  - na próxima vez, roda isolado pela ponte do Carla (`carla-bridge`) quando disponível, ou fica "não carregado"
    com aviso;
  - uma opção por plugin força o isolamento.

---

### 11. Faixa de áudio: o desenho da implementação (13/11)

A decisão é o modelo C (`gravacao-levantamento.md` §6): uma faixa nova e limpa, que vira a faixa de áudio; o
`SampleTrack` sai quando ela cobrir loops e samples, e os projetos convertem. Ela é feita sobre as peças que já
existem (`AudioStream`, `PeakCache`, `TimeStretcher`, `AudioRecorder`, a base `Clip`/`ClipView`), sem o código
antigo de reprodução do `SampleTrack` (um `SamplePlayHandle` criado com `new` no áudio para cada clipe).

Passos, cada um com testes, A/B e commit:
1. **Desfazer por comandos** (`UndoCommand`): `apply`/`revert` com só a diferença, numa linha do tempo única com o
   journal antigo (um comando é um passo do `ProjectJournal`, e os grupos valem para os dois). Os comandos acham
   a faixa pelo id do journal e o item por um id próprio, então um objeto que sumiu é pulado.
2. **Modelo** (`AudioTrack`, `AudioItem`): o item tem o arquivo (original e renderizado, com o timing, como o
   `SampleClip`), posição, comprimento e início no arquivo em ticks, ganho em dB, fades de entrada e saída (5 ms
   por padrão, ajustáveis, com curva), lane e mudo, e a memória (automático, RAM ou disco). A lane 0 é a de cima, e
   é a que toca; uma lane de comp, quando existe, toca no lugar dela. Toda edição é um comando. Salva e carrega.
3. **Reprodução:** um player por faixa, criado uma vez, que soma os itens da lane que toca a cada período, a partir
   de um snapshot imutável publicado pela interface a cada mudança (o antigo é liberado fora do áudio). Fades,
   ganho e crossfade onde itens se sobrepõem; reamostragem com estado por item, contínua entre períodos. Nada de
   `new` no áudio.
4. **Interface:** o cabeçalho de duas linhas (nome, M/S, armar e monitor; entrada, medidor, VOL e PAN), os itens
   com a forma de onda dos picos, a alça de ganho no topo, as alças de fade nos cantos, aparar, cortar e slip; as
   lanes visíveis, com a que toca destacada, e o comping arrastando sobre as lanes.
5. **Gravação na faixa nova:** cada passada é uma lane nova no topo (a que toca), e o loop faz uma lane por volta.
6. **Tempo:** os itens seguem o mapa de tempo, renderizados pela tela de processamento como os clipes de hoje.
7. **Conversão:** "Adicionar faixa de áudio" cria a faixa nova. **Decisão do usuário (13/11): cada faixa pensada
   para o seu objetivo.** A faixa de sample fica, para tocar samples curtos na grade e as batidas do Editor de
   Sequências, e deixa de gravar (sem armar, entrada nem monitoração). A faixa de áudio é a de gravação e de
   arquivos longos. Ao abrir, as faixas de sample com gravações (takes ou clipes presos ao tempo) viram faixas de
   áudio; as de samples na grade ficam como estão.

## Ordem de trabalho

1. Etapa 0: sanitizers e correções.
2. Etapa 1: thread de áudio limpa, o transporte e o contador de frames.
3. API nova de drivers, o driver CoreAudio, e a gravação funcionando no Mac de ponta a ponta:
   1. mídia;
   2. gravador;
   3. faixa de gravação;
   4. interface mínima.
4. Mapa de tempo; time-stretch renderizado.
5. Desfazer por comandos; lanes, comping, fades e ganho; a interface completa de duas linhas.
6. Gravação MIDI.
7. Etapa 1, parte final: o motor sem o mutex do período.
8. Etapa 0, parte final: carga robusta, autosave e recuperação, crash handler e isolamento de plugins.
9. ALSA e JACK (escritos e compilando; testados no Linux pelo usuário).
10. Integração final: testes, sanitizers, A/B, gravação real no Mac, documentação.

As anotações do usuário de 13/11 (mesa, Explorador, transporte, configurações, arquivos, exportação, samples e
plugins) viraram as etapas 5 a 13, em `plano-anotacoes-13-11.md`, com a ordem em relação a este plano.

## Estado (noite de 12/11/2026)

Feito, com testes, A/B e commit (em `master`, sem push):
- **Etapa 0, sanitizers:**
  - `build-asan` (ASan + UBSan) e `build-tsan` configurados com `-DWANT_DEBUG_ASAN/UBSAN/TSAN=ON`.
  - Rodam limpos:
    - os testes;
    - a bateria A/B renderizada;
    - abrir, tocar e fechar a interface.
  - Correções:
    - o espelho intercalado do `AudioBuffer` com faixas de mais de 2 canais;
    - os workers perdiam o despertar e travavam ao sair (agora usam `std::atomic::wait`);
    - cerca de 230 corridas de dados achadas pelo TSan nos renders:
      - no motor: `Song`, `Mixer`, `InstrumentTrack`, `InstrumentPlayHandle`, VST sync e o renderizador;
      - nos plugins: Xpressive, Sid e Flanger;
    - os plugins TAP não linkavam com sanitizers;
    - os testes não leem mais a configuração do usuário (`LINUXMUSIC_CONFIG_DIR`).
  - Achados de terceiros, nos submódulos, não alterados:
    - no reSID (`filter.cc`), a conversão de um valor negativo para `unsigned short` é comportamento indefinido; o resultado da tabela depende da plataforma;
    - no `fmopl.c` do OpulenZ, deslocamento à esquerda de valor negativo.
- **Etapa 0, crash:**
  - `CrashHandler` grava a pilha em `crash-report.txt`, e a partida seguinte avisa onde ele está.
  - O autosave e a recuperação do LMMS continuam como estavam.
- **Thread de áudio:** o osciloscópio lê por um `TripleBuffer`, sem sinal Qt do áudio.
- **Drivers:**
  - **API duplex:** `renderNextBuffer(input, output)` e `AudioEngine::capture()`, com nomes de entradas, latências, xruns e captura sob demanda.
  - **CoreAudio** (novo, primeira escolha no Mac): aggregate device privado para entrada e saída separadas.
  - **ALSA** reescrito:
    - duplex com `snd_pcm_link`, no formato nativo da placa;
    - recuperação de xrun;
    - reserva da placa por `org.freedesktop.ReserveDevice1`;
    - foi escrito e checado com os headers da alsa-lib 1.2.12, e ainda precisa ser testado no Linux.
  - **JACK:** N entradas, com latências.
  - **PortAudio:** entrega a entrada.
  - **SDL:** não captura mais (o callback de entrada dele não anda junto com o de saída).
- **Gravação:**
  - `AudioRecorder` + `Song::recordAudio()`, com ring SPSC, thread de disco e WAV float de 32 bits.
  - Alinhamento à amostra:
    - a regra é período − 2 períodos − latência de entrada e de saída − calibração (`recording/latencyoffset`);
    - provado com um loopback no `RecordingTest`.
  - Punch in e punch out tocando.
  - A monitoração por software (desligada, ligada ou auto) passa pelos efeitos e pela mesa.
  - Os takes viram `SampleClip` nas faixas de sample armadas, na pasta `<projeto>_media`.
- **Interface:**
  - No cabeçalho da faixa de sample: R (armar) e o botão de entrada, com o menu de entradas e a monitoração.
  - O Gravar do transporte grava as faixas armadas; sem faixa armada, grava MIDI no Piano Roll como antes.

Feito depois (mesma noite e manhã de 13/11):
- **Gravação:**
  - pré-roll e count-in, com o metrônomo sem leitura de arquivo no áudio;
  - takes em loop: cada volta vira um take do mesmo clipe;
  - comping: cortar o clipe e escolher o take de cada parte;
  - mudança de tempo durante a gravação não corta o take (o gravador trabalha em ticks);
  - medidor de nível de entrada no cabeçalho da faixa;
  - captura sob demanda: o pedido do microfone aparece quando uma faixa precisa da entrada, sem deadlock ao abrir projetos com faixa armada.
- **Gravação MIDI na linha do tempo** (`MidiRecorder`), em overdub ou replace, com o R nas faixas de instrumento.
- **Desfazer:** a gravação inteira desfaz e refaz num passo só (grupos no `ProjectJournal`). Também foi corrigido o bug que criava 2 passos falsos a cada salvamento de faixa de instrumento.
- **Mapa de tempo** (`TempoMap`):
  - marcadores com salto ou rampa, seguidos na reprodução;
  - a automação de BPM de projetos antigos vira marcadores ao abrir;
  - a faixa TEMPO fica sob a régua do Editor de Arranjo.
- **Etapa 0:** carga robusta. 140 projetos corrompidos abrem sem crash nem UB.
- **ALSA** reescrito e **JACK** com N entradas (ver acima).
- **Hardware no Mac:** o CoreAudio com o microfone abre o aggregate e roda sem xruns. Neste processo de teste o macOS entregou silêncio, o que indica falta de permissão de microfone para o processo ou entrada muda.

- **Time-stretch renderizado** (13/11):
  - `TimeStretcher` com a Rubber Band (R3, offline, com um key frame em cada compasso) e queda para reamostragem;
  - a tela de processamento (`TempoRenderController`): o modal "N itens, ~X s", o progresso item por item, e o
    Cancelar que volta o BPM e o mapa de tempo. Tudo ou nada, sem arquivo sobrando;
  - os itens gravados ficam musicais: a mudança de tempo mantém lugar, tamanho e início em ticks, e a reprodução
    acha o frame pelo timing.
  - Falta: limpar os renderizados que ficaram órfãos (junto com a mídia, item 4).

- **Mídia: streaming do disco e cache de picos** (13/11):
  - `AudioStream`: o arquivo é lido em blocos de 32768 frames. A thread de disco (`DiskReader`) lê adiantado os
    blocos dos próximos 6 s de cada clipe (e o início do ciclo perto do fim dele), pedidos pelo `StreamScheduler`
    a partir da posição da música, na thread principal. O áudio pega os blocos sem esperar. Um bloco que falta toca
    silêncio, é contado e é lido na hora pela thread de disco; renderizando offline (exportação), o áudio lê o bloco
    ele mesmo, então nada fica de fora. Os blocos que não tocam mais são liberados e só são apagados quando o
    período que podia estar lendo terminou (contagem de períodos).
  - **Opção por clipe, como a chave RAM do Ableton** (pedido do usuário): Automático (arquivos de 20 s ou mais vêm
    do disco), Carregar na RAM ou Ler do disco, no menu do clipe e salva no projeto. Os clipes do Editor de
    Sequências ficam sempre na RAM, e um arquivo que o libsndfile não lê também.
  - `PeakCache`: os picos de cada arquivo (256 frames por pico), calculados em segundo plano e guardados em
    `~/Library/Caches/.../peaks` (no Linux, `~/.cache/...`), validados pelo tamanho e pela data do arquivo. A forma
    de onda de um clipe do disco vem deles. Fica no cache do usuário, e não ao lado da mídia, porque os arquivos de
    fábrica e de pastas sem escrita também precisam de picos.
  - Verificado: tocar ao vivo e exportar do disco dá o mesmo resultado da RAM, bit a bit (`StreamingTest` e um
    render pela linha de comando), sem falta de bloco ao vivo; o TSan não acha corridas.

- **Limpeza dos arquivos órfãos** (13/11):
  - cada pasta lista num arquivo oculto (`.linuxmusic-generated`) os arquivos que o programa fez nela: takes,
    renderizações para o tempo e as cópias deles na pasta de mídia. Só esses são limpos, nunca um arquivo que o
    usuário pôs lá;
  - quando um projeto fecha (abrir outro, criar um novo, sair), os arquivos feitos para ele que o arquivo salvo não
    usa vão para a **Lixeira do sistema**, não são apagados, porque outro projeto ("Salvar como" na mesma pasta)
    pode ainda usá-los. É ao fechar, e não ao salvar, para o desfazer nunca apontar para um arquivo que sumiu;
  - um projeto sem arquivo grava numa pasta só dele (`recordings/unsaved-<data>`), presa por um lock enquanto a
    instância roda. Ao fechar sem salvar, ela é limpa e removida; ao abrir o programa, as pastas deixadas por uma
    queda que nenhuma instância segura e que o projeto recuperado não usa também são limpas;
  - `ProjectMediaTest` cobre isso, inclusive abrir outro projeto (que troca o nome do arquivo antes de fechar o
    atual) e um projeto que não abre (nada é limpo).

- **Forma de onda ao vivo** (13/11): a thread de disco do gravador faz os picos de cada take enquanto o escreve
  (como o `PeakCache` faria lendo o arquivo); a faixa mostra o take crescendo em vermelho, do início ao cursor, com
  a forma de onda (`LiveTakeOverlay`, 20 vezes por segundo, copiando só os picos novos). Ao parar, os picos viram o
  arquivo de picos de cada passada, idênticos aos feitos do arquivo depois (`RecordingTest`), e o clipe aparece já
  desenhado. Cada passada de um loop entra na lista de arquivos gerados.
- **Driver Dummy com duas entradas silenciosas**, para experimentar a gravação sem placa de som
  (`LINUXMUSIC_DUMMY_INPUT_TONE=1` toca um tom nelas, para desenvolvimento). Não quando o motor só renderiza.

- **Faixa de áudio, passos 1 a 3** (13/11, seção 11):
  - desfazer por comandos (`UndoCommand`, `ProjectJournal::execute`), numa história só com os checkpoints;
  - a base `AudioInputTrack` (volume, pan, canal da mesa, entrada, armar, monitorar, medidor), da qual derivam a
    faixa de sample e a nova;
  - `AudioTrack` e `AudioItem`: itens com arquivo (RAM ou disco), ganho, fades de 5 ms, lane e timing de tempo;
    ganho, fades, lane, memória e lane tocada mudam por comandos; salva e carrega (`audiotrack`, `audioitem`);
  - o player: um play handle por faixa, vozes pré-alocadas que seguem os ticks da música e recomeçam nos saltos,
    a lista publicada pela interface (`Handoff`), crossfade de potência igual nas sobreposições e cópia exata das
    amostras quando o arquivo está na taxa do motor;
  - `AudioTrackTest`: posição, fades, ganho, lanes, crossfade, loop, salvar e carregar, desfazer; TSan limpo.
  - A view da faixa é a da faixa de sample, generalizada; o cabeçalho de duas linhas e as alças vêm no passo 4.
- **Faixa de áudio, passo 5: gravação** (13/11): uma faixa de áudio armada grava como a de sample; as passadas
  viram lanes novas no topo (a última passada na lane que toca, as lanes de antes abaixo), cada item preso ao
  tempo em que foi tocado. Um desfazer volta tudo. Os arquivos dos itens são recursos do projeto (copiados ao
  salvar, nunca limpos enquanto usados), e os itens do disco são lidos adiantados como os clipes.
- **Corrigido:** mover um take sem mudar o tempo pedia renderização. O último ponto do timing ficava no fim exato do
  arquivo, dentro de um tick, e a comparação usava a grade. Agora o ponto fica na grade (o time-stretch preserva o
  fim real do áudio), e os takes de antes são comparados com meio tick de tolerância nesse ponto.
- **Faixa de áudio, passos 4, 6 e 7** (13/11): cabeçalho de duas linhas; itens nas suas lanes, com as que não
  tocam esmaecidas; alças de fade e ganho; o crossfade desenhado; "Ouvir esta lane" e comp; os itens seguem o mapa de
  tempo (renderizados como os clipes); o botão "Adicionar faixa de áudio"; só a faixa de áudio grava; arquivos
  longos importados viram faixa de áudio; faixas de sample com gravações, armadas ou com entrada abrem como faixas
  de áudio (takes viram lanes). Arrastar clipes para outra faixa do mesmo tipo, como no REAPER.
- **Lanes opcionais** (13/11, pedido do usuário): nas configurações, Gravação → Lanes. Desligadas, a gravação é
  simples: o take novo fica na lane que toca e substitui o que cobre (os itens por baixo são aparados, cortados em
  volta ou removidos, sem mexer nos arquivos), e no loop só a última passada fica. A interface do passo 4 esconde
  as lanes nesse modo.

Por fazer, na ordem do plano:
- **Mapa de tempo:** a fórmula de compasso por marcador. (O desfazer das edições ficou pronto em 13/11: cada edição
  da faixa TEMPO é um comando, e um arraste é um passo.)
- **Edição e interface:**
  - desfazer por comandos (o atual é por grupos de snapshots);
  - lanes visíveis;
  - fades automáticos;
  - ganho por item;
  - a faixa nova (`AudioTrack`) e o cabeçalho de duas linhas.
- **Etapa 1:**
  - o motor sem o mutex do período;
  - o transporte com contador de frames.
- **Etapa 0:**
  - isolamento de plugins;
  - a lista de problemas ao abrir um projeto.
- **Conhecido:**
  - a reprodução de samples (`Sample::play`) atrasa 1 amostra por causa do reamostrador, mesmo na razão 1; a gravação está alinhada ao que se ouve;
  - a entrada do microfone no hardware do Mac não foi testada, porque o pedido de permissão do macOS bloqueia sem alguém para responder.

## Verificação
- **Testes de unidade** (`tests/`) para cada peça: mapa de tempo, rings, gravador com um driver falso, alinhamento
  de latência, mídia, faixa de gravação, desfazer, carga corrompida.
- **Driver falso** (`AudioDeviceFake` nos testes): entrada sintética conhecida (por exemplo, um pulso num frame
  definido) e latência declarada, para provar que o take cai na posição certa, amostra por amostra.
- **A bateria A/B** a cada passo: os projetos existentes não podem mudar de som.
- **Gravação real no Mac:** CoreAudio com o microfone e a saída do Mac (e o loopback para a calibração).
