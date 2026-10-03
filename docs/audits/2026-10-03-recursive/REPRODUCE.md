# Reproduce this audit's checks

Use an owner-managed worktree based on `ad1c6e6b9474dd80098703ae15cac241bba0eb41` with this evidence directory available.
Never apply the scratch probes in the main checkout or an active development worktree.
Commands below assume the worktree root.
The audit branch changes evidence only after the probes are restored.

## Shared release build and baseline checks

```sh
./session-lifecycle.sh run -- cargo test --workspace --release --no-run -j 3
./session-lifecycle.sh run -- env WGPU_BACKEND=metal HARMONIGRAPH_REQUIRE_GPU=1 HARMONIGRAPH_REQUIRE_FFMPEG=1 cargo test --workspace --release -- --test-threads=1
./session-lifecycle.sh run -- cargo build --release -p harmonigraph-offline -j 3
node --test tools/adaptive-tuning-simulator/model.test.mjs
```

Workspace tests unify development features, including assertion hooks; this is not an assertion of identical build features to the installed plugin.
The separate offline executable uses its package's normal release configuration.
No golden baselines are blessed.

## Timings

Run only after compilation and other audit-owned work ends.
The runner refuses competing cargo/rustc/ffmpeg/offline processes, takes the `/private/tmp/yan-deep-audits-expensive.lock` advisory lock shared with Otonal, records process names/load before and after each run, and executes commands sequentially.
It does not terminate or suspend other applications.
Active desktop/game/DAW load remains a limitation.
After Otonal coordination began, builds and test execution also used `experiments/with_shared_lock.py` inside the repository lifecycle wrapper.
Do not nest that wrapper around the measurement runner, which acquires the same lock itself.

```sh
python3 docs/audits/2026-10-03-recursive/experiments/prepare_workloads.py
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/run_measurements.py ui --rounds 3 --tag rerun
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/run_measurements.py export --rounds 3 --tag rerun
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/run_measurements.py gpu --rounds 3 --tag rerun
```

The UI scenario uses the existing 1600×1000 point, 2×-density test driver, six held MIDI voices and 800 mono samples per frame at 48 kHz, 60 warmups and 600 measured frames.
It excludes GPU callback preparation and native host/window scheduling.

Exports include real ffmpeg x264 encoding, soundtrack muxing and final encoder completion.
Synthetic input is a deterministic eight-second, six-voice progression with four harmonics per voice and current default appearance.
The historical input is an explicit scratch conversion of a local v5 take, preserving its canonical events/parameters/configuration but converting every repeated header to v6 and current default appearance.
It is not the old captured appearance and not a newly recorded v6 host take.
Original recording and WAV stay unchanged; generated inputs/output are local ignored artifacts.
The manifest records source hash and every transformation.
When the historical recording is unavailable, the synthetic workload remains reproducible; skip that runner case explicitly rather than substitute data silently.

## Narrow probes

```sh
python3 docs/audits/2026-10-03-recursive/experiments/probes.py apply
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/with_shared_lock.py cargo test --workspace --release audit_ --no-run -j 3 > docs/audits/2026-10-03-recursive/logs/build-probes.log 2>&1
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/with_shared_lock.py python3 docs/audits/2026-10-03-recursive/experiments/run_probes.py
python3 docs/audits/2026-10-03-recursive/experiments/probes.py restore
```

Keep the workspace package/feature selection consistent with the initial build; narrowing the package set can rebuild a different dependency graph.
The runner compiles the tiny native layout query against the real cached dependency rlibs, then runs each scratch test binary separately and asserts that one test was actually reached.
The NaN assertion fails intentionally on the audited source; its exit 101 is saved without preventing later probes.
Inspect every exit in logs/probe-results.json, then restore source.
This is reproduction of a defect, not an expected failure in the unmodified baseline suite.
If more than one dependency rlib exists, choose the compatible artifact explicitly rather than guessing.
The guarded script requires exact original source before application and exact probe source before restoration.
If it refuses, inspect the difference; do not overwrite unrelated edits.
It preserves the test-only patch.
No shipping behavior is changed by these snippets.

- Hub: three real 1024-entry queues; matching and mismatched epoch controls, counting popped/retained entries.
- Nonfinite input: one NaN, followed by two complete clean audio windows; independently requires finite, lit newest history before asserting display recovery.
  A failure of this scratch assertion reproduces the hypothesis; it is separate from the unmodified baseline suite.
- Naming: fixed actual Just E through Just and current equal-tempered lookup; asserts failed node lookup before fallback.
- GPU: fixed 1080p Stars surface, only halo resolution changes, retained old handle prevents identity reuse from hiding replacement; other shapes stay equal.
  Thirty stable/changing pairs alternate order; queue submission and poll occur outside the prepare timer.
  This is stage cost, not a measured benefit from a candidate fix.
  Memory shape equality is not a claim of memory identity.

Choose a fresh --tag on each measurement retry; the runner refuses to overwrite an existing log.
Use --case historical1080 or chord720/chord1080 for a bounded export retry, and --test-binary with the exact build-log executable if several variants exist.
After a workspace test build, rebuild the normal offline package feature set before timing exports; the test build can replace its executable with unified features.
Keep repeated measurements separate from compilation.
`logs/measurements.jsonl` preserves exact executable paths, arguments, environments and workload context for the baseline samples.
See `HOST-VALIDATION.md` for checks deliberately left to a safe Bitwig session.

## Completed-media inspection

```sh
./session-lifecycle.sh run -- python3 docs/audits/2026-10-03-recursive/experiments/with_shared_lock.py python3 docs/audits/2026-10-03-recursive/experiments/inspect_exports.py
```

The script names the two completed local outputs from this audit; select your corresponding completed output names for a fresh --tag.
It verifies streams/frame counts and extracts local-only frames for visual inspection.
The broad GPU sweep command above is preserved for continuation but was not executed in this audit; the completed suite, exports and narrow allocation probe supplied the relevant current evidence.
