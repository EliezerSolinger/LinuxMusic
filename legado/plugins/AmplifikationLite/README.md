# Amplifikation Lite (nativo)

Amp de guitarra nativo do LinuxMusic, modelado a partir do **Kuassa Amplifikation Lite** (VST gratuito,
2012). Ele foi medido **de fora, como caixa-preta**: sinais de teste entram no plugin original e a saída é
analisada. O binário nunca foi desmontado, porque o EULA proíbe.

- **Estudo completo:** `~/Desktop/AmplifikationLite-RE/NOTAS.md`, com medições, scripts e dados.
- **Arte (`*.png`):** é da Kuassa e foi usada com permissão do Arie (chairman e desenvolvedor de DSP da Kuassa).
  Convém guardar essa permissão por escrito.

## Controles (os 9 do original, com as mesmas faixas 0..1 e os mesmos padrões)

| Controle | Padrão | O que faz no modelo |
|---|---|---|
| Input | 0.2 | ganho proporcional ao knob (0 = mudo) |
| Channel | Clean | Clean / Rhythm / Lead (chave de 3 posições) |
| Gain | 0.5 | filtro do gain (bright cap): tabela de biquads, x = g no clean e g³ nos outros |
| Bass / Middle / Treble | 0.5 | tone stack TMB exato: R1 220k, R2 1M (taper v²), R3 10k, R4 47k, C1 470p, C2 100n, C3 22n |
| Presence | 0.5 | tabela exata de biquads P(v)/P(1), 101 posições |
| Volume | 0.5 | ganho proporcional ao knob, depois de toda a distorção |
| Cabinet | INT | **INT**: o gabinete interno, medido do original; **OFF**; **EXT**: o IR carregado |

- **Botão de carregar IR:** é um botão só e aparece apenas em EXT.
- **Chaves:** um clique sobe ou desce uma posição.
- **Posições na tela:** batem ao pixel com um screenshot do original.

## Cadeia de processamento (`AmpLiteDsp`, mono, a 48 kHz)

```
input -> filtro do gain -> passa-altas de entrada + shelf -> x K1
-> [oversampling 4x] estágio 1 (tabela da curva) [decimação]
-> x M -> tone stack -> passa-altas de acoplamento + passa-baixas (Miller)
-> FIR entre os estágios (decimação / correção de agudos)
-> [oversampling 4x] estágio 2 (curva + saturação de fluxo do transformador de saída) [decimação]
-> resposta do power amp (FIR de 4096 pontos, convolução particionada)
-> presence -> volume -> gabinete INT (2048 pontos: 256 diretos + o resto particionado, sem latência)
-> (no efeito) gabinete EXT
```

- **Taxa interna:** o modelo roda a 48 kHz, a taxa em que foi medido. Com o motor em outra taxa, o plugin
  reamostra com libsamplerate.
- **Latência:** ~450 amostras, informada pelo `latency()`.
- **Dados:** `AmpLiteData.h` é **gerado**. Não edite à mão; rode `export_cpp.py` do estudo:
  ```
  python export_cpp.py AmpLiteData.h
  ```
- **Reset:** o `reset()` leva tudo ao silêncio (projeto novo, exportação).
  Os testes `BuiltinPluginsTest::effectsFromRest(amplifikationlite)` e `effects(amplifikationlite)` passam.

## Fidelidade atual (C++ contra o plugin original, gain 1, TS 0.5)

Com o gabinete INT, que é o padrão:

| Canal | Pequeno sinal | Senoides | Dois tons | Notas e acordes | Média |
|---|---|---|---|---|---|
| Clean | ±0,02 dB | -51 dB | -45 dB | -29 a -30 dB | **-37 dB** |
| Rhythm | ±0,02 dB | -41 dB | -36 dB | -24 dB | **-29 dB** |
| Lead | ±0,07 dB | -33 dB | -21 dB | -21 a -22 dB | **-23 dB** |

Com o gabinete desligado (mais exigente, porque os agudos ficam expostos):

Os números são o ESR do nulo da forma de onda, medido pelo `compare.py` do estudo.

| Canal | Pequeno sinal (100 Hz–18 kHz) | Senoides | Dois tons | Notas e acordes | Média |
|---|---|---|---|---|---|
| Clean | ±0,03 dB | -46,6 dB | -41 dB | -21 dB | **-30 dB** |
| Rhythm | ±0,03 dB | -35,5 dB | -26,7 dB | -15 dB | **-21 dB** |
| Lead | ±0,07 dB | -27 dB | -19 dB | -15 dB | **-18 dB** |

- **Rhythm:** graves e médios ficam em -24 a -27 dB. O erro se concentra em 8–16 kHz, onde o nível bate
  (±1 dB) mas a forma de onda não.
- **Lead:** 3 estágios (o 1º e o 2º sem filtro relevante entre eles; o passa-altas do acoplamento foi fixado
  em 15 Hz, porque o ajuste periódico não enxerga DC). Ajuste: -26 dB nos harmônicos medidos.

## Pendências

1. **Lead:** chegar perto do ajuste (-26 dB) também no C++. Hoje o C++ fica ~3–6 dB abaixo em níveis
   médios, provavelmente pela decimação entre os estágios.
2. **Rhythm:** acertar os agudos dos sinais densos.
3. **INT:** o IR (`CabinetIr` em `AmpLiteData.h`) é o gabinete da Kuassa, medido pela razão gabinete ligado /
   desligado. Erro ≤ 0,15 dB de 20 Hz a 18 kHz, e é igual em qualquer ajuste. Permissão para distribuí-lo
   confirmada pelo usuário em 18/11, assim como a das imagens.
4. **Abaixo de 60 Hz:** o FIR do power amp erra ~1 dB em 30 Hz. Dá para trocar por um IIR mais uma linha
   de atraso, que também tiraria ~5 ms de latência.

## Arquivos

| Arquivo | Conteúdo |
|---|---|
| `AmplifikationLite.{h,cpp}` | o efeito: reamostragem, gabinete EXT, reset, latência |
| `AmplifikationLiteControls.{h,cpp}` | os parâmetros, salvar/carregar, painel, knob de filmstrip, chave de 3 posições |
| `AmpLiteDsp.{h,cpp}` | o modelo |
| `AmpLiteData.h` | constantes geradas |
| `*.png` | arte original (background, silverknob, silverswitch, irload, logo, about) |
| `../AmpAndIr/Convolver.*` | a convolução particionada (ganhou `reset()` para este plugin) |

## Para commitar (nota para o outro agente)

Tudo isto foi feito pela sessão do amp, ainda **sem commit**. Commite por caminho, sem `git add -A`:

```
git add plugins/AmplifikationLite cmake/modules/PluginList.cmake plugins/AmpAndIr/Convolver.h plugins/AmpAndIr/Convolver.cpp
```

- **`PluginList.cmake`:** a única mudança é a linha `AmplifikationLite`, logo depois de `AmpAndIr`.
- **`Convolver.*`:** só o `reset()` foi acrescentado.
- **Não entram no commit:** `AmplifikationLite_113b_Mac_VST.zip` e a pasta `AmplifikationLite_113b_Mac_VST/`
  na raiz. São o plugin original da Kuassa, cujo EULA proíbe redistribuir junto com outros produtos.
- Sugestão de mensagem: `Amplifikation Lite: a native amp, modeled on Kuassa's (black-box measurements)`.
