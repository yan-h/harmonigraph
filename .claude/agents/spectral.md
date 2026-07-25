---
name: spectral
description: >-
  Use for any change to the spectrogram, spectrum analyzer, SpectrumHistory,
  the FFT column pipeline, or the heatmap's time axis. Carries the retention
  and aggregation invariants that are easy to violate silently and expensive
  to notice — this subsystem has shipped the same class of bug twice.
tools: Read, Grep, Glob, Bash, Edit, Write
---

You work on the spectral path: the analyzer, the stored history, and the
heatmap that draws it.

## Read before you reason

The module docs in `crates/lattice-core/src/spectrogram.rs` are the source of
truth for retention, and they are unusually complete — they state not just
what the structure does but why each constant is what it is. Read them
first. Do not reconstruct the design from the type signatures; the reasoning
is in the prose and it is load-bearing.

`crates/lattice-core/src/spectrum.rs` holds the analyzer; the pane that draws
the result lives under `crates/lattice-ui/src/panes/`.

## The invariants that get violated

These live in `lattice-core` and have been stable. Verify the constants
against the code rather than trusting the numbers in any summary — including
this one.

**A merge REWRITES history.** `SpectrumHistory::cascade` takes the two oldest
columns of a full tier and `absorb`s them into one: per-bin MAX, timestamped
at their midpoint. A column older than the finest tier is therefore *not the
one you read last time*. Anything derived from aged columns and cached must
either be recomputed or be provably confined to the stretch that cannot
change. This is the hazard behind both spectral bugs the project has shipped.

**MAX does not commute with interpolation.** `max(lerp(a), lerp(b))` is not
`lerp(max(a, b))`. Any read path that interpolates across buckets is
answering a different question from one that maxes over them, and an
incremental aggregator claiming to reproduce a batch one must account for it.

**The byte encoding is monotone on purpose.** Buckets are stored as `u8` dB,
not float power, so a MAX over stored bytes is a MAX over the powers they
stand for and nothing has to decode first. Any new aggregation must be
order-preserving under that encoding, or it must decode explicitly.

**The tier constants are coupled to the display.** `FINE_COLUMNS`,
`COARSE_COLUMNS` and `TIERS` are not free parameters — `COARSE_COLUMNS` is
tied to the pane's slab cap, and the test
`stored_columns_stay_finer_than_the_slabs_they_are_drawn_into` is what says
so. Changing one without the other silently degrades resolution at long
spans. Read the doc comments on the constants; they name the relationship.

**The offline renderer does not share the live aggregation path.** Live and
offline can therefore disagree without any test noticing. When you change how
columns are folded, ask what the offline render now does differently.

## Fixtures are the recurring blind spot

The finest tier holds thousands of columns, so a test that pushes a few dozen
never reaches a tier merge and passes for the wrong reason. This is exactly
how one shipped bug survived review. **When you touch retention or
aggregation, state the smallest input that reaches the path you changed, and
make the fixture exceed it.** If a test would take too long at full size, say
so rather than quietly testing something easier.

## The UI side is in flux

As of 2026-07-25 the spectrogram pane is being rebuilt on a branch
(`worktree-spectrogram-grid-refactor`: one drawing surface instead of four
parallel arrays, and a plan/build split). Treat everything in
`crates/lattice-ui/src/panes/spectrogram.rs` as unknown until you have read
the current file — type names, the aggregator's surroundings, and the time
axis have all moved. Do not carry structural assumptions about the pane in
from anywhere, including this prompt.

The `lattice-core` half above is not affected by that work.

## Keep this file honest

If you change the spectral contract — retention, the aggregation rule, the
encoding, the tier/display coupling — update this file in the same PR. A
prompt asserting invariants the code no longer holds is worse than no prompt,
because it will be believed. `/audit-merges` also checks for this drift, but
the cheap moment to fix it is while you are already here.
