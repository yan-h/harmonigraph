//! The individual panes. Adding a pane = add a `Tab` variant, a title, and
//! a body function; it immediately participates in docking, and gets the
//! shared state (hover, console, tracker) for free.

use egui::Sense;
use lattice_core::tuning;
use lattice_render::lattice_paint_callback;
use lattice_scene::{channel_color, derive_scene, NodeStyle, OctaveStyle};

use crate::theme;

use crate::params::{ParamBackend, ParamKey};
use crate::widgets::ValueBar;
use crate::SharedState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Tab {
    Lattice,
    Tuning,
    View,
    Appearance,
    Console,
    Spectral,
    Notes,
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
            Tab::View => "View".into(),
            Tab::Appearance => "Appearance".into(),
            Tab::Console => "Console".into(),
            Tab::Spectral => "Spectral".into(),
            Tab::Notes => "Notes".into(),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Lattice => lattice_pane(ui, self.state, self.now),
            Tab::Tuning => tuning_pane(ui, self.state, self.params, self.now),
            Tab::View => view_pane(ui, self.state),
            Tab::Appearance => appearance_pane(ui, self.state, self.params),
            Tab::Console => console_pane(ui, self.state),
            Tab::Spectral => spectral_pane(ui, self.state, self.now),
            Tab::Notes => notes_pane(ui, self.state),
        }
    }
}

/// The 3D lattice view: orbit camera on drag, zoom on scroll, pick on hover.
fn lattice_pane(ui: &mut egui::Ui, state: &mut SharedState, now: f64) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
    if rect.width() < 1.0 || rect.height() < 1.0 {
        return;
    }

    // Camera input: plain drag orbits; shift-drag or middle-drag pans
    // (speeds and clamps live on Camera itself).
    let shift = ui.input(|i| i.modifiers.shift);
    let panning = response.dragged_by(egui::PointerButton::Middle)
        || (response.dragged_by(egui::PointerButton::Primary) && shift);
    if panning {
        let delta = response.drag_delta();
        state.camera.pan(glam::Vec2::new(delta.x, delta.y));
    } else if response.dragged_by(egui::PointerButton::Primary) {
        let delta = response.drag_delta();
        state.camera.orbit(glam::Vec2::new(delta.x, delta.y));
    }
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            state.camera.zoom(scroll);
        }
    }
    if response.double_clicked() {
        state.camera = Default::default();
    }

    let scene = derive_scene(
        &state.tracker,
        &state.tuning,
        &state.view,
        &state.frame_params,
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

    if state.learn_active {
        draw_learn_overlay(ui, rect, now);
    }
    if state.view.show_labels {
        draw_node_labels(ui, rect, &scene, state.view.show_cents, state.view.meantone);
    }
    hover_tooltip(response, state);
}

/// Learn mode is armed: show it ON the lattice too, so the mode is obvious
/// even when the Tuning tab (and its Learn toggle) is hidden.
fn draw_learn_overlay(ui: &egui::Ui, rect: egui::Rect, now: f64) {
    let color = theme::armed().gamma_multiply(learn_pulse(now));
    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect.shrink(1.5),
        0,
        egui::Stroke::new(2.0, color),
        egui::StrokeKind::Inside,
    );
    outlined_text(
        &painter,
        rect.left_top() + egui::vec2(10.0, 8.0),
        egui::Align2::LEFT_TOP,
        "LEARN".to_string(),
        egui::FontId::monospace(12.0),
        color,
        theme::well().gamma_multiply(learn_pulse(now)),
    );
}

/// Labels on hovered and sounding nodes, drawn as egui text over the 3D
/// view (projected with the same camera as the nodes): the note name
/// centered on the node, optionally its pitch class in cents just below.
fn draw_node_labels(
    ui: &egui::Ui,
    rect: egui::Rect,
    scene: &lattice_scene::Scene,
    show_cents: bool,
    meantone: bool,
) {
    let projector = scene.projector(glam::Vec2::new(rect.width(), rect.height()));
    for node in &scene.nodes {
        if !(node.hovered || node.activation > 0.0) {
            continue;
        }
        let Some(p) = projector.project(node.world_pos) else {
            continue;
        };
        // Fade with the activation envelope; hovered idle nodes get a dim
        // but readable label.
        let strength = if node.hovered { 1.0 } else { visibility_floor(node.activation) };
        let center = egui::pos2(rect.min.x + p.x, rect.min.y + p.y);
        let outline = theme::well().gamma_multiply(strength);
        // Meantone tempers out the syntonic comma, so drop the comma marks
        // (E- and E name the same pitch).
        let name = node.lattice_pos.note_name();
        let name = if meantone { name.without_syntonic_commas() } else { name };
        // Monospace for in-lattice text: labels align across nodes and
        // match the technical feel of the readouts.
        outlined_text(
            ui.painter(),
            center,
            egui::Align2::CENTER_CENTER,
            name.to_string(),
            egui::FontId::monospace(12.0),
            theme::text().gamma_multiply(strength),
            outline,
        );
        if show_cents {
            outlined_text(
                ui.painter(),
                center + egui::vec2(0.0, 9.0),
                egui::Align2::CENTER_TOP,
                format!("{:.2}", node.cents),
                egui::FontId::monospace(10.0),
                theme::text_dim().gamma_multiply(strength),
                outline,
            );
        }
    }
}

/// Text drawn over the 3D view, haloed so it stays readable whatever
/// ends up behind it (bright nodes, edges, glow). The outline color
/// should be the skin's recessed surface (`theme::well`), which
/// contrasts with its text color by construction.
///
/// The halo is the galley stamped around two rings: a tight opaque ring
/// for contrast and a wider faint one that fades the edge out. Every
/// sample sits at the same radius, snapped to whole physical pixels —
/// mixed cardinal/diagonal offsets and sub-pixel radii both read as a
/// lumpy outline on high-DPI displays.
fn outlined_text(
    painter: &egui::Painter,
    anchor: egui::Pos2,
    align: egui::Align2,
    text: String,
    font: egui::FontId,
    color: egui::Color32,
    outline: egui::Color32,
) {
    let galley = painter.layout_no_wrap(text, font, egui::Color32::PLACEHOLDER);
    let pos = align.anchor_size(anchor, galley.size()).min;
    let ppp = painter.ctx().pixels_per_point();
    let snap = |pt: f32| (pt * ppp).round().max(1.0) / ppp;
    // Soft ring first so the crisp ring and fill paint over it. Stamp
    // alpha is well below the intended rim alpha because neighboring
    // stamps overlap and accumulate.
    for (radius, alpha) in [(snap(2.0), 0.15), (snap(1.2), 1.0)] {
        let ring = outline.gamma_multiply(alpha);
        for i in 0..16 {
            let angle = std::f32::consts::TAU * i as f32 / 16.0;
            let off = egui::vec2(angle.cos(), angle.sin()) * radius;
            painter.galley(pos + off, galley.clone(), ring);
        }
    }
    painter.galley(pos, galley, color);
}

/// Hover tooltip: pitch class + sounding octaves.
fn hover_tooltip(response: egui::Response, state: &SharedState) {
    let Some(pos) = state.hovered else {
        return;
    };
    let pc = state.tuning.pitch_class(pos);
    let octaves: Vec<String> = state
        .tracker
        .voices()
        .filter(|v| state.tuning.matches(v.pitch_class, pc))
        .map(|v| v.display_octave().to_string())
        .collect();
    let name = pos.note_name();
    let name = if state.view.meantone { name.without_syntonic_commas() } else { name };
    let mut text = format!(
        "{}  ({}, {}, {})  {}",
        name, pos.threes, pos.fives, pos.sevens, pc
    );
    if !octaves.is_empty() {
        text.push_str(&format!("  octaves: {}", octaves.join(" ")));
    }
    response.on_hover_ui(|ui| {
        ui.label(text);
    });
}

/// The dimmest-visible convention shared with the shader (level_floor in
/// lattice.wgsl): quiet elements sit at 35% and scale up to full.
fn visibility_floor(level: f32) -> f32 {
    0.35 + 0.65 * level
}

/// The visible lattice node whose pitch class most closely matches `pc`
/// under the current tuning (several can match when the tolerance is
/// wide). Every pane that answers "which node is this pitch class" uses
/// this, so they can't disagree.
fn nearest_visible_node(
    view: &lattice_scene::ViewConfig,
    tuning: &lattice_core::Tuning,
    pc: lattice_core::PitchClass,
) -> Option<lattice_core::LatticePos> {
    view.visible_positions()
        .filter(|&pos| tuning.matches(pc, tuning.pitch_class(pos)))
        .min_by_key(|&pos| pc.distance_to(tuning.pitch_class(pos)))
}

/// Attention pulse for armed-mode indicators: a slow, shallow breathe
/// (calm "armed", not "alarmed").
fn learn_pulse(now: f64) -> f32 {
    0.78 + 0.22 * (now * 2.0 * std::f64::consts::PI * 0.6).sin() as f32
}

/// One editable ValueBar for a parameter, with automation-gesture
/// bracketing so a drag records as a single host gesture.
fn param_bar(ui: &mut egui::Ui, params: &dyn ParamBackend, key: ParamKey) {
    let mut value = params.get(key);
    let response = ValueBar::new(&mut value, key.range(), key.label())
        .eased(key.logarithmic())
        .decimals(2)
        .show(ui);
    // Bracket drags so the host records one automation gesture per drag;
    // one-shot changes (typed values) go through set() alone.
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

/// One ValueBar per parameter, with automation-gesture bracketing.
fn param_bars(ui: &mut egui::Ui, params: &dyn ParamBackend, keys: &[ParamKey]) {
    for &key in keys {
        param_bar(ui, params, key);
    }
}

/// The major-third bar while meantone mode drives it: read-only, showing
/// the derived value (four fifths minus two octaves) the lattice actually
/// uses. Distinct label + dimmed bar make the lock obvious.
fn locked_third_bar(ui: &mut egui::Ui, params: &dyn ParamBackend) {
    let mut derived = tuning::meantone_third(params.get(ParamKey::Three));
    ValueBar::new(&mut derived, ParamKey::Five.range(), "Major third (¢, locked)")
        .decimals(2)
        .locked(true)
        .show(ui)
        .on_hover_text(
            "Meantone: the major third follows the perfect fifth \
             (four fifths minus two octaves)",
        );
}

fn tuning_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
    now: f64,
) {
    // Tuning sliders. In meantone mode the major third is locked to four
    // perfect fifths, so its bar is shown read-only at the derived value.
    for &key in &ParamKey::TUNING {
        if key == ParamKey::Five && state.view.meantone {
            locked_third_bar(ui, params);
        } else {
            param_bar(ui, params, key);
        }
    }

    ui.horizontal(|ui| {
        if ui.button("Just").clicked() {
            params.set(ParamKey::Three, tuning::THREE_JUST);
            params.set(ParamKey::Five, tuning::FIVE_JUST);
            params.set(ParamKey::Seven, tuning::SEVEN_JUST);
            // Just intonation keeps the syntonic comma, so it isn't a
            // meantone: drop the lock instead of silently overriding the
            // just third we just set.
            state.view.meantone = false;
        }
        if ui.button("12-TET").clicked() {
            params.set(ParamKey::Three, tuning::THREE_12TET);
            params.set(ParamKey::Five, tuning::FIVE_12TET);
            params.set(ParamKey::Seven, tuning::SEVEN_12TET);
            // 12-TET is itself a meantone (400 = 4·700 − 2400), so it's
            // consistent either way; leave the lock as the user has it.
        }
        // Meantone mode: lock the major third to four perfect fifths.
        let meantone = ui
            .add(egui::Button::new("Meantone").selected(state.view.meantone))
            .on_hover_text(
                "Lock the major third to four perfect fifths (temper out \
                 the syntonic comma); note-name labels drop their comma marks",
            );
        if meantone.clicked() {
            if state.view.meantone {
                // Turning off: keep the third where the lock left it so the
                // now-editable bar doesn't jump.
                params.set(
                    ParamKey::Five,
                    tuning::meantone_third(params.get(ParamKey::Three)),
                );
            }
            state.view.meantone = !state.view.meantone;
        }
        // v1's tuning-learn mode: while engaged, the tuning re-learns
        // instantly whenever the set of held notes changes (see root_ui).
        // A real Button (selectable labels draw no background when off, so
        // toggle_value read as plain text next to Just/12-TET).
        let learn = ui
            .add(egui::Button::new("Learn").selected(state.learn_active))
            .on_hover_text("While active, continuously set the tuning from the held notes");
        if learn.clicked() {
            state.learn_active = !state.learn_active;
        }
        if state.learn_active {
            // Pulsing armed ring so the engaged mode can't be missed.
            ui.painter().rect_stroke(
                learn.rect.expand(2.0),
                egui::CornerRadius::same(6),
                egui::Stroke::new(2.0, theme::armed().gamma_multiply(learn_pulse(now))),
                egui::StrokeKind::Outside,
            );
        }
    });

    ui.separator();
    // Cross-pane highlight demo: this pane reacts to the lattice hover,
    // reporting the hovered node's pitch class under the current tuning.
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

/// What the grid shows: per-axis extents and window center.
fn view_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    for (extent, range, label) in [
        (&mut state.view.extent_threes, 1.0..=8.0, "Fifths extent"),
        (&mut state.view.extent_fives, 1.0..=8.0, "Thirds extent"),
        (&mut state.view.extent_sevens, 0.0..=4.0, "Sevenths extent"),
        // Window center in lattice steps from C (v1's Grid X/Y/Z).
        (&mut state.view.center_threes, -20.0..=20.0, "Fifths center"),
        (&mut state.view.center_fives, -20.0..=20.0, "Thirds center"),
        (&mut state.view.center_sevens, -20.0..=20.0, "Sevenths center"),
    ] {
        let mut value = *extent as f32;
        if ValueBar::new(&mut value, range, label).integer().show(ui).changed() {
            *extent = value as i32;
        }
    }
}

/// Cosmetic settings, apart from the structural View pane: how things
/// fade and color, not what the grid shows.
fn appearance_pane(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend) {
    param_bars(ui, params, &ParamKey::APPEARANCE);
    ui.checkbox(&mut state.view.show_labels, "Note labels");
    // Cents ride on the labels, so the toggle grays out with them off.
    ui.add_enabled(
        state.view.show_labels,
        egui::Checkbox::new(&mut state.view.show_cents, "Cent values"),
    );

    // Held-note render style: switchable experiments (idle nodes look the
    // same in all of them). Compare live while notes play.
    ui.horizontal(|ui| {
        ui.label("Node style");
        for (style, label) in [
            (NodeStyle::Steady, "Steady"),
            (NodeStyle::Breathe, "Breathe"),
            (NodeStyle::Corona, "Corona"),
            (NodeStyle::Sparks, "Sparks"),
            (NodeStyle::Wire, "Wire"),
        ] {
            ui.selectable_value(&mut state.view.node_style, style, label);
        }
    });
    ui.checkbox(&mut state.view.show_chord_edges, "Chord edges")
        .on_hover_text("Light up lattice edges between simultaneously held nodes");

    // Octave indicator style: kept as switchable design candidates so they
    // can be compared live while notes play. The Ticks variants differ in
    // the reference frame showing where the octave range starts/ends.
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
            egui::FontId::monospace(10.0),
            theme::text_dim(),
        );
    }

    // Where the visible lattice nodes sit: small ticks along the bottom.
    for pos in state.view.visible_positions() {
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
        let activation = voice.activation(now, state.frame_params.pitch_class_fade_time);
        if activation <= 0.0 {
            continue;
        }
        let x = x_of(voice.pitch_class.to_cents());
        let height =
            rect.height() * 0.85 * activation * visibility_floor(voice.velocity);
        let c = channel_color(
            voice.channel,
            voice.pitch,
            state.frame_params.darkest_pitch,
            state.frame_params.brightest_pitch,
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
        state.hovered = nearest_visible_node(&state.view, &state.tuning, pc);
        painter.text(
            egui::pos2(pointer.x + 6.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            format!("{cents:.0} cents"),
            egui::FontId::monospace(10.5),
            theme::text(),
        );
    }
}


/// Debug printout of all held notes, one line per voice, sorted by
/// descending absolute pitch. Note names use the 12-TET spelling of the
/// MIDI key; octave numbers use Bitwig's convention (middle C = C3); the
/// cents column shows the sounding pitch class (including per-note
/// tuning); the node column shows which lattice position the pitch class
/// lights up under the current tuning/tolerance and view extents ("--"
/// = sounding but not represented anywhere on the visible lattice).
fn notes_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    const KEY_NAMES: [&str; 12] = [
        "C", "C\u{266F}", "D", "D\u{266F}", "E", "F",
        "F\u{266F}", "G", "G\u{266F}", "A", "A\u{266F}", "B",
    ];

    let mut voices: Vec<_> = state
        .tracker
        .voices()
        .filter(|v| v.state == lattice_core::VoiceState::Held)
        .collect();
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
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for voice in voices {
                let node = nearest_visible_node(&state.view, &state.tuning, voice.pitch_class)
                    .map(|pos| {
                        let name = pos.note_name();
                        let name =
                            if state.view.meantone { name.without_syntonic_commas() } else { name };
                        name.to_string()
                    });
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
                    text = text
                        .color(theme::warning_text())
                        .background_color(theme::warning_bg());
                }
                ui.label(text);
            }
        });
}
