# A1–A3: audio, analysis and musical state

Investigator: audio_tuning, gpt-6-astra high; independently challenged by render, gpt-6-astra high.
Pinned ad1c6e6b. No host failure claim.

## A1 — bound discarded work too

Hub::collect (`plugin/src/tuning/hub.rs:600–654`) limits accepted batch length to BATCH_EVENTS (2048), but wrong-epoch pops do not spend it.
Three full CAPTURE_RING (1024) rows can therefore consume 3072 records with an empty batch.
Across instances, Tune Reset can advance Session epoch after Hub adopted its own callback epoch (`tune.rs:219–228,309–316,387`; `session.rs:222`; `hub.rs:375–420`).
New-epoch records can be replenished while the old-epoch collector discards them.
Same-instance callback serialization does not prohibit that overlap.
#788 introduced this shape; #1417 fixed analogous storage-vs-work bounds in configuration drains, not this collector.

Candidate: finite per-row entry snapshot, preserving accepted cap, requires no persistent state and retains stale-cleanup throughput.
Alternative: global visited-pop count, spending on every successful pop, gives a tighter total callback bound but can defer valid records behind stale backlog.
Preserve rotation and queued tail; test mixed stale/valid rows across callbacks.
Do not add a scheduler, epoch handshake, second queue or persisted fairness state.

Confidence before experiment: verified code mechanism and legal overlap; actual duration/frequency unmeasured.
Independent challenge rejects any claim of demonstrated infinite production stall: Tune input has an 8192 pending capacity, and a refill hook imposes scheduling.
The finite-capacity no-thread probe is the primary reproducible evidence; hook interleaving is supplemental.
Verdict: pursue a small explicit work bound if the reproduction confirms it, not a new RT framework.

Other traced paths: audio ingress snapshots descriptor count (`audio_ingress.rs:111–124`); retained whole frames and gap semantics are bounded (`:81–106`).
Closed background ownership double-checks open state around try_lock (`background.rs:285–310`) and joins before shared teardown (`:327–347`).
Tune emission has a budget (`tune.rs:503–543`); reply replenishment requires captures from the same serialized Tune and does not establish this independent-refill case.
Dense-cohort policy cost remains accepted #790, not reopened by theoretical maximum counts.

## A2 — analyzer resource dependencies

#1407 / 90dbf18c already repaired duplicate construction.
FFT resources depend on size; tapers/normalization on size+count; bucket map on size+rate; all estimator changes reset readiness.
Source discontinuity clears retained audio/hop state while retaining estimator resources/history.
Code: `analysis/src/lib.rs:184–225,585–602`; `ui/src/spectrum.rs:274–283,350–410`; `plugin/src/editor/input.rs:79–103`.

Inspected fixtures reach actual boundaries: taper-only plan retention checks window-minus-one then complete output against a fresh analyzer; joint config changes channels/rate/window/tapers together and again on retained stereo; estimator-hop test starts at 9007 frames off-grid; source-discontinuity test includes overflow and enough replacement samples; opening race uses a barrier.
Verdict: retain precise invalidation and single ownership; reject another cache/planner layer.

## A3 — frozen acoustic pitch versus current display

Onset stores snapshot/correction/node (`tuning/state.rs:100–124`); keyboard/axes edits change subsequent policy, not old voice correction.
Engine change clears adaptive context at the next attack group (`hub.rs:553–560`) while held output remains; later expressions read stored correction (`tune.rs:550–575`).
Display intentionally consumes actual pitch, not historic attack_node (`core/notes.rs:704–733`), then matches current scene tuning (`scene/derive.rs:85`, `scene/motion.rs:330`).
Namer is recreated per frame and uses actual pitch/current tuning/current equivalences (`ui/panes/spectral/names.rs:621–644`, `spectral/mod.rs:441`).
Hold Just E then switch to linked 12-TET: sound remains about 13.686 cents below new E; at .5 cent tolerance that node should no longer light and naming can fall back to piano E.
Historical assignment is not authority over today's lattice geometry.

Verdict: retain these distinct owners. No stale-name cache found.
Combined held-note × keyboard × engine × temperament behavior lacks one inspected end-to-end fixture; existing tests cover parts.
Namer fallback test at `names.rs:2581` calls only equal_tempered_name, not failed Namer lookup; replace its fixture with actual lookup during a future test change.
Scratch direct-Namer probe requested to verify this clean claim.

## Current documentation contradiction

`docs/adaptive-tuning-plugin.md:86–88` and `docs/adaptive-tuning.md:460–463,590–591,768` say channel bends never reach display/take and cite removed control flow.
#1223 / 134821ce added per-voice Bend publication (`hub.rs:950–965`); #1272 / e69566b0 repaired fanout loss.
Keep #783 as dated historical context; consolidate current behavior description.
The channel-bend regression executes exported Hub callbacks and validates connected 60/60.5/61 segments through both lanes, but does not run file writer.
Overflow fixture arms and reaches 64 voices ×33 bends =2112 deltas, asserting 2048+gap+64-voice repair.
Do not call take-lane delivery on-disk proof, or direct-Hub fixture a paired-Tune fixture.
