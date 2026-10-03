# Native-host questions left unverified

These are conditional follow-ups, not a requirement to execute every scenario before using this audit.
Do them only when the owner has finished the active session and chooses a disposable Bitwig project.
This audit did not close/reopen a user window, swap the installed plugin, save user settings, play/stop transport or record a new real-host take.
Record the actual loaded overlay tag and revision before interpreting results.

## 1. Save freshness under real editor load

Use a dense passage with dock and Video preview visible.
Change one obvious appearance value and camera position, then save while the editor stays open.
Reopen a copy and compare the value, appearance and host-owned camera automation.
Repeat once during a sustained parameter drag.
Instrument UiState::map only in a dedicated experimental build if the saved value is stale: record lock acquisition/timeout and serialization durations separately.
The 20 ms threshold bounds only editor-lock waiting, not total save duration.
A stale fallback is a known permitted path; establish frequency before redesigning snapshot ownership.

## 2. Cross-instance epoch reset

Use one Hub and three Tune paths, first ordinary chords then an explicit dense controller stress clip.
Reset one Tune while Hub processing overlaps; record callback wall-time distributions, epoch mismatch counts and records visited/accepted, with the sample rate and block size.
Do not label a finite collector probe as proof of audible dropouts.
After a bounded-drain implementation, verify deferred matching records and existing row rotation, full-ring refusal, held notes and no new correction latency during normal play.

## 3. Held note through tuning edits

Hold adaptive Just E; change displayed axes/equivalence to linked 12-TET, then keyboard eligibility, then engine Off.
Confirm old acoustic correction and later player expression remain frozen/relative to onset while a new note uses current policy.
Confirm displayed node occupancy and name follow actual pitch under current lattice, not historic assigned coordinates.
The scratch Namer test proves lookup/fallback only; it is not this full host gesture sequence.

## 4. Record and export parity

Capture a small recognizable phrase with a mid-note channel bend and camera gesture.
Exercise ordinary Stop, backward loop, forward seek and a pause/resume; let each writer/export finish.
Read the actual take and check its WAV tail and warnings, then export at 24 and 60 fps.
Compare bend segments/camera events and the selected look, including a queued re-render after changing the live appearance.
Unit parity deliberately does not prove arbitrary-cadence pixel/envelope equality.

## 5. Reopen, resize and hidden idle

Hide/close the editor during active input and during recording, reopen after a short and >history-horizon gap, then repeatedly resize at native Retina scale.
Check bounded recovery, released marks, preserved color tails, no font corruption and take completion.
Measure native CPU/RSS and reopen/resize p95 with no local build, export or other audit running.
Do not terminate unrelated applications; record remaining competing load.

## 6. Representative long-history or target-allocation hitch

Only if a visible hitch is observed, save the exact look/take and dimensions first.
For #886 retain a genuinely dense, long history that crosses the rebuild rung; do not reuse a tiny/silent fixture as evidence of the full path.
For halo-only churn record a real drag's complete frame cost and texture-creation events.
The headless prepare probe measures a stage, not the frame a person sees.
