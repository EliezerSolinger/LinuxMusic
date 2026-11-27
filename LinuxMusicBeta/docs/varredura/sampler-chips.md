# Varredura de qualidade: sampler-chips

Área: `lm-sampler`, `lm-plugins-sampler`, `lm-plugins-sid`, `lm-plugins-freeboy`, `lm-plugins-opulenz`, `lm-plugins-guitarix`, `lm-plugins-ampandir`, `lm-plugins-stk`, `lm-smf`, `lm-osc`, `lm-midimap`. Auditoria só de leitura, sobre `origin/master` (b051d6281). Cada achado foi conferido lendo o código ao redor. Nada foi compilado nem executado.

Os leitores de formato (SF2, SFZ, PAT, GIG, SMF, OSC) tiveram prioridade. Para cada um verificou-se: se todo comprimento, deslocamento e contagem lido do ficheiro é validado antes de indexar, fatiar ou alocar; se há overflow em `offset + size`; se chunk de tamanho zero pode causar ciclo infinito; se a recursão tem limite; e se ficheiro malformado devolve erro em vez de pânico.

## Resumo

| Severidade | Quantidade |
|---|---|
| CRÍTICO (pode travar, corromper dados ou falhar o áudio) | 7 |
| ALTO (resultado errado, regra do audio thread quebrada, ou overflow em entrada do usuário) | 4 |
| MÉDIO (fragilidade ou determinismo) | 13 |
| BAIXO (limpeza) | 15 (B1-B16, sem B4) |

### Os 5 piores

1. `lm-plugins-opulenz/src/opl.rs:305-311` (com 318, 323, 375, 380, 395): tabelas `ar_table`/`dr_table` de 75 entradas indexadas por `taxa + ksr`, que chega a 75. Pânico de índice no audio thread com uma configuração alcançável pela interface. (C6)
2. `lm-sampler/src/import/gig.rs:482-497` (e 677): combinações de dimensões sem limite. Um `.gig` malformado explode em memória e tempo na importação. (C4)
3. `lm-plugins-ampandir/src/amp.rs:96-110`: a taxa de um `.nam` entra sem validação em `model_frames` e em `vec![0.0; model_frames]`. Alocação gigante aborta o processo, e o `catch_unwind` não apanha abort. (C7)
4. `lm-plugins-sampler/src/load.rs:17-18`: `vec![0.0; total * channels]` com `info.frames` vindo do cabeçalho do ficheiro. Mesmo problema de abort. (C5)
5. `lm-sampler/src/import/sf2.rs:321` (com 342, 364, 365): fatias com índices de bag não monotónicos. Um SF2 malformado causa pânico. (C1)

---

## lm-sampler

### CRÍTICO

**C1. `src/import/sf2.rs:321, 342, 364, 365`: pânico em SF2 malformado.**
As fatias `&self.pgen[g0.min(n)..g1.min(n)]`, `&self.pmod[m0..m1]`, `&self.igen[..]` e `&self.imod[..]` panicam quando `g0 > g1` ou `m0 > m1`. Isso acontece quando os índices de bag do ficheiro não são crescentes. O `.min(len)` não corrige a ordem.
Correção: validar em `bag_range` que `from ≤ to` e que os índices são crescentes, devolvendo `sampler.sf2.invalid`. Alternativa: `.get(a..b).unwrap_or(&[])`.

**C2. `src/import/sf2.rs:446-447, 553-556`: pânico com amostra de comprimento zero.**
`decode` rejeita só `end < start`, então `start == end` passa e dá `frames == 0`. Em `build_zone`, `offset(...).clamp(0, frames - 1)` é `clamp(0, -1)`, que panica. `clamp(start + 1, frames)` também panica.
Correção: rejeitar `h.end <= h.start` em `decode`, ou saltar a zona quando `frames == 0`.

**C3. `src/import/sfz.rs:303-306`: pânico com amostra de comprimento zero.**
`z.start = int(r, "offset").unwrap_or(0).clamp(0, frames - 1)` é avaliado para toda região. `read_wav` aceita um WAVE sem frames (devolve `Ok` com `Pcm` vazio), então `clamp(0, -1)` panica. Com o opcode `end`, `clamp(1, frames)` também panica.
Correção: rejeitar ou saltar amostra vazia antes de `region()`, com erro `sampler.sfz.empty_sample`.

**C4. `src/import/gig.rs:482-497` e `677`: explosão combinatória com `.gig` malformado.**
`zones` vem do byte `3lnk` sem limite a `1 << bits` (linha 677: `if zones == 0 { 1 << bits } else { zones }`). `picks_of` produz até 255 escolhas por dimensão, e `combos` é o produto de todas as dimensões, até 8. Oito dimensões de 1 bit com `zones = 255` dão `255^7` combinações, cada uma clonada em `combo.clone()`. A importação trava ou esgota a memória.
Correção: `zones = min(zones, 1 << bits)` na leitura, e um teto no número de combinações antes da expansão.

### ALTO

**A1. `src/interp.rs:124-172` (linhas 126, 129-130, 135-138, 156, 160): aritmética sobre posição enorme.**
Com `position` gigante, `position.floor() as i64` satura em `i64::MAX`, e `i + 1`, `i + 2` ou `first + t` estouram. Em debug é pânico no audio thread; em release a leitura vira lixo. O caminho alcançável é `note_expression(Tuning)`: não há limite em `engine.rs:729` (`v.detune = t * 100.0`), e o passo exponencial (`engine.rs:883`, `exp2f(cents / 1200)`) pode levar a posição a valores astronômicos. Uma voz em loop mantém essa posição.
Correção: limitar `cents`/`step` (por exemplo ±4800 cents) e encerrar a voz quando a posição sair de `[-len, 2·len]`. No `read`, usar aritmética saturada.

**A2. `src/import/sfz.rs:78-86`: `#include` sem orçamento total.**
A profundidade é limitada a 8, mas não o número de inclusões. Um ficheiro que se inclui `k` vezes expande `k^8` vezes, relendo cada vez do disco. A importação trava ou esgota a memória.
Correção: limite do total de inclusões (por exemplo 64) e detecção de ciclos.

**A3. `src/import/sfz.rs:54-55, 306, 309`: overflow inteiro com valores do ficheiro.**
`(octave + 1) * 12 + semis` é `i32` sobre um valor que pode chegar perto de `i32::MAX` (`key=c2000000000`). `e + 1` em `end` e em `loop_end` estoura quando `e == i64::MAX`; o `int()` satura para `i64::MAX` a partir de floats grandes. Em debug é pânico; em release dá fim ou fim de loop errados.
Correção: `checked_add`/`checked_mul` ou `saturating_*`, e rejeitar valores fora do intervalo do campo.

**A4. `src/import/gig.rs:619` (com 133 e 677): índice de zona de velocidade fora do intervalo.**
`usize::from(v) / d.zone_size()` pode dar índice `≥ zones`. Com `zones = 3`, `zone_size = 42`, então `v = 126..127` dão zona 3. A máscara `& mask` deixa o valor passar, e o slot escolhido pertence a outra zona: velocidades altas tocam a amostra errada ou ficam mudas.
Correção: `.min(zones - 1)`.

### MÉDIO

**M1. `src/engine.rs:839-855`: a saída depende do tamanho do bloco do host.**
`render` recomeça os sub-blocos de `SUB` frames em zero a cada chamada. Pitch, glide, filtro e pan são atualizados no início de cada sub-bloco. O mesmo instrumento com `render(64)` e com `render(33)+render(31)` produz áudio diferente, então o bounce difere da reprodução.
Correção: alinhar a grelha de sub-blocos ao frame absoluto (`self.frame % SUB`) e testar a invariância ao tamanho do bloco.

**M2. `src/import/sfz.rs:90-91`: ordem de iteração de `HashMap` no `#define`.**
`for (k, v) in &defines { line = line.replace(k, v) }` aplica as substituições em ordem aleatória por processo. Macros que se referem a outras dão resultados diferentes entre execuções.
Correção: `BTreeMap`, ou expansão até ponto fixo.

**M3. `src/model.rs:322-333`: `sample_frames` é quadrático.**
`seen.contains(&p)` sobre `Vec` é O(zonas × amostras distintas). Com `MAX_ZONES = 200_000` pode levar segundos.
Correção: `HashSet<*const Pcm>`.

**M4. `src/import/sf2.rs:201-217`: `chunks()` aceita chunk truncado em silêncio.**
Quando `8 + size` passa do fim, o corpo é cortado sem erro e os registos parciais são descartados. Um SF2 truncado importa sem avisar.
Correção: `sampler.sf2.invalid` quando `8 + size > len`.

**M5. `src/import/gig.rs:386-388` (`wave_of`) e `409-414`: busca linear por slot.**
`waves.iter().position` é O(slots × waves) no pior caso. Relevante só para ficheiros grandes.
Correção: índice por offset (`HashMap<u64, usize>`) construído uma vez.

### BAIXO

**B14. `src/import/sfz.rs:252`:** `folder.join(sample)` aceita caminhos absolutos e `..`, lendo ficheiros fora da pasta do SFZ. Não há exfiltração (o conteúdo vira amostra), mas o alcance é maior do que o necessário.
**B15. `src/import/sfz.rs:366-367`:** `int(...).max(0) as u32` trunca `group` e `off_by` acima de `u32::MAX` em silêncio.
**B16. `src/engine.rs:397`:** `velocity` NaN vira velocidade 0, e a nota é ignorada sem aviso.

---

## lm-plugins-sampler

### CRÍTICO

**C5. `src/load.rs:17-18`: alocação pelo cabeçalho do ficheiro.**
`let total = usize::try_from(info.frames)...; let mut data = vec![0.0f32; total * channels];` usa o número de frames declarado pelo libsndfile. Um cabeçalho que declara 10^12 frames causa alocação de terabytes: abort do processo (o `catch_unwind` não apanha abort), ou overflow em `total * channels` em release.
Correção: não pré-alocar pelo cabeçalho. Ler em blocos crescentes com teto, por exemplo proporcional ao tamanho do ficheiro.

### MÉDIO

**M6. `src/load.rs:62-68`: ficheiro temporário previsível.**
`std::env::temp_dir().join(format!("lm-sampler-{pid}-{n}.ogg"))` é escrito com `fs::write`. Em `/tmp` partilhado, outro usuário pode criar um symlink com esse nome antes. Além disso, `rate` é ignorado (`let _ = rate;`, linha 68).
Correção: nome aleatório com criação exclusiva (`O_EXCL`), e usar ou remover `rate`.

**M7. `src/source.rs:113-114, 165-166`: bank/program fora de intervalo viram 0 sem aviso.**
`u16::try_from(s.int("bank")?).unwrap_or(0)` toca o preset 0 quando o valor é inválido, sem erro para o usuário.
Correção: erro `plugin.sampler.preset` quando fora de `u16`.

### BAIXO

**B1. `src/kits.rs:93, 416`:** o caminho `CARGO_MANIFEST_DIR/../../../legado/data/...` é compilado no binário. É problema de higiene de release.

---

## lm-plugins-sid

Nenhum achado. O processador não aloca em `render`, `clone_from` é in-place, a fila de escritas do chip é limitada por `TICK` e `SCRATCH`, e os índices de registro (`base = v * 7`, até 22) estão dentro de `REGISTERS = 25`. `Sid::new` devolve erro (não pânico) para taxa inválida.

---

## lm-plugins-freeboy

### MÉDIO

**M8. `src/blip.rs:151` (`BlipBuffer::set_bass_freq`) e build de `src/lib.rs`:** a divisão `(i64::from(freq) << 16) / self.sample_rate` panica quando a taxa é 0. `GbApu::new` (`apu.rs:402`) chama `set_sample_rate`, que chama `set_bass_freq`. O build do freeboy não verifica `sample_rate == 0`, ao contrário do mallets. O `catch_unwind` do build converte o pânico em erro, mas o plugin não carrega.
Correção: a mesma guarda `sample_rate == 0 → Err` do mallets.

**M9. `src/lib.rs:364-376` (`FreeBoySynth::read`):** o ciclo `while done < left.len()` depende de `end_frame` produzir amostras. Se o fator de conversão for 0 (taxa muito baixa, `clock_rate_factor` arredonda para 0), `frames_avail` nunca sobe e o ciclo gira para sempre no audio thread.
Correção: se `end_frame` não produzir amostras, preencher silêncio e sair.

### BAIXO

**B2. `src/lib.rs:306`:** `(hi << 4) + lo` com valores de `wave` não limitados no `from_state` (`as_real().unwrap_or(0.0) as f32`, sem clamp). `i32` pode estourar com valores fora do intervalo do campo.

---

## lm-plugins-opulenz

### CRÍTICO

**C6. `src/opl.rs:198-199, 221-222, 254-257`, usado em `305-311, 318, 323, 375, 380, 395`: índice fora de `ar_table`/`dr_table` no audio thread.**
As tabelas têm 75 entradas (`[i32; 75]`, preenchidas em `for i in 60..75`). Os índices são `ar_table[a + ksr]` (com `a = ar << 2 ≤ 60`) e `dr_table[rr + ksr]` (com `rr ≤ 60`). `ksr = kcode >> ksr_shift` (linha 318), e `kcode` vai até 15 (bloco 7 com o bit alto do fnum). Com KSR ligado, `ksr_shift = 0`, então `ksr` chega a 15. `60 + 15 = 75` é índice fora do limite: pânico no `process`. A voz é marcada como morta, e o instrumento fica mudo.
Alcance pela interface: `op1Attack = 0` (ou `op1Decay`/`op1Release = 0`) com KSR ligado, tocando notas de bloco 7 (≈3 kHz e acima). Os padrões (`op1Attack = 14`, e KSR do op2 com `op2Attack = 1`) ficam dentro de 75, então não aparece no uso normal.
Correção: `ar_table` e `dr_table` com 76 entradas, preenchendo 60..76 (`ar`: `EG_AED - 1`; `dr`: `dr_table[60]`), ou limitar `ksr` a 14 em `calc_fcslot`. Acrescentar teste com `ar = 15`, `rr = 60`, `kcode = 15`.

### BAIXO

**B3. `src/lib.rs:309-316`:** `velocity` NaN dá atenuação 0 (volume máximo) em vez de ignorar a nota. Correção: tratar `!velocity.is_finite()`.

---

## lm-plugins-guitarix

### MÉDIO

**M10. `src/processor.rs:463`: parâmetro NaN ou infinito não é rejeitado.**
`value.clamp(spec.min, spec.max)` deixa NaN como está (`f32::clamp` só panica com limites NaN, não com valor NaN). O NaN vai ao DSP via `set_everywhere`. A guarda de NaN de `run` limpa as instâncias a cada bloco, então a unidade fica muda até o parâmetro mudar.
Correção: `if !value.is_finite() { return; }` antes do clamp.

### BAIXO

**B5. `src/schema.rs:156`:** `keys.len() - 1` causa underflow (pânico) para um seletor sem valores. É dado do próprio Guitarix, não do usuário.
**B6. `src/schema.rs:176`:** `panic!` para unidade desconhecida. Erro de programação; `plan()` poderia devolver `Result`.

---

## lm-plugins-ampandir

### CRÍTICO

**C7. `src/amp.rs:96-98, 107-110`: taxa do `.nam` sem limite.**
`model_frames = (MAX_FRAMES * model_rate / rate).ceil() as usize + 64` usa `model.expected_rate()`, que vem do ficheiro. Com taxa de 10^15, `model_frames` fica astronômico. Ele é usado em `model.reset(model_rate, model_frames)` (linha 98) e em `vec![0.0; model_frames]` (linhas 107-110). Alocação gigante: abort do processo.
Correção: validar `model_rate` (finito, por exemplo entre 8 kHz e 768 kHz) antes de calcular `model_frames`; devolver erro `plugin.ampandir.nam_rate`.
As alocações dentro de `lm-sys-nam` não foram auditadas (ver "Fora da área").

### MÉDIO

**M11. `src/amp.rs:96`: `loudness` do modelo sem verificação.**
`model.loudness().map_or(1.0, |l| 10^((TARGET - l)/20))`: um `l` NaN ou infinito gera ganho NaN ou infinito, aplicado a todas as amostras no `process`. O guard de NaN do renderer limpa a saída, mas o efeito é silêncio até trocar o modelo.
Correção: `if !g.is_finite() { 1.0 }`, ou erro.

### BAIXO

**B7. `src/amp.rs:172-177`:** `let _ = r.up.reset();` e `let _ = r.down.reset();` ignoram erros. Se o reset falhar, o estado do conversor não é limpo.
**B8. `src/lib.rs:76-100` (`impulse_sides`):** assume que todos os canais têm o mesmo número de frames (`channel[..frames]`). `SampleData` garante isso hoje, mas a suposição não está escrita.

---

## lm-plugins-stk

### MÉDIO

**M12. `src/stk/banded_wg.rs:314-316` (`pluck`) e `:399` (`tick`): divisão por zero que vira ciclo longo ou NaN.**
`min_len = self.delay[self.modes - 1].reference` pode ser 0: quando `set_and_clear` falha (`:141`), `reference` não muda. Então `x / 0.0 as i32` satura em `i32::MAX`, e o `for _ in 0..n` (linha 318) roda até ~2·10^9 vezes no `note_on`, no audio thread. Em `tick`, `input /= self.modes` com `modes == 0` (possível quando `set_frequency` corta os modos a taxas baixas) dá NaN na saída.
Correção: `if min_len <= 0.0 { return; }` em `pluck`; e tratar `modes == 0` em `tick` (silêncio).

### BAIXO

**B9. `src/stk/filters.rs:260, 380` (`SineWave::tick` e `FileLoop::tick`):** os ciclos `while self.time < 0.0 { += TABLE }` levam tempo proporcional ao valor de `time`. Com `rate` ou `phase_offset` enormes, o ciclo é longo. `rem_euclid` resolve.

### Verificado e sem achado

- `src/lib.rs` (mallets): o build verifica `sample_rate == 0`; `prepare()` pré-aquece `kernel()`, `sine_table()` e as três ondas (`rawwaves.rs:76-80`), então nenhuma tabela `LazyLock` é construída no audio thread.
- `src/mallets.rs`: `Note::clone_from` não aloca (só copia o trecho do `ring` que pode ser lido); `SCALERS[p]` e `TRIMS[...]` com índices limitados.
- `src/stk/tube_bell.rs`, `src/stk/modal_bar.rs`: `rem_euclid(9)` em `set_preset`; índices de `ring` com `% RING`; `k[(p+1)*TAPS..(p+2)*TAPS]` dentro de `(PHASES+1)*TAPS`.
- `src/stk/banded_wg.rs` `clone_from`: quando os comprimentos batem (o caso normal), copia sem alocar; `DelayL::clone_from` idem.

---

## lm-smf

### BAIXO

**B10. `src/lib.rs:272`:** byte de status `0xF1..=0xFE` (sistema em tempo real, como `0xF8`) faz o ficheiro inteiro falhar com `smf.unknown_event`, em vez de ignorar o byte. Não é pânico, mas um SMF válido pode ser recusado.
**B11. `src/lib.rs:282-301` (`put_vlq`, `put_meta`):** `put_vlq` tem buffer de 4 bytes e assume `v < 2^28`. Os chamadores limitam os deltas (`min(0x0fff_ffff)`), mas o comprimento de meta-evento (`body.len() as u32`) não é limitado. Só é alcançável com meta-evento acima de 256 MB.

### Verificado e sem achado

- Leitor: `bytes()` com `checked_add` (linha 117); `read_smf` trata `len < 6`; `ntracks` limitado pelo laço `r.at < data.len()`, e cada chunk consome ≥8 bytes (sem ciclo infinito com chunk de tamanho zero); running status tratado; `0x51`/`0x58` com `len` verificado antes de indexar; RIFF/RMID com `checked`/`saturating` nos avanços; sem recursão; `#![forbid(unsafe_code)]`.

---

## lm-osc

### BAIXO

**B12. `src/lib.rs:181-185` (`write_str`):** texto com byte NUL é codificado sem erro, gerando pacote que o próprio decodificador recusa (`osc.trailing`). Correção: recusar NUL na codificação.

### Verificado e sem achado

- Decodificador: `Reader::take` com `checked_add` (linha 223); `decode_at` recusa tamanho que não é múltiplo de 4 antes de tudo; `MAX_DEPTH = 8` aplicado antes de cada bundle (linha 267), sem recursão além disso; `MAX_ARGS = 256` aplicado antes de `Vec::with_capacity` (linhas 296-299); tamanhos de blob com `usize::try_from` e `take` verificado; `string()` checa NUL, padding e UTF-8; cada item de bundle consome pelo menos 4 bytes (sem ciclo infinito); `#![forbid(unsafe_code)]`; testes cobrem entradas malformadas.

---

## lm-midimap

### MÉDIO

**M13. `src/transform.rs:353, 360` com `src/doc.rs:267-268` (e demais `real_or(...) as f32`):** `min`, `max` e `step` do documento são convertidos sem verificar finitude. Um projeto com valor 1e39 vira `inf` em `f32`; `forward` devolve NaN (`inf - inf`), e o `clamp` deixa passar o NaN, que chega ao parâmetro do alvo.
Correção: em `Mapping::from_value`, recusar valores não finitos com `midimap.bad_mapping`.

### BAIXO

**B13. `src/doc.rs:145`:** `channel.min(15)` troca o canal 16 ou maior por 15 sem aviso. Um mapeamento de canal inválido passa a valer para o canal 15.

### Verificado e sem achado

- `table.rs`: `feed` sem alocação (só `Readings` com `MAX_READINGS = 4`); índice `channels[port*16 + channel]` protegido por `port < ports`; `inherit` é fusão O(n+m) sem alocação; `partition_point` sobre tabelas ordenadas.
- `message.rs`: `Raw::parse` rejeita bytes de dados ≥ 0x80, então `leds[key]` (`mcu.rs`) e `Cc14 msb + 32` (`feedback.rs`) não estouram.
- `learn.rs`: `seen` limitado por `KEEP = 128`; `seen[0]` seguro, pois o primeiro item é empurrado logo após `start`.
- `mcu.rs::receive`: `bytes[7..len-1]` com `len ≥ 8`; `pos < 2*LCD_WIDTH` antes de indexar; `meters[strip]` com `strip < 8`; `faders[channel]` com `channel ≤ 8`.
- `feedback.rs`: `Limiter` tem estado de tamanho fixo.

---

## lm-sampler: demais arquivos lidos

- `src/import/pat.rs`: `key_of` com clamp; `clamp(0, frames)` em `a`/`b` é seguro para `frames ≥ 0`. Ver "Fora da área" sobre `parse_patch`.
- `src/import/avl.rs`, `src/import/drums.rs`, `src/import/audio.rs`: sem achado de pânico; `k + 1` em `assign_outputs` só é avaliado quando `k < keys.1 ≤ 127`.
- `src/env.rs`, `src/filter.rs`: sem alocação e sem pânico; `fall` e `set` protegidos contra divisão por zero.
- `src/pcm.rs`: `read_wav` com limites implícitos (`data.len() / frame`, `chunks_exact`); `at(i)` verifica limites.
- `src/engine.rs` (demais partes): pool pré-alocado; `held` com capacidade fixa; `window_bounds` com `clamp` de limites ordenados; `allocate` e `legato_move` sem alocação.

---

## o que está bom

- **Leitores de formato (SMF, OSC):** toda leitura passa por um leitor com limites verificados por `checked_add`; limites de profundidade e de argumentos são aplicados antes de alocar; todo laço avança pelo menos um byte por iteração; `#![forbid(unsafe_code)]` nos dois crates.
- **GIG:** `u8_at`/`u16_at`/`u32_at` devolvem 0 fora dos limites, então o parser não indexa diretamente; `children()` usa `saturating_add` e sempre avança.
- **SF2:** registros lidos com `chunks_exact` de tamanho fixo, e os campos de cada registro estão dentro do tamanho (`phdr` 38, `shdr` 46).
- **Motor e voz:** pool pré-alocado, `render` sem alocação, `Arc` usado só como contagem no audio thread, `same_sound`/`take_over` por ponteiro, `Pcm::at` com limites, e o ghost do sampler é guardado no processador e libertado na thread principal (comentário no código).
- **Padrão `clone_from` sem alocação e pré-aquecimento de tabelas:** sid, freeboy, opulenz e stk copiam voz no lugar; stk pré-aquece as tabelas `LazyLock` no build (`rawwaves::prepare`).
- **Guitarix:** guarda de NaN com limpeza das instâncias; FIFOs e conversores dimensionados no build.
- **midimap:** `feed` sem alocação; crescimento limitado (`KEEP`, `MAX_READINGS`, `Limiter` de tamanho fixo).
- **Convolver (ampandir):** buffers fixos na struct; `process` não aloca.

---

## Fora da área (não auditado; registrar para outras áreas)

- `lm_dsp::patman::parse_patch` (parser binário do `.pat`, usado por `src/import/pat.rs`): não auditado. O importador só repassa os dados; limites de offset, contagem e alocação estão no parser de `lm-dsp`. Este é o ponto mais importante a verificar para o PAT.
- `lm_dsp::drumsynth::render` (texto `.ds` não confiável, em `drums.rs` e `source.rs::kit_pieces`): não auditado.
- `lm-sys-nam` (`Model::load`, `reset`): as alocações de `reset` com `model_frames` (C7) dependem dele.
- `lm-sys-guitarix`, `lm-sys-resid`, `lm-sys-samplerate`, `lm-sys-sndfile`: FFI, fora da área.
- `lm-render/src/renderer.rs:634`: `adopt` roda na thread de render, e o processador substituído é largado ali. Os processadores `ampandir` (`fading`), `guitarix` (`groups`/`fading`) e `sampler` (`ghost`) guardam estado antigo em `old`. Se o drop liberar `Vec`/`Box` no audio thread, a regra é quebrada. Não verificado.

## Observações sobre a metodologia

- Estes achados vêm de leitura. Nenhum foi reproduzido com teste ou build. Os de maior prioridade (C1, C2, C3, C4, C6, C7) merecem teste de regressão com ficheiro malformado ou caso limite.
- Em `dev` o perfil tem overflow checks ligado, então os overflows de A1 e A3 viram pânico em debug; em `release` (overflow checks desligado) viram wrap silencioso. Ambos os casos são errados.
- Nos plugins, `catch_unwind` converte pânico de build em erro e pânico de `process` em nó morto (silêncio). Isso contém o crash, mas não corrige o caso: o instrumento fica mudo.

## Resultado da confirmação

Confirmação por leitura do código; nada foi executado (só `cargo check`). Commit 1: `1d27b88c1`; commit 2: o seguinte, "Sampler chips: ..." (ver o relatório final).

| Achado | Veredito | Commit |
|---|---|---|
| C1 SF2 índices de bag fora de ordem | confirmado e corrigido: `validate_indices` no `parse` recusa índices decrescentes | 1d27b88c1 |
| C2 SF2 amostra de tamanho zero | confirmado e corrigido: `decode` recusa `end <= start` e SF3 vazio | 1d27b88c1 |
| C3 SFZ WAV vazio | confirmado e corrigido: `sampler.sfz.empty_sample` ao carregar | 1d27b88c1 |
| C4 GIG explosão de combinações | confirmado e corrigido: `zones` limitado a `1 << bits` na leitura (total de bits já ≤ 8) | 1d27b88c1 |
| C5 alocação pelo cabeçalho em `decode_file` | confirmado e corrigido: leitura em blocos com teto de 2^28 amostras | 1d27b88c1 |
| C6 OpulenZ tabela de taxas | confirmado e corrigido: tabelas com 76 entradas (60 + 15 + 1) | 1d27b88c1 |
| C7 ampandir taxa do `.nam` | confirmado e corrigido (por outro agente, commit 257375caa, antes do meu): `checked_model_rate` | 257375caa |
| A1 interp posição enorme | confirmado e corrigido na raiz: `read` limita a posição a 1e15 e trata não finito; cents limitados a ±12000 no passo da voz | segundo commit |
| A2 SFZ `#include` sem orçamento | confirmado e corrigido: máximo de 64 inclusões no total | 1d27b88c1 |
| A3 SFZ overflow inteiro | confirmado e corrigido: `parse_key` com `checked_*`, `saturating_add` em `end` e `loop_end` | 1d27b88c1 |
| A4 GIG zona de velocidade | confirmado e corrigido: `.min(zones - 1)` | 1d27b88c1 |
| M2 SFZ ordem do `#define` | confirmado e corrigido: `BTreeMap` | 1d27b88c1 |
| M3 `sample_frames` quadrático | confirmado e corrigido: `HashSet` | segundo commit |
| M8 freeboy taxa 0 | confirmado e corrigido: `from_state` recusa taxa fora de 8 kHz a 768 kHz | segundo commit |
| M9 freeboy ciclo no audio thread | confirmado e corrigido: silêncio se o quadro não produz amostras | segundo commit |
| M10 guitarix parâmetro NaN | confirmado e corrigido: não finito é ignorado antes do clamp | segundo commit |
| M11 ampandir loudness | confirmado e corrigido: loudness não finito vira ganho 1 | segundo commit |
| M12 stk banded_wg | confirmado e corrigido: guarda `min_len <= 0` e `modes.max(1)` | segundo commit |
| M13 midimap valores não finitos | confirmado e corrigido: `Mapping::from_value` recusa min, max, curve, step não finitos | segundo commit |
| M1 saída depende do bloco do host | corrigido na rodada de desenho: grade de atualização no quadro absoluto do motor, valores por voz guardados entre bordas; teste `the_output_does_not_depend_on_the_host_block_size` | 8eb7d8537 |
| M4 SF2 chunk truncado | falso na prática: a amostra além do chunk já dá `a sample past the data`; o resto é descartado sem efeito sonoro | n/a |
| M5 GIG busca linear | não tratado: custo só em arquivos grandes, sem risco de falha | n/a |
| M6 arquivo temporário Ogg | corrigido na rodada de desenho: arquivo exclusivo, 0600, nome imprevisível, desvinculado na hora (`Reader::from_file`) | rodada de desenho (lm-plugins-sampler, lm-sys-sndfile) |
| M7 bank/program fora de intervalo | não tratado: o esquema do plugin já limita o campo | n/a |
| SID | sem achado | n/a |
