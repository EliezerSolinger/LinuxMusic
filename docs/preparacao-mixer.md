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

### Correção encontrada pela bateria A/B

O SC4 (LADSPA) saía mudo desde a fase 3: o projeto tinha os pinos no formato antigo (um `pins` dentro de outro), que
passou a ser lido como vazio. O efeito ficava sem entrada, dormia e zerava o canal. `AudioPortsModel::loadSettings`
volta a ler esse formato (teste em `AudioPortsTest`).
