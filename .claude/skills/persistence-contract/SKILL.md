---
name: persistence-contract
description: How a saved blob survives a change — the container-level serde(default) rule and its two field-level exceptions, and why the UI_PERSIST_VERSION floor is no guard against a dropped enum variant. Use before adding, renaming or dropping a persisted field, struct or enum variant.
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

### The two field-level exceptions

`UiPersist::ui_scale` is the blob's one field-level `default = "..."`, and only because an `f32`'s own default of 0.0 is a scale of nothing.
Don't add others to it.

The offline renderer's `Layout` is outside the rule on purpose.
It is a `.ron` a person writes by hand (`--dump-layout` prints a preset to start from) rather than state the plugin saves, so `panes` is REQUIRED, the struct carries no container-level default at all, and `background` holds the tree's only other field-level `default = "..."`.

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
`load_persist` returns whether it applied and writes the reason to the console, the offline renderer prints to stderr, and `a_refused_blob_says_why` holds both.
Dropping an enum variant is still fine;
say so in the PR body, and keep the refusal audible.
