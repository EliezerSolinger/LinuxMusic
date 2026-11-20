# Funcionalidades do legado (LinuxMusic, fork do LMMS)

Estado: **completo** (19/11), lido no código do legado por áreas. Este documento é o inventário de tudo o que o programa antigo (`legado/`) faz. Ele
serve a duas coisas:

1. **Referência para quem prototipa as telas** do programa novo (`LinuxMusicBeta`, Rust + GPUI).
2. **Lista de conferência da 1.0**: a 1.0 tem todas as funcionalidades do legado, reescritas no contrato novo
   (`decisoes-autonomas.md`), menos as que o desenho novo substitui de propósito.

**O programa novo não é um clone do legado** (decisão do usuário). O que conta aqui é a **capacidade**: o que o usuário
consegue fazer, com quais opções e parâmetros, e com que comportamento. A tela do legado aparece só como "onde fica
hoje", para quem quiser abrir o programa velho e ver. Atalhos e gestos entram como funcionalidade (o que fazem), não
como posição de botão a copiar. As telas sem design aprovado são repensadas; os designs aprovados (2 sem pan, 3, 5 e 8,
em `design-sugestoes/`) continuam valendo.

Tudo aqui foi lido no código do legado (`legado/src`, `legado/plugins`, `legado/include`) e nos documentos dele
(`legado/docs`). Os caminhos são relativos a `legado/`.

## Como ler

- **Id** (`F-AREA-NNN`): estável. Outros documentos citam a funcionalidade pelo id. Um id nunca é reaproveitado; se
  uma funcionalidade sair do inventário, a linha fica, riscada, com o motivo.
- **Status:**
  - **core**: precisa estar na primeira fatia ou no esqueleto (`arquitetura-nova.md`, "A ordem de construção":
    gravar com monitoramento, faixa com TripleOscillator, mixer com um efeito, master, transporte, salvar e abrir, e
    os plugins de validação).
  - **1.0**: entra antes da 1.0.
  - **obsoleto — substituído por …**: o desenho novo troca de propósito; diz pelo quê.
  - **aberto**: o desenho novo ainda não diz o que acontece; está na lista de perguntas no fim.
- "Ctrl" quer dizer Cmd no macOS, como no legado.

## Áreas

| Prefixo | Seção |
|---|---|
| F-JAN, F-IJN | 1. A casca do programa (páginas, menus, transporte) e a janela do instrumento |
| F-NOT, F-ARR, F-REG, F-PAD, F-AUT, F-PAS | 2. Edição: notas, arranjo, régua e marcadores, padrões, curvas, gravação por passos |
| F-TRK | 3. Faixas |
| F-CLP | 4. Clipes e itens |
| F-REC | 5. Gravação |
| F-MIX | 6. Mixer e mesa analógica |
| F-INS, F-FX, F-HOST | 7. Instrumentos, efeitos e plugins de terceiros |
| F-CTL | 8. Automação e controladores |
| F-PRJ | 9. Projeto e arquivos |
| F-CFG | 10. Configurações |
| F-BRW | 11. Navegador, presets e amostras |
| F-GER | 12. Outros |

---

## 1. A casca do programa: páginas, menus, transporte, janela do instrumento

No legado há uma janela só, com uma barra no topo (Salvar, o transporte no centro, e a troca de página
Composição | Mixagem | engrenagem), três páginas e janelas flutuantes por cima. Não há barra de status. O programa
novo decidiu: **Composição e Mixagem são páginas separadas**, com abas no topo e um atalho, e a barra de transporte
aparece nas duas (`decisoes-autonomas.md`). O resto da disposição é repensado.

### 1.1 Páginas, painéis e janelas (hoje: `src/gui/MainWindow.cpp`, `DockPanel.cpp`, `SubWindow.cpp`, `SideBarWidget.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-JAN-001 | Página Composição | Arranjo e editor de padrões em abas em cima; editor de notas e de automação em abas embaixo; rack de controladores e notas do projeto em abas à direita (começa recolhida) | core (a página; a disposição é repensada) |
| F-JAN-002 | Página Mixagem | A mesa inteira; Ctrl+5 alterna Composição ↔ Mixagem | core |
| F-JAN-003 | Página Configurações | As configurações como página (engrenagem, Ctrl+,); sair dela aplica; abre sozinha na primeira execução e quando o dispositivo de áudio falha | core (design 8 aprovado) |
| F-JAN-004 | Dois navegadores | O da Composição (instrumentos, favoritos, projetos, amostras, presets, arquivos do projeto) e o da Mixagem (efeitos), cada um só na sua página; à esquerda ou à direita (configuração, ao vivo); Ctrl+8 mostra/esconde | core |
| F-JAN-005 | Recolher uma área | Um botão recolhe a área de abas de cima ou de baixo até a barra de abas, lembrando a altura; abrir de novo um editor já aberto recolhe a área dele | 1.0 |
| F-JAN-006 | Janelas flutuantes | Janelas de instrumento e de efeito flutuam sobre as páginas; arrastar o fundo move todas; cada uma pode **sair para uma janela própria do sistema** (destacar) e voltar; "Destacar todas" (Ctrl+Shift+D) e "Encaixar todas" (Ctrl+Alt+Shift+D); comportamento ao fechar uma destacada (configuração); posição, tamanho e visibilidade salvos no projeto | 1.0 (a posição vai para `interface.json`) |
| F-JAN-007 | Título da janela | "<projeto>[*] - LinuxMusic <versão>", "Sem título", e "Sessão recuperada. Salve o seu trabalho!" | core |
| F-JAN-008 | Botão Salvar pisca no autosave | Duas piscadas lentas quando o autosave escreve | 1.0 |
| F-JAN-009 | Perguntar antes de perder mudanças | Novo/Abrir/Sair: Salvar / Descartar / Cancelar; texto próprio para sessão recuperada | core |
| F-JAN-010 | Ao fechar | Fecha as configurações, manda para a lixeira as mídias geradas que o projeto não usa, termina o autosave, apaga o arquivo de recuperação | core (sem a lixeira: P-13) |
| F-JAN-011 | Dicas (tooltips) | Em quase todo controle; podem ser desligadas (ao vivo) | 1.0 |
| F-JAN-012 | Teclas globais | Espaço toca/para o último editor tocado; Shift+Espaço pausa; com uma janela de instrumento na frente, o teclado do computador toca notas; senão R grava e C liga o loop | core |
| F-JAN-013 | Mensagens flutuantes | Avisos curtos que somem (projeto salvo, carregando amostra, dicas) | core (avisos, I-14) |
| F-JAN-014 | Fundo com imagem | Imagem de fundo da área das janelas (sem opção na tela) | obsoleto — sem tela no legado; o tema novo é dado |

### 1.2 Menus e atalhos (hoje: `MainWindow.cpp`, `src/gui/Shortcuts.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-JAN-020 | Arquivo | Novo (Ctrl+N) · Novo a partir de modelo ▸ · Abrir (Ctrl+O) · Recentes ▸ · Salvar (Ctrl+S; sem nome vira Salvar como) · Salvar como (Ctrl+Shift+S) · Salvar como nova versão (Ctrl+Alt+S) · Salvar como modelo padrão · Importar (MIDI) · Importar arquivos de áudio (cada um numa faixa, no compasso do cursor) · Exportar (Ctrl+E) · Exportar faixas (Ctrl+Shift+E) · Exportar MIDI (Ctrl+M) · Sair (Ctrl+Q) | core (as ações; detalhes na seção 9) |
| F-JAN-021 | Editar | Desfazer (Ctrl+Z) · Refazer (Ctrl+Shift+Z / Ctrl+Y) · Escalas e mapas de teclas · Configurações (Ctrl+,) | core |
| F-JAN-022 | Ver | Arranjo Ctrl+1 · Padrões Ctrl+2 · Notas Ctrl+3 · Automação Ctrl+4 · Mixagem Ctrl+5 · Controladores Ctrl+6 · Notas do projeto Ctrl+7 · Navegador Ctrl+8 · Tela cheia F11 · Destacar/Encaixar todas · Rolagem suave · Nomes das notas no editor de notas | core (os atalhos de página); o resto 1.0 |
| F-JAN-023 | Ferramentas | Uma entrada por plugin de ferramenta (hoje só o Tap Tempo) | 1.0 |
| F-JAN-024 | Ajuda | Ajuda online (abre a documentação do LMMS) · Sobre | aberto (P-34): ajuda de quem? |
| F-JAN-025 | Atalhos configuráveis | 9 ações com tecla mudável nas configurações: Gravar R · Loop C · Dividir no cursor S · Marcador M · Região sobre o loop Shift+R · Copiar · Cortar · Colar no cursor · Duplicar Ctrl+D. Os outros atalhos são fixos | core — no novo **todo** atalho é configurável (ações como dado, item 6) |
| F-JAN-026 | Paleta de comandos | Não existe no legado | (novo, Ctrl+K; só para registro) |

### 1.3 Barra de transporte (hoje: `src/gui/TransportBar.cpp`; o design 3 aprovado vale)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-JAN-040 | Ao início | Leva o cursor ao 0 (tocando, continua do início) | core |
| F-JAN-041 | Parar, Tocar/Pausar | Tocar/pausar a **música** (Espaço) | core |
| F-JAN-042 | Gravar | Ver F-REC-020 a F-REC-029; com o menu da seta | core |
| F-JAN-043 | Loop | Liga/desliga o loop da régua (C), sincronizado com a régua | core |
| F-JAN-044 | Relógio de posição e relógio de tempo | Um grande e um pequeno; clicar escolhe o formato de cada um (lembrado): compassos e tempos "BBB.b.ttt" (segue as fórmulas do mapa), minutos e segundos, amostras (exato tocando), timecode SMPTE 30 fps; a borda do grande tem a cor do estado | core |
| F-JAN-045 | Estado em palavra | Ver F-REC-026 | core |
| F-JAN-046 | Andamento | Arrastar para cima/baixo (1 BPM a cada 3 px), roda ±1 (Shift ±10), duplo clique digita; clique abre o menu: digitar, **bater o andamento** (média das últimas 8, recomeça depois de 2 s), "mudar o andamento a partir do cursor (mapa)" | core |
| F-JAN-047 | Fórmula de compasso | Botão "N/D" que abre o editor de numerador e denominador | core |
| F-JAN-048 | Metrônomo, pré-roll, contagem | Ver F-REC-023 a F-REC-025 | core |
| F-JAN-049 | Mudo do master | Ver F-MIX-015 | 1.0 |
| F-JAN-050 | Medidores CPU e OUT | Duas barras: carga do áudio e nível RMS do master; vermelho acima de 95 % | core |
| F-JAN-051 | Volume e transposição geral do projeto | O projeto ainda salva volume (0–200) e transposição (−12–12) sem nenhum controle na tela; a transposição vale para as faixas com "usar a transposição geral" | aberto (P-37) |

### 1.4 Janela do instrumento (hoje: `src/gui/instrument/`; o design 5 aprovado vale)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-IJN-001 | Cabeçalho | Nome da faixa (renomeia ao vivo); setas para ir ao instrumento anterior/seguinte na mesma janela; mudo, solo | core |
| F-IJN-002 | VOL, PAN, PITCH, RANGE, CHAN | Volume 0–400 %, 100 · Pan −100–100 · Afinação ±100 cents, 0 · Faixa de pitch bend 1–60 semitons, 2 · Canal do mixer (escolher ou novo) | core (volume, afinação, faixa de bend, destino) · obsoleto: pan |
| F-IJN-003 | Salvar preset da faixa | Grava a faixa (instrumento e ajustes, sem mudo/solo) num `.xpf` em `<presets>/<instrumento>/` | 1.0 (no novo, um preset é uma árvore num arquivo; `.xpf` velhos: P-09) |
| F-IJN-004 | Soltar na janela | Um instrumento do navegador troca o instrumento; um `.xpf` carrega; um arquivo de plugin (SoundFont, `.xiz`) carrega com o plugin certo, trocando se precisar | core |
| F-IJN-005 | A janela lembra a aba, o tamanho e a posição; o teclado do computador toca | | 1.0 |
| F-IJN-006 | Aba do plugin | A tela própria de cada instrumento (seção 7) | core |
| F-IJN-010 | Envelopes e LFOs | Três alvos: VOLUME, CORTE, RESSONÂNCIA. Envelope: Pré-atraso 0–2, 0 · Ataque 0–2, 0 · Hold 0–2, 0,5 · Decay 0–2, 0,5 · Sustain 0–1, 0,5 · Release 0–2, 0,1 · Quantidade −1–1, 0 (os tempos seguem uma curva exponencial até 5 s por segmento). LFO: Pré-atraso 0–1 · Ataque 0–1 · Velocidade 0,001–1, 0,1, sync · Quantidade −1–1, 0 · Forma: senoide, triângulo, serra, quadrada, aleatória, **do usuário** (soltar uma amostra) · "FREQ × 100" · "o LFO controla a quantidade do envelope". Clicar no gráfico alterna a quantidade 0 ↔ 1; o gráfico do envelope tem a escala Dinâmica / Absoluta / Relativa | 1.0 |
| F-IJN-011 | Filtro | Liga · Tipo (os 22 do DualFilter, F-FX-013) · Corte 1–14000 Hz, 14000 · Ressonância mín–10, 0,5 | 1.0 |
| F-IJN-012 | Aviso de instrumento que não aceita | "Envelopes, LFOs e filtros não valem para este instrumento" (fluxo único), ou parcial | 1.0 |
| F-IJN-013 | Empilhar acordes | Liga · Acorde (tabela abaixo) · Extensão 1–9 oitavas, 1 | 1.0 |
| F-IJN-014 | Mono, legato e glide | Liga · Legato (ligado: a voz vai para a nota nova sem recomeçar) · Glide 0–2000 ms, 0 | 1.0 (o `VoicePool` tem poly, mono e legato) |
| F-IJN-015 | Arpejo | Liga · Acorde · Extensão 1–9 oitavas · Repetições 1–8 · Ciclo 0–6 notas · Pular 0–100 % · Faltar 0–100 % · Tempo 25–2000 ms, 200, sync · Gate 1–200 %, 100 · Direção: sobe, desce, sobe e desce, desce e sobe, aleatória · Modo: livre, ordenado, sincronizado | 1.0 |
| F-IJN-016 | Tabela de acordes e escalas | 95 entradas: oitava, maior, menor, sus, aum, dim, 6, 7, 9, 11, 13 e variantes; e as escalas maior, menores harmônica e melódica, tons inteiros, diminuta, pentatônicas, bebop, blues, árabe, enigmática, napolitanas, húngara, os modos gregos, cromática, persa etc. Usada também no editor de notas | 1.0 |
| F-IJN-017 | MIDI | Entrada: liga, canal 0–16 (0 = todos), velocidade fixa −1–127 (−1 = não). Saída: liga, canal, velocidade fixa, programa 1–128, nota fixa. Dispositivos de entrada e saída (clientes tipo sequenciador). Velocidade base 1–127, 63 (essa velocidade = 100 %) | 1.0 (saída MIDI de faixa: P-38) |
| F-IJN-018 | Transposição geral | Liga (padrão): a faixa segue a transposição do projeto | aberto (P-37) |
| F-IJN-019 | Microafinador | Liga · Escala ativa (10 posições por projeto) · Mapa de teclas ativo (10) · "Importar as faixas de nota do mapa" (ligado). Não vale para instrumentos MIDI | 1.0 |
| F-IJN-020 | Editor de escalas e mapas | Por escala: descrição, carregar/salvar `.scl` (Scala), intervalos em texto (cents com ponto, ou razão), aplicar com validação. Por mapa: descrição, carregar/salvar `.kbm`, graus por tecla (x = sem som), primeira e última tecla, tecla do meio (C4), nota base (A4), frequência base 0,1–9999,999 Hz (440) | 1.0 |
| F-IJN-021 | Teclado na janela | Clique toca (velocidade pela altura do clique), arrasto desliza; marcadores de nota base (A4), primeira e última nota, que se arrastam e se automatizam; teclas fora da faixa apagadas | core (tocar) · 1.0 (marcadores) |
| F-IJN-022 | Teclado do computador como piano | Fileira de baixo Z S X D C V G B H N J M , L . ; / e fileira de cima Q 2 W 3 E R 5 T 6 Y 7 U I 9 O 0 P [ = ] (duas oitavas e pouco), pela posição da tecla; perder o foco solta todas | core |
| F-IJN-023 | Aba de efeitos da faixa | A cadeia de efeitos da faixa | obsoleto — os efeitos moram no canal da mesa (F-FX-002) |
| F-IJN-024 | Janela da faixa de amostra | Nome, mudo, solo, volume, pan, canal, efeitos | obsoleto — faixa única |
| F-IJN-025 | Rack de CC MIDI | Por faixa: liga (desligado) e 128 botões CC 0–127 que mandam CC ao instrumento; automatizáveis | 1.0 |

## 2. Edição

### 2.1 Notas (hoje: Piano Roll, `src/gui/editors/PianoRoll.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-NOT-001 | Desenhar notas | Clique no vazio cria uma nota com o tamanho escolhido, na grade (arredonda para o mais perto), com a última velocidade e pan usados; a nota nova fica selecionada e é tocada. Arrastar o corpo move; arrastar a cauda redimensiona. Botão direito apaga (arrastando, apaga várias). Clicar numa nota torna a velocidade, o pan e o tamanho dela os "últimos". Atalho Shift+D | core |
| F-NOT-002 | Apagar notas | Ferramenta borracha: clique ou arrasto apaga (Shift+E) | core |
| F-NOT-003 | Selecionar | Retângulo de seleção (Shift+S); Shift+arrasto alterna as notas do retângulo; segurar Ctrl em qualquer ferramenta vira seleção temporária; Ctrl+A tudo; Ctrl+Shift+A e Esc nada | core |
| F-NOT-004 | Curva de pitch bend por nota | Ferramenta "detune" (Shift+T): desenhar uma curva de afinação sobre a nota, nas notas selecionadas; botão direito apaga pontos; o primeiro ponto é fixo; Shift+clique abre a curva no editor de automação | 1.0 (aberto, P-24: o novo tem automação por nota?) |
| F-NOT-005 | Faca | Shift+K: arrastar uma **linha** (pode ser diagonal) corta as notas selecionadas (ou todas) ao longo dela; Ctrl = sem grade; Shift ao soltar tira os pedaços curtos; Esc/botão direito cancela | 1.0 |
| F-NOT-006 | Strum | Shift+J: arrastar uma nota de um acorde desloca os inícios das notas do acorde em escada; Shift curva a escada | 1.0 |
| F-NOT-007 | Mover notas | Modo de grade "Nudge" (move pelo passo, mantém o deslocamento) ou "Snap" (o início ou o fim cola na grade, o que estiver mais perto). Alt = sem grade. Shift no começo do arrasto = duplica. Mudar de tecla arrastando toca a nota | core |
| F-NOT-008 | Redimensionar notas | Muda todas as selecionadas na grade; Alt sem grade (mín. 1 tique); Shift = estica a seleção proporcionalmente; Shift+Ctrl = "pegajoso": empurra as notas seguintes. Uma nota só movida deixa de ficar selecionada depois | core |
| F-NOT-009 | Teclado | ↑/↓ rola; Shift+↑/↓ transpõe ±1 semitom; Ctrl+↑/↓ ±1 oitava (tamanho da oitava do microafinador se ligado); Shift+←/→ move pelo passo (Shift+Alt 1 tique); Ctrl+←/→ um compasso; Alt+←/→ vai ao clipe anterior/seguinte da faixa; Delete apaga; Home leva o cursor ao 0; Ctrl+0–9 escolhe o tamanho de nota; Alt+0–9 a quantização | core |
| F-NOT-010 | Velocidade e pan das notas | Uma faixa abaixo das notas com Velocidade (0–200 %, 100) ou Pan (−100–100); pintar com o mouse nas notas perto ou nas selecionadas (Alt = só a de baixo do cursor); botão do meio volta ao padrão; roda ±1; duplo clique pede o valor; a opacidade da nota mostra a velocidade; altura da faixa ajustável | core |
| F-NOT-011 | Ferramentas de notas | Colar (Shift+G: junta notas encostadas da mesma tecla); Preencher (Shift+F: estende até a próxima); Cortar sobreposições (Shift+C); Tamanho mínimo = o último; Tamanho máximo = o último; Inverter no tempo (Shift+R); Quantizar posições e/ou tamanhos (seleção ou tudo) | 1.0 (quantizar: core) |
| F-NOT-012 | Quantização e tamanho de nota | Quantização: "travar na nota" (= tamanho da nota) e 1/1–1/64 mais tercinas 1/3–1/192; tamanho de nota: "última nota", 1/1–1/64, tercinas; salvos entre sessões | core |
| F-NOT-013 | Zoom e rolagem | Zoom horizontal 12,5–800 %, vertical 25–400 %; roda rola na vertical; Shift ou roda horizontal rola na horizontal; Ctrl+roda zoom no ponto do mouse; Ctrl+Alt+roda muda a quantização; Ctrl+Shift+roda muda o tamanho de nota. A grade usa a fórmula de compasso do lugar do clipe na música | core |
| F-NOT-014 | Tom, escala e acorde | Escolher tom (C–B) e escala (todas da lista de acordes/escalas) marca as notas da escala no teclado inteiro; escolher um acorde faz o clique inserir o acorde inteiro (Shift+clique insere como arpejo); marcar/desmarcar semitons, oitavas, a escala ou o acorde atual pelo menu da tecla; "selecionar todas as notas desta tecla" | 1.0 |
| F-NOT-015 | Teclado do piano | Clique toca (velocidade pela posição horizontal na tecla), arrasto faz glissando; teclas desligadas pelo mapa do microafinador aparecem diferentes; nomes das notas opcionais nas notas | core |
| F-NOT-016 | Notas fantasmas | Mostrar as notas de outro clipe por baixo (escolhido no menu do clipe), com botão para limpar | 1.0 |
| F-NOT-017 | Tocar o clipe | Espaço toca só este clipe em loop | 1.0 |
| F-NOT-018 | Gravar no editor de notas | Gravar do teclado MIDI com o clipe tocando; gravar com a música (ou o padrão) tocando ("acompanhar"); quantizar automático (configuração) | 1.0 |
| F-NOT-019 | Copiar, cortar, colar | Ctrl+X/C/V; colar no cursor, na grade; o colado fica selecionado; funciona entre projetos | core |
| F-NOT-020 | Importar e exportar clipe | Arquivo `.xpt`/`.xptz`, com a opção "só as notas selecionadas"; importar avisa que substitui | obsoleto — substituído por copiar/colar entre projetos (o formato XML não existe no novo); aberto (P-25) se quiser um arquivo de clipe |
| F-NOT-021 | Selecionar por região da régua | Ctrl+arrasto na régua seleciona as notas daquele trecho | 1.0 |
| F-NOT-022 | Abrir o editor sem clipe | Num projeto vazio, cria e abre o primeiro clipe da primeira faixa de instrumento | 1.0 |
| F-NOT-023 | Teclado do computador como piano | As teclas sem modificador tocam o instrumento da faixa | 1.0 |

### 2.2 Gravação por passos (hoje: barra do Piano Roll, `src/gui/editors/StepRecorder.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-PAS-001 | Gravar um passo de cada vez | Liga/desliga; começa no cursor (ou no 0 se a música toca); o passo tem o tamanho de nota; apertar notas (teclado MIDI ou do computador) avança um passo; notas seguras juntas formam um acorde (70 ms de tolerância); → estende o passo, ← encurta; o passo atual aparece desenhado | 1.0 |

### 2.3 Arranjo (hoje: Song Editor, `src/gui/editors/SongEditor.cpp`, `TrackContainerView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-ARR-001 | Ferramentas | Desenhar (padrão: clicar no vazio cria um clipe do tipo da faixa; arrastar move e redimensiona); Faca (clicar num clipe corta no ponto); Seleção (retângulo sobre faixas × tempo, que acompanha a rolagem e o zoom); Ctrl no modo desenhar = seleção temporária | core |
| F-ARR-002 | Inserir e remover compasso | Shift+Insert/Shift+Enter insere um compasso no cursor (empurra tudo depois); Shift+Delete/Shift+Backspace remove | 1.0 |
| F-ARR-003 | Cursor por compasso | ←/→ volta/avança um compasso (pelo mapa de andamentos); Home vai ao 0 | core |
| F-ARR-004 | Apagar e selecionar | Delete apaga os selecionados; Ctrl+A, Ctrl+Shift+A, Esc | core |
| F-ARR-005 | Dividir no cursor | S divide os clipes selecionados que cruzam o cursor (nenhum selecionado: todos), num desfazer | core |
| F-ARR-006 | Copiar, cortar, colar, duplicar | Ctrl+C/X mantêm a forma relativa; Ctrl+V cola **no cursor**, na faixa do primeiro selecionado (ou a do mouse, ou a primeira); Ctrl+D duplica logo depois, arredondado a compassos inteiros. Botão direito no vazio: Colar. Colar respeita tipos de faixa compatíveis | core |
| F-ARR-007 | Roda estilo REAPER | Roda = zoom horizontal no mouse; Ctrl+roda = altura de todas as faixas; Alt+roda ou roda horizontal = rolar na horizontal; Shift+roda ou roda sobre os cabeçalhos = rolar na vertical; Ctrl+0 volta o zoom | core |
| F-ARR-008 | Zoom | 4–400 px por compasso (padrão 128), lembrado | core |
| F-ARR-009 | Grade | 8, 4, 2, 1 compassos, 1/2, 1/4 (padrão), 1/8, 1/16 de compasso; "grade proporcional" que acompanha o zoom (64 compassos a 1/64); a grade conta compassos pelo mapa de andamentos (respeita as mudanças de fórmula) | core |
| F-ARR-010 | Criar faixas | Botões para faixa de padrão, de amostra, de áudio, de automação, pasta; faixa de instrumento nasce ao soltar um instrumento ou preset | core (no novo: tipo único; ver seção 3) |
| F-ARR-011 | Soltar arquivos no arranjo | Do gerenciador de arquivos: projeto abre (pergunta se salva); áudio **mais longo que um limite** vira faixa de áudio nova com o item no compasso do ponto (ou do cursor); áudio curto vira faixa de instrumento com o sampler carregado. Do navegador interno: amostra, instrumento (carrega em segundo plano), preset, SoundFont, VST, patch, projeto importado (MIDI), faixa arrastada (cria a cópia). Um desfazer | core (áudio) · 1.0 (o resto) |
| F-ARR-012 | Faixa selecionada | Clicar em qualquer lugar de uma faixa (cabeçalho, linha ou clipe) a torna a faixa selecionada (destacada) | core |
| F-ARR-013 | Depois de abrir um projeto | O arranjo vem para frente e rola para o início | 1.0 |
| F-ARR-014 | Grade de fundo | Faixas alternadas a cada 4 compassos (ou seguindo as fórmulas), linhas de compasso e de grade, a grade fina engrossa para manter ≥ 4 px | core |

### 2.4 Régua, loop, marcadores, regiões e andamento (hoje: régua de todo editor e a faixa de tempo do Song Editor, `TimeLineWidget.cpp`, `TempoLaneWidget.cpp`, `src/core/SongMarkers.cpp`, `TempoMap.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-REG-001 | Transporte em cada editor | Tocar/parar (Espaço), pausar/continuar (Shift+Espaço), gravar, gravar acompanhando, gravação por passos; maximizar o editor (Shift+F11). O fork esconde esses botões quando a barra principal os tem | obsoleto em parte — a barra de transporte fica visível nas duas páginas (decisão de 19/11); os atalhos ficam (core) |
| F-REG-002 | Linha do tempo própria por editor | Cada editor (música, padrão, clipe) tem cursor, início e fim do loop, loop ligado e comportamento ao parar | aberto (P-26): o novo tem um transporte só |
| F-REG-003 | Rolagem automática | 3 estados: por página, contínua, desligada | core |
| F-REG-004 | Ao parar | Volta ao início / **volta para onde começou** (padrão) / fica onde parou | core |
| F-REG-005 | Mover o cursor | Clicar/arrastar na régua | core |
| F-REG-006 | Desenhar o loop na régua | Arrastar na metade de cima da régua desenha o trecho do loop e o liga (na grade; Ctrl = tique); C liga/desliga o loop | core |
| F-REG-007 | Editar o loop com Shift | Três modos (configuração e menu da régua): dois botões (Shift+esquerdo = início, Shift+direito = fim), pegar o mais perto, alças (arrastar as pontas ou o corpo). Pontos magnéticos na grade (Ctrl desliga); passar um do outro troca; nunca tamanho zero. Menu: "início do loop aqui", "fim do loop aqui" | core |
| F-REG-008 | Selecionar uma região pela régua | Ctrl+arrasto seleciona os clipes de todas as faixas naquele trecho | 1.0 |
| F-REG-009 | Números dos compassos | Pela fórmula de compasso do mapa (rótulo em cada mudança); rareiam quando o zoom diminui | core |
| F-REG-010 | Cursor | Linha que acompanha; fica **vermelha gravando** | core |
| F-REG-011 | Marcadores com nome | M cria no cursor ("Marcador N"); duplo clique na linha de marcadores ou menu "Adicionar um marcador aqui" (pede o nome); um por tique; arrastar move; duplo clique renomeia; menu: Renomear, Cor, "Marca uma mudança de andamento" (cria/tira um ponto de andamento no mesmo lugar), Tocar daqui, Remover. **Teclas 1–9 pulam para os marcadores 1–9** (como no REAPER). Desfazível | 1.0 |
| F-REG-012 | Regiões | Arrastar na linha de regiões desenha; Shift+R cria uma sobre o loop; duplo clique cria sobre o loop ou de um compasso; arrastar move, as pontas redimensionam (mín. 1/16 de compasso); **um clique numa região a torna o loop**; sobrepostas permitidas. Menu: Renomear, Cor, Loop sobre a região, Tocar do início, **Exportar a região como áudio**, Remover | 1.0 |
| F-REG-013 | Mapa de andamentos | Pontos de andamento com BPM; duplo clique ou menu "mudança de andamento aqui" (pede o BPM); editar; **rampa até o próximo ponto**; fórmula de compasso por ponto ("Batidas no compasso" 1–máx, "Valor da batida" 1/2/4/8/16/32, "Tirar a mudança"); mudar a fórmula cola no início do compasso; arrastar move (menos o do 0), um desfazer por arrasto; mostra o BPM quando não há mudanças. Automação antiga de andamento vira pontos | core (pontos e fórmulas) · 1.0 (rampas, decididas antes da 1.0) |
| F-REG-014 | Mudar o andamento no cursor | Uma ação da janela principal | 1.0 |

### 2.5 Padrões (Beat+Bassline) (hoje: Pattern Editor, `src/gui/editors/PatternEditor.cpp`, `src/core/PatternStore.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-PAD-001 | Editor de padrões | Uma lista de padrões; cada faixa do editor tem um clipe por padrão; escolher o padrão (combo, +/−); novo padrão; clonar padrão; acrescentar faixa de amostra ou de automação ao editor; tocar o padrão | obsoleto — substituído por itens MIDI que se repetem, editados numa vista de passos (`arquitetura-nova.md`, item 2) |
| F-PAD-002 | Passos | Clique liga/desliga o passo; roda muda a velocidade ±5 (0–100); passos mais altos mais claros; acrescentar passos (+1 compasso), remover, clonar (dobra copiando). No arranjo, os passos se editam quando o zoom passa de 96 px/compasso | 1.0 (como a vista de passos de um item MIDI) |
| F-PAD-003 | Faixa de padrão e clipe de padrão | Uma faixa por padrão; o clipe toca o padrão em loop, pode começar no meio; duplo clique abre o editor; renomear | obsoleto — substituído por itens MIDI que se repetem |
| F-PAD-004 | Amostras na grade do padrão | Faixa de amostra dentro do editor de padrões, para batidas com amostras curtas | aberto (P-23): no novo, uma bateria com amostras é um instrumento (AudioFileProcessor, AvlDrums, DrumMachines) tocado pela vista de passos? |

### 2.6 Curvas de automação (hoje: Automation Editor, `src/gui/editors/AutomationEditor.cpp`, `src/core/AutomationClip.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-AUT-001 | Desenhar pontos | Clique cria um ponto (valor pela altura, no passo do parâmetro); arrastar o ponto move tempo e valor; Ctrl sem grade; Shift+clique traça uma reta de pontos desde o último (um por passo); botão direito apaga pontos arrastando. Shift+D | core |
| F-AUT-002 | Apagar pontos | Borracha (Shift+E) | core |
| F-AUT-003 | Valor de saída (salto no ponto) | Shift+C: cada ponto tem valor de entrada e de saída (degrau no ponto); botão direito volta a saída à entrada | aberto (P-27): o novo tem pontos com `curve`; o degrau é dois pontos no mesmo tique? |
| F-AUT-004 | Tangentes | Shift+T: arrastar as tangentes do ponto (trava as dele); botão direito destrava | obsoleto — substituído pela forma `curve` (−1 a 1) de cada ponto |
| F-AUT-005 | Editar o valor | Duplo clique num ponto pede o valor | core |
| F-AUT-006 | Espelhar | Vertical e horizontal, do clipe ou do trecho selecionado | 1.0 |
| F-AUT-007 | Interpolação | Discreta, linear, cúbica (Hermite) com tensão 0–1, por clipe | obsoleto — substituído pela `curve` por ponto; parâmetros discretos nunca interpolam (`arquitetura-nova.md`, item 1) |
| F-AUT-008 | Zoom, grade e rolagem | Horizontal 12,5–800 %; vertical automático (ajusta ao mín./máx.) ou 25–1600 %; quantização 1/1–1/192 com tercinas; a mesma roda do editor de notas; ↑/↓ rola, ←/→ move o cursor, Home vai ao 0 | core |
| F-AUT-009 | Fantasmas | Mostrar por baixo as notas de um clipe MIDI ou a onda de uma amostra | 1.0 |
| F-AUT-010 | Ligar um controle soltando-o | Ctrl+arrastar um botão qualquer e soltar no editor liga o controle à curva (avisa se já estava) | obsoleto — substituído por: a lane pertence ao parâmetro (a automação mora no dono, `arquitetura-nova.md`); o gesto de "mostrar a automação deste botão" é de quem desenhar |
| F-AUT-011 | Tocar a curva | Espaço, com régua e loop | 1.0 |

## 3. Faixas

No programa novo há **um tipo de faixa**, que pode ter itens de áudio e de MIDI, um instrumento, volume, mudo e solo,
**sem pan**, e uma lista de saídas para canais da mesa (`arquitetura-nova.md`, item 2). A lista abaixo diz o que cada
tipo do legado faz e para onde vai.

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-TRK-001 | Propriedades comuns | Nome, cor (opcional), mudo, solo, altura (≥ 32, padrão 56), profundidade na pasta, camada ouvida | `src/core/Track.cpp` | core |
| F-TRK-002 | Faixa de instrumento | Um instrumento; volume, pan, afinação em cents, faixa de pitch bend (1–60, 2), usar a transposição geral, nota base, primeira e última nota, canal do mixer, porta MIDI, rack de CC MIDI, armar, modo de gravação MIDI e quantização | `src/tracks/InstrumentTrack.cpp` | core (sem pan; detalhes na seção 1, janela do instrumento) |
| F-TRK-003 | Faixa de amostra | Amostras curtas na grade; volume, pan, canal; funciona no editor de padrões | `src/tracks/SampleTrack.cpp` | obsoleto — substituído pela faixa única com itens de áudio (e P-23 para as batidas) |
| F-TRK-004 | Faixa de áudio | Gravações e arquivos como itens, com camadas e fades; só na música; volume, pan, armar, monitorar, entrada, canal | `src/tracks/AudioTrack.cpp` | core (sem pan) |
| F-TRK-005 | Faixa de padrão | Uma por padrão | `src/tracks/PatternTrack.cpp` | obsoleto — substituído por itens MIDI que se repetem |
| F-TRK-006 | Faixa de automação | Guarda clipes de automação ligados a qualquer controle; soltar um controle nela cria um clipe ligado; "ligar/desligar a gravação de todos" | `src/tracks/AutomationTrack.cpp` | obsoleto — substituído pelas lanes de automação no dono do parâmetro (faixa ou canal da mesa) |
| F-TRK-007 | Pasta | Agrupa as faixas seguintes (aninhável); recolher/expandir; mudo e solo da pasta valem para dentro; volume e pan do canal de grupo; mover a pasta move o conteúdo; regra: só um nível mais fundo que a de cima, e só se ela for pasta | `src/tracks/FolderTrack.cpp` | core (organizar, recolher, mudo/solo do grupo) · obsoleto: o canal de grupo (a pasta não soma) |
| F-TRK-008 | Camadas da faixa | Todo clipe tem camada; a faixa cresce uma altura por camada; a ouvida em destaque, as outras apagadas | `Track` (lanes) | core |
| F-TRK-009 | Canal automático por faixa nova; clone ganha canal novo | | `Track` | obsoleto — ver F-MIX-002 |
| F-TRK-010 | Tamanho da faixa | Termina no fim do último clipe | `Track` | core |

### 3.1 Cabeçalho da faixa (o design 02 aprovado, sem o knob de pan)

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-TRK-020 | Número e cor | Coluna com o número da faixa sobre a cor dela (escurece quando muda); é a alça de mover | `TrackGrip.cpp` | core |
| F-TRK-021 | Armar, nome, M, S | Linha 1 do cabeçalho; o nome com ícone, cortado quando não cabe | `TrackView::buildHeader` | core |
| F-TRK-022 | Knobs de volume e pan | Knobs pequenos como no REAPER, sem régua; agem no volume e pan **da faixa** (antes do canal); numa pasta, no canal de grupo | `TrackChannelControls.cpp` | core (volume) · obsoleto: pan (decisão de 19/11) |
| F-TRK-023 | Botão FX | "FX" ou "FX n", aceso quando há efeitos; abre a cadeia da faixa (ou a do canal compartilhado) | `TrackView.cpp` | aberto (P-28): no novo os efeitos são do canal; o botão abre os inserts do canal da saída principal? |
| F-TRK-024 | Botão I/O | Canal do mixer (novo canal ou lista), entrada MIDI, menu MIDI (dispositivos de entrada e saída, rack de CC), entrada de áudio e monitoramento | `TrackView.cpp` | core (no novo: destino das saídas, entrada, monitor) |
| F-TRK-025 | Terceira linha (faixa alta) | Entrada MIDI ("Omni"/canal/"-") ou de áudio ("-"/"Entrada: N"/"N e N+1") e o número do canal; some quando o cabeçalho é estreito | `TrackView.cpp` | core |
| F-TRK-026 | Medidor do canal na borda | 6 px, RMS, −60–0 dB | `TrackLevelMeter` | core (a saída principal) |
| F-TRK-027 | Clique no nome | Abre/fecha a janela do instrumento (opção "uma janela de instrumento por vez" fecha as outras), a de efeitos, ou o editor de padrões; duplo clique ou botão direito renomeia no lugar (Esc cancela); soltar preset/plugin no nome troca o instrumento | `TrackLabelButton.cpp` | core |
| F-TRK-028 | Indicador de atividade | Pisca com as notas; fica vermelho com "áudio corrompido"; clicar toca a nota padrão | `FadeButton.cpp` | 1.0 |
| F-TRK-029 | Mover e copiar faixas | Arrastar a alça move (um desfazer); arrastar para o lado entra/sai da pasta de cima; Ctrl (Alt no Mac) arrasta uma cópia, que pode ir a outro editor ou outra instância | `TrackView.cpp` | core (mover, pastas) · 1.0 (copiar arrastando) |
| F-TRK-030 | Altura e largura | Arrastar a borda de baixo (ou Shift+arrasto) muda a altura; Shift+clique volta ao padrão; arrastar a borda direita do cabeçalho muda a largura de todos; Ctrl+Alt+roda ±1 px, Shift+Alt+roda ±5 px. O design 02 tem três alturas | `TrackView.cpp` | core |
| F-TRK-031 | Menu da faixa | Clonar (logo abaixo); Remover (pergunta, com "não perguntar de novo"); Limpar (apaga os clipes); Exportar a faixa como áudio; Exportar a faixa como MIDI; canal do mixer; Entrar na pasta de cima / Sair da pasta; MIDI; conector de pinos; "um canal do mixer por saída"; ligar/desligar a gravação da automação; Cor ▸ mudar (paleta) / padrão / sortear (48 cores) / zerar as cores dos clipes | `TrackOperationsWidget.cpp` | core (clonar, remover, limpar, cor, pastas) · 1.0 (exportar áudio/MIDI, canais por saída) · obsoleto: conector de pinos |
| F-TRK-032 | Soltar coisas numa faixa | Uma faixa do mesmo tipo solta substitui os ajustes; amostra numa faixa de áudio cria um item no ponto; preset/plugin numa faixa de instrumento carrega; controle numa faixa de automação cria a curva | `TrackContentWidget.cpp` | core (amostra, instrumento) · 1.0 (o resto) |
| F-TRK-033 | Clique na linha vazia | Cria um clipe do tipo da faixa no ponto (MIDI de 1 compasso, automação, amostra vazia, item de áudio vazio, padrão); Ctrl+clique vira seleção | `TrackContentWidget.cpp` | core (item MIDI) · aberto (P-29: item de áudio vazio faz sentido?) |
| F-TRK-034 | Menu do botão armar (instrumento) | Overdub / Substituir / Take numa camada; quantizar ao gravar | ver F-REC-101 | 1.0 |

## 4. Clipes e itens

### 4.1 Comum a todo clipe (`src/core/Clip.cpp`, `src/gui/clips/ClipView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CLP-001 | Propriedades | Início, tamanho, deslocamento do conteúdo (aparar à esquerda mantém o conteúdo no lugar), nome, cor (opcional: senão a da faixa), mudo, tamanho automático (segue o conteúdo), camada | core |
| F-CLP-002 | Mover | Arrastar o corpo; com seleção, move a seleção inteira; por padrão a **distância** anda em passos da grade (mantém o fora-da-grade); Shift = o início ou o fim cola na grade; Alt/Ctrl depois de começar = livre. Mostra a posição (compasso:tique) | core |
| F-CLP-003 | Redimensionar e aparar | Borda direita = fim; borda esquerda = aparar o início (o conteúdo fica; MIDI e automação não estendem antes do conteúdo); passos da grade, Shift = absoluto, Alt/Ctrl = livre (mín. 1 tique); mostra o tamanho | core |
| F-CLP-004 | Mover para outra faixa | Arrastar para cima/baixo leva para outra faixa do mesmo tipo (toda a seleção ou nada), um desfazer | core |
| F-CLP-005 | Selecionar e copiar com o mouse | Ctrl+clique (Alt no Mac) alterna a seleção; Ctrl+arrasto num selecionado arrasta uma **cópia** (pode ir para outra faixa ou instância); soltar um clipe sobre outro do mesmo tipo copia o conteúdo para ele | core (seleção, cópia) · 1.0 (soltar sobre outro) |
| F-CLP-006 | Mudo e apagar com o mouse | Ctrl+botão direito ou Ctrl+meio = mudo; Shift+botão direito ou botão do meio = apagar | core |
| F-CLP-007 | Faca | Um marcador segue o mouse (na grade a partir da borda esquerda, direita ou global; Ctrl/Alt livre); soltar divide em dois. MIDI: Shift ao soltar = **divisão destrutiva** (as notas são cortadas de verdade). Botão direito cancela | core (divisão) · 1.0 (destrutiva) |
| F-CLP-008 | Menu comum | Apagar · Cortar · Copiar · Colar · Mudo · Exportar o clipe como áudio · Cor ▸ mudar/padrão/sortear · Tamanho automático liga/desliga · Ouvir esta camada · Comp. Os textos viram "…seleção" com vários | core (o básico) · 1.0 (exportar, tamanho automático) |
| F-CLP-009 | Aparência | Mudo fica dessaturado; selecionado mais escuro; camada não ouvida apagada | core |
| F-CLP-010 | Área de transferência | Clipes e notas vão como texto para a área de transferência do sistema e colam em outro projeto ou instância | core (no novo: a árvore como JSON, H13) |
| F-CLP-011 | Desfazer | Um passo por arrasto, por colar, por mudança de camada, por edição de item | core (gesto = um passo, I-3.7) |

### 4.2 Itens MIDI (`src/tracks/MidiClip.cpp`, `src/gui/clips/MidiClipView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CLP-020 | Abrir no editor de notas | Duplo clique; menu "Abrir no piano roll" | core |
| F-CLP-021 | Fantasma | "Usar como fantasma no piano roll" / "no editor de automação" | 1.0 |
| F-CLP-022 | Limpar notas, juntar, cortar o que sobra | Limpar todas as notas; **Juntar a seleção** (≥ 2 clipes da mesma faixa viram um com todas as notas); tirar as notas fora dos limites visíveis | 1.0 |
| F-CLP-023 | Transpor | Pede os semitons, limitado para todas as notas ficarem em 0–127; na seleção | 1.0 |
| F-CLP-024 | Nome | Voltar ao nome padrão; mudar o nome | core |
| F-CLP-025 | Tamanho automático | O clipe na música termina no compasso inteiro depois da última nota (mín. 1 compasso) | core |
| F-CLP-026 | Desenho | Mini piano roll das notas, linhas de compasso, nome, ícone de mudo | core |

### 4.3 Clipes de amostra (`src/core/SampleClip.cpp`, `src/gui/clips/SampleClipView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CLP-040 | Carregar a amostra | Duplo clique abre o arquivo; soltar uma amostra no clipe troca | obsoleto — substituído pelo item de áudio (F-CLP-060); soltar um arquivo cria o item |
| F-CLP-041 | Crossfades com tensão | Comprimento do fade em cada ponta e a tensão da curva, com alças; Ctrl/Alt na grade | aberto (P-30): o item de áudio tem fades automáticos; curvas ajustáveis eram planejadas |
| F-CLP-042 | Inverter a amostra | Toca ao contrário | aberto (P-31): não existe no item de áudio |
| F-CLP-043 | Exportar a amostra | Grava o buffer como FLAC | 1.0 (exportar o item) |
| F-CLP-044 | Memória | Automático (arquivos longos do disco) / carregar na RAM / ler do disco | aberto (P-32) |
| F-CLP-045 | Takes | Ver F-REC-045 | obsoleto |
| F-CLP-046 | Tempo | Amostras comuns ficam em segundos (o tamanho em tiques muda com o BPM); gravações ficam em tiques | core (o `timebase` de cada item) |

### 4.4 Itens de áudio (`src/core/AudioItem.cpp`, `src/gui/clips/AudioItemView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CLP-060 | Propriedades | Arquivo, ganho −60 (mudo)–+24 dB, fade de entrada e de saída (5 ms cada), memória, camada, original + andamento (gravação), versão renderizada, lista de xruns | core |
| F-CLP-061 | Alças no item | Faixa de 7 px no alto: arrastar o canto esquerdo = fade de entrada, o direito = fade de saída (em ms), o meio = **ganho** (0,5 dB por px, Shift 0,05); um desfazer ao soltar; texto "Fade in: N ms", "Ganho: x dB" | core |
| F-CLP-062 | Crossfade automático | Itens sobrepostos na camada ouvida se cruzam com potência igual na sobreposição; fades ≤ metade do item; em quadros | core |
| F-CLP-063 | Desenho | Forma de onda do trecho, **marcas vermelhas nos xruns**, curvas de fade como se ouve, linha de ganho, nome e ganho se ≠ 0 dB, apagado fora da camada ouvida | core |
| F-CLP-064 | Menu | Memória ▸ automático (≥ 20 s do disco) / RAM / disco; "Zerar ganho e fades" | core (zerar) · aberto (memória, P-32) |
| F-CLP-065 | Normalizar | Só planejado | 1.0 |
| F-CLP-066 | Curvas de fade ajustáveis | Só planejado | aberto (P-30) |
| F-CLP-067 | Um cursor por voz | Até 32 itens soando ao mesmo tempo por faixa | obsoleto — limite do motor velho |

### 4.5 Clipes de automação (`src/core/AutomationClip.cpp`, `src/gui/clips/AutomationClipView.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CLP-080 | Vários controles numa curva | Um clipe liga a vários controles; o menu "N conexões" desliga cada um; soltar um controle no clipe acrescenta (reescalando os pontos) | obsoleto — substituído por uma lane por parâmetro (P-33 se quiser "uma curva para vários") |
| F-CLP-081 | Gravar a automação | Ligar a gravação no clipe: mexer no controle tocando grava os pontos (só as mudanças); ícone vermelho | 1.0 (gravação de automação, I-6.4) |
| F-CLP-082 | Abrir, limpar, nome, espelhar | Duplo clique abre o editor; Limpar; nome padrão = o do controle; espelhar vertical/horizontal | core (abrir) · 1.0 (o resto) |
| F-CLP-083 | Tamanho | No mínimo 1 compasso | obsoleto — a lane do parâmetro cobre a música inteira |


## 5. Gravação

A gravação é o objetivo do fork. Fontes: `legado/docs/gravacao-como-usar.md` (o guia do usuário), `plano-gravacao.md`,
`gravacao-levantamento.md` (as decisões de 12/11), `pendencias.md`, e o código. Onde o legado só planejou e não fez,
a linha diz "só planejado".

### 5.1 Faixas, entradas, armar e monitorar

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-001 | Faixa que grava áudio | A faixa de áudio é a faixa que grava. A faixa de instrumento grava notas. Pasta, padrão e automação não gravam. A faixa de amostra deixou de gravar (13/11); projetos antigos com gravações em faixa de amostra viram faixa de áudio ao abrir | `src/tracks/AudioTrack.cpp`, `AudioInputTrack.cpp`, `Transport::canRecord` | core — no programa novo há um tipo só de faixa, com itens de áudio e de MIDI |
| F-REC-002 | Escolher a entrada da faixa | Nenhuma; uma entrada mono (qualquer canal do dispositivo, com o nome que o driver dá); ou um par estéreo **só ímpar-par** (1/2, 3/4…). Mostra "o dispositivo não tem entradas" quando não há. O botão mostra "-", "1", "3" ou "1/2" | menu de entrada no cabeçalho, `src/gui/tracks/SampleTrackView.cpp` | core (pares quaisquer ou só ímpar-par: decidir no desenho) |
| F-REC-003 | Uma entrada em várias faixas | A mesma entrada alimenta várias faixas ao mesmo tempo | `AudioInput` | core |
| F-REC-004 | Erros ao gravar sem entrada | "X está armada mas não tem entrada"; "X grava da entrada N, que o dispositivo não tem" | `Song::record` | core (como aviso, I-14) |
| F-REC-005 | Armar | Botão ● por faixa (áudio e instrumento), vermelho quando armada; várias faixas armadas juntas | `m_recordArmModel` | core |
| F-REC-006 | Gravar sem nada armado | Arma a faixa selecionada se ela grava; senão explica como armar | `TransportBar::record`, `Transport::record` | core (decisão de 12/11 mantida) |
| F-REC-007 | Monitoramento por faixa | Desligado / Ligado / **Auto** (padrão: armada e parada ou gravando; não quando só tocando). Mudo e solo da faixa calam o monitor. O sinal monitorado passa pelos efeitos e pelo canal do mixer | `src/core/InputMonitorHandle.cpp` | core |
| F-REC-008 | Gravar seco | O arquivo é a entrada sem efeitos; efeitos só no monitor e na reprodução | `AudioRecorder` | core |
| F-REC-009 | Medidor de entrada no cabeçalho | −60–0 dBFS, verde, amarelo de −12, vermelho de −3, retenção do pico; mostra a entrada mesmo sem monitorar (para ajustar o nível antes de gravar) | `InputLevelMeter`, `SampleTrackView.cpp` | core |
| F-REC-010 | Desligar plugins com latência na faixa armada | Só planejado | `plano-gravacao.md` §6 | aberto (P-10) |
| F-REC-011 | Efeitos de entrada gravados, gravar a saída, entrada com mais de 2 canais | Só planejado (lista do REAPER) | `gravacao-levantamento.md` §3 | aberto (P-10) |

### 5.2 Transporte de gravação

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-020 | Gravar parado | Toca do cursor menos o pré-roll e grava as faixas armadas; os takes começam no cursor; ao parar, o cursor volta para onde estava | `Song::record`, `src/core/Transport.cpp` | core |
| F-REC-021 | Punch-in e punch-out manuais | Gravar **tocando** começa a gravar no ponto ouvido; apertar Gravar de novo para de gravar e a música continua; Parar encerra tudo | `Song::stopRecording` | core |
| F-REC-022 | Auto-punch (por região de tempo) | Só planejado: gravar a passada toda e recortar o item à região de punch | `plano-gravacao.md` §6 | 1.0 (o desenho novo já tem `transport.punch`) |
| F-REC-023 | Pré-roll | 0, 1, 2 ou 4 compassos (até 16 no código), contados pelo mapa de andamentos; escolhido nas configurações e num menu do botão de pré-roll | `recording/prerollbars` | core |
| F-REC-024 | Contagem | O metrônomo toca durante o pré-roll, mesmo desligado (padrão: desligada) | `recording/countin` | core |
| F-REC-025 | Metrônomo | Liga/desliga no transporte; toca "tocando e gravando" ou "só gravando"; volume −60–+12 dB (0); acento no primeiro tempo (ligado), com dois sons; segue as mudanças de compasso; não entra na exportação nem no mixer | `src/core/Metronome.cpp` | core (som próprio e saída própria do dispositivo: I-5.5) |
| F-REC-026 | Estado do transporte em palavra e cor | PARADO / PAUSADO / TOCANDO / GRAVANDO / "PRÉ-ROLL n…" / "CONTAGEM n…"; o relógio muda de cor (verde, vermelho, amarelo) | `TransportBar.cpp` | core (design 3 aprovado) |
| F-REC-027 | Um desfazer tira a gravação inteira | Áudio e MIDI de todas as faixas armadas; ao parar, a gravação sempre fica | `Song::record` (grupo do diário) | core |
| F-REC-028 | Trocar de dispositivo durante a gravação é recusado | "muda quando a gravação acabar" | `AudioEngine` | core |
| F-REC-029 | Gravar MIDI no clipe aberto no editor de notas com a música tocando | Uma opção no menu do botão Gravar | `TransportBar.cpp` | 1.0 (ver F-NOT, gravação no editor de notas) |

### 5.3 Loop, takes, camadas e comp

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-040 | Gravar em loop | Cada passada completa vira um take numa camada; uma passada com menos de 90% da mais longa (parada no meio) é descartada | `Song::landOnLanes` | core |
| F-REC-041 | "Takes em camadas" (padrão) | As passadas novas entram **por cima**; a última toca; as antigas ficam abaixo, apagadas | `recording/lanes`, `Track::insertLanesOnTop` | core |
| F-REC-042 | Modo "gravador simples" | Com "takes em camadas" desligado, o take novo substitui o que cobre (os arquivos ficam), em loop só fica a última passada, e as camadas somem da tela | `Track::clearSpan` | aberto (P-11) |
| F-REC-043 | Ouvir uma camada | "Ouvir esta camada" escolhe qual camada toca | menu do clipe, `ClipView.cpp` | 1.0 |
| F-REC-044 | Comp por menu | Dividir o item numa camada de baixo e "Comp: usar este clipe na camada ouvida" (substitui o que cobre, um desfazer). Comp arrastando pelas camadas: só planejado | `ClipView.cpp` | obsoleto — substituído pela lista `comp` do item (trecho → camada, crossfade de 5 ms editável por fronteira, `decisoes-autonomas.md`); o gesto de comp é de quem desenhar |
| F-REC-045 | Takes de clipe de amostra (mecanismo antigo) | Menu "Takes": escolher o take N, remover o ativo; o nome mostra "(take 2/3)" | `SampleClipView.cpp` | obsoleto — substituído pelas camadas |
| F-REC-046 | Takes presos ao andamento | O take guarda o andamento em que foi gravado e segue as mudanças (ver F-REC-090) | `Song::tempoKeyTicks` | 1.0 (itens gravados nascem `musical`) |

### 5.4 Arquivos e motor de gravação

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-060 | Formato do arquivo | WAV 32 bits float, intercalado, um arquivo por faixa e por passada. FLAC/24 bits: só planejado | `src/core/AudioRecorder.cpp` | obsoleto — substituído por RF64, um arquivo contínuo por entrada com os cortes de loop marcados (`decisoes-autonomas.md`); escolher formato fica aberto (P-12) |
| F-REC-061 | Nome e lugar dos arquivos | `<faixa>_<n>.wav` em `<projeto>_media/`; projeto não salvo grava em `recordings/unsaved-<data>/` (limpa ao fechar sem salvar; pastas órfãs limpas na partida) | `src/core/ProjectMedia.cpp` | obsoleto — substituído por `audio/` dentro da pasta do projeto, nome provisório e depois o hash (C10) |
| F-REC-062 | Copiar os arquivos usados ao salvar | Todo arquivo usado (fora os de fábrica) é copiado para a pasta de mídia; "Salvar como" leva os arquivos | `ProjectMedia` | core (no novo: importar copia para `audio/`) |
| F-REC-063 | Lixo dos arquivos gerados | Arquivos gerados que o projeto salvo não usa vão para a **lixeira do sistema** ao fechar; nunca toca em arquivo do usuário | `.linuxmusic-generated` | aberto (P-13): o novo diz só que nada em `audio/` é apagado enquanto o histórico o cita (H4) |
| F-REC-064 | Escrita contínua e segura | Anel de 10 s até uma thread de disco; o cabeçalho é atualizado a cada 1 s, então uma queda perde no máximo um segundo | `AudioRecorder` | core |
| F-REC-065 | Disco lento | No legado, o que falta vira silêncio no lugar certo e uma mensagem com os ms perdidos | `AudioRecorder` | obsoleto — substituído por: anel cheio é erro explícito, aquela entrada para e o arquivo fecha no último bloco bom (I-11) |
| F-REC-066 | Falha de escrita | O take para e guarda o que foi escrito, com mensagem | `AudioRecorder` | core |
| F-REC-067 | Faixa apagada durante a gravação | Os arquivos ficam, com mensagem | `Song` | core (vira "Take recuperado" numa faixa nova, H8) |
| F-REC-068 | Onda ao vivo | O take cresce em vermelho do início ao cursor, com a forma de onda ao vivo (20 por segundo); ao parar, o item já aparece desenhado | `src/gui/tracks/LiveTakeOverlay.cpp` | core |
| F-REC-069 | Recuperar takes depois de uma queda | Autosave 2 s depois de cada mudança (no máximo a cada 30 s), também tocando e gravando; na partida seguinte oferece recuperar; os takes em andamento são consertados e entram nas camadas | `src/core/Autosave.cpp`, `src/core/WavRepair.cpp` | core (diário + takes órfãos de `audio/`) |
| F-REC-070 | Mudar o andamento no meio da gravação | Não corta o take | `AudioRecorder` | core |

### 5.5 Latência e xruns

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-080 | Alinhar o take | Cada quadro capturado volta ao período que foi ouvido (latência de entrada + saída + calibração); o take cai na grade com precisão de amostra. A latência do mixer (PDC) **não** entra | `Song::recordingLatency` | core (a do caminho até o master entra no novo) |
| F-REC-081 | Calibração por loopback | "Medir…": ligar a saída na entrada com um cabo; toca um pulso MLS de 127 amostras, ouve 1,5 s em todas as entradas, acha a correlação; mensagens de sucesso e de "o pulso não voltou, confira o cabo" | `src/core/LatencyCalibration.cpp`, configurações | core |
| F-REC-082 | Ajuste manual | −48000–+48000 quadros. Em ms: só planejado. Entrada e saída separadas: não | configurações → Gravação | core (por dispositivo; unidade a decidir) |
| F-REC-083 | Latência real mostrada | Ao lado do tamanho do bloco | configurações → Áudio | core |
| F-REC-084 | Xruns marcados no take | O arquivo continua (silêncio no buraco); os pontos ficam salvos e aparecem em vermelho no take; mensagem no fim "o áudio falhou N vezes". ALSA, CoreAudio e PortAudio contam; JACK não | `AudioDevice::reportXrun` | core |
| F-REC-085 | Falha por trava do motor | Se a interface segura a trava mais de meio período, o período sai mudo e conta como falha | `AudioEngine` | obsoleto — o motor novo não tem trava (`arquitetura-nova.md`) |
| F-REC-086 | Contador global de xruns fora da gravação | Só planejado | — | aberto (P-14) |

### 5.6 Gravação MIDI

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-100 | Gravar notas na linha do tempo | Faixas de instrumento armadas gravam o que chega pela entrada MIDI delas, com o mesmo punch e pré-roll; teclas seguras terminam no Parar; grava a velocidade | `src/core/MidiRecorder.cpp` | 1.0 (logo depois da primeira fatia) |
| F-REC-101 | Modos de gravação MIDI | Overdub (padrão: as notas vão para o clipe que cobre o lugar ou para um novo no compasso); Substituir (as notas do trecho somem); Take numa camada | menu do ● da faixa de instrumento | 1.0 |
| F-REC-102 | Quantizar ao gravar | Grade desligada (padrão) / 1/4 / 1/8 / 1/16 / 1/32 / 1/64; força 50 / 75 / 100 %; só o início da nota | mesmo menu | 1.0 |
| F-REC-103 | Entrada MIDI da faixa | Receber liga/desliga, dispositivos, canal (todos, padrão) | botão MIDI no cabeçalho | 1.0 |
| F-REC-104 | CC, pitch bend, aftertouch, program change na gravação; um take MIDI por passada de loop; compensar a latência MIDI | Não feitos | — | 1.0 (I-21.4) |

### 5.7 Andamento e itens gravados

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-090 | Re-renderizar takes quando o andamento muda | Mudar o BPM ou o mapa de andamentos avisa "vai renderizar N itens, ~X s", com progresso; Cancelar desfaz a mudança. Rubber Band R3 a partir do **original**; sem Rubber Band, reamostra (muda a altura) e avisa. Aprende a velocidade da máquina para a estimativa | `src/gui/TempoRenderController.cpp`, `src/core/TimeStretcher.cpp` | 1.0 (time-stretch antes da 1.0; operação de fundo, I-2) |

### 5.8 Dispositivos de áudio e MIDI

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-REC-120 | Trocar de dispositivo sem reiniciar | As mudanças valem na hora (reabre 0,4 s depois da última); avisa quando não abre e qual foi usado; avisa quando a taxa do dispositivo é outra ("o áudio é convertido") | `AudioEngine::reopenAudioDevice` | core |
| F-REC-121 | Taxa de amostragem | 44,1 / 48 / 88,2 / 96 / 176,4 / 192 kHz para projetos novos; **o projeto abre na própria taxa** | configurações → Áudio | core (taxa fixa por projeto; trocar a do projeto é uma operação) |
| F-REC-122 | Tamanho do bloco | 32–4096 quadros (de 32 em 32 até 256, depois de 256 em 256), padrão 256, botão de voltar ao padrão | configurações → Áudio | core |
| F-REC-123 | ALSA | Dispositivo de saída e de entrada ("nenhum", "o mesmo da saída", ou outro); prefere `hw:` exclusivo e reserva a placa do PipeWire/PulseAudio pelo ReserveDevice1; `default`/`pipewire`/`pulse` compartilhados; formatos nativos float/S32/S24/S16; captura abre todos os canais (até 64); `snd_pcm_link` com aviso se não liga. Nunca rodou numa máquina Linux | `src/core/audio/AudioAlsa.cpp` | core |
| F-REC-124 | Dois dispositivos ALSA diferentes | Só desligados um do outro, sem compensar deriva | `AudioAlsa.cpp` | obsoleto — substituído por: combinações sempre explícitas (U7) |
| F-REC-125 | JACK | Nome do cliente; número de entradas 0–32 (2), cada uma com a conexão; 2 saídas com conexão; liga às portas físicas por padrão; latência do JACK; mensagens de servidor caído/reiniciado; JACK MIDI no mesmo cliente. Sem contagem de xruns | `AudioJack.cpp` | 1.0 (depois do ALSA) |
| F-REC-126 | CoreAudio (macOS) | Saída e entrada; dispositivos diferentes viram um agregado privado com compensação de deriva; a entrada só abre quando precisa (pede permissão do microfone na hora) | `AudioCoreAudio.cpp` | 1.0 (macOS depois; o agregado implícito fica obsoleto — substituído por combinação explícita) |
| F-REC-127 | PortAudio, PulseAudio, sndio, SDL | Outros backends; Pulse, sndio e SDL só tocam (sem entrada) | `src/core/audio/` | obsoleto — substituído por ALSA direto e JACK/PipeWire (`arquitetura-nova.md`, item 14) |
| F-REC-128 | Dispositivo falso | Sem placa; 2 entradas silenciosas opcionais (ou um tom) para testar | `AudioDummy` | core (dispositivo nulo do `lm-headless` e dos testes) |
| F-REC-129 | Saída só estéreo | Todo driver toca 2 canais; sem saídas multicanal | `DEFAULT_CHANNELS` | obsoleto — o novo escolhe os canais de saída do master no dispositivo |
| F-REC-130 | MIDI | ALSA seq, ALSA raw, JACK MIDI, CoreMIDI (hotplug, portas virtuais), sndio, WinMM; reabre ao vivo e religa as faixas aos mesmos dispositivos, avisando os que sumiram | `src/core/midi/` | core: ALSA seq · 1.0: JACK MIDI, CoreMIDI, ALSA raw (aberto, P-16) · obsoleto: sndio, WinMM (sem Windows) |
| F-REC-131 | "Controlador para a faixa selecionada" | O teclado MIDI toca sempre a faixa selecionada | configurações → MIDI | 1.0 |

## 6. Mixer e mesa analógica

No legado, o mixer (página "Mixagem", Ctrl+5 alterna com a Composição) é o FX Mixer do LMMS transformado numa mesa
analógica. No programa novo, **faixa e mesa são separadas**, com roteamento livre (`arquitetura-nova.md`, item 2), e
**o rack da mesa analógica é portado** para o `lm-dsp` (já em andamento, com A/B). Fontes: `legado/docs/multicanal.md`,
`preparacao-mixer.md`, `plano-compressores.md`, `pesquisa-compressores.md`, e o código (`src/core/Mixer.cpp`,
`AnalogChannelStrip.cpp`, `DeskDynamics.cpp`, `DeskFilters.cpp`, `DeskTolerance.cpp`, `GainStage.cpp`,
`TubeStage.cpp`, `MasterSection.cpp`, `MonoBass.cpp`; `src/gui/MixerView.cpp`, `MixerChannelView.cpp`,
`MasterSectionView.cpp`).

### 6.1 Canais

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-001 | Modo analógico ou digital da mesa inteira | Analógico: todo canal (fora o master) tem o rack; os efeitos aparecem como LEDs. Digital: sem processamento embutido, os efeitos aparecem com nome (5 visíveis, "+N"). Os ajustes do rack ficam guardados no digital. Configuração "projetos novos usam a mesa analógica" (padrão: não) | `Mixer::m_analogModel` | aberto (P-17): no novo todo canal tem o rack, com seções que se ligam |
| F-MIX-002 | Canal automático por faixa | Cada faixa de instrumento, amostra ou áudio ganha um canal com o nome e a cor dela, que segue a faixa; pasta ganha um canal de grupo | `Mixer`, `Track` | obsoleto — substituído por: faixa nova manda para o master; a ação "criar canais para as saídas" faz isso quando o usuário pede; a pasta não soma |
| F-MIX-003 | Várias faixas num canal | Cada uma com o próprio volume | `Mixer` | core |
| F-MIX-004 | Criar canal | Um "canal fantasma" tracejado depois do último (clique cria), Shift+Insert; numerados sem buracos | `MixerView.cpp` | core |
| F-MIX-005 | Apagar canal | Delete, menu "Remover canal" (pergunta se está em uso, com "não perguntar de novo" e a configuração correspondente), "Remover canais sem uso". Master e retornos não se apagam. Desfazível | `MixerView.cpp` | core (no novo: as saídas que mandavam para ele vão ao master, com aviso) |
| F-MIX-006 | Renomear canal | Duplo clique, Enter/F2, menu; renomear o canal de uma faixa renomeia a faixa | `MixerChannelView.cpp` | core (a ligação com o nome da faixa fica obsoleta) |
| F-MIX-007 | Cor do canal | Mudar, voltar ao padrão, sortear | menu do canal | 1.0 |
| F-MIX-008 | Reordenar | Alt+←/→, menu mover; "Organizar os canais como as faixas" (ação, nunca automático, um desfazer) | `MixerView.cpp` | 1.0 |
| F-MIX-009 | Selecionar canal | Clique; ←/→; Shift/Ctrl+duplo clique no número do canal da faixa abre a Mixagem nele | `MixerView.cpp` | 1.0 |
| F-MIX-010 | Escolher o canal da faixa | Número no cabeçalho: digitar, ou menu "Mandar para: novo canal / lista" | `MixerChannelLcdSpinBox` | core (no novo: o destino de cada saída da faixa) |
| F-MIX-011 | Recolher os canais de uma pasta | Triângulo no canal de grupo esconde os filhos (só vista); os filhos têm uma linha da cor do grupo | `MixerView.cpp` | aberto (P-18): sem canal de grupo automático, como agrupar na mesa? |
| F-MIX-012 | Um canal por saída / por peça | Cria um canal de grupo "<faixa> (peças)" e um canal por saída do instrumento, roteados | menu da faixa, AvlDrums | core (a ação "criar canais para as saídas") |
| F-MIX-013 | Teclado na mesa | Delete, ←/→ seleciona, Alt+←/→ move, ↑/+ e ↓/− mudam o fader do canal selecionado (1 dB; Shift 3 dB; Ctrl 0,1 dB), Shift+Insert cria, Enter/F2 renomeia | `MixerView::keyPressEvent` | 1.0 |
| F-MIX-014 | Mudo e solo do canal | Solo como no REAPER: os canais solados, o caminho até o master e as fontes deles tocam; vários solos; o solo não mexe nos botões de mudo; fontes de sidechain continuam alimentando. Canal mudo continua processando (medidores em zero, sem salto) | `Mixer::applySolo` | core (as regras dos dois solos: `arquitetura-nova.md`, item 2) |
| F-MIX-015 | Mudo do master no transporte | Botão de alto-falante | `TransportBar.cpp` | 1.0 |
| F-MIX-016 | "Canais da faixa" (pares 2–16) | Quantos pares o canal tem; o par 1/2 passa pela mesa, os outros só pelos efeitos | menu do canal | obsoleto — substituído pelo número de canais de cada canal da mesa e as conversões explícitas |

### 6.2 Roteamento, envios e latência

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-020 | Envios | Cada canal manda para outros com quantidade 0–1 e pré ou pós fader (pré: depois dos efeitos e antes da seção estéreo). Interface estilo FL: com um canal selecionado, cada outro mostra um botão de envio, a quantidade e uma seta de retorno. **O botão de pré-fader não existe** (só o modelo) | `MixerRoute`, `SendButtonIndicator.cpp` | core (pré/pós com botão) |
| F-MIX-021 | Recusar laços | Um envio que criaria laço é recusado | `Mixer` | core |
| F-MIX-022 | Roteamento que segue as pastas | O canal da faixa vai para o canal de grupo da pasta | `updateFolderRouting` | obsoleto — a pasta não soma; roteamento livre |
| F-MIX-023 | Dois retornos de efeito fixos (FX 1, FX 2) | Sempre presentes, depois dos canais; cada canal tem dois botões FX1/FX2 que são os envios pós-fader (padrão 0) | `FxReturnCount` | aberto (P-19): no novo um retorno é um canal comum; ter botões rápidos de envio em todo canal é questão de desenho |
| F-MIX-024 | Sidechain | "Entrada de sidechain de ▸" e "Mandar como sidechain para ▸" no canal; "Fonte de sidechain" no efeito com mais de 2 entradas | menu do canal e do efeito | 1.0 (envio de sidechain para a porta do insert, item 3) |
| F-MIX-025 | Compensação de latência (PDC) | Cada canal = a entrada mais atrasada + os efeitos; cada entrada e envio é atrasado para alinhar; o seco dos efeitos com latência também | `Mixer::compensateLatency` | core |
| F-MIX-026 | Limpar áudio corrompido | Canal que solta NaN/inf é silenciado (LED vermelho no efeito); configuração "Mix sanitization" | `Mixer` | core |

### 6.3 Caminho do sinal num canal

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-030 | Trim | −60–+24 dB, 0 (−60 = desligado); limpo | `m_trimModel` | core |
| F-MIX-031 | Mono do canal | (L+R)/2 dos dois lados, depois do trim, com crossfade de um período | `m_monoModel` | 1.0 |
| F-MIX-032 | Ponto de inserção dos efeitos | Pré-ganho / **Pós-ganho** (padrão) / Pós-filtro / Pós-dinâmica / Pós-EQ / Pré-pan (pós-fader). No digital, todos antes do fader menos o último | `m_fxInsertModel` | aberto (P-20): onde os inserts entram em relação ao rack no novo |
| F-MIX-033 | Inverter fase e trocar L/R | Botões Ø e ⇄ | `m_phaseInvertModel`, `m_swapChannelsModel` | 1.0 |
| F-MIX-034 | Largura (width) | 0–200 %, 100, mid/side; tirada da tela (o modelo ainda processa) | `m_widthModel` | aberto (P-20): some ou não |
| F-MIX-035 | Fader | Linear 0–4 (+12 dB), 1 = 0 dB; mapa cúbico em dB; abaixo de −120 dB = −inf; clicar no botão pega sem pular, clicar fora pula; roda ±1 dB (Shift 3, Ctrl 0,1); duplo clique pede o valor em dB; marcas a cada 6 dB opcionais | `m_volumeModel`, `src/gui/widgets/Fader.cpp` | core (no novo: duplo clique volta a 0 dB, decisão de 19/11) |
| F-MIX-036 | Pan | −100–+100, 0, lei linear com unidade no centro | `m_panModel` | core |

### 6.4 O rack analógico do canal (todos os parâmetros)

Status de toda a seção: **1.0** (o DSP já está sendo portado para o `lm-dsp` com A/B). Seções neutras são
transparentes bit a bit (menos o cruzamento do 4558); toda mudança é suave, sem estalo.

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-040 | GAIN | Posição 0–1, padrão 0,25 (0 dB a um quarto do curso); 0 = mudo; até 0,25 cúbico (−18 dB em 0,125); acima, linear em dB até **+60 dB**; mostra dB e "-inf". Subir o ganho satura o estágio; o fader devolve o nível | `AnalogChannelStrip`, `GainKnob` | 1.0 |
| F-MIX-041 | Modelo do estágio de ganho | FLAT, 4558 (padrão), TL072, 5532, 5534, 12AX7, 12AX7 V (frio), 12AX7 H (quente), 12AT7, 12AU7. Clique = próximo, botão direito = lista com descrição. Op-amps com joelho, trilho, slew, corrente e (4558) cruzamento sempre ligado; válvulas pelo modelo de Koren com corrente de grade | `src/core/GainStage.cpp`, `TubeStage.cpp` | 1.0 |
| F-MIX-042 | Válvulas leve ou realista | Lâmpada do filamento: amarela "LT" (leve, tabelas, ~0,6 % de um núcleo) ou vermelha "RL" (realista, solução nodal com Newton e sobreamostragem 4×, ~10 %); clique alterna | `m_realisticModel` | 1.0 |
| F-MIX-043 | LEDs SAT e CLIP | SAT acende com a saturação (some devagar); CLIP vermelho 3 dB antes do limite; nas válvulas a lâmpada brilha com a saturação | `SaturationLeds` | 1.0 |
| F-MIX-044 | LCUT | 100 Hz, Butterworth de 3ª ordem (18 dB/oit), antes do estágio de ganho | `m_lowCutModel` | 1.0 |
| F-MIX-045 | HPF e LPF | HPF 20–1000 Hz (20 = desligado), LPF 1000–20000 Hz (20000 = desligado), 12 dB/oit, casados com o analógico até Nyquist | `DeskFilters` | 1.0 |
| F-MIX-046 | Mono bass | BASS 0–100 % (0 = desligado, 100 = graves todos em mono), FREQ 20–300 Hz, 120; cruzamento LR24 | `src/core/MonoBass.cpp` | 1.0 (a pendência do usuário sobre o sentido do botão já está resolvida: 0 = desligado) |
| F-MIX-047 | Compressor: 9 modelos | SSL (padrão), API 2500, dbx 160, 1176, LA-2A, 670 (Fairchild), Gates Sta-Level, Altec 436, RCA BA6A, cada um com cor, filtros e transformadores próprios | `src/core/DeskDynamics.cpp` | 1.0 |
| F-MIX-048 | COMP | 0–1, 0: limiar (SSL +4…−26 dBu; API +10…−20; dbx +12…−38), "PR n" no LA-2A, ou ganho de entrada 0–40 dB nos de limiar fixo | `m_compressorAmountModel` | 1.0 |
| F-MIX-049 | RATIO | 0–1, 0 = desligado (bit a bit): SSL/dbx 20^pos (∞ no fim); API 1,5/2/3/4/6/10/∞; 1176 4/8/12/20/ALL; LA-2A COMP/LIMIT; Gates 1,6+4,4·pos; Altec, Fairchild com leitura própria | `m_compressorRatioModel` | 1.0 |
| F-MIX-050 | ATK | FAST / NORM (padrão) / SLOW: o ataque mais rápido, o usual e o mais lento que o modelo permite | `m_compressorSpeedModel` | 1.0 |
| F-MIX-051 | REL | 0–1, 0,5 (valor de fábrica), na unidade de cada modelo (SSL 0,1–0,3–4 s; API 0,05–0,3–3; 1176 0,05–0,25–1,1; LA-2A fator; 670 TC1–6; dbx automático) | `m_compressorReleaseModel` | 1.0 |
| F-MIX-052 | Compensação automática e link | Ganho de compensação automático em todos (0 VU = −18 dBFS RMS sai onde entrou); estéreo ligado | `DeskDynamics` | 1.0 |
| F-MIX-053 | Gate | Limiar −30–+10 dBu, −10; RANGE 0–40 dB, 0 (= fora); modos STD (padrão), KICK, SNARE, TOMS, VOX, GTR, BASS, NOISE, EXP, cada um com hold, release e filtro de chave próprios; histerese do SSL | `m_gateModel`, `m_gateModeModel` | 1.0 |
| F-MIX-054 | EQ de 4 bandas (Soundcraft/Mackie) | HI shelf 12 kHz, HMF pico 2,5 kHz, LMF pico 400 Hz, LO shelf 80 Hz; cada um ±15 dB, 0; frequências **fixas**; Q proporcional | `DeskEq` | 1.0 |
| F-MIX-055 | Tolerância dos componentes | Sempre ligada: desvios fixos por canal (ganho ±0,1 dB; frequências ±1 %; ganhos do EQ ±2 %; ponto de saturação ±3 %), tirados do **número** do canal, iguais em toda plataforma; mover o canal muda o caráter | `src/core/DeskTolerance.cpp` | 1.0 (aberto, P-21: semente pelo número ou pelo id do canal) |

### 6.5 Master

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-060 | Ordem do master | PRE → DRIVE (analógico) → efeitos do master → ESTÉREO → BUS COMP → POST → XFMR (analógico) → monitoração. Sem pan, sem rack de canal, sem envios | `src/core/MasterSection.cpp` | 1.0 |
| F-MIX-061 | Faders PRE L e PRE R | 0–2 (+6 dB), 1, com medidores próprios; LINK (ligado: andam juntos) | `m_preLeftModel`, `m_preLinkModel` | 1.0 |
| F-MIX-062 | POST | O volume do master, até +12 dB | `m_volumeModel` | core |
| F-MIX-063 | DRIVE do master | Mesmo seletor de estágio de ganho, LEDs e lâmpada do canal | `MasterSection` | 1.0 |
| F-MIX-064 | XFMR | Transformador de saída estilo Neve depois do fader (desligado) | `m_transformerModel` | 1.0 |
| F-MIX-065 | ESTÉREO | WIDTH 0–200 %; BASS mono 0–100 %, 0, e FREQ 20–300 Hz, 120; AIR (shelf só no lado, 12 kHz) −6–+6 dB, 0 | `MasterSection` | 1.0 |
| F-MIX-066 | BUS COMP (SSL G) | IN (desligado); THR −15–+15, 5; RATIO 2/4/10 (4); ATK 0,1/0,3/1/3/10/30 ms (10); REL 0,1/0,3/0,6/1,2 s/Auto (Auto); MAKE −5–+15 dB, 0; ponteiro de redução de ganho 0–20 dB. HPF de sidechain: só planejado | `MasterSection` | 1.0 |
| F-MIX-067 | Monitoração | LEVEL −60–+6 dB, DIM (−20 dB), CUT, MONO, SIDE (exclusivos). **Não salva, não desfaz, não vai para a exportação** | `m_monitorLevelModel` | 1.0 |
| F-MIX-068 | Medidores do master | Dois VU de ponteiro (−20–+3 VU, 0 VU = −18 dBFS RMS, 300 ms), correlação (+1 a −1), goniômetro | `MasterSectionView.cpp` | 1.0 |

### 6.6 Medidores, botões e efeitos na mesa

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-MIX-070 | Medidor do canal | Pós-fader, −42–+9 dBFS, faixas verde/amarela/vermelha, marca de 0 dBFS; pico sobe na hora e cai; linha de pico segura 1,5 s; indicador numérico do pico acima do fader (clique zera). Escolher tipo de medidor (VU, PPM, K): só planejado | `src/core/ChannelMeter.cpp`, `Fader.cpp`, `PeakIndicator.cpp` | core (tipos: aberto, P-22) |
| F-MIX-071 | Medidor no cabeçalho da faixa | O RMS do canal da faixa na borda direita do cabeçalho | `TrackLevelMeter` | core (no novo: a saída principal da faixa, com indicação de outras) |
| F-MIX-072 | Botões da mesa | Tampa colorida (cores SSL/Soundcraft), arco a partir do centro, texto na unidade do modelo; botões de escolha (clique = próximo, direito = lista com descrição) | `DeskKnob.cpp`, `ChoiceButton` | 1.0 (o visual é de quem desenhar) |
| F-MIX-073 | Efeitos no canal | Ver, ligar/desligar, abrir, reordenar, trocar, remover, presets; arrastar um efeito do navegador para um canal o acrescenta; o painel do canal selecionado mostra a cadeia inteira | `MixerChannelView.cpp`, `EffectRackView.cpp` | core (ver F-FX-002) |

## 7. Instrumentos, efeitos e plugins de terceiros

Regras gerais do legado que valem para quase todo plugin:
- Todo parâmetro de plugin é automatizável e pode ser ligado a um controlador (seção 8), salvo onde está dito.
  Desenhos de onda e tabelas não são automatizáveis.
- Um instrumento "por nota" toca cada nota numa voz própria do host (polifônico); um instrumento de "fluxo único"
  administra as próprias vozes (o host manda MIDI). O tempo de release que o host dá depois do note-off vem do plugin.
- O formato dos parâmetros abaixo é `Nome: mín–máx, padrão, passo/unidade`. "Log" quer dizer botão em escala
  logarítmica. "Sync" quer dizer que o botão pode seguir o andamento (valores em notas: 1/4, 1/8 etc.).
- No programa novo cada plugin é reescrito no contrato novo (`arquitetura-nova.md`, peça 7): os parâmetros ficam com
  chave de texto permanente, as escolhas por chave e não por número. O inventário abaixo é a lista do que cada um tem
  que voltar a ter. As esquisitices marcadas "quirk" são comportamento do legado: o porte decide se mantém (a
  compatibilidade com projetos antigos não conta, porque o `.mmp` não abre).

### 7.1 Instrumentos

#### F-INS-001 AudioFileProcessor (sampler simples) — core

- **O que faz:** toca uma amostra; a altura segue a nota em relação à nota base. Pontos de início, fim e volta,
  inversão, três modos de loop, modo "stutter".
- **Onde:** `plugins/AudioFileProcessor/` (`AudioFileProcessor.cpp`, `AudioFileProcessorWaveView.cpp`).
- **Parâmetros:** Amplify 0–500 %, 100, passo 1 · Início 0–1 (fração da amostra), 0 · Fim 0–1, 1 · Ponto de volta
  0–1, 0 · Inverter (liga/desliga), desligado · Modo de loop: desligado / liga / pingue-pongue, desligado · Stutter
  ("continuar a amostra entre notas"), desligado · Declick (fades curtos no início e no fim e cruzamento na volta do
  loop), ligado · Interpolação: nenhuma / linear / sinc, linear.
- **Regras dos pontos:** início > fim troca os dois; o ponto de volta fica dentro de [início, fim]; arrastar a volta
  além do fim empurra o fim. O botão do ponto de volta some com o loop desligado.
- **O usuário pode:** abrir uma amostra (um `.xpf` escolhido ali carrega como preset da faixa); arrastar uma amostra do
  navegador ou um clipe de amostra para o plugin; ver a forma de onda com os marcadores de início, fim e volta e
  arrastá-los (clique a até 4 px do marcador); rolar (arrasto horizontal) e dar zoom (arrasto vertical, roda) na onda;
  ver a duração da amostra e a posição tocando.
- **Comportamento:** polifônico, estéreo; release de 3 ms; sem loop, a nota dura o resto da amostra no editor de
  passos. Com stutter, cada nota continua de onde a anterior parou; a nota mais grave (< 20 Hz) volta ao início sem
  tocar. Aceita pitch bend. Carregar uma amostra renomeia a faixa, se o nome da faixa ainda era o da amostra anterior.
- **Formatos:** wav, ogg, ds (DrumSynth), spx, au, voc, aif/aiff, flac, raw, mp3 (se o libsndfile tiver).
  Amostra ausente vira aviso "Sample not found".
- **Na beta:** plugin `audiofileprocessor`, em `lm-plugins-instruments`, sobre o `sample_player` do `lm-dsp`: `amplify`,
  `startPoint`, `endPoint`, `loopPoint`, `reverse`, `stutter`, `declick`; campos `sample` (recurso de áudio), `loop`
  (`off`, `on`, `pingPong`) e `interpolation` (`none`, `linear`, `sinc`). Pontos ordenados como no `pointChanged`;
  stutter e a tecla mágica como no legado. Contrato e A/B por composição. Fica com outros: a duração da nota no editor
  de passos (o host), renomear a faixa e o empurrão entre os botões ao arrastar (a interface), a posição de tocar na
  forma de onda (dados ao vivo do painel).

#### F-INS-002 AvlDrums (baterias acústicas AVL, do fork) — core

- **O que faz:** os kits de bateria AVL (SF2) num FluidSynth interno, com o mapa de notas do avldrums.lv2 e saídas
  separadas por peça.
- **Onde:** `plugins/AvlDrums/`; kits em `data/avldrums/*.sf2`, fotos e mapas de clique em `data/avldrums/images/`.
- **Parâmetros:** Kit: Black Pearl (padrão), Red Zeppelin, Blonde Bop, Blonde Bop Hot Rod, Buskman's Holiday
  (percussão) · Saída por peça (liga/desliga), desligado; indisponível no Buskman's.
- **Mapa de notas:** 36 bumbo; 37/38/40 caixa; 42/44/46/48 chimbal; 45/47 tom; 41/43 surdo; 35/39/54/56/61 percussão;
  o resto pratos.
- **Saídas por peça:** 7 grupos, 9 canais: bumbo, caixa, chimbal, tom e surdo mono; pratos e percussão estéreo.
  Desligado, uma saída estéreo.
- **O usuário pode:** escolher o kit; ligar as saídas por peça; criar com um botão **um canal do mixer para cada saída
  e um canal de grupo que soma todos**, já roteados ("Mixer channel per piece"); ver a foto do kit e **clicar numa peça
  da foto para tocá-la** (a peça acende com a velocidade).
- **Comportamento:** fluxo único, recebe MIDI com precisão de amostra; sem pitch bend; velocidade 1–127. Repassa só
  CC (menos 10, 42, 64, 66, 68). Kit seco (reverb e chorus do FluidSynth desligados). Exportação determinística
  (o sintetizador é refeito e zerado no `reset`). Trocar a taxa recarrega o kit. Kit que não carrega vira aviso.

#### F-INS-003 BitInvader (onda desenhada) — 1.0

- **O que faz:** uma tabela de um ciclo desenhada pelo usuário (até 200 pontos), tocada em loop na altura da nota.
- **Onde:** `plugins/BitInvader/`, gráfico em `src/gui/widgets/Graph.cpp`.
- **Parâmetros:** Tamanho da amostra 4–200, 200, passo 1 · Interpolação, desligada · Normalizar, desligado ·
  Anti-aliasing, ligado · Forma da onda (tabela −1–1, 200 pontos, não automatizável), padrão senoide × 0,35.
- **O usuário pode:** desenhar a onda arrastando o mouse (Shift+clique traça reta desde o último ponto); aplicar
  formas prontas (senoide, triângulo, serra, quadrada, ruído branco); carregar um arquivo de áudio como onda; suavizar.
- **Comportamento:** polifônico, mono duplicado em L/R, release 1,5 ms. Com anti-aliasing, gera tabelas de banda
  limitada e segue o pitch bend. Quirk: soltar uma amostra no gráfico é aceito e não faz nada.
- **Na beta:** plugin `bitinvader`: `normalize` e os campos `sampleLength`, `wave`, `interpolation`, `antiAliasing` (os dois interruptores refaziam as tabelas: são campos, a mudança se constrói fora do áudio e as notas fazem crossfade), em `lm-plugins-instruments` (contrato e A/B por composição contra o `playNoteImpl` do legado).

#### F-INS-004 DrumMachines (baterias eletrônicas clássicas, do fork) — 1.0

- **O que faz:** baterias eletrônicas clássicas no mapa de bateria General MIDI: patches DrumSynth renderizados na
  taxa do motor ou gravações em FLAC.
- **Onde:** `plugins/DrumMachines/`; dados em `<factorySamplesDir>/drumsynth/<máquina>/*.ds` e
  `<factorySamplesDir>/drummachines/<máquina>/*.flac`; fotos `photo_*.jpg` com créditos.
- **Parâmetro:** Máquina: CR-78, CR-8000, TR-606, TR-77, TR-808 (padrão), TR-909, LinnDrum, Electro, R&B, SP-1200
  style, MPC60 style, DMX, TR-707. Até 24 peças por máquina; as variações sem nota GM vão para 82, 83…
- **O usuário pode:** escolher a máquina; ver a foto da máquina (ou um painel desenhado, no CR-78); tocar cada peça
  num pad clicável que acende ao tocar.
- **Comportamento:** fluxo único, 64 vozes com roubo da mais antiga; ganho = velocidade (linear); fade-in de 0,5 ms;
  chimbais se cortam (grupo de corte); **ignora note-off** (a peça toca até o fim); estéreo. Trocar a taxa
  re-sintetiza o kit.

#### F-INS-005 FreeBoy (APU do Game Boy) — 1.0

- **O que faz:** cada nota toca os 4 canais do Game Boy (2 quadradas, 1 de onda, 1 de ruído) num APU emulado próprio.
- **Onde:** `plugins/FreeBoy/` (Gb_Apu do game-music-emu).
- **Parâmetros (passo 1):** Sweep time 0–7, 4 · Sweep direction, desligado · Sweep rate shift 0–7, 4 · Duty canal 1
  0–3 (12,5/25/50/75 %), 2 · Volume canal 1 0–15, 15 · Direção do envelope canal 1, desligado · Passo do envelope
  canal 1 0–7, 0 · Duty canal 2 0–3, 2 · Volume canal 2 0–15, 15 · Direção do envelope canal 2 · Passo do envelope
  canal 2 0–7, 0 · Volume canal 3 (onda) 0–3, 3 · Volume canal 4 (ruído) 0–15, 15 · Direção do envelope canal 4 ·
  Passo do envelope canal 4 0–7, 0 · Largura do registrador de ruído (15/7 bits), desligado · Nível SO1 (direita)
  0–7, 7 · Nível SO2 (esquerda) 0–7, 7 · Canal 1–4 para SO1 e para SO2 (liga/desliga; o canal 4 nasce desligado nos
  dois) · Treble −100–200, −20 (dB) · Bass −1–600, 461 (Hz) · Onda do canal 3 (32 passos de 4 bits, desenhada, não
  automatizável), toda zero.
- **O usuário pode:** desenhar a onda do canal 3 num gráfico em degraus.
- **Comportamento:** polifônico (um APU por nota), estéreo; release 23 ms; os canais tonais só aceitam 65–4000 Hz;
  o ruído escolhe o divisor mais perto da nota. Quirk: as chaves salvas de SO1/SO2 são trocadas.

#### F-INS-006 GigPlayer (arquivos GigaSampler .gig) — 1.0

- **O que faz:** toca instrumentos `.gig` escolhidos por banco e programa, com envelope ADSR, amostras de release e
  dimensões simples.
- **Onde:** `plugins/GigPlayer/` (libgig), `PatchesDialog.cpp`.
- **Parâmetros:** Banco 0–999, 0 · Patch 0–127, 0 · Ganho 0–5, 1, passo 0,01 · arquivo.
- **O usuário pode:** abrir um `.gig` (começa na pasta de GIG das configurações; renomeia a faixa); escolher banco e
  programa numa lista de bancos e programas, ouvindo ao selecionar; ver o nome do patch.
- **Comportamento:** fluxo único, sem pitch bend; até 128 notas, 256 vozes, 16 amostras por nota (o excesso é
  descartado em silêncio); dimensões: velocidade, release trigger, key-switch, pedal de sustain, aleatório (round
  robin não implementado); só o primeiro loop (normal ou bidirecional); a amostra inteira vai para a RAM.

#### F-INS-007 Kicker (sintetizador de bumbo) — 1.0

- **O que faz:** senoide com varredura exponencial de frequência, envelope, distorção que varia do início ao fim,
  ruído e clique.
- **Onde:** `plugins/Kicker/`.
- **Parâmetros:** Frequência inicial 5–1000 Hz, 150 · Frequência final 5–1000 Hz, 40 · Duração 5–5000 ms, 440, sync ·
  Distorção inicial 0–100, 0,8 · Distorção final 0–100, 0,8 · Ganho 0,1–5, 1 · Inclinação do envelope 0,01–1, 0,163 ·
  Ruído 0–1, 0 · Clique 0–1, 0,4 · Inclinação da frequência 0,001–1, 0,06 · Ataque 0,1–20 ms, 1 · Começar na nota
  (frequência inicial = nota), ligado · Terminar na nota, desligado.
- **Comportamento:** polifônico, sem pitch bend; a nota termina sozinha depois da Duração, seja qual for o tamanho da
  nota, com release de 12 ms; mono → estéreo.
- **Na beta:** plugin `kicker`, em `lm-plugins-instruments`: os 13 botões do legado em camelCase e as funções de instrumento; a nota se solta sozinha depois do comprimento (na amostra exata, não no período seguinte); a fase da varredura volta a cada período (o legado acumulava em precisão simples e desafinava com a taxa); contrato e A/B por composição contra o `playNoteImpl` do legado.

#### F-INS-008 Lb302 (imitação monofônica da TB-303) — 1.0

- **Onde:** `plugins/Lb302/`.
- **Parâmetros:** Corte do VCF 0–1,5, 0,75 · Ressonância 0–1,25, 0,75 · Env Mod 0–1, 0,1 · Decay 0–1, 0,1 (tempo =
  2,3·v + 0,2 s) · Distorção 0–1, 0 · Forma de onda (12): serra, triângulo, quadrada, quadrada arredondada, Moog,
  senoide, exponencial, ruído branco, serra BL (padrão), quadrada BL, triângulo BL, Moog BL · Slide Decay 0–1, 0,6 ·
  Slide, desligado · Dead (nota não redispara o envelope se ainda soa), desligado · Filtro 24 dB/oitava (3 polos),
  desligado. "Accent" existe no código, sem efeito, sem tela, não salvo.
- **Comportamento:** **monofônico**; quando várias notas começam juntas, **a mais grave vence**; com Slide, a próxima
  nota desliza a partir desta. Segue pitch bend. Quirk: o botão "exponencial" diz "White noise wave".
- **Na beta:** plugin `lb302`, em `lm-plugins-instruments`: os 9 botões e o campo `waveform`; um fluxo só (a nota mais nova soa, com slide e dead), velocidade e expressões de pan e afinação da nota; sem as funções de instrumento (o legado não dava envelope nem filtro a um instrumento de fluxo único; acordes e arpejo de um fluxo monofônico soariam só a nota mais nova). Contrato e A/B por composição, bit a bit.

#### F-INS-009 Monstro (3 osciladores com matriz de modulação) — 1.0

- **Onde:** `plugins/Monstro/`.
- **Osciladores** (Volume 0–200 %, padrão **10**; Pan −100–100, 0; Grosso −24–24 semitons; Fino L e Fino R −100–100
  cents; Fase estéreo −180–180°):
  - Osc 1: pulso de banda limitada, Largura de pulso 0,25–99,75 %, 50; envia sync na subida e/ou na descida.
  - Osc 2: forma de onda (15: senoide, triângulo BL, serra BL, rampa BL, quadrada BL, Moog BL, quadrada suave, senoide
    absoluta, exponencial, ruído branco, triângulo, serra, rampa, quadrada e Moog digitais); sync duro, sync reverso.
  - Osc 3: sem fino; Mistura do sub-oscilador −100–100, 0 (entre forma 1 e forma 2); duas formas; sync duro/reverso.
  - Modulação Osc 2 → Osc 3: Mistura, AM, FM, PM.
- **LFO 1 e 2:** forma (senoide, triângulo, serra, rampa, quadrada, Moog, quadrada suave, senoide absoluta,
  exponencial, aleatório, aleatório suave); Ataque 0–2000 ms, sync; Taxa (período) 0,1–10000 ms, 1, sync; Fase
  −180–180°.
- **Envelopes 1 e 2:** Pré-atraso 0–2000 ms (sync), Ataque 0–2000, Hold 0–4000, Decay 0–4000, Release 0–4000 (ms,
  padrão 0); Sustain 0–1, 1; Inclinação −1–1, 0.
- **Matriz:** 44 quantidades −1–1, padrão 0: fontes Env1, Env2, LFO1, LFO2 × destinos volume, fase e altura de cada
  oscilador, largura de pulso do Osc 1 e mistura do sub do Osc 3.
- **O usuário pode:** alternar entre a vista dos operadores e a vista da matriz.
- **Comportamento:** polifônico, estéreo; release = o maior release dos dois envelopes.
- **Na beta:** plugin `monstro`, em `lm-plugins-instruments`: os 44 botões e as 44 quantidades da matriz em camelCase (`osc1Volume`…, `osc1VolEnv1`…), as formas de onda e a modulação Osc 2→3 como campos, um campo `<botão>Sync` por botão de tempo; a voz se solta pelos envelopes. Contrato e A/B por composição (com os botões sincronizados). A escolha da vista é da interface.

#### F-INS-010 Nes (sintetizador estilo NES) — 1.0

- **Onde:** `plugins/Nes/`.
- **Canais 1 e 2 (pulso):** liga (ligado) · Grosso −24–24 · Volume 0–15, 15 · Envelope liga, loop, duração 0–15 ·
  Duty 12,5/25/50/75 % (canal 1: 12,5; canal 2: 50) · Sweep liga, quantidade −7–7, taxa 0–7.
- **Canal 3 (triângulo):** liga, grosso, volume 0–15.
- **Canal 4 (ruído):** liga (**desligado**), volume, envelope liga/loop/duração, modo de ruído (curto/metálico),
  usar a frequência da nota, frequência do ruído 0–15, varredura da frequência −7–7, quantizar a frequência (ligado).
- **Master:** Volume 0–2, 1 · Vibrato 0–15, 0.
- **Comportamento:** polifônico, cada nota é um NES inteiro; mistura mono com as proporções do NES, duplicada em L/R;
  release praticamente nulo (corta no note-off).
- **Na beta:** plugin `nes`, em `lm-plugins-instruments`: os 33 botões do legado em camelCase (`ch1Enabled`…`ch4NoiseQuantize`, `masterVolume`, `vibrato`) e os duty cycles como campos (`ch1DutyCycle`, `ch2DutyCycle`); os botões agem nas notas que soam. Contrato e A/B por composição.

#### F-INS-011 OpulenZ (FM de 2 operadores, OPL2/AdLib) — 1.0

- **Onde:** `plugins/OpulenZ/` (emulador do adplug).
- **Operadores 1 (modulador) e 2 (portadora):** Ataque 0–15 (14 / 1) · Decay 0–15 (14 / 3) · Sustain 0–15 (3 / 14) ·
  Release 0–15 (10 / 12) · Nível 0–63 (62 / 63) · Escala de nível 0–3 · Multiplicador 0–15 (0 / 1) · Key scaling ·
  Envelope percussivo · Tremolo (ligado no op 1) · Vibrato (ligado no op 2) · Forma: senoide, meia senoide, senoide
  absoluta, quarto de senoide. As dicas mostram os tempos reais (ataque 0–3800 ms; decay/release 0–20000 ms).
- **Global:** Realimentação do op 1 0–7, 0 · FM (desligado = aditivo), ligado · Vibrato fundo, Tremolo fundo ·
  Volume −24–24 dB, **+16**.
- **O usuário pode:** carregar patches `.sbi` (SoundBlaster), que renomeiam a faixa.
- **Comportamento:** **9 vozes** (a décima nota é descartada); recebe note-on/off, pressão de tecla (muda o nível),
  pitch bend (faixa por RPN 0, padrão 100 cents). A velocidade escala o nível da portadora (em FM) ou dos dois.
  Mono duplicado. Quirk: um parâmetro "Patch" GM existe sem tela e sem salvar.

#### F-INS-012 Organic (aditivo, timbre de órgão) — 1.0

- **Onde:** `plugins/Organic/`.
- **Global:** Distorção 0–0,99, 0 · Volume 0–200 %, 100.
- **8 osciladores**, cada um: Forma (senoide, serra, quadrada, triângulo, Moog, exponencial) · Harmônico 0–17 (oitava
  abaixo, quinta abaixo, fundamental, 2º…16º; padrão: osc N = N−1) · Volume 0–100 %, 100 · Pan −100–100, 0 ·
  Desafinação estéreo −1200–1200 cents, 0.
- **O usuário pode:** sortear volumes, desafinações e formas com um botão "Randomise".
- **Comportamento:** polifônico, estéreo, fase inicial aleatória por nota; fade-in curto; sem release.
- **Na beta:** plugin `organic`: por oscilador `harmonic`, `volume`, `panning`, `detune` e o campo `shape` (a escolha de seis formas), mais `distortion` e `volume`, em `lm-plugins-instruments` (contrato e A/B por composição contra o `playNoteImpl` do legado).

#### F-INS-013 PatMan (patches GUS .pat) — 1.0

- **O que faz:** toca patches `.pat` do Gravis Ultrasound (multiamostra, um instrumento, uma camada).
- **Onde:** `plugins/Patman/`.
- **Parâmetros:** Loop (usa os pontos de loop do patch), ligado · Tune (afina a amostra pela nota; desligado toca na
  altura original), ligado · arquivo.
- **O usuário pode:** abrir um `.pat` (começa em `/usr/share/midi/freepats` se existir); soltar um `.pat` do
  navegador; abrir um `.pat` em qualquer lugar escolhe o PatMan.
- **Comportamento:** escolhe a amostra cuja nota raiz está mais perto da nota (ignora faixas de tecla, envelopes,
  pan, tremolo do patch); mono duplicado; release da faixa. Erros claros (não é GUS, mais de uma camada, truncado).
- **Na beta:** plugin `patman`, em `lm-plugins-instruments`: `looped` e `tuned` (ligados) e o campo `patch` (recurso de
  tipo `pat`), lido no build com os erros do legado (`patman.not_gus`, `patman.layers`, `patman.truncated`); a
  amostra mais perto da nota pelo conversor linear do libsamplerate (o porte). Contrato e A/B por composição. Abrir e
  soltar um `.pat` são da interface.

#### F-INS-014 Sf2 Player (SoundFont) — 1.0

- **O que faz:** toca SoundFonts `.sf2`/`.sf3` num FluidSynth por instrumento.
- **Onde:** `plugins/Sf2Player/` (`PatchesDialog`).
- **Parâmetros:** Banco 0–999, 0 · Patch 0–127, 0 · Ganho 0–5, 1 · Reverb liga (desligado), Tamanho da sala 0–1,
  Amortecimento 0–1, Largura 0–1, Nível 0–1 · Chorus liga (desligado), Vozes 0–10, Nível 0–10, Velocidade 0,29–5 Hz,
  Profundidade 0–46 ms. Os padrões do reverb e do chorus são os do FluidSynth.
- **O usuário pode:** abrir um SoundFont (começa na pasta de SF2 das configurações; renomeia a faixa); escolher o
  patch numa lista de bancos e patches **com busca**, ouvindo ao navegar, Cancelar volta ao anterior; soltar um
  SoundFont do navegador ou do gerenciador de arquivos; um instrumento novo já abre o **SoundFont padrão das
  configurações** no primeiro preset.
- **Comportamento:** fluxo único; uma nota fora de 0–127 é descartada; detune, glide e pan por nota aplicados por
  voz; **com o envelope de volume da faixa ligado (aba ENV), ele substitui o envelope do SoundFont**; respeita o pedal
  de sustain; pitch bend e faixa de bend; **segue o microafinador** (tabela de 128 teclas); acima de 96 kHz renderiza
  a 96 e converte. Exportação determinística.

#### F-INS-015 sfxr (efeitos sonoros retrô) — 1.0

- **O que faz:** gerador de efeitos de jogo do sfxr; cada nota toca um efeito inteiro (a nota termina sozinha).
- **Onde:** `plugins/Sfxr/`.
- **Parâmetros (0–1 ou −1–1):** Attack 0 · Sustain Time 0,3 · Sustain Punch 0 · Decay 0,4 · Start Frequency 0,3 · Min
  Frequency 0 · Slide 0 · Delta Slide 0 · Vibrato Depth 0 · Vibrato Speed 0 · Change Amount 0 · Change Speed 0 · Square
  Duty 0 · Duty Sweep 0 · Repeat Speed 0 · Phaser Offset 0 · Phaser Sweep 0 · LP Cutoff 1 · LP Cutoff Sweep 0 · LP
  Resonance 0 · HP Cutoff 0 · HP Cutoff Sweep 0 · Onda: quadrada (padrão), serra, senoide, ruído.
- **O usuário pode:** gerar um efeito pronto por categoria (moeda, laser, explosão, power-up, dano, pulo, blip), um
  aleatório, ou "mutar" o atual (±0,05 em metade dos parâmetros); tudo isso desfazível.
- **Comportamento:** roda a 44,1 kHz por dentro; a nota base toca o efeito na altura desenhada, outras teclas
  transpõem; mono duplicado.
- **Na beta:** plugin `sfxr`, em `lm-plugins-instruments`: os 22 botões do legado em camelCase e o campo `wave`; o som a 44,1 kHz esticado à altura da nota sobre o A4, a nota acaba com o som (`Synth::finished`). Contrato e A/B por composição.

#### F-INS-016 SID (Commodore 64) — 1.0

- **Onde:** `plugins/Sid/` (reSID).
- **Por voz (3):** Largura de pulso 0–4095, 2048 · Ataque 0–15, 8 · Decay 0–15, 8 · Sustain 0–15, 15 · Release 0–15,
  8 (as dicas mostram os tempos reais) · Grosso −24–24 · Forma: pulso, triângulo (padrão), serra, ruído · Sync · Ring
  mod · Filtrada · Test.
- **Global:** Corte 0–2047, 1024 (mostra Hz) · Ressonância 0–15, 8 · Filtro passa-alta / passa-banda / passa-baixa
  (padrão) · Desligar voz 3 · Volume 0–15, 15 · Chip MOS6581 ou MOS8580 (padrão).
- **Comportamento:** um chip por nota (polifônico), as 3 vozes tocam a nota; release pelo envelope do SID; mono
  duplicado. Quirk: um modo de filtro por vez (o chip real combina).

#### F-INS-017 SlicerT (fatiador de loops) — 1.0

- **O que faz:** carrega uma amostra, acha as fatias pelos ataques e põe cada fatia numa tecla: a nota base toca a
  amostra inteira, as seguintes as fatias 1, 2…
- **Onde:** `plugins/SlicerT/`.
- **Parâmetros:** Limiar 0–2, 0,6 · Fade out 0–100 ms, 10 · BPM original 1–999 (detectado ao carregar) · Grade das
  fatias: desligada, 1/1…1/32 · Sincronizar com o andamento, desligado · Loop: desligado / loop / pingue-pongue ·
  lista de fatias.
- **O usuário pode:** carregar a amostra (clique, botão, ou arrastar uma amostra ou clipe de amostra); ver o andamento
  detectado (60–200 BPM); refazer a detecção; arrastar os marcadores, criar com duplo clique, apagar com o botão
  direito; navegar (barra de visão geral com alças, zoom vertical na roda, clique do meio volta); apagar todas as
  fatias; **copiar um padrão MIDI com uma nota por fatia para colar no editor de notas**.
- **Comportamento:** com sync, a velocidade é BPM original / andamento (muda a altura); sem loop, a fatia toca uma vez
  com fade; teclas além das fatias ficam mudas. Precisa de FFTW.

#### F-INS-018 Mallets (STK: marimba, vibrafone, sinos…) — 1.0

- **Onde:** `plugins/Stk/Mallets/`; precisa das rawwaves do STK (pasta STK das configurações), senão avisa e não toca.
- **Parâmetros:** Instrumento: Marimba (padrão), Vibraphone, Agogo, Wood 1, Reso, Wood 2, Beats, Two fixed, Clump,
  Tubular bells, Uniform bar, Tuned bar, Glass, Tibetan bowl · ModalBar: Hardness 0–128, 64; Position 0–64, 64;
  Vibrato gain 0–128, 0; Vibrato freq 0–128, 0; Stick mix 0–128, 0 · TubeBell: Modulator, Crossfade, LFO speed, LFO
  depth, ADSR (0–128, 64) · BandedWG: Pressure 0,1–128, 64; Speed 0,1–128, 64; Bowed (salvo, sem tela) · Spread
  (atraso estéreo) 0–255, 0 · Aleatoriedade 0–1, 0.
- **O usuário pode:** a escolha do instrumento troca a página de botões (três modelos).
- **Comportamento:** a aleatoriedade varia alguns parâmetros por nota; a altura segue a nota ao vivo; volume nivelado
  por preset.

#### F-INS-019 TripleOscillator — core (primeira fatia)

- **O que faz:** três osciladores estéreo, com modulação do 1 pelo 2 e do 2 pelo 3.
- **Onde:** `plugins/TripleOscillator/`, oscilador em `src/core/Oscillator.cpp`. O oscilador já foi portado para o
  `lm-dsp` com A/B (`implementacao.md`).
- **Por oscilador:** Volume 0–200 %, 20 · Pan −100–100, 0 · Grosso −24–24 semitons (osc 1: 0, osc 2: −12, osc 3:
  −24) · Fino esquerdo e direito −100–100 cents · Fase inicial 0–360° · Desafinação de fase estéreo 0–360° · Forma:
  senoide, triângulo, serra, quadrada, serra Moog, exponencial, ruído branco, **onda do usuário** · Tabela sem
  aliasing (ligado).
- **Modulação** (1←2 e 2←3): PM, AM, Mistura (padrão), Sync, FM.
- **O usuário pode:** escolher a forma de cada oscilador; carregar um arquivo como onda do usuário (gera a tabela sem
  aliasing; arquivo ausente só avisa se for usado).
- **Comportamento:** polifônico; fade-in e release da faixa.

#### F-INS-020 Vibed (cordas vibrantes) — 1.0

- **O que faz:** modelo de corda tipo Karplus-Strong, 9 cordas independentes por nota, cada uma com o próprio impulso
  desenhado.
- **Onde:** `plugins/Vibed/`.
- **Por corda (9):** Liga (só a 1 nasce ligada) · Volume 0–200 %, 100 · Rigidez 0–0,05 · Posição da palheta 0–0,05,
  0 · Posição do captador 0–0,05, 0,05 · Pan −1–1 · Desafinação −0,1–0,1 · Fuzz 0–0,75 · Comprimento 1–16, 1 ·
  Impulso (usa a forma como velocidade inicial), desligado · Oitava: −2, −1, F (padrão), 2…7 · Forma (128 pontos,
  desenhada).
- **O usuário pode:** escolher qual corda editar; desenhar a forma; aplicar senoide, triângulo, serra, quadrada,
  ruído, arquivo; suavizar; normalizar.
- **Comportamento:** **sem pitch bend**; sem release (as cordas decaem sozinhas).
- **Na beta:** plugin `vibed`, em `lm-plugins-instruments`: por corda `string`, `volume`, `stiffness`, `pick`, `pickup`, `panning`, `detune`, `fuzziness`, `impulse` (parâmetros) e `harmonic`, `length`, `shape` (campos: decidem a memória da corda); as cordas correm no próprio relógio (2 × comprimento × 44,1 kHz) em qualquer taxa do projeto; nenhuma nota aloca. Contrato e A/B por composição.

#### F-INS-021 Watsyn (4 osciladores de onda desenhada) — 1.0

- **O que faz:** dois pares A (A1←A2) e B (B1←B2) misturados por um crossfader A-B com envelope próprio.
- **Onde:** `plugins/Watsyn/`.
- **Por oscilador (A1, A2, B1, B2):** Volume 0–200 %, 32 · Pan −100–100 · Multiplicador de frequência 1–24 (/8), 8 ·
  Desafinação esquerda e direita −600–600 cents · Onda desenhada (220 pontos).
- **Global:** Mistura A-B −100–100, 0 · Quantidade do envelope −200–200 · Ataque, Hold, Decay do envelope 0–2000 ms,
  sync · Crosstalk A1→B2 0–100 · Modulação A2→A1 e B2→B1: Mistura, AM, Ring, PM · Anti-aliasing (ligado).
- **O usuário pode:** escolher qual onda editar e desenhá-la; carregar de arquivo; deslocar a fase ±15°; normalizar;
  inverter; suavizar; formas prontas.
- **Na beta:** plugin `watsyn`, em `lm-plugins-instruments`: por oscilador `a1Volume`…`b2RightDetune`; `abMix`,
  `envAmount`, `envAttack`, `envHold`, `envDecay`, `crosstalk`; campos `a1Graph`…`b2Graph` (220 pontos), `aMod`,
  `bMod`, `antiAliasing` e o sync de cada tempo do envelope. As ondas são sobreamostradas no build (fora do áudio),
  como o `srccpy` do legado. Contrato e A/B por composição. As ferramentas de desenho (carregar, fase, normalizar,
  inverter, suavizar, formas prontas) são do painel: editam o campo.

#### F-INS-022 Xpressive (síntese por expressões) — 1.0

- **O que faz:** o usuário escreve expressões matemáticas para 2 saídas (O1, O2) e 3 tabelas de onda (W1–W3).
- **Onde:** `plugins/Xpressive/` (exprtk).
- **Parâmetros:** A1, A2, A3 −1–1, 1 (uso livre nas expressões) · Suavização de W1–W3 0–70 · Interpolar W1–W3 · Pan
  O1 −1–1, 1 · Pan O2 −1–1, −1 · Transição de release 0–500 ms, 50 · textos das expressões e as ondas desenhadas.
- **O usuário pode:** editar cada expressão com reavaliação ao vivo e o gráfico do resultado (LED vermelho se inválida,
  saída muda); desenhar W1–W3 quando a expressão está vazia; formas prontas; abrir uma ajuda completa com as
  variáveis (t, f, key, bnote, srate, tempo, v, rel, trel, seed, A1–A3) e funções (W1–W3, cent, semitone, last,
  integrate, rand, randv, formas de onda, e a matemática do exprtk).
- **Comportamento:** sem release próprio; a cauda é o que a expressão faz com `rel`/`trel`.

#### F-INS-023 ZynAddSubFX — 1.0

- **O que faz:** o ZynAddSubFX 2.x embutido.
- **Onde:** `plugins/ZynAddSubFx/` (`LocalZynAddSubFx`, `RemoteZynAddSubFx` com a interface FLTK).
- **Parâmetros do host (0–127, mandados como CC):** Portamento 0 (0 = desligado) · Corte do filtro 64 · Ressonância
  64 · Bandwidth 64 · Ganho de FM 127 · Centro da ressonância 64 · Largura da ressonância 64 · Repassar CCs da faixa
  (ligado).
- **O usuário pode:** abrir e soltar presets `.xiz` (banco de fábrica em `<factoryPresetsDir>/ZynAddSubFX`; renomeia a
  faixa); abrir a interface completa do Zyn.
- **Comportamento:** fluxo único; MIDI aplicado no início do período (não preciso na amostra); o estado inteiro do Zyn
  vai no projeto. Quirk: mostrar ou esconder a interface recria o sintetizador (com a interface ele roda num processo
  separado; sem ela, dentro). **Aberto:** no programa novo é plugin interno (dentro do processo) ou de terceiros (fora)?
  E a interface FLTK? (pergunta P-05.)

#### F-INS-024 Janela de "plugin não encontrado" e instrumento fantasma — 1.0

- **O que faz:** um plugin que falta vira um instrumento/efeito mudo que guarda o estado e o salva de volta.
- **Onde:** `src/core/Plugin.cpp` (DummyInstrument/DummyEffect).
- **Status:** 1.0, já previsto (`arquitetura-nova.md`, peça 7: "plugin não disponível", a árvore é guardada).

### 7.1b Hospedeiros de instrumentos de terceiros

#### F-HOST-001 VeSTige (instrumentos VST2) — aberto

- **O que faz:** hospeda instrumentos VST2 sempre em processo separado: `.dll` do Windows pelo wine (32 ou 64 bits),
  `.so` nativos no Linux.
- **Onde:** `plugins/Vestige/`, `plugins/VstBase/` (`RemoteVstPlugin`).
- **O usuário pode:** abrir o plugin (começa na pasta de VST das configurações; aviso de carregamento); mostrar ou
  esconder a interface do plugin (embutida ou em janela própria, conforme a configuração); trocar de programa (anterior,
  próximo, menu com todos); abrir e salvar presets `.fxp`/`.fxb`; abrir uma janela genérica com um botão por parâmetro
  (filtro "automatizados/todos", "VST Sync" puxa os valores do plugin); desligar todas as notas; soltar um arquivo VST
  na faixa.
- **Parâmetros:** um botão 0–1 por parâmetro do plugin, com o texto que o plugin mostra; salvos só se automatizados ou
  ligados a controlador; o resto vai no "chunk" do plugin.
- **Comportamento:** MIDI com posição no bloco; canais conforme o plugin; **latência informada**; andamento e posição
  passados ao plugin.
- **Status:** **aberto** — o desenho novo fala de LV2, CLAP e depois VST3; VST2 não é citado (pergunta P-01).

#### F-HOST-002 Carla Rack e Carla Patchbay — aberto

- **O que faz:** embute o Carla (rack em série ou patchbay com roteamento livre) como instrumento; o Carla hospeda
  LV2, LADSPA, DSSI, VST2, VST3, SF2, SFZ.
- **Onde:** `plugins/CarlaBase/`, `plugins/CarlaRack/`, `plugins/CarlaPatchbay/`.
- **O usuário pode:** abrir a interface do Carla; ver os parâmetros que o Carla expõe numa janela com busca, filtro
  por grupo e "só os ligados"; automatizar os de entrada; ver os de saída.
- **Comportamento:** estado inteiro do Carla no projeto; MIDI preciso; andamento e compasso passados; latência
  informada (soma no rack, máximo no patchbay).
- **Status:** **aberto** (pergunta P-02): com LV2, CLAP e VST3 próprios, o Carla ainda é preciso?

#### F-HOST-003 Instrumentos LV2 — 1.0

- **O que faz:** hospeda qualquer plugin LV2 classificado como instrumento (sem entrada de áudio principal).
- **Onde:** `plugins/Lv2Instrument/`, `src/core/lv2/`, `src/gui/Lv2ViewBase.cpp`.
- **O usuário pode:** ver uma grade genérica de controles (liga/desliga para `toggled`, lista para `enumeration`, número
  para inteiros, botão para o resto, escala log quando o plugin pede); ler a descrição do plugin; recarregar o plugin.
  Plugins mono rodam em duas cópias (L e R) com os controles ligados, e cada controle pode ser desligado da ligação.
- **Recusados com motivo:** recurso obrigatório não suportado, sem canais de áudio, mais de uma porta MIDI, faixas
  inválidas, e uma lista de bloqueio de plugins instáveis (desligável nas configurações).
- **Faltas do legado (não são funcionalidade a manter):** a interface própria do plugin está desligada; o estado do
  plugin (extensão state) não é salvo, só os controles; presets não carregam; roda dentro do processo.
- **Status:** 1.0, fase 2, fora do processo (`decisoes-autonomas.md`), com o "chunk" de estado salvo.

### 7.2 Efeitos

#### F-FX-001 Controles comuns de todo efeito — core

- **O que faz:** cada efeito numa cadeia tem: **liga/desliga** (fica vermelho, com aviso, quando o efeito solta
  inf/NaN; o canal é silenciado) · **Wet/Dry** −1–1, 1 (molhado = v, seco = 1 − v; o seco é atrasado pela latência
  do efeito para alinhar) · **Decay** 1–8000 ms, sync: depois de tanto tempo de silêncio na saída o efeito dorme até
  chegar sinal (só quando a configuração "efeitos continuam rodando sem entrada" está desligada). Não há Gate neste
  fork.
- **Onde:** `src/core/Effect.cpp`, `src/gui/EffectView.cpp`.
- **Status:** core. O "Decay"/auto-quit é otimização do motor velho: **aberto** se volta (pergunta P-07).

#### F-FX-002 Cadeia de efeitos — core

- **O que faz:** uma lista ordenada de efeitos num canal do mixer (e, no legado, também na faixa de instrumento e de
  amostra). O usuário pode: ligar/desligar a cadeia inteira (nasce **desligada**); acrescentar arrastando do navegador
  de efeitos para a posição desejada, ou com duplo clique (vai para o canal selecionado); reordenar (arrastar, ou
  Mover acima/abaixo); trocar um efeito por outro no mesmo lugar ("Replace with…"); remover; abrir a janela de
  controles do efeito; carregar e salvar o preset de um efeito (`.xfx` em `<presets do usuário>/effects/`); escolher
  a fonte de sidechain (outro canal do mixer, sem criar laço) quando o efeito tem mais de 2 entradas; abrir o
  conector de pinos (efeitos multicanal); rodar num processo separado (só terceiros).
- **Latência:** a soma das latências dos efeitos entra na compensação do canal.
- **Efeito que não carrega** fica como "efeito fantasma" e o estado dele é preservado ao salvar.
- **Onde:** `src/core/EffectChain.cpp`, `src/gui/EffectRackView.cpp`, `src/gui/EffectBrowser.cpp`,
  `src/gui/modals/EffectSelectDialog.cpp`.
- **Destino no programa novo:** os efeitos moram **só** nos canais da mesa, como inserts (`arquitetura-nova.md`,
  especificação item 2); a cadeia na faixa de instrumento/amostra fica **obsoleta — substituída pelos inserts do
  canal para onde a faixa manda**. "Rodar em processo separado" fica **obsoleto — substituído por: terceiros sempre
  fora do processo, internos sempre dentro** (I-9.6). O resto é core.

#### F-FX-003 Navegador de efeitos — 1.0

- **O que faz:** lista de efeitos com busca, agrupada por categoria (EQ e filtros, Dinâmica, Atraso e reverb,
  Modulação, Distorção, Altura e frequência, Estéreo e utilidade, Análise, Controle, Guitarix, Outros) e por
  hospedeiro (LADSPA, LV2…); plugins com subplugins (Guitarix) ganham um nível por grupo; ícones coloridos por
  categoria. Efeitos aposentados (Amplifier, StereoMatrix) não aparecem.
- **Onde:** `src/gui/EffectBrowser.cpp`, `src/core/PluginCategories.cpp`.

#### F-FX-004 AmpAndIr (modelo de amplificador NAM + resposta ao impulso, do fork) — 1.0

- **O que faz:** ganho de entrada → modelo de amplificador do Neural Amp Modeler (`.nam`, opcional) → convolução com
  resposta ao impulso (gabinetes, corpos de violão, salas, opcional) → ganho de saída.
- **Onde:** `plugins/AmpAndIr/`; IRs de fábrica em `<factorySamplesDir>/impulse-responses/`.
- **Parâmetros:** IN −24–24 dB, 0 · OUT −24–24 dB, 0 · modelo (arquivo) · IR (arquivo).
- **O usuário pode:** carregar um modelo `.nam`; abrir o site TONE3000 para baixar modelos; escolher uma IR da
  biblioteca de fábrica (Violão: Classical, Dreadnought; Gabinetes: 1x10 small, 1x12 combo, 2x12 open back, 4x12
  closed back; Salas: Hall, Plate, Small room, Studio live room) ou de um arquivo (wav, flac, aif/aiff, ogg); tirar
  o modelo ou a IR.
- **Comportamento:** o modelo NAM é mono: com modelo carregado o sinal vira mono. Volume normalizado a −18 dB como no
  plugin NAM; o sinal é reamostrado para a taxa do modelo. A IR é reamostrada e normalizada em energia (trocar de IR
  mantém o volume); IR estéreo é por lado. **Latência informada** (bloco de 256 da convolução + reamostragem).
  Carrega fora do áudio. Arquivo ausente vira aviso e o resto do projeto abre.

#### F-FX-005 Amplifier — obsoleto

- **O que fazia:** volume 0–200 %, pan −100–100, ganho esquerdo e direito 0–200 %.
- **Onde:** `plugins/Amplifier/`. Já aposentado no fork (some dos navegadores, só abre em projeto antigo).
- **Status:** obsoleto — substituído pelo trim, fader e pan do canal da mesa (F-MIX).
- **Na beta:** plugin `amplifier` (para abrir projetos antigos): `volume`, `panning`, `leftGain`, `rightGain`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-006 AmplifikationLite (clone do amplificador Kuassa, do fork) — 1.0

- **O que faz:** amplificador de guitarra de 3 canais (Clean, Rhythm, Lead) medido como caixa-preta do VST da Kuassa:
  tone stack TMB exato, presença, gabinete interno ou IR do usuário.
- **Onde:** `plugins/AmplifikationLite/` (README com a cadeia de DSP e a tabela de fidelidade). Outra sessão está
  trabalhando nele; a decisão do usuário é que ele nasça direto no contrato novo (`arquitetura-nova.md`).
- **Parâmetros (0–1, padrão 0,5 salvo onde dito):** Input (0,2) · Canal Clean/Rhythm/Lead (Clean) · Gain · Bass ·
  Middle · Treble · Presence · Volume · Gabinete INT / OFF / EXT (INT).
- **O usuário pode:** usar a pele original (botões em filmstrip, chaves de 3 posições: clique na metade de cima sobe,
  na de baixo desce; a roda também muda); carregar uma IR (só com EXT; usa o primeiro canal); abrir o "Sobre".
- **Comportamento:** mono (soma L+R, mesma saída nos dois lados); processa a 48 kHz por dentro; **latência ~450
  amostras, informada**.

#### F-FX-007 BassBooster — 1.0

- **Onde:** `plugins/BassBooster/`. **Parâmetros:** Freq 50–200 Hz, 100 · Gain 0,1–5, 1 · Ratio 0,1–10, 2.
- **Na beta:** plugin `bassbooster`: `frequency`, `gain`, `ratio`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-008 Bitcrush — 1.0

- **Onde:** `plugins/Bitcrush/`.
- **Parâmetros:** Ganho de entrada −20–20 dBFS, 0 · Ruído de entrada 0–100 %, 0 · Ganho de saída −20–20, 0 · Clip de
  saída −20–20, 0 · Taxa 20–44100 Hz, 44100 · Diferença estéreo −50–50 %, 0 · Níveis 1–256, 256 · Taxa ligada
  (ligado) · Profundidade ligada (ligado).
- **Comportamento:** sobreamostragem ×5 com filtro anti-alias.
- **Na beta:** plugin `bitcrush`: `inputGain`, `inputNoise`, `outputGain`, `outputClip`, `sampleRate`, `stereoDifference`, `levels`, `rateEnabled`, `depthEnabled`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-009 Compressor — core (plugin de validação)

- **Onde:** `plugins/Compressor/`.
- **Parâmetros:** Threshold −60–0 dBFS, −8, log · Ratio 1–20, 1,8, log · Attack 0,005–250 ms, 10, log · Release
  1–2500 ms, 100, log · Knee 0–96 dB, 12 · Hold 0–500 ms, 0 · Range −240–0 dB, −240 · RMS Size 0–250, 1 · Modo
  Esquerda/Direita ou Mid/Side · Detector RMS ou Pico · Lookahead (liga, introduz 20 ms) e duração 0–20 ms · Balanço
  de entrada −1–1 · Balanço de saída −1–1 · Compressor/Limitador · Ganho de saída −60–30 dB, 0 · Ganho de entrada
  −60–30 dB, 0 · Stereo Link: desligado / máximo (padrão) / média / mínimo / mistura, e a mistura 0–3, 1 · Balanço
  estéreo do detector −1–1 · Ganho de compensação automático · Audition (ouvir só o que foi tirado) · Feedback
  (detector na saída) · Ataque automático 0–100 % · Release automático 0–100 % · Tilt do detector −6–6 dB (−6 =
  passa-baixa, +6 = passa-alta) e frequência do tilt 20–20000 Hz, 150 · Mix 0–100 %, 100.
- **O usuário pode:** ver um gráfico rolando com nível de entrada, de saída e redução de ganho; ver a curva de
  transferência; dar zoom na faixa de dB com a roda (3–96 dB); esconder o painel com duplo clique; os botões sem
  efeito no modo atual aparecem marcados.
- **Quirk:** o lookahead atrasa 20 ms e **não é informado** à compensação de latência. O porte deve informar.
- **Na beta:** plugin `compressor`: os 27 botões do legado em camelCase e `stereoLink`; com o lookahead ligado, latência informada (e pedida de novo quando o botão muda); medidores de entrada, saída e redução na janela, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-010 CrossoverEQ — 1.0

- **Onde:** `plugins/CrossoverEQ/`.
- **Parâmetros:** Cruzamento 1/2 50–10000 Hz, 125 · 2/3 50–20000, 1250 · 3/4 50–20000, 5000 (os cruzamentos se
  mantêm em ordem) · Ganho das bandas 1–4 −60–30 dB, 0 · Banda 1–4 ativa (ligado). Filtros Linkwitz-Riley.
- **Na beta:** plugin `crossovereq`: `crossover12`, `crossover23`, `crossover34`, `gain1`–`gain4`, `band1Active`–`band4Active`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-011 Delay — core (plugin de validação, ou o ReverbSC)

- **Onde:** `plugins/Delay/`.
- **Parâmetros:** Tempo 0,01–5 s, 0,5, sync · Realimentação 0–1, 0 · Taxa do LFO (período) 0,01–5 s, 2, sync ·
  Quantidade do LFO 0–0,5 s, 0, sync · Ganho de saída −60–20 dB, 0 (com medidor L/R).
- **O usuário pode:** ajustar tempo e realimentação juntos num pad XY.
- **Na beta:** plugin `delay`: `delayTime`, `feedback`, `lfoTime`, `lfoAmount`, `outGain` (medidores de saída na janela), em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-012 Dispersion (all-pass de ordem alta) — 1.0

- **Onde:** `plugins/Dispersion/`. **Parâmetros:** Quantidade de all-pass 0–999, 0 · Freq 20–20000 Hz, 200, log ·
  Reso 0,01–8 oitavas, 0,707 · Realimentação −1–1, 0 · Remover DC, desligado.
- **Na beta:** plugin `dispersion`: `amount`, `frequency`, `resonance`, `feedback`, `dcOffsetRemoval`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-013 DualFilter — 1.0

- **Onde:** `plugins/DualFilter/`.
- **Parâmetros:** para cada um dos 2 filtros: liga (ligado) · Tipo (22: passa-baixa, passa-alta, passa-banda csg,
  passa-banda czpg, notch, all-pass, Moog, 2× passa-baixa, RC passa-baixa/banda/alta 12 dB, RC passa-baixa/banda/alta
  24 dB, formante vocal, 2× Moog, SV passa-baixa/banda/alta/notch, formante rápido, Tripole) · Corte 1–20000 Hz,
  7000 · Ressonância mín–10, 0,5 · Ganho 0–200 %, 100. Mix −1 (só o 1) a +1 (só o 2), 0.
- Os mesmos 22 tipos são os do filtro da janela do instrumento (F-IJN-011).
- **Na beta:** plugin `dualfilter`: `filter1Enabled`, `filter1Type`, `cutoff1`, `resonance1`, `gain1`, `mix` e os mesmos do filtro 2, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-014 DynamicsProcessor (curva de dinâmica desenhada) — 1.0

- **Onde:** `plugins/DynamicsProcessor/`.
- **Parâmetros:** Entrada 0–5, 1 · Saída 0–5, 1 · Ataque 1–500 ms, 10 · Release 1–500 ms, 100 · Estéreo: máximo
  (padrão) / média / desligado · Curva de transferência (200 pontos, 0–1), padrão linear.
- **O usuário pode:** desenhar a curva entrada → saída com o mouse; voltar à linear; suavizar; subir ou descer a curva
  inteira 1 dB.
- **Na beta:** plugin `dynamicsprocessor`: `inputGain`, `outputGain`, `attackTime`, `releaseTime`, `stereoMode`, campo `curve` (o ponto do nível na curva na janela), em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-015 Eq (equalizador de 8 bandas) — core (plugin de validação)

- **Onde:** `plugins/Eq/`.
- **Bandas:** passa-alta, low-shelf, 4 picos, high-shelf, passa-baixa; todas nascem **desligadas**.
- **Parâmetros:** Ganho de entrada e de saída −60–20 dB, 0 · Ganho das shelves e picos −18–18 dB, 0 · Res do HP/LP
  0,003–10, 0,707 · Res das shelves 0,55–10, 0,707 · Largura dos picos 0,1–4 oitavas, 0,3 · Frequências (log):
  HP 31, low-shelf 80, picos 120/250/2000/4000, high-shelf 12000, LP 18000 Hz · Inclinação do HP e do LP 12/24/48
  dB/oitava (12) · Analisar entrada, Analisar saída (ligados).
- **O usuário pode:** arrastar a alça numerada de cada banda na curva (frequência e ganho); mudar Q/largura com a roda
  sobre a alça (Ctrl = fino); ver os valores ao passar o mouse; ver a curva total e os **espectros de entrada e saída**
  por cima; medidores na entrada e na saída.
- **Na beta:** plugin `eq`: ganhos de entrada e saída, as 8 bandas (`hp…`, `lowShelf…`, `peak1…`–`peak4…`, `highShelf…`, `lp…`), `hpSlope`/`lpSlope`, `analyseIn`/`analyseOut`; medidores na janela. Diferença intencional: a ressonância de uma passagem de 24/48 dB/oct vai numa seção só, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-016 Flanger — 1.0

- **Onde:** `plugins/Flanger/`.
- **Parâmetros:** Atraso 0,0001–0,05 s, 0,001 · Taxa (período) 0,01–60 s, 0,25, sync · Quantidade 0–0,0025, 0 · Fase
  estéreo 0–360°, 90 · Realimentação −1–1, 0 · Ruído 0–0,05, 0 · Inverter realimentação, desligado.
- **Comportamento:** o LFO reinicia quando a reprodução começa ou para.
- **Na beta:** plugin `flanger`: `delayTime`, `lfoPeriod`, `lfoAmount`, `stereoPhase`, `feedback`, `noise`, `invert`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-017 FrequencyShifter (deslocador de frequência e phaser "barberpole") — 1.0

- **Onde:** `plugins/FrequencyShifter/`.
- **Parâmetros:** Deslocamento −20000–20000 Hz, 0 (visor numérico) · Mix 0–1, 1 · Spread −50–50 Hz, 0 · Fase −2–2 ·
  Ring 0–1 · Harmônicos 0–1 · Tom 100–22000 Hz, 22000 · Glide 0–1, 0,05 · LFO 0–2000 Hz, 0 · Taxa do LFO 0–200 Hz,
  0,2 · Fase estéreo do LFO 0–1 · Atraso 0–2000 ms e fino 0–20 ms · Realimentação 0–1 · Amortecimento do atraso
  100–22000 Hz, 22000 · Glide do atraso 0–1, 0,05 · Antirreflexo · Roteamento Send / Pass / Mute · Botões
  automatizáveis "zerar o deslocador" e "zerar o LFO".
- **O usuário pode:** abrir um texto de ajuda que explica cada controle.
- **Na beta:** plugin `frequencyshifter`: os 19 botões do legado (os dois botões de reinício viram parâmetros que agem ao ir a 1) e `routeMode`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-018 GranularPitchShifter — 1.0

- **Onde:** `plugins/GranularPitchShifter/`.
- **Parâmetros:** Altura −48–24 semitons, 1 · Espalhamento estéreo da altura −24–24 · Tamanho do grão 2–1000 Hz, 10 ·
  Spray 0–0,5 s, 0,005 · Jitter 0–1 oitava · Twitch 0–1 oitava · Espalhamento estéreo do spray 0–1 · Forma do grão
  1–2, 2 · Fade 0,001–1, 1 · Realimentação 0–1 · Latência mínima 0–1 s, 0,01 · Pré-filtro, ligado · Densidade 1–16,
  1 · Glide 0–1 s, 0,01 · Tamanho do buffer: 5 s / 10 s / 40 s / 40 s / 120 s (limita as faixas dos botões; trocar
  realoca).
- **O usuário pode:** abrir um texto de ajuda.
- **Quirk:** atraso variável não informado à compensação.
- **Na beta:** plugin `granularpitchshifter`: os 14 botões e `ringBufferLength` (trocar o anel é um processador novo que não adota: o velho toca a cauda), em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-019 Guitarix (pedais, amplificadores e gabinetes do Guitarix, embutidos, do fork) — 1.0

- **O que faz:** o DSP do Guitarix compilado dentro do programa. Cada unidade do Guitarix é um efeito separado:
  **97 efeitos** no navegador, agrupados pelas categorias do próprio Guitarix (distorção, fuzz, modulação, efeitos de
  guitarra, controle de tom…).
- **Onde:** `plugins/Guitarix/` (`gx/` tem as fontes do Guitarix).
- **Unidades:** gain, low_highpass, highbooster, eqs, cabeçote (ampstack), crybaby, poweramp, wah, detune, várias
  distorções e fuzzes (ts9sim, bossds1, mxrdis, fuzzface, muff, rangem…), compressor, expander, overdrive, echo,
  delay, freeverb, tremolo, phaser, chorus, flanger, eq gráfico, gabinete (cab), pré (pre), noise gate, ring
  modulator, univibe, multibanda (mbc, mbd, mbe…), e as estéreo (chorus, flanger, phaser, stereodelay, stereoecho,
  moog, zita_rev1, stereoverb, univibe, panoram_enhancer, bassEnhancer…). A lista completa está em
  `GuitarixUnits.cpp`.
- **Parâmetros:** os que cada unidade declara (nome, faixa, passo, escala log, listas de escolha); medidores de saída
  aparecem como medidores. Unidades de seleção: Crybaby (crybaby/autowah), Wah (10 modelos), Tonestack (27 modelos:
  bassman, twin, princeton, jcm800, ac30, mesa…), PowerAmp (6 modelos). O cabeçote tem 19 modelos de válvula, gate de
  entrada e saída, presença por convolução, reverb de realimentação, limitador.
- **IRs embutidas:** gabinete 17 (4x12, 2x12, 1x12, 4x10, 2x10, estilos HighGain, Twin, Bassman, Marshall, AC-30,
  Princeton, A2, 1x15, Mesa…); pré 10 (AC30, Bassman, Tube, Fender, JCM800, JTM45, Mesa, Boutique, Ampeg,
  Rectifier). Não carrega arquivo nenhum.
- **O usuário pode:** usar a tela de cada unidade refeita a partir do layout original do Guitarix (botões, seletores
  com menu, chaves, sliders, medidores de LED, abas).
- **Comportamento:** unidade mono roda uma cópia por lado; processa em blocos fixos de até 256; latência do detune
  não informada.
- **Status:** 1.0 (como 97 efeitos internos; a forma de mostrá-los no navegador do programa novo é de quem desenhar o
  navegador).

#### F-FX-020 LOMM (compressor multibanda para cima e para baixo, estilo OTT) — 1.0

- **Onde:** `plugins/LOMM/`.
- **Global:** Profundidade 0–1, 0,4 · Tempo 0–10, 1 (escala ataque e release) · Volume de entrada −48–48 dB, 0 ·
  Volume de saída −48–48 dB, 8 · Profundidade para cima 0–2, 1 · para baixo 0–2, 1 · Mix 0–1, 1 · Tempo automático
  0–1, 0 · RMS 0–500 ms, 10 (0 = pico) · Knee 0–36 dB, 6 · Range 0–96 dB, 36 · Balanço −18–18 dB · Escalar a saída
  pela profundidade (ligado) · Stereo Link · Feedback · Mid/Side · Lookahead (liga) e duração 0–20 ms · Suprimir a
  compressão para cima no lado (só em M/S).
- **Cruzamentos:** Alto/Médio 20–20000 Hz, 2500 · Médio/Grave 20–20000 Hz, 88,3; cada um pode ser desligado.
- **Por banda (alta, média, baixa):** liga · Volume de entrada e de saída −48–48 dB (saída 4,6 / 0 / 4,6) · Limiar
  acima −72–0 dB (−30,3 / −25 / −28,6) e Razão acima 1–99,99 (99,99 / 66,7 / 66,7) · Limiar abaixo (−35,6 / −36,6 /
  −35,6) e Razão abaixo (4,17) · Ataque 0–1000 ms (13,5 / 22,4 / 47,8) · Release 0–1000 ms (132 / 282 / 282).
- **O usuário pode:** ver os níveis de entrada e saída por banda; **arrastar os limiares direto no visor** (à
  esquerda move o de baixo, à direita o de cima, no meio os dois); clique do meio volta o limiar; "Init" zera tudo
  para um ponto neutro (pergunta antes, desfazível).
- **Na beta:** plugin `lomm`: os 50 botões do legado em camelCase; latência do lookahead informada; medidores das 3 bandas na janela, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-021 MultitapEcho — 1.0

- **Onde:** `plugins/MultitapEcho/`.
- **Parâmetros:** Passos (ecos) 4–32, 16 · Duração do passo 1–500 ms, 100, sync · Ganho seco no buffer −80–20 dB, 0 ·
  Trocar entradas (pingue-pongue), desligado · Estágios do passa-baixa 1–4, 1 · Gráfico de amplitude por eco (−60–0
  dB, desenhado) · Gráfico de corte por eco (20 Hz–20 kHz, desenhado).
- **O usuário pode:** desenhar os dois gráficos em barras. Quirk: só os ecos 0–19 têm o filtro certo ao abrir.
- **Na beta:** plugin `multitapecho`: `steps`, `stepLength`, `dryGain`, `swapInputs`, `lowpassStages`, campos `amplitude` e `lowpass` (32 valores cada), em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-022 Oscilloscope — 1.0

- **Onde:** `plugins/Oscilloscope/`. Só mostra; o áudio passa intacto.
- **Parâmetros:** Escala 0–1000 %, 100 · Janela 1–5000 ms, 500 · Deslocamento no histórico 0–1, 1 · Pausa · Estéreo
  (L e R em cores) ou mono.
- **O usuário pode:** dar zoom com a roda em torno do cursor; arrastar para voltar até 5 s no histórico; ver linhas de
  clipe.

#### F-FX-023 Peak Controller (seguidor de envelope que vira controlador) — 1.0

- **O que faz:** mede o nível do sinal e cria um **controlador** que qualquer botão pode seguir (seção 8).
- **Onde:** `plugins/PeakControllerEffect/`, `src/core/PeakController.cpp`.
- **Parâmetros:** Base 0–1, 0,5 · Quantidade −1–1, 1 · Multiplicador 0–32, 1 · Ataque 0–0,999, 0 · Release 0–0,999,
  0 · Limiar 0–1, 0 · Silenciar a saída (para usar como sidechain), desligado · Valor absoluto, ligado.
- **Comportamento:** um valor por período (resolução do período de áudio).

#### F-FX-024 ReverbSC — core (plugin de validação, ou o Delay)

- **Onde:** `plugins/ReverbSC/` (Soundpipe). Já portado para o `lm-dsp`.
- **Parâmetros:** Ganho de entrada −60–15 dB, 0 · Tamanho 0–1, 0,89 · Cor 100–15000 Hz, 10000 · Ganho de saída
  −60–15 dB, 0. Todos com precisão de amostra.
- **Na beta:** plugin `reverbsc`: `inputGain`, `size`, `color`, `outputGain`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-025 SlewDistortion (distorção de 2 bandas e limitador de slew) — 1.0

- **Onde:** `plugins/SlewDistortion/`.
- **Por unidade (2):** Tipo (Hard Clip, Tanh, Soft Clip 1, Soft Clip 2, Senoidal, Foldback, Retificação completa,
  meia, suave, Bitcrush) · Drive −24–24 dB, 0 · Slew Up e Slew Down −96–6, 6, e Ligar os dois (ligado) · Bias −2–2 ·
  Warp 0–0,99 · Crush 0–24 · Volume de saída −24–24 dB · Ataque 0,01–200 ms, 2 · Release 0,01–800 ms, 20 · Dinâmica
  0–1 · Slew dinâmico −8–8 · Mix 0–1, 1.
- **Global:** Remover DC (ligado) · Multibanda, desligado · Divisão 100–20000 Hz, 200 · Sobreamostragem 1×, 2×, 4×,
  8×, 16×, 32×.
- **O usuário pode:** arrastar Drive, Bias e Saída como alças; ver as curvas de transferência e os medidores; abrir a
  ajuda.
- **Na beta:** plugin `slewdistortion`: os 13 botões de cada banda, `type1`/`type2`, `dcRemove`, `multiband`, `oversample`, `split`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-026 Spectrum Analyzer — 1.0

- **Onde:** `plugins/SpectrumAnalyzer/`. Só mostra; analisa só com a janela aberta.
- **Controles:** Pausa · Congelar referência · Cascata (espectrograma) · Média · Estéreo · Retenção de pico · Frequência
  log (ligado) · Amplitude log (ligado) · Faixa de frequência: completa (padrão), audível, graves, médios, agudos ·
  Faixa de amplitude: estendida, audível (padrão), alta, baixa · Tamanho da FFT 256–16384 (2048) · Janela:
  retangular, Blackman-Harris (padrão), Hamming, Hanning.
- **Avançado:** Resolução do envelope 0,1–3, 0,25 · Resolução do espectro 0,1–3, 1,5 · Queda do pico 0,95–0,999,
  0,992 · Peso da média 0,01–0,5, 0,15 · Linhas da cascata 50–1000, 300 · Gama da cascata 0,1–1, 0,3 · Sobreposição
  1–4, 2 · Zero padding 0–4, 2.
- **O usuário pode:** ler frequência e dB (ou tempo, na cascata) sob o cursor.

#### F-FX-027 StereoEnhancer — 1.0

- **Onde:** `plugins/StereoEnhancer/`. **Parâmetro:** Largura 0–180, 0. Quirk: o mesmo valor é atraso em amostras e
  ângulo em graus.
- **Na beta:** plugin `stereoenhancer`: `width`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-028 StereoMatrix — obsoleto

- **O que fazia:** matriz 2×2 (L→L 1, L→R 0, R→L 0, R→R 1, cada um −1–1): trocar lados, somar em mono, inverter
  polaridade. Já aposentado no fork.
- **Status:** obsoleto — substituído pela fase, troca e mono do canal da mesa (F-MIX).
- **Na beta:** plugin `stereomatrix` (para abrir projetos antigos): `leftToLeft`, `leftToRight`, `rightToLeft`, `rightToRight`, em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-029 Vectorscope — 1.0

- **Onde:** `plugins/Vectorscope/`. Só mostra.
- **Parâmetros:** Escala log, desligado · Linhas (ou pontos), ligado.
- **O usuário pode:** zoom 20–1000 % na roda; escolher a cor do traço com duplo clique.

#### F-FX-030 WaveShaper (curva de transferência desenhada) — 1.0

- **Onde:** `plugins/WaveShaper/`.
- **Parâmetros:** Entrada 0–5, 1 · Saída 0–5, 1 · Cortar a entrada em 0 dB, desligado · Curva (200 pontos, 0–1),
  linear.
- **O usuário pode:** desenhar a curva (Shift+clique traça reta); voltar à linear; suavizar; +1 dB / −1 dB.
- **Na beta:** plugin `waveshaper`: `inputGain`, `outputGain`, `clipInput`; campo `shape` (a curva de 200 pontos), em `lm-plugins-effects` (contrato e A/B pelo plugin contra o legado).

#### F-FX-031 Efeitos LADSPA — aberto

- **O que faz:** hospeda efeitos (e "instrumentos" geradores) LADSPA. Conjuntos embutidos compilados das fontes:
  calf (veal), caps, cmt, swh, tap.
- **Onde:** `src/core/LadspaManager.cpp`, `Ladspa2LMMS.cpp`, `LadspaControl.cpp`, `plugins/LadspaEffect/`.
- **Busca:** `LADSPA_PATH`, a pasta das configurações, a pasta embutida, `/usr/lib/ladspa`, `/usr/lib64/ladspa`,
  `/usr/local/lib…`, `/Library/Audio/Plug-Ins/LADSPA`; tudo carregado na partida.
- **Parâmetros:** um controle por porta: liga/desliga, inteiro, tempo (portas "(s)" viram ms e podem seguir o
  andamento), contínuo (escala log quando pedido); faixa ausente vira 0–1 (ou 0–10 para "GAIN"); uma porta de saída
  chamada "latency" informa a latência. Plugins mono rodam em dois (L e R) com "ligar canais" por controle.
- **O usuário pode:** editar numa grade com barras por parâmetro e por canal. Sem presets.
- **Status:** **aberto** (pergunta P-03): o desenho novo lista LV2, CLAP e VST3; LADSPA não é citado. Os conjuntos
  embutidos (calf, caps, swh, tap) cobrem efeitos que muita gente usa.

#### F-FX-032 Efeitos LV2 — 1.0

- **O que faz e onde:** o mesmo hospedeiro de F-HOST-003, para plugins com entrada de áudio. Busca padrão do lilv;
  plugins com problema não aparecem; lista de bloqueio (desligável). Sem interface própria, sem estado salvo, sem
  presets no legado.
- **Status:** 1.0 (fase 2, fora do processo, com estado e presets).

#### F-FX-033 Efeitos VST2 — aberto

- **O que faz:** como F-HOST-001, para efeitos; procura `*.dll`/`*.so` na pasta de VST das configurações. Janela com
  mostrar/esconder a interface, gerenciar parâmetros, presets `.fxp`/`.fxb`, programa anterior/próximo e menu.
- **Status:** **aberto** (P-01).

#### F-FX-034 Conector de pinos — obsoleto

- **O que faz:** por plugin, uma grade que liga os canais da faixa às entradas do plugin e as saídas do plugin aos
  canais da faixa (clicar na célula liga/desliga); por padrão as saídas extras (peças da bateria) vão ao par
  principal e a entrada de sidechain aos canais 3/4.
- **Onde:** `src/gui/PinConnector.cpp`, `src/core/AudioPortsModel.cpp`.
- **Status:** obsoleto — substituído pelas **saídas da faixa** (uma por porta declarada do instrumento, cada uma para
  um canal da mesa), pelas conversões explícitas de canais do compilador e pelos envios de sidechain para a porta do
  insert (`arquitetura-nova.md`, especificação itens 2 e 3).

#### F-FX-035 Proteção contra plugins que derrubam o programa — obsoleto (em parte)

- **O que faz:** o relatório de queda diz qual plugin rodava; ele entra numa lista de suspeitos. Da próxima vez: um
  LADSPA ou LV2 suspeito roda num processo próprio, com aviso; os outros viram fantasma ao abrir o projeto (com aviso)
  ou o programa pergunta "carregar mesmo assim?". Processo separado travado por mais de 2 s é morto e o plugin fica
  mudo. Nas configurações, "Não carregados desde uma queda" com o botão "Carregar de novo".
- **Onde:** `src/core/PluginGuard.cpp`, `src/core/RemotePlugin.cpp`, `src/core/CrashHandler.cpp`,
  `plugins/LadspaEffect/RemoteLadspaHost.cpp`, `plugins/Lv2Effect/RemoteLv2Host.cpp`.
- **Status:** obsoleto — substituído por: **terceiros sempre fora do processo**; se o processo cair, a faixa fica muda
  com aviso e o plugin é recriado da árvore (`arquitetura-nova.md`, item 7). Fica como 1.0 só o aviso ao usuário e
  um jeito de recriar o plugin (a ação "recriar o plugin" do aviso, I-14).

#### F-FX-036 Categorias de plugins — 1.0

- **O que faz:** cada plugin interno tem categoria, cor e sigla. Instrumentos: Sintetizadores (subtrativo, wavetable,
  aditivo, cordas, fórmulas), Samplers, Baterias, Modelagem física, Chiptune, Hospedeiros. Efeitos: ver F-FX-003.
  Plugins de terceiros agrupados por hospedeiro.
- **Onde:** `src/core/PluginCategories.cpp`. Quirk: o Spectrum Analyzer cai em "Outros" por um nome trocado.

### 7.3 Ferramentas e filtros de arquivo que são plugins no legado

#### F-FX-037 Tap Tempo — 1.0

- **O que faz:** bater o andamento num botão grande (ou na barra de espaço); toca um clique a cada batida (o primeiro
  tempo do compasso com outro som), a menos que "Mudo" esteja marcado; mostra o BPM, o período em ms e a frequência
  em Hz (com uma casa decimal se "Precisão"); "Reset" zera; "Sync" põe o andamento da música no valor medido.
  Batidas com mais de 2 s de intervalo recomeçam; depois de 3 batidas usa a mediana das últimas 3, suavizada.
- **Onde:** `plugins/TapTempo/` (menu Ferramentas).

#### F-FX-038 Importar MIDI — 1.0

- Descrito em F-PRJ (seção 9), onde a funcionalidade mora. Código em `plugins/MidiImport/`.

#### F-FX-039 Exportar MIDI — 1.0

- Descrito em F-PRJ (seção 9). Código em `plugins/MidiExport/`.

## 8. Automação e controladores

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-CTL-001 | Rack de controladores | Lista dos controladores do projeto; "Adicionar" cria um LFO (único tipo que se adiciona ali); os Peak Controllers aparecem quando o efeito é criado; cada um: abrir os controles, renomear (duplo clique), mover acima/abaixo (Alt+↑/↓), remover (avisa se tem ligações, "não dá para desfazer") | `src/gui/ControllerRackView.cpp` | 1.0 |
| F-CTL-002 | LFO controlador | Base 0–1, 0,5 · Velocidade 0,01–20, 2, sync · Quantidade −1–1, 1 · Fase 0–360°, 0 · Forma: senoide, triângulo, serra, quadrada, serra Moog, exponencial, ruído branco, do usuário (duplo clique escolhe o arquivo) · Multiplicador ×1 / ×100 / ÷100 | `src/core/LfoController.cpp` | 1.0 |
| F-CTL-003 | Peak controller | Ver F-FX-023 | `src/core/PeakController.cpp` | 1.0 |
| F-CTL-004 | Ligar um controle a um controlador ou ao MIDI (MIDI learn) | Janela de conexão: controlador MIDI (canal, número do CC, "**detectar automaticamente**": o próximo CC que chegar define os dois, dispositivos de entrada) ou um controlador do projeto; função de mapeamento (só um texto, não implementada); recusa laços | `src/gui/modals/ControllerConnectionDialog.cpp` | 1.0 |
| F-CTL-005 | Menu de todo controle | Voltar ao padrão (mostra o valor), copiar o valor, colar o valor, tirar as ligações com outros controles, conectar a um controlador / editar ou tirar a conexão; nos botões: escala linear ou logarítmica; nos botões com sync: as notas (Sem sync, 8 tempos, inteira, meia, semínima, colcheia, semicolcheia, fusa, personalizada) | `src/gui/AutomatableModelView.cpp`, `TempoSyncKnob.cpp` | core (padrão, copiar, colar) · 1.0 (o resto; linear/log por botão: P-06) |
| F-CTL-006 | Gestos em todo controle | Botão do meio volta ao padrão; arrastar na vertical (Shift = 4× mais devagar); Shift+clique arrasta o **valor** para outro botão; Ctrl+arrasto (Alt no Mac) arrasta o **controle**: soltar noutro controle **liga os dois**, soltar numa faixa/clipe de automação automatiza; roda: 100 passos o curso todo (Shift 10, Ctrl 1000, Alt 2000); duplo clique pede o valor (volume em dBFS); passar o mouse mostra o valor depois de 700 ms; um desfazer por arrasto | `FloatModelEditorBase.cpp`, `Knob.cpp`, `LcdSpinBox.cpp` | core (arrastar, roda, duplo clique, padrão, um desfazer) · 1.0 (arrastar valor) · aberto (ligar dois controles: P-04) |
| F-CTL-007 | Automação em clipes na linha do tempo | Ver F-CLP-080 a F-CLP-083 e F-AUT | — | core (curvas no dono do parâmetro) |
| F-CTL-008 | Gravar automação | Ver F-CLP-081 | — | 1.0 |
| F-CTL-009 | A mão vence a automação | (No novo: com a vaga "tocada", a mão vence; I-6) | — | core |

## 9. Projeto e arquivos

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-PRJ-001 | Novo projeto | Abre o modelo padrão do usuário, senão o de fábrica, senão um projeto vazio (andamento padrão, compasso 4/4) | `Song::createNewProject` | core |
| F-PRJ-002 | Modelos | "Novo a partir de modelo" (do usuário e de fábrica: CR8000, ClubMix, Empty, TR808, default); abrir um modelo deixa o projeto sem nome; "Salvar como modelo padrão" | `TemplatesMenu.cpp` | 1.0 |
| F-PRJ-003 | Abrir projeto | Pergunta antes de perder mudanças; cursor de espera; carregamento cancelável (vira projeto novo); erros juntados num relatório ("repetido N vezes"); aviso quando a versão é outra | `Song`, `DataFile.cpp` | core (abrir a pasta com `projeto.json`, com estado de carregamento, I-2) |
| F-PRJ-004 | Abrir `.mmp`/`.mmpz` | O formato do LMMS em XML (compactado ou não); projetos do LMMS antigo abrem "como estão", sem conversão | `DataFile.cpp` | obsoleto — o programa novo não abre `.mmp` (decisão de 19/11) |
| F-PRJ-005 | Recusar projeto perigoso | Recusa projetos com caminhos `local:` de plugin ("poderia rodar código malicioso") | `Song` | core (C9: um projeto de fora não lê arquivos quaisquer) |
| F-PRJ-006 | Guardar o que não entende | Elementos desconhecidos do projeto são guardados e salvos de volta | `Song` | core (o mesmo vale para a árvore de um plugin ausente) |
| F-PRJ-007 | Salvar e Salvar como | Salvar sem nome vira Salvar como; sobrescrever pergunta; mensagem "Projeto salvo" / "Projeto NÃO salvo"; atualiza os recentes | `MainWindow`, `VersionedSaveDialog.cpp` | core (salvar atômico) |
| F-PRJ-008 | Salvar como nova versão | Ctrl+Alt+S: aumenta o número da versão no nome até um livre; botões +/− de versão no diálogo | `VersionedSaveDialog.cpp` | 1.0 |
| F-PRJ-009 | Compactar o arquivo | `.mmpz` (padrão) ou `.mmp` | configuração | obsoleto — o projeto é uma pasta com JSON legível |
| F-PRJ-010 | Cópia de segurança ao salvar | A versão anterior fica ao lado (ligado) | configuração | core (cópias em `backup/`) |
| F-PRJ-011 | Pacote do projeto | "Salvar como pacote (com os recursos)": pasta com o projeto e `resources/` | `ProjectMedia` | obsoleto — substituído por "exportar como arquivo único" (`.zip` da pasta), 1.0 |
| F-PRJ-012 | Descartar as conexões MIDI ao salvar | Opção do Salvar como | `VersionedSaveDialog.cpp` | aberto (P-36) |
| F-PRJ-013 | Juntar as mídias ao salvar | Arquivos fora da pasta são copiados para `<projeto>_media/` (o mesmo nome e conteúdo é reaproveitado), os de dentro ficam relativos, os de fábrica ficam onde estão | `ProjectMedia::gather` | core (no novo: importar copia para `audio/` pelo hash) |
| F-PRJ-014 | Prefixos de caminho | `userprojects:`, `factorysample:`, `usersoundfont:` etc. no projeto | `src/core/PathUtil.cpp` | obsoleto — recursos por hash; kits e SoundFonts grandes do sistema por caminho + hash |
| F-PRJ-015 | Recentes | Lista de 50, mostra os 15 mais recentes que existem (sem modelos e sem a recuperação) | `RecentProjectsMenu.cpp` | core |
| F-PRJ-016 | Reabrir o último projeto na partida | Configuração (desligada) | configurações | 1.0 |
| F-PRJ-017 | Autosave e recuperação | Ver F-REC-069; o arquivo de recuperação também marca "instância rodando"; na partida: Recuperar / Descartar | `Autosave.cpp`, `main.cpp` | core (diário; H10 para o projeto aberto duas vezes) |
| F-PRJ-018 | Notas do projeto | Texto rico salvo no projeto: desfazer/refazer, copiar/cortar/colar, fonte, tamanho, negrito (Ctrl+B), itálico (Ctrl+I), sublinhado (Ctrl+U), alinhamento (Ctrl+L/E/R/J), cor | `src/gui/ProjectNotes.cpp` | 1.0 |
| F-PRJ-019 | Importar MIDI | SMF e RIFF RMID; uma faixa por (faixa do SMF × canal), com o Sf2 Player e o SoundFont padrão (PatMan se não houver; avisa se não há SoundFont padrão); canal 10 → banco 128; mudanças de andamento e de compasso vão para o mapa; CC7 → volume, CC10 → pan, pitch bend → afinação, os outros CC → rack de CC; program change → patch; as notas viram clipes separados onde há silêncio de mais de um compasso; progresso | `plugins/MidiImport/` | 1.0 (no novo: CC10 vai para onde, sem pan na faixa? P-39) |
| F-PRJ-020 | Importar arquivos de áudio | Vários de uma vez; cada um numa faixa nova no compasso do cursor ("stems") | `MainWindow` | core |
| F-PRJ-021 | Exportar MIDI | SMF tipo 1, 128 tiques por semínima; o mapa de andamentos em toda faixa (rampa = um evento por semínima); uma faixa MIDI por faixa de instrumento e por linha de padrão; canais em ordem pulando o 10 (banco 128 vai para o 10); banco e programa do instrumento; velocidade escalada pelo volume da faixa. Não exporta automação, CC, pan, mudo. Sem opções. Também "Exportar a faixa como MIDI" | `plugins/MidiExport/` | 1.0 |
| F-PRJ-022 | Exportar áudio (assistente) | **O quê:** a música inteira ou o trecho do loop; "como loop" (sem a cauda); repetir 1–64 vezes. **Formatos, vários de uma vez:** WAV 16/24 (padrão)/32 float; FLAC 16/24; OGG qualidade 0–10 (6); MP3 128/192/256/320 kbps (320); taxa 44,1/48 (padrão)/88,2/96 kHz. **Processamento:** normalizar nenhum / pico (dBFS, −1) / loudness (LUFS, ITU-R BS.1770, −14), alvo −60–0; fade in e fade out 0–30 s; dither TPDF para 16 e 24 bits (ligado). **Onde:** pasta (padrão `<projeto>/exports`) e nome. Renderiza uma vez e escreve os formatos; cancelável; mostra os arquivos e abre a pasta | `src/gui/modals/ExportWizard.cpp`, `src/core/ExportProcessor.cpp` | core (WAV, a música inteira, determinístico, I-13) · 1.0 (o resto) |
| F-PRJ-023 | Exportar uma faixa, um clipe, uma região | O mesmo assistente limitado à faixa (as outras mudas), ao trecho do clipe, ou à região (com o nome dela) | menus da faixa, do clipe e da região | 1.0 |
| F-PRJ-024 | Exportar as faixas (stems) | Escolher a pasta; formato WAV/FLAC/OGG/MP3 com taxa (44,1–192 kHz), profundidade (16/24/32 float), compressão do FLAC 0–8, bitrate 64–320 (160), modo do MP3 (mono/estéreo/joint); "como loop (sem o compasso extra)", "entre os marcadores do loop", "renderizar o loop N vezes"; um arquivo por faixa não muda (`<n>_<faixa>.<ext>`), respeitando o solo | `ExportProjectDialog.cpp`, `RenderManager.cpp` | 1.0 (no novo: exportar por faixa ou por canal da mesa? P-40) |
| F-PRJ-025 | Exportar o trecho: rápido ou exato | (Novo: "Rápido" com 10 s de aquecimento, ou "Exato"; a música inteira é sempre exata) | — | (novo; só para registro) |
| F-PRJ-026 | Exportar a amostra de um clipe | Escreve o buffer como FLAC, fora da thread principal | `ThreadedExportManager.cpp` | 1.0 |
| F-PRJ-027 | Preset de efeito e de faixa | `.xfx` (um efeito), `.xpf` (faixa de instrumento) | `DataFile` | 1.0 (no formato novo; P-09 para os de fábrica) |

## 10. Configurações

O legado tem as configurações como página, com busca, que aplicam na hora (o design 8 aprovado). Cada linha diz se
"aplica na hora" ou "pede reinício"; uma faixa no alto oferece "Mais tarde" / "Reiniciar agora" (reinicia e reabre o
projeto). O arquivo de configuração é gravado 500 ms depois da última mudança. Onde: `src/gui/modals/SetupDialog.cpp`,
`src/core/ConfigManager.cpp`.

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-CFG-001 | Busca nas configurações | Procura nos títulos das seções, nos rótulos e nas dicas, sem acento; mostra só as linhas que batem | core |
| F-CFG-002 | Aplicar na hora e reinício | Sem OK/Cancelar; o que pede reinício avisa, com "Reiniciar agora" | core |
| F-CFG-010 | Geral | Reabrir o último projeto (desligado) · Compactar os projetos (ligado; obsoleto) · Guardar cópia ao salvar (ligado) · Autosave (ligado) · Projetos novos usam a mesa analógica (desligado; P-17) · Arquivos de áudio longos viram faixa de áudio (1–3600 s) · Prévias tocam até o fim (desligado) · Perguntar antes de apagar faixa (ligado) · Perguntar antes de apagar canal em uso (ligado) · Idioma (reinicia; padrão o do sistema) | core (autosave, idioma) · 1.0 (o resto) |
| F-CFG-011 | Áudio | Sistema de áudio (JACK, ALSA, PulseAudio, CoreAudio, PortAudio, SDL, sndio, Dummy); o bloco do driver (F-REC-123 a F-REC-128); taxa para projetos novos; tamanho do bloco com a latência medida e avisos (não potência de 2, ≤ 32 quadros); linha de estado com o que o motor informou | core (ALSA, nulo) · 1.0 (JACK, CoreAudio) |
| F-CFG-012 | MIDI | Sistema MIDI (ALSA seq, ALSA raw, JACK MIDI, sndio, WinMM, Apple MIDI, Dummy); dispositivo para os do tipo raw; controlador para a faixa selecionada; quantizar o que se grava no editor de notas | core (ALSA seq) · 1.0 |
| F-CFG-013 | Gravação | Calibração (F-REC-081, F-REC-082) · Pré-roll · Contagem · Takes em camadas (P-11) · Metrônomo: quando, volume, acento | core |
| F-CFG-014 | Plugins | Janelas VST: sem embutir / Qt / Win32 / XEmbed · Janelas de plugin por cima (desligado) · "Não carregados desde uma queda" com "Carregar de novo" | aberto (P-01 para VST) · obsoleto: a lista de suspeitos (F-FX-035) |
| F-CFG-015 | Pastas | Pasta de trabalho (reinicia) · Plugins VST · Plugins LADSPA (lista) · SoundFonts · **SoundFont padrão** · Arquivos GIG | 1.0 (VST e LADSPA: P-01, P-03) |
| F-CFG-016 | Aparência | Dicas (ligadas) · Cabeçalhos de faixa compactos · Navegador à direita · Uma janela de instrumento por vez · Nomes das notas no editor · Marcas no fader a cada 6 dB · Rolagem suave no arranjo · Posição tocando no AudioFileProcessor (ligada) · Janelas destacadas: encaixar e mostrar / encaixar e esconder / sempre destacadas · Modo de editar o loop (F-REG-007) · Seguir o cursor: não / por página (padrão) / contínuo · Pasta do tema (reinicia) | 1.0 (o tema: P-08) |
| F-CFG-017 | Teclado e atalhos | Tabela ação → tecla, com "padrão", para as 9 ações | core (todas as ações) |
| F-CFG-018 | Avançado | Limpar áudio corrompido (ligado) · Efeitos continuam rodando sem entrada (ligado; P-07) · Memória de tempo real (64 MB) | core (limpar) · aberto (o resto) |
| F-CFG-019 | Primeira execução e pasta de trabalho | Pergunta se cria a pasta de trabalho; cria `projects/`, `templates/`, `samples/`, `presets/`, `samples/gig/`, `samples/soundfonts/`, `vst/`, `plugins/ladspa/`, `logs/` quando precisa; modo portátil com um arquivo ao lado do programa | 1.0 (modo portátil: P-35) |

## 11. Navegador, presets e amostras (hoje: `src/gui/BrowserPanel.cpp`, `FileBrowser.cpp`, `PluginBrowser.cpp`)

| Id | Funcionalidade | O que faz | Status |
|---|---|---|---|
| F-BRW-001 | Páginas do navegador | Instrumentos, Favoritos, Projetos, Amostras, Presets, Arquivos do projeto (a pasta do projeto atual); Efeitos na Mixagem. Lembra a página e se está visível. Não há "Home" nem "Raiz" | core (instrumentos, amostras, efeitos) · 1.0 (o resto) |
| F-BRW-002 | Pastas do usuário e de fábrica juntas | As duas numa árvore só (subpastas iguais se juntam), os de fábrica depois de uma linha "Arquivos de fábrica"; ordem: pastas primeiro, nome sem caixa; F5 atualiza; as pastas abertas ficam abertas | 1.0 |
| F-BRW-003 | Tipos reconhecidos | Projetos, presets (`xpf`, `xml`, `xiz`, `lv2`), SoundFonts (`sf2`, `sf3`), patches (`pat`), MIDI (`mid`, `midi`, `rmi`), VST (`dll`, `so`), áudio (wav ogg ds flac spx voc aif aiff au raw, mp3), modelos NAM (`.nam`); o resto pelo tipo MIME ou pelo plugin que diz aceitar | core (áudio) · 1.0 |
| F-BRW-004 | Busca | Ctrl+F; em segundo plano, recursiva; palavras separadas por espaço, "frases entre aspas"; todas as palavras no nome; pastas e arquivos; barra de progresso; uma busca nova cancela a anterior | 1.0 |
| F-BRW-005 | Prévia | Clique num arquivo toca: amostra (com aviso de carregando); SoundFont, `.xiz`, `.gig`, `.pat`, presets e faixas: uma nota longa na tecla padrão. Para ao soltar o botão, a menos que seja amostra e "prévias até o fim" esteja ligado; para ao perder o foco ou trocar de página. ↑/↓ andam e tocam as amostras; Espaço segurado toca o selecionado | 1.0 (I-7.3, com estado de carregamento) |
| F-BRW-006 | Enter e duplo clique | Enter = para o arranjo (faixa de instrumento nova, ou sampler para amostras); Ctrl+Enter = para o editor de padrões; Shift+Enter = faixa de amostra nova. Duplo clique: projeto abre, MIDI importa, áudio longo vira faixa, o resto vira instrumento | 1.0 (sem o editor de padrões) |
| F-BRW-007 | Arrastar do navegador | Presets, amostras, SoundFonts, patches, VST, MIDI, projetos (os destinos estão em F-ARR-011, F-TRK-032, F-IJN-004) | core |
| F-BRW-008 | Menu do arquivo | Caminho; mostrar no gerenciador de arquivos (selecionando o arquivo, conforme o sistema); favoritar/desfavoritar arquivo ou pasta; mandar para a faixa de instrumento ativa; mandar para uma faixa de instrumento nova; para um sampler novo; para uma faixa de amostra nova; para um SlicerT novo; pasta: abrir no gerenciador | 1.0 |
| F-BRW-009 | Instrumentos | Busca; árvore por categoria (cor, sigla, subcategorias dos sintetizadores), "Outros" no fim; plugins com subplugins ganham a própria raiz; ícone e nome, descrição na dica; arrastar para uma faixa; "mandar para uma faixa de instrumento nova" (carrega em segundo plano) | core |
| F-BRW-010 | Efeitos | Ver F-FX-003 | core |
| F-BRW-011 | Miniaturas de onda | Picos em várias resoluções, em segundo plano | core (os picos do I-12) |
| F-BRW-012 | Conteúdo de fábrica | **Projetos:** 4 demos ("shorties"), 5 modelos, 1 tutorial. **Presets:** AudioFileProcessor 6, BitInvader 14, Kicker 10, LB302 6, Monstro 4, Nescaline 5, OpulenZ 15, Organic 10, SID 6, Sf2Player 58, TripleOscillator 72, Vibed 2, Watsyn 3, Xpressive 22, ZynAddSubFX 962. **Amostras** (~23 MB): baixos, loops de baixo, batidas, baterias eletrônicas, baterias, 746 patches DrumSynth, efeitos, 11 IRs, diversos, formas, o SoundFont padrão (FluidR3Mono_GM.sf3), cordas e pads, formas de onda. **Kits AVL** (151 MB). Tabelas de onda | 1.0 (os projetos e os presets precisam ser refeitos no formato novo: P-09) |

## 12. Outros

| Id | Funcionalidade | O que faz | Onde (hoje) | Status |
|---|---|---|---|---|
| F-GER-001 | Renderizar pela linha de comando | `render <projeto>` e `rendertracks <projeto>`, com formato (wav/flac/ogg/mp3), float 32, bitrate, modo do MP3, loop, saída, taxa, perfil do motor; progresso no console | `src/main.cpp` | core (`lm-headless render`) |
| F-GER-002 | Outras ações da linha de comando | `dump`, `compress`, `upgrade` (XML do formato velho), `makebundle`; `--import <midi> [-e]`; `--geometry`; `--config`; `--allowroot` (recusa rodar como root); `--version` com plataforma e opções | `src/main.cpp` | obsoleto: dump, compress, upgrade (formato velho) · aberto (P-35): import, geometry, config, root |
| F-GER-003 | Variáveis de ambiente | Pasta de dados extra; pasta de configuração isolada (testes); captura de tela automática para desenvolvimento; lixeira de teste; excluir plugins por expressão; ligar plugins bloqueados; checagens de API de plugin | `main.cpp`, `PluginFactory.cpp` | aberto (P-35) |
| F-GER-004 | Relatório de queda | Um manipulador de sinal escreve o backtrace em `crash-report.txt`; na partida seguinte avisa "o programa fechou de repente" e diz o plugin suspeito | `src/core/CrashHandler.cpp` | 1.0 |
| F-GER-005 | Registro de erros | `logs/errors.log` e stderr; sem trava, usável do áudio; repetidos juntados "(×N)"; aviso/erro com hora e lugar | `src/core/ErrorLog.cpp` | core |
| F-GER-006 | Avisos ao usuário | Dica flutuante, informação, aviso, erro, pergunta sim/não, progresso com Cancelar; sem tela só registram | `src/core/UserNotice.cpp` | core (I-14) |
| F-GER-007 | Temas | "default" (ícones e imagens completos) e "linuxmusic" (cinzas planos e acento verde-azulado); escolhido pela pasta | `data/themes`, `LmmsPalette.cpp` | obsoleto — substituído por temas como dado (`arquitetura-nova.md`, item 12); P-08 |
| F-GER-008 | Traduções | 37 idiomas herdados do LMMS (os textos novos do fork quase sem tradução); as do Qt também | `data/locale` | core (português e inglês desde o começo) · aberto (P-08: os outros 35) |
| F-GER-009 | Sobre | Versão e ambiente (selecionável), autores, colaboradores, tradutores, licença | `AboutDialog.cpp` | 1.0 |
| F-GER-010 | Escolher cor | Paleta de 48 cores (padrão, paleta de faixa 8 matizes × 6 tons, paleta da mesa) | `ColorChooser.cpp` | 1.0 |
| F-GER-011 | Diálogo de arquivo | Atalhos para Mesa, Downloads, Música, a pasta de trabalho, volumes montados; lembra a última pasta | `FileDialog.cpp` | 1.0 |
| F-GER-012 | Partida | Tela de abertura com o progresso; pasta de trabalho; tema; recuperação; abrir o arquivo da linha de comando (ou o evento de abrir do macOS); senão o último projeto; senão um novo | `GuiApplication.cpp`, `main.cpp` | core (sem tela de abertura se for rápido; estados de carregamento, I-2) |
| F-GER-013 | Reiniciar pelo programa | Salva (ou pergunta), relança e reabre o projeto | `GuiApplication::restartAfterQuit` | 1.0 |
| F-GER-014 | Upgrade da configuração de versões antigas do LMMS | | `ConfigManager.cpp` | obsoleto — preferências novas |
| F-GER-015 | Ajuda sobre o que cada coisa faz | Só dicas; nenhum modo "o que é isto"; ajuda online do LMMS | — | aberto (P-34) |

## Resumo por área

A contagem usa o primeiro status de cada linha (uma linha "core (x) · 1.0 (y)" conta como core). "Novo" são duas
linhas só de registro (a paleta de comandos e a exportação rápida/exata), que não são do legado.

| Área | Funcionalidades | core | 1.0 | obsoleto | aberto |
|---|---|---|---|---|---|
| 1. Casca e janela do instrumento (F-JAN, F-IJN) | 55 | 29 | 19 | 3 | 3 (+1 novo) |
| 2. Edição (F-NOT, F-PAS, F-ARR, F-REG, F-PAD, F-AUT) | 67 | 35 | 22 | 7 | 3 |
| 3. Faixas (F-TRK) | 25 | 18 | 2 | 4 | 1 |
| 4. Clipes e itens (F-CLP) | 37 | 22 | 6 | 5 | 4 |
| 5. Gravação (F-REC) | 64 | 37 | 12 | 9 | 6 |
| 6. Mixer e mesa analógica (F-MIX) | 59 | 18 | 33 | 3 | 5 |
| 7. Instrumentos, efeitos, terceiros (F-INS, F-FX, F-HOST) | 66 | 9 | 49 | 4 | 4 |
| 8. Automação e controladores (F-CTL) | 9 | 4 | 5 | 0 | 0 |
| 9. Projeto e arquivos (F-PRJ) | 27 | 11 | 10 | 4 | 1 (+1 novo) |
| 10. Configurações (F-CFG) | 12 | 8 | 3 | 0 | 1 |
| 11. Navegador (F-BRW) | 12 | 6 | 6 | 0 | 0 |
| 12. Outros (F-GER) | 15 | 5 | 5 | 3 | 2 |
| **Total** | **448** | **202** | **172** | **42** | **30** |

Na seção 7, cada plugin conta como uma linha, mas carrega dezenas de parâmetros: são **23 instrumentos internos (mais o
instrumento fantasma), 3 hospedeiros de instrumento, 27 efeitos internos (2 já aposentados; o Guitarix conta como um,
com 97 unidades), a ferramenta Tap Tempo, os hospedeiros LADSPA, LV2 e VST2 e a infraestrutura comum** (cadeia,
navegador, categorias, conector de pinos, proteção contra quedas).

## Perguntas em aberto

O desenho novo (`arquitetura-nova.md`, `interfaces.md`, `decisoes-autonomas.md`) ainda não diz o que acontece com estas
funcionalidades do legado. Cada uma é citada pelo número nas tabelas.

| # | Pergunta | Funcionalidades |
|---|---|---|
| P-01 | **VST2**: o desenho lista LV2, CLAP e depois VST3. O legado hospeda VST2 (Windows pelo wine e Linux nativo), com presets `.fxp`/`.fxb` e janelas embutidas. Fica, sai, ou entra depois da 1.0? | F-HOST-001, F-FX-033, F-CFG-014, F-CFG-015 |
| P-02 | **Carla** (rack e patchbay): com hospedeiros próprios, ainda é preciso? | F-HOST-002 |
| P-03 | **LADSPA**: não é citado. O legado traz conjuntos embutidos (calf, caps, cmt, swh, tap) que muitos projetos usam | F-FX-031, F-CFG-015 |
| P-04 | **Ligar dois controles** (Ctrl+arrastar um botão sobre outro faz os dois andarem juntos): como fica no modelo de ids? | F-CTL-006 |
| P-05 | **ZynAddSubFX**: plugin interno (no processo) ou de terceiros (fora)? E a interface FLTK dele? | F-INS-023 |
| P-06 | **Escala linear/logarítmica por botão** (escolha do usuário no menu de cada botão): fica no esquema do parâmetro ou na interface? | F-CTL-005 |
| P-07 | **"Decay" / auto-quit dos efeitos** e a configuração "efeitos continuam rodando sem entrada"; a memória de tempo real configurável | F-FX-001, F-CFG-018 |
| P-08 | **Temas e traduções**: os dois temas do legado viram temas-dado? E os 35 idiomas além de português e inglês? | F-GER-007, F-GER-008, F-CFG-016 |
| P-09 | **Conteúdo de fábrica**: os ~1200 presets (962 do Zyn), os modelos e as 4 demos precisam ser refeitos no formato novo. Um conversor de `.xpf`/`.xiz`? Os `.xiz` do Zyn continuam válidos se o Zyn for o mesmo | F-BRW-012, F-IJN-003, F-PRJ-027 |
| P-10 | **Gravação além do básico**: desligar os plugins com latência na faixa armada; efeitos de entrada gravados; gravar a saída; entradas com mais de 2 canais | F-REC-010, F-REC-011 |
| P-11 | **Modo "gravador simples"** (takes em camadas desligado: o take novo substitui o que cobre): continua como opção? | F-REC-042, F-CFG-013 |
| P-12 | **Formato da gravação**: o novo grava RF64 float; escolher FLAC ou 24 bits? | F-REC-060 |
| P-13 | **Mídias geradas e não usadas** vão para a lixeira ao fechar (legado). O novo só diz que nada é apagado enquanto o histórico cita. Quem limpa `audio/`, e quando? | F-REC-063, F-JAN-010 |
| P-14 | **Contador de xruns** visível fora da gravação | F-REC-086 |
| P-15 | **Gravação MIDI**: CC, pitch bend, aftertouch, program change; um take por passada de loop; compensar a latência MIDI | F-REC-104 |
| P-16 | **ALSA raw MIDI** (além do ALSA seq) | F-REC-130 |
| P-17 | **Mesa analógica ou digital**: no legado a mesa inteira troca; no novo todo canal tem o rack, com seções que ligam. Há um "modo digital"? E a configuração para projetos novos? | F-MIX-001, F-CFG-010 |
| P-18 | **Agrupar canais na mesa**: sem canal de grupo automático da pasta, como recolher e organizar os canais? | F-MIX-011 |
| P-19 | **Retornos de efeito**: o legado tem FX 1 e FX 2 fixos e dois botões de envio em todo canal. No novo um retorno é um canal comum: os botões rápidos ficam? | F-MIX-023 |
| P-20 | **Onde os inserts entram em relação ao rack** (o legado tem 6 pontos de inserção) e se o width do canal some | F-MIX-032, F-MIX-034 |
| P-21 | **Tolerância por canal**: tirada do número (posição) do canal no legado; no novo, do número ou do id? (Mover o canal muda o som no legado.) | F-MIX-055 |
| P-22 | **Tipos de medidor** (VU, PPM, K) e balística escolhível | F-MIX-070 |
| P-23 | **Batidas com amostras**: no legado, faixa de amostra dentro do editor de padrões. No novo, um instrumento de bateria (AFP, DrumMachines, AvlDrums) tocado pela vista de passos basta? | F-PAD-004, F-TRK-003 |
| P-24 | **Curva de afinação por nota** (pitch bend desenhado em cada nota): existe no novo? | F-NOT-004 |
| P-25 | **Arquivo de clipe** (`.xpt`, exportar/importar as notas de um clipe) | F-NOT-020 |
| P-26 | **Loop e cursor próprios por editor** (a música, o padrão e o clipe têm cada um o seu): o novo tem um transporte só | F-REG-002 |
| P-27 | **Degrau num ponto de automação** (valor de entrada e de saída): dois pontos no mesmo tique? | F-AUT-003 |
| P-28 | **Botão FX do cabeçalho da faixa**: abre os inserts do canal da saída principal? | F-TRK-023 |
| P-29 | **Item de áudio vazio** criado com um clique na linha | F-TRK-033 |
| P-30 | **Curvas de fade e crossfade ajustáveis** (tensão nos clipes de amostra; planejado nos itens) | F-CLP-041, F-CLP-066 |
| P-31 | **Tocar um item de áudio ao contrário** (existe nos clipes de amostra) | F-CLP-042 |
| P-32 | **Ler do disco ou carregar na RAM** (automático acima de 20 s no legado); o `ResourceStore` do novo carrega tudo? | F-CLP-044, F-CLP-064 |
| P-33 | **Uma curva para vários controles** (um clipe de automação ligado a vários) | F-CLP-080 |
| P-34 | **Ajuda**: o legado só aponta para a documentação online do LMMS | F-JAN-024, F-GER-015 |
| P-35 | **Linha de comando e ambiente**: `--import`, `--geometry`, `--config`, recusar root, modo portátil, variáveis de ambiente | F-GER-002, F-GER-003, F-CFG-019 |
| P-36 | **"Descartar as conexões MIDI ao salvar"** (para levar um projeto a outra máquina) | F-PRJ-012 |
| P-37 | **Volume e transposição gerais do projeto** (salvos sem tela no legado) e "usar a transposição geral" por faixa | F-JAN-051, F-IJN-018 |
| P-38 | **Saída MIDI da faixa** para dispositivos externos (canal, programa, nota e velocidade fixos). A decisão de 19/11 tira só "MIDI de uma faixa tocando o instrumento de outra" | F-IJN-017 |
| P-39 | **Importar MIDI sem pan na faixa**: para onde vai o CC10? E o CC7, volume da faixa ou do canal? | F-PRJ-019 |
| P-40 | **Exportar stems**: por faixa (legado) ou por canal da mesa, ou os dois? | F-PRJ-024 |
