# Pendências

A lista do que falta, atualizada a cada passo (o detalhe de cada item fica em `plano-gravacao.md`).

## Feito: etapa 1 (a thread de áudio)

- [x] **Thread de áudio sem alocação, E/S ou trava** (etapa 1): sem alocação (abaixo), a E/S de arquivo nas threads
  de disco (streaming e gravação), e a trava do período sem esperar além de meio período ("Motor sem trava")
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
- [x] **Motor sem trava** (etapa 1): a thread do dispositivo não espera mais as mudanças do modelo: a trava do
  período é temporizada, e o período do dispositivo espera no máximo meio período; se a mudança continua (carregar
  um projeto), o período sai em silêncio, a música não anda nele, e conta como queda (a gravação marca no take, como
  um xrun). A exportação e os testes ainda esperam (não podem perder período). As outras travas que a thread de
  áudio pega são finas (canais do mixer entre as threads do período, pontos de automação, buffer de valores), com a
  thread principal segurando só por instantes. TSan nos testes de threads (com `tests/tsan.supp` para o que o TSan
  não vê através do Qt). Fica como ideia para depois: publicar as listas como snapshots, se a medição mostrar
  necessidade
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
- [x] **Fórmula de compasso por marcador** no mapa de tempo: um marcador pode mudar a fórmula (um compasso começa
  ali; o cortado antes termina ali); a do início é a da música. A régua, a grade e o encaixe do Arranjo seguem os
  compassos (o encaixe é por posição em compassos, igual ao de sempre sem mudanças), assim como o metrônomo (acento
  no 1 de cada compasso), o pré-roll, o mostrador, as setas, inserir/remover compasso, o tempo do Carla e dos VST, o
  MIDI exportado e importado, e o Piano Roll e o Editor de Automação (na fórmula de onde o clipe começa). A faixa de
  tempo mostra e edita a fórmula
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
- [x] **Integração final:** 41 testes; ASan e UBSan nos testes e nos 83 projetos sem nenhum erro; TSan nos testes de
  threads; A/B igual (só os plugins aleatórios variam); `gravacao-como-usar.md` atualizado. A gravação real no Mac
  fica no item do microfone, abaixo
- [ ] **Linux:** testar o ALSA e o JACK numa máquina Linux (o usuário)
  - O JACK já foi testado neste Mac em 17/11, com um servidor `jackd` de teste (`-d dummy`), também sob ASan:
    áudio e MIDI do JACK na mesma conexão, tocando e fechando sem erro, e as ligações automáticas às portas físicas
    certas. Falta o hardware real e o ALSA.
- [ ] **Mac:** testar a entrada do microfone no hardware (precisa de alguém para aceitar o pedido de permissão)
- [x] **Demos com amostras removidas:** a pasta `data/projects/demos` foi apagada (pedido do usuário em 17/11: voltam
  quando a nova versão estiver pronta), junto com três projetos de `shorties` que citavam amostras tiradas
  (Skiessi-222, sv-DnB-Startup, DirtyLove). O `ProjectMediaTest` agora também lê os `.mmpz`, que antes escapavam
  dele
- [x] **Arte própria** do logo e do splash: um anel de gravar em volta das barras de uma forma de onda, no verde do
  tema (`tools/artwork`: os SVGs e o gerador de todos os ícones, do splash e das artes dos instaladores)

## Refatoração do núcleo (17–18/11): concluída

- [x] As fases 0 a 10 de `docs/plano-refatoracao.md` (andamento e commits no fim dele). O núcleo, como ficou:
  `docs/estado-atual.md`.

## Pedidos de 17/11 (depois da refatoração do núcleo)

O usuário pediu que cada item seja feito com muita atenção. Vêm depois do `docs/plano-refatoracao.md`. A exceção é a
queda do Mallets, que é queda e entra antes.

- [x] **Mallets sem STK derruba o programa (segfault)** (consertado em a8aebdef0: a queda e o falso "falta o STK"; falta decidir se o STK continua como dependência e completar os recursos). Ao abrir, ele avisa que falta o STK e cai.
  - Consertar a queda.
  - Depois, varrer os outros instrumentos pelo mesmo padrão: dependência ausente seguida de uso.
  - Decidir se a dependência própria do STK é necessária. Hoje ela serve só a ele: ver o que o Mallets usa do STK e
    se dá para trazer só isso (licença do STK: MIT-like) ou embutir os arquivos de dados (rawwaves) que faltam.
  - Completar os recursos que faltam nele.
- [ ] **Cadeia de efeitos vira "Rack de efeitos"**, como os racks analógicos:
  - mais larga, com o painel de cada efeito que couber direto no rack;
  - por efeito, uma barrinha de topo bem pequena (ligar, nome, mover, remover), com os knobs P/L e DEC mais
    discretos;
  - o botão de configuração some para os efeitos que cabem;
  - os de tela grande ganham "Ajustar efeito", que abre a janela deles;
  - vale para os nativos e os LADSPA. Conferir o tamanho de cada janela antes de escolher a largura.
- [ ] **Sintetizadores incompletos:** levantar comentários na internet e em fóruns sobre cada sintetizador, listar
  as limitações e os problemas citados pelos usuários, e consertar todos. Primeiro a lista por sintetizador, com as
  fontes; depois os consertos.
  - Pesquisa feita: `docs/pesquisas/sintetizadores-reclamacoes.md` (22 instrumentos, com as issues do LMMS).
  - Os primeiros da lista:
    - [x] queda ao abrir a janela do AudioFileProcessor (#8647): o desenho da onda agora aceita qualquer pedido
      (faixa fora do sample, vazia, sem número, sample sem quadros, miniatura vazia) e desenha o que está dentro,
      ou nada; o AudioFileProcessor sem sample não divide por zero. `SampleThumbnailRendererTest`;
    - [x] Vibed a 192 kHz (divisão inteira que dava zero; numa taxa menor que a base, divisão por zero);
    - [x] Xpressive trava por overflow (fração por `absFraction`, sem converter para `int`);
    - [x] notas presas (OpulenZ, Zyn com arpejador): o `BuiltinPluginsTest` toca, em cada instrumento, notas que
      atravessam o fim do ciclo, para no meio, e exige silêncio seis segundos depois: nenhum ficou preso;
    - [x] LB302 não reseta entre exportações (#7234), e o mesmo em todos: uma exportação (ou um projeto novo) começa
      do repouso. O mixer já fazia isso nos canais; agora cada plugin faz no seu `reset()`, na thread de áudio, quando
      a época do mixer muda (`PluginInstance::process`). O teste: tocar alto, pedir o repouso, e o período seguinte
      sai em silêncio absoluto — falhavam 5 instrumentos e 12 efeitos (LB302, OpulenZ, Sf2, AVL, Zyn; BassBooster,
      Bitcrush, CrossoverEQ, Delay, DualFilter, Flanger, FrequencyShifter, GranularPitchShifter, LOMM,
      MultitapEcho, ReverbSC, SlewDistortion). E exportar duas vezes dá o mesmo bit a bit (`InstrumentFixesTest`).
      Achados no caminho: o OpulenZ apagava o emulador antes de tirar a play handle do motor (e o trocava fora da
      trava); o reset do chip OPL não zerava LFOs, realimentação e fase (commit local no submódulo adplug); o
      FluidSynth guarda parte do passado nas vozes, então o Sf2Player e o AVL trocam o sintetizador por um novo na
      thread principal (`Mixer::processingStateDiscarding`); o Zyn entrega o bloco calculado na vez anterior; o
      `StereoDelay` escrevia fora do buffer numa taxa menor que a de criação. Nenhum `reset()` aloca na thread de
      áudio (o `AudioThreadAllocationTest` pede o repouso no meio);
    - [x] notas que não tocavam: o motor descarta notas novas quando a carga medida passa de 99%, e isso valia também
      renderizando fora de tempo real (só a exportação era exceção): depois de um período lento, as primeiras notas
      do projeto seguinte sumiam. Agora só com o dispositivo tocando em tempo real. E uma handle descartada por
      outro caminho (projeto novo, notas silenciadas) ficava na lista "a remover": agora todo descarte passa por
      `AudioEngine::disposePlayHandle()`, que a tira de lá (senão uma handle nova no mesmo endereço seria removida);
    - [x] volumes padrão muito desiguais (#1207, #7852): medidos (o momento mais alto de uma nota, janelas de 50
      ms), iam de -32 dB (ZynAddSubFX) a -2 dB (Monstro). Agora todos entre -10 e -16 dB, só nos instrumentos
      novos (projetos e presets guardam os seus valores): TripleOscillator 33 → 20 por oscilador, Monstro 33 → 10,
      Watsyn 100 → 32, BitInvader com a senoide a 0,35, Xpressive com as expressões pela metade, o ZynAddSubFX com o
      volume mestre no topo, e o OpulenZ ganhou um botão VOL (em dB; +16 nos novos, 0 nos projetos de antes, desenhado
      na arte com as letras dela). `InstrumentFixesTest` verifica;
    - [x] (no caminho) carregar um soundfont segurava a trava do motor enquanto o FluidSynth trocava as fontes:
      sob carga, 7 a 10 ms e um período pulado. Agora um sintetizador novo é montado sem trava e só os ponteiros
      trocam (0,00 ms medidos);
    - o host LV2 não abre a interface nativa dos plugins. **Parado, esperando o usuário (18/11):**
      - precisa do `suil` (no Homebrew, não instalado): instalar é baixar um pacote, o que pede a sua aprovação;
      - os LV2 de terceiros rodam isolados em outro processo, e muitas interfaces pedem `instance-access` (o plugin
        no mesmo processo; a do Carla, a única com interface instalada aqui, pede). Então a interface teria de abrir
        no processo isolado, com o suil lá. Decidir: instalar o suil e abrir as interfaces no processo isolado
        (recomendado), ou rodar no processo do programa os LV2 cuja interface peça isso.
    - [x] aliasing (#6098), medido como a potência fora dos harmônicos de uma nota aguda (FFT de 2^15 amostras,
      janela Blackman-Harris; o chão da medida é -88 dB):
      - BitInvader e Watsyn tocam as ondas desenhadas por tabelas sem harmônicos acima do corte (`DrawnWave`: os
        harmônicos da onda como ela toca, calculados dos pontos, parados em degraus ou ligados por retas; uma
        tabela a cada dois semitons, lida com cúbica): de -15/-12 dB para -88 dB. Botão "Anti-aliasing" nos dois,
        ligado nos novos e desligado nos projetos de antes (tocam como tocavam);
      - Organic usa as tabelas do Oscillator, como o TripleOscillator (sempre: as fases aleatórias já tornavam cada
        render diferente);
      - TripleOscillator com PM e FM: a tabela era a da altura da nota, e a modulação empurra a onda acima dela
        (-10 dB). Agora a da altura mais alta que a onda alcança no período, mais as bandas laterais (regra de
        Carson): -60 dB ou menos. Uma FM muito funda passa da metade da taxa até numa senoide, e isso nenhuma tabela
        tira (precisaria de oversampling);
      - Monstro: o oscilador 1 (pulso) só punha um 0 em cada degrau (-22 dB a 1 kHz, -14 a 7 kHz); agora são duas
        serras com banda limitada (as dos osciladores 2 e 3): -73/-67 dB, no mesmo volume. Também nos projetos de
        antes, porque substitui o anti-aliasing que ele já tentava ter;
      - Os outros instrumentos citados ficam como estão: o LB302 já tem as suas ondas com banda limitada (as
        variantes "BL"), o Kicker são senoides graves, SID, Nescaline e Sfxr imitam chips cujo caráter é a onda
        crua, e o AFP já tem a interpolação sinc na escolha de interpolação.
    - [x] modo mono, legato e portamento em qualquer instrumento (#6516): grupo "MONO" na aba de funções do
      instrumento, ao lado do empilhamento (ligado ou não, "Legato" ou recomeçar, e o deslize em ms; a aba cabe na
      altura de antes: mais alta, a janela crescia e a arte de cada instrumento se repetia embaixo). As notas seguradas (teclas
      de um dispositivo ou do piano, notas dos clipes) ficam na ordem em que vieram; uma voz toca a última. Legato: a
      voz vai para a nova tecla sem recomeçar; reiniciar: a voz acaba e outra começa. Quando a nota tocando acaba e
      outra ainda está segurada, a voz volta para ela. Notas que se encostam no piano roll (uma começa onde a outra
      acaba) também deslizam. O deslize vale quando as notas se sobrepõem. Instrumentos que recebem MIDI (Zyn, VST)
      recebem nota nova ligada e a velha desligada; o Sf2Player segue o legato e o deslize afinando as vozes. Fica
      salvo na faixa; projetos de antes ficam com ele desligado. Segurança entre threads: as notas dos clipes são
      tratadas na parte da música do período, as teclas sob a trava de mudanças do motor, e a voz se tira do modo
      mono como primeira coisa ao ser destruída. `MonoModeTest` (frequência e vozes contadas a cada momento) e o
      `AudioThreadAllocationTest` com mono.
    - [x] Sf2Player e GIG Player:
      - ADSR próprio (#3233): os dois tocam um fluxo só, que os envelopes da faixa não alcançam, e a aba ENV não fazia
        nada. Agora o envelope de volume da aba ENV vira o envelope de cada voz (no Sf2, os geradores de envelope do
        FluidSynth; no GIG, o envelope dele no lugar do do arquivo), quando a quantidade não é 0. A aba mostra o
        envelope de volume e acinzenta o LFO, os outros alvos e o filtro, com uma explicação;
      - arrastar um `.sf2`/`.sf3` (do navegador ou do gerenciador de arquivos) para cima do Sf2Player troca o
        soundfont (#7480); os campos de arquivo e patch mostram o nome do arquivo e o caminho inteiro e o
        banco/patch na dica (#7482);
      - GIG (#3825): lia os samples do arquivo na thread de áudio, um bloco por período para cada sample tocando (o
        processador inteiro com kits grandes), e alocava a cada nota (31 alocações num teste curto). Agora os samples
        do instrumento ficam na memória ao escolhê-lo (como o FluidSynth faz com o soundfont), as vozes são feitas
        antes com os seus reamostradores, e nada é alocado na thread de áudio (`AudioThreadAllocationTest` com um
        `.gig` feito pelo `wav2gig`). No caminho: uma voz que acabava apagava o que as outras já tinham mixado no
        período, e a iteração das regiões guardava estado no instrumento (compartilhado entre threads);
      - GIG: a dimensão do pedal de sustain (#4517) escolhe os samples gravados com o pedal abaixado; e a altura
        segue a nota no meio dela (legato e deslize do modo mono).
    - [x] (no caminho, um erro meu de 18/11) o ZynAddSubFX no processo próprio (como roda com a janela dele aberta)
      parava de vez com as mensagens novas de volume e de repouso: o laço de mensagens já segura o mutex do mestre
      (um `std::mutex`) e elas o pegavam de novo. Os testes usam o Zyn dentro do programa, que só trava uma vez: o
      `ZynRemoteTest` agora roda o processo de verdade e falha sem a correção.
    - [x] ZynAddSubFX: o knob de portamento (#3936) mandava o CC65, que só liga ou desliga (no 64): de 0 a 63 não
      fazia nada e o tempo não se ajustava. Agora 0 desliga e acima dele é o tempo (a escala do próprio Zyn, de 0,02
      a 2 s); projetos de antes ficam como estavam (abaixo de 64 desligado; acima, ligado no tempo padrão);
    - [x] ZynAddSubFX no processo próprio: iniciava os motores de áudio do Zyn, e o "NULL" renderizava, numa thread
      própria e em tempo real, um mestre global que ninguém ouvia, disputando com o do plugin as variáveis globais do
      synth (o TSan achou). Agora os motores são criados sem iniciar. Não mudava o som nem a CPU de forma mensurável;
      o `ZynRemoteTest` também confere que a mesma nota sai igual, bit a bit, em outro processo.
    - ZynAddSubFX ainda por fazer: o corte de áudio ao mostrar a janela (#1825) precisa ser medido com a janela de
      verdade (a interface só pega o mutex do mestre nos callbacks, e o VU usa outro); a automação de mais controles
      (#4263) pede um jeito de expor os parâmetros do Zyn como modelos.
    - [x] FreeBoy (#6556, #8043): medido, não há offset DC com o padrão (o filtro "Bass" do emulador o tira); os
      botões de saída diziam o lado errado (o de "SO2 (Esquerda)" saía à direita: o comportamento era o do Game Boy,
      SO1 = direita) e os dois volumes de saída agiam como um só (o emulador tem um volume): rótulos corrigidos e
      cada lado no seu volume. O ruído ao trocar de nota (#3824) e a CPU alta (#6144) não se reproduziram aqui;
    - [x] AudioFileProcessor: microfades (#5855, botão novo, ligado nos novos: fade de 2 ms no começo e no fim e
      crossfade onde o loop volta, a partir do que vem antes do início do loop), o diálogo de abrir aceita o preset
      .xpf que ele salva (#7492), e o botão do ponto de loop some com o loop desligado (#7819).
    - [x] itens menores: a automação do RES do LB302 chega a 1,25 (o editor arredondava os níveis e prendia em 1,
      #7980); a largura de pulso mínima do Monstro tocava a 25% (os limites misturavam unidades) e agora vai até um
      quadro de largura (#3046); o Patman diz por que um patch não carregou (#8163) e aceita o `.pat` arrastado do
      navegador (#8065); os presets do Xpressive com onda própria apontavam para um caminho que só existia na máquina
      de quem os fez, e o caminho "data:/" das amostras de fábrica não chegava ao FluidSynth e ao STK (que abrem o
      arquivo direto): o novo Sf2Player nem carregava o soundfont padrão (`PathUtil::toAbsolute` resolve os prefixos
      de busca do Qt). Todos os presets de fábrica carregam sem erro (`InstrumentFixesTest`);
    - [x] Mallets (#7573): os presets de guia de onda (barra uniforme, barra afinada, vidro, tigela tibetana) davam um
      zumbido baixo: a frequência da voz era posta a cada período, e o STK limpa as linhas de atraso nisso. E com
      arco eram mudos (-80 dB): a integração do arco era 0 (um modelo nunca feito), o controle de "movimento" passava
      a voz a seguir um arco movido por controlador, e a velocidade era sobrescrita. Medido no STK sozinho (55 Hz a
      3,5 kHz, 15 s por nota): com integração 0,75, a pressão no trecho em que as barras pegam e a velocidade da nota
      vezes a do botão, todos cantam firmes e no nível dos outros presets. Um teste exige som tonal e audível em
      cada preset, sustentado com arco. Achado pelo TSan: o `BandedWG` do STK não inicializa a velocidade integrada
      do arco (com integração 0 ela era zerada a cada amostra; agora ficava o lixo da memória, e a voz dava infinito
      desde a primeira amostra); a voz nasce em repouso. O `ModalBar` e o `TubeBell` não dependem da memória
      (testados com ela cheia de lixo).
    - [x] SlicerT: o BPM era o comprimento do sample tomado como um compasso (#8102); agora vem dos ataques (o
      fluxo espectral, sua autocorrelação e um pente nos tempos e entre eles, depois a oitava mais provável), e um
      loop de compassos inteiros fica com o tempo exato. Acerta os 13 beats que vêm com o programa (o jungle de 168
      dá 84, o mesmo pulso) e loops sintéticos de 87 a 174 BPM com e sem sobra no fim. As fatias podem tocar em
      loop ou vaivém enquanto a nota é segurada (#7364; `Sample::play` com limites próprios, sem a nota guardar
      uma cópia do sample, que soltaria memória na thread de áudio), com o fade-out como release. No caminho: o
      fade do fim ignorava a taxa do sample e dividia por zero com fade 0; a primeira fatia de um sample que começa
      em silêncio era apagada; o plano do FFTW nunca era destruído; o ajuste das fatias não era salvo; e a onda de
      um sample aberto com o projeto não aparecia.
- [ ] **Bibliotecas de sons para os players:**
  - GIG player: uma biblioteca de arquivos `.gig` dentro dele, para escolher, mais o botão de carregar um próprio;
  - o mesmo para `.sf2` (Sf2Player) e `.pat` (Patman). Hoje não há nem uma biblioteca mínima.
  - Só arquivos com licença que permita distribuir com um programa GPL. Conferir a licença de cada um e
    registrá-la (créditos).
  - Pesquisa feita: `docs/pesquisas/bibliotecas-de-sons.md`.
    - SF2: GeneralUser GS e instrumentos CC0 do FreePats.
    - GIG: não existe um conjunto `.gig` pronto com licença livre confirmada. Vamos gerar os nossos com o
      `wav2gig` da libgig, a partir de amostras CC0 (VSCO-2 CE, VCSL, FreePats).
    - PAT: o conjunto GUS do FreePats (GPL-2+).
    - O `data/samples/CMakeLists.txt` ainda não instala `.gig` nem `.pat`.
- [x] **Kicker: ataque padrão 0 dá um clique** a cada nota. O padrão passa a ser um ataque mínimo, nunca 0.
  Conferir os outros instrumentos e efeitos com o mesmo problema.
  - O Kicker ganhou o botão **Attack** (0,1 a 20 ms, padrão 1 ms), no envelope de volume, desenhado na arte como os
    outros. O "Click" continua (ele começa a senoide fora do zero); o ataque sobe do silêncio depois dele. Projetos
    de antes (sem ataque, que era 0) abrem com o mínimo, 0,1 ms: o mais perto de como soavam, sem o salto.
  - Varredura: o `BuiltinPluginsTest` toca uma nota em cada instrumento interno, como vem, e exige que nos dois
    primeiros quadros ela não passe de um quarto do seu pico. Tinham o mesmo salto: Organic (fases aleatórias),
    Monstro, sfxr, SID (o original pulava para 1,3 no segundo quadro), Vibed e Xpressive. Eles passam a usar o
    `applyFadeIn` que o LMMS já tinha (só o TripleOscillator usava): até ~2 ms, mais curto quanto mais aguda a
    nota. Ele agora dura o mesmo tempo em qualquer taxa (era contado em quadros de 44,1 kHz) e trata os dois
    canais igual (o direito ficava sem o último quadro do fade).
  - Achado no caminho: **o Mallets ficava mudo** sem arquivo de configuração (a primeira execução, os testes): a
    pasta do STK só era procurada ao ler o arquivo. Agora as pastas padrão (VST, LADSPA, STK) são resolvidas também
    na criação das configurações. O teste do Sf2Player agora toca com um soundfont de verdade.
  - O ASan, com o Mallets tocando de verdade e outros sorteios, achou mais dois: o filtro de formantes do
    envelope (o "Vogal") lia além da tabela de vogais acima de 17,5 kHz (a frequência ia até 20 kHz; a tabela, até
    17,5), e o GranularPitchShifter lia antes do começo do anel quando um grão grande, agudo e com spray pedia mais
    do que o anel guarda.
  - Efeitos: os ataques com 0 no mínimo (LOMM, Peak Controller) são tempos de detecção do nível, não o começo de
    um som: não dão clique.
- [x] **Configurações que só valem depois de reiniciar** (o backend de áudio, por exemplo): ao sair da tela de
  configurações, nada muda até reiniciar à mão. Feito com o design 08 (item abaixo): cada opção vale assim que
  muda, sem "Salvar".
  - O áudio (sistema, dispositivo, taxa, buffer) e o MIDI abrem de novo sozinhos, 0,4 s depois da última mudança
    (arrastar o buffer abre o dispositivo uma vez); o que ele disser (outro dispositivo, outra taxa) aparece na
    própria seção, sem caixa de mensagem.
  - Passaram a valer na hora: dicas, cabeçalhos compactos, explorador à direita, nomes das notas no Piano Roll,
    marcas dos faders, cursor do AudioFileProcessor, modo de seguir o cursor (os editores abertos também),
    limpeza da mixagem e "efeitos continuam sem entrada" (cada buffer adota a mudança na própria thread de
    áudio). O meio comum: `ConfigManager::valueChanged`, que agora também avisa quando a chave é nova.
  - Pedem reinício de verdade: idioma, tema, pasta de trabalho e pastas LADSPA (lidas ao abrir). A faixa
    "Reiniciar agora" fecha o programa pelo caminho normal (pergunta se salva o projeto) e o `main()` o abre de
    novo com o projeto, depois de tudo fechado.
  - Aplicar na hora o que der.
  - O que precisar reiniciar avisa e oferece reiniciar.
  - Levantar a lista de opções e o que cada uma precisa.
  - Levantamento do código: `docs/pesquisas/levantamentos-codigo.md`.
- [x] **Categorias dos instrumentos:**
  - o Vibed sai de "modelagem física" e vai para "sintetizador";
  - subcategorias de sintetizador, porque são muitos.
  - Feito: os sintetizadores em Subtrativos (TripleOscillator, LB302, Monstro), Tabela de ondas (Watsyn,
    BitInvader), Aditivos (Organic, ZynAddSubFX), Cordas (Vibed) e Fórmulas (Xpressive). O navegador mostra a
    subcategoria como um segundo nível dentro da categoria (a busca também a esconde quando nada dela é achado).
    O mesmo segundo nível serve às categorias de efeitos (o Guitarix). `PluginCategoriesTest`.
- [x] **Plugin de baterias eletrônicas:** um instrumento visual como o AVL Drums, que:
  - usa as amostras de baterias eletrônicas que já temos;
  - mapeia cada peça numa nota MIDI;
  - troca de bateria com a foto de cada uma, como no AVL.
  - Feito: "Drum Machines" (categoria Bateria), com 9 kits, as máquinas do DrumSynth que vêm com o programa
    (CR-78, CR-8000, TR-606, TR-77, TR-808, TR-909, LinnDrum) e os kits Electro e R&B: 110 peças, cada uma na sua
    nota do mapa de bateria General MIDI (as variações sem nota lá, a partir da 82). As peças são sintetizadas ao
    escolher o kit (thread principal) e tocadas da memória com vozes feitas antes (zero alocações na thread de
    áudio); o chimbal fechado abafa o aberto, como nas máquinas; meio milissegundo de entrada (o DrumSynth começa
    num degrau, que estala). A tela desenha a máquina de cada kit (o corpo, o painel, as cores dela) com um pad
    por peça, que acende quando a peça toca e a toca quando clicado. `DrumMachinesTest`: os 9 kits carregam e
    tocam, e o abafamento (o teste falha sem ele).
  - Cada kit mostra a foto real da máquina (a pedido do usuário, 18/11), do Wikimedia Commons com licença livre
    e crédito na própria foto e em `plugins/DrumMachines/photos/CREDITS.md` e `doc/AUTHORS`: TR-808, TR-909,
    TR-606, CR-8000, TR-77, LinnDrum; o kit Electro, a Oberheim DMX, e o R&B, a E-mu SP-1200. A CR-78 não tem foto
    com licença livre no Commons: o painel dela continua desenhado. A foto preenche o plugin (em alta resolução,
    até 1920 px), os pads numa faixa sobre a parte de baixo dela.
  - Máquinas novas, uma por uma (pedido do usuário, 18/11; as decisões dele: DMX e TR-707 emuladas no DrumSynth,
    porque os sons delas são gravações PCM dos fabricantes e não podem ser distribuídos; MPC60 e SP-1200, que são
    samplers sem sons próprios, como kits no estilo delas feitos de sons livres; a CR-78 com gravações dela):
    - CR-78: agora com a própria máquina gravada (Freesound: o pacote CC0 do wikter, e o pandeiro, CC BY 4.0, do
      Bjornmossa; as prévias públicas em alta qualidade, sem login, convertidas para FLAC), em
      `data/samples/drummachines/cr78`. O plugin toca também gravações, convertidas para a taxa do motor ao
      carregar.
  - Corrigido (pedido do usuário): o plugin não se redesenhava ao trocar de máquina. Causa: `Plugin::createView`
    chamava `setModel` de novo numa tela que já tinha o modelo, e o `ModelView::setModel` desconecta os sinais do
    modelo para a tela: o que o construtor da tela ligara ao instrumento se perdia (o mesmo acontecia com o
    `uiClosed` do Carla). Agora não reatribui o mesmo modelo, e a tela liga o sinal no `modelChanged()`.
- [ ] **Guitarix integrado de verdade:** as peças dele viram efeitos nativos do LinuxMusic, compilados aqui
  dentro: cabeçotes, amplificadores, gabinetes, pedais e efeitos. Não é hospedar o programa nem os LV2 dele.
  - Cada peça é um efeito interno, no processo e no caminho do render, como os outros nativos
    ([[builtin-plugins-in-process]]).
  - No navegador de efeitos, o Guitarix é uma categoria própria, com as peças como subplugins dela (por exemplo:
    Guitarix → Amplificadores, Cabeçotes, Gabinetes, Pedais...). Isso usa o mesmo segundo nível de categoria das
    subcategorias de sintetizador.
  - **Decisão do usuário (17/11):** o mesmo código e os mesmos nomes do Guitarix: unidades, parâmetros e
    presets, sem renomear. Fica registrado que alguns nomes são marcas de terceiros, como o próprio Guitarix já usa.
  - **Som igual, visual nosso (usuário, 17/11, substitui o que estava aqui):**
    - O núcleo de cada efeito é o mesmo código do Guitarix, adaptado só no necessário para rodar aqui: o C++
      gerado pelo Faust, os zita, as tabelas e os IRs. Ele tem que soar igual.
    - Conferir com uma comparação de áudio contra o Guitarix (os LV2 dele), unidade por unidade.
    - **O visual é o do Guitarix, desenhado aqui em Qt (usuário, 17/11):** a mesma aparência de cada unidade,
      desenhada pelos nossos widgets Qt, não pelo xputty nem pelo cairo.
      - Vêm do Guitarix: as imagens (PNG dos painéis e dos knobs, em `trunk/src/LV2/xputty/resources` e nos
        `gx_*.lv2`), os botões, as cores e a disposição.
      - Cada unidade cabe no rack de efeitos.
  - **Licença (usuário, 17/11):** o Guitarix é GPL, e o LinuxMusic é GPL-2 ou posterior. Pode vir tudo, inclusive
    as partes GPL-3 (zita-resampler e zita-convolver). Com elas, o programa distribuído vale como GPL-3, o que a
    licença permite. Obrigatório: citar o Guitarix e os autores nos créditos (Sobre e arquivo de créditos) e manter
    os avisos de copyright nos arquivos portados.
  - Pesquisa feita: `docs/pesquisas/guitarix.md`.
    - O código próprio do Guitarix é GPL-2+; o `zita-convolver` e o `zita-resampler` são GPL-3 e o `abgate` é
      LGPL-3. Pela decisão acima, tudo pode vir, o mesmo código.
    - 44 unidades sobreamostram com o zita-resampler: vem junto, como no Guitarix.
    - O C++ gerado pelo Faust já vem pronto no repositório dele: não precisamos do Faust.
    - Inventário: 18 pré-amplificadores, 6 amplificadores de potência, 27 tonestacks, 17 gabinetes e cerca de 80
      pedais.
    - Piloto sugerido: o Tube Screamer.
  - Antes de começar:
    - levantar como o DSP dele é gerado (Faust) e o que isso exige no nosso build;
    - listar todas as peças e o que já temos de parecido (AmpAndIr/NAM, mixer analógico);
    - definir a interface de cada uma, que deve caber no rack de efeitos (item acima).
  - Andamento (19/11): **fase 0 e piloto (TS9) feitos.** O efeito `guitarix` (categoria própria no navegador)
    tem as unidades do Guitarix como subplugins, pelos ids e nomes dele. O código de cada unidade é o gerado do
    Guitarix, sem mudança (em `plugins/Guitarix/gx/`), compilado contra os cabeçalhos dele (`gx_plugin.h`,
    `gx_faust_support.h`...) e o zita-resampler que ele usa para rodar a 96 kHz (GPL-3, aceito pelo usuário): soa
    como no Guitarix. Cada unidade em mono roda em duas instâncias (esquerda e direita). Os parâmetros viram modelos
    a partir do `register_par` da própria unidade (salvos pelos ids do Guitarix). As unidades são refeitas na thread
    principal quando a taxa muda e antes de um repouso (o reamostrador guarda o passado, e configurá-lo aloca); na
    thread de áudio só processam: zero alocações (`AudioThreadAllocationTest`), e o `BuiltinPluginsTest` passa
    (extremos dos parâmetros, repouso). `GuitarixTest`: o TS9 distorce um seno, mais com mais drive, e o level é o
    nível de saída em dB. Créditos no `doc/AUTHORS`.
  - As 41 unidades do `pluginlib` do Guitarix (overdrives, distorções, fuzzes, multibanda, flanger, Zita Rev1...),
    na ordem do rack dele, com os nomes e ids dele (o `pluginlib.h` dele declara as fábricas). O `GuitarixTest` passa
    por cada uma: como vem, tudo no mínimo e no máximo (som finito), e o repouso depois de dois passados diferentes dá
    o mesmo som (falha em 41 delas se o repouso não refaz a unidade). Zero alocações em cada uma. Algumas (as de
    tabelas de diodo assimétricas) dão uma constante pequena no silêncio, como no Guitarix, que só bloqueia o DC no
    começo da cadeia dele.
  - A aparência do Guitarix: cada unidade é desenhada como no rack dele, com o estilo padrão dele ("Guitarix"): o
    painel (as imagens `background1`/`background2` dele, o bisel e a sombra), os puxadores dos lados, os parafusos
    nos cantos, o título, e os controles dele desenhados como a libgxw desenha (GxKnob: a imagem do knob, o anel
    tracejado com o arco do valor em laranja e o indicador; GxSelector; GxFastMeter;
    GxValueDisplay; GxHSlider, GxVSlider; GxSwitch; GxPaintBox), na fonte dele (Roboto Condensed). A disposição é a dele: o glade de cada unidade (a
    parte "rackbox", a unidade aberta) é lido e montado com widgets Qt, com as regras de caixa do GTK (expand, fill,
    pack end, padding, grids homogêneas, frames, alinhamentos, abas `tab_rack`), e os rótulos ligados a um knob
    recebem o nome do parâmetro, como o GxRegler faz. Os 321 `var_id` dos layouts das 41 unidades batem com os
    parâmetros delas. Os knobs funcionam como os do LinuxMusic (arrastar, roda, automação, valor ao passar o mouse).
  - Os medidores das unidades (parâmetros de saída do Guitarix, "O") agora são saídas: a unidade escreve, a tela
    lê; não viram knob, automação nem são salvos no projeto (antes eram knobs e o processamento sobrescrevia o que
    a unidade media). Os parâmetros que o Guitarix não salva ("s") também não são salvos; um parâmetro sem nome leva
    o fim do id com inicial maiúscula, como no Guitarix. `GuitarixTest::outputsAreNotControls` confere.
  - As 44 unidades Faust do rack do Guitarix (`gx_effects` e o módulo de amp do `gx_amps`): ganho, filtros, EQs
    (seletiva, de picos, gráfica, Baxandall, 3 bandas), distorções, compressor, expander, ecos e delays (mono e
    estéreo, digital, duck), reverbs (Freeverb, Stereoverb, Dattorro), tremolo, phaser, chorus, flanger, moog, ring
    modulator, panorama, bass enhancer, fuzz face, transistor buffer... São os arquivos que o Guitarix gera
    (`src/faust-generated`), como estão, cada um compilado dentro do namespace dele em `gx_engine`, como o
    `gx_faust_plugins.cpp` do Guitarix faz (o CMake gera um arquivo por unidade). Eram 85 unidades, na ordem do
    rack dele (as mono, depois as estéreo). As 85 passam pelo `GuitarixTest` (extremos, repouso) e pelo teste de
    alocações (zero). Os glades delas (inline, ou em arquivo do `builder` dele, embutidos aqui) usam também
    GxPaintBox, GxSwitch, GxVSlider e GtkEventBox; os 247 `var_id` batem com os parâmetros. Uma unidade sem UI no
    Guitarix (o Transistor Buffer) mostra só o título, como lá. Nenhuma unidade usa só o construtor em pilha do
    Guitarix, então ele não foi feito.
  - Corrigido: os medidores mostram o valor como o GxFastMeter mostra: na faixa do parâmetro (os das unidades, de
    passo 0), ou, num parâmetro com passo, como potência em dB pelo `log_meter`. (Antes todos iam pelo `log_meter`:
    o do compressor, 0 a 40, aparecia aceso no repouso.)
  - O Vibe (mono e estéreo), o abGate e o ReverseDelay (código escrito à mão do Guitarix, com os glades em arquivo
    deles e o GxPortDisplay do buffer): 88 unidades. Dois defeitos do código do Vibe, resolvidos sem mudá-lo: (1) o
    LFO dele guarda o estado no arquivo (o Guitarix roda uma instância de cada unidade); aqui cada instância tem a
    sua cópia, posta no arquivo em volta do processamento, sob uma trava (outros Vibes, em outras threads). Sem isso
    as duas instâncias da unidade mono (L e R) avançavam o mesmo LFO duas vezes por bloco: velocidade dobrada e os
    canais defasados (`GuitarixTest::vibeChannelsAlike` falha assim). (2) Membros dele (o estado dos filtros) não
    são iniciados nem pelo construtor nem pelo `init`: o Vibe é feito sobre memória zerada.
  - O repouso agora refaz as unidades pela fábrica delas (nem todas zeram todo o estado ao serem reiniciadas), e a
    iniciação de uma unidade segura a trava de mudanças do motor: algumas preenchem tabelas do arquivo (as mesmas
    que outros efeitos leem processando).
  - Corrigido: os rótulos saíam cinza-claro (a folha de estilo do programa passava por cima da cor); agora são
    `#222222` no verde e `#aaaaaa` nos inversos, como no Guitarix.
  - As unidades que são uma escolha entre outras (o `ModuleSelectorFromList` do motor dele): Crybaby (crybaby e
    autowah), Wah (10 modelos), Tonestack (27) e PowerAmp (6 estágios de potência), com os glades em arquivo
    deles: 92 unidades. Um `PluginDef` nosso faz o que o seletor dele faz (registra o seletor e os parâmetros de
    todos os módulos, processa o escolhido, zera o estado do módulo ao escolhê-lo, como o motor dele zera a cadeia).
    `GuitarixTest::selectorsChooseTheirModules` passa por cada módulo de cada um.
  - Corrigido: um parâmetro com lista de valores (seletor) tem a faixa da lista, como no Guitarix (inteiro: 0 a
    n-1; float: do mínimo, um por valor); antes ficava com a faixa registrada (0 a 0 nos seletores do motor).
  - As caixas e os preamps por convolução: Cabinet (17 caixas) e Amp Impulse (10 preamps), com as respostas ao
    impulso do Guitarix (`gx_cabinet_data.cc`, `gx_preamp_data.cc`) e o zita-convolver que ele usa: 94 unidades.
    Fazem o que o `CabinetConvolver`/`PreampConvolver` dele faz (sobe a ~96 kHz, convolui, desce, filtro de graves,
    agudos e nível). O conversor sobre o zita-convolver é nosso (o `GxSimpleConvolver` dele depende do motor dele
    inteiro): um nível só de partição, do tamanho do bloco, que o zita-convolver faz todo na thread que chama, sem
    threads próprias para esperar (a exportação sai igual em qualquer velocidade). Trocar a caixa refaz a resposta
    na thread principal (o Guitarix verifica a cada 200 ms na thread da interface), só quando um parâmetro mudou e
    com a trava de mudanças. Os parâmetros são aplicados antes de iniciar a unidade, e o tamanho do período
    reinicia a unidade. O teste dos seletores troca as 17 caixas e as 10 respostas de preamp.
  - O cabeçote do amp ("Amp"): a cadeia fixa que o motor do Guitarix põe em volta do rack, como uma unidade: noise
    gate, noise shaper, os 19 amps do seletor `tube.select` (o `ampstack`), soft clip, bass booster, saída do amp,
    presença (o `ContrastConvolver` dele, por convolução), o reverb de feedback, o master, o balanço, o anti-fizz,
    os medidores de saída (o `MaxLevel`: pico desde a última leitura) e o limitador; os liga/desliga começam como no
    preset de fábrica dele. A interface é a dele: o `ampbox.glade` e a faixa de detalhes do `mainpanel.glade`,
    pintadas como ele pinta (faixa escura no meio do cabeçote), e, sem amp ("---"), só o knob de ganho, como na
    janela dele. 95 unidades. Os amps compilam num arquivo só, como no Guitarix (as tabelas das válvulas são
    definidas no `valve.h`). (No caminho: duas classes nossas derivadas do `PluginDef` não o iniciavam, e o efeito
    chamava um ponteiro de função com lixo; varridas as quatro.)
  - No navegador de efeitos e no diálogo de escolha, as unidades ficam em subcategorias dentro de "Guitarix", as
    categorias do próprio Guitarix (controle de tom, distorção, diversos, eco/delay, efeitos de guitarra, fuzz,
    modulação, reverb). As sub-plugins de um plugin ganharam um grupo (`Key::group()`), que esses dois mostram como
    um nível próprio ("Guitarix › Distorção" no diálogo).
  - O Detune (o `smbPitchShift` do Guitarix, pitch shifter por FFT, de Stephan Bernsee): 96 unidades. O código é o
    dele, portado (o que dependia do motor dele passou a ser da unidade; a troca de latência refaz os buffers na
    thread principal; os buffers de `new[]` agora saem com `delete[]`). `GuitarixTest::detuneShifts` confere que 7
    semitons levam o seno de 220 Hz para a quinta.
  - As unidades recebem sempre blocos do mesmo tamanho, como no Guitarix (o período do JACK dele): a maior potência
    de dois, até 256, que divide o período. O Detune só processa blocos do tamanho com que foi montado.
  - Falta do Guitarix:
    - o conversor com arquivo de IR (`jconv`, mono e estéreo): a escolha e o ajuste da IR são o editor dele
      (`iredit`: forma de onda, linha de ganho, offset, comprimento), uma janela própria a portar;
    - os amps neurais (NAM, RTNeural) e o JCM800 Preamp (`jcm800pre`): precisam do Eigen (e do NAM Core e do
      RTNeural), que não estão instalados; baixar pede a sua aprovação;
    - não vêm, por serem ferramentas do programa Guitarix e não efeitos do rack (o LinuxMusic tem as suas): afinador,
      osciloscópio, looper, gravador, sequenciador de bateria, metrônomo e o medidor de VU como unidade.
- [ ] **Controle das faixas:** redesenhar no estilo do REAPER. O desenho atual ainda está ruim.
- [x] **Transporte novo (design aprovado em 17/11):** `docs/design-sugestoes/03-transporte.png`.
  - Botões: para o começo, parar, tocar, gravar (o menu dele grava MIDI no clipe do Piano Roll), ciclo.
  - Dois relógios, o grande e o pequeno, cada um no formato escolhido com um clique (compassos e tempos, minutos
    e segundos, amostras, timecode SMPTE a 30 quadros), guardado nas configurações. A borda do grande fica verde
    tocando, vermelha gravando e amarela na contagem.
  - O estado numa palavra colorida: PARADO, PAUSADO, TOCANDO, GRAVANDO, CONTAGEM/PRÉ-ROLL com os compassos que
    faltam.
  - O andamento no lugar: arrastar para cima ou para baixo, rodinha (Shift: de 10 em 10), duplo clique para
    digitar; um clique oferece digitar, marcar batendo (Tap: a média das últimas batidas; para sozinho dois
    segundos depois) e mudar o andamento a partir do cursor no mapa de tempo.
  - A fórmula de compasso num botão "4/4" (abre os dois números), o metrônomo e o pré-roll num botão que mostra
    os compassos (com a contagem no menu dele).
  - `TransportBarTest`: os formatos, o formato guardado, a rodinha, arrastar e o Tap.
  - Um visor grande de posição e um menor de tempo, clicáveis para trocar o formato.
  - Estado escrito e colorido (parado, tocando, gravando, contagem).
  - Andamento editável no lugar, com Tap.
- [x] **Configurações novas (design aprovado em 17/11):** `docs/design-sugestoes/08-configuracoes.png`.
  - Seções Geral, Áudio, MIDI, Gravação, Plugins, Pastas, Aparência, Teclado e atalhos, Avançado; a busca acha
    pelo nome, pela linha de ajuda e pelas escolhas, sem diferenciar maiúsculas nem acentos.
  - A latência ao lado do buffer: a que o dispositivo mede, quando ele informa; senão, a do buffer.
  - Em janela estreita, a linha de ajuda vai para baixo do controle. `SetupDialogTest` cobre seções, busca,
    gravar na hora, avisos, o motor seguindo e o dispositivo abrindo uma vez só.
  - Seções numa lista lateral com busca.
  - Cada opção diz se aplica na hora ou pede reinício.
  - A latência real ao lado do buffer.
  - Faixa "Reiniciar agora" para o que pede reinício.
  - Junta-se ao item "Configurações que só valem depois de reiniciar".
- As outras sete telas de `docs/design-sugestoes` esperam a revisão do usuário: não implementar antes.

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

## Observado (18/11)

- [ ] O `ArpeggioSyncTest` falhou uma vez numa rodada completa do ctest com os builds de sanitizador compilando ao
  mesmo tempo (carga muito alta), e passou ao repetir; 72 execuções em paralelo depois não reproduziram. O log da
  falha se perdeu (o ctest o sobrescreveu ao repetir): o log de cada rodada passa a ser guardado para pegar o
  detalhe se voltar.

## Achado na fase 10 (17/11)

- [ ] Entrada MIDI × porta apagada, em todos os drivers (ALSA, JACK, CoreMIDI): a thread do MIDI entrega o evento à
  porta sem nada que impeça a porta de ser apagada ao mesmo tempo. Segurar uma trava durante a entrega pode travar
  (a nota desligada pega a trava do motor; limpar o projeto apaga portas segurando essa trava). Precisa de um desenho
  comum aos drivers (por exemplo, a entrada passar por uma fila sem trava lida no início do período, como o JACK já
  faz no callback) e do teste numa máquina Linux. Ver `docs/plano-refatoracao.md`, fase 10.2.

