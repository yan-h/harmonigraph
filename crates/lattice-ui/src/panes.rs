//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function; it immediately participates in docking, and gets the
//! shared state (hover, console, tracker) for free.

use egui::Sense;
use lattice_core::tuning;
use lattice_render::lattice_paint_callback;
use lattice_core::coords;
use lattice_scene::{channel_color, derive_scene, OctaveStyle};

use crate::theme;

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::ValueBar;
use crate::SharedState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
            Tab::Spectral => spectral_pane(ui, self.state, self.now),
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

    // Note-name labels on hovered and sounding nodes, drawn as egui text
    // over the 3D view (projected with the same camera as the nodes).
    if state.view.show_labels {
        let viewport = glam::Vec2::new(rect.width(), rect.height());
        for node in &scene.nodes {
            if !(node.hovered || node.activation > 0.0) {
                continue;
            }
            let Some(p) = scene.project(viewport, node.world_pos) else {
                continue;
            };
            // Fade with the activation envelope; hovered idle nodes get a
            // dim but readable label.
            let strength = if node.hovered {
                1.0
            } else {
                0.35 + 0.65 * node.activation
            };
            let color = theme::text().gamma_multiply(strength);
            ui.painter().text(
                egui::pos2(rect.min.x + p.x, rect.min.y + p.y + 14.0),
                egui::Align2::CENTER_TOP,
                node.lattice_pos.note_name().to_string(),
                egui::TextStyle::Small.resolve(ui.style()),
                color,
            );
        }
    }

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
            "{}  ({}, {}, {})  {}",
            pos.note_name(),
            pos.threes,
            pos.fives,
            pos.sevens,
            pc
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
        let response = bar.show(ui);
        // Bracket drags so the host records one automation gesture per
        // drag; one-shot changes (typed values) go through set() alone.
        if response.drag_started() {
            params.begin_set(key);
        }
        if response.changed() {
            params.set(key, value);
        }
        if response.drag_stopped() {
            params.end_set(key);
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
        // v1's tuning-learn: infer C offset and prime tunings from the
        // currently held pitch classes (hold a justly tuned chord, click).
        let learn = ui
            .button("Learn")
            .on_hover_text("Set tuning from the currently held notes");
        if learn.clicked() {
            let classes: Vec<_> = state
                .tracker
                .voices()
                .filter(|v| v.state == lattice_core::VoiceState::Held)
                .map(|v| v.pitch_class)
                .collect();
            let learned = tuning::learn_tuning(&classes);
            for (value, key) in [
                (learned.c_offset, ParamKey::COffset),
                (learned.three, ParamKey::Three),
                (learned.five, ParamKey::Five),
                (learned.seven, ParamKey::Seven),
            ] {
                if let Some(value) = value {
                    params.set(key, value);
                }
            }
            state.console.log(format!(
                "learn: {} held classes -> {:?}",
                classes.len(),
                learned
            ));
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
    // can be compared live while notes play. The Ticks variants differ in
    // the reference frame showing where the octave range starts/ends.
    ui.checkbox(&mut state.view.show_labels, "Note labels");

    ui.horizontal(|ui| {
        ui.label("Octaves");
        for (style, label) in [
            (OctaveStyle::Off, "Off"),
            (OctaveStyle::Dots, "Dots"),
            (OctaveStyle::Rings, "Rings"),
        ] {
            ui.selectable_value(&mut state.view.octave_style, style, label);
        }
    });
    ui.horizontal(|ui| {
        ui.label("Ticks");
        for (style, label) in [
            (OctaveStyle::TicksRail, "Rail"),
            (OctaveStyle::TicksLadder, "Ladder"),
            (OctaveStyle::TicksCaps, "Caps"),
            (OctaveStyle::TicksMid, "Mid-C mark"),
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

/// Pitch-class meter: sounding voices as bars on a 0-1200 cents axis.
/// MIDI-derived (velocity/activation weighted), not an audio FFT — that
/// upgrade needs audio analysis plumbed from the audio thread.
///
/// Hover sync goes both ways: the lattice-hovered pitch class shows as a
/// band here, and hovering a position here highlights the matching lattice
/// node (if one is in view).
fn spectral_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::well());

    let x_of = |cents: f32| rect.left() + rect.width() * (cents / 1200.0);

    // 12-TET reference grid.
    for i in 0..12 {
        let x = x_of(i as f32 * 100.0);
        painter.line_segment(
            [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
            egui::Stroke::new(1.0, theme::panel()),
        );
        painter.text(
            egui::pos2(x + 3.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("{}", i * 100),
            egui::TextStyle::Small.resolve(ui.style()),
            theme::text_dim(),
        );
    }

    // Where the visible lattice nodes sit: small ticks along the bottom.
    for pos in coords::positions_within(
        -state.view.extent_threes..=state.view.extent_threes,
        -state.view.extent_fives..=state.view.extent_fives,
        -state.view.extent_sevens..=state.view.extent_sevens,
    ) {
        let x = x_of(state.tuning.pitch_class(pos).to_cents());
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom() - 10.0),
                egui::pos2(x, rect.bottom() - 4.0),
            ],
            egui::Stroke::new(1.0, theme::text_dim()),
        );
    }

    // Cross-pane highlight: the pitch class hovered in ANY pane shows as a
    // tolerance-wide band.
    if let Some(pos) = state.hovered {
        let pc = state.tuning.pitch_class(pos);
        let half_width =
            (rect.width() * (state.tuning.tolerance / 1200.0)).max(1.5);
        let x = x_of(pc.to_cents());
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x - half_width, rect.top()),
                egui::pos2(x + half_width, rect.bottom()),
            ),
            0.0,
            theme::accent_fill(),
        );
    }

    // Voice bars: height follows the same envelope as the lattice glow,
    // weighted by velocity; color matches the lattice node color.
    for voice in state.tracker.voices() {
        let activation = voice.activation(now, state.view.highlight_time);
        if activation <= 0.0 {
            continue;
        }
        let x = x_of(voice.pitch_class.to_cents());
        let height =
            rect.height() * 0.85 * activation * (0.35 + 0.65 * voice.velocity);
        let c = channel_color(
            voice.channel,
            voice.pitch,
            state.view.darkest_pitch,
            state.view.brightest_pitch,
        );
        let color = egui::Color32::from_rgb(
            (c.x * 255.0) as u8,
            (c.y * 255.0) as u8,
            (c.z * 255.0) as u8,
        );
        painter.line_segment(
            [
                egui::pos2(x, rect.bottom()),
                egui::pos2(x, rect.bottom() - height),
            ],
            egui::Stroke::new(3.0, color),
        );
    }

    // Hovering here highlights the matching lattice node (if in view) and
    // shows the cents under the cursor.
    if let Some(pointer) = response.hover_pos() {
        let cents = ((pointer.x - rect.left()) / rect.width() * 1200.0).clamp(0.0, 1200.0);
        let pc = lattice_core::PitchClass::from_cents(cents);
        state.hovered = coords::positions_within(
            -state.view.extent_threes..=state.view.extent_threes,
            -state.view.extent_fives..=state.view.extent_fives,
            -state.view.extent_sevens..=state.view.extent_sevens,
        )
        .find(|&pos| state.tuning.matches(pc, state.tuning.pitch_class(pos)));
        painter.text(
            egui::pos2(pointer.x + 6.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{cents:.0} cents"),
            egui::TextStyle::Small.resolve(ui.style()),
            theme::text(),
        );
    }
}
