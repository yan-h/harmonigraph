---
name: persistence-contract
description: How a saved blob survives a change — the container-level serde(default) rule, what a dropped enum variant costs at the editor and take doors, and why the UI_PERSIST_VERSION floor is no guard against one. Use before adding, renaming or dropping a persisted field, struct or enum variant.
---

# The persistence contract

The policy, and its one limit that a break must not be SILENT, live in `CLAUDE.md`.
This skill is the mechanism:
what keeps a break loud, and where each guard stops.

## A project saves its fields one by one

A project's state is a set of named fields beside the parameter values, and the two kinds fail differently.
The three editor fields that `crates/harmonigraph-plugin/src/saved_fields.rs` owns (`lattice-maps`, `editor-state`, `ui-state`):
one that does not parse is skipped with a Console line and the rest of the restore goes ahead,
and one the save lacks keeps what the instance holds.
Every restore line that reports a loss, at the editor's door or from the host restore, opens the Console on it: one spelling, `SharedState::report_loss` (Yan, 2026-11-22, `docs/intent/editor-shell.md`).
`SavedWindow` (the `editor-state` field) has no container-level default, so a window missing its `size` is that field skipped; a missing `docked` has a field-level default and loads as `false`.
The tuning's two fields, the Hub's `musical-settings` (`crates/harmonigraph-plugin/src/configuration.rs`) and every instance's `tuning-instance` (`crates/harmonigraph-plugin/src/tuning/setup.rs`),
open at their defaults both when the save lacks them and when they do not parse;
one that does not parse also writes a line in the log and, in a Hub, one in the Console naming it, which opens on it, and the rest of the restore goes ahead.
This supersedes the rule that such a field refused the whole restore with a line in the log alone:
the editor then stayed untouched, so the next save wrote the fresh look over the project's (#1927).
Nothing in these fields refuses a restore whole:
the tuning edit queue that once could is gone, since the main thread owns the tuning record and a restore writes it at once (#1938).
A parameter value is saved as an integer or a float (`ParamValue`), a choice by its index.
One that does not decode, such as one under a variant `ParamValue` no longer has, decodes as `ParamValue::Unread` and costs that value alone:
its row opens at its default with a line in the log (`Table::defaulted`), and a Hub also writes a Console line and opens the Console; a Tune only logs.

## Every struct inside the look carries a container-level `#[serde(default)]`

Inside `ui-state`, the editor's look and layout, the rule is finer.
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
an unknown KEY is skipped by serde, so retiring a field costs that field's setting alone,
and each door names it (#1927 B): `install_document` writes its path to the Console and opens it,
and an export says it as a `warning:` from the renderer or a line under the Video pane's Export button for a Recorded look,
as the renderer also names each value the look's load repaired, such as one past a range a later build narrowed, which the pane's Recorded export reaches by queueing the look unrepaired (#1956 Q2);
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
The one exception to "a save stores what the editor holds": an editor that has installed nothing and opened no window since it was fresh saves `""`, so its project keeps following the fresh look of whichever build loads it.
An empty restore installs the fresh look and leaves that flag as it was.
A restore therefore resets what it installs and nothing else, and a window session resets only what belongs to the window (`start_window_session`).
A new persisted field needs no step at either:
`UiPersist::capture` and `apply_persist` are two of its three sites.
The third is `UiPersist::stores`, the key an edit is compared on to mark the project modified (#1813).
It destructures the document, so a new top-level field of `UiPersist` does not compile until it is compared there;
each member compares whole by its derived `PartialEq`, so a field added inside one is in the key with nothing to do.
What no compiler catches: a `#[serde(skip)]` field added to any saved type is compared all the same, and marks the project at every change to it until `stores` copies it in from the live state.
State that belongs to the window rather than to the look, such as the Stars solo (#1828) and the Analyzer's spectrum hold (`SpectrumHold`), lives outside the saved types instead.
A value the editor writes by itself rather than by an edit is settled where it is written or copied in there likewise.

## Appearance and editor workspace have separate boundaries

`PictureState` owns `AppearanceDocument` directly;
`SharedState` adds the editor workspace around the picture.
Editor saves nest it beside the workspace;
take/record transport its serialized text opaquely.
`AppearanceDocument::parse` checks the appearance version and normalizes its settings once before export chooses output configuration or initializes drawing.
Editor load calls the same `normalize` implementation on its nested document before applying any workspace state.
An editor version-floor bump does not invalidate a recorded appearance.
`install_appearance` installs the normalized value and nothing else.
The links are not in the appearance:
the runtime holds them, resolved from the configuration's owner,
and the plugin saves them in `musical-settings` (#1677).
