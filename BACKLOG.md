# Backlog

Parking lot for small things noticed while verifying changes (usually in
Bitwig). Capture takes five seconds; nothing here is assigned work.

**Claude: never work on items in this file unless explicitly asked to.**
If asked mid-session to "add X to the backlog," append it under Items and
return to what you were doing — do not fix it.

Format: one line per item, `[area]` tag first, blank line between items.
Messy wording is fine — items get triaged and restated at dispatch time.
The blank lines are not cosmetic: this file is committed and several
sessions edit it at once, and git conflicts on hunks that touch adjacent
lines — so two sessions each deleting their own finished item conflict when
those items are neighbours, and merge cleanly when a blank line separates
them.

Areas: `ui`, `render`, `scene`, `core`, `standalone`, `plugin`, `build`, `docs`

## Sessions edit this file

The items are COMMITTED, so a session that finishes one deletes its line in
the same PR that does the work, and a skipped one gets an indented `—` note
saying why rather than being left looking untouched.

They are committed because the alternative was worse. Kept as uncommitted
working-tree edits, this file was reachable only from the main checkout —
and a background session works in a worktree, where the harness rejects
writes to the shared checkout. So every session ended by handing back a
list of lines to delete by hand, which is a step that gets skipped, and a
backlog that lies about what is still open is worse than one nobody prunes.

What that costs, and it is a real cost: this becomes the most contended
file in the repo, touched by every parallel session — the overlap the root
CLAUDE.md warns about, accepted deliberately here. Blank-line separation is
the cheap half of the mitigation. The other half is that a conflict in this
file is always trivial, because it is a list of independent lines: take
both sides. Never resolve one by dropping an item you did not finish.

## Items

[ui] collapsing or expanding a pane still flickers: everything teleports a
large amount LEFT for an instant and eases back over what looks like several
frames. Dragging the window border is fine since the spectrogram upload fix.
Measured and eliminated, so a future look need not repeat any of it: the frame
loop is clean (host streams set_size continuously, adopted within 1-2ms, `ui`
size matches `host` on every frame, ~5ms ticks, no hitch, one transitional
frame); no font-atlas uploads happen at all during a resize; the view's bounds
are applied exactly and immediately (`set 400x766` -> AppKit reports 400x766 on
the same call and every frame after, no intermediate values, nothing
animating). Two compositor hypotheses were tried in the plugin and did NOT fix
it, and were reverted rather than left in: disabling Core Animation's implicit
bounds animation (CATransaction with actions off around setFrameSize) and
pinning the layer's contentsGravity to top-left so stale contents are not
stretched across the new bounds. Both are in the history of PR #120 if wanted.
What has NOT been tried: making the bounds change and the new content land in
one commit (`presentsWithTransaction` on the CAMetalLayer, which needs
wgpu-hal-level access), and looking at what Bitwig does with the parent window
around the resize.

[settings] I don't like having the scrollbar on the top list of settings. Any solutions you can think of?
  — at 1512x886 there is now no scrollbar of either kind in the settings column: the tab bar clears its six names by 76pt, and folding Notes/Console gave the panes the height they were short of (Tuning/Nodes/Analyzer scrolled by 20/63/120pt before it, zero after). Pinned by `the_settings_column_needs_no_scroll_bar_at_the_window_it_was_dialled_in`. Widening the column was tried and withdrawn: it takes 8pt off the Spectral pane, which is already within a few points of being narrower than the perf HUD it has to hold. Reopen with the window size you saw it at if it is still there.

[ui] with Notes/Console folded by default, collapsing the settings leaf makes BOTH children of that split collapsed, so the whole settings column folds sideways to one rail — one click where it used to take two. Three reviewers read `fold::paint` as labelling that rail "Notes" (the settings leaf's body is 0px, so only the log leaf gets a name painted). Nobody could drive the painter to confirm it. If true, the rail naming the wrong pane is the bug, not the one-click fold.

[lattice] when resizing the pane, I see text popping in for a split second at random nodes

> Work through BACKLOG.md. First triage: restate each item as you understand
> it and flag anything ambiguous — ask me about those before touching them.
> Then fix the clear ones, one commit per item. Remove completed items from
> this file; annotate skipped ones with why.
