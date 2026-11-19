# Toolbar nova (transporte centralizado)

Referência visual: o mockup do redesign do LMMS feito por budislav
([#1911](https://github.com/LMMS/lmms/issues/1911)), e o PR [#5261](https://github.com/LMMS/lmms/pull/5261), que começou
a implementá-lo (velho demais para mesclar; foi usado só como referência).

## Como é hoje

- A barra de cima (`MainWindow`) é uma grade com:
  - botões de arquivo (novo, novo a partir de template, abrir, recentes, salvar, exportar) e o metrônomo;
  - botões que abrem cada editor.
- Tempo, relógio, compasso, volume e pitch master, osciloscópio e CPU **são colocados ali pelo Song Editor**, no
  construtor dele (`SongEditor.cpp`, via `addWidgetToToolBar`). Com isso, a toolbar depende do Song Editor.
- Cada editor tem o próprio play/stop (e o Piano Roll também gravar), o que repete o transporte em vários lugares.
- Os números usam imagens de LCD verdes (`lcd_19green.png` etc.).

## Como fica

```
[salvar] [ajuda]        [metrônomo] [Part|Song] [▶] [●] [●▶] [■]  000:00.000  TEMPO 140  4/4  VOL 100  PITCH 0  CPU ▮▮▯ OUT ▮▮▯        [Layout ▾]
```

1. **Esquerda:** salvar e ajuda. Novo, abrir, recentes e exportar continuam no menu Arquivo.
2. **Centro, a barra de transporte (`TransportBar`):**
   - metrônomo;
   - **Part / Song:** escolhe o que o play toca, o padrão do Pattern Editor ou a música inteira;
   - **Play/Pause, Gravar, Gravar tocando e Stop**;
   - relógio (MIN : SEC . MSEC, ou compasso:batida ao clicar), **TEMPO**, **compasso**, **VOL** e **PITCH** master;
   - medidores de **CPU** e de **saída (OUT)**, em barras planas.
3. **Direita:** seletor de **layout**, com arranjos prontos dos painéis:
   - **Arranjo:** só o Song Editor, painel de baixo recolhido;
   - **Mixagem:** Mixer aberto e alto;
   - **Composição:** Piano Roll aberto.
4. **Os editores deixam de ter transporte próprio onde ele foi para a barra:** Song Editor e Pattern Editor perdem
   play/stop (os atalhos de teclado continuam funcionando). Piano Roll e Automation Editor mantêm os deles, porque tocam
   só o clipe aberto, que é outra função.
5. **Números planos:** o tema LinuxMusic troca as imagens de LCD por dígitos planos verde-água, gerados por
   `tools/make_lcd_pixmaps.sh`. Isso vale para todos os displays numéricos do app, não só a toolbar.

## Comportamento do transporte

| Botão | Ação |
|---|---|
| Play (modo Song) | Toca a música; se já estiver tocando a música, pausa |
| Play (modo Part) | Toca o padrão aberto no Pattern Editor; se já estiver tocando, pausa |
| Stop | Para tudo e encerra uma gravação MIDI em andamento |
| Gravar / Gravar tocando | Gravação **MIDI** no clipe aberto no Piano Roll (a gravação de áudio vem na etapa 3) |
| Metrônomo | Liga/desliga o metrônomo |

- O botão de play mostra **pause** enquanto algo toca.
- O seletor Part/Song acompanha o que está tocando: se o play vier do Pattern Editor, ele vai para Part.

## Passos de implementação

1. `TransportBar` (`include/TransportBar.h`, `src/gui/TransportBar.cpp`) com os controles acima, ligados aos modelos do
   `Song` (tempo, compasso, volume e pitch master) e aos editores (play, stop e gravar pelas ações deles).
2. Widget de medidores (`TransportMeters`), que desenha CPU e saída master como barras, atualizado pelo
   `MainWindow::periodicUpdate`.
3. Tirar do construtor do Song Editor a montagem de tempo, relógio, compasso, volume, pitch, osciloscópio e CPU, junto
   com os slots e membros que só existiam para isso.
4. Reorganizar a barra do `MainWindow`: esquerda, `TransportBar` centralizada e seletor de layout à direita. Remover os
   botões de arquivo e de abrir editores (as abas e os menus já cobrem isso).
5. Esconder play/stop nas barras do Song Editor e do Pattern Editor.
6. Seletor de layout com os três arranjos de painéis.
7. Dígitos planos para o tema LinuxMusic (`tools/make_lcd_pixmaps.sh` gera as imagens com ImageMagick).
8. Compilar, testar e conferir com `LINUXMUSIC_SCREENSHOT`.

## Situação (11/11/2026)

Implementado: passos 1 a 8.

- A barra de transporte e os medidores estão em `TransportBar`.
- O Song Editor não monta mais nada na toolbar, e Song Editor e Pattern Editor não têm mais play/stop próprios.
- O seletor de layout tem três arranjos: Arranjo, Mixagem e Composição.
- Os atalhos Ctrl+1 a Ctrl+7 continuam funcionando.
- No passo 7, em vez de gerar imagens com fontes do sistema (o que embutiria glifos de fontes com licença
  restritiva), o `LcdWidget` ganhou a propriedade `flatDigits`. Com ela, desenha os dígitos com a fonte da interface.
  O tema LinuxMusic liga essa propriedade no CSS (`qproperty-flatDigits: true`), e todos os displays numéricos do app
  ficam planos.

Pendente:
- testar o transporte interativamente (play, pause, Part/Song, gravação MIDI);
- traduzir os textos novos;
- o ponto decimal dos displays com casas decimais (`LcdFloatSpinBox`) ainda usa a imagem antiga.
