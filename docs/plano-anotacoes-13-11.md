# Plano: anotações do usuário de 13/11/2026

As anotações vieram de uma análise do sistema feita pelo usuário. Cada item abaixo diz:
- o que o código faz hoje (com arquivo e linha, conferidos em `65e0c0327`);
- o que muda;
- a decisão tomada onde a anotação deixava escolha.

As etapas vêm **depois** do plano de gravação (`plano-gravacao.md`), na ordem da seção "Ordem" no fim. As regras são as
mesmas: sem atalhos, testes e A/B a cada passo, um commit por passo.

Tamanho: P (pequeno), M (médio), G (grande).

---

## Etapa 5: correções rápidas e o Linux

| # | Item | Hoje | Mudança | Tam. |
|---|---|---|---|---|
| 5.1 | Ícones quebrados no Linux, puxador do fader invisível | `embed::loadPixmap` (`src/gui/embed.cpp:83-125`) abre `artwork:<nome>` sem extensão pelo `QImageReader`. Os 14 `.svg` (`fader_knob`, `gear`, `detach`, `edit_knife`…) só carregam com o plugin de imagem `qsvg`. Fora do Windows ele não é instalado (`cmake/install/CMakeLists.txt:2-6`); sem ele vem um `QPixmap{1,1}`. | `loadPixmap` passa a renderizar o `.svg` direto com `QSvgRenderer` (o Qt Svg já está linkado), no tamanho pedido e com o `devicePixelRatio`. O pacote Linux leva o plugin também. Teste: carregar todos os ícones do tema com o plugin desligado (`QT_PLUGIN_PATH` vazio). Também `arp_up_on`/`arp_down_on` (`LfoControllerDialog.cpp:231-234`), que não existem. | P |
| 5.2 | Projeto novo vazio e mesa em modo digital | `Song::createNewProject` (`Song.cpp:1347-1400`) carrega o `default.mpt`, que tem TripleOscillator, sample, pattern com Kicker e automação, sem o atributo `analog`, então a mesa abre analógica (`Mixer.cpp:467`). O fallback (`:1370-1385`) cria as mesmas faixas. | `default.mpt` vazio com `analog="0"`. O fallback não cria faixas. Um `default.mpt` antigo do usuário (o "salvar como modelo padrão") continua valendo. | P |
| 5.3 | Tirar o botão TOL, tolerância sempre ligada | O botão está em `MixerView.cpp:284-297` e o modelo em `Mixer::m_toleranceModel`, falso por padrão (`Mixer.cpp:468`). Os projetos novos ligam (`Song.cpp:1395,1416`); os antigos abrem sem (`forgetTolerance`). | Sai o botão. O padrão passa a ser ligado. **Decisão:** projeto antigo sem o atributo abre **com** tolerância, como pede a anotação. A A/B registra essa mudança de som como esperada. | P |
| 5.4 | Tirar WIDTH, REL e TRIM do painel da direita | São controles do `MixerView` (`MixerView.cpp:215-256`), não da tira: TRIM (223), WIDTH (226) e REL (244, só no modo analógico). O TRIM também existe na tira (`MixerChannelView.cpp:529`). | Saem os três do painel. Os modelos continuam (projetos e automação). O TRIM fica só na tira; o REL do compressor fica no compressor da tira. **Decisão:** o WIDTH sai da interface e o modelo continua (100 %). | P |
| 5.5 | Tirar o botão de ajuda | `help_button`, `MainWindow.cpp:426,456`. | Sai. A ajuda continua no menu até a etapa 9. | P |
| 5.6 | O salvar pisca com fade no autosave | `project_save` é local (`MainWindow.cpp:424`); `autoSave()` está em `:1530-1552`. | O botão vira membro. Um autosave que deu certo dispara um `QVariantAnimation` de opacidade da cor de destaque do tema, 2 ciclos em ~1,2 s. | P |
| 5.7 | Tirar o LADSPA browser | É um plugin Tool (`plugins/LadspaBrowser`, listado em `cmake/modules/PluginList.cmake:44`); o menu Ferramentas é montado sozinho. | Sai da lista e a pasta é apagada. O `LadspaEffect` continua. | P |
| 5.8 | Tirar os efeitos inúteis | Amplifier (vol/pan/ganho L-R) e StereoMatrix (matriz L/R) repetem o que a mesa já faz. BassBooster repete o LO da EQ. StereoEnhancer repete o WIDTH. | **Decisão:** Amplifier e StereoMatrix saem do explorador e dos menus, mas o plugin continua compilado: projetos antigos que os usam abrem e tocam igual. Um campo `hidden` na tabela de categorias (`PluginCategories`) faz isso. BassBooster e StereoEnhancer ficam até o usuário decidir. | P |
| 5.9 | Abas escuras em todo lugar | O tema em uso é o `default` (configuração `paths/theme`). Ele não tem regra de `QTabBar` para as docas (cai no Fusion). As regras escuras estão só em `themes/linuxmusic/style.css:1229-1265`, e o `QTabBar#browserHeader` não tem regra em nenhum dos dois. | As regras escuras das docas (fundo `#26292d`, selecionada `#3a3d42`), mais `#browserHeader`, vão para o `default/style.css`, com as cores do tema LMMS. | P |

## Etapa 6: mesa — roteamento, ganho e a cadeia de efeitos

- **6.1 Duas ou mais faixas no mesmo canal, cada uma com volume e pan** (M).
  - Hoje `Mixer::setUpTrackChannel` (`Mixer.cpp:1498-1561`) cria um canal novo sempre que o destino já é usado por outra faixa. O volume e o pan da faixa já existem antes do canal (`AudioBusHandle.cpp:147-230`).
  - Mudança: escolher no LCD do canal da faixa um canal já usado passa a ser aceito. As duas faixas somam no canal, cada uma com seu VOL/PAN do cabeçalho.
  - O passo que move os efeitos da faixa para o canal (`:1550`) só move quando o canal tem uma faixa só. Com duas, os efeitos ficam na faixa.
  - `releaseTrackChannel` já conta quantos usam o canal.
  - Teste: duas faixas num canal somam igual a cada uma sozinha.
- **6.2 Canais FX isolados e numeração 1, 2, 3** (M).
  - Hoje as voltas FX (`ensureFxReturns`, `Mixer.cpp:1796-1832`) são canais comuns no mesmo índice. Uma faixa criada depois ganha um número maior que elas, e nada impede uma faixa ou um envio de apontar para uma volta FX.
  - Mudança:
    1. As voltas ficam numa lista própria do mixer, mostradas como "FX A", "FX B".
    2. Os canais das faixas são numerados 1..N sem buracos.
    3. O LCD do canal da faixa e os envios entre canais não aceitam uma volta FX. Só os botões de envio FX alimentam uma volta.
    4. Projetos salvos convertem os índices ao abrir (o atributo `fxreturn` já existe).
- **6.3 Ponto de inserção da cadeia de efeitos** (G).
  - Hoje a cadeia roda depois da tira inteira: trim → mono → PRE do master → tira analógica → **efeitos** → master → pré-fader → largura/pan/fader (`Mixer.cpp:190-459`).
  - Mudança: um seletor no rack do canal com **antes do ganho, depois do ganho (padrão), depois dos filtros, depois da dinâmica, depois da EQ e antes do pan**.
  - `AnalogChannelStrip::process` (`AnalogChannelStrip.cpp:125-249`) é dividido em estágios (ganho e low cut, filtros, dinâmica, EQ). O laço por amostra vira um laço por estágio sobre o bloco, e a cadeia roda entre dois estágios.
  - O modo digital usa as mesmas posições, com os estágios digitais.
  - A/B: o padrão "depois do ganho" **muda** a ordem atual ("antes do pan"). **Decisão:** projeto antigo abre com "antes do pan", que é o som de hoje; projeto novo começa com "depois do ganho".
- **6.4 Tirar o "adicionar efeito" e o modal** (P).
  - O botão está em `EffectRackView.cpp:74-80`; o arrastar do explorador já funciona (`:251-260`).
  - Mudança: sai o botão. Duplo clique num efeito do explorador de mixagem o põe no canal selecionado.
  - O `EffectSelectDialog` fica só como código de instanciação (`instantiateDragged`); a parte de diálogo sai.
- **6.5 Os botões do efeito só com ícone, e o pin connector com um cabo P10** (P).
  - Hoje já são ícones (`gear`, e `tool` em `EffectView.cpp:90,111`).
  - Mudança: um `pin_connector.svg` novo (plugue P10 com cabo, no traço dos ícones do tema).
- **6.6 Faixas de ganho e saturação** (M). O que foi medido no código:
  - Pot de GAIN: `MaxGain = 30 dB`, unidade em 25 % do curso (`AnalogChannelStrip.h:69-70`). Os op-amps só dobram em +18 a +21 dBFS (`GainStage.h:58-66`). Um sinal perto de 0 dBFS precisa de +18 dB de ganho para colorir: é o "não satura o suficiente".
  - TRIM: −24..+24 dB (`Mixer.cpp:110`). Não chega a mudo.
  - Volume da faixa: 0–200 % (+6 dB, `volume.h:36`). O fader do canal e o PRE do master vão até +6 dB.
  - **Não há −6 dB no caminho do áudio.** Os "−6" do código são a cor de aviso do medidor (`Fader.cpp:667`), uma marca da escala (`MixerChannelView.cpp:96`) e o "air" lateral ±6. A lei de pan é linear com unidade no centro.
  - **Plano da investigação:** um teste mede o ganho de ponta a ponta de faixa → canal → master com tudo em zero, nos modos analógico e digital, em mono e estéreo, e com pan no centro. Se aparecer −6 dB, a causa é corrigida na origem. A suspeita é a soma estéreo↔mono ou a lei de pan de algum instrumento.
  - **Decisões:**
    - GAIN vai a **+60 dB**, como um pré de console, com a unidade no mesmo lugar do curso. Os pontos de dobra dos op-amps passam a ser alcançados no meio do curso, com o modelo de saturação revisto para soar progressivo, sem degrau.
    - O TRIM ganha **−∞**: abaixo de −60 dB é mudo, com curva logarítmica.
    - O volume da faixa e o fader do canal vão a **+12 dB**.
    - A conversão de projeto mantém os valores em dB, então nada muda de som ao abrir.
- **6.7 Cores dos faders** (P).
  - Hoje o puxador é um pixmap só (`Fader.h:177`).
  - Mudança: uma propriedade `knobColor` (CSS) pinta o puxador. As cores são uma por papel: master PRE, master POST e FX. Os canais ficam com a cor de hoje.

## Etapa 7: compressor do mixbus fiel ao SSL G

- Hoje é `MasterSection::processCompressor` (`MasterSection.cpp:209-260`): pico feed-forward com knee suave, ataque e release contínuos e um release automático heurístico. Mostra a redução numa barra horizontal (`GainReductionMeter`).
- Mudança, seguindo o SSL G (pesquisa em `pesquisa-compressores.md`):
  - um VCA com detector RMS/pico do circuito original;
  - ataque em degraus: 0,1 / 0,3 / 1 / 3 / 10 / 30 ms;
  - release em degraus: 0,1 / 0,3 / 0,6 / 1,2 s e Auto (o duplo tempo do original);
  - ratio 2, 4 e 10, e o knee do original;
  - make-up;
  - sidechain HPF.
- Interface:
  - o **VU de agulha próprio** de redução de ganho;
  - as chaves giratórias e o painel no desenho do original, com as cores do tema LMMS.
- Projetos salvos: os valores contínuos são levados ao degrau mais perto. A/B marcada como mudança esperada.
- Tamanho: G.

## Etapa 8: o Explorador

- **8.1 Nome e lugar** (M).
  - Hoje há um único `BrowserPanel` fora das páginas (`MainWindow.cpp:117-162`), visível em Composição e Mixagem.
  - Mudança:
    - **Explorador de composição:** instrumentos, presets, samples e arquivos do projeto. Fica só na página Composição.
    - **Explorador de mixagem:** só efeitos. Fica só na página Mixagem.
    - Cada um é um `BrowserPanel`, com suas chaves de configuração. O `showPage()` mostra o da página.
    - A aba se chama "Explorador" e ganha ícone próprio.
- **8.2 Botões quadrados juntos e centralizados** (P). Hoje `BrowserPanel.cpp:59-61` tem espaçamento 3 e um stretch à direita. Passa a ter espaçamento 0, com stretch dos dois lados.
- **8.3 Só a busca** (P).
  - Saem:
    - o título em maiúsculas (`BrowserPanel.cpp:64-75`);
    - os textos de explicação (`PluginBrowser.cpp:66-71`, `EffectBrowser.cpp:98-100`);
    - as caixas Usuário/Fábrica/Ocultos (`FileBrowser.cpp:153-212`).
  - Os filtros ficam fixos: usuário e fábrica visíveis, ocultos não.
- **8.4 Sem a aba Computador; a pasta do projeto no lugar** (M).
  - A aba "Computador" (`MainWindow.cpp:148-160`, raiz `home` + `/Volumes`) sai.
  - Entra "Projeto": a pasta do arquivo do projeto, atualizada em `projectFileNameChanged`/`projectLoaded`. Projeto não salvo mostra a pasta temporária de mídia.
  - Os caminhos ficam relativos ao projeto (etapa 11).
- **8.5 Ícone por tipo** (M).
  - Hoje os arquivos são tipados pela extensão (`FileItem::determineFileType`, `FileBrowser.cpp:1243`) em 7 ícones, e os efeitos não têm ícone (`EffectBrowser::fill`, `:132-172`).
  - Mudança:
    - **Arquivos:** tipo por extensão e, sem extensão conhecida, pelo MIME (`QMimeDatabase`). Os ícones são áudio, MIDI, projeto, preset, soundfont, IR, NAM e plugin.
    - **Plugins:** o ícone do plugin, ou o da sua categoria.
    - **Categorias com cor:** EQ e filtros azul, dinâmica amarelo, delay e reverb roxo, modulação verde, distorção vermelho, tom e frequência laranja, estéreo e utilitário cinza, análise ciano. As tabelas de `PluginCategories` já existem e ganham `color`, `icon` e `hidden`. O `Plugin::Descriptor` não muda, porque os inicializadores posicionais de todos os plugins quebrariam.

## Etapa 9: transporte, régua, editores e atalhos

- **9.1 Transporte no estilo do Logic** (M).
  - Hoje o `TransportBar.cpp:123-246` tem metrônomo, Part/Song, ▶, ●, ●▶, ■, relógio, tempo, VOL e PITCH.
  - Mudança:
    - ícones novos (SVG) e o desenho dos botões do Logic: grupo arredondado, ● vermelho, ciclo amarelo e metrônomo;
    - **sai Part/Song:** o ▶ toca o arranjo, e o editor de patterns volta a ter o próprio ▶;
    - **saem VOL e PITCH;** entra um **mudo do master** ligado ao `m_muteModel` do canal master. Os modelos de volume e tom do master ficam, com os valores salvos.
- **9.2 Selecionar um trecho na régua e o ciclo** (M).
  - Hoje o loop é definido com Shift+arrastar ou pelo menu de contexto (`TimeLineWidget.cpp:289-436`). Clique simples move o cursor; Ctrl+arrastar seleciona clipes.
  - Mudança:
    - arrastar na metade de cima da régua cria o trecho (com snap), e clique simples continua movendo o cursor;
    - um botão **Ciclo** no transporte;
    - a tecla **C** (Logic) e **L** (REAPER).
- **9.3 Barras do Piano Roll e do Arranjo com ícones clicáveis** (M). Os `ComboBox` com rótulo (zoom, quantização, tamanho de nota, escala, acorde e snap: `PianoRoll.cpp:5328-5393`, `SongEditor.cpp:810-872`) viram `QToolButton` com ícone e menu.
- **9.4 Ctrl+C / Ctrl+V no Arranjo** (M).
  - Hoje não há atalho: copiar e colar só pelo menu do clipe. `ClipView::paste` (`ClipView.cpp:1364-1378`) cola sobre o clipe clicado e **apaga esse clipe**.
  - Mudança:
    - Ctrl+C, Ctrl+X, Ctrl+V e Ctrl+D (duplicar) no `SongEditor::keyPressEvent`;
    - colar no cursor de reprodução, na faixa selecionada, mantendo a distribuição entre as faixas;
    - colar nunca apaga clipe; sobreposição vira lane/take.
- **9.5 Expandir e recolher em cada área de abas** (P). Um botão no canto de cada `QTabWidget` das docas (`setCornerWidget`) usa os `revealDock`/`collapseDock` que já existem. A doca de cima passa a recolher também.
- **9.6 Atalhos como o REAPER** (M).
  - Hoje os atalhos estão espalhados em vários `keyPressEvent`.
  - Mudança: um registro central de atalhos (ação, tecla padrão e editável nas configurações), com os padrões do REAPER:
    - Espaço tocar/parar; R gravar; Ctrl+R gravar já armado;
    - Home/End;
    - S cortar o item no cursor; mudo e solo da faixa selecionada em teclas próprias (no REAPER, S é cortar);
    - Ctrl+roda zoom horizontal; Alt+roda zoom vertical;
    - Alt+arrastar a borda do item estica (com o time-stretch);
    - Ctrl+Z / Ctrl+Shift+Z.
  - Os conflitos com os atalhos atuais (Shift+D/E/S/T no Piano Roll) ficam como estão no editor que já os tem.

- **9.7 Marcadores na régua, como no REAPER** (M; pedido do usuário em 13/11).
  - Um marcador é um ponto da régua com um **texto** opcional (por exemplo "Refrão", "Verso 2"), com cor, para
    anotar as partes da música. Ele **pode ou não** marcar uma mudança de tempo: a mudança de tempo é uma
    propriedade do marcador (BPM, salto ou rampa), ligada ou desligada.
  - **Modelo (decidido ao fazer):** os marcadores com nome são uma lista própria do projeto (`SongMarkers`: tick,
    nome, cor), e o tempo fica no `TempoMap`. Um marcador "marca o tempo" quando há uma mudança de tempo no mesmo
    tick (uma opção no menu dele). Assim a reprodução fica exatamente como era.
  - **Interface:** uma faixa de marcadores sob a régua. A faixa TEMPO atual vira essa faixa, e os marcadores sem
    tempo aparecem como bandeiras com o texto.
    - Duplo clique cria, arrastar move, e o menu de contexto renomeia, muda a cor, liga ou desliga o tempo e apaga.
    - Teclas, como no REAPER: M insere um marcador no cursor; 1 a 9 vão ao marcador; Shift+M (não usada no REAPER)
      abre a lista.
  - **Regiões**, como no REAPER, ficam para depois: um trecho com nome (do início ao fim de uma parte), que pode
    virar a seleção do ciclo com um clique e uma faixa de exportação no assistente.
  - Desfazer: cada edição de marcador é um passo.

## Etapa 10: configurações dentro do app e o menu

- **10.1 Configurações como página** (M).
  - Hoje o `SetupDialog` é um `QDialog` com 5 abas, OK/Cancelar e aviso de reinício.
  - Mudança:
    - vira um widget na terceira página do `m_pages`, aberto por um botão de engrenagem ao lado de COMPOSIÇÃO/MIXAGEM;
    - tem "Aplicar" e "Descartar" no rodapé, e o aviso de reinício fica igual;
    - `showPage`/`page()` deixam de supor duas páginas;
    - o desligar do journal passa para a hora em que a página é aplicada.
- **10.2 Menu de cima** (M). Revisto depois das etapas 5–10.1, com o que sobra em cada menu:
  - **Arquivo:** novo, abrir, recentes, salvar, salvar como, importar, exportar (o assistente) e sair;
  - **Editar:** desfazer e refazer;
  - **Ver:** as áreas;
  - **Ajuda:** sobre.
  - Configurações e escalas saem para a página nova.

## Etapa 11: arquivos do projeto

- **11.1 O projeto guarda o que usa** (M).
  - Hoje os caminhos são gravados por `PathUtil::toShortestRelative` (`PathUtil.cpp:236`), que só usa `local:` se pedido, e ninguém pede. O "salvar como pacote" (`DataFile::copyResources`, `:414`) copia só `sampleclip@src` e `audiofileprocessor@src`, copia samples de fábrica também, e não segue SF2, GIG, Patman, SlicerT nem os takes.
  - Mudança:
    1. Ao salvar, um mapa de **todas** as dependências de arquivo, cada plugin declarando as suas. Entram SampleClip (com os takes, o original e o renderizado), AudioFileProcessor, SlicerT, Sf2Player, GigPlayer, Patman, IR e NAM.
    2. O que não é da fábrica nem da biblioteca do usuário é copiado para `<projeto>_media/` e gravado como `local:`.
    3. "Salvar como" em outra pasta copia esses arquivos junto.
    4. Cada elemento novo entra em `ELEMENTS_WITH_RESOURCES` (`DataFile.cpp:60`). A checagem de segurança de `Song.cpp:1454` rejeita `local:` fora dessa lista.
  - A cópia roda numa thread com progresso.
- **11.2 Sample ou faixa de áudio: o critério dos 30 s** (P).
  - Hoje soltar áudio no Arranjo cria sempre uma faixa de instrumento com AudioFileProcessor (`TrackContainerView.cpp:462`).
  - Mudança: a duração vem do cabeçalho (`sf_info`, sem decodificar). Acima de 30 s cria uma faixa de áudio com o clipe no ponto do drop; até 30 s, o instrumento como hoje.
- **11.3 Vários arquivos de uma vez, um por faixa (stems)** (M).
  - Hoje o drag leva um valor só (`StringPairDrag`), e nada aceita `text/uri-list` (arquivos do Finder).
  - Mudança:
    - o Arranjo aceita URLs;
    - o explorador ganha seleção múltipla;
    - "Arquivo > Importar áudio…" aceita vários arquivos.
  - Cada arquivo vira uma faixa, todas no mesmo ponto, num grupo de desfazer, com progresso.

## Etapa 12: exportação

- **12.1 Assistente passo a passo** (G).
  - Hoje o `ExportProjectDialog` (296 linhas) exporta um formato por vez, direto ao encoder (`ProjectRenderer::run`).
  - O assistente tem 4 passos:
    1. **Trecho:** a música, o ciclo ou entre marcadores; quantas repetições; cauda.
    2. **Formatos:** vários de uma vez (WAV, FLAC, OGG e MP3, cada um com sua taxa, bits e bitrate).
    3. **Pós-processamento:** normalizar (pico ou LUFS integrado, alvo em dB), fades, dither na redução de bits.
    4. **Pasta de destino e nome.**
  - Renderiza uma vez para um WAV float temporário, aplica o pós-processamento e codifica cada formato a partir dele. É preciso um codificador offline que leia um buffer: os `AudioFile*` hoje só recebem do motor.
- **12.2 Exportar uma faixa ou um clipe pelo menu de contexto** (M).
  - Áudio (WAV, MP3, FLAC, OGG): `RenderManager` com uma lista de faixas (o solo do `renderTracks`), no trecho do clipe quando é um clipe.
  - MIDI: `MidiExport::tryExport` com uma faixa. Para um clipe, uma faixa temporária com só aquele clipe.

## Etapa 13: samples, instrumentos e plugins novos

- **13.1 Limpar os samples** (P).
  - Ficam:
    - basses, bassloops, beats;
    - os eletrônicos de `drums`;
    - effects, misc;
    - `shapes` e `waveforms` (usados pelos osciladores);
    - os pads sintéticos de `stringsnpads`;
    - os DrumSynth das máquinas (606, 808, 909, CR78, CR8000, Linn, electro).
  - Saem:
    - `instruments` (22 acústicos) e `latin`;
    - os acústicos de `drums` e de `stringsnpads`;
    - os DrumSynth `acoustic`, `instrument`, `latin` e `farfisa`.
  - O que usa o que sai é revisto junto (um teste de carga confere que nenhum arquivo de `data/` aponta para sample inexistente):
    - os presets `SString`, `orion` e `subbass`;
    - a demo `StrictProduction-DearJonDoe`;
    - o modelo `AcousticDrumset`.
- **13.2 Instrumentos acústicos nativos** (M). O lugar dos samples acústicos é ocupado por um instrumento nativo sobre o FluidSynth, como o AVL Drums.
  - **Decisão:** uma soundfont GM de licença livre (GeneralUser GS ou FluidR3 em sf3), com presets por instrumento (piano, cordas, sopros e violões) no explorador.
  - Os modelados que já existem (Mallets do STK, Vibed, Organic) ganham presets no lugar dos samples.
- **13.3 Plugin de IR e NAM** (G).
  - Não há convolução nem NAM no código.
  - **Convolução:** um convolvedor particionado uniforme sobre o FFTW (já é dependência), com latência zero na primeira partição (direta) e as seguintes em FFT.
  - **NAM:** o NeuralAmpModelerCore (MIT, com Eigen e nlohmann/json) como submódulo.
  - Um plugin "Amp e IR", com:
    - modelo NAM (opcional);
    - IR;
    - mix;
    - ganho de entrada e saída.
  - A biblioteca pequena tem IRs de caixa de guitarra, de violão (corpo) e de salas/reverb, mais alguns NAM de amp. Só arquivos CC0 ou CC-BY, com os créditos em `data/ir/LICENSES`.

---

## Ordem

Depois do item 19 do plano de gravação (time-stretch), que está em andamento:
1. **Etapa 5** inteira: são correções que o usuário vê já, e o ícone no Linux bloqueia o teste dele.
2. **Etapa 6:** 6.4, 6.5, 6.7, depois 6.6, 6.1, 6.2 e 6.3.
3. **Etapa 8** (Explorador) e **9.1/9.2** (transporte e ciclo).
4. O resto do plano de gravação (faixa de áudio de duas linhas, interface da gravação, mídia).
5. **Etapas 11, 9.3–9.6, 10, 12, 7, 13**.

Cada passo atualiza este documento, marcando o que foi feito.

## Estado

- **Etapa 5 feita** (13/11, com testes, A/B e commits):
  - 5.1: os ícones SVG carregam sem o plugin `qsvg`, e as imagens SVG do CSS viram PNG no cache quando ele falta
    (`ThemeImagesTest`, que falha sem a correção);
  - 5.2: projeto novo vazio, com a mesa digital;
  - 5.3: sem o TOL, com a tolerância sempre ligada. Na A/B, os projetos antigos (que abrem no modo analógico)
    mudam de −40 a −65 dB, como esperado; essa passa a ser a referência;
  - 5.4: o painel ficou só com MONO e PAN. O release do compressor foi para a tira, ao lado de um knob ATK em
    degraus (FAST/NORM/SLOW), que substitui o botão de ataque;
  - 5.5 e 5.6: sem o botão de ajuda, e o salvar pisca no autosave;
  - 5.7: sem o LADSPA browser;
  - 5.8: Amplifier e Stereo Matrix fora das listas; projetos que os usam tocam igual;
  - 5.9: abas escuras nas áreas e no explorador.
- **Etapa 6 feita** (13/11):
  - 6.1: várias faixas no mesmo canal, cada uma com VOL e PAN. Abrir o projeto não separa mais; faixas que
    dividiam um canal que não era de faixa (projetos antigos) ganham um canal cada, como antes;
  - 6.2: as voltas FX ficam sempre por último e não recebem faixa, e os canais são numerados 1, 2, 3. As voltas
    não mostram número e têm a sua própria tolerância (faixa 1001, 1002), que não muda quando entram canais. Na
    A/B, só o `ladspa-sc4`, que tinha voltas no meio dos canais, muda (−41 dB);
  - 6.3: seletor "FX" no painel, com antes do ganho, depois do ganho (padrão), depois dos filtros, depois da
    dinâmica, depois da EQ e antes do pan (pós-fader). Os projetos antigos abrem "depois da EQ", onde os efeitos
    estavam; a A/B não muda;
  - 6.4: sem o "Adicionar efeito": arrastar ou dar duplo clique no explorador. O duplo clique num slot vazio abre os
    efeitos no explorador;
  - 6.5: ícone do pin connector (plugue P10 com cabo);
  - 6.6: GAIN até +60 dB (projetos antigos mantêm o ganho), TRIM até desligado, faixas e faders até +12 dB.
    **Não há −6 dB no caminho**: `GainStagingTest` mede 0,00 dB de ponta a ponta (−0,06 dB pela tira analógica).
    A A/B achou o `MaxTrackVolume` estourando o `volume_t` de 8 bits, e isso foi corrigido;
  - 6.7: puxadores dos faders por papel (canal verde, FX azul, PRE laranja, POST vermelho).
- **Etapa 8 feita** (13/11):
  - um Explorador por página: Composição (instrumentos, favoritos, projetos, samples, presets e arquivos do projeto)
    e Mixagem (efeitos);
  - sem a aba Computador: a pasta do projeto no lugar dela;
  - só o campo de busca (sem título, textos, caixas Usuário/Fábrica/Ocultos e o botão de recarregar; a lista é
    lida de novo ao aparecer). Os botões ficam juntos e centralizados, e a aba se chama "Explorador", com ícone;
  - categorias com cor e ícone genérico de duas letras; cada efeito com o próprio logo emoldurado na cor da
    categoria. Arquivos sem extensão conhecida são tipados pelo MIME, e `.nam` tem tipo e ícone;
  - traduções em português das strings novas da sessão.
- **Etapa 9.1 e 9.2 feitas** (13/11):
  - transporte num grupo arredondado como no Logic (play verde, gravar vermelho, ciclo amarelo, metrônomo azul);
  - sem Part/Song: o Editor de Sequências voltou a ter o próprio play;
  - sem VOL e PITCH: no lugar, o mudo do master;
  - o ciclo: arrastar na metade de cima da régua seleciona o trecho e liga o ciclo; tecla C. A tecla R grava.
- **Etapa 9.4–9.6 feitas** (13/11):
  - no Arranjo, Ctrl+C, Ctrl+X, Ctrl+V (no cursor de reprodução) e Ctrl+D (duplica logo depois da seleção). Colar
    nunca apaga clipe;
  - cada área de abas recolhe e expande com um botão no canto;
  - mouse como no REAPER: a roda dá zoom horizontal, Ctrl+roda muda a altura das faixas, Alt+roda rola na
    horizontal e Shift+roda na vertical. S corta no cursor;
  - pendente: o registro central de atalhos editável nas configurações.
- **Etapa 10 feita** (13/11):
  - as configurações viraram uma página, atrás de uma engrenagem ao lado de COMPOSIÇÃO/MIXAGEM, com Cmd+,;
  - menu: mantido. As configurações pelo menu abrem a página; a ajuda saiu da barra e ficou no menu.
- **Etapa 11 feita** (13/11):
  - ao salvar, o projeto copia para `<projeto>_media/` todo arquivo que usa e que não é de fábrica, com caminho
    `local:`. "Salvar como" leva os arquivos. A checagem de `local:` passou a ser por atributo;
  - acima de 30 s, o arquivo vira faixa de sample com o clipe; abaixo, instrumento;
  - vários arquivos de uma vez (do Finder ou de "Importar arquivos de áudio..."): uma faixa por arquivo.
- **Configurações** (13/11, pedidos do usuário):
  - a tela ocupa a página toda, com a aba Geral em duas colunas;
  - tem o botão Salvar. Ao sair da página com mudanças, um modal pergunta Salvar, Descartar ou Cancelar (ficar);
  - opções novas: projetos novos com mesa analógica ou digital; quantos segundos fazem um arquivo virar faixa de
    áudio; o grupo Gravação (calibração da latência, pré-roll, contagem);
  - saíram as opções sem efeito: o osciloscópio do master e a imagem de fundo;
  - a tela está em português.
- **Etapa 12 feita** (13/11):
  - assistente de exportação em 4 passos (trecho, formatos, processamento, pasta). Renderiza uma vez e grava todos
    os formatos de uma vez, com normalização por pico ou loudness (ITU-R BS.1770, testada com a referência de
    −3,01 LUFS), fades e dither TPDF;
  - pelo menu de contexto: exportar uma faixa sozinha como áudio ou MIDI, e um clipe sozinho como áudio.
- **Etapa 13.1 feita** (13/11): os samples acústicos saíram (106 arquivos, com o modelo AcousticDrumset e a demo
  StrictProduction, que dependiam deles). Um teste garante que nenhum preset, demo ou modelo aponta para um sample
  que não existe.
- **Corrigido:** o aviso "Sample not found: samples/empty.wav", que vinha de 21 presets do TripleOscillator.
- **Etapa 9.7 feita** (13/11): marcadores com nome e cor numa linha acima do tempo, sob a régua.
  - Duplo clique cria ou renomeia, arrastar move. O menu renomeia, muda a cor, marca ou desmarca a mudança de
    tempo, toca dali e apaga.
  - M cria um marcador no cursor e 1 a 9 vão aos marcadores, como no REAPER.
  - São salvos no projeto, e cada edição é um passo de desfazer.
  - Regiões ficam para depois.
- **Etapa 9.3 feita** (13/11): nas barras do Piano Roll e do Arranjo, os seletores viraram botões de ícone que
  mostram o valor atual e abrem o mesmo menu.
- **Falta das anotações:**
  - 7 (compressor SSL do mixbus);
  - 13.2 (instrumentos acústicos por soundfont);
  - 13.3 (plugin IR/NAM);
  - o registro central de atalhos;
  - regiões na régua.
