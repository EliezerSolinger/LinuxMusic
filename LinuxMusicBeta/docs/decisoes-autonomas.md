# Decisões tomadas sem o usuário

Em 19/11 o usuário autorizou a implementação e saiu, pedindo que todas as decisões fossem tomadas de forma autônoma
até a 1.0. Cada decisão abaixo tem o motivo, para o usuário revisar quando voltar. Mudar qualquer uma delas é
possível; a lista existe para isso ser fácil.

Formato: **decisão** — motivo. Data entre parênteses.

## Escopo e ordem

- **A 1.0 tem todos os recursos do LinuxMusic**, reescritos no contrato novo (19/11). É o pedido do usuário. A ordem é
  a de `interfaces.md` (Parte 4): primeiro a base, depois a primeira fatia (gravar, tocar, mixar, salvar), depois os
  plugins e os editores, um a um, cada um com os testes de contrato.
- **Designs:** os aprovados (2 sem pan, 3, 5, 8) são especificação. Para o resto da interface, a referência é a
  interface atual do legado (decisão do usuário, temporária).

## Pendências que estavam com o usuário

- **Gravação MIDI entra antes da 1.0, logo depois da primeira fatia de áudio** (19/11) — é recurso do LinuxMusic (o
  Piano Roll grava) e foi decisão de 12/11 ("MIDI e áudio juntos na v1").
- **Itens de áudio: cada item tem `timebase` (`musical` ou `audio`); itens gravados nascem `musical`, e o time-stretch
  entra antes da 1.0** (19/11) — junta a decisão de 12/11 (gravados seguem o BPM) com a especificação nova (escolha
  por item). Biblioteca: Rubber Band (GPL, compatível), em `lm-sys-rubberband`, com o original sempre guardado.
- **Rampas de andamento entram antes da 1.0, depois da primeira fatia** (19/11) — decisão de 12/11; ficam depois das
  conversões exatas em segmentos constantes, com a regra de arredondamento escrita e testada antes.
- **Um arquivo de gravação por entrada, com os cortes de loop marcados** (19/11) — é o desenho revisado (U1, U5), e
  recuperar uma queda fica mais simples com um arquivo contínuo.
- **Crossfade padrão do comping: 5 ms, editável por fronteira** (19/11) — proposta aceita sem objeção.
- **Velocidade: 0 a 1, linear; a curva é do host, por faixa; o plugin declara no esquema o que faz com ela; o teste
  confere só que a resposta é crescente e coerente** (19/11) — evita impor uma lei de volume a instrumentos que usam a
  velocidade para timbre.
- **LV2 continua antes da 1.0, como plugin de terceiros (fase 2)**, e sai da lista de plugins de validação da
  primeira fatia (19/11) — o usuário disse em 18/11 que o LV2 não precisava entrar no rack, e terceiros são fase 2.
- **Trocar a taxa de amostragem de um projeto existe**, como uma `Operation` que reconverte `audio/` e recompila
  (19/11) — sem isso, um projeto feito a 44,1 kHz fica preso para sempre.
- **MIDI de uma faixa tocando o instrumento de outra: não entra na 1.0** (19/11) — os envios são de áudio. Itens MIDI
  numa faixa sem instrumento ficam mudos, e a faixa mostra "sem instrumento".
- **Exportar um trecho:** o padrão é "Rápido" com 10 s de aquecimento, e a janela mostra o custo estimado do "Exato";
  exportar a música inteira é sempre exato (19/11) — revisão V18.
- **Solo, medidor e monitor de faixa com várias saídas:** os padrões propostos ficam (especificação, item 2) (19/11).

## Técnicas

- **Versão do Rust:** `rust-version` no `Cargo.toml` do workspace com a versão estável instalada (1.99), sem
  `rust-toolchain.toml` (19/11) — o Rust desta máquina é do Homebrew; um arquivo de toolchain forçaria o rustup a
  baixar outra.
- **A árvore (`Value`) usa a crate `imbl`**, sujeita à medida das 10 mil notas (19/11). Se a medida falhar, troca por
  nós próprios, sem mudar a interface.
- **JSON:** `serde_json` para ler (com `preserve_order` desligado, já que as chaves são ordenadas) e uma escrita
  própria pequena com `ryu` para garantir o formato estável (19/11).
- **D-Bus no Linux (rtkit, logind, ReserveDevice1):** `zbus` no modo bloqueante, numa thread própria (19/11) — Rust
  puro, sem `unsafe` nosso; a thread não é de tempo real.
- **`libm` em Rust puro** no DSP (`lm-dsp`) e no render (19/11) — revisão V2, determinismo entre distribuições.
- **Linux sem máquina Linux:** o código só de Linux (ALSA, rtkit, logind) compila e é testado no CI do GitHub
  (Ubuntu), com o PCM `null` do ALSA para os caminhos de áudio e o `snd-aloop` quando o runner permitir (19/11). O
  teste numa placa de verdade fica marcado como pendente para o usuário.
- **Disco (34 GB livres):** um `target` só, em `LinuxMusicBeta/target`; localmente só debug e release; ASan, TSan,
  `miri` e `loom` no CI (19/11).
- **A biblioteca de interface sai do protótipo** (`interfaces.md`, I-15). O protótipo é feito em paralelo com o núcleo,
  não no fim (revisão V14).
