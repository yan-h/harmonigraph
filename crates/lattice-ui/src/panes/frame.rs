//! The Frame controls: how you look at the lattice (projection, camera angle,
//! saved angles) and how much of it shows (per-axis extents and window
//! center). Purely what's framed — the note styling lives in [`super::nodes`]
//! and [`super::scene`], the render/workspace knobs in [`super::panel`].
//!
//! Not a pane of its own. These were a "Frame" tab until it and Tuning — both
//! short, and both about the lattice itself rather than how it is drawn —
//! became two sections of one tab; [`super::tuning`] draws this below its own
//! controls. Kept as a file because the two halves have nothing to say to each
//! other beyond sharing a tab.

use crate::widgets::{button_row, button_row_wrapped, choice_row, ValueBar};
use crate::{CameraPreset, SharedState};
use super::normalize_deg;
use lattice_scene::Camera;
use lattice_scene::Projection;
use lattice_scene::SevensLabel;

/// Camera framing and the lattice window: projection, angle, and per-axis
/// extents/center. Drawn into the Tuning tab, under its own section heading.
pub(super) fn frame_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    // Projection: perspective converges with depth; orthographic keeps
    // equal intervals at equal screen offsets everywhere (isometric-style
    // reading — depth shows only through the node size cue and occlusion).
    button_row(ui, |ui| {
        ui.label("Projection");
        for (proj, label) in [
            (Projection::Perspective, "Perspective"),
            (Projection::Orthographic, "Orthographic"),
            (Projection::Cabinet, "Cabinet"),
        ] {
            ui.selectable_value(&mut state.camera.projection, proj, label)
                .on_hover_text(match proj {
                    Projection::Perspective => "Depth converges and shrinks, like a real camera",
                    Projection::Orthographic => {
                        "Uniform scale at every depth; parallel lines stay parallel"
                    }
                    Projection::Cabinet => {
                        "Face-on fifths/thirds sheet, undistorted; sevenths shear \
                         to a fixed screen arrow (drag pans; orbit is disabled)"
                    }
                });
        }
    });
    if state.camera.projection == Projection::Cabinet {
        // Cabinet's two drafting knobs: where the sevens axis points on
        // screen, and how long a seventh-step draws relative to a
        // front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
        let mut degrees = state.camera.cabinet_angle.to_degrees();
        if ValueBar::new(&mut degrees, 0.0..=90.0, "Sevenths angle")
            .show(ui)
            .changed()
        {
            state.camera.cabinet_angle = degrees.to_radians();
        }
        ValueBar::new(&mut state.camera.cabinet_scale, 0.1..=1.0, "Sevenths length").show(ui);
    }
    // Camera angles are meaningless under cabinet (fixed viewpoint), so
    // this whole block hides there (the cabinet knobs show instead).
    if state.camera.projection != Projection::Cabinet {
        // The two numbers that fully determine an orthographic view (and
        // the orbit of the other projections) — the same state orbit
        // drags edit, exposed numerically so a view is reproducible.
        let mut yaw_deg = normalize_deg(state.camera.yaw.to_degrees());
        if ValueBar::new(&mut yaw_deg, -180.0..=180.0, "Camera yaw")
            .show(ui)
            .changed()
        {
            state.camera.yaw = yaw_deg.to_radians();
        }
        let pitch_limit_deg = Camera::PITCH_LIMIT.to_degrees();
        let mut pitch_deg = state.camera.pitch.to_degrees();
        if ValueBar::new(&mut pitch_deg, -pitch_limit_deg..=pitch_limit_deg, "Camera pitch")
            .show(ui)
            .changed()
        {
            state.camera.pitch = pitch_deg.to_radians();
        }
        // Under orthographic, the readable meaning of an angle pair: how
        // long a unit step along each lattice axis draws on screen.
        if state.camera.projection == Projection::Orthographic {
            let d = (state.camera.target - state.camera.eye()).normalize_or_zero();
            let f = |c: f32| (1.0 - c * c).max(0.0).sqrt();
            ui.weak(format!(
                "Axis lengths — thirds {:.2} · fifths {:.2} · sevenths {:.2}",
                f(d.x),
                f(d.y),
                f(d.z),
            ));
        }

        // One-click reading angles: built-ins plus user-saved presets.
        button_row_wrapped(ui, |ui| {
            ui.label("Angle");
            if ui
                .button("Flat")
                .on_hover_text("Face the fifths/thirds sheet straight on")
                .clicked()
            {
                state.camera.yaw = 0.0;
                state.camera.pitch = 0.0;
            }
            if ui
                .button("Isometric")
                .on_hover_text("Classic isometric angle: all three axes equally foreshortened")
                .clicked()
            {
                state.camera.yaw = std::f32::consts::FRAC_PI_4;
                state.camera.pitch = (1.0 / 2f32.sqrt()).atan();
            }
            let mut delete = None;
            for (i, preset) in state.camera_presets.iter().enumerate() {
                let response = ui
                    .button(&preset.name)
                    .on_hover_text("Apply this saved angle (right-click to delete)");
                if response.clicked() {
                    state.camera.yaw = preset.yaw;
                    state.camera.pitch = preset.pitch;
                }
                response.context_menu(|ui| {
                    if ui.button("Delete").clicked() {
                        delete = Some(i);
                        ui.close();
                    }
                });
            }
            if let Some(i) = delete {
                state.camera_presets.remove(i);
            }
        });
        button_row(ui, |ui| {
            ui.add(
                egui::TextEdit::singleline(&mut state.preset_name)
                    .hint_text("preset name")
                    .desired_width(110.0),
            );
            if ui.button("Save angle").clicked() {
                let trimmed = state.preset_name.trim();
                let name = if trimmed.is_empty() {
                    // Nameless saves still get a self-describing label.
                    format!(
                        "y{:.0} p{:.0}",
                        normalize_deg(state.camera.yaw.to_degrees()),
                        state.camera.pitch.to_degrees()
                    )
                } else {
                    trimmed.to_string()
                };
                state.camera_presets.push(CameraPreset {
                    name,
                    yaw: state.camera.yaw,
                    pitch: state.camera.pitch,
                });
                state.preset_name.clear();
            }
        });
    }

    for (extent, range, label) in [
        // Ranges must contain the ViewConfig defaults (10/6) or the bar
        // could never drag back to them.
        (&mut state.view.extent_threes, 1.0..=12.0, "Fifths extent"),
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

    sevens_layer_controls(ui, state);
}

/// How the sheets other than the home one draw. Size and label are inert
/// with the sevenths extent at 0 (a flat lattice has only the home sheet), so
/// they disable themselves rather than pretending otherwise. The two gutter
/// bars stay live: every sounding node clears, the home sheet included, so at
/// extent 0 they still cut the grid lines under the notes.
///
/// What they are all for: the 5-limit sheet wants its pitch classes as large
/// as they will go, and turning depth on asks the same rectangle to hold
/// three or five times the nodes. The way out is not to shrink the home sheet
/// — that is the picture — but to let the sevens layer sit ON it, smaller and
/// clearing its own gutter.
fn sevens_layer_controls(ui: &mut egui::Ui, state: &mut SharedState) {
    let has_depth = state.view.extent_sevens != 0;
    ui.add_enabled_ui(has_depth, |ui| {
        ValueBar::new(&mut state.view.sevens_size, 0.15..=1.0, "Sevenths size")
            .show(ui)
            .on_hover_text(
                "How much smaller a node draws for each step off the home \
                 sheet. Smaller BOTH ways -- this is distance from the home \
                 sheet, not depth toward you -- so the home sheet stays the \
                 largest thing on screen. 1 draws every sheet alike",
            );
    });
    ValueBar::new(&mut state.view.sevens_gutter, 0.0..=0.5, "Sevenths gutter")
        .show(ui)
        .on_hover_text(
            "The dark gap a node clears around itself, so a sheet reads \
             over the ones behind it instead of needing room of its own. \
             Measured past the node's own edge, and the same width on \
             screen whatever size the node draws at. The home sheet clears \
             too, cutting the grid lines under a sounding note even with no \
             depth at all. 0 draws none",
        );
    ValueBar::new(&mut state.view.sevens_gutter_soft, 0.0..=0.5, "Sevenths gutter fade")
        .show(ui)
        .on_hover_text(
            "How gradually the gap ends, independent of how wide it is. \
             0 is a hard edge; past the gutter's own width it softens \
             outward rather than eating into the node",
        );
    ui.add_enabled_ui(has_depth, |ui| {
        choice_row(
            ui,
            "Sevenths label",
            &mut state.view.sevens_label,
            &[
                (
                    SevensLabel::Comma,
                    "Comma",
                    "The name, plus the signed cents to the home-sheet note \
                     that wears the same name -- the septimal comma, which \
                     moves as you retune",
                ),
                (
                    SevensLabel::Cents,
                    "Cents",
                    "The pitch class alone, in cents. Says what the node is \
                     and nothing it isn't",
                ),
                (
                    SevensLabel::Name,
                    "Name",
                    "The note name, as the home sheet gets. Note that it is \
                     the SAME name the node two fifths down wears: the \
                     spelling carries no sevenths information at all",
                ),
                (SevensLabel::None, "None", "No text off the home sheet"),
            ],
        );
    });
}
