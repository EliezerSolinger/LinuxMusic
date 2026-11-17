# Guitarix nativo no LinuxMusic: pesquisa

Escrito em 17/11/2026. Fonte principal: o repositório oficial
[brummer10/guitarix](https://github.com/brummer10/guitarix) (ramo `master`, commit `2a357f65`, de 31/10/2026; versão
declarada no `wscript`: 0.47.0), lido numa cópia temporária fora do projeto. O projeto também está no
[SourceForge](https://sourceforge.net/projects/guitarix/). Os caminhos abaixo são relativos a `trunk/` no repositório.

Objetivo do pedido: trazer as cabeças, os amplificadores, as caixas, os pedais e os efeitos do Guitarix como efeitos
nativos compilados dentro do LinuxMusic (no caminho de renderização, no mesmo processo), sem hospedar o programa
Guitarix nem os seus pacotes LV2.

**Decisão do usuário (17/11):** no navegador de efeitos, o Guitarix é uma categoria própria, com as peças como
subplugins dela (por exemplo: amplificadores, cabeçotes, gabinetes, pedais).

**Decisão do usuário (17/11):** portar o mesmo código com os mesmos nomes do Guitarix: unidades, parâmetros e
presets. Isso vale por cima da sugestão de renomear feita mais abaixo.

**Decisão do usuário (17/11), som e visual:** o DSP é o mesmo código do Guitarix, para soar igual. A
interface tem a mesma aparência do Guitarix (imagens, botões, cores, disposição), desenhada pelos nossos widgets Qt
no rack de efeitos, sem o xputty nem o cairo.

**Decisão do usuário (17/11), licença:** vem tudo, inclusive o zita-resampler e o zita-convolver (GPL-3). O
LinuxMusic é GPL-2 ou posterior, então o programa com eles passa a ser distribuído como GPL-3. Obrigatório citar o
Guitarix e os autores nos créditos e manter os avisos de copyright. Isso vale por cima das sugestões de trocar o
reamostrador mais abaixo.

## Resumo

- **Licença: compatível**, com duas exceções que é preciso evitar. O código escrito para o Guitarix é GPL-2.0 ou
  posterior. Só três peças são GPL-3/LGPL-3: o `zita-convolver`, o `zita-resampler` e o `abgate.cc`. Nenhuma das três
  precisa vir junto: temos o nosso convolvedor (FFTW) e os nossos reamostradores (hiir e libsamplerate).
- **Não precisamos do Faust na compilação.** O C++ gerado a partir dos `.dsp` está no repositório (`src/faust-generated`,
  `src/plugins/generated`, `src/LV2/faust-generated`). O mesmo vale para as tabelas das válvulas, as IRs das caixas e
  a tabela do TS9.
- **Cada unidade é uma classe pequena e autossuficiente**, com `init(taxa)`, `clear_state_f()` e
  `compute(n, entrada, saída)`. Os parâmetros são `float` registrados por ponteiro. Para extrair uma unidade, basta
  trocar o "invólucro" `PluginDef` por uma classe nossa e o reamostrador do Guitarix pelo nosso.
- **Inventário:** 18 pré-amplificadores valvulados, 6 estágios de potência, 27 tonestacks, 17 caixas (IR embutida) e
  10 IRs de pré, 1 pré JCM800 por simulação de circuito, além de cerca de 80 pedais e efeitos.
- **Piloto sugerido:** o Tube Screamer (`ts9sim`), e logo depois a cadeia "amp 12AX7 + tonestack + caixa".

## 1. Licenças

### O que diz o projeto

O arquivo [`trunk/COPYING`](https://github.com/brummer10/guitarix/blob/master/trunk/COPYING) começa com uma nota
anterior ao texto da GPL. Em resumo, ela diz:

- o código escrito para o Guitarix é "GPL versão 2 ou posterior";
- o Guitarix *como um todo* é distribuído sob GPL-3 ou posterior, porque inclui algumas peças GPL-3+ ou LGPL-3+;
- as peças que causam isso são:
  - LGPL-3+: `src/plugins/abgate.cc`, de Antanas Bružas;
  - GPL-3+: `zita-resampler` 1.1 e `zita-convolver` 3, de Fons Adriaensen;
- todos os outros arquivos são GPL-2+, e o cabeçalho de cada arquivo vale para aquele arquivo, não para o projeto.

Os autores são Hermann Meyer, James Warden, Andreas Degert e Pete Shorthose.

Procurei por "version 3", "GPLv3" e "Lesser General" em todo o `src/` (fora o `xputty` da interface LV2). Só
apareceram os arquivos já citados (`zita-convolver*`, `zita-resampler-1.1.0`, `abgate.cc`), o `ladspa.h` (LGPL, que é
só cabeçalho da API e não interessa aqui) e um `.cc` gerado pelo gperf para o JSON-RPC (também não interessa).

### Parte por parte

| Parte | Onde | Licença | Situação para nós (GPL-2+) |
|---|---|---|---|
| Fontes Faust dos efeitos e amps | `src/faust/*.dsp`, `src/plugins/*.dsp`, `src/LV2/faust/*.dsp`, `guitarix.lib`, `trany.lib` | GPL-2+ (nota do `COPYING`). Alguns `.dsp` declaram outra coisa, como o `freeverb.dsp`, que declara "BSD" | Compatível |
| C++ gerado pelo Faust | `src/faust-generated/*.cc`, `src/plugins/generated/*.cc`, `src/LV2/faust-generated/*.cc` | Herda a licença do `.dsp`. A saída do compilador Faust não fica presa à licença do compilador, e as bibliotecas padrão do Faust (`stdfaust.lib`) trazem exceção para o código gerado ([faustlibraries](https://github.com/grame-cncm/faustlibraries)) | Compatível |
| Tabelas das válvulas (12AX7, 12AU7, 12AT7, 6V6, 6DJ8, 6C16, 7199P, KT88, EL34) e tabelas de potência | `src/gx_head/engine/tabels/*.cc` e `*.h`, `src/plugins/7199P.cc`, `KT88.cc`, `SVEL34.cc` | GPL-2+ (cabeçalho do `valve.h`). Geradas por `tools/gx_tube_transfer.py`, também GPL-2+ | Compatível |
| IRs das caixas e dos pré, embutidas em C | `src/gx_head/engine/gx_cabinet_data.cc`, `gx_preamp_data.cc` | GPL-2+ (cabeçalho do arquivo) | Compatível |
| Tabela do TS9 | `src/plugins/ts9nonlin.cc` (gerada por `tools/ts9sim.py`) | GPL-2+ | Compatível |
| Pré JCM800 (simulação de circuito) | `src/plugins/jcm800pre.cpp` (gerado por um script; usa Eigen) | GPL-2+. O Eigen é MPL-2.0 e já vem conosco junto do NAM | Compatível |
| Univibe | `src/plugins/vibe.cc` | Ryan Billing (Rakarrack): a licença mudou de GPL-2 para GPL-2+ com permissão dele, segundo o próprio cabeçalho | Compatível |
| Detune (`smbPitchShift`) | `src/headers/gx_internal_plugins.h` | Stephan M. Bernsee, "Wide Open License", que é permissiva | Compatível (manter o aviso) |
| Zita Rev1 em Faust | `src/plugins/zita_rev1.dsp` | Copia o `re.zita_rev1_stereo` das bibliotecas do Faust (licença no estilo MIT/STK). Não é o código C++ do Fons | Compatível |
| **zita-convolver** 3 | `src/zita-convolver*` | **GPL-3+** | **Não trazer.** Usar o nosso `Convolver` (FFTW) |
| **zita-resampler** 1.1 | `src/zita-resampler-1.1.0` | **GPL-3+** | **Não trazer.** Usar hiir ou libsamplerate |
| **abgate** | `src/plugins/abgate.cc` | **LGPL-3+** | **Não trazer** (o gate é simples de refazer) |
| IRs em WAV "BestPlugins" | `IR/BestPlugins_Amps/*.wav` (21) e `IR/BestPlugins_Bands/*.wav` (cerca de 70) | GPL-2+, de David Fau Casquel ([LICENSE](https://github.com/brummer10/guitarix/blob/master/trunk/IR/BestPlugins_Amps/LICENSE)) | Compatível. Atenção: os nomes dos arquivos usam marcas e nomes de bandas, então convém renomear |
| `greathall.wav` | `IR/` | Sem arquivo de licença próprio, então vale a nota geral (GPL-2+) | Provavelmente compatível. Confirmar com o autor antes de distribuir |
| Presets de fábrica | `factorysettings/*.gx` etc. | Sem cabeçalho, então vale a nota geral (GPL-2+) | Compatível, mas o formato é do Guitarix. Melhor refazer como presets nossos |
| NAM e RTNeural | `src/NAM`, `src/RTNeural` (submódulos) | MIT e BSD-3 | Já temos o NAM (veja a seção 5) |

**Conclusão:** tudo o que interessa para portar (DSP gerado, tabelas e IRs) é GPL-2+ e pode entrar num programa
GPL-2.0 ou posterior. O cuidado é técnico, não jurídico: vários arquivos gerados chamam o `FixedRateResampler`
(`src/headers/gx_resampler.h`), que é um invólucro do zita-resampler GPL-3+. É preciso trocar essa chamada pelo
nosso reamostrador, senão o binário passaria a ser GPL-3. Os créditos (copyright dos autores, Ryan Billing,
Bernsee) devem ir para o cabeçalho de cada arquivo portado e para o `README`/créditos.

**Marcas:** o Guitarix usa nomes como "Tube Screamer", "DS1", "Rat", "Big Muff", "JCM-800 Style" e "Mesa Boogie
Style". Para a interface, o melhor é usar nomes descritivos nossos, como o próprio Guitarix já faz em parte com
"... Style", "Inspired by". Isso não é questão de licença do código, mas é bom cuidado.

## 2. Como o DSP é construído

- **Faust → C++ pelo `tools/dsp2cc`.** O script em Python chama o compilador Faust e embrulha a saída numa classe
  `Dsp : public PluginDef` dentro de um `namespace` com o nome da unidade. O `wscript` fixa a versão do Faust usada
  para os arquivos do repositório (`check_in_faust_version = '2.79.3'`). Sem Faust instalado, a compilação usa os
  arquivos prontos ("Use prebuild faust files").
- **Os arquivos gerados estão no repositório.** São 140 em `src/faust-generated`, 41 em `src/plugins/generated` e
  187 em `src/LV2/faust-generated`. Não precisamos do Faust na nossa compilação. Só precisaríamos dele para mudar um
  `.dsp`, e mesmo assim só para regenerar, nunca na compilação normal.
- **Precisão:** os arquivos gerados usam `double` nas variáveis de estado e nas constantes, e `FAUSTFLOAT` = `float`
  na entrada e na saída.
- **Taxa interna fixa.** Os amps valvulados, os estágios de potência e os pedais por simulação de circuito declaram
  `samplerate 96000`. Eles reamostram a entrada para 96 kHz (`smp.up`), processam e voltam (`smp.down`). Isso
  acontece em 44 arquivos gerados:
  - os 18 `gxamp*`;
  - `champ`, `epiphone`, `princeton`, `orangedarkterror`, `plexiel34`, `supersonic`;
  - `fuzzface`, `gx_ampmodul`;
  - `ts9sim`, `bossds1`, `bmp`, `aclipper`, `mxrdist`, `muff`, `fumaster`, `cstb`, `fuzzfacefm`/`rm`, `bfuzz`,
    `buzz`, `eldist`, `foxeylady`, `ruin`, `scream`, `susta`, `mbd`.

  O buffer de 96 kHz é um VLA (`FAUSTFLOAT buf[smp.max_out_count(count)]`), uma extensão do GCC e do Clang. No
  porte, ele deve virar um buffer alocado no `init`.

### Dependências, por parte

| Parte | Precisa de |
|---|---|
| DSP gerado (amps, tonestacks, pedais, modulação, delays, reverbs Faust) | Só `<cmath>`, `<algorithm>` e os cabeçalhos de apoio `gx_faust_support.h` (macros `FAUSTFLOAT`, `faustpower`) e `gx_plugin.h` (struct `PluginDef`, `ParamReg`). Os amps incluem também `valve.h`, as tabelas e às vezes `clipping.h`/`trany.h` |
| Reamostragem para 96 kHz | No Guitarix, o zita-resampler (GPL-3+). **Nós:** hiir (`include/OversamplingHelpers.h`, já usado) ou libsamplerate |
| Caixas e pré (IR) | No Guitarix, `GxSimpleConvolver` sobre o zita-convolver + FFTW3f. A IR é moldada por um filtro Faust (`cabinet_impulse_former`: graves/agudos). **Nós:** o `Convolver` do AmpAndIr (FFTW, partições uniformes) |
| Convolvedor de IR de arquivo (jconv) | zita-convolver, libsndfile, glibmm. **Nós:** já temos (AmpAndIr) |
| Pré JCM800 | Eigen (já está no submódulo do NAM) |
| Detune | FFTW3 |
| Motor, presets, interface | glibmm/sigc++, boost (iostreams, format), GTK/gxw, JACK. **Nada disso vem junto** |

## 3. Inventário

A contagem de parâmetros vem de `registerFloatVar` e `registerIntVar` em cada arquivo gerado. Os IDs são os do
Guitarix.

### 3.1 Pré-amplificadores valvulados ("amp stack")

Ficam em `src/faust/gxamp*.dsp` e `src/faust-generated/gxamp*.cc`, e a lista está em `builtin_amp_plugins`
([`gx_engine.cpp`](https://github.com/brummer10/guitarix/blob/master/trunk/src/gx_head/engine/gx_engine.cpp)).

Cada modelo tem 3 estágios de triodo, resolvidos pelas tabelas `Ftube`/`Ranode` com realimentação de catodo, mais
um "multiband distortion" opcional (`gxdistortion`) que entra antes. Os parâmetros são 6:

- `amp2.stage1.Pregain`;
- `amp2.stage2.gain1`;
- `gxdistortion.drive`;
- `gxdistortion.wet_dry`;
- `amp.highgain` (deixa passar abaixo de 31 Hz).

Os modelos, por ID:

| Grupo | Modelos |
|---|---|
| Só pré | `12ax7` (gxamp), `12AU7` (gxamp3), `12AT7` (gxamp14), `6DJ8` (gxamp10), `6C16` (gxamp18), `6V6` (gxamp2) |
| Com realimentação | `12ax7 feedback` (gxamp9), `12AU7 feedback` (gxamp11), `12AT7 feedback` (gxamp17), `6DJ8 feedback` (gxamp13) |
| Pré + master 6V6 | `pre 12ax7/ master 6V6` (gxamp5), `pre 12AU7/…` (gxamp4), `pre 12AT7/…` (gxamp15), `pre 6DJ8/…` (gxamp12) |
| Pré + push-pull 6V6 | `pre 12ax7/ push-pull 6V6` (gxamp7), `pre 12AU7/…` (gxamp8), `pre 12AT7/…` (gxamp16), `pre 6DJ8/…` (gxamp6) |
| Sem amp | `noamp` (gxnoamp) |

Em volta do amp, a cadeia fixa do Guitarix tem:

- antes: `noise_shaper` e `noisegate`;
- depois: `softclip` (`amp.clip`), `bassbooster`, `gx_ampout` (volume mestre) e o "contrast" (uma convolução curta
  de presença: `contrast_ir_desc` em `gx_cabinet_data.cc`, mais o `presence_level`).

### 3.2 Estágios de potência ("poweramp")

Os 6 modelos estão em `builtin_poweramp_plugins` (`src/faust/<nome>.dsp`). Usam tabelas próprias de transferência
positiva e negativa (`tabels/*_table.h`, `*_neg_table.h`). Os parâmetros são 2: `poweramp.Pregain` e
`poweramp.Gain`.

- `champ`: Single ended 6V6GT;
- `epiphone`: Single ended EL84;
- `princeton`: Single ended 6V6;
- `orangedarkterror`: Push Pull EL84;
- `plexiel34`: Push Pull EL34;
- `supersonic`: Push Pull 6L6.

### 3.3 Tonestacks

São 27 modelos (`src/faust/tonestack_*.dsp`, `builtin_tonestack_plugins`). Cada um é a rede passiva clássica de
graves, médios e agudos, com os componentes do circuito modelado (o modelo de Yeh). Os parâmetros são 3:
`amp.tonestack.Bass`, `Middle` e `Treble`.

Modelos: default, Bassman, Twin Reverb, Princeton, JCM-800, JCM-2000, JTM-45, M-Lead, M2199, AC-30, AC-15, Mesa
Boogie, SOL 100 (Soldano), Junior, Fender, Deville, Gibsen, Engl, Ampeg, Rev.Rocket, Ibanez, Roland, Peavey, Triple
Giant (Bogner), Trio Preamp (Groove), Hughes&Kettner, MIG 100 H (Sovtek). Todos aparecem na interface com o sufixo
"… Style".

### 3.4 Caixas e pré por IR

- **Caixas (`CabinetConvolver`)**: 17 IRs curtas embutidas como arrays C (`CabDesc_imp<N>`, de 68 a 1000 amostras a
  48 kHz) em [`gx_cabinet_data.cc`](https://github.com/brummer10/guitarix/blob/master/trunk/src/gx_head/engine/gx_cabinet_data.cc):
  4x12, 2x12, 1x12, 4x10, 2x10, HighGain, Twin, Bassman, Marshall, AC-30, Princeton, A2, 1x15, Mesa Boogie,
  Briliant, Vitalize e Charisma.
  - Parâmetros: `cab.Level`, `cab.bass`, `cab.treble` e a seleção.
  - O grave e o agudo não filtram o áudio: filtram a própria IR (`cabinet_impulse_former`) antes de ela ir ao
    convolvedor.
  - Existe também a versão estéreo.
- **Pré por IR (`PreampConvolver`)**: 10 IRs em `gx_preamp_data.cc` (AC30, Bassman, Tube, Fender, JCM800, JTM45, Mesa
  Boogie, Boutique, Ampeg, Rectifier), com o mesmo esquema de Level, graves e agudos.
- **Convolvedor genérico (jconv)**: carrega WAV de fora (por padrão o `greathall.wav`; há também os pacotes
  BestPlugins). Isso equivale ao nosso AmpAndIr.

### 3.5 Pedais e efeitos

Unidades em Faust (`src/faust` e `src/plugins`; o gerado fica em `faust-generated` e `plugins/generated`):

| Categoria | Unidades (ID: nome) e parâmetros principais |
|---|---|
| Overdrive e distorção | `ts9sim` Tube Screamer (drive, level, tone; tabela `ts9nonlin`); `bossds1` DS1 (drive, Tone, Level); `aclipper` "Rat" (drive, tone, level); `mxrdis` MXR Distortion+ (drive, Volume); `coverd` C Overdrive; `eldist` Electra Distortion; `bboom` Bass Boom; `overdrive` (drive, wet_dry); `gx_distortion` multibanda (18 parâmetros); `mbd` MultiBand Distortion (25); `mbclip` MultiBand Clipper (20); `distortion2` WaveSharper; `thick_distortion`; `bitdowner`; `ampmodul` Postamp (8) |
| Fuzz | `fuzzface` (Fuzz, Level); `fuzzfacefm` Fuller; `fuzzfacerm` Mayer; `bmp` Big Muff Pi (Sustain, Tone, Volume); `muff`; `bfuzz` Bass Fuzz; `buzz` Buzz Box; `astrofuzz`; `axface`; `ffreak`; `foxeylady`; `fumaster`; `fuzzdrive`; `hornet`; `ruin`; `scream` Screaming Bird; `susta` Sustainer; `cstbt1` Tone Blender |
| Boosters e buffers | `rangem` Rangemaster (Boost, wet_dry); `lpbboost` LPB-1; `buffb` Buffer Booster; `trbuff` Transistor Buffer; `hfb` HF Brightener; `hogsfoot`; `mole`; `highbooster`; `bassbooster`; `GCB_95` |
| Wah | `crybaby` (wah, level, wet_dry) e `autowah` (o seletor "crybaby"); 10 modelos de wah: colwah, dallaswah, foxwah, jenwah, maestrowah, selwah, voxwah (V847), rolwah, colbwah, jenbasswah; `rolandwah` Double Beat; `dunwah`/`fuzzwah` (em `src/plugins`) |
| Dinâmica | `compressor` e `expander` (attack, release, threshold, ratio, knee, makeup); `mbc`/`mbcs` compressor multibanda de 5 bandas (39); `noisegate`; `hardlim`. O `abgate` fica de fora por ser LGPL-3 |
| Modulação | `chorus`/`chorus_mono`; `mbchor` chorus multibanda; `flanger`/`flanger_mono`; `flanger_mono_gx`; `phaser`/`phaser_mono`; `tremolo` (depth, freq, SINE); `univibe` (vibe.cc, da Rakarrack); `ring_modulator`(`_st`); `moog` (filtro) |
| Delay e eco | `delay`; `echo`; `stereodelay`; `stereoecho`; `dide`/`didest` Digital Delay (9: bpm, notes, feedback, filtros, freeze); `duckDelay`(`St`); `mbdel` e `mbe` multibanda; `reversedelay` (C++) |
| Reverb | `freeverb`; `stereoverb`; `dattorros_progenitor` (plate de Dattorro, 10); `zita_rev1` (11) |
| EQ e tom | `eq` Peak EQ (12); `eqs` Scaleable EQ (30); `graphiceq` (22); `tonemodul` 3 bandas; `baxandall`; `biquad`; `low_highpass`; `IR` (filtro ressonante "ImpulseResponse"); `feedback`; `bassEnhancer`; `panoram_enhancer` |
| Outros | `detune` (smbPitchShift, FFTW); afinador, looper, gravador, sequenciador de bateria e osciloscópio (fora do escopo) |

Há também **unidades que só existem como LV2** (`src/LV2/*.lv2`, com DSP em `src/LV2/faust`):

- GX Alembic (pré de estúdio F-2B);
- Redeye Chump, Big Chump e Vibro Chump;
- GX Metal Amp e Metal Head;
- Studio Pre;
- W20;
- Tape (gxtape) e Tube Delay, Tube Tremolo, Tube Vibrato;
- EchoCat;
- TiltTone;
- Room Simulator;
- ShimmiZita;
- OC-2 (oitavador);
- Switched Tremolo;
- uniBar;
- MB Reverb.

O DSP delas está igualmente pronto em `src/LV2/faust-generated`, num invólucro diferente (classe com
`connect(port, data)` no lugar de `ParamReg`), e é igualmente fácil de extrair. A lista comentada está em
[`documentation/lv2-plugins-overview.md`](https://github.com/brummer10/guitarix/blob/master/documentation/lv2-plugins-overview.md).

O **pré JCM800** (`JCM800Pre`, `src/plugins/jcm800pre.cpp`, cerca de 4150 linhas) não é Faust. É uma simulação de
circuito gerada por script (o "DK simulator", em `tools/ampsim`), que resolve o circuito com Eigen.

## 4. Quão difícil é extrair uma unidade

A interface gerada é a mesma em todas as unidades. Veja, por exemplo,
[`faust-generated/gxamp.cc`](https://github.com/brummer10/guitarix/blob/master/trunk/src/faust-generated/gxamp.cc):

- `namespace <unidade> { class Dsp : public PluginDef { … } }`, com o estado como membros (`double fRecN[2]`,
  `fConstN`...);
- `void init(unsigned int taxa)`: calcula as constantes da taxa e chama `clear_state_f()`;
- `void clear_state_f()`: zera os históricos;
- `void compute(int n, FAUSTFLOAT* in0, FAUSTFLOAT* out0)`, mono; as unidades estéreo têm
  `(n, in0, in1, out0, out1)`;
- os parâmetros:
  - cada controle é um `FAUSTFLOAT fVsliderN` mais um ponteiro `fVsliderN_`;
  - `register_par(const ParamReg&)` chama `registerFloatVar("id", …, &var, padrão, mín, máx, passo)` e guarda o
    ponteiro devolvido;
  - dentro do `compute`, macros `#define fVslider0 (*fVslider0_)` leem pelo ponteiro;
  - a suavização dos controles já está no DSP (`fRec = 0.001·alvo + 0.999·fRec`);
- o código de apoio é mínimo: `gx_faust_support.h` (cerca de 40 linhas úteis) e a struct `PluginDef`/`ParamReg` de
  `gx_plugin.h`, que podemos substituir por um mini-cabeçalho nosso.

### O que o porte de uma unidade exige

1. Copiar o `.cc` gerado para `plugins/<Rack>/dsp/`, sem mexer no algoritmo.
2. Trocar `PluginDef` por uma base nossa, sem virtuais no caminho quente. Duas opções:
   - um *shim* `gx_plugin.h` mínimo, em que `ParamReg` liga cada ID a um `FloatModel` nosso (ou só a um `float`
     que o efeito atualiza a cada bloco);
   - ou um script que reescreve o invólucro, preferível para muitas unidades.
3. Trocar o `gx_resample::FixedRateResampler` (zita, GPL-3) por um nosso com a mesma interface
   (`setup`, `up`, `down`, `max_out_count`), em cima do hiir: 96 kHz a partir de 48 kHz é exatamente 2x. Se a taxa
   do motor não for 48 kHz, há duas saídas:
   - libsamplerate em fluxo, como o AmpModel já faz;
   - ou rodar o DSP na taxa 2x do motor, com o risco de mudar o timbre dos modelos fixados em 96 kHz.
4. Tirar o VLA e pré-alocar o buffer no `init` com o tamanho máximo do bloco.
5. Para os amps: trazer `valve.h` e as tabelas (cerca de 1600 linhas cada válvula, só dados), mais
   `clipping.h`/`trany.h` quando usados.
6. Para as caixas: as IRs de `gx_cabinet_data.cc` mais o `cabinet_impulse_former` (Faust, pronto). O conjunto vai
   para o nosso `Convolver`, refeito fora da linha de áudio quando o grave ou o agudo mudam, como já fazemos no
   AmpAndIr com o `Handoff`.

**Esforço estimado:** uma unidade Faust simples leva algumas horas, quando o invólucro já existe. Os amps valvulados
levam cerca de 1 dia para o primeiro, por causa das tabelas e da reamostragem, e minutos para cada um dos outros 17.
As caixas levam cerca de 1 dia (convolução e moldagem da IR). O Univibe e o JCM800 Pre levam 1 a 2 dias cada (C++ à
mão; Eigen).

## 5. O que o LinuxMusic já tem

| Peça nossa | O que faz | Relação com o Guitarix |
|---|---|---|
| `plugins/AmpAndIr` (Amp & IR) | Modelo NAM (NeuralAmpModelerCore, MIT, com Eigen) mais convolução de IR de arquivo (`Convolver.cpp`, partições uniformes, FFTW), com entrada, saída e W/D. Troca de modelo e de IR sem trava (`Handoff`). Reamostra o NAM com libsamplerate | Cobre o "NAM module" e o "jconv" do Guitarix. O `Convolver` serve para as 17 caixas e as 10 IRs de pré do Guitarix: só falta embutir as IRs e o moldador de graves/agudos |
| `src/core/AnalogChannelStrip` + `GainStage`/`TubeStage` | Faixa do mixer analógico: estágio de ganho plano, op-amp ou válvula 12AX7 (Dempwolf-Zölzer, resolvido por amostra), com sobreamostragem por hiir | É um modelo de válvula próprio, de console (pouca distorção). O Guitarix usa tabelas pré-calculadas de várias válvulas e tem 3 estágios por amp. Não se substituem; as tabelas do Guitarix podem até servir de referência para mais válvulas no `TubeStage` |
| `include/OversamplingHelpers.h` (hiir) | Sobe e desce a amostragem em 2x, 4x... | Substituto direto do zita-resampler para os amps e pedais de 96 kHz |
| `plugins/Amplifier`, `WaveShaper`, `Bitcrush`, `SlewDistortion` | Ganho e distorções genéricas do LMMS | Não modelam pedal nem válvula |
| `plugins/Compressor`, `DynamicsProcessor`, `LOMM` | Dinâmica | Sobrepõem-se ao compressor e ao expander do Guitarix; o multibanda `mbc` é mais próximo do LOMM |
| `plugins/Delay`, `MultitapEcho`, `Flanger`, `ReverbSC`, `Eq`, `CrossoverEQ`, `DualFilter`, `StereoEnhancer` | Efeitos genéricos | Sobrepõem-se a delay, flanger, reverb e EQ. O que o Guitarix acrescenta de novo são os de "sabor guitarra": wahs, univibe, tremolo, phaser, Dattorro, Zita Rev1, tonestacks |

**Em resumo:** o que falta mesmo é a cadeia de guitarra (pedais, pré valvulado, potência, tonestack, caixa). O
NAM, a convolução e a sobreamostragem já existem e devem ser reaproveitados, e não trazidos de novo do Guitarix.

## 6. Plano de porte em fases

**Fase 0: base comum (sem som novo)**

- Plugin novo, por exemplo `plugins/GuitarRack`, com um diretório `dsp/gx/` para os arquivos do Guitarix, cada um
  com cabeçalho de copyright e de licença restaurado (os gerados não trazem cabeçalho).
- Montar a base com:
  - um `gx_shim.h` com `FAUSTFLOAT`, `faustpower`, `PluginDef` e `ParamReg` mínimos;
  - um `FixedRateResampler` nosso sobre o hiir (2x de 48 para 96 kHz) com fallback em libsamplerate;
  - um teste que roda um seno e compara com uma saída de referência gerada uma vez pelo Guitarix.
- Um script em `tools/` que importa um `.cc` gerado e aplica as trocas (o invólucro, o VLA, o namespace), para não
  editar 100 arquivos à mão.

**Fase 1: piloto, o Tube Screamer (`ts9sim`)**

Motivos para começar por ele:

- tem 3 parâmetros, é mono e é muito conhecido;
- passa por tudo o que é difícil (os 96 kHz, a tabela `ts9nonlin`), mas é pequeno (cerca de 400 linhas).

Critérios de aceitação:

- resposta igual à do Guitarix, por comparação numérica a 48 kHz;
- sem alocação na linha de áudio;
- latência declarada (o atraso dos filtros do hiir);
- carga de CPU medida.

**Fase 2: a cadeia de amp**

- Os 18 pré + noamp, os 6 estágios de potência e os 27 tonestacks como **um único efeito "Amp"**, com seletores
  (pré, potência, tonestack) e os knobs Gain/Pregain, Drive, Bass/Middle/Treble, Master e Presence.
- Dentro dele, a ordem do Guitarix: noise gate → pré → tonestack → potência → caixa → contrast.

**Fase 3: caixas**

- As 17 caixas e as 10 IRs de pré, embutidas, num seletor do Amp (e também como efeito separado).
- Usam o `Convolver` do AmpAndIr (vale extrair o `Convolver` para `src/core` ou para uma biblioteca comum) e o
  `cabinet_impulse_former` para graves e agudos.
- Os WAV BestPlugins podem ser instalados como IRs de usuário do AmpAndIr, renomeados.

**Fase 4: pedais, por categoria**

- Primeiro overdrive e fuzz, depois wah, depois modulação, delay e reverb. Os simples são só Faust; o univibe e o
  reversedelay são C++.
- Os efeitos que já temos (delay simples, freeverb, eq) só entram se trouxerem algo novo.
- Por último as unidades só-LV2 (Redeye Chumps, Metal Head, Alembic, Studio Pre, Tape, Tube Delay, OC-2) e o
  JCM800 Pre (Eigen).

**Fase 5: interface num rack compacto**

- Cada unidade vira uma "faixa" de altura fixa no rack de efeitos: nome à esquerda, liga/desliga, 2 a 5 knobs
  (`Knob` com `FloatModel`) e, quando houver, um `ComboBox` de modelo.
- A descrição dos controles sai direto do `register_par` (ID, padrão, mín, máx, passo), então dá para gerar o
  diálogo automaticamente para os pedais. Só o Amp e as caixas merecem um painel feito à mão.
- As unidades com muitos parâmetros (mbc com 39, eqs com 30, graphiceq com 22) abrem um painel expandido.

Duas decisões para registrar:

1. **Um plugin com sub-plugins.** Melhor um "GuitarRack" com uma lista de sub-plugins (como o LADSPA e o LV2 fazem
   no LMMS) do que 100 plugins separados. Assim a lista de efeitos não incha e a compilação fica num lugar só.
2. **Nada de GPL-3 no binário.** O plano não depende de nenhum código GPL-3 (zita-convolver, zita-resampler,
   abgate). Isso deve ficar escrito em `docs/pendencias.md` ao começar, para ninguém trazer o reamostrador do
   Guitarix por engano.
