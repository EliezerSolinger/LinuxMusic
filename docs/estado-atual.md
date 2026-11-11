# Estado atual (ponto de partida)

## Base do código

- Branch de trabalho: `linuxmusic`.
- Ponto de partida: o branch `feature/recording-stage-one` do LMMS ([LMMS#7786](https://github.com/LMMS/lmms/pull/7786)),
  sincronizado com o `master` do LMMS de setembro de 2026 (versão 1.3.0-alpha.2).
- PRs do LMMS incorporados:
  - [LMMS#7903](https://github.com/LMMS/lmms/pull/7903): ring buffer lock-free para a entrada de áudio.
  - [LMMS#7627](https://github.com/LMMS/lmms/pull/7627): exportar o áudio de um clipe de sample (menu "Export sample buffer").

## Renomeação para LinuxMusic

Já feita no nível do produto: executável `linuxmusic`, textos da interface e traduções, `~/.linuxmusicrc.xml`,
`~/Documents/LinuxMusic`, `/usr/lib/linuxmusic` e `/usr/share/linuxmusic`, `.desktop`, ícones, tipo MIME
`application/x-linuxmusic-project`, man page, bash completion, AppImage e nome do cliente no JACK, PulseAudio e ALSA.

Mantido de propósito: o namespace C++ `lmms` e as macros `LMMS_*` (internos e invisíveis para o usuário),
as extensões `.mmp`/`.mmpz` e o logo/splash do LMMS (ainda falta arte própria).

## Trazendo atualizações do LMMS

A base é o `master` do LMMS (linha 1.3.0-alpha), não a 1.2 estável. Para trazer novidades:

```sh
git fetch upstream
git merge upstream/master
python3 tools/rebrand.py . $(git diff --name-only HEAD~1 HEAD -- src include plugins data/locale)
```

O `tools/rebrand.py` troca "LMMS" por "LinuxMusic" nos textos visíveis (strings do código, arquivos `.ui` e traduções)
que chegaram no merge.

## Sobre a gravação herdada do LMMS

A gravação que veio do branch `feature/recording-stage-one` **não é a base da gravação do LinuxMusic**. Ela é
limitada demais:
- Grava um clipe marcado com "Set/clear record".
- Guarda o áudio na memória e depois o embute no projeto.
- Não tem seleção de entrada por faixa, monitoração nem gravação direta em disco.

Esse código será revisto na etapa 1 (remover ou isolar) e substituído na etapa 3 por um sistema próprio.

## Como compilar

```sh
cmake -S . -B build -DCMAKE_BUILD_TYPE=RelWithDebInfo
cmake --build build
./build/linuxmusic
```

No macOS, use `brew bundle install` para instalar as dependências e acrescente
`-DCMAKE_PREFIX_PATH="$(brew --prefix qt@5)"` ao comando do `cmake`. No Ubuntu, veja
`.github/workflows/deps-ubuntu-24.04-gcc.txt`.
