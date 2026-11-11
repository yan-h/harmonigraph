//! The root shell itself: the tab registry, and the repaint pacing
//! [`root_ui`] asks for.

use super::harness::*;
use crate::*;

#[test]
fn repainting_keeps_a_detached_extension_smooth_past_the_history_window() {
    use harmonigraph_core::{NoteEvent, SourceId};

    let mut state = fresh();
    state.picture.appearance.spectrum.show_roll = true;
    state.picture.appearance.spectrum.roll_fraction = 0.5;
    state.picture.appearance.spectrum.roll_seconds = 1.0;
    state.picture.appearance.spectrum.roll_lead_release = 2.0;
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    state.picture.runtime.tracker.handle_event(NoteEvent::off(0.1, SourceId::DIRECT, 0, 60));
    let ctx = super::probe::themed();

    // A roll that is not drawn holds nothing, though its note is in its window.
    draw_panes(&ctx, &mut state, 1.5, &[]);
    assert!(!Animating::of(&state).roll, "an undrawn roll held the loop awake");
    draw_panes(&ctx, &mut state, 1.6, &[Pane::Spectral]);
    assert!(Animating::of(&state).roll, "the extension past the window idled");
    draw_panes(&ctx, &mut state, 2.2, &[Pane::Spectral]);
    assert!(!Animating::of(&state).roll, "the roll held the loop past its extension");
}

/// Run one frame of `panes` at `t`, each filling a pass of its own, as a shell
/// draws them: what each picture says about when it stops moving is written by
/// the draw, so a reason is only as fresh as the last frame drawn.
///
/// The host's Fade is the one the state already runs, so a test's Fade holds.
fn draw_panes(ctx: &egui::Context, state: &mut SharedState, t: f64, panes: &[Pane]) {
    struct Host(f32);
    impl ParamBackend for Host {
        fn get(&self, key: params::ParamKey) -> f32 {
            if key == params::ParamKey::Fade {
                self.0
            } else {
                0.0
            }
        }
        fn set(&self, _: params::ParamKey, _: f32) {}
    }
    let host = Host(state.picture.runtime.frame_params.fade_time);
    begin_frame(&mut state.picture, &host, t);
    for &pane in panes {
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 200.0),
                )),
                time: Some(t),
                ..Default::default()
            },
            |ui| crate::draw_pane(ui, pane, &mut state.picture, t, 0),
        );
    }
}

/// A background glow's release outlasts the note's own departure, and with the
/// roll hidden and no audio nothing else keeps the frame loop awake through it:
/// the halo's tail would repaint at the idle poll. The loop stays awake while
/// the glow's fade state says its light still moves a pixel (level times the
/// glow gain over half a code value), and settles after.
///
/// Driven through the real lattice pass, with the stepped levels read back from
/// the nodes it drew as the independent measure of when the fade is invisible.
#[test]
fn a_background_glows_release_tail_keeps_repainting_with_the_roll_hidden() {
    use harmonigraph_core::{NoteEvent, SourceId};

    // At a Fade of nothing, so the light arrives at once and the tail is the
    // glow's own release.
    let fresh = || {
        let mut state = fresh();
        state.picture.runtime.frame_params.fade_time = 0.0;
        state
    };
    let mut state = fresh();
    state.picture.appearance.spectrum.show_roll = false;
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    let view = state.picture.appearance.view.clone();
    assert!(view.glow_draws(), "the fresh glow draws");
    let gain = view.glow_strength;
    let span = view.note_animation.departure(state.picture.runtime.frame_params.fade_time);
    assert!(span < 2.0, "the note's own departure ({span} s) must be over 3 s after the off");

    let ctx = super::probe::themed();
    // The brightest light on the last drawn frame, after the filter stepped it.
    let draw = |state: &mut SharedState, t: f64| {
        draw_panes(&ctx, state, t, &[Pane::Lattice]);
        state.picture.surfaces.spare_nodes.iter().map(|n| n.glow.level).fold(0.0_f32, f32::max)
    };

    let (mut awake, mut idle) = (0, 0);
    let mut first_idle = None;
    let mut released = false;
    let mut t = 0.0;
    while t < 60.0 {
        // Fed as a shell feeds it, when the clock gets there: an off dated
        // ahead of the frame is read as no note at all.
        if t >= 1.0 && !released {
            released = true;
            state.picture.runtime.tracker.handle_event(NoteEvent::off(t, SourceId::DIRECT, 0, 60));
        }
        let level = draw(&mut state, t);
        if t < 0.9 {
            // Fixture reach: the node is in the window and lit.
            if t > 0.8 {
                assert!(level > 0.5, "the held note's light never arrived ({level})");
            }
        } else if t >= 3.0 {
            assert!(!Animating::of(&state).roll, "the roll must not be what keeps this awake");
            let visible = level * gain > 0.5 / 255.0;
            assert_eq!(
                Animating::of(&state).releasing,
                visible,
                "at {t} s the light is {level} at gain {gain}"
            );
            if visible {
                awake += 1;
            } else {
                idle += 1;
                first_idle.get_or_insert(t);
            }
        }
        t += if t < 3.0 { 0.05 } else { 0.25 };
    }
    // The tail is a long one and it does end: a loop that never idled would
    // pass the equality above only by the light never falling.
    assert!(awake > 20, "the tail never outlasted the note ({awake} frames)");
    assert!(idle > 20, "the loop never went idle ({idle} frames)");
    assert!(first_idle.is_some_and(|t| t < 40.0), "went idle at {first_idle:?}");

    // A surface that stops being drawn mid-tail holds nothing from the next
    // frame.
    let mut abandoned = fresh();
    abandoned.picture.appearance.spectrum.show_roll = false;
    abandoned.picture.runtime.tracker.handle_event(NoteEvent::on(
        0.0,
        SourceId::DIRECT,
        0,
        60,
        1.0,
    ));
    for k in 0..=80 {
        let t = f64::from(k) * 0.05;
        if k == 20 {
            abandoned.picture.runtime.tracker.handle_event(NoteEvent::off(
                t,
                SourceId::DIRECT,
                0,
                60,
            ));
        }
        draw(&mut abandoned, t);
    }
    assert!(Animating::of(&abandoned).releasing, "the tail was over before the surface hid");
    draw_panes(&ctx, &mut abandoned, 4.05, &[]);
    assert!(!Animating::of(&abandoned).releasing, "an undrawn surface held the loop awake");

    state.picture.appearance.view.glow_strength = 0.0;
    draw(&mut state, 60.0);
    assert!(!Animating::of(&state).releasing, "a glow that does not draw held the loop awake");
}

/// A released note keeps the loop awake for its whole departure, the
/// fade-out delay and then the stagger's sweep, and not past it. The span is
/// written out here as numbers, so a departure that loses either term is
/// caught rather than agreed with; the lattice answers from the motion it ran,
/// so it is drawn.
#[test]
fn a_released_note_keeps_repainting_through_its_staggered_departure() {
    use harmonigraph_core::{NoteEvent, SourceId};

    let mut state = fresh();
    state.picture.appearance.spectrum.show_roll = false;
    state.picture.appearance.view.glow_strength = 0.0;
    let animation = &mut state.picture.appearance.view.note_animation;
    animation.stagger_spread = 0.5;
    animation.fade_out_delay = 0.4;
    state.picture.runtime.frame_params.fade_time = 0.5;
    let ctx = super::probe::themed();
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    let mut t = 0.0_f64;
    while t < 1.0 + 1.0 {
        if (t - 1.0).abs() < 1e-9 {
            state.picture.runtime.tracker.handle_event(NoteEvent::off(
                1.0,
                SourceId::DIRECT,
                0,
                60,
            ));
        }
        draw_panes(&ctx, &mut state, t, &[Pane::Lattice]);
        t = ((t + 0.01) * 100.0).round() / 100.0;
    }
    // 0.4 s of hold, then 0.5 s of fade for the first slice and the last
    // starting half a fade later: 1.15 s past the off.
    draw_panes(&ctx, &mut state, 1.0 + 1.1, &[Pane::Lattice]);
    assert!(state.picture.runtime.tracker.voices().next().is_none(), "the voice outlasted 1 s");
    assert!(Animating::of(&state).releasing, "the departure was cut short");
    draw_panes(&ctx, &mut state, 1.0 + 1.2, &[Pane::Lattice]);
    assert!(!Animating::of(&state).releasing, "the loop stayed awake past the departure");
}

/// Feeds `seconds` of audio from `t` in 10 ms blocks, a full-scale A4 while
/// `tone` and digital silence otherwise, as a host streams both, and answers
/// whether the editor animated after each block.
///
/// Each block draws `panes`, since every picture answers from what it drew.
fn stream(
    ctx: &egui::Context,
    state: &mut SharedState,
    t: &mut f64,
    seconds: f64,
    tone: bool,
    panes: &[Pane],
) -> Vec<(f64, bool)> {
    const RATE: f32 = 48_000.0;
    let cfg = state.picture.appearance.spectrum;
    let mut animated = Vec::new();
    for _ in 0..(seconds * 100.0).round() as usize {
        let start = (*t * f64::from(RATE)).round() as usize;
        let block: Vec<f32> =
            (start..start + 480)
                .map(|i| {
                    if tone {
                        (std::f32::consts::TAU * 440.0 * i as f32 / RATE).sin()
                    } else {
                        0.0
                    }
                })
                .collect();
        *t += 0.01;
        state.picture.runtime.spectrum.push_samples(&block, 1, RATE, *t, &cfg);
        draw_panes(ctx, state, *t, panes);
        animated.push((*t, Animating::of(state).audible));
    }
    animated
}

/// A host streams silence with its transport stopped, and drawing it at full
/// rate cost Bitwig's main thread about 30% for a still picture (#1670 Q2).
/// Flowing silence animates nothing; a sound does, for as long as its last
/// audible column can still be on the heatmap — the Span, then the slab it
/// was folded into, the blur's reach and the colour memory's fade — and
/// silence past that idles however long it flows. A tilt that lifts silence
/// off the bed's colour puts it back, since the history's edges then show.
///
/// The reach is written out as the guard summed it before the heatmap
/// answered for itself (#1673), against the slab the draw chose, so the
/// heatmap's own answer is held to it at every block.
#[test]
fn flowing_silence_idles_once_the_last_sound_has_left_the_span() {
    let mut state = fresh();
    let ctx = super::probe::themed();
    let span = 20.0;
    state.picture.appearance.spectrum.roll_seconds = span as f32;
    assert!(state.picture.appearance.spectrum.show_spectrogram, "the heatmap is drawn");
    let atmosphere = &mut state.picture.appearance.spectrum.atmosphere;
    atmosphere.time_softness = 500.0;
    atmosphere.color_release = 0.6;
    assert!(atmosphere.color_release > atmosphere.color_pickup, "release outlasts pickup");
    assert!(atmosphere.effects().cloud, "the cloud and its colour memory are drawn");
    let memory = 6.234 * f64::from(atmosphere.color_pickup.max(atmosphere.color_release));
    let mut t = 0.0;
    let panes = [Pane::Spectral];

    let before = stream(&ctx, &mut state, &mut t, 2.0, false, &panes);
    assert!(before.iter().all(|&(_, on)| !on), "silence with nothing heard animated");
    let playing = stream(&ctx, &mut state, &mut t, 1.0, true, &panes);
    assert!(playing[20..].iter().all(|&(_, on)| on), "audible audio idled");
    let slab = state.picture.surfaces.spectrogram.at(0).held_bucket.expect("the heatmap drew");
    assert!(slab >= 0.1, "a slab of {slab} s is too narrow to tell from the blocks");
    let reach = span + slab + 1.5 + memory;
    let stopped = t;
    let after = stream(&ctx, &mut state, &mut t, reach + 3.0, false, &panes);

    // The curve's release outlasts the tone by seconds; the columns do not.
    let heard = state.picture.runtime.spectrum.last_audible();
    let (heard, curve) = (heard.column.unwrap(), heard.curve.unwrap());
    assert!(heard < curve && curve < stopped + 6.0, "column {heard}, curve {curve}, {stopped}");
    for (now, on) in after {
        assert_eq!(on, now - heard <= reach, "at {now}: last heard at {heard}");
    }
    assert!(!Animating::of(&state).any(), "something else held a still picture awake");

    let cfg = &mut state.picture.appearance.spectrum;
    (cfg.tilt, cfg.volume_floor_db, cfg.high_midi) = (-6.0, -100.0, 135.0);
    draw_panes(&ctx, &mut state, t, &panes);
    assert!(Animating::of(&state).audible, "silence drawn off the bed's colour idled");

    // A heatmap switched off holds nothing, whatever it last said.
    state.picture.appearance.spectrum.show_spectrogram = false;
    draw_panes(&ctx, &mut state, t, &panes);
    assert!(!Animating::of(&state).audible, "a hidden heatmap held the loop awake");
}

/// A heatmap whose pane stops being drawn — folded, hidden, the Video tab
/// closed — moves nothing, so it holds nothing, though its last draw said the
/// sound would scroll for a Span yet; drawn again mid-tail it holds again.
#[test]
fn a_heatmap_that_stops_being_drawn_holds_nothing_until_it_is_drawn_again() {
    let mut state = fresh();
    let ctx = super::probe::themed();
    state.picture.appearance.spectrum.roll_seconds = 60.0;
    let panes = [Pane::Spectral];
    let mut t = 0.0;
    stream(&ctx, &mut state, &mut t, 1.0, true, &panes);
    let stopped = t;
    let drawn = stream(&ctx, &mut state, &mut t, 0.5, false, &panes);
    assert!(drawn.iter().all(|&(_, on)| on), "the heatmap never held the sound");

    let hidden = stream(&ctx, &mut state, &mut t, 15.0, false, &[]);
    assert!(hidden.iter().all(|&(_, on)| !on), "a hidden heatmap held the loop awake");

    // Long past the curve, well inside the Span: only the heatmap moves.
    let curve = state.picture.runtime.spectrum.moving_until().expect("a tone was heard");
    assert!(curve < t && t < stopped + 30.0, "the curve holds until {curve}, now {t}");
    let revealed = stream(&ctx, &mut state, &mut t, 0.1, false, &panes);
    assert!(revealed.iter().all(|&(_, on)| on), "a heatmap drawn again mid-tail idled");
}

/// With the heatmap hidden nothing on screen holds a sound for a Span: the
/// curve and the lattice ring settle seconds after the tone, and the editor
/// idles then rather than a Span later. The ring answers from its own levels
/// and its wedges' fade, so the lattice is drawn, and once the editor has gone
/// still neither the lattice it draws nor the ring's bytes move.
#[test]
fn with_the_heatmap_hidden_the_rings_release_alone_keeps_it_animating() {
    let mut state = fresh();
    let ctx = super::probe::themed();
    state.picture.appearance.spectrum.roll_seconds = 60.0;
    state.picture.appearance.spectrum.show_spectrogram = false;
    state.picture.appearance.view.spectral_ring_width = 0.1;
    // Slow enough that the ring's levels settle last, after the curve and the
    // wedges' fade, so the bytes compared below are what the loop waits on.
    state.picture.appearance.view.spectral_ring_release = 0.6;
    assert!(state.picture.appearance.view.spectral_ring_draws(), "the ring is drawn");
    let panes = [Pane::Lattice, Pane::Spectral];
    let mut t = 0.0;
    stream(&ctx, &mut state, &mut t, 1.0, true, &panes);
    let mut ring_outlasted_the_display = false;
    let mut wedges_faded = false;
    let mut still: Option<(f64, (String, Vec<u8>))> = None;
    for _ in 0..200 {
        stream(&ctx, &mut state, &mut t, 0.1, false, &panes);
        let runtime = &state.picture.runtime;
        ring_outlasted_the_display |= !runtime.spectrum.moving(t)
            && (runtime.ring_levels.moving() || runtime.ring_fade.moving());
        wedges_faded |= runtime.ring_fade.moving();
        let nodes =
            (format!("{:?}", state.picture.surfaces.spare_nodes), runtime.ring_levels.bytes());
        match &still {
            Some((at, drawn)) => {
                assert_eq!(&nodes, drawn, "the lattice or its ring moved after {at}")
            }
            None if !Animating::of(&state).any() => still = Some((t, nodes)),
            None => {}
        }
        if still.is_some() {
            assert!(!Animating::of(&state).any(), "woke at {t} with nothing heard");
        }
    }
    assert!(ring_outlasted_the_display, "the ring never held the loop past the curve");
    assert!(wedges_faded, "the ring's gate never opened, so its fade was never measured");
    let heard = state.picture.runtime.spectrum.last_audible().curve.unwrap();
    let (idle, _) = still.expect("the ring's release outlasted 20 s");
    assert!(idle - heard < 5.0, "idled {} s after the curve last held anything", idle - heard);
}

/// A Clear takes the heard columns off the heatmap, and with them the Span they
/// would have scrolled through: the editor idles once the curve has settled.
#[test]
fn a_cleared_heatmap_holds_no_sound_for_a_span() {
    let mut state = fresh();
    let ctx = super::probe::themed();
    state.picture.appearance.spectrum.roll_seconds = 60.0;
    let panes = [Pane::Spectral];
    let mut t = 0.0;
    stream(&ctx, &mut state, &mut t, 1.0, true, &panes);
    // Once the analysis window has let go of the tone, or the columns after
    // the Clear still hold it and scroll it across as they should.
    stream(&ctx, &mut state, &mut t, 1.0, false, &panes);
    state.picture.runtime.spectrum.clear_history();
    let after = stream(&ctx, &mut state, &mut t, 10.0, false, &panes);
    let heard = state.picture.runtime.spectrum.last_audible().curve.unwrap();
    let last = after.iter().rev().find(|&&(_, on)| on).expect("the curve settled at once").0;
    assert!(last - heard < 1.0, "a Clear held the full rate {} s past the sound", last - heard);
}

/// A note arriving in an idle editor paints on the next tick, not at the idle
/// poll: it makes a voice, and a voice is what `root_ui` paces by. The input
/// worker's drain no longer reports an arrival, so this is what tells an idle
/// editor one came.
///
/// The roll and the release fade also keep a fresh note animating, so the voice
/// is isolated by clearing the roll before the frame (Clear display history on
/// a held note), leaving the voice as the only thing in motion.
#[test]
fn a_voice_in_an_idle_editor_asks_for_the_next_tick() {
    use harmonigraph_core::{NoteEvent, SourceId};

    let delay = |output: &egui::FullOutput| {
        output.viewport_output.values().map(|viewport| viewport.repaint_delay).min().unwrap()
    };
    let mut state = fresh();
    let mut harness = DockHarness::new();
    // Settled: the opening frames lay the dock out and ask to be redrawn.
    let mut idle = harness.frame(&mut state, vec![]);
    for _ in 0..4 {
        idle = harness.frame(&mut state, vec![]);
    }
    // The whole poll: egui takes its predicted frame time off a delay, which
    // left 33 ms and drew every fifth refresh at 144 Hz (#1673).
    assert_eq!(delay(&idle), crate::IDLE_REPAINT_INTERVAL, "an idle editor waits one poll");

    let at = harness.next_time();
    state.picture.runtime.tracker.handle_event(NoteEvent::on(at, SourceId::DIRECT, 0, 60, 1.0));
    state.picture.runtime.tracker.clear_roll();
    assert!(state.picture.runtime.tracker.voices().next().is_some(), "the note still sounds");
    let sounding = harness.frame(&mut state, vec![]);
    assert_eq!(delay(&sounding), std::time::Duration::ZERO, "a voice repaints at once");
}

/// Tab with a modifier goes to the host, so it must not also run egui's focus
/// traversal: Shift-Tab would focus the last widget, and from then on the
/// editor claims every key, so Space presses that widget instead of reaching
/// the DAW.
#[test]
fn a_tab_with_a_modifier_moves_no_focus_and_keeps_the_chrome() {
    let tab = |modifiers| egui::Event::Key {
        key: egui::Key::Tab,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    let mut state = fresh();
    let mut harness = DockHarness::new();
    harness.frame(&mut state, vec![]);
    harness.frame(&mut state, vec![tab(egui::Modifiers::SHIFT)]);
    harness.frame(&mut state, vec![]);
    assert_eq!(harness.ctx.memory(|m| m.focused()), None, "Shift-Tab focused a widget");
    assert!(!state.workspace.interaction.prefs.frameless, "Shift-Tab is not the chrome toggle");
    harness.frame(&mut state, vec![tab(egui::Modifiers::NONE)]);
    harness.frame(&mut state, vec![]);
    assert_eq!(harness.ctx.memory(|m| m.focused()), None, "Tab focused a widget");
    assert!(state.workspace.interaction.prefs.frameless, "bare Tab hides the chrome");
}

/// Every tab needs an id of its own, and the title is not allowed to be its
/// source: the workspace keys each tab BODY's `Ui` on `Viewer::id`
/// (`workspace.rs`), so two tabs whose ids came from a shared title would
/// share their body state — scrolling one pane scrolls the other.
/// Variant-keyed ids are what leave a name free to be repeated across
/// sections, as a display and the settings tab holding its knobs may be.
#[test]
fn every_tab_has_its_own_id_whatever_its_title_says() {
    let mut state = fresh();
    let params = RecordingBackend::default();
    let tabs = [
        panes::Tab::Lattice,
        panes::Tab::Tuning,
        panes::Tab::LatticeSettings,
        panes::Tab::AnalyzerSettings,
        panes::Tab::Mappings,
        panes::Tab::System,
        panes::Tab::Console,
        panes::Tab::Spectral,
        panes::Tab::Spiral,
        panes::Tab::Video,
    ];
    let viewer = panes::Viewer {
        state: &mut state.picture,
        interaction: &mut state.workspace.interaction,
        params: &params,
        now: 0.0,
    };

    let ids: Vec<egui::Id> = tabs.iter().map(|&tab| viewer.id(&tab)).collect();
    for (i, a) in ids.iter().enumerate() {
        for (j, b) in ids.iter().enumerate().skip(i + 1) {
            assert_ne!(a, b, "{:?} and {:?} share a tab id", tabs[i], tabs[j]);
        }
    }
}

/// The picture panes fill their body exactly, so a scroll area around one can
/// only shift a picture that is meant to sit still.
#[test]
fn the_picture_panes_do_not_scroll() {
    let mut state = fresh();
    let params = RecordingBackend::default();
    let viewer = panes::Viewer {
        state: &mut state.picture,
        interaction: &mut state.workspace.interaction,
        params: &params,
        now: 0.0,
    };
    for tab in [panes::Tab::Lattice, panes::Tab::Spectral, panes::Tab::Spiral] {
        assert_eq!(viewer.scroll_bars(&tab), [false, false], "{tab:?} is scrollable");
    }
    // Settings panes are lists and must stay reachable in a short column, but
    // VERTICALLY only: a both-axes area gives the body unbounded width, and the
    // panes that fill the space then never report vertical overflow, so the
    // wheel can't scroll them. Horizontal off; vertical on.
    for &tab in workspace::Section::Settings.tabs() {
        assert_eq!(viewer.scroll_bars(&tab), [false, true], "{tab:?} cannot scroll vertically");
    }
}

/// A separator reads against BOTH grounds a pane can paint, because the two
/// sides of one boundary are not one surface. A picture pane fills its body with
/// the well edge to edge — `Viewer::tab_style_override` takes its margin to zero,
/// which is the arrangement the test above is the other half of — while a
/// settings pane shows the tab body's panel through. So a divider borrowed from
/// either grey vanishes along whichever boundaries happen to be drawn in it, and
/// the lattice against the analyzer, two picture panes in one grey, has no edge
/// between them at all.
///
/// A ratio and a modest one, because this palette is deep and its span is
/// narrow: the lightest grey the chrome owns is 1.93 against the well, so WCAG's
/// 3:1 for non-text is out of reach for any colour that still looks like this
/// instrument. 1.25 is the line between a divider and a coincidence — the panel
/// against the well is 1.08 and fails it, which is the case worth catching, a
/// separator dialled to a grey borrowed from the chrome around it.
#[test]
fn a_separator_reads_against_both_grounds_a_pane_can_paint() {
    /// The WCAG relative-luminance ratio between two opaque colours.
    fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
        fn luminance(c: egui::Color32) -> f32 {
            let channel = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(c.r()) + 0.7152 * channel(c.g()) + 0.0722 * channel(c.b())
        }
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    const FLOOR: f32 = 1.25;

    // Every state a separator is drawn in, not the idle one alone: a hover or a
    // drag that stopped reading would be the same bug on the frame it matters.
    for (state, color) in [
        ("idle", theme::hairline()),
        ("hovered", theme::accent_edge()),
        ("dragged", theme::accent()),
    ] {
        for (ground, fill) in [
            ("the picture panes' ground", theme::picture()),
            ("a settings pane's panel", theme::panel()),
        ] {
            let ratio = contrast(color, fill);
            assert!(
                ratio >= FLOOR,
                "a {state} separator is {ratio:.2} against {ground}, under {FLOOR} — \
                 that boundary is invisible",
            );
        }
    }
}

/// Narrow headers keep one row and put all destinations in the current-tab menu.
#[test]
fn every_settings_tab_fits_on_its_tab_bar() {
    for width in [700.0, 1000.0, 1512.0] {
        let mut state = fresh();
        let mut harness = DockHarness::at(egui::vec2(width, 700.0));
        harness.settle(&mut state);
        let output = harness.frame(&mut state, vec![]);
        let leaf = state.workspace.layout_runtime.rects[workspace::Section::Settings as usize];
        let body = state.workspace.layout_runtime.body(panes::Tab::Tuning).unwrap();
        let rail = theme::tab_bar_height(1.0);
        assert!((body.top() - leaf.top() - rail).abs() < 0.5);
        let title = output
            .shapes
            .iter()
            .find_map(|cs| match &cs.shape {
                egui::Shape::Text(t)
                    if t.galley.text() == "Tuning"
                        && leaf.contains(t.pos)
                        && t.pos.y < body.top() =>
                {
                    Some((egui::Rect::from_min_size(t.pos, t.galley.size()), cs.clip_rect))
                }
                _ => None,
            })
            .expect("current tab title");
        assert!(title.1.contains_rect(title.0));
    }
}

/// A line that stops in open space ends round: a disc half its width over each
/// end, in the stroke's own color, laid where egui actually draws the line — a
/// level line off the pixel grid is snapped onto it first, caps and all. A
/// translucent line, or one in a faded (disabled) painter, keeps egui's square
/// ends, which a disc would paint over twice.
#[test]
fn an_opaque_line_ends_round_and_a_translucent_one_square() {
    let (a, b) = (egui::pos2(20.0, 40.3), egui::pos2(80.0, 40.3));
    let caps = |color: egui::Color32, disabled: bool| {
        let stroke = egui::Stroke::new(theme::LINE_WIDTH, color);
        let mut y = a.y;
        let out = super::probe::painted_full(egui::vec2(100.0, 80.0), |ui| {
            stroke.round_center_to_pixel(ui.pixels_per_point(), &mut y);
            if disabled {
                ui.disable();
            }
            theme::round_ended(ui.painter(), vec![a, b], stroke);
        });
        assert_ne!(y, a.y, "fixture must start the line off the pixel grid");
        let line = out.shapes.iter().any(|cs| {
            matches!(cs.shape, egui::Shape::LineSegment { points, .. }
                if points == [egui::pos2(a.x, y), egui::pos2(b.x, y)])
        });
        assert!(line, "the line is not the segment egui would have drawn at {y}");
        let meshes = out
            .shapes
            .iter()
            .filter_map(|cs| match &cs.shape {
                egui::Shape::Mesh(mesh) => Some(mesh.calc_bounds()),
                _ => None,
            })
            .collect::<Vec<_>>();
        (y, meshes)
    };
    let (y, round) = caps(theme::text(), false);
    assert_eq!(round.len(), 1, "both caps are one mesh");
    let cap = egui::Vec2::splat(theme::LINE_WIDTH);
    for x in [a.x, b.x] {
        let disc = egui::Rect::from_center_size(egui::pos2(x, y), cap);
        assert!(round[0].expand(0.01).contains_rect(disc), "no cap over ({x}, {y}): {round:?}");
    }
    assert!(
        (round[0].center().y - y).abs() < 0.01,
        "the caps sit off the line egui draws: {round:?} against {y}"
    );
    assert!(
        caps(theme::text().gamma_multiply(0.5), false).1.is_empty(),
        "a translucent line got caps"
    );
    assert!(caps(theme::text(), true).1.is_empty(), "a line in a disabled section got caps");
}
