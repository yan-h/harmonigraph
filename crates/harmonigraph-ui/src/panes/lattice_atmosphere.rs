//! Dusk controls and a bounded set of drifting lights behind the lattice.
//! Nebula texture and breathing belong to the renderer's existing glow pass.
//! Motes use ordinary egui geometry shared by the editor and offline draw path.
//! Phase is decorative: exports repeat, but takes do not store the live clock origin.

use egui::{vec2, Color32, Mesh, Painter, Rect, Vec2};
use harmonigraph_scene::{
    AtmosphereSettings, Camera, Projection, ViewConfig, ATMOSPHERE_MOTES_MAX,
};

pub(super) fn settings(ui: &mut egui::Ui, settings: &mut AtmosphereSettings) {
    use crate::widgets::{toggle_switch, ValueBar};

    super::section(ui, "Atmosphere (prototype)");
    toggle_switch(ui, &mut settings.enabled, "Enabled");
    if ui.small_button("Reset defaults").clicked() {
        *settings = AtmosphereSettings::default();
    }
    ui.add_enabled_ui(settings.enabled, |ui| {
        ui.label(egui::RichText::new("Nebula glow").strong());
        ValueBar::new(&mut settings.nebula_depth, 0.0..=1.0, "Texture depth")
            .percent().show(ui).on_hover_text("Cloud texture in the combined lattice glow. 0% restores smooth halos. Requires Lattice glow below; colors come from the notes.");
        multiplier(ui, &mut settings.nebula_scale, "Cloud size", 0.25..=4.0);
        multiplier(ui, &mut settings.nebula_speed, "Cloud speed", 0.0..=20.0)
            .on_hover_text("1× is a slow drift. 0 freezes the cloud motion.");

        ui.label(egui::RichText::new("Stars and fireflies").strong());
        let mut count = settings.mote_count as f32;
        if ValueBar::new(&mut count, 0.0..=ATMOSPHERE_MOTES_MAX as f32, "Light count")
            .integer().show(ui).changed() {
            settings.mote_count = count as u32;
        }
        ValueBar::new(&mut settings.warm_fraction, 0.0..=1.0, "Firefly share")
            .percent().show(ui).on_hover_text("0% gives only cool stars; 100% gives only warm fireflies.");
        multiplier(ui, &mut settings.mote_size, "Light size", 0.25..=4.0);
        multiplier(ui, &mut settings.mote_brightness, "Light brightness", 0.0..=4.0);
        tint(ui, "Star color", &mut settings.cool_color);
        tint(ui, "Firefly color", &mut settings.warm_color);
        multiplier(ui, &mut settings.drift_amount, "Wander distance", 0.0..=4.0);
        multiplier(ui, &mut settings.drift_speed, "Wander speed", 0.0..=4.0);
        ValueBar::new(&mut settings.twinkle_amount, 0.0..=1.0, "Twinkle depth")
            .percent().show(ui).on_hover_text("0% keeps brightness steady; higher values let each light fade further between flashes.");
        multiplier(ui, &mut settings.twinkle_speed, "Twinkle speed", 0.0..=4.0);
        ValueBar::new(&mut settings.parallax, 0.0..=1.0, "Camera parallax")
            .percent().show(ui).on_hover_text("0% keeps the backdrop fixed while you move the lattice. Higher values let nearby lights shift more than distant ones.");

        ui.label(egui::RichText::new("Breathing halos").strong());
        ValueBar::new(&mut settings.breath_amount, 0.0..=1.0, "Breathing depth")
            .percent().show(ui).on_hover_text("Brightness variation in the existing lattice glow. 0% keeps it steady; 100% allows deep fades. Requires Lattice glow to be enabled below.");
        multiplier(ui, &mut settings.breath_speed, "Breathing speed", 0.0..=4.0)
            .on_hover_text("1× is the original slow breathing. 0 keeps the halo at its normal brightness.");
    });
}

fn multiplier(
    ui: &mut egui::Ui,
    value: &mut f32,
    label: &str,
    range: std::ops::RangeInclusive<f32>,
) -> egui::Response {
    crate::widgets::ValueBar::new(value, range, label).unit(1.0, "×").show(ui)
}

fn tint(ui: &mut egui::Ui, label: &str, color: &mut [u8; 3]) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        ui.color_edit_button_srgb(color);
    });
}

fn hash(mut n: u32) -> f32 {
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846c_a68b);
    n ^= n >> 16;
    (n >> 8) as f32 / 16_777_216.0
}

// Zero alpha is additive light in egui's premultiplied mesh.
fn light(rgb: [f32; 3], strength: f32) -> Color32 {
    Color32::from_rgb_additive(
        (rgb[0] * strength).round().clamp(0.0, 255.0) as u8,
        (rgb[1] * strength).round().clamp(0.0, 255.0) as u8,
        (rgb[2] * strength).round().clamp(0.0, 255.0) as u8,
    )
}

pub(super) fn paint(painter: &Painter, rect: Rect, camera: Camera, view: &ViewConfig, now: f64) {
    if !rect.is_positive() || !now.is_finite() {
        return;
    }
    let settings = view.atmosphere.sanitized();
    if !settings.enabled {
        return;
    }
    let mesh = atmosphere(
        rect,
        parallax(camera, view, settings.parallax),
        settings,
        now,
        painter.ctx().pixels_per_point(),
    );
    painter.with_clip_rect(rect).add(egui::Shape::mesh(mesh));
}

/// Camera target is residual: follow_camera transfers whole cells into the
/// view center. Reconstruct the continuous coordinate before mapping it into
/// the backdrop, using f64 so large centers do not erase small pan increments.
fn parallax(camera: Camera, view: &ViewConfig, amount: f32) -> Vec2 {
    if amount == 0.0 {
        return Vec2::ZERO;
    }
    let x = f64::from(camera.target.x) + f64::from(view.center_fives) * f64::from(view.spacing);
    let y = f64::from(camera.target.y) + f64::from(view.center_threes) * f64::from(view.spacing);
    let orbit = if camera.projection == Projection::Cabinet {
        Vec2::ZERO
    } else {
        vec2(camera.yaw.sin(), -camera.pitch.sin()) * 0.025
    };
    (vec2((x * 0.1).tanh() as f32, -(y * 0.1).tanh() as f32) * 0.06 + orbit) * amount
}

fn atmosphere(
    rect: Rect,
    parallax: Vec2,
    settings: AtmosphereSettings,
    now: f64,
    ppp: f32,
) -> Mesh {
    let mut mesh = Mesh::default();
    let count = if settings.mote_brightness > 0.0 { settings.mote_count } else { 0 };
    mesh.vertices.reserve((count * 37) as usize);
    mesh.indices.reserve((count * 180) as usize);
    // Fixed seeds keep each mote's identity through resizing and repeated
    // layout passes. Bounded wandering avoids wraparound pops at the edges.
    let scale = (rect.height() / 800.0).clamp(0.45, 2.0);
    for i in 0..count {
        let seed = i * 11 + 73;
        let phase = f64::from(hash(seed + 2)) * std::f64::consts::TAU;
        // A repeating, dispersed rank keeps identities stable when count is
        // changed; at the default quarter, every fourth mote is a firefly.
        let near = ((i % 4) as f32 + hash(seed + 6)) * 0.25 < settings.warm_fraction;
        let depth = if near { 1.0 } else { 0.35 };
        let drift_time = now * f64::from(settings.drift_speed);
        let wander = vec2(
            (drift_time * (0.09 + f64::from(hash(seed + 3)) * 0.06) + phase).sin() as f32,
            (drift_time * 0.071 + phase * 1.7).sin() as f32,
        ) * (0.025 * depth * settings.drift_amount);
        let uv = vec2(hash(seed), hash(seed + 1)) + wander - parallax * depth;
        let edge = (uv.x.min(1.0 - uv.x).min(uv.y.min(1.0 - uv.y)) * 18.0).clamp(0.0, 1.0);
        let pulse_time = now * f64::from(settings.twinkle_speed);
        let pulse = (0.5
            + 0.5 * (pulse_time * (0.48 + f64::from(hash(seed + 4)) * 0.4) + phase).sin())
            as f32;
        let brightness = (1.0 - settings.twinkle_amount + settings.twinkle_amount * pulse.powi(3))
            * edge
            * settings.mote_brightness;
        let radius = if near { 1.1 + hash(seed + 5) * 0.55 } else { 0.55 + hash(seed + 5) * 0.35 }
            * scale
            * settings.mote_size;
        let rgb = if near { settings.warm_color } else { settings.cool_color }.map(f32::from);
        mote(&mut mesh, rect.min + uv * rect.size(), radius, 1.0 / ppp.max(0.1), rgb, brightness);
    }
    mesh
}

fn mote(
    mesh: &mut Mesh,
    center: egui::Pos2,
    radius: f32,
    pixel: f32,
    rgb: [f32; 3],
    brightness: f32,
) {
    const SEGMENTS: u32 = 12;
    // Filter the core over at least one physical pixel, without enlarging the
    // entire skirt sixfold. Smaller requested sizes reduce the light as well,
    // so the bottom of Size stays useful below a pixel.
    let core = radius.max(pixel);
    let brightness = brightness * (radius / core).powi(2);
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, light(rgb, brightness));
    // A small bright core with a broad, very faint skirt. Three rings keep
    // the falloff soft without requesting a blur or hundreds of circles.
    for (r, level) in [
        (core, 0.32),
        ((radius * 3.0).max(core + pixel * 0.5), 0.035),
        ((radius * 6.0).max(core + pixel), 0.0),
    ] {
        for j in 0..SEGMENTS {
            let angle = j as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            mesh.colored_vertex(
                center + vec2(angle.cos(), angle.sin()) * r,
                light(rgb, brightness * level),
            );
        }
    }
    for j in 0..SEGMENTS {
        let next = (j + 1) % SEGMENTS;
        mesh.add_triangle(base, base + 1 + j, base + 1 + next);
        for ring in 0..2 {
            let a = base + 1 + ring * SEGMENTS;
            let b = a + SEGMENTS;
            mesh.add_triangle(a + j, b + j, a + next);
            mesh.add_triangle(a + next, b + j, b + next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dusk_is_bounded_deterministic_and_moves_without_notes() {
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 800.0));
        let config = AtmosphereSettings::default();
        let first = atmosphere(rect, Vec2::ZERO, config, 0.0, 1.0);
        let again = atmosphere(rect, Vec2::ZERO, config, 0.0, 1.0);
        let later = atmosphere(rect, Vec2::ZERO, config, 3.0, 1.0);
        assert_eq!(first.vertices, again.vertices);
        assert_ne!(first.vertices, later.vertices);
        assert_eq!(first.vertices.len(), 96 * 37);
        assert!(first.is_valid());
        assert!(first.vertices.iter().any(|v| v.color.r() > 80));
        let distant = atmosphere(
            rect,
            parallax(
                Camera { target: glam::vec3(1e30, -1e30, 0.0), ..Camera::default() },
                &ViewConfig::default(),
                1.0,
            ),
            config,
            0.0,
            1.0,
        );
        assert!(
            distant.vertices.iter().any(|v| { rect.contains(v.pos) && v.color.r() > 80 }),
            "a long pan must not carry away every firefly"
        );
    }

    #[test]
    fn parallax_survives_the_lattices_coordinate_rebase() {
        let mut view = ViewConfig::default();
        let mut camera = Camera {
            target: glam::vec3(view.spacing * 0.501, -view.spacing * 1.501, 0.0),
            ..Camera::default()
        };
        let before = parallax(camera, &view, 1.0);
        view.follow_camera(&mut camera);
        assert_ne!(view.center_fives, 0, "reach the coordinate reset");
        assert_ne!(view.center_threes, 0, "reach both axes");
        assert!((before - parallax(camera, &view, 1.0)).length() < 1e-7);
        assert_eq!(parallax(camera, &view, 0.0), Vec2::ZERO);
    }
}
