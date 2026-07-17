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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OctaveStyle {
    /// No octave indication.
    Off,
    /// Small dots orbiting the disc; clock position = octave.
    Dots,
    /// Concentric rings; inner ring = lowest octave.
    Rings,
    /// Tick column with a faint full-height rail as the reference frame.
    TicksRail,
    /// Tick column with every octave slot shown as a dim pip (LED-meter
    /// style); lit pips = sounding octaves.
    #[default]
    TicksLadder,
    /// Tick column with a rail plus emphasized end caps at the bottom and
    /// top of the octave range.
    TicksCaps,
    /// Ladder plus a brighter marker line at the middle-C octave (4), as a
    /// musically meaningful anchor.
    TicksMid,
}

impl OctaveStyle {
    /// Index used by the shader (uniform `misc.z`).
    pub fn shader_index(self) -> u32 {
        match self {
            OctaveStyle::Off => 0,
            OctaveStyle::Dots => 1,
            OctaveStyle::Rings => 2,
            OctaveStyle::TicksRail => 3,
            OctaveStyle::TicksLadder => 4,
            OctaveStyle::TicksCaps => 5,
            OctaveStyle::TicksMid => 6,
        }
    }
}

/// Purely-visual settings (not host-automatable parameters). The UI layer
/// persists these separately from plugin parameters.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
    /// Pitch (MIDI note) mapped to the darkest gradient color on
    /// pitch-colored channels (9-13). Mirrors a plugin parameter.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
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
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
        }
    }
}

/// Simple orbit camera. Angles in radians.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
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
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
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

fn lch(l: f64, c: f64, h: f64) -> Vec4 {
    // The conversion is unclamped and out-of-gamut LCH inputs yield values
    // outside 0..255 (v1's graphics stack clamped downstream; we must do it
    // ourselves before handing colors to the shader).
    let rgb = color_space::Rgb::from(color_space::Lch::new(l, c, h));
    Vec4::new(
        (rgb.r.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.g.clamp(0.0, 255.0) / 255.0) as f32,
        (rgb.b.clamp(0.0, 255.0) / 255.0) as f32,
        1.0,
    )
}

/// Ported verbatim from v1 (`editor/color.rs`): channels 0-8 have fixed
/// LCH colors; 9-13 are colored by pitch height on an LCH gradient between
/// `darkest_pitch` and `brightest_pitch` (MIDI note numbers); 14 renders as
/// an outline; 15 never reaches here (ignored by the tracker).
fn channel_color(channel: u8, pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> Vec4 {
    match channel {
        0 => lch(48.0, 45.0, 32.0),   // red
        1 => lch(65.0, 60.0, 68.0),   // orange
        2 => lch(80.0, 42.0, 83.0),   // yellow
        3 => lch(65.0, 50.0, 120.0),  // green
        4 => lch(60.0, 40.0, 280.0),  // blue
        5 => lch(50.0, 55.0, 305.0),  // purple
        6 => lch(70.0, 30.0, 340.0),  // pink
        7 => lch(80.0, 0.0, 0.0),     // white
        8 => lch(0.0, 0.0, 0.0),      // black
        9..=13 => {
            let t = f64::from(
                (pitch.clamp(darkest_pitch, brightest_pitch) - darkest_pitch)
                    / (brightest_pitch - darkest_pitch).max(0.01),
            );
            lch(
                t * 80.0,
                85.0 - t * 60.0,
                (-100.0 + t * 190.0).rem_euclid(360.0),
            )
        }
        // Channel 14: drawn as an outline; the color is a bright neutral.
        _ => Vec4::new(0.85, 0.85, 0.88, 1.0),
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
        let mut outlined = false;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let a = voice.activation(now, view.highlight_time);
                if a > activation {
                    activation = a;
                    color = channel_color(
                        voice.channel,
                        voice.pitch,
                        view.darkest_pitch,
                        view.brightest_pitch,
                    );
                    outlined = voice.channel == 14;
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
            outlined,
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
    fn pitch_colored_channels_vary_with_pitch() {
        let mut tracker = NoteTracker::new();
        for note in [24, 108] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 9,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let low = channel_color(9, 24.0, 24.0, 108.0);
        let high = channel_color(9, 108.0, 24.0, 108.0);
        assert_ne!(low, high);
        // Brightest pitch should be, well, brighter.
        assert!(high.truncate().length() > low.truncate().length());
    }

    #[test]
    fn channel_14_voices_render_outlined() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 14,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene = derive_scene(
            &tracker,
            &Tuning::default(),
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
        assert!(origin.outlined);
    }

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
