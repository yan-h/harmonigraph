# Low preset: L1 production verification

Low selects the accepted L1 dimensions from [the experiment](https://github.com/yan-h/harmonigraph/pull/1270):
far three at one third,
foreground composite at one half,
and foreground halos at 50% and 30%.
It uses the shared Stars renderer for both the spectrogram and lattice background.
High remains the default.
Older binaries cannot parse appearances saved with the new Low enum variant.

## Picture verification

Two adjacent frames at each of 1920×1080 and 3840×2160 replay the experiment's recording-derived field,
palette,
logical pane,
clock and default Stars settings with color memory disabled.
The production branch includes the shared Stars extraction that landed after the experiment.
99.9998% of RGBA channel values match the accepted L1 captures exactly;
only 168 of 82,944,000 differ,
each by one quantization level.
`parity.json` records each frame separately.
This is a bounded parity check,
not a claim of byte identity across all settings.

The new `spectrogram-starfield-low` export golden was inspected beside Medium and passes unblessed.
All seven existing offline golden tests and the renderer's existing lattice goldens pass unchanged.
The 374 passing renderer tests include Low's odd-size allocations,
partial-pane clipping at fractional scale,
color-history retention,
and lattice visibility and profile transitions.
The saved-state round-trip covers Low in both views and the offline appearance document.
All five UI settings-range checks pass.
The strict production Metal catalog passes:
only image dimensions change,
and the WGSL diff is a comment,
so the existing compiled corpus remains valid.

## Reproduce the parity check

Apply `capture.patch` on this production change in an owner-managed worktree.
It changes only the temporary test capture harness;
restore that file after the probe.
The private inputs and accepted raw L1 captures must still be at the paths named by the script and manifest.
NumPy and ffmpeg are required.

```sh
git apply /absolute/path/to/capture.patch
cargo test --release -p harmonigraph-render --lib --features shader-assets-tools --no-run --message-format=json > /private/tmp/low-parity-build.json
export RESEARCH_WORKTREE="$PWD"
python3 /absolute/path/to/check-parity.py
```

The script selects the built test executable from Cargo's JSON,
renders the matching frames,
and checks a maximum channel error of one.
Capture-run timings include readback and encoding and are not performance evidence.
The earlier experiment contains the performance comparison;
this check verifies the selected picture survived the production port.
