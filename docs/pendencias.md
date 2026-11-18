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
    - queda ao abrir a janela do AudioFileProcessor (#8647): não se repetiu aqui (o teste da interface abriu a
      janela do `inst-afp` sem cair em 17/11). Testar ainda com um arquivo carregado e mudando o loop;
    - Vibed a 192 kHz (divisão inteira que dá zero);
    - Xpressive trava por overflow;
    - notas presas (OpulenZ, Zyn com arpejador, troca de canal);
    - LB302 não reseta entre exportações;
    - volumes padrão muito desiguais;
    - o host LV2 não abre a interface nativa dos plugins;
    - aliasing em vários osciladores;
    - FreeBoy com offset DC.
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
- [ ] **Categorias dos instrumentos:**
  - o Vibed sai de "modelagem física" e vai para "sintetizador";
  - subcategorias de sintetizador, porque são muitos.
- [ ] **Plugin de baterias eletrônicas:** um instrumento visual como o AVL Drums, que:
  - usa as amostras de baterias eletrônicas que já temos;
  - mapeia cada peça numa nota MIDI;
  - troca de bateria com a foto de cada uma, como no AVL.
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

