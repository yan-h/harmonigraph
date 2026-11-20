//! Shell ownership, synchronous visual runtime, viewport resources and editor
//! workspace. Persisted documents are assembled explicitly at the shell boundary.

use std::collections::VecDeque;

use harmonigraph_core::LatticePos;
use harmonigraph_perf::{PerfStats, ShellTimings};
use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_scene::{Camera, DrawnWindow};

use crate::VisualRuntime;
use crate::{panes, workspace};

/// Scrollback for the debug console pane. Shells and panes log via
/// [`Console::log`].
#[derive(Default)]
pub struct Console {
    pub(crate) lines: VecDeque<String>,
}

impl Console {
    /// Lines kept before the oldest is dropped.
    pub(crate) const MAX_LINES: usize = 500;

    pub fn log(&mut self, line: impl Into<String>) {
        if self.lines.len() == Self::MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    pub fn clear(&mut self) {
        self.lines.clear();
    }
}

/// Take recording and video export: the whole contract between the Video pane
/// and the shell.
///
/// The shell owns the actual recorder — `harmonigraph_record::Control` in the
/// plugin, nothing at all in the standalone, which records through an env var
/// instead — and this is what the two say to each other: the pane writes the
/// toggles, the shell writes back what the recorder is doing. Nothing in here
/// reaches the recorder itself, which is why the pane compiles in a shell that
/// has none.
///
/// Runtime-only. A take
/// is a deliberate act, so nothing else here is ever resumed on load — an
/// editor that reopened armed would record a session nobody asked it to.
///
/// The plugin owns the recorder separately. `take_pass` exchanges its status
/// and explicit actions with this workspace interaction state once per GUI
/// callback: the status before the UI draws, the actions after.
#[derive(Default)]
pub struct TakeState {
    /// Whether this shell can record at all. Gates the control: a shell that
    /// cannot (or a build without a writer) simply doesn't show it, rather
    /// than offering a button that does nothing.
    pub supported: bool,
    /// Toggled by the Video pane, acted on by the shell.
    pub recording: bool,
    /// Whether the transport is actually rolling (capture is happening), as
    /// opposed to armed-and-waiting. Drives the record indicator: a steady dot
    /// while rolling, a breathing one while it waits. Shell-set.
    pub rolling: bool,
    /// Shell-supplied one-liner shown under the toggle: where the file is
    /// going, how many events, or what went wrong.
    pub status: String,
    pub last_take: Option<std::path::PathBuf>,
    pub exports: Vec<harmonigraph_take::render::ExportJob>,
    pub export_actions: Vec<ExportAction>,
    pub export_paths: String,
    /// Empty selects the current appearance; otherwise a slot.
    pub export_look: Option<usize>,
    pub export_recorded: bool,
    pub export_error: String,
}

/// Settings are captured at the button press, before the shell's next frame.
pub enum ExportAction {
    Queue {
        take: std::path::PathBuf,
        appearance: String,
        /// The pixels the Exports list shows for the job: what the renderer
        /// derives from `appearance`, computed from the same document and
        /// not an input to the render.
        size: [u32; 2],
    },
    Cancel(u64),
    Retry(u64),
    ClearFinished,
    /// Open the folder recorded takes, and so their videos, are written to.
    OpenTakeFolder,
    /// Show this finished video in the file browser.
    Reveal(std::path::PathBuf),
}

/// Shell aggregate. Drawing and runtime code borrow its domains independently.
pub struct SharedState {
    pub picture: PictureState,
    pub workspace: Workspace,
    /// Nothing installed and no window opened since the state was fresh: a
    /// host save stores `""`, so a project whose editor was never opened keeps
    /// following the fresh look of whichever build loads it (#1757 Q2). An
    /// empty restore leaves it as it was, so one into an editor that has held
    /// settings never turns its later saves into `""`.
    untouched: bool,
    /// What a save stores as of the last restore or edit the host was told
    /// of, with every automatic write since settled into it as it was made
    /// (the layout fitted to the window, in `workspace::show`): the key
    /// [`report_edit`](Self::report_edit) compares the live settings against.
    pub(crate) reported: UiPersist,
    /// A frame of the gesture still open differed from
    /// [`reported`](Self::reported), so its end reports even if it came back.
    moved: bool,
    /// Told, under the editor lock, when an edit changes what a save stores:
    /// the plugin has the host mark its project modified (#1813).
    on_edit: Option<Box<dyn Fn() + Send>>,
}

/// Shared live/offline picture. A picture pane cannot access the editor dock,
/// shell actions, recording controls or interface preferences.
pub struct PictureState {
    pub runtime: VisualRuntime,
    pub appearance: crate::AppearanceDocument,
    pub surfaces: SurfaceState,
    pub instruments: Instruments,
}

/// Viewport geometry and temporal graphics, separate from input history.
pub struct SurfaceState {
    pub(crate) spectrogram: crate::spectrogram::SpectrogramSurfaces,
    pub(crate) note_names: std::collections::HashMap<usize, panes::spectral::names::Thinning>,
    /// The lattice node the pointer is over, if any.
    ///
    /// Shared state that one pane writes and one pane reads: the lattice
    /// picks it and the lattice highlights it. It sits here rather than in
    /// the pane because the offline renderer has to force it to `None` (no
    /// pointer, and a recorded frame must not carry one), and because a
    /// second pane answering "which node is this pitch" is a standing
    /// temptation — the analyzer did, and its pitch axis is continuous, so
    /// it wrote a node the pointer had only landed near. Nothing outside
    /// [`crate::panes::lattice`] should write this.
    pub hovered: Option<LatticePos>,
    /// The block of lattice the docked pane drew LAST frame, for the readers
    /// that have to say what the picture is showing — the analyzer's red "off
    /// the lattice" band, the roll's and spiral's note names, and the name a
    /// pitch gets when the reach cannot spell it. `None` until a lattice pane has
    /// drawn one; [`PictureState::shown`] is what to read, and it falls back to
    /// the view's reach.
    ///
    /// Reported rather than computed from the view, because the window is
    /// derived per pane from that pane's own aspect
    /// ([`harmonigraph_scene::ViewConfig::scrolled`]), and the dock knows its
    /// lattice's rect only as it draws it.
    ///
    /// LAST frame's, and that is what makes it safe to read from another pane:
    /// the dock draws its panes in whatever order the user has arranged them,
    /// so a band asking about THIS frame would answer from the reach or from
    /// the window depending on where the lattice sits in the layout. A frame
    /// of lag on a window that only moves with the camera is invisible; an
    /// answer that changes with the dock arrangement is not.
    ///
    /// The docked copy's, and only ever its. A composed frame, an export or the
    /// Video preview, draws its lattice first and hands its spectral pane the
    /// window that draw returns instead ([`crate::draw_panes`]), publishing
    /// nothing here. The preview's is a second lattice at a second aspect, and
    /// letting it publish would make the docked answers jump with a tab that is
    /// not the one being read — the same argument that keeps the preview out
    /// of the GPU timing slot.
    pub drawn: Option<DrawnWindow>,
    /// What the docked lattice has published so far THIS frame, rotated into
    /// [`drawn`](Self::drawn) by `begin_frame`. The perf overlay's node count
    /// reads it directly, because that read happens after every pane has
    /// drawn and a diagnostic holding its last good reading is the one that
    /// misleads.
    pub drawn_this_frame: Option<DrawnWindow>,
    /// Whether each picture drawn so far THIS frame is still moving, cleared by
    /// `begin_frame` and read by the repaint guard once every pane has drawn
    /// ([`Animating`](crate::Animating)). Runtime-only.
    pub(crate) moving_this_frame: crate::Animating,
    /// Surface format of the shell's swapchain; the lattice render pipeline
    /// must match it.
    pub target_format: TextureFormat,
    /// Device-bound compiled pipelines outlive windows, while their pane
    /// buffers and textures remain in each window's callback resources.
    pub(crate) lattice_pipelines: std::sync::Arc<harmonigraph_render::LatticePipelineCache>,
    /// The vectors the lattice panes' frames are built in, lent to each frame
    /// and handed back once it has drawn.
    pub(crate) lattice_frames: std::sync::Arc<harmonigraph_render::LatticeFrames>,
    /// The ground the lattice pane paints its rect with, which it also hands
    /// the scene (see [`harmonigraph_scene::Scene::background`]). Defaults to
    /// [`harmonigraph_scene::skin::picture_color`], the black every picture
    /// pane paints — right for the plugin and the standalone harness.
    ///
    /// ONE field doing both jobs deliberately: the pane paints exactly what it
    /// hands over, so the fill and the ground the picture is composited against
    /// cannot drift apart. Anything setting this is choosing the pane's colour,
    /// not just describing it.
    ///
    /// A shell that composes its panes differently MUST set this: the offline
    /// renderer clears the whole frame to its layout's own background and draws
    /// the panes over that, so the pane's fill has to land on a ground already
    /// that colour. A pane standing on the wrong one shows up as a rectangle
    /// visibly lighter or darker than the picture around it, and exported video
    /// is the one place that is hardest to notice and most expensive to get
    /// wrong.
    pub background: glam::Vec4,
    /// Where every node's own light has got to, and what it is made of, per
    /// lattice surface ([`GlowFade`](crate::panes::glow_fade::GlowFade)).
    /// Runtime-only.
    ///
    /// Here for the reason the two above it are: the offline renderer carries
    /// this struct between frames and nothing else, so a light kept anywhere
    /// else would linger live and not in an export. Keyed by the surface id
    /// each lattice pane claims, because each draws its own window and lights
    /// the nodes in it.
    pub(crate) glow_fade: std::collections::HashMap<usize, crate::panes::glow_fade::GlowFade>,
    /// One entrance per visible node, independent of optional Glow. Runtime only.
    pub(crate) node_motion: std::collections::HashMap<usize, harmonigraph_scene::NodeMotion>,
    /// The last lattice frame's nodes, handed back once it has drawn and laid
    /// out again by the next (`derive_scene_with_extra`). One for every
    /// surface, since they draw one after another.
    pub(crate) spare_nodes: Vec<harmonigraph_scene::NodeInstance>,
    /// Where the analyzer's divider stands on the DOCKED pane as that pane is
    /// resized — the spectrum keeps its size and the spectrogram takes the
    /// difference. See [`panes::spectral::SpectrumHold`].
    ///
    /// Runtime-only, and it is the answer rather than the setting: the dial
    /// itself stays in [`appearance.spectrum`](crate::AppearanceDocument::spectrum), because that
    /// is what a project saves and a take renders from, and a length in POINTS
    /// is a fact about the window this session happens to be open in.
    pub(crate) spectrum_hold: panes::spectral::SpectrumHold,
    /// Which Stars depths each picture solos, from the Stars `Solo` toggles: a
    /// viewing aid for one editor window rather than part of the look (#1828).
    /// No save, slot, undo or export holds it, so it stays through a slot
    /// switch, an undo or a restore until it is switched off or the window
    /// closes ([`SharedState::start_window_session`]). Only the working
    /// lattice and spectrogram panes are handed it, so the Video preview draws
    /// what the render will (Yan, #1862); crate-private as well, so the
    /// offline renderer cannot set it.
    pub(crate) star_solo: StarSolo,
}

/// Which Stars depths the lattice and the spectrogram each solo
/// ([`SurfaceState::star_solo`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct StarSolo {
    pub lattice: [bool; harmonigraph_scene::star_plan::STAR_DEPTHS],
    pub spectrogram: [bool; harmonigraph_scene::star_plan::STAR_DEPTHS],
}

/// The editor's preferences: how this editor, on this screen, is set up to
/// work. The line between these and what [`UiPersist`] keeps beside them is
/// what a reset costs: each of these is a moment's redo (a scale, a switch, a
/// fold, a framing), where the look, its slots, the camera presets and the
/// layout with its folded regions are the user's work.
///
/// Saved whole under one key, so a preference is a field here, its line in
/// `Default`, and a repair in [`Self::sanitize`] if it needs one; saving and
/// loading copy the struct. None of it rides in a take or a recorded
/// appearance.
#[derive(Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EditorPrefs {
    /// Which settings sections are folded, as "page/section" titles
    /// (persisted).
    ///
    /// Here rather than in egui `Context` memory, and the home is load-bearing:
    /// the plugin builds a brand new `Context` every time the editor window
    /// opens (see [`shell::Opening`](crate::shell::Opening)), so
    /// a fold kept in memory springs open with every reopen. Each section's
    /// header reads and writes it through the hand-off
    /// [`panes::section`] describes.
    pub folded_sections: std::collections::BTreeSet<String>,
    /// How often the editor draws, as a fraction of the display's refresh
    /// rate. Persisted. See [`FrameRate`].
    pub frame_rate: FrameRate,
    /// How big the panel chrome draws — type, spacing, control heights, tab
    /// bars — as a multiple of the design size. Persisted. See
    /// [`crate::theme::ui_scale`], which is where it takes effect and where
    /// the reasoning lives.
    ///
    /// A property of the SCREEN the plugin is open on rather than of the
    /// piece, which is why it is here beside `frame_rate` and not in `view`:
    /// `ViewConfig` is what a recorded frame is composed from, and a laptop
    /// dialling its panel down must not change what a render of the same
    /// project comes out looking like. [`root_ui`](crate::root_ui), the
    /// System bar and `install_document` read it; the offline renderer never
    /// reaches any of them.
    pub ui_scale: f32,
    /// Hide the editor's tab and fold bars for a continuous live picture.
    /// Tab toggles them back; this preference is never recorded in a take.
    pub frameless: bool,
    /// What the chrome's colours are made of (see
    /// [`harmonigraph_scene::skin::Skin::from_dials`]). Persisted. Here beside
    /// `ui_scale` for the same reason: it colors the panel, never the picture
    /// (whose colors are fixed, see `harmonigraph_scene::skin::PICTURE`), so a
    /// render cannot depend on it.
    pub skin_dials: harmonigraph_scene::skin::SkinDials,
    /// Where the performance overlay's top-left corner sits, in editor
    /// points. Persisted; `None` until the HUD is dragged, which is the only
    /// thing that ever writes it (see [`crate::perf::draw_overlay`]).
    ///
    /// Here rather than in `view` for the reason `ui_scale` above it is: the
    /// overlay is a development instrument over the picture and never part of
    /// one, so where it was pushed to on this screen has no business in
    /// [`ViewConfig`](harmonigraph_scene::ViewConfig). This editor preference
    /// stays outside recorded appearance; only [`root_ui`](crate::root_ui)
    /// reads it and the offline renderer never draws the HUD.
    pub perf_pos: Option<egui::Pos2>,
    /// Show the performance overlay (a small draggable HUD with frame rate,
    /// memory and workload counts; per-stage CPU time waits for
    /// [`Self::show_perf_detail`]). Persisted.
    ///
    /// Off by default: the HUD is a development instrument, and it sits over
    /// the picture the plugin exists to draw. The System tab, under
    /// Performance, is where it gets switched on.
    ///
    /// Here rather than in `view` for the reason `perf_pos` above it is: the
    /// overlay is what the picture is read AGAINST while it is dialled, not
    /// part of the picture, so it rides in no take and no recorded appearance.
    /// `the_performance_overlay_ships_off` holds the default.
    pub show_perf: bool,
    /// Expand the overlay from the headline numbers into the full per-stage
    /// breakdown of where a frame goes. Persisted.
    ///
    /// Off by default: the breakdown exists to answer "which stage is eating
    /// the frame", and once it has, a dozen rows of scaffolding is not what
    /// you want sitting over the picture. Inert while `show_perf` is off.
    pub show_perf_detail: bool,
    /// The Spiral tab's framing (persisted; see [`panes::spiral::SpiralView`]).
    ///
    /// Here rather than in the appearance for the reason [`Self::show_perf`] is: the
    /// Spiral is an editor tab and never exported (#974), so its framing is
    /// how this editor is looking at the picture, and rides in no take.
    pub(crate) spiral: panes::spiral::SpiralView,
}

impl Default for EditorPrefs {
    fn default() -> Self {
        Self {
            folded_sections: panes::folded_by_default(),
            frame_rate: FrameRate::default(),
            // The design size, which a fresh install opens at and a blob
            // without the key loads as; an `f32`'s own default of 0.0 is a
            // scale of nothing.
            ui_scale: 1.0,
            frameless: false,
            skin_dials: Default::default(),
            perf_pos: None,
            show_perf: false,
            show_perf_detail: false,
            spiral: Default::default(),
        }
    }
}

impl EditorPrefs {
    /// Repair what a hand-edited or older blob can carry, once on load.
    fn sanitize(&mut self) {
        // Clamped here rather than only where it is drawn, so the control
        // cannot read out a number the chrome is not at: `set_ui_scale`
        // would take a hand-edited 5.0 down to the top of the range while
        // the bar went on saying 500%.
        self.ui_scale = crate::theme::sane_ui_scale(self.ui_scale);
        // Clamped for the reason the scale is: each bar reads out what the
        // chrome is drawn in.
        self.skin_dials.sanitize();
        self.spiral.sanitize();
        // A hand-edited NaN is dropped rather than honoured, on the grounds
        // the Spiral's framing is repaired on above: it positions drawn
        // geometry, and NaN geometry is a panic inside egui's
        // tessellator. A dropped position opens the HUD where an undragged one
        // opens, which is a place the user can see it and drag it from.
        self.perf_pos = self.perf_pos.filter(|pos| pos.is_finite());
    }
}

/// Editor interaction and shell actions. Panes borrow this separately from
/// the layout being traversed, so a reset request cannot replace a live dock.
#[derive(Default)]
pub struct Interaction {
    pub(crate) appearance_editor: crate::appearance_edit::AppearanceEditor,
    /// How this editor is set up to work, saved whole (see [`EditorPrefs`]).
    pub prefs: EditorPrefs,
    /// Which of the Analyzer's regions are folded, and the composition they
    /// fold back open to (see [`panes::spectral::collapse::Regions`]).
    ///
    /// The layout's content rather than a preference, so it is saved, loaded
    /// and reset with the layout.
    pub(crate) analyzer_regions: panes::spectral::collapse::Regions,
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see Camera in the Lattice page's View section).
    pub camera_presets: Vec<CameraPreset>,
    /// Entry buffer for naming a new preset. Runtime-only.
    pub preset_name: String,
    /// Take recording and video export, which the Video pane and the shell
    /// pass between them — see [`TakeState`].
    pub take: TakeState,
    pub(crate) reset_layout: bool,
    /// Whether the editor is in a window of its own rather than the host's,
    /// which the System tab flips; `None` in a shell that cannot do that.
    /// Runtime only here: the plugin saves the setting with its window size,
    /// writes it here before every frame, and reads it back after.
    pub own_window: Option<bool>,
    /// Where the Analyzer section is docked, copied from the workspace layout
    /// before the panes draw and back after. Runtime only: the layout persists
    /// it; this copy lets the Analyzer settings page set it.
    pub(crate) dock: workspace::Position,
    /// A settings tab a picture's right-click menu asked for, which the
    /// workspace opens after the sections draw.
    pub(crate) open_settings: Option<panes::Tab>,
}

/// Fixed section layout and editor interaction, separate from the picture.
/// Shell actions are consumed after drawing; saving selects persisted fields.
pub struct Workspace {
    pub interaction: Interaction,
    pub(crate) layout: workspace::Layout,
    pub(crate) layout_runtime: workspace::Runtime,
    /// One-shot change to the outer window, consumed by its shell.
    pub(crate) window_size_change: egui::Vec2,
    pub min_window_size: egui::Vec2,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            interaction: Interaction::default(),
            layout: workspace::Layout::default(),
            layout_runtime: workspace::Runtime::default(),
            window_size_change: egui::Vec2::ZERO,
            min_window_size: egui::Vec2::ZERO,
        }
    }
}

impl Workspace {
    /// Take each request once, including when a host refuses it.
    pub fn take_window_size_change(&mut self) -> Option<egui::Vec2> {
        let change = std::mem::take(&mut self.window_size_change);
        (change.abs().max_elem() >= 0.5).then_some(change)
    }

    /// Return to the default arrangement after this frame's panes finish.
    pub fn reset_layout(&mut self) {
        self.interaction.reset_layout = true;
    }
}

/// What the frame publishes about ITSELF: the measurements the performance
/// overlay reads, and the side channels the draw callbacks write them through.
///
/// Grouped for the second half, which is the part worth saying out loud. None
/// of the three is contended; in each the concurrency primitive IS the return
/// path, for one of two reasons.
///
/// `roll_notes` and the atlas trackers use an atomic and `Mutex`es because they
/// are written from a `&SharedState` — the roll draws from one, and so does the
/// label batch's flush — with no way to hand a value back up the call stack.
///
/// `lattice_stats` is an `Arc` for the harder version of the same problem, and
/// NOT because of a shared borrow: egui stores a paint callback as
/// `Arc<dyn Any + Send + Sync>`, so the sink has to be OWNED by the callback
/// rather than borrowed from anything — `'static` leaves no lifetime a borrow
/// could go in at. `prepare` then runs behind `&self`, several frames after
/// the frame that asked for the timing.
///
/// Spread flat among the ordinary fields around them those reasons are
/// invisible, and the obvious reading — that something here is contended — is
/// the wrong one.
///
/// `timings` and `perf` are the other end of the same frame: what the shell
/// measured before `root_ui` ran, and the rolling windows `root_ui` folds all
/// of it into. The atlas trackers ride here for the mechanism rather than the
/// meaning: the draw path updates them through a shared reference, once per
/// flush and uncontended.
///
/// None of it is persisted — `save_persist` builds `UiPersist` field by field,
/// so what is grouped here cannot reach the blob either way.
///
/// What keeps it clear of a RECORDED frame is three different arguments, not
/// one, and the difference is worth having: `timings` and `perf` belong to
/// `root_ui`, which the offline renderer never enters at all. The three side
/// channels it DOES write, every offline frame, because it calls `draw_panes` —
/// but nothing offline reads `lattice_stats` or `roll_notes`, so those are
/// dead writes, and the atlas trackers depend only on the context's glyph
/// assets. Both `lattice_stats` and `roll_notes` carry wall-clock or per-frame
/// values, so a future offline READ of either is exactly what would break
/// determinism.
pub struct Instruments {
    /// Lattice callback timings, as f32 bits, consumed once by the performance
    /// overlay. The renderer's default distinguishes absent samples from real
    /// zero-cost measurements. Never read by the offline renderer, which also
    /// never asks for the GPU timestamp feature.
    pub(crate) lattice_stats: std::sync::Arc<harmonigraph_render::LatticeStats>,
    /// How many note segments the docked roll handed its paint callback last
    /// frame — the geometry `verts` does NOT see, four vertices at a time
    /// instead of several hundred.
    ///
    /// Reported so the roll's load stays visible while it draws from its own
    /// vertex buffer rather than egui's: without it the overlay would show the
    /// cost vanish with nothing standing in its place, and "is the roll drawing
    /// at all" would have no answer.
    ///
    /// Only the docked pane (surface 0) publishes; the Render preview is a
    /// second roll on screen and reporting its count as THE count would be
    /// wrong, exactly as it is for the preview's lattice.
    pub(crate) roll_notes: std::sync::atomic::AtomicU32,
    /// What the shell measured about the previous frame. Written by the shell
    /// before `root_ui` and read once, by `FrameCosts::assemble`; no pane
    /// touches it. The one field here a shell outside this crate writes, which
    /// is why it alone is `pub` — and why the type is `harmonigraph-perf`'s
    /// rather than this crate's: a contract between the shell and the overlay's
    /// model is not something the crate in between should own.
    pub timings: ShellTimings,
    /// Rolling frame-rate / CPU / memory numbers for the performance overlay.
    /// Filled and drawn by [`root_ui`](crate::root_ui).
    pub(crate) perf: PerfStats,
}

impl Default for Instruments {
    fn default() -> Self {
        Instruments {
            lattice_stats: std::sync::Arc::new(harmonigraph_render::LatticeStats::default()),
            roll_notes: std::sync::atomic::AtomicU32::new(0),
            timings: ShellTimings::default(),
            perf: PerfStats::default(),
        }
    }
}

/// How often the editor draws, as a fraction of the display's refresh rate.
///
/// A fraction rather than a number of frames per second because the plugin's
/// frames come from the display, one per refresh, and can only land on one.
/// macOS takes a preferred rate as a hint and rounds it to a divisor of the
/// panel's rate: measured on a 144 Hz panel, 120 ran at 144, and 90 and 60 at
/// 72 (#1551). A fraction is the rate the editor will actually run at.
///
/// Read by the plugin to pace its window, never by any drawing code: the
/// offline renderer steps its own clock and never reaches the editor, so a
/// recorded frame cannot depend on this. The standalone harness and the
/// non-macOS shells draw when their windowing stack asks, whatever this says.
///
/// Persisted in the editor blob, so dropping a variant costs a project that
/// saved it this key, with a Console line (see the persistence-contract skill).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FrameRate {
    #[default]
    Display,
    Half,
    Third,
    Quarter,
}

impl FrameRate {
    /// Draw on every this-many refreshes.
    pub fn divisor(self) -> u32 {
        match self {
            FrameRate::Display => 1,
            FrameRate::Half => 2,
            FrameRate::Third => 3,
            FrameRate::Quarter => 4,
        }
    }
}

/// A saved camera angle: what the built-in Flat/Isometric buttons are,
/// but user-defined. Only the orbit angles — projection, zoom, and pan
/// are deliberately not captured, so a preset composes with any of them.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct CameraPreset {
    pub name: String,
    pub yaw: f32,
    pub pitch: f32,
}

impl CameraPreset {
    /// Fit a deserialized preset to what the camera's own controls can
    /// produce, the same way [`Camera::sanitize`] fits the live camera and for
    /// the same reason — with one more door to cover.
    ///
    /// A preset button assigns these two STRAIGHT into `camera.yaw` and
    /// `camera.pitch`, so applying one is neither `orbit`, which clamps the
    /// pitch on every drag, nor the load-time repair, which clamps it once.
    /// Left unfitted this is the only remaining path by which a hand-edited
    /// blob reaches those fields, and it reaches them one click later than the
    /// load that would have repaired them — which is what makes it quiet: the
    /// lattice opens drawing correctly and breaks when a button is pressed.
    fn sanitize(&mut self) {
        let fresh = Camera::default();
        // Periodic, so any finite yaw draws; a NaN one NaNs `eye()` and with it
        // the whole view matrix, and `orbit`'s `yaw -= delta` cannot walk back
        // out of NaN.
        self.yaw = if self.yaw.is_finite() { self.yaw } else { fresh.yaw };
        self.pitch = if self.pitch.is_finite() { self.pitch } else { fresh.pitch }
            .clamp(-Camera::PITCH_LIMIT, Camera::PITCH_LIMIT);
    }
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        let picture = PictureState::new(target_format);
        let workspace = Workspace::default();
        let reported = UiPersist::capture(&workspace, &picture.appearance);
        Self { picture, workspace, untouched: true, reported, moved: false, on_edit: None }
    }

    /// Call `notice` whenever an edit changes what a host save stores.
    pub fn on_edit(&mut self, notice: impl Fn() + Send + 'static) {
        self.on_edit = Some(Box::new(notice));
    }

    /// Tell the edit notice if what a save stores has moved since it was last
    /// told, now or at any frame of the gesture this ends. Every frame that
    /// holds no drag, scroll or pinch open, and each close, so a gesture is
    /// one notice, at its end, even one that came back to where it started.
    ///
    /// The key is everything [`UiPersist`] saves but what the editor writes
    /// by itself or the host learns otherwise (see [`UiPersist::stores`]).
    /// A field compare rather than a serialization, so a frame pays a
    /// fraction of a microsecond; the copy is taken only when it moved.
    pub(crate) fn report_edit(&mut self) {
        if !self.moved && self.reported.stores(&self.workspace, &self.picture.appearance) {
            return;
        }
        self.settle();
        if let Some(notice) = &self.on_edit {
            notice();
        }
    }

    /// Take what a save stores now as told, with nothing of a gesture still
    /// to report.
    fn settle(&mut self) {
        self.reported = UiPersist::capture(&self.workspace, &self.picture.appearance);
        self.moved = false;
    }

    /// The end of a frame: one with no gesture open reports what moved, and
    /// one inside a gesture notes whether it differs.
    pub(crate) fn end_frame(&mut self, ctx: &egui::Context) {
        if !crate::gesture_open(ctx) {
            self.report_edit();
        } else if !self.moved {
            self.moved = !self.reported.stores(&self.workspace, &self.picture.appearance);
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.picture.runtime.console.log(line);
    }

    /// Serialize the parts of the UI worth restoring across sessions
    /// (section layout, camera, view settings). Parameters are NOT included —
    /// they live in the host's plugin state.
    ///
    /// A plugin host save stores this through [`save_answer`](Self::save_answer),
    /// and the standalone harness on the way out. Recording instead serializes
    /// the live appearance directly, so capture never depends on either.
    pub fn save_persist(&self) -> String {
        // Keep the editor document in the same RON format as appearance.
        ron::to_string(&UiPersist::capture(&self.workspace, &self.picture.appearance))
            .unwrap_or_default()
    }

    /// [`save_persist`](Self::save_persist) laid out for a person to read,
    /// one field to a line down to `depth` levels of nesting: what the
    /// `read-plugin-state` tool prints a project's editor settings as.
    pub fn save_persist_pretty(&self, depth: usize) -> String {
        let config = ron::ser::PrettyConfig::new().depth_limit(depth);
        let persist = UiPersist::capture(&self.workspace, &self.picture.appearance);
        ron::ser::to_string_pretty(&persist, config).unwrap_or_default()
    }

    /// What a host save stores: the live settings, serialized, or `""` for an
    /// editor nobody has opened since it was fresh. The live state is the
    /// only copy of the settings: a restore installs into it, a save
    /// serializes out of it, and nothing reads a saved document back.
    pub fn save_answer(&self) -> std::sync::Arc<str> {
        if self.untouched {
            "".into()
        } else {
            self.save_persist().into()
        }
    }

    /// Install a saved editor document: a host restore, or the standalone
    /// harness's last run.
    ///
    /// The empty document is what an editor never opened saves, so it installs
    /// the fresh settings (#1757 Q3); a save then stores `""` only if the
    /// editor has held nothing else and no window has opened. A value naming a variant this build does not have costs its
    /// own key, which opens at its default (#1757 Q4); refused input leaves the
    /// current content intact. Either loss opens the Console and says there
    /// what was lost; a save then stores the settings as they stand, like any
    /// other. Appearance normalization is shared with recording/export;
    /// workspace restoration remains editor-only.
    ///
    /// A restore discards the look slots' undo histories; a window closing and
    /// reopening does not (#1740).
    ///
    /// Whether the document's settings were installed, whole or in part.
    /// Installed or refused, it is no edit: what a save stores now is taken as
    /// told, and a gesture it lands in reports only what moves after it.
    pub fn install_document(&mut self, serialized: &str) -> bool {
        let installed = self.install(serialized);
        self.settle();
        installed
    }

    fn install(&mut self, serialized: &str) -> bool {
        if serialized.is_empty() {
            self.apply_persist(UiPersist::capture(
                &Workspace::default(),
                &crate::AppearanceDocument::default(),
            ));
            return true;
        }
        let (persist, dropped) = match crate::persist_keys::decode::<UiPersist>(serialized) {
            Ok(decoded) => decoded,
            // SAYING SO is the whole point of this arm. Nothing in the tree
            // reads an older spelling any more, so what fails here — a value
            // that is not RON, or a dropped variant no key holds of its own —
            // costs the layout, the camera and every view setting at once. A
            // dropped variant under a key costs that key alone, and a dropped
            // KEY nothing: serde skips one it has no field for, which is how a
            // blob still carrying the retired `dock` loads intact. An accepted
            // break, but not a silent one — a project opening at defaults with
            // no explanation reads as data loss, and this is the difference
            // between that and a break someone chose.
            Err(err) => {
                return self.refuse_persist(format!("the blob did not parse ({err})"));
            }
        };
        if persist.version < UI_PERSIST_VERSION {
            return self.refuse_persist(format!(
                "version {} is below the floor of {UI_PERSIST_VERSION}",
                persist.version,
            ));
        }
        let appearance = match persist.appearance.normalize() {
            Ok(appearance) => appearance,
            Err(err) => {
                return self.refuse_persist(err);
            }
        };
        self.apply_persist(UiPersist { appearance, ..persist });
        self.untouched = false;
        if dropped.is_empty() {
            return true;
        }
        for lost in dropped {
            self.log(format!(
                "persist dropped {} — this build has no {}, so that setting opens at its default",
                lost.key, lost.variant,
            ));
        }
        self.reveal_console();
        true
    }

    /// Every write an install makes, from a document whose appearance is
    /// already normalized.
    fn apply_persist(&mut self, persist: UiPersist) {
        self.workspace.interaction.prefs = persist.prefs;
        self.workspace.interaction.prefs.sanitize();
        self.workspace.layout = persist.layout;
        self.workspace.layout.sanitize();
        self.workspace.interaction.analyzer_regions = persist.analyzer_regions;
        self.workspace.interaction.analyzer_regions.sanitize();
        // The regions' geometry is new, so the next open frame takes the
        // spectrum hold from it.
        self.picture.surfaces.spectrum_hold.relayout();
        // A restore into an open window replaces the layout a fold or a
        // divider drag in flight was made for.
        self.workspace.layout_runtime = workspace::Runtime::default();
        self.workspace.window_size_change = egui::Vec2::ZERO;
        self.picture.install_appearance(persist.appearance);
        let mut look_slots = persist.look_slots;
        look_slots.sanitize();
        self.workspace.interaction.appearance_editor =
            crate::appearance_edit::AppearanceEditor::restore(look_slots);
        self.workspace.interaction.camera_presets = persist.camera_presets;
        for preset in &mut self.workspace.interaction.camera_presets {
            preset.sanitize();
        }
    }

    /// Reject a saved document loudly, leaving the current content as it was.
    fn refuse_persist(&mut self, reason: String) -> bool {
        self.log(format!("persist ignored — {reason}"));
        self.reveal_console();
        false
    }

    /// An install lost something. The Console is the last settings tab, a
    /// diagnostic rather than a page visited while playing, so this selects it
    /// and unfolds the settings: the diagnostic is the only thing that
    /// explains why the editor opened on fresh settings.
    fn reveal_console(&mut self) {
        self.workspace.layout.select(panes::Tab::Console);
        self.workspace.layout.folded[workspace::Section::Settings as usize] = false;
    }

    /// Start drawing in a newly built window. The state outlives every window,
    /// so what a session carries is the settings, the look slots and their
    /// undo histories; what belongs to the window that closed is reset here.
    /// From the first window on, a save stores the settings it shows.
    pub(crate) fn start_window_session(&mut self) {
        self.untouched = false;
        self.workspace.layout_runtime = workspace::Runtime::default();
        self.workspace.window_size_change = egui::Vec2::ZERO;
        self.workspace.interaction.appearance_editor.start_window_session();
        // The new window lays the Analyzer out afresh.
        self.picture.surfaces.spectrum_hold.relayout();
        // A viewing aid for one window (`SurfaceState::star_solo`).
        self.picture.surfaces.star_solo = Default::default();
    }

    /// The editor is closing, or has stopped being seen: an appearance edit
    /// still open, a drag that cut off, becomes one undo entry. The plugin's
    /// close calls this before it refreshes a save's fallback, so the fallback
    /// holds the session. A gesture the close cut off is reported here.
    pub fn finish_open_edit(&mut self) {
        self.workspace.interaction.appearance_editor.end_window_session(&self.picture.appearance);
        self.report_edit();
    }
}

/// The current [`UiPersist`] layout version, and the FLOOR under it. Bumped
/// when a saved shape changes in a way that still parses but would load
/// wrong. Versions 1 to 6 were bumped for tab set changes, which stranded a
/// saved dock with missing or doubled tabs; since #1056 the layout is fixed
/// and every tab is always placed, so a tab change no longer owes one.
///
/// A bump costs the whole blob, not the layout alone: `install_document` refuses
/// anything below this outright, so camera, view, spectrum and render settings
/// all fall back to defaults with it. That is what lets a format change be
/// made outright rather than shimmed.
///
/// What it costs is a real project's settings, and the cost is paid rather
/// than avoided. The plugin ids gate everything below version 2 — a project
/// that old names an identity this binary does not claim, so its state never
/// arrives — but NOTHING gates a blob one bump behind, which is what a project
/// saved by the previous build carries. Changing the ids again would buy that
/// gate back at the price of orphaning every project that loads the plugin,
/// which is the worse trade; the refusal is made audible instead. See
/// [`SharedState::install_document`], which sets out both halves and which of its
/// callers each covers.
///
/// 7: camera, view, spectrum, spiral and render moved into one appearance
/// document. Previous editor saves are refused whole, with no migration. The
/// spiral framing has since moved back out to a top-level editor key (#1332)
/// without a bump: the copy an older save nests in its appearance is ignored,
/// so that framing opens on the whole disc once.
/// The editor's preferences have since moved under one `prefs` key (#1677),
/// also without a bump: serde skips an older save's top-level copies, so they
/// open at their defaults once and its layout, look, slots and presets load.
pub(crate) const UI_PERSIST_VERSION: u32 = 7;

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize reports refusal and leaves the current content intact,
/// apart from revealing the Console that carries the report.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct UiPersist {
    /// serde(default) reads a pre-versioning blob as version 0, which is below
    /// the floor — [`SharedState::install_document`] refuses it entirely.
    pub(crate) version: u32,
    pub(crate) layout: workspace::Layout,
    /// Which Analyzer regions are folded, saved beside the layout (see
    /// [`Interaction::analyzer_regions`]).
    pub(crate) analyzer_regions: panes::spectral::collapse::Regions,
    pub(crate) appearance: crate::AppearanceDocument,
    pub(crate) camera_presets: Vec<CameraPreset>,
    pub(crate) look_slots: crate::appearance_edit::Slots,
    /// See [`EditorPrefs`]; a blob without them opens at their defaults.
    pub(crate) prefs: EditorPrefs,
}

impl UiPersist {
    /// The saved half of a workspace and its appearance.
    fn capture(workspace: &Workspace, appearance: &crate::AppearanceDocument) -> Self {
        UiPersist {
            version: UI_PERSIST_VERSION,
            layout: workspace.layout.clone(),
            analyzer_regions: workspace.interaction.analyzer_regions.clone(),
            appearance: appearance.clone(),
            camera_presets: workspace.interaction.camera_presets.clone(),
            look_slots: workspace.interaction.appearance_editor.slots.clone(),
            prefs: workspace.interaction.prefs.clone(),
        }
    }

    /// Whether this document holds what a save of `workspace` and `appearance`
    /// stores, but for what the editor writes by itself or the host learns
    /// otherwise or never, which is copied in from the live state first so
    /// that this document takes it on:
    ///
    /// - the host-owned camera ([`AppearanceDocument::copy_host_owned`]),
    ///   which reaches it as parameters and which automation moves;
    /// - what the Analyzer's regions lay out by themselves
    ///   ([`Regions::copy_layout`]).
    ///
    /// The layout `workspace::show` fits to the window is settled where it is
    /// written instead, since the layout is also edited.
    ///
    /// What the compiler holds: the destructure fails to build until a new
    /// top-level field of this document is compared, and each member compares
    /// whole by its derived `PartialEq`, so a field added inside one is in the
    /// key by itself. What it does not: a `#[serde(skip)]` field added to any
    /// of them is compared all the same, and marks the project at every change
    /// until it is copied in here.
    ///
    /// [`AppearanceDocument::copy_host_owned`]: crate::AppearanceDocument::copy_host_owned
    /// [`Regions::copy_layout`]: crate::panes::spectral::collapse::Regions::copy_layout
    fn stores(&mut self, workspace: &Workspace, appearance: &crate::AppearanceDocument) -> bool {
        let UiPersist {
            version: _,
            layout,
            analyzer_regions,
            appearance: own,
            camera_presets,
            look_slots,
            prefs,
        } = self;
        let interaction = &workspace.interaction;
        crate::AppearanceDocument::copy_host_owned(
            (&mut own.camera, &mut own.view),
            (&appearance.camera, &appearance.view),
        );
        analyzer_regions.copy_layout(&interaction.analyzer_regions);
        *own == *appearance
            && *layout == workspace.layout
            && *analyzer_regions == interaction.analyzer_regions
            && *camera_presets == interaction.camera_presets
            && *look_slots == interaction.appearance_editor.slots
            && *prefs == interaction.prefs
    }
}

impl PictureState {
    pub fn new(target_format: TextureFormat) -> Self {
        Self {
            runtime: VisualRuntime::default(),
            appearance: crate::AppearanceDocument::default(),
            surfaces: SurfaceState::new(target_format),
            instruments: Instruments::default(),
        }
    }
}

impl SurfaceState {
    fn new(target_format: TextureFormat) -> Self {
        Self {
            spectrogram: Default::default(),
            note_names: Default::default(),
            hovered: None,
            drawn: None,
            drawn_this_frame: None,
            moving_this_frame: Default::default(),
            target_format,
            lattice_pipelines: Default::default(),
            lattice_frames: Default::default(),
            background: harmonigraph_scene::skin::picture_color(),
            glow_fade: std::collections::HashMap::new(),
            node_motion: std::collections::HashMap::new(),
            spare_nodes: Vec::new(),
            spectrum_hold: panes::spectral::SpectrumHold::default(),
            star_solo: StarSolo::default(),
        }
    }
}

impl PictureState {
    /// An owned handle lets plugin teardown join initialization after releasing
    /// the shared UI lock. Ordinary editor close leaves this cache alive.
    pub fn editor_graphics(&self) -> std::sync::Arc<harmonigraph_render::LatticePipelineCache> {
        self.surfaces.lattice_pipelines.clone()
    }

    /// Put the audio ring back to a standing start — nothing carried, in
    /// either half.
    ///
    /// The two halves are ONE state and are cleared together. Both step
    /// against the clock and both hold at a step of zero, so anything drawing
    /// a SETTING rather than a frame of an animation — a probe taking every
    /// shot at one moment — has to clear both or the shot before is handed
    /// straight back. Clearing [`VisualRuntime::ring_fade`] alone leaves the
    /// new shot's gate reading the previous shot's grid, which is a picture of
    /// neither.
    pub fn reset_ring(&mut self) {
        self.runtime.ring_fade = harmonigraph_scene::RingFade::default();
        self.runtime.ring_levels = crate::panes::spectral_fold::RingLevels::default();
    }
    /// Install an already normalized appearance at a load boundary.
    pub fn install_appearance(&mut self, appearance: crate::AppearanceDocument) {
        self.appearance = appearance;
    }
    /// The block of lattice the picture is currently showing, which is what
    /// every "is this pitch on the lattice" question has to be asked of.
    ///
    /// Three readers ask it through [`Namer`](crate::panes::spectral::names::Namer):
    /// the analyzer's red band checks for a matching node, while the roll's
    /// and the spiral's note names choose a spelling.
    /// Asking the view's REACH instead — which the band used to do — makes it
    /// contradict what the lattice is drawing, because the drawn window is
    /// the camera's and runs wider than the reach under everything but
    /// cabinet: at 16:9, fully zoomed out, perspective draws 73% of its nodes
    /// outside it, so a lit node could wear a red band down the spectrum.
    ///
    /// The name readers ask differently on purpose — the reach first,
    /// this only where the reach comes up empty — so a name does not move
    /// under a pan. The two answers are allowed to differ, and where they do
    /// the picture is what wears the band: a pitch the reach can spell but the
    /// pane is not drawing is named and banded at once.
    ///
    /// The reach is the fallback rather than the answer, for the frame before
    /// the docked lattice first draws and for a frame after one where it was
    /// folded, as it is for a composed layout with no lattice pane in it at
    /// all ([`crate::draw_panes`]). There is no picture to describe
    /// there, and the reach is the only window that does not depend on one.
    pub fn shown(&self) -> DrawnWindow {
        self.surfaces.drawn.unwrap_or_else(|| self.appearance.view.reach())
    }
    /// Tell the state what ground the lattice pane stands on — what it paints,
    /// and what it hands the scene (see the `background` field). Takes sRGB
    /// bytes, the form every shell already has its background color in, so no
    /// shell needs glam to say it.
    pub fn set_background(&mut self, rgb: (u8, u8, u8)) {
        self.surfaces.background = harmonigraph_scene::skin::ground_color(rgb);
    }
    /// The same ground as an egui color, for the pane that paints it.
    ///
    /// Opaque, and it has to be: this fill is what the picture stands on, and
    /// a translucent one would let the dock's own tab body through and put the
    /// lattice back on the panel it was moved off.
    pub(crate) fn background_ink(&self) -> egui::Color32 {
        crate::panes::scene_color(self.surfaces.background, 1.0)
    }
    /// Forget everything that accumulates as the plugin runs: the lattice
    /// trail, the piano roll, the spectrogram, and every node's own light.
    /// Display state only — nothing about the tuning, the take, or the
    /// render.
    ///
    /// Named as a set because the Video pane's "Clear display history" clears it
    /// before a take, and a fifth accumulation would have to join it here or
    /// that button quietly stops living up to its name. Each pane still clears
    /// what it draws — Labels the trail, the Analyzer its roll and spectrogram
    /// together — so this is not a replacement for those buttons, it is the
    /// case where all four are wanted at one moment.
    ///
    /// Held notes fare differently across the four and deliberately so: the
    /// lattice keeps what is still sounding while the roll drops it (see
    /// [`NoteRoll::clear`](harmonigraph_core::NoteRoll::clear)). Each clear
    /// answers that for itself; evening them up here would make this do
    /// something the pane buttons do not. The glow follows the lattice: each
    /// surface's [`GlowFade`](crate::panes::glow_fade::GlowFade) drops the
    /// lights of released notes, cutting the fade they were still riding, and
    /// keeps a sounding note's light where it is. Dropping that one too would
    /// not re-light it at once: a new light starts dark under a Glow attack, so
    /// every held halo would go out and fade back up.
    pub fn clear_accumulated(&mut self) {
        self.runtime.tracker.clear_history();
        self.runtime.tracker.clear_roll();
        self.runtime.spectrum.clear_history();
        for glow in self.surfaces.glow_fade.values_mut() {
            glow.clear_released();
        }
        self.surfaces.note_names.clear();
    }
    /// The causal tracker's rolling window, filling in as notes arrive.
    pub fn roll(&self) -> &harmonigraph_core::NoteRoll {
        self.runtime.tracker.roll()
    }
}
