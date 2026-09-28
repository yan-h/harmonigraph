# Reproduce Stars architecture trials

Read [REPORT.md](REPORT.md) first.
This directory is evidence, not production code.
All experiment hooks are contained in `research.patch`; the production tree is unchanged in this PR.

Use an owner-managed worktree at `7711b98d2c28c737b5a9c44dd298eb3d96efe589` and copy this directory outside that checkout.
The patch adds temporary runtime source selection and two ignored probes.
It deliberately bypasses the shipped Metal catalog via source mode.
The test and offline binaries must therefore never be mistaken for a selected production implementation.

```sh
bundle=/absolute/path/to/copied/evidence
export RESEARCH_WORKTREE="$PWD"
git apply "$bundle/research.patch"
cargo test --release -p harmonigraph-render --lib --no-run --message-format=json > /private/tmp/stars-build.json
export RESEARCH_TEST_BINARY=$(python3 -c 'import json; a=[json.loads(s) for s in open("/private/tmp/stars-build.json")]; print(next(x["executable"] for x in a if x.get("executable") and x.get("target",{}).get("name")=="harmonigraph_render"))')
python3 "$bundle/run.py" synthetic-replay base-a,base-b,group3-short75,group3-50 --input synthetic --frames 240
```

Python requires NumPy; PNG generation uses the included small PNG helper and needs no Pillow.
Timing requires timestamp queries and must run alone: no other GPU probes, builds, captures or video encoding.
The default input is the private take-derived fixture; use `--input synthetic` for a self-contained run.
Set `HG_FIXTURE` to a directory containing `take-levels.u8` and `flat-levels.u8` (each 960 by 1024 unsigned bytes) and `palette.rgba` (4096 RGBA entries) for recorded timing and image probes.
The exact reused fixtures are identified by SHA-256 in `source-manifest.json`; they are not committed.
The default fixture location is `/private/tmp/stars-full-halo-compare`.
Each timing/capture directory refuses overwrite; use a new label.

The committed WGSL files are the exact tested source substitutions.
`generate.py` reconstructs them from `shaders/base.wgsl` and the inherited complete-response reference.
The complete-response reference originates in the prior #1244 evidence, itself adapted from immutable snapshot `a5ed674ea18427001ea9029fb9f8c17f3827c116`.
The final harness fixes the shifted image fixture and take clock described in the report; manifests retain the distinct historical binary hashes.

```sh
python3 "$bundle/probe_images.py" replay-parity base-a,micro,complete,direct4,direct4-ref9,full-residual4,full-a,group5-100 1.25 0.5 7.3
cargo build --release -p harmonigraph-plugin -p harmonigraph-offline
export TAKE_FILE=/absolute/path/to/take-2026-09-11_03-16-17.take
python3 "$bundle/capture.py" base-a group3-short100 group3-short75 group3-50 direct4 group5-50
python3 "$bundle/gallery.py" shortlist base-a group3-short100 group3-short75 group3-50 direct4 group5-50
python3 "$bundle/compare_movies.py"
```

The original movies, lossless arrays and GPU readbacks remain local artifacts; the committed package includes hashes, metrics and selected contact sheets instead of adding gigabytes of private captures to Git.
Recorded movies require the original take; no audio or raw take is included.
`timings` contains all completed raw measurements and settings, including the explicitly excluded disturbed run.
`probes` retains both the rejected shifted fixture and corrected parity results.
Process snapshots are omitted because unrelated application command lines are not relevant evidence to publish.

The final diff changes documentation and research artifacts only.
No Metal corpus update is required until a candidate becomes production code.
