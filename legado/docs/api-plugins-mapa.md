# Mapa do branch de trabalho da nova API de plugins

> **Estado:** itens 1 a 13 da ordem de trabalho concluídos no branch `multicanal` (fases 1 e 2 de
> [multicanal.md](multicanal.md), exceto a janela do pin connector integrada à nova interface).

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

## Situação dos plugins (branch `multicanal`, 11/11/2026)

Todos os plugins usam o desenho novo. Os headers do sistema antigo de templates foram removidos.

- **Efeitos nativos:** todos em `process(ProcessContext&)`.
  - Estéreo intercalado in-place pelo `LegacyEffect`: a maioria.
  - Só entrada (2 entradas, 0 saídas): SpectrumAnalyzer, Vectorscope e Oscilloscope.
- **LADSPA:** portas de áudio reais ligadas direto aos buffers planares do host, sem cópia.
  - Plugins mono continuam rodando duas vezes, com controles vinculáveis.
  - Agora aparecem plugins com contagens diferentes, como 1→2 ou com entrada de sidechain.
- **LV2:** todas as portas de áudio, incluindo sidechain e opcionais. As entradas ficam em dois grupos:
  principal e sidechain.
- **Instrumentos:** `Instrument` herda `AudioProcessor`.
  - Os de fluxo único (Zyn, Carla, LV2, Vestige, OpulenZ, Sf2Player, GigPlayer, Lb302) passam pelo
    `PluginInstance` e pelo pin connector a cada período.
  - Os baseados em `NotePlayHandle` desenham cada nota no buffer da nota, e o `PluginInstance` aplica os pinos de
    saída.
  - Todos os nativos usam o `LegacyInstrument`.
- **Plugins remotos (Zyn, VST):** usam a memória compartilhada do `PluginInstance`.
  - O cliente recebe uma tabela de `PointerOffset` por canal.
  - Quando o processo remoto informa novas contagens de canais, o host refaz as portas na thread principal.
- **Carla:** contagens de canais vindas do descritor nativo; processa direto nos buffers planares.
- **Taxa de amostragem:** o `PluginInstance` reativa o plugin quando ela muda. LADSPA, LV2 e Zyn recriam suas
  instâncias em `activate()`, com o motor de áudio pausado, sem mutex no processamento.

### Verificação

- Testes: `AudioPortsTest` com 26 casos, incluindo instrumentos legados, planares com 4 saídas e roteamento de
  saída por nota.
- Comparação de renders A/B entre a branch `linuxmusic` e a `multicanal`, com projetos gerados: um instrumento
  por projeto, um efeito por projeto e 7 plugins LADSPA. Ferramentas no scratchpad: `gen_projects.py` e
  `abtest.sh`. Resultado:
  - Todos os casos determinísticos ficam idênticos até 1 bit (ordem de soma entre threads).
  - Os casos não determinísticos também variam na própria referência: ruído, fases aleatórias, ou o dj_eq do
    LADSPA, que amplifica a variação de 1 bit.
- A comparação revelou um bug herdado do LMMS, já corrigido: a thread que renderiza o período também processa
  trabalhos de áudio, e na exportação e nos callbacks dos drivers ela não desligava os denormais.
  - Efeitos com cauda de decaimento davam resultados diferentes conforme a thread que pegava o trabalho.
  - No x86 isso ainda causava picos de CPU.
- O host VST (VstEffect, Vestige, VstPlugin) passa em compilação só de sintaxe no macOS. O `RemoteVstPlugin`
  (Wine) e o LV2 em execução precisam ser testados no Linux.

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
