# Arquitetura nova: proposta radical

Estado: **direção aprovada pelo usuário em 19/11; implementação ainda não autorizada.** Substitui o desenho incremental de
`legado/docs/design-ciclo-de-vida.md`. As evidências estão em `LinuxMusicBeta/docs/plano-ciclo-de-vida.md`.

Decisões do usuário (19/11):
- Os plugins são reescritos no contrato novo; não há o recuo para o modelo do Tracktion.
- Os plugins **quebram por um tempo** e ficam incompatíveis: só alguns principais continuam funcionando, para
  validar a base. Os outros são reescritos depois, sobre a base pronta.
- **O estado de cada plugin é um objeto chave-valor (uma árvore como JSON)**, e carregar, descarregar e salvar é
  reinserir esses valores (peça 7, "Estado do plugin").
- **Não só o plugin: cada pedaço do programa vira dado** que se exporta como objeto chave-valor. O projeto inteiro é
  uma árvore de valores (peça 2, "O programa como dado"). Os objetos do programa só leem a árvore ou executam a partir
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
- **Uma faixa pode ter várias saídas, mono ou estéreo, cada uma para um canal da mesa** (19/11; a bateria é o caso
  típico).
- **Faixa e mesa de mixagem são separadas, com roteamento livre** (19/11): a faixa é a linha do tempo e manda para um
  canal da mesa; vários podem mandar para o mesmo; o rack e os efeitos moram nos canais (especificação, item 2).
- **O rack da mesa analógica será portado.** Todo código de efeito (do rack, dos plugins) mora no pacote `lm-dsp` e
  é reaproveitado por quem precisar (19/11).
- **Plataforma: Linux com ALSA primeiro, macOS depois** (19/11). Configurações que juntam dispositivos são sempre
  explícitas.
- **O núcleo é escrito em Rust.** O compilador impede use-after-free, ponteiro nulo e corrida entre threads, os
  bugs que as auditorias acharam. A interface sai de um protótipo comparando GPUI, Slint e Qt. As bibliotecas em C
  (libsndfile, FluidSynth, lilv, libsamplerate) são ligadas por FFI, em pacotes `lm-sys-*` com ficha de segurança. A
  lógica dos drivers e o DSP do legado são portados, e o DSP com teste A/B contra o original. Onde este documento
  diz "C++" ou mostra código em C++, vale a forma em Rust de `interfaces.md`.
- **As interfaces entre as peças são novas**, desenhadas do zero para o modelo seguro (`interfaces.md`).
- **Toda operação que possa passar de 200 ms tem tempo e estado de carregamento:** roda fora da thread principal, o
  objeto aparece no lugar final marcado como carregando, e a interface nunca mostra um estado intermediário
  (`interfaces.md`, I-2).

**Este documento é o "porquê"; o "como" é `interfaces.md`.** Em qualquer conflito de detalhe entre os dois, vale
`interfaces.md`. A revisão de coerência de 19/11 alinhou as peças 2 a 8 com ele.

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
     │ lê por id, puxa as mudanças (Feed)
     ▼
  Documento (o projeto)  ── cópia ──► Compilador ── Install ──► Renderer (dono dos processadores)
   dono único de tudo                 (thread                    aplica as Install em ordem,
   objetos referidos por id           própria)                   nunca espera, nunca aloca,
   desfazer = raízes anteriores                                  não conhece o Qt nem a árvore
     ▲                                                               │
     ├──────── devolução (o que o áudio largou; liberado aqui) ◄─────┤
     │  parâmetros (vagas atômicas) ───────────────────────────────► │
     │  comandos (transporte, MIDI, prévia) ───────────────────────► │
     └──────── telemetria (quadro a ~60 Hz + anéis de eventos) ◄─────┤
                                                                     │
  Thread de disco ◄──────── gravação (anel próprio, que nunca perde) ┘
```

Só **estas coisas cruzam** entre as threads, cada uma com o seu tipo (`interfaces.md`, Parte 2, regra 3). Nada mais
cruza.

1. **Instalação e devolução:** o programa e os processadores vão ao áudio numa `Install`; tudo que o áudio larga
   volta pela devolução e é liberado na thread principal.
2. **Vagas de parâmetro:** atômicas, de capacidade fixa.
3. **Anéis de um produtor para um consumidor**, de tamanho fixo e dimensionados em tempo: comandos para o áudio,
   eventos de telemetria, a gravação para o disco. Valores contínuos (medidores, posição) vão num quadro de
   telemetria.
4. **Dados imutáveis compartilhados** (`Arc` de algo que ninguém muda: áudio carregado, picos).

---

## As peças

### 0. O núcleo não depende do Qt, depende do dado (decisão do usuário, 19/11)

**A regra:** tudo abaixo da interface é Rust **sem nenhuma dependência do Qt ou de qualquer biblioteca de
interface**. A interface é só uma camada que lê o dado e pede transações.

| Camada (pacotes, `interfaces.md` Parte 3) | Pode usar biblioteca de interface? | O que é |
|---|---|---|
| `lm-state` (valor, esquema, JSON) | não | a árvore, a serialização, a validação |
| `lm-doc` (raiz, histórico, ids, transações, `Feed`s) | não | o modelo |
| `lm-render`, `lm-compile` (programa, processadores, vozes, vagas, telemetria) | não | a execução |
| `lm-io` (dispositivos de áudio e MIDI, arquivos, recursos) | não | a borda com o sistema |
| `lm-session` (dono de tudo acima, `pump()`) | não | a montagem |
| `lm-plugin`, `lm-plugins-*` (esquema + processador) | não | o contrato do plugin |
| `lm-gui-*` (vistas, editores dos plugins, janelas) | **sim** | uma interface sobre o dado |

**O que substitui o que vinha do Qt no núcleo:**
- Sinais e slots → `Feed`s do documento (por id ou tipo de nó, que a interface puxa a cada quadro) e a telemetria.
  Ninguém no núcleo emite sinal.
- `QObject`, pai do Qt, `QPointer` → não há objeto do modelo vivo (peça 2); ids no lugar de ponteiros.
- `QString` → `String`/`Arc<str>` em UTF-8. `QDomDocument`/XML → o formato da árvore é JSON. Não há XML em lugar
  nenhum: o formato antigo não é lido (decisão de 19/11).
- `QThread`, `QMutex`, `QTimer` → threads da biblioteca padrão criadas pela `Session`, atômicos, anéis, e o
  `session.pump()`, chamado por um timer do laço de eventos (sem interface, pelo laço do `lm-headless`).
- `QFile`, `QDir`, `QSettings` → `std::fs`/`std::path` e a árvore de preferências.

**A interface é guiada pelo dado:**
- Uma vista recebe um id, lê o nó pelo `Ref`, desenha, e a cada quadro puxa o `Feed` do id e redesenha o que mudou.
- Uma ação do usuário vira uma transação (em código Rust, direto; I-15).
- A vista não guarda cópia do estado, nem ponteiro para objeto do núcleo além da `Session`.
- O widget morre: o `Feed` dele morre junto, e nada mais acontece.

**Garantias de build:**
- os pacotes do núcleo **não dependem** do Qt nem de crate de interface: o `cargo-deny` (lista de crates proibidas
  por pacote) e um teste do grafo de dependências conferem isso a cada build;
- o `lm-headless` abre um projeto, toca, grava e exporta usando só o núcleo. Se ele funciona sem nenhuma biblioteca
  de interface, a regra está de pé.

### 1. `Session`: o fim das globais

- Uma `Session` é dona de tudo, e a ordem dos membros é a ordem de destruição, escrita num lugar só: `interfaces.md`,
  I-16 (o dispositivo é o primeiro a morrer).
- Ela é passada a quem precisa. Não existe `Engine::getSong()`, `getGUI()` nem `ConfigManager::inst()` no núcleo.
- Testes criam várias sessões no mesmo processo.
- A ordem de criação e destruição é a ordem dos membros, escrita num lugar só e verificada pelo compilador.
- Elimina a lacuna 8 e os 32 destrutores que dependem de globais.

### 2. O programa como dado (decisão do usuário, 19/11)

**A regra:** todo o estado do programa que importa é uma **árvore de valores imutável** (`state::Value`, a mesma da
peça 7a). Projeto, faixas, clipes, notas, automação, mixer, roteamento, transporte, mapa de andamentos, plugins,
preferências: tudo é dado. **Nenhum objeto do programa guarda estado próprio.** Cada objeto é de um destes três tipos:

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
  "tempoMap": [ { "tick": 0, "bpm": "120", "timeSig": [4, 4] } ],   // bpm: decimal exato em texto (racional, I-18)
  "tracks": [
    { "id": "t:17", "name": "Baixo", "parent": null, "volume": 0.8, "mute": false, "solo": false,
      "instrument": { "id": "p:5", "plugin": "tripleoscillator", "version": 1, "state": { "params": { "vol1": 0.8 } } },
      "outputs": [ { "id": "o:20", "port": "main", "channels": 2, "to": "m:3" } ],
      "items": [
        { "id": "c:52", "kind": "midi", "start": 0, "length": 3840,
          "notes": { "812": { "key": 57, "start": 0, "length": 480, "velocity": 0.79 } } }
      ],
      "automation": [ { "id": "a:9", "target": { "node": "p:5", "param": "vol1" },
                        "points": [ { "tick": 0, "value": 0.8 }, { "tick": 1920, "value": 0.2, "curve": 0.0 } ] } ] },
    { "id": "t:18", "name": "Voz", "parent": null, "volume": 1.0, "mute": false, "solo": false,
      "input": { "device": "in", "channels": [1] }, "armed": false, "monitor": "auto",
      "outputs": [ { "id": "o:21", "port": "main", "channels": 1, "to": "m:4" } ],
      "items": [
        { "id": "c:60", "kind": "audio", "start": 0, "length": 7680,
          "lanes": [
            { "id": "l:61", "source": { "kind": "audio", "key": "<hash>" }, "sourceOffset": 0 },
            { "id": "l:62", "source": { "kind": "audio", "key": "<hash>" }, "sourceOffset": 0 } ],
          "comp": [ { "from": 0, "to": 3840, "lane": "l:62", "fade": 240 },
                    { "from": 3840, "to": 7680, "lane": "l:61" } ],
          "fadeIn": 48, "fadeOut": 48, "gain": 1.0 } ] },
    { "id": "t:30", "name": "Bateria", "parent": null, "volume": 1.0, "mute": false, "solo": false,
      "instrument": { "id": "p:31", "plugin": "avldrums", "version": 1, "state": { "kit": "black-pearl" } },
      "outputs": [
        { "id": "o:32", "port": "kick",  "channels": 1, "to": "m:10" },
        { "id": "o:33", "port": "snare", "channels": 1, "to": "m:11" },
        { "id": "o:34", "port": "oh",    "channels": 2, "to": "m:12" },
        { "id": "o:35", "port": "room",  "channels": 2, "to": "m:12" } ],
      "items": [] }
  ],
  "mixer": {
    "master": { "id": "m:0", "channels": 2, "volume": 1.0, "rack": { "...": "..." }, "inserts": [],
                "output": { "device": "out", "channels": [1, 2] } },
    "channels": [
      { "id": "m:3", "name": "Graves", "channels": 2, "volume": 1.0, "pan": 0.0, "mute": false, "solo": false,
        "rack": { "tube": { "on": true, "drive": 0.3 }, "eq": { "...": "..." }, "dynamics": { "...": "..." } },
        "inserts": [ { "id": "p:6", "plugin": "compressor", "version": 1, "state": { "params": {} } } ],
        "sends": [ { "id": "s:7", "to": "m:5", "amount": 0.3, "pre": false } ],
        "output": "m:0" },
      { "id": "m:4", "name": "Voz", "channels": 1, "volume": 1.0, "pan": 0.0, "mute": false, "solo": false,
        "rack": { "...": "..." }, "inserts": [], "sends": [], "output": "m:0" },
      { "id": "m:5", "name": "Reverb", "channels": 2, "volume": 0.7, "pan": 0.0, "mute": false, "solo": false,
        "rack": { "...": "..." },
        "inserts": [ { "id": "p:8", "plugin": "reverbsc", "version": 1, "state": { "params": {} } } ],
        "sends": [], "output": "m:0" }
    ]
  }
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
  - chaves em ordem alfabética (`interfaces.md`, I-3.1);
  - números de ponto flutuante escritos de forma que voltem exatamente iguais (`ryu`, a representação mais curta que volta igual);
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
- `session.transact(label("Mover clipe"), |e| e.set(clip_id, &[key("start")], Value::int(384)))` monta uma raiz nova que divide com a
  velha tudo que não mudou (dados persistentes: só o caminho até o nó alterado é copiado).
- **O desfazer é a lista de raízes anteriores.** Não existe comando inverso escrito à mão, nem `JournallingObject`,
  nem mapa de ids do histórico.
- Ao confirmar, a transação calcula a **diferença** (nós criados, alterados e removidos, por id) e:
  - deixa a diferença pronta para os `Feed`s daqueles ids, uma vez;
  - avisa o compilador, que recompila só o que mudou: um nó de render é reaproveitado quando o nó da árvore é o
    mesmo objeto, o que se confere por identidade e não por comparação profunda.
- **Gestos contínuos** (arrastar um botão, mover uma nota com o mouse) viram **um** passo de desfazer, criado ao soltar.
  Durante o arrasto, o parâmetro vai ao áudio pela vaga atômica, sem esperar a transação. As regras completas (vários
  gestos ao mesmo tempo, cancelar, desfazer no meio) estão em `interfaces.md`, I-3.7.

**O armazenamento:** a árvore é feita de vetores e mapas persistentes, com compartilhamento estrutural (a crate `imbl`,
se passar na medida das 10 mil notas editadas uma a uma; `interfaces.md`, Parte 4).

**Por que isso resolve a raiz:**
- **Ciclo de vida do modelo:** não existe, porque o modelo não tem objetos vivos. Um valor não morre na mão de
  ninguém; um id que não resolve mais é só um id que não resolve.
- **Desfazer, salvar, abrir, copiar, colar, duplicar faixa, presets, recuperação após queda e autosave** viram a mesma
  operação sobre a mesma árvore. Hoje cada um é um caminho de código separado, e cada um tem os seus bugs.
- **Recuperação após queda:** o autosave é só escrever a raiz atual numa thread de trabalho. Como a raiz é imutável,
  dá para escrever sem travar nada.
- **Testes:** qualquer estado do programa é um arquivo JSON que um teste carrega.

### 2a. `Document`: o dono da árvore e os ids

O "como" está em `interfaces.md`, I-3. O essencial:
- O documento guarda **a raiz atual**, **o histórico** (as raízes anteriores, com o `Diff` de cada passo) e **dois
  índices persistentes**, que andam junto com cada raiz: id → (pai, lista) e as referências reversas (quem aponta para
  cada id).
- **Não há objetos do modelo vivos.** Quem lê uma faixa usa um `TrackRef { id }`, que lê a raiz atual. Um id que
  sumiu responde "não existe", nunca lê memória liberada.
- **Fora da árvore, ninguém guarda ponteiro para o modelo: guarda o id.** Dentro dela também: o canal da faixa, o alvo
  da automação e o envio são ids.
- **Toda mudança é uma transação**, recusada inteira se deixar a árvore inválida. A mudança é entregue uma vez, e
  quem quer saber dela a puxa (`Feed`): ninguém é chamado de volta.
- **Remover é um passo só**, e as referências ao que saiu são tratadas na mesma transação (`OnRemove`). A destruição
  em duas fases (lacuna 1) deixa de ser necessária.
- Elimina as lacunas 1, 3 (do lado da interface) e 4.

### 3. O programa de render, imutável

O "como" está em `interfaces.md`, I-5. O essencial:
- O compilador lê uma cópia da árvore e produz uma `Install`: o programa (listas planas de nós, rotas, atrasos da PDC,
  eventos de cada item, curvas de automação) e os processadores a acrescentar, trocar ou remover.
- **Os processadores pertencem ao `Renderer`**, na thread de áudio, e só ele os mexe. O estado de DSP que precisa
  continuar entre programas (vozes, caudas, linhas de atraso) fica neles.
- **O áudio aplica as instalações em ordem e devolve tudo que saiu** (o programa velho, os processadores removidos ou
  trocados) por um anel de devolução. A thread principal libera. **O áudio nunca aloca nem libera.**
- **Edição pequena não recompila.** Mudar um parâmetro escreve uma vaga atômica. Mover uma nota recompila só a lista
  de eventos daquele item.
- **Sem trava de mudanças.** O áudio nunca espera a interface e nunca toca silêncio por causa dela.
- Elimina as lacunas 2, 3 (do lado do áudio) e 7.

### 4. Parâmetros: vagas atômicas

O "como" está em `interfaces.md`, I-6:
- vagas atômicas de **capacidade fixa**, alocadas ao criar o motor, com um bitset de vagas sujas;
- quem dá o número de cada vaga é o compilador;
- o processador só vê **eventos** de parâmetro, precisos na amostra, e a automação é avaliada no render;
- com a vaga "tocada" (a mão mexendo), a mão vence a automação.

O valor automatizado não é escrito de volta no modelo: vai pela telemetria, para a interface mostrar o botão andando.
Elimina a lacuna 5 pela raiz: não existe `setTempo` rodando no áudio nem sinal emitido de lá.

### 5. Telemetria: o único caminho do áudio para a interface

O "como" está em `interfaces.md`, I-8:
- **valores contínuos** (medidores, posição) vão num quadro publicado a ~60 Hz, e quem lê pega o mais recente;
- **eventos** vão em anéis de tamanho fixo, com política escrita para quando enchem;
- o que **carrega posse** (devoluções) vai em anéis que nunca perdem.

O render não conhece o Qt nem a árvore. A gravação tem o próprio anel até a thread de disco (`interfaces.md`, I-11).

### 6. Eventos para o áudio: o fim dos play handles cruzando threads

O "como" está em `interfaces.md`, I-7:
- notas do MIDI, transporte e prévias viram **comandos**, num anel por produtor;
- o MIDI chega com carimbo de tempo, e o note-off vai sempre para quem recebeu o note-on (U25);
- **as vozes são do processador do instrumento**, alocadas no build, e nunca cruzam threads;
- quem pede uma prévia guarda um `PreviewId`, não um ponteiro.

Elimina a lacuna 3 (a posse dos play handles), o `PresetPreviewPlayHandle`, a afinidade de thread do
`SamplePlayHandle` e os vazamentos do TapTempo e do `clearNewPlayHandles`.

### 7. Plugins: contrato novo, todos reescritos

O "como" está em `interfaces.md`, I-9. O essencial:
- **O descritor** declara o esquema (os campos e os parâmetros), as portas e a função `build`, que constrói o
  processador **a partir da árvore**, numa operação de fundo, alocando ali tudo o que ele vai precisar (inclusive para
  qualquer valor de qualquer parâmetro, U31).
- **O processador** (`process`, `reset`, `latency`, `adopt`) roda no áudio e **não aloca, não trava, não lança**.
- **O estado do plugin é a árvore, e nada mais** (7a).
- **Recursos grandes** (amostra, soundfont, tabela de onda) são imutáveis e compartilhados. Trocar a amostra é trocar a
  referência na árvore; o processador novo é construído com o recurso novo e entra numa `Install`, com o antigo
  tocando até ele ficar pronto (T7). Isso conserta, por construção, o TripleOscillator, o Xpressive, o Patman e o AFP.
- **Vozes:** o `VoicePool` do host, com a política de roubo e os modos poly, mono e legato (U27). Nunca aloca para
  ganhar polifonia (o Mallets e o FreeBoy alocavam).
- **O editor** (a janela do plugin) é registrado na `gui` e recebe só o id do nó, o `Feed` dele e os dados ao vivo do
  plugin (U38). Não toca no processador nem é dono de nada.
- **Plugins de terceiros** passam pelo mesmo contrato por um adaptador, fora do processo (I-9.6).
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

Os tipos (`Value`, os esquemas, `ParamSpec`, o descritor) estão em `interfaces.md` (I-3.1, I-3.3, I-9.1). As regras
que valem para todo plugin:
- os parâmetros ficam em `"params"`, por **chave de texto permanente** (`interfaces.md`, C5: renomear exige
  apelido);
- as escolhas (kits, formas de onda, modos) são por chave de texto, nunca por número (U30);
- mudar um valor padrão exige subir a versão do esquema (U29);
- estado opaco acima de 64 KB vai como recurso (U24);
- os plugins de terceiros guardam `"params"` e `"chunk"` (o estado opaco deles).

**O que cada operação vira:**

| Operação | Hoje | Com a árvore |
|---|---|---|
| Salvar o projeto | `saveSettings` de cada plugin, à mão | a árvore vira JSON dentro do arquivo |
| Abrir o projeto | `loadSettings` à mão, com o plugin já vivo | validar pelo esquema → `upgrade` → `build` |
| Preset | outro caminho de XML | uma árvore num arquivo |
| Desfazer uma mudança no plugin | journalling por modelo | a árvore anterior |
| Copiar a faixa | clonar objetos do Qt | copiar a árvore |
| Trocar a taxa de amostragem | cada plugin trata `sampleRateChanged` | `build` de novo, com o `ProcessSpec` novo |
| Descarregar e recarregar | não existe de forma segura | descartar o processador; a árvore fica |
| Plugin caiu (fora do processo) | perde o estado | recria a partir da árvore |

**Testes gerados para todo plugin a partir do esquema** (nenhum plugin volta a funcionar sem passar):
1. padrão → processador → salvar → abrir → a árvore é igual, bit a bit;
2. árvores sorteadas dentro das faixas do esquema → ida e volta igual;
3. **sem estado escondido**: o mesmo estado, construído duas vezes e renderizado com as mesmas notas, dá o mesmo
   áudio bit a bit;
4. trocar uma chave que não é parâmetro, tocando: o processador novo entra sem alocação no áudio e sem estalo;
5. cada versão antiga do esquema, por arquivos guardados nos testes: o `upgrade` chega à versão atual;
6. trocar a taxa de amostragem: reconstrói e soa;
7. os testes da Parte 6 de `interfaces.md`: varredura de cada parâmetro sob a armadilha de alocação (U31), notas
   curtíssimas e redisparo (U26), 30 min de afinação (U35), as quatro taxas (U36).

**Plugins ainda não reescritos** simplesmente não existem no programa novo: não aparecem na lista e não há
marcador. Os projetos antigos não abrem (peça 2), então não há estado antigo a preservar. Se um projeto **novo** citar
um plugin que sumiu (por exemplo, numa versão de teste), a faixa abre muda com o aviso "plugin não disponível", e a
árvore dele é guardada e salva de volta intacta: um dado nunca é apagado por faltar quem o entenda.

### 8. Threads com tipo

O "como" está em `interfaces.md`, I-1 e I-10:
- o que só pode rodar na thread principal pertence à `Session`, que não pode sair dela (`!Send`);
- o lado do áudio é o `Renderer`, que só a callback tem; o pacote do render não conhece a árvore;
- a exportação é **outro motor**, sobre uma cópia da árvore, sem dispositivo; não toca na sessão;
- o dispositivo tem a barreira no `stop()` (e no `Drop`), herdada por todo backend. Nenhum `terminate()`.

Elimina a lacuna 6.

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
- **O mapa de andamentos** é uma lista de pontos `{tick, bpm, timeSig}`, com saltos entre pontos. **Rampas: em
  aberto** (decisão de 12/11 contra F2 no `interfaces.md`, I-18; ver "O que ainda falta decidir"). A
  conversão tique↔amostra é exata: aritmética inteira (`i128`) dentro de cada segmento, com a posição canônica do
  transporte em amostras e o tique sempre recalculado, nunca somado bloco a bloco (`interfaces.md`, I-18).
- **Itens de áudio** têm posição em tiques (andam com a música) ou, por escolha do item, em amostras (ficam presos ao
  tempo real, como uma gravação ao vivo que não deve esticar). O campo `timebase: "musical" | "audio"` decide.
- **Tudo é preciso na amostra.** O render divide o bloco nos instantes dos eventos (nota, ponto de automação, início
  de item) e processa cada pedaço com o valor certo. Um plugin recebe os eventos com o deslocamento em amostras dentro
  do bloco (como no CLAP) e não precisa dividir nada sozinho.
- A automação é uma curva (pontos com forma `curve` de −1 a 1) avaliada por amostra quando o parâmetro pede (volume,
  pan) ou por sub-bloco de até 32 amostras (botões de plugin), e suavizada. **Parâmetros discretos** (`Choice`,
  `Stepped`, `Bool`) **nunca são interpolados nem suavizados**: mudam no ponto exato.

### 2. Faixas e mesa de mixagem: duas coisas separadas (decisão do usuário, 19/11)

"Faixa é uma coisa, e mesa de mix é outra." O modelo tem duas partes, ligadas por **roteamento livre**, como o
FX Mixer do LMMS:

**A faixa** é a linha do tempo:
- os itens de áudio e de MIDI, a automação, a entrada, armar e monitorar (que são da faixa, nunca do canal: decisão
  de 12/11);
- o instrumento, numa faixa que toca MIDI (um plugin que recebe notas e gera áudio);
- **volume, mudo e solo**, e **sem pan** (decisão do usuário, 19/11): o pan é do canal da mesa. O botão de pan do
  cabeçalho aprovado no design 02 sai;
- **as saídas** (decisão do usuário, 19/11): uma faixa tem **uma lista de saídas**, cada uma mono, estéreo ou com
  mais canais, e **cada saída manda para um canal da mesa**. O caso típico é a bateria: bumbo e caixa em mono,
  overheads e ambiência em estéreo, cada um no seu canal. Uma faixa simples tem uma saída só.
  - **De onde vêm as saídas:** numa faixa com instrumento, das **portas de saída que o instrumento declara** (o kit
    do AvlDrums declara uma porta por peça ou grupo, com nome e número de canais). Numa faixa de áudio, de uma
    divisão dos canais do item (um take de 8 canais pode sair como 8 mono, ou 4 estéreo).
  - **Cada saída tem chave estável** (`"kick"`, `"oh"`), como os parâmetros (`interfaces.md`, C5): trocar o kit por
    outro que tenha a mesma peça mantém o roteamento dela.
  - **Uma saída nova** (o instrumento passou a declarar mais portas) nasce mandando para o mesmo canal da saída
    principal. **Uma saída que sumiu** sai da lista na mesma transação, com aviso.
  - **Várias saídas, de várias faixas, podem mandar para o mesmo canal.** Uma faixa nova **não** cria canal: a saída
    principal nasce mandando para o master, e o usuário escolhe outro. (Uma ação "criar canais para as saídas" faz
    isso de uma vez, por exemplo para a bateria, mas é uma ação, não um automatismo.)
- tipos: um só, que pode ter itens de áudio e de MIDI. O editor de padrões do LMMS não existe como modelo: um
  "padrão" é um item MIDI que se repete, e uma vista de passos edita itens MIDI.
- **Pasta** (`"parent"`) organiza as faixas na linha do tempo (recolher, mudar, solar o grupo). Ela **não soma áudio**:
  somar é trabalho da mesa.

**Uma faixa com várias saídas:**
- o volume e o mudo da faixa valem para **todas** as saídas (decisão do usuário, 19/11). A faixa não tem pan: o pan é
  sempre o do canal da mesa;
- o medidor do cabeçalho da faixa (design 02) mostra a **saída principal**, com uma indicação de que há outras;
- o monitoramento da faixa armada passa pela **saída principal** e pelo canal para onde ela vai. Duas faixas armadas
  que mandam para o mesmo canal somam ali, o que é o esperado.

**A mesa de mixagem** é a soma:
- **canais**, que existem por si, criados e removidos na mesa;
- cada canal tem **o rack da mesa analógica** (válvulas com memória, EQ, dinâmica, transformador, tolerância: o
  trabalho do legado, portado para o `lm-dsp`), **inserts** (plugins de efeito), volume, pan, mudo, solo e **envios**
  para outros canais (pré ou pós fader);
- a **saída** de cada canal vai para outro canal ou para o master;
- o **master** é um canal especial, único, que vai para o dispositivo;
- os efeitos moram na mesa (inserts dos canais); a faixa só tem o instrumento;
- **a automação mora no dono do parâmetro**: as lanes de um canal da mesa (volume, pan, botões do rack, parâmetros
  dos inserts) ficam em `"automation"` do próprio canal; as da faixa e do instrumento dela, na faixa. Apagar uma faixa
  nunca leva junto a automação da mesa.

**Os dois solos** (padrão proposto, a confirmar pelo usuário; o solo do FX Mixer do LMMS teve bugs exatamente aqui):
- **solar uma faixa** silencia as outras faixas; a mesa continua inteira (os retornos de reverb seguem tocando o que
  chega a eles);
- **solar um canal da mesa** deixa tocar tudo que manda para ele (as faixas e os canais acima) e o caminho dele até o
  master (os canais abaixo); o resto da mesa fica mudo;
- **os dois ligados ao mesmo tempo**: vale a interseção. Toca só o que passa por uma faixa solada **e** por um canal
  solado (ou pelo caminho dele até o master);
- é regra do compilador, testada com um grafo de exemplo para cada caso.

### 3. Roteamento como grafo, com compensação de latência

- O grafo do render é: **saídas das faixas → canais da mesa → master**, mais os **envios entre canais** e as **entradas de
  sidechain** dos inserts.
- **Sidechain:** a fonte é um envio de um canal da mesa (`{"id": "s:9", "to": "p:7", "port": "sidechain"}`) ou uma
  saída de faixa; o destino é a porta de sidechain de um insert. Se o insert some, o envio de sidechain sai junto
  (`OnRemove::RemoveOwner`).
- O compilador monta o grafo, **recusa ciclos** (a transação que criaria um ciclo entre canais é recusada, com
  mensagem), ordena e divide entre os workers. Quando várias faixas vão para o mesmo canal, a soma segue uma ordem
  fixa (render determinístico).
- **Remover um canal** para onde saídas de faixas mandam: essas saídas passam a mandar para o master, na mesma
  transação, com aviso. Remover uma faixa não mexe na mesa.
- **PDC:** cada plugin e o rack informam a sua latência. O compilador calcula, para cada caminho até o master, o
  atraso que falta e insere linhas de atraso, reservadas no programa. Mudar a latência recompila. Medidores e cursor
  mostram o tempo compensado.
- **Barramentos com N canais:** cada faixa e cada canal da mesa têm `channels` (1, 2, 6...). Os plugins declaram as
  portas que aceitam, e o compilador põe conversões explícitas (mono→estéreo pelo pan, estéreo→mono) onde a contagem
  muda.
- **Monitoramento** (H5): a entrada da faixa armada passa pelo canal para onde a faixa manda (com o rack e os inserts
  dele), fora da PDC.

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
  Esse caminho **não recebe o atraso da PDC** (`interfaces.md`, H5).

**Decisões de gravação do usuário (12/11) que continuam valendo no programa novo:**
- os takes são gravados **secos** (o monitoramento passa pelos plugins da faixa, mas o arquivo é o da entrada);
- apertar Gravar **tocando** faz punch-in; sem faixa armada, Gravar arma a faixa selecionada;
- uma entrada pode alimentar várias faixas; entradas mono ou estéreo;
- **num xrun, a gravação continua** e o ponto é marcado no take (um marcador visível no item);
- ao parar, a gravação **sempre fica**, e o desfazer a remove;
- o **metrônomo vai só para o monitoramento**: não entra na gravação nem na exportação (padrão; uma saída própria do
  dispositivo pode ser escolhida);
- latência: a informada pelo driver, a calibração por loopback (o resíduo, `interfaces.md` H19) **e um ajuste manual**
  por dispositivo;
- trocar de dispositivo, de bloco ou de taxa **sem reiniciar** o programa (fora da gravação, U10);
- fades e crossfades automáticos nas bordas dos itens; cada item tem o próprio ganho;
- MIDI: entrada por faixa (dispositivo e canal, padrão todos), overdub por padrão com a opção por faixa (overdub,
  substituir, camada nova); quantização da entrada opcional, desligada.

Superadas por decisões posteriores: a pasta `<projeto>_media/` (agora `audio/` dentro da pasta do projeto) e o
desfazer por comandos (agora a lista de raízes). Em conflito, à espera do usuário: ver "O que ainda falta decidir".

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

- Exportar o mesmo projeto duas vezes dá o mesmo arquivo, bit a bit, em qualquer máquina com a mesma compilação. Vale com
  plugins internos; plugins de terceiros podem não ser determinísticos (`interfaces.md`, H18). Toda thread de render liga
  FTZ/DAZ (H6).
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
- **O `lm-headless` no CI**, sem tela, com ASan/UBSan, TSan e LeakSanitizer.

### 11. Controle e gerência remotos (depois; desenhado em `interfaces.md`, I-20)

Decisão do usuário (19/11): o núcleo é **controlável e gerenciável remotamente**; **não há backend de áudio em rede**
(áudio em rede, se alguém quiser, vem pelo PipeWire ou pelo JACK). **O protocolo externo é separado da interface
Rust:** dentro do programa, a interface gráfica fala com o núcleo por código; para fora, há um protocolo próprio, só
de dados, versionado e estável.

O texto original deste item, que o I-20 detalha:

- Um servidor local (socket, com JSON por linha) que expõe as ações (item 6), a leitura da árvore e as assinaturas
  por id. OSC para superfícies de controle, sobre as mesmas ações.
- Como tudo é ação e dado, scripts, superfícies de controle e agentes de IA comandam o programa pelo mesmo caminho que
  a interface.

### 12. Interface desenhada pela GPU (depois)

- Formas de onda, medidores, régua e rolagem desenhados pela GPU, pela biblioteca que o protótipo escolher (GPUI, Slint ou Qt), com os picos vindos do cache
  de recursos.
- Temas como dado (uma árvore de cores e medidas nas preferências). A `gui` lê o tema, e nada no código tem cor fixa.

### 13. Plugins de terceiros, por ordem (depois)

LV2 e CLAP primeiro (abertos, e o CLAP é quase o próprio contrato do item 7), depois VST3. Todos pelo processo
separado do item 7.

### 14. Áudio do sistema

**Linux com ALSA primeiro, macOS depois** (decisão do usuário, 19/11). Já era a decisão de 12/11 para a gravação:
"ALSA primeiro, depois JACK", com o `hw` em exclusivo e o ReserveDevice1 para um dispositivo ocupado.

- **F1:** ALSA direto (`hw` em exclusivo, captura e reprodução ligadas), e o ALSA seq para MIDI.
- Depois: JACK (e o PipeWire pela API do JACK). Depois: CoreAudio e CoreMIDI, no Mac.
- Configurações que juntam dispositivos são explícitas (`interfaces.md`, U7).
- Trocar de dispositivo sem fechar o programa: a sessão para o dispositivo (com a barreira), reabre e recompila.
- Um dispositivo desconectado no meio vira um aviso, e o programa continua parado, sem cair.

### 15. Interface traduzível (desde o primeiro texto)

Todo texto visível passa pela tradução desde o primeiro commit da `gui` (`tr()` no Qt; no núcleo, as mensagens são
chaves com parâmetros, traduzidas pela interface). Português e inglês desde o começo.


## Como cada lacuna desaparece

| Lacuna | Por que deixa de existir |
|---|---|
| 1 Destruição numa fase | Ninguém fora do dono tem ponteiro; o áudio não tem lista de objetos do modelo para sair |
| 2 Sem recolher memória | O áudio devolve tudo que larga pelo anel de devolução; a thread principal libera; o áudio não alcança o documento |
| 3 Posse dos play handles | Vozes são do processador; quem pede prévia tem um id |
| 4 Modelo ↔ vista | Vistas só têm id e puxam as mudanças; widget nunca é dono de modelo |
| 5 Áudio emite sinais | O render não conhece o Qt; automação e tempo saem pela telemetria |
| 6 Afinidade de thread | Tipos separados por lado; exportação é outra sessão de render |
| 7 Trabalho sob a trava | Não há trava de mudanças |
| 8 Globais | `Session` explícita, ordem de destruição pelos membros |
| 9 Contrato dos plugins | o `build` aloca, o `process` não; vozes pelo host; verificado no CI |

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
LinuxMusicBeta/            ← o programa novo: um workspace do Cargo
  Cargo.toml               ← o workspace; build próprio, independente do legado
  rust-toolchain.toml      ← a versão do Rust, fixada
  deny.toml                ← cargo-deny: crates e licenças permitidas
  crates/
    lm-base, lm-state, lm-doc, lm-time, lm-render, lm-plugin, lm-compile, lm-io, lm-session, lm-remote
    lm-dsp                 ← os códigos de efeito e síntese (portados do legado, reaproveitados por todos)
    lm-plugins-*           ← os plugins internos
    lm-sys-*               ← as ligações com C e com o sistema (o único lugar com unsafe)
    lm-headless            ← o executável sem interface
    lm-gui-*               ← a interface (a biblioteca que o protótipo escolher)
  docs/                    ← este documento e os que descendem dele
legado/                    ← o fork do LMMS como estava, congelado, só para consulta
```

Os pacotes, as dependências entre eles e o que cada um contém estão em `interfaces.md`, Parte 3.

- **`legado/`** recebe tudo que hoje está na raiz (src, include, plugins, cmake, data, tests, tools, buildtools,
  `CMakeLists.txt`, `vcpkg.json`), movido com `git mv` num commit só, para o histórico de cada arquivo continuar
  seguindo. Ele continua compilando a partir de `legado/`, para consulta e comparação de som, mas ninguém mais
  desenvolve nele. Uma tag `lmms-legado` marca o último commit antes da mudança.
- **Os documentos** de `docs/` que descrevem o programa velho (plano de refatoração, pendências, etapas) vão para
  `legado/docs/`. Este documento, `LinuxMusicBeta/docs/plano-ciclo-de-vida.md` (as lições) e a pasta `design-sugestoes/` vão para
  `LinuxMusicBeta/docs/`. **Só os designs aprovados viram especificação da interface nova:** o 2 (cabeçalho de faixa,
  agora sem o knob de pan), o 3 (barra de transporte), o 5 (janela do instrumento) e o 8 (configurações). **Os outros
  (1, 4, 6, 7 e 9) não estão aprovados**: são só sugestões, esperando a revisão do usuário, e ninguém os implementa
  como especificação.
- **Quando:** depois que cada sessão terminar o que tem pela metade no código velho e commitar (decisão do usuário).
  O movimento é um commit só, feito quando nenhuma outra sessão estiver com arquivos modificados.

### A ordem de construção

1. **Esqueleto sem funcionalidade, com testes** (a ordem e os critérios estão em `interfaces.md`, Parte 4):
   - `state::Value` + esquemas + JSON estável;
   - `Document` + ids + transações + histórico + `Feed`s;
   - `Session` + `pump()`;
   - as vagas de parâmetro, a telemetria, o programa + compilador + instalação e devolução;
   - um dispositivo de áudio com barreira;
   - o `lm-headless` tocando um programa vazio.
2. **A primeira fatia vertical**, testada de ponta a ponta no `lm-headless` e depois na interface:
   - faixa de áudio com gravação e monitoramento de entrada (o objetivo do fork);
   - uma faixa de instrumento com o TripleOscillator;
   - o mixer com um efeito, o master e o transporte;
   - salvar e abrir a pasta do projeto.
3. **Os plugins de validação** (lista abaixo), cada um com os testes gerados pelo esquema.
4. **As outras funcionalidades, uma a uma.** Cada uma entra só com os testes de contrato (ciclo de vida, tempo real,
   editar tocando, desfazer, ida e volta pelo JSON).

### Redes de proteção do programa novo (desde o primeiro commit)

Os detalhes estão em `interfaces.md` (Parte 2, I-17 e Parte 4). Em resumo:
- `#![forbid(unsafe_code)]` em todo pacote que não é `lm-sys-*`;
- `cargo-deny`: crates e licenças permitidas, e nenhum pacote do núcleo dependendo de Qt ou de biblioteca de interface;
- `clippy` com `-D warnings` e `disallowed-methods` (por exemplo, os métodos de ponto flutuante que caem na `libm` do
  sistema);
- a armadilha de alocação (`assert_no_alloc`) em toda callback de áudio dos testes;
- `loom` e `miri` onde há sincronização própria;
- ASan, TSan e LSan nos testes, no Linux;
- os testes gerados para cada plugin a partir do esquema.

## Decisões do usuário sobre o começo (19/11)

Aprovados ("aprovo tudo por enquanto"):
- a primeira fatia: gravar com monitoramento de entrada, uma faixa com o TripleOscillator, um efeito, o master, o
  transporte, salvar e abrir;
- os plugins de validação da tabela abaixo;
- o protótipo de interface (GPUI, Slint, Qt) de `interfaces.md`, I-15.

**A implementação não está autorizada.** Nenhum código em `LinuxMusicBeta/` até o usuário dar a ordem.

### O que ainda falta decidir (19/11)

- **Itens gravados e o andamento.** Em 12/11 o usuário decidiu que itens gravados são **sempre musicais**: seguem o
  BPM por time-stretch (provavelmente o Rubber Band), renderizado a partir do original guardado. A especificação nova
  (item 1) dá a cada item a escolha `musical` ou `audio` e não tem time-stretch na F1. Qual vale, e o time-stretch
  entra na F1?
- **Rampas de andamento.** Em 12/11: marcadores de andamento com saltos e rampas. A especificação nova deixa as rampas
  para a F2.
- **Um arquivo por entrada ou por faixa e por passada.** Em 12/11: um arquivo intercalado por faixa e por passada. O
  desenho novo (U1) grava um arquivo contínuo por entrada, com os cortes de loop.
- **Crossfade padrão do comping** (U2: proposta de 5 ms) e **a velocidade** (U28).

### Proposta: plugins que ficam funcionando para validar a base

A ideia é cobrir cada tipo de problema do contrato com um plugin, e não cobrir funcionalidades:

| Plugin | Por que este | O que ele valida |
|---|---|---|
| TripleOscillator | sintetizador principal | vozes, parâmetros, onda do usuário como recurso (o bug de troca sem trava) |
| AudioFileProcessor | sampler | `ResourceStore`, amostra trocada tocando |
| AvlDrums | baterias (trabalho já feito no fork) | kit como recurso, muitas vozes curtas, roubo de voz |
| Eq | efeito com vários parâmetros mudando juntos | eventos de parâmetro no mesmo offset, aplicados juntos |
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

### O que a pesquisa mudou, e o que veio depois

- **Recolher o que o áudio largou:** a pesquisa sugeriu o recolhimento por período (QSBR, um contador por período e
  dois períodos de margem). O desenho final é mais simples e mais forte: **o áudio devolve tudo que larga por um anel
  de devolução**, e a thread principal libera (`interfaces.md`, I-5). O contador de período ficou só como ideia de
  apoio.
- **Valores que precisam chegar juntos** (coeficientes de filtro): a pesquisa sugeriu um `seqlock`. O desenho final
  usa **eventos de parâmetro no mesmo offset**, que o processador aplica juntos (`interfaces.md`, I-6.2). Não há
  `seqlock`.
- **Vozes por slot map** (índice + geração): ficou, no `VoicePool` (`interfaces.md`, I-9.2). As referências P0661 e
  EnTT não foram verificadas na fonte; o desenho não depende delas.

### Regras do Qt que a pesquisa confirmou (valem só se o protótipo escolher o Qt para a interface)

- `QPointer` só fica nulo dentro do `~QObject`, **depois** dos destrutores derivados: ele protege contra ponteiro
  solto, mas não serve de aviso de ciclo de vida. Com ids (peça 2), a interface quase não precisa dele.
  ([QObject](https://doc.qt.io/qt-6/qobject.html))
- `deleteLater` não roda dentro de laços aninhados (modais, `processEvents`). Nada do modelo depende dele.
- Uma conexão em fila copia argumentos ponteiro sem garantia de vida, e emitir na fila aloca e pega uma trava (não
  verificado no código do Qt). Por isso a telemetria é fila própria + temporizador, nunca sinal.
- O `QTimer` + fila é o mesmo que o JUCE recomenda; o próprio JUCE avisa que `triggerAsyncUpdate()` pode travar e não
  deve ser chamado do áudio.
  ([juce_AsyncUpdater.h](https://github.com/juce-framework/JUCE/blob/master/modules/juce_events/broadcasters/juce_AsyncUpdater.h))
- **Se a interface for em Qt** (decidido pelo protótipo): o clazy, com a checagem `connect-3arg-lambda` (lambda sem
  objeto de contexto), entra no CI da `gui`.

### Diferença deliberada em relação a todos eles

JUCE, Tracktion e Ardour ainda deixam o processador ser o mesmo objeto que o modelo do plugin (`AudioProcessor`,
`Plugin`). Aqui eles são separados: o estado do plugin é a árvore, no documento, e o `Processor` fica no render (peça 7). É mais
trabalho por plugin, mas tira do plugin a possibilidade de a vista ou o modelo tocarem no estado de DSP, que é o
erro do TripleOscillator, do Xpressive e do Vestige. Se o custo de reescrever todos os plugins pesar demais, o
recuo é o modelo do Tracktion (o plugin pertence ao modelo, e o grafo só referencia), mantendo todo o resto deste
documento.
