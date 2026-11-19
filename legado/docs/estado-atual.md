# Estado atual

## Base do código

- Branch de trabalho: `master` (desde 12/11/2026; o `multicanal` foi juntado nele).
- Ponto de partida: o branch `feature/recording-stage-one` do LMMS ([LMMS#7786](https://github.com/LMMS/lmms/pull/7786)),
  sincronizado com o `master` do LMMS de setembro de 2026 (versão 1.3.0-alpha.2).
- PRs do LMMS incorporados:
  - [LMMS#7903](https://github.com/LMMS/lmms/pull/7903): ring buffer lock-free para a entrada de áudio.
  - [LMMS#7627](https://github.com/LMMS/lmms/pull/7627): exportar o áudio de um clipe de sample (menu "Export sample buffer").

## Renomeação para LinuxMusic

Já feita no nível do produto: executável `linuxmusic`, textos da interface e traduções, `~/.linuxmusicrc.xml`,
`~/Documents/LinuxMusic`, `/usr/lib/linuxmusic` e `/usr/share/linuxmusic`, `.desktop`, ícones, tipo MIME
`application/x-linuxmusic-project`, man page, bash completion, AppImage e nome do cliente no JACK, PulseAudio e ALSA.

Mantido de propósito: o namespace C++ `lmms` e as macros `LMMS_*` (internos e invisíveis para o usuário) e as
extensões `.mmp`/`.mmpz`. O logo, os ícones e o splash são próprios (`tools/artwork`, que gera todos a partir dos
SVGs); os créditos do LMMS e a GPL continuam no Sobre e no splash.

## Trazendo atualizações do LMMS

A base é o `master` do LMMS (linha 1.3.0-alpha), não a 1.2 estável. Para trazer novidades:

```sh
git fetch upstream
git merge upstream/master
python3 tools/rebrand.py . $(git diff --name-only HEAD~1 HEAD -- src include plugins data/locale)
```

O `tools/rebrand.py` troca "LMMS" por "LinuxMusic" nos textos visíveis (strings do código, arquivos `.ui` e traduções)
que chegaram no merge.

## Gravação

A gravação herdada do branch `feature/recording-stage-one` (um clipe marcado, áudio na memória embutido no projeto)
foi substituída por um sistema próprio: faixa de áudio com entrada, armar e monitoração por faixa, gravação direta
em disco, takes em lanes, comp, calibração de latência e gravação MIDI. Ver `gravacao-como-usar.md` e
`plano-gravacao.md`.

## Arquitetura do núcleo (depois de `plano-refatoracao.md`, 17–18/11/2026)

- **Thread de áudio:** não aloca, não faz E/S e não espera a interface. As mudanças no que o render percorre
  (faixas, clipes, canais do mixer, controladores, play handles) são preparadas fora e publicadas sob
  `AudioEngine::changesGuard()`, por instantes (o `EditWhilePlayingTest` mede a maior trava de cada edição).
- **Posse:** cada objeto tem dono explícito (`unique_ptr`, `Track::create`/`Track::destroy`); objetos entram nas
  listas do áudio por último no construtor e saem primeiro no destrutor.
- **Parâmetros:** efeitos e instrumentos de fluxo único aplicam as mudanças no começo do `process()`; os de notas
  em várias threads, em `Instrument::preparePeriod()`.
- **Determinismo:** o mesmo projeto renderiza bit a bit igual (`tools/check/determinism.sh`), qualquer que seja a
  ordem das threads.
- **Plugins:** os internos rodam no processo, no caminho do render; LADSPA e LV2 podem rodar isolados, por efeito
  (opcional; automático para um plugin suspeito). Um plugin travado ou que cai não derruba o programa.
- **Exportação:** loudness e pico real medidos com a libebur128 (opcional, `WANT_EBUR128`).
- **MIDI:** os drivers de bytes usam o `MidiByteParser`; o driver do macOS foi reescrito (hotplug, portas
  virtuais).

## Como compilar

```sh
cmake -S . -B build -G Ninja -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build build
./build/linuxmusic
```

No macOS, `brew bundle install` instala as dependências (Qt 6). No Ubuntu, veja
`.github/workflows/deps-ubuntu-24.04-gcc.txt`.

## Como verificar

- Testes: `QT_QPA_PLATFORM=offscreen ctest --test-dir build/tests -j4`.
- Comparação de som (A/B) contra uma referência, renderizando os projetos de `tools/check/ab-projects`:
  `tools/check/ab.sh build <referência> <saída>`.
- Interface: `tools/check/gui-smoke.sh build <saída>`.
- Determinismo: `tools/check/determinism.sh build <saída>`.
- Sanitizadores: builds separados com ASan/UBSan e com TSan (`tools/check/sanitize-all.sh`; ver
  `tools/check/README.md`).
