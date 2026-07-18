# Backlog

Parking lot for small things noticed while verifying changes (usually in
Bitwig). Capture takes five seconds; nothing here is assigned work.

**Claude: never work on items in this file unless explicitly asked to.**
If asked mid-session to "add X to the backlog," append it under Items and
return to what you were doing — do not fix it.

Format: one line per item, `[area]` tag first. Messy wording is fine —
items get triaged and restated at dispatch time.

Areas: `ui`, `render`, `scene`, `core`, `standalone`, `plugin`, `build`, `docs`

## Items

<!-- - [ui] example: console pane scrollbar jumps when new lines arrive -->

## Dispatching (notes for Yan)

Suggested prompt when draining the list (optionally scoped, e.g. "just the
[ui] items"):

> Work through BACKLOG.md. First triage: restate each item as you understand
> it and flag anything ambiguous — ask me about those before touching them.
> Then fix the clear ones, one commit per item. Remove completed items from
> this file; annotate skipped ones with why.
