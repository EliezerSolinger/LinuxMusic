# Auditoria de texto das telas (26/11)

O programa novo "ficou verboso" perto do legado. Regra: denso, profissional, rótulos sem explicações; a explicação útil vira tooltip; erro e segurança ficam, em uma frase curta. Telas com design aprovado (cabeçalho de faixa 02, transporte 03, janela de instrumento 05, configurações 08) mantêm o layout; só o texto encurta.

Legenda da coluna "ação": remover, encurtar para "X", tooltip. Coluna "feito": sim, ou o motivo de ter ficado.

Verificação desta rodada: só `cargo check -p lm-gui -p lm-gui-plugins -p lm-gui-app --all-targets` (sem testes, por regra do dia).

## Configurações (design 08)

| # | Onde | Texto atual | Por que sobra | Ação | Feito |
|---|---|---|---|---|---|
| 1 | subtítulo de cada página (10) | "Projetos e como o programa se comporta.", "Tudo aqui vale na hora…", etc. | explica o óbvio; o legado não tinha | remover | sim |
| 2 | cada linha | selo "vale na hora" em todas | informação repetida em ~60 linhas; o normal não é notícia | mostrar só "pede reinício" | sim |
| 3 | linhas com dica (~25) | dica de 1 a 3 linhas abaixo do controle | explicação embaixo do controle | tooltip no rótulo | sim |
| 4 | Geral, "Long audio files…" | "Mais longo que isto, um arquivo de áudio solto no projeto vira faixa de áudio; mais curto, amostra" | longo | encurtar (tooltip) | sim |
| 5 | Áudio, sistema | "ALSA direto, a placa usada sozinha (hw); PipeWire e PulseAudio são pedidos a liberar" | explicação | "ALSA direto, a placa sozinha (hw)" no tooltip | sim |
| 6 | Áudio, estado | "Aberto: X → Y a 48 kHz, bloco N (combinado com o dispositivo), latência em A ms, saída B ms" | linha de status verbosa | "Aberto: X → Y a …, bloco N, A ms entrada, B ms saída" | sim |
| 7 | Áudio, estado | "Nenhum dispositivo de áudio aberto: o programa não toca nem grava" | segunda frase explica | "Nenhum dispositivo de áudio aberto" | sim |
| 8 | Áudio, Bluetooth | "Usar o microfone dele derruba a saída para o perfil de chamada, mais de 150 ms de latência" | longa | "O microfone acrescenta mais de 150 ms" | sim |
| 9 | Áudio, calibração | notas "Calibrado para este dispositivo, taxa e conversor" / "Não calibrado aqui: a gravação usa só a latência que o dispositivo informa, e avisa" | longas | "Calibrado" / "Não calibrado" | sim |
| 10 | Áudio, calibração | legenda "Ligue uma saída a uma entrada com um cabo: um pulso toca cinco vezes…" | instrução embaixo do botão | remover | sim |
| 11 | Áudio, permissão tempo real | "Sem isso o programa toca assim mesmo, mas o áudio pode estalar com carga. Para configurar, e depois entrar de novo:" | parágrafo | "Para configurar, e depois entrar de novo:" | sim |
| 12 | Gravação, "Espaço em disco" | "Armar avisa quando as entradas armadas não cabem em 10 minutos…" | linha inteira só de explicação | remover a linha | sim |
| 13 | Autosave, "Diário" | "Sempre ligado: toda mudança vai para backup/diario.jsonl…" | linha inteira só de explicação, sem controle | remover a linha | sim |
| 14 | Avançado, "Proteção da audição" | "Fixo: um pico acima de +24 dBFS no master corta a saída…" | linha inteira sem controle | remover a linha | sim |
| 15 | Teclado, atalho em conflito | "Já usado por "X": aperte de novo para tomá-las, Esc cancela" | segunda cláusula | "Em uso por "X": aperte de novo para tomá-las" | sim |
| 16 | Teclado, captura | "Aperte as novas teclas… (Esc cancela)" | parêntese | "Aperte as novas teclas…" | sim |

## Exportar

| # | Onde | Texto atual | Por que sobra | Ação | Feito |
|---|---|---|---|---|---|
| 1 | O quê: a música toda | "Compassos 1 a N, M:SS; sempre exato" | repete o que o botão diz | só a duração | sim |
| 2 | O quê: o loop / a região | "Compassos A a B (o loop na régua)", "A região: compassos A a B" | dois textos para o mesmo dado | "Compassos A a B" | sim |
| 3 | O quê: uma faixa | "A música toda com as outras faixas mudas" | explicação | remover | sim |
| 4 | Rápido/Exato | parágrafo "Começa N s antes e descarta…; quase idêntico, e o relatório avisa" / "Renderiza do começo…; idêntico bit a bit…" | parágrafo | "Renderiza X, uns Y" | sim |
| 5 | Como loop | "Como loop: sem a cauda no fim (reverbs, releases)" é o rótulo do interruptor | rótulo longo | "Como loop" | sim |
| 6 | Formatos | "Vários de uma vez, cada um um arquivo na pasta escolhida no fim." | explicação | remover | sim |
| 7 | Processamento | "Aplicado a todos os formatos." | explicação | remover | sim |
| 8 | Processamento, dither | "Dither (TPDF) para 16 e 24 bits" | complemento óbvio | "Dither (TPDF)" | sim |
| 9 | Formatos | "Escolha pelo menos um formato." | | "Escolha um formato." | sim |
| 10 | Onde, aviso | "Estes plugins de terceiros não prometem o mesmo resultado duas vezes…" | longo | "Não determinístico: …" | sim |
| 11 | Relatório | "Trecho rápido: quase idêntico ao mesmo trecho na música toda, não bit a bit" | longo | "Trecho rápido: não idêntico bit a bit" | sim |

## Projeto (abrir, salvar como, estado, importar)

| # | Onde | Texto atual | Por que sobra | Ação | Feito |
|---|---|---|---|---|---|
| 1 | Abrir | "Projetos do programa antigo (.mmp, .mmpz) não abrem aqui" | aviso permanente | remover | sim |
| 2 | Importar/Exportar MIDI | "Um arquivo MIDI (.mid) ou de áudio; entra no cursor" / "As notas do projeto como arquivo MIDI (tipo 1)" | explicação sob o campo | remover | sim |
| 3 | Salvar como | "{pasta}/projeto.json, audio/ (copiado), backup/" | detalhe interno | remover | sim |
| 4 | Estado, só leitura | "Aberto em X (processo N, desde T): abriu só para leitura, assim os dois nunca escrevem por cima" | segunda cláusula | "…: só leitura" (mantido: segurança) | sim |
| 5 | Estado, versão nova | "Salvo pelo LinuxMusic X: abriu só para leitura e nunca é salvo por cima" | idem | "…: só leitura" | sim |
| 6 | Estado, migração | "Convertido de versão antiga: o original fica em projeto.json.vN.bak" | nome de arquivo interno | "Convertido do formato N; o original foi mantido" | sim |
| 7 | Modo seguro | "o programa parou enquanto este plugin rodava; abriu desligado, faixa muda, ajustes mantidos" | parágrafo | "Modo seguro: estes plugins estão desligados" | sim |
| 8 | Limpar arquivos | "N arquivos vão para a lixeira do sistema, e o histórico de desfazer é limpo" | já é aviso de perda de dados | encurtado, mantido | sim |

## Projeto novo

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | abaixo da taxa | "Fixa para este projeto: o dispositivo abre nela, ou converte na borda" | remover | sim |

## Sobre

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | Envolvidos | "Contribuidores ordenados por número de commits:" | remover | sim |
| 2 | Licença | "Você pode usar, estudar, compartilhar e mudar; o que compartilhar, compartilha sob a mesma licença." | remover | sim |
| 3 | Manual | "O manual vem com o programa, em português e inglês, e abre sem internet" | remover | sim |
| 4 | Manual, capítulos (9) | "Começando: a pasta do projeto, o dispositivo, a primeira gravação" etc. | encurtar para o nome ("Primeiros passos"…) | sim |
| 5 | Tradução | "Português e inglês com o programa; os outros idiomas do legado onde o texto ainda casa" | "Português, English" | sim |
| 6 | Direitos | "Copyright © 2004-2026." | tirar o ponto final | sim |

## Avisos

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | 31 avisos do núcleo (tempo real, arquivo ausente, só leitura, trava, takes recuperáveis, modo seguro, conversão, calibração, Bluetooth, dispositivo, JACK/PipeWire, placas diferentes, servidor, exportação sem espaço…) | duas ou três cláusulas cada | uma frase curta (traduções pt-BR; as chaves em inglês são do núcleo e não mudaram) | sim |

Mantidos por segurança: perda de dados, só leitura, modo seguro, disco cheio, dispositivo que não abriu.

## Janela de instrumento (design 05) e Efeito

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | sem instrumento | "Esta faixa não tem instrumento." | "Sem instrumento" | sim |
| 2 | plugin ausente | "…não está instalado. A faixa fica muda; os ajustes são mantidos e salvos com o projeto." | "…não está instalado; a faixa fica em silêncio" | sim |
| 3 | plugin ausente | "Ache no navegador e solte aqui." | remover (o botão basta) | sim |
| 4 | abas Envelopes, Acordes, Microafinação | "…não valem para este instrumento: ele toca as próprias vozes." (3 textos) | "Indisponível para este instrumento" | sim |
| 5 | aba Efeitos | "Os efeitos ficam no canal da mesa "X", para onde vai a saída principal." | `Canal "X"` | sim |
| 6 | aba Efeitos, janela Efeito | "Nenhum efeito ainda." | "Sem efeitos" | sim |
| 7 | aba MIDI, velocidade | "Notas MIDI na velocidade base tocam a 100 %." | remover | sim |
| 8 | kit de bateria | "Clique numa peça para ouvir" | remover | sim |
| 9 | afinação | "Intervalos, um por linha: cents (701.955) ou razões (3/2)" e "Grau da escala de cada tecla (x para sem som)" | "Intervalos (cents ou razão)", "Grau de cada tecla" | sim |
| 10 | afinação, erros | "A linha N não é um intervalo (cents com ponto, ou razão como 3/2)." e o da tecla | uma frase curta | sim |
| 11 | sem faixa | "Nenhuma faixa de instrumento." | "Sem instrumento" | sim |

## Piano Roll, Automação, Editor de passos

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | Piano Roll vazio | "Duplo clique num item MIDI na Composição para abrir aqui" | "Nenhum item MIDI" | sim |
| 2 | Automação vazia | "Duplo clique numa lane de automação para abrir aqui" | "Nenhuma pista de automação" | sim |
| 3 | Automação | "O núcleo não dá faixa de valores para este parâmetro: a pista não pode ser desenhada" | "Sem faixa de valores para este parâmetro" | sim |
| 4 | Passos vazio | "Escolha os itens a editar na lista acima" | "Nenhum item" | sim |
| 5 | tabelas de idioma | ~45 textos de ajuda de rodapé (cliques, arrastos, atalhos) que nenhuma tela mais usa | remover do locale (já são tooltips curtos como no legado) | sim |

## Paleta, Roteamento, Mapas MIDI, Ligação MIDI, Workspace

| # | Onde | Texto atual | Ação | Feito |
|---|---|---|---|---|
| 1 | rodapé da paleta | "↑ ↓ para escolher · Enter para executar · Esc para fechar" | remover | sim |
| 2 | paleta vazia | "Nenhum comando tem essas letras" | "Nenhum comando" | sim |
| 3 | Roteamento | "Cada saída de uma faixa vai para um canal da mesa; várias podem ir para o mesmo (N canais)" | remover | sim |
| 4 | Mapas MIDI vazio | "Nenhum mapa. Clique com o botão direito num controle e escolha Aprender MIDI." | "Nenhum mapa" | sim |
| 5 | Ligação MIDI | "Nenhum controle MIDI move isto ainda." | "Nenhum mapa" | sim |
| 6 | Workspace, fechar | "O projeto tem alterações que não foram salvas." | "O projeto tem alterações não salvas." | sim |

## Sem itens (revisadas)

Transporte (03), cabeçalho de faixa (02), Composição, Mesa/Mixagem, Marcadores, Navegador e Arquivos: só rótulos, valores e tooltips; nada a cortar. As descrições de plugins no locale (22 frases) já não apareciam em nenhuma tela e saíram.

## Para o usuário decidir

- O selo "vale na hora" saiu de todas as linhas do design 08 (só "pede reinício" ficou). Se quiser de volta, é uma linha em `apply_badge`.
- As chaves dos 31 avisos vêm do núcleo em inglês; só a tradução pt-BR encurtou. Para encurtar também o inglês, o texto precisa mudar no núcleo.
- Chaves de locale duplicadas entre arquivos que existiam antes desta rodada: nenhuma sobrou na verificação feita aqui (teste não rodado).
