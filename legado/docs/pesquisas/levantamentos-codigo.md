# Levantamentos no código para os pedidos de 17/11

Escrito em 17/11/2026, lendo o código. Cada item aponta o que foi achado e o que falta medir. Os pedidos estão em
`docs/pendencias.md`, seção "Pedidos de 17/11".

## Mallets: "falta o STK" e a queda

- **Por que avisa que falta o STK.**
  - O caminho salvo na configuração ativa (`build/.linuxmusicrc.xml`) é `/usr/share/stk/rawwaves/`, o último dos
    alternativos do `ConfigManager`, que não existe no Mac.
  - O STK do Homebrew fica em `/opt/homebrew/share/stk/rawwaves`, e esse caminho não estava na lista.
  - O instalador do Mac e o do Windows copiam os arquivos para `<dados>/stk/rawwaves`, mas no Mac o `ConfigManager`
    também não olhava ali.
  - **Corrigido** (ainda não compilado nem gravado em commit), e passa a valer em toda inicialização: o caminho
    salvo só é usado se tiver o `sinewave.raw`. Senão vale o primeiro, nesta ordem, que tiver os arquivos:
    1. a pasta de dados do programa;
    2. o pacote;
    3. a pasta onde a compilação achou o STK (`LMMS_STK_RAWWAVE_ROOT`, no `lmmsconfig.h`);
    4. o Homebrew;
    5. `/usr/local`;
    6. `/usr`.
  - Se nenhuma tiver, isso vai para o log de erros.
- **A queda: reproduzida e consertada.** Reproduzida com o projeto de um Mallets e a pasta do STK inexistente:
  SIGSEGV em `InstrumentTrackWindow::resizeEvent`.
  - A vista do Mallets abria uma `QMessageBox` modal no próprio construtor.
  - A caixa roda os eventos enquanto a janela do instrumento está pela metade.
  - `updateInstrumentView()` tinha acabado de apagar a vista antiga sem zerar o ponteiro, e o `resizeEvent` a usava.
  - Conserto:
    - o ponteiro é zerado antes de criar a vista nova;
    - as vistas da janela começam nulas e `adjustTabSize` ignora as que ainda não existem;
    - o Mallets mostra o aviso dentro da própria vista, sem caixa modal.
  - Varredura: nenhum outro plugin abre caixa modal ao ser construído (o LOMM e o AmpAndIr só abrem por ação do
    usuário).
- **O STK é necessário?** O Mallets usa do STK três instrumentos (ModalBar, TubeBell, BandedWG) e os arquivos de onda
  (`rawwaves`). A licença do STK é parecida com a MIT. A pesquisa decide entre trazer só essas classes e os arquivos
  para dentro do plugin (sem dependência externa) ou manter o pacote.

## Kicker: o clique a cada nota

- O Kicker não tem controle de ataque. O botão "Click" (padrão 0,4) começa a senoide com a fase deslocada
  (`clickModel × 0,25` → `SweepOsc`), então a primeira amostra salta de 0 para cerca de 0,59. Esse é o clique.
- Proposta: um controle de ataque com mínimo maior que 0 (por exemplo 0,1 ms) e padrão de cerca de 1 ms, aplicado
  depois do salto de fase, para que o "Click" continue servindo a quem quer o estalo.
- Projetos antigos: decidir se recebem o ataque mínimo (muda o som deles) ou ficam como estavam pela versão do
  preset (`KICKER_PRESET_VERSION`).

## Configurações que pedem reinício

- O áudio e o MIDI já reabrem ao salvar (`SetupDialog::accept`, `reopenAudioDevice`/`reopenMidiClient`): marcar o
  áudio como alterado depende dos sinais dos widgets da aba de áudio. Falta reproduzir o caso do usuário (trocar o
  backend e fechar a tela) para ver se o "alterado" deixa de ser marcado ou se a janela foi fechada sem salvar.
- Pedem reinício hoje (`showRestartWarning`):
  - dicas (tooltips);
  - botões compactos das faixas;
  - uma janela de instrumento só;
  - barra lateral à direita;
  - salvar sem compressão (mmpz).
  Cada um pode ser aplicado na hora com um sinal para as vistas.
- O resto é lido quando é usado: idioma e tema precisam de reinício de verdade, e então a tela deve oferecer
  reiniciar.

## Rack de efeitos: tamanho das janelas

- Fundos fixos medidos (largura × altura em pixels):
  - Delay 300×208;
  - DynamicsProcessor 224×319;
  - FrequencyShifter 288×360;
  - GranularPitchShifter 305×180;
  - LOMM 400×256;
  - MultitapEcho 245×300;
  - SlewDistortion 638×271;
  - WaveShaper 224×274;
  - Eq 500×500.
- Os outros montam o layout no código. Falta medir o tamanho real de cada janela com o programa aberto: um modo que
  abre cada efeito e grava o `sizeHint()`.

## Categorias dos instrumentos

- As categorias estão em `src/core/PluginCategories.cpp` (`s_instrumentCategories`). O Vibed (`vibedstrings`) está em
  "Physical modeling" junto com o Mallets: passa para "Synthesizers".
- O navegador de plugins tem um nível só de categoria. As subcategorias pedem um segundo nível na árvore.
- Proposta de subcategorias de sintetizador (para o usuário confirmar):
  - **Analógico virtual / subtrativo:** TripleOscillator, LB302, Monstro;
  - **Tabela de onda / aditivo:** Watsyn, Organic, BitInvader;
  - **Por expressão:** Xpressive;
  - **Completo / FM e outros:** ZynAddSubFX;
  - **Cordas:** Vibed.
- "Physical modeling" fica só com o Mallets. Se o Mallets deixar de depender do STK, isso continua igual.

## Baterias eletrônicas: o que já temos

- `data/samples/drumsynth/` tem kits do DrumSynth (arquivos `.ds`: parâmetros que o nosso `DrumSynth` transforma em
  som), um por máquina:
  - CR-78 (11 peças), CR-8000 (11), Linn (16), TR-606 (7), TR-77 (8), TR-808 (15), TR-909 (13);
  - Electro (14), R&B (15), Jorgensohn (10), Ferraro (8), Magnetboy (27);
  - também há pastas por tipo de peça (`misc_*`, `effects`).
- `data/samples/drums/` tem 53 amostras `.ogg` soltas (bumbos, caixas, chimbais, palmas).
- O plugin novo pode tratar cada pasta de máquina como um kit:
  - ler os `.ds` (renderizados uma vez, ao carregar);
  - mapear as peças no padrão General MIDI (bumbo 36, caixa 38, palma 39, chimbal fechado 42, aberto 46 etc.)
    pelo nome do arquivo;
  - trocar de kit com uma imagem de cada máquina, como no AVL Drums.
- As imagens precisam ser próprias (desenhadas por nós), sem marcas registradas: os nomes das máquinas são de
  fabricantes.
