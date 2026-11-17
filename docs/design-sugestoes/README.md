# Sugestões de design (17/11/2026)

Propostas para o programa todo, menos o mixer e a cadeia de efeitos, que ficam como estão. Elas mantêm as cores do
tema atual: o fundo cinza-azulado escuro e o verde do LMMS. Cada imagem mostra a proposta, e várias mostram também o
que existe hoje, para comparar. Os números verdes na imagem correspondem às explicações da coluna da direita.

Marque o que aprovar (troque `[ ]` por `[x]`), ou escreva ao lado o que mudar. Nada disso foi implementado ainda.

| # | Imagem | Aprovo | Observações |
|---|---|---|---|
| 1 | [Janela principal](01-janela-principal.png): uma barra superior só, navegador com abas, inspetor à direita, editores encaixados embaixo, barra de status | [ ] | |
| 2 | [Cabeçalho de faixa no estilo REAPER](02-cabecalho-de-faixa.png): três alturas, cores por estado, volume em régua | [ ] | |
| 3 | [Barra de transporte](03-transporte.png): um visor grande, estados por cor e por palavra | [ ] | |
| 4 | [Piano roll](04-piano-roll.png): uma barra com texto, escala à vista, notas fantasmas, lanes com abas | [ ] | |
| 5 | [Janela do instrumento](05-janela-do-instrumento.png): painel encaixado, abas com texto, rodapé fixo | [ ] | |
| 6 | [Navegador](06-navegador.png): busca única, subcategorias, painel de detalhe, pré-escuta | [ ] | |
| 7 | [Botões, controles e cores](07-botoes-e-controles.png): tamanhos, estados, gestos e cores iguais em tudo | [ ] | |
| 8 | [Configurações](08-configuracoes.png): aplica na hora, avisa o que pede reinício | [ ] | |
| 9 | [Editor de sequências](09-editor-de-sequencias.png): passos maiores, grupos de 4, passo atual destacado | [ ] | |

A imagem 7 é a base das outras: se ela for aprovada, as outras telas passam a usar as mesmas peças.

## Como foram feitas

As telas são HTML desenhado à mão (`fontes/*.html`, com as cores em `fontes/base.css`), fotografado com o Chrome sem
janela. Para refazer depois de uma mudança, rode `python3 gerar.py` dentro de `fontes/`. As capturas do programa atual
usadas para comparar estão em `fontes/atual-*.png`.
