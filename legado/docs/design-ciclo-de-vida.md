# Design do ciclo de vida dos objetos

O **como** do `LinuxMusicBeta/docs/plano-ciclo-de-vida.md`, que diz o **porquê** (as lacunas e as evidências) e a ordem das fases.
Este documento é escrito pela sessão de design. Quem implementa lê a seção da fase antes de começar e anota aqui,
na seção da fase, o que decidir na hora, para revisão.

Estado: **em suspenso.** O usuário pediu mudanças radicais. A proposta está em `LinuxMusicBeta/docs/arquitetura-nova.md`, e este
desenho (que remenda as classes atuais) só vale se ela for recusada. Não implementar a fase 2 antes dessa decisão.

---

## Princípios (valem para todas as fases)

### P0. A solução correta, não a de menor mudança

Decisão do usuário (19/11): "sempre a solução de arquitetura correta, nem que precise modificar todos os plugins";
"prefiro 10 funcionalidades que funcionem a 200 instáveis". Por isso:

- **Nada provisório e nada de compatibilidade.** Um sinal que muda de significado muda de nome, para que cada conexão
  seja revista. Uma API que muda de contrato substitui a antiga, sem atalho para a velha. Nenhum "por enquanto, até
  a fase N".
- **Todo plugin e todo chamador são reescritos** quando o contrato da base muda.
- **Uma funcionalidade que não cumpre o contrato fica desligada até cumprir**: ciclo de vida, tempo real, testes da
  fase. Um plugin que não passa sai da lista de plugins; uma ação da interface que não passa sai do menu. A lista do
  que desligar é proposta aqui e decidida pelo usuário.

### P1. Retirar e destruir são duas operações

- **Retirar** (`withdraw()`): o objeto sai do sistema **inteiro**. Ele sai das listas que o áudio percorre (sob a
  trava de mudanças), desconecta o que o áudio pode chamar nele, avisa quem o observa e para o que ele mesmo
  alimenta (entrada MIDI, play handles). Depois disso, nada fora do dono alcança o objeto.
- **Destruir** (`delete`): só libera. Um destrutor não emite sinal, não pega trava, não procura o objeto em lista
  nenhuma e não chama `Engine::...`.

Por quê: em C++, o destrutor da base roda depois que a parte derivada morreu. Todo trabalho de "sair do sistema"
feito ali enxerga um objeto pela metade (lacuna 1). Feito no `withdraw()`, o objeto ainda está inteiro.

### P2. Quem chama `withdraw()` é o dono, nunca o destrutor

A única porta de saída é uma função do dono: `Track::destroy`, `Track::deleteClip`, `Clip::destroy`,
`Clip::destroyLater`. Os destrutores ficam `protected`, e o compilador recusa um `delete` vindo de fora. Para guardar
num `std::unique_ptr` há o `Deleter` de cada tipo.

### P3. Quando dá para destruir logo, e quando é preciso esperar

O áudio segura a trava de mudanças durante o período inteiro (`AudioEngine::renderNextBuffer` → `lock_guard` em
`m_changeMutex`, `AudioEngine.cpp:411`), e os workers rodam dentro desse período. Então:

- **Retirado sob a trava, lido só sob a trava** (faixas, clipes, canais, efeitos nas listas): destruir logo depois do
  `withdraw()`, na thread principal. Nenhum período começa com o objeto na lista.
- **Lido sem a trava** (`Handoff`, atômicos, `RtShared`, threads de fora como o worker do LV2): esperar um período
  terminar, com `retire(..., After::Period)` (fase 3).
- **Destruir agora quebraria quem está na pilha** (a vista que pediu a remoção ainda está no meio do seu evento):
  `retire(..., After::Turn)`, na próxima volta do laço de eventos.

Esperar não conserta uma conexão direta do áudio para um objeto meio destruído (o bug do tempo, lacuna 5). A raiz
disso é o áudio emitir sinais, e isso acaba na fase 5. Independente dela, toda conexão que um objeto recebe de algo
que vive mais que ele (a música, o motor, outra faixa) é guardada pelo próprio objeto e desfeita **sob a trava**
dentro do `withdraw()` (o `ConnectionSet` de `Lifecycle`, abaixo). Nenhuma conexão desse tipo fica esperando o
`~QObject`.

### P4. Um dono por vez, também depois de retirado

Enquanto está publicado, o dono de um clipe é a faixa (`m_clips`; o pai do Qt é só o lugar na árvore de nomes, a
exceção já anotada em `regras-de-codigo.md`). Ao ser retirado, o objeto **sai da árvore do Qt** (`setParent(nullptr)`)
e passa a um único dono: quem o destrói já, ou a fila de `retire`. Assim, um pai do Qt que morra depois não o apaga
de novo.

### P5. O estado é verificável

Todo objeto com ciclo de vida guarda o estágio em que está. Em build de depuração:
- o destrutor confere que o objeto não está publicado;
- `withdraw()` confere que roda na thread principal.

Um caminho esquecido vira uma asserção em teste, não um use-after-free em produção.

---

## Vocabulário

| Estágio | Quer dizer | Entra por |
|---|---|---|
| `Made` | construído, ninguém o vê (nem o áudio, nem as vistas) | construtor |
| `Published` | nas listas; o áudio e as vistas o veem | `Track::publish`, `Clip::enterTrack` |
| `Withdrawn` | fora de tudo, ainda inteiro; só o dono o tem | `withdraw()` |
| (destruído) | — | `delete` pelo dono ou pela fila de `retire` |

`Made → Withdrawn` é permitido: um objeto construído e nunca publicado (um projeto que falha ao carregar) passa pelo
mesmo caminho, e o `withdraw()` só marca o estágio.

---

## Peça comum: `Lifecycle` (fase 2)

Arquivo novo `include/Lifecycle.h`. Não é `QObject`; é uma base pequena, herdada ao lado de `Model`.

```cpp
namespace lmms
{

//! The stage an object is in (legado/docs/design-ciclo-de-vida.md). Main thread only: the audio thread never reads it
//! (it sees the lists the object is in, or not)
class LMMS_EXPORT Lifecycle
{
public:
	enum class Stage : std::uint8_t { Made, Published, Withdrawn };

	auto stage() const -> Stage { return m_stage; }
	auto isPublished() const -> bool { return m_stage == Stage::Published; }

	Lifecycle(const Lifecycle&) = delete;
	auto operator=(const Lifecycle&) -> Lifecycle& = delete;

protected:
	Lifecycle() = default;
	//! Debug builds: a published object deleted is a path that skipped withdraw()
	~Lifecycle();

	//! Made → Published (asserts the stage and the main thread)
	void markPublished();
	//! Starts withdrawing: false when it was done already (withdraw() returns early; it may be called twice)
	auto beginWithdraw() -> bool;
	//! → Withdrawn
	void markWithdrawn();

	//! Connections this object receives from senders that outlive it (the song, the engine, another track):
	//! made through connectFrom(), cut by withdraw() under the change lock (cutConnections())
	template<class Sender, class Signal, class Receiver, class Slot>
	void connectFrom(const Sender* sender, Signal signal, const Receiver* receiver, Slot slot)
	{
		m_connections.push_back(QObject::connect(sender, signal, receiver, slot));
	}
	//! [under the change lock, in withdraw()]
	void cutConnections();

private:
	Stage m_stage = Stage::Made;
	bool m_withdrawing = false; //!< (between beginWithdraw() and markWithdrawn(): a second withdraw() returns)
	std::vector<QMetaObject::Connection> m_connections;
};

} // namespace lmms
```

- **Regra do `connectFrom`:** um objeto com ciclo de vida que se conecta a um sinal de quem vive mais que ele usa
  `connectFrom`, nunca `connect` direto. Exemplos: `SampleClip` ← `Song::tempoChanged`, `TempoSyncKnobModel` ←
  `Song::tempoChanged`, faixa ← `Mixer`. Conexões entre o objeto e os próprios membros (que morrem com ele) continuam
  com `connect`. Regra de busca para o CI: `connect(Engine::` e `connect(song` em classes com `Lifecycle`.

- `~Lifecycle()` faz `assert(m_stage != Stage::Published)` e, se as checagens de `PluginChecks` estiverem ligadas,
  `ThreadCheck::violation(...)`, para pegar também em release com checagens.
- Não é atômico: só a thread principal mexe no estágio, e `markPublished`/`beginWithdraw` conferem isso com
  `ThreadCheck::isMainThread()`.
- O construtor de cópia de `Clip` (`Clip.cpp`, `Clip(const Clip&)`) passa a inicializar `Lifecycle` do zero (`Made`),
  nunca copiar o estágio.

---

## Fase 2: `Track` e `Clip`

### Por que esta fase primeiro

Ela conserta, pela raiz:
- o use-after-free do Piano Roll (lacuna 1);
- o `delete` de um clipe que ainda está na faixa (`AutomatableModel.cpp:192`);
- `destroyedClip`/`destroyedMidiClip`/`destroyedTrack` emitidos por objetos meio destruídos;
- um caso latente achado agora, ao ler o caminho de remoção da vista (abaixo).

**Caso latente (lido no código, não reproduzido):** `ClipView::remove` (`ClipView.cpp:195-204`) e a mudança de
faixa (`ClipView.cpp:1255-1263`) tiram o clipe da faixa e chamam `clip->deleteLater()`. Até a próxima volta do laço,
o clipe está fora de `m_clips`, mas continua filho do Qt da faixa. Se a faixa for destruída nesse intervalo,
acontece o seguinte:
1. `~Track` só apaga os clipes de `m_clips`;
2. o `~QObject` da faixa apaga os filhos, inclusive esse clipe;
3. `~Clip` chama `getTrack()->removeClip(this)` numa faixa cujo `m_clips` já foi destruído;
4. depois, o `deleteLater` pendente apagaria o clipe pela segunda vez.

Precisa de uma remoção da faixa na mesma volta do laço, ou de um laço aninhado; por isso é raro, mas é o padrão de
dois donos (P4).

### Classe `Track`

**Cabeçalho** (`include/Track.h`):

```cpp
class LMMS_EXPORT Track : public Model, public JournallingObject, public Lifecycle
{
public:
	//! Withdraws @p track (see withdraw()) and deletes it. The only way a track is deleted
	static void destroy(Track* track);
	struct Deleter { void operator()(Track* track) const { destroy(track); } };

	/**
	 * Takes the track out of everything, whole: tells its views and editors, stops what it feeds (withdrawParts()),
	 * withdraws and deletes its clips, then leaves its container and the automation under the change lock,
	 * together with what the derived track unlists (unlistParts()). Main thread; a second call does nothing.
	 */
	void withdraw();

protected:
	//! Only frees: no signals, no lock, no lists, no Engine (see legado/docs/design-ciclo-de-vida.md, P1)
	~Track() override;

	//! [derived] Stops what the track feeds and what feeds it, while it is whole; the change lock is NOT held (it
	//! may wait, e.g. for the MIDI input). Called before its clips go
	virtual void withdrawParts() {}
	//! [derived] Leaves the lists the audio thread reads; called UNDER the change lock, with the base's own
	//! unlisting: brief, no signals, no waiting
	virtual void unlistParts() {}

signals:
	//! The track is about to go, still whole (emitted by withdraw())
	void aboutToBeRemoved();
};
```

O sinal `destroyedTrack` **deixa de existir** e no lugar dele entra `aboutToBeRemoved` (P0). Como o momento muda, de
"no destrutor" para "inteira, antes de sair", cada conexão é revista ao trocar o nome.

**`Track::withdraw()`, nesta ordem:**

1. `if (!beginWithdraw()) { return; }` (confere a thread principal).
2. `emit aboutToBeRemoved();`: as vistas fecham e os editores largam o que é dela, **com a faixa inteira**.
3. `withdrawParts();`: a parte derivada para o que alimenta (lista abaixo).
4. Os clipes saem, um por um, com a faixa inteira: `while (!m_clips.empty()) { Clip::destroy(m_clips.back()); }`.
   Cada um passa pelo próprio `Clip::withdraw()`. É aqui que o Piano Roll larga o `MidiClip` e chama `disconnect`
   em `pianoModel()`/`microtuner()`, que ainda existem.
5. Os plugins da faixa são retirados, ainda inteiros: o instrumento e cada efeito da cadeia passam por
   `PluginInstance::withdraw()` (abaixo). Nenhum plugin se desregistra no próprio destrutor.
6. Um só momento sob a trava, com **tudo** que o áudio lê:
   ```cpp
   {
   	const auto guard = AudioEngine::changesGuard();
   	m_trackContainer->unlistTrack(this);         // (a variante de removeTrack que supõe a trava já pega)
   	AutomationClip::forgetObjectsUnderLocked(this);
   	Engine::audioEngine()->unlistPlayHandlesOf(this, retired); // (abaixo)
   	unlistParts();
   	cutConnections();
   }
   ```
   As funções chamadas aqui supõem a trava já pega e não emitem sinal. Cada uma que hoje pega a trava sozinha
   ganha a variante "já travado", e a versão antiga deixa de existir onde só este caminho a usava.
7. Fora da trava: os play handles retirados no passo 6 vão para a fila de `retire` (`After::Turn`). Não são
   apagados na thread de áudio nem ali dentro da trava.
8. `markWithdrawn();`

**Play handles da faixa:** `AudioEngine::unlistPlayHandlesOf(const Track*, std::vector<std::unique_ptr<PlayHandle,
PlayHandle::Disposer>>& out)` tira das três listas (`m_playHandles`, `m_newPlayHandles`, `m_playHandlesToRemove`)
os handles da faixa e os devolve ao chamador, que é o dono deles a partir dali. Isso substitui
`removePlayHandlesOfTypes`/`removePlayHandlesOf`, que hoje apagam no meio. A posse completa dos handles é da fase 3
(`PlayHandleRef`). Esta fase já não apaga handle sob a trava nem no áudio.

**`Track::destroy(track)`:** `if (!track) return; track->withdraw(); delete track;`.

**`~Track()`:** sem `lock()`, sem emitir sinal, sem `removeTrack`. Só
`assert(m_clips.empty() && "a track's clips go in withdraw()")`. O `blockSignals(true)` deixa de ser necessário.

**O que sai dos destrutores das faixas e vai para `withdrawParts()`/`unlistParts()`** (os destrutores que hoje
tocam no motor, lista da auditoria):

| Faixa | Hoje no destrutor | Vai para |
|---|---|---|
| `InstrumentTrack` (`InstrumentTrack.cpp:203`) | `m_midiPort.closeInput()`, `midiRecorder().forget(this)`, desligar o auto-assign de MIDI, `removePlayHandlesOfTypes(...)` (`:678`) | `withdrawParts()`: fechar a entrada, esquecer no gravador, auto-assign; os play handles em `unlistParts()` se a remoção não pegar a trava sozinha, senão em `withdrawParts()` |
| `SampleTrack` (`SampleTrack.cpp:83-85`) | `removePlayHandlesOfTypes(this, SamplePlayHandle)` | `withdrawParts()` |
| `AudioInputTrack` (`AudioInputTrack.cpp:65-67`) | `removePlayHandlesOfTypes(this, InputMonitorHandle)` | `withdrawParts()` |
| `AudioTrack` (`AudioTrack.cpp:297-301`) | `removePlayHandlesOfTypes(this, AudioTrackPlayHandle)` | `withdrawParts()` |
| `PatternTrack` (`PatternTrack.cpp:68-70`) | `removePlayHandlesOfTypes(...)` e o que mais houver | `withdrawParts()` |

O implementador confere cada destrutor inteiro: tudo que chama `Engine::`, emite sinal ou procura em lista vai para
um dos dois ganchos. Ao destrutor fica só a liberação de membros.

### `PluginInstance` e os plugins (nesta fase, porque a faixa retira os seus)

- `PluginInstance` herda `Lifecycle` e ganha `withdraw()`, chamado pela faixa (passo 5) e pela `EffectChain` ao
  remover um efeito. Nesta ordem:
  1. `deactivate()`, que já existe;
  2. `withdrawParts()` do plugin, um gancho novo em `Plugin`, para o plugin parar threads e workers próprios (o
     worker do LV2 faz o join aqui, **antes** de a instância ser liberada: lacuna 2);
  3. sob a trava: os play handles do instrumento saem por `unlistPlayHandlesOf(instrument, retired)`, e o plugin
     corta as próprias conexões.
- **Todos os plugins são revistos.** Sai de cada destrutor o `Engine::audioEngine()->removePlayHandlesOf(this)`:
  Sf2Player, GigPlayer, OpulenZ, AvlDrums, DrumMachines, Lv2Instrument, Carla, ZynAddSubFx, Vestige (a lista sai de
  `grep -rn removePlayHandlesOf plugins`). Com isso o destrutor só libera. O destrutor de `Plugin` fica `protected`,
  e o único dono é o `PluginInstance`.
- `Lv2Proc::shutdownPlugin()` (`Lv2Proc.cpp:515-524`) passa a ser: join do worker → `deactivate` → `free`.
- **Regra de busca para o CI:** nenhum destrutor em `plugins/` chama `Engine::`.

**Quem destrói faixas hoje e passa a usar `Track::destroy`:** todos já usam (o destrutor é `protected`). Confira só
que `TrackContainer::clearAllTracks` e `~TrackContainer` passam por `destroy`. A asserção de `~Lifecycle` pega
qualquer um que não passe.

### Classe `Clip`

**Cabeçalho** (`include/Clip.h`):

```cpp
class LMMS_EXPORT Clip : public Model, public JournallingObject, public Lifecycle
{
public:
	//! Withdraws and deletes @p clip now. Not from inside an event of one of its views (see destroyLater())
	static void destroy(Clip* clip);
	struct Deleter { void operator()(Clip* clip) const { destroy(clip); } };

	//! Withdraws the clip now, and deletes it on the main loop's next turn: for a view removing its own clip from
	//! an event (the view is still on the stack). Once withdrawn, the clip leaves the Qt tree: its track may go
	//! first without deleting it again
	void destroyLater();

	/**
	 * Takes the clip out of everything, whole: tells its views and editors (destroyedClip()), stops what it plays
	 * (withdrawParts()), then leaves its track and the song under the change lock, with what the derived clip
	 * unlists (unlistParts()). Main thread; a second call does nothing.
	 */
	void withdraw();

protected:
	//! Only frees (see P1)
	~Clip() override;

	virtual void withdrawParts() {} //!< [derived] no lock held; may wait
	virtual void unlistParts() {}   //!< [derived] under the change lock; brief

signals:
	//! About to go, still whole (emitted by withdraw())
	void aboutToBeRemoved();
};
```

`destroyedClip` e `destroyedMidiClip` **deixam de existir** (P0). Quem precisa do `MidiClip*` (o Piano Roll, o
gravador de passos) conecta `aboutToBeRemoved` com o clipe como contexto e o captura.

**`Clip::withdraw()`, nesta ordem:**

1. `if (!beginWithdraw()) { return; }`
2. `emit aboutToBeRemoved();`
3. `withdrawParts();`: o `SampleClip` para de tocar (hoje no destrutor; a auditoria confirmou que `~SampleClip`
   faz isso antes dos membros morrerem, então é só mudar de lugar).
4. Um só momento sob a trava, se o clipe está publicado:
   ```cpp
   {
   	const auto guard = AudioEngine::changesGuard();
   	m_track->unlistClip(this);  // (erase em m_clips e Song::forgetClip, supondo a trava pega)
   	Engine::audioEngine()->unlistPlayHandlesOf(this, retired); // (os do SampleClip)
   	unlistParts();
   	cutConnections();            // ex.: SampleClip ← Song::tempoChanged, feito com connectFrom
   }
   ```
5. Fora da trava: `m_track->clipLeft(this)`: o resto do atual `removeClip` (`clip->disconnect(track)`,
   `checkLanes()`, `updateLength()`, `setModified()`). Não roda se a faixa estiver retirando (o estágio da faixa diz
   isso), porque ela vai embora inteira.
6. `markWithdrawn();`

**`Clip::destroy(clip)`:** `clip->withdraw(); delete clip;`.

**`Clip::destroyLater()`:**
`withdraw(); setParent(nullptr); Engine::retirement().retire(std::unique_ptr<Clip, Clip::Deleter>{this}, After::Turn);`
O `Deleter` faz só o `delete`, porque o clipe já foi retirado. A fila de `retire` entra nesta fase (abaixo), e não há
`deleteLater` provisório.

**`~Clip()`:** sem `emit`, sem `removeClip`. O `TODO` em `ClipView.cpp:200-203` some, porque o próprio comentário
pede exatamente isso.

**`Track::removeClip` e `Track::deleteClip` deixam de existir.** No lugar:
- `unlistClip(Clip*)`: privada, chamada sob a trava por `Clip::withdraw()`;
- `clipLeft(Clip*)`: privada, o resto fora da trava.

Os dois ficam acessíveis ao `Clip` por `friend class Clip`. Apagar um clipe é só `Clip::destroy(clip)` ou
`clip->destroyLater()`. `Track::deleteClips()` fica, porque é uma operação da faixa (esvaziar), e chama
`Clip::destroy` em cada um.

### A fila de `retire` (entra nesta fase)

`include/Retirement.h`. O dono é o `Engine`, num `std::unique_ptr`, criado no `init()` e esvaziado e destruído no
`destroy()`, com o áudio parado.

```cpp
namespace lmms
{

//! Deletes what is handed over once nothing can reach it any more (legado/docs/design-ciclo-de-vida.md, P3). Main thread
class LMMS_EXPORT Retirement
{
public:
	enum class After : std::uint8_t
	{
		Turn,   //!< the main loop's next turn (a caller still on the stack, e.g. a view's event)
		Period, //!< an audio period ended after this call (read without the change lock: Handoff, atomics)
	};

	template<class T, class D>
	void retire(std::unique_ptr<T, D> object, After after);

	//! Deletes what is due; called by a main-thread timer and on Engine::destroy() (then everything is due)
	void collect(bool everything = false);
};

} // namespace lmms
```

- Cada item guarda o objeto apagado por tipo (um `std::unique_ptr<void, void(*)(void*)>` montado a partir do
  `unique_ptr<T, D>`), o `After` e, para `Period`, o valor de `periodsRendered()` no momento da chamada.
- `Turn`: apagado no próximo `collect()`, que roda por um `QTimer` de 0 ms disparado na primeira entrega da volta.
- `Period`: apagado quando `periodsRendered()` passou do valor guardado. O incremento em `AudioEngine.cpp:435` passa a
  ser `memory_order_release`, e a leitura `acquire`. Com o motor parado o contador não anda, e nesse caso
  `collect()` só apaga quando o motor diz que nenhum período está em andamento, o que exige a barreira do
  dispositivo (lacuna 6, fase 5). Até a fase 5, `Period` só é usado com o motor rodando ou no `Engine::destroy`.
- Ninguém mais usa `deleteLater` para objetos do modelo (faixas, clipes, plugins, play handles). `deleteLater` fica
  para widgets.

**Mover um clipe de faixa** (`ClipView.cpp:1255-1263`): hoje cria a cópia na outra faixa, tira o original e dá
`deleteLater`. Passa a ser `copy`, depois `clip->destroyLater()`. É o mesmo caminho de remover.

### Lugares que mudam (lista para quem implementa)

- `include/Lifecycle.h`, `src/core/Lifecycle.cpp`: novos.
- `include/Track.h`, `src/core/Track.cpp`: `withdraw`, `destroy`, `~Track`, `removeClip` → `unlistClip`/`clipLeft`,
  `deleteClip`, `deleteClips`; `publish` chama `markPublished()` depois de `tc->addTrack(t)`.
- `include/Clip.h`, `src/core/Clip.cpp`: `withdraw`, `destroy`, `destroyLater`, `~Clip` protegido;
  `Clip::enterTrack()` chama `markPublished()`.
- Faixas: `InstrumentTrack`, `SampleTrack`, `AudioInputTrack`, `AudioTrack`, `PatternTrack`, `AutomationTrack`,
  `FolderTrack`: destrutores → `withdrawParts`/`unlistParts`.
- Clipes: `MidiClip` (o `emit` do `:79`), `SampleClip`, `AutomationClip`, `PatternClip`, `AudioItem`.
- Quem apaga clipes:
  - `ClipView.cpp:195-204` e `:1255-1263` → `destroyLater()`;
  - `AutomatableModel.cpp:192` (`delete p`) → `Clip::destroy(p)`;
  - `PatternEditor.cpp:177`, `:275` e `TrackOperationsWidget.cpp:203` já usam `deleteClips()` e não mudam.
- Qualquer outro `delete` de clipe passa a não compilar: o destrutor é `protected`.
- Todas as conexões a `destroyedTrack`, `destroyedClip` e `destroyedMidiClip` (`grep -rn "destroyed\(Track\|Clip\|MidiClip\)"`)
  são revistas e passam a `aboutToBeRemoved`.
- Todas as conexões que faixas, clipes e plugins recebem da `Song`, do `Engine`, do `Mixer` ou de outra faixa passam a
  `connectFrom`.
- `include/Retirement.h`, `src/core/Retirement.cpp`: novos; `Engine` passa a ser dono.
- `PluginInstance` (`withdraw`), `Plugin` (destrutor `protected`, gancho `withdrawParts`), todos os plugins da lista
  acima, e o `Lv2Proc::shutdownPlugin`.
- `AudioEngine`: `unlistPlayHandlesOf(...)` no lugar de `removePlayHandlesOf`/`removePlayHandlesOfTypes`.

### Pronto quando

- **Teste novo `LifecycleTest`** (no `ctest`, também sob ASan):
  1. uma faixa de instrumento com um `MidiClip` e um observador que, no `destroyedMidiClip`, faz o mesmo que o Piano
     Roll (`instrumentTrack()->pianoModel()->disconnect(...)`, `microtuner()->keymapModel()`). `Track::destroy` sem
     erro do ASan;
  2. `clip->destroyLater()` seguido de `Track::destroy(track)` na mesma volta do laço: o clipe é apagado uma vez só;
  3. um `AutomationClip` global apagado por `Clip::destroy` com a música tocando;
  4. a asserção de `~Lifecycle` dispara num `delete` direto (com um tipo de teste que exponha o destrutor).
- A bateria de sempre: `ctest`, `tools/check/ab.sh`, ASan/UBSan e TSan (é memória e thread), e abrir/fechar a
  interface com projetos de demonstração no build de depuração (a asserção pega caminhos esquecidos).
- Nenhum destrutor de `Track`/`Clip` ou derivado chama `Engine::`, emite sinal ou pega trava. Regra de busca para o
  CI: dentro dos corpos de `~*Track` e `~*Clip`, nada de `Engine::`, `emit `, `Guard`, `lock(`.

### Decidido (antes estava em aberto)

- Tudo que o áudio lê sai num só momento sob a trava (passo 6 da faixa, passo 4 do clipe), com variantes "já
  travado". Medir com `longestChangeHold()` antes e depois. Se o momento ficar longo demais para faixas muito
  grandes, a resposta certa é deixar de percorrer listas sob a trava (a fase 3 troca a busca por tipo em
  `m_playHandles` pela posse direta), e não voltar a pegar a trava várias vezes.

### Funcionalidades a desligar se não passarem (P0)

Proposta para o usuário decidir, a partir das auditorias:

- **Vestige e VstEffect**: a janela "Manage parameters" apaga modelos do plugin e sobrevive à troca de instrumento
  (lacuna 4). Ela fica desligada até ser refeita com os modelos só no plugin.
- **Xpressive**: troca a expressão sem trava enquanto toca e compila a expressão no áudio a cada nota. Fica
  desligado até compilar na thread principal e publicar por `RtShared`.
- **JACK**: a volta depois de o servidor cair deixa portas e cliente MIDI soltos. Até a fase 5, cair o servidor
  encerra o dispositivo com aviso, em vez de tentar voltar.

---

## Fases 3 a 7

A escrever, depois da pesquisa externa. Já decidido:

- **Fase 3:** `PlayHandleRef` (id + geração), no modelo do `MidiPort::serial()`. `addPlayHandle(std::unique_ptr)`.
  Handles nunca apagados no áudio: vão para a `Retirement`.
- **Fase 4:** `ModelView<T>`.
- **Fase 5:** a caixa de correio do áudio para a interface, as asserções de thread e as ações adiadas da trava.
- **Fase 6:** as globais.
- **Fase 7:** o contrato de tempo real dos plugins.
