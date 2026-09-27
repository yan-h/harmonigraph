# Live production verification

These measurements use the actual runtime profile enum, grouped allocation and shader uniforms.
The temporary timing and image modules were removed before the production commit.
Apply production-plus-temporary-declarations.patch to the research snapshot (whose renderer is baseline fd7c8f9), then place live_parity.rs and live_timing.rs in crates/harmonigraph-render/src/spectrogram/tests/.
Build the release render tests and follow the proposal READMEs.
The runner has local absolute paths; adjust them and the compiled executable name for reproduction.

Timing uses a 960×540-point pane at 2× and 4× display scales, 60 warmup rounds and 240 measured rounds, with balanced six-case order and duplicate Half/Full controls.
The GPU interval starts with the source pass and ends with the dependent final composite; it is not whole-DAW frame time.
Recorded fixtures are private and omitted.

P2 and P3 match the accepted grouped reference pixels exactly in all captured inputs and sizes.
Half and Full have at most one 8-bit color-level difference from the frozen reference; the comparison JSON records the non-exact result.
No available pixel differs by more than one level and alpha is unchanged.
