# Varredura de qualidade: dsp-plugins

Área: `lm-dsp`, `lm-plugin`, `lm-plugins-effects`, `lm-plugins-instruments`, `lm-plugins-noteeffects`, `lm-plugins-tripleoscillator`, `lm-plugins-testsine`, `lm-expr`. Auditoria somente leitura, base `origin/master` (b051d6281).

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO | 2 |
| ALTO | 2 |
| MÉDIO | 9 |
| BAIXO | 8 |
| **Total** | **21** |

Os 5 piores:

1. **CRÍTICO** `lm-dsp/src/sfxr.rs:492`: o laço `while rendered < pitched` roda pitched/BLOCK vezes; com uma afinação (Tuning) enorme a frequência vira `inf`, `pitched` satura em `i32::MAX` e a thread de áudio trava.
2. **CRÍTICO** `lm-dsp/src/band_limited_wave.rs:65` e `:103`, usado em `lm-dsp/src/lb302.rs:436`: as tabelas do LB302 são geradas na primeira nota, dentro do callback de áudio. `band_limited_wave::prepare()` existe mas nunca é chamada.
3. **ALTO** `lm-plugins-effects/src/effect.rs:135`: parâmetros de efeito chegam sem checar `is_finite` nem faixa; um NaN fica preso no estado de filtros e realimentações até o reset.
4. **ALTO** `lm-plugin/src/instrument/mod.rs:524`: `velocity.clamp(0.0, 1.0)` devolve NaN para entrada NaN; a Tuning/Pan de `notes.rs:373` também não é validada, e o NaN vai para a fase dos osciladores de cada voz.
5. **MÉDIO** `lm-dsp/src/oscillator.rs:193`: `69 + semitones` com `semitones = i32::MAX` (freq infinita) estoura: panic em debug, wrap em release.

---

## Achados por crate

### lm-dsp

**[CRÍTICO] `lm-dsp/src/sfxr.rs:486-505` (render)**
`pitched = whole.max(0) as usize` com `whole = due as i32` (saturado). O laço externo avança `BLOCK` por iteração, mas o laço interno só escreve frames enquanto `source < rendered + count`, então para pitch grande a maioria das iterações não produz saída: ~pitched/BLOCK chamadas a `synth.update` por bloco. A entrada vem de `lm-plugins-instruments/src/sfxr.rs:158` (`pitch = freq / BASE_FREQ`), e `freq` cresce sem limite com a Tuning de `NoteExpression` (`notes.rs:373`, sem clamp). Tuning de algumas centenas de semitons já causa glitch; de ~1000 trava.
Correção: limitar `pitch` (por exemplo a [1/64, 64]) antes do render e tratar `freq` não finito como silêncio; limitar `pitched` a algo como `frames * 64`.

**[CRÍTICO] `lm-dsp/src/band_limited_wave.rs:65-66, 103-105` com `lm-dsp/src/lb302.rs:254` e `:436`**
`waves()` é `OnceLock::get_or_init`: a primeira chamada de `oscillate()` gera as quatro MipMaps inteiras (somas de harmônicos sobre todas as tabelas). `Lb302::new` não chama `prepare()`, e a construção do plugin (`lm-plugins-instruments/src/lb302.rs:278`) também não. Resultado: a primeira nota do LB302 faz o trabalho de construção de tabelas dentro do callback de áudio, com alocação e glitch audível. Os outros usuários (Monstro via `saw_scales()`, `monstro.rs:303`) já pré-aquecem pelo `new`; o LB302 é o caso faltante.
Correção: chamar `band_limited_wave::prepare()` em `Lb302::new` (ou no `build` do plugin), como o `prepare_wave_tables()` do TripleOscillator (`lm-plugins-tripleoscillator/src/lib.rs:217`).

**[MÉDIO] `lm-dsp/src/oscillator.rs:191-194` (`wave_table_band_from_freq`)**
`(12.0 * log2f(freq/440)).ceil() as i32` satura em `i32::MAX` para freq infinita; `69 + semitones` então estoura (panic em debug, wrap em release, onde a faixa final corrige o valor). Mesmo padrão em `lm-plugins-instruments/…/vibed.rs:123` e `:205`: `(sr/pitch) as i32 + 1` estoura com pitch 0 (freq 0 por Tuning muito negativa).
Correção: `saturating_add` e clamp de `semitones` antes da soma; limitar pitch/freq na entrada da voz.

**[MÉDIO] `lm-dsp/src/delay.rs:82-86` (`Lfo::set_frequency`)**
A guarda `frequency < 0 || frequency > nyq || frequency == self.frequency` deixa passar NaN (todas as comparações são falsas). A fase fica NaN e `%=` não a recupera; o LFO congela para sempre (até `reset`). O sinal não é corrompido porque `set_length` ignora NaN, mas o efeito deixa de modular.
Correção: `if !frequency.is_finite() || …` no início da função.

**[MÉDIO] `lm-dsp/src/dynamics.rs:971`, `:1004`, `:1040` (vari_mu, optical, feed-forward)**
`solve()` (até 30 iterações com closures que chamam `knee`/`exp`/`log`) roda **por amostra** no caminho de compressão. É custo de CPU no áudio, não uma falha de corretude, mas multiplica o trabalho por amostra por até ~32.
Correção: resolver por piece (ou por bloco), ou usar aproximação de passo fixo.

**[MÉDIO] `lm-dsp/src/xpressive.rs:168` (`smooth`)**
`size = ((smoothness * 5.0) as i32 | 1) as usize` sem limite superior. A entrada vem de `lm-plugins-instruments/src/xpressive.rs:265` (`s.int(...) as f32`, valor cru do estado). Um inteiro grande (por exemplo 10^9) aloca `vec![0; 2^31]` e faz convolução O(n·size). Roda na construção (não no áudio), mas trava ou estoura memória.
Correção: clamp de `smoothness` a `[0, SMOOTH_MAX]` dentro de `smooth()`.

**[BAIXO] `lm-dsp/src/granular_pitch_shifter.rs:368` (`process` chama `prepare`)**
`prepare()` faz `ring.resize` e `clear` (alocação no áudio). É alcançável só se `s.range` diferir do `self.range` do processador, e hoje não ocorre (o range é campo de estado fixo por processador, `lm-plugins-effects/src/granular_pitch_shifter.rs:122`). É ramo morto perigoso.
Correção: remover o ramo ou trocá-lo por `debug_assert_eq!` mais um erro de build.

**[BAIXO] `lm-dsp/src/delay.rs:57-59` (`StereoDelay::tick`, `MonoDelay::tick`)**
Com `length == 0` (`time` 0 ou NaN), `read = write_index` e o efeito devolve a amostra mais antiga do buffer (até 20 s atrás), não atraso zero. A faixa do esquema (0,01 s mínimo) protege o caso normal.
Correção: `length.max(1.0)` em `set_length`.

**[BAIXO] `lm-dsp/src/frequency_shifter.rs:280` (`reset`)**
`reset()` não zera `true_shift`, `true_phase` nem `true_delay`; o botão de pânico deixa glides antigos. Junto com o achado de NaN de `true_delay` (ver ALTO `effect.rs`), um NaN persiste mesmo após `reset`.
Correção: zerar esses campos em `reset()` (e não deixar NaN entrar, ver ALTO).

### lm-plugin

**[ALTO] `lm-plugin/src/instrument/mod.rs:524` (`event`, NoteOn) e `lm-plugin/src/instrument/notes.rs:373` (`express`)**
`velocity.clamp(0.0, 1.0)` devolve NaN para NaN (`f32::clamp` preserva NaN); a velocidade vai para `VoiceNote` e daí para o nível da voz e a saída. `express()` guarda `Tuning`/`Pan` sem checar `is_finite`: `Tuning` NaN gera frequência NaN e a fase do oscilador (`osc.phase += step`, por voz) fica NaN até a nota acabar. A mesma falta existe em `lm-plugin/src/note_effect.rs` (`Expressions::set`).
Correção: `if velocity.is_finite() { velocity.clamp(0,1) } else { 0.0 }` (ou `max(0).min(1)`, que ignora NaN); em `express`, ignorar valores não finitos e limitar Tuning (por exemplo ±120 semitons).

**[MÉDIO] `lm-plugin/src/instrument/mod.rs:447` (`Instrument::new`) e `lm-dsp/src/nes.rs:222`**
Na construção, `synth.set_param(i, *v)` recebe o valor cru do estado, enquanto `Instrument::set_param` (linha 502) clampa e rejeita não finitos. O NES depende disso: `NOISE_FREQS[15 - s.noise_freq as usize]` (nes.rs:222) estoura se `ch4NoiseFreq > 15`. Hoje o schema (`lm-state`) rejeita o valor fora da faixa, então não há falha alcançável; é fragilidade.
Correção: passar `v.clamp` também na construção e usar `15usize.saturating_sub(...)` no NES.

**[BAIXO] `lm-plugin/src/instrument/mod.rs:502` (`set_param`) e `lm-plugin/src/note_effect.rs:133` (`NoteFx::set`)**
`value.clamp(lo, hi)` entra em pânico se `lo > hi` ou se um limite for NaN. Os limites vêm do esquema (constantes), então é só fragilidade.
Correção: `if lo <= hi { clamp } else { value }`, ou validar no build.

**[BAIXO] `lm-plugin/src/instrument/notes.rs:192` e `:250`**
O campo `dropped` é incrementado quando notas são recusadas por falta de espaço, mas nunca é lido nem reportado. Notas são descartadas sem aviso.
Correção: expor o contador (diagnóstico) ou removê-lo.

**[BAIXO] `lm-plugin/src/instrument/shaping.rs` (final do arquivo)**
`#[allow(dead_code)] const _: usize = GRID;` é um resto sem função.
Correção: remover.

### lm-plugins-effects

**[ALTO] `lm-plugins-effects/src/effect.rs:135-139` (`EffectProcessor::event`) e `lm-plugins-effects/src/effect.rs` (`Knobs::set`, linha 250)**
Todo `EventKind::Param` vai direto para `set_param` de cada efeito, sem `is_finite` e sem clamp à faixa do esquema (o `Instrument` e o `NoteFx` fazem as duas coisas). Consequências concretas: NaN em `reverb_sc.rs` entra na realimentação (`v0 *= feedback`, `filter_state`) e fica no buffer até o reset; NaN em `granular_pitch_shifter.rs` vai para `ring[at]` (`filtered + out*feedback`) e envenena o anel; NaN em `frequency_shifter.rs` (`damp_state`, `true_delay` com `clamp`, que propaga NaN) também persiste. Mesmo sem NaN, valores fora da faixa (feedback > 1) podem divergir.
Correção: em `EffectProcessor::event` aplicar a mesma regra de `Instrument::set_param` (rejeitar não finito, clamp à faixa do esquema), e fazer `Knobs::set` rejeitar não finitos.

**[MÉDIO] `lm-plugins-effects/src/granular_pitch_shifter.rs` e `lm-dsp/src/granular_pitch_shifter.rs:126`, `:307`**
`GRAIN_ROOM = 64` e `grains.push(...)` sem limite de capacidade. Com density 16 e twitch alto, a contagem de grãos simultâneos chega perto de 64 (2·density, dobrado pelo twitch), e um push acima da capacidade realoca na thread de áudio.
Correção: pré-reservar um teto calculado (por exemplo 256) ou descartar grãos novos ao atingir o teto, como o Arpeggiator faz.

**[MÉDIO] `lm-plugins-noteeffects/src/scale.rs:102` (`ScaleFx::event`)**
Quando `sent` está cheio, o NoteOn sai (`out.push`) mas não é gravado; o NoteOff correspondente cai no ramo `None` e sai com a tecla original, não a mapeada. Resultado: nota presa ou NoteOff para a tecla errada, em uma situação de muitas notas.
Correção: não emitir o NoteOn quando `sent` estiver cheio (ou dimensionar `sent` pelo máximo de notas).

**[BAIXO] `lm-plugins-noteeffects/src/chords.rs:84-88`**
Com `made` cheio, a função retorna no meio do acorde: sai um acorde parcial, sem aviso. Mesmo padrão de `notes.rs:dropped`.
Correção: verificar capacidade para o acorde inteiro antes de começar, ou aceitar e registrar.

**[BAIXO] `lm-plugins-noteeffects/src/note_effect.rs` (`IdCounter::next`)**
Ao chegar em `0x7FFF_FFFF`, o contador volta a `GENERATED_IDS` e pode reutilizar números de notas ainda soando. Com 2^31 notas é improvável, mas a regra do próprio módulo é "nunca reusar número de nota que ainda soa".
Correção: aceitar o wrap só com nota-off garantido, ou usar `u64`.

### lm-expr

**[MÉDIO] `lm-expr/src/compile.rs:90` e `:101`**
`argc = args.len().min(255) as u8`: com mais de 255 argumentos em `min`/`max`/`sum`/função de usuário, o `Ins::Many`/`Ins::Call` leva só os 255 últimos valores e a profundidade calculada (`1 - argc`) fica menor que a real. O `stack_len` sai subestimado e `put` ignora as escritas fora do slice: o resultado é silenciosamente errado (zeros), sem pânico. Expressões de até 8000 tokens chegam lá.
Correção: rejeitar com `CompileError` mais de 255 argumentos em `parser.rs` (`arguments`, `builtin_call`).

**[MÉDIO] `lm-expr/src/parser.rs` (`MAX_TOKENS` = 8000), `lm-expr/src/compile.rs` (`Emitter::node`), `lm-expr/src/opt.rs`**
A profundidade do parser é limitada (MAX_DEPTH 200 via `expression`, verificado: toda recursão passa por `expression`). Mas uma cadeia à esquerda de ~4000 termos (`a+a+a…`, dentro de 8000 tokens) produz uma árvore de ~4000 níveis, e `Emitter::node`, `opt` e o `Drop` de `Node` recursam nessa profundidade. Não medi o uso de pilha; em uma thread de 2 MiB pode estourar. O próprio comentário em `parser.rs:14-15` reconhece o risco.
Correção: limitar a profundidade da árvore no parser (por exemplo 512 níveis de nós) ou tornar `Emitter::node` iterativo.

### lm-plugin / lm-plugins-instruments

**[MÉDIO] `lm-plugins-instruments/src/sfxr.rs:158`**
Ver o CRÍTICO de `lm-dsp/src/sfxr.rs:492`: a correção principal fica no lado do `pitch`.

---

## O que está bom

- **Arquitetura de preparação:** `new()`/`build` alocam (`VoicePool`, `Run::new` com capacidades fixas, `Scratch` em `Box`), e o processamento usa só slices de tamanho `GRID`. Os `Vec::with_capacity(MAX_*)` de `Run`, `Arpeggiator`, `Chords`, `ScaleFx`, `Expressions` têm guarda de capacidade antes de cada `push` (`notes.rs`, `arpeggiator.rs:331,350`, `chords.rs:85-91`, `scale.rs:106`).
- **Tabelas pré-aquecidas quando é o caso:** `prepare_wave_tables()` no build do TripleOscillator e do Organic; `tube_models()` em `GainStage::new`; `saw_scales()` em `Monstro::new`; `rawwaves::prepare()` chamado em `lm-plugins-stk/src/lib.rs:219`. O único caso faltante é o LB302 (CRÍTICO acima).
- **Clones no áudio sem alocar:** `PlaybackState::clone_from` copia o buffer em `*self.buffer = *source.buffer` (sample_player.rs:109); os `Voice` de sfxr/monstro/slicer usam `Box<[..;N]>` ou tipos `Copy`, e `copies_voices()` é `false` no xpressive, que tem histórico com `Vec` (xpressive.rs:245).
- **Índices de tabela e atraso bem guardados:** `ring_sample` e `hermite_sample` tratam o wrap do anel (granular, frequency_shifter); `StereoEnhancer` usa `rem_euclid`; `MultitapLine` trata `offset` enorme com `offset % size`; `Waveform::sample` e `user_wave_sample` tratam fase logo abaixo do período; `Instrument::set_param` e `NoteFx::set` rejeitam não finitos.
- **Spectrum Analyzer:** toda a análise roda na thread worker; o áudio só faz `rtrb::Producer::push` sem bloqueio, com flag `stop` (Relaxed) e `Drop` que a liga (`spectrum_analyzer.rs:218-221`).
- **Interpolador do sample player:** o conversor (`lm-plugins-instruments/src/src_port.rs`) consome toda a entrada disponível e carrega `last_value`/`last_position` entre chamadas; não há ponto em que o view fique preso e silencie a saída.
- **lm-expr VM:** `eval` não aloca nem entra em pânico (`at`/`put` usam `get`/`get_mut`), saltos são só para frente (sem laço infinito), `fast_exp` é square-and-multiply com expoente ≤ 60 (`opt.rs`), `Root`/`Roundn`/`Shl` tratam NaN e fora de faixa sem pânico; `lexer::number_end` e `string_to_real` limitam o expoente a 4 dígitos e usam `saturating_*`.
- **Parser:** `branch`, `bracketed`, `ternary`, `arguments` e `Sub`/`Add` unários passam todos por `expression()`, que conta a profundidade; `builtin_call` confere aridade antes de `args.remove(..)`.
- **Índices de parâmetro:** `Knobs::get` e `Expr::call_count` usam índices constantes ou `get`; `Rng::below` tem `assert!(n > 0)`, e o único chamador do áudio com n variável (`arpeggiator.rs:302`) faz `.max(1)`.

## Cobertura (honesta)

Lido por completo: `lm-plugin/src/instrument/mod.rs`, `notes.rs`, `shaping.rs`, `note_effect.rs`, `stream.rs` (caminho de render); `lm-plugins-effects/src/effect.rs`; `lm-plugins-noteeffects/src/arpeggiator.rs`, `chords.rs`, `scale.rs`; `lm-plugins-tripleoscillator/src/lib.rs`; `lm-dsp/src/delay.rs`, `modulation_effects.rs` (MultitapLine, MonoDelay), `granular_pitch_shifter.rs`, `frequency_shifter.rs` (núcleo), `oscillator.rs` (tabelas e `update`), `reverb_sc.rs` (núcleo), `sample_player.rs` (`play`/`render`), `sfxr.rs` (render); `lm-plugins-instruments/src/src_port.rs`, `sfxr.rs`; `lm-expr/src/parser.rs`, `vm.rs`, `ops.rs`, `lexer.rs`, `compile.rs`, `xpressive.rs` (host state); `lm-dsp/src/xpressive.rs` (smooth).

Por padrão (grep de alocação, pânico, casts de índice, divisões) e leitura dirigida apenas: `lm-dsp/src/dynamics.rs`, `lb302.rs`, `monstro.rs`, `nes.rs`, `organic.rs`, `watsyn.rs`, `kicker.rs`, `drum_machines.rs`, `slicert.rs`, `vibed.rs`, `drumsynth.rs`, `patman.rs`, `loudness.rs`, `tube.rs`, `gain_stage.rs`, `channel_strip.rs`, `master.rs`, `eq.rs`, `filters.rs`, `basic_filters.rs`, `envelope.rs`, `sound_shaping.rs`, `small_effects.rs`, `distortion_effects.rs`, `slew_distortion.rs`, `oversampling.rs`, `filter_effects.rs`, `dynamics_processor.rs`, `compressor.rs`, `lomm.rs`; efeitos `oscilloscope.rs`, `vectorscope.rs`, `eq.rs` e demais wrappers de `lm-plugins-effects`; instrumentos `lb302.rs`, `monstro.rs`, `kicker.rs`, `watsyn.rs`, `vibed.rs`, `organic.rs`, `nes.rs`, `bit_invader.rs`, `xpressive.rs`, `monstro.rs`. Esses arquivos podem ter achados que não apareceram nesta passada; a próxima varredura deve lê-los inteiros.

## Resultado da confirmação

| Achado | Veredito | Commit |
|---|---|---|
| CRÍTICO sfxr.rs laço com afinação absurda | confirmado e corrigido (pitch limitado a [1/64, 64], não finito vira silêncio) | 9f56158aa |
| CRÍTICO LB302 tabelas na primeira nota | confirmado e corrigido (`Lb302::new` chama `prepare()`) | 9f56158aa |
| amp.rs `.nam` com cabeçalho hostil | confirmado e corrigido (arquivo > 64 MiB e taxa fora de 8 kHz a 768 kHz ou razão fora de 1/64..64 viram `Err`) | ver log (NAM amp) |
| ALTO effect.rs parâmetros não finitos | confirmado e corrigido (rejeita não finito em `event` e `Knobs::set`); o clamp à faixa do esquema não foi feito: `BuildContext` não traz as faixas, exigiria mudar a interface | 1e617dda2 |
| ALTO velocity NaN / Tuning / Pan | confirmado e corrigido (velocidade, Tuning limitado a ±120, não finito ignorado, `Expressions::set`) | 1e617dda2 |
| MÉDIO oscillator.rs `69 + semitones` | confirmado e corrigido (`saturating_add`) | ver log (Bound band index) |
| MÉDIO Lfo::set_frequency NaN | confirmado e corrigido | idem |
| MÉDIO xpressive smooth sem limite | confirmado e corrigido (`SMOOTH_MAX`) | idem |
| MÉDIO lm-expr mais de 255 argumentos | confirmado e corrigido (parser rejeita) | idem |
| MÉDIO ScaleFx `sent` cheio | confirmado e corrigido (nota descartada em vez de NoteOff errado) | idem |
| MÉDIO vibed `(sr/pitch) as i32 + 1` | confirmado e não corrigido: coberto em parte pelo limite de Tuning; não revisado a fundo | - |
| MÉDIO dynamics `solve()` por amostra | não corrigido: custo de CPU, não corretude; mudar mexeria no som | - |
| MÉDIO granular grãos sem teto | não verificado | - |
| MÉDIO Instrument::new / NES | falso na prática: o esquema rejeita valores fora da faixa (como o relatório admite) | - |
| MÉDIO lm-expr profundidade de árvore | não verificado | - |
