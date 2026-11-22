//! An editor-only loading screen, drawn by egui while the lattice compiles.
//! Its state belongs to this egui context and is never persisted or exported.

use harmonigraph_render::startup::{Stage, Status, WindowStartup};

use crate::{theme, SharedState};

#[derive(Clone, Default)]
struct Loading {
    progress: WindowStartup,
    finishing: bool,
}

fn id() -> egui::Id {
    egui::Id::unique("harmonigraph-editor-startup")
}

/// Opt this window into stage progress. Call after restoring its theme/state;
/// offline rendering does not call this and keeps its synchronous first frame.
pub fn begin_editor_loading(ctx: &egui::Context) {
    begin_editor_loading_with(ctx, WindowStartup::default());
}

/// [`begin_editor_loading`] from `progress`, which a test hands
/// [`WindowStartup::preparing`] to reach the loading screen in a build whose
/// default progress starts at `Ready`.
pub(crate) fn begin_editor_loading_with(ctx: &egui::Context, progress: WindowStartup) {
    ctx.data_mut(|data| data.insert_temp(id(), Loading { progress, finishing: false }));
}

/// Current opening phase for native-shell diagnostics; `None` after handoff.
pub fn editor_loading_status(ctx: &egui::Context) -> Option<Status> {
    ctx.data(|data| data.get_temp::<Loading>(id())).map(|loading| {
        if loading.finishing {
            Status::Preparing(Stage::Interface)
        } else {
            loading.progress.status()
        }
    })
}

pub(crate) fn draw(ui: &mut egui::Ui, state: &SharedState) -> bool {
    let Some(mut loading) = ui.ctx().data(|data| data.get_temp::<Loading>(id())) else {
        return false;
    };
    // Every open, lattice or not: until the job is ready, each other pane's
    // first frame would build its own programs on the host's main thread
    // (#1743), and a lattice revealed later would compile there too.
    let status = loading.progress.status();
    let stage = match status {
        Status::Ready { built } if !built || loading.finishing => {
            ui.ctx().data_mut(|data| data.remove::<Loading>(id()));
            return false;
        }
        Status::Ready { .. } => {
            // Present this label before the first normal frame prepares its
            // window-specific text, atlas and pane resources.
            loading.finishing = true;
            Stage::Interface
        }
        Status::Preparing(stage) => stage,
        Status::Failed => Stage::Graphics,
    };

    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, 0.0, theme::panel());
    let center = rect.center();
    ui.painter().text(
        center - egui::vec2(0.0, 24.0),
        egui::Align2::CENTER_CENTER,
        "Harmonigraph",
        egui::FontId::proportional(22.0),
        theme::text(),
    );
    let message = if status == Status::Failed {
        "Graphics initialization failed.\nReload the plugin to try again."
    } else {
        stage.label()
    };
    ui.painter().text(
        center + egui::vec2(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        message,
        egui::FontId::proportional(15.0),
        theme::text(),
    );
    if matches!(status, Status::Preparing(_)) {
        ui.painter().add(loading.progress.callback(
            rect,
            state.picture.surfaces.lattice_pipelines.clone(),
            state.picture.surfaces.target_format,
        ));
    }
    if status != Status::Failed {
        // About every 60 Hz refresh.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    ui.ctx().data_mut(|data| data.insert_temp(id(), loading));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::probe::Defaults;
    use crate::{panes::Tab, workspace};

    #[test]
    fn loading_frames_age_released_notes_without_pruning_live_voices() {
        let ctx = egui::Context::default();
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.workspace.layout = workspace::Layout::solo(Tab::Lattice);
        use harmonigraph_core::{NoteEvent, SourceId};
        state.picture.runtime.tracker.handle_event(NoteEvent::on(
            0.0,
            SourceId::DIRECT,
            0,
            60,
            1.0,
        ));
        state.picture.runtime.tracker.handle_event(NoteEvent::off(1.0, SourceId::DIRECT, 0, 60));
        state.picture.runtime.tracker.handle_event(NoteEvent::on(
            4.0,
            SourceId::DIRECT,
            0,
            64,
            1.0,
        ));
        state.picture.runtime.tracker.handle_event(NoteEvent::off(4.99, SourceId::DIRECT, 0, 64));
        state.picture.runtime.tracker.handle_event(NoteEvent::on(
            4.0,
            SourceId::DIRECT,
            0,
            67,
            1.0,
        ));
        assert_eq!(state.picture.runtime.tracker.voices().count(), 3);
        begin_editor_loading_with(&ctx, WindowStartup::preparing());
        let _ = crate::tests::probe::run(&ctx, egui::RawInput::default(), |ui| {
            crate::root_ui(ui, &mut state, &Defaults, 5.0, "test @0123456");
        });
        assert!(matches!(editor_loading_status(&ctx), Some(Status::Preparing(_))));
        let remaining: Vec<_> =
            state.picture.runtime.tracker.voices().map(|voice| voice.note).collect();
        assert_eq!(remaining, vec![67, 64], "only the expired release should be pruned");
    }

    /// An open with no lattice in sight still loads, so every pane's programs
    /// come from the worker rather than from its own first frame on the
    /// host's main thread (#1743): a Spectral-only layout, and one whose
    /// lattice section is folded.
    #[test]
    fn every_open_uses_the_loading_path() {
        let spectral = workspace::Layout::solo(Tab::Spectral);
        let mut folded = workspace::Layout::solo(Tab::Lattice);
        folded.folded[workspace::Section::of(Tab::Lattice) as usize] = true;
        for layout in [spectral, folded] {
            let ctx = egui::Context::default();
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.workspace.layout = layout;
            assert!(!state.workspace.layout.visible(Tab::Lattice), "the lattice shows");
            begin_editor_loading_with(&ctx, WindowStartup::preparing());
            let mut frame = || {
                crate::tests::probe::run(&ctx, egui::RawInput::default(), |ui| {
                    crate::root_ui(ui, &mut state, &Defaults, 0.0, "test @0123456");
                })
            };
            let loading = |output: &egui::FullOutput| {
                output.shapes.iter().any(|shape| {
                    matches!(
                        &shape.shape,
                        egui::Shape::Text(text) if text.galley.text() == Stage::Graphics.label()
                    )
                })
            };
            assert!(loading(&frame()), "the open skipped the loading screen");
            assert!(loading(&frame()), "the second frame left the loading screen");
            // The splash's own pace, the whole 16 ms rather than every
            // refresh. From the third frame, since a fresh context asks to
            // repaint at once, and egui's first sync of its theme to the
            // window asks again.
            let output = frame();
            assert!(loading(&output), "the third frame left the loading screen");
            let delay = output.viewport_output.values().map(|v| v.repaint_delay).min().unwrap();
            assert_eq!(delay, std::time::Duration::from_millis(16), "the splash's poll");
            assert!(output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))));
            assert_eq!(editor_loading_status(&ctx), Some(Status::Preparing(Stage::Graphics)));
        }
    }
}
