# Adaptive tuning evidence archive

The maintained account of what the feature does is [`docs/adaptive-tuning.md`](../../adaptive-tuning.md).
This directory holds measurements instead:
numbers that were expensive to produce, cheap to lose, and true of the tree they were taken on rather than of the tree today.

Everything here is **frozen**.
It is not maintained, it is not corrected in place, and nothing in it should be read as a description of current behaviour.
Where a number has since moved, the movement is recorded below rather than written over the original —
the drift is usually the more useful fact, because it says what a change actually bought.

## What is here

| File | What it measures | Measured on |
| --- | --- | --- |
| [`memory-ledger.md`](memory-ledger.md) | Every allocation the adaptive session made, cell by cell, against a 144 MiB ceiling, plus the projected plan for the storage that had not been built yet | The tree as of the aggregation, channel-wave and `CaptureArena` stages |
| [`replay-callback-cost.md`](replay-callback-cost.md) | Wall-clock cost of one replay block across 16 Sources and the Hub, guarded-optimized-dev and release, on a named machine | Commit `f5f7464f` |
| [`../../data/replay-callbacks-f5f7464f.csv`](../../data/replay-callbacks-f5f7464f.csv), [`.json`](../../data/replay-callbacks-f5f7464f.json) | The 728 per-owner/phase summary rows and the run metadata behind that report | Commit `f5f7464f` |
| [`../615/`](../615/README.md) | The Bitwig host-timing spike: callback order, event delivery, transport and offline cases, with its own manifest and checksums | The #615 probe apparatus, since deleted |

The two markdown files were moved here whole.
Only their cross-references were repointed and a freeze banner added;
no measurement in either was edited.

## What has changed since

[#712](https://github.com/yan-h/harmonigraph/issues/712) removed the mechanisms much of this was measuring —
the shared capture arena, the channel-wave controller replay, both recovery modules, the resumable cohort sort, the configuration timeline and the publication repair protocol.
So the ledger's *populations* are stale in two different ways at once, and it is worth separating them.

**Stale because the mechanism is gone.** The "constrained future allocation plan" reserves storage for things that were never built or have since been deleted:
the 1,024 × 1,024-bit cohort adjacency matrix and its traversal indices, the retained configuration timeline, the pending manifest windows, the future plan free-index array and the 4 MiB policy workspace.
The 150,890,091-byte plan and its 104,853 bytes of slack under the 144 MiB cap describe a design that no longer exists.
The cap itself was never a measured constraint;
it was a self-imposed ceiling.

**Stale because the number moved.** These are the measurements later stages took against the same fixtures, and they are the interesting half:

| Quantity | Ledger / audit | Later | Now |
| --- | ---: | ---: | ---: |
| Publication channels | 2,483,712 | 2,910,496 | 2,172,800 |
| `Hub::new` retained | 36,549,232 | | 6,796,160 |
| `Registry::register_hub` retained | 8,798,912 | | 517,440 |
| Unpaired Harmonigraph instance, total | 45,348,144 | | 7,313,600 |
| Publication `Item` | 176 | 224 | 200 |
| `SourceBaseline` frame | 15,304 | 15,816 | 13,360 |

The publication row is the one worth reading twice.
It was measured at 2,483,712 B and called the largest single item in the ledger;
by the time a stage went to shrink it, it had *grown* 17% to 2,910,496 B, because the ledger's own cell sizes were wrong in two places —
`Item` was 224 bytes rather than the recorded 176, and the baseline bank 537,744 rather than 520,336. It ends at 2,172,800 B, 25% below where it actually stood.
A ledger nobody re-measures drifts in the direction that flatters it.

The instance rows are one change rather than three:
the Hub used to allocate sixteen rows, sixteen ring triples and a whole plan ledger in its constructor, for every Harmonigraph whether a Tune was ever paired to it or not.
It now allocates a Tune's row, queues and plan slots at pairing, on the main thread.
A Harmonigraph with no Tune allocates nothing for tuning.

### The terminal-fault checkpoint deltas

The ledger's opening line used to point at a later checkpoint document for "the latest terminal-stage measurements".
That document was one of the twenty-two collapsed into the design document, and its ledger section is preserved here rather than lost with it:

> The final isolated guarded allocator fixture measured an ordinary session at 106,125,144 bytes and four sessions at 424,500,576 bytes.
> Source is 31,256 bytes, Row 31,128, Sequencer 5,808 (including its 3,936-byte Recovery), and SourceControl 31,248.
> The additional factual-voice backing is 10,240 bytes and its free-index allocation is 512 bytes for 256 `u16` entries.
> Against the independently checked inventory projection 150,640,891, the future delta is 5,416 bytes:
> 17 Source headers add 680, 16 Rows add 256, Sequencer adds 3,968, and the new factual free-index backing adds 512.
> The resulting specified future subtotal is 150,646,307 bytes, leaving 348,637 below 144 MiB;
> charging the later test-only Frozen ownership marker's 16 bytes conservatively gives 150,646,323 and 348,621 spare.

`Recovery` is gone, so the 3,936 bytes inside `Sequencer` are gone with it, and the projection those deltas were computed against is void for the reason above.
The measured session totals are still what they were.

## Why this is kept at all

Because the alternative is re-measuring.
Every number here cost a fixture, a machine in a known state and a run;
several of them were the argument that settled a design question, and one of them —
the publication drift —
is only legible *because* the old number was preserved instead of being quietly updated.

The same reasoning is why `CLAUDE.md` sends an unfinished investigation to an issue rather than a backlog line.
What a session measured is the expensive thing it produced.
