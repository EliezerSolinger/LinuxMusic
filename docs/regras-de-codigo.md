# Regras de código do núcleo

Valem para código novo e para todo código que for mexido. O código antigo vai sendo trazido para elas aos poucos
(`docs/plano-refatoracao.md`, fase 4).

## Quem é dono de cada objeto

- **Um dono só, e explícito.** Quem cria e apaga um objeto o guarda num `std::unique_ptr` (ou num contêiner de
  `unique_ptr`). Um `QObject` pode, em vez disso, ter um pai do Qt que o apaga, mas **nunca as duas coisas**: um
  objeto num `unique_ptr` não tem pai do Qt.
- **Quem só usa, observa.** Um ponteiro cru ou uma referência quer dizer "não sou dono". Se o objeto observado pode
  morrer antes de quem observa:
  - quando é um `QObject`, use `QPointer`, que vira nulo sozinho;
  - quando não é, quem observa precisa ser avisado da morte (um sinal, ou o dono limpando a lista de observadores).
- **Nada de `delete` manual** em código novo. As exceções são o alocador de tempo real e o pool de notas
  (`RealtimeAllocator`, `NotePlayHandleManager`), que cuidam da própria memória.
- **Passar a posse é visível na assinatura:** a função recebe um `std::unique_ptr<T>`. Um `T*` nunca transfere a
  posse.
- **`std::shared_ptr` só quando a posse é mesmo dividida** (por exemplo, um buffer de amostra que vários clipes
  usam), nunca para "não pensar em quem é o dono".

## Threads

- **A thread de áudio não aloca, não trava e não lança exceção.**
  - Para avisar algo, use `ErrorLog::report` (só textos literais e um número).
  - Para passar dados à interface, use atômicos (`std::atomic`, `AtomicPeak`) ou as filas sem trava que já existem.
- **Valor lido por uma thread e escrito por outra é atômico** (`relaxed` quando é só um valor) ou fica protegido
  pela trava do motor (`requestChangeInModel`).
- **Conexões entre threads têm um objeto de contexto** (para virarem fila) ou são substituídas por atômicos lidos no
  temporizador da interface.
- **Nunca abrir janelas fora da thread principal.** O núcleo avisa o usuário por `UserNotice` (veja abaixo).

## Erros

- **Um erro tratado sem exceção vai para o `ErrorLog`.** Nunca some em silêncio.
- **Dados de fora (arquivos, plugins, aparelhos) são validados, não assumidos com `assert`.** Um `assert` só testa
  invariantes internas, porque some na versão final.
- **O núcleo não abre janelas:** avisos e perguntas ao usuário passam por `UserNotice` (`tests/scripted/check-core-gui`
  impede o núcleo de incluir a interface).

## Verificação antes de cada commit

`ctest`, `tools/check/ab.sh` e, quando o assunto é threads ou memória, os builds com ASan/UBSan e TSan
(`tools/check/README.md`). Os avisos novos do compilador contam como erros: leia-os.
