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
- The tempered links (Meantone, Marvel) are tuning, not appearance: the runtime holds them, resolved from the configuration's owner, and the plugin saves them in its musical settings. The standalone app, with no such settings, no longer keeps its links across a restart and opens with both linked. #1677 area 2 Q1
- One list of the appearance fields the host owns, `AppearanceDocument::copy_host_owned` beside `set_camera_movement`: a look resets them on capture and keeps the live document's on apply, and the state extractor's overlay is tested against it. #1677 R3
- One call both takes an app shortcut and claims it from the host, so a shortcut is registered in one place each frame. #1675 Q2, built in #1690
- The poison list in `tests/settings_ranges.rs` stays a hand list, not a walk of the serialized document: its completeness test already forces a row per float setting, and a walker would need its own list of the values that cannot be driven to ±1e6. Declined, not to be re-proposed unchanged. #1677 R6
