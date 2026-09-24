---
name: metal-corpus
description: When a change owes a regenerated Metal shader corpus, how to check it locally, and how to regenerate it. Use when a diff touches a .wgsl shader, harmonigraph-render pipelines or bind groups, harmonigraph-offline, vendor/wgpu-hal, Cargo.lock, or when the strict-catalog gate or the Metal shader assets workflow is red.
---

# A shader-affecting edit owes a regenerated Metal corpus

The always-loaded rule and the gate command are in `CLAUDE.md`;
this file is why the key is wider than `.wgsl`, what not to use as the signal, and how to regenerate.

`crates/harmonigraph-metal-assets/assets` holds precompiled Metal libraries keyed on shader hashes,
so **editing any `.wgsl` invalidates it, and the edit is not finished until the corpus is regenerated in the SAME commit**.

A `.wgsl` edit is the common case rather than the whole key, and treating it as the whole key is this repo's own too-narrow-key mistake written in prose.
The hash is over the **generated MSL** and the compiler options, which carry `hal::BACKEND_VERSION` —
so a bind-group or pipeline-layout change in Rust, a `vendor/wgpu-hal` bump or a `Cargo.lock` move invalidates the corpus with no `.wgsl` anywhere in the diff.
The workflow's own `paths:` filter already names the real set (`crates/harmonigraph-render/**`, `Cargo.lock`, `vendor/wgpu-hal/**`, `crates/harmonigraph-offline/**` and more);
read that, not the file extension.
`232d750e..24952475` is the worked example of the gap: 665 insertions across eight `harmonigraph-render` files, zero `.wgsl`, and by a `.wgsl`-only rule none of it owed anything.

It is also too coarse the other way: a COMMENT-only `.wgsl` edit owes nothing at all, because naga strips comments and the generated MSL is byte-identical.
`916c2429` is the worked example — it edited a comment in `spectrogram.wgsl` with no asset commit, and the corpus stayed valid.
So `git log -- crates/harmonigraph-metal-assets/assets` is a cheap check rather than the definition:
a shader PR absent from that log is a question, not yet a verdict.

The definition is the corpus itself, it is a `ci.sh` gate, and it answers locally in about fifteen seconds:

```
HARMONIGRAPH_SHADER_ASSETS=strict cargo test -p harmonigraph-render \
  --features shader-assets-tools -- --ignored --exact \
  shader_assets::catalog::production_metal_asset_catalog
```

`strict` drops the compile-from-source fallback, so a missing library fails the pipeline that wanted it and names its key.
The catalog is the enumeration of production constructors the corpus is generated FROM, so this covers every library in it rather than whatever the golden frames happen to draw —
and it is the same test in the same mode the `Metal shader assets` workflow reports as `strict-catalog`.

**Do not grep the fallback notice instead**, however much it looks like the signal:

```
Harmonigraph: Metal asset <hash> unavailable; compiling from source
```

That is an `eprintln!`, and libtest replays a test's captured output only when the test FAILS, so a green run swallows it whether the corpus is stale or not.
`cargo test -p harmonigraph-render golden 2>&1 | grep "compiling from source"` stood in `CLAUDE.md` as the definition and was silent in BOTH states;
a whole batch of sessions read that silence as a pass (#947).
`cargo test -p harmonigraph-render golden -- --nocapture` is what makes it visible, and is worth reading once the gate is already red, because it names every key that run missed where the gate stops at the first.

`cargo test --workspace`, `cargo fmt --all --check` and `cargo clippy` still pass against a stale corpus.
The gate is in `ci.sh`'s `isolated` group, so `Full CI` fails on one now —
but `Metal shader assets` still runs what it does not, the offline renderer's own pipelines and the corpus's recorded compiler flags and fallback controls, and still arrives as a separate workflow.
A branch honestly reported as "Full CI green" can still be `UNSTABLE` and unmergeable;
read `mergeStateStatus` rather than the one workflow whose name sounds like it covers everything.

Regenerate on the runner, not here:

```
gh workflow run "Metal shader assets" --ref <branch> -f regenerate=true
```

then `tools/shader-assets.py import` the `production-metal-assets` artifact.
`tools/shader-assets.py generate` does work locally and is the slow way to learn that it is the wrong path —
it rebuilds the renderer eight times over, against the production corpus and then against three deliberately broken variants of it, and on this machine it had produced nothing after twenty minutes.
PR #918 is the worked example, and it cost a full CI cycle on a diff whose own tests were green the whole time.
