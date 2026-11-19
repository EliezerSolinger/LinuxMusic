# Funcionalidades do legado (LinuxMusic, fork do LMMS)

Estado: **em escrita** (19/11). Este documento é o inventário de tudo o que o programa antigo (`legado/`) faz. Ele
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
| F-JAN | 1. Janelas, páginas e painéis (a casca do programa) |
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

<!-- As seções 1 a 6 entram acima da 7 conforme o levantamento termina. -->

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

#### F-INS-008 Lb302 (imitação monofônica da TB-303) — 1.0

- **Onde:** `plugins/Lb302/`.
- **Parâmetros:** Corte do VCF 0–1,5, 0,75 · Ressonância 0–1,25, 0,75 · Env Mod 0–1, 0,1 · Decay 0–1, 0,1 (tempo =
  2,3·v + 0,2 s) · Distorção 0–1, 0 · Forma de onda (12): serra, triângulo, quadrada, quadrada arredondada, Moog,
  senoide, exponencial, ruído branco, serra BL (padrão), quadrada BL, triângulo BL, Moog BL · Slide Decay 0–1, 0,6 ·
  Slide, desligado · Dead (nota não redispara o envelope se ainda soa), desligado · Filtro 24 dB/oitava (3 polos),
  desligado. "Accent" existe no código, sem efeito, sem tela, não salvo.
- **Comportamento:** **monofônico**; quando várias notas começam juntas, **a mais grave vence**; com Slide, a próxima
  nota desliza a partir desta. Segue pitch bend. Quirk: o botão "exponencial" diz "White noise wave".

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

<!-- 7.1 continua com os instrumentos de Patman a ZynAddSubFx, e os hospedeiros (Vestige, Carla, LV2). -->

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

#### F-FX-008 Bitcrush — 1.0

- **Onde:** `plugins/Bitcrush/`.
- **Parâmetros:** Ganho de entrada −20–20 dBFS, 0 · Ruído de entrada 0–100 %, 0 · Ganho de saída −20–20, 0 · Clip de
  saída −20–20, 0 · Taxa 20–44100 Hz, 44100 · Diferença estéreo −50–50 %, 0 · Níveis 1–256, 256 · Taxa ligada
  (ligado) · Profundidade ligada (ligado).
- **Comportamento:** sobreamostragem ×5 com filtro anti-alias.

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

#### F-FX-010 CrossoverEQ — 1.0

- **Onde:** `plugins/CrossoverEQ/`.
- **Parâmetros:** Cruzamento 1/2 50–10000 Hz, 125 · 2/3 50–20000, 1250 · 3/4 50–20000, 5000 (os cruzamentos se
  mantêm em ordem) · Ganho das bandas 1–4 −60–30 dB, 0 · Banda 1–4 ativa (ligado). Filtros Linkwitz-Riley.

#### F-FX-011 Delay — core (plugin de validação, ou o ReverbSC)

- **Onde:** `plugins/Delay/`.
- **Parâmetros:** Tempo 0,01–5 s, 0,5, sync · Realimentação 0–1, 0 · Taxa do LFO (período) 0,01–5 s, 2, sync ·
  Quantidade do LFO 0–0,5 s, 0, sync · Ganho de saída −60–20 dB, 0 (com medidor L/R).
- **O usuário pode:** ajustar tempo e realimentação juntos num pad XY.

#### F-FX-012 Dispersion (all-pass de ordem alta) — 1.0

- **Onde:** `plugins/Dispersion/`. **Parâmetros:** Quantidade de all-pass 0–999, 0 · Freq 20–20000 Hz, 200, log ·
  Reso 0,01–8 oitavas, 0,707 · Realimentação −1–1, 0 · Remover DC, desligado.

#### F-FX-013 DualFilter — 1.0

- **Onde:** `plugins/DualFilter/`.
- **Parâmetros:** para cada um dos 2 filtros: liga (ligado) · Tipo (22: passa-baixa, passa-alta, passa-banda csg,
  passa-banda czpg, notch, all-pass, Moog, 2× passa-baixa, RC passa-baixa/banda/alta 12 dB, RC passa-baixa/banda/alta
  24 dB, formante vocal, 2× Moog, SV passa-baixa/banda/alta/notch, formante rápido, Tripole) · Corte 1–20000 Hz,
  7000 · Ressonância mín–10, 0,5 · Ganho 0–200 %, 100. Mix −1 (só o 1) a +1 (só o 2), 0.
- Os mesmos 22 tipos são os do filtro da janela do instrumento (F-INS, seção 1).

#### F-FX-014 DynamicsProcessor (curva de dinâmica desenhada) — 1.0

- **Onde:** `plugins/DynamicsProcessor/`.
- **Parâmetros:** Entrada 0–5, 1 · Saída 0–5, 1 · Ataque 1–500 ms, 10 · Release 1–500 ms, 100 · Estéreo: máximo
  (padrão) / média / desligado · Curva de transferência (200 pontos, 0–1), padrão linear.
- **O usuário pode:** desenhar a curva entrada → saída com o mouse; voltar à linear; suavizar; subir ou descer a curva
  inteira 1 dB.

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

#### F-FX-016 Flanger — 1.0

- **Onde:** `plugins/Flanger/`.
- **Parâmetros:** Atraso 0,0001–0,05 s, 0,001 · Taxa (período) 0,01–60 s, 0,25, sync · Quantidade 0–0,0025, 0 · Fase
  estéreo 0–360°, 90 · Realimentação −1–1, 0 · Ruído 0–0,05, 0 · Inverter realimentação, desligado.
- **Comportamento:** o LFO reinicia quando a reprodução começa ou para.

#### F-FX-017 FrequencyShifter (deslocador de frequência e phaser "barberpole") — 1.0

- **Onde:** `plugins/FrequencyShifter/`.
- **Parâmetros:** Deslocamento −20000–20000 Hz, 0 (visor numérico) · Mix 0–1, 1 · Spread −50–50 Hz, 0 · Fase −2–2 ·
  Ring 0–1 · Harmônicos 0–1 · Tom 100–22000 Hz, 22000 · Glide 0–1, 0,05 · LFO 0–2000 Hz, 0 · Taxa do LFO 0–200 Hz,
  0,2 · Fase estéreo do LFO 0–1 · Atraso 0–2000 ms e fino 0–20 ms · Realimentação 0–1 · Amortecimento do atraso
  100–22000 Hz, 22000 · Glide do atraso 0–1, 0,05 · Antirreflexo · Roteamento Send / Pass / Mute · Botões
  automatizáveis "zerar o deslocador" e "zerar o LFO".
- **O usuário pode:** abrir um texto de ajuda que explica cada controle.

#### F-FX-018 GranularPitchShifter — 1.0

- **Onde:** `plugins/GranularPitchShifter/`.
- **Parâmetros:** Altura −48–24 semitons, 1 · Espalhamento estéreo da altura −24–24 · Tamanho do grão 2–1000 Hz, 10 ·
  Spray 0–0,5 s, 0,005 · Jitter 0–1 oitava · Twitch 0–1 oitava · Espalhamento estéreo do spray 0–1 · Forma do grão
  1–2, 2 · Fade 0,001–1, 1 · Realimentação 0–1 · Latência mínima 0–1 s, 0,01 · Pré-filtro, ligado · Densidade 1–16,
  1 · Glide 0–1 s, 0,01 · Tamanho do buffer: 5 s / 10 s / 40 s / 40 s / 120 s (limita as faixas dos botões; trocar
  realoca).
- **O usuário pode:** abrir um texto de ajuda.
- **Quirk:** atraso variável não informado à compensação.

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

<!-- 7.2 continua com os efeitos de LOMM a WaveShaper; 7.3 com os hospedeiros de terceiros. -->
