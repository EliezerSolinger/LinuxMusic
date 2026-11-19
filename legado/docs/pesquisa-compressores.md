# Compressores e gate da faixa analógica: pesquisa

Pesquisa feita antes de implementar os modelos de compressor e os modos do gate da faixa analógica (passo 23), a
pedido do usuário: "faça uma investigação bem profunda sobre cada um deles, salve todos os detalhes num md, melhore o
conteúdo, faça um plano de implementação, só aí implemente".

O documento tem três partes:
1. **Síntese:** o que cada modelo é e como os controles da faixa (COMP, RATIO, FAST/NORM/SLOW e o RELEASE do
   painel) se ligam ao aparelho real. É a base do plano (`plano-compressores.md`).
2. **Modos do gate.**
3. **Dossiês completos** por aparelho (anexos A a E), com tabelas de controles, especificações, literatura de
   modelagem e fontes.

A síntese passou por uma revisão independente contra os anexos, e as 32 correções foram aplicadas: atribuições,
marcas de origem, contradições e omissões.

## Como ler os valores

Cada dossiê marca a origem de cada número. As marcas variam um pouco de dossiê para dossiê; aqui elas se reduzem a:

| Marca nesta síntese | Significa | Marcas nos dossiês |
|---|---|---|
| **doc** | Especificação ou medição de fonte primária: manual original, folha de dados, artigo com medição | [SPEC], [DOC], [D], [FAB], [PAPER], [MEDIDO], [V] |
| **sec** | Fonte secundária confiável: fabricante atual, Sound On Sound, review, reprodução de documento | [SEC], [REV], [REVIEW], [DOC-UA], [DOC-2] |
| **der** | Calculado ou inferido a partir de dados documentados | [DERIVADO], [DER], [CALC] |
| **nosso** | Escolha nossa, onde não há dado: usar, mas poder revisar | [EST], [ESTIMATIVA], [PROPOSTA], [J], [A] |
| **lenda** | Fórum ou loja, sem medição | [LORE] |
| **?** | Desconhecido, ou citado de memória sem conferência | [?], DESCONHECIDO, [M] |

**Níveis:**
- 0 VU = +4 dBu = −18 dBFS RMS: o alinhamento da mesa (`DbuToDbfs`).
- Os dBm antigos (600 Ω) tratamos como dBu.
- Os manuais do 1176 e do dbx 160 usam 0 dB = 0,775 V, que é o dBu.
- Onde a referência não é dada, como no threshold da SSL, a leitura em dBu é **nossa**.

---

## Parte 1: síntese por modelo

### Visão geral

| Modelo | Origem | Topologia | Elemento de ganho | Detector | Caráter |
|---|---|---|---|---|---|
| SSL (canal E) | Console 4000 E, 1979 | Feed-forward (doc) | VCA classe A | RMS verdadeiro (doc) | Limpo, preciso |
| API 2500 | API | Feedback (OLD) ou feed-forward (NEW) (doc) | VCAs; THAT 2180 (sec; o manual diz "three VCAs") | RMS, depois do filtro THRUST (doc) | Punch, grave preservado |
| dbx 160 | dbx, 1976 | Feed-forward (doc) | VCA Blackmer | RMS verdadeiro, τ ≈ 36 ms (der) | Duro, ataque e release automáticos |
| 1176 (rev. D/E, ~1973) | UREI; rev. A em 1967 | Feedback (doc) | JFET como resistor variável | Retificador de onda completa + RC (pico/média) (doc) | Ataque ultrarrápido, agressivo |
| LA-2A | Teletronix, anos 60 | Feedback (sec, UA) | Célula óptica T4: painel EL + LDR de CdS | Sem retificador: a célula integra; a luz depende da tensão **e da frequência** (doc) | Suave, lento, release em dois tempos |
| Fairchild 670 | Fairchild, anos 50 (manual de 1959) | Feedback (doc) | Vari-mu, 4× 6386 push-pull por canal | Retificador + rede RC por "time constant" (doc) | Denso, ratio cresce com o nível |
| Gates Sta-Level | Gates, 1956 | Feedback (doc) | Vari-mu, 6386 push-pull | 6AL5 retificando a saída (doc) | Lento, denso, release muito longo |
| Altec 436C | Altec, anos 60 | Feedback (doc) | Vari-mu, 6BC8 push-pull | 6AL5 (doc) | Colorido, o que mais distorce |
| RCA BA-6A | RCA, 1950 | Feedback (doc) | Vari-mu, 2× 6SK7 pentodos push-pull | 6H6 (doc) | Agressivo, ~10:1 |

**Topologia:**
- Seis modelos são sempre de **realimentação** (feedback): 1176, LA-2A e os quatro vari-mu. O API também é, no modo
  OLD.
- No feedback, o detector lê a saída. Nos vari-mu, isso faz o ratio e o joelho nascerem do laço, e não de uma curva
  fixa (doc, manuais Gates e Altec: subir o threshold sobe o ratio).
- **API em OLD:** a detecção acontece depois do make-up, então subir o ganho comprime mais (doc). Ali o ratio
  efetivo é 1 + k (der), e o ∞ não é alcançável.

**Saída:**
- No hardware, a SSL tem make-up automático (doc) e o API tem automático opcional (doc).
- dbx 160 e 1176 têm knob de saída.
- O Fairchild 670 não tem controle de saída: ganho fixo de 7 dB (doc). O Altec 436C também não tem (doc). O Gates
  tem só uma seção variável de 6 dB no pad de saída (doc).
- Na faixa, o make-up é automático em todos, como já decidido para a SSL: um sinal no nível nominal sai onde entrou
  (nosso).

### Como os controles da faixa se ligam a cada modelo

Decisões do usuário:
- COMP e RATIO são fiéis a cada modelo;
- FAST, NORM e SLOW escolhem o ataque "dentro do que o modelo permite";
- no painel fica só o RELEASE.

Onde o aparelho não tem o controle, a ligação é **nossa**, e a posição padrão reproduz o comportamento documentado.

| Modelo | COMP (faixa) | RATIO (faixa; o mínimo desliga) | FAST / NORM / SLOW (ataque) | RELEASE (painel) |
|---|---|---|---|---|
| SSL | Threshold +4 a −26 (doc; leitura em dBu nossa: a Waves dá +10 a −20, exatamente 6 dB acima) | 1:1 a ∞, contínuo (doc) | 3 / 30 ms para 20 dB de redução (doc); SLOW 60 ms (nosso) | 0,1 a 4 s, logarítmico (doc), mais uma parte rápida fixa de valor ? (sec UA) → 40 ms (nosso) |
| API 2500 | Threshold +10 a −20 dBu (doc, original) | 1,5 / 2 / 3 / 4 / 6 / 10 / ∞ em passos (doc); em OLD, o ∞ vira k = 50 (nosso) | 0,3 / 3 / 30 ms, três dos sete passos (doc); a definição exata do tempo é ? | 0,05 a 3 s, contínuo (doc) |
| dbx 160 | Threshold −38 a +12 dBu (doc) | 1:1 a ∞ (≈120:1), contínuo (doc) | NORM: τ RMS de 36 ms (der), que dá os ataques e os ~120 dB/s do manual. FAST 18 ms e SLOW 72 ms (nosso) mudam também a soltura, para ~240 e ~60 dB/s | Não existe; knob inativo |
| 1176 | Input: empurra o sinal para o threshold fixo de cada ratio (doc) | 4 / 8 / 12 / 20 / ALL em passos (doc) | 20 / 200 / 800 µs nos R de ataque (doc: faixa de 20 a 800 µs). Medido (Moore, A=3, R=7): ~1 ms de overshoot e 5 ms até 63% | 50 ms a 1,1 s (doc); a lei do knob é ? |
| LA-2A | Peak Reduction: ganho do sidechain (sec, UA) | COMP (~4:1, joelho suave) / LIMIT (~∞) (sec, UA; o manual de 1966 não tem a chave) | 5 / 10 / 20 ms em torno dos 10 ms médios (sec, UA; o manual de 1966 diz "essencialmente instantâneo"); o resto é nosso | Não existe no aparelho. Nosso: escala a fase lenta. A posição padrão dá o documentado: 50% em ~60 ms, total de 0,5 a 5 s ou 1 a 15 s conforme a redução anterior (doc) |
| Fairchild 670 | Input em degraus de 1 dB, ganho unitário com ~10 dB de atenuação (doc), + AC threshold | DC threshold: a forma da curva, de ~2:1 a ~30:1 (doc) | NORM: o ataque da posição TC, como no aparelho, 0,2 ms em 1, 2 e 6 e 0,4 ms em 3 a 5 (doc). FAST 0,2 e SLOW 0,8 ms (nosso; 0,8 ms na pos. 4 é sec, boletim reproduzido pela UA, e conflita com o manual) | Chave TC de 1 a 6: 0,3 / 0,8 / 2 / 5 s / auto 2 e 10 s / auto 0,3, 10 e 25 s, medidos a partir de 10 dB de redução (doc) |
| Gates Sta-Level | Input; saída fixa em +10 a +12 dBm (doc) | Resistor de ratio: 1,6 a 6:1, padrão 3,3:1 (doc, ajuste interno) | Uma chave só acopla ataque e release (doc): FAST = Double, 25 ms com release dependente do programa; NORM = Single, 75 ms com release fixo de 7,05 s até 63%; SLOW = Single com 150 ms (nosso) | Nosso: o resistor interno de release. Double: 0,75 a 3,75 s, de fábrica 2,35 s (rápido) e 3,1 s (lento). Single: 2,25 a 11,25 s, de fábrica 7,05 s (doc) |
| Altec 436C | Input/Gain (doc) | Threshold de 0 a +16 dBm, que leva o ratio de 2:1 a 4:1 (doc) | NORM 50 ms, fixo no aparelho (doc); FAST 25 e SLOW 100 ms (nosso) | 0,3 a 1,3 s (doc) |
| RCA BA-6A | Input; limita a partir de +29,5 dBm na saída (doc) | Bias da curva: até ~10:1 no máximo (doc); o resto nosso | Single 0,6 ms / Dual, 0,6 ms + caminho lento de 0,9 s / só o caminho lento (doc os tempos; o arranjo é nosso) | Não existe. Nosso: escala o caminho rápido (0,33 s de fábrica, doc); o lento fica em 2 s (doc) |

**Notas:**
- **COMP é sempre "quanto comprime":** girar para a direita comprime mais. Nos aparelhos com threshold fixo (1176,
  vari-mu, LA-2A), isso equivale a empurrar a entrada, e o modelo trata como threshold equivalente. A cor que
  depende do quanto se empurra continua valendo.
- **Ratios em passos:** no API e no 1176, o knob RATIO anda em degraus. A dica mostra o passo: "4:1", "ALL".
- **dbx 160:** os três ataques do manual (15, 5 e 3 ms para 10, 20 e 30 dB acima) e a soltura de 120 dB/s saem de
  **um único** detector RMS com τ ≈ 36 ms (der: o modelo prevê 16,4, 7,0 e 2,9 ms). A soltura vale 4,343/τ dB/s,
  então FAST e SLOW a mudam também.
- **Thresholds do 1176** por ratio: ≈ −30, −26, −25 e −24 dB para 4, 8, 12 e 20 (doc, com ressalvas):
  - são ±2 dB;
  - vêm de um manual UREI tardio (rev. G em diante, entrada com CI), não do D/E;
  - a célula de 4:1 está ilegível no OCR;
  - a Sound On Sound dá ≈ −32 para 4:1.
- **Fairchild:** ataque e release vêm juntos na chave TC. Separar o ataque (FAST/SLOW) é **nosso**; o NORM
  reproduz o aparelho.

### Curva estática

| Modelo | Joelho | Comportamento |
|---|---|---|
| SSL | Suave; largura ? → 10 dB (nosso). O console tem opção de joelho duro e LIN REL, que sobe o threshold 6 dB (doc); a faixa não oferece (decisão do usuário) | Ratio do knob acima do joelho |
| API 2500 | Hard / Med / Soft (doc); larguras ? → 0, ~6 e ~12 dB (nosso). A faixa usa MED fixo (nosso) | Em OLD, ratio efetivo 1 + k (der) |
| dbx 160 | Duro (doc) | Ratio do knob |
| 1176 | ? (sec: "suave", mais duro nos ratios altos) → de 6 dB em 4:1 a 2 dB em 20:1 (nosso) | ALL (doc, manual UA): ratio efetivo entre 12 e 20:1, curva mais severa ("platô"), ataque atrasado nos transientes, distorção que "aumenta radicalmente". A inclinação negativa é lenda: não modelar |
| LA-2A | Muito largo: começa a −30 dB de entrada e fica horizontal a partir de −20 dB; menos de 1 dB a mais de saída para +20 dB de entrada (doc, curva de 1966, sem chave; o ponto de quebra depende do Peak Reduction) | COMP ~4:1 suave, LIMIT ~∞ (sec) |
| Fairchild 670 | Contínuo: de 1:1 a 20:1 acima de +2 dBm (doc, ajuste de fábrica) | O ratio cresce com o nível |
| Gates | Do laço: ~3,3:1 (doc) | Tabela de bias × redução de ganho (doc): −2,4 V dá 5 dB, −19 V dá 20 dB, −76 V dá 40 dB; ajuste der: GR ≈ 16,6·ln(1 + \|Vc\|/8,1) |
| Altec 436C | Do laço | 2:1 com threshold em 0 dBm, 4:1 com +16 dBm (doc) |
| RCA BA-6A | Duro (der, leitura do gráfico de 1950) | 20 dB a mais na entrada dão ~1,9 dB a mais na saída (doc), ~10:1 |

### Release dependente do programa

| Modelo | Mecanismo |
|---|---|
| SSL | Uma parte rápida fixa, de valor ?, mais a lenta do knob (sec, UA) |
| dbx 160 | Sai do detector RMS (der) |
| LA-2A | Duas fases: 50% em ~60 ms, depois de 0,5 a 5 s ou 1 a 15 s (o manual dá os dois). Memória: quanto mais tempo e mais forte a célula foi iluminada, mais lenta a soltura (doc e sec). Os números de vactrol LED dos artigos (subida ~12 ms, queda ~250 ms) **não são da T4**; o modelo precisa de um estado lento e de um estado de memória (nosso, proposta do anexo C) |
| Fairchild 5 e 6 | Ramos RC em série (100k + 8 µF e 100k + 20 µF, doc via Raffensperger): picos isolados soltam rápido, picos repetidos lento. O release documentado vale ~2,3 a 2,9 × R·C (der): a lei de ganho precisa produzir essa razão |
| Gates (Double) | Rede de dois capacitores (doc). O 90% leva ~4× o 63%, então a tensão de controle decai antes de passar pela curva, e não em dB (der) |
| RCA (Dual) | Dois caminhos: rápido (0,6 ms / 0,33 s) e lento (0,9 s / 2 s) (doc) |

Os modelos de realimentação com rede RC soltam pela tensão de controle, que passa pela curva de ganho do tubo. Isso
dá a forma real da soltura sem nenhuma regra extra.

### Coloração (distorção do elemento de ganho)

| Modelo | Dados | Harmônicos |
|---|---|---|
| SSL | THD+N < 0,05% a +20 dBu (doc, X-Rack); −76,9 dBu a 0 dBu (doc, Revival 2025, não o E original) | Desprezível |
| API 2500 | Sem dado de THD com redução de ganho (?) | Limpo; a cor dos estágios 2510/2520 e do transformador (sec) |
| dbx 160 | Em ∞: 2º 0,075%, 3º 0,5%; o 3º cai pela metade a cada oitava (doc) | O 3º vem da ondulação do detector modulando o VCA (der): surge sozinho no modelo, que só precisa ser calibrado |
| 1176 | Medido (Moore, rev. D): 0,01% a 0 dBu, 0,06% a +9 dBu, 0,25% a +16 dBu sem redução; com ~8 dB de redução, 0,40% a 125 Hz e 0,07% a 1 kHz (doc) | 2º e 3º fortes (doc, Moore); o 2º cresce com a redução pela assimetria do canal do FET (doc). No grave, a distorção vem das constantes rápidas: o ganho segue a forma de onda, e cresce com release mais rápido (doc, UREI e Moore) |
| LA-2A | < 0,35% a +10 dBm, < 0,75% a +16 dBm, < 0,5% com ≥ 30 dB de redução (doc) | 2º do estágio 12AX7 (der, sem medição) |
| Fairchild 670 | THD < 1% até +18 dBm; < 1% com 10 dB de redução a +12 dBm (doc). **IM** (não THD) do gráfico do manual, ~9% com 16,5 dB de redução a +12 dBm (doc), cresce forte com a redução | Ímpares por ser push-pull (der); a simulação de Raffensperger não é validada |
| Gates | ≤ 1% com 0 a 30 dB de compressão (doc) | ?: push-pull → ímpares (der) |
| Altec 436C | < 1,5% com 25 dB, < 2,5% com 30 dB de redução (doc) | ?; o mais colorido |
| RCA BA-6A | < 1% com 12 dB de redução, < 0,6% sem (doc) | ?: push-pull → ímpares (der) |

**Restrição medida, relativa:** Moore (2016) compara o 1176 com o LA-2A e o Fairchild. O 1176 tem 3º muito mais
forte e mais harmônicos ímpares que os dois (doc). A calibração respeita essa ordem.

### Resposta em frequência (pergunta do usuário)

Duas coisas diferentes:
- a resposta do **caminho do áudio**, que se ouve;
- a do **detector**, que muda o que o compressor "escuta".

| Modelo | Caminho do áudio | Detector |
|---|---|---|
| SSL | Plano (mesa) | Plano; o console pode mandar os filtros do canal para o sidechain (doc), coisa que a faixa não oferece |
| API 2500 | ±0,5 dB de 20 Hz a 50 kHz (doc) | THRUST: inclinação de +10 dB/década, −15 dB em 20 Hz e +15 dB em 20 kHz no LOUD (doc); MED corta abaixo de ~200 Hz e reforça acima de 2–4 kHz (doc, sem curva exata) |
| dbx 160 | ? → plano | Plano |
| 1176 | 20 Hz a 20 kHz ±1 dB (doc). Transformador de saída dentro do laço (doc); a saturação do transformador de entrada é opcional (nosso) | Plano |
| LA-2A | ±0,1 dB de 30 Hz a 15 kHz (doc): praticamente plano | R37: plano de fábrica; no máximo, ~10 dB a mais de redução em 15 kHz do que abaixo de 1 kHz (doc). A luz da célula também depende da frequência (doc) |
| Fairchild 670 | ±1 dB de 40 Hz a 15 kHz (doc); transformadores | Plano |
| Gates | ±1 dB de 30 Hz a 15 kHz, com 0 a 30 dB de redução (doc) | Plano |
| Altec 436C | ±1,5 dB de 30 Hz a 15 kHz (doc) | Plano |
| RCA BA-6A | ±1 dB abaixo do limiar; +1/−2 dB até 20 dB de redução (doc): a resposta **muda com a compressão** | Plano |

**Como ler as faixas:** ±1 dB (ou ±1,5 dB) é a tolerância **dentro** da faixa; nada se diz fora dela. Os filtros de
borda são **nossos**, ajustados **por aparelho** para cumprir cada especificação:
- LA-2A: ±0,1 dB, então nada audível;
- Fairchild: a partir de 40 Hz;
- 1176: de 20 Hz a 20 kHz;
- RCA: um filtro dinâmico, que segue a redução de ganho.

Os transformadores entram com o modelo do XFMR do master, com parâmetros por aparelho (nosso).

### Link estéreo no hardware

| Modelo | Link |
|---|---|
| SSL e API | Somam as tensões de controle (doc) |
| LA-2A | Tensão de controle comum (doc) |
| 1176 | Pelo 1176SA, que dobra o ataque mínimo para 40 µs (doc) |
| dbx 160 e Fairchild original | Sem link (doc) |

A faixa é sempre linkada pelo maior dos dois canais (nosso, como hoje).

### O que não se sabe (e o que fazemos)

| Lacuna | O que fazemos (nosso) |
|---|---|
| Largura dos joelhos (SSL, API, 1176) | SSL 10 dB; API MED 6 dB; 1176 de 6 dB (4:1) a 2 dB (20:1) |
| Parte rápida do release da SSL | 40 ms |
| Constantes RMS (SSL, API) | 5 ms na SSL (mantém o atual), 10 ms no API |
| Fórmula do make-up automático | A regra do nível nominal, já em uso |
| Espectro harmônico dos valvulados | Push-pull: 3º dominante e 2º pequeno; estágio single-ended (saída do LA-2A): 2º. Respeitando a ordem medida por Moore |
| Curva da célula T4 | Estados rápido, lento e de memória, calibrados pelos 60 ms até 50% e pela soltura total do manual |
| Lei dos knobs de ataque e release (1176) | Logarítmica |
| THD do API com redução | Leve, pelos estágios discretos (sec); nunca mais que a SSL × 2 |
| ∞ do API em OLD | k = 50 (≈ 51:1) |

---

## Parte 2: modos do gate

Decisão do usuário: o gate mantém os knobs GATE (threshold) e RANGE, e ganha um seletor de modo. O modo traz pronto
o que não tem knob:
- tipo (gate ou expander de 2:1);
- ataque, hold, release e histerese;
- filtros da chave (o que o gate "escuta").

O painel perde os controles do gate.

Partimos da tabela do anexo E (A.4), com dados do manual do Drawmer DS201, dos presets da Yamaha, da Sound On Sound
e do SSL. Ajustes nossos:
- respeitar os limites do SSL: release de 0,1 a 4 s;
- ataque como inclinação em dB: 40 dB em 1,5 ms (26,7 dB/ms, NORMAL) ou em 100 µs (400 dB/ms, FAST);
- filtros de chave fixos por modo como extensão. O console E pode mandar os filtros do canal para o sidechain (doc),
  mas o módulo X-Rack não tem filtros de chave.

| Modo | Tipo | Ataque | Hold | Release | Histerese | Chave HPF | Chave LPF | Base |
|---|---|---|---|---|---|---|---|---|
| **STD** | Gate | Fast | 0 | 0,33 s | SSL variável | — | — | Yamaha "Gate", 0 ms e 331 ms (doc) |
| **KICK** | Gate | Fast | 50 ms | 0,1 s | 3 dB | 35 Hz | 250 Hz | DS201 (doc); release no mínimo do SSL (o DS201 usa 50 ms, a Yamaha 400 ms) |
| **SNARE** | Gate | Fast | 50 ms | 0,24 s | 3 dB | 1 kHz | 2 kHz | DS201 e Yamaha (doc) |
| **TOMS** | Gate | Fast | 100 ms | 0,75 s | 6 dB | 60 Hz | 3 kHz | Yamaha Tom, release (doc); resto nosso |
| **VOX** | Expander 2:1 | Normal | 0 | 0,1 s | 4 dB | 150 Hz | — | Houghton, hold 0 e chave 150 Hz (doc); resto nosso |
| **GTR** | Gate | Normal | 50 ms | 0,3 s | 6 dB | 80 Hz | — | SOS (sec); valores nossos |
| **BASS** | Gate | Normal | 0,2 s | 0,2 s | 3 dB | — | — | Nosso. O DS201 usa esses tempos num baixo disparado pela chave externa do bumbo, e a faixa não tem chave externa |
| **NOISE** | Expander 2:1 | Normal | 50 ms | 0,28 s | 3 dB | 150 Hz | — | Yamaha "Click Erase", 2:1 e 284 ms (doc); resto nosso |
| **EXP** | Expander 2:1 | Normal | 0 | 0,1 s | SSL variável | — | — | SSL (doc) |

**Notas:**
- **Histerese:** é **nossa** em todos os modos; nenhuma fonte dá números.
  - "SSL variável" é a curva já implementada: 2 dB com o threshold em +10 dBu até 8 dB em −30 dBu. A SSL só diz que
    ela cresce quando o threshold desce.
  - Nos outros modos, o valor da tabela substitui essa curva.
- **Ratio do gate:** os documentos da SSL divergem:
  - o manual do módulo E (X-Rack) diz ∞:1;
  - o Native Channel Strip, 40:1;
  - o guia citado pelo usuário, 20:1.

  Mantemos 20:1, que é defensável (≥ 20:1 é praticamente fechado com o range de 40 dB).
- **Detector:** pico com 10 ms de queda, como hoje (nosso). O X-Rack diz RMS verdadeiro, mas um RMS lento atrasaria
  a abertura nos transientes.
- **O release interage com o RANGE** (doc): é uma constante de tempo em dB, então fechar 40 dB leva mais tempo que
  fechar 10.
- **Sem lookahead:** é uma emulação analógica (nosso).
- **Sem modo de overheads:** nenhuma fonte recomenda gate ali.
- **Projetos de antes:** EXP ligado vira EXP; o resto, STD.

---

## Parte 3: dossiês

Os dossiês estão completos, com as fontes de cada um, como foram escritos na pesquisa:
- **Anexo A:** SSL E (canal) e API 2500/525
- **Anexo B:** dbx 160 e 1176
- **Anexo C:** LA-2A e Fairchild 670
- **Anexo D:** Gates Sta-Level, Altec 436 e RCA BA-6A
- **Anexo E:** modos de gate e técnicas de DSP (detectores, célula óptica, FET, vari-mu, distorção)

---

## Anexo A: Compressores SSL (4000 E/G, canal) e API (2500 e 525): dados para modelagem

Pesquisa para o channel strip analógico do LinuxMusic. Cada dado é marcado assim:

- **[DOC]**: documentado em fonte primária (manual ou folha de especificações do fabricante).
- **[REV]**: dito em review técnica de revista ou em manual de plug-in licenciado (Waves, UAD).
- **[LORE]**: fórum ou opinião, sem medição.
- **[EST]**: estimativa ou interpretação minha, para guiar a implementação. Não é especificação.
- **[?]**: dado desconhecido, não encontrado.

---

### 1. SSL SL 4000 E: compressor/limitador do canal

#### 1.1 História e revisões

| Item | Dado | Status |
|---|---|---|
| Console | SL 4000 E, lançado em 1979 | [DOC] SSL Channel Strip Guide 2025 |
| Antecessor (4000 B) | Dinâmica inspirada no Bus Compressor: **detecção de pico** e "sidechain VCA within a feedback loop". VCA dbx 202 "black can" compartilhado com o nível do canal, o que obrigava a dinâmica a ficar pós-EQ. O 4000 B introduziu o **auto make-up** (threshold e make-up acoplados). | [DOC] Channel Strip Guide |
| Topologia do E | Abandona o VCA compartilhado e passa a ter **VCA Class A separado (Aphex), em arquitetura feed-forward**. O Aphex foi descontinuado e trocado por um VCA dbx. | [DOC] Channel Strip Guide |
| Detector | **"True RMS converter"** no side-chain. Elemento de ganho: "all-discrete... Class A VCA", idêntico ao do console original (sobre o Revival 4000). | [DOC] Revival 4000 User Guide |
| Ataque | Primeiro console SSL com **ataque dependente do programa, entre 3 ms e 30 ms**, que reage à frequência e à amplitude do sinal. Modo FAST ATTACK fixo em 3 ms. | [DOC] Channel Strip Guide |
| Revisões | Houve várias revisões da dinâmica do canal. A **"Revision 4"** é considerada a referência: introduziu soft knee, ataque FAST/SLOW e release logarítmico. | [DOC] Channel Strip Guide |
| Revival 4000 (reedição, 2025) | Mantém o circuito E e acrescenta opções de revisões anteriores: **hard knee** (desliga o soft knee), fast attack e release **LIN**/LOG. | [DOC] Revival 4000 User Guide |

#### 1.2 Controles e faixas

| Controle | Faixa / passos | Fonte / status |
|---|---|---|
| RATIO | Contínuo, de **1:1** (seção inativa) a **∞:1** ("true limiter" no fim do curso) | [DOC] Revival; [REV] Waves: "1 to infinity" |
| THRESHOLD | **+4 dB a −26 dB** (Revival). O manual Waves do E/G-Channel dá **+10 dB a −20 dB**. A diferença é de exatamente 6 dB, a mesma do deslocamento do LIN REL (ver abaixo). A referência (dBu) não é dita explicitamente. | [DOC] Revival; [REV] Waves |
| Auto make-up | Embutido no THRESHOLD: baixar o threshold aumenta o ganho, mantendo "a steady output level regardless of the amount of compression". A Waves diz que o make-up é "calculated from the Ratio and Threshold settings". **A lei exata não está documentada.** | [DOC] Revival; [REV] Waves; lei: [?] |
| RELEASE | Contínuo, **0,1 s a 4 s** | [DOC] Revival; [REV] Waves |
| Componente rápida do release | A UA descreve o release como dependente do programa: há sempre uma componente rápida para recuperar transientes, e o knob ajusta só a componente lenta (0,1 a 4 s). | [REV] UA WebZine 2009, via resultado de busca. O original retornou 404. |
| FAST ATT (F.ATK) | Ligado: **"3 ms for 20 dB gain reduction"**. Desligado: **"30 ms for 20 dB gain reduction"** (especificado como "30 ms per 20 dB") e dependente do programa. A Waves diz 1 ms no modo fast e "auto-sensing, program dependent" no modo lento. | [DOC] Revival; [REV] Waves (1 ms é o valor do plug-in) |
| Knee | Soft knee por padrão. Botão hard knee ("more aggressive"). **A largura do soft knee não é documentada.** | [DOC] Revival; largura: [?] |
| LIN REL | Troca o release de logarítmico para linear e **"also raises the threshold by 6dB"**. | [DOC] Revival |
| Modos combinados | Soft/Hard × Log/Lin dá "four very different modes". Hard + Lin é a combinação "far more dramatic". | [DOC] Revival |
| Side-chain | O filtro (HPF 20–500 Hz, LPF 3–16 kHz) e/ou o EQ podem ir para o side-chain (DYN S/C). Há entrada de side-chain externa. | [DOC] Revival |
| Posição | Pré-EQ por padrão, ou pós-EQ ("CH OUT"/"DYN POST EQ") | [DOC] Revival; [REV] Waves |
| Link estéreo | No console, os canais podem ser linkados. No Revival, a S/C LINK **soma as tensões de controle**, e a unidade com mais GR comanda as duas. | [DOC] Revival |
| Medidor | LEDs de GR (5 segmentos no Revival/500) | [DOC]/[REV] |
| Gate/expander, mesmo VCA | Range 0–40 dB; threshold −30 a +10 dB com histerese variável; ataque de 1,5 ms por 40 dB (fast: 100 µs); release de 0,1 a 4 s. A Waves diz: "the same gain change circuitry is used for both" | [DOC] Revival; [REV] Waves |

#### 1.3 Balística: interpretação para DSP

- **"30 ms per 20 dB"** sugere que o ataque é especificado como o tempo para atingir 20 dB de GR, o que lembra um slew rate em dB/s (≈ 0,67 dB/ms no modo lento, ≈ 6,7 dB/ms no fast). Isso é **[EST]**. O fabricante não diz se é uma constante RC ou um slew. Combinado com "program dependent 3–30 ms", um modelo razoável é um RC cujo τ diminui quando o excesso sobre o threshold cresce.
- **Release "logarithmic" × "linear"**: o significado elétrico exato não está documentado **[?]**. Leitura plausível **[EST]**:
  - LOG = descarga RC (exponencial) da tensão de controle em dB, o que soa natural.
  - LIN = descarga a corrente constante (rampa linear em dB/s), mais "dramática".
- **Detector RMS** com constante de média própria. **Valor desconhecido [?].**
- Feed-forward: a curva estática é imposta diretamente pelo gain computer, e a razão é exata.

#### 1.4 Curva estática

- Soft knee padrão, com largura **[?]**. Na falta de dado, sugiro **[EST]** um knee quadrático (Giannoulis) com W ≈ 6–10 dB e opção de W = 0 (hard).
- Razão contínua até ∞:1. Não há documentação de razão negativa ("over-compression").

#### 1.5 Coloração e medições

| Medição (Revival 4000) | Valor | Condição |
|---|---|---|
| Dynamics circuit in: ruído | −76,7 dBu típico | 0 dB, sem ponderação, 20 Hz–20 kHz, terminação 150 Ω |
| Dynamics circuit in: THD+N | −76,9 dBu típico (≈ 0,014% relativo a 0 dBu, por conta minha) | 0 dB, 0 dBu out, 1 kHz. **Sem GR especificada.** |
| THD em função da GR | **[?]**: não encontrado em fonte primária | |

- Caráter: "punchy", "aggressive compression", som dos anos 80 em bateria [DOC, marketing SSL]. O Revival só adiciona "grit e cor" se for empurrado [REV SOS].
- O G é descrito pela SSL como "tighter response... greater transient control", mantendo o punch do E [DOC, marketing]. Fóruns divergem sobre qual dos dois é mais agressivo [LORE].

#### 1.6 Uso típico

Não há receita oficial. As práticas comuns são ataque fast em bateria/caixa e auto make-up acompanhando o threshold **[LORE]**. Não há números confiáveis.

#### 1.7 G Series: diferença em relação ao E

- **Canal 4000 G**: a mudança principal foi o **EQ** ('292/'383 "G-EQ"). A dinâmica é praticamente o mesmo circuito, com refinamentos incrementais. A Waves usa a mesma seção de dinâmica, com os mesmos números, nos plug-ins E-Channel e G-Channel [REV]. Algumas fontes dizem que os G tardios receberam VCAs melhores [LORE]. Não há números distintos documentados **[?]**.
- **9000/Duality** (para contexto): VCA SSM 2018 (9000) ou THAT 2181A (Duality), com os mesmos tempos e escolha **Peak vs RMS** [DOC Channel Strip Guide].
- **Bus Compressor (G-Comp, centro do console)**:
  - Ratio 2, 4, 10:1
  - Ataque 0,1 / 0,3 / 1 / 3 / 10 / 30 ms
  - Release 0,1 / 0,3 / 0,6 / 1,2 s + Auto (dependente da duração dos picos)
  - Threshold e make-up contínuos [REV UAD/Waves, via busca; DOC SSL 500 G Comp]
  - O guia do módulo 500 diz [DOC] que o knee (threshold efetivo) **muda com o RATIO**: "Decreasing the RATIO setting lowers the effective threshold"
  - Side-chain **"dominant"**: os canais L e R são retificados por **detector de pico full-wave**, e o mais alto controla a GR estéreo. A versão 500 tem HPF no side-chain [DOC].
  - Trabalho de VA sobre o SSL 500 G-Bus: o melhor modelo usa knee suave com largura calculada automaticamente a partir de threshold e ratio, e um detector "switching one-pole" (attack/release) [REV arXiv 2504.04589].

---

### 2. API 2500 Stereo Bus Compressor

#### 2.1 Topologia

| Item | Dado | Status |
|---|---|---|
| Elemento de ganho | VCAs múltiplos por canal: o manual original diz **"three VCA's"**. A Mix (2010s) diz 3× THAT 2180 desde 2005. A SOS conta **4× THAT 2180 + 1× THAT 2252 (detector RMS)** por canal. O número varia por lote. | [DOC] manual 2500; [REV] Mix, SOS |
| Detector | **RMS** (THAT 2252, segundo a SOS). Um detector por canal, mesmo linkado. | [DOC]/[REV] |
| TYPE | **NEW** = feed-forward: o detector pega o sinal antes do VCA, e a razão é exata. **OLD** = feedback: o detector pega o sinal depois do VCA/saída (2520), "como API 525, 1176, 660". | [DOC] |
| Side-chain externo | Só funciona em NEW. Em OLD é ignorado (a Waves troca para NEW automaticamente). | [DOC]; [REV] Waves |
| Áudio | Discreto: op-amp 2510 na entrada, 2520 na saída, **saída a transformador** | [DOC] |
| Link | Os dois canais são completos. No link, as **tensões de controle são somadas** em proporção, e cada canal mantém o seu detector. | [DOC] |
| Peculiaridade OLD | Com make-up manual em OLD, a detecção acontece **depois do make-up**: aumentar o GAIN aumenta a compressão. | [REV] SOS (coerente com o diagrama de blocos do 2500+, que tira o OLD da saída) |

#### 2.2 Controles e faixas

| Controle | Valores | Status |
|---|---|---|
| THRESH | Contínuo, **+10 dBu a −20 dBu** (2500 original). **+20 a −20 dBu** no 2500+ e no 50th. LED de "over threshold" nas versões novas. | [DOC] |
| ATTACK | 7 posições: **0,03 / 0,1 / 0,3 / 1 / 3 / 10 / 30 ms**. A forma de especificação (constante de tempo? GR alvo?) não é dita **[?]**. | [DOC] |
| RATIO | 7 posições: **1.5, 2, 3, 4, 6, 10, ∞:1** | [DOC] |
| RELEASE | 7 posições: **0,05 / 0,1 / 0,2 / 0,5 / 1 / 2 s / VAR** | [DOC] |
| VARIABLE RELEASE | Contínuo, **50 ms a 3 s** (ativo quando RELEASE = VAR) | [DOC] |
| Release automático/dependente do programa | **Não existe**: o release é só manual | [REV] SOS. TapeOp fala em "program-dependent", mas contradiz o manual. |
| KNEE | HARD / MED / SOFT. HARD é transição abrupta. MED, um "slight fade-in". SOFT, um "gradual fade-in", parecido com "over-easy". O gráfico do manual não tem escala; **largura em dB [?]**. | [DOC] |
| THRUST | **NORM**: plano. **MED**: corta graves abaixo de ~200 Hz e reforça agudos acima de ~2–4 kHz, médios planos (no gráfico, ≈ −15 dB em 20 Hz e ≈ +15 dB em 20 kHz). **LOUD**: inclinação contínua de **+10 dB/década (≈ +3 dB/oitava)**, **−15 dB em 20 Hz e +15 dB em 20 kHz**, inverso do ruído rosa. Patente US 5,170,437. | [DOC] manual 2500 (texto e gráfico) |
| THRUST (outra leitura) | A SOS descreve MED como passa-altas de 2 dB/oitava e LOUD como 4 dB/oitava, o que **diverge** do manual. A Mix (529) descreve MED como −3 dB/oitava abaixo de 200 Hz e +3 dB/oitava de 4 a 20 kHz. | [REV]. Use o manual. |
| L/R LINK | IND (0%), 50, 60, 70, 80, 90, 100% | [DOC] |
| SHAPE (link) | 2500 original: filtro só no caminho do link, em **HP / LP / BP** (e plano). Versões 2500+/50th: **FAST** (pico; atenua a influência do RMS) / **SLOW** (RMS; atenua a de pico) / ambos. Frequências de corte **[?]**. | [DOC] |
| GAIN (make-up) | Manual de **0 a +24 dB**, ou **AUTO** em função de THRESH e RATIO, "o mesmo que o Ceiling do 525". Abaixo de +4 dBu, girar o THRESH sobe a saída até ~+4 dBu, que depois é mantida. | [DOC] |
| TILT | Trimpot de CV de **±2 dB** L/R. Só age acima do threshold. | [DOC] |
| IN / BYP | Soft bypass da GR, e bypass a relé (hard-wire, ativo também sem energia) | [DOC] |
| BLEND/MIX (2500+ e 50th) | Crossfade (dry/wet) ou paralelo (dry 100% + wet em %), com detent em 50% | [DOC] |
| Medidores | VU (0 VU = +4 dBu). GR de 0 a 20 dB no medidor. **GR máxima de 30 dB**. | [DOC] |
| Calibração Waves | +10 dBu = −12 dBFS, ou seja, **0 dBFS = +22 dBu** (+4 dBu = −18 dBFS) | [REV] Waves |

#### 2.3 Especificações (2500+)

| Item | Valor | Status |
|---|---|---|
| Nível máximo de saída | +32 dB | [DOC] |
| Resposta em frequência | ±0,5 dB, 20 Hz–50 kHz | [DOC] |
| S/N | 120 dB | [DOC] |
| THD | **Não publicada [?]**. Nos fóruns aparece "2180B 0,02% THD", que é dado do chip, não do aparelho [LORE]. | |

#### 2.4 Caráter sonoro

- O manual fala em "NEW much harder, OLD very smooth" e diz que com "SOFT, LOUD e OLD... you can hardly hear the compression" [DOC]. THRUST é o "little more punch" switch [DOC].
- TapeOp: "strongly colored" mesmo em soft bypass com threshold em +20 (efeito do caminho 2510/2520 e do transformador). NEW soa "raw, aggressive, in-your-face" [REV].
- SOS: "sweet and clean", sem ruído ou distorção audível. Linkar abaixo de 100% pode deixar a imagem "andar". OLD é sujeito a overload em transientes (malha fechada) [REV].
- Mix (529): FF "tighter, drier, almost clinical", FB "smoother... vibe-y" [REV].

#### 2.5 Uso típico

- Quick start oficial: todos os knobs às 12h, KNEE MED, THRUST NORM, TYPE NEW, LINK 50%, SHAPE SLOW, auto make-up, sem blend [DOC]. Às 12h, nos seletores de 7 posições, isso corresponde a **[EST]** attack 1 ms, ratio 4:1 e release 0,5 s.
- Bus: 2–3 dB de GR [REV MusicTech, via busca]. Masterização: ratio 1.5:1 [REV TapeOp]. Bateria em paralelo, com THRUST e ganho altos [REV TapeOp].

#### 2.6 API 525 (o "caráter API" clássico)

| Item | Valor | Status |
|---|---|---|
| Topologia | **Feedback**, detecção de **pico**, discreto (2510/2520), saída a transformador | [DOC] folha API 525 |
| Ratio | Chave COMP **2:1** ou LIMIT **20:1** | [DOC] |
| Ataque | Fixo, **15 µs** | [DOC] |
| Release | 0,1 / 0,5 / 2,0 / 2,5 s nas especificações. O texto do mesmo documento diz 0,1/0,5/1,5/2,0 s, e o painel diz "all out=0.1, 1 In=.05(sic), 2 In=2.0, 1&2 In=2.5". O documento é **inconsistente**. O release varia com a frequência: conteúdo agudo/full-band solta mais rápido. | [DOC] |
| Controles | INPUT (threshold), OUTPUT (make-up), **CEILING** (move threshold e output juntos), D-S (de-esser por inversão da curva de energia vocal), OFF, bypass | [DOC] |
| Specs | GR máxima de 25 dB; THD 0,5% máx (30 Hz–20 kHz); ruído 98 dB abaixo de +10 dBu com 10 dB de compressão; resposta ±0,5 dB (30 Hz–20 kHz) | [DOC] |
| 525A (revisão) | Ataque 15 µs–15 ms; release 0,05–2 s em 7 passos | [REV] varejo, via busca |

---

### 3. Modelagem em DSP

#### 3.1 Esqueleto comum (Giannoulis, Massberg e Reiss, JAES 2012; MathWorks)

Gain computer com soft knee (T = threshold, R = razão, W = largura do knee, em dB):

```
x < T - W/2           : y = x
|x - T| <= W/2        : y = x + (1/R - 1) * (x - T + W/2)^2 / (2W)
x > T + W/2           : y = T + (x - T)/R
g_c = y - x
```

Suavização attack/release (branching):

```
g_s[n] = a_A*g_s[n-1] + (1-a_A)*g_c[n]   se g_c <= g_s[n-1]
         a_R*g_s[n-1] + (1-a_R)*g_c[n]   senão
a = exp(-log(9)/(Fs*T))
```

Com esse `a`, T é o tempo de 10% a 90% (definição da MathWorks).

Auto make-up (MathWorks, gain computer em 0 dB):

```
T < -W/2   : M = -(T - T/R)
caso knee  : M = -(1/R - 1)(T - W/2)^2/(2W)
```

O paper recomenda feed-forward por ser estável e previsível, e compara detecção de pico com RMS e domínio linear com logarítmico.

#### 3.2 Mapeamento proposto [EST]

**SSL E (canal)**:
- Detector RMS com janela fixa (τ desconhecido; sugiro ~5–10 ms **[EST]**).
- Gain computer FF com soft/hard knee.
- Ataque derivado de "30 ms (ou 3 ms) por 20 dB", com τ reduzido quando o excesso é grande (ataque dependente do programa).
- Release em duas partes: uma rápida fixa e uma lenta de 0,1 a 4 s, em LOG (RC) ou LIN (rampa em dB/s, com threshold +6 dB).
- Auto make-up acoplado a T/R.
- O VCA Class A pode receber saturação leve (sem dado de THD com GR).

**API 2500**:
- Pré-filtro THRUST (shelving/tilt de +10 dB/década) antes de um detector RMS.
- TYPE NEW: o detector recebe a entrada. TYPE OLD: o detector recebe a saída (pós make-up), em feedback com atraso de 1 amostra ou resolvido iterativamente.
- Knee em 3 larguras (valores **[?]**; sugiro **[EST]** 0 / ~6 / ~12 dB).
- Ataque e release em passos.
- Link: `CV_L' = CV_L + k·f(CV_R)`, com k = link% e f = filtro SHAPE.
- Saída com transformador e op-amp discreto (saturação leve).

**SSL G Bus**: detector de pico full-wave "dominant" (max(L,R)), knee dependente do ratio, ataque e release em passos + auto. Ver arXiv 2504.04589 para validação.

---

### 4. Lacunas principais

1. Largura do soft knee (SSL) e dos knees MED/SOFT (API): não publicadas.
2. Constantes de tempo dos detectores RMS (SSL E; THAT 2252 no API): não encontradas.
3. Lei exata do auto make-up (SSL e API).
4. Definição precisa do "attack" do API (constante de tempo × tempo até X dB).
5. THD em função da GR: não há dado primário para nenhum dos dois. Só o 525 tem THD (0,5% máx).
6. Significado elétrico de "log release" × "linear release" no SSL.
7. Diferenças de dinâmica entre os canais E e G: não documentadas numericamente.
8. Frequências de corte do SHAPE (HP/LP) do link do API 2500.

---

### Fontes

- SSL, *Revival 4000 User Guide V1.1*: https://solidstatelogic.com/assets/uploads/downloads/Revival%204000%20User%20Guide%20V1.1.pdf
- SSL, *The SSL Channel Strip Guide* (2025, v1.0): https://eu1.download.solidstatelogic.com/Channel%20Strip%20Guide/SSL%20Channel%20Strip%20Guide%20v1%202025.pdf
- SSL, *500 Series G Comp Module User Guide*: https://www.solidstatelogic.com/assets/uploads/downloads/SSL_500_Series_G_Comp_Module_User_Guide.pdf
- Waves, *SSL E-Channel & G-Channel User Manual*: https://assets.wavescdn.com/pdf/plugins/ssl-g-channel.pdf
- Sound On Sound, review SSL Revival 4000: https://www.soundonsound.com/reviews/ssl-revival-4000
- API, *2500 Stereo Bus Compressor* manual original (rev. 18-06-18): https://apiaudio.com/docs/manuals/2500_user_18-06-18.pdf
- API, *2500 50th Anniversary Operator's Manual*: https://apiaudio.com/docs/manuals/2500-50th_user_2019-07-18.pdf
- API, *2500+ Operator's Manual* (specs e diagrama de blocos): https://apiaudio.com/docs/manuals/2500+_user_23-01-09.pdf
- API, *2500 Side Chain note*: https://apiaudio.com/docs/files/2500-SideChainNote_2019-11-01.pdf
- API, folha do *525 Feedback Compressor/Limiter*: https://www.barryrudolph.com/recall/manuals/br525.pdf
- Waves, *API 2500 User Manual*: https://assets.wavescdn.com/pdf/plugins/api-2500.pdf
- Sound On Sound, review API 2500: https://www.soundonsound.com/reviews/api-2500
- Tape Op #52, review API 2500: https://tapeop.com/reviews/gear/52/2500-stereo-buss-compressor
- Mix, review API 529 (cita os VCAs do 2500 e as curvas do Thrust): https://www.mixonline.com/technology/review-api-audio-529-stereo-compressor
- Gu, Zhang, Juvela e Wu, *Solid State Bus-Comp* (arXiv 2504.04589): https://arxiv.org/abs/2504.04589
- MathWorks, *Dynamic Range Control* (equações de Giannoulis et al. e Zölzer): https://www.mathworks.com/help/audio/ug/dynamic-range-control.html
- Giannoulis, Massberg e Reiss, "Digital Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 60(6), 2012 (só a página do e-lib e a citação; não li o PDF): https://www.aes.org/e-lib/browse.cfm?elib=16354
- Dados vistos só em resultados de busca, sem abrir a página: UA WebZine 2009 sobre o 4K Channel Strip (https://www.uaudio.com/webzine/2009/april/doctors.html, hoje 404); faixas do SSL G Bus no manual UAD (https://help.uaudio.com/hc/en-us/articles/30847649785748, 403); release do 525A no varejo.

---

## Anexo B: dbx 160 e UREI/UA 1176: pesquisa para modelagem na strip analógica do LinuxMusic

Legenda de confiabilidade usada em todo o documento:

- **[SPEC]**: número tirado do manual ou da folha de especificações do fabricante (fonte primária, lida diretamente).
- **[FAB]**: afirmação do fabricante fora da tabela de specs (texto do manual, blog oficial).
- **[MEDIDO]**: medição publicada (artigo AES ou acadêmico).
- **[DERIVADO]**: cálculo meu a partir de números [SPEC]. A conta está mostrada e é verificável.
- **[LORE]**: fórum, revendedor, review ou tradição oral. Não usar como verdade.
- **[DESCONHECIDO]**: não encontrei número confiável.

---

### 1. dbx 160 (160/161 original, 160VU), com 160X/160A e OverEasy como contexto

#### 1.1 Família e diferenças entre modelos

| Modelo | Ano | Notas relevantes para modelagem |
|---|---|---|
| **160 / 161** | 1976 | Manual "Model 160/161". Gabinete de madeira com medidor VU iluminado. Também é chamado de "160VU". O 161 é a versão semi-pro (I/O RCA desbalanceado, saída máx. +18 dB). **Knee duro.** Ataque e release **sem controle**, dependentes do programa. Razão de 1:1 até ∞ (≈120:1), **sem razões negativas**. [SPEC] |
| **160VU** | 1976 | É o nome comum do 160 com medidor VU. A Mix cita VCA "200" (discreto, 7 transistores, "silver can") e detector RMS 207/208. Faixa de threshold de 10 mV a 3 V, igual à do manual 160. [LORE/imprensa] |
| **165 / 165A** | 1978 | Primeiro com **OverEasy** (knee suave). Tem knobs de attack e release e um "Peak Stop" (limitador a diodo no 165A). VCA 202. [imprensa: Mix] |
| **160X** | anos 80 | 1U, LEDs. Seleção **OverEasy/hard knee**. Razão de 1:1 até ∞:1 e depois até **−1:1 ("Infinity+")**. Entrada de detector separada (sidechain), link estéreo "true RMS". Release de 0 a 500 ms, dependente do programa. CIs SIP: VCA 146732 (equivalente a 2150/THAT 2159) e RMS 146742. [SPEC + Mix] |
| **160A** (atual) | 1990s+ | Mesma lógica do 160X: OverEasy, Infinity+, threshold de −40 a +20 dBu, "true power summing" no link. [SPEC do manual 160A] |

**Conclusão para modelagem:** o "dbx 160 clássico" (160/160VU) é **hard knee**, sem razões negativas e sem controles de tempo. OverEasy e Infinity+ (negativas) pertencem ao 160X/160A, e o OverEasy já existia no 165. Se a strip tiver um modo "160", OverEasy e razão negativa devem ser opções explicitamente "estilo 160X", não do 160 original.

#### 1.2 Topologia [SPEC/FAB, manual 160/161 e 160X]

- **Feed-forward.** O detector lê a **entrada** e o VCA aplica o ganho, sem laço de realimentação. O manual diz que isso permite compressão "infinita" estável e que ataque e release "acompanham" o envelope.
- **Elemento de ganho:** VCA Blackmer (célula log/antilog), com controle exponencial em dB. A lei de controle dos VCAs 200/202 é de cerca de −6 mV/dB ("decilinear") [imprensa: Mix].
- **Detector:** **true RMS** (detector RMS de domínio logarítmico da dbx). A saída do detector é proporcional ao nível em dB.
- **Link estéreo:**
  - 160/161: **não tem link**. O manual explica a decisão por causa das tolerâncias e manda usar o 162 para estéreo.
  - 160X/160A: cabo de strapping TRS e botão SLAVE. Os detectores RMS das duas unidades "conversam". O 160A especifica "True Power Summing", isto é, soma das energias RMS dos dois canais.
- Entrada balanceada sem transformador (50 kΩ balanceada, 25 kΩ desbalanceada). Saída single-ended com compensação de loop de terra e impedância de 25 Ω. O 160 **não tem transformador** no caminho (o transformador Jensen JT-123-DBX era opcional a partir do 160X).

#### 1.3 Controles do painel do 160/161 [SPEC]

| Controle | Faixa / passos | Observações |
|---|---|---|
| THRESHOLD | Contínuo, de **10 mV (−38 dB) a 3 V (+12 dB)**. Marcas: 10 mV, 30 mV, 100 mV, 300 mV, 1 V, 3 V | Referência 0 dB = 0,775 V. Os LEDs BELOW (âmbar) e ABOVE (vermelho) se referem só à entrada. Com seno exatamente no threshold, os dois LEDs ficam meio acesos. |
| COMPRESSION (ratio) | Contínuo, de **1:1 a ∞ (≈120:1)**. Marcas: 1, 1.5, 2, 3, 4, 6, 10, 20, ∞ | Acima de 10:1 é considerado "limiting". A curva do potenciômetro é **[DESCONHECIDO]**. O 160X diz ter "escala expandida nas razões baixas". |
| OUTPUT GAIN | **±20 dB**, contínuo, com 0 dB às 12 h | Independente do threshold e da razão. |
| METER (3 botões) | INPUT, OUTPUT, GAIN CHANGE | Escala de 60 dB (**−40 a +20 dB**). 0 VU = **+4 dB (1,23 V)** de fábrica, recalibrável por trimpot traseiro de −10 dB (250 mV) a +10 dB (2,5 V). |
| POWER | liga/desliga | — |

#### 1.4 Controles do 160X (para um modo "160X/160A") [SPEC]

| Controle | Faixa |
|---|---|
| THRESHOLD | **−40 a +20 dBm (7,8 mV a 7,8 V RMS)** |
| COMPRESSION RATIO | 1:1 → ∞:1 → **−1:1** (Infinity+). Marcas do knob na fig. 3-2B: 1:1, 1.5(?), 2:1, 3:1, 4:1, 6:1, 10:1, ∞:1, −8:1(?), −3:1(?), −1:1. Legibilidade baixa; **conferir antes de copiar**. |
| OUTPUT GAIN | −20 a +20 dB |
| OVER EASY | botão (soft knee) / solto = hard knee |
| BYPASS (hard-wire), SLAVE, DISPLAY (IN/OUT) | — |
| Medidores | 19 LEDs de nível de −40 a +20 dB e 12 LEDs de GR de −1 a −40 dB. Compressão máxima: **>60 dB** |

#### 1.5 Ataque e release

**160/161 [SPEC]:**
- Ataque, definido como "tempo para reduzir o sinal em 63% da mudança de nível": **15 ms para 10 dB** acima do threshold, **5 ms para 20 dB**, **3 ms para 30 dB**.
- **Release rate: 120 dB/s.**
- Os dois "variam automaticamente com a taxa de mudança do nível do programa".

**160X [SPEC]:**
- Ataque idêntico (15/5/3 ms para 10/20/30 dB), medido na região de compressão infinita.
- Release: "dependente do programa; varia automaticamente de 0 a 500 ms, afetado pelos controles do painel".

**[DERIVADO] Por que esses números saem de um único RMS de constante τ.** O detector RMS dbx faz média exponencial do sinal ao quadrado e entrega o resultado em dB. Isso tem duas consequências:

1. **Release.** Depois de um degrau para baixo, o valor médio quadrático decai como e^(−t/τ). Em dB, a queda é **linear**, com inclinação 10·log10(e)/τ = 4,343/τ dB/s. Para 120 dB/s, **τ ≈ 36,2 ms**. Isso explica o "release em dB/s constante": a curva de release é uma **rampa linear em dB**, não uma exponencial em dB.
2. **Ataque.** Com o mesmo τ = 36,2 ms, um degrau para cima de ΔdB faz o nível em dB atingir 63% de ΔdB nos tempos abaixo. A conta usa P(t) = 1 + (10^(Δ/10) − 1)(1 − e^(−t/τ)), resolvida para 10·log10(P) = 0,632·Δ.

| Δ acima do threshold | Spec dbx | Modelo RMS com τ = 36,2 ms |
|---|---|---|
| 10 dB | 15 ms | **16,4 ms** |
| 20 dB | 5 ms | **7,0 ms** |
| 30 dB | 3 ms | **2,9 ms** |

A concordância é boa. Ela sustenta modelar o 160 como **um único detector RMS (média de x² com τ ≈ 35–36 ms, valor convertido para dB)**, sem constantes de ataque e release separadas. A "dependência do programa" sai da própria física do RMS. Esse τ é **inferência minha, não spec**. O valor real depende do capacitor de timing do detector 207/208, que **[DESCONHECIDO]**.

**Ripple:** com τ finito, o detector deixa passar um ripple em 2f (ver 1.7 sobre o 3º harmônico).

#### 1.6 Curva estática

- **160/161:** hard knee, com ponto de rotação no threshold (fig. 5 do manual). Abaixo do threshold o ganho é 1:1. Acima, a inclinação é 1/R. "∞" ≈ 120:1, então ainda sobe 1 dB a cada 120 dB.
- **160X hard knee:** "define a razão exata". A razão vai de 1:1 a ∞:1 e depois a −1:1.
- **Infinity+ / razões negativas (160X/160A) [SPEC]:** acima do threshold, a saída **cai** quando a entrada sobe. Em −1:1, cada +1 dB de entrada acima do threshold dá −1 dB de saída. Matematicamente: `GR_dB = (L − T)·(1 − 1/R)`, com 1/R variando de 1 (1:1) a 0 (∞) até −1 (−1:1). Para parametrizar, o mais limpo é usar a **inclinação s = 1/R ∈ [−1, 1]**. O manual 160X ilustra usos com sidechain e com delay para "anticipatory compression".
  - **Erro no manual 160A:** o texto diz "up to a maximum of 1:1 (i.e., a 1dB increase in input level above threshold causes a 1dB decrease in output level)". O número correto é −1:1. O texto do 160X ("1 dB decrease... causes 1 dB increase") também está invertido em relação à figura. **A figura 3-2A é a referência:** a curva −1:1 desce acima do threshold.
- **OverEasy (165, 160X, 160A):** knee suave. O "threshold" fica no meio da região de transição, e a razão nominal só é atingida "um pouco acima" do threshold. No 160X, em OverEasy, a razão efetiva é "dependente do programa, afetada por THRESHOLD e RATIO".
  - **Largura do knee: [DESCONHECIDO]**, sem número publicado. **[ESTIMATIVA visual]** a partir da fig. 3-1 do 160X: a curva sai de 1:1 cerca de 8–10 dB abaixo do threshold e chega à assíntota cerca de 5–10 dB acima. Uma largura total de knee de **~15–20 dB** é um chute razoável, a calibrar de ouvido ou em medição.
- **Faixa máxima de GR:** 160X >60 dB [SPEC]. No 160, o medidor de GR cobre 60 dB, mas a GR máxima **[DESCONHECIDO]**.

#### 1.7 Coloração (THD, ruído, resposta)

**160 [SPEC]:**
- **0,075% de 2º harmônico** em compressão infinita e +4 dBm de saída. **0,5% de 3º harmônico típico** em razão infinita.
- Nota do manual: o 3º harmônico domina o THD e **cai linearmente com a frequência**. Em 100 Hz é metade do valor em 50 Hz.
- Ruído de entrada equivalente de **−78 dBm** (não ponderado, entrada em curto).
- Entrada máxima de +21 dB (8,7 V). Saída máxima de +26 dB (15,5 V) bridging ou +24 dBm em 600 Ω.
- **Resposta em frequência: [DESCONHECIDO]** no manual 160/161 (não consta).

**160X [SPEC]:**
- Abaixo do threshold (1 kHz, 0 dBm): **2º 0,07%, 3º 0,07%**.
- Acima do threshold, em compressão infinita (1 kHz): **2º 0,07%, 3º 0,02%**.
- Nota do manual: o 2º é pouco afetado por razão, tempos e frequência. O 3º **cai com tempos mais lentos, frequências mais altas e razões menores**.
- Resposta de +0/−1 dB de 20 Hz a 20 kHz. Ruído < −89 dBm. Faixa dinâmica > 113 dB. Slew rate > 10 V/µs.

**[DERIVADO/inferência] Mecanismo.** O 3º harmônico que cresce em baixa frequência e com razão alta é **modulação de ganho pelo ripple do detector**. O x² filtrado deixa ripple em 2f, que modula o ganho do VCA, e AM em 2f sobre a portadora f gera um componente em 3f. A amplitude cai com f porque o ripple de um filtro de 1ª ordem é ∝ 1/(2f·τ). Ela cresce com a razão porque a sensibilidade do ganho ao nível detectado é (1 − 1/R).

O 2º harmônico vem principalmente de **assimetria/descasamento do VCA Blackmer** (trimável no hardware) e quase não depende da GR. Por isso um modelo fiel não precisa de waveshaper explícito para o 3º: basta deixar o ripple do RMS chegar ao ganho. Um leve termo par fixo (~0,07%) cobre o VCA.

#### 1.8 Uso típico [FAB, manuais]

- **Compressão:** razões de 1:1 a 4:1. **Limiting:** de 10:1 a ∞. Ganho de saída às 12 h (0 dB) como padrão.
- Threshold 10 dB ou mais acima do nível médio para agir só nos picos.
- Voz: começar em ~2:1 (160X) ou 4:1 em OverEasy com 6–10 dB de GR (160A). Baixo: ~4:1.
- **Folclore de estúdio [LORE]:** bumbo e caixa com 160 em razões altas pelo "estalo" do RMS. Não há número.

#### 1.9 Modelagem DSP do dbx 160

**Esquema recomendado (feed-forward, domínio log):**

1. `ms[n] = ms[n−1] + α·(x[n]² − ms[n−1])`, com α = 1 − e^(−1/(τ·fs)) e τ ≈ 36 ms [DERIVADO]. Use **a mesma α** para subida e descida, porque é RMS verdadeiro.
2. `L = 10·log10(ms + ε) + calibração`. Em estéreo linkado no estilo 160A, faça `ms = ms_L + ms_R` (soma de potências).
3. Computador de ganho: hard knee `GR = max(0, L − T)·(1 − s)`, com s = 1/R ∈ [−1, 1]. Para OverEasy, use o knee quadrático de Giannoulis et al. (2012) com largura W de ~15–20 dB [ESTIMATIVA].
4. **Não suavize o GR de novo.** O VCA real aplica o ganho instantaneamente. Suavizar mataria o ripple que produz o 3º harmônico e o release linear em dB.
5. `y = x · 10^(−GR/20) · outGain`, mais um termo par muito pequeno, se desejado.

**Referências:**
- Giannoulis, Massberg & Reiss, "Digital Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 60(6), 2012: topologias feed-forward e feedback, detectores e knee suave.
- Folha de dados do detector RMS THAT 2252 (sucessor do detector dbx): integração log-domain com R/C externo. Ataque e release ficam amarrados pela matemática do RMS.

**Gaps:** o valor real do capacitor de timing do 207/208, a curva do potenciômetro de razão, a largura do knee OverEasy e a resposta em frequência do 160 original.

---

### 2. UREI / Universal Audio 1176 (Rev A "blue stripe", C/D/E LN, F–H, reedição 1176LN)

#### 2.1 Revisões [FAB: manual UA 1176LN 2009; Sound On Sound 2001; Wikipedia]

| Rev. | Ano | Mudança |
|---|---|---|
| A | 1967 | Original. Painel de alumínio escovado com seção do medidor azul ("bluestripe"). O pré-amp usava FETs, segundo a Wikipedia. |
| AB | 1967 | Mais estabilidade e um pouco menos de ruído. |
| B | 1968 | Pequenas mudanças no pré-amp. Segundo a Wikipedia, os FETs do pré-amp foram trocados por bipolares. |
| C | set/1970 | Painel preto. Circuito **LN (low noise)** de Brad Plunkett em módulo epóxi. O LN mantém o FET mais na região linear, com menos distorção. |
| D | ~1973 | LN integrado na placa. |
| E | início dos anos 70 | Igual à D, mais seletor 220 V e resistor de 10 MΩ na chave de razão (evita "pops"). **D/E são as revisões consideradas de melhor som**, e a reedição UA (2000) se baseia nelas. |
| F | 1973 | Saída push-pull classe AB (vinda do 1109) no lugar da classe A (vinda do 1108). Driver do medidor passa a op-amp. |
| G | — | Entrada por op-amp diferencial **sem transformador**. A Wikipedia diz que a dependência de programa foi "removida" [não verificado em primária]. |
| H | — | Painel prateado de novo. |
| AE (UA, 2008) | — | Acrescenta razão 2:1 e ataque "Slo" fixo de 10 ms. |

Sobre o **Rev A "blue stripe"**, o blog da UA diz só que as revisões diferem em "ganho, threshold, knee, início da distorção e pontos doces". Não há números publicados por revisão: **[DESCONHECIDO]**. A reputação de ser "mais sujo e agressivo" é **[LORE]**.

#### 2.2 Topologia [FAB: manual UA 1176LN, seção "Circuit Details"; manual UREI 1176LN]

- **Feedback.** O sidechain lê o sinal **depois** do atenuador a FET, na saída do pré-amp e antes do potenciômetro de Output (R23). O manual UA diz: "the 1176LN is a feedback style compressor since the sidechain circuit samples the signal level after the gain reduction".
- **Elemento de ganho:** um JFET como **resistor controlado por tensão (VVR)** na perna "shunt" de um divisor em L, com resistor série R5 (UA) ou R6 (UREI). Abaixo do threshold, o FET está em polarização quiescente de alta resistência. Acima, a tensão de controle reduz a polarização e a resistência cai, atenuando o sinal.
- **Cadeia (Rev A–F):**
  1. Input pot, um atenuador passivo em "bridged-T" de 600 Ω.
  2. Transformador de entrada.
  3. Atenuador a FET.
  4. Pré-amp de baixo ruído com bastante realimentação negativa.
  5. Output pot.
  6. Amplificador de linha classe A: Darlington mais 2N3053.
  7. **Transformador de saída** com enrolamentos extras de realimentação.
- **Detector:** inversor de fase e seguidor de emissor (Q7/Q8, Q9/Q10 no manual UA) alimentam dois diodos em **retificação de onda completa** (CR2/CR3). O resultado é filtrado por C22.
  - **R55 (Attack)** controla a velocidade de carga de C22. **R56 (Release)** controla a descarga.
  - É um detector de **pico/média retificada com RC assimétrico**, não RMS.
  - Os **botões de razão** fazem duas coisas: definem quanto sinal vai ao sidechain e aplicam uma **polarização DC aos diodos**, que é o threshold.
- **Threshold de calibração ("Q bias", R59):** com seno, Input e Output ajustados para 0 dB de saída, gira-se R59 até a saída cair **1,0 dB**. O threshold depende do FET específico.
- **Estéreo:** só com o acessório externo 1176SA. Em estéreo, os knobs de Attack e Release das duas unidades interagem, e **o ataque mais rápido dobra para 40 µs** [FAB].

#### 2.3 Controles do painel (reedição UA 1176LN e manual UREI)

| Controle | Faixa | Direção / observações |
|---|---|---|
| INPUT | Contínuo (marcas até 48, por convenção; "24" às 12 h ≈ ganho unitário com OUTPUT também em 24) | Ajusta o nível que entra **e portanto o threshold efetivo**. O threshold é fixo; mais Input significa mais GR. |
| OUTPUT | Contínuo (idem) | Make-up gain e drive do amplificador de saída. |
| ATTACK | **20 µs a 800 µs** [SPEC]. Marcas de 1 a 7. Totalmente anti-horário = **OFF** (posição com clique) | **Horário = MAIS RÁPIDO.** "7" é o mais rápido. Em OFF o sinal passa pelos circuitos (cor) **sem GR**. |
| RELEASE | **50 ms a 1,1 s** [SPEC], para 63% de recuperação (UREI). Marcas de 1 a 7 | **Horário = MAIS RÁPIDO.** |
| RATIO | Botões **4, 8, 12, 20** (:1). São intertravados por projeto, mas dá para apertar vários de uma vez | 4 e 8 = compressão. 12 e 20 = limiting. **Razões mais altas sobem o threshold.** Sem botão algum: sem compressão (só cor), segundo a UA. |
| METER | Botões **GR, +8, +4, OFF** (OFF também desliga a unidade) | VU. Em +4/+8, 0 VU = +4/+8 dBm na saída. Em GR, 0 = sem GR, e a agulha cai com a GR. Trimpot "Zero Set"/"GR 0 adjust" no painel, entre Input e Output. |

**Nota sobre a direção dos knobs:** o manual UA diz textualmente que os tempos ficam "faster, not slower, as their corresponding knobs are turned up (clockwise)". O review do Ovnilab diz que o release é o contrário, o que **contradiz a fonte primária e deve ser ignorado**.

**Mapeamento knob → tempo:** **[DESCONHECIDO]**. A lei dos potenciômetros R55 e R56 não é publicada. Os números 1–7 "não significam nada" em tempo absoluto [LORE, Ovnilab]. Moore cita engenheiros que descrevem o ataque como "de rápido a mais rápido" [LORE citado em AES].

#### 2.4 Threshold por razão [SPEC: manual UREI 1176LN, texto OCR do archive.org]

O threshold do 1176 é fixo, com referência 0 dB = 0,775 V. Valores ±2 dB, com saída "com ~10 dB de reserva":

| Razão | Entrada no threshold | Saída de limiting no threshold |
|---|---|---|
| 20:1 | −24 dB | +10 dBm |
| 12:1 | −25 dB | +9 dBm |
| 8:1 | −26 dB | +8 dBm |
| 4:1 | **−30 dB** ("o threshold mais baixo", segundo a seção 3.5.1; a célula da tabela está ilegível no OCR) | +7 dBm |

A Sound On Sound (2001) dá "cerca de −32 dBm em 4:1 e −25 dBm em 20:1". É secundária e coerente na ordem de grandeza.

**Atenção:** essa tabela vem de um manual UREI de revisão tardia (entrada por amplificador diferencial IC2, 20 kΩ, ou seja, Rev G ou posterior). A reedição UA (D/E) tem entrada de 600 Ω com transformador. Os valores absolutos podem diferir; a tendência (threshold sobe com a razão) é a mesma [FAB UA].

**Knee:** a Wikipedia, citando o manual, chama de "soft knee". Moore (2012), observando a figura do manual UREI, diz que o knee **fica mais duro em razões altas** e mais suave em 4:1/8:1. Isso é qualitativo; a largura do knee é **[DESCONHECIDO]**.

#### 2.5 Ataque, release e dependência do programa

- **[SPEC] UREI:** ataque "menos de 20 µs para 100% de recuperação" (sic), ajustável até 800 µs. Release de 50 ms a 1,1 s "para 63% de recuperação".
- **[SPEC] UA:** ataque de 20 a 800 µs, release de 50 ms a 1,1 s.
- **[MEDIDO] Moore et al. (AES 140, 2016)**, 1176 Rev D, ataque em 3 e release em 7, burst de 0→12→0 dBu, ~8 dB de GR: há um **pico de ~1 ms** em que o sinal ultrapassa o threshold, e depois **5 ms até 63%** do nível de regime. O release é "mais suave que o ataque". Isso mostra que o ataque nominal em µs **não** equivale ao tempo de assentamento da GR com programa real.
- **Dependência do programa [FAB, Will Shanks/UA via Moore 2012 e Wikipedia]:** o 1176 comprime na razão escolhida nos transientes, mas "a razão sempre sobe um pouco depois do transiente". O release é mais rápido depois de transientes e mais lento depois de compressão sustentada.
  - Mecanismo provável **[inferência]**: a topologia feedback combinada com o C22 carregado/descarregado por R55/R56 e com a não-linearidade do FET e dos diodos.
- **[SPEC UREI, texto]:** "a distorção de pico em baixa frequência aumenta com release mais rápido". Release muito curto deixa o ganho se recuperar entre ciclos de graves, o que é audível e gera harmônicos.

#### 2.6 Modo "all buttons in" / "British mode"

**[FAB, manual UA]:**
- A distorção "aumenta radicalmente devido a um atraso no ataque dos transientes iniciais", descrito como um "reverse look-ahead".
- A razão vai para "algo entre 12:1 e 20:1".
- Os pontos de polarização mudam em todo o circuito, o que altera ataque e release.
- O medidor "fica louco", muitas vezes encostado no máximo.
- A percepção de distorção aumenta nos graves.

**[FAB, blog UA]:** a distorção de compressão "sobe muito". Combinações parciais são variações; segundo a UA, só os botões externos importam (4+20 equivale a todos). A Pulsar diz o mesmo sobre 4+20 [LORE de fabricante de plugin].

**[MEDIDO/AES, Moore 2016 e tese 2017]:**
- O FET opera "fora das condições de projeto, com o ponto de polarização bem fora da calibração".
- Isso dá distorção significativa e um comportamento temporal de "pumping".
- A tese conclui que o 1176 "impõe quantidades significativas de distorção" em all-buttons e com ataque e release rápidos.

**[Wikipedia citando Shanks]:** a curva de compressão fica mais severa, em forma de "platô".

**Curva estática com "overshoot" ou inclinação negativa:** a ideia de que, em all-buttons, a saída **cai** acima de certo nível (curva em "S" invertido) é comum em fóruns e em material de plugins. **Não encontrei medição primária publicada com números: [LORE/DESCONHECIDO].** O que é documentado é razão entre 12:1 e 20:1, mais distorção, ataque atrasado e GR que fica "presa".

**[Inferência sobre o mecanismo elétrico]:**
- Cada botão de razão seleciona um divisor que define o ganho do sidechain e a polarização DC dos diodos.
- Com vários botões apertados, esses divisores ficam **em paralelo**. O sidechain recebe mais sinal (laço de realimentação mais forte, razão alta) e a polarização dos diodos/FET muda (threshold e ponto de operação deslocados).
- Isso leva o FET a uma região de V_DS e V_GS em que a resistência deixa de ser linear com o sinal. Resultado: distorção forte, sobretudo em graves, que têm grande excursão por mais tempo.
- **Não confirmado em esquemático nesta pesquisa.**

#### 2.7 Coloração (THD, ruído, resposta)

**[SPEC] UA 1176LN:**
- Resposta de 20 Hz a 20 kHz, ±1 dB. Ganho de **45 dB ±1 dB**. A página de produto da UA diz 50 dB; há conflito entre fontes.
- Distorção: "> 0,5% THD de 50 Hz a 15 kHz com limiting, release em 1,1 s". O ">" é quase certamente erro de digitação no manual UA; o manual UREI diz **"menos de 0,5%"**.
- Saída de +22 dBm com THD de no máximo 0,5%.
- **S/N > 81 dB** com sinal no threshold de limiting (30 Hz a 18 kHz; o UREI diz 30 Hz a 15 kHz).
- Entrada de 600 Ω (bridged-T, flutuante). Carga de saída de 600 Ω.

**[SPEC] UREI (rev. tardia):**
- Entrada de 20 kΩ balanceada / 10 kΩ desbalanceada, máxima de +20 dB.
- Saída máxima de +24 dBm em 600 Ω (12,28 V). Carga de 150 Ω ou mais.
- THD < 0,5% (50 Hz a 15 kHz, com limiting, release de 1,1 s).

**[MEDIDO] Moore et al., AES 140 (1176 Rev D):**

| Condição | THD 1176 |
|---|---|
| Sem compressão, 1 kHz, 0 dBu | 0,01% |
| Sem compressão, +9 dBu | 0,06% |
| Sem compressão, +16 dBu | 0,25% |
| Comprimindo (~8 dB GR, A=3, R=7), 125 Hz | 0,40% |
| Release, 125 Hz | 0,27% |
| Comprimindo, 1 kHz | 0,07% |
| Release, 1 kHz | 0,01% |

- **Harmônicos:** o 1176 gera um **3º harmônico bem mais pronunciado** e harmônicos **ímpares mais fortes** que o LA-2A e o Fairchild, além de 2º e 3º fortes, segundo Moore.
- A distorção é **muito maior em graves** (125 Hz contra 1 kHz). Ela é atribuída aos tempos rápidos: o ganho segue o ciclo da onda.
- **Comportamento FET × GR [FAB + inferência]:** o FET como VVR só é linear para V_DS pequeno. Quanto mais GR (menor R_DS), maior a tensão do sinal sobre o canal em relação ao ponto de operação, e mais distorção, principalmente de 2ª ordem pela assimetria do canal. O circuito LN (Rev C em diante) existe justamente para reduzir isso.
- Há também a distorção do **transformador de saída** (dentro do laço de realimentação da classe A, o que a reduz) e a distorção de **modulação do ganho por ripple do detector** com ataque e release rápidos. Esta última gera os ímpares e o "grit" em graves.
- **Números por revisão: [DESCONHECIDO].**

#### 2.8 Uso típico

- **"Dr. Pepper":** ataque às 10 h, release às 2 h, 4:1, depois Input e Output a gosto [FAB, blog/manual UA].
- **Voz:** 4:1 com bastante GR. Ataque mais rápido para dar "mordida" (Mike Shipley, no manual UA).
- **Baixo:** 4:1 com ataque e release rápidos, perto das 3 h, e Input/Output perto de "24". Para mais distorção, 8:1 e mais Input [FAB, manual UA]. Owsinski sugere 8:1 com ataque ao meio-dia e release às 3–4 h [citado em Moore].
- **Bateria e room mics:** all-buttons, muita entrada, ataque lento (o transiente passa) e release rápido [FAB, manual UA].
- **Mix:** 4:1, até ~4 dB de GR, é "transparente" [FAB].
- **Só cor:** Attack em OFF, ou nenhum botão de razão.
- **Distorção controlada:** ataque e release no máximo (7), sobretudo em all-buttons.

#### 2.9 Modelagem DSP do 1176

**Literatura:**
- **Gerat, Eichas & Zölzer**, "Virtual Analog Modeling of a UREI 1176LN Dynamic Range Control System", AES 143rd Convention, 2017, paper 9852. Modelo **paramétrico em blocos** (detector, computador de ganho e não-linearidades) com parâmetros otimizados por **Levenberg–Marquardt** só a partir de medições de entrada e saída. Validado com métricas objetivas e teste de audição.
- **Dissertação de mestrado da Universidade de Aveiro (2014)** sobre o 1176LN usando o **método Nodal DK** (simulação de circuito em espaço de estados). Repositório ria.ua.pt; **não consegui abrir o PDF**, então autor e detalhes estão **não verificados**.
- **Moore, Till & Wakefield**, AES 140 (2016): medições de THD e de timing, úteis como **alvos de validação**. Moore & Wakefield, AES 142 (2017): correlação entre "agressividade" percebida e **roughness**. Tese de Moore (Huddersfield, 2017).
- **Giannoulis, Massberg & Reiss**, JAES 2012: como fazer a versão **feedback** de forma estável no digital (detector a partir da saída com um sample de atraso, ou resolução implícita).

**Esquema recomendado (feedback, com não-linearidades):**

1. `u = x · gInput` (o Input define o threshold efetivo). A saturação leve do transformador de entrada (Rev A–F) é opcional.
2. Atenuador FET: `v = u · Rfet/(Rs + Rfet)`, com `Rfet = f(Vc)` (por exemplo o modelo de canal de JFET na região ôhmica, `R ≈ 1/(2K(VGS − VP − VDS/2))`). O termo VDS/2 é o que gera distorção dependente do sinal e da GR. Uma versão barata é `v = u·g(Vc) + k·u²·(1 − g)`, com 2º harmônico crescendo com a GR.
3. Pré-amp com ganho fixo (~45–50 dB no total, junto com o estágio de saída), seguido do sidechain:
   `d = |pre_out · kRatio| − bias(ratio)`, com retificação de onda completa e diodo "mole".
   O capacitor C22 é carregado por R55 (ataque, τ_a ∈ [20 µs, 800 µs]) e descarregado por R56 (release, τ_r ∈ [50 ms, 1,1 s], definido para 63%).
   **Mapear o knob de 1 a 7 de forma logarítmica e invertida (7 = mais rápido)** [ESTIMATIVA: a lei real é desconhecida].
4. `Vc = f(C22)`. Resolva o laço com um sample de atraso ou com Newton por amostra.
5. Make-up via Output, depois o estágio classe A com transformador: saturação suave assimétrica fraca (THD de ~0,01% em 0 dBu a ~0,25% em +16 dBu, conforme as medições de Moore), mais um passa-altas de transformador (resposta ±1 dB de 20 Hz a 20 kHz).
6. **All-buttons:** não é uma "razão" a mais. Modelar como:
   - `kRatio` maior (soma em paralelo dos divisores);
   - bias deslocado (threshold diferente e ponto de operação do FET fora do lugar, com o coeficiente de distorção do FET muito maior);
   - atraso extra no ataque ("reverse look-ahead": um par de ms de lag antes de a GR agir);
   - release mais "agarrado" e brusco (a Pulsar fala em "queda súbita").
   Calibrar para razão efetiva de 12:1 a 20:1 e muito mais THD em graves. A inclinação negativa é opcional e **não documentada**.
7. Estéreo (1176SA): GR comum e ataque mínimo de 40 µs.

**Alvos de validação:**
- THD de 0,01/0,06/0,25% em 0/+9/+16 dBu sem compressão.
- 0,40% (125 Hz) e 0,07% (1 kHz) com ~8 dB de GR.
- Overshoot de ~1 ms e 63% da GR em ~5 ms (A=3, R=7).
- Thresholds de −30/−26/−25/−24 dB (0,775 V) para 4/8/12/20.

**Gaps:** a lei dos potenciômetros de ataque e release, os valores de R55/R56/C22 e do divisor de razão (só nos esquemáticos, que não abri), a curva estática medida em all-buttons, a largura do knee, as diferenças numéricas do Rev A e o comportamento exato da dependência do programa no release.

---

### 3. Tabela-resumo para a strip

| Item | dbx 160 (clássico) | dbx 160X/160A | 1176LN (D/E) |
|---|---|---|---|
| Topologia | Feed-forward | Feed-forward + sidechain externo | **Feedback** |
| Elemento | VCA Blackmer | VCA (2150/2159) | JFET como VVR |
| Detector | True RMS (τ ≈ 36 ms [DERIVADO]) | True RMS | Retificador de onda completa + RC assimétrico (pico/média) |
| Threshold | 10 mV a 3 V (−38 a +12 dB) | −40 a +20 dBm | Fixo, ≈ −30 (4:1) a −24 dB (20:1); Input empurra o sinal |
| Razão | 1:1 a ∞ (≈120:1), contínua | 1:1 → ∞ → −1:1 | 4 / 8 / 12 / 20; all-buttons ≈ 12–20:1 |
| Knee | Duro | OverEasy ou duro | "Soft", mais duro em razões altas [qualitativo] |
| Ataque | 15/5/3 ms para 10/20/30 dB (auto) | Igual | 20–800 µs (knob, horário = rápido) |
| Release | 120 dB/s (linear em dB, auto) | 0–500 ms (auto) | 50 ms–1,1 s (knob, horário = rápido) |
| Saída | ±20 dB | ±20 dB | Contínua (ganho máx. 45 dB) |
| THD | 2º 0,075%, 3º 0,5% em ∞ | 2º 0,07%, 3º 0,02–0,07% | < 0,5% (spec); medido de 0,01 a 0,4% |
| Ruído | EIN −78 dBm | EIN < −89 dBm | S/N > 81 dB |
| Link | Nenhum | Soma de potências RMS | 1176SA, ataque mín. 40 µs |

---

### Fontes

Primárias, lidas diretamente:
- Manual dbx 160/161 (scan): http://www.technicalaudio.com/pdf/dbx/dbx_160.pdf (cópia via web.archive.org)
- Manual dbx 160X (scan): http://www.technicalaudio.com/pdf/dbx/dbx_160x.pdf (cópia via web.archive.org)
- Manual dbx 160A: https://www.la-bs.com/Objets/160A_me.pdf
- Manual Universal Audio 1176LN (2009, v3): https://kmraudio.com/cdn/shop/files/1176ln_manual.pdf (o original fica em https://media.uaudio.com/assetlibrary/1/1/1176ln_manual.pdf)
- Manual UREI 1176LN (texto OCR): https://archive.org/stream/Urei_1176LN_owners_manual/Urei_1176LN_owners_manual_djvu.txt
- UA, "Tips & Tricks — 1176 Classic Limiter Collection": https://www.uaudio.com/blogs/ua/1176-collection-tips

Artigos e medições:
- Moore, Till & Wakefield, "An Investigation into the Sonic Signature of Three Classic Dynamic Range Compressors", AES 140, 2016: https://eprints.hud.ac.uk/28482/1/MooreINvestigationAES.pdf
- Moore, "All Buttons In: An investigation into the use of the 1176 FET compressor in popular music production", JARP, 2012: https://eprints.hud.ac.uk/id/eprint/27391/
- Moore, tese de doutorado (Huddersfield, 2017): https://eprints.hud.ac.uk/id/eprint/34118/
- Gerat, Eichas & Zölzer, "Virtual Analog Modeling of a UREI 1176LN Dynamic Range Control System", AES 143, 2017: https://www.aes.org/e-lib/browse.cfm?elib=19249 (só o resumo)
- Giannoulis, Massberg & Reiss, "Digital Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 2012: https://www.aes.org/e-lib/browse.cfm?elib=16354 (só o resumo e a citação)
- Dissertação da Universidade de Aveiro sobre o 1176LN (Nodal DK): https://ria.ua.pt/bitstream/10773/12791/1/tese.pdf (não abriu; citada via busca)
- THAT Corp, detector RMS 2252 (página do produto): https://thatcorp.com/?p=1716

Imprensa e secundárias:
- Mix, "Birth of a Classic: the dbx 160 Compressor": https://www.mixonline.com/technology/birth-of-a-classic-the-dbx-160-compressor
- Sound On Sound, review do UA 1176LN (2001): https://www.soundonsound.com/node/4922926
- Wikipedia, "1176 Peak Limiter": https://en.wikipedia.org/wiki/1176_Peak_Limiter
- Pulsar Audio, "The History of All-buttons-in Mode": https://pulsar.audio/blog/the-history-of-all-buttons-in-mode/
- Ovnilab, review do 1176LN (contradiz o manual na direção do release; não confiável): https://ovnilab.com/reviews/1176ln.shtml

---

## Anexo C: Teletronix LA-2A e Fairchild 660/670 — dossiê para modelagem no strip analógico do LinuxMusic

Convenções de confiabilidade usadas neste documento:

- **[DOC]** — número/afirmação tirada de documento primário (manual original do fabricante, boletim técnico, esquema).
- **[DOC-UA]** — documento da Universal Audio (fabricante da reedição, com acesso às unidades/arquivos originais). Fonte secundária forte.
- **[PAPER]** — artigo acadêmico revisado (DAFx, SPIE, AES).
- **[REVIEW]** — revista/artigo técnico (Sound On Sound etc.).
- **[LORE]** — fórum, varejista, interpretação não verificada.
- **[CALC]** — conta minha a partir de dados documentados.
- **[PROPOSTA]** — sugestão de engenharia para o LinuxMusic, não é dado do hardware.
- **[?]** — desconhecido / não encontrado em fonte primária.

---

### 1. Teletronix LA-2A Leveling Amplifier

#### 1.1 Visão geral e histórico relevante para modelagem

- Projeto de James F. Lawrence II (Teletronix, Pasadena). LA-2A lançado ~1962; Teletronix vendida à Babcock em 1965; Putnam (Studio Electronics → UREI) adquire em 1967; fim de produção ~1969; "três variações" do LA-2A nesse período [DOC-UA manual 2000]. Reedições: UREI 1979, Harman/JBL 1992, UA 1999/2000 [Wikipedia].
- Célula T4A trocada por T4B por volta do nº de série 1640 [LORE — Vintage King, via busca; não verificado em documento primário].
- **O manual original de 1966 (rev. 3-66) não tem chave Limit/Compress**: os controles de painel listados são "Gain (input level), Peak Reduction and Meter Selector Switch" [DOC 1966]. A chave aparece em versões posteriores — originalmente na traseira, muitos donos a moveram para a frente; na reedição UA está no painel frontal [DOC-UA webzine 2003].

#### 1.2 Topologia

| Item | Descrição | Fonte |
|---|---|---|
| Elemento de ganho | Atenuador eletro-óptico T4: painel eletroluminescente (EL) + fotocélula de CdS como **perna inferior de um divisor de tensão**, colocado **antes** do primeiro estágio amplificador. Os estágios valvulados não mudam de ponto de operação com a redução de ganho. | [DOC 1966], [DOC-UA] |
| Conteúdo da T4 | Um painel EL e **duas** fotocélulas: uma para redução de ganho, outra (casada à mão) para o medidor de GR. | [DOC-UA manual], [DOC-UA webzine] |
| Detector | Não há retificação nem filtragem: o painel EL é excitado diretamente pela tensão de áudio (AC); a "detecção/integração" é a física do EL + fotocondutor. "Rectification and filtering of the audio to produce a control signal are not necessary". | [DOC 1966] |
| Sidechain | Peak Reduction (R2, 100 kΩ) → 12AX7 (V3) amplificador de tensão → rede de pré-ênfase (R37) → 6AQ5 (V4) driver do EL. | [DOC 1966], [DOC-UA] |
| Feedforward vs feedback | UA: "**The LA-2A is a feed-back style compressor** … the signal that is used to drive the side-chain circuit is affected by the gain reduced signal." Interpretação de fórum: em **Compress** o sidechain é tomado diretamente sobre a célula (feedback puro); em **Limit** é tomado da junção de dois resistores, mistura de feedback e feedforward, o que "simula" razão mais alta. | [DOC-UA manual]; chave: [LORE GroupDIY] |
| Caminho de áudio | Trafo de entrada → T4 → Gain (R1 100 kΩ) → 12AX7 (V1, ganho de tensão ~40 dB) → 12BH7A em "totem pole / double cathode follower" (V2) → trafo de saída. Realimentação global ≈ 20 dB. | [DOC 1966] |
| Link estéreo | Terminais 6 (link) e 7 (terra) interligados: a tensão de controle torna-se **comum** às duas unidades (soma/compartilhamento do drive do EL); "a gain-reduction control voltage generated in either amplifier will cause equal gain reduction in both units". Fio blindado < 2 pés. Calibração por R3 (Stereo Balance, 1 MΩ, traseira) para GR igual com ~5 dB de redução. | [DOC 1966], [DOC-UA] |
| Válvulas | 2× 12AX7A, 1× 12BH7A, 1× 6AQ5 | [DOC 1966], [DOC-UA] |

#### 1.3 Controles

| Controle | Tipo / faixa | Função | Fonte |
|---|---|---|---|
| **Gain** | Potenciômetro contínuo (R1 100 kΩ). Escala 0–100 na reedição UA [?: não documentada no original]. Ganho total do aparelho 40 ± 1 dB. | Ganho de make-up; "does not affect the compression"; ajustar **depois** do Peak Reduction. O manual de 1966 chama de "Gain (input level)". Recomenda-se pad fixo na entrada para operar o Gain em ~50%. | [DOC 1966], [DOC-UA] |
| **Peak Reduction** | Potenciômetro contínuo (R2 100 kΩ). Escala 0–100 na reedição UA [?]. | Ganho do sidechain → controla **ao mesmo tempo limiar e quantidade de compressão** (limiar fixo internamente; o knob muda o drive). | [DOC-UA] |
| **Limit / Compress** | Chave de 2 posições (versões tardias e reedição). | Muda a curva I/O: Compress = curva mais suave e razão menor; Limit = razão mais alta. Sem números oficiais no manual UA. UA webzine: Compress "soft knee, 'fixed' (not really) at a 4:1 ratio", Limit "infinity ratio", "in practice … differences subtle". Pedal UAFX (produto moderno) especifica 4:1 / 12:1. Outra fonte da UA citada em busca fala em ≈3:1 / ≈∞:1 com ressalva de serem não lineares e dependentes da frequência [não lida diretamente]. | [DOC-UA manual], [DOC-UA webzine], [LORE] |
| **Meter** | Chave de 3 posições: +10 / Gain Reduction / +4. | Saída (0 VU = +10 dBm ou +4 dBm) ou GR em dB (usa a 2ª fotocélula). Zero do medidor por trimpot de fenda no painel. | [DOC 1966], [DOC-UA] |
| **R37 — "Gain reduction frequency response control"** (pré-ênfase do sidechain) | Trimpot traseiro, 1 MΩ. Horário = "flat" (de fábrica). | Aumentar a resistência reduz a componente de baixa frequência que chega ao Peak Reduction (C12 0,001 µF passa os agudos). **Máximo: ≈10 dB a mais de redução em 15 kHz do que abaixo de 1 kHz.** Motivo: pré-ênfase FM de 75 µs (+17 dB em 15 kHz). Para música normalmente "flat". | [DOC 1966], [DOC-UA] |
| **R3 Stereo Balance** | Trimpot traseiro, 1 MΩ | Casar GR em par estéreo. | [DOC 1966], [DOC-UA] |

#### 1.4 Ataque e release (célula T4)

| Grandeza | Valor | Fonte / observação |
|---|---|---|
| Ataque (spec original) | "essentially instantaneous" (texto do escaneado ilegível no parêntese, parece "micro-sec.") | [DOC 1966] — é a especificação de época, otimista |
| Ataque (reedição) | "Very fast attack time" | [DOC-UA manual] |
| Ataque médio | **≈10 ms, dependente da frequência** ("frequency dependent attack, with an average attack time of 10 milliseconds") | [DOC-UA webzine 2003] — é a origem do "10 ms" muito citado; **não está no manual original** |
| Dependência de frequência do EL | "The amount of light depends upon the applied voltage **and frequency**." | [DOC 1966] |
| Release — tabela de specs | **≈0,06 s para 50 % de release; 0,5 a 5 s para release completo**, "depending upon amount of previous reduction" | [DOC 1966] e idêntico em [DOC-UA manual] |
| Release — descrição de circuito (mesmo manual de 1966!) | "about **60 milliseconds for 50% release**, and then a gradual release over a period of **1 to 15 seconds** to the point of complete release" | [DOC 1966] — o próprio manual é inconsistente (0,5–5 s vs 1–15 s) |
| Release — UA | Rápido **40–80 ms** até ~metade, restante "over as much as several seconds" ("two-stage decay") | [DOC-UA manual] |
| Memória | Tempo de recuperação depende de **quanto tempo** e **com que brilho** a luz incidiu; release mais lento após compressão longa e/ou profunda. UA webzine: "the T4 really seems [to have] a time constant of **several minutes**" (o medidor pode demorar muito para voltar ao repouso). | [DOC-UA manual], [DOC-UA webzine] (o "vários minutos" é observação, não spec) |
| GR máxima | até 40 dB; "at least 30 dB of gain reduction … with less than 0.5% harmonic distortion" | [DOC 1966] |

Mecanismo físico (para modelagem): no CdS, o "ligar" (luz → queda de resistência) é rápido e fica mais rápido com mais luz; o "desligar" é lento e tem componentes múltiplas devido a armadilhas (recombinação Shockley-Read-Hall com defeitos) — dá origem à memória [PAPER Najnudel et al. 2023]. Ordem de grandeza para vactrols LED/CdS genéricos: dezenas de µs a ms (ligar), centenas de ms (desligar) [PAPER Najnudel 2023]; VTL5C3/2 ≈ 12 ms subida / 250 ms descida (datasheet Perkin-Elmer via [PAPER Parker & D'Angelo 2013]). **Atenção: a T4 usa painel EL, não LED; esses números de vactrol não são da T4.**

#### 1.5 Curva estática

- Figura 1 do manual de 1966 ("Typical gain reduction plot"): ponto de quebra ("breakaway") em **−30 dB de entrada**, compressão que "gradually increases over the first 10 dB of input level rise", e a partir de **−20 dB** a curva fica horizontal: "the input increases an additional 20 dB, but the output increases **less than 1 dB**" [DOC 1966]. Isto é: joelho muito largo (~10 dB) seguido de limitação (razão efetiva > 20:1 no trecho final). É a curva de fábrica (antes da chave Limit/Compress); a posição relativa dos pontos depende do Peak Reduction.
- Razão não é constante: "the compression ratios are nonlinear and frequency dependent" [DOC-UA via busca]; "the ratio will fluctuate depending on the source" [DOC-UA webzine].
- Por ser feedback com atenuador resistivo (R_série 68 kΩ/2,7 kΩ + LDR), a razão aproximada em regime vem da lei R(luz) do CdS e da lei luz(tensão, frequência) do EL — nenhuma das duas está documentada numericamente para a T4 [?].
- Uso recomendado pelo fabricante: "Maximum benefit is obtained by running **4 to 8 DB** of compression continuously"; 20–30 dB contínuos reduzem a dinâmica musical [DOC 1966].

#### 1.6 Coloração, ruído, resposta

| Especificação | Valor | Fonte |
|---|---|---|
| THD | < 0,35 % a +10 dBm; < 0,75 % a +16 dBm (saída) | [DOC 1966], [DOC-UA] |
| THD com GR | < 0,5 % com ≥ 30 dB de GR; "no increase in harmonic distortion" com GR | [DOC 1966] |
| Resposta | ±0,1 dB, 30 Hz – 15 kHz | [DOC 1966], [DOC-UA] |
| Ruído | 75 dB abaixo de +10 dBm (apêndice UA: "better than 70 dB") | [DOC 1966], [DOC-UA] |
| Ganho | 40 ± 1 dB | [DOC 1966], [DOC-UA] |
| Nível | Saída +10 dBm nominal, +16 dBm máx.; entrada +16 dBm máx. | [DOC 1966] |
| Impedâncias | 1966: 50/150/250/600 Ω bal./desbal.; UA: 600 Ω bal. Fonte de saída ≈150 Ω a 800 Hz | [DOC 1966], [DOC-UA] |
| Perfil harmônico | **[?] Não há espectro harmônico publicado em fonte primária.** Inferência de circuito: V1 12AX7 single-ended (tende a 2ª harmônica), saída 12BH7 totem-pole + 20 dB de realimentação global (baixa THD); a fotocélula é resistiva e quase linear na faixa de áudio. Ressalva: em graves, o EL é excitado a 2× a frequência do sinal e a LDR pode seguir parcialmente o ripple → modulação/IMD em baixas frequências com muita GR [LORE/inferência, não verificada]. | — |
| Ruído LF no dataset | O dataset SignalTrain do LA-2A contém "low frequency oscillating noise" na saída, ausente na entrada (os autores usaram filtro DC-block na loss) | [PAPER Wright & Välimäki 2022] |

#### 1.7 Uso típico

- Fabricante: ajustar Peak Reduction no material típico para não passar de 100 % de modulação; 4–8 dB contínuos [DOC 1966].
- Peak Reduction primeiro, Gain depois (independentes) [DOC 1966, DOC-UA].
- R37 em "flat" para música [DOC-UA]; Mike Shipley: com o controle traseiro fora do flat, o LA-2A "rolls off certain frequencies when you reduce more gain" e é útil para vozes estridentes [DOC-UA apêndice, depoimento].
- Uso clássico: voz, baixo; GR pesada sem artefatos audíveis graças ao release com memória [DOC-UA webzine].

#### 1.8 Como se modela (literatura)

| Trabalho | Abordagem | Relevância |
|---|---|---|
| Wright & Välimäki, "Grey-box modelling of dynamic range compression", DAFx-22, pp. 304–311 | Cadeia DSP clássica (gain computer hard/soft-knee + detector one-pole / one-pole com chaveamento ataque-release / one-pole modulado por RNN + make-up estático ou GRU) com parâmetros previstos por MLP a partir do Peak Reduction; treinado no dataset **SignalTrain LA-2A** (modo compress, PR 0–100 em passos de 10, 20 min/ajuste). < 10 % das operações do baseline GRU com precisão similar. | Mostra que um compressor "clássico" com detector de **constante de tempo modulada pelo estado** chega perto do LA-2A real. |
| Hawley, Colburn, Mimilakis, "SignalTrain / Profiling audio compressors with deep neural networks", AES 147 (2019), arXiv:1905.11928 | Black-box + publicação do dataset LA-2A (limit/compress, PR 0–100). | Dataset de referência para validar. |
| Steinmetz & Reiss, "Efficient neural networks for real-time modeling of analog dynamic range compression", AES 152 (2022), arXiv:2102.06200 | TCN causal sobre o dataset LA-2A. | Estado da arte black-box. |
| Simionato & Fasciani, DAFx-22 "Deep learning conditioned modeling of optical compression"; arXiv:2408.12549 (SSM seletivo) | Black-box condicionado; modelam TubeTech CL 1B e LA-2A; ajustes de ataque rápido/release lento são os mais difíceis. | Confirma dificuldade do release lento/memória. |
| Najnudel, Müller, Hélie, Roze, "Power-balanced dynamic modeling of vactrols: application to a VTL5C3/2", DAFx-23 | Modelo físico port-Hamiltoniano do CdS: cargas de elétrons/lacunas com recombinação SRH (eq. 3–4), R = 1/(µ⁺q⁺ + µ⁻q⁻) em paralelo/série com R_dark e R_light (eq. 7–8); acoplamento óptico passivo f(P_D) = P₀P_D^α₀ + P₁P_D^α₁ (dupla inclinação). Ataque e release **dependentes do programa por construção**. Base do plugin comercial UVI "Opal" (que declara modelar a T4 com PHS). | Melhor modelo físico publicado da fotocélula; é LED, não EL, mas a parte CdS se aplica. |
| Parker & D'Angelo, "A digital model of the Buchla lowpass-gate", DAFx-13 | Vactrol heurístico: integrador/one-pole cuja constante muda com o sinal da derivada (≈12 ms subida, ≈250 ms descida para VTL5C3/2) e é **modulada pelo valor atual** (mais rápido quando mais iluminado). | Modelo barato e suficiente para "attack dependente de nível". |
| Eichas & Zölzer, "Modeling of an optocoupler-based audio dynamic range control circuit", Proc. SPIE 9948, 99480W (2016) | Medição da curva estática do AOI + detector com coeficientes ataque/release distintos (inclusive por sinal do semiciclo) + LPF; otimização iterativa. Cita a T4 como 1º AOI, LA-2A controlado pelo sinal de saída. | Metodologia de identificação por medições E/S. |

**Modelo prático sugerido para o LinuxMusic [PROPOSTA — não é dado do hardware]:**

1. Topologia feedback: o sidechain lê a saída do divisor (sinal já atenuado). Para quebrar o loop sem atraso, usar o estado da célula do sample anterior (o loop é lento — a LDR filtra — então o erro é pequeno; com oversampling, menor ainda).
2. Driver EL: `e = PR_gain · HPF_R37(x_atenuado)`; brilho do EL ∝ |e|^β com ganho crescente com a frequência (manual: luz depende de tensão **e** frequência) → isso produz naturalmente o "ataque dependente de frequência".
3. Fotocélula com dois estados: `s_fast` (ataque τ_a ~ 1–10 ms, decrescendo com a luz; release rápido que leva ~60 ms até 50 %) e `s_slow` (release de 0,5–15 s cuja constante **cresce com a exposição acumulada**: um terceiro estado "memória" m que integra brilho·tempo com τ de dezenas de s a minutos e alonga o τ do s_slow). Condutância G = G_dark + k·(s_fast + s_slow)^γ.
4. Ganho: `g = R_série_eq / (R_série_eq + R_série)` invertido para divisor: `g = R_LDR / (R_série + R_LDR)`; R_série = 68 kΩ (R5/R6 da fig. 5 da UA) é ponto de partida.
5. Compress/Limit: em Limit somar uma fração do sinal de **entrada** ao sidechain (parcialmente feedforward), conforme a interpretação de circuito [LORE].
6. Calibrar contra: 50 % de release ≈ 60 ms; release completo 0,5–5 s após GR curta e mais lento após GR longa; joelho ~10 dB e curva quase horizontal após ~10 dB acima da quebra; THD < 0,35 % no caminho limpo. Validar com o dataset SignalTrain.

---

### 2. Fairchild 660 / 670 (limitador vari-mu)

#### 2.1 Visão geral

- Projeto de Rein Narma (anos 1950) [PAPER Raffensperger 2012]. 670 = dois limitadores num chassi, L/R independentes ou "Vertical-Lateral" (M/S) para corte de disco estéreo; 660 = versão mono de um canal [DOC boletim técnico]. Manual de instruções do 670 datado de **dezembro de 1959** ("supersedes all previous issues") [DOC manual 670].
- **Dados do 660 em fonte primária não foram obtidos** — o boletim técnico diz que "the following basic design points apply to both" [DOC], então ataque/release/curva devem ser os mesmos, mas especificações próprias do 660 (ganho, válvulas) estão **[?]**.

#### 2.2 Topologia

| Item | Descrição | Fonte |
|---|---|---|
| Elemento de ganho | Um **único estágio push-pull classe A** de triodos de corte remoto (variable-µ) **6386**, com "extremely high control voltage". Por canal, 4 válvulas 6386 (duplos triodos; V101–V104), em paralelo nas metades push e pull. Total 8× 6386 no 670. | [DOC manual], [DOC boletim], esquema A-96039 |
| Princípio | A tensão de controle (DC) é aplicada em **modo comum** às grades das duas metades; o ganho cai porque o µ/transcondutância do 6386 cai com polarização mais negativa. Em push-pull o deslocamento DC (o "thump") se cancela no trafo de saída — daí "complete absence of audible thumps". | [DOC], [PAPER Raffensperger] |
| Feedforward vs feedback | **Feedback**: o sidechain lê a saída (secundário do trafo de saída T102 → rede resistiva → trafo do sidechain T103). "The Fairchild 670 circuit is a feedback design as the 'sidechain' amplifier monitors the output voltage". | esquema; [PAPER Raffensperger]; [REVIEW SOS] |
| Sidechain (por canal) | T103 (Np/Ns = 1/17 no modelo) → AC Threshold (par de pots 100 kΩ lin.) e DC Threshold (100 kΩ lin., interno) → 12AX7 (V105, 1º estágio **classe B com distorção de crossover intencional regulada pelo DC Threshold**) → 12BH7A (V106) → 2× 6973 push-pull (V107/V108) → trafo T104 (com enrolamento terciário de realimentação para baixa impedância de saída, necessária para carregar os capacitores rápido = ataque rápido) → **retificador em ponte** de diodos de germânio → rede RC de constante de tempo (chave de 6 posições) → grades dos 6386. | esquema; [PAPER Raffensperger] |
| Ganho do sidechain | Ganho nominal dos estágios 2–3 ≈ 8,4× (estimado por simulação), saída limitada a ±100 V; R_out ≈ 160 Ω; corrente máx. ≈ 0,5 A (modelo). | [PAPER Raffensperger] (valores de simulação, não medição) |
| Link estéreo | **Não há link de sidechain no original**: dois limitadores independentes (L/R) ou M/S. Link de sidechain é modificação comum (ex.: unidade Ocean Way modelada pela UA). | [DOC manual], [DOC-UA webzine Dez/2003] |
| Modo Vertical-Lateral | Matriz na entrada (L,R → soma/diferença), limitação independente, matriz de volta na saída. Canal superior = lateral (soma), inferior = vertical (diferença). Separação: 60 dB em L-R, 40 dB em V-L. | [DOC manual] |
| Válvulas (670) | 8× 6386; 1× 6084; 1× 5651; 2× 12AX7; 2× 12BH7; 1× EL34; 4× 6973; 1× GZ34 (5V4). (EL34 = regulador série de B+ a 240 V.) | [DOC manual] |
| Físico | 14" de painel (8U), 11" de profundidade, ≈65 lb; 117 V, 3 A. Aquecimento de ≥ ½ h antes de ajustar. | [DOC manual] |

#### 2.3 Controles (por canal, salvo indicação)

| Controle | Tipo / faixa | Função | Fonte |
|---|---|---|---|
| **Input Gain** | Atenuador em degraus, **1 dB por degrau** (atenuador "T" 600 Ω). Número total de degraus: **[?]** (foto do painel sugere escala ~0–20). Ganho unitário com ≈ **10 dB de atenuação**. No plugin UA, unidade em "18" (convenção do plugin). | Nível no estágio vari-mu (quanto mais entrada, mais GR para o mesmo limiar, e mais distorção). | [DOC manual], [DOC-UA webzine] |
| **(AC) Threshold** | Potenciômetro contínuo no painel (R115, par 100 kΩ lin. com 24 kΩ ao centro → efetivo ~76 kΩ com taper linear por partes). Totalmente anti-horário = sem limitação (amplificador de linha de ganho unitário). | Ganho do sidechain. Ajusta o ponto de **10 dB de limitação**. | [DOC manual], [PAPER Raffensperger] |
| **DC Threshold** | Trimpot de fenda **dentro do chassi** (R117, 100 kΩ lin.). | Ajusta o ponto de **3 dB de limitação**; na prática controla joelho, razão e limiar junto com o AC Threshold (bias do 1º estágio classe B do sidechain). UA (plugin, chamado "DC Bias"): girar horário → razão menor, joelho mais largo e limiar mais baixo. | [DOC manual], [PAPER], [DOC-UA webzine] |
| **Time Constant** | Chave rotativa de **6 posições** | Ver tabela 2.4. | [DOC manual] |
| **Metering** | Chave de 3 posições: BAL / ZERO / BAL (mede corrente de placa de cada metade). Em operação o medidor mostra GR. | Calibração do balanço. | [DOC manual], esquema (S101: 1 balance, 2 zero, 3 balance) |
| **Balance** | Trimpot de fenda no painel (R105, 100 Ω) | Equilibra correntes das metades push/pull; mal ajustado → "thud" de uma polaridade ou da outra nos ataques. UA: mínimo de thump com o ponto da fenda às 12 h. Desbalanços de ~¼ dB não causam problema. | [DOC manual], [DOC-UA webzine] |
| **Zero** | Trimpot (R142, 2,5 kΩ) | Zero do medidor / bias das grades (modelo de Raffensperger: V_bias ≈ −7,2 V; V_bal ≈ −3,1 V). | esquema; [PAPER] |
| **Mode (AGC)** | Chave comum: Left-Right / Vertical-Lateral | Ver acima. | [DOC manual] |
| Saída | **Não há controle de saída** no original (ganho fixo 7 dB). O plugin UA acrescenta. | [DOC manual], [DOC-UA webzine] |

#### 2.4 Constantes de tempo

Tabela do **manual de 1959** (release medido "from 10 db of limiting") [DOC manual]:

| Pos. | Ataque | Release |
|---|---|---|
| 1 | 0,2 ms | 0,3 s |
| 2 | 0,2 ms | 0,8 s |
| 3 | 0,4 ms | 2 s |
| 4 | 0,4 ms | 5 s |
| 5 | 0,4 ms | Automático: 2 s para picos individuais, 10 s para picos múltiplos |
| 6 | 0,2 ms | Automático: 0,3 s para picos individuais, 10 s para picos múltiplos, 25 s para nível de programa consistentemente alto |

Divergência: o "Fairchild tech bulletin" reproduzido pela UA (webzine Dez/2003) e o SOS dão **0,8 ms** de ataque na posição 4; o manual de 1959 dá 0,4 ms. Ataque global: o manual diz "full limiting effect during the first 10,000ths of a second" (sic) e o boletim técnico diz "in the first **5,000th** of a second" (= 0,2 ms) [DOC].

Rede RC da constante de tempo (fig. 8/tabela 3 de Raffensperger, coerente com o esquema: R137 = 220 kΩ fixo; pos. 1 põe 68 kΩ em paralelo → 51,9 kΩ; pos. 2 põe 470 kΩ em paralelo → 149,9 kΩ) [PAPER] [CALC verificação]:

| Pos. | C_T | R_T | Ramo R_U–C_U | Ramo R_V–C_V | τ = R_T·C_T [CALC] | Release doc / τ [CALC] |
|---|---|---|---|---|---|---|
| 1 | 2 µF | 51,9 kΩ | aberto | aberto | 0,104 s | ≈2,9 |
| 2 | 2 µF | 149,9 kΩ | aberto | aberto | 0,30 s | ≈2,7 |
| 3 | 4 µF | 220 kΩ | aberto | aberto | 0,88 s | ≈2,3 |
| 4 | 8 µF | 220 kΩ | aberto | aberto | 1,76 s | ≈2,8 |
| 5 | 4 µF | 220 kΩ | 100 kΩ – 8 µF | aberto | — (multi-polo) | — |
| 6 | 2 µF | 220 kΩ | 100 kΩ – 8 µF | 100 kΩ – 20 µF | — (multi-polo) | — |

Mecanismo do release dependente do programa (pos. 5/6): com nível alto sustentado, C_U e C_V (ramos série R-C) também se carregam; quando o sinal cai, eles realimentam a carga em C_T, alongando o release; um pico isolado só carrega C_T (release rápido) [PAPER Raffensperger]. O ataque é limitado pela corrente que o driver 6973/T104 consegue entregar à capacitância, não por um resistor de ataque explícito [PAPER].

#### 2.5 Curva estática

- Manual: "Variable from 1:1 to 1:20 [i.e. até 20:1] above a predetermined level. Predetermined level factory-adjusted to +2 dBm." [DOC]
- Boletim/manual: pode ser ajustado como **compressor 2:1 com limiar 5 dB abaixo** do nível normal de programa, ou **limitador de pico 30:1 com limiar 10 dB acima**, "or anywhere in-between" [DOC].
- Gráfico "Input vs. Output curves" (dez/1959), leitura visual minha das curvas [CALC, ±0,5 dB]:
  - Curva 1 (AC Thr. todo anti-horário): linear, ganho ≈ unitário com o atenuador em unidade.
  - **Curva 3 (ajuste de fábrica)**: linear até ≈ +2 dBm de entrada; ≈ +4,5 dBm de saída com +10 dBm de entrada (≈3:1 nesse trecho); ≈ +6 dBm de saída com +20 dBm (≈7:1); quase plana depois — **razão crescente com o nível, joelho suave**.
  - Curva 5 (AC todo horário, DC levemente a partir do anti-horário): segue linear até ≈ +8 dBm e satura em ≈ +10 dBm de saída → limitador de joelho relativamente duro.
  - Curva 4 (AC todo horário, DC levemente anti-horário a partir do horário): compressão desde abaixo de −10 dBm, terminando em ≈ 0 dBm de saída → joelho muito largo.
  - Curva 2 (AC levemente horário, DC todo horário): compressão suave, ≈ +11 dBm de saída com +25 dBm de entrada.
- SOS: ≈ 1:1 a 2:1 em picos menores, subindo a ≈ 20:1 em sinais altos; joelho suave [REVIEW].
- Por que a razão cresce: (a) o ganho do 6386 vs. tensão de grade é aproximadamente exponencial/remote-cutoff (não dB-linear); (b) o 1º estágio do sidechain é classe B com zona morta ajustável (DC Threshold), e a ponte de germânio tem limiar ≈ 0,3 V por diodo; (c) topologia feedback [PAPER Raffensperger].

#### 2.6 Coloração, ruído, resposta

| Especificação | Valor | Fonte |
|---|---|---|
| Impedância E/S | 600 Ω / 600 Ω | [DOC manual] |
| Nível de saída | +4 ou +8 VU; clipping em **+27 dBm** | [DOC manual] |
| Ganho | **7 dB** (sem limitação); configurável para ganho unitário | [DOC manual] |
| Resposta | 40 Hz – 15 kHz ±1 dB | [DOC manual] |
| Ruído | 70 dB abaixo de +4 dBm; ruídos de limitação "below audibility" | [DOC manual] |
| Distorção (IM ou harmônica) | < 1 % em qualquer nível até +18 dBm de saída (sem limitação); **< 1 % com 10 dB de limitação e +12 dBm de saída** | [DOC manual] |
| Gráfico de IM (SMPTE 60 Hz + 7 kHz 4:1, mar/1959) | A IM cresce com a GR e com o nível de saída. Leitura visual [CALC]: a 0 dBm de saída fica < 1 % até ≈ 15–16 dB de GR e ≈ 2,5 % perto de 19–20 dB; a +12 dBm de saída ≈ 1 % por volta de 8–10 dB de GR e ~9 % em ≈ 16,5 dB; a +24 dBm de saída começa ≈ 4 % com 0 dB de GR e chega a ≈ 9 % com ≈ 7 dB. Ou seja, **a distorção sobe fortemente com a quantidade de limitação**, e mais cedo quanto maior o nível. | [DOC manual, gráfico] |
| Harmônicos | Push-pull balanceado cancela pares → **predominância de ímpares (3ª)**. Modelo WDF de Raffensperger: tom de 250 Hz a −3,8 dBm → 3ª harmônica **−21 dB** (φ_AC = 0,1) e **−14 dB** (φ_AC = 1,0) relativa à fundamental. **Atenção:** o próprio autor diz que é "large by modern standards" e o modelo não foi validado contra hardware; esses valores são **muito maiores** que a spec de < 1 % do manual → usar só como tendência (distorção ímpar crescente com GR), não como calibração. | [PAPER Raffensperger]; [DOC] para spec |
| Desbalanço | Push-pull fora de balanço → passagem de componente de controle ("thump") e retorno de pares harmônicos [inferência de circuito + DOC-UA sobre o thump]. | — |
| Transformadores | Entrada e saída do estágio de sinal com relação 9:1 / 1:9 (modelo); capacitor de bypass C1 = 4 µF; Lm ≈ 35,7 H (modelo). Determinam a resposta em graves. | [PAPER Raffensperger] (parâmetros de modelo, não medidos) |

#### 2.7 Uso típico

- Manual: ajuste inicial com Input Gain em ganho unitário (~10 dB de atenuação), TC na posição 3, AC Threshold do zero até a limitação desejada. Posição 3 = sugestão de uso geral; **1 ou 2** para música popular ou fala; **4** para música clássica; **5 e 6** para muita correção automática de nível (e "least audible" sob limitação contínua) [DOC manual].
- Lembrete do manual: VU é instrumento de média; picos podem estar até 10 dB acima do "0" [DOC].
- Hoje: posição 1 muito usada em vozes e grupos de voz; "cola" em grupos de metais/teclados/vozes; menos indicado para bus de bateria/mix com muito grave [REVIEW SOS].
- Lateral/vertical para efeitos de imagem ou masterização para vinil; com link de sidechain em M/S, transiente presente nos dois canais comprime um pouco mais [DOC-UA webzine].

#### 2.8 Como se modela (literatura)

- **Raffensperger, "Toward a wave digital filter model of the Fairchild 670 limiter", DAFx-12 (York)** — único modelo de circuito publicado:
  - Modelo black-box do triodo 6386 (ajustado ao datasheet GE de 1953), corrente de placa em A (eq. 1, conferida com extração em layout da Tabela 1):
    `Ia = p1·Vak^p2 / { (p3 − p4·Vgk)^p5 · [p6 + exp(p7·Vak − p8·Vgk)] }`
    com p1 = 3,981e-8, p2 = 2,383, p3 = 0,5, p4 = 0,1, p5 = 1,8, p6 = 0,5, p7 = −0,03922, p8 = 0,2. Corrente de grade desprezada (Vgk < 0). Verificação de sanidade [CALC]: Vak = 250 V → Ia ≈ 20 mA em Vgk = −10 V e ≈ 2 mA em Vgk = −40 V (corte remoto, coerente com a fig. 2 do artigo).
  - Estágio de sinal em WDF (trafos com Rp, Lp, Rs, Ls, Cw, Rc, Lm), 2 válvulas por metade, saída = push − pull. Grades: `Vg = ±V_RL + Vx`, `Vx = Vsc(t−1) + Vbias`, Vbias = −7,2 V.
  - Sidechain: 1º estágio classe B `Vstage1 = ln[(1+e^{Vpot−φ'DC})/(1+e^{−Vpot−φ'DC})]`, φ'DC = 12,2(φDC + 0,1); ganho 8,4 com hard-clip ±100 V; ponte de diodos com transição exponencial suave (λ = 10, Vd = 0,3 V germânio, R_out = 160 Ω); saturação de corrente suave a I_max = 0,5 A; rede RC da tabela acima em WDF.
  - 8× oversampling; o atraso fictício de 1 amostra no loop de feedback do sidechain prejudica a captura de transientes muito agudos (ex.: chimbal).
  - Implementação aberta: **ZamChild670** (zam-plugins, `wavechild670.cpp`, `wdfcircuits.cpp`) baseada neste artigo.
- **Abordagem de nível de comportamento [PROPOSTA]:**
  1. Ganho = função do bias de grade: `g(Vc) = gm(Vg0 + Vc)/gm(Vg0)` com gm de remote-cutoff (aprox. exponencial em Vg), dando razão crescente com o nível.
  2. Distorção: aplicar uma não linearidade **ímpar** (ex.: tanh simétrico/série com só termos ímpares) cuja intensidade cresce com a GR e com o nível de saída, calibrada para < 1 % THD com 10 dB de GR a +12 dBm e crescendo como no gráfico de IM.
  3. Sidechain feedback a partir da saída, retificação de onda completa com zona morta (DC Threshold) e ganho (AC Threshold); carregar a rede RC de 1, 2 ou 3 capacitores conforme a tabela de posições (isso reproduz de graça o release dependente de programa das posições 5/6).
  4. Ataque: limitar a corrente de carga (slew) para obter 0,2–0,4 ms.
  5. Opção "balance" fora do centro: injetar uma fração do sinal de controle (passa-baixas) na saída → reproduz o "thump".

---

### 3. Comparação rápida para o strip

| Aspecto | LA-2A | Fairchild 670 |
|---|---|---|
| Topologia | Feedback (UA); Limit = parte feedforward [LORE] | Feedback (sidechain lê a saída) |
| Elemento | T4: EL + LDR CdS em divisor | 4× 6386 vari-mu push-pull por canal |
| Ataque | "instantâneo" (1966) / ~10 ms médio dependente de freq. (UA) | 0,2–0,4 ms (0,8 ms na pos. 4 segundo o boletim reproduzido pela UA) |
| Release | 50 % em ~60 ms; total 0,5–5 s ou 1–15 s (manual contraditório); memória | 0,3 / 0,8 / 2 / 5 s; auto 2–10 s; auto 0,3–10–25 s |
| Curva | quebra a −30 dB, horizontal 10 dB acima, < 1 dB de subida em +20 dB | 1:1 → 20:1 acima de +2 dBm (fábrica); 2:1 a 30:1 conforme AC/DC threshold |
| THD | < 0,35 % @ +10 dBm; < 0,5 % com 30 dB GR | < 1 % até +18 dBm; < 1 % com 10 dB GR @ +12 dBm; cresce forte com GR |
| Harmônicos | [?] (inferência: 2ª da 12AX7, baixa por feedback) | Ímpares (push-pull) |
| Ruído | −75 dB ref. +10 dBm | −70 dB ref. +4 dBm |
| Resposta | ±0,1 dB 30 Hz–15 kHz | ±1 dB 40 Hz–15 kHz |
| Ganho | 40 dB | 7 dB |

### 4. Lacunas conhecidas

- Curvas R(luz) e luz(tensão, frequência) da T4/T4B: **não publicadas**; nenhum artigo encontrado com medições da T4 especificamente (os modelos físicos usam vactrols LED VTL5C3/2).
- Valores exatos do circuito da chave Limit/Compress do LA-2A: só interpretação de fórum (GroupDIY, página bloqueada para leitura direta).
- Espectro harmônico medido de LA-2A e Fairchild reais: **não encontrado** em fonte primária.
- Faixa total do atenuador de entrada do 670 e escala do AC Threshold: [?].
- Especificações próprias do 660: [?].
- Parâmetros do modelo de Raffensperger (trafos, ganho do sidechain, Vbias) vêm de simulação/esquema, **não** de medição de hardware; o próprio autor diz que o modelo precisa ser validado contra uma unidade real.

### Fontes

Primárias (fabricante):
- Teletronix, *Leveling Amplifier Model LA-2A Instruction Manual* (rev. 3-66, esquema rev. 3-18-66) — https://www.steampoweredradio.com/pdf/teletronix/Teletronix%20LA-2A%20Leveling%20Amplifier%20Circa%201966.pdf
- Universal Audio, *LA-2A User's Guide* rev. 1.3 (2000) — https://web.archive.org/web/20061113094825/http://www.uaudio.com/_works/pdf/manuals/LA-2A_manual.pdf
- Fairchild Recording Equipment Corp., *Model 670 Stereo Limiter Instruction Manual* (dez/1959, com curvas e esquema A-96039) — http://thehistoryofrecording.com/Manuals/Fairchild/Fairchild_670_stereo_limiting_amplifier_instructions_Schematics.pdf (lido via web.archive.org)
- Fairchild, *Technical Bulletin Model 660, 670* — http://thehistoryofrecording.com/Literature/Fairchild/Fairchild_670_Datasheet.pdf (lido via web.archive.org)
- Página índice: https://www.historyofrecording.com/fairchild670.html

UA (secundária forte):
- W. Shanks, "Compression Obsession: The Amazing Release Character of the LA-2A", UA WebZine v1n3, jun/2003 — https://web.archive.org/web/20241217221906/https://www.uaudio.com/webzine/2003/june/text/content4.html
- W. Shanks, "Fairchild 670: Plugged in or Plug-In", UA WebZine v1n9, dez/2003 — http://www.uaudio.com/webzine/2003/december/text/content4.html (via web.archive.org)

Artigos:
- P. Raffensperger, "Toward a Wave Digital Filter Model of the Fairchild 670 Limiter", DAFx-12 — https://www.dafx.de/paper-archive/2012/papers/dafx12_submission_9.pdf
- J. Najnudel, R. Müller, T. Hélie, D. Roze, "Power-Balanced Dynamic Modeling of Vactrols: Application to a VTL5C3/2", DAFx-23 — https://www.dafx.de/paper-archive/2023/DAFx23_paper_50.pdf
- J. Parker, S. D'Angelo, "A Digital Model of the Buchla Lowpass-Gate", DAFx-13 — https://www.dafx.de/paper-archive/2013/papers/44.dafx2013_submission_56.pdf
- F. Eichas, U. Zölzer, "Modeling of an optocoupler-based audio dynamic range control circuit", Proc. SPIE 9948 (2016) — https://www.hsu-hh.de/ant/wp-content/uploads/sites/699/2017/10/Eichas-Modeling-of-an-optocoupler-based-audio-dynamic-range-control-circuit-99480W.pdf
- A. Wright, V. Välimäki, "Grey-Box Modelling of Dynamic Range Compression", DAFx-22 — https://acris.aalto.fi/ws/portalfiles/portal/89253381/Wright_et_alii_GREY_BOX_MODELLING_OF_DYNAMIC_RANGE_COMPRESSION.pdf
- R. Simionato, S. Fasciani, "Modeling Time-Dependent Responses of Optical Compressors with Selective State Space Models" — https://arxiv.org/abs/2408.12549
- C. Steinmetz, J. Reiss, "Efficient neural networks for real-time modeling of analog dynamic range compression" — https://arxiv.org/abs/2102.06200 (citado; não lido na íntegra)
- S. Hawley et al., "Profiling audio compressors with deep neural networks" (SignalTrain) — arXiv:1905.11928 (citado via Wright 2022; não lido na íntegra)
- Implementação aberta ZamChild670 — https://sources.debian.org/src/zam-plugins/4.2%2Bds-1/plugins/ZamChild670/wavechild670.cpp

Revisões / outras:
- Sound On Sound, "Fairchild 660 & 670" — https://www.soundonsound.com/reviews/fairchild-660-670
- Wikipedia, "LA-2A Leveling Amplifier" — https://en.wikipedia.org/wiki/LA-2A_Leveling_Amplifier
- GroupDIY, "Please explain LA-2 limit compress switch" — https://groupdiy.com/threads/please-explain-la-2-limit-compress-switch.79347 (só trechos via busca; página bloqueada)
- Vintage King, "Teletronix LA-2A" (T4A→T4B ~nº 1640) — https://vintageking.com/pages/teletronix-la-2a-optical-compressor-limiter (só via busca)
- UVI Opal (declara modelar a T4 por PHS) — https://www.uvi.net/opal

---

## Anexo D: Compressores variable-mu vintage: Gates Sta-Level (e Level Devil), Altec 436 (A/B/C) e RCA BA-6A

Pesquisa para modelar esses aparelhos no channel strip analógico do LinuxMusic.
Data: 2026-11-12.

### Convenções de confiabilidade

Cada número vem marcado com a sua origem:

| Marca | Significado |
|---|---|
| **[DOC]** | Documento original do fabricante: manual, spec sheet, catálogo ou artigo técnico do projetista. |
| **[DOC-2]** | Fonte secundária séria: artigo histórico técnico ou review de revista (SOS, Radio World, Mix). |
| **[DER]** | Calculado por mim a partir de valores [DOC], como RC, ajuste de curva ou leitura de gráfico. |
| **[EST]** | Estimativa ou princípio de engenharia sem medição publicada para o aparelho. |
| **[LORE]** | Fórum, texto de loja ou boca a boca. É útil para "caráter", mas não serve como especificação. |
| **DESCONHECIDO** | Procurei e não encontrei. |

---

### 0. Resumo comparativo

| | Gates Sta-Level M5167 (1956) | Altec 436C (anos 60) | RCA BA-6A (1950/51) |
|---|---|---|---|
| Elemento de ganho | 1× 6386, duplo triodo remote-cutoff, push-pull [DOC] | 1× 6BC8, duplo triodo, push-pull [DOC/esquema 436A] | 2× 6SK7, pentodos remote-cutoff, push-pull [DOC] |
| Detector | 6AL5 (duplo diodo) retificando a saída 6V6 [DOC] | 6AL5 retificando a saída 6CG7 [DOC/esquema] | 6H6 (duplo diodo) retificando a saída 6V6 [DOC] |
| Topologia | **Feedback**: detector na saída, controle no 1º estágio [DOC] | **Feedback** [DOC/esquema] | **Feedback** [DOC] |
| Ataque | 75 ms (Single), 25 ms (Double) [DOC] | 50 ms fixo [DOC] | 0,6 ms (Single e Dual-fast), 0,9 s (Dual-slow) [DOC] |
| Release (63%) | 2,25 a 11,25 s; padrão 7,05 s (Single). Double: 0,75 a 3,75 s (fast), 0,8 a 12 s (slow), dependente do programa [DOC] | 0,3 a 1,3 s ajustável [DOC] | 0,33 s (Single), 2 s (Dual, picos sustentados) [DOC] |
| Razão | ~3,3:1 de fábrica; 1,6:1 a 6:1 por resistor [DOC] | 2:1 com threshold 0 dBm, 4:1 com +16 dBm [DOC] | 20 dB → 2 dB (~10:1) [DOC] |
| GR máximo | 30 dB nominal, até 40 dB [DOC] | 30 dB [DOC] | Sem número; 15 a 20 dB recomendados [DOC] |
| THD | ≤1% de 50 Hz a 15 kHz, com 0 a 30 dB de compressão [DOC] | <1,5% com 25 dB de GR; <2,5% com 30 dB de GR [DOC] | <1% com 12 dB de GR; <0,6% sem GR [DOC] |
| Resposta | ±1 dB de 30 Hz a 15 kHz, com 0 a 30 dB de GR [DOC] | ±1,5 dB de 30 Hz a 15 kHz [DOC] | ±1 dB abaixo do limiar; +1/−2 dB até 20 dB de GR [DOC] |
| Ruído | ≥65 dB abaixo de +8 dBm [DOC] | 74 dB abaixo da saída nominal; EIN −111 dBm [DOC] | 83 dB abaixo do verge of limiting [DOC] |

Todas as três unidades são **compressores feedback**: o detector lê a saída do amplificador e
devolve um bias negativo à grade do tubo remote-cutoff de entrada. Essa é a escolha de
topologia mais importante para o modelo. Por causa dela, ratio e knee nascem do laço, e não
de uma curva estática imposta.

---

### 1. Gates Sta-Level, M5167

#### 1.1 Identificação e histórico
- Fabricado pela Gates Radio Company, Quincy (Illinois). O manual é o IB-727, de 30/04/1956 [DOC].
- Lançado em 1956. Derivou do GE BA-9B "Uni-Level" (1955), mas foi projetado para não infringir a patente da GE [DOC-2, Telos].
- O próprio manual o descreve como "automatic gain amplifier designed to keep a constant
  average level", e não como limitador de pico instantâneo. Ele deve ser seguido de um peak limiter [DOC].
- O manual se descreve como "gated automatic gain type". Duas fontes secundárias (Radio World e
  Telos) afirmam que o Sta-Level **não** tinha gating e que, sem sinal, subia ao ganho máximo e
  trazia junto ruído e hum. Segundo elas, o gating de verdade (congelar o ganho na ausência de
  áudio) veio com o **Level Devil** [DOC-2]. No manual, "gated" parece referir-se ao limiar de
  retificação polarizado (o retificador só conduz acima de +25 V). Isso é **inferência minha**.

#### 1.2 Especificações de fábrica [DOC, manual M5167, p. 1]

| Item | Valor |
|---|---|
| Ganho | 35 dB ±2 dB com pads de entrada e saída; 62 dB ±2 dB sem pads |
| Distorção | ≤1% de 50 Hz a 15 kHz, com 0 a 30 dB de compressão lida no medidor |
| Resposta | ±1 dB de 30 Hz a 15 kHz, com 0 a 30 dB de compressão |
| Ruído | ≥65 dB abaixo de +8 dBm (ou de saída maior), em qualquer ganho |
| Razão e limiar | ≈3,3:1. Limiar de **+10 a +12 dBm na saída**, com o controle de saída no máximo |
| Impedâncias | 500/600 Ω de fábrica; 150/250 ou 500/600 Ω pelos taps dos transformadores |
| Tubos | 2× 6V6, 1× 6386, 1× 12AT7, 1× 6AL5, 1× OB2, 1× 5Y3GT |
| Pad de entrada | ~20 dB, tipo H balanceado. Aceita até +24 dBm com o pad; sem o pad, até +4 dBm |
| Pad de saída | 10 a 16 dB, com seção variável de 6 dB, que é o controle "output" |
| Sem pads | Limiar mínimo de entrada ≈ −44 dBm. Meio de faixa (15 dB de GR) ≈ −30 dBm na entrada e ≈ +24 dBm na saída |

#### 1.3 Topologia [DOC, "Principles and theory of operation"]
- **Caminho de sinal:** transformador de entrada → V1, uma 6386 com as duas seções em push-pull
  (estágio de ganho variável) → V2, uma 12AT7 em push-pull → V3/V4, 6V6 em push-pull → transformador de saída.
  Estágios acoplados por RC.
- **Realimentação negativa global** vai das placas das 6V6 aos catodos de V2. O manual diz que ela reduz
  ruído e distorção e torna a seleção de tubos menos crítica. V1 fica fora dessa realimentação: é
  "premium tube with good balance between sections".
- O manual diz que o push-pull é usado do início ao fim para **cancelar o "bias thump"** (o transiente
  de baixa frequência causado pela mudança de bias) e evitar motor-boating.
- **Sidechain:** a saída das 6V6 vai aos catodos da V6 (6AL5), que tem bias de **+25 V**. Só há
  retificação quando o pico do sinal passa de 25 V, ou seja, ≈17,5 V RMS. Esse é o **limiar** (delay
  bias). A tensão retificada é filtrada pela rede RC e aplicada ao retorno de grade da V1.
- Segundo o manual, a V5 (OB2) regula 105 V para que o ponto de bias não varie com a rede elétrica.
  O artigo da Telos atribui ao OB2 os 25 V do limiar.
- Radio World e Telos (artigos [DOC-2]) dizem que a 12AT7 "is superfluous" e que retirá-la era uma
  modificação comum para reduzir ruído e distorção [DOC-2/LORE].

#### 1.4 Bias de controle × redução de ganho [DOC, p. 7]

O manual tabela a tensão na grade da V1 para cada nível de compressão:

| GR (dB) | 5 | 10 | 15 | 20 | 25 | 30 | 35 | 40 |
|---|---|---|---|---|---|---|---|---|
| Bias (V) | −2,4 | −6 | −11,6 | −19 | −30,5 | −43 | −59 | −76 |

- O manual nota que essa relação não é linear: a curva sobe mais nos níveis altos de compressão. Isso vem
  do desenho A-11734, que não está no PDF consultado.
- **[DER] Ajuste:** GR(dB) ≈ 16,6 · ln(1 + |Vc| / 8,1 V). O erro máximo é ≈1,1 dB entre 0 e −76 V.
  Esta é a "lei de ganho" do tubo 6386 neste circuito, pronta para virar uma lookup table ou uma fórmula.
- O artigo da Telos menciona um potenciômetro opcional no bias de catodo da 6386 que cria um "porch",
  limitando a faixa de GR para reduzir o bombeamento de ruído [DOC-2].

#### 1.5 Curva estática e razão [DOC, p. 7]
- Razão de **3,3:1** com a V6 polarizada em +25 V (R32 = 24 kΩ).
- Aumentar R32 eleva o bias da V6 e leva a razão **até 6:1**. Os níveis de entrada e de saída
  precisam subir, e **a distorção aumenta**.
- Reduzir R32 abaixo de +25 V leva a razão **até 1,6:1**: 16 dB a mais na entrada viram 10 dB a mais na saída.
- Explicação do manual: o bias retificado é função da **tensão RMS**, que cresce de forma logarítmica
  com o nível em dB. Com um bias de partida maior, a mesma variação em dB precisa de uma variação
  relativa menor de tensão. Assim, **limiar mais alto implica razão maior**. O Altec 436C mostra o
  mesmo comportamento (ver 2.3). É uma assinatura dos variable-mu feedback com retificador polarizado.
- Na prática, segundo o manual: com o medidor em 15 dB de GR, uma entrada 15 dB abaixo do normal sai
  ~4 dB baixa, e uma entrada 15 dB acima sai ~4 dB alta. **[DER]** 30 dB de variação na entrada viram
  ~8 dB na saída, ≈3,75:1 nessa região.
- Faixa: 30 dB de GR é a faixa normal. Ele "will actually compress up to 40 dB with very low distortion" [DOC].
- Knee: o manual não fala em knee. **[DER/EST]** O knee é macio por três motivos: (a) o retificador
  polarizado conduz gradualmente; (b) a lei de ganho da 6386 é logarítmica em |Vc|; (c) há o laço de
  feedback. Pela tabela, a razão local cai de ~2 dB/V para ~0,3 dB/V.

#### 1.6 Ataque e release [DOC, pp. 2, 5, 6 e 8]

**Rede de tempo.** C10 = 0,5 µF; C11 = 1,0 µF; R35 = 56 kΩ (carga, ou seja, ataque); R36 = 4,7 MΩ
e R37 = 10 MΩ, que são os valores de fábrica marcados com \*. O manual calcula o ataque com 0,05 MΩ
em vez dos 56 kΩ de R35.

**Chave "Single / Double"** no painel frontal:

- **Single:** C10 ∥ C11 = 1,5 µF.
  - Ataque = 0,05 MΩ × 1,5 µF = **75 ms**. O manual diz "at least 75 milliseconds is required to cause compression".
  - Release = R36 × 1,5 µF = 4,7 × 1,5 = **7,05 s até 63%**, e **28,2 s até 90%**.
  - O release fica fixo, quase sem depender do programa.
  - O manual o indica para música clássica e para o que exige a maior faixa dinâmica.
- **Double**, o modo dependente do programa:
  - Ataque = 0,05 × 0,5 µF = **25 ms**. C11 não interfere no ataque.
  - Com picos curtos e isolados, C11 quase não carrega e a recuperação é rápida.
  - Com picos sustentados, C11 carrega por R37 e precisa descarregar por R37 + R36, o que dá uma recuperação lenta.
  - O manual: "speech and other short occasional peaks have a rapid recovery time and the average level
    of modulation is kept very high. Music with long passages… slower recovery time automatically".
  - Indicado para fala e para a maior parte da música popular.

**Tabela oficial de recuperação [DOC, p. 2]:** tempos medidos, em segundos. A linha \* é o padrão de fábrica.

| R36 (MΩ) | R37 (MΩ) | Single 63% | Single 90% | Double-slow 63% | Double-slow 90% | Double-fast 63% | Double-fast 90% |
|---|---|---|---|---|---|---|---|
| 1,5 | 1,5 | 2,25 | 10,0 | 2,0 | 8,0 | 0,75 | 3,0 |
| 1,5 | 10 | 2,25 | 10,0 | 0,80 | 3,0 | 0,75 | 3,0 |
| 3,3 | 1,5 | 4,95 | 19,8 | 5,5 | 17,0 | 1,65 | 6,6 |
| 3,3 | 3,3 | 4,95 | 19,8 | 4,0 | 21,0 | 1,65 | 6,6 |
| 3,3 | 15 | 4,95 | 19,8 | 2,1 | 12,0 | 1,65 | 6,6 |
| 4,7 | 1,5 | 7,05 | 28,2 | 7,0 | 22,0 | 2,35 | 9,4 |
| 4,7 | 4,7 | 7,05 | 28,2 | 4,2 | 25,0 | 2,35 | 9,4 |
| **4,7\*** | **10\*** | **7,05** | **28,2** | **3,1** | **28,0** | **2,35** | **9,4** |
| 4,7 | 20 | 7,05 | 28,2 | 2,5 | 14,0 | 2,35 | 9,4 |
| 7,5 | 1,5 | 11,25 | 45,0 | 12,0 | 37,5 | 3,75 | 15,0 |
| 7,5 | 10 | 11,25 | 45,0 | 7,1 | 45,0 | 3,75 | 15,0 |
| 7,5 | 20 | 11,25 | 45,0 | 4,5 | 36,0 | 3,75 | 15,0 |

A tabela original tem 23 linhas. Aqui está um subconjunto representativo, com todas as linhas de R36
e os extremos de R37.

**Observações para o modelo:**
- **[DER]** O 90% medido é ≈4× o 63% em todas as linhas Single. Para um decaimento exponencial puro
  de uma tensão, os 90% chegariam em 2,3τ. A diferença é coerente com a **lei de ganho não linear**
  (§1.4): o último dB de GR corresponde a uma fração grande da tensão de bias, então "os últimos dB
  demoram mais". A review da SOS sobre o Retro Sta-Level relata exatamente isso: "the last few dB
  take longer to clear" [DOC-2]. **No modelo, a recuperação deve ser um decaimento exponencial da
  tensão de controle passado pela lei de ganho, e não um decaimento em dB.**
- "Double-fast" é o caso com C11 descarregado: τ = R36 × 0,5 µF; por exemplo, 4,7 × 0,5 = 2,35 s ✓.
  "Double-slow" é o caso com C11 carregado depois de picos sustentados.
- Faixas com resistores de fábrica: Single **2,25 a 11,25 s** (63%); Double-fast **0,75 a 3,75 s**.
  Os tempos são muito longos para os padrões de estúdio de hoje: é um AGC.

#### 1.7 Controles

| Controle | Original M5167 [DOC] | Observação |
|---|---|---|
| Input | Pot duplo de 100 kΩ (R1) no painel. Ajuste: o medidor de compressão deve picar em 15 (linha grossa) | Contínuo, sem marcação em dB |
| Output | Seção variável de 6 dB do pad de saída (R38, 10 kΩ). Pad total de 10 a 16 dB | Contínuo |
| Recovery | Chave "Single / Double" no painel | O tempo base se troca por resistores internos (R36/R37) |
| Medidor | "Compression meter", 0 a 30+ dB de GR, com zero ajustável atrás do painel | Calibrado para o limiar de fábrica |
| Threshold / ratio | **Não há** controle de painel. Mudam pelo R32 interno; o manual desaconselha pô-los no painel | Limiar fixo, como no LA-2A (operação de dois botões) |
| Ataque | Não há controle; é fixado pela posição Single/Double | |

O clone Retro Instruments Sta-Level acrescenta um modo Triple e uma chave rotativa de release de seis
posições. Usa duas 6BJ6 ligadas como triodo no lugar da 6386, com soquete opcional para a 6386 [DOC-2, SOS].

#### 1.8 Coloração
- THD ≤1% com até 30 dB de GR [DOC]. **Perfil harmônico: DESCONHECIDO**; não achei medições.
  **[EST]** Com push-pull bem balanceado, os harmônicos pares se cancelam e o desbalanço da 6386 sob
  bias variável gera pares. A distorção sobe com o GR e com ratios mais altos, como o próprio manual
  afirma no caso de 6:1.
- Transformadores de entrada e de saída: Triad nos originais. O artigo da Telos diz que os UTC eram considerados melhores [DOC-2].
- "Thump" subsônico em transientes com ataque rápido e GR alto; a review da SOS sobre o clone sugere um low-cut depois do aparelho [DOC-2].
- Graves pesados "quebram" (saturam) antes do esperado em transientes. A SOS descreve o caráter
  como "thick, velvety, gooey" [DOC-2, clone].

#### 1.9 Level Devil, M-5546 (1958) [DOC, manual M-5546]

| Item | Valor |
|---|---|
| Impedâncias | 600 Ω de entrada e de saída |
| Entrada | −35 VU a +27 VU, com pads H de 10 e 20 dB |
| Saída | +14 VU no transformador; +8 VU nos terminais, com pad H de 6 dB |
| Distorção | ≤1% de 50 Hz a 15 kHz até 10 dB de limiting; ≤2% até 25 dB |
| Resposta | ±1 dB de 30 Hz a 15 kHz |
| Ruído | −60 dB ou melhor, a partir de 10 dB de limiting |
| **Expansão máxima** | **10 dB** |
| **Limiting máximo** | **25 dB**, com menos de 2% de distorção |
| **Ataque do limiter** | **10 ms** |
| **Release do limiter** | **1,5 a 2,0 s** |
| **Subida do expansor** | **2,0 s** |
| **Release do expansor** | **4,0 a 5,0 s** |
| Ganho | 50 dB sem pads |
| Tubos | 2× 5749, 2× 12AU7, 2× 12AT7, 2× 12AX7, 2× OB2, 1× EF86, 1× 5V4G |

- O Level Devil combina AGC com expansor. Até 10 dB de "expansão" eleva o ganho de programas fracos.
  Abaixo de −10 VU relativo, o medidor volta ao meio da escala com −10 dB, condição de "no signal".
  Esse é o **gating** que impede o ganho de subir no silêncio. O expansor pode ser desligado pela S3 [DOC].
- O único controle de operação é o atenuador de entrada AT3, atrás do painel [DOC].
- **[EST]** A 5749 é a versão industrial da 6BA6, um pentodo remote-cutoff, e provavelmente é o
  elemento de ganho variável. Não confirmei isso no esquema.

---

### 2. Altec 436 (A / B / C)

#### 2.1 Variantes

| Variante | Data | Controles | Fonte |
|---|---|---|---|
| **436A** | ~1956, data com "?" no Radiomuseum | Só volume de entrada. Limiar e tempos fixos | Mix e MusicTech [DOC-2]; Technical Letter #10 da Altec [DOC] |
| **436B** | ~1958 | Acrescenta controle de ganho de entrada. Tempos fixos | MusicTech [DOC-2] |
| **436C** | anos 60, Altec já subsidiária da Ling-Temco-Vought | Ganho, **threshold**, **release time**, power, medidor de dB de compressão | Spec sheet AL-1401-4 [DOC] |

- Os tubos 6BC8, 6CG7 e 6AL5 são os mesmos no 436A (esquema da Altec) e no 436C (spec sheet) [DOC].
- **Os valores de limiar, ataque e release específicos do 436A e do 436B: DESCONHECIDOS.** O 50 ms costuma
  ser repetido para a família, mas só está documentado para o 436C.
- O EMI **RS124** é um 436B modificado pela EMI a partir de 1959. Ganhou atenuador de saída e chave
  "recovery" de seis posições com posições "Hold" intermediárias, que desconectam o capacitor de tempo
  dos resistores de release [DOC-2/LORE]. Sabe-se que as constantes de tempo variam de unidade para
  unidade em Abbey Road. Os valores exatos são DESCONHECIDOS.

#### 2.2 Especificações do 436C [DOC, spec sheet AL-1401-4]

| Item | Valor |
|---|---|
| Ganho | 56 dB com fonte de 15 kΩ; 40 dB em ponte sobre linha de 600 Ω (o texto "Architects & Engineers" diz 42 dB) |
| Resposta | ±1,5 dB de 30 Hz a 15 kHz |
| Potência | +24 dBm como amplificador puro |
| THD | <1,5% de 35 Hz a 15 kHz com **25 dB** de compressão; <2,5% de 25 Hz a 10 kHz com **30 dB** de compressão (o texto A&E diz 35 Hz a 10 kHz), com threshold em 0 dB |
| Ruído | 74 dB abaixo da saída nominal; −111 dBm de ruído equivalente de entrada |
| Impedância de entrada | 15 kΩ, transformador bridging não aterrado |
| Fonte | Até 15 kΩ |
| Carga | 150/600 Ω |
| **Compressão máxima** | **30 dB** |
| **Ataque** | **50 ms, fixo** |
| **Release** | **0,3 a 1,3 s até 63% de recuperação**, ajustável |
| **Threshold** | **0 a +16 dBm de saída**, ajustável |
| **Razão** | **2:1 com threshold em 0 dBm; 4:1 com +16 dBm** |
| Controles | Gain, Threshold, Release Time, Power. Threshold e release têm trava de eixo |
| Tubos | 6BC8, 6CG7, 6AL5 |
| Função extra | Sem a 6AL5, vira amplificador de linha comum |
| Alimentação | 117 V, 60 Hz, 20 W |

#### 2.3 Topologia [DOC, esquema 6190 do 436A]
- O esquema da Technical Letter #10 mostra o 436A **modificado** (TB-103 na entrada, divisor de
  200 kΩ/15 kΩ para um limiar maior, ganho de 58 dB):
  - **V1, 6BC8:** as duas seções triodo em **push-pull**. As grades recebem o secundário com center-tap
    do transformador de entrada, e o bias de controle entra no center-tap. Cargas de placa
    R3 = R4 = 47 kΩ ±1%.
  - **V2, 6CG7:** push-pull, alimentando o transformador de saída 16402.
  - **V3, 6AL5:** retificador de onda completa. Alimentado das **placas da 6CG7** (saída) por
    C5/C6 = 0,022 µF e R10/R11 = 220 kΩ. **Topologia feedback.**
  - O **limiar** é um delay bias no catodo da 6AL5: +110 V no divisor de 200 kΩ/15 kΩ da versão
    modificada. No 436C, o pot de threshold faz esse papel.
  - Filtro de controle: R9 = 1 MΩ com C4. **O valor de C4 está ilegível no scan.**
  - O medidor M1 mede a corrente de catodo da 6BC8 (shunt R8 de 34 Ω).
- **Tensões com 20 dB de compressão**, valores do esquema entre parênteses:
  - Placa da 6BC8: 65 V sem compressão, **285 V** com compressão.
  - Bias de grade: −0,5 V sem compressão, **−38 V** com compressão.
  - **[DER]** Na 6BC8 deste circuito, cerca de −37,5 V de controle produzem 20 dB de GR. É o único ponto
    da lei de ganho que encontrei; a curva completa é DESCONHECIDA. Para comparar: a 6386 do Sta-Level
    precisa de −19 V para os mesmos 20 dB.
- Do mesmo jeito que no Sta-Level, o 436C tem **razão maior quando o threshold é maior** (2:1 → 4:1).
  É o mecanismo do delay bias somado ao feedback.

#### 2.4 Controles (436C)

| Controle | Faixa | Tipo |
|---|---|---|
| Gain ("Volume") | Ganho de entrada contínuo. O painel mostra marcas de 0 a 10 | Pot |
| Threshold | 0 a +16 dBm de saída | Pot com trava |
| Release Time | 0,3 a 1,3 s | Pot com trava |
| Ataque | 50 ms fixo | Não existe |
| Medidor | "dB Compression", 0 a 30 dB | VU |

#### 2.5 Coloração
- THD de 1,5% com 25 dB de GR e 2,5% com 30 dB [DOC]. É mais distorção que no Sta-Level e no BA-6A,
  o que combina com um circuito mais simples, de 3 tubos, e sem realimentação global.
- **[EST]** O 436A não tem NFB global em torno da 6BC8 no esquema. Um fórum (GroupDIY) diz que a 6BC8 e a
  6CG7 trabalham em push-pull "para cancelar a distorção considerável" de usar a 6BC8 desse jeito [LORE].
- A Mix (restauração de um 438) cita 40 Hz a 10 kHz ±1,5 dB e 2,5% com 30 dB; o 438 é o irmão com pré
  de microfone [DOC-2]. A Preservation Sound relata "fairly flat 40-15K" e que o aparelho acrescenta
  ~20 dB ou mais de nível quando comprime forte. Não há controle de saída [DOC-2].
- Perfil harmônico (2º × 3º): DESCONHECIDO.

#### 2.6 Uso típico hoje
- Baixo: Geoff Emerick passava o baixo de McCartney por RS124 (436B modificados), às vezes duas ou três
  vezes. Era preferido ao Fairchild 660 para baixo [DOC-2, MusicTech].
- Bus: nos grupos I e II da REDD.51 [DOC-2].
- Joe Meek usava 436A e 436B modificados para tempos curtos e bombeamento pesado [DOC-2].
- A modificação comum documentada em fórum chega a ~30 ms de ataque e 200 ms de release mínimos [LORE].

---

### 3. RCA BA-6A

#### 3.1 Identificação
- Descrito por C. A. Singer (RCA) em *Audio Engineering*, novembro de 1950: "Performance and Operation
  of a New Limiting Amplifier" [DOC]. Anunciado em *Broadcasting* em 1951 [DOC]. Aparece no catálogo RCA
  Broadcast Equipment, 3ª ed., 1957, pp. 57–58, MI-11225 [DOC].
- Foi feito para os canais de entrada de fala de transmissores AM, FM e de som de TV, e também para
  gravação, onde evita sobrecorte do disco [DOC].

#### 3.2 Especificações [DOC, catálogo RCA 1957, p. 58]

| Item | Valor |
|---|---|
| Impedância de fonte e de entrada | 600/150 Ω, balanceada ou não |
| Resposta (ref. 1 kHz) | ±1 dB de 30 Hz a 15 kHz abaixo do limiar; **+1/−2 dB até 20 dB de GR** |
| Nível de entrada | −24 dBm mínimo (no verge of limiting); +14 dBm máximo |
| Saída máxima (limiting off, 1 kHz) | 38,5 dBm (amplificador de 10 W) |
| Saída no verge of limiting | 29,5 dBm ±1 dB, controles de saída no mínimo de atenuação |
| Ganho | 54 dB ±1 dB a 1 kHz, de 600 Ω para 600 Ω |
| Controle de entrada | 20 passos de 2 dB |
| Controle de saída | 20 passos de 1 dB, mais ajuste fino |
| S/R | 83 dB no verge of limiting |
| THD (RMS total) | <1% de 100 Hz a 15 kHz com 12 dB de GR; sem GR e com 30 dBm na saída: <0,6% de 50 Hz a 15 kHz e <1,2% a 30 Hz |
| Limiar | 29,5 ±0,5 dBm de saída |
| **Razão acima do limiar** | **20 dB entram, 2 dB saem** (≈10:1). O artigo de 1950 diz "greater than 10 to 1"; a Fig. 7 mostra 20 dB → 1,9 dB |
| Tubos | 2× 6SK7, 2× 6J7, 2× 6V6-GT, 1× 6H6, 1× OD3/VR150, 1× 5R4GY |
| Consumo | 105 W |

#### 3.3 Constantes de tempo [DOC, catálogo 1957 e artigo de 1950]

| Modo (chave BAL-LIM) | Ataque | Release |
|---|---|---|
| Single | 600 µs | 0,33 s |
| Dual, fast action | 600 µs | 0,33 s |
| Dual, slow action | 0,9 s | 2 s |

**Circuito** (artigo de 1950, Fig. 4):
- **Single:** C1 = 0,1 µF com R11 = 3,3 MΩ. C1 carrega muito rápido (0,0006 s) e descarrega mais devagar (0,33 s).
  **[DER]** 3,3 MΩ × 0,1 µF = 0,33 s ✓. Os 0,6 ms de carga implicam uma impedância de fonte do
  retificador de ≈6 kΩ. Isso é inferência, não há documento.
- **Dual:** C18 em série com R37 = 3,3 MΩ, os dois em paralelo com C1. C18 carrega devagar por R37
  (0,9 s) e descarrega ainda mais devagar (2,0 s) por R37 + R11.
  - **[DER]** O desenho marca C18 como "27 µF". Os tempos publicados só fecham com **0,27 µF**:
    3,3 MΩ × 0,27 µF = 0,89 s, e 6,6 MΩ × 0,27 µF = 1,78 s ≈ 2 s. É provável que falte o ponto
    decimal no desenho.
- **Comportamento:** depois de um pico isolado, o release é curto (0,33 s), o que evita o "buraco".
  Depois de uma série de picos, o release fica longo (~2 s), o que evita "breathing" e "pumping".
  É um **release dependente do programa por dois capacitores**, a mesma ideia do Double do Sta-Level e
  das posições 5 e 6 do Fairchild 670 [DOC].
- **Posições da BAL-LIM:**
  - LIMIT-OFF: amplificador sem compressão.
  - Single-limit e Dual-limit: os dois modos de compressão.
  - BALANCE A e BALANCE B: balanceamento dos 6SK7.

  A eserviceinfo tem um manual de uma unidade modificada que lista OFF, Limit-Single, Double, A e B [LORE].

#### 3.4 Topologia [DOC, artigo de 1950 e catálogo 1957]
- Amplificador balanceado de **três estágios**, todo em push-pull:
  1. 2× **6SK7**, pentodos remote-cutoff: estágio de ganho variável.
  2. 2× **6J7**: estágio de ganho fixo, acoplado à primeira por transformador para manter o balanço.
  3. 2× **6V6-GT**: saída, 10 W.
- **NFB nos dois últimos estágios.** A tensão de tela dos 6SK7 é estabilizada pela OD3.
- **Anti-thump:** as tensões de tela e de catodo dos 6SK7 são ajustáveis (controles Bal A e Bal B) para
  casar qualquer par ao longo de toda a faixa de GR. O artigo chama isso de "high signal-to-thump ratio".
- **Sidechain:** a 6H6 é um retificador de onda completa ligado ao **estágio de saída** por capacitores e
  resistores de isolação. É **feedback**. Os catodos da 6H6 ficam polarizados positivamente, o que dá
  o limiar. A tensão filtrada vai às grades de controle dos 6SK7.
- Medidor rotativo de 4": GR, corrente de catodo de cada tubo, balanço e tensão de filamento [DOC].

#### 3.5 Curva estática [DOC, Fig. 7 do artigo de 1950, "MI-11225 Limiting Characteristic"]
- Abaixo do verge of limiting (~29,5 dBm de saída com cerca de −24 dBm de entrada), o ganho é linear,
  de 53,8 dB.
- Acima disso, 20 dB a mais na entrada produzem só 1,9 dB a mais na saída. **[DER]** Isso dá ≈10,5:1.
- No gráfico, o joelho parece **relativamente duro** comparado ao do Sta-Level. A curva passa de 1:1 para
  ~10:1 em pouco mais de 1 a 2 dB. Isto é leitura visual do gráfico [DER].
- **GR máximo: DESCONHECIDO.** As especificações vão até 20 dB de GR, e o artigo recomenda no máximo 15 a 20 dB em programa.

#### 3.6 Coloração
- Fabricante: THD <1% com 12 dB de GR; resposta +1/−2 dB até 20 dB de GR [DOC].
- Usuários e lojas: "thick, fat, gluey", "ton of low mid", "gritty". Reputação ligada a Jimmy Page, sem
  confirmação [LORE]. As notas de construção de S. Komiya comparam: o Federal 864 tem agudos mais
  abertos, o BA-6A tem "low mid, ton of low mid" [LORE].
- Distribuição de harmônicos: DESCONHECIDA. **[EST]** O caminho todo é push-pull e pentodos 6SK7 sob bias
  variável, então o 3º harmônico deve dominar quando o par está balanceado, e o 2º cresce com o desbalanço.

#### 3.7 Uso hoje [LORE e lojas]
- Bateria (caixa e bumbo), baixo, guitarras, voz e mix bus, muitas vezes com pouca compressão e
  aproveitando o caráter dos transformadores e tubos.
- Ataque de 600 µs: entre os três, é o único que se comporta como **limitador de pico** de verdade.

---

### 4. Modelagem em DSP de variable-mu

#### 4.1 Literatura
1. **Raffensperger, P., "Toward a Wave Digital Filter Model of the Fairchild 670 Limiter", DAFx-12 (York, 2012).**
   - Modelo caixa-preta do triodo **6386** remote-cutoff, ajustado ao datasheet da GE:
     `Ia = p1·Vak^p2 / [ (p3 − p4·Vgk)^p5 · (p6 + exp(p7·Vak − p8·Vgk)) ]`, com
     p1 = 3,981e-8, p2 = 2,383, p3 = 0,5, p4 = 0,1, p5 = 1,8, p6 = 0,5, p7 = −0,03922, p8 = 0,2.
     A corrente de grade é desprezada porque Vgk < 0.
   - O amplificador de sinal é um push-pull classe A em WDF. Equação implícita do tubo resolvida por
     Newton-Raphson. Capacitor de catodo dividido em 2× 2C1 com atraso unitário para quebrar o laço sem
     atraso. Transformadores modelados com Rp, Lp, Rs, Ls, Lm, Rc e Cw.
   - O sidechain usa não linearidades estáticas caixa-preta mais a rede RC de tempo (seis posições,
     com valores de C e R em tabela). Mostra como redes com dois e três capacitores geram o
     release dependente do programa.
   - Oversampling de **8×**.
   - O **atraso unitário fictício** no laço de feedback piora o limiting de transientes agudos (hi-hat
     vaza). Isso é relevante aqui, porque os três aparelhos são feedback.
   - Código GPL: github.com/praffensperger/wavechild670.
2. **Giannoulis, Massberg e Reiss, "Digital Dynamic Range Compressor Design — A Tutorial and Analysis", JAES 60(6), 2012, pp. 399–408.**
   - Compara topologias feedforward e feedback, detecção de pico e RMS, e smoothing de ataque e release
     no domínio linear e no logarítmico. É a referência para um modelo comportamental.
3. Citados no paper do Raffensperger (não consultei diretamente): modelos de triodo de Koren e de
   Cardarilli; WDF de válvulas de Karjalainen e Pakarinen; o argumento de Yeh sobre oversampling de 8×.

#### 4.2 Caminho recomendado para o LinuxMusic [EST: proposta minha]

Um modelo comportamental com "física mínima" é mais barato que WDF completo e preserva o que importa:

1. **Laço de feedback com um passo de atraso.** Para não ter a perda de transientes que o Raffensperger
   relata, resolva o laço instantâneo. Duas opções: (a) uma iteração de ponto fixo ou de Newton por
   amostra sobre `y = x·g(Vc)` e `Vc = f(rect(y))`; (b) oversampling de 2× a 4× no sidechain.
2. **Detector:** retificador de onda completa com delay bias, ou seja, conduz só acima de `Vth`:
   `i = max(0, |y|·k − Vth)/Rsrc`. Ele carrega a rede RC de ataque e release:
   - Sta-Level: C10 e C11, R35, R36 e R37, na configuração Single ou Double.
   - BA-6A: C1 e C18 (0,27 µF), R11 e R37.
   - 436C: R9 ∥ C4, com R9 variável para o release. C4 é desconhecido; calibre por τ = 0,3 a 1,3 s.

   Simule a rede como um sistema de estado de 1 ou 2 capacitores (Euler implícito ou trapezoidal).
   É isso que gera o release dependente do programa **sem precisar de truque**.
3. **Lei de ganho** g(Vc): é a característica do tubo remote-cutoff.
   - Sta-Level: use a tabela do §1.4 ou `GR = 16,6·ln(1+|Vc|/8,1)` dB.
   - 436: só existe o ponto −38 V → 20 dB. Ajuste a mesma forma logarítmica a esse ponto [EST].
   - BA-6A: calibre pelo limiar e pela razão documentados. Não há dados da 6SK7 nesta pesquisa.
   - Opção mais física: a equação do 6386 do Raffensperger para o Sta-Level (que usa a mesma 6386 do Fairchild).
4. **A razão sai do laço.** Não imponha uma curva estática. Calibre `k` e `Vth` para reproduzir:
   - Sta-Level: 3,3:1 com limiar de +10/+12 dBm.
   - 436C: 2:1 com threshold em 0 dBm e 4:1 com +16 dBm. Isto é um teste de validação ótimo.
   - BA-6A: 10:1 com limiar de 29,5 dBm.
5. **Coloração:**
   - Duas metades push-pull com `g(Vc ± δ)` e um parâmetro de desbalanço δ. Isso gera o 2º harmônico e o
     "thump", que é o vazamento do sinal de controle para a saída.
   - Uma não linearidade suave dependente do GR, calibrada aos THD documentados: Sta-Level ≤1% com
     30 dB; 436C 1,5% com 25 dB e 2,5% com 30 dB; BA-6A <1% com 12 dB.
   - Transformadores de entrada e de saída como filtros e saturação de baixa frequência. Use a resposta
     documentada: 436C ±1,5 dB de 30 Hz a 15 kHz; BA-6A −2 dB com 20 dB de GR.
6. **Oversampling** de pelo menos 2× a 4× nas não linearidades. O artigo usa 8×.

---

### 5. Lacunas (DESCONHECIDO)
- Perfil harmônico medido (2º × 3º etc.) de qualquer um dos três.
- **Altec 436A e 436B:** limiar, ataque, release e razão de fábrica. Valor de C4 na rede de release
  (ilegível no esquema). Curva completa da 6BC8 sob bias.
- **RCA BA-6A:** GR máximo, curva de ganho × bias da 6SK7 e valores de componentes além da Fig. 4.
  O manual de instrução original (IB) não foi acessado; está em Scribd e eserviceinfo, ambos bloqueados.
- **Sta-Level:** o desenho A-11734 (curva de compressão) não aparece no PDF consultado.
- **Retro Sta-Level:** tempos em ms dos modos Single, Double e Triple e da chave de seis posições.
- **EMI RS124:** tempos das seis posições.

---

### Fontes

Primárias:
- Gates Radio Co., *Instructions M5167 Sta-Level Amplifier*, IB-727, 30/04/1956: https://www.worldradiohistory.com/Archive-Catalogs/Gates-Harris/Gates-Sta-Level-Amplifier-M5167-1956.pdf
- Gates Radio Co., *Instruction Book Level Devil Amplifier M-5546*, 1958: https://www.barryrudolph.com/recall/manuals/gates_level_devil_1958.pdf
- Altec Lansing, *436C Compressor Amplifier*, spec sheet AL-1401-4: https://www.preservationsound.com/wp-content/uploads/2011/04/Altec_436C.pdf
- Altec Lansing, *Technical Letter #10, 436A Amplifier*, com o esquema 6190: https://retronik.silicium.org/DOCUMENTS/Audiovideo/Altec-Lansing/Altec-Lansing-436-A-Schematic.pdf
- C. A. Singer (RCA), "Performance and Operation of a New Limiting Amplifier", *Audio Engineering*, nov. 1950, pp. 18–19 e 69–70: https://www.worldradiohistory.com/Archive-All-Audio/Archive-Audio/50s/Audio-1950-Nov.pdf
- RCA, *Broadcast Equipment for AM FM Television*, 3ª ed., 1957, pp. 57–58 (BA-6A): https://www.worldradiohistory.com/Archive-Catalogs/RCA/RCA-Broadcast-Catalog-3rd-1957.pdf
- Anúncio RCA BA-6A, *Broadcasting*, 02/04/1951: https://www.worldradiohistory.com/Archive-All-BC/Broadcasting-Magazine/BC-1951/BC-1951-04-02.pdf

Literatura de DSP:
- P. Raffensperger, "Toward a Wave Digital Filter Model of the Fairchild 670 Limiter", DAFx-12: https://www.dafx12.york.ac.uk/papers/dafx12_submission_9.pdf
- Wavechild670 (código): https://github.com/praffensperger/wavechild670
- D. Giannoulis, M. Massberg e J. Reiss, JAES 2012. Usei apenas os metadados e o resumo: https://aes2.org/publications/elibrary-page/?id=16354

Secundárias:
- Telos Alliance, "The Sta-Level Turns 60": https://blogs.telosalliance.com/the-sta-level-turns-60
- Radio World, "The Sta-Level: Effective Audio Control": https://www.radioworld.com/news-and-business/the-stalevel-effective-audio-control
- History of Recording, Gates Sta-Level: https://www.historyofrecording.com/Gates_Sta-Level.html
- History of Recording, RCA BA-6A: https://historyofrecording.com/RCA_BA-6A_Limiting_Amplifier.html
- Sound On Sound, review do Retro Instruments Sta-Level: https://www.soundonsound.com/reviews/retro-instruments-sta-level
- MusicTech, "Vintage: Altec 436": https://musictech.com/2014/05/altec-436/
- Mix, "Dynamic Resurrection" (Altec 436/438): https://www.mixonline.com/recording/dynamic-resurrection-365173
- Preservation Sound, "Altec 436 Compressor: Taming the Output Level": https://www.preservationsound.com/altec-436-compressor-taming-the-output-level/
- Preservation Sound, tag "tube limiter" (BA-6A e Federal AM-864): https://preservationsound.com/tag/tube-limiter

Fórum, loja e lore:
- S. Komiya, notas de construção de um BA-6A: https://www.preservationsound.com/wp-content/uploads/2015/01/S_Komiya_RCA_Ba6a_DIY_notes.pdf
- AudioScape ASA-6A (clone): http://audioscape.squarespace.com/nineteen-inch-rack/asa6a
- eserviceinfo, manual de BA-6A modificado (só o preview): https://www.eserviceinfo.com/preview_html.php?fileid=59507&previewid=30772
- BunkerOfDoom, BA-6A: https://bunkerofdoom.com/kd5oei/ba6a/index.html

---

## Anexo E: Gate/Expander estilo SSL E e balística de compressores: pesquisa

Pesquisa para o LinuxMusic. A faixa analógica tem um gate/expander no estilo SSL E. Dois controles ficam em knobs: **THRESHOLD** (−30 a +10 dBu) e **RANGE** (0–40 dB). O resto vem de um **MODO** fixo.

Legenda das marcações usadas em todo o documento:

- **[D]**: valor documentado, com a fonte citada (manual, artigo ou tabela de presets).
- **[A]**: valor documentado, mas adaptado aos limites do hardware SSL E. Exemplo: release mínimo de 0,1 s, ataque só em dois valores. A adaptação é explicada.
- **[J]**: decisão de julgamento minha. Nenhuma fonte dá esse número, então é ponto de partida para ajuste de ouvido ou medição.

---

### Parte A: Gate/Expander por fonte

#### A.1 O que o hardware SSL E documenta

| Parâmetro | X-Rack E Series Dynamics (manual SSL 82S6XR0L0A) | SSL Native Channel Strip (User Guide V6) |
|---|---|---|
| Modo | "∞:1 Gate or as a 2:1 Expander when the EXP button is pressed" | Gate **40:1**, Expander **2:1** |
| Threshold | Especificação: −30 a +10 dB. O texto da seção diz "+10dBu to –20dBu", e o manual se contradiz aí. | 30 dB a +10 dB |
| Range | 0–40 dB | 0–40 dB |
| Ataque | "controlled linear attack time of 1.5ms per 40dB"; FAST = 100 µs per 40 dB | igual |
| Release | "time constant (speed), variable from 0.1 to 4 seconds". O manual avisa: "this control interacts with the RANGE control". | 0,1–4 s |
| Hold | não existe no X-Rack | 0–4 s, adição do plug-in |
| Histerese | "Variable hysteresis is incorporated in the threshold circuitry which increases as the threshold is lowered… allows instruments to decay below the open threshold before gating" | mesmo texto; "Threshold" é o limiar de abertura |
| Key/sidechain | entrada KEY externa e LINK somando os CVs. **Não há filtros de key no módulo.** | EQ e filtros da faixa podem ser roteados para o sidechain, com S/C LISTEN |
| Detector | "A true RMS converter is used in the side chain". O texto fala do módulo em geral e não deixa claro se o gate usa o mesmo detector. | — |

Observações:
- O guia citado pelo usuário dá **20:1** para o gate. As fontes SSL que encontrei dão **∞:1** (X-Rack) e **40:1** (Native). Conclusão: a razão do gate varia conforme o produto SSL. Qualquer valor ≥20:1 é defensável. [D, divergente]
- Nenhuma fonte SSL dá números para a histerese, só o comportamento: ela cresce quando o threshold desce.
- No Waves SSL E-Channel, segundo trechos do manual vistos na busca, o modo padrão é expander e o botão GATE muda para gate. O range é 0–40 dB e a histerese é descrita como variável.

#### A.2 Referências de hardware e presets com números

##### Drawmer DS201: diagramas de operação do manual oficial (2008), pp. 8–11

Faixas dos controles: L.F. key 25 Hz–4 kHz, H.F. key 250 Hz–35 kHz, Attack 10 µs–1 s, Hold 2 ms–2 s, Decay 2 ms–4 s, Range 0–90 dB. O manual diz que os filtros "only affect the way the DS201 responds… they do not have any direct effect on the output signal".

| Aplicação (manual DS201) | Key LF (HPF) | Key HF (LPF) | Attack | Hold | Decay | Range |
|---|---|---|---|---|---|---|
| Gating a Kick Drum | 35 Hz | 250 Hz | 0,1 ms | 50 ms | 50 ms | "Set to User Preference" |
| Gating a Snare Drum | 1 kHz | 2 kHz | 0,1 ms | 50 ms | 50 ms | idem |
| Bass Guitar keyed from Kick (key externa) | 35 Hz | 250 Hz | 0,1 ms | 0,2 s | 0,2 s | idem |
| Ducking Music from Vox | 25 Hz | 30 kHz | usuário | usuário | usuário | — |

##### Yamaha LS9: Dynamics Library de fábrica (manual LS9, pp. 227–228)

O manual do CS1D avisa que Hold e Decay valem a 44,1 kHz.

| Preset | Tipo | Thr | Range/Ratio | Attack | Hold | Decay/Release |
|---|---|---|---|---|---|---|
| Gate | GATE | −26 dB | range −56 dB | 0 ms | 2,56 ms | 331 ms |
| A. Dr BD | GATE | −11 dB | range −53 dB | 0 ms | 1,93 ms | 400 ms |
| A. Dr SN | GATE | −8 dB | range −23 dB | 1 ms | 0,63 ms | 238 ms |
| A. Dr SN | EXPANDER | −23 dB | 2:1, knee 2 | 0 ms | — | 151 ms |
| A. Dr Tom | EXPANDER | −20 dB | 2:1, knee 2 | 2 ms | — | 749 ms |
| Expand | EXPANDER | −23 dB | 1,7:1, knee 2 | 1 ms | — | 70 ms |
| Click Erase | EXPANDER | −33 dB | 2:1, knee 2 | 1 ms | — | 284 ms |
| Ducking | DUCKING | −19 dB | range −22 dB | 93 ms | 1,20 s | 6,32 s |

Os presets de vozes ("Solo Vocal1/2", "Announcer") são compressores ou companders, não gates.

##### SSL Drumstrip (plug-in), User Guide V6

Gate com limiares Open e Close separados, ou seja, histerese explícita. Open e Close Thr vão de 0 a −30 dB, Range 0 a −80 dB, Attack 0–0,1 ms, Hold 0–4 s, Release 0–1 s. O guia diz que, para gating de verdade, o range deve ser −80 dB, e que range menor serve para "cleaning up a drum track containing reverb".

#### A.3 Recomendações em artigos e livros

| Tema | O que a fonte diz | Fonte |
|---|---|---|
| Hold | "set Hold to around 50mS, or the shortest time I can get away with without jitter" | D. Mellor, SOS jan/1996 (DS201 tips) |
| Hold fixo | gates sem hold costumam ter hold fixo de "about 20 to 30ms", para não disparar a cada ciclo de grave | P. White, SOS abr/2001 (Advanced Gating pt. 1) |
| Hold contra chatter | baixo grave e acordes de guitarra com trêmolo fazem o gate trepidar; a cura é aumentar o hold | SOS abr/2003 (Using your DAW's gates) |
| Ataque | percussivos: "just a few tens of microseconds"; ataques lentos: "10ms or more" | SOS 2001 pt. 1 |
| Ataque e clique | começar em zero e subir até o clique sumir | M. Houghton, SOS set/2018 |
| Histerese | dois limiares "a few dB(s)" separados | SOS 2001 pt. 1; SOS 2003 |
| Histerese, número | "Generally, −6 dB is a good place to start" | Apple, manual do Logic Pro (Noise Gate) |
| Key, caixa contra chimbal | HF key em "2kHz or so" | Mellor, SOS 1996 |
| Key, caixa (alternativa) | key em passa-faixa de 200 Hz com 1 oitava; chimbal em 15 kHz com 0,5 oitava | Mellor, SOS out/1988 (How it works: the noise gate) |
| Key, caixa contra bumbo | cortar graves no sidechain da caixa; cuidado, filtrar demais um som de ataque rápido atrasa a abertura | SOS 2001 pt. 1 |
| Key, bumbo | "filtering out everything above 100Hz or so" | SOS 2003 |
| Key, voz | LF e HF entre 1 kHz e 5 kHz (o texto diz "1Hz", provável erro de digitação); contra vazamento de grave, LF ≈ 250 Hz | Mellor, SOS 1996 |
| Key contra hum | filtrar abaixo de "about 200Hz" | Senior & White, SOS mai/2001 pt. 2 |
| Expander na voz (riding) | attack 0,1 ms, release 25 ms, ratio ~1,4:1, hold zero; key com HPF de 12 dB/oct em ~150 Hz e +4–5 dB em 1,8–2 kHz | Houghton, SOS 2018 |
| Profundidade | "as little as 6dB" basta; "12dB is usually enough" para chimbal vazando na caixa | SOS (Avid Pro Expander) |
| Range na voz | usar range para reduzir a respiração em vez de cortá-la | SOS 2003 |
| Detector | peak para percussivos, RMS para fontes constantes como voz | SOS 2003 |
| Lookahead | "a millisecond or two" | SOS 2003 |
| Lookahead (Pro Tools Dyn3) | começa a processar 2 ms antes de o sinal cruzar o threshold | Avid, ajuda do Dyn3 (trecho visto na busca) |
| Release | "the most important parameter… after the gating threshold"; casar com o decaimento natural | SOS 2001 pt. 1 |

#### A.4 Proposta de tabela de modos

O hardware impõe restrições. O ataque tem só dois valores, NORMAL (1,5 ms/40 dB) e FAST (100 µs/40 dB). O release vai de 0,1 a 4 s. Range e threshold ficam no knob. Hold, histerese e filtros de key não existem no E original. Proponho incluí-los como parte fixa do modo, marcados como extensão.

| Modo | Tipo | Ataque | Hold | Release | Histerese | Key HPF | Key LPF | Lookahead |
|---|---|---|---|---|---|---|---|---|
| **KICK** | Gate | FAST [D: DS201 0,1 ms] | 50 ms [D: DS201] | 0,1 s [A: DS201 usa 50 ms, abaixo do mínimo SSL; Yamaha BD usa 400 ms] | 3 dB [J] | 35 Hz [D: DS201] | 250 Hz [D: DS201; SOS: "~100 Hz"] | 0 [J] |
| **SNARE** | Gate | FAST [D: DS201] | 50 ms [D: DS201] | 0,24 s [D: Yamaha A.Dr SN 238 ms; DS201 usa 50 ms] | 3 dB [J] | 1 kHz [D: DS201] | 2 kHz [D: DS201; Mellor "2kHz or so"] | 0 [J] |
| **TOMS** | Gate | FAST [J; Yamaha Tom usa 2 ms] | 100 ms [J] | 0,75 s [D: Yamaha A.Dr Tom 749 ms] | 6 dB [J; Logic sugere −6 dB para começar] | 60 Hz [J] | 3 kHz [J] | 0 [J] |
| **VOX** | Expander 2:1 | NORMAL [J; Yamaha Expand 1 ms] | 0 [D: Houghton "zero hold"] | 0,1 s [A: Yamaha 70 ms e Houghton 25 ms ficam abaixo do mínimo SSL] | 4 dB [J] | 150 Hz [D: Houghton] | off [J; Mellor usa 5 kHz só contra vazamento] | 0 |
| **GTR** | Gate | NORMAL [J; SOS: ataque rápido demais clica em fontes não percussivas] | 50 ms [J; SOS: subir o hold contra chatter de trêmolo] | 0,3 s [J] | 6 dB [J] | 80 Hz [J; SOS: tirar grave do sidechain] | off | 0 |
| **BASS** | Gate | NORMAL [J] | ≥ 25 ms. Proponho 0,2 s [D: DS201 bass-from-kick usa 0,2 s; o piso de 25 ms vem de 1/41,2 Hz, o E1, e bate com os 20–30 ms do SOS] | 0,2 s [D: DS201] | 3 dB [J] | off | off | 0 |
| **NOISE** | Expander 2:1 | NORMAL [J] | 50 ms [J] | 0,28 s [D: Yamaha Click Erase 284 ms, 2:1] | 3 dB [J] | 150 Hz [J; entre os ~200 Hz anti-hum do SOS e os 150 Hz de Houghton] | off | 0 |
| **EXP** | Expander 2:1 | NORMAL [D: SSL padrão] | 0 [D: E original] | 0,1 s [A: Yamaha Expand 70 ms] | SSL variável [J, ver A.5] | off | off | 0 |
| **STD** | Gate | FAST [D: Yamaha "Gate" 0 ms] | 0 (Yamaha 2,56 ms) [D] | 0,33 s [D: Yamaha "Gate" 331 ms] | SSL variável [J] | off | off | 0 |

Notas sobre a tabela:
1. **Overheads.** Nenhuma fonte recomenda gate em overhead. A Yamaha usa compander em "A. Dr OverTop". Não proponho modo próprio. Se o usuário quiser, EXP com range baixo resolve. [J]
2. **Diálogo.** Não achei fonte de referência (SSL, SOS ou manual) com números para expander em diálogo. Os sites de podcast e OBS que apareceram na busca se contradizem: release de 12 ms em um, 50–120 ms em outro. NOISE se apoia no preset Yamaha "Click Erase" e o resto é julgamento.
3. **Kick: 50 ms ou 400 ms.** O DS201 (50 ms) e a Yamaha (400 ms) divergem muito. Como o release mínimo do SSL é 0,1 s, fiquei com 0,1 s para um bumbo "tight". A escolha é [A] e discutível.
4. **Lookahead.** Uma emulação analógica não deve ter lookahead. Se for oferecido, que seja um toggle de 2 ms nos modos de bateria, seguindo o valor do Pro Tools Dyn3. Ele exige compensação de latência no mixer. [J]
5. **Filtros de key.** São uma extensão em relação ao E. Para manter a autenticidade, eles poderiam valer só em KICK, SNARE e TOMS, que são os casos documentados no DS201. [J]

#### A.5 Histerese no estilo SSL (julgamento)

A SSL só documenta o comportamento: a histerese "increases as the threshold is lowered". Uma forma possível, toda [J]:

```
H(T) = H_min + (H_max − H_min) · (T_max − T) / (T_max − T_min)
T ∈ [−30, +10] dBu
```

Os valores H_min ≈ 2 dB e H_max ≈ 8 dB são [J] e não têm fonte. A coluna de histerese da tabela entraria como offset do modo, ou substituiria H(T) nos modos de bateria. Se houver um E real para medir (seno com rampa subindo e descendo), dá para calibrar.

---

### Parte B: DSP de balística e detectores

#### B.1 Giannoulis, Massberg & Reiss (JAES 60(6):399–408, 2012)

> Não consegui baixar o PDF integral: o certificado do servidor da QMUL falhou e AES e Academia devolveram 403. As fórmulas abaixo estão marcadas pelo grau de verificação: **[V]** confere com uma fonte secundária que lemos (MathWorks, que cita o artigo; o código do OpenAL Soft; o fórum JUCE citando o artigo); **[M]** é reconstrução da memória do artigo e precisa ser conferida no original.

**Coeficiente de tempo.** O artigo define [M]:

```
α = exp(−1 / (τ · fs))
```

τ é o tempo para atingir 1 − 1/e ≈ 63 % do degrau. Para especificar tempo de subida de 10 a 90 %, a variante é [V] (MathWorks; Yu & Fazekas usam 2200·T/t_ms, que dá o mesmo resultado):

```
α = exp(−ln 9 / (fs · T))     (≈ 2,2 τ)
```

**Gain computer com soft knee** [V, MathWorks citando Giannoulis]. Tudo em dB, com T = threshold, R = razão e W = largura do joelho:

```
y_G = x_G                                      se 2(x_G − T) < −W
y_G = x_G + (1/R − 1)(x_G − T + W/2)² / (2W)    se 2|x_G − T| ≤ W
y_G = T + (x_G − T)/R                           se 2(x_G − T) > W
```

**Expander/gate (versão espelhada).** Esta derivação é minha, conferida algebricamente: casa valor e derivada nas bordas do joelho. Use R_e = razão do expander (2 para o SSL; 20, 40 ou ∞ para o gate) e "range" como piso:

```
y_G = T + R_e (x_G − T)                          se x_G < T − W/2
y_G = x_G + (1 − R_e)(x_G − T − W/2)² / (2W)     se |x_G − T| ≤ W/2
y_G = x_G                                        se x_G > T + W/2
g_dB = max(−Range, y_G − x_G)
```

**Detectores de pico.** x_L é a entrada do detector, y_L a saída.
- Branching [M]: `y_L[n] = α_A y_L[n−1] + (1−α_A) x_L[n]` se x_L > y_L[n−1]; senão `α_R y_L[n−1]`.
- Smooth branching [M]: igual, mas a fase de release também mistura a entrada: `α_R y_L[n−1] + (1−α_R) x_L[n]`.
- Decoupled [M]: `y_1[n] = max(x_L[n], α_R y_1[n−1])`, depois `y_L[n] = α_A y_L[n−1] + (1−α_A) y_1[n]`.
- **Smooth decoupled** [V, código OpenAL Soft "smooth decoupled peak detector"]:

```
y_1[n] = max( x_L[n], α_R y_1[n−1] + (1−α_R) x_L[n] )
y_L[n] = α_A y_L[n−1] + (1−α_A) y_1[n]
```

No modo desacoplado o envelope de ataque se soma ao de release. O artigo, citado no fórum JUCE, diz que isso "guarantees that the release time can never be shorter than the attack time" [V]. O OpenAL compensa usando τ_R,efetivo = τ_R − τ_A [V].
- RMS [M]: `y²[n] = α y²[n−1] + (1−α) x²[n]`.

**Onde fica o detector.** Segundo o resumo do abstract visto na busca, a recomendação é colocar o detector **depois** do gain computer, **no domínio log**, sobre o sinal de redução de ganho `x_L = x_G − y_G`. O resultado é envelope suave, sem atraso de ataque, e joelho que funciona independente das constantes de tempo [V parcial]. O OpenAL Soft faz exatamente isso: `x_L = −slope·y_G` seguido do smooth decoupled em log [V].

**Feed-forward e feedback.**
- Feed-forward: sidechain s[n] = x[n].
- Feedback: s[n] = y[n−1]. Yu & Fazekas classificam o LA-2A como "technically a feedback compressor" [V].
- Álgebra de regime estacionário (derivação minha): se o gain computer aplica `g = −k(y − T)` sobre a saída, então `y − T = (x − T)/(1+k)`, logo a razão efetiva é **R = 1 + k**. Para ∞:1 seria preciso k → ∞, por isso feedback não faz limiting perfeito nem lookahead. Isso bate com a discussão do artigo, que eu lembro em linhas gerais [M].

**Gate com hold** [V, MathWorks]. São contadores de hold C_A e C_R, com k = hold em amostras:

```
g_s[n] = α_A g_s[n−1] + (1−α_A) g_c[n]   se C_A > k  e g_c ≤ g_s[n−1]
       = g_s[n−1]                         se C_A ≤ k
       = α_R g_s[n−1] + (1−α_R) g_c[n]   se C_R > k  e g_c > g_s[n−1]
       = g_s[n−1]                         se C_R ≤ k
```

#### B.2 Aplicação direta ao gate SSL E (proposta)

1. **Detector.** Retificação ou RMS, convertida para dBu. O X-Rack diz que usa "true RMS converter", mas para gate de bateria um RMS de janela curta ou pico é mais seguro [J].
2. **Máquina de estados com histerese** [J, implementação padrão]: abre se L > T; fecha se L < T − H(T), e só depois que o hold expira.
3. **Ataque "linear 1.5 ms/40 dB".** Interpreto como slew-rate em dB [J, leitura literal do manual]: Δ_max = 40 dB / (1,5 ms · fs) por amostra, ou 40 dB / (0,1 ms · fs) no modo FAST. Isso dá 26,7 dB/ms e 400 dB/ms.
4. **Release.** Uma constante de tempo de 0,1–4 s, como one-pole em dB rumo a −Range. O manual SSL diz que o release "interacts with the RANGE control". Com one-pole em dB, a duração total aumenta com o range, o que reproduz a interação [J].
5. **Curva estática.** Gate com R_e ≥ 20 (20, 40 ou ∞ conforme a fonte) e expander 2:1. Os dois limitados por −Range (fórmula de B.1).

#### B.3 Release dependente do programa

| Modelo | Fórmula ou comportamento | Status |
|---|---|---|
| Crest factor (Giannoulis et al., "Parameter automation…", 2013, implementado no OpenAL Soft) | `C² = y²_peak / y²_rms`, com os dois detectores a 200 ms; `τ_A = 2·τ_A,médio / C²`; `τ_R = 2·τ_R,médio / C² − τ_A` | [V, código] |
| Release em dois estágios (LA-2A) | UA: ataque médio de 10 ms; release "about 60 milliseconds for 50% of the release, and anywhere from 1 to 15 seconds for the rest" | [D, UA, citado por Yu & Fazekas] |
| Fairchild 670, posições 5 e 6 | Posição 5: 0,4 ms de ataque, release de "2 s" para picos isolados e 10 s para múltiplos picos. Posição 6: 0,2 ms de ataque, 0,3 s para picos isolados, 10 s para múltiplos, 25 s para nível alto constante. Também: P1 0,2 ms/0,3 s; P2 0,2 ms/0,8 s; P3 0,4 ms/2 s; P4 0,4 ms (ou 0,8 ms) / 5 s. | [D, Telefunken/Fairchild. A página tem erros de digitação ("3 sec" em P1, ".2 sec" em P5), e outras fontes divergem no ataque de P4] |
| Soma de one-poles (Eichas & Zölzer) | suavização do ganho por três filtros de 1ª ordem combinados com pesos α1, α2 ∈ [0,1], ajustados por otimização sobre medições | [V] |

Uma implementação genérica de dois estágios, que é [J] mas tem estrutura padrão: `g = w·g_fast + (1−w)·g_slow`. O estágio lento carrega mais quanto mais tempo dura a redução de ganho ("memória"), e w sai do ajuste (por exemplo, 50 % do release em ~60 ms no LA-2A).

#### B.4 Célula óptica (opto)

Dados medidos por Eichas & Zölzer (SPIE 2016) num VTL5C2, que é LED + LDR [V]:
- LDR > 1 MΩ com I_LED < 0,07 mA; < 300 Ω com I_LED > 30 mA.
- Ligar 0→10 mA: o LDR cai abaixo de 1 kΩ em 5 ms.
- Desligar 10→0 mA: leva mais de 500 ms para passar de 1 MΩ, cerca de 100 vezes mais lento.
- No circuito completo, o ataque (>100 ms) é dominado pelo capacitor em paralelo ao LED. O release é dominado pela célula.
- O LA-2A usa a célula T4: lâmpada ou painel eletroluminescente mais LDR de CdS. É um nivelador realimentado: "amplification… driven by the modified output signal".

Estrutura de modelo [V/J]:
1. Usar topologia feedback (s = y[n−1]).
2. Converter o nível em "luz" com um one-pole rápido na subida.
3. Mapear a luz para R_LDR com uma curva medida ou lei de potência, que é [J].
4. Fazer o release da R_LDR em dois estágios ou com uma constante dependente do histórico (B.3).
5. Calcular o ganho pelo divisor resistivo do circuito. No exemplo de Eichas o ganho é `g = 1 + R_A/R_B`, com R_A = R3 ∥ R_LDR.

Eichas também usa coeficientes de ataque e release diferentes conforme o **sinal** da amostra anterior, para gerar harmônicos ímpares [V].

#### B.5 FET (1176)

- Documentado pela UA [D]: ataque de 20 a 800 µs, release de 50 ms a 1,1 s; razões 4, 8, 12 e 20:1 (o 1176AE também tem 2:1). No "All Buttons" mode, "compression distortion increases radically". Com ataque e release muito rápidos, "minute level fluctuations sound like distortion".
- O FET como resistor controlado por tensão (Siliconix AN105) [V]:

```
r_DS ≈ r_DS(on) / (1 − V_GS / V_GS(off))
V_out = V_in · r_DS / (R + r_DS)      (atenuador divisor)
```

A distorção vem da curvatura das retas de polarização com V_DS. O pinch-off acontece em V_DS = V_GS − V_GS(off). A nota mostra o caso: sem realimentação, distorção acima de 8 %; com metade de V_DS realimentada no gate por dois resistores iguais, abaixo de 0,5 %. A faixa útil de r_DS é tipicamente 10:1, e chega a mais de 100:1.
- Modelo de distorção [J, a partir do AN105 e da lei quadrática clássica de Shichman–Hodges, não verificada aqui]: calcular a corrente na região ôhmica com `I_D ∝ 2(V_GS − V_P)V_DS − V_DS²` usando o V_DS instantâneo do áudio. O termo V_DS² produz harmônicos pares que crescem com a redução de ganho. A compensação parcial de V_DS/2 controla quanto sobra.

#### B.6 Variable-mu (Fairchild, Manley)

- Válvula de corte remoto: o ganho (na prática o g_m) cai à medida que a polarização de grade fica mais negativa. A tensão de controle soma-se ao áudio no ponto de operação. O estágio é push-pull, e o componente de controle (o "thump") se cancela no transformador de saída, o que exige ramos casados. A razão efetiva sobe com o nível, dando um joelho naturalmente suave, e a saturação aparece nos transientes [V, Pulsar Audio].
- A UA relata que a curva ganho×bias depende tanto da geometria de cada válvula que precisou ser medida. Não encontrei a fonte primária, só um trecho visto na busca.
- Fairchild 670 [D, Telefunken]: razão de 1:2 a 1:30, conforme a quantidade de limiting e os thresholds. Leva 8× 6386, entre outras válvulas. Constantes de tempo em B.3.
- Modelo [J]: tabela medida g(V_bias); sidechain em feedback; ganho aplicado como `y = g(V_c + k·x)·x` em vez de `g(V_c)·x`. A dependência do sinal instantâneo gera a saturação ligada à redução de ganho, e o push-pull simétrico cancela os termos pares de controle.

#### B.7 Distorção do elemento de ganho: resumo de técnicas

| Técnica | Mecanismo | Status |
|---|---|---|
| Ganho dependente do sinal instantâneo (FET ou vari-mu) | g = f(V_controle, x[n]) em vez de f(V_controle) | [J], com base física no AN105 e na Pulsar |
| Assimetria ataque/release por polaridade | coeficientes diferentes para amostras + e −, gerando ímpares | [V, Eichas] |
| Ripple do controle com release rápido | modulação AM do áudio pelo envelope; é a "compression distortion" do 1176 | [D, UA] e [J] |
| VCA classe A (SSL) | THD+N < 0,05 % a +20 dBu (X-Rack). Distorção baixa, então uma não linearidade leve basta | [D] |
| Filtro LTI na entrada e na saída | Eichas acrescenta um filtro LTI medido (resposta ao impulso) ao modelo genérico | [V] |

---

### Fontes

Lidas diretamente (fetch ou PDF extraído):
- SSL X-Rack E Series Dynamics User's Guide: https://support.download.solidstatelogic.com/Legacy/User%20Manuals/82S6XR0L0A%20-%20X-Rack%20E%20Series%20Dynamics.pdf
- SSL Native v6.5 User Guide (Channel Strip, Drumstrip): https://www.solidstatelogic.com/assets/uploads/downloads/plug-ins/SSL%20Native%20v6.5%20-%20User%20Guide.pdf
- Drawmer DS201 Operator's Manual (2008): https://r2.gear4music.com/media/18/182269/download_182269.pdf
- SOS, Advanced Gating Techniques pt. 1 (P. White, 2001): https://www.soundonsound.com/techniques/advanced-gating-techniques-part-1
- SOS, Advanced Gating Techniques pt. 2 (Senior & White, 2001): https://www.soundonsound.com/techniques/advanced-gating-techniques-part-2
- SOS, Drawmer DS201 Dual Gate Tips (D. Mellor, 1996): https://www.soundonsound.com/techniques/drawmer-ds201-dual-gate-tips
- SOS, Production Techniques Using Gates & Expanders (M. Houghton, 2018): https://www.soundonsound.com/techniques/production-techniques-using-gates-expanders
- SOS, Using Your DAW's Gates & Expanders (2003): https://www.soundonsound.com/techniques/using-your-daws-gates-expanders
- SOS, Avid Pro Expander: https://www.soundonsound.com/techniques/avid-pro-expander
- SOS/Muzines, How It Works: the Noise Gate (Mellor, 1988): https://www.muzines.co.uk/articles/how-it-works-the-noise-gate/3994
- Yamaha LS9, Dynamics library list: https://www.manualsdir.com/manuals/600840/yamaha-ls9.html?page=227 e ?page=228
- Apple Logic Pro, Noise Gate: https://help.apple.com/logicpro/mac/10.4.5/en.lproj/lgcef1bec259.html
- MathWorks, Dynamic Range Control (cita Giannoulis et al.): https://www.mathworks.com/help/audio/ug/dynamic-range-control.html
- OpenAL Soft, core/mastering.cpp (smooth decoupled, crest factor): https://raw.githubusercontent.com/kcat/openal-soft/master/core/mastering.cpp
- Fórum JUCE (citação do artigo sobre o detector decoupled): https://forum.juce.com/t/peak-follower-with-independent-attack-release-and-level/55455
- Eichas & Zölzer, Modeling of an Optocoupler-Based Audio DRC Circuit (SPIE 2016): https://www.hsu-hh.de/ant/wp-content/uploads/sites/699/2017/10/Eichas-Modeling-of-an-optocoupler-based-audio-dynamic-range-control-circuit-99480W.pdf
- Yu & Fazekas, Sound Matching an Analogue Levelling Amplifier… (AES AIMLA 2025): https://arxiv.org/pdf/2509.10706
- UA, LA-2A Collection Tips & Tricks: https://www.uaudio.com/blogs/ua/la-2a-collection-tips-tricks
- UA, 1176 Collection Tips: https://www.uaudio.com/blogs/ua/1176-collection-tips
- Telefunken, Fairchild 670: https://www.telefunken-elektroakustik.com/product/fairchild-670
- Pulsar Audio, Variable-Bias Compression Explained: https://pulsar.audio/?p=237233
- Siliconix/Vishay AN105, FETs as Voltage-Controlled Resistors: https://www.vishay.com/docs/70598/70598.pdf

Usadas só pelo trecho da busca (sem leitura integral):
- AES E-Library, Giannoulis, Massberg & Reiss 2012 (registro e abstract): https://www.aes.org/e-lib/browse.cfm?elib=16354
- Avid Pro Tools, Dyn3 Expander/Gate Look Ahead: https://apps.avid.com/proToolsFirstHelp/version12.0/enu/Pro%20Tools%20First%20Help/Dyn3.42.27.html
- Waves SSL E-Channel manual (range 0–40 dB, padrão expander): https://manualshive.com/en/manual/waves/ssl-4000-e/3932093?page=9
- UA 1176LN, faixas de ataque e release: https://help.uaudio.com/hc/en-us/articles/34530260482324-1176-Classic-FET-Compressor-Manual
