//! Editor appearance history. The live document remains PictureState's;
//! snapshots deliberately exclude host state and output settings. The
//! Analyzer's navigation (pitch range, History, Level) is part of the look, so
//! undo and the slots carry it (#1635).

use crate::{AppearanceDocument, SpectrumConfig};
use harmonigraph_scene::{Camera, ViewConfig};

/// Undo, then the redo chords. Taken through [`crate::keys`], so the native
/// shell keeps them from the host: the same key cannot edit plugin history and
/// the host's history at once.
pub(crate) const APPEARANCE_SHORTCUTS: &[egui::KeyboardShortcut] = &[
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z),
    egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    ),
    #[cfg(not(target_os = "macos"))]
    egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::Y),
];

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Look {
    camera: Camera,
    view: ViewConfig,
    spectrum: SpectrumConfig,
}

impl Look {
    pub(crate) fn capture(appearance: &AppearanceDocument) -> Self {
        let mut view = appearance.view.clone();
        Self::preserve_external(&mut view, &ViewConfig::default());
        Self {
            camera: Camera {
                projection: appearance.camera.projection,
                cabinet_angle: appearance.camera.cabinet_angle,
                cabinet_scale: appearance.camera.cabinet_scale,
                ..Default::default()
            },
            view,
            spectrum: appearance.spectrum,
        }
    }

    fn preserve_external(to: &mut ViewConfig, from: &ViewConfig) {
        to.center_fives = from.center_fives;
        to.center_threes = from.center_threes;
    }

    pub(crate) fn apply(&self, appearance: &mut AppearanceDocument) {
        let mut view = self.view.clone();
        Self::preserve_external(&mut view, &appearance.view);
        appearance.camera.projection = self.camera.projection;
        appearance.camera.cabinet_angle = self.camera.cabinet_angle;
        appearance.camera.cabinet_scale = self.camera.cabinet_scale;
        appearance.view = view;
        appearance.spectrum = self.spectrum;
    }
}

/// Persisted as a fixed-length array, so changing this refuses older blobs whole.
pub(crate) const SLOT_COUNT: usize = 4;

/// The comparison slots. The active slot is always the live document, so its
/// entry is empty; there is no second writable copy. A slot never visited is
/// empty too, and starts as a copy of the current look when switched to.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Slots {
    stored: [Option<Look>; SLOT_COUNT],
    active: usize,
}

impl Slots {
    pub(crate) fn sanitize(&mut self) {
        self.active = self.active.min(SLOT_COUNT - 1);
        self.stored[self.active] = None;
        for look in self.stored.iter_mut().flatten() {
            look.camera.sanitize();
            look.view.sanitize();
            look.spectrum.sanitize();
        }
    }

    /// The look `slot` holds apart from the live document; `None` means the
    /// slot shows the current look.
    pub(crate) fn stored(&self, slot: usize) -> Option<&Look> {
        self.stored[slot].as_ref()
    }
}

pub(crate) fn slot_name(slot: usize) -> String {
    format!("Slot {}", slot + 1)
}

const HISTORY_LIMIT: usize = 64;

#[derive(Default)]
pub(crate) struct History {
    undo: Vec<Look>,
    redo: Vec<Look>,
    pending: Option<Look>,
}

impl History {
    fn push(&mut self, before: Look, after: &Look) {
        if before == *after {
            return;
        }
        if self.undo.len() == HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.undo.push(before);
        self.redo.clear();
    }

    pub(crate) fn finish(&mut self, appearance: &AppearanceDocument) {
        if let Some(before) = self.pending.take() {
            self.push(before, &Look::capture(appearance));
        }
    }

    pub(crate) fn observe(
        &mut self,
        before: Look,
        appearance: &AppearanceDocument,
        dragging: bool,
    ) {
        let after = Look::capture(appearance);
        if before != after && self.pending.is_none() {
            self.pending = Some(before);
        }
        if !dragging {
            self.finish(appearance);
        }
    }

    fn undo(&mut self, appearance: &mut AppearanceDocument) {
        self.finish(appearance);
        if let Some(before) = self.undo.pop() {
            self.redo.push(Look::capture(appearance));
            before.apply(appearance);
        }
    }

    fn redo(&mut self, appearance: &mut AppearanceDocument) {
        self.finish(appearance);
        if let Some(after) = self.redo.pop() {
            self.undo.push(Look::capture(appearance));
            after.apply(appearance);
        }
    }
}

enum Command {
    Switch(usize),
    SaveTo(usize),
}

#[derive(Default)]
pub(crate) struct AppearanceEditor {
    history: History,
    /// The inactive slots' histories; the active slot's entry is empty.
    parked: [History; SLOT_COUNT],
    pub(crate) slots: Slots,
    // Execute after the body commits any text field losing focus this frame.
    command: Option<Command>,
}

impl AppearanceEditor {
    fn switch(&mut self, slot: usize, appearance: &mut AppearanceDocument) {
        let active = self.slots.active;
        if slot == active {
            return;
        }
        self.history.finish(appearance);
        self.slots.stored[active] = Some(Look::capture(appearance));
        if let Some(look) = self.slots.stored[slot].take() {
            look.apply(appearance);
        }
        self.parked[active] = std::mem::take(&mut self.history);
        self.history = std::mem::take(&mut self.parked[slot]);
        self.slots.active = slot;
    }

    /// Copies the current look into another slot. Overwriting a stored look is
    /// undoable from that slot; an empty slot had no look to go back to.
    pub(crate) fn save_to(&mut self, slot: usize, appearance: &AppearanceDocument) {
        if slot == self.slots.active {
            return;
        }
        let look = Look::capture(appearance);
        if let Some(before) = self.slots.stored[slot].replace(look.clone()) {
            self.parked[slot].push(before, &look);
        }
    }

    pub(crate) fn restore(slots: Slots) -> Self {
        Self { slots, ..Default::default() }
    }

    pub(crate) fn end_frame(
        &mut self,
        before: Look,
        appearance: &mut AppearanceDocument,
        ctx: &egui::Context,
        text_edit_was_focused: bool,
    ) {
        // A drag, wheel or pinch closes as one entry on the first frame after
        // it ends. A frame that scrolls a settings pane changes no look, so it
        // holds nothing open on its own.
        self.history.observe(before, appearance, crate::gesture_open(ctx));
        // TextEdit leaves key events in the input even after handling them.
        // Keep its ownership for the whole frame: Undo followed by Enter can
        // undo text and release focus in one pass, but must not also undo a look.
        if crate::kept_focus(ctx) && !text_edit_was_focused && !ctx.text_edit_focused() {
            let taken = crate::keys::take_each(ctx, APPEARANCE_SHORTCUTS);
            for shortcut in &taken {
                if *shortcut == APPEARANCE_SHORTCUTS[0] {
                    self.history.undo(appearance);
                } else {
                    self.history.redo(appearance);
                }
            }
            if !taken.is_empty() {
                ctx.request_repaint();
            }
        } else {
            // Read by the text edit, and kept from the host by its focus: claimed
            // as well for a frame that ends the edit, which the next one undoes on.
            crate::keys::claim(ctx, APPEARANCE_SHORTCUTS);
        }
        if let Some(command) = self.command.take() {
            self.history.finish(appearance);
            match command {
                Command::Switch(slot) => self.switch(slot, appearance),
                Command::SaveTo(slot) => self.save_to(slot, appearance),
            }
            ctx.request_repaint();
        }
    }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            // Each slot stands on its own fill, as every choice option does.
            let mut chosen = self.slots.active;
            for slot in 0..SLOT_COUNT {
                crate::widgets::option_button(ui, &mut chosen, slot, &(slot + 1).to_string(), "");
            }
            if chosen != self.slots.active {
                self.command = Some(Command::Switch(chosen));
            }
            ui.menu_button("Save to", |ui| {
                for slot in (0..SLOT_COUNT).filter(|&slot| slot != self.slots.active) {
                    if ui.button(slot_name(slot)).clicked() {
                        self.command = Some(Command::SaveTo(slot));
                        ui.close();
                    }
                }
            });
        })
        .response
        .on_hover_text(format!(
            "Slots compare appearances; camera movement, tuning and output stay as they are. \
                 Edits land in the active slot, each slot keeps its own undo history, and an \
                 unused slot starts as a copy of the current look. {} undoes an edit and {} \
                 redoes it.",
            ui.ctx().format_shortcut(&APPEARANCE_SHORTCUTS[0]),
            ui.ctx().format_shortcut(&APPEARANCE_SHORTCUTS[1]),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut_event(shortcut: egui::KeyboardShortcut) -> egui::Event {
        egui::Event::Key {
            key: shortcut.logical_key,
            physical_key: None,
            modifiers: shortcut.modifiers,
            pressed: true,
            repeat: false,
        }
    }

    #[test]
    fn shortcuts_undo_redo_and_leave_other_keys_alone() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(original.clone(), &appearance, false);
        let changed = Look::capture(&appearance);
        for redo in &APPEARANCE_SHORTCUTS[1..] {
            for (shortcut, expected) in [(APPEARANCE_SHORTCUTS[0], &original), (*redo, &changed)] {
                let _ = ctx.run_ui(
                    egui::RawInput { events: vec![shortcut_event(shortcut)], ..Default::default() },
                    |ui| {
                        editor.end_frame(
                            Look::capture(&appearance),
                            &mut appearance,
                            ui.ctx(),
                            false,
                        );
                        assert_eq!(&Look::capture(&appearance), expected);
                        assert!(
                            ui.input(|i| i.events.is_empty()),
                            "handled shortcuts are consumed"
                        );
                    },
                );
            }
        }
        for shortcut in [
            egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Z),
            egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
                egui::Key::Z,
            ),
        ] {
            let _ = ctx.run_ui(
                egui::RawInput { events: vec![shortcut_event(shortcut)], ..Default::default() },
                |ui| {
                    editor.end_frame(Look::capture(&appearance), &mut appearance, ui.ctx(), false);
                    assert_eq!(Look::capture(&appearance), changed);
                    assert!(!ui.input(|i| i.events.is_empty()));
                },
            );
        }
    }

    #[test]
    fn text_field_undo_does_not_change_appearance_even_with_empty_text_history() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(original, &appearance, false);
        let mut text = String::new();
        let mut time = 0.0;
        let mut frame = |events| {
            time += 1.0;
            let _ = ctx.run_ui(
                egui::RawInput { time: Some(time), events, ..Default::default() },
                |ui| {
                    let text_edit_was_focused = ui.ctx().text_edit_focused();
                    let response = ui.add(egui::TextEdit::singleline(&mut text));
                    if time == 1.0 {
                        response.request_focus();
                    }
                    editor.end_frame(
                        Look::capture(&appearance),
                        &mut appearance,
                        ui.ctx(),
                        text_edit_was_focused,
                    );
                    assert_eq!(appearance.spectrum.attack, 0.2);
                    assert_eq!(editor.history.undo.len(), 1);
                },
            );
            text.clone()
        };
        frame(vec![]);
        assert_eq!(frame(vec![egui::Event::Text("Bright".into())]), "Bright");
        frame(vec![]);
        assert_eq!(frame(vec![shortcut_event(APPEARANCE_SHORTCUTS[0])]), "");
        assert_eq!(frame(vec![shortcut_event(APPEARANCE_SHORTCUTS[0])]), "");
        assert_eq!(frame(vec![egui::Event::Text("Warm".into())]), "Warm");
        frame(vec![]);
        assert_eq!(
            frame(vec![
                shortcut_event(APPEARANCE_SHORTCUTS[0]),
                shortcut_event(egui::KeyboardShortcut::new(
                    egui::Modifiers::NONE,
                    egui::Key::Enter
                )),
            ]),
            ""
        );
        assert!(!ctx.text_edit_focused(), "Enter must release the field's focus");
    }

    #[test]
    fn slider_drag_is_one_action_and_redo_is_invalidated_by_an_edit() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let original = appearance.spectrum.attack;
        let mut editor = AppearanceEditor::default();
        let mut rect = egui::Rect::NOTHING;
        let mut frame = |events| {
            let _ = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(400.0, 100.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let before = Look::capture(&appearance);
                    rect = crate::widgets::ValueBar::new(
                        &mut appearance.spectrum.attack,
                        0.0..=0.5,
                        "Attack",
                    )
                    .show(ui)
                    .rect;
                    editor.end_frame(before, &mut appearance, ui.ctx(), false);
                },
            );
            rect
        };
        let rect = frame(vec![]);
        let start = rect.center();
        frame(vec![egui::Event::PointerMoved(start)]);
        frame(vec![crate::tests::probe::press(start, true)]);
        for offset in [20.0, 40.0, 60.0] {
            frame(vec![egui::Event::PointerMoved(start + egui::vec2(offset, 0.0))]);
        }
        frame(vec![crate::tests::probe::press(start + egui::vec2(60.0, 0.0), false)]);
        assert_ne!(appearance.spectrum.attack, original, "fixture must actually drag the slider");
        let changed = appearance.spectrum.attack;
        assert_eq!(editor.history.undo.len(), 1);
        editor.history.undo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, original);
        editor.history.redo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, changed);
        editor.history.undo(&mut appearance);
        let before = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(before, &appearance, false);
        assert!(editor.history.redo.is_empty());
    }

    /// Zoom the Analyzer with `wheel(frame)` events over 40 frames and assert
    /// the whole gesture, however many frames it changed the look on, is one
    /// undo entry.
    fn assert_one_undoable_zoom(
        navigation: crate::panes::spectral::Navigation,
        wheel: impl Fn(usize) -> Option<egui::Vec2>,
    ) {
        let ctx = crate::tests::probe::themed();
        let mut state = crate::tests::probe::fresh_picture();
        state.appearance.spectrum.low_midi = 36.0;
        state.appearance.spectrum.high_midi = 96.0;
        let original = Look::capture(&state.appearance);
        let mut editor = AppearanceEditor::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0));
        let pointer = rect.center();
        let mut zooming_frames = 0;
        for frame in 0..40 {
            let mut events = vec![egui::Event::PointerMoved(pointer)];
            if let Some(delta) = wheel(frame) {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run_ui(
                egui::RawInput { screen_rect: Some(rect), events, ..Default::default() },
                |ui| {
                    let before = Look::capture(&state.appearance);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    crate::panes::spectral::spectral_pane(
                        &mut child, &mut state, 0.0, 1, 1.0, navigation,
                    );
                    zooming_frames += usize::from(before != Look::capture(&state.appearance));
                    editor.end_frame(before, &mut state.appearance, ui.ctx(), false);
                },
            );
        }
        assert!(zooming_frames >= 3, "the wheel must zoom over several frames: {zooming_frames}");
        assert_ne!(Look::capture(&state.appearance), original);
        assert_eq!(editor.history.undo.len(), 1);
        editor.history.undo(&mut state.appearance);
        assert_eq!(Look::capture(&state.appearance), original);
    }

    #[test]
    fn a_wheel_zoom_of_the_docked_analyzer_is_one_undoable_look_edit() {
        // Three wheel notches on consecutive frames, then the smoothed scroll
        // running out over the frames after them.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Docked, |frame| {
            (2..5).contains(&frame).then(|| egui::vec2(0.0, 20.0))
        });
    }

    #[test]
    fn a_trackpad_zoom_with_quiet_frames_between_events_is_one_undoable_look_edit() {
        // Point deltas under 8 pt are applied unsmoothed, so the frames
        // between two events read a zero scroll delta.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Docked, |frame| {
            ((2..12).contains(&frame) && frame % 2 == 0).then(|| egui::vec2(0.0, 6.0))
        });
    }

    #[test]
    fn a_wheel_zoom_of_the_video_previews_analyzer_is_one_undoable_look_edit() {
        // The preview takes the wheel's delta for itself, leaving the frame's
        // scroll delta zero while the wheel is still turning.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Preview, |frame| {
            (2..5).contains(&frame).then(|| egui::vec2(0.0, 20.0))
        });
    }

    #[test]
    fn a_secondary_button_drag_of_the_docked_analyzer_is_one_undoable_look_edit() {
        // egui starts a drag on a press of any button, and the Analyzer pans on
        // `dragged()`, so a right-button drag moves the look like a left one.
        let ctx = crate::tests::probe::themed();
        let mut state = crate::tests::probe::fresh_picture();
        state.appearance.spectrum.low_midi = 36.0;
        state.appearance.spectrum.high_midi = 96.0;
        let original = Look::capture(&state.appearance);
        let mut editor = AppearanceEditor::default();
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 500.0));
        let start = rect.center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut events: Vec<Vec<egui::Event>> =
            vec![vec![egui::Event::PointerMoved(start)], vec![button(start, true)]];
        let mut at = start;
        for _ in 0..12 {
            at += egui::vec2(15.0, 0.0);
            events.push(vec![egui::Event::PointerMoved(at)]);
        }
        events.push(vec![button(at, false)]);
        events.push(vec![]);
        let mut moving_frames = 0;
        for events in events {
            let _ = ctx.run_ui(
                egui::RawInput { screen_rect: Some(rect), events, ..Default::default() },
                |ui| {
                    let before = Look::capture(&state.appearance);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    crate::panes::spectral::spectral_pane(
                        &mut child,
                        &mut state,
                        0.0,
                        1,
                        1.0,
                        crate::panes::spectral::Navigation::Docked,
                    );
                    moving_frames += usize::from(before != Look::capture(&state.appearance));
                    editor.end_frame(before, &mut state.appearance, ui.ctx(), false);
                },
            );
        }
        assert!(moving_frames >= 3, "the drag must pan over several frames: {moving_frames}");
        assert_eq!(editor.history.undo.len(), 1);
        editor.history.undo(&mut state.appearance);
        assert_eq!(Look::capture(&state.appearance), original);
    }

    #[test]
    fn every_slot_stands_on_its_own_fill() {
        let ctx = crate::tests::probe::themed();
        let mut editor = AppearanceEditor::default();
        let size = egui::vec2(400.0, 100.0);
        let out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| editor.toolbar(ui),
        );
        let fills: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == crate::theme::widget() => Some(rect.rect),
                _ => None,
            })
            .collect();
        for slot in 0..SLOT_COUNT {
            let label = (slot + 1).to_string();
            let at = out
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => Some(text.pos),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("slot {label} is drawn"));
            assert!(fills.iter().any(|fill| fill.contains(at)), "slot {label} has no fill");
        }
    }

    #[test]
    fn window_focus_loss_finishes_a_pending_action() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let before = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(before, &appearance, true);
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::WindowFocused(false)],
                ..Default::default()
            },
            |ui| {
                assert!(ui.input(|i| i.focused), "fixture matches baseview's unchanged raw flag");
                editor.end_frame(Look::capture(&appearance), &mut appearance, ui.ctx(), false);
            },
        );
        assert!(editor.history.pending.is_none());
        assert_eq!(editor.history.undo.len(), 1);
    }

    #[test]
    fn slots_keep_their_own_histories_and_save_to_is_undoable_in_its_target() {
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = Look::capture(&appearance);
        editor.switch(1, &mut appearance); // An unused slot starts as the current look.
        assert_eq!(Look::capture(&appearance), original);
        let before = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(before, &appearance, false);
        let bright = Look::capture(&appearance);
        editor.switch(0, &mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        assert!(editor.history.undo.is_empty());
        editor.save_to(1, &appearance);
        editor.switch(1, &mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        editor.history.undo(&mut appearance);
        assert_eq!(Look::capture(&appearance), bright, "the overwrite undoes first");
        editor.history.undo(&mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        editor.history.redo(&mut appearance);
        assert_eq!(Look::capture(&appearance), bright);
        editor.switch(3, &mut appearance);
        assert_eq!(Look::capture(&appearance), bright);
        editor.switch(0, &mut appearance);
        assert_eq!(Look::capture(&appearance), original);
    }

    #[test]
    fn project_restore_retains_slots_but_discards_history() {
        let mut state = crate::tests::probe::fresh();
        let original = Look::capture(&state.picture.appearance);
        let editor = &mut state.workspace.interaction.appearance_editor;
        editor.switch(1, &mut state.picture.appearance);
        state.picture.appearance.spectrum.attack = 0.2;
        editor.history.observe(original.clone(), &state.picture.appearance, false);
        editor.save_to(2, &state.picture.appearance);
        let blob = state.save_persist();
        assert!(state.load_persist(&blob));
        let editor = &mut state.workspace.interaction.appearance_editor;
        assert_eq!(editor.slots.active, 1);
        assert!(editor.slots.stored(1).is_none(), "the active slot is the live document");
        assert_eq!(editor.slots.stored(2).unwrap().spectrum.attack, 0.2);
        assert!(editor.history.undo.is_empty());
        editor.switch(0, &mut state.picture.appearance);
        assert_eq!(Look::capture(&state.picture.appearance), original);
    }

    #[test]
    fn keyboard_undo_runs_after_the_body_commits_its_edit() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = appearance.spectrum.attack;
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![shortcut_event(APPEARANCE_SHORTCUTS[0])],
                ..Default::default()
            },
            |ui| {
                let before = Look::capture(&appearance);
                // ValueBar commits a typed value on losing focus, after toolbar drawing.
                appearance.spectrum.attack = 0.2;
                editor.end_frame(before, &mut appearance, ui.ctx(), false);
            },
        );
        assert_eq!(appearance.spectrum.attack, original);
        editor.history.redo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, 0.2);
    }

    #[test]
    fn undo_preserves_incoming_camera_and_output() {
        let mut appearance = AppearanceDocument::default();
        let mut history = History::default();
        let before = Look::capture(&appearance);
        let original = appearance.spectrum.attack;
        appearance.spectrum.attack = 0.2;
        history.observe(before, &appearance, true);
        appearance.camera.distance = 7.0;
        appearance.view.center_fives = 12;
        appearance.render.short_edge = 1920;
        history.finish(&appearance);
        history.undo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, original);
        assert_eq!(appearance.camera.distance, 7.0);
        assert_eq!(appearance.view.center_fives, 12);
        assert_eq!(appearance.render.short_edge, 1920);
    }
    #[test]
    fn queued_slot_survives_undo_slot_edits_and_incoming_camera() {
        use crate::params::{ParamBackend, ParamKey};
        struct Camera;
        impl ParamBackend for Camera {
            fn get(&self, key: ParamKey) -> f32 {
                key.default_value()
            }
            fn set(&self, _: ParamKey, _: f32) {}
            fn camera_value(&self, key: ParamKey) -> Option<f32> {
                Some(if key == ParamKey::CameraDistance { 7.0 } else { key.default_value() })
            }
        }
        let mut state = crate::tests::probe::fresh();
        let appearance = &mut state.picture.appearance;
        let editor = &mut state.workspace.interaction.appearance_editor;
        let original = Look::capture(appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(original.clone(), appearance, false);
        editor.save_to(1, appearance);
        editor.history.undo(appearance); // The export must take slot 1, not the current look.
        appearance.sync_camera(&Camera);
        appearance.render.short_edge = 1080;
        state.workspace.interaction.take.export_look = Some(1);
        let queued = crate::panes::render::capture_export(
            appearance,
            &state.workspace.interaction,
            std::path::Path::new("music.take"),
        )
        .unwrap();
        let editor = &mut state.workspace.interaction.appearance_editor;
        editor.switch(1, appearance);
        appearance.spectrum.attack = 0.8;
        editor.switch(0, appearance);
        editor.save_to(1, appearance);
        appearance.spectrum.attack = 0.8;
        appearance.camera.distance = 5.0;
        appearance.render.short_edge = 720;
        let crate::ExportAction::Queue { appearance: blob, size, .. } = queued else {
            panic!("queue request")
        };
        let captured = AppearanceDocument::parse(&blob).unwrap();
        assert_eq!(captured.spectrum.attack, 0.2);
        assert_eq!(captured.camera.distance, 7.0);
        assert_eq!(captured.render.short_edge, 1080);
        assert_eq!(size, captured.render.frame.pixels(1080));
        assert_eq!(appearance.spectrum.attack, 0.8);
    }
}
