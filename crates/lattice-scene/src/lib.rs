//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.

use glam::{Mat4, Vec2, Vec3, Vec4};
use lattice_core::{coords, LatticePos, NoteTracker, Tuning};

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
pub fn lattice_to_world(pos: LatticePos, spacing: f32) -> Vec3 {
    Vec3::new(
        pos.fives as f32 * spacing,
        pos.threes as f32 * spacing,
        pos.sevens as f32 * spacing,
    )
}

/// How a node indicates which octaves its pitch class is sounding in.
/// All modes read the same per-node octave bitmask; the fragment shader
/// draws the glyphs. Kept as options side by side for design comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OctaveStyle {
    /// No octave indication.
    Off,
    /// Small dots orbiting the disc; clock position = octave.
    #[default]
    Dots,
    /// Concentric rings; inner ring = lowest octave.
    Rings,
    /// A column of ticks beside the disc; bottom = lowest octave.
    Ticks,
}

impl OctaveStyle {
    /// Index used by the shader (uniform `misc.z`).
    pub fn shader_index(self) -> u32 {
        match self {
            OctaveStyle::Off => 0,
            OctaveStyle::Dots => 1,
            OctaveStyle::Rings => 2,
            OctaveStyle::Ticks => 3,
        }
    }
}

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug)]
pub struct ViewConfig {
    /// World-space distance between adjacent nodes.
    pub spacing: f32,
    /// Extent of the displayed lattice along each axis (± steps from origin).
    pub extent_threes: i32,
    pub extent_fives: i32,
    pub extent_sevens: i32,
    /// Seconds a note stays highlighted after release (mirrors the plugin
    /// parameter; the shell copies it in each frame).
    pub highlight_time: f32,
    /// How nodes indicate sounding octaves.
    pub octave_style: OctaveStyle,
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
            extent_threes: 3,
            extent_fives: 3,
            extent_sevens: 1,
            highlight_time: 1.0,
            octave_style: OctaveStyle::default(),
        }
    }
}

/// Simple orbit camera. Angles in radians.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.4,
            pitch: 0.3,
            distance: 12.0,
            fov_y: 45f32.to_radians(),
        }
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let dir = Vec3::new(
            self.pitch.cos() * self.yaw.sin(),
            self.pitch.sin(),
            self.pitch.cos() * self.yaw.cos(),
        );
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // perspective_rh produces 0..1 clip-space depth, which is what wgpu
        // expects.
        Mat4::perspective_rh(self.fov_y, aspect.max(0.01), 0.1, 200.0) * self.view()
    }

    /// Camera-space right/up axes in world space, for billboarding.
    pub fn right_up(&self) -> (Vec3, Vec3) {
        let view = self.view();
        // Rows of the rotation part of the view matrix.
        let right = Vec3::new(view.x_axis.x, view.y_axis.x, view.z_axis.x);
        let up = Vec3::new(view.x_axis.y, view.y_axis.y, view.z_axis.y);
        (right, up)
    }
}

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// RGBA base color (alpha unused for now).
    pub color: Vec4,
    /// 0 = idle, 1 = fully lit. Held notes are 1; released notes decay.
    pub activation: f32,
    /// Bit i set = the node's pitch class is sounding in MIDI octave i
    /// (0..=15 clamped). Drives the per-node octave indicators.
    pub octave_mask: u16,
    pub hovered: bool,
    /// The node's pitch class in cents, for labels/tooltips.
    pub cents: f32,
}

/// Everything the renderer needs for one frame.
pub struct Scene {
    pub nodes: Vec<NodeInstance>,
    pub camera: Camera,
    /// Seconds since app start; available to shaders for animation.
    pub time: f32,
    /// Base node radius in world units (scales with lattice spacing).
    pub node_radius: f32,
    pub octave_style: OctaveStyle,
}

/// Per-channel color palette (channels 0-8 distinct, like v1). Channels 9+
/// fall back to a pitch-height gradient. TODO(port): v1's full channel
/// semantics (10-14 by pitch, 15 outlined, 16 ignored) and the
/// darkest/brightest pitch params.
fn channel_color(channel: u8, note: u8) -> Vec4 {
    const PALETTE: [[f32; 3]; 9] = [
        [0.95, 0.35, 0.25],
        [0.25, 0.65, 0.95],
        [0.35, 0.85, 0.40],
        [0.95, 0.75, 0.25],
        [0.75, 0.40, 0.95],
        [0.25, 0.90, 0.85],
        [0.95, 0.45, 0.70],
        [0.60, 0.70, 0.30],
        [0.55, 0.55, 0.95],
    ];
    if (channel as usize) < PALETTE.len() {
        let [r, g, b] = PALETTE[channel as usize];
        Vec4::new(r, g, b, 1.0)
    } else {
        let t = f32::from(note) / 127.0;
        Vec4::new(0.3 + 0.7 * t, 0.4, 1.0 - 0.7 * t, 1.0)
    }
}

const IDLE_COLOR: Vec4 = Vec4::new(0.16, 0.17, 0.20, 1.0);

/// Build the frame's scene. `hovered` comes from last frame's picking (the
/// usual immediate-mode one-frame latency, invisible in practice).
pub fn derive_scene(
    tracker: &NoteTracker,
    tuning: &Tuning,
    view: &ViewConfig,
    camera: Camera,
    hovered: Option<LatticePos>,
    now: f64,
) -> Scene {
    let mut nodes = Vec::new();

    for pos in coords::positions_within(
        -view.extent_threes..=view.extent_threes,
        -view.extent_fives..=view.extent_fives,
        -view.extent_sevens..=view.extent_sevens,
    ) {
        let node_pc = tuning.pitch_class(pos);

        let mut activation = 0.0f32;
        let mut octave_mask = 0u16;
        let mut color = IDLE_COLOR;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let a = voice.activation(now, view.highlight_time);
                if a > activation {
                    activation = a;
                    color = channel_color(voice.channel, voice.note);
                }
                octave_mask |= 1 << voice.octave.clamp(0, 15) as u16;
            }
        }

        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos: lattice_to_world(pos, view.spacing),
            color,
            activation,
            octave_mask,
            hovered: hovered == Some(pos),
            cents: node_pc.to_cents(),
        });
    }

    Scene {
        nodes,
        camera,
        time: now as f32,
        node_radius: view.spacing * 0.25,
        octave_style: view.octave_style,
    }
}

impl Scene {
    /// CPU picking: the node whose screen projection is nearest the pointer,
    /// within `max_px`. Every pane that wants "hover a pitch class" uses
    /// this and writes the result to the shared UI state.
    pub fn pick(&self, viewport_px: Vec2, pointer_px: Vec2, max_px: f32) -> Option<LatticePos> {
        let view_proj = self.camera.view_proj(viewport_px.x / viewport_px.y.max(1.0));
        let mut best: Option<(f32, LatticePos)> = None;
        for node in &self.nodes {
            let clip = view_proj * node.world_pos.extend(1.0);
            if clip.w <= 0.0 {
                continue;
            }
            let ndc = clip.truncate() / clip.w;
            let px = Vec2::new(
                (ndc.x * 0.5 + 0.5) * viewport_px.x,
                (1.0 - (ndc.y * 0.5 + 0.5)) * viewport_px.y,
            );
            let d = px.distance(pointer_px);
            if d <= max_px && best.map_or(true, |(bd, _)| d < bd) {
                best = Some((d, node.lattice_pos));
            }
        }
        best.map(|(_, pos)| pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_core::{NoteEvent, NoteEventKind};

    #[test]
    fn held_note_lights_matching_nodes() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60, // C4: pitch class 0, octave 4
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let tuning = Tuning::default(); // 12-TET: origin node matches C exactly
        let scene = derive_scene(
            &tracker,
            &tuning,
            &ViewConfig::default(),
            Camera::default(),
            None,
            0.0,
        );
        let origin = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::ORIGIN)
            .unwrap();
        assert_eq!(origin.activation, 1.0);
        assert_eq!(origin.octave_mask, 1 << 4);
    }
}
