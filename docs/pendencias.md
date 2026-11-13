# Pendências

A lista do que falta, atualizada a cada passo (o detalhe de cada item fica em `plano-gravacao.md`).

## Em andamento

- [ ] **Thread de áudio sem alocação, E/S ou trava** (etapa 1)
  - [x] detector: `AudioThreadAllocationTest` conta cada alocação feita na thread de áudio, por lugar (todas,
    não só as primeiras)
  - [x] automação sem alocação (era metade)
  - [x] ciclo de vida das notas: buffers de período de um pool (`BufferManager`), notas filhas e da faixa em listas
    ligadas pela própria nota, filtro da nota e o seu segundo estágio feitos com o lugar da nota no pool, um espaço
    de 16 KB na nota para os dados do instrumento (`makePluginData`), arpejo sem lista alocada
  - [x] o motor: lista de clipes, lista de faixas, envios do mixer, listas de play handles reservadas
  - [x] o teste exige zero: música com faixa de áudio e instrumento; acordes, arpejo, filtro duplo, envio e efeito;
    cada instrumento tocando
  - [x] instrumentos sem alocar: TripleOscillator, BitInvader, Kicker, LB302, Monstro, NES, Organic, Sfxr, SID,
    Watsyn (versões das ondas compartilhadas), FreeBoy, OpulenZ, AudioFileProcessor, SlicerT
  - [x] os que fazem a voz com bibliotecas: FreeBoy e Mallets com vozes feitas antes (`VoicePool`,
    `FreshVoicePool`, som idêntico); ZynAddSubFX, Xpressive e Vibed pelo pool de tempo real do `operator new`
    (`RealtimeAllocator`, TLSF de 64 MB); estados do libsamplerate num pool; estado aleatório no núcleo
  - [x] o teste vê também o `malloc` direto (Qt, C) pela biblioteca `AllocationObserver`
  - [x] faixa de sample (grade): os clipes tocam com handles feitos antes, que voltam à faixa (`dispose()` no lugar
    do `delete` do motor)

## A fazer

Decisões do usuário ainda não feitas (`gravacao-levantamento.md` §6):

- [x] **Gravar sem faixa armada** arma a faixa selecionada (clique no cabeçalho ou na faixa; ela fica destacada), se ela grava; se não, avisa
- [x] **Calibração por loopback:** botão "Medir..." em Configurações → Áudio → Gravação (`LatencyCalibration`;
  o teste acha a latência escondida de um loopback simulado, exata). Antes: botão "calibrar" que toca um pulso, grava pela entrada (com um cabo), acha o pico por
  correlação e salva o offset
- [x] **Metrônomo configurável:** só gravando ou também tocando, volume, acento no 1 (Configurações → Áudio →
  Gravação); o clique não aloca mais na thread de áudio (um play handle próprio)
- [x] **Xrun gravando:** o ponto fica marcado em vermelho no take (salvo no projeto), e o fim da gravação avisa
  quantas vezes o áudio falhou
- [x] **MIDI, take em lane:** as lanes passaram para a base de todas as faixas e clipes (`Clip::lane`,
  `Track::playingLane`); a faixa de instrumento toca só a lane que ouve, e o modo "Take em lane" do R grava numa lane
  nova em cima
- [x] **MIDI, quantização ao gravar:** grade (1/4 a 1/64) e força (50, 75, 100%) por faixa, desligada por padrão,
  no menu do R
- [x] **MIDI:** entrada MIDI por faixa no cabeçalho: botão com receber, dispositivos e canal (omni por padrão)
- [x] **Cabeçalho de duas linhas em todas as faixas** (altura padrão 56): instrumento e áudio com nome, atividade,
  armar e entrada em cima, canal, VOL e PAN embaixo; sample e pasta com o mesmo desenho; sequência e automação com a
  mesma altura
- [x] **Trocar dispositivo, buffer ou taxa sem reiniciar:** ao salvar, o dispositivo reabre com os ajustes novos
  (bloqueado durante a gravação; avisa se abriu outro dispositivo ou outra taxa). O driver MIDI também: as portas
  das faixas e controladores passam para o novo, ligadas aos mesmos dispositivos onde ele os tem (avisa os que
  faltam), e os menus MIDI se refazem; o MIDI do JACK, que usa a conexão do áudio do JACK, fecha antes dele e reabre
  depois (antes ficava com um ponteiro solto)
- [x] **A taxa é a do projeto:** o projeto guarda a sua; abri-lo reabre o dispositivo nela (projeto novo: a das
  configurações); a placa que não tem roda na sua, os arquivos são convertidos e o relatório do projeto avisa

Motor e estabilidade:

- [x] **Transporte com contador de frames** (etapa 1): `Transport` no núcleo, com o estado num atômico (parado, pausa,
  tocando, pré-roll, count-in, gravando; o fim do pré-roll vem da thread de áudio), o frame da linha do tempo exato
  pelo mapa de tempo (contado de período em período, recontado nos saltos e no ciclo), e Tocar, Parar e Gravar como
  ações do núcleo, que a barra e o Arranjo chamam. O relógio, o LFO e o Carla usam esse frame
- [ ] **Motor sem trava** (etapa 1): listas publicadas como snapshots imutáveis, travas finas tentadas, e o fim do
  mutex do período inteiro; cada passo com TSan e A/B
- [x] **Autosave contínuo** (etapa 0): `Autosave` tira um snapshot 2 s depois de cada mudança (no máximo a cada 30 s
  enquanto muda sem pausa), também tocando e gravando, e uma thread o grava de forma atômica (`QSaveFile`). Uma
  gravação em andamento vai no snapshot; ao recuperar, os arquivos dos takes são completados (`WavRepair`, o
  cabeçalho passa a dizer o que o arquivo tem) e os takes entram nas faixas como numa gravação. As configurações
  ficaram com só a chave e uma explicação
- [x] **Faixa de sample: saltos vistos na thread de áudio.** O salto (play head movido, ciclo) é visto pela própria
  faixa, que recomeça os clipes no mesmo período; antes, um sinal da interface chegava depois, e a exportação com
  faixa de sample podia sair diferente de uma vez para outra
- [x] **Plugins suspeitos** (etapa 0): enquanto o código de um plugin roda (carregar, processar, tocar a nota,
  restaurar os ajustes), a thread diz qual é (`PluginGuard::Running`), e o relatório de queda o nomeia. Na próxima
  abertura ele vira suspeito: o aviso da queda diz qual foi, os projetos o abrem "não carregado" com os ajustes
  guardados (e salvos como estavam), e adicioná-lo pede confirmação. Configurações → Desempenho → Plugins lista os
  suspeitos, com "Carregar de novo". Rodar isolado pela ponte do Carla não é possível para os efeitos nativos, LV2 e
  LADSPA (o Carla não tem um invólucro de efeito para eles no LMMS); VST e ZynAddSubFX já rodam em outro processo
- [ ] **Fórmula de compasso por marcador** no mapa de tempo
- [x] **Latência dos plugins:** VST informa o `initialDelay` do plugin (ao carregar e quando ele avisa que mudou);
  o Carla soma a dos plugins ligados do rack (no patchbay, a maior, porque as ligações não chegam ao host); o
  ZynAddSubFX não atrasa a saída, então 0 está certo
- [x] **Varredura com ASan e UBSan** renderizando todos os projetos (A/B e demos): queda ao fechar um projeto com o
  efeito do PeakController (o efeito se removia da cadeia no meio da limpeza), índices trocados no ZynAddSubFX,
  envelope do Sfxr lido além do fim, DrumSynth (espaço no fim do envelope virava ponto, número longo estourava 8
  bytes, taxa fora da faixa escrevia além do bloco), conversões indefinidas no SID, OpulenZ, TAP, SWH e Calf
- [x] **Sample::play** na razão exata de 1 toca as amostras como estão (antes o reamostrador atrasava uma amostra,
  ou filtrava, no sinc); a A/B muda só no AudioFileProcessor, por isso
- [x] **Lista de problemas ao abrir um projeto:** samples e arquivos que sumiram, soundfonts, kits, VST e plugins
  não encontrados vão para o relatório (antes, um plugin não encontrado abria uma janela para cada um); um
  instrumento que não carrega guarda os ajustes e é salvo como estava (antes, salvar o apagava)
- [ ] **Antes de publicar:** as correções em submódulos (ZynAddSubFX, adplug, veal, swh, tap, resid) estão em
  commits locais no ramo `linuxmusic` de cada um; precisam ir para forks do LinuxMusic e o `.gitmodules` apontar
  para eles
- [ ] **Integração final:** testes, sanitizers, A/B, gravação real no Mac, documentação
- [ ] **Linux:** testar o ALSA e o JACK numa máquina Linux (o usuário)
- [ ] **Mac:** testar a entrada do microfone no hardware (precisa de alguém para aceitar o pedido de permissão)
- [ ] **Arte própria** do logo e do splash (hoje são os do LMMS)

## Feito recentemente (13/11)

- [x] Faixa de áudio: modelo, player, interface (duas linhas, lanes, alças, crossfade, comp), tempo, gravação,
  botão, conversão das faixas de sample com gravações
- [x] Lanes opcionais nas configurações
- [x] Fades de 5 ms e crossfade automático; arrastar clipes para outra faixa
- [x] Clipes que cruzam mudanças de tempo ficam na grade em todo lugar
- [x] Mover um take sem mudar o BPM não pede mais renderização
- [x] Desfazer das edições do mapa de tempo
- [x] Padrão de 120 BPM
- [x] Streaming do disco com opção RAM/disco por clipe, cache de picos, forma de onda ao vivo, limpeza de órfãos
