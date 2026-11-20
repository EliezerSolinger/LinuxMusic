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
- **JSON: leitor e escritor próprios** no `lm-state` (19/11, revisto pelo agente do `lm-state`): o `serde_json` aceita
  em silêncio um inteiro maior que `u64` (vira real), não garante ida e volta exata dos reais sem uma opção que
  mudaria o workspace inteiro, e deixa uma chave repetida vencer sem aviso. O escritor usa `ryu`.
- **Inteiros no JSON: até ±(2^53 − 1)**, o maior valor exato em JSON; `to_json` devolve `Result` (19/11).
- **Um recurso no JSON é um objeto com exatamente `key` e `kind`, ambos válidos**; um objeto comum com essa forma
  exata é recusado na escrita (`state.json.ambiguous_resource`), para a ida e volta ser exata (19/11).
- **Esquemas ganharam** `FieldKind::Id`, `NoteMap` (C1) e `Any` (estado opaco, até 64 KB), e `required`/`nullable`;
  campos desconhecidos são recusados (19/11).
- **A árvore fica no `imbl`**: a medida das 10 mil notas deu 4,35 µs por edição (critério: 1 ms) e ~5,8 KB de
  histórico por edição, linear (19/11).
- **Sem dependências de teste fora da lista do workspace** (a política do CI): o `lm-state` usa um gerador
  semeado próprio em vez do `proptest` (19/11).
- **D-Bus no Linux (rtkit, logind, ReserveDevice1):** `zbus` no modo bloqueante, numa thread própria (19/11) — Rust
  puro, sem `unsafe` nosso; a thread não é de tempo real.
- **`libm` em Rust puro** no DSP (`lm-dsp`) e no render (19/11) — revisão V2, determinismo entre distribuições.
- **Linux sem máquina Linux:** o código só de Linux (ALSA, rtkit, logind) compila e é testado no CI do GitHub
  (Ubuntu), com o PCM `null` do ALSA para os caminhos de áudio e o `snd-aloop` quando o runner permitir (19/11). O
  teste numa placa de verdade fica marcado como pendente para o usuário.
- **Disco (34 GB livres):** um `target` só, em `LinuxMusicBeta/target`; localmente só debug e release; ASan, TSan,
  `miri` e `loom` no CI (19/11).
- ~~A biblioteca de interface sai do protótipo~~ — **decisão do usuário (19/11): a interface é GPUI, e só GPUI.** O
  protótipo deixou de comparar bibliotecas e virou a validação da GPUI com o núcleo (`prototipo-interface.md`).

## Do `lm-time` (19/11)

- **O BPM é guardado em milionésimos inteiros** (de 1 a 1000 BPM, até 6 casas decimais); o texto "119.88" vira a
  fração exata 2997/25. Esses limites são o que mantém a aritmética exata.
- **`tick_at(s)` é o último tique já começado** (`max{t : sample_at(t) <= s}`), e não o `floor` simples escrito no
  I-18: com o `floor`, um evento cairia no bloco errado sempre que `sample_at` arredondasse para baixo. Assim, tique →
  amostra → tique é exato sempre que os tiques não são mais finos que as amostras (na prática, sempre a 960 PPQ).
- **As rampas de andamento já existem e são exatas:** o andamento varia linearmente no tempo (BPM por segundo), a
  única lei cuja integral continua racional; as conversões são inteiras, em 256 bits. A rampa entre dois andamentos
  iguais vira segmento constante; o último ponto não pode ser rampa.
- **Inteiro de 256 bits próprio** (`lm-time/src/wide.rs`), em vez de uma crate, pela política de dependências do CI.

## Das ligações de mídia (19/11)

- **Sem crates `-sys` de terceiros:** as declarações `extern "C"` do libsndfile, do libsamplerate e do FluidSynth são
  próprias, com um `build.rs` que chama o `pkg-config` e exige versões mínimas (libsndfile ≥ 1.1.0 por causa do MP3,
  FluidSynth ≥ 2.2.0, libsamplerate ≥ 0.1.9). As crates `-sys` existentes eram velhas ou sem manutenção. O CI usa o
  Ubuntu 24.04.
- **FluidSynth só aceita de 8 a 96 kHz.** Um projeto acima de 96 kHz com o AvlDrums precisa que o plugin renderize a
  96 kHz e converta para a taxa do projeto com o `lm-sys-samplerate` (decisão para o autor do plugin, no porte).
- **O kit do FluidSynth é seco:** reverb e chorus internos desligados; o reverb é da mesa.

## Decisões do usuário sobre a interface (19/11)

- **O protótipo em GPUI cobre o sistema inteiro**, todas as telas do aplicativo completo, e vira a base do `lm-gui`.
- **Na mesa de mixagem, cada canal tem um fader deslizante junto da barra do medidor**, como numa mesa de verdade,
  com gesto, valor em dB e duplo clique para 0 dB. O cabeçalho da faixa continua com o knob de volume (design 02,
  sem pan).
- **A tela de mixagem é separada da tela de composição** (decisão do usuário): duas páginas, com abas no topo e um
  atalho. A Composição tem a linha do tempo e a gravação (decisão de 12/11: a gravação mora na Composição); a
  Mixagem tem a mesa inteira. A barra de transporte fica visível nas duas.
- **O programa novo não é um clone do legado** (decisão do usuário): todas as funcionalidades do legado existem na
  1.0, mas as telas são repensadas. Para as áreas sem design aprovado, o legado é a referência do que o programa faz,
  e não de como a tela é. O inventário das funcionalidades está em `funcionalidades-legado.md`.


## Respostas às perguntas do levantamento do legado (P-01 a P-40, `funcionalidades-legado.md`) (19/11)

Critério: a 1.0 tem todas as funcionalidades do legado, cada uma na forma certa para a arquitetura nova; o que muda
de forma diz pelo que é substituído.

- **P-01 VST2:** entra na 1.0, **fora do processo** (o processo auxiliar do I-9.6), com declarações próprias da
  interface VST2 (como o VeSTige do legado, compatível com a GPL). VST2 nativo do Linux primeiro; plugins de Windows
  pelo wine, depois, ainda antes da 1.0. CLAP e VST3 também, como na especificação.
- **P-02 Carla:** obsoleta, substituída pelo nosso processo auxiliar, que hospeda LV2, CLAP, VST2, VST3 e LADSPA.
- **P-03 LADSPA:** **confirmada pelo usuário em 20/11** (escolheu entre portar, linkar e fora do processo): entra na 1.0, fora do processo, como terceiros, sem port para Rust e sem linkar o C. Os conjuntos que o legado distribui (calf, caps,
  cmt, swh, tap) continuam distribuídos e carregados pelo auxiliar.
- **P-04 e P-33 (ligar dois botões, uma curva para vários controles):** viram um **controle compartilhado**: um nó de
  automação ou de controlador com vários alvos. Arrastar com Ctrl de um botão a outro cria esse vínculo.
- **P-05 ZynAddSubFX:** fora do processo (tem threads próprias e janela FLTK), com a janela dele aberta como janela
  externa, presa à faixa.
- **P-06 (alternar linear/log por botão):** fica, como opção de exibição, guardada no documento da interface.
- **P-07:** o "dormir no silêncio" dos efeitos vira contrato do processador: ele declara a cauda, e o host não o
  chama depois de a entrada ficar em silêncio por mais que a cauda. O ajuste do pool de memória de tempo real é
  obsoleto (o áudio não aloca).
- **P-08 Temas e idiomas:** os temas são dado (escuro padrão e claro). A 1.0 sai em português e inglês; as traduções
  do legado para os outros idiomas são convertidas onde o texto equivale, e o resto fica para colaboradores.
- **P-09 Presets, modelos e demos:** um **conversor à parte** (fora do programa e do núcleo) transforma os presets do
  legado dos plugins reescritos para o formato novo; os presets do Zyn (`.xiz`) são lidos pelo próprio Zyn. Modelos e
  demos são refeitos no formato novo.
- **P-10 Gravação:** "monitoramento de baixa latência" desliga, na faixa armada, os plugins com latência; "gravar com
  os efeitos" é uma opção por faixa (o padrão continua sendo seco, como decidido em 12/11); gravar a saída de uma
  faixa (reamostrar) entra; entradas com mais de 2 canais entram (as faixas têm N canais).
- **P-11 Modo de gravador simples** (sem camadas: a passada nova substitui): entra como opção.
- **P-12 Formato de gravação:** grava sempre em RF64 `f32` (sobrevive a quedas); ao fechar o take, uma opção converte
  para FLAC ou 24 bits.
- **P-13 Arquivos sem uso:** a ação explícita do H4 manda os arquivos para a **lixeira do sistema**, como o legado.
- **P-14:** o contador de xruns fica sempre visível no transporte.
- **P-15 Gravação MIDI:** grava CC, pitch bend e aftertouch; uma camada por volta do loop, como no áudio; um ajuste de
  latência de entrada por porta MIDI.
- **P-16:** o ALSA raw MIDI entra como segundo backend de MIDI do ALSA, depois do seq.
- **P-17 Modo analógico/digital da mesa:** fica (no digital, o rack fica limpo, sem modelagem).
- **P-18 Grupos:** um grupo é um canal para onde outros mandam (roteamento livre), e "vincular faders selecionados"
  dá o grupo relativo.
- **P-19:** um projeto novo nasce com dois canais de retorno, e cada canal mostra os botões de envio para os retornos
  que existem.
- **P-20 Pontos de insert:** cada insert tem um ponto em relação às seções do rack (os 6 pontos do legado), e a largura
  do canal fica.
- **P-21:** a tolerância é semeada pelo **id do canal**, então reordenar não muda o som.
- **P-22 Medidores:** pico, RMS, VU, K-meters e LUFS no master, escolhíveis.
- **P-23 Batidas com amostras:** um instrumento de pads de amostras (feito do DSP do AudioFileProcessor), tocado pelo
  editor de passos.
- **P-24:** curvas de pitch bend por nota entram (expressão por nota, como no CLAP).
- **P-25 `.xpt`:** obsoleto; um clipe é exportado como um pedaço de JSON, o mesmo formato de copiar e colar.
- **P-26:** o piano roll tem loop e cursor próprios do item.
- **P-27:** um ponto de automação pode ter valor de entrada e de saída diferentes (um degrau).
- **P-28:** o botão FX do cabeçalho da faixa abre os inserts do canal da mesa para onde vai a saída principal.
- **P-29, P-30, P-31:** criar um item de áudio vazio com um clique, curvas de fade ajustáveis e inverter um item de
  áudio (renderizado como recurso novo) entram.
- **P-32:** itens acima de 30 s tocam do disco (como o `DiskReader` do legado); os curtos ficam na memória.
- **P-34 Ajuda:** um manual próprio, em português e inglês, distribuído com o programa.
- **P-35:** o `lm-headless` cobre a linha de comando; variáveis de ambiente e o modo portátil (configuração ao lado do
  executável) entram.
- **P-36:** obsoleto; as ligações MIDI ficam nas preferências da máquina, não no projeto.
- **P-37:** a transposição global do projeto ganha um controle no transporte; o volume global é o do master.
- **P-38:** a saída MIDI de uma faixa para aparelhos externos entra (o MIDI entre faixas continua fora da 1.0).
- **P-39:** importar MIDI cria um canal da mesa por faixa importada; CC7 vira automação de volume da faixa, e CC10 vira
  automação de pan desse canal.
- **P-40:** a exportação de stems tem as duas opções: por faixa ou por canal da mesa.

## Coordenação e disco (19/11)

- **Cada agente num git worktree próprio**, com checkout só de `LinuxMusicBeta`, e push direto para o `master`
  (`implementacao.md`). Motivo: com muitos agentes na mesma árvore, um `pull --autostash` misturou trabalho pela metade
  de vários e um commit "por caminho" levava pedaços dos outros.
- **Perfil de desenvolvimento enxuto:** `debug = "line-tables-only"` e `incremental = false` no workspace. O `target`
  chegou a 11 GB com o disco em 12 GB livres; as linhas bastam para os backtraces.


## Do `lm-doc` (19/11)

- **As versões dos esquemas** ficam num objeto `"versions"` na raiz do arquivo salvo (`{"track": 1, ...}`), que não
  existe na árvore viva; cada plugin guarda a própria `"version"`. Um tipo mais novo ou desconhecido abre o projeto só
  para leitura, com aviso.
- **O desfazer troca de estado** (guarda o estado de cada passo, mais o `Diff` e o inverso), em O(1), com o `meta`
  atual reaplicado. "Desfazer tudo" volta à raiz original bit a bit, exceto o `meta`, cujo contador só cresce.
- **Gestos só fazem** `set`, `set_param`, `set_note` e `move_to`; qualquer outra coisa é recusada
  (`doc.gesture.structural`).
- **Campos que faltam assumem o padrão** em `insert`, `add_note` e `set`; notas inseridas ou coladas sempre ganham
  números novos.
- **Plugins desconhecidos são guardados intactos**, com aviso. Abrir um projeto conserta um `meta.issued` baixo demais
  e informa limites estourados como aviso, sem recusar.
- **Uma faixa nova sem instrumento** ganha uma saída `main` para o master. Trocar o tipo de plugin remove as lanes de
  automação de parâmetros que deixaram de existir, com aviso.
- **Campos acrescentados ao esquema:** `track.folder`, `track.collapsed`, `track.channels`, `transport.metronome`,
  `item.timebase` e `item.mute`.

## Servidor e cliente (decisão do usuário, 19/11)

- **O LinuxMusicBeta funciona como servidor e como cliente de rede.** Local: a interface fala direto com o núcleo no
  mesmo processo. Cliente: a mesma interface fala com um núcleo remoto pela rede; o som sai na máquina do núcleo.
- **Por isso a interface fala com o núcleo por uma porta (`CorePort`) cujos métodos recebem dados**, não
  fechamentos: a `LocalPort` chama direto, sem serializar (a eficiência fica), e a `RemotePort` manda os mesmos dados
  pela rede, com um espelho local da árvore para as leituras. Isso substitui a regra anterior de "a interface fala por
  fechamentos"; os fechamentos continuam dentro do núcleo.
- **O protocolo de rede é a `RemotePort`**: completo para a interface remota (mesma versão exigida) e um subconjunto
  estável para ferramentas externas.


## Do Rubber Band e do lilv (19/11)

- **Time-stretch com o motor R3 ("finer") do Rubber Band**, offline, numa `Operation`, com o tamanho de saída exato
  vindo do mapa de andamentos; o R2 oscila 1–2% de afinação em notas longas.
- **lilv ≥ 0.24.10.** O host não oferece o recurso `log:log` aos plugins LV2 (o Rust estável não define funções C
  variádicas); um plugin que o exige é recusado com `lilv.missing_feature`, e um que o tem como opcional funciona.
- **A ordem de destruição de um plugin LV2** é: encerrar o worker (join), entregar as últimas respostas, desativar e
  só então liberar. Um teste prova que a ordem do legado dá segfault.
- **Para exportar, o worker do LV2 é síncrono** (determinismo); ao vivo, roda numa thread própria.

## Do `lm-render` (19/11)

- **`gen` virou `generation`** (palavra reservada na edição 2024 do Rust).
- **`add` e `replace` levam um `NodeCore`** (processador + semente do `Rng` + escritor de dados ao vivo), montado na
  thread principal; `replace` leva o índice opcional de um nó de cauda (U32).
- **Todo nó tem `NodeIndex`**, inclusive somas e strips, para cada saída de faixa e cada canal da mesa terem medidor.
- **O pan é balanço em nós estéreo** (`esq = min(1, 1 − pan)`, `dir = min(1, 1 + pan)`); em outros números de canais é
  ignorado.
- **A proteção de ouvido do master se recupera sozinha** depois de 1 s abaixo de +24 dBFS.
- **Os comandos de prévia usam o anel de produtor**; com todas as vagas ocupadas, a prévia mais velha some com fade.
- **Curvas de automação:** `(e^{kt}−1)/(e^k−1)` com `k = 6·forma`; a forma é do segmento que termina no ponto.
- **Automação por bloco numa grade de 32 amostras** do relógio do motor, o que a deixa igual para qualquer tamanho de
  bloco.
- **Workers sem trava:** o processador e o bloco de buffers de um nó vão e voltam por anéis por worker; só nós acima
  de 3 µs vão para um worker; somas, atrasos e strips rodam em série, na ordem do programa (determinismo).

## O protocolo adiantado para depurar no ThinkPad (19/11)

- **Uma versão mínima do protocolo (`lm-remote` e a `RemotePort`) vem logo depois da `lm-session`**, antes do resto da
  primeira fatia: transporte, telemetria, avisos e atividades, a árvore espelhada, as ações e as edições, por socket
  local ou TCP na rede local. Motivo (conversa com o usuário): o núcleo roda no ThinkPad, a máquina Linux do projeto,
  como `lm-headless serve`, e a interface GPUI roda no Mac como cliente. Assim o ALSA é depurado onde roda, sem
  compilar a GPUI no ThinkPad, que tem pouco disco.
- Papéis completos, TLS para fora da rede local e o subconjunto estável para ferramentas externas continuam para a fase
  2.

## Do CI (19/11)

- **Licenças das dependências: Apache-2.0 é permitida** (decisão do coordenador) — a GPUI e o gpui-component são
  Apache-2.0, compatível só com a GPL versão 3; o binário que os contém é distribuído sob a GPLv3, o que a
  GPL-2.0-ou-posterior permite. As crates nossas continuam GPL-2.0-ou-posterior. GPL-3.0-only, LGPL-3.0 e AGPL seguem
  proibidas (`LinuxMusicBeta/deny.toml`).
- **O `--locked` sai dos jobs de build e teste**; um passo do job `policy` confere sozinho que o `Cargo.lock` bate com
  as crates do commit — um lock adiantado (commitado antes da crate) deixa só esse passo vermelho, não o CI inteiro.
- **No master, uma execução do CI nunca é cancelada pelo próximo push** (os agentes fazem push a cada poucos minutos);
  em PRs e outros ramos a execução antiga é cancelada.


## Do `lm-compile` (19/11)

- **Campos opcionais no esquema, sem padrão** (para não mudar os padrões congelados, U29): `point` do insert (P-20;
  ausente = `post-eq`, o padrão do `lm-dsp`), `mixer.mode` (P-17; ausente = analógico), `transport.transpose` e
  `track.transpose` (P-37; ausentes = 0 e "segue").
- **O rack é um nó por trecho de seções sem insert no meio** (estágios ganho, filtros, dinâmica, EQ+transformador; no
  master: drive, estéreo+bus comp, transformador depois do fader). Todos os trechos são o mesmo processador de rack,
  avisado de quais estágios roda, com os parâmetros num índice só (`rack_params`: os campos reais das seções, na
  ordem do esquema). A semente é o id do canal (P-21). **No modo digital não há nó de rack** (o canal fica limpo, e
  os campos do rack ficam guardados sem vaga).
- **O volume e o mudo da faixa ficam num nó de ganho só**, antes das saídas: uma vaga serve todas as saídas. O
  monitoramento tem uma **segunda vaga do mesmo volume** (`slots_of` devolve as duas; a `Session` escreve todas).
- **A PDC da saída de uma faixa vai para antes do nó da saída**, para o monitoramento, que entra no nó da saída
  principal, nunca ser atrasado (H5). MIDI ao vivo num instrumento armado passa pela PDC do caminho (como o áudio do
  instrumento).
- **Solo de canal:** tocam os solados, as fontes deles e tudo que eles alcançam por saída e envio; o resto é calado
  **antes do fader** (os envios pré também calam). Um canal que só alcança um audível por sidechain é calado **depois**
  do fader e continua alimentando o sidechain (F-MIX-014). O mudo do próprio canal fica depois do pan (os envios pré
  continuam, como numa mesa).
- **Um plugin em bypass (insert ou instrumento) não é construído nem chamado**: a cadeia passa por cima dele, e a
  faixa com o instrumento em bypass só toca os itens de áudio. Revisto em 20/11 a pedido do modo seguro (U21): o
  plugin que travou o callback não pode continuar no programa nem desligado (o `Renderer` chama todo nó); tirar o
  bypass pede um build, com o carregamento de sempre.
- **Itens de áudio:** `start`/`length` e o comping em tiques (ou amostras com `timebase: "audio"`), `sourceOffset`,
  fades e o `fade` do comping em amostras; sem comping toca a última lane; o crossfade do comping é linear, com o
  segmento anterior estendido pelo `fade` do seguinte. Itens acima de 30 s são classificados para o disco (P-32) e
  tocam da memória até o renderer ler do disco.
- **Um envio que repete a rota de outro** (mesmo nó de origem e destino) passa por um nó de soma próprio, para cada um
  ter a sua linha de atraso.
- **Contagem (I-5.5):** `TransportSettings::count_in { bars, on_play }`. Um `Play` parado conta `bars` compassos no
  metrônomo (mesmo com o metrônomo desligado), na fórmula e no andamento de onde a música começa, com os cliques onde
  caem os tempos que vêm depois do início; a música começa na amostra seguinte. Por padrão só ao gravar (gravação
  armada); a preferência `Environment::count_in_on_play` conta também ao tocar. `Stop` cancela.
- **Itens longos do disco (P-32), combinado com o agente do `lm-io`:** o recurso acima de 30 s vem do store como
  `ResourceState::OnDisk { rate, channels, frames }`; o compilador faz um `lm_render::stream` por segmento tocado (o
  mesmo enquanto o item não muda) e entrega `Compiled::streams` (recurso + `StreamFeeder`); a `Session` passa cada um
  ao `DiskReader` do `lm-io` (`serve(path, rate, feeder)`, thread própria `lm-stream`, separada da de gravação pelo
  U6). O anel: pedaços de 8192 quadros em duas regiões mapeadas por número (≈3 s à frente do playhead, ≈1 s do início
  do loop), cada vaga um seqlock sobre atômicos; o áudio nunca espera, o que falta toca como silêncio exato e vira
  `Telemetry::StreamUnderrun`. O renderer publica a cada bloco onde cada item precisa de dados e onde o loop volta. A
  exportação espera `streams_pending()` ficar falso (T8). Mudar um item longo refaz o stream e relê o começo.
- **Monitoramento de baixa latência (P-10):** com a preferência `Environment::low_latency_monitoring`, os inserts com
  latência no caminho da saída principal de cada faixa armada até o master ficam construídos mas fora da cadeia (o
  caminho perde a latência deles e a PDC que causavam); o rack não é desligado (não é plugin). Voltar não espera build.
## Da interface: `lm-port` e `lm-gui` (20/11)

- **`CorePort::finished()`** (pedido pela `RemotePort`, 20/11): os resultados dos pedidos que responderam `Waiting`,
  coletados a cada `pump` como `(RequestId, Finished)`; o corpo padrão devolve vazio (a `LocalPort` e o `FakePort`
  respondem tudo `Ready`).
- **O `CorePort` mora no pacote `lm-port`**, sem GPUI e sem fechamentos; ele reexporta os tipos que já existem
  (`Label`, `Error`, `OperationId`, `HostTime` do `lm-base`; `NodeId`, `Value`, `PathStep`, `Resource` do `lm-state`;
  `Diff`, `Feed`, `GestureId`, `Topic` do `lm-doc`; `Meter` do `lm-render`) e define só o que faltava — porque o
  `lm-session` (a `LocalPort`) e o cliente de rede (a `RemotePort`) implementam a mesma porta, e a interface não pode
  depender da sessão.
- **Acréscimos ao I-15** (escritos lá): `drain`, `modified`/`can_undo`/`can_redo`, `dismiss_notice`, `preferences`/
  `set_preference`, `status`, `devices` e o serviço do navegador — o protótipo mostrou que as telas precisam deles e
  que, no modo cliente, nada disso pode ser lido direto da máquina da interface.
- **O núcleo de mentira da interface é um `Document` de verdade** (`lm-port-fake`, só para testes e para rodar a
  interface antes do `lm-session`): desfazer, refazer, gestos, feeds e as regras dos esquemas são os do `lm-doc`; só o
  motor (telemetria, picos, carregamentos, avisos, navegador, dispositivos) é de mentira. A interface nunca depende
  dele (só `dev-dependency`).
- **`GestureId::from_raw`** no `lm-doc`, para a `RemotePort`, com teste de que um número não dado é recusado com erro.
- **`CorePort::plugins()` e `plugin_schema(id)`** (agente C, combinado com o A): o editor genérico de plugin, o
  "Trocar" do instrumento e o "acrescentar efeito" precisam dos plugins e dos esquemas, e no modo cliente eles são do
  servidor; corpo padrão vazio só na transição.
- **O mapeamento MIDI no `CorePort`** (agente A, com os nomes do agente da I-21): `midi_learn`, `cancel_learn`,
  `learning`, `unmap`, `drain_midi_ui` e `midi_activity`, com os tipos do `lm-midimap` reexportados, e
  `TelemetryView::hand_won`. Os métodos têm corpo padrão só na transição, para o `lm-session` e o `lm-remote`
  seguirem compilando; o falso já responde, e as portas de verdade passam a responder (a `LocalPort` pelo agente da
  I-21). A `RemotePort` precisa levar `hand_won` na telemetria do fio.
- **O programa é o pacote `lm-gui-app` (binário `linuxmusicbeta`, agente A, 20/11)**, e não um binário dentro do
  `lm-gui`:
  - o `lm-gui` fala só com o `CorePort` e não depende do `lm-session` nem do `lm-remote`; o programa junta os três;
  - o nome começa por `lm-gui` porque o `ci/check-core-no-gui.py` trata todo `lm-*` que não seja `lm-gui*` como
    núcleo, e o programa depende da GPUI;
  - sem argumentos, abre a sessão local e mostra os projetos recentes; com uma pasta, abre o projeto (uma operação,
    com a regra dos 200 ms desde a primeira janela); com `--connect`, abre como cliente do `lm-remote`.
- **Os testes de interface do `lm-gui` só rodam em compilação com `debug_assertions`:** a GPUI só grava os
  `debug_selector` nessa compilação, e os testes acham os elementos por eles. Na compilação de release da CI, os
  testes de interface ficam de fora, e não falham por seletores que não existem; o job de debug os roda.
- **O `lm-remote` leva as chamadas novas (agente A, 20/11):**
  - `ui`, o aprendizado em curso, `hand_won` e a atividade MIDI vão do servidor ao espelho como `prefs` e `status`:
    no retrato inicial e quando mudam;
  - as ações da interface mapeadas vão como `midi_ui`;
  - `set_ui`, `midi_learn`, `unmap` e `cancel_learn` são pedidos (o aprendizado responde com `Finished::Learned`);
  - o alvo de um mapa viaja na forma do documento (`midi.target`);
  - o roteiro único (LocalPort contra RemotePort) passou a cobrir tudo isso.
- **O documento da interface no `CorePort`** (`ui`/`set_ui`, agente A, pedido pelo agente da I-21 e pelo B): a
  seleção (`selection: { track, channel, plugin }`) e o estado da interface (as docas do B) vivem nele, como a I-3.8
  manda. A `Workspace` escreve a seleção quando uma página muda (observando as páginas), e só se ela mudou.
- **A interface da I-21 no `lm-gui` (agente A):**
  - "Aprender MIDI", "Tirar o mapa MIDI" e "Ligação MIDI…" ficam no menu de todo controle da mesa (botões,
    faders, envios, interruptores e seletores) e no botão de volume da faixa;
  - a janela de ligação é uma tela modal sobre um `MapTarget`;
  - "Mapas MIDI" é uma terceira página (Cmd/Ctrl+3), com busca, edição no lugar e o monitor de atividade;
  - o aviso "mova um controle" aparece com Cancelar enquanto se aprende;
  - as ações da interface mapeadas chegam pelo `drain_midi_ui` e rodam pelo registro.
  - Nas pistas do arranjo: o modo da pista, "a mão venceu" com Rearmar, o botão global no transporte, um menu da
    pista (abrir no editor, afinar, converter para a faixa de CC do item) e os itens movidos com `items.move`,
    levando a automação (ligado por padrão).
  - A edição de pontos (lápis livre, linha, seleção, escalar e deslocar) fica no editor de automação, do agente B.


### Janela de efeito e navegador no `lm-gui`: o que difere do legado e por quê (agente C, 20/11)

Regra do usuário (20/11): a janela de efeito e o navegador seguem o legado (layout, densidade, aparência); só muda o
que o modelo novo obriga. A janela do instrumento segue o design 05, aprovado.

- **Rack de efeitos (`EffectRackView`)**: mesmo cartão por efeito (LED que liga e desliga, engrenagem dos controles,
  nome em negrito embaixo), arrastar na vertical move de lugar, o mesmo menu (mover acima e abaixo, trocar por,
  carregar e salvar preset, remover) e "Acrescentar efeito" no fim. Diferenças:
  - **sem o botão Molhado/Seco**: o esquema do nó de plugin (`lm-doc`, `ty::PLUGIN`) não tem molhado/seco por insert;
    entra no cartão quando o esquema tiver;
  - **sem o DECAY**: o "dormir no silêncio" virou contrato do processador (P-07);
  - **sem o conector de pinos**: obsoleto (as saídas da faixa, I-3.9);
  - **o ponto do insert fica no cartão** (P-20: um por insert), e só no modo analógico. O legado tinha uma caixa só
    por canal. No modo digital não há seções do rack, então o rack é só a cadeia, como no legado;
  - **a engrenagem abre os controles embaixo do cartão**, dentro da doca, e não numa subjanela: a casca do programa
    novo não tem subjanelas MDI. Os controles vêm do editor genérico, montado pelo esquema do plugin;
  - **sem a "fonte de sidechain" no menu** por enquanto: a porta não diz quais plugins têm a porta de sidechain
    (o `PluginInfo` não traz as portas);
  - **sem "rodar num processo separado"**: terceiros sempre fora do processo, internos sempre dentro (I-9.6);
  - a latência de cada efeito e a da cadeia aparecem (a mesa compensa, PDC).
- **Navegador (`FileBrowser`/`PluginBrowser`)**: as mesmas páginas, o título da página, uma busca, a árvore densa com as
  pastas do usuário e depois as de fábrica sob a linha "Arquivos de fábrica", o clique que toca a prévia, o menu do
  item e o arrastar. Diferenças:
  - **as páginas como texto numa linha em cima**, no lugar da barra lateral de ícones com texto girado (a GPUI não
    gira texto, e a doca da esquerda já é a barra);
  - **a busca passa pelo núcleo** (`CorePort::search`): no modo cliente os arquivos são do servidor; o carregamento
    aparece embaixo, no componente único;
  - **a prévia é do núcleo** (`prepare_preview`, `preview_play`, I-7.3), e a linha de baixo diz o que toca, com ■;
  - **"mandar para" vai à faixa ou ao canal ativo**: a faixa da janela do instrumento ou o canal da janela de
    efeito (o legado só tinha a faixa ativa). "Mandar para uma faixa nova / um sampler novo / um SlicerT novo" esperam
    as ações de criar faixa e o sampler com amostra no esquema;
  - **"mostrar no gerenciador de arquivos" é uma ação do núcleo** (`browser.reveal`): o arquivo está na máquina do
    núcleo.
- **Janela do instrumento**: a aba MIDI diz que a entrada e a saída MIDI da faixa chegam com o roteamento MIDI do
  documento (o esquema da faixa ainda não tem); a afinação tem a afinação, a faixa do bend e "seguir a transposição
  geral" (P-37); o microafinador e as teclas (primeira, última, base) esperam os seus campos no esquema. O nome do
  último preset carregado é estado da interface (o documento não guarda nome de preset: um preset é um estado, 7a).
- **Campos novos da faixa e do projeto (agente C, autorizado, 21/11)**, todos opcionais, com o ausente mantendo o
  sentido antigo (U29: versões novas com `upgrade` sem mudança, padrões abençoados): na faixa (v2) `midiOutput`
  (porta por apelido, I-21.1; canal; programa, velocidade e nota fixos), `baseVelocity` (63), `microtuner` (liga,
  vaga da escala, vaga do mapa, "importar as faixas do mapa") e `keys` (primeira, última, base); na entrada MIDI da
  faixa (v2) `fixedVelocity`; no projeto (v2) `tuning` com 10 escalas e 10 mapas (F-IJN-019/020). Os arquivos Scala
  (`.scl`, `.kbm`) são lidos por `lm_doc::tuning` nas ações do núcleo `tuning.import_scale` e
  `tuning.import_keymap { slot, path }`, um passo de desfazer. `PluginInfo.sidechain` diz quem tem a porta de
  sidechain (pelas `Ports` do estado padrão).
- **Presets pelo núcleo**: `plugin.load_preset { node, entry }` (um passo de desfazer) e `plugin.save_preset
  { node, name }` (um arquivo nos presets do usuário), ações do núcleo; a lista vem da página Presets do navegador.

## Do `lm-plugin` (19/11)

- **O `build` recebe um `BuildContext`** (`spec`, `resources`, `channels`) em vez de três argumentos: um efeito precisa
  saber quantos canais tem o canal em que está inserido para alocar tudo no build (U31). O `Registry::build` entrega ao
  plugin o estado **completo** (os padrões congelados no que a árvore omite), validado, e roda o build em
  `catch_unwind` (`plugin.build.panicked`, H14).
- **O `lm-plugin` depende do `lm-doc`** (além de `lm-state` e `lm-render`): os tipos `Port`/`Ports`/`PortsFn` são os do
  documento, para não haver dois.
- **O índice de um parâmetro é a sua posição em `schema.params`**, e o valor do evento `Param` está na unidade do
  parâmetro (não normalizado), como a automação do `lm-doc`.
- **As funções de instrumento (F-IJN-002 afinação e bend, F-IJN-010, 011, 013, 014, 015) moram no estado do
  instrumento**, com chaves fixas acrescentadas ao esquema por `add_instrument_functions` (a faixa do `lm-doc` não as
  tem). Interruptores automatizáveis do legado viram parâmetros `Stepped` de 0 a 1 (`filterOn`, `chordOn`, `arpOn`,
  `*LfoX100`...); toda escolha é `Choice` (formas de LFO, tipo de filtro, acorde, direção e modo do arpejo,
  sincronia, `voiceMode` = poly/mono/legato, que junta os dois botões do legado). Mudar uma escolha reconstrói o
  processador, que adota as vozes. Microafinador (F-IJN-019) e MIDI (F-IJN-017) ficam para quando o projeto tiver as
  escalas e as portas: são do projeto, não do plugin.
- **Mono e legato são do instrumento, por nota-base, e não do `VoicePool`** (que fica sempre em `Poly`): no legado o
  mono vale para a nota-base com o acorde inteiro; no `VoicePool` ele cortaria o acorde. Trocar o modo tocando não
  solta as notas.
- **Peças numa grade de 32 amostras do relógio do motor**: o áudio é igual, bit a bit, para qualquer bloco múltiplo de
  32 (V3). O LFO sincronizado deriva a fase da amostra da linha do tempo (repetível no locate e no loop, U37; num
  andamento constante é a fase do compasso).
- **Velocidade linear como nível** para os instrumentos com as funções de instrumento (o legado multiplicava pelo
  volume da nota); o teste gerado confere −6,02 dB em 0,5 e −20 dB em 0,1.
- **Adopt sem estalo:** quando o estado muda (não um rebuild idêntico), o sintetizador velho toca cópias das vozes
  por 10 ms em crossfade com o novo (`Synth: Clone`, voz clonada sem alocar). Um rebuild do mesmo estado continua
  bit a bit.
- **Diferenças intencionais do legado:** o primeiro passo do arpejo começa na amostra da nota (o legado atrasava 1
  amostra); o predelay do LFO e a escolha da tabela do oscilador valem por peça de até 32 amostras (o legado, por
  período de 256 ou mais); o LFO aleatório tem semente fixa por instrumento.
- **O teste de 30 minutos (U35) roda sempre**: renderizar 30 min do TripleOscillator leva ~3 s.
- **Duas correções fora da pasta:** `Buffers::from_block`/`BlockShape` no `lm-render` (um host fora do renderer: a
  bancada de testes, um render isolado de plugin) e, no `lm-dsp`, a leitura da tabela de banda limitada numa fase a
  um fio de um período inteiro, que indexava além da tabela (achada pelos testes do TripleOscillator).

## Varredura de índices do `lm-dsp` (20/11)

Depois da correção da tabela do oscilador, todo índice que o `lm-dsp` calcula a partir de um `float` foi revisto
(`tests/edges.rs` leva cada um ao limite; o A/B continua passando contra as referências do legado). Cada correção
mantém o som dentro da faixa dos botões; fora dela, faz o que a conta manda em vez de ler fora do buffer:

- **Frequency Shifter:** com um atraso um fio acima de um número inteiro de quadros, a leitura um fio abaixo de 0
  arredondava, em precisão simples, exatamente para o tamanho do anel, e o índice passava do fim (o mesmo erro do
  oscilador). Agora volta à posição 0 do anel, o mesmo ponto da curva de Hermite.
- **Stereo Enhancer:** o índice de leitura dá a volta no anel com `rem_euclid` (o legado somava o tamanho uma vez só,
  o que bastava apenas para larguras de 0 a 180).
- **Compressor:** um lookahead maior que o buffer feito para os 20 ms do botão é limitado ao buffer (antes, a
  subtração de `usize` estourava).
- **Multitap Echo:** o deslocamento dá a volta no anel antes da soma, e um bloco menor que o período não lê além dele.
- **ReverbSC:** a posição de leitura volta ao anel com `rem_euclid` (igual à subtração única do legado sempre que
  esta bastava).
- **Sample:** `render` põe os pontos dentro da amostra (`Bounds::within`), como `play` já fazia, e um cursor de loop
  antes da amostra começa no primeiro quadro.
- **Sem mudança (já corretos, agora testados no limite):** Delay e linha mono (comprimento entre 0 e o da linha),
  Wave Shaper e Dynamics Processor (200 pontos, ramos para < 1 e ≥ 200), tabela do usuário (`% frames`), válvula
  (posição limitada antes do índice), formantes (vogal limitada a 4).

## A faixa do canal em segmentos (20/11)

- **`ChannelStrip::process_stages(frames, settings, had_input, tolerance, stages: Range<usize>)`**, com
  `STRIP_STAGES = 4` (0 ganho: pot, low cut, estágio de ganho; 1 filtros: HPF, LPF, mono bass; 2 dinâmica: gate,
  compressor; 3 EQ) e `stage_after(InsertPoint) -> usize` (o `k` do segmento `0..k` antes de um insert). Para o
  rack que o `lm-compile` parte quando um insert fica entre seções.
- **Bit a bit igual ao `process`** em toda divisão `0..k` + `k..4`, medidores e cauda incluídos
  (`tests/strip_stages.rs`, sob a armadilha de alocação). Para isso o `process` passou a ser os quatro estágios
  em sequência, cada seção com o seu começo, processamento e fim dentro do próprio estágio (antes, todos os começos
  vinham no início do bloco e todos os fins no final: a mesma conta, já que nenhuma seção lê o estado de outra; o A/B
  dá os mesmos números em todos os 166 casos). O único dado do bloco que cruza estágios, se algum filtro soa depois
  da entrada (a cauda), é acumulado estágio a estágio e fechado pelo estágio 3; por isso `had_input` tem de ser o
  mesmo em todos os segmentos de um bloco, e todo bloco passa por todos os estágios, em ordem.

## MIDI e automação (decisão do usuário, 20/11; especificação em `interfaces.md`, I-21)

- **Do usuário:**
  - "tudo pode e deve ser mapeável MIDI";
  - interfaces MIDI, gravação MIDI e gravação e edição de automação;
  - "o suporte a retorno MIDI também, tudo na 1.0".
- **Minhas, com o motivo:**
  - **O mapa mora no projeto**, porque o botão tem que voltar a obedecer ao abrir o projeto. Os dispositivos ficam
    por apelido, resolvido nas preferências da máquina, para o projeto funcionar noutra máquina (P-36). Os mapas
    que valem em todo projeto ficam nas preferências, com o mesmo esquema.
  - **"Tudo mapeável" sem código por recurso:** os alvos são só parâmetro, campo e ação, os três tipos que já
    existem, com um teste gerado que mapeia todos.
  - **Parâmetros mapeados no `Renderer`**, na amostra do evento e com atraso fixo de um bloco: menor latência,
    nenhum jitter, e a gravação fica onde foi tocada. Campos e ações vão pela `Session`, em até 10 ms.
  - **Um gesto MIDI vira um passo de desfazer**, encerrado 500 ms depois da última mensagem ou ao soltar o toque.
  - **Mackie Control Universal na 1.0**, porque é o protocolo dos controladores com faders motorizados que o retorno
    pede. HUI, MIDI Clock e MTC ficam depois da 1.0: o legado não tem.
  - **Modos de automação** Desligado/Ler/Toque/Trava/Escrever, os de todo DAW. Gravação: uma passada por passo de
    desfazer.
  - **CC sem mapa gravado vai para o item MIDI** (toca de volta no instrumento); **CC com mapa vira automação** do
    alvo.
  - **Gravação MIDI à prova de queda**, com registro só de acréscimo e `fsync` a cada segundo, como o áudio.

## Do `lm-io` (20/11)

- **A barreira é a posse da callback:** a thread do dispositivo é dona do `CallbackHandle`, que devolve a callback (o
  `Renderer`) à `Device` no seu `Drop`. "A callback voltou" é a barreira; sem ela em 2 s, a `Device` é abandonada e uma
  thread vigia fecha o dispositivo se a callback um dia voltar (D11).
- **O gravador roda na mesma callback que o `Renderer`**, logo depois dele, com as `RecordSpan` do bloco. Grava todo
  bloco tocado com a gravação armada (pré-roll e fora do punch são alças, U4); as `RecordSpan` dizem onde está o take.
  Um arquivo por entrada e por passada do transporte; as voltas do loop ficam no mesmo arquivo, com cortes (U1).
- **Armar é confirmado:** `Device::arm` manda o `RecordSender` pela callback, e a `Session` espera `is_armed()` (um
  período) antes de mandar `Command::Record`. Sem isso, o primeiro bloco gravado podia escapar.
- **`recording.json` em linhas JSON** (uma por evento, só acréscimo, lido até a primeira linha inválida); as `RecordSpan`
  vão em pedaços de 1 s para uma queda guardar onde está o take; ao finalizar vira `<hash>.recording.json`, removido pela
  `Session` quando o take entra no diário (`forget_record`).
- **Nomes dos arquivos de áudio:** `audio/<hash>.<ext>` com a extensão do formato (`wav` para RF64 e WAV, `flac`, `aiff`,
  `ogg`, `mp3`); importar com a mesma taxa copia os bytes, com outra converte para RF64 f32.
- **Picos:** arquivo próprio `LMPEAKS\0` versão 1 (cabeçalho, contagem de pares por nível, os níveis em `i16` LE por
  canal), base 64 e fator 4 até um par; escrita por `.tmp` + rename.
- **Calibração:** rajada de ruído branco com janela de Hann, 16384 quadros por padrão (com 2048, alto-falante e microfone
  do ThinkPad com o ventilador davam SNR 4,6); o pico de correlação tem que ficar 8× acima do RMS da correlação fora do
  pico e do eco de 100 ms, e as cinco medidas concordarem em 1 ms.
- **Correção de deriva:** reamostrador adaptativo na captura, com um PI sobre o nível da FIFO de entrada (limite de
  2000 ppm); testado com relógios a 300 ppm.
- **O `ResourceView` fica no `lm-io`** (chaves de 64 hex; `get` para o que está na memória, `on_disk` para itens longos).
  O `lm-compile` tem o próprio trait `Resources`, que a `Session` implementa por cima.
- **Recursos com taxa diferente da do projeto não tocam do disco** (`io.stream.rate_mismatch`): toda importação converte.
- **No ThinkPad, rtkit e logind só respondem a um processo da sessão gráfica:** os testes rodam com `systemd-run --user`
  (o polkit trata a sessão SSH como remota). O rtkit dá `SCHED_RR`, não `SCHED_FIFO`.


## A mesa de mixagem (decisão do usuário, 20/11)

- **O que o usuário disse:** depois de ver o protótipo, que "o design da mesa de mixagem quebrou muito", que o
  original tinha muitas coisas boas e que o novo ficou verboso.
- **A causa:** o design da mesa não está aprovado, então a referência era o legado, mas o protótipo o "repensou".
- **A decisão:**
  - **A mesa nova é conservadora com a do legado.** A analógica segue o desenho exato de uma mesa analógica de
    verdade, como no legado: a ordem das seções, os botões, as posições e a aparência.
  - **A mesa digital continua simples**, como no legado.
  - **Há um botão visível para alternar entre a mesa analógica e a digital** (`mixer.mode`, P-17), e a página
    inteira troca de desenho.
  - Só muda o que o modelo novo exige: roteamento livre, várias saídas por faixa, os inserts nos pontos do rack, a
    faixa sem pan, os faders sobre os medidores e a página separada. Cada diferença fica registrada com o motivo.
### A mesa nova e a do legado: o que muda e por quê (20/11, agente A)

A página Mixagem do `lm-gui` reproduz `MixerView`, `MixerChannelView` e `MasterSectionView` do legado: as mesmas
medidas em pixels, as mesmas cores, a ordem das seções (ENTRADA, FILTRO, DYN, EQ), os botões e as posições, os dois
modos e a chave ANALOG / DIGITAL. Só muda o que segue:

- **Roteamento livre.** O botão de envio e o botão do envio do canal selecionado ficaram como no legado, agora sobre
  os `sends` do modelo: o botão cria ou apaga o envio do canal selecionado para aquele canal. A saída de um canal é
  exclusiva no modelo novo, então ela é escolhida no menu do canal ("Saída") e na tela de roteamento. Os menus de
  sidechain do legado saem da mesa: um envio para a entrada de sidechain de um insert (`send.port`) vem com os plugins
  que têm essa entrada.
- **Várias saídas por faixa:** ficam na tela de roteamento, não na mesa.
- **Inserts nos pontos do rack (P-20):** no legado havia um ponto por canal, num seletor nos ajustes do canal. No
  modelo, cada insert tem o seu ponto, então ele fica na linha de cada efeito do rack. No modo digital o seletor some,
  porque lá todos os pontos viram a cadeia simples.
- **Os faders sobre os medidores:** o fader do legado já desenhava os medidores atrás da alça, e isso fica igual. A
  Mixagem é uma página própria, e não uma janela.
- **Transformador e tolerâncias por canal:** o modelo tem `rack.transformer.on` e `rack.tolerance.on` em cada canal, e
  o legado não os mostrava na faixa. Eles viram dois interruptores com LED nos ajustes do canal (modo analógico).
  - Os `on` do tubo, da EQ e da dinâmica não aparecem, como no legado.
  - RATIO e RANGE no zero desligam o compressor e o gate.
- **O master só tem M:** o master do modelo não tem solo, inversão de fase nem troca de lados.
- **Retornos e grupos:**
  - Um retorno é um canal que recebe envios (P-19). Os botões FX1 e FX2 apontam para os dois primeiros retornos.
  - Um grupo é um canal para onde outros saem (P-18). Ele tem o triângulo do legado para esconder os seus canais.
  - A vinculação de faders (P-18) entra como um LED "Link" nos ajustes, junto com Ctrl ou Cmd+clique.
- **A monitoração** (LEVEL, DIM, CUT, MONO, SIDE) vai para `preferences().monitor`. É da máquina, e não entra no
  documento, no desfazer nem na exportação.
- **As cores da mesa** são as do legado nos dois temas, como num aparelho.
- **Campos novos e opcionais no `lm-doc`, que o legado tinha e o modelo não:**
  - no canal: `trim`, `mono`, `phaseInvert`, `swap` e `color` (este pode ser nulo);
  - no master: `preLeft`, `preRight` e `preLink`, os faders PRE e o LINK.
  - Eles ainda precisam chegar ao motor, pelo `lm-compile` e pelo `lm-dsp`.
- **Indicadores aproximados até a telemetria os trazer:**
  - SAT e CLIP leem o pico do canal, e não o estágio de ganho;
  - o goniômetro e a correlação vêm do nível;
  - o medidor de redução de ganho do compressor de barramento fica em 0.
- **Botões e menus:**
  - Um duplo clique num botão volta ao padrão (F-CTL-005); no legado, ele abria a caixa de valor. A roda do mouse nos
    botões ainda falta.
  - No rack, cada efeito tem LED, nome, ponto, W/D (o `wet` do insert, parâmetro `insert.wet`) e "Controles". Os
    botões DECAY e GATE do `EffectView` do legado não voltam: eram do mecanismo de dormir do LMMS, que o motor novo
    não tem.
  - "Cor > Mudar" mostra as 8 cores das faixas, no lugar do seletor de cores.
  - O renomear acontece no próprio canal: Enter ou F2 começa, e Enter termina, como no legado.
- **Abaixo da altura mínima do legado** (a faixa mais alta), a página rola em vez de espremer os faders.

## Os efeitos do legado como plugins (20/11)

- **Um pacote, `lm-plugins-effects`, com um módulo e um `static Descriptor` por efeito** (cada um com o seu id, o seu
  esquema e as suas fixtures), em vez de um pacote por efeito: os efeitos dividem o processador (`effect`: um
  algoritmo por par de canais, peças na grade de 32 amostras cortadas também nos eventos, rampas dos botões na peça em
  que mudam, `adopt` que leva a memória do som) e a bancada do A/B, e 20 pacotes quase vazios pesariam no disco e na
  compilação. Para isso o `contract_tests!` do `lm-plugin` ganhou um segundo argumento, a pasta dos dados do plugin
  (`tests/data/<id>`).
- **Chaves dos parâmetros:** o nome do botão do legado em camelCase (`leftGain`, `stereoDifference`), com as faixas,
  os padrões e as unidades do legado. Interruptores automatizáveis são parâmetros `Stepped` de 0 a 1, como decidido no
  `lm-plugin`. Frequências usam escala logarítmica (só a apresentação: os valores são os do legado).
- **Só o sinal molhado:** a mistura seco/molhado é do host, como no legado.
- **O A/B passa pelo plugin:** o estado do plugin montado a partir dos botões de cada caso do `lm-dsp/ab`, a mesma
  entrada, a bancada do contrato, comparado com a referência que o próprio plugin do legado escreveu. Onde o
  harness do legado gira um botão antes do primeiro período (a rampa a partir do padrão), a comparação começa no
  segundo período.
- **Diferenças intencionais:** a realimentação do Dispersion fica dentro de ±0,999 (em ±1 o laço de passa-tudo não
  perde nada e cresce sem fim com uma entrada na frequência de um polo; qualquer valor até ±0,999 é o do legado); o
  Bitcrush não zera de novo os contadores de taxa nos dois períodos seguintes a uma mudança (encanamento do legado,
  não DSP).
- **O Linkwitz-Riley do `lm-dsp` virou duas seções Butterworth em forma direta transposta** (o mesmo filtro): a
  seção única de 4.ª ordem do legado, em forma direta II, guarda um estado interno de (taxa / corte)^4 vezes o sinal, e
  mudar o corte tocando soltava esse estado num estouro (24 dB no caso `crossovereq` do A/B; 1e37 varrendo de 50 Hz a
  20 kHz, o que derrubava a suíte do contrato do Crossover EQ). Os casos `crossovereq` e `lomm-deep-linked` do A/B do
  `lm-dsp` passam a comparar até a mudança (lista `UNTIL`, com o motivo); antes dela, e em todos os outros casos que
  usam o filtro (Bitcrush, Slew Distortion, LOMM), o A/B continua nulo.
- **O bloqueador de DC do ReverbSC tem o mesmo corte (~77 Hz) em qualquer taxa** (U36): o polo 0,99 do Soundpipe é
  o do legado a 48 kHz; fixo, ele dobrava o corte a 96 kHz e o reverb perdia 1,6 dB de graves.
- **Dual Filter:** os casos com o Moog em ressonância 2,5 (auto-oscilante) não se comparam pelo plugin: a fase do tom
  depende de como os botões chegaram lá (o harness do legado os gira a partir dos padrões no primeiro período); o A/B
  do `lm-dsp` os cobre com as rampas do legado.
- **Botões de ação do legado** (reiniciar o deslocador e o LFO do Frequency Shifter) são parâmetros `Stepped` de 0 a
  1 que agem ao ir a 1, como o botão.
- **Eq:** a ressonância de uma passagem de 24 ou 48 dB/oct vai na primeira seção, as outras em 0,707 (o padrão do
  legado). O legado punha a ressonância em todas as seções, e o pico era a ressonância ao quadrado ou à quarta (+80 dB
  com ressonância 10 a 48 dB/oct), o que derrubava a suíte do contrato; no padrão as duas formas são iguais. O `lm-dsp`
  guarda o modo do legado (`resonance_on_every_section`) para o seu A/B. O analisador de espectro da janela do Eq é
  análise do host sobre o áudio do canal, não um quadro do plugin.
- **Uma divisão por zero do Eq do `lm-dsp`:** o crossfade dos filtros dividia por `frames − 1`; numa chamada de um
  quadro (uma peça cortada num evento) isso dava NaN. Uma chamada de um quadro termina o crossfade nele.
- **Latência de lookahead** (Compressor, LOMM): a do anel inteiro (20 ms), informada no build quando o lookahead está
  ligado; o processador pede `LatencyChanged` ao host quando o botão muda, e o host recompila a compensação.
- **O nível máximo da suíte do contrato** (`TestHints::max_level`, padrão 1000): um plugin cujos botões somam mais
  ganho legítimo declara o seu (o LOMM: 288 dB). Acima dele, ou não finito, é estouro.
- **Granular Pitch Shifter:** trocar o comprimento do anel não adota (o anel muda de tamanho; o velho toca a cauda,
  U32), e cada parâmetro cuja faixa o legado mudava com o anel é limitado à faixa daquele anel. Dois casos novos no A/B
  do `lm-dsp` (`granular-defaults-without-spray`, `granular-grains`), escritos pelo plugin do legado, comparam o plugin
  sem as rampas do primeiro período do harness.
- **Amplifier e Stereo Matrix** existem para abrir projetos antigos (F-FX-005 e F-FX-028 continuam obsoletos no
  navegador).


## Do `lm-remote` (20/11)

- **Transporte: socket Unix (0600) na máquina e TCP só em 127.0.0.1, alcançado de outra máquina por túnel SSH**
  (`ssh -N -L 7400:127.0.0.1:7400 host`), **em vez de TLS no programa**. Motivo: o SSH já autentica (chaves) e
  cifra, e é o caminho que existe hoje até o ThinkPad (Cloudflare + chave do Mac); TLS exigiria uma pilha de
  criptografia nova na lista do `cargo-deny`, certificados e pareamento próprios, sem ganho para o caso do usuário. Um
  `--tcp` em outro endereço funciona (para uma rede local confiável), mas não é o padrão. TLS pode entrar depois sem
  mudar o protocolo (é só o `Stream`).
- **Autenticação:** no socket Unix, só o próprio usuário conecta (permissão 0600) e ele é `admin`. No TCP, um token
  dado no aperto de mão (`--token TOKEN:papel`), comparado inteiro; o papel pedido nunca passa do do token.
- **Versões:** a interface (`mode: "gui"`) exige o mesmo `PROTOCOL` e o mesmo programa (`CARGO_PKG_VERSION`); a recusa
  é `remote.version_mismatch` com as duas versões. Ferramentas (`mode: "tool"`) falam o subconjunto estável
  (`find`, `get`, `apply`, `run_action`, `undo`, `redo`, `telemetry` até 10 Hz, `health`) com `tool_api` 1.
- **Reconexão:** a `RemotePort` tenta de novo em segundo plano, com pausa crescente de 100 ms a 2 s, sem limite de
  tentativas (o indicador mostra `Reconnecting`); versão ou token errados são finais (`Failed`). Ao voltar, o espelho
  é ressincronizado pelo snapshot e os `Feed`s recebem a diferença como um `Diff` (por id). Pedidos em voo falham com
  `remote.disconnected`; os gestos abertos foram cancelados pelo servidor (I-20.5).
- **O espelho é um `Document` do `lm-doc`** atualizado por `Document::mirror(raiz, diff)`: o servidor manda as
  diferenças da árvore (achadas por `same_node`: listas reconstruídas pelo `id`, mover uma nota manda uma nota) e o
  `Diff` que o núcleo publicou, que chega aos `Feed`s do cliente como está — por isso a interface vê os mesmos
  `Diff`s nos dois modos.
- **Os resultados `Waiting`** são coletados por `CorePort::finished()` (acréscimo ao `lm-port`). A resposta de uma
  mudança sempre chega depois da atualização do espelho que ela causou.
- **Orçamentos de tempo nos testes** (V8 do `lm-compile`): o orçamento real (5 ms) vale onde é medido; com `CI`
  definido (runners compartilhados, o macOS do GitHub deu 5,3 ms), vale um teto de 3× o orçamento, ainda bem abaixo de
  uma regressão de verdade (o caso quadrático que o teste pegou dava 17 ms), e a medida é impressa no log.
- **Conectar é em segundo plano** (`RemotePort::start` → `Connecting::poll`), pela regra dos 200 ms.
- **Chaves de tradução recebidas** (`Label`, `Notice`) viram `&'static str` por uma tabela limitada (16 mil chaves de
  até 160 bytes); o excesso vira `remote.unknown_key`.


## O visual próprio dos plugins (decisão do usuário, 20/11; `interfaces.md`, I-22)

- **Do usuário:** os plugins nativos do legado com identidade visual própria (instrumentos e efeitos como o Guitarix
  e o AmplifikationLite) são portados no próprio estilo. Os sem visual ou com botões soltos usam o editor genérico.
- **Minhas:**
  - **Os painéis moram num crate de interface, `lm-gui-plugins`**, nunca no crate do plugin, que continua sem
    GPUI, e falam só pelo `CorePort`, para funcionar também no modo cliente.
  - **A arte é copiada do legado.**
  - **As imagens de terceiros só entram com a permissão registrada.**
- **Do agente dos painéis (`lm-gui-plugins`, 20/11):**
  - **Um painel é dado** (`spec.rs`: tamanho, fundo, cada controle com posição, tipo e alvo), lido do `*View.cpp` do
    legado, e uma só view (`panel.rs`) desenha e trata os gestos de todos. Motivo: os 46 painéis repetem os mesmos
    widgets; os gestos de F-CTL-005/006 ficam escritos uma vez e testados por um só roteiro.
  - **Cada controle é rasterizado na CPU numa imagem por estado** (`raster.rs`, com a semântica das chamadas de
    QPainter que o legado faz: linha com ponta quadrada ou redonda, arco nos ângulos do Qt, caneta com gradiente
    radial, pixmap por cima), guardada pelo ângulo em graus inteiros, como o `m_cache` do `Knob`. Motivo: o custo da
    GPUI é por primitiva (um knob em caminhos seria 3 ou 4) e desenhar como o Qt desenha é o jeito de ficar fiel. O
    painel desenha na escala da tela (linhas nítidas em 2x; os pixmaps do legado, todos 1x, reamostrados como o Qt).
  - **Sem dependência nova:** o PNG é lido pelo `image` (o mesmo da GPUI) e o fundo SVG vai para a GPUI como imagem
    (ela usa o resvg). Não entrou `tiny-skia` nem `resvg` no `[workspace.dependencies]`.
  - **Os gestos são os do legado, com os números do legado:** 0,4 % da faixa por pixel, Shift 4× mais devagar
    (com o resto que não chegou ao passo carregado, então em passo 1 a mudança vem a cada 3 pixels, como lá); roda
    com 100/10/1000/2000 notches; o passo de cada controle é o do modelo do legado (`KnobSpec::step`), porque o
    `ParamSpec` não tem passo; a escala "logarítmica" é a curva de potência `e` do legado (`linearToLogScale`).
    Shift no clique carrega o valor (F-CTL-006); por isso o Shift lento vale quando apertado durante o arrasto, como
    no legado.
  - **O alvo de cada controle é o mesmo de um mapa MIDI** (`map_target`: `MapTarget::Param { node, key }` ou
    `MapTarget::Field { node, path: [state, <campo>] }`). Escolhas (forma de onda,
    modulação) são campos `Choice`, mapeáveis por `MapTarget::Field`; parâmetros são mapeáveis e automatizáveis.
  - **Menu de todo controle:** restaurar, copiar e colar o valor (uma área de cópia para todos os painéis, como o
    legado), "Aprender MIDI" e "Tirar o mapa MIDI" pela porta (`CorePort::midi_learn` e `unmap`, no escopo do
    projeto), criar ou tirar a pista de automação (no dono do plugin: faixa, canal ou master) e linear/logarítmico
    (P-06, por enquanto só na view: falta o documento da interface na porta).
  - **O "logarítmico" de cada knob é o do modelo do legado** (`KnobSpec::log`, de `setScaleLogarithmic`), não o
    `Scale::Log` do esquema, que é só a apresentação do editor genérico: o Kicker tem as frequências em `Log` no
    esquema, mas os knobs do legado eram lineares no curso.
  - **Diálogos montados por layout do Qt** (BassBooster, ReverbSC, Flanger, Amplifier, StereoMatrix): no legado a
    posição vinha do layout com a fonte do sistema, então nunca foi um número fixo. O painel tem o tamanho da arte
    (o tamanho para que ela foi desenhada) e cada knob fica centrado na sua célula igual dentro das margens de 9 px
    do layout; o StereoMatrix põe os knobs nos círculos que a arte desenha para eles. Os rótulos desses knobs usam
    10 px (`SMALL_FONT_SIZE`), o tamanho que cabe na arte.
  - **"MIDI binding…" no menu** emite `PanelEvent::OpenBinding(MapTarget)`; a janela que hospeda o painel (que pega
    a entidade com `open_entity`) responde abrindo a janela de ligação do `lm-gui` (o painel não depende do `lm-gui`).
  - **O `VolumeKnob` guarda o seu 0 dBFS** (`setZeroDbfsPoint`: 100 nos instrumentos, 1 no FDBK e no NOISE do
    Flanger), para o texto e a digitação em dBFS darem os números do legado.
  - **Ligar dois controles com Ctrl+arrasto (P-04) não entra ainda:** o controle compartilhado não existe no núcleo.
  - **O arquivo da onda do usuário (TripleOscillator) é importado pela porta** (`CorePort::import`) e o campo e a
    escolha mudam num passo. No cliente a resposta chega depois: o host entrega por `PortHost::take_imported`, o
    que pede ao `Core` do `lm-gui` guardar os `Finished::Imported` do `finished()` até o painel pedir.
  - **`lm-port-fake` ganhou `FakePort::with_plugins`** (os esquemas reais dos plugins substituem os falsos, e os nós
    do demo desses plugins começam nos padrões reais) e `last_param_write`, para os testes moverem os parâmetros de
    verdade. O fake continua sem depender do `lm-plugin`: quem chama passa o esquema.
  - **Textos:** as chaves são as frases em inglês do legado, então as traduções do `pt.ts` do legado entram como
    estão (`locales/pt-BR/panels.txt`); o teste confere que nenhuma falta.
  - **Inventário (`docs/plugins-visual.md`):** 46 com visual próprio, 4 genéricos, 2 filtros sem janela, 10 de
    terceiros fora do processo (os 3 do Carla obsoletos). Amplifier e StereoMatrix (obsoletos no navegador) ganham
    painel porque existem para abrir projetos antigos.

## Do `lm-session` (20/11)

- **Salvar roda na thread `Disk`, em ordem com o diário**: as linhas mandadas antes do salvamento ficam no arquivo, as
  depois vão para o diário novo; uma falha (disco cheio) deixa o arquivo velho intacto e o diário continua nele. O
  número das linhas nunca volta a zero na sessão.
- **Recuperação:** o autosave guarda a base e o número até onde vai; ele vence quando o diário não tem tudo o que
  ele tem (uma linha recusada pelo disco ou marcada incompleta), e só as linhas depois dele são repetidas. Um diário
  de outra base é posto de lado (`.orphan`), nunca aplicado nem apagado.
- **A trava (H10)** compara a hora de início do processo dada pelo `ps` (`lstart`), igual no Linux e no Mac, sem
  `unsafe`; o mesmo processo abrindo o projeto duas vezes abre a segunda só para leitura.
- **O documento da interface tem um registro de esquemas só dele**, para o `versions` do projeto salvo listar só os
  tipos do projeto.
- **Projeto sem título** numa pasta temporária própria, apagada ao fechar a sessão se não tiver áudio.
- **Os takes de uma gravação entram numa transação `Origin::Recording` só** (um desfazer tira a gravação inteira) e
  são finalizados um de cada vez enquanto o `lm-io` não corrigir a colisão de nomes de takes idênticos.
- **Um plugin do documento que o programa instalado ainda não tem conta como carregando**: nenhum alvo fica
  "pronto" antes de soar (T10).
- **Rack:** o processador do rack mora no `lm-session` (o compilador pede, a sessão constrói), sobre o
  `ChannelStrip::process_stages` do `lm-dsp` para os trechos entre inserts; o DRIVE do master entra como o ganho do
  potenciômetro antes do estágio.
- **`lm-headless render` renderiza o arquivo salvo, não o diário**, para ser a referência determinística.


## Os editores de notas no `lm-gui`: o que difere do legado (agente B, 20/11)

Regra do usuário (20/11): os designs 04 e 09 não foram aprovados, então o Piano Roll, o editor de automação e o
editor de passos seguem a tela do legado; esta lista é só o que muda, e por quê.

- **B-PR-1 Sem ícones:** os botões das barras têm texto curto ou um símbolo (▶ ● ■), porque o `lm-gui` ainda não tem
  o conjunto de ícones do legado. Quando os ícones entrarem, voltam os do legado, no mesmo lugar.
- **B-PR-2 Loop e cursor do item na régua (P-26):** o legado tocava o clipe em loop inteiro; agora a régua do editor
  tem o laço do item (botão direito ou Shift na régua mexe nos pontos) e o cursor do item. Espaço toca só o item.
- **B-PR-3 Fantasma escolhido no editor:** o legado marcava o fantasma pelo menu do clipe na Composição; aqui o combo
  "Notas fantasmas" da barra lista os itens MIDI e limpa, porque o menu do item é de outra tela. O menu do item pode
  ganhar a mesma ação depois.
- **B-PR-4 Faca e Strum no menu de ferramentas:** como no legado (são modos, com Shift+K e Shift+J), e o botão do menu
  mostra o modo ativo.
- **B-PR-5 Apagar arrastando vira uma transação ao soltar:** o documento não aceita tirar notas dentro de um gesto;
  as notas somem da tela durante o arrasto e saem num passo de desfazer ao soltar. Copiar com Shift arrastando também
  entra ao soltar, com a prévia na tela. Desenhar é um passo e o tamanho arrastado outro, como no legado.
- **B-PR-6 Pan e curva de afinação por nota são campos do item** (`pan`, `bend`, P-24), não a automação por nota
  do legado (`detuning`), que abria o editor de automação. Shift+clique no modo afinação não abre outro editor.
- **B-PR-7 Importar e exportar clipe (`.xpt`)** saiu (P-25): copiar e colar entre projetos substitui.
- **B-PR-8 A área de velocidade tem altura fixa** (o legado arrastava a borda): a doca de baixo do `lm-gui` tem altura
  fixa hoje. Volta quando a doca for redimensionável.
- **B-PR-9 Teclas:** as do legado, pelo registro de ações no contexto `PianoRoll`; Esc também desmarca (o legado só
  tinha Ctrl+Shift+A), porque Esc já fecha e cancela no resto do programa.
- **B-AE-1 Tangentes viram a `curve` do ponto:** o modo "editar tangentes" (Shift+T) saiu; no lugar, cada trecho
  "curva" tem uma alça no meio que se arrasta para dobrá-lo (−1 a 1). A progressão (discreta, linear, cúbica com
  tensão) passa a valer por ponto (o trecho que termina nele), não pelo clipe todo: os botões dão a forma dos pontos
  novos e mudam a dos selecionados.
- **B-AE-2 Valores de saída (Shift+C)** continuam, como o campo `out` do ponto (P-27); o botão direito volta a saída à
  entrada, como no legado.
- **B-AE-3 Uma lane por parâmetro, de dono a dono:** não há clipe de automação. O editor abre a lane pedida (ou a
  primeira); o combo "Lane" escolhe outra. Ao lado, o modo da lane (Desligado, Ler, Toque, Trava, Escrever) e
  "Rearmar" (I-21.5). O indicador "a mão venceu" entra quando o `lm-port` publicar `hand_won`.
- **B-AE-4 Acréscimos do I-21.5:** lápis livre (Shift+P, afinado ao soltar), seleção por retângulo (Shift+S),
  recortar, copiar e colar, escalar e deslocar valores e afinar a curva (menu "Ferramentas"; ações do núcleo
  `automation.scale` e `automation.thin`, com a conta feita na interface quando o núcleo não as tem).
- **B-AE-5 Cada edição grava a lista `points` inteira** (os pontos não têm id); arrastar é um gesto, e acrescentar um
  ponto é um passo e arrastá-lo outro.
- **B-PE-1 Sem padrões:** o legado tinha uma lista de padrões e uma faixa por padrão; no modelo novo um "padrão" é um
  item MIDI que se repete, então a barra escolhe os itens mostrados (os de bateria e os que se repetem, de início).
  Acrescentar, tirar e clonar passos valem para todos os itens mostrados, como valiam para o padrão inteiro.
- **B-PE-2 Bateria é um instrumento só:** no legado cada peça era uma faixa; aqui o item de um kit (AVL Drums,
  DrumMachines) mostra uma linha por peça que toca (ou todas, "Todas as peças"), pelos nomes do mapa General MIDI.
  Um item melódico é uma linha de passos com altura (Alt+arrastar muda a nota; um passo novo toma a nota anterior, ou
  a nota padrão do legado, 57).
- **B-PE-3 O tamanho do passo e o swing** são campos do item (`step`, `swing`), escolhidos na barra.
- **B-PE-4 Sem botões de volume e pan por linha:** no legado a linha era a faixa (com os botões dela); aqui as linhas
  são peças de um item, e o volume e o pan ficam no cabeçalho da faixa e na mesa.
- **B-PE-5 "Acrescentar faixa de amostra/automação" ao editor** saiu: amostras em batida são o instrumento de pads
  (P-23) e a automação mora na lane do parâmetro.
- **B-LK-1 Os ícones do legado** (`data/themes/default`, 307 PNG/SVG, 1,3 MB) ficam embutidos no `lm-gui`
  (`assets/icons`, `ui::icon(nome)`, o nome é o do arquivo, como o `getIconPixmap` do legado). As barras das telas
  usam os ícones onde o legado tinha ícones; texto só onde o legado tinha texto. O transporte (design 03 aprovado)
  mantém o desenho e troca os símbolos pelos ícones do legado; os botões pintados da mesa (página de mixagem) ficam
  como estão, porque reproduzem os da mesa do legado.
- **B-LK-2 O tema escuro é o do legado** (`style.css`): texto `#d1d8e4`, fundo da janela `#111314`, verde `#0bd556`,
  grade `#3c444c`, e campos próprios dos editores (grade, notas, teclas e velocidade do piano roll; grade e gráfico
  da automação), dos menus, do laço e dos passos. O tema continua dado (`themes/*.txt`); o claro define os mesmos
  campos. As cores próprias da mesa ficam na página dela (decisão do agente A).
- **B-LK-3 As docas se redimensionam** arrastando a divisória (a de baixo, dos editores, e as dos lados), e o
  tamanho fica no documento da interface (`docks: {bottom, left, right}`, I-3.8), salvo com o projeto; no legado as
  janelas eram livres (MDI), e o tamanho do editor ficava no arquivo do projeto.

## Expressões de nota, loop e swing do item MIDI (20/11)

- **A forma do evento** (combinada com o autor das funções de instrumento do `lm-plugin`):
  `EventKind::NoteExpression { note, expression: Pan | Tuning, value, ramp }`. O valor vai linear do atual até `value`
  em `ramp` amostras a partir do offset (0: na hora); um evento novo substitui uma rampa em andamento a partir de onde
  ela chegou; toda nota começa com pan 0 e afinação 0; a ordem no mesmo offset é depois do note-on. No item MIDI ela é
  `ItemEventKind::Expression { n, expression, value, until }` em tiques, e o renderer converte a rampa em amostras
  pelo mapa de andamentos: uma curva de bend é um evento por segmento, exata na amostra, e não um por amostra.
- **Pan da nota** (F-NOT-010): um evento no início da nota, se o pan não for 0; a lei é a do `InstrumentTrack` do
  legado (balanço: esquerda = 1 se p ≤ 0, senão 1 − p; direita = 1 se p ≥ 0, senão 1 + p).
- **Bend da nota** (P-24): pontos `{at, semitones}` em tiques desde o início da nota, lineares entre si; antes do
  primeiro ponto vale o valor dele; o que passa do fim da nota não é enviado. A afinação soma à altura antes do
  bend range e do glide do instrumento, e vale para todas as notas que a nota base toca (acordes, arpejo).
- **Loop do item** (`loop`, tiques): as notas que começam antes de `loop` se repetem a cada `loop` até o fim do item;
  as notas são cortadas no fim do item; cada repetição tem números de nota próprios (U25).
- **Swing** (`swing` 0..1, `step` em tiques, padrão 240): as notas que começam na segunda metade de cada par de
  passos (a posição no item, módulo `2·step`, a partir de `step`) atrasam `swing · step / 2` tiques (a 1, o
  contratempo cai em 3/4 do par, como o swing de 75% das baterias eletrônicas). O legado não tinha swing: a regra é
  esta, nova.
- **Campos ausentes mantêm o sentido antigo**: sem `pan`, sem `bend`, `loop` igual ao comprimento, `swing` 0.


## Os instrumentos do legado como plugins (20/11)

- **Um pacote, `lm-plugins-instruments`, um módulo e um `static Descriptor` por instrumento**, como os efeitos: cada um
  um `Synth` dentro das funções de instrumento do `lm-plugin`, com o seu id, esquema e fixtures. O A/B é por composição,
  como o do TripleOscillator: o `lm-dsp` compara o DSP com o legado; o pacote confere que o plugin o conduz como o
  `playNoteImpl` do legado.
- **Expressões de nota no `Instrument`** (o autor do `lm-plugin` pediu que eu as fizesse): guardadas por nota-base, lidas
  por cada voz a cada peça; um passo de arpejo começado depois de uma afinação já soa afinado; uma nota solta guarda os
  últimos valores. A afinação vale por peça (como o glide), o pan por amostra.
- **`Synth::finished`**: um synth diz que a voz acabou sozinha (o Kicker depois do comprimento, uma amostra tocada até o
  fim), e a voz é liberada sem esperar o note-off.
- **Kicker:** a nota se solta na amostra em que o comprimento acaba (o legado: no período seguinte); a fase da varredura
  volta a cada período (o legado a deixava crescer em precisão simples e a afinação mudava com a taxa: 2 cents a 96 kHz);
  os três casos `kicker-*` do A/B do `lm-dsp` ficam em `INTENTIONAL` com esse motivo (−64 dBFS meio segundo depois).
  Sem sonda de afinação no contrato (uma nota dura no máximo 5 s); a afinação em 44,1/48/96/192 kHz tem teste próprio.
- **Vozes que não alocam:** as vozes do BitInvader e do Organic no `lm-dsp` ganharam `restart` (uma nota começa na
  voz já feita) e um `Clone` cujo `clone_from` copia para o espaço que a voz já tem (o crossfade de uma troca copia vozes
  no áudio). O `render` do Organic montava um `Vec` de parâmetros a cada chamada: agora um array.
- **LB302 é um processador próprio, não um `Synth`:** a TB-303 é um fluxo só (o slide e o dead dependem da nota
  anterior), o que as vozes do `Instrument` não guardam; o autor do `lm-plugin` aceitou um processador próprio para
  ela. Não tem as funções de instrumento, como o legado não dava envelope e filtro a instrumentos de fluxo único;
  acordes e arpejo ficam de fora (de um fluxo monofônico só soaria a nota mais nova). `Lb302::set_frequency` no
  `lm-dsp` move a afinação da nota sem recomeçá-la (o legado lia a frequência da nota a cada período).

- **Vibed sem alocar no áudio:** as linhas de atraso de cada corda são feitas no build, com espaço para a nota mais
  grave (tecla 0 no menor `pitch`) com o harmônico e o comprimento dela; uma nota recomeça a corda nesse espaço. Por
  isso `harmonic` e `length` são campos (no legado eram botões automatizáveis): uma corda mais longa do que as vozes
  do instrumento velho guardam não se adota (`Synth::can_adopt`, novo no `lm-plugin`) e o velho toca a cauda (U32).
  Uma afinação abaixo da tecla 0 toca a corda do tamanho do espaço.
- **As cordas do Vibed correm no próprio relógio:** 2 × comprimento × 44,1 kHz em qualquer taxa do projeto, lendo uma
  amostra de saída a cada `taxa interna / taxa do projeto` amostras internas (entre duas, interpoladas, quando não é
  inteiro). A 44,1 kHz é exatamente a corda do legado (o A/B do `lm-dsp` continua bit a bit); o legado corria a um
  múltiplo da taxa do projeto (menos acima de 88,2 kHz), e o decaimento e o timbre mudavam com a taxa (1,1 dB a
  192 kHz). Sem sonda de afinação: a altura é a do comprimento inteiro em amostras, como no legado.
- **Monstro:** os 14 botões de tempo (ataque e taxa dos LFOs, os cinco tempos dos envelopes) têm cada um um campo
  `<botão>Sync` (`off`, `2/1` … `1/32`), como o `TempoSyncKnob` do legado (sem o "personalizado"): sincronizado, o
  botão vale a nota no andamento da música, no máximo o fim do botão. Para isso o `Synth` ganhou `set_tempo` (chamado
  antes da primeira peça e a cada mudança de andamento). A voz se solta pelos próprios envelopes, então o `Synth`
  ganhou `RELEASE_FADE` (falso no Monstro): o `Instrument` só segura a voz pelo release maior, sem a rampa do
  `applyRelease`, como o legado. As formas de onda e a modulação Osc 2→3 são campos `Choice` (U30).
- **AudioFileProcessor: os conversores "none" e "linear" do libsamplerate portados para Rust**, amostra por amostra
  (testados contra a biblioteca): o crossfade de uma troca toca uma cópia de cada voz, e o libsamplerate só copia um
  estado alocando outro (`src_clone`). Numa entrada de um ou dois quadros, o linear da biblioteca lê a memória antes
  da entrada; o porte usa o último quadro da chamada anterior (o mesmo quadro quando a entrada é contígua, como no
  buffer do player). O sinc continua o da biblioteca, e uma voz em sinc não se copia: o `Synth` ganhou
  `copies_voices`; um instrumento que toca sinc não é adotado por um mudado (o velho toca a cauda, U32); o mesmo
  estado ainda é adotado. A interpolação de uma nota é escolhida quando ela começa, como no legado. O empurrão entre
  os botões de ponto ao arrastar é gesto da interface; o plugin só ordena os pontos (`pointChanged`).
- **Os duty cycles do Nes são campos `Choice`** (`12.5`, `25`, `50`, `75`), não números escalonados: são opções com
  nome (U30), mesmo que o grupo de botões do legado fosse automatizável.

## As telas do agente D no `lm-gui`: o que muda em relação ao legado (20/11)

Regra do usuário (20/11): fora dos designs aprovados (2, 3, 5, 8), as telas seguem os diálogos do legado, de forma
conservadora; só entra o que o modelo novo pede.
- **Configurações:** seguem o design 08 (aprovado). A página MIDI segue a I-21.1 (porta por cliente + porta, apelido,
  papel, entrada e saída, latências, conectada ou não; perfis de controlador da I-21.3), no lugar do "sistema MIDI" e
  do "dispositivo para os raw" do legado — cada porta diz o próprio backend.
- **Exportar:** o assistente de 4 páginas do legado (o quê, formatos, processamento, onde), com os mesmos controles.
  Diferenças: no trecho, "Rápido" (padrão, 10 s de aquecimento) ou "Exato", com o custo (U37, V18); "uma faixa" com as
  outras mudas (F-PRJ-023) e os stems por faixa ou por canal (P-40) na primeira página, no lugar do diálogo à parte
  "Exportar as faixas"; o aviso dos plugins de terceiros (H18) na última; o progresso e o relatório na mesma janela.
  "Entre marcadores" fica para quando os marcadores entrarem nos esquemas do `lm-doc`.
- **Projeto:** no lugar dos diálogos de arquivo do sistema, um diálogo pequeno com os recentes e o caminho (o projeto é
  uma pasta, e no modo cliente os arquivos são do servidor, I-15); "Salvar como" com os botões − + de versão do
  "salvar como nova versão" (F-PRJ-008). O estado do projeto (só leitura H10/H11, takes interrompidos U5 com
  "Recuperar / Descartar" por take, modo seguro U21) fica no mesmo diálogo, junto de três comandos que não têm outro
  lugar sem barra de menus: exportar como arquivo único, salvar como modelo, limpar os arquivos sem uso (H4, P-13).
- **Projeto novo:** um diálogo curto (nome, pasta, taxa, modelo), porque a taxa é fixa por projeto (item 8) e o
  projeto é uma pasta; o legado só abria o modelo padrão.
- **Sobre:** o diálogo do legado (versão e ambiente; Sobre, Autores, Envolvidos, Tradução, Licença, com o AUTHORS e o
  CONTRIBUTORS do legado), mais o botão "Copiar" do ambiente e a aba "Manual" (P-34).
- **Avisos e paleta:** não existem no legado (I-14, item 6); ficaram enxutos: a doca de avisos com as atividades, a
  paleta com busca, grupo e teclas.
- **Foco:** fechar um modal (Esc) devolve as teclas às páginas, e o workspace retoma o foco quando o campo de texto que
  o tinha some (sem isso, as teclas globais paravam depois de fechar a paleta).
## Do Amp & IR (20/11)

- **Dois pacotes:** `lm-plugins-ampandir` (o plugin, o convolver e a FFT em Rust seguro) e `lm-sys-nam` (o Neural
  Amp Modeler embrulhado). Motivo: o convolver é código do próprio legado e pequeno, então foi portado; o NAM é de
  terceiros (MIT), grande e apoiado no Eigen, então fica em C++ atrás de uma ficha I-17.
- **A FFT é escrita no pacote** (radix-2 de meia largura e a separação real, fatores em dupla precisão), sem crate
  nova: o convolver só precisa de 512 pontos, e o A/B contra a FFTW do legado fica entre −130 e −138 dBFS.
- **O NAM, o Eigen e o nlohmann/json vêm copiados em `lm-sys-nam/vendor`** (o NAM na revisão do legado, 0b3d3c9;
  do Eigen só os 230 cabeçalhos que o NAM inclui em arm64 e x86-64, ~4 MB, MPL-2.0; o json.hpp 3.12.0, MIT), e não
  dos pacotes do sistema, contra a preferência pelo sistema. Motivos: o NAM fixa um Eigen (3.5 de desenvolvimento)
  diferente do Ubuntu 24.04 (3.4.0) e do Homebrew (5.0.1) — três Eigens são três núcleos de multiplicação de
  matrizes, e o mesmo modelo soaria um pouco diferente em cada máquina; com a cópia, o código é o que o legado
  rodava, e o A/B dos amps dá igual bit a bit; o ThinkPad não tem os pacotes nem `sudo`. Nada novo em
  `ci/system-deps` por isso. Compilado sempre com `-O2 -DNDEBUG` (como o legado) e o arquivo estático inteiro
  (`+whole-archive`): as arquiteturas do NAM se registram por objetos estáticos que nada cita pelo nome.
- **Exceções do C++ nunca atravessam:** o calço pega todas; carregar ou preparar um modelo que lança vira o erro
  `nam.load`/`nam.reset` com a mensagem do NAM; um `process` que lançasse daria silêncio.
- **O modelo é um recurso de tipo `nam`, carregado como bytes** (o JSON do `.nam` inteiro), não um caminho: o
  projeto leva o modelo com ele e o plugin não lê arquivos (C9). O IR é um recurso `audio`, decodificado na taxa
  do projeto pelo `lm-io` (`Quality::Best`, o mesmo conversor que o legado usava no IR: o caso `ir-resampled` do
  A/B dá −131,7 dBFS). **Pendente fora daqui:** a sessão hoje só pede recursos `audio` (`request_all_resources`);
  os de tipo `nam` precisam ser carregados como `Loaded::Bytes` pelo `lm-io`/`lm-session`.
- **Um modelo de teste para a suíte do contrato:** o `test_loaded` do `lm-plugin` dava 4 bytes a todo recurso que não
  fosse áudio, e um `.nam` de 4 bytes não constrói. Mudança pequena no `lm-plugin/src/testing/material.rs`: um
  pacote pode guardar o conteúdo de teste de um tipo em `tests/data/resources/<tipo>` (aqui, o LSTM de exemplo do
  NAM, 2 KB); sem o arquivo, nada muda para os outros plugins.
- **A reamostragem do amp é a do legado:** `lm-sys-samplerate` em qualidade média, a mesma conversão em laço, a
  mesma almofada de 256 quadros e a latência medida do mesmo jeito (um impulso por conversores iguais, pico refinado
  por parábola). O modelo é preparado para 4096 quadros como no legado (o aquecimento do NAM depende disso), e
  recebe peças de até 32.
- **Peças na grade de 32 do relógio do motor, cortadas nos eventos dos ganhos:** o áudio é o mesmo para qualquer
  bloco (V3), com o amp reamostrado e o IR ligados (`tests/behaviour.rs`). O legado aplicava os ganhos por período;
  aqui no quadro do evento (no A/B a mudança cai num começo de período e dá igual).
- **Só o sinal molhado:** o W/D do legado é o seco/molhado do host, que atrasa o seco pela latência informada (o
  bloco do IR, 256, mais a reamostragem do amp quando o modelo roda noutra taxa).
- **Canais:** um amp e um par de convolvers por par de canais do canal (um último canal ímpar é par de si mesmo); o
  legado era sempre estéreo.
- **Trocar o IR ou o modelo tocando:** o processador novo assume o som do velho; com os mesmos recursos segue bit a
  bit, com outros a cadeia velha toca por 10 ms em crossfade com a nova (sem estalo). A cadeia velha fica guardada
  até o processador voltar à thread principal: nada é liberado no áudio.
- **Pânico (`AllSoundOff`):** zera os convolvers e a FIFO da reamostragem; a memória interna do modelo NAM não tem
  como ser zerada sem alocar e some sozinha em milissegundos.
- **IR vazio ou mudo é erro do build** (`plugin.ampandir.ir_empty`, `plugin.ampandir.ir_silent`), como o legado
  recusava o arquivo.
- **A/B contra o `.so` original** (`lm-plugins-ampandir/ab`: o `libampandir.so` do legado, sem recompilar, com o IR
  e o modelo carregados pelas configurações dele, a 44,1 kHz em períodos de 256): ganhos, amp na taxa do motor, amp
  reamostrado — iguais bit a bit; IR mono, estéreo, reamostrado e amp + IR — de −128,9 a −138,5 dBFS (a FFT).

## Chips e amplificadores (20/11)

Os instrumentos de chip do legado (Sid, FreeBoy, OpulenZ) e, em seções próprias, o Guitarix e o Amp & IR.

- **Portar ou embrulhar, plugin por plugin** (pedido: portar para Rust seguro o que é do legado ou pequeno o
  bastante para portar fielmente; embrulhar o resto num `lm-sys-*` com ficha):
  - **Sid: embrulhado (`lm-sys-resid`).** O reSID 1.0 tem ~5 mil linhas de C++ de terceiros (Dag Lem, GPL-2.0 ou
    posterior), com um filtro do 6581 em ponto fixo a partir de tabelas feitas em `double` na partida; uma
    tradução faria o som depender de cada arredondamento traduzido. As fontes que o legado compilava ficam em
    `crates/lm-sys-resid/vendor/resid` (com o `siddefs.h` configurado e as tabelas de onda que o `samp2src.pl`
    gerava) e o `build.rs` compila com as flags do legado (GNU C++20, `-O2`, a contração de ponto flutuante do
    compilador): o A/B dá bit a bit.
  - **FreeBoy e OpulenZ: portados para Rust** dentro dos crates dos plugins (o APU do Game Boy do game-music-emu e o
    OPL2 do MAME/adplug são pequenos e de aritmética inteira): ver abaixo.
- **`cc` nas dependências do workspace** para os `build.rs` que compilam C/C++ incluído no repositório (reSID,
  Guitarix, Neural Amp Modeler), com `parallel`. Já estava no `Cargo.lock` por outras crates; nenhuma crate nova.
  O pedido de dependência nova à sessão de design fica registrado aqui.
- **`Synth::release` no `lm-plugin`** (correção fora da pasta, aditiva, padrão sem efeito): o sintetizador fica
  sabendo quando a voz é solta, no começo da peça em que o release começa (depois do `start` se a nota começa na
  mesma peça). O SID precisa: o release do chip é o gate do envelope dele. A voz fantasma do crossfade também é
  solta.
- **Sid, como toca uma nota:** um chip novo por nota (cópia de um chip feito no build, sem alocar), os 25
  registradores calculados como o `playNoteImpl` calculava e escritos na ordem e no tempo do `sid_fillbuffer` (9
  ciclos depois de cada escrita, 4 antes de cada registrador de controle). O legado escrevia tudo a cada período do
  motor dele; aqui a cada **256 quadros da nota** (o período padrão do legado, contado do começo da nota: a nota soa
  igual em qualquer tamanho de bloco e em qualquer ponto da linha do tempo) e na hora em que algum registrador muda
  (botão, altura, gate: o legado esperava o período seguinte). Reescrever não é inócuo no chip: a escrita no
  registrador de controle recalcula a saída da onda (e anda o pipeline de meio ciclo do 8580), como um player de C64
  escrevendo a cada quadro; por isso o tique fica.
- **Diferenças intencionais do Sid:**
  - o legado chamava `set_chip_model` a cada período, e o reSID zera os integradores do filtro nessa chamada: o som
    filtrado recomeçava do zero a cada período (um zumbido na frequência do período). Aqui o modelo só é aplicado
    quando muda;
  - o legado pedia `relTime[release]` **quadros** de release (a tabela é em milissegundos: a 48 kHz o release de
    300 ms do SID era cortado em 6 ms; o upstream convertia, o `desiredReleaseTimeMs` do fork trocou a unidade).
    Aqui o release dura os milissegundos;
  - uma sequência de escritas que não cabe numa peça (a 192 kHz, 32 amostras são 164 ciclos e a sequência leva 237)
    continua na peça seguinte, ciclo a ciclo; o legado perdia as escritas depois da última amostra do período, e no
    8580 (cuja escrita só pousa no ciclo seguinte) a voz 3 nem chegava ao chip num período curto.
- **A/B do Sid** (`crates/lm-plugins-sid/ab`): o harness carrega o `libsid.so` do legado numa faixa de instrumento
  e toca as notas pelo `NotePlayHandle` (o `playNoteImpl` do plugin, o sound shaping e o volume da nota), com os
  ajustes pelo `restoreState` do próprio plugin. Pelo plugin, em blocos de 256, 32 e 1024: **bit a bit** do quadro
  128 até o fim das notas (o fade-in é das funções de instrumento, que o calculam por peça de 32 quadros; o legado,
  por período, como no A/B do TripleOscillator), e nos casos filtrados no primeiro período (por causa do filtro
  zerado acima). Um segundo teste dirige o chip exatamente como o laço do legado (modelo a cada período, registradores
  pelo `SidSynth::registers` do plugin, a sequência do `sid_fillbuffer`) e dá bit a bit em todas as notas inteiras,
  as filtradas também: as únicas diferenças do plugin são as de cima. O harness serve aos três instrumentos de chip
  (`build-references.sh <pasta ab do crate>`).
- **FreeBoy: o `Gb_Apu` e o `Blip_Buffer` do game-music-emu portados para Rust** (`crates/lm-plugins-freeboy/src/apu.rs`
  e `blip.rs`, ~1 mil linhas, LGPL-2.1 ou posterior, aritmética inteira com estouro que dá a volta como no C), com
  o `Stereo_Buffer` e o laço do `GbApuWrapper`: quadros de 70224 ciclos feitos quando o anterior acaba, relógio de
  CPU falso de 4 ciclos por escrita. O núcleo do kernel (seno janelado) com o `libm`; o A/B mostra que dá o mesmo.
  As escritas que o legado fazia a cada período vêm a cada 256 quadros da nota e quando mudam (como no Sid). O
  envelope dos canais 1, 2 e 4 só é escrito no começo da nota, como no legado. **A/B bit a bit** em 9 casos
  (quadrados, sweep, onda desenhada, ruído de 7 bits, saídas separadas com treble e bass, acorde, mudança de botões,
  nota fora da faixa tonal), em blocos de 256, 32 e 1024, do quadro 128 até o fim das notas. As chaves SO1/SO2
  trocadas na gravação do legado viram `ch<n>ToSo1`/`ch<n>ToSo2` pelo que fazem. A sonda de afinação do contrato
  usa o padrão sem o sweep do canal 1 (que sobe a nota) e a frequência que o registrador dá (A4 = 131072/297 Hz).
- **OpulenZ: o YM3812 do `fmopl.c` (adplug/MAME 0.37a, LGPL-2.1 ou posterior) portado para Rust**
  (`crates/lm-plugins-opulenz/src/opl.rs`), tabelas feitas como no original em `double`, contadores de fase sem sinal
  com volta. Não portados porque o OpulenZ nunca chega a eles: o modo de ritmo, os timers/IRQ e o ADPCM do Y8950.
  O instrumento é um fluxo só (como o legado, single-streamed e MIDI): nove vozes na ordem do LRU do legado, pitch
  bend com a faixa do RPN 0, pressão de tecla, expressão de afinação por nota. Sem as funções de instrumento (o legado
  também não as dava a um instrumento de fluxo único). **A velocidade** segue a lei proporcional do legado com a
  velocidade base padrão da faixa (63): por isso `Velocity::Timbre` (degraus de 0,75 dB).
- **Diferenças intencionais do OpulenZ:** uma mudança de botão chega ao chip na amostra (o legado: no período
  seguinte); o note-off solta só a sua nota (o legado soltava todas as vozes da mesma tecla); a profundidade de
  tremolo/vibrato muda de fato (o `fmopl.c` guardava as tabelas da última chip renderizada em globais e só as relia ao
  renderizar outra chip); todo projeto usa a lei de velocidade proporcional; trocar a forma de onda (um campo `Choice`,
  que reconstrói) faz crossfade de 10 ms entre a chip velha e a nova, como as funções de instrumento; a nota de
  duração zero (note-off antes do note-on na mesma amostra) toca e é solta na hora. O patch GM sem tela e sem
  gravação do legado não foi portado; a leitura de `.sbi` é da janela (pendente com o painel).
- **A/B do OpulenZ:** o harness dos chips toca as notas por `NotePlayHandle` (o MIDI que elas mandam) e renderiza pelo
  `InstrumentPlayHandle`, avançando o contador de período dos modelos e chamando o `preparePeriod` do instrumento
  como o motor faz. 7 casos (padrão, aditivo com formas de onda, realimentação e escalas, LFOs com KSR e envelope
  percussivo, acorde com 5 notas, volume e velocidade, mudança de botões): abaixo de −138 dBFS em todos (o resto é o
  arredondamento do `expf` do ganho), blocos de 256, 32 e 1024.
- **`TestHints::tail` no `lm-plugin`** (correção fora da pasta): quanto tempo uma nota do estado padrão pode ainda
  soar depois do note-off; o teste de notas curtas espera esse tempo. O release do OPL no padrão do OpulenZ (taxa 3 da
  portadora) leva 9,8 s até o silêncio, como no legado: 10 s declarados. O padrão continua 3,4 s.
- **As referências dos chips sem contração de ponto flutuante:** o `build-references.sh` reconstrói `libsid`,
  `libfreeboy` e `libopulenz` (e as bibliotecas `gme` e `adplug`) com `-ffp-contract=off`, e o `lm-sys-resid` compila
  assim também: as tabelas feitas em `double` dão o mesmo em x86-64 e arm64, em qualquer compilador. As referências
  não mudaram (o clang não contraía nada nelas).
- **A tabela do VCR do filtro 6581 vem pronta** (`vendor/resid/vcr_n_ids_term.h`, gerada por
  `tools/dump-vcr-table.sh` a partir do `filter.cc` intacto do legado, no macOS): ela é a única do reSID feita com
  `exp` e `log1p`, que cada biblioteca C arredonda de um jeito, e o A/B do Sid falhava no Linux (x86-64 e aarch64)
  contra as referências feitas no Mac. O resto das tabelas usa só `+ − × ÷ √`, exatos em IEEE sem contração.
- **Target próprio do worktree** (`/private/tmp/claude-501/wt/chips-amps-target`, apagado no fim): o compartilhado
  misturava artefatos do `lm-plugin` e do `lm-render` de outros worktrees mesmo com o `touch` (erros de campos e
  tipos que não existem na árvore deste), como a seção do `lm-session` já registrou.
- **Importar `.sbi` no OpulenZ é uma ação do núcleo** (`opulenz.import_sbi {node, path | bytes}`, em
  `lm-session/src/plugin_actions.rs`; a leitura em `lm_plugins_opulenz::sbi`, como o `loadFile` do legado): um passo
  de desfazer, roda igual do painel, da paleta e de um cliente remoto. `path` é do computador da sessão (o servidor);
  `bytes` leva o próprio arquivo, para o cliente que o leu onde roda. O legado não exportava `.sbi`: sem exportação.
  O legado renomeava a faixa com o nome do patch; aqui o nome vai no rótulo do desfazer e a faixa fica como está (o
  nome da faixa é do usuário). Sid, FreeBoy e OpulenZ entraram no `internal_plugins` da sessão.

## Do `lm-session`, continuação (20/11)

- **Modo seguro (U21):** um vigia lê o `Watch` do renderer a cada 250 ms; com os blocos parados por 2 s e o
  dispositivo ligado, grava o nó em que a callback está em `backup/suspeito.json`. Abrir o projeto com o arquivo põe
  aquele plugin em `bypass` (um passo desfazível), com aviso e `status().safeMode`; o arquivo é apagado ao usar. O
  `bypass` de um instrumento ou insert tira o nó do programa (o compilador nem o constrói nem o chama).
- **A mesa do legado no motor:** `trim`, `mono` (com crossfade) antes do rack e de todo insert; `phaseInvert` e `swap`
  depois do tap pré-fader e antes do fader; os PRE do master antes do estágio de ganho dele (o master do legado não
  tem botão de drive: o `tube.drive` do master não é usado). São estágios do rack (`Input`, `Polarity`), construídos
  nos dois modos da mesa quando o canal tem os campos.
- **Monitoração** (`preferences().monitor`): no último trecho do master, só no programa ao vivo (o `Environment` da
  exportação não a tem); o LEVEL anda pela vaga, os botões recompilam.
- **Medidores da mesa** (SAT, CLIP como contador, redução do gate, do compressor e do barramento, correlação,
  goniômetro de 64 pontos) vão pelo quadro de display de cada nó do rack e são juntados por canal em
  `LocalPort::display(m:N)` (tipo `"rack"`, layout em `lm_session::rack::display`).
- **Wet/dry do insert:** campo opcional `wet` do nó de plugin (sem padrão, então os padrões congelados não mudam e a
  versão fica), parâmetro de chave `insert.wet`; o compilador soma a saída do insert com o sinal seco por `1 − wet`
  (`HostParam::complement`, numa segunda instância da vaga), e a PDC atrasa o seco pela latência do insert (teste de
  impulso com um insert de latência 100: um só impulso somado). Pela U29 o nó de plugin foi à **versão 2** (o `wet`
  muda o significado; a versão 1 não tem o campo e sobe sem mudança) e os padrões foram reabençoados.
- **O target compartilhado não é seguro mesmo com o `touch`:** dois worktrees compilando o mesmo pacote ao mesmo
  tempo trocam o artefato um do outro (um teste do `lm-compile` falhou com o `lm-doc` de outro worktree). Este
  worktree passou a usar um target próprio, apagado no fim.

## Da implementação de MIDI e automação (I-21, 20/11)

- **A lógica do mapa num pacote próprio, `lm-midimap`, sem o `lm-state`** (a forma no documento atrás da feature
  `doc`) — o `lm-render` usa a tabela compilada e não pode depender da árvore (R1).
- **O mapa vai ao áudio numa `Install`**, sozinho (`Install::map_only`) ou com o programa, e o estado de cada mapa
  (pickup, toggle, metades de 14 bits, NRPN) passa para a tabela nova por uma fusão por id, em O(n), sem alocar —
  seguir a seleção e trocar de banco recompilam só o mapa. **O `compile_map` roda na thread principal** (é barato: não
  monta programa) sobre a tabela da última instalação; o compilador continua o único que dá vagas.
- **O retorno é feito na `Session`**, a partir do quadro de telemetria (que agora traz o valor de toda vaga ligada,
  não só das automatizadas) e do documento, com o limite de 10 ms e a janela de 50 ms em nanossegundos — assim um só
  caminho serve parâmetros, campos, a mão, a automação e o desfazer. O anel de saída do `Renderer` fica para a saída
  MIDI das faixas (P-38).
- **Campos e ações aplicam os botões contra o valor do documento** (toggle, momentâneo, gatilho, passos de encoder);
  no áudio, esses mapas são `Jump`. Um botão numa escolha anda pelas opções; um knob escolhe pela posição.
- **Uma ação que o registro do núcleo não conhece é da interface** e vai ao `MidiUiAction`.
- **Parâmetro sem campo no documento** (o nível do monitor) é movido pelo MIDI, mas o fim do gesto não escreve nada.
- **Um `f32` do motor vira o `f64` do documento pelo decimal mais curto** (0,05f32 é 0,05), e é limitado à faixa do
  esquema: sem isso, o topo de uma faixa era recusado como acima dela.
- **A lei dos ganhos (volume, envio) é a do fader do legado** (decisão do coordenador, 20/11; `Fader.cpp`,
  `computeScaledRatio`): curso `((dB + 120) / (topo + 120))³`, abaixo de −120 dB é silêncio; com máximo 4 (+12 dB), 0 dB
  fica em ~75 % do curso (−6 dB em 64 %, −60 dB em 9 %). Uma lei só, no `ParamRange` do núcleo, serve o MIDI, o MCU, a
  automação e os faders da interface, que a pedem pela porta (`param_range`): a mesa, os controladores e o legado
  concordam. Substitui a primeira escolha (−60 dB até o topo em dB, 0 dB em ~83 %). Panorâmica linear; parâmetros log
  em escala log.
- **A quantidade de um envio tem vaga** (é parâmetro no `param_key`), para a mão e o MIDI a moverem sem compilar.
- **Bancos:** `follow` ganhou `trackBank` e `channelBank` (a superfície trabalha num banco que só os botões de banco
  movem); `track`/`channel`/`plugin` seguem a seleção do documento da interface (`selection: {track, channel,
  plugin}`, campo novo do esquema da interface).
- **Perfis:** um JSON com mapas, um molde de faixa repetido por faixa do banco (`stripNumber`, `stripChannel`,
  `stripTouch`, `stripParam`) e o tamanho do banco; os embutidos são `generic` (editável: uma cópia na pasta das
  preferências vence) e `mcu`. O módulo MCU só faz visor, medidores e o mostrador de banco.
- **Aprender recusa** alvos que são ações `midi.*` (o controle mexeria na própria fonte).
- **Gravação de automação:** a tolerância é medida na curva como ela toca (reta na unidade do parâmetro), em dB para
  ganhos (0,1 dB), cents para log (1 cent), 0,5 % da faixa no resto, degraus exatos nos discretos; a passada substitui o
  trecho com saltos nas pontas, para a curva fora dela não mudar. Sem pista e com o botão global em Ler/por pista, nada
  é gravado; o botão global em Toque/Trava/Escrever cria a pista.
- **"A mão venceu"** é marcado quando um controle move um parâmetro com pista em Ler ou Toque, e fica até o rearme.
- **`automation.scale`** trabalha no espaço normalizado do parâmetro (o que o editor manda).
- **Gravação MIDI:** as mensagens vão às faixas armadas com instrumento cujo `midiInput` aceita a porta e o canal
  (sem portas = todas); `followSelection` grava só na faixa selecionada. Em camadas, cada passada é um item e as
  anteriores ficam mudas; em substituir, as notas que começam sob o take saem. O `midilog` é texto de uma linha por
  mensagem (resiste a um corte no meio da linha) e é apagado quando o take entra na árvore.
- **As faixas de controle do item** (`item.controls`) são um campo opcional novo: o item continua na versão 2.
- **Testes da `Session` com todos os plugins em depuração esperam por condição, não por tempo:** o render em
  depuração com todos os plugins é mais lento que o tempo real, então o relógio do dispositivo atrasa e uma mensagem
  carimbada com o relógio do sistema espera a sua hora (a regra do atraso fixo funcionando).
- **Gravação por passos e gravação no item aberto ficam no núcleo** (F-PAS-001, F-NOT-018, F-REC-029; combinado com
  o agente B): o cursor mora no documento da interface (`stepRecord: {item, at, step}`), o item de destino em
  `recordInto`; as teclas vêm do monitor de atividade (`midi_input`), filtradas pela entrada MIDI da faixa do item;
  um acorde entra quando a última tecla sobe, num passo de desfazer, e o item cresce para caber. As ações:
  `notes.step_record`, `step_keys` (o teclado do computador), `step_rest`, `step_back`, `step_length` (→/←),
  `step_stop`, `notes.record_into`. Gravando no item aberto, o take se funde nele (camadas viram soma); o laço próprio
  do item (P-26) continua na interface até ser um modo do transporte.
- **O gesto MIDI é um gesto do documento** (I-3.7; o T4 já cita "um controlador MIDI noutro"), e não uma transação no
  fim (20/11, a lacuna do S20): a árvore segue o controle a cada `pump` (um `gesture_update` com o último valor, sem
  passo no histórico), então um programa compilado no meio — um processador reconstruído, um nó novo — nasce do valor
  movido; o `commit` no fim do gesto é o único passo de desfazer, e o diário o recebe a cada 1 s (T11). Enquanto o MIDI
  segura uma vaga, a `Session` não a reescreve a partir da árvore (o áudio tem valores mais novos que ela). Uma
  transação alheia no meio entra como passo próprio com o valor do gesto naquele instante, como manda o T4. A outra
  saída — levar o valor vivo da vaga para os valores iniciais do programa novo — deixaria a árvore errada até o fim do
  gesto (um processador construído do estado leria o valor velho) e foi descartada.

## O host de plugins de terceiros fora do processo (I-9.6, 20/11)

- **Quem fica fora:** só plugins de terceiros (LV2, CLAP, VST3, LADSPA, VST2 pelo wine). Os nossos continuam no
  caminho de render, no processo do núcleo. O núcleo tem um `Proxy` (um `Processor` comum) e o plugin roda no
  `lm-plugin-host`, um binário separado. O núcleo não linka nenhuma biblioteca de formato; só o helper linka.
- **Um helper por instância** (por enquanto, e é o padrão): um plugin que cai ou trava leva junto só ele mesmo, e
  o "recriar" é só reabrir aquele processo. Agrupar (um helper por fabricante ou por formato, para economizar
  memória) cabe no mesmo protocolo, porque cada bloco compartilhado é de um plugin. Vai ser uma preferência, com o
  padrão em um por instância. Ainda não está implementado.
- **Memória compartilhada POSIX** (`shm_open`, mapeada pelos dois lados e apagada com `unlink` logo depois do
  `ready`, para não sobrar nada se os dois morrerem). O bloco tem um cabeçalho de palavras atômicas, áudio de
  entrada, saída e sidechain (f32), até 4096 eventos de 6 palavras na entrada e 256 de parâmetro na saída.
  `lm-sys-ipc` é o único `unsafe`.
- **Aperto de mão por bloco sem trava:**
  - O núcleo escreve a entrada, incrementa `REQUEST` e acorda o helper.
  - O helper processa, copia `REQUEST` em `DONE` e acorda o núcleo.
  - O núcleo gira 64 vezes e depois espera pela palavra `DONE` com prazo.
  - Para acordar: **futex** compartilhado (não `PRIVATE`) no Linux e `__ulock_wait`/`__ulock_wake` com
    `UL_COMPARE_AND_WAIT_SHARED` no macOS.
  - Por que não eventfd, pipe ou semáforo: cada um custa um descritor e uma chamada que pode bloquear sem prazo do
    lado de quem escreve (pipe cheio). O futex e o ulock esperam numa palavra que já está no bloco, com prazo, e não
    fazem nada quando ninguém espera. O semáforo POSIX com nome não tem espera com prazo no macOS.
  - Nada aloca e nada trava do lado do núcleo; o teste roda sob a armadilha do `assert_no_alloc`.
- **Prazo:**
  - Ao vivo, 60% da duração do bloco. O resto fica para o grafo do núcleo.
  - Um bloco perdido é silêncio só naquele nó e é contado (`Hosted::missed`, para a telemetria). Não há aviso por
    bloco, para não inundar a fila; o aviso vem quando ele trava.
  - Se o helper ainda está ocupado com o bloco anterior, o bloco seguinte também é perdido, sem esperar. Assim um
    plugin lento nunca atrasa o callback mais de um prazo.
  - Na exportação não há prazo (`Deadline::Wait`, S19): ela espera o plugin, para sair igual bit a bit.
- **Travou:** 16 blocos perdidos seguidos = travado. O supervisor mata o processo, o nó fica em silêncio e aparece o
  aviso `plugin_host.hung`.
- **Caiu:** o supervisor vê a saída do processo pelo `try_wait`, marca o nó morto e acorda o `DONE`. O nó fica em
  silêncio e aparece o aviso `plugin_host.crashed`. O estado salvo por último continua no nó do documento, então
  "recriar" é construir o nó de novo, que sobe outro helper com aquele estado.
- **Canal de controle:** linhas JSON (o JSON do `lm-state`) no stdin/stdout do helper, versão de protocolo 1. As
  mensagens são `hello`, `load`/`ready`, `save`/`state`, `gui`, `quit` e `error`. Ficam fora do caminho de áudio,
  sempre com prazo do lado do núcleo.
- **Estado no nó:** os parâmetros do plugin são params do esquema `plugin:<id>` (mapeáveis por MIDI e
  automatizáveis). O que não é parâmetro vai no campo `chunk` (texto hex).
  - No LV2, o chunk tem os valores das portas de controle seguidos do estado do lilv.
  - Um chunk que não se lê abre o plugin nos padrões, e o `ready` diz `restored: false`. Não é um erro que impede
    abrir o projeto.
  - O id é `<formato>-<16 hex do BLAKE3 da URI>`: estável e sem caracteres que o esquema não aceita.
- **Varredura como `Operation`:** a lista vem só dos metadados (`list`). Cada plugin é sondado (`probe`) num helper
  próprio, com prazo. O que cai ou estoura o prazo entra na lista de bloqueados com o motivo, e os bloqueados não são
  sondados de novo (`skipped`). Um plugin bloqueado não derruba a varredura nem o núcleo.
- **Na sessão** (`lm-session/src/external.rs`, combinado com o agente do `lm-session`):
  - **A varredura é uma `Operation`.** O que ela acha e a lista de bloqueados ficam nas preferências
    (`plugins.scanned`, `plugins.blocked`, `plugins.paths`). Ao abrir, a sessão põe no catálogo os plugins da última
    varredura.
  - **O catálogo só cresce enquanto a sessão roda.** Um plugin que o projeto usa nunca sai: o documento, o compilador
    e o histórico continuam citando o nome dele. O documento segue com o registro maior (`Document::widen`, que
    recusa um registro sem algum tipo ou com versão diferente) e o compilador com o catálogo maior
    (`Compiler::set_catalog`). Nada é reaberto e o histórico fica. Fechar e reabrir o projeto era a alternativa e
    perderia o desfazer.
  - **O helper de cada nó:** a sessão guarda um `Weak<Hosted>` tirado do proxy construído (`Proxy::hosted`). Assim o
    helper vive exatamente enquanto o processador vive.
  - **O `"chunk"` não reconstrói o processador** (`lm_plugin::CHUNK_KEY`, ignorado pelo `same_processor`, como os
    params). Ele chega ao documento numa transação `Origin::Plugin`:
    - quando o plugin avisa (`StateChanged`) ou antes de cada salvar, pedido ao helper numa operação;
    - junto vêm os valores dos parâmetros que o estado carrega.
    Quando o chunk do documento difere do que o plugin tem (desfazer, preset), a sessão o entrega ao plugin rodando
    (`restore`), sem construir de novo.
  - **Parâmetros mudados na janela do plugin** (`ParamChanged`) viram um gesto, fechado 300 ms depois da última
    mudança (I-9.4).
  - **Caiu ou travou:** aviso com a ação `plugin.recreate`. Ela reconstrói o nó (`Compiler::renew`) a partir do
    estado do documento (o último guardado). O antigo, já em silêncio, toca até o novo ficar pronto.
  - **Na exportação** o proxy espera cada bloco (`Deadline::Wait`).
  - Um estado acima de 64 KB como recurso (U24) ainda não foi feito: o chunk vai inteiro na árvore.
- **CLAP** (`lm-sys-clap`, 20/11):
  - **O ABI foi escrito à mão**, sem bindgen. Os cabeçalhos do CLAP 1.2.6 (MIT) ficam no repositório só para os
    testes: um programa em C compara o tamanho, o alinhamento e os offsets de cada struct com os do Rust, e os
    plugins de teste são escritos em C contra eles. Um erro de transcrição aparece no teste e não num plugin de
    verdade.
  - **Varredura em dois tempos.** Listar um CLAP roda código do plugin (o `init` e a criação, para ler parâmetros e
    portas). Por isso o helper primeiro só nomeia os arquivos (`files`), e cada arquivo é listado num helper próprio
    com prazo. Um arquivo que derruba ou trava a listagem fica bloqueado como `file:<caminho>`. Formatos cuja
    listagem não roda código (LV2, `test`) respondem `no_files` e seguem como antes.
  - **Identidade:** a URI é o id do CLAP. O arquivo onde ele foi achado vai junto (`Scanned::file`,
    `LoadRequest::file`) e é tentado primeiro; se o arquivo mudou de lugar, procura-se nas pastas.
  - **Parâmetros:** a chave é `p<id>` (os ids do CLAP são estáveis entre versões). Os ocultos ficam de fora.
  - **Threads.** A thread principal do helper passou a ser a thread principal do plugin: o canal de controle é lido
    por uma thread própria, e entre as mensagens o plugin ganha a vez a cada 10 ms (`Plugin::idle`: callbacks,
    restarts, novas leituras dos parâmetros). O `thread-check` do CLAP responde pelo papel que cada thread declarou.
  - **Restart** (uma latência nova, portas novas): o lado de áudio para no bloco seguinte (`stop_processing` na
    thread de áudio) e toca silêncio; a thread principal desativa, reativa e lê a latência de novo. A nova latência
    chega ao núcleo pela palavra `LATENCY`, que vira `LatencyChanged` e recompila a PDC. Um estado restaurado ao
    carregar faz o restart antes do `ready`, então a latência informada já é a nova.
  - **Transporte para todos os formatos:** o bloco compartilhado passou a levar PPQ e fórmula de compasso, e o
    `Plugin::process` recebe o transporte. O CLAP recebe batidas, segundos, andamento, tocando/gravando e compasso.
    O LV2 ainda não usa (`time:Position` fica para depois).
  - **Notas:**
    - expressões com o id da nota; uma rampa vira um valor a cada 32 quadros até acabar;
    - o pan vai de −1..1 para 0..1;
    - "soltar todas" é um note-off curinga e "silêncio" é um choke curinga.
  - **Sem alocação** do lado do host no `process` (testado sob a armadilha). No helper, a lista de canais por
    bloco também deixou de alocar (vetores na pilha).
- **Pendente:**
  - Plugin mono numa faixa estéreo processa só o canal 0. Falta rodar duas instâncias em dual mono.
  - Janela do plugin pelo helper (I-22): o protocolo tem `gui`; o LV2 precisa do suil/GTK.
  - Prioridade de tempo real na thread de áudio do helper (pelo `lm-sys-linux`).
  - Os formatos VST3, LADSPA e VST2-wine; janela do CLAP (item 4).

## Do `lm-session`, continuação 2 (20/11)

- **A exportação que diferia 1 em 6 não era corrida de render:** em 50 rodadas da fatia com target privado, 2
  falharam, e nas duas as amostras eram idênticas bit a bit; só os bytes do arquivo diferiam. O libsndfile grava no
  chunk PEAK dos WAV/AIFF de ponto flutuante a hora do relógio (`time(NULL)`), então duas exportações que cruzam um
  segundo diferem em 4 bytes do cabeçalho. Correção no `lm-sys-sndfile`: `SFC_SET_ADD_PEAK_CHUNK` desligado (WAV,
  AIFF); o RF64 ignora esse pedido, então o carimbo é zerado no `close`. Teste: as mesmas amostras dão os mesmos
  bytes 1,1 s depois, em WAV, RF64 e AIFF. Depois da correção: 60 rodadas sem falha, e a fatia também roda com o
  dispositivo em blocos de 32, 64, 128 e 1024 (a exportação não depende do bloco ao vivo).
- **Modo seguro para quedas nativas:** `lm_sys_linux::crash` (em todo Unix; o macOS serve para desenvolver). Os
  tratadores de `SIGSEGV`, `SIGBUS`, `SIGILL` e `SIGABRT` são instalados uma vez; o arquivo (`backup/.suspeito.json.crash`)
  é aberto antes, e a sessão formata antes uma nota por nó de plugin a cada instalação. No tratador: átomos,
  `pthread_self`, `write`, `fsync`, `rename` para `suspeito.json`, e depois o tratador anterior (sanitizadores, o do
  Rust para estouro de pilha) ou a ação padrão, com o sinal levantado de novo: o processo morre como morreria. Só se
  acusa um nó quando a thread que caiu é de render (marcada pelo gancho de threads da sessão, índice do `Watch`):
  uma queda da thread principal não acusa o plugin que por acaso estava tocando. Testes: processo filho com plugin
  que lê o endereço 8 (nulo + deslocamento); o projeto reabre em modo seguro com ele em `bypass`.
- **`lm-headless slice <pasta nova> [--device NOME] [--seconds S] [--input N] [--block N]`:** roda a fatia (no Dummy
  com ruído de ventoinha quando não há `--device`), imprime o relatório em JSON e falha se a exportação depois de
  reabrir não for idêntica.

## Do Mallets (STK) (20/11)

- **O STK portado para Rust, não ligado** (`lm-plugins-stk`, um `static Descriptor`, id `mallets`): as classes que o
  Mallets toca (`ModalBar` sobre `Modal`, `TubeBell` sobre `FM`, `BandedWG`, e `Envelope`, `ADSR`, `SineWave`,
  `FileWvIn`, `FileLoop`, `BiQuad`, `OnePole`, `TwoZero`, `DelayL`, `BowTable`), na mesma ordem de contas e em precisão
  dupla. Motivo: o STK guarda a taxa numa estática global (`Stk::sampleRate`), o que quebra duas instâncias em taxas
  diferentes e pede trava para mudar, e lê as rawwaves de uma pasta quando uma voz é feita (no áudio, no legado,
  quando as vozes prontas acabavam). Sem `lm-sys-stk`, sem pacote de sistema novo, sem configuração "pasta do STK".
- **As rawwaves dentro do pacote** (`rawwaves/`: `marmstk1.raw`, `sinewave.raw`, `fwavblnk.raw`, 3 KB, com a licença do
  STK, de estilo MIT, que permite redistribuir) por `include_bytes!`, decodificadas uma vez no build.
- **Três modelos, três tipos de `Synth`:** a voz de uma nota é feita para o seu modelo (barra, sino, guia de onda), e o
  instrumento toca um modelo por vez. Trocar o preset dentro do modelo é adotado (as notas guardam o seu preset, como
  no legado); trocar de modelo não é (U32: as notas velhas vão para a cauda). Motivo: as guias de onda curvadas precisam
  de linhas de até 4096 amostras por modo, e toda voz de todo modelo com elas custaria dezenas de MB por instância. O
  teste gerado do adopt troca `marimba` por `vibraphone` (mesmo modelo), por isso `preset` não está em `no_adopt`.
- **Velocidade (U28):** `Timbre`; a velocidade `v` é o volume de nota do legado `200 v` (MIDI 127 é a batida mais dura,
  como o legado mapeava), e a saída é a do legado (a do plugin vezes o `volume / 100` da faixa) vezes ½: a regra que faz
  todo instrumento de nível do programa novo, em `v`, soar como o do legado em `200 v`, 6 dB abaixo.
- **Esquema:** os 15 botões do legado como parâmetros com as faixas e padrões dele (`hardness`, `position`,
  `vibratoGain`, `vibratoFreq`, `stickMix`, `modulator`, `crossfade`, `lfoSpeed`, `lfoDepth`, `adsr`, `pressure`,
  `speed`, `bowed` — o "strike" salvo sem botão —, `spread`, `randomness`); `preset` é `Choice` com chaves de texto;
  `levelled` (padrão ligado, como os instrumentos novos do legado) e `oldVersion` (projetos de antes da versão dos
  presets) são campos. O importador de projetos do legado escreve `levelled` desligado quando o atributo falta e
  `oldVersion` com os ajustes que o `loadSettings` fazia sem atributo `version` (não é deste pacote).
- **Aleatoriedade determinística:** cada nota semeia o `fastRand` do legado (`lm_dsp::math::LegacyRand`) com um sorteio
  do `Rng` do nó. Nada mais é aleatório: as classes do STK usadas não chamam `rand()` (o `Noise` não entra).
- **O mesmo som em qualquer taxa (U36)**, sem mudar nada a 44,1 kHz (a taxa padrão do legado, onde a porta é a conta da
  biblioteca): os raios dos polos das barras viram `r^(44100/taxa)`; o golpe (a onda, o envelope, o passa-baixa e o
  ganho da baqueta) é feito a 44,1 kHz e levado à taxa por interpolação de banda limitada (sinc janelado, 32 pontos; o
  STK toca a onda com interpolação linear, cujas imagens se dobram diferente em cada taxa: 3 dB no vibrafone a 96 kHz);
  o ganho dos ressonadores (dois polos sem zeros: o `setResonance` sem normalizar apaga os zeros do construtor) é
  escalado pelo espectro e por `sin(2πf/taxa)`; a divisão de modos acima de Nyquist é a de 44,1 kHz; o diferenciador da
  realimentação do sino e o ataque e o integrador do arco são escalados; as linhas das guias de onda têm, em segundos, o
  comprimento inteiro que o STK lhes dá a 44,1 kHz (uma linha inteira mais um passa-tudo de primeira ordem exato na
  frequência do modo: a interpolação linear amortecia os modos agudos a cada volta). Medido (`tests/rates.rs`): barras
  e sinos a 0,06 dB e 0,16 cent de 44,1 kHz em 48/96/192 kHz; guias dedilhadas a 0,2 dB; arco a 0,3 dB a 48 kHz e até
  3 dB numa nota aguda (A5) a 192 kHz (o atrito do arco é não linear e as harmônicas dele se dobram a 44,1 kHz, no
  próprio legado), sempre a 0,4 cent.
- **Diferenças intencionais do legado:** (1) as atualizações de cada período (vibrato, ADSR e LFO do sino, a altura)
  valem por peça de até 32 amostras; o A/B toca o legado em períodos de 32; (2) a espera do lado direito (`spread`) é em
  amostras a 44,1 kHz, igual em tempo em qualquer taxa; (3) uma guia de onda abaixo da nota mais grave que a primeira
  linha do STK guarda a 44,1 kHz (10,8 Hz) toca nessa nota (o legado não conseguia pôr a linha e tocava no comprimento
  anterior); (4) tudo o que está no item anterior, fora de 44,1 kHz.
- **A/B (U34):** `ab/legacy-harness.cpp` liga o objeto do plugin do legado (o `MalletsSynth` de `Mallets.cpp`) às fontes
  do STK 5.0.1 compiladas sem FMA, como o `MixerTest` é ligado, e reproduz o que o `playNoteImpl` faz em volta dele
  (aleatoriedade, voz, nível, controles por período); 33 casos (os 14 presets, dedilhado e arco, aleatoriedade, spread,
  versão velha, sem nivelamento, botões e altura mudando no meio da nota, notas extremas). Pela nota (a porta do
  `playNoteImpl`, 28 casos bit a bit) e pelo plugin (com as funções de instrumento, depois do fade-in, 25 bit a bit); os
  outros abaixo de −150 dBFS (o `pow` do arco, e volumes de nota que não são exatos em `v = volume / 200`). Contra o `libstk.a` instalado como está
  (com as FMAs do build do Homebrew, `LIBRARY_STK=1`), o pior caso fica em −168 dBFS.
- **Sonda de afinação do contrato:** o arco na barra afinada em Ré♯5. O teste gerado mede um período, e nenhum som do
  instrumento tem um exato (barras e sinos são inarmônicos e decaem; os modos das guias de onda também são inarmônicos
  e, com as linhas inteiras do STK, saem da razão pelo truncamento, o que puxa o período medido até ~2 cents); o arco
  dura, a barra afinada é quase harmônica, e em Ré♯5 os truncamentos se cancelam (0,02 cent nas quatro taxas). A
  afinação de cada preset em cada taxa e 30 min de arco (0,0002 cent) são testados pela frequência do modo fundamental
  em `tests/rates.rs`.
- **Recursos de outros tipos na sessão (U24, C9):** a sessão pede ao `lm-io` pelo tipo (`begin_load_kind`/`finish_kind`,
  operação própria `kind_ops`); o build recebe `Loaded::Bytes` (NAM, IR, wavetable, preset) ou `Loaded::File`
  (SoundFont, .gig, kits), conforme `lm_io::kind_policy`; a exportação lê pela mesma política. Teste com o AmpAndIr:
  um `.nam` importado em `state/` molda o som ao vivo e na exportação; um ausente dá `Missing` ("localizar arquivo").
- **Duas folgas do "assentado" corrigidas:** (1) entre o `submit` e o compilador pegar o trabalho (e entre terminar e
  publicar o resultado) não havia atividade e a sessão se dizia assentada; o `Compiler` ganhou `pending()` (número da
  submissão contra o da última respondida). (2) um build que falha (um plugin que recusa recurso ausente) deixava o
  nó em substituição para sempre; agora fica em `failed_builds` até o documento pedir outro build dele.

## A lei de ganho dos faders (20/11)

- **A decisão:** os faders de volume e de envio usam a lei do fader do legado (`Fader::computeScaledRatio`: um cubo
  sobre −120 dB até o topo, com 0 dB em cerca de 75 % do curso). O núcleo é o dono dessa lei, no `ParamRange`; a
  interface e o mapeamento MIDI a leem de lá.
- **O motivo:** a regra do usuário de ser conservador com o legado. A curva por partes do protótipo era invenção, e
  abaixo de cerca de −12 dB a mesa não bateria com o fader do legado.
