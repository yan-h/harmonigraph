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
/// The fragment shader draws the glyphs from the per-node octave bitmask.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum OctaveStyle {
    /// No octave indication.
    Off,
    /// Small dots around the disc; angle tracks absolute pitch (middle C
    /// straight up, 45deg clockwise per octave, pitch class within the
    /// octave included). All other styles keep this angle convention and
    /// change only the glyph shape.
    #[default]
    Dots,
    /// Teardrop petals rooted just inside the rim, growing out of the
    /// note like a flower.
    Petals,
    /// Plumes erupting from the rim, flickering in length.
    Flares,
    /// Blobs seated on the rim so the disc outline bulges at each pitch
    /// angle.
    Bumps,
    /// Annular pizza-slice sectors filling the ring around the disc, with
    /// a gap ring off the rim and constant-thickness gaps between
    /// neighbors.
    Slices,
}

/// How held/active notes are rendered. All styles share the same instance
/// data (activation + per-note phase); the fragment/vertex shader switches
/// on a uniform. Kept as switchable candidates for live comparison — idle
/// nodes look identical in every style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum NodeStyle {
    /// The original look: steady disc + glow. The aliases absorb styles
    /// that used to exist (Breathe, Sparks) so persisted view blobs that
    /// still name them keep loading.
    #[default]
    #[serde(alias = "Breathe", alias = "Sparks")]
    Steady,
    /// Held nodes become slowly tumbling wireframe octahedra.
    Wire,
    /// Held discs become billowing balls of gas ringed by a flame edge,
    /// with every sounding octave's color swirled through the interior.
    Corona,
    /// Gas ball variant: octave colors sheared into rotating spiral
    /// streaks, like stirred paint.
    Vortex,
    /// Gas ball variant: fast boiling granulation cells over larger octave
    /// color patches, with sparse prominences at the rim.
    Plasma,
    /// Gas ball variant: octave colors as slowly drifting curtains.
    Aurora,
    /// Gas ball variant: thin turbulent veins sweeping through the octave
    /// colors.
    Marble,
    /// Gas ball variant: big soft blobs of single octave colors wandering
    /// lava-lamp style.
    Lava,
    /// Gas ball variant: glowing ridged-noise threads over dark gas.
    Filament,
    /// Pattern variant: soft color waves wrapping the sphere around a
    /// per-node axis, slowly revolving.
    Stripes,
    /// Pattern variant: soft color rings radiating from the face center,
    /// bunching toward the limb.
    Rings,
    /// Pattern variant: beach-ball sectors around a tilted pole, slowly
    /// turning.
    Pinwheel,
    /// Pattern variant: two-armed spiral of color waves hugging the
    /// sphere.
    Spiral,
    /// Pattern variant: soft checkerboard on the globe graticule.
    Checker,
    /// Pattern variant: rounded glowing tiles on a slowly revolving
    /// globe, over dim gaps.
    Tiles,
}

impl NodeStyle {
    /// Index used by the shader (uniform `misc.w`).
    pub fn shader_index(self) -> u32 {
        match self {
            NodeStyle::Steady => 0,
            NodeStyle::Wire => 1,
            NodeStyle::Corona => 2,
            NodeStyle::Vortex => 3,
            NodeStyle::Plasma => 4,
            NodeStyle::Aurora => 5,
            NodeStyle::Marble => 6,
            NodeStyle::Lava => 7,
            NodeStyle::Filament => 8,
            NodeStyle::Stripes => 9,
            NodeStyle::Rings => 10,
            NodeStyle::Pinwheel => 11,
            NodeStyle::Spiral => 12,
            NodeStyle::Checker => 13,
            NodeStyle::Tiles => 14,
        }
    }

    /// The field family — everything except Steady and Wire: styles whose
    /// active discs paint the swirled octave-color field (noise-driven gas
    /// or deterministic patterns). These animate on global time with a
    /// stable per-node seed (see [`derive_scene`]), so note events never
    /// restart the pattern. Mirrors `is_field_style` in lattice.wgsl; keep
    /// in sync.
    pub fn is_field_style(self) -> bool {
        !matches!(self, NodeStyle::Steady | NodeStyle::Wire)
    }
}

impl OctaveStyle {
    /// Index used by the shader (uniform `misc.z`).
    pub fn shader_index(self) -> u32 {
        match self {
            OctaveStyle::Off => 0,
            OctaveStyle::Dots => 1,
            OctaveStyle::Petals => 2,
            OctaveStyle::Flares => 3,
            OctaveStyle::Bumps => 4,
            OctaveStyle::Slices => 5,
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
    /// How nodes indicate sounding octaves.
    pub octave_style: OctaveStyle,
    /// Draw note-name labels on hovered and sounding nodes.
    /// serde(default) keeps older persisted blobs loadable.
    #[serde(default = "default_true")]
    pub show_labels: bool,
    /// Under each note-name label, also show the node's pitch class in
    /// cents. Only meaningful while `show_labels` is on.
    #[serde(default = "default_true")]
    pub show_cents: bool,
    /// How held notes are rendered (see NodeStyle).
    #[serde(default)]
    pub node_style: NodeStyle,
    /// Light up lattice edges between simultaneously sounding adjacent
    /// nodes, so a chord's interval structure renders as geometry.
    #[serde(default)]
    pub show_chord_edges: bool,
    /// Meantone mode: lock the major-third tuning to four perfect fifths
    /// (temper out the syntonic comma). While on, the third-tuning value is
    /// derived from the fifth (in `root_ui`) and note-name labels drop
    /// their comma marks; Learn mode toggles this from the held chord.
    #[serde(default)]
    pub meantone: bool,
    /// Offscreen render resolution as a multiple of the pane's native pixel
    /// size: >1 supersamples (crisper glyph edges), <1 renders coarse and
    /// upscales. 1.0 reproduces the pre-offscreen-pass output exactly.
    #[serde(default = "default_render_scale")]
    pub render_scale: f32,
}

fn default_true() -> bool {
    true
}

fn default_render_scale() -> f32 {
    1.0
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

impl Default for ViewConfig {
    fn default() -> Self {
        ViewConfig {
            spacing: 1.0,
            // A tall window of fifths and a wide band of thirds out of the
            // box; sevenths stay opt-in.
            extent_threes: 10,
            extent_fives: 6,
            extent_sevens: 0,
            center_threes: 0,
            center_fives: 0,
            center_sevens: 0,
            octave_style: OctaveStyle::default(),
            show_labels: true,
            show_cents: true,
            node_style: NodeStyle::default(),
            show_chord_edges: false,
            meantone: false,
            render_scale: default_render_scale(),
        }
    }
}

/// Per-frame mirrors of the host-automatable appearance parameters. The
/// shell copies these from its param backend every frame (see root_ui).
/// Deliberately NOT part of [`ViewConfig`] or the persist blob: the param
/// system owns these values, and persisting a copy would create a second
/// source of truth that's dead on arrival at load time.
#[derive(Clone, Copy, Debug)]
pub struct FrameParams {
    /// Seconds a released note's pitch class keeps fading.
    pub pitch_class_fade_time: f32,
    /// Seconds an octave indicator keeps fading after release; independent
    /// of the note highlight.
    pub octave_fade_time: f32,
    /// Pitch (MIDI note) mapped to the darkest gradient color on
    /// pitch-gradient channels.
    pub darkest_pitch: f32,
    /// Pitch mapped to the brightest gradient color.
    pub brightest_pitch: f32,
}

impl Default for FrameParams {
    fn default() -> Self {
        FrameParams {
            pitch_class_fade_time: 1.0,
            octave_fade_time: 1.0,
            darkest_pitch: 24.0,
            brightest_pitch: 108.0,
        }
    }
}

/// How the camera maps depth to the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Projection {
    /// Classic perspective: farther content converges and shrinks.
    #[default]
    Perspective,
    /// Orthographic ("isometric-style"): uniform scale at every depth, so
    /// equal intervals render at equal screen offsets everywhere and
    /// parallel lattice lines stay parallel. Depth then reads only
    /// through the deliberate cues (node depth-scale, occlusion).
    Orthographic,
    /// Cabinet (oblique): the camera faces the fifths/thirds sheet
    /// straight on (orbit is ignored; the UI turns plain drags into
    /// pans), which renders that primary plane with zero distortion, and
    /// the sevens axis shears to a uniform screen offset — every
    /// seventh-step is the same arrow anywhere on screen. Direction and
    /// length of that arrow are [`Camera::cabinet_angle`] and
    /// [`Camera::cabinet_scale`].
    Cabinet,
}

fn default_cabinet_angle() -> f32 {
    45f32.to_radians()
}

fn default_cabinet_scale() -> f32 {
    0.5
}

/// Simple orbit camera. Angles in radians.
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub fov_y: f32,
    /// serde(default) keeps pre-projection persisted blobs loadable.
    #[serde(default)]
    pub projection: Projection,
    /// Cabinet only: on-screen direction of the sevens axis, radians
    /// counterclockwise from screen-right (drafting convention picks 30°,
    /// 45°, or 60°; default 45°).
    #[serde(default = "default_cabinet_angle")]
    pub cabinet_angle: f32,
    /// Cabinet only: screen length of one seventh-step as a fraction of a
    /// front-plane step (0.5 = classic cabinet, 1.0 = cavalier).
    #[serde(default = "default_cabinet_scale")]
    pub cabinet_scale: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: Vec3::ZERO,
            yaw: 0.4,
            pitch: 0.3,
            distance: 12.0,
            fov_y: 45f32.to_radians(),
            projection: Projection::default(),
            cabinet_angle: default_cabinet_angle(),
            cabinet_scale: default_cabinet_scale(),
        }
    }
}

impl Camera {
    /// Yaw/pitch radians per dragged pixel.
    const ORBIT_SPEED: f32 = 0.01;
    /// Pan speed per pixel, scaled by distance so a pan feels the same at
    /// any zoom.
    const PAN_SPEED: f32 = 0.0016;
    /// Multiplicative zoom rate per scroll unit.
    const ZOOM_RATE: f32 = 0.002;
    /// Keep pitch shy of the poles so look_at's up vector stays valid.
    pub const PITCH_LIMIT: f32 = 1.5;
    /// Zoom clamp; CLIP_NEAR/CLIP_FAR must bracket it with margin.
    pub const MIN_DISTANCE: f32 = 2.0;
    pub const MAX_DISTANCE: f32 = 80.0;

    /// Orbit around the target by a drag delta in pixels.
    pub fn orbit(&mut self, delta: Vec2) {
        self.yaw -= delta.x * Self::ORBIT_SPEED;
        self.pitch = (self.pitch + delta.y * Self::ORBIT_SPEED)
            .clamp(-Self::PITCH_LIMIT, Self::PITCH_LIMIT);
    }

    /// Pan the target by a drag delta in pixels, grab-style: the content
    /// follows the pointer.
    pub fn pan(&mut self, delta: Vec2) {
        let (right, up) = self.right_up();
        let k = self.distance * Self::PAN_SPEED;
        self.target += up * (delta.y * k) - right * (delta.x * k);
    }

    /// Zoom by a scroll delta (positive = closer), clamped to the working
    /// range.
    pub fn zoom(&mut self, scroll: f32) {
        self.distance = (self.distance * (1.0 - scroll * Self::ZOOM_RATE))
            .clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    pub fn eye(&self) -> Vec3 {
        // Cabinet is a fixed-viewpoint projection: the eye always faces
        // the fifths/thirds sheet straight on, whatever yaw/pitch say (they
        // keep their values for when the user switches back).
        let dir = if self.projection == Projection::Cabinet {
            Vec3::Z
        } else {
            Vec3::new(
                self.pitch.cos() * self.yaw.sin(),
                self.pitch.sin(),
                self.pitch.cos() * self.yaw.cos(),
            )
        };
        self.target + dir * self.distance
    }

    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye(), self.target, Vec3::Y)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        // Both glam _rh constructors produce 0..1 clip-space depth, which
        // is what wgpu expects.
        let aspect = aspect.max(0.01);
        let proj = match self.projection {
            Projection::Perspective => {
                Mat4::perspective_rh(self.fov_y, aspect, CLIP_NEAR, CLIP_FAR)
            }
            // The ortho window is the perspective frustum's cross-section
            // at the target, so toggling projections keeps the framing at
            // the focus plane, and zoom (distance) keeps scaling the view.
            Projection::Orthographic => {
                let half_h = self.distance * (self.fov_y * 0.5).tan();
                let half_w = half_h * aspect;
                Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, CLIP_NEAR, CLIP_FAR)
            }
            // Orthographic window plus a shear: view-space depth relative
            // to the focus plane becomes a screen offset along
            // `cabinet_angle` scaled by `cabinet_scale`. The focus plane
            // itself is unmoved (the shear's translation term cancels
            // there), so framing still matches the other projections.
            Projection::Cabinet => {
                let half_h = self.distance * (self.fov_y * 0.5).tan();
                let half_w = half_h * aspect;
                let ortho =
                    Mat4::orthographic_rh(-half_w, half_w, -half_h, half_h, CLIP_NEAR, CLIP_FAR);
                let (sin, cos) = self.cabinet_angle.sin_cos();
                let (kx, ky) = (self.cabinet_scale * cos, self.cabinet_scale * sin);
                let mut shear = Mat4::IDENTITY;
                shear.z_axis.x = kx;
                shear.z_axis.y = ky;
                shear.w_axis.x = kx * self.distance;
                shear.w_axis.y = ky * self.distance;
                ortho * shear
            }
        };
        proj * self.view()
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

/// Camera clip planes; must bracket Camera::MIN/MAX_DISTANCE with margin.
const CLIP_NEAR: f32 = 0.1;
const CLIP_FAR: f32 = 200.0;

/// Node radius as a fraction of the lattice spacing.
const NODE_RADIUS_FACTOR: f32 = 0.25;

/// Octave indicator slots (MIDI octaves 0..=9).
pub const OCTAVE_SLOTS: usize = 10;

/// Seconds an octave indicator eases in after note-on. Keeps a fresh
/// octave's color GROWING into the gas swirl instead of instantly
/// repainting its share of the disc (and softens dots-mode pop-in);
/// short enough to still feel immediate.
const OCTAVE_ATTACK_TIME: f64 = 0.15;

/// Samples in the pitch->color lookup the dots octave style uses to tint
/// each dot by its own octave's pitch. The shader mirrors this length.
pub const DOT_RAMP_N: usize = 16;

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
    /// Seconds since the strongest voice lighting this node started; 0
    /// when idle. Computed in f64 and handed to the shader small: absolute
    /// f32 seconds lose millisecond precision within a day of DAW uptime,
    /// which would visibly quantize the animation.
    pub age: f32,
    /// Small constant seeding animation variety. NOT a timestamp — only
    /// ever used as a seed. Per-note (derived from the note-on time,
    /// wrapped) for age-driven styles; a stable per-node hash for field
    /// styles (see [`NodeStyle::is_field_style`]).
    pub seed: f32,
    /// Render as an outline instead of a filled disc (channel 14, v1's
    /// "channel 15" in MIDI convention).
    pub outlined: bool,
    pub hovered: bool,
    /// Depth-cue size multiplier (see [`depth_scale`]): nodes nearer the
    /// eye than the camera's focus distance grow, farther ones shrink,
    /// exaggerating the perspective so depth reads at a glance.
    pub scale: f32,
    /// On the home (center sevens) sheet. Home nodes keep a blank
    /// placeholder ring while idle; off-sheet nodes draw nothing.
    pub on_home: bool,
    /// The node's pitch class in cents under the current tuning, for the
    /// in-lattice cents readout.
    pub cents: f32,
}

/// A glowing beam between two simultaneously sounding, lattice-adjacent
/// nodes (one unit step along exactly one prime axis = one interval).
#[derive(Clone, Copy, Debug)]
pub struct EdgeInstance {
    pub a: Vec3,
    pub b: Vec3,
    pub color: Vec4,
    /// min of the two nodes' activations: the beam fades with whichever
    /// endpoint fades first. (Grid segments: line opacity.)
    pub strength: f32,
    /// Grid segments only: render as short dashes (the sevens-axis links
    /// between sheets). Never set on chord beams.
    pub dashed: bool,
}

/// Everything the renderer needs for one frame.
pub struct Scene {
    pub nodes: Vec<NodeInstance>,
    pub camera: Camera,
    /// Seconds for global shader animation, wrapped hourly so f32
    /// precision holds in long sessions. The field styles clock on this so
    /// their fields keep flowing across note events (at worst the pattern
    /// jumps once an hour at the wrap); age-driven styles use
    /// [`NodeInstance`]'s `age`/`seed` instead (unwrapped age math).
    pub time: f32,
    /// Base node radius in world units (scales with lattice spacing).
    pub node_radius: f32,
    pub octave_style: OctaveStyle,
    pub node_style: NodeStyle,
    /// Chord edges (empty when the toggle is off).
    pub edges: Vec<EdgeInstance>,
    /// The faint background grid (see [`derive_grid`]): one segment per
    /// adjacent pair of visible positions, inset so every node position
    /// keeps a circular gap where its disc draws while sounding. Segments
    /// between two sounding notes light up with their blended color.
    /// Reuses [`EdgeInstance`]; `strength` carries the line opacity.
    pub grid: Vec<EdgeInstance>,
    /// Pitch->color lookup for the dots octave style, matching the disc
    /// gradient; the renderer hands it to the shader (see [`pitch_ramp_lut`]).
    pub dot_ramp: [Vec4; DOT_RAMP_N],
    /// Gradient endpoints (MIDI notes) the shader maps a dot's pitch through
    /// to index `dot_ramp`; mirror the disc coloring's `FrameParams`.
    pub darkest_pitch: f32,
    pub brightest_pitch: f32,
    /// Offscreen render resolution multiplier (see [`ViewConfig`]); the
    /// renderer sizes its offscreen color+depth target by this.
    pub render_scale: f32,
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

/// Normalized pitch height in 0..1 across the gradient range: 0 at
/// `darkest_pitch`, 1 at `brightest_pitch` (both MIDI note numbers).
fn pitch_ramp_t(pitch: f32, darkest_pitch: f32, brightest_pitch: f32) -> f64 {
    f64::from(
        (pitch.clamp(darkest_pitch, brightest_pitch) - darkest_pitch)
            / (brightest_pitch - darkest_pitch).max(0.01),
    )
}

/// The pitch-gradient LCH ramp as a function of normalized height `t`
/// (0..1). Shared by the node disc color and the dots octave style's
/// per-dot tint, so a dot is the same color as the disc that pitch lights.
fn pitch_ramp_lch(t: f64) -> Vec4 {
    lch(t * 80.0, 85.0 - t * 60.0, (-100.0 + t * 190.0).rem_euclid(360.0))
}

/// The pitch ramp sampled into [`DOT_RAMP_N`] colors evenly spaced over the
/// full `t` range, for the shader's per-dot color lookup (the shader maps a
/// dot's pitch to a `t` and indexes this). Endpoints of the disc gradient
/// are applied shader-side, so this LUT itself is range-independent.
pub fn pitch_ramp_lut() -> [Vec4; DOT_RAMP_N] {
    std::array::from_fn(|k| pitch_ramp_lch(k as f64 / (DOT_RAMP_N - 1) as f64))
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
            pitch_ramp_lch(pitch_ramp_t(pitch, darkest_pitch, brightest_pitch))
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
    frame: &FrameParams,
    camera: Camera,
    hovered: Option<LatticePos>,
    now: f64,
) -> Scene {
    let mut nodes = Vec::new();
    let center = view.center();
    let eye = camera.eye();

    for pos in view.visible_positions() {
        let node_pc = tuning.pitch_class(pos);

        let mut activation = 0.0f32;
        let mut octaves = [0f32; OCTAVE_SLOTS];
        let mut color = skin::active_skin().node_idle;
        let mut outlined = false;
        let mut age = 0.0f32;
        let mut seed = 0.0f32;

        // O(nodes × voices); fine at this scale. If extents grow large,
        // index voices by quantized pitch class instead.
        for voice in tracker.voices() {
            if tuning.matches(voice.pitch_class, node_pc) {
                let a = voice.activation(now, frame.pitch_class_fade_time);
                if a > activation {
                    activation = a;
                    color = channel_color(
                        voice.channel,
                        voice.pitch,
                        frame.darkest_pitch,
                        frame.brightest_pitch,
                    );
                    outlined = ChannelRole::of(voice.channel) == ChannelRole::Outline;
                    // Subtract in f64, then narrow: both endpoints are
                    // large after long sessions; only the difference is
                    // safe as f32.
                    age = (now - voice.on_time).max(0.0) as f32;
                    seed = (voice.on_time % 256.0) as f32;
                }
                let slot = voice.octave.clamp(0, OCTAVE_SLOTS as i8 - 1) as usize;
                // Smoothstep ease-in over the first OCTAVE_ATTACK_TIME;
                // release still fades on the octave envelope.
                let a = ((now - voice.on_time) / OCTAVE_ATTACK_TIME).clamp(0.0, 1.0) as f32;
                let attack = a * a * (3.0 - 2.0 * a);
                octaves[slot] = octaves[slot]
                    .max(voice.activation(now, frame.octave_fade_time) * attack);
            }
        }

        // Field styles animate one continuous field per node — global time
        // as the clock plus a stable per-node seed — so pressing,
        // retriggering, or stacking notes lights the flow up without ever
        // restarting or reshuffling it. Age-driven styles keep the
        // per-note seed (wire's tumble phases).
        let seed = if view.node_style.is_field_style() { node_seed(pos) } else { seed };

        // World positions are relative to the window center, keeping the
        // displayed region under the camera wherever the window pans.
        let centered = pos - center;
        let world_pos = lattice_to_world(centered, view.spacing);
        nodes.push(NodeInstance {
            lattice_pos: pos,
            world_pos,
            color,
            activation,
            octaves,
            age,
            seed,
            outlined,
            hovered: hovered == Some(pos),
            scale: depth_scale(world_pos.distance(eye), camera.distance),
            on_home: pos.sevens == view.center_sevens,
            cents: node_pc.to_cents(),
        });
    }

    let edges = if view.show_chord_edges { derive_edges(&nodes) } else { Vec::new() };
    let grid = derive_grid(view, &nodes);

    Scene {
        nodes,
        camera,
        time: (now % 3600.0) as f32,
        node_radius: view.spacing * NODE_RADIUS_FACTOR,
        octave_style: view.octave_style,
        node_style: view.node_style,
        edges,
        grid,
        dot_ramp: pitch_ramp_lut(),
        darkest_pitch: frame.darkest_pitch,
        brightest_pitch: frame.brightest_pitch,
        render_scale: view.render_scale,
    }
}

/// Depth-cue strength: the exponent on (focus distance / node distance)
/// that sets a node's size multiplier. 0 would disable the cue (plain
/// perspective); 1 roughly doubles perspective's own shrink-with-distance.
const DEPTH_SCALE_EXPONENT: f32 = 0.8;
/// Clamp on the multiplier so nodes stay recognizable when the camera
/// gets very close to (or very far from) part of the lattice.
const DEPTH_SCALE_RANGE: (f32, f32) = (0.4, 2.0);

/// Depth-cue size multiplier for a node `dist` from the eye, with the
/// camera focused (eye-to-target) at `focus`: 1 at the focus distance, so
/// the lattice's overall look is unchanged where the user is looking;
/// larger when nearer, smaller when farther. Perspective alone shrinks a
/// distant node too subtly for depth to read at lattice scale — this
/// exaggerates it.
fn depth_scale(dist: f32, focus: f32) -> f32 {
    (focus / dist.max(0.01))
        .powf(DEPTH_SCALE_EXPONENT)
        .clamp(DEPTH_SCALE_RANGE.0, DEPTH_SCALE_RANGE.1)
}

/// How far a grid segment stops short of each node center, as a factor of
/// the node radius. Larger than the disc's visual radius (~0.83 × radius,
/// see the quad math in lattice.wgsl) so the gap fully contains the circle
/// a sounding note draws there, with a slim margin.
const GRID_INSET_FACTOR: f32 = 1.05;

/// Line opacity of a grid segment whose two endpoint notes both sound.
const GRID_LIT_OPACITY: f32 = 0.85;

/// The faint background grid: idle positions draw no disc, so these
/// segments carry the lattice's structure instead, inset at both ends so
/// each node position keeps a clear circular gap. Only the home (center)
/// sheet draws an idle grid; other sheets' lines light when both
/// endpoints sound, and the dashed sevens-axis links light as the chain
/// from any sounding off-sheet note down to the home sheet. Lit segments
/// take the sounding notes' color and fade with their envelope.
fn derive_grid(view: &ViewConfig, nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let inset = view.spacing * NODE_RADIUS_FACTOR * GRID_INSET_FACTOR;
    let base = skin::active_skin().grid_line;
    let index: std::collections::HashMap<LatticePos, &NodeInstance> =
        nodes.iter().map(|n| (n.lattice_pos, n)).collect();
    let mut grid = Vec::new();
    for node in nodes {
        let p = node.lattice_pos;
        // +1 steps only, so each undirected pair appears once; positions
        // outside the window simply miss the index.
        for (axis, step) in [
            LatticePos::new(p.threes + 1, p.fives, p.sevens),
            LatticePos::new(p.threes, p.fives + 1, p.sevens),
            LatticePos::new(p.threes, p.fives, p.sevens + 1),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(neighbor) = index.get(&step) else {
                continue;
            };
            let along_sevens = axis == 2;
            // Only the home (center) sheet draws an idle grid; other
            // sheets' lines and the links between sheets stay invisible
            // until the music lights them. Links render dashed.
            let on_home = !along_sevens && p.sevens == view.center_sevens;
            let idle = if on_home { base.w } else { 0.0 };
            let dashed = along_sevens;

            // Both endpoints sounding lights any segment.
            let mut lit = node.activation.min(neighbor.activation);
            let mut lit_color = (node.color + neighbor.color) * 0.5;

            // A sevens link also lights as part of the chain from any
            // sounding node beyond it (away from the home sheet) down to
            // the home sheet, so an off-sheet note always hangs from a
            // visible chain even while the notes under it are silent.
            if along_sevens {
                let (lo, hi) = if p.sevens >= view.center_sevens {
                    (p.sevens + 1, view.center_sevens + view.extent_sevens)
                } else {
                    (view.center_sevens - view.extent_sevens, p.sevens)
                };
                for s in lo..=hi {
                    if let Some(n) = index.get(&LatticePos::new(p.threes, p.fives, s)) {
                        if n.activation > lit {
                            lit = n.activation;
                            lit_color = n.color;
                        }
                    }
                }
            }

            // Fully invisible: skip the instance instead of shipping a
            // discarded quad.
            if idle <= 0.0 && lit <= 0.0 {
                continue;
            }
            let dir = (neighbor.world_pos - node.world_pos).normalize_or_zero();
            grid.push(EdgeInstance {
                a: node.world_pos + dir * inset,
                b: neighbor.world_pos - dir * inset,
                color: base.lerp(lit_color, lit),
                strength: idle + (GRID_LIT_OPACITY - idle) * lit,
                dashed,
            });
        }
    }
    grid
}

/// Stable per-node animation seed for the field styles: a hash of the
/// lattice position folded into the same small range as the per-note
/// seed. A node's gas pattern becomes part of its identity — the same
/// every time it lights, decorrelated from its neighbors'.
fn node_seed(pos: LatticePos) -> f32 {
    let h = pos
        .threes
        .wrapping_mul(731)
        .wrapping_add(pos.fives.wrapping_mul(2683))
        .wrapping_add(pos.sevens.wrapping_mul(9461));
    (h as f32 * 0.618_034).rem_euclid(256.0)
}

/// Chord edges: every pair of active, lattice-adjacent nodes gets a beam.
/// O(active²), and active counts are tiny.
fn derive_edges(nodes: &[NodeInstance]) -> Vec<EdgeInstance> {
    let active: Vec<&NodeInstance> = nodes.iter().filter(|n| n.activation > 0.0).collect();
    let mut edges = Vec::new();
    for (i, a) in active.iter().enumerate() {
        for b in &active[i + 1..] {
            if a.lattice_pos.is_adjacent(b.lattice_pos) {
                edges.push(EdgeInstance {
                    a: a.world_pos,
                    b: b.world_pos,
                    color: (a.color + b.color) * 0.5,
                    strength: a.activation.min(b.activation),
                    dashed: false,
                });
            }
        }
    }
    edges
}

/// Cached projection for one (camera, viewport) pair: build once, then
/// project many points (labels, picking) without rebuilding the
/// view-projection matrix per node.
pub struct Projector {
    view_proj: Mat4,
    viewport_px: Vec2,
}

impl Projector {
    /// Project a world position into viewport pixels (origin top-left).
    /// `None` when behind the camera.
    pub fn project(&self, world: Vec3) -> Option<Vec2> {
        let clip = self.view_proj * world.extend(1.0);
        // Behind the camera (or nearer than the near plane): perspective
        // flips w negative there; orthographic keeps w at 1 and instead
        // sends clip z below the near plane's 0. Test both so the check
        // holds under either projection.
        if clip.w <= 0.0 || clip.z < 0.0 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(Vec2::new(
            (ndc.x * 0.5 + 0.5) * self.viewport_px.x,
            (1.0 - (ndc.y * 0.5 + 0.5)) * self.viewport_px.y,
        ))
    }
}

impl Scene {
    pub fn projector(&self, viewport_px: Vec2) -> Projector {
        Projector {
            view_proj: self.camera.view_proj(viewport_px.x / viewport_px.y.max(1.0)),
            viewport_px,
        }
    }

    /// Convenience for one-off projections; per-node loops should reuse a
    /// [`Scene::projector`].
    pub fn project(&self, viewport_px: Vec2, world: Vec3) -> Option<Vec2> {
        self.projector(viewport_px).project(world)
    }

    /// CPU picking: the node whose screen projection is nearest the pointer,
    /// within `max_px`. Every pane that wants "hover a pitch class" uses
    /// this and writes the result to the shared UI state.
    pub fn pick(&self, viewport_px: Vec2, pointer_px: Vec2, max_px: f32) -> Option<LatticePos> {
        let projector = self.projector(viewport_px);
        let mut best: Option<(f32, LatticePos)> = None;
        for node in &self.nodes {
            let Some(px) = projector.project(node.world_pos) else {
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

    fn scene_of(
        tracker: &NoteTracker,
        tuning: &Tuning,
        view: &ViewConfig,
        frame: &FrameParams,
        now: f64,
    ) -> Scene {
        derive_scene(tracker, tuning, view, frame, Camera::default(), None, now)
    }

    fn origin_node(scene: &Scene) -> &NodeInstance {
        scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::ORIGIN)
            .unwrap()
    }

    #[test]
    fn pitch_colored_channels_vary_with_pitch() {
        let low = channel_color(9, 24.0, 24.0, 108.0);
        let high = channel_color(9, 108.0, 24.0, 108.0);
        assert_ne!(low, high);
        // Brightest pitch should be, well, brighter.
        assert!(high.truncate().length() > low.truncate().length());
    }

    #[test]
    fn dot_ramp_lut_reproduces_the_pitch_gradient() {
        // The dots octave style tints each dot by sampling `pitch_ramp_lut`
        // the way the shader does (linear interp across DOT_RAMP_N entries).
        // Reconstructing that here must land on the disc's gradient color for
        // the same pitch, so a dot is the color of the disc its pitch lights.
        let lut = pitch_ramp_lut();
        let (dark, bright) = (24.0f32, 108.0f32);
        for pitch in [24.0f32, 36.0, 54.0, 60.0, 72.0, 96.0, 108.0] {
            let t = ((pitch - dark) / (bright - dark)).clamp(0.0, 1.0);
            let f = t * (DOT_RAMP_N - 1) as f32;
            let i0 = f.floor() as usize;
            let i1 = (i0 + 1).min(DOT_RAMP_N - 1);
            let lut_color = lut[i0].lerp(lut[i1], f - f.floor());
            // Same pitch through the disc path (channel 9 is pitch-gradient).
            let disc = channel_color(9, pitch, dark, bright);
            assert!(
                (lut_color - disc).truncate().length() < 0.05,
                "pitch {pitch}: lut {lut_color:?} vs disc {disc:?}"
            );
        }
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
        let frame = FrameParams { pitch_class_fade_time: 1.0, ..FrameParams::default() };
        let scene =
            scene_of(&tracker, &Tuning::default(), &ViewConfig::default(), &frame, 0.6);
        let origin = origin_node(&scene);
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
        let frame = FrameParams {
            pitch_class_fade_time: 0.2,
            octave_fade_time: 2.0,
            ..FrameParams::default()
        };
        // Prune with the longer of the two, as root_ui does.
        tracker.prune(1.0, frame.pitch_class_fade_time.max(frame.octave_fade_time));
        let scene =
            scene_of(&tracker, &Tuning::default(), &ViewConfig::default(), &frame, 1.0);
        let origin = origin_node(&scene);
        assert_eq!(origin.activation, 0.0, "pitch class fade has ended");
        assert!(
            origin.octaves[4] > 0.0 && origin.octaves[4] < 0.75,
            "octave still mid-fade, got {}",
            origin.octaves[4]
        );
    }

    #[test]
    fn age_and_seed_derive_from_the_note_on_time() {
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 10.0,
            channel: 0,
            note: 60,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene = scene_of(
            &tracker,
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            12.5,
        );
        let origin = origin_node(&scene);
        assert!((origin.age - 2.5).abs() < 1e-6);
        assert!((origin.seed - 10.0).abs() < 1e-6);
        // Idle nodes carry neutral animation inputs.
        let idle = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::new(1, 1, 0))
            .unwrap();
        assert_eq!((idle.age, idle.seed), (0.0, 0.0));
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
            scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
        let center_node = scene
            .nodes
            .iter()
            .find(|n| n.lattice_pos == LatticePos::new(5, 0, 0))
            .unwrap();
        assert_eq!(center_node.world_pos, Vec3::ZERO);
    }

    #[test]
    fn chord_edges_connect_adjacent_active_nodes() {
        // A deliberately small window (±3/±3): just intonation keeps pitch
        // classes unique at that size, so the edge count is exact. Wider
        // windows bring in schisma near-duplicates — e.g. (8, 1, 0) sits
        // ~2¢ from C, within this test's 5¢ tolerance — which light up and
        // add parallel edges (correct behavior, but noisy to pin; same
        // reason 12-TET's enharmonic duplicates are avoided here).
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
        let view = ViewConfig {
            show_chord_edges: true,
            extent_threes: 3,
            extent_fives: 3,
            ..ViewConfig::default()
        };
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
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
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        assert_eq!(scene.edges.len(), 0);
    }

    #[test]
    fn grid_segments_connect_neighbors_but_leave_node_gaps() {
        // A 3×3 window: 2·3 horizontal + 3·2 vertical inter-neighbor
        // segments, none along the unused sevens axis.
        let view = ViewConfig {
            extent_threes: 1,
            extent_fives: 1,
            extent_sevens: 0,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.grid.len(), 12);
        for seg in &scene.grid {
            // Inset at both ends: shorter than the node spacing...
            let len = seg.a.distance(seg.b);
            assert!(len < view.spacing * 0.99, "segment not inset, len {len}");
            // ...and clear of every disc (visual radius ~0.9 × node_radius),
            // so the gap fully contains the circle a played note draws.
            for node in &scene.nodes {
                for p in [seg.a, seg.b] {
                    assert!(
                        p.distance(node.world_pos) > scene.node_radius * 0.9,
                        "segment endpoint {p:?} inside the disc at {:?}",
                        node.world_pos
                    );
                }
            }
        }

        // Panning the window keeps the grid attached to the visible nodes
        // (both are derived in centered world space).
        let panned = ViewConfig { center_threes: 3, ..view };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &panned,
            &FrameParams::default(),
            0.0,
        );
        assert_eq!(scene.grid.len(), 12);
        let max_node = scene
            .nodes
            .iter()
            .map(|n| n.world_pos.length())
            .fold(0.0f32, f32::max);
        for seg in &scene.grid {
            assert!(seg.a.length() <= max_node && seg.b.length() <= max_node);
        }
    }

    #[test]
    fn home_sheet_nodes_are_flagged_for_the_blank_ring() {
        // Follows the panned window center, not sevens == 0.
        let view = ViewConfig {
            extent_threes: 0,
            extent_fives: 0,
            extent_sevens: 1,
            center_sevens: 2,
            ..ViewConfig::default()
        };
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        for n in &scene.nodes {
            assert_eq!(n.on_home, n.lattice_pos.sevens == 2, "{:?}", n.lattice_pos);
        }
    }

    #[test]
    fn off_sheet_grid_appears_only_where_the_music_reaches() {
        // A window two sevens layers deep above/below the center, so the
        // chain rule has an intermediate link to prove itself on.
        let view = ViewConfig {
            extent_threes: 1,
            extent_fives: 0,
            extent_sevens: 2,
            ..ViewConfig::default()
        };
        let is_link = |s: &EdgeInstance| (s.b.z - s.a.z).abs() > 0.25;
        let off_home = |s: &EdgeInstance| is_link(s) || s.a.z.abs() > 0.5;

        // Idle: only the home sheet's solid lines exist.
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &view,
            &FrameParams::default(),
            0.0,
        );
        assert!(!scene.grid.is_empty());
        assert!(scene.grid.iter().all(|s| !off_home(s) && !s.dashed && s.strength > 0.0));

        // Hold the note two sevens steps up from C (12-TET default:
        // 2 × 1000¢ → pitch class 800¢ = G#/Ab, MIDI 68). It lights node
        // (0,0,2) only, yet BOTH links of the chain down to the home
        // sheet must display, dashed, in that note's color — the nodes
        // under it are silent.
        let mut tracker = NoteTracker::new();
        tracker.handle_event(NoteEvent {
            time: 0.0,
            channel: 0,
            note: 68,
            kind: NoteEventKind::On { velocity: 1.0 },
        });
        let scene =
            scene_of(&tracker, &Tuning::default(), &view, &FrameParams::default(), 0.0);
        let column_links: Vec<&EdgeInstance> = scene
            .grid
            .iter()
            .filter(|s| is_link(s) && s.a.x.abs() < 0.01 && s.a.y.abs() < 0.01)
            .collect();
        // The two links spanning 0->1 and 1->2; nothing below the sheet.
        assert_eq!(column_links.len(), 2, "{column_links:?}");
        for link in &column_links {
            assert!(link.a.z > -0.5 && link.dashed && link.strength > 0.5, "{link:?}");
        }
        // No off-sheet IN-SHEET lines appeared: the played node's sheet
        // neighbors are silent, so only the chain and home sheet render.
        assert!(scene
            .grid
            .iter()
            .all(|s| is_link(s) || s.a.z.abs() < 0.5));
    }

    #[test]
    fn depth_scale_exaggerates_proximity() {
        // Neutral at the focus distance, monotonic on either side, clamped
        // at the extremes.
        assert!((depth_scale(12.0, 12.0) - 1.0).abs() < 1e-6);
        assert!(depth_scale(6.0, 12.0) > 1.0);
        assert!(depth_scale(24.0, 12.0) < 1.0);
        assert_eq!(depth_scale(0.001, 12.0), DEPTH_SCALE_RANGE.1);
        assert_eq!(depth_scale(1e6, 12.0), DEPTH_SCALE_RANGE.0);

        // And the scene wires it in: the node nearest the eye renders
        // larger than the farthest one.
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let eye = scene.camera.eye();
        let dist = |n: &&NodeInstance| n.world_pos.distance(eye);
        let near = scene.nodes.iter().min_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
        let far = scene.nodes.iter().max_by(|a, b| dist(a).total_cmp(&dist(b))).unwrap();
        assert!(
            near.scale > far.scale,
            "near {} vs far {}",
            near.scale,
            far.scale
        );
    }

    #[test]
    fn grid_lights_between_played_neighbors() {
        // C and G held (one step on the threes axis; same window/tuning
        // rationale as chord_edges_connect_adjacent_active_nodes): the
        // segment between them takes the lit opacity and the notes' blended
        // color; a segment with only one sounding endpoint stays at the
        // faint base look.
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
        let view = ViewConfig {
            extent_threes: 3,
            extent_fives: 3,
            ..ViewConfig::default()
        };
        let scene = scene_of(&tracker, &tuning, &view, &FrameParams::default(), 0.0);
        let base = skin::active_skin().grid_line;
        let segment_at = |mid: Vec3| {
            scene
                .grid
                .iter()
                .find(|s| ((s.a + s.b) * 0.5).distance(mid) < 1e-4)
                .unwrap()
        };

        // C sits at the origin, G one step up the threes (world y) axis.
        let lit = segment_at(Vec3::new(0.0, 0.5, 0.0));
        assert!(lit.strength > base.w, "lit opacity, got {}", lit.strength);
        assert!(
            (lit.color - base).truncate().length() > 0.1,
            "lit segment tinted by the notes, got {:?}",
            lit.color
        );

        // F–C below: C sounds but F doesn't, so the segment stays faint.
        let unlit = segment_at(Vec3::new(0.0, -0.5, 0.0));
        assert_eq!(unlit.strength, base.w);
        assert_eq!(unlit.color, base);
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
        let scene = scene_of(
            &tracker,
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        assert!(origin_node(&scene).outlined);
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
        // Sampled after OCTAVE_ATTACK_TIME: the octave indicator eases in,
        // so at the note-on instant itself it is still at zero.
        let scene = scene_of(
            &tracker,
            &tuning,
            &ViewConfig::default(),
            &FrameParams::default(),
            0.5,
        );
        let origin = origin_node(&scene);
        assert_eq!(origin.activation, 1.0);
        assert_eq!(origin.octaves[4], 1.0);
    }

    #[test]
    fn camera_target_projects_to_viewport_center() {
        let camera = Camera::default();
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let viewport = Vec2::new(800.0, 600.0);
        let p = scene.project(viewport, camera.target).unwrap();
        assert!((p.x - 400.0).abs() < 0.5, "x = {}", p.x);
        assert!((p.y - 300.0).abs() < 0.5, "y = {}", p.y);
    }

    #[test]
    fn points_behind_the_camera_do_not_project() {
        for projection in [
            Projection::Perspective,
            Projection::Orthographic,
            Projection::Cabinet,
        ] {
            let camera = Camera { projection, ..Camera::default() };
            let mut scene = scene_of(
                &NoteTracker::new(),
                &Tuning::default(),
                &ViewConfig::default(),
                &FrameParams::default(),
                0.0,
            );
            scene.camera = camera;
            // Continue from the target through the eye and beyond it.
            let behind = camera.eye() + (camera.eye() - camera.target);
            assert_eq!(
                scene.project(Vec2::new(800.0, 600.0), behind),
                None,
                "{projection:?}"
            );
        }
    }

    #[test]
    fn cabinet_faces_the_sheet_and_shears_sevens_uniformly() {
        let viewport = Vec2::new(800.0, 600.0);
        // Orbit angles are ignored: cabinet always faces the sheet.
        let camera = Camera {
            projection: Projection::Cabinet,
            yaw: 1.0,
            pitch: -0.7,
            ..Camera::default()
        };
        assert_eq!(camera.eye(), Vec3::new(0.0, 0.0, camera.distance));

        let mut s = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        s.camera = camera;
        let px = |w: Vec3| s.project(viewport, w).unwrap();

        // Target centered; front-plane steps map to pure screen axes
        // (the sheet renders undistorted).
        let origin = px(Vec3::ZERO);
        assert!((origin - Vec2::new(400.0, 300.0)).length() < 0.5, "{origin:?}");
        let dx = px(Vec3::X) - origin;
        assert!(dx.x > 1.0 && dx.y.abs() < 1e-3, "{dx:?}");
        let dy = px(Vec3::Y) - origin;
        assert!(dy.y < -1.0 && dy.x.abs() < 1e-3, "{dy:?}"); // screen y is down

        // A +sevens step (toward the viewer) is the same up-right arrow
        // anywhere on the sheet, at half scale split evenly over x/y.
        let dz = px(Vec3::Z) - origin;
        let dz_elsewhere = px(Vec3::new(3.0, -2.0, 1.0)) - px(Vec3::new(3.0, -2.0, 0.0));
        assert!(dz.distance(dz_elsewhere) < 1e-3, "{dz:?} vs {dz_elsewhere:?}");
        let k = 0.5 * std::f32::consts::FRAC_1_SQRT_2;
        assert!((dz.x - dx.x * k).abs() < 0.1, "{dz:?} vs {dx:?}");
        assert!((dz.y - dy.y * k).abs() < 0.1, "{dz:?} vs {dy:?}");

        // The knobs steer the arrow: angle 0 at full (cavalier) scale
        // shears purely horizontally, one front-plane step long.
        s.camera.cabinet_angle = 0.0;
        s.camera.cabinet_scale = 1.0;
        let dz = s.project(viewport, Vec3::Z).unwrap() - s.project(viewport, Vec3::ZERO).unwrap();
        assert!((dz.x - dx.x).abs() < 0.1 && dz.y.abs() < 1e-3, "{dz:?} vs {dx:?}");
    }

    #[test]
    fn orthographic_matches_perspective_at_the_focus_plane_and_is_uniform() {
        let viewport = Vec2::new(800.0, 600.0);
        let perspective = Camera::default();
        let ortho = Camera { projection: Projection::Orthographic, ..perspective };
        let mut s = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );

        // The target projects to the viewport center in both projections.
        s.camera = ortho;
        let p = s.project(viewport, ortho.target).unwrap();
        assert!((p.x - 400.0).abs() < 0.5 && (p.y - 300.0).abs() < 0.5, "{p:?}");

        // Framing matches at the focus plane: a point one unit up (in view
        // space) from the target lands on the same pixel either way.
        let (_, up) = perspective.right_up();
        let in_plane = perspective.target + up;
        let ortho_px = s.project(viewport, in_plane).unwrap();
        s.camera = perspective;
        let persp_px = s.project(viewport, in_plane).unwrap();
        assert!(ortho_px.distance(persp_px) < 0.5, "{ortho_px:?} vs {persp_px:?}");

        // The property the projection exists for: equal world offsets give
        // equal pixel offsets at ANY depth. Step one unit right at the
        // focus plane and again two units toward the eye; perspective
        // renders the nearer step longer, orthographic identically.
        s.camera = ortho;
        let (right, _) = ortho.right_up();
        let toward_eye = (ortho.eye() - ortho.target).normalize() * 2.0;
        let d_focus = s.project(viewport, ortho.target + right).unwrap()
            - s.project(viewport, ortho.target).unwrap();
        let d_near = s.project(viewport, ortho.target + toward_eye + right).unwrap()
            - s.project(viewport, ortho.target + toward_eye).unwrap();
        assert!(d_focus.distance(d_near) < 1e-3, "{d_focus:?} vs {d_near:?}");
    }

    #[test]
    fn pick_selects_the_node_nearest_the_pointer() {
        let scene = scene_of(
            &NoteTracker::new(),
            &Tuning::default(),
            &ViewConfig::default(),
            &FrameParams::default(),
            0.0,
        );
        let viewport = Vec2::new(800.0, 600.0);
        // Pointer exactly on the projected origin node must pick it, not a
        // neighbor; a pointer far outside every node picks nothing.
        let origin_px = scene.project(viewport, Vec3::ZERO).unwrap();
        assert_eq!(scene.pick(viewport, origin_px, 24.0), Some(LatticePos::ORIGIN));
        assert_eq!(scene.pick(viewport, Vec2::new(-500.0, -500.0), 24.0), None);
    }

    #[test]
    fn camera_right_up_is_orthonormal_to_the_view() {
        let camera = Camera::default();
        let (right, up) = camera.right_up();
        assert!((right.length() - 1.0).abs() < 1e-5);
        assert!((up.length() - 1.0).abs() < 1e-5);
        assert!(right.dot(up).abs() < 1e-5);
        let view_dir = (camera.target - camera.eye()).normalize();
        assert!(right.dot(view_dir).abs() < 1e-5);
        assert!(up.dot(view_dir).abs() < 1e-5);
    }

    #[test]
    fn camera_input_respects_clamps() {
        let mut camera = Camera::default();
        camera.orbit(Vec2::new(0.0, 10_000.0));
        assert_eq!(camera.pitch, Camera::PITCH_LIMIT);
        camera.orbit(Vec2::new(0.0, -100_000.0));
        assert_eq!(camera.pitch, -Camera::PITCH_LIMIT);
        camera.zoom(1e6);
        assert_eq!(camera.distance, Camera::MIN_DISTANCE);
        camera.zoom(-1e9);
        assert_eq!(camera.distance, Camera::MAX_DISTANCE);
        // Panning moves the target in the view plane, never toward the eye.
        let before = camera.eye() - camera.target;
        camera.pan(Vec2::new(40.0, -25.0));
        let after = camera.eye() - camera.target;
        assert!((before - after).length() < 1e-4);
    }
}
