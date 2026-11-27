# Interfaces entre as peças

Estado: **aprovado pelo usuário em 19/11** ("aprovo tudo por enquanto"). **Nenhuma implementação está autorizada:**
nenhum código em `LinuxMusicBeta/` até o usuário autorizar explicitamente. Substitui `api-esqueleto.md` (o rascunho em
C++, só histórico).

Decisões que este documento segue:
- **O núcleo é em Rust.** **A interface é GPUI**, e só GPUI (decisão do usuário, 19/11; I-15).
- **Interfaces novas, desenhadas do zero** para o modelo seguro. Do legado vem só lógica (drivers, DSP), nunca a
  forma de uma interface.
- **Toda operação que possa passar de 200 ms tem tempo e estado de carregamento** (decisão do usuário, 19/11; I-2).
- A arquitetura e a especificação da base estão em `arquitetura-nova.md`.

Revisões incorporadas, citadas no texto:
- **(R1–R18):** primeira revisão da sessão de implementação, sobre o rascunho em C++;
- **(S1–S22):** segunda revisão da sessão de implementação, sobre a primeira versão deste documento;
- **(D1–D13):** releitura da sessão de design, sobre a mesma versão;
- **(T1–T20):** terceira revisão da sessão de implementação, sobre a segunda versão;
- **(G1–G3):** os três casos de gesto que a regra nova do T4 pedia (quarta passada);
- **(R-a, R-b):** revisão do I-20 (as edições do protocolo como dado; picos em `i16`). Depois, decisão do usuário: o
  protocolo externo é separado da interface Rust, que segue usando código.
- **(H1–H19):** furos do programa no mundo real (disco cheio, usuário fora de ordem, a plataforma), achados pela
  sessão de design, corrigidos pela revisão e aprovados pelo usuário em 19/11 (Parte 5);
- **(U1–U38):** furos de uma sessão real de gravação e do que deu errado no legado, achados pela sessão de
  implementação e aprovados pelo usuário em 19/11 (Parte 6).

- **(C1–C13):** revisão de coerência da sessão de design (Parte 7). Termos definidos em C13.

**Plataforma: Linux com ALSA primeiro, macOS depois** (decisão do usuário, 19/11). JACK/PipeWire vêm depois do ALSA.

**O rack da mesa analógica será portado**, e todo código de efeito vai para o pacote `lm-dsp` (decisão do usuário,
19/11; Parte 3).
Configurações que juntam dispositivos são sempre explícitas (U7).

**Áudio em rede: não há backend próprio** (decisão do usuário, 19/11). **O núcleo é controlável e gerenciável
remotamente** (decisão do usuário, 19/11): I-20.

**Fases.** Cada interface diz quando entra:
- **F1:** a primeira fatia vertical;
- **F2:** depois dela.

O desenho de F2 está aqui para a base nascer com a forma certa, mas ninguém implementa F2 antes de a F1 estar pronta.

---

## Parte 1: o inventário

| # | De → para | Fase | Onde |
|---|---|---|---|
| 1 | Interface ↔ documento (ler, mudar, desfazer, saber o que mudou) | F1 | I-3 |
| 2 | Documento → compilador → áudio (o programa) | F1 | I-5 |
| 3 | Interface → áudio (parâmetro ao vivo) | F1 | I-6 |
| 4 | Interface, MIDI → áudio (transporte, notas) | F1 | I-7 |
| 5 | Áudio → interface (medidores, cursor, avisos) | F1 | I-8 |
| 6 | Áudio → disco (gravação) | F1 | I-11 |
| 7 | Dispositivo → áudio (a callback) | F1 | I-10 |
| 8 | Host ↔ plugin interno (construir, processar, trocar) | F1 | I-9 |
| 9 | Recursos → compilador e interface (carregando, picos) | F1 | I-12 |
| 10 | Operações demoradas → interface (estado de carregamento) | F1 | I-2 |
| 11 | Metrônomo e contagem | F1 | I-5.5 |
| 12 | Transporte: quem sabe onde a música está | F1 | I-7.2 |
| 13 | Núcleo → usuário (avisos e erros) | F1 | I-14 |
| 14 | Núcleo ↔ biblioteca de interface | F1 | I-15 |
| 15 | Montagem e desmontagem da sessão | F1 | I-16 |
| 16 | Diário e autosave | F1 | I-4 |
| 17 | Ações (menus, atalhos, paleta, API externa) | F1 | I-19 |
| 18 | Tempo (tiques ↔ amostras) | F1 | I-18 |
| 19 | Pânico dentro de um plugin | F1 | I-9.3 |
| 20 | Preferências → dispositivos (trocar de placa) | F1 | I-10.3 |
| 21 | Calibração de latência | F1 | I-10.4 |
| 22 | Exportação | F1 | I-13 |
| 23 | Prévia no navegador | F2 | I-7.3 |
| 24 | Plugin → host (latência mudou, erro, estado mudado pelo próprio plugin) | F2 | I-9.4 |
| 25 | Plugin → a própria janela (osciloscópio, espectro) | F2 | I-9.5 |
| 26 | Plugins de terceiros fora do processo | F2 | I-9.6 |
| 27 | Gravação de automação | F2 | I-6.4 |
| 28 | **Modo cliente e controle remoto**: a mesma interface sobre a `RemotePort`; ferramentas externas num subconjunto estável | F2 (a porta é F1) | I-15, I-20 |

---

## Parte 2: as regras de todas as interfaces

1. **A thread é um tipo.** O que só pode rodar na thread principal pertence a um objeto que **não pode sair dela**
   (`!Send`): a `Session` e tudo dentro dela (I-1). O lado do áudio é o `Renderer`, que só a callback tem. Chamar da
   thread errada não compila.
2. **Posse passa por movimento.** Nenhuma peça guarda referência para uma irmã. O contexto vem emprestado a cada
   chamada.
3. **Só quatro coisas cruzam entre threads**, cada uma com o seu tipo:
   - mensagens de instalação e de devolução (I-5);
   - vagas de parâmetro (I-6);
   - anéis de **um produtor para um consumidor**, de tamanho fixo, e o quadro de telemetria (I-7, I-8, I-11);
   - dados imutáveis compartilhados (`Arc` de algo que ninguém muda).
4. **Todo anel tem tamanho fixo e uma política escrita para quando enche.** Há duas: **contar como perdido e
   avisar**, ou **erro explícito que para a operação**. Nenhum anel cresce, e nenhum produtor de tempo real espera.
   **Todo anel é dimensionado em tempo** (C4).
   **Mensagem que carrega posse** (um `Box` ou um `Arc` voltando) **nunca vai num anel que perde**: vai num anel
   próprio, dimensionado para o máximo possível (T9). **Valor contínuo** (medidores, posição) não vai por anel: vai
   num quadro de telemetria por triplo buffer, e quem lê pega o mais recente (T5).
5. **A thread de áudio nunca aloca nem libera.** Tudo que ela larga volta à thread principal por um anel de
   devolução, inclusive todo `Arc` que entrou no áudio por fora de um processador (S7). A armadilha de alocação
   (`assert_no_alloc`) envolve a callback inteira em depuração e pega tanto a alocação quanto a liberação.
6. **A thread de áudio nunca vê a árvore.** O `lm-render` não depende do `lm-state` (R1). Os comandos e a telemetria
   se referem a nós por `NodeIndex` + `ProgramGen` (S1).
7. **Erros são valores** (`Result<T, Error>`), com chave de mensagem traduzível. Um pânico é bug e é contido
   (I-9.3).
8. **Nada que possa passar de 200 ms roda na thread principal** nem trava a interface. Toda operação assim é uma
   `Operation` (I-2): tem estado, progresso quando há como medir, e cancelamento quando faz sentido. A interface
   **nunca** mostra um estado intermediário: mostra o carregamento no lugar do objeto.
9. **`unsafe` só nos pacotes `lm-sys-*`.** Todo o resto tem `#![forbid(unsafe_code)]`. Estruturas sem trava vêm só
   de crates de uma lista permitida (I-17), verificada pelo `cargo-deny` (S8).
10. **Toda mensagem é um `enum` fechado.** Não há mensagem genérica com texto ou ponteiro dentro.
11. **`#[must_use]`** em todo tipo cujo esquecimento muda o comportamento (`OperationHandle`, `GestureId`,
    `Subscription`, `Result`) (S21).

---

## Parte 3: os pacotes

```
lm-base      Main, Role, Error, Label, Cancel, Progress, Phase, OperationHandle (genéricos, sem alvo), os anéis
             (sobre rtrb), HostTime, Clock (injetável nos testes)
lm-state     Value, NodeId, Schema, Json                                       → lm-base
lm-doc       Document, Edit, Diff, History, Feed, Refs, índices                → lm-state
lm-time      TempoMap, conversões exatas                                       → lm-base
lm-render    Processor, Buffers, Event, SampleData, ParamTable, Program, Install, Renderer, EngineControl,
             Commands, Telemetry                                               → lm-base, lm-time   (NÃO lm-state)
lm-dsp       os algoritmos de efeito e síntese, puros: filtros, dinâmica, válvulas, transformador, tolerância,
             osciladores, envoltórias (o que vem do legado, portado com A/B)   → lm-base
lm-plugin    Descriptor, a construção a partir do Value                        → lm-state, lm-render
lm-compile   Compiler: Value → Install                                         → lm-state, lm-render, lm-plugin, lm-time
lm-io        Device, Backend (as implementações, nesta ordem: Dummy, ALSA; depois JACK; depois CoreAudio),
             MidiInput (ALSA seq primeiro), DiskWriter,
             ResourceStore, Peaks                                              → lm-base, lm-render, lm-sys-*
lm-session   Session, Target, Activity, Actions, Journal, Autosave, Preferences, Notices, Export → todos acima
lm-plugins-* os plugins internos                                               → lm-plugin
lm-sys-*     ligações cruas com C e com o sistema; o unsafe e as trampolinas das callbacks moram aqui
lm-headless  o executável sem interface                                        → lm-session
lm-gui-*     a interface (decidida pelo protótipo)                             → lm-session
```

- **O `lm-dsp` é a pasta dos códigos de efeito** (decisão do usuário, 19/11): todo o DSP que vem do legado (o rack
  da mesa analógica, os efeitos, os osciladores) é portado para lá **uma vez** e reaproveitado por qualquer plugin e
  pelo rack. Regras do pacote:
  - só algoritmos: não conhece a árvore, o host, os eventos nem a interface;
  - cada algoritmo separa **preparar** (aloca, recebe taxa e tamanho máximo de bloco) de **processar** (não aloca, não
    trava, não lança), testado sob a armadilha de alocação;
  - sem `unsafe`; fase e contadores em `f64` (U35); funções matemáticas da `libm` em Rust puro (V2);
  - cada algoritmo portado tem o seu teste A/B contra o original do legado, com a régua do U34 e a lista de diferenças
    intencionais.
- **O `SampleData`** (áudio decodificado, imutável) é definido no `lm-render`, porque é o que os processadores e a
  prévia consomem. O `lm-io` o produz (S1).
- **As implementações dos backends** (CoreAudio etc.) ficam no `lm-io`, em Rust seguro, sobre as ligações cruas do
  `lm-sys-*`. O `lm-sys` não implementa o `Backend` (S16).
- **Nenhum pacote de baixo cita tipo de cima** (T2): o `lm-base` não conhece `NodeId`; o `lm-io` não conhece o
  documento (fala de entradas e caminhos de arquivo, e a `Session` traduz).
- **Os registros** (`Registry` de plugins, `SchemaRegistry`) são montados no início e depois ficam imutáveis, dentro
  de `Arc`: podem ir para qualquer `Operation` (T3).

---

## I-1. A thread principal como tipo (F1)

```rust
/// Proof of being on the main thread: at most one alive per thread (thread_local), so `cargo test`, which runs each
/// test on its own thread, can make one per test. Not Send, not Sync (S15)
pub struct Main { _not_send: PhantomData<*const ()> }

impl Main {
    /// None if this thread already has one
    pub fn acquire() -> Option<Main>;
}

pub enum Role { Main, Audio, Worker, Disk, Midi, Compile }
pub fn role() -> Role;
```

- **A `Session` guarda o `Main`**, então ela mesma é `!Send`. Tudo que é da thread principal é um método da
  `Session` (ou de algo que só se alcança por ela), e nenhum método precisa receber o token (S15).
- **No executável**, `Session::create` também exige que seja a thread principal do sistema operacional (no macOS, o
  CoreAudio, o CoreMIDI e a interface pedem isso). Nos testes, não.
- Threads só são criadas pela `Session`, com nome e papel. As bibliotecas em C que criam as suas estão declaradas na
  ficha (I-17).

## I-2. Operações demoradas e estados de carregamento (F1)

### I-2.1 A regra dos 200 ms

Toda operação que **possa** passar de 200 ms numa máquina comum é uma `Operation`. "Possa" quer dizer qualquer uma
das condições abaixo, mesmo que normalmente seja rápida:
- lê ou escreve disco;
- decodifica áudio;
- constrói um plugin;
- compila um programa;
- abre um dispositivo;
- varre portas ou plugins;
- espera outro processo.

**Lista de partida** (cada interface abaixo marca as suas com ⏳):

| Operação | Alvo do carregamento | Cancelável |
|---|---|---|
| Abrir projeto (ler, validar, migrar, repetir o diário) | o projeto | sim |
| Salvar, salvar como, autosave | o projeto | não (rápido de fato; só mostra se passar) |
| Importar áudio (copiar, converter taxa, hash, picos) | o item ou a lista de importação | sim |
| Carregar um recurso (amostra, soundfont, kit) | o nó que usa o recurso | sim |
| Construir um plugin (incluindo o processo auxiliar de terceiros) | o nó do plugin | sim |
| Compilar o programa (registrado pelo próprio compilador, T10) | o projeto (só aparece se passar do limite) | não (é substituído pelo próximo) |
| Abrir ou trocar o dispositivo de áudio | o dispositivo | não |
| Varrer portas MIDI ou plugins de terceiros | a lista | sim |
| Calibrar latência | o dispositivo | sim |
| Exportar | a exportação | sim |
| Fechar uma gravação (fechar o RF64, gerar os picos) | o take | não |
| Recuperar takes interrompidos | o projeto | sim |
| Preparar a prévia de um preset | a prévia | sim |
| Gerar picos de um recurso | o item | sim |

### I-2.2 A interface

```rust
/// What a long operation is doing, as data the UI reads
pub enum Phase {
    Queued,
    Running { progress: Option<f32>, stage: Label },   // progress 0..1 when measurable; stage: "Decoding"
    Done,
    Failed(Error),
    Cancelled,
}

/// lm-session: what the loading shows on; the object stays in its final place, marked as loading (T2)
pub enum Target { Project, Node(NodeId), Resource(Resource), Device, Export(ExportId), Scan(ScanKind) }

#[must_use]
pub struct OperationHandle<T> { /* poll() -> Option<Result<T>>, cancel(), id() */ }

impl Session {
    /// Starts `work` on a worker and registers it in the activity list for `target`
    pub fn start<T: Send + 'static>(&mut self, label: Label, target: Target, cancellable: bool,
        work: impl FnOnce(&Cancel, &Progress) -> Result<T> + Send + 'static) -> OperationHandle<T>;
    pub fn loading(&self, target: &Target) -> Loading;            // what the UI draws
    pub fn activity(&self) -> impl Iterator<Item = &OperationInfo>;  // all running, for a global indicator
    pub fn cancel(&mut self, op: OperationId);
}

/// The UI's view of one target
pub enum Loading {
    Ready,
    /// Started less than 200 ms ago: the UI draws nothing yet (no flicker on fast operations)
    Pending,
    /// Past 200 ms: the UI draws the loading state in place of the object (progress if known)
    Visible { progress: Option<f32>, stage: Label, cancellable: bool, since: Duration },
    Failed(Error),     // with a retry action in the notice (I-14)
    /// A resource the node needs is not where the project says (H17): not a loading, an action "locate file"
    Missing { resource: Resource },
}

pub const LOADING_THRESHOLD: Duration = Duration::from_millis(200);
```

- **`Phase` e `OperationHandle` são do `lm-base`**, genéricos; o `Target` e o `Activity` são do `lm-session` (T2).
- **Uma `Operation` nunca toca a `Session`.** Ela recebe dados (cópias, `Arc` imutáveis) e devolve um valor. Quem
  aplica o resultado é a `Session`, no `pump` (T18).
- **O compilador também registra a atividade dele** no `Activity`, no alvo `Project`, mesmo rodando na thread
  `Compile` e não pelo `start` (T10).
- **Agregação por alvo** (T10): o `loading(Node(id))` junta tudo de que o nó depende (a construção do plugin dele e
  os recursos que ele cita). O `since` é o da operação mais antiga; o progresso é o menor; se qualquer uma falhou, é
  `Failed`.
- **O relógio é injetável** (`Clock` do `lm-base`): os testes avançam o tempo à mão, e "nunca mostra intermediário"
  fica determinístico (T10).

### I-2.3 O que cada lado garante

- **O núcleo:**
  - o objeto entra **no lugar final** imediatamente (a faixa existe, o item está na linha do tempo), marcado como
    carregando;
  - um nó **novo** é **silêncio** até estar pronto: o compilador o compila como "esperando" (I-12). Nunca toca pela
    metade;
  - um nó que **troca** (outro SoundFont, outro kit) **continua tocando o antigo** até o novo ficar pronto, e aí
    acontece o `adopt` (T7). O silêncio fica só para o que é novo;
  - **fora do tempo real** (exportação e `lm-headless render`), "esperando" nunca vira silêncio: espera-se tudo
    ficar pronto antes de renderizar (T8);
  - o resultado entra por **uma** transação ou instalação, de uma vez.
- **A interface:**
  - antes de 200 ms, não desenha nada diferente (sem piscar);
  - depois de 200 ms, desenha o carregamento **no lugar** do objeto (a forma de onda vira uma barra de progresso, a
    janela do plugin mostra o carregamento em vez de botões vazios);
  - depois de 2 s, mostra o botão de cancelar, se a operação permitir;
  - uma falha vira aviso com "tentar de novo".
- **O pool (`Workers`):** ao ser solto, o que ainda está na fila termina como `Cancelled` sem rodar (nenhuma alça fica
  em `Queued` para sempre) e só o trabalho em andamento é esperado; uma fila com mutex envenenado segue funcionando; se
  o sistema recusar todas as threads, `start` falha a operação com `error.operation.no_worker` em vez de derrubar o
  processo.
- **Valores de parâmetro:** todo evento `Param` entra na lista de eventos do nó por um ponto só (`EventList::push`) e é
  limitado ali à faixa do esquema do plugin (`ProgramBuilder::param_limits`, preenchido pelo compilador a partir do
  `ParamSpec`); não finito é descartado. Automação, modulação, MIDI e a mão passam pelo mesmo limite, então nenhum
  plugin repete a checagem.
- **Um componente de carregamento só**, na biblioteca de interface, usado por todas as telas. Ele lê o `Loading` do
  alvo.
- **Medição:** a duração de toda operação vai para o perfil da sessão. Uma operação que passa de 200 ms sem estar
  na lista é registrada em depuração como "operação lenta não declarada".

### I-2.4 Testes

- Um modo de teste injeta atraso em disco, decodificação e construção de plugin. Cada tela do protótipo e da fatia
  é conferida para mostrar o carregamento e nunca um estado intermediário.
- O `lm-headless` imprime as operações e o progresso.

---

## I-3. O documento (F1)

### I-3.1 `Value`

```rust
/// Immutable; clone is O(1). Lists are persistent vectors and objects are persistent maps with sorted keys (imbl,
/// pending the 10,000-note measurement, S22)
#[derive(Clone)]
pub struct Value(/* ... */);

impl Value {
    pub fn int(v: i64) -> Value;
    pub fn real(v: f64) -> Result<Value>;                 // NaN and ±Inf refused; -0.0 kept (R15)
    pub fn text(v: impl Into<Arc<str>>) -> Value;
    pub fn get(&self, key: &str) -> Option<&Value>;
    pub fn at_path(&self, path: &[PathStep]) -> Option<&Value>;
    pub fn with(&self, key: &str, v: Value) -> Value;
    pub fn same_node(&self, other: &Value) -> bool;      // O(1) identity
}
```

- **Sem conversões implícitas** (R15).
- **As chaves de um objeto ficam em ordem alfabética** (S14). O JSON sai estável e o diff fica simples. As listas
  mantêm a ordem delas.
- **JSON:** reais sempre com `.0` ou expoente; `-0.0` preservado; inteiros acima de 2^53 recusados na leitura (os
  ids são texto). A escrita usa `ryu` (R15).

### I-3.2 Ids

```rust
/// "t:17". One global counter for every kind, notes included; never given twice (R7, S12)
pub struct NodeId { pub kind: u8, pub number: u64 }
pub const ROOT: NodeId = NodeId { kind: b'r', number: 0 };
```

- **O contador fica em `"meta": { "issued": N }`**, a única parte da raiz que o desfazer **não** restaura (R7).
- **As notas também tiram números do contador global** (`"n": 812`). Desfazer a criação de uma nota e criar outra
  nunca devolve o número (S12). As notas não entram no índice de ids (são muitas): ficam num mapa persistente por `n` dentro do item (C1).
- **Todo objeto que guarda uma referência tem `id`:** envios, lanes de automação, entradas de sidechain. Assim, o
  `OnRemove::RemoveOwner` sempre tem o que remover (D4).

### I-3.3 Esquemas e referências

```rust
pub enum FieldKind {
    Bool, Int { min: i64, max: i64 }, Real { min: f64, max: f64 }, Text, Choice(&'static [&'static str]),
    Resource(&'static str), Ref { to: &'static [u8], on_remove: OnRemove },
    List(Box<FieldKind>), Object(&'static str),
}
pub enum OnRemove { RemoveOwner, Clear, Refuse, Redirect(NodeId) }   // (R10); Redirect: e.g. an output to the master
```

Uma chave de parâmetro publicada é permanente; renomear exige apelido no esquema (C5).

### I-3.4 Os índices (S13, R8)

Cada raiz carrega, junto, **dois índices persistentes**. Desfazer e refazer os trazem com a raiz:
- **id → (pai, chave da lista):** a posição dentro da lista é achada na hora (as listas com id são curtas: faixas,
  itens de uma faixa, plugins, envios). Inserir e mover não reescrevem os irmãos;
- **referências reversas:** quem aponta para cada id. É o que o `remove` usa para aplicar o `OnRemove`.

### I-3.5 Transações e edições

```rust
impl Session {
    pub fn transact(&mut self, label: Label, edit: impl FnOnce(&mut Edit) -> Result<()>) -> Result<Diff>;
    pub fn undo(&mut self) -> Result<Diff>;
    pub fn redo(&mut self) -> Result<Diff>;
    pub fn find(&self, id: NodeId) -> Option<Value>;      // a clone, never a pointer into a root (R8)
}

impl Edit<'_> {
    pub fn set(&mut self, id: NodeId, path: &[PathStep], v: Value) -> Result<()>;   // a field at any depth (D8)
    pub fn insert(&mut self, parent: NodeId, list: &str, index: usize, node: Value) -> Result<NodeId>;
    pub fn remove(&mut self, id: NodeId) -> Result<()>;                              // applies OnRemove
    pub fn move_to(&mut self, id: NodeId, parent: NodeId, list: &str, index: usize) -> Result<()>; // final index (R11)
    pub fn add_note(&mut self, item: NodeId, note: Value) -> Result<u64>;            // its "n"
    pub fn set_note(&mut self, item: NodeId, n: u64, path: &[PathStep], v: Value) -> Result<()>; // (T12)
    pub fn remove_note(&mut self, item: NodeId, n: u64) -> Result<()>;
}
// PathStep::Note(n) also reaches a note inside a path (T12)

pub struct Diff {
    pub created: Vec<NodeId>, pub removed: Vec<NodeId>,
    pub changed: Vec<Changed>,          // node and paths; for an item: notes created, changed, removed by "n" (T12)
    pub structural: bool,
    pub params: Vec<(NodeId, Arc<str>, f64)>,   // applied to the slots of nodes already installed (S20)
    pub origin: Origin,                 // User, Undo, Redo, Gesture, Plugin, Recording, Journal
}
```

**Dentro do programa, uma mudança é código** (decisão do usuário, 19/11): a interface gráfica e as ações usam o
`transact` com fechamento, o `gesture_update` com fechamento e o `heavy_edit`, direto em Rust, sem serializar. É o
caminho mais eficiente, e o compilador confere tudo. A forma serializável das mudanças (`EditOp`) existe **só no
protocolo externo** (I-20), que a traduz para essas mesmas chamadas.

Uma transação inválida (esquema, ciclo no roteamento, pasta dentro de si mesma, **passar dos `Limits` do motor**,
H12) é recusada inteira e não muda nada. Os limites são regras do documento, não só do compilador: o usuário nunca vê
uma faixa aceita que não toca.

**Edições pesadas** (colar milhares de notas, quantizar o projeto, importar MIDI) não rodam na thread principal: H7.

### I-3.6 Saber o que mudou: o modelo de puxar (S3, D6)

```rust
#[must_use]
pub struct Feed { /* a receiver of diffs for one topic */ }

impl Session {
    pub fn feed(&mut self, topic: Topic) -> Feed;
}
impl Feed {
    /// The diffs since the last drain, merged. Called by the UI in its frame, after Session::pump
    pub fn drain(&mut self, session: &Session) -> Option<Diff>;
}
pub enum Topic { Node(NodeId), Type(&'static str), All }
```

- **Ninguém é chamado de volta.** A interface, a cada quadro, pergunta a cada `Feed` o que mudou e redesenha.
- **A mescla é sobre o estado final** (T13): criado e removido entre dois `drain` não aparece; removido vence
  mudado; mudado várias vezes aparece uma vez, com a união dos caminhos.
- **Um `Feed` largado** avisa a `Session` (pelo `Drop`, por um canal `Rc` compartilhado) e ela para de acumular para
  ele.
- Some a reentrância por construção: quem reage abre uma transação depois, com a `Session` livre.
- A interface não precisa de `Rc<RefCell<>>`.

### I-3.7 Gestos (S2)

```rust
#[must_use]
pub struct GestureId(u64);

impl Session {
    pub fn begin_gesture(&mut self, label: Label) -> GestureId;
    pub fn gesture_update(&mut self, g: &GestureId, edit: impl FnOnce(&mut Edit) -> Result<()>) -> Result<()>;
    pub fn gesture_commit(&mut self, g: GestureId) -> Result<Diff>;
    pub fn gesture_cancel(&mut self, g: GestureId);
}
```

- O gesto não empresta nada: a interface guarda o `GestureId` entre quadros.
- **Vários gestos ao mesmo tempo são permitidos** (o mouse num botão e um controlador MIDI noutro) (T4).
- **A regra única do histórico (substitui a do "topo", T4):** enquanto um gesto está aberto, os `update`s mudam a
  raiz atual, **sem criar passo** no histórico. O passo do gesto entra no histórico **no `commit`**, com a raiz
  daquele momento. Uma transação alheia no meio do gesto entra como passo próprio, na hora, com a raiz como estava
  (incluindo o valor do gesto naquele instante). Desfazer volta sempre a um estado que realmente existiu. Dois gestos
  alternados dão dois passos, um em cada `commit`.
- **Largado sem `commit` nem `cancel`:** o `Drop` do `GestureId` manda o id por um canal `Rc` compartilhado com a
  `Session`, e ela o confirma no próximo `pump`, com aviso em depuração. `commit` e `cancel` consomem o id sem
  disparar o `Drop`. O `#[must_use]` faz o esquecimento aparecer na compilação (S21, T4).
- Durante o gesto, o valor vai ao áudio pela vaga (I-6) e a vaga fica marcada como tocada.
- **O gesto guarda os caminhos que tocou e os valores de antes dele.** É o que torna os três casos abaixo possíveis.
- **Cancelar (G1):** restaura **só os caminhos que aquele gesto tocou**, com os valores de antes do gesto, sobre a raiz
  atual. Uma transação alheia que entrou no meio e outro gesto aberto não são apagados. A vaga do parâmetro volta
  junto. Se algum passo alheio entrou no histórico durante o gesto, o cancelamento entra como passo próprio
  ("Cancelar <rótulo>"), para o histórico continuar sendo uma lista de estados que existiram; se nenhum entrou, não
  há passo.
- **Desfazer ou refazer com gesto aberto (G2):** o `undo` e o `redo` primeiro fazem o `commit` de **todos** os gestos
  abertos e só então desfazem. O primeiro Ctrl+Z no meio de um arrasto desfaz o próprio arrasto. Os `GestureId`
  daqueles gestos passam a ser inválidos: um `update` depois disso devolve `GestureClosed`, e a interface solta o
  controle.
- **Alvo removido no meio (G3):** se o nó que o gesto mexe sai da árvore (uma transação alheia, ou um desfazer pelo
  G2), o próximo `update` devolve `GestureTargetGone` e o gesto é encerrado sem passo. A vaga não é escrita, porque o
  nó saiu do programa.
- **Diário (T11):** um gesto vai ao diário no `commit`; enquanto aberto, a cada 1 s, para um arrasto longo não se
  perder numa queda. Nunca um patch por movimento do mouse.

### I-3.9 Faixas, saídas e a mesa (decisão do usuário, 19/11)

- **Faixa e mesa são separadas** (`arquitetura-nova.md`, especificação, item 2), com **roteamento livre**: a lista
  `outputs` de cada faixa diz para qual canal da mesa (`"to": "m:3"`) vai cada saída; várias saídas podem ir para o
  mesmo canal.
- **Cada saída é um nó com id** (`o:32`), com a chave da porta (`"port": "kick"`), o número de canais e o destino.
  `"to"` é uma referência com `OnRemove` próprio: remover o canal faz a saída mandar para o master (um quarto caso do
  `OnRemove`: `Redirect(MASTER)`).
- **Quando o instrumento muda de portas** (outro kit, outro plugin), a `Session` faz a reconciliação **na mesma
  transação** que trocou o estado:
  - porta que continua (mesma chave): a saída fica como estava;
  - porta nova: uma saída nova, mandando para o destino da saída principal;
  - porta que sumiu: a saída sai da lista, e o `Diff` e um aviso dizem quais.
- **O compilador** gera, para cada saída, um caminho até o canal de destino, e soma no canal em ordem fixa
  (determinismo). A PDC é calculada por saída.
- **O rack da mesa e os inserts** são do canal da mesa, não da faixa.
- **A reconciliação das portas é uma regra de domínio do `Edit`** (M1), registrada para o tipo "faixa com
  instrumento": roda no commit de **toda** transação que mudou o estado de um instrumento, venha de onde vier (a
  interface, uma ação, o protocolo remoto, um `heavy_edit`, colar, aplicar um preset). Nenhum caminho fica sem ela. O
  replay do diário não reconcilia, porque os patches já trazem o resultado.
- **Endereço de um parâmetro** (M5): `(id do nó, caminho da chave)`, por exemplo `{"node": "m:3", "param":
  "rack.tube.drive"}` ou `{"node": "p:5", "param": "vol1"}`. Todas as chaves do caminho são permanentes (C5). As
  seções do rack (`tube`, `eq`, `dynamics`, `transformer`, `tolerance`) são chaves fixas do esquema do canal, sem id
  próprio.
- **O rack é um nó do programa** (M5), processado pelo `Renderer` como os inserts, com a sua latência entrando na PDC
  e o seu medidor.
- **Faixa de áudio e as saídas** (M6): a faixa de áudio declara o próprio formato (`"channels"`, como a entrada), e as
  saídas dividem esse formato. Um item com outro número de canais é convertido ao tocar (mono → estéreo pelo pan,
  estéreo → mono pela soma), e as saídas nunca mudam por causa de um item.
- **Medidores** (M9): cada saída de faixa e cada canal da mesa são nós medidos no quadro de telemetria.
- **Monitoramento** (M10): passa pela saída principal da faixa armada.
- **A faixa não tem pan** (decisão do usuário, 19/11): só volume, mudo e solo; o volume e o mudo valem para todas
  as saídas. O pan é do canal da mesa. Os dois solos: especificação, item 2 (`arquitetura-nova.md`), padrão proposto a
  confirmar pelo usuário.

### I-3.8 O documento da interface (D9)

O estado da interface (janelas, zoom, rolagem, seleção) é **um segundo `Document`**, com o mesmo `Value`, os mesmos
esquemas e o mesmo `Feed`, **sem histórico de desfazer** e **fora do diário**. É salvo em `interface.json` só no
salvamento e no autosave (T16). A seleção se refere a ids do
projeto; um id que sumiu sai da seleção no próximo `pump`.

## I-4. Diário e autosave (F1) ⏳

**O diário grava diferenças (patches), não ações (R16):**

```
backup/diario.jsonl
{"base": "<sha256 do projeto.json>", "seq": 0}
{"seq": 1, "patch": [{"op": "set", "path": [...], "value": ...}]}
```

- Ao abrir, se o `base` bate, os patches são reaplicados, como parte da operação "abrir projeto". Se não bate, o
  diário é guardado à parte e o usuário é avisado.
- Salvar recomeça o diário.
- A escrita é numa thread `Disk`, por um anel. Se o anel encher, o diário é marcado incompleto e um autosave completo
  é feito em seguida. Nunca há perda silenciosa.

---

## I-5. Do documento ao áudio (F1)

### I-5.1 Os dois lados do motor

```rust
pub fn engine(spec: ProcessSpec, limits: Limits) -> (EngineControl, Renderer);

pub struct Limits {
    pub max_nodes: usize, pub max_params: usize, pub max_command_producers: usize,
    pub max_previews: usize, pub installs_in_flight: usize,
}
```

O `Renderer` é dono dos processadores, numa tabela de tamanho fixo (`max_nodes`).

### I-5.2 A mensagem de instalação

```rust
pub struct Install {
    pub gen: ProgramGen,
    pub program: Box<Program>,
    pub add: Vec<(NodeIndex, Box<dyn Processor>)>,
    pub replace: Vec<(NodeIndex, Box<dyn Processor>)>,      // adopt() (R18)
    pub remove: Vec<NodeIndex>,
    pub producers: Vec<(ProducerIndex, CommandReceiver)>,   // new command rings (I-7.1)
    /// Reserved on the main thread with room for everything the audio thread will put back: the old program, the
    /// removed and the replaced processors (D2). The audio thread only pushes into reserved space
    pub returned: Returned,
}
```

### I-5.3 As regras da instalação (S5, S6)

- **O `Renderer` aplica todas as `Install` do anel, em ordem**, no início do bloco. Nunca pula uma: cada uma é uma
  diferença em relação à anterior (S5). A coalescência das raízes (R17) acontece antes, no compilador.
- Mais instalações que `installs_in_flight`: elas esperam numa **fila da thread principal**. Nada bloqueia.
- **O compilador é o único que aloca `NodeIndex` e vagas de parâmetro.** Ele lembra os pendentes. Um índice removido
  só volta a ficar livre quando a `Install` que o removeu tiver voltado pelo anel de devolução (S6).
- **A sequência, no `Renderer`:**
  1. move os `add` para a tabela;
  2. troca os `replace` com `adopt`, só com o mesmo plugin e a mesma versão maior; o velho não processa mais (R18);
  3. tira os `remove`;
  4. instala os anéis novos;
  5. troca o programa;
  6. põe o que saiu em `returned` e devolve a mensagem.
- **A thread principal libera tudo no `EngineControl::collect`** e avisa o compilador de quais índices voltaram a
  ficar livres.

### I-5.4 O compilador ⏳

```rust
impl Compiler {
    /// Only the newest root is compiled; older ones submitted meanwhile are skipped (R17)
    pub fn submit(&mut self, root: Value, resources: ResourceView);
    pub fn poll(&mut self) -> Option<Result<Box<Install>>>;
    pub fn returned(&mut self, freed: &Returned);   // indices free again (S6)
    pub fn ready(&mut self, node: NodeId, processor: Box<dyn Processor>); // a built processor (T6)
}
```

- A compilação roda na thread `Compile`. Enquanto ela trabalha, **o áudio continua com o programa anterior**: nunca
  há silêncio por compilação. Se passar de 200 ms, o alvo `Project` mostra "aplicando".
- **O compilador não constrói plugins** (T6), e um build só começa quando os recursos dele estão carregados (C11). Construir é sempre uma `Operation` num worker (⏳), com o alvo no nó do
  plugin. O compilador emite o nó como "esperando" (novo) ou mantém o processador antigo (troca, T7). Quando o
  processador fica pronto, a `Session` o entrega ao compilador (`compiler.ready(node, processor)`), que o põe na
  próxima `Install`. Assim, um SoundFont que demora 2 s não atrasa nenhuma outra compilação.
- **Tudo vira estrutura plana do `lm-render`**: eventos por item, curvas por segmento, rotas, atrasos da PDC, a
  tabela `NodeIndex ↔ NodeId` e a tabela de parâmetros (I-6). Nenhum `Value` atravessa (R1).
- **Valores iniciais dos parâmetros** vão no programa: um nó novo nasce com o valor da árvore (S20).
- **Parâmetros:** `f64` na árvore, `f32` no programa, com a mesma conversão ao vivo e na exportação (R3).

### I-5.5 Metrônomo e contagem (F1)

O compilador sempre põe um nó de metrônomo no programa, a partir do mapa de andamentos e das configurações do
transporte. Os sons do metrônomo são recursos. A contagem é o pré-roll tocando o metrônomo. A saída do metrônomo é
escolhida nas preferências: o master ou uma saída própria do dispositivo.

## I-6. Parâmetros (F1)

### I-6.1 As vagas (S8, S10)

```rust
/// Fixed capacity (Limits::max_params), made by engine(): an Arc<[AtomicU32]> (f32 bits) plus a dirty bitset
/// (one AtomicU64 per 64 slots). No growth, so no unsafe
pub struct ParamSlots { /* ... */ }

impl ParamSlots {
    pub fn write(&self, slot: ParamSlot, value: f32);       // main: store, then set the dirty bit
    pub fn touch(&self, slot: ParamSlot, touching: bool);   // a gesture started or ended
}
```

- **A ordem de memória** (T15, testada com `loom`):
  - quem escreve: guarda o valor com `Relaxed` e depois liga o bit sujo com `fetch_or(Release)`;
  - o `Renderer`: troca a palavra do bitset por 0 com `swap(0, Acquire)` e só depois lê os valores daquela palavra.
- **O "tocado"** é um segundo bitset, com a mesma ordem de memória.
- **Quem dá os índices das vagas é o compilador** (I-5.3). O mapa `(NodeId, parâmetro) → ParamSlot` vem na `Install`
  e é guardado pela `Session` quando ela volta confirmada. É com ele que a `Session` escreve as vagas no desfazer e
  no refazer (R12, D1).

### I-6.2 Um caminho só até o processador (R3)

- No início do bloco, o `Renderer` varre **só as vagas sujas** (o bitset) e gera eventos `Param` no offset 0 (S10).
- A automação gera eventos no offset exato. Nas curvas, a cada sub-bloco de até 32 amostras, ou por amostra nos
  parâmetros `Rate::Sample`.
- **Vaga tocada: a mão vence a automação** até soltar (o modo "touch").
- O processador **só vê eventos**. O valor da vaga só é lido diretamente no `reset` e no `adopt`.

### I-6.3 Escrita no desfazer

Ao aplicar um `Diff` com `params`, a `Session` escreve as vagas dos nós **já instalados**. Para os nós criados na
mesma transação, vale o valor inicial do programa (S20).

### I-6.4 Gravação de automação (F2)

A telemetria manda as vagas tocadas com a posição em amostras, num anel que nunca perde (C3). A `Session` acumula e, ao soltar o gesto ou parar o
transporte, cria a curva numa transação de `Origin::Recording`, simplificada.

## I-7. Comandos para o áudio (F1)

### I-7.1 Um anel por produtor, com número fixo (S17, D5)

```rust
pub enum Command {
    Play, Stop, Locate { sample: i64 }, Record { on: bool }, Panic,
    Midi { port: PortIndex, at: HostTime, bytes: [u8; 3] },    // routed by the program's port table
    Preview(PreviewCommand),                                    // F2
}
```

- O `Renderer` tem um array fixo de `max_command_producers` receptores. A thread principal tem o primeiro. Cada
  entrada MIDI que aparece ganha um anel, entregue ao áudio dentro de uma `Install`. Uma entrada que some sai por um
  "remover produtor" numa `Install`, e o receptor volta pelo `returned` dela (T9).
- **Sem sysex na F1** (T14). Na F2, o sysex vai num anel próprio de mensagens de tamanho máximo fixo (e mensagem maior
  é recusada com aviso).
- **O MIDI não sabe de faixas.** Ele manda `port` + `HostTime` + bytes. A tabela "porta → nós" está no programa,
  compilada das configurações de entrada das faixas (S1).
- **Os relógios** estão definidos em C6 (Linux: `CLOCK_MONOTONIC`, com o PCM e o seq do ALSA convertidos para ele).
- **Carimbo de tempo (R14):** o `Renderer` converte o `HostTime` em offset com o `HostTime` do início do bloco, que
  o dispositivo informa. Por padrão, há um bloco de atraso fixo, sem tremor. A opção "menor latência, com tremor"
  fica nas preferências.

### I-7.2 O transporte (F1)

- A posição verdadeira é do `Renderer`, em amostras, e chega à interface pela telemetria (com interpolação para
  desenhar).
- A árvore guarda só o persistente: loop, punch, pré-roll.
- `Locate` com gravação ativa é recusado e avisado.
- **Editar o mapa de andamentos tocando preserva a posição musical** (H9): o compilador manda, junto da `Install`, um
  `Relocate { tick }`, e o `Renderer` reposiciona na amostra que o mapa novo dá para aquele tique.

### I-7.3 Prévia (F2, caminho próprio) (D7) ⏳

```rust
pub enum PreviewCommand {
    Sample { id: PreviewId, data: Arc<SampleData>, gain: f32 },
    Preset { id: PreviewId, processor: Box<dyn Processor>, key: u8 },  // built before, in an Operation
    NoteOff { id: PreviewId },
    Stop { id: PreviewId },
}
```

- A prévia **não passa pelo compilador**. O programa tem um barramento de prévia com `max_previews` vagas próprias.
  O comando entrega o dado ou o processador pronto.
- **Tudo volta pela devolução:** ao terminar, a vaga devolve o `Arc` ou o `Box` numa mensagem
  `PreviewEnded { id, returned }`, liberada na thread principal (S7). Essa mensagem vai num **anel próprio que nunca
  perde**, dimensionado em `max_previews` (T9).
- Preparar a prévia de um preset é uma `Operation` (⏳). O navegador mostra o carregamento se passar de 200 ms.

## I-8. Telemetria (F1)

```rust
/// Continuous values: peaks accumulate per block in the renderer's own memory; a frame is published into a triple
/// buffer at most ~60 times a second (H16). The UI reads the latest
pub struct TelemetryFrame {
    pub gen: ProgramGen,
    pub position: i64, pub host_time: HostTime, pub playing: bool, pub recording: bool,
    pub meters: Box<[Meter]>,               // Limits::max_nodes, allocated by engine(); by NodeIndex, read with the table of `gen`
}

/// Events: rings that count what they drop
pub enum Telemetry {
    // Touched goes in its own lossless ring (C3), not here
    PluginRequest { gen: ProgramGen, node: NodeIndex, request: PluginRequest },
    Xrun { late_by: u32 },
    NodePanicked { gen: ProgramGen, node: NodeIndex },
    RecordingOverflow { input: u32, at_sample: i64 },
    Dropped { kind: TelemetryKind, count: u32 },
}
```

- **As mensagens com posse** (`PreviewEnded`, e a devolução das `Install`) vão nos seus anéis próprios, que nunca
  perdem (T9).
- **Toda mensagem sobre um nó leva a `ProgramGen`** (S9, D3). A `Session` guarda as tabelas das duas últimas
  gerações e traduz com a certa. Uma mensagem de uma geração mais velha é descartada.
- Um anel por tipo, cada um com a sua política (regra 4).

---

## I-9. Plugins

### I-9.1 O descritor (F1)

```rust
pub struct Descriptor {
    pub id: &'static str,
    pub schema: PluginSchema,
    /// Named audio ports. An instrument may declare several outputs (a drum kit: "kick" mono, "oh" stereo...),
    /// and the layout may depend on the state (the kit): `ports(state)` is called at build. Port keys are permanent,
    /// like parameter keys (C5)
    pub ports: fn(&Value) -> Ports,
    pub display: Option<DisplaySpec>,                  // F2
    /// In an Operation (⏳): everything the processor will ever need is allocated here
    pub build: fn(&Value, &ProcessSpec, &ResourceView) -> Result<Box<dyn Processor>>,
}
```

### I-9.1a Efeito de nota (`PluginKind::NoteEffect`) (26/11)

Um terceiro tipo de plugin, ao lado do instrumento e do efeito de áudio: **sem áudio e sem portas**, mora na lista
`noteEffects` da faixa, antes do instrumento, e transforma o fluxo de notas.

- **No contrato.** O processador implementa `Processor::process_notes(&mut NoteContext)`: recebe, ordenados, os próprios
  eventos `Param` e o fluxo que chega (notas, expressões, mensagens de canal, `AllNotesRelease`, `AllSoundOff`) e empurra
  o que emite em `cx.out` (`NoteOut`, de capacidade fixa, sem alocar), no deslocamento em que cada evento vence. Sem
  áudio: o `process` não faz nada. O nó do programa é `NodeKind::NoteEffect`.
- **No programa.** `ProgramBuilder::note_chain(instrumento, efeitos)`. O que as fontes (itens, entrada ao vivo, a
  liberação de notas pendentes, o rack de CC) empurram para o instrumento passa pela cadeia, em ordem, antes do passe;
  o que sai do último é o que o instrumento toca e o que a saída MIDI para dispositivos manda. Um efeito ainda sem
  processador (construindo) ou morto deixa o fluxo passar. Quando a cadeia muda (efeito entra, sai, é ignorado ou muda de
  lugar), o instrumento recebe `AllNotesRelease` pela cadeia nova.
- **Regras do efeito** (documentadas em `lm_plugin::note_effect`): determinístico e sem depender do tamanho do bloco;
  o tempo vem de `transport.frame` (nunca salta) e o andamento de `transport.bpm`; o aleatório, de `cx.rng` ou de um hash
  de uma semente; as notas que ele cria têm números a partir de `GENERATED_IDS` e substituem as que recebeu; encaminha
  as expressões e as liberações; o note-off leva a tecla com que o note-on saiu.
- **Testes gerados.** `lm_plugin::note_effect_tests!(DESCRIPTOR, "pasta")`: estado (padrões, estados aleatórios,
  atualizações, padrões congelados) e os próprios (determinismo e independência do bloco sob a armadilha de alocação,
  notas equilibradas, liberações encaminhadas, varredura de parâmetros, `adopt` com notas tocando, `reset`).

### I-9.2 O processador e os buffers (F1) (S4, D12)

```rust
pub trait Processor: Send {
    fn process(&mut self, cx: &mut ProcessContext<'_>);
    fn reset(&mut self) {}
    fn latency(&self) -> u32 { 0 }
    fn adopt(&mut self, old: &mut dyn Processor) {}     // same plugin id and major version only (R18)
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// One contiguous, preallocated block per node; channels by index. No arrays of slices are built per block
pub struct Buffers<'b> { /* ... */ }
impl Buffers<'_> {
    pub fn input(&self, ch: usize) -> &[f32];
    pub fn output(&mut self, ch: usize) -> &mut [f32];
    pub fn in_out(&mut self, ch: usize) -> &mut [f32];   // in place: input and output are the same channel
    pub fn sidechain(&self, ch: usize) -> Option<&[f32]>;
}

pub struct ProcessContext<'b> {
    pub frames: usize,
    pub buffers: Buffers<'b>,
    pub events: &'b [Event],
    pub transport: &'b TransportInfo,
    pub rng: &'b mut Rng,
    pub host: &'b mut HostOutbox,                      // F2
    pub display: Option<&'b mut DisplayWriter>,        // F2
}
```

- O processador é `Send` e não é `Sync`.
- Os tempos de vida do contexto impedem guardar buffers ou eventos depois do bloco.
- **Vozes:** `VoicePool<V>`, alocado no `build`, com a política de roubo da arquitetura.

### I-9.3 Pânico (F1) (S11, D10)

- **Um hook de pânico próprio**, instalado pela `Session`. Na thread de áudio, ele não imprime nada (nada de E/S nem
  de trava): só anota o nó num slot pré-alocado. A mensagem do pânico é impressa depois, na thread principal, a
  partir da telemetria.
- Cada `process` roda dentro de `catch_unwind(AssertUnwindSafe(...))`. O nó que entrou em pânico é marcado morto: dá
  silêncio e **nunca é chamado de novo**. Ele fica na tabela, morto, até a thread principal mandar o `remove` (ou o
  `replace`, ao recriar) numa `Install`, e volta pelo `returned` dela (T9).
- **A trampolina da callback do dispositivo**, no `lm-sys`, também envolve o render inteiro em `catch_unwind`: um
  pânico nunca atravessa para o C (D10).
- O caminho do pânico aloca (a mensagem). É a única exceção à regra 5, e só acontece quando já houve um bug. Os
  testes contam pânico como falha.
- `panic = "unwind"` no build.
- **Fora do áudio também** (H14): o compilador e cada `Operation` rodam dentro de `catch_unwind`. Um pânico vira
  `Failed` com aviso; o compilador é recriado (com a vista do que está instalado, pedida de novo à `Session`), e o áudio
  segue com o programa anterior.

### I-9.4 O plugin falando com o host (F2) (S18)

```rust
pub enum PluginRequest { LatencyChanged, ParamChanged { index: u32, value: f32 }, Error(PluginErrorKey), StateChanged }
```

- `LatencyChanged` → recompilar.
- `ParamChanged` → transação `Origin::Plugin`, agrupada em 300 ms.
- `StateChanged` **só existe para plugins de terceiros**: o estado é pedido ao processo auxiliar numa `Operation`,
  sem passar pelo áudio. Os internos não têm `StateChanged`: o estado deles só muda pela árvore (S18).

### I-9.5 Dados ao vivo para a janela do plugin (F2)

Um triplo buffer de tamanho fixo, declarado no descritor e alocado com o processador (crate `triple_buffer`). O
áudio escreve e a janela lê o último quadro completo. O formato dos bytes é do plugin.

**Convenção do quadro (que `CorePort::display(node)` serve):** palavras `f32` little-endian (`DisplaySpec.bytes` é
múltiplo de 4; uma sobra de menos de 4 bytes é descartada). `DisplayView.frame` é o quadro decodificado e
`DisplayView.kind` é o id do plugin (`"rack"` para um canal ou o master da mesa, cujo layout é `rack::display`).
Vale para **todo nó de plugin** (instrumento ou insert, interno ou de terceiros) que declara `DisplaySpec`
(`Registry::display(id)`): a sessão cria o canal ao construir o processador, entrega o `DisplayWriter` no
`ProcessContext` e guarda o `DisplayReader`; ler não bloqueia nem aloca na thread de áudio. NaN = "sem leitura".
O `lm-remote` leva o quadro como está (`KIND_DISPLAY`, só enquanto a janela assina).

### I-9.6 Plugins de terceiros fora do processo (F2) ⏳

- Um processo auxiliar, memória compartilhada (buffers, eventos, pedidos) e um semáforo do sistema por bloco. O objeto
  de memória é anônimo e é o PROCESSO PRINCIPAL que o cria e o dimensiona (Linux: `memfd` com selos contra encolher e
  crescer; macOS: `shm_open` com o nome removido na hora); o auxiliar o recebe por descritor (`SCM_RIGHTS`, descritor 3)
  e nunca abre nada por nome. Um bloco com mais eventos do que a memória comporta roda em pedaços consecutivos, sem
  perder evento.
- O auxiliar é não confiável e não prende o principal nem a thread de áudio: dentro dele, a thread de áudio só _tenta_
  pegar o plugin (sem esperar; ocupado, o bloco sai em silêncio) e a thread principal espera a vez; e ele termina sozinho
  quando o principal some (canal de controle fechado ou pai trocado).
- **Ao vivo:** prazo por bloco; perdeu o prazo, aquele bloco do nó sai em silêncio; depois de N perdidos seguidos, o
  processo é dado como travado, morto, e a `Session` oferece recriá-lo a partir da árvore.
- **Exportação: espera sem prazo** (S19), para o arquivo sair igual bit a bit.
- Iniciar o processo auxiliar e pedir o estado são `Operation`s.
- **A varredura de plugins também roda num processo auxiliar** (H15). Um plugin que derruba a varredura vai para uma
  lista de bloqueados nas preferências, com aviso, e não é tentado de novo até o usuário pedir (`plugin.unblock`). Uma
  queda bloqueia na hora; um estouro de tempo é só uma contagem, e o plugin é tentado de novo na varredura seguinte
  (bloqueia no segundo estouro).
- O protocolo é versionado.

---

## I-10. Dispositivos (F1)

### I-10.1 A barreira (R5, S16, D11)

```rust
pub trait Backend: Send {                          // lm-io: Dummy, ALSA (F1); JACK; CoreAudio (over lm-sys-*)
    fn open(&mut self, config: &DeviceConfig) -> Result<DeviceInfo>;
    fn start(&mut self, callback: CallbackHandle) -> Result<()>;
    fn stop(&mut self);
}

pub struct Device { /* backend, Arc<CallbackGuard> */ }

impl Device {
    pub fn start(&mut self, renderer: Renderer) -> Result<()>;
    /// The renderer comes back only after the last callback has returned
    pub fn stop(&mut self) -> Result<Renderer, StopTimedOut>;
}

impl Drop for Device {
    fn drop(&mut self) { /* stop() with the barrier; on timeout: report and leak the stuck renderer, never kill */ }
}
```

- **O `Drop` faz a barreira** (S16): a `Session` que cair sem chamar `stop` ainda espera a última callback.
- **Se o `stop` estourar o tempo:** o `Renderer` fica preso na callback travada. A `Session` descarta o
  `EngineControl` velho, cria um par novo com `engine()`, recompila a partir da árvore e avisa o usuário. O
  dispositivo travado é abandonado, não morto (D11).
- **`Device::start` devolve `StartError`:** `Refused { error, renderer }` (o renderer volta) ou `Lost { error }` (o
  backend falhou e ficou com a callback: o device é abandonado como num `stop` travado, chave `io.device.start_lost`).
  `Device::open` de hardware é recusado (`io.device.too_many_abandoned`) enquanto `MAX_STUCK_DEVICES` (3) devices
  abandonados ainda estiverem presos; o Dummy nunca é recusado.
- O `CallbackHandle` é a única forma de o backend chamar o render, e conta a entrada e a saída.
- O tamanho máximo de bloco vem do período **negociado** com o dispositivo, não do pedido (C7).

### I-10.2 Entrada MIDI (F1)

`MidiInput` tem a thread do backend e o próprio anel de comandos. O `stop` espera a thread sair. Varrer portas é uma
`Operation` (⏳).

### I-10.3 Trocar de placa (F1) ⏳

É uma transação nas preferências. Em seguida, uma `Operation` no alvo `Device`:
1. `stop` (barreira);
2. abrir o novo dispositivo;
3. reamostrador de borda, se a taxa for diferente da do projeto;
4. `start` com o mesmo `Renderer`.

### I-10.4 Calibração (F1) ⏳

Uma ação que é uma `Operation`: um programa de medição (impulso e escuta), cinco medidas, a mediana guardada nas
preferências do dispositivo, **por taxa de amostragem e por reamostrador de borda ligado ou não** (H19). Gravar numa
combinação ainda não calibrada usa a latência informada pelo dispositivo e avisa.

## I-11. Gravação (F1) (R4)

```rust
pub fn recording_channel(seconds: f32, channels: usize, rate: u32) -> (RecordSender, RecordReceiver);
/// lm-io knows inputs and files, not the document (T2); the Session turns it into the track's item or lane
pub struct Take { pub input: InputIndex, pub path: PathBuf, pub start_sample: i64, pub frames: i64 }
```

- O anel é só da gravação, com a thread de disco como único consumidor, dimensionado em segundos (padrão 10).
- **Cheio = erro explícito**, nunca perda silenciosa:
  1. aquela entrada para de gravar;
  2. a telemetria manda `RecordingOverflow`;
  3. o arquivo é fechado até o último bloco bom;
  4. o usuário vê o tempo exato.
- **Num xrun de captura**, o buraco é preenchido com silêncio do tamanho exato e marcado; o take nunca sai da grade
  (C2).
- **Nome do arquivo:** provisório (`rec-<uuid>`) enquanto grava, e o hash BLAKE3 ao fechar (C10).
- **Ao parar:** fechar o arquivo e gerar os picos é uma `Operation` no alvo do take (⏳). O take aparece no lugar
  imediatamente, marcado carregando, e entra na árvore numa transação `Origin::Recording`, com a latência descontada.
- Ao abrir um projeto com gravações interrompidas, recuperá-las é uma `Operation` oferecida ao usuário.
- **Take de uma faixa que sumiu** (apagada ou desarmada durante a gravação, H8): o take nunca é descartado. Vira um item
  numa faixa nova, "Take recuperado", com aviso.
- **Espaço em disco** (H3): ao armar, a `Session` confere o espaço livre e avisa se não couberem 10 minutos de todas
  as entradas armadas. Disco cheio durante a gravação é tratado como o anel cheio: erro explícito, arquivo fechado até
  o último bloco bom.

## I-12. Recursos (F1) ⏳

```rust
// lm-io: the pure work and the state of what is loaded (T1)
pub fn import_file(source: &Path, project: &Path, rate: u32, cancel: &Cancel, progress: &Progress)
    -> Result<ImportedFile>;                            // runs inside an Operation
pub fn load_file(path: &Path, cancel: &Cancel, progress: &Progress) -> Result<Arc<SampleData>>;
impl ResourceStore {
    pub fn view(&self) -> ResourceView;                 // immutable: what is loaded now
    pub fn peaks(&self, key: &str) -> Option<Arc<Peaks>>;
    pub fn loaded(&mut self, key: String, data: Arc<SampleData>, peaks: Arc<Peaks>);
}

// lm-session: starting the work is the Session's (T1)
impl Session {
    pub fn import(&mut self, path: PathBuf) -> OperationHandle<Resource>;
    pub fn request(&mut self, r: &Resource);            // loads in an Operation targeted at the resource
}
```

- Um nó cujo recurso não carregou é compilado como **"esperando" (silêncio)**. O alvo `Node` daquele nó mostra o
  carregamento. Quando o recurso fica pronto, a `Session` manda recompilar.
- Os picos são gerados no carregamento e gravados em `audio/<hash>.peaks`, em níveis de zoom (o formato está no
  I-20.4). A interface nunca lê o áudio para
  desenhar; enquanto os picos não existem, ela mostra o carregamento no lugar da forma de onda.
- Um `Arc<SampleData>` que entra no áudio volta pela devolução (regra 5).
- Recursos que nenhum programa usa há 5 minutos são descartados (C8). Chaves e caminhos são validados (C9).
- **Nenhum arquivo de `audio/` é apagado automaticamente** enquanto alguma raiz do histórico ou do diário o cita (H4).

## I-13. Exportação (F1) ⏳

```rust
pub fn export(root: Value, resources: ResourceView, plugins: Arc<Registry>, target: ExportTarget,
    cancel: &Cancel, progress: &Progress) -> Result<ExportReport>;
```

- É uma função sobre uma cópia da árvore, com o próprio compilador e o próprio `Renderer`, sem dispositivo. Roda numa
  `Operation`, com progresso e cancelamento.
- Não toca na sessão nem no transporte. Exportar enquanto grava é seguro.
- Plugins de terceiros sem prazo (S19).
- **Espera tudo ficar pronto** (recursos carregados, plugins construídos) antes do primeiro bloco. "Esperando" nunca
  vira silêncio no arquivo (T8).
- **O `lm-headless render` é exatamente este caminho** (T19). Ele é o teste de referência do render determinístico.

## I-14. Avisos (F1)

```rust
pub struct Notice {
    pub key: &'static str, pub args: Vec<(&'static str, Value)>, pub severity: Severity,
    pub target: Option<Target>, pub actions: Vec<ActionId>,     // "try again", "recreate the plugin"
}
```

Fila da `Session`. A interface mostra; o `lm-headless` imprime. O núcleo nunca escreve texto pronto.

## I-15. A fronteira com a interface gráfica: o `CorePort` (F1) (S3, D6)

**Decisão do usuário (19/11): o LinuxMusicBeta funciona como servidor e como cliente de rede.** No modo local, a
interface e o núcleo estão no mesmo processo e conversam direto. No modo cliente, a interface roda num computador e
o núcleo noutro (o `lm-headless serve`, I-20.7), que é quem toca e grava; a interface conversa com ele pela rede. O
som sai sempre na máquina do núcleo (não há áudio pela rede).

**Uma interface só, uma porta com duas implementações:**

```rust
/// Everything the GUI may do with the core. Arguments and results are DATA (no closures, no references into the
/// core), so the same calls work in-process and over the network
pub trait CorePort {
    fn pump(&mut self);                                        // each frame and on the 10 ms timer (H2)
    // Reading: from the port's mirror of the tree (in-process: the document's own root)
    fn root(&self) -> &Value;
    fn find(&self, id: NodeId) -> Option<Value>;
    fn feed(&mut self, topic: Topic) -> Feed;                  // pulled each frame (I-3.6)
    // Changing: edits as data (EditOp, below), actions by id, gestures by id
    fn apply(&mut self, label: Label, ops: Vec<EditOp>) -> Pending<Applied>;
    fn run_action(&mut self, action: ActionId, params: Value) -> Pending<()>;
    fn undo(&mut self) -> Pending<()>;
    fn redo(&mut self) -> Pending<()>;
    fn begin_gesture(&mut self, label: Label) -> GestureId;
    fn gesture_apply(&mut self, g: &GestureId, ops: Vec<EditOp>);
    fn gesture_commit(&mut self, g: GestureId);
    fn gesture_cancel(&mut self, g: GestureId);
    fn param_write(&mut self, node: NodeId, param: &str, value: f32);   // live, while dragging
    // Live notes (the test keyboard, the computer keyboard, the piano roll): data only, never waits, no answer. A
    // MIDI command to the track's instrument on the track's own port, like live MIDI input; the interface releases
    // every note it started (mouse-up, focus loss, window closed)
    fn note_on(&mut self, track: NodeId, key: u8, velocity: u8);
    fn note_off(&mut self, track: NodeId, key: u8);
    fn all_notes_off(&mut self, track: NodeId);
    // Long operations, notices, telemetry, heavy data
    fn loading(&self, target: &Target) -> Loading;
    fn activity(&self) -> Vec<OperationInfo>;
    fn cancel(&mut self, op: OperationId);
    fn notices(&mut self) -> Vec<Notice>;
    fn telemetry(&self) -> &TelemetryView;                     // meters and position, interpolated
    fn peaks(&mut self, resource: &Resource, level: u8, from: i64, to: i64) -> Pending<PeakBlock>;
    fn display(&self, node: NodeId) -> Option<DisplayView>;   // a plugin's live data (F2)
}

/// A result that may not be there yet: in-process it is ready at once; over the network it arrives in a later pump
pub enum Pending<T> { Ready(Result<T>), Waiting(RequestId) }
```

- **`LocalPort`** (F1): dona da `Session`, no mesmo processo. Cada método é uma chamada Rust direta com os valores
  Rust, **sem serializar nada**: a eficiência de "conversar por código" fica toda. Por dentro, ela transforma uma
  `EditOp` numa `transact` com fechamento; o código do núcleo (as ações, as edições pesadas, os testes) continua
  usando fechamentos.
- **`RemotePort`** (F2): os mesmos dados, serializados pelo protocolo (I-20). Ela mantém um **espelho da árvore**,
  atualizado pelas diferenças que o servidor manda, então ler (`root`, `find`, os `Refs`, os `Feed`s) é local e
  imediato também no modo cliente. Só as mudanças esperam a resposta do servidor (`Pending`); o arrasto de um botão
  vai pela `param_write` e pelo gesto sem esperar ninguém.
- **A interface só conhece o `CorePort`.** Ela não guarda ponteiro para nada do núcleo: só ids, `Feed`s,
  `GestureId`s, `OperationId`s e `Pending`s. **Nenhum método da porta recebe fechamento**: é isso que permite a
  mesma interface nos dois modos (este ponto substitui a regra anterior de "a interface fala por fechamentos").

```rust
/// One edit as data (the port's and the protocol's form; the core turns a list into one transact)
pub enum EditOp {
    Set { id: Ref, path: Vec<PathStep>, value: Value },
    Insert { temp: u32, parent: Ref, list: String, index: usize, node: Value },
    Remove { id: Ref },
    Move { id: Ref, parent: Ref, list: String, index: usize },
    AddNote { temp: u32, item: Ref, note: Value },
    SetNote { item: Ref, n: NoteRef, path: Vec<PathStep>, value: Value },
    RemoveNote { item: Ref, n: NoteRef },
}
pub enum Ref { Id(NodeId), Temp(u32) }          // Temp: a node created earlier in the same list
pub enum NoteRef { N(u64), Temp(u32) }
pub struct Applied { pub diff: Diff, pub created: Vec<(u32, NodeId)>, pub notes: Vec<(u32, u64)> }
```

- Edições pesadas (quantizar, colar) são ações com parâmetros (`"notes.quantize" { item, grid }`), que rodam no
  núcleo (I-19, H7).
- **A biblioteca é a GPUI.** A porta vive numa entidade da GPUI, e o `pump` roda num timer do laço de eventos dela.
- **Acréscimos de 20/11 (pacote `lm-port`, decididos no protótipo):**
  - `drain(&self, &mut Feed) -> Option<Diff>`: um `Feed` drena contra o documento, que a interface nunca tem na mão;
  - `modified()`, `can_undo()`, `can_redo()`: para perguntar antes de perder mudanças e acender desfazer e refazer;
  - `dismiss_notice(id)`;
  - `finished()`: os resultados dos pedidos que responderam `Waiting`, coletados a cada `pump`;
  - `import(path)`: importar um arquivo de áudio para os recursos do projeto (I-12); no cliente, a `RemotePort` envia
    o arquivo do cliente em pedaços, retomável e conferido pelo hash, e o servidor importa o envio;
  - o lado da sessão que não é o documento do projeto: `preferences()` e `set_preference(path, value)` (I-16),
    `status()` (o arquivo do projeto e o acesso H10, os projetos recentes, os modelos, os takes interrompidos U5, o
    modo seguro U21, o relatório da última exportação, como uma árvore de `Value`) e `devices()` (`Pending`);
  - **o serviço do navegador**: `browse(page, parent, show_hidden)`, `search(query)` (em segundo plano; uma busca nova
    substitui a anterior; o carregamento aparece em `Target::Scan(Library)`), `prepare_preview(entry)` (a forma de onda
    da prévia vem por `peaks` no recurso da prévia, I-7.3), `preview_play`, `preview_stop` e `set_favorite`. No modo
    cliente os arquivos são do servidor, então o navegador também passa pela porta;
  - o `Notice` da porta leva o seu número, as ações com o parâmetro de cada uma, as repetições e se foi lido; a
    `TelemetryView` é por nó (`NodeId`), com a posição e o carimbo de tempo, as entradas, os takes que crescem, a
    contagem e os xruns com as suas posições;
  - `GestureId::from_raw(n)` no `lm-doc`: a `RemotePort` nomeia um gesto aberto no servidor pelo número; um número que
    o documento não deu é recusado com erro, nunca com pânico;
  - `plugins()` (`PluginInfo`: id, nome, instrumento ou efeito, categoria, terceiros, latência; os mesmos ids das
    páginas de plugins do navegador) e `plugin_schema(id)` (o esquema principal, de que o editor genérico de plugin
    monta os controles); toda porta de verdade responde (o falso pelo registro dele, a `LocalPort` pelos plugins
    internos, a `RemotePort` pelo que buscou ao conectar).
  - `preferences().monitor` = { level (dB), dim, cut, mono, side }: a monitoração do master (o MONITOR do legado),
    que é da máquina e não entra no documento, no desfazer nem na exportação (`lm-port/src/trees.rs`);
  - **o mapeamento MIDI (I-21.2)**: `midi_learn(target, scope) -> Pending<LearnId>`, `cancel_learn(id)`,
    `learning()` (a interface mostra "mova um controle"), `unmap(target, scope)` ("Tirar o mapa"),
    `drain_midi_ui(&mut Vec<MidiUiAction>)` (o `Feed` das ações da interface mapeadas) e
    `midi_activity(since) -> (u64, Vec<MidiActivity>)` (a atividade crua, com perda, as últimas 256); os tipos do
    mapa (`MapTarget`, `NodeRef`, `MapScope`, `Mapping`, `DocSource`, `Message`) vêm do `lm-midimap` e o `lm-port` os
    reexporta. `TelemetryView::hand_won` lista as pistas em que a mão venceu (o dono e a chave do parâmetro), até o
    `automation.rearm`. As implementações padrão são só da transição: toda porta de verdade responde;
  - **o documento da interface (I-3.8)**: `ui()` e `set_ui(path, value)`, sem desfazer e fora do diário. A interface
    escreve ali a seleção, `selection: { track, channel, plugin }` (a faixa em foco, o canal selecionado na mesa, o
    plugin com a janela aberta), que os alvos MIDI que seguem a seleção leem (I-21.2); a forma está em
    `lm-port/src/trees.rs`.
- **Testes:** o mesmo roteiro de interface roda contra a `LocalPort` e contra a `RemotePort` ligada a um servidor no
  mesmo processo (por um socket local), e tem que dar a mesma árvore e os mesmos `Diff`s.
- **O protótipo** é escrito contra um núcleo de mentira que já tem a forma do `CorePort` (dados nos argumentos, sem
  fechamentos), para a interface de verdade nascer sobre ele.

- **A lei dos valores (20/11):** `CorePort::param_range(nó, chave) -> Option<ParamRange>` devolve a faixa e a curva de
  um parâmetro (linear, log, em degraus, ganho), a mesma que o compilador usa para o MIDI mapeado e a automação e que a
  `Session` usa nas edições de automação. `ParamRange` e `RangeScale` moram no `lm-base` (puro) e são reexportados pelo
  `lm-render` e pelo `lm-port`. A interface desenha e edita só com ela; sem resposta, a lane não é desenhada, e uma
  ação recusada (`automation.scale`, `automation.thin`) aparece como erro, sem conta feita na interface.

## I-16. A sessão (F1) (R6, S15, S16)

```rust
pub struct Session {
    // Fields drop in declaration order: what must die first comes first
    device: Option<Device>,          // its Drop runs the barrier before anything else goes (S16)
    midi: Vec<MidiInput>,
    disk: DiskWriter, journal: Journal, autosave: Autosave,
    compiler: Compiler, engine: EngineControl,
    commands: CommandSender, telemetry: TelemetryReceiver,
    actions: Actions,                // no references: gets an ActionContext per run (R6)
    activity: Activity,              // the operations (I-2)
    ui_document: Document,           // I-3.8
    document: Document,
    resources: ResourceStore,
    plugins: Registry, schemas: SchemaRegistry, preferences: Preferences, workers: Workers,
    main: Main,                      // last: the session is !Send because of it
}
```

**Quem chama o `pump`** (H2): uma thread-relógio própria da `Session` acorda a thread principal a cada 10 ms (pelo laço
de eventos da biblioteca de interface, ou pelo laço do `lm-headless`), independente de desenhar. Uma janela
minimizada ou escondida não para a coleta da devolução, os takes nem os carregamentos.

**`Session::pump()`**, nesta ordem (T17):
1. coleta a devolução do áudio e libera índices no compilador;
2. lê o quadro de telemetria e os anéis de eventos (com as tabelas por geração);
3. trata `PreviewEnded`, os pedidos dos plugins e os takes;
4. atualiza as operações (progresso, fim, falha) e entrega ao compilador os processadores construídos;
5. confirma os gestos largados (T4) e esquece os `Feed`s largados (T13);
6. aplica as vagas do desfazer e do refazer;
7. junta as diferenças desde o último `pump` e as entrega aos `Feed`s;
8. manda compilar se a raiz mudou de forma;
9. instala as `Install` prontas;
10. agenda o diário (gestos só no commit ou a cada 1 s, T11) e o autosave.

**Os workers na desmontagem** (T18): o `Drop` dos workers cancela as operações abertas e espera todas terminarem
(join). Como uma `Operation` nunca toca a `Session`, a ordem dos campos continua valendo: o resultado que chega tarde
cai num `OperationHandle` que ninguém mais lê.

Abrir, salvar, exportar e trocar de dispositivo são `Operation`s; a `Session` nunca bloqueia nelas.

## I-17. Ligações com C, crates sem trava e verificação (F1)

**A ficha de cada `lm-sys-*`** (no topo do `lib.rs`, preenchida e testada):

| Pergunta | Exigido |
|---|---|
| Quem é dono de cada handle? | um tipo com `Drop`, um dono só |
| Em que thread cada função pode ser chamada? | pelos tipos (`!Send`, ou só pela `Session`) |
| Aloca, trava, faz E/S? | por função; o que faz nunca é chamado pelo processador |
| Cria threads? | quais, e como param **antes** de liberar o que usam |
| Chama de volta o nosso código? | por trampolina com `catch_unwind` |
| Testes | do pacote, sob ASan, mais a armadilha de alocação no que roda no áudio |

**Os primeiros pacotes `lm-sys`** (F1, Linux): `lm-sys-alsa` (PCM e seq), `lm-sys-sndfile`, `lm-sys-fluidsynth` (apagado em 27/11),
`lm-sys-samplerate`, `lm-sys-fpu`, `lm-sys-linux` (rtkit, logind e ReserveDevice1 pelo D-Bus, `mlockall`). Depois:
`lm-sys-jack`, `lm-sys-lilv`. O Mac (`lm-sys-coreaudio`, `lm-sys-coremidi`, `lm-sys-macos`) vem em segundo.

**Outras crates permitidas no núcleo:** `blake3` (C10).

**Crates sem trava permitidas** (o resto do núcleo não tem `unsafe`, S8): `rtrb` (anéis SPSC), `triple_buffer`,
`assert_no_alloc`, `crossbeam-utils` (só `CachePadded`) e os atômicos da biblioteca padrão. Uma crate nova nessa
lista exige decisão escrita aqui. O `cargo-deny` confere a lista e as licenças (compatíveis com GPL-2.0-ou-posterior).

**Verificação:**
- `loom` para o par instalação/devolução e para as vagas com bitset sujo;
- `miri` nos testes do `lm-base` e do `lm-state`;
- ASan e TSan nos testes que usam `lm-sys`.

## I-18. Tempo (F1) (R13)

```rust
pub struct TempoMap { /* segments of constant tempo, each with exact start_sample and start_tick */ }
impl TempoMap {
    pub fn tick_at(&self, sample: i64) -> i64;     // i128 arithmetic within the segment, floor
    pub fn sample_at(&self, tick: i64) -> i64;     // i128, round-half-even
    pub fn segments_in(&self, from: i64, to: i64) -> impl Iterator<Item = Segment>;
}
```

- A posição canônica é em amostras, e o tique é sempre recalculado.
- O `bpm` é racional.
- O bloco é dividido nas mudanças de andamento.
- As rampas de andamento ficam para F2, com a regra de arredondamento escrita e testada antes.

## I-19. Ações (F1) (D13)

```rust
pub struct ActionSpec {
    pub id: ActionId,                      // "track.add", "transport.record"
    pub label: Label,
    pub params: Option<&'static str>,      // the schema type of its parameters
    pub run: fn(&mut ActionContext, &Value) -> Result<()>,
    pub enabled: fn(&ActionContext) -> bool,
}

/// What an action may use, lent for the call. It holds no references after the call
pub struct ActionContext<'s> { /* transact, undo/redo, commands (transport), start(Operation), notices, find */ }
```

- Menus, atalhos, barra de ferramentas, paleta de comandos e a API externa (F2) são vistas do registro de ações.
- Uma ação que pode passar de 200 ms inicia uma `Operation` e volta na hora.

---

## I-20. Controle e gerência remotos (F2)

**O protocolo de rede é a `RemotePort` do I-15** (decisão do usuário, 19/11): a interface gráfica remota usa
exatamente a superfície do `CorePort`. Ele tem dois públicos:
- **a interface gráfica remota**, que usa a porta inteira e exige a **mesma versão** do programa no cliente e no
  servidor (o aperto de mão confere; versões diferentes recusam com mensagem clara);
- **ferramentas externas** (scripts, painéis, superfícies de controle, agentes de IA), que usam um **subconjunto
  estável e versionado** (ações, edições, gerência, assinaturas e telemetria em taxa reduzida), com compatibilidade
  escrita entre versões.

O `lm-remote` traduz cada mensagem para chamadas da `LocalPort` do servidor, e é por isso que o comportamento é o
mesmo nos dois modos. Papéis e limites (I-20.6) valem para os dois públicos; a interface gráfica remota pede o papel
"administrar".

### I-20.1 O pacote e a thread

```
lm-remote    o servidor do protocolo                                            → lm-session
```

- O servidor tem threads próprias de rede, que **nunca tocam a `Session`** (ela é `!Send`). Cada pedido vira uma
  mensagem numa fila para a thread principal, atendida no `pump`; a resposta volta por um canal para a thread de
  rede.
- A rede nunca toca o áudio. Escrita de parâmetro remota passa pelas mesmas vagas (I-6), com taxa limitada. Notas
  vindas de um cliente remoto entram por um anel de produtor próprio (I-7.1), com o aviso de que a rede acrescenta
  tremor.
- **Perder a conexão nunca para a reprodução nem a gravação.**

### I-20.2 Transporte e formato

- **Socket de domínio Unix**, local, ligado por padrão no `lm-headless serve` e desligado por padrão no programa com
  interface. Permissão `0600`: só o mesmo usuário conecta.
- **TCP com TLS**, desligado por padrão; ligar é uma escolha explícita nas preferências, com pareamento por token.
  (Decidido em 20/11, `decisoes-autonomas.md`: o TCP escuta em 127.0.0.1 e outra máquina chega por túnel SSH, que
  autentica e cifra; TLS no programa fica para depois, sem mudar o protocolo.)
- Quadros com tamanho na frente. **Mensagens em JSON** (legíveis, fáceis de depurar e de usar por script); a
  telemetria pode pedir uma codificação binária (CBOR) para economizar banda.
- **Aperto de mão com versão do protocolo.** Cliente e servidor de versões incompatíveis recusam com mensagem clara.

### I-20.3 O que o protocolo oferece (a superfície do I-15, mais a gerência)

| Grupo | Pedidos |
|---|---|
| Ler | `find(id)`, ler um pedaço da árvore por caminho, os `Refs` como objetos |
| Mudar | `apply` (uma lista de `EditOp`, abaixo), `undo`, `redo`, `run_action(id, params)` |
| Gestos | `begin_gesture`, `gesture_apply` (lista de `EditOp`), `gesture_commit`, `gesture_cancel` |
| Assinar | `feed(topic)` → fluxo de `Diff`s; telemetria (posição, medidores) na taxa que o cliente pedir, até 60 Hz; atividade e carregamentos (I-2); avisos |
| Dados grandes | picos de um recurso por intervalo; enviar um arquivo para importar (vira uma `Operation` de importação) |
| Gerência | listar, abrir, criar, salvar e fechar projetos; estado da sessão (tocando, gravando, xruns, carga do render, espaço em disco); dispositivos e preferências; a sequência de saída (U13); registro de eventos; um `health` para monitoramento |

**As edições no protocolo** são as `EditOp` do I-15.

### I-20.4 Dados pesados: formas de onda, medidores, displays de plugin

A regra: **nunca se manda áudio para desenhar.** Cada tipo de dado pesado tem a sua forma, pensada para a tela. Os
dados são os mesmos para a interface local e para o cliente remoto (o arquivo de picos, os valores dos medidores);
o que muda é o caminho: a interface local lê direto, e o protocolo manda pedaços.

**Formas de onda: picos em níveis de zoom, imutáveis, pelo hash.**
- Ao carregar ou gravar um recurso, o núcleo gera um arquivo `audio/<hash>.peaks` com vários níveis (uma "pirâmide",
  como os mipmaps de textura): o nível 0 tem um par (mínimo, máximo) a cada 64 amostras por canal; cada nível seguinte
  junta 4 do anterior (256, 1024, 4096... amostras por par). Os valores são **`i16`**, com um cabeçalho versionado.
  Com `i8` (256 degraus), um take gravado com folga, com picos em torno de −30 dBFS, ocuparia uns 8 degraus e
  apareceria achatado e serrilhado no zoom vertical, justamente o caso comum na gravação (revisão R-b).
- **Quanto pesa:** 1 hora estéreo a 48 kHz tem 172,8 milhões de amostras por canal. O nível de 1024 amostras por par
  tem ~169 mil pares por canal, ou ~1,35 MB para os dois canais. O arquivo inteiro, com todos os níveis, fica em torno
  de 1/16 do áudio em `f32`.
- **O que a tela pede:** uma faixa de 2000 pixels mostra 2000 pares por canal no nível certo para o zoom: ~16 KB por
  faixa estéreo. 64 faixas na tela: ~1 MB, **uma vez**. Rolar e dar zoom pedem só os pedaços novos.
- **O pedido:** `peaks(hash, level, from, to)`. Como o conteúdo de um hash nunca muda, o cliente **guarda o que
  recebeu para sempre** (num cache em disco, pelo hash) e nunca pede de novo. Um projeto reaberto no mesmo cliente
  desenha sem tráfego.
- **Interface local:** lê o mesmo arquivo direto (mapeado em memória) e entrega à GPU; nenhum caminho especial.

**Forma de onda crescendo durante a gravação** (U3): os picos novos chegam como **anexos** a cada ~100 ms: só os pares
do nível 0 do trecho recém-gravado, por entrada armada (~3 KB/s por canal), e os níveis de cima são completados pelo
cliente. Ao parar, o `.peaks` definitivo tem o mesmo conteúdo, então o cliente não precisa baixar de novo.

**Medidores e cursor:** um quadro por envio, na taxa que o cliente pediu (até 60 Hz, padrão 30 Hz), só com as faixas
que ele assinou (as visíveis): pico e RMS por canal em `u16` (dB quantizado). 64 faixas estéreo a 30 Hz: ~15 KB/s. A
posição vai junto com o carimbo de tempo, e o cliente interpola entre quadros para o cursor andar liso.

**Displays de plugin** (osciloscópio, espectro, I-9.5): **só enquanto a janela está aberta** no cliente, que assina e
cancela. O plugin declara o tamanho máximo do quadro; o servidor manda o quadro mais recente na taxa pedida (padrão
30 Hz), sem fila: quadro velho é descartado, nunca acumulado. Um espectro de 512 faixas em `u8` a 30 Hz: ~15 KB/s.

**Ouvir remotamente:** não há. Sem backend de áudio em rede (decisão do usuário), o cliente remoto vê e controla, e
quem ouve está junto da máquina que toca.

**Rede lenta:** cada fluxo contínuo (medidores, displays, posição) tem prioridade menor que as respostas de pedidos e
os `Diff`s; numa rede congestionada, a taxa dos quadros cai sozinha (o servidor só manda o próximo quando o anterior
saiu), e nunca há fila crescendo.

### I-20.5 Vários clientes ao mesmo tempo

- Todas as transações, de todos os clientes e da interface local, são **serializadas na thread principal**, em ordem
  de chegada. **Um histórico só**; cada passo leva o nome do cliente que o fez, e o desfazer é global (como numa
  mesa compartilhada).
- **Gestos têm dono:** cada gesto é do cliente que o abriu. Se a conexão cai com um gesto aberto, ele é **cancelado**
  (G1: só os caminhos dele voltam).
- Um `Feed` ou uma assinatura de telemetria de um cliente que caiu é descartado.

### I-20.6 Segurança

- **Papéis por cliente**, dados no pareamento:
  - **observar**: ler, assinar, telemetria;
  - **controlar**: o anterior, mais transações, ações e transporte;
  - **administrar**: o anterior, mais abrir, salvar e fechar projetos, dispositivos, preferências e sair.
- O protocolo **nunca executa código** do cliente e **nunca dá acesso a caminhos** fora das pastas permitidas (a pasta
  de projetos e a de importação). Importar de fora é enviar o arquivo, não apontar um caminho do servidor.
- O decodificador do protocolo passa pelo mesmo fuzzing do JSON (Especificação da base, item 10): nenhuma mensagem
  derruba o servidor.
- Limites por cliente: tamanho de mensagem, taxa de pedidos, número de assinaturas.

### I-20.7 O `lm-headless` como serviço

`lm-headless serve [--socket <caminho>] [--tcp <porta>]` roda o núcleo sem interface como um serviço, gerenciado só
pelo protocolo: abre projetos, toca, grava, exporta e informa o estado. É o caso "um computador de gravação na sala,
controlado de outro", e também o jeito de testar o protocolo no CI.

### I-20.8 Pronto quando (F2)

- Para cada pedido do protocolo, um teste confere que ele produz a mesma árvore que a chamada interna equivalente (o
  tradutor não inventa comportamento).
- Dois clientes editando ao mesmo tempo: o histórico fica coerente e cada passo tem o nome do cliente.
- Cair a conexão no meio de um gesto cancela só aquele gesto; cair no meio de uma gravação não a para.
- Fuzzing do decodificador por 1 h sem queda.
- Um projeto de 64 faixas de 1 hora abre num cliente remoto com o cache vazio e mostra a tela inicial com menos de
  2 MB transferidos; reaberto, com zero de formas de onda transferidas.
- Um cliente com papel "observar" não consegue mudar nada (testado para cada pedido).

## I-21. MIDI e automação (F1, tudo na 1.0)

**Decisões do usuário (20/11):**
- **Tudo é mapeável por MIDI:** todo parâmetro, todo campo e toda ação.
- **Na 1.0 entram:** o mapeamento, o retorno MIDI (LEDs, faders motorizados), as interfaces MIDI, a gravação MIDI
  completa e a gravação e a edição de automação.
- Esta seção junta e completa o que estava espalhado:
  - na especificação: P-15, P-16 e P-38, a I-6.4 e a I-10.2;
  - no inventário: F-CTL-004, F-REC-100 a 104 e 130/131, F-AUT-001 a 011, F-IJN-017 e 025, F-NOT-018 e F-PAS-001.
- **Em conflito com aqueles itens, vale esta seção.**

**A regra de arquitetura:** nenhum recurso precisa de código próprio para ser mapeável. O mapa aponta para os mesmos
três tipos de alvo que já existem no núcleo:
- os parâmetros (I-6);
- os campos do documento (I-3);
- as ações do registro (I-19, incluídas as ações da interface).

Um parâmetro, campo ou ação novo nasce mapeável. **Teste gerado:** percorre todos os esquemas e o registro inteiro e
mapeia cada alvo.

### I-21.1 Interfaces MIDI (dispositivos)

- **Backends:**
  - ALSA seq: o primeiro, núcleo;
  - ALSA raw MIDI (P-16): para placas que o seq não expõe bem;
  - JACK MIDI: junto com o JACK;
  - CoreMIDI: no macOS, depois.
- **Uma porta tem identidade estável:** nome do cliente + nome da porta, nunca o número do ALSA, que muda a cada
  ligação.
  - As preferências da máquina dão a cada porta:
    - entrada e saída ligadas;
    - o papel: **instrumento**, **controlador** ou os dois;
    - a latência de entrada (P-15);
    - um **apelido** ("Teclado", "Controlador 1").
  - O projeto guarda só apelidos (P-36): um projeto aberto noutra máquina pede, uma vez, a porta de cada apelido.
- **Ligar e desligar a quente:** os anúncios do seq (porta nova, porta que sumiu) chegam como eventos.
  - O que usava uma porta que sumiu mostra "desconectado" e continua no projeto.
  - Quando a porta volta, tudo religa sozinho, com um aviso (I-14).
- **Portas próprias:** o programa expõe "LinuxMusicBeta entrada" e "LinuxMusicBeta saída" no seq, para outros
  programas ligarem nele.
- **Entrada:**
  - Cada evento chega com o carimbo do relógio da fila do seq, convertido para `CLOCK_MONOTONIC` (C6), menos a
    latência da porta.
  - O `Renderer` põe o evento na amostra exata **com atraso fixo de um bloco**: sem jitter, a mesma latência sempre,
    e a gravação fica onde foi tocada.
  - Mensagens aceitas:
    - notas, CC (7 bits e pares de 14 bits), NRPN/RPN, pitch bend, aftertouch de canal e polifônico;
    - program change;
    - SysEx: só passa para plugins e para o retorno de perfis de controlador; não grava.
- **Saída** (`MidiOutput`):
  - Uma thread por backend, alimentada pelo `Renderer` por um anel com carimbos.
  - Os eventos vão para a fila do seq com o horário marcado, então saem na hora exata, não na hora do bloco.
  - Servem a esta saída: a saída MIDI de faixas para aparelhos externos (P-38), o retorno (I-21.3) e a compensação de
    latência de saída por porta.
- **Entrada da faixa** (F-REC-103, F-REC-131):
  - portas (lista ou "todas as de papel instrumento") e filtro de canal;
  - "seguir a seleção": o teclado toca a faixa selecionada.
  - Uma faixa armada ou em monitoramento recebe.
- **Ponte MIDI do sistema:** fora. Não há roteamento de MIDI entre faixas na 1.0 (P-38).
- **MIDI Clock e MTC:** depois da 1.0. O legado não tem.

### I-21.2 Mapeamento

```rust
/// One mapping: a source on a port, a transform, a target. Lives in the project (`project.midiMap`), or in the
/// machine's preferences for mappings that hold in every project (the same schema). The project wins on a conflict
pub struct Mapping {
    pub source: Source,        // port alias (or Any) + channel + message
    pub target: MapTarget,
    pub transform: Transform,
    pub feedback: bool,        // send the target's value back (I-21.3); on by default when the port has an output
    pub consume: bool,         // the message does not also reach instruments (default true)
}

pub enum Message {
    Cc { number: u8 }, Cc14 { msb: u8 },         // 14-bit pair: msb and msb+32
    Nrpn { number: u16 }, Rpn { number: u16 },   // 14-bit
    Note { key: u8 }, PitchBend, ChannelPressure, PolyPressure { key: u8 }, ProgramChange,
}

pub enum MapTarget {
    Param { node: NodeRef, key: String },         // any parameter: plugin, rack section, fader, send, pan, master
    Field { node: NodeRef, path: Vec<PathStep> }, // switches and choices (mute, solo, arm, monitor, a wave shape)
    Action { id: ActionId, params: Value, value_into: Option<Vec<PathStep>> }, // any registered action
}

/// A fixed node, or one that follows the selection (a control surface works on "the selected track")
pub enum NodeRef {
    Id(NodeId),
    Selected { role: SelRole, offset: u32 },     // SelRole: Track, Channel, Plugin (the focused one); offset: banks
}

pub struct Transform {
    pub min: f32, pub max: f32, pub invert: bool, pub curve: Curve,     // in the target's normalized range
    pub mode: Mode,         // Absolute, Relative(Encoding: TwosComplement | SignMagnitude | Offset64), with a step
    pub takeover: Takeover, // Jump, Pickup (waits until the control passes the value), Scale (converges)
    pub button: Button,     // for Note and on/off CCs: Momentary, Toggle, Trigger (on press)
}
```

- **Onde o mapa mora:** no projeto (`project.midiMap`), porque o botão tem que voltar a obedecer quando o projeto
  abre.
  - Os mapas "de máquina" (transporte, desfazer, os botões de um controlador que valem em todo projeto) ficam nas
    preferências, com o mesmo esquema.
  - O alvo de um mapa do projeto é uma referência com `OnRemove::RemoveOwner`: o mapa some com o alvo e volta com o
    desfazer.
- **Um controle pode mover vários alvos**, que é o "controle compartilhado" de P-04/P-33. **Um alvo pode ter vários
  controles**: vale o último que mexeu.
- **O caminho em tempo real** (parâmetros):
  - o compilador compila o mapa numa tabela fixa, na `Install` (I-5): a fonte vira a vaga, já com a faixa e a curva;
  - o estado de cada mapa (metade de um par de 14 bits, pickup, toggle) mora em memória alocada na compilação;
  - o `Renderer`, ao receber o evento, gera o `Param` **na amostra do evento**, marca a vaga como tocada (a mão vence
    a automação, I-6.2) e escreve o valor na vaga;
  - não passa pela thread principal nem aloca;
  - a telemetria manda as vagas mexidas pelo MIDI (I-6.4).
- **Fim do gesto:** o gesto MIDI termina 500 ms depois da última mensagem, ou com o toque solto num fader sensível ao
  toque (I-21.3). Aí a `Session` grava o valor final no documento: um passo de desfazer por gesto, com rótulo
  "MIDI: <alvo>".
- **Campos e ações:**
  - não são tempo real: o `Renderer` manda um evento pelo anel sem perda, e a `Session` aplica no próximo `pump`
    (≤ 10 ms);
  - um campo vira uma transação;
  - uma ação roda pelo registro, com `value_into` levando o valor do controle para dentro dos parâmetros da ação (por
    exemplo, o program change escolhe o preset de número N);
  - as ações da interface (zoom, rolar, trocar de página) chegam à GUI pelo `Feed` `MidiUiAction`.
- **Alvos que seguem a seleção:** quando a seleção muda (no documento da interface, I-3.8), a `Session` manda
  recompilar só a tabela do mapa. Isso é barato.
- **Aprender** (F-CTL-004), pelo `CorePort`: `midi_learn(target) -> Pending<LearnId>`, que tem cancelamento.
  - A `Session` olha a atividade MIDI crua (um `Feed` com perda, que também alimenta o monitor de atividade da
    interface).
  - O primeiro movimento define a fonte. Nos 300 ms seguintes, ela detecta:
    - o par de 14 bits;
    - o NRPN;
    - o encoder relativo: só valores perto de 1/127 ou 63/65.
  - Depois cria o mapa numa transação.
  - Um laço (o alvo controla a própria fonte) é recusado.
- **Interface:** "Aprender MIDI" e "Tirar o mapa" no menu de todo controle (F-CTL-005), a janela de ligação, e uma
  página "Mapas MIDI" com todos os mapas do projeto e da máquina, com edição, busca e o monitor de atividade.

### I-21.3 Retorno MIDI e perfis de controlador

- **Retorno:** um mapa com `feedback` manda o valor do alvo de volta pela saída da mesma porta, sempre que o valor
  muda, venha a mudança da mão, da automação, do desfazer ou de outro controle.
  - Mensagens: CC, par de 14 bits, nota com velocidade para o LED, pitch bend para fader motorizado.
  - **Limite de taxa:** um valor por controle a cada 10 ms, sempre com o último valor.
  - **Sem eco:** o valor que acabou de chegar daquela fonte não volta para ela por 50 ms, para não brigar com o fader
    motorizado.
  - Ao abrir o projeto, ao trocar o banco e ao religar a porta, todos os valores são mandados de novo.
- **Toque:** uma fonte de toque opcional por mapa (a nota de toque dos faders motorizados) liga e desliga a vaga
  tocada. Com o fader tocado, a automação não o move.
- **Perfis de controlador:** um perfil é um arquivo de dados (`controllers/*.json`) com um conjunto de mapas de
  máquina, os bancos (`offset` de 8 em 8) e o retorno.
  - Na 1.0 há:
    - **um perfil genérico editável**;
    - **o protocolo Mackie Control Universal**: 8 faders motorizados com toque, V-pots relativos com anel de LED,
      mute/solo/arm/seleção, transporte, bancos, o visor com nome e valor, e os medidores pelo retorno.
  - O MCU precisa de código próprio, além dos mapas, para o visor (SysEx) e os medidores: fica num módulo do
    `lm-session` que só traduz entre o protocolo e os mapas, sem outro caminho para o núcleo.
  - HUI: depois da 1.0.

### I-21.4 Gravação MIDI

- **O que grava:** faixas armadas gravam notas com velocidade, CC, pitch bend, aftertouch e program change (P-15),
  com o mesmo punch, pré-roll, contagem e laço da gravação de áudio. As teclas seguradas terminam no Parar.
- **Para onde vai:**
  - as mensagens **sem mapa** vão para o item MIDI, em faixas de controle dentro do item, e tocam de volta no
    instrumento;
  - as mensagens **com mapa** viram **automação do alvo** (I-21.5).
- **Modos** (F-REC-101): somar (overdub, o padrão), substituir o trecho, ou take numa camada nova.
  - Em laço, cada passada é uma camada, e uma passada com menos de 90 % do laço é descartada, como no áudio.
- **Quantizar ao gravar** (F-REC-102): grade, força e "só o início".
- **Também gravam:** o passo a passo (F-PAS-001) e a gravação no item aberto no editor de notas (F-NOT-018,
  F-REC-029).
- **À prova de queda:** os eventos gravados vão, a cada segundo, para um arquivo de registro só de acréscimo
  (`audio/rec-<uuid>.midilog`), com `fsync`. A recuperação depois de uma queda segue a do áudio (`recording.json`,
  U5).
- **Desfazer:** um desfazer tira a gravação inteira, de todas as faixas (F-REC-027).

### I-21.5 Automação: gravar e editar

- **Onde mora:** cada faixa e cada canal da mesa têm a lista `automation` (o esquema do `lm-doc`). Cada pista aponta
  para um parâmetro de qualquer nó do dono: plugin, seção do rack, fader, envio, pan, master.
  - Pontos com forma (degrau, curva −1 a 1, suave com tensão) e salto (P-27).
  - Os campos automatizáveis são parâmetros em degraus (a regra do `lm-plugin`).
  - O andamento é a pista de andamento, com rampas (I-18).
- **Modos por pista**, com um botão global que vale para todas:

| Modo | Tocando |
|---|---|
| Desligado | a curva não toca |
| Ler | a curva manda; a mão vence só enquanto toca (I-6.2) e volta para a curva em 50 ms |
| Toque | grava só enquanto o controle está tocado; ao soltar, volta para a curva em 50 ms |
| Trava | grava a partir do primeiro toque e segura o último valor até parar |
| Escrever | grava a pista inteira enquanto toca, apagando o que havia |

- **Gravação:**
  - As fontes: a mão na interface (`param_write` com toque), o MIDI mapeado e o cliente remoto, todas pela vaga
    tocada.
  - A telemetria manda as vagas tocadas com a posição em amostras pelo anel que nunca perde (I-6.4, C3).
  - A `Session` acumula e simplifica, com a tolerância na unidade do parâmetro (0,1 dB, 1 cent, 0,5 % da faixa) e
    degraus exatos nos parâmetros discretos.
  - Ela grava ao soltar, ao parar e a cada volta do laço: **uma transação de `Origin::Recording` por passada**, um
    passo de desfazer. Em laço, a passada nova substitui o trecho que ela cobriu.
  - Uma pista que ainda não existe é criada na mesma transação.
  - O cabeçalho mostra "a mão venceu" até o rearme (o botão de rearmar volta todas as pistas para a curva).
- **Edição:**
  - Todo o F-AUT:
    - desenhar e mover pontos, borracha, valor no ponto;
    - espelhar, zoom e grade, fantasmas das notas e da onda;
    - tocar a curva.
  - Além disso:
    - **lápis livre**, simplificado ao soltar;
    - **linha** (Shift+clique);
    - **selecionar trechos**, recortar, copiar e colar;
    - **escalar e deslocar valores** da seleção;
    - **afinar** (tirar pontos redundantes);
    - **mover a automação junto com os itens** (opção, ligada por padrão);
    - converter entre as faixas de controle de um item MIDI e uma pista de automação.
  - As edições pesadas (afinar, converter, escalar muitos pontos) são ações do núcleo (I-19), com mais de 200 ms
    virando `Operation`.
- **Na interface** (`lm-gui`, editor de automação, a tela do legado com estes acréscimos):
  - a barra mostra a lane aberta, o **modo da lane** (combo que grava `mode`) e **Rearmar** (`automation.rearm`);
  - **"a mão venceu"**: a lane cujo par (dono, parâmetro) está em `TelemetryView.hand_won` mostra o aviso no
    cabeçalho da lane e no editor, até o rearme; o botão global de modo (`transport.automationMode`) fica no
    transporte;
  - lápis livre (Shift+P) manda os pontos crus e chama `automation.thin` ao soltar; Shift+clique traça a linha; a
    seleção por retângulo (Shift+S) serve a recortar, copiar, colar, escalar e deslocar (`automation.scale`); cada
    ponto mostra a forma do trecho que termina nele (alça no meio do trecho "curva") e o valor de saída (`out`) como
    um segundo ponto vazado na mesma vertical.

### I-21.6 Pronto quando

- **Mapeamento:**
  - O teste gerado mapeia todo parâmetro de todo esquema, todo campo automatizável e toda ação do registro, e cada
    um responde a um CC.
  - Um CC mapeado chega ao processador na amostra certa, com o atraso fixo de um bloco, sob a armadilha de alocação.
  - 14 bits, NRPN e os três modos relativos convertem exatamente; pickup não pula; toggle e momentâneo.
  - Aprender detecta 7 bits, 14 bits, NRPN e encoder relativo a partir de sequências gravadas.
  - Remover o alvo remove o mapa; desfazer o devolve.
  - Seguir a seleção e trocar de banco recompilam só a tabela do mapa.
- **Retorno:** o valor volta em toda origem de mudança; sem eco por 50 ms; limite de 10 ms; tudo reenviado ao abrir.
  - Contra uma porta virtual no ThinkPad.
  - O MCU contra um simulador de MCU em teste: faders, toque, V-pots, visor, bancos.
- **Interfaces:**
  - Desligar e religar uma porta (porta virtual) mantém os mapas e religa sozinho.
  - O apelido resolve noutra máquina.
  - A saída MIDI sai no horário marcado, com menos de 1 ms de erro no ThinkPad.
- **Gravação MIDI:**
  - Notas, CC, pitch bend, aftertouch e program change gravados nas posições certas.
  - Camadas por passada.
  - `kill -9` no meio devolve tudo até o último segundo.
  - Um desfazer tira tudo.
- **Automação:**
  - Os cinco modos, cada um num teste com o tempo injetado.
  - A curva gravada reproduz o movimento dentro da tolerância.
  - Uma passada por passo de desfazer.
  - A mão vence e volta em 50 ms.
  - Gravar e tocar ao mesmo tempo, em 64 pistas, sem alocar no áudio.

## I-22. O visual próprio dos plugins (decisão do usuário, 20/11)

**O que o usuário disse:** os plugins nativos do legado que tinham **identidade visual própria** mantêm essa
identidade. São os instrumentos e os efeitos, como o Guitarix e o AmplifikationLite, com o fundo, os botões, as
luzes e os gráficos de cada um. Eles são portados **no próprio estilo**, fiéis ao legado. Os que não tinham visual, ou
eram só botões soltos, usam o editor genérico (I-9), sem portar visual nenhum.

- **Onde mora:** o painel visual é interface, então **nunca entra no crate do plugin**, que continua sem GPUI.
  - Ele mora num crate de painéis, `lm-gui-plugins`, com um registro `id do plugin → painel`.
  - A janela do instrumento (design 05) e a janela de efeitos mostram o painel registrado. Sem registro, mostram o
    editor genérico.
- **Só pelo `CorePort`:** o painel lê o estado na árvore, mexe nos parâmetros por gestos e `param_write`, e desenha
  os dados vivos do plugin (a curva do EQ, o osciloscópio, a forma de onda) pelo `display(node)` (F2).
  - Não tem nenhum ponteiro para o processador.
  - Funciona igual no modo cliente.
- **Fidelidade:**
  - o painel reproduz o do legado na posição, no tamanho e na aparência: as mesmas imagens, os mesmos tipos de botão
    (os knobs do LMMS em cada estilo, os LEDs, os seletores de onda, os visores);
  - as mesmas interações, de F-CTL-005 e F-CTL-006;
  - todo controle do painel é o mesmo parâmetro mapeável por MIDI e automatizável (I-21).
- **As imagens:**
  - Os arquivos de arte do legado (`artwork.png`/`.svg`, `background.png` e os demais) são copiados para
    `lm-gui-plugins/assets/<plugin>/`. Eles são GPL como o resto.
  - As imagens de terceiros só entram com a permissão registrada. Por exemplo, as do AmplifikationLite foram
    liberadas pelo autor em 18/11 (memória do projeto, `amplifikation-blackbox`).
- **Escala:** o painel é desenhado na escala da tela, com as imagens em 1x e 2x quando o legado tiver, sem borrar.
- **Inventário:** `docs/plugins-visual.md` lista cada plugin nativo do legado, se tinha visual próprio ou não, com o
  arquivo de onde vem e o estado do port. É o "pronto quando" desta seção, junto com um teste sem janela por painel:
  abre, mexe em cada controle e confere o parâmetro.

---

## Parte 5: o programa no mundo real (H1–H19)

**Plataforma (decisão do usuário, 19/11): Linux com ALSA primeiro, macOS depois.** Em cada ponto, o comportamento no
Linux vem primeiro; o do macOS vem depois, marcado "Mac". As correções da revisão da sessão de implementação estão
incorporadas.

### H1. Prioridade e memória das threads de áudio

```rust
// lm-io: what every audio-rendering thread does when it starts, and undoes before the device stops
pub trait RealtimeThreads {
    fn join(&self, period: Duration) -> Result<RtMembership>;
}
/// Leaves on Drop, on the same thread that joined: not Send
#[must_use] pub struct RtMembership { _not_send: PhantomData<*const ()> /* ... */ }
```

- **Linux:**
  - o áudio e os workers pedem `SCHED_FIFO` com a mesma prioridade, pelo `rtkit` (D-Bus) ou direto, se o
    `RLIMIT_RTPRIO` permitir (grupo `audio`, `limits.conf`). **O `rtkit` dá `SCHED_RR`, não `SCHED_FIFO`** (medido
    no ThinkPad, Ubuntu 24.04): os dois valem como tempo real. A única diferença é que o RR reveza threads da mesma
    prioridade em fatias de 100 ms, e cada thread de áudio dorme no `poll` muito antes disso, a cada período;
  - **memória travada:** `mlockall(MCL_CURRENT | MCL_FUTURE)` ao armar ou tocar, se o `RLIMIT_MEMLOCK` permitir, e as
    pilhas das threads de áudio pré-tocadas na criação. Uma falta de página na thread de áudio é um estalo;
  - sem permissão de tempo real: o programa toca mesmo assim e mostra um aviso, uma vez, com o que configurar (o grupo
    `audio` e os limites).
- **Mac:** os workers entram no `os_workgroup` do dispositivo e usam `THREAD_TIME_CONSTRAINT_POLICY` com o período do
  bloco.
- **Trocar de placa** (I-10.3): os workers saem do grupo ou da prioridade do dispositivo velho e entram no novo.
- As threads de exportação **não** são de tempo real.

### H2. O `pump` não depende de desenhar

- O `pump` roda **na thread principal**, disparado por um timer do laço de eventos principal (o da biblioteca de
  interface, ou o do `lm-headless`), a cada 10 ms, que não depende de quadros desenhados.
- Enquanto toca, grava ou exporta, a `Session` impede o sistema de dormir:
  - **Linux:** um inibidor do `systemd-logind` (`org.freedesktop.login1.Manager.Inhibit`, "sleep:idle"), devolvido ao
    parar;
  - **Mac:** `beginActivity` com `NSActivityUserInitiated | NSActivityIdleSystemSleepDisabled`, que também desliga o
    App Nap.
- **Critério:** o `pump` parado por 10 s não perde nada. A gravação não depende dele, os anéis têm política escrita e a
  devolução tem espaço reservado.

### H3. Sem espaço em disco

Falta de espaço **nunca perde dado em silêncio**.

| Onde | O que acontece |
|---|---|
| Armar | aviso se não couberem 10 min das entradas armadas; **recusa abaixo de 30 s** (decisão do usuário) |
| Gravar | o `DiskWriter` pré-reserva o arquivo ~30 s à frente (Linux `fallocate`; Mac `F_PREALLOCATE`), para o disco cheio aparecer cedo, com margem para fechar o arquivo e o cabeçalho limpos; a gravação para e o usuário vê o tempo exato |
| Salvar | o `projeto.json` antigo continua intacto (tmp + rename só no fim) |
| Diário | marcado incompleto, aviso, nova tentativa no próximo salvamento |
| Autosave | um aviso por sessão, sem repetir em laço |
| Importar | a `Operation` falha antes de copiar, dizendo quanto falta |

### H4. Arquivos de áudio e o desfazer

Nada em `audio/` é apagado automaticamente enquanto alguma raiz do histórico ou do diário o cita. A limpeza é a ação
explícita "Limpar arquivos não usados", que mostra o que sai e o espaço liberado, e **zera o desfazer** (avisado
antes).

### H5. Monitoramento e alinhamento do take

- O caminho de monitoramento da faixa armada **não recebe o atraso da PDC**: quem toca se ouve com a latência mínima.
- **O alinhamento do take** (corrigido pela revisão): quem grava toca junto com a reprodução, que chega atrasada pela
  PDC do caminho até a saída. O deslocamento do take é **latência de ida e volta + PDC do caminho de reprodução no
  início da gravação**.
- **Uma mudança que altere essa PDC durante a gravação é recusada**, com aviso (por exemplo, inserir um plugin com
  latência numa faixa que toca). Ela pode ser feita ao parar.
- Teste: um impulso gravado contra uma reprodução com um plugin de 1000 amostras de latência cai na grade.

### H6. Números subnormais e determinismo

- Toda thread que renderiza liga o descarte de subnormais ao iniciar:
  - **x86-64** (o Linux típico): FTZ e DAZ no `MXCSR`;
  - **aarch64** (Linux ARM e Mac): o bit `FZ` do `FPCR`, que cobre os dois.
- Mexer nesses registradores exige `unsafe`, então isso mora num `lm-sys-fpu`.
- **O bit a bit vale por plataforma.** x86-64 e aarch64 não precisam dar o mesmo arquivo (por causa de FMA e da
  `libm`), e as referências dos testes são guardadas por plataforma.

### H7. Edições que passariam de 200 ms

```rust
impl Session {
    pub fn heavy_edit(&mut self, label: Label, target: Target,
        compute: impl FnOnce(&Value, &Cancel, &Progress) -> Result<Patch> + Send + 'static) -> OperationHandle<Diff>;
}
/// The ready edit and the nodes it was computed from. If any of them is no longer `same_node` in the current root,
/// it is a conflict: the edit is computed again once, or refused with a notice (revisão)
pub struct Patch { pub ops: Vec<PatchOp>, pub based_on: Vec<(NodeId, Value)> }
```

### H8. Take de faixa que sumiu

O take vira um item numa faixa nova, "Take recuperado", com aviso. Nunca é descartado (I-11).

### H9. Editar o andamento tocando

- Tocando: a posição musical é preservada (`Relocate { tick }` junto da `Install`, I-7.2).
- **Gravando: editar o mapa de andamentos é recusado**, com aviso, porque o `Relocate` quebraria o take. A edição pode
  ser feita ao parar.

### H10. O projeto aberto duas vezes

- `projeto.lock` na pasta, com o processo, **a hora de início do processo**, a máquina e a hora. Assim, um PID
  reaproveitado depois de reiniciar não engana a trava (revisão).
- Trava de processo vivo, ou de outra máquina: o projeto abre **só para leitura**, com aviso. Trava de processo morto
  nesta máquina: é retirada, com aviso.
- Só leitura: as transações funcionam, mas salvar, diário e autosave ficam desligados; "Salvar como" é o caminho.

### H11. Projeto de uma versão mais nova, e a cópia antes de migrar

- Um nó com versão de esquema maior que a do programa faz o projeto abrir **só para leitura**, com aviso. Ele nunca é
  salvo por cima.
- **Ao migrar um projeto de esquema mais velho**, o original é guardado como `projeto.json.v<N>.bak` antes do primeiro
  salvamento (revisão). Assim a versão anterior do programa ainda tem o que abrir.

### H12. Limites como regra do documento

Ver I-3.5.

### H13. Colar

- A área de transferência guarda o trecho (`Value`), os recursos citados (hash e caminho de origem) e **o uuid do
  projeto de origem** (`meta.uuid`).
- Colar dá ids novos ao trecho e corrige as referências internas.
- Uma referência para fora do trecho **é mantida se o trecho veio do mesmo projeto** (por exemplo, um envio para uma
  faixa que existe e não foi copiada). **Se veio de outro projeto**, ela cai no `OnRemove` do campo (revisão).
- Recursos de outro projeto são copiados para o `audio/` do destino numa `Operation`, e os itens colados aparecem
  carregando. Trecho grande: `heavy_edit`.

### H14. Pânico fora do áudio

O compilador e as operações rodam em `catch_unwind`. **A `Session` guarda a tabela autoritativa do que está
instalado** (a última `Install` confirmada). É com ela que um compilador recriado depois de um pânico reconstrói a
própria vista (revisão).

### H15. Varredura de plugins de terceiros (F2)

A varredura roda num processo auxiliar, com uma lista de bloqueados.

### H16. Medidores a ~60 Hz

O áudio acumula os picos e publica o quadro a ~60 Hz.

### H17. Recurso ausente

O `Loading::Missing` oferece "localizar arquivo" (conferir o hash do arquivo escolhido é uma `Operation`) e
**"localizar pasta"**, que resolve de uma vez todos os ausentes de uma biblioteca que mudou de lugar (revisão).

### H18. Determinismo e plugins de terceiros

O render é determinístico com plugins internos. A exportação avisa quais plugins de terceiros não garantem o mesmo
resultado, e o teste de referência só usa plugins internos.

### H19. Calibração

Guarda-se **o resíduo** (a medida menos a latência que o dispositivo informa), por dispositivo, taxa e reamostrador de
borda. Assim a calibração continua valendo quando o tamanho do bloco muda (revisão).

---

## Parte 6: furos da sessão real e do legado (U1–U38)

Achados pela sessão de implementação: U1–U24 vêm de uma sessão real de gravação, de cancelar no meio, do que passa de
200 ms e de plugins mal escritos; U25–U38 vêm de bugs reais que ela corrigiu no legado. Aprovados pelo usuário em
19/11, com as decisões dele marcadas. Linux primeiro.

### Gravação de ponta a ponta

**U1. Gravar em loop.** O `Renderer` manda, num anel que nunca perde, um `LoopWrapped { input, sample_in_take }` a cada
volta. O `Take` leva a lista de cortes, e a `Session` faz uma camada por volta, todas sobre o mesmo recurso com
deslocamentos diferentes.

**U2. Comping por id.** Cada camada tem id (D4) e o `comp` aponta por id, nunca por índice. Toda fronteira do comp tem
crossfade: **padrão de 5 ms, editável em cada fronteira** (a confirmar pelo usuário).

**U3. Forma de onda crescendo.** O `DiskWriter` calcula os picos enquanto grava e os publica num `triple_buffer`. A
interface desenha o item "gravando" a partir deles.

**U4. Pré-roll e punch gravam as sobras** (decisão do usuário). O que entra antes do punch e no pré-roll é gravado como
alça: o item começa no punch, mas a borda pode ser estendida para trás, e para depois do punch-out.

**U5. Queda no meio da gravação.** Antes do primeiro bloco, um registro `recording.json` é escrito ao lado do arquivo,
com a faixa, a entrada, o `start_sample`, a latência descontada e a PDC usada no alinhamento. Os cortes de loop vão
sendo acrescentados. A recuperação (I-11) usa esse registro para pôr o take no lugar certo.

**U6. Uma thread de disco só para a gravação.** O diário, o autosave e os picos ficam em outra, para que o `fsync` de
um nunca atrase o outro.
- **Linux:** salvar é escrever o tmp, dar `fsync` no arquivo, `rename`, e `fsync` na pasta.
- **Mac:** o mesmo, com `F_FULLFSYNC` (o `fsync` do macOS não garante que os dados cheguem ao disco).

**U7. Dispositivos: configuração explícita** (decisão do usuário).
- O usuário escolhe, nas preferências, **o dispositivo de entrada e o de saída**. No ALSA, a F1 trabalha com **um
  dispositivo `hw` duplex** (entrada e saída da mesma placa), com captura e reprodução ligadas (`snd_pcm_link`) para
  andarem no mesmo relógio.
- **Entrada e saída em placas diferentes** (relógios diferentes) só funcionam se o usuário ligar explicitamente a opção
  "corrigir a deriva entre as placas" (um reamostrador adaptativo na entrada), com o aviso de que ela acrescenta
  latência. O programa **nunca** junta dispositivos sozinho.
- **Mac:** o mesmo princípio. O programa usa o dispositivo agregado que o usuário criou (Configuração de Áudio e MIDI)
  ou a opção explícita de correção de deriva; nunca cria um agregado sozinho.
- **Bluetooth** (no Linux, pelo PipeWire; no Mac, AirPods): usar o microfone de um fone Bluetooth derruba a saída para
  o perfil de chamada e a latência passa de 150 ms. Isso vira aviso ao armar.

**U8. Permissões e dispositivo ocupado.**
- **Linux:** sem permissão em `/dev/snd/*` (usuário fora do grupo `audio`), a abertura falha, e o aviso diz o que
  fazer. Com o dispositivo ocupado pelo PipeWire ou pelo PulseAudio, o programa pede a liberação pelo protocolo
  **ReserveDevice1** (D-Bus, como o JACK faz), como o usuário decidiu em 12/11. Se o dono recusar, o aviso dá as opções
  (outra placa, ou o JACK, na F2).
- **Mac:** a permissão de microfone (TCC). Sem ela, a entrada abre e grava silêncio. A `Session` confere a autorização
  antes de armar e mostra o caminho para as configurações. O `Info.plist` precisa do `NSMicrophoneUsageDescription`.

**U9. Microfonia.** Com o microfone e o alto-falante internos do mesmo computador, o monitor `auto` não monitora, e a
faixa mostra por quê.

**U10. Durante a gravação**, trocar de dispositivo, de taxa ou de tamanho de bloco é recusado, ou a gravação para limpa
antes, à escolha do usuário.
- **Linux:** com o `hw` aberto em exclusivo, ninguém muda a taxa por fora.
- **Mac:** outro programa pode mudar a taxa da placa compartilhada. A notificação de propriedade faz a gravação parar
  limpa e religa o reamostrador de borda.

**U11. Placa removida ou máquina suspensa.**
- **Linux:** o ALSA devolve `-ENODEV` (ou erro no `poll`). **Mac:** chega a notificação de dispositivo removido.
- O caminho:
  1. barreira do `stop`;
  2. a gravação é fechada até o último bloco;
  3. o `Renderer` passa para o `Dummy`, e a sessão continua viva, parada;
  4. aviso;
  5. quando a placa volta, "reconectar" (no Linux, pela varredura periódica ou pelo `udev`).

**U12. Entrada ausente.** Uma faixa cuja entrada (`channels: [3]`) não existe na placa atual mostra o estado "entrada
ausente" (como o `Missing`). A entrada nunca é remapeada em silêncio.

### Cancelar e fechar no meio

**U13. A sequência de saída.** Ao pedir para fechar com gravação, exportação, importação ou salvamento rodando:
1. a gravação para e fecha;
2. as operações canceláveis são canceladas;
3. as não canceláveis terminam, com o carregamento visível;
4. vem a pergunta de salvar;
5. só então a `Session` é destruída.

- **Linux:** fechar a janela e os sinais `SIGTERM`/`SIGINT` seguem esse caminho. Os sinais são tratados por um
  `signalfd`, ou por um handler que só escreve num pipe, e entregues ao laço principal.
- **Mac:** `applicationShouldTerminate` com `NSTerminateLater`.
- No desligamento do sistema, o tempo é curto. Vale o que já está no diário e no `recording.json`.

**U14. Importação cancelada** não deixa arquivo pela metade. A cópia vai para um `.tmp` e só é renomeada para o hash no
fim. Os `.tmp` órfãos são apagados ao abrir o projeto.

### O que ainda passava de 200 ms

**U15. Liberar coisas grandes.** O `collect` manda o que é pesado (processadores grandes, programas, raízes velhas) para
uma **thread de descarte**, que só libera memória.

**U16. O `stop` com barreira** tem tempo limite de 2 s e só acontece no `Drop` da `Session` ou dentro de uma `Operation`
(a troca de placa já é uma).

**U17. Listar dispositivos** é uma `Operation`. No ALSA, enumerar placas e *hints* pode demorar com dispositivos USB;
no Mac, com Bluetooth e agregados. A tela de preferências mostra o carregamento.

**U18. Desfazer grande.** O histórico guarda o `Diff` de cada passo (e o inverso), calculado na transação. Desfazer e
refazer não comparam raízes.

**U19. A inicialização.** A janela aparece primeiro e mostra o carregamento. Registrar plugins, ler as preferências e
abrir o último projeto vêm depois do primeiro quadro.

### Um plugin mal escrito

**U20. NaN, infinito e proteção de ouvido.** O `Renderer` confere a saída de cada nó a cada bloco, numa varredura
barata. NaN ou infinito fazem o nó virar silêncio e ficar marcado, como no pânico, com aviso. O master tem uma proteção
fixa: um pico acima de +24 dBFS corta a saída (com um fade curtíssimo) e avisa. É proteção de ouvido, não um efeito.

**U21. Plugin em laço infinito ou queda de verdade.**
- Antes de cada `process`, o `Renderer` anota o `NodeIndex` corrente num atômico.
- Um vigia detecta a callback parada e grava no diário qual nó era.
- Uma queda de verdade (`SIGSEGV` dentro de um `lm-sys`) grava o mesmo, pelo tratador de queda.
- Ao reabrir, o projeto entra em **modo seguro**, com aquele plugin desligado e um aviso.

**U22. Builds travados.** Os builds de plugin usam um pool próprio de workers. Salvar, o diário e a importação nunca
dividem worker com um build.

**U23. Saídas não escritas.** O host zera as saídas antes de cada `process` (no `in_out`, a saída já tem a entrada). O
`latency()` é lido só no build; uma mudança depois só vale por `LatencyChanged`.

**U24. Estado opaco grande.** Valores opacos acima de 64 KB (um blob de terceiros, uma tabela desenhada) vão como
`Resource` por hash, em `state/`, e a árvore só guarda a referência.

### O que o legado ensinou

**U25. Notas presas.**
- O `Renderer` guarda uma tabela fixa de notas ativas, `(porta, canal, tecla) → (nó, note_id)`.
- O note-off sempre vai para o nó do note-on, mesmo que o programa tenha mudado entre um e outro (faixa desarmada,
  plugin trocado, transposição mudada).
- Se o nó sai do programa (`remove` ou `replace`), o `Renderer` gera o fim de cada nota dele.
- O mesmo vale para as notas dos itens quando o loop volta, no `locate`, no `stop`, e quando o item é editado ou apagado
  no meio de uma nota.

**Critério:** nenhuma nota presa em mil trocas de programa com notas tocando.

**U26. Notas curtíssimas.** No mesmo offset, o note-off vem antes do note-on da mesma tecla, para o redisparo funcionar.
Um processador trata o on e o off no mesmo bloco: a voz começa e entra no release. Os testes gerados incluem notas de
1 amostra, redisparo na mesma amostra e mais notas que vozes.

**U27. Mono e legato.** O `VoicePool` tem as políticas `Poly`, `Mono` e `Legato`, com a pilha das teclas seguradas
(tamanho fixo, 128). A última nota tem prioridade, e soltá-la volta à anterior.

**U28. Velocidade.** `velocity` é `f32` de 0 a 1, linear, e a curva de resposta é do host, por faixa. **A confirmar
pelo usuário:** ou o plugin declara no esquema o que faz com a velocidade (e o teste confere só que a resposta é
crescente e coerente com o declarado), ou há uma regra fixa de nível (0,5 ≈ −6 dB).

**U29. Padrões congelados por versão.** Mudar o valor padrão de um campo exige subir a versão do esquema, com um
`upgrade` que escreve o padrão antigo nos projetos velhos. O teste guarda os padrões de cada esquema publicado num
arquivo de referência, e mudar um sem subir a versão falha.

**U30. Escolhas por chave de texto.** Toda enumeração (presets, kits, formas de onda, modos) é `Choice` com chave
estável, nunca um número. `Stepped` é só para passos numéricos de verdade (oitavas, semitons).

**U31. Nada aloca ao mudar um parâmetro.** Tudo o que qualquer valor de qualquer parâmetro possa precisar é alocado no
build, inclusive todos os ramos de cada `Choice`. **Teste gerado para cada plugin:** varrer cada parâmetro de ponta a
ponta e todos os valores de cada `Choice`, com acordes tocando e mais notas que vozes, sob a armadilha de alocação. É
critério do `lm-plugin` e do porte de cada plugin.

**U32. Quando o `adopt` não é possível.** O `adopt()` devolve `Adopted::{Yes, No}`. Com `No` (o FluidSynth não passa
vozes entre instâncias, e plugins de terceiros também não):
- o processador velho recebe "todas as notas em release";
- ele continua num **nó de cauda**, gerado pelo compilador, até a cauda acabar (no máximo 2 s), e só então é removido;
- o novo já recebe as notas novas.

**U33. FluidSynth** (base do AvlDrums, F1; o crate e o plugin foram apagados em 27/11, o Sampler os substitui, mas a regra vale para qualquer biblioteca de C). Tudo isto vai na ficha do `lm-sys-fluidsynth`:
- `synth.threadsafe-api` desligado: a instância só é chamada pela thread de áudio;
- `synth.cpu-cores = 1`: sem threads próprias;
- um SoundFont nunca é carregado numa instância que está tocando, sempre numa instância nova no build, com a troca do
  U32.

**U34. A régua do A/B contra o legado.** Não é bit a bit. Para cada plugin portado:
- teste de nulo abaixo de −90 dBFS, ou envoltória e espectro dentro de uma tolerância escrita;
- uma **lista das diferenças intencionais** (bugs do legado corrigidos, a fase em `f64`), cada uma com o motivo.

**U35. Precisão em sessões longas.** Fase e contadores de tempo ficam em `f64`, ou em inteiros com volta. O teste
gerado toca uma nota por 30 min de render e confere a afinação no fim.

**U36. Independência da taxa.** O teste gerado roda cada plugin a 44,1, 48, 96 e 192 kHz e confere afinação e nível.

**U37. Exportar um trecho.** Decisão do usuário: **ele escolhe a opção na hora de exportar**.
- **"Exato":** renderiza desde o começo do projeto e descarta o que vem antes do trecho. O resultado é igual, bit a bit,
  ao mesmo trecho dentro da música inteira.
- **"Rápido":** começa alguns segundos antes do trecho (o aquecimento, escolhido na mesma janela) e descarta o
  aquecimento. O resultado é quase igual, e a exportação diz que não é idêntico.
- Regra para os plugins: LFOs sincronizados ao andamento derivam a fase da posição em tiques (sem estado), para que o
  `locate` e o loop sejam repetíveis.

**U38. A janela do plugin de um nó que sumiu.** O editor de plugin recebe só o `NodeId`, um `Feed` do nó e o
`DisplayReader`. A `gui` fecha a janela quando o `Feed` diz "removido", e o `DisplayReader` de um nó removido devolve
`None`, nunca dados velhos. **Critério:** remover a faixa com a janela do plugin aberta, mil vezes, sob ASan.

---

## Parte 7: coerência e detalhes (C1–C13)

Achados pela sessão de design na revisão de coerência de 19/11 e aprovados pelo usuário. O primeiro achado dessa
revisão foi que o `arquitetura-nova.md` contradizia este documento. Ele foi limpo e agora diz que, em qualquer
conflito, vale este documento.

### C1. As notas de um item ficam num mapa por número

- Dentro de um item MIDI, as notas ficam num **mapa persistente por `n`** (`"notes": { "812": {...}, ... }` no
  JSON), e não numa lista.
- Achar, mudar ou remover uma nota pelo `n` custa O(log n), sem percorrer o item.
- A ordem por tempo é montada pelo compilador, ao gerar a lista de eventos do item.
- O teste das 10 mil notas (Parte 4) também mede arrastar uma nota a 60 quadros por segundo.

### C2. Um xrun de captura não desalinha o take

- Num xrun, a gravação continua e o ponto é marcado (decisão de 12/11). Mas um xrun de captura **perde amostras de
  entrada**.
- O `Renderer` mede quantos quadros se perderam:
  - **Linux:** pelos carimbos de tempo do ALSA (`snd_pcm_status`), ou pela diferença entre os contadores da captura e
    da reprodução ligadas;
  - **Mac:** pelo carimbo de tempo do dispositivo na callback.
- O `DiskWriter` escreve **exatamente esse número de amostras de silêncio**, marcadas no take. A grade nunca anda.
- **Critério do `lm-io`:** um xrun provocado no meio de uma gravação de 1 h deixa o take inteiro na grade.

### C3. A gravação de automação não perde pontos

- Os valores tocados (`Touched`) vão num **anel próprio que nunca perde**, dimensionado em tempo (C4).
- Se ele encher, é erro explícito, como na gravação de áudio: a gravação de automação daquele parâmetro para, e o
  usuário é avisado. Nunca há perda silenciosa.

### C4. Todo anel é dimensionado em tempo

- O critério "o `pump` parado por 10 s não perde nada" (H2) vale para **todo** anel cujo consumidor é a thread
  principal.
- Cada anel tem, ao lado da definição, a conta do pior caso em 10 s. Por exemplo: o `LoopWrapped` com o loop mais
  curto permitido; os eventos de nota de 16 portas MIDI a 1000 eventos por segundo.
- Os anéis que têm perda como política escrita (avisos de xrun, pedidos de plugin) podem ser menores, e a perda é
  contada.

### C5. Chaves de parâmetro são para sempre

- A automação e as referências de outras faixas apontam para um parâmetro pela **chave de texto** (`"vol1"`). A
  migração de esquema é por nó e não alcança quem aponta de fora.
- **Regra:** uma chave de parâmetro publicada nunca muda e nunca é reaproveitada. Para renomear, o esquema declara um
  **apelido** (`aliases: [("vol_1", "vol1")]`). Quem aponta pela chave velha resolve pelo apelido, e o salvamento
  seguinte grava a chave nova.
- Remover um parâmetro exige subir a versão do esquema. O `upgrade` diz o que fazer com a automação que apontava para
  ele, e a `Session` aplica isso às referências de fora na mesma migração.

### C6. Os relógios no Linux

- **`HostTime` é o `CLOCK_MONOTONIC`** no Linux, ou o `mach_absolute_time` no Mac, em nanossegundos. Nunca é o relógio
  de parede.
- **PCM do ALSA:** o carimbo de tempo é configurado para o monotônico (`snd_pcm_sw_params_set_tstamp_type`,
  `MONOTONIC`) e ligado (`tstamp_mode`). O `HostTime` do início de cada bloco vem do status do PCM.
- **ALSA seq:** a porta de entrada usa uma fila com carimbo em tempo real. O tempo da fila é convertido para o
  monotônico pela diferença medida no início. Se a conversão não for possível, o carimbo é posto na chegada, na thread
  do MIDI, com o aviso de que há mais tremor.
- Teste: com o `Dummy`, um evento MIDI gerado num tempo conhecido cai no offset certo, com tolerância de ±1 amostra.

### C7. O período do ALSA é o negociado

- O ALSA pode dar um período diferente do pedido, ou variar o número de quadros por callback.
- O `ProcessSpec::max_block` vem do período **negociado**, depois de abrir o dispositivo.
- O `Renderer` aceita qualquer número de quadros até o máximo, e divide um bloco maior em sub-blocos.
- A exportação usa um bloco fixo, escrito na especificação.

### C8. A memória dos recursos não só cresce

- O `ResourceStore` descarta um recurso carregado que **nenhum programa vivo usa e ninguém pediu há 5 minutos**
  (configurável).
- O `Arc` só é solto quando sai a última referência, e a liberação vai para a thread de descarte (U15).
- Se o recurso for pedido de novo, ele volta a carregar, com o estado de carregamento normal.
- Os picos (`.peaks`) ficam no disco e não contam como recurso carregado.

### C9. Um projeto de fora não lê arquivos quaisquer

- A chave de um recurso dentro de `audio/` (ou de `state/`) tem que ser **um hash hexadecimal válido**, do tamanho
  certo. Qualquer outra coisa (`../`, barras, caminho absoluto) é recusada na validação do projeto.
- **Caminhos externos** (soundfonts e kits do sistema) só são seguidos se estiverem dentro das pastas que o usuário
  permitiu nas preferências. Fora delas, o recurso aparece como `Missing`, com a ação "permitir esta pasta".
- Links simbólicos dentro do projeto que apontam para fora da pasta não são seguidos.

### C10. O nome do arquivo de um take, e o hash

- **O hash é o BLAKE3**, que é rápido para arquivos grandes e tem uma crate pura em Rust. Ele entra na lista de crates
  do I-17.
- Um take grava com **nome provisório** (`audio/rec-<uuid>.rf64`), e é esse o nome que o `recording.json` guarda.
- Ao fechar a gravação, uma `Operation` calcula o hash e renomeia o arquivo para ele. O item entra na árvore já com o
  hash.
- A recuperação depois de uma queda reconhece os dois nomes: um `rec-<uuid>` com `recording.json` é um take
  interrompido.

### C11. A construção de um plugin espera os recursos dele

- O `build` de um plugin só começa quando **todos os recursos que o estado dele cita** estão carregados.
- O carregamento do nó (I-2.3) mostra as duas etapas: "carregando a amostra", depois "preparando o instrumento".
- Um recurso `Missing` deixa o nó em `Missing`, sem build. Quando o recurso é localizado, o build começa.

### C12. Revisão final de coerência antes do código

A última etapa antes de o usuário autorizar o código é uma revisão **só de coerência** entre `interfaces.md` e
`arquitetura-nova.md`, feita pelas duas sessões. Cada termo usado tem definição, cada mecanismo é descrito num lugar
só, e nenhuma seção contradiz outra.

### C13. Termos usados e definidos

| Termo | Definição |
|---|---|
| `ProgramGen` | `u64`, incrementado pelo compilador a cada `Install` que produz; começa em 1 e nunca volta |
| `NodeIndex` | `u32`, índice na tabela de processadores do `Renderer` (`< Limits::max_nodes`), com a geração do slot ao lado |
| `ParamSlot` | `u32`, índice nas vagas de parâmetro (`< Limits::max_params`) |
| `HostTime` | `u64`, nanossegundos do relógio monotônico do sistema (C6) |
| `Label` | a chave de um texto traduzível mais os argumentos (`key: &'static str`, `args`); nunca um texto pronto |
| `Rng` | um gerador `xoshiro256**` por nó, semeado com `project.seed` e o `NodeId` do nó (render determinístico) |
| `PreviewId`, `OperationId`, `GestureId`, `ExportId` | `u64` dados pela `Session`, crescentes, nunca repetidos dentro de uma sessão |
| `Resource` | `{ kind, key }`, em que `key` é o hash BLAKE3 em hexadecimal (C9, C10) |

---

## Parte 4: ordem de implementação e "pronto quando" (S22)

**Plataformas no CI:** Linux x86-64 é a principal (todos os testes, ASan, TSan, LSan e as referências de render de
x86-64). Linux aarch64 e macOS aarch64 rodam os testes, cada um com as suas próprias referências (H6).

**Versão mínima do Rust:** fixada no `rust-toolchain.toml` (a estável do dia em que o esqueleto começar). O CI roda
`clippy` com `-D warnings`, `cargo-deny`, testes, `miri` (onde indicado), `loom` (onde indicado), e ASan/TSan no
Linux.

| Ordem | Pacote | Pronto quando |
|---|---|---|
| 1 | `lm-base` | anéis e `OperationHandle` testados com `loom` e `miri`; `Main` por thread testado |
| 2 | `lm-state` | ida e volta pelo JSON em árvores sorteadas, bit a bit; fuzzing de `from_json` + validação por 1 h sem queda; **medida das 10 mil notas** (uma edição por nota, mil vezes: menos de 1 ms por edição e memória do histórico linear) decide se o `imbl` serve |
| 3 | `lm-time` | ida e volta tique ↔ amostra exata em mapas sorteados; nenhum erro acumulado em 24 h de blocos simulados |
| 4 | `lm-doc` | transações aleatórias + desfazer tudo = raiz original; refazer tudo = raiz final; `OnRemove` testado em cada caso; índices iguais aos recalculados do zero depois de cada passo; gestos com transações alheias no meio; cancelar com uma transação alheia no meio preserva a alheia (G1); desfazer com gesto aberto confirma o gesto e o desfaz primeiro (G2); gesto com o alvo removido devolve `GestureTargetGone` sem passo (G3) |
| 5 | `lm-render` | FTZ/DAZ ligados em toda thread de render, e um sinal com subnormais renderizado no áudio e na exportação dá o mesmo resultado (H6); um programa feito à mão soa no `Dummy`; mil instalações seguidas sob `loom`/TSan sem erro; nada aloca nem libera no áudio (armadilha ligada); render bit a bit igual duas vezes; PDC alinha dois caminhos (impulso) |
| 6 | `lm-plugin` + um plugin de teste (seno) | os testes gerados pelo esquema passam |
| 7 | `lm-compile` | compila árvores sorteadas; reaproveita nós iguais; nunca reutiliza um índice antes da devolução |
| 8 | `lm-io` (Dummy, ALSA PCM, ALSA seq, DiskWriter, ResourceStore), testado numa máquina Linux de verdade e não só no CI | barreira testada com callback lenta; gravação de 1 h sem perda; anel cheio vira erro explícito; recursos com atraso injetado mostram o carregamento; trocar o SoundFont tocando não dá silêncio (o antigo toca até o novo ficar pronto, T7) |
| 9 | `lm-session` + `lm-headless` | (e os da Parte 5:) disco cheio simulado em salvar, diário, gravação e importação não perde dados; projeto travado abre só para leitura; esquema mais novo abre só para leitura; colar entre dois projetos não repete ids; take de faixa apagada vira "Take recuperado"; janela minimizada por 10 min gravando não para a coleta; criar e destruir a sessão mil vezes tocando, sob ASan e LSan; salvar → abrir → salvar dá o mesmo arquivo; matar o processo e reabrir volta à última ação; toda operação da lista de I-2 aparece no `activity`; exportar com recursos e plugins ainda carregando dá o mesmo arquivo que exportar com tudo pronto (T8); com o relógio injetado, nenhum alvo mostra estado intermediário |
| 9b | MIDI e automação (I-21) | todos os itens de I-21.6 |
| 10 | A primeira fatia | `arquitetura-nova.md`: gravar com monitoramento, TripleOscillator, um efeito, salvar e abrir |
| 10b | Revisão final de coerência dos dois documentos (C12) | as duas sessões confirmam por escrito que não há contradição nem termo sem definição |
| 11 | O protótipo de interface e depois a `gui` | o protótipo decide a biblioteca; a `gui` usa só a superfície de I-15 |

## Decisões (19/11)

Aprovados pelo usuário: esta versão; o protótipo de interface (a lista de I-15, com o componente de carregamento);
os plugins de validação e a primeira fatia (em `arquitetura-nova.md`). **A implementação não está autorizada**: espera
a ordem explícita do usuário.
