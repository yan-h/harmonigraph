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

#[derive(Default)]
pub(crate) struct AppearanceEditor {
    history: History,
    /// Commands performed in the settings toolbar already manage history.
    commanded: bool,
}

impl AppearanceEditor {
    pub(crate) fn begin_frame(&mut self) {
        self.commanded = false;
    }

    pub(crate) fn end_frame(
        &mut self,
        before: Look,
        appearance: &AppearanceDocument,
        ctx: &egui::Context,
    ) {
        if !self.commanded {
            self.history.observe(
                before,
                appearance,
                crate::kept_focus(ctx)
                    && ctx.dragged_id().is_some()
                    && ctx.input(|i| i.pointer.primary_down()),
            );
        }
    }

    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui, appearance: &mut AppearanceDocument) {
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.history.undo.is_empty() || self.history.pending.is_some(), egui::Button::new("Undo")).clicked() {
                self.history.undo(appearance);
                self.commanded = true;
            }
            if ui.add_enabled(!self.history.redo.is_empty(), egui::Button::new("Redo")).clicked() {
                self.history.redo(appearance);
                self.commanded = true;
            }
        }).response.on_hover_text("Undo/Redo affects appearance. Keyboard undo, camera movement, tuning and automatable controls use the host's undo. Output and editor layout are separate.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slider_drag_is_one_action_and_redo_is_invalidated_by_an_edit() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let original = appearance.spectrum.tilt;
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
                    editor.begin_frame();
                    let before = Look::capture(&appearance);
                    rect = crate::widgets::ValueBar::new(
                        &mut appearance.spectrum.tilt,
                        -1.5..=6.0,
                        "Tilt",
                    )
                    .show(ui)
                    .rect;
                    editor.end_frame(before, &appearance, ui.ctx());
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
        assert_ne!(appearance.spectrum.tilt, original, "fixture must actually drag the slider");
        let changed = appearance.spectrum.tilt;
        assert_eq!(editor.history.undo.len(), 1);
        editor.history.undo(&mut appearance);
        assert_eq!(appearance.spectrum.tilt, original);
        editor.history.redo(&mut appearance);
        assert_eq!(appearance.spectrum.tilt, changed);
        editor.history.undo(&mut appearance);
        let before = Look::capture(&appearance);
        appearance.spectrum.tilt = 2.0;
        editor.history.observe(before, &appearance, false);
        assert!(editor.history.redo.is_empty());
    }

    #[test]
    fn window_focus_loss_finishes_a_pending_action() {
        let ctx = crate::tests::probe::themed();
        let mut appearance = AppearanceDocument::default();
        let mut editor = AppearanceEditor::default();
        let before = Look::capture(&appearance);
        appearance.spectrum.tilt = 2.0;
        editor.history.observe(before, &appearance, true);
        let _ = ctx.run_ui(
            egui::RawInput {
                events: vec![egui::Event::WindowFocused(false)],
                ..Default::default()
            },
            |ui| {
                assert!(ui.input(|i| i.focused), "fixture matches baseview's unchanged raw flag");
                editor.end_frame(Look::capture(&appearance), &appearance, ui.ctx());
            },
        );
        assert!(editor.history.pending.is_none());
        assert_eq!(editor.history.undo.len(), 1);
    }

    #[test]
    fn undo_preserves_incoming_camera_tuning_output_and_editor_state() {
        let mut appearance = AppearanceDocument::default();
        let mut history = History::default();
        let before = Look::capture(&appearance);
        let original = appearance.spectrum.tilt;
        appearance.spectrum.tilt = 2.0;
        history.observe(before, &appearance, true);
        appearance.camera.distance = 7.0;
        appearance.view.center_fives = 12;
        appearance.view.meantone = true;
        appearance.view.frameless = true;
        appearance.render.short_edge = 1920;
        history.finish(&appearance);
        history.undo(&mut appearance);
        assert_eq!(appearance.spectrum.tilt, original);
        assert_eq!(appearance.camera.distance, 7.0);
        assert_eq!(appearance.view.center_fives, 12);
        assert!(appearance.view.meantone && appearance.view.frameless);
        assert_eq!(appearance.render.short_edge, 1920);
    }
}
