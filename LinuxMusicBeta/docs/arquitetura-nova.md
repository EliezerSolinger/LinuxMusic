# Arquitetura nova: proposta radical

Estado: **direção aprovada pelo usuário em 19/11.** Substitui o desenho incremental de
`legado/docs/design-ciclo-de-vida.md`. As evidências estão em `LinuxMusicBeta/docs/plano-ciclo-de-vida.md`.

Decisões do usuário (19/11):
- Os plugins são reescritos no contrato novo; não há o recuo para o modelo do Tracktion.
- Os plugins **quebram por um tempo** e ficam incompatíveis: só alguns principais continuam funcionando, para
  validar a base. Os outros são reescritos depois, sobre a base pronta.
- **O estado de cada plugin é um objeto chave-valor (uma árvore como JSON)**, e carregar, descarregar e salvar é
  reinserir esses valores (peça 7, "Estado do plugin").
- **Não só o plugin: cada pedaço do programa vira dado** que se exporta como objeto chave-valor. O projeto inteiro é
  uma árvore de valores (peça 2, "O programa como dado"). Os objetos de C++ só leem a árvore ou executam a partir
  dela.
- **O núcleo não depende do Qt, depende do dado** (peça 0). O Qt fica só na interface.
- **O projeto é uma pasta com um `projeto.json`** (peça 2).
- **Por enquanto, quebramos tudo, inclusive os projetos:** o programa novo não abre `.mmp`/`.mmpz`, não tem
  importador e não guarda estado de plugins antigos.
- **É um programa novo:** nasce em `LinuxMusicBeta/`, e o código velho vai para `legado/`, no mesmo repositório e no
  `master`, depois que cada sessão terminar o que tem pela metade ("Um programa novo, não uma migração").
- **A especificação da base** (tempo preciso, faixa única, roteamento com PDC, gravação com takes, diário, ações
  como dado, contrato no estilo CLAP, taxa fixa, render determinístico, testes, e os itens para depois) foi aprovada
  inteira (seção "Especificação da base").

Princípio do usuário (19/11): a arquitetura correta, mesmo que todo plugin seja reescrito; 10 funcionalidades
estáveis valem mais que 200 instáveis.

---

## O problema de raiz, numa frase

**O áudio e a interface mexem nos mesmos objetos.**

- `Track::play` percorre os clipes da faixa (objetos do Qt), a automação escreve valores direto nos modelos, e cada
  plugin é ao mesmo tempo um `QObject` com sinais, o modelo dos parâmetros e o processador de áudio.
- Para isso funcionar existe uma trava global. O áudio a segura durante o período inteiro; a interface a pega para
  editar. Quando a interface demora, o áudio desiste e **toca silêncio naquele período**
  (`AudioEngine.cpp:405-409`): editar ou carregar um projeto pode causar falhas no som.
- Todas as lacunas do plano saem disso:
  - um objeto precisa sair das listas do áudio antes de morrer (lacunas 1 e 2);
  - o áudio guarda ponteiros para coisas da interface (lacuna 3);
  - o áudio emite sinais do Qt (lacuna 5);
  - não há fronteira clara de thread (lacuna 6);
  - tudo de pesado acontece segurando a trava (lacuna 7).

Remendar cada lacuna mantém a causa. A proposta **remove a causa**: o áudio deixa de enxergar os objetos do modelo.

---

## A arquitetura em uma figura

```
  THREAD PRINCIPAL                                   THREADS DE ÁUDIO (período + workers)
  ─────────────────                                  ───────────────────────────────────
  Interface (vistas)                                 
     │ lê por id, assina mudanças                    
     ▼                                               
  Documento (o projeto)  ── transação ──► Compilador ──► Programa de render (imutável)
   dono único de tudo                     (thread       │   troca atômica no início
   objetos referidos por id               principal ou  │   do período; o antigo
   desfazer = transações                  de trabalho)  ▼   aposentado depois
     │                                               Render: nós de processamento,
     │  parâmetros (atômicos) ─────────────────────► lê; nunca espera, nunca aloca,
     │                                               não conhece o Qt
     ▲                                                   │
     └──────── telemetria (fila sem trava) ◄─────────────┘
               medidores, cursor, valores da automação,
               áudio gravado, xruns, fim de vozes
```

Só há **quatro pontos de cruzamento** entre as threads, e cada um tem um tipo próprio. Nada mais cruza.

1. **Programa de render:** publicado inteiro, por troca atômica.
2. **Parâmetros:** atômicos num armazém plano.
3. **Eventos para o áudio** (nota da entrada MIDI, prévia, transporte): fila sem trava.
4. **Telemetria para a interface:** fila sem trava.

---

## As peças

### 0. O núcleo não depende do Qt, depende do dado (decisão do usuário, 19/11)

**A regra:** tudo abaixo da interface é C++ puro, **sem nenhum cabeçalho do Qt**. O Qt é só uma das interfaces
possíveis, que lê o dado e pede transações.

| Camada | Pode usar o Qt? | O que é |
|---|---|---|
| `state` (valor, esquema, JSON) | não | a árvore, a serialização, a validação |
| `document` (raiz, histórico, ids, transações, assinaturas) | não | o modelo |
| `render` (compilador, programa, processadores, vozes, parâmetros, telemetria) | não | a execução |
| `io` (dispositivos de áudio e MIDI, arquivos, recursos) | não | a borda com o sistema |
| `session` (dono de tudo acima, `pump()`) | não | a montagem |
| plugins: esquema + processador | não | o contrato do plugin |
| `gui` (vistas, editores dos plugins, janelas) | **sim** | uma interface sobre o dado |

**O que substitui o que vinha do Qt no núcleo:**
- Sinais e slots → assinaturas do `Document` (por id ou tipo de nó, RAII) e a telemetria (filas sem trava). Ninguém
  no núcleo emite sinal.
- `QObject`, pai do Qt, `QPointer` → não há objeto do modelo vivo (peça 2); ids no lugar de ponteiros.
- `QString` → `std::string` em UTF-8. `QDomDocument`/XML → o formato da árvore é JSON, com um leitor e escritor
  próprios ou uma biblioteca sem Qt. Não há XML em lugar nenhum: o formato antigo não é lido (decisão de 19/11).
- `QThread`, `QMutex`, `QTimer` → `std::thread`, `std::jthread`, atômicos, e o `session.pump()`, que a interface
  chama no próprio temporizador (sem interface, os testes e a linha de comando chamam direto).
- `QFile`, `QDir`, `QSettings` → `std::filesystem` e a árvore de preferências.

**A interface é guiada pelo dado:**
- Uma vista recebe um id, lê o nó pelo `Ref`, desenha, assina o id e redesenha quando chega a diferença.
- Uma ação do usuário vira uma transação.
- A vista não guarda cópia do estado, nem ponteiro para objeto do núcleo além da `Session`.
- O widget morre: a `Subscription` dele morre junto, e nada mais acontece.

**Garantias de build:**
- os alvos do núcleo **não ligam** com o Qt no CMake: um `#include <Q...>` ali não compila;
- o `tests/scripted/check-core-gui` vira "nada fora de `gui` inclui o Qt";
- um executável de teste sem interface (`lmms-headless`) abre um projeto, toca, grava e exporta usando só o núcleo.
  Se ele funciona sem o Qt, a regra está de pé.

### 1. `Session`: o fim das globais

- Uma `Session` é dona, nesta ordem de membros (e, portanto, de destruição): dispositivos, render, compilador,
  documento, desfazer.
- Ela é passada a quem precisa. Não existe `Engine::getSong()`, `getGUI()` nem `ConfigManager::inst()` no núcleo.
- Testes criam várias sessões no mesmo processo.
- A ordem de criação e destruição é a ordem dos membros, escrita num lugar só e verificada pelo compilador.
- Elimina a lacuna 8 e os 32 destrutores que dependem de globais.

### 2. O programa como dado (decisão do usuário, 19/11)

**A regra:** todo o estado do programa que importa é uma **árvore de valores imutável** (`state::Value`, a mesma da
peça 7a). Projeto, faixas, clipes, notas, automação, mixer, roteamento, transporte, mapa de andamentos, plugins,
preferências: tudo é dado. **Nenhum objeto de C++ guarda estado próprio.** Cada objeto é de um destes três tipos:

1. **Leitor:** um acesso tipado a um nó da árvore (`TrackRef`, `ClipRef`), com getters que leem a árvore. Para
   mudar algo, só por transação.
2. **Executor:** peça de execução construída **a partir** da árvore (o programa de render, os processadores, as
   vozes, os dispositivos). Pode ser destruída e reconstruída a qualquer momento sem perder nada.
3. **Vista:** widget que mostra um nó e pede transações.

**O teste de verdade da arquitetura:** se algo do programa não pode ser reconstruído a partir da árvore, é bug.
Destruir todos os executores e todas as vistas e reconstruí-los da árvore tem que dar o mesmo programa, com o mesmo
som.

**A forma da árvore** (exemplo, resumido; as regras de cada parte estão em "Especificação da base"):

```json
{
  "schema": 1,
  "project": { "sampleRate": 48000, "ppq": 960, "seed": 271828 },
  "transport": { "loop": { "on": false, "from": 0, "to": 15360 },
                 "punch": { "on": false, "from": 0, "to": 0 }, "preRoll": { "bars": 1 } },
  "tempoMap": [ { "tick": 0, "bpm": 120.0, "timeSig": [4, 4] } ],
  "tracks": [
    { "id": "t:1", "name": "Master", "role": "master", "channels": 2,
      "volume": 1.0, "pan": 0.0, "mute": false, "solo": false,
      "plugins": [ { "id": "p:4", "plugin": "eq", "version": 1, "state": { "params": {} } } ],
      "sends": [], "output": { "device": "out", "channels": [1, 2] } },
    { "id": "t:17", "name": "Baixo", "channels": 2, "parent": null,
      "volume": 0.8, "pan": 0.0, "mute": false, "solo": false,
      "plugins": [ { "id": "p:5", "plugin": "tripleoscillator", "version": 1, "state": { "params": { "vol1": 0.8 } } } ],
      "sends": [ { "to": "t:1", "amount": 1.0, "pre": false } ],
      "items": [
        { "id": "c:52", "kind": "midi", "start": 0, "length": 3840,
          "notes": [ { "key": 57, "start": 0, "length": 480, "velocity": 100 } ] }
      ],
      "automation": [ { "id": "a:9", "target": { "node": "p:5", "param": "vol1" },
                        "points": [ { "tick": 0, "value": 0.8 }, { "tick": 1920, "value": 0.2, "curve": 0.0 } ] } ] },
    { "id": "t:18", "name": "Voz", "channels": 1, "parent": null,
      "input": { "device": "in", "channels": [1] }, "armed": false, "monitor": "auto",
      "plugins": [], "sends": [ { "to": "t:1", "amount": 1.0, "pre": false } ],
      "items": [
        { "id": "c:60", "kind": "audio", "start": 0, "length": 7680,
          "lanes": [
            { "take": 1, "source": { "kind": "audio", "key": "<hash>" }, "sourceOffset": 0 },
            { "take": 2, "source": { "kind": "audio", "key": "<hash>" }, "sourceOffset": 0 } ],
          "comp": [ { "from": 0, "to": 3840, "take": 2 }, { "from": 3840, "to": 7680, "take": 1 } ],
          "fadeIn": 48, "fadeOut": 48, "gain": 1.0 } ] }
  ]
}
```

- **Cada nó de objeto tem um `"id"`** estável. Ids substituem todos os ponteiros entre partes: o canal da faixa, o
  alvo da automação, o envio. A árvore não tem ciclos e não tem ponteiros.
- **Cada tipo de nó tem um esquema** com versão e `upgrade`: a faixa, o clipe, o canal, o projeto, como no plugin. O
  projeto inteiro migra de versão por funções puras e testáveis, não por código espalhado em `loadSettings`.
- **O arquivo do projeto é a árvore serializada.** Salvar é escrever a raiz, e abrir é ler, validar pelos esquemas e
  rodar os `upgrade`. Os `upgrade` servem às versões futuras **do formato novo**: um projeto salvo hoje continua
  abrindo depois que a árvore mudar.

**O projeto no disco é uma pasta** (decisão do usuário, 19/11):

```
Minha Música/
  projeto.json   ← a árvore do projeto: só ela entra no desfazer e é o que se salva
  interface.json ← a árvore da interface (janelas, zoom, rolagem): separada, para não sujar o diff da música
  audio/         ← gravações e amostras importadas, cada uma pelo hash do conteúdo
  backup/        ← autosaves: cópias do projeto.json com data
```

- **O áudio não entra no JSON.** O JSON aponta para ele: `{ "kind": "audio", "key": "<hash>", "name": "Voz take 3" }`.
  Uma amostra importada é copiada para `audio/`, para o projeto não quebrar quando o arquivo original mudar de lugar.
  Soundfonts e kits grandes do sistema podem ficar fora da pasta, por caminho mais hash, e o projeto avisa se o hash
  não bater.
- **"Exportar como arquivo único"** junta a pasta num `.zip` para compartilhar. Abrir um `.zip` o extrai numa pasta.
- **O JSON é estável:**
  - chaves sempre na mesma ordem;
  - números de ponto flutuante escritos de forma que voltem exatamente iguais (`std::to_chars` com a representação
    mais curta);
  - ids como texto (`"t:17"`), porque inteiros acima de 2^53 perdem precisão em JSON;
  - indentado, para que duas versões se comparem com `diff` e o projeto possa ser versionado no git.
- **Salvar é atômico:** escreve em `projeto.json.tmp` e renomeia. Um corte de energia no meio não deixa um projeto
  pela metade.
- **O desfazer não é salvo.** O arquivo guarda só a raiz atual.
- **Os `.mmp`/`.mmpz` antigos não abrem** (decisão de 19/11: "por enquanto quebramos tudo"). Quem precisar deles usa
  a versão atual do programa. Um conversor, se vier, será uma ferramenta à parte, fora do programa e do núcleo.

**O que NÃO é dado da árvore** (e por quê):
- **Valores ao vivo**: medidores, cursor de reprodução, valor automatizado no momento. Vêm do render pela telemetria
  e são derivados, não estado.
- **Áudio e amostras**: vão para o `ResourceStore` e entram na árvore como referência por hash. Uma gravação vira um
  recurso, e o clipe que aponta para ele entra na árvore por transação, quando o take termina.
- **Caches**: picos de onda, o programa compilado, buffers carregados. Todos se refazem a partir da árvore e dos
  recursos.
- **Estado da interface** (janelas abertas, zoom, rolagem, seleção): é **outra** árvore de valores, separada do
  projeto, com o mesmo mecanismo. Pode ser salva junto do projeto, mas não entra no desfazer da música.
- **Preferências**: uma terceira árvore, salva no arquivo de configuração. Substitui o `ConfigManager` global.

**Transações sobre a árvore:**
- `doc.transact("Mover clipe", [&](Edit& e) { e.set(clipId, "start", 384); })` monta uma raiz nova que divide com a
  velha tudo que não mudou (dados persistentes: só o caminho até o nó alterado é copiado).
- **O desfazer é a lista de raízes anteriores.** Não existe comando inverso escrito à mão, nem `JournallingObject`,
  nem mapa de ids do histórico.
- Ao confirmar, a transação calcula a **diferença** (nós criados, alterados e removidos, por id) e:
  - avisa as vistas que assinam aqueles ids, uma vez;
  - avisa o compilador, que recompila só o que mudou: um nó de render é reaproveitado quando o nó da árvore é o
    mesmo objeto, o que se confere por identidade e não por comparação profunda.
- **Gestos contínuos** (arrastar um botão, mover uma nota com o mouse) abrem **uma** transação que vai se
  substituindo enquanto o gesto dura. O desfazer recebe um passo só. Durante o arrasto, o parâmetro vai ao áudio pela
  vaga atômica, sem esperar a transação.

**A biblioteca:** dados persistentes em C++ já existem prontos. A `immer` (vetores e mapas persistentes com
compartilhamento estrutural, licença Boost, está no vcpkg) cobre listas grandes, como clipes com milhares de notas,
sem copiar a lista inteira a cada nota. A alternativa é escrever o `Value` com `std::shared_ptr<const ...>` por nó,
mais simples mas com cópia linear em listas grandes. **A decidir no esqueleto, medindo** um clipe com 10 mil notas
editado nota a nota.

**Por que isso resolve a raiz:**
- **Ciclo de vida do modelo:** não existe, porque o modelo não tem objetos vivos. Um valor não morre na mão de
  ninguém; um id que não resolve mais é só um id que não resolve.
- **Desfazer, salvar, abrir, copiar, colar, duplicar faixa, presets, recuperação após queda e autosave** viram a mesma
  operação sobre a mesma árvore. Hoje cada um é um caminho de código separado, e cada um tem os seus bugs.
- **Recuperação após queda:** o autosave é só escrever a raiz atual numa thread de trabalho. Como a raiz é imutável,
  dá para escrever sem travar nada.
- **Testes:** qualquer estado do programa é um arquivo JSON que um teste carrega.

### 2a. `Document`: o dono da árvore e os ids

- O `Document` guarda **a raiz atual** (um `state::Value` imutável), **o histórico** (as raízes anteriores, para
  desfazer e refazer) e **um índice** de id → caminho na árvore, atualizado pela diferença de cada transação.
- **Não há objetos do modelo vivos.** Não há `Track`, `Clip` nem `Model` com estado; não há pai do Qt; não há
  `unique_ptr` de modelo. Quem quer ler uma faixa pega um `TrackRef{doc, id}`, um acesso tipado que lê a raiz atual
  pelo índice. Um `TrackRef` cujo id sumiu responde "não existe", nunca lê memória liberada.
- **Fora da árvore, ninguém guarda ponteiro para o modelo: guarda o id.** A vista, o editor, a automação ("esta curva
  controla o parâmetro X do nó t:17") e o roteamento ("esta faixa manda para ch:3") são todos ids, inclusive dentro
  da própria árvore.
- **Toda mudança passa por uma transação** (`doc.transact("Mover clipe", [&](Edit& e) { ... })`), que:
  - monta a raiz nova e empilha a velha no histórico: isso **é** o desfazer. Saem o `JournallingObject`, o
    `ProjectJournal` e o mapa de ids do histórico;
  - confere os esquemas dos nós alterados, e uma transação que deixaria a árvore inválida é recusada inteira;
  - ao terminar, entrega a diferença (ids criados, alterados, removidos) **uma vez**, a quem assinou;
  - marca o programa de render como desatualizado.
- **Remover é um passo só:** o nó sai da raiz nova. Ninguém o alcança, porque o áudio não lê a árvore e os outros
  têm só id. O nó continua nas raízes antigas do histórico, e é isso que faz o desfazer trazê-lo de volta igual. A
  destruição em duas fases (lacuna 1) deixa de ser necessária.
- **Assinaturas são do núcleo, sem Qt** (peça 0): `doc.subscribe(id, callback)` ou por tipo de nó. As entregas
  acontecem na thread principal, em `session.pump()`. Uma assinatura é um objeto RAII (`Subscription`): destruí-lo
  cancela, e uma vista morta nunca é chamada.
- Elimina as lacunas 1, 3 (do lado da interface) e 4, e o padrão de "vista que observa objeto que morre".

### 3. O programa de render, imutável

- O compilador lê o documento e produz um `RenderProgram`: listas planas de nós (fonte de clipes de áudio, eventos de
  notas, instrumento, efeito, canal, envio, master, metrônomo, monitor de entrada), a ordem de execução e o
  paralelismo entre workers, as curvas de automação já resolvidas e as listas de eventos de cada clipe.
- **É imutável depois de publicado.** O áudio o recebe por troca atômica no início do período (o `Handoff` que já
  existe). O antigo vai para a aposentadoria e é destruído na thread principal quando o período que o usava terminou.
- **Estado de DSP que precisa continuar entre programas** (vozes tocando, cauda de reverb, linhas de atraso,
  instâncias de plugin) não fica no programa. Fica em **processadores**, objetos do lado do render que os programas
  referenciam.
  - O compilador reaproveita o processador de um nó que continua existindo.
  - Um processador que nenhum programa vivo referencia é aposentado depois do período.
  - A contagem de referências é feita só pela thread principal; o áudio nunca a mexe.
- **Edição pequena não recompila.** Mudar um parâmetro escreve um atômico. Mover uma nota troca só a lista de
  eventos daquele clipe, que é imutável, por cópia, e a velha é aposentada.
- **Sem trava de mudanças.** O áudio nunca espera a interface e nunca toca silêncio por causa dela. Carregar um
  projeto monta um documento e um programa novos e troca tudo de uma vez.
- Elimina as lacunas 2 (o áudio não alcança nada que a interface apaga), 3 (do lado do áudio) e 7 (não há trava
  para segurar).

### 4. Parâmetros: um armazém de atômicos

- Cada parâmetro automatizável (volume, pan, botões de plugin) é uma vaga num `ParameterStore`: um vetor de
  `std::atomic<float>` alocado quando o parâmetro é criado, com um id estável.
- A interface escreve na vaga, e o render lê e suaviza.
- A automação é avaliada **no render**, a partir das curvas compiladas. O valor automatizado **não é escrito de
  volta no modelo**: vai pela telemetria, para a interface mostrar o botão andando.
- Elimina a lacuna 5 pela raiz: o tempo seguindo o mapa de andamentos é só um valor que o render calcula, e não
  existe `setTempo` rodando no áudio nem sinal emitido de lá.

### 5. Telemetria: o único caminho do áudio para a interface

- Uma fila sem trava (um anel por tipo: medidores, cursor, valores automatizados, blocos gravados, avisos de xrun,
  vozes terminadas), esvaziada por um temporizador da interface.
- O render **não conhece o Qt**: não inclui cabeçalho do Qt, não emite sinal, não chama `invokeMethod`. Uma regra
  de build garante isso, como o `check-core-gui` já faz entre o núcleo e a interface.
- A gravação usa o mesmo caminho: o áudio escreve blocos na fila e uma thread de disco grava.

### 6. Eventos para o áudio: o fim dos play handles cruzando threads

- Notas da entrada MIDI, prévias do navegador de arquivos e do preset, o metrônomo e o transporte viram **eventos**
  numa fila sem trava para o render, com um id de quem pediu.
- **As vozes são do processador do instrumento**, saem de um pool alocado no `prepare()` e nunca cruzam threads.
- O navegador de arquivos guarda um `PreviewId`, não um ponteiro. "Parar a prévia" manda um evento com esse id;
  se a voz já acabou, o evento não faz nada.
- Elimina a lacuna 3 (a posse dos play handles), o `PresetPreviewPlayHandle`, a afinidade de thread do
  `SamplePlayHandle` e os vazamentos do TapTempo e do `clearNewPlayHandles`.

### 7. Plugins: contrato novo, todos reescritos

Cada plugin vira duas partes que não se conhecem por ponteiro:

- **`PluginModel`** (thread principal, no documento): declara os parâmetros (que viram vagas no `ParameterStore`),
  guarda o estado salvável e os recursos imutáveis (amostra, onda desenhada, expressão compilada), e tem a vista.
- **`Processor`** (render, sem Qt), com um contrato verificado:
  - `prepare(const ProcessSpec&)` na thread principal: taxa, tamanho máximo de bloco e número máximo de vozes;
    **aloca tudo aqui**;
  - `process(ProcessContext&) noexcept` no áudio: **não aloca, não trava, não lança**;
  - `reset()` e `release()`.
- **Estado do plugin: uma árvore de valores, e nada mais** (pedido do usuário, 19/11). Detalhado na seção
  "Estado do plugin", abaixo.
- **Recursos grandes** (amostra, tabela de onda, expressão compilada) são imutáveis e compartilhados. Trocar a
  amostra cria um recurso novo na thread principal e o publica no próximo programa (ou por `RtShared`). O antigo é
  aposentado. Isso conserta, por construção, o TripleOscillator, o Xpressive, o Patman e o AFP.
- **Vozes:** o plugin declara o tipo de voz e o host as aloca no `prepare()`. `noteOn`/`noteOff` são eventos. Nada
  de `new` por nota (Vibed, Xpressive, Patman).
- **Política de vozes, comum a todos os instrumentos:**
  - o plugin declara `maxVoices` no `ProcessSpec` (com um padrão do host, ajustável pelo usuário por instrumento);
  - o host reserva exatamente isso no `prepare()`, num *slot map*;
  - **acabadas as vozes, rouba uma**, na ordem: a mais antiga já em release, depois a mais baixa (pela envoltória),
    depois a mais antiga. A voz roubada faz um fade curto (alguns milissegundos, reservado junto com ela), sem clique;
  - **nunca aloca** para ganhar polifonia. Hoje o Mallets e o FreeBoy alocam quando faltam vozes (o teste estendido
    de 19/11 mediu 9344 e 325 alocações com acordes).
  - O monofônico/legato (`MonoMode`) é o caso `maxVoices = 1` com transferência de estado, não um caminho à parte.
- **Carregar um preset tocando:** monta um processador novo fora do áudio e troca no próximo programa, levando o
  estado de vozes quando der.
- **A vista do plugin** se liga a ids de parâmetro e ao `PluginModel`, nunca ao `Processor`. A janela "Manage
  parameters" do Vestige não tem como apagar modelos, porque não é dona de nada.
- **Plugins de fora** (LV2, VST, Carla, ZynAddSubFx com threads próprias) passam pelo mesmo contrato por um
  adaptador. O que não couber no contrato roda isolado num processo à parte ou fica desligado.
- Elimina a lacuna 9 e as lacunas 2 e 4 do lado dos plugins.

### 7a. Estado do plugin: uma árvore de valores

**A regra:** tudo que define um plugin (o que se salva, o que se desfaz, o que se copia, o que vai num preset) é uma
**árvore de valores** que pertence ao documento. O plugin não guarda estado escondido em lugar nenhum. Carregar,
descarregar, salvar, desfazer, copiar e trocar a taxa de amostragem viram a mesma operação: construir o processador
a partir da árvore.

**Por que isso elimina uma classe inteira de bugs:** hoje cada plugin escreve à mão `saveSettings`/`loadSettings` em
XML, guarda estado em membros soltos (o modelo, a vista, o processador) e precisa manter tudo coerente ao carregar
tocando. As auditorias acharam o resultado:
- o TripleOscillator e o Xpressive trocam membros sem trava no `loadSettings`;
- o Vestige e o VST recriam modelos por cima dos velhos;
- a janela "Manage parameters" apaga os modelos do plugin.

Com a árvore como única verdade, não há o que ficar incoerente.

**O valor** (`include/state/Value.h`, sem Qt e sem render, testável sozinho):

```cpp
namespace lmms::state
{

//! A plugin's state, as JSON: what is saved, undone, copied and put in a preset. Immutable once made (a change
//! makes a new tree that shares what did not change)
class Value
{
public:
	// null, bool, int64, double, string, a resource, a list, an object (keys in order)
	using Object = std::vector<std::pair<std::string, Value>>; // (ordered: saved files and diffs are stable)
	using List = std::vector<Value>;
	struct Resource { std::string kind; std::string key; }; // e.g. {"sample", "<content hash>"}: see below
	// ...accessors, comparison (==, exact), hashing
};

} // namespace lmms::state
```

- Imutável e compartilhado por partes: mudar uma chave cria uma árvore nova que reaproveita os galhos iguais. O
  desfazer guarda as árvores velhas sem custo de cópia, e a comparação de igualdade fica rápida.
- Números de ponto flutuante são guardados exatamente (`double`). O teste de ida e volta compara bit a bit.

**O esquema:** cada plugin declara a forma da sua árvore, e o host valida tudo por ele.

```cpp
struct ParamSpec    // a value the user and automation move: becomes a ParameterStore slot
{
	std::string key;            // "cutoff" (stable: the saved files use it)
	float min, max, def;        // and the scale (linear, log), step, unit, text formatting
	bool automatable = true;
};

struct StateSchema
{
	int version;                       // raised whenever the shape changes
	std::vector<ParamSpec> params;     // the knobs
	state::Value defaults;             // the whole default tree (params included)
	//! Takes a tree saved by an older version to this one (each version's step, chained)
	std::function<state::Value(state::Value, int savedVersion)> upgrade;
};
```

- Os parâmetros ficam na árvore (em `"params": { "cutoff": 0.4, ... }`). O host cria uma vaga do `ParameterStore`
  para cada um. Durante a reprodução, a vaga atômica é a fonte de leitura do áudio; a árvore é atualizada pela
  transação quando o usuário mexe, e com o valor final quando a automação termina de gravar.
- O resto do estado (forma de onda desenhada, expressão, mapa de fatias, modo) fica em chaves próprias, com tipo
  declarado no esquema.

**Os recursos grandes** (amostras, soundfonts, impulsos, kits) não entram na árvore como dados. Entram como uma
referência (`Resource{"sample", hash}`) a um `ResourceStore` do documento, que:
- guarda o caminho original e o hash do conteúdo;
- carrega uma vez, numa thread de trabalho, num buffer imutável compartilhado;
- dá ao processador um ponteiro para o buffer pronto (o processador nunca abre arquivo).

Trocar a amostra é trocar a referência na árvore. Dois plugins com a mesma amostra dividem o mesmo buffer.

**O contrato do plugin:**

```cpp
struct PluginDescriptor
{
	std::string id;                       // "tripleoscillator"
	auto schema() const -> const StateSchema&;
	//! [main or worker thread] A processor made from @p state alone: all it will ever need is allocated here
	std::function<std::unique_ptr<Processor>(const state::Value& state, const ProcessSpec& spec,
		const Resources& resources)> createProcessor;
};
```

O descritor do plugin é do núcleo e não conhece o Qt. As outras duas partes de um plugin ficam fora dele:

- **O editor** (a janela do plugin) é registrado na camada `gui`, pelo mesmo id: `gui::registerEditor("tripleoscillator",
  factory)`. A janela recebe a `Session` e o id do nó do plugin. Ela lê a árvore, edita por transação e escreve nas
  vagas de parâmetro. Um plugin sem editor registrado ganha o editor genérico, montado a partir do esquema: um botão
  por `ParamSpec`. Como esse editor sempre existe, um plugin já é útil antes de ter janela própria.

- **O processador nasce da árvore e nunca devolve estado.** Não existe `Processor::save()`. Ele recebe mudanças de
  duas formas:
  - **parâmetro**: lê a vaga atômica a cada bloco;
  - **qualquer outra chave**: o host constrói um processador novo a partir da árvore nova, numa thread de trabalho, e
    troca no próximo programa. Para não cortar o som, o processador pode implementar `adopt(Processor& old)` e levar
    as vozes e caudas do antigo, no áudio, sem alocar.
- **A vista do plugin não toca no processador.** Ela edita a árvore por transação e os parâmetros pela vaga.
- **Plugins de terceiros** (LV2, VST, CLAP) usam o mesmo formato: `"params"` com os parâmetros que eles expõem e
  `"chunk"` com o estado opaco deles, como um blob em base64. O adaptador lê o chunk só ao construir o processador e
  na hora de salvar pede o chunk novo, que vira uma transação.

**O que cada operação vira:**

| Operação | Hoje | Com a árvore |
|---|---|---|
| Salvar o projeto | `saveSettings` de cada plugin, à mão | a árvore vira JSON dentro do arquivo |
| Abrir o projeto | `loadSettings` à mão, com o plugin já vivo | validar pelo esquema → `upgrade` → `createProcessor` |
| Preset | outro caminho de XML | uma árvore num arquivo |
| Desfazer uma mudança no plugin | journalling por modelo | a árvore anterior |
| Copiar a faixa | clonar objetos do Qt | copiar a árvore |
| Trocar a taxa de amostragem | cada plugin trata `sampleRateChanged` | `createProcessor` de novo, com o `ProcessSpec` novo |
| Descarregar e recarregar | não existe de forma segura | descartar o processador; a árvore fica |
| Plugin caiu (fora do processo) | perde o estado | recria a partir da árvore |

**Testes gerados para todo plugin a partir do esquema** (nenhum plugin volta a funcionar sem passar):
1. padrão → processador → salvar → abrir → a árvore é igual, bit a bit;
2. árvores sorteadas dentro das faixas do esquema → ida e volta igual;
3. **sem estado escondido**: o mesmo estado, construído duas vezes e renderizado com as mesmas notas, dá o mesmo
   áudio bit a bit;
4. trocar uma chave que não é parâmetro, tocando: o processador novo entra sem alocação no áudio e sem estalo
   (o `adopt`);
5. cada versão antiga do esquema, por arquivos guardados nos testes: o `upgrade` chega à versão atual;
6. trocar a taxa de amostragem: reconstrói e soa.

**Plugins ainda não reescritos** simplesmente não existem no programa novo: não aparecem na lista e não há
marcador. Os projetos antigos não abrem (peça 2), então não há estado antigo a preservar. Se um projeto **novo** citar
um plugin que sumiu (por exemplo, numa versão de teste), a faixa abre muda com o aviso "plugin não disponível", e a
árvore dele é guardada e salva de volta intacta: um dado nunca é apagado por faltar quem o entenda.

### 8. Threads com tipo

- Os tipos do documento só aceitam a thread principal: toda mutação passa por `transact`, que confere.
- Os tipos do render só existem do lado do render; o documento não os inclui.
- A exportação é **outra sessão de render** sobre uma cópia imutável do programa. Ela não chama nada do documento.
  "Exportar com gravação ativa" deixa de existir como problema, porque a exportação não toca na `Song`.
- O dispositivo de áudio tem uma barreira: `stop()` espera a callback em andamento sair (um contador na base).
  Nenhum `terminate()`.
- Elimina a lacuna 6.

---

## Especificação da base (aprovada pelo usuário em 19/11)

Os itens 1 a 10 entram no esqueleto ou na primeira fatia, porque mudam a forma do núcleo ou custam pouco se entrarem
cedo. Os itens 11 a 15 vêm depois que a base estiver firme, mas a base é desenhada para recebê-los sem mudar de
forma.

### 1. Tempo sem perda de precisão

- **Duas unidades, uma ponte:**
  - posição musical em tiques, `int64`, com **960 tiques por semínima** (`project.ppq`);
  - posição de áudio em amostras, `int64`.
  - O mapa de andamentos (`tempoMap`) é a única conversão entre as duas. Mudar o andamento não mexe em nenhum
    clipe; muda só onde cada tique cai em amostras.
- **O mapa de andamentos** é uma lista de pontos `{tick, bpm, timeSig}`, com rampa linear opcional entre pontos. A
  conversão tique→amostra é exata e acumulada em `double` a partir do ponto anterior, nunca somando erro período a
  período.
- **Itens de áudio** têm posição em tiques (andam com a música) ou, por escolha do item, em amostras (ficam presos ao
  tempo real, como uma gravação ao vivo que não deve esticar). O campo `timebase: "musical" | "audio"` decide.
- **Tudo é preciso na amostra.** O render divide o bloco nos instantes dos eventos (nota, ponto de automação, início
  de item) e processa cada pedaço com o valor certo. Um plugin recebe os eventos com o deslocamento em amostras dentro
  do bloco (como no CLAP) e não precisa dividir nada sozinho.
- A automação é uma curva (pontos com forma `curve` de −1 a 1) avaliada por amostra quando o parâmetro pede (volume,
  pan) ou por sub-bloco de até 32 amostras (botões de plugin), e sempre suavizada.

### 2. Um tipo de faixa só

- **A faixa é o canal.** Toda faixa tem volume, pan, mudo, solo, número de canais, cadeia de plugins, envios e saída.
  Ela pode ter itens de áudio e de MIDI misturados, e pode ter instrumento (o primeiro plugin da cadeia recebe MIDI e
  gera áudio) ou não.
- **Pasta é faixa** (`"parent": "t:20"` nas filhas). A faixa pai soma as filhas, que passam pela cadeia dela.
- **O master é uma faixa** (`"role": "master"`). Não há "canal do mixer" separado nem tipos de faixa diferentes.
- **O mixer é uma vista** das faixas, não um objeto à parte.
- O editor de padrões do LMMS não existe como modelo. Um "padrão" é um item MIDI que se repete; se for preciso, uma
  vista de passos edita itens MIDI.

### 3. Roteamento como grafo, com compensação de latência

- **Envios** (`sends`) de qualquer faixa para qualquer faixa, pré ou pós fader, e para uma **entrada de sidechain**
  de um plugin (`{"to": "p:7", "port": "sidechain"}`).
- O compilador do programa de render monta o grafo, **recusa ciclos** (a transação que criaria um ciclo é recusada,
  com mensagem), ordena e divide entre os workers.
- **PDC:** cada plugin informa a sua latência. O compilador calcula, para cada caminho até o master, o atraso que
  falta e insere linhas de atraso, reservadas no programa. Mudar a latência de um plugin recompila. Medidores e cursor
  mostram o tempo compensado.
- **Barramentos com N canais:** cada faixa tem `channels` (1, 2, 6...). Os plugins declaram as portas que aceitam, e
  o compilador põe conversões explícitas (mono→estéreo pelo pan, estéreo→mono) onde a contagem muda.

### 4. Gravação que nunca perde um take

- **Escrita contínua em disco:** os blocos gravados vão do áudio para uma fila sem trava e de lá para uma thread de
  disco, que escreve em **RF64** (WAV sem limite de 4 GB) e atualiza o cabeçalho a cada poucos segundos. Uma queda
  deixa um arquivo legível até os últimos segundos. Ao abrir de novo, o programa oferece recuperar os takes órfãos de
  `audio/`.
- **Latência de ida e volta** (saída + entrada do dispositivo + a do sistema) medida pela calibração e guardada nas
  preferências por dispositivo. O take é posicionado descontando-a, então cai na grade.
- **O relógio do take é o do dispositivo** (o número da amostra do período em que o bloco chegou), não o relógio do
  sistema.
- **Pré-roll, punch in/out e loop:** em loop, cada volta vira uma camada (`lanes`) do mesmo item. A escolha do melhor
  pedaço de cada camada é a lista `comp`. Trocar o comp é uma transação, e nenhum áudio é apagado.
- **Monitoramento de entrada** por faixa (`"monitor": "auto" | "on" | "off"`). Em `auto`, monitora quando armada e
  parada ou gravando. O caminho de monitoramento passa pelos plugins da faixa, com latência mínima.

### 5. Diário de transações

- Cada transação confirmada é anexada a `backup/diario.jsonl`, como uma linha com a diferença (os nós que mudaram). A
  escrita é feita por uma thread de trabalho; a interface não espera.
- Ao abrir uma pasta com diário mais novo que o `projeto.json` (o programa caiu), o programa reaplica o diário e volta
  à **última ação**. O diário é zerado a cada salvamento.
- O autosave continua, como cópia periódica do `projeto.json` em `backup/`.

### 6. Toda ação é dado

- Uma ação tem `id` (`"track.add"`, `"transport.record"`), nome traduzível, parâmetros (num `state::Value`) e uma
  função que a executa como uma transação (ou um comando do transporte).
- O registro de ações é do núcleo e não depende do Qt. Menus, atalhos, barras e a **paleta de comandos** (Ctrl+K) são
  vistas desse registro.
- Os atalhos ficam na árvore de preferências (`"keys": { "transport.record": "R" }`).
- A API externa (item 11) chama as mesmas ações.

### 7. Contrato de plugin no estilo CLAP

O contrato da peça 7 segue o desenho do CLAP onde ele resolve os mesmos problemas:
- **eventos com deslocamento em amostras** dentro do bloco (nota, parâmetro, MIDI);
- **portas de áudio e de nota declaradas** (o que alimenta o roteamento do item 3);
- **latência informada** (o que alimenta a PDC);
- **regras de thread explícitas**: o que é chamado na thread principal e o que é chamado no áudio;
- **estado como dado**: a árvore da peça 7a.

Os plugins internos e os de terceiros passam pelo mesmo contrato. **Plugins de terceiros rodam em processo
separado:** o áudio vai e volta por memória compartilhada, sem alocar, e o processo do plugin é dirigido pelo período.
Se ele cair, a faixa fica muda com aviso, e o plugin é recriado a partir da árvore.

### 8. Taxa de amostragem fixa por projeto

- `project.sampleRate` é escolhida ao criar o projeto. O dispositivo é aberto nessa taxa; se ele não aceitar, o
  programa avisa e reamostra **na borda** (um só conversor de alta qualidade na saída e na entrada).
- Amostras importadas numa taxa diferente são convertidas uma vez, em alta qualidade, ao entrar em `audio/`, e o
  arquivo convertido é o que o projeto usa. Nenhum reamostrador roda nos itens durante a reprodução, a não ser que o
  usuário peça um efeito de pitch ou estiramento.

### 9. Render determinístico

- Exportar o mesmo projeto duas vezes dá o mesmo arquivo, bit a bit, em qualquer máquina com a mesma compilação.
- Todo aleatório (ruído, humanizar, osciladores com fase aleatória) usa um gerador por nó, semeado a partir de
  `project.seed` e do id do nó. Nunca `rand()`, nunca o relógio.
- A soma de sinais de vários workers é feita numa ordem fixa, decidida pelo compilador, e não pela ordem em que os
  workers terminam.

### 10. Testes que a arquitetura permite

- **Transações aleatórias + desfazer tudo** voltam à árvore original, bit a bit.
- **Ida e volta pelo JSON** de qualquer árvore válida.
- **Fuzzing** do leitor de JSON e da validação por esquema: nenhum arquivo, por pior que seja, derruba o programa.
- **Os testes de cada plugin** gerados pelo esquema (peça 7a).
- **Render com arquivo de referência:** projetos de teste no formato novo, renderizados e comparados bit a bit (o
  item 9 permite).
- **O `lmms-headless` no CI**, sem tela, com ASan/UBSan, TSan e LeakSanitizer.

### 11. API de controle externo (depois)

- Um servidor local (socket, com JSON por linha) que expõe as ações (item 6), a leitura da árvore e as assinaturas
  por id. OSC para superfícies de controle, sobre as mesmas ações.
- Como tudo é ação e dado, scripts, superfícies de controle e agentes de IA comandam o programa pelo mesmo caminho que
  a interface.

### 12. Interface desenhada pela GPU (depois)

- Formas de onda, medidores, régua e rolagem desenhados pela GPU (Qt Quick ou `QRhi`), com os picos vindos do cache
  de recursos.
- Temas como dado (uma árvore de cores e medidas nas preferências). A `gui` lê o tema, e nada no código tem cor fixa.

### 13. Plugins de terceiros, por ordem (depois)

LV2 e CLAP primeiro (abertos, e o CLAP é quase o próprio contrato do item 7), depois VST3. Todos pelo processo
separado do item 7.

### 14. Áudio do sistema (depois da primeira fatia)

- Linux: PipeWire pela API do JACK, e ALSA direto. Mac: CoreAudio.
- Trocar de dispositivo sem fechar o programa: a sessão para o dispositivo (com a barreira), reabre e recompila.
- Um dispositivo desconectado no meio vira um aviso, e o programa continua parado, sem cair.

### 15. Interface traduzível (desde o primeiro texto)

Todo texto visível passa pela tradução desde o primeiro commit da `gui` (`tr()` no Qt; no núcleo, as mensagens são
chaves com parâmetros, traduzidas pela interface). Português e inglês desde o começo.


## Como cada lacuna desaparece

| Lacuna | Por que deixa de existir |
|---|---|
| 1 Destruição numa fase | Ninguém fora do dono tem ponteiro; o áudio não tem lista de objetos do modelo para sair |
| 2 Sem recolher memória | Programa e processadores aposentados por período; o áudio não alcança o documento |
| 3 Posse dos play handles | Vozes são do processador; quem pede prévia tem um id |
| 4 Modelo ↔ vista | Vistas só têm id e assinam transações; widget nunca é dono de modelo |
| 5 Áudio emite sinais | O render não conhece o Qt; automação e tempo saem pela telemetria |
| 6 Afinidade de thread | Tipos separados por lado; exportação é outra sessão de render |
| 7 Trabalho sob a trava | Não há trava de mudanças |
| 8 Globais | `Session` explícita, ordem de destruição pelos membros |
| 9 Contrato dos plugins | `prepare` aloca, `process` não; vozes pelo host; verificado no CI |

---

## Custo, sem esconder

- **É a reescrita do núcleo:** `Song`, `Track` e derivados, `Clip` e derivados, `AudioEngine`, `Mixer`, `Plugin`, o
  desfazer, a automação e todos os plugins internos. As vistas são adaptadas para ids e transações.
- **O formato de projeto quebra:** o programa novo não abre `.mmp`/`.mmpz` (decisão de 19/11). Os projetos de
  demonstração e os projetos de teste da bateria A/B (`tools/check/ab.sh`) precisam ser refeitos no formato novo, e a
  A/B passa a comparar o motor novo com referências gravadas, não com o motor velho.
- **Por um tempo, funcionalidades somem** e só voltam quando cumprem o contrato. É a troca que o usuário pediu: 10
  estáveis antes de 200.

## Um programa novo, não uma migração (decisão do usuário, 19/11)

"Não vamos mais manter compatibilidade nenhuma: estamos fazendo um NOVO programa." Não há motor novo crescendo ao
lado do velho nem funcionalidades passando de um para o outro. O programa novo nasce do zero, com a estrutura
deste documento. O código velho é **material de consulta**: algoritmos de DSP, drivers de áudio e MIDI, a lógica de
gravação e de medidores. Nunca serve de arquitetura, e nenhum arquivo dele é ligado ao programa novo sem ser
reescrito no contrato novo.

### O repositório

Mesmo repositório, direto no `master`, com duas pastas na raiz:

```
LinuxMusicBeta/            ← o programa novo
  CMakeLists.txt           ← build próprio, independente do legado
  core/                    ← SEM Qt (tools/check/qt-free-dirs.txt)
    state/                 ←   state::Value, esquemas, JSON
    document/              ←   raiz, histórico, ids, transações, assinaturas, ResourceStore
    render/                ←   compilador, programa, processadores, vozes, ParameterStore, telemetria, aposentadoria
    io/                    ←   dispositivos de áudio e MIDI (com a barreira), arquivos
    session/               ←   Session, pump()
  plugins/                 ← SEM Qt: um diretório por plugin, esquema + processador
  gui/                     ← Qt: janelas, vistas, editores dos plugins (gui/plugins/<id>/), editor genérico
  app/                     ← o executável com interface
  headless/                ← lmms-headless: abrir, tocar, gravar, exportar sem interface
  tests/
  docs/                    ← este documento e os que descendem dele
legado/                    ← o fork do LMMS como estava, congelado, só para consulta
```

- **`legado/`** recebe tudo que hoje está na raiz (src, include, plugins, cmake, data, tests, tools, buildtools,
  `CMakeLists.txt`, `vcpkg.json`), movido com `git mv` num commit só, para o histórico de cada arquivo continuar
  seguindo. Ele continua compilando a partir de `legado/`, para consulta e comparação de som, mas ninguém mais
  desenvolve nele. Uma tag `lmms-legado` marca o último commit antes da mudança.
- **Os documentos** de `docs/` que descrevem o programa velho (plano de refatoração, pendências, etapas) vão para
  `legado/docs/`. Este documento, `LinuxMusicBeta/docs/plano-ciclo-de-vida.md` (as lições) e os designs de interface aprovados
  (cabeçalho de faixa REAPER, janela do instrumento) vão para `LinuxMusicBeta/docs/`: os designs aprovados viram
  especificação da interface nova.
- **Quando:** depois que cada sessão terminar o que tem pela metade no código velho e commitar (decisão do usuário).
  O movimento é um commit só, feito quando nenhuma outra sessão estiver com arquivos modificados.

### A ordem de construção

1. **Esqueleto sem funcionalidade, com testes** (a API vem na próxima seção deste documento):
   - `state::Value` + esquemas + JSON estável;
   - `Document` + ids + transações + histórico + assinaturas;
   - `Session` + `pump()`;
   - `ParameterStore`, telemetria, `RenderProgram` + compilador + aposentadoria por período;
   - um dispositivo de áudio com barreira;
   - o `lmms-headless` tocando um programa vazio.
2. **A primeira fatia vertical**, testada de ponta a ponta no `lmms-headless` e depois na interface:
   - faixa de áudio com gravação e monitoramento de entrada (o objetivo do fork);
   - uma faixa de instrumento com o TripleOscillator;
   - o mixer com um efeito, o master e o transporte;
   - salvar e abrir a pasta do projeto.
3. **Os plugins de validação** (lista abaixo), cada um com os testes gerados pelo esquema.
4. **As outras funcionalidades, uma a uma.** Cada uma entra só com os testes de contrato (ciclo de vida, tempo real,
   editar tocando, desfazer, ida e volta pelo JSON).

### Redes de proteção do programa novo (desde o primeiro commit)

- `qt-free.sh` com `LinuxMusicBeta/core` e `LinuxMusicBeta/plugins`.
- Regras de busca, só em `LinuxMusicBeta/`:
  - `terminate()`;
  - `Qt::DirectConnection`;
  - `connect` com lambda sem objeto de contexto (clazy `connect-3arg-lambda`, em `gui/`);
  - membro ponteiro para tipo do núcleo numa vista, exceto a `Session` (as vistas guardam ids);
  - `new`/`delete` fora dos alocadores, e destrutor que chama a `Session`.
- `AudioThreadAllocationTest` reescrito para o render novo, valendo para todo plugin desde o primeiro.
- ASan/UBSan e TSan nos testes do programa novo; LeakSanitizer no Linux.

## O que preciso que o usuário decida

1. ~~A direção~~: aprovada em 19/11, a arquitetura nova.
2. **A fatia vertical do passo 3:** é essa a lista de funcionalidades que precisa existir primeiro?
3. **Os plugins que continuam funcionando** durante a reescrita (proposta abaixo; os outros saem até serem reescritos).

### Proposta: plugins que ficam funcionando para validar a base

A ideia é cobrir cada tipo de problema do contrato com um plugin, e não cobrir funcionalidades:

| Plugin | Por que este | O que ele valida |
|---|---|---|
| TripleOscillator | sintetizador principal | vozes, parâmetros, onda do usuário como recurso (o bug de troca sem trava) |
| AudioFileProcessor | sampler | `ResourceStore`, amostra trocada tocando |
| AvlDrums | baterias (trabalho já feito no fork) | kit como recurso, muitas vozes curtas, roubo de voz |
| Eq | efeito com parâmetros em bloco | `seqlock` dos coeficientes |
| Compressor | dinâmica | estado de DSP contínuo, `adopt` |
| Delay ou ReverbSC | cauda longa | `adopt` sem cortar a cauda |
| um LV2 qualquer | terceiros | o adaptador com `"chunk"` e os workers do LV2 |

O Amplifikation Lite está sendo escrito agora por outra sessão; o melhor é ele nascer direto no contrato novo
(decisão do usuário).

---

## O que outros programas fazem, e o que muda aqui por causa disso

Pesquisa de 19/11, nas fontes primárias (código e documentação). Itens marcados "não verificado" vêm de memória ou
de fonte secundária.

### Confirma a direção

- **JUCE `AudioProcessorGraph`** monta uma `RenderSequence` imutável na thread de mensagens (chamando `prepareToPlay`
  dos nós novos ali) e a entrega ao áudio por troca de ponteiro. O áudio usa `ScopedTryLock`: se não conseguir, fica
  com a sequência antiga, em vez de silêncio. O comentário no código diz: "Swap pointers rather than assigning to avoid
  calling delete here". A sequência velha é apagada por um `Timer` de 500 ms na thread de mensagens. Cada nó da
  sequência segura o seu `Node::Ptr`, então um nó removido vive até a sequência velha morrer. É o desenho das peças 3
  e 7 deste documento.
  ([juce_AudioProcessorGraph.cpp](https://github.com/juce-framework/JUCE/blob/master/modules/juce_audio_processors_headless/processors/juce_AudioProcessorGraph.cpp))
- **Tracktion Engine**: os plugins pertencem ao modelo, e o grafo de render só os referencia ("can only be
  *referenced* by the graph, not *owned* by it"). Os detalhes da troca do grafo não foram verificados.
  ([fórum JUCE](https://forum.juce.com/t/node-graph-clarifications/56170))
- **Ardour** passa `Session&` para todo lado (só o `AudioEngine` é global), tem `ENSURE_GUI_THREAD`, que **aborta
  também em release**, e confere que a trava de processamento está pega com `assert(!process_lock().trylock())`.
  ([gui_thread.h](https://github.com/Ardour/ardour/blob/master/libs/gtkmm2ext/gtkmm2ext/gui_thread.h))

### O que o Ardour ensina pelo que deu errado

O Ardour usa a mesma estrutura de hoje (o processamento e a interface mexem nos mesmos objetos, `shared_ptr` e RCU
por lista), e o código dele mostra os mesmos sintomas:

- `~Route` desconecta "cedo, para não receber sinais enquanto é destruído" e evita `clear_processors` porque "a sessão
  pode estar meio destruída". É a nossa lacuna 1, resolvida à mão em cada classe.
- O último `shared_ptr` pode cair na thread de processamento e rodar o destrutor ali. Para o grafo de processamento,
  eles mandam a destruição para a thread "butler".
- `RCUManager::flush()` tem o comentário "XXX may be unsafe", e um teste de `use_count()` foi desligado com `#if 0`
  por causa de uma corrida.

Isso reforça a escolha de **não** usar `shared_ptr` + RCU por lista como solução geral: o render não deve ter posse
compartilhada de nada que a interface também segure.
([rcu.h](https://github.com/Ardour/ardour/blob/master/libs/pbd/pbd/rcu.h),
[route.cc](https://github.com/Ardour/ardour/blob/master/libs/ardour/route.cc),
[session.cc](https://github.com/Ardour/ardour/blob/master/libs/ardour/session.cc))

### Ajustes neste documento por causa da pesquisa

1. **Aposentadoria por período (a peça 3) com margem de dois períodos.** O áudio incrementa o contador no fim do
   período, depois de os workers terminarem (hoje em `AudioEngine.cpp:435`), com `release`. A thread principal anota
   o contador **depois** da troca, com `acquire`, e libera quando ele passou de `anotado + 2`. Com o dispositivo parado,
   a barreira do `stop()` (peça 8) conta como ponto de quiescência. É o QSBR (recolhimento por estado de quiescência)
   especializado para um leitor de tempo real. O custo no áudio é um atômico por período, sem registro de leitores. O
   `reclaim_object` do `crill` faz o mesmo de forma genérica, mas só existe no ramo `timur/rcu` e exige registrar os
   leitores com mutex; o contador de período é mais simples e já cobre os workers.
   ([reclaim_object.h](https://github.com/crill-dev/crill/blob/timur/rcu/include/crill/reclaim_object.h))
2. **Parâmetros:** um `std::atomic<float>` por vaga continua sendo o certo para valores soltos. Para blocos de valores
   que precisam chegar juntos (os coeficientes de um filtro, um conjunto de pontos de envelope), usar um `seqlock`
   (`crill::seqlock_object`: um escritor, tipo trivialmente copiável, leitura sem espera quando não há escrita
   concorrente), em vez de vários atômicos que o áudio pode ler pela metade.
   ([seqlock_object.h](https://github.com/crill-dev/crill/blob/main/include/crill/seqlock_object.h))
3. **Vozes e prévias por *slot map*** (índice + geração, P0661): capacidade fixa alocada no `prepare()`, busca O(1)
   sem alocar, e um id velho resolve para nada. A geração com 32 bits torna a volta do contador desprezível. Os
   pedidos (parar, consultar) vão por fila de comandos. P0661 e o EnTT não foram verificados na fonte.
   ([P0661](https://wg21.link/p0661))

### Regras do Qt que a pesquisa confirmou

- `QPointer` só fica nulo dentro do `~QObject`, **depois** dos destrutores derivados: ele protege contra ponteiro
  solto, mas não serve de aviso de ciclo de vida. Com ids (peça 2), a interface quase não precisa dele.
  ([QObject](https://doc.qt.io/qt-6/qobject.html))
- `deleteLater` não roda dentro de laços aninhados (modais, `processEvents`). Nada do modelo depende dele.
- Uma conexão em fila copia argumentos ponteiro sem garantia de vida, e emitir na fila aloca e pega uma trava (não
  verificado no código do Qt). Por isso a telemetria é fila própria + temporizador, nunca sinal.
- O `QTimer` + fila é o mesmo que o JUCE recomenda; o próprio JUCE avisa que `triggerAsyncUpdate()` pode travar e não
  deve ser chamado do áudio.
  ([juce_AsyncUpdater.h](https://github.com/juce-framework/JUCE/blob/master/modules/juce_events/broadcasters/juce_AsyncUpdater.h))
- **No CI:** o clazy tem a checagem `connect-3arg-lambda` (lambda sem objeto de contexto). Ela entra na fase 1.

### Diferença deliberada em relação a todos eles

JUCE, Tracktion e Ardour ainda deixam o processador ser o mesmo objeto que o modelo do plugin (`AudioProcessor`,
`Plugin`). Aqui eles são separados: o `PluginModel` fica no documento e o `Processor` no render (peça 7). É mais
trabalho por plugin, mas tira do plugin a possibilidade de a vista ou o modelo tocarem no estado de DSP, que é o
erro do TripleOscillator, do Xpressive e do Vestige. Se o custo de reescrever todos os plugins pesar demais, o
recuo é o modelo do Tracktion (o plugin pertence ao modelo, e o grafo só referencia), mantendo todo o resto deste
documento.
