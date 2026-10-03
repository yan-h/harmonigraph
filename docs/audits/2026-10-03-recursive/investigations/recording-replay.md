# T1–T2: recording, replay and export ownership

Investigators: coordinator plus independent state_scene (gpt-6-sol high).
Status: stated ordering/ownership questions deeply examined; host scheduling and pixel parity across arbitrary cadence remain unverified.

## T1 — stop, seek, looping and finalization

The callback observes arm and bar before transport (`plugin/lib.rs:780–821`).
Lifecycle keeps accepted audio end distinct from host position, defers stopped-rewind splits until resume, and ends a one-file trigger on forward discontinuity (`record/recorder/lifecycle.rs:91–199`).
Captured intent admits the rest of a callback despite concurrent Stop (`record/recorder.rs:411–425`).
Source closure is an audio-publication frontier, distinct from configuration closure (`:390–403`; `plugin/configuration/recording.rs:274–314`).
Pump drains record and publication lanes, flushes, then accounts failure/ready Stop/disconnect (`writer.rs:245–379`).
Readiness requires producer, configuration and source epoch closures and no retained passes (`:768–774`).

These states protect different late-data owners, so collapsing them would erase evidence required for file finalization.
History paid for this: #895 production/test pump divergence, #712 incomplete markers across passes, #1397 late-source ordering, #1405 fixture export joins.
Pump is now shared by real worker and FileWriter, avoiding parallel test machinery.

Fixture reach: `writer/canonical_tests.rs:70–161` routes delayed history to a retained pass and checks both WAV tails; `:217–271` fills all 128 passes; `:1311–1454` exercises real-file forward/paused seek, ordinary pause and audio-only last pass.
`writer/audio_tests.rs:182–219,252–315` covers worker Stop/rollover and both one-file Stop orders.

Verdict: reject a recorder protocol rewrite or merged closure flags; no supported defect found in these questions.

## T2 — replay and selected appearance

Take parsing stably sorts canonical events and independently sorts params/config (`take/lib.rs:363–442`).
Replay applies through frame time (`offline/replay.rs:97–126`); recorded pitches make current configuration independent of event assignment.
Selected appearance is parsed/normalized once before output setup (`offline/render.rs:85–100`), installed before frames (`:180–209`), and audio feeds before begin_frame (`:262–295`).
Export queue captures appearance at enqueue (`record/recorder/render_job.rs:228–269`), processes FIFO (`:272–335`), and publishes via exclusive hard link under cancellation lock (`:451–486`).

Parity fixture `offline/replay/recorder_parity.rs:178–327` uses real publication consumer and writer pump for canonical gaps, repair and closure waits.
Camera cadence fixture `:333–373` writes a take directly, so does not alone prove recorder capture; `record/recorder/writer/tests.rs:1149` separately covers initial/changed/reset parameter persistence.
The parity module explicitly excludes host/Hub, audio, envelope and arbitrary pixel cadence equivalence; do not widen its claim.

Verdict: keep shared runtime/replay model and bounded export queue.
Documentation corrections: `docs/offline-rendering.md:46` says camera edits are not recorded despite the current camera parameter path and its own later section; `:66` says malformed appearance falls back, but renderer returns an error.
