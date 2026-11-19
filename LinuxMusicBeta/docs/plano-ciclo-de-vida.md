# Ciclo de vida e memória: diagnóstico e plano de arquitetura

Levantamento de 18/11, só de leitura: quatro auditorias (núcleo, interface, plugins, threads) mais a leitura das
classes base. Nada foi mudado no código. Os números de linha são os de 18/11 e podem andar.

A tese: boa parte dos problemas de memória não está no código de cima, mas no que a camada de baixo **deixa de
fornecer**. Sem um molde de ciclo de vida, cada faixa, clipe, plugin e vista improvisa o seu, e os erros se repetem
do mesmo jeito em lugares diferentes. Por isso o plano ataca primeiro as bases; os bugs pontuais, no fim, são a
evidência.

Complementa `docs/regras-de-codigo.md` e as fases 4 e 8 de `docs/plano-refatoracao.md`.

**Revisão de 19/11** (depois de conferir com a sessão da gravação):

- O programa troca o `operator new`/`delete` (`src/core/RealtimeNew.cpp`): na thread de áudio, `new` sai de um pool de
  tempo real (`RealtimeAllocator`), e um `delete` de memória do sistema vai para `deferFree`, liberado depois por
  outra thread. Então "`new` na thread de áudio" não quer dizer `malloc` do sistema, e "liberação na thread de
  áudio" não quer dizer `free` ali. O que continua sendo problema é: passar do tamanho do pool (aí cai no sistema),
  o trabalho de CPU de quem constrói ou destrói (parse do exprtk, destrutores que desregistram portas) e as travas.
  As lacunas 2 e 9 foram corrigidas com isso.
- Existe o `AudioThreadAllocationTest` (`tests/src/core/AudioThreadAllocationTest.cpp`), que roda pelo `ctest` com
  o `AllocationObserver` e vê o `malloc` do sistema. Ele toca 22 instrumentos e todos os efeitos internos com os
  parâmetros padrão, e passou em 18/11. A lacuna 9 dizia que não havia teste assim: estava errado.
- Já consertados antes deste levantamento (as auditorias leram o código com eles): entrada MIDI entregue pela
  thread de áudio só a portas registradas (90c1f30a7); janela da faixa de áudio que sobrevivia à faixa
  (b353b1efe); `m_notes` sob a trava de mudanças (7a420001b); LB302 com ponteiro de nota apagada (cc22895ef);
  Guitarix alocando no áudio (b65696066). Depois do levantamento: faixa de áudio derrubando o programa ao abrir o
  projeto (3f7d88786, não muda nenhum item daqui).

## O que já está certo (conferido)

- Faixas só são apagadas por `Track::destroy` (destrutores `protected`).
- `EffectChain::removeEffect`/`clear` e `Mixer::deleteChannel` tiram sob a trava e apagam depois.
- `PeakController` e `ControllerConnection` sem `delete this`; a fila de automação gravada é limpa por `forgetClip`.
- `ModelView::m_model` é `QPointer`; `AutomationClip` guarda `QPointer<AutomatableModel>`.
- `AudioRecorder`: sessão zerada sob a trava, thread de disco com join, a faixa do take conferida de novo.
- A aposentadoria de blocos do `AudioStream`/`DiskReader` e o `Handoff<T>` nunca liberam na thread de áudio.
- Fila de MIDI de entrada, workers do motor, autosave, `ThreadedExportManager`, pool do `PeakCache`.
- Dados por nota dos instrumentos quase todos em `makePluginData`/`destroyPluginData`; troca de amostra no AFP,
  Patman, BitInvader e Watsyn sob `changesGuard()` ou por troca atômica.
- A ordem do `Engine::destroy`, tirando a brecha do JACK (lacuna 6).

## O que o plano antigo diz e não vale

- Os ponteiros estáticos do `Engine` continuam crus (`new`/`deleteHelper`, `Engine.cpp:47-57`, `110-140`);
  `Engine::s_instanceOfMe` nunca é apagado. A fase 4 empurrou para a fase 8, que não mexeu neles.
- "ASan limpo" no Mac não diz nada sobre vazamento: o LeakSanitizer não roda com o clang da Apple. Os vazamentos
  abaixo passaram por isso.
- "O JACK de áudio e de MIDI sabem quando o outro some" vale, menos depois de o servidor cair e voltar (lacuna 6).
- "O piano roll larga o clipe apagado" vale, mas o caminho de largar é que lê memória liberada (lacuna 1).

---

## Lacuna 1: destruição numa fase só

**O que falta na base:** o trabalho de "sair do sistema" (sair das listas do áudio, desconectar, avisar quem
observa) mora no destrutor da classe base, que em C++ roda depois que a parte derivada já morreu.

Evidências:

- **Piano Roll ao apagar a faixa** (use-after-free, conferido). `~Track` apaga os clipes (`Track.cpp:84-87`)
  depois que `~InstrumentTrack` já destruiu `m_piano` e `m_microtuner` (`InstrumentTrack.h:425-427`). `~MidiClip`
  emite `destroyedMidiClip` → `PianoRoll::hideMidiClip` → `setCurrentMidiClip(nullptr)`, que faz
  `instrumentTrack()->pianoModel()->disconnect(this)` e o mesmo no `microtuner()` (`PianoRoll.cpp:850-864`,
  `948-953`). O `blockSignals(true)` do `~Track` só bloqueia os sinais da faixa, não os dos clipes.
  Passos: abrir um clipe no Piano Roll e apagar a faixa.
- **`~Clip` é público** (`Clip.h:58`) e só sai da lista no destrutor da base (`Clip.cpp:87-95`).
  `AutomatableModel.cpp:192` faz `delete p` num `AutomationClip` da faixa global de automação: o áudio lê essa faixa
  todo tique (`Song::automatedValuesAt`, `Song.cpp:1776`). Carregar um preset antigo (com `<automationpattern>`) com
  a música tocando pode ler memória liberada.
- `destroyedClip()` e `destroyedMidiClip` (`MidiClip.cpp:79`) são emitidos por objeto meio destruído.
- **Cada instrumento lembra sozinho de se desregistrar:** `Engine::audioEngine()->removePlayHandlesOf(this)` em 9
  destrutores (Sf2Player, GigPlayer, OpulenZ, AvlDrums, ...). `PluginInstance::detach()`
  (`PluginInstance.cpp:177`) poderia fazer por todos antes do `m_plugin.reset()` e não faz.
- As conexões só caem no `~QObject`, depois dos membros derivados (é o que abre a lacuna 5).

**Mudança:** ciclo de vida explícito, com estados controlados pela base: `Criado → Publicado → Retirado →
Destruído`.

- O framework chama `withdraw()` antes de qualquer destrutor: sai das listas do áudio, desconecta tudo, avisa os
  observadores.
- Destrutores `protected`; a única saída é um `Deleter` (como `Track::Deleter`), levado até o fim: os clipes saem
  dentro de `Track::destroy`, não no `~Track`; `Clip` ganha `Clip::Deleter` que passa por `Track::deleteClip`.
- `PluginInstance::detach()` faz o `removePlayHandlesOf` e a desconexão; os 9 destrutores perdem essa linha.
- Em depuração, o destrutor da base faz `assert(estado == Retirado)`.
- Vale para: Track, Clip, Plugin (via `PluginInstance`), PlayHandle, Controller, AudioBusHandle, canal do mixer.

## Lacuna 2: não há como recolher memória, só a trava global

**O que falta na base:** a única ferramenta é a trava que para o mundo (`requestChangeInModel`). O padrão é tirar
da lista sob a trava e dar `delete` logo depois, mas o objeto continua alcançável por `DirectConnection` vinda do
áudio, por threads que a trava não cobre, ou pelo último `shared_ptr`, que destrói o objeto na thread de áudio. A
memória em si é tratada pelo `operator delete` do programa (`deferFree` na thread de áudio); o que falta é um
mecanismo para o **objeto**: destruí-lo (rodar o destrutor) fora do áudio e só quando ninguém mais o alcança.

Evidências:

- **LV2:** `Lv2Proc::shutdownPlugin()` (`Lv2Proc.cpp:515-524`) faz `lilv_instance_deactivate`/`free` antes de
  `m_worker.reset()` (o join). O worker pode estar dentro de `work()` (`Lv2Worker.cpp:150-160`), por exemplo
  carregando um arquivo. Ao remover, recarregar, mudar a taxa ou fechar o projeto.
- **TripleOscillator** (`TripleOscillator.cpp:138-139` duplo clique na onda; `291-292` `loadSettings`): atribui
  `m_sampleBuffer` e `m_userAntiAliasWaveTable` (`shared_ptr`) sem trava enquanto `playNoteImpl` copia os mesmos
  (`338-341`). O contador de referências pode corromper: use-after-free. Ao carregar onda ou preset tocando.
- **Xpressive** (`Xpressive.cpp:529/532` editor; `151-152` `loadSettings`): troca o `QByteArray` de
  `m_outputExpression[i]` enquanto o `playNoteImpl` passa `constData()` ao exprtk. Ao digitar com notas tocando.
- **Destrutores pesados na thread de áudio** (a memória é adiada pelo `operator delete`, mas o destrutor roda ali):
  `PlayHandle::dispose()` faz `delete this` (`PlayHandle.h:94`) dentro das etapas do render (amostras feitas na
  interface, com `Sample` e `AudioBusHandle`, cujo destrutor ainda chama `unregisterPort`);
  `SamplePlayHandle::dispose` sem pool (`SamplePlayHandle.cpp:102`); Patman solta o último `Sample` em
  `deleteNotePluginData` depois de `loadPatch` (`:235`, `:402`); TripleOscillator solta a onda antiga em
  `destroyPluginData`; FreeBoy o `own` do fallback; Xpressive `delete ExprSynth`. Falta medir se algum desses
  destrutores trava ou faz trabalho longo (o `unregisterPort` é o suspeito).

**Mudança:** duas peças na base.

- **`AudioEngine::retire(std::unique_ptr<T>)`**: anota o período atual e libera na thread principal quando o áudio
  já passou dele. É o esquema do `AudioStream`/`DiskReader`, generalizado. Toda remoção de algo que o áudio enxerga
  sai por aqui, e `dispose()` nunca dá `delete` na thread de áudio.
- **`RtShared<T>`**: publicação atômica da interface para o áudio; o valor antigo vai para o `retire`.
- No LV2: join do worker (ou `m_workLock`) antes de liberar a instância. Regra geral: dono de thread faz join antes
  de liberar o que ela usa.

## Lacuna 3: posse ambígua dos play handles

**O que falta na base:** o motor é dono dos handles, mas recebe ponteiros crus, e quem criou guarda ponteiros crus.
Não existe "referência que sabe se o objeto ainda vive".

Evidências:

- **FileBrowser:** `AudioEngine::clear()` → `clearInternal()` (`AudioEngine.cpp:531-541`) apaga os handles que não
  vivem com a faixa, inclusive o que `FileBrowserTreeWidget::m_previewPlayHandle` guarda. Depois,
  `mouseReleaseEvent` lê `m_previewPlayHandle->type()` (`FileBrowser.cpp:876-879`) e `stopPreview()` chama
  `removePlayHandle(solto)`. Passos: música tocando, "deixar a prévia terminar" ligado, Stop, clicar outro arquivo.
- **PresetPreviewPlayHandle** (`PresetPreviewPlayHandle.cpp:187-195`): `addPlayHandle` em xrun crítico devolve a
  nota ao pool na hora (`AudioEngine.cpp:794-808`); `m_previewNote` e o atômico do contêiner de prévia apontam para
  uma vaga que pode ser de outra nota (`isFinished()` `:222`, `noteOff()` no destrutor `:207`).
- **Handles terminados nunca recolhidos:** `renderStageEffects` (`AudioEngine.cpp:330-336`) pula os de afinidade com
  outra thread. O TapTempo (`TapTempo.cpp:60`) deixa um `SamplePlayHandle` com `Sample` e `AudioBusHandle` por
  batida, processado todo período até um Stop.
- **`clearNewPlayHandles`** (`AudioEngine.cpp:509-525`, chamado por `Song::loadingCancelled`) descarta sem
  liberar: as vagas de nota não voltam ao pool, e `InstrumentTrack::m_notes[key]` pode apontar para nota nunca
  adotada.
- **`~AudioEngine`** (`:159-185`) não libera o que sobra em `m_playHandles`, `m_playHandlesToRemove` e
  `m_newPlayHandles`; o handle do metrônomo vaza a cada `Engine::destroy`.
- `NotePlayHandle::m_parent` (`NotePlayHandle.cpp:146`) é cru; `~NotePlayHandle` não zera o `m_parent` dos filhos.
- `Metronome::release()` (`Metronome.cpp:142-153`) manda o handle para a remoção e já faz `m_bus.reset()`; só não
  quebra porque roda depois de `stopProcessing()`.
- Comparação de endereço como "ainda vive?": `TrackChannelControls::m_boundChannel`
  (`TrackChannelControls.cpp:120-126`), `MixerView::m_collapsedGroups` (`MixerView.cpp:424`). Um canal novo no mesmo
  endereço passa no teste.

**Mudança:**

- `addPlayHandle(std::unique_ptr<PlayHandle>)` devolve um `PlayHandleRef` (id + geração) que o motor resolve sob a
  trava. `removePlayHandle` de um handle desconhecido não faz nada.
- `clear()` não apaga handles com dono externo (ou avisa o dono); `clearNewPlayHandles` e `~AudioEngine` liberam.
- O mesmo padrão de referência com geração substitui `m_parent` das notas, as comparações de endereço e os sinais
  em fila que levam ponteiro cru (lacuna 4).

## Lacuna 4: contrato modelo ↔ vista fraco

**O que falta na base** (`include/ModelView.h`, `src/gui/ModelView.cpp`):

- a vista é dona do modelo **conforme uma flag** (`isDefaultConstructed`);
- quem chama `setModel` precisa dizer se o modelo antigo ainda vive (`isOldModelValid`);
- `castModel` não tem tipo, então as vistas copiam o modelo para um `T* m_x` cru, o que anula o `QPointer` da base;
- não há gancho "o modelo morreu"; a desconexão é `model->sub()->disconnect(this)`, que atravessa o modelo.

Evidências:

- **Cópias cruas do modelo** (`m_x = castModel<T>()`): `LfoControllerDialog.cpp:250`, `PeakControllerDialog.cpp:83`,
  `SampleTrackWindow.cpp:198`, `InstrumentTrackWindow.cpp:348`, `EnvelopeGraph.cpp:57`,
  `InstrumentFunctionViews.cpp:85/134/218`, `EnvelopeAndLfoView.cpp:215`, `PianoView.cpp:262`,
  `InstrumentSoundShapingView.cpp:122`, `TrackView.cpp:403`, `Vestige.cpp:713`, `Patman.cpp:653`. Outros campos
  crus: `ClipView::m_clip` (`ClipView.h:306`), `TrackView::m_track` (`:273`), `PianoRoll::m_midiClip` (`:383`),
  `StepRecorder::m_midiClip`, `EffectControlDialog::m_effectControls`, `MidiClipView`/`SampleClipView`/
  `AutomationClipView::m_clip`, `PatternTrackView::m_patternTrack`, `MidiCCRackView::m_track`. A vista de um clipe
  apagado só some no `deleteLater`; dentro de um laço aninhado (menu, modal) ela ainda aparece em
  `findChildren<selectableObject*>` (`Rubberband.cpp:79`, `SongEditor.cpp:532-536`).
- **Vista que sobrevive à faixa:** `TrackView` liga `destroyedTrack` a `close()` com `WA_DeleteOnClose`
  (`TrackView.cpp:100-103`); `~InstrumentTrackView` faz `delete model()->m_midiPort.m_readablePortsMenu`
  (`InstrumentTrackView.cpp:245-246`), e `updateActivity`/`corruptStateUpdate` (`584-602`) leem `model()` a cada
  `periodicUpdate`. Quando a faixa morre primeiro (restauração do histórico → `clearAllTracks`,
  `TrackContainer.cpp:87-91`; `~PatternTrack`), é ponteiro nulo. `makeFxButton` (`TrackView.cpp:298-306`) captura um
  `EffectChain*` cru com `button` como contexto.
- **Vista que apaga modelos:** `~ManageVestigeInstrumentView` (`Vestige.cpp:1145-1174`) apaga
  `m_vi->knobFModel[i]` e lê `m_vi`. `closePlugin()` (`:527`) zera `p_subWindow` sem apagar a vista, que fica filha
  da janela do instrumento. Ao carregar preset com "Manage parameters" aberto e depois trocar o instrumento: double
  free ou use-after-free. `~ManageVSTEffectView` (`VstEffectControls.cpp:501-512`) apaga os modelos de parâmetro do
  efeito ao fechar a janela (e a automação some do projeto salvo); `loadSettings` (`:85-103`) recria por cima sem
  liberar. Mesmo padrão em `Vestige.cpp:1023-1048`. Os modelos guardam ponteiros crus para janelas (`m_scrollArea`,
  `m_subWindow`, `p_subWindow`).
- **Menus criados pela vista, guardados no modelo, apagados pela vista** (`InstrumentTrackView.cpp:119-120`,
  `245-246`).
- **Vazamentos por posse dividida entre pais:** `MixerChannelView` cria `m_effectRackView` com pai
  `mixerView->m_racksWidget` (`MixerChannelView.cpp:478`) e não tem destrutor; vaza um por canal apagado e por canal
  a cada projeto carregado. `createMixerMenu` faz `new QMenu(title)` sem pai (`InstrumentTrackView.cpp:512`,
  SampleTrackView, FolderTrackView) e `TrackOperationsWidget::updateMenu` (`:309`) o põe no menu; `QMenu::clear()`
  não apaga submenus: vaza um `QMenu` por abertura do menu da faixa.
- **`delete menu` depois de `exec()` com pai `this`** (se `this` morrer durante o `exec`, é double delete):
  `EffectView.cpp:271`, `MixerChannelView.cpp:942`, `LfoControllerDialog.cpp:242`, `TempoSyncKnob.cpp:176`,
  `TempoSyncBarModelEditor.cpp:158`. O `ControllerView` já faz certo, com `QPointer<CaptionMenu>`.
- **Sinais em fila com ponteiro cru como argumento** (o Qt descarta a chamada se o receptor morrer, não se o
  argumento morrer): `trackRemovalScheduled(TrackView*)` → `deleteTrackView` (`TrackOperationsWidget.cpp:126-129`,
  `212`), `trackAdded(Track*)` (`TrackContainerView.cpp:142-145`), `clipAdded(Clip*)` (`TrackView.cpp:104-107`),
  `EffectView::deletedPlugin`/`replaceRequested`/`loadPresetRequested`/`savePresetRequested`
  (`EffectRackView.cpp:312-315`), `ControllerView::removedController`. Com o `sendPostedEvents()` dentro de
  `Song::clearProject` (`Song.cpp:1807-1808`), "Novo projeto" logo depois de uma remoção em fila entrega
  `deleteTrackView(morta)`. `EffectRackView::deletePlugin` (`283-287`) faz `erase(find(...))` sem testar `end()`.
- Ponteiros crus para subjanelas apagados no destrutor: `EffectView::m_subWindow` (`EffectView.h:104`, `.cpp:139`),
  `ControllerView::m_subWindow` (`ControllerView.h:84`, `.cpp:94`); só não dão double delete pela ordem de criação.

**Mudança:** `ModelView<T>` tipada.

- Guarda `QPointer<T>` e um conjunto de conexões (`QMetaObject::Connection`) que cai sozinho; nada de
  `model->sub()->disconnect(this)`.
- `modelGone()` virtual, chamado no `destroyed` e nos `destroyedX`: desliga `periodicUpdate` e o que lê o modelo.
- **Nunca** é dona do modelo: some a flag `isDefaultConstructed`; o widget que precisa de um modelo próprio o guarda
  num `unique_ptr` explícito. Some o `isOldModelValid`.
- Modelos não apontam para widgets (no máximo `QPointer`). Os modelos de parâmetro do Vestige/VST ficam só no
  instrumento/controles, num `std::vector<std::unique_ptr<FloatModel>>` recriado num lugar só.
- O pai do Qt de um widget é o dono dele, ou ele vive num `unique_ptr`. `createMixerMenu(QWidget* parent)` exige o
  pai. Menus de contexto na pilha, ou `QPointer` + `delete`.
- Sinal em fila não leva `QObject*`: `QMetaObject::invokeMethod(receptor, [p = QPointer(x)] { if (p) ... },
  Qt::QueuedConnection)`, e o receptor confere se o item ainda está na lista.

## Lacuna 5: a thread de áudio fala por sinais do Qt

**O que falta na base:** `Model`/`AutomatableModel` emitem `dataChanged` em qualquer thread, e não há um canal
oficial do áudio para a interface.

Evidências:

- Seguindo o mapa de andamentos, o áudio chama `m_tempoModel.setValue(bpm, true)` (`Song.cpp:306-309`, `364`), que
  emite `dataChanged` (`AutomatableModel.cpp:323`) → `Song::setTempo` por `DirectConnection` (`Song.cpp:111`) →
  `tempoChanged` na thread de áudio → `SampleClip::tempoChanged` (`SampleClip.cpp:56`, `107`, mexe em offset e
  tamanho) e `TempoSyncKnobModel::calculateTempoSyncTime` (`TempoSyncKnobModel.cpp:48`), também diretos. Os dois são
  apagados na thread principal fora da trava (`Track.cpp:543-547`, `EffectChain.cpp:298-299`), e as conexões só caem
  no `~QObject`. Um período nessa janela chama um `SampleClip` ou um efeito meio destruído. `setTempo` também trava o
  mutex de cada nota e aloca eventos em fila.

**Mudança:** o áudio nunca emite sinais.

- Escreve em atômicos e marca "sujo"; uma caixa de correio áudio → principal, esvaziada por um temporizador da
  interface, emite. O fork já faz assim com a automação gravada; vira regra da base `AutomatableModel`.

## Lacuna 6: afinidade de thread não é imposta

Evidências:

- **Exportar com gravação ativa:** `ProjectRenderer::run()` (`ProjectRenderer.cpp:150-171`, um `QThread`) chama
  `Song::startExport()`/`stopExport()` (`Song.cpp:1597`, `1650`) → `Song::stop()` (`773-850`) →
  `finishAudioRecording()`/`landOnLanes()`/`finishMidiRecording()` na thread de exportação: cria `AudioItem`,
  `MidiClip`, notas, `QFile::remove`, `PeakCache::store()` enquanto a interface desenha os mesmos clipes. Os objetos
  novos ficam com a afinidade da thread errada. Nada impede: nem `exportProject()`, nem `ExportProjectDialog`, nem
  `AudioEngine::setAudioDevice()` recusam com gravação ativa (`reopenAudioDevice()`, `AudioEngine.cpp:574`, recusa).
- **Parar o dispositivo não é barreira:** o `stopProcessingImpl` do JACK é vazio; uma callback que já passou do
  `isRunning()` renderiza depois de `Engine::destroy` passo 1. `renderNextBufferImpl` escreve `m_capturePointers`
  fora da trava (`AudioEngine.h:428-453`), e `prepareCapture()` supõe o dispositivo parado (`AudioEngine.cpp:481`).
- **JACK depois de o servidor cair:** `restartAfterZombified()` (`AudioJack.cpp:119`) chama `initJackClient()`, que
  acrescenta em `m_outputPorts`/`m_inputPorts` sem limpar (`197`, `210`): o callback usa portas do cliente morto, as
  entradas dobram, o cliente velho não fecha. `shutdownCallback` (`492-497`) zera `m_client` sem atômico; aí
  `~MidiJack` (`MidiJack.cpp:118-161`) vê `jackClient()` nulo e não chama `removeMidiClient()`, e
  `AudioJack::m_midiClient` fica apontando para um `MidiJack` apagado.
- **`terminate()` em threads:** `MidiAlsaRaw.cpp:61-72`, `MidiSndio.cpp:66-74`, `MidiJack` (`wait(1000)` e depois
  `terminate()`); `MidiAlsaSeq.cpp:125-137` fecha o handle mesmo se o `wait` falhou;
  `AudioDevice::stopProcessingThread` (`AudioDevice.cpp:62-73`) faz `terminate()` depois de 30 s e pode matar uma
  thread com `m_changeMutex` na mão.
- **CoreAudio:** `AudioObjectRemovePropertyListener` não espera callbacks em andamento; `~AudioCoreAudio` libera
  `this` e `propertyListener` lê `device->m_device` (`AudioCoreAudio.cpp:463-470`, `588`).
- Corridas sem liberação: `AudioInputTrack::m_input` escrito sem trava e lido pelos workers em
  `InputMonitorHandle::play`; mutex na thread de áudio em `MidiAlsaSeq.cpp:163`; lambdas em fila de
  `InstrumentTrack::processInEvent` capturam a faixa crua (seguro por `MidiRecorder::forget`, menos se uma faixa nova
  nascer no mesmo endereço).

**Mudança:**

- Tipos presos a uma thread: `ThreadCheck::assertMainThread` no transporte da `Song`, no fim da gravação, na criação
  de clipes e no `PeakCache`. A exportação só renderiza: `startExport`/`stopExport` vão para a thread principal; com
  gravação ativa, exportar é recusado (ou a gravação para antes).
- A base `AudioDevice` conta as callbacks em andamento em volta de `renderNextBuffer`; `stopProcessing()` espera o
  contador zerar.
- A volta do JACK passa por `AudioEngine::reopenAudioDevice` (fecha o MIDI, refaz o dispositivo);
  `removeMidiClient()` não depende de `jackClient()`.
- Nenhum `terminate()`: espera limitada e join de verdade. No CoreAudio, listeners em fila própria, esvaziada antes
  de destruir.

## Lacuna 7: trabalho pesado segurando a trava

Evidências:

- `Song::stop` (`Song.cpp:844`) e `Song::stopRecording` (~`1402`) chamam `AudioRecorder::stop()` sob a trava: join
  da thread de disco, `sf_writef`/`sf_close` e até 10 s de buffer escritos com o áudio parado.
- `Song::clearProject` (`Song.cpp:1802-1830`) emite `aboutToClearProject`, roda `QCoreApplication::sendPostedEvents()`
  e apaga faixas e canais sob a trava: qualquer slot da interface roda com o áudio travado (risco de deadlock se um
  deles esperar o áudio) e todo sinal em fila vira reentrada (lacuna 4).
- `Mixer::deleteChannel` emite `channelDeleted` dentro da trava (`Mixer.cpp:837`).

**Mudança:** o guard da trava acumula ações adiadas (aposentar, emitir, join) e as executa depois de soltar: "colete
sob a trava, aja depois". `AudioRecorder::stop` se divide em "desliga a sessão sob a trava" e "esvazia e fecha sem
ela". O `sendPostedEvents()` sai do `clearProject` (ou roda antes do `aboutToClearProject`, fora da trava).

## Lacuna 8: globais por toda parte

**O que falta na base:** as dependências não entram pelo construtor; cada objeto pega o que precisa de uma global.

Números (fora de código de terceiros): `Engine::getSong()` 619, `Engine::audioEngine()` 458,
`ConfigManager::inst()` 281, `getGUI()` 196, `Engine::mixer()` 103, `Engine::patternStore()` 65,
`Engine::projectJournal()` 56. São 32 destrutores que tocam em globais: a ordem de destruição depende delas.

Estáticos de vida própria:

- instâncias de LFO criadas e apagadas preguiçosamente sem trava (`EnvelopeAndLfoParameters.cpp:162-166`,
  `191-195`), lidas pelo áudio sem testar nulo (`AudioEngine.cpp:368`); só não quebra porque a prévia sempre tem um
  envelope vivo;
- `static Controller* dummy` em `Controller::create` (`Controller.cpp:239-249`): nunca apagado, fica em
  `s_allControllers` entre vidas do motor, com `m_valueBuffer` do tamanho do período do primeiro motor (um segundo
  motor com período maior lê além do fim, `:149`);
- `DiskReader` estático (`AudioStream.cpp:344`): a thread só para no destrutor estático, depois do `QApplication`;
- `Editor::s_lastPlayedEditor` (`Editor.h:85`), `ClipView::s_textFloat`, `QPixmap` estáticos em funções
  (`SaControlsDialog.cpp:95-307`, `Vestige.cpp:930`, `Nes.cpp:734`, `Lb302.cpp:709`, `Organic.cpp:379`,
  `AudioFileProcessorView.cpp:242`), `Organic` `s_harmonics`;
- `~Mixer` → `deleteChannelSend` usa `Engine::mixer()` já zerado (`Mixer.cpp:559-563`, `960`);
- `Song::m_controllers`: controladores com a `Song` de pai do Qt e também apagados à mão (`Song.cpp:2466`), dois
  donos.

**Mudança:**

- Terminar o 8.2 do plano de refatoração: dependências pelo construtor (um `EngineContext&`).
- Os ponteiros do `Engine` em `unique_ptr`; todo estático que o motor usa vira membro do `Engine`/`AudioEngine`,
  criado no `init()` e destruído no `destroy()`.
- Regra de lint: destrutor não toca em global.

## Lacuna 9 (plugins): o contrato de tempo real não é verificado

**O que já existe:** o `makePluginData` para dados por nota de tamanho fixo; o pool de tempo real atrás do
`operator new` (`RealtimeNew.cpp`); o `AudioThreadAllocationTest`, que pega `malloc` do sistema tocando cada
instrumento e efeito interno com os parâmetros padrão.

**O que falta na base:** o teste só exercita o caminho padrão (parâmetros padrão, 16 notas, sem trocar nada
enquanto toca). Não há um molde para vozes de tamanho variável (o `FreshVoicePool` existe, mas só o Mallets usa), e o
pool não protege contra blocos maiores que ele nem contra o custo de CPU de construir a voz no áudio.

Casos que o teste não exercita, com o cenário para reproduzir:

- **GranularPitchShifter** `m_ringBuf.resize(...)` no `process()` ao mudar o range, até ~46 MB: passa do pool e cai
  no `malloc` do sistema (`GranularPitchShifterEffect.cpp:262-279`). Cenário: efeito num canal, música tocando,
  girar o botão Range para o máximo. `m_grains.push_back` passa do `reserve(8)` (`:202`, `:290`). Cenário: densidade
  de grãos alta com tamanho de grão grande.
- **Patman** fora da lista do teste. `new handle_data`, `new Sample::PlaybackState` e `make_shared<Sample>()` por nota
  (`Patman.cpp:~428-433`), liberados com `delete` (`:171-176`). Vão para o pool, então o risco é o pool, não o
  `malloc`. Cenário: Patman com um `.pat` carregado tocando acordes; e Patman sem patch (o `make_shared<Sample>()`
  vazio).
- **Xpressive** dois `new ExprFront` e `new ExprSynth` por nota (`Xpressive.cpp:193`, `195`, `210`, `226`): saem do
  pool (o teste confirma), mas o parse e a compilação do exprtk rodam no áudio a cada nota. É custo de CPU, não de
  memória. Cenário: acordes rápidos com uma expressão longa, medindo o tempo do período.
- **Vibed** `new StringContainer` e até 9 `VibratingString` por nota (`Vibed.cpp:202`, `253`;
  `VibratingString.cpp:47-80`): saem do pool. O tamanho dos buffers depende da altura da nota. Cenário: notas muito
  graves com as 9 cordas ligadas, conferindo o pico do pool (`RealtimeAllocator::stats()`).
- **Caminhos de reserva:**
  - FreeBoy `make_unique<GbApuWrapper>` quando o pool de vozes acaba (`FreeBoy.cpp:246`). Cenário: mais vozes ao
    mesmo tempo que o pool tem.
  - Mallets `makeVoice` lendo arquivos do STK (`Mallets.cpp:339`). Cenário: mais de 16 notas juntas, ou trocar o
    preset e tocar antes de o temporizador repor as vozes.
  - `SampleTrack.cpp:103` `new SamplePlayHandle` quando o pool acaba. Cenário: muitos clipes de amostra começando no
    mesmo período.
  - `BasicFilters::setFilterType` `new BasicFilters` (`BasicFilters.h:282`). Cenário: trocar o tipo de filtro do
    envelope para um que use subfiltro enquanto a nota toca.
  - `NotePlayHandleManager::acquire`/`release` com `QReadWriteLock` e `addSlots` (`NotePlayHandle.cpp:788-859`).
    Cenário: mais notas ao mesmo tempo do que as vagas iniciais.
- Menores: `Sf2Player.cpp:772`, `782` `push_back` sob `QMutex`; `printf` em `SidInstrument.cpp:413`; `qDebug` em
  `Lb302.cpp:545`.
- `SaControlsDialog.cpp:208`, `224`, `276`, `299`: `connect` com lambda sem contexto; cada vez que a janela é refeita,
  mais uma cópia, e cada mudança replaneja o FFTW N vezes sob as travas.

Classes copiáveis que são donas de ponteiro cru (uma cópia = double free): `BasicFilters` (`m_subFilter`,
`BasicFilters.h:302`, `927`), `LocklessList` (`m_allocator`, `LocklessList.h:47-53`), `RemotePluginBase`
(`m_in`/`m_out`, `RemotePluginBase.h:507-508`), `FreshVoicePool.h:51-77`, `LocklessAllocator.cpp:80-81`;
`new[]`/`delete[]` à mão em `AudioFileWave.cpp:111-119`, `MidiAlsaRaw.cpp:71`, `MidiAlsaSeq.cpp:577`.
`AutomationClip` copia `m_objects` antes de travar `m_clipMutex` (`AutomationClip.cpp:72`, `81`).

**Mudança:**

- Contrato do plugin: tudo que é alocado vem num `prepare()`/`activate()` na thread principal; `playNote` e
  `process` não alocam. Vozes de tamanho variável saem de um `FreshVoicePool` (Xpressive: compilar a expressão uma
  vez na thread principal e clonar). O GranularPitchShifter aloca o anel máximo uma vez.
- `destroyPluginData` e as quedas de `shared_ptr` do lado do plugin passam pelo `retire` (lacuna 2).
- Estender o `AudioThreadAllocationTest` com os cenários acima: mexer nos parâmetros enquanto toca (o teste já
  sorteia valores no `BuiltinPluginsTest`; falta juntar as duas coisas), trocar preset tocando, polifonia acima dos
  pools, incluir o Patman. Medir também o pico do pool, não só o `malloc`.
- Copiar classe dona de ponteiro: `= delete` ou membros `unique_ptr`/`vector`.

---

## Ordem sugerida

1. **Redes de proteção, para medir o resto:**
   - o `AudioThreadAllocationTest` (que já existe) estendido com parâmetros mexidos enquanto toca, troca de preset e
     polifonia acima dos pools (lacuna 9);
   - `EngineLifecycleTest` com LeakSanitizer no Linux;
   - `clang-tidy` (`cppcoreguidelines-owning-memory`, `cppcoreguidelines-special-member-functions`) também em
     `include/` e `plugins/`;
   - regras de grep no CI: `connect` com 3 argumentos e lambda (sem contexto); `T* m_` de modelo em classes de vista
     (`grep -nE "^\s*(Track|Clip|MidiClip|InstrumentTrack|Effect\w*|Controller|MixerChannel)\s*\*\s*m_"` nos
     cabeçalhos de vista, janela e editor); `terminate()`; `Qt::DirectConnection` em modelos.
2. **`retire()` e o ciclo de vida em fases** (lacunas 1 e 2): é o que mais elimina bugs de uma vez.
3. **`PlayHandleRef` e as referências com geração** (lacuna 3).
4. **`ModelView<T>`** (lacuna 4).
5. **Caixa de correio áudio → interface** (lacuna 5), asserções de thread e barreira do dispositivo (lacuna 6), ações
   adiadas da trava (lacuna 7).
6. **Injeção de contexto** no lugar das globais (lacuna 8): contínuo, junto de cada peça mexida.
7. **Contrato de tempo real dos plugins** (lacuna 9), depois de o teste estendido apontar os casos.

## Bugs concretos que já valem correção pontual

Por risco, cada um com o padrão de raiz entre parênteses:

- [ ] Piano Roll ao apagar a faixa com o clipe aberto (lacuna 1).
- [ ] TripleOscillator: troca da onda/preset sem trava enquanto toca (lacuna 2).
- [ ] Xpressive: troca da expressão sem trava enquanto toca (lacuna 2).
- [ ] Exportar com gravação ativa roda a `Song` na thread de exportação (lacuna 6).
- [ ] `FileBrowser::m_previewPlayHandle` depois de Stop (lacuna 3).
- [ ] LV2: instância liberada antes do join do worker (lacuna 2).
- [ ] Vestige/VST: vista de parâmetros apagando modelos do plugin (lacuna 4).
- [ ] `AutomatableModel.cpp:192` `delete` de clipe que ainda está na faixa; `~Clip` público (lacuna 1).
- [ ] `PresetPreviewPlayHandle` com nota devolvida ao pool em xrun crítico (lacuna 3).
- [ ] Tempo pelo mapa emitindo `tempoChanged` na thread de áudio (lacuna 5).
- [ ] JACK depois de o servidor cair: portas e cliente MIDI soltos (lacuna 6).
- [ ] `~InstrumentTrackView`/`periodicUpdate` lendo `model()` nulo (lacuna 4).
- [ ] Sinais em fila com ponteiro cru + `sendPostedEvents()` no `clearProject` (lacunas 4 e 7).
- [ ] Gravador fazendo join e escrita em disco sob a trava (lacuna 7).
- [ ] Vazamentos: `EffectRackView` por canal, `QMenu` por abertura do menu da faixa, handles do TapTempo,
      `clearNewPlayHandles`, handles que sobram no `~AudioEngine` (lacunas 3 e 4).
- [ ] GranularPitchShifter: girar o Range tocando aloca até ~46 MB no `malloc` do sistema (lacuna 9).
- [ ] Patman fora do `AudioThreadAllocationTest`; Xpressive compila a expressão no áudio a cada nota (lacuna 9).
- [ ] `terminate()` nos drivers MIDI e no `AudioDevice` (lacuna 6).
- [ ] Estáticos: LFO, `Controller` dummy, `DiskReader` (lacuna 8).
