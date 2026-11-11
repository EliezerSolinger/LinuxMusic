# Etapa 3: sistema de gravação no estilo REAPER

## Objetivo

Implementar gravação de áudio ao vivo de verdade, com o modelo de trabalho do REAPER: você arma as faixas, escolhe
a entrada de cada uma, aperta gravar, e cada faixa armada grava direto em arquivo no disco enquanto o resto do
projeto toca.

Este sistema é construído do zero sobre os canais da etapa 2. Ele **não** reaproveita a gravação herdada do branch
`feature/recording-stage-one` do LMMS (veja [estado-atual.md](estado-atual.md)).

## Um novo tipo de faixa: a faixa de gravação

A "Sample Track" herdada do LMMS não representa o que a gravação precisa. Ela foi feita para tocar loops e samples
soltos, não para editar áudio gravado. Por isso a gravação terá um **tipo de faixa novo, a faixa de gravação**
(faixa de áudio), pensada para isso desde o começo:

- **Vários clipes de áudio por faixa, em qualquer posição**, inclusive sobrepostos.
- **Crossfade automático** quando dois clipes se sobrepõem, com curva ajustável.
- **Fade in e fade out** em cada clipe, com curva ajustável. O PR #7900 do LMMS, já mesclado, fez isso para a
  Sample Track e pode servir de base.
- **Edição não destrutiva:** cortar, aparar as bordas, mover o conteúdo dentro do clipe (slip edit) e ganho por clipe,
  sem alterar o arquivo gravado no disco.
- **Takes em lanes:** cada passada de gravação vira um take na mesma faixa, e dá para escolher ou montar o melhor
  (comping).
- **Entrada, armar e monitorar** ficam na própria faixa e no canal do mixer ligado a ela (etapa 2).

A Sample Track atual continua existindo para loops e samples simples, pelo menos até a faixa de gravação cobrir o que
ela faz.

## Funcionalidades esperadas

### Essenciais (primeira versão)

- **Armar faixas para gravação** (record arm), uma ou várias ao mesmo tempo.
- **Entrada por faixa:** escolher a entrada da interface de áudio (mono ou estéreo) que cada faixa grava.
- **Monitoração de entrada:** desligada, ligada ou automática (só quando armada e gravando).
- **Gravação direta em disco:** o áudio vai para arquivos WAV (ou FLAC) na pasta de mídia do projeto, escritos por
  uma thread de disco separada. A thread de áudio nunca espera pelo disco.
- **Forma de onda desenhada ao vivo** enquanto grava.
- **Compensação de latência:** alinhar o áudio gravado com o que está tocando, usando a latência informada pelo
  driver, com ajuste manual.
- **Medidores de entrada** em cada faixa armada.
- **Metrônomo e contagem antes de gravar** (pre-roll / count-in).

### Depois da primeira versão

- **Punch in/out** por seleção de tempo.
- **Gravação em loop com takes:** cada volta do loop gera um take, e os takes ficam em lanes da faixa para escolher
  o melhor.
- **Modos de gravação** como os do REAPER: entrada de áudio, saída da faixa (resample) e MIDI.
- **Recuperação após falha:** arquivos gravados não se perdem se o programa fechar no meio da gravação.

## Requisitos técnicos

- **Linux primeiro:**
  - JACK/PipeWire é o backend principal para gravação com baixa latência.
  - ALSA e PulseAudio também precisam funcionar.
- **Tempo real seguro:**
  - Na thread de áudio, nada de alocação de memória, mutex ou acesso a disco.
  - A captura passa por ring buffers lock-free até a thread de disco.
- **Formato de projeto:** pode mudar livremente para referenciar os arquivos gravados, guardar os takes e as
  configurações de entrada e monitoração de cada faixa.
- **Entrada só quando necessária:** hoje os drivers SDL e PortAudio capturam a entrada (o microfone) o tempo todo,
  mesmo sem nenhuma faixa armada. O ring buffer de entrada enche e o log é inundado de avisos de "input ring buffer
  overflow", escritos da thread de áudio. A captura deve abrir só quando houver faixa armada ou monitorando, e os
  avisos não podem ser impressos da thread de tempo real.

## Critério de conclusão

- É possível gravar várias faixas ao mesmo tempo a partir de uma interface de áudio no Linux.
- Há monitoração de entrada, e o áudio gravado fica alinhado ao projeto.
- Os arquivos ficam no disco e o projeto reabre com tudo no lugar.
