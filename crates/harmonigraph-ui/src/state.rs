//! [`SharedState`], the one instance of everything the UI reads and mutates
//! each frame, plus what of it survives a session: the dock arrangement,
//! camera, and settings written through [`UiPersist`](crate::state::UiPersist).

use std::collections::VecDeque;

use egui_dock::{DockState, NodeIndex};
use harmonigraph_core::{Comma, LatticePos, NoteTracker, PitchClass, Tuning};
use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_scene::{Camera, FrameParams, ViewConfig};

use crate::perf::{self, PerfStats};
use crate::{fold, panes, text};
use crate::{AudioSpectrum, RenderConfig, RenderProgress, SpectrumConfig, WholeSong};

/// Scrollback for the debug console pane. Shells and panes log via
/// [`SharedState::log`].
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
/// Runtime-only but for [`render_config`](Self::render_config), which is a
/// setting rather than a live flag and persists with the rest of them. A take
/// is a deliberate act, so nothing else here is ever resumed on load — an
/// editor that reopened armed would record a session nobody asked it to.
///
/// In the plugin's editor `self.take` is the recorder and `self.ui.take` is
/// this; the frame-by-frame copying between them is `sync_take`.
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
    /// What to do with a take once it is finished. The one persisted field
    /// here — see [`UiPersist`], which carries it as `render`.
    pub render_config: RenderConfig,
    /// How far the video render running in the background has got, or `None`
    /// when none is. Shell-set every frame, like [`status`](Self::status).
    pub render_progress: Option<RenderProgress>,
}

/// Everything the UI reads and mutates each frame. One instance lives in the
/// shell (inside the editor state in the plugin, inside the app in the
/// standalone harness).
pub struct SharedState {
    pub tracker: NoteTracker,
    /// Snapshot of the tuning parameters, refreshed each frame in
    /// [`root_ui`](crate::root_ui) so core/scene code never touches the param system.
    pub tuning: Tuning,
    pub view: ViewConfig,
    /// Per-frame mirrors of the appearance parameters, refreshed alongside
    /// `tuning` (the param system owns the real values; these are never
    /// persisted).
    pub frame_params: FrameParams,
    pub camera: Camera,
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
    pub console: Console,
    /// Surface format of the shell's swapchain; the lattice render pipeline
    /// must match it.
    pub target_format: TextureFormat,
    /// The color the lattice pane is painted ONTO, which only the sevens
    /// knockout reads (see [`harmonigraph_scene::Scene::background`]). Defaults
    /// to the skin's panel, which is what `egui_dock` fills a tab body with
    /// — right for the plugin and the standalone harness.
    ///
    /// A shell that composes its panes differently MUST set this: the
    /// offline renderer clears to its layout's own background and draws the
    /// panes over that, several shades darker than the panel, and a
    /// knockout clearing to the wrong ground shows up as a disc that is
    /// visibly too light. Exported video is the one place this is hardest
    /// to notice and most expensive to get wrong.
    pub background: glam::Vec4,
    /// While true, tuning params continuously re-learn from the held notes
    /// (v1's learn mode). Runtime-only; never persisted.
    pub learn_active: bool,
    /// Held pitch classes the last learn ran against (change detection).
    pub(crate) last_learned_classes: Option<Vec<PitchClass>>,
    /// Per comma (indexed by [`Comma::index`]): the tuning axes (microcents)
    /// that comma's auto-detect last saw, so it judges each tuning exactly
    /// once.
    ///
    /// This is what lets a comma switch be switched OFF: an unchanged tuning
    /// gets no second verdict, so the mode stays where it was put until the
    /// tuning itself moves. It also keeps the detect off the plugin's
    /// in-flight parameter writes, which report the value being written away
    /// from for a frame or more (see `begin_frame`).
    ///
    /// One entry per comma, and each holds only the axes ITS identity reads
    /// (see `judged_axes`) — a seventh that moved must not re-open the
    /// syntonic question, or dragging the seventh would re-engage a meantone
    /// that was just switched off.
    ///
    /// Runtime-only. A saved project carries the modes themselves, and
    /// reopening one is exactly when the detects should look afresh.
    pub(crate) temper_judged: [Option<(i32, i32, i32)>; Comma::COUNT],
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see the Frame pane).
    pub camera_presets: Vec<CameraPreset>,
    /// Entry buffer for naming a new preset. Runtime-only.
    pub preset_name: String,
    /// Take recording and video export, which the Video pane and the shell
    /// pass between them — see [`TakeState`].
    pub take: TakeState,
    /// Audio-derived spectrum for the Spectral pane. Runtime-only.
    pub spectrum: AudioSpectrum,
    /// The Spectral pane's settings (Analyzer tab; persisted).
    pub spectrum_config: SpectrumConfig,
    /// Offline playhead render: the whole take's spectrogram laid out
    /// statically with a playhead at `now`, instead of the live scrolling
    /// window. `Some` only in the offline renderer. Runtime-only, never
    /// persisted (mirrors `learn_active`).
    pub whole_song: Option<WholeSong>,
    /// Set by the Panel pane's "Reset layout" button; consumed by root_ui
    /// AFTER the frame's DockArea writes the dock back (panes run inside
    /// that pass, so a direct write from one would be overwritten).
    pub(crate) reset_layout: bool,
    pub(crate) dock: DockState<panes::Tab>,
    /// What each sideways fold is holding — the width the window owes a folded
    /// pane when it opens again, which is the one part of the layout that
    /// cannot be read back off the dock (see [`fold`]).
    pub(crate) folds: fold::Folds,
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
    /// The pane layout itself: a width per pane in points, and what the window
    /// is doing to it (see [`fold::Dial`]). Runtime-only — a layout loaded into
    /// a window it was not saved at is seeded from the fractions it finds in
    /// the dock.
    pub(crate) dial: fold::Dial,
    /// What the frame measures and publishes about itself — see
    /// [`Instruments`], which is also where the reason these are not five more
    /// flat fields lives.
    pub instruments: Instruments,
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
}

/// What the frame publishes about ITSELF: the measurements the performance
/// overlay reads, and the side channels the draw callbacks write them through.
///
/// Grouped for the second half, which is the part worth saying out loud. None
/// of the three is contended; in each the concurrency primitive IS the return
/// path, for one of two reasons.
///
/// `roll_notes` and `font_atlas` are an atomic and a `Mutex` because they are
/// written from a `&SharedState` — the roll draws from one, and so does the
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
/// of it into. `font_atlas` is the odd member — a mirror rather than a
/// measurement — and it rides here for the mechanism rather than the meaning:
/// it is the third thing the draw path publishes through a shared reference,
/// taken once per flush and uncontended for exactly the same reason.
///
/// None of it is persisted — `save_persist` builds `UiPersist` field by field,
/// so what is grouped here cannot reach the blob either way.
///
/// What keeps it clear of a RECORDED frame is three different arguments, not
/// one, and the difference is worth having: `timings` and `perf` belong to
/// `root_ui`, which the offline renderer never enters at all. The three side
/// channels it DOES write, every offline frame, because it calls `draw_pane` —
/// but nothing offline reads `lattice_stats` or `roll_notes`, so those are
/// dead writes, and the atlas mirror is a function of the glyphs the frame
/// drew rather than of anything timed. Both `lattice_stats` and `roll_notes`
/// carry wall-clock or per-frame values, so a future offline READ of either is
/// exactly what would break determinism.
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
    /// What the label callback's copy of egui's font atlas holds (see
    /// [`text::AtlasMirror`]). Taken once per flush, uncontended.
    pub(crate) font_atlas: std::sync::Mutex<text::AtlasMirror>,
    /// And what the LATTICE's copy holds. Its node names are drawn inside its
    /// own scene pass, off a texture of its own, and a mirror answers for one
    /// texture — see [`text::AtlasMirror`] for why that makes it a second
    /// mirror rather than a second reader of the first.
    pub(crate) lattice_atlas: std::sync::Mutex<text::AtlasMirror>,
    /// What the shell measured about the previous frame. Written by the shell
    /// before `root_ui` and read once, by `perf::FrameCosts::assemble`; no pane
    /// touches it. The one field here a shell outside this crate writes, which
    /// is why it alone is `pub`.
    pub timings: perf::ShellTimings,
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
            timings: perf::ShellTimings::default(),
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

/// Where the pictures end and the settings column begins, as a fraction of
/// the window's width. The settings column gets what is left.
///
/// It is a named constant because the layout is not the only thing that
/// depends on it. What the column has to clear is the widest thing in it,
/// which is its own TAB BAR — six tab names need 347.5pt laid across it,
/// measured — so this fraction and the window width together decide whether
/// egui_dock draws a scroll bar over the settings.
///
/// Widening the column is what a scroll bar over the TAB BAR costs, and the
/// price is charged to the picture twice over: 0.68 would carry the tab bar
/// down to a window about 1090pt wide instead of 1240, but it also takes 8pt
/// off the Spectral pane, which is already within a few points of being
/// narrower than the perf HUD it has to contain. So the column is not widened
/// on account of a bar that does not appear at 1240pt and wider.
///
/// Nothing checks that automatically. The test that did held the whole column
/// — tab bar and pane content together — to needing no scroll bar of either
/// kind, and it came out when the Nodes pane was allowed to scroll. A pane
/// scrolling is now a normal thing; the TAB BAR overflowing is still not, and
/// it is a figure to re-measure by hand rather than a guarantee.
pub(crate) const SETTINGS_SPLIT: f32 = 0.72;

/// The default pane arrangement: big lattice with the Spectral pane
/// beside it on the right (sharing the pitch intuition: what sounds is
/// what lights up), the tuning column further right, console and notes
/// folded to a tab bar below that. Users can re-dock at runtime; the result
/// persists via UiPersist, and the Panel pane's "Reset layout" button
/// returns here.
pub(crate) fn default_dock() -> DockState<panes::Tab> {
    let mut dock = DockState::new(vec![panes::Tab::Lattice]);
    let surface = dock.main_surface_mut();
    let [lattice, right] = surface.split_right(
        NodeIndex::root(),
        SETTINGS_SPLIT,
        vec![
            // Reading outward from the picture: what the lattice is (its
            // tuning, and how it's framed), then how a note is drawn, the
            // scene around it, the analyzer, video export, and the plugin's
            // own render/layout knobs last.
            panes::Tab::Tuning,
            panes::Tab::Nodes,
            panes::Tab::Scene,
            panes::Tab::Analyzer,
            panes::Tab::Video,
            panes::Tab::Panel,
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
    surface.split_right(lattice, 0.72, vec![panes::Tab::Spectral]);
    dock
}

impl SharedState {
    pub fn new(target_format: TextureFormat) -> Self {
        let dock = default_dock();

        SharedState {
            tracker: NoteTracker::new(),
            tuning: Tuning::default(),
            view: ViewConfig::default(),
            frame_params: FrameParams::default(),
            camera: Camera::default(),
            hovered: None,
            console: Console::default(),
            target_format,
            background: harmonigraph_scene::skin::panel_color(),
            learn_active: false,
            last_learned_classes: None,
            temper_judged: [None; Comma::COUNT],
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take: TakeState::default(),
            spectrum: AudioSpectrum::default(),
            spectrum_config: SpectrumConfig::default(),
            whole_song: None,
            reset_layout: false,
            dock,
            folds: fold::Folds::default(),
            window_width_change: 0.0,
            min_window_width: 0.0,
            dial: fold::Dial::default(),
            instruments: Instruments::default(),
            fps_cap: None,
            ui_scale: default_ui_scale(),
        }
    }

    pub fn log(&mut self, line: impl Into<String>) {
        self.console.log(line);
    }

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
        self.reset_layout = true;
    }

    /// Serialize the parts of the UI worth restoring across sessions
    /// (dock layout, camera, view settings). Parameters are NOT included —
    /// they live in the host's plugin state.
    pub fn save_persist(&self) -> String {
        // RON rather than JSON: dock layout rects can be NaN (before first
        // layout), which JSON cannot round-trip.
        ron::to_string(&UiPersist {
            version: UI_PERSIST_VERSION,
            dock: self.dock.clone(),
            folds: self.folds.clone(),
            camera: self.camera,
            view: self.view.clone(),
            camera_presets: self.camera_presets.clone(),
            spectrum: self.spectrum_config,
            render: self.take.render_config.clone(),
            fps_cap: self.fps_cap,
            ui_scale: self.ui_scale,
        })
        .unwrap_or_default()
    }

    /// Drop everything that belongs to a particular egui context. Shells MUST
    /// call this whenever they build one.
    ///
    /// The plugin's editor creates a brand new `Context` every time its window
    /// opens, while this state lives on across them — so a `TextureHandle`
    /// taken from the previous one survives into the new window looking
    /// perfectly valid. It isn't: `set` on it reaches a context nobody is
    /// drawing any more, and its id names a texture the new renderer never
    /// allocated. The spectrogram simply vanished after hiding and re-showing
    /// the window, and stayed gone, because nothing ever asked for a fresh
    /// handle.
    ///
    /// The label mirror is the same trap one layer along. It holds the atlas
    /// size, scale and fill ratio it last saw and the texel of every glyph
    /// drawn at them — all four readings of one context — and answers "already
    /// uploaded" from them. Carried into a new window it answers for texels in
    /// a texture the new renderer never allocated, so the callback finds no
    /// atlas, paints nothing, and every haloed label stays absent: nothing else
    /// asks for a refresh, because the mirror IS what asks.
    pub fn release_context_resources(&mut self) {
        self.spectrum.release_textures();
        // Both mirrors: the lattice's names are drawn off a texture of their
        // own, and a mirror left answering for the window that closed strands
        // its renderer exactly the same way.
        for mirror in [&mut self.instruments.font_atlas, &mut self.instruments.lattice_atlas] {
            mirror
                .get_mut()
                .expect("the label mirror is never held across a panic")
                .forget_context();
        }
    }

    /// Restore state saved by [`save_persist`](Self::save_persist). Unknown or
    /// corrupt input is ignored (fresh defaults win over a broken restore), and
    /// so is anything older than [`UI_PERSIST_VERSION`].
    ///
    /// Refusing an older blob rather than migrating it is safe because no
    /// older blob can reach this build THROUGH THE HOST. The version reached 2
    /// on 2026-07-23; the plugin's `CLAP_ID` and `VST3_CLASS_ID` changed on
    /// 2026-07-26, three days later. A project saved before the version bump
    /// therefore names a plugin identity this binary does not claim, so the
    /// host never loads us into that slot and its state never arrives here.
    ///
    /// That argument covers the editor and nothing else, and this has two
    /// other callers with no identity gate behind them: the offline renderer
    /// reading a `.take` header, and the standalone reading its `app.ron`. A
    /// take is an archive — `harmonigraph-take` refuses only takes from the
    /// FUTURE — so an old one opens and hands its `ui_state` straight here.
    /// What keeps that harmless is that every take on disk is at the current
    /// version, which the next bump ends for all of them at once; the floor is
    /// mirrored in [`render_config_from_persist`] so the two doors into one
    /// blob at least agree, and a refused take renders wholly at defaults
    /// rather than a recorded frame wrapped around them.
    ///
    /// That is what the identity change bought and what a future one would
    /// buy again: a clean floor under the format. A bump WITHOUT one is a
    /// different matter — it would strand real projects — so raising this
    /// constant means either writing the migration or changing the ids.
    pub fn load_persist(&mut self, serialized: &str) {
        if let Ok(persist) = ron::from_str::<UiPersist>(serialized) {
            if persist.version < UI_PERSIST_VERSION {
                return;
            }
            // The dock being installed is not the one the dial's points were
            // measured against, and its node count cannot say so (see
            // [`fold::Dial::forget`]) — so the load has to. What the incoming
            // layout is dialled to is the fractions in the blob's own dock,
            // plus the widths its folds carry.
            self.dial.forget();
            self.folds = persist.folds;
            self.dock = persist.dock;
            self.camera = persist.camera;
            self.view = persist.view;
            // The incoming project's comma modes are its own, so the verdicts
            // this session reached about the tuning it was showing say nothing
            // about them — and a blob that predates a comma arrives with that
            // mode off, waiting for the detect its serde default just switched
            // on. Held back, an editor that loads a project at the tuning it
            // already had would never look (see `temper_judged`); a host
            // pushing state into a live editor, on undo or a preset change, is
            // exactly that case.
            self.temper_judged = [None; Comma::COUNT];
            // Not a migration: both fit a deserialized blob to what its
            // controls can produce, which a hand-edited RON need not have.
            self.view.sanitize();
            self.camera_presets = persist.camera_presets;
            self.spectrum_config = persist.spectrum;
            self.spectrum_config.sanitize();
            self.take.render_config = persist.render;
            // The render frame's two-way `stacked` flag became a named side,
            // and the `--size` that used to sit in the Options text became the
            // Resolution control. Both changed AFTER the version last moved,
            // so a blob this function accepts can still carry either.
            self.take.render_config.migrate_legacy();
            self.fps_cap = persist.fps_cap;
            // Clamped here rather than only where it is drawn, so the control
            // cannot read out a number the chrome is not at: `set_ui_scale`
            // would take a hand-edited 5.0 down to the top of the range while
            // the bar went on saying 500%.
            self.ui_scale = crate::theme::sane_ui_scale(persist.ui_scale);
        }
    }
}

/// The chrome scale a blob without one loads as: the design size, which is
/// what every project saved before the control existed was drawn at.
///
/// Named rather than `#[serde(default)]`, which for an `f32` is 0.0 — a scale
/// of nothing, and every one of those blobs.
fn default_ui_scale() -> f32 {
    1.0
}

/// The current [`UiPersist`] layout version, and the FLOOR under it. Bumped
/// when the `Tab` set changes shape (rename/split/add/merge), which would
/// otherwise strand the user with missing or doubled tabs.
///
/// A bump costs the whole blob, not the dock alone: `load_persist` refuses
/// anything below this outright, so camera, view, spectrum and render settings
/// all fall back to defaults with it. That is what lets the migrations for
/// older formats be deleted rather than carried forever, and it is why raising
/// this means either writing the migration or changing the plugin ids again
/// (see [`SharedState::load_persist`], which sets out what the id change
/// bought and which of its callers the argument covers).
///
/// 2: Tuning and Frame merged into one tab. A version-1 layout has both, and
/// they now name the same variant — kept as the floor's worked example, since
/// a dock opening with the merged pane in it twice is what the refusal avoids.
pub(crate) const UI_PERSIST_VERSION: u32 = 2;

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize silently falls back to defaults.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UiPersist {
    /// serde(default) reads a pre-versioning blob as version 0, which is below
    /// the floor — [`SharedState::load_persist`] refuses it entirely.
    #[serde(default)]
    pub(crate) version: u32,
    pub(crate) dock: DockState<panes::Tab>,
    /// serde(default) keeps pre-sideways-fold blobs loadable (as nothing
    /// folded, which is what they were).
    #[serde(default)]
    pub(crate) folds: fold::Folds,
    pub(crate) camera: Camera,
    pub(crate) view: ViewConfig,
    /// serde(default) keeps pre-preset persisted blobs loadable.
    #[serde(default)]
    pub(crate) camera_presets: Vec<CameraPreset>,
    /// serde(default) keeps pre-Spectrum-tab blobs loadable.
    #[serde(default)]
    pub(crate) spectrum: SpectrumConfig,
    #[serde(default)]
    pub(crate) render: RenderConfig,
    /// serde(default) keeps pre-cap blobs loadable (as uncapped).
    #[serde(default)]
    pub(crate) fps_cap: Option<f32>,
    /// Pre-scale blobs load at the design size — see [`default_ui_scale`].
    #[serde(default = "default_ui_scale")]
    pub(crate) ui_scale: f32,
}

/// Parse just the render settings out of a persisted UI-state blob — so the
/// offline renderer can default its size, layout, and lead-in to what the take
/// was composed for, without building a whole [`SharedState`].
///
/// The whole [`RenderConfig`] rather than the frame alone, because more than
/// the frame is wanted out here: `main` sizes and lays out from
/// [`frame`](RenderConfig::frame) and starts from
/// [`lead_in`](RenderConfig::lead_in). One door into the blob, so a setting the
/// renderer honours cannot be one somebody forgot to add an accessor for.
///
/// Migrated on the way out. Takes recorded before the four sides carry a
/// `stacked` flag here, and before the Resolution control a `--size` inside
/// `extra_args`; the renderer reading them is the whole reason those shims
/// exist, since re-rendering an old take must still compose the frame it was
/// framed at. See [`RenderConfig::migrate_legacy`].
///
/// Floored like [`SharedState::load_persist`], and it has to be: the offline
/// renderer reads one blob through BOTH, this for the frame it composes at and
/// that for the lattice it draws. Honour it here alone and an old take renders
/// at its recorded size and aspect around a scene nobody dialled in, which
/// reads as a working render rather than a refused blob. Refusing here instead
/// leaves `main`'s `unwrap_or_default` composing at a frame it can see.
///
/// Costs no shim. Both migrations above are NEWER than the floor — the four
/// sides and the Resolution control landed 2026-07-27 and 2026-07-28, the
/// floor moved 2026-07-23 — so every blob they exist for is at the floor
/// already.
pub fn render_config_from_persist(serialized: &str) -> Option<RenderConfig> {
    ron::from_str::<UiPersist>(serialized).ok().filter(|p| p.version >= UI_PERSIST_VERSION).map(
        |persist| {
            let mut render = persist.render;
            render.migrate_legacy();
            render
        },
    )
}

impl SharedState {
    /// Tell the state what ground the lattice is being composited over (see
    /// the `background` field). Takes sRGB bytes, the form every shell
    /// already has its background color in, so no shell needs glam to say it.
    pub fn set_background(&mut self, rgb: (u8, u8, u8)) {
        self.background = harmonigraph_scene::skin::ground_color(rgb);
    }

    /// Forget everything that accumulates as the plugin runs: the lattice
    /// trail, the piano roll, and the spectrogram. Display state only —
    /// nothing about the tuning, the take, or the render.
    ///
    /// Named as a set because the Video pane's "Clear everything" clears it
    /// before a take, and a fourth accumulation would have to join it here or
    /// that button quietly stops living up to its name. The three keep their
    /// own buttons in the panes that own them; this is not a replacement for
    /// them, it is the case where all three are wanted at one moment.
    ///
    /// Held notes fare differently across the three and deliberately so: the
    /// lattice keeps what is still sounding while the roll drops it (see
    /// [`NoteRoll::clear`](harmonigraph_core::NoteRoll::clear)). Each clear
    /// answers that for itself; evening them up here would make this do
    /// something neither of the single buttons does.
    pub fn clear_accumulated(&mut self) {
        self.tracker.clear_history();
        self.tracker.clear_roll();
        self.spectrum.clear_history();
    }
}
