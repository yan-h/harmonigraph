# T4: live and offline temporal cadence

Investigator: state_scene, gpt-6-sol high; source and fixture inspection at ad1c6e6b.
Status: time and ownership paths deeply examined; no host run, build, or new pixel experiment.

## Contract and ordering

`offline/render.rs:273–295` computes time from the frame index, replays all records at or before that time, feeds the audio interval ending at the frame, then calls `begin_frame`.
`offline/replay.rs:94–126` retains factual note timestamps while delivering configuration, notes and params by each stream's time order.
`ui/lib.rs:376–396` and `ui/runtime.rs:93–106` observe current camera/configuration/parameter mirrors before pruning voices.
This preserves musical event, roll and current-tuning timing; it does not establish image equality across display schedules.

Live input anchors note and audio source timestamps onto the GUI clock through one `ClockMapper` (`plugin/editor/input.rs:40–98`).
The same drain order serves open and closed schedulers (`:112–125`).
The fast-bounce fixture at `plugin/editor/input.rs:210–275` compares 1×, 8× and 32× note onsets and analyzer columns, with enough audio to produce over 400 FFT hops.
It proves timeline and sample-grid parity, not live-versus-export pixel or envelope parity.
Replay tests at `offline/replay.rs:295–337,341–480,483–535,553–570` cover visibility/prune cadence, canonical roll history, resolved configuration at several cadences, and a note's factual timestamp.
`offline/replay/recorder_parity.rs:1–3` expressly excludes audio, envelope, pixel and arbitrary replay-cadence equivalence.

## Cropped starts and audio history

On frame zero, `frame.saturating_sub(1)` makes `from == now`, so `prepare_frame` feeds an empty audio slice even when `--start` is later than the recording's beginning (`offline/render.rs:262–293`).
The code documents this choice: feeding future columns on the opening frame could advance live retention beyond the picture's far edge at low frame rates.
The test at `offline/render.rs:427–507` reaches starts at song zero and late offsets and frame rates 1, 4, 30, 60, 120 and 30000/1001.
It asserts zero first-frame columns, no future columns, and eventual analyzer-column timestamps and bytes independent of frame batching, including the half-window lag.
It therefore confirms a cold visual start, not that a cropped export is a byte-identical slice of one rendered from song zero.

`offline/main.rs:238–247` and `docs/offline-rendering.md:267,270–288` define `--start` as an absolute song position that skips to a passage.
Default start uses the earliest captured event/audio time because earlier take data does not exist.
The docs use “trim” and “skip” but do not explicitly say that the analyzer and carried visual history begin cold at an explicit crop.
That wording is a small ambiguity, not a demonstrated product defect.

## Frame-sampled visual state

The tracker and node animation use event times rather than snapping note edges to frame times.
`scene/motion.rs:461–630` seeds at a bounded history horizon and replays factual on, bend and off edges, including edges delivered late or present before a cropped first frame.
The coarse-versus-detailed short-stab fixture at `scene/motion.rs:943–972` reaches a reversal and checks the same pose and opacity at a common time.
It does not cover every parameter automation schedule or all rendered pixels.

The ring's measured target is sampled when the lattice draws (`ui/panes/spectral_fold.rs:376–408,471–508`).
`scene/spectral.rs:626–641` advances its gate envelope by elapsed time, and the first step settles immediately.
Thus a transient between two low-rate frames cannot have the same carried ring history as one observed at a higher frame rate.
That is normal for a sampled visual filter, not evidence that musical event times changed.

Spectral color memory uses `dt = now - previous.now` and exponential pickup/release coefficients (`render/spectrogram/atmosphere.rs:1632–1656`).
`render/spectrogram/color_memory_tests.rs:559–640` reaches 1 fps and 120 fps and compares the same result for a constant changed input over one second; it also checks paused redraw, palette edit, backward seek and disabled memory.
The constant-input fixture proves elapsed-time integration, not equivalence for an input that rises and falls between low-rate frames.
`offline/render.rs:818–955` verifies repeatable pixels for a fixed export cadence and non-vacuous carried light or drifting atmosphere; it does not compare different cadences.

## Verdict and narrow follow-up

No new correctness defect or replay redesign is justified by this inspection.
The supported guarantees are accurate event/configuration timing, deterministic pixels for a fixed render schedule, and analyzer sample-grid placement after the cold first frame.
Pixel identity between different frame rates, or between a cropped export and a full export cut afterward, is not a supported invariant.

If a real export shows an objectionable opening, compare one take with a held note, steady audio followed by an abrupt release, and nonzero ring and color memory at a shared output size.
Render 60 fps once from song zero and once with `--start 1.0`; compare matching frames at 1.0, 1.1 and 1.5 seconds while asserting the same roll and tuning.
Separately compare 30 fps and 60 fps at matching timestamps to quantify frame-sampled effects.
Inspect the visible area and recovery time before deciding whether a documentation clarification or any warmup cost is warranted.
