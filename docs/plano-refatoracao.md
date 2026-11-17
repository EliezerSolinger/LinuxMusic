# Plano de refatoração da base (núcleo, threads, memória, entrada e plugins)

Escrito em 17/11/2026 a partir de uma revisão do núcleo (`src/core`, `src/tracks`, `include`). O objetivo é uma
base estável e segura: menos classes inteiras de quedas, áudio sem travas nem corridas de dados, e arquivos e
plugins de fora tratados como entrada não confiável.

## Regras para todas as etapas

- **Uma etapa por commit**, com mensagem que diz o que muda e por quê.
- **Antes de cada commit:**
  - `ninja` sem avisos novos;
  - `ctest` com todos os testes passando;
  - a bateria A/B igual à referência tirada antes da refatoração (`tools/check/ab.sh`).
  - Nas etapas que mexem em threads ou memória, rodar também as compilações com ASan/UBSan e TSan.
- **Refatorar não muda o som.** Qualquer diferença na A/B acima do ruído dos plugins aleatórios é erro.
- **Plugins internos ficam no caminho de áudio**, no mesmo processo e na mesma thread, pela menor latência
  possível. A segurança deles vem de testes, sanitizadores e *fuzzing*. Isolar em outro processo só vale para
  plugins de terceiros, e só quando escolhido.
- `src/gui/ActionGroup.cpp` nunca entra em commit. Nada de push.
- Depois de corrigir uma queda, procurar o mesmo padrão no resto do código e corrigir todos os casos.
- O que for decidido no caminho é anotado aqui, na seção da etapa.

## Levantamento (17/11/2026)

| Problema | Onde | Quanto |
|---|---|---|
| Núcleo que inclui cabeçalhos da interface | `src/core` | 17 arquivos (`Song`, `DataFile`, `TrackContainer`, `Clip`, `SampleClip`, `PatternClip`, `AutomationClip`, `AudioItem`, `Effect`, `Controller`, `StepRecorder`, `AudioPortsModel`, `ConfigManager`, `Plugin`, `SampleBuffer`, `AudioJack`, `MidiJack`, `AudioFileDevice`, `main`) |
| Janelas de aviso abertas pelo núcleo | `QMessageBox` em `src/core` | 11 arquivos |
| Checagens de "tem interface?" | `getGUI()` | 250 no total; 66 no núcleo, 32 só em `Song.cpp` |
| Trava global de alterações | `requestChangeInModel` | 50 chamadas; `std::recursive_timed_mutex` |
| Sinais disparados fora da thread da interface | `InstrumentTrack::newNote/endNote` (thread MIDI), modelos alterados pela automação e pelos controladores | a levantar na etapa 2.1 |
| Valor de parâmetro lido pelo áudio e escrito pela interface sem ser atômico | `AutomatableModel::m_value`, `m_valueChanged` | todos os parâmetros |
| `new` sem dono declarado / `delete` manual | `src`, `include` | 1363 / 501 |
| Singletons globais | `Engine::getSong()` 607, `Engine::audioEngine()` 450, `Engine::mixer()` 92 | |
| Descompactação sem limite | `DataFile::loadData` e `main.cpp` (`qUncompress`) | 2 |
| `assert` (some na versão final) | `src` | 257 |
| Comunicação com plugins em outro processo sem limites | `RemotePluginBase::readString` aceita qualquer tamanho; a leitura espera em laço de `usleep(5)` sem prazo | |
| Ferramentas de verificação fora do repositório | A/B, varredura ASan, gerador de projetos corrompidos | 3 scripts |

## Ordem das fases

| Fase | Assunto | Risco | Esforço |
|---|---|---|---|
| 0 | Ferramentas de verificação no repositório | baixo | baixo |
| 1 | Núcleo sem interface | médio | médio |
| 2 | Sinais e valores entre threads | médio | médio |
| 3 | Entrada não confiável (projetos, MIDI, amostras) | baixo | médio |
| 4 | Posse de memória | médio | médio, gradual |
| 5 | Plugins internos à prova de falhas | baixo | médio |
| 6 | Plugins de terceiros e IPC | médio | alto |
| 7 | Estado de render sem trava (retratos imutáveis) | alto | alto |
| 8 | Ciclo de vida explícito (singletons) | médio | contínuo |
| 9 | Revisão final e documentação | — | baixo |

A fase 7 vem depois das fases 2 e 4 porque depende de valores atômicos e de posse clara dos objetos.

---

## Fase 0: ferramentas de verificação no repositório

Hoje a bateria A/B, a varredura ASan e o gerador de projetos corrompidos ficam numa pasta temporária. Se ela
some, perdemos a forma de provar que nada mudou.

- **0.1** Criar `tools/check/`:
  - `ab-render.sh <build> <saída>`: renderiza os projetos de teste em WAV float de 32 bits;
  - `ab-compare.py <a> <b>`: diferença máxima e RMS de cada par, em dBFS, e o pior caso;
  - `ab-projects/`: os 44 projetos de teste, que cobrem cada instrumento e efeito interno e os LADSPA;
  - `sanitize-all.sh <build-asan> <saída>`: renderiza os projetos de teste e os de `data/projects`, apontando
    qualquer erro do ASan ou do UBSan;
  - `corrupt-projects.py <saída> <projetos…>`: variantes cortadas, com bytes trocados, com valores fora da
    faixa e sem elementos, sempre as mesmas (semente fixa);
  - `README.md` com o uso.
  - As referências de áudio ficam fora do repositório (são 75 MB). O script recebe o caminho delas.
- **0.2** `tests/scripted/check-core-gui`: falha se um arquivo do núcleo incluir um cabeçalho da interface. Na
  primeira versão, uma lista de exceções com os casos atuais; a fase 1 esvazia essa lista. Registrado no
  `ctest`.

**Pronto quando:** a referência A/B do commit de partida foi gerada com os scripts novos e a comparação dela
consigo mesma dá zero.

## Fase 1: núcleo sem interface

Fim do tipo de queda "rodou sem interface e alguém chamou `getGUI()->…`". O núcleo deixa de conhecer janelas.

- **1.1 Avisos ao usuário.** Criar `include/UserNotice.h` e `src/core/UserNotice.cpp`:
  - `notifyUser(severity, title, text)`;
  - a interface instala quem mostra o aviso (`QMessageBox`, `TextFloat`);
  - sem interface, o aviso vai para `stderr`;
  - chamado fora da thread principal, o aviso é entregue à thread principal por fila (nunca abre janela fora
    dela);
  - teste próprio: sem interface, de outra thread, e com quem mostra o aviso instalado.
- **1.2** Trocar os `QMessageBox` do núcleo por `notifyUser`: `DataFile`, `SampleBuffer`, `Plugin`,
  `ImportFilter`, `ConfigManager`, `Song`, `PeakController`, `MidiJack`, `AudioJack` e `AudioFileDevice`.
  Onde o núcleo *pergunta* algo ao usuário, a pergunta passa para quem chamou (a interface), e o núcleo recebe
  a resposta como parâmetro.
- **1.3 Fábricas de vistas.** Os `createView()`/`instantiateView()` dos modelos do núcleo ficam declarados no
  modelo, mas passam a ser definidos nos arquivos da vista:
  - `SampleClip`, `PatternClip`, `AutomationClip`, `MidiClip`, `AudioItem`, `Effect`, `Controller`,
    `AudioPortsModel` e as faixas;
  - assim os `.cpp` do núcleo deixam de incluir vistas.
- **1.4 `Song` e os editores.**
  - Limpar o projeto: `Song` emite um sinal `projectCleared`/`projectLoaded`, e cada editor se limpa ou se
    atualiza sozinho.
  - Estado da interface salvo no projeto (Piano Roll, rack de controladores, editor de automação, notas):
    um registro de "partes do projeto que a interface guarda" (`Song::addProjectStateExtension`), que a
    interface preenche e que salva e carrega na mesma ordem de hoje, para o arquivo continuar idêntico.
- **1.5 Restante do núcleo:**
  - `TrackContainer`: o progresso do carregamento vira uma função de retorno; a interface mostra o
    `QProgressDialog`;
  - `Clip`, quando abre o editor de automação: passa para a vista;
  - `StepRecorder`, que conversa com seu widget: por sinais;
  - `AudioJack` e `MidiJack`, avisos e menus: via `notifyUser`/sinais;
  - `AudioFileDevice`, que conversa com o `ExportProjectDialog`: por sinais;
  - `ConfigManager`: usa `QCoreApplication`, sem `MainWindow`;
  - `Clipboard`: usa `QGuiApplication`;
  - `InstrumentFunctions`, `InstrumentSoundShaping`, `DataFile`: o `embed.h` sai do núcleo onde for só
    imagem.
- **1.6 Controles de dispositivo.** As telas de configuração dos drivers (classes `setupWidget` dentro de
  `AudioPulseAudio.h`, `AudioCoreAudio.h` etc.) vão para `src/gui`, registradas por nome do driver.
- **1.7 `main.cpp`** fica em `src/` como o único ponto que conhece as duas camadas.
- **1.8 Garantia.** A lista de exceções do `check-core-gui` fica vazia. Se der, compilar os arquivos do
  núcleo num alvo próprio sem os cabeçalhos do `QtWidgets`, para o compilador recusar uma inclusão nova.

**Pronto quando:**
- nenhum arquivo do núcleo inclui interface;
- as `getGUI()` restantes ficam todas em `src/gui`;
- importar, renderizar e exportar pela linha de comando funcionam sob ASan;
- os projetos salvos antes e depois saem idênticos byte a byte (teste).

## Fase 2: sinais e valores entre threads

- **2.1 Levantamento.** Listar todo sinal emitido de código que roda nas threads de áudio, MIDI ou dos workers:
  - `InstrumentTrack::processInEvent`, que roda na thread MIDI;
  - modelos alterados por automação e controladores durante o render;
  - `Song`, nos sinais de posição;
  - `Mixer`, `AudioEngine`, `PeakController`, `LfoController`.
  - Cada conexão que cruza threads fica explícita: `Qt::QueuedConnection`, ou uma flag atômica que a
    interface lê no seu timer. O resultado fica registrado aqui.
- **2.2** `AutomatableModel::m_value` e `m_valueChanged` viram `std::atomic` com ordem `relaxed`: sem custo
  nas plataformas que temos, e sem corrida de dados formal. Revisar quem lê `m_value` direto.
- **2.3** `InstrumentTrack::newNote`/`endNote`: o estado do teclado na tela passa a ser lido pela interface
  (flags atômicas por tecla), e não mais avisado por sinal da thread MIDI.
- **2.4** Um teste com TSan que abre um projeto, toca, mexe em parâmetros e em automação de outra thread e
  fecha. A partir daí ele entra na bateria de verificação.

**Pronto quando:** o TSan sai limpo no teste novo e nos que já existem, sem suprimir nada além do falso
positivo de dentro do Qt que já está documentado.

## Fase 3: entrada não confiável

- **3.1 Descompactação com limite.** `qUncompress` lê o tamanho final nos 4 primeiros bytes. Recusar acima de
  um limite (512 MB para projetos), com mensagem clara, em `DataFile` e em `main.cpp`.
- **3.2 `DataFile`.**
  - Recusar arquivo vazio, raiz errada e versão do futuro com mensagem, sem `assert`.
  - Números fora da faixa limitados pelo próprio modelo, já que `AutomatableModel::setValue` limita. Revisar
    os atributos lidos com `toInt()`/`toFloat()` sem validação, que viram índices ou tamanhos (número de
    faixas, canais do mixer, tamanho de clipe, posição).
- **3.3 MIDI e SF2.** Eventos e tamanhos validados, sem `assert` sobre os dados do arquivo.
- **3.4 `assert` sobre dados de fora.** Revisar os 257: os que testam entrada viram erro tratado; os que
  testam invariantes internas ficam.
- **3.5 Teste de carregamento com arquivos corrompidos** (`LoadCorruptedTest`): gera variantes dos projetos de
  teste (as mesmas do `corrupt-projects.py`, com semente fixa) e carrega cada uma. Passa se nenhuma derrubar o
  programa; sob ASan, também se não houver erro de memória.
- **3.6 Caminhos de amostras e presets** vindos de um projeto: sem leitura fora do que o usuário pode abrir de
  qualquer forma (não é sandbox), mas sem estouro com caminhos enormes ou inválidos.

**Pronto quando:** o teste 3.5 e a varredura com o gerador de corrompidos passam sob ASan/UBSan.

## Fase 4: posse de memória

- **4.1 Regra escrita** em `docs/regras-de-codigo.md`:
  - quem é dono usa `std::unique_ptr` (ou o pai do Qt, nunca os dois);
  - quem só observa um `QObject` que pode morrer usa `QPointer`;
  - nada de `delete` manual em código novo.
- **4.2 `.clang-tidy`** com `cppcoreguidelines-owning-memory`, `bugprone-*`, `clang-analyzer-*` e
  `performance-*`, aplicado primeiro ao núcleo.
- **4.3 Núcleo:**
  - `AudioEngine`: `m_audioDev`, `m_midiClient` e os workers;
  - `Engine`: os ponteiros estáticos;
  - `Mixer`: os canais;
  - `Song`/`TrackContainer`: as faixas;
  - os clipes;
  - `Plugin`/`Effect`/`EffectChain`;
  - `ControllerConnection`.
- **4.4 Observadores** que guardam ponteiros para objetos de vida mais curta: vistas que apontam para modelos,
  o `MidiJack` que aponta para o `AudioJack`, conexões de controladores. Viram `QPointer` ou passam a ser
  avisados quando o objeto morre.

**Pronto quando:** o núcleo não tem `delete` manual fora das exceções anotadas (objetos do Qt com pai e o
alocador de tempo real), e o ASan está limpo nos testes, na A/B e ao abrir e fechar a interface.

## Fase 5: plugins internos à prova de falhas

Os plugins internos continuam no caminho de áudio. Eles ficam seguros por verificação:

- **5.1 `BuiltinPluginsTest`**, para cada instrumento e efeito interno:
  - instanciar, tocar notas e processar áudio;
  - mudar a taxa de amostragem;
  - salvar e carregar;
  - e com valores extremos nos parâmetros (mínimo, máximo e aleatórios com semente fixa).
  - Passa sem queda e sem `NaN`/`inf` na saída.
  - Sob ASan/UBSan, sem erro de memória nem comportamento indefinido.
- **5.2 Sem alocação nem trava no processamento.** Estender o `AudioThreadAllocationTest` a todos os plugins
  internos e corrigir os que alocam.
- **5.3** O que os testes encontrarem é corrigido, com varredura do mesmo padrão nos outros plugins.

## Fase 6: plugins de terceiros e IPC

- **6.1 `RemotePluginBase`:**
  - tamanhos de mensagem e de texto limitados e validados, recusando negativos e gigantes;
  - leitura com prazo, que confere se o outro processo ainda vive;
  - se o plugin some ou trava, o áudio dele vira silêncio e ele é marcado como suspeito no `PluginGuard`.
- **6.2** Testes com um cliente falso: mensagens malformadas, processo que morre no meio, processo que não
  responde.
- **6.3 Hospedeiro de plugins em outro processo** (`linuxmusic-plugin-host`), opcional por plugin, para LADSPA
  e LV2 de terceiros, reaproveitando o protocolo do `RemotePlugin`:
  - primeiro LADSPA (API simples);
  - depois LV2 sem interface própria, com portas de controle, áudio, átomos/MIDI e estado;
  - liga sozinho para um plugin que o `PluginGuard` marcou como suspeito, e o usuário pode ligar para qualquer
    plugin de terceiros.
  - Plugins internos nunca.

## Fase 7: estado de render sem trava

Hoje a interface e o render dividem uma trava recursiva (`m_changeMutex`). A meta é que o áudio nunca espere a
interface.

- **7.1 Levantamento** das 50 chamadas de `requestChangeInModel`: o que cada uma protege e o que o render lê.
- **7.2 Infraestrutura:**
  - um `RenderSnapshot<T>` publicado por ponteiro atômico, ou com o `TripleBuffer` que já existe;
  - uma fila de lixo esvaziada fora da thread de áudio, para liberar retratos antigos sem alocar nem liberar
    memória no áudio;
  - testes, inclusive com TSan.
- **7.3 Roteamento do mixer** compilado num retrato imutável (canais, envios, ordem de processamento). A
  interface altera o modelo e publica um retrato novo; o render pega o retrato no início do período.
- **7.4** Lista de faixas e de *play handles* do render.
- **7.5** Cadeias de efeitos.
- **7.6** O que sobrar de `requestChangeInModel` fica restrito a trocas estruturais raras (trocar o
  dispositivo, carregar um projeto). Cada caso que sobrar é anotado aqui com o motivo.

**Pronto quando:** tocar e editar ao mesmo tempo (adicionar e remover faixas, efeitos, envios) não faz o
render esperar (contador `periodsSkipped` zerado num teste de estresse), e o TSan sai limpo.

## Fase 8: ciclo de vida explícito

- **8.1** Documentar e testar a ordem de criação e destruição do `Engine`. `EngineLifecycleTest` faz
  `init`/`destroy` duas vezes seguidas, com projeto aberto, sob ASan.
- **8.2** As peças novas e as refeitas (gravação, mapa de tempo, exportação, metrônomo) recebem as dependências
  pelo construtor, em vez de chamar `Engine::…` lá dentro, o que permite testá-las sozinhas.
- **8.3** Encerramento: parar o áudio e o MIDI, depois o projeto, depois os gerenciadores. Nenhum objeto
  destruído pode ser usado por uma thread que ainda roda.

## Fase 9: revisão final

- Rodar tudo: testes, A/B, ASan/UBSan (testes, renders e corrompidos) e TSan.
- Conferir cada item deste plano e marcar `[x]` com o commit.
- Atualizar `docs/estado-atual.md` e `docs/pendencias.md`.

---

## Andamento

(Atualizado a cada etapa.)

### Fase 0: concluída
- [x] 0.1 `tools/check/` com A/B, varredura de sanitizadores e projetos corrompidos (4f5467075^).
  - Projetos aleatórios por natureza ficam em `ab-random.txt`.
  - O limite é -90 dBFS: builds idênticos diferem até -97 dBFS pela ordem de soma das threads.
- [x] 0.2 `tests/scripted/check-core-gui`, também no `ctest` e na CI (4f5467075).
- [x] Extra: `tools/check/gui-smoke.sh`, que abre a interface offscreen com cada projeto (bbbeed3fe).

### Fase 1: concluída
- [x] 1.1 `UserNotice`: avisos, perguntas e progresso, com teste (97acb4f6f).
- [x] 1.2 O núcleo não abre janelas (8a4fc1e6c).
  - Achados: `PeakController`, `ImportFilter`, o efeito LADSPA e o progresso do `MidiImport` abriam janelas sem
    interface.
  - O `AudioFileDevice` dava `exit()` lá de dentro; agora a renderização pela linha de comando sai com erro.
  - Os codificadores Ogg/MP3 que não iniciavam liberavam estado não inicializado (8343ce85c).
- [x] `PixmapLoader` separado do `embed.h` (c17ebfa7b).
- [x] 1.3 As vistas são criadas no código da interface (6393e6494).
- [x] 1.4 O `Song` sem os editores (e94580252).
  - Registro de estados do projeto e sinal `aboutToClearProject`.
  - Achado: sem interface, salvar perdia notas e estados dos editores, e a linha do tempo (loop) não carregava.
- [x] 1.5 `InstrumentTrack`/`MidiClip` não consultam o Piano Roll (853b0a819).
  - Achado: a thread de áudio lia `pianoRoll()->isRecording()`, uma corrida de dados.
  - `Clip`, `Clipboard`, `StepRecorder` e a descrição LV2 também saíram (64e3fb3ad).
  - Achado: colar com a área de transferência vazia desreferenciava nulo.
- [x] 1.6 As telas dos drivers foram para `src/gui` (396968a83).
- [x] 1.7 `main.cpp` em `src/` (99455c04a).
- [x] 1.8 A garantia ficou com o `check-core-gui`, que agora segue as inclusões transitivamente e roda no `ctest` e
  na CI. A lista de exceções está vazia.
  - Decisão: não separei o núcleo num alvo do CMake sem `QtWidgets`. Os plugins se ligam ao executável, e o
    `lmmsobjs` tem *export header*, definições e AUTOMOC próprios: o risco é alto e o ganho é pequeno, porque a
    checagem já pega qualquer inclusão nova.
- Para investigar (Fase 6): `ladspa-dj_eq` renderiza diferente entre execuções (-76 dB), já antes da
  refatoração.
- Ferramenta: a worktree com o build de antes da refatoração (`56a30bfff`) fica em
  `~/.claude/jobs/49dcb579/tmp/refactor/base-src/build`, para comparar comportamentos.
