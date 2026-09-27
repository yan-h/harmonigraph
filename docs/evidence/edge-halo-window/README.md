# Off-pane glow owners

Issue [#1189](https://github.com/yan-h/harmonigraph/issues/1189) is reproduced at `b3dceb19` on Apple M1 Pro / Metal.
The initial reproduction and rejected broad-window candidate are retained below.
The final implementation adds only selected off-pane light owners.

At that historical baseline,
apply `probe.patch` and run:

```sh
HARMONIGRAPH_REQUIRE_GPU=1 cargo test --release -p harmonigraph-render scrolled_window_keeps_the_halos_of_off_pane_nodes -- --nocapture
```

The probe compares the live scrolled window against the same window padded by five lattice steps on each edge.
It uses a held five-note chord,
one sheet,
the current fresh view,
and a 512×512 camera at horizontal offsets 0 and 0.25.
It is a scratch reproduction rather than committed test coverage:
the assertion expects the eventual fix to match the padded reference.

| Camera offset | Current nodes | Padded reference nodes | Changed RGBA channels | Largest byte difference |
|---|---:|---:|---:|---:|
| 0 | 225 | 625 | 1,380 | 1 |
| 0.25 | 210 | 600 | 32,277 | 4 |

Applying `wide-margin.patch` as well sizes the window margin from the renderer's full halo radius.
The same comparisons become byte-identical,
but the production window grows to 361 and 342 nodes respectively:
60–63% more derived positions in this fixture.
This is a count of work,
not a measured frame-time slowdown.

The candidate also breaks the existing node-budget guarantee:
a fully zoomed-out cabinet pane at aspect 3,
nine sheets,
and shear 1 asks for 21,681 nodes against the 20,480 cap.
The existing cap then trims the enlarged window.
The narrow-window and zoom-extent tests also fail.

The broad-margin candidate was reverted.
Raising the cap would trade more CPU/GPU work for a small edge-light correction and would weaken an existing resource bound.
The selective implementation keeps the ordinary window unchanged and shares its node constructor and animation path.

## Selective owner design

The candidate rectangle bounds the glow billboard rather than the ordinary node.
For a tilted camera,
its maximum radius expands both the sheet-depth slab and its visible XY bounds.
The existing horizon clamp limits candidate enumeration to 20,480 positions.
Only positions outside the ordinary window are considered.
They must match current MIDI or a pitch in NodeMotion's bounded roll horizon,
or already carry MIDI motion or a GlowFade owner,
and their billboard must intersect the pane.
Audio rings are deliberately not a light source.
Existing maps are queried at candidate positions;
there is no extra persistent owner cache or event replay algorithm.

Extras use only the ordinary window's remaining node budget.
When candidates exceed that space,
nearer positions win;
a full ordinary window gets no extras.
This preserves the existing resource limit even for degenerate cameras,
at the cost of incomplete peripheral light when the limit is reached.
The published drawn and naming windows do not change.
The constructor merges ordinary positions and sorted extras in canonical lattice order.
That order matters:
appending extras changed equal-depth FP16 accumulation by one byte in the cabinet reference;
the merged order restores exactness without changing renderer sorting.

The lattice-map overlay also resolves its at-most-thirteen requested positions by binary search in that canonical scene order.
Its optimized lookup introduced by #1221 used rectangular-window indices directly,
which sparse insertion shifts.
The overlay retains ordinary-window membership and sorts the resulting scene indices,
so map annotations keep their positions and paint order without a new lookup cache.
A regression inserts halo owners before ordinary nodes and checks the actual map-outline and hover positions,
including exclusion of a valid map assignment outside the ordinary window.

## Image measurements

The scratch probe in `selective-probe.patch` exercises the actual UI composition path and renders its result through the production paint callback.
Apply it to this implementation and run:

```sh
HARMONIGRAPH_REQUIRE_GPU=1 cargo test -p harmonigraph-ui --lib scratch_gpu_selective_composition_matches_padded -- --nocapture
cargo test -p harmonigraph-ui --lib scratch_populated_candidate_cost -- --nocapture
```

The reference expands the ordinary window by ten steps on each edge.
A five-note held chord is followed through note-off and a glow release that outlives the MIDI ink.
All 24 comparisons are byte-identical:
three projections,
default and maximum reach,
and four held/release timestamps.
The initial ideal-selector probe also checked camera offsets 0 and 0.25.
At the default cabinet view it needed 33 and 31 extras respectively:
258 and 241 total nodes,
about 15% more than the ordinary window instead of the broad candidate's 60–63%.
Maximum-reach probes and orthographic/perspective views also matched their padded references.

An incoming pan does not have the same history as an arbitrarily padded scene.
With attack 0.4,
a settled held chord,
and camera X moving from 0 to 2 at time 1.05,
the first frame differs from the padded-history reference in 45,734 channels with maximum difference 6/255.
At time 1.45 that falls to 27,012 channels and 2/255.
The existing GlowFade policy makes newly owned lights attack from darkness;
it is intentionally preserved.
An outgoing light crossing the ordinary window edge keeps its existing incarnation and level,
so this change removes the previous outgoing cutoff rather than restarting that light.

## CPU measurements and maintained coverage

Apple M1 Pro,
optimized development test profile,
populated NodeMotion and GlowFade maps,
1,000 selector iterations and 100 full-composition iterations:

| Fixture | Ordinary nodes | Extras | Border candidates | Selector | Full composition |
|---|---:|---:|---:|---:|---:|
| Default cabinet, five held notes | 225 | 33 | 216 | 21.20 µs | 116.01 µs |
| Zoomed-out cabinet, aspect 3, nine sheets, shear 1 | 19,251 | 0 | 1,206 | 165.66 µs | 8,560.36 µs |

These are local CPU measurements,
not an end-to-end frame-time claim.
The dense fixture scans a capped candidate rectangle of 19,899 positions and finds no intersecting extra light.
Earlier full-map scans and a temporary owner HashSet cost about 892 µs there;
direct candidate lookups avoid that unnecessary work and allocation.

Maintained tests cover ordinary-node membership and relative order,
carried glow after MIDI ink expires,
a real audio-only ring followed by a pan,
zero spare node capacity,
an outgoing MIDI pan with nonzero attack,
and short between-frame pitch bends surviving only in bounded roll history.
The GPU and CPU measurement harness stays in this evidence patch rather than adding benchmark-sized fixtures to normal coverage.

## Actual GPU cost

After rebasing onto the one-pass renderer at `505529af`,
a separate timestamp probe compares the old 210-node window with the 241-node selective scene at camera X 0.25.
Both use the fresh view's bloom strength 1.0223677 and the same held chord.
The opening timestamp precedes callback preparation command buffers;
the closing timestamp ends the dependent pane-composite pass.
The bracket therefore includes the full lattice GPU workload rather than an empty closing pass.

Apply `gpu-cost.patch` and run:

```sh
HARMONIGRAPH_REQUIRE_GPU=1 cargo test -p harmonigraph-ui --lib scratch_selective_gpu_cost -- --nocapture
```

Two isolated runs alternate old and selective order on each frame,
discard 20 warm-up frames,
and retain 80 samples per variant and size.
Values below are GPU milliseconds:

| Size | Run | Old p10 / median / p90 | Selective p10 / median / p90 |
|---|---:|---:|---:|
| 512×512 | 1 | 1.3793 / 1.8078 / 2.4557 | 1.4710 / 1.7864 / 2.4841 |
| 512×512 | 2 | 1.3747 / 1.7766 / 2.2314 | 1.2935 / 1.7143 / 2.2451 |
| 1536×1536 | 1 | 5.2475 / 5.7236 / 6.0477 | 5.2655 / 5.6903 / 6.0622 |
| 1536×1536 | 2 | 5.2467 / 5.5630 / 5.9074 | 5.2972 / 5.5350 / 6.0050 |

No consistent regression is distinguishable from this measurement noise.
The slightly lower selective medians are not evidence of a speedup:
the lower and upper quantiles move in both directions,
and most of each added halo lies outside the pane.
These measurements support the bounded selective design but do not promise zero cost on other scenes or hardware.
