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
- **P-03 LADSPA:** entra na 1.0, fora do processo, como terceiros. Os conjuntos que o legado distribui (calf, caps,
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
- **Um insert em bypass continua no programa, desligado da cadeia**: o processador fica vivo e voltar não espera um
  build.
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

