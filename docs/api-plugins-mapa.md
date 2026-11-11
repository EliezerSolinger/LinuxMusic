# Mapa do branch de trabalho da nova API de plugins

Levantamento de 11/11/2026 do branch `messmerd/pin-connector-with-audio-buffer` (último commit "Refactor and use
AudioBuffer (very WIP)", de 17/06/2026). Serve de guia para terminar a fase 1 de [multicanal.md](multicanal.md).

## Desenho novo (no estilo CLAP)

- **`AudioProcessor`** (`include/AudioProcessor.h`): interface do plugin.
  - Estados: `Loaded`, `Active`, `Processing`, `Sleeping`, `Error`.
  - Ciclo de vida: `activate(sampleRate, maxFrames)`, `deactivate`, `startProcessing`, `stopProcessing`, `reset`.
  - Processamento: `process(ProcessContext&) -> ProcessStatus`.
  - Descrição dos canais: `channelCount`, `channelGroupCount`, `getChannelGroup`, `getInplacePair` (pares 1:1 de
    entrada↔saída que dividem o mesmo buffer).
- **`ProcessContext`**: buffers de entrada e saída (`in()`, `out()`, ou `inOut()` quando tudo é in-place) e a nota
  (`nph()`).
- **`Host`**: o que o plugin pode pedir ao host (`rescanAudioPorts`, `useRemotePluginBuffers`).
- **`PluginInstance`**: o lado do host para cada plugin.
  - Guarda o plugin, os buffers de entrada e saída (locais ou em memória compartilhada para plugins remotos), os
    pinos (`AudioPortsModel`) e o estado.
  - `process(trackChannels, nph)` deve: rotear os canais da faixa para as entradas, chamar `process`, rotear as
    saídas de volta e cuidar de sono, bypass, sanitização e auto-quit.
- **`AudioPortsModel`**: matriz de pinos de entrada (faixa→plugin) e de saída (plugin→faixa). Também detecta o
  roteamento direto (o atalho sem cópia).

## Situação dos plugins no branch

- **Desenho novo (`process(ProcessContext&)`): 1** (Amplifier, incompleto).
- **Sistema antigo de templates do #7459: 25 efeitos.**
  - BassBooster, Bitcrush, Compressor, CrossoverEQ, Delay, Dispersion, DualFilter, DynamicsProcessor, Eq, Flanger,
    GranularPitchShifter, LOMM, LadspaEffect, Lv2Effect, MultitapEcho, PeakControllerEffect, ReverbSC,
    StereoEnhancer, StereoMatrix, WaveShaper.
  - SpectrumAnalyzer, Vectorscope, ZynAddSubFx.
  - VstEffect, Vestige (e o VstBase).
- **API intermediária (`playImpl`/`playNoteImpl`): 22 instrumentos.** AudioFileProcessor, BitInvader, CarlaBase,
  FreeBoy, GigPlayer, Kicker, Lb302, Lv2Instrument, Monstro, Nes, OpulenZ, Organic, Patman, Sf2Player, Sfxr, Sid,
  SlicerT, Mallets, TripleOscillator, Vibed, Watsyn, Xpressive.
- **API antiga original: 2.** FrequencyShifter, SlewDistortion.

## Ordem de trabalho

1. Núcleo compilando: `PluginInstance` com os construtores corretos de `AudioBuffer`/`AudioBufferData` (estado
   vazio válido), sem o `ProcessStatus` duplicado.
2. `AudioPortsModel` concreto: o aviso de mudança de canais vira callback tratado pelo `PluginInstance`.
3. Posse e ciclo de vida no `PluginInstance`: `unique_ptr<Plugin>`, `unload`, conversão para efeito "dummy",
   `rescanAudioPorts` e a máquina de estados (`activate` após carregar e ao mudar a taxa de amostragem ou o tamanho
   do período). O `InstrumentTrack` passa a guardar o `PluginInstance` por `unique_ptr`.
4. Mapeamento in-place pelo `mapAccessBuffer`.
5. Roteador planar sem template: `send(faixa → entradas)`, `receive(saídas → faixa)` e roteamento direto mapeando
   os canais da faixa.
6. `PluginInstance::process` completo: acordar e dormir, bypass, roteamento, sanitização, silêncio, auto-quit e
   detecção de áudio corrompido.
7. `Effect` com ciclo de vida padrão; `LegacyEffect` (estéreo intercalado, in-place) para portar os efeitos;
   `DummyEffect`.
8. `EffectChain`, `EffectRackView` e `EffectView` na posse nova, e o pin connector pelo `PluginInstance`.
9. Todos os efeitos em `process(ProcessContext&)`; LADSPA e LV2 com grupos de canais reais.
10. `Instrument` herdando `AudioProcessor`, `LegacyInstrument` (0 entradas, 2 saídas), play handles passando pelo
    `PluginInstance`, e todos os instrumentos portados.
11. Plugins remotos (VST, ZynAddSubFx) na memória compartilhada do `PluginInstance`.
12. Testes: `AudioPortsTest` reescrito para o modelo de pinos, o roteador novo e o `PluginInstance` (processadores
    falsos com pares in-place, roteamento direto e vários grupos), e `AudioBufferTest` atualizado.
13. Remover o sistema antigo: `AudioPlugin.h`, `AudioPorts.h`, `PluginAudioPorts.h`, `RemotePluginAudioPorts.h`,
    `AudioPortsSettings.h` e `LmmsPolyfill.h`, além dos métodos antigos de `Instrument`.
