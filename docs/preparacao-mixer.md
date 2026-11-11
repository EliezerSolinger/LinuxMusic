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

### Interface nova do mixer
- Slots de efeito visíveis em cada canal.
- **Arrastar e soltar** plugins do navegador direto num slot ou num canal.
- **Menu de slot:** trocar plugin, mover e salvar/carregar preset.

### Qualidade
- **Testes automatizados** de mixagem e roteamento são escritos **antes** de mexer no núcleo.
- O branch **ainda não** é enviado ao GitHub, então não há CI de Linux por enquanto. Tudo continua sendo compilado só
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
9. Interface nova do mixer: slots por canal, arrastar e soltar, menu de slot.

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
- Os controles novos ainda **não aparecem na interface**; isso fica para o passo 9.

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
