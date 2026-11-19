# Trabalho de UI que já existe no LMMS (levantamento de 11/11/2026)

Levantamento feito antes da etapa 2 para não começar do zero. Fontes: issues e PRs do repositório `LMMS/lmms` e o
histórico do `master`.

## 1. Conceito de redesign oficial: a referência visual

> **Pivô de conceito (11/11/2026):** o mixer e o sistema do LinuxMusic vão ficar **diferentes** do esquema de design
> sugerido pelo LMMS (o conceito de redesign #1911 e os mockups dele). O projeto já pivotou para um conceito próprio:
> trilhas organizadas em **hierarquia** (pastas e canais de grupo), mixer e gerenciamento de plugins no estilo do
> FL Studio, gravação no estilo do REAPER, e os canais extras do núcleo multicanal escondidos como encanamento
> interno. O redesign do LMMS continua servindo só como referência de layout geral (janela única, painéis
> encaixados); ele não define o mixer nem a organização das trilhas. Detalhes em
> [preparacao-mixer.md](preparacao-mixer.md).


- **[#1911 "LMMS redesign concept"](https://github.com/LMMS/lmms/issues/1911)**, de budislav. Aberto desde 2015,
  com 236 comentários. É uma interface de **janela única**, sem janelas flutuantes, com painéis encaixados.
  - Mockups: <https://www.behance.net/gallery/194917259/LMMS-Redesign-Concept>
  - Os mantenedores aceitaram esse conceito como a direção certa, mas nunca o implementaram por inteiro. O LMMS foi
    só se aproximando dele aos poucos.
- **[#4398](https://github.com/LMMS/lmms/issues/4398)** (2018) discutiu o caminho para isso. A conclusão foi evoluir
  sobre o Qt Widgets atual, peça por peça, e não reescrever. A ideia era criar um *framework* de painéis, para que
  plugins e editores encaixem em painéis padronizados em vez de cada um desenhar sua própria janela.
- **[#6393](https://github.com/LMMS/lmms/issues/6393)** (2022, fechado) propôs um roadmap de UI por versão:
  1. Remover os botões de arquivo da toolbar e melhorar o suporte a temas.
  2. Tirar o osciloscópio da toolbar.
  3. Implementar a nova toolbar (#5261) e o novo rack de controladores (#2944).
  4. Criar uma barra lateral direita com propriedades, rack de efeitos e controladores.
  5. Por fim, a janela única do #1911.

## 2. PRs abertos que podem ser aproveitados

| PR | O que faz | Interesse para o LinuxMusic |
|---|---|---|
| [#5261 Modernize the toolbar](https://github.com/LMMS/lmms/pull/5261) | Toolbar nova no estilo do #1911 | Alto: ponto de partida visual da janela principal (92 comentários, precisa de rework) |
| [#2944 Redesign of the Controller Rack](https://github.com/LMMS/lmms/pull/2944) | Controladores dentro do rack, menos janelas flutuantes | Médio |
| [#7878 Make Resizing Tracks more Natural](https://github.com/LMMS/lmms/pull/7878) | Redimensionar faixas arrastando a borda, sem precisar do Shift | Alto: usabilidade básica, ativo em 10/2026 |
| [#7900 Add Sample Clip Fading](https://github.com/LMMS/lmms/pull/7900) | Fade in/out nos clipes de áudio | Alto: essencial para editar áudio gravado (etapa 3) |
| [#7849 Drag files to/from external apps](https://github.com/LMMS/lmms/pull/7849) | Arrastar arquivos de e para outros programas | Médio |
| [#8177 Sync Playback Between Editors](https://github.com/LMMS/lmms/pull/8177) | Sincroniza a reprodução entre os editores | Médio |
| [#7894 Add clip content shifting](https://github.com/LMMS/lmms/pull/7894) | Deslocar o conteúdo dentro do clipe | Médio |

## 3. Mixer, canais e roteamento: base da etapa 2

| PR / issue | O que faz | Interesse |
|---|---|---|
| [#7459 Add support for multi-channel plugins](https://github.com/LMMS/lmms/pull/7459) | Plugins com várias entradas e saídas e o **"Plugin Pin Connector", copiado do REAPER** (messmerd, ativo em 09/2026) | Alto: multicanal e roteamento de pinos (útil para entradas de gravação), mas a interface de plugins deve seguir o FL Studio, não o REAPER |
| [#8317 Decouple tracks from the Mixer](https://github.com/LMMS/lmms/pull/8317) | Refatoração que desacopla faixas e mixer via signals/slots | Alto: facilita criar o channel strip por faixa |
| [#6407 Automatic adding and linking of mixer strips](https://github.com/LMMS/lmms/pull/6407) / [#1215](https://github.com/LMMS/lmms/issues/1215) | Cria e liga automaticamente um canal do mixer para cada faixa nova | Alto: no estilo REAPER, cada faixa já é um canal (rascunho de 2022/2023) |
| [#7714 Multiple Mixer Channel Selection](https://github.com/LMMS/lmms/pull/7714) | Selecionar vários canais e agir em todos | Médio |
| [#8346 Scroll mixer with arrow keys](https://github.com/LMMS/lmms/pull/8346) | Navegação no mixer pelo teclado | Baixo |
| [#1471 Better Sample Track support](https://github.com/LMMS/lmms/issues/1471) | Lista consolidada do que falta nas faixas de áudio | Referência de requisitos |

## 4. O que já está no código (herdado do `master` do LMMS)

Melhorias de UI que já temos e não precisamos refazer:

- **Janelas destacáveis** (#3532, 02/2026).
- **Faders com escala em dB** (#7636) e faders escaláveis com marca de 0 dB (#7045).
- **Botões de mute/solo novos** (#7708).
- Ícones migrando para **SVG**, com escala correta (#7769, #8123, #8128).
- Tema Classic atualizado (#7799).
- Navegação arrastando o workspace (#7595).
- **Suporte a Qt6** (#7339). O LMMS já gera o build de macOS e Windows ARM com Qt6 (#8544), e o build de Linux com
  Qt6 está em [#8118](https://github.com/LMMS/lmms/pull/8118).

## Conclusões para a etapa 2

1. **Adotar o #1911 como referência visual** (janela única, painéis encaixados) e seguir o caminho decidido no #4398:
   evoluir o Qt Widgets aos poucos, sem reescrever do zero.
2. **Migrar para Qt6 no começo da etapa 2**, antes de redesenhar. Assim, a UI nova não é feita em cima do Qt5.
3. **Trazer primeiro os PRs de base do mixer:** #8317 (desacoplar), #6407 (um canal por faixa) e #7459 (pin connector
   e multicanal). Juntos, eles aproximam o modelo de canais do REAPER e preparam a entrada de áudio da etapa 3.
4. **Trazer os PRs de usabilidade baratos:** #7878 (redimensionar faixas) e #7900 (fades em clipes).
5. **Usar o #5261 como base da toolbar nova**, revisando o código, porque ele tem conflitos antigos.

Cada PR deve ser trazido como merge separado (`git fetch upstream pull/<n>/head`), compilado e testado antes do próximo.

## Situação dos PRs no LinuxMusic (11/11/2026)

| PR | Situação |
|---|---|
| #8118 Qt6 no Linux | Mesclado. O Qt6 é o padrão (`WANT_QT6=ON`). |
| #8317 Desacoplar faixas e mixer | Mesclado. |
| #7878 Redimensionar faixas | Mesclado. |
| #7900 Fades em clipes de sample | Mesclado (conflitos resolvidos à mão em `ClipView` e `SamplePlayHandle`). |
| #7459 Plugins multicanal / pin connector | Adiado. O autor está levando o PR ao LMMS em partes (`AudioBuffer` #7945, buffer views #8016), e o branch do PR ficou desatualizado em relação a essas partes. Será reimplementado sobre o código atual num passo próprio, com testes de áudio. |
| #6407 Canal do mixer automático por faixa | Velho demais para mesclar (feito sobre o antigo `MixerLine`). Será reimplementado no redesign do mixer. |
| #5261 Toolbar moderna | Velho demais para mesclar (de 2020, ainda usa o antigo `FxMixer`). Será reimplementado como referência visual. |
| #2944 Rack de controladores | Velho demais para mesclar (de 2019). Será reimplementado no painel da direita. |

## Redesign: o que já foi feito

- **Janela única com abas** (base do #1911):
  - em cima, Song Editor e Pattern Editor;
  - embaixo, painel recolhível com Mixer, Piano Roll, Automation Editor e "Instruments & Effects";
  - à direita, painel recolhível com Controladores e Notas.
- **Instrumentos e efeitos abrem em abas** dentro de "Instruments & Effects" (uma aba por janela, com botão de fechar),
  em vez de janelas flutuantes.
- Classe nova `DockPanel` (`include/DockPanel.h`): faz `parentWidget()->show()` selecionar a aba certa, para que o
  código antigo que mostrava subjanelas continue funcionando.
- Ferramenta de desenvolvimento: `LINUXMUSIC_SCREENSHOT=<arquivo.png>` salva uma imagem da janela e fecha o app;
  `LINUXMUSIC_SCREENSHOT_ACTION=instrument` abre o primeiro instrumento antes da captura.

- **Painel do navegador como no redesign** (11/11/2026):
  - a barra lateral de botões verticais virou um painel com a aba "Browser", uma fileira de botões pequenos com ícone e
    o nome da seção abaixo;
  - "Minha pasta" e "Raiz/Volumes" viraram uma aba só, "Arquivos";
  - os filtros de conteúdo ficam numa linha;
  - mostrar/ocultar fica no menu Exibir (Ctrl+8), e o painel lembra a última aba;
  - as cores continuam as do tema do LMMS; só o layout segue o design.

Ainda em janelas flutuantes: VST (Vestige/VstEffect), parâmetros do Carla, diálogos de controladores e janelas pequenas
como o MeterDialog.
