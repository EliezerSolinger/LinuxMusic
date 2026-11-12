# Preparação para o mixer no estilo FL Studio

Decisões tomadas em 11/11/2026, antes de redesenhar o mixer. Referência de produto: o mixer e o gerenciamento de
efeitos do FL Studio (veja [README.md](README.md)).

## Decisões

### Efeitos só no mixer
- Todo efeito fica nos **slots dos canais do mixer**, como no FL Studio.
- O rack de efeitos próprio das faixas (instrumento e sample) **será removido**.

### Um canal automático por faixa
- Toda faixa nova **cria e liga um canal do mixer** com o mesmo nome e cor (a ideia do LMMS #6407).
- Continua sendo possível mandar várias faixas para o mesmo canal.

### Volume e pan da faixa viram atalhos
- O knob de **volume** e o de **pan** da faixa passam a ser **atalhos** para o fader e o pan do canal ligado a ela.
- Só existe um volume e um pan de verdade, no mixer.
- Se várias faixas mandam para o mesmo canal, mexer no atalho de uma muda o canal de todas.

### O canal do mixer ganha
- **pan**;
- **largura estéreo**, **inverter fase** e **trocar L/R**;
- **ganho de entrada (trim)**, antes dos efeitos;
- **sends pré e pós-fader** para outros canais.

### Núcleo de áudio preparado antes da interface nova
- **Compensação de latência (PDC):** os efeitos informam a latência e o mixer alinha os caminhos.
- **Medidores seguros e RMS:** corrigir a condição de corrida na leitura dos picos e acrescentar pico retido e RMS.
- **Entrada do canal:** o modelo de dados diz de onde vem o som do canal (faixas ou uma entrada da interface de áudio),
  preparando a gravação da etapa 3, sem gravar ainda.
- **Desfazer/refazer no mixer:** criar, apagar e mover canais e mudar envios entram no histórico.
- **Sidechain e multicanal (#7459):** entram nesta fase, reimplementados sobre o código atual (as primeiras partes, como
  `AudioBuffer`, já estão no `master` do LMMS).

### Organização em hierarquia (decisão de 11/11/2026)
- O usuário vê **hierarquia**: trilhas-pasta e canais de grupo (como as *folder tracks* do REAPER e os grupos do
  Bitwig/Ableton). Uma pasta "Bateria" com bumbo, caixa e pratos dentro soma num canal de grupo "Bateria", que vai
  para o master; recolher a pasta esconde as trilhas filhas, e o fader do grupo controla todas.
- Pastas podem conter pastas. No mixer, o canal de grupo aparece com os canais filhos ao lado ou recolhidos dentro
  dele.
- A **pista grupo é só agrupamento**, como no Ableton/Bitwig (e diferente do REAPER, onde a pasta pode ter itens):
  - sem clipes, sem gravação, sem instrumento e sem entrada de áudio própria;
  - tem um canal de grupo no mixer (fader, pan, mute/solo, efeitos e envios) que recebe a soma das filhas;
  - mute do grupo silencia tudo dentro; solo numa filha mantém o grupo (e os grupos acima) tocando;
  - recolhida, a linha do grupo mostra um resumo dos clipes das filhas, só para ver.
- Os **canais extras em pares** (1/2, 3/4…) do núcleo multicanal ficam **internos**: são o encanamento do sidechain e
  das saídas múltiplas, e só aparecem no pin connector, como recurso avançado. A tela não fala em "canais 3/4".
- **Sidechain pelos dois lados:** no canal de origem, "enviar como sidechain para o canal X" (estilo FL Studio), e no
  plugin, um seletor "fonte do sidechain" (estilo Ableton/Bitwig). Os dois criam o mesmo envio para os pares internos.
- **Instrumento multi-saída** (ex.: bateria AVL): "um canal por peça" cria **só no mixer** um grupo com um canal
  filho por peça, ligados por baixo pelos pares de canais, pinos e envios. O arranjo continua com uma única pista
  do instrumento, com as notas.
- O menu provisório "Track channels" / "Sidechain send" do canal some quando a interface nova entrar.

### Interface nova do mixer
- Slots de efeito visíveis em cada canal.
- **Arrastar e soltar** plugins do navegador direto num slot ou num canal.
- **Menu de slot:** trocar plugin, mover e salvar/carregar preset.

### Qualidade
- **Testes automatizados** de mixagem e roteamento são escritos **antes** de mexer no núcleo.
- O branch **ainda não** é enviado ao GitHub, então não há CI de Linux por enquanto (decisão reconfirmada em
  11/11/2026: os testes no Linux, como VST pelo Wine, LV2 em execução e ALSA/JACK/PulseAudio, ficam para mais tarde). Tudo continua sendo compilado só
  no macOS; o risco de algo quebrar no Linux sem percebermos fica registrado.

## Ordem de implementação

1. Testes de mixagem e roteamento sobre o comportamento atual.
2. Canal completo no núcleo: pan, largura estéreo, fase, L/R, trim, sends pré/pós.
3. Medidores seguros, pico retido e RMS.
4. Entrada do canal no modelo de dados.
5. Desfazer/refazer no mixer.
6. Sidechain e multicanal (#7459), reimplementados.
7. Compensação de latência (PDC).
8. Canal automático por faixa; volume e pan da faixa viram atalhos; remoção do rack de efeitos das faixas.
9. Hierarquia: trilhas-pasta e canais de grupo no modelo de dados (com testes de soma, mute/solo em cascata,
   salvar/carregar e desfazer), e envio de sidechain e "uma trilha por peça" sobre os pares internos.
10. Interface nova do mixer: slots por canal, arrastar e soltar, menu de slot, pastas e grupos recolhíveis.
11. "Um canal por peça" para instrumentos multi-saída (bateria AVL): cria o grupo com um canal por peça no mixer
    e faz o roteamento interno. Depende dos passos 9 e 10; até lá, as saídas da bateria são roteadas à mão pelo pin
    connector e pelos envios.

## Andamento

### Passo 1: testes (feito)

`tests/src/core/MixerTest.cpp` cobre o comportamento atual:
- sinal no master e volume do master;
- canal novo enviando ao master;
- volume do canal vezes quantidade do envio;
- soma de canais e canais encadeados;
- mute e solo;
- limpeza dos buffers entre períodos;
- detecção de loop de roteamento;
- apagar canal;
- medidor de pico pós-fader.

Detalhes que os testes revelaram:
- `Engine::init()` liga a thread do driver Dummy, que processa o mixer sozinha. Os testes a param
  (`AudioEngine::stopProcessing()`, agora público) e avançam os períodos manualmente.
- **Comportamento a corrigir no passo 2:** a rampa que suaviza mudanças de parâmetro só avança quando o canal processa
  sinal. Se o fader de um canal em silêncio muda e depois chega som, o som começa com uma rampa a partir do valor
  **antigo**. O processamento novo do canal deve consumir as mudanças de parâmetro em todo período, com ou sem sinal.

### Passo 2: canal completo (feito)

Caminho do sinal de cada canal, na ordem:
1. entradas: faixas e envios de outros canais, pré ou pós-fader;
2. **trim** (−24 a +24 dB);
3. efeitos;
4. ponto **pré-fader**, de onde saem os envios pré-fader;
5. **trocar L/R** e **inverter fase**;
6. **largura estéreo** (mid/side: 0% = mono, 100% = original, 200% = dobro);
7. **pan**;
8. **fader**;
9. envios pós-fader e saída.

Detalhes:
- Mudança estrutural: **cada canal aplica o próprio fader**. Antes, quem aplicava o volume de um canal era o canal que
  recebia o sinal, o que impedia envios pré-fader.
- O master também aplica o próprio fader no processamento dele.
- Corrigido o problema da rampa: o canal lê os parâmetros em todo período, com ou sem sinal.
- Os parâmetros novos e a opção pré-fader dos envios são salvos no projeto. Projetos sem esses atributos carregam com
  os valores padrão.
- `MixerTest` tem 22 testes, incluindo cada etapa do caminho do sinal, a rampa e salvar/carregar.
- Um demo renderizado antes e depois tem o mesmo nível (RMS 3.101 contra 3.103). O render dos demos não é
  determinístico (ruído e fase aleatória), então não dá para comparar amostra por amostra.
- Os controles novos ganharam interface no passo 12 (painel do canal selecionado).

### Passo 3: medidores seguros e RMS (feito)

- Classe nova `ChannelMeter`: a thread de áudio grava os níveis em atômicos, sem lock.
- A interface lê com `takePeaks()`, que devolve o maior pico desde a leitura anterior, ou nada se não houve período
  novo.
- Acabou a condição de corrida: antes, a interface lia e zerava variáveis comuns que a thread de áudio escrevia.
- O RMS é a média dos últimos ~300 ms, por canal L/R.
- O pico retido e a queda gradual continuam na interface (`Fader`).
- O medidor OUT da toolbar usa o RMS do master, porque os picos são consumidos pelo mixer.
- Corrigido outro furo: canais **mutados** não passavam pelo processamento. O medidor não zerava, e mudanças de
  parâmetro feitas com o canal mutado faziam rampa ao desmutar. Agora eles passam por `processMuted()`.
- `MixerTest` tem 25 testes.

### Passo 4: entrada do canal (feito)

Cada canal tem agora, no modelo de dados:
- `m_input` (`ChannelInput`): nenhuma entrada (só faixas) ou entradas da interface de áudio, dizendo o primeiro canal
  e se é mono ou par estéreo. É alterada por `Mixer::setChannelInput()`, que emite `channelInputChanged`;
- `m_recordArmModel`: armado para gravar;
- `m_monitorModel`: monitoração desligada, ligada ou automática (só enquanto armado). O padrão é automática, como no
  REAPER.

Tudo é salvo e carregado com o projeto. O áudio de entrada ainda **não** é lido nem tocado; isso é da etapa 3.

### Passo 5: desfazer/refazer no mixer (feito)

- Estas operações agora entram no histórico: criar, apagar e mover canal, criar e apagar envio, mudar a entrada do
  canal. Volume, pan e outros parâmetros já entravam, pelos próprios modelos.
- **Uma ação é um passo** de desfazer. Essas operações chamam umas às outras (apagar canal apaga envios), então só a
  mais externa registra o ponto (`Mixer::StructureChange`). Durante a mudança, o histórico fica desligado para os
  outros objetos.
- O estado guardado no histórico **inclui o canal de cada faixa**. Apagar um canal renumera os canais das faixas, e
  desfazer devolve cada faixa ao canal certo. Isso só vale para o histórico; o arquivo do projeto não muda.
- `loadSettings` passou a ignorar filhos que não são `mixerchannel`.
- Renomear canal e mudar a cor passaram a entrar no histórico no passo 12.
- `MixerTest` tem 30 testes.

### Passo 6: sidechain e multicanal (feito)

Veja [multicanal.md](multicanal.md), fases 1 a 3b.

### Passo 8: canal automático por faixa (feito)

Feito antes do passo 7 (PDC), que depende da extensão de latência da fase 4 da API.

- **Faixa nova:** toda faixa de instrumento ou de sample criada na música ou no editor de sequências ganha um canal do
  mixer próprio, com o nome e a cor dela, enviando ao master (`Mixer::setUpTrackChannel()`). O canal acompanha o
  nome e a cor da faixa enquanto for dela.
- **Projetos antigos:** ao carregar, cada faixa passa pela mesma regra (`Mixer::setUpTrackChannels()`, no fim de
  `Song::loadProject`):
  - faixa sozinha num canal comum: esse canal passa a ser dela;
  - faixa no master, num canal usado por outras faixas ou num barramento (um canal que recebe envios): ganha um canal
    próprio, que envia para onde a faixa mandava antes. O som e o roteamento continuam os mesmos.
- **Efeitos da faixa:** vão para a frente dos efeitos do canal. O rack de efeitos da faixa sai da interface: a aba
  "Efeitos" da janela do instrumento e a janela da faixa de sample mostram os efeitos do canal.
- **Volume e pan da faixa:**
  - O volume da faixa vinha antes dos efeitos, por isso vira o **trim** do canal, que também vem antes, e um
    compressor ou uma distorção recebem o mesmo nível de antes. O que passa da faixa do trim (±24 dB) vai para o
    fader. O pan da faixa soma ao pan do canal.
  - Os knobs VOL e PAN da faixa (na lista de faixas e na janela) passam a ser **atalhos** do fader e do pan do canal
    (`gui::TrackChannelControls`).
  - **Exceção:** volume ou pan da faixa com automação ou controlador ficam na faixa, porque a automação age sobre
    eles. O knob correspondente continua mostrando o da faixa.
- **Faixa que manda para um canal de outros** (escolhido à mão depois): não tem canal próprio. Os knobs e o rack
  mostram o volume, o pan e os efeitos da própria faixa, que continuam funcionando. Nada se perde.
- **Apagar a faixa** apaga o canal dela, se nada mais usa o canal.
- O trim passou a ter passo de 0,001 dB (era 0,1), para o volume das faixas antigas virar trim sem arredondar.

Diferenças de som medidas na bateria A/B (43 projetos):
- Iguais, a menos de 1 LSB, nos projetos com volume da faixa em 100%.
- Com volume da faixa diferente de 100%, o **primeiro período** com som muda: na build antiga, o volume da faixa
  começava numa rampa de 100% até o valor salvo (o `valueBuffer` do modelo interpolava desde o padrão). Agora o
  volume vale desde a primeira amostra. Efeitos com memória (eco, reverb, compressor) carregam essa diferença no
  rastro. É a correção de um defeito, não uma regressão.

**Corrida entre threads (resolvida no passo 13; registro de como estava):**
- O LB302 sob carga (12 renders em paralelo) sai com a nota deslocada em ~40% das vezes. A build de referência (sem
  as fases 1 a 3) faz o mesmo, então o defeito vem do LMMS.
- Mesma família, só nesta build: sob carga pesada (8 renders em paralelo), cerca de 1 render em 10 de
  instrumentos por nota (TripleOscillator) diverge da referência a partir de um limite de nota, com diferença de fase
  de ~0,5%. As frequências das notas são iguais. Para reproduzir: renderizar `inst-tripleosc` 10 vezes em paralelo e
  comparar. Nas 8 vezes em que rodou, a build de referência não mostrou o problema.
- Investigar as duas junto com a parte F da fase 4 (instrumentos por nota), que muda a ordem em que notas e
  instrumentos são processados.

`MixerTest` tem 46 testes.

### Passo 7: compensação de latência, PDC (feito)

- **Extensão de latência (parte C da fase 4, só esta peça):** `AudioProcessor::latency()` diz quantos quadros a
  saída do plugin atrasa em relação à entrada. `PluginInstance::latency()` devolve 0 com o plugin inativo e com o
  efeito desligado ou com falha, porque nesse caso o áudio passa sem atraso. `EffectChain::latency()` soma as
  latências da cadeia.
- **Diferente do CLAP:** a latência é lida na **thread de áudio**, no início de cada período, e não na principal
  depois de um pedido de reinício. LADSPA e LV2 informam a latência por uma porta de saída que o plugin escreve
  enquanto processa, então esse é o momento em que o valor vale. Por isso `latency()` precisa ser barata e não pode
  travar.
- **Plugins:** LADSPA, pela porta de saída de controle chamada `latency` (a convenção usada pelos plugins). LV2, pela
  porta com `lv2:reportsLatency` (`lilv_plugin_get_latency_port_index`). VST, Carla e ZynAddSubFX ainda informam 0.
- **Mixer** (`Mixer::compensateLatency()`, no início de `masterMix()`):
  - latência de um canal = a da sua entrada mais atrasada + a dos seus efeitos;
  - o som das faixas conta como 0;
  - cada entrada do canal é atrasada para chegar junto com a mais atrasada:
    - o som das faixas, pelo atraso de entrada do canal (todos os pares, para o sidechain também);
    - cada envio, pelo seu próprio atraso, inclusive envios pré-fader e de sidechain.
  - As linhas de atraso (`LatencyDelay`) só são realocadas quando a latência muda. A mudança vale a partir do
    período seguinte, sempre no mesmo período, então o render continua determinístico.
  - Canal mudo esvazia as suas linhas, para não soltar som velho quando volta.
  - `Mixer::latency()` é a latência da mixagem toda. A gravação (etapa 3) vai usá-la para alinhar o que for gravado.
- **Completado no passo 12:** latência de instrumentos, sinal seco dos efeitos com latência e o primeiro período
  dos plugins LADSPA e LV2.
- **Por escolha, como no REAPER:** na reprodução, a mixagem toda sai atrasada pela latência do master; a música não
  é adiantada.
- Testes: `MixerTest` tem 50 (efeito de teste que atrasa e informa a latência: caminhos alinhados, entrada das faixas
  de um barramento, efeito desligado, soma ao longo do caminho).

### Passo 9: hierarquia, trilhas-pasta e canais de grupo (feito)

- **Modelo, como no REAPER:** as faixas continuam numa lista; cada uma tem uma profundidade
  (`Track::folderDepth()`, salva como `folderdepth`). Uma **trilha-pasta** (`FolderTrack`, tipo novo) contém as
  faixas logo abaixo dela que são mais profundas. Uma faixa só pode ficar um nível mais funda que a de cima, e só se
  a de cima for uma pasta (`TrackContainer::updateFolders()` corrige o que vier diferente).
- **A pasta só agrupa:** sem clipes, sem gravação. Tem um **canal de grupo** no mixer
  (`MixerChannel::m_groupChannel`, salvo como `groupchannel`), criado com ela.
- **Roteamento segue a árvore** (`Mixer::updateFolderRouting()`): o canal de cada faixa manda para o canal de grupo da
  sua pasta, ou para o master se estiver no topo. Só as saídas principais para o master ou para canais de grupo são
  mudadas. Um canal mandado à mão só para outros canais fica como está, e envios extras (por exemplo, para um reverb)
  também.
- **Mudo e solo em cascata:**
  - o mudo da pasta é o mudo do canal de grupo;
  - solo numa faixa mantém tocando as pastas que a contêm;
  - solo numa pasta mantém tocando o que ela contém, menos o que já estava mudo.
- **Apagar uma pasta** sobe o que ela contém um nível e apaga o canal de grupo.
- **Desfazer:** a profundidade de todas as faixas é guardada como um objeto só (`TrackContainer::FolderLayout`), então
  mover uma pasta com o que ela contém é um passo. O roteamento não é guardado à parte: é refeito a partir das
  pastas.
- **Interface:**
  - botão "Adicionar trilha-pasta" na barra do editor de arranjo;
  - no menu da faixa, "Mover para a pasta acima" e "Tirar da pasta";
  - faixas dentro de pastas aparecem recuadas;
  - a pasta tem um botão para recolher ou mostrar as faixas, e knobs VOL e PAN do canal de grupo.
- As faixas do editor de sequências não entram em pastas.
- Recolher grupos no mixer veio no passo 10; arrastar faixas para dentro de pastas, no passo 12.
- Os canais do mixer agora mostram o nome da faixa logo ao serem criados (antes ficava "Canal N" até reabrir).
- `MixerTest` tem 55 testes: roteamento para o grupo e de volta, pastas aninhadas, apagar pasta, solo em pasta,
  desfazer, salvar.

### Passo 10: interface nova do mixer (feito, primeira versão)

Decisão: aproveitar o rack de efeitos que já existe (`EffectRackView`/`EffectView`: LED, W/D, decay, controles, pin
connector) como a coluna de slots do canal selecionado, como o painel direito do FL Studio. Um widget novo teria
abertos janelas de controle duplicadas. O que entrou:

- **Menu do slot** (clique direito num efeito):
  - mover para cima e para baixo (arrastar com o mouse já reordenava);
  - **substituir** por outro efeito;
  - **carregar** e **salvar preset**: arquivo `.xfx` do tipo `effectsettings`, guardado em
    `<presets do usuário>/effects/`;
  - **fonte de sidechain**, nos efeitos com mais entradas que o par principal: lista os canais e liga ou desliga o
    envio para as entradas 3/4 deste canal;
  - remover.
- **Sidechain pelos dois lados**, no menu da faixa do mixer: "Entrada de sidechain de ▸" (outros canais para este) e
  "Enviar como sidechain para ▸" (este canal para outros). Os dois usam `Mixer::toggleSidechainSend()`, que dá 4
  canais ao destino quando precisa. O item provisório do passo 6 saiu.
- **Página "Efeitos" no navegador** (`EffectBrowser`):
  - lista os efeitos por categoria, e os LADSPA/LV2 pelo host, com busca;
  - arrastar um efeito para uma faixa do mixer o põe no fim dos efeitos dela;
  - arrastar para o rack põe no ponto onde soltou, e vale também para a aba de efeitos da janela do instrumento;
  - clique duplo adiciona ao canal selecionado.
- **Slots em miniatura** em cada faixa do mixer: um quadrado por efeito (aceso ligado, apagado desligado, vermelho
  com áudio corrompido). A dica mostra os nomes.
- **Grupos recolhíveis:** a faixa do canal de grupo de uma pasta tem um triângulo (também no menu) que esconde ou
  mostra os canais da pasta. Esconder vale só na tela e não é salvo. Os canais dentro de uma pasta têm uma linha na
  borda esquerda, na cor do canal de grupo.
- `EffectChain` ganhou `insertEffect`, `replaceEffect`, `moveEffect` e `saveEffect`/`loadEffect` públicos. O rack passou
  a desenhar os efeitos sempre na ordem da cadeia.
- Cores: as do tema LMMS, sem mudança no CSS.
- `MixerTest` tem 57 testes (inserir, mover e substituir efeitos; sidechain pelos dois lados).

O que ficou para depois aqui (arrastar faixas para pastas, ordenar canais junto do grupo, slots com nome nas faixas
do mixer) foi feito no passo 12.

### Passo 11: um canal por peça (feito)

- `AudioProcessor::channelGroupName()` dá nome aos grupos de canais (a bateria AVL: Kick, Snare, Hi-Hat, Tom,
  Floor Tom, Cymbals, Percussion).
- `Mixer::setUpChannelPerOutput()` serve para qualquer instrumento com mais de um grupo de saída:
  - o canal da faixa ganha um par por grupo (3/4, 5/6...);
  - os pinos do instrumento mandam o grupo g para o par g + 1 (`PluginInstance::routeOutputGroupsToPairs()`; grupo
    mono vai para os dois lados do par) e nada para o par principal;
  - cada grupo ganha um canal com o nome dele, que recebe o seu par;
  - um canal de grupo, "<faixa> (pieces)", soma as peças e manda para onde o canal da faixa mandava.
  - É um passo de desfazer. Se o canal da faixa volta a ter 2 canais, as saídas voltam todas para o par principal,
    para o instrumento não ficar mudo.
- Interface:
  - botão "Mixer channel per piece" na janela da bateria AVL (com a saída por peça ligada);
  - "Mixer channel per output" no menu da faixa, para qualquer instrumento multi-saída.
- `InstrumentTrack::setInstrumentInstance()` põe um instrumento já criado na faixa (usado nos testes).
- `MixerTest` tem 58 testes.
- Os limites daqui (canais das peças ficavam ao apagar a faixa e não seguiam a pasta) foram resolvidos no passo 12.

### Passo 12: acabamento (feito)

O que as etapas anteriores deixaram pela metade, e a revisão do que estava improvisado.

**Canais feitos para uma faixa** (as peças da bateria e o canal de grupo delas):
- Cada um lembra o canal da faixa para o qual foi feito (`MixerChannel::m_owner`, salvo como `owner`).
- Ficam logo depois do canal da faixa: primeiro o canal de grupo, depois as peças.
- Seguem a faixa quando ela muda de pasta: o canal de grupo das peças manda para onde o canal da faixa manda.
- Saem junto com a faixa.
- Pedir "um canal por peça" de novo não cria outro conjunto.

**Ordem dos canais:**
- "Organizar canais como as faixas", no menu do canal (`Mixer::arrangeChannelsByTracks()`), põe os canais na ordem
  das faixas, como o mixer do REAPER: o canal de cada faixa, os canais feitos para ela, depois os do editor de
  sequências, depois os barramentos, na ordem em que estavam. É um passo de desfazer.
- **Decisão:** a ordem não muda sozinha quando as faixas mudam. No LMMS os canais são numerados e o número aparece na
  faixa; reordenar sem pedir mudaria esses números o tempo todo.
- A tela do mixer acompanha as trocas anunciadas pelo mixer (`channelsSwapped`), então qualquer reordenação aparece
  certa.

**Arrastar faixas pela alça** (`TrackContainer::moveTrackStep()`):
- Uma pasta é arrastada com tudo o que contém.
- Uma pasta recolhida é ultrapassada de uma vez.
- A faixa entra e sai de pastas no caminho: fica no nível da faixa acima do novo lugar, ou dentro dela se for uma
  pasta aberta, como no REAPER.
- Arrastar para o lado (três recuos) põe a faixa na pasta acima ou a tira da pasta.
- A **ordem das faixas** passou a ser guardada no histórico junto com as profundidades (`FolderLayout`, atributo
  `order`). Um arraste inteiro é um passo de desfazer. Ao desfazer, as faixas voltam uma a uma para o lugar, e os
  padrões do editor de sequências seguem as suas faixas.
- "Tirar da pasta" põe a faixa depois do resto da pasta. Antes, as faixas abaixo dela também saíam.

**Histórico:**
- Renomear e mudar a cor de um canal entram no histórico (`Mixer::setChannelName()`, `setChannelColor()`).
- **Decisão:** no canal de uma faixa, renomear ou mudar a cor muda a faixa, e o canal acompanha. Nome e cor da faixa
  não entram no histórico do LMMS. Ao desfazer outra mudança do mixer, os canais das faixas voltam a seguir as
  faixas, para nunca ficarem com nomes diferentes.

**Latência (PDC):**
- **Instrumentos:** cada faixa de instrumento se registra no mixer com a latência do seu instrumento
  (`Mixer::addLatencySource()`). O som das faixas de um canal conta com essa latência, e as outras entradas e o
  resto da mixagem esperam por ele. A latência é atualizada quando o instrumento muda e, nos instrumentos de fluxo
  único, a cada período.
- **Sinal seco:** `Effect::mixDrySignal()` mistura o seco depois do knob W/D, atrasado pela latência do efeito.
  Antes, o host misturava o seco sem atraso, e o seco e o processado saíam desalinhados. LV2 e LADSPA usam essa
  função no lugar da mistura própria.
- **Primeiro período:** os plugins escrevem a latência enquanto processam. Agora um plugin LADSPA ou LV2 com porta de
  latência roda uma vez em silêncio ao ser ativado e é reativado, como o Ardour faz. A latência é conhecida antes do
  primeiro período.

**Interface:**
- **Slots com nome** em cada faixa do mixer:
  - o LED liga e desliga o efeito;
  - clique duplo abre os controles do efeito, ou adiciona um efeito num slot vazio;
  - o botão direito abre o menu do slot.
  - Cabem cinco; com mais, o último mostra "+N".
- **Painel do canal selecionado**, acima dos efeitos: trim, inverter fase, trocar L/R, largura estéreo e pan. Até
  aqui, esses ajustes do passo 2 não tinham interface.
- Cada faixa do mixer ganhou um knob pequeno de pan.
- Cores do tema LMMS (o LED aceso usa o verde dos LCDs).
- O navegador lembra a página pelo nome (`ui/browserpage`), não mais pela posição, que mudou com a página "Efeitos".
  A posição salva por builds anteriores é lida na ordem de então.

**Refeito por estar improvisado:**
- A latência dos instrumentos começou como um aviso a cada período, que sumia depois de 32 períodos sem aviso.
  Virou o registro acima, sem prazo.
- O arraste juntava passos de desfazer desligando o histórico por fora. `changeFolderDepth()` ganhou um parâmetro
  para isso.
- O "canal segue a faixa" marcava a faixa com uma propriedade dinâmica do Qt para não conectar duas vezes. Virou
  conexão única (`Qt::UniqueConnection`) com `Mixer::followTracks()`.
- Os knobs de uma pasta sem canal próprio controlavam modelos de mentira, que não faziam nada. Agora controlam o canal
  que a pasta usa.

`MixerTest` tem 63 testes. Novos: canais por saída pertencem à faixa (lugar, pasta, ordenar, salvar, apagar), faixas
movem com as pastas (inclusive desfazer a ordem), renomear e cor no histórico, latência de faixa, sinal seco atrasado.

**Fora do mixer, continua pendente:** entrada de áudio, armar e monitorar. Os ajustes já existem no canal, mas só
funcionam com a gravação (etapa 3).

### Passo 13: mute e solo só nos canais (feito)

Pedido do usuário: faixa e canal tinham mute e solo cada um, o que era redundante. Fica só o dos canais.

**Solo do mixer refeito** (`Mixer::applySolo()`, no início de cada período):
- **Como no REAPER:** com canais em solo, só eles tocam, junto com o caminho até o master (os canais para onde
  mandam) e as suas fontes (os canais que mandam para eles). Solo numa pasta toca o que ela contém; solo numa faixa
  dentro da pasta passa pelo canal de grupo.
- Vários canais em solo ao mesmo tempo.
- **O solo não mexe mais nos botões de mute.** O solo herdado do LMMS mutava todos os outros canais e restaurava
  depois. Um mute feito durante o solo se perdia, e o projeto era salvo com os mutes falsos.
- **Sidechain:** a fonte de um sidechain de algo que toca continua alimentando o sidechain sem ser ouvida.
- O solo do mixer só funcionava com a janela do mixer aberta (a conexão ficava em `MixerView`), então o render pela
  linha de comando ignorava o solo. Agora está no núcleo.

**Faixas:**
- Os botões M e S das faixas de instrumento, de sample e das pastas (na lista de faixas e nas janelas) são os do
  canal (`TrackChannelControls`), como VOL e PAN.
- **Exceções do mute:**
  - faixa num canal usado por outras faixas: o mute é o da faixa, porque mutar o canal mutaria as outras;
  - mute da faixa com automação: fica na faixa, onde a automação age.
- O solo de uma faixa num canal compartilhado põe em solo o canal por onde ela toca.
- Faixas de sequência e de automação não têm canal: mantêm o mute (liga ou desliga os clipes) e perdem o solo.
- Saíram `Track::toggleSolo()`, o modelo de solo da faixa e a opção "Mutar faixas de automação durante o solo".
- Clipes e o indicador da faixa aparecem mutados quando o canal da faixa está mutado (`Track::isMutedInMix()`).
- **Exportar faixas separadas:**
  - pula as faixas mutadas no canal;
  - tira o solo dos canais enquanto exporta, para nenhuma faixa sair muda, e devolve o solo no fim.

**Projetos de antes:**
- O mute de cada faixa passa para o canal dela.
- Uma faixa em solo põe o seu canal em solo.
- Enquanto uma faixa estava em solo, o LMMS mantinha as outras mutadas. Cada faixa volta ao mute de antes do solo,
  que o projeto guardava (`mutedBeforeSolo`).
- O solo de uma faixa de sequência não tem equivalente e é descartado.

**Corrida entre threads do TripleOscillator e do LB302** (passo 8), achada e corrigida:
- **Causa:** uma nota que terminava num período era solta no próprio `play()`, e soltava junto as suas sub-notas
  (de arpejo, por exemplo) com o quadro da soltura naquele período.
- As notas de um período são renderizadas em paralelo. Uma sub-nota renderizada antes da nota-mãe recebia a soltura
  um período atrasada. A soltura, e tudo depois dela, dependia de qual thread chegava primeiro.
- O defeito é do LMMS. Esta build o mostrava mais porque cada nota agora faz mais trabalho (os pinos em `playNote`),
  o que muda o tempo das threads.
- **Correção:** o motor de áudio solta as notas que terminam no período antes de renderizá-lo
  (`NotePlayHandle::releaseIfEnding()`, em `renderStageInstruments()`).
- **Resultado:** 10 renders em paralelo de `inst-tripleosc` e de `inst-lb302` agora batem com a referência (o LB302
  errava em ~40%).
- O arquivo de referência de `ladspa-halfwet` tinha sido gravado no estado errado da corrida; a build de referência,
  renderizada de novo, bate com esta.
- **Sobra ±1 LSB em algumas dezenas de amostras:** é a ordem em que as notas paralelas são somadas no buffer da faixa
  (soma de ponto flutuante não é associativa), também no LMMS.

`MixerTest` tem 66 testes. Novos: solo silencia os outros sem mexer nos mutes, caminho e fontes, dois solos, sidechain
de canal em solo, solo em pasta, mute e solo de projeto antigo indo para o canal.

### Passo 14: modos analógico e digital (feito)

Pedido do usuário: a mesa tem dois modos. No digital, nada de processamento embutido, só os efeitos com os nomes na
faixa. No analógico (o padrão), cada canal tem os controles clássicos de uma mesa analógica, e os efeitos digitais
viram LEDs.

**Modo:**
- Um só para o mixer inteiro (`Mixer::isAnalog()`), salvo no projeto (atributo `analog`) e com desfazer.
- Projetos de antes abrem no analógico, com as faixas neutras: soam igual.
- No digital, o processamento analógico é desligado, mas os ajustes ficam guardados para quando voltar ao analógico.
- Botões ANALOG / DIGITAL acima do painel do canal selecionado.

**Faixa analógica** (`AnalogChannelStrip`), em cada canal menos o master, entre o ganho de entrada e os efeitos. Na
ordem do sinal:
1. **GAIN:** é o trim do canal (±24 dB), linear.
2. **LC (low cut):** corta abaixo de 80 Hz (24 dB/oitava). Fica **antes** do estágio de ganho, a pedido do usuário,
   para o amp-op ou a válvula não saturarem com graves ruins.
3. **Estágio de ganho:** amp-op ou, com o botão **TUBE**, válvula.
   - **Amp-op**, modelado no 4558, o amp-op de muitas mesas Soundcraft e Mackie:
     - linear até +18 dBFS, satura suavemente até o trilho em +24 dBFS;
     - o trilho negativo é 5% mais baixo, como nos chips reais, o que dá harmônicos pares;
     - slew rate de 1,7 V/µs (o do 4558), que só limita sinais muito altos e muito agudos, como no chip.
     - **Oversampling de 4×** (os `Upsampler`/`Downsampler` do núcleo, os mesmos do SlewDistortion), aplicado só ao
       que o amp-op muda: saída menos entrada, que é zero abaixo do joelho. O sinal limpo não passa pelos filtros do
       oversampling, então abaixo do joelho o áudio sai intocado, bit a bit. Blocos 12 dB abaixo do joelho pulam o
       oversampling.
     - **Decisão:** a primeira versão saturava acima de +6 dBFS. O A/B mostrou que projetos comuns passam disso
       dentro do canal (em ponto flutuante é normal) e mudavam de som. O joelho foi para a folga de uma mesa real.
   - **Válvula (TUBE)**, um estágio pré de 12AX7:
     - usa a curva de transferência que Tim Goetze amostrou de uma simulação SPICE para o CAPS
       (`src/core/r12ax7.h`, copiado sem mudanças; a cópia do CAPS embutido fica como está);
     - ganho unitário para sinais pequenos, a mesma polaridade (a válvula inverte) e sem o offset de polarização;
     - começa a clipar em +12 dBFS e colore um pouco abaixo disso. O GAIN leva a válvula à saturação.
     - O sinal inteiro passa pelo oversampling de 4× e depois por um bloqueador de DC em 5 Hz (a assimetria cria
       offset).
4. **Filtros:** HPF (20 Hz a 1 kHz) e LPF (1 a 20 kHz), 12 dB/oitava, desligados nos extremos.
5. **Dinâmica**, com detecção estéreo ligada:
   - GATE: um knob, o limiar; abre em 1 ms, segura 20 ms e fecha em 100 ms;
   - COMP: um knob, o limiar; razão 4:1, joelho suave de 6 dB (a mesma fórmula do plugin Compressor) e ganho de
     compensação automático, metade do que o compressor tira de um sinal em 0 dBFS.
6. **EQ clássico de 4 bandas**, a pedido do usuário (±15 dB, frequências fixas):
   - HI: prateleira em 12 kHz;
   - HMF: pico em 2,5 kHz;
   - LMF: pico em 400 Hz;
   - LO: prateleira em 80 Hz.
   As bandas médias têm largura de 1,4 oitava.

**Comportamento:**
- Cada seção na posição neutra é pulada, então uma faixa neutra deixa o áudio intocado.
- Depois que a entrada silencia, os filtros ainda tocam o rabo por 50 ms.
- Canal mudo zera o estado da faixa.

**Reaproveitamento:**
- Os filtros e o EQ são as classes do plugin Eq. `EqFilter.h` saiu do plugin e foi para o núcleo (`include/`), e o
  plugin e o mixer usam o mesmo código, incluindo a transição suave entre ajustes ao longo do período. Ganhou só
  `clearHistory()` e os includes de que precisa.
- O oversampling é o do núcleo (`OversamplingHelpers.h`, com a biblioteca hiir, que o núcleo agora também usa). A
  curva da válvula é a do CAPS.
- O gate e o compressor são código próprio, poucas linhas. A lógica do plugin Compressor está presa dentro do efeito
  (modelos, slots, fator de crista, lookahead). Separá-la exigiria reescrever o plugin, com risco de mudar o som
  dele.

**Interface** (cores do tema LMMS):
- **Faixa no modo analógico:** seções INPUT (GAIN, LC, TUBE), FILTER (HPF, LPF), DYN (GATE, COMP) e EQ (HI, HMF,
  LMF, LO), com título e divisória. O botão TUBE aceso brilha laranja, como uma válvula.
- **Knobs de mesa** (`DeskKnob`): capa colorida por seção, como nas Soundcraft e SSL, sobre saia escura, ponteiro
  branco e nome embaixo.
  - Cores: GAIN vermelho, filtros cinza, gate verde-água, compressor amarelo; EQ como na SSL: HI laranja, HMF verde,
    LMF azul, LO marrom.
  - Comportam-se como qualquer knob: arrastar, roda do mouse, valor e menu de automação.
- O master não tem faixa analógica, como numa mesa real, mas guarda o espaço para os faders ficarem alinhados.
- Os efeitos aparecem como LEDs ao lado do fader no analógico e como a lista com nomes no digital.

`MixerTest` tem 73 testes. Novos:
- amp-op só satura acima do joelho, com trilhos assimétricos;
- válvula com ganho unitário para sinais pequenos, assimétrica e limitada;
- amp-op saturado sem aliasing: o 3º harmônico de um seno de 15 kHz, que dobraria para 900 Hz, fica mais de 50 dB
  abaixo da fundamental;
- válvula no canal: unitária para sinal baixo, comprime sinal alto;
- LC, HPF, LPF e as bandas do EQ medidos com seno;
- o modo digital ignora a faixa e guarda os ajustes;
- gate fecha abaixo do limiar; compressor aperta mais o sinal forte;
- modo e faixa salvos.

### Passo 15: master à direita, retornos de efeito e seção master (feito)

Pedido do usuário: nas duas mesas, o master fica à direita, junto com 2 bus de efeitos (para um reverb, por
exemplo) com dois knobs de envio em cada canal, e o master ganha os recursos das mesas de estúdio. Das opções
sugeridas, ele escolheu o compressor de barramento, a seção de monitoração e o medidor de correlação, e pediu VUs
analógicos na monitoração.

**Retornos de efeito** (`Mixer::fxReturns()`):
- **Canais:** dois, FX 1 e FX 2, marcados com `MixerChannel::m_fxReturn` (salvo como `fxreturn`), sempre presentes
  (`Mixer::ensureFxReturns()`).
  - Projetos novos e de antes os ganham ao carregar, depois dos canais das faixas, que ficam com os primeiros
    números.
  - Não podem ser removidos como "sem uso".
- **Envios:** todo canal manda para eles depois do fader, com quantidade 0 de início.
  - **Decisão:** os envios são rotas comuns do mixer. Os knobs FX1 e FX2 de cada faixa são a quantidade dessas
    rotas, então são salvos, entram no desfazer e aceitam automação, sem um segundo sistema de envios.
  - Um envio no zero (sem automação) é pulado na mistura e não custa nada.
  - O roteamento das pastas ignora os envios aos retornos.

**Seção master** (`MasterSection`), nas duas mesas:
- **Compressor de barramento**, no master depois dos efeitos e antes do fader, inspirado no da SSL série G:
  - detecção estéreo ligada, joelho suave estreito (2 dB);
  - razão 2, 4 ou 10; ataque de 0,1 a 30 ms; release de 0,1 a 1,2 s; make-up até +15 dB;
  - **AUTO:** o release vai de 0,1 s em picos curtos a 1,2 s com compressão sustentada (que o compressor mede ao longo
    de 1 s), para voltar rápido dos picos sem "bombear" em material denso;
  - desligado por padrão (botão IN), salvo no projeto, entra no desfazer;
  - medidor de redução de ganho de 0 a 20 dB.
- **Monitoração:** nível, DIM (−20 dB), CUT e MONO. **Decisões:**
  - age só no que vai para as caixas: o mixer não aplica durante a exportação, e os medidores mostram a mixagem, não
    o que se ouve;
  - são estados de escuta: não são salvos no projeto nem entram no desfazer (abrir um projeto com CUT ligado
    confundiria);
  - mudanças em rampa ao longo de um período, sem clique.
- **Medidores:**
  - **VUs analógicos** L e R: face iluminada, escala de −20 a +3 VU proporcional à tensão (como num VU de verdade),
    agulha com a balística de 300 ms (o RMS do master). 0 VU = −18 dBFS RMS, o alinhamento usual entre digital e
    analógico;
  - **correlação** de L e R ao longo de 300 ms: +1 mono, 0 largo, abaixo de 0 eles se cancelam.

**Interface:**
- À direita dos canais: FX 1, FX 2 e o master. O master é uma faixa mais larga, com a seção master (METERS, BUS
  COMP, MONITOR) em cima, no lugar dos controles de envio e da faixa analógica dos outros canais, e o fader embaixo,
  como numa mesa. (Na primeira versão a seção master era um painel separado ao lado da faixa do master; o usuário
  pediu que fosse uma peça só.)
- Os faders se alinham: o mixer dá à área de cima de todas as faixas a altura da mais alta (`MixerView::alignStrips()`),
  nos dois modos.
- Cada faixa tem os knobs FX1 e FX2 (`DeskKnob`, que ganhou arquivos próprios).
- **Canal fantasma:** o botão + virou um canal tracejado depois do último canal; um clique cria um canal no lugar
  dele.

`MixerTest` tem 77 testes. Novos:
- retornos recebem os envios, salvos, e projetos de antes os ganham;
- o compressor aperta a mixagem e informa a redução;
- a correlação segue a fase;
- a monitoração muda só o que se ouve.

### Passo 16: seção estéreo, goniômetro, faders PRE e falha de período silencioso (feito)

Pedidos do usuário: tratamentos mid/side no master (a partir do plugin GMonoBass), com um estudo completo para fazer
do jeito certo; dois faders no master, PRE (L e R, com botão de travar) e POST, mais largos e com régua de dB; o
master sem pan.

**Estudo (as decisões):** M = (L+R)/2, S = (L−R)/2; L = M+S, R = M−S, reconstrução exata.
- **MONO BASS:**
  - **Errado:** S − passa-baixas(S). O passa-baixas IIR atrasa a fase, então a subtração não cancela o grave (com
    corte em 120 Hz sobra ~0 dB a 30 Hz).
  - **Certo:** crossover Linkwitz-Riley de 24 dB/oitava: S' = HP(S) + largura·LP(S). O mid passa pelo mesmo
    crossover, LP(M)+HP(M), um passa-tudo de ganho plano, para que mid e side tenham a mesma fase e a imagem acima do
    corte não "borre".
  - **Estrutura do filtro:** a primeira versão usava o `LinkwitzRiley` do núcleo, uma seção de 4ª ordem na forma
    direta II, e trocava de frequência com dois crossovers e crossfade. O teste de estalo mostrou um transiente na troca
    de frequência (degrau de 0,0123 num seno cuja inclinação natural é 0,0071): a forma direta II não aceita coeficientes
    mudando com o sinal passando.
  - Refeito com filtros de variáveis de estado TPT (Zavalishin), dois Butterworth em série por lado do corte, feitos
    para modulação: a frequência desliza amostra a amostra. O maior degrau caiu para 0,00714, a inclinação do próprio
    seno, sem estalo nenhum.
  - Ligar e desligar faz crossfade de um período entre o processado (com a fase do passa-tudo) e o seco.
  - FREQ de 20 a 300 Hz (padrão 120), BASS (largura do grave) de 0 a 100% (100 = desligado).
- **WIDTH:** é a largura do próprio master, que agora é aplicada aqui, antes do compressor de barramento, e não mais
  depois dele. Muda em rampa ao longo do período.
- **AIR:** prateleira de agudo só no side, em 12 kHz, de −6 a +6 dB (`EqHighShelfFilter`, que suaviza as mudanças).
  Processa um período a mais em 0 dB antes de ser pulado, para não dar degrau.
- **Monitoração SIDE:** o MONO já é o mid, (L+R)/2 nos dois lados, então um botão MID seria duplicado. O SIDE põe
  (L−R)/2 nos dois lados, em fase. MONO e SIDE são exclusivos.
- **Goniômetro:**
  - a thread de áudio grava pares (side, mid) do master num anel lock-free (512 pontos, um a cada dois quadros);
  - a interface desenha a nuvem com o mid para cima, o side para os lados e L e R nas diagonais;
  - mono é uma linha vertical.
- **Ordem no master:** PRE → efeitos de inserção → STEREO (WIDTH, MONO BASS, AIR) → BUS COMP → POST.

**Faders do master:**
- PRE L e PRE R: o nível da mixagem antes de todo o processamento do master, com medidores próprios.
- LINK (ligado por padrão): os dois andam juntos; ao religar, o R vai para onde está o L. O fader que acompanha não
  cria um passo de desfazer próprio.
- POST é o fader do master de antes.
- Faders mais largos (24 e 30 px) e régua de dB entre PRE e POST (`Fader::positionOf()`, a mesma conta das marcas do
  fader).
- O master não tem mais pan: o knob saiu da faixa e do painel e o processamento ignora o pan do master. O pan do master
  só existia nesta build, desde o passo 2; o LMMS nunca teve.

**Falha encontrada pela bateria A/B: período silencioso (corrigida):**
- **Sintoma:** de vez em quando, um período inteiro da saída (256 quadros) saía em silêncio. Aconteceu em cerca de 1
  render a cada 15 dos projetos de teste, depois que os retornos de efeito acrescentaram um nível de dependência (canal
  → FX → master).
- **Investigação:**
  - a instrumentação mostrou que o `masterMix()` saía sem o master processado;
  - logo depois, o master aparecia processado, com as dependências completas.
- **Causa:** o `masterMix()` olhava o estado dos canais um depois do outro e desistia quando nenhum estava na fila ou em
  processamento. O master, olhado primeiro, podia entrar na fila enquanto os outros eram olhados; a varredura não via
  nada em andamento e desistia. O defeito é do LMMS.
- **Correção:** um canal entra na fila enquanto o último canal que manda para ele é processado, e a espera pela fila só
  termina quando todos os jobs enfileirados acabaram, inclusive os enfileirados durante a espera. Então basta uma
  espera, sem varredura.
- **Resultado:** 40 renders seguidos do projeto que falhava, nenhum período silencioso.

`MixerTest` tem 83 testes. Novos:
- largura do master antes do compressor;
- mono bass estreita só o grave e mantém o nível do mid;
- mono bass liga, desliga e muda de frequência sem estalo;
- air sobe só o agudo do side;
- faders PRE travados e soltos;
- monitoração SIDE e goniômetro.

### Passo 17: faixa com volume e pan próprios, MONO e mono bass por canal (feito)

**Mudança de conceito, pedida pelo usuário:** a faixa volta a ter volume e pan próprios, como no REAPER: a faixa é a
**fonte** e o canal é a **mixagem**.
- Os knobs VOL e PAN da faixa (na lista e nas janelas) são os da própria faixa e agem antes do canal, como no LMMS
  original. Deixaram de ser atalhos do fader e do pan do canal, que era a decisão do passo 8.
- `Mixer::setUpTrackChannel()` não move mais o volume da faixa para o trim do canal nem o pan para o pan do canal.
  Efeitos, mute e solo continuam indo para o canal.
- As pastas, que não têm volume e pan próprios, continuam mostrando os do canal de grupo.
- **Consequência boa:** os projetos de teste com volume de faixa diferente de 100%, que no passo 8 mudavam no primeiro
  período, voltaram a bater bit a bit com a build de referência.
- Projetos salvos por esta build entre os passos 8 e 16 já têm o volume no trim do canal e a faixa em 100%; continuam
  soando igual.

**MONO por canal** (menos o master, que tem o MONO na monitoração):
- Soma o canal em mono, (L+R)/2 nos dois lados, depois do trim e antes da faixa analógica e dos efeitos: eles recebem um
  sinal mono, e o pan passa a posicionar a fonte em vez de só balancear L e R.
- Ligar e desligar faz crossfade de um período, sem estalo.
- Está no painel do canal selecionado (nos dois modos) e na seção INPUT da faixa analógica.

**Mono bass por canal:**
- Na seção FILTER da faixa analógica: BASS (largura do side abaixo da frequência, 0 = mono, 100 = desligado) e FREQ.
- **Reaproveitamento:** o mono bass do master saiu para a classe `MonoBass` (crossover Linkwitz-Riley de filtros de
  variáveis de estado TPT, o mid pelo mesmo crossover, crossfade de liga/desliga), usada pelo master e pelas faixas.

`MixerTest` tem 85 testes. Novos: MONO soma e troca sem estalo; mono bass da faixa estreita só o grave e não age no
modo digital. Os testes que verificavam o volume da faixa indo para o trim foram reescritos para a regra nova.

### Passo 18: modelos do estágio de ganho (feito)

Pedido do usuário: o botão TUBE vira um seletor de modelo, com FLAT (sem nada), dois amp-ops (fundo azul) e algumas
válvulas (fundo âmbar), aproveitando o código de válvulas que já existia.

**Modelos** (`AnalogChannelStrip::Drive`):
- **FLAT:** estágio de ganho limpo, nada é processado.
- **4558** (padrão, o comportamento de antes), o amp-op das Soundcraft e Mackie mais acessíveis: joelho em +18 dBFS,
  trilho negativo 5% mais baixo (harmônicos pares), slew lento de 1,7 V/µs.
- **5532** (NE5532), o amp-op das mesas de som limpo (SSL, Neve, Amek, Mackie Onyx): joelho em +21 dBFS, trilhos
  simétricos, slew de 9 V/µs.
  - **Decisão:** a primeira escolha foi o NE5534. O usuário perguntou pelo NE5532, que é a versão dupla (dois
    amp-ops num chip, com compensação interna) e a que as mesas de fato usam. A diferença medível é o slew (9 contra
    13 V/µs).
- **Válvulas:** as três aproximações do 12AX7 que o CAPS embutido já tinha (Tim Goetze, GPL):
  - **12AX7:** a curva amostrada da simulação SPICE (a de antes);
  - **12AX7 V** (vintage): o polinômio original do CAPS, ajustado "de ouvido", o mais rico em harmônicos pares;
  - **12AX7 H** (hot): o polinômio ajustado à curva, mais forte nos ímpares.
  - Todas normalizadas igual: ganho unitário para sinal pequeno, mesma polaridade, sem o offset de polarização,
    começando a clipar em +12 dBFS. Os polinômios clipam onde a inclinação zera, como no CAPS.
  - **Decisão:** não inventar curvas de outras válvulas (12AT7, EF86) sem dados; ficam as três que vêm de modelos
    reais.

**Interface:**
- O botão ocupa a largura da seção INPUT e mostra o nome do modelo.
- Um clique avança para o próximo modelo; o botão direito lista todos, com a descrição.
- Fundo neutro no FLAT, azul nos amp-ops e âmbar nas válvulas.

**Compatibilidade:** salvo como `drive`; projetos com o botão TUBE de antes ligado abrem no 12AX7.

`MixerTest` tem 86 testes:
- amp-ops: joelho e trilhos de cada modelo, o 5532 mais limpo e simétrico;
- válvulas: unitárias, assimétricas, limitadas e diferentes entre si;
- FLAT limpo mesmo muito acima dos trilhos;
- compatibilidade do TUBE antigo.

### Passo 19: mais modelos, LEDs SAT e CLIP, estágio de ganho no master e pot de GAIN (feito)

Pedido do usuário: o 5534 e o TL072 também, outras válvulas, um LED que pisca conforme satura e um de clipping que
acende um pouco antes, o mesmo sistema de saturação no master (sem pot de ganho, depois do fader PRE) e o GAIN
começando mudo, com 0 dB no meio e saturação depois do meio, como numa mesa real.

**Estágio de ganho em classe própria** (`GainStage`, `include/GainStage.h`):
- O código saiu da `AnalogChannelStrip` para ser usado pelos canais e pelo master.
- Os números dos modelos são os mesmos de antes; os novos foram para o fim, então os projetos abrem iguais.
- O seletor oferece os modelos em `GainStage::DisplayOrder`: FLAT, amp-ops, válvulas.

**Modelos novos:**
- **TL072** (entrada JFET, muito equipamento de estúdio): joelho em +19 dBFS, trilhos um pouco mais baixos e levemente
  assimétricos, slew de 13 V/µs.
- **5534** (NE5534, o single): igual ao 5532, com slew de 13 V/µs.
- **12AT7 e 12AU7:** modelo de triodo de Norman Koren com os parâmetros publicados por ele, num estágio de catodo comum
  típico (+250 V, 47k de placa, catodo de 820 Ω e 1,5k com bypass). A curva de transferência do estágio é resolvida
  numericamente (ponto de operação e tensão de placa por bisseção) numa tabela de 4097 pontos, calculada uma vez no
  construtor (thread principal, não no áudio).
  - **Decisão:** no passo 18 ficou "não inventar curvas sem dados"; aqui as curvas vêm de um modelo físico publicado,
    não de ajuste de ouvido.
  - **Correção:** normalizar pelo corte (como o 12AX7) fazia os dois clipar já em ~0 dBFS, porque o lado positivo
    satura antes, quando a grade começa a conduzir. A normalização usa o primeiro ponto de clipagem, qualquer que seja
    o lado. Resultado: parecidos com o 12AX7 em nível normal, e com mais headroom passado o ponto de clipagem
    (entrada -8: 12AX7 -3,2; 12AT7 -5,3; 12AU7 -6,2), coerente com o ganho menor.

**LEDs SAT e CLIP** (abaixo do seletor, na seção INPUT, e no master):
- O estágio mede o pico de cada período antes de processar.
- **SAT:** brilho de 0 a 1.
  - Nos amp-ops, do joelho ao trilho.
  - Nas válvulas, dos 12 dB abaixo do ponto de clipagem até ele.
  - Acende na hora e apaga devagar, como um LED de mesa.
- **CLIP:** vermelho, acende 3 dB antes do limite (o trilho do amp-op, o ponto de clipagem da válvula, ou +24 dBFS no
  FLAT, onde uma mesa clipa) e segura por uns 10 quadros da interface.
- A interface lê os valores com `takeSaturation`/`takeClip` (atômicos, sem trava). Apagado, o LED mostra a cor fraca,
  como a lente de um LED de mesa.

**Pot de GAIN** (`AnalogChannelStrip::m_gainModel`, posição de 0 a 1):
- Todo à esquerda é mudo; no meio é 0 dB; todo à direita é +30 dB.
  - Até o meio: (2p)³, que dá -18 dB a um quarto.
  - Depois do meio: linear em dB.
- Fica antes do LC e do estágio de ganho, com rampa por período (sem estalo).
- O knob mostra o ganho em dB ("-inf" no mudo).
- Antes, o GAIN da faixa era o trim do canal. O trim continua no painel do canal, como ajuste digital limpo, e projetos
  antigos mantêm o trim que tinham.
- Com a música em níveis normais, a saturação aparece da metade para a direita: nas válvulas, a partir de 0 dBFS no
  estágio; nos amp-ops, perto do joelho.

**Master:**
- `MasterSection::m_driveModel`, padrão 4558, e um `GainStage` depois dos faders PRE (que o empurram) e antes dos
  efeitos do master.
- Só no modo analógico, sem pot de ganho.
- Linha DRIVE na seção master (seletor + LEDs), visível só no modo analógico.
- Salvo em `masterstrip` como `drive`.

**Interface:** `DriveButton`, `SaturationLeds` e `GainKnob` foram para `GainStageControls`, usados pelo canal e pelo
master.

**A/B:**
- Os projetos determinísticos batem com a referência s15, com uma ressalva: renders seguidos do mesmo projeto já
  diferiam entre si em ±1 LSB em umas 20 amostras. A ordem de soma de ponto flutuante varia com as threads; isso já
  aparecia entre s23 e s15, antes desta mudança.
- Os plugins aleatórios por natureza variam como sempre.

`MixerTest` tem 90 testes. Novos:
- cada modelo aparece uma vez no seletor;
- medição de SAT e CLIP;
- mapeamento do pot (mudo, 0 dB, +15 dB, +30 dB, -18 dB a um quarto, monotônico) e saturação depois do meio;
- estágio de ganho do master (empurrado pelo PRE, ausente no modo digital, salvo e lido).

Os testes de amp-op e válvula agora cobrem todos os modelos.

### Passo 20: fidelidade da simulação (decisões do usuário, feito)

Decidido pelo usuário, pergunta por pergunta, em 2026-11-12. Implementar na ordem abaixo.

**1. Válvulas com memória**
- Dois modos:
  - **Leve** (padrão): curva estática + filtros em volta.
  - **Realista:** o estágio resolvido a cada amostra.
- Ao escolher uma válvula, aparece uma lâmpada na seção INPUT do canal (e na linha DRIVE do master):
  - clicar alterna leve/realista;
  - a luz de dentro é amarelinha no leve e vermelha no realista;
  - a lâmpada substitui o LED SAT (brilha conforme satura); o CLIP continua.
- O circuito é um estágio de pré: um triodo em catodo comum.
- **Realista:**
  - triodo de Koren com os capacitores de grade, catodo e acoplamento e a capacitância Miller;
  - fonte com filtro RC (sag);
  - oversampling 4x.
- **Leve:** precisa ser mais realista que o de hoje. Tem:
  - Miller: passa-baixa na grade pela capacitância de cada modelo;
  - blocking: deslocamento da polarização depois de picos;
  - sag aproximado;
  - o mesmo esquema de ganho real.
- **Ganho real com compensação:** o GAIN controla a tensão na grade e a saída é atenuada depois, então cada válvula
  satura num ponto diferente do pot.
- **Calibração:** com o GAIN no meio, o 12AX7 começa a colorir com picos de -6 dBFS.
- **12AX7 V e H** viram variantes físicas do 12AX7:
  - V: polarização fria, mais pares;
  - H: polarização quente com mais tensão, mais ímpares.

**Parte 1 feita.** Como ficou:
- **Código:** `TubeStage` (`include/TubeStage.h`).
  - Triodo de Koren para a corrente de placa e de Dempwolf-Zölzer para a corrente de grade.
  - Circuito de catodo comum com Ci 22n, Rgl 1M, grid stopper 68k, Ck 22µ, Co 22n, RL 1M, Rs 10k, Cs 22µ.
- **Válvulas:**

  | Modelo | Placa | Catodo | B+ |
  |---|---|---|---|
  | 12AX7 | 100k | 1,5k | 250 V |
  | 12AX7 V (frio) | 100k | 2,7k | 250 V |
  | 12AX7 H (quente) | 100k | 1k | 330 V |
  | 12AT7 | 47k | 820 | 250 V |
  | 12AU7 | 47k | 1,5k | 250 V |

  - O 12AX7 deixou a tabela SPICE do CAPS (`r12ax7.h` removido) para usar o mesmo modelo físico das outras válvulas.
    O som dele muda um pouco.
- **Modo realista:**
  - análise nodal de 6 nós com capacitores trapezoidais, método DK: a matriz linear é invertida uma vez e o Newton
    roda só nas duas correntes não lineares;
  - custa cerca de 10% de um núcleo por canal estéreo.
- **Modo leve:**
  - tabelas estáticas na reta de carga AC (catodo fixo pelo capacitor);
  - Miller como passa-baixa na grade, com a capacitância de cada modelo;
  - blocking: Ci carregado pela corrente de grade e descarregado por Rgl;
  - sag: a corrente média pelo RC da fonte escala as curvas;
  - custa cerca de 0,6% de um núcleo.
  - Acompanha o circuito resolvido com menos de 1 dB de diferença em ganho e harmônicos (testado).
- **Calibração:** com o GAIN no meio, o 12AX7 começa a clipar em 0 dBFS. Em -6 dBFS o 2º harmônico fica em
  -29 dB. O 12AT7 clipa em +4 dBFS e o 12AU7 em +11 dBFS.
- **LEDs:** o SAT das válvulas cobre de 6 dB abaixo do ponto de clipagem até 6 dB acima. O CLIP acende 3 dB antes
  de +6 dB.
- **Lâmpada:** substitui o SAT quando o modelo é uma válvula.
  - O filamento fica amarelo (LT, leve) ou vermelho (RL, realista) e brilha conforme satura.
  - O clique alterna o modo; a escolha é salva como `realistic`, no canal e no master.
- **Testes:**
  - leve × realista concordam;
  - calibração;
  - V com mais pares, H com mais ímpares;
  - Miller do 12AX7 maior que o do 12AU7;
  - +30 dB de GAIN sem instabilidade;
  - lâmpada salva.

**2. EQ e filtros**
- Referência: Soundcraft/Mackie.
- Médios fixos (2,5 kHz e 400 Hz).
- Q proporcional ao ganho e um pouco de interação entre bandas, como no circuito.
- Biquads com casamento analógico até Nyquist (Vicanek/Orfanidis) no EQ, HPF e LPF.
- LC em 100 Hz, 18 dB/oitava.

**Parte 2 feita.** Como ficou:
- **Código:** `DeskFilters` (`include/DeskFilters.h`).
- **Filtros com casamento analógico** (`Biquad`):
  - segunda ordem pelo método de Martin Vicanek (2016): polos por invariância ao impulso; zeros que casam a
    magnitude em DC, na frequência central e em Nyquist;
  - primeira ordem com casamento exato em três pontos: DC, Nyquist e onde a curva muda mais (o polo também é
    resolvido).
- **Precisão até 20 kHz em 44,1 kHz** (testado contra a resposta analógica):

  | Filtro | Erro máximo |
  |---|---|
  | LPF em 1 kHz | menor que 0,1 dB |
  | Picos | menor que 0,5 dB, perto de Nyquist |
  | Shelf de 12 kHz | menor que 0,4 dB |
  | LPF em 15 kHz | menor que 1 dB |

  Na transformação bilinear de antes, o LPF chegava a -∞ em Nyquist.
- **Mudança de ajuste:** os coeficientes andam linearmente ao longo de um período, sem estalo (`RampedBiquad`).
- **EQ (`DeskEq`):**
  - shelves de primeira ordem (Baxandall), 80 Hz e 12 kHz, com o canto 3 dB antes do ganho total;
  - picos em 400 Hz e 2,5 kHz.
- **Q proporcional:** 2,5 oitavas na metade do ganho em ±3 dB, estreitando até uma oitava em ±15 dB. O Q sai em
  forma fechada da largura.
- **Interação entre bandas:** os reforços ficam em paralelo e os cortes na realimentação,
  H = (1 + Σ reforços) / (1 + Σ cortes), como no circuito ativo dessas mesas.
  - O laço sem atraso dos cortes é resolvido algebricamente.
  - Sozinha, cada banda faz exatamente o que o knob diz. Juntas, bandas vizinhas somam menos que a soma em dB.
  - Uma banda que passa de reforço a corte vai a plano por um período, sem estalo.
- **LC:** 100 Hz, Butterworth de terceira ordem (18 dB/oitava).
- **HPF/LPF:** continuam em 12 dB/oitava, agora com casamento analógico.
- **Testes:**
  - cada filtro contra a resposta analógica;
  - cada banda sozinha;
  - Q proporcional;
  - interação de reforços e de cortes;
  - o processamento bate com a resposta calculada;
  - troca de lado sem salto;
  - LC de terceira ordem.

**3. Amp-op**
- Mantém o joelho suave.
- Sem banda passante dependente do ganho.
- Acrescenta:
  - a distorção de crossover do 4558;
  - a inversão de fase do TL072 passado o limite de modo comum;
  - o limite de corrente de saída.

**Parte 3 feita.** Como ficou:
- **Crossover do 4558:** decisão do usuário: sempre, fiel ao chip.
  - O estágio de saída classe AB tem uma zona morta de 40 mV, -d·tanh(x/d).
  - A realimentação reduz o erro pelo ganho de malha: ganho DC de 106 dB e GBW de 3 MHz, ou seja, a derivada do
    erro dividida por 2π·GBW.
  - Resultado: 3º harmônico em -88 dB a 1 kHz e -75 dB a 8 kHz, a -6 dBFS. Cresce com a frequência, como no
    chip.
  - Silêncio continua silêncio.
  - Os outros amp-ops seguem bit a bit transparentes abaixo do joelho.
  - Consequência: um canal analógico neutro com 4558 deixa de ser idêntico ao digital, bem abaixo do audível.
- **Inversão do TL072:** abaixo do limite de modo comum (V- + 4 V, ou -11 V, ou +22,6 dBFS de pico negativo), a
  saída salta para o trilho positivo, a sobrecarga "feia" característica dele.
- **Limite de corrente:** as saídas acionam 600 Ω.
  - O trilho efetivo é o menor entre o trilho e corrente × 600 Ω.
  - O 4558 (20 mA) cede um pouco antes do trilho.
  - O TL072 (15 mA) cede bem antes: não foi feito para 600 Ω.
  - O 5532/5534 (26 mA), feito para 600 Ω, não muda.
  - SAT e CLIP usam o trilho efetivo.
- **Testes:**
  - trilho efetivo por modelo;
  - inversão do TL072;
  - crossover entre -80 e -120 dB e crescendo com a frequência;
  - 5532 bit a bit transparente;
  - silêncio intacto.
  - A medição de harmônicos agora usa janela Blackman-Harris: a retangular vazava o fundamental em -65 dB.
  - Os testes de roteamento, que comparam níveis exatos, rodam a mesa digital. Os da mesa analógica a ligam
    explicitamente.

**4. Dinâmica**
- Compressor de canal no estilo SSL:
  - na faixa, o knob COMP e um botão FAST;
  - no painel do canal, ratio, release e PEAK/RMS.
- Gate/expander no estilo SSL:
  - na faixa, o knob GATE;
  - no painel, range, release, hold e modo expander/gate.

**Parte 4 feita.** Os números vêm do guia oficial do módulo SSL E Series Dynamics.
- **Decisões do usuário durante a parte:**
  - o knob RATIO vai para a faixa e liga o compressor (1:1 = fora);
  - o RANGE vai para a faixa e liga o gate (0 = fora), como no módulo SSL.
- **Código:** `ChannelCompressor` e `ChannelGate` (`include/DeskDynamics.h`).
- **Compressor:**
  - detector RMS verdadeiro (5 ms), ou de pico;
  - ratio de 1:1 a ∞ (limitador);
  - joelho "over-easy" de 10 dB, ou duro;
  - threshold de +4 a -26 dBu;
  - make-up automático: um sinal no nível nominal (0 VU, -18 dBFS RMS) sai onde entrou, quanto quer que comprima;
  - ataque de 30 ms por 20 dB de redução, ou 3 ms com FAST: uma inclinação em dB, que depende do programa;
  - release de 0,1 a 4 s, como constante de tempo em dB (curva logarítmica). Com LIN REL vira inclinação
    constante, igual à da curva logarítmica a partir de 20 dB, e o threshold sobe 6 dB (como no guia).
- **Gate/expander:**
  - gate de 20:1, ou expander de 2:1 (EXP);
  - range de 0 a 40 dB;
  - threshold de -30 a +10 dBu;
  - histerese que cresce quando o threshold desce: 2 dB em +10 dBu, 8 dB em -30 dBu (o guia só diz que é
    variável; os valores são nossos);
  - abertura linear de 1,5 ms por 40 dB, ou 100 µs com FAST;
  - hold de 0 a 4 s (decisão do usuário: o módulo SSL não tem);
  - release de 0,1 a 4 s;
  - chave de pico com 10 ms de queda, para não fechar nas passagens por zero.
- **Níveis:** dBu → dBFS por +4 dBu = -18 dBFS RMS.
- **Faixa:** COMP, RATIO (∞ no fim, "1:1 (off)" no início), FAST, GATE e RANGE.
- **Painel do canal** (só no modo analógico, fora do master):
  - compressor: REL, Peak, Hard knee, Lin. release;
  - gate: REL, HOLD, Expander, Fast.
- **Compatibilidade:** projetos com a dinâmica de um knob de antes viram um compressor de 4:1 e um gate de 40 dB nos
  mesmos thresholds (dBFS convertidos para dBu).
- **Testes:**
  - make-up mantém o nível nominal;
  - 4:1 dá 3 dB para cada 12 dB;
  - limitador;
  - knee;
  - LIN REL +6 dB;
  - FAST mais de 5× mais rápido;
  - gate fecha no range;
  - histerese;
  - expander 2:1;
  - hold;
  - RATIO e RANGE ligam;
  - compatibilidade.

**5. Imperfeições**
- Sem ruído de fundo e sem crosstalk.
- **Tolerância por canal:** variações pequenas e fixas por canal.
  - Ligada por padrão em projetos novos, com um botão global perto de ANALOG/DIGITAL.
  - Projetos antigos abrem com ela desligada.
- **Transformador de saída no master,** estilo Neve: saturação de grave e queda leve acima de 20 kHz. Só com um botão
  para ativar.

**Parte 5 feita.**
- **Tolerância por canal** (`ChannelTolerance`, `include/DeskTolerance.h`): desvios fixos por canal, sorteados do
  número do canal (SplitMix64, igual em toda plataforma). O canal 3 é sempre o mesmo canal 3, como numa mesa
  física. Faixas, uniformes:
  - ganho: ±0,1 dB;
  - frequências do LC, HPF, LPF e das bandas do EQ: ±1%;
  - ganhos do EQ: ±2%;
  - ponto de saturação do estágio de ganho: ±3% (a entrada é escalada e a saída desfeita).
- **Contrato com quem mixa:** o canal 3 tem o mesmo caráter em todo projeto e em toda versão do programa, para
  que a pessoa aprenda a sua mesa ("o canal 3 é um pouco mais brilhante"). Pedido do usuário.
  - Cada tolerância sai do seu próprio fluxo: um hash SplitMix64 da semente fixa, do número do canal e do número
    fixo da tolerância (`Part`), só com aritmética inteira, igual em toda plataforma.
  - Uma tolerância nova ganha um número novo e não muda as outras.
  - O teste `channelTolerancesAreFrozen` trava os valores exatos de alguns canais.
  - O caráter segue o número (a posição) do canal, como numa mesa física: um canal movido de lugar assume o
    caráter da nova posição.
- **Botão TOL** ao lado de ANALOG/DIGITAL, só no modo analógico, salvo como `tolerance` na mesa.
  - Projetos sem o atributo abrem com TOL desligado e soam igual.
  - Projetos novos (com ou sem template) abrem com TOL ligado, a não ser que o template tenha escolhido.
- **Transformador de saída no master** (`OutputTransformer`), estilo Neve, depois do fader do master. Só no modo
  analógico, com o botão XFMR na seção DRIVE, desligado por padrão e salvo como `transformer`. Modelo físico:
  - a fonte aciona a indutância de magnetização do núcleo; o fluxo é a integral da tensão;
  - corrente de magnetização = (φ + p³/φs²)/Lm, em que p segue o fluxo com uma zona morta: o operador "play", a
    histerese (coercitividade de 2% do fluxo de saturação);
  - resolvido implicitamente a cada amostra (Newton).
- **Comportamento do transformador:**
  - canto grave em 3 Hz;
  - satura no grave alto: 3º harmônico em -44 dB (0,6%) a 30 Hz perto do fundo de escala, cai com a frequência ao
    cubo;
  - não faz nada nos médios (menos de -90 dB em 1 kHz);
  - -0,46 dB em 20 kHz (indutância de dispersão, primeira ordem em 60 kHz).
  - Ao ligar de novo, o fluxo recomeça do zero, sem baque.
- **Testes:**
  - tolerâncias fixas, dentro das faixas e diferentes entre canais;
  - dois canais iguais saem levemente diferentes com TOL e iguais sem;
  - projetos antigos sem TOL;
  - transformador: harmônicos no grave, médios limpos, resposta em 20 Hz e 20 kHz;
  - XFMR só no modo analógico.

- **Correção encontrada pela bateria A/B:** projetos sem elemento `<mixer>` (como os de teste) não passam por
  `Mixer::loadSettings`, então herdavam o padrão do construtor e abriam com TOL ligado. Agora o padrão é desligado:
  `Song::loadProject` zera o TOL antes de carregar, e só o projeto novo o liga.
- **A/B depois do passo 20:** os projetos antigos, que abrem no modo analógico com 4558, diferem da referência só
  pelo crossover do 4558, decidido como sempre ligado. Isso dá até 15 LSB (cerca de -67 dBFS de pico) nas bordas
  das ondas, em poucos % das amostras, porque há dois 4558 no caminho (canal e master).

O passo 20 está completo. `MixerTest` tem 104 testes.

### Passo 21: mute e solo voltam para as faixas (feito)

Pedido do usuário: o mute da faixa muta a faixa, não o canal. Logo depois: o solo da faixa também volta. Isso
reverte a parte das faixas do passo 13, mas não o mecanismo antigo do LMMS. Como no passo 17 (a faixa tem volume e
pan próprios), a faixa é a fonte e o canal é a mixagem: cada um tem o seu M e o seu S.

**Faixa (`Track::isSilenced()`, `TrackContainer::updateSilence()`):**
- **Mute:** silencia a fonte da faixa, antes do canal. O mute de uma pasta silencia tudo que ela contém.
- **Solo, como no REAPER:**
  - com faixas em solo, só elas tocam, mais tudo que as pastas em solo contêm; as faixas de automação continuam
    agindo;
  - várias faixas podem ficar em solo ao mesmo tempo;
  - o solo não mexe nos botões de mute (o LMMS solava mutando as outras faixas e restaurando depois).
- A silência é recalculada quando muda um mute ou um solo e quando faixas entram, saem ou mudam de lugar.
- O áudio consulta uma flag atômica: o barramento da faixa, as notas, os samples, as faixas de sequência e a
  automação vinda delas.
- Faixas de sequência e de automação têm solo de novo.
- Clipes e o indicador da faixa aparecem mutados quando a faixa está silenciada ou o canal dela está mutado.

**Canal:** o M e o S do mixer continuam como no passo 13, no canal (`Mixer::applySolo()`). Os dois solos convivem.

**Projetos:**
- O solo da faixa volta a ser salvo (`solo`).
- **Projetos do LMMS:** a faixa em solo continua em solo na faixa, e as outras recuperam o mute de antes do solo
  (`mutedBeforeSolo`, só quando o projeto tem esse atributo).
- Projetos salvos por esta build entre os passos 13 e 20 têm os mutes e solos das faixas nos canais, e continuam
  assim.

**Exportar faixas separadas:** tira também os solos das faixas enquanto exporta, e os devolve no fim.

**Verificado no render:** duas faixas iguais dão -5,97 dBFS; com solo numa, ou mute numa, dão -10,68 dBFS, o nível
de uma só.

`MixerTest` tem 106 testes. Novos:
- mute e solo da faixa sem tocar no canal;
- dois solos;
- automação sempre tocando;
- solo e mute de pasta;
- tirar a faixa da pasta em solo a silencia;
- salvar e carregar;
- solo de projeto do LMMS.

### Passo 22: GAIN com 0 dB em um quarto, TRIM e LCUT na faixa, PAN de mesa (feito)

Pedidos do usuário:
- **GAIN:** o 0 dB vai para um quarto do curso (antes ficava no meio).
  - Mudo à esquerda; abaixo de um quarto, a curva cúbica de antes, com -18 dB no meio do caminho até o 0 dB.
  - Acima de um quarto, linear em dB até +30 dB à direita: a maior parte do curso dá ganho.
  - O GAIN aumenta o nível de verdade: a saturação vem junto, e é o fader que traz o volume de volta.
  - Salvo como `gainpot`. Projetos com o pot de antes (`gain`, 0 dB no meio, passos 19 a 21) são convertidos para
    a posição com o mesmo ganho (`positionFromMiddleUnity()`).
- **INPUT:** o knob TRIM vai para o lugar do LC, ao lado do GAIN. O TRIM limpo continua também no painel do canal.
- **LCUT:** o botão do low cut passa a se chamar LCUT, no tamanho do MONO, ao lado dele.
- **PAN:** no modo analógico, o PAN da faixa vira um knob de mesa (`DeskKnob`). No digital continua o knob de antes.

`MixerTest` tem 106 testes: o do pot foi refeito para 0 dB em um quarto, com +15 dB em 62,5%, e testa a conversão
dos projetos de antes.

### Passo 23: modelos de compressor e modos do gate (feito)

Pedido do usuário:
- um alternador de modelo de compressor, como o das válvulas;
- FAST, NORM e SLOW;
- um gate mais simples, com modos fixos;
- antes de implementar, uma pesquisa profunda num `.md`, melhorada, e um plano.

Isso está em `docs/pesquisa-compressores.md` (síntese revisada mais 5 dossiês) e `docs/plano-compressores.md`.

**Compressor** (`ChannelCompressor`, `include/DeskDynamics.h`):
- **Modelos:** SSL (canal E), API 2500, dbx 160, 1176, LA-2A, Fairchild 670, Gates Sta-Level, Altec 436 e RCA
  BA-6A. Um motor só, montado por uma tabela de especificação; não são nove classes.
- **Os controles da faixa são posições**, e cada modelo as lê como o aparelho:
  - COMP: o threshold, ou a entrada empurrando um threshold fixo;
  - RATIO: o ratio, seus passos, COMP/LIMIT, ou a curva do vari-mu. O mínimo desliga, bit a bit;
  - FAST, NORM e SLOW: o ataque dentro do que o modelo permite;
  - RELEASE (painel): o release, ou a chave de time constant do Fairchild. O meio é o ajuste de fábrica.
  - Os knobs mostram o valor nas unidades do modelo, e as dicas dizem o que fazem nele.
- **Motores:**
  - **Feed-forward** (SSL, dbx): detector RMS, joelho, balística. O dbx não tem ataque nem release: os tempos do
    manual saem do detector RMS de 36 ms.
  - **Feedback em log** (API em OLD, 1176): o detector lê a saída, e o laço é resolvido em cada amostra (regula
    falsi). O ratio e o joelho saem do laço. O API tem o THRUST no detector; o 1176 tem os passos 4/8/12/20 e o ALL.
  - **Vari-mu:** o retificador carrega a rede RC do aparelho (no Fairchild, os valores de cada posição TC, com os
    ramos em série das posições 5 e 6). A tensão de controle passa pela lei do tubo, e o laço é resolvido em cada
    amostra. No Fairchild, a lei de corte remoto faz o ratio crescer com o nível.
  - **Óptico** (LA-2A): o painel EL integra o sinal. A célula tem um estado rápido, um lento e a memória: a
    soltura é mais lenta depois de compressão longa. COMP ~4:1, LIMIT ~11:1, os dois saindo do laço.
- **Cor e resposta:**
  - a distorção do elemento de ganho é oversampled 4x e calibrada pelas especificações;
  - os harmônicos ímpares dominam nos push-pull;
  - os filtros de banda são ajustados à especificação de cada aparelho; no RCA, a resposta segue a redução de ganho;
  - os transformadores usam o modelo do XFMR, agora com parâmetros por aparelho.
- **Make-up automático em todos:** um sinal no nível nominal sai onde entrou.

**Gate** (`ChannelGate`): GATE e RANGE continuam nos knobs, e o modo traz o resto.
- **Modos:** STD, KICK, SNARE, TOMS, VOX, GTR, BASS, NOISE e EXP.
- **O que cada modo define:**
  - gate de 20:1 ou expander de 2:1;
  - ataque normal ou rápido, hold, release e histerese;
  - filtros da chave: o KICK não abre com o chimbal.

**Faixa:** DYN com COMP, RATIO, o modelo (colorido pela família: VCA azul, FET verde, óptico amarelo, vari-mu
âmbar), FAST/NORM/SLOW, GATE, RANGE e o modo do gate. O painel do canal ficou só com o RELEASE e o que ele faz no
modelo; no dbx 160, ele fica inativo.

**Projetos de antes:**
- A SSL dos passos 20 a 22 vira o modelo SSL nas mesmas posições: threshold, ratio, FAST e release.
- A dinâmica de um knob vira 4:1 e gate de 40 dB.
- EXP vira o modo EXP.
- Peak, hard knee e lin. release somem: o modelo define esses comportamentos.

**Verificado contra a pesquisa:**

| Modelo | Medido |
|---|---|
| SSL | Ataque de 20 dB em ~30 ms (NORM) e ~3 ms (FAST); 4:1 |
| dbx 160 | Ataque de 63% em 8 a 30 ms para 10 dB acima; release de 80 a 170 dB/s (doc: 120); 3º harmônico da ondulação |
| API | Passos de ratio; com THRUST, o grave é menos comprimido |
| 1176 | 4:1 e 20:1 do laço; THD baixo em 1 kHz, maior no grave e no ALL |
| LA-2A | Metade da redução sai em menos de 0,2 s; soltura mais lenta depois de compressão longa; LIMIT mais plano que COMP; THD < 0,75% |
| Fairchild | Ratio crescendo com o nível; TC1 mais rápido que TC4; THD < 1%, com o 3º acima do 2º |
| Gates | Perto de 3,3:1 |
| Altec | Ratio de ~2:1 a ~4:1 |
| RCA | ~10:1; o Dual solta mais devagar depois de picos sustentados |

**Custo por canal estéreo:**

| Modelo | Custo |
|---|---|
| SSL e dbx | 0,05 a 0,07% de um núcleo |
| API, 1176 e Gates | 1,2 a 1,6% |
| LA-2A, Fairchild e Altec | 1,9 a 2,3% |
| RCA | 3,2% (duas redes RC resolvidas por amostra) |

O RCA passa da meta de ~2% e fica como candidato a otimização.

`MixerTest` tem 114 testes.

### Passo 24: knob de mono bass invertido (feito)

Pedido do usuário: o knob de mono bass, no canal e no master, começa em 0% (desligado, o padrão) e vai até 100%
(todo mono). Antes, 100% era desligado e 0% era todo mono.

- **Modelo:** é a "quantidade de mono" (`m_monoBassModel`). O processamento usa a largura 100 − valor.
- **Salvo como** `monobassamount`.
- **Projetos de antes** (`monobass` no canal, `basswidth` no master): o valor é invertido (novo = 100 − antigo), e as
  automações do knob também.
  - Os nós da automação guardam o valor do knob, então são invertidos depois que o projeto resolve as automações
    (`Mixer::invertLegacyAutomation()` e `convertLegacyAutomation()`, chamado em `Song::loadProject`).
- Testado: o padrão 0, a conversão do canal e do master, e a automação invertida.

### Correção encontrada pela bateria A/B

O SC4 (LADSPA) saía mudo desde a fase 3: o projeto tinha os pinos no formato antigo (um `pins` dentro de outro), que
passou a ser lido como vazio. O efeito ficava sem entrada, dormia e zerava o canal. `AudioPortsModel::loadSettings`
volta a ler esse formato (teste em `AudioPortsTest`).

## Para depois (anotado pelo usuário)

- **Inverter o knob de mono bass:** feito (passo 24).
