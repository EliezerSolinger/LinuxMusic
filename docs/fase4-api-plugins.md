# Fase 4: nova API de plugins (#8275)

A proposta do LMMS ([#8275](https://github.com/LMMS/lmms/issues/8275), de Dalton Messmer) é um rascunho: define
objetivos e nomes, mas não assinaturas, e manda seguir o CLAP nos detalhes. Este documento fixa o desenho do
LinuxMusic, que segue o CLAP. Cada parte termina compilando, com testes e com commit próprio.

## Partes

| Parte | Conteúdo |
|---|---|
| A | **Threads e estados verificados.** Thread principal (a do Qt) e thread de áudio (simbólica: a que processa o plugin no período). Estados do CLAP (inativo, ativo, processando, dormindo, erro) e transições válidas; cada método do plugin com a thread e o estado em que pode ser chamado, verificados pelo host. |
| B | **Fila de eventos.** Entrada (host → plugin: notas, expressões de nota, MIDI) e saída (plugin → host), entregues no `ProcessContext` com o quadro de cada evento, no lugar do `handleMidiEvent` e dos mutex de eventos dos plugins. Produtores de outras threads (entrada MIDI, teclado da tela) escrevem numa fila sem trava. |
| C | **Extensões.** Plugin e host perguntam um ao outro por extensões pelo nome (como no CLAP). Do host: transporte (andamento, compasso, posição), registro de mensagens, verificação de thread. Do plugin: latência (base da compensação de latência, PDC), estado (salvar/carregar sem Qt). |
| D | **`PluginId`, `PluginDescriptor`, `PluginEntry`.** Identificador único com conversão de/para texto; descritor único para plugins e subplugins; ponto de entrada `lmms_plugin_entry` por biblioteca, com vários plugins por biblioteca (LV2 efeito e instrumento numa biblioteca só) e descoberta de subplugins fora da thread principal. Os plugins atuais continuam funcionando por um adaptador até serem migrados. |
| E | **Cabeçalhos públicos.** `include/lmms/plugin/` com a API que um plugin de terceiros usa, `include/lmms/README.md` e o alvo CMake `lmms::plugin`. Os plugins nativos continuam no repositório. |
| F | **Instrumentos por nota (#8294).** Decidir e implementar como os instrumentos baseados em `NotePlayHandle` passam a ter um único fluxo, sem várias threads de áudio ao mesmo tempo no mesmo plugin. |

## Parte A: threads e estados

- **Thread principal:** a thread do Qt (`QCoreApplication::instance()->thread()`).
- **Thread de áudio:** qualquer thread do motor de áudio enquanto renderiza (a do período e as de trabalho). Do
  ponto de vista de um plugin, é uma só por vez. Com o motor pausado (`requestChangeInModel`), o host pode agir como
  a thread de áudio do plugin, porque nenhuma outra está processando.
- **Estados e transições** (como no CLAP):
  - inativo → ativo: `activate()` [principal]
  - ativo → processando: `startProcessing()` [áudio], chamado pelo host no primeiro período depois de ativar
  - processando → ativo: `stopProcessing()` [áudio]
  - ativo → inativo: `deactivate()` [principal], só sem estar processando
  - `process()` [áudio], só processando; `reset()` [áudio], só ativo
  - portas de áudio (`channelCount()`, grupos, pares in-place) [principal], lidas pelo host com o plugin inativo
- **Verificação:** violações vão para um tratador (`PluginChecks`). Em builds de depuração ela aborta. Com
  `LMMS_PLUGIN_CHECKS=1` vale em qualquer build. Os testes instalam um tratador que registra as violações.

## Parte B: eventos (feita)

- `PluginEvent` (notas com IDs, expressões de nota, MIDI cru) e `LockFreeQueue` (MPMC limitada, sem trava).
- `PluginInstance::sendEvent()` aceita eventos de qualquer thread. Cada período, eles chegam em
  `ProcessContext::inEvents()`:
  - ordenados por quadro;
  - no mesmo quadro, em ordem fixa (notas que terminam, MIDI, expressões, notas novas por tecla), para o resultado
    não depender da thread que enviou cada nota;
  - eventos depois do fim do período ficam para o seguinte.
- Instrumentos que usam a fila: AVL Drums, ZynAddSubFX, Carla, LV2 e Vestige. Carla e AVL Drums perderam os mutex de
  eventos. O LV2 deixou de somar a posição da música ao carimbo de tempo dos eventos MIDI (bug herdado).
- **Pendente:** num render do ZynAddSubFX, o período do ataque da primeira nota sai em silêncio, e na build de
  referência ele soa. O resto é igual. Acontece também pelo caminho MIDI antigo (`handleMidiEvent`), então veio das
  fases 1 a 3, não da fila. A referência também recebe um pitch bend que a build nova não recebe. Investigar antes de
  fechar a fase 4.
