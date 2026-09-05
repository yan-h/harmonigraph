# Bounded trail scratch measurement

This is a partial experiment for [#648 A](https://github.com/yan-h/harmonigraph/issues/648), based on `70e9f48dddb3c14e5cb2d2173d5604f8a4c34e84`.
It covers only `TrailField`'s remembered-pitch temporary.
General scene and callback buffer recycling and #648 B indexing/caching are outside this result.

## Candidate and overlap

Replace the per-frame `Vec<PitchClass>` with `[PitchClass; NoteHistory::MAX_VISITS]` and a populated length.
The array is initialized safely, filled once in `history.visits()` order and scanned through its populated slice.
It retains no musical answers between frames and introduces no cache keys, indexing, unsafe code, dependency or caller change.
The current history bound is 384 and a pitch class occupies four bytes, so the array payload is 1,536 bytes.
The `NoteHistory::record` insertion path enforces the bound and the private history map has no bypassing writer.

The inspected tuning heads were #636 `f6f1bde`, #637 `ee33033`, #639 `ddeeb50` and #653 `152bf183`.
Their changes leave `trail.rs` and history storage, iteration and capacity unchanged.
The #653 worktree was clean at inspection.
Existing scene trail fixtures and event constructors overlap source identity, so they were left untouched.

## Method

Release Rust 1.92 on macOS 26.6.2, arm64. The coordinator serialized local Cargo operations and benchmarks with the tuning and GPU tasks.
Builds used this worktree's target directory and sccache, with `CARGO_BUILD_JOBS=2` and `RUST_TEST_THREADS=2`.

A one-off standalone probe used the original `trail.rs` from `git show 70e9f48d:crates/harmonigraph-scene/src/trail.rs` as a reference module and included the candidate production file by path.
Both used the real `NoteHistory`, `PitchClass`, `Tuning`, `ViewConfig` and `NodeInstance` types.
The probe re-exported the scene's `style`, `view` and `NodeInstance` so both unmodified modules resolved their crate paths.
Its manifest depended on this worktree's core and scene crates without changing any workspace manifest or fixture.

The history was populated via `NoteHistory::record` using a copied voice, in reverse pitch order, and its exact visit count was asserted.
Counts were 0, 1, 15 and 384, including completed history independently of held voices.
The ordinary window had extents (6, 10, 0), giving 273 nodes, all on the home sheet.
The large window had extents (28, 14, 4), giving 14,877 nodes, of which 1,653 were home nodes.
Nodes were derived once from an empty tracker, outside the measured scopes.

Two pitch fixtures exercised each window and visit count:

- Mixed: remembered pitches `60 + i * 0.03125` MIDI units, with actual node pitch classes under `Tuning::just()`.
- Miss: remembered pitches `60 + i * 0.01`, node classes `800 + (node_index % 31)` cents, and an assertion that no node class matches any visit, forcing every home-node scan to reach the end.

Timing used the normal allocator.
Independent non-inlined wrappers invoked each implementation's build alone or build plus apply/drop, with black-boxed inputs and outputs.
Eleven paired samples alternated reference/candidate order, with each batch calibrated to approximately 8 ms from the reference duration.
Tables report the median nanoseconds per invocation.
Node buffers were reused in the probe;
scene derivation, node-buffer allocation/reset, label construction, GPU work and host-frame scheduling are excluded.
These are isolated CPU scopes, not complete plugin-frame timings or predicted frame-rate gains.

A separate build enabled a global allocator that counted allocation/reallocation calls and requested bytes around one invocation, with fixture creation excluded.
Timing and allocation instrumentation were never combined.
Scratch sources and raw CSV results were written under `/private/tmp/lattice-trail-scratch-probe` and were not added to the repository.

## Output checks

Outside the timing loop, the reference and candidate were applied to copies of the same nodes with trail reset to zero.
Every trail float was compared by its bits and every `NodeInstance` field was compared through its complete debug representation.
Checks covered both windows, each history count, mixed and full-miss fixtures, `Past` → `Played` → `All` → `Past`, history clear and repopulation, repeated pitch classes in multiple octaves, and circular tolerances of 0, 0.5, 2, 50 and 600 cents.

A capacity probe also populated 1, 15 and 384 visits at `60.1 + i * 0.02` MIDI units.
A home node for every remembered pitch had to receive exactly 1.0, including the final visit;
an off-sheet match and an unvisited zero-cent node had to remain zero.
The latter distinguishes unused initialized array slots from remembered pitches.
Existing scene trail tests remain the committed behavioral coverage, while this probe verifies the storage change without adding another event-constructor caller during source-identity integration.

## Result and decision

Retain the bounded array as a small allocation-churn reduction.
It removes one allocation for every nonempty trail frame without changing caller ownership or traversing the history tree for each home node.
The build-only scope becomes slower, by about 11 ns at one visit, 24 ns at 15 visits and 63–94 ns at 384 visits.
Build plus apply shows no material ordinary-window regression:
the largest positive nonempty ordinary delta is 8 ns (0.16%), while the zero-history path differs by about 0.12 ns.
The small mixed timing differences do not establish a CPU speedup, and no complete host-frame performance claim is made.
The retained implementation is safe and confined to one production file;
the array's setup cost is accepted for the measured removal of the heap temporary.

Allocation results were identical across both windows, both pitch fixtures and build/build-plus-apply scopes:

| Remembered visits | Reference allocation calls / requested bytes | Candidate allocation calls / requested bytes |
|---:|---:|---:|
| 0 | 0 / 0 | 0 / 0 |
| 1 | 1 / 16 | 0 / 0 |
| 15 | 1 / 60 | 0 / 0 |
| 384 | 1 / 1,536 | 0 / 0 |

CPU medians, in nanoseconds:

| Window | Visits | Fixture | Build reference | Build candidate | Build + apply reference | Build + apply candidate | Build + apply change |
|---|---:|---|---:|---:|---:|---:|---:|
| ordinary | 0 | mixed | 1.60 | 1.60 | 1.70 | 1.82 | +7.14% |
| ordinary | 0 | miss | 1.59 | 1.61 | 1.70 | 1.82 | +6.97% |
| ordinary | 1 | mixed | 32.14 | 43.43 | 471.34 | 457.47 | -2.94% |
| ordinary | 1 | miss | 32.29 | 43.25 | 447.92 | 447.14 | -0.17% |
| ordinary | 15 | mixed | 61.68 | 85.71 | 5,057.99 | 5,066.15 | +0.16% |
| ordinary | 15 | miss | 62.36 | 85.54 | 5,008.74 | 5,012.73 | +0.08% |
| ordinary | 384 | mixed | 974.50 | 1,037.46 | 113,552.36 | 111,302.92 | -1.98% |
| ordinary | 384 | miss | 1,044.40 | 1,138.88 | 130,283.76 | 129,567.52 | -0.55% |
| large | 0 | mixed | 1.58 | 1.59 | 1.71 | 1.82 | +6.07% |
| large | 0 | miss | 1.58 | 1.58 | 1.72 | 1.83 | +6.73% |
| large | 1 | mixed | 32.11 | 43.29 | 13,684.34 | 13,376.22 | -2.25% |
| large | 1 | miss | 31.98 | 43.12 | 13,039.18 | 12,952.54 | -0.66% |
| large | 15 | mixed | 62.00 | 85.52 | 35,907.05 | 35,206.64 | -1.95% |
| large | 15 | miss | 62.32 | 85.29 | 35,020.64 | 34,798.61 | -0.63% |
| large | 384 | mixed | 976.17 | 1,042.96 | 641,783.30 | 642,835.45 | +0.16% |
| large | 384 | miss | 972.64 | 1,041.93 | 762,572.95 | 744,520.80 | -2.37% |

Both separately compiled probe runs passed every output comparison and capacity assertion.
The six existing scene trail tests also passed.

## Validation commands

The temporary probe commands were run sequentially from the worktree, with the target directory explicitly pointing into it:

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 CARGO_TARGET_DIR="$PWD/target" cargo run --release --offline --manifest-path /private/tmp/lattice-trail-scratch-probe/Cargo.toml
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 CARGO_TARGET_DIR="$PWD/target" cargo run --release --offline --manifest-path /private/tmp/lattice-trail-scratch-probe/Cargo.toml --features count-allocations
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test -p harmonigraph-scene trail -- --nocapture
```

Delivery also requires broader scene and lattice/offline golden validation, with outcomes recorded in the pull request:

```sh
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test --release -p harmonigraph-scene
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo test --release -p harmonigraph-render -p harmonigraph-offline golden
CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
```

The golden sets guard the broader picture;
the direct trail comparisons above are the fixtures that exercise remembered visits through the full bound.
No golden is to be changed or blessed for this storage change.

## Independent review

One requested Claude Opus/xhigh source-only review found no defect changing current output.
Its comment-clarity suggestion was applied:
the source now explicitly says the array removes the allocation while preserving the existing contiguous scan.
The suggested defensive `zip` fill was declined.
The reviewer identified no current way for history to exceed `MAX_VISITS`, and truncating the fill would silently discard visits if the producer's bound were ever broken.
`NoteHistory::record` already enforces the public bound, while the direct probe above exercised the full 384-visit path.
No second review was run.
