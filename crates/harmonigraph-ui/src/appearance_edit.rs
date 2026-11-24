//! Editor appearance history. The live document remains PictureState's;
//! snapshots deliberately exclude host state and output settings. The
//! Analyzer's navigation (pitch range, History, Level) is part of the look, so
//! undo carries it (#1635).

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

/// The appearance as undo holds it: the document's camera, view and spectrum,
/// with the fields the host owns ([`AppearanceDocument::copy_host_owned`])
/// reset on capture and the live document's kept on apply. Output settings are
/// not part of a look.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Look {
    camera: Camera,
    view: ViewConfig,
    spectrum: SpectrumConfig,
}

impl Look {
    pub(crate) fn capture(appearance: &AppearanceDocument) -> Self {
        let mut look = Self {
            camera: appearance.camera,
            view: appearance.view.clone(),
            spectrum: appearance.spectrum,
        };
        AppearanceDocument::copy_host_owned(
            (&mut look.camera, &mut look.view),
            (&Camera::default(), &ViewConfig::default()),
        );
        look
    }

    pub(crate) fn apply(&self, appearance: &mut AppearanceDocument) {
        let mut look = self.clone();
        AppearanceDocument::copy_host_owned(
            (&mut look.camera, &mut look.view),
            (&appearance.camera, &appearance.view),
        );
        appearance.camera = look.camera;
        appearance.view = look.view;
        appearance.spectrum = look.spectrum;
    }
}

const HISTORY_LIMIT: usize = 64;

/// The look's undo history: what undo and redo go back and forth through,
/// and the edit still open while a gesture is.
#[derive(Default)]
pub(crate) struct AppearanceEditor {
    undo: Vec<Look>,
    redo: Vec<Look>,
    pending: Option<Look>,
}

impl AppearanceEditor {
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

    /// Closes the edit still open as one undo entry: a drag that has ended,
    /// or one the window's close cut off.
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

    #[cfg(feature = "testing")]
    pub(crate) fn undo_depth(&self) -> usize {
        self.undo.len()
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
        self.observe(before, appearance, crate::gesture_open(ctx));
        // TextEdit leaves key events in the input even after handling them.
        // Keep its ownership for the whole frame: Undo followed by Enter can
        // undo text and release focus in one pass, but must not also undo a look.
        //
        // A press that shares its pass with the editor losing the keyboard is
        // taken all the same: it was the editor's when it was pressed, and the
        // claim the last pass made has already kept it from the host.
        if !text_edit_was_focused && !ctx.text_edit_focused() {
            let taken = crate::keys::take_each(ctx, APPEARANCE_SHORTCUTS);
            for shortcut in &taken {
                if *shortcut == APPEARANCE_SHORTCUTS[0] {
                    self.undo(appearance);
                } else {
                    self.redo(appearance);
                }
            }
            if !taken.is_empty() {
                ctx.request_repaint();
            }
        } else {
            // Read by the text edit, the editor's by the shell's text-field rule:
            // claimed as well for a frame that ends the edit, which the next one
            // undoes on.
            crate::keys::claim(ctx, APPEARANCE_SHORTCUTS);
        }
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
        editor.observe(original.clone(), &appearance, false);
        let changed = Look::capture(&appearance);
        for redo in &APPEARANCE_SHORTCUTS[1..] {
            for (shortcut, expected) in [(APPEARANCE_SHORTCUTS[0], &original), (*redo, &changed)] {
                let _ = crate::tests::probe::run(
                    &ctx,
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
            let _ = crate::tests::probe::run(
                &ctx,
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
        editor.observe(original, &appearance, false);
        let mut text = String::new();
        let mut time = 0.0;
        let mut frame = |events| {
            time += 1.0;
            let _ = crate::tests::probe::run(
                &ctx,
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
                    assert_eq!(editor.undo.len(), 1);
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
            let _ = crate::tests::probe::run(
                &ctx,
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
        assert_eq!(editor.undo.len(), 1);
        editor.undo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, original);
        editor.redo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, changed);
        editor.undo(&mut appearance);
        let before = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.observe(before, &appearance, false);
        assert!(editor.redo.is_empty());
    }

    /// Zoom the Analyzer with `wheel(frame)` events over 40 frames and assert
    /// the whole gesture, however many frames it changed the look on, is one
    /// undo entry. `focused` is the window's keyboard focus throughout.
    fn assert_one_undoable_zoom(
        navigation: crate::panes::spectral::Navigation,
        focused: bool,
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
                    source: egui::MouseWheelSource::Unknown,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput { screen_rect: Some(rect), events, focused, ..Default::default() },
                |ui| {
                    let before = Look::capture(&state.appearance);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    let shown = state.shown();
                    crate::panes::spectral::spectral_pane(
                        &mut child,
                        &mut state,
                        0.0,
                        1,
                        1.0,
                        navigation,
                        shown,
                        crate::panes::lattice::NO_SOLO,
                    );
                    zooming_frames += usize::from(before != Look::capture(&state.appearance));
                    editor.end_frame(before, &mut state.appearance, ui.ctx(), false);
                },
            );
        }
        assert!(zooming_frames >= 3, "the wheel must zoom over several frames: {zooming_frames}");
        assert_ne!(Look::capture(&state.appearance), original);
        assert_eq!(editor.undo.len(), 1);
        editor.undo(&mut state.appearance);
        assert_eq!(Look::capture(&state.appearance), original);
    }

    #[test]
    fn a_wheel_zoom_of_the_docked_analyzer_is_one_undoable_look_edit() {
        // Three wheel notches on consecutive frames, then the smoothed scroll
        // running out over the frames after them.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Docked, true, |frame| {
            (2..5).contains(&frame).then(|| egui::vec2(0.0, 20.0))
        });
    }

    #[test]
    fn a_trackpad_zoom_with_quiet_frames_between_events_is_one_undoable_look_edit() {
        // Point deltas under 8 pt are applied unsmoothed, so the frames
        // between two events read a zero scroll delta.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Docked, true, |frame| {
            ((2..12).contains(&frame) && frame % 2 == 0).then(|| egui::vec2(0.0, 6.0))
        });
    }

    #[test]
    fn a_wheel_zoom_over_a_window_without_the_keyboard_is_one_undoable_look_edit() {
        // Hovered but never clicked: the focus flag is false on every frame,
        // and no frame loses focus.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Docked, false, |frame| {
            (2..5).contains(&frame).then(|| egui::vec2(0.0, 20.0))
        });
    }

    #[test]
    fn a_wheel_zoom_of_the_video_previews_analyzer_is_one_undoable_look_edit() {
        // The preview takes the wheel's delta for itself, leaving the frame's
        // scroll delta zero while the wheel is still turning.
        assert_one_undoable_zoom(crate::panes::spectral::Navigation::Preview, true, |frame| {
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
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput { screen_rect: Some(rect), events, ..Default::default() },
                |ui| {
                    let before = Look::capture(&state.appearance);
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
                    let shown = state.shown();
                    crate::panes::spectral::spectral_pane(
                        &mut child,
                        &mut state,
                        0.0,
                        1,
                        1.0,
                        crate::panes::spectral::Navigation::Docked,
                        shown,
                        crate::panes::lattice::NO_SOLO,
                    );
                    moving_frames += usize::from(before != Look::capture(&state.appearance));
                    editor.end_frame(before, &mut state.appearance, ui.ctx(), false);
                },
            );
        }
        assert!(moving_frames >= 3, "the drag must pan over several frames: {moving_frames}");
        assert_eq!(editor.undo.len(), 1);
        editor.undo(&mut state.appearance);
        assert_eq!(Look::capture(&state.appearance), original);
    }

    /// A pinch in flight holds a look edit open, and the editor losing the
    /// keyboard in the same pass finishes it all the same.
    #[test]
    fn window_focus_loss_finishes_a_pending_action() {
        for (lost, finished) in [(true, true), (false, false)] {
            let ctx = crate::tests::probe::themed();
            let mut appearance = AppearanceDocument::default();
            let mut editor = AppearanceEditor::default();
            let before = Look::capture(&appearance);
            appearance.spectrum.attack = 0.2;
            editor.observe(before, &appearance, true);
            let mut events = vec![egui::Event::Zoom(1.1)];
            if lost {
                events.push(egui::Event::WindowFocused(false));
            }
            let _ = crate::tests::probe::run(
                &ctx,
                egui::RawInput { events, focused: !lost, ..Default::default() },
                |ui| {
                    editor.end_frame(Look::capture(&appearance), &mut appearance, ui.ctx(), false);
                },
            );
            assert_eq!(editor.pending.is_none(), finished, "focus lost: {lost}");
            assert_eq!(editor.undo.len(), usize::from(finished));
        }
    }

    #[test]
    fn project_restore_discards_history() {
        let mut state = crate::tests::probe::fresh();
        let original = Look::capture(&state.picture.appearance);
        state.picture.appearance.spectrum.attack = 0.2;
        let editor = &mut state.workspace.interaction.appearance_editor;
        editor.observe(original, &state.picture.appearance, false);
        assert_eq!(editor.undo.len(), 1, "the fixture holds an edit to discard");
        let blob = state.save_persist();
        assert!(state.install_document(&blob));
        assert!(state.workspace.interaction.appearance_editor.undo.is_empty());
        assert_eq!(state.picture.appearance.spectrum.attack, 0.2);
    }

    #[test]
    fn keyboard_undo_runs_after_the_body_commits_its_edit() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = appearance.spectrum.attack;
        let _ = crate::tests::probe::run(
            &ctx,
            egui::RawInput {
                events: vec![shortcut_event(APPEARANCE_SHORTCUTS[0])],
                ..Default::default()
            },
            |ui| {
                let before = Look::capture(&appearance);
                // ValueBar commits a typed value on losing focus, late in the frame.
                appearance.spectrum.attack = 0.2;
                editor.end_frame(before, &mut appearance, ui.ctx(), false);
            },
        );
        assert_eq!(appearance.spectrum.attack, original);
        editor.redo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, 0.2);
    }

    #[test]
    fn undo_preserves_incoming_camera_and_output() {
        let mut appearance = AppearanceDocument::default();
        let mut history = AppearanceEditor::default();
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
    /// A look leaves out exactly what the host's camera movement writes:
    /// capture cannot see anything `set_camera_movement` sets, and apply hands
    /// all of it back untouched, while every other camera field rides along.
    /// A field added to the host's movement and not to `copy_host_owned` makes
    /// the first assertion fail.
    #[test]
    fn a_look_resets_and_keeps_exactly_the_host_owned_fields() {
        let mut still = AppearanceDocument::default();
        still.camera.projection = harmonigraph_scene::Projection::Cabinet;
        still.camera.cabinet_scale = 0.6;
        still.spectrum.attack = 0.2;
        let mut moved = still.clone();
        moved.set_camera_movement([1.1, -0.3, 9.0, 12.25, -3.75]);
        assert_ne!(moved.camera_movement(), still.camera_movement());
        assert_eq!(Look::capture(&moved), Look::capture(&still), "capture saw the host camera");

        let mut other = still.clone();
        other.camera.cabinet_scale = 0.3;
        assert_ne!(Look::capture(&other), Look::capture(&still), "the rest of the camera rides");
        let movement = moved.camera_movement();
        let (target, centers) =
            (moved.camera.target, (moved.view.center_fives, moved.view.center_threes));
        Look::capture(&other).apply(&mut moved);
        assert_eq!(moved.camera_movement(), movement);
        assert_eq!(moved.camera.target, target);
        assert_eq!((moved.view.center_fives, moved.view.center_threes), centers);
        assert_eq!(Look::capture(&moved), Look::capture(&other));
    }
}
