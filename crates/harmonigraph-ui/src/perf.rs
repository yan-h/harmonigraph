//! The performance overlay: a small corner HUD over the editor showing the
//! frame rate, the process's memory, and the workload driving them — enough
//! to see at a glance whether the plugin is working the machine hard.
//!
//! The picture is the whole of this module: which rows the HUD shows, and how
//! they are painted. Everything the numbers are MADE of — the windows and
//! their peaks, the stage table, the memory read, the build tag — is
//! `harmonigraph-perf`, and so are the `libc` dependency and the `build.rs`
//! that the last two of those need, neither of which anything else in this
//! crate has a use for.
//!
//! What a frame COSTS lives one checkbox further in, under `show_perf_detail`
//! (see [`draw_overlay`]). The headline list carries the frame interval and
//! its worst recent frame, but the interval's mean is the rate in another
//! unit — `fps` is literally its reciprocal — so the only cost reading the
//! HUD gives without the breakdown is that peak.
//!
//! Interactive only. [`root_ui`](crate::root_ui) times the frame, folds the
//! numbers in through [`PerfStats::record`], and draws the overlay; the
//! offline renderer bypasses `root_ui` entirely, so nothing in this module
//! ever runs on its (deterministic) draw path — no wall-clock read reaches a
//! recorded frame.

use harmonigraph_perf::{memory_readout, PerfStats, Stage, BUILD_TAG, STAGES};

/// Points between the overlay and the corner it sits in.
const OVERLAY_INSET: f32 = 8.0;

/// The overlay's rows: `(depth, label, value, peak)`.
///
/// Split out from the drawing so what the two modes SHOW is assertable
/// without a font, a context, or a painter — the reason `tick` is absent
/// from the basic list is a decision worth a test, and it is invisible in a
/// picture.
fn overlay_rows(perf: &PerfStats, detail: bool) -> Vec<(u8, &'static str, String, Option<String>)> {
    let fading = perf.workload.active_voices.saturating_sub(perf.workload.held_voices);
    // A timed row: the window mean, and the worst frame of the last few
    // windows. Two numbers because neither answers the question alone — the
    // mean says what the stage costs, the peak says whether that cost is
    // steady, and with only one on screen a row reading 2 ms could equally be
    // a stage that got slower or a stage that hiccupped once.
    //
    // Taken by index, which is the same index into [`STAGES`] and into the
    // windows: the label and the depth a row prints under come from the table
    // that says how the stage is measured, so a row cannot end up describing
    // one stage and printing another's number.
    let timed = |i: usize| {
        let (stage, w) = (&STAGES[i], &perf.windows()[i]);
        let (mean, peak) = (format!("{:.1}", w.shown_mean), format!("{:.1} ms", w.shown_peak));
        (stage.depth, stage.label, mean, Some(peak))
    };
    // Depth, label, value, peak. What the nesting means, and why it is worth
    // reading down, is on [`STAGES`], which is where the depths are set.
    //
    // That reading holds for the MEAN column only, which is why the peak sits
    // apart and dimmer. Means are additive — `egui` and `render` sum to `tick`
    // because each frame's parts do — but two stages' worst frames are almost
    // never the same frame, so the peaks do not sum to anything and a column
    // of them must not look like it does.
    //
    // Without `detail` the list is what you read to NOTICE something is wrong:
    // the rate, its worst recent frame, and the workload behind them. A row
    // that only answers WHERE the frame went belongs to the breakdown —
    // `tick` and everything nested under it, and the GPU passes beside them —
    // because that is the question the breakdown exists to answer, and until
    // it is asked the rows are scaffolding sitting over the picture.
    //
    // Note what that leaves: `frame`'s MEAN is the header's `fps` in another
    // unit (`fps()` is its reciprocal), so the peak column is the only thing
    // the headline list says about cost. Reading a rising cost off the basic
    // HUD means watching that peak, or turning the breakdown on.
    let mut rows: Vec<(u8, &str, String, Option<String>)> = vec![timed(Stage::Frame as usize)];
    if detail {
        // Every stage the table says to print, in its order — `tick` and
        // everything nested under it. Filtered out of the table rather than
        // named by a range, so a stage appears in the breakdown by having been
        // added to [`STAGES`], and cannot be measured every frame and then
        // left off the list.
        rows.extend(
            STAGES.iter().enumerate().filter(|(_, s)| s.breakdown).map(|(i, _)| timed(i)),
        );
        let gpu = &STAGES[Stage::Gpu as usize];
        rows.push((gpu.depth, gpu.label, {
            // Both passes on one line, at the depth the table gives them: the
            // top level, because they run alongside the CPU stages rather than
            // inside any of them.
            //
            // Means only. Two peaks as well would be four numbers on one row,
            // and the row would stop being readable long before it became more
            // useful.
            let lattice = if !perf.gpu_supported {
                "n/a".to_owned()
            } else if perf.have_gpu {
                format!("{:.1}", perf.window(Stage::Gpu).shown_mean)
            } else {
                "—".to_owned()
            };
            format!("{:.1} ui · {lattice} 3d", perf.window(Stage::EguiGpu).shown_mean)
        }, None));
        rows.push((0, "verts", format!("{}k in {} prims", perf.verts / 1000, perf.prims), None));
        // The roll's geometry, which `verts` does not see: it goes to the
        // GPU as instances on the roll's own buffer, four vertices a note.
        rows.push((1, "roll", format!("{} notes", perf.roll_notes), None));
        // What the spectrogram's two caches are NOT absorbing. Both should read
        // zero while a window merely scrolls; anything else is a layer that has
        // fallen back to redrawing the whole heatmap, which costs milliseconds
        // and looks exactly like a correct picture.
        let (folds, rings) = perf.spec_fallbacks;
        // Naming which reason dominated is what turns the rate into somewhere
        // to look: the image changed, the pane changed, or the window moved
        // where the texture could not follow.
        //
        // The counting crate reports the SLOT; the name belongs here, because
        // the slots are the analyzer's own restart reasons and nothing below
        // this crate knows what its cache gave up on.
        let why = match perf.spec_restart_slot {
            None => String::new(),
            Some(slot) => format!(" ({})", crate::spectrogram::Restart::LABELS[slot]),
        };
        rows.push((1, "spec", format!("{folds:.0}/s refold · {rings:.0}/s ring{why}"), None));
    }
    rows.extend([
        (0, "memory", memory_readout(perf.rss_bytes), None),
        (
            0,
            "voices",
            format!("{} held · {fading} fading", perf.workload.held_voices),
            None,
        ),
        (
            0,
            "nodes",
            format!(
                "{}  ·  {:.2}× scale",
                perf.workload.visible_nodes, perf.workload.render_scale
            ),
            None,
        ),
    ]);
    rows
}

/// Draw the overlay in the top-right corner of `area` — a picture pane's
/// body, or the editor clear of the tab bar when neither is on screen (see
/// `perf_overlay_area`, which is where the choice and its reasons live). A
/// floating, non-interactive panel so it never steals clicks from the view
/// under it.
pub(crate) fn draw_overlay(
    ctx: &egui::Context,
    area: egui::Rect,
    perf: &PerfStats,
    detail: bool,
) {
    let fps = perf.fps();
    // Only flag a low rate while something is actually animating — an idle
    // editor is meant to drop to the poll rate, so a low idle number is fine.
    let health = if perf.workload.animating && fps < 30.0 {
        egui::Color32::from_rgb(0xE5, 0x7A, 0x5A) // warm red
    } else if perf.workload.animating && fps < 50.0 {
        egui::Color32::from_rgb(0xE0, 0xB0, 0x4A) // amber
    } else {
        egui::Color32::from_rgb(0x7A, 0xC8, 0x8A) // calm green
    };
    let state = if perf.workload.animating { "live" } else { "idle" };

    let dim = egui::Color32::from_gray(0x9A);
    let bright = egui::Color32::from_gray(0xE6);
    // Chrome, so it follows the chrome scale — a HUD left at full size over a
    // panel dialled down would be the largest type in the editor. It sits over
    // the picture but is not part of it: the offline renderer never reaches
    // here, so nothing recorded moves with this.
    let scale = crate::theme::ui_scale(ctx);
    let mono = egui::FontId::monospace(11.0 * scale);
    let head_font = egui::FontId::monospace(12.0 * scale);

    let rows = overlay_rows(perf, detail);

    // Painted straight onto a foreground layer rather than assembled from
    // widgets inside an Area.
    //
    // The Area was already `interactable(false)`, which is enough to keep it
    // out of `layer_id_at` — but every `ui.label` inside it still registered a
    // widget rect, and those win the pointer regardless. The result was a dead
    // zone the size of the HUD in the corner of the lattice: no scroll-to-zoom
    // and no drag-to-orbit under it, for as long as the overlay was up. A
    // readout that changes the thing it is measuring is worse than no readout.
    // Nothing below allocates a widget, so nothing can take the pointer.
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("perf_overlay"),
    ));
    let layout = |text: &str, font: &egui::FontId, color: egui::Color32| {
        ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), color))
    };

    // Three columns, all measured rather than assumed: labels left-aligned in
    // the first, means RIGHT-aligned in the second and peaks in the third, so
    // the digits and the unit line up down the list.
    //
    // The label column is sized from the widest label actually present, not
    // a hardcoded width: a fixed seven characters fits until a row is called
    // "lattice gpu" — eleven characters, with the value column starting
    // underneath it. Measuring cannot drift out of step with the rows.
    let col_gap = 10.0 * scale;
    // Between the labels and the means: the peak answers a different question
    // from the cost beside it, and reading it as a second opinion on that cost
    // is the misreading worth designing against.
    let peak_ink = egui::Color32::from_gray(0xB4);
    let head_fps = layout(&format!("{fps:.0} fps"), &head_font, health);
    let head_state = layout(state, &mono, dim);
    // Say which column is which. Two unlabelled columns of milliseconds is a
    // guess, and the wrong guess — taking a peak for a cost — sends you
    // optimizing a stage that is already fast.
    let head_mean = layout("avg", &mono, dim);
    let head_peak = layout("peak", &mono, dim);
    // Indent by depth, so nesting reads without any drawn guides.
    let labels: Vec<_> = rows
        .iter()
        .map(|(depth, label, _, _)| {
            layout(&format!("{:indent$}{label}", "", indent = *depth as usize * 2), &mono, dim)
        })
        .collect();
    let values: Vec<_> = rows.iter().map(|(_, _, v, _)| layout(v, &mono, bright)).collect();
    let peaks: Vec<_> = rows
        .iter()
        .map(|(_, _, _, p)| p.as_ref().map(|p| layout(p, &mono, peak_ink)))
        .collect();

    let label_col = labels.iter().map(|g| g.rect.width()).fold(0.0f32, f32::max);
    let peak_col = peaks
        .iter()
        .flatten()
        .map(|g| g.rect.width())
        .fold(head_peak.rect.width(), f32::max);
    // Sized over the rows that HAVE a peak only. A row without one (memory,
    // voices) spans both number columns instead, so letting "3 held · 1
    // fading" set this width would shove the peaks off the right edge for
    // nothing.
    let mean_col = values
        .iter()
        .zip(&peaks)
        .filter(|(_, p)| p.is_some())
        .map(|(v, _)| v.rect.width())
        .fold(head_mean.rect.width(), f32::max);
    let spanned = values
        .iter()
        .zip(&peaks)
        .filter(|(_, p)| p.is_none())
        .map(|(v, _)| v.rect.width())
        .fold(0.0f32, f32::max);
    // Everything right of the labels: whichever of the two number columns
    // together and the widest spanning row claims more.
    let nums_x = label_col + col_gap;
    let mean_right = nums_x + (mean_col + col_gap + peak_col).max(spanned) - peak_col - col_gap;
    let peak_right = nums_x + (mean_col + col_gap + peak_col).max(spanned);

    let mut lines: Vec<Vec<(f32, std::sync::Arc<egui::Galley>)>> = Vec::new();
    lines.push(vec![
        (0.0, head_fps.clone()),
        (head_fps.rect.width() + 4.0 * scale, head_state),
    ]);
    lines.push(vec![
        (mean_right - head_mean.rect.width(), head_mean),
        (peak_right - head_peak.rect.width(), head_peak),
    ]);
    for ((label, value), peak) in labels.into_iter().zip(values).zip(peaks) {
        // Right-aligned inside its column, so "4.2" and "12.5" end on the same
        // edge and can be read down the list.
        let mut parts = vec![(0.0, label)];
        match peak {
            Some(peak) => {
                parts.push((mean_right - value.rect.width(), value));
                parts.push((peak_right - peak.rect.width(), peak));
            }
            // Nothing to put in the peak column, so the value takes both and
            // ends on the same right edge — the block still squares off.
            None => parts.push((peak_right - value.rect.width(), value)),
        }
        lines.push(parts);
    }
    // Which build this is — the answer to "did the swap take?", which is a
    // question you have before you trust any number above it.
    //
    // Its OWN line rather than a row in the grid above: the tag is identity,
    // not a measurement, and a long branch name in the value column would
    // widen BOTH columns for every row. And WRAPPED to the width the grid
    // already needs, so it cannot widen the HUD either — the overlay has to
    // fit inside the analyzer pane, which is not always wide, and a branch
    // name is arbitrarily long. A long one costs a second line, where there is
    // room to spare.
    let grid_width = lines
        .iter()
        .map(|parts| parts.iter().map(|(x, g)| x + g.rect.width()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    let tag = ctx.fonts_mut(|f| {
        f.layout(format!("build  {BUILD_TAG}"), mono.clone(), dim, grid_width)
    });
    lines.push(vec![(0.0, tag)]);

    let row_gap = 1.0 * scale;
    let width = lines
        .iter()
        .map(|parts| parts.iter().map(|(x, g)| x + g.rect.width()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    let height = lines
        .iter()
        .map(|parts| parts.iter().map(|(_, g)| g.rect.height()).fold(0.0f32, f32::max))
        .sum::<f32>()
        + row_gap * lines.len().saturating_sub(1) as f32;

    let margin = egui::vec2(8.0, 6.0) * scale;
    // Top-RIGHT of `area`, and clamped to it so a pane too narrow to hold the
    // overlay shows its left edge rather than pushing the numbers off screen.
    //
    // Placed outright rather than anchored. Anchoring existed because a
    // widget-built overlay only learns its own width after laying out, and the
    // right edge has to stay put as the numbers change width underneath it.
    // Measuring the galleys up front settles the size before anything is
    // drawn, so the position is simply known.
    // ...and never wider than the pane it hangs in. Clamping the left edge is
    // not enough on its own: this is painted on a foreground layer whose clip
    // is the whole screen, so nothing else stops a readout wider than its pane
    // from crossing the separator and covering the settings column — including
    // the collapse arrow at the left of every tab bar, which is the control
    // that brings a folded pane back, and the thing the overlay's placement
    // exists to stay off. The analyzer is the narrowest pane the default dock
    // has, and a sideways fold can drive the window to its floor without anyone
    // dragging it there.
    //
    // A readout too wide for its pane is cut off rather than moved or dropped.
    // At a window that narrow no arrangement of it is readable, and the numbers
    // are worth less than the controls underneath them.
    let size = egui::vec2(width, height) + margin * 2.0;
    let inset = OVERLAY_INSET * scale;
    let size = egui::vec2(size.x.min((area.width() - inset * 2.0).max(0.0)), size.y);
    let origin = egui::pos2(
        (area.right() - inset - size.x).max(area.left()),
        area.top() + inset,
    );
    let plate = egui::Rect::from_min_size(origin, size);
    painter.rect_filled(plate, 4.0 * scale, egui::Color32::from_black_alpha(0xC0));
    // The rows are laid out at their own width, which the clamp above does not
    // change, so the plate is what holds them in.
    let painter = painter.with_clip_rect(plate);

    let mut y = origin.y + margin.y;
    for parts in lines {
        let row_height =
            parts.iter().map(|(_, g)| g.rect.height()).fold(0.0f32, f32::max);
        for (dx, galley) in parts {
            painter.galley(egui::pos2(origin.x + margin.x + dx, y), galley, bright);
        }
        y += row_height + row_gap;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_perf::{FrameCosts, Workload};

    /// The basic overlay answers "is something wrong", the breakdown answers
    /// "where did the frame go" — so every row that only serves the second
    /// question waits for `detail`, `tick` and `gpu` included. The breakdown
    /// EXPANDS the headline list rather than replacing it: the headline rows
    /// keep their order and their place, so a glance at one mode transfers
    /// to the other.
    ///
    /// Worth asserting rather than eyeballing: both modes draw a plausible
    /// HUD, and a row leaking back into the basic list is a change nobody
    /// notices until the corner is cluttered again.
    #[test]
    fn the_basic_overlay_is_the_headline_rows_only() {
        let perf = PerfStats::default();
        let label_of = |rows: &[(u8, &'static str, String, Option<String>)]| {
            rows.iter().map(|(_, label, _, _)| *label).collect::<Vec<_>>()
        };

        let basic = label_of(&overlay_rows(&perf, false));
        assert_eq!(
            basic,
            ["frame", "memory", "voices", "nodes"],
            "the basic overlay is the rate, its worst frame, and the workload behind them",
        );

        // WHICH rows the breakdown adds, and at what depth, is
        // [`the_breakdown_is_this_exact_list_of_rows`]. Here it is only the
        // relationship between the two modes.
        let detail = label_of(&overlay_rows(&perf, true));
        // The breakdown EXPANDS the headline list: every basic row survives,
        // in order, and — the part a subsequence check alone does not say —
        // each stays where it was, with the new rows spliced INTO the list
        // rather than parked before or after it. Checking only the
        // subsequence let the whole breakdown move to the end of the HUD and
        // still pass, which is exactly the glance that stops transferring.
        assert_eq!(detail.first(), basic.first(), "the breakdown must still open on `frame`");
        let tail = basic.len() - 1;
        assert_eq!(
            detail[detail.len() - tail..],
            basic[1..],
            "the workload rows belong at the foot of both lists",
        );

    }

    /// The breakdown, written out: every row, in order, at its depth.
    ///
    /// Deliberately a second copy of [`STAGES`], kept by hand, because the two
    /// ways that table goes wrong SILENTLY are omission and transposition and
    /// neither is visible from inside it. Turning a `measured` entry into a
    /// `by_hand` one drops its row while the stage goes on accumulating and
    /// latching every frame: the HUD simply stops mentioning a cost it is
    /// still measuring, and nothing about the remaining rows looks wrong. The
    /// index assertion under the table is no help — it makes REORDERING a
    /// compile error, which was already the one mistake that could not happen
    /// quietly.
    ///
    /// So adding a stage costs a line here, and that is the point rather than
    /// an oversight. A `contains` check over a handful of the rows is what
    /// this replaced, and it saw neither failure.
    ///
    /// The depths are half of what is pinned. The nesting is what lets a total
    /// and its parts be read against each other; flattened, the breakdown
    /// draws a plausible list of numbers that has quietly stopped saying what
    /// contains what.
    #[test]
    fn the_breakdown_is_this_exact_list_of_rows() {
        let perf = PerfStats::default();
        let rows: Vec<(u8, &str)> =
            overlay_rows(&perf, true).iter().map(|(depth, label, _, _)| (*depth, *label)).collect();
        assert_eq!(
            rows,
            [
                (0, "frame"),
                // `tick` and its two halves, `egui` and `render`, which sum to
                // it by construction.
                (0, "tick"),
                (1, "egui"),
                (2, "shell"),
                (2, "ui"),
                (1, "render"),
                (2, "tess"),
                (2, "tex up"),
                (2, "buf up"),
                (3, "ubuf"),
                (4, "prep"),
                (4, "poll"),
                (4, "write"),
                (4, "scene"),
                (3, "around"),
                (2, "wait"),
                (2, "encode"),
                (2, "submit"),
                // The GPU pair share this one line, so `egui gpu`'s own label
                // and depth never reach the screen.
                (0, "gpu"),
                (0, "verts"),
                (1, "roll"),
                (1, "spec"),
                // The headline rows, which the breakdown keeps at the foot.
                (0, "memory"),
                (0, "voices"),
                (0, "nodes"),
            ],
        );
    }

    /// Every row prints the cost its label names.
    ///
    /// Transposing two `sample` closures in [`STAGES`] puts scene's cost under
    /// `write` and write's under `scene`. Both rows stay, both hold a
    /// plausible millisecond figure, and the HUD is confidently wrong about
    /// where the frame went — which is worse than a missing row, because this
    /// overlay is the diagnostic channel and a number under a name gets
    /// believed. Only a test that relates a [`FrameCosts`] field to the label
    /// it ends up under can see it.
    ///
    /// One distinct value per field, so no two rows can read the same number
    /// and swapping any pair changes what prints. The three DERIVED rows are
    /// pinned as their arithmetic rather than as a field, since that is what
    /// they are; their values are chosen not to collide with any reading
    /// either, so `egui` cannot quietly print `shell`'s.
    ///
    /// Every row is here, not just the millisecond ones. `spec` and `voices`
    /// are the rows most exposed to this and the last to be covered: each
    /// carries TWO independently-named quantities inside one format string, so
    /// a transposition needs no second row to hide in and no type to disagree
    /// with. Reading a refold rate as a ring rate sends the next investigation
    /// to the aggregator when the ring cache is what stopped absorbing — and
    /// `spec` is the readout kept precisely because it is the one that would
    /// have caught both of the spectrogram's silent performance bugs.
    ///
    /// The last four are set on `perf` rather than driven through `record`:
    /// what is under test is which field reaches which label, and the
    /// derivations that fill them (a rate over an interval, an eased memory
    /// reading) are pinned by tests of their own.
    #[test]
    fn every_breakdown_row_reports_the_cost_it_names() {
        // Readings 1..12 in table order, then the three the derived rows are
        // computed from, spread far enough up that no difference lands on a
        // reading: egui = 100 - 60, buf up = 50 - 4, around = 50 - 4 - 5.
        let costs = FrameCosts {
            shell_ms: 1.0,
            cpu_ms: 2.0,
            tess_ms: 3.0,
            texture_ms: 4.0,
            ubuf_ms: 5.0,
            prepare_ms: 6.0,
            poll_ms: 7.0,
            write_ms: 8.0,
            scene_ms: 9.0,
            acquire_ms: 10.0,
            encode_ms: 11.0,
            submit_ms: 12.0,
            egui_gpu_ms: 20.0,
            lattice_gpu_ms: 30.0,
            render_ms: 60.0,
            tick_ms: 100.0,
            upload_ms: 50.0,
            prims: 7,
            verts: 5000,
            roll_notes: 13,
            spectrogram_fallbacks: Default::default(),
        };
        let mut perf = PerfStats::default();
        let mut now = 0.0;
        // Past one latch with a real interval behind it, so `frame` has a
        // measurement of its own rather than the opening frame's absent one.
        for _ in 0..20 {
            now += 1.0 / 60.0;
            perf.record(
                costs,
                now,
                // 17 held and 36 sounding, so `fading` is 19 — a third value
                // rather than a difference either operand could stand in for.
                Workload {
                    active_voices: 36,
                    held_voices: 17,
                    visible_nodes: 21,
                    render_scale: 3.25,
                    animating: true,
                },
            );
        }
        perf.spec_fallbacks = (14.0, 15.0);
        perf.spec_restart_slot = Some(4);
        perf.rss_bytes = 490 * 1024 * 1024;

        let rows = overlay_rows(&perf, true);
        let value_of = |label: &str| {
            rows.iter().find(|(_, l, _, _)| *l == label).map(|(_, _, v, _)| v.clone())
        };
        for (label, value) in [
            // 60 Hz in milliseconds — the header's fps in the unit every other
            // row is held in.
            ("frame", "16.7"),
            ("tick", "100.0"),
            ("egui", "40.0"),
            ("shell", "1.0"),
            ("ui", "2.0"),
            ("render", "60.0"),
            ("tess", "3.0"),
            ("tex up", "4.0"),
            ("buf up", "46.0"),
            ("ubuf", "5.0"),
            ("prep", "6.0"),
            ("poll", "7.0"),
            ("write", "8.0"),
            ("scene", "9.0"),
            ("around", "41.0"),
            ("wait", "10.0"),
            ("encode", "11.0"),
            ("submit", "12.0"),
            // The one row carrying two stages' numbers: egui's pass and the
            // lattice's, in that order.
            ("gpu", "20.0 ui · 30.0 3d"),
            ("verts", "5k in 7 prims"),
            ("roll", "13 notes"),
            // The two caches, in the order the labels name them, with the
            // dominant reason on the end where it reads as a note on the ring
            // rate rather than as part of it.
            ("spec", "14/s refold · 15/s ring (pane)"),
            ("memory", "490 MB"),
            ("voices", "17 held · 19 fading"),
            ("nodes", "21  ·  3.25× scale"),
        ] {
            assert_eq!(
                value_of(label).as_deref(),
                Some(value),
                "`{label}` must report the cost it names",
            );
        }
    }
    /// Labels and values must not collide, whatever the rows are called.
    ///
    /// The label column was a hardcoded seven characters until a row named
    /// "lattice gpu" arrived and the values started printing on top of it.
    /// Driving the assertion off the SAME `rows` the overlay builds means a
    /// future row long enough to break the layout fails here instead.
    #[test]
    fn the_value_column_clears_the_longest_label() {
        let ctx = egui::Context::default();
        crate::theme::apply_theme(&ctx); // real metrics, not egui's fallback
        let area = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1000.0, 800.0));
        let mut perf = PerfStats::default();
        // A reading in every row, so none of them lays out as a short
        // placeholder and hides the widest case.
        perf.record(
            FrameCosts {
                // Enough of both to lay out at their widest: the row carries
                // two rates, so a build falling back hard is the long case.
                spectrogram_fallbacks: (900, &[150; crate::spectrogram::Restart::COUNT]),
                shell_ms: 1.0,
                cpu_ms: 2.0,
                tess_ms: 3.0,
                egui_gpu_ms: 4.0,
                lattice_gpu_ms: 5.0,
                acquire_ms: 6.0,
                tick_ms: 7.0,
                render_ms: 8.0,
                upload_ms: 9.0,
                texture_ms: 8.5,
                prims: 0,
                verts: 0,
                roll_notes: 0,
                prepare_ms: 1.0,
                poll_ms: 0.5,
                ubuf_ms: 1.2,
                write_ms: 0.25,
                scene_ms: 0.25,
                encode_ms: 10.0,
                submit_ms: 11.0,
            },
            1.0,
            Workload { animating: true, ..Default::default() },
        );

        let output = ctx.run_ui(
            egui::RawInput { screen_rect: Some(area), ..Default::default() },
            |ui| draw_overlay(ui.ctx(), area, &perf, true), // detail on: the widest case
        );
        let mut texts: Vec<(egui::Rect, String)> = output
            .shapes
            .iter()
            .filter_map(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => Some((
                    egui::Rect::from_min_size(text.pos, text.galley.size()),
                    text.galley.text().to_owned(),
                )),
                _ => None,
            })
            .collect();
        texts.sort_by(|a, b| a.0.top().total_cmp(&b.0.top()));
        assert!(texts.len() > 8, "expected a row per reading, got {}", texts.len());

        // Within each row (same top), nothing may start before the previous
        // piece ends.
        for pair in texts.windows(2) {
            let ((a, at), (b, bt)) = (&pair[0], &pair[1]);
            if (a.top() - b.top()).abs() > 0.5 {
                continue; // different rows
            }
            assert!(
                b.left() >= a.right(),
                "{at:?} and {bt:?} overlap: {a:?} then {b:?}",
            );
        }
    }
}
