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

/// What the UI persists of the analyzer's display settings, with the serde
/// defaults that keep older blobs loadable. The render settings persist too
/// but live in `harmonigraph-take` — see the re-export below.
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
    SpectralOrientation, SpectrogramColor, SpectrumConfig, SpectrumWindow, SCALE_BAR_RANGE,
    TILT_STEPS,
};
// The render settings live in `harmonigraph-take` because they are take
// payload: a take carries the frame it was composed at, so a re-render
// reproduces that framing rather than whatever the editor is set to now.
// `RenderProgress` travels with them for a different reason — it is what the
// recorder reports back while a render runs, and it lives beside the settings
// so `harmonigraph-record` needs no editor to hand it over.
//
// Re-exported here because every pane, shell and test already reaches them
// through this crate, and where a type is defined is not their business.
pub use harmonigraph_take::{
    LatticeSide, RenderConfig, RenderFrame, RenderProgress, RenderTrigger,
};
pub(crate) use config::{
    COLOR_RANGE_MIN_SPAN, LEVEL_MAX_DB, LEVEL_MIN_DB, LEVEL_RANGE_MIN_SPAN, PITCH_RANGE_MIN_SPAN,
    ROLL_SECONDS_MAX, ROLL_SECONDS_MIN,
};
pub use spectrum::{AudioSpectrum, SpectrogramColumn, SpectrumHistory, WholeSong};
pub(crate) use spectrum::{SpectrogramCache, SpectrogramKey};
pub use perf::ShellTimings;
pub use state::{render_config_from_persist, CameraPreset, Console, SharedState, TakeState};
pub(crate) use state::default_dock;

use harmonigraph_core::{Comma, PitchClass, Tuning};
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
/// editor is a guest inside a host window: let the host take focus mid-drag,
/// or drag over the editor from a press that was never ours, and the release
/// is delivered somewhere that is not us. egui then believes the button is
/// still down forever.
///
/// So: no pointer, or no focus, means no drag. Neither is the ordinary end of
/// a gesture, and a gesture no longer reaches either by going out of the
/// window: the plugin shell holds the pointer's exit back for as long as a
/// button is down (`mouse_exited` in the vendored baseview), so a slider
/// dragged past the window edge keeps tracking, and the exit arrives here only
/// once the button is up. What is left for this to catch is a release that
/// never comes at all, where the alternative is every settings pane's wheel
/// dead until the next click.
///
/// Not a `Sense::drag` problem in any one pane — a ValueBar strands the wheel
/// exactly as well as the Analyzer's pan does, which is why this sits once at
/// the root rather than in whichever pane the drag came from.
fn end_stranded_drag(ctx: &egui::Context) {
    if ctx.dragged_id().is_none() {
        return;
    }
    if !kept_focus(ctx) || ctx.input(|i| i.pointer.latest_pos().is_none()) {
        ctx.stop_dragging();
    }
}

/// Whether the editor still has the window whatever gesture is in flight began
/// in.
///
/// A host that takes focus mid-drag is handed the release, so what is left here
/// is a button egui believes is held forever — and anything of ours that follows
/// the pointer for as long as it is held would follow it around the screen (see
/// `fold::Grip`). Losing focus is the one unambiguous end of a gesture: a
/// pointer that has merely left the window is still dragging, and still ours.
///
/// Focus is read from the EVENT as well as the flag. `InputState::focused` comes
/// from `RawInput::focused`, which starts true and only moves if an integration
/// sets it — and the plugin's (vendored egui-baseview) reports focus by pushing
/// `WindowFocused` and filling in `ViewportInfo`, never that field. The flag
/// alone would therefore be true forever in the one shell this is most needed
/// in. Reading both means neither a shell that sets the flag nor one that only
/// sends the event is missed, and a shell that says nothing either way (the
/// offline renderer, the tests) is untouched.
fn kept_focus(ctx: &egui::Context) -> bool {
    ctx.input(|i| {
        i.focused && !i.events.iter().any(|e| matches!(e, egui::Event::WindowFocused(false)))
    })
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

    // The chrome scale, before anything lays out at the old one. The shell
    // built `ui` from the style as it stood on the way in, so a scale that has
    // just moved leaves this frame's copy a size behind — and the frame it
    // would be behind on is the one being dragged, where every intermediate
    // size shows. `reset_style` takes the rebuilt one from the context, which
    // `set_ui_scale` has already put there.
    if theme::set_ui_scale(ui.ctx(), state.ui_scale) {
        ui.reset_style();
    }
    // Read back rather than reused: `set_ui_scale` clamps, and the dock has to
    // be built at the scale that actually took.
    let ui_scale = theme::ui_scale(ui.ctx());

    // Cleared before the panes run, so a frame with the roll hidden (or the
    // Spectral pane not on screen at all) reports zero notes rather than
    // whatever the last frame that had one reported.
    state.instruments.roll_notes.store(0, std::sync::atomic::Ordering::Relaxed);

    // Frameless mode hides every tab bar (the Lattice and Spectral panes
    // meet with no chrome between them — clean for captures). The pane
    // separators keep their regular width, so the spacing between windows
    // matches framed mode. No tab bar also means no way to click back to
    // the Panel pane (which holds the checkbox) if it's hidden, so Tab
    // works from anywhere. It toggles rather than only restoring, so the
    // chrome comes and goes on one key while a take is set up — the
    // checkbox is then just where the feature is documented.
    //
    // Tab is egui's focus-walk key, and this takes it: nothing here is
    // driven from the keyboard, and the one place typing Tab means
    // something else — a text field mid-edit — keeps it. Cancelling the
    // focus move egui already queued from the same press is part of the
    // toggle, or a capture grows a focus ring around whatever control the
    // walk landed on.
    if !ui.ctx().text_edit_focused()
        && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Tab))
    {
        state.view.frameless = !state.view.frameless;
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
    }
    let mut dock_style = theme::dock_style(ui.style(), ui_scale);
    if state.view.frameless {
        dock_style.tab_bar.height = 0.0;
    }

    // DockState has to be moved out while panes borrow the rest of `state`.
    let mut dock = std::mem::replace(&mut state.dock, DockState::new(vec![]));
    // Before the dock lays out: a pane collapsed inside a horizontal split
    // folds sideways to a rail, which is a split fraction, which is layout's
    // input. egui_dock's own vertical folds need nothing from us.
    //
    // What the fold takes comes off the window rather than off the pane beside
    // it, and the window is the shell's to resize — so the points are banked
    // here and spent after this frame (see `take_window_width_change`). The
    // flags a fold or an unfold moves wait for that window before they land
    // (see `fold::Wait`), so the frame that asks draws what the frame before it
    // drew and no boundary moves until the window is there to hold it.
    //
    // What the pointer is doing, before the fold reads the fractions last frame
    // left behind: a fraction that moved with no gesture behind it is egui_dock's
    // own per-frame clamp, not a drag the layout should follow — and where the
    // pointer IS is what a drag on a separator asks for, once one has hold of a
    // boundary (see `fold::drags`).
    // A gesture the window has lost is not one to go on following: the pointer
    // leaving is a drag that is still ours, but the focus leaving is not (see
    // `kept_focus`).
    let (gesturing, at) = ui.input(|i| {
        (
            i.pointer.any_down() || i.pointer.any_released(),
            i.pointer.latest_pos().map(|at| at.x),
        )
    });
    state.dial.watch_pointer(gesturing && kept_focus(ui.ctx()), at);
    let area = fold::area_width(ui, &dock_style);
    state.window_width_change +=
        state.folds.apply(&mut dock, &dock_style, area, state.min_window_width, &mut state.dial);
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
    // rectangles can place — and the handles that pull them back open, which
    // land on `dial` for the next frame to price (see `fold::grab`).
    fold::paint(ui, &mut dock, &dock_style, &mut state.dial);
    state.dock = dock;
    // Deferred from the Panel pane's button: replacing the dock BEFORE the
    // write-back above would be silently undone.
    if std::mem::take(&mut state.reset_layout) {
        state.dock = default_dock();
        // The default layout has every pane open, so the window gets back
        // whatever the folds being thrown away were holding.
        state.window_width_change += state.folds.clear(&state.dial, area);
        // The flags describe the tree being thrown away (see [`fold::Dial::forget`]).
        state.dial.forget();
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
    state.instruments.perf.record(
        perf::FrameCosts::assemble(
            state.instruments.timings,
            cpu_ms,
            &state.instruments.lattice_stats,
            state.instruments.roll_notes.load(std::sync::atomic::Ordering::Relaxed),
            state.spectrum.spectrogram_fallbacks(),
        ),
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
            perf_overlay_area(state, ui.max_rect(), ui_scale),
            &state.instruments.perf,
            state.view.show_perf_detail,
        );
    }
}

/// Where the performance overlay hangs its top-right corner: the Spectral
/// pane's body, so the HUD sits over the spectrogram rather than over the
/// lattice, which is the picture being watched.
///
/// The Lattice's body when the analyzer is not on screen — another tab
/// selected in its leaf, or the leaf folded away — so the overlay never
/// strands itself on a pane nobody can see. A tab BODY either way, and that
/// is the point rather than a detail of which pane: dock chrome lives along
/// the top of the window, so the HUD hung on the window instead lands ON it,
/// over the settings column's tab bar and, with the column folded, over the
/// collapse arrow that brings a pane back. Covering the control that undoes
/// the fold is the worst place on screen for it, and it is exactly where
/// folding the analyzer would otherwise put it.
///
/// `editor` is the last resort, for a dock with neither picture on screen,
/// and is inset past the tab bar on the same grounds.
fn perf_overlay_area(state: &SharedState, editor: egui::Rect, scale: f32) -> egui::Rect {
    pane_body(state, &panes::Tab::Spectral)
        .or_else(|| pane_body(state, &panes::Tab::Lattice))
        .unwrap_or_else(|| {
            let mut area = editor;
            area.min.y += theme::tab_bar_height(scale);
            area
        })
}

/// A docked tab's drawn surface, or `None` when it is not on screen.
fn pane_body(state: &SharedState, tab: &panes::Tab) -> Option<egui::Rect> {
    let path = state.dock.find_tab(tab)?;
    let egui_dock::Node::Leaf(leaf) = &state.dock[path.surface][path.node] else {
        return None;
    };
    // `viewport` is the tab BODY; the picture panes drop their margin, so it
    // is exactly the drawn surface. `Rect::NOTHING` until the dock has laid
    // out once (a first frame, or a freshly loaded layout) — and STALE while
    // the leaf is collapsed, which is why the flag is checked rather than the
    // rect: a folded leaf keeps the viewport it had when it was open.
    (!leaf.collapsed && leaf.active == path.tab && leaf.viewport.is_positive())
        .then_some(leaf.viewport)
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
    // Auto-detect, then lock, one comma at a time and up the primes — the
    // order of `Comma::ALL`, and the order that makes the two identities
    // compose: the septimal one reads the third, so it has to see the third
    // the syntonic lock has already derived, not the inert param under it.
    //
    // Auto-detect: a tuning that IS one of these temperaments engages its
    // mode, whatever put it there — the 12-TET preset, a learned chord, a
    // drag of any bar. Engage only. Once a lock holds, the axis it derives
    // stops being read at all, so a released mode would have to be
    // re-detected from axes that no longer describe the lattice — dragging
    // the fifth would drop the lock it is meant to carry along. Releasing is
    // an explicit edit of the derived axis instead (see `panes::tuning`),
    // which is the one gesture that can mean nothing else.
    //
    // And each comma judges each tuning ONCE, which is what gives its switch
    // a working OFF direction: nothing about an unchanged tuning can have
    // changed the answer, so a mode switched off stays off until the tuning
    // itself moves.
    //
    // Once is also the only reading that survives the plugin's parameters.
    // A `set` there is queued for the host — nih-plug: "the parameter's
    // actual value will only be changed when the output event is written" —
    // so for a frame or more after ANY tuning write, `get` still reports the
    // value being written away FROM. Judging those stale axes afresh undoes
    // the edit that is in flight: the mode the user just switched off comes
    // back for a frame (press twice to disengage), and a Just preset re-locks
    // on the tuning it is leaving. Judging once means the stale frames say
    // nothing, and a tuning gets its verdict when it arrives.
    // Judged against the PARAMS, before any lock derives an axis from them:
    // what a verdict is about is the tuning someone set, and a mode switch is
    // not a tuning edit. Keyed on the derived values instead, releasing
    // meantone would move the third the septimal identity reads and re-open a
    // marvel verdict the user had just switched off.
    let params_tuning = state.tuning;
    for comma in Comma::ALL {
        let axes = judged_axes(comma, &params_tuning);
        if state.view.temper_auto(comma)
            && !state.view.tempers(comma)
            && state.temper_judged[comma.index()] != Some(axes)
        {
            let tuning = &state.tuning;
            *state.view.temper_mut(comma) = comma.is_tempered(
                tuning.three_cents(),
                tuning.five_cents(),
                tuning.seven_cents(),
            );
        }
        // Every frame, engaged or not: an engaged mode that is switched off
        // must find its own tuning already judged, or the frame after the
        // switch reads as a tuning nobody has looked at.
        state.temper_judged[comma.index()] = Some(axes);
        // Then the lock itself: derive the axis here, so the whole pipeline
        // (scene pitch classes, matching, readouts) sees the derived value
        // without any tempering awareness of its own. It is exact in integer
        // microcents, so comma-equivalent nodes collapse to one pitch. The
        // derived axis's param is left untouched (inert while the lock is on).
        if state.view.tempers(comma) {
            state.tuning.temper(comma);
        }
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

/// The tuning axes one comma's identity reads, in microcents — the key its
/// auto-detect judges a tuning by. Always the PARAM values, never a locked
/// axis derived from them (see `begin_frame`).
///
/// Only the axes it reads, and that is the point: a seventh that moved says
/// nothing about the syntonic comma, so re-opening that question would
/// re-engage a meantone the user had just switched off. The unread axis is
/// zeroed rather than left out so both keys are one type.
fn judged_axes(comma: Comma, tuning: &Tuning) -> (i32, i32, i32) {
    match comma {
        Comma::Syntonic => (tuning.three, tuning.five, 0),
        Comma::SeptimalKleisma => (tuning.three, tuning.five, tuning.seven),
    }
}

/// The axes one comma's identity reads, as a learned chord states them, or
/// `None` when the chord did not pin all of them down — a bare fifth settles
/// nothing about the syntonic comma, and a triad nothing about the septimal.
/// This is the only place a learn decides what a chord is evidence FOR.
///
/// The septimal identity reads the third the LATTICE will use, which is the
/// derived one whenever the syntonic lock holds — including one this same
/// chord has just engaged, an arm earlier in `Comma::ALL` order. Read the
/// played third instead and a chord that is not a marvel against the tuning
/// in force engages the mode anyway, which the engage-only detect can then
/// never release.
///
/// The syntonic comma reads no seventh at all, so it gets none: the 0 stands
/// for an axis its identity does not look at, and `is_meantone` does not.
fn learned_axes(
    comma: Comma,
    learned: &harmonigraph_core::LearnedTuning,
    view: &harmonigraph_scene::ViewConfig,
) -> Option<(f32, f32, f32)> {
    let (three, five) = (learned.three?, learned.five?);
    match comma {
        Comma::Syntonic => Some((three, five, 0.0)),
        Comma::SeptimalKleisma => {
            let five = if view.tempers(Comma::Syntonic) {
                harmonigraph_core::tuning::meantone_third(three)
            } else {
                five
            };
            Some((three, five, learned.seven?))
        }
    }
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
        // Auto-engage (or release) each comma's mode from what was learned:
        // when the chord pins down every axis that comma's identity reads,
        // turn its mode on iff they sit in that relationship. A chord that
        // fixes only some of them leaves the mode as the user left it — a
        // bare fifth says nothing about the third.
        //
        // The one place the auto-detect also RELEASES, and it can: a chord
        // fixing every axis of an identity states the whole relationship in
        // one gesture, so a learned just third is as explicit as dragging
        // one. With a detect off, learn retunes the axes and leaves that mode
        // alone — one switch governs every automatic decision about its comma.
        for comma in Comma::ALL {
            if !state.view.temper_auto(comma) {
                continue;
            }
            if let Some((three, five, seven)) = learned_axes(comma, &learned, &state.view) {
                *state.view.temper_mut(comma) = comma.is_tempered(three, five, seven);
            }
        }
        state
            .console
            .log(format!("learn: {} held classes -> {:?}", classes.len(), learned));
    }
    state.last_learned_classes = Some(classes);
}

#[cfg(test)]
mod tests;
