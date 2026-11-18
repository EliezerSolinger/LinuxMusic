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
| 10 | Bibliotecas confirmadas: libebur128 e libremidi | médio | médio |

A fase 7 vem depois das fases 2 e 4 porque depende de valores atômicos e de posse clara dos objetos. A fase 10
foi pedida pelo usuário em 17/11, depois do levantamento em `docs/levantamento-bibliotecas.md`; ela roda antes da
revisão final (fase 9).

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
  e LV2 de terceiros, reaproveitando o protocolo do `RemotePlugin`. Decisão do usuário (17/11): o isolamento fica
  com o nosso `RemotePlugin`, e a ponte do Carla não é adotada:
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

### Decisão (17/11): medir primeiro, encurtar a trava; retratos só onde a medida pedir

A trava de mudança já não é o que o plano supunha: o render não espera por ela, desiste dela depois de meio
período e toca um período de silêncio (`periodsSkipped`). O problema real é quanto tempo a interface a segura.
Por isso, em vez de reescrever o render sobre retratos (7.2 a 7.5), a fase:

1. mede: `EditWhilePlayingTest` toca como uma placa de som (um período a cada período, numa thread) enquanto a
   thread principal edita como o usuário, e diz, por tipo de edição, quantos períodos foram pulados e quanto tempo
   a trava ficou presa no máximo (`AudioEngine::takeLongestChangeHold()`); roda também com TSan;
2. encurta cada espera longa com a mesma regra: **o que é pesado se faz fora da trava; dentro dela, só o momento
   de pôr ou tirar a peça do que o render percorre** (preparar, publicar; retirar, destruir);
3. corrige as corridas que aparecem no caminho: estruturas que o render percorre e que a interface mudava sem a
   trava;
4. só cria retratos imutáveis se, depois disso, uma edição ainda segurar a trava por tempo que se ouça.

Medidas (período de 256 quadros a 44,1 kHz, meio período = 2,9 ms):

| Edição | Antes | Depois |
|---|---|---|
| Criar uma faixa de instrumento | 3 a 4,6 ms | 0,01 ms |
| Apagar uma faixa de instrumento (como a interface faz) | 5,3 a 5,6 ms | 0,05 ms |
| Trocar o instrumento | 0,01 ms | |
| Pôr e tirar um efeito | 0,00 ms | |
| Criar e apagar um canal do mixer com envio | 0,06 ms | |
| Notas e mover um clipe | 0,00 ms | |
| Mudar o andamento | 0,00 ms | |

Passos feitos:

- **Criar uma faixa:** `Track::create` monta a faixa inteira (e, vinda de um projeto ou de um clone, restaura o
  estado dela, carregando instrumento e amostras) sem a trava, e só então a publica (`Track::publish`): a trava
  fica presa só para pôr a faixa na lista. O canal do mixer dela e as pastas vêm depois, cada passo do mixer com
  a trava pelo seu instante; o sinal `trackAdded` (que monta as vistas) sai com a trava livre. Uma faixa só entra
  num contêiner por `Track::create`.
- **Criar um canal do mixer:** o canal é montado (modelos, buffers, faixa de console) fora da trava e posto na
  lista sob ela. Antes, `createChannel` acrescentava à lista **sem trava nenhuma**, enquanto o render a percorria
  (`masterMix`): acrescentar pode mudar a lista inteira de lugar na memória. Era uma corrida de verdade.
- **Apagar uma faixa:** `Track::destroy` primeiro tira a faixa do contêiner (a trava só para isso: o render deixa
  de tocá-la, e ela sai inteira) e faz a automação de outras faixas soltar os controles dela, todos num instante
  só (`AutomationClip::forgetObjectsUnder`), guardando os ids para o desfazer religar. Depois apaga a faixa sem a
  trava: as peças dela que o render usa (notas tocando, barramentos, porta MIDI) se soltam sozinhas, cada uma no
  seu instante. Antes, a interface apagava a faixa inteira sob a trava (descarregar o instrumento incluído), e um
  `delete` sem trava deixava o render tocar uma faixa já meio destruída (o TSan mostrou). Agora é o único jeito de
  apagar uma faixa: os destrutores das faixas são protegidos, e o compilador recusa um `delete`.
- **Mesma regra, pelo caminho** (corridas achadas na varredura):
  - pôr, tirar e mover faixas na lista (`addTrack`, `removeTrack`, `moveTrack`) passa pela trava;
  - os clipes de uma faixa: `addClip`, `removeClip` e a troca de lugar passam pela trava; `Track::deleteClip`
    tira o clipe e só depois o apaga (antes, apagar um clipe deixava o render ver um clipe MIDI sem as notas);
  - o clipe que o piano roll toca sozinho: `Song::m_midiClipToPlay` ficava apontando para um clipe apagado;
    agora o clipe que sai da faixa é esquecido sob a trava e o tocar para;
  - os números dos padrões (`PatternTrack::s_infoMap`), que o render usa para achar o padrão: mudam sob a trava,
    e ao apagar um padrão os clipes dele saem e os números descem num instante só; o render lia o mapa com
    `operator[]`, que **insere** quando não acha, e agora usa `patternIndex()`, que só lê;
  - os controles de um clipe de automação: pôr (`addObject`) e tirar (`objectDestroyed`) passam pela trava,
    pega antes do mutex do clipe (o render segura a trava quando pega o mutex: a ordem contrária travaria os
    dois); a cópia de um clipe agora também é avisada quando um controle é apagado, então não sobra ponteiro
    nulo; `~AutomatableModel` avisa primeiro, ainda inteiro;
  - `AudioEngine::changesGuard()`: a trava do motor de áudio, ou nada quando ele ainda não existe ou já foi
    desmontado (nada toca então).
  - achados pelo TSan com o teste novo (apagar um clipe, um padrão, uma faixa automatizada, e o clipe que o piano
    roll toca, tudo tocando): `InstrumentTrack::play` olhava o instrumento antes de travar a faixa (a troca de
    instrumento o substitui com a faixa travada); `Engine::framesPerTick` e o canal do mixer de um barramento
    (`AudioBusHandle`) agora são atômicos; um LFO entra na lista que o render percorre só quando está inteiro e
    sai primeiro, e a marca de "forma do LFO desatualizada", escrita pelas duas threads, é atômica; o render, com
    o clipe do piano roll esquecido, não toca nada até parar (antes, lia o ponteiro nulo).
  - O TSan também acusava o mapa de pontos da automação, protegido pelo `QRecursiveMutex` do clipe (o TSan não o
    enxerga: o Qt não é compilado com TSan). Resolvido no passo 7.4, abaixo.

| Edição (com o teste novo) | Trava presa no máximo |
|---|---|
| Criar e apagar um clipe com notas | 0,00 ms |
| Criar e apagar um padrão | 0,01 ms |
| Apagar uma faixa cujos controles outra faixa automatiza | 0,08 ms |

- **Clipes entram na faixa inteiros:** o construtor de `Clip` punha o clipe na faixa antes de as partes dele
  existirem (um clipe MIDI sem notas, um de automação sem pontos), e o render podia tocá-lo assim. Agora cada
  clipe concreto entra por último no seu construtor (`Clip::enterTrack()`), e sai antes de ser desmontado
  (`Track::deleteClip()`).
- **Apagar um canal do mixer:** a renumeração dos canais e das faixas continua num instante só sob a trava (o
  render vê tudo antes ou depois); o canal sai da lista ali, e a automação solta os controles dele, mas ele é
  destruído depois, sem a trava, levando os efeitos e os plugins deles. `moveTrack` agora também passa pela
  trava.

| Edição | Trava presa no máximo |
|---|---|
| Criar e apagar um canal com três efeitos e o volume automatizado | 0,07 ms |

- **7.4 Automação sem mutex no render** (um item da fase 4 também: "QMutex restantes na thread de áudio"): quem
  muda os pontos ou os controles de um clipe de automação pega a trava do motor de áudio e depois o mutex do clipe
  (`AutomationClip::Change`); o render, que segura a trava, lê sem pegar mutex nenhum, então nunca espera a
  interface. A gravação de automação era a única escrita feita pela thread de áudio: agora o render põe os valores
  numa fila sem trava (`Song::m_recordedAutomation`), e a thread principal os põe no clipe (a cada 25 ms, e quando
  um clipe sai da faixa, que leva os seus junto). A marca "gravando" do clipe é atômica. O clipe de detuning de
  uma nota tocando continua gravado pelo render: só ele usa esse clipe enquanto a nota toca.
  Com isso o TSan sai **limpo** nos testes de edição tocando, estresse, automação, gravação, mixer e modelos.

| Edição | Trava presa no máximo |
|---|---|
| Desenhar pontos num clipe de automação que está tocando | 0,01 ms |
| Gravar automação tocando | (o render não espera) |

- **7.5 Carregar instrumentos e arquivos tocando.** O teste agora mede também quanto o render de cada período
  demora (o render pode esperar por uma trava do próprio plugin, e isso não aparece como período pulado):
  - **trocar o instrumento:** o novo é feito (e, vindo de um preset ou projeto, restaurado) sem trava nenhuma, e
    entra no lugar do velho num instante sob a trava da faixa e a do motor (`InstrumentTrack::swapInstrument`); as
    notas que chegaram pelo MIDI nesse meio tempo saem com o velho, que é apagado depois, sem as travas (antes, a
    troca era só sob a trava da faixa, e uma nota vinda do MIDI podia tocar pelo instrumento sendo trocado);
  - **Sf2 Player:** lia o soundfont inteiro segurando o mutex do sintetizador, que o render também pega: carregar um
    soundfont de 30 MB fazia **um período levar 24 ms** (o programa todo atrasava uns 4 períodos). Agora o arquivo
    é lido por um sintetizador à parte, sem trava, e passado para o do instrumento num instante (0,30 ms por
    período no máximo; a trava fica 0,55 ms, o FluidSynth liberando o soundfont velho). A troca de sintetizador
    quando muda a taxa de amostragem segue a mesma regra;
  - **GigPlayer:** o mesmo (lia o .gig inteiro sob o mutex que o render pega); agora lê sem trava e troca num
    instante, e o render só *tenta* o mutex (se a interface o segura naquele instante, só esse instrumento fica
    quieto naquele período);
  - **AudioFileProcessor, SlicerT, Patman:** trocavam a amostra (e as fatias, e os patches) **sem trava nenhuma**
    enquanto o render tocava; agora leem antes e põem no lugar num instante sob a trava; as vistas do SlicerT mudam
    uma cópia das fatias e a entregam ao instrumento (`setSlicePoints`). O SlicerT lia uma fatia além do fim;
  - **navegador de arquivos:** segurava a trava do motor durante o carregamento inteiro (um preset, uma amostra,
    um instrumento com o arquivo dele, uma importação): não segura mais, cada peça cuida de si; o Vestige também;
  - **um plugin reativado** (portas novas, ou ir para um processo próprio e voltar): `PluginInstance::reactivate`
    tira o plugin do processamento num instante sob a trava e faz o resto sem ela (só esse plugin deixa de ser
    processado; um efeito deixa o som passar); o estado `Active` é publicado por último, com release/acquire.
    **Mover um efeito para um processo próprio prendia a trava 21 a 25 ms** (o processo nascendo e carregando o
    plugin); agora 0,00 ms, com o som idêntico.

| Edição tocando | Trava presa | Período mais longo |
|---|---|---|
| Carregar um soundfont de 20 a 30 MB no Sf2 Player | 0,55 ms | 0,30 ms (antes 24 ms) |
| Carregar uma amostra no AudioFileProcessor ou no SlicerT | 0,00 ms | 0,21 ms |
| Trocar o instrumento por Sf2 Player | 0,01 ms | 0,22 ms |
| Mover um efeito LV2 ou LADSPA para um processo próprio | 0,00 ms (antes 21 a 25) | |

### 7.6 O que ainda pega a trava, e por quê (17/11)

Toda edição medida segura a trava por menos de 0,1 ms (a carga de um soundfont, 0,55 ms, é o FluidSynth liberando o
velho): bem abaixo do meio período (2,9 ms) que o render espera antes de desistir. O que sobra pega a trava só no
instante de pôr ou tirar algo do que o render percorre, com o trabalho pesado fora dela:

| Grupo | Funções | O que dura sob a trava |
|---|---|---|
| Faixas e clipes | `Track::publish`, `addClip`, `removeClip`, `swapPositionOfClips`, `TrackContainer::addTrack`, `removeTrack`, `moveTrack(s)`, `PatternTrack` (números), `PatternStore::takePattern`, `Clip::movePosition`, `ClipView::remove`, `moveToTrackUnder` | pôr/tirar um ponteiro numa lista, renumerar |
| Mixer | `createChannel`, `deleteChannel`, `createRoute`, `deleteChannelSend`, `moveChannelLeft`, `setChannelCount`, `add/removeLatencySource` | pôr/tirar um canal ou envio, renumerar |
| Efeitos | `EffectChain::append/insert/remove/replace/move…/clear/prependEffects` | pôr/tirar o efeito na lista (feito antes, destruído depois) |
| Plugins | `PluginInstance::reactivate` (só desativar), `convertToDummyEffect`, `InstrumentTrack::swapInstrument`, `updateTrackChannelCount`, `setSample`/`setFont`/`swapInstance`/`setSlicePoints`/`loadPatch` dos instrumentos, `SampleClip::setSample*`, `AudioItem::load` | trocar um ponteiro (o novo feito antes, o velho destruído depois) |
| Automação e controles | `AutomationClip::Change` (todo ponto ou controle mudado), `addObject`, `objectDestroyed`, `forgetObjectsIf`, `AutomatableModel::setControllerConnection`, `Controller` (criar/destruir), `ControllerConnection::attach` | mudar o mapa de pontos ou a lista de controles |
| Notas e play handles | `AudioEngine::removePlayHandle(s…)`, `removeAudioBusHandle`, `clearNewPlayHandles`, `InstrumentTrack::silenceAllNotes`, `processInEvent` (nota desligada vinda do MIDI), `updateBaseNote`, `PresetPreviewPlayHandle` | tirar notas da lista |
| Andamento e afinação | `Song::setTempo`, `TempoMap::*`, `Song::setScale`, `setKeymap`, `TempoRenderController::revert` | trocar o mapa de andamentos ou a escala |
| Mudanças estruturais raras (ficam assim) | `Song::clearProject`, `Song::loadProject`, `Song::stop`, `stopRecording`, `AudioEngine::openMidiClient`/`closeMidiClient`, a troca de dispositivo de áudio, `Lv2ControlBase::reload` (o botão "recarregar" do plugin) | o projeto inteiro, o dispositivo, um plugin recriado; o tocar está parado, ou o usuário pediu |

Tiradas da lista nesta fase (seguravam a trava pelo trabalho todo): criar e apagar faixas e canais, carregar
arquivos pelo navegador, `TrackView::dropEvent` (soltar uma faixa sobre outra) e `Track::loadTrack` (apagar os
clipes sob uma trava de fora), o Vestige abrindo um plugin, mover um plugin para um processo próprio.

**Fase 7 pronta** pelo critério do plano: tocar e editar ao mesmo tempo não pula período nenhum
(`EditWhilePlayingTest`, 16 tipos de edição), nenhum período demora por esperar a interface (o teste mede), e o
TSan sai limpo. Os retratos imutáveis (7.2 a 7.5 do plano original) não foram necessários: a medida não pediu.

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

## Fase 10: bibliotecas confirmadas (libebur128 e libremidi)

Só as duas que passaram na verificação de `docs/levantamento-bibliotecas.md`. Regra do usuário: **uma biblioteca só
entra se for melhor que a nossa, medido aqui, e tiver no mínimo todos os recursos que temos.**

Processo, igual para as duas:
1. A biblioteca entra como dependência opcional do build e é ligada atrás da mesma interface que já usamos
   (`ExportProcessor::measure`, `MidiClient`). O código atual continua.
2. Os mesmos testes rodam contra as duas implementações. Cada item das listas abaixo é um teste ou uma medição
   registrada aqui.
3. Ela só substitui a nossa se cumprir todos os itens de "mantém" e algum de "melhora", sem piorar latência,
   alocação na thread de áudio, determinismo nem A/B. Se não cumprir, fica a nossa, e o motivo vai anotado aqui com
   os números.
4. O código antigo só sai num commit separado, depois de tudo isso.

### 10.1 libebur128 (medição de loudness na exportação)
- **Mantém** o que o `ExportProcessor` faz hoje:
  - loudness integrado BS.1770 com as portas absoluta (-70 LUFS) e relativa (-10 LU);
  - pico de amostra;
  - qualquer taxa de amostragem e número de canais que exportamos;
  - silêncio (resultado sem loudness);
  - medição sobre o render inteiro, com cancelamento;
  - normalização para alvo de pico ou de loudness;
  - o mesmo resultado em toda execução.
- **Melhora** (pelo menos um, medido):
  - os sinais oficiais da EBU (Tech 3341 e 3342) dentro da tolerância da norma, onde a nossa falhar;
  - pico verdadeiro (*true peak*, com sobreamostragem), para normalizar sem passar de 0 dBTP;
  - LRA, loudness de curto prazo e momentâneo, para o assistente de exportação mostrar.
- **Medir:**
  - os sinais da EBU nas duas implementações;
  - a diferença em LUFS nos projetos A/B;
  - o tempo de medição de um render longo.

### 10.2 libremidi (entrada e saída MIDI)
- **Mantém** tudo o que os drivers atuais fazem:
  - clientes de sequenciador (ALSA seq, CoreMIDI): listam as portas legíveis e graváveis, inscrevem cada porta de
    faixa nos aparelhos escolhidos, avisam quando a lista de portas muda, e dão o nome da porta de origem de um
    evento (usado para aprender um controlador);
  - portas com o nome da faixa onde o driver atual as cria (ALSA seq), visíveis a outros programas;
  - clientes crus (ALSA raw, JACK, sndio, WinMM) e o modo de cada porta (entrada, saída ou as duas);
  - JACK: o MIDI no mesmo cliente do áudio, com o instante de cada evento dentro do período (precisão de amostra);
  - trocar o driver sem reiniciar (`AudioEngine::closeMidiClient`/`openMidiClient`), com as portas voltando
    inscritas nos mesmos aparelhos;
  - mensagens de tempo real e posição (`MidiSync`, `MidiStart`, `MidiSongPosition`);
  - nada alocado nem travado na thread de áudio ao enviar e receber (`AudioThreadAllocationTest`);
  - Linux, macOS, Windows MSVC e MinGW.
- **Melhora** (pelo menos um, medido):
  - conexão e desconexão de aparelhos a quente;
  - MIDI 2/UMP;
  - PipeWire;
  - USB identificado;
  - menos código nosso por plataforma (hoje são cerca de 2.150 linhas nos drivers).
- **Medir:**
  - latência de entrada, de evento a nota, com o teste de laço de volta;
  - jitter do instante dentro do período;
  - alocação e trava na thread de áudio;
  - os testes de MIDI que já existem (`MidiClientChangeTest`, `MidiTempoMapTest`), passando nas duas
    implementações.
- O ALSA e o JACK no Linux continuam dependendo do teste numa máquina Linux (pendência do usuário).

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

### Fase 2: sinais e valores entre threads
- [x] 2.1 Levantamento empírico: `tools/check/gui-smoke.sh` com `GUI_SMOKE_ACTIONS=play` no build TSan toca projetos
  com a interface aberta (a ação `play`, a095f2689). Corridas encontradas e corrigidas:
  - teclas do `Piano` (44cc1a831);
  - medidores do EQ, Delay e Compressor, e o visualizador do compressor (`AtomicPeak`, 7bfa93263);
  - controladores atualizados por várias threads, e a interface lendo o buffer (ee197b514, 24df4d38f);
  - posição e loop do `Timeline` (f528569cb; duas referências pendentes que isso criou foram corrigidas em
    23ecf4196);
  - `InstrumentTrack` emitia um sinal por nota nas threads de áudio e MIDI, agora conta (554043cee);
  - ZynAddSubFX: o gerador aleatório e os contadores globais (74f600343, bc6d6c7b8).
- [x] 2.2 `AutomatableModel`: o valor é atômico (6fc4f7c6f).
- [x] 2.3 O indicador de atividade da faixa lê contadores atômicos em vez de receber sinais (554043cee).
- [x] 2.4 `ThreadStressTest` (98c34d514): a interface e o áudio ao mesmo tempo. Ele achou as referências pendentes e
  o pool de notas fora do ciclo de vida do motor; agora o `Engine` é dono dele (43b11b9cc).
- Achados de determinismo no caminho (as renderizações variavam entre execuções):
  - o LB302 tocava a nota que chegasse por último, na ordem das threads (fd62e74ad);
  - o barramento somava as notas na ordem em que as threads as criavam (dcf9fe6b3).
  - Agora as renderizações são idênticas bit a bit entre execuções (`tools/check/determinism.sh`), fora as
    aleatórias e o ladspa-mbeq (planos do FFTW medidos na hora).
- Outros achados: a captura de tela travava com uma caixa modal de erro de carregamento, e os avisos agora vão
  sempre também para o log (2569f0c6b).
- Para fases seguintes (anotado aqui):
  - Fase 5: os `calc*`/`update*` dos efeitos e instrumentos internos rodam na thread que muda o modelo (interface ou
    automação) e escrevem coeficientes que o `process()` lê. É o mesmo padrão em quase todos; a correção é
    recalcular no `process()` quando o modelo mudou (`isValueChanged()`).
  - Fase 5: instrumentos de fluxo único (SF2, GIG, OpulenZ, SID, FreeBoy, LV2) recebem notas simultâneas na ordem das
    threads: a alocação de vozes pode variar. A saída é ordenar os eventos do período como no LB302.
  - Fase 6 (terceiros): no ZynAddSubFX, o tempo do LFO "contínuo" é um só para todas as instâncias. Cada `Master`
    precisa do seu, como o `AbsTime` do Zyn 3.
  - Fase 7: os sinais `valueChanged` de cada controlador por período e os `dataChanged` emitidos pela automação na
    thread de áudio alocam eventos do Qt (do pool de tempo real).


### Fase 3: entrada não confiável
- [x] 3.1 Projetos compactados descompactam com limite: zlib próprio, 512 MB, tamanho anunciado conferido
  (67716855d, `CompressedProjectTest`).
- [x] 3.2 / 3.5 `LoadCorruptedTest` (4e3c84085): variantes danificadas dos 43 projetos A/B, carregadas e tocadas,
  também sob ASan/UBSan. Achou:
  - estouro de heap ao carregar gráficos em base64 em oito plugins (40db94239);
  - tecla fora do teclado indexando a tabela de notas (5d5f1f561);
  - `bool` não inicializado no DynamicsProcessor (94a47bd83);
  - exceção do reamostrador na thread de áudio, que encerrava o programa (7155dbc0d).
- [x] 3.3 Fila do worker LV2: mensagens inteiras e as sem dados (27727f85c, `Lv2WorkerTest`). MIDI: um SF2 que não
  carregou não tem banco (578226e65).
- [x] 3.4 Os `assert` sobre dados de fora viraram checagens (MidiImport, LV2, AudioPortsModel). Os que restam testam
  invariantes internas.
- [x] 3.6 Caminhos enormes, inválidos, para fora ou pastas fazem parte das variantes do `LoadCorruptedTest`.
- [x] Pedido do usuário (17/11): onde um erro deixou de lançar exceção ou de derrubar o programa, ele vai para um
  log de erros. O `ErrorLog` (075f31904) é seguro na thread de áudio (sem trava nem alocação) e grava em
  `<pasta de trabalho>/logs/errors.log` e no stderr. Ele está ligado em todos esses pontos e recebe também os avisos
  ao usuário (fabdde10e).

### Fase 4: posse de memória: concluída
- [x] 4.1 Regras em `docs/regras-de-codigo.md`; 4.2 `.clang-tidy` do núcleo e das faixas (54e32ae7f).
- [x] `AudioEngine`: dispositivos de áudio e MIDI e workers em `unique_ptr` desde a criação; `Mixer`: canais e rotas
  (afdc6c798, 54e32ae7f).
- [x] Plugins: `Plugin::instantiate` e as de instrumento, efeito e ferramenta devolvem `unique_ptr`; `pluginAs<T>()`
  no lugar do `delete` manual; a janela principal é dona das ferramentas (fbcfd5930).
- [x] `ControllerConnection`: o modelo é dono da conexão (`unique_ptr`), e a conexão é dona do controlador MIDI.
  Quando um controlador da música some, a conexão avisa (`orphaned()`) e o modelo a apaga: acabou o `delete this`.
  Trocas sob a trava do motor. Consertou:
  - conexão num arquivo com controlador de outro tipo (LFO, pico, desconhecido): derrubava o programa;
  - carregar o estado de novo (desfazer) vazava a conexão anterior, que continuava ligada;
  - conexão movida para outro controlador era apagada quando o primeiro sumia;
  - o controlador "vazio" compartilhado podia ser apagado (e apagado de novo) ao limpar a música.
  Teste: `ControllerConnectionTest`.
- [ ] Faixas e clipes: já têm um dono, o pai do Qt (contêiner e faixa); os `delete` que restam apagam antes um
  filho do Qt, o que a regra aceita. Falta conferir se algum é chamado com o motor tocando sem a trava.
- [ ] `PeakController`: apaga a si mesmo quando o efeito morre fora da lista da música (prévia). Precisa de um dono
  único (o efeito).
- [x] Donos explícitos no resto do núcleo:
  - a faixa de automação global da música, o barramento da calibração de latência, a amostra e o barramento
    próprios do `SamplePlayHandle`, o arquivo lido pela prévia de preset;
  - as descrições do `LadspaManager` (por valor), o `QMimeData` da área de transferência;
  - os buffers do envelope/LFO, do `RingBuffer` e do `RmsHelper` (`std::vector`).
  - Também foram consertados:
    - o `RmsHelper` liberava com `delete` o que veio de `new[]`;
    - o atraso do Flanger (`MonoDelay`) tinha o mesmo erro e, com a taxa de amostragem mais baixa, escrevia fora do
      buffer;
    - as notas fantasma do piano roll vazavam (agora são guardadas por valor);
    - a exportação em segundo plano (`ThreadedExportManager`) só gravava a primeira exportação: a thread terminava
      mas continuava marcada como rodando. Agora é uma fila com condição de espera, que grava o que falta antes de
      parar (`ThreadedExportTest`).
  - Apagados `Delay.h` e `FifoBuffer.h`, que ninguém usava, e a decodificação base64 para vetor cru.
- [ ] Notas do `MidiClip`: o clipe é o único dono, mas guarda `Note*` num `NoteVector`, porque o mesmo vetor é lido
  pelos editores e pela thread de áudio sem alocar. Os `delete` ficam todos dentro do `MidiClip`. Trocar por
  `unique_ptr` muda 52 lugares; fica para quando o render usar instantâneos (fase 7).
- [ ] Estáticos com criação preguiçosa (`PresetPreviewPlayHandle::s_previewTC`, as instâncias de LFO do envelope):
  vão com os do `Engine` na fase 8.
- [x] 4.4 Observadores.
  - JACK: o MIDI e o áudio que dividem a conexão sabem quando o outro some. A callback lê o cliente MIDI uma vez
    só, e `removeMidiClient()` espera ela sair. Sem ligação salva, as portas vão para as físicas certas
    (eb4a00480, b5e845b9b). Testado com um `jackd` de teste neste Mac, também sob ASan.
  - Os outros já estavam protegidos:
    - as vistas observam os modelos por `QPointer` (`ModelView`);
    - as automações guardam `QPointer<AutomatableModel>`;
    - o piano roll e o gravador de passos largam o clipe apagado (`destroyedMidiClip`);
    - o `PeakController` larga o efeito pelo `destroyed`;
    - os ponteiros para o "pai" (clipe → faixa, instrumento → faixa, controles → efeito) apontam para quem vive
      mais.
- Exceções anotadas que ficam:
  - os filhos do Qt apagados antes do pai (faixas e clipes);
  - as notas do `MidiClip`;
  - os estáticos (fase 8);
  - o `PeakController` que apaga a si mesmo (vai para a fase 5, plugins internos).
- Os ponteiros estáticos do `Engine` ficam crus até a fase 8 (ciclo de vida).

### Fase 5: plugins internos à prova de falhas: concluída
- [x] 5.1 `BuiltinPluginsTest`: cada instrumento (20) e efeito (25) interno, achado pela fábrica de plugins.
  - Toca com os parâmetros no padrão, todos no mínimo, todos no máximo e em três sorteios.
  - Salva e carrega, e muda a taxa para 96 e 192 kHz e de volta.
  - A saída fica finita, e ASan/UBSan ficam limpos.
  - Os hospedeiros de terceiros e os players que precisam de arquivos próprios (GIG, PAT) ficam de fora.
  - Achou:
    - FrequencyShifter: o atraso passava do buffer circular (lia fora dele) e, na troca de taxa, o índice de escrita
      ficava além do buffer;
    - na varredura do mesmo padrão, o LOMM: o índice do buffer de antecipação não voltava a zero quando ele
      encolhia.
- [x] 5.2 `AudioThreadAllocationTest` cobre todos os efeitos internos e também o Sf2Player e o AVL Drums.
  - O Sf2Player alocava na thread de áudio: os dados de cada nota com `new`, e a lista de notas crescia. Agora
    usa o espaço da nota (`makePluginData`) e uma lista com espaço reservado.
- [ ] Ainda com trava na thread de áudio: as `QMutex` do Sf2Player (síntese e lista de notas) e do envelope/LFO.
  Vão na fase 7 (render sem trava).

### Fase 6: plugins de terceiros e IPC (em andamento)
- [x] 6.1 `RemotePluginBase`:
  - Tamanhos vindos do outro processo limitados nos dois caminhos: socket no Linux e no Mac, fila em memória
    compartilhada no Windows. Textos e partes de mensagem acima do máximo, ou negativos, quebram a conexão em vez de
    alocar ou esperar para sempre.
  - Na fila em memória compartilhada:
    - as posições escritas pelo outro processo são conferidas antes de cada uso;
    - os dados de uma mensagem anunciada têm prazo de 1 s;
    - o que quebra vai para o `ErrorLog`.
  - No socket, um sinal (`EINTR`) não quebra mais a conexão.
  - O marcador de "inválido" é atômico.
  - `waitForMessage(mensagem, prazo)`: no socket usa `poll`; na fila, o novo `SystemSemaphore::tryAcquireFor`
    (`sem_timedwait` no Linux, tentativas curtas no Mac, que não o tem, e espera com prazo no Windows).
  - Um plugin que não termina um período em 2 s (`RemotePlugin::ProcessTimeout`):
    - fica mudo;
    - vai para o log;
    - tem o processo encerrado pelo vigia.
    Antes, prendia a thread de áudio para sempre.
- [x] 6.2 `RemotePluginIpcTest`, com um "plugin" falso do outro lado de um par de sockets:
  - mensagem certa;
  - plugin calado (prazo);
  - tamanho de texto e número de partes impossíveis;
  - processo que morre no meio da mensagem;
  - conexão já quebrada;
  - prazo do semáforo.
  - Também foi conferido um processo remoto real (o ZynAddSubFX tocando na interface).
  - O caminho da fila (Windows) passou por uma checagem de sintaxe forçada aqui.
- [x] O plugin que travou vira suspeito no `PluginGuard` e o usuário é avisado (quem usa o `RemotePlugin` informa a
  identidade: Vestige, VstEffect, ZynAddSubFX).
  - `RemotePluginHangTest`, de ponta a ponta: um processo de plugin de teste (`tests/src/helpers/FakeRemotePlugin`,
    com o mesmo protocolo) responde e depois trava. O teste confere:
    - a thread de áudio desiste em 2 s;
    - as chamadas seguintes saem na hora;
    - o processo é encerrado;
    - o plugin vira suspeito.
  - Sob TSan, ele mostrou uma corrida no vigia (`m_quit` era `volatile bool`). A varredura trocou por atômicos os
    `volatile bool` entre threads: os `m_quit` dos drivers MIDI, o `m_released` da nota e o `m_bufferUsage` do
    barramento.
  - O `ThreadStressTest` sob TSan mostrou outra: a nota de um acorde ou arpejo recalculava a frequência das notas
    filhas enquanto elas tocavam em outra thread. Agora ela só as marca, e cada uma recalcula a sua.
- [x] 6.3 LADSPA isolado, opcional por efeito (o padrão continua no programa):
  - `RemoteLadspaHost`, com o mesmo protocolo do `RemotePlugin`:
    - carrega a biblioteca do plugin e cria as instâncias;
    - o áudio vai pelos buffers compartilhados que já existem (`usesSharedMemoryBuffers`);
    - os controles vão por um bloco de memória compartilhada, com o mesmo layout dos dois lados
      (`include/LadspaPorts.h`, sem Qt). Nenhuma mensagem nova por período, nada alocado na thread de áudio;
    - os denormais ficam desligados, como nas threads de áudio.
  - Ligar e desligar:
    - pelo menu do efeito ("Rodar em um processo separado");
    - salvo no projeto;
    - automático para um plugin suspeito, que antes era recusado e agora carrega isolado, com aviso.
  - Se o processo do plugin cai:
    - o programa continua e o efeito fica mudo, com o sinal seco passando;
    - o erro vai para o log;
    - o plugin vira suspeito e o usuário é avisado.
    Antes, travado e morto se confundiam.
  - A identidade no `PluginGuard` ignora o atributo "isolated" (é o mesmo plugin).
  - `LadspaIsolationTest`:
    - isolado, o som é idêntico (diferença zero) em três plugins TAP: mono e estéreo, com filtros, eco e
      reverb;
    - vai e volta durante o uso;
    - fica salvo e carregado;
    - um plugin de teste que cai (`tests/src/helpers/CrashingLadspa.c`) só derruba o próprio processo, e
      depois carrega isolado sozinho.
  - O teste achou e foi consertado:
    - um efeito com plugin que não carregou (um LADSPA não instalado) derrubava o programa ao salvar ou
      carregar, porque não tem controles; a interface tinha o mesmo problema;
    - o `SharedMemoryData` não era exportado para os plugins.
  - Observado, ainda não investigado: a primeira renderização com um plugin recém-carregado sai um período
    atrasada; as seguintes não.
- [x] 6.3 LV2 isolado, também opcional por efeito, como o LADSPA:
  - `RemoteLv2Host`: carrega só o bundle do plugin com lilv e oferece os mesmos *features* do programa (URID,
    opções, worker, os de tamanho de bloco). O worker é síncrono quando o programa só renderiza e tem thread quando
    há interface, como no programa.
  - O efeito manda um plano das portas: canais de áudio, lugares no bloco de controles para controles e CV,
    portas de eventos vazias. O remoto não tem regras próprias. Os modelos continuam no programa (`Lv2Proc` sem
    instância local).
  - Menu, projeto e isolamento automático de suspeitos, como no LADSPA.
  - `Lv2IsolationTest`, com um plugin LV2 de teste (`tests/src/helpers/TestLv2`: ganho pelo worker, CV, latência,
    porta de eventos, queda sob comando): som idêntico, controles chegando, ida e volta, salvo e carregado, queda
    isolada e suspeito carregando isolado.
  - Achados no caminho, consertados na causa:
    - **Portas CV:** um plugin com porta CV era recusado; e, se não fosse, a porta ficava ligada a nulo. Agora
      ela tem buffer e modelo.
    - **Efeito sem plugin numa cadeia:** um efeito cujo plugin não carregou (um LV2 não instalado) derrubava o
      programa. A própria cadeia passa a recusá-lo, venha de onde vier.
    - **A saída um período atrasada:** `renderNextPeriod` devolvia o período anterior (os buffers trocavam no
      começo do período). Consequências:
      - a placa de som recebia tudo um período mais tarde que o necessário (latência a mais à toa);
      - o primeiro período de cada reprodução era o último da anterior;
      - a exportação perdia o último período.
      Agora os buffers trocam no fim, e o gravador e a calibração contam um período a menos (só a entrada chega
      atrasada). A A/B confirmou o mesmo som, com um período a mais no fim. A referência foi refeita
      (a antiga está em `base1-before-period-fix`).
    - **Estado de antes num projeto novo ou numa exportação:** o drive, o transformador, o compressor de
      barramento e as linhas de atraso guardavam o que tocou antes. Um projeto novo e uma exportação agora começam
      do repouso (`Mixer::discardProcessingState()`, feito por cada canal na thread de áudio).
