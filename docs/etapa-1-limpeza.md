# Etapa 1: remover plugins e funcionalidades não usados

## Objetivo

Reduzir o LinuxMusic ao que realmente será usado. Menos código significa:
- compilação mais rápida;
- menos dependências para instalar no Linux;
- menos lugares onde as etapas 2 e 3 precisam mexer.

## Como trabalhar

1. Decidir, item por item, o que fica e o que sai (tabelas abaixo; a coluna **Decisão** fica a cargo do autor do projeto).
2. Remover em commits pequenos, um plugin ou funcionalidade por commit, para facilitar reverter se algo fizer falta.
3. A cada remoção:
   - tirar o plugin de `plugins/` e da lista em `cmake/modules/PluginList.cmake`;
   - remover presets e samples ligados a ele em `data/`;
   - remover dependências e submódulos (`.gitmodules`) que ficarem sem uso;
   - verificar se algum projeto demo em `data/projects` depende dele.
4. Compilar e rodar os testes (`ctest`) depois de cada remoção.

## Inventário de plugins

A coluna **Sugestão** é apenas um ponto de partida para a discussão.

### Instrumentos

| Plugin | O que é | Sugestão | Decisão |
|---|---|---|---|
| TripleOscillator | Sintetizador subtrativo de 3 osciladores | Manter (núcleo) | |
| AudioFileProcessor | Sampler de um arquivo de áudio | Manter (núcleo) | |
| Sf2Player | Player de SoundFont2 (FluidSynth) | Manter | |
| ZynAddSubFx | Sintetizador completo (submódulo grande) | Avaliar | |
| Kicker | Gerador de bumbo | Avaliar | |
| SlicerT | Fatiador de loops | Avaliar | |
| Lb302 | Imitação incompleta do TB-303 | Avaliar | |
| Monstro, Watsyn, Organic, BitInvader, Xpressive | Sintetizadores nativos variados | Avaliar | |
| Vibed | Modelagem de cordas | Avaliar | |
| Stk | Instrumentos físicos (biblioteca STK) | Avaliar | |
| GigPlayer | Player de arquivos GIG (libgig) | Avaliar | |
| Patman | Player de GUS patches | Candidato a remover | |
| Sfxr | Efeitos sonoros de videogame | Candidato a remover | |
| FreeBoy, Nes, Sid, OpulenZ | Emuladores de chips de videogame e OPL2 | Candidatos a remover | |

### Efeitos

| Plugin | O que é | Sugestão | Decisão |
|---|---|---|---|
| Eq | Equalizador paramétrico | Manter (núcleo) | |
| Compressor | Compressor | Manter (núcleo) | |
| Delay, ReverbSC | Delay e reverb | Manter | |
| Amplifier, StereoEnhancer, StereoMatrix | Utilitários de ganho e estéreo | Avaliar (podem virar parte do canal do mixer na etapa 2) | |
| SpectrumAnalyzer, Oscilloscope, Vectorscope | Medidores visuais | Avaliar | |
| DynamicsProcessor, LOMM, CrossoverEQ, MultitapEcho, Flanger, DualFilter | Efeitos variados | Avaliar | |
| Bitcrush, WaveShaper, SlewDistortion, Dispersion, FrequencyShifter, GranularPitchShifter | Efeitos criativos | Avaliar | |
| BassBooster | Reforço de graves | Candidato a remover (o Eq cobre) | |
| PeakControllerEffect | Controlador por envelope de pico | Avaliar | |
| TapTempo | Ferramenta de tap tempo | Avaliar | |

### Hosts de plugins externos

| Plugin | O que é | Sugestão | Decisão |
|---|---|---|---|
| Lv2Effect, Lv2Instrument | Host de plugins LV2 | Manter (é o padrão de plugins no Linux) | |
| LadspaEffect, LadspaBrowser | Host de plugins LADSPA e os plugins LADSPA embutidos (calf, swh, tap, caps, cmt) | Avaliar | |
| CarlaBase, CarlaRack, CarlaPatchbay | Integração com o Carla | Avaliar | |
| Vestige, VstEffect, VstBase | Host de VST2 de Windows via Wine | Avaliar (complexo de manter) | |

### Importação e exportação

| Plugin | O que é | Sugestão | Decisão |
|---|---|---|---|
| MidiImport, MidiExport | Importar e exportar MIDI | Manter | |
| HydrogenImport | Importar projetos do Hydrogen | Candidato a remover | |

## Outras funcionalidades a avaliar

| Item | Observação | Decisão |
|---|---|---|
| Backends de áudio | No Linux, o foco natural é JACK/PipeWire, ALSA e PulseAudio. Avaliar remover OSS, sndio, SoundIO e talvez SDL e PortAudio. | |
| Empacotamento para Windows (NSIS, MinGW, MSVC) | O destino é Linux. | |
| Empacotamento para macOS (DMG) | O macOS hoje é usado só para desenvolver. Manter pelo menos a compilação. | |
| Gravação herdada do LMMS (`SampleRecordHandle`, "Set/clear record") | Será substituída na etapa 3. | |
| Step recorder, microtuner, controladores LFO e MIDI | Avaliar o uso real. | |
| Presets, samples e projetos demo em `data/` | Remover junto com os plugins correspondentes. | |
| Traduções (`data/locale`) | Definir quais idiomas manter. | |

## Critério de conclusão

- Todas as decisões das tabelas estão preenchidas.
- Os itens marcados para remoção foram removidos.
- O projeto compila no Linux e no macOS, os testes passam e os projetos demo restantes abrem e tocam.
