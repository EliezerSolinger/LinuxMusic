# API do esqueleto

O **como** de `arquitetura-nova.md` (o **porquê** e a especificação da base). Este documento é escrito pela sessão de
design; quem implementa anota aqui o que decidir na hora, na seção do módulo, para revisão.

Estado: **substituído por `interfaces.md`** (19/11). Este rascunho foi escrito em C++, antes da decisão de escrever o
núcleo em Rust. Fica só como histórico; a revisão de 18 pontos feita sobre ele está incorporada em `interfaces.md`.

Convenções:
- C++20 (C++23 onde o compilador do Mac e do Linux aceitarem: `std::expected`). Sem exceções no núcleo: erros
  são valores (`Result<T> = std::expected<T, Error>`). Exceções só na borda com bibliotecas, capturadas ali.
- Comentários do código em inglês, como no legado. Textos ao usuário são chaves traduzíveis (especificação, item
  15).
- Toda função diz a thread em que pode ser chamada: `[main]`, `[audio]`, `[worker]`, `[any]`. Em build de
  depuração, cada `[main]` e `[audio]` confere (`ThreadRole::require(...)`), e em release também nos pontos de entrada
  públicos da `Session`.

---

## Mapa dos módulos

```
core/state     Value, Resource, NodeId, Schema, Json            (depende de: nada)
core/document  Document, Edit, Diff, History, Subscription, Refs (depende de: state)
core/render    Processor, Events, ParameterStore, Telemetry, Commands,
               RenderProgram, Compiler, Engine, Retirement      (depende de: state)
core/io        AudioDevice, MidiInput, DiskWriter, ResourceStore (depende de: state, render)
core/session   Session, Actions, Journal, Autosave, Preferences (depende de: todos acima)
plugins/<id>   descritor: esquema + processador                  (depende de: state, render)
gui/           Qt                                                (depende de: session, e lê o resto)
headless/      lmms-headless                                     (depende de: session)
```

O `render` não depende do `document`: o compilador recebe uma raiz (`state::Value`) e devolve um programa. Assim, o
render pode ser testado com árvores escritas à mão, sem documento nem sessão.

---

## core/state

### `NodeId`: ids que nunca se repetem

```cpp
//! A node's id: a kind letter and a number never given twice in a project (the project keeps the next number).
//! Written "t:17" in JSON. Never reused, so an old id never finds a new node: no generation counter is needed
struct NodeId
{
	char kind;          // 't' track, 'c' item, 'p' plugin, 'a' automation, 'n' note list... (see the schemas)
	std::uint64_t number;
	friend auto operator<=>(const NodeId&, const NodeId&) = default;
};
```

- O próximo número fica na raiz (`"nextId": 1234`) e entra no desfazer como qualquer outro valor. Desfazer a criação
  de uma faixa **não** devolve o número: a raiz desfeita tem o `nextId` antigo, mas o `Document` guarda o maior
  número já dado e nunca desce dele.
- Notas não têm id (são valores numa lista do item). A vista se refere a uma nota pelo item mais o índice, dentro de
  uma transação.

### `Value`: a árvore imutável

```cpp
namespace lmms::state
{

struct Resource { std::string kind; std::string key; }; // {"audio", "<sha256>"}

//! An immutable JSON-like value; copying is cheap (shares the nodes). A change makes a new value that shares
//! what did not change
class Value
{
public:
	enum class Kind : std::uint8_t { Null, Bool, Int, Real, String, Resource, List, Object };

	Value();                              // null
	Value(bool), Value(std::int64_t), Value(double), Value(std::string), Value(Resource);
	static auto list(std::initializer_list<Value>) -> Value;
	static auto object(std::initializer_list<std::pair<std::string_view, Value>>) -> Value; // keys in this order

	auto kind() const -> Kind;
	auto asBool() const -> std::optional<bool>;      // (and asInt, asReal, asString, asResource)

	// Object: keys keep their order (saved files and diffs stay stable)
	auto get(std::string_view key) const -> const Value*;            // nullptr: no such key, or not an object
	auto with(std::string_view key, Value value) const -> Value;     // set or append
	auto without(std::string_view key) const -> Value;
	auto keys() const -> /* range of std::string_view */;

	// List
	auto size() const -> std::size_t;
	auto at(std::size_t index) const -> const Value&;
	auto withAt(std::size_t index, Value value) const -> Value;
	auto withInserted(std::size_t index, Value value) const -> Value;
	auto withoutAt(std::size_t index) const -> Value;

	//! Same contents (deep; exact for doubles: bit for bit)
	friend auto operator==(const Value&, const Value&) -> bool;
	//! The same node, not only equal contents (O(1)): what the compiler and the diff use to skip what did not change
	auto isSameNode(const Value& other) const -> bool;
};

} // namespace lmms::state
```

- **A escolha do armazenamento é interna** (`immer` ou nós próprios com `shared_ptr<const ...>`). A interface acima
  não muda com ela. Critério para escolher, no primeiro módulo: um item com 10 mil notas, editado nota a nota mil
  vezes, sem passar de 1 ms por edição nem crescer a memória mais que linearmente no histórico.
- `Value` é seguro para ler de qualquer thread (imutável). Ninguém o modifica no lugar.

### `Schema`: a forma de cada tipo de nó

```cpp
struct FieldSpec
{
	std::string key;
	Value::Kind kind;
	Value defaultValue;
	std::optional<double> min, max;     // numbers
	std::vector<std::string> choices;   // strings that are an enum
	bool required = true;
};

struct ParamSpec // a plugin's parameter: becomes a ParameterStore slot (see render)
{
	std::string key;
	double min, max, def;
	enum class Scale : std::uint8_t { Linear, Log, Stepped } scale = Scale::Linear;
	std::string unit;                   // "dB", "Hz", "%"
	bool automatable = true;
	enum class Rate : std::uint8_t { Block, Sample } rate = Rate::Block; // sample: volume, pan (spec item 1)
};

struct NodeSchema
{
	std::string type;                   // "track", "item.midi", "item.audio", "plugin:tripleoscillator"
	int version;
	std::vector<FieldSpec> fields;
	std::vector<ParamSpec> params;      // (plugins)
	//! Takes a node saved by version @p from to `version`, one step each, chained by the registry
	std::function<Result<Value>(Value node, int from)> upgrade;
};

class SchemaRegistry
{
public:
	void add(NodeSchema schema);                               // [main] at start-up
	auto find(std::string_view type) const -> const NodeSchema*;
	auto validate(const Value& node) const -> Result<void>;     // [any] the error names the path and the field
	auto upgrade(Value node) const -> Result<Value>;            // [any]
	auto defaults(std::string_view type) const -> Value;        // [any]
};
```

### `Json`: leitura e escrita estáveis

```cpp
auto toJson(const Value& value) -> std::string;                 // [any] indented, keys in order, shortest doubles
auto fromJson(std::string_view text) -> Result<Value>;          // [any] never crashes on bad input (fuzzed)
```

- Números de ponto flutuante: `std::to_chars` (representação mais curta que volta igual). Inteiros e reais se
  distinguem pela presença de `.`/`e`. Um inteiro maior que 2^53 é recusado ao ler (ids são texto).
- Biblioteca: escrita própria (é pequena e precisa ter essas garantias exatas), ou uma sem Qt que as dê. O teste de
  ida e volta e o fuzzing decidem.

**Pronto quando:** ida e volta bit a bit em árvores sorteadas; fuzzing de `fromJson` + `validate` por uma hora sem
queda (libFuzzer, ASan); a medida das 10 mil notas acima.

---

## core/document

```cpp
namespace lmms::doc
{

struct Diff
{
	std::vector<NodeId> created, removed;
	std::vector<std::pair<NodeId, std::vector<std::string>>> changed; // node and its changed keys
	bool structure = false; // something that changes the render graph (tracks, plugins, sends, items)
};

class Edit // only inside a transaction
{
public:
	auto root() const -> const Value&;                       // as edited so far
	auto get(NodeId id) const -> const Value*;
	void set(NodeId id, std::string_view key, Value value);
	//! Puts @p node (its "id" filled in here) in @p parent's list @p list at @p index; returns the new id
	auto insert(NodeId parent, std::string_view list, std::size_t index, Value node) -> NodeId;
	void remove(NodeId id);                                  // and everything under it
	void move(NodeId id, NodeId newParent, std::string_view list, std::size_t index);
	void fail(Error error);                                  // the transaction is refused whole
};

class Document
{
public:
	Document(const SchemaRegistry& schemas, Value root);    // [main] the root is validated and upgraded first

	auto root() const -> const Value&;                       // [main]
	auto snapshot() const -> Value;                          // [main] a copy any thread may keep (cheap)
	auto find(NodeId id) const -> const Value*;              // [main] nullptr: gone

	//! [main] Runs @p edit on a new root; validates the changed nodes; on success the new root is current, the
	//! old one goes to the history, and the diff is queued for the subscribers (delivered by deliver(), never
	//! from inside transact())
	auto transact(std::string label, const std::function<void(Edit&)>& edit) -> Result<Diff>;

	//! [main] A continuous gesture (dragging a knob): one undo step however many updates
	class Gesture;
	auto beginGesture(std::string label) -> Gesture;

	auto undo() -> Result<Diff>;                             // [main]
	auto redo() -> Result<Diff>;                             // [main]
	auto history() const -> const History&;                  // [main] labels, can undo/redo

	auto subscribe(NodeId id, std::function<void(const Diff&)> onChange) -> Subscription;      // [main]
	auto subscribeType(std::string type, std::function<void(const Diff&)> onChange) -> Subscription;
	auto subscribeAll(std::function<void(const Diff&)> onChange) -> Subscription;
	void deliver();                                          // [main] called by Session::pump()
};

//! Cancels its subscription when destroyed (a dead view is never called). Move-only
class Subscription;

} // namespace lmms::doc
```

- **Entrega fora do `transact`:** a vista que reage a uma mudança pode pedir outra transação sem reentrância. Ela só
  é entregue no próximo `deliver()`.
- **Validação por nó alterado**, não pela árvore inteira (o custo é proporcional à mudança).
- **O histórico** guarda raízes. Limite configurável por número de passos; como as raízes dividem nós, o custo é o
  das mudanças, não o de cópias.
- **Regras de domínio** que o esquema sozinho não pega (ciclo no roteamento, pasta dentro de si mesma) são
  verificadores registrados por tipo, chamados na validação. A transação que viola uma delas é recusada inteira, com
  a mensagem.

### `Refs`: leitores tipados

```cpp
//! Reads a track in the current root. Holds only the id: a ref to a removed track answers exists() == false
class TrackRef
{
public:
	TrackRef(const Document& doc, NodeId id);
	auto exists() const -> bool;
	auto name() const -> std::string;
	auto volume() const -> double;
	auto items() const -> /* range of ItemRef */;
	// ...one getter per schema field; no setters (changes go through transact())
};
```

Os `Refs` são escritos à mão no começo (poucos tipos). Se crescerem, passam a ser gerados a partir dos esquemas.

**Pronto quando:**
- transações aleatórias + desfazer tudo = a raiz original;
- refazer tudo = a raiz final;
- uma assinatura destruída não é chamada;
- uma transação inválida não muda nada;
- as regras de ciclo funcionam.

---

## core/render

### Eventos e contexto de processamento (o contrato do plugin, no estilo CLAP)

```cpp
namespace lmms::render
{

struct ProcessSpec { double sampleRate; std::uint32_t maxBlock; std::uint32_t maxVoices; };

struct Event
{
	std::uint32_t offset;               // samples into the block
	enum class Type : std::uint8_t { NoteOn, NoteOff, NoteChoke, Param, Midi } type;
	// NoteOn/Off: key, velocity, channel, noteId (stable per note: for a voice to find its note again)
	// Param: slot, value (a jump; smoothing is the processor's)
	// Midi: three bytes
};

struct Transport
{
	bool playing, recording;
	std::int64_t tickAtStart;           // musical position at the block's first sample
	std::int64_t sampleAtStart;         // timeline position in samples
	double bpm; int timeSigNum, timeSigDen;
};

struct ProcessContext
{
	std::uint32_t frames;               // <= ProcessSpec::maxBlock
	std::span<const float* const> inputs;  // per input channel (ports flattened)
	std::span<float* const> outputs;
	std::span<const Event> events;      // sorted by offset
	const Transport& transport;
	const ParameterReader& params;      // the plugin's slots, read without locks
	Rng& rng;                           // seeded per node (spec item 9)
};

//! [audio] Never allocates, locks or throws: checked by the allocation trap in the tests
class Processor
{
public:
	virtual ~Processor() = default;                      // [main or worker] (retired, never on the audio thread)
	virtual void process(ProcessContext& context) noexcept = 0;
	virtual void reset() noexcept {}                     // [audio] silence: voices off, tails cleared
	virtual auto latency() const noexcept -> std::uint32_t { return 0; } // samples (feeds the PDC)
	//! [audio] Takes over what @p old was playing (voices, tails) when a non-parameter key changed and this
	//! processor replaces it. Must not allocate. Default: nothing (a cut)
	virtual void adopt(Processor& old) noexcept {}
};

} // namespace lmms::render
```

- **As vozes** de um instrumento são do processador, alocadas no construtor (que recebe `ProcessSpec::maxVoices`),
  num *slot map*, com a política de roubo da peça 7 de `arquitetura-nova.md`. Um ajudante do núcleo
  (`VoicePool<V>`) faz isso para todos os instrumentos.
- **O descritor do plugin:**

```cpp
struct PluginDescriptor
{
	std::string id;                         // "tripleoscillator"
	NodeSchema schema;                      // "plugin:<id>"; params included
	struct Ports { std::uint32_t audioIn, audioOut; bool noteIn, sidechainIn; } ports;
	//! [main or worker] Everything the processor will ever need is allocated here
	std::function<Result<std::unique_ptr<Processor>>(const Value& state, const ProcessSpec& spec,
		const ResourceView& resources)> createProcessor;
};
```

### `ParameterStore`

```cpp
//! Atomic parameter values, by slot. Fixed capacity in chunks that never move: the audio thread reads a slot
//! through a table of chunk pointers that only ever grows (new chunks are published, old ones never freed until
//! the session ends)
class ParameterStore
{
public:
	auto allocate(std::size_t count) -> SlotRange;          // [main]
	void release(SlotRange range);                          // [main] reused only after a period (Retirement)
	void write(Slot slot, float value);                     // [main] the UI, while dragging
	auto read(Slot slot) const -> float;                    // [audio] relaxed load
};
```

- Para blocos de valores que precisam chegar juntos, um `SeqlockSlot<T>` com o mesmo regime: um escritor
  `[main]`, leitura sem espera `[audio]`.
- **A ligação parâmetro ↔ vaga** fica no programa de render (o compilador a monta a partir da árvore). A árvore
  guarda o valor confirmado; a vaga, o valor ao vivo. Ao terminar um gesto, a transação grava na árvore o valor
  final.

### Telemetria e comandos

```cpp
//! Audio → main. One single-producer ring per kind, fixed size, allocated at start-up. Full: the newest value
//! replaces (meters, position) or the event is counted as dropped (never blocks)
class Telemetry
{
public:
	// [audio]
	void meter(NodeId node, const Peaks& peaks);
	void position(std::int64_t sample, std::int64_t tick);
	void automated(NodeId node, std::uint32_t param, float value);
	void recorded(RecordedBlock block);        // the disk thread reads these, not the UI
	void event(TelemetryEvent event);          // xrun, voice ended, preview ended, plugin crashed
	// [main]
	void drain(TelemetrySink& sink);           // called by Session::pump()
};

//! Main/MIDI → audio. Multi-producer to single consumer, fixed size, never allocates on push
class Commands
{
public:
	auto push(Command command) -> bool;        // [any] false: full (the caller reports it)
	// play, stop, locate(tick), record(on/off), noteOn/Off(node, key, velocity, previewId), panic
};
```

### `RenderProgram`, `Compiler`, `Engine`, `Retirement`

```cpp
//! Immutable once built: the nodes, their order, the workers' partition, the PDC delays, the event lists of the
//! items, the automation curves, and the processors it uses (not owned: see ProcessorPool)
class RenderProgram;

class Compiler
{
public:
	//! [worker] Builds a program from @p root, reusing @p previous's nodes whose tree node isSameNode(), and the
	//! processors of the plugins whose state did not change (a changed non-parameter key gets a new processor,
	//! which adopt()s the old one in the audio thread on the first block)
	auto compile(const Value& root, const RenderProgram* previous, ProcessorPool& pool,
		const ProcessSpec& spec, const ResourceView& resources) -> Result<std::unique_ptr<RenderProgram>>;
};

class Engine
{
public:
	explicit Engine(const ProcessSpec& spec, Telemetry& telemetry, Commands& commands, ParameterStore& params);
	void install(std::unique_ptr<RenderProgram> program);  // [main] handed to the audio thread; old one retired
	//! [audio] The device's callback: takes the newest program, runs the commands, renders, counts the period
	void render(const AudioBlock& block) noexcept;
	auto periods() const -> std::int64_t;                  // [any] acquire load (Retirement)
};

class Retirement
{
public:
	enum class After : std::uint8_t { Turn, Period };
	template<class T, class D> void retire(std::unique_ptr<T, D> object, After after); // [main]
	void collect(bool everything = false);                 // [main] by Session::pump(); everything at the end
};
```

- `install` usa o `Handoff` (troca atômica). O programa que o áudio larga volta pela própria troca e é aposentado
  com `After::Period` (margem de dois períodos, `arquitetura-nova.md`).
- **Processadores e programas:** um processador é retirado quando nenhum programa vivo o usa. O `ProcessorPool`
  conta os usos, só na thread principal.
- **Workers:** o programa já vem dividido em níveis (o que pode rodar em paralelo). O `Engine` tem N workers fixos,
  criados no início, que esperam por semáforo sem trava. A soma em cada faixa pai segue a ordem do programa (render
  determinístico).

**Pronto quando:**
- um programa feito à mão (um oscilador de teste → faixa → master) soa no dispositivo de teste;
- trocar o programa a cada bloco por mil blocos, sob TSan, não dá erro;
- nada aloca no áudio (a armadilha de alocação);
- o mesmo programa renderizado duas vezes dá o mesmo áudio bit a bit;
- a PDC alinha dois caminhos com latências diferentes, conferido por um impulso.

---

## core/io

```cpp
class AudioDevice
{
public:
	virtual ~AudioDevice() = default;
	virtual auto open(const DeviceConfig& config) -> Result<DeviceInfo> = 0; // [main] rate, block, channels, latencies
	virtual auto start(AudioCallback& callback) -> Result<void> = 0;          // [main]
	//! [main] Returns only after the last callback has returned (the barrier): counted in this base class around
	//! every call into the callback, so every backend gets it
	virtual void stop() = 0;
};
```

- **Ordem dos dispositivos:** um `DummyDevice` (thread própria, ou passo manual nos testes) primeiro; depois
  CoreAudio (a máquina do usuário); depois JACK/PipeWire e ALSA.
- `DeviceInfo` traz as latências de entrada e saída que o dispositivo informa. A calibração de ida e volta
  (especificação, item 4) mede o resto e guarda nas preferências.
- **`DiskWriter`:** uma thread lê os blocos gravados da telemetria, escreve RF64, atualiza o cabeçalho a cada 2 s e,
  ao parar, fecha o arquivo e entrega o `Resource` para a transação que cria o take.
- **`ResourceStore`:** `import(path) -> Result<Resource>` `[worker]` (copia para `audio/`, converte a taxa, calcula o
  hash); `acquire(Resource) -> ResourceHandle` `[main]` (carrega numa thread de trabalho; o handle diz quando está
  pronto). Os dados carregados são imutáveis e compartilhados.

---

## core/session

```cpp
class Session
{
public:
	static auto create(const SessionConfig& config) -> Result<std::unique_ptr<Session>>; // [main]
	~Session();                                            // [main] the order below, reversed

	auto newProject(double sampleRate) -> Result<void>;     // [main]
	auto open(const std::filesystem::path& folder) -> Result<void>;  // [main] replays the journal if newer
	auto save() -> Result<void>;                            // [main] write tmp + rename; the journal restarts
	auto saveAs(const std::filesystem::path& folder) -> Result<void>;

	auto document() -> doc::Document&;                      // [main]
	auto actions() -> Actions&;                             // [main]
	auto preferences() -> Preferences&;                     // [main]
	auto plugins() const -> const PluginRegistry&;          // [any]
	auto resources() -> ResourceStore&;                     // [main]
	auto telemetry() -> Telemetry&;                         // (sinks registered by the UI)

	//! [main] Does everything that crosses into the main thread: delivers the document's diffs, installs a
	//! compiled program, drains the telemetry, collects the retirement, schedules the journal and the autosave.
	//! The UI calls it from its timer; tests and lmms-headless call it in their loop
	void pump();
};
```

**Ordem dos membros** (criados nesta ordem e destruídos na inversa, escrita uma vez, aqui):

1. `Preferences`
2. `SchemaRegistry` + `PluginRegistry` (os plugins internos registrados por uma lista explícita, sem inicialização
   estática mágica)
3. `ResourceStore`
4. `Document`
5. `Actions`
6. `Retirement`
7. `ParameterStore`, `Telemetry`, `Commands`
8. `Engine`, `Compiler` (e sua thread de trabalho)
9. `Journal`, `Autosave`, `DiskWriter`
10. `AudioDevice`, `MidiInput`: os últimos a nascer e **os primeiros a morrer**. Ao destruir, a barreira do
    `stop()` garante que nenhuma callback roda quando o resto começa a ir embora.

**`Actions`:** `add(ActionSpec)` `[main]`; `run(id, params) -> Result<void>` `[main]`; `list()`. Cada ação é uma
transação ou um comando do transporte.

**Pronto quando:**
- criar e destruir a sessão mil vezes seguidas, com projeto aberto e tocando no dispositivo de teste, sob ASan e
  LSan, não dá erro;
- salvar, abrir e salvar dá o mesmo `projeto.json` bit a bit;
- matar o processo no meio e abrir de novo volta à última ação, pelo diário.

---

## headless

```
lmms-headless new   <pasta> [--rate 48000]
lmms-headless render <pasta> <saida.wav>           # determinístico: o teste de referência usa isto
lmms-headless play  <pasta> [--seconds N]
lmms-headless record <pasta> --track t:18 --seconds N
lmms-headless action <pasta> <id> [json]           # roda uma ação e salva (para testes e scripts)
```

É o primeiro programa a funcionar e a prova de que o núcleo não precisa do Qt.

---

## Ordem de implementação

1. `state` (o valor, o JSON, os esquemas), com os testes e a decisão `immer` × nós próprios.
2. `document`, com os testes de transação e desfazer.
3. `render` com o dispositivo de teste: um programa à mão soando, a troca de programas sob TSan, a armadilha de
   alocação.
4. `session` + `headless`: `new`, `render` e `action` funcionando com um plugin de teste (um oscilador senoidal).
5. A primeira fatia (`arquitetura-nova.md`): CoreAudio, gravação, TripleOscillator, um efeito, salvar e abrir.
6. A `gui`, sobre o que já funciona sem ela.

## Decisões em aberto (para o usuário ou para a implementação)

- `immer` ou nós próprios: decide a medida do módulo `state`.
- A biblioteca de JSON: escrita própria ou uma sem Qt, decide o teste de ida e volta.
- O número de workers padrão: núcleos físicos − 1, ajustável nas preferências.
