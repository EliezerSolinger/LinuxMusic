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
