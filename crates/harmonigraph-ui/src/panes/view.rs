//! The head of the Lattice settings page: how you look at the lattice
//! (projection, camera angle, saved angles) and how much of it shows (the
//! depth axis and its center). Purely what's framed — a note's own layers are
//! [`super::nodes`], the colors everything is painted with [`super::color`],
//! the render/workspace knobs [`super::system`].
//!
//! Part of the Lattice page ([`super::pages`]) rather than of [`super::tuning`], though view
//! and tuning answer halves of one question — where the nodes sit in pitch,
//! and which of them you are looking at. That kinship is real about the
//! CONTENT and no help on a label, which shows one word: these are twelve
//! control rows to Tuning's five, so a tab merging the two is named for its
//! smaller half, and the camera is reachable only by opening something called
//! Tuning and scrolling.
//!
//! Called View and not Frame because the Video tab's Frame is the video's —
//! aspect, letterbox, crop ticks — and one word naming two unrelated things is
//! the thing the names are audited against (#286).
//!
//! Two parts: the Camera (where you stand and what the lens does) and the
//! Sevenths (how many sheets there are, which is home, and how the ones behind
//! it draw). The Lattice page's View section leads with the sevenths strip and
//! keeps the rest in its More fold ([`super::pages`]). The angle presets live
//! inside Camera rather than under a name of their own, because Cabinet hides
//! that whole block — a name of its own would stand over nothing.

use super::normalize_deg;
use crate::widgets::{button_row, choice_row, LayerStrip, ValueBar};
use crate::{AppearanceDocument, CameraPreset};
use harmonigraph_scene::Camera;
use harmonigraph_scene::Projection;
use harmonigraph_scene::SevensLabel;

/// Room for a short camera-preset name. Asked for flat, with no clamp against
/// the pane: `TextEdit` already takes `desired_width.at_most(available_width)`,
/// so the field shrinks with the row on its own and a narrow column caps it
/// well under this.
const PRESET_NAME_WIDTH: f32 = 110.0;

/// Camera framing: projection, angle, and saved angles.
pub(super) fn camera(
    ui: &mut egui::Ui,
    appearance: &mut AppearanceDocument,
    interaction: &mut crate::Interaction,
) {
    super::block(ui, "Camera");
    // Projection: perspective converges with depth; orthographic keeps
    // equal intervals at equal screen offsets everywhere (isometric-style
    // reading — depth shows only through the node size cue and occlusion).
    choice_row(
        ui,
        "Projection",
        &mut appearance.camera.projection,
        &[
            (
                Projection::Perspective,
                "Perspective",
                "Depth converges and shrinks, like a real camera",
            ),
            (
                Projection::Orthographic,
                "Orthographic",
                "Uniform scale at every depth; parallel lines stay parallel",
            ),
            (
                Projection::Cabinet,
                "Cabinet",
                "Fifths and thirds stay face-on; seventh layers are offset diagonally. Drag to pan; orbit is disabled.",
            ),
        ],
    );
    if appearance.camera.projection == Projection::Cabinet {
        // Cabinet's two drafting knobs: where the sevens axis points on
        // screen, and how long a seventh-step draws relative to a
        // front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
        let mut degrees = appearance.camera.cabinet_angle.to_degrees();
        if ValueBar::new(&mut degrees, 0.0..=90.0, "Depth angle")
            .unit(1.0, "°")
            .decimals(1)
            .show(ui)
            .on_hover_text("Direction of the depth axis, in degrees above horizontal.")
            .changed()
        {
            appearance.camera.cabinet_angle = degrees.to_radians();
        }
        ValueBar::new(&mut appearance.camera.cabinet_scale, 0.1..=1.0, "Depth step scale")
            .unit(1.0, "×")
            .show(ui)
            .on_hover_text(
                "Length of a depth step relative to a step in the front layer. \
                     0.5× gives classic cabinet projection; \
                     1× gives equal step lengths.",
            );
    }
    // Camera angles are meaningless under cabinet (fixed viewpoint), so
    // this whole block hides there (the cabinet knobs show instead).
    if appearance.camera.projection != Projection::Cabinet {
        // The two numbers that fully determine an orthographic view (and
        // the orbit of the other projections) — the same state orbit
        // drags edit, exposed numerically so a view is reproducible.
        let mut yaw_deg = normalize_deg(appearance.camera.yaw.to_degrees());
        if ValueBar::new(&mut yaw_deg, -180.0..=180.0, "Horizontal angle")
            .unit(1.0, "°")
            .decimals(1)
            .show(ui)
            .on_hover_text(
                "Rotate left or right around the lattice. 0° faces the fifths/thirds layer.",
            )
            .changed()
        {
            appearance.camera.yaw = yaw_deg.to_radians();
        }
        let pitch_limit_deg = Camera::PITCH_LIMIT.to_degrees();
        let mut pitch_deg = appearance.camera.pitch.to_degrees();
        if ValueBar::new(&mut pitch_deg, -pitch_limit_deg..=pitch_limit_deg, "Vertical angle")
            .unit(1.0, "°")
            .decimals(1)
            .show(ui)
            .on_hover_text("Tilt the view up or down. 0° looks straight at the lattice.")
            .changed()
        {
            appearance.camera.pitch = pitch_deg.to_radians();
        }
        // Under orthographic, the readable meaning of an angle pair: how
        // long a unit step along each lattice axis draws on screen.
        if appearance.camera.projection == Projection::Orthographic {
            let d = (appearance.camera.target - appearance.camera.eye()).normalize_or_zero();
            let f = |c: f32| (1.0 - c * c).max(0.0).sqrt();
            crate::widgets::weak(
                ui,
                format!(
                    "Axis scale: thirds {:.2}× · fifths {:.2}× · sevenths {:.2}×",
                    f(d.x),
                    f(d.y),
                    f(d.z),
                ),
            );
        }

        // One-click reading angles: built-ins plus user-saved presets.
        let labels: Vec<String> = ["Flat", "Isometric"]
            .iter()
            .map(|s| s.to_string())
            .chain(interaction.camera_presets.iter().map(|p| p.name.clone()))
            .collect();
        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
        crate::widgets::preset_row(ui, "Angle", &labels, |ui, menu| {
            if ui.button("Flat").on_hover_text("Face the fifths/thirds sheet straight on").clicked()
            {
                appearance.camera.yaw = 0.0;
                appearance.camera.pitch = 0.0;
                if menu {
                    ui.close();
                }
            }
            if ui
                .button("Isometric")
                .on_hover_text("Classic isometric angle: all three axes equally foreshortened")
                .clicked()
            {
                appearance.camera.yaw = std::f32::consts::FRAC_PI_4;
                appearance.camera.pitch = (1.0 / 2f32.sqrt()).atan();
                if menu {
                    ui.close();
                }
            }
            let mut delete = None;
            for (i, preset) in interaction.camera_presets.iter().enumerate() {
                let response = ui
                    .button(&preset.name)
                    .on_hover_text("Apply this saved angle (right-click to delete)");
                if response.clicked() {
                    appearance.camera.yaw = preset.yaw;
                    appearance.camera.pitch = preset.pitch;
                    if menu {
                        ui.close();
                    }
                }
                egui::Popup::context_menu(&response)
                    .style(crate::widgets::menu_style(ui.ctx()))
                    .show(|ui| {
                        if ui.button("Delete").clicked() {
                            delete = Some(i);
                            ui.close();
                        }
                    });
            }
            if let Some(i) = delete {
                interaction.camera_presets.remove(i);
            }
        });
        button_row(ui, |ui| {
            let field = crate::widgets::row_field(ui, &mut interaction.preset_name)
                .hint_text("preset name")
                .desired_width(PRESET_NAME_WIDTH * crate::theme::ui_scale(ui.ctx()));
            ui.add(field);
            if ui
                .button("Save angle")
                .on_hover_text(
                    "Save the current yaw and pitch as a preset button on the \
                         Angle row.",
                )
                .clicked()
            {
                let trimmed = interaction.preset_name.trim();
                let name = if trimmed.is_empty() {
                    // Nameless saves still get a self-describing label.
                    format!(
                        "y{:.0} p{:.0}",
                        normalize_deg(appearance.camera.yaw.to_degrees()),
                        appearance.camera.pitch.to_degrees()
                    )
                } else {
                    trimmed.to_string()
                };
                interaction.camera_presets.push(CameraPreset {
                    name,
                    yaw: appearance.camera.yaw,
                    pitch: appearance.camera.pitch,
                });
                interaction.preset_name.clear();
            }
        });
    }
}

/// The depth axis, whole: which sheets there are, which one is home, and how
/// the ones off the home sheet draw. Layer size falloff and Off-home layer labels are
/// inert while the strip holds one sheet (a flat lattice has only the home
/// sheet), so they disable themselves rather than pretending otherwise; the
/// strip is what turns depth on, and is live whatever it is set to.
///
/// Two functions, because the strip leads the View section and the two rows
/// that shape the other sheets sit in its More fold: no saved project had moved
/// either. Still one subject — what the strip sets is which SHEETS there are,
/// and the rows are how those sheets draw.
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
/// What the size and the labels are for: the 5-limit sheet wants its pitch
/// classes as large as they will go, and turning depth on asks the same
/// rectangle to hold three or five times the nodes. The way out is not to
/// shrink the home sheet — that is the picture — but to let the sevens layer sit
/// ON it, smaller and clearing its own space.
///
/// The Clearance bar itself is with the node settings, not here. It is cut by
/// every sounding node on every sheet, so it is a property of the node rather
/// than of this layer, whatever its field names say.
pub(super) fn sevens_strip(ui: &mut egui::Ui, appearance: &mut AppearanceDocument) {
    // Which sheets, and which of them is home, in lattice steps from C (v1's
    // Grid Z). One control because the three are one answer: an end means
    // nothing without knowing where home is, and home means nothing outside
    // the ends.
    LayerStrip::new(
        &mut appearance.view.min_sevens,
        &mut appearance.view.center_sevens,
        &mut appearance.view.max_sevens,
        appearance.view.sevens_size,
    )
    .show(ui)
    .on_hover_text(
        "Which seventh layers the lattice draws and which of them is home, \
             counted in seventh steps from the layer containing C. Drag an end to \
             add or drop layers on that side, the middle mark to move home, \
             between them to slide the whole stack. Double-click for the home \
             layer alone.",
    );
}

/// How the sheets off home draw, inert while the strip holds one sheet.
pub(super) fn sevens_depth(ui: &mut egui::Ui, appearance: &mut AppearanceDocument) {
    let has_depth = appearance.view.max_sevens != appearance.view.min_sevens;
    ui.add_enabled_ui(has_depth, |ui| {
            ValueBar::new(&mut appearance.view.sevens_size, 0.15..=1.0, "Layer size falloff")
            .unit(1.0, "×")
                .show(ui)
                .on_hover_text(
                    "Node size multiplier for each step away from the center layer, in either direction. 1× keeps every layer the same size.",
                );
            choice_row(
                ui,
                "Off-home layer labels",
                &mut appearance.view.sevens_label,
                &[
                    (
                        SevensLabel::Name,
                        "Name",
                        "Note names with septimal marks to distinguish seventh layers.",
                    ),
                    (
                        SevensLabel::Cents,
                        "Cents",
                        "Pitch class in cents on layers away from the center.",
                    ),
                    (SevensLabel::None, "None", "Hide labels on layers away from the center."),
                ],
            );
        });
}
