//! Shell ownership, synchronous visual runtime, viewport resources and editor
//! workspace. Persisted documents are assembled explicitly at the shell boundary.

use std::collections::VecDeque;

use egui_dock::{DockState, NodeIndex};
use harmonigraph_core::{Comma, LatticePos};
use harmonigraph_perf::{PerfStats, ShellTimings};
use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_scene::{Camera, DrawnWindow};

use crate::{fold, panes, text};
use crate::{RenderProgress, VisualRuntime};

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
/// The plugin owns the recorder separately. `sync_take` exchanges its status
/// and explicit actions with this workspace interaction state once per GUI callback.
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
    /// Whether a take has been recorded this session — the shell sets it so the
    /// Video pane can offer "Re-render take".
    pub last_ready: bool,
    /// One-shot: set by the Video pane's "Re-render take" button, consumed by
    /// the shell to render the last take with the CURRENT settings.
    pub render_now: bool,
    /// One-shot: set by the Video pane's "Cancel render" button, consumed by
    /// the shell to stop the render in flight and delete the part of the video
    /// it had written. The take itself is kept, so
    /// [`render_now`](Self::render_now) can start over from it.
    pub cancel_render: bool,
    /// How far the video render running in the background has got, or `None`
    /// when none is. Shell-set every frame, like [`status`](Self::status).
    pub render_progress: Option<RenderProgress>,
}

/// Shell aggregate. Drawing and runtime code borrow its domains independently.
pub struct SharedState {
    pub picture: PictureState,
    pub workspace: Workspace,
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
    pub(crate) spectrogram: crate::spectrum::SpectrogramSurfaces,
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
    /// the lattice" band, the Notes pane's node column, and the name a pitch
    /// gets when the reach cannot spell it. `None` until a lattice pane has
    /// drawn one; [`PictureState::shown`] is what to read, and it falls back to
    /// the view's reach.
    ///
    /// Reported rather than computed from the view, because the view holds no
    /// window to compute it from: it is derived per pane from that pane's own
    /// aspect ([`harmonigraph_scene::ViewConfig::scrolled`]), so the only
    /// place it exists is inside the draw that built it.
    ///
    /// LAST frame's, and that is what makes it safe to read from another pane:
    /// the dock draws its panes in whatever order the user has arranged them,
    /// so a band asking about THIS frame would answer from the reach or from
    /// the window depending on where the lattice sits in the layout. A frame
    /// of lag on a window that only moves with the camera is invisible; an
    /// answer that changes with the dock arrangement is not.
    ///
    /// The DOCKED copy alone. The Video tab's preview is a second lattice at
    /// a second aspect, and letting it publish would make these answers jump
    /// with a tab that is not the one being read — the same argument that
    /// keeps the preview out of the GPU timing slot.
    pub drawn: Option<DrawnWindow>,
    /// What the docked lattice has published so far THIS frame, rotated into
    /// [`drawn`](Self::drawn) by `begin_frame`. The perf overlay's node count
    /// reads it directly, because that read happens after every pane has
    /// drawn and a diagnostic holding its last good reading is the one that
    /// misleads.
    pub drawn_this_frame: Option<DrawnWindow>,
    /// Surface format of the shell's swapchain; the lattice render pipeline
    /// must match it.
    pub target_format: TextureFormat,
    /// Device-bound compiled pipelines outlive windows, while their pane
    /// buffers and textures remain in each window's callback resources.
    pub(crate) lattice_pipelines: std::sync::Arc<harmonigraph_render::LatticePipelineCache>,
    /// The ground the lattice pane paints its rect with, which it also hands
    /// the scene (see [`harmonigraph_scene::Scene::background`]). Defaults to
    /// the skin's well, the recessed grey every picture pane paints — right for
    /// the plugin and the standalone harness.
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
    /// Where every node's own light has got to, and which row of the ink strip
    /// is keeping its colour, per lattice surface
    /// ([`GlowFade`](crate::panes::glow_fade::GlowFade)). Runtime-only.
    ///
    /// Here for the reason the two above it are: the offline renderer carries
    /// this struct between frames and nothing else, so a light kept anywhere
    /// else would linger live and not in an export. Keyed by the surface id
    /// each lattice pane claims, because the strip it describes is that pane's
    /// own texture.
    pub(crate) glow_fade: std::collections::HashMap<usize, crate::panes::glow_fade::GlowFade>,
    /// Where the analyzer's divider stands on the DOCKED pane as that pane is
    /// resized — the spectrum keeps its size and the spectrogram takes the
    /// difference. See [`panes::spectral::SpectrumHold`].
    ///
    /// Runtime-only, and it is the answer rather than the setting: the dial
    /// itself stays in [`appearance.spectrum`](crate::AppearanceDocument::spectrum), because that
    /// is what a project saves and a take renders from, and a length in POINTS
    /// is a fact about the window this session happens to be open in.
    pub(crate) spectrum_hold: panes::spectral::SpectrumHold,
}

/// Editor interaction and shell actions. Panes borrow this separately from
/// the layout being traversed, so a reset request cannot replace a live dock.
pub struct Interaction {
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see the Lattice page's Camera section).
    pub camera_presets: Vec<CameraPreset>,
    /// Entry buffer for naming a new preset. Runtime-only.
    pub preset_name: String,
    /// Take recording and video export, which the Video pane and the shell
    /// pass between them — see [`TakeState`].
    pub take: TakeState,
    /// Which of the Display tab's pages is showing (persisted).
    ///
    /// Here rather than in egui `Context` memory, and the home is load-bearing:
    /// the plugin builds a brand new `Context` every time the editor window
    /// opens (the trap [`PictureState::release_context_resources`] sets out), so
    /// a choice kept in memory springs back to Colors with every reopen. The
    /// picker writes clicks straight here and reads the body to draw off the
    /// same field, so there is one source of truth for it.
    pub display_page: panes::display::DisplayPage,
    /// Upper bound on how often the UI is drawn, in frames per second;
    /// `None` leaves it uncapped (as fast as the display can present).
    /// Persisted.
    ///
    /// Read by the shells to pace themselves, and by [`root_ui`](crate::root_ui) only to
    /// schedule repaints — never by any drawing code. The offline renderer
    /// steps its own clock and never reaches `root_ui`, so a recorded frame
    /// cannot depend on this and the determinism test stays honest.
    ///
    /// The repaint request alone cannot enforce this, and shells must not
    /// rely on it: egui takes the SMALLEST delay any caller asks for in a
    /// pass, and a zero-delay `request_repaint` (an input event, a hover
    /// animation, the plugin's own MIDI-drain repaint) additionally forces
    /// the following pass to zero. A cap expressed that way evaporates
    /// exactly when the UI is busy — the case it exists for. The plugin
    /// therefore drives its window's frame timer from this value, which is a
    /// hard bound because a frame that is never asked for is never drawn.
    pub fps_cap: Option<f32>,
    /// How big the panel chrome draws — type, spacing, control heights, tab
    /// bars — as a multiple of the design size. Persisted. See
    /// [`crate::theme::ui_scale`], which is where it takes effect and where
    /// the reasoning lives.
    ///
    /// A property of the SCREEN the plugin is open on rather than of the
    /// piece, which is why it is here beside `fps_cap` and not in `view`:
    /// `ViewConfig` is what a recorded frame is composed from, and a laptop
    /// dialling its panel down must not change what a render of the same
    /// project comes out looking like. [`root_ui`](crate::root_ui) is the
    /// only thing that reads it, and the offline renderer never reaches
    /// there.
    pub ui_scale: f32,
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
    pub(crate) reset_layout: bool,
}

/// Editor docking and interaction. The dock borrows its own tree while the
/// Viewer borrows only `interaction` and the independent picture. Shell actions
/// are consumed after traversal; saving explicitly selects persisted fields.
pub struct Workspace {
    pub interaction: Interaction,
    /// The arrangement itself: which panes are where, which tab of a leaf is
    /// selected, and which leaves are collapsed. egui_dock owns everything in
    /// it — the collapsed flags a fold reads are its own (see [`fold`]) — and
    /// it is the one member here with no default to fall back on, which is why
    /// a blob missing it costs the whole document rather than this field alone
    /// (see [`UiPersist`]).
    pub(crate) dock: DockState<panes::Tab>,
    /// What each sideways fold is holding — the width the window owes a folded
    /// pane when it opens again, which is the one part of the layout that
    /// cannot be read back off the dock (see [`fold`]).
    pub(crate) folds: fold::Folds,
    /// The pane layout itself: a width per pane in points, and what the window
    /// is doing to it (see [`fold::Dial`]). Runtime-only — a layout loaded into
    /// a window it was not saved at is seeded from the fractions it finds in
    /// the dock.
    pub(crate) dial: fold::Dial,
    /// Points the window has to gain (or lose, if negative) before the next
    /// frame, because a pane folded sideways or came back and every other pane
    /// is keeping its width.
    ///
    /// The UI cannot resize the window itself — the plugin has to ask its
    /// host, the standalone harness its windowing system — so it says how many
    /// points and the shell spends them. Logical points, which is what both
    /// shells size their windows in.
    ///
    /// Runtime-only, and TAKEN rather than read (see
    /// [`take_window_width_change`](Self::take_window_width_change)), so a
    /// shell that never asks — the offline renderer, which never reaches
    /// `root_ui` at all — simply never resizes.
    pub(crate) window_width_change: f32,
    /// The narrowest the shell will let its window become, in the same points
    /// [`take_window_width_change`](Self::take_window_width_change) is answered
    /// in. At the floor a window has stopped answering, and the pane layout
    /// stops following it (see [`fold`]) — otherwise a fold the window will not
    /// shrink far enough for would re-dial the layout to the window it got
    /// rather than the one it asked for, and hand the difference back on the
    /// way out.
    ///
    /// Set by the shell. Zero — the default, and what a shell that never
    /// resizes leaves it at — means no floor.
    pub min_window_width: f32,
}

impl Default for Workspace {
    fn default() -> Self {
        Workspace {
            dock: default_dock(),
            folds: fold::Folds::default(),
            dial: fold::Dial::default(),
            window_width_change: 0.0,
            min_window_width: 0.0,
            interaction: Interaction::default(),
        }
    }
}

impl Workspace {
    /// How much wider (or, negative, narrower) the window has to be for the
    /// sideways folds the last frame settled — `None` when it can stay as it
    /// is, which is nearly every frame.
    ///
    /// Shells call this once per frame, AFTER [`root_ui`](crate::root_ui), and
    /// resize by the points they are given. Taking it rather than reading it
    /// is what keeps one fold to one resize: a shell whose host refuses the
    /// new size is not asked again on the next frame, since asking forever
    /// would fight the host over every frame for as long as the pane stays
    /// folded.
    ///
    /// Changes under half a point are dropped rather than passed on, which is
    /// where rounding to whole pixels stops moving a window at all: below it a
    /// shell would ask for the size it already has, and never be satisfied.
    pub fn take_window_width_change(&mut self) -> Option<f32> {
        let change = std::mem::take(&mut self.window_width_change);
        (change.abs() >= 0.5).then_some(change)
    }

    /// Discard the (persisted) dock arrangement and return to the default
    /// layout. Camera, view settings, and presets are untouched. Takes
    /// effect at the end of the frame (see the `reset_layout` field).
    pub fn reset_dock_layout(&mut self) {
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
/// channels it DOES write, every offline frame, because it calls `draw_pane` —
/// but nothing offline reads `lattice_stats` or `roll_notes`, so those are
/// dead writes, and the atlas trackers depend only on the context's glyph
/// assets. Both `lattice_stats` and `roll_notes` carry wall-clock or per-frame
/// values, so a future offline READ of either is exactly what would break
/// determinism.
pub struct Instruments {
    /// GPU time of the lattice's passes in milliseconds, as f32 bits, written
    /// by the render callback and read by the performance overlay. Carries the
    /// `GPU_TIME_UNSUPPORTED` / `GPU_TIME_PENDING` sentinels, which are NaN bit
    /// patterns rather than zero — a lattice pass below the timer's resolution
    /// is a real reading of 0.0 ms, so zero cannot mean "nothing" here. See the
    /// seed in [`Instruments::default`], which is what stops a fresh editor
    /// reporting a fabricated 0.0 before the first readback lands.
    ///
    /// Same shape the plugin already uses to publish its sample rate. Never
    /// read by the offline renderer, which also never asks for the feature, so
    /// it has no timer to begin with.
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
    /// The label callback's context-local fallback font and mark publication
    /// state (see [`text::AtlasMirror`]). Taken once per flush, uncontended.
    pub(crate) font_atlas: std::sync::Mutex<text::AtlasMirror>,
    /// The same publication state for the lattice callback, whose node names
    /// draw inside its own scene pass and whose mark sheet has a GPU copy of
    /// its own.
    pub(crate) lattice_atlas: std::sync::Mutex<text::AtlasMirror>,
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
            lattice_stats: {
                let stats = harmonigraph_render::LatticeStats::default();
                // The sentinel that says "no reading has landed yet", which the
                // overlay draws as `—` rather than as a zero. Set here rather
                // than being `LatticeStats`'s own default: zero is a legitimate
                // GPU time, so the distinction belongs to whoever is going to
                // read it back.
                stats.gpu_ms.store(
                    harmonigraph_render::GPU_TIME_PENDING,
                    std::sync::atomic::Ordering::Relaxed,
                );
                std::sync::Arc::new(stats)
            },
            roll_notes: std::sync::atomic::AtomicU32::new(0),
            font_atlas: Default::default(),
            lattice_atlas: Default::default(),
            timings: ShellTimings::default(),
            perf: PerfStats::default(),
        }
    }
}

/// A saved camera angle: what the built-in Flat/Isometric buttons are,
/// but user-defined. Only the orbit angles — projection, zoom, and pan
/// are deliberately not captured, so a preset composes with any of them.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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

/// Where the pictures end and the settings column begins, as a fraction of
/// the window's width. The settings column gets what is left.
///
/// It is a named constant because the layout is not the only thing that
/// depends on it. What the column has to clear is the widest thing in it,
/// which is its own TAB BAR — this fraction and the window width together
/// decide whether egui_dock scrolls the tab bar over the settings. Three tabs
/// is what makes the bar fit at the editor's own `DEFAULT_SIZE` (#287): a tab
/// per settings pane wants a window of about 1428pt at this fraction,
/// measured, which is why those panes are pages of the Display tab rather than
/// tabs of their own.
///
/// Widening the column is what scrolling the TAB BAR would cost, and the price
/// is charged to the picture: a smaller fraction buys the bar a narrower window
/// to survive, and takes that width straight off the Spectral pane, which is
/// the narrowest picture the default layout has. So the column is not widened
/// on account of the bar.
///
/// `every_settings_tab_fits_on_its_tab_bar` (in `tests::shell`, so not linkable
/// from here) is what checks it — at `DEFAULT_SIZE` and at the window the UI
/// is dialled against — by the CLIP rather than by re-deriving egui_dock's
/// sums.
/// It holds the tab bar alone, and has to: a pane scrolling is a normal thing,
/// so a guard over tab bar and pane content together fires on the panes that
/// are meant to scroll.
pub(crate) const SETTINGS_SPLIT: f32 = 0.72;

/// The default pane arrangement: big lattice with the Spectral pane
/// beside it on the right (sharing the pitch intuition: what sounds is
/// what lights up), the tuning column further right, console and notes
/// folded to a tab bar below that. Users can re-dock at runtime; the result
/// persists via UiPersist, and the System page's "Reset layout" button
/// returns here.
pub(crate) fn default_dock() -> DockState<panes::Tab> {
    let mut dock = DockState::new(vec![panes::Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [lattice, right] = surface.split_right(
        NodeIndex::root(),
        SETTINGS_SPLIT,
        vec![
            // Reading outward from the picture: what the lattice is, then how
            // everything is drawn (the Display pages run the same way one level
            // down, out to the machine around the pictures), and video export
            // last.
            panes::Tab::Tuning,
            panes::Tab::Display,
            panes::Tab::Video,
        ],
    );
    // Notes first so it sits left of Console and is the selected tab by
    // default (egui_dock makes tab index 0 active).
    let [_, log] = surface.split_below(right, 0.55, vec![panes::Tab::Notes, panes::Tab::Console]);
    // Folded to its tab bar, because neither pane is looked at while playing:
    // Notes is a readout of what the tracker already draws on the lattice and
    // Console is a diagnostic. Open they take 45% of the settings column's
    // height, which is the half of it the settings themselves want -- see the
    // scroll every settings pane carries.
    //
    // The COLLAPSE ARROW is what brings them back, not the tab name: egui_dock
    // reaches `set_collapsed` from the arrow's own square alone, and clicking
    // "Notes" on a folded bar only selects a tab whose body stays hidden. The
    // split fraction survives the fold, so the pane comes back the size it
    // went away.
    //
    // A vertical fold, so egui_dock does the whole of it; `Folds` only exists
    // for the horizontal ones (see `fold`).
    //
    // This is the DEFAULT, which is to say it reaches a fresh instance and
    // "Reset layout" and nothing else. A project that has saved a layout keeps
    // the one it saved, since the arrangement is persisted and
    // `UI_PERSIST_VERSION` is bumped for a changed tab SET rather than a
    // changed default -- and throwing away a dialed-in layout to deliver a
    // default is the worse trade.
    surface[log].set_collapsed(true);
    // Spectral as a column just right of the lattice: what sounds is directly
    // beside what lights up. Paired with the "Left" default orientation
    // (SpectrumConfig::default), which is the one that reads under the lattice
    // and beside it alike. Drag it wherever from here — egui_dock docks it
    // freely, and the orientation stays where it was set rather than following
    // the shape the pane lands in.
    //
    // Spiral shares that leaf rather than taking room of its own, and is the
    // second tab there so the Analyzer is still what opens (egui_dock makes tab
    // index 0 active). The two are one analyzer drawn two ways off one
    // `SpectrumConfig`, so they are alternatives to switch between rather than
    // pictures to watch at once — and a disc wants a square, which is the one
    // shape a tall column beside the lattice is not.
    surface.split_right(lattice, 0.72, vec![panes::Tab::Spectral, panes::Tab::Spiral]);
    dock
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        Self { picture: PictureState::new(target_format), workspace: Workspace::default() }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.picture.runtime.console.log(line);
    }

    /// Serialize the parts of the UI worth restoring across sessions
    /// (dock layout, camera, view settings). Parameters are NOT included —
    /// they live in the host's plugin state.
    ///
    /// `LatticeEditorHandle::Drop` writes this enclosing editor document into
    /// `params.ui_state` when the window closes. Recording instead serializes
    /// the live appearance directly, so capture never depends on that last save.
    pub fn save_persist(&self) -> String {
        // RON rather than JSON: dock layout rects can be NaN (before first
        // layout), which JSON cannot round-trip.
        ron::to_string(&UiPersist {
            version: UI_PERSIST_VERSION,
            dock: self.workspace.dock.clone(),
            folds: self.workspace.folds.clone(),
            display_page: self.workspace.interaction.display_page,
            appearance: self.picture.appearance.clone(),
            camera_presets: self.workspace.interaction.camera_presets.clone(),
            fps_cap: self.workspace.interaction.fps_cap,
            ui_scale: self.workspace.interaction.ui_scale,
            perf_pos: self.workspace.interaction.perf_pos,
        })
        .unwrap_or_default()
    }

    /// Restore the whole editor document. Refused input leaves the current state
    /// intact and reports why on the console. Appearance normalization is shared
    /// with recording/export; workspace restoration remains editor-only.
    pub fn load_persist(&mut self, serialized: &str) -> bool {
        let persist = match ron::from_str::<UiPersist>(serialized) {
            Ok(persist) => persist,
            // SAYING SO is the whole point of this arm. Nothing in the tree
            // reads an older spelling any more, so a blob naming a variant
            // this build has dropped — a retired orientation or sweep mode —
            // fails the parse HERE, and what falls out is the dock, the camera
            // and every view setting reverting at once. A dropped KEY is the
            // other case entirely and costs nothing: serde skips one it has no
            // field for, which is how a blob still naming `node_style` or
            // `spectrogram_color` loads intact. The
            // version floor cannot catch it: the version is read out of a
            // value that never parsed. An accepted break, but not a silent
            // one — a project opening at defaults with no explanation reads
            // as data loss, and this is the difference between that and a
            // break someone chose.
            Err(err) => {
                self.log(format!("persist ignored — the blob did not parse ({err})"));
                return false;
            }
        };
        if persist.version < UI_PERSIST_VERSION {
            self.log(format!(
                "persist ignored — version {} is below the floor of {UI_PERSIST_VERSION}",
                persist.version,
            ));
            return false;
        }
        let appearance = match persist.appearance.normalize() {
            Ok(appearance) => appearance,
            Err(err) => {
                self.log(format!("persist ignored — {err}"));
                return false;
            }
        };
        // The dock being installed is not the one the dial's points were
        // measured against, and its node count cannot say so (see
        // [`fold::Dial::forget`]) — so the load has to. What the incoming
        // layout is dialled to is the fractions in the blob's own dock,
        // plus the widths its folds carry.
        self.workspace.dial.forget();
        self.workspace.folds = persist.folds;
        self.workspace.dock = persist.dock;
        self.workspace.interaction.display_page = persist.display_page;
        self.picture.install_appearance(appearance);
        self.workspace.interaction.camera_presets = persist.camera_presets;
        for preset in &mut self.workspace.interaction.camera_presets {
            preset.sanitize();
        }
        self.workspace.interaction.fps_cap = persist.fps_cap;
        // Clamped here rather than only where it is drawn, so the control
        // cannot read out a number the chrome is not at: `set_ui_scale`
        // would take a hand-edited 5.0 down to the top of the range while
        // the bar went on saying 500%.
        self.workspace.interaction.ui_scale = crate::theme::sane_ui_scale(persist.ui_scale);
        // A hand-edited NaN is dropped rather than honoured, on the grounds
        // the spiral framing above is repaired on: it positions drawn
        // geometry, and NaN geometry is a panic inside egui's tessellator. A
        // dropped position opens the HUD where an undragged one opens, which
        // is a place the user can see it and drag it from.
        self.workspace.interaction.perf_pos = persist.perf_pos.filter(|pos| pos.is_finite());
        true
    }
}

/// The chrome scale a blob without one loads as: the design size, which is
/// what a fresh install opens at.
///
/// Named rather than `#[serde(default)]`, which for an `f32` is 0.0 — a scale
/// of nothing.
fn default_ui_scale() -> f32 {
    1.0
}

/// The current [`UiPersist`] layout version, and the FLOOR under it. Bumped
/// when the `Tab` set changes shape (rename/split/add/merge), which would
/// otherwise strand the user with missing or doubled tabs.
///
/// A bump costs the whole blob, not the dock alone: `load_persist` refuses
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
/// [`SharedState::load_persist`], which sets out both halves and which of its
/// callers each covers.
///
/// 2: Tuning and Frame merged into one tab. A version-1 layout has both, and
/// they now name the same variant — kept as the floor's worked example, since
/// a dock opening with the merged pane in it twice is what the refusal avoids.
///
/// 3: that merge undone, and `Panel` renamed to `System`. Two breaks, and only
/// one of them would reach this check. The RENAME fails the parse outright —
/// `Panel` is a variant no build has any more — so a version-2 blob dies in
/// `load_persist`'s `Err` arm before the version is ever read, which is loud
/// and is why that arm says what it says. The SPLIT is the one this floor is
/// for, and it is the quieter of the two: a version-2 dock names only `Tuning`,
/// which still parses and still draws, so without a bump an old project would
/// open with the tuning bars intact and the whole camera simply absent, no tab
/// to reach it by and nothing said. That is the silent break the floor exists
/// to turn into an audible one.
///
/// 4: `View`, `Nodes`, `Scene` and `Analyzer` merged into the Display tab's
/// collapsible sections (#287 — four tabs is what fits the default window).
/// The same two-break shape as 3. A version-3 dock still holding any of the
/// four names a retired variant and dies in the `Err` arm, loudly, before the
/// version is read. The floor is for the layout that had CLOSED all four: it
/// parses and draws, and without a bump it would open with no Display tab —
/// camera, note styling and analyzer knobs all unreachable, nothing said, and
/// no mechanism to re-add a missing tab but "Reset layout".
///
/// 5: the Spiral pane added (#342). The only ADDITION in this list, and it is
/// the quiet half of 3 and 4 on its own: nothing in a version-4 blob names a
/// variant this build has dropped, so it parses and draws perfectly — with no
/// Spiral tab anywhere in it, and no way to add one but "Reset layout". A tab
/// that exists in the binary and in no project is the same silent break from
/// the other direction, so it is the same floor that answers it.
///
/// 6: the System tab retired into the Display tab's System page. The same
/// two-break shape as 3 and 4, and the loud half takes nearly every real blob:
/// a saved dock holding `System` names a variant no build has any more, so it
/// dies in `load_persist`'s `Err` arm before the version is read. The floor is
/// for the dock that had already dragged the tab away — it parses and draws,
/// and what it then carries is a tab list the binary and the project disagree
/// about, with the Display tab possibly dropped too and the four pages behind
/// it reachable only by "Reset layout". That is the same silent break 5's
/// addition is, from the subtraction side, and it is the same floor that
/// answers it.
///
/// 7: camera, view, spectrum, spiral and render moved into one appearance
/// document. Previous editor saves are refused whole, with no migration.
pub(crate) const UI_PERSIST_VERSION: u32 = 7;

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize reports refusal and leaves the current state intact.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UiPersist {
    /// serde(default) reads a pre-versioning blob as version 0, which is below
    /// the floor — [`SharedState::load_persist`] refuses it entirely.
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) dock: DockState<panes::Tab>,
    // Workspace sections default independently. The dock is required because
    // it has no Default; appearance has its own container-level defaults.
    #[serde(default)]
    pub(crate) folds: fold::Folds,
    #[serde(default)]
    pub(crate) display_page: panes::display::DisplayPage,
    #[serde(default)]
    pub(crate) appearance: crate::AppearanceDocument,
    #[serde(default)]
    pub(crate) camera_presets: Vec<CameraPreset>,
    /// A missing cap reads as uncapped.
    #[serde(default)]
    pub(crate) fps_cap: Option<f32>,
    /// A blob without one loads at the design size — see [`default_ui_scale`],
    /// and note that this is the BLOB's one field-level `default = "..."`,
    /// because an `f32`'s own default of 0.0 is a scale of nothing. The
    /// offline renderer's `Layout::background` is the tree's only other, and
    /// answers to a different rule: a hand-written `.ron` rather than saved
    /// state, so it defaults its fields one at a time and requires `panes`.
    #[serde(default = "default_ui_scale")]
    pub(crate) ui_scale: f32,
    /// Where the performance overlay was dragged to; a blob without one opens
    /// it where an undragged HUD opens. See [`Interaction::perf_pos`].
    #[serde(default)]
    pub(crate) perf_pos: Option<egui::Pos2>,
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
            hovered: None,
            drawn: None,
            drawn_this_frame: None,
            target_format,
            lattice_pipelines: Default::default(),
            background: harmonigraph_scene::skin::picture_color(),
            glow_fade: std::collections::HashMap::new(),
            spectrum_hold: panes::spectral::SpectrumHold::default(),
        }
    }
}
impl Default for Interaction {
    fn default() -> Self {
        Self {
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take: TakeState::default(),
            display_page: panes::display::DisplayPage::default(),
            fps_cap: None,
            ui_scale: default_ui_scale(),
            perf_pos: None,
            reset_layout: false,
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
        // A restored project must judge its comma modes again even at the
        // tuning the previous project already showed.
        self.runtime.temper_judged = [None; Comma::COUNT];
    }
    /// Drop everything that belongs to a particular egui context. Shells MUST
    /// call this whenever they build one.
    ///
    /// The plugin's editor creates a brand new `Context` every time its window
    /// opens, while this state lives on across them — so anything here that
    /// describes what a context's renderer holds survives into the new window
    /// looking perfectly valid. The spectrogram's GPU mirror is exactly that: it
    /// states which slabs the grid buffer holds, and a frame writes only the
    /// slabs that have moved against it, so carried into a window whose renderer
    /// allocated nothing it would patch two slabs of a buffer that was never
    /// written.
    ///
    /// The label trackers carry the fallback atlas guards and the mark sheet's
    /// publication key, all of which describe one context. Carrying them into
    /// another context can suppress the first publication to its renderer.
    pub fn release_context_resources(&mut self) {
        self.surfaces.spectrogram.release_gpu_grids();
        // Each callback owns its fallback and mark texture, so each publication
        // tracker describes the context that closed.
        for mirror in [&mut self.instruments.font_atlas, &mut self.instruments.lattice_atlas] {
            mirror
                .get_mut()
                .expect("the label mirror is never held across a panic")
                .forget_context();
        }
    }
    /// The block of lattice the picture is currently showing, which is what
    /// every "is this pitch on the lattice" question has to be asked of.
    ///
    /// Two readers ask it and they must agree, because they are describing the
    /// same picture: the analyzer's red band says a sounding note has no node,
    /// and the Notes pane's column says which node. Asking the view's REACH
    /// instead — which is what they both did — makes them contradict what the
    /// lattice is drawing, because the drawn window is the camera's and runs
    /// wider than the reach under everything but cabinet: at 16:9, fully
    /// zoomed out, perspective draws 73% of its nodes outside it, so a lit
    /// node could wear a red band down the spectrum.
    ///
    /// The analyzer's NAME is a third reader and asks differently on purpose —
    /// the reach first, this only where the reach comes up empty — so a name
    /// does not move under a pan. The two answers are allowed to differ, and
    /// where they do the picture is what wears the band: a pitch the reach can
    /// spell but the pane is not drawing is named and banded at once. See
    /// [`note_name`](crate::panes::spectral::names).
    ///
    /// The reach is the fallback rather than the answer, for the frame before
    /// the first lattice draw and for a layout with no lattice pane in it at
    /// all. There is no picture to describe there, and the reach is the only
    /// window that does not depend on one.
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
    /// Named as a set because the Video pane's "Clear everything" clears it
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
    /// something the pane buttons do not. The glow is the one exception —
    /// dropping every surface's [`GlowFade`](crate::panes::glow_fade::GlowFade)
    /// does not spare a held note's light, because a light re-seeds rather than
    /// fading up from nothing (see [`glow_fade::apply`](crate::panes::glow_fade::apply)),
    /// so a still-sounding node is lit again on the very next frame — this
    /// only cuts the fade a released node's light was still riding.
    pub fn clear_accumulated(&mut self) {
        self.runtime.tracker.clear_history();
        self.runtime.tracker.clear_roll();
        self.runtime.spectrum.clear_history();
        self.surfaces.glow_fade.clear();
    }
    /// The roll currently on screen: the take's own, laid out statically, in
    /// offline playhead mode; the causal tracker's rolling window, filling in
    /// as notes arrive, live.
    pub fn roll(&self) -> &harmonigraph_core::NoteRoll {
        match self.runtime.whole_song.as_ref() {
            Some(ws) => &ws.roll,
            None => self.runtime.tracker.roll(),
        }
    }
}
