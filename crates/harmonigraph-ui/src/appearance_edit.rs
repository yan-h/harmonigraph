//! Editor appearance history. The live document remains PictureState's;
//! snapshots deliberately exclude host state, navigation and output settings.

use crate::{AppearanceDocument, SpectrumConfig};
use harmonigraph_scene::{Camera, ViewConfig};

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
        to.meantone = from.meantone;
        to.meantone_auto = from.meantone_auto;
        to.marvel = from.marvel;
        to.marvel_auto = from.marvel_auto;
        to.frameless = from.frameless;
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

/// Project-local library and the inactive comparison snapshot. The active
/// slot is always the live document; there is no second writable copy.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct SavedLooks {
    pub(crate) named: std::collections::BTreeMap<String, Look>,
    inactive: Option<Look>,
    active_b: bool,
}

impl SavedLooks {
    pub(crate) fn sanitize(&mut self) {
        for look in self.named.values_mut().chain(self.inactive.iter_mut()) {
            look.camera.sanitize();
            look.view.sanitize();
            look.spectrum.sanitize();
        }
    }
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
    Undo,
    Redo,
    Switch,
    Recall(String),
    Save,
}

#[derive(Default)]
pub(crate) struct AppearanceEditor {
    history: History,
    inactive_history: History,
    pub(crate) saved: SavedLooks,
    name: String,
    selected: Option<String>,
    // Execute after the body commits any text field losing focus this frame.
    command: Option<Command>,
}

impl AppearanceEditor {
    fn switch(&mut self, appearance: &mut AppearanceDocument) {
        self.history.finish(appearance);
        let active = Look::capture(appearance);
        let next = self.saved.inactive.replace(active.clone()).unwrap_or(active);
        next.apply(appearance);
        std::mem::swap(&mut self.history, &mut self.inactive_history);
        self.saved.active_b = !self.saved.active_b;
    }

    fn recall(&mut self, name: &str, appearance: &mut AppearanceDocument) {
        self.history.finish(appearance);
        if let Some(look) = self.saved.named.get(name) {
            let before = Look::capture(appearance);
            look.apply(appearance);
            self.history.push(before, &Look::capture(appearance));
        }
    }

    fn save(&mut self, appearance: &AppearanceDocument) {
        let name = self.name.trim();
        if !name.is_empty() && !self.saved.named.contains_key(name) {
            self.saved.named.insert(name.to_owned(), Look::capture(appearance));
            self.selected = Some(name.to_owned());
            self.name.clear();
        }
    }

    pub(crate) fn restore(saved: SavedLooks) -> Self {
        Self { saved, ..Default::default() }
    }

    pub(crate) fn end_frame(
        &mut self,
        before: Look,
        appearance: &mut AppearanceDocument,
        ctx: &egui::Context,
    ) {
        self.history.observe(
            before,
            appearance,
            crate::kept_focus(ctx)
                && ctx.dragged_id().is_some()
                && ctx.input(|i| i.pointer.primary_down()),
        );
        if let Some(command) = self.command.take() {
            self.history.finish(appearance);
            match command {
                Command::Undo => self.history.undo(appearance),
                Command::Redo => self.history.redo(appearance),
                Command::Switch => self.switch(appearance),
                Command::Recall(name) => self.recall(&name, appearance),
                Command::Save => self.save(appearance),
            }
            ctx.request_repaint();
        }
    }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(!self.history.undo.is_empty() || self.history.pending.is_some(), egui::Button::new("Undo")).clicked() {
                self.command = Some(Command::Undo);
                    }
            if ui.add_enabled(!self.history.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                self.command = Some(Command::Redo);
                    }
            for (is_b, label) in [(false, "A"), (true, "B")] {
                if ui.selectable_label(self.saved.active_b == is_b, label).clicked() && self.saved.active_b != is_b {
                    self.command = Some(Command::Switch);
                }
            }
        }).response.on_hover_text("Undo/Redo affects appearance. Keyboard undo, camera movement, tuning and automatable controls use the host's undo. Output and editor layout are separate.");
        egui::CollapsingHeader::new("Looks").show(ui, |ui| {
            ui.label("Looks keep appearance; camera movement, tuning and output stay as they are.");
            egui::ComboBox::from_id_salt("saved-look")
                .selected_text(self.selected.as_deref().unwrap_or("Choose look"))
                .width(ui.available_width().max(1.0))
                .truncate()
                .show_ui(ui, |ui| {
                    for name in self.saved.named.keys() {
                        ui.selectable_value(&mut self.selected, Some(name.clone()), name);
                    }
                });
            crate::widgets::button_row(ui, |ui| {
                let selected =
                    self.selected.clone().filter(|name| self.saved.named.contains_key(name));
                if ui.add_enabled(selected.is_some(), egui::Button::new("Recall")).clicked() {
                    self.command = Some(Command::Recall(selected.clone().unwrap()));
                }
                if ui.add_enabled(selected.is_some(), egui::Button::new("Delete")).clicked() {
                    self.saved.named.remove(selected.as_deref().unwrap());
                    self.selected = None;
                }
            });
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .hint_text("New look name")
                    .desired_width(ui.available_width()),
            );
            let name = self.name.trim();
            let valid = !name.is_empty() && !self.saved.named.contains_key(name);
            if ui.add_enabled(valid, egui::Button::new("Save current look")).clicked() {
                self.command = Some(Command::Save);
            }
            if !self.name.trim().is_empty() && self.saved.named.contains_key(self.name.trim()) {
                ui.label("That name is already saved. Choose another name or delete it first.");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                    editor.end_frame(before, &mut appearance, ui.ctx());
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
                editor.end_frame(Look::capture(&appearance), &mut appearance, ui.ctx());
            },
        );
        assert!(editor.history.pending.is_none());
        assert_eq!(editor.history.undo.len(), 1);
    }

    #[test]
    fn comparison_histories_and_named_recall_share_the_live_document() {
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = Look::capture(&appearance);
        editor.switch(&mut appearance); // B starts as a snapshot of A.
        let before = Look::capture(&appearance);
        appearance.spectrum.attack = 0.2;
        editor.history.observe(before, &appearance, false);
        editor.name = "Bright".into();
        editor.save(&appearance);
        let bright = Look::capture(&appearance);
        editor.switch(&mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        assert!(editor.history.undo.is_empty());
        editor.recall("Bright", &mut appearance);
        assert_eq!(Look::capture(&appearance), bright);
        editor.history.undo(&mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        editor.switch(&mut appearance);
        assert_eq!(Look::capture(&appearance), bright);
        editor.history.undo(&mut appearance);
        assert_eq!(Look::capture(&appearance), original);
        editor.history.redo(&mut appearance);
        assert_eq!(Look::capture(&appearance), bright);
    }

    #[test]
    fn project_restore_retains_looks_and_comparison_but_discards_history() {
        let mut state = crate::tests::probe::fresh();
        let original = Look::capture(&state.picture.appearance);
        let editor = &mut state.workspace.interaction.appearance_editor;
        editor.switch(&mut state.picture.appearance);
        state.picture.appearance.spectrum.attack = 0.2;
        editor.history.observe(original.clone(), &state.picture.appearance, false);
        editor.name = "Saved B".into();
        editor.save(&state.picture.appearance);
        let blob = state.save_persist();
        assert!(state.load_persist(&blob));
        let editor = &mut state.workspace.interaction.appearance_editor;
        assert!(editor.saved.active_b);
        assert_eq!(editor.saved.named["Saved B"].spectrum.attack, 0.2);
        assert!(editor.history.undo.is_empty());
        editor.switch(&mut state.picture.appearance);
        assert_eq!(Look::capture(&state.picture.appearance), original);
    }

    #[test]
    fn toolbar_command_runs_after_the_body_commits_its_edit() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let original = appearance.spectrum.attack;
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let before = Look::capture(&appearance);
            editor.command = Some(Command::Undo);
            // ValueBar commits a typed value on losing focus, after toolbar drawing.
            appearance.spectrum.attack = 0.2;
            editor.end_frame(before, &mut appearance, ui.ctx());
        });
        assert_eq!(appearance.spectrum.attack, original);
        editor.history.redo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, 0.2);
    }

    #[test]
    fn undo_preserves_incoming_camera_tuning_output_and_editor_state() {
        let mut appearance = AppearanceDocument::default();
        let mut history = History::default();
        let before = Look::capture(&appearance);
        let original = appearance.spectrum.attack;
        appearance.spectrum.attack = 0.2;
        history.observe(before, &appearance, true);
        appearance.camera.distance = 7.0;
        appearance.view.center_fives = 12;
        let tuning = (
            appearance.view.meantone,
            appearance.view.meantone_auto,
            appearance.view.marvel,
            appearance.view.marvel_auto,
        );
        appearance.view.meantone = !tuning.0;
        appearance.view.meantone_auto = !tuning.1;
        appearance.view.marvel = !tuning.2;
        appearance.view.marvel_auto = !tuning.3;
        appearance.view.frameless = true;
        appearance.render.short_edge = 1920;
        history.finish(&appearance);
        history.undo(&mut appearance);
        assert_eq!(appearance.spectrum.attack, original);
        assert_eq!(appearance.camera.distance, 7.0);
        assert_eq!(appearance.view.center_fives, 12);
        assert_eq!(
            (
                appearance.view.meantone,
                appearance.view.meantone_auto,
                appearance.view.marvel,
                appearance.view.marvel_auto
            ),
            (!tuning.0, !tuning.1, !tuning.2, !tuning.3)
        );
        assert!(appearance.view.frameless);
        assert_eq!(appearance.render.short_edge, 1920);
    }
}
