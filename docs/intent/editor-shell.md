# Intent: Editor shell, layout and saved state

Yan’s decisions recorded for future drift audits.

## 2026-10-02 (issue sweep)

- Retain Tab / Hide tab bars for the live editor, saved as an editor preference rather than appearance. Tab takes priority over focus traversal outside text edits and restores hidden fold controls. #1314 Editor Q1

The display link and frame pacing decisions (#1551) are in `host-window-and-pacing.md`.

## 2026-11-02 (drift audit, #1635)

- One wheel or trackpad zoom of the Analyzer is one undoable look edit, as a drag is: the gesture closes as one entry when it stops. Navigation stays in the look, so slots keep carrying the Analyzer's pitch range. #1635 Editor Q1, #1633

## 2026-11-10 (maintainability audit, #1677)

- A nested settings group repairs a non-finite value from its pane's or group's fresh value where it has one (Stars, Shadow), while a missing key loads the type default. Making them one rule was built and reverted in #1679, because a stored star speed means something different in each pane. #1677 area 5 Q2
- A retired setting adds no row to the `RETIRED_KEYS` test ledger: it keeps one row per shape of value a retired key can leave (a number, a flag, an enum token, a string, a struct), and a retirement edits no test. #1677 area 5 Q1
- Tail and the Tuning page's delay and map-offset fields are value bars, so they reset on double-click and type on Cmd-click like every other number; Stop at bar, ranging to 100,000, stays a drag value, the one a clippy rule lets through. #1677 area 3 Q1
- A bar's hover ends with the gestures that bar answers to, supplied by the widget in one wording, so no hover hand-writes them and none promises a reset or typed entry its bar lacks. #1677 area 3 Q2
- The tempered links (Meantone, Marvel) are tuning, not appearance: the runtime holds them, resolved from the configuration's owner, and the plugin saves them in its musical settings. The standalone app, with no such settings, no longer keeps its links across a restart and opens with both linked. #1677 area 2 Q1
- One list of the appearance fields the host owns, `AppearanceDocument::copy_host_owned` beside `set_camera_movement`: a look resets them on capture and keeps the live document's on apply, and the state extractor's overlay is tested against it. #1677 R3
- One call both takes an app shortcut and claims it from the host, so a shortcut is registered in one place each frame. #1675 Q2, built in #1690
- The poison list in `tests/settings_ranges.rs` stays a hand list, not a walk of the serialized document: its completeness test already forces a row per float setting, and a walker would need its own list of the values that cannot be driven to ±1e6. Declined, not to be re-proposed unchanged. #1677 R6 (Superseded the same day by #1714, which derived the poison from the serialized blob, `settings_ranges.rs:3-8`; the hand list is gone. #1739)

## 2026-11-14 (premises audit, #1739)

Calls marked "delegated" were made by the audit session after Yan asked it to settle the decisions with an obvious answer; the reason is part of the line.

- The editor's live state becomes the only copy of its saved settings: a host restore arrives through a mailbox the next lock holder applies, a save serializes the live state, and opening or closing the window stops reloading the blob, which removes both causes of #1740. Taken to `propose-rework` as F1′. Delegated: it is the one standing rework with a live defect behind it, and the smaller fix for #1740 leaves the drift class (#324, #1301, #1329, #1740) in place. #1739 D1
- Note fade and the pitch colour range move from host parameters into the look, so they undo, follow slots and export with it; their automation goes. Delegated: #1395 already keeps colours and note styling out of automation, and no take or project read shows them automated. #1739 D4, #1744
- The Analyzer's region folds keep resizing the window, with the Spiral borrowing a folded region's width. Delegated: the fold fix chain has been quiet since 2026-10-09, Tape was built without touching the lend, and the own window grants every resize at once, so moving the folds inside the pane would reverse #1124's answer on code weight alone. Declined. #1739 D8
- Completeness guards observe every input path (bars found by perturbing saved values, view floats NaN-poisoned in turn) instead of hand inventories; prototyped and timed first. #1739, #1752
- The standalone harness publishes egui's font texture through eframe, so its labels take the production text path and the CPU atlas mirror goes. #1739, #1754
