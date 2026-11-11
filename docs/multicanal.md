# Passo 6 da preparação do mixer: áudio multicanal, pin connector e sidechain

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
