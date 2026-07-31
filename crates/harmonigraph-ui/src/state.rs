//! [`SharedState`], the one instance of everything the UI reads and mutates
//! each frame, plus what of it survives a session: the dock arrangement,
//! camera, and settings written through [`UiPersist`](crate::state::UiPersist).

use std::collections::VecDeque;

use egui_dock::{DockState, NodeIndex};
use harmonigraph_core::{LatticePos, NoteTracker, PitchClass, Tuning};
use harmonigraph_render::wgpu::TextureFormat;
use harmonigraph_scene::{Camera, FrameParams, ViewConfig};

use crate::perf::{self, PerfStats};
use crate::{fold, panes, text};
use crate::{AudioSpectrum, RenderConfig, SpectrumConfig, WholeSong};

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

/// How far a video render running in the background has got.
///
/// Frames rather than a fraction, because frames are what the renderer counts
/// and "3400/5400" says something a filled bar cannot: how long is left, at
/// whatever rate you have been watching it go.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RenderProgress {
    /// Frames written so far.
    pub done: u64,
    /// Frames the render is aiming for — 0 until the renderer has said, which
    /// is a moment into the run (it has a take to read and an encoder to
    /// start first).
    pub total: u64,
}

impl RenderProgress {
    /// The share done, in `0..=1`, or `None` while the total is unknown —
    /// which is not the same as zero, and must not draw as it.
    pub fn fraction(self) -> Option<f32> {
        (self.total > 0).then(|| (self.done as f32 / self.total as f32).clamp(0.0, 1.0))
    }
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
    /// The fifth/third pair (microcents) the meantone switch was last
    /// switched OFF at, while the auto-detect was on. The detect skips
    /// exactly this pair, and nothing else.
    ///
    /// Without it that switch has a dead direction: turning the mode off
    /// hands the derived third to the param, which leaves a pair that IS a
    /// meantone, so the detect re-engages on the next frame and the press
    /// does nothing you can see. With it, "off" holds until the tuning is
    /// something else — any edit to either axis, however small, is a new
    /// pair and the detect gets to decide again.
    ///
    /// Runtime-only. A saved project carries the mode itself, and reopening
    /// one is exactly when the detect should look at the tuning afresh.
    pub(crate) meantone_declined: Option<(i32, i32)>,
    /// User-saved camera angles, applied like the built-in Flat/Isometric
    /// presets (persisted; see the Frame pane).
    pub camera_presets: Vec<CameraPreset>,
    /// Entry buffer for naming a new preset. Runtime-only.
    pub preset_name: String,
    /// Take recording, for offline video rendering. The shell owns the
    /// actual recorder; these three fields are the whole contract.
    /// Runtime-only — a take is a deliberate act, never resumed on load.
    ///
    /// `take_supported` gates the control: shells that cannot record
    /// (or a build without a writer) simply don't show it, rather than
    /// offering a button that does nothing.
    pub take_supported: bool,
    /// Toggled by the Video pane, acted on by the shell.
    pub take_recording: bool,
    /// Whether the transport is actually rolling (capture is happening), as
    /// opposed to armed-and-waiting. Drives the record indicator: a steady dot
    /// while rolling, a breathing one while it waits. Shell-set, runtime-only.
    pub take_rolling: bool,
    /// One-shot: set by the Video pane's "Re-render take" button, consumed by the
    /// shell to render the last take with the CURRENT settings. Runtime-only.
    pub render_now: bool,
    /// Whether a take has been recorded this session — the shell sets it so the
    /// Video pane can offer "Re-render take". Runtime-only.
    pub last_take_ready: bool,
    /// Record the input bus alongside the notes, so the render has a
    /// spectrum and a soundtrack without a separate bounce. Persisted
    /// with the render settings rather than the take state, since it is
    /// a preference, not a live flag.
    pub take_audio: bool,
    /// What to do with a take once it is finished (persisted).
    pub render_config: RenderConfig,
    /// Shell-supplied one-liner shown under the toggle: where the file is
    /// going, how many events, or what went wrong.
    pub take_status: String,
    /// How far the video render running in the background has got, or `None`
    /// when none is. Shell-set every frame, like
    /// [`take_status`](Self::take_status). Runtime-only.
    pub render_progress: Option<RenderProgress>,
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
    /// GPU time of the lattice's passes in milliseconds, as f32 bits, written
    /// by the render callback and read by the performance overlay. 0 means no
    /// reading — the device didn't grant timestamp queries, or none has landed
    /// yet.
    ///
    /// An atomic rather than a return value because the measurement crosses a
    /// boundary the call stack doesn't: it is produced inside egui-wgpu's
    /// paint callback, several frames after the frame that asked for it. Same
    /// shape the plugin already uses to publish its sample rate.
    ///
    /// Runtime-only, never persisted, and never read by the offline renderer —
    /// which also never asks for the feature, so it has no timer to begin with.
    pub(crate) lattice_stats: std::sync::Arc<harmonigraph_render::LatticeStats>,
    /// How many note segments the docked roll handed its paint callback last
    /// frame — the geometry `verts` does NOT see, four vertices at a time
    /// instead of several hundred.
    ///
    /// Reported so the roll's load stays visible while it draws from its own
    /// vertex buffer rather than egui's: without it the overlay would show
    /// the cost vanish with nothing standing in its place, and "is the roll
    /// drawing at all" would have no answer. An atomic for the same reason
    /// `lattice_stats` is one — the roll draws from a `&SharedState`.
    ///
    /// Only the docked pane (surface 0) publishes; the Render preview is a
    /// second roll on screen and reporting its count as THE count would be
    /// wrong, exactly as it is for the preview's lattice.
    pub(crate) roll_notes: std::sync::atomic::AtomicU32,
    /// What the label callback's copy of egui's font atlas holds (see
    /// [`text::AtlasMirror`]). Behind a lock for the same reason the other
    /// per-frame publishing is behind atomics: labels are drawn from a
    /// `&SharedState`. Taken once per flush, uncontended.
    pub(crate) font_atlas: std::sync::Mutex<text::AtlasMirror>,
    /// What the shell measured about the previous frame, for the
    /// performance overlay. Written by the shell before `root_ui` and read
    /// once, by `perf::FrameCosts::assemble`; no pane touches it.
    pub timings: perf::ShellTimings,
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
    /// Rolling frame-rate / CPU / memory numbers for the performance overlay.
    /// Runtime-only; filled and drawn by [`root_ui`](crate::root_ui), never by the offline
    /// renderer (so recorded frames stay deterministic).
    pub(crate) perf: PerfStats,
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
/// It is a named constant because two things have to agree on it: the layout,
/// and the test that holds the settings column to fitting its own content
/// without a scroll bar. What the column has to clear is the widest thing in
/// it, which is its own TAB BAR — six tab names need 347.5pt laid across it,
/// measured — so this fraction and the window width together decide whether
/// egui_dock draws a scroll bar over the settings.
///
/// Widening the column is what a scroll bar over the settings costs, and the
/// price is charged to the picture twice over: 0.68 would carry the tab bar
/// down to a window about 1090pt wide instead of 1240, but it also takes 8pt
/// off the Spectral pane, which is already within a few points of being
/// narrower than the perf HUD it has to contain. So the column is not widened
/// on account of a bar that does not appear at the window this is dialled in
/// for — see `the_settings_column_needs_no_scroll_bar_at_the_window_it_was_
/// dialled_in`, which is what would notice if that stopped being true.
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
            meantone_declined: None,
            camera_presets: Vec::new(),
            preset_name: String::new(),
            take_supported: false,
            take_recording: false,
            take_rolling: false,
            render_now: false,
            last_take_ready: false,
            take_audio: false,
            render_config: RenderConfig::default(),
            take_status: String::new(),
            render_progress: None,
            spectrum: AudioSpectrum::default(),
            spectrum_config: SpectrumConfig::default(),
            whole_song: None,
            reset_layout: false,
            dock,
            folds: fold::Folds::default(),
            window_width_change: 0.0,
            min_window_width: 0.0,
            dial: fold::Dial::default(),
            lattice_stats: {
                let stats = harmonigraph_render::LatticeStats::default();
                stats
                    .gpu_ms
                    .store(harmonigraph_render::GPU_TIME_PENDING, std::sync::atomic::Ordering::Relaxed);
                std::sync::Arc::new(stats)
            },
            roll_notes: std::sync::atomic::AtomicU32::new(0),
            font_atlas: Default::default(),
            timings: perf::ShellTimings::default(),
            fps_cap: None,
            ui_scale: default_ui_scale(),
            perf: PerfStats::default(),
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
            render: self.render_config.clone(),
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
    pub fn release_context_resources(&mut self) {
        self.spectrum.release_textures();
    }

    /// Restore state saved by [`save_persist`](Self::save_persist). Unknown or
    /// corrupt input is ignored (fresh defaults win over a broken restore), and
    /// so is anything older than [`UI_PERSIST_VERSION`].
    ///
    /// Refusing an older blob rather than migrating it is safe because no
    /// older blob can reach this build. The version reached 2 on 2026-07-23;
    /// the plugin's `CLAP_ID` and `VST3_CLASS_ID` changed on 2026-07-26, three
    /// days later. A project saved before the version bump therefore names a
    /// plugin identity this binary does not claim, so the host never loads us
    /// into that slot and its state never arrives here. Versions 0 and 1 are
    /// unreachable by construction, not merely unlikely.
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
            // Not a migration: both fit a deserialized blob to what its
            // controls can produce, which a hand-edited RON need not have.
            self.view.sanitize();
            self.camera_presets = persist.camera_presets;
            self.spectrum_config = persist.spectrum;
            self.spectrum_config.sanitize();
            self.render_config = persist.render;
            // The render frame's two-way `stacked` flag became a named side,
            // and the `--size` that used to sit in the Options text became the
            // Resolution control. Both changed AFTER the version last moved,
            // so a blob this function accepts can still carry either.
            self.render_config.migrate_legacy();
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

/// The current [`UiPersist`] layout version. Bumped when the `Tab` set changes
/// shape (rename/split/add/merge) so `load_persist` can refresh a stale dock
/// instead of stranding the user with missing tabs.
///
/// 2: Tuning and Frame merged into one tab. A version-1 layout has both, and
/// they now name the same variant — without the refresh the dock would open
/// with the merged pane in it twice.
pub(crate) const UI_PERSIST_VERSION: u32 = 2;

/// On-disk format of [`SharedState::save_persist`]. Bump thoughtfully; a
/// failed deserialize silently falls back to defaults.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UiPersist {
    /// serde(default) reads a pre-versioning blob as version 0, which
    /// [`SharedState::load_persist`] treats as "refresh the dock layout".
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
pub fn render_config_from_persist(serialized: &str) -> Option<RenderConfig> {
    ron::from_str::<UiPersist>(serialized).ok().map(|persist| {
        let mut render = persist.render;
        render.migrate_legacy();
        render
    })
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
