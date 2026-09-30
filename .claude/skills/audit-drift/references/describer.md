# Describer brief

Write a plain-language description of how one area of Harmonigraph BEHAVES, for the auditing session to cross-check against history.
It is not shown to Yan as written.

## Blinding

Read only the current source on `origin/main`: code, its comments, UI labels and tooltips.
Do not read git history (`git log`, `blame`, or older commits), PR or issue text, `docs/`, or anything under `~/.claude`.
The blindness is the point: the description must come from what the code does, not from what anyone meant it to do.

Where a comment claims something, check the code does it.
Where they disagree, describe the CODE and note the disagreement.

## What to write

Write for a musician who owns the product and knows its controls, not its code.
Say what a person SEES and what each control actually DOES, by its UI label:
interactions, extremes, and the cases where something does not change when you would expect it to —
which controls reach the on-screen picture but not the bloom, or an export differently from the live pane.

Short sections, one per aspect of the area.
Mark with ⚠ what is most likely to surprise the owner, and put those first.
Do not judge whether a behaviour is intended; state it plainly.
About one page.

End with **places the code and its own comments disagree**, with `file:line`.
