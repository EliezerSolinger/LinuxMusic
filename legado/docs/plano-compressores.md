# Plano: modelos de compressor e modos do gate (passo 23)

Base: `pesquisa-compressores.md`. Os valores vêm de lá, com a origem marcada (doc, sec, der, nosso). Este plano diz
como implementar, em que ordem e como verificar.

## O que o usuário decidiu

**Compressor:**
- Um alternador de modelo, como o das válvulas, com 9 modelos: SSL, API 2500, dbx 160, 1176, LA-2A, Fairchild 670,
  Gates Sta-Level, Altec 436 e RCA BA-6A.
- COMP e RATIO são fiéis a cada modelo. Onde o aparelho não tem ratio, o RATIO escolhe o que ele tem: COMP/LIMIT no
  LA-2A, a curva nos vari-mu.
- O botão FAST passa a ter três posições: FAST, NORM e SLOW. Elas escolhem o ataque dentro do que o modelo permite.
- No painel fica só o RELEASE. Nos modelos que têm uma chave de "time constant", ele é essa chave.
- Curvas de frequência dos aparelhos: replicar, tanto as do caminho do áudio quanto as do detector.

**Gate:**
- Os knobs GATE e RANGE continuam.
- Ganha um seletor de modo com os valores prontos do que não tem knob.
- O painel perde os controles do gate.

## 1. Modelo de dados (`AnalogChannelStrip`)

| Controle | Modelo | Salvo como | Antes |
|---|---|---|---|
| Modelo do compressor | `IntModel`, `CompressorModel`, padrão SSL | `compmodel` | — |
| COMP | `FloatModel`, posição de 0 (comprime menos) a 1, padrão 0 | `comppos` | `compthreshold` (dBu) |
| RATIO | `FloatModel`, posição de 0 (desligado) a 1, padrão 0 | `ratiopos` | `compratio` (1 a 20) |
| Velocidade | `IntModel`: FAST, NORM, SLOW; padrão NORM | `compspeed` | `compfast` |
| RELEASE (painel) | `FloatModel`, posição de 0 a 1 | `releasepos` | `comprelease` (s) |
| GATE | `FloatModel`, dBu de −30 a +10 (como está) | `gatethreshold` | — |
| RANGE | `FloatModel`, dB de 0 a 40 (como está) | `gaterange` | — |
| Modo do gate | `IntModel`, `GateMode`, padrão STD | `gatemode` | `gateexp`, `gatefast`, `gaterelease`, `gatehold` |

**Posições em vez de unidades:**
- **Por quê:** cada modelo tem uma faixa e uma unidade (dBu, peak reduction, ratio em passos, chave TC). O knob
  guarda a posição, e o modelo a traduz. Os números dos aparelhos ficam numa tabela só (`CompressorSpec`).
- **Ao trocar de modelo:** os knobs ficam onde estão e mudam de sentido, como trocar de aparelho no rack.
- **A dica de cada knob** mostra o valor na unidade do modelo: "−12 dBu", "PR 40", "4:1", "ALL", "LIMIT", "TC 3 (2 s)".

**Projetos de antes:**
- Viram SSL, com as posições equivalentes aos dBu, ratio e segundos salvos.
- `compfast` vira FAST ou NORM.
- Do gate: EXP vira o modo EXP; o resto vira STD.
- As opções peak, hard knee e lin. release da SSL somem (decisão do usuário: "knee, detector e curva de release
  passam a ser do modelo"). Um projeto que as usava soa um pouco diferente, e o doc registra isso.

## 2. Motor (`DeskDynamics`)

Um compressor só (`ChannelCompressor`), montado a partir de uma tabela de especificação por modelo. Não são nove
classes. As peças:

### 2.1 Detector (o que o compressor escuta)

| Tipo | Modelos | Implementação |
|---|---|---|
| RMS | SSL (τ 5 ms), API (10 ms), dbx 160 (36 ms, der) | Média de x² com um polo, em log |
| Retificador + RC | 1176, vari-mu | Onda completa, carga com R de ataque e descarga com R de release num capacitor |
| Célula óptica | LA-2A | Três estados (nosso, proposta do anexo C): rápido, lento e de memória, que carrega com o tempo e a força da exposição. Calibrado pelos 60 ms até 50% e pela soltura total do manual. Os números de vactrol LED dos artigos não são da T4 e não servem. A luz do painel depende da frequência (doc): um leve reforço de agudos na entrada da célula |

- **Filtro do detector:**
  - THRUST do API: tilt de +10 dB/década, um shelf de primeira ordem combinado (`Biquad`), com o LOUD documentado.
  - R37 do LA-2A: plano, como sai de fábrica (doc).
- **Estéreo:** linkado, pelo maior dos dois canais, como hoje.

### 2.2 Computador de ganho e topologia

- **Feed-forward** (SSL, dbx, API NEW): curva estática com joelho, pela fórmula de Giannoulis, Massberg e Reiss.
- **Feedback** (API OLD, 1176, LA-2A, vari-mu):
  - o detector lê a saída;
  - em vez de usar a saída da amostra anterior (o atraso de uma amostra deixa passar transientes, segundo
    Raffensperger), o laço é **resolvido em cada amostra**: ganho = f(entrada × ganho), por Newton em log, com 2 a
    4 iterações;
  - assim, ratio e joelho nascem do laço, como no aparelho.
- **Vari-mu:** a tensão de controle (CV) do retificador vira redução de ganho pela lei do tubo:
  - Gates: GR ≈ 16,6·ln(1 + |Vc|/8,1) (der da tabela do manual);
  - Fairchild: o 6386 de Raffensperger, simplificado para a curva estática de ganho × bias;
  - Altec e RCA: a mesma forma de lei, calibrada pelos pontos documentados (Altec: 20 dB com −38 V na grade; RCA:
    ~10:1).
- **RATIO por modelo:**
  - SSL e dbx: ratio contínuo;
  - API e 1176: passos (1176: 4, 8, 12, 20 e ALL, com os thresholds fixos documentados por ratio);
  - LA-2A: COMP/LIMIT;
  - Fairchild: DC threshold (a forma da curva);
  - Gates: o resistor de ratio, de 1,6 a 6:1;
  - Altec: o threshold acoplado, de 2:1 a 4:1;
  - RCA: o bias, até ~10:1.
- **API em OLD:** o ∞ vira k = 50 (≈ 51:1), porque o feedback não alcança o ∞ (nosso). O joelho é MED de 6 dB,
  fixo (nosso).
- **Make-up automático:** calculado no `begin()`. Resolve o estado estável para um sinal no nível nominal, com o
  laço quando é feedback, e devolve a redução que ele sofre.

### 2.3 Balística

| Modelo | Ataque (FAST / NORM / SLOW) | Release |
|---|---|---|
| SSL | Inclinação de 20 dB em 3 / 30 / 60 ms | Parte rápida fixa de 40 ms (nosso) mais a lenta de 0,1 a 4 s, logarítmica |
| API | Exponencial: 0,3 / 3 / 30 ms | Exponencial de 0,05 a 3 s |
| dbx 160 | Sai do RMS: τ de 18 / 36 / 72 ms; o NORM reproduz o manual | Sai do RMS: ~120 dB/s no NORM, ~240 no FAST, ~60 no SLOW; RELEASE inativo |
| 1176 | R de ataque: 20 / 200 / 800 µs; alvo medido: ~1 ms de overshoot e 5 ms até 63% (Moore, A=3, R=7) | R de release: 50 ms a 1,1 s |
| LA-2A | Célula: 5 / 10 / 20 ms | Duas fases: 50% em ~60 ms, mais a fase lenta com memória. O RELEASE escala a fase lenta (nosso); a posição padrão dá a soltura do manual |
| Fairchild | NORM: o ataque da posição TC, como no aparelho (0,2 ms em 1, 2 e 6; 0,4 ms em 3 a 5). FAST 0,2 e SLOW 0,8 ms | Rede RC da chave TC 1 a 6, com os ramos em série das posições 5 e 6 (valores de Raffensperger). A lei de ganho precisa dar o release documentado, ~2,3 a 2,9 × R·C |
| Gates | Como a chave do aparelho, que acopla ataque e release: FAST = Double (25 ms), NORM = Single (75 ms), SLOW = Single com 150 ms (nosso) | Double: rede de dois capacitores, dependente do programa. Single: um capacitor, 7,05 s até 63%. O RELEASE é o resistor interno (Double 0,75 a 3,75 s, Single 2,25 a 11,25 s); a posição padrão dá o de fábrica |
| Altec | 25 / 50 / 100 ms | 0,3 a 1,3 s |
| RCA | Single (0,6 ms) / Dual (0,6 ms mais o caminho lento de 0,9 s) / só o lento | Rápido 0,33 s (RELEASE escala) e lento 2 s |

Nos modelos com rede RC, o release decai na tensão de controle, que passa pela lei de ganho. A forma da soltura e a
dependência do programa saem sozinhas.

### 2.4 Cor e resposta em frequência

- **Distorção do elemento de ganho:** um polinômio de 2ª e 3ª ordem sobre o sinal no elemento, com o 2º sem DC. A
  intensidade depende da redução de ganho e do nível. Calibrado pelos números do doc:
  - 1176: 0,07% a 1 kHz com ~8 dB de redução;
  - Fairchild: < 1% com 10 dB de redução a +12 dBm, crescendo forte com a redução;
  - Altec: 1,5% com 25 dB;
  - Gates: ≤ 1%;
  - RCA: < 1% com 12 dB;
  - LA-2A: < 0,5%.
- **Harmônicos por topologia:** push-pull dá o 3º, single-ended dá o 2º (nosso, porque não há espectro medido).
  No 1176, o 2º e o 3º são fortes, e o 2º cresce com a redução (assimetria do FET, doc). A ordem medida por Moore
  vale: o 1176 tem o 3º bem mais forte que o LA-2A e o Fairchild.
- **SSL e API** ganham distorção mínima: a SSL abaixo dos 0,05% a +20 dBu do X-Rack; o API um pouco acima, pelos
  estágios discretos (nosso).
- **O que aparece sozinho:** a distorção do dbx (3º a 0,5% em ∞, caindo uma oitava por oitava) e a do 1176 no
  grave (0,40% a 125 Hz) vêm da ondulação do detector modulando o ganho. Aparecem no modelo e só precisam ser
  conferidas.
- **Oversampling de 4x** (o hiir do `GainStage`) nos modelos com distorção: 1176, LA-2A e os vari-mu.
- **Caminho do áudio**, ajustado por aparelho para cumprir cada especificação, que é a tolerância dentro da faixa
  (nosso):
  - LA-2A (±0,1 dB de 30 Hz a 15 kHz): sem filtro audível;
  - Fairchild: ±1 dB de 40 Hz a 15 kHz;
  - Gates: ±1 dB de 30 Hz a 15 kHz;
  - Altec: ±1,5 dB de 30 Hz a 15 kHz;
  - 1176: de 20 Hz a 20 kHz;
  - RCA: filtro dinâmico que segue a redução de ganho (+1/−2 dB até 20 dB).
- **Transformadores:** com o modelo do XFMR (`OutputTransformer`), parâmetros por aparelho, nos valvulados. No
  1176, o de saída fica dentro do laço; o de entrada é opcional (nosso).
- **Os VCAs (SSL, API, dbx)** ficam planos, como os aparelhos.

### 2.5 Custo

| Modelo | Custo por canal estéreo (estimativa) |
|---|---|
| SSL, API, dbx (feed-forward, sem oversampling) | Como hoje (< 0,5% de um núcleo) |
| Modelos com laço, oversampling e transformador | Até ~2% de um núcleo |

Se algum passar disso, entra no plano de otimização. Um teste mede o custo.

## 3. Interface

- **`ChoiceButton`:** o `DriveButton` generalizado. Alterna entre opções com nome, descrição e "tipo" para a cor, e
  o botão direito abre a lista. Vale para o modelo do compressor, a velocidade e o modo do gate. O `DriveButton`
  passa a usá-lo.
- **Cores do modelo:**
  - VCA (SSL, API, dbx): azul, como os amp-ops;
  - FET (1176): verde;
  - óptico (LA-2A): âmbar claro;
  - vari-mu: âmbar, como as válvulas.
- **Seção DYN:**
  ```
  COMP   RATIO
  [  SSL  ]          modelo
  [ NORM  ]          FAST / NORM / SLOW
  GATE   RANGE
  [  STD  ]          modo do gate
  ```
- **Knobs:** COMP, RATIO e RELEASE mostram o valor na unidade do modelo, através de uma subclasse do `DeskKnob`.
- **Dicas:** cada knob diz o que faz no modelo atual, por exemplo "COMP: Peak Reduction (LA-2A)".
- **Painel do canal:**
  - compressor: só o RELEASE, desativado no dbx, mostrando a posição da chave TC no Fairchild;
  - gate: some.

## 4. Testes

**Alvos dos documentos, um teste por modelo:**

| Modelo | O que conferir |
|---|---|
| SSL | Ataque de 20 dB em ~30 ms (NORM) e ~3 ms (FAST); ratio; make-up mantém o nominal |
| API | Passos de ratio; THRUST reduz a reação ao grave; OLD em feedback (ratio efetivo) |
| dbx 160 | Ataques de ~15, 5 e 3 ms a 10, 20 e 30 dB acima; release de ~120 dB/s; 3º harmônico ~0,5% em ∞ |
| 1176 | Thresholds por ratio; ataque e release dentro das faixas; THD ~0,07% a 1 kHz e maior a 125 Hz com 8 dB de redução; ALL mais distorcido |
| LA-2A | 50% da soltura em ~60 ms; soltura mais lenta depois de compressão longa (memória); LIMIT mais plano que COMP; THD < 0,5% |
| Fairchild | Release por posição TC, a partir de 10 dB de redução, perto da tabela; ratio crescendo com o nível; THD < 1% a 10 dB |
| Gates | Ratio ~3,3:1; recuperação de 63% e 90% na proporção do manual (~4×) |
| Altec | Ratio de 2:1 a 4:1 conforme o RATIO; THD < 1,5% a 25 dB |
| RCA | ~10:1; Dual solta rápido depois de pico isolado e lento depois de picos sustentados |

**Para todos:**
- RATIO no mínimo desliga, bit a bit;
- trocar de modelo não estala;
- silêncio continua silêncio;
- sinal extremo continua finito;
- custo por modelo;
- conversão de projetos de antes.

**Gate:** cada modo com seus tempos; os filtros de chave (o KICK ignora o chimbal); a histerese do modo substitui
a curva do SSL, menos em STD e EXP; projetos de antes.

## 5. Ordem

1. Tabela `CompressorSpec`, detectores e balística, com SSL, API e dbx (feed-forward). Testes.
2. Laço feedback resolvido por amostra, com 1176 e API OLD. Testes.
3. Célula óptica, com o LA-2A. Testes.
4. Vari-mu e redes RC, com Fairchild, Gates, Altec e RCA. Testes.
5. Cor (distorção e oversampling) e caminho do áudio (filtros e transformadores). Calibração pelos alvos do doc.
6. Modos do gate.
7. Interface: `ChoiceButton`, knobs com unidade, DYN e painel.
8. Conversão de projetos, documentação (passo 23 em `preparacao-mixer.md`), bateria A/B e commits por etapa.

Cada etapa termina com testes passando e um commit.
