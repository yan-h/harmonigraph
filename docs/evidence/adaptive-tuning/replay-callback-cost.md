# Targeted known-state replay callback observations

> **Frozen.**
> Measured at commit `f5f7464f`, before the [#712](https://github.com/yan-h/harmonigraph/issues/712) simplification, and not maintained since.
> The workload it times included the channel-wave replay that no longer exists, so the reproduction command below still names a live test that no longer runs this workload.
> [The archive index](README.md) records what has changed.

Both final runs passed the actual production CLAP fixture at workload commit `f5f7464ff663fad3f6aaad51c68206a6d1d4992a`, with production Source unchanged from the independently reviewed `0e1a0339` correction.
Each profile ran once under normal ambient app load, including 16 success and 16 partial-prefix rejection/repair episodes.
The largest observed sum of all 16 Source callbacks and the Hub callback in one replay block was **3.951876 ms in guarded optimized-dev** and **3.264163 ms in release**;
both maxima occurred during the 256-onset phase.
The setup-continuation maxima were 0.578583 ms and 0.569917 ms respectively.
These are measured samples, not a worst-case bound or full #616 D512/Bitwig deadline qualification.

The [complete per-owner/phase summary statistics](../../data/replay-callbacks-f5f7464f.csv) contain 728 rows:
720 replay summaries and eight existing empty/held summaries.
The [run metadata](../../data/replay-callbacks-f5f7464f.json) preserves the exact commands, revision, environment, profile, timestamps, reach receipts and single baseline-onset observation.
No favorable run was selected from repeated final measurements.

## Environment and timer boundary

The coordinator verified a MacBookPro18,3 with Apple M1 Pro, eight physical/eight logical CPUs and 16 GiB memory, running Darwin 25.6.0 ARM64_T6000. The compiler was rustc 1.92.0 (`ded5c06cf21d2b93bffd5d884aa6e96934ee4234`), LLVM 21.1.3, targeting aarch64-apple-darwin.
Optimized-dev uses workspace opt-level 2 and dependency opt-level 3;
release uses the repository release profile with LTO disabled.
`nice-plug/assert_process_allocs` was selected for both runs.
The wrapper's allocation/deallocation guard is enabled only with debug assertions, so optimized-dev supplies that evidence and release does not.

At the coordinator's 2026-09-06 11:08:26 UTC clearance, no Cargo, rustc, clang, linker or sccache process was present, and this task was the sole agent build owner.
User apps remained active:
WindowServer 60.8%, BitwigPluginHost 50.6%, CodexService 23.7%, Codex 15.2%, Renderer 10.3% and Bitwig 9.5% in that per-process snapshot.
Those percentages are not total machine utilization or continuous load monitoring.
No user app was stopped or changed.
The guarded run began at 11:10:28 UTC and release at 11:11:55 UTC;
the exact timestamp strings are retained in the metadata.

Each duration brackets the actual exported `plugin.process` call, including the wrapper and preallocated test-host accepted/rejected event bookkeeping.
Input construction, assertions, Source snapshots, canonical-record draining and instance construction occur outside that timer.
For every replay block, all 16 Source durations and the Hub duration are summed before computing the aggregate distribution.
That sum excludes the host's scheduling, inter-callback work and jitter;
it is not an observed Bitwig graph deadline.
The values also do not isolate the allocation guard's cost from other profile and ambient-load differences.

## Workload and functional reach

Every owner advances by 512 samples at 44.1 kHz.
Source00 is the nonparticipating raw-MIDI replay target, Sources01–03 supply 192 held voices, Sources04–15 are 12 enrolled silent Tunes, and the Hub has 63 DIRECT voices.
Together with target A, these reach 256 real session reservations.

Each episode accepts known neutral pedal seeds, 13 raw bank/program and RPN/NRPN transaction messages, and 2,064 later CC7 headers.
It then captures delayed B while A remains active, preserving responsive original A controls and physical Off.
The younger wave replays exactly 2,081 setup writes;
four continuation callbacks each reach 512 real output attempts, with the remaining 33 setup writes before handoff.
No consumed CC88=37 is restored.

Success accepts adjacent CC88=55 and B-On at the same actual sample, followed by exact original CC7, pitch bend value 8,256, pressure, sostenuto, Off and Up offsets.
B's original duration of 36 samples is preserved, and positive canonical B-On/Off records carry those actual samples.
Rejection derives its attempt positions from the remaining setup, accepts CC88=55, rejects the exact B-On bytes, rejects the first neutral repair, then accepts CC88=0. The preallocated host trace proves both refused messages;
no B voice or canonical B note is invented.

Every episode proves Hub received/applied cuts equal Source acknowledgements:
4,181 on success and 4,175 on rejection.
Observed peak target occupancy is 2,086 Pending records, 2,074 references and 1,280 journal entries;
rejection additionally reaches one emergency entry.
Accepted journal/emergency history and all session credits settle before destruction.
Reconstructable transaction history deliberately remains at 2,084 Pending/2,074 references while the producer is live;
actual producer destruction then returns all registry counts to zero.
Live Reset is not misreported as clearing reconstruction history.

The full-capacity 8,192-event/32,768-reference and 64-target service cases are covered elsewhere functionally;
this timing fixture does not claim to measure their worst-case cost.
Unknown overwritten initial controller state remains outside this known-state slice and still requires the separate product decision.

## Aggregate block observations

All values below are **mean / p95 / maximum in microseconds**, summing all 17 measured callbacks in each block.
`n` is the number of block sums per profile.
The full CSV also includes p50, each of the 17 owners separately, and actual attempt/acceptance totals.
Percentiles use the fixture's sorted sample at index `floor(n * 0.95)`;
with 16 observations, p95 is the observed maximum.

| Variant | Phase | n | Guarded optimized-dev | Release |
| --- | --- | ---: | ---: | ---: |
| success | enrollment | 16 | 290.443 / 352.210 / 352.210 | 252.095 / 376.960 / 376.960 |
| success | onset256 | 16 | 3070.242 / 3355.499 / 3355.499 | 2876.276 / 3264.163 / 3264.163 |
| success | history | 688 | 124.986 / 210.499 / 407.209 | 107.378 / 168.251 / 331.000 |
| success | prefix | 16 | 77.661 / 147.583 / 147.583 | 74.607 / 161.833 / 161.833 |
| success | younger-capture | 16 | 78.112 / 146.961 / 146.961 | 72.318 / 129.166 / 129.166 |
| success | old-release | 16 | 1055.320 / 1268.793 / 1268.793 | 1016.828 / 1270.748 / 1270.748 |
| success | setup | 64 | 292.319 / 432.458 / 550.793 | 274.204 / 424.792 / 554.253 |
| success | handoff | 16 | 93.864 / 167.626 / 167.626 | 92.885 / 155.538 / 155.538 |
| success | reporting | 128 | 126.461 / 322.041 / 438.667 | 106.096 / 280.834 / 343.249 |
| success | cleanup | 256 | 134.700 / 1463.002 / 1552.918 | 124.992 / 1426.749 / 1800.581 |
| reject-repair | enrollment | 16 | 324.456 / 481.749 / 481.749 | 234.482 / 280.584 / 280.584 |
| reject-repair | onset256 | 16 | 3103.946 / 3951.876 / 3951.876 | 2896.046 / 3043.504 / 3043.504 |
| reject-repair | history | 688 | 128.053 / 201.794 / 414.332 | 107.346 / 161.460 / 328.462 |
| reject-repair | prefix | 16 | 80.683 / 227.498 / 227.498 | 68.729 / 119.917 / 119.917 |
| reject-repair | younger-capture | 16 | 74.928 / 91.499 / 91.499 | 67.039 / 94.294 / 94.294 |
| reject-repair | old-release | 16 | 1034.499 / 1135.831 / 1135.831 | 969.948 / 1171.043 / 1171.043 |
| reject-repair | setup | 64 | 290.736 / 433.584 / 578.583 | 278.686 / 442.750 / 569.917 |
| reject-repair | rejection-repair | 16 | 101.427 / 254.209 / 254.209 | 93.588 / 231.665 / 231.665 |
| reject-repair | reporting | 128 | 133.739 / 306.666 / 445.788 | 107.230 / 267.373 / 378.749 |
| reject-repair | cleanup | 256 | 135.019 / 1445.458 / 1869.250 | 125.153 / 1403.832 / 1768.708 |

The individual setup owners show where that aggregate was spent.
Each row contains 64 callbacks per profile, again in mean / p95 / maximum microseconds.

| Variant | Owner | Guarded optimized-dev | Release |
| --- | --- | ---: | ---: |
| success | Source00 | 116.430 / 139.959 / 161.750 | 109.121 / 139.584 / 175.458 |
| success | Hub | 143.347 / 266.584 / 297.125 | 142.068 / 281.917 / 387.291 |
| reject-repair | Source00 | 116.253 / 140.500 / 161.792 | 113.842 / 163.917 / 254.167 |
| reject-repair | Hub | 143.185 / 268.458 / 317.209 | 140.906 / 275.417 / 342.375 |

## Existing empty and held observations

The original timing workload also ran in each profile, with 256 observations per row.
Its held-state label means 256 held voices, not full storage or all service branches.
Values are mean / p95 / maximum microseconds.

| Original phase/owner | Guarded optimized-dev | Release |
| --- | ---: | ---: |
| empty-source | 1.675 / 1.667 / 22.666 | 1.093 / 1.125 / 22.875 |
| empty-hub16enrolled | 13.647 / 19.500 / 104.834 | 12.609 / 16.791 / 59.375 |
| held64-source | 2.048 / 3.667 / 9.167 | 1.546 / 2.500 / 28.416 |
| held256-hub16enrolled | 13.822 / 26.459 / 98.541 | 12.551 / 21.667 / 73.625 |

Its single 256-onset Hub callback was 3.383458 ms in optimized-dev and 3.171750 ms in release.
That single-owner observation is separate from the repeated replay episodes' 17-owner sums above.

## Reproduction and validation

The existing exact timing-test exclusion still excludes every added timing workload from the normal functional suite.
`HARMONIGRAPH_REPLAY_REHEARSAL=1` runs one episode per variant with elapsed-time reports suppressed for fixture development.
It was unset for both final measurements.

```
env -u HARMONIGRAPH_REPLAY_REHEARSAL RUSTC_WRAPPER='' CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 cargo test -p harmonigraph-plugin --lib --features nice-plug/assert_process_allocs performance::tests::observed_callback_cost_at_empty_and_full_session_state -- --exact --nocapture --test-threads=1
env -u HARMONIGRAPH_REPLAY_REHEARSAL RUSTC_WRAPPER='' CARGO_BUILD_JOBS=2 RUST_TEST_THREADS=1 cargo test --release -p harmonigraph-plugin --lib --features nice-plug/assert_process_allocs performance::tests::observed_callback_cost_at_empty_and_full_session_state -- --exact --nocapture --test-threads=1
```

Both exact tests passed:
optimized-dev in 1.65 s and release in 1.57 s, excluding compilation.
The stable helper also passed independent fixture review, guarded rehearsal, 145 existing functional tests with the exact timing test excluded, focused Clippy and formatting checks.
Actual ordinary memory remains 60,123,376 bytes per session in the [allocation ledger](memory-ledger.md).
This checkpoint does not complete #616/#621 policy/capacity integration, representative simultaneous musical workload coverage, host scheduling validation, routing/calibration audition or the pending unknown-state decision.
