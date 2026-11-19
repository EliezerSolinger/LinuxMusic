# Andamento da implementação

O quadro do que está feito, do que está em andamento e de quem faz cada parte. Cada pacote só é marcado como pronto
quando passa no critério de "pronto quando" de `interfaces.md` (Parte 4). As decisões tomadas sem o usuário estão em
`decisoes-autonomas.md`.

| # | Pacote | Quem | Estado |
|---|---|---|---|
| 1 | `lm-base` | sessão de design | **feito** (19/11): `Main`, `Role`, `Error`/`Label`, `Cancel`/`Progress`/`Phase`/`OperationHandle`/`Workers`, anéis, `HostTime`/`Clock`; 16 testes; `loom` do par terminar/consultar no CI |
| 2 | `lm-state` | agente | a fazer |
| 3 | `lm-time` | agente | a fazer |
| 4 | `lm-doc` | agente | a fazer (depende do 2) |
| 5 | `lm-render` | agente | a fazer |
| — | `lm-dsp` (códigos de efeito do legado, com A/B) | sessão de implementação | em andamento |
| 6 | `lm-plugin` + plugin de teste | — | a fazer |
| 7 | `lm-compile` | — | a fazer |
| 8 | `lm-io` (Dummy, ALSA, ALSA seq, DiskWriter, ResourceStore) | — | a fazer |
| 9 | `lm-session` + `lm-headless` | — | a fazer |
| 10 | a primeira fatia | — | a fazer |
| 10b | revisão final de coerência | — | a fazer |
| 11 | protótipo de interface (GPUI, Slint, Qt) e a `gui` | — | a fazer, em paralelo com o núcleo |
| — | CI no GitHub (Linux x86-64 principal; Linux aarch64 e macOS) | — | a fazer |

## Como os agentes trabalham

- Cada agente é dono de uma pasta `crates/<nome>` e só escreve nela (e nas linhas da sua linha nesta tabela).
- O workspace (`Cargo.toml`) inclui toda pasta em `crates/*`; uma dependência nova do workspace é pedida à sessão de
  design.
- Todos leem, antes de começar: `arquitetura-nova.md`, `interfaces.md` (a seção do seu pacote, a Parte 2, a Parte 3 e
  as partes 5 a 7), `decisoes-autonomas.md` e o código dos pacotes de que dependem.
- Um `target` só (`LinuxMusicBeta/target`); localmente, só debug e release.
- Commit só dos próprios arquivos, por caminho, e push para o `origin/master` a cada etapa.
