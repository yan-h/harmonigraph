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
- Folding the Analyzer's Spectrum or history region keeps its space inside the Analyzer instead of resizing the window, retiring `region_widths`, the Spiral's width lend and the fold handshake; section folds still resize the window. This meets #1124's need (the Spiral gets its width) by removing its cause, replacing #1149's lend. First declined on delegation because the fold fixes had gone quiet; Yan reopened it, judging by what the code costs to own rather than how recently it broke. #1739 D8, #1755
- Completeness guards observe every input path (bars found by perturbing saved values, view floats NaN-poisoned in turn) instead of hand inventories; prototyped and timed first. #1739, #1752
- The standalone harness publishes egui's font texture through eframe, so its labels take the production text path and the CPU atlas mirror goes. #1739, #1754

## 2026-11-14 (premises audit #1739, rework #1757)

- The editor's live settings are the only copy: a host restore installs into them, a host save serializes them, and opening or closing the editor window reads nothing saved, so the look slots' undo histories survive a close (#1740). A restore still starts them afresh. #1757
- A project saved with the editor shut is rewritten in this build's shape: keys this build does not know are dropped, missing keys are saved at today's defaults, and repaired values are saved repaired. #1757 Q1
- An editor never opened keeps saving an empty settings blob, so its project follows later default captures. #1757 Q2
- Restoring an empty settings blob means the fresh look, not "keep what the editor shows". #1757 Q3
- A save the host makes from inside a callback the editor makes under its lock (a parameter gesture's flush, a camera move) cannot take the lock, and stores the newest of the last restore, close, successful save or first frame to draw the editor rather than the live settings: the stated limit of #1301. From the frame after the first one to draw the editor, it never stores the empty blob, since restoring that is the fresh look; a save re-entered during that first frame itself still can. #1757 (first-frame half superseded below)
- A document this build could not fully read is saved verbatim until a frame draws the editor past its loading screen, whatever that frame draws; a frame with no area counts, an occluded window's frames count. Accepted without a fourth red-team round. #1757 QD (superseded below)
- At the editor's door, a saved value naming an enum variant this build does not have costs only the key that holds it, which opens at its default with a Console line, rather than the whole layout, look, slots and presets. This supersedes the whole-parse rule `AGENTS.md` carried. #1757 Q4
- At the take and export door the parse stays strict: a take whose recorded look names a dropped variant keeps refusing to export, loudly. #1757 Q5
- The protocol and the per-key decode landed as one PR, so the per-key half cannot be reverted on its own. #1757 QA

## 2026-11-14 (simplifying #1757's protocol)

- A save always stores what the editor holds. A restore this build refuses whole, or cuts a key from, still opens the Console and says why, but the next save stores the settings as they stand rather than the document as it came. This supersedes QD and the "never lost silently" rating for this case: on macOS the hold changed a save made before a frame next drew the editor — before the window was ever opened, or with it shut after an earlier session, as when a preset is loaded then — and per-key decoding already bounds a dropped variant's loss to one key with a Console line. #1757
- An editor that has installed nothing and opened no window since it was fresh still saves the empty blob (Q2); installing a document or opening a window ends that, rather than a frame drawing the editor. An empty restore leaves it as it was, so it never turns an editor's later saves into the empty blob. #1757
- A save that cannot take the editor lock stores the newest of the last restore, close or successful save, the stated limit of #1301. The first drawn frame no longer refreshes it: the fallback is the empty blob only after a fresh start or an empty restore, both of which leave the fresh look, so before any save or close the refreshed copy restored the same look as the empty blob it replaced. #1757
- No guard is kept for an X11 frame after the close, or for a restore landing inside an X11 frame's hold: on macOS, the one platform that ships, closes, frames, restores and saves share the host's main thread. #1757

## 2026-11-17 (#1813, in chat)

- Any edit to what `ui-state` saves marks the Bitwig project modified, as a musical setting's does: the layout and pane sizes, view settings, look slots, camera presets and editor preferences (option A). A restore, opening the editor, and what the editor writes by itself, such as the layout fitted to the window, mark nothing. #1813

## 2026-11-18 (#1824, in chat)

- An edit to what `editor-state` saves, the window's size and the System tab's Own window choice, marks the Bitwig project modified too, through the same notice as `ui-state` (option A). #1824
That a host-driven resize while the editor opens does not count, that a restore does not, and that the opening ends with the window's first frame are the implementer's reading, settled by the coordinating session, not Yan's answer: option A left the host resize open and named no restore.

## 2026-11-18 (maintainability audit, #1861)

Yan took the recommendations made after the audit was checked against the code ("go with all recommendations", 2026-11-18).

- `read-plugin-state` may build and run Rust: it becomes a program beside the plugin that reads a project through the plugin's own restore code, `set_camera_movement` and `copy_host_owned`, keeping a raw dump, and the Python scrapers and the pre-#1685 Spiral fallback go. This supersedes the 2026-11-10 line that the extractor's overlay is tested against `copy_host_owned`: it calls the function instead. To be built. #1861 Q2, R2
- Unselected Curve channel toggles (Hue, Brightness, Saturation) and Stars Solo layer buttons wear the same filled button as every other option, and the plain egui toggle is disallowed in the UI crate. #1861 Q3

## 2026-11-18 (#1814, in chat, after step 7)

- The editor adopts egui-plugin-window's `EditorSize`, as its own PR after step 7 (#1865), retiring Harmonigraph's own copy of the resize negotiation. [#1814 comment](https://github.com/yan-h/harmonigraph/issues/1814#issuecomment-7002343121)
- Bitwig's border drag stops at the editor's limits, the own window's 400 × 300 to 8192 × 8192 points, through `gui.adjust_size`, and a host `set_size` outside them is refused. Before, a size below the minimum was clamped to it and one above the maximum taken, so the host's window had no maximum. #1814
- A restore while the editor is open (a preset, Bitwig's undo) resizes the window to the restored size at its next frame, rather than the project holding a size the window does not have. #1814

## 2026-11-19 (#1875, in chat)

- A restore marks the project modified neither by itself nor through the resize it makes at the next frame. This is now Yan's answer ("no", recommendation taken), replacing the coordinating session's reading recorded under 2026-11-18 (#1824) for the restore half. #1875
- Closing the editor right after a section fold, before the window has resized, saves the fold's requested width, so the editor reopens as it was left. Yan's answer ("keep", recommendation taken). #1875
