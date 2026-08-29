//! The Spectral pane — the audio FFT curve, the sounding voices, and the
//! piano roll of what has been played, all over a shared MIDI-pitch axis —
//! and its settings pane.
//!
//! Everything is drawn in an abstract *(pitch, depth)* plane and mapped to
//! the screen by [`Axes`] at the last moment, so the whole pane
//! turns together when its orientation changes and no element has to know
//! which way is up.
//!
//! A directory rather than a file because this is four drawn layers over one
//! plane, not one pane: [`roll`], [`spectrogram`] and [`names`] each fill a
//! share of the same depth axis, and all three read the same [`axes`]. Held
//! in one file, that shared coordinate system was reachable only as a pane's
//! insides, and the pane was the most-edited file in the crate.

pub(crate) mod axes;
mod gestures;
pub(crate) mod names;
pub(crate) mod roll;
pub(super) mod settings;
pub(crate) mod spectrogram;

/// The Span drag's gain, reached by `spectrogram`'s
/// `a_span_drag_refolds_once_per_rung_crossed` so that its fixture drags at the
/// rate the pane drags at rather than at a copy of it — the whole claim there
/// is about how far a drag travels between ladder rungs, which this sets.
#[cfg(test)]
pub(crate) use gestures::DEPTH_ZOOM_PER_DRAG_POINT;
/// The wheel's gain, shared with the [`spiral`](super::spiral) pane so that one
/// wheel over one analyzer drawn two ways spins at one rate — see
/// [`ZOOM_PER_SCROLL_POINT`](gestures::ZOOM_PER_SCROLL_POINT) for the rate and
/// [`navigate`](super::spiral::navigate) for why the two panes do different
/// things with it.
pub(crate) use gestures::ZOOM_PER_SCROLL_POINT;
pub(crate) use gestures::{hold_spectrum, SpectrumHold};
pub(super) use settings::spectrum_settings_pane;

use crate::panes::window_shows_node;
use crate::{theme, SharedState};
use axes::{
    frequency_grid, label_anchor, level_grid, loudness, plot_budget, power_db,
    spectrogram_level_db, text_scales, Axes, PitchScale, TimeAxis, LABEL_GAP_PT, LABEL_INSET_PT,
    MARKING_PT, PROFILE_PT,
};
use egui::Sense;
use gestures::{drag_split, drag_zoom, spectrum_split};

/// How faint a ruling is drawn against [`theme::hairline`], the pane's
/// quietest line already: the marks that anchor a ladder first, the steps
/// between them second.
///
/// Both grids take the stronger ink for the same job and reach it from opposite
/// directions. On the FREQUENCY axis it is the decade boundary: a log axis
/// draws every decade at the same length, so nothing in the spacing says the
/// step size changes tenfold at 100 Hz and again at 1 kHz — mark those three and
/// the picture reads as one ruler repeated rather than as lines that
/// inexplicably bunch up. Its numbered marks do not need it, because the number
/// is written ON the ruling and points at nothing else.
///
/// On the LEVEL axis it is exactly the numbered rulings that take it, because
/// there the number is written BESIDE its line rather than across it, and the
/// lines it is not written beside are identical to the one it is. A number set
/// against every second or fifth ruling then has to be attributed by counting,
/// and counting is the reading a grid exists to remove — most of all where the
/// type is large against the separation, which is the case the grid is already
/// coarsening for (see [`NUMBERED_LINE_SHARE`](axes)). Inking the named line
/// answers it outright.
///
/// Both stay quieter than the now-line, which is the one line on this pane that
/// divides two pictures rather than measuring one.
const RULING_FADE: (f32, f32) = (0.55, 0.28);

/// The closest two level numbers may be set, in points along the DEPTH axis —
/// the axis they are stacked on, and the one [`level_grid`] thins both them and
/// its rulings against.
///
/// Measured off the type about to draw them rather than assumed, because both
/// things it turns on move. The markings follow the pane's size; and which way
/// the depth axis runs is the orientation's to say, so a number stacked along a
/// horizontal depth axis is spaced by its WIDTH and one stacked down a vertical
/// axis by its height — which on this pane's type differ by more than a factor
/// of two. Projecting the galley onto the depth direction asks that question
/// once instead of case-matching four layouts.
///
/// The widest label the ladder can produce, since the type is monospaced and the
/// window bottoms out at [`LEVEL_MIN_DB`](crate::LEVEL_MIN_DB): four characters.
/// Plus [`LABEL_GAP_PT`] on each side, which is the reach of the halo every
/// label here carries — the rims are what touch first, not the ink.
fn level_label_room(painter: &egui::Painter, axes: &Axes, font: &egui::FontId) -> f32 {
    let galley =
        painter.layout_no_wrap("-100".to_owned(), font.clone(), egui::Color32::PLACEHOLDER);
    let (size, depth) = (galley.size(), axes.dir_depth());
    size.x * depth.x.abs() + size.y * depth.y.abs() + LABEL_GAP_PT * 2.0
}

/// Which side of its ruling a level number is set on, as the depth offset
/// [`Axes::text_anchor`] takes: the side the analyzer's own peaks reach, always.
///
/// Which is to say the side the ANALYZER is on. The spectrum owns one end of the
/// depth axis and hands the rest to the roll, so with the analyzer down the left
/// of the pane its numbers sit left of their lines, down the right they sit
/// right, along the top they sit above. That falls out of one rule rather than
/// four, because the spectrum MIRRORS to meet the now-line: the end its peaks
/// reach is the outer edge in every layout, and the end they stand on is the
/// boundary with the roll.
///
/// The alternative is to choose per number — nearest edge, or whichever side has
/// room — and it reads worse than the case it fixes. A column of numbers is
/// scanned as a column, so one of them answering to a different rule stops the
/// eye at exactly the number that is hardest to place. Where the room genuinely
/// is not there, [`level_grid`] declines to write the number at all.
fn level_label_into(joined: bool) -> f32 {
    // Joined, the spectrum is mirrored: the ceiling sits at the SMALLER depth,
    // so growing toward it runs back down the axis. Standing alone it is the
    // far end and the offset runs forward.
    if joined {
        -LABEL_GAP_PT
    } else {
        LABEL_GAP_PT
    }
}

/// Three views of the same music over one shared MIDI-pitch axis: the
/// audio spectrum as a curve (FFT of the input bus, every partial at its
/// actual pitch), the sounding MIDI voices as bars, and the piano roll of
/// what has been played. All are optional; the settings live in
/// [`spectrum_settings_pane`].
///
/// The depth axis is shared out between the roll (the far end) and the
/// spectrum (the baseline end) at `split`, which is also where the voice
/// bars hang from: a note crosses that one line out of the roll and into
/// the spectrum peak it is making.
///
/// A pointer over this pane changes nothing on it and nothing on the
/// lattice, in either direction. Hovering here to light the matching lattice
/// node is the version that shipped and was withdrawn: the pitch axis is
/// continuous and about fourteen cents wide per point, so a pointer on it
/// does not aim at a node so much as land near one, and the highlight read
/// as a label flickering at unrelated nodes rather than as an answer.
/// Lighting a band here for the lattice-hovered pitch class is the same
/// trade the other way round — in every octave, it puts a stripe across the
/// whole picture, too loud an answer to a pointer resting somewhere else.
pub(crate) fn spectral_pane(
    ui: &mut egui::Ui,
    state: &mut SharedState,
    now: f64,
    // Which spectrogram surface this is: 0 the docked pane / offline render,
    // 1 the Render preview — two live spectrograms in a frame need their own
    // grid.
    surface: usize,
) {
    use harmonigraph_core::spectrum::{BINS_PER_SEMITONE, SPECTRUM_MIN_MIDI};

    let cfg = state.spectrum_config;
    // Drag-sensing, so the pitch range can be panned and the Span or the Level
    // zoomed by grabbing the picture (see `drag_zoom`). Registered BEFORE the
    // divider's own band, which
    // is what leaves the divider on top where the two overlap: egui hands a
    // drag to the last widget registered over the pointer.
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::drag());
    if rect.width() < 10.0 || rect.height() < 10.0 {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::well());

    let axes = Axes::new(rect, &cfg);

    // Offline playhead render: the whole take laid out statically with a
    // sweeping playhead. It takes the whole pane (split = 0), which also drops
    // the live curve and voice bars via their `split > 0` guards — leaving the
    // spectrogram, roll, and playhead.
    let whole_song = state.whole_song.is_some();
    // The divider is grabbable whenever the far region is turned ON, even
    // where it has been dragged shut (`roll_fraction` 0 or 1) — otherwise
    // shutting it would be one-way. Whole-song has no divider: the spectrum
    // isn't drawn at all there.
    // Where the divider stands as the frame opens: the dial, or the docked
    // pane's hold on the spectrum's size (see `spectrum_split`). Both gestures
    // take it rather than reading the config themselves, so the band a hand
    // grabs is the line the picture shows.
    let at_split = spectrum_split(state, surface);
    let divider = (!whole_song && (cfg.show_roll || cfg.show_spectrogram))
        .then(|| drag_split(ui, &axes, state, surface, at_split));
    drag_zoom(ui, &axes, &response, state, surface, at_split);
    // Re-snapshot: the two drags above just wrote `roll_fraction`, the pitch
    // range, the Span or the level ceiling, and everything below has to be this
    // frame's values, not the ones from before the drag. The ceiling is the
    // sharpest case — it is what `loudness` maps through, so a stale copy would
    // draw the curve and the heatmap a frame behind the hand dragging them.
    //
    // The split is re-read for the same reason and answers it by itself: a drag
    // has just moved the dial out from under the hold, which `spectrum_split`
    // reads as the hold no longer describing this picture — so the divider
    // follows the pointer this frame, and the hold re-takes it on the next.
    let cfg = state.spectrum_config;
    let split = if whole_song { 0.0 } else { spectrum_split(state, surface) };

    // The axis: absolute pitch, linear in MIDI note = logarithmic in
    // frequency, so every octave gets equal room and every note draws at
    // its actual pitch. The displayed range is the Analyzer section's pitch
    // range, which is free to start anywhere — it is not snapped to C.
    let min_midi = cfg.low_midi;
    // Never trust the pair to be ordered. A zero or negative span divides by
    // zero in PitchScale and paints NaN geometry, which egui panics on — and
    // a panic here takes the plugin's editor down inside the host. The range
    // bar can't produce one; a hand-edited state blob can.
    let max_midi = cfg.high_midi.max(min_midi + crate::PITCH_RANGE_MIN_SPAN);
    let scale = PitchScale { min_midi, max_midi, span: max_midi - min_midi };
    // Everything this pane sets text at, decided once from the range it just
    // settled on: the markings hold their size, the names follow the zoom.
    let text = text_scales(&cfg, &axes, scale.span, painter.ctx().pixels_per_point());
    // Settled here rather than where the labels are drawn, because the volume
    // grid asks how much room a number takes before it decides which of its
    // lines get one.
    let marking_font = egui::FontId::monospace(MARKING_PT * text.markings);
    // dB depth mapping: the Analyzer section's ceiling tops out where the profile
    // line lands ON the pane's edge (see `plot_budget`) and its floor sets the
    // bottom. Tilt is the conventional reference slope (negative), so the display
    // SUBTRACTS it per octave above the 1 kHz pivot: -4.5 lifts treble
    // 4.5 dB/oct.
    let budget = plot_budget(split, axes.depth_len());
    let d_of = |power: f32, midi: f32| loudness(&cfg, power, midi) * budget;
    // The spectrum joins the spectrogram: its region mirrors so the baseline
    // sits on the now-line (against the spectrogram's newest column) and the
    // peaks point outward. With no roll/spectrogram (split == 1) there's
    // nothing to join, so it stands up from the outer edge as usual.
    let joined = split < 1.0;
    let sd = |d: f32| if joined { split - d } else { d };
    // Labels ride the end of the spectrum its PEAKS reach, not the baseline
    // they stand on, and which screen edge that is flips with the mirroring
    // above — see `label_anchor` for both.
    let (label_d, label_into) = label_anchor(split);
    // Where a level sits on the pane: the level axis runs 0 (the floor, the
    // baseline the curve stands on) to 1 (the ceiling its peaks reach) over the
    // spectrum's depth budget, mirrored by `sd` exactly as the curve is. One
    // mapping for the grid and the numbers, so a ruling lands where a peak of
    // that level lands and the number on it is the number the curve is read
    // against.
    let level_d = |level: f32| sd(level * budget);

    // A uniform dark bed under the whole spectrogram region, so it reads as one
    // surface. The heatmap mesh only covers the depths that actually have
    // columns, and its silence is black; without this bed the un-covered depths
    // (before history fills the window, or past its oldest column) show the
    // lighter pane `well` in jarring patches. Black is the heatmap's own silence
    // color, so covered and un-covered silence match whatever the quad is tinted
    // with: `Color32` is premultiplied, so a black texel over this bed
    // composites to black at every alpha.
    if cfg.show_spectrogram && split < 1.0 {
        let bed = egui::Rect::from_two_pos(axes.at(0.0, split), axes.at(1.0, 1.0));
        painter.rect_filled(bed, 0.0, egui::Color32::BLACK);
    }

    // Axis markings: one frequency ladder, ruled faintly across the spectrum
    // and numbered at the analyzer-standard 1-2-5 series of each decade (see
    // `frequency_grid`). One source for both, so a number always has its own
    // line under it and the two can't drift apart.
    //
    // A mark at every C with Bitwig octave numbers is the alternative reading
    // and stays rejected: every ribbon already carries its note NAME, spelled
    // the lattice's way and placed at the pitch that is sounding. What an axis
    // is for is the other question — where in the spectrum a band sits, which
    // is a frequency.
    let grid = frequency_grid(&scale, axes.pitch_len());

    // ...and one level ladder crossing it, ruled every 10 dB wherever a 10 dB
    // step fits the analyzer it is drawn on (see `level_grid`). The two grids
    // answer the two questions the pane is read for — WHERE in the spectrum a
    // band sits, and HOW LOUD it is — and the second had no answer at all
    // beyond the Level range bar's two end values, which are numbers about the pane
    // rather than marks on it.
    //
    // Measured against the spectrum's depth BUDGET rather than its whole share:
    // that is the length the curve's own ceiling is mapped onto, so a ruling
    // and a peak of the same level are drawn at one place.
    let level_len = budget * axes.depth_len();
    let levels = level_grid(&cfg, level_len, level_label_room(&painter, &axes, &marking_font));

    // The rulings, before anything is drawn over them: they are what the
    // picture sits ON rather than lines across it, which is what makes a grid
    // affordable here at all — over the spectrum's fill or the roll's ribbons
    // they would be a mesh laid across two pictures for a reading the numbers
    // already give. Under them they answer the reading BETWEEN two numbers,
    // which on an axis logarithmic in frequency is not where the eye puts it.
    //
    // They stop at the now-line because that is where the spectrum stops. Run
    // the full depth they would outrun the spectrogram's heatmap — which only
    // grows out from the now-line as history accumulates — and sit bare on the
    // bed ahead of it.
    //
    // The guard is that same rule at its limit rather than a crash guard: a
    // `split` of 0 is a pane with no spectrum on it at all (whole-song mode,
    // and a roll dragged shut over the curve), so there is nothing to rule.
    // egui tessellates a zero-length segment to an invisible degenerate quad,
    // so what this saves is a shape per ruling in every frame of a `--playhead`
    // export, not a NaN.
    if split > 0.0 {
        for ruling in &grid {
            let fade = if ruling.decade { RULING_FADE.0 } else { RULING_FADE.1 };
            painter.line_segment(
                [axes.at(ruling.t, 0.0), axes.at(ruling.t, split)],
                egui::Stroke::new(1.0, theme::hairline().gamma_multiply(fade)),
            );
        }
        // The volume grid, clean across the pitch axis — every ruling the full
        // width of the picture it measures, where a frequency ruling covers the
        // spectrum's share of the depth axis. Between them they mesh the
        // spectrum's region and nothing else, on the same argument: they are
        // what the curve stands on, and a level ruled on past the now-line
        // would be a statement about loudness laid across a heatmap that reads
        // its own.
        for level in &levels {
            let fade = if level.numbered { RULING_FADE.0 } else { RULING_FADE.1 };
            painter.line_segment(
                axes.across_pitch(level_d(level.level)),
                egui::Stroke::new(1.0, theme::hairline().gamma_multiply(fade)),
            );
        }
    }

    let axis_labels: Vec<(f32, String)> = grid
        .iter()
        .filter(|ruling| ruling.numbered)
        .map(|ruling| {
            let hz = ruling.hz;
            let label = if hz >= 1_000.0 { format!("{}k", hz / 1_000.0) } else { format!("{hz}") };
            (ruling.t, label)
        })
        .collect();

    // Nothing to pump here: the analyzer runs off the samples the shell pushes
    // (see `AudioSpectrum::push_samples`), so the spectrogram's columns arrive
    // whether or not the curve is drawn — and whether or not this pane is.
    // The far share of the depth axis: a spectrogram heatmap of the audio
    // and/or the piano roll of what has been played, both on the same
    // `now`-anchored time axis. The spectrogram lays down first (it's a
    // background) and the roll's ribbons sit over it. The spectrum curve is
    // drawn between the two and does not come into that order at all: it has
    // the near share of the axis to itself, so the only place anything it
    // paints can meet anything the roll paints is ON the line dividing them —
    // which is why the marks for `now` go after all three. Turning the ribbons
    // off (`show_roll`) with the spectrogram on leaves the heatmap alone.
    if split < 1.0 && cfg.show_spectrogram {
        spectrogram::draw_spectrogram(&painter, &axes, &scale, state, split, now, surface);
    }

    // Audio spectrum: the FFT of the shell's audio source, every partial
    // at its actual pitch. Fundamentals line up under their voice bars;
    // the harmonic series marches up the axis from each note.
    if split > 0.0 {
        if let Some(levels) = state.spectrum.display(now) {
            // Only the buckets inside the pitch range.
            // One slab per pitch PIXEL, each reading the whole run of buckets
            // that falls in it — not one slab per bucket. The axis holds
            // thousands of buckets and the pane a few hundred pixels, so
            // per-bucket meant thousands of shapes a frame stacked on top of
            // each other, which was survivable only while most buckets were
            // zero.
            //
            // The run under a pixel is RESAMPLED by the heatmap's own operator
            // ([`spectrogram::footprint_mean`]), not read by a MAX, and the two
            // must stay the same read: a pixel of the curve and a pixel of the
            // heatmap cover the same buckets. A MAX here would put a ridge and
            // the curve above it at different heights on the same tone — and it
            // carries the same fault on its own account, since the largest of N
            // draws grows with N, so the curve's noise floor would lift as the
            // pitch axis was zoomed out.
            //
            // DEVICE pixels, which is what the heatmap's `rows` counts, and the
            // reason `ppp` is here at all: sharing the operator is only half of
            // sharing the read, since the footprint fed to it is the width of
            // one column. Sampled per POINT this integrates `ppp` times as many
            // buckets per column as the heatmap does, which on a 2x display is
            // the same disagreement a MAX would cause, arrived at from the
            // other side.
            let bucket_x = |midi: f32| (midi - SPECTRUM_MIN_MIDI) * BINS_PER_SEMITONE as f32;
            let ppp = crate::spectrogram::pane_ppp(painter.ctx());
            let cols = crate::spectrogram::curve_cols(axes.pitch_len(), ppp);
            let visible: Vec<(f32, f32, f32)> = (0..cols)
                .map(|c| {
                    let edge = |i: usize| scale.min_midi + scale.span * i as f32 / cols as f32;
                    let level = spectrogram::footprint_mean(
                        levels,
                        bucket_x(edge(c)),
                        bucket_x(edge(c + 1)),
                    );
                    let t = (c as f32 + 0.5) / cols as f32;
                    (scale.min_midi + t * scale.span, t, level)
                })
                .collect();

            // Color from the SAME gradient as the spectrogram, keyed by the
            // volume-color dB window, so the curve reads in the heatmap's scheme
            // rather than a flat accent. `tint` keeps the gradient's hue/brightness and only
            // sets opacity (gamma_multiply would darken it toward black).
            let hue = |power: f32, midi: f32| {
                spectrogram::cell_color(
                    cfg.spectrogram_gradient,
                    spectrogram_level_db(&cfg, power_db(power), midi),
                )
            };
            let tint = |c: egui::Color32, a: u8| {
                egui::Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
            };

            // The spectrum is a filled shape, like the spectrogram — no outline
            // curve. Each slab is one bucket in its own gradient color, opaque
            // enough to read as a solid fill; densely packed, their tops make
            // the shape's edge (no separate line to fray).
            let slab = axes.pitch_len() / cols as f32 + 0.5;
            for &(midi, t, level) in &visible {
                let d = d_of(level, midi);
                if d * axes.depth_len() > 0.5 {
                    painter.line_segment(
                        [axes.at(t, sd(0.0)), axes.at(t, sd(d))],
                        egui::Stroke::new(slab, tint(hue(level, midi), 210)),
                    );
                }
            }

            // ...and a light rim along their tops, the same edge the note
            // ribbons carry. The spectrum's own colors come from the
            // spectrogram's gradient, so where the curve is quiet it is drawn
            // at that gradient's dark end — against the pane's dark background,
            // with no edge, the shape simply stops existing. Follows the
            // profile the slabs make rather than being a separate curve.
            if let Some(edge) = roll::keyline(&cfg, 1.0) {
                let top: Vec<egui::Pos2> = visible
                    .iter()
                    .map(|&(midi, t, level)| axes.at(t, sd(d_of(level, midi))))
                    .collect();
                painter.add(egui::Shape::line(top, egui::Stroke::new(PROFILE_PT, edge)));
            }
        }
    }

    // Notes sounding at a pitch the lattice has no node for, flagged as a red
    // band down the spectrum at that pitch. The lattice shows nothing for such
    // a note — by construction, since this asks the same window it drew — so
    // this pane is where you would otherwise never learn one was playing, and
    // the band says it in the spectrum's own territory, where there is room
    // for it, instead of recoloring the note and costing you the one thing the
    // ribbon's color is for. Same match the Notes pane
    // uses, over the same window.
    if split > 0.0 && !whole_song {
        let shown = state.shown();
        let mut voices: Vec<&harmonigraph_core::Voice> = state
            .tracker
            .voices()
            .filter(|v| !window_shows_node(&shown, &state.tuning, v.pitch_class))
            .collect();
        // Translucent bands accumulate where they overlap, so the paint
        // order is part of the picture. The tracker's own order is stable —
        // held voices by (channel, note), then the release tail — but it is
        // not this one: pitch decides which band is on top here.
        voices.sort_unstable_by(|a, b| {
            a.pitch.total_cmp(&b.pitch).then(a.channel.cmp(&b.channel)).then(a.note.cmp(&b.note))
        });
        let half = (cfg.roll_thickness * 0.5 / scale.span).max(0.0);
        // One envelope for the whole roll, as every other caller takes it: it
        // is a property of the view and the frame, and rebuilding it per voice
        // would read as if it could vary between them.
        let env = state.view.envelope(&state.frame_params);
        for voice in voices {
            let strength = voice.activation(now, &env);
            if strength <= 0.0 || !scale.contains(voice.pitch) {
                continue;
            }
            let t = scale.t_of(voice.pitch);
            let band = egui::Rect::from_two_pos(axes.at(t - half, 0.0), axes.at(t + half, split));
            painter.rect_filled(band, 0.0, theme::warning_text().gamma_multiply(0.3 * strength));
        }
    }

    // The roll. Its ribbons occupy the far side of the split and the spectrum
    // the near side, so the only thing this shares with the line below is what
    // happens AT the line: a sounding note's ribbon reaches it and carries its
    // lead a little way past, into the spectrum peak it is making (see
    // `roll::lead`, and the divider below for what still draws over it).
    if split < 1.0 && cfg.show_roll {
        roll::draw_roll(&painter, &axes, &scale, state, split, now, surface);
    }

    // The now-line, where the roll hands over to the spectrum — drawn after
    // all three things it divides rather than at the end of the roll. It marks
    // the boundary between two pictures, so it has to sit ON them, and each of
    // the three arrives at it from its own side: the spectrum curve's fill
    // paints down to it, the spectrogram's quad reaches it from the far side,
    // and a sounding note's ribbon crosses it. Any of them eating into the
    // line leaves a divider that thins and flickers with the music instead of
    // holding still — and the roll is the worst of the three, taking half the
    // line's width under every ribbon that is sounding, which is exactly where
    // the picture is busiest and the boundary hardest to follow.
    //
    // The cost is that a note's lead passes UNDER the line rather than over
    // it, so the boundary is drawn across every ribbon reaching through it.
    // That is the right way round: what a sounding note has to show is that it
    // reaches the boundary and comes out the other side as the spectrum peak it
    // is making, and a hairline over the tongue costs it none of that — where
    // an unbroken line is what makes the two sides one picture to read across,
    // and a line chewed away wherever a note crosses reads as a drawing error
    // rather than as the note.
    //
    // Always drawn (there is no setting): the handover is what the pane is
    // built around, so the boundary is marked whether or not anything is
    // sounding on it.
    if !whole_song && split < 1.0 && split > 0.0 {
        painter.line_segment(axes.across_pitch(split), egui::Stroke::new(1.0, theme::hairline()));
    }

    // Whole-song mode marks `now` with a playhead instead — the one moving
    // thing sweeping across a static spectrogram and roll — and it goes here,
    // beside the line it replaces, for the identical reason. This mode gives
    // the roll the WHOLE depth axis (`split` is 0), so the playhead crosses
    // every ribbon on the pane rather than meeting them end-on: drawn under
    // the roll it comes out dashed, notched by each note it passes, and this
    // is the mark the mode is built around. It is also the render behind
    // `--playhead` video export, where a notched sweep is baked into a file.
    if whole_song {
        let time = TimeAxis::new(state, split, now);
        painter.line_segment(
            axes.across_pitch(time.playhead_depth()),
            egui::Stroke::new(1.5, theme::accent()),
        );
    }

    // Axis labels last, riding on top of the spectrogram, spectrum, and
    // voice bars: a label only earns its place if you can read which
    // frequency it marks, and a loud slab would otherwise bury it.
    // Haloed exactly like the lattice's node labels, and for the same reason:
    // whatever is behind them is a picture, not a background. A pitch label
    // over a bright spectrogram slab, or over the spectrum's own fill, has no
    // contrast to rely on at all.
    let mut labels = crate::text::TextBatch::default();
    // Which way the label's own edge faces its ruling: back down the pitch
    // axis, since the anchor above offsets it up that axis and it grows the
    // same way.
    let facing = -axes.dir_pitch();
    for (p, label) in axis_labels {
        let (pos, align) = axes.text_anchor(p, label_d, LABEL_GAP_PT, label_into);
        // That anchor pins the label's layout BOX, and the gap wanted is
        // between the ruling and the digits — so the air the font leaves on
        // the facing edge comes back off it, and `LABEL_GAP_PT` is what a
        // reader measures rather than a floor under it.
        let pos = pos + facing * crate::text::ink_inset(&painter, &label, &marking_font, facing);
        labels.text(
            &painter,
            pos,
            align,
            label,
            marking_font.clone(),
            theme::text_dim(),
            theme::well(),
        );
    }
    // The level numbers, up the high-pitch end of the analyzer — the one end
    // of the pitch axis where a column of numbers crosses nothing it is not
    // measuring. Written where the frequency numbers are not: those ride the
    // depth axis' far edge and read ACROSS the pitch axis, so the two columns
    // meet only in the corner between them.
    //
    // Set BESIDE their rulings rather than across them — a ruling through the
    // middle of the digits cuts the number it is supposed to be named by — and
    // always on the same side of them, the one the analyzer's peaks reach (see
    // `level_label_into`).
    let level_edge = axes.dir_pitch();
    let level_depth = axes.dir_depth();
    let into = level_label_into(joined);
    for level in levels.iter().filter(|level| level.numbered) {
        let label = format!("{}", level.db);
        let (pos, align) = axes.text_anchor(1.0, level_d(level.level), -LABEL_INSET_PT, into);
        // Two corrections, on the two axes the anchor offsets along: the inset a
        // reader measures runs from the pane's edge to the digits, and the gap
        // from the ruling to them. Both are to the INK, so the font's own air on
        // each facing edge comes back off — the same correction the frequency
        // labels make against their rulings, on one axis more.
        let toward_line = -level_depth * into.signum();
        let pos = pos
            + level_edge * crate::text::ink_inset(&painter, &label, &marking_font, level_edge)
            + toward_line * crate::text::ink_inset(&painter, &label, &marking_font, toward_line);
        labels.text(
            &painter,
            pos,
            align,
            label,
            marking_font.clone(),
            theme::text_dim(),
            theme::well(),
        );
    }
    // Each note's own name, over the ribbon it belongs to. In the same batch
    // as the axis labels, and so over the same pictures: a name that could be
    // buried by a loud slab — or by the ribbon it is naming — names nothing.
    let note_names = names::plan(state, &axes, &scale, split, now, text.names);
    names::draw(&painter, &note_names, text.names.label, &mut labels);
    // Flushed before the divider: a batch is drawn where it is flushed, and
    // the divider belongs over the plots, not under the names.
    labels.flush(&painter, rect, state, crate::text::spectral_labels(surface), names_slide(&cfg));

    // The divider, over the plots so it stays findable against a loud
    // spectrogram. Nothing at rest — the roll's now-line already marks where
    // it is, and the offline render (which has no pointer) must keep emitting
    // exactly the shapes it always did.
    if let Some(divider) = &divider {
        let lit = if divider.dragged() {
            Some(theme::accent())
        } else if divider.hovered() {
            Some(theme::accent_edge())
        } else {
            None
        };
        if let Some(color) = lit {
            painter.line_segment(axes.across_pitch(split), egui::Stroke::new(2.0, color));
        }
    }
}

/// Which way this pane's text travels, for the glyph shader's reconstruction
/// filter (`FILTER_TAP` in `text.wgsl`, and [`harmonigraph_render::SlideAxis`]).
///
/// A note name rides the TIME axis: its pitch is where it sits, and time is
/// what scrolls under it. So the answer is the orientation's own, and it is
/// both of the horizontal ones and both of the vertical ones rather than the
/// default either way — a filter fixed on x reconstructs nothing at all for a
/// pane running time down the page, and does it silently, since a picture that
/// is merely resampled worse looks like a picture.
///
/// The axis labels share the batch and stand still, so they are indifferent to
/// this; the one answer serves both.
fn names_slide(cfg: &crate::SpectrumConfig) -> harmonigraph_render::SlideAxis {
    harmonigraph_render::SlideAxis::vertical(cfg.orientation.is_time_vertical())
}

#[cfg(test)]
mod tests;
