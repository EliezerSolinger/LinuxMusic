# Instrumentos nativos: reclamações e limitações relatadas por usuários

Pesquisa feita em 17/11/2026. Fontes principais: issues do GitHub do LMMS (abertas e fechadas, buscadas por nome de instrumento), fórum lmms.io e documentação oficial. O Reddit (r/lmms) e o KVR quase não trouxeram nada específico dos instrumentos nativos; o grosso das queixas concretas está no GitHub.

Legenda da coluna "nosso código" (olhada rápida em `plugins/<Nome>/`, sem testar áudio):
- **parece presente**: o código ainda tem o comportamento relatado;
- **parece resolvido**: há correção visível no código ou a issue foi fechada com correção que o fork já inclui (o fork tem o upstream até o commit `f2a4cd2`, de 10/11/2026);
- **não verificado**: não deu para confirmar só lendo o código.

---

## Problemas que valem para vários instrumentos

- **Aliasing nos instrumentos nativos**: só o TripleOscillator usa wavetables com banda limitada. BitInvader, Kicker, Organic, Sfxr, Watsyn, Monstro, AFP (interpolação sinc existe, mas não é padrão), LB302 (parcial), Nescaline e SID ainda geram aliasing. Issue aberta [#6098](https://github.com/LMMS/lmms/issues/6098), relacionada à [#3605](https://github.com/LMMS/lmms/issues/3605) (oversampling nos plugins, aberta). Nosso código: **parece presente** (só TripleOscillator, LB302 e Vibed referenciam banda limitada ou oversampling).
- **Volumes padrão muito diferentes entre instrumentos**: o Nescaline sai cerca de 28 dB acima do patch inicial do Zyn, o Organic sai alto demais e o Zyn baixo demais. Issues abertas [#1207](https://github.com/LMMS/lmms/issues/1207) (desde 2014) e [#7852](https://github.com/LMMS/lmms/issues/7852). A proposta da #7852: Zyn de 64 para cerca de 100, osciladores do TripleOscillator e do Monstro de 33% para 10–15%, Organic de 100% para 50%, mudando só instrumentos novos. Nosso código: **parece presente**.
- **Falta modo monofônico/legato e portamento em todos os instrumentos** (só o LB302 tem slide). Issue aberta [#6516](https://github.com/LMMS/lmms/issues/6516). Nosso código: **parece presente** (não há modo mono no núcleo).
- **Pitch bend e pan por nota no piano roll não funcionam** em Zyn, LB302 e OpulenZ. Issue aberta [#3955](https://github.com/LMMS/lmms/issues/3955). Nosso código: **não verificado**.
- **Notas muito curtas vindas de MIDI às vezes não tocam**: toques curtos de pad (menos de 10 ms) são ignorados. Issue aberta [#6264](https://github.com/LMMS/lmms/issues/6264). Nosso código: **não verificado**.
- **Estalo no fim da nota em instrumentos "crus"** (sem release). Relatos no fórum: [Popping at end of note](https://lmms.io/forum/viewtopic.php?p=85183); upstream, a [#3246](https://github.com/LMMS/lmms/issues/3246) (TripleOscillator/Organic, fechada) e a [#3086](https://github.com/LMMS/lmms/issues/3086) (fechada). Nosso código: **parece resolvido** no núcleo; o que sobra depende do preset.
- **Interface dos instrumentos com tamanho fixo e sem HiDPI**: [#2510](https://github.com/LMMS/lmms/issues/2510) (aberta), [#6636](https://github.com/LMMS/lmms/issues/6636) (fechada) e [#3534](https://github.com/LMMS/lmms/issues/3534) (redesenho, aberta). Nosso código: **parece presente**.

---

## TripleOscillator

- **Os osciladores "saem de sincronia"**: uma nota longa com 3 dentes de serra varia de timbre como uma varredura, e o som fica menos "punchy". Issue aberta [#2047](https://github.com/LMMS/lmms/issues/2047), 55 comentários. Em parte isso é efeito do detune entre osciladores; falta uma opção de fase fixa ou aleatória por oscilador. Nosso código: **parece presente** (há só o ajuste de phase offset).
- **Aliasing que sobra com PM e FM**: a wavetable é escolhida pela frequência base, não pela frequência modulada ([#6098](https://github.com/LMMS/lmms/issues/6098)). A dúvida da [#8529](https://github.com/LMMS/lmms/issues/8529) (aberta) é se o FM funciona corretamente. Já resolvidos upstream: clipping no FM ([#6357](https://github.com/LMMS/lmms/issues/6357), fechada) e saída estranha com PM forte ([#3816](https://github.com/LMMS/lmms/issues/3816), fechada). Nosso código: **parece presente** (aliasing com PM/FM).
- **Ruído ou passa-alta dá resultados diferentes conforme as configurações de exportação**, inclusive entre GUI e linha de comando. Issue aberta [#5749](https://github.com/LMMS/lmms/issues/5749). Nosso código: **não verificado**.
- **Não tem unison/supersaw, nem mais de 3 osciladores** (pedido recorrente de quem compara com outros DAWs). Nosso código: **parece presente** (sem unison).

## AudioFileProcessor (AFP)

- **Segfault ao abrir a janela do AFP** em `SampleThumbnailRenderer::visualize`, uma regressão do refactor [#8596](https://github.com/LMMS/lmms/issues/8596). Issue aberta [#8647](https://github.com/LMMS/lmms/issues/8647). A correção proposta é o PR #8644. Nosso código: **provavelmente presente**: o fork está exatamente no commit `f2a4cd2` citado no relato e não tem o #8644.
- **O AFP não consegue carregar o preset `.xpf` que ele mesmo salva**: o diálogo só lista áudio. Issue aberta [#7492](https://github.com/LMMS/lmms/issues/7492). Nosso código: **parece presente** (`openAudioFile()` usa o filtro de áudio).
- **Falta fade-in/fade-out nos pontos de início, loop e fim** para tirar estalos de loops. Issue aberta [#5855](https://github.com/LMMS/lmms/issues/5855); há relatos no fórum de clique no loop e no início do sample ([1](https://lmms.io/forum/viewtopic.php?p=73229), [2](https://lmms.io/forum/viewtopic.php?p=3511)). Nosso código: **parece presente**.
- **Falta tocar o trecho depois do loop no note-off** (um quinto ponto, de "release"). Issue aberta [#5793](https://github.com/LMMS/lmms/issues/5793). Nosso código: **parece presente**.
- **Crash quando LFOs automatizam o início e o fim do loop e eles se cruzam**. Issue aberta [#2149](https://github.com/LMMS/lmms/issues/2149). Uma variante, o loopback passar do endpoint ([#4773](https://github.com/LMMS/lmms/issues/4773)), foi fechada. Nosso código: **não verificado**.
- **Clique ou pico no canal direito quando notas se sobrepõem** em padrões de bateria. Issue aberta [#7073](https://github.com/LMMS/lmms/issues/7073). Nosso código: **não verificado**.
- **Interface**: o knob de loopback continua visível com o loop desligado ([#7819](https://github.com/LMMS/lmms/issues/7819), aberta); há pedidos de uma interface no estilo do SlicerT ([#8037](https://github.com/LMMS/lmms/issues/8037)) e de editar samples do Sample Track no AFP ([#4971](https://github.com/LMMS/lmms/issues/4971)); usuários reclamam da falta de um editor de sample ([fórum](https://lmms.io/forum/viewtopic.php?p=63148)). Nosso código: **parece presente**.
- **Interpolação de alta qualidade não é o padrão**: a sinc custa CPU, e há pedido de configuração global ([#7429](https://github.com/LMMS/lmms/issues/7429), fechada). Nosso código: **não verificado**.

## BitInvader

- **Aliasing**: inclui `Oscillator.h` mas não usa as wavetables ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **"Normalize" deixa o volume ensurdecedor**, principalmente com a senoide. [#5867](https://github.com/LMMS/lmms/issues/5867) e [#6405](https://github.com/LMMS/lmms/issues/6405), ambas fechadas. Nosso código: **não verificado** (vale um teste de volume).
- **Desenho de onda limitado**: pedido de um pad X-Y e de ferramentas melhores ([#1994](https://github.com/LMMS/lmms/issues/1994), aberta); há também o comportamento ao aumentar o comprimento depois de desenhar ([#3797](https://github.com/LMMS/lmms/issues/3797), fechada). Nosso código: **parece presente**.

## Kicker

- **Glitch no note-off** ([#7225](https://github.com/LMMS/lmms/issues/7225), fechada), **crash** ([#7392](https://github.com/LMMS/lmms/issues/7392), fechada) e **automação de um knob que não volta ao reabrir o projeto** ([#6797](https://github.com/LMMS/lmms/issues/6797), fechada). Nosso código: **parece resolvido**.
- **Aliasing** (prioridade baixa, porque raramente toca agudo; [#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Falta camada de ruído/transiente e um envelope de pitch mais flexível**; os pedidos genéricos de sintetizador de bateria ([#1460](https://github.com/LMMS/lmms/issues/1460), aberta) e de envelope de pitch ([#1466](https://github.com/LMMS/lmms/issues/1466), aberta) citam o Kicker. Nosso código: **parece presente**.

## LB302

- **Não reseta o estado antes e depois de exportar**: duas exportações seguidas saem com fase diferente. Issue aberta [#7234](https://github.com/LMMS/lmms/issues/7234). Nosso código: **parece presente** (não há hook de exportação no `Lb302.cpp`).
- **A automação do RES não alcança a faixa toda**: o knob vai de 0 a 1,25, mas os pontos ficam travados em 1. Issue aberta [#7980](https://github.com/LMMS/lmms/issues/7980). Nosso código: **parece presente** (`m_vcfResKnob` vai de 0 a 1,25; a trava fica no editor de automação).
- **Pedido de oscilador com fase travada** (opção a free-run) para empilhar vários LB302 sem cancelamento de fase. Issue aberta [#8506](https://github.com/LMMS/lmms/issues/8506). Nosso código: **parece presente**.
- **Monofônico: com duas notas simultâneas, uma é escolhida "aleatoriamente"** ([#2565](https://github.com/LMMS/lmms/issues/2565), fechada como comportamento esperado). Ignorar a velocity foi corrigido ([#8286](https://github.com/LMMS/lmms/issues/8286), fechada). O filtro não se ajustava à taxa de amostragem ([#5615](https://github.com/LMMS/lmms/issues/5615), fechada). Nosso código: **parece resolvido** (o volume da nota já entra no VCA).
- **Sem pitch bend e pan por nota** ([#3955](https://github.com/LMMS/lmms/issues/3955)). Nosso código: **não verificado**.

## Monstro

- **Bug na largura de pulso mínima**: com o PW no mínimo, algumas notas soam erradas. Issue aberta [#3046](https://github.com/LMMS/lmms/issues/3046). Nosso código: **parece presente** (o mesmo `PW_MIN`, com clamp).
- **Aliasing**: usa oversampling, mas com filtragem ruim antes do downsampling ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Zumbido no Osc 2** ([#7334](https://github.com/LMMS/lmms/issues/7334), fechada). Nosso código: **parece resolvido**.
- **Volume padrão alto** ([#7852](https://github.com/LMMS/lmms/issues/7852)). Nosso código: **parece presente**.

## Nescaline (Nes)

- **Muito alto por padrão**, distorcendo em acordes ([#1207](https://github.com/LMMS/lmms/issues/1207), aberta). Nosso código: **parece presente**.
- **Saída ruidosa e sem de-aliasing**, ao contrário de outros emuladores de NES ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Falta documentação** dos parâmetros ([#6532](https://github.com/LMMS/lmms/issues/6532), aberta, vale também para FreeBoy e SID).
- **Crash ao trocar o instrumento quando há um controller ligado ao pitch** ([#8271](https://github.com/LMMS/lmms/issues/8271), aberta; problema do núcleo que apareceu com o Nes). Nosso código: **não verificado** (o commit `c094233c4` mexeu em conexões de controller).

## OpulenZ

- **Tom preso depois de parar a reprodução em loop**: só para mutando a faixa. Issue aberta [#2529](https://github.com/LMMS/lmms/issues/2529). Nosso código: **parece presente** (não há "all notes off" ao parar).
- **Ligar ou desligar o botão FM com notas tocando gera distorção**. Issue aberta [#3754](https://github.com/LMMS/lmms/issues/3754). Nosso código: **parece presente**.
- **Notas presas ao trocar o canal de saída no meio da nota** ([#8030](https://github.com/LMMS/lmms/issues/8030), aberta; o problema é o roteamento MIDI do núcleo, mas aparece no OpulenZ). Nosso código: **não verificado**.
- **O preset de fábrica `Organ_leslie.xpf` dispara um aviso de LADSPA** e não dá para arrastá-lo. Issue aberta [#7839](https://github.com/LMMS/lmms/issues/7839). Nosso código: **não verificado** (é o preset que está em `data/presets`).
- **Sem pitch bend por nota** ([#3955](https://github.com/LMMS/lmms/issues/3955)); **o knob Scale não fazia nada** ([#8306](https://github.com/LMMS/lmms/issues/8306), fechada). Há pedido de importar presets OPL de trackers ([#6163](https://github.com/LMMS/lmms/issues/6163), fechada). Nosso código: **não verificado**.

## Organic

- **Muito alto por padrão** (osciladores a 100%; [#7852](https://github.com/LMMS/lmms/issues/7852)). Nosso código: **parece presente**.
- **Aliasing** ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Interface pobre**: pedido de botões novos ([#968](https://github.com/LMMS/lmms/issues/968), aberta desde 2014); tooltips e rótulos errados ([#2459](https://github.com/LMMS/lmms/issues/2459) e [#2691](https://github.com/LMMS/lmms/issues/2691), fechadas). Nosso código: **parece presente** (sem filtro nem envelope próprio).

## FreeBoy

- **Ruído ao trocar de nota** mesmo com só o canal 2 ligado. Issue aberta [#3824](https://github.com/LMMS/lmms/issues/3824), 19 comentários. Nosso código: **não verificado**.
- **Offset DC causa estalos e clipping** com várias notas. Issue aberta [#6556](https://github.com/LMMS/lmms/issues/6556). Nosso código: **parece presente** (a saída do `Gb_Apu` vai direto para o buffer, sem bloqueio de DC).
- **Uso de CPU anormal** com certos ajustes, mesmo mutado. Issue aberta [#6144](https://github.com/LMMS/lmms/issues/6144). Nosso código: **não verificado**.
- **Canais estéreo bagunçados**: SO1 e SO2 não correspondem a esquerdo e direito, e o tooltip aparece duplicado. Issue aberta [#8043](https://github.com/LMMS/lmms/issues/8043). Nosso código: **não verificado**.
- **O canal de ruído deveria vir desligado por padrão** ([#4331](https://github.com/LMMS/lmms/issues/4331), fechada).

## Patman

- **Crash com um `.pat` ausente ou inválido** ao tocar a primeira nota. Issue aberta [#8163](https://github.com/LMMS/lmms/issues/8163). Nosso código: **parece resolvido** contra o crash (`selectSample` cai para um `Sample` vazio), mas o erro só vai para o `printf`, sem aviso ao usuário.
- **Arrastar um `.pat` para cima do Patman não troca o patch** (o código espera "samplefile", e o drop vem como "patchfile"). Issue aberta [#8065](https://github.com/LMMS/lmms/issues/8065). Nosso código: **não verificado**.
- **Ruído com polifonia** ([#2991](https://github.com/LMMS/lmms/issues/2991), fechada) e **som áspero sem patch carregado** ([#3909](https://github.com/LMMS/lmms/issues/3909), fechada).

## GIG Player

- **100% de CPU com certos `.gig`** (como o "RGD Kit 5"), confirmado por outro usuário. Issue aberta [#3825](https://github.com/LMMS/lmms/issues/3825). Nosso código: **não verificado**.
- **O pedal de sustain não segura a ressonância** de notas já soltas, ao contrário de um piano real. Issue aberta [#4517](https://github.com/LMMS/lmms/issues/4517). Nosso código: **parece presente** (o CC64 só aparece como dimensão do gig, sem lógica de sustain).
- **Falta ADSR próprio** no GIG e no SF2 ([#3233](https://github.com/LMMS/lmms/issues/3233), aberta). Nosso código: **parece presente**.
- **Esquecia os samples carregados** ([#2537](https://github.com/LMMS/lmms/issues/2537), fechada).

## Sf2Player

- **Não aceita arrastar um `.sf2` para cima de uma instância** para trocar o soundfont. Issue aberta [#7480](https://github.com/LMMS/lmms/issues/7480). Nosso código: **parece presente** (não há `dropEvent` no `Sf2Player.cpp`).
- **Campos de arquivo e patch pequenos e GUI confusa** ([#7482](https://github.com/LMMS/lmms/issues/7482) e [#7496](https://github.com/LMMS/lmms/issues/7496), abertas); há pedido de renomear para "Soundfont Player" ([#5361](https://github.com/LMMS/lmms/issues/5361), aberta) e de trocar rápido o soundfont padrão ([#8005](https://github.com/LMMS/lmms/issues/8005), aberta). Nosso código: **parece presente**.
- **Separar automaticamente as peças de bateria de um SF2 em várias faixas** ([#8269](https://github.com/LMMS/lmms/issues/8269), aberta). Nosso código: **parece presente**.
- **Falta ADSR próprio e não há aba ENV/LFO** ([#3233](https://github.com/LMMS/lmms/issues/3233); [manual](https://docs.lmms.io/user-manual/instruments/sf2-player)). Nosso código: **parece presente**.
- **Pitch bend "não faz nada"** para alguns usuários; no fórum, a sugestão é automatizar o knob de pitch ([fórum](https://lmms.io/forum/viewtopic.php?p=84131)). Nosso código: **não verificado** (há `fluid_synth_pitch_bend`; o problema provável é o alcance).
- **Bugs de carga em 2026**: soundfonts que deixam de carregar ([#8504](https://github.com/LMMS/lmms/issues/8504), [#8228](https://github.com/LMMS/lmms/issues/8228) e [#8209](https://github.com/LMMS/lmms/issues/8209)), crash no destrutor do knob ([#8224](https://github.com/LMMS/lmms/issues/8224)) e crash ao abrir a GUI antes de o soundfont terminar de carregar ([#8227](https://github.com/LMMS/lmms/issues/8227)). Todas estão fechadas. Nosso código: **parece resolvido** (o fork inclui o upstream até 10/11/2026), mas vale um teste.
- **Clone de uma faixa SF2 mostra o texto "Sf2 Player" nos steps** ([#4327](https://github.com/LMMS/lmms/issues/4327), aberta). Nosso código: **não verificado**.

## Sfxr

- **Aliasing; o código é muito particular** ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Supõe 44,1 kHz**: exportava errado em taxas altas ([#389](https://github.com/LMMS/lmms/issues/389), fechada). Nosso código: **parece presente em parte**: ainda há `currentSampleRate / 44100` no cálculo do pitch, vale revisar.
- **Desafinado** ([#1010](https://github.com/LMMS/lmms/issues/1010), fechada) e **saída fora da faixa** ([#1877](https://github.com/LMMS/lmms/issues/1877), fechada). Há pedido de knobs com mais resolução ([#5404](https://github.com/LMMS/lmms/issues/5404), fechada). Nosso código: **não verificado**.

## SID

- **Crash aleatório tocando a demo "Krem Kaakkuja"** em build debug. Issue aberta [#7638](https://github.com/LMMS/lmms/issues/7638). Nosso código: **não verificado** (vale rodar com ASan).
- **Falta documentação** ([#6532](https://github.com/LMMS/lmms/issues/6532)); há pedido de importar música SID ([#1722](https://github.com/LMMS/lmms/issues/1722), aberta).
- **Xrun na primeira nota, atraso antes da nota e cliques** ([#7461](https://github.com/LMMS/lmms/issues/7461), [#4113](https://github.com/LMMS/lmms/issues/4113) e [#5958](https://github.com/LMMS/lmms/issues/5958), fechadas). Nosso código: **parece resolvido**.

## SlicerT

- **A detecção de BPM erra** (por exemplo, 125 em vez de 140, ou 147 em vez de 140). Issue aberta [#8102](https://github.com/LMMS/lmms/issues/8102). Nosso código: **parece presente**: `findBPM()` só divide 240 pelo comprimento do sample e dobra ou corta até cair entre 100 e 200.
- **Faltam modos de loop por fatia** (loop, ping-pong, como no AFP). Issue aberta [#7364](https://github.com/LMMS/lmms/issues/7364). Nosso código: **parece presente** (`Sample::Loop::Off` fixo).
- **Problemas de layout depois do update de UI** ([#7510](https://github.com/LMMS/lmms/issues/7510), aberta, afetando também LV2) e **crash ao abrir arquivo em build debug no MSVC** ([#7072](https://github.com/LMMS/lmms/issues/7072), aberta). Nosso código: **não verificado**.
- **Observação do código**: o sync de BPM muda a velocidade por reamostragem, então o pitch muda junto com o tempo (não há time-stretch). Nosso código: **parece presente**.

## Mallets (STK)

- **Alguns presets (Uniform Bar, Tuned Bar, Glass, Tibetan Bowl) só soltam ruído**. Issue aberta [#7573](https://github.com/LMMS/lmms/issues/7573) (macOS). O histórico do problema é longo: sem som em Tubular Bells ([#2455](https://github.com/LMMS/lmms/issues/2455)), sons estranhos ao trocar o tipo ([#4269](https://github.com/LMMS/lmms/issues/4269)) e plugin quebrado no master ([#6464](https://github.com/LMMS/lmms/issues/6464)), todas fechadas. Nosso código: **não verificado** (depende da STK do sistema e dos rawwaves).
- **STK ausente nos pacotes** e caminho fixo em `/usr` ([#6489](https://github.com/LMMS/lmms/issues/6489), [#3227](https://github.com/LMMS/lmms/issues/3227) e [#5535](https://github.com/LMMS/lmms/issues/5535), fechadas). Nosso código: **não verificado**.
- **Problemas na fase de release do envelope** ([#4672](https://github.com/LMMS/lmms/issues/4672), aberta). Nosso código: **não verificado**.
- **O LMMS travava ao substituir o Mallets por outro instrumento** ([#4617](https://github.com/LMMS/lmms/issues/4617), fechada).

## Watsyn

- **Aliasing** ([#6098](https://github.com/LMMS/lmms/issues/6098)). Nosso código: **parece presente**.
- **Pop no envelope A2→A1** ([#5414](https://github.com/LMMS/lmms/issues/5414), fechada) e **mostrava o oscilador errado** ([#1526](https://github.com/LMMS/lmms/issues/1526), fechada). Nosso código: **parece resolvido**.
- **Editor de onda limitado** ([#1994](https://github.com/LMMS/lmms/issues/1994), aberta, compartilhada com o BitInvader).

## Vibed

- **Exportar a 192 kHz sai errado com Length = 1**. Issue aberta [#7221](https://github.com/LMMS/lmms/issues/7221). Nosso código: **parece presente e a causa está visível**: em `VibratingString.cpp`, `m_oversample = 2 * oversample / int(sampleRate / baseSampleRate)` dá **zero** com Length 1 a 192 kHz (2·1/4 = 0, em divisão inteira).
- **Notas aleatórias sumindo** ([#8219](https://github.com/LMMS/lmms/issues/8219), fechada) e **perda de dados ao salvar e reabrir** ([#6671](https://github.com/LMMS/lmms/issues/6671), fechada). Nosso código: **parece resolvido**.
- **Distorção com P > 0 em certas notas** ([#3663](https://github.com/LMMS/lmms/issues/3663), fechada) e **escala estranha do knob Length** ([#3703](https://github.com/LMMS/lmms/issues/3703), fechada).
- **Sem modo mono e sem portamento**, que seriam naturais para um sintetizador de cordas ([#6516](https://github.com/LMMS/lmms/issues/6516)).

## Xpressive

- **Crash por overflow**: `positiveFraction` converte para `int` valores finitos fora da faixa de `int`. Issue aberta [#5877](https://github.com/LMMS/lmms/issues/5877). Nosso código: **parece presente** (`ExprSynth.h:129`; só NaN e infinito são tratados).
- **Expressões que pararam de tocar no nightly** ([#8162](https://github.com/LMMS/lmms/issues/8162), fechada), **crash com `integrate`** ([#7468](https://github.com/LMMS/lmms/issues/7468), fechada) e **double delete e SEGV ao fechar** ([#4379](https://github.com/LMMS/lmms/issues/4379) e [#3951](https://github.com/LMMS/lmms/issues/3951), fechadas). Nosso código: **parece resolvido**.
- **O diálogo de ajuda se comporta mal** ([#5570](https://github.com/LMMS/lmms/issues/5570), fechada); há discussão sobre onde a ajuda deve ficar ([#5596](https://github.com/LMMS/lmms/issues/5596), aberta).

## ZynAddSubFX

- **"Show GUI" causa um corte grave de áudio** durante a reprodução, porque o Zyn roda em processo remoto. Issue aberta [#1825](https://github.com/LMMS/lmms/issues/1825); há também picos de CPU com a GUI aberta ([#3631](https://github.com/LMMS/lmms/issues/3631), aberta) e um pedido de embutir a UI como nos VSTs ([#8499](https://github.com/LMMS/lmms/issues/8499), aberta). Nosso código: **parece presente**.
- **O preset soa diferente com a GUI aberta** ([#7720](https://github.com/LMMS/lmms/issues/7720), fechada, 30 comentários) e **o som volta ao padrão** ([#3855](https://github.com/LMMS/lmms/issues/3855) e [#2865](https://github.com/LMMS/lmms/issues/2865), fechadas). Nosso código: **parece resolvido**, mas é um ponto sensível.
- **O preview de presets "vaza" estado**: depois de prever um preset externo, os outros mudam de som ([#6074](https://github.com/LMMS/lmms/issues/6074), aberta; [fórum](https://lmms.io/forum/viewtopic.php?f=7&t=34452)), e o preview herda o arpejo do preset anterior ([#1644](https://github.com/LMMS/lmms/issues/1644), aberta). Nosso código: **não verificado**.
- **Notas presas com o arpejador** ao parar em loop. Issue aberta [#1762](https://github.com/LMMS/lmms/issues/1762). Nosso código: **não verificado**.
- **O knob de portamento não funciona de 0 a 64**: na prática é o CC65, um liga/desliga. Issue aberta [#3936](https://github.com/LMMS/lmms/issues/3936). Nosso código: **parece presente** (o knob vai de 0 a 127 e manda o CC65 direto).
- **Só 7 controles automatizáveis**; pedem automação de todos ([#4263](https://github.com/LMMS/lmms/issues/4263), aberta) e ligar cutoff e resonance do SUB e do ADD ([#711](https://github.com/LMMS/lmms/issues/711), aberta). Nosso código: **parece presente**.
- **Glitch a 192 kHz** ([#5534](https://github.com/LMMS/lmms/issues/5534), aberta), **chiado na exportação com eco** ([#5135](https://github.com/LMMS/lmms/issues/5135), aberta) e **pedido de buffer interno configurável** ([#3091](https://github.com/LMMS/lmms/issues/3091), aberta). Nosso código: **não verificado**.
- **Versão do Zyn antiga (2.4.x, FLTK)**; pedem o Zyn-Fusion ([#4179](https://github.com/LMMS/lmms/issues/4179), aberta), e a fonte da UI é minúscula ([#5386](https://github.com/LMMS/lmms/issues/5386), fechada). Nosso código: **parece presente**.

## Lv2Instrument (host LV2)

- **Não abre a GUI nativa do plugin LV2**: o botão "Show GUI" está desativado no código. Nosso código: **parece presente**: em `src/gui/Lv2ViewBase.cpp` há `if (false) // TODO: check if the plugin has the UI extension`, e `toggleUI()` está vazio. É a maior limitação da integração LV2 ([#562](https://github.com/LMMS/lmms/issues/562), aberta; [#4954](https://github.com/LMMS/lmms/issues/4954), plano de extensões, aberta).
- **Layout dos controles genéricos quebra** com plugins de muitos parâmetros ([#6798](https://github.com/LMMS/lmms/issues/6798) e [#7510](https://github.com/LMMS/lmms/issues/7510), abertas). Nosso código: **não verificado**.
- **Quase nenhum plugin LV2 aparece no AppImage** ([#5832](https://github.com/LMMS/lmms/issues/5832), aberta; empacotamento e caminhos de busca).
- **Exportação meio tom abaixo** ([#6401](https://github.com/LMMS/lmms/issues/6401), fechada) e **perda de automação ao exportar** ([#6786](https://github.com/LMMS/lmms/issues/6786), fechada). Nosso código: **parece resolvido** (`activate()` recarrega se a taxa muda).
- **Sem suporte a mais de 2 canais de áudio** ([#5108](https://github.com/LMMS/lmms/issues/5108), aberta). Nosso código: **não verificado** (pode conflitar com o trabalho multicanal do fork).

---

## Prioridade sugerida

Ordenada por valor para o usuário e esforço:

1. **Crash do AFP ao abrir a janela (#8647)**: o fork está exatamente no commit afetado. Aplicar o PR #8644 do upstream, ou corrigir `SampleThumbnailRenderer::visualize`. É crítico e barato.
2. **Vibed a 192 kHz (#7221)**: a divisão inteira zera o oversample. É uma correção de uma linha (calcular em float, com mínimo de 1).
3. **Xpressive overflow (#5877)**: trocar o `static_cast<int>` por `std::floor` em `positiveFraction`. Uma linha, elimina um crash.
4. **Notas e tons presos**: OpulenZ ao parar (#2529), Zyn com arpejador (#1762) e a troca de canal no meio da nota (#8030). Fazer um "all notes off/reset" consistente ao parar e ao exportar resolve também o **LB302 não resetar estado na exportação (#7234)**.
5. **Normalizar os volumes padrão dos instrumentos (#1207, #7852)**: só para instâncias novas, sem quebrar projetos antigos.
6. **GUI nativa de LV2 (Show GUI desativado)**: grande ganho de usabilidade para o host LV2, via suil, que já está no CMake.
7. **Anti-aliasing nos instrumentos nativos (#6098)**: começar pelo BitInvader, Organic e Watsyn (podem usar as wavetables do `Oscillator`) e pelo PM/FM do TripleOscillator; depois o Monstro (melhorar o filtro de decimação).
8. **FreeBoy**: bloquear DC (#6556), corrigir o roteamento estéreo SO1/SO2 (#8043) e investigar CPU e ruído (#6144, #3824).
9. **AFP**: microfades no início, loop e fim (#5855), aceitar `.xpf` no diálogo (#7492) e esconder o loopback com o loop desligado (#7819).
10. **Sf2Player**: arrastar soundfont para trocar (#7480), campos maiores (#7482) e ADSR simples (#3233, também no GIG); **GIG**: sustain correto (#4517) e CPU (#3825).
11. **SlicerT**: detecção de BPM real, por onset/autocorrelação (#8102), e modos de loop (#7364).
12. **Zyn**: tirar o corte de áudio do "Show GUI" (#1825), corrigir o knob de portamento (#3936) e ampliar a automação (#4263).
13. **Modo mono/legato com portamento genérico** no núcleo (#6516): beneficia todos os instrumentos de uma vez.
14. **Itens menores**: faixa de automação do RES do LB302 (#7980), PW mínimo do Monstro (#3046), aviso visível quando o Patman falha ao carregar (#8163), DnD no Patman (#8065), preset `Organ_leslie.xpf` (#7839) e investigar os presets do Mallets que só dão ruído (#7573).
