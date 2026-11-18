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
| Cabinet | OFF | **INT** passa como OFF (o IR interno é da Kuassa e falta permissão); OFF; **EXT** usa o IR carregado |

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
-> presence -> volume -> gabinete (EXT)
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

## Fidelidade atual (C++ contra o plugin original, gain 1, TS 0.5, gabinete desligado)

Os números são o ESR do nulo da forma de onda, medido pelo `compare.py` do estudo.

| Canal | Pequeno sinal (100 Hz–18 kHz) | Senoides | Dois tons | Notas e acordes | Média |
|---|---|---|---|---|---|
| Clean | ±0,03 dB | -46,6 dB | -41 dB | -21 dB | **-30 dB** |
| Rhythm | ±0,03 dB | -35,5 dB | -26,7 dB | -15 dB | **-21 dB** |
| Lead | exato | — | — | — | **provisório** |

- **Rhythm:** graves e médios ficam em -24 a -27 dB. O erro se concentra em 8–16 kHz, onde o nível bate
  (±1 dB) mas a forma de onda não.
- **Lead:** usa as curvas do Rhythm com os filtros lineares exatos do Lead, então só o pequeno sinal é exato.

## Pendências

1. **Lead:** medir e ajustar os 3 estágios dele (2 antes do TS e 1 depois).
2. **Rhythm:** acertar os agudos dos sinais densos.
3. **INT:** depende da permissão da Kuassa para usar o IR de gabinete deles.
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
