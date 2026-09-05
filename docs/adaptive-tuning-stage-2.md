# Adaptive tuning stage 2: engineering contracts

Stage 2 resolves the engineering specification before production plugin wiring in [#617](https://github.com/yan-h/harmonigraph/issues/617) and sequencing in [#616](https://github.com/yan-h/harmonigraph/issues/616).
It does not implement aggregation, adaptive output, the [#621](https://github.com/yan-h/harmonigraph/issues/621) musical policy or the [#632](https://github.com/yan-h/harmonigraph/issues/632) Stop fix.
The exact committed base is `83d1191bd26c2f65ed2e18c6d56431aa895dda19`, the reviewed [stage 1 draft PR #634](https://github.com/yan-h/harmonigraph/pull/634) on `codex/631-clap-discovery`.
That base includes the accepted production decisions from merged [PR #633](https://github.com/yan-h/harmonigraph/pull/633).

The branch is `codex/adaptive-tuning-engineering-contracts` in the Codex-managed worktree `/Users/yan/.codex/worktrees/4bda/harmonigraph`.
The earlier proposed branch name, `codex/adaptive-tuning-contracts`, already belongs to merged PR #625 and remains untouched at `efd5168f016b0061a400c3016afa476a9773ffb2` locally and remotely.
The coordinator selected the unique stage 2 name to avoid replacing that history.
The stacked draft PR targets `codex/631-clap-discovery`, not main.
Its description and final handoff carry the exact pushed stage head, avoiding a self-referential commit hash in this file.

## Decisions and file inventory

[`adaptive-tuning-contracts.md`](adaptive-tuning-contracts.md) is the single detailed specification for:

- bounded SPSC transfer windows with retained source backlog, explicit admission and per-enclosing-callback work budgets;
- saved pairing versus runtime identity, source/incarnation scope and hub direct-input reservation;
- owned mailbox publication, active callback-boundary lease adoption, acquire/release ordering, off-thread lease reclamation and bounded registry retirement;
- continuous input/output coverage, immutable per-pass membership, configuration revisions and canonical lifecycle ordering;
- wrapper-level bounded output admission, per-attempt actual acceptance, partial-onset fault containment and independent release completion;
- source emission gates and generation/cut acknowledgements that preserve racing accepted output;
- prospective suffix revocation, retained unsounded requests, translation waves and responsive established-voice events;
- channel drain/neutral gates between different translations, Stop edge/cut propagation, new stopped input, loop handling and Off's pre-transition obligations;
- complete held baseline, separate retained-output acknowledgement/canonical history publication, and a separately complete chunked pending-request manifest;
- a fixed retained configuration timeline ahead of the input frontier, with explicit marker retirement/exhaustion;
- conservative byte calculations and production fixtures/measurements still owed.

[`adaptive-tuning.md`](adaptive-tuning.md) links the engineering decisions and updates the implementation order without claiming production behavior.
[`tuning-probe-bitwig.md`](tuning-probe-bitwig.md) links the subsequently accepted Stop/late behavior and the analytical D512 risk while preserving the historical measurements and build revisions.
This handoff records scope, evidence and the review boundary.
No Rust, framework, probe, persisted shape, plugin descriptor or binary changes in this stage.

## Inspected evidence and rejected approaches

Read the descriptions and discussion of #617, #616, #621 and #632, accepted PR #633, stage 1's actual handoff, current product design, complete Bitwig measurements and retained probe/framework code at the exact base.
Those four issues and the inspected PRs had no additional comment discussion at inspection time.
The coordinator supplied read-only protocol and independent Max preparation;
this implementer spawned no helper, ran no Claude review and remained the sole writer.

The current nice-plug `send_event` returns no admission/acceptance result and appends to an initially 512-capacity `VecDeque`.
The wrapper later pops and `try_push`es each event, and `ProcessTrace::Output` observes acceptance after that call.
The probe has already changed its local held table and popped pending data by then.
Therefore an observation hook alone cannot implement truthful production state, suppress a rejected onset's later tuning, or guarantee independent emergency delivery.
The production emitter must own that submission/completion boundary.
No framework change was made here, and reject-all probe testing was not presented as partial-acceptance coverage.

CLAP exposes one-event acceptance and no reservation for a note-on/tuning pair.
An accepted onset followed by rejected tuning is irrevocable partial output, possibly audible before termination.
The specification rejects rollback claims, an allowed unretuned fallback, delayed tuning repair and claiming silence after a rejected release.
It records accepted prefixes, closes new attack admission, and retains emergency release debt through acceptance or established host termination.
A rejected onset without an accepted prefix remains pending for legal retry unless explicitly canceled.

A generation check before queueing cannot exclude a concurrent hub revocation at actual output.
The selected emission gate has one hub controller, one bounded source claim before real host calls, a BUSY-preserving close, durable outcomes before BUSY clears, and acknowledged output/disposition cuts before exact-generation reopen.
Already accepted output stays truth, even for a closing epoch or a future event timestamp.
Fence acknowledgement and release completion are distinct.
Plain/volatile seqlock payloads, unbounded CAS retries, final `Arc` destruction in audio, and scanning the complete plan/history pool each callback are rejected.

The whole-stream extra shift in the probe preserved one late note's duration but postponed an older sounding voice's release on Stop in trial 15. It is not the production scheduler.
The contract preserves established lifetimes' translations and separately schedules unsounded waves, with explicit shared-channel dependencies and accepted cancellation boundaries.
It does not discard a pending adaptive request merely because the user selected Off later.

The D512 counterexample is analytical:
raw B sample 511 with offset +64 maps to 575, beyond A/C's progress 512;
if the next B drain precedes the hub, the earliest reply-driven output is raw 1024, after the raw deadline 1023. Offsets 448 through 511 can be 64 through one samples late under that order.
Trial 24 observed the extra-callback shape at buffer 1,024 and D2,048, not the smaller candidate.
No new standalone spike or live host trial ran, and neither D nor the accepted buffer target changed.
The exact boundary and production-host fixtures remain in #616's specification.

## Validation and measurements

The selected pool layout was calculated with Python integer arithmetic:
127,800,832 bytes, approximately 121.88 MiB before the 16 MiB overhead allowance, within the selected 144 MiB per-session ceiling.
The review fixes add a 4,096-record canonical publication ring and a separate 128-marker retained configuration timeline;
the fixed registry attachment slots fit the existing overhead allowance.
These are **calculated budgets**, not compiled `size_of` values, allocation measurements, callback benchmarks or evidence of supported event throughput.
Production must assert and measure the actual types, allocated arenas, high-water marks and complete callback cost, including maximum #621 policy work.

Focused documentation validation consists of Rust formatting, Markdown semantic-break checks, local Markdown target/anchor resolution, pool-table arithmetic and diff whitespace checks.
Executed `cargo fmt --all` and `cargo fmt --all --check`, with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`, successfully.
The repository-wide semantic-break check and explicit checks of both new files passed.
All 26 local Markdown targets/anchors in the four changed documents resolved, all 17 byte-budget rows recomputed exactly, and `git diff --check` passed.
The formatter's unrelated indentation-only rewrite of `docs/settings.md` was restored to the exact base before validation and is absent from this stage.
The draft PR records the executed commands/results and the exact-head Actions snapshot.
Full CI runs in Actions;
no local Cargo build/test workload or new musical regression fixture is needed for this documentation-only stage.
No release build is required, no new binary tag is claimed, and the shared Bitwig slot was not touched.

## Independent review cycle 1

The coordinator's Max and Xhigh reviewers inspected committed head `c0d372c8e13d24e954947dc64bd8aee405384019` and returned four findings.
All four were confirmed against the written contract and corrected;
none was waived.
These are specification fixes and production fixtures still owed, not executed new musical tests:

- **Future shared-channel gestures:** the earlier gate considered only queued controller conflicts, then allowed one later CC at the earliest wave's time. A zero-shift and +1,000-shift held voice could therefore receive a pedal-up before the younger translated release. Every different-shift wave now waits for old channel lifetimes/obligations to drain and pedals to become neutral, even when no controller was queued at admission. Older releases remain responsive. New Off notes retain zero correction but can wait under the same failure schedule; Off does not restore D prematurely.
- **Retained history across a baseline:** an empty held baseline previously authorized journal retirement, erasing an undrained completed on/tuning/off lifetime. Baseline acknowledgement now replaces current state only. A separate output-retention acknowledgement transfers full records to the hub's audio owner for exactly-once ordered canonical publication. Source retirement, musical recovery and credit release never wait for GUI/background/disk acknowledgement. Actual downstream publication failure has its own gap/incomplete-take diagnostic; a baseline cannot manufacture such loss.
- **Active discovery and pairing:** endpoints were previously handed only to inactive instances, leaving tuner-before-hub activation without an adoption path. Fixed instance-owned offer/return slots now transfer preallocated leases at enclosing callback boundaries, with healthy audio progress, coverage and baseline admission separate from registry acknowledgement. Active re-pairing finishes old obligations and detaches both audio owners before new adoption or off-thread reclamation.
- **Unbound configuration timeline:** copying configurations into bound requests did not cap markers accumulating beyond a blocked frontier. The hub now retains at most 128 timeline markers separately from the command queue and restore slots, charges each until finalized/cohort binding completes, and reports actual required-storage exhaustion without dropping intermediate automation revisions.

Each correction names a production fixture that reaches the reported shape:
future CC after attempted late-wave admission, a complete undrained lifetime plus empty baseline, an already-active tuner before its hub, and 129 required markers against the actual 128-marker timeline.
The revised head is committed/pushed to the same [draft PR #635](https://github.com/yan-h/harmonigraph/pull/635) for bounded cycle 2 review;
its exact hash and CI snapshot are recorded in that PR and the coordinator handoff.

## Independent review and remaining production work

The stage ends at a committed, pushed, open **draft PR, not merged**, ready for independent Max review at its exact head.
The original implementer validates and fixes confirmed findings.
Nothing here closes #617/#616/#621/#632 or claims the musical feature is complete.

The next implementing stages owe the production emitter and source-aware aggregation, actual layout assertions and real-time guards, policy/rescheduling/lifecycle fixtures sized to the selected limits, and normal D512 validation including the calibrated boundary counterexample.
Missing/rejected host output and stopped callbacks must remain honest failure states through recovery.
Any changed delay, capacity or host restriction must be documented against executed evidence.
When plugin-affecting code does change, commit first, release-build both `harmonigraph-plugin` and `harmonigraph-offline`, read the loadable build tag and leave the shared slot for Yan to select.
