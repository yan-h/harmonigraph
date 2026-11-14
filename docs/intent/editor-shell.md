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
- The poison list in `tests/settings_ranges.rs` stays a hand list, not a walk of the serialized document: its completeness test already forces a row per float setting, and a walker would need its own list of the values that cannot be driven to ±1e6. Declined, not to be re-proposed unchanged. #1677 R6

## 2026-11-14 (premises audit #1739, rework #1757)

- The editor's live settings are the only copy: a host restore installs into them, a host save serializes them, and opening or closing the editor window reads nothing saved, so the look slots' undo histories survive a close (#1740). A restore still starts them afresh. #1757
- A project saved with the editor shut is rewritten in this build's shape: keys this build does not know are dropped, missing keys are saved at today's defaults, and repaired values are saved repaired. #1757 Q1
- An editor never opened keeps saving an empty settings blob, so its project follows later default captures. #1757 Q2
- Restoring an empty settings blob means the fresh look, not "keep what the editor shows". #1757 Q3
- A save the host makes from inside a callback the editor makes under its lock (a parameter gesture's flush, a camera move) cannot take the lock, and stores the newest of the last restore, close or successful save rather than the live settings: the stated limit of #1301. #1757
- A document this build could not fully read is saved verbatim until a frame draws the editor past its loading screen, whatever that frame draws; a frame with no area counts, an occluded window's frames count. Accepted without a fourth red-team round. #1757 QD
- At the editor's door, a saved value naming an enum variant this build does not have costs only the key that holds it, which opens at its default with a Console line, rather than the whole layout, look, slots and presets. This supersedes the whole-parse rule `AGENTS.md` carried. #1757 Q4
- At the take and export door the parse stays strict: a take whose recorded look names a dropped variant keeps refusing to export, loudly. #1757 Q5
- The protocol and the per-key decode landed as one PR, so the per-key half cannot be reverted on its own. #1757 QA
