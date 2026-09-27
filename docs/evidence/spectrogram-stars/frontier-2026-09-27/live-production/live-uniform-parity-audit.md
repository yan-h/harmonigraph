# Production Uniform image parity audit

P2/P3 are byte-exact to their accepted frozen grouped references at1080p and4K, for both the take and flat fixtures.
Half/Full have a bounded difference: every changed channel is exactly +1 or −1 byte level, alpha is unchanged, and no RGB pixel exceeds1LSB.

| Size | Case | Input | Changed RGB channels | Changed pixels | Pixel fraction |
| --- | --- | --- | ---: | ---: | ---: |
| 1920×1080 | Half | take | 7,970 | 7,821 | 0.3772% |
| 1920×1080 | Half | flat | 11,764 | 11,563 | 0.5576% |
| 1920×1080 | Full | take | 7,689 | 7,549 | 0.3641% |
| 1920×1080 | Full | flat | 11,053 | 10,864 | 0.5239% |
| 3840×2160 | Half | take | 31,522 | 30,980 | 0.3735% |
| 3840×2160 | Half | flat | 45,860 | 45,076 | 0.5435% |
| 3840×2160 | Full | take | 30,515 | 30,028 | 0.3620% |
| 3840×2160 | Full | flat | 44,989 | 44,249 | 0.5335% |

Signed changes are nearly balanced in the bounded1080p CPU audit.
The following counts show negative/positive changes for R,G,B respectively:

| Case/input | R −/+ | G −/+ | B −/+ | Signed R,G,B sum |
| --- | ---: | ---: | ---: | --- |
| Half/take | 1809/1904 | 773/775 | 1353/1356 | +95,+2,+3 |
| Half/flat | 3146/3279 | 986/1004 | 1644/1705 | +133,+18,+61 |
| Full/take | 1866/1860 | 729/749 | 1228/1257 | −6,+20,+29 |
| Full/flat | 3021/3037 | 904/874 | 1638/1579 | +16,−30,−59 |

Differences occur throughout the image rather than concentrating at texture edges.
The frozen grouped uniform controls are byte-identical to frozen original-array Half/Full, so this is an old-to-new Uniform rendering difference, not reference-selection error.
P2/P3 exactness strongly excludes a shared fixture, input, clock or default-setting mismatch.

Source inspection found the same Uniform target dimensions, RGBA16Float format, depth ordering and normalized sampling coordinates.
The changes are runtime per-depth uniform reads and shader control flow, which can alter generated arithmetic.
The sparse, signed, one-level differences are consistent with compiler arithmetic/rounding drift, but this audit does not causally prove the compiler explanation.
The original-array-lookup regression passed; it preserves the new halo-writing shader and therefore does not independently establish old whole-pipeline byte identity.

Decision: retain the single production implementation and report this bounded change explicitly.
Do not claim byte-exact original controls or add another rendering path merely to reproduce rounding.
These results establish the captured fixtures only, not every possible setting or animation frame.

Evidence: `images-live-1080/comparison.json`, `images-live-4k/comparison.json`, and `live-parity-uniform-audit/{analyze.py,results.json}` under `/private/tmp/stars-investigation`.
Only existing files were analyzed; no GPU work, builds or repository edits were performed by this audit.
