# Ideias para o futuro

Ideias registradas para avaliar depois das etapas atuais. Nenhuma tem prazo.

## Modo sem conversões (uma única taxa de amostragem em todo o projeto)

**Ideia:** o projeto inteiro funciona numa única taxa de amostragem e num único formato, sem nenhuma reamostragem em
tempo real. O objetivo é economizar processamento e evitar a perda de qualidade das conversões.

Como funcionaria:

- **O projeto define a taxa de amostragem** (por exemplo, 48 kHz) e o formato interno de áudio.
- **A interface de áudio abre na mesma taxa do projeto**, tanto para tocar quanto para gravar. Se o dispositivo não
  suportar essa taxa, o usuário é avisado em vez de o programa converter em silêncio.
- **Samples são convertidos uma única vez, ao entrar no projeto.** Ao importar ou arrastar um arquivo de áudio, ele é
  convertido para a taxa e o formato do projeto, e a cópia convertida fica na pasta do projeto. Durante a reprodução,
  nada é convertido.
- **A gravação já sai no formato do projeto**, porque o dispositivo de entrada trabalha na mesma taxa.
- **Todos os plugins rodam na taxa do projeto.** Plugins que hoje reamostram internamente (alguns efeitos e
  instrumentos que trabalham numa taxa fixa) precisam ser adaptados ou ficar marcados como "não compatíveis com o modo
  sem conversões".

Pontos a investigar quando chegar a hora:

- Hoje o motor de áudio já tem uma taxa única de saída, mas a reprodução de samples (`Sample::play` /
  `AudioResampler`) reamostra em tempo real sempre que a taxa do arquivo difere da do motor. Também há efeitos que
  usam reamostragem interna (por exemplo, as funções `sampleDown`/`sampleBack` que existiam em `Effect`).
- O que fazer com o pitch de samples (tocar um sample em outra nota exige reamostragem por definição). Isso continua
  existindo como recurso musical, mas não como conversão de taxa.
- Como tratar a mudança da taxa do projeto depois de criado: reconverter todos os samples da pasta do projeto, uma vez.
- Relação com a etapa 3 (gravação): o gravador deve escrever direto no formato do projeto, sem conversão.
