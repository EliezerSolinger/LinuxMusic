# Levantamento: o que ficaria melhor com uma biblioteca de código aberto

Escrito em 17/11/2026, a partir do código atual. Cada linha diz:
- o que o programa faz hoje com código próprio;
- que biblioteca madura faz o mesmo;
- o que se ganha e quanto custa trocar.

O LinuxMusic é GPL-2.0-ou-posterior, então as licenças MIT, BSD, zlib, Apache-2.0, LGPL e GPL-2+ servem. Uma
biblioteca só GPL-3 obrigaria o programa todo a ser distribuído como GPL-3.

As licenças citadas são as que eu conheço; confirmar na hora de adotar.

## Prioridade alta (ganho grande, risco controlado)

| Área | Hoje | Biblioteca | Ganho | Custo |
|---|---|---|---|---|
| **Entrada e saída MIDI** | 6 drivers próprios, ~2.150 linhas: ALSA seq e raw, JACK, CoreMIDI, WinMM, sndio | **libremidi** (BSD) ou RtMidi (MIT) | Hotplug, MIDI 2.0/UMP, listagem de portas e uma API única nas 3 plataformas. Os drivers do Linux nunca foram testados aqui (pendência do ALSA/JACK). | Médio: a interface `MidiClient`/`MidiPort` continua; muda o que fica por baixo. |
| **Medição de loudness na exportação** | BS.1770 escrito à mão no `ExportProcessor`: LUFS integrado com portas absoluta e relativa | **libebur128** (MIT) | Implementação de referência testada contra a suíte da EBU: pico verdadeiro (*true peak*, com sobreamostragem), LRA e loudness de curto prazo, prontos. Normalizar para streaming exige pico verdadeiro correto. | Baixo: uma função. |
| **Hospedar plugins de terceiros em processo separado** (fase 6 do plano) | IPC próprio, ~2.700 linhas: memória compartilhada SysV, semáforos e protocolo feito à mão, só para VST e Zyn | **Ponte do Carla** (*plugin bridge*, GPL-2+, já está no repositório como submódulo) | Isolamento pronto e testado para LADSPA, DSSI, LV2, VST2, VST3 e CLAP, com interface própria. Evita escrever e manter um hospedeiro e um protocolo novos. | Médio: o Carla já é usado pelos plugins CarlaRack/Patchbay; falta usá-lo como hospedeiro opcional de um plugin isolado. |
| **Relatório de queda** | `CrashHandler` próprio: sinal + pilha simbólica escrita num arquivo | **crashpad** (Apache-2.0) ou sentry-native em modo local (MIT) | Minidumps completos (todas as threads, registradores) que abrem no lldb/gdb depois, inclusive do Windows. O "plugin rodando" do `PluginGuard` vai como anotação. Configurado sem envio, nada sai da máquina. | Médio: muda o build nas 3 plataformas. |

## Prioridade média

| Área | Hoje | Biblioteca | Ganho | Custo |
|---|---|---|---|---|
| **Drivers de áudio** | 7 drivers próprios, ~2.500 linhas: ALSA, JACK, PulseAudio, CoreAudio, SDL, sndio e PortAudio | **miniaudio** (domínio público ou MIT-0) ou RtAudio (MIT) | Um backend só, cobrindo ALSA, PulseAudio/PipeWire, JACK, CoreAudio, WASAPI e sndio, com duplex e listagem de dispositivos. Menos código para testar em máquinas que não temos. | Médio a alto: o CoreAudio próprio faz dispositivo agregado, latências e captura sob demanda, que precisariam ser conferidos. Sugestão: trocar ALSA, Pulse, sndio, SDL e PortAudio e manter JACK e CoreAudio nativos. |
| **Filas e alocador de tempo real** | ~1.600 linhas próprias: `SpscRing`, `TripleBuffer`, `LocklessList`, `LocklessAllocator`, `RingBuffer` e o TLSF do `RealtimeAllocator` | **moodycamel ReaderWriterQueue / ConcurrentQueue** (BSD) e **TLSF de referência** (mattconte/tlsf, BSD) | Estruturas usadas e revisadas por muitos projetos. As nossas têm testes, mas `LocklessList`/`LocklessAllocator` são legado e duplicam o que já existe. | Baixo a médio, trocando uma por vez com os testes atuais. |
| **Escrita de MIDI (exportação)** | `MidiFile.h/.cpp` próprio, ~570 linhas | **libremidi** (já escreve SMF) ou midifile (craigsapp, BSD) | Formato 0 e 1, *running status*, meta-eventos. A leitura já usa o portsmf. | Baixo, junto com a troca do MIDI. |
| **Log de erros** | `ErrorLog` próprio (feito hoje): seguro na thread de áudio, rotação simples | **Quill** (MIT): log assíncrono de baixa latência, com *backend* em outra thread | Formatação tipada, vários destinos, rotação por tamanho e data, níveis. | Baixo, mas o nosso já cumpre o essencial: só se o log crescer. |
| **Fuzzing do carregamento** | Gerador próprio de projetos corrompidos (`corrupt-projects.py`, `LoadCorruptedTest`) | **libFuzzer** (LLVM do Homebrew; o clang da Apple não traz) ou AFL++ | Mutação guiada por cobertura: encontra caminhos que mutações aleatórias não alcançam (MIDI, SF2, `DataFile`). | Baixo: alvos pequenos, só em builds de teste. |

## Prioridade baixa (o próprio está bom, ou o ganho é pequeno)

- **Plugins LADSPA e LV2 hospedados no processo** (~5.300 linhas sobre lilv): o Carla faria isso também, mas o
  código atual funciona. Vale migrar junto com a ponte do Carla (acima) para ter um caminho só.
- **Reamostragem**: já é a libsamplerate. Para exportar com mais qualidade e velocidade, o **soxr** (LGPL) ou o
  **r8brain** (MIT) são melhores, mas não é urgente.
- **FFT**: já é o FFTW. Os planos criados com `FFTW_MEASURE` medem tempos na hora, e é por isso que o ladspa-mbeq
  varia a -138 dB entre execuções. Para resultados idênticos, usar `FFTW_ESTIMATE` ou *wisdom* salvo. O **pffft**
  (BSD) seria uma alternativa leve.
- **Filtros (`BasicFilters`, ~930 linhas)**, osciladores e mixagem SIMD (`MixHelpers`): DSP clássico, simples de
  manter. Bibliotecas como xsimd ou Highway trariam pouco.
- **Desfazer/refazer** (`ProjectJournal`) e **configuração** (`ConfigManager`, XML): o `QUndoStack` do Qt não
  resolveria o que o nosso resolve (diário de modelos por ID).
- **Leitura de áudio**: já é a libsndfile (WAV, FLAC, Ogg, MP3) e a libvorbis. Está bom.
- **Esticar o tempo**: já é o Rubber Band.

## Ordem sugerida

1. **libebur128**: uma função, mais correção na exportação.
2. **libremidi**: tira cerca de 2.700 linhas (entrada, saída e escrita MIDI) e cobre o Linux que não testamos aqui.
3. **Ponte do Carla** para o isolamento de plugins de terceiros (substitui o que estava planejado na fase 6).
4. **crashpad**: diagnósticos melhores das quedas, inclusive as de plugins.
5. **miniaudio** para os drivers secundários.
