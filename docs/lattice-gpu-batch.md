# First lattice GPU batch

Parent:
`70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84` (PR #652).
This batch covers #642 A, #641, #644 A/B and #645 A.
The depth removal subsumes the intermediate discard-store slice.
Detailed timing, history ownership, view retirement and later GPU algorithms remain separate work.

## Implementation slices

- The docked owner's two beginning-of-pass timestamps now bracket all lattice preparation:
shadows, ink history and convolution, glow, ordered scene composition and optional bloom.
A small opening pass precedes every optional stage;
the dedicated tail pass and asynchronous readback cycle remain.
The final egui composite belongs to the host pass and is excluded.
Previews do not advance or consume the docked timer.
- `vs_ink_blur` collapses nonpositive-glow instances exactly as `vs_ink_strip` already does.
Both replacement-write passes still clear obsolete rows.
An active owner of row zero performs its own convolution;
glow-free audio rings no longer repeat it.
- The unread `Depth32Float` texture, scene attachment and pipeline depth declarations are removed.
Painter order remains responsible for occlusion, including interleaved markers and each label's shadow then ink.
- An optional `LatticeBloom` owns the independent nodes-only attachment and three bloom intermediates.
It is first allocated on a drawable frame with positive bloom strength, then retained through silence until bloom is disabled or the viewport is replaced.
Single- and dual-attachment `ScenePipelines` are built by the same factory at startup and hot reload.
Glyph ink still writes only the visible picture;
label shadows preserve their existing treatment of both images when bloom is on.
A transparent shared texel supplies the unused composite binding when bloom is off.

Bloom's retained key is its enabled state within an `Offscreen` of fixed scene and native sizes.
Changing a positive strength updates uniforms only.
Crossing zero replaces only bloom allocations and the final composite binding;
main colour, shadows, glow and ink history remain untouched.
Empty frames release disabled bloom but retain enabled targets for the next note.
Resize retains the existing same-capacity strip transfer.
Capacity growth retains the existing replacement/reseed behaviour, including loss of a release colour that has no current ink.
There is no saved-state or scene/instance API change.

## Allocation effect

These are texture-descriptor sizes, excluding driver overhead, compression and tiling.
They establish allocation removal, not physical traffic or an FPS prediction.

| Pane and scale | Removed depth | Bloom allocations avoided while off |
|---|---:|---:|
| 256 × 256 native, scale 1 | 262,144 bytes | 720,896 bytes |
| 3840 × 2160 native, scale 2 | 126.6 MiB | 276.9 MiB |

The bloom sum includes the scene-resolution nodes-only image and a native half-size image plus two quarter-size images.
At 4K/scale 2 those components are 253.1 and 23.7 MiB respectively.
Bloom-off scene passes also avoid clearing, writing and storing their second attachment, including the fullscreen glow draw.
Repeated positive strengths keep the same allocation;
each off/on cycle creates exactly the four bloom images required on re-entry.
The main picture and glow history do not churn with those cycles.

## Verification and measurements

The focused target-lifetime fixture checks actual GPU handles and byte-exact stable pixels through repeated bloom on/off/on during an inkless release, native resize and render scale changes.
It separately crosses capacity 1 → 2 to verify the existing reseed baseline, reuses row zero with a different colour, and exercises glow off/on.
The cull fixture compares five frames against the retained unculled blur shader with 25 shipped nodes, covering mixed audio/MIDI, release, glow ending while audio remains and row reuse.
The reload fixture rebuilds both attachment variants from real shader files and draws names, shadows and glow through both.
The timer fixture requires a positive returned measurement and verifies that a preview leaves its owner's readback state and published value unchanged.
Existing lattice and offline goldens are required without blessing.

The ignored complete-preparation probe uses beginning timestamps at both ends and reports CPU wall time around the complete callback as well as GPU time.
`PROBE_BLOOM=0` and `PROBE_BLOOM=1` select comparable runs without changing geometry.
Cold CPU figures include pipeline and target creation;
steady figures exclude ten warmup frames.
The initial local release renderer suite with hot reload passed 235 tests, with four timing probes intentionally ignored.
An uncaptured repeat of the built executable passed the same 235 tests in 6.75 seconds with no adapter or timestamp skips.
All 13 lattice goldens remained unchanged.
The initial cold compilation took 1 minute 6 seconds with two build jobs.

Exactly one authorized Claude Opus/xhigh review ran read-only against the pinned base.
It found one retained-key defect:
empty frames released enabled bloom targets, forcing reallocation at the next note.
The fix retains enabled targets through silence, releases disabled bloom even while empty, and defers first allocation until geometry exists.
The existing lifetime fixture gained a silence-to-note identity assertion;
its affected GPU recheck passed in 0.22 seconds with no skips.
Crossing bloom strength zero still intentionally creates or drops bloom resources.
No second review ran and no other findings remain.

Remote Full CI and Security audit passed at `7fbca792`, including workspace clippy/tests, both renderer feature paths, all five offline goldens, the plugin check, vendored tests and documentation checks.
The separate local release offline golden run passed all five images in 0.52 seconds after a 1 minute 23 second compilation, with two timing probes intentionally ignored and no adapter skips.
No lattice or offline baseline was blessed or edited.

## Paired performance comparison

Measured on an Apple M1 Pro on 2026-09-05, using the release renderer with hot reload enabled.
The baseline was an archive of the pinned parent with only the candidate's identical timing probe copied in;
the candidate executable was built at `7fbca792`.
No production source differed from those revisions.
Two rounds ran baseline then candidate, each with bloom strength 0 then 1, strictly sequentially under the shared machine lease.
Each case used 768 × 768 pixels, ten warmup frames and 120 measured frames.
The named view ships 355 lit nodes and 30 synthetic names;
the audio fixture ships 225 rings with zero or one positive-glow owner of row zero.
All 16 probe processes completed with working timestamps and no adapter skips.

Each table cell lists the GPU median in milliseconds for round 1 / round 2.
Both sides measure the complete preparation encoder, excluding the final egui composite.

| Fixture | Bloom | Baseline GPU | Candidate GPU |
|---|---:|---:|---:|
| Gaussian, live width | 0 | 0.656 / 0.990 | 1.075 / 0.952 |
| Gaussian, maximum width | 0 | 0.965 / 0.482 | 0.414 / 1.105 |
| Distance, live width | 0 | 0.628 / 0.412 | 0.425 / 0.533 |
| Distance, maximum width | 0 | 0.772 / 0.557 | 0.652 / 0.744 |
| Audio rings, no MIDI glow | 0 | 0.771 / 0.637 | 0.618 / 0.606 |
| Audio rings, one MIDI glow | 0 | 0.620 / 1.198 | 0.786 / 1.325 |
| Gaussian, live width | 1 | 1.471 / 1.262 | 1.234 / 1.514 |
| Gaussian, maximum width | 1 | 1.461 / 0.941 | 1.560 / 0.904 |
| Distance, live width | 1 | 0.984 / 0.664 | 0.788 / 0.806 |
| Distance, maximum width | 1 | 0.921 / 1.009 | 1.046 / 1.001 |
| Audio rings, no MIDI glow | 1 | 1.632 / 0.694 | 0.660 / 1.352 |
| Audio rings, one MIDI glow | 1 | 2.329 / 0.817 | 0.851 / 0.830 |

Every matched baseline/candidate p10–p90 interval overlaps.
Across these runs p10 ranges from 0.159 to 0.282 ms and p90 from 1.257 to 4.200 ms.
Even the audio-only bloom-off case, whose median improves in both rounds, has broadly overlapping distributions:
baseline p10–p90 is 0.197–4.006 / 0.183–3.978 ms and candidate is 0.206–4.028 / 0.197–3.999 ms.
These results do not establish a reliable general GPU or frame-rate improvement, nor isolate a stage saving from any one slice of this batch.

| Steady preparation CPU medians, range across cases and rounds | Baseline | Candidate |
|---|---:|---:|
| Named views | 0.218–0.245 ms | 0.215–0.244 ms |
| Audio-ring views | 0.133–0.154 ms | 0.131–0.157 ms |

The largest absolute CPU median difference in a matched pair is 0.012 ms.
This wall clock surrounds the complete `prepare` callback, including packing, staging and encoding;
it excludes `from_scene`, label collection, submission and the host composite.
It is deliberately broader than the overlay's write scope.

The median first preparation across 24 fresh-resource cases per side is 38.093 ms baseline and 49.026 ms candidate.
The candidate eagerly creates both attachment variants, a startup cost accepted in this slice to keep feature switches and hot reload simple.
This aggregate includes pipeline and target creation and does not isolate their individual contributions.
Driver/cache outliers also remain visible:
the first baseline case took 2965.825 ms and one candidate audio case took 366.661 ms.
All other baseline first preparations were 33.619–48.593 ms and all other candidate first preparations were 46.061–59.165 ms.
These are first-callback observations in the measured session, not controlled cold-driver benchmarks.

Reproduce the two workloads with the ignored tests `a_frame_of_names_at_each_kernel_costs_this_much` and `a_frame_of_audio_rings_costs_this_much`, using `PROBE_BLOOM=0` or `1` and `PROBE_FRAMES=120`.
Use the identical probe on both revisions, pass `--exact --ignored --nocapture --test-threads=1`, and keep build jobs and test threads at two when sharing this machine.
The baseline compilation took 1 minute;
the comparison completed within its five-minute compile-and-run window.

## Remaining measurement boundary

The result supports the allocation and redundant-work removals above, but does not select a residual GPU bottleneck or justify a conditional algorithm.
There is no measured stage-specific optimization to dispatch from these totals.
Keep #642 B and #649–#651 gated until a concrete cost question calls for detailed attribution.
At that point distinguish shadow raster/blur, ink history/convolution, full-resolution glow, ordered scene and bloom;
count actual positive-glow rows rather than held voices.
For CPU attribution, split first resource creation from steady packing, staging and encoding before pricing another abstraction or allocation change.
History ownership and hidden-view retirement in #645 B/C remain separately scoped and are not implemented here.

The handoff records the final commit's CI result and both fresh release artifacts, with the tag read from the plugin binary.
