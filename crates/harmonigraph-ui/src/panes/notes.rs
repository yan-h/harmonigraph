//! The Console pane (log scrollback) and the Notes pane (held-voice
//! debug printout).

use super::{display_note_name, nearest_shown_node, KEY_NAMES};
use crate::widgets::button_row;
use crate::{theme, SharedState};

pub(super) fn console_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    button_row(ui, |ui| {
        ui.label(format!("{} held", state.tracker.held_count()));
        if ui.button("Clear").clicked() {
            state.console.clear();
        }
    });
    // Its own area rather than the dock's, because it sticks to the bottom and
    // the Clear row above it does not scroll — so it needs the bar's lane
    // reserved out of its width (see [`theme::reserve_scroll_gutter`]).
    theme::reserve_scroll_gutter(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        for line in state.console.lines() {
            ui.monospace(line);
        }
    });
}

/// Debug printout of all held notes, one line per voice, sorted by
/// descending absolute pitch. Note names use the 12-TET spelling of the
/// MIDI key; octave numbers use Bitwig's convention (middle C = C3); the
/// cents column shows the sounding pitch class (including per-note
/// tuning); the node column shows which lattice position the pitch class
/// lights up under the current tuning/tolerance and view extents ("--"
/// = sounding but not represented anywhere on the visible lattice).
pub(super) fn notes_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    let mut voices: Vec<_> =
        state.tracker.voices().filter(|v| v.state == harmonigraph_core::VoiceState::Held).collect();
    if voices.is_empty() {
        ui.weak("No held notes.");
        return;
    }
    voices.sort_by(|a, b| b.pitch.total_cmp(&a.pitch));

    ui.monospace(
        egui::RichText::new("note  oct     cents  node     ch")
            .monospace()
            .color(theme::text_dim()),
    );
    // Its own area rather than the dock's, so the column header above stays put
    // while the voices scroll under it; the bar's lane comes out of its width.
    theme::reserve_scroll_gutter(ui);
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        // The window the lattice is drawing, so a voice lit on a node
        // is never listed as having none.
        let shown = state.shown();
        for voice in voices {
            let node = nearest_shown_node(&shown, &state.tuning, voice.pitch_class)
                .map(|pos| display_note_name(pos, state.view.tempered()).to_string());
            let line = format!(
                "{name:<4} {oct:>4} {cents:>8.2}\u{a2}  {node:<7} {ch:>2}",
                name = KEY_NAMES[usize::from(voice.note % 12)],
                oct = voice.display_octave(),
                cents = voice.pitch_class.to_cents(),
                node = node.as_deref().unwrap_or("--"),
                ch = voice.channel + 1,
            );
            let mut text = egui::RichText::new(line).monospace();
            if node.is_none() {
                // Sounding but invisible on the lattice: flag the row.
                text = text.color(theme::warning_text()).background_color(theme::warning_bg());
            }
            ui.label(text);
        }
    });
}
