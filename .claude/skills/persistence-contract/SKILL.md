---
name: persistence-contract
description: How a saved blob survives a change — the container-level serde(default) rule, and why the UI_PERSIST_VERSION floor is no guard against a dropped enum variant. Use before adding, renaming or dropping a persisted field, struct or enum variant.
---

# The persistence contract

The policy, and its one limit that a break must not be SILENT, live in `CLAUDE.md`.
This skill is the mechanism:
the two things that keep a break loud, and where each stops.

## Every persisted struct carries a container-level `#[serde(default)]`

A struct NESTED inside a persisted struct needs the attribute in its own right.
A struct added without it is invisible at its declaration —
nothing about the type says it is missing —
so the coverage is the `*_missing_any_one_*` sweeps in `crates/harmonigraph-ui/src/tests/persist.rs`, which walk every key or section rather than pinning one field.
Add a persisted struct, and those tests are what catch a forgotten attribute.

`impl Default` is therefore the one and only source of a field's fallback:
no second set of values anywhere, and retuning the fresh look is free.
A key missing from a blob costs that key alone.

To see which structs currently carry it:

```
grep -rn --include='*.rs' -A4 '#\[serde(default)\]' crates/ | grep 'pub struct'
```

### Editor workspace defaults

`UiPersist`'s `Default` uses the same design-scale helper as `Interaction`, since an `f32` default of 0.0 is a scale of nothing.
A missing editor version defaults to 0 and is refused by the version floor.

`panes::Tab` is persisted as the selected analyzer and settings tabs in `UiPersist::layout`, so removing one of its variants is a dropped variant (below), and its doc comment says so.
The offline renderer's `Layout` and `Placement`, and the standalone view picker `Pane`, are runtime types with no serde derive;
the captured lattice placement and split persist in `RenderFrame`.

## The floor is no guard against a dropped variant

`UI_PERSIST_VERSION` refuses a blob below it whole rather than half-reading it.
What survives an old blob otherwise is only what serde gives free:
an unknown KEY is skipped, so retiring a field is safe.
An unknown VARIANT is not —
it fails the parse and drops the entire persist, layout and camera with it.

The floor does not help, though it is easy to assume otherwise:
the version is read out of a struct that never parsed, so the check never runs.
Raising the floor does nothing for a dropped variant at any value.

What makes that acceptable is that the refusal is LOUD —
`load_persist` returns whether it applied and writes the reason to the console, the offline renderer prints to stderr, and focused editor/offline tests hold refusal and defaulting.
Keep the refusal audible when dropping a variant.

## Appearance and editor workspace have separate boundaries

`PictureState` owns `AppearanceDocument` directly;
`SharedState` adds the editor workspace around the picture.
Editor saves nest it beside the workspace;
take/record transport its serialized text opaquely.
`AppearanceDocument::parse` checks the appearance version and normalizes its settings once before export chooses output configuration or initializes drawing.
Editor load calls the same `normalize` implementation on its nested document before applying any workspace state.
An editor version-floor bump does not invalidate a recorded appearance.
`install_appearance` installs the normalized value and restores its tuning links alongside the next parameter snapshot.
