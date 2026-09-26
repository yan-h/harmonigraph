# Performance and technical-debt audit — 2026-09-26

**Assessment: keep the architecture and make focused changes.** The largest verified opportunity removes redundant work from the audio callback without adding a cache or changing the learning boundary.
Dense lattice animation deserves a separate optimization experiment.
The performance fixtures need repair before their labels can be used as evidence for further optimization.
This audit changes no product code.

Audited revision: `b26fa162`.
Star shader optimization was excluded because another session owns it.
Three independent read-only reviews covered audio/core/analysis,
UI/editor,
and recording/export/tooling;
the coordinating pass covered scene/render ownership and checked the consequential findings.
This is a current-source audit,
not a merge-interaction audit or a claim that every path is defect-free.

## Recommended order

| Order | Work | Benefit and cost |
| --- | --- | --- |
| 1 | Build one complete confirmed snapshot at callback start; remove per-delta Hub publication | Removes repeated audio-thread work and incremental-capacity hazards; small boundary refactor with turnover/learning verification. |
| 2 | Preserve incomplete-source status at the learning boundary | Prevents learning from an accidentally truncated chord; small state-boundary correction. |
| 3 | Repair the busy/large-lattice performance fixtures | Makes subsequent performance decisions trustworthy; small diagnostic-only change. |
| 4 | Reduce repeated full-lattice animation sweeps | Material gain is plausible in wide/deep views and event bursts; medium complexity, requiring exact behavior comparisons. |
| 5 | Revisit existing dense spectrogram refold issue [#886](https://github.com/yan-h/harmonigraph/issues/886) | Already measured interaction hitch; folding and logarithmic quantization dominate, so another copy/cache optimization is unlikely to solve it. |
| 6 | Tighten CLI export output and numeric validation | Two bounded correctness improvements; lower priority for normal plugin use. |

No feature removal or broader architecture rewrite is recommended.
Implementation remains separate from this discovery report.

## 1. Redundant full-state publication in the audio callback

**Confirmed performance defect; high confidence.** [Hub::flush](../crates/harmonigraph-plugin/src/tuning/hub.rs) calls `confirm` at line 1001 for every published delta.
[State::publish_confirmed](../crates/harmonigraph-plugin/src/tuning/state.rs) rebuilds a source-sized array;
[ConfirmedPitches::replace_source](../crates/harmonigraph-core/src/confirmed.rs) validates,
clears,
and reinserts the source using repeated scans of the 256-entry session store.
Every delta republishes the source's already-final state.
This runs with Learn and Retune disabled too.

The full-state publication in `Hub::begin` already refreshes every source before the only production consumer,
`Owner::group_end` at [configuration.rs:423](../crates/harmonigraph-plugin/src/configuration.rs).
The [CLAP wrapper](../vendor/nice-plug/src/wrapper/clap/wrapper.rs) orders begin/refresh before configuration/learning,
then input delivery and later processing/flush (lines 2259–2276).
No consumer reads the repeated flush publications before the next begin refresh.
However,
those intermediate mutations still affect capacity admission:
`replace_source` validates against other sources before clearing them.
If 256 old voices are confirmed and a later-index source releases 64 while an earlier-index source gains 64,
refreshing the earlier row first sees the stale later rows and rejects a final snapshot that actually fits.
Release-first flush publications can avoid that rejection today.
Deleting them alone is therefore not behavior-equivalent.
The turnover witness asserts exact voice identities and confirms the failure with a retained store,
then verifies both a fresh snapshot and the current release-first flush sequence retain all 256 expected voices.

The optimized current-source probe measures the confirmation kernel only,
including the 17-source begin refresh and the indicated per-delta repetitions:

| Held voices | Deltas per callback | CPU time in preserved recheck |
| --- | ---: | ---: |
| None | 0 | 5.6 µs |
| One source, 8 | 0 | 7.9 µs |
| One source, 10 | 32 | 102.1 µs |
| One source, 16 | 32 | 159.2 µs |
| Four sources, 16 each | 64 | 373.4 µs |
| Four sources, 64 each | 0 | 145.4 µs |
| Four sources, 64 each | 256 | 8.07 ms |
| Four sources, 64 each | 2048 | 64.1 ms |

The 2048 case is the batch ceiling,
not a claim about a typical musical passage.
The smaller cases establish useful savings without relying on that stress case.
These are process CPU measurements on macOS arm64,
not DAW callback latency or measured audio dropouts.
Repeated runs vary;
the reproduction directory preserves the run behind this table rather than implying a fixed performance guarantee.

**Recommendation:** make the begin refresh construct one complete snapshot from an empty confirmed store,
then remove per-delta flush publication.
Preserve explicit incomplete-source status as described below.
This gives the snapshot one publication boundary and avoids validating new rows against obsolete rows.
A dirty-bit system or replacement indexed store would add maintenance obligations unnecessarily.
Verify a full-capacity handoff from a later source to an earlier source,
as well as learning after notes,
expressions,
source departures and resets across callback boundaries.

## 2. Incomplete source state becomes confirmed silence

**Confirmed correctness defect; recovery-path severity, high confidence.** [Hub::confirm](../crates/harmonigraph-plugin/src/tuning/hub.rs) passes `row.state.complete` as the `live` argument at line 1016.
When false,
`State::publish_confirmed` submits an empty source replacement.
`ConfirmedPitches::replace_source` then clears that source and its incomplete marker,
so the remaining sources can be treated as a complete chord.

The reproducer uses the actual core,
State and Event modules.
A 64-voice source plus a three-voice source starts with 67 confirmed voices.
A finite but out-of-range tuning expression takes State's existing rejection branch.
After the exact confirmation call used by Hub:

```text
state.complete=false
confirmed.complete=true
confirmed rows=3
LearningState::infer(..., true)=Ok(Some(LearnedTuning { ... }))
```

Capacity exhaustion sets the same incomplete flag;
its lost-release transport sequence was inspected but not reproduced through a full host callback.
The demonstrated case is a bounded state-boundary witness,
not evidence that ordinary valid playing commonly encounters it.

**Recommendation:** represent incomplete separately from absent or empty at this boundary,
using the confirmed store's existing invalidation mechanism.
Avoid another recovery protocol.
Verify that an incomplete source suppresses learning despite a learnable second source,
and that an authoritative complete replacement restores it.

## 3. Performance fixtures miss their stated workloads

**Confirmed diagnostic debt; high confidence.** Two independent mistakes in [tests/profile.rs](../crates/harmonigraph-ui/src/tests/profile.rs) make plausible-looking numbers misleading:

- The busy loop delivers On and a future-dated Off back-to-back before painting (lines 167–179).
  `NoteTracker` releases immediately when the Off is delivered;
  it does not schedule it for that future time.
  The busy notes are therefore already released,
  rather than held for the stated quarter second.
  Reused source/channel/key identities also replace the fixture's baseline held chord.
- The “819 nodes” and “3075 nodes” cases change naming extents (lines 245–254).
  `ViewConfig::scrolled` derives the drawn third/fifth bounds from camera and pane aspect,
  so both variants draw the same window.
  The scratch witness confirms equality of their complete `DrawnWindow` values;
  its absolute count is aspect-dependent and is not the count of an instrumented real dock.

The existing profiles run successfully;
passing does not validate these workloads.
In particular,
the busy/no-note-names timing difference does not establish the cost of genuinely held notes.

**Recommendation:** deliver releases when their frame arrives,
use identities that preserve the baseline chord,
and assert the intended held count.
Vary camera or pane geometry for larger drawn lattices,
and report the actual drawn count.
Keep these checks in the existing probes rather than creating another benchmark framework.
This complements the test-maintenance work in [#1081](https://github.com/yan-h/harmonigraph/issues/1081),
but concerns workload reach rather than cosmetic layout assertions.

## 4. Motion replay multiplies work by event times and visible nodes

**Measured scaling problem; high confidence in cost, proposed optimization not implemented.** [NodeMotion::step](../crates/harmonigraph-scene/src/motion.rs) advances all stored visible nodes for every distinct event time (lines 541–543),
then calls `gates` (line 572).
`gates` scans visible nodes against held pitches,
then calls `read_slots` for another node-by-held-pitch scan (lines 265–289 and 370–393).
Final current-state reconciliation performs another full pass.
The event-dependent work is approximately proportional to event times × visible nodes × held voices,
plus animation-array sweeps.

A release probe uses `ViewConfig::scrolled`,
a 16:9 pane,
valid camera distances of 12 or 24,
Cabinet scale 1,
and zero or four sevens sheets on either side.
It times `NodeMotion::step` only,
with a retained scene and ten held notes:

| Reachable drawn window | No new events | One short note per frame | Eight short notes per frame |
| --- | ---: | ---: | ---: |
| 345 nodes | 0.095 ms | 0.200 ms | 1.042 ms |
| 975 nodes | 0.245 ms | 0.493 ms | 2.593 ms |
| 12,555 nodes | 3.760 ms | 8.086 ms | 41.933 ms |

Values are median process CPU time from the first camera-derived run,
with ten warm-up frames and thirty measured frames.
Each synthetic short note supplies two distinct event times within a frame;
this is a scaling fixture,
not a claim that eight such notes every frame represent typical playing.
The nine-sheet case is a supported but dense view.
A repeat under different concurrent local load was slower even in process CPU time;
frequency and cache contention still affect these numbers.
No scene derivation,
GPU rendering or frame-present timing is included.

**Recommendation:** first fuse the duplicated gate/reading traversal and measure its benefit.
If the event multiplier remains material,
construct a per-step set of relevant nodes for intermediate replay,
retaining full initial/final reconciliation.
Include all starting,
intermediate and final held pitches,
as well as audio-ring and still-animating nodes;
an unchanged held pitch can become melody or bass when another note changes.
Prefer a per-step set to a persistent index with new invalidation rules.
A full output pass remains necessary,
so this cannot remove every O(nodes) cost.

This is a medium-risk optimization because late delivery,
reopened views,
same-time replacements,
retuning,
mark promotion and audio-only rings are real behavior.
Compare complete node outputs for those cases before accepting a faster path.
Do not solve the cost by dropping event-time animation or lowering the visible-node cap silently.

## 5. PNG output retains old trailing frames

**Confirmed CLI export defect; lower priority, high confidence.** [Sink::create and Sink::push](../crates/harmonigraph-offline/src/sink.rs) restart numbering at zero and overwrite only files written in the current run.
A three-frame run followed by a one-frame run to the same output stem leaves all three PNG files.
A downstream sequence import can therefore include frames from the older render.
The witness calls the actual sink and needs no GPU.

**Recommendation:** give a sequence explicit ownership,
for example a fresh run directory or refusal to reuse a populated sequence namespace.
Avoid broad glob deletion in an arbitrary user directory.
A successful shorter rerender should have an unambiguous frame set.

## 6. CLI numeric parsing silently changes invalid inputs

**Confirmed by source inspection; lower priority.** [main.rs](../crates/harmonigraph-offline/src/main.rs) parses CRF as floating point and casts it to `u32` (line 194).
Consequently `--crf -1` becomes 0 and fractional values truncate;
zero requests lossless x264 output,
which can be unexpectedly large.
Generic float parsing also accepts NaN and infinity for FPS,
scale and time arguments.
There is no central finite/range check before creating `Settings` and the sink.
`Settings::frame_count` casts the resulting product to `u64`.

**Recommendation:** parse CRF as an integer with a supported range,
validate finite positive FPS/scale and finite time options at the CLI boundary,
and reject invalid input with the option name.
This is ordinary input validation,
not a request to add compatibility coercions or a new configuration layer.
No huge export or disk-exhaustion experiment was run.

## Designs worth retaining and limits of this audit

The ordinary held-chord headless runtime profile measured median 0.654 ms for the active dock,
including 0.104 ms of analysis,
and 0.839 ms with Video preview.
Unlike the busy fixture above,
this scenario actually retains its chord.
The measurements exclude GPU callbacks and native-host presentation.
They support keeping ordinary UI churn lower priority,
not a guarantee of DAW frame rate.

The allocation probe counted 756 allocations / 816.5 KB in `root_ui` and 130 / 1728.5 KB in tessellation.
Those counts alone do not justify caches or pooling when the measured ordinary CPU path is already small.
The spectrogram reported zero full reaggregations over 200 steady frames.
No new confirmed stale-cache defect was found in the reviewed UI/render paths.

The following boundaries and mechanisms appear appropriate:

- Lock-free bounded audio ingress,
  reusable FFT plans/scratch,
  narrow spectrum-plan keys,
  and power-domain stereo combination.
- Separate current notes,
  pitch history,
  timed roll history and recorded/replayed facts,
  each serving a different consumer.
- Shared UI shell and workspace/picture separation;
  GPU pane ownership,
  retained pipelines and shared shadow scheduling.
- Streaming WAV decode,
  reusable frame buffers and bounded encoder queue.
  Per-frame GPU readback is serialized,
  but the older documented export profile was dominated by encoder handoff.
  Measure a current representative take before adding a deeper readback ring and its frame buffers.
- Existing Metal corpus,
  feature-isolated checks,
  vendor-patch tests and golden frames.
  They have current consumers;
  their size alone is not evidence that removing them reduces net maintenance cost.

Optional future investigation:
[policy selection](../crates/harmonigraph-core/src/policy.rs) scores candidates before eliminating keyboard-ineligible ones.
A cheaper eligibility pass may save scoring,
but the inspected synthetic ordinary cases were only tens of microseconds.
Prioritize the clearly redundant confirmation work before changing policy evaluation.

Known open issues were checked to avoid duplicates:
[#1157](https://github.com/yan-h/harmonigraph/issues/1157),
[#1153](https://github.com/yan-h/harmonigraph/issues/1153),
[#1151](https://github.com/yan-h/harmonigraph/issues/1151),
[#1154](https://github.com/yan-h/harmonigraph/issues/1154),
[#1150](https://github.com/yan-h/harmonigraph/issues/1150),
[#1039](https://github.com/yan-h/harmonigraph/issues/1039),
[#1081](https://github.com/yan-h/harmonigraph/issues/1081) and [#886](https://github.com/yan-h/harmonigraph/issues/886).
The GPU timing issue #1150 is an additional reason not to treat the overlay as an unquestioned optimization baseline.

A suspected publication-ring orphan on an invalid route was not elevated:
production fails the take on that outcome,
so the inspected sequence did not demonstrate continuing user-visible corruption.
No full DAW session,
new representative end-to-end video benchmark,
or complete CI run was performed for this read-only source audit.

[Reproduction sources, raw outputs and scope notes](evidence/performance-debt-audit/README.md) accompany this report.
