# Gravação: como usar

O que já funciona (13/11/2026). O plano completo e o estado de cada parte estão em `plano-gravacao.md`.

## Configurar o áudio

Em **Configurações → Áudio**, escolha o driver.

### Mac: CoreAudio

O padrão no Mac. Escolha:
- **Dispositivo de saída:** por exemplo, os alto-falantes ou a interface de áudio.
- **Dispositivo de entrada:**
  - o microfone ou a interface;
  - **[Mesmo da saída]** para uma interface que faz as duas coisas;
  - **[Nenhum]** para não gravar.

Detalhes:
- Quando entrada e saída são aparelhos diferentes, o programa junta os dois num dispositivo agregado privado, com a entrada acompanhando o relógio da saída.
- O macOS pede permissão para o microfone na primeira vez que uma faixa precisa da entrada, não ao abrir o programa.

### Linux: ALSA

- **Saída:** de preferência um dispositivo `hw:` da placa, por exemplo `hw:CARD=USB,DEV=0`.
  - É exclusivo e trabalha no formato próprio da placa, sem conversão.
  - O programa pede a placa ao PipeWire/PulseAudio pela DBus e a devolve ao fechar.
- **Entrada:** **[Mesma da saída]** usa a captura da mesma placa, no mesmo relógio.
- **Plugs:** os dispositivos `default`, `pipewire` e `pulse` também funcionam, compartilhados com outros programas.

### Linux: JACK

Em **Entradas**, escolha quantas portas de entrada criar (até 32) e a que cada uma se conecta.

### Tamanho do buffer

É o tamanho do buffer das configurações. Quanto menor, menor a latência.

## Faixas de áudio

A faixa de gravação é a **faixa de áudio** (botão "Adicionar faixa de áudio" no Editor de Arranjo, com o ícone de
onda e ponto vermelho). A faixa de sample continua para tocar samples curtos na grade e as batidas do Editor de
Sequências, e não grava. Projetos antigos com gravações em faixas de sample abrem com elas como faixas de áudio.

O cabeçalho tem duas linhas:
- **Em cima:** o nome, a atividade, **R** (arma a faixa; vermelho quando armada) e o **botão de entrada**, que
  mostra `-` (sem entrada), `1`, `3` (mono) ou `1/2` (estéreo). O menu dele traz:
  - a escolha da entrada, mono ou em pares estéreo, com os nomes que o dispositivo dá;
  - a **monitoração por software**, que passa a entrada pelos efeitos da faixa e pela mesa:
    - **Desligada.**
    - **Ligada:** sempre.
    - **Auto:** com a faixa armada, ouve com a música parada ou gravando, mas não só tocando ("tape auto").
- **Embaixo:** o canal da mesa, o **medidor de nível** da entrada (mostra a entrada mesmo sem monitorar; verde,
  amarelo a partir de −12 dBFS, vermelho a partir de −3 dBFS), VOL e PAN.

### Itens

- Todo item tem fades de 5 ms nas pontas, para não entrar nem sair de repente. Onde dois itens da mesma lane se
  sobrepõem, eles fazem crossfade de potência igual sobre a sobreposição, sozinhos.
- No topo do item: as alças dos fades nos cantos e a do ganho ao longo dele (Shift: mais fino). Cada arraste é um
  passo de desfazer.
- Arrastar um item para cima ou para baixo o leva para outra faixa do mesmo tipo.
- No menu do item: a memória (Automático, RAM ou disco, como no Ableton), zerar ganho e fades, e com lanes,
  "Ouvir esta lane" e "Comp".

### Lanes

Com **Configurações → Áudio → Gravação → Lanes** ligado (padrão), cada passada vai para uma lane nova no topo, a que
toca; as de baixo ficam esmaecidas. Para montar o melhor take (comping): corte o item da lane de baixo no trecho
desejado e use "Comp: usar este item na lane que toca". Desligado, a gravação é simples: o take novo substitui o que
cobre, e no loop só a última passada fica.

## Faixas de instrumento (MIDI)

- **R:** arma a faixa. As notas tocadas nela, pela entrada MIDI da faixa, vão para a linha do tempo.
- **Botão direito no R:**
  - **Overdub** (padrão): as notas novas se juntam às que já estão lá.
  - **Replace:** ao parar, as notas antigas do trecho gravado saem.

## Gravar

O botão **Gravar** da barra de transporte faz o seguinte:
- **Com a música parada:** começa a tocar a partir do cursor e grava as faixas armadas.
- **Com a música tocando:** começa a gravar ali mesmo (punch in). Apertar de novo sai da gravação (punch out), e a música continua tocando.
- **Parar:** encerra.

Ao encerrar:
- cada take vira um clipe na faixa;
- os arquivos vão para a pasta `<projeto>_media`, ao lado do projeto;
- se o projeto ainda não foi salvo, vão para `recordings` na pasta de trabalho;
- o formato é WAV float de 32 bits.

Sem nenhuma faixa armada, o botão grava MIDI no clipe aberto no Piano Roll, como antes.

O menu do botão Gravar (a setinha) tem:
- **Pré-roll:** a música começa 1, 2 ou 4 compassos antes do cursor, e o take começa no cursor.
- **Count-in:** o metrônomo toca durante o pré-roll.

### Loop e takes

- **Gravação em loop:** ligue o loop da linha do tempo e grave.
  - Cada volta completa vira um take do mesmo clipe, e a última fica ativa.
  - Uma volta com menos de 90% do loop (a que foi parada no meio) é descartada.
- **Escolher o take:** pelo menu do clipe, em **Takes**. O nome do clipe mostra, por exemplo, "take 2/3".
- **Comping:** corte o clipe em partes e escolha um take para cada parte. As partes guardam todos os takes.

## Alinhamento e latência

- O take cai no lugar em que foi tocado, com precisão de uma amostra.
  - A conta usa as latências que o driver informa.
  - Os testes provam isso com um loopback falso, para qualquer latência e tamanho de callback.
- **Desvio:** se ainda houver diferença (conversores da interface, por exemplo), meça em **Configurações → Áudio →
  Gravação → Medir...**: ligue um cabo de uma saída a uma entrada, e o programa toca um pulso, encontra onde ele
  volta e guarda o ajuste.

## Quedas (xruns)

- Quando o dispositivo falha (xrun), o ponto fica marcado em vermelho no take, e o fim da gravação avisa quantas
  vezes aconteceu.
- O mesmo vale quando o programa está ocupado mudando o projeto (carregando outro, por exemplo) por mais de meio
  período: aquele período sai em silêncio, a música espera, e a queda fica marcada no take.

## Mapa de tempo e fórmula de compasso

Na faixa **TEMPO**, acima da régua do Arranjo:
- **Duplo clique** ou **botão direito → Mudar o tempo aqui:** um marcador com o BPM a partir dali.
- **Rampa até o próximo marcador** (no menu do marcador): o tempo vai mudando até o do marcador seguinte.
- **Fórmula de compasso...** (no menu do marcador) ou **Mudar a fórmula de compasso aqui...** (num lugar vazio): a
  fórmula a partir dali, por exemplo 3/4 ou 7/8. Um compasso começa no marcador. No marcador do início, é a fórmula
  da música (a do mostrador COMPASSO).

A régua, a grade, o encaixe, o metrônomo, o pré-roll, o mostrador de posição, os plugins (Carla e VST) e os
arquivos MIDI exportados e importados seguem os compassos do mapa. O Piano Roll e o Editor de Automação mostram o
clipe na fórmula do lugar onde ele começa. Os takes gravados guardam o BPM em que foram gravados e acompanham as
mudanças de tempo (time-stretch).

## Se o programa fechar sozinho

- **Projeto:** o programa salva um instantâneo do projeto 2 segundos depois de cada mudança (no máximo a cada 30
  segundos enquanto você mexe sem parar), também tocando e gravando. Na próxima abertura, ele oferece recuperar.
  Uma gravação em andamento também volta: os takes são completados a partir dos arquivos, até o último momento
  gravado.
- **Plugin:** se o programa caiu enquanto um plugin trabalhava, o aviso da próxima abertura diz qual foi. Esse
  plugin não é mais carregado até você permitir: os projetos o abrem "não carregado", com os ajustes guardados (e
  salvos como estavam). Em **Configurações → Desempenho → Plugins**, a lista dos suspeitos tem **Carregar de novo**.

## Trocar de dispositivo sem reiniciar

Ao salvar as configurações, o dispositivo de áudio e o driver MIDI reabrem com o que mudou, sem reiniciar o
programa (não durante uma gravação). As entradas MIDI das faixas continuam ligadas aos mesmos aparelhos, onde o
driver novo os tiver; os que faltarem são avisados.

## O que ainda não existe

- **Linux:** o ALSA e o JACK ainda precisam ser testados numa máquina Linux de verdade.
- **Mac:** a entrada do microfone ainda precisa ser testada no aparelho (o pedido de permissão do macOS precisa de
  alguém para aceitar).
