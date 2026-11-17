# Verification tools

Scripts that check a build against the one before a change. Every step of a refactoring (see
`docs/plano-refatoracao.md`) runs them before it is committed.

| Script | What it does |
|---|---|
| `ab-render.sh <build> <out>` | Renders the projects in `ab-projects/` (each built-in instrument and effect, and some LADSPA ones) as 32-bit float WAVs. |
| `ab-compare.py <a> <b>` | Compares two renders: largest and RMS difference per project. Fails if a project differs by more than -90 dBFS. Projects in `ab-random.txt` use random numbers and are only reported. |
| `ab.sh <build> <reference>` | Both of the above against a reference rendered from the commit the work started on. |
| `sanitize-all.sh <build> <out>` | Renders the A/B and the demo projects with an ASan/UBSan or TSan build (`-DWANT_DEBUG_ASAN=ON` / `-DWANT_DEBUG_TSAN=ON`) and reports any sanitizer finding, failure or hang. |
| `corrupt-projects.py <out> <projects>` | Writes damaged copies of projects, always the same ones. |
| `gui-smoke.sh <build> <out> [projects]` | Opens the GUI headless with each project (the A/B and demo projects by default), shows the Piano Roll and the Arrangement, saves a screenshot and quits. Reports crashes, hangs and sanitizer findings. |
| `corrupt-all.sh <build> <out>` | Renders damaged copies of the A/B projects: refusing them is fine, crashing or hanging is not. Best with a sanitizer build. |

Typical use:

```sh
tools/check/ab-render.sh build /tmp/reference        # once, on the commit before the change
# … change, build …
QT_QPA_PLATFORM=offscreen ctest --test-dir build/tests -j4
tools/check/ab.sh build /tmp/reference
tools/check/sanitize-all.sh build-asan /tmp/asan
tools/check/corrupt-all.sh build-asan /tmp/corrupt
```

Identical builds differ by up to about -97 dBFS on a few projects: the worker threads sum the mixer in whatever
order they finish, and feedback in some effects makes that rounding audible to the comparison.
