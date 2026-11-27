# Varredura de qualidade: área `gui`

Crates: `lm-gui`, `lm-gui-plugins`, `lm-gui-app`, `lm-port`, `lm-port-fake`. Auditoria somente leitura, feita sobre `origin/master` (b051d6281).

Cobertura: os ~60 mil linhas de código não-teste foram varridas por padrões (unwrap/expect, indexação, `clamp`, casts, `let _ =` sobre `Pending`, gestos, subscrições, threads, I/O, `Box::leak`). Leitura integral: `lm-port` (todos os arquivos), `lm-gui-app/main.rs`, `lm-gui/core.rs`, `lm-gui/tiles.rs`, `lm-gui/live.rs`, `lm-gui/workspace.rs` (partes de abertura, notices, detached), `lm-gui-plugins/lib.rs`, `lm-gui-plugins/panels/guitarix.rs` (construção), `lm-port-fake/lib.rs` (operações e peaks). As telas grandes (`mixing.rs`, `instrument.rs`, `piano_roll.rs`, `composition/edit.rs`, `automation.rs`, `panel.rs`) foram revisadas nos pontos de risco encontrados pela varredura, não linha a linha. Achados nessas telas podem existir fora dos pontos listados.

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO | 1 |
| ALTO | 2 |
| MÉDIO | 9 |
| BAIXO | 11 |

Nenhum `unsafe` fora de `lm-sys-*`. Todos os crates têm `#![forbid(unsafe_code)]`.

**As 5 piores:**

1. CRÍTICO: `lm-gui-plugins/src/panel.rs:900-905`. `f64::clamp` entra em pânico quando o mínimo é maior que o máximo ou um limite é NaN. Os limites `start`/`end` vêm do valor do parâmetro na árvore, sem nenhum `clamp` antes. Fora de 0..1 (ou com `start < 0`), o zoom da forma de onda derruba o programa.
2. ALTO: `lm-gui/src/tiles.rs:37-47`. O cache de blocos de pico (`blocks`) só é liberado por recurso (`forget`), nunca por tamanho ou uso. Cresce sem limite durante a sessão. Os nomes de recurso (`names`) também nunca diminuem.
3. ALTO: `lm-gui/src/core.rs:103-105` com `lm-gui/src/tiles.rs:117-131` e `lm-gui/src/pages/composition.rs:577-581`. Na cliente (`RemotePort`), a resposta `Finished::Peaks` é descartada por `_ => {}`. A GUI repete `peaks()` para cada bloco em espera a cada quadro, sem deduplicar e sem podar `retry`, e mantém a animação ativa (redesenho contínuo).
4. MÉDIO: `lm-gui/src/screens/piano_roll.rs:465-471` e `2790-2800`. `Clip::read` percorre toda a árvore e reconstrói o `NoteMap` inteiro a cada chamada. Há 26 chamadas de `clip(cx)`/`notes(cx)`, incluindo a cada movimento de mouse durante arrasto. O custo cresce com o número de notas.
5. MÉDIO: `lm-gui/src/pages/marks.rs:385`. `(d.from0 + dt).clamp(0, d.to0 - min)` entra em pânico quando `to0 < min` (região menor que um passo da grade).

## CRÍTICO

### lm-gui-plugins

- **`lm-gui-plugins/src/panel.rs:900-901` e `905`** (CRÍTICO). `let bounded_from = (from + step_from).clamp(0.0, start);`, `let bounded_to = (to + step_to).clamp(end, 1.0);` e `(from + to_step).clamp(0.0, start)`. Os valores `start` e `end` vêm de `self.param(pk.start/end)` (linhas 305-333), que devolve o número cru da árvore sem limitar. Se `start < 0`, `end > 1` ou `start`/`end` forem NaN, `clamp` entra em pânico. Gatilho: ação de zoom na forma de onda (`wave_zoom`). Correção: limitar uma vez, `let (start, end) = (start.clamp(0.0, 1.0), end.clamp(0.0, 1.0))`, e usar `max`/`min` nos cálculos, sem `clamp` com limites variáveis.

## ALTO

### lm-gui

- **`lm-gui/src/tiles.rs:37-47`, `blocks: HashMap<(u32,u8,u32), Arc<PeakBlock>>`** (ALTO, crescimento sem limite). Cada bloco tem 256 pares (1 KB) e nunca sai do mapa, exceto quando o recurso é esquecido (`forget`, usado só por gravação). Com uma sessão longa, muitos takes e zoom em vários níveis, a memória só sobe. `names` (linha 39) também nunca diminui. Correção: limite em bytes com LRU, descartando blocos de recursos fora da tela ou do projeto atual; limpar `names` ao trocar de projeto.

- **`lm-gui/src/core.rs:103-105`, `lm-gui/src/tiles.rs:117-131`, `lm-gui/src/pages/composition.rs:577-581`** (ALTO, cliente). `for (id, answer) in self.port.finished()` ignora `Finished::Peaks` (`_ => {}`). O `TileCache::fetch` depende de o `RemotePort` guardar o bloco depois que ele chega (`peaks_asked`, `store_peaks` em `lm-remote/src/client.rs`). Enquanto o bloco está em espera, `fetch` chama `port.peaks()` de novo a cada quadro, para cada bloco visível e também para os que já saíram da tela (`retry` nunca é podado). Cada chamada abre um pedido novo no servidor, e `request_animation_frame` mantém o quadro ativo. Correção: tratar `Finished::Peaks` no `Core` (ou entregar ao `TileCache` pelo id), deduplicar pedidos em voo por chave e podar `retry` quando o bloco não está mais visível.

## MÉDIO

- **`lm-gui-plugins/src/panel.rs` (sem `on_release` nem `Drop`; `drag` e `graph` em 411-446, 671, 1063-1119)** (MÉDIO, gesto aberto). Um `PluginPanel` destruído no meio de um arrasto (janela fechada, instrumento trocado) deixa o gesto do núcleo aberto: `mouse_up` usa `WeakEntity::update`, que falha quando a entidade já foi solta. Correção: `cx.on_release` que faz `gesture_cancel` (ou `commit`) de `drag` e `graph`.

- **`lm-gui/src/screens/piano_roll.rs:381-391`, `1659-1668`; `lm-gui/src/workspace.rs:670-682`** (MÉDIO, gesto aberto). O `on_release`, o `on_blur` e o `observe_window_activation` do editor soltam notas e modo passo, mas não encerram o `Drag` com `g: GestureId` (`drag_up` só encerra no mouse-up). Fechar o editor ou perder a ativação no meio de um arrasto deixa o gesto aberto. Da mesma forma, `tempo_drag` (workspace) só encerra no mouse-up dentro da janela; se o mouse-up não chegar, o gesto de tempo fica aberto. Correção: encerrar (commit ou cancel) o gesto em `on_release`/`on_blur`, e também em `window_activation` falsa no workspace.

- **`lm-gui/src/screens/piano_roll.rs:465-471` e `2790-2800`** (MÉDIO, custo por quadro). `Clip::read(root, id)` varre `tracks` e `items` e constrói um `NoteMap` novo a cada chamada. `render` chama uma vez (linha 1975), mas cada `drag_move`/`mouse_move` chama `self.notes(cx)` de novo (linhas 804, 963, 1036...). Com clipes de milhares de notas, cada evento custa O(N) em alocações. Correção: guardar o clipe lido e invalidar pela revisão do feed (`Feed`/`Diff`).

- **`lm-gui/src/screens/piano_roll.rs:909-910` e `918-919`** (MÉDIO, pânico). `((from.0 - origin.start as f64).round() as i64).clamp(0, origin.length)` entra em pânico se `origin.length < 0`. O comprimento vem da árvore. Correção: `clamp(0, origin.length.max(0))`.

- **`lm-gui/src/pages/marks.rs:385`** (MÉDIO, pânico). `(d.from0 + dt).clamp(0, d.to0 - min)` entra em pânico quando `d.to0 < min`, por exemplo uma região vazia ou menor que um passo da grade. Correção: `.max(0).min(d.to0 - min)`, ou `if hi < 0 { 0 }`.

- **`lm-gui/src/core.rs:52-54, 94-108` (mapas `imported`, `entries`, `previews`)** (MÉDIO, crescimento). As respostas que esperaram ficam no mapa até alguém pedi-las (`take_entries`, `take_preview`, `take_imported`). Se o pedido foi substituído (busca nova) ou o navegador fechou, a resposta nunca sai. Correção: registrar os ids pendentes e descartar respostas sem dono; limitar o tamanho.

- **Uso sistêmico de `let _ = port.…`** (MÉDIO). Há 86 chamadas em `lm-gui` e `lm-gui-plugins` que jogam fora o `Pending` de `apply`, `run_action`, `set_preference` e `set_favorite`. Erros `Ready(Err)` nunca chegam ao usuário na tela (exemplos: `pages/transport.rs:70, 100, 115, 138`; `pages/midi_maps.rs:53, 59, 72, 80`; `screens/midi_binding.rs:39, 45, 70`; `pages/composition/edit.rs:370-371, 890-923, 1117, 1638`). Só o que o núcleo emite como notice aparece. Correção: um ponto único que converte `Ready(Err)` em toast (ou log), e tratar `Waiting` pelo `finished()`.

- **`lm-gui/src/model.rs:53-76` (`tempo_map`) e dependências** (MÉDIO, bypass da porta). A GUI reconstrói o mapa de tempo (`lm_time::TempoMap`) a partir da árvore, em vez de pedir ao núcleo. Dois conversores de tick/amostra podem divergir: quando o núcleo recusa um ponto, a GUI cai silenciosamente para 120 BPM. O cast `n as u8`, `d as u8` (linha 66) trunca numerador acima de 255 sem aviso. Além disso, `lm-gui` depende de `lm-state` (`keymap/presets.rs`, leitura de JSON) e `lm-time` (`model.rs`, `view.rs`, `pages/composition*.rs`) em código não-teste. São crates de formato e de tempo, não o motor, mas quebram a regra "só `CorePort`". `lm-gui-plugins` só usa `lm-plugins-effects` dentro de teste (`panels/eq.rs:630`, `#[cfg(test)]`), então está limpo.

- **`lm-gui-app/src/main.rs:130`** (MÉDIO, erro ignorado). `let _ = session.open(folder);`. Um projeto passado na linha de comando que não abre é ignorado em silêncio, e o programa segue com o projeto sem título. Correção: se falhar, mostrar notice ou sair com erro.

## BAIXO

- **`lm-gui/src/pages/transport.rs:175`, `lm-gui/src/pages/composition.rs:343`, `lm-gui/src/pages/composition/edit.rs:1165`** (BAIXO, `unwrap` em estado de arrasto). Seguros hoje, porque o estado foi testado uma linha antes. Substituir por `if let`/`let else` com o gesto já copiado.
- **`lm-gui/src/screens/piano_roll.rs:980`** (BAIXO). `(x.key as i32 + delta.1) as u8` sem `clamp`. Limitado só indiretamente pelo `clamp` de `dk` (linha 842). A linha 2384 já faz `clamp(0, 127)`. Aplicar o mesmo.
- **`lm-gui/src/pages/mixing.rs:244`** (BAIXO). `ch[...]` com `Scale::Choice` vazio entra em pânico. Hoje vem do esquema, então não acontece.
- **`lm-gui/src/bench.rs:139, 146`** (BAIXO). `partial_cmp().unwrap()`. Só roda com `LM_BENCH`, e durações de `Instant` não são NaN.
- **`lm-gui/src/model.rs:58, 72-73`** (BAIXO). `expect` em constantes válidas.
- **`lm-gui-plugins/src/panel.rs:606-607, 706, 1698-1700`** (BAIXO). `keys[…min(keys.len()-1)]` e `(i+1).min(cb.items.len()-1)` underflow se a lista estiver vazia. Listas vêm de `static`, então não acontece hoje. Usar `saturating_sub`.
- **`lm-gui-plugins/src/panels/guitarix.rs:235, 261`** (BAIXO). `Box::leak` por controle. É limitado: `build` roda uma vez, em `LazyLock` (linha 296). Intencional; documentar.
- **`lm-port/src/lib.rs` (padrões `fn … { Vec::new() }` / `Err("…unsupported")`)** (BAIXO). Os defaults de `import`, `midi_learn`, `unmap`, `ui`/`set_ui`, `plugins` e `plugin_schema` respondem "não suportado" sem aviso. Servem de transição; quando a porta estiver completa, trocar por métodos obrigatórios para que um port novo não compile sem eles.
- **`lm-port-fake/src/lib.rs:302`** (BAIXO). `OperationId(1_000_000 + ops.len() + pumps*7)` pode repetir um id depois de remoções. Usar contador monotônico.
- **`lm-port-fake/src/lib.rs:307-321`** (BAIXO). `thread::Builder::spawn(...).ok()`: se o spawn falhar, a operação nunca marca `done` e o carregamento fica para sempre. É só do fake.
- **`lm-port-fake/src/lib.rs:896`** (BAIXO). `xrun_marks.push` sem limite, em um fake que roda sem parar.

## O que está bom

- Nenhum `unsafe`. `#![forbid(unsafe_code)]` em todos os crates.
- Notas ao vivo: `screens/live.rs` tem um `Live` por janela que registra o que começou e solta tudo em mouse-up, perda de foco, desativação e fechamento (`on_release`). Os `all_notes_off` por faixa cobrem notas órfãs.
- Subscrições: todas são guardadas em `_subs`/`_sub`. A janela do workspace e as páginas guardam o `workspace` como `WeakEntity`, então não há ciclo `Workspace` ↔ filho. O `Core` não guarda nenhuma entidade da GUI.
- Pump: a tarefa de 10 ms (`core.rs:50-66`) para quando a entidade é solta (`this.update` falha → `break`). `_pump` é guardado.
- Pendências: `Pending::Waiting` é tratado em `browser.rs` (`take_entries`/`take_preview`), em `tiles.rs` (volta a pedir) e em `lm-gui-plugins` (`take_imported`). O tipo `Pending` tem `#[must_use]`.
- Carregamento de arquivos fora da thread principal: as leituras de peso de `panel.rs:1380` e `1467` usam `background_executor().spawn` com limite de 64 MB. A decodificação PNG (`raster::decode_png`) acontece uma vez, por `OnceLock`.
- Notices: `workspace.rs:collect_toasts` deduplica por `(id, repeats)`, limita `toasts` (`TOASTS_MAX`) e poda `seen_notices`.
- Índices de lista (`piano_roll.rs:328-332`, `automation.rs` com `i > 0`, `tempo.rs:404` com `d.index >= pts.len()`, `browser.rs:step` com `rows.is_empty()`, `mixing.rs:move_selection`) são guardados antes de indexar.
- Tamanho de tela e teclas lidos da árvore têm limite (`kept("grid")` em `piano_roll.rs:328`; `steps(clip)` com `.max(1)`; `transpose` com verificação antes de mover).
- Aberturas de tela em `workspace.rs:open` substituem (não duplicam) uma tela já aberta. Telas destacadas registram o retorno em `should_close`.
- `lm-port` é só tipos e trait; sem IO nem pânico. `TelemetryView::peak` limita o canal antes de fatiar.

## Resultado da confirmação

| Achado | Veredito | Commit |
|---|---|---|
| CRÍTICO panel.rs `wave_zoom` clamp | confirmado e corrigido: `start`/`end` limitados a 0..1 (NaN vira 0) e `fit_between` sem pânico | ff9402c5a |
| Mesmo padrão em outros painéis (graph, schemas de plugin, `Range::fit`, fader, lomm, graphs, instrument) | confirmado e corrigido: `fit_between` / `max().min()`, limites de schema podem vir invertidos ou NaN | ff9402c5a |
| ALTO tiles.rs cache de blocos sem limite | confirmado e corrigido: teto de 16384 blocos (16 MB); `names` não limpo (só cresce com recursos distintos, sem limpeza) | commit seguinte |
| ALTO `Finished::Peaks` descartado, repetição por quadro | parcialmente corrigido: `retry` agora só repete blocos ainda visíveis e a animação para; deduplicação de pedidos em voo no `RemotePort` corrigida na rodada de desenho (`peaks` devolve `Waiting` com o id do pedido já em voo); o `Core` ignorar `Finished::Peaks` fica com o agente de lm-gui | f0d35be06 |
| MÉDIO marks.rs:385 clamp | confirmado e corrigido | ff9402c5a |
| MÉDIO piano_roll.rs:909/918 clamp com `length < 0` | confirmado e corrigido | ff9402c5a |
| Novos achados do mesmo padrão: export.rs bars, edit.rs TrimStart, piano_roll.rs:980 cast | confirmado e corrigido | ff9402c5a / commit seguinte |
| MÉDIO gestos abertos (panel, piano_roll, tempo_drag) | confirmado e não corrigido: exige `on_release` com cancelamento por gesto em várias telas; fora do fôlego desta rodada | n/a |
| MÉDIO `Clip::read` por evento | confirmado e não corrigido: cache invalidado por revisão do feed é mudança de desenho | n/a |
| MÉDIO mapas `imported`/`entries`/`previews` | confirmado e não corrigido: exige registro de ids pendentes | n/a |
| MÉDIO `let _ = port...` sistêmico | confirmado e não corrigido: exige ponto único de toast | n/a |
| MÉDIO `tempo_map` na GUI | confirmado e não corrigido: mudança de arquitetura | n/a |
| MÉDIO main.rs:130 `session.open` | falso: `open` devolve `OperationId` (operação mostrada na UI), não `Result` | n/a |
