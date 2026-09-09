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
    egui::Id::new("harmonigraph-editor-startup")
}

/// Opt this window into stage progress. Call after restoring its theme/state;
/// offline rendering does not call this and keeps its synchronous first frame.
pub fn begin_editor_loading(ctx: &egui::Context) {
    ctx.data_mut(|data| data.insert_temp(id(), Loading::default()));
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

fn needs_lattice(state: &SharedState) -> bool {
    state.workspace.dock.iter_all_nodes().any(|(path, node)| {
        let egui_dock::Node::Leaf(leaf) = node else { return false };
        matches!(
            leaf.tabs.get(leaf.active.0),
            Some(crate::panes::Tab::Lattice | crate::panes::Tab::Video)
        ) && !std::iter::successors(Some(path.node), |node| node.parent())
            .any(|node| state.workspace.dock[path.surface][node].is_collapsed())
    })
}

pub(crate) fn draw(ui: &mut egui::Ui, state: &SharedState) -> bool {
    let Some(mut loading) = ui.ctx().data(|data| data.get_temp::<Loading>(id())) else {
        return false;
    };
    // Stay dormant until a pane needs the lattice. Removing the opt-in here
    // would make its first later reveal compile synchronously on the UI thread.
    if !needs_lattice(state) {
        return false;
    }
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
        egui::FontId::proportional(14.0),
        theme::text_dim(),
    );
    if matches!(status, Status::Preparing(_)) {
        ui.painter().add(loading.progress.callback(
            rect,
            state.picture.surfaces.lattice_pipelines.clone(),
            state.picture.surfaces.target_format,
        ));
    }
    if status != Status::Failed {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(16));
    }
    ui.ctx().data_mut(|data| data.insert_temp(id(), loading));
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::Tab;

    struct Defaults;
    impl crate::params::ParamBackend for Defaults {
        fn get(&self, key: crate::params::ParamKey) -> f32 {
            key.default_value()
        }
        fn set(&self, _: crate::params::ParamKey, _: f32) {}
    }

    #[test]
    fn loading_frames_age_released_notes_without_pruning_live_voices() {
        if matches!(WindowStartup::default().status(), Status::Ready { .. }) {
            eprintln!("hot reload uses synchronous initialization; no loading frames");
            return;
        }
        let ctx = egui::Context::default();
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.workspace.dock = egui_dock::DockState::new(vec![Tab::Lattice]);
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
        begin_editor_loading(&ctx);
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::root_ui(ui, &mut state, &Defaults, 5.0);
        });
        assert!(matches!(editor_loading_status(&ctx), Some(Status::Preparing(_))));
        let remaining: Vec<_> =
            state.picture.runtime.tracker.voices().map(|voice| voice.note).collect();
        assert_eq!(remaining, vec![67, 64], "only the expired release should be pruned");
    }

    #[test]
    fn hidden_lattice_tabs_do_not_trigger_graphics_preparation() {
        let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
        state.workspace.dock = egui_dock::DockState::new(vec![Tab::Spectral, Tab::Lattice]);
        assert!(!needs_lattice(&state), "an inactive tab must not trigger a compile");
        let root = egui_dock::NodeIndex::root();
        let egui_dock::Node::Leaf(leaf) = &mut state.workspace.dock.main_surface_mut()[root] else {
            unreachable!()
        };
        leaf.active = egui_dock::TabIndex(1);
        assert!(needs_lattice(&state));
        state.workspace.dock.main_surface_mut()[root].set_collapsed(true);
        assert!(!needs_lattice(&state), "a folded lattice must stay cheap to open");
        state.workspace.dock = egui_dock::DockState::new(vec![Tab::Video]);
        assert!(needs_lattice(&state), "the video preview also draws the lattice");
    }

    #[test]
    fn first_reveal_after_hidden_frames_uses_the_loading_path() {
        if matches!(WindowStartup::default().status(), Status::Ready { .. }) {
            eprintln!("hot reload uses synchronous initialization; no loading frames");
            return;
        }
        for tab in [Tab::Lattice, Tab::Video] {
            let ctx = egui::Context::default();
            let mut state = SharedState::new(harmonigraph_render::wgpu::TextureFormat::Bgra8Unorm);
            state.workspace.dock = egui_dock::DockState::new(vec![Tab::Spectral, tab]);
            begin_editor_loading(&ctx);
            let loading_text = |output: &egui::FullOutput| {
                output.shapes.iter().any(|shape| {
                matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == Stage::Graphics.label())
            })
            };
            for _ in 0..3 {
                let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                    crate::root_ui(ui, &mut state, &Defaults, 0.0);
                });
                assert!(!loading_text(&output), "hidden panes do not request preparation");
            }
            assert!(editor_loading_status(&ctx).is_some(), "first-use opt-in survives");
            let path = state.workspace.dock.find_tab(&tab).unwrap();
            state.workspace.dock.set_active_tab(path).unwrap();
            let output = ctx.run_ui(egui::RawInput::default(), |ui| {
                crate::root_ui(ui, &mut state, &Defaults, 0.0);
            });
            assert!(loading_text(&output), "first reveal enters asynchronous preparation");
            assert!(output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_))));
            assert_eq!(editor_loading_status(&ctx), Some(Status::Preparing(Stage::Graphics)));
        }
    }
}
