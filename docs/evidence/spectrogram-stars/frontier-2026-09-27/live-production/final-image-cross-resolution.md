# Frozen final-image differences

Mean absolute RGB difference / RGB mean squared error against the same-kind full reference. Values are byte-channel units and byte-channel². These are image differences, not perceptual quality or GPU performance.

| Profile | 1080p take | 1080p flat | 4K take | 4K flat |
| --- | ---: | ---: | ---: | ---: |
| Half | 1.3430 / 6.4233 | 1.6829 / 7.0431 | 0.5172 / 1.2748 | 0.6539 / 1.4470 |
| Uniform 75 | 0.7847 / 2.9766 | 0.9993 / 3.4191 | 0.2578 / 0.4610 | 0.3262 / 0.5358 |
| Uniform 87.5 | 0.6253 / 2.0994 | 0.7989 / 2.4426 | 0.1978 / 0.3134 | 0.2503 / 0.3673 |
| P2 | 0.3154 / 0.7471 | 0.4011 / 0.8830 | 0.1219 / 0.1828 | 0.1567 / 0.2215 |
| P3 | 0.1681 / 0.3422 | 0.2081 / 0.3986 | 0.0600 / 0.0809 | 0.0754 / 0.0977 |
| F | 0.0298 / 0.0821 | 0.0407 / 0.1047 | 0.0142 / 0.0236 | 0.0196 / 0.0311 |
| Native complete | — | — | 0.0105 / 0.0105 | 0.0131 / 0.0131 |

The 4K capture is one frozen phase (jitter 0.5, clock 3), with take and flat stills. It is not the earlier three-phase 1080p image model. The 1080p P2/P3/F/half captures use `images-grouped`, whose full reference is byte-identical to `curve-p0`. The 1080p uniform 75/87.5 captures use `images-reversed` and that directory's own full reference, which differs from p0. Every number compares candidate and reference from the same directory and kind.

The 4K `full-complete` result differs from full-a by at most one RGB byte value. There are 260,619 differing RGB channels in take and 327,044 in flat; alpha bytes are unchanged. The separate native-complete fallback at 1.25× was byte-exact in `images-complete-fallback/parity.json`.
