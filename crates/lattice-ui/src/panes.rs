//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function; it immediately participates in docking, and gets the
//! shared state (hover, console, tracker) for free.

use egui::Sense;
use lattice_core::tuning;
use lattice_render::lattice_paint_callback;
use lattice_scene::{derive_scene, OctaveStyle};

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::ValueBar;
use crate::SharedState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Lattice,
    Tuning,
    Console,
    Spectral,
}

pub struct Viewer<'a> {
    pub state: &'a mut SharedState,
    pub params: &'a dyn ParamBackend,
    pub now: f64,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        match tab {
            Tab::Lattice => "Lattice".into(),
            Tab::Tuning => "Tuning".into(),
            Tab::Console => "Console".into(),
            Tab::Spectral => "Spectral".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Lattice => lattice_pane(ui, self.state, self.now),
            Tab::Tuning => tuning_pane(ui, self.state, self.params),
            Tab::Console => console_pane(ui, self.state),
            Tab::Spectral => spectral_pane(ui),
        }
    }
}

/// The 3D lattice view: orbit camera on drag, zoom on scroll, pick on hover.
fn lattice_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }

    // Camera input.
    if response.dragged() {
        let delta = response.drag_delta();
        state.camera.yaw -= delta.x * 0.01;
        state.camera.pitch = (state.camera.pitch + delta.y * 0.01)
            .clamp(-1.5, 1.5);
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            state.camera.distance = (state.camera.distance * (1.0 - scroll * 0.002))
                .clamp(2.0, 80.0);
        }
    }
    if response.double_clicked() {
        state.camera = Default::default();
    }

    let scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        state.camera,
        state.hovered,
        now,
    );

    // Picking updates the *shared* hover state, one frame behind the scene
    // it was derived from (imperceptible, standard for immediate mode).
    if let Some(pointer) = response.hover_pos() {
        state.hovered = scene.pick(
            glam::Vec2::new(rect.width(), rect.height()),
            glam::Vec2::new(pointer.x - rect.min.x, pointer.y - rect.min.y),
            24.0,
        );
    } else if !response.dragged() {
        state.hovered = None;
    }

    ui.painter()
        .add(lattice_paint_callback(rect, &scene, state.target_format, 0));

    // Hover tooltip: pitch class + sounding octaves.
    if let Some(pos) = state.hovered {
        let pc = state.tuning.pitch_class(pos);
        let octaves: Vec<String> = state
            .tracker
            .voices()
            .filter(|v| state.tuning.matches(v.pitch_class, pc))
            .map(|v| v.octave.to_string())
            .collect();
        let mut text = format!(
            "({}, {}, {})  {}",
            pos.threes, pos.fives, pos.sevens, pc
        );
        if !octaves.is_empty() {
            text.push_str(&format!("  octaves: {}", octaves.join(" ")));
        }
        response.on_hover_ui(|ui| {
            ui.label(text);
        });
    }
}

fn tuning_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    ui.heading("Tuning");

    for key in ParamKey::ALL {
        let mut value = params.get(key);
        let bar = ValueBar::new(&mut value, key.range(), key.label())
            .eased(key.logarithmic())
            .decimals(2);
        if bar.show(ui).changed() {
            params.set(key, value);
        }
    }

    ui.horizontal(|ui| {
        if ui.button("Just").clicked() {
            params.set(ParamKey::Three, tuning::THREE_JUST);
            params.set(ParamKey::Five, tuning::FIVE_JUST);
            params.set(ParamKey::Seven, tuning::SEVEN_JUST);
        }
        if ui.button("12-TET").clicked() {
            params.set(ParamKey::Three, tuning::THREE_12TET);
            params.set(ParamKey::Five, tuning::FIVE_12TET);
            params.set(ParamKey::Seven, tuning::SEVEN_12TET);
        }
    });

    ui.separator();
    ui.heading("View");
    for (extent, range, label) in [
        (&mut state.view.extent_threes, 1.0..=8.0, "Fifths extent"),
        (&mut state.view.extent_fives, 1.0..=8.0, "Thirds extent"),
        (&mut state.view.extent_sevens, 0.0..=4.0, "Sevenths extent"),
    ] {
        let mut value = *extent as f32;
        if ValueBar::new(&mut value, range, label).integer().show(ui).changed() {
            *extent = value as i32;
        }
    }

    // Octave indicator style: kept as switchable design candidates so they
    // can be compared live while notes play.
    ui.horizontal(|ui| {
        ui.label("Octaves");
        for (style, label) in [
            (OctaveStyle::Dots, "Dots"),
            (OctaveStyle::Rings, "Rings"),
            (OctaveStyle::Ticks, "Ticks"),
            (OctaveStyle::Off, "Off"),
        ] {
            ui.selectable_value(&mut state.view.octave_style, style, label);
        }
    });

    ui.separator();
    // Cross-pane highlight demo: this pane reacts to the lattice hover.
    match state.hovered {
        Some(pos) => {
            let pc = state.tuning.pitch_class(pos);
            ui.label(format!(
                "Hovered: ({}, {}, {}) = {}",
                pos.threes, pos.fives, pos.sevens, pc
            ));
        }
        None => {
            ui.weak("Hover a node to inspect it");
        }
    }
}

fn console_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    ui.horizontal(|ui| {
        ui.label(format!("{} held", state.tracker.held_count()));
        if ui.button("Clear").clicked() {
            state.console.clear();
        }
    });
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for line in state.console.lines() {
                ui.monospace(line);
            }
        });
}

fn spectral_pane(ui: &mut egui::Ui) {
    ui.weak("Spectral view: not yet implemented.");
    // TODO: needs audio (or per-voice frequency) data plumbed from the
    // shell; the pane pattern is the same as the lattice pane.
}
