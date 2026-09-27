# Configurable jitter cost

Measured on M1 Pro / Metal on 2026-09-27.
The baseline uses the fixed half-jitter shader from `e8426e183bb5ff41259a6be4785900e4b12b684b`.
Its Cloud struct gets an unused `star_geometry` row so the old constants read the same uploaded uniform layout as the candidate.
The candidate uses the configurable shader at its default 50% jitter.
Both keep all five layers,
four neighbors,
color memory and the automatic full-resolution 2+3 split at 4K.

| Run | Fixed mean | Configurable mean | GPU time change | A/A difference |
| --- | ---: | ---: | ---: | ---: |
| 1080p A | 5.534 ms | 5.544 ms | +0.19% | +0.23% |
| 1080p B | 5.538 ms | 5.538 ms | -0.01% | +0.15% |
| 4K A | 21.591 ms | 21.118 ms | -2.19% | -1.38% |
| 4K B | 17.053 ms | 16.922 ms | -0.76% | +0.46% |

No clear additional GPU cost is measured for configurability.
The 1080p differences fit inside the identical-control variation.
The candidate is slightly faster in the 4K runs,
but absolute times change substantially between those runs and A/A variation is appreciable,
so this is not evidence of a reliable speedup.
These are synthetic offscreen GPU intervals,
not live Bitwig frame times.

The [existing paired harness](../four-neighbor-current/README.md) is reused:
two identical baseline cases and one candidate,
independent resources and color history,
all six execution orders,
60 warmup rounds and 240 measured rounds per case.
The baseline column averages the two baseline means;
positive A/A means baseline B took less time than A.
Raw samples,
logs and unrounded results are adjacent.
The `reduction_percent` field in `analysis.json` is the negative of the table's time-change column.

## Reproduce

Use the revision carrying this report in an owner-managed worktree.
The measured candidate shader blob is `d3a80d639e4d127c24fb5724e4b9f2f3a8535c3c`.
Keep Jitter at its 0.5 default.
Prepare the baseline before applying the scratch harness:

```sh
python3 - <<'PYCODE'
from pathlib import Path
import subprocess
shader = subprocess.check_output([
    "git", "show",
    "e8426e183bb5ff41259a6be4785900e4b12b684b:crates/harmonigraph-render/src/shaders/spectrogram.wgsl",
], text=True)
needle = "    star_life: f32,\n"
assert shader.count(needle) == 1
shader = shader.replace(needle, needle + "    star_geometry: vec4<f32>,\n")
Path("/private/tmp/stars-four-baseline.wgsl").write_text(shader)
PYCODE
git apply docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
cargo test --release -p harmonigraph-render --lib --no-run
python3 docs/evidence/spectrogram-stars/four-neighbor-current/run.py --binary /path/to/printed/test-executable --output /private/tmp/stars-jitter-repeat
git apply -R docs/evidence/spectrogram-stars/four-neighbor-current/harness.patch
```

Remove the scratch harness before building a plugin.
