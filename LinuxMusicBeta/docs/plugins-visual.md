# O visual dos plugins do legado (inventário)

Este documento é o inventário pedido em `interfaces.md`, I-22: cada plugin nativo do legado (as 62 pastas de
`legado/plugins/`), se tinha visual próprio ou não, de onde vem esse visual e o estado do port. Ele guia o port dos
painéis para `lm-gui-plugins`:

- quem tem **visual próprio** (arte de fundo, controles posicionados com imagens, widgets ou gráficos desenhados
  pelo próprio plugin) é portado fiel, no próprio estilo;
- quem é **genérico / botões soltos** usa o editor genérico (I-9), sem visual portado;
- quem é **de terceiros fora do processo** abre a janela do próprio plugin (`decisoes-autonomas.md`, P-01, P-02,
  P-03, P-05);
- quem é **obsoleto** não aparece no programa novo, ou só existe para abrir projetos antigos (dito em cada caso).

Levantado no código do legado em 20/11, só por leitura. As posições, os tamanhos e as cores são os do código e das
imagens: é isso que o port reproduz.

## Convenções

- Caminhos relativos a `legado/`. As imagens de um plugin são carregadas por `PLUGIN_NAME::getIconPixmap("nome")`,
  que lê `plugins/<Plugin>/nome.png` (ou `.svg`); `embed::getIconPixmap` lê do tema (`data/themes/default/`).
- Tamanhos das imagens medidos com `sips` (PNG) ou lidos do `width`/`height`/`viewBox` (SVG). Todas as imagens são 1x;
  nenhuma tem versão 2x no legado.
- Coordenadas em pixels, relativas ao canto superior esquerdo da janela do plugin, com as constantes e os laços já
  resolvidos. "Dica" é o texto de `setHintText` (prefixo + unidade) ou `setToolTip`.
- Fontes: `SMALL_FONT_SIZE` 10 px, `DEFAULT_FONT_SIZE` 12 px, `LARGE_FONT_SIZE` 14 px (`include/FontHelper.h`).
  `ComboBox::DEFAULT_HEIGHT` 22.
- **Instrumentos:** a janela é `InstrumentViewFixedSize`, 250×250 (`include/InstrumentView.h`), com o `artwork.png`
  (250×250) como fundo pela paleta (`setAutoFillBackground(true)` +
  `QPalette::setBrush(backgroundRole(), PLUGIN_NAME::getIconPixmap("artwork"))`), salvo nota.
- **Efeitos:** `EffectControlDialog`; o fundo também vem pela paleta. Quando a janela é dimensionada por layout
  (`setSizeConstraint(QLayout::SetFixedSize)`), o tamanho real vem do layout e o fundo é repetido a partir de (0, 0)
  se a janela passar da imagem; o tamanho do SVG é o tamanho pretendido.
- Os `artwork.svg` dos efeitos simples usam o mesmo fundo: gradiente vertical de `#101116` (em cima) a `#08090c`
  (embaixo). Os painéis internos (Bitcrush, CrossoverEQ, DualFilter) usam um segundo gradiente de `#2d333a` (em cima)
  a `#1a1d21` (embaixo), contorno preto de 1 px, raio 2,065, filete branco com opacidade 0,1 na primeira linha de
  dentro e sombra preta 0,25/0,3 na última.
- `PixmapButton::setActiveGraphic()` redimensiona o botão para a imagem ativa: um `resize()` feito antes no código
  não vale. O tamanho de um `PixmapButton` é o da imagem.
- Widgets filhos sem layout e sem `resize()` (`Draggable`, `QGraphicsView`...) ficam do tamanho do `sizeHint()`
  (`adjustSize()` no primeiro show).
- Os visores vivos se atualizam pelo `MainWindow::periodicUpdate()`, um timer de `1000 / 60` ms (60 por segundo,
  `src/gui/MainWindow.cpp:262`), salvo nota.
- Ícones de onda do tema usados por vários instrumentos (`data/themes/default/`, todos 14×14):
  `sin_wave_{active,inactive}.png`, `triangle_wave_*`, `saw_wave_*`, `square_wave_*`, `round_square_wave_*`,
  `moog_saw_wave_*`, `exp_wave_*`, `white_noise_wave_*`, `usr_wave_*`.
- As cores dos knobs `Styled` vêm do `style.css` do tema (tabela em "Os widgets do LMMS"); cada seção repete a regra
  do seu plugin.

## Os widgets do LMMS

Todo painel com visual próprio é montado com estes widgets do legado. Eles são reproduzidos uma vez, em
`lm-gui-plugins`, e cada painel só os posiciona. Fontes: `legado/src/gui/widgets/*.cpp`, `legado/include/*.h`,
`legado/data/themes/default/style.css` e as imagens de `legado/data/themes/default/`. Tamanhos em pixels lógicos
(as imagens do tema são 1x).

### A paleta do tema (`LmmsPalette`, `style.css`)

| Papel | Cor |
|---|---|
| `background` (janela) | `#262b30` |
| `windowText` | `#1de276` (cor do ponteiro e do arco dos knobs Dark/Bright/Small) |
| `base` | `#101213` |
| `text`, `buttonText`, `brightText`, `highlightedText` | `#d1d8e4` |
| `button` | `#3f4750` |
| `shadow` | `rgb(0,0,0)` (cor do ponteiro do knob Vintage) |
| `highlight` | `#262b30` |

**O tema `linuxmusic` do fork** (`legado/data/themes/linuxmusic/style.css`, só o CSS; as imagens que faltam vêm do
`default`, que continua sendo o padrão em `ConfigManager.h`, `DEFAULT_THEME_PATH`) troca a paleta e o verde: fundo
`#2f3237`, `windowText` `#2bd4a8`, `base` `#1c1e21`, texto `#d6d9dd`, botão `#44484e`; nos plugins, o mesmo CSS com
estas trocas: Sfxr `#repeatKnob` `#2a2c2f`; Watsyn `#mixKnob` e `#mixenvKnob` `#3fe0b6`; knobs do Compressor
`#27c79c`; traço do Vectorscope `rgba(43,212,168,255)`; `fullAmountColor` dos gráficos de envelope e LFO
`rgb(43,212,168)`. Este inventário usa os valores do `default`; o port escolhe o tema pela configuração.

Texto flutuante do valor (`SimpleTextFloat`) e dica (`QToolTip`): fundo `#040506`, texto `#d1d8e4` (a dica tem
canto de 4 px). Tamanhos de fonte (`FontHelper.h`): `SMALL_FONT_SIZE` 10 px, `DEFAULT_FONT_SIZE` 12 px,
`LARGE_FONT_SIZE` 14 px.

### Knob (`Knob.cpp`, `Knob.h`, base `FloatModelEditorBase.cpp`)

**Tipos com imagem** (`KnobType`, a imagem é o corpo inteiro do knob; o widget tem o tamanho da imagem, mais a
legenda se houver):

| KnobType | Imagem | Tamanho | Raio `r = largura/2 − 1` | Centro `mid` | Ponteiro (de → até o centro) | Cor do ponteiro |
|---|---|---|---|---|---|---|
| `Dark28` | `knob01.png` | 29×29 | 13,5 | (largura/2, 14) | de `(r−10)/3 = 1,17` até `r−4 = 9,5`, a linha toda deslocada (+1, +1) | `windowText` `#1de276` |
| `Bright26` | `knob02.png` | 27×27 | 12,5 | (largura/2, 13) | de 1 até `r−5 = 7,5` | `#1de276` |
| `Small17` | `knob03.png` | 19×19 | 8,5 | (largura/2, 9) | de 1 até `r−2 = 6,5` | `#1de276` |
| `Vintage32` | `knob05.png` | 32×32 | 15 | (largura/2, 16) | de 2 até `r−2 = 13` | `shadow` preto |

- **Ângulo:** curso total `m_totalAngle` = 270° (mínimo aceito 10°). O ângulo é
  `int((v − (min+max)/2) / (max − min) · 270) % 360`, com `v` no valor "inverso da escala" (knobs logarítmicos andam
  em log); vai de −135° (mínimo, embaixo à esquerda) a +135° (máximo, embaixo à direita), 0° para cima, sentido
  horário. O ponto a raio `ρ` é `(mid.x + sin(a)·ρ, mid.y − cos(a)·ρ)`.
- **Ponteiro:** linha com caneta de 2 px, antialias.
- **Arco** (todos os tipos com imagem): retângulo `(mid.x − s/2, 1, s, s)` com `s = largura da imagem − 2`, caneta
  de 2 px.
  - Arco de fundo: de 315° (ângulos do Qt, 0° = 3 h, anti-horário) com extensão de 270°, cor do arco = cor do
    ponteiro com alfa 70.
  - Arco do valor: começa no ângulo do valor central do modelo (`centerValue`, normalmente o mínimo; o meio para
    knobs bipolares) e vai até o ângulo atual, desenhado na cor do ponteiro (opaco), por cima.
- **Desligado** (`!isEnabled()`): a imagem vira tons de cinza (`convertPixmapToGrayScale`); ponteiro
  `rgb(120,120,120)`, arco `rgba(120,120,120,70)` (`style.css`).
- **Legenda** (`setLabel`): texto centrado embaixo da imagem, cor `textColor` (padrão branco `rgb(255,255,255)`).
  - Modo legado (`LegacyFixedFontSize`): fonte de 10 px, linha de base a 2 px da borda de baixo, altura do widget =
    altura da imagem + 10, largura = máx(imagem, texto).
  - Modo normal: fonte do widget, linha de base em `altura − descent`, altura = imagem + altura da fonte.
- **Styled** (sem imagem): o tamanho vem de fora (`setFixedSize` ou o layout); desenha **só o ponteiro**, sem
  arco, sem legenda.
  - Linha de `innerRadius` a `outerRadius` a partir de `centerPoint`, largura `lineWidth`, ponta redonda,
    antialias.
  - Se `outerColor` existe, a caneta é um gradiente radial centrado em `centerPoint` com raio `outerRadius`: a 0,4
    a cor `color` do CSS, a 1,0 a `outerColor`.
  - Padrões do código: `innerRadius` 1, `outerRadius` 10, `lineWidth` 0, `centerPoint` (0, 0). Cada painel define
    os valores no CSS (tabela abaixo) ou no código (`setCenterPointX/Y`, `setOuterRadius`...).
- **Interação** (F-CTL-005/006, reproduzir igual):
  - arrastar na vertical: `Δvalor = 0,4 · Δy · (max − min)/100`, ou seja, 250 px varrem a faixa; com Shift, ÷4 e
    no máximo ±4 páginas por evento; o valor anda em passos do `step`, com resto acumulado;
  - roda: 100 passos varrem a faixa; Shift 10, Ctrl 1000, Alt 2000 (no Alt, a roda horizontal também vale);
    rolagem "natural" invertida;
  - duplo clique: diálogo para digitar o valor;
  - Shift + clique: arrasta o valor (`float_value`); Ctrl + clique (`KBD_COPY_MODIFIER`): arrasta o modelo para
    ligar automação ou controlador;
  - texto flutuante com o valor à direita do knob (`largura + 2, 0`): aparece 700 ms depois de o mouse entrar e
    fica 2 s; durante o arrasto, fica visível.
- **`VolumeKnob`**: o mesmo knob, com o texto em dBFS (`-∞ dBFS`, `x.xx dBFS`, `(inverted)` para negativos).

**Styled por plugin (`style.css`)** — é aqui que mora a aparência dos knobs dos instrumentos:

| Seletor | `color` (0,4 do gradiente) | `outerColor` | inner | outer | centro (x, y) | largura |
|---|---|---|---|---|---|---|
| `TripleOscillatorView Knob` | `rgb(255,255,255)` | `rgb(255,255,255)` | 2 | 7 | 13,0; 14,0 | 2 |
| `KickerInstrumentView Knob#smallKnob` | `#595959` | `black` | 3 | 11,0 | 14,5; 14,5 | 2 |
| `KickerInstrumentView Knob#largeKnob` | `#0c3b89` | `#519fff` | 12,0 | 14,5 | 17,0; 17,0 | 3 |
| `AudioFileProcessorView Knob` | `rgb(240,147,14)` | `rgb(30,35,37)` | 4 | 11,2 | 18,5; 16,5 | 3 |
| `OrganicInstrumentView Knob` | `rgb(124,207,98)` | `rgb(13,42,4)` | 2 | 7,5 | 10,5; 10,5 | 1,5 |
| `OrganicInstrumentView Knob#harmKnob` | `rgb(205,98,216)` | `rgb(18,4,18)` | (herda) | | | |
| `OrganicInstrumentView Knob#fx1Knob, #volKnob` | `rgb(157,157,157)` | `rgb(37,37,37)` | 4 | 10,0 | 18,5; 13,8 | 2 |
| `Sf2InstrumentView Knob` | `#ff00ea` | `rgb(20,5,18)` | 2 | 9,2 | 15,5; 15,42 | 2 |
| `SfxrInstrumentView Knob` | `#000` | `rgb(194,177,145)` | 2 | 9 | (do código) | 2 |
| `Sfxr #envKnob` / `#freqKnob` / `#changeKnob` / `#sqrKnob` / `#repeatKnob` / `#phaserKnob` / `#filterKnob` | `#263352` / `#1e4a22` / `#591c1c` / `#3b2714` / `#292929` / `#144c4d` / `#47224c` | `#4b66a4` / `#3c9544` / `#b23737` / `#724c27` / `#515151` / `#299899` / `#8e4397` | (herda) | | | |
| `OpulenzInstrumentView Knob` | `rgb(128,128,128)` | `rgb(255,255,255)` | 2 | 9 | (do código) | 2 |
| `SidInstrumentView Knob` | `rgb(113,95,80)` | `rgb(255,255,255)` | 2 | 7 | (do código) | 2 |
| `SlicerTView Knob` | `rgb(162,128,226)` | `rgb(162,128,226)` | 1 | 11 | (do código) | 3 |
| `WatsynView Knob` | — | — | 1 | 7 | 9,5; 9,5 | 2 |
| `Watsyn #aKnob` / `#bKnob` / `#mixenvKnob` / `#xtalkKnob` | `#43b2ff` / `#fc5431` / `#43ff82` / `#fb50fb` | iguais à `color` | (herda) | | | |
| `Watsyn #mixKnob` | `#43ff82` | `#43ff82` | 1 | 13 | 15,5; 15,5 | 2 |
| `MonstroView Knob` | `#ffffff` | `#aaaaaa` | 6 | 9 | 10; 10 | 2,5 |
| `NesInstrumentView Knob` | `#e7231b` | `#fff` | 8,0 | 11,0 | 14,5; 14,5 | 2 |
| `CompressorControlDialog Knob` | `#2fcc71` | `#2fcc71` | (do código) | | | 2 |
| `FrequencyShifterControlDialog Knob` | `#ffffff` | `#ffffff` | (do código) | | | 3 (2 em `#fs_glide`, `#fs_dglide`, `#fs_phase`) |

"(do código)" quer dizer que o seletor não define o valor e o painel o define em C++ (ver a seção do plugin); "(herda)"
quer dizer que vale o da linha geral do mesmo painel.

### LedCheckBox (`LedCheckBox.cpp`)

- Imagens: `led_yellow.png`, `led_green.png`, `led_red.png` (aceso) e `led_off.png` (apagado), todas 16×14.
  `LedColor` só tem `Yellow`, `Green`, `Red` (existe `led_blue.png` 16×14 no tema, sem uso pelo `LedCheckBox`).
- Tamanho: `16 + 5 + largura do texto` × (altura do LED no modo legado; máx(14, altura da fonte) no normal).
- Modo legado (`legacyMode = true`): fonte de 12 px; LED em (0, 0); texto com sombra `rgb(64,64,64)` em (20, 11) e
  texto branco em (19, 10) (linhas de base).
- Modo normal: LED centrado na vertical; texto no retângulo que começa 21 px à direita, com a cor do tema
  (`#d1d8e4`).
- Clique alterna (é um `AutomatableButton` marcável). Ctrl + clique arrasta o modelo.

### PixmapButton (`PixmapButton.cpp`) e AutomatableButtonGroup (`AutomatableButton.cpp`)

- Desenha a imagem "ativa" se o modelo vale verdadeiro **ou** se está pressionado; senão a "inativa", sempre em
  (0, 0), sem escala. O tamanho do widget é o tamanho lógico da imagem ativa (`embed::logicalSize`, respeita 2x).
- Padrão sem imagens definidas: `led_yellow` / `led_off`.
- Marcável (`setCheckable(true)`): o clique alterna o modelo.
- Não marcável: botão momentâneo; o modelo vai a verdadeiro no aperto e a falso na soltura (o painel reage ao
  `clicked()`), e a imagem ativa aparece enquanto o botão está apertado.
- Em `AutomatableButtonGroup` (seletores de onda, de modo): funciona como rádio; clicar no já ativo não faz nada;
  o grupo tem um `IntModel` com o índice. Ctrl + clique arrasta o modelo do grupo.
- Duplo clique emite `doubleClicked()`.

### NStateButton (`NStateButton.cpp`)

`ToolButton` com uma lista de estados (ícone + dica); cada clique esquerdo passa ao próximo estado, em ciclo. A
dica é a do estado, ou a geral se o estado não tiver.

### LcdWidget e LcdSpinBox (`LcdWidget.cpp`, `LcdSpinBox.cpp`)

- A imagem é uma tira de 12 células (`charsPerPixmap = 12`) por 2 linhas: linha de cima = ligado, de baixo =
  desligado. Células 0–9 = dígitos, 10 = vazio, 11 = sinal de menos; o pedaço depois da 12.ª célula é a borda.
  - `lcd_19green.png` (padrão), `lcd_19red.png`, `lcd_19purple.png`, `lcd_21pink.png`: 143×38, logo célula de
    11×19 (143/12 = 11 inteiro), margem `m_marginWidth` = 5.
  - `lcd_11green.png`: 78×22, célula 6×11, margem 3.
  - Os `lcd_*_dot.png` (4×38, 6×22) são o ponto decimal do `LcdFloatSpinBox`.
- Desenho: borda esquerda (meia célula da área de margem), depois células vazias até completar `numDigits`, depois
  os dígitos (alinhados à direita), depois a borda direita (largura `margem − 1`); tudo deslocado 1 px para dentro;
  moldura `PE_Frame` afundada do estilo. Zero à esquerda opcional (`leadingZero`). Valores com texto próprio
  (`m_textForValue`) mostram o texto.
- Tamanho: `11 · numDigits + 2·(1 + 5)` × `19 + 2` (sem legenda); com legenda, altura + 9 e largura máx(…, texto
  em 12 px). Legenda centrada embaixo: sombra `rgb(64,64,64)` deslocada (+1, +1) e texto branco.
- Modo "dígitos planos" (do fork, `setFlatDigits`): a mesma grade desenhada com a fonte do widget em negrito, sobre
  `rgb(28,30,33)`; cor verde `rgb(43,212,168)`, vermelho `rgb(232,84,72)`, roxo `rgb(155,140,255)`, rosa
  `rgb(255,111,174)`; a linha desligada com alfa 110.
- **LcdSpinBox** (interação): arrastar na vertical só se o clique for na faixa dos dígitos (`y < altura da célula +
  2`): `valor −= Δy/2 · step` com resto acumulado; Shift ÷4 (máx ±4); roda ±1 `step`; duplo clique abre "Set
  value"; menu de contexto com as ações padrão.

### Graph (`Graph.cpp`)

- Desenho a mão de uma tabela (`graphModel`, padrão 128 pontos de −1 a 1); o widget tem a borda de 2 px do fundo
  (a área útil começa em (2, 2) e vai até largura−3/altura−3).
- Estilos (`Graph::Style`):
  - `Linear`: segmentos entre pontos, antialias, e o último volta ao primeiro (cíclico);
  - `LinearNonCyclic`: igual, sem o segmento de volta;
  - `Nearest`: degraus (horizontal no valor do ponto, depois vertical até o próximo), sem antialias;
  - `Bar`: uma barra por ponto do valor até o mínimo, preenchida com a cor do gráfico com alfa 100, e um traço de
    1 px no topo.
- Cor padrão `#FFAA00` (`QColor(0xFF,0xAA,0x00)`), caneta de 1 px; cada plugin troca com `setGraphColor`.
- Fundo: o widget não pinta fundo; o plugin põe a imagem pela paleta (`wavegraph.png` etc.). Depois do traço, o
  `foreground` opcional é desenhado em (0, 0).
- Durante o desenho (botão apertado) o cursor some e aparece uma cruz `#707C91` (horizontal e vertical na posição).
- Interação: clique define o ponto sob o mouse; arrastar desenha linhas entre a posição anterior e a atual (sem
  pular colunas); Shift + clique traça uma reta da última posição até a atual. Soltar emite `drawn()`.
  Aceita arrastar um arquivo de amostra (`samplefile`), sem efeito no código atual.

### ComboBox (`ComboBox.cpp`)

- Altura fixa 22 (`DEFAULT_HEIGHT`), fonte de 12 px, a largura vem do painel.
- Fundo: `combobox_bg.png` (50×20) repetido em `(2, 2, largura−2, altura−4)`; moldura `PE_Frame` do estilo.
- Botão da seta: os 18 px da direita (`CB_ARROW_BTN_WIDTH`). Divisória: linha `shadow` (preto, alfa 124) em
  `x = largura − 19` e linha `highlight` (`#262b30`, alfa 124) em `x = largura − 18`, de y 1 a altura−3. Seta
  `combobox_arrow.png` (11×15; `combobox_arrow_selected.png` quando aberta) em `(largura − 15, 4)`.
- Texto: recortado em `(4, 2, largura − 26, altura − 2)`; ícone do item (até 16 px de altura) em (5, 3) e texto
  3 px depois; linha de base `y = (altura + altura da fonte)/2`; sombra `rgb(64,64,64)` em (x+1, y−3) e texto
  `rgb(224,224,224)` em (x, y−4).
- Interação: clique na seta abre o menu com todos os itens (embaixo, ou à direita se não couber); clique no resto
  passa ao próximo item; clique direito volta ao anterior; roda para baixo = próximo.
- Modo botão (do fork, quando há ícone): retângulo arredondado de 4 px `rgb(32,36,42)` (`rgb(48,54,61)` com o
  mouse em cima), texto `#d1d8e4`, seta triangular `rgb(140,150,163)` desenhada.

### TempoSyncKnob (`TempoSyncKnob.cpp`)

Um `Knob` igual aos outros no desenho. Diferença no menu de contexto: "Set linear/logarithmic" e o submenu "Tempo
Sync" (No Sync, Eight beats, Whole, Half, Quarter, 8th, 16th, 32nd note, Custom…), com os ícones `note_*` do tema;
cada nota só aparece se couber no máximo do botão no andamento atual. Mexer no knob desliga a sincronia.

### AutomatableSlider (`AutomatableSlider.cpp`, `style.css`)

`QSlider` estilizado: trilho `#040506` de 2 px com canto de 2 px; puxador `main_slider.png` (26×10) na vertical e
`horizontal_slider.png` (10×26) na horizontal. Clique sem Ctrl arrasta; roda mostra o valor; duplo clique pede o
valor.

### GroupBox (`GroupBox.cpp`)

Caixa com barra de título de 11 px: fundo da paleta (`#262b30` no CSS), borda e barra de título no fundo
`darker(150)`; LED `led_green`/`led_off` em (3, 0) que liga/desliga o modelo (clique em `1 < y < 13`); título em
12 px na cor `text`, em x 22 (com LED) ou 6, linha de base 11.

Os `QGroupBox` dentro de `EffectControlDialog` (efeitos LADSPA e similares): fundo `#262b30`, canto 4 px,
`padding: 10px 2px 1px`, título centrado em cima.

## Instrumentos com visual próprio

### AudioFileProcessor

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:**
  - `plugins/AudioFileProcessor/AudioFileProcessorView.cpp/.h` (painel), `AudioFileProcessorWaveView.cpp/.h` (visor de onda + classe `knob`), `AudioFileProcessor.cpp/.h` (modelos).
  - Imagens (todas embutidas via `EMBEDDED_RESOURCES *.png`, todas referenciadas):
    - `plugins/AudioFileProcessor/artwork.png` 250 x 250 (fundo, desenhado em paintEvent)
    - `logo.png` 48 x 48 (descritor, `PluginPixmapLoader("logo")`)
    - `select_file.png` 20 x 18 (ativo e inativo iguais)
    - `reverse_on.png` / `reverse_off.png` 20 x 18
    - `loop_off_on.png` / `loop_off_off.png` **46 x 18**
    - `loop_on_on.png` / `loop_on_off.png` 20 x 18
    - `loop_pingpong_on.png` / `loop_pingpong_off.png` 20 x 18
    - `stutter_on.png` / `stutter_off.png` 20 x 18
    - `declick_on.png` / `declick_off.png` 20 x 18
- **Janela:** 250 x 250 (InstrumentViewFixedSize). Fundo: `artwork.png` desenhado em `paintEvent` em (0,0) (não via palette).
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| PixmapButton (`select_file` ativo/inativo), cursor PointingHand, não checável | — (abre FileDialog::openAudioFile; aceita .xpf) | (227, 72) 20x18 | tooltip "Open sample" |
| PixmapButton checável (`reverse_on`/`reverse_off`) | m_reverseModel "Reverse sample" | (164, 105) | tooltip "Reverse sample" |
| PixmapButton checável (`loop_off_on`/`loop_off_off`), no grupo de loop (índice 0) | m_loopModel "Loop mode" (0..2) | (190, 105) 46x18 | tooltip "Disable loop" |
| PixmapButton checável (`loop_on_on`/`loop_on_off`), grupo índice 1 | m_loopModel | (190, 124) | tooltip "Enable loop" |
| PixmapButton checável (`loop_pingpong_on`/`loop_pingpong_off`), grupo índice 2 | m_loopModel | (216, 124) | tooltip "Enable ping-pong loop" |
| AutomatableButtonGroup (os 3 de loop acima) | m_loopModel "Loop mode" | — | — |
| PixmapButton checável (`stutter_on`/`stutter_off`) | m_stutterModel "Stutter" | (164, 124) | tooltip "Continue sample playback across notes" |
| PixmapButton checável (`declick_on`/`declick_off`) | m_declickModel "Declick" (padrão true) | (216, 105) | tooltip "Declick: short fades at the start and the end, a crossfade where the loop wraps" |
| VolumeKnob KnobType::Bright26 | m_ampModel "Amplify" (100, 0..500) | (5, 108) | hint "Amplify:" unidade "%" |
| AudioFileProcessorWaveView::knob (Knob Bright26) | m_startPointModel "Start of sample" (0..1) | (50, 108) | hint "Start point:" "" |
| AudioFileProcessorWaveView::knob (Bright26) — visível só se loop != Off | m_loopPointModel "Loopback point" | (90, 108) | hint "Loopback point:" "" |
| AudioFileProcessorWaveView::knob (Bright26) | m_endPointModel "End of sample" | (130, 108) | hint "End point:" "" |
| ComboBox | m_interpolationModel "Interpolation mode" (itens "None", "Linear", "Sinc"; padrão 1) | setGeometry(142, 62, 82, 22) | — |
| AudioFileProcessorWaveView | (sample) | (2, 172) 245x75 | — |

  Regra de tema `lmms--gui--AudioFileProcessorView lmms--gui--Knob`: color rgb(240,147,14); outerColor rgb(30,35,37); innerRadius 4; outerRadius 11.2; centerPoint (18.5, 16.5); lineWidth 3 (as qproperties só valem para knobs Styled; os daqui são Bright26).
- **Visores vivos:**
  - `AudioFileProcessorWaveView` em (2,172), 245 x 75, padding `s_padding = 2` -> área do gráfico (2,2, 241 x 71) dentro do widget. Desenha a onda do sample via `SampleThumbnailRenderer::visualize` (pen branca #FFFFFF) num QPixmap 241x71 transparente, no intervalo [m_from, m_to] de frames (zoom/scroll), com amplificação e reverso. Atualizado por timer de 30 ms (QTimer no painel) que lê `playingFrame()` e chama `isPlaying(frame)`.
  - Interação: arrastar perto (<4 px) das linhas start/end/loop move o ponto; arrasto horizontal = desliza a janela; vertical = zoom; roda = zoom. Cursores SizeHor / OpenHand / ClosedHand.
- **Desenho próprio:**
  - Painel (`AudioFileProcessorView::paintEvent`): artwork em (0,0); nome do arquivo em fonte `SMALL_FONT_SIZE` (10 px), cor QColor(255,255,255), `drawText(8, 99, ...)` (baseline y=99). Truncado pela esquerda: acrescenta caracteres do fim enquanto largura de `nome + "..."` < 210 px; prefixa "..." se cortou.
  - WaveView `paintEvent`:
    - pixmap do gráfico em (2,2).
    - linha do loop: cor #7FFFFF (0x7F,0xFF,0xFF), vertical na altura total do gráfico.
    - linhas start e end: #FFFFFF.
    - se end-start > 2 px: preenchimento entre start+1 e end: QColor(95,175,255,50); entre loop+1 e end (se > 2 px): QColor(95,205,255,65).
    - cursor de reprodução (se frame != 0 e config `ui/animateafp` ligada): QLinearGradient horizontal de startX até played_px, stops 0 transparente, 0.8 e 1 QColor(0,120,255,180), preenchendo de start+1 até played_px; linha vertical branca em played_px.
    - faixa de título: QLinearGradient (0,0)->(width*0.7, 0), stops 0 e 0.4 QColor(16,111,170,180), 1 transparente; retângulo (2,2, 241 x 14).
    - texto "Sample length: N" em branco, fonte 10 px, em (4, 12). Formato: >20000 ms -> "Ns" inteiro; >2000 ms -> uma casa decimal "N.Ns"; senão "Nms".
- **Notas:**
  - `loop_off_*.png` tem 46 px de largura em (190,105), cobrindo x 190..235; o botão declick (20x18) em (216,105) é criado depois e fica por cima (x 216..235). A parte visível do loop_off é só 190..215 (26 px). Reproduzir a sobreposição ou cortar a imagem.
  - O knob de loop some quando o loop está em Off (upstream #7819).
  - O painel aceita drop de "samplefile" e de clipe de Sample track.

### AvlDrums

- **Classificação:** visual próprio (foto da bateria + mapa de peças, desenho próprio) ; **Categoria:** instrumento
- **Arquivos:** `plugins/AvlDrums/AvlDrums.cpp/.h` (view `AvlDrumsView`). Sem imagens na pasta do plugin; logo do descritor = `PixmapLoader("lmms-plugin-logo")` (genérico do tema). As imagens são lidas em tempo de execução de `ConfigManager::dataDir() + "avldrums/images/"`:
  - `data/avldrums/images/black_pearl.png` 1024 x 512 + `black_pearl.map.png` 1024 x 512
  - `red_zeppelin.png` + `red_zeppelin.map.png` (1024 x 512)
  - `blonde_bop.png` + `blonde_bop.map.png` (1024 x 512)
  - `blonde_bop_hr.png` 1024 x 512 (usa `blonde_bop.map.png`)
  - `buskman.png` + `buskman.map.png` (1024 x 512)
  - Todas referenciadas pela tabela `Kits`.
  - Procedência registrada: `data/avldrums/SOURCE.txt` diz que as fotos e mapas vêm da UI do avldrums.lv2 (x42, Robin Gareus; `gui/*.png.h` e `gui/*.map.h`, commit 9389f16f), repositório GPL-2.0, convertidas de arrays C para PNG sem outras mudanças. Os SF2 são de Glen MacArthur, CC-BY-SA 3.0 com exceção (`data/avldrums/README`). Licença registrada; não é caso de "sem permissão".
- **Janela:** `InstrumentView` redimensionável (não FixedSize). sizeHint 640 x 360; mínimo 420 x 234 (`ControlsHeight` 34 + 200). Fundo: autoFill, cor Window QColor(0x16,0x18,0x1b) = #16181b.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| ComboBox | m_kitModel "Drum kit" (itens: "Black Pearl", "Red Zeppelin", "Blonde Bop", "Blonde Bop Hot Rod", "Buskman's Holiday (percussion)") | setGeometry(8, 6, 230, 22) | — |
| LedCheckBox texto "Output per drum piece (9 channels)", LedColor::Green; desabilitado no kit Buskman | m_multiOutputModel "Output per drum piece" | (250, 10) | tooltip "Kick, snare, hi-hat, tom and floor tom (mono), cymbals and percussion (stereo) get their own outputs, which the pin connector can route to the track's auxiliary channels" |
| QPushButton "Mixer channel per piece" (adjustSize, NoFocus); habilitado só com kit multi-saída e multi-saída ligada | — (cria canais de mixer por peça + grupo) | (width - w - 8, 6), reposicionado em resizeEvent | tooltip "Creates a mixer channel for each drum piece and a group channel summing them, and routes the outputs there" |
| (a view inteira) | — | — | tooltip "Click a drum to play it" |

- **Visores vivos:**
  - Foto do kit: área = (0, 34, width, height-34); a foto é escalada KeepAspectRatio e centrada (`photoRect`). Em 640x360: área 640x326 -> foto 640x320 em (0, 37).
  - Realce das peças: para cada uma das 27 peças (notas 35..61), um QImage do recorte do mapa com cor QColor(255,214,120) alfa 170 (premultiplicado), desenhado com `CompositionMode_Screen` e opacidade = brilho. Mapa: vermelho R >= 10 -> índice de cor (R-10)/9 -> peça via tabela `MapPieces` {0,2,4,1,3,12,10,6,7,5,8,11,9,13,14,16,23,15,17,18,22,21,20,24,19,25}.
  - Brilho: timer de 33 ms; `takeHits()` (velocidade/127); decaimento x0.85 por tick; abaixo de 0.02 zera.
  - Clique: o pixel do mapa sob o cursor dá a nota; NoteOn com baseVelocity do MIDI port, NoteOff ao soltar.
- **Desenho próprio:** `paintEvent` com SmoothPixmapTransform; se o kit não carregou ou não há foto: texto "The drum kit could not be loaded" cor #e06050, fonte 11 px, centralizado com quebra, no photoRect. Senão `drawImage(photoRect, foto)` e os realces.
- **Notas:** as imagens não são embutidas (vêm de data/). O mapa do Blonde Bop Hot Rod reutiliza o do Blonde Bop (como no avldrums.lv2).

### BitInvader

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/BitInvader/BitInvader.cpp/.h` (view no .cpp, classe `BitInvaderView`).
  - `plugins/BitInvader/artwork.png` 250 x 250 (fundo via palette)
  - `wavegraph.png` 204 x **133** (fundo do Graph via palette)
  - `smooth_active.png` / `smooth_inactive.png` 14 x 14
  - `logo.png` 48 x 48 (descritor)
  - Do tema (`data/themes/default/`, 14x14): sin_wave, triangle_wave, saw_wave, square_wave, white_noise_wave, usr_wave (active/inactive).
  - Todas referenciadas.
- **Janela:** 250 x 250; `setAutoFillBackground(true)` + palette backgroundRole = `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Knob KnobType::Dark28 | m_sampleLength "Sample length" (200, 4..200, passo 1) | (6, 201) | hint "Sample length" "" |
| Graph (Graph::Style::Nearest), 204x134, autoFill, fundo `wavegraph.png`, graphColor QColor(255,255,255) | m_graph (-1..1, 200 amostras) | (23, 59) 204x134 (comentário: "55,120 - 2px border") | tooltip "Draw your own waveform here by dragging your mouse on this graph." |
| PixmapButton (`sin_wave_active`/`_inactive`, tema) | — (sinWaveClicked) | (131, 205) | "Sine wave" |
| PixmapButton (`triangle_wave_*`) | — | (145, 205) | "Triangle wave" |
| PixmapButton (`saw_wave_*`) | — | (159, 205) | "Saw wave" |
| PixmapButton (`square_wave_*`) | — | (173, 205) | "Square wave" |
| PixmapButton (`white_noise_wave_*`) | — | (187, 205) | "White noise" |
| PixmapButton (`usr_wave_*`) | — (abre arquivo de onda) | (201, 205) | "User-defined wave" |
| PixmapButton (`smooth_active`/`smooth_inactive`, do plugin) | — (smoothClicked) | (215, 205) | "Smooth waveform" |
| LedCheckBox texto "Interpolation", LedColor::Yellow | m_interpolation "Interpolation" | (131, 221) | — |
| LedCheckBox texto "Normalize", LedColor::Green | m_normalize "Normalize" | (131, 236) | — |
| LedCheckBox texto tr("Anti-aliasing"), LedColor::Green | m_antiAliasing "Anti-aliasing" (padrão true) | (38, 236) | tooltip "High notes without the tones their harmonics make when they fold back below half the sample rate" |

- **Visores vivos:** Graph (widget de tema) em (23,59) 204x134 desenhando a tabela de onda de 200 pontos, estilo Nearest, cor branca; editável com o mouse. Atualiza ao mudar o modelo.
- **Desenho próprio:** nenhum paintEvent próprio.
- **Notas:** os botões de onda não são grupo (não checáveis; só disparam ações). `wavegraph.png` tem 133 px de altura e o Graph 134: a palette ladrilha o pixmap, então a última linha (y=133) repete a linha 0 da imagem.

### DrumMachines

- **Classificação:** visual próprio (foto da máquina ou painel desenhado + pads desenhados) ; **Categoria:** instrumento
- **Arquivos:** `plugins/DrumMachines/DrumMachines.cpp/.h` (view `DrumMachinesView`). Logo do descritor = `PixmapLoader("lmms-plugin-logo")` (genérico). Fotos embutidas (`EMBEDDED_RESOURCES photos/*.jpg`), carregadas com `PLUGIN_NAME::getIconPixmap("photo_xxx.jpg")`:

| Arquivo (plugins/DrumMachines/photos/) | Tamanho (px) | Kit onde aparece | Crédito desenhado sobre a foto |
|---|---|---|---|
| photo_cr8000.jpg | 1920 x 1440 | CR-8000 | "Photo: Blair Rideout, CC BY 2.0" |
| photo_tr606.jpg | 1123 x 556 | TR-606 | "Photo: Midas Wouters, CC BY-SA 3.0" |
| photo_tr77.jpg | 1536 x 1152 | TR-77 | "Photo: Trouby, CC BY-SA 3.0" |
| photo_tr808.jpg | 1920 x 1121 | TR-808 | "Photo: Brandon Daniel, Clusternote, CC BY-SA 2.0" |
| photo_tr909.jpg | 1920 x 1134 | TR-909 | "Photo: Brandon Daniel, Clusternote, CC BY-SA 2.0" |
| photo_linn.jpg | 1920 x 1053 | LinnDrum | "Photo: Linn Electronics / Forat Electronics, CC BY-SA 3.0" |
| photo_simmons.jpg | 1920 x 1280 | Electro | "Simmons SDS 5. Photo: Ben Franske, CC BY-SA 2.5" |
| photo_mpc2000xl.jpg | 1920 x 1220 | R&B | "Akai MPC2000XL. Photo: withe, public domain" |
| photo_sp1200.jpg | 1920 x 1440 | SP-1200 style | "E-mu SP-1200. Photo: 2xUeL, public domain" |
| photo_mpc60.jpg | **450 x 419** | MPC60 style | "Akai MPC60. Photo: Kimi95, CC BY 3.0" |
| photo_dmx.jpg | 1920 x 1227 | DMX | "Oberheim DMX. Photo: Alison, CC BY-SA 4.0" |
| photo_tr707.jpg | 1483 x 943 | TR-707 | "Photo: Speculos, CC BY-SA 3.0" |

  Todas as 12 são referenciadas. O CR-78 não tem foto (comentário: nenhuma com licença livre): usa o painel desenhado.
  **Procedência:** `plugins/DrumMachines/photos/CREDITS.md` registra para cada foto a origem (Wikimedia Commons, com URL do arquivo), autor e licença (CC BY 2.0, CC BY-SA 2.0/2.5/3.0/4.0, CC BY 3.0, domínio público), "resized to 1920 pixels wide at most; otherwise unchanged". Ou seja, são fotos de terceiros **com licença registrada** (não há permissão direta dos autores, mas as licenças CC/PD permitem o uso). Obrigações a manter no porte: atribuição visível (já desenhada na foto) e, nas CC BY-SA, share-alike para derivadas da foto. Nenhuma foto está "sem permissão registrada".
- **Janela:** `InstrumentView` redimensionável. sizeHint 640 x 360; mínimo 420 x 234. Fundo: autoFill, Window #16181b.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| ComboBox | m_kitModel "Drum machine" (13 itens: CR-78, CR-8000, TR-606, TR-77, TR-808, TR-909, LinnDrum, Electro, R&B, SP-1200 style, MPC60 style, DMX, TR-707; padrão índice 4 = TR-808) | setGeometry(8, 6, 230, 22) | — |
| (a view inteira) | — | — | tooltip "Click a pad to play it" |
| Pads desenhados (clicáveis) | notas GM de cada peça | ver geometria | — |

- **Geometria (constantes):** ControlsHeight 34; PadsPerRow 8 (painel desenhado); PhotoPadHeight 36; PhotoPadMinWidth 44; PadGap 6; BandMargin 8.
  - `machineRect` = rect().adjusted(8, 38, -8, -8). Em 640x360: (8, 38, 624 x 314).
  - Com foto: colunas = cabem `fit = (largura-16 + 6) / 50` por fileira, limitado a [1, n]; fileiras = ceil(n/fit); colunas = ceil(n/fileiras) (igualadas; ex. 21 pads -> 11 e 10). Faixa dos pads (`padsRect`) no rodapé da máquina, altura = fileiras*36 + (fileiras-1)*6 + 16. Foto (`photoRect`) = do topo da máquina até padsRect.top - 6. Pads: área = padsRect encolhido 8; largura = (área.w - 6*(colunas-1)) / colunas; altura 36; centrados verticalmente.
    - Exemplo TR-808 (15 pads) em 640x360: fit 12 -> 2 fileiras, 8 colunas; padsRect (8, 258, 624 x 94); photoRect (8, 38, 624 x 214); pads 70 x 36 em x = 16 + 76*c, y = 266 (fileira 0, 8 pads) e 308 (fileira 1, 7 pads).
  - Sem foto (CR-78, 11 pads): colunas = min(n, 8), área = machineRect.adjusted(18, 64, -18, -22), gap 8, pads quadrados (altura = min(largura, (área.h - 8*(fileiras-1))/fileiras)). Em 640x360: área (26, 102, 588 x 228); pads 66 x 66 em x = 26 + 74*c, y = 146 e 220.
- **Visores vivos:** brilho dos pads por timer de 33 ms (`takeHits`, velocidade/127, decaimento x0.85, corte em 0.02). Clique num pad: NoteOn (baseVelocity) e NoteOff ao soltar.
- **Desenho próprio (`paintEvent`, Antialiasing):**
  - Sem kit carregado: texto "The drum machine could not be loaded", cor #e06050, centrado em machineRect.
  - Corpo: rounded rect (raio 10) em machineRect, QLinearGradient vertical de body.lighter(118) a body.darker(125); contorno body.darker(160), 1.5 px.
  - Com foto: recortada em rounded rect raio 10 no photoRect, escala = max(w/srcW, h/srcH), recorte central (cover). Se a escala > 1.5 (foto pequena, caso do MPC60 em telas grandes): fundo = foto reduzida a 24x24 e ampliada a 240x240 (desfoque) esticada no retângulo, escurecida com QColor(0,0,0,90), e a foto em si encaixada em 92% do retângulo, no máximo 1.5x seu tamanho, centrada horizontalmente, topo = max(4, centrado). Contorno QColor(0,0,0,160) 1.5 px. Crédito: fonte 9 px, pílula QColor(0,0,0,140) raio 4, largura = texto + 10, altura = altura da fonte + 4, no canto inferior direito (right-8, bottom - altura - 10), texto #eeeeee centrado.
  - Sem foto (painel): rounded rect raio 6 em machineRect.adjusted(12,12,-12,-12) com look.panel; faixa de acento (left+6, top+40, w-12, 3) com look.accent; nome do kit em fonte 22 px negrito, letterSpacing absoluto 1.5, cor look.text, em (left+14, top+6, w-28, 32) alinhado à esquerda; "DRUM MACHINE" em 10 px, cor look.text.darker(130), mesmo retângulo alinhado à direita.
  - Pads: rounded rect raio 5; cor = look.pads[i % 4]. Se brilho > 0: 3 halos (ring 3..1) rounded rect raio 7 expandidos 2*ring, cor color.lighter(140) com alfa 0.18*brilho/ring. Preenchimento: gradiente vertical de (lit ou color).lighter(112) a (lit ou color.darker(125)), onde lit = cor + (1-cor)*0.6*brilho por canal. Contorno color.darker(170) 1 px. Tinta: #1e1e1e se lightness > 0.55, senão #f2f2f2. Rótulo da peça em 13 px negrito centrado na parte de cima (pad sem os 25% de baixo); número da nota em 9 px, centralizado embaixo (pad.adjusted(0, h*0.5, 0, -4), AlignBottom).
- **Cores por kit (Look: body, panel, accent, text; pads[4]):**

| Kit | body | panel | accent | text | pads |
|---|---|---|---|---|---|
| CR-78 | #6b4226 | #1d1d1d | #e06b2a | #eee6d2 | #e8dfc8, #e06b2a, #e8dfc8, #9a9a9a |
| CR-8000 | #3a3a3a | #242424 | #d9a227 | #e6e6e6 | #d9a227, #cfcfcf, #8cb4d8, #cfcfcf |
| TR-606 | #c9ccd1 | #2b2b2b | #c8372d | #202020 | #c8372d, #d8d8d8, #c8372d, #d8d8d8 |
| TR-77 | #5a3b26 | #e8dcc0 | #3b6fb6 | #222222 | #3b6fb6, #f4eee0, #3b6fb6, #f4eee0 |
| TR-808 | #4a4a46 | #2f2f2c | #ec7a1c | #e8e3d3 | #d8322b, #ec7a1c, #e9c43a, #e8e3d3 |
| TR-909 | #b9bcbe | #3a3a3a | #f08a24 | #1e1e1e | #f08a24, #e2e2e2, #e2e2e2, #e2e2e2 |
| LinnDrum | #1b1b1b | #2a2a2a | #c9a227 | #e6e1d5 | #e6e1d5, #c9a227, #e6e1d5, #9c9c9c |
| Electro | #20242a | #12151a | #2fd1c5 | #d8f4f1 | #2fd1c5, #d64fc8, #2fd1c5, #6c7ae0 |
| R&B | #2b2135 | #1a1420 | #d65db1 | #f0e0ec | #d65db1, #e8c56a, #d65db1, #b9a8d8 |
| SP-1200 style | #5a5d62 | #46494e | #d03030 | #ececec | #2b2b2b, #e9e7e1, #2b2b2b, #d03030 |
| MPC60 style | #c9c9c4 | #8e8e8a | #c82b25 | #222222 | #9b9d9f, #b3b5b7, #9b9d9f, #7f8183 |
| DMX | #161618 | #2c2d30 | #4f8fd6 | #e8e8e8 | #e8e8e8, #4f8fd6, #e8e8e8, #9a9ca0 |
| TR-707 | #c8c6c0 | #a9a7a1 | #e86a2c | #2a2a2a | #4a4a4a, #e86a2c, #4a4a4a, #6e6e6e |

  (panel/accent/text só aparecem no painel desenhado, ou seja, só no CR-78; body aparece em todos, na faixa dos pads.)
- **Pads por kit (rótulo -> nota GM):**
  - CR-78 (11): BD 36, SD 38, RS 37, HH 42, CY 49, HB 60, LB 61, CG 63, CL 75, MA 70, TB 54.
  - CR-8000 (11): BD 36, SD 38, RS 37, CP 39, CH 42, OH 46, CY 49, CB 56, CL 75, HC 63, LC 64.
  - TR-606 (7): BD 36, SD 38, CH 42, OH 46, CY 49, LT 45, HT 50.
  - TR-77 (8): BD 36, SD 38, HH 42, CY 49, HT 50, LT 45, C1 78, C2 79.
  - TR-808 (15): BD 36, BD2 35, RS 37, SD 38, CP 39, SD2 40, CH 42, OH 46, LT 45, MT 47, HT 50, RD 51, CB 56, CL 75, BDL 82.
  - TR-909 (13): BD 36, BD2 35, SD 38, SD2 40, CP 39, CH 42, PH 44, OH 46, OH2 82, BD3 83, BD4 84, BD5 85, CP2 86.
  - LinnDrum (16): BD 36, ST 37, SD 38, CP 39, T5 43, T4 45, T3 47, T2 48, T1 50, TB 54, CB 56, CM 62, C1 63, C2 64, CA 69, C3 82.
  - Electro (14): BD 36, BD2 35, SD 38, SD2 40, BM 41, CH 42, OH 46, LT 45, HT 50, BDR 82, SDR 83, SEQ 84, HQ 85, HQ2 86.
  - R&B (15): BD 36, BD2 35, SD 38, CP 39, SD2 40, CH 42, OH 46, BDM 82, BDO 83, CPF 84, SDP 85, SDS 86, S+H 87, REV 88, HR 89.
  - SP-1200 style (13): BD 36, BD2 35, RS 37, SD 38, CP 39, SD2 40, CH 42, PH 44, OH 46, SH 70, WB 76, BDL 82, SDL 83.
  - MPC60 style (13): BD 36, BD2 35, RS 37, SD 38, CP 39, SD2 40, CH 42, PH 44, OH 46, CA 69, SH 70, SD3 82, BD3 83.
  - DMX (21): B1 36, B2 35, RIM 37, S1 38, CLP 39, S2 40, T6 41, HC 42, T5 43, HA 44, T4 45, HO 46, T3 47, T2 48, CR 49, T1 50, RD 51, TB 54, SH 70, B3 82, S3 83.
  - TR-707 (15): BD1 36, BD2 35, RIM 37, SD1 38, HCP 39, SD2 40, CH 42, LT 45, OH 46, MT 47, CR 49, HT 50, RD 51, TB 54, CB 56.
- **Notas:** a foto só aparece depois que o kit terminou de carregar (`hasPhoto()` compara com `loadedKit()`); enquanto carrega, a view mostra o kit anterior. A foto do MPC60 (450x419) é a única pequena o bastante para cair no modo "fundo desfocado + foto centrada". No tamanho padrão 640x360 a escala é 624/450 = 1.39 (< 1.5), então ainda sai o recorte normal. O modo desfocado entra quando a escala passa de 1.5: view mais larga que ~691 px, ou área da foto mais alta que ~628 px.

### FreeBoy

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/FreeBoy/FreeBoy.cpp/.h` (view `FreeBoyInstrumentView`, knob `FreeBoyKnob`).
  - `plugins/FreeBoy/artwork.png` 250 x 250 (palette)
  - `btn_on.png`, `btn_off.png`, `btn_up.png`, `btn_down.png`, `btn_7.png`, `btn_15.png`: todos 37 x 13
  - `logo.png` 48 x 48
  - Todas referenciadas.
- **Janela:** 250 x 250; palette backgroundRole = `artwork.png`.
- **FreeBoyKnob:** Knob KnobType::Styled, setFixedSize(30,30), centerPoint (15,15), innerRadius 8, outerRadius 13, totalAngle 270, lineWidth 1, outerColor #F1FF93. (cor do ponteiro não definida; não há regra de tema para FreeBoyInstrumentView.)
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| FreeBoyKnob | m_so1VolumeModel "Right output level" (7, 0..7) | (5, 58) | hint "SO1 volume (Right):"; tooltip "SO1 volume (Right)" |
| FreeBoyKnob | m_so2VolumeModel "Left output level" | (37, 58) | hint "SO2 volume (Left):"; tooltip "SO2 volume (Left)" |
| FreeBoyKnob | m_trebleModel "Treble" (-20, -100..200) | (69, 58) | hint "Treble:"; tooltip "Treble" |
| FreeBoyKnob | m_bassModel "Bass" (461, -1..600) | (101, 58) | hint "Bass:"; tooltip "Bass" |
| PixmapButton checável (`btn_on`/`btn_off`) | m_ch1So2Model "Channel 1 to SO2 (Left)" | (148, 51) | "Channel 1 to SO2 (Left)" |
| idem | m_ch2So2Model "Channel 2 to SO2 (Left)" | (148, 63) | "Channel 2 to SO2 (Left)" |
| idem | m_ch3So2Model "Channel 3 to SO2 (Left)" | (148, 75) | "Channel 3 to SO2 (Left)" |
| idem (setChecked(false)) | m_ch4So2Model "Channel 4 to SO2 (Left)" (padrão false) | (148, 87) | "Channel 4 to SO2 (Left)" |
| idem | m_ch1So1Model "Channel 1 to SO1 (Right)" | (208, 51) | "Channel 1 to SO1 (Right)" |
| idem | m_ch2So1Model "Channel 2 to SO1 (Right)" | (208, 63) | "Channel 2 to SO1 (Right)" |
| idem | m_ch3So1Model "Channel 3 to SO1 (Right)" | (208, 75) | "Channel 3 to SO1 (Right)" |
| idem (setChecked(false)) | m_ch4So1Model "Channel 4 to SO1 (Right)" (padrão false) | (208, 87) | "Channel 4 to SO1 (Right)" |
| FreeBoyKnob | m_ch1VolumeModel "Channel 1 volume" (15, 0..15) | (5, 106) | hint "Square channel 1 volume:"; tooltip idem sem ":" |
| FreeBoyKnob | m_ch1SweepStepLengthModel "Length of each step in sweep" (0..7) | (37, 106) | hint "Length of each step in sweep:" |
| FreeBoyKnob | m_ch1WavePatternDutyModel "Wave pattern duty cycle" (2, 0..3) | (69, 106) | hint "Wave pattern duty cycle:" |
| FreeBoyKnob | m_ch1SweepRtShiftModel "Sweep rate shift amount" (4, 0..7) | (101, 106) | hint "Sweep rate shift amount:" |
| FreeBoyKnob | m_ch1SweepTimeModel "Sweep time" (4, 0..7) | (133, 106) | hint "Sweep time:" |
| PixmapButton checável (ativo `btn_down`, inativo `btn_up`) | m_ch1SweepDirModel "Sweep direction" | (167, 108) | "Sweep direction" |
| PixmapButton checável (ativo `btn_up`, inativo `btn_down`) | m_ch1VolSweepDirModel "Volume sweep direction" | (207, 108) | "Volume sweep direction" |
| FreeBoyKnob | m_ch2VolumeModel "Channel 2 volume" | (5, 155) | hint "Square channel 2 volume:" |
| FreeBoyKnob | m_ch2SweepStepLengthModel "Length of each step in sweep" | (37, 155) | hint "Length of each step in sweep:" |
| FreeBoyKnob | m_ch2WavePatternDutyModel "Wave pattern duty cycle" | (69, 155) | hint "Wave pattern duty cycle:" |
| PixmapButton checável (ativo `btn_up`, inativo `btn_down`) | m_ch2VolSweepDirModel "Volume sweep direction" | (102, 156) | "Volume sweep direction" |
| FreeBoyKnob | m_ch4VolumeModel "Channel 4 volume" (15, 0..15) | (144, 155) | hint "Noise channel volume:" |
| FreeBoyKnob | m_ch4SweepStepLengthModel "Length of each step in sweep" | (176, 155) | hint "Length of each step in sweep:" |
| PixmapButton checável (ativo `btn_up`, inativo `btn_down`) | m_ch4VolSweepDirModel "Volume sweep direction" | (207, 157) | "Volume sweep direction" |
| PixmapButton checável (ativo `btn_7`, inativo `btn_15`) | m_ch4ShiftRegWidthModel "Shift Register width" | (207, 171) | "Shift register width" |
| FreeBoyKnob | m_ch3VolumeModel "Channel 3 volume" (3, 0..3) | (5, 204) | hint "Wave pattern channel volume:" |
| Graph (Style::Nearest, graphColor #4E832B) | m_graphModel (0..15, 32 amostras, inteiro) | (37, 199) resize 208x47 | tooltip "Wave pattern graph" |

  Todos os FreeBoyKnob têm setToolTip igual ao hint sem os dois-pontos.
- **Visores vivos:** Graph em (37,199) 208x47: tabela de onda do canal 3 (32 passos, 0..15), estilo Nearest, cor #4E832B, editável. Sem fundo próprio (sem autoFill/palette): fica sobre a artwork.
- **Desenho próprio:** nenhum.
- **Notas:** canal 3 on/off comentado (havia botão em (176,53)). m_ch2VolSweepDirButton e m_ch4VolSweepDirButton recebem nome tr("Volume sweep direction") no construtor; os outros botões têm nome nullptr.

### GigPlayer

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/GigPlayer/GigPlayer.cpp/.h` (view `GigInstrumentView`, knob `gigKnob`), `PatchesDialog.cpp/.h/.ui` (diálogo).
  - `plugins/GigPlayer/artwork.png` 250 x 250 (palette)
  - `fileselect_on.png` / `fileselect_off.png` 18 x 14
  - `patches_on.png` / `patches_off.png` 18 x 14
  - `logo.png` 48 x 48
  - Tema: `data/themes/default/lcd_21pink.png` 143 x 38 (LcdSpinBox "21pink"). Não existe `lcd_21pink_dot.png` no tema (só `lcd_19pink_dot.png`).
  - Todas referenciadas.
- **Janela:** 250 x 250; palette backgroundRole = `artwork.png` (aplicada no fim do construtor).
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| PixmapButton (`fileselect_on`/`fileselect_off`), cursor PointingHand | — (showFileDialog, filtro "GIG Files (*.gig)") | (223, 68) | tooltip "Open GIG file" |
| PixmapButton (`patches_on`/`patches_off`), cursor PointingHand, desabilitado sem arquivo | — (showPatchDialog) | (223, 94) | tooltip "Choose patch" |
| LcdSpinBox(3 dígitos, "21pink") | m_bankNum "Bank" (0..999) | (111, 150) | — |
| LcdSpinBox(3, "21pink") | m_patchNum "Patch" (0..127) | (161, 150) | — |
| QLabel (nome do arquivo, sem ".gig", elidido à esquerda) | — | setGeometry(61, 70, 156, 14) | — |
| QLabel (nome do patch, elidido à esquerda) | — | setGeometry(61, 94, 156, 14) | — |
| gigKnob = Knob KnobType::Bright26, setFixedSize(31, 38) | m_gain "Gain" (1.0, 0..5, passo 0.01) | (32, 140) | hint "Gain: " (com espaço) "" |

- **Visores vivos:** só as duas QLabels (texto); fonte padrão do widget (não há regra de tema para GigInstrumentView; a regra `> QLabel { font-size:10px }` é só do Sf2InstrumentView). Cor: palette padrão.
- **Desenho próprio:** nenhum paintEvent.
- **Notas:** PatchesDialog (só registro): QDialog modal gerado de `PatchesDialog.ui`, título "<nome da trilha> - GIG patches" (no .ui "Qsynth: Channel Preset"), tamanho do .ui 480x350 (mínimo 300x150); QSplitter com duas QTreeWidget (bancos: coluna "Bank"; programas: colunas "Patch", "Name") e botões OK / Cancel. Não usa imagens.

### Kicker

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/Kicker/Kicker.cpp/.h` (view `KickerInstrumentView`; classes `KickerKnob`, `KickerEnvKnob`, `KickerLargeKnob`), `KickerOsc.h` (DSP).
  - `plugins/Kicker/artwork.png` 250 x 250 (palette)
  - `logo.png` 48 x 48
- **Janela:** 250 x 250; palette backgroundRole = `artwork.png`.
- **Constantes:** ROW1=14, ROW2=70, ROW3=126, LED_ROW=63, COL1=14, COL2=70, COL3=126, COL4=167, COL5=208, END_COL=62.
- **Tipos de knob:**
  - `KickerKnob`: Knob Styled, 29x29, objectName "smallKnob".
  - `KickerEnvKnob`: TempoSyncKnob Styled, 29x29, objectName "smallKnob".
  - `KickerLargeKnob`: Knob Styled, 34x34, objectName "largeKnob".
  - Tema `Knob#smallKnob`: color #595959; outerColor black; innerRadius 3; outerRadius 11.0; centerPoint (14.5, 14.5); lineWidth 2.
  - Tema `Knob#largeKnob`: color #0c3b89; outerColor #519fff; innerRadius 12.0; outerRadius 14.5; centerPoint (17.0, 17.0); lineWidth 3.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| KickerLargeKnob | m_startFreqModel "Start frequency" (150, 5..1000) | (14, 14) 34x34 | hint "Start frequency:" "Hz" |
| KickerLargeKnob | m_endFreqModel "End frequency" (40, 5..1000) | (62, 14) 34x34 | hint "End frequency:" "Hz" |
| KickerKnob | m_slopeModel "Frequency slope" | (126, 14) | hint "Frequency slope:" "" |
| KickerKnob | m_clickModel "Click" | (208, 14) | hint "Click:" "" |
| LedCheckBox texto "", LedColor::Green | m_startNoteModel "Start from note" (padrão true) | (22, 63) | — |
| LedCheckBox texto "", Green | m_endNoteModel "End to note" | (70, 63) | — |
| KickerKnob | m_distModel "Start distortion" | (167, 70) | hint "Start distortion:" "" |
| KickerKnob | m_distEndModel "End distortion" | (208, 70) | hint "End distortion:" "" |
| KickerKnob | m_gainModel "Gain" | (14, 126) | hint "Gain:" "" |
| KickerEnvKnob (TempoSyncKnob) | m_decayModel "Length" (440, 5..5000) | (70, 126) | hint "Envelope length:" "ms" |
| KickerKnob | m_envModel "Envelope slope" | (126, 126) | hint "Envelope slope:" "" |
| KickerKnob | m_attackModel "Attack" (1, 0.1..20) | (169, 126) (COL4 + 2) | hint "Attack:" "ms" |
| KickerKnob | m_noiseModel "Noise" | (208, 126) | hint "Noise:" "" |

- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** m_versionModel (sem view). A linha ROW2 em COL1..COL3 fica vazia (só os LEDs em y=63). O knob de Attack (fork) está em COL4+2.

### Lb302

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/Lb302/Lb302.cpp/.h` (view `Lb302SynthView`), `README` (bugs antigos, sem informação de imagens).
  - `plugins/Lb302/artwork.png` 250 x 250 (palette, `static` pixmap)
  - `logo.png` 48 x 48
  - Tema (14x14): saw_wave, triangle_wave, square_wave, round_square_wave, moog_saw_wave, sin_wave, exp_wave, white_noise_wave (active/inactive).
- **Janela:** 250 x 250; palette backgroundRole = `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Knob Bright26 | m_vcfCutKnob "VCF Cutoff Frequency" (0.75, 0..1.5) | (75, 130) | hint "Cutoff Freq:" "" |
| Knob Bright26 | m_vcfResKnob "VCF Resonance" (0.75, 0..1.25) | (120, 130) | hint "Resonance:" "" |
| Knob Bright26 | m_vcfModKnob "VCF Envelope Mod" (0.1) | (165, 130) | hint "Env Mod:" "" |
| Knob Bright26 | m_vcfDecKnob "VCF Envelope Decay" (0.1) | (210, 130) | hint "Decay:" "" |
| Knob Bright26 | m_slideDecKnob "Slide Decay" (0.6) | (210, 75) | hint "Slide Decay:" "" |
| Knob Bright26 | m_distKnob "Distortion" (0) | (210, 190) | hint "DIST:" "" |
| LedCheckBox "" (Yellow, padrão) | m_db24Toggle "24dB/oct Filter" | (10, 150) | tooltip "303-es-que, 24dB/octave, 3 pole filter" |
| LedCheckBox "" (Yellow) | m_slideToggle "Slide" | (10, 180) | — |
| LedCheckBox "" (Yellow) | m_deadToggle "Dead" | (10, 200) | — |
| AutomatableButtonGroup m_waveBtnGrp | m_waveShape "Waveform" (8, 0..11) | — | — |
| PixmapButton `saw_wave_*` (grupo idx 0) | m_waveShape | (10, 96) | "Click here for a sawtooth wave." |
| PixmapButton `triangle_wave_*` (1) | m_waveShape | (26, 96) | "Click here for a triangle wave." |
| PixmapButton `square_wave_*` (2) | m_waveShape | (42, 96) | "Click here for a square wave." |
| PixmapButton `round_square_wave_*` (3) | m_waveShape | (58, 96) | "Click here for a square wave with a rounded end." |
| PixmapButton `moog_saw_wave_*` (4) | m_waveShape | (74, 96) | "Click here for a moog-like wave." |
| PixmapButton `sin_wave_*` (5) | m_waveShape | (90, 96) | "Click for a sine wave." |
| PixmapButton `exp_wave_*` (6) | m_waveShape | (106, 96) | "Click here for an exponential wave." |
| PixmapButton `white_noise_wave_*` (7) | m_waveShape | (122, 96) | "Click here for white noise." |
| PixmapButton `saw_wave_*` (8, bandlimited) | m_waveShape | (146, 96) | "Click here for bandlimited sawtooth wave." |
| PixmapButton `square_wave_*` (9, BL) | m_waveShape | (162, 96) | "Click here for bandlimited square wave." |
| PixmapButton `triangle_wave_*` (10, BL) | m_waveShape | (178, 96) | "Click here for bandlimited triangle wave." |
| PixmapButton `moog_saw_wave_*` (11, BL) | m_waveShape | (194, 96) | "Click here for bandlimited moog-like wave." |

  (x = 10 + 16*i para i=0..7; x = 10 + 16*i - 8 para i=9..12.)
- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** m_accentToggle existe no modelo ("Accent") mas o LED está comentado (estaria em (10,200), hoje ocupado por Dead). O botão exponencial recebe nome tr("White noise wave") (erro antigo de nome; o tooltip está certo). Padrão da onda: 8 = bandlimited saw.

### Monstro

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/Monstro/Monstro.cpp/.h` (view `MonstroView`; constantes de layout em Monstro.h linhas 52-91).
  - `artwork_op.png` 250 x 250 (fundo da sub-view Operadores), `artwork_mat.png` 250 x 250 (fundo da Matriz)
  - `opview_active.png` / `opview_inactive.png` 125 x 16; `matview_active.png` / `matview_inactive.png` 125 x 16
  - `tinyled_on.png` / `tinyled_off.png` 15 x 8
  - `mix_active/inactive.png`, `am_active/inactive.png`, `fm_active/inactive.png`, `pm_active/inactive.png`: 20 x 15
  - Ícones dos itens das ComboBox (PluginPixmapLoader): `sin.png` 15x17, `tri.png` 15x17, `saw.png` 15x17, `ramp.png` 15x16, `sqr.png` 15x17, `moog.png` 15x17, `sqrsoft.png` 16x17, `sinabs.png` 15x17, `exp.png` 15x17, `noise.png` 17x17, `rand.png` 13x15
  - `logo.png` 48 x 48
  - Todas referenciadas.
- **Janela:** 250 x 250. Duas sub-views QWidget 250x250 em (0,0), cada uma com autoFill + palette (`artwork_op` / `artwork_mat`). Os dois botões de aba são filhos do MonstroView, criados depois das sub-views (ficam por cima de ambas).
- **Constantes resolvidas:** O1ROW=22, O2ROW=61, O3ROW=100, LFOROW=139, E1ROW=178, E2ROW=217; KNOBCOL1..7 = 16, 46, 76, 106, 136, 166, 196; LFOCOL1..3 = 46, 72, 98; LFOCOL4..6 = 171, 197, 223; MATCOL1..4 = 32, 57, 82, 107; MATCOL5..8 = 149, 174, 199, 224; MATROW1..6 = 22, 61, 100, 139, 178, 217.
- **Knobs (makeKnob):** `T(KnobType::Styled)`, setFixedSize(20, 20), objectName por grupo ("osc1Knob", "osc2Knob", "osc3Knob", "lfoKnob", "envKnob", "matrixKnob"; nenhuma regra de tema por nome). Tema `MonstroView Knob`: color #ffffff; outerColor #aaaaaa; outerRadius 9; innerRadius 6; centerPoint (10, 10); lineWidth 2.5.
- **Abas (no MonstroView):**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| PixmapButton (`opview_active`/`opview_inactive`), grupo idx 0 | m_selectedView "Selected view" (0..1) | (0, 0) 125x16 | "Operators view" |
| PixmapButton (`matview_active`/`matview_inactive`), grupo idx 1 | m_selectedView | (125, 0) 125x16 | "Matrix view" |

#### Monstro: sub-view Operadores (`artwork_op.png`)

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| VolumeKnob Styled 20x20 | m_osc1Vol "Osc 1 volume" (10, 0..200) | (16, 22) | "Volume" "%" |
| Knob | m_osc1Pan "Osc 1 panning" | (46, 22) | "Panning" "" |
| Knob | m_osc1Crs "Osc 1 coarse detune" (-24..24) | (76, 22) | "Coarse detune" " semitones" |
| Knob | m_osc1Ftl "Osc 1 fine detune left" | (106, 22) | "Fine tune left" " cents" |
| Knob | m_osc1Ftr "Osc 1 fine detune right" | (136, 22) | "Fine tune right" " cents" |
| Knob | m_osc1Spo "Osc 1 stereo phase offset" | (166, 22) | "Stereo phase offset" " deg" |
| Knob | m_osc1Pw "Osc 1 pulse width" (50, 0.25..99.75) | (196, 22) | "Pulse width" "%" |
| PixmapButton checável (`tinyled_on`/`off`) | m_osc1SSR "Osc 1 sync send on rise" | (230, 34) | "Send sync on pulse rise" |
| idem | m_osc1SSF "Osc 1 sync send on fall" | (230, 44) | "Send sync on pulse fall" |
| VolumeKnob | m_osc2Vol "Osc 2 volume" | (16, 61) | "Volume" "%" |
| Knob | m_osc2Pan "Osc 2 panning" | (46, 61) | "Panning" |
| Knob | m_osc2Crs "Osc 2 coarse detune" | (76, 61) | "Coarse detune" " semitones" |
| Knob | m_osc2Ftl "Osc 2 fine detune left" | (106, 61) | "Fine tune left" " cents" |
| Knob | m_osc2Ftr "Osc 2 fine detune right" | (136, 61) | "Fine tune right" " cents" |
| Knob | m_osc2Spo "Osc 2 stereo phase offset" | (166, 61) | "Stereo phase offset" " deg" |
| ComboBox (itens com ícone, setwavemodel) | m_osc2Wave "Osc 2 waveform" | setGeometry(204, 68, 42, 22) | — |
| tinyled | m_osc2SyncR "Osc 2 sync reverse" | (191, 58) | "Reverse sync oscillator 2" |
| tinyled | m_osc2SyncH "Osc 2 sync hard" | (212, 58) | "Hard sync oscillator 2" |
| VolumeKnob | m_osc3Vol "Osc 3 volume" | (16, 100) | "Volume" "%" |
| Knob | m_osc3Pan "Osc 3 panning" | (46, 100) | "Panning" |
| Knob | m_osc3Crs "Osc 3 coarse detune" | (76, 100) | "Coarse detune" " semitones" |
| Knob | m_osc3Spo "Osc 3 Stereo phase offset" | (106, 100) | "Stereo phase offset" " deg" |
| Knob | m_osc3Sub "Osc 3 sub-oscillator mix" (-100..100) | (136, 100) | "Sub-osc mix" "" |
| ComboBox (setwavemodel) | m_osc3Wave1 "Osc 3 waveform 1" | setGeometry(160, 107, 42, 22) | — |
| ComboBox (setwavemodel) | m_osc3Wave2 "Osc 3 waveform 2" | setGeometry(204, 107, 42, 22) | — |
| tinyled | m_osc3SyncR "Osc 3 Sync reverse" | (191, 97) | "Reverse sync oscillator 3" |
| tinyled | m_osc3SyncH "Osc 3 sync hard" | (212, 97) | "Hard sync oscillator 3" |
| ComboBox (setlfowavemodel) | m_lfo1Wave "LFO 1 waveform" | setGeometry(2, 146, 42, 22) | — |
| TempoSyncKnob Styled 20x20 | m_lfo1Att "LFO 1 attack" (0..2000) | (46, 139) | "Attack" " ms" |
| TempoSyncKnob | m_lfo1Rate "LFO 1 rate" (1, 0.1..10000) | (72, 139) | "Rate" " ms" |
| Knob | m_lfo1Phs "LFO 1 phase" | (98, 139) | "Phase" " deg" |
| ComboBox (setlfowavemodel) | m_lfo2Wave "LFO 2 waveform" | setGeometry(127, 146, 42, 22) | — |
| TempoSyncKnob | m_lfo2Att "LFO 2 attack" | (171, 139) | "Attack" " ms" |
| TempoSyncKnob | m_lfo2Rate "LFO 2 rate" | (197, 139) | "Rate" " ms" |
| Knob | m_lfo2Phs "LFO 2 phase" | (223, 139) | "Phase" " deg" |
| TempoSyncKnob | m_env1Pre "Env 1 pre-delay" | (16, 178) | "Pre-delay" " ms" |
| TempoSyncKnob | m_env1Att "Env 1 attack" | (46, 178) | "Attack" " ms" |
| TempoSyncKnob | m_env1Hold "Env 1 hold" | (76, 178) | "Hold" " ms" |
| TempoSyncKnob | m_env1Dec "Env 1 decay" | (106, 178) | "Decay" " ms" |
| Knob | m_env1Sus "Env 1 sustain" (1, 0..1) | (136, 178) | "Sustain" "" |
| TempoSyncKnob | m_env1Rel "Env 1 release" | (166, 178) | "Release" " ms" |
| Knob | m_env1Slope "Env 1 slope" (-1..1) | (196, 178) | "Slope" "" |
| TempoSyncKnob | m_env2Pre "Env 2 pre-delay" | (16, 217) | "Pre-delay" " ms" |
| TempoSyncKnob | m_env2Att "Env 2 attack" | (46, 217) | "Attack" " ms" |
| TempoSyncKnob | m_env2Hold "Env 2 hold" | (76, 217) | "Hold" " ms" |
| TempoSyncKnob | m_env2Dec "Env 2 decay" | (106, 217) | "Decay" " ms" |
| Knob | m_env2Sus "Env 2 sustain" | (136, 217) | "Sustain" "" |
| TempoSyncKnob | m_env2Rel "Env 2 release" | (166, 217) | "Release" " ms" |
| Knob | m_env2Slope "Env 2 slope" | (196, 217) | "Slope" "" |
| PixmapButton (`mix_active`/`mix_inactive`), grupo idx 0 | m_o23Mod "Osc 2+3 modulation" (0..3) | (225, 185) | "Mix osc 2 with osc 3" |
| PixmapButton (`am_*`), idx 1 | m_o23Mod | (225, 200) | "Modulate amplitude of osc 3 by osc 2" |
| PixmapButton (`fm_*`), idx 2 | m_o23Mod | (225, 215) | "Modulate frequency of osc 3 by osc 2" |
| PixmapButton (`pm_*`), idx 3 | m_o23Mod | (225, 230) | "Modulate phase of osc 3 by osc 2" |

Itens das ComboBox:
- `setwavemodel` (osc2, osc3 w1/w2), 15 itens: "Sine wave" (sin), "Bandlimited Triangle wave" (tri), "Bandlimited Saw wave" (saw), "Bandlimited Ramp wave" (ramp), "Bandlimited Square wave" (sqr), "Bandlimited Moog saw wave" (moog), "Soft square wave" (sqrsoft), "Absolute sine wave" (sinabs), "Exponential wave" (exp), "White noise" (noise), "Digital Triangle wave" (tri), "Digital Saw wave" (saw), "Digital Ramp wave" (ramp), "Digital Square wave" (sqr), "Digital Moog saw wave" (moog).
- `setlfowavemodel` (LFO 1/2), 11 itens: "Sine wave" (sin), "Triangle wave" (tri), "Saw wave" (saw), "Ramp wave" (ramp), "Square wave" (sqr), "Moog saw wave" (moog), "Soft square wave" (sqrsoft), "Abs. sine wave" (sinabs), "Exponential wave" (exp), "Random" (rand), "Random smooth" (rand).

#### Monstro: sub-view Matriz (`artwork_mat.png`)

Todos: Knob Styled 20x20, objectName "matrixKnob", hint "Modulation amount" "", modelos -1..1 (padrão 0). Colunas por fonte: env1, env2, lfo1, lfo2.

| Linha (y) | Cols 32 / 57 / 82 / 107 (env1, env2, lfo1, lfo2) | Cols 149 / 174 / 199 / 224 (env1, env2, lfo1, lfo2) |
|---|---|---|
| MATROW1 = 22 | m_vol1env1/env2/lfo1/lfo2 "Osc 1 - Vol env 1" ... "Osc 1 - Vol LFO 2" | m_pit1env1..lfo2 "Osc 1 - Pit env 1" ... |
| MATROW2 = 61 | m_phs1* "Osc 1 - Phs ..." | m_pw1* "Osc 1 - PW env 1" ... |
| MATROW3 = 100 | m_vol2* "Osc 2 - Vol ..." | m_pit2* "Osc 2 - Pit ..." |
| MATROW4 = 139 | m_phs2* "Osc 2 - Phs ..." | (vazio) |
| MATROW5 = 178 | m_vol3* "Osc 3 - Vol ..." | m_pit3* "Osc 3 - Pit ..." |
| MATROW6 = 217 | m_phs3* "Osc 3 - Phs ..." | m_sub3* "Osc 3 - Sub env 1" ... |

Coordenadas completas (x, y): esquerda (32|57|82|107, 22|61|100|139|178|217) = 24 knobs; direita (149|174|199|224, 22|61|100|178|217) = 20 knobs. Total 44.

- **Visores vivos / Desenho próprio:** nenhum (só pixmaps e knobs Styled).
- **Notas:**
  - `updateLayout()` só é chamado no clique dos botões de aba; `modelChanged()` não o chama. Ao abrir um projeto salvo com a Matriz selecionada, o botão "matview" aparece ativo mas a sub-view Operadores continua visível (o construtor sempre mostra Operadores). Na versão nova, sincronizar a sub-view com m_selectedView.
  - Os tinyleds de sync R/H de osc2 e osc3 ficam 3 px acima da linha dos knobs (O2ROW-3=58, O3ROW-3=97).
  - O tooltip das abas é a única indicação textual; os rótulos estão desenhados nas artworks.

### Nes

- **Classificação:** visual próprio ; **Categoria:** instrumento
- **Arquivos:** `plugins/Nes/Nes.cpp/.h` (view `NesInstrumentView`; macros `makeknob`, `makenesled`, `makedcled` em Nes.h).
  - `plugins/Nes/artwork.png` 250 x 250 (palette, `static`)
  - `nesled_on.png` / `nesled_off.png` 14 x 12
  - `nesdc1_on.png`, `nesdc2_on.png`, `nesdc3_on.png`, `nesdc4_on.png`, `nesdc_off.png`: 12 x 10
  - `logo.png` 48 x 48
  - Todas referenciadas.
- **Janela:** 250 x 250; palette backgroundRole = `artwork.png`.
- **Tipos:** `makeknob`: Knob Styled, setFixedSize(29, 29), objectName "". Tema `NesInstrumentView Knob`: color #e7231b; outerColor #fff; outerRadius 11.0; innerRadius 8.0; centerPoint (14.5, 14.5); lineWidth 2. `makenesled`: PixmapButton checável `nesled_on`/`nesled_off`. `makedcled`: PixmapButton (não checável, dentro de AutomatableButtonGroup) ativo `nesdcN_on`, inativo `nesdc_off`.
- **Constantes:** KNOB_Y1..4 = 24, 81, 138, 195; KNOB_X1..7 = 12, 46, 84, 117, 151, 172, 206.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| nesled | m_ch1Enabled "Channel 1 enable" (true) | (12, 12) | "Enable channel 1" |
| Knob 29x29 | m_ch1Volume "Channel 1 volume" (15, 0..15) | (12, 24) | hint "Volume" "" |
| Knob | m_ch1Crs "Channel 1 coarse detune" (-24..24) | (46, 24) | "Coarse detune" |
| nesled | m_ch1EnvEnabled "Channel 1 envelope enable" | (84, 12) | "Enable envelope 1" |
| Knob | m_ch1EnvLen "Channel 1 envelope length" (0..15) | (84, 24) | "Envelope length" |
| nesled | m_ch1EnvLooped "Channel 1 envelope loop" | (129, 12) | "Enable envelope 1 loop" |
| dcled `nesdc1_on` (idx 0) | m_ch1DutyCycle "Channel 1 duty cycle" (0, 0..3) | (117, 42) | "12.5% Duty cycle" |
| dcled `nesdc2_on` (1) | m_ch1DutyCycle | (130, 42) | "25% Duty cycle" |
| dcled `nesdc3_on` (2) | m_ch1DutyCycle | (143, 42) | "50% Duty cycle" |
| dcled `nesdc4_on` (3) | m_ch1DutyCycle | (156, 42) | "75% Duty cycle" |
| nesled | m_ch1SweepEnabled "Channel 1 sweep enable" | (172, 12) | "Enable sweep 1" |
| Knob | m_ch1SweepAmt "Channel 1 sweep amount" (-7..7) | (172, 24) | "Sweep amount" |
| Knob | m_ch1SweepRate "Channel 1 sweep rate" (0..7) | (206, 24) | "Sweep rate" |
| nesled | m_ch2Enabled "Channel 2 enable" (true) | (12, 69) | "Enable channel 2" |
| Knob | m_ch2Volume "Channel 2 volume" | (12, 81) | "Volume" |
| Knob | m_ch2Crs "Channel 2 coarse detune" | (46, 81) | "Coarse detune" |
| nesled | m_ch2EnvEnabled "Channel 2 envelope enable" | (84, 69) | "Enable envelope 2" |
| Knob | m_ch2EnvLen "Channel 2 envelope length" | (84, 81) | "Envelope length" |
| nesled | m_ch2EnvLooped "Channel 2 envelope loop" | (129, 69) | "Enable envelope 2 loop" |
| dcled 1..4 | m_ch2DutyCycle "Channel 2 duty cycle" (padrão 2) | (117, 99), (130, 99), (143, 99), (156, 99) | "12.5% / 25% / 50% / 75% Duty cycle" |
| nesled | m_ch2SweepEnabled "Channel 2 sweep enable" | (172, 69) | "Enable sweep 2" |
| Knob | m_ch2SweepAmt "Channel 2 sweep amount" | (172, 81) | "Sweep amount" |
| Knob | m_ch2SweepRate "Channel 2 sweep rate" | (206, 81) | "Sweep rate" |
| nesled | m_ch3Enabled "Channel 3 enable" (true) | (12, 126) | "Enable channel 3" |
| Knob | m_ch3Volume "Channel 3 volume" | (12, 138) | "Volume" |
| Knob | m_ch3Crs "Channel 3 coarse detune" | (46, 138) | "Coarse detune" |
| Knob | m_masterVol "Master volume" (1.0, 0..2) | (117, 138) | "Master volume" |
| Knob | m_vibrato "Vibrato" (0..15) | (151, 138) | "Vibrato" |
| nesled | m_ch4Enabled "Channel 4 enable" (false) | (12, 183) | "Enable channel 4" |
| Knob | m_ch4Volume "Channel 4 volume" | (12, 195) | "Volume" |
| Knob | m_ch4NoiseFreq "Channel 4 noise frequency" (0..15) | (46, 195) | "Noise Frequency" |
| nesled | m_ch4EnvEnabled "Channel 4 envelope enable" | (84, 183) | "Enable envelope 4" |
| Knob | m_ch4EnvLen "Channel 4 envelope length" | (84, 195) | "Envelope length" |
| nesled | m_ch4EnvLooped "Channel 4 envelope loop" | (129, 183) | "Enable envelope 4 loop" |
| Knob | m_ch4Sweep "Channel 4 noise frequency sweep" (-7..7) | (117, 195) | "Frequency sweep" |
| nesled | m_ch4NoiseQuantize "Channel 4 quantize" (true) | (162, 183) | "Quantize noise frequency when using note frequency" |
| nesled | m_ch4NoiseFreqMode "Channel 4 frequency mode" | (148, 203) | "Use note frequency for noise" |
| nesled | m_ch4NoiseMode "Channel 4 noise mode" | (148, 224) | "Noise mode" |

  Todas as dicas de knob têm unidade "".
- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** os LEDs de loop de envelope ficam em x=129 fixo (não é constante). Canal 3 não tem envelope nem sweep. Os botões de duty cycle não são checáveis individualmente; o AutomatableButtonGroup marca o ativo.

### OpulenZ

- **Classificação:** visual próprio (artwork de fundo + PixmapButtons LED e forma de onda posicionados à mão); **Categoria:** instrumento
- **Arquivos:** `plugins/OpulenZ/OpulenZ.cpp` (view em `OpulenzInstrumentView`, linhas ~774-970), `plugins/OpulenZ/OpulenZ.h`. Imagens: `plugins/OpulenZ/artwork.png` 250x250; `led_on.png`/`led_off.png` 16x17; `wave1_on/off.png` 24x19; `wave2_on/off.png` 21x19; `wave3_on/off.png` 21x19; `wave4_on/off.png` 25x19; `logo.png` 48x48 (ícone do descritor). Nenhum SVG. Todas as imagens são referenciadas.
- **Janela:** 250x250 fixa; fundo `artwork.png`.
- **Controles:** knobs via macro `KNOB_GEN`: `Knob(KnobType::Styled)`, `setFixedSize(22,22)`, `setCenterPointX(11.0)`, `setCenterPointY(11.0)`, `setTotalAngle(270.0)`. Estilo do tema (`OpulenzInstrumentView Knob`): `color: rgb(128,128,128)`, `outerColor: rgb(255,255,255)`, `innerRadius 2`, `outerRadius 9`, `lineWidth 2`. Botões `BUTTON_GEN`: `PixmapButton` checkable, ativo `led_on`, inativo `led_off`, com `setToolTip`. Botões `WAVEBUTTON_GEN`: `PixmapButton` (não checkable explicitamente; exclusivos via `AutomatableButtonGroup`) com `waveN_on`/`waveN_off`.

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Knob Styled 22x22 | `op1_a_mdl` ("Op 1 attack") 0..15 | 6, 48 | "Attack" + " (<tempo>)" dinâmico |
| Knob Styled 22x22 | `op1_d_mdl` ("Op 1 decay") | 34, 48 | "Decay" + " (<tempo>)" |
| Knob Styled 22x22 | `op1_s_mdl` ("Op 1 sustain") | 62, 48 | "Sustain" |
| Knob Styled 22x22 | `op1_r_mdl` ("Op 1 release") | 90, 48 | "Release" + " (<tempo>)" |
| Knob Styled 22x22 | `feedback_mdl` ("Op 1 feedback") 0..7 | 128, 48 | "Feedback" |
| Knob Styled 22x22 | `op1_lvl_mdl` ("Op 1 level") 0..63 | 166, 48 | "Level" |
| Knob Styled 22x22 | `op1_scale_mdl` ("Op 1 level scaling") 0..3 | 194, 48 | "Scale" |
| Knob Styled 22x22 | `op1_mul_mdl` ("Op 1 frequency multiplier") 0..15 | 222, 48 | "Frequency multiplier" + " (N semitones)" |
| PixmapButton led_on/led_off 16x17 | `op1_ksr_mdl` ("Op 1 key scaling rate") | 9, 87 | tooltip "Keyboard scaling rate" |
| PixmapButton led | `op1_perc_mdl` ("Op 1 percussive envelope") | 36, 87 | "Percussive envelope" |
| PixmapButton led | `op1_trem_mdl` ("Op 1 tremolo") | 65, 87 | "Tremolo" |
| PixmapButton led | `op1_vib_mdl` ("Op 1 vibrato") | 93, 87 | "Vibrato" |
| AutomatableButtonGroup `op1_waveform` | `op1_waveform_mdl` ("Op 1 waveform") 0..3 | (grupo, sem geometria) | — |
| PixmapButton wave1_on/off 24x19 | grupo op1, valor 0 | 154, 86 | "Sine" |
| PixmapButton wave2_on/off 21x19 | grupo op1, valor 1 | 178, 86 | "Half sine" |
| PixmapButton wave3_on/off 21x19 | grupo op1, valor 2 | 199, 86 | "Absolute sine" |
| PixmapButton wave4_on/off 25x19 | grupo op1, valor 3 | 220, 86 | "Quarter sine" |
| Knob Styled 22x22 | `op2_a_mdl` ("Op 2 attack") | 6, 138 | "Attack" + tempo |
| Knob Styled 22x22 | `op2_d_mdl` ("Op 2 decay") | 34, 138 | "Decay" + tempo |
| Knob Styled 22x22 | `op2_s_mdl` ("Op 2 sustain") | 62, 138 | "Sustain" |
| Knob Styled 22x22 | `op2_r_mdl` ("Op 2 release") | 90, 138 | "Release" + tempo |
| Knob Styled 22x22 | `op2_lvl_mdl` ("Op 2 level") | 166, 138 | "Level" |
| Knob Styled 22x22 | `op2_scale_mdl` ("Op 2 level scaling") | 194, 138 | "Scale" |
| Knob Styled 22x22 | `op2_mul_mdl` ("Op 2 frequency multiplier") | 222, 138 | "Frequency multiplier" + " (N semitones)" |
| PixmapButton led | `op2_ksr_mdl` ("Op 2 key scaling rate") | 9, 177 | "Keyboard scaling rate" |
| PixmapButton led | `op2_perc_mdl` ("Op 2 percussive envelope") | 36, 177 | "Percussive envelope" |
| PixmapButton led | `op2_trem_mdl` ("Op 2 tremolo") | 65, 177 | "Tremolo" |
| PixmapButton led | `op2_vib_mdl` ("Op 2 vibrato") | 93, 177 | "Vibrato" |
| AutomatableButtonGroup `op2_waveform` | `op2_waveform_mdl` ("Op 2 waveform") | — | — |
| PixmapButton wave1 | grupo op2, 0 | 154, 176 | "Sine" |
| PixmapButton wave2 | grupo op2, 1 | 178, 176 | "Half sine" |
| PixmapButton wave3 | grupo op2, 2 | 199, 176 | "Absolute sine" |
| PixmapButton wave4 | grupo op2, 3 | 220, 176 | "Quarter Sine" (Q maiúsculo, diferente do op1) |
| PixmapButton led | `fm_mdl` ("FM") | 9, 220 | "FM" |
| PixmapButton led | `vib_depth_mdl` ("Vibrato depth") | 65, 220 | "Vibrato depth" |
| PixmapButton led | `trem_depth_mdl` ("Tremolo depth") | 93, 220 | "Tremolo depth" |
| Knob Styled 22x22 | `m_volumeModel` ("Volume") -24..24 dB, passo 0.1, padrão 16 | 222, 219 | "Volume" + " dB" |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum `paintEvent`.
- **Notas:**
  - Dicas dinâmicas (`updateKnobHints`, chamado em `dataChanged`): Attack usa tabela `{0, 0.2, 0.4, 0.9, 1.8, 3.7, 7.4, 15, 30, 60, 120, 240, 480, 950, 1900, 3800}` ms; Decay/Release `{0, 1.2, 2.5, 5, 10, 20, 40, 80, 160, 320, 640, 1300, 2600, 5200, 10000, 20000}` ms; formatação `knobHintHelper`: >1000 -> "N s" (0 casas), >10 -> "N ms" (0 casas), senão "N.N ms"; o texto fica " (<valor>)". Multiplicador: tabela de semitons `{-12, 0, 12, 19, 24, 28, 31, 34, 36, 38, 40, 40, 43, 43, 47, 47}` -> " (N semitones)".
  - `m_patchModel` ("Patch", 0..127) existe mas não tem widget (linha `m_patch->setModel` comentada).
  - Não há rótulos em QLabel: todos os textos (OP1/OP2, ADSR etc.) estão desenhados no `artwork.png`.
  - Os quatro botões de onda ficam encostados sem folga: 154+24=178, 178+21=199, 199+21=220, 220+25=245.

### Organic

- **Classificação:** visual próprio (artwork + knobs Styled em grade + PixmapButton); **Categoria:** instrumento
- **Arquivos:** `plugins/Organic/Organic.cpp` (classes `OrganicKnob`, `OrganicVolumeKnob`, `OrganicInstrumentView`, linhas ~343-505), `plugins/Organic/Organic.h` (`NUM_OSCILLATORS = 8`, `NUM_HARMONICS = 18`, `HARMONIC_NAMES`, `WAVEFORM_NAMES`). Imagens: `plugins/Organic/artwork.png` 250x250; `randomise.png` 93x16; `randomise_pressed.png` 93x16; `logo.png` 48x49. Sem SVG. Todas referenciadas.
- **Janela:** 250x250; fundo `artwork.png` (pixmap guardado em `static s_artwork`).
- **Controles:** `OrganicKnob` = `Knob(KnobType::Styled)` com `setFixedSize(21,21)`; `OrganicVolumeKnob` = `VolumeKnob(KnobType::Styled)` 21x21. Estilo do tema (`OrganicInstrumentView Knob`): `color rgb(124,207,98)`, `outerColor rgb(13,42,4)`, `innerRadius 2`, `outerRadius 7.5`, `centerPoint (10.5,10.5)`, `lineWidth 1.5`. `#harmKnob`: `color rgb(205,98,216)`, `outerColor rgb(18,4,18)`. `#fx1Knob`, `#volKnob`: `color rgb(157,157,157)`, `outerColor rgb(37,37,37)`, `innerRadius 4`, `outerRadius 10.0`, `centerPoint (18.5,13.8)`, `lineWidth 2`.

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| OrganicKnob (Styled), objectName `fx1Knob` | `m_fx1Model` ("Distortion") 0..0.99 | 15, 201; 37x47 (redimensionado após o 21x21) | "Distortion:" sem unidade |
| OrganicVolumeKnob (Styled), objectName `volKnob` | `m_volModel` ("Volume") 0..200 | 60, 201; 37x47 | "Volume:" + "%" |
| PixmapButton `randomise` / `randomise_pressed` 93x16 | sem modelo; `clicked()` -> `randomiseSettings()` | 148, 224 | texto do botão "Randomise" (sem tooltip) |

Grade de osciladores (criada em `modelChanged()`), `x = 53 + i*24`, `y` por linha com `y=91`, `rowHeight=26`; i = 0..7 -> x = 53, 77, 101, 125, 149, 173, 197, 221:

| Linha | Widget | Modelo (label, i+1 = N) | y | Dica |
|---|---|---|---|---|
| harmônico | OrganicKnob 21x21, objectName `harmKnob` | `m_osc[i]->m_harmModel` ("Osc N harmonic") 0..17, padrão = i | 65 | "Osc N harmonic:" + " (<HARMONIC_NAMES[v]>)" |
| forma de onda | OrganicKnob 21x21 | `m_oscModel` ("Osc N waveform") 0..5 | 91 | "Osc N waveform:" + " (<WAVEFORM_NAMES[v]>)" |
| volume | `VolumeKnob(Styled)` 21x21 (não é OrganicVolumeKnob) | `m_volModel` ("Osc N volume") 0..100 | 117 | "Osc N volume:" + "%" |
| pan | OrganicKnob 21x21 | `m_panModel` ("Osc N panning") -100..100 | 143 | "Osc N panning:" + "" |
| detune | OrganicKnob 21x21 | `m_detuneModel` ("Osc N stereo detuning") -1200..1200 | 169 | "Osc N stereo detuning" + " cents" |

Coordenadas completas da grade (x; y = 65/91/117/143/169): osc1 x=53, osc2 x=77, osc3 x=101, osc4 x=125, osc5 x=149, osc6 x=173, osc7 x=197, osc8 x=221.

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum.
- **Notas:** `HARMONIC_NAMES` (18): Octave below, Fifth below, Fundamental, 2nd harmonic ... (ver `Organic.h:55`); `WAVEFORM_NAMES` (6): Sine wave, Saw wave, Square wave, Triangle wave, Moog saw wave, Exponential wave (ver `Organic.h:76`). Os knobs grandes fx1/vol têm 37x47 com centro no tema em (18.5,13.8): a área abaixo do knob fica vazia (rótulos estão na arte). O texto de `PixmapButton(this, tr("Randomise"))` é só nome acessível; não é desenhado.

### Patman

- **Classificação:** visual próprio (artwork, PixmapButtons e `paintEvent` com nome do arquivo); **Categoria:** instrumento
- **Arquivos:** `plugins/Patman/Patman.cpp` (`PatmanView`, linhas ~454-660), `plugins/Patman/Patman.h`. Imagens: `plugins/Patman/artwork.png` 250x250; `select_file.png`/`select_file_on.png` 20x16; `loop_on.png`/`loop_off.png` 20x18; `tune_on.png`/`tune_off.png` 20x18; `logo.png` 48x48. Sem SVG. Todas referenciadas.
- **Janela:** 250x250; fundo `artwork.png`. Aceita drop (`setAcceptDrops(true)`): string-pair "patchfile", ou "samplefile" terminado em `.pat`.

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| PixmapButton `openFileButton` ativo `select_file_on`, inativo `select_file` 20x16; cursor PointingHand | sem modelo; `clicked()` -> `openFile()` (FileDialog "Open patch file", filtro "Patch-Files (*.pat)", pasta inicial `/usr/share/midi/freepats` se existir) | 227, 86 | tooltip "Open patch" |
| PixmapButton `loopButton` checkable `loop_on`/`loop_off` 20x18 | `m_loopedModel` (BoolModel, padrão true, sem label; salvo como "looped") | 195, 138 | "Loop mode" |
| PixmapButton `tuneButton` checkable `tune_on`/`tune_off` 20x18 | `m_tunedModel` (padrão true, "tuned") | 223, 138 | "Tune mode" |

- **Visores vivos:** nome do arquivo do patch (texto), atualizado no sinal `fileChanged()`.
- **Desenho próprio:** `paintEvent`: `QPainter`, fonte = fonte do widget com `adjustedToPixelSize(font(), SMALL_FONT_SIZE)` (10 px), cor padrão da paleta (sem `setPen`); `drawText(8, 116, 235, 16, AlignLeft|TextSingleLine|AlignVCenter, m_displayFilename)`. Texto inicial "No file selected". `updateFilename()`: monta o texto a partir do fim do caminho, caractere a caractere, enquanto `largura(texto + "...") < 225`; se cortou, prefixa "...".
- **Notas:** o retângulo branco do nome está na arte; o texto cai em (8,116)-(243,132).

### Sf2Player

- **Classificação:** visual próprio (artwork + PixmapButtons + LCDs + QLabels posicionados); **Categoria:** instrumento
- **Arquivos:** `plugins/Sf2Player/Sf2Player.cpp` (`Sf2Knob`, `Sf2InstrumentView`, linhas ~1060-1400), `plugins/Sf2Player/Sf2Player.h`; diálogo `plugins/Sf2Player/PatchesDialog.cpp/.h/.ui`. Imagens: `plugins/Sf2Player/artwork.png` 250x250; `fileselect_on.png`/`fileselect_off.png` 18x14; `patches_on.png`/`patches_off.png` 18x14; `reverb_on.png`/`reverb_off.png` 50x14; `chorus_on.png`/`chorus_off.png` 50x14; `logo.png` 48x48. Do tema: `data/themes/default/lcd_21pink.png` 143x38 (fonte de dígitos do `LcdSpinBox` estilo "21pink"). Sem SVG. Todas as imagens do plugin são referenciadas.
- **Janela:** 250x250; fundo `artwork.png`. Aceita drop de `.sf2`/`.sf3` (string-pair "soundfontfile", ou "samplefile"/"importedproject" com essa extensão, ou URL local única).
- **Controles:** `Sf2Knob` = `Knob(KnobType::Styled)` `setFixedSize(31, 38)`. Estilo do tema (`Sf2InstrumentView Knob`): `color #ff00ea`, `outerColor rgb(20,5,18)`, `innerRadius 2`, `outerRadius 9.2`, `centerPoint (15.5,15.42)`, `lineWidth 2`. QLabels filhos diretos: `font-size: 10px` (regra `Sf2InstrumentView > QLabel`).

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| PixmapButton `fileselect_on`/`fileselect_off` 18x14, cursor PointingHand | sem modelo; `clicked()` -> `showFileDialog()` ("Open SoundFont file", filtro "SoundFont Files (*.sf2 *.sf3)") | 217, 107 | tooltip "Open SoundFont file" |
| PixmapButton `patches_on`/`patches_off` 18x14, cursor PointingHand; desabilitado sem arquivo | sem modelo; `clicked()` -> `showPatchDialog()` | 217, 125 | "Choose patch" |
| LcdSpinBox(3 dígitos, "21pink") | `m_bankNum` ("Bank") 0..999 | 131, 62 | — |
| LcdSpinBox(3, "21pink") | `m_patchNum` ("Patch") 0..127 | 190, 62 | — |
| QLabel `m_filenameLabel` | nome do arquivo (completeBaseName, elidido à direita) | 58, 109; 156x11 | tooltip: caminho absoluto, ou "No soundfont: open one, or drag it here" |
| QLabel `m_patchLabel` | nome do patch atual (elidido à direita) | 58, 127; 156x11 | tooltip "Bank %1, patch %2: %3" (vazio se sem patch) |
| Sf2Knob 31x38 | `m_gain` ("Gain") 0..5, padrão 1 | 86, 55 | "Gain:" sem unidade |
| PixmapButton checkable `reverb_on`/`reverb_off` 50x14 | `m_reverbOn` ("Reverb") | 14, 180 | "Apply reverb (if supported)" |
| Sf2Knob | `m_reverbRoomSize` ("Reverb room size") 0..1 | 93, 160 | "Room size:" |
| Sf2Knob | `m_reverbDamping` ("Reverb damping") 0..1 | 130, 160 | "Damping:" |
| Sf2Knob | `m_reverbWidth` ("Reverb width") 0..1 | 167, 160 | "Width:" |
| Sf2Knob | `m_reverbLevel` ("Reverb level") 0..1 | 204, 160 | "Level:" |
| PixmapButton checkable `chorus_on`/`chorus_off` 50x14 | `m_chorusOn` ("Chorus") | 14, 226 | "Apply chorus (if supported)" |
| Sf2Knob | `m_chorusNum` ("Chorus voices") 0..10 passo 1 | 93, 206 | "Voices:" |
| Sf2Knob | `m_chorusLevel` ("Chorus level") 0..10 | 130, 206 | "Level:" |
| Sf2Knob | `m_chorusSpeed` ("Chorus speed") 0.29..5 | 167, 206 | "Speed:" |
| Sf2Knob | `m_chorusDepth` ("Chorus depth") 0..46 passo 0.05 | 204, 206 | "Depth:" |

- **Visores vivos:** os dois LCDs (bank/patch) e os dois QLabels (arquivo/patch), atualizados por `fileChanged()`, `dataChanged()` de bank/patch; `fileLoading()` desabilita o botão de patches.
- **Desenho próprio:** nenhum `paintEvent` (os textos são QLabels, cor da paleta/tema, 10 px).
- **Notas:**
  - `PatchesDialog` (apenas registro): QDialog de `PatchesDialog.ui` (Qsynth "Channel Preset"), tamanho inicial 480x350, mínimo 300x150, título da janela "<nome da trilha> - Soundfont patches"; duas QTreeWidget (banco à esquerda, largura máx. 80, coluna 0 = 30 px; programas à direita), botões OK/Cancel. Ao escolher, escreve em `m_bankNum`/`m_patchNum` e no `m_patchLabel`.
  - Restos de layout antigo (`QVBoxLayout`/`QHBoxLayout`) estão comentados; a posição é absoluta.
  - O LcdSpinBox não define posição de rótulo; o rótulo "BANK"/"PATCH" está na arte.

### Sfxr

- **Classificação:** visual próprio (artwork, grade de knobs Styled coloridos por objectName, PixmapButtons); **Categoria:** instrumento
- **Arquivos:** `plugins/Sfxr/Sfxr.cpp` (`SfxrKnob`, macros `createKnob`/`createButton`/`createButtonLocalGraphic`, `SfxrInstrumentView` linhas ~553-1060), `plugins/Sfxr/Sfxr.h` (constantes de layout, linhas 47-64), `plugins/Sfxr/readme.txt` (licença MIT do sfxr original). Imagens (todas em `plugins/Sfxr/`): `artwork.png` 250x250; `sfxr_square_wave_active/inactive.png`, `sfxr_saw_wave_active/inactive.png`, `sfxr_sin_wave_active/inactive.png`, `sfxr_white_noise_wave_active/inactive.png` 14x14; `pickup_`, `laser_`, `explosion_`, `powerup_`, `hit_`, `jump_`, `blip_` `active/inactive.png` 14x14; `randomize_active/inactive.png` 40x14; `mutate_active/inactive.png` 40x14; `logo.png` 48x48. Sem SVG. Todas referenciadas (a macro `createButton`, que usaria `embed::getIconPixmap`, não é usada).
- **Janela:** 250x250; fundo `artwork.png`.
- **Constantes (`Sfxr.h`):** `WAVEFORM_BASE_X=20`, `WAVEFORM_BASE_Y=15`, `WAVEFORM_BUTTON_WIDTH=16`; `GENERATOR_BASE_X=110`, `GENERATOR_BASE_Y=25`, `GENERATOR_BUTTON_WIDTH=16`; `RAND_BUTTON_X=160`, `RAND_BUTTON_Y=4`; `MUTA_BUTTON_X=205`, `MUTA_BUTTON_Y=4`; `KNOBS_BASE_X=20`, `KNOBS_BASE_Y=50`, `KNOB_BLOCK_SIZE_X=40`, `KNOB_BLOCK_SIZE_Y=40`.
- **Controles:** `SfxrKnob` = `Knob(KnobType::Styled)`, `setFixedSize(20,20)`, `setCenterPointX(10.0)`, `setCenterPointY(10.0)`, `setTotalAngle(270.0)`, `setLineWidth(1)`. `createKnob`: `setHintText(tr(name ":"), "")` e `setToolTip(tr(name))`. Estilo do tema (`SfxrInstrumentView Knob`): `color #000`, `outerColor rgb(194,177,145)`, `innerRadius 2`, `outerRadius 9`, `lineWidth 2` (o `qproperty-lineWidth: 2` do tema é aplicado no polish, depois do construtor, logo deve prevalecer sobre o `setLineWidth(1)`; o mesmo vale para Opulenz: o tema não define centerPoint, então valem os do código). Por objectName: `#envKnob` color `#263352` / outer `#4b66a4`; `#freqKnob` `#1e4a22`/`#3c9544`; `#changeKnob` `#591c1c`/`#b23737`; `#sqrKnob` `#3b2714`/`#724c27`; `#repeatKnob` `#292929`/`#515151`; `#phaserKnob` `#144c4d`/`#299899`; `#filterKnob` `#47224c`/`#8e4397`. Todos os modelos são `SfxrZeroToOneFloatModel` (0..1, passo 0.001), label sem `tr` igual ao nome abaixo.

Knobs (x = 20 + 40*col, y = 50 + 40*linha):

| Widget (objectName) | Modelo (label) | Posição | Dica / tooltip |
|---|---|---|---|
| SfxrKnob `envKnob` | `m_attModel` ("Attack Time") | 20, 50 | "Attack Time:" / "Attack Time" |
| SfxrKnob `envKnob` | `m_holdModel` ("Sustain Time") | 60, 50 | "Sustain Time:" |
| SfxrKnob `envKnob` | `m_susModel` ("Sustain Punch") | 100, 50 | "Sustain Punch:" |
| SfxrKnob `envKnob` | `m_decModel` ("Decay Time") | 140, 50 | "Decay Time:" |
| SfxrKnob `freqKnob` | `m_startFreqModel` ("Start Frequency") | 20, 90 | "Start Frequency:" |
| SfxrKnob `freqKnob` | `m_minFreqModel` ("Min Frequency") | 60, 90 | "Min Frequency:" |
| SfxrKnob `freqKnob` | `m_slideModel` ("Slide") | 100, 90 | "Slide:" |
| SfxrKnob `freqKnob` | `m_dSlideModel` ("Delta Slide") | 140, 90 | "Delta Slide:" |
| SfxrKnob `freqKnob` | `m_vibDepthModel` ("Vibrato Depth") | 180, 90 | "Vibrato Depth:" |
| SfxrKnob `freqKnob` | `m_vibSpeedModel` ("Vibrato Speed") | 220, 90 | "Vibrato Speed:" |
| SfxrKnob `changeKnob` | `m_changeAmtModel` ("Change Amount") | 20, 130 | "Change Amount:" |
| SfxrKnob `changeKnob` | `m_changeSpeedModel` ("Change Speed") | 60, 130 | "Change Speed:" |
| SfxrKnob `sqrKnob` | `m_sqrDutyModel` ("Square Duty") | 140, 130 | "Square Duty (Square wave only):" |
| SfxrKnob `sqrKnob` | `m_sqrSweepModel` ("Duty Sweep") | 180, 130 | "Duty Sweep (Square wave only):" |
| SfxrKnob `repeatKnob` | `m_repeatSpeedModel` ("Repeat Speed") | 20, 170 | "Repeat Speed:" |
| SfxrKnob `phaserKnob` | `m_phaserOffsetModel` ("Phaser Offset") | 140, 170 | "Phaser Offset:" |
| SfxrKnob `phaserKnob` | `m_phaserSweepModel` ("Phaser Sweep") | 180, 170 | "Phaser Sweep:" |
| SfxrKnob `filterKnob` | `m_lpFilCutModel` ("LP Filter Cutoff"), padrão 1 | 20, 210 | "LP Filter Cutoff:" |
| SfxrKnob `filterKnob` | `m_lpFilCutSweepModel` ("LP Filter Cutoff Sweep") | 60, 210 | "LP Filter Cutoff Sweep:" |
| SfxrKnob `filterKnob` | `m_lpFilResoModel` ("LP Filter Resonance") | 100, 210 | "LP Filter Resonance:" |
| SfxrKnob `filterKnob` | `m_hpFilCutModel` ("HP Filter Cutoff") | 140, 210 | "HP Filter Cutoff:" |
| SfxrKnob `filterKnob` | `m_hpFilCutSweepModel` ("HP Filter Cutoff Sweep") | 180, 210 | "HP Filter Cutoff Sweep:" |

Botões (`PixmapButton`, `<res>_active` / `<res>_inactive`, tooltip = nome):

| Widget | Modelo / ação | Posição | Dica |
|---|---|---|---|
| AutomatableButtonGroup `m_waveBtnGroup` | `m_waveFormModel` ("Wave") 0..3, padrão Square | — | — |
| PixmapButton `sfxr_square_wave` 14x14 | grupo, valor 0 | 20, 15 | "Square Wave" |
| PixmapButton `sfxr_saw_wave` 14x14 | grupo, 1 | 36, 15 | "Saw Wave" |
| PixmapButton `sfxr_sin_wave` 14x14 | grupo, 2 | 52, 15 | "Sine Wave" |
| PixmapButton `sfxr_white_noise_wave` 14x14 | grupo, 3 | 68, 15 | "Noise" |
| PixmapButton `pickup` 14x14 | `genPickup()` | 110, 25 | "Generate pick up/coin sfx" |
| PixmapButton `laser` | `genLaser()` | 126, 25 | "Generate laser/shoot sfx" |
| PixmapButton `explosion` | `genExplosion()` | 142, 25 | "Generate explosion sfx" |
| PixmapButton `powerup` | `genPowerup()` | 158, 25 | "Generate power up sfx" |
| PixmapButton `hit` | `genHit()` | 174, 25 | "Generate hit/hurt sfx" |
| PixmapButton `jump` | `genJump()` | 190, 25 | "Generate jump sfx" |
| PixmapButton `blip` | `genBlip()` | 206, 25 | "Generate blip/select sfx" |
| PixmapButton `randomize` 40x14 | `randomize()` | 160, 4 | "Generate random sfx" |
| PixmapButton `mutate` 40x14 | `mutate()` | 205, 4 | "Mutate sfx" |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum.
- **Notas:** os botões de forma de onda usam `KNOBS_BASE_X` (=20) como x base, não `WAVEFORM_BASE_X` (também 20; mesmo resultado). Os geradores/random/mutate não são checkable (cliques momentâneos que alteram os modelos). A pré-escuta (`previewSound`) está desligada (connects comentados). Lacunas na grade (col 2 da linha 2, cols 1-2 da linha 3, col 5 das linhas 0,2,3,4) são intencionais; a arte tem os rótulos.

### Sid

- **Classificação:** visual próprio (artwork, knobs Styled pequenos, PixmapButtons de texto com versão "red" para ativo); **Categoria:** instrumento
- **Arquivos:** `plugins/Sid/SidInstrument.cpp` (`sidKnob`, `SidInstrumentView`, linhas ~450-760), `plugins/Sid/SidInstrument.h`. Imagens (`plugins/Sid/`, ativo = `<nome>red.png`, inativo = `<nome>.png`): `artwork.png` 250x250; `hp`/`hpred` 24x12; `bp`/`bpred` 21x12; `lp`/`lpred` 22x12; `3off`/`3offred` 40x12; `6581`/`6581red` 37x12; `8580`/`8580red` 40x12; `pulse`/`pulsered` 20x12; `triangle`/`trianglered` 19x12; `saw`/`sawred` 19x12; `noise`/`noisered` 21x12; `sync`/`syncred` 40x12; `ring`/`ringred` 37x12; `filter`/`filterred` 40x12; `test`/`testred` 37x12; `logo.png` 48x48. Sem SVG. Todas referenciadas.
- **Janela:** 250x250; fundo `artwork.png`.
- **Controles:** `sidKnob` = `Knob(KnobType::Styled)`, `setFixedSize(16,16)`, `centerPoint (7.5, 7.5)`, `innerRadius 2`, `outerRadius 8`, `totalAngle 270`, `lineWidth 2`. Tema (`SidInstrumentView Knob`): `color rgb(113,95,80)`, `outerColor rgb(255,255,255)`, `innerRadius 2`, `outerRadius 7`, `lineWidth 2` (o tema, aplicado no polish, deve prevalecer: outerRadius efetivo 7).

Globais:

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| sidKnob 16x16 | `m_volumeModel` ("Volume") 0..15, padrão 15 | 7, 64 | "Volume:" ""; tooltip dinâmico = valor inteiro |
| sidKnob | `m_filterResonanceModel` ("Resonance") 0..15 | 35, 64 | "Resonance:" ""; tooltip = valor inteiro |
| sidKnob | `m_filterFCModel` ("Cutoff frequency") 0..2047, padrão 1024 | 63, 64 | inicial "Cutoff frequency:" " Hz"; dinâmico "Cutoff frequency: " + " (N Hz)", N = int(9970/2047*v + 30); tooltip "N Hz" |
| AutomatableButtonGroup `m_passBtnGrp` | `m_filterModeModel` ("Filter type") 0=HighPass,1=BandPass,2=LowPass; padrão LowPass | — | — |
| PixmapButton `hpred`/`hp` 24x12 | grupo filtro, 0 | 140, 77 | "High-pass filter " (com espaço final) |
| PixmapButton `bpred`/`bp` 21x12 | grupo filtro, 1 | 164, 77 | "Band-pass filter " |
| PixmapButton `lpred`/`lp` 22x12 | grupo filtro, 2 | 185, 77 | "Low-pass filter " |
| PixmapButton checkable `3offred`/`3off` 40x12 | `m_voice3OffModel` ("Voice 3 off") | 207, 77 | "Voice 3 off " |
| AutomatableButtonGroup `m_sidTypeBtnGrp` | `m_chipModel` ("Chip model") 0=MOS6581, 1=MOS8580; padrão 8580 | — | — |
| PixmapButton `6581red`/`6581` 37x12 | grupo chip, 0 | 170, 59 | "MOS6581 SID " |
| PixmapButton `8580red`/`8580` 40x12 | grupo chip, 1 | 207, 59 | "MOS8580 SID " |

Por voz (loop i = 0..2; N = i+1; dy = i*50 -> knobs y = 114/164/214, botões de onda y = 101/151/201, ring/filter y = 116/166/216, sync/test y = 134/184/234):

| Widget | Modelo (label) | Posição (voz1 / voz2 / voz3) | Dica |
|---|---|---|---|
| sidKnob | `m_voice[i]->m_attackModel` ("Voice N attack") 0..15 | (7,114) / (7,164) / (7,214) | "Attack: " + " (<attackTime>)"; tooltip = tempo |
| sidKnob | `m_decayModel` ("Voice N decay") | (35,114) / (35,164) / (35,214) | "Decay: " + " (<decRelTime>)" |
| sidKnob | `m_sustainModel` ("Voice N sustain") padrão 15 | (63,114) / (63,164) / (63,214) | "Sustain:"; tooltip = valor inteiro |
| sidKnob | `m_releaseModel` ("Voice N release") | (91,114) / (91,164) / (91,214) | "Release: " + " (<decRelTime>)" |
| sidKnob | `m_pulseWidthModel` ("Voice N pulse width") 0..4095, padrão 2048 | (119,114) / (119,164) / (119,214) | inicial "Pulse Width:"; dinâmico "Pulse width: " + " (v/40.95%)"; tooltip "v/40.95%" |
| sidKnob | `m_coarseModel` ("Voice N coarse detuning") -24..24 | (147,114) / (147,164) / (147,214) | "Coarse:" " semitones"; tooltip "N semitones" |
| AutomatableButtonGroup `wfbg` | `m_waveFormModel` ("Voice N wave shape") 0=Square(pulse),1=Triangle,2=Saw,3=Noise; padrão Triangle | — | — |
| PixmapButton `pulsered`/`pulse` 20x12 | onda 0 | (187,101) / (187,151) / (187,201) | "Pulse wave" |
| PixmapButton `trianglered`/`triangle` 19x12 | onda 1 | (168,101) / (168,151) / (168,201) | "Triangle wave" |
| PixmapButton `sawred`/`saw` 19x12 | onda 2 | (207,101) / (207,151) / (207,201) | "Saw wave" |
| PixmapButton `noisered`/`noise` 21x12 | onda 3 | (226,101) / (226,151) / (226,201) | "Noise" |
| PixmapButton checkable `syncred`/`sync` 40x12 | `m_syncModel` ("Voice N sync") | (207,134) / (207,184) / (207,234) | "Sync" |
| PixmapButton checkable `ringred`/`ring` 37x12 | `m_ringModModel` ("Voice N ring modulate") | (170,116) / (170,166) / (170,216) | "Ring modulation" |
| PixmapButton checkable `filterred`/`filter` 40x12 | `m_filteredModel` ("Voice N filtered") | (207,116) / (207,166) / (207,216) | "Filtered" |
| PixmapButton checkable `testred`/`test` 37x12 | `m_testModel` ("Voice N test") | (170,134) / (170,184) / (170,234) | "Test" |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum.
- **Notas:**
  - Tabelas: `attackTime` = 2 ms, 8 ms, 16 ms, 24 ms, 38 ms, 56 ms, 68 ms, 80 ms, 100 ms, 250 ms, 500 ms, 800 ms, 1 s, 3 s, 5 s, 8 s; `decRelTime` = 6 ms, 24 ms, 48 ms, 72 ms, 114 ms, 168 ms, 204 ms, 240 ms, 300 ms, 750 ms, 1.5 s, 2.4 s, 3 s, 9 s, 15 s, 24 s.
  - Ordem visual das ondas é triangle(168), pulse(187), saw(207), noise(226), diferente da ordem dos valores.
  - Knobs, além da dica, recebem `setToolTip` dinâmico com o valor formatado (atualizado por `dataChanged`).
  - Os botões de onda não são checkable explicitamente (a exclusividade vem do grupo).

### SlicerT

- **Classificação:** visual próprio (sem artwork de fundo fixo: fundo pintado em `paintEvent`, imagem `toolbox.png` como faixa, logotipo, waveform interativo próprio); **Categoria:** instrumento
- **Arquivos:** `plugins/SlicerT/SlicerTView.cpp/.h` (view, constantes em `SlicerTView.h:57-73`), `plugins/SlicerT/SlicerTWaveform.cpp/.h` (visor), `plugins/SlicerT/SlicerT.cpp` (modelos). Imagens (`plugins/SlicerT/`): `full_logo.png` 142x30; `toolbox.png` 447x97; `sync_active.png`/`sync_inactive.png` 84x18; `clear_slices_active.png`/`clear_slices_inactive.png` 84x18; `copy_midi.png` 20x20; `reset_slices.png` 20x20; `folder_icon.png` 16x12; `slice_indicator_arrow.png` 8x5; `logo.png` 50x50 (descritor). Do tema: `data/themes/default/sample_track.png` 20x20 (via `embed::getIconPixmap("sample_track")`, ícone de "sem amostra"), `lcd_19purple.png` 143x38 (dígitos do LcdSpinBox "19purple"). Sem SVG. Todas referenciadas.
- **Janela:** `InstrumentView` (NÃO fixa; redimensionável): `setMinimumSize(516, 400)`, `setMaximumSize(10000, 10000)`, `setAutoFillBackground(true)`, aceita drop ("samplefile" ou clip de trilha Sample `clip_<Type::Sample>`, usa o atributo `src`). Todas as posições são recalculadas em `resizeEvent` a partir de `width()`/`height()`.
- **Constantes:** `s_textBoxHeight=20`, `s_textBoxWidth=50`, `s_topBarHeight=50`, `s_bottomBoxHeight=97`, `s_bottomBoxOffset=65`, `s_sampleBoxHeight=14`, `s_folderButtonWidth=15` (não usado), `s_leftBoxWidth=400`, `s_x1=35`, `s_x2=85`, `s_x3=160`, `s_x4=190`, `s_x5=275`, `s_x6=325`. `m_y1 = height() - 65`. `ComboBox::DEFAULT_HEIGHT = 22`.
- **Controles:** knobs de `createStyledKnob()`: `Knob(KnobType::Styled)`, `setFixedSize(50,40)`, `centerPoint (24.0, 15.0)`. Tema (`SlicerTView Knob`): `color rgb(162,128,226)`, `outerColor rgb(162,128,226)`, `innerRadius 1`, `outerRadius 11`, `lineWidth 3`. Posições na fórmula geral e no tamanho mínimo 516x400 (m_y1 = 335):

| Widget | Modelo (label) | Posição (fórmula -> em 516x400) | Dica |
|---|---|---|---|
| Knob Styled 50x40 | `m_noteThreshold` ("Note threshold") 0..2, padrão 0.6 | (s_x1-25, m_y1) -> (10, 335) | tooltip "Threshold used for slicing" (sem setHintText) |
| Knob Styled 50x40 | `m_fadeOutFrames` ("FadeOut") 0..100 ms, padrão 10 | (s_x2-25, m_y1) -> (60, 335) | "Fade Out per note in milliseconds" |
| QPushButton com ícone `reset_slices` 20x20 | `SlicerT::updateSlices` | (s_x3-15, m_y1+3) -> (145, 338) | "Reset slices" |
| QPushButton com ícone `copy_midi` 20x20 | `exportMidi()` (copia note-list para a área de transferência) | (s_x4+2, m_y1+3) -> (192, 338) | "Copy midi pattern to clipboard" |
| LcdSpinBox(3, "19purple") | `m_originalBPM` ("Original bpm") 1..999 | (s_x5-13, m_y1+4) -> (262, 339) | "Original sample BPM" |
| ComboBox 55x22 | `m_sliceSnap` ("Slice snap"): Off, 1/1, 1/2, 1/4, 1/8, 1/16, 1/32 | `setGeometry(185,200,55,22)` no construtor, depois (s_x6-8, m_y1+3) -> (317, 338) | "Set slice snapping for detection" |
| PixmapButton checkable `sync_active`/`sync_inactive` 84x18 | `m_enableSync` ("BPM sync") | (width-100, m_y1-7) -> (416, 328) | "Enable BPM sync" |
| PixmapButton `clear_slices_active`/`clear_slices_inactive` 84x18 | `clearSlices()` (pontos = {0, 1}) | (width-100, m_y1+17) -> (416, 352) | "Clear all slices" |
| ComboBox 84x22 (`setFixedSize`) | `m_loopMode` ("Loop"): Off, Loop, Ping-pong | (width-100, m_y1+40) -> (416, 375) | "How slices play while their note is held: once, looped or back and forth" |
| PixmapButton `folder_icon` (ativo e inativo iguais) 16x12 | `openFiles()` (`FileDialog::openAudioFile`) | (width-20, height-97-14+1) -> (496, 290) | "Open sample selector" |
| SlicerTWaveform | — | (0, 50); tamanho (width, height-97-50-14) -> 516x239 | — |

- **Visores vivos:** `SlicerTWaveform` (QWidget, criado com 248x128 e redimensionado; `setMouseTracking(true)`):
  - Geometria interna: `s_seekerHorMargin=5`, `s_seekerVerMargin=6`, `s_middleMargin=6`, `s_arrowHeight=5`; `m_seekerHeight=40`; `m_seekerWidth = w - 10`; `m_editorHeight = h - 40 - 6 - 6 = h - 52`; `m_editorWidth = w`. Seeker (mini mapa da amostra inteira) desenhado em (5, 6) tamanho (w-10)x40; editor (trecho ampliado) desenhado em (0, 52) tamanho w x (h-52).
  - Dados: a amostra original (`m_originalSample`) via `SampleThumbnailRenderer::visualize` (cache de miniaturas), pontos de corte `m_slicePoints` (0..1), posição de reprodução vinda do sinal `SlicerT::isPlaying(current, start, end)`; limitada a no mínimo 10 ms entre redesenhos (`s_minMilisPassed`).
  - Cores: fundo vazio `(0,0,0,0)`; onda externa `s_waveformColor (123,49,212)`; fundo do seeker `(0,0,0,255)`; fundo do editor `(15,15,15,255)`; máscara `(151,65,255)` (pixels dessa cor são repintados com o "interno"); onda interna `(183,124,255)`; linha tocando `(255,255,255,200)`; realce da nota tocando `(255,255,255,70)`; marcador de corte `(218,193,255)`; sombra do marcador `(136,120,158)`; marcador realçado `(255,255,255)`; contorno do seeker `(178,115,255)`; interior do seeker `(178,115,255,100)`; escurecimento fora do seeker `(0,0,0,120)`; moldura do editor `(53,22,90)`; fim do gradiente `(29,16,47)`.
  - Seeker (`drawSeeker`): fundo preto, onda, linhas verticais de corte (1 px, cor de corte), linha da posição atual (branco 200), retângulo da nota tocando (branco 70), área selecionada preenchida `(178,115,255,100)` largura-1, áreas fora escurecidas `(0,0,0,120)`, retângulo arredondado da seleção `QPen(178,115,255, 1)` raio 2.
  - Editor (`drawEditor`): fundo `(15,15,15)`; sem amostra: texto "Click to load sample" (fonte da família atual, 9 pt, cor `(255,255,255,70)`, centrado num retângulo 200x200 em (w/2-100, h_ed/2-110)) e ícone `sample_track` (mascarado só no branco) centrado, deslocado 13 px. Com amostra: linha central horizontal em y = h_ed/2 + 5 (branco 70); onda em (0, 5); faixa superior 5 px preta; posição atual e nota tocando; cada corte: normal = linha sombra 1 px em x-1 `(136,120,158)` + linha 1 px `(218,193,255)` + seta `slice_indicator_arrow` em (x-4, 0); corte mais próximo do mouse = linha 2 px branca + seta. Moldura: linhas `(53,22,90)` em y=5 e y=h_ed-1.
  - `paintEvent` do waveform: retângulo cheio com `QLinearGradient` de (0,0) a (w, h - h_ed), `(15,15,15)` -> `(29,16,47)`, sem contorno; sombra arredondada do seeker em (3, 4, w-10+4, 44) raio 4 com `(0,0,0,120)`; pixmap do seeker em (5,6); pixmap do editor em (0, 52).
  - Interação: hover muda cursor (SizeHor perto de corte/borda do seeker, raio 0.02 normalizado; SizeAll no meio do seeker se houver zoom); arrastar bordas (distância mínima 0.13) ou meio do seeker; arrastar cortes; clique direito apaga corte (se >2); duplo clique esquerdo no editor cria corte; botão do meio reseta zoom; roda = zoom vertical (`m_zoomLevel += delta/360*0.5`, mínimo 0); clique sem amostra abre o seletor de arquivos.
- **Desenho próprio (`SlicerTView::paintEvent`, em 516x400; boxTopY = height-97 = 303):**
  - Fonte inicial: família atual, 7 pt.
  - Faixa inferior esquerda: `drawPixmap(QRect(0, boxTopY, 400, 97), toolbox.png)` (447x97 escalado para 400 de largura).
  - Faixa inferior direita: `fillRect(400, boxTopY, width-400, 97, QColor(23,26,31))`.
  - Barra superior: `fillRect(0, 0, width, 50, QColor(20,23,27))`; linhas `QColor(56,58,60)` em y=49 e y=0 (largura toda); linha `(56,58,60)` em y=boxTopY; divisória vertical `QColor(56,24,94)` em x=400 de boxTopY até o fim.
  - Logo `full_logo` em (10, (50-30)/2 = 10), 142x30.
  - Rótulos (pen `(255,255,255)`, 7 pt, `AlignCenter`, caixa 50x20, y = m_y1+27 = 362): "Threshold" em x=10; "Fade Out" em x=60; "Reset" em x=135; "Midi" em x=182; "BPM" em x=259; "Snap" em x=317.
  - Fundo dos knobs (antialias): arco duas vezes, primeiro `QPen(QColor(159,124,223,100), 4)` e depois `QPen(QColor(159,124,223,255), 2)`, retângulos (20,335,30,30) e (70,335,30,30), início -45 graus, extensão 270 graus (`-45*16`, `270*16`); círculo interno preenchido `QColor(106,90,138)` sem contorno, raio 9, centros (35, 350) e (85, 350).
  - Barra do nome da amostra: `fillRect(0, boxTopY-14, width, 14, QColor(5,5,5))` -> (0,289,516,14); linha vertical `(56,58,60)` em x=width-24 de 289 a 303; texto do nome (ou "No sample loaded") em `(5, 289, width, 14)`, `AlignLeft`, pen `(255,255,255,180)`, fonte 8 pt.
- **Notas:** `m_midiExportButton` e `m_resetButton` são `QPushButton` comuns com ícone (aparência do tema para QPushButton). O `connect` deles usa `&PixmapButton::clicked` (funciona porque o sinal é herdado de QAbstractButton). As posições dependem do tamanho atual da janela; o lado direito acompanha `width()`. Constante `s_folderButtonWidth` não é usada.

### Stk (Mallets)

- **Classificação:** visual próprio (artwork em três subpainéis que trocam conforme o preset); **Categoria:** instrumento
- **Arquivos:** `plugins/Stk/Mallets/Mallets.cpp` (`MalletsInstrumentView`, linhas ~509-700), `plugins/Stk/Mallets/Mallets.h`. Imagens: `plugins/Stk/Mallets/artwork.png` 250x250; `logo.png` 48x48. Sem SVG. Todas referenciadas.
- **Janela:** `InstrumentViewFixedSize` 250x250. A própria view não define fundo; três QWidgets filhos 250x250 em (0,0), cada um com `artwork.png` como fundo (`setWidgetBackground`), e só um visível por vez (`changePreset()`): preset < 9 -> ModalBar; preset == 9 -> TubeBell; preset > 9 -> BandedWG.
- **Controles:** todos `Knob(KnobType::Vintage32, <rótulo>, SMALL_FONT_SIZE, parent)` — o rótulo é desenhado pelo próprio Knob abaixo do botão, fonte 10 px.

Comuns (filhos da view, acima dos subpainéis):

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| ComboBox | `m_presetsModel` ("Instrument"): 0 Marimba, 1 Vibraphone, 2 Agogo, 3 Wood 1, 4 Reso, 5 Wood 2, 6 Beats, 7 Two fixed, 8 Clump, 9 Tubular bells, 10 Uniform bar, 11 Tuned bar, 12 Glass, 13 Tibetan bowl | 140, 50; 99x22 | — |
| Knob Vintage32, rótulo "Spread" | `m_spreadModel` ("Spread") 0..255 int | 190, 140 | "Spread:" "" |
| Knob Vintage32, rótulo "Random" | `m_randomModel` ("Randomness") 0..1 | 190, 190 | "Random:" "" |

Subpainel ModalBar (presets 0-8):

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Knob Vintage32 "Hardness" | `m_hardnessModel` ("Hardness") 0..128 | 30, 90 | "Hardness:" |
| Knob Vintage32 "Position" | `m_positionModel` ("Position") 0..64 | 110, 90 | "Position:" |
| Knob Vintage32 "Stick mix" | `m_stickModel` ("Stick mix") | 190, 90 | "Stick mix:" |
| Knob Vintage32 "Vibrato gain" | `m_vibratoGainModel` ("Vibrato gain") | 30, 140 | "Vibrato gain:" |
| Knob Vintage32 "Vibrato frequency" | `m_vibratoFreqModel` ("Vibrato frequency") | 110, 140 | "Vibrato frequency:" |

Subpainel TubeBell (preset 9):

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Knob Vintage32 "Modulator" | `m_modulatorModel` ("Modulator") | 30, 90 | "Modulator:" |
| Knob Vintage32 "Crossfade" | `m_crossfadeModel` ("Crossfade") | 110, 90 | "Crossfade:" |
| Knob Vintage32 "ADSR" | `m_adsrModel` ("ADSR") | 190, 90 | "ADSR:" |
| Knob Vintage32 "LFO speed" | `m_lfoSpeedModel` ("LFO speed") | 30, 140 | "LFO speed:" |
| Knob Vintage32 "LFO depth" | `m_lfoDepthModel` ("LFO depth") | 110, 140 | "LFO depth:" |

Subpainel BandedWG (presets 10-13):

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Knob Vintage32 "Pressure" | `m_pressureModel` ("Pressure") 0.1..128 | 30, 90 | "Pressure:" |
| Knob Vintage32 "Speed" | `m_velocityModel` ("Speed") 0.1..128 | 30, 140 | "Speed:" |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum `paintEvent`.
- **Notas:** `m_strikeModel` ("Bowed") não tem widget (LedCheckBox comentado em (138,25)). Se os arquivos de onda do STK faltam (`m_filesMissing`), aparece um QLabel de aviso em (10,10) 230x70, quebra de linha, centrado, stylesheet `background: rgba(0,0,0,200); color: #ff6b6b; padding: 6px;`, texto "STK's wave files were not found, so this instrument plays nothing. Install the STK package, or set its folder in Settings, Paths.". Os rótulos dos knobs são desenhados pelo Knob (não estão na arte); a arte é a mesma para os três subpainéis. Todos os modelos de 0..128 têm passo 0.1 e padrão 64, exceto Vibrato gain/frequency e Stick mix (padrão 0).

### TripleOscillator

- **Classificação:** visual próprio (artwork, knobs Styled, PixmapButtons de modulação e forma de onda); **Categoria:** instrumento
- **Arquivos:** `plugins/TripleOscillator/TripleOscillator.cpp` (`TripleOscKnob`, `TripleOscillatorView`, linhas ~400-713), `plugins/TripleOscillator/TripleOscillator.h` (`NUM_OF_OSCILLATORS = 3`). Imagens (`plugins/TripleOscillator/`): `artwork.png` 250x250; `pm_`, `am_`, `mix_`, `sync_`, `fm_` `active/inactive.png` 35x12; `sin_shape_`, `triangle_shape_`, `saw_shape_`, `square_shape_`, `moog_saw_shape_`, `exp_shape_`, `white_noise_shape_` `active/inactive.png` 15x16; `usr_shape_active/inactive.png` 14x16; `wavetable_active/inactive.png` 15x16; `logo.png` 48x48. Sem SVG. Todas referenciadas.
- **Janela:** 250x250; fundo `artwork.png`.
- **Constantes locais:** `mod_x = 66`, `mod1_y = 58`, `mod2_y = 75`, `osc_y = 109`, `osc_h = 52`; knobs `knob_y = 109 + i*52` (109, 161, 213); botões de onda `btn_y = 96 + i*52` (96, 148, 200).
- **Controles:** `TripleOscKnob` = `Knob(KnobType::Styled)` `setFixedSize(28,35)`; volume = `VolumeKnob(KnobType::Styled)` 28x35. Tema (`TripleOscillatorView Knob`): `color rgb(255,255,255)`, `outerColor rgb(255,255,255)`, `innerRadius 2`, `outerRadius 7`, `centerPoint (13.0, 14.0)`, `lineWidth 2`.

Modulação (dois grupos exclusivos; valores = `Oscillator::ModulationAlgo`: 0 PhaseModulation, 1 AmplitudeModulation, 2 SignalMix (padrão), 3 SynchronizedBySubOsc, 4 FrequencyModulation):

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| AutomatableButtonGroup `m_mod1BtnGrp` | `m_osc[0]->m_modulationAlgoModel` ("Modulation type 1") | — | — |
| PixmapButton `pm_active/inactive` 35x12 | grupo 1, 0 | 66, 58 | "Modulate phase of oscillator 1 by oscillator 2" |
| PixmapButton `am_` | grupo 1, 1 | 101, 58 | "Modulate amplitude of oscillator 1 by oscillator 2" |
| PixmapButton `mix_` | grupo 1, 2 | 136, 58 | "Mix output of oscillators 1 & 2" |
| PixmapButton `sync_` | grupo 1, 3 | 171, 58 | "Synchronize oscillator 1 with oscillator 2" |
| PixmapButton `fm_` | grupo 1, 4 | 206, 58 | "Modulate frequency of oscillator 1 by oscillator 2" |
| AutomatableButtonGroup `m_mod2BtnGrp` | `m_osc[1]->m_modulationAlgoModel` ("Modulation type 2") | — | — |
| PixmapButton `pm_` | grupo 2, 0 | 66, 75 | "Modulate phase of oscillator 2 by oscillator 3" |
| PixmapButton `am_` | grupo 2, 1 | 101, 75 | "Modulate amplitude of oscillator 2 by oscillator 3" |
| PixmapButton `mix_` | grupo 2, 2 | 136, 75 | "Mix output of oscillators 2 & 3" |
| PixmapButton `sync_` | grupo 2, 3 | 171, 75 | "Synchronize oscillator 2 with oscillator 3" |
| PixmapButton `fm_` | grupo 2, 4 | 206, 75 | "Modulate frequency of oscillator 2 by oscillator 3" |

Por oscilador (i = 0..2, N = i+1; y knobs 109/161/213; y botões 96/148/200):

| Widget | Modelo (label) | Posição (x; y osc1/osc2/osc3) | Dica |
|---|---|---|---|
| VolumeKnob Styled 28x35 | `m_volumeModel` ("Osc N volume") 0..200, padrão 20 | x=6 | "Osc N volume:" + "%" |
| TripleOscKnob 28x35 | `m_panModel` ("Osc N panning") -100..100 | x=35 | "Osc N panning:" "" |
| TripleOscKnob | `m_coarseModel` ("Osc N coarse detuning") -24..24, padrão -12*i | x=82 | "Osc N coarse detuning:" + " semitones" |
| TripleOscKnob | `m_fineLeftModel` ("Osc N fine detuning left") -100..100 | x=111 | "Osc N fine detuning left:" + " cents" |
| TripleOscKnob | `m_fineRightModel` ("Osc N fine detuning right") -100..100 | x=140 | "Osc N fine detuning right:" + " cents" |
| TripleOscKnob | `m_phaseOffsetModel` ("Osc N phase-offset") 0..360 | x=188 | "Osc N phase-offset:" + " degrees" |
| TripleOscKnob | `m_stereoPhaseDetuningModel` ("Osc N stereo phase-detuning") 0..360 | x=217 | "Osc N stereo phase-detuning:" + " degrees" |
| PixmapButton checkable `wavetable_active/inactive` 15x16 | `m_useWaveTableModel` (BoolModel padrão true, sem label) | x=110 (y botões) | "Use alias-free wavetable oscillators." |
| AutomatableButtonGroup `wsbg` | `m_waveShapeModel` ("Osc N wave shape"), padrão Sine | — | — |
| PixmapButton `sin_shape_` 15x16 | onda 0 Sine | x=128 | "Sine wave" |
| PixmapButton `triangle_shape_` | 1 Triangle | x=143 | "Triangle wave" |
| PixmapButton `saw_shape_` | 2 Saw | x=158 | "Saw wave" |
| PixmapButton `square_shape_` | 3 Square | x=173 | "Square wave" |
| PixmapButton `moog_saw_shape_` | 4 MoogSaw | x=188 | "Moog-like saw wave" |
| PixmapButton `exp_shape_` | 5 Exponential | x=203 | "Exponential wave" |
| PixmapButton `white_noise_shape_` | 6 WhiteNoise | x=218 | "White noise" |
| PixmapButton `usr_shape_` 14x16 | 7 UserDefined; `doubleClicked()` -> `oscUserDefWaveDblClick()` (abre `FileDialog::openWaveformFile`) | x=233 | "User-defined wave" |

Coordenadas expandidas: osc1 knobs y=109, botões y=96; osc2 knobs y=161, botões y=148; osc3 knobs y=213, botões y=200.

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum.
- **Notas:** o oscilador 3 não tem grupo de modulação (só há mod1 e mod2). Os knobs Styled 28x35 têm o centro (13,14) definido pelo tema; o espaço abaixo é área de rótulo da arte. Comentário "// 82, 109" solto no código (posição do coarse knob do osc1).

### Vibed

- **Classificação:** visual próprio (artwork, Graph com foreground próprio, seletores de 9 botões com pixmaps); **Categoria:** instrumento
- **Arquivos:** `plugins/Vibed/Vibed.cpp` (`VibedView`, linhas ~266-520), `plugins/Vibed/Vibed.h` (`s_sampleLength = 128`, `s_stringCount = 9`), `plugins/Vibed/NineButtonSelector.cpp/.h`. Imagens (`plugins/Vibed/`): `artwork.png` 250x250; `wavegraph4.png` 132x104 (foreground do Graph); `button_-2_on/off`, `button_-1_on/off`, `button_f_on/off`, `button_1_on/off` ... `button_9_on/off` 14x14; `smooth_active/inactive.png` 14x14; `normalize_active/inactive.png` 14x14; `logo.png` 48x48. Do tema (`embed::getIconPixmap`, `data/themes/default/`): `sin_wave_active/inactive.png`, `triangle_wave_active/inactive.png`, `saw_wave_active/inactive.png`, `square_wave_active/inactive.png`, `white_noise_wave_active/inactive.png`, `usr_wave_active/inactive.png`, todos 14x14. Sem SVG.
  - **Não referenciadas no código:** `plugins/Vibed/button_up.png` e `plugins/Vibed/button_down.png` (14x14).
- **Janela:** 250x250; fundo `artwork.png`. Menu de contexto da view: `CaptionMenu` com o displayName do modelo.
- **Controles:** a view mostra UMA corda por vez; `showString(str)` religa todos os widgets aos modelos da corda `str` (0..8) escolhida no seletor "String". Modelos são arrays `m_xxxModels[str]`, labels com N = str+1.

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| VolumeKnob(KnobType::Bright26) | `m_volumeModels[s]` ("String N volume") 0..200, padrão DefaultVolume (100) | 103, 142 | "String volume:" + "%" |
| Knob Bright26 | `m_stiffnessModels[s]` ("String N stiffness") 0..0.05 | 129, 142 | "String stiffness:" |
| Knob Bright26 | `m_pickModels[s]` ("Pick N position") 0..0.05 | 153, 142 | "Pick position:" |
| Knob Bright26 | `m_pickupModels[s]` ("Pickup N position") 0..0.05, padrão 0.05 | 177, 142 | "Pickup position:" |
| Knob Bright26 | `m_panModels[s]` ("String N panning") -1..1 | 105, 187 | "String panning:" |
| Knob Bright26 | `m_detuneModels[s]` ("String N detune") -0.1..0.1 | 150, 187 | "String detune:" |
| Knob Bright26 | `m_randomModels[s]` ("String N fuzziness") 0..0.75 | 194, 187 | "String fuzziness:" |
| Knob Bright26 | `m_lengthModels[s]` ("String N length") 1..16 | 23, 193 | "String length:" |
| Graph (Style::Linear padrão) com `setForeground(wavegraph4)` | `m_graphModels[s]` (graphModel -1..1, 128 amostras, inicia senoidal) | 76, 21; 132x104 | windowTitle "Impulse Editor" |
| LedCheckBox("" , cor padrão Yellow) | `m_impulseModels[s]` ("Impulse N") | 23, 94 | tooltip "Impulse" |
| LedCheckBox("", nome "Enable waveform", Yellow) | `m_powerModels[s]` ("String N"; padrão true só na corda 1) | 212, 130 | "Enable/disable string" |
| NineButtonSelector "Octave" (`m_harmonic`) | `m_harmonicModels[s]` (NineButtonSelectorModel 0..8, padrão 2) | 21, 127; 50x50 | windowTitle "Octave" (título do menu de contexto) |
| NineButtonSelector "String" (`m_stringSelector`) | modelo interno próprio (não salvo), padrão 0 | 21, 39; 50x50 | windowTitle "String" |
| PixmapButton `smooth_active/inactive` 14x14 | `smoothClicked()` -> `graphModel::smooth()` | 79, 129 | "Smooth waveform" |
| PixmapButton `normalize_active/inactive` 14x14 | `normalizeClicked()` | 96, 129 | "Normalize waveform" |
| PixmapButton tema `sin_wave_*` 14x14 | `setWaveToSine()` | 212, 24 | "Sine wave" |
| PixmapButton tema `triangle_wave_*` | `setWaveToTriangle()` | 212, 41 | "Triangle wave" |
| PixmapButton tema `saw_wave_*` | `setWaveToSaw()` | 212, 58 | "Saw wave" |
| PixmapButton tema `square_wave_*` | `setWaveToSquare()` | 212, 75 | "Square wave" |
| PixmapButton tema `white_noise_wave_*` | `setWaveToNoise()` | 212, 92 | "White noise" |
| PixmapButton tema `usr_wave_*` | `setWaveToUser()` (tooltip vira o nome do arquivo) | 212, 109 | "User-defined wave" |

**NineButtonSelector** (widget próprio, `NineButtonSelector.cpp`): `QWidget` + `IntModelView`, `setFixedSize(50,50)`, `move(x,y)`; 9 `PixmapButton` em grade 3x3, botão i em (1 + (i%3)*17, 1 + (i/3)*17) relativos -> (1,1), (18,1), (35,1), (1,18), (18,18), (35,18), (1,35), (18,35), (35,35); ícones `onOffIcons[i*2]` = ativo, `[i*2+1]` = inativo; só o botão selecionado fica checked; emite `NineButtonSelection(int)`; menu de contexto `CaptionMenu(windowTitle())`. Modelo interno padrão `NineButtonSelectorModel(default, 0, 8, nullptr, "", true)`.
  - Octave (em 21,127): ordem dos botões i=0..8 = `-2`, `-1`, `f`, `2`, `3`, `4`, `5`, `6`, `7` (posições absolutas: -2 (22,128), -1 (39,128), f (56,128), 2 (22,145), 3 (39,145), 4 (56,145), 5 (22,162), 6 (39,162), 7 (56,162)); padrão 2 = "f" (fundamental).
  - String (em 21,39): botões `1`..`9` (absolutas: 1 (22,40), 2 (39,40), 3 (56,40), 4 (22,57), 5 (39,57), 6 (56,57), 7 (22,74), 8 (39,74), 9 (56,74)); padrão 0 = corda 1.

- **Visores vivos:** `Graph` em (76,21) 132x104: desenha `m_samples` do graphModel da corda (128 pontos, -1..1) por cima do foreground `wavegraph4.png`; cor padrão do Graph `QColor(0xFF,0xAA,0x00)`, linha 1 px antialias, estilo Linear (cíclico); editável com o mouse. Atualiza quando o modelo muda.
- **Desenho próprio:** nenhum `paintEvent` na view.
- **Notas:** os botões de forma de onda não são checkable (ações momentâneas sobre o graph). O botão "Octave" e os knobs são todos por corda. `button_8`/`button_9` só aparecem no seletor de corda; `button_1` só no de corda (o de oitava usa `f`).

### Watsyn

- **Classificação:** visual próprio (artwork, 4 Graphs com fundo `wavegraph.png`, knobs Styled coloridos, muitos PixmapButtons); **Categoria:** instrumento
- **Arquivos:** `plugins/Watsyn/Watsyn.cpp` (`WatsynView`, linhas ~711-1310), `plugins/Watsyn/Watsyn.h` (`#define A1ROW 26`, `A2ROW 49`, `B1ROW 72`, `B2ROW 95`; `GRAPHLEN = 220`; `MOD_MIX=0, MOD_AM=1, MOD_RM=2, MOD_PM=3`; `A1_OSC=0, A2_OSC=1, B1_OSC=2, B2_OSC=3`; template `makeKnob` linhas 351-360). Imagens (`plugins/Watsyn/`): `artwork.png` 250x250; `wavegraph.png` 224x105; `a1_`, `a2_`, `b1_`, `b2_` `active/inactive.png` 40x16; `amix_`, `aam_`, `arm_`, `apm_`, `bmix_`, `bam_`, `brm_`, `bpm_` `active/inactive.png` 40x16; `load_`, `phl_`, `phr_`, `norm_`, `inv_`, `smooth_`, `sin_`, `tri_`, `saw_`, `sqr_` `active/inactive.png` 16x16; `logo.png` 48x48. Sem SVG. Todas referenciadas.
- **Janela:** 250x250; fundo `artwork.png`.
- **Controles:** `makeKnob<T>(x, y, hint, unit, objName)`: `T(KnobType::Styled, this)`, `move(x,y)`, `setHintText(hint, unit)`, `setObjectName(objName)`, `setFixedSize(19,19)`. Tema (`WatsynView Knob`): `innerRadius 1`, `outerRadius 7`, `centerPoint (9.5,9.5)`, `lineWidth 2`; `#aKnob` color/outer `#43b2ff`; `#bKnob` `#fc5431`; `#mixKnob` `#43ff82`, `outerRadius 13`, `centerPoint (15.5,15.5)`; `#mixenvKnob` `#43ff82`; `#xtalkKnob` `#fb50fb`.

Knobs dos osciladores (linhas y: A1=26, A2=49, B1=72, B2=95; colunas x: vol 130, pan 154, mult 178, ltune 202, rtune 226):

| Widget (objectName) | Modelo (label) | Posição | Dica |
|---|---|---|---|
| VolumeKnob Styled 19x19 (`aKnob`) | `a1_vol` ("Volume A1") 0..200, padrão 32 | 130, 26 | "Volume" + "%" |
| VolumeKnob (`aKnob`) | `a2_vol` ("Volume A2") | 130, 49 | "Volume" + "%" |
| VolumeKnob (`bKnob`) | `b1_vol` ("Volume B1") | 130, 72 | "Volume" + "%" |
| VolumeKnob (`bKnob`) | `b2_vol` ("Volume B2") | 130, 95 | "Volume" + "%" |
| Knob Styled (`aKnob`) | `a1_pan` ("Panning A1") -100..100 | 154, 26 | "Panning" "" |
| Knob (`aKnob`) | `a2_pan` ("Panning A2") | 154, 49 | "Panning" |
| Knob (`bKnob`) | `b1_pan` ("Panning B1") | 154, 72 | "Panning" |
| Knob (`bKnob`) | `b2_pan` ("Panning B2") | 154, 95 | "Panning" |
| Knob (`aKnob`) | `a1_mult` ("Freq. multiplier A1") 1..24, padrão 8 | 178, 26 | "Freq. multiplier" + "/8" |
| Knob (`aKnob`) | `a2_mult` ("Freq. multiplier A2") | 178, 49 | idem |
| Knob (`bKnob`) | `b1_mult` ("Freq. multiplier B1") | 178, 72 | idem |
| Knob (`bKnob`) | `b2_mult` ("Freq. multiplier B2") | 178, 95 | idem |
| Knob (`aKnob`) | `a1_ltune` ("Left detune A1") -600..600 | 202, 26 | "Left detune" + " cents" |
| Knob (`aKnob`) | `a2_ltune` ("Left detune A2") | 202, 49 | idem |
| Knob (`bKnob`) | `b1_ltune` ("Left detune B1") | 202, 72 | idem |
| Knob (`bKnob`) | `b2_ltune` ("Left detune B2") | 202, 95 | idem |
| Knob (`aKnob`) | `a1_rtune` ("Right detune A1") -600..600 | 226, 26 | "Right detune" + " cents" |
| Knob (`aKnob`) | `a2_rtune` ("Right detune A2") | 226, 49 | idem |
| Knob (`bKnob`) | `b1_rtune` ("Right detune B1") | 226, 72 | idem |
| Knob (`bKnob`) | `b2_rtune` ("Right detune B2") | 226, 95 | idem |

Mix / envelope / crosstalk:

| Widget (objectName) | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Knob Styled (`mixKnob`) 31x31 (redimensionado após o 19x19) | `m_abmix` ("A-B Mix") -100..100 | 4, 3 | "A-B Mix" "" |
| Knob (`mixenvKnob`) 19x19 | `m_envAmt` ("A-B Mix envelope amount") -200..200 | 88, 3 | "Mix envelope amount" |
| TempoSyncKnob Styled (`mixenvKnob`) | `m_envAtt` ("A-B Mix envelope attack") 0..2000 ms | 88, 26 | "Mix envelope attack" + " ms" |
| TempoSyncKnob (`mixenvKnob`) | `m_envHold` ("A-B Mix envelope hold") | 88, 49 | "Mix envelope hold" + " ms" |
| TempoSyncKnob (`mixenvKnob`) | `m_envDec` ("A-B Mix envelope decay") | 88, 72 | "Mix envelope decay" + " ms" |
| Knob (`xtalkKnob`) | `m_xtalk` ("A1-B2 Crosstalk") 0..100 | 88, 95 | "Crosstalk" "" |
| LedCheckBox("Anti-aliasing", Green) | `m_antiAliasing` ("Anti-aliasing") padrão true | 150, 2 | "High notes without the tones their harmonics make when they fold back below half the sample rate" |

Botões:

| Widget | Modelo / ação | Posição | Dica |
|---|---|---|---|
| AutomatableButtonGroup `m_selectedGraphGroup` | `m_selectedGraph` ("Selected graph") 0..3 | — | cliques chamam `updateLayout()` |
| PixmapButton `a1_active/inactive` 40x16 | seleção 0 (A1) | 4, 121 | "Select oscillator A1" |
| PixmapButton `a2_` | 1 (A2) | 44, 121 | "Select oscillator A2" |
| PixmapButton `b1_` | 2 (B1) | 84, 121 | "Select oscillator B1" |
| PixmapButton `b2_` | 3 (B2) | 124, 121 | "Select oscillator B2" |
| AutomatableButtonGroup `m_aModGroup` | `m_amod` ("A2-A1 modulation") 0..3 | — | — |
| PixmapButton `amix_` 40x16 | MOD_MIX 0 | 4, 50 | "Mix output of A2 to A1" |
| PixmapButton `aam_` | MOD_AM 1 | 4, 66 | "Modulate amplitude of A1 by output of A2" |
| PixmapButton `arm_` | MOD_RM 2 | 4, 82 | "Ring modulate A1 and A2" |
| PixmapButton `apm_` | MOD_PM 3 | 4, 98 | "Modulate phase of A1 by output of A2" |
| AutomatableButtonGroup `m_bModGroup` | `m_bmod` ("B2-B1 modulation") | — | — |
| PixmapButton `bmix_` 40x16 | 0 | 44, 50 | "Mix output of B2 to B1" |
| PixmapButton `bam_` | 1 | 44, 66 | "Modulate amplitude of B1 by output of B2" |
| PixmapButton `brm_` | 2 | 44, 82 | "Ring modulate B1 and B2" |
| PixmapButton `bpm_` | 3 | 44, 98 | "Modulate phase of B1 by output of B2" |
| PixmapButton `load_` 16x16 | `loadClicked()` -> `setWaveToUser()` no graph selecionado | 173, 121 | "Load a waveform from a sample file" |
| PixmapButton `phl_` | `phaseLeftClicked()` | 193, 121 | "Shift phase by -15 degrees" |
| PixmapButton `phr_` | `phaseRightClicked()` | 210, 121 | "Shift phase by +15 degrees" |
| PixmapButton `norm_` | `normalizeClicked()` | 230, 121 | "Normalize" |
| PixmapButton `inv_` | `invertClicked()` | 230, 138 | "Invert" |
| PixmapButton `smooth_` | `smoothClicked()` | 230, 155 | "Smooth" |
| PixmapButton `sin_` | `sinWaveClicked()` | 230, 176 | "Sine wave" |
| PixmapButton `tri_` | `triWaveClicked()` | 230, 194 | "Triangle wave" |
| PixmapButton `saw_` | `sawWaveClicked()` | 230, 212 | "Saw wave" |
| PixmapButton `sqr_` | `sqrWaveClicked()` | 230, 230 | "Square wave" |

- **Visores vivos:** 4 `Graph(this, Graph::Style::Linear, 224, 105)` sobrepostos em (4, 141), só o selecionado visível (`updateLayout`); fundo via paleta `wavegraph.png` (224x105) com `setAutoFillBackground(true)`; cor da curva A1/A2 `QColor(0x43,0xb2,0xff)`, B1/B2 `QColor(0xfc,0x54,0x31)`; modelos `a1_graph`..`b2_graph` (graphModel -1..1, `GRAPHLEN = 220` pontos); tooltip "Draw your own waveform here by dragging your mouse on this graph."; editáveis com o mouse; redesenham quando o modelo muda.
- **Desenho próprio:** nenhum `paintEvent` próprio (só o Graph genérico).
- **Notas:** o botão de serra é criado com nome `tr("Triangle wave")` (erro de cópia), mas o tooltip é "Saw wave". `m_selectedGraphGroup->setModel` é chamado também no construtor (antes de `modelChanged`). Comentário no `.h` diz que GRAPHLEN "deve ser igual à largura do widget", mas o widget tem 224 (o Graph desenha com 2 px de margem: 224-4 = 220). Os botões de ação não são checkable.

### Xpressive

- **Classificação:** visual próprio (artwork, Graph com fundo `wavegraph.png`, editor de texto, PixmapButtons); **Categoria:** instrumento
- **Arquivos:** `plugins/Xpressive/Xpressive.cpp` (`XpressiveKnob`, `XpressiveView` linhas ~266-800, `XpressiveHelpView` ~800-870), `plugins/Xpressive/Xpressive.h` (`W1_EXPR=0, W2_EXPR=1, W3_EXPR=2, O1_EXPR=3, O2_EXPR=4`), `plugins/Xpressive/ExprSynth.cpp/.h` (motor, sem GUI). Imagens (`plugins/Xpressive/`): `artwork.png` 250x250; `wavegraph.png` 180x81; `w1_`, `w2_`, `w3_` `active/inactive.png` 22x14; `o1_`, `o2_` `active/inactive.png` 19x14; `help_active/inactive.png` 50x14; `logo.png` 48x48 (também ícone da janela de ajuda). Do tema (`embed::getIconPixmap`, 14x14 cada): `sin_wave_*`, `moog_saw_wave_*`, `exp_wave_*`, `saw_wave_*`, `usr_wave_*`, `triangle_wave_*`, `square_wave_*`, `white_noise_wave_*` (`_active`/`_inactive`).
  - **SVGs existem e NÃO são referenciados no código:** `plugins/Xpressive/Xpressive.svg` (width="17in" height="11in", viewBox="0 0 431.8 279.4"; fonte da arte) e `plugins/Xpressive/Xpressive_logo.svg` (width="10in" height="10in", viewBox="0 0 254 254").
- **Janela:** 250x250; fundo `artwork.png`.
- **Constantes locais:** `COL_KNOBS=191`, `BASE_START=2`, `ROW_KNOBSA1=2`, `ROW_KNOBSA2=34`, `ROW_KNOBSA3=66`, `ROW_KNOBSP1=102`, `ROW_KNOBSP2=134`, `ROW_KNOBREL=166`, `ROW_BTN=87`, `ROW_WAVEBTN=2+233-26=209`, `EXPR_TEXT_Y=104`, `EXPR_TEXT_H=90`.
- **Controles:** `XpressiveKnob` = `Knob(KnobType::Styled, parent, name)` com `setFixedSize(29,29)`, `centerPoint (14.5,14.5)`, `innerRadius 4`, `outerRadius 9`, `totalAngle 300`, `lineWidth 3` (o terceiro argumento é o nome do modelo-placeholder, não um rótulo desenhado). Não há regra de tema específica para `XpressiveView` (só a regra geral `Knob { lineInactiveColor rgb(120,120,120); arcInactiveColor rgba(120,120,120,70) }`; cores do arco vêm do padrão do Knob).

| Widget | Modelo (label) | Posição | Dica |
|---|---|---|---|
| Graph(Linear, 180x81), fundo `wavegraph.png`, cor `QColor(255,255,255)` | graph do item selecionado (ver Visores) | 3, 3 | "Draw your own waveform here by dragging your mouse on this graph." |
| AutomatableButtonGroup `m_selectedGraphGroup` | `m_selectedGraph` ("Selected graph") 0..6 (usa 0..4) | — | cliques chamam `updateLayout()` |
| PixmapButton `w1_active/inactive` 22x14 | seleção 0 (W1) | 3, 87 | "Select oscillator W1" |
| PixmapButton `w2_` | 1 (W2) | 26, 87 | "Select oscillator W2" |
| PixmapButton `w3_` | 2 (W3) | 49, 87 | "Select oscillator W3" |
| PixmapButton `o1_` 19x14 | 3 (O1) | 79, 87 | "Select output O1" |
| PixmapButton `o2_` | 4 (O2) | 101, 87 | "Select output O2" |
| PixmapButton `help_active/inactive` 50x14 (fora do grupo) | `helpClicked()` | 133, 87 | "Open help window" |
| QPlainTextEdit `m_expressionEditor` | expressão de W1..W3 / O1..O2 (texto) | 3, 104; 180x90 | — |
| LedCheckBox("", Red), desabilitado | `m_exprValid` (BoolModel; 1 = expressão inválida) | 168, 192 (EXPR_TEXT_Y+EXPR_TEXT_H-2) | — |
| Knob Styled "Smoothness" 25x25, centro (12.5,12.5), inner 4, outer 9, ângulo 280, linha 3 | `m_smoothW1/2/3` ("W1/W2/W3 smoothing") 0..70 (só em W1..W3) | 66, 198 (EXPR_TEXT_Y+EXPR_TEXT_H+4) | "Smoothness" "" |
| LedCheckBox("Interpolate", Green) | `m_interpolateW1/2/3` (sem label; só em W1..W3) | 2, 230 | — |
| PixmapButton tema `moog_saw_wave_*` 14x14 | anexa "moogsaww(t)" / "moogsaww(integrate(f))" | 4, 195 | "Moog-saw wave" |
| PixmapButton tema `exp_wave_*` | "expw(t)" / "expw(integrate(f))" | 18, 195 | "Exponential wave" |
| PixmapButton tema `saw_wave_*` | "saww(t)" / "saww(integrate(f))" | 32, 195 | "Saw wave" |
| PixmapButton tema `usr_wave_*` (só em W1..W3) | limpa texto e `setWaveToUser()` | 46, 195 | "User-defined wave" |
| PixmapButton tema `sin_wave_*` | "sinew(t)" / "sinew(integrate(f))" | 4, 209 | "Sine wave" |
| PixmapButton tema `triangle_wave_*` | "trianglew(t)" / "trianglew(integrate(f))" | 18, 209 | "Triangle wave" |
| PixmapButton tema `square_wave_*` | "squarew(t)" / "squarew(integrate(f))" | 32, 209 | "Square wave" |
| PixmapButton tema `white_noise_wave_*` | "randsv(t*srate,0)" | 46, 209 | "White noise" |
| XpressiveKnob 29x29 (nome "A1") | `m_parameterA1` ("A1") -1..1, padrão 1 | 191, 2 | "General purpose 1:" "" |
| XpressiveKnob ("A2") | `m_parameterA2` ("A2") | 191, 34 | "General purpose 2:" |
| XpressiveKnob ("A3") | `m_parameterA3` ("A3") | 191, 66 | "General purpose 3:" |
| XpressiveKnob ("O1 panning") | `m_panning1` ("Panning 1") -1..1, padrão 1 | 191, 102 | "O1 panning:" |
| XpressiveKnob ("O2 panning") | `m_panning2` ("Panning 2") padrão -1 | 191, 134 | "O2 panning:" |
| XpressiveKnob ("Release transition") | `m_relTransition` ("Rel trans") 0..500 | 191, 166 | "Release transition:" + "ms" (sem espaço) |

- **Visores vivos:** `Graph` em (3,3) 180x81, curva branca `QColor(255,255,255)`, estilo Linear, fundo `wavegraph.png` (paleta + autoFill). Modelo trocado em `updateLayout()`: W1/W2/W3 -> `m_graphW1..3` (4096 pontos, `GRAPH_LENGTH`), O1/O2 -> `m_graphO1/O2` (360 pontos). Em W1..W3 é editável com o mouse só se smoothing = 0 e o texto estiver vazio; em O1/O2 fica desabilitado. Cada edição do texto (`textChanged`) recompila a expressão (exprtk) e reamostra o graph (t de 0 a 1 em `length` passos; inf/NaN viram 0); os knobs A1..A3 (`sliderMoved`) também disparam a reavaliação; o LED vermelho acende quando a expressão não compila.
- **Desenho próprio:** nenhum `paintEvent`.
- **Notas:**
  - `updateLayout()` mostra/esconde: em O1/O2 escondem-se o knob Smoothness, o botão `usr_wave` e o LED Interpolate.
  - Os botões de onda são ações (não checkable): anexam texto ao editor; em O1/O2 usam a forma com `integrate(f)`.
  - Janela de ajuda `XpressiveHelpView` (QTextEdit com HTML fixo `s_helpText`, título "Xpressive Help", só leitura/seleção), janela MDI fixa 300x500, sem botão maximizar, ícone `logo`; singleton.
  - O LED "Interpolate" usa texto literal sem `tr`.
  - Expressões padrão: O1 = `0.5*sinew(integrate(f*(1+0.05sinew(12t))))*(2^(-(1.1+A2)*t)*(0.4+0.1(1+A3)+0.4sinew((2.5+2A1)t))^2)`; O2 = `0.5*(expw(integrate(f*atan(500t)*2/pi))*0.5+0.12)`.


## Efeitos com visual próprio

Amplifier e StereoMatrix estão aqui porque têm arte própria, mas são **obsoletos no navegador** (F-FX-005, F-FX-028): os plugins `amplifier` e `stereomatrix` existem em `lm-plugins-effects` só para abrir projetos antigos (`decisoes-autonomas.md`, "Os efeitos do legado como plugins").

### AmplifikationLite

- **Classificação:** visual próprio (fundo em bitmap, knobs e chaves de filmstrip em posições absolutas, botões invisíveis em zonas de clique). **Categoria:** efeito.
- **Arquivos:**
  - `plugins/AmplifikationLite/AmplifikationLiteControls.{h,cpp}` (diálogo, `FilmstripKnob`, `ToggleSwitch`), `AmplifikationLite.cpp` (descritor, logo).
  - Imagens (todas embutidas por `EMBEDDED_RESOURCES *.png`, todas referenciadas pelo código; não existe SVG):

    | Arquivo | Tamanho | Uso |
    |---|---|---|
    | `plugins/AmplifikationLite/background.png` | 600 x 230 | fundo do painel (`setPalette`), define o tamanho da janela |
    | `plugins/AmplifikationLite/silverknob.png` | 36 x 3636 | filmstrip vertical dos knobs: **101 quadros de 36 x 36** (3636 / 36) |
    | `plugins/AmplifikationLite/silverswitch.png` | 31 x 117 | filmstrip vertical da chave de 3 posições: **3 quadros de 31 x 39** (117 / 39) |
    | `plugins/AmplifikationLite/irload.png` | 28 x 28 | ícone do botão de carregar IR |
    | `plugins/AmplifikationLite/logo.png` | 193 x 62 | logo do descritor (`PluginPixmapLoader("logo")`, navegador de plugins) |
    | `plugins/AmplifikationLite/about.png` | 311 x 206 | imagem da caixa "About" |

  - **Proveniência:** toda a arte é da Kuassa (Amplifikation Lite original). Permissão de uso dada pelo Arie (chairman e desenvolvedor de DSP da Kuassa), confirmada pelo usuário em **18/11** (README do plugin: "Permissão para distribuí-lo confirmada pelo usuário em 18/11, assim como a das imagens"; CMakeLists: "the artwork (*.png) is Kuassa's, used with permission"; memória do projeto: "Arie OK'd the Resources images"). O README recomenda guardar essa permissão **por escrito**: hoje ela está registrada só no README, no CMakeLists e no texto da caixa About, sem documento da Kuassa anexado ao repositório. O IR do gabinete INT (`CabinetIr` em `AmpLiteData.h`) também é da Kuassa (medido), com a mesma permissão de 18/11.
  - README do plugin (`plugins/AmplifikationLite/README.md`): diz que as "Posições na tela batem ao pixel com um screenshot do original"; o botão de IR é um só e só aparece em EXT; chaves sobem/descem uma posição por clique. Avisa que o VST original da Kuassa (`AmplifikationLite_113b_Mac_VST.zip` e a pasta, na raiz do repositório) **não** entra em commit (EULA proíbe redistribuir).
- **Janela:** `setFixedSize(background.size())` = **600 x 230**; fundo = `background.png` via `QPalette` (`setAutoFillBackground(true)`). As constantes de posição dizem: "the original's panel (600 x 230): each control's top-left corner, matched to the pixel against a screenshot of the original (Amplifikation-Lite-1.1.1-Screenshot.jpeg)".
- **Controles:**

  | Widget | Modelo (rótulo) | Posição (x, y, w, h) | Dica |
  |---|---|---|---|
  | `FilmstripKnob` (silverknob, quadro 36) | `m_input` tr("Input"), 0..1, padrão 0.2, passo 0.001 | 53, 146, 36, 36 | `setHintText("Input:", "")` |
  | `ToggleSwitch` (silverswitch) | `m_channel` tr("Channel"), 0..2 passo 1, padrão 0 (0 clean, 1 rhythm, 2 lead) | 141, 145, 31, 39 | tooltip "Channel: clean, rhythm, lead" |
  | `FilmstripKnob` | `m_gain` tr("Gain"), 0..1, padrão 0.5 | 168, 146, 36, 36 | "Gain:" |
  | `FilmstripKnob` | `m_bass` tr("Bass"), padrão 0.5 | 236, 146, 36, 36 | "Bass:" |
  | `FilmstripKnob` | `m_middle` tr("Middle"), padrão 0.5 | 293, 146, 36, 36 | "Middle:" |
  | `FilmstripKnob` | `m_treble` tr("Treble"), padrão 0.5 | 350, 146, 36, 36 | "Treble:" |
  | `FilmstripKnob` | `m_presence` tr("Presence"), padrão 0.5 | 406, 146, 36, 36 | "Presence:" |
  | `FilmstripKnob` | `m_volume` tr("Volume"), padrão 0.5 | 475, 146, 36, 36 | "Volume:" |
  | `ToggleSwitch` (silverswitch) | `m_cabinet` tr("Cabinet"), 0..2 passo 1, padrão 0 (0 INT, 1 OFF, 2 EXT) | 525, 145, 31, 39 | tooltip "Cabinet: INT (built-in), OFF, EXT (a loaded impulse response)" |
  | `QPushButton` plano, ícone `irload.png` 28x28, `background: transparent; border: none;`, cursor mão | abre seletor de arquivo do IR externo | 557, 160, 28, 28 | sem IR: "Load the EXT cabinet's impulse response"; com IR: "EXT cabinet: %1 (click to load another)" (%1 = nome base do arquivo) |
  | `QPushButton` plano invisível (zona de clique), transparente sem borda, cursor mão | abre a caixa About | 562, 198, 26, 26 (`AboutArea`) | tooltip "About" |

  - Os 7 knobs são `FloatModelEditorBase` com manipulação **vertical** (arrastar para cima/baixo), cursor `PointingHandCursor`.
  - **Desenho do knob:** quadro = `lround(normValue * (frames - 1))`, frames = 101; desenha a fatia `(0, quadro*36, 36, 36)` da tira em (0,0).
  - **Chave de 3 posições:** quadro = `clamp(lround(valor), 0, 2)`; desenha `(0, posição*39, 31, 39)`. Quadro 0 = posição 0 (clean / INT), quadro 1 = 1 (rhythm / OFF), quadro 2 = 2 (lead / EXT).
  - **Zonas de clique da chave:** clique esquerdo com `y < altura/2` (= 19,5 px) sobe uma posição (`-1`), senão desce uma (`+1`), limitado a 0..2. Arrastar e soltar não fazem nada (`mouseMove`/`mouseRelease` vazios). Roda: `angleDelta().y() > 0` → -1, senão +1. Outros botões (direito etc.) vão para o `FloatModelEditorBase` (menu de contexto/automação).
  - **Botão de IR:** visível **só** quando `cabinet() == External` (atualiza em `m_cabinet.dataChanged` e `impulseResponseChanged`). Clique abre `FileDialog::getOpenFileName(tr("Impulse response"), filtro "Audio files (*.wav *.flac *.aif *.aiff *.ogg)")`; erro em `QMessageBox::warning("Amplifikation Lite", erro)`. O caminho é salvo no atributo `ir` do XML (relativo mais curto).
- **Caixa About:** `QDialog` modal, título tr("Amplifikation Lite"), `QVBoxLayout` com um `QLabel` mostrando `about.png` (311 x 206) e embaixo um `QLabel` com quebra de linha: "Modeled in LinuxMusic from measurements of Kuassa's Amplifikation Lite. Artwork by Kuassa, used with permission." Margens/espaçamento = padrões do Qt.
- **Visores vivos:** nenhum.
- **Desenho próprio:** só os `paintEvent` de filmstrip acima (nenhuma cor, fonte ou texto desenhado; todo o texto do painel está no `background.png`).
- **Notas:** `logo.png` só aparece no navegador de plugins (descritor). Ordem dos controles na tela, da esquerda para a direita: Input, Channel (chave), Gain, Bass, Middle, Treble, Presence, Volume, Cabinet (chave), botão IR (só em EXT), About (canto inferior direito). Os knobs têm y = 146 e as chaves y = 145 (1 px acima).

### Amplifier

- **Classificação:** visual próprio (fundo artwork.svg), controles em grade; **Categoria:** efeito
- **Arquivos:** `plugins/Amplifier/AmplifierControlDialog.cpp/.h`, `AmplifierControls.cpp/.h`. Imagem: `plugins/Amplifier/artwork.svg` (width="100" height="110", sem viewBox).
- **Janela:** por layout (`QGridLayout`, `SetFixedSize`); artwork pretende 100x110. Fundo: artwork = gradiente base + linha horizontal em y = 55 (`M100 55H0`, stroke `#191a20`, 2 px) dividindo as duas linhas de knobs.
- **Controles:** grade 2x2, cada célula `Qt::AlignHCenter`.

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `VolumeKnob` Bright26, rótulo tr("VOL") | `m_volumeModel` tr("Volume"), 100, 0..200 | linha 0, col 0 | "Volume:" "%" |
| `Knob` Bright26, tr("PAN") | `m_panModel` tr("Panning"), 0, -100..100 | linha 0, col 1 | "Panning:" "%" |
| `VolumeKnob` Bright26, tr("LEFT") | `m_leftModel` tr("Left gain"), 100, 0..200 | linha 1, col 0 | "Left gain:" "%" |
| `VolumeKnob` Bright26, tr("RIGHT") | `m_rightModel` tr("Right gain"), 100, 0..200 | linha 1, col 1 | "Right gain:" "%" |

- **Visores vivos:** nenhum. **Desenho próprio:** nenhum (só o SVG).

### BassBooster

- **Classificação:** visual próprio (fundo artwork.svg liso), controles em layout; **Categoria:** efeito
- **Arquivos:** `plugins/BassBooster/BassBoosterControlDialog.cpp/.h`, `BassBoosterControls.cpp/.h`. Imagem: `plugins/BassBooster/artwork.svg` (width="128" height="64").
- **Janela:** por layout (`QVBoxLayout` com `addSpacing(4)` no topo, depois `QHBoxLayout`; `SetFixedSize`); artwork 128x64 = só o gradiente base.
- **Controles:** uma linha horizontal: `Knob` Bright26 tr("FREQ") -> `m_freqModel` tr("Frequency") 100, 50..200, dica "Frequency:" "Hz"; `Knob` Bright26 tr("GAIN") -> `m_gainModel` tr("Gain") 1.0, 0.1..5.0, dica "Gain:" ""; `Knob` Bright26 tr("RATIO") -> `m_ratioModel` tr("Ratio") 2.0, 0.1..10, dica "Ratio:" "".
- **Visores vivos / Desenho próprio:** nenhum.

### Bitcrush

- **Classificação:** visual próprio (artwork.svg com dois painéis, controles em posições absolutas); **Categoria:** efeito
- **Arquivos:** `plugins/Bitcrush/BitcrushControlDialog.cpp/.h`, `BitcrushControls.cpp/.h`. Imagem: `plugins/Bitcrush/artwork.svg` (width="181" height="128").
- **Janela:** `setFixedSize(181, 128)`. Fundo artwork: gradiente base 181x128; dois painéis (gradiente `#1a1d21`->`#2d333a`, contorno preto 1 px, ry 2.065): painel IN em x 9.5, y 9.5, 40x109; painel OUT em x 131.5, y 9.5, 40x109; filetes de brilho (branco .1) em y 10..11 e sombra (preto .25) em y 117..118 dentro de cada painel. A coluna do meio (x ≈ 50..131) fica sem painel.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `QLabel` tr("IN"), fonte 12 px (`DEFAULT_FONT_SIZE`) | — | (24, 15) | — |
| `QLabel` tr("OUT"), fonte 12 px | — | (139, 15) | — |
| `Knob` Bright26, rótulo tr("GAIN") 10 px | `m_inGain` tr("Input gain"), 0, -20..20 | (16, 32) | "Input gain:" " dBFS" |
| `Knob` Bright26, tr("NOISE") 10 px | `m_inNoise` tr("Input noise"), 0, 0..100 | (14, 76) | "Input noise:" "%" |
| `Knob` Bright26, tr("GAIN") 10 px | `m_outGain` tr("Output gain"), 0, -20..20 | (138, 32) | "Output gain:" " dBFS" |
| `Knob` Bright26, tr("CLIP") 10 px | `m_outClip` tr("Output clip"), 0, -20..20 | (138, 76) | "Output clip:" " dBFS" |
| `LedCheckBox` sem texto, Green | `m_rateEnabled` tr("Rate enabled"), true | (64, 14) | tooltip "Enable sample-rate crushing" |
| `LedCheckBox` sem texto, Green | `m_depthEnabled` tr("Depth enabled"), true | (101, 14) | tooltip "Enable bit-depth crushing" |
| `Knob` Bright26, tr("FREQ") 10 px | `m_rate` tr("Sample rate"), 44100, 20..44100 | (59, 32) | "Sample rate:" " Hz" |
| `Knob` Bright26, tr("STEREO") 10 px | `m_stereoDiff` tr("Stereo difference"), 0, -50..50 | (72, 76) | "Stereo difference:" "%" |
| `Knob` Bright26, tr("QUANT") 10 px | `m_levels` tr("Levels"), 256, 1..256 | (92, 32) | "Levels:" "" |

- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** o x de NOISE (14) difere do GAIN (16) para centrar o rótulo mais largo. `outClip->setHintText` tem indentação com espaços (irrelevante).

### Compressor

- **Classificação:** visual próprio (janela redimensionável, fundo 100% desenhado em código, gráfico grande com histórico rolante + curva de knee + grade; painel de controles é uma imagem `controlsBox` sobre o gráfico) ; **Categoria:** efeito
- **Arquivos:**
  - `plugins/Compressor/CompressorControlDialog.cpp` / `.h` (view), `CompressorControls.cpp` (modelos), `plugins/Eq/EqFader.h` (fader reaproveitado).
  - Imagens (todas referenciadas):
    - `controlsBox.png` 720x280 (painel de controles, QLabel)
    - `knob_enabled.png` 30x30 (anel atrás de knob pequeno opcional; usado 3x)
    - `knob_enabled_large.png` 56x56 (anel atrás do knob Ratio)
    - `rms_sel/unsel.png` 78x18; `peak_sel/unsel.png` 78x17
    - `leftright_sel/unsel.png` 93x18; `midside_sel/unsel.png` 93x17
    - `compressor_sel/unsel.png` 93x18; `limiter_sel/unsel.png` 93x18
    - `unlinked_sel/unsel.png` 78x18; `maximum_sel/unsel.png` 78x17; `average_sel/unsel.png` 78x17; `minimum_sel/unsel.png` 78x17; `blend_sel/unsel.png` 78x17
    - `autogain_sel/unsel.png` 93x18; `feedback_sel/unsel.png` 93x18
    - `audition_sel/unsel.png` 39x39
    - `lookahead_sel/unsel.png` 70x15
  - Sem SVG. Nenhuma imagem órfã.
- **Janela:** redimensionável. `setMinimumSize(800, 360)`, `resize(800, 560)` (padrão), `sizeHint()` = 800x560. Máximo prático 1920x1080 (os pixmaps internos têm `MAX_COMP_SCREEN_X/Y` = 1920x1080). `setAutoFillBackground(false)`, `WA_NoSystemBackground`. Fundo: `fillRect` com `backgroundColor` = rgba(7, 8, 9, 255) (#070809) em `paintEvent`.
  - Constantes (`CompressorControlDialog.h`): `COMP_MILLI_PER_PIXEL = 6`, `MIN_COMP_SCREEN 800x360`, `MAX_COMP_SCREEN 1920x1080`, `COMP_SCREEN 800x560`, `COMP_KNEE_LINES = 20`, `COMP_BOX_X = 720`, `COMP_BOX_Y = 280`, `COMP_GRID_SPACING = 3.f` dB, `COMP_GRID_MAX = 96.f` dB. `KNEE_SCREEN_X/Y` definidos mas não usados.
  - Origem do painel (recalculada em `resizeEvent` -> `resetCompressorView`): `boxX = (largura - 720) * 0.5`, `boxY = altura - 40 - 280`. No tamanho padrão 800x560: **boxX = 40, boxY = 240**. Todos os controles são posicionados relativos a (boxX, boxY); a coluna "Posição" dá o offset e, entre colchetes, a posição absoluta no tamanho padrão.
- **Controles:** knobs são todos `Knob(KnobType::Styled)`. CSS do tema (`data/themes/default/style.css:1180`): `color: #2fcc71; qproperty-outerColor: #2fcc71; qproperty-lineWidth: 2`.
  - "Large" (`makeLargeKnob`): `setFixedSize(56,56)`, outerRadius 23, innerRadius 15, centro (28,28).
  - "Small" (`makeSmallKnob`): `setFixedSize(30,30)`, outerRadius 10, innerRadius 4, centro (15,15).

| Widget | Modelo | Posição (offset no painel) [absoluta 800x560] | Dica |
|---|---|---|---|
| QLabel `controlsBox` (720x280), transparente a mouse | — | (0,0) [40,240] | — |
| QLabel `knob_enabled_large` 56x56 (atrás do Ratio) | visível se !limiter | (267,21) [307,261] | — |
| QLabel `knob_enabled` 30x30 (atrás do Lookahead Length) | visível se lookahead | (221,135) [261,375] | — |
| QLabel `knob_enabled` 30x30 (atrás do RMS) | visível se !peakmode | (429,209) [469,449] | — |
| QLabel `knob_enabled` 30x30 (atrás do Blend) | visível se stereoLink==4 | (587,197) [627,437] | — |
| Knob Styled Large 56x56 | `m_thresholdModel` tr("Threshold") (-8, -60..0) | (137,21) [177,261] | hint "Threshold:" " dBFS"; tooltip "Volume at which the compression begins to take place" |
| Knob Styled Large | `m_ratioModel` tr("Ratio") (1.8, 1..20) | (267,21) [307,261] | "Ratio:" ":1"; "How far the compressor must turn the volume down after crossing the threshold". Oculto quando Limiter |
| Knob Styled Large | `m_attackModel` tr("Attack") (10, 0.005..250) | (397,21) [437,261] | "Attack:" " ms"; "Speed at which the compressor starts to compress the audio" |
| Knob Styled Large | `m_releaseModel` tr("Release") (100, 1..2500) | (527,21) [567,261] | "Release:" " ms"; "Speed at which the compressor ceases to compress the audio" |
| Knob Styled Small 30x30 | `m_mixModel` tr("Mix") (100, 0..100) | (27,13) [67,253] | "Mix:" "%"; "Balance between wet and dry signals" |
| Knob Styled Small | `m_autoAttackModel` tr("Auto Attack") (0..100) | (460,38) [500,278] | "Auto Attack:" "%"; "Automatically control attack value depending on crest factor" |
| Knob Styled Small | `m_autoReleaseModel` tr("Auto Release") (0..100) | (590,38) [630,278] | "Auto Release:" "%"; "Automatically control release value depending on crest factor" |
| Knob Styled Small | `m_kneeModel` tr("Knee") (12, 0..96) | (97,135) [137,375] | "Knee:" " dB"; "Smooth out the gain reduction curve around the threshold" |
| Knob Styled Small | `m_rangeModel` tr("Range") (-240, -240..0) | (159,135) [199,375] | "Range:" " dBFS"; "Maximum gain reduction" |
| Knob Styled Small | `m_lookaheadLengthModel` tr("Lookahead Length") (0..20) | (221,135) [261,375] | "Lookahead Length:" " ms"; "How long the compressor has to react to the sidechain signal ahead of time". Visível só se Lookahead ligado |
| Knob Styled Small | `m_holdModel` tr("Hold") (0..500) | (283,135) [323,375] | "Hold:" " ms"; "Delay between attack and release stages" |
| Knob Styled Small | `m_tiltModel` tr("Tilt") (0, -6..6) | (364,138) [404,378] | "Tilt Gain:" " dB"; "Bias the sidechain signal to the low or high frequencies.  -6 db is lowpass, 6 db is highpass." |
| Knob Styled Small | `m_tiltFreqModel` tr("Tilt Frequency") (150, 20..20000) | (415,138) [455,378] | "Tilt Frequency:" " Hz"; "Center frequency of sidechain tilt filter" |
| Knob Styled Small | `m_stereoBalanceModel` tr("Stereo Balance") (-1..1) | (522,137) [562,377] | "Stereo Balance:" ""; "Bias the sidechain signal to the left/right or mid/side" |
| Knob Styled Small | `m_rmsModel` tr("RMS Size") (1, 0..250) | (429,209) [469,449] | "RMS Size:" ""; "Size of the RMS buffer". Visível só em modo RMS |
| Knob Styled Small | `m_blendModel` tr("Blend") (1, 0..3) | (587,197) [627,437] | "Stereo Link Blend:" ""; "Blend between unlinked/maximum/average/minimum stereo linking modes". Visível só com stereoLink = Blend |
| Knob Styled Small | `m_inBalanceModel` tr("Input Balance") (-1..1) | (27,219) [67,459] | "Input Balance:" ""; "Bias the input audio to the left/right or mid/side" |
| Knob Styled Small | `m_outBalanceModel` tr("Output Balance") (-1..1) | (662,219) [702,459] | "Output Balance:" ""; "Bias the output audio to the left/right or mid/side" |
| EqFader 23x116 (Fader, modelIsLinear=false), VU L/R de `m_inPeakL/R` | `m_inGainModel` tr("Input Gain") (0, -60..30); nome do fader tr("Input gain") | (31,91) [71,331] | hint "Gain" "dBFS"; tooltip "Input volume"; `setDisplayConversion(false)` |
| EqFader 23x116, VU de `m_outPeakL/R` | `m_outGainModel` tr("Output Gain") (0, -60..30); nome tr("Output gain") | (666,91) [706,331] | "Gain" "dBFS"; "Output volume" |
| PixmapButton `audition_sel/unsel` 39x39, checkable | `m_auditionModel` tr("Audition"); nome do botão tr("Soft Clip") (sic) | (658,14) [698,254] | "Play the delta signal" |
| PixmapButton `lookahead_sel/unsel` 70x15, checkable | `m_lookaheadModel` tr("Lookahead") | (202,171) [242,411] | "Enable Lookahead, which introduces 20 milliseconds of latency" |
| PixmapButton `feedback_sel/unsel` 93x18, checkable | `m_feedbackModel` tr("Feedback"); nome tr("Soft Clip") (sic) | (98,206) [138,446] | "Use the compressor's output as the sidechain input". Oculto quando Lookahead ligado |
| PixmapButton `autogain_sel/unsel` 93x18, checkable | `m_autoMakeupModel` tr("Auto Makeup Gain") | (220,206) [260,446] | "Automatically change makeup gain depending on threshold, knee, and ratio settings" |
| AutomatableButtonGroup: `compressor_sel/unsel` 93x18 | `m_limiterModel` tr("Limiter") valor 0 | (98,231) [138,471] | "Compress the audio" |
|  ... `limiter_sel/unsel` 93x18 | `m_limiterModel` valor 1 | (98,248) [138,488] | "Set Ratio to infinity (is not guaranteed to limit audio volume)" |
| AutomatableButtonGroup: `leftright_sel/unsel` 93x18 | `m_midsideModel` tr("Mid/Side") valor 0 | (220,231) [260,471] | "Compress left and right audio" |
|  ... `midside_sel/unsel` 93x17 | `m_midsideModel` valor 1 | (220,248) [260,488] | "Compress mid and side audio" |
| AutomatableButtonGroup: `rms_sel/unsel` 78x18 | `m_peakmodeModel` tr("Peak Mode") valor 0 | (337,231) [377,471] | "Use RMS of the input" |
|  ... `peak_sel/unsel` 78x17 | `m_peakmodeModel` valor 1 | (337,248) [377,488] | "Use absolute value of the input" |
| AutomatableButtonGroup: `unlinked_sel/unsel` 78x18 | `m_stereoLinkModel` tr("Stereo Link") (padrão 1) valor 0 | (495,180) [535,420] | "Compress each channel separately" |
|  ... `maximum_sel/unsel` 78x17 | valor 1 | (495,197) [535,437] | "Compress based on the loudest channel" |
|  ... `average_sel/unsel` 78x17 | valor 2 | (495,214) [535,454] | "Compress based on the averaged channel volume" |
|  ... `minimum_sel/unsel` 78x17 | valor 3 | (495,231) [535,471] | "Compress based on the quietest channel" |
|  ... `blend_sel/unsel` 78x17 | valor 4 | (495,248) [535,488] | "Blend between stereo linking modes" |

- **Visores vivos:** o próprio diálogo inteiro é o visor. Atualizado em `updateDisplay()` a cada `periodicUpdate` (60 Hz), mas a rolagem é por tempo real: `pixels = (ms acumulados) / 6` (`COMP_MILLI_PER_PIXEL`), o resto fica para o próximo tick; ou seja, ~166,7 px/s, independente do fps. Se o efeito não está processando, os picos de display são forçados para `COMP_NOISE_FLOOR = 0.000001` (-120 dBFS) e ganho 1.
  - Dados: `m_peakAvg` = média L/R de `m_displayPeak[i].take(m_currentPeak[i])` (pico máximo desde o último tick, nível do sidechain); `m_gainAvg` = média L/R de `m_displayGain[i].take(m_currentGain[i])` (ganho aplicado, linear). Convertidos para dBFS (`ampToDbfs`) e para Y.
  - Mapeamento: `dbfsToYPoint(d) = (-((d + dbRange) / dbRange) + 1) * altura` = `-d / dbRange * altura` (0 dBFS no topo, -dbRange na base). `dbfsToXPoint(d) = kneeH - dbfsToYPoint(d)` onde `kneeH = kneeW = altura` (região do knee é um quadrado altura x altura colado à esquerda). `m_dbRange` inicial = 36 dB; roda do mouse altera em passos de 3 dB, limitado a 3..96.
  - Camadas (cada uma é um QPixmap 1920x1080 transparente, pintado nesta ordem em `paintEvent`):
    1. Fundo: `fillRect(0,0,W,H, #070809)` com CompositionMode_Source.
    2. `m_graphPixmap` (grade, ver Desenho próprio).
    3. `m_visPixmap` (histórico rolante: volume de entrada, saída e redução de ganho).
    4. `m_kneePixmap` desenhado com `setOpacity(0.25)` (curva de transferência inteira, apagada).
    5. `m_kneePixmap2` em opacidade 1, só se `isProcessingAudio()`: é a mesma curva de knee, mas revelada só da esquerda até o X do nível de entrada atual (efeito de "preenchimento" da curva acompanhando o sinal).
    6. `m_miscPixmap` (linhas de threshold pontilhadas).
    7. Por cima: widgets filhos (QLabel controlsBox e controles).
  - Duplo clique (`mouseDoubleClickEvent`) alterna a visibilidade de TODOS os controles e do controlsBox (modo "só gráfico"); ao reexibir, respeita as regras de visibilidade condicionais (RMS, blend, lookahead, ratio).
- **Desenho próprio:**
  - `drawGraph()` (em `m_graphPixmap`, refeito em resize e wheel): antialias desligado; limpa com transparente. Fonte `QFont("Arial", max(int(H / 1080 * 24), 12))` (pontos; em 560 px de altura = 12). Pen `m_graphColor` rgba(209,216,228,50) largura 1 (a pen `m_textColor` rgba(209,216,228,50) é ajustada antes mas sobrescrita; mesmas cores). Para `i = 0 .. dbRange/3` (13 iterações com dbRange 36):
    - linha horizontal em `y = dbfsToYPoint(-3*i)` de x=0 a x=W;
    - linha vertical em `x = dbfsToXPoint(-3*i)` de y=0 a y=kneeH (só na região do knee);
    - texto `QString::number(i * -3)` ("0", "-3", ..., "-36") em `QRectF(W-50, y, 50, 50)`, `AlignRight | AlignTop`.
    - Valores no padrão 800x560, dbRange 36 (int truncado): y = 0, 46, 93, 140, 186, 233, 280, 326, 373, 420, 466, 513, 560; x = 560, 514, 467, 420, 374, 327, 280, 234, 187, 140, 94, 47, 0. Textos em x=750..800.
  - `resetCompressorView()` (no resize): limpa `m_visPixmap` e desenha uma linha vertical em x=W de y=0 a H com pen `m_resetColor` rgba(200,100,15,200) largura 3 (só ~1,5 px fica visível na borda direita), "para o conteúdo repentino não ficar feio".
  - `drawVisPixmap()` a cada tick: copia o próprio pixmap deslocado `-n` px (CompositionMode_Source) e limpa a faixa nova `[W-n, W]`. Antialias ligado. Com `n` = pixels deste tick, interpolação linear entre o ponto anterior e o atual (`std::lerp`):
    - Área de entrada: para cada coluna `i` em 0..n-1, linha vertical em `x = W-n+i` de `y = lerp(lastY, Y)` até H, pen `m_inVolAreaColor` rgba(209,216,228,17) largura 1.
    - Linha de entrada: `(W-n-1, lastY) -> (W, Y)`, pen `m_inVolColor` rgba(209,216,228,100) largura 1.
    - Área de saída: idem com `y = lerp(lastY + lastGainY, Y + gainY)`, pen `m_outVolAreaColor` rgba(209,216,228,30) largura 1. (saída = entrada em dB + redução, somado em pixels)
    - Linha de saída: `(W-n-1, lastY+lastGainY) -> (W, Y+gainY)`, pen `m_outVolColor` rgba(209,216,228,240) largura 1.
    - Linha de redução de ganho: `(W-n-1, lastGainY) -> (W, gainY)`, pen `m_gainReductionColor` rgba(180,100,100,210) largura **2** (desce a partir do topo; 0 dB de redução = y 0).
  - `redrawKnee()` (quando `m_redrawKnee` é sinalizado pelo DSP ou por resize/wheel) em `m_kneePixmap`: limpa; antialias ligado; pen `m_kneeColor` rgba(39,171,95,255) (#27ab5f) largura 3. `ratio` efetivo = 0 se Limiter ligado, senão `m_ratioVal` (inverso, 1/ratio no DSP). Pontos: `k1 = thr - knee`, `k2X = thr + knee`, `k2Y = thr + knee*ratio`, `ratioPoint = thr + (-thr*ratio)`.
    - Reta 1: `(0, kneeH)` -> `(X(k1), Y(k1))`.
    - Reta 2 (se `X(k2X) < kneeH`): `(X(k2X), Y(k2Y))` -> `(kneeH, Y(ratioPoint))`.
    - Se knee != 0: pen `m_kneeColor2` rgba(9,171,160,255) (#09aba0) largura 3; 20 segmentos (`COMP_KNEE_LINES`): `x_n = lerp(k1, k2X, (i+1)/20)`, `y_n = x_n + (ratio-1)*(x_n - thr + knee)^2 / (4*knee)`, de (x_{n-1}, y_{n-1}) partindo de (k1, k1).
    - Apaga tudo à direita de `x = kneeW + 1`. Também limpa `m_kneePixmap2` e zera `m_lastKneePoint`.
  - `drawKneePixmap2()` a cada tick: `kneePoint = X(ampToDbfs(peakAvg))`; se cresceu, copia de `m_kneePixmap` o retângulo `(lastKnee, 0, kneePoint-lastKnee, kneeH)`; se diminuiu, apaga de `kneePoint` em diante.
  - `drawMiscPixmap()` (quando `m_redrawThreshold`): limpa; antialias; pen `m_threshColor` rgba(39,171,95,100) largura 2 `Qt::DotLine`; linha horizontal `y = Y(threshold)` de 0 a W; linha vertical `x = kneeH - Y(threshold)` de 0 a kneeH.
  - Todas as cores acima também são `Q_PROPERTY` e o tema (`style.css:1165-1178`) define exatamente os mesmos valores.
- **Notas:**
  - O painel `controlsBox.png` fica POR CIMA do gráfico (é um QLabel filho), a ~40 px da borda inferior e centralizado horizontalmente; o gráfico aparece em volta e através das áreas transparentes da imagem.
  - Os anéis `knob_enabled*` são criados antes dos knobs, então ficam atrás deles na mesma posição; aparecem/somem junto com o knob condicional.
  - Visibilidade condicional: Ratio + anel some com Limiter; RMS + anel só com peakmode=RMS(0); Blend + anel só com stereoLink=4; Lookahead Length + anel só com Lookahead ligado; botão Feedback some com Lookahead ligado.
  - Os botões Audition e Feedback recebem o nome `tr("Soft Clip")` (copy-paste no original; afeta só o nome do controle, não a imagem).
  - Fader: `EqFader` é `Fader` com tamanho fixo 23x116 (min=max=23x116) e VUs estéreo alimentados por `AtomicPeak`, queda de 1.07x por tick a 60 Hz (ver Eq).

### CrossoverEQ

- **Classificação:** visual próprio (artwork.svg com painel), controles em layout; **Categoria:** efeito
- **Arquivos:** `plugins/CrossoverEQ/CrossoverEQControlDialog.cpp/.h`, `CrossoverEQControls.cpp/.h`. Imagem: `plugins/CrossoverEQ/artwork.svg` (width="167" height="218").
- **Janela:** `setFixedSize(167, 218)`. Artwork: gradiente base; painel único em x 5.5, y 54.5, 156x158 (gradiente `#1a1d21`->`#2d333a`, contorno preto, rx/ry 2.065), brilho branco .1 em y 55..56, sombra preta .3 em y 211..212. O painel envolve os faders e botões de mute; a faixa de cima (y 0..54) é dos knobs de crossover.
- **Controles:** `QVBoxLayout` (margens padrão) com: (a) `QHBoxLayout` de knobs, cada um `AlignHCenter`; (b) `QGridLayout` com margens 4,10,4,5, linha 0 = faders, linha 1 = botões mute.

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `Knob` Bright26, rótulo "1/2" (sem tr) 10 px | `m_xover12` "Band 1/2 Crossover", 125, 50..10000 | linha de knobs, 1o | "Band 1/2 crossover" "Hz" |
| `Knob` Bright26, "2/3" 10 px | `m_xover23` "Band 2/3 Crossover", 1250, 50..20000 | 2o | "Band 2/3 crossover" "Hz" |
| `Knob` Bright26, "3/4" 10 px | `m_xover34` "Band 3/4 Crossover", 5000, 50..20000 | 3o | "Band 3/4 crossover" "Hz" |
| `Fader` (sem pixmap de knob próprio, `modelIsLinear=false`, `setDisplayConversion(false)`, `setRenderUnityLine(false)`) | `m_gain1` "Band 1 Gain", 0, -60..30 dB | grade (0,0) AlignHCenter | "Band 1 gain" "dBFS" |
| `Fader` idem | `m_gain2` "Band 2 Gain" | (0,1) | "Band 2 gain" "dBFS" |
| `Fader` idem | `m_gain3` "Band 3 Gain" | (0,2) | "Band 3 gain" "dBFS" |
| `Fader` idem | `m_gain4` "Band 4 Gain" | (0,3) | "Band 4 gain" "dBFS" |
| `AutomatableButton` checável, objectName `btn-mute-inv` | `m_mute1` "Mute Band 1", default **true** | (1,0) AlignCenter | tooltip "Mute band 1" |
| idem | `m_mute2` "Mute Band 2", true | (1,1) | "Mute band 2" |
| idem | `m_mute3` "Mute Band 3", true | (1,2) | "Mute band 3" |
| idem | `m_mute4` "Mute Band 4", true | (1,3) | "Mute band 4" |

- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** os nomes de modelo aqui não usam `tr()`. O botão `btn-mute-inv` tem lógica invertida: o modelo true significa "banda ativa" (não mutada). Estilo (`style.css`): 20x20 (padding 2, ícone 14, borda 1). Não marcado (= mutado): ícone `resources:speaker_slash.svg`, borda `#890120`, fundo gradiente vermelho `#e2515b` / `#d40237` / `#d40237` / `#900122` (paradas 0, .1, .9, 1). Marcado (= tocando): ícone `resources:speaker.svg`, borda `#080b12` (topo `#0e1218`, base `#03060b`), raio 2, fundo gradiente translúcido (branco 20% -> 2%, preto 2% -> 40%). Labels dos knobs sem `tr()`.

### Delay

- **Classificação:** visual próprio (artwork + widget próprio `XyPad` com paintEvent) ; **Categoria:** efeito
- **Arquivos:** `plugins/Delay/DelayControlsDialog.cpp/.h` (inclui classe `XyPad`), `DelayControls.cpp`, `plugins/Eq/EqFader.h`.
  - Imagens: `artwork.png` 300x208 (referenciada). Sem SVG.
- **Janela:** `setFixedSize(300, 208)`, fundo `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| TempoSyncKnob Bright26, rótulo tr("DELAY"), `LabelRendering::LegacyFixedFontSize` | `m_delayTimeModel` tr("Delay samples") (0.5, 0.01..5 s) | (10,14) | "Delay time:" " s" |
| VolumeKnob Bright26, rótulo tr("FDBK"), LegacyFixedFontSize, zeroDbfs 1 | `m_feedbackModel` tr("Feedback") (0, 0..1) | (11,58) | "Feedback amount:" "" |
| TempoSyncKnob Bright26, rótulo tr("RATE"), Legacy | `m_lfoTimeModel` tr("LFO frequency") (2, 0.01..5) | (11,119) | "LFO frequency:" " s" |
| TempoSyncKnob Bright26, rótulo tr("AMNT"), Legacy | `m_lfoAmountModel` tr("LFO amount") (0, 0..0.5) | (11,159) | "LFO amount:" " s" |
| EqFader 23x116 (fixo; `setMaximumHeight(196)` não tem efeito), VU `m_outPeakL/R` | `m_outGainModel` tr("Output gain") (0, -60..20); nome tr("Out gain") | (263,45) | "Gain:" " dBFS"; displayConversion false |
| XyPad 200x200 | X = `m_feedbackModel`, Y = `m_delayTimeModel` | (50,5) | — |

- **Visores vivos:** XyPad repinta quando qualquer dos dois modelos muda (`dataChanged`). EqFader VU a 60 Hz.
- **Desenho próprio (`XyPad::paintEvent`):** sem fundo (transparente, mostra o artwork). Pen `QColor(200,200,200,200)` largura 8, `RoundCap`, `BevelJoin`, antialias; desenha um único ponto (bolinha de 8 px) em `x = (fb - min)/(range/200)`, `y = (delay - min)/(range/200)` (delay mínimo no topo, cresce para baixo). Mouse: arrastar com botão pressionado dentro de 0..199 seta X e Y linearmente.
- **Notas:** o XyPad (50..250 x 5..205) é criado depois do fader, mas não se sobrepõe a ele (fader em x 263).

### DualFilter

- **Classificação:** visual próprio (artwork.svg com dois painéis, posições absolutas); **Categoria:** efeito
- **Arquivos:** `plugins/DualFilter/DualFilterControlDialog.cpp/.h`, `DualFilterControls.cpp/.h`. Imagem: `plugins/DualFilter/artwork.svg` (width="373" height="109"). Os itens dos ComboBox usam pixmaps do tema: `filter_lp`, `filter_hp`, `filter_bp`, `filter_notch`, `filter_ap`, `filter_2lp` (via `PixmapLoader`, em `data/themes/default/`).
- **Janela:** `setFixedSize(373, 109)`. Artwork: gradiente base; painel filtro 1 em x 9.5, y 9.5, 156x90; painel filtro 2 em x 207.5, y 9.5, 156x90 (gradiente `#1a1d21`->`#2d333a`, contorno preto, raio 2.065); brilho branco .1 em y 10..11 e sombra preta .3 em y 98..99 de cada painel; dois tracinhos de conexão horizontais em y 50.5 (x 166..173 e x 200..207, stroke `#191a20` 3 px) ligando os painéis ao knob MIX central.
- **Controles:** todos com rótulo 10 px.

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `LedCheckBox` sem texto, Green | `m_enabled1Model` tr("Filter 1 enabled"), true | (12, 11) | tooltip "Enable/disable filter 1" |
| `Knob` Bright26 tr("FREQ") | `m_cut1Model` tr("Cutoff frequency 1"), 7000, 1..20000 | (24, 26) | "Cutoff frequency" "Hz" |
| `Knob` Bright26 tr("RESO") | `m_res1Model` tr("Q/Resonance 1"), 0.5, minQ..10 | (74, 26) | "Resonance" "" |
| `VolumeKnob` Bright26 tr("GAIN") | `m_gain1Model` tr("Gain 1"), 100, 0..200 | (124, 26) | "Gain" "%" |
| `ComboBox` | `m_filter1Model` tr("Filter 1 type") | geometry (19, 70, 137, 22) | — |
| `Knob` Bright26 tr("MIX") | `m_mixModel` tr("Mix"), 0, -1..1 | (173, 37) | "Mix" "" |
| `LedCheckBox` sem texto, Green | `m_enabled2Model` tr("Filter 2 enabled"), true | (210, 11) | tooltip "Enable/disable filter 2" |
| `Knob` Bright26 tr("FREQ") | `m_cut2Model` tr("Cutoff frequency 2"), 7000, 1..20000 | (222, 26) | "Cutoff frequency" "Hz" |
| `Knob` Bright26 tr("RESO") | `m_res2Model` tr("Q/Resonance 2") | (272, 26) | "Resonance" "" |
| `VolumeKnob` Bright26 tr("GAIN") | `m_gain2Model` tr("Gain 2"), 100, 0..200 | (322, 26) | "Gain" "%" |
| `ComboBox` | `m_filter2Model` tr("Filter 2 type") | geometry (217, 70, 137, 22) | — |

  Itens de tipo de filtro (iguais nos dois, ícone entre parênteses): Low-pass (lp), Hi-pass (hp), Band-pass csg (bp), Band-pass czpg (bp), Notch (notch), All-pass (ap), Moog (lp), 2x Low-pass (2lp), RC Low-pass 12 dB/oct (lp), RC Band-pass 12 dB/oct (bp), RC High-pass 12 dB/oct (hp), RC Low-pass 24 dB/oct (lp), RC Band-pass 24 dB/oct (bp), RC High-pass 24 dB/oct (hp), Vocal Formant (hp), 2x Moog (2lp), SV Low-pass (lp), SV Band-pass (bp), SV High-pass (hp), SV Notch (notch), Fast Formant (hp), Tripole (lp).
- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** espaçamento dos knobs de 50 px; MIX deslocado 11 px para baixo (y 37) para ficar entre os painéis, na altura dos tracinhos.

### DynamicsProcessor

- **Classificação:** visual próprio (artwork + Graph desenhável com fundo próprio + PixmapButtons) ; **Categoria:** efeito
- **Arquivos:** `plugins/DynamicsProcessor/DynamicsProcessorControlDialog.cpp/.h` (classe `DynProcControlDialog`), `DynamicsProcessorControls.cpp`.
  - Imagens (todas referenciadas): `artwork.png` 224x319; `wavegraph.png` 204x205 (fundo do Graph); `reset_active/inactive.png` 46x13; `smooth_active/inactive.png` 46x13; `add1_active/inactive.png` 29x13; `sub1_active/inactive.png` 29x13; `max_active/inactive.png` 78x17; `avg_active/inactive.png` 78x16; `unl_active/inactive.png` 78x17. Sem SVG.
- **Janela:** `setFixedSize(224, 319)`, fundo `artwork.png` (palette).
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Graph `Style::LinearNonCyclic` 204x205, fundo `wavegraph.png` (palette + autofill), `setGraphColor(QColor(85, 204, 145))`, maxSize 204x205 | `m_wavegraphModel` (0..1, 200 amostras) | (10,6) | — |
| VolumeKnob Bright26, rótulo tr("INPUT") fonte SMALL_FONT_SIZE, `setZeroDbfsPoint(1.f)` | `m_inputModel` tr("Input gain") (1, 0..5) | (26,223) | "Input gain:" "" |
| VolumeKnob Bright26, rótulo tr("OUTPUT") | `m_outputModel` tr("Output gain") (1, 0..5) | (76,223) | "Output gain:" "" |
| Knob Bright26, rótulo tr("ATTACK") | `m_attackModel` tr("Attack time") (10, 1..500) | (24,268) | "Peak attack time:" "ms" |
| Knob Bright26, rótulo tr("RELEASE") | `m_releaseModel` tr("Release time") (100, 1..500) | (74,268) | "Peak release time:" "ms" |
| PixmapButton `reset_active/inactive` 46x13 (ação, não checkable) | slot `resetClicked()` | (162,223) | "Reset wavegraph" |
| PixmapButton `smooth_active/inactive` 46x13 | `smoothClicked()` | (162,239) | "Smooth wavegraph" |
| PixmapButton `add1_active/inactive` 29x13 | `addOneClicked()` | (131,223) | "Increase wavegraph amplitude by 1 dB" |
| PixmapButton `sub1_active/inactive` 29x13 | `subOneClicked()` | (131,239) | "Decrease wavegraph amplitude by 1 dB" |
| PixmapButton `max_active/inactive` 78x17 (grupo) | `m_stereomodeModel` tr("Stereo mode") valor 0 | (131,257) | "Process based on the maximum of both stereo channels" |
| PixmapButton `avg_active/inactive` 78x16 | valor 1 | (131,274) | "Process based on the average of both stereo channels" |
| PixmapButton `unl_active/inactive` 78x17 | valor 2 | (131,290) | "Process each stereo channel independently" |

- **Visores vivos:** nenhum além do Graph editável (curva de transferência entrada->saída; não mostra sinal).
- **Desenho próprio:** nenhum paintEvent próprio; o Graph é widget do tema.
- **Notas:** os `resize(13,48)`, `resize(13,29)`, `resize(78,17)` etc. do código são anulados por `setActiveGraphic` (tamanho final = imagem). Os rótulos dos knobs (INPUT/OUTPUT/ATTACK/RELEASE) são desenhados pelo próprio Knob (fonte 10 px).

### Eq

- **Classificação:** visual próprio (artwork de fundo 500x500, curva desenhada em QGraphicsScene com alças arrastáveis, dois analisadores de espectro FFT) ; **Categoria:** efeito
- **Arquivos:**
  - `plugins/Eq/EqControlsDialog.cpp/.h`, `EqParameterWidget.cpp/.h` (cena + alças), `EqCurve.cpp/.h` (classes `EqHandle` e `EqCurve`), `EqSpectrumView.cpp/.h` (`EqAnalyser` + `EqSpectrumView`), `EqFader.h`, `EqControls.cpp` (modelos).
  - Imagens:
    - `artwork.png` 500x500 (fundo)
    - `bandLabel1.png` .. `bandLabel8.png` e `bandLabel1off.png` .. `bandLabel8off.png`, todos 29x14 (botões de ativar banda; nome montado em código: `"bandLabel" + to_string(i+1)` e `+ "off"`)
    - `handle1.png` .. `handle8.png` e `handle1inactive.png` .. `handle8inactive.png`, todos 15x15 (alças; nome montado: `"handle" + to_string(n+1)` + `"inactive"` se a banda está desligada)
    - `handlehover.png` 15x15 (sobreposto à alça sob o mouse)
    - `12dB.png`/`12dBoff.png`, `24dB.png`/`24dBoff.png`, `48dB.png`/`48dBoff.png`, todos 29x14 (tipo de corte HP/LP)
  - Sem SVG. Nenhuma imagem órfã (bandLabel* e handle* parecem órfãs numa busca por string literal, mas são carregadas por nome concatenado).
- **Janela:** `setFixedSize(500, 500)`, fundo `artwork.png` via `setPalette` + `setAutoFillBackground(true)`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| EqSpectrumView (entrada) 450x200 | `m_inFftBands` (EqAnalyser) | (26,17) | — |
| EqSpectrumView (saída) 450x200 | `m_outFftBands` | (26,17) | — |
| EqParameterWidget 450x200 (QGraphicsView + cena 450x200 com 8 EqHandle + EqCurve) | todas as bandas | (26,17) | — |
| EqFader 23x80 (fixo) | `m_inGainModel` tr("Input gain") (0, -60..20); nome tr("Input gain"); VU `m_inPeakL/R` | (23,295) | hint "Gain" "dBv"; displayConversion false |
| EqFader 23x80 | `m_outGainModel` tr("Output gain") (0, -60..20); VU `m_outPeakL/R` | (453,295) | "Gain" "dBv" |
| EqFader 23x80, loop i=1..6 (`x = 126 + 44*(i-1)`), nome tr("") | ganho da banda i: i=1 `m_lowShelfGainModel` tr("Low-shelf gain"); 2 `m_para1GainModel` tr("Peak 1 gain"); 3 `m_para2GainModel` tr("Peak 2 gain"); 4 `m_para3GainModel` tr("Peak 3 gain"); 5 `m_para4GainModel` tr("Peak 4 gain"); 6 `m_highShelfGainModel` tr("High-shelf gain"). Todos 0, -18..18. VU = peakL/R da banda | x = 126, 170, 214, 258, 302, 346; y = 295 | "Gain" "dB" |
| PixmapButton `bandLabel{i+1}` / `bandLabel{i+1}off` 29x14, checkable, loop i=0..7 | active da banda: 0 `m_hpActiveModel` tr("HP active"); 1 `m_lowShelfActiveModel` tr("Low-shelf active"); 2..5 `m_para1..4ActiveModel` tr("Peak N active"); 6 `m_highShelfActiveModel` tr("High-shelf active"); 7 `m_lpActiveModel` tr("LP active") | x = 79, 123, 167, 211, 255, 299, 343, 387 (`81+44i-2`); y = 276 | sem tooltip |
| Knob Bright26, freq, loop i=0..7 | 0 `m_hpFeqModel` tr("HP freq") 31; 1 `m_lowShelfFreqModel` tr("Low-shelf freq") 80; 2 `m_para1FreqModel` tr("Peak 1 freq") 120; 3 `m_para2FreqModel` 250; 4 `m_para3FreqModel` 2000; 5 `m_para4FreqModel` 4000; 6 `m_highShelfFreqModel` tr("High-shelf freq") 12000; 7 `m_lpFreqModel` tr("LP freq") 18000. Faixa 20..20000 | x = 81, 125, 169, 213, 257, 301, 345, 389 (`81+44i`); y = 396 | "Frequency:" "Hz" |
| Knob Bright26, res/BW, loop i=0..7 | 0 `m_hpResModel` tr("HP res") 0.707 (0.003..10); 1 `m_lowShelfResModel` tr("Low-shelf res") 0.707 (0.55..10); 2..5 `m_para1..4BwModel` tr("Peak N BW") 0.3 (0.1..4); 6 `m_highShelfResModel` tr("High-shelf res") 0.707 (0.55..10); 7 `m_lpResModel` tr("LP res") 0.707 (0.003..10) | x = 81 .. 389 (mesmos), y = 440 | i=2..5: "Bandwidth: " " Octave"; demais: "Resonance: " "" |
| LedCheckBox (sem texto, cor padrão Yellow) | `m_analyseInModel` tr("Analyse IN") (true) | (172,240) | — |
| LedCheckBox (sem texto, Yellow) | `m_analyseOutModel` tr("Analyse OUT") (true) | (302,240) | — |
| PixmapButton `12dB`/`12dBoff` 29x14 (grupo HP) | `m_hp12Model` tr("HP 12"); grupo `m_hpTypeModel` tr("High-pass type") valor 0 | (79,298) | — |
| PixmapButton `24dB`/`24dBoff` (HP) | `m_hp24Model` tr("HP 24"); grupo valor 1 | (79,328) | — |
| PixmapButton `48dB`/`48dBoff` (HP) | `m_hp48Model` tr("HP 48"); grupo valor 2 | (79,358) | — |
| PixmapButton `12dB`/`12dBoff` (grupo LP) | `m_lp12Model` tr("LP 12"); grupo `m_lpTypeModel` tr("Low-pass type") valor 0 | (387,298) | — |
| PixmapButton `24dB`/`24dBoff` (LP) | `m_lp24Model` tr("LP 24"); valor 1 | (387,328) | — |
| PixmapButton `48dB`/`48dBoff` (LP) | `m_lp48Model` tr("LP 48"); valor 2 | (387,358) | — |

  Grupos: `AutomatableButtonGroup(this, tr("HP group"))` e `tr("LP group")`. Cada botão de corte também tem setModel no BoolModel individual (hp12/24/48), mas o grupo depois reatribui o modelo do grupo.

- **Visores vivos:**
  - **EqSpectrumView** (2 instâncias sobrepostas em (26,17), 450x200, `WA_TranslucentBackground`). Atualização: `periodicUpdate` (60 Hz) chama `periodicalUpdate()` -> `m_analyser->setActive(isVisible())` + `update()`. O analisador só calcula quando ativo; após cada FFT desativa-se até o próximo tick.
    - Cores: entrada `QColor(77, 101, 242, 150)` (azul translúcido); saída `QColor(0, 255, 239, 150)` (ciano translúcido). Ordem de criação: entrada, saída, depois o parameter widget por cima.
    - Dados (`EqAnalyser::analyze`): mono `(L+R)*0.5`, buffer de 2048 amostras (`FFT_BUFFER_SIZE`), janela Blackman-Harris (a0 0.35875, a1 0.48829, a2 0.14128, a3 0.01168), FFTW r2c de tamanho 4096 (buffer zero-padded), `absspec` + `compressbands` em `MAX_BANDS = 2048` bandas de 0 a SR/2; `energy = max(bands) / max(buffer)`.
    - Desenho: se energia <= 0 e soma de picos <= 0, não desenha nada. Para cada banda x: `peak = fh*2/3 * (20*log10(band/energy) - (-36)) / 36` (piso -36 dB, 0 dB relativo = 2/3 da altura); picos >= altura são pulados; ataque instantâneo, queda `altura /= 1.07` por tick. Caminho: começa em (0, H), `lineTo(freqToXPixel(bandToFreq(x), 450), H - altura)` para cada banda, termina em (W, H), fecha. `bandToFreq(i) = i * SR / 4096`. Preenchido com a cor (`fillPath`) e contornado com pen da mesma cor largura 1, `RoundCap`, `BevelJoin`, antialias. Entre ticks pinta só o caminho em cache (só fill).
  - **EqParameterWidget** (450x200): `QGraphicsScene` com `sceneRect(0,0,450,200)`, `QGraphicsView` com stylesheet `"border-style: none; background: transparent;"` e sem scrollbars (o view não recebe resize; fica com o sizeHint, ~450x200). Escalas:
    - `freqToXPixel(f, w) = (log10(f) - log10(20)) / (log10(20000) - log10(20)) * w` (20 Hz..20 kHz em 450 px, log).
    - `gainToYPixel(g) = h*0.5 - g * (200/36)` (±18 dB em 200 px; 5,556 px/dB; 0 dB em y=100).
    - Banda 0 (HP) e 7 (LP) ficam sempre em y = 100 (só movem em X). Demais bandas em (freq, gain).
  - **EqHandle** (8 itens, z=1, movíveis, hover): pixmap `handle{n+1}` ou `handle{n+1}inactive` (15x15), desenhado em `(-8, -7)` relativo ao centro (`-(15/2)-1`, `-(15/2)`). `boundingRect` = (-7,-7,15,15). Restrito ao retângulo 0..450 x 0..200. Roda do mouse muda resonance/BW: passo 0.15 (0.01 com Ctrl), limite 0.1..10 (0.1..4 para Para).
    - No hover: desenha `handlehover.png` em (-8,-7) por cima, e uma caixa de info: `QRectF(rectX, rectY, 80, 30)` com rectX = rectY = -40, ajustados para não sair da cena (se y<40: rectY += 40 - y; se x<40: rectX += 40 - x; se x > 410: rectX -= 40 - (450 - x)). Fonte: fonte do painter com pixel size `SMALL_FONT_SIZE` (10). `drawRect` com pen preta, `fillRect` `QColor(6, 106, 43, 180)`. Texto centralizado em 2 linhas: `"Freq: " + freq + "\n" + ("Reso: " ou "BW: " para Para) + valor`; primeiro em preto deslocado +1,+1 (sombra), depois em branco (255,255,255).
  - **EqCurve** (item 0,0,450,200): curva composta. Quando o modelo muda: para cada banda ativa calcula a curva biquad (RBJ cookbook) em cada x de 0..449 e faz uma "média" peculiar: `main[x] += curve_y*nAtivas - (nAtivas*(h/2) - h)` por banda, depois `main[x] = main[x]/nAtivas - h/2`. HP/LP 24 dB multiplicam o ganho em dB por 2, 48 dB por 3. Com 0 bandas ativas, nada é desenhado. A curva é cacheada num pixmap 450x200 com pen branca (`Qt::white`) largura 2, antialias.
    - Hover sobre qualquer alça: `m_alpha` sobe +10 por repaint até 40; sem hover desce -10 até 0. Desenha a curva individual da banda sob o mouse com pen `QColor(qRgba(6,106,43,242))` largura 2 (nota: o construtor `QColor(QRgb)` ignora o alfa, cor efetivamente opaca #066a2b). Depois preenche a área abaixo da curva composta até y=200 com `QColor(255,255,255,m_alpha)`.
  - **EqFader** (VU): `periodicUpdate` 60 Hz; pega o pico se maior (`takeAbove`), senão cai `/1.07` por tick.
- **Desenho próprio:** descrito acima (EqHandle::paint, EqCurve::paint, EqSpectrumView::paintEvent). Nenhum paintEvent no diálogo em si.
- **Notas:**
  - Duplo clique no diálogo alterna a altura da subjanela-pai entre 283 px e a altura original (modo compacto mostrando só a parte de cima).
  - Mexer em knob/fader de uma banda (sem alça em hover) liga a banda automaticamente (`updateHandle`). Arrastar a alça também liga a banda.
  - As 3 camadas (2 espectros + parameter widget) estão exatamente no mesmo retângulo (26,17,450,200).
  - O analisador de espectro só roda se o widget está visível (economia de CPU).

### Flanger

- **Classificação:** visual próprio (fundo artwork.svg liso), controles em layout; **Categoria:** efeito
- **Arquivos:** `plugins/Flanger/FlangerControlsDialog.cpp/.h`, `FlangerControls.cpp/.h`. Imagem: `plugins/Flanger/artwork.svg` (width="195" height="75"), só gradiente base.
- **Janela:** por layout (`QVBoxLayout` `SetFixedSize`): linha de 6 knobs (`QHBoxLayout`) + `LedCheckBox` alinhado à esquerda embaixo. Artwork pretende 195x75.
- **Controles:** (ordem da esquerda para a direita)

| Widget | Modelo | Dica |
|---|---|---|
| `Knob` Bright26 tr("DELAY") | `m_delayTimeModel` tr("Delay samples"), 0.001, 0.0001..0.050 | "Delay time: " (com espaço extra) "s" |
| `TempoSyncKnob` Bright26 tr("RATE") | `m_lfoFrequencyModel` tr("LFO frequency"), 0.25, 0.01..60 (escala 60000) | "Period:" " Sec" |
| `Knob` Bright26 tr("AMNT") | `m_lfoAmountModel` tr("Amount"), 0, 0..0.0025 | "Amount:" "" |
| `Knob` Bright26 tr("PHASE") | `m_lfoPhaseModel` tr("Stereo phase"), 90, 0..360 | "Phase:" " degrees" |
| `VolumeKnob` Bright26 tr("FDBK"), `setZeroDbfsPoint(1.f)` | `m_feedbackModel` tr("Feedback"), 0, -1..1 | "Feedback amount:" "" |
| `VolumeKnob` Bright26 tr("NOISE"), `setZeroDbfsPoint(1.f)` | `m_whiteNoiseAmountModel` tr("Noise"), 0, 0..0.05 | "White noise amount:" "" |
| `LedCheckBox` tr("Invert"), Yellow | `m_invertFeedbackModel` tr("Invert"), false | (sem); linha de baixo, AlignLeft |

- **Visores vivos / Desenho próprio:** nenhum.

### FrequencyShifter

- **Classificação:** visual próprio (artwork + knobs Styled com tamanhos customizados + PixmapButtons + LCD grande) ; **Categoria:** efeito
- **Arquivos:** `plugins/FrequencyShifter/FrequencyShifterControlDialog.cpp/.h` (inclui `FrequencyShifterHelpView`), `FrequencyShifterControls.cpp`.
  - Imagens (todas referenciadas): `artwork.png` 288x360; `antireflect_on/off.png` 78x18; `send_on/off.png` 82x18; `pass_on/off.png` 82x18; `mute_on/off.png` 82x18; `reset_shifter_on/off.png` 59x12; `reset_lfo_on/off.png` 59x12; `help_on/off.png` 18x18. Sem SVG.
- **Janela:** `setFixedSize(288, 360)`, fundo `artwork.png`.
- **Controles:** todos os knobs são `Knob(KnobType::Styled)` com `objectName` (usado pelo CSS) e geometria de `setupKnobGeometry(k, w, h)`: `setFixedSize(w,h)`, centro (w/2, h/2), `outer = max(1, cx-3)`, `inner = max(1, outer - (w>=40 ? 16 : w>=24 ? 10 : 6))`. Resultados: **K60** 60x60 centro (30,30) outer 27 inner 11; **K36** 36x36 centro (18,18) outer 15 inner 5; **K24** 24x24 centro (12,12) outer 9 inner 1; **K19** 19x19 centro (9,9) outer 6 inner 1. CSS (`style.css:1195-1204`): `color: #ffffff; qproperty-outerColor: #ffffff; qproperty-lineWidth: 3`; para `#fs_glide`, `#fs_dglide`, `#fs_phase`: `lineWidth: 2`. Hints sem dois-pontos e sem tr().

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| LcdFloatSpinBox 6.3 dígitos, estilo "19red", seamless | `m_freqShift` "Frequency Shift" (0, -20000..20000) | (100,43) | — (nome tr("Frequency Shift")) |
| Knob Styled K60 `fs_mix` | `m_mix` "Mix" (1, 0..1) | (18,30) 60x60 | "Mix" "" |
| Knob Styled K24 `fs_spread` | `m_spreadShift` "Spread Shift" (0, -50..50) | (235,24) 24x24 | "Spread" "Hz" |
| Knob Styled K24 `fs_phase` (lineWidth 2) | `m_phase` "Phase" (0, -2..2) | (235,72) 24x24 | "Phase" "" |
| Knob Styled K36 `fs_ring` | `m_ring` "Ring" (0..1) | (24,115) 36x36 | "Ring" "" |
| Knob Styled K36 `fs_harm` | `m_harmonics` "Harmonics" (0..1) | (72,115) | "Harmonics" "" |
| Knob Styled K36 `fs_tone` | `m_tone` "Tone" (22000, 100..22000) | (120,115) | "Tone" "Hz" |
| Knob Styled K19 `fs_glide` (lineWidth 2) | `m_glide` "Glide" (0.05, 0..1) | (200,147) 19x19 | "Glide" "" |
| Knob Styled K36 `fs_lfo` | `m_lfoAmount` "LFO Amount" (0, 0..2000) | (18,200) | "LFO" "Hz" |
| Knob Styled K36 `fs_lforate` | `m_lfoRate` "LFO Rate" (0.2, 0..200) | (66,200) | "LFO Rate" "Hz" |
| Knob Styled K36 `fs_lfost` | `m_lfoStereoPhase` "LFO StereoPhase" (0..1) | (114,200) | "LFO Stereo Phase" "" |
| Knob Styled K36 `fs_delay` | `m_delayLengthLong` "Delay Length" (0, 0..2000) | (18,282) | "Delay Length" "ms" |
| Knob Styled K36 `fs_feedback` | `m_feedback` "Feedback" (0..1) | (114,282) | "Feedback" "" |
| Knob Styled K24 `fs_finedelay` | `m_delayLengthShort` "Fine Delay Length" (0, 0..20) | (24,324) | "Delay Length (fine)" "ms" |
| Knob Styled K24 `fs_damp` | `m_delayDamp` "Delay Damping" (22000, 100..22000) | (120,324) | "Delay Damping" "Hz" |
| Knob Styled K19 `fs_dglide` (lineWidth 2) | `m_delayGlide` "Delay Glide" (0.05, 0..1) | (245,315) | "Delay Glide" "" |
| PixmapButton `antireflect_on/off` 78x18, checkable | `m_antireflect` "Antireflect" | (188,122) | "Anti-reflect" |
| PixmapButton `send_on/off` 82x18 (grupo) | `m_routeMode` "Route Mode" valor 0 | (188,199) | "Route: Send" |
| PixmapButton `pass_on/off` 82x18 | valor 1 | (188,217) | "Route: Pass" |
| PixmapButton `mute_on/off` 82x18 | valor 2 | (188,235) | "Route: Mute" |
| PixmapButton `reset_shifter_on/off` 59x12, não checkable (momentâneo, automatizável) | `m_resetShifter` "Reset Shifter" | (77,5) | "Reset the shifter's oscillator phases to 0 (automatable)" |
| PixmapButton `reset_lfo_on/off` 59x12, não checkable | `m_resetLfo` "Reset LFO" | (60,179) | "Reset the LFO phase to 0 (automatable)" |
| PixmapButton `help_on/off` 18x18 | abre help | (256,278) | "Open help window" |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum paintEvent; o visual vem do artwork + knobs Styled brancos.
- **Notas:** help em `QTextEdit` (título tr("Frequency Shifter Help"), sem maximizar), créditos "Plugin by Lost Robot / GUI by Haeleon". Rótulos de modelos sem `tr()`. O LCD vermelho grande (6 inteiros + 3 decimais, "19red" 143x38 por dígito-strip) é o elemento central.

### GranularPitchShifter

- **Classificação:** visual próprio (artwork + controles posicionados + LCDs + PixmapButtons) ; **Categoria:** efeito
- **Arquivos:** `plugins/GranularPitchShifter/GranularPitchShifterControlDialog.cpp/.h` (inclui `GranularPitchShifterHelpView`), `GranularPitchShifterControls.cpp`.
  - Imagens (todas referenciadas): `artwork.png` 305x180; `help_active/inactive.png` 11x11; `prefilter_active/inactive.png` 50x18. Sem SVG.
  - O help view pede `PLUGIN_NAME::getIconPixmap("logo")`, mas **não existe `logo.png`** na pasta (ícone vazio).
- **Janela:** `setFixedSize(305, 180)`, fundo `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Knob Bright26 | `m_sizeModel` tr("Grain Size") (10, 2..1000) | (19,78) | "Grain Size:" " Hz" |
| Knob Bright26 | `m_sprayModel` tr("Spray") (0.005, 0..0.5) | (116,10) | "Spray:" " seconds" |
| Knob Bright26 | `m_jitterModel` tr("Jitter") (0..1) | (158,10) | "Jitter:" " octaves" |
| Knob Bright26 | `m_twitchModel` tr("Twitch") (0..1) | (200,10) | "Twitch:" " octaves" |
| Knob Bright26 | `m_spraySpreadModel` tr("Spray Stereo") (0..1) | (188,60) | "Spray Stereo Spread:" "" |
| Knob Bright26 | `m_shapeModel` tr("Shape") (2, 1..2) | (135,110) | "Grain Shape:" "" |
| Knob Bright26 | `m_fadeLengthModel` tr("Fade Length") (1, 0.001..1) | (188,110) | "Fade Length:" "" |
| Knob Bright26 | `m_feedbackModel` tr("Feedback") (0..1) | (258,45) | "Feedback:" "" |
| Knob Bright26 | `m_minLatencyModel` tr("Minimum Allowed Latency") (0.01, 0..1) | (258,92) | "Minimum Allowed Latency:" " seconds" |
| Knob Small17 | `m_densityModel` tr("Density") (1, 1..16) | (66,157) | "Density:" "x" |
| Knob Small17 | `m_glideModel` tr("Glide") (0.01, 0..1) | (8,157) | "Glide:" " seconds" |
| LcdFloatSpinBox 3.2 "11green", seamless | `m_pitchModel` tr("Pitch") (1, -48..24) | (15,41) | tooltip "Pitch" |
| LcdFloatSpinBox 3.2 "11green", seamless | `m_pitchSpreadModel` tr("Pitch Stereo Spread") (0, -24..24) | (133,66) | tooltip "Pitch Stereo Spread" |
| PixmapButton `help_active/inactive` 11x11 | abre help | (278,159) | "Open help window" |
| PixmapButton `prefilter_active/inactive` 50x18, checkable | `m_prefilterModel` tr("Prefilter") (true) | (8,133) | "Prefilter" |
| ComboBox | `m_rangeModel` tr("Ring Buffer Length"): "5 Seconds", "10 Seconds (Size)", "40 Seconds (Size and Pitch)", "40 Seconds (Size and Spray and Jitter)", "120 Seconds (All of the above)" | geometry (189,155,80,22) | — |

- **Visores vivos:** nenhum.
- **Desenho próprio:** nenhum.
- **Notas:** existe um `QPushButton button("Show Help", this)` criado na pilha dentro do construtor; é destruído ao fim do construtor e nunca aparece (código morto). Texto de help não passa por `tr()`; créditos "Plugin by Lost Robot / GUI by thismoon". `controls->updateRange()` é chamado ao final para ajustar os ranges conforme o ring buffer.

### Guitarix

- **Classificação:** visual próprio (cada unidade é desenhada como no rack do Guitarix: painel em bitmap com bevel e sombra, alças, parafusos, knobs com imagem + anel pintado, seletores, medidores de LED, sliders, chaves). **Categoria:** efeito (um plugin com 97 subplugins = unidades do Guitarix, `GuitarixSubPluginFeatures`).
- **Proveniência e licença das imagens:** `gx/rcstyles/*.png` e `gx/builder/*.glade` vêm do Guitarix upstream (estilo padrão "Guitarix": `rcstyles/gx_head_Guitarix.scss` sobre `minimal.scss`; desenho do libgxw). Licença: `gx/COPYING` (Guitarix como um todo sob **GPL-3.0 ou posterior**; código próprio do Guitarix GPL-2.0+). O CMakeLists diz: "Guitarix's own files (gx/), as they are in Guitarix ... (GPL-3.0 or later: see gx/COPYING and gx/zita-resampler/README)" e "Its look: Guitarix's default style's images (gx/rcstyles) and its font (gx/fonts: Roboto Condensed, Apache 2.0)". A fonte `gx/fonts/RobotoCondensed-Regular.ttf` tem `gx/fonts/LICENSE.txt` (Apache 2.0). Não há README/NOTES de proveniência por imagem dentro de `plugins/Guitarix/` (só `gx/COPYING`, `gx/fonts/LICENSE.txt`, `gx/zita-resampler/README`, `gx/zita-convolver/README`); as imagens entraram no commit `70dfb3d5b` (18/11, "Guitarix: its look"). O uso está coberto pela licença GPL registrada em `gx/COPYING`, não por permissão pessoal; a ressalva é que, como GPL-3+, qualquer reuso dessas imagens no programa novo leva a obrigação GPL junto.
- **Arquivos (view):** `plugins/Guitarix/GuitarixControlDialog.{h,cpp}` (diálogo), `GuitarixLayout.{h,cpp}` (leitor de glade → widgets Qt), `GuitarixWidgets.{h,cpp}` (widgets desenhados), `GuitarixUnits.{h,cpp}` (lista das 97 unidades), `GuitarixSelectors.cpp` (unidades que escolhem um módulo: crybaby, wah, tonestack, poweramp, ampstack), `GuitarixConvolution.cpp` (cab, pre, contraste), `GuitarixAmp.cpp` (cabeçote), `GuitarixDetune.cpp`, `GuitarixVibe.cpp`, `GuitarixEffect.{h,cpp}` (parâmetros). Recursos embutidos: `gx/rcstyles/*.png`, `gx/fonts/RobotoCondensed-Regular.ttf`, `gx/builder/*.glade` (lidos como `:/artwork/guitarix/<arquivo>`).
- **Imagens (`plugins/Guitarix/gx/rcstyles/`, carregadas por `guitarix::image(nome)` = `PLUGIN_NAME::getIconPixmap(nome)`). Não há SVG.**

  | Arquivo | Tamanho | Usado por | Referenciado? |
  |---|---|---|---|
  | `background1.png` | 800 x 200 | `paintSkin` (painel, ladrilhado), `PaintBox` `box_uni_1_expose` | sim |
  | `background2.png` | 800 x 200 | `paintSkin` (caixa alternativa), `PaintBox` `box_uni_2_expose` | sim |
  | `handle_left.png` | 32 x 32 | `RackUnit` (alça esquerda, ladrilhada na vertical) | sim |
  | `handle_right.png` | 32 x 32 | `RackUnit` (alça direita) | sim |
  | `screw.png` | 32 x 32 | `paintScrews` | sim |
  | `knob_small.png` | 38 x 38 | `Knob` `KnobKind::Small` (GxSmallKnob) | sim |
  | `knob_small_r.png` | 68 x 68 | `Knob` `SmallR` (GxSmallKnobR) | sim |
  | `knob_mid.png` | 90 x 90 | `Knob` `Mid` (GxMidKnob) | sim |
  | `knob_big.png` | 114 x 134 | `Knob` `Big` (GxBigKnob); não quadrado (sombra embaixo) | sim |
  | `selector_icon.png` | 15 x 15 | `Selector` | sim |
  | `hslider.png` | 206 x 24 | `HSlider`: trilho = primeiros 134 px; cursor = próximos 36 px; cursor em hover = últimos 36 px | sim (1 uso: `univibe.panning`) |
  | `vslider.png` | 24 x 206 | `VSlider`: trilho = 134 px do topo; cursor = 36 px seguintes; hover = 36 últimos | sim |
  | `portdisplay.png` | 81 x 5 | `PortDisplay` (janela de 40 px que desliza) | sim (1 uso: reversedelay) |
  | `switch_on.png` / `switch_off.png` | 16 x 16 | `Switch` base `switch` | sim (2 usos) |
  | `switch_mid_on.png` / `switch_mid_off.png` | 10 x 10 | `Switch` base `switch_mid` | sim (2 usos) |
  | `minitoggle_on.png` / `minitoggle_off.png` | 10 x 10 | `Switch` base `minitoggle` | sim (9 usos, cabeçote) |
  | `frbutton_on.png` / `frbutton_off.png` | 21 x 19 | `Switch` base `frbutton` (Freeze dos digital delays) | sim (2 usos: `dide.Freeze`, `didest.Freeze`) |

  - `minitoggle_*` e `switch_mid_*` são **byte a byte idênticos** (mesmo MD5), cada par.
  - Todas as 21 PNGs são referenciadas (direto no código ou por `base_name` dos glades).
  - O `mainpanel.glade` cita pixbufs `gx_plugins.png`, `gx_config.png`, `gx_tuner.png`, `gx_rack.png`, `gx_split.png`, `gx_fold.png`, `gx_show.png` e o ícone `rack_shrink` (GtkImage `image4`): **não existem** no repositório e não são carregados (ficam fora das raízes lidas, ou o leitor ignora `GtkImage`).
- **Fonte:** Roboto Condensed (`gx/fonts/RobotoCondensed-Regular.ttf`, Apache 2.0); se não carregar, fonte padrão com `QFont::Condensed`. Tamanho em pontos a 96 dpi: `pixelSize = lround(pt * 96 / 72)` → 7 pt = 9 px, 8 pt = 11 px, 10 pt = 13 px; seletor fixo 10 px.

#### Estrutura da janela (todas as unidades)

`GuitarixControlDialog` = `QVBoxLayout` margens 0 com um `RackUnit(effect->unitName(), buildControls(...))`, `SetFixedSize` (tamanho = o que o layout pede; não há tamanho fixo numérico).

`RackUnit` (gx_rack_unit_expose do GxPaintBox):
- Layout vertical: margens **esq 32, topo 0, dir 32, baixo 4**; espaçamento 4.
- Título: `QLabel(nome da unidade do Guitarix)`, fonte Roboto Condensed 10 pt (13 px), cor `#222222`, altura fixa **31**, margem interna esquerda 4. (O botão liga/desliga de 23 px do título no Guitarix não existe aqui: é o liga/desliga do efeito da LinuxMusic.)
- Abaixo: os controles da unidade, centralizados (`stretch 1, AlignCenter`).
- Pintura: `paintSkin(rect, alternateTop=0.6, alternateBottom=0)` + alças + parafusos (ver abaixo).

`paintSkin(area, top, bottom)`: h_painel = h - 4.
1. `background1` ladrilhado em (0,0,w,h_painel).
2. Se h > 64 e houver caixa alternativa: `background2` ladrilhado em (0, int(h_painel*top), w+1, int(h_painel*(1-bottom-top))+1), com origem do ladrilho em (0, top).
3. Bevel 0.11: gradiente vertical de branco alfa 0.055 (topo) a preto alfa 0.11 (base), aplicado em SoftLight e depois em SourceOver sobre (0,0,w,h_painel).
4. Sombra: gradiente vertical de preto alfa 0.8 a 0 nos 4 px finais (h_painel..h).

`paintScrews(area)`: screw 32x32, h_s = h - 3. Se h_s > 64: parafusos em (0,0), (0, h_s-32), (w-32, h_s-32), (w-32, 0). Senão, se h_s > 32: dois parafusos a meia altura, y = h_s/2 - 16, em x = 0 e x = w-32. Senão nenhum.

Alças (`RackUnit::paintEvent`): `handle_left` ladrilhado em (0,0,32,h_painel); linha 1 px preta alfa 0.8 em x=31 e linha 1 px branca alfa 0.15 em x=32. `handle_right` ladrilhado em (w-32,0,32,h_painel); linha preta alfa 0.5 em x=w-33 e branca alfa 0.05 em x=w-32. Depois os parafusos (por cima das alças).

`SkinBox(top, bottom, conteúdo)` (usado só no cabeçote `ampstack`): margens **32, 4, 32, 8**; pinta `paintSkin(rect, top, bottom)` + parafusos (sem alças). Parte 1 do cabeçote (`amp_background`): `SkinBox(0.33, 0.33)`; parte 2 (`ampdetail_normal`): `SkinBox(0, 0)`. As duas partes empilhadas em `QVBoxLayout` margens 0 espaçamento 4, dentro do `RackUnit`.

#### Regras do layout (GladeReader, `GuitarixLayout.cpp`)

Cada unidade descreve a UI num glade (GtkBuilder XML), inline no `.cc` (`glade_def`) ou num arquivo de `gx/builder/`. É lido o objeto `rackbox` (a unidade expandida); o cabeçote lê `ampbox.glade#amp_background` e `mainpanel.glade#ampdetail_normal`. As posições **não são absolutas**: saem das regras de caixa do GTK reproduzidas em Qt. Para reproduzir ao pixel, aplique estas regras à árvore de cada unidade (listadas mais abaixo):

- Objeto com `visible=False` não é criado.
- `GtkBox`/`GxPaintBox`: `QBoxLayout` horizontal (padrão) ou vertical (`orientation=vertical`), margens 0, `spacing` do glade. Empacotamento por filho: `expand` (padrão False; também verdadeiro se o filho tem `hexpand`/`vexpand` na direção da caixa), `fill` (padrão True), `padding` (espaço antes **e** depois do filho), `pack_type=end` (filhos "end" vão no fim, em ordem inversa; se nenhum filho expande, entra um stretch entre os "start" e os "end"). Filho que expande recebe stretch 1. Alinhamento transversal: `valign`/`halign` start/end/center; um controle (folha) fica centralizado; uma caixa que expande sem preencher fica centralizada também no eixo principal.
- `GtkGrid`: `QGridLayout` margens 0, `column_spacing`, `row_spacing`; filho em (`top_attach`, `left_attach`, `height`, `width`); folha centralizada; `halign` sobrescreve o horizontal. `column_homogeneous`/`row_homogeneous`: toda coluna/linha com mínimo = maior sizeHint de qualquer filho e stretch 1.
- `GtkLabel`: `QLabel` com o texto do glade (rich text se `use_markup`), alinhamento por `xalign` (<0.25 esquerda, >0.75 direita, senão centro). Estilo `.rack_label`: 8 pt (11 px) cor `#222222`. Estilo `.rack_label_inverse` ou qualquer rótulo dentro de um notebook: 7 pt (9 px) cor `#aaaaaa`. Rótulo apontado por um controle (`label_ref`) mostra o **nome do parâmetro**, não o texto do glade (por isso muitos rótulos "label" no glade).
- `GtkFrame`: rótulo em cima (alinhado por `label_xalign`, padrão 0 = esquerda), filho embaixo, margens internas **4, 0, 4, 4**. Classe `.frame_inversed`: borda 1 px `rgba(50,50,50,178)` começando na altura do centro do rótulo, com a linha de cima interrompida em volta do rótulo (3 px de folga de cada lado).
- `GtkAlignment`: margens = `left/top/right/bottom_padding`, primeiro filho com stretch 1 (folha centralizada).
- `GtkNotebook`: `QTabWidget` documentMode, fonte 8 pt; style sheet: painel `#1a1a1a` sem borda, margem 0 4 4 4; abas transparentes, texto `#995600`, padding 2px 8px 4px 8px; aba selecionada `#ff9000`; hover `#555555`. Cada página com margens 5,5,5,5.
- Tamanho pedido (`width_request`/`height_request`): mínimo. Margens (`margin_*`): numa caixa somam às margens de conteúdo; num controle, o controle é posto num `QWidget` com essas margens e centralizado.
- Outras classes (janela, `GtkImage`, `GtkEventBox`...) : se tiverem filho, vira o filho; senão são ignoradas.
- Controle com `var_id` sem parâmetro registrado fica desabilitado.

Cabeçote (`showAmpControls`): com o seletor `tube.select` no último valor ("---", sem amp) só aparecem `gxbigknobgain` e `labelgain2`; com amp, aparecem `gxmediumknobpregain`, `gxmediumknobdrive`, `gxmediumknobdist`, `gxmediumknobgain` e seus rótulos (`labelpregain`, `labeldrive`, `labeldist`, `labelgain`), e o knob grande some. Atualiza a cada mudança do seletor.

`plainControls` (unidade com load_ui mas sem glade): parâmetros em linha (`QHBoxLayout` espaçamento 10), cada um rótulo `.rack_label` em cima e `Knob SmallR` (ou `Selector` se tiver valores) embaixo. Nenhuma unidade atual cai aqui. `trbuff` não tem load_ui: mostra só o título.

#### Widgets próprios e seu desenho (`GuitarixWidgets.cpp`)

Cores: controle `#ff9000` (ControlColor); desligado/anel apagado `#211006`; rótulo `#222222`; moldura do seletor `#272727`; fundo do medidor `#222222`; fundo de entrada `#1a1a1a`.

**Knob** (`GxKnob`, `FloatModelEditorBase` vertical, arraste/roda/automação/valor no hover). Tamanho fixo = imagem + 2*8 em x, + 2*4 em y:

| Tipo (classe glade) | Imagem | Tamanho do widget | Raio do anel | Largura do anel | LED (traço / espaço) | Raio do indicador | Largura do indicador | Comprimento do indicador |
|---|---|---|---|---|---|---|---|---|
| Small (`GxSmallKnob`) | knob_small 38x38 | 54 x 46 | 15 | 2 | 3 / 2 | 11 | 2 | 5 |
| SmallR (`GxSmallKnobR`) | knob_small_r 68x68 | 84 x 76 | 26 | 3 | 3 / 2 | 12 | 3 | 3 |
| Mid (`GxMidKnob`) | knob_mid 90x90 | 106 x 98 | 35 | 6 | 2 / 2 | 27 | 2 | 6 |
| Big (`GxBigKnob`) | knob_big 114x134 | 130 x 142 | 50 | 3 | sem tracejado | 46 | 3 | 6 |

Desenho (numa superfície do tamanho da imagem, depois centralizada no widget): imagem em (0,0); centro = (w_img/2, h_img/2); zona morta embaixo de 40° de cada lado (`ScaleZero` = 40°); ângulo = 40° + estado * 2*(180° - 40°) (estado 0..1, normValue). Anel: arco tracejado (padrão = ledSize/ringWidth, ledDistance/ringWidth; ponta reta) em `#211006`, de (90°+40°) com extensão (320° - 40°) = 280° em sentido horário (ângulos do Cairo convertidos para Qt com sinal invertido). Arco do valor no mesmo traço em `#ff9000` (ou `#211006` se desabilitado), de 130° por (ângulo - 40°). Se largura do anel ≥ 3: passada SoftLight sobre o arco com gradiente radial (preto alfa 0.6 nas bordas, branco alfa 0.4 no meio). Indicador: linha de (centro + (-sin a * r_ind, cos a * r_ind)) até (centro + (-sin a * (r_ind - comp), cos a * (r_ind - comp))), cor do controle, ponta reta; com anel ≥ 3 também passada SoftLight.

**Selector** (`GxSelector`): fonte 10 px; bordas 4 (topo), 24 (dir), 4 (baixo), 8 (esq); raio 3; padding superior 3; borda do valor 4 de cada lado. Tamanho fixo = (texto mais largo dos rótulos + 2*4 + 8 + 24, altura da fonte + 1 + 3 + 4 + 4). Desenho: retângulo arredondado (raio 3) `#272727`; área de texto (8, 4, w-32, h-8) preta; inset (gradiente preto alfa 0.33 → branco alfa 0.1, expandido 1 px); preto de novo; "glass" (sombra preta alfa 0.5 → 0 nos 3 px de cima). Ícone `selector_icon` centrado na borda direita: x = w - (24 + 15)/2, y = (h - 15)/2. Texto do valor em `#ff9000` (desabilitado `#211006`), centralizado no topo da área de texto + 3 px. Clique esquerdo sem modificador abre `QMenu` com todos os valores (marcado o atual) em (0, h); arrastar não muda valor; duplo clique = clique; roda passa pelos valores; cursor mão.

**Meter** (`GxFastMeter`): LED 2 x 2, borda 1, dimensão padrão 2 (`dimen` do glade se > 0), mínimo 1 LED. sizeHint vertical = (1 + dim*3, 1 + 1*3) = 7 x 4 com dim 2; horizontal invertido. Ocupa o espaço dado, barra no começo. Desenho: fundo `#222222`; tamanho cortado para não ter LED pela metade; gradiente de baixo para cima (ou esquerda→direita): 0 `#b96901`, 0.717 `#d4cd00`, `top=(n-2)/n` `#ff0500`, top+0.0001 `#ff1100`; linhas de 1 px `#1a1a1a` separando LEDs a cada 3 px e colunas a cada 3 px; inset; tudo escurecido com preto alfa 0.8; depois a parte acesa (n_leds * nível arredondado) redesenhada sem escurecer, de baixo para cima (vertical) ou da esquerda (horizontal). Atualização: sinal `periodicUpdate` da MainWindow. Escala: parâmetro com passo ≠ 0 ou opção `L` = potência em dB via `log_meter` (tabela por faixas de -70 a +6 dB, máximo 115); senão posição linear na faixa. Opção `M` = pico (lido e zerado a cada leitura). `falloff=True`: cai linearmente 1 escala em 500 ms.

**HSlider** (`GxHSlider`): tamanho fixo 134 x 24; trilho = recorte (0,0,134,24) de hslider.png; cursor = recorte (134 ou 170 em hover, 0, 36, 24) em x = lround(estado * (134 - 36)). Manipulação horizontal.

**VSlider** (`GxVSlider`): tamanho fixo 24 x 134; trilho = (0,0,24,134); cursor = recorte (0, 134 ou 170 em hover, 24, 36) em y = (134-36) - lround(estado*(134-36)) (para cima = mais).

**Switch** (`GxSwitch`): imagens `<base>_on`/`<base>_off` (base padrão `switch`); tamanho = maior imagem + 4 em cada eixo (switch 20x20, switch_mid/minitoggle 14x14, frbutton 25x23); imagem centralizada; desabilitado com opacidade 0.5; clique esquerdo alterna min/max; ligado se valor > (min+max)/2; cursor mão.

**PaintBox** (`GxPaintBox`): `box_uni_1_expose` → `background1` ladrilhado; `box_uni_2_expose` → `background2` ladrilhado; outras funções (`live_box_expose`, nenhuma) → não pinta nada.

**PortDisplay** (`GxPortDisplay`): mínimo 40 x 7 (metade da imagem, altura da imagem + 2), altura fixa; desenha a fatia (40 - valor_px, 0, 40, 5) de portdisplay.png em (0,0), valor_px = lround((v - min)/(max - min) * 40); atualização por `periodicUpdate`.

**ValueDisplay** (`GxValueDisplay`): fonte 7 pt; tamanho fixo = (largura de "-000." + `round_digits` zeros + 16, altura da fonte + 8); retângulo arredondado raio 6 preto + glass; valor com `round_digits` casas, centralizado, em `#ff9000`.

#### Unidades: árvore de layout de cada uma

Legenda: `GtkBox H/V sp=N` = caixa horizontal/vertical com espaçamento N; `{...}` = propriedades do objeto (tamanho pedido, margens, alinhamentos, expand do próprio filho); `<...>` = empacotamento no pai (`expand`, `fill`, `padding`, `pack_type`, ou `left_attach/top_attach/width/height` no grid); `.classe` = classe de estilo. Controle: classe glade, `var_id`, **nome do parâmetro** (é o que o rótulo `label_ref` mostra e o prefixo da dica `"<nome>:"` do knob/seletor/slider/chave), `[tipo opções; padrão, mín, máx, passo]` do registro, `dica:` = descrição registrada (não mostrada pelo código atual, exceto `tooltip_text` dos medidores, listado como `tooltip=`). Todas as dicas de knobs/seletores/sliders/chaves são `setHintText("<nome>:", "")` (sem sufixo de unidade). O título de cada janela é o nome entre aspas no cabeçalho da unidade. A ordem é a do navegador (`GuitarixUnits.cpp`).

#### 1. `gain` — "Volume" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/gain.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `gain.gain` "Gain" [Float S; 0.0,-4e+01,4e+01,0.1] dica:"gain (dB)" label_ref=`label1:rack_label` <expand=False fill=False>

#### 2. `low_highpass` — "Low/High Filter" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/low_high_pass.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox H sp=4 `hbox2` <expand=True fill=False>
      - GtkBox V sp=0 `vbox6` <expand=True fill=True>
        - Label `label2` "" xalign=0.5 <expand=True fill=False>
        - GxSwitch `low_high_pass.lhp.on_off` "low fi.highpass" [Float B; 0.0,0.0,1.0,1.0] base_name=switch <expand=False fill=False>
        - Label `label4` "" xalign=0.5 <expand=True fill=True>
      - GtkBox V sp=0 `vbox2` <expand=False fill=False>
        - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `low_high_pass.lhp.high_freq` "Highpass" [Float S; 1.3e+02,2e+01,7.04e+03,1e+01] label_ref=`label1:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox3` <expand=False fill=False>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `low_high_pass.lhp.low_freq` "Lowpass" [Float S; 5e+03,2e+01,1.2e+04,1e+01] label_ref=`label2:rack_label` <expand=False fill=False>
    - Label `label1` "    " xalign=0.5 <expand=True fill=True>
    - GtkBox H sp=4 `hbox3` <expand=True fill=False>
      - GtkBox V sp=0 `vbox4` <expand=False fill=False>
        - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `low_high_pass.lhc.low_freq` "Lowcut" [Float S; 1.3e+02,2e+01,1e+03,1e+01] dica:"low-freq cutoff Hz" label_ref=`label3:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox5` <expand=False fill=False>
        - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `low_high_pass.lhc.high_freq` "Highcut" [Float S; 5e+03,1e+03,1.2e+04,1e+01] dica:"high-freq cutoff Hz" label_ref=`label4:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox7` <expand=True fill=True>
        - Label `label3` "" xalign=0.5 <expand=True fill=False>
        - GxSwitch `low_high_pass.lhc.on_off` "low highcutoff" [Float B; 0.0,0.0,1.0,1.0] base_name=switch <expand=False fill=False>
        - Label `label5` "" xalign=0.5 <expand=True fill=True>

#### 3. `highbooster` — "Treble Boost" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/highbooster.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `highbooster.Level` "Level" [Float S; 0.5,0.0,2e+01,0.5] label_ref=`label1:rack_label` <expand=False fill=False>

#### 4. `eqs` — "Scaleable EQ" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/selecteq.cc`

- GxPaintBox H sp=0 paint=box_uni_2_expose `rackbox` {margin_left=4 margin_right=5}
  - GtkGrid `table1` colsp=0 rowsp=10 homog=F/F {margin_left=10 margin_right=10} <expand=True fill=True>
    - GtkBox V sp=5 `vbox2` <left_attach=0 top_attach=2>
      - Label `label1:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs31_25` "Qs31_25" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox3` <left_attach=1 top_attach=2>
      - Label `label2:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs62_5` "Qs62_5" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <left_attach=2 top_attach=2>
      - Label `label3:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs125` "Qs125" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox5` <left_attach=3 top_attach=2>
      - Label `label4:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs250` "Qs250" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox6` <left_attach=4 top_attach=2>
      - Label `label5:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs500` "Qs500" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox7` <left_attach=5 top_attach=2>
      - Label `label6:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs1k` "Qs1k" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox8` <left_attach=6 top_attach=2>
      - Label `label7:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs2k` "Qs2k" [Float S; 5e+01,1.0,1e+02,1.0] dica:"bandwidth" <expand=False fill=False>
    - GtkBox V sp=5 `vbox9` <left_attach=7 top_attach=2>
      - Label `label8:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs4k` "Qs4k" [Float S; 5e+01,1.0,1e+02,1.0] <expand=False fill=False>
    - GtkBox V sp=5 `vbox10` <left_attach=8 top_attach=2>
      - Label `label9:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs8k` "Qs8k" [Float S; 5e+01,1.0,1e+02,1.0] <expand=False fill=False>
    - GtkBox V sp=5 `vbox11` <left_attach=9 top_attach=2>
      - Label `label10:rack_label_inverse` "Q" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `eqs.Qs16k` "Qs16k" [Float S; 5e+01,1.0,1e+02,1.0] <expand=False fill=False>
    - GxVSlider `eqs.fs31_25` "Fs31_25" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 31.25 Hz" {hexpand=True} <left_attach=0 top_attach=1>
    - GxVSlider `eqs.fs62_5` "Fs62_5" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 62.5 Hz" {margin_left=8 margin_right=8 hexpand=True} <left_attach=1 top_attach=1>
    - GxVSlider `eqs.fs125` "Fs125" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 125 Hz" {hexpand=True} <left_attach=2 top_attach=1>
    - GxVSlider `eqs.fs250` "Fs250" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 250 Hz" {hexpand=True} <left_attach=3 top_attach=1>
    - GxVSlider `eqs.fs500` "Fs500" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 500 Hz" {hexpand=True} <left_attach=4 top_attach=1>
    - GxVSlider `eqs.fs1k` "Fs1k" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 1 kHz" {hexpand=True} <left_attach=5 top_attach=1>
    - GxVSlider `eqs.fs2k` "Fs2k" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 2 kHz" {hexpand=True} <left_attach=6 top_attach=1>
    - GxVSlider `eqs.fs4k` "Fs4k" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 4 kHz" {hexpand=False} <left_attach=7 top_attach=1>
    - GxVSlider `eqs.fs8k` "Fs8k" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 8 kHz" {hexpand=True} <left_attach=8 top_attach=1>
    - GxVSlider `eqs.fs16k` "Fs16k" [Float S; 0.0,-4e+01,3e+01,0.1] dica:"gain (dB) at 16 kHz" {hexpand=True} <left_attach=9 top_attach=1>
    - GxValueDisplay `eqs.freq31_25` "Freq31_25" [Float S; 31.25,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=0 top_attach=0>
    - GxValueDisplay `eqs.freq62_5` "Freq62_5" [Float S; 62.5,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=1 top_attach=0>
    - GxValueDisplay `eqs.freq125` "Freq125" [Float S; 125.0,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=2 top_attach=0>
    - GxValueDisplay `eqs.freq250` "Freq250" [Float S; 2.5e+02,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=3 top_attach=0>
    - GxValueDisplay `eqs.freq500` "Freq500" [Float S; 5e+02,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=4 top_attach=0>
    - GxValueDisplay `eqs.freq1k` "Freq1k" [Float S; 1e+03,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=5 top_attach=0>
    - GxValueDisplay `eqs.freq2k` "Freq2k" [Float S; 2e+03,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=6 top_attach=0>
    - GxValueDisplay `eqs.freq4k` "Freq4k" [Float S; 4e+03,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=7 top_attach=0>
    - GxValueDisplay `eqs.freq8k` "Freq8k" [Float S; 8e+03,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=8 top_attach=0>
    - GxValueDisplay `eqs.freq16k` "Freq16k" [Float S; 1.6e+04,2e+01,2e+04,1.0] dica:"Hz" round_digits=0 <left_attach=9 top_attach=0>

#### 5. `ampstack` — "Amp" (categoria Guitarix: -)

Glade: `gx/builder/ampbox.glade` raiz `amp_background`, `gx/builder/mainpanel.glade` raiz `ampdetail_normal` ; código: `GuitarixAmp.cpp`

- GxPaintBox H sp=0 paint=(nenhuma) `amp_background:ampbox` .ampbox
  - GtkBox H sp=5 `hboxmain` {margin_left=20 margin_right=20 margin_top=3 margin_bottom=3} <expand=True fill=True>
    - GtkGrid `tableampleft` colsp=3 rowsp=3 homog=F/F <expand=True fill=True>
      - Label `labelpregain:effekt_label` "Pre Gain" .effekt_label xalign=0.5 <left_attach=0 top_attach=0>
      - Label `labeldrive:effekt_label` "Drive" .effekt_label xalign=0.5 <left_attach=1 top_attach=3>
      - Label `labeldist:effekt_label` "Clean/Dist" .effekt_label xalign=0.5 <left_attach=0 top_attach=3>
      - Label `labelgain:effekt_label` "Master Gain" .effekt_label xalign=0.5 <left_attach=1 top_attach=0>
      - GxSmallKnobR `gxdistortion.drive` "Drive" [Float SA; 0.35,0.0,1.0,0.01] <left_attach=1 top_attach=2>
      - GxSmallKnobR `amp2.stage2.gain1` "Gain1" [Float SA; -6.0,-2e+01,2e+01,0.1] <left_attach=1 top_attach=1>
      - GxSmallKnobR `gxdistortion.wet_dry` "Wet_dry" [Float SA; 1e+02,0.0,1e+02,1.0] {vexpand=True} <left_attach=0 top_attach=2>
      - GxSmallKnobR `amp2.stage1.Pregain` "Pregain" [Float SA; -6.0,-2e+01,2e+01,0.1] {vexpand=True} <left_attach=0 top_attach=1>
      - Label `labeldummy1:effekt_label` "" .effekt_label xalign=0.5 {vexpand=True} <left_attach=2 top_attach=0>
      - Label `labelgain2:effekt_label` "Master Gain" .effekt_label xalign=0.5 <left_attach=2 top_attach=3>
      - GxBigKnob `amp2.stage2.gain1` "Gain1" [Float SA; -6.0,-2e+01,2e+01,0.1] {vexpand=True} <left_attach=2 top_attach=1 height=2>
    - GtkGrid `tableampcenter` colsp=0 rowsp=16 homog=F/F <expand=True fill=True>
      - GxSelector `tube.select` "select" [Int S; enum: 19 amps (gxamp...gxnoamp; o último, "---", = sem amp)] <left_attach=0 top_attach=0>
      - GtkBox H sp=0 `hboxcenter` <left_attach=0 top_attach=1>
        - GtkGrid `tablebassboost` colsp=0 rowsp=4 homog=F/F <expand=True fill=True>
          - GxSmallKnobR `bassbooster.Level` "Level" [Float S; 1e+01,0.5,2e+01,0.5] <left_attach=0 top_attach=1>
          - GxSwitch `amp.bass_boost.on_off` "on/off" [Bool B; padrão ligado] base_name=minitoggle {halign=center} <left_attach=0 top_attach=2>
          - Label `label6:effekt_label` " Bass Boost  " .effekt_label xalign=0.5 <left_attach=0 top_attach=0>
        - GtkGrid `tablepresence` colsp=0 rowsp=4 homog=F/F <expand=True fill=True>
          - GxSmallKnobR `con.Level` "Level" [Float S; 1.0f,0.1f,5.0f,0.1f] <left_attach=0 top_attach=1>
          - GxSwitch `con.on_off` "on/off" [Bool B; padrão ligado] base_name=minitoggle {halign=center} <left_attach=0 top_attach=2>
          - Label `label7:effekt_label` " Presence " .effekt_label xalign=0.5 <left_attach=0 top_attach=0>
        - GtkGrid `tablereverb` colsp=0 rowsp=4 homog=F/F <expand=True fill=True>
          - GxSmallKnobR `amp.wet_dry` "Wet_dry" [Float S; 0.0,-1.0,1.0,0.01] <left_attach=0 top_attach=1>
          - GxSwitch `amp.feed_on_off` "reverb_on_of" [Float B; 0.0,0.0,1.0,1.0] base_name=minitoggle {halign=center} <left_attach=0 top_attach=2>
          - Label `label8:effekt_label` " Reverb  " .effekt_label xalign=0.5 <left_attach=0 top_attach=0>
      - GtkBox V sp=0 (sem id) .meterbox {valign=center} <left_attach=0 top_attach=2>
        - GtkBox V sp=0 (sem id) .meterbox .stereo {valign=center} <expand=False fill=True>
          - GxFastMeter `maxlevel.left` "Left" [Float SLMO; 0.f,0.f,1.f,0.f] dica:"Overall Rack output" orient=horizontal dimen=0 falloff=True {valign=center} <expand=False fill=True>
          - GxFastMeter `maxlevel.right` "Right" [Float SLMO; 0.f,0.f,1.f,0.f] dica:"Overall Rack output" orient=horizontal dimen=0 falloff=True {valign=center} <expand=False fill=True>
    - GtkGrid `tableampright` colsp=0 rowsp=6 homog=F/F <expand=True fill=True>
      - GxBigKnob `amp.out_master` "Level" [Float S; 0.0,-5e+01,4.0,0.1] dica:"Overall Rack output Volume" {vexpand=True} <left_attach=0 top_attach=1>
      - Label `labelmaster:effekt_label` " Master Volume  " .effekt_label xalign=0.5 <left_attach=0 top_attach=2>
      - GxPaintBox H sp=0 paint=live_box_expose `gxlogobox` {vexpand=True} <left_attach=0 top_attach=0>
      - GtkBox V sp=0 (sem id) <left_attach=1 top_attach=1>
        - Label (sem id) "Limiter" xalign=0.5 {margin_top=6 halign=center valign=start} <expand=False fill=True>
        - GtkBox H sp=0 (sem id) {height_request=80 margin_bottom=25} <expand=True fill=True>
          - GxFastMeter `hardlim.vleft` "Vleft" [Float SON; 0,0.0,1.0,0] dica:"Rack output limiter left" orient=vertical dimen=0 falloff=True {margin_left=10 halign=center} <expand=True fill=False>
          - GxFastMeter `hardlim.vright` "Vright" [Float SON; 0,0.0,1.0,0] dica:"Rack output limiter right" orient=vertical dimen=0 falloff=True {margin_right=10 halign=center} <expand=True fill=False>
        - GxSwitch `antyfizz.on_off` "on/off" [Bool B; padrão desligado] base_name=minitoggle {margin_left=5 margin_right=5 margin_top=5 margin_bottom=1 halign=center} <expand=False fill=True>
      - Label (sem id) "Anti Fizz" xalign=0.5 <left_attach=1 top_attach=2>
      - GtkBox V sp=0 (sem id) <left_attach=1 top_attach=0>
        - GxSwitch `amp.highgain` "Highgain" [Float BA; 0.0,0.0,1.0,1.0] dica:"Allow frequencies below 31Hz" base_name=minitoggle {margin_left=5 margin_right=5 margin_top=5 margin_bottom=5 halign=center} <expand=False fill=True>
        - Label (sem id) "LowEnd" xalign=0.5 <expand=False fill=True>

- GtkBox H sp=26 `ampdetail_normal` {margin_left=4 margin_right=4 margin_bottom=4 halign=end}
  - GtkGrid `table3` colsp=2 rowsp=2 homog=F/F {margin_top=4} <expand=False fill=True>
    - Label `label4:beffekt_label` "Noise Gate" .beffekt_label xalign=0 <left_attach=0 top_attach=0 width=2>
    - GxSwitch `noise_gate.on_off` "on/off" [Bool B; padrão desligado] base_name=minitoggle {halign=end valign=center} <left_attach=0 top_attach=1>
    - GtkAlignment `alignment44` pad L0 T0 R0 B0 <left_attach=1 top_attach=1>
      - GxSmallKnobR `noise_gate.threshold` "Threshold" [Float S; 0.017f,0.01f,0.31f,0.001f]
  - GtkGrid `table4` colsp=2 rowsp=2 homog=F/F {margin_top=4} <expand=False fill=True>
    - Label `label5:beffekt_label` "Compressor" .beffekt_label xalign=0.5 <left_attach=0 top_attach=0 width=2>
    - GxSwitch `shaper.on_off` "on/off" [Bool B; padrão desligado] base_name=minitoggle {halign=end valign=center} <left_attach=0 top_attach=1>
    - GtkAlignment `alignment1` pad L0 T0 R0 B0 <left_attach=1 top_attach=1>
      - GxSmallKnobR `shaper.sharper` "sharper" [Float S; 1.0,1.0,1e+01,1.0]
  - GtkBox V sp=0 `vbox1` <expand=False fill=True pack_type=end>
    - (GtkImage `image4` ignorado)
  - GtkGrid `table5` colsp=2 rowsp=2 homog=F/F {margin_top=4} <expand=False fill=True>
    - Label `label6:beffekt_label` "Mono Level Out" .beffekt_label xalign=0.5 <left_attach=0 top_attach=0 width=2>
    - GxSwitch `amp.on_off` "on/off" [Bool B; padrão desligado] base_name=minitoggle {halign=end valign=center} <left_attach=0 top_attach=1>
    - GtkAlignment `alignment24` pad L0 T0 R0 B0 <left_attach=1 top_attach=1>
      - GxSmallKnobR `amp.out_amp` "Level" [Float S; 0.0,-2e+01,4.0,0.1]
  - GtkGrid `table6` colsp=2 rowsp=2 homog=F/F {margin_top=4} <expand=False fill=True>
    - Label `label7:beffekt_label` "Clip" .beffekt_label xalign=0.5 <left_attach=0 top_attach=0 width=2>
    - GxSwitch `amp.clip.on_off` "on/off" [Bool B; padrão desligado] base_name=minitoggle {halign=end valign=center} <left_attach=0 top_attach=1>
    - GtkAlignment `alignment34` pad L0 T0 R0 B0 <left_attach=1 top_attach=1>
      - GxSmallKnobR `amp.fuzz` "Fuzz" [Float S; 0.0,0.0,1.99,0.01]


#### 6. `crybaby` — "Crybaby" (categoria Guitarix: Guitar Effects)

Glade: `gx/builder/crybaby_ui.glade` raiz `rackbox`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox3` <expand=True fill=False>
    - GxSelector `crybaby.autowah` "select" [Int S; 0,0,0 (enum: crybaby/autowah)] <expand=False fill=False padding=8>
    - GtkBox V sp=5 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "Wah" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `crybaby.wah` "Wah" [Float SA; 0.0f,0.0f,1.0f,0.01f] <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=True>
      - GxMidKnob `crybaby.level` "Level" [Float SA; 0.1f,0.0f,1.0f,0.01f] <expand=False fill=True>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "Dry/Wet" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `crybaby.wet_dry` "dry/wet" [Float SA; 1e+02f,0.0f,1e+02f,1.0f] <expand=False fill=True>
    - Label `label1` "                      " xalign=0.5 <expand=True fill=True>

#### 7. `poweramp` — "PowerAmp" (categoria Guitarix: Guitar Effects)

Glade: `gx/builder/poweramps_ui.glade` raiz `rackbox`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox6` <expand=True fill=True>
      - GtkBox H sp=60 (sem id) <expand=True fill=True>
        - GtkBox V sp=0 (sem id) <expand=False fill=True>
          - Label `label1:rack_label` "Input" .rack_label xalign=0.5 <expand=False fill=True>
          - GxMidKnob `poweramp.Pregain` "Pregain" [Float SA; 0.0,-3e+01,3e+01,0.1] <expand=False fill=True>
        - GtkBox V sp=0 (sem id) <expand=False fill=True>
          - GxSelector `poweramp.mode` "select" [Int S; 0,0,0 (enum: 6 poweramps)] <expand=False fill=False>
        - GtkBox V sp=0 (sem id) <expand=False fill=True>
          - Label `label2:rack_label` "Output" .rack_label xalign=0.5 <expand=False fill=True>
          - GxMidKnob `poweramp.Gain` "Gain" [Float SA; 0.0,-3e+01,3e+01,0.1] <expand=False fill=True>

#### 8. `wah` — "Wah" (categoria Guitarix: Guitar Effects)

Glade: `gx/builder/wah_ui.glade` raiz `rackbox`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=1 `hbox3` <expand=True fill=False>
    - GtkGrid `table1` colsp=6 rowsp=4 homog=F/F <expand=False fill=False>
      - GxSelector `wah.mode` "Mode" [Float SA; 0.0,0.0,2.0,1.0] <left_attach=1 top_attach=2>
      - GxSelector `wah.select` "select" [Int S; 0,0,0 (enum: 10 wahs)] <left_attach=1 top_attach=1>
      - Label `label2` "" xalign=0.5 <left_attach=1 top_attach=0>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "Wah" .rack_label xalign=0.5 <expand=False fill=True>
      - GxMidKnob `wah.Wah` "Wah" [Float SA; 0.0,0.0,1.0,0.01] <expand=False fill=False>
    - GtkBox V sp=5 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "Alien Freq" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `wah.freq` "Alien Freq" [Float SA; 24.0,24.0,3.6e+02,1.0] dica:"LFO in Beats per Minute" <expand=False fill=True>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "Dry/Wet" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `wah.wet_dry` "dry/wet" [Float SA; 1e+02,0.0,1e+02,1.0] <expand=False fill=True>
    - Label `label1` "          " xalign=0.5 <expand=True fill=True>

#### 9. `smbPitchShift` — "Detune" (categoria Guitarix: Misc)

Glade: `gx/builder/gx_detune_ui.glade` raiz `rackbox` ; código: `GuitarixDetune.cpp`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=20 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `smbPitchShift.semitone` "Detune" [Float S; 0.0,-12.,12.,0.1] label_ref=`label1:rack_label` <expand=True fill=True>
      - GxSelector `smbPitchShift.octave` "add harmonics" [Int B; 0,0,0,octave_values] dica:"add harmonics" <expand=False fill=False pack_type=end>
    - GtkBox V sp=1 `vbox3` <expand=True fill=True>
      - GtkBox H sp=5 `hbox2` <expand=True fill=True>
        - GtkBox V sp=0 `vbox4` <expand=True fill=True>
          - Label `label10:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `smbPitchShift.dry` "Dry" [Float S; 50.0,0.0,100.0,1] dica:"Dry amount" label_ref=`label10:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox5` <expand=True fill=True>
          - Label `label:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `smbPitchShift.wet` "Wet" [Float S; 50.0,0.0,100.0,1] dica:"Wet amount" label_ref=`label:rack_label` <expand=False fill=False>
      - GxSelector `smbPitchShift.latency` "latency settings" [Int B; 0,0,0,latency_set] dica:"latency settings" <expand=False fill=False pack_type=end>
      - GxSelector `smbPitchShift.l` "compensate latency" [Float S; 0.0f,0.0f,1.0f,1.0f] dica:"compensate latency" <expand=False fill=False pack_type=end>
    - GtkBox H sp=5 `hbox3` <expand=True fill=True>
      - GtkBox V sp=0 `vbox6` <expand=True fill=True>
        - Label `label12:rack_label` "Sub" .rack_label xalign=0.5 <expand=True fill=True>
        - GxVSlider `smbPitchShift.a` "low" [Float S; 1.0,0.0,2.0,0.01] dica:"Sub" <expand=True fill=True>
      - GtkBox V sp=0 `vbox7` <expand=True fill=True>
        - Label `label22:rack_label` "Low" .rack_label xalign=0.5 <expand=True fill=True>
        - GxVSlider `smbPitchShift.b` "middle low" [Float S; 1.0,0.0,2.0,0.01] dica:"Low" <expand=True fill=True>
      - GtkBox V sp=0 `vbox8` <expand=True fill=True>
        - Label `label32:rack_label` "Mid" .rack_label xalign=0.5 <expand=True fill=True>
        - GxVSlider `smbPitchShift.c` "middle treble" [Float S; 1.0,0.0,2.0,0.01] dica:"Mid" <expand=True fill=True>
      - GtkBox V sp=0 `vbox9` <expand=True fill=True>
        - Label `label42:rack_label` "Hi" .rack_label xalign=0.5 <expand=True fill=True>
        - GxVSlider `smbPitchShift.d` "treble" [Float S; 1.0,0.0,2.0,0.01] dica:"Hi" <expand=True fill=True>

#### 10. `gx_distortion` — "Multi Band Distortion" (categoria Guitarix: Distortion)

Glade: `gx/builder/gx_distortion_ui.glade` raiz `rackbox` ; código: `gx/faust-generated/gx_distortion.cc`

- GtkBox H sp=1 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=5 `vbox3` <expand=True fill=True>
      - GtkFrame `frame1:frame_inversed` label="" label_xalign=0 .frame_inversed <expand=True fill=True>
        - GtkAlignment `alignment1` pad L4 T4 R4 B4
          - GtkBox H sp=4 `hbox3`
            - GtkBox V sp=0 `vbox4` <expand=True fill=True>
              - Label `label12:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.drive` "Drive" [Float S; 0.64,0.0,1.0,0.01] label_ref=`label12:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox5` <expand=True fill=True>
              - Label `label13:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.low_drive` "Lo" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label13:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox6` <expand=True fill=True>
              - Label `label14:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.middle_l_drive` "LoMid" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label14:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox7` <expand=True fill=True>
              - Label `label15:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.middle_h_drive` "HiMid" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label15:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox8` <expand=True fill=True>
              - Label `label16:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.high_drive` "Hi" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label16:rack_label` <expand=True fill=True>
        - (rótulo do frame:)
        - Label `label1:rack_label` "Drive" .rack_label xalign=0.5
      - GtkFrame `frame2:frame_inversed` label="" label_xalign=0 .frame_inversed <expand=True fill=True>
        - GtkAlignment `alignment2` pad L4 T4 R4 B4
          - GtkBox H sp=4 `hbox4`
            - GtkBox V sp=0 `vbox9` <expand=True fill=True>
              - Label `label17:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.gain` "Gain" [Float S; 2.0,-1e+01,1e+01,0.1] label_ref=`label17:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox11` <expand=True fill=True>
              - Label `label18:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.low_gain` "Lo" [Float S; 1e+01,-1e+01,2e+01,0.1] label_ref=`label18:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox10` <expand=True fill=True>
              - Label `label19:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.middle_l_gain` "LoMid" [Float S; 1e+01,-1e+01,2e+01,0.1] label_ref=`label19:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox12` <expand=True fill=True>
              - Label `label20:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.middle_h_gain` "HiMid" [Float S; 1e+01,-1e+01,2e+01,0.1] label_ref=`label20:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox13` <expand=True fill=True>
              - Label `label21:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `gx_distortion.high_gain` "Hi" [Float S; 1e+01,-1e+01,2e+01,0.1] label_ref=`label21:rack_label` <expand=True fill=True>
        - (rótulo do frame:)
        - Label `label2:rack_label` "Gain" .rack_label xalign=0.5
      - GtkBox H sp=8 `hbox2` <expand=True fill=True>
        - GtkFrame `frame4:frame_inversed` label="" label_xalign=0 .frame_inversed <expand=True fill=True>
          - GtkAlignment `alignment4` pad L4 T4 R4 B4
            - GtkBox H sp=4 `hbox6`
              - GtkBox V sp=0 `vbox17` <expand=True fill=True>
                - Label `label7:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
                - GxSmallKnobR `gx_distortion.trigger` "Trigger" [Float S; 0.12,0.0,1.0,0.01] label_ref=`label7:rack_label_inverse` <expand=True fill=True>
              - GtkBox V sp=0 `vbox18` <expand=True fill=True>
                - Label `label8:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
                - GxSmallKnobR `gx_distortion.vibrato` "Vibrato" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label8:rack_label_inverse` <expand=True fill=True>
              - GtkBox V sp=0 `vbox19` <expand=True fill=True>
                - Label `label22` "
" xalign=0.5 <expand=False fill=False>
                - GxSwitch `gx_distortion.resonator.on_off` "resonat" [Float B; 0.0,0.0,1.0,1.0] base_name=switch_mid {width_request=32 height_request=32} <expand=True fill=True>
                - Label `label1` "
" xalign=0.5 <expand=False fill=False>
          - (rótulo do frame:)
          - Label `label4:rack_label_inverse` "Resonator" .rack_label_inverse xalign=0.5
        - GtkFrame `frame3:frame_inversed` label="" label_xalign=0 .frame_inversed <expand=True fill=True>
          - GtkAlignment `alignment3` pad L4 T4 R4 B4
            - GtkBox H sp=6 `hbox5`
              - GtkBox V sp=0 `vbox14` <expand=True fill=True>
                - Label `label9:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
                - GxSmallKnob `gx_distortion.split_low_freq` "Split Lo" [Float S; 2.5e+02,2e+01,6e+02,1e+01] label_ref=`label9:rack_label_inverse` <expand=True fill=True>
              - GtkBox V sp=0 `vbox15` <expand=True fill=True>
                - Label `label10:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
                - GxSmallKnob `gx_distortion.split_middle_freq` "Split Mid" [Float S; 6.5e+02,6e+02,1.25e+03,1e+01] label_ref=`label10:rack_label_inverse` <expand=True fill=True>
              - GtkBox V sp=0 `vbox16` <expand=True fill=True>
                - Label `label11:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
                - GxSmallKnob `gx_distortion.split_high_freq` "Split Hi" [Float S; 1.25e+03,1.25e+03,1.2e+04,1e+01] label_ref=`label11:rack_label_inverse` <expand=True fill=True>
          - (rótulo do frame:)
          - Label `label3:rack_label_inverse` "X-Over" .rack_label_inverse xalign=0.5
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
      - GxMidKnob `gx_distortion.level` "Level" [Float S; 0.0,0.0,0.5,0.01] label_ref=`label5:rack_label` <expand=True fill=True>
      - GtkBox V sp=0 `vbox20` <expand=True fill=True padding=10>
      - Label `label6:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GxMidKnob `gx_distortion.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label6:rack_label_inverse` <expand=True fill=True>

#### 11. `bitdowner` — "BitDowner" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/faust-generated/bitdowner.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bitdowner.input_gain` "Input" [Float S; 0.0,-4e+01,4e+01,0.1] dica:"Gain (dB)" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bitdowner.bit_down` "Bit Down" [Float S; 16.0,1.0,16.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bitdowner.downsampling` "Smpl Down" [Float S; 1.0,1.0,2e+02,1.0] dica:"Downsampling (samples to skip ba.count)" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bitdowner.volume` "Volume" [Float S; 0.0,-9e+01,12.0,0.1] dica:"Volume (dB)" label_ref=`label4:rack_label` <expand=False fill=False>

#### 12. `thick_distortion` — "Thick Distortion" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/faust-generated/thick_distortion.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `thick_distortion.input_gain` "Input" [Float S; 0.0,0.0,1.2e+02,0.1] dica:"Gain (dB)" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `thick_distortion.thickness` "Thickness" [Float S; 0.8,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `thick_distortion.volume` "Volume" [Float S; 0.0,-9e+01,4e+01,0.1] dica:"Volume (dB)" label_ref=`label4:rack_label` <expand=False fill=False>

#### 13. `ts9sim` — "Tube Screamer" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/ts9sim.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ts9sim.drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ts9sim.level` "Level" [Float S; -16.0,-2e+01,4.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ts9sim.tone` "Tone" [Float SL; 4e+02,1e+02,1e+03,1.03] label_ref=`label3:rack_label` <expand=False fill=False>

#### 14. `aclipper` — "Rat" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/aclipper.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `aclipper.drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `aclipper.level` "Level" [Float S; -2.0,-2e+01,12.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `aclipper.tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 15. `mxrdis` — "MXR Distortion Plus" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/mxrdist.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "Volume" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `mxrdis.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "Distortion" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `mxrdis.drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>

#### 16. `bossds1` — "DS1" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/bossds1.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bossds1.drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bossds1.Level` "Level" [Float S; 3.0,-2e+01,12.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bossds1.Tone` "Tone" [Float S; 0.7,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 17. `bmp` — "BigMuffPi" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/bmp.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bmp.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bmp.Tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bmp.Sustain` "Sustain" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 18. `IR` — "ImpulseResponse" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/impulseresponse.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False pack_type=end>
    - GxSelector `IR.auto_freq` "auto freq" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True padding=8>
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `IR.freq` "Freq" [Float S; 4.4e+02,2e+01,1.2e+04,1e+01] dica:"frequency (Hz)" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox3` <expand=True fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `IR.peak` "Peak" [Float S; 1.0,0.0,1e+01,0.2] dica:"peak gain" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <expand=True fill=True>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `IR.bandwidth` "Bandwidth" [Float S; 1e+02,2e+01,2e+04,1e+01] dica:"bandwidth (Hz)" label_ref=`label3:rack_label` <expand=False fill=False>
    - Label `label1` "             " xalign=0.5 <expand=True fill=True>

#### 19. `compressor` — "Compressor" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/faust-generated/compressor.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `compressor.ratio` "Ratio" [Float S; 2.0,1.0,2e+01,0.1] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=8 `vbox8` <expand=True fill=True>
      - GtkBox H sp=10 `hbox2` <expand=True fill=True>
        - GtkBox V sp=0 `vbox3` <expand=False fill=False>
          - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `compressor.threshold` "Threshold" [Float S; -2e+01,-96.0,1e+01,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox4` <expand=False fill=False>
          - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `compressor.knee` "Knee" [Float S; 3.0,0.0,2e+01,0.1] label_ref=`label3:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox5` <expand=False fill=False>
          - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `compressor.attack` "Attack" [Float S; 0.002,0.0,1.0,0.001] label_ref=`label4:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox6` <expand=False fill=False>
          - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `compressor.release` "Release" [Float S; 0.5,0.0,1e+01,0.01] label_ref=`label5:rack_label` <expand=False fill=False>
      - GxFastMeter `compressor.v1` "V1" [Float SOLN; 0,0.0,4e+01,0] orient=horizontal dimen=3 falloff=F

#### 20. `expander` — "Expander" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/faust-generated/expander.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `expander.ratio` "Ratio" [Float S; 2.0,1.0,2e+01,0.1] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=8 `vbox8` <expand=True fill=True>
      - GtkBox H sp=10 `hbox2` <expand=True fill=True>
        - GtkBox V sp=0 `vbox3` <expand=False fill=False>
          - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `expander.threshold` "Threshold" [Float S; -4e+01,-96.0,1e+01,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox4` <expand=False fill=False>
          - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `expander.knee` "Knee" [Float S; 3.0,0.0,2e+01,0.1] label_ref=`label3:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox5` <expand=False fill=False>
          - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `expander.attack` "Attack" [Float S; 0.001,0.0,1.0,0.001] label_ref=`label4:rack_label` <expand=False fill=False>
        - GtkBox V sp=0 `vbox6` <expand=False fill=False>
          - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
          - GxSmallKnobR `expander.release` "Release" [Float S; 0.1,0.0,1e+01,0.01] label_ref=`label5:rack_label` <expand=False fill=False>
      - GxFastMeter `expander.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=horizontal dimen=3 falloff=F

#### 21. `overdrive` — "Overdrive" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/faust-generated/overdrive.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `overdrive.drive` "Drive" [Float S; 1.0,1.0,2e+01,0.1] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `overdrive.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label2:rack_label` <expand=False fill=False>

#### 22. `echo` — "Echo" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/echo.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `echo.bpm` "BPM" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Echo in Beats per Minute" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `echo.percent` "Percent" [Float S; 0.0f,0.0f,1e+02f,0.1f] label_ref=`label2:rack_label` <expand=False fill=False>

#### 23. `delay` — "Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/delay.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `delay.bpm` "BPM" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Delay in Beats per Minute" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `delay.gain` "Gain" [Float S; 0.0f,-2e+01f,2e+01f,0.1f] label_ref=`label2:rack_label` <expand=False fill=False>

#### 24. `freeverb` — "Freeverb" (categoria Guitarix: Reverb)

Glade inline (`glade_def`) em `gx/faust-generated/freeverb.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `freeverb.damp` "HF Damp" [Float S; 0.5,0.0,1.0,0.025] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `freeverb.RoomSize` "Room Size" [Float S; 0.5,0.0,1.0,0.025] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `freeverb.wet_dry` "Wet/Dry" [Float S; 5e+01,0.0,1e+02,1.0] label_ref=`label3:rack_label` <expand=False fill=False>

#### 25. `biquad` — "BiQuad Filter" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/biquad.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `biquad.Freq` "Frequency" [Float S; 1.2e+03,3e+02,3e+03,5.0] label_ref=`label1:rack_label` <expand=False fill=False>

#### 26. `tremolo` — "Tremolo" (categoria Guitarix: Modulation)

Glade: `gx/builder/tremolo_ui.glade` raiz `rackbox` ; código: `gx/faust-generated/tremolo.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox3` <expand=True fill=False>
    - GxSelector `tremolo.SINE` "SINE" [Float B; 0.0,0.0,1.0,1.0] <expand=False fill=False padding=8>
    - GtkBox V sp=5 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "Frequency" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `tremolo.freq` "Freq" [Float S; 5.0,0.1,5e+01,0.1] <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=True>
      - GxMidKnob `tremolo.depth` "Depth" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=True>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "Dry/Wet" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `tremolo.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" <expand=False fill=True>
    - Label `label1` "              " xalign=0.5 <expand=True fill=True>

#### 27. `phaser_mono` — "Phaser Mono" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/phaser_mono.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `phaser_mono.level` "Level" [Float S; 0.0f,-6e+01f,1e+01f,0.1f] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `phaser_mono.lfobpm` "Speed (bpm)" [Float S; 3e+01f,24.0f,3.6e+02f,1.0f] dica:"Speed in Beats per Minute" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `phaser_mono.wet_dry` "Dry/Wet" [Float S; 1e+02f,0.0f,1e+02f,1.0f] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 28. `chorus_mono` — "Chorus Mono" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/chorus_mono.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `chorus_mono.freq` "Freq" [Float S; 2.0f,0.0f,1e+01f,0.01f] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `chorus_mono.level` "Level" [Float S; 0.5f,0.0f,1.0f,0.01f] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `chorus_mono.wet_dry` "Dry/Wet" [Float S; 1e+02f,0.0f,1e+02f,1.0f] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 29. `flanger_mono` — "Flanger Mono" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/flanger_mono.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono.lfobpm` "BPM" [Float S; 24.0,24.0,3.6e+02,1.0] dica:"LFO in Beats per Minute" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `flanger_mono.level` "Level" [Float S; 0.0,-6e+01,1e+01,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 30. `flanger_mono_gx` — "Flanger GX" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/plugins/flanger_gx.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=4 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.freq` "Speed" [Float SL; 0.2,0.05,1e+01,1.06] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.depth` "Depth" [Float S; 0.5,0.0,5.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.width` "Width" [Float S; 5.0,0.0,1e+01,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.feedback` "Feedback" [Float S; -0.707,-0.99,0.99,0.01] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.mix` "Mix" [Float S; 0.0,-1.0,1.0,0.1] label_ref=`label5:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox7` <expand=False fill=False>
      - Label `label6:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `flanger_mono_gx.wet` "Wet" [Float S; 1e+02,0.0,1e+02,1.0] label_ref=`label6:rack_label` <expand=False fill=False>

#### 31. `feedback` — "Feedback" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/gx_feedback.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `feedback.feedback` "Feedback" [Float S; 0.0,-1.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `feedback.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] label_ref=`label2:rack_label` <expand=False fill=False>

#### 32. `eq` — "Peak EQ" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/peak_eq.cc`

- GtkBox H sp=0 `rackbox`
  - GtkGrid `table1` colsp=14 rowsp=3 homog=F/F <expand=True fill=False>
    - GxSmallKnob `eq.bandwidth1` "Q" [Float SL; 41.0,5.0,2e+04,1.01] dica:"bandwidth (hz)" label_ref=`label1:rack_label_inverse` <left_attach=0 top_attach=3>
    - GxSmallKnob `eq.bandwidth2` "Q" [Float SL; 2.2e+02,5.0,2e+04,1.01] dica:"bandwidth (hz)" label_ref=`label2:rack_label_inverse` <left_attach=1 top_attach=3>
    - GxSmallKnob `eq.bandwidth3` "Q" [Float SL; 8.8e+02,5.0,2e+04,1.01] dica:"bandwidth (hz)" label_ref=`label3:rack_label_inverse` <left_attach=2 top_attach=3>
    - GxSmallKnob `eq.bandwidth4` "Q" [Float SL; 1.76e+03,5.0,2e+04,1.01] dica:"bandwidth (hz)" label_ref=`label4:rack_label_inverse` <left_attach=3 top_attach=3>
    - GxSmallKnobR `eq.level1` "Sub" [Float S; 0.0,-5e+01,5e+01,0.1] dica:"gain (dB)" label_ref=`label5:rack_label` <left_attach=0 top_attach=2>
    - GxSmallKnobR `eq.level2` "Low" [Float S; 0.0,-5e+01,5e+01,0.1] dica:"gain (dB)" label_ref=`label6:rack_label` <left_attach=1 top_attach=2>
    - GxSmallKnobR `eq.level3` "Mid" [Float S; 0.0,-5e+01,5e+01,0.1] dica:"gain (dB)" label_ref=`label7:rack_label` <left_attach=2 top_attach=2>
    - GxSmallKnobR `eq.level4` "High" [Float S; 0.0,-5e+01,5e+01,0.1] dica:"gain (dB)" label_ref=`label8:rack_label` <left_attach=3 top_attach=2>
    - GxValueDisplay `eq.peak1` "Peak1" [Float SL; 1.1e+02,2e+01,2.2e+04,1.01] dica:"frequency (hz)" round_digits=0 <left_attach=0 top_attach=0>
    - GxValueDisplay `eq.peak2` "Peak2" [Float SL; 4.4e+02,2e+01,2.2e+04,1.01] dica:"frequency (hz)" round_digits=0 <left_attach=1 top_attach=0>
    - GxValueDisplay `eq.peak3` "Peak3" [Float SL; 1.76e+03,2e+01,2.2e+04,1.01] dica:"frequency (hz)" round_digits=0 <left_attach=2 top_attach=0>
    - GxValueDisplay `eq.peak4` "Peak4" [Float SL; 3.52e+03,2e+01,2.2e+04,1.01] dica:"frequency (hz)" round_digits=0 <left_attach=3 top_attach=0>
    - Label `label1:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <left_attach=0 top_attach=4>
    - Label `label2:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <left_attach=1 top_attach=4>
    - Label `label3:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <left_attach=2 top_attach=4>
    - Label `label4:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <left_attach=3 top_attach=4>
    - Label `label5:rack_label` "label" .rack_label xalign=0.5 <left_attach=0 top_attach=1>
    - Label `label6:rack_label` "label" .rack_label xalign=0.5 <left_attach=1 top_attach=1>
    - Label `label7:rack_label` "label" .rack_label xalign=0.5 <left_attach=2 top_attach=1>
    - Label `label8:rack_label` "label" .rack_label xalign=0.5 <left_attach=3 top_attach=1>

#### 33. `dide` — "Digital Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/digital_delay.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox V sp=0 `vbox2` <expand=True fill=True>
    - Label `label1` "" xalign=0.5 <expand=False fill=False>
    - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `dide.bpm` "BPM" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Delay in Beats per Minute" label_ref=`label1:rack_label` <expand=True fill=True>
    - Label `label2` "" xalign=0.5 <expand=False fill=False>
  - GtkBox V sp=10 `vbox8` <expand=True fill=True>
    - GtkBox H sp=0 `hbox1` <expand=True fill=True>
      - GtkBox V sp=0 `vbox3` <expand=True fill=True>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `dide.feedback` "Feedback" [Float S; 5e+01f,1.0f,1e+02f,1.0f] dica:"Percentage of the feedback level in the de.delay loop" label_ref=`label2:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox4` <expand=True fill=True>
        - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `dide.level` "Level" [Float S; 5e+01f,1.0f,1e+02f,1.0f] dica:"Percentage of the de.delay gain level" label_ref=`label3:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox5` <expand=True fill=True>
        - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `dide.highpass` "Hipass" [Float S; 1.2e+02f,2e+01f,2e+04f,1.0f] dica:"Highpass filter frequency in the feedback loop" label_ref=`label4:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox6` <expand=True fill=True>
        - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `dide.howpass` "Lopass" [Float S; 1.2e+04f,2e+01f,2e+04f,1.0f] dica:"Lowpass filter frequency in the feedback loop" label_ref=`label5:rack_label` <expand=False fill=False>
    - GtkGrid `table1` colsp=6 rowsp=3 homog=F/F <expand=False fill=False>
      - GxSelector `dide.mode` "Mode" [Float S; 0.0f,0.0f,3.0f,1.0f] label_ref=`label21:rack_label_inverse` {hexpand=True} <left_attach=1 top_attach=1>
      - Label `label11:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=0 top_attach=0>
      - Label `label21:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=0 top_attach=1>
      - GxSelector `dide.notes` "Notes" [Float S; 4.0f,0.0f,17.0f,1.0f] dica:"Note setting for bpm" label_ref=`label11:rack_label_inverse` {hexpand=True} <left_attach=1 top_attach=0 width=3>
      - Label `label31:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=2 top_attach=1>
      - GxSwitch `dide.Freeze` "Freeze" [Float B; 0.0,0.0,1.0,1.0] dica:"Freeze the current delay" base_name=frbutton label_ref=`label31:rack_label_inverse` <left_attach=3 top_attach=1>
  - GtkBox V sp=0 `vbox7` <expand=True fill=True>
    - Label `label6:rack_label1` "" .rack_label1 xalign=0.5 <expand=False fill=False>
    - Label `label6:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `dide.gain` "Gain" [Float S; 1e+02f,0.0f,1.2e+02f,1.0f] dica:"Overall gain of the de.delay line in percent" label_ref=`label6:rack_label` <expand=True fill=True>
    - Label `label6:rack_label2` "" .rack_label2 xalign=0.5 <expand=False fill=False>

#### 34. `amp.tonestack` — "Tonestack" (categoria Guitarix: Tone Control)

Glade: `gx/builder/amp.tonestack_ui.glade` raiz `rackbox`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False>
    - GxSelector `amp.tonestack.select` "select" [Int S; 0,0,0 (enum: 27 tonestacks)] <expand=True fill=False padding=8>
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `amp.tonestack.Bass` "Bass" [Float SA; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=True>
    - GtkBox V sp=0 `vbox3` <expand=True fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `amp.tonestack.Middle` "Middle" [Float SA; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=True>
    - GtkBox V sp=0 `vbox4` <expand=True fill=True>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `amp.tonestack.Treble` "Treble" [Float SA; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=True>
    - Label `label1` "           " xalign=0.5 <expand=True fill=True>

#### 35. `cab` — "Cabinet" (categoria Guitarix: ?)

Glade: `gx/builder/cabinet_ui.glade` raiz `rackbox`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False>
    - GxSelector `cab.select` "select" [Int B; 0,0,0 (enum: lista de cabinets)] <expand=True fill=False padding=8>
    - GtkBox V sp=5 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `cab.bass` "Bass" [Float SA; 0.0,-1e+01,1e+01,0.5] dica:"Bass" label_ref=`label1:rack_label` <expand=False fill=True>
    - GtkBox V sp=5 `vbox3` <expand=True fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `cab.treble` "Treble" [Float SA; 0.0,-1e+01,1e+01,0.5] dica:"Treble" label_ref=`label2:rack_label` <expand=False fill=True>
    - GtkBox V sp=0 `vbox4` <expand=True fill=True>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxMidKnob `cab.Level` "Level" [Float SA; 1.0,0.5,5.0,0.5] dica:"Level" label_ref=`label3:rack_label` <expand=False fill=True>
    - Label `label1` "           " xalign=0.5 <expand=True fill=True>

#### 36. `pre` — "Amp Impulse" (categoria Guitarix: ?)

Glade: `gx/builder/ampimpulse_ui.glade` raiz `rackbox`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False>
    - GxSelector `pre.select` "select" [Int B; 0,0,0 (enum: lista de preamps)] <expand=True fill=False padding=8>
    - GtkBox V sp=5 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `pre.bass` "Bass" [Float SA; 0.0,-1e+01,1e+01,0.5] dica:"Bass" label_ref=`label1:rack_label` <expand=False fill=True>
    - GtkBox V sp=5 `vbox3` <expand=True fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxSmallKnobR `pre.treble` "Treble" [Float SA; 0.0,-1e+01,1e+01,0.5] dica:"Treble" label_ref=`label2:rack_label` <expand=False fill=True>
    - GtkBox V sp=0 `vbox4` <expand=True fill=True>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
      - GxMidKnob `pre.Level` "Level" [Float SA; 1.0,0.1,2.1,0.1] dica:"Level" label_ref=`label3:rack_label` <expand=False fill=True>
    - Label `label1` "               " xalign=0.5 <expand=True fill=True>

#### 37. `abgate` — "abGate" (categoria Guitarix: Misc)

Glade: `gx/builder/abgate_ui.glade` raiz `rackbox` ; código: `gx/plugins/abgate.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `abgate.threshold` "Threshold" [Float S; -70,-70,12,1] dica:"threshold (dB) value\nput it just above the noise level" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `abgate.attack` "Attack" [Float S; 4,0.1,500,0.1] dica:"time (ms) until gate opens (sensitivity)" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `abgate.hold` "Hold" [Float S; 5,5,3000,0.1] dica:"time (ms) to keep open when input is below threshold" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `abgate.decay` "Decay" [Float S; 50,5,4000,0.1] dica:"duration (ms) of shutting gate down" label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `abgate.gaterange` "Range" [Float S; -30,-90,-20,1] dica:"damping (dB) when the gate is closed" label_ref=`label5:rack_label` <expand=False fill=False>

#### 38. `ring_modulator` — "Ring Modulator Mono" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/ring_modulator.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ring_modulator.freq` "Freq" [Float S; 2.4e+02,1.2e+02,1.6e+03,0.5] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ring_modulator.dry/wet` "Mix" [Float S; 0.5,0.0,1.0,0.05] label_ref=`label2:rack_label` <expand=False fill=False>

#### 39. `graphiceq` — "Graphic EQ" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/graphiceq.cc`

- GxPaintBox H sp=4 paint=box_uni_2_expose `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=4 `vbox2` .metered-slider <expand=True fill=True>
      - Label `label1:rack_label_inverse` "< 31" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox2` <expand=True fill=True>
        - GxFastMeter `graphiceq.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g1` "G1" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) below 31.25 Hz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox3` .metered-slider <expand=True fill=True>
      - Label `label2:rack_label_inverse` "62" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox3` <expand=True fill=True>
        - GxFastMeter `graphiceq.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g2` "G2" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 62.5 Hz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox4` .metered-slider <expand=True fill=True>
      - Label `label3:rack_label_inverse` "125" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox4` <expand=True fill=True>
        - GxFastMeter `graphiceq.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g3` "G3" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 125 Hz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox5` .metered-slider <expand=True fill=True>
      - Label `label4:rack_label_inverse` "250" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox5` <expand=True fill=True>
        - GxFastMeter `graphiceq.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g4` "G4" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 250 Hz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox6` .metered-slider <expand=True fill=True>
      - Label `label5:rack_label_inverse` "500" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox6` <expand=True fill=True>
        - GxFastMeter `graphiceq.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g5` "G5" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 500 Hz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox7` .metered-slider <expand=True fill=True>
      - Label `label6:rack_label_inverse` "1k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox7` <expand=True fill=True>
        - GxFastMeter `graphiceq.v6` "V6" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g6` "G6" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 1 kHz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox8` .metered-slider <expand=True fill=True>
      - Label `label7:rack_label_inverse` "2k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox8` <expand=True fill=True>
        - GxFastMeter `graphiceq.v7` "V7" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g7` "G7" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 2 kHz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox9` .metered-slider <expand=True fill=True>
      - Label `label8:rack_label_inverse` "4k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox9` <expand=True fill=True>
        - GxFastMeter `graphiceq.v8` "V8" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g8` "G8" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 4 kHz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox10` .metered-slider <expand=True fill=True>
      - Label `label9:rack_label_inverse` "8k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox10` <expand=True fill=True>
        - GxFastMeter `graphiceq.v9` "V9" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g9` "G9" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 8 kHz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox11` .metered-slider <expand=True fill=True>
      - Label `label10:rack_label_inverse` "16k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox11` <expand=True fill=True>
        - GxFastMeter `graphiceq.v10` "V10" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g10` "G10" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) at 16 kHz" <expand=True fill=True>
    - GtkBox V sp=4 `vbox12` .metered-slider <expand=True fill=True>
      - Label `label11:rack_label_inverse` "> 16k" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GtkBox H sp=0 `hbox12` <expand=True fill=True>
        - GxFastMeter `graphiceq.v11` "V11" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True {halign=end} <expand=True fill=True>
        - GxVSlider `graphiceq.g11` "G11" [Float S; 0.0,-3e+01,2e+01,0.1] dica:"gain (dB) above 16 kHz" <expand=True fill=True>

#### 40. `univibe_mono` — "Vibe Mono" (categoria Guitarix: Modulation)

Glade: `gx/builder/vibe_ui.glade` raiz `rackbox` ; código: `gx/plugins/vibe.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=5 `vbox21` <expand=False fill=False>
      - Label `label11:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `univibe_mono.freq` "Tempo" [Float SA; 4.4,0.1,1e+01,0.1] dica:"LFO frequency (Hz)" label_ref=`label11:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `univibe_mono.width` "Width" [Float S; (ver vibe.cc)] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `univibe_mono.depth` "Depth" [Float S; (ver vibe.cc)] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `univibe_mono.fb` "F/B" [Float S; (ver vibe.cc)] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `univibe_mono.wet_dry` "Wet/Dry" [Float S; (ver vibe.cc)] label_ref=`label4:rack_label` <expand=False fill=False>

#### 41. `mbc` — "Multi Band Compressor" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/plugins/mbc.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=8 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=True fill=True>
      - GtkBox H sp=0 `hbox2`
        - GtkBox V sp=4 `vbox3` <expand=False fill=True>
          - GtkBox H sp=15 `hbox4` <expand=True fill=True>
            - GtkBox V sp=0 `vbox4` <expand=True fill=True>
              - Label `label_1:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Ratio1` "Ratio1" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox5` <expand=True fill=True>
              - Label `label_2:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup1` "Makeup1" [Float S; 13.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox6` <expand=True fill=True>
              - Label `label_3:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup-Threshold1` "Makeup-Threshold1" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbc.Mode1` "Mode1" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table1` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbc.Attack1` "Attack1" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbc.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_4:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_7:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_5:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbc.Release1` "Release1" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 1")
      - GtkBox H sp=0 `hbox5`
        - GtkBox V sp=4 `vbox7` <expand=False fill=True>
          - GtkBox H sp=15 `hbox6` <expand=True fill=True>
            - GtkBox V sp=0 `vbox8` <expand=True fill=True>
              - Label `label_8:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Ratio2` "Ratio2" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox9` <expand=True fill=True>
              - Label `label_9:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup2` "Makeup2" [Float S; 1e+01,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox10` <expand=True fill=True>
              - Label `label_10:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup-Threshold2` "Makeup-Threshold2" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbc.Mode2` "Mode2" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table2` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbc.Attack2` "Attack2" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbc.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_11:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_14:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_13:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_12:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbc.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbc.Release2` "Release2" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=0 `hbox7`
        - GtkBox V sp=4 `vbox11` <expand=False fill=True>
          - GtkBox H sp=15 `hbox8` <expand=True fill=True>
            - GtkBox V sp=0 `vbox12` <expand=True fill=True>
              - Label `label_15:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Ratio3` "Ratio3" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox13` <expand=True fill=True>
              - Label `label_16:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup3` "Makeup3" [Float S; 4.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox14` <expand=True fill=True>
              - Label `label_17:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup-Threshold3` "Makeup-Threshold3" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbc.Mode3` "Mode3" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table3` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbc.Attack3` "Attack3" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbc.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_18:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_21:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_20:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_19:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbc.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbc.Release3` "Release3" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=0 `hbox9`
        - GtkBox V sp=4 `vbox15` <expand=False fill=True>
          - GtkBox H sp=15 `hbox10` <expand=True fill=True>
            - GtkBox V sp=0 `vbox16` <expand=True fill=True>
              - Label `label_22:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Ratio4` "Ratio4" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox17` <expand=True fill=True>
              - Label `label_23:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup4` "Makeup4" [Float S; 8.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox18` <expand=True fill=True>
              - Label `label_24:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup-Threshold4` "Makeup-Threshold4" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbc.Mode4` "Mode4" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table4` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbc.Attack4` "Attack4" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbc.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_25:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_28:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_27:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_26:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbc.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbc.Release4` "Release4" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=0 `hbox11`
        - GtkBox V sp=4 `vbox19` <expand=False fill=True>
          - GtkBox H sp=15 `hbox12` <expand=True fill=True>
            - GtkBox V sp=0 `vbox20` <expand=True fill=True>
              - Label `label_29:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Ratio5` "Ratio5" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox21` <expand=True fill=True>
              - Label `label_30:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup5` "Makeup5" [Float S; 11.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox22` <expand=True fill=True>
              - Label `label_31:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbc.Makeup-Threshold5` "Makeup-Threshold5" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbc.Mode5` "Mode5" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table5` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbc.Attack5` "Attack5" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - Label `label_32:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_34:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_33:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbc.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbc.Release5` "Release5" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
          - Label `label_35:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox2` {margin_bottom=4} <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0 <expand=False fill=False>
      - GtkBox H sp=0 `hbox3` <expand=True fill=True>
        - GxFastMeter `mbc.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbc.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbc.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbc.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbc.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 42. `mbd` — "MultiBand Distortion" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/mbd.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=False fill=True>
      - GtkBox H sp=4 `hbox2` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>
          - GtkBox V sp=0 `vbox3` <expand=True fill=True>
            - Label `label_81:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Drive1` "Drive1" [Float S; 0.0,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 `vbox4` <expand=True fill=True>
            - Label `label_91:rack_label` "Offset" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Offset1` "Offset1" [Float S; 0.0,0.0,0.5,0.01] dica:"Brings in even harmonics" <expand=False fill=False>
          - GtkBox V sp=0 `vbox6` <expand=False fill=True>
            - Label `label_10:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbd.Gain1` "Gain1" [Float S; 0.0,-25.0,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table1` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_141:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbd.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=1>
      - (aba: "Band 1")
      - GtkBox H sp=4 `hbox5` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=0 `vbox8` <expand=True fill=True>
            - Label `label_8:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Drive2` "Drive2" [Float S; 0.0,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 `vbox9` <expand=True fill=True>
            - Label `label_9:rack_label` "Offset" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Offset2` "Offset2" [Float S; 0.0,0.0,0.5,0.01] dica:"Brings in even harmonics" <expand=False fill=False>
          - GtkBox V sp=0 `vbox10` <expand=False fill=True>
            - Label `label_11:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbd.Gain2` "Gain2" [Float S; 0.0,-25.0,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table2` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_13:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbd.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_14:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbd.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=4 `hbox7` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox8` <expand=True fill=True>
          - GtkBox V sp=0 `vbox7` <expand=True fill=True>
            - Label `label_82:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Drive3` "Drive3" [Float S; 0.0,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 `vbox11` <expand=True fill=True>
            - Label `label_92:rack_label` "Offset" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Offset3` "Offset3" [Float S; 0.0,0.0,0.5,0.01] dica:"Brings in even harmonics" <expand=False fill=False>
          - GtkBox V sp=0 `vbox12` <expand=False fill=True>
            - Label `label_12:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbd.Gain3` "Gain3" [Float S; 0.0,-25.0,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table3` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_132:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbd.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_142:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbd.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=4 `hbox9` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox10` <expand=True fill=True>
          - GtkBox V sp=0 `vbox14` <expand=True fill=True>
            - Label `label_83:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Drive4` "Drive4" [Float S; 0.0,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 `vbox15` <expand=True fill=True>
            - Label `label_93:rack_label` "Offset" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Offset4` "Offset4" [Float S; 0.0,0.0,0.5,0.01] dica:"Brings in even harmonics" <expand=False fill=False>
          - GtkBox V sp=0 `vbox13` <expand=False fill=True>
            - Label `label_1:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbd.Gain4` "Gain4" [Float S; 0.0,-25.0,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table4` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_133:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbd.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_143:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbd.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=4 `hbox11` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox12` <expand=True fill=True>
          - GtkBox V sp=0 `vbox18` <expand=True fill=True>
            - Label `label_84:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Drive5` "Drive5" [Float S; 0.0,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 `vbox19` <expand=True fill=True>
            - Label `label_94:rack_label` "Offset" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbd.Offset5` "Offset5" [Float S; 0.0,0.0,0.5,0.01] dica:"Brings in even harmonics" <expand=False fill=False>
          - GtkBox V sp=0 `vbox16` <expand=False fill=True>
            - Label `label_15:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbd.Gain5` "Gain5" [Float S; 0.0,-25.0,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table5` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_134:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbd.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=0>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox5` <expand=True fill=True>
      - Label `label_99:rack_label` "Gain" .rack_label xalign=0.5 <expand=True fill=True>
      - GxMidKnob `mbd.Gain` "Gain" [Float S; 0.0,-4e+01,4.0,0.1] <expand=True fill=True>
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0.5 <expand=False fill=False>
      - GtkBox H sp=0 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbd.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbd.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbd.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbd.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbd.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 43. `mbe` — "MultiBand Echo" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/plugins/mbe.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=True fill=True>
      - GtkBox H sp=4 `hbox2` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>
          - GtkBox V sp=0 `vbox3` <expand=True fill=True>
            - Label `label_81:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.time1` "Time1" [Float S; 3e+01,24.0,3.6e+02,1.0] dica:"Echo in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox4` <expand=True fill=True>
            - Label `label_91:rack_label` "Amount" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.percent1` "Percent1" [Float S; 1e+01,0.0,1e+02,0.1] <expand=False fill=False>
        - GtkGrid `table1` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_141:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbe.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=1>
      - (aba: "Band 1")
      - GtkBox H sp=4 `hbox5` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=0 `vbox8` <expand=True fill=True>
            - Label `label_8:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.time2` "Time2" [Float S; 6e+01,24.0,3.6e+02,1.0] dica:"Echo in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox9` <expand=True fill=True>
            - Label `label_9:rack_label` "Amount" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.percent2` "Percent2" [Float S; 3e+01,0.0,1e+02,0.1] <expand=False fill=False>
        - GtkGrid `table2` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_13:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbe.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_14:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbe.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=4 `hbox7` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox8` <expand=True fill=True>
          - GtkBox V sp=0 `vbox7` <expand=True fill=True>
            - Label `label_82:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.time3` "Time3" [Float S; 1.2e+02,24.0,3.6e+02,1.0] dica:"Echo in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox11` <expand=True fill=True>
            - Label `label_92:rack_label` "Amount" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.percent3` "Percent3" [Float S; 45.0,0.0,1e+02,0.1] <expand=False fill=False>
        - GtkGrid `table3` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_132:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbe.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_142:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbe.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=4 `hbox9` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox10` <expand=True fill=True>
          - GtkBox V sp=0 `vbox14` <expand=True fill=True>
            - Label `label_83:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.time4` "Time4" [Float S; 1.5e+02,24.0,3.6e+02,1.0] dica:"Echo in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox15` <expand=True fill=True>
            - Label `label_93:rack_label` "Amount" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.percent4` "Percent4" [Float S; 2e+01,0.0,1e+02,0.1] <expand=False fill=False>
        - GtkGrid `table4` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_133:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbe.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_143:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbe.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=4 `hbox11` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox12` <expand=True fill=True>
          - GtkBox V sp=0 `vbox18` <expand=True fill=True>
            - Label `label_84:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.time5` "Time5" [Float S; 2.4e+02,24.0,3.6e+02,1.0] dica:"Echo in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox19` <expand=True fill=True>
            - Label `label_94:rack_label` "Amount" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbe.percent5` "Percent5" [Float S; 0.0,0.0,1e+02,0.1] <expand=False fill=False>
        - GtkGrid `table5` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_134:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbe.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=0>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox2` {halign=center} <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0 <expand=False fill=False>
      - GtkBox H sp=10 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbe.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbe.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbe.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbe.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbe.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 44. `mbdel` — "MultiBand Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/plugins/mbdel.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=True fill=True>
      - GtkBox H sp=4 `hbox2` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>
          - GtkBox V sp=0 `vbox3` <expand=True fill=True>
            - Label `label_81:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.delay1` "Delay1" [Float S; 3e+01,24.0,3.6e+02,1.0] dica:"Delay in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox4` <expand=True fill=True>
            - Label `label_91:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.gain1` "Gain1" [Float S; -1e+01,-2e+01,2e+01,0.1] <expand=False fill=False>
        - GtkGrid `table1` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_141:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbdel.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=1>
      - (aba: "Band 1")
      - GtkBox H sp=4 `hbox5` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=0 `vbox8` <expand=True fill=True>
            - Label `label_8:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.delay2` "Delay2" [Float S; 6e+01,24.0,3.6e+02,1.0] dica:"Delay in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox9` <expand=True fill=True>
            - Label `label_9:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.gain2` "Gain2" [Float S; -5.0,-2e+01,2e+01,0.1] <expand=False fill=False>
        - GtkGrid `table2` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_13:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbdel.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_14:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbdel.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=4 `hbox7` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox8` <expand=True fill=True>
          - GtkBox V sp=0 `vbox7` <expand=True fill=True>
            - Label `label_82:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.delay3` "Delay3" [Float S; 9e+01,24.0,3.6e+02,1.0] dica:"Delay in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox11` <expand=True fill=True>
            - Label `label_92:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.gain3` "Gain3" [Float S; -2.0,-2e+01,2e+01,0.1] <expand=False fill=False>
        - GtkGrid `table3` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_132:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbdel.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_142:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbdel.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=4 `hbox9` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox10` <expand=True fill=True>
          - GtkBox V sp=0 `vbox14` <expand=True fill=True>
            - Label `label_83:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.delay4` "Delay4" [Float S; 1.2e+02,24.0,3.6e+02,1.0] dica:"Delay in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox15` <expand=True fill=True>
            - Label `label_93:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.gain4` "Gain4" [Float S; 0.0,-2e+01,2e+01,0.1] <expand=False fill=False>
        - GtkGrid `table4` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_133:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbdel.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_143:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbdel.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=4 `hbox11` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox12` <expand=True fill=True>
          - GtkBox V sp=0 `vbox18` <expand=True fill=True>
            - Label `label_84:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.delay5` "Delay5" [Float S; 1.5e+02,24.0,3.6e+02,1.0] dica:"Delay in Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox19` <expand=True fill=True>
            - Label `label_94:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbdel.gain5` "Gain5" [Float S; -1e+01,-2e+01,2e+01,0.1] <expand=False fill=False>
        - GtkGrid `table5` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_134:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbdel.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=0>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox2` {halign=center} <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0 <expand=False fill=False>
      - GtkBox H sp=10 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbdel.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbdel.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbdel.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbdel.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbdel.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 45. `mbclip` — "MultiBand Clipper" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/mbclipper.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=False fill=True>
      - GtkBox H sp=4 `hbox2` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>
          - GtkBox V sp=0 `vbox3` <expand=True fill=True>
            - Label `label_81:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbclip.Drive1` "Drive1" [Float S; 0.33,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 (sem id) <expand=False fill=True>
            - Label `label_9:rack_label` "Gain" .rack_label xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbclip.Gain1` "Gain1" [Float S; 0.0,-4e+01,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table1` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4} <expand=True fill=True>
          - Label `label_141:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbclip.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=1>
      - (aba: "Band 1")
      - GtkBox H sp=4 `hbox5` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=0 `vbox8` <expand=True fill=True>
            - Label `label_8:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbclip.Drive2` "Drive2" [Float S; 0.5,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 (sem id) <expand=False fill=True>
            - Label `label_9:rack_label1` "Gain" .rack_label1 xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbclip.Gain2` "Gain2" [Float S; 0.0,-4e+01,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table2` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4} <expand=True fill=True>
          - Label `label_13:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbclip.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_14:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbclip.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=4 `hbox7` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox8` <expand=True fill=True>
          - GtkBox V sp=0 `vbox7` <expand=True fill=True>
            - Label `label_82:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbclip.Drive3` "Drive3" [Float S; 0.65,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 (sem id) <expand=False fill=True>
            - Label `label_9:rack_label2` "Gain" .rack_label2 xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbclip.Gain3` "Gain3" [Float S; 0.0,-4e+01,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table3` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4} <expand=True fill=True>
          - Label `label_132:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbclip.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_142:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbclip.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=4 `hbox9` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox10` <expand=True fill=True>
          - GtkBox V sp=0 `vbox14` <expand=True fill=True>
            - Label `label_83:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbclip.Drive4` "Drive4" [Float S; 0.33,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 (sem id) <expand=False fill=True>
            - Label `label_9:rack_label3` "Gain" .rack_label3 xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbclip.Gain4` "Gain4" [Float S; 0.0,-4e+01,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table4` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4} <expand=True fill=True>
          - Label `label_133:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbclip.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_143:rack_label` "High" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbclip.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=4 `hbox11` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox12` <expand=True fill=True>
          - GtkBox V sp=0 `vbox18` <expand=True fill=True>
            - Label `label_84:rack_label` "Drive" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbclip.Drive5` "Drive5" [Float S; 0.1,0.0,1.0,0.01] dica:"Amount of distortion" <expand=False fill=False>
          - GtkBox V sp=0 (sem id) <expand=False fill=True>
            - Label `label_9:rack_label4` "Gain" .rack_label4 xalign=0.5 <expand=False fill=True>
            - GxSmallKnobR `mbclip.Gain5` "Gain5" [Float S; 0.0,-4e+01,4.0,0.1] <expand=False fill=True>
        - GtkGrid `table5` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4} <expand=True fill=True>
          - Label `label_134:rack_label` "Low" .rack_label xalign=0.5 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbclip.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=0>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox5` <expand=True fill=True>
      - Label `label_99:rack_label` "Gain" .rack_label xalign=0.5 <expand=True fill=True>
      - GxMidKnob `mbclip.Gain` "Gain" [Float S; 0.0,-4e+01,4.0,0.1] <expand=True fill=True>
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0.5 <expand=False fill=False>
      - GtkBox H sp=0 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbclip.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbclip.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbclip.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbclip.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbclip.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 46. `bboom` — "Bass Boom" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/bassboom.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bboom.Drive` "Drive" [Float S; 0.33,0.0,1.0,0.01] dica:"Amount of SubBass Harmonics" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bboom.Gain` "Gain" [Float S; 0.0,-4e+01,4.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>

#### 47. `mbchor` — "Multi Band Chorus" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/plugins/mbchor.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=True fill=True>
      - GtkBox H sp=0 `hbox2` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>
          - GtkBox V sp=0 `vbox3` <expand=True fill=True>
            - Label `label_81:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.freq1` "Freq1" [Float S; 3e+01,24.0,3.6e+02,1.0] dica:"Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox4` <expand=True fill=True>
            - Label `label_91:rack_label` "Delay" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.delay1` "Delay1" [Float S; 0.02,0.0,0.2,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox5` <expand=True fill=True>
            - Label `label_102:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.depth1` "Depth1" [Float S; 0.02,0.01,1.0,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox6` <expand=True fill=True>
            - Label `label_103:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.level1` "Level1" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
        - GtkGrid `table1` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_141:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbchor.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=1>
      - (aba: "Band 1")
      - GtkBox H sp=0 `hbox5` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=0 `vbox8` <expand=True fill=True>
            - Label `label_8:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.freq2` "Freq2" [Float S; 6e+01,24.0,3.6e+02,1.0] dica:"Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox9` <expand=True fill=True>
            - Label `label_9:rack_label` "Delay" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.delay2` "Delay2" [Float S; 0.04,0.0,0.2,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox10` <expand=True fill=True>
            - Label `label_10:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.depth2` "Depth2" [Float S; 0.04,0.01,1.0,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox23` <expand=True fill=True>
            - Label `label_101:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.level2` "Level2" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
        - GtkGrid `table2` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_13:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbchor.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_14:rack_label` "High" .rack_label xalign=1 {hexpand=False} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbchor.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=0 `hbox7` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox8` <expand=True fill=True>
          - GtkBox V sp=0 `vbox7` <expand=True fill=True>
            - Label `label_82:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.freq3` "Freq3" [Float S; 9e+01,24.0,3.6e+02,1.0] dica:"Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox11` <expand=True fill=True>
            - Label `label_92:rack_label` "Delay" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.delay3` "Delay3" [Float S; 0.06,0.0,0.2,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox12` <expand=True fill=True>
            - Label `label_104:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.depth3` "Depth3" [Float S; 0.06,0.01,1.0,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox13` <expand=True fill=True>
            - Label `label_105:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.level3` "Level3" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
        - GtkGrid `table3` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_132:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbchor.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_142:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbchor.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=0 `hbox9` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox10` <expand=True fill=True>
          - GtkBox V sp=0 `vbox14` <expand=True fill=True>
            - Label `label_83:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.freq4` "Freq4" [Float S; 1.2e+02,24.0,3.6e+02,1.0] dica:"Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox15` <expand=True fill=True>
            - Label `label_93:rack_label` "Delay" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.delay4` "Delay4" [Float S; 0.08,0.0,0.2,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox16` <expand=True fill=True>
            - Label `label_106:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.depth4` "Depth4" [Float S; 0.08,0.01,1.0,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox17` <expand=True fill=True>
            - Label `label_107:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.level4` "Level4" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
        - GtkGrid `table4` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_133:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbchor.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label `label_143:rack_label` "High" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=0>
          - GxSmallKnob `mbchor.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=0 `hbox11` {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4}
        - GtkBox H sp=0 `hbox12` <expand=True fill=True>
          - GtkBox V sp=0 `vbox18` <expand=True fill=True>
            - Label `label_84:rack_label` "BPM" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.freq5` "Freq5" [Float S; 1.5e+02,24.0,3.6e+02,1.0] dica:"Beats per Minute" <expand=False fill=False>
          - GtkBox V sp=0 `vbox19` <expand=True fill=True>
            - Label `label_94:rack_label` "Delay" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.delay5` "Delay5" [Float S; 0.1,0.0,0.2,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox20` <expand=True fill=True>
            - Label `label_108:rack_label` "Depth" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.depth5` "Depth5" [Float S; 0.1,0.01,1.0,0.01] <expand=False fill=False>
          - GtkBox V sp=0 `vbox21` <expand=True fill=True>
            - Label `label_109:rack_label` "Level" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `mbchor.level5` "Level5" [Float S; 0.5,0.0,1.0,0.01] <expand=False fill=False>
        - GtkGrid `table5` colsp=4 rowsp=4 homog=F/True {margin_left=4 margin_right=4 margin_top=4 margin_bottom=4 halign=center} <expand=True fill=True>
          - Label `label_134:rack_label` "Low" .rack_label xalign=1 {hexpand=True} <left_attach=0 top_attach=1>
          - GxSmallKnob `mbchor.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - Label (sem id) "" xalign=0.5 <left_attach=0 top_attach=0>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox2` {halign=center} <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0 <expand=False fill=False>
      - GtkBox H sp=10 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbchor.v1` "V1" [Float SOLN; 0,0.0,1.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbchor.v2` "V2" [Float SOLN; 0,0.0,1.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbchor.v3` "V3" [Float SOLN; 0,0.0,1.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbchor.v4` "V4" [Float SOLN; 0,0.0,1.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>
        - GxFastMeter `mbchor.v5` "V5" [Float SOLN; 0,0.0,1.0,0] orient=vertical dimen=0 falloff=True <expand=True fill=True>

#### 48. `GCB_95` — "GCB 95" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/plugins/gcb_95.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `GCB_95.hotpotz` "Wah" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `GCB_95.Volume` "Volume" [Float S; 0.0,-2e+01,4.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>

#### 49. `duckDelay` — "Duck Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/duck_delay.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `duckDelay.time` "Delay" [Float S; 5e+02,1.0,2e+03,1.0] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelay.feedback` "Feedback" [Float S; 0.0,0.0,1.0,0.05] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelay.attack` "Attack" [Float S; 0.1,0.05,0.5,0.05] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelay.relese` "Release" [Float S; 0.1,0.05,2.0,0.05] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `duckDelay.amount` "Amount" [Float S; 0.5,0.0,56.0,0.05] label_ref=`label5:rack_label` <expand=False fill=False>

#### 50. `reversedelay` — "ReverseDelay" (categoria Guitarix: Echo / Delay)

Glade: `gx/builder/reversedelay_ui.glade` raiz `rackbox` ; código: `gx/plugins/reversedelay.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=13 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "Buffer" .rack_label xalign=0.5 <expand=False fill=False>
      - GxPortDisplay `reversedelay.buf_indication` "Buf_indication" [Float SNO; 0.0,0.0,1.0,0.01] {width_request=100} <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `reversedelay.time` "Time (ms)" [Float S; 500,200,2000,1] dica:"Delay time in milliseconds" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `reversedelay.feedback` "Feedback" [Float S; 0,0,1,0.05] dica:"Feedback" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `reversedelay.window` "Window (%)" [Float S; 50,0,100,1] dica:"Crossfade between delayed chunks in percents" label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `reversedelay.drywet` "Dry/Wet" [Float S; 0.5,0,1,0.05] dica:"Dry/Wet" label_ref=`label5:rack_label` <expand=False fill=False>

#### 51. `baxandall` — "Baxandall" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/baxandall.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=25 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `baxandall.ba` "bass" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `baxandall.tr` "treble" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 52. `distortion2` — "WaveSharper" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/faust-generated/distortion2.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `distortion2.gain` "Gain" [Float S; 0.0,-3e+01,1e+01,0.1] dica:"Volume Level" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `distortion2.drive` "Drive" [Float S; 5e+01,0.0,1e+02,1.0] dica:"Distortion Level" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `distortion2.Hfreq` "Hi Pass" [Float SL; 3e+01,2e+01,2e+03,1.08] dica:"Highpass Frequency" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `distortion2.freq` "Lo Pass" [Float SL; 1e+04,1e+03,2e+04,1.08] dica:"Lowpass Frequency" label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `distortion2.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label5:rack_label` <expand=False fill=False>

#### 53. `fuzzface` — "Fuzz Face" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/faust-generated/fuzzface.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzface.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `fuzzface.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 54. `trbuff` — "Transistor Buffer" (categoria Guitarix: Guitar Effects)

Sem glade (load_ui ausente): `gx/faust-generated/trbuff.cc`. Mostra só a barra de título (RackUnit sem controles).

#### 55. `fuzzfacefm` — "Fuzz Face Fuller" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/fuzzfacefm.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzfacefm.Input` "Input" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzfacefm.Drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzfacefm.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `fuzzfacefm.Level` "Level" [Float S; 0.5,0.0,1,0.01] label_ref=`label4:rack_label` <expand=False fill=False>

#### 56. `fuzzfacerm` — "Fuzz Face Mayer" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/fuzzfacerm.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzfacerm.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `fuzzfacerm.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 57. `hornet` — "Hornet" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/hornet.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `hornet.Sustain` "Sustain" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `hornet.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `hornet.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 58. `susta` — "Sustainer" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/susta.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `susta.Sustain` "Sustain" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `susta.Volume` "Volume" [Float S; 0.5,0.0,1,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 59. `hfb` — "HF Brightener" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/hfb.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `hfb.Intensity` "Intensity" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `hfb.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 60. `muff` — "Muff" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/muff.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `muff.Tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `muff.Level` "Level" [Float S; 0.5,0.0,1,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 61. `scream` — "Screaming Bird" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/scream.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `scream.Scream` "Scream" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>

#### 62. `lpbboost` — "LPB-1 Booster" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/lpbboost.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `lpbboost.Boost` "Boost" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>

#### 63. `foxeylady` — "Foxey Lady" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/foxeylady.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `foxeylady.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `foxeylady.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 64. `hogsfoot` — "Hogs Foot" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/hogsfoot.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `hogsfoot.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `hogsfoot.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label2:rack_label` <expand=False fill=False>

#### 65. `cstbt1` — "Colorsound Tone Blender" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/cstb.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `cstbt1.Attack` "Attack" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `cstbt1.Tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `cstbt1.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `cstbt1.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label4:rack_label` <expand=False fill=False>

#### 66. `coverd` — "C Overdrive" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/coverd.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `coverd.Bass` "Bass" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `coverd.Treble` "Treble" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `coverd.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `coverd.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label4:rack_label` <expand=False fill=False>

#### 67. `rangem` — "Rangemaster" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/rangem.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `rangem.Boost` "Boost" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `rangem.wet_dry` "wet/dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label2:rack_label` <expand=False fill=False>

#### 68. `ruin` — "Ruiner" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/ruin.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ruin.Intensity` "Intensity" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ruin.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ruin.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 69. `ffreak` — "Fat Furry Freak" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/ffreak.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ffreak.Gate` "Gate" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ffreak.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ffreak.Tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ffreak.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ffreak.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label5:rack_label` <expand=False fill=False>

#### 70. `fumaster` — "Fuzz Master" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/fumaster.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fumaster.Tone` "Tone" [Float S; 0.5,0.0,0.99,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `fumaster.Level` "Level" [Float S; 0.5,0.0,1,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fumaster.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 71. `fuzzdrive` — "Fuzz Drive" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/fuzzdrive.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzdrive.Distortion` "Drive" [Float S; 0.5,0.0,0.99,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `fuzzdrive.Level` "Level" [Float S; -2.0,-2e+01,12.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `fuzzdrive.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label3:rack_label` <expand=False fill=False>

#### 72. `rolandwah` — "Roland Double Beat" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/plugins/rolandwah.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=23 `vbox6` <expand=False fill=False>
      - Label `label01:rack_label` "Mode" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSelector `rolandwah.mode` "Mode" [Float B; 0.0,0.0,1.0,1.0] <expand=False fill=False>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `rolandwah.lfobpm` "Alien Freq" [Float S; 24.0,24.0,3.6e+02,1.0] dica:"LFO in Beats per Minute" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `rolandwah.Fuzz` "Fuzz" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `rolandwah.Wah` "Wah" [Float S; 0.5,0.02,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `rolandwah.wet_dry` "Dry/Wet" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label4:rack_label` <expand=False fill=False>

#### 73. `astrofuzz` — "Astrotone Fuzz" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/astrofuzz.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `astrofuzz.Attack` "Attack" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `astrofuzz.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `astrofuzz.Tone` "Tone" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 74. `buffb` — "Buffer Booster" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/buffb.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `buffb.Intensity` "Intensity" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `buffb.Volume` "Volume" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 75. `eldist` — "Electra Distortion" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/plugins/eldist.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `eldist.Drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>

#### 76. `mole` — "The Mole" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/plugins/mole.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `mole.Boost` "Boost" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `mole.wet_dry` "wet/dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label2:rack_label` <expand=False fill=False>

#### 77. `buzz` — "Buzz Box" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/buzz.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `buzz.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `buzz.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 78. `bfuzz` — "Bass Fuzz Pedal" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/bfuzz.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bfuzz.Drive` "Drive" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bfuzz.Thickness` "Thickness" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bfuzz.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bfuzz.wet_dry` "Wet/Dry" [Float S; 1e+02,0.0,1e+02,1.0] dica:"percentage of processed signal in output signal" label_ref=`label4:rack_label` <expand=False fill=False>

#### 79. `axface` — "Axis Face" (categoria Guitarix: Fuzz)

Glade inline (`glade_def`) em `gx/plugins/axface.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `axface.Attack` "Attack" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `axface.Level` "Level" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `axface.Smooth` "Smooth" [Float S; 0.5,0.0,1.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>

#### 80. `chorus` — "Chorus" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/chorus.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `chorus.delay` "Delay" [Float S; 0.02f,0.0f,0.2f,0.01f] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `chorus.depth` "Depth" [Float S; 0.02f,0.0f,1.0f,0.01f] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `chorus.freq` "Freq" [Float S; 3.0f,0.0f,1e+01f,0.01f] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `chorus.level` "Level" [Float S; 0.5f,0.0f,1.0f,0.01f] label_ref=`label4:rack_label` <expand=False fill=False>

#### 81. `flanger` — "Flanger" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/flanger.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox V sp=0 `vbox2` <expand=True fill=True>
    - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `flanger.lfobpm` "BPM" [Float S; 24.0,24.0,3.6e+02,1.0] dica:"LFO in Beats per Minute" label_ref=`label1:rack_label` <expand=True fill=True>
  - GtkBox V sp=10 `vbox8` <expand=True fill=True>
    - GtkBox H sp=0 `hbox1` <expand=True fill=True>
      - GtkBox V sp=0 `vbox3` <expand=True fill=True>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `flanger.feedback gain` "Feedback" [Float S; 0.0,0.0,1.0,0.01] label_ref=`label2:rack_label` <expand=True fill=False>
      - GtkBox V sp=0 `vbox4` <expand=True fill=True>
        - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `flanger.flange delay` "Delay" [Float S; 1e+01,0.0,2e+01,0.01] label_ref=`label3:rack_label` <expand=True fill=False>
      - GtkBox V sp=0 `vbox5` <expand=True fill=True>
        - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `flanger.flange delay offset` "Offset" [Float S; 1.0,0.0,2e+01,0.01] label_ref=`label4:rack_label` <expand=True fill=False>
      - GtkBox V sp=0 `vbox6` <expand=True fill=True>
        - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `flanger.depth` "Depth" [Float S; 1.0,0.0,1.0,0.01] label_ref=`label5:rack_label` <expand=True fill=False>
    - GtkBox H sp=6 `hbox2` <expand=True fill=True>
      - Label `label11:rack_label_inverse` "label" .rack_label_inverse xalign=1 <expand=False fill=True>
      - GxSelector `flanger.invert` "Invert" [Float B; 0.0,0.0,1.0,1.0] label_ref=`label11:rack_label_inverse` <expand=True fill=True>
  - GtkBox V sp=0 `vbox7` <expand=True fill=True>
    - Label `label6:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `flanger.level` "Level" [Float S; 0.0,-6e+01,1e+01,0.1] label_ref=`label6:rack_label` <expand=True fill=True>

#### 82. `phaser` — "Phaser" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/phaser.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=15 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=4 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `phaser.level` "Level" [Float S; 0.0f,-6e+01f,1e+01f,0.1f] label_ref=`label1:rack_label` <expand=False fill=False>
      - GxSelector `phaser.invert` "Invert" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True>
    - GtkBox V sp=0 `vbox3` <expand=True fill=True>
      - GtkBox H sp=2 `hbox2` <expand=True fill=True>
        - GtkBox V sp=0 `vbox4` <expand=True fill=True>
          - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
          - GxSmallKnobR `phaser.feedback gain` "Feedback" [Float S; 0.0f,0.0f,1.0f,0.01f] label_ref=`label2:rack_label` <expand=True fill=True>
        - GtkBox V sp=0 `vbox5` <expand=True fill=True>
          - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
          - GxSmallKnobR `phaser.depth` "Depth" [Float S; 1.0f,0.0f,1.0f,0.01f] label_ref=`label3:rack_label` <expand=True fill=True>
        - GtkBox V sp=0 `vbox6` <expand=True fill=True>
          - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
          - GxSmallKnobR `phaser.Notch width` "Width" [Float S; 1e+03f,1e+01f,5e+03f,1.0f] label_ref=`label4:rack_label` <expand=True fill=True>
      - GtkBox H sp=2 `hbox3` <expand=True fill=True>
        - GtkBox V sp=0 `vbox7` <expand=True fill=True>
          - GxSmallKnobR `phaser.NotchFreq` "Min Freq" [Float S; 1.5f,1.1f,4.0f,0.01f] label_ref=`label5:rack_label_inverse` <expand=True fill=True>
          - Label `label5:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
        - GtkBox V sp=0 `vbox8` <expand=True fill=True>
          - GxSmallKnobR `phaser.MaxNotch1Freq` "Max Freq" [Float S; 8e+02f,2e+01f,1e+04f,1.0f] label_ref=`label6:rack_label_inverse` <expand=True fill=True>
          - Label `label6:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
        - GtkBox V sp=0 `vbox9` <expand=True fill=True>
          - GxSmallKnobR `phaser.MinNotch1Freq` "Freq" [Float S; 1e+02f,2e+01f,5e+03f,1.0f] label_ref=`label7:rack_label_inverse` <expand=True fill=True>
          - Label `label7:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
    - GtkBox V sp=4 `vbox10` <expand=True fill=True>
      - Label `label8:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `phaser.lfobpm` "Speed (bpm)" [Float S; 3e+01f,24.0f,3.6e+02f,1.0f] dica:"Speed in Beats per Minute" label_ref=`label8:rack_label` <expand=False fill=False>
      - GxSelector `phaser.VibratoMode` "VibratoMode" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True>

#### 83. `stereodelay` — "Stereo Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/stereodelay.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=2 `hbox1` <expand=True fill=False pack_type=end>
    - GxSelector `stereodelay.invert` "Invert" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereodelay.l_gain` "Gain L" [Float S; 0.0f,-2e+01f,2e+01f,0.1f] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereodelay.lbpm` "Delay L" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Left Delay in Beats per Minute" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=4 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereodelay.lfobpm` "LFO Freq" [Float S; 24.0f,24.0f,3.6e+02f,1.0f] dica:"LFO in Beats per Minute" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereodelay.r_gain` "Gain R" [Float S; 0.0f,-2e+01f,2e+01f,0.1f] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereodelay.rbpm` "Delay R" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Right Delay in Beats per Minute" label_ref=`label5:rack_label` <expand=False fill=False>

#### 84. `stereoecho` — "Stereo Echo" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/stereoecho.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=2 `hbox1` <expand=True fill=False pack_type=end>
    - GxSelector `stereoecho.invert` "Invert" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoecho.percent_l` "Release L" [Float S; 0.0f,0.0f,1e+02f,0.1f] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoecho.lbpm` "Time L" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Echo in Beats per Minute" label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=4 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoecho.lfobpm` "LFO Freq" [Float S; 24.0f,24.0f,3.6e+02f,1.0f] dica:"LFO in Beats per Minute" label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoecho.percent_r` "Release R" [Float S; 0.0f,0.0f,1e+02f,0.1f] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoecho.rbpm` "Time R" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Echo in Beats per Minute" label_ref=`label5:rack_label` <expand=False fill=False>

#### 85. `moog` — "Moog Filter" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/moog.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `moog.Q` "Q" [Float S; 1.0,0.0,4.0,0.1] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `moog.fr` "Freq" [Float S; 3e+03,4.4e+02,6e+03,1e+01] label_ref=`label2:rack_label` <expand=False fill=False>

#### 86. `ampmodul` — "Postamp" (categoria Guitarix: Distortion)

Glade inline (`glade_def`) em `gx/faust-generated/gx_ampmodul.cc`

- GtkBox H sp=8 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ampmodul.feedbac` "Dry/FB" [Float S; 0.0,-1.0,1.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ampmodul.wet_dry` "Dry/Wet" [Float S; 0.0,-1.0,1.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ampmodul.level` "Level" [Float S; -2e+01,-4e+01,4.0,0.1] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ampmodul.amp2.stage1.tube1` "Tube I" [Float S; 6.0,-2e+01,2e+01,0.1] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ampmodul.amp2.stage2.tube2` "Tube II" [Float S; 6.0,-2e+01,2e+01,0.1] label_ref=`label5:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox7` <expand=False fill=False>
      - Label `label6:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ampmodul.feedback` "Wet/FB" [Float S; 0.0,-1.0,1.0,0.01] label_ref=`label6:rack_label` <expand=False fill=False>

#### 87. `tonemodul` — "3 Band EQ" (categoria Guitarix: Tone Control)

Glade inline (`glade_def`) em `gx/faust-generated/tonecontroll.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=5 `vbox2` <expand=True fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `tonemodul.Bass` "Bass" [Float S; 0.0,-5.0,5.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox3` <expand=True fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `tonemodul.Middle` "Middle" [Float S; 0.0,-5.0,5.0,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `tonemodul.Treble` "Treble" [Float S; 0.0,-5.0,5.0,0.01] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=False>
    - GtkBox V sp=4 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `tonemodul.sharper` "Sharp" [Float S; -2.0,-2.5,5.0,0.1] label_ref=`label4:rack_label` <expand=False fill=False>
      - GtkBox H sp=0 `hbox2` <expand=True fill=True>
        - GtkBox H sp=0 `hbox3` <expand=True fill=True>
        - GxSwitch `tonemodul.ON` "ON" [Float B; 0.0,0.0,1.0,1.0] base_name=switch_mid <expand=False fill=False>
        - GtkBox H sp=0 `hbox4` <expand=True fill=True>

#### 88. `didest` — "Digital Stereo Delay" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/digital_delay_st.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox V sp=0 `vbox2` <expand=True fill=True>
    - Label `label1` "" xalign=0.5 <expand=False fill=False>
    - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `didest.bpm` "BPM" [Float S; 1.2e+02f,24.0f,3.6e+02f,1.0f] dica:"Delay in Beats per Minute" label_ref=`label1:rack_label` <expand=True fill=True>
    - Label `label2` "" xalign=0.5 <expand=False fill=False>
  - GtkBox V sp=10 `vbox8` {margin_bottom=4 halign=center} <expand=True fill=True>
    - GtkBox H sp=4 `hbox1` <expand=True fill=True>
      - GtkBox V sp=0 `vbox3` <expand=True fill=True>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `didest.feedback` "Feedback" [Float S; 5e+01f,1.0f,1e+02f,1.0f] dica:"Percentage of the feedback level in the de.delay loop" label_ref=`label2:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox4` <expand=True fill=True>
        - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `didest.level` "Level" [Float S; 5e+01f,1.0f,1e+02f,1.0f] dica:"Percentage of the de.delay gain level" label_ref=`label3:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox5` <expand=True fill=True>
        - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `didest.highpass` "Hipass" [Float S; 1.2e+02f,2e+01f,2e+04f,1.0f] dica:"Highpass filter frequency in the feedback loop" label_ref=`label4:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox6` <expand=True fill=True>
        - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxSmallKnobR `didest.howpass` "Lopass" [Float S; 1.2e+04f,2e+01f,2e+04f,1.0f] dica:"Lowpass filter frequency in the feedback loop" label_ref=`label5:rack_label` <expand=False fill=False>
    - GtkGrid `table1` colsp=6 rowsp=3 homog=F/F {margin_left=8 margin_right=8} <expand=False fill=False>
      - GxSelector `didest.mode` "Mode" [Float S; 0.0f,0.0f,3.0f,1.0f] label_ref=`label21:rack_label_inverse` {hexpand=True} <left_attach=1 top_attach=1>
      - Label `label11:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=0 top_attach=0>
      - Label `label21:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=0 top_attach=1>
      - GxSelector `didest.notes` "Notes" [Float S; 4.0f,0.0f,17.0f,1.0f] dica:"Note setting for bpm" label_ref=`label11:rack_label_inverse` {hexpand=True} <left_attach=1 top_attach=0 width=3>
      - Label `label31:rack_label_inverse` "label" .rack_label_inverse xalign=1 <left_attach=2 top_attach=1>
      - GxSwitch `didest.Freeze` "Freeze" [Float B; 0.0,0.0,1.0,1.0] dica:"Freeze the current delay" base_name=frbutton label_ref=`label31:rack_label_inverse` <left_attach=3 top_attach=1>
  - GtkBox V sp=0 `vbox7` <expand=True fill=True>
    - Label `label6:rack_label1` "" .rack_label1 xalign=0.5 <expand=False fill=False>
    - Label `label6:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
    - GxMidKnob `didest.gain` "Gain" [Float S; 1e+02f,0.0f,1.2e+02f,1.0f] dica:"Overall gain of the de.delay line in percent" label_ref=`label6:rack_label` <expand=True fill=True>
    - Label `label6:rack_label2` "" .rack_label2 xalign=0.5 <expand=False fill=False>

#### 89. `stereoverb` — "Stereo Verb" (categoria Guitarix: Reverb)

Glade inline (`glade_def`) em `gx/faust-generated/stereoverb.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GxSelector `stereoverb.invert` "Invert" [Float B; 0.0,0.0,1.0,1.0] <expand=True fill=True>
    - GtkBox V sp=5 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoverb.LFO freq` "LFO Freq" [Float S; 0.2,0.0,5.0,0.01] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `stereoverb.RoomSize` "Room Size" [Float S; 0.5,0.0,1.0,0.025] label_ref=`label2:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox4` <expand=False fill=False>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoverb.damp` "Damp" [Float S; 0.5,0.0,1.0,0.025] label_ref=`label3:rack_label` <expand=False fill=False>
    - GtkBox V sp=5 `vbox5` <expand=False fill=False>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `stereoverb.wet_dry` "Dry/Wet" [Float S; 5e+01,0.0,1e+02,1.0] label_ref=`label4:rack_label` <expand=False fill=False>

#### 90. `zita_rev1` — "Zita Rev1" (categoria Guitarix: Reverb)

Glade inline (`glade_def`) em `gx/plugins/zita_rev1.cc`

- GtkBox H sp=0 `rackbox`
  - GtkBox V sp=16 `vbox` <expand=True fill=True>
    - GtkBox H sp=5 `hbox2` <expand=False fill=True>
      - GtkFrame `frame4` label="" label_xalign=0.5 <expand=True fill=True>
        - GtkAlignment `alignment4` pad L2 T2 R2 B2
          - GtkBox H sp=0 `hbox6`
            - GtkBox V sp=0 `vbox7` <expand=True fill=True>
              - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
              - GxSmallKnobR `zita_rev1.input.in_delay` "Pre Delay" [Float S; 6e+01,2e+01,1e+02,1.0] dica:"Delay in ms before reverberation begins" label_ref=`label1:rack_label` <expand=False fill=True>
            - GtkBox V sp=0 `vbox9` <expand=True fill=True>
              - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `zita_rev1.decay_times.low_rt60` "Low" [Float SL; 3.0,1.0,8.0,1.08] dica:"T60 = time (in seconds) to decay 60dB in low-frequency band" label_ref=`label3:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox8` <expand=True fill=True>
              - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=True>
              - GxSmallKnob `zita_rev1.decay_times.lf_x` "Freq X" [Float SL; 2e+02,5e+01,1e+03,1.08] dica:"Crossover frequency (Hz) separating low and middle frequencies" label_ref=`label2:rack_label` <expand=True fill=True>
              - GxValueDisplay `zita_rev1.decay_times.lf_x` "Freq X" [Float SL; 2e+02,5e+01,1e+03,1.08] dica:"Crossover frequency (Hz) separating low and middle frequencies" round_digits=0 {margin_right=6} <expand=False fill=True>
            - GtkBox V sp=0 `vbox10` <expand=True fill=True>
              - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `zita_rev1.decay_times.mid_rt60` "Mid" [Float SL; 2.0,1.0,8.0,1.08] dica:"T60 = time (in seconds) to decay 60dB in middle band" label_ref=`label4:rack_label` <expand=True fill=True>
            - GtkBox V sp=0 `vbox11` <expand=True fill=True>
              - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
              - GxSmallKnobR `zita_rev1.decay_times.hf_damping` "HF Damp" [Float SL; 6e+03,1.5e+03,2.352e+04,1.08] dica:"Frequency (Hz) at which the high-frequency T60 is half the middle-band's T60" label_ref=`label5:rack_label` <expand=True fill=True>
        - (rótulo do frame:)
        - Label `label121:rack_label` "Reverb" .rack_label xalign=0.5
    - GtkBox H sp=4 `hbox3` <expand=False fill=True>
      - GtkFrame `frame1:frame_inversed` label="" label_xalign=0.5 .frame_inversed <expand=True fill=True>
        - GtkAlignment `alignment1` pad L5 T5 R5 B5
          - GtkBox H sp=0 `hbox4`
            - GtkBox H sp=0 `vbox1` <expand=True fill=True padding=2>
              - Label `label6:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
              - GxSmallKnob `zita_rev1.equalizer1.eq1_freq` "Freq" [Float SL; 315.0,4e+01,2.5e+03,1.08] dica:"Center-frequency of second-order Regalia-Mitra peaking equalizer section 1" label_ref=`label6:rack_label_inverse` <expand=True fill=True>
            - GtkBox H sp=0 `vbox2` <expand=True fill=True padding=2>
              - Label `label7:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
              - GxSmallKnob `zita_rev1.equalizer1.eq1_level` "Level" [Float S; 0.0,-15.0,15.0,0.1] dica:"Peak level in dB of second-order Regalia-Mitra peaking equalizer section 1" label_ref=`label7:rack_label_inverse` <expand=True fill=True>
        - (rótulo do frame:)
        - Label `label15:rack_label_inverse` "Filter 1" .rack_label_inverse xalign=0.5
      - GtkFrame `frame2:frame_inversed` label="" label_xalign=0.5 .frame_inversed <expand=True fill=True>
        - GtkAlignment `alignment2` pad L5 T5 R5 B5
          - GtkBox H sp=0 `hbox7`
            - GtkBox H sp=0 `vbox3` <expand=True fill=True padding=2>
              - Label `label8:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
              - GxSmallKnob `zita_rev1.equalizer2.eq2_freq` "Freq" [Float SL; 1.5e+03,1.6e+02,1e+04,1.08] dica:"Center-frequency of second-order Regalia-Mitra peaking equalizer section 2" label_ref=`label8:rack_label_inverse` <expand=True fill=True>
            - GtkBox H sp=0 `vbox4` <expand=True fill=True padding=2>
              - Label `label9:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
              - GxSmallKnob `zita_rev1.equalizer2.eq2_level` "Level" [Float S; 0.0,-15.0,15.0,0.1] dica:"Peak level in dB of second-order Regalia-Mitra peaking equalizer section 2" label_ref=`label9:rack_label_inverse` <expand=True fill=True>
        - (rótulo do frame:)
        - Label `label14:rack_label_inverse` "Filter 2" .rack_label_inverse xalign=0.5
  - GtkBox V sp=20 `vbox13` <expand=True fill=True>
    - GtkAlignment `alignment3` pad L0 T18 R0 B0 <expand=True fill=True>
      - GtkBox V sp=0 `vbox5`
        - Label `label10:rack_label` "label" .rack_label xalign=0.5 <expand=True fill=True>
        - GxSmallKnobR `zita_rev1.output.dry_wet_mix` "Dry/Wet" [Float S; 0.0,-1.0,1.0,0.01] dica:"-1 = dry, 1 = wet" label_ref=`label10:rack_label` <expand=True fill=True>
    - GtkBox V sp=0 `vbox6` <expand=True fill=True padding=2>
      - Label `label11:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=True fill=True>
      - GxSmallKnobR `zita_rev1.output.level` "Level" [Float S; 0.0,-7e+01,4e+01,0.1] dica:"Output scale factor" label_ref=`label11:rack_label_inverse` <expand=True fill=True>

#### 91. `univibe` — "Vibe" (categoria Guitarix: Modulation)

Glade: `gx/builder/vibe_stereo_ui.glade` raiz `rackbox` ; código: `gx/plugins/vibe.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox V sp=6 `vbox2` <expand=True fill=True>
    - GtkBox H sp=10 `hbox1` <expand=True fill=False>
      - GtkBox V sp=0 `vbox7` <expand=True fill=True>
        - GtkBox H sp=0 `hbox6` <expand=True fill=True>
          - GtkBox V sp=5 `vbox21` <expand=False fill=False pack_type=end>
            - Label `label11:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `univibe.freq` "Tempo" [Float SA; 4.4,0.1,1e+01,0.1] dica:"LFO frequency (Hz)" label_ref=`label11:rack_label` <expand=False fill=False>
          - GtkBox V sp=5 `vbox3` <expand=False fill=False pack_type=end>
            - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `univibe.width` "Width" [Float S; (ver vibe.cc)] label_ref=`label1:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox4` <expand=False fill=False>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxMidKnob `univibe.depth` "Depth" [Float S; (ver vibe.cc)] label_ref=`label2:rack_label` <expand=False fill=False>
      - GtkBox V sp=0 `vbox5` <expand=True fill=True>
        - GtkBox H sp=0 `hbox5` <expand=True fill=True>
          - GtkBox V sp=5 `vbox6` <expand=False fill=False>
            - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `univibe.fb` "F/B" [Float S; (ver vibe.cc)] label_ref=`label3:rack_label` <expand=False fill=False>
          - GtkBox V sp=5 `vbox8` <expand=False fill=False>
            - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `univibe.wet_dry` "Wet/Dry" [Float S; (ver vibe.cc)] label_ref=`label4:rack_label` <expand=False fill=False>
    - GtkBox H sp=10 `hbox7` <expand=True fill=False>
      - Label `label2` "" xalign=0.5 <expand=True fill=True>
      - Label `label41:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - GxSmallKnob `univibe.lrcross` "XOver" [Float S; 0,-1,1,0.01] dica:"left/right channel crossing" label_ref=`label41:rack_label_inverse` <expand=False fill=False>
      - GtkBox V sp=0 `vbox9` <expand=False fill=False>
        - Label `label31:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
        - GtkAlignment `alignment1` pad L0 T0 R0 B0 <expand=False fill=False>
          - GxHSlider `univibe.panning` "Pan" [Float S; 0,-1,1,0.01] dica:"panning of output (left / right)" label_ref=`label31:rack_label_inverse`
      - GxSmallKnob `univibe.stereo` "Phase" [Float SA; 0.11,-0.5,0.5,0.01] dica:"LFO phase shift between left and right channels" label_ref=`label21:rack_label_inverse` <expand=False fill=False>
      - Label `label21:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
      - Label `label3` "" xalign=0.5 <expand=True fill=True>

#### 92. `mbcs` — "Multi Band Compressor Stereo" (categoria Guitarix: Guitar Effects)

Glade inline (`glade_def`) em `gx/plugins/mbcs.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=8 `hbox1` <expand=True fill=True>
    - GtkNotebook `notebook` <expand=True fill=True>
      - GtkBox H sp=0 `hbox2`
        - GtkBox V sp=4 `vbox3` <expand=False fill=True>
          - GtkBox H sp=15 `hbox4` <expand=True fill=True>
            - GtkBox V sp=0 `vbox4` <expand=True fill=True>
              - Label `label_1:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Ratio1` "Ratio1" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox5` <expand=True fill=True>
              - Label `label_2:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup1` "Makeup1" [Float S; 13.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox6` <expand=True fill=True>
              - Label `label_3:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup-Threshold1` "Makeup-Threshold1" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbcs.Mode1` "Mode1" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table1` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbcs.Attack1` "Attack1" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_4:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_7:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_5:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbcs.Release1` "Release1" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 1")
      - GtkBox H sp=0 `hbox5`
        - GtkBox V sp=4 `vbox7` <expand=False fill=True>
          - GtkBox H sp=15 `hbox6` <expand=True fill=True>
            - GtkBox V sp=0 `vbox8` <expand=True fill=True>
              - Label `label_8:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Ratio2` "Ratio2" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox9` <expand=True fill=True>
              - Label `label_9:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup2` "Makeup2" [Float S; 1e+01,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox10` <expand=True fill=True>
              - Label `label_10:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup-Threshold2` "Makeup-Threshold2" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbcs.Mode2` "Mode2" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table2` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbcs.Attack2` "Attack2" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_11:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_14:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_13:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_12:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b1_b2` "Crossover B1-B2 (hz)" [Float SL; 8e+01,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbcs.Release2` "Release2" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 2")
      - GtkBox H sp=0 `hbox7`
        - GtkBox V sp=4 `vbox11` <expand=False fill=True>
          - GtkBox H sp=15 `hbox8` <expand=True fill=True>
            - GtkBox V sp=0 `vbox12` <expand=True fill=True>
              - Label `label_15:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Ratio3` "Ratio3" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox13` <expand=True fill=True>
              - Label `label_16:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup3` "Makeup3" [Float S; 4.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox14` <expand=True fill=True>
              - Label `label_17:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup-Threshold3` "Makeup-Threshold3" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbcs.Mode3` "Mode3" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table3` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbcs.Attack3` "Attack3" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_18:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_21:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_20:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_19:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b2_b3` "Crossover B2-B3 (hz)" [Float SL; 2.1e+02,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbcs.Release3` "Release3" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 3")
      - GtkBox H sp=0 `hbox9`
        - GtkBox V sp=4 `vbox15` <expand=False fill=True>
          - GtkBox H sp=15 `hbox10` <expand=True fill=True>
            - GtkBox V sp=0 `vbox16` <expand=True fill=True>
              - Label `label_22:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Ratio4` "Ratio4" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox17` <expand=True fill=True>
              - Label `label_23:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup4` "Makeup4" [Float S; 8.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox18` <expand=True fill=True>
              - Label `label_24:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup-Threshold4` "Makeup-Threshold4" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbcs.Mode4` "Mode4" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table4` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbcs.Attack4` "Attack4" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=3 top_attach=1>
          - Label `label_25:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_28:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
          - Label `label_27:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_26:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b3_b4` "Crossover B3-B4 (hz)" [Float SL; 1.7e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbcs.Release4` "Release4" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
      - (aba: "Band 4")
      - GtkBox H sp=0 `hbox11`
        - GtkBox V sp=4 `vbox19` <expand=False fill=True>
          - GtkBox H sp=15 `hbox12` <expand=True fill=True>
            - GtkBox V sp=0 `vbox20` <expand=True fill=True>
              - Label `label_29:rack_label` "Ratio" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Ratio5` "Ratio5" [Float S; 2.0,1.0,1e+02,0.1] dica:"Compression ratio" <expand=True fill=True>
            - GtkBox V sp=0 `vbox21` <expand=True fill=True>
              - Label `label_30:rack_label` "Makeup" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup5` "Makeup5" [Float S; 11.0,-5e+01,5e+01,0.1] dica:"Post amplification and threshold" <expand=True fill=True>
            - GtkBox V sp=0 `vbox22` <expand=True fill=True>
              - Label `label_31:rack_label` "Anticlip" .rack_label xalign=0.5 <expand=False fill=False>
              - GxSmallKnobR `mbcs.Makeup-Threshold5` "Makeup-Threshold5" [Float S; 2.0,0.0,1e+01,0.1] dica:"Threshold correction, an anticlip measure" <expand=True fill=True>
          - GxSelector `mbcs.Mode5` "Mode5" [Float S; 1.0,1.0,3.0,1.0] dica:"Compress or Mute the selected band, or Bypass The Compressor" <expand=True fill=True>
        - GtkGrid `table5` colsp=0 rowsp=4 homog=True/F {halign=center} <expand=True fill=True>
          - GxSmallKnob `mbcs.Attack5` "Attack5" [Float S; 0.012,0.001,1.0,0.001] dica:"Time before the compressor starts to kick in" <left_attach=1 top_attach=0>
          - Label `label_32:rack_label` "Att" .rack_label xalign=1 <left_attach=0 top_attach=0>
          - Label `label_34:rack_label` "Low" .rack_label xalign=1 <left_attach=0 top_attach=1>
          - Label `label_33:rack_label` "Rel" .rack_label xalign=1 <left_attach=2 top_attach=0>
          - GxSmallKnob `mbcs.crossover_b4_b5` "Crossover B4-B5 (hz)" [Float SL; 5e+03,2e+01,2e+04,1.08] dica:"Crossover fi.bandpass frequency" <left_attach=1 top_attach=1>
          - GxSmallKnob `mbcs.Release5` "Release5" [Float S; 1.25,0.01,1e+01,0.01] dica:"Time before the compressor releases the sound" <left_attach=3 top_attach=0>
          - Label `label_35:rack_label` "High" .rack_label xalign=1 <left_attach=2 top_attach=1>
      - (aba: "Band 5")
    - GtkBox V sp=0 `vbox2` <expand=True fill=True>
      - Label `label5:rack_label` "Levels" .rack_label xalign=0 <expand=False fill=False>
      - GtkBox H sp=0 `hbox3` {margin_bottom=4} <expand=True fill=True>
        - GxFastMeter `mbcs.v1` "V1" [Float SOLN; 0,-7e+01,5.0,0] dica:"Sum of Band1" orient=vertical dimen=0 falloff=True tooltip="Sum of Band1 " <expand=True fill=True>
        - GxFastMeter `mbcs.v2` "V2" [Float SOLN; 0,-7e+01,5.0,0] dica:"Sum of Band2" orient=vertical dimen=0 falloff=True tooltip="Sum of Band2" <expand=True fill=True>
        - GxFastMeter `mbcs.v3` "V3" [Float SOLN; 0,-7e+01,5.0,0] dica:"Sum of Band3" orient=vertical dimen=0 falloff=True tooltip="Sum of Band3" <expand=True fill=True>
        - GxFastMeter `mbcs.v4` "V4" [Float SOLN; 0,-7e+01,5.0,0] dica:"Sum of Band4" orient=vertical dimen=0 falloff=True tooltip="Sum of Band4" <expand=True fill=True>
        - GxFastMeter `mbcs.v5` "V5" [Float SOLN; 0,-7e+01,5.0,0] dica:"Sum of Band5" orient=vertical dimen=0 falloff=True tooltip="Sum of Band5" <expand=True fill=True>

#### 93. `dattorros_progenitor` — "Plate Reverb" (categoria Guitarix: Reverb)

Glade inline (`glade_def`) em `gx/faust-generated/dattorros_progenitor.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox V sp=0 `vbox13` <expand=True fill=False>
    - GtkBox H sp=0 `hbox3` <expand=True fill=True>
      - GtkBox V sp=0 `vbox4` <expand=False fill=False>
        - Label `label1` "" xalign=0.5 <expand=True fill=True>
        - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxMidKnob `dattorros_progenitor.decay` "Decay" [Float S; 0.1,0.0,0.5,0.01] label_ref=`label2:rack_label` <expand=False fill=False>
        - Label `label2` "" xalign=0.5 <expand=True fill=True>
      - GtkBox V sp=6 `vbox2` <expand=True fill=False>
        - GtkBox H sp=3 `hbox2` <expand=False fill=False>
          - GtkBox V sp=0 `vbox3` <expand=False fill=False>
            - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `dattorros_progenitor.predelay ms` "Predelay" [Float S; 0.0,0.0,2e+02,1e+01] label_ref=`label1:rack_label` <expand=False fill=False>
          - GtkBox V sp=0 `vbox8` <expand=False fill=False>
            - Label `label11:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `dattorros_progenitor.bandwidth` "Bandwidth" [Float S; 0.9,0.1,0.95,0.0005] label_ref=`label11:rack_label` <expand=False fill=False>
          - GtkBox V sp=0 `vbox9` <expand=False fill=False>
            - Label `label21:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `dattorros_progenitor.damping` "HF Damp" [Float S; 0.1,0.1,0.95,0.0005] label_ref=`label21:rack_label` <expand=False fill=False>
          - GtkBox V sp=0 `vbox7` <expand=False fill=False>
            - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
            - GxSmallKnobR `dattorros_progenitor.excursion` "Excursion" [Float S; 0.0,0.0,16.0,0.5] label_ref=`label5:rack_label` <expand=False fill=False>
        - GtkFrame `frame1:frame_inversed` label="" label_xalign=0 .frame_inversed <expand=True fill=True>
          - GtkBox H sp=0 `hbox4`
            - GtkBox V sp=0 `vbox5` <expand=True fill=True>
              - GxSmallKnob `dattorros_progenitor.input diff 1` "Input 1" [Float S; 0.1,0.0,0.75,0.01] label_ref=`label3:rack_label_inverse` <expand=False fill=False>
              - Label `label3:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
            - GtkBox V sp=0 `vbox6` <expand=True fill=True>
              - GxSmallKnob `dattorros_progenitor.input diff 2` "Input 2" [Float S; 0.1,0.0,0.625,0.01] label_ref=`label4:rack_label_inverse` <expand=False fill=False>
              - Label `label4:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
            - GtkBox V sp=0 `vbox10` <expand=True fill=True>
              - GxSmallKnob `dattorros_progenitor.decay diff 1` "Decay 1" [Float S; 0.1,0.0,0.7,0.01] label_ref=`label31:rack_label_inverse` <expand=False fill=False>
              - Label `label31:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
            - GtkBox V sp=0 `vbox11` <expand=True fill=True>
              - GxSmallKnob `dattorros_progenitor.decay diff 2` "Decay 2" [Float S; 0.1,0.0,0.5,0.01] label_ref=`label41:rack_label_inverse` <expand=False fill=False>
              - Label `label41:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
          - (rótulo do frame:)
          - Label `label300:rack_label_inverse` "Diffusion" .rack_label_inverse xalign=0.5
      - GtkBox V sp=0 `vbox12` <expand=False fill=False>
        - Label `label3` "" xalign=0.5 <expand=True fill=True>
        - Label `label51:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
        - GxMidKnob `dattorros_progenitor.dry/wet` "Dry/Wet" [Float S; 0.5,0.0,1.0,0.05] label_ref=`label51:rack_label` <expand=True fill=True>
        - Label `label4` "" xalign=0.5 <expand=True fill=True>

#### 94. `panoram_enhancer` — "Panoram Enhancer" (categoria Guitarix: Misc)

Glade inline (`glade_def`) em `gx/faust-generated/panoram_enhancer.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `panoram_enhancer.Delay width` "Delay" [Float S; 0.0,0.0,1.0,0.01] dica:"Delay Width" label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `panoram_enhancer.Frequency width` "Frequency" [Float S; 0.0,0.0,1.0,0.01] dica:"Frequency Width" label_ref=`label2:rack_label` <expand=False fill=False>

#### 95. `bassEnhancer` — "Bass Enhancer" (categoria Guitarix: Misc)

Glade inline (`glade_def`) em `gx/faust-generated/bass_enhancer.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `bassEnhancer.Frequency` "Frequency" [Float S; 1e+02,6e+01,2.4e+02,5.0] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `bassEnhancer.HarmonicsdB` "Harmonics" [Float S; 0.0,-16.0,32.0,0.1] label_ref=`label2:rack_label` <expand=False fill=False>

#### 96. `ringModulatorSt` — "Ring Modulator" (categoria Guitarix: Modulation)

Glade inline (`glade_def`) em `gx/faust-generated/ring_modulator_st.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=10 `hbox1` <expand=True fill=False pack_type=end>
    - GtkBox V sp=0 `vbox2` <expand=False fill=False>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxMidKnob `ringModulatorSt.freq` "Freq" [Float S; 2.4e+02,1.2e+02,1.6e+03,0.5] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=False>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `ringModulatorSt.dry/wet` "Mix" [Float S; 0.5,0.0,1.0,0.05] label_ref=`label2:rack_label` <expand=False fill=False>

#### 97. `duckDelaySt` — "Duck Delay St" (categoria Guitarix: Echo / Delay)

Glade inline (`glade_def`) em `gx/faust-generated/duck_delay_st.cc`

- GtkBox H sp=4 `rackbox`
  - GtkBox H sp=0 `hbox1` <expand=True fill=False>
    - GtkBox V sp=0 `vbox2` <expand=False fill=True>
      - Label `label1:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxBigKnob `duckDelaySt.time` "Delay" [Float S; 5e+02,1.0,2e+03,1.0] label_ref=`label1:rack_label` <expand=False fill=False>
    - GtkBox V sp=0 `vbox3` <expand=False fill=True>
      - Label `label2:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.feedback` "Feedback" [Float S; 0.0,0.0,1.0,0.05] label_ref=`label2:rack_label` <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.attack` "Attack" [Float S; 0.1,0.05,0.5,0.05] label_ref=`label2:rack_label_inverse` <expand=False fill=False>
      - Label `label2:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
    - GtkBox V sp=0 `vbox4` <expand=False fill=True>
      - Label `label3:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.pingpong` "Ping Pong" [Float S; 0.0,0.0,1.0,0.05] label_ref=`label3:rack_label` <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.release` "Release" [Float S; 0.1,0.05,2.0,0.05] label_ref=`label3:rack_label_inverse` <expand=False fill=False>
      - Label `label3:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
    - GtkBox V sp=0 `vbox5` <expand=False fill=True>
      - Label `label4:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.coloration` "Coloration" [Float S; 0.0,-1.0,1.0,0.05] label_ref=`label4:rack_label` <expand=False fill=False>
      - GxSmallKnobR `duckDelaySt.amount` "Amount" [Float S; 0.5,0.0,56.0,0.05] label_ref=`label4:rack_label_inverse` <expand=False fill=False>
      - Label `label4:rack_label_inverse` "label" .rack_label_inverse xalign=0.5 <expand=False fill=False>
    - GtkBox V sp=0 `vbox6` <expand=False fill=True>
      - Label `label5:rack_label` "label" .rack_label xalign=0.5 <expand=False fill=False>
      - GxBigKnob `duckDelaySt.effect` "Effect" [Float S; 0.0,-16.0,4.0,0.1] label_ref=`label5:rack_label` <expand=False fill=False>


#### Notas finais (Guitarix)

- `trbuff` (Transistor Buffer) não descreve UI: janela só com título.
- No cabeçote (`ampstack`), a raiz `amp_background` é um `GxPaintBox` sem `paint_func` (não pinta; o fundo vem do `SkinBox` 0.33/0.33), e o `gxlogobox` (`live_box_expose`) também não pinta nada. O `GtkImage image4` (`rack_shrink`) de `ampdetail_normal` é ignorado.
- As abas dos notebooks (`(aba: ...)`) são as páginas das unidades multibanda e afins; o rótulo da aba vem do objeto `type=\"tab\"` do glade.
- Unidades com seletor de módulo (crybaby, wah, amp.tonestack, poweramp, cab, pre) mostram a lista de valores no menu do `Selector`; os textos vêm dos `value_pair` (value_label ou value_id) registrados.

### LOMM

- **Classificação:** visual próprio (artwork + paintEvent com medidores de nível e zonas de threshold arrastáveis + muitos LCDs e PixmapButtons) ; **Categoria:** efeito
- **Arquivos:** `plugins/LOMM/LOMMControlDialog.cpp/.h`, `LOMMControls.cpp`, `LOMM.h` (`LOMM_MAX_LOOKAHEAD = 20.f`).
  - Imagens:
    - `artwork.png` 400x256
    - `crossover_led_green.png` / `crossover_led_off.png` 15x14 (usados 2x)
    - `high_band_active/inactive.png`, `mid_band_active/inactive.png`, `low_band_active/inactive.png` 114x22
    - `depthScaling_active/inactive.png` 37x8
    - `stereoLink_active/inactive.png` 32x19
    - `init_active/inactive.png` 32x19
    - `feedback_active/inactive.png` 31x17
    - `midside_active/inactive.png` 30x17
    - `lowSideUpwardSuppress_active/inactive.png` 15x19
    - `lookahead_active/inactive.png` 64x16
    - **Órfãs (não referenciadas no código):** `split1Enabled_active.png`, `split1Enabled_inactive.png`, `split2Enabled_active.png`, `split2Enabled_inactive.png` (29x19). Os botões de split usam `crossover_led_*`.
  - Sem SVG.
- **Janela:** `setFixedSize(400, 256)`, fundo `artwork.png`.
- **Controles:** helpers: `createKnob(type, x, y, model, hint, unit, tooltip)`; `createLcdFloatSpinBox(intDigits, fracDigits, style, name, x, y, model, tooltip)` com `setSeamless(true,true)` (o 4º argumento é o nome do widget, não um rótulo visível); `createPixmapButton(name, x, y, model, active, inactive, tooltip)` sempre `setCheckable(true)`.

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Knob Bright26 | `m_depthModel` tr("Depth") (0.4, 0..1) | (10,4) | "Depth:" ""; "Compression amount for all bands" |
| Knob Bright26 | `m_timeModel` tr("Time") (1, 0..10) | (10,41) | "Time:" ""; "Attack/release scaling for all bands" |
| Knob Bright26 | `m_inVolModel` tr("Input Volume") (0, -48..48) | (10,220) | "Input Volume:" " dB"; "Input volume" |
| Knob Bright26 | `m_outVolModel` tr("Output Volume") (8, -48..48) | (363,220) | "Output Volume:" " dB"; "Output volume" |
| Knob Bright26 | `m_upwardModel` tr("Upward Depth") (1, 0..2) | (10,179) | "Upward Depth:" ""; "Upward compression amount for all bands" |
| Knob Bright26 | `m_downwardModel` tr("Downward Depth") (1, 0..2) | (363,179) | "Downward Depth:" ""; "Downward compression amount for all bands" |
| Knob Bright26 | `m_mixModel` tr("Mix") (1, 0..1) | (363,4) | "Mix:" ""; "Wet/Dry of all bands" |
| LcdFloatSpinBox 5.2 "11green" | `m_split1Model` tr("High/Mid Split") (2500, 20..20000) | (352,76) | "High/Mid Crossover" |
| LcdFloatSpinBox 5.2 "11green" | `m_split2Model` tr("Mid/Low Split") (88.3) | (352,156) | "Mid/Low Crossover" |
| PixmapButton `crossover_led_green/off` 15x14 | `m_split1EnabledModel` tr("Enable High/Mid Split") | (369,104) | "High/mid band split" |
| PixmapButton `crossover_led_green/off` 15x14 | `m_split2EnabledModel` tr("Enable Mid/Low Split") | (369,126) | "Mid/low band split" |
| PixmapButton `high_band_active/inactive` 114x22 | `m_band1EnabledModel` tr("Enable High Band") | (143,66) | "Enable High Band" |
| PixmapButton `mid_band_*` 114x22 | `m_band2EnabledModel` tr("Enable Mid Band") | (143,146) | "Enable Mid Band" |
| PixmapButton `low_band_*` 114x22 | `m_band3EnabledModel` tr("Enable Low Band") | (143,226) | "Enable Low Band" |
| Knob Bright26 | `m_inHighModel` tr("High Input Volume") (0) | (53,43) | "High Input Volume:" " dB"; "Input volume for high band" |
| Knob Bright26 | `m_inMidModel` tr("Mid Input Volume") | (53,123) | "Mid Input Volume:" " dB"; "Input volume for mid band" |
| Knob Bright26 | `m_inLowModel` tr("Low Input Volume") | (53,203) | "Low Input Volume:" " dB"; "Input volume for low band" |
| Knob Bright26 | `m_outHighModel` tr("High Output Volume") (4.6) | (320,43) | "High Output Volume:" " dB"; "Output volume for high band" |
| Knob Bright26 | `m_outMidModel` tr("Mid Output Volume") (0) | (320,123) | "Mid Output Volume:" " dB"; "Output volume for mid band" |
| Knob Bright26 | `m_outLowModel` tr("Low Output Volume") (4.6) | (320,203) | "Low Output Volume:" " dB"; "Output volume for low band" |
| LcdFloatSpinBox 3.3 "11green" | `m_aThreshHModel` tr("Above Threshold High") (-30.3, -72..0) | (300,13) | "Downward compression threshold for high band" |
| LcdFloatSpinBox 3.3 | `m_aThreshMModel` tr("Above Threshold Mid") (-25) | (300,93) | "Downward compression threshold for mid band" |
| LcdFloatSpinBox 3.3 | `m_aThreshLModel` tr("Above Threshold Low") (-28.6) | (300,173) | "Downward compression threshold for low band" |
| LcdFloatSpinBox 2.2 | `m_aRatioHModel` tr("Above Ratio High") (99.99, 1..99.99) | (284,44) | "Downward compression ratio for high band" |
| LcdFloatSpinBox 2.2 | `m_aRatioMModel` tr("Above Ratio Mid") (66.7) | (284,124) | "Downward compression ratio for mid band" |
| LcdFloatSpinBox 2.2 | `m_aRatioLModel` tr("Above Ratio Low") (66.7) | (284,204) | "Downward compression ratio for low band" |
| LcdFloatSpinBox 3.3 | `m_bThreshHModel` tr("Below Threshold High") (-35.6) | (59,13) | "Upward compression threshold for high band" |
| LcdFloatSpinBox 3.3 | `m_bThreshMModel` tr("Below Threshold Mid") (-36.6) | (59,93) | "Upward compression threshold for mid band" |
| LcdFloatSpinBox 3.3 | `m_bThreshLModel` tr("Below Threshold Low") (-35.6) | (59,173) | "Upward compression threshold for low band" |
| LcdFloatSpinBox 2.2 | `m_bRatioHModel` tr("Below Ratio High") (4.17) | (87,44) | "Upward compression ratio for high band" |
| LcdFloatSpinBox 2.2 | `m_bRatioMModel` tr("Below Ratio Mid") | (87,124) | "Upward compression ratio for mid band" |
| LcdFloatSpinBox 2.2 | `m_bRatioLModel` tr("Below Ratio Low") | (87,204) | "Upward compression ratio for low band" |
| Knob Small17 | `m_atkHModel` tr("Attack High") (13.5, 0..1000) | (120,61) | "Attack High:" " ms"; "Attack time for high band" |
| Knob Small17 | `m_atkMModel` tr("Attack Mid") (22.4) | (120,141) | "Attack Mid:" " ms"; "Attack time for mid band" |
| Knob Small17 | `m_atkLModel` tr("Attack Low") (47.8) | (120,221) | "Attack Low:" " ms"; "Attack time for low band" |
| Knob Small17 | `m_relHModel` tr("Release High") (132) | (261,61) | "Release High:" " ms"; "Release time for high band" |
| Knob Small17 | `m_relMModel` tr("Release Mid") (282) | (261,141) | "Release Mid:" " ms"; "Release time for mid band" |
| Knob Small17 | `m_relLModel` tr("Release Low") (282) | (261,221) | "Release Low:" " ms"; "Release time for low band" |
| Knob Small17 | `m_rmsTimeModel` tr("RMS Time") (10, 0..500) | (380,42) | "RMS Time:" " ms"; "RMS size for sidechain signal (set to 0 for Peak mode)" |
| Knob Small17 | `m_kneeModel` tr("Knee") (6, 0..36) | (356,42) | "Knee:" " dB"; "Knee size for all compressors" |
| Knob Small17 | `m_rangeModel` tr("Range") (36, 0..96) | (24,146) | "Range:" " dB"; "Maximum gain increase for all bands" |
| Knob Small17 | `m_balanceModel` tr("Balance") (0, -18..18) | (13,114) | "Balance:" " dB"; "Bias input volume towards one channel" |
| Knob Small17 | `m_autoTimeModel` tr("Auto Time") (0, 0..1) | (24,80) | "Auto Time:" ""; "Speed up attack and release times when transients occur" |
| PixmapButton `depthScaling_*` 37x8 | `m_depthScalingModel` tr("Scale output volume with Depth") (true) | (51,0) | "Scale output volume with Depth parameter" |
| PixmapButton `stereoLink_*` 32x19 | `m_stereoLinkModel` tr("Stereo Link") | (52,237) | "Apply same gain change to both channels" |
| PixmapButton `feedback_*` 31x17 | `m_feedbackModel` tr("Feedback") | (317,238) | "Use output as sidechain signal instead of input". Oculto quando Lookahead ligado |
| PixmapButton `midside_*` 30x17 | `m_midsideModel` tr("Mid/Side") | (285,238) | "Compress mid/side channels instead of left/right" |
| PixmapButton `lowSideUpwardSuppress_*` 15x19 | `m_lowSideUpwardSuppressModel` tr("Suppress upward compression for side band") | (106,180) | mesmo texto. Visível só com Mid/Side ligado |
| PixmapButton `lookahead_*` 64x16 | `m_lookaheadEnableModel` tr("Lookahead") | (147,0) | "Enable lookahead with fixed 20 ms latency" |
| LcdFloatSpinBox 2.2 "11green" | `m_lookaheadModel` tr("Lookahead Length") (0, 0..20) | (214,2) | "Lookahead length" |
| PixmapButton `init_*` 32x19, NÃO checkable, sem modelo | slot `resetAllParameters()` | (84,237) | "Clear all parameters" |

- **Visores vivos:** área de medidores desenhada no `paintEvent`, `update()` a cada `periodicUpdate` (60 Hz).
  - Constantes (`LOMMControlDialog.h`): `LOMM_DISPLAY_MIN = -72`, `LOMM_DISPLAY_MAX = 0` dB, `LOMM_DISPLAY_X = 125`, `LOMM_DISPLAY_WIDTH = 150` (x 125..275), `LOMM_DISPLAY_HEIGHT = 13`, `LOMM_DISPLAY_Y = {24, 41, 106, 123, 186, 203}` (pares L/R por banda: alta y 24 e 41, média 106 e 123, baixa 186 e 203). `DB_PER_PIXEL = 72/150 = 0.48`.
  - `dbfsToX(d) = clamp(125 + (d + 72)/72 * 150, 125, 275)`.
  - Dados: `m_effect->m_displayIn[banda][canal]` e `m_displayOut[banda][canal]` (dBFS) e os thresholds dos modelos.
- **Desenho próprio (`paintEvent`)**, sem antialias:
  - Para cada banda i (0 alta, 1 média, 2 baixa), região vertical de `Y[2i]` até `Y[2i+1] + 13` (30 px: 24..54, 106..136, 186..216):
    - Zona "above" (compressão para baixo): `fillRect(xA, Y[2i], 275 - xA, 30)` com `QColor(255,255,0,31)` e linha vertical em xA com pen `QColor(255,255,0,255)` largura 1 (amarelo).
    - Zona "below" (compressão para cima): `fillRect(125, Y[2i], xB - 125, 30)` com `QColor(255,0,0,31)` e linha vertical em xB com pen `QColor(255,0,0,255)` largura 1 (vermelho).
  - Para cada banda, níveis:
    - Entrada L: linha vertical em x(inL) de `Y[2i] + 4` até `Y[2i] + 13`; entrada R: de `Y[2i+1]` até `Y[2i+1] + 9`. Pen `QColor(200,200,200,80)` largura 1.
    - Saída L: de `Y[2i]` até `Y[2i] + 13`; saída R: de `Y[2i+1]` até `Y[2i+1] + 13`. Pen `QColor(255,255,255,255)` largura 1.
  - Interação com mouse na área (x de 115 a 285, y dentro da banda): botão esquerdo arrasta: à esquerda do threshold "below" move o below; à direita do "above" move o above; entre os dois move ambos juntos (limitados aos ranges). 1 px = 0.48 dB. Botão do meio reseta o(s) threshold(s) correspondente(s). Ctrl/Shift desativam.
- **Notas:** imagens órfãs `split1Enabled_*`/`split2Enabled_*`. O botão Init está ligado a `resetAllParameters()`.

### MultitapEcho

- **Classificação:** visual próprio (artwork + 2 Graphs de barras com fundo próprio) ; **Categoria:** efeito
- **Arquivos:** `plugins/MultitapEcho/MultitapEchoControlDialog.cpp/.h`, `MultitapEchoControls.cpp`.
  - Imagens: `artwork.png` 245x300; `graph_bg.png` 204x105 (fundo dos dois Graphs). Ambas referenciadas. Sem SVG.
- **Janela:** `setFixedSize(245, 300)`, fundo `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Graph `Style::Bar` 204x105, fundo `graph_bg.png`, cor `QColor(11,213,86)`, maxSize 204x105 | `m_ampGraph` (-60..0 dB, 16 amostras; tamanho segue Steps) | (30,10) | — |
| Graph `Style::Bar` 204x105, fundo `graph_bg.png`, cor `QColor(0,200,187)` | `m_lpGraph` (0..3, 16 amostras) | (30,125) | — |
| LcdSpinBox 2 dígitos (estilo padrão), nome "Steps" (sem rótulo visível) | `m_steps` "Steps" (16, 4..32) | (20,245) | — |
| TempoSyncKnob Bright26, rótulo tr("Length"), SMALL_FONT_SIZE | `m_stepLength` "Step length" (100, 1..500, escala 500) | (100,245) | "Step length:" " ms" |
| Knob Bright26, rótulo tr("Dry") | `m_dryGain` "Dry gain" (0, -80..20) | (150,245) | "Dry gain:" " dBFS" |
| Knob Bright26, rótulo tr("Stages") | `m_stages` "Lowpass stages" (1, 1..4, passo estrito) | (200,245) | "Low-pass stages:" "x" |
| LedCheckBox texto "Swap inputs", Green | `m_swapInputs` "Swap inputs" | (20,275) | "Swap left and right input channels for reflections" |

- **Visores vivos:** nenhum (só os Graphs editáveis).
- **Desenho próprio:** nenhum.
- **Notas:** nomes dos modelos aqui são strings sem `tr()`. O texto "Swap inputs" do LED não passa por tr().

### Oscilloscope

- **Classificação:** visual próprio (visor pintado `OscilloscopeGraph`); **Categoria:** efeito (visualizador)
- **Arquivos:** `plugins/Oscilloscope/OscilloscopeControlDialog.cpp/.h`, `plugins/Oscilloscope/OscilloscopeGraph.cpp/.h`, `OscilloscopeControls.cpp/.h`. Imagens: `data/themes/default/play.png` (20x20) e `data/themes/default/pause.png` (20x20) via `embed::getIconPixmap`. Nenhuma imagem própria no diretório do plugin, nenhum SVG. Ícone do descritor: `lmms-plugin-logo` (tema, SVG).
- **Janela:** sem tamanho fixo; `QVBoxLayout` com margens 0,0,0,0 e spacing 5. Topo: gráfico (mínimo 400x200, `Expanding`/`Expanding`, cresce com a janela). Embaixo: um `QWidget` com `QHBoxLayout` margens 5,0,5,0, spacing 10. Fundo: `setAutoFillBackground(true)` sem paleta própria, ou seja cor de fundo da paleta do tema (`LmmsPalette background #262b30`).
- **Controles:** (na ordem da linha horizontal inferior, da esquerda para a direita)

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `PixmapButton` checável; ativo = `play` (tema), inativo = `pause` (tema) | `m_pauseModel` tr("Pause"), default false | linha inferior, 1o | tooltip "Pause" |
| `LedCheckBox` texto "Stereo" (literal, sem tr), cor Green | `m_stereoModel` tr("Stereo"), default false | 2o | (sem) |
| `Knob` Bright26, rótulo tr("Window Size") | `m_lengthModel` tr("Length"), 500, 1..5000, passo 0.001, escala logarítmica | 3o | sem setHintText; só `setUnit(" ms")` |
| `Knob` Bright26, rótulo tr("Offset") | `m_phaseModel` tr("Phase"), 1.0, 0..1, passo 0.00001 | 4o | (sem) |
| `VolumeKnob` Bright26, rótulo tr("Scale") | `m_ampModel` tr("Amplitude"), 100, 0..1000, passo 0.00001 | 5o | (sem; VolumeKnob mostra %/dB) |

- **Visores vivos:** `OscilloscopeGraph` (QWidget, min 400x200). Fonte: `LocklessRingBufferReader<SampleFrame>` do `Oscilloscope::inputBuffer()`, copiado para um anel local de `sampleRate * MaxBufferLengthSeconds` quadros (`MaxBufferLengthSeconds = 5.0f`). O áudio só escreve no buffer quando `m_pauseModel` é false (`Oscilloscope.cpp:75`). Atualiza a 60 fps (`periodicUpdate -> update()`).
  - Janela visível: `windowSizeFrames = ringSize * length / lengthMax`; `framesPerPixel = max(1, windowSizeFrames / width)`; início = `ringIndex + (ringSize - windowSizeFrames) + phase * ringSize`, quantizado ao múltiplo de `framesPerPixel` (evita tremulação).
  - Para cada coluna: em modo HQ (`framesPerPixel > 1`) calcula min/máx dos quadros da coluna e desenha uma linha vertical de `(x, h*(1-min*amp)/2)` a `(x, h*(1-max*amp)/2)`; senão liga amostra atual à próxima. `amp = m_ampModel/100`.
  - Mono: canal = média L/R, cor `monoColor`. Estéreo: L depois R, cada um com sua cor, modo de composição `CompositionMode_Plus` (sobreposição soma).
- **Desenho próprio:** `fillRect` com `backgroundColor`; linha central horizontal em `y = h/2` com `minorLineColor`; duas linhas de clipping em `y = h/2*(1-amp) - 1` e `y = h/2*(1+amp) + 1` com `clippingLineColor` (com amp = 1 ficam fora da área). Pen de 1 px (padrão), sem antialias. Cores: padrões no código (`OscilloscopeGraph.h`): mono `QColor(255,255,255)`, left `(126,146,255,128)`, right `(255,128,128,128)`, minorLine `(100,100,100)`, clippingLine `(100,0,0)`, background `(0,0,0)`. O tema sobrescreve (`style.css:1186`): mono `rgb(255,255,255)`, left `#289fff`, right `#ee043b`, minorLine `rgb(100,100,100)`, clippingLine `#ffc976`, background `rgb(0,0,0)`. Sem texto.
- **Notas:** interação no gráfico: roda do mouse = zoom no tamanho da janela, ancorado na posição do mouse (`exp2(-delta/240)`), ajustando `m_phaseModel`; arrastar com o botão = desloca a fase (wrap em 0..1). O seletor de estilo `lmms--gui--Oscilloscope` em `style.css:236` é do osciloscópio da barra principal, não deste plugin.

### PeakControllerEffect

- **Classificação:** visual próprio (fundo artwork.svg liso), controles em layout; **Categoria:** efeito (gera controlador)
- **Arquivos:** `plugins/PeakControllerEffect/PeakControllerEffectControlDialog.cpp/.h`, `PeakControllerEffectControls.cpp/.h`. Imagens: `plugins/PeakControllerEffect/artwork.svg` (width="240" height="80"), só gradiente base; ícone da janela `data/themes/default/controller.png` (via `embed::getIconPixmap("controller")`, `setWindowIcon`).
- **Janela:** por layout (`QVBoxLayout` `SetFixedSize`): linha de 6 knobs + linha de 2 LEDs. Artwork pretende 240x80.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `Knob` Bright26 tr("BASE") | `m_baseModel` tr("Base value"), 0.5, 0..1 | linha 1, 1o | "Base:" "" |
| `Knob` Bright26 tr("AMNT") | `m_amountModel` tr("Modulation amount"), 1, -1..1 | 2o | "Modulation amount:" "" |
| `Knob` Bright26 tr("MULT") | `m_amountMultModel` tr("Amount multiplicator"), 1, 0..32 | 3o | "Amount multiplicator:" "" |
| `Knob` Bright26 tr("ATCK") | `m_attackModel` tr("Attack"), 0, 0..0.999 | 4o | "Attack:" "" |
| `Knob` Bright26 tr("DCAY") | `m_decayModel` tr("Release"), 0, 0..0.999 | 5o | "Release:" "" |
| `Knob` Bright26 tr("TRSH") | `m_tresholdModel` tr("Treshold"), 0, 0..1 | 6o | "Treshold:" "" |
| `LedCheckBox` tr("Mute output"), Yellow | `m_muteModel` tr("Mute output"), false | linha 2, 1o | — |
| `LedCheckBox` tr("Absolute value"), Yellow | `m_absModel` tr("Absolute value"), true | linha 2, 2o | — |

- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** "Treshold" com erro de grafia no original (manter ou corrigir na porta).

### ReverbSC

- **Classificação:** visual próprio (fundo artwork.svg liso), controles em layout; **Categoria:** efeito
- **Arquivos:** `plugins/ReverbSC/ReverbSCControlDialog.cpp/.h`, `ReverbSCControls.cpp/.h`. Imagem: `plugins/ReverbSC/artwork.svg` (width="185" height="55"), só gradiente base.
- **Janela:** por layout (`QHBoxLayout` `SetFixedSize`), uma linha de 4 knobs; artwork 185x55.
- **Controles:** `Knob` Bright26 tr("Input") -> `m_inputGainModel` tr("Input gain") 0, -60..15, dica "Input gain:" "dB"; `Knob` Bright26 tr("Size") -> `m_sizeModel` tr("Size") 0.89, 0..1, "Size:" ""; `Knob` Bright26 tr("Color") -> `m_colorModel` tr("Color") 10000, 100..15000, "Color:" ""; `Knob` Bright26 tr("Output") -> `m_outputGainModel` tr("Output gain") 0, -60..15, "Output gain:" "dB".
- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** rótulos em caixa mista ("Input", não "INPUT"), diferente dos outros.

### SlewDistortion

- **Classificação:** visual próprio (artwork 638x271 + paintEvent com medidores invertidos e curvas de transferência + `Draggable` com pixmap como "faders") ; **Categoria:** efeito
- **Arquivos:** `plugins/SlewDistortion/SlewDistortionControlDialog.cpp/.h` (inclui `SlewDistortionHelpView`, QTextEdit), `SlewDistortionControls.cpp`, `SlewDistortion.h`; widget do tema `src/gui/widgets/Draggable.cpp`.
  - Imagens (todas referenciadas): `artwork.png` 638x271; `handle.png` 28x15; `handle_zero.png` 28x15; `link_on/off.png` 22x22; `dc_on/off.png` 82x18; `mb_on/off.png` 82x18; `oversample_1x/2x/4x/8x/16x/32x_on/off.png` 26x18 cada; `help_on/off.png` 14x14. Sem SVG. Help window usa `PixmapLoader("lmms-plugin-logo")` (tema) como ícone.
- **Janela:** `setFixedSize(638, 271)`, fundo `artwork.png`.
- **Controles:** `makeKnob(x, y, hint, unit, model, smol)` = Bright26 (ou Small17 se smol). Nenhum knob tem tooltip, só hint (sem unidade).

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| ComboBox | `m_distType1Model` tr("Type 1"): itens "Hard Clip", "Tanh", "Fast Soft Clip 1", "Fast Soft Clip 2", "Sinusoidal", "Foldback", "Full Rectify", "Half Rectify", "Smooth Rectify", "Bitcrush" | geometry (85,26,115,22) | — |
| ComboBox | `m_distType2Model` tr("Type 2"), mesmos itens | (85,147,115,22) | — |
| Draggable vertical `handle.png` (28x15), `handle_zero.png` quando valor == 0 | `m_drive1Model` tr("Drive 1") (0, -24..24) | x=16; pointA=108 (mín), pointB=34 (máx); y = int(lerp(108,34,norm) - 7.5); padrão y=63, faixa y 100..26 | — |
| Draggable | `m_drive2Model` tr("Drive 2") | x=16; A=229, B=155; padrão y=184 (faixa 221..147) | — |
| Draggable | `m_bias1Model` tr("Bias 1") (0, -2..2) | x=416; A=112, B=34; padrão y=65 (faixa 104..26) | — |
| Draggable | `m_bias2Model` tr("Bias 2") | x=416; A=233, B=155; padrão y=186 (faixa 225..147) | — |
| Draggable | `m_outVol1Model` tr("Out Vol 1") (0, -24..24) | x=594; A=108, B=34; padrão y=63 | — |
| Draggable | `m_outVol2Model` tr("Out Vol 2") | x=594; A=229, B=155; padrão y=184 | — |
| Knob Bright26 | `m_slewUp1Model` tr("Slew Up 1") (6, -96..6) | (96,65) | "Slew Up 1:" |
| Knob Bright26 | `m_slewUp2Model` tr("Slew Up 2") | (96,186) | "Slew Up 2:" |
| Knob Bright26 | `m_slewDown1Model` tr("Slew Down 1") (ou `m_slewUp1Model` se Link 1 ligado) | (163,65) | "Slew Down 1:" |
| Knob Bright26 | `m_slewDown2Model` tr("Slew Down 2") (ou Up 2 se Link 2) | (163,186) | "Slew Down 2:" |
| PixmapButton `link_on/off` 22x22, checkable | `m_slewLink1Model` tr("Slew Link 1") (true) | (132,70) | "Slew Link 1" |
| PixmapButton `link_on/off` 22x22 | `m_slewLink2Model` tr("Slew Link 2") (true) | (132,191) | "Slew Link 2" |
| Knob Bright26 | `m_dynamics1Model` tr("Dynamics 1") (0..1) | (225,26) | "Dynamics 1:" |
| Knob Bright26 | `m_dynamics2Model` | (225,147) | "Dynamics 2:" |
| Knob Bright26 | `m_dynamicSlew1Model` tr("Dynamic Slew 1") (-8..8) | (267,26) | "Dynamic Slew 1:" |
| Knob Bright26 | `m_dynamicSlew2Model` | (267,147) | "Dynamic Slew 2:" |
| Knob Bright26 | `m_attack1Model` tr("Attack 1") (2, 0.01..200) | (225,65) | "Attack 1:" |
| Knob Bright26 | `m_attack2Model` | (225,186) | "Attack 2:" |
| Knob Bright26 | `m_release1Model` tr("Release 1") (20, 0.01..800) | (267,65) | "Release 1:" |
| Knob Bright26 | `m_release2Model` | (267,186) | "Release 2:" |
| Knob Bright26 | `m_warp1Model` tr("Warp 1") (0..0.99) | (329,26) | "Warp 1:" |
| Knob Bright26 | `m_warp2Model` | (329,147) | "Warp 2:" |
| Knob Bright26 | `m_crush1Model` tr("Crush 1") (0..24) | (371,26) | "Crush 1:" |
| Knob Bright26 | `m_crush2Model` | (371,147) | "Crush 2:" |
| Knob Bright26 | `m_mix1Model` tr("Mix 1") (1, 0..1) | (338,78) | "Mix 1:" |
| Knob Bright26 | `m_mix2Model` tr("Mix 2") | (338,199) | "Mix 2:" |
| PixmapButton `dc_on/off` 82x18, checkable | `m_dcRemoveModel` tr("DC Offset Remover") (true) | (9,248) | "DC Offset Removal" |
| PixmapButton `mb_on/off` 82x18, checkable | `m_multibandModel` tr("Multiband") | (99,248) | "Multiband" |
| Knob Small17 | `m_splitModel` tr("Split") (200, 100..20000) | (190,249) | "Split:" |
| PixmapButton `oversample_1x_on/off` 26x18 (grupo) | `m_oversamplingModel` tr("Oversample") valor 0 | (454,248) | "Disable Oversampling" |
| PixmapButton `oversample_2x_*` | valor 1 | (479,248) | "2x Oversampling" |
| PixmapButton `oversample_4x_*` | valor 2 | (504,248) | "4x Oversampling" |
| PixmapButton `oversample_8x_*` | valor 3 | (529,248) | "8x Oversampling" |
| PixmapButton `oversample_16x_*` | valor 4 | (554,248) | "16x Oversampling" |
| PixmapButton `oversample_32x_*` | valor 5 | (579,248) | "32x Oversampling" |
| PixmapButton `help_on/off` 14x14 | abre `SlewDistortionHelpView` | (614,250) | "Open help window" |

- **Visores vivos:** `update()` em todo `periodicUpdate` (60 Hz).
  - **Medidores invertidos** (o artwork já tem a escala/gradiente; o código cobre a parte de cima com cinza escuro): retângulos de 8x75. Entrada: (22,31) L banda 1, (30,31) R banda 1, (22,152) L banda 2, (30,152) R banda 2. Saída: (600,31), (608,31), (600,152), (608,152). Dados `m_effect->m_inPeakDisplay[4]` / `m_outPeakDisplay[4]`; valor -1 = "sem dado novo" (mantém o anterior); piso `SLEW_DISTORTION_MIN_FLOOR`; após leitura escreve -1. Escala -24..+24 dBFS; altura coberta = `int(75 * (1 - norm))`, `fillRect` a partir do topo com `QColor(10,10,10)`. Sem queda suave (salto direto).
  - **Curvas de transferência**: `curveRect1 = (452,10,100,100)`, `curveRect2 = (452,131,100,100)`.
- **Desenho próprio (`paintEvent`)**, antialias ligado:
  - Grade: pen `QColor(36,40,48)` `Qt::DotLine` largura padrão. Para i=1..7: x = left + i*100/8 + 1 (divisão inteira) = 465, 478, 490, 503, 515, 528, 540, de y top+1 até bottom; y = top + i*100/8 + 1: retângulo 1 = 23, 36, 48, 61, 73, 86, 98; retângulo 2 = 144, 157, 169, 182, 194, 207, 219; de x left+1 (453) a right (551).
  - Eixos: pen `QColor(62,66,75)` largura 2: vertical em x = center.x + 2 = 503 (top+1..bottom); horizontal em y = center.y + 2 = 61 (ret. 1) / 182 (ret. 2), de x 453 a 551.
  - Curva: pen `QColor(34,226,108)` largura 2; 201 pontos (`numSteps = 200`), x de -1 a 1; aplica drive (dB->amp), bias, warp/crush e a função do tipo de distorção (mesmas fórmulas do DSP: hard clip, tanh, soft clip 1/2, sinusoidal por polinômio, foldback, full/smooth/half rectify, bitcrush 16 níveis/drive); escala de amplitude `(100-2)/100`; `px = left + (x+1)/2*100`, `py = bottom - (y+1)/2*100 + 1`; desenhada como QPainterPath com `setClipRect(rect)`.
- **Notas:**
  - `Draggable`: o pixmap é desenhado esticado em `rect()`; o widget fica 28x15 pelo `sizeHint` (adjustSize no show). Arrasto vertical em coordenadas do pai: `progress = (y - pointA) / (pointB - pointA)`, clamp 0..1. `handle_zero.png` aparece quando o valor é exatamente 0 (valor padrão do `setDefaultValPixmap`).
  - Link: com Slew Link ligado o knob Slew Down passa a usar o modelo de Slew Up (e Down recebe o valor de Up).
  - Help: janela `QTextEdit` (rich text) em subjanela MDI, título "Slew Distortion Help", sem botão maximizar, ícone `lmms-plugin-logo`; créditos "Plugin by Lost Robot / GUI by thismoon".
  - Não há knob para Drive/Bias/Out Vol: são só as alças `handle` arrastáveis sobre trilhos do artwork.

### SpectrumAnalyzer

- **Classificação:** visual próprio (dois visores pintados + botões com ícones SVG); **Categoria:** efeito (visualizador)
- **Arquivos:** `plugins/SpectrumAnalyzer/SaControlsDialog.cpp/.h`, `SaSpectrumView.cpp/.h`, `SaWaterfallView.cpp/.h`, `SaControls.cpp/.h`, `SaProcessor.cpp/.h`. Imagens (todas SVG, via `PLUGIN_NAME::getIconPixmap`, escaladas em código):

| Arquivo | width/height/viewBox | Uso |
|---|---|---|
| `plugins/SpectrumAnalyzer/play.svg` | 100mm x 100mm, `0 0 100 100` | Pause ativo |
| `plugins/SpectrumAnalyzer/pause.svg` | 100mm x 100mm, `0 0 100 100` | Pause inativo |
| `plugins/SpectrumAnalyzer/freeze.svg` | 100mm x 100mm, `0 0 100 100` | Freeze ativo |
| `plugins/SpectrumAnalyzer/freeze_off.svg` | 100mm x 100mm, `0 0 100 100` | Freeze inativo |
| `plugins/SpectrumAnalyzer/x_log.svg` | 100mm x 100mm, `0 0 100 100` | Log X ativo |
| `plugins/SpectrumAnalyzer/x_linear.svg` | 100mm x 100mm, `0 0 100 100` | Log X inativo |
| `plugins/SpectrumAnalyzer/y_log.svg` | 100mm x 100mm, `0 0 100 100` | Log Y ativo |
| `plugins/SpectrumAnalyzer/y_linear.svg` | 100mm x 100mm, `0 0 100 100` | Log Y inativo |
| `plugins/SpectrumAnalyzer/block_size.svg` | 100mm x 100mm, `0 0 100 100` | ícone (QLabel) ao lado do combo de bloco FFT |
| `plugins/SpectrumAnalyzer/window.svg` | 99.73542mm x 99.73542mm, `0 0 99.73542 99.73542` | ícone (QLabel) ao lado do combo de janela FFT |
| `plugins/SpectrumAnalyzer/advanced_on.svg` | 50mm x 150mm, `0 0 50 150` | botão Avançado ativo |
| `plugins/SpectrumAnalyzer/advanced_off.svg` | 50mm x 150mm, `0 0 50 150` | botão Avançado inativo |
| `plugins/SpectrumAnalyzer/advanced_src.svg` | 50mm x 150mm, `0 0 50 150` | **NÃO referenciado pelo código** (fonte Inkscape) |

  Tamanhos escalados (com DPR 1): `iconSize = 22x22`; `buttonSize = 1.2*iconSize = 26x26` (QSize*1.2, arredondado: 26.4 -> 26); `advButtonSize = (m_configHeight/3) x m_configHeight = 25x75` (`m_configHeight = 75`). Todos multiplicados por `devicePixelRatio()` e marcados com esse DPR (nítidos em HiDPI). Ícone do descritor: `lmms-plugin-logo` (tema).
- **Janela:** redimensionável. `sizeHint`: largura = 400 (sizeHint do espectro); altura = `75 + 200 + 50 = 325` sem waterfall, `75 + 200 + 350 + 50 = 675` com waterfall. Layout: `QHBoxLayout` mestre, margens 2,6,2,8, contendo um `QSplitter` vertical com 3 painéis: (1) seção de controles (altura máxima 75), (2) `SaSpectrumView`, (3) `SaWaterfallView` (visível só se `m_waterfallModel`). Ao ligar o waterfall, a subjanela cresce até a altura do sizeHint; ao desligar, encolhe para o sizeHint. Fundo: paleta do tema (sem autoFill).
- **Controles:** seção de controles = `QHBoxLayout` (margens 0) com [config_widget (stretch 10)] [advanced_widget (stretch 10, oculto)] [stretch 0] [botão Avançado]. O botão alterna: marcado mostra advanced e esconde config; desmarcado o inverso.

  Config básica (`QGridLayout`, altura máx 75; col 3 stretch 2, col 5 stretch 3):

| Widget | Modelo | Posição (linha, coluna, span) | Dica |
|---|---|---|---|
| `PixmapButton` 26x26: ativo `play`, inativo `pause` | `m_pauseModel` tr("Pause"), false | (0,0) span 2x1, AlignHCenter | tooltip "Pause data acquisition" |
| `PixmapButton` 26x26: ativo `freeze`, inativo `freeze_off` | `m_refFreezeModel` tr("Reference freeze"), false | (2,0) span 2x1, AlignHCenter | tooltip "Freeze current input as a reference / disable falloff in peak-hold mode." |
| `LedCheckBox` tr("Waterfall"), Yellow, min 100x12 | `m_waterfallModel` tr("Waterfall"), false | (0,1) | tooltip "Display real-time spectrogram" |
| `LedCheckBox` tr("Averaging"), min 100x12 | `m_smoothModel` tr("Averaging"), false | (1,1) | tooltip "Enable exponential moving average" |
| `LedCheckBox` tr("Stereo"), min 100x12 | `m_stereoModel` tr("Stereo"), false | (2,1) | tooltip "Display stereo channels separately" |
| `LedCheckBox` tr("Peak hold"), min 100x12 | `m_peakHoldModel` tr("Peak hold"), false | (3,1) | tooltip "Display envelope of peak values" |
| `PixmapButton` 22x22: ativo `x_log`, inativo `x_linear` | `m_logXModel` tr("Logarithmic frequency"), true | (0,2) span 2x1, AlignRight | tooltip "Switch between logarithmic and linear frequency scale" |
| `ComboBox` min 100x22, máx 200x22 | `m_freqRangeModel` tr("Frequency range"); itens: Full (auto), Audible, Bass, Mids, High; default "Full (auto)" | (0,3) span 2x1 | tooltip "Frequency range" |
| `PixmapButton` 22x22: ativo `y_log`, inativo `y_linear` | `m_logYModel` tr("Logarithmic amplitude"), true | (2,2) span 2x1, AlignRight | tooltip "Switch between logarithmic and linear amplitude scale" |
| `ComboBox` min 100x22, máx 200x22 | `m_ampRangeModel` tr("Amplitude range"); itens: Extended, Audible, Loud, Silent; default "Audible" | (2,3) span 2x1 | tooltip "Amplitude range" |
| `QLabel` com pixmap `block_size` 22x22 | — | (0,4) span 2x1, AlignRight | — |
| `ComboBox` min 100x22, máx 200x22 | `m_blockSizeModel` tr("FFT block size"); itens "256 (High time res.)", 512, 1024, 2048, 4096, 8192, "16384 (High freq. res.)"; default 2048 | (0,5) span 2x1 | tooltip "FFT block size" |
| `QLabel` com pixmap `window` 22x22 | — | (2,4) span 2x1, AlignRight | — |
| `ComboBox` min 100x22, máx 200x22 | `m_windowModel` tr("FFT window type"); itens Rectangular (Off), Blackman-Harris (Default), Hamming, Hanning; default Blackman-Harris | (2,5) span 2x1 | tooltip "FFT window type" |
| `PixmapButton` 25x75: ativo `advanced_on`, inativo `advanced_off`, checável, SEM modelo | — | extrema direita da seção, após stretch | tooltip "Access advanced settings" |

  Config avançada (`QGridLayout`, altura máx 75, todos `Knob` Small17 com rótulo em `SMALL_FONT_SIZE` (10 px), AlignCenter):

| Widget | Modelo | Posição (linha, col) | Dica |
|---|---|---|---|
| Knob Small17 tr("Envelope res.") | `m_envelopeResolutionModel` tr("Peak envelope resolution"), 0.25, 0.1..3.0, 0.05 | (0,0) | hint "Maximum number of envelope points drawn per pixel:" ""; tooltip "Increase envelope resolution for better details, decrease for better GUI performance." |
| Knob Small17 tr("Spectrum res.") | `m_spectrumResolutionModel` tr("Spectrum display resolution"), 1.5, 0.1..3.0, 0.05 | (1,0) | hint "Maximum number of spectrum points drawn per pixel:"; tooltip "Increase spectrum resolution for better details, decrease for better GUI performance." |
| Knob Small17 tr("Falloff factor") | `m_peakDecayFactorModel` tr("Peak decay multiplier"), 0.992, 0.95..0.999, 0.001 | (0,1) | hint "Multiply buffered value by"; tooltip "Decrease to make peaks fall faster." |
| Knob Small17 tr("Averaging weight") | `m_averagingWeightModel` tr("Averaging weight"), 0.15, 0.01..0.5, 0.01 | (1,1) | hint "New sample contributes"; tooltip "Decrease to make averaging slower and smoother." |
| Knob Small17 tr("Waterfall height") | `m_waterfallHeightModel` tr("Waterfall history size"), 300, 50..1000, 50 | (0,2) | hint "Number of lines to keep:"; tooltip "Increase to get slower scrolling, decrease to see fast transitions better. Warning: medium CPU usage." |
| Knob Small17 tr("Waterfall gamma") | `m_waterfallGammaModel` tr("Waterfall gamma correction"), 0.30, 0.10..1.00, 0.05 | (1,2) | hint "Gamma value:"; tooltip "Decrease to see very weak signals, increase to get better contrast." |
| Knob Small17 tr("Window overlap") | `m_windowOverlapModel` tr("FFT window overlap"), 2, 1..4, 1 | (0,3) | hint "Number of times each sample is processed:"; tooltip "Increase to prevent missing fast transitions arriving near FFT window edges. Warning: high CPU usage." |
| Knob Small17 tr("Zero padding") | `m_zeroPaddingModel` tr("FFT zero padding"), 2, 0..4, 1 | (1,3) | hint "Processing buffer is" + sufixo " steps larger than input block"; tooltip "Increase to get smoother-looking spectrum. Warning: high CPU usage." |

- **Visores vivos:**
  - `SaSpectrumView`: mínimo 360x170, sizeHint 400x200, política Minimum. Retângulo do gráfico: top = 1, bottom = `h - 20`, left = 26, right = `w - 26` (margens laterais de 26 px para rótulos de amplitude; 20 px embaixo para rótulos de frequência). Dados: espectro L/R já em escala de exibição vindo do `SaProcessor` (FFT do bloco escolhido, janela, overlap, zero-padding). Buffers de exibição: com "Averaging", `disp = spec*w + disp*(1-w)`; sem, cópia direta; pausado não atualiza. Peak hold: `peak = max(spec, peak*decay)`; freeze copia o buffer atual como referência e congela. Caminho: preenchido de baixo, um ponto por pixel (resolução = `spectrumRes * DPR` pontos por pixel; usa o valor máximo dentro do pixel). Mono: `fillPath(pathL, colorMono)`. Estéreo: R primeiro com `colorR`, depois L com `colorL` (alfa 135 se somam). Envoltória de pico/referência: `drawPath` com pen 1 px na cor do canal (`colorL` / `colorR`; mono usa `colorL`). 60 fps.
  - Escalas X: log: `(log10 f - log10 min)/(log10 max - log10 min) * largura`; linear proporcional. Faixas: Full = 5 Hz (log; 0 em linear) .. Nyquist; Audible 20..20000; Bass 20..300; Mids 200..5000; High 4000..20000. Escalas Y (dB): Extended -80..+20; Audible -50..0; Loud -30..0; Silent -60..-10; linear: converte para 10^(dB/10).
  - Marcas de frequência log: séries 10, 20, 50 × 10^n (texto "k" a partir de 1000, ex "1k", "2k", "5k", "10k", "20k"); se a faixa < 2 décadas, adiciona 14, 30, 70 × 10^n (com "k" a partir de 1500). Linear: passo 50 (faixa < 500), 100 (< 1000), 1000 (< 5000), senão 2000. Marcas de amplitude log: passo 3 dB se faixa < 20*heightScale, 6 dB se < 45*heightScale, senão 10 dB (`heightScale = h / 200`), rótulo inteiro em dB. Linear: ~5 rótulos (10 se `h/200 >= 1.5`), de 0.001 a 999.
  - `SaWaterfallView`: mínimo 300x150, sizeHint 400x350, Expanding. Retângulo: top 1, bottom `h - 2`, left 26, right `w - 26`. Dados: imagem `QImage` RGB32 de `waterfallWidth x waterfallHeight` (largura = bins mapeados, altura = histórico de linhas, padrão 300) pré-computada no `SaProcessor`; nova linha entra no topo e rola para baixo. Cor do pixel: estéreo `rgb = colorL*ampL^γ + colorR*ampR^γ`; mono `colorMonoW * ampL^γ` (γ = `m_waterfallGammaModel`). Linha inteira em `qRgb(42,0,0)` (vermelho escuro) quando a CPU não acompanha (overload). Escalada com `SmoothTransformation` para o retângulo. Sem dados: `fillRect` preto `QColor(0,0,0)`. 60 fps; informa ao processador se está visível.
- **Desenho próprio:**
  - Cores (definidas no código, `SaControls.cpp`, não no tema): `colorL (51,148,204,135)`, `colorR (204,107,51,135)`, `colorMono (51,148,204,204)`, `colorMonoW (64,185,255,255)`, `colorBG (7,7,7,255)`, `colorGrid (30,34,38,255)`, `colorLabels (192,202,212,255)`.
  - Espectro, ordem: `fillRect(left, top, width, bottom, colorBG)`; linhas verticais de frequência de y = 2 a bottom, pen `colorGrid` 1 px; rótulos de frequência caixa 24x15 em y = bottom + 5, centrados na linha (primeiro alinhado à esquerda em `left` se não couber; último alinhado à direita em `right - 24`), cor `colorLabels`; linhas horizontais de amplitude de `left+1` a `right-1`, `colorGrid` 1 px; rótulos de amplitude duplicados nos dois lados: caixa 24x15, à esquerda em `x = left - 24 - 2` alinhado à direita, à direita em `x = right + 2` alinhado à esquerda, centrados verticalmente na linha (o mais alto, se `pos < 8`, ancorado no topo em `top - 2`; o mais baixo, se perto do fundo, ancorado em `bottom - 15 + 2`). Em estéreo, rótulos esquerdos na cor `colorL.lighter()` e direitos `colorR.lighter()`. Depois o espectro, o cursor e por fim o contorno: `drawRoundedRect(left, 1, width, bottom, 2, 2)` pen `colorGrid` 2 px. Antialias ligado. Fonte padrão do widget.
  - Cursor (espectro e waterfall, ao clicar/arrastar dentro do retângulo): cruz de linhas 1 px `colorGrid.lighter()` em toda a altura/largura do retângulo; caixa em `(left+5, top+5)`, fundo `QColor(0,0,0,64)`, margem interna 3, largura = largura do texto "-99.9 dBFS" (espectro) ou "20000 Hz " (waterfall) + 6, altura = 2*(altura da linha + 3). Texto em `colorLabels`: linha 1 "%1 Hz"; linha 2 "x.xxx dBFS" (log Y), número linear com 3 casas (linear Y), ou "x.xxx s" (waterfall).
  - Waterfall: rótulos de tempo (s) nos dois lados, caixa 20x16, margem 2, cor `colorLabels`, passo ≈ 30 px (mín 0.1 s), texto com 3 caracteres; contorno `drawRoundedRect(left, top, width, height, 2, 2)` pen `colorGrid` 2 px.
- **Notas:** o botão Avançado é um `PixmapButton` alto e estreito (25x75) na borda direita; o `QSplitter` permite o usuário redimensionar espectro/waterfall. `advanced_src.svg` é só fonte. Os pixmaps são `static` (calculados uma vez com o DPR da primeira janela).

### StereoMatrix

- **Classificação:** visual próprio (artwork.svg com diagrama de matriz); **Categoria:** efeito
- **Arquivos:** `plugins/StereoMatrix/StereoMatrixControlDialog.cpp/.h`, `StereoMatrixControls.cpp/.h`. Imagem: `plugins/StereoMatrix/artwork.svg` (width="160" height="185", 4010 bytes).
- **Janela:** `setFixedSize(160, 185)`. Artwork, em ordem de pintura:
  1. listras verdes `#2c974f` verticais 11.2 px de largura em x 0, 37.2, 74.4, 111.6, 148.8 (altura toda) — **cobertas** pelo retângulo de gradiente pintado em seguida (invisíveis; resto de edição).
  2. gradiente base 160x185.
  3. linhas 3 px, join round: vermelha `#d40237` (cap round) `M98.6 129.7 L135.8 92.5 L98.6 55.3 V92.5 L61.4 129.7`; azul `#2784d5` (cap round) `M61.4 129.7 L24.2 92.5 L61.4 55.3 V92.5 L98.6 129.7`; "chaveta" superior cinza `#5a6776` de (61,42.5) até (99,42.5) com pico em (79.8,25.9).
  4. quadrados 13x13 (rx 2.065, stroke 3 da mesma cor): azul `#2784d5` em (54.5, 42.5) e (54.5, 129.5); vermelho `#d40237` em (92.5, 42.5) e (92.5, 129.5); chaveta inferior cinza `#5a6776` de (61,142.5) a (99,142.5) com pico para baixo em (79.8,159.1).
  5. texto vetorizado branco `#fff`: "IN" no topo (x ≈ 73..87, y 9.8..19.4); "L" dentro do quadrado azul superior (x 56.2..65.8, y 44.2..53.8) e "R" no vermelho superior (x 94.2..103.8); de novo "L"/"R" nos quadrados inferiores (y 131.2..140.8); "OUT" embaixo (x ≈ 64.7..96.7, y 165.4..175).
  6. quatro círculos (raio 13) de gradiente base como "berço" de knob, centros (24.2, 92.5), (61.4, 92.5), (98.6, 92.5), (135.8, 92.5).
- **Controles:** `QHBoxLayout` (margens padrão), cada knob `AlignHCenter`, sem rótulo (construtor `Knob(KnobType::Bright26, this)`). Não há `move()`: a posição real vem do layout, distribuindo 4 knobs de 26 px na largura de 160 e centrados verticalmente em 185 (y do knob ≈ (185-26)/2 ≈ 79.5, coincide com os círculos em y 79.5..105.5). Posições pretendidas pela arte (canto superior esquerdo do knob 26x26): (11.2, 79.5), (48.4, 79.5), (85.6, 79.5), (122.8, 79.5).

| Widget | Modelo | Posição pretendida (centro) | Dica |
|---|---|---|---|
| `Knob` Bright26, sem rótulo | `m_llModel` tr("Left to Left"), 1.0, -1..1 | 1o, centro (24.2, 92.5) | "Left to Left Vol:" "" |
| `Knob` Bright26 | `m_lrModel` tr("Left to Right"), 0.0, -1..1 | 2o, (61.4, 92.5) | "Left to Right Vol:" "" |
| `Knob` Bright26 | `m_rlModel` tr("Right to Left"), 0.0, -1..1 | 3o, (98.6, 92.5) | "Right to Left Vol:" "" |
| `Knob` Bright26 | `m_rrModel` tr("Right to Right"), 1.0, -1..1 | 4o, (135.8, 92.5) | "Right to Right Vol:" "" |

- **Visores vivos / Desenho próprio:** nenhum.
- **Notas:** leitura do diagrama: entrada L (azul, em cima à esquerda) desce em diagonal para os knobs LL (24.2) e, pela linha vertical em x 61.4, para LR; entrada R (vermelha) para RR (135.8) e RL (98.6); as diagonais cruzam até as saídas L/R embaixo. Na porta, fixar os knobs nos centros dos círculos em vez de depender do layout.

### Vectorscope

- **Classificação:** visual próprio (visor pintado `VectorView`); **Categoria:** efeito (visualizador)
- **Arquivos:** `plugins/Vectorscope/VecControlsDialog.cpp/.h`, `plugins/Vectorscope/VectorView.cpp/.h`, `VecControls.cpp/.h`. Nenhuma imagem; nenhum SVG. Ícone do descritor: `lmms-plugin-logo` (tema).
- **Janela:** `sizeHint` 275x300 (sem tamanho fixo, redimensionável). `QVBoxLayout` margens 0. Topo: `VectorView` (mínimo 200x200, sizeHint 300x300, `Expanding`). Embaixo: `QHBoxLayout` (margens padrão). Fundo: não define autoFill nem paleta (fundo do subwindow/tema).
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| `LedCheckBox` tr("Log. scale"), cor Yellow (padrão) | `m_logarithmicModel` tr("Logarithmic scale"), default false | linha inferior, à esquerda | tooltip "Display amplitude on logarithmic scale to better see small values." |
| (stretch) | | entre os dois | |
| `LedCheckBox` tr("Lines"), Yellow | `m_linesModeModel` tr("Lines rendering"), default true | linha inferior, à direita | tooltip "Render with lines." |

- **Visores vivos:** `VectorView`. Fonte: `LocklessRingBuffer<SampleFrame>` do efeito (`getBuffer()`); a cada repaint lê tudo o que chegou (`read_max(capacity)`). Atualiza a 60 fps se visível. Antialias ligado.
  - Para cada quadro: se log, `d = sqrt(L²+R²)`, fator `log10(1+9|d|)/d`. Ponto = `(side, -mid)` com `mid = L+R`, `side = L-R`. Cor do traço = `colorTrace.darker(100 + índiceDoQuadro)` (escurece progressivamente dentro do bloco). Largura `2/(scale*zoom)` (≈2 px na tela). Modo linhas: linha do ponto anterior ao atual (só se mudou); senão `drawPoint`. Composição `CompositionMode_Plus`.
  - Transformações: `scale = min(w,h)/4.1`; origem em `(w/2, min(w,h)/2)`; Y invertido; traço também multiplicado por `m_zoom`.
- **Desenho próprio:** fundo `fillRect(rect(), Qt::black)` (preto fixo). Grade (composição normal, coords da grade): círculo de raio 2 em torno da origem, pen `colorGrid` largura `2.5/scale` (≈2.5 px), sólida, RoundCap/BevelJoin; duas linhas pontilhadas (`Qt::DotLine`) da origem até `(-sqrt(2.1), sqrt(2.1))` e `(sqrt(2.1), sqrt(2.1))` (diagonais a 45° para cima-esquerda e cima-direita). Rótulos "L" e "R": fonte do painter ajustada para 26 px, negrito, cor `colorLabels`; "L" girado -45°, "R" girado +45°, centrados horizontalmente e transladados `-(min(w,h)/2) - 10` a partir do centro (ficam fora do círculo, nas pontas das diagonais). Info de zoom: durante 1000 ms após girar a roda, texto tr("Zoom: %1 %") centrado em baixo, baseline em `h - descent - 2`, cor `colorLabels`, fonte padrão. Cores padrão (código e tema `style.css:1206`, iguais): trace `rgba(60,255,130,255)`, grid `rgba(76,80,84,128)`, labels `rgba(76,80,84,255)`.
- **Notas:** roda = zoom 20..1000 %, passo `angleDelta/6` (20 % por clique de 15°). Duplo clique abre `ColorChooser` para trocar a cor do traço (easter egg; não é salvo no projeto).

### WaveShaper

- **Classificação:** visual próprio (artwork + Graph com fundo próprio + PixmapButtons) ; **Categoria:** efeito
- **Arquivos:** `plugins/WaveShaper/WaveShaperControlDialog.cpp/.h`, `WaveShaperControls.cpp`.
  - Imagens (todas referenciadas): `artwork.png` 224x274; `wavegraph.png` 204x205; `reset_active/inactive.png` 46x13; `smooth_active/inactive.png` 46x13; `add1_active/inactive.png` 29x13; `sub1_active/inactive.png` 29x13. Sem SVG.
- **Janela:** `setFixedSize(224, 274)`, fundo `artwork.png`.
- **Controles:**

| Widget | Modelo | Posição | Dica |
|---|---|---|---|
| Graph `LinearNonCyclic` 204x205, fundo `wavegraph.png`, cor `QColor(85, 204, 145)` | `m_wavegraphModel` (0..1, 200 amostras) | (10,6) | — |
| VolumeKnob Bright26, rótulo tr("INPUT"), SMALL_FONT_SIZE, zeroDbfs 1 | `m_inputModel` tr("Input gain") (1, 0..5) | (26,225) | "Input gain:" "" |
| VolumeKnob Bright26, rótulo tr("OUTPUT") | `m_outputModel` tr("Output gain") (1, 0..5) | (76,225) | "Output gain:" "" |
| PixmapButton `reset_*` 46x13 | `resetClicked()` | (162,221) | "Reset wavegraph" |
| PixmapButton `smooth_*` 46x13 | `smoothClicked()` | (162,237) | "Smooth wavegraph" |
| PixmapButton `add1_*` 29x13 | `addOneClicked()` | (131,221) | "Increase wavegraph amplitude by 1 dB" |
| PixmapButton `sub1_*` 29x13 | `subOneClicked()` | (131,237) | "Decrease wavegraph amplitude by 1 dB" |
| LedCheckBox texto "Clip input", nome tr("Clip input"), LedColor::Green | `m_clipModel` (false, sem nome) | (131,252) | "Clip input signal to 0 dB" |

- **Visores vivos:** só o Graph editável.
- **Desenho próprio:** nenhum.
- **Notas:** mesmos padrões do DynamicsProcessor (resizes anulados). O texto do LED "Clip input" não passa por tr(). `m_clipModel` não tem display name.

## Genéricos / botões soltos

Usam o editor genérico (I-9); nada a portar do visual.

### AmpAndIr

- **Classificação:** genérico / botões soltos. **Categoria:** efeito.
- **Arquivos:** `plugins/AmpAndIr/AmpAndIrControls.{h,cpp}`. **Nenhuma imagem própria**: o CMakeLists não tem `EMBEDDED_RESOURCES`, o descritor usa o logo genérico do tema (`PixmapLoader("lmms-plugin-logo")`). Não existe SVG próprio.
  - `plugins/AmpAndIr/NeuralAmpModelerCore/media/tone3000-logo.svg` (width="210" height="32" viewBox="0 0 210 32") está no diretório vendorizado do NeuralAmpModelerCore (MIT) e é referenciado **apenas** pelo `README.md` desse vendor (`<img src="media/tone3000-logo.svg">`). **Não é usado** pelo código nem pelo build do plugin. É logo de terceiro (TONE3000): sem permissão registrada para uso na interface (sem permissão registrada); como não é usado, nada a portar.
- **Janela:** `QGridLayout`, margens 8,8,8,8, espaçamento horizontal 8 (vertical = padrão), `SetFixedSize` (tamanho resultante do layout). Sem fundo próprio.
- **Controles:**

  | Widget | Modelo (rótulo) | Posição (célula do grid) | Dica |
  |---|---|---|---|
  | `Knob` `KnobType::Bright26`, rótulo "IN" | `m_inputModel` tr("Input gain"), -24..24 dB, passo 0.1, padrão 0 | linha 0, coluna 0, ocupa 2 linhas x 1 col | "Input gain:" + " dB" |
  | `QLabel` (nome do modelo), largura mínima 150 | texto "No amp model" ou nome base do .nam | linha 0, coluna 1 | - |
  | `QPushButton` "Amp model..." | abre menu: "From a file (.nam)...", "Find amp models online (TONE3000)..." (abre https://www.tone3000.com), "None" (só se houver modelo) | linha 1, coluna 1 | - |
  | `QLabel` (nome do IR), largura mínima 150 | "No impulse response" ou nome base | linha 0, coluna 2 | - |
  | `QPushButton` "Impulse response..." | menu: um submenu por pasta de `factorySamplesDir()/impulse-responses` (cada .wav), separador, "From a file...", "None" (se houver IR) | linha 1, coluna 2 | - |
  | `Knob` `Bright26`, rótulo "OUT" | `m_outputModel` tr("Output gain"), -24..24 dB, padrão 0 | linha 0, coluna 3, 2 linhas x 1 col | "Output gain:" + " dB" |

  Ordem: IN, amp, IR, OUT ("the signal's way"). Erros em `QMessageBox::warning("Amp & IR", ...)`.
- **Visores vivos / desenho próprio:** nenhum.

### Dispersion

- **Classificação:** genérico / botões soltos; **Categoria:** efeito
- **Arquivos:** `plugins/Dispersion/DispersionControlDialog.cpp/.h`, `DispersionControls.cpp/.h`. Nenhuma imagem; nenhum SVG.
- **Janela/Controles:** `setAutoFillBackground(true)` sem paleta (fundo do tema `#262b30`); `QHBoxLayout`, spacing 5, `SetFixedSize`. Da esquerda para a direita: `LcdSpinBox` 3 dígitos (estilo padrão), rótulo tr("AMOUNT"), modelo `m_amountModel` tr("Amount") inteiro 0..999 (`MAX_DISPERSION_FILTERS = 999`), tooltip "Number of all-pass filters"; `Knob` Bright26 tr("FREQ") -> `m_freqModel` tr("Frequency") 200, 20..20000, dica "Frequency:" tr("Hz"); `Knob` Bright26 tr("RESO") -> `m_resoModel` tr("Resonance") 0.707, 0.01..8, dica "Resonance:" tr("octaves"); `Knob` Bright26 tr("FEED") -> `m_feedbackModel` tr("Feedback") 0, -1..1, dica "Feedback:" ""; `AutomatableButton` checável com texto tr("DC"), objectName `btn` (estilo de botão do tema), modelo `m_dcModel` tr("DC Offset Removal") false, tooltip "Remove DC Offset".

### StereoEnhancer

- **Classificação:** genérico / botões soltos; **Categoria:** efeito
- **Arquivos:** `plugins/StereoEnhancer/StereoEnhancerControlDialog.cpp/.h`, `StereoEnhancerControls.cpp/.h`. Nenhuma imagem.
- **Janela/Controles:** sem fundo próprio; `QHBoxLayout` `SetFixedSize` com um único `Knob` Bright26 tr("WIDTH") -> `m_widthModel` tr("Width") 0, 0..180, passo 1, dica "Width:" " samples".

### TapTempo

- **Classificação:** genérico / botões soltos (widgets Qt nativos, sem arte); **Categoria:** ferramenta (janela de ferramenta, `ToolPluginView`)
- **Arquivos:** `plugins/TapTempo/TapTempoView.cpp/.h`, `TapTempo.cpp/.h`. Imagem: `plugins/TapTempo/logo.png` (20x20), usada só como ícone do descritor (`PluginPixmapLoader("logo")`, menu de ferramentas), não na janela.
- **Janela:** tamanho fixo pelo layout (`SetFixedSize`, `QSizePolicy::Fixed`); flags do subwindow: `MSWindowsFixedSizeDialogHint`, sem botão maximizar. Começa oculta. Fundo: paleta do tema. Foco forte (StrongFocus) na view: barra de espaço dispara `animateClick()` no botão Tap.
- **Controles:** `QVBoxLayout` principal (margens padrão):
  1. `QPushButton` Tap, 200x200 fixo, fonte padrão ajustada a 24 px, texto inicial tr("Tap to begin"); depois de tocar mostra o BPM (0 casas, ou 1 com Precision). `AlignCenter`. NoFocus.
  2. `QHBoxLayout`: `QPushButton` tr("Reset") (tooltip "Reset counter and sidebar information") e `QPushButton` tr("Sync") (tooltip "Sync with project tempo"), cada um `AlignCenter`.
  3. `QHBoxLayout`: à esquerda `QVBoxLayout` com `QCheckBox` tr("Precision") (tooltip "Display in high precision") e `QCheckBox` tr("Mute") (tooltip "Mute metronome"); à direita `QVBoxLayout` com `QLabel` ms (inicial tr("0 ms"), tooltip "BPM in milliseconds", `AlignHCenter`) e `QLabel` Hz (inicial tr("0.0000 Hz"), tooltip "Frequency of BPM", `AlignHCenter`).
  Sem modelos automatizáveis (estado só na view/plugin). Textos atualizados: ms = 60000/bpm com 0 ou 1 casa; Hz = bpm/60 com 4 casas. Fechar a janela faz reset.
- **Notas:** usa `QCheckBox`/`QPushButton` nativos, não `LedCheckBox`/`PixmapButton`.

## Filtros de arquivo, sem janela

MidiImport e MidiExport são filtros de importação e exportação (F-FX-038, F-FX-039, descritos em F-PRJ), não janelas de plugin.

### MidiImport

- **Classificação:** sem janela própria; **Categoria:** filtro de importação (`Plugin::Type::ImportFilter`)
- **Arquivos:** `plugins/MidiImport/MidiImport.cpp/.h` (+ `portsmf/`). Nenhuma imagem.
- **Janela:** nenhuma view de plugin. O arquivo é escolhido pelo app: `MainWindow::onImportProject()` abre `FileDialog` título tr("Import file"), diretório de projetos do usuário, filtros "MIDI sequences (*.mid *.midi *.rmi);;All file types (*.*)", modo ExistingFiles; também pelo navegador de arquivos (`FileBrowser.cpp:925`).
- **Diálogos do próprio plugin:** (1) `UserNotice::information` título tr("Setup incomplete"), quando não há SoundFont padrão configurada (texto pedindo configurar em Edit->Settings) ou, sem FluidSynth compilado, aviso de que não haverá som; (2) `UserNotice::Progress` "Please wait..." / "Importing MIDI-file..." durante a leitura (barra de progresso por trilha). Faixas criadas recebem nome tr("Track") + " N".

### MidiExport

- **Classificação:** sem janela própria; **Categoria:** filtro de exportação (`Plugin::Type::ExportFilter`, sem ícone: pixmap `nullptr`)
- **Arquivos:** `plugins/MidiExport/MidiExport.cpp/.h`, `MidiFile.cpp/.h`. Nenhuma imagem.
- **Janela:** nenhuma; nenhum diálogo de opções nem `UserNotice` no plugin. O app abre `FileDialog` (`MainWindow.cpp` ~1775-1801): título tr("Select file for project-export..."), modo AcceptSave, sufixo padrão "mid", nome sugerido = nome do projeto ou tr("untitled") + ".mid"; depois chama `Song::exportProjectMidi()`. Se o filtro não carregar, só registra erro em `ErrorLog` (sem diálogo).


## Imagens sem uso e procedência

**Sem uso no código** (não precisam ir para `lm-gui-plugins/assets/`):

- `plugins/Vibed/button_up.png`, `plugins/Vibed/button_down.png` (14×14).
- `plugins/Xpressive/Xpressive.svg`, `plugins/Xpressive/Xpressive_logo.svg` (fontes vetoriais da arte).
- `plugins/LOMM/split1Enabled_{active,inactive}.png`, `plugins/LOMM/split2Enabled_{active,inactive}.png` (29×19);
  os botões de divisão usam `crossover_led_*`.
- `plugins/SpectrumAnalyzer/advanced_src.svg` (fonte Inkscape do botão Avançado).
- `plugins/AmpAndIr/NeuralAmpModelerCore/media/tone3000-logo.svg` (só o README do código vendorizado cita).
- No `plugins/StereoMatrix/artwork.svg`, as listras verdes `#2c974f` ficam cobertas pelo gradiente (invisíveis).
- No tema, `led_blue.png` não é usado pelo `LedCheckBox`.
- Ao contrário, o `GranularPitchShifter` pede `PLUGIN_NAME::getIconPixmap("logo")` para o ícone da janela de ajuda, e a pasta não tem `logo.png` (o ícone sai vazio).

**Procedência das imagens de terceiros:**

- **AmplifikationLite** (`background`, `silverknob`, `silverswitch`, `irload`, `logo`, `about`): arte da Kuassa,
  uso permitido pelo autor em 18/11 (memória do projeto, `amplifikation-blackbox`). O README do plugin recomenda
  guardar essa permissão por escrito.
- **Guitarix** (`gx/rcstyles/*.png`, as 21 usadas): do Guitarix original, GPL-3.0+ (`gx/COPYING`); fonte Roboto
  Condensed, Apache 2.0. Não há nota de procedência por imagem.
- **AvlDrums** (`data/avldrums/images/`): da interface do avldrums.lv2 (x42), GPL-2.0, registrado em
  `data/avldrums/SOURCE.txt`.
- **DrumMachines** (`photos/*.jpg`): Wikimedia Commons, CC BY/BY-SA ou domínio público, cada uma com autor e
  licença em `photos/CREDITS.md`; manter a atribuição e, nas BY-SA, o share-alike.
- **TONE3000** (`tone3000-logo.svg`): **sem permissão registrada**; não é usado, então não entra.
- Todas as demais imagens são do próprio LMMS (GPL, como o resto).

## Terceiros fora do processo

Estes hospedam plugins de terceiros. No programa novo eles rodam **fora do processo**, no processo auxiliar do
I-9.6, e abrem a **janela do próprio plugin** (`decisoes-autonomas.md`: P-01 VST2, P-02 Carla, P-03 LADSPA, P-05
ZynAddSubFX). Não há painel a portar. O painel de hospedeiro do legado (carregar, presets, mostrar a janela) vira
interface comum do host, não arte de plugin; a arte desses painéis **não** é portada.

| Pasta | O que era no legado | Arte do legado | No programa novo |
|---|---|---|---|
| `CarlaBase` | Base do Carla Rack e do Carla Patchbay (`Carla.cpp`): painel com fundo, botão "Show GUI" (ícone `zoom`) e janela de parâmetros (`controller`, filtros `edit_erase`, `automation`) | `artwork-rack.png` 250×250, `artwork-patchbay.png` 250×250, `logo.png` | **obsoleto** (P-02): o auxiliar hospeda LV2, CLAP, VST2, VST3 e LADSPA diretamente |
| `CarlaPatchbay` | Descritor do Carla Patchbay sobre `CarlaBase` | `logo.png` | **obsoleto** (P-02) |
| `CarlaRack` | Descritor do Carla Rack sobre `CarlaBase` | `logo.png` | **obsoleto** (P-02) |
| `LadspaEffect` | Efeitos LADSPA (conjuntos `calf`, `caps`, `cmt`, `swh`, `tap` na pasta). Janela gerada: `QGroupBox` "Channel N" por canal, controles por `LadspaWidgetFactory` (`LedCheckBox` verde não legado, barras `BarModelEditor`/`TempoSyncBarModelEditor`), "Link Channels"; `LadspaMatrixControlDialog` em matriz | só `logo.png` | fora do processo (P-03); os parâmetros aparecem no editor genérico (I-9) |
| `Lv2Effect` | Efeitos LV2 (`Lv2FxControlDialog`, base `Lv2ViewBase` em `src/gui/`): controles gerados e botão da interface própria do plugin | só `logo.png` | fora do processo (F-FX-032); janela do plugin ou editor genérico |
| `Lv2Instrument` | Instrumentos LV2 (`Lv2InsView`, mesma base) | só `logo.png` | fora do processo (F-HOST-003) |
| `Vestige` | Instrumentos VST2. `VestigeInstrumentView::paintEvent` desenha `artwork.svg` (250×250) em (0, 0) e o nome do plugin em negrito 12 px; botões abrir plugin (216, 81), controles (216, 101), presets (190/209/228, 201), abrir/salvar preset (200, 224)/(224, 224) | `artwork.svg`, `select_file*.svg`, `controls*.svg`, `logo*.svg` | fora do processo (P-01), janela do plugin |
| `VstBase` | Biblioteca do hospedeiro VST (`VstPlugin.cpp`, `RemoteVstPlugin*`), sem janela própria | nenhuma | substituída pelo auxiliar (P-01) |
| `VstEffect` | Efeitos VST2 (`VstEffectControlDialog`): botões de controles e presets | `controls*.svg`, `logo.svg` | fora do processo (P-01, F-FX-033) |
| `ZynAddSubFx` | ZynAddSubFX: painel do LMMS com `artwork.png` (250×250) e botão "Show GUI"; a interface real é FLTK, num processo à parte (`RemoteZynAddSubFx`), com os pixmaps de `zynaddsubfx/pixmaps/` | `artwork.png`, `logo.png`, pixmaps do FLTK | fora do processo (P-05), janela FLTK do próprio Zyn presa à faixa |

## Resumo

Contagem das 62 pastas: **46 com visual próprio** (22 instrumentos, 24 efeitos), **4 genéricos** (AmpAndIr,
Dispersion, StereoEnhancer, TapTempo), **2 filtros sem janela** (MidiImport, MidiExport), **10 de terceiros fora do
processo** (3 deles, os do Carla, obsoletos). Dos 46 com visual próprio, Amplifier e StereoMatrix são obsoletos no
navegador e existem só para abrir projetos antigos; o painel deles vale para esses projetos.

| Plugin | Categoria | Classificação | Arquivo(s) principal(is) | Estado do port |
|---|---|---|---|---|
| AmpAndIr | efeito | genérico / botões soltos | `AmpAndIrControls.cpp` | plugin existe (lm-plugins-ampandir); genérico (sem painel; o editor genérico precisa escolher os arquivos `ir` e `nam`) |
| Amplifier | efeito | visual próprio (`artwork.svg`); obsoleto no navegador (F-FX-005), existe para abrir projetos antigos | `AmplifierControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `amplifier`) |
| AmplifikationLite | efeito | visual próprio (arte da Kuassa, permitida em 18/11) | `AmplifikationLiteControls.cpp`, `background.png`, `silverknob.png`, `silverswitch.png`, `irload.png`, `logo.png`, `about.png` | falta o plugin |
| AudioFileProcessor | instrumento | visual próprio | `AudioFileProcessorView.cpp`, `AudioFileProcessorWaveView.cpp`, `artwork.png` | falta o plugin |
| AvlDrums | instrumento | visual próprio (foto e mapa de peças) | `AvlDrums.cpp`, `data/avldrums/images/*.png` | falta o plugin |
| BassBooster | efeito | visual próprio (`artwork.svg`) | `BassBoosterControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `bassbooster`) |
| BitInvader | instrumento | visual próprio | `BitInvader.cpp`, `artwork.png`, `wavegraph.png` | plugin existe (lm-plugins-instruments): painel a fazer |
| Bitcrush | efeito | visual próprio (`artwork.svg`) | `BitcrushControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `bitcrush`) |
| CarlaBase | instrumento (hospedeiro) | terceiros fora do processo; obsoleto (P-02) | `Carla.cpp` | obsoleto |
| CarlaPatchbay | instrumento (hospedeiro) | terceiros fora do processo; obsoleto (P-02) | `CarlaPatchbay.cpp` | obsoleto |
| CarlaRack | instrumento (hospedeiro) | terceiros fora do processo; obsoleto (P-02) | `CarlaRack.cpp` | obsoleto |
| Compressor | efeito | visual próprio | `CompressorControlDialog.cpp`, `controlsBox.png` e botões | plugin existe (lm-plugins-effects): painel a fazer |
| CrossoverEQ | efeito | visual próprio (`artwork.svg`) | `CrossoverEQControlDialog.cpp`, `artwork.svg` | plugin existe (lm-plugins-effects): painel a fazer |
| Delay | efeito | visual próprio | `DelayControlsDialog.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Dispersion | efeito | genérico / botões soltos | `DispersionControlDialog.cpp` | plugin existe (lm-plugins-effects); genérico (sem painel) |
| DrumMachines | instrumento | visual próprio (fotos ou painel desenhado e pads) | `DrumMachines.cpp`, `photos/*.jpg` | falta o plugin |
| DualFilter | efeito | visual próprio (`artwork.svg`) | `DualFilterControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `dualfilter`) |
| DynamicsProcessor | efeito | visual próprio | `DynamicsProcessorControlDialog.cpp`, `artwork.png`, `wavegraph.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Eq | efeito | visual próprio | `EqControlsDialog.cpp`, `EqCurve.cpp`, `EqSpectrumView.cpp`, `EqParameterWidget.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Flanger | efeito | visual próprio (`artwork.svg`) | `FlangerControlsDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `flanger`) |
| FreeBoy | instrumento | visual próprio | `FreeBoy.cpp`, `artwork.png` | plugin existe (lm-plugins-freeboy): painel a fazer |
| FrequencyShifter | efeito | visual próprio | `FrequencyShifterControlDialog.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| GigPlayer | instrumento | visual próprio | `GigPlayer.cpp`, `artwork.png` | falta o plugin |
| GranularPitchShifter | efeito | visual próprio | `GranularPitchShifterControlDialog.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Guitarix | efeito | visual próprio (97 unidades em rack) | `GuitarixControlDialog.cpp`, `GuitarixLayout.cpp`, `GuitarixWidgets.cpp`, `GuitarixUnits.cpp`, `gx/rcstyles/*.png` | falta o plugin |
| Kicker | instrumento | visual próprio | `Kicker.cpp`, `artwork.png` | **painel feito** (`panels/kicker.rs`, teste `kicker`; o knob de duração sem a sincronia de andamento, que o plugin novo não tem) |
| LOMM | efeito | visual próprio | `LOMMControlDialog.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| LadspaEffect | efeito (hospedeiro) | terceiros fora do processo (P-03) | `LadspaControlDialog.cpp` | fora (terceiros) |
| Lb302 | instrumento | visual próprio | `Lb302.cpp`, `artwork.png` | **painel feito** (`panels/lb302.rs`, teste `lb302`) |
| Lv2Effect | efeito (hospedeiro) | terceiros fora do processo | `Lv2FxControlDialog.cpp` | fora (terceiros) |
| Lv2Instrument | instrumento (hospedeiro) | terceiros fora do processo | `Lv2Instrument.cpp` | fora (terceiros) |
| MidiExport | ferramenta (filtro de exportação) | sem janela (filtro de arquivo) | `MidiExport.cpp` | genérico (sem painel) |
| MidiImport | ferramenta (filtro de importação) | sem janela (filtro de arquivo) | `MidiImport.cpp` | genérico (sem painel) |
| Monstro | instrumento | visual próprio (vistas operador e matriz) | `Monstro.cpp`, `artwork_op.png`, `artwork_mat.png` | falta o plugin |
| MultitapEcho | efeito | visual próprio | `MultitapEchoControlDialog.cpp`, `artwork.png`, `graph_bg.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Nes | instrumento | visual próprio | `Nes.cpp`, `artwork.png` | plugin existe (lm-plugins-instruments): painel a fazer |
| OpulenZ | instrumento | visual próprio | `OpulenZ.cpp`, `artwork.png` | falta o plugin |
| Organic | instrumento | visual próprio | `Organic.cpp`, `artwork.png` | plugin existe (lm-plugins-instruments): painel a fazer |
| Oscilloscope | efeito (visualizador) | visual próprio (visor desenhado) | `OscilloscopeControlDialog.cpp`, `OscilloscopeGraph.cpp` | falta o plugin |
| Patman | instrumento | visual próprio | `Patman.cpp`, `artwork.png` | falta o plugin |
| PeakControllerEffect | efeito (gera controlador) | visual próprio (`artwork.svg`) | `PeakControllerEffectControlDialog.cpp`, `artwork.svg` | falta o plugin |
| ReverbSC | efeito | visual próprio (`artwork.svg`) | `ReverbSCControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `reverbsc`) |
| Sf2Player | instrumento | visual próprio | `Sf2Player.cpp`, `artwork.png` | falta o plugin |
| Sfxr | instrumento | visual próprio | `Sfxr.cpp`, `artwork.png` | plugin existe (lm-plugins-instruments): painel a fazer |
| Sid | instrumento | visual próprio | `SidInstrument.cpp`, `artwork.png` | plugin existe (lm-plugins-sid): painel a fazer |
| SlewDistortion | efeito | visual próprio | `SlewDistortionControlDialog.cpp`, `artwork.png` | plugin existe (lm-plugins-effects): painel a fazer |
| SlicerT | instrumento | visual próprio (janela redimensionável) | `SlicerTView.cpp`, `SlicerTWaveform.cpp`, `toolbox.png` | falta o plugin |
| SpectrumAnalyzer | efeito (visualizador) | visual próprio (espectro e cascata desenhados) | `SaControlsDialog.cpp`, `SaSpectrumView.cpp`, `SaWaterfallView.cpp`, ícones `.svg` | falta o plugin |
| StereoEnhancer | efeito | genérico / botões soltos | `StereoEnhancerControlDialog.cpp` | plugin existe (lm-plugins-effects); genérico (sem painel) |
| StereoMatrix | efeito | visual próprio (`artwork.svg`); obsoleto no navegador (F-FX-028), existe para abrir projetos antigos | `StereoMatrixControlDialog.cpp`, `artwork.svg` | **painel feito** (`panels/effects.rs`, teste `stereomatrix`) |
| Stk (Mallets) | instrumento | visual próprio (três subpainéis) | `Stk/Mallets/Mallets.cpp`, `Stk/Mallets/artwork.png` | falta o plugin |
| TapTempo | ferramenta | genérico / botões soltos | `TapTempoView.cpp` | genérico (sem painel) |
| TripleOscillator | instrumento | visual próprio | `TripleOscillator.cpp`, `artwork.png` | **painel feito** (`lm-gui-plugins/src/panels/tripleoscillator.rs`, teste `tripleoscillator`) |
| Vectorscope | efeito (visualizador) | visual próprio (visor desenhado) | `VecControlsDialog.cpp`, `VectorView.cpp` | falta o plugin |
| Vestige | instrumento (hospedeiro) | terceiros fora do processo (P-01) | `Vestige.cpp` | fora (terceiros) |
| Vibed | instrumento | visual próprio | `Vibed.cpp`, `NineButtonSelector.cpp`, `artwork.png` | plugin existe (lm-plugins-instruments): painel a fazer |
| VstBase | biblioteca do hospedeiro | terceiros fora do processo (P-01), sem janela | `VstPlugin.cpp`, `RemoteVstPlugin.cpp` | fora (terceiros) |
| VstEffect | efeito (hospedeiro) | terceiros fora do processo (P-01) | `VstEffectControlDialog.cpp` | fora (terceiros) |
| Watsyn | instrumento | visual próprio | `Watsyn.cpp`, `artwork.png`, `wavegraph.png` | falta o plugin |
| WaveShaper | efeito | visual próprio | `WaveShaperControlDialog.cpp`, `artwork.png`, `wavegraph.png` | plugin existe (lm-plugins-effects): painel a fazer |
| Xpressive | instrumento | visual próprio | `Xpressive.cpp`, `artwork.png`, `wavegraph.png` | falta o plugin |
| ZynAddSubFx | instrumento (hospedeiro) | terceiros fora do processo (P-05) | `ZynAddSubFx.cpp`, `RemoteZynAddSubFx.cpp` | fora (terceiros) |
