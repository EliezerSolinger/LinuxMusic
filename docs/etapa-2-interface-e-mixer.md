# Etapa 2: remodelar interface e usabilidade, e preparar a base da mesa de som e dos canais

## Objetivo

Remodelar a interface e a usabilidade em vários pontos e, principalmente, deixar a mesa de som e os canais prontos
para receber a gravação da etapa 3. No final desta etapa, cada faixa deve ter um canal de verdade, mesmo que a
entrada de áudio ainda não grave nada.

**Antes de começar, leia [pesquisa-ui-existente.md](pesquisa-ui-existente.md).** O LMMS já tem um conceito de
redesign aceito (#1911) e vários PRs de UI e mixer abertos que podem ser aproveitados.

## Parte A: interface e usabilidade

Pontos a remodelar (lista a detalhar e priorizar):

- [ ] Levantar os principais incômodos de uso do LMMS atual e listá-los aqui.
- [ ] Layout da janela principal e organização dos editores (Song Editor, Piano Roll, Pattern Editor, Mixer).
- [ ] Atalhos de teclado e fluxo de trabalho do dia a dia.
- [ ] Tema visual e legibilidade.
- [ ] Logo, splash e identidade visual próprios do LinuxMusic.

## Parte B: base da mesa de som e dos canais

Hoje, no LMMS, as faixas apenas apontam para um canal do mixer, e o mixer não tem noção de entrada de áudio
externa. A base a preparar:

- **Canal por faixa (channel strip):** volume, pan, mute/solo, medidor de nível e inserts de efeitos, no próprio
  canal da faixa.
- **Entrada do canal:** o modelo de dados do canal já deve prever uma entrada de áudio, ou seja, qual entrada da
  interface (mono ou estéreo) alimenta aquela faixa. Ela só será usada na etapa 3.
- **Botões de armar e monitorar:** podem existir na interface já nesta etapa, ainda sem efeito, para fixar o layout.
- **Roteamento:** buses, sends e saída master, de forma clara e visível.
- **Medidores confiáveis** de entrada e saída.

## Critério de conclusão

- A interface remodelada está em uso.
- Cada faixa tem um canal com entrada configurável no modelo de dados e na interface.
- O projeto compila no Linux, os testes passam e projetos existentes do LinuxMusic tocam corretamente.
