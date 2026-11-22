//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod layout;
mod marks;
mod panes;
pub mod params;
mod perf;
/// The open/frame/close sequence a windowed shell runs around [`root_ui`],
/// and the one window floor both shells hold.
pub mod shell;
mod startup;
pub use startup::{begin_editor_loading, editor_loading_status};
pub(crate) mod text;
pub mod theme;
pub mod widgets;

mod workspace;

/// What the UI persists of the analyzer's display settings, and the serde
/// defaults behind them.
mod config;
/// The editor door's per-key decode of a saved document.
mod persist_keys;
mod runtime;
/// The spectrogram heatmap's texture-ring/cache machinery. The pane that
/// draws it, `panes::spectral::spectrogram`, holds only the mesh and the
/// draw call.
mod spectrogram;
/// The analyzer and its heatmap caches, none of it persisted.
mod spectrum;
/// [`SharedState`] and the blob it saves itself into.
mod state;
pub mod tape;
#[cfg(feature = "testing")]
pub mod testing;
/// The Video pane's settings, which the appearance document carries.
mod video;
pub use runtime::{PendingLabel, VisualRuntime, PENDING_AFTER};

pub use layout::{Layout, Placement};

// `config`, `spectrum` and `state` are an arrangement of this file's insides,
// not a change to what the crate exports: everything that was `pub` here is
// re-exported here, so no path outside this crate moves. They are named rather
// than counted because `spectrogram` sits among them and is NOT one of them:
// it came out of `panes::spectral` rather than out of this file, was never
// `pub` here, and is re-exported nowhere.
//
// Four crate-internal names are the exception, and are reached by their own
// path rather than through here: `UI_PERSIST_VERSION` and `TextureFormat`,
// which only the tests wanted, and `sane_scale` and `SpectrogramSurface`,
// which nothing outside their own module wanted at all.
pub use config::{
    NameAnchor, SpectralOrientation, SpectrogramPreset, SpectrumConfig, SpectrumWindow,
    SCALE_BAR_RANGE, TILT_STEPS,
};
pub(crate) use config::{
    LEVEL_MAX_DB, LEVEL_MIN_DB, LEVEL_RANGE_MIN_SPAN, PITCH_RANGE_MIN_SPAN, ROLL_LEAD_MAX,
    ROLL_LEAD_RELEASE_MAX, ROLL_SECONDS_MAX, ROLL_SECONDS_MIN,
};
// The Video settings sit at the crate root like the analyzer's above, which is
// where the panes, both shells and the offline renderer name them. They are
// declared in `video` beside the appearance document that carries them, so a
// new one is one declaration and one name here, in this crate alone.
pub use spectrum::{AnalyzedColumn, AudioSpectrum, Delivery, SpectrumAnalysis};
pub use state::{
    CameraPreset, Console, EditorPrefs, ExportAction, ExportLook, FrameRate, Interaction,
    PictureState, SharedState, SurfaceState, TakeState,
};
pub use video::{
    LatticeSide, RenderConfig, RenderFrame, RenderTrigger, SpectrogramRender, VideoEncoder,
    STOP_BAR_RANGE, TAIL_RANGE,
};

// The overlay's model. `ShellTimings` — the one piece of it a windowed shell
// writes — is deliberately not re-exported from here: the contract runs from
// the shell to the model, and routing it through the crate that only passes it
// along is what made the UI look like its owner.
use harmonigraph_perf::{FrameCosts, Workload};
use params::ParamBackend;

/// End a drag whose release is never coming, because it is holding every
/// scroll area in the editor hostage.
///
/// egui decides whether a `ScrollArea` may take the wheel with
/// `ui.rect_contains_pointer(outer_rect) && ui.ctx().dragged_id().is_none()`
/// (`scroll_area.rs`). That second test is GLOBAL — not "is this area being
/// dragged" but "is anything anywhere being dragged" — so a single stale drag
/// silently stops the wheel in every settings pane at once, and stays that way
/// until something clears it.
///
/// It is not the only way to that symptom. A wheel egui reads as a zoom — Cmd
/// or Ctrl held, or only believed held — never reaches a `ScrollArea` either,
/// while the pictures go on zooming from `zoom_delta`; the shell's half of that
/// is egui-baseview Patch 16 in `PATCHES.md`.
///
/// A stale drag is easy to come by in a plugin window. egui ends a drag on the
/// button release, and deliberately keeps one alive when the pointer merely
/// leaves the viewport ("when dragging a slider and the mouse leaves the
/// viewport, we still want the drag to work" — `input_state`, which is why
/// `PointerGone` does not clear the pressed button either). But a plugin
/// editor is a guest inside a host window: let the host take focus mid-drag,
/// or drag over the editor from a press that was never ours, and the release
/// is delivered somewhere that is not us. egui then believes the button is
/// still down forever.
///
/// So: no pointer, or the focus lost this pass, means no drag. Neither is the
/// ordinary end of a gesture, and a gesture no longer reaches either by going
/// out of the window: the plugin shell holds the pointer's exit back for as
/// long as a button is down (`mouse_exited` in baseview), so a slider
/// dragged past the window edge keeps tracking, and the exit arrives here only
/// once the button is up. What is left for this to catch is a release that
/// never comes at all, where the alternative is every settings pane's wheel
/// dead until the next click.
///
/// A release lost while the window keeps BOTH the focus and the pointer — a
/// host popup handed the mouse-up, a modal opened under the finger — is
/// indistinguishable from a gesture still going by either signal above. Two
/// answers cover it, and only one of them is here. The shell's is the
/// authority: the plugin's frame tick asks the OS whether the button is really
/// held and synthesises the release it is owed (`settle_stuck_buttons` in
/// baseview), which is a question neither egui nor this can put. What
/// this has of its own is the WHEEL. A notch is a hand doing something that is
/// not holding a button, so a drag whose widget the pointer is not even over
/// any more is finished, whatever egui still believes the button is doing —
/// and the wheel is exactly the input that the stale drag is stopping, so the
/// evidence arrives at the moment it is needed.
///
/// The exception the rule has to leave is the picture panes, where
/// scroll-to-zoom during a drag-to-orbit is ONE gesture rather than two. There
/// the pointer is over the widget being dragged, which is what separates the
/// two cases without any pane having to declare which kind it is.
///
/// Ending the drag rather than merely reporting it is what makes the first
/// notch scroll: egui spreads a wheel event over the frames that follow it, and
/// the dock's scroll areas run later in this same pass.
///
/// The answer is a line for the Console, and only for the wheel arm. Losing the
/// pointer or the focus mid-drag is ordinary — every gesture let go outside the
/// window ends that way — but a drag the wheel has to step over is a release
/// that went missing AND was not repaired by the shell, which is the state
/// worth naming out loud while it is still reproducible.
///
/// Not a `Sense::drag` problem in any one pane — a ValueBar strands the wheel
/// exactly as well as the Analyzer's pan does, which is why this sits once at
/// the root rather than in whichever pane the drag came from.
fn end_stranded_drag(ctx: &egui::Context) -> Option<String> {
    let dragged = ctx.dragged_id()?;
    let (at, wheeled) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            i.events.iter().any(|e| matches!(e, egui::Event::MouseWheel { .. })),
        )
    });
    if lost_focus_this_pass(ctx) || at.is_none() {
        ctx.stop_dragging();
        return None;
    }
    if !wheeled {
        return None;
    }
    // Asked outside the `input` closure: `read_response` takes the context's
    // write lock, and reaching for it from under the read one deadlocks. It
    // answers from the previous pass, which is the only place a rect exists
    // this early in this one.
    let held = ctx.read_response(dragged).map(|r| r.rect);
    if held.zip(at).is_some_and(|(rect, at)| rect.contains(at)) {
        return None;
    }
    ctx.stop_dragging();
    Some(match held {
        Some(r) => format!(
            "wheel: a drag on {:.0},{:.0} {:.0}x{:.0} outlived its release",
            r.min.x,
            r.min.y,
            r.width(),
            r.height()
        ),
        None => "wheel: a drag on an undrawn widget outlived its release".to_owned(),
    })
}

/// Whether the editor lost the keyboard this pass, which ends whatever
/// gesture is in flight.
///
/// A host that takes focus mid-drag is handed the release, so what is left here
/// is a button egui believes is held forever — and anything of ours that follows
/// the pointer for as long as it is held would follow it around the screen (see
/// `workspace::Runtime`). Losing focus is the one unambiguous end of a gesture: a
/// pointer that has merely left the window is still dragging, and still ours.
///
/// The end is the frame focus is LOST, read from the `WindowFocused(false)`
/// event, not the state of being unfocused that `InputState::focused` holds.
/// Both shells set that flag, and it stays false for as long as the editor
/// lacks the keyboard: a window hovered but never clicked, which is ordinary
/// with the host in front. A wheel or pinch over such a window is still one
/// gesture, so on the flag its edits would split into many undo entries and
/// host gestures. A shell that sends no event (the offline renderer, the tests)
/// never ends one this way.
fn lost_focus_this_pass(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false))))
}

/// Whether a pointer gesture is still under way this frame, so the edits it
/// makes are one: one undoable look edit (`AppearanceEditor::end_frame`) and
/// one host parameter gesture (`camera_movement::finish_edits`).
///
/// A drag on any button counts, as egui starts a drag on a press of any of
/// them and the panes navigate on `dragged()`. A wheel or pinch holds no
/// button and is one gesture all the same: egui keeps `is_scrolling` true
/// between a trackpad's events and until ~150 ms after the last wheel event,
/// and a picture may zero the frame's scroll delta once it has taken it, so the
/// delta says nothing about whether the wheel is still turning. Any scroll in
/// the window holds the gesture open, so two edits made inside one continuous
/// scroll merge.
///
/// A held button alone does not: it asks for something dragged as well. A drag
/// [`end_stranded_drag`] gives up on has lost its release, and egui believes
/// that button held until the next one arrives; on the button alone, every edit
/// until the user's next click would join one undo entry and one host gesture.
pub(crate) fn gesture_open(ctx: &egui::Context) -> bool {
    !lost_focus_this_pass(ctx)
        && (ctx.dragged_id().is_some() && ctx.input(|i| i.pointer.any_down())
            || ctx.input(|i| i.is_scrolling() || i.zoom_delta() != 1.0))
}

/// Draw one frame of the whole UI into `ui`, which is expected to cover the
/// window (egui-baseview hands the plugin editor exactly that; eframe hands
/// the standalone harness the same via its `App::ui` hook).
///
/// The rest of the sequence a shell runs around this — theming a new context,
/// the fold floor, the window session, and
/// spending what a fold leaves the window needing — is [`shell`], which both
/// shells call rather than keep in step by hand.
///
/// The one step still owed by the caller: before calling this, feed the
/// frame's MIDI into `state.picture.runtime.tracker` and its audio samples into
/// `state.picture.runtime.spectrum`. `now` is seconds on the shell's clock, and must be the
/// SAME clock that timestamped those `NoteEvent`s — envelopes are derived from
/// the difference.
///
/// That step cannot move into [`shell::Frame`] with the others, and the reason
/// is a borrow rather than taste: a shell's rings and the parameters it draws
/// from live in ONE struct — the plugin's `PluginParamBackend` reads a field of
/// the same `EditorShared` its drain writes through — so a helper that ran the
/// feed while holding the state would be holding the backend's own borrow.
/// Both shells therefore feed first and hand over what they fed.
/// `build_tag` is the identity stamped into that shell's binary; it reaches
/// only the interactive performance overlay.
pub fn root_ui(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    params: &dyn ParamBackend,
    now: f64,
    build_tag: &str,
) {
    // Loading frames still receive notes and automation from the shell.
    begin_frame(&mut state.picture, params, now);
    // The skin before anything paints, the loading screen included: every
    // color accessor reads it, and on a thread shared with another editor the
    // one in force is whichever that editor last set. A scale change below
    // rebuilds the style from it.
    let reskinned = theme::set_skin(ui.ctx(), state.workspace.interaction.prefs.skin_dials);
    if startup::draw(ui, state) {
        return;
    }
    if let Some(stranded) = end_stranded_drag(ui.ctx()) {
        state.picture.runtime.console.log(stranded);
    }

    // The chrome scale, before anything lays out at the old one. The shell
    // built `ui` from the style as it stood on the way in, so a scale that has
    // just moved leaves this frame's copy a size behind — and the frame it
    // would be behind on is the one being dragged, where every intermediate
    // size shows. `reset_style` takes the rebuilt one from the context, which
    // `set_ui_scale` has already put there.
    if theme::set_ui_scale(ui.ctx(), state.workspace.interaction.prefs.ui_scale) || reskinned {
        ui.reset_style();
    }
    // Cleared before the panes run, so a frame with the roll hidden (or the
    // Spectral pane not on screen at all) reports zero notes rather than
    // whatever the last frame that had one reported.
    state.picture.instruments.roll_notes.store(0, std::sync::atomic::Ordering::Relaxed);

    // Hide chrome for a continuous live picture. Tab stays available even
    // when hidden bars leave no route to System settings or a folded pane.
    // It takes precedence over keyboard focus traversal outside text edits;
    // cancelling the queued focus move keeps the toggle from moving focus too.
    // Bare Tab alone, as the shell reserves it. A Tab with a modifier goes to
    // the host, so it moves no focus here either. A focused text field, a
    // value being typed included, owns bare Tab and Shift-Tab by the shell's
    // text-field rule, and egui moves the focus on them to the next or the
    // previous control; claimed here as well for a frame that ends the edit,
    // whose Tab the next frame toggles on.
    if ui.ctx().text_edit_focused() {
        keys::claim(ui.ctx(), &[FRAMELESS_SHORTCUT]);
    } else {
        let tab = ui.input(|i| i.key_pressed(egui::Key::Tab));
        if keys::take(ui.ctx(), FRAMELESS_SHORTCUT) > 0 {
            state.workspace.interaction.prefs.frameless =
                !state.workspace.interaction.prefs.frameless;
        }
        if tab {
            ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        }
    }
    let before_camera = state.picture.appearance.camera_movement();
    let before_edit = appearance_edit::Look::capture(&state.picture.appearance);
    let text_edit_was_focused = ui.ctx().text_edit_focused();
    let cpu_start = std::time::Instant::now();
    let workspace = &mut state.workspace;
    let frameless = workspace.interaction.prefs.frameless;
    if let Some(change) = workspace::show(
        ui,
        &mut workspace.layout,
        &mut state.reported.layout,
        &mut workspace.layout_runtime,
        &mut panes::Viewer {
            state: &mut state.picture,
            interaction: &mut workspace.interaction,
            params,
            now,
        },
        frameless,
    ) {
        workspace.window_size_change = change;
    }
    state.workspace.interaction.appearance_editor.end_frame(
        before_edit,
        &mut state.picture.appearance,
        ui.ctx(),
        text_edit_was_focused,
    );
    camera_movement::finish_edits(&mut state.picture.appearance, before_camera, params, ui.ctx());
    let cpu_ms = cpu_start.elapsed().as_secs_f32() * 1000.0;

    // Each picture drawn says when it next changes on screen (#1911): at once
    // while a value still fades (sounding or decaying voices, a release, a
    // glow, the smoothed spectrum), or when a scroll next crosses a device
    // pixel (the roll and the heatmap after the sound, a Tape playhead). The
    // earliest wins, and with nothing sooner than one poll the idle poll
    // draws, which bounds how stale a change that animates nothing gets: an automated
    // parameter, the console, a take's status. egui repaints on input events
    // by itself, so interaction never waits on this, and a note repaints
    // itself, since it makes a voice.
    //
    // Audio that merely arrives does not count: a host streams silence with
    // its transport stopped (#1670 Q2).
    let animating = Animating::of(state);
    match animating.next_change() {
        // As fast as the shell offers. The frame rate setting is the shell's
        // to apply, by offering fewer frames: a repaint delay cannot bound it,
        // because egui takes the smallest delay asked for in a pass and any
        // input or hover asks for zero.
        Some(seconds) if seconds <= 0.0 => ui.ctx().request_repaint(),
        // The shell draws on the first refresh at or after the delay, so a
        // scroll steps one to two device pixels, never more.
        Some(seconds) => {
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(seconds))
        }
        None => ui.ctx().request_repaint_after(IDLE_REPAINT_INTERVAL),
    }
    // What the overlay judges against the chosen rate: a picture asking for
    // every frame the link offers, a fast scroll included.
    let target_fps = state.picture.instruments.timings.target_fps;
    let animating = animating.every_frame(target_fps);

    // Performance overlay: fold this frame's numbers in and, if it's on, draw
    // the HUD. Interactive path only — the offline renderer never reaches
    // root_ui, so nothing here touches a recorded frame.
    state.picture.instruments.perf.record(
        FrameCosts::assemble(
            state.picture.instruments.timings,
            cpu_ms,
            &state.picture.instruments.lattice_stats,
            state.picture.instruments.roll_notes.load(std::sync::atomic::Ordering::Relaxed),
            state.picture.surfaces.spectrogram.full_uploads(),
        ),
        now,
        Workload {
            active_voices: state.picture.runtime.tracker.voices().count(),
            held_voices: state.picture.runtime.tracker.held_count(),
            visible_nodes: state.picture.surfaces.drawn_this_frame.map_or(0, |w| w.count()),
            render_scale: state.picture.appearance.view.render_scale,
            animating,
        },
    );
    if state.workspace.interaction.prefs.show_perf {
        // The whole editor, which is the region the HUD may be dragged around
        // in — where inside it the HUD sits is the user's, and `perf_pos` is
        // where that answer lives.
        perf::draw_overlay(
            ui.ctx(),
            ui.max_rect(),
            &mut state.workspace.interaction.prefs.perf_pos,
            &state.picture.instruments.perf,
            state.workspace.interaction.prefs.show_perf_detail,
            build_tag,
        );
    }
    // Last, after the overlay's drag has written where the HUD sits.
    state.end_frame(ui.ctx());
}

/// Everything that must happen once per frame before any pane draws:
/// follow the host's camera and tuning parameters and age out voices whose
/// fade has completed.
///
/// [`root_ui`] calls this itself. It is public for shells that compose
/// their own layout instead of using the dock — the offline renderer
/// draws [`Pane`]s directly, and skipping this would leave it rendering
/// last frame's tuning against never-pruned voices.
pub fn begin_frame(state: &mut PictureState, params: &dyn ParamBackend, now: f64) {
    state.appearance.sync_camera(params);
    state.runtime.advance(&state.appearance, params, now);

    // Rotated here so the window belongs to a whole frame rather than to a
    // point in the dock's draw order: the lattice publishes as it builds, and
    // the panes that describe the picture read the finished answer from the
    // frame before. Taking it also clears this frame's slot, so a frame where
    // the lattice is not drawn at all — its section folded, or laid out too
    // small to draw — reports no window rather than going on showing the last
    // one that was. A diagnostic that holds its last good reading is the one
    // that misleads.
    state.surfaces.drawn = state.surfaces.drawn_this_frame.take();
    // And for the same reason, what the pictures drawn say about when they
    // stop moving: one not drawn this frame holds the loop for nothing.
    state.surfaces.moving_this_frame = Animating::default();
}

/// A pane that stands on its own, outside the dock.
///
/// Only the *views* are here. The settings panes edit state that a
/// non-interactive renderer cannot change and a viewer should not see, so
/// they are deliberately unreachable this way.
///
/// The Spiral is not one: it is an editor tab only, never exported (#974).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    /// The 3D lattice.
    Lattice,
    /// The spectrum curve, spectrogram and piano roll.
    Spectral,
}

/// Draw a composed frame — each pane at its placement, with no dock or tab
/// bar — as an export does. Callers must have run [`begin_frame`] for this
/// `now` already. These are picture copies: the lattice omits editor
/// navigation, picking, map annotations and Learn-mode chrome, as the Video
/// preview does.
///
/// The lattice draws first, and the spectral pane describes the window it
/// returns: the one it drew this frame, in the rect egui allocated it (#1833).
/// Unlike the dock, whose panes draw in the order the user arranged them and so
/// read the window of the frame before (see [`begin_frame`]), a composed frame
/// has one answer for the frame it draws; reading the frame before's put a
/// false off-lattice band down an export's first frame and lagged camera
/// automation by a frame. With no lattice placement the reach is the window,
/// as [`PictureState::shown`] falls back to it. Nothing here publishes to
/// [`SurfaceState::drawn`](state::SurfaceState::drawn), which is the docked
/// lattice's.
///
/// A placement's `surface` is its index in `placements`, and it is what makes
/// two placements of one pane two pictures instead of one picture drawn twice.
/// Every pane here holds something between frames — a GPU instance buffer, a
/// bloom chain, a folded slab grid, a text batch — keyed on its surface, and
/// egui-wgpu runs every `prepare` before any `paint`, so two copies sharing a
/// key do not merely share work: the second tears down what the first built and
/// the first then paints the second's picture across its own rect.
///
/// The index rather than a count of the placements of this pane, because it is
/// what a caller already has in hand and it holds still for as long as the copy
/// does; a key that changed as a layout was read would churn the state it
/// names. [`Layout::resolve`] drops a placement too small to draw, so the index
/// is into what it returned rather than into the file — which is the same list
/// every frame of a render, the size being fixed for the whole of one.
pub fn draw_panes(
    ui: &mut egui::Ui,
    placements: &[(Pane, egui::Rect)],
    state: &mut PictureState,
    now: f64,
) {
    debug_assert!(layout::lattice_first(placements), "{placements:?}");
    let mut shown = None;
    for (surface, &(pane, rect)) in placements.iter().enumerate() {
        let ui = &mut ui.new_child(egui::UiBuilder::new().max_rect(rect));
        match pane {
            Pane::Lattice => {
                let (rect, _) = ui.allocate_exact_size(ui.available_size(), egui::Sense::hover());
                if rect.width() < 1.0 || rect.height() < 1.0 {
                    continue;
                }
                ui.painter().rect_filled(rect, 0.0, state.background_ink());
                // Exported pictures share the preview's scene path, without
                // the working pane's navigation, picking, or Learn-mode chrome.
                let window = panes::lattice::draw_lattice(
                    ui,
                    rect,
                    state,
                    now,
                    surface,
                    state.surfaces.background,
                    None,
                    None,
                    1.0,
                    panes::lattice::NO_SOLO,
                );
                shown = shown.or(Some(window));
            }
            // Text sizes itself off the pane, here as everywhere.
            Pane::Spectral => {
                let shown = shown.unwrap_or_else(|| state.appearance.view.reach());
                let navigation = panes::spectral::Navigation::None;
                let solo = panes::lattice::NO_SOLO;
                panes::spectral::spectral_pane(
                    ui, state, now, surface, 1.0, navigation, shown, solo,
                );
            }
        }
    }
}

/// Repaint cadence while nothing animates: newly arriving MIDI shows up
/// within one poll even without an input event.
const IDLE_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// Repaint cadence of the chrome's own cues, about 30 Hz: the armed Record
/// button's breath and a running export's progress stripes. Each asks for its
/// next frame this far out rather than holding the whole editor at the
/// display rate: a full redraw at 144 Hz costs Bitwig's main thread (#1673),
/// and these run for as long as Record waits armed or an export runs. They are not [`Reason`]s, which are the picture's
/// motion alone, so the frame log reads these passes as the idle-rate draws
/// they are (#1872).
pub(crate) const CHROME_REPAINT_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(33);

/// [`CHROME_REPAINT_INTERVAL`] in seconds, for a picture's cue that
/// [`Animating::hold_for`] paces as the chrome's are: Learn's armed pulse.
pub(crate) const CHROME_CUE: f64 = CHROME_REPAINT_INTERVAL.as_secs_f64();

/// Declares [`Reason`] from one list, each reason beside the name the frame
/// log gives it, so a reason added or renamed is written once.
macro_rules! reasons {
    ($($(#[$doc:meta])* $reason:ident => $name:literal,)*) => {
        /// Each reason the editor draws sooner than its idle poll, apart so the
        /// frame log can say which one held it there (#1670 Q2).
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Reason {
            $($(#[$doc])* $reason,)*
        }

        impl Reason {
            /// Every reason, in the order the frame log lists them.
            const ALL: &'static [Reason] = &[$(Reason::$reason),*];

            /// The word the frame log's `animating` row uses for it.
            pub fn name(self) -> &'static str {
                match self {
                    $(Reason::$reason => $name,)*
                }
            }
        }
    };
}

reasons! {
    /// A voice the tracker still holds, sounding or fading. The one reason
    /// read rather than reported: [`Animating::of`] asks the tracker.
    Voices => "voices",
    /// Learn's armed pulse, drawn where it can be seen: the lattice's badge
    /// and border, or the Tuning pane's ring round its switch. Learn armed
    /// where neither draws — Lattice Map, which suspends it, or both out of
    /// sight — leaves a still picture to the idle poll (#1834). A cue like
    /// the Record button's breath, so it asks for frames at the chrome's
    /// [`CHROME_REPAINT_INTERVAL`] rather than every refresh (Yan, #1911).
    LearnPulse => "learn",
    /// A note the live roll draws, as a ribbon, a name or a Lead, still inside
    /// its own Span: its body has not yet scrolled past the far edge, or its
    /// Lead is still fading through its release. A Lead fading holds every
    /// frame; a ribbon or a name only scrolls, so it asks for the frame where
    /// the roll next crosses a device pixel (#1911). One off the pitch zoom
    /// draws nothing and so holds nothing, and an overhang still sliding off
    /// the far edge finishes at the idle poll (Yan, #1861 on #1835). Tape
    /// mode's roll holds nothing: its song axis does not scroll.
    RollInSpan => "roll",
    /// A lattice node still moving after its voice was pruned
    /// ([`NodeMotion::moving`](harmonigraph_scene::NodeMotion::moving): a
    /// departure's Fade-out delay, its stagger and the presence it holds for
    /// them), or a node's Background glow still moving on a layer that draws
    /// it: its level falling, or its colour still easing toward its reading
    /// ([`GlowFade::step_paced`](panes::glow_fade::GlowFade::step_paced)),
    /// which can outlast the slices by many seconds. The departure asks for
    /// every frame; the glow asks for the frame its lights next move half a
    /// code value, which a slow release reaches less often than the display
    /// refreshes (#1915).
    Releasing => "releasing",
    /// A picture drawn from audio above silence. Samples arriving are not
    /// enough, because a host streams silence with its transport stopped and
    /// drawing that at full rate cost Bitwig's main thread about 30% for a
    /// still picture (#1670 Q2). So each picture drawn from the spectrum says
    /// whether what it drew is still moving:
    ///
    /// - The smoothed display the curve, the Spiral and the ring read
    ///   ([`AudioSpectrum::moving`]), off when its columns last drew anything:
    ///   a curve fallen under the faintest dB any reading draws holds nothing,
    ///   while silence still arrives (#1917 Q2).
    /// - Each heatmap surface, off the run and slab it drew, its Span and its
    ///   atmosphere's tail
    ///   ([`draw_spectrogram`](panes::spectral::spectrogram::draw_spectrogram)):
    ///   every frame while the display it reads moves or Stars twinkle, and
    ///   otherwise paced by the device pixel its scroll or its cloud's drift
    ///   next crosses (#1911).
    /// - The lattice ring's levels, carried on its own attack and release
    ///   ([`RingLevels`](panes::spectral_fold::RingLevels)), and whether each
    ///   wedge is drawn, on the note Fade
    ///   ([`RingFade`](harmonigraph_scene::RingFade)). That fade is the whole of
    ///   a ring-posed node's leaving: the pose itself snaps back behind a
    ///   presence already at zero, so nothing in `Releasing` moves for it.
    ///
    /// A stage downstream of another answers only for its own state: while the
    /// stage it reads is still moving, that stage holds the loop, and each
    /// frame it holds steps the next one on what it reads now.
    Audible => "audible",
    /// A Tape mode playhead the editor draws moving visibly across the
    /// stretch of song shown: an audition's, or the host's position while the
    /// transport rolls, still advancing on the axis as it moves, paced at the
    /// time it takes to cross a device pixel (`marks` in the Analyzer's Tape
    /// mode, #1911). Not the transport rolling or an audition playing as such
    /// (#1837).
    Playhead => "playhead",
}

/// When each reason next changes the picture, as the pictures drawn this frame
/// report it.
///
/// Each picture drawn says when it next changes on screen, from the state or
/// stamp its own motion runs on, so a delay a picture gains reaches the repaint
/// guard with no edit here (#1677 R1): at once while a value still fades
/// ([`hold`](Self::hold)), or when a scroll next crosses a device pixel
/// ([`hold_for`](Self::hold_for)), since a pass between two pixels draws a
/// picture that differs from the last by under a pixel (#1911). Learn's pulse
/// asks for the chrome's 30 Hz. The shell's copy
/// ([`moving_this_frame`](SurfaceState::moving_this_frame)) is cleared by
/// [`begin_frame`] beside [`drawn_this_frame`](SurfaceState::drawn_this_frame),
/// and for its reason: a picture not drawn this frame — its pane folded or
/// hidden, its layer off, the Video tab closed — moves nothing, so it holds
/// nothing, whatever its last draw said. One revealed again reports on its
/// first draw. Read by [`Animating::of`] once every pane has drawn, in the
/// same pass and so at the same `now`: each answer is about the picture its
/// own draw just made.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Animating([f64; Reason::ALL.len()]);

impl Default for Animating {
    /// Nothing held.
    fn default() -> Self {
        Animating([f64::INFINITY; Reason::ALL.len()])
    }
}

impl Animating {
    /// What the pictures drawn this frame reported, and the tracker's voices.
    pub fn of(state: &SharedState) -> Self {
        let mut held = state.picture.surfaces.moving_this_frame;
        held.hold_if(Reason::Voices, state.picture.runtime.tracker.voices().next().is_some());
        held
    }

    /// Hold the loop for `reason` at every frame the shell offers.
    pub(crate) fn hold(&mut self, reason: Reason) {
        self.hold_for(reason, 0.0);
    }

    /// Hold the loop for `reason` at every frame if `moving`.
    pub(crate) fn hold_if(&mut self, reason: Reason, moving: bool) {
        if moving {
            self.hold(reason);
        }
    }

    /// Ask for a frame `seconds` from now for `reason`, unless it already
    /// asked for one sooner. One at or past the idle poll holds nothing: the
    /// poll draws it as soon, so a scroll that slow, or a mark standing still
    /// to within rounding, is a still picture.
    pub(crate) fn hold_for(&mut self, reason: Reason, seconds: f64) {
        if seconds < IDLE_REPAINT_INTERVAL.as_secs_f64() {
            let slot = &mut self.0[reason as usize];
            *slot = slot.min(seconds.max(0.0));
        }
    }

    /// Whether `reason` asked for any frame, at once or paced.
    pub fn asks_for(self, reason: Reason) -> bool {
        self.after(reason).is_some()
    }

    /// Seconds until the frame `reason` asked for, if it asked for one.
    pub fn after(self, reason: Reason) -> Option<f64> {
        Some(self.0[reason as usize]).filter(|seconds| seconds.is_finite())
    }

    /// Whether any reason asked for a frame sooner than the idle poll.
    pub fn asks(self) -> bool {
        self.0.iter().any(|seconds| seconds.is_finite())
    }

    /// Seconds until the earliest frame any reason asked for.
    pub fn next_change(self) -> Option<f64> {
        Some(self.0.iter().copied().fold(f64::INFINITY, f64::min)).filter(|s| s.is_finite())
    }

    /// Whether the editor wants every frame the link offers at `target_fps`
    /// (60 where nothing paces it, as the standalone): a value fading, or a
    /// scroll that crosses a device pixel within one frame.
    pub fn every_frame(self, target_fps: Option<f32>) -> bool {
        let frame = 1.0 / f64::from(target_fps.unwrap_or(60.0));
        self.next_change().is_some_and(|seconds| seconds < frame)
    }
}

/// The names of the reasons that ask for a frame, space-separated: what the
/// frame log's `animating` row carries. One asking for a later frame than the
/// next carries `~`, as `roll~`.
impl std::fmt::Display for Animating {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let held = Reason::ALL.iter().filter(|&&r| self.asks_for(r));
        for (i, &reason) in held.enumerate() {
            let paced = if self.0[reason as usize] > 0.0 { "~" } else { "" };
            write!(f, "{}{}{paced}", if i == 0 { "" } else { " " }, reason.name())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

pub mod lattice_maps;

mod appearance;
mod appearance_edit;
pub mod keys;

/// Toggles the hidden chrome.
const FRAMELESS_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::Tab);
mod camera_movement;
pub mod framing;
pub use appearance::AppearanceDocument;
