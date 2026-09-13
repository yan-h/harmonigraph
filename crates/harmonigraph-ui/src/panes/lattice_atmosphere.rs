//! Dusk prototype: a coarse cloud mesh and a bounded set of drifting lights.
//! Both are ordinary egui geometry behind the lattice, shared by the editor
//! and offline draw path. No simulation, texture history, or additional GPU pass.
//! Phase is decorative: exports repeat, but takes do not store the live clock origin.

use egui::{vec2, Color32, Mesh, Painter, Rect, Vec2};
use harmonigraph_scene::{Camera, Projection};

const CLOUD_COLUMNS: u32 = 48;
const CLOUD_ROWS: u32 = 32;
const MOTES: u32 = 96;

fn hash(mut n: u32) -> f32 {
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb_352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846c_a68b);
    n ^= n >> 16;
    (n >> 8) as f32 / 16_777_216.0
}

fn noise(p: Vec2) -> f32 {
    let x = p.x.floor() as i32;
    let y = p.y.floor() as i32;
    let f = vec2(p.x - p.x.floor(), p.y - p.y.floor());
    let s = f * f * (vec2(3.0, 3.0) - f * 2.0);
    let at = |dx, dy| {
        hash((x.wrapping_add(dx) as u32).wrapping_mul(0x9e37_79b9) ^ y.wrapping_add(dy) as u32)
    };
    let a = egui::lerp(at(0, 0)..=at(1, 0), s.x);
    let b = egui::lerp(at(0, 1)..=at(1, 1), s.x);
    egui::lerp(a..=b, s.y)
}

// Zero alpha is additive light in egui's premultiplied mesh. Keep the cloud
// colours faint and leave the user's background visible through the field.
fn light(rgb: [f32; 3], strength: f32) -> Color32 {
    Color32::from_rgb_additive(
        (rgb[0] * strength).round().clamp(0.0, 255.0) as u8,
        (rgb[1] * strength).round().clamp(0.0, 255.0) as u8,
        (rgb[2] * strength).round().clamp(0.0, 255.0) as u8,
    )
}

pub(super) fn paint(painter: &Painter, rect: Rect, camera: Camera, now: f64) {
    if !rect.is_positive() || !now.is_finite() {
        return;
    }
    let mesh = atmosphere(rect, camera, now);
    painter.with_clip_rect(rect).add(egui::Shape::mesh(mesh));
}

fn atmosphere(rect: Rect, camera: Camera, now: f64) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.vertices.reserve(((CLOUD_COLUMNS + 1) * (CLOUD_ROWS + 1) + MOTES * 37) as usize);
    mesh.indices.reserve((CLOUD_COLUMNS * CLOUD_ROWS * 6 + MOTES * 180) as usize);
    let aspect = rect.width() / rect.height().max(1.0);
    // Use the shared presentation clock in f64 until after taking sin. Long
    // sessions retain slow motion, and seeking offline reproduces the frame.
    let drift = vec2((now * 0.021).sin() as f32, (now * 0.017).cos() as f32) * 0.22;
    // A distant field only shifts a little. Bound navigation so a long pan
    // cannot push every mote out of view, and respect Cabinet's fixed angle.
    let orbit = if camera.projection == Projection::Cabinet {
        Vec2::ZERO
    } else {
        vec2(camera.yaw.sin(), -camera.pitch.sin()) * 0.025
    };
    let parallax =
        vec2((camera.target.x * 0.1).tanh(), -(camera.target.y * 0.1).tanh()) * 0.06 + orbit;
    for y in 0..=CLOUD_ROWS {
        for x in 0..=CLOUD_COLUMNS {
            let uv = vec2(x as f32 / CLOUD_COLUMNS as f32, y as f32 / CLOUD_ROWS as f32);
            let p = vec2(uv.x * aspect, uv.y) * 2.8 + parallax;
            let warp = vec2(noise(p + drift), noise(p + vec2(8.3, 2.7) - drift));
            let cloud = noise(p + warp * 0.7 + drift);
            let veil = noise(p * 1.8 - drift + vec2(3.1, 7.4));
            let edge = (16.0 * uv.x * (1.0 - uv.x) * uv.y * (1.0 - uv.y)).sqrt();
            let density = ((cloud - 0.28).max(0.0) * 1.6).powi(2) * edge;
            let plum = veil;
            let rgb = [8.0 + 12.0 * plum, 10.0 + 7.0 * (1.0 - plum), 26.0];
            mesh.colored_vertex(rect.min + uv * rect.size(), light(rgb, density));
        }
    }
    for y in 0..CLOUD_ROWS {
        for x in 0..CLOUD_COLUMNS {
            let a = y * (CLOUD_COLUMNS + 1) + x;
            let b = a + CLOUD_COLUMNS + 1;
            mesh.add_triangle(a, a + 1, b);
            mesh.add_triangle(a + 1, b + 1, b);
        }
    }
    // Fixed seeds keep each mote's identity through resizing and repeated
    // layout passes. Bounded wandering avoids wraparound pops at the edges.
    let scale = (rect.height() / 800.0).clamp(0.45, 2.0);
    for i in 0..MOTES {
        let seed = i * 11 + 73;
        let phase = f64::from(hash(seed + 2)) * std::f64::consts::TAU;
        let near = i % 4 == 0;
        let depth = if near { 1.0 } else { 0.35 };
        let wander = vec2(
            (now * (0.09 + f64::from(hash(seed + 3)) * 0.06) + phase).sin() as f32,
            (now * 0.071 + phase * 1.7).sin() as f32,
        ) * (0.025 * depth);
        let uv = vec2(hash(seed), hash(seed + 1)) + wander - parallax * depth;
        let edge = (uv.x.min(1.0 - uv.x).min(uv.y.min(1.0 - uv.y)) * 18.0).clamp(0.0, 1.0);
        let pulse =
            (0.5 + 0.5 * (now * (0.48 + f64::from(hash(seed + 4)) * 0.4) + phase).sin()) as f32;
        let brightness = (0.08 + 0.92 * pulse.powi(3)) * edge;
        let radius =
            if near { 1.1 + hash(seed + 5) * 0.55 } else { 0.55 + hash(seed + 5) * 0.35 } * scale;
        let rgb = if near { [230.0, 182.0, 100.0] } else { [100.0, 137.0, 180.0] };
        mote(&mut mesh, rect.min + uv * rect.size(), radius, rgb, brightness);
    }
    mesh
}

fn mote(mesh: &mut Mesh, center: egui::Pos2, radius: f32, rgb: [f32; 3], brightness: f32) {
    const SEGMENTS: u32 = 12;
    let base = mesh.vertices.len() as u32;
    mesh.colored_vertex(center, light(rgb, brightness));
    // A small bright core with a broad, very faint skirt. Three rings keep
    // the falloff soft without requesting a blur or hundreds of circles.
    for (r, level) in [(1.0, 0.32), (3.0, 0.035), (6.0, 0.0)] {
        for j in 0..SEGMENTS {
            let angle = j as f32 * std::f32::consts::TAU / SEGMENTS as f32;
            mesh.colored_vertex(
                center + vec2(angle.cos(), angle.sin()) * (radius * r),
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
        let first = atmosphere(rect, Camera::default(), 0.0);
        let again = atmosphere(rect, Camera::default(), 0.0);
        let later = atmosphere(rect, Camera::default(), 3.0);
        assert_eq!(first.vertices, again.vertices);
        assert_ne!(first.vertices, later.vertices);
        assert_eq!(first.vertices.len(), 49 * 33 + 96 * 37);
        assert!(first.is_valid());
        assert!(first.vertices.iter().any(|v| v.color.r() > 80));
        let distant = atmosphere(
            rect,
            Camera { target: glam::vec3(1e30, -1e30, 0.0), ..Camera::default() },
            0.0,
        );
        assert!(
            distant.vertices[49 * 33..]
                .iter()
                .any(|v| { rect.contains(v.pos) && v.color.r() > 80 }),
            "a long pan must not carry away every firefly"
        );
    }
}
