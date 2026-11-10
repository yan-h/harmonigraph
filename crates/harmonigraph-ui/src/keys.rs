//! The app shortcuts a frame reads with nothing focused, and the claim the
//! native shell makes on them so the host does not act on the same press.
//!
//! Taking a shortcut and claiming it are one call, [`take`]: a shortcut read
//! here is a shortcut kept from the host. The shell hands [`claimed`] to
//! egui-baseview's `KeyCapture::CaptureShortcuts` at the end of each frame.
//!
//! A claim is a prediction. macOS asks the shell at key-down whether to pass
//! a key up, before the frame that will read it runs, so the claims a key
//! meets are the ones the PREVIOUS frame registered. That is why [`take`]
//! registers whether or not a press arrived: a shortcut that applies only
//! under a condition (the map arrows, over the lattice in Map mode) is
//! claimed on every frame the condition holds, ready for the press that has
//! not come yet.
//!
//! Matched exactly, as the shell matches: egui's own `consume_shortcut` lets
//! extra modifiers through (Shift-Tab matches Tab), which would act here on a
//! press the shell passes up to the host.

use egui::KeyboardShortcut;

/// This pass's claims, stamped with the pass that made them so a pass that
/// registers nothing — the loading screen — claims nothing, rather than
/// whatever the last drawn frame did.
#[derive(Clone, Default)]
struct Claims {
    pass: u64,
    shortcuts: Vec<KeyboardShortcut>,
}

fn claims_id() -> egui::Id {
    egui::Id::new("harmonigraph_ui::keys::Claims")
}

/// Keep `shortcuts` from the host this frame without reading them: for a
/// branch where a focused widget reads the key instead, which the shell then
/// claims by its focus anyway. Claiming it here too keeps the key from the
/// host on the frame the focus leaves, which the next frame takes it on.
pub(crate) fn claim(ctx: &egui::Context, shortcuts: &[KeyboardShortcut]) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|data| {
        let claims = data.get_temp_mut_or_default::<Claims>(claims_id());
        if claims.pass != pass {
            *claims = Claims { pass, shortcuts: Vec::new() };
        }
        for shortcut in shortcuts {
            if !claims.shortcuts.contains(shortcut) {
                claims.shortcuts.push(*shortcut);
            }
        }
    });
}

/// Claim `shortcuts` and remove this frame's presses of exactly any of them,
/// returning which was pressed, in the order the presses arrived.
pub(crate) fn take_each(
    ctx: &egui::Context,
    shortcuts: &[KeyboardShortcut],
) -> Vec<KeyboardShortcut> {
    claim(ctx, shortcuts);
    let mut taken = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key { key, modifiers, pressed: true, .. } = event else {
                return true;
            };
            let Some(shortcut) = shortcuts.iter().find(|shortcut| {
                *key == shortcut.logical_key && modifiers.matches_exact(shortcut.modifiers)
            }) else {
                return true;
            };
            taken.push(*shortcut);
            false
        });
    });
    taken
}

/// Claim `shortcut` and remove this frame's presses of exactly it, returning
/// how many there were.
pub(crate) fn take(ctx: &egui::Context, shortcut: KeyboardShortcut) -> usize {
    take_each(ctx, &[shortcut]).len()
}

/// The shortcuts this pass has claimed, for the shell to keep from the host
/// until the next frame. Read inside the pass, after the UI has drawn.
pub fn claimed(ctx: &egui::Context) -> Vec<KeyboardShortcut> {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|data| data.get_temp::<Claims>(claims_id()))
        .filter(|claims| claims.pass == pass)
        .map_or_else(Vec::new, |claims| claims.shortcuts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Key, Modifiers};

    fn press(key: Key, modifiers: Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    /// A shortcut taken is a shortcut claimed, pressed or not, and for the
    /// pass that took it only.
    #[test]
    fn a_shortcut_taken_is_claimed_for_that_frame_whether_or_not_it_was_pressed() {
        let ctx = egui::Context::default();
        let tab = KeyboardShortcut::new(Modifiers::NONE, Key::Tab);
        for events in [vec![], vec![press(Key::Tab, Modifiers::NONE)]] {
            let pressed = events.len();
            let _ = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                assert_eq!(take(ui.ctx(), tab), pressed);
                assert_eq!(claimed(ui.ctx()), [tab]);
            });
        }
        let _ = ctx.run_ui(Default::default(), |ui| {
            assert_eq!(claimed(ui.ctx()), [], "a frame that takes nothing claims nothing");
        });
    }

    /// Exactly the chord the shell claims, so no press acts both here and in
    /// the host; and in order, so undo then redo in one frame stays that.
    #[test]
    fn take_removes_exact_presses_in_the_order_they_arrived() {
        let ctx = egui::Context::default();
        let undo = KeyboardShortcut::new(Modifiers::COMMAND, Key::Z);
        let redo = KeyboardShortcut::new(Modifiers::COMMAND.plus(Modifiers::SHIFT), Key::Z);
        let events = vec![
            press(Key::Z, redo.modifiers),
            press(Key::Z, Modifiers::COMMAND.plus(Modifiers::ALT)),
            press(Key::Z, undo.modifiers),
        ];
        let _ = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            assert_eq!(take_each(ui.ctx(), &[undo, redo]), [redo, undo]);
            assert_eq!(ui.input(|i| i.events.len()), 1, "Cmd-Option-Z is the host's");
        });
    }
}
