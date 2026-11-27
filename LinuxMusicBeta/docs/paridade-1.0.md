# Paridade com o legado para a 1.0

Auditoria de 26/11 do que falta no LinuxMusicBeta para a 1.0, feita só lendo o código (nenhuma compilação, nenhum teste).
Entrada: `funcionalidades-legado.md` (448 ids; 42 são obsoletos e ficam de fora, restam **406**: as de status core, 1.0 e as
"aberto" cuja decisão (P-xx) já foi tomada), `decisoes-autonomas.md` e `implementacao.md`.

**Nota sobre o grau de confiança.** "feito" quer dizer que há código no núcleo e, onde a funcionalidade tem tela, na GUI,
mas **nada disto foi rodado nesta auditoria e muito do código nunca foi rodado por quem o escreveu** (vários agentes
empurraram sem testar; há commits que dizem "written without running its tests"; na pausa de 20/11 havia 19 testes
falhando). Então todo "feito" aqui é **"feito (não testado)"**. O que a auditoria mediu é presença e ligação (código
chamado por alguém), não correção. A leitura foi feita por quatro leitores em paralelo (um por grupo de seções), com
amostragem pelo autor; os trechos "falta" vêm de ler o código, mas pode haver um caso isolado de leitura errada.

Estados: **feito** (núcleo e GUI onde há tela), **parcial** (a coluna "O que falta" diz o quê), **falta** (nada encontrado).
Um campo de preferência sem leitor, uma ação só no `lm-port-fake`, um backend não selecionável ou um painel que não chama
a ação contam como parcial.

## Resumo por seção

| Seção | Área | Itens | feito | parcial | falta |
|---|---|---:|---:|---:|---:|
| F-JAN | Casca do programa, menus e transporte | 32 | 18 | 12 | 2 |
| F-IJN | Janela do instrumento | 20 | 13 | 6 | 1 |
| F-NOT | Editor de notas | 22 | 19 | 3 | 0 |
| F-PAS | Gravação por passos | 1 | 0 | 1 | 0 |
| F-ARR | Arranjo | 14 | 7 | 5 | 2 |
| F-REG | Régua, marcadores e andamentos | 13 | 13 | 0 | 0 |
| F-PAD | Padrões | 2 | 1 | 0 | 1 |
| F-AUT | Curvas de automação | 8 | 6 | 2 | 0 |
| F-TRK | Faixas | 21 | 10 | 8 | 3 |
| F-CLP | Clipes e itens | 32 | 18 | 9 | 5 |
| F-REC | Gravação | 55 | 29 | 25 | 1 |
| F-MIX | Mixer e mesa analógica | 56 | 51 | 4 | 1 |
| F-INS | Instrumentos | 24 | 14 | 9 | 1 |
| F-HOST | Plugins de terceiros (hospedagem) | 3 | 1 | 1 | 1 |
| F-FX | Efeitos e ferramentas | 35 | 20 | 10 | 5 |
| F-CTL | Automação e controladores | 9 | 3 | 3 | 3 |
| F-PRJ | Projeto e arquivos | 23 | 15 | 6 | 2 |
| F-CFG | Configurações | 12 | 2 | 10 | 0 |
| F-BRW | Navegador | 12 | 1 | 10 | 1 |
| F-GER | Outros | 12 | 2 | 10 | 0 |
| **Total** | | **406** | **225** | **150** | **31** |

## Seções

## F-JAN: Casca do programa, menus e transporte

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-JAN-001 | Página Composição | feito |  | lm-gui/src/pages/composition.rs,lm-gui/src/workspace.rs |
| F-JAN-002 | Página Mixagem | feito |  | lm-gui/src/pages/mixing.rs,lm-gui/src/workspace.rs |
| F-JAN-003 | Página Configurações | parcial | é um diálogo modal (Ctrl+,), não uma página; não abre sozinha na primeira execução nem quando o dispositivo de áudio falha; sair não aplica nada de especial (aplica ao vivo) | lm-gui/src/screens/settings.rs,lm-gui/src/workspace.rs |
| F-JAN-004 | Dois navegadores | parcial | só um navegador (esquerda); Ctrl+8 mostra e esconde; sem navegador separado de efeitos na Mixagem nem opção esquerda/direita | lm-gui/src/screens/browser.rs,lm-gui/src/screens/mod.rs |
| F-JAN-005 | Recolher uma área | falta | nenhum botão recolhe a área de abas; só há divisores arrastáveis dos docks | lm-gui/src/workspace.rs |
| F-JAN-006 | Janelas flutuantes | parcial | páginas e telas não modais destacam para janela do sistema (Ver > Destacar, Destacar/Encaixar todas), fechar encaixa de volta, o destacado vai para o documento da interface; falta flutuar sobre as páginas e a política ao fechar destacada | lm-gui/src/workspace.rs,lm-gui/src/screens/mod.rs |
| F-JAN-007 | Título da janela | parcial | o nome e o marcador de modificado ficam na barra de abas; o título da janela do sistema é fixo "LinuxMusicBeta"; sem "Sem título" nem "Sessão recuperada" | lm-gui/src/workspace.rs,lm-gui/src/lib.rs |
| F-JAN-008 | Botão Salvar pisca no autosave | falta | nenhuma piscada do botão Salvar quando o autosave escreve | lm-session/src/session.rs |
| F-JAN-009 | Perguntar antes de perder mudanças | parcial | Sair/fechar pergunta Salvar/Não salvar/Cancelar e Novo avisa, mas Abrir projeto não pergunta; sem texto próprio para sessão recuperada | lm-gui/src/workspace.rs,lm-gui/src/screens/new_project.rs |
| F-JAN-010 | Ao fechar | parcial | o núcleo mantém autosave e journal, mas não há rotina explícita ao fechar (fechar configurações, terminar o autosave, apagar o arquivo de recuperação) conferida na GUI; sem lixeira (P-13) fica por conta de project.clean_unused | lm-session/src/session.rs,lm-session/src/core_actions.rs |
| F-JAN-011 | Dicas (tooltips) | parcial | há dicas em vários controles, mas a preferência interface.tooltips não é lida por ninguém (não dá para desligar) | lm-gui/src/screens/settings.rs,lm-gui/src/screens/piano_roll.rs |
| F-JAN-012 | Teclas globais | parcial | Espaço, Shift+Espaço, R, L e Home funcionam; o teclado do computador toca notas na janela de instrumento e no editor de notas; loop em L e não C (configurável) | lm-gui/src/actions.rs,lm-gui/src/workspace.rs |
| F-JAN-013 | Mensagens flutuantes | feito |  | lm-gui/src/workspace.rs,lm-gui/src/screens/notices.rs,lm-session/src/notices.rs |
| F-JAN-020 | Arquivo | parcial | barra de menus pronta (nativa no macOS, na janela no Linux) com Salvar como nova versão (Ctrl+Alt+S, ação project.save_version) e Salvar como modelo; faltam Recentes e Novo a partir de modelo como submenus (a tela Novo projeto tem os modelos) e Exportar faixas (os stems estão na tela Exportar) | lm-gui/src/menus.rs,lm-gui/src/actions.rs,lm-session/src/core_actions.rs |
| F-JAN-021 | Editar | parcial | menu Editar com Desfazer, Refazer e Configurações; Escalas e mapas de teclas fica na aba Afinação da janela do instrumento | lm-gui/src/menus.rs,lm-gui/src/screens/instrument.rs |
| F-JAN-022 | Ver | parcial | menu Ver: páginas Ctrl+1..3, Piano Roll Ctrl+4, Automação Ctrl+5, Passos Ctrl+6, Navegador Ctrl+8, Tela cheia F11, Destacar, Destacar e Encaixar todas; faltam Notas do projeto e rolagem suave/nomes das notas no menu (preferências sem consumidor) | lm-gui/src/menus.rs,lm-gui/src/actions.rs |
| F-JAN-023 | Ferramentas | feito |  | lm-gui/src/menus.rs,lm-gui/src/tap.rs,lm-gui/src/pages/transport.rs |
| F-JAN-024 | Ajuda | feito | P-34: manual próprio pt/en em docs/manual, aberto por Ajuda > Manual (aba Manual do Sobre); sem ajuda online | lm-gui/src/screens/manual.rs,lm-gui/src/screens/about.rs,docs/manual |
| F-JAN-025 | Atalhos configuráveis | feito |  | lm-gui/src/actions.rs,lm-gui/src/screens/settings.rs |
| F-JAN-026 | Paleta de comandos | feito |  | lm-gui/src/screens/palette.rs,lm-gui/src/actions.rs |
| F-JAN-040 | Ao início | feito |  | lm-gui/src/pages/transport.rs,lm-gui/src/actions.rs |
| F-JAN-041 | Parar, Tocar/Pausar | feito |  | lm-gui/src/pages/transport.rs,lm-gui/src/actions.rs |
| F-JAN-042 | Gravar | feito |  | lm-gui/src/pages/transport.rs,lm-session/src/core_actions.rs |
| F-JAN-043 | Loop | feito |  | lm-gui/src/pages/transport.rs,lm-gui/src/actions.rs |
| F-JAN-044 | Relógio de posição e relógio de tempo | feito |  | lm-gui/src/pages/transport.rs,lm-gui/src/view.rs |
| F-JAN-045 | Estado em palavra | feito |  | lm-gui/src/pages/transport.rs |
| F-JAN-046 | Andamento | parcial | arrastar, roda e botão Tap (média das últimas 8 batidas, recomeça após 2 s) mudam o andamento; faltam duplo clique para digitar, menu do clique, Shift+roda ±10 | lm-gui/src/pages/transport.rs,lm-gui/src/tap.rs |
| F-JAN-047 | Fórmula de compasso | feito | editor de numerador e denominador no transporte (altera o primeiro ponto do mapa) | lm-gui/src/pages/transport.rs |
| F-JAN-048 | Metrônomo, pré-roll, contagem | feito |  | lm-gui/src/pages/transport.rs,lm-compile/src/core.rs |
| F-JAN-049 | Mudo do master | feito | botão Master no transporte e ação transport.master_mute | lm-gui/src/pages/transport.rs |
| F-JAN-050 | Medidores CPU e OUT | feito | CPU (carga do renderizador, telemetria load) e OUT (pico do master), vermelho acima de 95 % | lm-gui/src/pages/transport.rs,lm-port/src/lib.rs,lm-session/src/session.rs |
| F-JAN-051 | Volume e transposição geral do projeto | feito | aberto: P-37 respondida (transposição geral no transporte; volume global é o do master); o botão Transpose existe e o compilador usa transport.transpose | lm-gui/src/pages/transport.rs,lm-compile/src/core.rs |

## F-IJN: Janela do instrumento

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-IJN-001 | Cabeçalho | feito |  | lm-gui/src/screens/instrument.rs |
| F-IJN-002 | VOL, PAN, PITCH, RANGE, CHAN | feito |  | lm-plugin/src/instrument/schema.rs,lm-gui/src/screens/instrument.rs |
| F-IJN-003 | Salvar preset da faixa | feito |  | lm-session/src/core_actions.rs,lm-gui/src/screens/instrument.rs |
| F-IJN-004 | Soltar na janela | parcial | solta instrumento (troca) e preset do mesmo plugin; soltar SoundFont, .xiz, outro arquivo de plugin que troca o plugin não é tratado; .xpf não existe | lm-gui/src/screens/instrument.rs,lm-gui/src/screens/browser.rs |
| F-IJN-005 | A janela lembra a aba, o tamanho e a posição; o teclado do c | falta | a janela não lembra aba, tamanho nem posição (nenhuma gravação no documento da interface); o teclado do computador não toca | lm-gui/src/screens/instrument.rs |
| F-IJN-006 | Aba do plugin | feito |  | lm-gui/src/screens/instrument.rs,lm-gui-plugins/src/panel.rs |
| F-IJN-010 | Envelopes e LFOs | parcial | envelopes e LFOs (3 alvos, sync, x100, gráficos) existem no núcleo e na GUI; falta a forma do usuário (soltar amostra no LFO) e a escala Dinâmica/Absoluta/Relativa do gráfico | lm-plugin/src/instrument/shaping.rs,lm-gui/src/screens/instrument.rs,lm-dsp/src/envelope.rs |
| F-IJN-011 | Filtro | feito | por ora no módulo shaping.rs, que vira efeito de áudio no próximo passo | lm-plugin/src/instrument/shaping.rs,lm-gui/src/screens/instrument.rs |
| F-IJN-012 | Aviso de instrumento que não aceita | feito |  | lm-gui/src/screens/instrument.rs |
| F-IJN-013 | Empilhar acordes | feito | agora é o efeito de nota Acordes (26/11, item 2); migração dos projetos antigos em lm-doc/migrate.rs | lm-plugins-noteeffects/src/chords.rs,lm-gui/src/screens/instrument/note_effects.rs |
| F-IJN-014 | Mono, legato e glide | feito | ficam no instrumento (são da voz) | lm-plugin/src/instrument/notes.rs,lm-gui/src/screens/instrument.rs |
| F-IJN-015 | Arpejo | feito | agora é o efeito de nota Arpejador (livre, ordenado, sincronizado, tempo com sync, gate); o sustain segura o arpejo | lm-plugins-noteeffects/src/arpeggiator.rs,lm-gui/src/screens/instrument/note_effects.rs |
| F-IJN-016 | Tabela de acordes e escalas | feito |  | lm-plugins-noteeffects/src/table.rs,lm-gui/src/screens/piano_roll.rs |
| F-IJN-017 | MIDI | parcial | entrada e saída MIDI de faixa existem (midiInput/midiOutput no núcleo e na aba MIDI); a velocidade base (baseVelocity) só é gravada, sem consumidor no núcleo | lm-gui/src/screens/instrument.rs,lm-session/src/midi/mod.rs,lm-compile/src/core.rs |
| F-IJN-018 | Transposição geral | feito | aberto: P-37 respondida; a faixa tem o seletor de seguir a transposição geral e o compilador a aplica | lm-gui/src/screens/instrument.rs,lm-compile/src/core.rs |
| F-IJN-019 | Microafinador | feito |  | lm-doc/src/tuning.rs,lm-compile/src/keys.rs,lm-gui/src/screens/instrument.rs |
| F-IJN-020 | Editor de escalas e mapas | parcial | o editor de escalas/mapas e o leitor de .scl/.kbm existem, mas as ações tuning.import_scale/import_keymap só existem no lm-port-fake, não na sessão real; faltam salvar .scl/.kbm | lm-gui/src/screens/instrument.rs,lm-doc/src/tuning.rs,lm-port-fake/src/lib.rs |
| F-IJN-021 | Teclado na janela | parcial | o teclado da janela toca a tecla no instrumento (CorePort::note_on, velocidade pela altura do clique, glissando, solta no mouse-up, perda de foco e ao fechar); faltam os marcadores de nota base/primeira/última | lm-gui/src/screens/instrument.rs |
| F-IJN-022 | Teclado do computador como piano | feito | o teclado do computador toca o instrumento na janela (foco na própria janela) e no editor de notas, no layout do legado (Z = Dó, duas fileiras); solta ao soltar a tecla, perder o foco e fechar | lm-gui/src/screens/piano_roll.rs |
| F-IJN-025 | Rack de CC MIDI | feito |  | lm-gui/src/screens/instrument.rs,lm-compile/src/core.rs,lm-doc/src/edit.rs |

## F-NOT: Editor de notas

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-NOT-001 | Desenhar notas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-002 | Apagar notas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-003 | Selecionar | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-004 | Curva de pitch bend por nota | feito | aberto: respondido em P-24 (curva de afinação como campo bend da nota, ferramenta Pitch); o Shift+clique que abre a curva no editor de automação não existe, já que a curva não é automação | lm-gui/src/screens/piano_roll.rs,lm-doc/src/schemas.rs,lm-compile/src/items.rs |
| F-NOT-005 | Faca | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-006 | Strum | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-007 | Mover notas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-008 | Redimensionar notas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-009 | Teclado | feito |  | lm-gui/src/screens/piano_roll.rs,lm-gui/src/actions.rs |
| F-NOT-010 | Velocidade e pan das notas | feito |  | lm-gui/src/screens/piano_roll.rs,lm-doc/src/schemas.rs |
| F-NOT-011 | Ferramentas de notas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-012 | Quantização e tamanho de nota | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-013 | Zoom e rolagem | feito |  | lm-gui/src/screens/piano_roll_bars.rs |
| F-NOT-014 | Tom, escala e acorde | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-015 | Teclado do piano | parcial | clicar nas teclas, glissando e velocidade pela posição soam no instrumento da faixa do clipe e soltam no mouse-up; faltam teclas do microafinador e nomes opcionais | lm-gui/src/screens/piano_roll.rs |
| F-NOT-016 | Notas fantasmas | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-017 | Tocar o clipe | parcial | o Espaço só anima o cursor e o loop do item na tela (ItemPlay): não toca o áudio do clipe em loop | lm-gui/src/screens/piano_roll.rs |
| F-NOT-018 | Gravar no editor de notas | parcial | o núcleo tem notes.record_into (gravar no item aberto), mas o piano roll não o chama nem usa o teclado MIDI: grava só as teclas do computador por conta própria; sem quantizar automático | lm-session/src/midi/step.rs,lm-session/src/midi/record.rs,lm-gui/src/screens/piano_roll.rs |
| F-NOT-019 | Copiar, cortar, colar | feito |  | lm-gui/src/screens/piano_roll/clip_text.rs |
| F-NOT-021 | Selecionar por região da régua | feito |  | lm-gui/src/screens/piano_roll.rs |
| F-NOT-022 | Abrir o editor sem clipe | feito |  | lm-gui/src/screens/piano_roll/first_clip.rs |
| F-NOT-023 | Teclado do computador como piano | feito | as teclas do computador tocam o instrumento da faixa do clipe em qualquer modo, e gravam ao vivo ou em passo pelo núcleo | lm-gui/src/screens/piano_roll.rs |

## F-PAS: Gravação por passos

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-PAS-001 | Gravar um passo de cada vez | feito | a tela usa o gravar passo do núcleo: notes.step_record ao ligar (e ao mudar o passo ou clicar na régua), notes.step_keys com o acorde ao soltar a última tecla, notes.step_rest (→), notes.step_back (← e Delete), notes.step_length (Shift+←/→), notes.step_stop ao desligar e fechar; o cursor vem do stepRecord do documento da interface | lm-session/src/midi/step.rs,lm-session/src/midi/actions.rs,lm-gui/src/screens/piano_roll.rs |

## F-ARR: Arranjo

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-ARR-001 | Ferramentas | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-ARR-002 | Inserir e remover compasso | feito |  | lm-session/src/arrange.rs,lm-gui/src/actions.rs |
| F-ARR-003 | Cursor por compasso | feito |  | lm-gui/src/pages/composition/edit.rs,lm-gui/src/actions.rs |
| F-ARR-004 | Apagar e selecionar | feito |  | lm-gui/src/pages/composition/edit.rs,lm-gui/src/actions.rs |
| F-ARR-005 | Dividir no cursor | feito |  | lm-session/src/arrange.rs,lm-gui/src/pages/composition/edit.rs |
| F-ARR-006 | Copiar, cortar, colar, duplicar | feito |  | lm-session/src/arrange.rs,lm-gui/src/pages/composition/edit.rs |
| F-ARR-007 | Roda estilo REAPER | feito | Ctrl+0 (arrange.zoom_reset) volta o zoom padrão | lm-gui/src/pages/composition.rs,lm-gui/src/view.rs |
| F-ARR-008 | Zoom | feito | o zoom é lembrado no documento da interface; limites de 4 a 400 px por compasso | lm-gui/src/view.rs,lm-gui/src/pages/composition.rs |
| F-ARR-009 | Grade | feito | grade proporcional que acompanha o zoom | lm-gui/src/pages/composition/edit.rs |
| F-ARR-010 | Criar faixas | feito | botões de faixa de instrumento, de áudio e de pasta (a faixa de automação vive no dono do parâmetro) | lm-gui/src/pages/composition.rs |
| F-ARR-011 | Soltar arquivos no arranjo | parcial | arquivos do sistema: áudio vira faixa de áudio nova, MIDI importa e projeto abre; não há o limite áudio longo/curto com sampler, nem soltar do navegador interno (amostra, instrumento, preset, SoundFont, VST, patch) | lm-gui/src/screens/files.rs,lm-gui/src/pages/composition.rs |
| F-ARR-012 | Faixa selecionada | feito |  | lm-gui/src/pages/composition.rs |
| F-ARR-013 | Depois de abrir um projeto | feito |  |  |
| F-ARR-014 | Grade de fundo | feito |  | lm-gui/src/pages/composition.rs |

## F-REG: Régua, marcadores e andamentos

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-REG-002 | Linha do tempo própria por editor | feito | aberto: respondido em P-26 (piano roll com cursor e loop próprios do item, ItemPlay); o transporte do arranjo continua único e o editor de passos/automação só tem cursor local | lm-gui/src/screens/piano_roll.rs,lm-gui/src/screens/automation.rs |
| F-REG-003 | Rolagem automática | feito |  | lm-gui/src/pages/composition.rs |
| F-REG-004 | Ao parar | feito |  | lm-session/src/session.rs |
| F-REG-005 | Mover o cursor | feito |  | lm-gui/src/pages/composition/ruler.rs |
| F-REG-006 | Desenhar o loop na régua | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-REG-007 | Editar o loop com Shift | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-REG-008 | Selecionar uma região pela régua | feito |  | lm-gui/src/pages/composition.rs |
| F-REG-009 | Números dos compassos | feito |  | lm-gui/src/pages/composition.rs |
| F-REG-010 | Cursor | feito |  | lm-gui/src/pages/composition.rs |
| F-REG-011 | Marcadores com nome | feito |  | lm-gui/src/pages/marks.rs |
| F-REG-012 | Regiões | feito |  | lm-gui/src/pages/marks.rs |
| F-REG-013 | Mapa de andamentos | feito |  | lm-gui/src/pages/composition/tempo.rs |
| F-REG-014 | Mudar o andamento no cursor | feito |  | lm-gui/src/pages/composition/tempo.rs,lm-gui/src/actions.rs |

## F-PAD: Padrões

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-PAD-002 | Passos | feito |  | lm-gui/src/screens/steps.rs |
| F-PAD-004 | Amostras na grade do padrão | falta | aberto: respondido em P-23 (instrumento de pads de amostras tocado pelo editor de passos); esse instrumento não existe (só AudioFileProcessor, DrumMachines e AvlDrums) | lm-plugins-instruments/src/audio_file_processor.rs,lm-plugins-instruments/src/drum_machines.rs |

## F-AUT: Curvas de automação

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-AUT-001 | Desenhar pontos | feito |  | lm-gui/src/screens/automation.rs |
| F-AUT-002 | Apagar pontos | feito |  | lm-gui/src/screens/automation.rs |
| F-AUT-003 | Valor de saída (salto no ponto) | feito | aberto: respondido em P-27 (campo out do ponto, ferramenta Shift+C, botão direito volta a saída) | lm-gui/src/screens/automation.rs,lm-doc/src/schemas.rs |
| F-AUT-005 | Editar o valor | feito |  | lm-gui/src/screens/automation.rs |
| F-AUT-006 | Espelhar | feito |  | lm-gui/src/screens/automation.rs |
| F-AUT-008 | Zoom, grade e rolagem | feito |  | lm-gui/src/screens/automation.rs |
| F-AUT-009 | Fantasmas | parcial | só as notas de um item MIDI; falta a onda de uma amostra | lm-gui/src/screens/automation.rs |
| F-AUT-011 | Tocar a curva | parcial | o Espaço só anima o cursor local do editor, sem loop nem régua do clipe | lm-gui/src/screens/automation.rs |

## F-TRK: Faixas

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-TRK-001 | Propriedades comuns | parcial | nome, cor, mudo, solo, pasta e camadas estão no documento; a altura não é da faixa (só um valor global da página) e cor/camada não têm edição na interface | lm-doc/src/schemas.rs,lm-gui/src/pages/composition.rs |
| F-TRK-002 | Faixa de instrumento | feito |  | lm-doc/src/schemas.rs,lm-gui/src/screens/instrument.rs |
| F-TRK-004 | Faixa de áudio | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/composition.rs |
| F-TRK-007 | Pasta | parcial | organizar, recolher, mudo/solo do grupo e entrar/sair da pasta existem; faltam criar pasta na interface e mover a pasta com o conteúdo | lm-gui/src/pages/composition/edit.rs,lm-compile/src/core.rs |
| F-TRK-008 | Camadas da faixa | feito |  | lm-gui/src/pages/composition.rs,lm-doc/src/schemas.rs |
| F-TRK-010 | Tamanho da faixa | feito |  | lm-gui/src/pages/composition.rs |
| F-TRK-020 | Número e cor | parcial | a coluna de número sobre a cor existe, mas não é a alça de mover (arrastar a faixa não existe) | lm-gui/src/pages/composition.rs |
| F-TRK-021 | Armar, nome, M, S | feito |  | lm-gui/src/pages/composition.rs |
| F-TRK-022 | Knobs de volume e pan | feito |  | lm-gui/src/pages/composition.rs |
| F-TRK-023 | Botão FX | feito | aberto: respondido em P-28 (o FX abre os inserts do canal da saída principal, open_fx) | lm-gui/src/pages/composition.rs |
| F-TRK-024 | Botão I/O | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-TRK-025 | Terceira linha (faixa alta) | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-TRK-026 | Medidor do canal na borda | feito |  | lm-gui/src/pages/composition.rs |
| F-TRK-027 | Clique no nome | falta | o nome no cabeçalho não tem clique: não abre a janela do instrumento nem de efeitos, sem renomear no lugar, sem soltar preset no nome | lm-gui/src/pages/composition.rs |
| F-TRK-028 | Indicador de atividade | feito | lâmpada no cabeçalho: verde com som na saída, vermelha com efeito do canal silenciado (áudio corrompido); o clique toca o dó central na faixa de instrumento | lm-gui/src/pages/composition.rs |
| F-TRK-029 | Mover e copiar faixas | parcial | só entrar/sair da pasta pelo menu da faixa; faltam arrastar para mover (EditOp::Move existe no núcleo mas a faixa não o usa) e copiar com Ctrl | lm-gui/src/pages/composition/edit.rs |
| F-TRK-030 | Altura e largura | parcial | Shift+clique volta a altura e Ctrl/Shift+Alt+roda mudam; faltam arrastar a borda de baixo, a largura do cabeçalho e a altura por faixa (é um valor global) | lm-gui/src/pages/composition.rs |
| F-TRK-031 | Menu da faixa | parcial | clonar, remover (com não perguntar), limpar, exportar MIDI e pasta existem; faltam exportar a faixa como áudio e o submenu Cor (mudar, padrão, sortear, zerar cores dos clipes) e ligar/desligar a gravação da automação | lm-gui/src/pages/composition/edit.rs,lm-session/src/clips.rs |
| F-TRK-032 | Soltar coisas numa faixa | falta | nada recebe soltar numa faixa: só soltar arquivos no arranjo inteiro; o navegador interno arrasta mas só a janela do instrumento e os efeitos aceitam | lm-gui/src/pages/composition.rs,lm-gui/src/screens/browser.rs |
| F-TRK-033 | Clique na linha vazia | parcial | aberto: respondido em P-29 (item de áudio vazio entra); o clique na linha vazia só cria item MIDI em faixa de instrumento, no áudio só existe Shift+E no cursor da faixa selecionada | lm-gui/src/pages/composition/edit.rs,lm-gui/src/pages/composition.rs |
| F-TRK-034 | Menu do botão armar (instrumento) | parcial | o campo recordMode (overdub/substituir/camadas) existe no documento e a gravação o usa, mas não há menu no botão armar nem quantizar ao gravar na interface | lm-doc/src/schemas.rs,lm-session/src/midi/record.rs,lm-gui/src/pages/composition.rs |

## F-CLP: Clipes e itens

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-CLP-001 | Propriedades | parcial | início, tamanho, mudo, ganho, fades e camada estão no documento e se editam; nome e cor do clipe não têm edição na interface e não há campo de tamanho automático | lm-doc/src/schemas.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-002 | Mover | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-CLP-003 | Redimensionar e aparar | feito |  | lm-gui/src/pages/composition/edit.rs,lm-session/src/clips.rs |
| F-CLP-004 | Mover para outra faixa | parcial | arrastar na vertical não leva o clipe para outra faixa; o documento aceita EditOp::Move mas a interface só muda o início | lm-gui/src/pages/composition/edit.rs |
| F-CLP-005 | Selecionar e copiar com o mouse | parcial | Ctrl+clique alterna a seleção; faltam Ctrl+arrasto copiando e soltar um clipe sobre outro copiando o conteúdo | lm-gui/src/pages/composition/edit.rs |
| F-CLP-006 | Mudo e apagar com o mouse | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-CLP-007 | Faca | parcial | a divisão pela faca existe; falta a divisão destrutiva do MIDI com Shift | lm-gui/src/pages/composition/edit.rs |
| F-CLP-008 | Menu comum | parcial | faltam exportar o clipe como áudio, Cor, tamanho automático e ouvir esta camada/comp no menu; o resto existe | lm-gui/src/pages/composition/edit.rs |
| F-CLP-009 | Aparência | feito |  | lm-gui/src/pages/composition.rs |
| F-CLP-010 | Área de transferência | parcial | a área de transferência é a da sessão (arrange.copy), não a do sistema: não cola em outro projeto ou instância como texto | lm-session/src/arrange.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-011 | Desfazer | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-CLP-020 | Abrir no editor de notas | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-CLP-021 | Fantasma | feito |  | lm-gui/src/screens/piano_roll.rs,lm-gui/src/screens/automation.rs |
| F-CLP-022 | Limpar notas, juntar, cortar o que sobra | parcial | juntar a seleção e limpar notas existem; falta tirar as notas fora dos limites visíveis (cortar o que sobra) | lm-session/src/clips.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-023 | Transpor | feito |  | lm-session/src/clips.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-024 | Nome | parcial | o campo name existe mas não há renomear nem voltar ao nome padrão na interface | lm-doc/src/schemas.rs |
| F-CLP-025 | Tamanho automático | parcial | só o editor de passos ajusta o tamanho do item ao conteúdo; o piano roll e a gravação não, e não há liga/desliga | lm-gui/src/screens/steps.rs |
| F-CLP-026 | Desenho | feito |  | lm-gui/src/pages/composition.rs |
| F-CLP-041 | Crossfades com tensão | falta | aberto: respondido em P-30 (curvas ajustáveis entram); o item só tem fadeIn/fadeOut em amostras sem tensão e sem alças de curva | lm-doc/src/schemas.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-042 | Inverter a amostra | falta | aberto: respondido em P-31 (inverter o item entra, renderizado como recurso novo); não há campo nem ação |  |
| F-CLP-043 | Exportar a amostra | feito | menu do clipe de áudio "Exportar a amostra" (`clip.export_sample`, F-PRJ-026) |  |
| F-CLP-044 | Memória | feito | aberto: respondido em P-32 (acima de 30 s toca do disco, automático); não há menu manual RAM/disco | lm-io/src/reader.rs,lm-compile/src/core.rs |
| F-CLP-046 | Tempo | feito |  | lm-doc/src/schemas.rs,lm-compile/src/items.rs |
| F-CLP-060 | Propriedades | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/composition/edit.rs |
| F-CLP-061 | Alças no item | feito |  | lm-gui/src/pages/composition/edit.rs |
| F-CLP-062 | Crossfade automático | feito |  | lm-compile/src/core.rs,lm-gui/src/pages/composition.rs |
| F-CLP-063 | Desenho | feito |  | lm-gui/src/pages/composition.rs |
| F-CLP-064 | Menu | feito | aberto: memória decidida em P-32 (automático, sem menu); "Zerar ganho e fades" está no menu do clipe | lm-gui/src/pages/composition/edit.rs,lm-io/src/reader.rs |
| F-CLP-065 | Normalizar | falta | não há ação de normalizar item de áudio (a normalização existente é a do export) |  |
| F-CLP-066 | Curvas de fade ajustáveis | falta | aberto: respondido em P-30 (curvas ajustáveis entram); sem campo de tensão nem interface | lm-doc/src/schemas.rs |
| F-CLP-081 | Gravar a automação | feito |  | lm-session/src/midi/automation.rs,lm-gui/src/screens/automation.rs |
| F-CLP-082 | Abrir, limpar, nome, espelhar | feito |  | lm-gui/src/screens/automation.rs,lm-gui/src/pages/composition.rs |

## F-REC: Gravação

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-REC-001 | Faixa que grava áudio | feito |  | lm-session/src/session.rs,lm-doc/src/schemas.rs |
| F-REC-002 | Escolher a entrada da faixa | feito |  | lm-gui/src/pages/composition/edit.rs,lm-session/src/session.rs |
| F-REC-003 | Uma entrada em várias faixas | feito |  | lm-session/src/session.rs,lm-io/src/record.rs |
| F-REC-004 | Erros ao gravar sem entrada | feito |  | lm-session/src/session.rs |
| F-REC-005 | Armar | feito |  | lm-gui/src/pages/composition.rs,lm-session/src/session.rs |
| F-REC-006 | Gravar sem nada armado | feito |  | lm-gui/src/pages/transport.rs,lm-session/src/session.rs |
| F-REC-007 | Monitoramento por faixa | feito |  | lm-compile/src/core.rs,lm-gui/src/pages/composition.rs |
| F-REC-008 | Gravar seco | feito |  | lm-io/src/record.rs,lm-session/src/session.rs |
| F-REC-009 | Medidor de entrada no cabeçalho | feito |  | lm-gui/src/pages/composition.rs,lm-port/src/lib.rs |
| F-REC-010 | Desligar plugins com latência na faixa armada | parcial | aberto: P-10 respondido; núcleo e preferência lowLatencyMonitoring existem (compile), mas não há chave na GUI de configurações | lm-compile/src/core.rs,lm-session/src/prefs.rs |
| F-REC-011 | Efeitos de entrada gravados, gravar a saída, entrada com mai | parcial | aberto: P-10 respondido; só entradas com N canais existem; faltam "gravar com os efeitos" por faixa e gravar a saída de uma faixa (reamostrar) | lm-session/src/session.rs |
| F-REC-020 | Gravar parado | feito |  | lm-session/src/session.rs,lm-render/src/renderer.rs |
| F-REC-021 | Punch-in e punch-out manuais | feito |  | lm-session/src/session.rs,lm-gui/src/pages/transport.rs |
| F-REC-022 | Auto-punch (por região de tempo) | feito |  | lm-render/src/renderer.rs,lm-session/src/session.rs |
| F-REC-023 | Pré-roll | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/transport.rs |
| F-REC-024 | Contagem | feito |  | lm-render/src/renderer.rs,lm-compile/src/core.rs |
| F-REC-025 | Metrônomo | feito |  | lm-compile/src/core.rs,lm-session/src/session.rs |
| F-REC-026 | Estado do transporte em palavra e cor | parcial | palavra Contagem e cor feitas; falta PAUSADO e "PRÉ-ROLL n" | lm-gui/src/pages/transport.rs,lm-session/src/session.rs |
| F-REC-027 | Um desfazer tira a gravação inteira | feito |  | lm-session/src/session.rs |
| F-REC-028 | Trocar de dispositivo durante a gravação é recusado | feito |  | lm-session/src/core_actions.rs,lm-gui/src/screens/settings.rs |
| F-REC-029 | Gravar MIDI no clipe aberto no editor de notas com a música  | parcial | núcleo tem notes.record_into/recordInto, mas nenhuma opção na GUI (menu do Gravar ou editor de notas) | lm-session/src/midi/step.rs,lm-gui/src/screens/piano_roll.rs |
| F-REC-040 | Gravar em loop | feito |  | lm-io/src/take.rs,lm-session/src/session.rs |
| F-REC-041 | "Takes em camadas" (padrão) | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/composition.rs |
| F-REC-042 | Modo "gravador simples" | feito |  | lm-gui/src/screens/settings.rs |
| F-REC-043 | Ouvir uma camada | parcial | o item de loop tem uma camada por passada e a última toca; falta o comando "Ouvir esta camada" na GUI | lm-compile/src/items.rs,lm-gui/src/pages/composition.rs |
| F-REC-046 | Takes presos ao andamento | feito |  | lm-doc/src/schemas.rs,lm-compile/src/stretch.rs |
| F-REC-062 | Copiar os arquivos usados ao salvar | feito |  | lm-session/src/core_actions.rs,lm-gui/src/screens/files.rs |
| F-REC-063 | Lixo dos arquivos gerados | feito | aberto: P-13 respondido; project.clean_unused existe com botão na tela do projeto | lm-session/src/core_actions.rs,lm-gui/src/screens/project.rs |
| F-REC-064 | Escrita contínua e segura | feito |  | lm-io/src/disk.rs,lm-io/src/take.rs |
| F-REC-066 | Falha de escrita | feito |  | lm-io/src/disk.rs,lm-session/src/session.rs |
| F-REC-067 | Faixa apagada durante a gravação | feito |  | lm-session/src/session.rs |
| F-REC-068 | Onda ao vivo | feito |  | lm-gui/src/pages/composition.rs,lm-port/src/lib.rs |
| F-REC-069 | Recuperar takes depois de uma queda | feito |  | lm-session/src/journal.rs,lm-io/src/take.rs |
| F-REC-070 | Mudar o andamento no meio da gravação | feito |  | lm-io/src/record.rs,lm-session/src/session.rs |
| F-REC-080 | Alinhar o take | feito |  | lm-session/src/session.rs |
| F-REC-081 | Calibração por loopback | feito |  | lm-io/src/calibrate.rs,lm-gui/src/screens/settings.rs |
| F-REC-082 | Ajuste manual | parcial | o ajuste manual é global (audio.latencyFrames), não por dispositivo, e só em quadros | lm-gui/src/screens/settings.rs,lm-session/src/session.rs |
| F-REC-083 | Latência real mostrada | feito |  | lm-gui/src/screens/settings.rs |
| F-REC-084 | Xruns marcados no take | parcial | aviso ao fim com a contagem das lacunas; faltam as lacunas em vermelho no take | lm-io/src/take.rs,lm-gui/src/pages/composition.rs |
| F-REC-086 | Contador global de xruns fora da gravação | feito | aberto: P-14 respondido; contador no transporte | lm-gui/src/pages/transport.rs |
| F-REC-100 | Gravar notas na linha do tempo | feito |  | lm-session/src/midi/record.rs |
| F-REC-101 | Modos de gravação MIDI | feito |  | lm-session/src/midi/record.rs,lm-doc/src/schemas.rs |
| F-REC-102 | Quantizar ao gravar | feito |  | lm-session/src/midi/record.rs,lm-gui/src/screens/settings.rs |
| F-REC-103 | Entrada MIDI da faixa | feito |  | lm-gui/src/screens/instrument.rs,lm-session/src/midi/step.rs |
| F-REC-104 | CC, pitch bend, aftertouch, program change na gravação; um t | feito |  | lm-session/src/midi/record.rs |
| F-REC-090 | Re-renderizar takes quando o andamento muda | parcial | compile e lm-io re-renderizam o stretch, mas não há aviso "vai renderizar N itens, ~X s" com progresso e Cancelar que desfaz na GUI; items.rerender só recarrega recursos | lm-compile/src/stretch.rs,lm-io/src/stretch.rs |
| F-REC-120 | Trocar de dispositivo sem reiniciar | feito |  | lm-gui/src/screens/settings.rs,lm-session/src/core_actions.rs |
| F-REC-121 | Taxa de amostragem | feito |  | lm-gui/src/screens/settings.rs,lm-session/src/actions.rs |
| F-REC-122 | Tamanho do bloco | feito |  | lm-gui/src/screens/settings.rs,lm-session/src/core_actions.rs |
| F-REC-123 | ALSA | feito |  | lm-io/src/alsa.rs,lm-gui-app/src/main.rs |
| F-REC-125 | JACK | parcial | só a ligação lm-sys-jack; não há backend JACK em lm-io nem DeviceSetup::Jack; a GUI mostra "JACK (phase 2)" desligado | lm-sys-jack/src/client.rs,lm-gui/src/screens/settings.rs |
| F-REC-126 | CoreAudio (macOS) | feito |  | lm-io/src/coreaudio.rs,lm-gui-app/src/main.rs |
| F-REC-128 | Dispositivo falso | feito |  | lm-io/src/dummy.rs,lm-session/src/session.rs |
| F-REC-130 | MIDI | parcial | ALSA seq e ALSA raw prontos (core e GUI); faltam JACK MIDI e CoreMIDI (previstos para 1.0) | lm-io/src/mididev.rs,lm-session/src/midi/devices.rs |
| F-REC-131 | "Controlador para a faixa selecionada" | feito |  | lm-session/src/midi/mod.rs,lm-gui/src/screens/instrument.rs |

## F-MIX: Mixer e mesa analógica

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-MIX-001 | Modo analógico ou digital da mesa inteira | feito | aberto: decidido em P-17 (fica; digital limpo) e feito conforme a decisão; o projeto novo abre na mesa analógica (`mixer.mode` ausente) | lm-doc/src/schemas.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-003 | Várias faixas num canal | feito |  | lm-doc/src/schemas.rs,lm-gui/src/screens/routing.rs |
| F-MIX-004 | Criar canal | feito |  | lm-gui/src/pages/mixing.rs,lm-gui/src/actions.rs |
| F-MIX-005 | Apagar canal | feito |  | lm-gui/src/pages/mixing.rs,lm-doc/src/schemas.rs |
| F-MIX-006 | Renomear canal | feito |  | lm-gui/src/pages/mixing.rs |
| F-MIX-007 | Cor do canal | feito |  | lm-gui/src/pages/mixing.rs |
| F-MIX-008 | Reordenar | feito |  | lm-gui/src/pages/mixing.rs |
| F-MIX-009 | Selecionar canal | feito |  | lm-gui/src/pages/mixing.rs,lm-gui/src/pages/composition.rs |
| F-MIX-010 | Escolher o canal da faixa | feito |  | lm-gui/src/screens/routing.rs,lm-gui/src/pages/composition.rs |
| F-MIX-011 | Recolher os canais de uma pasta | feito | aberto: decidido em P-18 (grupo = canal que recebe outros); recolher/mostrar o grupo existe na vista | lm-gui/src/pages/mixing.rs |
| F-MIX-012 | Um canal por saída / por peça | feito |  | lm-session/src/arrange.rs,lm-gui/src/screens/routing.rs |
| F-MIX-013 | Teclado na mesa | feito |  | lm-gui/src/actions.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-014 | Mudo e solo do canal | feito |  | lm-compile/src/core.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-015 | Mudo do master no transporte | feito |  | lm-gui/src/pages/transport.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-020 | Envios | feito |  | lm-compile/src/core.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-021 | Recusar laços | feito |  | lm-doc/src/rules.rs,lm-compile/src/core.rs |
| F-MIX-023 | Dois retornos de efeito fixos (FX 1, FX 2) | feito |  | lm-doc/src/document.rs,lm-doc/src/schemas.rs,lm-gui/src/model.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-024 | Sidechain | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/mixing.rs,lm-compile/src/core.rs |
| F-MIX-025 | Compensação de latência (PDC) | feito |  | lm-compile/src/core.rs |
| F-MIX-026 | Limpar áudio corrompido | feito |  | lm-render/src/renderer.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-030 | Trim | feito |  | lm-gui/src/pages/mixing.rs,lm-doc/src/schemas.rs |
| F-MIX-031 | Mono do canal | feito |  | lm-compile/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-032 | Ponto de inserção dos efeitos | feito | aberto: decidido em P-20 (seis pontos por insert); feito conforme a decisão | lm-compile/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-033 | Inverter fase e trocar L/R | feito |  | lm-compile/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-034 | Largura (width) | feito | | lm-doc/src/schemas.rs,lm-compile/src/rack.rs,lm-session/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-035 | Fader | feito |  | lm-gui/src/pages/mixing.rs,lm-gui/src/ui |
| F-MIX-036 | Pan | feito |  | lm-doc/src/schemas.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-040 | GAIN | feito |  | lm-dsp/src/gain_stage.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-041 | Modelo do estágio de ganho | feito |  | lm-dsp/src/gain_stage.rs,lm-dsp/src/tube.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-042 | Válvulas leve ou realista | feito |  | lm-dsp/src/tube.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-043 | LEDs SAT e CLIP | feito |  | lm-dsp/src/channel_strip.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-044 | LCUT | feito |  | lm-dsp/src/channel_strip.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-045 | HPF e LPF | feito |  | lm-dsp/src/filters.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-046 | Mono bass | feito |  | lm-dsp/src/mono_bass.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-047 | Compressor: 9 modelos | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-048 | COMP | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-049 | RATIO | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-050 | ATK | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-051 | REL | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-052 | Compensação automática e link | feito |  | lm-dsp/src/dynamics.rs |
| F-MIX-053 | Gate | feito |  | lm-dsp/src/dynamics.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-054 | EQ de 4 bandas (Soundcraft/Mackie) | feito |  | lm-dsp/src/eq.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-055 | Tolerância dos componentes | feito | aberto: decidido em P-21 (semente pelo id do canal); feito conforme a decisão | lm-dsp/src/tolerance.rs,lm-compile/src/rack.rs |
| F-MIX-060 | Ordem do master | feito |  | lm-dsp/src/master.rs,lm-compile/src/rack.rs |
| F-MIX-061 | Faders PRE L e PRE R | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-062 | POST | feito |  | lm-compile/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-063 | DRIVE do master | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-064 | XFMR | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-065 | ESTÉREO | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-066 | BUS COMP (SSL G) | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-067 | Monitoração | feito |  | lm-dsp/src/master.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-068 | Medidores do master | feito |  | lm-dsp/src/loudness.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-070 | Medidor do canal | feito | P-22: pico, RMS, VU e K-20/14/12 escolhíveis (preferência `interface.channelMeter`); o LUFS fica no master (BS.1770 mede o programa, ver decisoes-autonomas) | lm-render/src/renderer.rs,lm-gui/src/core.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-071 | Medidor no cabeçalho da faixa | feito |  | lm-gui/src/pages/composition.rs |
| F-MIX-072 | Botões da mesa | feito |  | lm-gui/src/pages/desk.rs,lm-gui/src/pages/mixing.rs |
| F-MIX-073 | Efeitos no canal | feito |  | lm-gui/src/pages/mixing.rs,lm-gui/src/screens/effect.rs,lm-gui/src/screens/browser.rs |

## F-INS: Instrumentos

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-INS-001 | AudioFileProcessor (sampler simples) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format audio, 26/11); o plugin antigo foi apagado (27/11). Painel do Sampler: onda com marcadores de início, fim e laço (arrastar, roda para o zoom), laço (não, sim, vai e volta), inverter, gaguejar e os três pontos em botões; falta o cursor da nota tocando e o declick | lm-sampler/src/import/audio.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-002 | AvlDrums (baterias acústicas AVL, do fork) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format kit, kit AVL, saídas por grupo); o plugin antigo foi apagado (27/11). Painel do Sampler: a foto do kit com as peças que acendem ao tocar (mapa do avldrums.lv2, brilho que cai 0,85 a cada 33 ms); falta clicar na foto para tocar | lm-sampler/src/import/avl.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-003 | BitInvader (onda desenhada) | parcial | painel próprio feito; falta o botão de onda do usuário (carregar amostra no gráfico), declarado não portado no painel | lm-plugins-instruments/src/bit_invader.rs,lm-gui-plugins/src/panels/bitinvader.rs |
| F-INS-004 | DrumMachines (baterias eletrônicas clássicas, do fork) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format kit, kit DrumMachines: `.ds` pela drumsynth e `.flac`); o plugin antigo foi apagado (27/11). O painel mostra o mapa de zonas com as teclas acesas; as fotos dos kits ainda não | lm-sampler/src/import/drums.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-005 | FreeBoy (APU do Game Boy) | feito |  | lm-plugins-freeboy/src/lib.rs,lm-gui-plugins/src/panels/freeboy.rs |
| F-INS-006 | GigPlayer (arquivos GigaSampler .gig) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format gig): leitor GIG em Rust puro, sem libgig (instrumentos, regiões, dimensões de velocidade, release trigger, pedal, aleatório, round robin, camadas e canal; EG1, laços, curvas de velocidade, PCM de 16 e 24 bits); amostras comprimidas recusadas com `sampler.gig.compressed`; key switches e dimensões de controlador tomam a primeira zona, como o GigPlayer; o plugin antigo foi apagado (27/11). Lista de presets no painel | lm-sampler/src/import/gig.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-007 | Kicker (sintetizador de bumbo) | feito |  | lm-plugins-instruments/src/kicker.rs,lm-gui-plugins/src/panels/kicker.rs |
| F-INS-008 | Lb302 (imitação monofônica da TB-303) | feito |  | lm-plugins-instruments/src/lb302.rs,lm-gui-plugins/src/panels/lb302.rs |
| F-INS-009 | Monstro (3 osciladores com matriz de modulação) | feito |  | lm-plugins-instruments/src/monstro.rs,lm-gui/src/screens/instrument.rs |
| F-INS-010 | Nes (sintetizador estilo NES) | feito |  | lm-plugins-instruments/src/nes.rs,lm-gui-plugins/src/panels/nes.rs |
| F-INS-011 | OpulenZ (FM de 2 operadores, OPL2/AdLib) | feito |  | lm-plugins-opulenz/src/lib.rs,lm-gui-plugins/src/panels/opulenz.rs |
| F-INS-012 | Organic (aditivo, timbre de órgão) | feito |  | lm-plugins-instruments/src/organic.rs,lm-gui-plugins/src/panels/organic.rs |
| F-INS-013 | PatMan (patches GUS .pat) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format pat, leitor de `lm_dsp::patman`); o plugin antigo foi apagado (27/11); botão de abrir e mapa de zonas no painel do Sampler | lm-sampler/src/import/pat.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-014 | Sf2 Player (SoundFont) | parcial (sampler escrito, ainda não testado) | trocado pelo `sampler` (format sf2: leitor RIFF próprio, presets, geradores, modulador de velocidade, laços; `.sf3` por decodificador Ogg do plugin); o plugin antigo foi apagado (27/11). Painel: botão de abrir, lista de presets lida do arquivo (`sampler.list_presets`) e mapa de zonas; o SoundFont padrão das preferências não é aplicado a instrumento novo (só o campo da tela de configurações) | lm-sampler/src/import/sf2.rs,lm-gui-plugins/src/panels/sampler.rs |
| F-INS-015 | sfxr (efeitos sonoros retrô) | feito |  | lm-plugins-instruments/src/sfxr.rs,lm-gui-plugins/src/panels/sfxr.rs |
| F-INS-016 | SID (Commodore 64) | feito |  | lm-plugins-sid/src/lib.rs,lm-gui-plugins/src/panels/sid.rs |
| F-INS-017 | SlicerT (fatiador de loops) | feito | painel `slicert` (516x400): onda da amostra com as fatias (clique cria, arrastar move entre as vizinhas, Shift ou botão direito apaga, as pontas ficam), limiar, fade-out, "achar fatias" (`slicert.analyze`, que também roda sozinha ao trocar a amostra), BPM original, ajuste, laço, sincronia, limpar e abrir amostra. Fica de fora o seeker (mini mapa de zoom), o destaque da fatia tocando e o botão de copiar o padrão MIDI | lm-gui-plugins/src/panels/slicert.rs,lm-plugins-instruments/src/slicert.rs,lm-session/src/plugin_actions.rs |
| F-INS-018 | Mallets (STK: marimba, vibrafone, sinos…) | feito |  | lm-plugins-stk/src/lib.rs,lm-gui/src/screens/instrument.rs |
| F-INS-019 | TripleOscillator | feito |  | lm-plugins-tripleoscillator/src/lib.rs,lm-gui-plugins/src/panels/tripleoscillator.rs |
| F-INS-020 | Vibed (cordas vibrantes) | parcial | plugin registrado, só o editor genérico (botões); falta a GUI de desenhar a forma de onda de cada corda (campo shape, lista) | lm-plugins-instruments/src/vibed.rs,lm-gui/src/screens/instrument.rs |
| F-INS-021 | Watsyn (4 osciladores de onda desenhada) | parcial | plugin registrado, só o editor genérico (botões); falta a GUI de desenhar as 4 ondas A1/A2/B1/B2 (campos de lista) | lm-plugins-instruments/src/watsyn.rs,lm-gui/src/screens/instrument.rs |
| F-INS-022 | Xpressive (síntese por expressões) | feito | plugin `xpressive` sobre `lm-dsp::xpressive` e `lm-expr`, painel com o editor, a lâmpada de validade e os gráficos das ondas W1-W3 e das saídas O1/O2: o plugin publica o quadro de display (cinco bandeiras de erro e a curva de cada saída, uma nota A4 de meio segundo, I-9.5) | lm-plugins-instruments/src/xpressive.rs,lm-gui-plugins/src/panels/xpressive.rs |
| F-INS-023 | ZynAddSubFX | decidido: hospedado | decisão P-05: fora do processo, como plugin de terceiros (threads próprias e janela FLTK); não é portado para Rust (um porte completo não cabe e perderia para o original). Entra pela hospedagem de terceiros (P11) |  |
| F-INS-024 | Janela de "plugin não encontrado" e instrumento fantasma | feito |  | lm-doc/src/edit.rs,lm-gui/src/screens/instrument.rs,lm-gui/src/screens/effect.rs |

## F-HOST: Plugins de terceiros (hospedagem)

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-HOST-001 | VeSTige (instrumentos VST2) | feito | atendido pelo desenho novo: VST2 está cortado (decisão do dono); instrumentos de terceiros entram por CLAP, VST3, LV2 e LADSPA no processo auxiliar | lm-plugin-host/src/main.rs,lm-plugin-proxy/src/scan.rs |
| F-HOST-002 | Carla Rack e Carla Patchbay | feito | atendido pelo processo auxiliar (P-02): hospeda LV2, CLAP, VST3 e LADSPA fora do processo; VST2 cortado | lm-plugin-host/src/main.rs,lm-session/src/external.rs,lm-plugin-proxy/src/hosted.rs |
| F-HOST-003 | Instrumentos LV2 | feito |  | lm-plugin-host/src/lv2.rs,lm-sys-lilv/src/instance.rs,lm-session/src/external.rs |

## F-FX: Efeitos e ferramentas

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-FX-001 | Controles comuns de todo efeito | feito |  | lm-gui/src/screens/effect.rs,lm-doc/src/schemas.rs |
| F-FX-002 | Cadeia de efeitos | feito |  | lm-gui/src/screens/effect.rs,lm-compile/src/rack.rs,lm-gui/src/pages/mixing.rs |
| F-FX-003 | Navegador de efeitos | parcial | o navegador lista os plugins (arrastar/clicar para o canal), mas sem as categorias de efeito (EQ, Dinâmica, Atraso e reverb…), sem nível por grupo do Guitarix nem ícones por categoria; só agrupa synth/effect e terceiros | lm-gui/src/screens/browser.rs,lm-session/src/port.rs |
| F-FX-004 | AmpAndIr (modelo de amplificador NAM + resposta ao impulso) | feito | painel `ampandir` no estilo próprio (a unidade de rack do Guitarix, I-22: o legado só tinha botões soltos): abrir o modelo `.nam` (`plugin.load_into_field`) e a IR (qualquer áudio), o nome de cada arquivo, tirar cada um, ganhos de entrada e saída | lm-gui-plugins/src/panels/ampandir.rs,lm-plugins-ampandir/src/lib.rs |
| F-FX-006 | AmplifikationLite (clone do amplificador Kuassa, do fork) | falta | nenhum crate AmplifikationLite; nada do DSP nem do painel |  |
| F-FX-007 | BassBooster | feito |  | lm-plugins-effects/src/bass_booster.rs,lm-gui-plugins/src/panels/effects.rs |
| F-FX-008 | Bitcrush | feito |  | lm-plugins-effects/src/bitcrush.rs,lm-gui-plugins/src/panels/effects.rs |
| F-FX-009 | Compressor | parcial | plugin registrado e editor genérico (botões); faltam na janela os medidores de entrada, saída e redução de ganho | lm-plugins-effects/src/compressor.rs,lm-gui/src/screens/effect.rs |
| F-FX-010 | CrossoverEQ | feito |  | lm-plugins-effects/src/crossover_eq.rs,lm-gui/src/screens/effect.rs |
| F-FX-011 | Delay | feito |  | lm-plugins-effects/src/delay.rs,lm-gui/src/screens/effect.rs |
| F-FX-012 | Dispersion (all-pass de ordem alta) | feito |  | lm-plugins-effects/src/dispersion.rs,lm-gui/src/screens/effect.rs |
| F-FX-013 | DualFilter | feito |  | lm-plugins-effects/src/dual_filter.rs,lm-gui-plugins/src/panels/effects.rs |
| F-FX-014 | DynamicsProcessor (curva de dinâmica desenhada) | feito |  | lm-plugins-effects/src/dynamics_processor.rs,lm-gui-plugins/src/panels/shapers.rs |
| F-FX-015 | Eq (equalizador de 8 bandas) | parcial | plugin registrado e editor genérico (botões das 8 bandas); falta o gráfico da curva e o analisador/medidores na janela | lm-plugins-effects/src/eq.rs,lm-gui/src/screens/effect.rs |
| F-FX-016 | Flanger | feito |  | lm-plugins-effects/src/flanger.rs,lm-gui-plugins/src/panels/effects.rs |
| F-FX-017 | FrequencyShifter (deslocador de frequência e phaser "barberp | feito |  | lm-plugins-effects/src/frequency_shifter.rs,lm-gui-plugins/src/panels/frequencyshifter.rs |
| F-FX-018 | GranularPitchShifter | feito |  | lm-plugins-effects/src/granular_pitch_shifter.rs,lm-gui-plugins/src/panels/granularpitchshifter.rs |
| F-FX-019 | Guitarix (pedais, amplificadores e gabinetes do Guitarix, em | feito |  | lm-plugins-guitarix/src/lib.rs,lm-gui/src/screens/effect.rs |
| F-FX-020 | LOMM (compressor multibanda para cima e para baixo, estilo O | parcial | painel feito, mas sem os medidores ao vivo das 3 bandas nem as zonas de limiar arrastáveis (declarado não portado no painel) | lm-plugins-effects/src/lomm.rs,lm-gui-plugins/src/panels/lomm.rs |
| F-FX-021 | MultitapEcho | feito |  | lm-plugins-effects/src/multitap_echo.rs,lm-gui-plugins/src/panels/multitapecho.rs |
| F-FX-022 | Oscilloscope | parcial | plugin e painel feitos (histórico de 5 s, janela, offset, escala, estéreo, pausa; o áudio passa sem mudar); falta arrastar o gráfico para rolar o offset e o zoom da roda que mantém o ponto sob o cursor (a roda muda o tamanho da janela) | lm-plugins-effects/src/oscilloscope.rs,lm-gui-plugins/src/panels/oscilloscope.rs |
| F-FX-023 | Peak Controller (seguidor de envelope que vira controlador) | obsoleto | virou o seguidor de pico, um modulador (decisão de 26/11): mede a saída de uma faixa ou canal com a matemática do legado (RMS, limiar, ganho, ataque e decaimento) | lm-render/src/modulation.rs |
| F-FX-024 | ReverbSC | feito |  | lm-plugins-effects/src/reverb_sc.rs,lm-gui-plugins/src/panels/effects.rs |
| F-FX-025 | SlewDistortion (distorção de 2 bandas e limitador de slew) | feito |  | lm-plugins-effects/src/slew_distortion.rs,lm-gui/src/screens/effect.rs |
| F-FX-026 | Spectrum Analyzer | parcial | plugin e painel feitos (FFT em thread própria com anel sem trava, 7 tamanhos, 4 janelas, eixos linear/log, faixas, média, pico/referência congelada, cascata, os 8 botões avançados; o áudio passa sem mudar); falta a leitura do cursor (frequência e nível sob o mouse) e os dois botões de resolução de pontos não têm efeito (o desenho é por pixel) | lm-plugins-effects/src/spectrum_analyzer.rs,lm-dsp/src/fft.rs,lm-gui-plugins/src/panels/spectrumanalyzer.rs |
| F-FX-027 | StereoEnhancer | feito |  | lm-plugins-effects/src/stereo_enhancer.rs,lm-gui/src/screens/effect.rs |
| F-FX-029 | Vectorscope | parcial | plugin e painel feitos (escala logarítmica, linhas ou pontos, traço que esmaece, grade e L/R; o áudio passa sem mudar); falta o zoom da roda (20 a 1000 %) e o seletor de cor do duplo clique. O legado deste repositório só tem essas duas opções (não há "alta qualidade" nem persistência) | lm-plugins-effects/src/vectorscope.rs,lm-gui-plugins/src/panels/vectorscope.rs |
| F-FX-030 | WaveShaper (curva de transferência desenhada) | feito |  | lm-plugins-effects/src/wave_shaper.rs,lm-gui-plugins/src/panels/shapers.rs |
| F-FX-031 | Efeitos LADSPA | feito | LADSPA fora do processo (P-03): `lm-plugin-host/src/ladspa.rs`; os conjuntos de LADSPA do legado não foram portados | lm-plugin-host/src/ladspa.rs,lm-plugin-proxy/src/scan.rs |
| F-FX-032 | Efeitos LV2 | feito | pastas por formato e varredura na página Plugins das configurações (`plugins.folders.<formato>`, ação `plugin.scan`) | lm-plugin-host/src/lv2.rs,lm-session/src/external.rs,lm-gui/src/screens/settings.rs |
| F-FX-033 | Efeitos VST2 | feito | atendido pelo desenho novo: VST2 está cortado (decisão do dono) | lm-plugin-host/src/main.rs |
| F-FX-036 | Categorias de plugins | parcial | o descritor só distingue instrumento (synth) de efeito (effect); faltam as categorias (subtrativo, samplers, baterias, chiptune; EQ, dinâmica…), cor e sigla por plugin | lm-session/src/port.rs,lm-port/src/browse.rs |
| F-FX-037 | Tap Tempo | falta | nenhum Tap Tempo (nem janela, nem ação, nem botão no transporte) |  |
| F-FX-038 | Importar MIDI | feito |  | lm-session/src/midifile.rs,lm-gui/src/screens/files.rs |
| F-FX-039 | Exportar MIDI | feito |  | lm-session/src/midifile.rs,lm-gui/src/screens/project.rs |

## F-CTL: Automação e controladores

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-CTL-001 | Rack de controladores | obsoleto | substituído pelos moduladores junto do dono do parâmetro (decisão de 26/11); sem rack do projeto | lm-doc/src/schemas.rs |
| F-CTL-002 | LFO controlador | parcial | modulador LFO: onda (inclui a do usuário), velocidade com sync ao tempo, multiplicador, fase e profundidade, todos automatizáveis e mapeáveis; vira parâmetros somados à automação, determinístico pela amostra da linha do tempo, na grade de 32 amostras; falta escolher o arquivo da onda do usuário na tela | lm-render/src/modulation.rs,lm-render/src/renderer.rs,lm-compile/src/core.rs |
| F-CTL-003 | Peak controller | obsoleto | ver F-FX-023 |  |
| F-CTL-004 | Ligar um controle a um controlador ou ao MIDI (MIDI learn) | parcial | MIDI learn e janela de ligação prontos; o "controlador do projeto" saiu: ligar um controle a um modulador é "Modular…" no menu (editor compacto) | lm-midimap/src/learn.rs,lm-gui/src/screens/midi_binding.rs,lm-gui/src/screens/modulator.rs |
| F-CTL-005 | Menu de todo controle | parcial | há padrão, copiar, colar, MIDI e linear/log; faltam tirar ligações com outros controles, conectar a controlador e escolher a nota do sync | lm-gui-plugins/src/panel.rs,lm-gui/src/screens/instrument.rs |
| F-CTL-006 | Gestos em todo controle | parcial | arrastar, roda, duplo clique, padrão e um desfazer feitos; Shift+arrasto do valor feito; falta Ctrl+arrasto para ligar dois controles (P-04, controle compartilhado não existe) e soltar em faixa de automação | lm-gui/src/screens/instrument.rs,lm-gui-plugins/src/panel.rs |
| F-CTL-007 | Automação em clipes na linha do tempo | feito |  | lm-gui/src/screens/automation.rs,lm-gui/src/pages/composition.rs |
| F-CTL-008 | Gravar automação | feito |  | lm-session/src/midi/automation.rs,lm-gui/src/screens/automation.rs |
| F-CTL-009 | A mão vence a automação | feito |  | lm-session/src/midi/automation.rs,lm-gui/src/screens/automation.rs |

## F-PRJ: Projeto e arquivos

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-PRJ-001 | Novo projeto | feito |  | lm-gui/src/screens/new_project.rs,lm-session/src/actions.rs |
| F-PRJ-002 | Modelos | parcial | só os modelos do usuário (pasta templates); sem modelos de fábrica (P-09, conversor à parte) e sem "salvar como modelo padrão" | lm-session/src/core_actions.rs,lm-gui/src/screens/new_project.rs |
| F-PRJ-003 | Abrir projeto | feito |  | lm-gui/src/screens/project.rs,lm-session/src/session.rs |
| F-PRJ-005 | Recusar projeto perigoso | feito |  | lm-session/src/session.rs,lm-gui/src/screens/settings.rs |
| F-PRJ-006 | Guardar o que não entende | feito |  | lm-doc/src/document.rs,lm-doc/src/edit.rs |
| F-PRJ-007 | Salvar e Salvar como | feito |  | lm-session/src/session.rs,lm-gui/src/screens/project.rs |
| F-PRJ-008 | Salvar como nova versão | feito |  | lm-gui/src/screens/project.rs |
| F-PRJ-010 | Cópia de segurança ao salvar | feito | o salvar é atômico; cada salvar guarda o arquivo que substitui em `backup/projeto-NNNN.json` (numerado, sem relógio) e apaga os mais velhos além de `backup.kept`; `backup.onSave` desliga (as duas opções já estavam na tela); falha da cópia vira aviso e o salvar segue; o original migrado (.bak) continua | lm-session/src/journal.rs,lm-session/src/folder.rs |
| F-PRJ-012 | Descartar as conexões MIDI ao salvar | feito | aberto: P-36 respondido; nada a fazer | lm-session/src/midi/devices.rs |
| F-PRJ-013 | Juntar as mídias ao salvar | feito |  | lm-session/src/core_actions.rs,lm-gui/src/screens/files.rs |
| F-PRJ-015 | Recentes | feito |  | lm-session/src/prefs.rs,lm-gui/src/screens/project.rs |
| F-PRJ-016 | Reabrir o último projeto na partida | feito | `general.reopenLast`: a partida abre o último projeto (gui-app e `Preferences::last_project`) | lm-gui/src/screens/settings.rs,lm-gui-app/src/main.rs |
| F-PRJ-017 | Autosave e recuperação | feito |  | lm-session/src/journal.rs,lm-gui/src/screens/project.rs |
| F-PRJ-018 | Notas do projeto | feito | campo `notes` na raiz (esquema 5, opcional, até 64 KiB), editado na janela do projeto (Notas do projeto, ação `file.notes`, sem tecla padrão), um passo de desfazer ao fechar | crates/lm-doc/src/schemas.rs,crates/lm-gui/src/screens/project.rs |
| F-PRJ-019 | Importar MIDI | feito | importa notas, andamento/compasso, CC7 (volume), CC10 (pan), pitch bend, pressão e os outros CC (faixas de controle do item), banco (CC0/CC32) e program (o primeiro escolhe o preset, os seguintes são faixas `programChange`), canal 10 = banco 128; o instrumento é um Sampler no SoundFont padrão (sem ele, o sintetizador provisório) | lm-session/src/midifile.rs,lm-gui/src/screens/files.rs |
| F-PRJ-020 | Importar arquivos de áudio | feito |  | lm-gui/src/screens/files.rs,lm-session/src/core_actions.rs |
| F-PRJ-021 | Exportar MIDI | feito | exporta SMF tipo 1 com mapa de andamentos, notas e faixas de controle; um Sampler em SF2/GIG grava banco (CC0/CC32) e program no início, e o banco 128 vai ao canal 10 | lm-session/src/midifile.rs,lm-gui/src/screens/palette.rs |
| F-PRJ-022 | Exportar áudio (assistente) | feito |  | lm-session/src/export.rs,lm-gui/src/screens/export.rs |
| F-PRJ-023 | Exportar uma faixa, um clipe, uma região | feito | faixa (na tela de exportar e no menu da faixa), clipe (menu do clipe) e região (menu da régua) | lm-gui/src/screens/export.rs,lm-gui/src/pages/composition.rs |
| F-PRJ-024 | Exportar as faixas (stems) | feito |  | lm-session/src/export.rs,lm-gui/src/screens/export.rs |
| F-PRJ-025 | Exportar o trecho: rápido ou exato | feito |  | lm-session/src/export.rs,lm-gui/src/screens/export.rs |
| F-PRJ-026 | Exportar a amostra de um clipe | feito | ação `clip.export_sample {item, path}` (operação com cancelar): o trecho da amostra que o item toca, em FLAC 24 bits; menu do clipe de áudio "Exportar a amostra". Ganho, fades e comp não entram: é a amostra, não a renderização | crates/lm-session/src/export.rs,crates/lm-gui/src/pages/composition/edit.rs |
| F-PRJ-027 | Preset de efeito e de faixa | feito |  | lm-session/src/core_actions.rs,lm-gui/src/screens/effect.rs,lm-gui/src/screens/instrument.rs |

## F-CFG: Configurações

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-CFG-001 | Busca nas configurações | feito |  | lm-gui/src/screens/settings.rs |
| F-CFG-002 | Aplicar na hora e reinício | parcial | aplica na hora e avisa o que pede reinício, mas "Reiniciar agora" reabre o programa sem o projeto (as variáveis LM_OPEN/LM_THEME_NAME que ele define não são lidas pelo aplicativo) | lm-gui/src/screens/settings.rs,lm-gui-app/src/main.rs |
| F-CFG-010 | Geral | parcial | só o idioma e a taxa de projeto novo funcionam; reabrir o último projeto, perguntar antes de apagar faixa/canal, prévias até o fim, áudio longo vira faixa, mesa analógica não têm consumidor; o autosave grava backup.autosaveMinutes mas o núcleo lê autosaveSeconds | lm-gui/src/screens/settings.rs,lm-session/src/prefs.rs,lm-session/src/session.rs |
| F-CFG-011 | Áudio | parcial | ALSA e CoreAudio funcionam; JACK desativado ("fase 2"), sem backend na sessão; bloco fixo em potências de 2 sem avisos | lm-gui/src/screens/settings.rs,lm-session/src/session.rs,lm-sys-jack/src/lib.rs |
| F-CFG-012 | MIDI | parcial | portas MIDI, papéis e latências funcionam (ALSA seq); a opção quantizar o gravado (recording.quantizeMidi) não tem consumidor; sem ALSA raw nem JACK MIDI | lm-gui/src/screens/settings.rs,lm-session/src/midi/devices.rs |
| F-CFG-013 | Gravação | parcial | calibração funciona e o formato vale; metronome.when/countInBars, volume e acento do metrônomo e recording.lanes (P-11) não têm consumidor no núcleo; pré-roll/contagem ficam no documento do projeto | lm-gui/src/screens/settings.rs,lm-session/src/prefs.rs,lm-compile/src/types.rs |
| F-CFG-014 | Plugins | parcial | aberto: P-01 respondida; a aba lista "não carregados desde uma queda" e duas opções (janelas por cima, uma janela de instrumento) sem consumidor, e a lista usa plugins.notLoaded enquanto o núcleo grava plugins.blocked; sem pastas/varredura de VST | lm-gui/src/screens/settings.rs,lm-session/src/external.rs |
| F-CFG-015 | Pastas | parcial | pastas de plugins por formato (LV2, CLAP, VST3, LADSPA) e varredura feitas na página Plugins; VST2 cortado; pasta de GIG não existe (o Sampler único abre qualquer arquivo e o navegador usa `sampleFolders`); SoundFont padrão (folders.soundfont) sem consumidor e "pastas permitidas" grava folders.allowed enquanto o núcleo lê allowedFolders (do agente das configurações) | lm-gui/src/screens/settings.rs,lm-session/src/core_actions.rs |
| F-CFG-016 | Aparência | parcial | tema e idioma funcionam; dicas, cabeçalhos compactos, nomes das notas, seguir o cursor, uma janela de instrumento por vez e janelas destacadas não têm consumidor; faltam marcas do fader, rolagem suave, modo do loop e pasta do tema | lm-gui/src/screens/settings.rs,lm-gui/src/ui/theme.rs |
| F-CFG-017 | Teclado e atalhos | feito |  | lm-gui/src/screens/settings.rs,lm-gui/src/actions.rs |
| F-CFG-018 | Avançado | parcial | aberto: P-07 respondida (cauda do processador; memória de tempo real obsoleta); a opção "limpar áudio corrompido" (advanced.sanitize) não tem consumidor | lm-gui/src/screens/settings.rs,lm-render/src/exec.rs |
| F-CFG-019 | Primeira execução e pasta de trabalho | parcial | o modo portátil (arquivo portable) existe só na localização das preferências; sem pergunta de primeira execução nem criação das pastas padrão e sem LINUXMUSIC_CONFIG na GUI de partida; aberto na decisão P-35 respondida | lm-session/src/prefs.rs,lm-gui-app/src/main.rs |

## F-BRW: Navegador

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-BRW-001 | Páginas do navegador | feito |  | lm-port/src/browse.rs,lm-session/src/port.rs,lm-gui/src/screens/browser.rs |
| F-BRW-002 | Pastas do usuário e de fábrica juntas | feito | pastas de fábrica (`LINUXMUSIC_FACTORY`, `factory` ao lado do programa, `../share/linuxmusic/factory`) juntam-se às do usuário pelo nome (amostras, projetos, presets); `tools/factory.sh` empacota | lm-session/src/port.rs,lm-session/src/browser.rs |
| F-BRW-003 | Tipos reconhecidos | feito | `browser::file_type`: áudio, MIDI, sf2/sf3, sfz, pat, gig, nam, ds, voc, lv2, vst3, clap; `info.type`; a tabela `OPENED_BY` liga o tipo ao plugin (sampler) | lm-session/src/browser.rs |
| F-BRW-004 | Busca | feito | índice em segundo plano (Operation, incremental por mtime, em `browser-index.json` na pasta de dados) e busca instantânea: palavras, frases entre aspas, `type:`, `tag:`/`#`, `is:fav`; vista Índice com filtros no navegador. Falta: barra de progresso da indexação, ação para editar etiquetas do usuário | lm-session/src/index.rs,lm-gui/src/screens/browser.rs |
| F-BRW-005 | Prévia | parcial | áudio, SoundFont, preset (instrumento) e MIDI (num SoundFont) renderizados num worker e tocados pelo motor de prévia; o MIDI usa `defaultSoundFont` ou o primeiro do índice; prévia de efeito e "até o fim" faltam | lm-session/src/preview.rs,lm-session/src/port.rs |
| F-BRW-006 | Enter e duplo clique | parcial | Enter envia plugin, preset, projeto, amostra/MIDI (faixa nova no cursor) e banco (instrumento que o abre); o Sampler ainda não existe, então o arquivo não é carregado nele; sem Shift+Enter | lm-gui/src/screens/browser.rs |
| F-BRW-007 | Arrastar do navegador | feito | soltar no arranjo ou numa faixa: áudio (item), MIDI (importa), preset (carrega na faixa), instrumento (troca) | lm-gui/src/screens/browser_drop.rs,lm-gui/src/pages/composition.rs |
| F-BRW-008 | Menu do arquivo | parcial | tem caminho, mostrar no gerenciador, favoritar e enviar à faixa ativa; faltam faixa de instrumento/sampler/amostra/SlicerT novos e abrir pasta no gerenciador | lm-gui/src/screens/browser.rs,lm-session/src/core_actions.rs |
| F-BRW-009 | Instrumentos | parcial | árvore por categoria (sintetizadores, chip, samplers, terceiros); sem subplugins nem descrição na dica | lm-session/src/port.rs,lm-session/src/browser.rs |
| F-BRW-010 | Efeitos | feito | categorias de efeito (dinâmica, filtros, distorção, tempo, estéreo, afinação, terceiros) | lm-session/src/port.rs |
| F-BRW-011 | Miniaturas de onda | feito | a prévia mostra a onda (serviço de picos) na linha de baixo | lm-gui/src/screens/browser.rs |
| F-BRW-012 | Conteúdo de fábrica | parcial | as pastas de fábrica são lidas e indexadas; as amostras do legado empacotam-se com `tools/factory.sh`; os presets de fábrica em JSON (conversor de desenvolvimento) ainda não existem | tools/factory.sh |

## F-GER: Outros

| Id | Funcionalidade | Estado | O que falta | Onde |
|---|---|---|---|---|
| F-GER-001 | Renderizar pela linha de comando | parcial | lm-headless render só grava WAV com --from/--to/--tail; faltam rendertracks, formato flac/ogg/mp3, float, bitrate, loop, taxa; a exportação completa está na ação project.export | lm-headless/src/main.rs,lm-session/src/export.rs |
| F-GER-003 | Variáveis de ambiente | parcial | aberto: P-35 respondida; só LINUXMUSIC_CONFIG e o arquivo portable (preferências); sem pasta de dados extra, captura de tela, exclusão/bloqueio de plugins por variável | lm-session/src/prefs.rs |
| F-GER-004 | Relatório de queda | parcial | manipulador de queda (Linux) grava a nota do plugin suspeito em backup/suspeito.json e o modo seguro avisa; sem crash-report.txt com backtrace e sem manipulador fora do Linux | lm-session/src/crashnote.rs,lm-sys-linux/src/crash.rs |
| F-GER-005 | Registro de erros | parcial | a fila de avisos junta repetidos e o headless imprime; não há logs/errors.log em arquivo nem registro de hora e lugar | lm-session/src/notices.rs,lm-headless/src/main.rs |
| F-GER-006 | Avisos ao usuário | feito |  | lm-session/src/notices.rs,lm-gui/src/screens/notices.rs |
| F-GER-008 | Traduções | feito | aberto: P-08 respondida (pt e en na 1.0; conversão dos outros idiomas depois); só pt-BR e en existem | lm-gui/src/ui/i18n.rs,lm-gui/locales/pt-BR/screens.txt |
| F-GER-009 | Sobre | parcial | mostra versão, ambiente (copiável), idioma, tema e licença; faltam autores, colaboradores e tradutores | lm-gui/src/screens/about.rs |
| F-GER-010 | Escolher cor | parcial | a paleta de 48 cores existe só no menu de cor dos canais da mesa; sem seletor de cor para faixas e sem o diálogo geral | lm-gui/src/pages/desk.rs,lm-gui/src/pages/mixing.rs |
| F-GER-011 | Diálogo de arquivo | parcial | usa os diálogos nativos de arquivo; sem atalhos próprios (Mesa, Downloads, Música, pasta de trabalho, volumes) nem última pasta lembrada pelo programa | lm-gui/src/screens/files.rs |
| F-GER-012 | Partida | parcial | abre o projeto dado na linha de comando ou mostra os recentes; "reabrir o último projeto" não tem consumidor; sem tela de abertura (intencional) | lm-gui-app/src/main.rs,lm-gui/src/screens/project.rs |
| F-GER-013 | Reiniciar pelo programa | parcial | "Reiniciar agora" salva e relança o programa, mas sem reabrir o projeto (nenhum argumento de projeto e as variáveis LM_OPEN/LM_THEME_NAME não são lidas) | lm-gui/src/screens/settings.rs,lm-gui-app/src/main.rs |
| F-GER-015 | Ajuda sobre o que cada coisa faz | parcial | aberto: P-34 respondida (manual próprio pt/en); só existem dicas e Sobre; o manual não existe | lm-gui/src/screens/about.rs |

## O que falta para a 1.0, em pacotes de trabalho

Cada pacote junta o que mexe nos mesmos arquivos, para ir a um agente só. Em ordem de prioridade: primeiro o que bloqueia
o uso básico (gravar, tocar, ouvir), depois o que é visível e comum, por último o que é cauda.

### P1. Gravação de ponta a ponta (a prioridade do usuário)
Arquivos: `lm-session` (session.rs, telemetria), `lm-render` (telemetry.rs, engine.rs), `lm-io` (record.rs, take.rs), `lm-gui` (pages/transport.rs, pages/composition.rs).
- Telemetria sem produtor: `Telemetry.inputs` (medidor de entrada, F-REC-009), `Telemetry.recording` (onda ao vivo, F-REC-068), `counting_in` (F-REC-026).
- Metrônomo que soa: a sessão nunca preenche `metronome_sounds`/`metronome_output` (F-REC-024/025); contagem muda; `metronome.when/countInBars/volume/accent` sem consumidor (F-CFG-013).
- Pré-roll sem consumo (F-REC-023), parar volta ao ponto de partida (F-REC-020), sair do punch com Gravar (F-REC-021), auto-punch fechando o item só na região (F-REC-022).
- Gravar em loop: `commit_recording` ignora os cortes, sem camadas novas por volta (F-REC-040/041/043, `recording.lanes` F-REC-042), takes presos ao andamento (F-REC-046), lacunas de xrun no item (F-REC-084).
- Aviso de armada sem entrada (F-REC-004), entrada estéreo no menu (F-REC-002), menu do botão armar: overdub/substituir/camadas e quantizar (F-TRK-034, F-REC-101/102, F-CFG-012), gravar MIDI no clipe do editor (F-REC-029, F-NOT-018), gravar com efeitos / gravar a saída (F-REC-011), chave de baixa latência na GUI (F-REC-010), ajuste manual por dispositivo (F-REC-082), aviso do re-render de andamento com progresso (F-REC-090).

### P2. Som ao vivo: teclado e notas da GUI para o instrumento
Arquivos: `lm-session/src/port.rs` (caminho de nota ao vivo), `lm-gui/src/screens/{instrument,piano_roll,steps}.rs`.
- Feito (P2): o caminho de nota ao vivo (`CorePort::note_on/note_off/all_notes_off`, I-15): teclado da janela do instrumento (F-IJN-021), teclado do computador (F-IJN-022, F-NOT-023, F-JAN-012), teclado do piano roll (F-NOT-015), nota ouvida ao desenhar e mover, imagens dos kits do AvlDrums e DrumMachines. Falta: tocar o clipe em loop (F-NOT-017, F-AUT-011).
- Feito (P2): gravar por passos pelo núcleo (F-PAS-001). Falta: abrir o editor sem clipe (F-NOT-022).
- Lembrar aba, tamanho e posição da janela do instrumento (F-IJN-005); velocidade base consumida (F-IJN-017); `tuning.import_scale/import_keymap` só existem no fake (F-IJN-020).

### P3. Casca: menus, janelas, transporte, avisos
Arquivos: `lm-gui/src/workspace.rs`, `actions.rs`, `pages/transport.rs`, `screens/{notices,about}.rs`.
- Feito: barra de menus, Tap Tempo, atalhos de Ver, CPU e OUT, mudo do master, editor da fórmula de compasso, mensagens flutuantes, janelas destacadas, manual pt/en.
- Falta: digitar o andamento e menu do clique (F-JAN-046), Recentes e Novo a partir de modelo como submenus (F-JAN-020), Escalas no menu Editar (F-JAN-021), Notas do projeto (F-JAN-022).
- Título da janela (F-JAN-007), pergunta ao Abrir (F-JAN-009), rotina ao fechar (F-JAN-010), piscar do Salvar no autosave (F-JAN-008), dicas ligáveis (F-JAN-011), recolher a área (F-JAN-005), janelas flutuantes/destacar (F-JAN-006), dois navegadores (F-JAN-004), Configurações como página e na primeira execução (F-JAN-003).
- Ajuda: Sobre com autores (F-GER-009); seletor de cor geral (F-GER-010); diálogo de arquivo (F-GER-011).

### P4. Configurações sem consumidor e partida
Arquivos: `lm-gui/src/screens/settings.rs`, `lm-session/src/prefs.rs`, `lm-gui-app`.
- Preferências que a GUI grava e ninguém lê: `general.reopenLast` (F-PRJ-016, F-GER-012), `interface.tooltips`, `followCursor`, `recording.quantizeMidi`, `advanced.sanitize` (F-CFG-018, F-MIX-026 ligado?), `folders.soundfont`, pastas permitidas, opções de janela (F-CFG-010, 013..016, 018).
- Reiniciar reabrindo o projeto (F-CFG-002, F-GER-013), primeira execução, pasta de trabalho e modo portátil (F-CFG-019), variáveis de ambiente (F-GER-003), JACK em `lm-io` e na sessão (F-REC-125, F-CFG-011, depois do ALSA), JACK MIDI e CoreMIDI (F-REC-130).
- Relatório de queda com backtrace e `logs/errors.log` (F-GER-004/005); `lm-headless render` com stems, formatos, loop (F-GER-001).

### P5. Arranjo, régua e andamentos
Arquivos: `lm-gui/src/pages/composition.rs`, `pages/composition/edit.rs`, `pages/marks.rs`.
- Mapa de andamentos editável: criar/mover/remover pontos, rampas, fórmulas, mudar o andamento no cursor (F-REG-013/014).
- Seguir o cursor, parar volta ao início, arrastar o cursor, cursor vermelho ao gravar (F-REG-003/004/005/010); selecionar clipes pela régua (F-REG-008); modos do loop (F-REG-007); cor e criação de marcadores e regiões (F-REG-011/012).
- Grade proporcional e zoom lembrado (F-ARR-008/009), Ctrl+0 (F-ARR-007), criar pasta (F-ARR-010), soltar do navegador/limiar áudio longo (F-ARR-011), trazer à frente depois de abrir (F-ARR-013), fundo com linhas de compasso (F-ARR-014).
- Piano roll com a fórmula de compasso do mapa (F-NOT-013), travar na nota, tercinas 1/96 e 1/192 (F-NOT-012), strum curvo (F-NOT-006), empurrar notas com Shift+Ctrl (F-NOT-008), Alt+setas (F-NOT-009), área de transferência do sistema (F-NOT-019, F-CLP-010).

### P6. Faixas e clipes: gestos que faltam
Arquivos: `lm-gui/src/pages/composition.rs`, `pages/composition/edit.rs`, `lm-session/src/clips.rs`.
- Arrastar faixa para mover/copiar e a alça do número (F-TRK-020/029), arrastar borda para altura e largura, altura por faixa (F-TRK-001/030), clique no nome (F-TRK-027), indicador de atividade (F-TRK-028), soltar sobre a faixa (F-TRK-032), clique na linha vazia de áudio (F-TRK-033), exportar a faixa como áudio e cores (F-TRK-031).
- Clipe: mover para outra faixa (F-CLP-004), Ctrl+arrasto copia (F-CLP-005), divisão destrutiva (F-CLP-007), menu: exportar, cor, tamanho automático, ouvir camada (F-CLP-008), nome e cor editáveis (F-CLP-001/024), tamanho automático no piano roll (F-CLP-025), cortar o que sobra (F-CLP-022).
- Áudio: curvas de fade/crossfade com tensão (F-CLP-041, F-CLP-066, P-30), inverter (F-CLP-042, P-31), exportar a amostra (F-CLP-043, F-PRJ-026), normalizar (F-CLP-065), amostras na grade do padrão / instrumento de pads (F-PAD-004, P-23).

### P7. Navegador
Arquivos: `lm-gui/src/screens/browser.rs`, `lm-session/src/browser.rs`, `lm-port`.
- Categorias de plugin e de efeito (F-BRW-009/010, F-FX-003, F-FX-036), tipos reconhecidos (sf2, pat, gig, nam, ds, lv2…) (F-BRW-003), busca com frases (F-BRW-004), prévia de SoundFont, preset e MIDI (F-BRW-005), Enter e arrastar para faixa/arranjo (F-BRW-006/007), menu do arquivo (F-BRW-008), miniaturas de onda (F-BRW-011), pastas de fábrica (F-BRW-002), conteúdo de fábrica e conversor (F-BRW-012, F-PRJ-002, P-09).

### P8. Controladores e automação (I-21)
Arquivos: `lm-doc/src/schemas.rs`, `lm-compile`, `lm-render`, `lm-gui`.
- Rack de controladores, LFO controlador, Peak Controller como controlador (F-CTL-001/002/003, F-FX-023), ligar a um controlador (F-CTL-004/005), Ctrl+arrasto para ligar dois controles (F-CTL-006); automação da onda da amostra (F-AUT-009).

### P9. Painéis dos instrumentos e efeitos (editor genérico no lugar do visual próprio, I-22)
Arquivos: `lm-gui-plugins/src/panels/*`, `lm-plugins-*`.
- Escolha de arquivo/amostra e desenho de ondas: AudioFileProcessor (F-INS-001), GigPlayer (F-INS-006), PatMan (F-INS-013), Sf2 Player (F-INS-014), SlicerT com fatias (F-INS-017), Vibed (F-INS-020), Watsyn (F-INS-021), BitInvader onda do usuário (F-INS-003), AmpAndIr .nam/IR (F-FX-004).
- Medidores e gráficos: Compressor (F-FX-009), Eq (F-FX-015), LOMM (F-FX-020).
- Envelopes/LFO com forma do usuário e escala dinâmica (F-IJN-010), soltar arquivo na janela (F-IJN-004).

### P10. Plugins que não existem
- Xpressive sobre o `lm-expr` (F-INS-022), ZynAddSubFX (F-INS-023), AmplifikationLite (F-FX-006, clone físico em outro projeto), Oscilloscope, Spectrum Analyzer, Vectorscope (F-FX-022/026/029), Peak Controller (junto com P8).

### P11. Hospedagem de terceiros
Arquivos: `lm-plugin-host`, `lm-plugin-proxy`, `lm-session/src/external.rs`, `settings.rs`.
- VST2 fora do processo (F-HOST-001, F-FX-033, P-01), LADSPA fora do processo (F-FX-031, P-03, F-HOST-002), configuração de pastas de plugins (F-FX-032, F-CFG-015).

### P12. Projeto e arquivos
Arquivos: `lm-session/src/{export,midifile,folder}.rs`, `lm-gui/src/screens/{project,export}.rs`.
- Notas do projeto (F-PRJ-018), cópia da versão anterior em `backup/` (F-PRJ-010), importar MIDI com pitch bend, CC, program e exportar com banco/programa (F-PRJ-019/021), exportar clipe (F-PRJ-023), modelo padrão (F-PRJ-002).

### P13. Mixer: acabamento
Arquivos: `lm-gui/src/pages/mixing.rs`, `lm-doc`, `lm-dsp/src/channel_strip.rs`.
- Largura do canal (F-MIX-034, P-20: decidida e sem campo), botão de pré-fader visível (F-MIX-020), medidor do canal com RMS/VU/K/LUFS (F-MIX-070), os dois retornos no projeto novo (F-MIX-023).
