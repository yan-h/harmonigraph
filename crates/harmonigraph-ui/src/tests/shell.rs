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
    // The extension is the lead's: with none, nothing is drawn past the window.
    state.picture.appearance.spectrum.roll_lead = 0.3;
    state.picture.appearance.spectrum.roll_lead_release = 2.0;
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    state.picture.runtime.tracker.handle_event(NoteEvent::off(0.1, SourceId::DIRECT, 0, 60));
    let ctx = super::probe::themed();

    // A roll that is not drawn holds nothing, though its note is in its window.
    draw_panes(&ctx, &mut state, 1.5, &[]);
    assert!(
        !Animating::of(&state).asks_for(Reason::RollInSpan),
        "an undrawn roll held the loop awake"
    );
    draw_panes(&ctx, &mut state, 1.6, &[Pane::Spectral]);
    assert!(
        Animating::of(&state).asks_for(Reason::RollInSpan),
        "the extension past the window idled"
    );
    draw_panes(&ctx, &mut state, 2.2, &[Pane::Spectral]);
    assert!(
        !Animating::of(&state).asks_for(Reason::RollInSpan),
        "the roll held the loop past its extension"
    );
}

/// The roll holds the loop for a note it draws that is still inside its own
/// Span, and for nothing else (#1835): not a note off the pitch zoom, which
/// draws nothing, and not an older note's overhang sliding off the far edge
/// because a newer, undrawn note is still inside the Span. The lone on-zoom
/// note is the control.
#[test]
fn the_roll_holds_the_loop_only_for_a_drawn_note_inside_its_span() {
    let held = |notes, now| {
        let (after, drawn) = roll_frame(notes, now, 10.0, 4.0);
        (after.is_some(), drawn)
    };
    // Long after each note's own fade, so the roll is the only thing that
    // could still be moving.
    let (held_, drawn) = held(&[(72, 0.0, 0.1)], 5.0);
    assert!(drawn > 0 && held_, "a note scrolling on the zoom idled");
    let (held_, drawn) = held(&[(36, 0.0, 0.1)], 5.0);
    assert!(drawn == 0 && !held_, "a note off the zoom held a still roll awake");
    // The older note's body left the Span at 10.1 s; its outline still slides
    // off the far edge. The newer note is inside the Span, off the zoom.
    let (held_, drawn) = held(&[(72, 0.0, 0.1), (36, 5.0, 5.1)], 10.15);
    assert!(drawn > 0, "the fixture's older note must still overhang the far edge");
    assert!(!held_, "an undrawn newer note held the older note's overhang at full rate");
}

/// A ribbon that only scrolls asks for the frame where the Span's scroll next
/// crosses a device pixel, which is as much sooner as the Span is shorter or
/// the pixels finer (#1911); a Lead still fading through its release asks for every frame.
#[test]
fn a_scrolling_roll_asks_for_its_next_pixel_and_a_fading_lead_for_every_frame() {
    let note = [(72, 0.0, 0.1)];
    let (paced, drawn) = roll_frame(&note, 5.0, 10.0, 4.0);
    let paced = paced.expect("a note scrolling on the zoom idled");
    assert!(drawn > 0 && paced > 0.0, "a scroll held every frame: {paced} s");
    let (half, _) = roll_frame(&note, 5.0, 5.0, 4.0);
    let half = half.expect("a note scrolling on a shorter Span idled");
    assert!((half / paced - 0.5).abs() < 1e-6, "half the Span paced at {half} s, not half {paced}");
    // Twice the pixels a point: a pixel is crossed twice as often.
    let (finer, _) = roll_frame(&note, 5.0, 10.0, 8.0);
    let finer = finer.expect("a note scrolling on finer pixels idled");
    assert!(
        (finer / paced - 0.5).abs() < 1e-6,
        "finer pixels paced at {finer} s, not half {paced}"
    );
    // Inside the Lead's 0.25 s release.
    let (fading, _) = roll_frame(&note, 0.2, 10.0, 4.0);
    assert_eq!(fading, Some(0.0), "a fading Lead was paced as a scroll");
}

/// A held note's Lead stands still at the now-line, its ribbon only scrolling
/// behind it, so the live roll asks for the frame where the ribbon next
/// crosses a device pixel rather than for every frame: a chord held over a
/// still picture costs the main thread its scroll, not the display rate.
#[test]
fn a_held_notes_lead_asks_for_no_frame_of_its_own() {
    let (held, drawn) = roll_frame(&[(72, 0.0, f64::INFINITY)], 5.0, 10.0, 4.0);
    let held = held.expect("a held note's ribbon idled");
    assert!(drawn > 0 && held > 0.0, "a held Lead asked for every frame: {held} s");
}

/// One frame of the Spectral pane at `now`, with `notes` (key, on, off) played
/// into a roll of `span` seconds at `ppp` device pixels a point (an infinite
/// off leaves its note held): fine ones,
/// as the Retina display's, make its scroll cross a device pixel sooner than
/// the idle poll would draw it:
/// when the roll asked for its next frame, and how many instances it drew.
fn roll_frame(notes: &[(u8, f64, f64)], now: f64, span: f32, ppp: f32) -> (Option<f64>, usize) {
    use harmonigraph_core::{NoteEvent, SourceId};

    let mut state = fresh();
    let spectrum = &mut state.picture.appearance.spectrum;
    spectrum.show_roll = true;
    spectrum.note_names = false;
    spectrum.roll_fraction = 0.5;
    spectrum.roll_seconds = span;
    spectrum.roll_lead_release = 0.25;
    spectrum.low_midi = 60.0;
    spectrum.high_midi = 84.0;
    let tracker = &mut state.picture.runtime.tracker;
    for &(key, on, off) in notes {
        tracker.handle_event(NoteEvent::on(on, SourceId::DIRECT, 0, key, 1.0));
        if off.is_finite() {
            tracker.handle_event(NoteEvent::off(off, SourceId::DIRECT, 0, key));
        }
    }
    let ctx = super::probe::themed_at(ppp);
    // egui lays out the first pass after a change of scale in its default
    // screen, 10,000 device pixels square, rather than the one given.
    draw_panes(&ctx, &mut state, now, &[Pane::Spectral]);
    crate::panes::spectral::roll::take_roll_draw_probe();
    draw_panes(&ctx, &mut state, now, &[Pane::Spectral]);
    let drawn =
        crate::panes::spectral::roll::take_roll_draw_probe().map_or(0, |probe| probe.note_count);
    (Animating::of(&state).after(Reason::RollInSpan), drawn)
}

/// A paced reason asks for its own frame and the sooner of two asks wins; one
/// at or past the idle poll is a still picture, which the poll draws as soon.
/// The perf overlay judges the rate against the link's only while some reason
/// wants every frame it offers (#1911).
#[test]
fn the_soonest_ask_wins_and_one_past_the_poll_holds_nothing() {
    let mut held = Animating::default();
    assert_eq!(held.next_change(), None);
    held.hold_for(Reason::Playhead, 0.02);
    held.hold_for(Reason::Playhead, 0.03);
    held.hold_for(Reason::RollInSpan, 0.01);
    assert_eq!(held.after(Reason::Playhead), Some(0.02), "a later ask replaced a sooner one");
    assert_eq!(held.next_change(), Some(0.01));
    assert!(!held.every_frame(Some(144.0)), "a 10 ms pace was judged against 144 Hz");
    assert!(held.every_frame(Some(60.0)), "a pace under one 60 Hz frame was not judged");
    held.hold(Reason::Releasing);
    assert_eq!(held.next_change(), Some(0.0));

    let mut still = Animating::default();
    still.hold_for(Reason::Audible, crate::IDLE_REPAINT_INTERVAL.as_secs_f64());
    assert!(!still.asks(), "a pace at the idle poll held the loop");
    assert_eq!(still.to_string(), "");
    assert_eq!(held.to_string(), "roll~ releasing playhead~");
}

/// Run one frame of `panes` at `t`, each filling a pass of its own, as a shell
/// draws them: what each picture says about when it stops moving is written by
/// the draw, so a reason is only as fresh as the last frame drawn.
fn draw_panes(ctx: &egui::Context, state: &mut SharedState, t: f64, panes: &[Pane]) {
    struct Host;
    impl ParamBackend for Host {
        fn get(&self, _: params::ParamKey) -> f32 {
            0.0
        }
        fn set(&self, _: params::ParamKey, _: f32) {}
    }
    begin_frame(&mut state.picture, &Host, t);
    for &pane in panes {
        let _ = crate::tests::probe::run(
            ctx,
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 200.0),
                )),
                time: Some(t),
                ..Default::default()
            },
            |ui| crate::draw_panes(ui, &[(pane, ui.max_rect())], &mut state.picture, t),
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
        state.picture.appearance.view.fade_time = 0.0;
        state
    };
    let mut state = fresh();
    state.picture.appearance.spectrum.show_roll = false;
    state.picture.runtime.tracker.handle_event(NoteEvent::on(0.0, SourceId::DIRECT, 0, 60, 1.0));
    let view = state.picture.appearance.view.clone();
    assert!(view.glow_draws(), "the fresh glow draws");
    let gain = view.glow_strength;
    let span = view.note_animation.departure(state.picture.appearance.view.fade_time);
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
            assert!(
                !Animating::of(&state).asks_for(Reason::RollInSpan),
                "the roll must not be what keeps this awake"
            );
            let visible = level * gain > 0.5 / 255.0;
            assert_eq!(
                Animating::of(&state).asks_for(Reason::Releasing),
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
    assert!(
        Animating::of(&abandoned).asks_for(Reason::Releasing),
        "the tail was over before the surface hid"
    );
    draw_panes(&ctx, &mut abandoned, 4.05, &[]);
    assert!(
        !Animating::of(&abandoned).asks_for(Reason::Releasing),
        "an undrawn surface held the loop awake"
    );

    state.picture.appearance.view.glow_strength = 0.0;
    draw(&mut state, 60.0);
    assert!(
        !Animating::of(&state).asks_for(Reason::Releasing),
        "a glow that does not draw held the loop awake"
    );
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
    state.picture.appearance.view.fade_time = 0.5;
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
    assert!(Animating::of(&state).asks_for(Reason::Releasing), "the departure was cut short");
    draw_panes(&ctx, &mut state, 1.0 + 1.2, &[Pane::Lattice]);
    assert!(
        !Animating::of(&state).asks_for(Reason::Releasing),
        "the loop stayed awake past the departure"
    );
}

/// Feeds `seconds` of audio from `t` in 10 ms blocks, a full-scale A4 while
/// `tone` and digital silence otherwise, as a host streams both, and answers
/// when the audio next asked for a frame after each block, if it asked.
///
/// Each block draws `panes`, since every picture answers from what it drew.
fn stream(
    ctx: &egui::Context,
    state: &mut SharedState,
    feed: &mut crate::spectrum::TestFeed,
    t: &mut f64,
    seconds: f64,
    tone: bool,
    panes: &[Pane],
) -> Vec<(f64, Option<f64>)> {
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
        feed.feed(&mut state.picture.runtime.spectrum, &block, 1, RATE, *t, &cfg);
        draw_panes(ctx, state, *t, panes);
        animated.push((*t, Animating::of(state).after(Reason::Audible)));
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
/// heatmap's own answer is held to it at every block. Inside it the heatmap
/// asks for every frame while the display it reads still moves, and after
/// that for the frame its scroll or its cloud's drift next crosses a device
/// pixel (#1911), fine pixels, as the Retina display's, making that sooner
/// than the idle poll.
#[test]
fn flowing_silence_idles_once_the_last_sound_has_left_the_span() {
    let mut state = fresh();
    let ctx = super::probe::themed_at(8.0);
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
    let mut feed = crate::spectrum::TestFeed::default();
    let panes = [Pane::Spectral];

    let before = stream(&ctx, &mut state, &mut feed, &mut t, 2.0, false, &panes);
    assert!(before.iter().all(|&(_, on)| on.is_none()), "silence with nothing heard animated");
    let playing = stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &panes);
    assert!(playing[20..].iter().all(|&(_, on)| on == Some(0.0)), "audible audio idled");
    let level = state.picture.surfaces.spectrogram.at(0).held_level.expect("the heatmap drew");
    let slab = crate::spectrogram::width(level);
    // A slab is about a device pixel, so fine pixels make it narrow; three
    // 10 ms blocks still tell it apart.
    assert!(slab >= 0.03, "a slab of {slab} s is too narrow to tell from the blocks");
    let reach = span + slab + 1.5 + memory;
    let stopped = t;
    let after = stream(&ctx, &mut state, &mut feed, &mut t, reach + 3.0, false, &panes);

    // The curve's release outlasts the tone by seconds; the columns do not.
    let heard = state.picture.runtime.spectrum.last_audible();
    let (heard, curve) = (heard.column.unwrap(), heard.curve_until.unwrap());
    assert!(
        heard < curve && curve < stopped + 6.0,
        "column {heard}, curve still at {curve}, {stopped}"
    );
    let mut paced = None;
    for (now, on) in after {
        if state.picture.runtime.spectrum.moving(now) {
            assert_eq!(on, Some(0.0), "at {now}: the display it reads still moved");
        } else if now - heard <= reach {
            let pixel = on.expect("a scroll inside the reach idled");
            assert!(pixel > 0.0 && pixel < 0.05, "at {now}: a scroll paced at {pixel} s");
            assert_eq!(*paced.get_or_insert(pixel), pixel, "at {now}: the scroll's pace moved");
        } else {
            assert_eq!(on, None, "at {now}: last heard at {heard}, past the reach");
        }
    }
    assert!(paced.is_some(), "the fixture never reached the paced scroll");
    assert!(!Animating::of(&state).asks(), "something else held a still picture awake");

    let cfg = &mut state.picture.appearance.spectrum;
    (cfg.tilt, cfg.volume_floor_db, cfg.high_midi) = (-6.0, -100.0, 135.0);
    draw_panes(&ctx, &mut state, t, &panes);
    assert!(
        Animating::of(&state).asks_for(Reason::Audible),
        "silence drawn off the bed's colour idled"
    );

    // A heatmap switched off holds nothing, whatever it last said.
    state.picture.appearance.spectrum.show_spectrogram = false;
    draw_panes(&ctx, &mut state, t, &panes);
    assert!(
        !Animating::of(&state).asks_for(Reason::Audible),
        "a hidden heatmap held the loop awake"
    );
}

/// A heatmap whose pane stops being drawn — folded, hidden, the Video tab
/// closed — moves nothing, so it holds nothing, though its last draw said the
/// sound would scroll for a Span yet; drawn again mid-tail it holds again.
#[test]
fn a_heatmap_that_stops_being_drawn_holds_nothing_until_it_is_drawn_again() {
    let mut state = fresh();
    let ctx = super::probe::themed_at(8.0);
    state.picture.appearance.spectrum.roll_seconds = 60.0;
    let panes = [Pane::Spectral];
    let mut t = 0.0;
    let mut feed = crate::spectrum::TestFeed::default();
    stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &panes);
    let stopped = t;
    let drawn = stream(&ctx, &mut state, &mut feed, &mut t, 0.5, false, &panes);
    assert!(drawn.iter().all(|&(_, on)| on.is_some()), "the heatmap never held the sound");

    let hidden = stream(&ctx, &mut state, &mut feed, &mut t, 15.0, false, &[]);
    assert!(hidden.iter().all(|&(_, on)| on.is_none()), "a hidden heatmap held the loop awake");

    // Long past the curve, well inside the Span: only the heatmap moves.
    let curve = state.picture.runtime.spectrum.moving_until().expect("a tone was heard");
    assert!(curve < t && t < stopped + 30.0, "the curve holds until {curve}, now {t}");
    let revealed = stream(&ctx, &mut state, &mut feed, &mut t, 0.1, false, &panes);
    assert!(revealed.iter().all(|&(_, on)| on.is_some()), "a heatmap drawn again mid-tail idled");
}

/// The heatmap's pace after a sound: the history's scroll or the cloud's
/// drift, whichever crosses a device pixel sooner, so a faster drift asks
/// sooner and a still one leaves the scroll's pace (#1911). Fresh audio with
/// the curve folded, and Stars twinkling on their own clock, ask for every
/// frame.
#[test]
fn the_heatmap_paces_its_scroll_and_drift_and_not_fresh_audio_or_stars() {
    // The pace the heatmap asks for 7 s after a tone, once the display it
    // reads has settled and well inside the Span, with the curve folded,
    // the cloud drawn at `speed`, in `style`; and whether the tone itself
    // asked for every frame.
    let tail = |speed: f32, style: harmonigraph_scene::CloudStyle| {
        let mut state = fresh();
        let ctx = super::probe::themed_at(8.0);
        let spectrum = &mut state.picture.appearance.spectrum;
        // Slow enough a scroll, about 39 ms a device pixel, that a drift at
        // 10x (about 13 ms) and 20x crosses its pixels sooner.
        spectrum.roll_seconds = 100.0;
        // The history takes the whole pane, so the curve draws nothing.
        spectrum.roll_fraction = 1.0;
        let atmosphere = &mut spectrum.atmosphere;
        (atmosphere.cloud_speed, atmosphere.cloud_style) = (speed, style);
        assert!(atmosphere.effects().cloud, "the cloud is drawn");
        let (mut t, mut feed, panes) =
            (0.0, crate::spectrum::TestFeed::default(), [Pane::Spectral]);
        let playing = stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &panes);
        let fresh = playing[20..].iter().all(|&(_, on)| on == Some(0.0));
        let after = stream(&ctx, &mut state, &mut feed, &mut t, 7.0, false, &panes);
        assert!(!state.picture.runtime.spectrum.moving(t), "the display still moves");
        (fresh, after.last().unwrap().1)
    };
    use harmonigraph_scene::CloudStyle::{Stars, Watercolor};
    let (fresh, still) = tail(0.0, Watercolor);
    assert!(fresh, "fresh audio with the curve folded was paced");
    let scroll = still.expect("a still wash's scroll idled");
    let (_, slow) = tail(10.0, Watercolor);
    let (_, fast) = tail(20.0, Watercolor);
    let (slow, fast) = (slow.expect("a drifting wash idled"), fast.expect("a drifting wash idled"));
    assert!(slow < scroll, "a wash drifting faster than the scroll waited for it: {slow} s");
    assert!((slow / fast - 2.0).abs() < 1e-6, "twice the drift paced at {fast} s, not half {slow}");
    let (_, stars) = tail(0.0, Stars);
    assert_eq!(stars, Some(0.0), "twinkling Stars were paced");
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
    let mut feed = crate::spectrum::TestFeed::default();
    stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &panes);
    let mut ring_outlasted_the_display = false;
    let mut wedges_faded = false;
    let mut still: Option<(f64, (String, Vec<u8>))> = None;
    for _ in 0..200 {
        stream(&ctx, &mut state, &mut feed, &mut t, 0.1, false, &panes);
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
            None if !Animating::of(&state).asks() => still = Some((t, nodes)),
            None => {}
        }
        if still.is_some() {
            assert!(!Animating::of(&state).asks(), "woke at {t} with nothing heard");
        }
    }
    assert!(ring_outlasted_the_display, "the ring never held the loop past the curve");
    assert!(wedges_faded, "the ring's gate never opened, so its fade was never measured");
    let curve = state.picture.runtime.spectrum.last_audible().curve_until.unwrap();
    let (idle, _) = still.expect("the ring's release outlasted 20 s");
    assert!(idle - curve < 5.0, "idled {} s after the curve went still", idle - curve);
}

/// A curve fallen under the faintest dB any reading of it draws holds nothing
/// while silence flows on (#1917 Q2): it stops asking for frames seconds
/// before the display reaches silence itself, with no hold after the column
/// it went flat in, and from the first frame nothing asks, the curve, the
/// Spiral and the lattice ring draw nothing new. Under a tilt, with the
/// curve's floor and the colours' apart, so each bound is a per-pitch one.
#[test]
fn a_spectrum_under_its_floor_holds_nothing_and_draws_nothing_new() {
    const RATE: f32 = 48_000.0;
    let mut state = fresh();
    let ctx = super::probe::themed();
    let cfg = &mut state.picture.appearance.spectrum;
    cfg.show_spectrogram = false;
    (cfg.tilt, cfg.floor_db, cfg.volume_floor_db, cfg.release) = (-6.0, -70.0, -85.0, 0.15);
    // The curve takes half the pane, over a range the tone's partials sit in.
    (cfg.roll_fraction, cfg.low_midi, cfg.high_midi) = (0.5, 40.0, 110.0);
    state.picture.appearance.view.spectral_ring_width = 0.1;
    assert!(state.picture.appearance.view.spectral_ring_draws(), "the ring is drawn");
    let mut t = 0.0;
    let mut feed = crate::spectrum::TestFeed::default();
    stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &[Pane::Lattice, Pane::Spectral]);
    let stopped = t;

    // What each picture drew, its paint callbacks left out: what they draw is
    // the ring's bytes, read beside them.
    let mut spiral = crate::panes::spiral::SpiralView::default();
    let mut frame = |state: &mut SharedState, t: f64| {
        begin_frame(&mut state.picture, &super::probe::Defaults, t);
        let mut drawn = Vec::new();
        for pane in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(320.0, 200.0),
                )),
                time: Some(t),
                ..Default::default()
            };
            let output = crate::tests::probe::run(&ctx, raw, |ui| match pane {
                0 => {
                    crate::draw_panes(ui, &[(Pane::Lattice, ui.max_rect())], &mut state.picture, t)
                }
                1 => {
                    crate::draw_panes(ui, &[(Pane::Spectral, ui.max_rect())], &mut state.picture, t)
                }
                _ => crate::panes::spiral::spiral_pane(ui, &mut state.picture, &mut spiral, t),
            });
            if pane > 0 {
                let shapes =
                    output.shapes.iter().filter(|s| !matches!(s.shape, egui::Shape::Callback(_)));
                drawn.push(format!("{:?}", shapes.collect::<Vec<_>>()));
            }
        }
        (drawn, state.picture.runtime.ring_levels.bytes())
    };
    // When the curve, and the Spiral, last drew something new.
    let (mut last, mut changed, mut display_still, mut still) = (None, [stopped; 2], None, None);
    for _ in 0..600 {
        t += 0.01;
        let cfg = state.picture.appearance.spectrum;
        feed.feed(&mut state.picture.runtime.spectrum, &[0.0; 480], 1, RATE, t, &cfg);
        let drawn = frame(&mut state, t);
        if let Some((pictures, _)) = &last {
            let pictures: &Vec<String> = pictures;
            for (at, (was, now)) in changed.iter_mut().zip(pictures.iter().zip(&drawn.0)) {
                if was != now {
                    *at = t;
                }
            }
        }
        // The curve and the Spiral draw nothing new once the display stops,
        // whatever the ring's own release still moves.
        let spectrum = &state.picture.runtime.spectrum;
        match &display_still {
            Some((at, _, then)) => {
                assert_eq!(&drawn.0, then, "the curve or the Spiral moved after {at:.2} s")
            }
            None if !spectrum.moving(t) => {
                let peak = spectrum.display.iter().copied().fold(0.0, f32::max);
                display_still = Some((t, 10.0 * peak.log10(), drawn.0.clone()));
            }
            None => {}
        }
        match &still {
            Some((at, then)) => assert_eq!(&drawn, then, "a picture moved after {at:.2} s"),
            None if !Animating::of(&state).asks() => still = Some((t, drawn.clone())),
            None => {}
        }
        last = Some(drawn);
    }
    for (picture, changed) in ["curve", "Spiral"].into_iter().zip(changed) {
        assert!(changed > stopped + 0.5, "the {picture}'s decay drew nothing ({changed} s)");
    }
    let changed = changed[0].max(changed[1]);
    let (display_still, peak_db, _) = display_still.expect("the display never went still");
    assert!(peak_db > -100.0, "went still at {display_still:.2} s with its peak at {peak_db} dB");
    assert!(
        display_still - changed < 0.5 * AudioSpectrum::HOLD_SECONDS,
        "the display last moved at {changed:.2} s and asked for frames until {display_still:.2} s",
    );
    assert!(still.is_some(), "the editor never went still");
}

/// A sound ending partway through a slab leaves that slab open, and each
/// silent column after it still joins the slab's power mean, which the
/// heatmap's fresh edge draws. So the heatmap asks for every frame until the
/// slab holding the last drawn column closes, though the curve, under a
/// release of nothing, went flat at once (#1917 Q2).
#[test]
fn a_heatmap_slab_still_taking_columns_after_a_sound_asks_for_every_frame() {
    let mut state = fresh();
    let ctx = super::probe::themed_at(2.0);
    let cfg = &mut state.picture.appearance.spectrum;
    (cfg.window, cfg.attack, cfg.release) = (crate::SpectrumWindow::Fast, 0.0, 0.0);
    (cfg.show_spectrogram, cfg.roll_seconds, cfg.roll_fraction) = (true, 40.0, 0.5);
    cfg.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::Watercolor;
    let panes = [Pane::Spectral];
    let mut t = 0.0;
    let mut feed = crate::spectrum::TestFeed::default();
    stream(&ctx, &mut state, &mut feed, &mut t, 1.03, true, &panes);
    let level = state.picture.surfaces.spectrogram.at(0).held_level.expect("the heatmap drew");
    let width = crate::spectrogram::width(level);
    let key = |time: f64| (time / width).floor() as i64;
    let open = |state: &mut SharedState| {
        state.picture.runtime.spectrum.pyramid().level(level).1.last().cloned()
    };
    let (mut slab, mut paced_open, mut moved) = (open(&mut state), 0, 0);
    for _ in 0..100 {
        let asked = stream(&ctx, &mut state, &mut feed, &mut t, 0.01, false, &panes)[0].1;
        let now_slab = open(&mut state);
        let spectrum = &state.picture.runtime.spectrum;
        let heard = spectrum.last_audible().column.expect("the tone was drawn");
        let newest = spectrum.newest_column().expect("columns arrive");
        if !spectrum.moving(t) && key(newest) == key(heard) {
            paced_open += 1;
            moved += usize::from(now_slab != slab);
            assert_eq!(asked, Some(0.0), "at {t:.2} s the open slab took a column and was paced");
        }
        slab = now_slab;
    }
    assert!(
        moved >= 2,
        "the fixture's open slab moved {moved} times of {paced_open} after the curve"
    );
}

/// With the heatmap the only picture of the spectrum on screen, a quiet
/// passage the curve's floor draws and the colours' floor does not leaves the
/// editor still once the last loud column has scrolled through the heatmap's
/// reach, though the curve's bound keeps moving off screen. Sound loud enough
/// for the heatmap then wakes it: each picture's own stamp wakes it.
#[test]
fn a_still_heatmap_alone_wakes_for_sound_after_a_passage_only_the_curve_draws() {
    const RATE: f32 = 48_000.0;
    let mut state = fresh();
    let ctx = super::probe::themed_at(2.0);
    let cfg = &mut state.picture.appearance.spectrum;
    // The history takes the whole pane, so the curve draws nothing.
    (cfg.show_spectrogram, cfg.roll_seconds, cfg.roll_fraction) = (true, 2.0, 1.0);
    (cfg.tilt, cfg.floor_db, cfg.volume_floor_db) = (0.0, -80.0, -60.0);
    cfg.atmosphere.cloud_style = harmonigraph_scene::CloudStyle::Watercolor;
    let mut analysis = crate::spectrum::SpectrumAnalysis::default();
    let mut frames = 0usize;
    // Ten milliseconds of A4 at `amplitude`, handed over as the plugin's
    // input worker hands it, then drawn: whether the hand-over woke the
    // editor, whether it was still before, and the time drawn at.
    let mut block = |state: &mut SharedState, amplitude: f32| {
        let samples: Vec<f32> = (frames..frames + 480)
            .map(|i| amplitude * (std::f32::consts::TAU * 440.0 * i as f32 / RATE).sin())
            .collect();
        frames += 480;
        let t = frames as f64 / f64::from(RATE);
        let still = !Animating::of(state).asks();
        let cfg = state.picture.appearance.spectrum;
        let mut deliveries = Vec::new();
        analysis.feed_source(&samples, 1, RATE, 0.0, &cfg, |d| deliveries.push(d));
        let woke = state.picture.runtime.spectrum.give(deliveries, &cfg);
        draw_panes(&ctx, state, t, &[Pane::Spectral]);
        (woke, still, t)
    };
    for _ in 0..100 {
        block(&mut state, 1.0);
    }
    // At -70 dB: over the curve's floor, under the colours'.
    let (mut went_still, mut quiet_end) = (None, 0.0);
    for _ in 0..6000 {
        let (_, still, t) = block(&mut state, 10f32.powf(-70.0 / 20.0));
        if still {
            went_still.get_or_insert(t);
        }
        quiet_end = t;
    }
    assert!(state.picture.runtime.spectrum.moving(quiet_end), "the curve did not draw it");
    let went_still = went_still.expect("the quiet passage kept the heatmap asking");
    let before = state.picture.runtime.spectrum.last_audible().column;
    let mut woken = false;
    for _ in 0..30 {
        let (woke, still, t) = block(&mut state, 1.0);
        let drawn = state.picture.runtime.spectrum.last_audible().column != before;
        if drawn && !woken {
            assert!(
                woke || !still,
                "at {t:.2} s, still since {went_still:.2} s, sound woke nothing"
            );
            woken = true;
        }
    }
    assert!(woken, "the loud return never reached the heatmap");
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
    let mut feed = crate::spectrum::TestFeed::default();
    stream(&ctx, &mut state, &mut feed, &mut t, 1.0, true, &panes);
    // Once the analysis window has let go of the tone, or the columns after
    // the Clear still hold it and scroll it across as they should.
    stream(&ctx, &mut state, &mut feed, &mut t, 1.0, false, &panes);
    state.picture.runtime.spectrum.clear_history();
    let after = stream(&ctx, &mut state, &mut feed, &mut t, 10.0, false, &panes);
    let curve = state.picture.runtime.spectrum.last_audible().curve_until.unwrap();
    let last =
        after.iter().rev().find(|&&(_, on)| on.is_some()).expect("the curve settled at once").0;
    assert!(last - curve < 1.0, "a Clear held the full rate {} s past the curve", last - curve);
}

/// The armed Record button's breath and a running export's progress stripes
/// are chrome, not picture motion: each asks for its next frame about 30 Hz
/// out rather than holding the editor at the display rate (#1872), and only
/// while some of it is in view: scrolled out of a settings page, it asks for
/// nothing.
#[test]
fn the_chrome_cues_animate_at_about_30_hz_while_in_view() {
    // The repaint delay a settled pass asks for, with the control `below`
    // points down a 400-point screen.
    let delay = |below: f32, add: &mut dyn FnMut(&mut egui::Ui)| {
        let ctx = super::probe::themed();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
        let input = || egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut pass = |ui: &mut egui::Ui| {
            ui.add_space(below);
            add(ui);
        };
        // Settled: a fresh context's opening passes ask to be redrawn.
        let mut output = crate::tests::probe::run(&ctx, input(), &mut pass);
        for _ in 0..4 {
            output = crate::tests::probe::run(&ctx, input(), &mut pass);
        }
        output.viewport_output.values().map(|viewport| viewport.repaint_delay).min().unwrap()
    };
    let mut breath = |ui: &mut egui::Ui| {
        crate::widgets::record_button(ui, &mut true, false, "Record");
    };
    let mut stripes = |ui: &mut egui::Ui| {
        crate::widgets::progress_bar(ui, Some(0.3), "Export", "30%");
    };
    let chrome = crate::CHROME_REPAINT_INTERVAL;
    assert_eq!(delay(0.0, &mut breath), chrome, "the armed Record button's breath");
    assert_eq!(delay(0.0, &mut stripes), chrome, "a running export's stripes");
    assert!(delay(1000.0, &mut breath) > chrome, "a breath out of view asked for frames");
    assert!(delay(1000.0, &mut stripes) > chrome, "stripes out of view asked for frames");
}

/// An idle editor waits the whole idle poll (#1673), and a note held over a
/// still lattice leaves it there once the note has arrived: a voice holds
/// nothing of its own, since the input worker asks for a pass at each edit
/// of the tracker (#1983), and each picture answers for its own motion, the
/// lattice for the note's arrival.
#[test]
fn a_held_note_over_a_still_lattice_asks_for_nothing_once_it_has_arrived() {
    use harmonigraph_core::{NoteEvent, SourceId};

    let delay = |output: &egui::FullOutput| {
        output.viewport_output.values().map(|viewport| viewport.repaint_delay).min().unwrap()
    };
    let mut state = fresh();
    state.workspace.layout = workspace::Layout::solo(panes::Tab::Lattice);
    state.picture.appearance.view.fade_time = 0.2;
    let mut harness = DockHarness::new();
    // Settled: the opening frames lay the dock out and ask to be redrawn.
    let mut idle = harness.frame(&mut state, vec![]);
    for _ in 0..4 {
        idle = harness.frame(&mut state, vec![]);
    }
    // The whole poll, not less (#1673).
    assert_eq!(delay(&idle), crate::IDLE_REPAINT_INTERVAL, "an idle editor waits one poll");

    let at = harness.next_time();
    state.picture.runtime.tracker.handle_event(NoteEvent::on(at, SourceId::DIRECT, 0, 60, 1.0));
    let arriving = harness.frame(&mut state, vec![]);
    assert_eq!(delay(&arriving), std::time::Duration::ZERO, "the note's arrival waited");
    let mut frames = 0;
    while Animating::of(&state).asks() {
        assert!(frames < 60 * 10, "something held the loop for 10 s: {}", Animating::of(&state));
        harness.frame(&mut state, vec![]);
        frames += 1;
    }
    // The frame after the one that settled, which egui redraws for its own
    // layout's sake.
    let still = harness.frame(&mut state, vec![]);
    assert!(state.picture.runtime.tracker.held_count() == 1, "the note must still be held");
    assert_eq!(delay(&still), crate::IDLE_REPAINT_INTERVAL, "a held note held the loop");
}

/// Tab with a modifier is not the chrome toggle and moves no focus. In the
/// plug-in it goes to the host and never reaches egui unless a text field has
/// the keyboard; this drives the UI directly, where it does reach egui, so
/// the UI itself must not run egui's focus traversal on it.
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
    let tabs: Vec<panes::Tab> =
        workspace::Section::ALL.iter().flat_map(|section| section.tabs()).copied().collect();
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
    let pictures = [workspace::Section::Lattice, workspace::Section::Analyzer];
    for &tab in pictures.iter().flat_map(|section| section.tabs()) {
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

/// A released note left scrolling through the roll, with nothing else in
/// motion, has the editor ask for the frame where the roll next crosses a
/// device pixel rather than for every frame the shell offers, or the idle
/// poll (#1911): on a short Span, where a pixel takes a few milliseconds, and
/// on a long one, where it takes longer than the 50 ms poll that slow scrolls
/// rode until #1994 HW1, as at the fresh Span in a full-size editor, and
/// shorter than the poll's quarter second now.
#[test]
fn a_roll_left_scrolling_asks_the_shell_for_its_next_pixel() {
    let short = left_scrolling(2.0, 4.0);
    assert!(short < 0.01, "a short Span on fine pixels paced at {short} s");
    let slow = left_scrolling(40.0, 4.0);
    assert!(
        slow > 0.05 && slow < crate::IDLE_REPAINT_INTERVAL.as_secs_f64(),
        "the fixture's slow scroll must pace between the old poll and the new: {slow} s"
    );
}

/// The pace the shell is asked for once a released note is left scrolling a
/// roll of `span` seconds on `ppp` device pixels a point, with nothing else in
/// motion, checked against the roll's own ask.
fn left_scrolling(span: f32, ppp: f32) -> f64 {
    use harmonigraph_core::{NoteEvent, SourceId};

    let delay = |output: &egui::FullOutput| {
        output.viewport_output.values().map(|viewport| viewport.repaint_delay).min().unwrap()
    };
    let mut state = fresh();
    // A short Lead release and note Fade, so the note settles well inside the
    // Span, and no Background glow, whose colour eases on for longer than a
    // short Span.
    state.picture.appearance.spectrum.roll_seconds = span;
    state.picture.appearance.spectrum.roll_lead_release = 0.25;
    state.picture.appearance.view.fade_time = 0.1;
    state.picture.appearance.view.glow_strength = 0.0;
    let mut harness = DockHarness::new();
    harness.ctx.set_pixels_per_point(ppp);
    let at = harness.next_time();
    let tracker = &mut state.picture.runtime.tracker;
    // A fortieth of the Span, so the ribbon is drawn at any Span.
    let length = f64::from(span) / 40.0;
    tracker.handle_event(NoteEvent::on(at, SourceId::DIRECT, 0, 72, 1.0));
    tracker.handle_event(NoteEvent::off(at + length, SourceId::DIRECT, 0, 72));
    let mut last = harness.frame(&mut state, vec![]);
    let mut frames = 0;
    // Until the note's voice, its Lead and the lattice's departure settle.
    while Animating::of(&state).next_change() == Some(0.0) {
        assert!(frames < 60 * 30, "something held every frame for 30 s: {}", Animating::of(&state));
        last = harness.frame(&mut state, vec![]);
        frames += 1;
    }
    // And the dock's own opening passes, which ask egui to redraw at once.
    for _ in 0..4 {
        last = harness.frame(&mut state, vec![]);
    }
    let held = Animating::of(&state);
    let roll = held.after(Reason::RollInSpan).expect("the roll stopped scrolling first");
    assert_eq!(held.next_change(), Some(roll), "something else asked sooner: {held}");
    let asked = delay(&last).as_secs_f64();
    assert!(
        (asked - roll).abs() < 1e-6,
        "the shell was asked for {asked} s, not the roll's {roll}"
    );
    roll
}
