# Documentação do LinuxMusic

O LinuxMusic é um fork do [LMMS](https://github.com/LMMS/lmms) com um objetivo claro: virar uma DAW para Linux
capaz de gravar áudio ao vivo de verdade, no estilo do REAPER.

Para chegar lá, o trabalho foi dividido em três etapas longas, feitas nesta ordem:

| Etapa | Tema | Documento |
|---|---|---|
| 1 | Limpar e reorganizar plugins e funcionalidades | [etapa-1-limpeza.md](etapa-1-limpeza.md) |
| 2 | Remodelar interface e usabilidade, e preparar a base da mesa de som e dos canais | [etapa-2-interface-e-mixer.md](etapa-2-interface-e-mixer.md) |
| 3 | Implementar um sistema de gravação no estilo REAPER | [etapa-3-gravacao.md](etapa-3-gravacao.md) |

A ordem é intencional. Primeiro se reduz o código para o que importa. Depois a interface e o mixer são
remodelados já pensando em canais com entrada de áudio. Só então a gravação é construída sobre essa base,
em vez de ser encaixada à força na estrutura antiga.

## Referências de produto

> **Pivô de conceito (11/11/2026):** o mixer e o sistema do LinuxMusic vão ficar **diferentes** do esquema de design
> sugerido pelo LMMS (o conceito de redesign #1911 e os mockups dele). O projeto já pivotou para um conceito próprio:
> trilhas organizadas em **hierarquia** (pastas e canais de grupo), mixer e gerenciamento de plugins no estilo do
> FL Studio, gravação no estilo do REAPER, e os canais extras do núcleo multicanal escondidos como encanamento
> interno. O redesign do LMMS continua servindo só como referência de layout geral (janela única, painéis
> encaixados); ele não define o mixer nem a organização das trilhas. Detalhes em
> [preparacao-mixer.md](preparacao-mixer.md).


- **Mixer, efeitos e gerenciamento de plugins: FL Studio.** Mixer com canais e slots de efeito sempre à vista,
  roteamento simples entre canais e plugins fáceis de achar, adicionar e organizar. O REAPER é fraco nesse ponto;
  até o rack de efeitos atual do LMMS é melhor que o dele.
- **Gravação de faixas: REAPER.** Armar faixas, escolher a entrada de cada uma, monitorar e gravar direto em disco.

## Regras gerais do projeto

- **Linux é o destino final.** Tudo precisa compilar e funcionar no Linux. O macOS é usado só como máquina de
  desenvolvimento; nada pode depender dele.
- **Sem compromisso com o formato de projeto do LMMS.** Os arquivos `.mmp`/`.mmpz` podem mudar livremente quando
  isso simplificar o código ou permitir algo novo.
- **Licença GPL-2.0 ou posterior**, herdada do LMMS. Os créditos aos autores originais (`doc/AUTHORS`,
  `doc/CONTRIBUTORS` e os cabeçalhos dos arquivos) devem ser mantidos.
- **Relação com o LMMS original:** o remote `upstream` aponta para `LMMS/lmms`. À medida que o código divergir
  (principalmente depois da etapa 1), puxar mudanças do LMMS vai ficar mais difícil. A partir daí, correções do
  upstream devem ser trazidas caso a caso (`git cherry-pick`), não com merge completo.

## Estado atual

Veja [estado-atual.md](estado-atual.md) para saber de onde o código partiu e o que já foi feito.

Antes da etapa 2, veja [pesquisa-ui-existente.md](pesquisa-ui-existente.md): é o levantamento do trabalho de UI e
mixer que já existe no LMMS.

Ideias registradas para depois das etapas atuais ficam em [ideias-futuras.md](ideias-futuras.md).

Antes do mixer novo, veja [preparacao-mixer.md](preparacao-mixer.md): as decisões e a ordem de implementação.
