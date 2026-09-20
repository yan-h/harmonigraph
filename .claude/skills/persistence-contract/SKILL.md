---
name: persistence-contract
description: How a saved blob survives a change — the container-level serde(default) rule and its field-level exception, and why the UI_PERSIST_VERSION floor is no guard against a dropped enum variant. Use before adding, renaming or dropping a persisted field, struct or enum variant.
---

# The persistence contract

Backwards compatibility is not a constraint here —
that policy and its one limit, that a break must not be SILENT, live in `CLAUDE.md`.
This skill is the mechanism:
the two things that keep a break loud, and where each stops.

## Every persisted struct carries a container-level `#[serde(default)]`

A struct NESTED inside a persisted struct needs the attribute in its own right.
A struct added without it is invisible at its declaration —
nothing about the type says it is missing —
so the coverage is two sweeping tests in `harmonigraph-ui`'s persist tests, which walk every key and every section rather than pinning one field.
Add a persisted struct, and those tests are what catch a forgotten attribute.

`impl Default` is therefore the one and only source of a field's fallback:
no second set of values anywhere, and retuning the fresh look is free.
A key missing from a blob costs that key alone.

To see which structs currently carry it:

```
grep -rn --include='*.rs' -A4 '#\[serde(default)\]' crates/ | grep 'pub struct'
```

### The field-level exception

`UiPersist::ui_scale` is the blob's one field-level `default = "..."`, and only because an `f32`'s own default of 0.0 is a scale of nothing.
Don't add others to it.

The offline renderer's `Layout` and `Placement` are runtime composition types, not serialized state.
Custom layout RON input and its dump interface were retired under #974, so their former field-level exception is gone.
`panes::Tab` is the editor dock's persisted enum — `UiPersist::dock` is a `DockState<panes::Tab>`, so `Tab`'s variants are the contract a saved dock depends on, and its own doc comment carries the reasoning and the #975 precedent for retiring one.
`Pane` is the standalone view picker and is not the dock's;
it still derives serde, but nothing in the tree serializes it.
`RenderFrame` still carries the captured placement and proportion.

## The floor is no guard against a dropped variant

`UI_PERSIST_VERSION` refuses a blob below it whole rather than half-reading it.
What survives an old blob otherwise is only what serde gives free:
an unknown KEY is skipped, so retiring a field is safe.
An unknown VARIANT is not —
it fails the parse and drops the entire persist, layout and camera with it.

It is worth being exact about why the floor does not help, because it is easy to assume otherwise:
the version is read out of a struct that never parsed, so the check never runs.
Raising the floor does nothing for a dropped variant at any value.

What makes that acceptable is that the refusal is LOUD —
`load_persist` returns whether it applied and writes the reason to the console, the offline renderer prints to stderr, and focused editor/offline tests hold refusal and defaulting.
Dropping an enum variant is still fine;
say so in the PR body, and keep the refusal audible.

## Appearance and editor workspace have separate boundaries

`PictureState` owns `AppearanceDocument` directly;
`SharedState` adds the editor workspace around the picture.
Editor saves nest it beside the workspace;
take/record transport its serialized text opaquely.
`AppearanceDocument::parse` checks the appearance version and normalizes its settings once before export chooses output configuration or initializes drawing.
Editor load calls the same `normalize` implementation on its nested document before applying any workspace state.
An editor version-floor bump does not invalidate a recorded appearance.
`install_appearance` installs the normalized value and clears the tuning detection verdicts;
context-owned resources and workspace dial invalidation retain their separate lifetimes.
