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

#### F-INS-013 PatMan (patches GUS .pat) — 1.0

- **O que faz:** toca patches `.pat` do Gravis Ultrasound (multiamostra, um instrumento, uma camada).
- **Onde:** `plugins/Patman/`.
- **Parâmetros:** Loop (usa os pontos de loop do patch), ligado · Tune (afina a amostra pela nota; desligado toca na
  altura original), ligado · arquivo.
- **O usuário pode:** abrir um `.pat` (começa em `/usr/share/midi/freepats` se existir); soltar um `.pat` do
  navegador; abrir um `.pat` em qualquer lugar escolhe o PatMan.
- **Comportamento:** escolhe a amostra cuja nota raiz está mais perto da nota (ignora faixas de tecla, envelopes,
  pan, tremolo do patch); mono duplicado; release da faixa. Erros claros (não é GUS, mais de uma camada, truncado).

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

#### F-INS-021 Watsyn (4 osciladores de onda desenhada) — 1.0

- **O que faz:** dois pares A (A1←A2) e B (B1←B2) misturados por um crossfader A-B com envelope próprio.
- **Onde:** `plugins/Watsyn/`.
- **Por oscilador (A1, A2, B1, B2):** Volume 0–200 %, 32 · Pan −100–100 · Multiplicador de frequência 1–24 (/8), 8 ·
  Desafinação esquerda e direita −600–600 cents · Onda desenhada (220 pontos).
- **Global:** Mistura A-B −100–100, 0 · Quantidade do envelope −200–200 · Ataque, Hold, Decay do envelope 0–2000 ms,
  sync · Crosstalk A1→B2 0–100 · Modulação A2→A1 e B2→B1: Mistura, AM, Ring, PM · Anti-aliasing (ligado).
- **O usuário pode:** escolher qual onda editar e desenhá-la; carregar de arquivo; deslocar a fase ±15°; normalizar;
  inverter; suavizar; formas prontas.

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

#### F-FX-021 MultitapEcho — 1.0

- **Onde:** `plugins/MultitapEcho/`.
- **Parâmetros:** Passos (ecos) 4–32, 16 · Duração do passo 1–500 ms, 100, sync · Ganho seco no buffer −80–20 dB, 0 ·
  Trocar entradas (pingue-pongue), desligado · Estágios do passa-baixa 1–4, 1 · Gráfico de amplitude por eco (−60–0
  dB, desenhado) · Gráfico de corte por eco (20 Hz–20 kHz, desenhado).
- **O usuário pode:** desenhar os dois gráficos em barras. Quirk: só os ecos 0–19 têm o filtro certo ao abrir.

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

#### F-FX-028 StereoMatrix — obsoleto

- **O que fazia:** matriz 2×2 (L→L 1, L→R 0, R→L 0, R→R 1, cada um −1–1): trocar lados, somar em mono, inverter
  polaridade. Já aposentado no fork.
- **Status:** obsoleto — substituído pela fase, troca e mono do canal da mesa (F-MIX).

#### F-FX-029 Vectorscope — 1.0

- **Onde:** `plugins/Vectorscope/`. Só mostra.
- **Parâmetros:** Escala log, desligado · Linhas (ou pontos), ligado.
- **O usuário pode:** zoom 20–1000 % na roda; escolher a cor do traço com duplo clique.

#### F-FX-030 WaveShaper (curva de transferência desenhada) — 1.0

- **Onde:** `plugins/WaveShaper/`.
- **Parâmetros:** Entrada 0–5, 1 · Saída 0–5, 1 · Cortar a entrada em 0 dB, desligado · Curva (200 pontos, 0–1),
  linear.
- **O usuário pode:** desenhar a curva (Shift+clique traça reta); voltar à linear; suavizar; +1 dB / −1 dB.

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
