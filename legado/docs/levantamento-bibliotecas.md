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

## Verificação da etapa 1 (17/11/2026, prioridade alta)

Fontes: a API pública do GitHub (licença, último push, commits desde 17/11/2025, colaboradores, versões), os arquivos
de licença, os README e as CIs dos repositórios, e o que este build já tem instalado. "Commits" e "colaboradores"
medem atividade, não qualidade: valem junto com os testes de cada projeto. A etapa 2 (protótipo medido aqui contra a
nossa implementação) ainda não foi feita.

| | libremidi | RtMidi (alternativa) | libebur128 | Carla (ponte) | crashpad | sentry-native |
|---|---|---|---|---|---|---|
| Licença (arquivo lido) | BSD-2; derivada do RtMidi (MIT) e do ModernMidi (BSD-2) | MIT | MIT | GPL-2.0-ou-posterior (cabeçalho SPDX do código) | Apache-2.0 | MIT |
| Último commit | 28/10/2026 | 15/10/2026 | 15/11/2026 | 19/03/2026 | 06/11/2026 | 17/11/2026 |
| Commits em 12 meses | 169 | 130 | 8 | 5 | 58 | 708 |
| Colaboradores | 36 | 92 | 21 | 63 | 200 | 106 |
| Última versão | v5.4.3 (01/2026) | 6.1.0 (10/2026) | v1.2.6 (02/2021) | v2.5.10 (07/2025) | sem versões (segue o Chromium) | 0.17.6 (11/2026) |
| Testes | CI: Ubuntu gcc e clang, macOS 14, Windows MSVC e ARM, MinGW64 (MSYS2), Android, WASM | CI própria | Testes com os sinais oficiais da EBU (Tech 3341 e 3342), *fuzzer*, cobertura exigida na CI (adicionada nesta semana) | CI; poucos testes automáticos | Os do Chromium | CI própria |

O que cada verificação mostrou:

- **libremidi: confirmada.**
  - Ativa, com licença compatível e CI em todas as nossas plataformas, inclusive MinGW. É C++20, como nós.
  - Backends: ALSA (seq e raw), JACK, CoreMIDI, WinMM, Windows MIDI Services, PipeWire e rede. Também trata MIDI 2 e
    conexão a quente (README).
  - **Falta saber:** se enviar e receber mensagens aloca memória ou trava. Isso só a etapa 2 responde, com o nosso
    `AudioThreadAllocationTest`.
  - O RtMidi é mais conservador (MIDI 1, API mais antiga) e é a alternativa caso a libremidi falhe nesse ponto.
- **libebur128: confirmada.**
  - Pouca atividade porque está estável (a última versão é de 2021), mas os testes usam os sinais de referência da
    EBU, tem *fuzzer* e acabou de ganhar uma exigência de cobertura na CI.
  - A etapa 2 é rodar os mesmos sinais da EBU no nosso BS.1770 e nela.
- **Ponte do Carla: rebaixada para média.**
  - Funciona: o Carla do Homebrew instalado aqui (2.5.10) traz o `carla-bridge-native`.
  - Mas o projeto está quase parado: 5 commits no ano, o último em março, e a última versão é de julho de 2025.
  - Além disso, o LinuxMusic liga o Carla de forma fraca, ou seja, ele só existe se o usuário o instalar. O isolamento
    dependeria de uma instalação externa que não controlamos.
  - Antes de decidir, comparar com a fase 6 original (o nosso `RemotePlugin`, que já funciona para VST e Zyn).
- **crashpad / sentry-native: rebaixados para baixa.** Não servem às nossas plataformas hoje:
  - o sentry-native pede macOS 15+, e o LinuxMusic sai para macOS 10.15 (Intel) e 11.0 (ARM);
  - no Windows, o crashpad precisa de clang (LLVM-MinGW ou MSVC) para gerar os `.pdb`, e o nosso Windows x64 sai com
    MinGW GCC.
  - O `CrashHandler` próprio continua. Revisitar se os alvos de plataforma mudarem.

## Ordem sugerida (revista depois da verificação)

1. **libebur128.**
2. **libremidi**, se passar no teste de alocação na thread de áudio; senão, RtMidi.
3. Decidir o isolamento de plugins de terceiros (fase 6): o nosso `RemotePlugin` ou a ponte do Carla, pesando o
   ritmo atual do Carla e a dependência externa.
4. **miniaudio** para os drivers secundários (falta verificar como as quatro de cima).

## Ordem sugerida (antes da verificação)

1. **libebur128**: uma função, mais correção na exportação.
2. **libremidi**: tira cerca de 2.700 linhas (entrada, saída e escrita MIDI) e cobre o Linux que não testamos aqui.
3. **Ponte do Carla** para o isolamento de plugins de terceiros (substitui o que estava planejado na fase 6).
4. **crashpad**: diagnósticos melhores das quedas, inclusive as de plugins.
5. **miniaudio** para os drivers secundários.
