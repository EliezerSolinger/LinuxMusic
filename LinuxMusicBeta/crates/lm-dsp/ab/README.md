# The A/B of lm-dsp against the legacy program

The rule (interfaces.md, U34): every case nulls below -90 dBFS against the legacy code, or is listed in
`tests/ab.rs` (`INTENTIONAL`) with the reason the port differs on purpose.

- `cases.json`: the cases, the only list (the harness and the test both read it).
- `legacy-harness.cpp`: runs the cases on the legacy code (the strip and the master as the legacy mixer drives them).
- `build-references.sh`: builds the harness against the legacy build and writes `reference/<case>.f32`.
- `reference/`: what the legacy code gave, interleaved stereo `f32`, little-endian.

To write the references (needs the legacy build, `cmake -S legado -B build && cmake --build build`):

```sh
CONTRACT_OFF="TubeStage GainStage AnalogChannelStrip DeskFilters DeskDynamics DeskTolerance MonoBass MasterSection" \
    crates/lm-dsp/ab/build-references.sh
```

`CONTRACT_OFF` rebuilds those legacy files without fused multiply-adds: the legacy build lets clang fuse `a * b + c`
into one rounding on arm64, and Rust never does, so with them fused the tubes' light model (whose blocking
capacitor integrates) drifts apart by rounding alone. Without them, the cases null bit for bit or to rounding.

`AB_TRACE=<case> cargo test -p lm-dsp --test ab` prints where a case first parts from the legacy code.

The effect plugins (`plugin-effect` cases) are the legacy plugins themselves, loaded and run as the mixer runs them,
from a plugin directory the script rebuilds without fused multiply-adds (their discrete logic, an attack or a hold
that starts or not, turns one rounding into an audible difference). The harness advances the models' period counter
before each block, as the engine does: without it, a model's value buffer of the first period is kept, and its knob
reads as changed in every period (the Compressor's hold was reset each block).

The legacy oscillator's moog saw, exponential and users' tables come from FFTW planned with `FFTW_MEASURE`, which
may pick another algorithm on another run: regenerating the references can change those files in their last bits.

The harness also works out each knob's value buffer at the start of each period. Left to the effect, a knob that
changed read as changed in that period (its change flag) and again in the next (its value buffer, worked out then):
the Bitcrush reset its rate counters twice. In the new program a change arrives once, as an event.
