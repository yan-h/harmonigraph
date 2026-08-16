//! The head of the Display tab's Lattice page: how you look at the lattice
//! (projection, camera angle, saved angles) and how much of it shows (the
//! depth axis and its center). Purely what's framed — a note's own layers are
//! [`super::nodes`], the colors everything is painted with [`super::color`],
//! the render/workspace knobs [`super::system`].
//!
//! Part of [`super::display`] rather than of [`super::tuning`], though view
//! and tuning answer halves of one question — where the nodes sit in pitch,
//! and which of them you are looking at. That kinship is real about the
//! CONTENT and no help on a label, which shows one word: these are thirteen
//! control rows to Tuning's five, so a tab merging the two is named for its
//! smaller half, and the camera is reachable only by opening something called
//! Tuning and scrolling.
//!
//! Called View and not Frame because the Video tab's Frame is the video's —
//! aspect, letterbox, crop ticks — and one word naming two unrelated things is
//! the thing the names are audited against (#286).
//!
//! Two sections: the Camera (where you stand and what the lens does) and the
//! Sevenths (how many sheets there are, which is home, and how the ones behind
//! it draw). The angle presets live inside Camera rather than under a heading
//! of their own, because Cabinet hides that whole block — a section of its own
//! would leave a heading standing over nothing.

use super::section;
use crate::widgets::{button_row, choice_row, ValueBar};
use crate::{CameraPreset, SharedState};
use super::normalize_deg;
use harmonigraph_scene::Camera;
use harmonigraph_scene::Projection;
use harmonigraph_scene::SevensLabel;

/// Room for a short camera-preset name. Asked for flat, with no clamp against
/// the pane: `TextEdit` already takes `desired_width.at_most(available_width)`,
/// so the field shrinks with the row on its own and a narrow column caps it
/// well under this.
const PRESET_NAME_WIDTH: f32 = 110.0;

/// Camera framing and the lattice window: projection, angle, and the depth
/// axis's extent and center.
pub(super) fn view_pane(ui: &mut egui::Ui, state: &mut SharedState) {
    // A plain heading rather than `section`: this leads the Lattice page, so
    // the rule `section` draws would sit directly under the page picker. The
    // Colors page and the Tuning pane open the same way.
    ui.heading("Camera");
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
        button_row(ui, |ui| {
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
            let field = crate::widgets::row_field(ui, &mut state.preset_name)
                .hint_text("preset name")
                .desired_width(PRESET_NAME_WIDTH * crate::theme::ui_scale(ui.ctx()));
            ui.add(field);
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

    sevens_section(ui, state);
}

/// The depth axis, whole: how many sheets there are, which one is home, and how
/// the ones behind the home sheet draw. Size and label are inert with the
/// sevenths extent at 0 (a flat lattice has only the home sheet), so they
/// disable themselves rather than pretending otherwise; the extent and the
/// center are what turn depth on, and are live whatever it is set to.
///
/// One section rather than an Extents heading over the first two: an extent
/// here is how many SHEETS there are, so it is the same subject as how those
/// sheets draw, and a heading over the pair spent a name on the distinction
/// between a count and a size.
///
/// The other two axes have no extent to set, which is why this one is not a
/// heading promising three. The fifths and thirds sheet is unbounded, and what
/// is drawn of it is whatever the pane is looking at (`ViewConfig::scrolled`) —
/// pan and the window walks with you, so a bar saying how much of it exists
/// would be a bar saying nothing. The sevens axis is different in kind rather
/// than merely spared: a sheet is not drawn somewhere on screen for the camera
/// to find, it is drawn over the home one at an offset, so how many there are
/// is a thing only a control can answer.
///
/// What the size and the label are for: the 5-limit sheet wants its pitch
/// classes as large as they will go, and turning depth on asks the same
/// rectangle to hold three or five times the nodes. The way out is not to
/// shrink the home sheet — that is the picture — but to let the sevens layer sit
/// ON it, smaller and clearing its own gutter.
///
/// The gutter itself is with the node settings, not here. It is cleared by every
/// sounding node on every sheet, so it is a property of the node rather than
/// of this layer, whatever its field names say.
fn sevens_section(ui: &mut egui::Ui, state: &mut SharedState) {
    section(ui, "Sevenths");
    for (extent, range, label) in [
        // Ranges must contain the ViewConfig defaults or the bar could never
        // drag back to them.
        (&mut state.view.extent_sevens, 0.0..=4.0, "Sevenths extent"),
        // Which sheet is home, in lattice steps from C (v1's Grid Z).
        (&mut state.view.center_sevens, -20.0..=20.0, "Sevenths center"),
    ] {
        let mut value = *extent as f32;
        if ValueBar::new(&mut value, range, label).integer().show(ui).changed() {
            *extent = value as i32;
        }
    }
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
        choice_row(
            ui,
            "Sevenths label",
            &mut state.view.sevens_label,
            &[
                (
                    SevensLabel::Name,
                    "Name",
                    "The note name, carrying the septimal mark that tells it \
                     from the node two fifths down that shares its letter",
                ),
                (
                    SevensLabel::Cents,
                    "Cents",
                    "The pitch class alone, in cents. Says what the node is \
                     and nothing it isn't",
                ),
                (SevensLabel::None, "None", "No text off the home sheet"),
            ],
        );
    });
}
