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

## Verificação
- **Testes de unidade** (`tests/`) para cada peça: mapa de tempo, rings, gravador com um driver falso, alinhamento
  de latência, mídia, faixa de gravação, desfazer, carga corrompida.
- **Driver falso** (`AudioDeviceFake` nos testes): entrada sintética conhecida (por exemplo, um pulso num frame
  definido) e latência declarada, para provar que o take cai na posição certa, amostra por amostra.
- **A bateria A/B** a cada passo: os projetos existentes não podem mudar de som.
- **Gravação real no Mac:** CoreAudio com o microfone e a saída do Mac (e o loopback para a calibração).
