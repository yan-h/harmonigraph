---
name: persistence-contract
description: How a saved blob survives a change — the container-level serde(default) rule, what a dropped enum variant costs at the editor and take doors, and why the UI_PERSIST_VERSION floor is no guard against one. Use before adding, renaming or dropping a persisted field, struct or enum variant.
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

To see which structs currently carry it, private and `pub(crate)` ones included (`UiPersist`, the editor's top-level save, is one):

```
grep -rn --include='*.rs' -A4 '#\[serde(default)\]' crates/ | grep -E -- '-[0-9]+-(pub[^ ]* )?struct '
```

### Editor workspace defaults

`EditorPrefs`'s `Default` is the one source of the UI scale's design size (1.0), since an `f32` default of 0.0 is a scale of nothing;
`UiPersist` derives its `Default` from its parts'.
A missing editor version defaults to 0 and is refused by the version floor.

`panes::Tab` is persisted as the selected analyzer and settings tabs in `UiPersist::layout`, so removing one of its variants is a dropped variant (below), and its doc comment says so.
The offline renderer's `Layout` and `Placement`, and the standalone view picker `Pane`, are runtime types with no serde derive;
the captured lattice placement and split persist in `RenderFrame`.

## A dropped variant costs its key at the editor's door

`UI_PERSIST_VERSION` refuses a blob below it whole rather than half-reading it.
What survives an old blob otherwise:
an unknown KEY is skipped by serde, so retiring a field costs nothing;
an unknown VARIANT is cut with the key that names it by the editor's per-key decode (`crates/harmonigraph-ui/src/persist_keys.rs`), so that key opens at its default and the rest of the document loads (#1757).
A variant no key holds of its own, a bare element of a list or map, still refuses the document whole;
none is persisted today.

The floor is no guard against a dropped variant, though it is easy to assume otherwise:
it compares a document's version, not the variants it names, so a project an earlier build saved at the current version passes it,
and raising it would refuse every older document whole rather than cost the one key.

What makes the loss acceptable is that it is LOUD and bounded —
`install_document` writes each cut key to the Console and opens it, and only that key is lost.
A host save then stores the settings as they stand, like any other:
the document as it came is not kept back.
`a_document_naming_any_one_unknown_variant_costs_that_key_alone` in `crates/harmonigraph-ui/src/tests/persist.rs` holds this for every variant a filled document writes,
so retiring a variant owes that test nothing.

The take and export door, `AppearanceDocument::parse`, keeps the strict parse (#1757 Q5):
a take whose recorded look names a dropped variant refuses to export, and the offline renderer prints why to stderr.

## The live editor state is the only copy

A host restore installs into the editor state through `install_document`, and a host save serializes out of it through `save_answer`;
nothing reads a saved document back, so a window opening or closing installs nothing (#1757).
The one exception to "a save stores what the editor holds": an editor never opened since it was fresh or restored empty saves `""`, so its project keeps following the fresh look of whichever build loads it.
A restore therefore resets what it installs and nothing else, and a window session resets only what belongs to the window (`start_window_session`).
A new persisted field needs no step at either:
`UiPersist::capture` and `apply_persist` are its two sites.

## Appearance and editor workspace have separate boundaries

`PictureState` owns `AppearanceDocument` directly;
`SharedState` adds the editor workspace around the picture.
Editor saves nest it beside the workspace;
take/record transport its serialized text opaquely.
`AppearanceDocument::parse` checks the appearance version and normalizes its settings once before export chooses output configuration or initializes drawing.
Editor load calls the same `normalize` implementation on its nested document before applying any workspace state.
An editor version-floor bump does not invalidate a recorded appearance.
`install_appearance` installs the normalized value and adopts the next parameter snapshot verbatim, without rejudging the tuning links.
The links are not in the appearance:
the runtime holds them, resolved from the configuration's owner,
and the plugin saves them in `musical-settings` (#1677).
