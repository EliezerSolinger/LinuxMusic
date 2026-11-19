# Interface em GPUI

Estado: **em andamento** (19/11). A biblioteca de interface é a **GPUI**, por decisão do usuário (19/11): não há mais
comparação com Slint e Qt. O protótipo em `LinuxMusicBeta/prototypes/gpui`, sobre o núcleo de mentira
`prototypes/fake-core`, serve de linha de base, de validação de como a GPUI se encaixa no núcleo (I-15) e de começo
da interface de verdade: ele vai prototipar **todas as telas do programa** e virar o `lm-gui`.

Este documento tem as medidas, como a versão da GPUI está fixada, o que a GPUI faz bem e mal no nosso caso, as
recomendações para o pacote `lm-gui` e uma seção por tela (o que foi feito, o que fica para o `lm-gui`, as
dificuldades da GPUI).

---

## 1. Como rodar e medir

```sh
cd LinuxMusicBeta/prototypes
CARGO_TARGET_DIR=../target cargo run --release -p proto-gpui          # o programa
CARGO_TARGET_DIR=../target cargo test -p proto-gpui -p fake-core      # testes (a GPUI roda sem janela)
CARGO_TARGET_DIR=../target cargo build --release -p proto-gpui && ./measure.sh 3   # medidas
```

Variáveis úteis: `LM_PAGE=mixing` abre na mixagem; `LM_OPEN=Settings,PianoRoll` abre telas no início;
`LM_WAVE=quads|path` troca o desenho das formas de onda (para comparar); `LM_THEME=arquivo` carrega outro tema;
`LM_BENCH=scroll|play|idle` liga a medição (abaixo).

**Método.** O próprio programa mede o tempo entre quadros e o tempo de CPU de cada quadro
(`fake-core/src/bench.rs`): o quadro começa no `render` da casca e termina na pintura do último elemento. O
`measure.sh` lê CPU e memória de fora (`ps`, `footprint`), entre o segundo 3 e o 21 de cada execução. Três modos:
- `scroll`: transporte tocando (64 medidores das faixas, 69 da mesa e o cursor mexendo) e a vista rolando e dando
  zoom por um roteiro fixo no tempo (zoom de 256 a 8192 amostras por pixel, rolagem horizontal e vertical); todo
  quadro redesenha todas as formas de onda visíveis;
- `play`: transporte tocando, vista parada;
- `idle`: parado.

**Máquina:** Mac mini M4 (10 núcleos), 16 GB, macOS 27.0.1. Antes de uma reinicialização, o monitor estava em
1920×1080 a 120 Hz (janela de 1600×980); depois, em 1280×720 a 120 Hz (janela de 1280×690, menos faixas e canais
visíveis). As duas séries estão separadas abaixo. A tela é 1x: **não deu para ver HiDPI** aqui.

**Execuções descartadas.** A máquina estava em uso por outra pessoa durante parte das medições. Uma janela coberta
por outra deixa de receber quadros do sistema (a GPUI para de desenhar, e a CPU cai para 2–4 %): toda execução com
menos de 114 quadros por segundo foi descartada. Os JSON de todas as execuções estão em `prototypes/results/`.

## 2. Medidas

### 2.1 Linha do tempo e mesa, 1280×690, formas de onda em ladrilhos (série atual)

| Medida | Parado | Tocando | Rolando e com zoom |
|---|---|---|---|
| Quadros por segundo | 0 (não desenha) | 120 | 120 |
| Intervalo entre quadros, mediana / p99 | — | 8,33 / 8,50 ms | 8,33 / 8,51 ms |
| Quadros perdidos (> 1,5 período) | — | 0–1 em 2400 | 0 em 2400 |
| CPU do quadro, mediana / p99 | — | 1,48 / 2,6 ms | 1,62 / 3,0 ms |
| CPU do processo (100 % = 1 núcleo) | 0,6–1,0 % | 25 % | 27 % |
| Memória (RSS) | 118 MB | 119 MB | 152 MB |
| Memória (`phys_footprint`) | 62–64 MB | 119 MB | 152 MB |

O protótipo já com a casca e a página de mixagem (24 canais visíveis, inspetor do rack) gasta 3,2 ms de CPU por
quadro tocando, a 120 quadros por segundo (medida curta, 3 s).

### 2.2 Como desenhar formas de onda: três jeitos medidos (1600×980, 13 faixas visíveis)

| Jeito | CPU do quadro tocando (mediana) | Rolando (mediana / p99) | Observação |
|---|---|---|---|
| Um retângulo por coluna de pixel | 5,0 ms | 5,9 / 7,1 ms | ~13 mil primitivas por quadro; o custo é a árvore de limites da cena da GPUI (`BoundsTree::insert`), que ordena cada primitiva |
| Um caminho preenchido por item | 5,35 ms | 6,6 / 9,3 ms | o custo passa para a triangulação na CPU (`lyon`); 44 quadros perdidos em 8 s |
| **Ladrilhos de imagem da pirâmide** | **2,2 ms** | **2,3 / 3,0 ms** | adotado: cada ladrilho (256 pares de um nível, 64 px de altura) é rasterizado uma vez e reaproveitado; rolar só move ladrilhos, o zoom entre dois níveis só estica na GPU |

Os números vêm de perfis com `sample` sobre um binário com símbolos. O custo da GPUI cresce com o **número de
primitivas**, não com a área pintada: qualquer desenho denso (formas de onda, piano roll com milhares de notas,
curvas de automação) precisa juntar primitivas ou virar imagem em cache.

### 2.3 Build e binário

| Medida | Valor |
|---|---|
| Build limpo, release (LTO fino), só GPUI | 48 s |
| Build limpo, debug | ~2 min 15 s |
| Build incremental (mudança no `main.rs`), release / debug | 9,6 s / 0,6–1,3 s |
| Binário release, sem símbolos, só GPUI | 7,2 MB |
| Binário release com o `gpui-component` | 19 MB |
| Crates no grafo de dependências | 517 |
| Disco do `target` do protótipo (debug + release) | ~3 GB |

(A medir de novo no fim, com todas as telas.)

## 3. Como a GPUI se encaixa no núcleo (I-15)

Tudo validado no protótipo, com testes que rodam a GPUI sem janela (`cargo test -p proto-gpui`):

- **A `Session` `!Send` mora numa entidade** (`Core`, em `gpui/src/core.rs`). As entidades da GPUI vivem na thread
  principal e não pedem `Send`, então a `Session` cabe sem adaptação. As vistas guardam um `Entity<Core>`, leem em
  `render` (`self.core.read(cx)`) e mudam nos eventos (`self.core.update(cx, ...)`). Nenhuma vista guarda ponteiro
  para o núcleo: só ids, `Feed`s e `GestureId`s. Não precisa de `Rc<RefCell<>>`.
- **O `pump` a cada 10 ms independe do desenho (H2)**: é uma tarefa no executor da thread principal, com um
  temporizador do relógio da GPUI. Ela só pede quadro (`cx.notify()`) quando algo mostrado mudou. Teste: 100 pumps
  em 1 s de relógio falso, sem nenhum quadro. Com a janela coberta, o desenho para e o `pump` continua.
- **Os `Feed`s são drenados no `render`** de cada vista (I-3.6): o rótulo de volume é refeito só quando o `Diff` diz.
- **Gestos por `GestureId` (I-3.7)**: o arrasto de um knob ou de um fader abre o gesto no clique, atualiza no
  movimento e confirma ao soltar. Teste: um arrasto simulado é um passo de desfazer só; duplo clique no fader volta a
  0 dB.
- **Carregamento (I-2)**: um componente só (`ui::loading_component`) lê o `Loading` do alvo; antes de 200 ms não
  desenha nada; depois, a barra de progresso no lugar da forma de onda; o botão de cancelar depois de 2 s.
- **Medidores a ~60 Hz (H16)** e **cursor interpolado**: o quadro de telemetria chega por triplo buffer; enquanto
  toca, `request_animation_frame` pede um quadro por atualização da tela (120 Hz aqui) e o cursor é interpolado
  entre quadros de telemetria pelo carimbo de tempo.
- **Parado não desenha**: 0 quadros e < 1 % de CPU.

## 4. A versão da GPUI e como está fixada

- A crate oficial `gpui` no crates.io está em **0.2.2, de outubro de 2025**: um ano atrás do Zed. Não serve.
- **`gpui-pre`** é um retrato semanal do `main` do Zed publicado no crates.io pelos mantenedores do `gpui-component`
  (Longbridge; o metadado de cada versão diz o commit do Zed). O `gpui-component` 0.10 exige `gpui-pre =0.3.12`
  (Zed `d2a4fb6`).
- **Fixado assim:** `gpui = { package = "gpui-pre", version = "=0.3.12" }`, `gpui-pre-platform =0.3.12`,
  `gpui-component =0.10.0`, com o `Cargo.lock` no repositório. Uma GPUI só no build.
- **Por que não o git do Zed:** baixar o repositório inteiro do Zed custa disco e tempo, e o `gpui-component` não
  compilaria contra outro commit. Se o `gpui-pre` parar de sair, o recuo é apontar `gpui` para o git do Zed no mesmo
  commit (`rev = "d2a4fb6..."`): é o mesmo código.
- **Atualizar** é uma tarefa própria: subir `gpui-pre` e `gpui-component` juntos, rodar os testes da `gui`, olhar as
  telas. A API muda entre semanas (exemplo visto: `ScrollWheelEvent` virou `ScrollEvent`).
- Recursos que tivemos de ligar: `font-kit` (sem ele o backend do macOS não desenha texto nenhum, só avisa no log) e,
  nesta máquina, `runtime_shaders` (só há as Command Line Tools, sem o compilador `metal` do Xcode; com o Xcode
  configurado, sai).

## 5. O `gpui-component` serve?

Serve para os controles comuns: o protótipo usa `Input`, `Slider`, `Switch` e `Button` dentro do nosso tema, e
`Root` (camada de sobreposição). Tem também listas virtuais, tabelas, docks, diálogos, menus, árvore, editor de
código e acessibilidade pela AccessKit. Licença Apache-2.0. Custos: o binário passa de 7 para 19 MB, e ele fixa a
versão da GPUI. Os controles de áudio (knob, fader com medidor, forma de onda, piano roll) são nossos, desenhados
direto na GPUI.

## 6. Linux

- A GPUI tem backend próprio para **X11 e Wayland** (`gpui-pre-linux`), com desenho por **wgpu** (Vulkan) e
  acessibilidade pela `accesskit_unix` (AT-SPI). É o mesmo código do Zed no Linux.
- **Não foi testado em Linux aqui** (não há máquina Linux nem o alvo instalado). O grafo de dependências para
  `x86_64-unknown-linux-gnu` resolve e baixa (`cargo fetch --target`).
- Riscos: placas sem Vulkan (máquinas antigas, algumas VMs); o desenho cai para o que o wgpu oferecer. A embalagem
  (Flatpak, .deb) é nossa, como a do Zed.
- **Recomendação:** um job no CI (Ubuntu) que compile o `lm-gui` e rode os testes sem janela, e um teste manual em X11
  e em Wayland antes da primeira fatia.

## 7. Riscos e como mitigar

| Risco | Mitigação |
|---|---|
| GPUI antes da 1.0, API muda toda semana | versão fixada com `=`; atualização como tarefa própria, com testes de tela sem janela |
| O `gpui-pre` é de terceiros (Longbridge) | é o código do Zed; recuo: git do Zed no mesmo commit |
| Desenho denso fica caro (custo por primitiva) | ladrilhos em cache para formas de onda; juntar primitivas; medir cada tela nova com `LM_BENCH` |
| Linux sem teste real | CI compilando e testando; teste manual em X11 e Wayland |
| Sem tradução embutida | tradução como dado, nossa (`ui/i18n.rs`, arquivos por área) |
| Acessibilidade dos controles próprios | a GPUI tem `role`, `aria_label`, `aria_numeric_value`: cada knob e fader declara papel e valor |
| HiDPI não visto aqui | ladrilhos rasterizados no fator de escala da janela (pendente) |

## 8. Recomendações para o `lm-gui`

- **Estrutura:** a do protótipo: `core` (a entidade dona da `Session` e do pump), `workspace` (a casca: transporte,
  abas das páginas, docas, modais, teclas), `pages` (Composição e Mixagem), `screens` (uma tela por arquivo, com um
  registro), `ui` (tema, tradução, componentes comuns, o componente de carregamento).
- **A `Session` numa entidade `Core`**; vistas leem em `render` e mudam em eventos; nenhuma referência guardada.
- **O pump numa tarefa com o relógio da GPUI** (assim roda igual no teste com relógio falso).
- **Tema como dado** (`themes/*.txt`, carregado no início; nenhuma cor no código) e **tradução como dado**
  (`locales/<língua>/<área>.txt`, chaves em inglês; um arquivo por área para ninguém pisar no outro).
- **Testes de tela sem janela** (`#[gpui::test]`, `debug_selector`, cliques e arrastos simulados) para cada gesto.
- **Medir cada tela** com o `LM_BENCH`: a meta é ficar abaixo de 4 ms de CPU por quadro a 120 Hz.

---

## 9. As telas

Divisão do trabalho (19/11): **A** = a casca, o kit comum, a Composição, o transporte, a Mixagem, o roteamento, as
medidas e as partes comuns deste documento; **B** = Piano Roll, automação, passos; **C** = janela do instrumento,
janela de efeito e cadeia de inserts, navegador; **D** = configurações, exportação, projeto, avisos, paleta, projeto
novo e "sobre". Os designs aprovados (2, 3, 5 e 8) são especificação; nas outras áreas, cada funcionalidade do legado
(`docs/funcionalidades-legado.md`) tem de existir, mas o layout é repensado, sem copiar a tela do legado.

Regras para quem escreve uma tela: a versão da GPUI fixada (não mudar); cores só de `ui::THEME`; todo texto por
`ui::tr`; os dados de mentira no seu módulo do `fake-core`; uma linha de registro em `screens/mod.rs`,
`fake-core/src/lib.rs` e `ui/i18n.rs`; commit por caminho e push a cada tela; não rodar medições de interface (a
máquina é compartilhada; as medidas são feitas no fim).

Cada seção abaixo diz: o que foi feito, o que fica para o `lm-gui`, e as dificuldades da GPUI.

### 9.1 Casca: páginas, docas, modais, teclas (A)
Feito: transporte sobre as duas páginas; abas Composição e Mixagem (Cmd/Ctrl+1, +2, Ctrl+Tab); docas embaixo (editores),
à esquerda (navegador) e à direita (instrumento, efeito, avisos); modais centralizados; Espaço toca e para só nas
páginas (um campo de texto guarda os espaços); Esc fecha o modal; um lançador provisório para cada tela.
Dificuldades: o analisador de atalhos desta GPUI não tem o modificador "secondary" (Cmd no Mac, Ctrl no Linux): uma
função nossa escolhe.

### 9.2 Composição: linha do tempo e cabeçalhos (A)
Feito até aqui: 64 faixas, cabeçalho do design 02 sem pan (número na cor, gravar, nome, M, S, knob de volume com
gesto, valor, FX, I/O, medidor), formas de onda em ladrilhos da pirâmide, rolagem e zoom (roda, trackpad, pinça),
régua com compassos, cursor interpolado, carregamento no lugar da forma de onda.
A fazer: várias saídas por faixa, pastas, itens MIDI, camadas de takes com comping, alças de pré-roll e punch, forma
de onda crescendo na gravação, lane de andamento com rampas, lanes de automação, loop e punch na régua.

### 9.3 Transporte (A, design 03)
Feito: visor grande em compassos, relógio, estado por palavra e cor, andamento, compasso, metrônomo, pré-roll.
A fazer: contagem, loop e punch ligados ao núcleo, trocar o formato do visor num clique, andamento editável no lugar.

### 9.4 Mixagem (A)
Feito: página própria; canais que existem por si (64 normais, 4 retornos, o master fixos à direita); por canal, as
seções do rack como chaves, os inserts, os envios (com nível em knob e marca de pré-fader), pan, M, S, **o fader
sobre o medidor** (gesto ao arrastar, duplo clique volta a 0 dB, valor em dB), a saída; o rack inteiro do canal
selecionado no inspetor à direita, com um knob por parâmetro (gesto, duplo clique volta ao padrão).
A fazer: escolher a saída, adicionar e remover inserts e envios, abrir a janela do efeito, criar e apagar canais.

### 9.5 Roteamento (A)
A fazer: a saída de cada faixa para um canal, e a ação "criar canais para as saídas".

### 9.6 Piano Roll (B)
Feito (`screens/piano_roll.rs`, dados em `fake-core/src/notes.rs`): as notas de um item MIDI, num mapa por número
`n` (C1), em tiques de 960 PPQ, com velocidade, pan e **curva de afinação por nota** (P-24). Layout repensado: uma
barra só (item, ferramentas, grade, tamanho, tom/escala/acorde, quantizar, ferramentas de nota, fantasma, tocar o
item, gravar); teclado, régua com **loop e cursor próprios do item** (P-26) e as marcas de fim do item e da parte
que repete, a grade de notas (as repetições do padrão aparecem apagadas), a lane de velocidade ou pan embaixo, e à
direita um **inspetor** da seleção (tecla, início, tamanho, velocidade, pan, afinação: arrastar muda todas as
selecionadas, duplo clique digita). Cobre F-NOT-001 a 019, 021, 023 e F-PAS-001: lápis (clique desenha, arrastar dá
o tamanho; o corpo move, a borda redimensiona, Shift no começo copia, Alt sem grade, Shift na borda estica a seleção
em proporção, botão direito apaga arrastando), seleção (retângulo, Shift alterna, Ctrl/Cmd seguro seleciona em
qualquer ferramenta), borracha, afinação (desenha a curva; botão direito deixa o primeiro ponto), **faca ao longo
de uma linha diagonal** (Shift ao soltar tira os pedaços curtos, Esc cancela), **strum em escada**; grade Snap
(início ou fim, o mais perto) e Nudge; tamanho "última nota" ou fixo; tom e escala destacam as linhas e o teclado,
e prendem as notas na escala; acorde insere o acorde, Shift+clique como arpejo; menu da tecla (marcar semitom,
oitavas, escala, acorde, desmarcar, selecionar as notas da tecla); quantizar (inícios, tamanhos, ambos, 50 %);
colar notas encostadas, preencher, cortar sobreposições, mínimo e máximo = o último, inverter, duplicar, transpor;
lane: pintar nas notas perto (ou só nas selecionadas; Alt só a de baixo), botão do meio volta ao padrão, roda ±1 %,
duplo clique digita; teclas do legado (Shift+↑↓ semitom, Ctrl+↑↓ oitava, Shift+←→ passo, Ctrl+←→ compasso, Home,
Ctrl+0–9 tamanho, Alt+0–9 grade, Ctrl+A, Esc, Delete, Ctrl+X/C/V/D/Z); roda, Shift+roda, Ctrl+roda (zoom no
mouse), Alt+roda (altura das teclas), pinça; Ctrl+arrasto na régua seleciona o trecho; o teclado do computador toca
(Z..M, Q..U, - e = mudam a oitava) e grava ao vivo (somar ou substituir, quantizar a entrada) ou **por passos**
(acorde com as teclas juntas, ←→ e Backspace movem o cursor); Espaço toca só o item, em loop. Um arrasto é um gesto
(um passo de desfazer), aberto no primeiro movimento real (um clique não deixa passo vazio); o mouse fica preso
durante o arrasto. Tudo pelo `CorePort`: `AddNote`, `SetNote` (`Field::Note`) e `RemoveNote`; os números novos vêm
do núcleo (o contador global, S12) e a tela os lê da raiz.

Fica para o `lm-gui`: som de verdade ao clicar nas teclas e ao mover notas (o núcleo de mentira não toca MIDI);
gravação MIDI com camadas por volta do loop e CC/pitch bend/aftertouch (P-15); o "pegajoso" (Shift+Ctrl
empurrando as seguintes) e os mapas do microafinador (F-NOT-015); copiar e colar entre projetos como JSON (P-25);
abrir o item escolhido na Composição (hoje o item vem de um menu da barra); alça para redimensionar a altura da
lane. Dificuldades da GPUI: o desenho é por primitiva, então só as notas visíveis são pintadas (o caso de 10 mil
notas, "Arpeggio", corta pela janela antes de pintar) e os nomes das notas só aparecem quando cabem; texto dentro
de um `canvas` exige moldar a linha à mão (`shape_line` + `paint`); `on_mouse_move` só chega com o mouse em cima do
elemento, então o arrasto registra ouvintes da janela (`window.on_mouse_event`) enquanto dura; não há "secondary"
nas teclas (Cmd/Ctrl decidido por nós); o Espaço do item precisou de uma ação no contexto `PianoRoll`, que vence o
`Pages` da casca.

### 9.7 Editor de automação (B)
Feito (`screens/automation.rs`): a curva de uma lane (a lane é do dono do parâmetro e cobre a música inteira),
escolhida na barra por dono e parâmetro; eixo de valores **nas unidades do parâmetro** (dB, Hz em escala log, %,
semitons em degraus, os nomes de uma escolha); cada ponto tem forma até o próximo: **Degrau, Curva** (`curve` de −1
a 1, com uma alça no meio do trecho para curvar) **ou Suave** (spline com a tensão da lane); **valor de saída
diferente do de entrada** (P-27: Alt+arrasto no ponto, botão direito desfaz); parâmetros discretos só desenham e
editam degraus e nunca interpolam. Cobre F-AUT-001 a 009 e 011 e F-CLP-081/082: lápis (clique cria e arrasta,
Shift+clique traça uma reta de pontos desde o último, um por passo da grade), lápis livre (o traço vira o menor
número de pontos que o segue), seleção, borracha (e botão direito arrastando), duplo clique digita o valor,
espelhar no tempo e no valor (seleção ou lane), **escalar os valores** da seleção (arrastar no inspetor),
ajustar a altura aos pontos ou Alt+roda no valor, grade com tercinas, Ctrl+roda zoom, pinça, ←→ e Home movem o
cursor, copiar/colar/apagar, notas fantasmas de um item MIDI, leitura do tempo e do valor sob o mouse, o valor
atual tocando, o botão de gravar a automação da lane. Uma lane que move vários parâmetros (o **controle
compartilhado**, P-33) lista os outros alvos. Pelo `CorePort`, um ponto é um nó (`Insert` na lista `points` da
lane, `Set` com `Field::Point`, `Remove`).

Fica para o `lm-gui`: a gravação de automação de verdade (mexer no botão tocando; o fake só guarda o "armado");
criar o controle compartilhado arrastando um botão para outro (P-04/P-33, gesto das telas de mesa e instrumento);
a lane dentro da Composição (dono A) reaproveitando este desenho. Dificuldades da GPUI: a curva é um traço e um
preenchimento com amostras a cada 3 px; o `PathBuilder` triangula na CPU (`lyon`), então o número de vértices tem
de ficar na largura da tela, não no número de pontos.

### 9.8 Editor de passos (B)
Feito (`screens/steps.rs`): substitui o Beat+Bassline (F-PAD-001 obsoleto): mostra vários itens MIDI que se repetem,
um grupo por item (escolhidos num menu), com **uma linha por peça do kit** (AVL Drums) ou de um **instrumento de
pads de amostras** (P-23), ou uma linha de passos com altura para um item melódico (o baixo). Cobre F-PAD-002: clique
liga e desliga, arrastar pinta (ou apaga, conforme o primeiro passo), botão direito apaga, a roda muda a
velocidade ±5 %, passos mais fortes mais claros (e uma barrinha da velocidade), acrescentar e tirar um compasso,
**clonar os passos** (dobra copiando), o tamanho do passo (1/8, 1/16, 1/32, 1/16T), o **swing** do item (arrastar), o
passo tocando aceso; embaixo, a lane da linha escolhida: velocidade, pan, altura (a nota de um passo melódico, a
afinação de uma peça de bateria) e duração; Alt+arrasto num passo melódico muda a nota; "todas as linhas" mostra as
peças sem notas. Tudo pelo `CorePort` (notas por `AddNote`/`SetNote`/`RemoveNote`, os números do item por
`Field::Clip`). Um arrasto é um gesto; o desfazer é Ctrl+Z.

Fica para o `lm-gui`: o swing aplicado na compilação do item (o fake só guarda o valor e o mostra); tocar o padrão
sozinho; arrastar um item da Composição para a vista; os passos editáveis na própria linha do tempo com zoom alto
(F-PAD-002, dono A). Dificuldades da GPUI: nenhuma nova; cada grupo é um `canvas`, uma primitiva por célula.

Testes destas três telas (`cargo test -p proto-gpui`): desenhar e mover uma nota é um passo de desfazer cada;
pintar a lane de velocidade; Ctrl+A e Shift+↑ pelo teclado e a faca; o menu do item; criar e arrastar um ponto de
automação; um ponto numa lane de escolha é degrau; espelhar duas vezes volta ao início; ligar, desligar e pintar
passos (um passo de desfazer); acrescentar, tirar e clonar passos. Em `fake-core`: quantizar, colar, faca,
inverter, strum, escalas e acordes, as formas das curvas, a escala log, o afinamento do traço.
### 9.9 Janela do instrumento e editor genérico de plugin (C, design 05)
### 9.10 Janela de efeito e cadeia de inserts (C)
### 9.11 Navegador (C)
### 9.12 Configurações (D, design 08)
### 9.13 Exportação (D)
### 9.14 Projeto: abrir, salvar, somente leitura, recuperação, modo seguro (D)
### 9.15 Avisos e atividades (D)
### 9.16 Paleta de comandos (D)
### 9.17 Projeto novo e "sobre" (D)
