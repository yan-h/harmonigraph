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
    // Do not compile a hidden lattice just because the editor was opened.
    if !needs_lattice(state) {
        ui.ctx().data_mut(|data| data.remove::<Loading>(id()));
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
            state.lattice_pipelines.clone(),
            state.target_format,
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
}
