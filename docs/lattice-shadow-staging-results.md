# Bounded lattice shadow staging

This partial [#648 A](https://github.com/yan-h/harmonigraph/issues/648) experiment starts from immediate parent `codex/lattice-history-owner` at `33dad566bafb008692cf151c67e0d8e0edf647f7` ([#665](https://github.com/yan-h/harmonigraph/pull/665)), stacked on [#664](https://github.com/yan-h/harmonigraph/pull/664) and [#662](https://github.com/yan-h/harmonigraph/pull/662).
It measures only the two `Vec<ShadowBox>` temporaries used by lattice `prepare` for node-cell and glyph-cell uploads.

## Decision and boundaries

Retain direct `Queue::write_buffer_with` staging for both uploads.
It removes the measured temporary Rust allocation/reallocation calls without retaining another CPU buffer.
A shared reusable vector also removed warm temporary allocations, but retained 2 MiB after the large-glyph workload and had no demonstrated advantage warranting that owner.
Neither alternative eliminates wgpu's native staging allocation.

The private helper converts each `ShadowBox` into a 64-byte array and writes through wgpu 29's `WriteOnly::into_chunks` / `write_iter` API.
Byte-array alignment is one;
there is no typed mapped-memory cast or read of existing staging contents.
Every uploaded byte is initialized, and `write_iter` rejects both short and excess input.
An empty upload checks its iterator is empty and requests no nonzero staging view.

Both existing upload gates remain.
Node indices still map through `packed.boxes.get(index).copied().unwrap_or(NO_CELL)`.
Glyph cells still concatenate each `Draw::Label(a, b, cell)` box exactly `b - a` times in draw order;
they are not scattered into a retained array.
Packing, Scene and callback inputs/interfaces, UI SharedState/lifecycle, CPU glow rows, matching and caches are unchanged.
General callback recycling and all of #648 B remain pending;
the independent trail slice [#661](https://github.com/yan-h/harmonigraph/pull/661) is separate.

At implementation, the actual adaptive-tuning write set in `827b` / `7009` covered plugin configuration/performance, recording/publication, vendor wrapper/CLAP code and Cargo files.
It contained no renderer production or Scene/UI SharedState changes.
Inherited golden/mark fixture event-constructor boundaries were left untouched.

## Method and reproducibility

Release Rust 1.92, macOS 26.6.2 (25G83), Apple M1 Pro / Metal.
Every local Cargo operation had a separately relayed exclusive machine lease, used sccache and this worktree's own target directory, and returned the lease immediately after terminal exit and an approved process-clearance check.
The raw adapter identification was required;
no sandbox-only adapter skip counted as validation.

The scratch probe constructs real `LatticeCallback`s from synthetic named/shadow scenes through `from_scene`, then uses real `shadow::pack` output.
It asserts actual instance counts, emitted glyph counts and contiguous label ranges before measuring either upload path.
Large cases stress upload cardinality;
they are not estimates of typical visible node or voice counts.
Missing boxes deliberately exercise `NO_CELL` with a nonempty node upload.
The missing-atlas case models `has_atlas = false`, not merely a callback missing its snapshot while the renderer retains a font.

Normal-allocator timing and allocation counting are separately compiled probe variants.
CPU upload scope includes mapping, temporary vector creation/drop when applicable, and both sequential wgpu staging calls.
It excludes fixture/callback construction, shadow packing, submission, GPU execution and readback.
Four warmup invocations precede each timed batch;
submission and completion waits occur outside it.
Fresh/reuse/direct order rotates across paired samples.
The first run used 11 samples of 16 upload pairs;
the ordinary-case repeat used 33 samples of 64 pairs.
These are isolated upload timings, not complete `prepare`, host-frame or GPU timings.

The separate allocator wraps the Rust system allocator with thread-local counters enabled only around the measured invocation.
Calls include allocations and reallocations;
requested bytes sum every request, including replacement capacities, and are neither peak live memory nor transferred bytes.
The counters do not intercept native Metal allocation.
Cold/warm vector construction counts and actual sequential-upload Rust counts are recorded separately.
The exploratory fill-only timing keeps both fresh vectors alive until return, unlike production's sequential upload/drop;
it is not used to select the candidate.

Scratch sources, README and full logs remain in `/private/tmp/lattice-shadow-staging-probe`.
They are verification artifacts rather than maintained tests.
To reproduce a variant on the exact parent, copy its source to `crates/harmonigraph-render/src/lattice_tests/shadow_staging_probe.rs`, register that test module in `lattice_tests.rs`, then run the command below under a fresh machine lease.
The allocation variant compiles out the timing loop;
the two timing variants compile out the counting allocator.

| Variant | Source file in the artifact directory | SHA-256 | Full log | Actual wall time |
|---|---|---|---|---:|
| Initial timing | `timing-source-v2.rs` | `cf79051ef45941d5ee9a1d9b248b1a853201ec42fe2fea82681fe44d3bc10c2f` | `timing-v2.log` | 13.20 s |
| Allocation counting | `allocation-source-v2-formatted.rs` | `e847b56b68f667ae7d66c533f87045b5118dae5e70b42265d9aa360079a9aff4` | `allocations-v2.log` | 12.07 s |
| Ordinary timing repeat | `timing-repeat-source-formatted.rs` | `cba6212aafa4636f9fbbad34bdc775eeee77af2f15f9e957da1fd003351b7445` | `timing-repeat.log` | 13.35 s |

```sh
env -u HARMONIGRAPH_BLESS CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=2 \
  CARGO_TARGET_DIR="$PWD/target" \
  cargo test --release -p harmonigraph-render shadow_staging_probe -- --ignored --nocapture --test-threads=1
```

An earlier attempt failed during compilation because it assumed `QueueWriteBufferView` exposed `chunks_exact_mut`.
It exited 101 after 86.88 s;
`timing.log` records that failure, not a measurement.
The corrected source uses the actual write-only API above.

## Allocation results

All three successful probe operations passed exact GPU buffer comparisons, including alternating destination buffers, growth/shrink, empty/missing-atlas/missing-box cases, the full large uploads and an untouched sentinel cell beyond each uploaded prefix.
Upload bytes below describe verified payload lengths, not allocation savings inferred from source.

| Workload | Actual nodes / glyphs | Node / glyph upload bytes | Fresh temporary calls / requested bytes | Cold reuse calls / requested bytes | Retained reuse capacity |
|---|---:|---:|---:|---:|---:|
| Small named | 6 / 7 | 384 / 448 | 3 / 1,152 | 2 / 1,152 | 768 B |
| Ordinary named | 273 / 75 | 17,472 / 4,800 | 7 / 33,600 | 1 / 17,472 | 17,472 B |
| Large nodes | 14,877 / 150 | 952,128 / 9,600 | 8 / 984,640 | 1 / 952,128 | 952,128 B |
| Large glyphs | 4,096 / 20,478 | 262,144 / 1,310,592 | 15 / 4,456,192 | 4 / 3,932,160 | 2,097,152 B |
| Empty | 0 / 0 | 0 / 0 | 0 / 0 | 0 / 0 | 0 B |
| Missing atlas | 273 / 75 | 17,472 / 0 | 1 / 17,472 | 1 / 17,472 | 17,472 B |
| Missing boxes | 273 / 0 | 17,472 / 0 | 1 / 17,472 | 1 / 17,472 | 17,472 B |
| No glyphs | 273 / 0 | 17,472 / 0 | 1 / 17,472 | 1 / 17,472 | 17,472 B |

Warm reuse recorded zero temporary calls/bytes in every case.
Direct writes remove the same temporary calls without retaining a vector.
Actual sequential upload counters confirm the difference:
ordinary fresh/reuse/direct were 13/6/6 Rust calls, and large-glyph fresh/reuse/direct were 21/6/6. The two-write reuse/direct scope still recorded 6 Rust calls requesting 748 bytes;
node-only gates recorded 4 calls requesting 684 bytes, and empty frames zero.
These remaining Rust-side wgpu counts do not count or eliminate the native staging allocations documented for both queue APIs.

## CPU result and limits

Medians in microseconds per node/glyph upload pair:

| Workload | Initial fresh / reuse / direct | Repeat fresh / reuse / direct | Repeat paired direct-minus-fresh median |
|---|---:|---:|---:|
| Small named | 11.784 / 9.023 / 9.872 | 7.544 / 7.191 / 7.388 | +0.022 |
| Ordinary named | 28.706 / 23.456 / 23.906 | 21.837 / 20.839 / 19.757 | −2.277 |
| Large nodes | 236.169 / 224.906 / 207.237 | — | — |
| Large glyphs | 388.268 / 358.318 / 314.026 | — | — |
| Empty | 0.008 / 0.005 / 0.005 | 0.004 / 0.003 / 0.004 | approximately 0 |
| Missing atlas | 17.464 / 15.711 / 16.055 | 15.218 / 15.284 / 15.204 | −0.205 |
| Missing boxes | 18.271 / 16.607 / 14.667 | 14.835 / 15.335 / 14.466 | −0.279 |
| No glyphs | 17.760 / 18.487 / 19.732 | 15.489 / 16.032 / 14.738 | −0.729 |

The first no-glyph median increase of 1.971 µs did not repeat.
In the repeat, paired deltas ranged from −11.139 to +11.669 µs for no-glyph frames and −3.543 to +4.619 µs for small named frames.
The observed ordinary medians do not show a material regression, but the overlapping distributions and outliers do not establish a universal CPU speedup.
The large-glyph initial paired deltas were negative in all 11 samples, with median −83.203 µs;
this is evidence for that isolated upload workload only.
The retention decision rests on measured allocation removal with minimal ownership cost and the ordinary-frame repeat, not on a frame-rate prediction.

## Integration acceptance

The maintained `prepare_shadow_uploads_preserve_exact_cells_and_picture_across_panes` fixture runs both actual prepare sites, checks emitted counts and exact contiguous glyph ranges, reads back the uploaded prefixes and untouched tails, and compares rendered pixels against the parent's collected-vector bytes queued before the same draws execute.
Every nonempty node frame must contain varying RGB pixels so two flat backgrounds cannot pass as preserved geometry.
It alternates two pane IDs through font absence/arrival, 6→273 node growth, 7→75 glyph growth, empty frames, shrink, `NO_CELL` mapping and a glyph-empty frame whose packed boxes remain nonempty.
Only the two test buffers gain `COPY_SRC` and one sentinel cell;
their production capacity metadata and upload paths remain in use.
The scratch probe above separately exercises the large upload cardinalities.

Local actual-adapter validation, both unchanged golden sets, the single requested Claude Opus/xhigh review against the verified immediate parent, and final post-commit plugin/offline release builds remain delivery gates.
No golden is to be blessed and no shared DAW slot is to be swapped.
