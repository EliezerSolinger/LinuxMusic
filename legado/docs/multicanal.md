# Passo 6 da preparação do mixer: nova API de plugins, áudio multicanal, pin connector e sidechain

> **Decisão final (11/11/2026): caminho 1.** Continuar o branch de trabalho do autor do #7459,
> `messmerd/pin-connector-with-audio-buffer`, que reconstrói o pin connector sobre o `AudioBuffer` **e** começa a
> nova API de plugins do LMMS ([#8275](https://github.com/LMMS/lmms/issues/8275)), e terminar tudo. O trabalho fica no
> branch `multicanal`; o `linuxmusic` só recebe quando estiver completo e testado. As seções abaixo da linha
> "Plano original" descrevem a ideia anterior (portar o PR de 2025) e ficam como referência.

## O que a nova API de plugins (#8275) entrega

Proposta de Dalton Messmer, baseada no CLAP:
- separação clara entre host e plugin, permitindo plugins de terceiros fora do código do app;
- suporte correto a CLAP e VST3;
- estados do plugin (ativo, processando…) e regras de quais métodos rodam em qual thread, verificadas em builds de
  depuração. Isso acaba com os mutex no processamento;
- fila de eventos lock-free (MIDI e outros) nos dois sentidos;
- sistema de extensões entre host e plugin;
- `PluginId`, `PluginDescriptor` e `PluginEntry` no lugar de `Plugin::Descriptor::SubPluginFeatures::Key`, com vários
  tipos de plugin na mesma biblioteca;
- menos dependência de Qt nos plugins;
- novo processamento dos instrumentos (#8294);
- base para um gerenciador de plugins, um banco de presets e um novo sistema de plugins remotos.

## Fases (caminho 1)

Ordem decidida em 11/11/2026: a fase 4 vem antes dos passos 7 a 10 do mixer ([preparacao-mixer.md](preparacao-mixer.md)).

| Fase | Conteúdo |
|---|---|
| 1 ✅ | Fazer o branch do autor compilar sobre o nosso código e terminar a migração dele para o `AudioBuffer`. Trazer de volta as melhorias que o `master` recebeu depois (detecção de áudio corrompido, auto-quit). Todos os testes passando. |
| 2 ✅ | Pin connector completo: todos os plugins nativos, LADSPA, LV2, VST, ZynAddSubFx e Carla; janela do pin connector. Detalhes e verificação em [api-plugins-mapa.md](api-plugins-mapa.md). |
| 3 ✅ | Canais de faixa e de mixer além do estéreo, envios com pares de canais e sidechain de ponta a ponta, com testes de mixer. Ver "Fase 3 pronta" abaixo. |
| 3b ✅ | Bateria AVL (kits do x42 avldrums: Black Pearl, Red Zeppelin, Blonde Bop, Blonde Bop Hot Rod e Buskman's Holiday) como instrumento nativo sobre o FluidSynth, em versões estéreo e multi-saída (9 canais). Os SF2 ficam no repositório; as amostras são CC-BY-SA 3.0, então vão com o README de atribuição e não podem ser modificadas sem renomear o kit. |
| 4 | Nova API de plugins (#8275): `Host`/`PluginInstance`, estados e regras de thread, fila de eventos lock-free, extensões, `PluginId`/`PluginDescriptor`/`PluginEntry`, cabeçalhos públicos `include/lmms/plugin`, separação host/plugin e novo processamento de instrumentos. |
| 5 | Hosts de CLAP e VST3. |
| 6 | Novo sistema de plugins remotos, gerenciador de plugins e banco de presets. |
| 7 | Medição de desempenho (custo por período do mixer e da cadeia de efeitos) antes e depois, e verificação final com renders de demos. |

## Fase 3 pronta (11/11/2026)

- **Canais da trilha no canal do mixer:** número configurável, em pares (menu do canal → "Track channels").
  - O par principal (1/2) passa por trim, efeitos, seção estéreo, pan e fader.
  - Os pares auxiliares (3/4, 5/6…) só passam pelos efeitos, conforme os pinos, e não chegam sozinhos à saída do
    canal.
- **Envios com pares:** cada envio leva um par do remetente para um par do destino.
  - Do par principal sai o sinal antes ou depois do fader; de um par auxiliar, o sinal depois dos efeitos.
  - O envio para um par auxiliar é um **envio de sidechain** (menu do canal → "Sidechain send from…", que cria o
    par 3/4 se for preciso).
- **Pinos:** a entrada de sidechain de um plugin (o segundo grupo de canais de entrada) é ligada aos canais 3/4
  assim que o canal os tem. Pinos salvos no projeto prevalecem.
- **Instrumentos multi-saída:** a trilha de instrumento tem os mesmos canais do seu canal do mixer.
  - O par principal segue o caminho normal da trilha.
  - Os pares auxiliares vão direto aos pares auxiliares do canal do mixer, de onde podem ser enviados a outros
    canais (por exemplo, cada peça de uma bateria para um canal).
- **Formato do projeto:**
  - `<mixerchannel channels="4">`;
  - `<send sourcepair="1" destinationpair="1">`;
  - um único elemento `<pins>` por plugin.
- **Testes:** 41 no `MixerTest` e 30 no `AudioPortsTest`, incluindo um compressor falso abafado por um envio de
  sidechain.
- **Pendências para a nova interface do mixer** (etapa 2): mostrar os pares e os envios de sidechain no próprio
  canal, rotular os canais dos pinos e escolher o par de destino de cada envio.

## Fase 3b pronta: AVL Drums (11/11/2026)

- **Plugin:** instrumento nativo `avldrums` (categoria Drums) sobre o FluidSynth do sistema, com os 5 kits em
  `data/avldrums`.
  - Os SF2 vieram sem modificação do avldrums.lv2 (commit 9389f16), com o README de atribuição e o `SOURCE.txt`.
- **Comportamento:** mesmo mapa de notas e mesma divisão de saídas do avldrums.lv2.
  - Modo estéreo, ou uma saída por peça: 9 canais em 7 grupos (bumbo, caixa, chimbal, tom e surdo mono; pratos e
    percussão estéreo).
  - O Buskman's Holiday só tem estéreo, como no original.
- **Pinos:** por padrão, todos os grupos vão para o par principal. Para separar as peças, os pinos levam cada
  grupo a um par auxiliar e os envios do mixer levam cada par a um canal.
- **Tela:** seletor de kit, chave de multi-saída e o kit desenhado, com as peças acendendo nas batidas.
- **Verificação:** os 5 kits renderizados nos dois modos, com as batidas no tempo. Teste da regra de grupos no
  `AudioPortsTest`.

### Variação ocasional herdada

Renders repetidos às vezes diferem em cerca de 0,0033 no release das notas ou em caudas de efeitos. Isso acontece
também na build de referência, portanto não é regressão. A corrida dos denormais já foi corrigida; o restante fica
para investigar na fase 7.

---

## Plano original

Decisão (11/11/2026): reimplementar **inteiro** o LMMS PR
[#7459](https://github.com/LMMS/lmms/pull/7459) (plugins multicanal e pin connector), de Dalton Messmer, sobre o
código atual, e completar o que ele deixou para depois: **faixas e canais de mixer com mais de dois canais**,
necessários para o sidechain.

## O que o #7459 faz

- **Plugins com qualquer número de entradas e saídas.** Cada efeito e instrumento declara em tempo de compilação o
  formato que quer (`AudioPortsSettings`):
  - número de canais de entrada e de saída (fixo ou dinâmico);
  - planar ou intercalado;
  - processamento no próprio buffer (*in place*) ou com entrada e saída separadas.

  A assinatura de `processImpl` sai disso. Exemplo: `processImpl(InterleavedBufferView<float, 2> inOut)` para um
  efeito estéreo comum.
- **Pin connector (como no REAPER):** uma matriz por plugin (`AudioPortsModel`) diz quais canais da faixa entram em
  quais entradas do plugin e quais saídas do plugin voltam para quais canais da faixa. Ela é salva no projeto e
  editada por uma janela própria (`PinConnector`).
- **Roteador (`AudioPorts::Router`):** copia os canais da faixa para os buffers do plugin conforme a matriz, e de
  volta, com um atalho sem cópia para o caso comum (L→L, R→R).
- **Plugins remotos** (VST via `RemoteVstPlugin`, ZynAddSubFx): os buffers compartilhados passam a ter o número de
  canais do plugin (`RemotePluginAudioPorts`).
- **Testes:** `AudioPortsTest` (cerca de 1.270 linhas).

O que o #7459 **não** faz: a faixa continua com 2 canais (`m_totalTrackChannels = DEFAULT_CHANNELS`, com um "TODO:
full routing"). Sem canais extras na faixa, uma entrada de sidechain do plugin não tem de onde receber sinal.

## Diferença em relação ao código atual

- O #7459 foi escrito sobre o `AudioBus` (pares de canais **intercalados**). Hoje o LMMS e o LinuxMusic usam o
  `AudioBuffer` (canais **planares**, organizados em grupos, com rastreamento de silêncio). O `AudioBuffer` é o
  sucessor do `AudioBus`, vindo das partes do #7459 que já entraram no LMMS (#7945, #8016).
- Portanto as peças do PR são **portadas para o `AudioBuffer`**, não copiadas.

## Desenho final

1. **Canais da faixa e do canal do mixer ("track channels"):** cada `AudioBuffer` de faixa e de canal do mixer tem N
   canais em grupos de pares. O grupo 0 é o principal (L/R). Os grupos seguintes são pares auxiliares (3/4, 5/6…),
   como os *track channels* do REAPER. O número de canais é configurável por canal do mixer.
2. **Pinos:** cada efeito e instrumento tem um `AudioPortsModel` que liga os canais da faixa às entradas e saídas do
   plugin. O padrão é L/R → entradas 1/2 → L/R. Uma entrada de sidechain do plugin vem ligada por padrão aos canais
   3/4.
3. **Envios do mixer com canais de origem e destino:** um envio leva um par de canais da origem para um par de canais
   do destino. Um **envio de sidechain** é um envio para o par 3/4 do canal que tem o compressor. O padrão continua
   sendo principal → principal.
4. **Mixer processando N canais:** soma, trim, efeitos, estéreo, pan e fader atuam no par principal. Os pares
   auxiliares passam pelos efeitos (pelos pinos), mas não vão para a saída do canal, a menos que um pino de saída os
   ligue ao par principal.

## Fases de implementação

Cada fase termina compilando, com todos os testes passando e com um commit próprio.

| Fase | Conteúdo |
|---|---|
| A | Tipos e configurações (`AudioPortsSettings`, `LmmsPolyfill`), `AudioPortsModel` e a parte dele em `AudioPortsTest` |
| B | Roteador e buffers dos plugins (`AudioPorts`) sobre o `AudioBuffer`, com testes |
| C | Nova API de processamento (`AudioPlugin`, `PluginAudioPorts`) em `Effect`, `Instrument`, `EffectChain` e nos play handles |
| D | Adaptação de todos os plugins nativos (efeitos e instrumentos) |
| E | LADSPA, LV2, VST, ZynAddSubFx e Carla (inclui `RemotePluginAudioPorts`) |
| F | Pin connector na interface |
| G | Além do PR: canais de faixa e de mixer além do estéreo, envios com pares de canais e sidechain de ponta a ponta, com testes de mixer |
| H | Verificação final: renders de projetos demo, testes e documentação |

## Créditos

O desenho e grande parte do código vêm do PR #7459 de Dalton Messmer (messmerd), sob GPL-2.0-or-later. A autoria é
mantida nos cabeçalhos dos arquivos portados e nas mensagens de commit.
