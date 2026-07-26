//! The shared UI shell: dockable panes, cross-pane hover state, and the
//! per-frame root function. Both the standalone harness and the plugin
//! editor call [`root_ui`] once per egui frame; everything else is internal.

pub mod layout;
pub mod params;
pub mod theme;
/// Haloed label text, collected as glyphs and drawn by one callback.
pub(crate) mod text;
pub mod widgets;
mod panes;
mod perf;

/// Folding a pane sideways, which egui_dock's own collapse arrow only does
/// downwards.
mod fold;

/// What the UI persists: the analyzer's display settings and the render
/// settings, with the serde defaults that keep older blobs loadable.
mod config;
/// The analyzer and its heatmap caches, none of it persisted.
mod spectrum;
/// [`SharedState`] and the blob it saves itself into.
mod state;

pub use layout::{Layout, Placement, PRESETS};

// The three modules above are an arrangement of this file's insides, not a
// change to what the crate exports: everything that was `pub` here is
// re-exported here, so no path outside this crate moves.
//
// Four crate-internal names are the exception, and are reached by their own
// path rather than through here: `UI_PERSIST_VERSION` and `TextureFormat`,
// which only the tests wanted, and `sane_scale` and `SpectrogramSurface`,
// which nothing outside their own module wanted at all.
pub use config::{
    RenderConfig, RenderFrame, RenderTrigger, SpectralOrientation, SpectrogramColor,
    SpectrumConfig, SpectrumLabels, SpectrumWindow, SCALE_BAR_RANGE, TILT_STEPS,
};
pub(crate) use config::{
    COLOR_RANGE_MIN_SPAN, LEVEL_MAX_DB, LEVEL_MIN_DB, LEVEL_RANGE_MIN_SPAN, PITCH_RANGE_MIN_SPAN,
    ROLL_SECONDS_MAX, ROLL_SECONDS_MIN,
};
pub use spectrum::{AudioSpectrum, SpectrogramColumn, SpectrumHistory, WholeSong};
pub(crate) use spectrum::{SpectrogramCache, SpectrogramKey};
pub use state::{render_frame_from_persist, CameraPreset, Console, SharedState};
pub(crate) use state::default_dock;

use harmonigraph_core::PitchClass;
use harmonigraph_scene::FrameParams;
use params::ParamBackend;

use egui_dock::{DockArea, DockState};

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
/// A stale drag is easy to come by in a plugin window. egui ends a drag on the
/// button release, and deliberately keeps one alive when the pointer merely
/// leaves the viewport ("when dragging a slider and the mouse leaves the
/// viewport, we still want the drag to work" — `input_state`, which is why
/// `PointerGone` does not clear the pressed button either). But a plugin
/// editor is a guest inside a host window: let go outside it, or let the host
/// take focus mid-drag, and the release is delivered somewhere that is not us.
/// egui then believes the button is still down forever.
///
/// So: no pointer, or no focus, means no drag. The gesture it costs is
/// resuming a drag that wandered out of the window and came back, which is
/// what egui's rule buys; the gesture it saves is scrolling any settings pane,
/// which is otherwise dead until the next click. Only drags started INSIDE the
/// window and released inside it survive, and those are all of them in
/// practice.
///
/// Not a `Sense::drag` problem in any one pane — a ValueBar strands the wheel
/// exactly as well as the Analyzer's pan does, which is why this sits once at
/// the root rather than in whichever pane the drag came from.
fn end_stranded_drag(ctx: &egui::Context) {
    if ctx.dragged_id().is_none() {
        return;
    }
    // Focus is read from the EVENT as well as the flag. `InputState::focused`
    // comes from `RawInput::focused`, which starts true and only moves if an
    // integration sets it — and the plugin's (vendored egui-baseview) reports
    // focus by pushing `WindowFocused` and filling in `ViewportInfo`, never
    // that field. The flag alone would therefore be true forever in the one
    // shell this is most needed in. Reading both means neither a shell that
    // sets the flag nor one that only sends the event is missed, and a shell
    // that says nothing either way (the offline renderer, the tests) is
    // untouched.
    let lost = ctx.input(|i| {
        i.pointer.latest_pos().is_none()
            || !i.focused
            || i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))
    });
    if lost {
        ctx.stop_dragging();
    }
}

/// Draw one frame of the whole UI into `ui`, which is expected to cover the
/// window (egui-baseview hands the plugin editor exactly that; eframe hands
/// the standalone harness the same via its `App::ui` hook).
///
/// The shell contract, which is otherwise only discoverable by reading both
/// shells: before calling this, feed the frame's MIDI into `state.tracker`
/// and its audio samples into `state.spectrum`. `now` is seconds on the
/// shell's clock, and must be the SAME clock that timestamped those
/// `NoteEvent`s — envelopes are derived from the difference.
pub fn root_ui(ui: &mut egui::Ui, state: &mut SharedState, params: &dyn ParamBackend, now: f64) {
    begin_frame(state, params, now);
    end_stranded_drag(ui.ctx());

    // Cleared before the panes run, so a frame with the roll hidden (or the
    // Spectral pane not on screen at all) reports zero notes rather than
    // whatever the last frame that had one reported.
    state.roll_notes.store(0, std::sync::atomic::Ordering::Relaxed);

    // Frameless mode hides every tab bar (the Lattice and Spectral panes
    // meet with no chrome between them — clean for captures). The pane
    // separators keep their regular width, so the spacing between windows
    // matches framed mode. No tab bar also means no way to click back to
    // the Panel pane (which holds the toggle) if it's hidden, so Esc always
    // restores.
    if state.view.frameless && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        state.view.frameless = false;
    }
    let mut dock_style = theme::dock_style(ui.style());
    if state.view.frameless {
        dock_style.tab_bar.height = 0.0;
    }

    // DockState has to be moved out while panes borrow the rest of `state`.
    let mut dock = std::mem::replace(&mut state.dock, DockState::new(vec![]));
    // Before the dock lays out: a pane collapsed inside a horizontal split
    // folds sideways to a rail, which is a split fraction, which is layout's
    // input. egui_dock's own vertical folds need nothing from us.
    state.folds.apply(&mut dock, &dock_style);
    // Time the whole dock build — every pane's layout and the scene
    // derivation — as the GUI thread's own per-frame CPU cost. The wgpu draw
    // is submitted inside and finishes off-thread, so this is CPU, not GPU.
    let cpu_start = std::time::Instant::now();
    DockArea::new(&mut dock)
        // Cloned because the rails are painted from the same style afterwards.
        .style(dock_style.clone())
        // The pane set is fixed, so closing chrome stays off — but the
        // collapse arrow earns its pixels: the Lattice and Spectral panes
        // fold down to their tab bar when screen space is tight.
        .show_close_buttons(false)
        .show_leaf_close_all_buttons(false)
        .show_leaf_collapse_buttons(true)
        .show_inside(ui, &mut panes::Viewer { state, params, now });
    let cpu_ms = cpu_start.elapsed().as_secs_f32() * 1000.0;
    // After it: the rails the folds left behind, which only this frame's
    // rectangles can place.
    fold::paint(ui, &dock, &dock_style);
    state.dock = dock;
    // Deferred from the Panel pane's button: replacing the dock BEFORE the
    // write-back above would be silently undone.
    if std::mem::take(&mut state.reset_layout) {
        state.dock = default_dock();
        state.folds.clear();
    }

    // Render continuously only while something is animating (sounding or
    // decaying voices); otherwise poll so newly arriving MIDI still shows
    // up promptly. egui repaints on input events by itself, so interaction
    // never waits on this. The plugin shell additionally requests a
    // repaint the moment it drains new note events.
    //
    // The piano roll keeps animating well past the last release fade — it
    // scrolls for as long as its window still reaches a played note — so
    // it gets its own say here. Without this the roll would advance in
    // 50 ms jerks once the voices died.
    //
    // Flowing audio counts too: the spectrum and spectrogram advance every
    // frame off the analyzer, so with audio playing but no MIDI they'd
    // otherwise crawl at the 50 ms idle poll.
    let animating = state.tracker.voices().next().is_some()
        || state.learn_active
        || roll_scrolling(state, now)
        || state.spectrum.is_flowing(now);
    if animating {
        // Uncapped means "as fast as the shell offers"; a cap turns that into
        // a minimum spacing between repaints. Only the request changes — the
        // frame that does get drawn is identical either way.
        match frame_interval(state.fps_cap) {
            Some(interval) => ui.ctx().request_repaint_after(interval),
            None => ui.ctx().request_repaint(),
        }
    } else {
        ui.ctx().request_repaint_after(IDLE_REPAINT_INTERVAL);
    }

    // Performance overlay: fold this frame's numbers in and, if it's on, draw
    // the corner HUD. Interactive path only — the offline renderer never
    // reaches root_ui, so nothing here touches a recorded frame.
    state.perf.record(
        perf::FrameCosts {
            shell_ms: state.shell_ms,
            cpu_ms,
            tess_ms: state.tess_ms,
            egui_gpu_ms: state.egui_gpu_ms,
            lattice_gpu_ms: f32::from_bits(
                state.lattice_stats.gpu_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            prepare_ms: f32::from_bits(
                state.lattice_stats.prepare_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            poll_ms: f32::from_bits(
                state.lattice_stats.poll_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            write_ms: f32::from_bits(
                state.lattice_stats.write_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            scene_ms: f32::from_bits(
                state.lattice_stats.scene_ms.load(std::sync::atomic::Ordering::Relaxed),
            ),
            acquire_ms: state.acquire_ms,
            tick_ms: state.tick_ms,
            render_ms: state.render_ms,
            upload_ms: state.upload_ms,
            ubuf_ms: state.ubuf_ms,
            texture_ms: state.texture_ms,
            prims: state.prims,
            verts: state.verts,
            roll_notes: state.roll_notes.load(std::sync::atomic::Ordering::Relaxed),
            spectrogram_fallbacks: state.spectrum.spectrogram_fallbacks(),
            encode_ms: state.encode_ms,
            submit_ms: state.submit_ms,
        },
        now,
        perf::Workload {
            active_voices: state.tracker.voices().count(),
            held_voices: state.tracker.held_count(),
            visible_nodes: state.view.visible_count(),
            render_scale: state.view.render_scale,
            animating,
        },
    );
    if state.view.show_perf {
        perf::draw_overlay(
            ui.ctx(),
            perf_overlay_area(state, ui.max_rect()),
            &state.perf,
            state.view.show_perf_detail,
        );
    }
}

/// Where the performance overlay hangs its top-right corner: the Spectral
/// pane's body, so the HUD sits over the spectrogram rather than over the
/// lattice, which is the picture being watched.
///
/// Falls back to `editor` (the whole window) whenever that pane is not on
/// screen — another tab selected in its leaf, or the leaf collapsed — so the
/// overlay never strands itself on a pane nobody can see.
fn perf_overlay_area(state: &SharedState, editor: egui::Rect) -> egui::Rect {
    let Some(path) = state.dock.find_tab(&panes::Tab::Spectral) else {
        return editor;
    };
    let egui_dock::Node::Leaf(leaf) = &state.dock[path.surface][path.node] else {
        return editor;
    };
    // `viewport` is the tab BODY; the picture panes drop their margin, so it
    // is exactly the drawn surface. `Rect::NOTHING` until the dock has laid
    // out once (a first frame, or a freshly loaded layout).
    if leaf.collapsed || leaf.active != path.tab || !leaf.viewport.is_positive() {
        return editor;
    }
    leaf.viewport
}

/// Everything that must happen once per frame before any pane draws:
/// refresh the per-frame mirrors of the parameters and age out voices
/// whose fade has completed.
///
/// [`root_ui`] calls this itself. It is public for shells that compose
/// their own layout instead of using the dock — the offline renderer
/// draws [`Pane`]s directly, and skipping this would leave it rendering
/// last frame's tuning against never-pruned voices.
pub fn begin_frame(state: &mut SharedState, params: &dyn ParamBackend, now: f64) {
    learn_step(state, params);

    state.tuning = params::tuning_from_params(params);
    // Meantone mode locks the major third to four perfect fifths: derive it
    // from the fifth here, so the whole pipeline (scene pitch classes,
    // matching, readouts) sees the locked value without any meantone
    // awareness of its own. The lock is exact in integer microcents, so
    // comma-equivalent nodes collapse to one pitch. The Five param is left
    // untouched (inert while the lock is on).
    if state.view.meantone {
        state.tuning.lock_meantone();
    }
    state.frame_params = FrameParams {
        fade_time: params.get(params::ParamKey::Fade),
        darkest_pitch: params.get(params::ParamKey::DarkestPitch),
        brightest_pitch: params.get(params::ParamKey::BrightestPitch),
    };
    // Every layer of a node now fades on this one time, so a voice is dead
    // to the display exactly when its envelope reaches zero.
    state.tracker.prune(now, state.frame_params.fade_time);
}

/// A pane that stands on its own, outside the dock.
///
/// Only the two *views* are here. The settings panes edit state that a
/// non-interactive renderer cannot change and a viewer should not see, so
/// they are deliberately unreachable this way.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Pane {
    /// The 3D lattice.
    Lattice,
    /// The spectrum, voice bars and piano roll.
    Spectral,
}

/// Draw one pane's body into `ui`, filling it, with no dock or tab bar.
///
/// Callers must have run [`begin_frame`] for this `now` already. Panes
/// still read hover and pointer state from `ui`, so an offline caller
/// feeding synthetic input simply gets no hover — which is what a
/// recording wants.
pub fn draw_pane(ui: &mut egui::Ui, pane: Pane, state: &mut SharedState, now: f64) {
    match pane {
        Pane::Lattice => panes::lattice::lattice_pane(ui, state, now),
        // One spectrogram per frame offline, so texture slot 0. Text sizes
        // itself off the pane, here as everywhere.
        Pane::Spectral => panes::spectral::spectral_pane(ui, state, now, 0),
    }
}

/// Repaint cadence while nothing animates: newly arriving MIDI shows up
/// within one poll even without an input event.
const IDLE_REPAINT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

/// The minimum spacing between repaints implied by a frame-rate cap, or
/// `None` when uncapped.
///
/// A cap that isn't a positive, finite rate is treated as uncapped rather
/// than turned into a zero or absurd interval. The control cannot produce
/// one, but a hand-edited persisted blob can, and "no cap" is the safe
/// reading of a nonsense value — a zero interval would merely be the
/// uncapped behaviour with extra steps, while a huge one would freeze the UI.
fn frame_interval(fps_cap: Option<f32>) -> Option<std::time::Duration> {
    match fps_cap {
        Some(fps) if fps.is_finite() && fps > 0.0 => {
            Some(std::time::Duration::from_secs_f32(1.0 / fps))
        }
        _ => None,
    }
}

/// Whether the piano roll still has something moving across it: its window
/// reaches back to a note that was sounding. Goes quiet once the last note
/// has scrolled off the far edge, so an idle plugin still idles.
fn roll_scrolling(state: &SharedState, now: f64) -> bool {
    let cfg = &state.spectrum_config;
    cfg.show_roll
        && cfg.roll_fraction > 0.0
        && state
            .tracker
            .roll()
            .latest_activity(now)
            .is_some_and(|last| now - last <= cfg.roll_seconds as f64)
}

/// One tick of learn mode (v1 semantics): while armed, whenever the set of
/// held pitch classes changes, re-infer the tuning and write it through the
/// param backend. Change-detected so the host only sees parameter sets when
/// something actually changed. No egui types — testable with a stub
/// backend.
fn learn_step(state: &mut SharedState, params: &dyn ParamBackend) {
    if !state.learn_active {
        state.last_learned_classes = None;
        return;
    }
    let mut classes: Vec<PitchClass> = state
        .tracker
        .voices()
        .filter(|v| v.state == harmonigraph_core::VoiceState::Held)
        .map(|v| v.pitch_class)
        .collect();
    classes.sort_unstable();
    classes.dedup();
    if state.last_learned_classes.as_ref() == Some(&classes) {
        return;
    }
    if !classes.is_empty() {
        let learned = harmonigraph_core::learn_tuning(&classes);
        for (value, key) in [
            (learned.c_offset, params::ParamKey::COffset),
            (learned.three, params::ParamKey::Three),
            (learned.five, params::ParamKey::Five),
            (learned.seven, params::ParamKey::Seven),
        ] {
            if let Some(value) = value {
                params.set(key, value);
            }
        }
        // Auto-engage (or release) meantone mode from what was learned:
        // when the chord pins down both a fifth and a third, turn meantone
        // on iff they sit in the meantone relationship. Chords that fix
        // only one of the two leave the mode as the user left it.
        if let (Some(three), Some(five)) = (learned.three, learned.five) {
            state.view.meantone = harmonigraph_core::tuning::is_meantone(three, five);
        }
        state
            .console
            .log(format!("learn: {} held classes -> {:?}", classes.len(), learned));
    }
    state.last_learned_classes = Some(classes);
}

#[cfg(test)]
mod tests;
