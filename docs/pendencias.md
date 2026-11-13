# Pendências

A lista do que falta, atualizada a cada passo (o detalhe de cada item fica em `plano-gravacao.md`).

## Em andamento

- [ ] **Thread de áudio sem alocação, E/S ou trava** (etapa 1)
  - [x] detector: `AudioThreadAllocationTest` conta cada alocação feita na thread de áudio e mostra onde
  - [x] automação sem alocação (era metade)
  - [ ] ciclo de vida das notas: `NotePlayHandle` e os dados de nota que cada instrumento cria com `new`
  - [ ] lista de play handles do motor
  - [ ] monitoração do master
  - [ ] o teste passa a exigir zero (tirar o `QEXPECT_FAIL`)

## A fazer

- [ ] **Transporte com contador de frames** (etapa 1): estado atômico (parado, tocando, gravando, pré-roll, count-in),
  contador absoluto de frames da linha do tempo, ações globais Tocar, Parar e Gravar
- [ ] **Motor sem trava** (etapa 1): listas publicadas como snapshots imutáveis, travas finas tentadas, e o fim do
  mutex do período inteiro; cada passo com TSan e A/B
- [ ] **Isolamento de plugins** (etapa 0): plugin ativo numa queda vai para a lista de suspeitos; da próxima vez roda
  isolado pela ponte do Carla, ou fica "não carregado" com aviso; opção por plugin para forçar o isolamento
- [ ] **Fórmula de compasso por marcador** no mapa de tempo
- [ ] **Integração final:** testes, sanitizers, A/B, gravação real no Mac, documentação
- [ ] **Linux:** testar o ALSA e o JACK numa máquina Linux

## Feito recentemente (13/11)

- [x] Faixa de áudio: modelo, player, interface (duas linhas, lanes, alças, crossfade, comp), tempo, gravação,
  botão, conversão das faixas de sample com gravações
- [x] Lanes opcionais nas configurações
- [x] Fades de 5 ms e crossfade automático; arrastar clipes para outra faixa
- [x] Clipes que cruzam mudanças de tempo ficam na grade em todo lugar
- [x] Mover um take sem mudar o BPM não pede mais renderização
- [x] Desfazer das edições do mapa de tempo
- [x] Padrão de 120 BPM
- [x] Streaming do disco com opção RAM/disco por clipe, cache de picos, forma de onda ao vivo, limpeza de órfãos
