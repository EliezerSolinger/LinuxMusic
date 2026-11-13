# Gravação: como usar

O que já funciona (12/11/2026). O plano completo e o estado de cada parte estão em `plano-gravacao.md`.

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
- **Desvio:** se ainda houver diferença (conversores da interface, por exemplo), ajuste `recording/latencyoffset`, em frames, no arquivo de configuração.

## O que ainda não existe

Está no plano, mas ainda falta:
- **mapa de tempo:** vários BPMs na música, com saltos e rampas;
- **time-stretch** renderizado quando o BPM muda;
- **desfazer:** a gravação ainda desfaz faixa por faixa, não numa entrada só;
- **edição:**
  - fades automáticos curtos nas bordas dos takes;
  - ganho por item;
- **faixa de gravação nova:** com cabeçalho de duas linhas e lanes visíveis;
- **leitura da mídia:** em streaming do disco (hoje o take vai inteiro para a memória);
- **calibração automática de latência** por loopback;
- **marcas de xrun** no take: o xrun é contado, mas não aparece no item.
