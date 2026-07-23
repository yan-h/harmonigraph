//! The View pane: per-axis lattice extents, window center, and camera
//! framing/projection.

use crate::widgets::{button_row, button_row_wrapped, toggle_switch, ValueBar};
use crate::{CameraPreset, SharedState};
use super::normalize_deg;
use lattice_scene::Camera;
use lattice_scene::Projection;

/// What the grid shows: per-axis extents and window center.
pub(super) fn view_pane(ui: &mut egui::Ui, state: &mut SharedState) {
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

    // Lattice render resolution relative to the pane's native pixels.
    // 1.0 is exact; above it supersamples (crisper glyph edges at some GPU
    // cost), below it renders coarse and upscales (cheaper on huge panes).
    ValueBar::new(&mut state.view.render_scale, 0.5..=2.0, "Render scale")
        .show(ui)
        .on_hover_text(
            "Lattice render resolution: 1.0 = native, higher supersamples, \
             lower renders coarse and upscales",
        );

    button_row(ui, |ui| {
        // Escape hatch for the persisted dock arrangement (it survives
        // every reopen, so a new default layout is otherwise unreachable).
        if ui
            .button("Reset layout")
            .on_hover_text("Restore the default pane arrangement")
            .clicked()
        {
            state.reset_dock_layout();
        }
        ui.checkbox(&mut state.view.frameless, "Frameless").on_hover_text(
            "Hide the tab bars so adjacent panes (lattice over spectrum) \
             record as one seamless surface. Esc restores.",
        );
        ui.checkbox(&mut state.view.show_perf, "Performance overlay").on_hover_text(
            "A corner HUD with frame rate, the GUI's CPU time per frame, \
             process memory, and the voice/node workload — to see if the \
             plugin is working the machine hard. (GPU time isn't measured \
             directly; the node count and render scale stand in for it.)",
        );
    });

    // Take recording: the input half of offline video rendering. A mode
    // with ongoing side effects (it keeps writing a file), so a switch
    // rather than a checkbox — the house rule in widgets.rs.
    if state.take_supported {
        super::section(ui, "Record");
        toggle_switch(ui, &mut state.take_recording, "Record take").on_hover_text(
            "Record everything the visualization is a function of — notes, \
             bends, parameter automation, and the current look — to a .take \
             file. Render it to video afterwards with lattice-offline, at any \
             resolution and frame rate. Events are stamped with transport \
             position, so nothing is captured until the transport rolls and \
             the take lines up with a bounce of the same song.",
        );
        if !state.take_status.is_empty() {
            ui.weak(&state.take_status);
        }

        let render = &mut state.render_config;
        ui.checkbox(&mut render.record_audio, "Record audio too").on_hover_text(
            "Write the plugin's audio input beside the take, so the render \
             gets a spectrum and a soundtrack with no separate bounce. \
             Needs the device to be somewhere audio actually reaches it — \
             after the instrument, or on a bus.",
        );
        ui.checkbox(&mut render.auto_render, "Render video when done").on_hover_text(
            "Run lattice-offline as soon as a take finishes, writing the \
             video next to the take. The render happens in the background \
             — it does not hold up the DAW.",
        );
        ui.checkbox(&mut render.playhead, "Whole-song playhead").on_hover_text(
            "Lay the whole take's spectrogram out at once and sweep a \
             playhead through it, instead of the live scrolling window. \
             Needs audio. Applies to every render of this take; the \
             --playhead flag turns it on too.",
        );
        if render.auto_render {
            crate::widgets::choice_row(
                ui,
                "When",
                &mut render.trigger,
                &[
                    (
                        crate::RenderTrigger::OnDisarm,
                        "Switched off",
                        "Render when you turn Record take off. The only \
                         choice that works with a looping transport.",
                    ),
                    (
                        crate::RenderTrigger::OnTransportStop,
                        "Transport stops",
                        "Render the moment the transport stops after \
                         recording something — a play-through or an audio \
                         export then needs no further clicks. Recording \
                         switches itself off too.",
                    ),
                ],
            );
            // Free-text paths rather than a file dialog: a plugin GUI has
            // no portable one, and these are set once and then left.
            labeled_path(ui, "Renderer", &mut render.renderer_path)
                .on_hover_text(
                    "Path to the lattice-offline binary. Leave empty to use \
                     the copy update-plugin.sh installs.",
                );
            labeled_path(ui, "Audio", &mut render.audio_path).on_hover_text(
                "Bounced WAV to mux in and feed the spectrum. Leave empty \
                 for a silent render with no spectrum curve.",
            );
            labeled_path(ui, "Options", &mut render.extra_args).on_hover_text(
                "Extra lattice-offline flags, split on spaces: \
                 --size 3840x2160 --layout side-by-side",
            );
        }
    }
}

/// A labeled single-line text field that fills the pane width — the shape
/// every path setting in the Record section uses.
fn labeled_path(ui: &mut egui::Ui, label: &str, value: &mut String) -> egui::Response {
    button_row(ui, |ui| {
        ui.label(label);
        ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()))
    })
}
