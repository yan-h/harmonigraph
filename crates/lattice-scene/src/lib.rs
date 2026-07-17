//! The scene layer: turns core state (note tracker + tuning) into a
//! render-friendly description, once per frame. The renderer consumes a
//! [`Scene`] and knows nothing about MIDI; the core knows nothing about
//! cameras or colors. Animation/envelope *policy* lives here.

pub mod skin;

use glam::{Mat4, Vec2, Vec3, Vec4};
use lattice_core::{coords, ChannelRole, LatticePos, NoteTracker, Tuning};

/// Axis mapping, matching v1's orientation: major thirds run horizontally
/// (x), fifths vertically (y), and harmonic sevenths in depth (z).
fn lattice_to_world(pos: LatticePos, spacing: f32) -> Vec3 {
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

/// How held/active notes are rendered. All styles share the same instance
/// data (activation + per-note phase); the fragment/vertex shader switches
/// on a uniform. Kept as switchable candidates for live comparison — idle
/// nodes look identical in every style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NodeStyle {
    /// The original look: steady disc + glow.
    #[default]
    Steady,
    /// Held nodes pulse size/glow on per-note phases.
    Breathe,
    /// Noise-driven flame edge around held discs.
    Corona,
    /// Bright particles orbiting held nodes.
    Sparks,
    /// Held nodes become slowly tumbling wireframe octahedra.
    Wire,
}

impl NodeStyle {
    /// Index used by the shader (uniform `misc.w`).
    pub fn shader_index(self) -> u32 {
        match self {
            NodeStyle::Steady => 0,
            NodeStyle::Breathe => 1,
            NodeStyle::Corona => 2,
            NodeStyle::Sparks => 3,
            NodeStyle::Wire => 4,
        }
    }
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
    /// Extent of the displayed lattice along each axis (± steps around the
    /// center).
    pub extent_threes: i32,
    pub extent_fives: i32,
    pub extent_sevens: i32,
    /// Center of the displayed window, in lattice steps from C (v1's Grid
    /// X/Y/Z). The center node renders at the world origin, so panning the
    /// window doesn't walk the content away from the camera.
    #[serde(default)]
    pub center_threes: i32,
    #[serde(default)]
    pub center_fives: i32,
    #[serde(default)]
    pub center_sevens: i32,
    /// Seconds a released note's pitch class keeps fading (mirrors the
    /// plugin parameter; the shell copies it in each frame).
    pub pitch_class_fade_time: f32,
    /// Seconds an octave indicator keeps fading after release; independent
    /// of the note highlight. serde(default) keeps older blobs loadable.
    #[serde(default = "default_octave_fade")]
    pub octave_fade_time: f32,
    /// How nodes indicate sounding octaves.
    pub octave_style: OctaveStyle,
    /// Pitch (MIDI note) mapped to the darkest gradient color on
    /// pitch-colored channels (9-13). Mirrors a plugin parameter.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
    /// Draw note-name labels on hovered and sounding nodes.
    /// serde(default) keeps older persisted blobs loadable.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    /// How held notes are rendered (see NodeStyle).
    #[serde(default)]
    pub node_style: NodeStyle,
    /// Light up lattice edges between simultaneously sounding adjacent
    /// nodes, so a chord's interval structure renders as geometry.
    #[serde(default)]
    pub show_chord_edges: bool,
}

fn default_true() -> bool {
    true
}

impl ViewConfig {
    /// Every lattice position the view currently displays. ALL consumers
    /// (scene derivation, spectral ticks, notes-pane mapping) must iterate
    /// this same set so "on the lattice" means one thing.
    pub fn visible_positions(&self) -> impl Iterator<Item = LatticePos> {
        coords::positions_within(
            self.center_threes - self.extent_threes..=self.center_threes + self.extent_threes,
            self.center_fives - self.extent_fives..=self.center_fives + self.extent_fives,
            self.center_sevens - self.extent_sevens..=self.center_sevens + self.extent_sevens,
        )
    }

    /// The displayed window's center as a lattice position.
    pub fn center(&self) -> LatticePos {
        LatticePos::new(self.center_threes, self.center_fives, self.center_sevens)
    }
}

fn default_octave_fade() -> f32 {
    1.0
}

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
            extent_threes: 3,
            extent_fives: 3,
            extent_sevens: 0,
            center_threes: 0,
            center_fives: 0,
            center_sevens: 0,
            pitch_class_fade_time: 1.0,
            octave_fade_time: 1.0,
            octave_style: OctaveStyle::default(),
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
            show_labels: true,
            node_style: NodeStyle::default(),
            show_chord_edges: false,
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

/// Octave indicator slots (MIDI octaves 0..=9).
pub const OCTAVE_SLOTS: usize = 10;

/// One lattice node, ready for instanced rendering.
#[derive(Clone, Copy, Debug)]
pub struct NodeInstance {
    pub lattice_pos: LatticePos,
    pub world_pos: Vec3,
    /// RGBA base color (alpha unused for now).
    pub color: Vec4,
    /// 0 = idle, 1 = fully lit. Held notes are 1; released notes decay.
    pub activation: f32,
    /// Per-octave activation (slot = MIDI octave 0..=9, clamped): each
    /// octave's indicator fades on its own voice's envelope, independent of
    /// the node's overall activation.
    pub octaves: [f32; OCTAVE_SLOTS],
    /// Start time (scene clock seconds) of the strongest voice lighting
    /// this node; 0 when idle. Animated node styles derive per-note phase
    /// and age from it.
    pub phase: f32,
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
    pub hovered: bool,
}

/// A glowing beam between two simultaneously sounding, lattice-adjacent
/// nodes (one unit step along exactly one prime axis = one interval).
#[derive(Clone, Copy, Debug)]
pub struct EdgeInstance {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
    /// min of the two nodes' activations: the beam fades with whichever
    /// endpoint fades first.
    pub strength: f32,
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
    pub node_style: NodeStyle,
    /// Chord edges (empty when the toggle is off).
    pub edges: Vec<EdgeInstance>,
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

/// Ported verbatim from v1 (`editor/color.rs`); the channel policy itself
/// lives in [`ChannelRole`]. Gradient channels are colored by pitch height
/// on an LCH ramp between `darkest_pitch` and `brightest_pitch` (MIDI note
/// numbers).
pub fn channel_color(channel: u8, pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> Vec4 {
    match ChannelRole::of(channel) {
        ChannelRole::FixedColor => match channel {
            0 => lch(48.0, 45.0, 32.0),  // red
            1 => lch(65.0, 60.0, 68.0),  // orange
            2 => lch(80.0, 42.0, 83.0),  // yellow
            3 => lch(65.0, 50.0, 120.0), // green
            4 => lch(60.0, 40.0, 280.0), // blue
            5 => lch(50.0, 55.0, 305.0), // purple
            6 => lch(70.0, 30.0, 340.0), // pink
            7 => lch(80.0, 0.0, 0.0),    // white
            _ => lch(0.0, 0.0, 0.0),     // 8: black
        },
        ChannelRole::PitchGradient => {
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
        // Outline voices get a bright neutral (the ring shape is the
        // signal). Ignored never reaches here — the tracker drops it.
        ChannelRole::Outline | ChannelRole::Ignored => Vec4::new(0.85, 0.85, 0.88, 1.0),
    }
}

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
    let center = view.center();

    for pos in view.visible_positions() {
        let node_pc = tuning.pitch_class(pos);

        let mut activation = 0.0f32;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = skin::active_skin().node_idle;
        let mut outlined = false;
        let mut phase = 0.0f32;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let a = voice.activation(now, view.pitch_class_fade_time);
                if a > activation {
                    activation = a;
                    color = channel_color(
                        voice.channel,
                        voice.pitch,
                        view.darkest_pitch,
                        view.brightest_pitch,
                    );
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                    phase = voice.on_time as f32;
                }
                let slot = voice.octave.clamp(0, OCTAVE_SLOTS as i8 - 1) as usize;
                octaves[slot] =
                    octaves[slot].max(voice.activation(now, view.octave_fade_time));
            }
        }

        // World positions are relative to the window center, keeping the
        // displayed region under the camera wherever the window pans.
        let centered = pos - center;
        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos: lattice_to_world(centered, view.spacing),
            color,
            activation,
            octaves,
            phase,
            outlined,
            hovered: hovered == Some(pos),
        });
    }

    // Chord edges: every pair of active nodes exactly one lattice step
    // apart gets a beam. O(active^2), and active counts are tiny.
    let mut edges = Vec::new();
    if view.show_chord_edges {
        let active: Vec<&NodeInstance> =
            nodes.iter().filter(|n| n.activation > 0.0).collect();
        for (i, a) in active.iter().enumerate() {
            for b in &active[i + 1..] {
                if a.lattice_pos.is_adjacent(b.lattice_pos) {
                    edges.push(EdgeInstance {
                        a: a.world_pos,
                        b: b.world_pos,
                        color: (a.color + b.color) * 0.5,
                        strength: a.activation.min(b.activation),
                    });
                }
            }
        }
    }

    Scene {
        nodes,
        camera,
        time: now as f32,
        node_radius: view.spacing * 0.25,
        octave_style: view.octave_style,
        node_style: view.node_style,
        edges,
    }
}

impl Scene {
    /// Project a world position into viewport pixels (origin top-left).
    /// `None` when behind the camera.
    pub fn project(&self, viewport_px: Vec2, world: Vec3) -> Option<Vec2> {
        let view_proj = self.camera.view_proj(viewport_px.x / viewport_px.y.max(1.0));
        let clip = view_proj * world.extend(1.0);
        if clip.w <= 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * viewport_px.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * viewport_px.y,
        ))
    }

    /// CPU picking: the node whose screen projection is nearest the pointer,
    /// within `max_px`. Every pane that wants "hover a pitch class" uses
    /// this and writes the result to the shared UI state.
    pub fn pick(&self, viewport_px: Vec2, pointer_px: Vec2, max_px: f32) -> Option<LatticePos> {
        let mut best: Option<(f32, LatticePos)> = None;
        for node in &self.nodes {
            let Some(px) = self.project(viewport_px, node.world_pos) else {
                continue;
            };
            let d = px.distance(pointer_px);
            if d <= max_px && best.is_none_or(|(bd, _)| d < bd) {
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
    fn octaves_fade_independently() {
        // Hold C4, tap-and-release C5: the octave-5 indicator must decay on
        // its own envelope even though the node stays fully active.
        let mut tracker = NoteTracker::new();
        for (note, kind) in [
            (60, NoteEventKind::On { velocity: 1.0 }), // C4 held
            (72, NoteEventKind::On { velocity: 1.0 }), // C5 tapped...
        ] {
            tracker.handle_event(NoteEvent { time: 0.0, channel: 0, note, kind });
        }
        tracker.handle_event(NoteEvent {
            time: 0.1,
            channel: 0,
            note: 72,
            kind: NoteEventKind::Off, // ...and released
        });

        // Half a pitch_class_fade_time after the release.
        let view = ViewConfig { pitch_class_fade_time: 1.0, ..ViewConfig::default() };
        let scene = derive_scene(&tracker, &Tuning::default(), &view, Camera::default(), None, 0.6);
        let origin = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::ORIGIN)
            .unwrap();
        assert_eq!(origin.activation, 1.0, "node stays lit by the held C4");
        assert_eq!(origin.octaves[4], 1.0, "held octave at full");
        assert!(
            origin.octaves[5] > 0.0 && origin.octaves[5] < 0.75,
            "released octave mid-fade, got {}",
            origin.octaves[5]
        );
    }

    #[test]
    fn octave_fade_time_is_independent_of_note_fade() {
        // Note fade short, octave fade long: after the note highlight ends,
        // the disc goes idle but the octave indicator is still fading.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::Off,
        });
        let view = ViewConfig {
            pitch_class_fade_time: 0.2,
            octave_fade_time: 2.0,
            ..ViewConfig::default()
        };
        // Prune with the longer of the two, as root_ui does.
        tracker.prune(1.0, view.pitch_class_fade_time.max(view.octave_fade_time));
        let scene =
            derive_scene(&tracker, &Tuning::default(), &view, Camera::default(), None, 1.0);
        let origin = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::ORIGIN)
            .unwrap();
        assert_eq!(origin.activation, 0.0, "pitch class fade has ended");
        assert!(
            origin.octaves[4] > 0.0 && origin.octaves[4] < 0.75,
            "octave still mid-fade, got {}",
            origin.octaves[4]
        );
    }

    #[test]
    fn window_center_pans_which_nodes_display() {
        let view = ViewConfig {
            center_threes: 5,
            extent_threes: 1,
            extent_fives: 0,
            extent_sevens: 0,
            ..ViewConfig::default()
        };
        let positions: Vec<_> = view.visible_positions().collect();
        assert_eq!(
            positions,
            vec![
                LatticePos::new(4, 0, 0),
                LatticePos::new(5, 0, 0),
                LatticePos::new(6, 0, 0)
            ]
        );

        // The center node renders at the world origin.
        let tracker = NoteTracker::new();
        let scene =
            derive_scene(&tracker, &Tuning::default(), &view, Camera::default(), None, 0.0);
        let center_node = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::new(5, 0, 0))
            .unwrap();
        assert_eq!(center_node.world_pos, Vec3::ZERO);
    }

    #[test]
    fn chord_edges_connect_adjacent_active_nodes() {
        // Just intonation makes lattice pitch classes unique within the
        // default extents (under 12-TET, enharmonic duplicate nodes light
        // up too and produce parallel edges - correct, but noisy to pin).
        // C and G (a fifth apart, one step on the prime-3 axis) held
        // together with a wide-enough tolerance: exactly one edge.
        let tuning = Tuning { tolerance: 5.0, ..Tuning::just() };
        let mut tracker = NoteTracker::new();
        for note in [60u8, 67] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let view = ViewConfig { show_chord_edges: true, ..ViewConfig::default() };
        let scene = derive_scene(&tracker, &tuning, &view, Camera::default(), None, 0.0);
        assert_eq!(scene.edges.len(), 1);
        assert_eq!(scene.edges[0].strength, 1.0);

        // C and F# (nothing within a step and 5 cents): no edge.
        let mut tracker = NoteTracker::new();
        for note in [60u8, 66] {
            tracker.handle_event(NoteEvent {
                time: 0.0,
                channel: 0,
                note,
                kind: NoteEventKind::On { velocity: 1.0 },
            });
        }
        let scene = derive_scene(&tracker, &tuning, &view, Camera::default(), None, 0.0);
        assert_eq!(scene.edges.len(), 0);
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
        assert_eq!(origin.octaves[4], 1.0);
    }
}
