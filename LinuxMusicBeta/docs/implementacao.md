# Andamento da implementação

O quadro do que está feito, do que está em andamento e de quem faz cada parte. Cada pacote só é marcado como pronto
quando passa no critério de "pronto quando" de `interfaces.md` (Parte 4). As decisões tomadas sem o usuário estão em
`decisoes-autonomas.md`.

| # | Pacote | Quem | Estado |
|---|---|---|---|
| 1 | `lm-base` | sessão de design | **feito** (19/11): `Main`, `Role`, `Error`/`Label`, `Cancel`/`Progress`/`Phase`/`OperationHandle`/`Workers`, anéis, `HostTime`/`Clock`; 16 testes; `loom` do par terminar/consultar no CI |
| 2 | `lm-state` | agente | **feito** (19/11): `Value` (imbl, O(1) clone e `same_node`, chaves ordenadas, igualdade bit a bit), `PathStep` com `Note(n)`, `NodeId`, `Resource` (chave BLAKE3 hex validada, C9), JSON próprio (leitura estrita e escrita estável com `ryu`, ida e volta bit a bit), `SchemaRegistry` (validação com caminho e campo, `upgrade` em cadeia, apelidos de parâmetro, `defaults`, `refs`/`ids`/`resources` para o `lm-doc`) e o teste dos padrões congelados (U29); 47 testes e alvo `cargo-fuzz` em `fuzz/`. **Medida das 10 mil notas: o `imbl` passa** — 4,4 µs por edição em média (release; pior 74 µs), histórico de mil raízes = 5,8 MB (~5,8 KB por edição, 0,05% de uma cópia do item de 11 MB), crescimento linear (1,44 MB a cada 250 edições), exatamente uma nota nova por edição |
| 3 | `lm-time` | agente | **feito** (19/11): `Bpm` racional exato (texto decimal, 1–1000, 6 casas), `Ppq`, `TimeSignature`, `TempoMap` com saltos e **rampas lineares no tempo** (exatas, inteiros de 256 bits próprios), `tick_at` = último tique iniciado (inverso exato de `sample_at`, meio-par), `segments_in`, `ticks_in`, compasso:tempo:tique com mudanças de fórmula, `relocate` (H9); sem dependências além do `lm-base`; 43 testes (mapas sorteados, referência independente, 24 h de blocos) |
| 4 | `lm-doc` | agente | **feito** (19/11): `Document` (raiz + índices persistentes id → (pai, contêiner) e referências reversas, `meta.issued`/`meta.uuid` fora do desfazer), os **esquemas do projeto** (versão 1, padrões congelados em `tests/data/defaults.json`; faixas sem pan com saídas `o:` para canais `m:`, mesa com rack fixo tube/eq/dynamics/transformer/tolerance, inserts, envios e sidechain, master `m:0`), `Edit` (`set` em qualquer profundidade, `insert`, `remove` com `OnRemove` nos quatro casos, `move_to` com índice final, notas por `n`, `paste` com ids novos), regras de domínio no commit (reconciliação de portas M1, ciclos, pastas, `Limits`, mapa de andamento, automação), `Diff` mesclado pelo estado final, histórico de estados com o `Diff` e o inverso de cada passo (U18), gestos G1–G3, `Feed`, diário (`JournalPatch`, replay sem regras), `Patch` do `heavy_edit`, abrir com upgrade/só leitura (H11); 48 testes. **Medida (release): edição de uma nota num projeto de 64 faixas × 10 mil notas = 15 µs (mediana; pior 99 µs); arrasto 4,7 µs por quadro; desfazer colar 10 mil notas = 0,56 ms** |
| 5 | `lm-render` | agente | a fazer |
| — | `lm-dsp` (códigos de efeito do legado, com A/B) | sessão de implementação | em andamento (19/11): **o rack da mesa analógica** (filtros casados e EQ da mesa, tolerâncias congeladas, transformador, sobreamostragem HIIR, válvulas, estágio de ganho, 9 compressores e gate, mono bass, faixa de canal, seção do master), **o oscilador** do TripleOscillator (8 formas, tabelas de banda limitada, ondas do usuário, PM/AM/mix/sync/FM), e os **efeitos de validação**: Delay, ReverbSC, Compressor e Eq. 33 testes de unidade e **A/B de 97 casos contra o legado** (`ab/`; os efeitos rodam os plugins originais), todos abaixo de -90 dBFS ou bit a bit, com uma diferença intencional escrita (libm, V2). A seguir: o resto do DSP dos plugins de validação (AudioFileProcessor, AvlDrums) e o dos outros instrumentos e efeitos
| — | `lm-sys-fpu`, `lm-sys-sndfile`, `lm-sys-samplerate`, `lm-sys-fluidsynth` | agente | **feito** (19/11): `lm-sys-fpu` feito (FTZ/DAZ no `MXCSR`, `FZ` no `FPCR`, guarda que restaura; 4 testes); `lm-sys-samplerate` feito (conversão única exata e cancelável, reamostrador contínuo sem alocação, latência medida; 12 testes, com armadilha do heap de C); `lm-sys-sndfile` feito (leitura WAV/RF64/AIFF/FLAC/OGG/MP3, escrita WAV/RF64/AIFF/FLAC/OGG/MP3, `sync` a cada ~2 s deixa o RF64 legível depois de uma queda, `reserve` com `fallocate`/`F_PREALLOCATE`; 14 testes); `lm-sys-fluidsynth` feito (construtor que carrega o SoundFont só numa instância nova, `Synth` do áudio `Send` e não `Sync`, threadsafe-api desligado, um núcleo, sem driver, várias saídas estéreo; nota, CC, programa e render sem alocar, conferido pela armadilha do heap de C; SF2 gerado no teste; 9 testes); o CI Linux precisa de `libsamplerate0-dev`, `libsndfile1-dev`, `libfluidsynth-dev` |
| — | `lm-sys-alsa`, `lm-sys-linux` | agente | em andamento (19/11): `lm-sys-alsa` (PCM `hw`/`plughw`/`null` com período negociado, f32/s32/s24/s24_3/s16 com conversão, mmap ou leitura/escrita, `snd_pcm_link`, carimbos `CLOCK_MONOTONIC`, xrun e quadros perdidos da captura (C2), `poll` com despertador; seq com fila em tempo real convertida para o `HostTime`, portas, conexões, envio, leitor parado por despertador; enumeração de placas, dispositivos, capacidades e *hints*; chaves `alsa.device_busy`/`alsa.permission_denied`); `lm-sys-linux` (SCHED_FIFO direto ou pelo rtkit, guarda que restaura; `mlockall` só com `RLIMIT_MEMLOCK` ilimitado, senão `LockedBuffer` e pilha travada; inibidor do logind; ReserveDevice1 com teste num `dbus-daemon` próprio); o CI Linux precisa de `libasound2-dev` e `dbus` |
| 6 | `lm-plugin` + plugin de teste | — | a fazer |
| 7 | `lm-compile` | — | a fazer |
| 8 | `lm-io` (Dummy, ALSA, ALSA seq, DiskWriter, ResourceStore) | — | a fazer |
| 9 | `lm-session` + `lm-headless` | — | a fazer |
| 10 | a primeira fatia | — | a fazer |
| 10b | revisão final de coerência | — | a fazer |
| 11 | protótipo de interface em GPUI (`prototypes/`) e a `gui` | agentes A (casca, composição, mixagem, roteamento, medidas), B, C, D | **em andamento** (19/11): GPUI escolhida pelo usuário; esqueleto com todas as telas registradas, composição e mixagem funcionando, 120 quadros/s; relatório em `docs/prototipo-interface.md` |
| — | CI no GitHub (Linux x86-64 principal; Linux aarch64 e macOS) | agente de CI | em andamento (19/11): `.github/workflows/beta.yml`, `deny.toml`, `ci/` |

## Como os agentes trabalham

- Cada agente é dono de uma pasta `crates/<nome>` e só escreve nela (e nas linhas da sua linha nesta tabela).
- **Nunca deixe uma pasta em `crates/` sem `Cargo.toml` válido**, nem por um instante: o workspace inclui `crates/*`, e
  uma pasta pela metade quebra o `cargo` de todos. Crie o pacote de uma vez (o `Cargo.toml` e o `src/lib.rs` mínimo
  que compila) antes de qualquer outro arquivo, ou monte-o fora de `crates/` e mova-o pronto.
- O workspace (`Cargo.toml`) inclui toda pasta em `crates/*`; uma dependência nova do workspace é pedida à sessão de
  design.
- Todos leem, antes de começar: `arquitetura-nova.md`, `interfaces.md` (a seção do seu pacote, a Parte 2, a Parte 3 e
  as partes 5 a 7), `decisoes-autonomas.md` e o código dos pacotes de que dependem.
- Um `target` só (`LinuxMusicBeta/target`); localmente, só debug e release.
- Commit só dos próprios arquivos, por caminho, e push para o `origin/master` a cada etapa.
