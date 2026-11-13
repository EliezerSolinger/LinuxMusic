# Pendências

A lista do que falta, atualizada a cada passo (o detalhe de cada item fica em `plano-gravacao.md`).

## Em andamento

- [ ] **Thread de áudio sem alocação, E/S ou trava** (etapa 1)
  - [x] detector: `AudioThreadAllocationTest` conta cada alocação feita na thread de áudio e mostra onde
  - [x] automação sem alocação (era metade)
  - [ ] ciclo de vida das notas: `NotePlayHandle` e os dados de nota que cada instrumento cria com `new`
  - [ ] lista de play handles do motor
  - [ ] monitoração do master
  - [ ] o teste passa a exigir zero (tirar o `QEXPECT_FAIL`)

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
  (bloqueado durante a gravação; avisa se abriu outro dispositivo ou outra taxa). O driver MIDI ainda pede reinício
- [ ] **A taxa é a do projeto** (hoje é a das configurações, global): o projeto guarda a sua, e quando a placa não
  tem, converte e avisa

Motor e estabilidade:

- [ ] **Transporte com contador de frames** (etapa 1): estado atômico (parado, tocando, gravando, pré-roll, count-in),
  contador absoluto de frames da linha do tempo, ações globais Tocar, Parar e Gravar
- [ ] **Motor sem trava** (etapa 1): listas publicadas como snapshots imutáveis, travas finas tentadas, e o fim do
  mutex do período inteiro; cada passo com TSan e A/B
- [ ] **Isolamento de plugins** (etapa 0): plugin ativo numa queda vai para a lista de suspeitos; da próxima vez roda
  isolado pela ponte do Carla, ou fica "não carregado" com aviso; opção por plugin para forçar o isolamento
- [ ] **Fórmula de compasso por marcador** no mapa de tempo
- [ ] **Latência dos plugins:** VST, Carla e ZynAddSubFX ainda informam 0
- [ ] **Sample::play** atrasa 1 amostra por causa do reamostrador, mesmo na razão 1
- [ ] **Lista de problemas ao abrir um projeto:** conferir que mostra tudo (samples que sumiram, plugins não carregados)
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
