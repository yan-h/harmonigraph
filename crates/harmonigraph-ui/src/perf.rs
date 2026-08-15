//! The performance overlay: a small HUD over the editor showing the frame
//! rate, the process's memory, and the workload driving them — enough to see
//! at a glance whether the plugin is working the machine hard.
//!
//! Where it sits is the user's, and nothing else's: it opens in the editor's
//! bottom-right corner and is DRAGGED from there to wherever it is wanted,
//! which is the only thing that ever moves it (see [`draw_overlay`]). The
//! position outlives the session — `SharedState::perf_pos`.
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

/// Points between the overlay and the editor's corner it opens in.
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

/// The build row, laid out to `width` — the HUD's own grid width, so naming
/// the build cannot widen the HUD.
///
/// Split out to take the tag as an ARGUMENT: `BUILD_TAG` is stamped from the
/// branch this compiles on, so the wrap is only exercised at all on a branch
/// whose name happens to be long. Nothing about the layout is decided here,
/// which is why it can be handed a tag nobody would ever build under.
fn tag_line(
    ctx: &egui::Context,
    tag: &str,
    font: &egui::FontId,
    color: egui::Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    ctx.fonts_mut(|f| f.layout(format!("build  {tag}"), font.clone(), color, width))
}

/// Draw the overlay over `editor`, at `pos` — dragging it writes the new
/// position back through that handle, and nothing else in the tree moves it.
///
/// `None` is "never dragged", and only then does the HUD have a position of
/// anyone else's choosing: the editor's bottom-right corner, inset, and read
/// off the editor ALONE — no pane, no tab bar, nothing about the arrangement
/// underneath it. That is where it OPENS, not where it belongs: the first drag
/// makes the position the user's and it is honoured from then on.
pub(crate) fn draw_overlay(
    ctx: &egui::Context,
    editor: egui::Rect,
    pos: &mut Option<egui::Pos2>,
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
    // already needs, so it cannot widen the HUD either: a branch name is
    // arbitrarily long, and a HUD as wide as one is a slab across the picture
    // it is measuring. A long one costs a second line instead, where there is
    // room to spare.
    let grid_width = lines
        .iter()
        .map(|parts| parts.iter().map(|(x, g)| x + g.rect.width()).fold(0.0f32, f32::max))
        .fold(0.0f32, f32::max);
    lines.push(vec![(0.0, tag_line(ctx, BUILD_TAG, &mono, dim, grid_width))]);

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
    // Sized from the galleys, which are measured before anything is drawn, so
    // the HUD is placed outright rather than anchored: an Area told where its
    // top-left goes, at a size it already knows.
    let size = egui::vec2(width, height) + margin * 2.0;
    let inset = OVERLAY_INSET * scale;
    // Where an undragged HUD opens — this function's docs say why it is this
    // corner, and why it is a starting point rather than a rule.
    let home = egui::pos2(
        editor.right() - inset - size.x,
        editor.bottom() - inset - size.y,
    );

    // Held inside the editor here, against the size measured THIS frame,
    // rather than by the Area's own `constrain_to`. An Area measures its
    // containment against the size it held last frame, so the frame a HUD
    // loses twenty rows — the breakdown switched off — it is shoved that block
    // of rows clear of the corner and snaps back on the next. The size is in
    // hand right here, so the clamp is exact on the frame it happens.
    //
    // The clamp is on what is DRAWN and never on what is stored: a HUD pushed
    // in by a window too small to hold it where it was dropped goes back there
    // once the window has the room again.
    let inside = |p: egui::Pos2| {
        egui::pos2(
            p.x.clamp(editor.left(), (editor.right() - size.x).max(editor.left())),
            p.y.clamp(editor.top(), (editor.bottom() - size.y).max(editor.top())),
        )
    };
    let at = inside(pos.unwrap_or(home));

    // An Area, so the plate can be dragged. It registers exactly ONE widget
    // rect — its own — because `allocate_space` takes layout room without
    // interacting, and that is the difference that matters: assembled out of
    // `ui.label`s, every row would register a rect of its own and win the
    // pointer, and the HUD would be a dead zone for the lattice's
    // scroll-to-zoom and drag-to-orbit whether or not anyone ever dragged it.
    // It takes the pointer over the plate and nowhere else, and the way out
    // from under it is the drag itself.
    //
    // The move is this function's rather than the Area's `movable`, which
    // would place the plate from a position the clamp above never sees.
    let area = egui::Area::new(egui::Id::new("perf_overlay"))
        .order(egui::Order::Foreground)
        .sense(egui::Sense::drag())
        .fade_in(false)
        // An Area constrains itself by DEFAULT, so this is the line that hands
        // the clamp above its job rather than an omission — left on, the stale
        // size wins and `inside` never decides anything.
        .constrain(false)
        .current_pos(at)
        .show(ctx, |ui| {
            // What `constrain_to` used to cover: a HUD too big for the window
            // is cut off at the editor's edge rather than drawn over whatever
            // is outside it.
            ui.set_clip_rect(ui.clip_rect().intersect(editor));
            let (_, plate) = ui.allocate_space(size);
            let painter = ui.painter();
            painter.rect_filled(plate, 4.0 * scale, egui::Color32::from_black_alpha(0xC0));

            let mut y = plate.top() + margin.y;
            for parts in lines {
                let row_height =
                    parts.iter().map(|(_, g)| g.rect.height()).fold(0.0f32, f32::max);
                for (dx, galley) in parts {
                    painter.galley(egui::pos2(plate.left() + margin.x + dx, y), galley, bright);
                }
                y += row_height + row_gap;
            }
        })
        .response
        .on_hover_and_drag_cursor(egui::CursorIcon::Grab);

    // Written back only while it is dragged, so an untouched HUD goes on
    // opening at `home` — and follows the corner as the window resizes —
    // while a placed one is left exactly where it was put.
    let grabbed_at = egui::Id::new("perf_overlay_grab");
    if area.dragged() {
        // Where the gesture started plus its whole travel, rather than a sum
        // of per-frame deltas: egui calls a press a drag only once it has
        // moved past a click's slop, and those first points are in the total
        // but in no frame's delta. Stored UNCLAMPED, so a pointer that leaves
        // the editor mid-gesture keeps the plate under it on the way back.
        let from = ctx.data_mut(|data| *data.get_temp_mut_or::<egui::Pos2>(grabbed_at, at));
        *pos = Some(from + area.total_drag_delta().unwrap_or_default());
    } else {
        ctx.data_mut(|data| data.remove::<egui::Pos2>(grabbed_at));
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
    /// The build tag wraps rather than widening the HUD, at any branch name.
    ///
    /// A branch name is arbitrarily long and the tag is the one row not sized
    /// by the grid, so it is the one line that can push the HUD out to a slab
    /// across the picture. Held with a tag far longer than anything that would
    /// really be built, because the real [`BUILD_TAG`] is whatever branch this
    /// compiles on: pinned against that, the assertion passes on a short name
    /// whether or not the wrap is there at all, which is how this stopped
    /// being checked.
    #[test]
    fn the_build_tag_wraps_instead_of_widening_the_hud() {
        let ctx = crate::tests::probe::themed();
        let font = egui::FontId::monospace(11.0);
        let grid = 160.0;
        let long = "worktree-a-branch-name-nobody-would-type-but-git-will-take @0123456";
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            let tag = tag_line(ui.ctx(), long, &font, egui::Color32::WHITE, grid);
            assert!(
                tag.rect.width() <= grid,
                "a {} pt tag ran past the {grid} pt grid: {:?}",
                long.len(),
                tag.rect,
            );
            assert!(tag.rect.height() > font.size, "a wrapped tag takes more than one line");
            // ...and a tag that FITS is left on one line rather than broken up
            // for the sake of it.
            let short = tag_line(ui.ctx(), "main @0123456", &font, egui::Color32::WHITE, grid);
            assert!(
                short.rect.height() < tag.rect.height(),
                "a tag that fits should stay on one line: {:?}",
                short.rect,
            );
        });
    }

    /// Labels and values must not collide, whatever the rows are called.
    ///
    /// The label column was a hardcoded seven characters until a row named
    /// "lattice gpu" arrived and the values started printing on top of it.
    /// Driving the assertion off the SAME `rows` the overlay builds means a
    /// future row long enough to break the layout fails here instead.
    #[test]
    fn the_value_column_clears_the_longest_label() {
        let ctx = crate::tests::probe::themed();
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

        // TWO passes, and the second is the one read: an Area's opening pass
        // is a sizing pass, which lays the contents out to measure them and
        // paints nothing. The HUD is drawn from the pass after.
        let mut pos = None;
        let mut frame = || {
            ctx.run_ui(
                egui::RawInput { screen_rect: Some(area), ..Default::default() },
                // Detail on: the widest case.
                |ui| draw_overlay(ui.ctx(), area, &mut pos, &perf, true),
            )
        };
        frame();
        let output = frame();
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

    /// Every reading the shell measures reaches a row: each `pub` field of
    /// [`Workload`] read as `workload.<field>` somewhere above, and each `pub`
    /// field of [`PerfStats`] read as `perf.<field>`.
    ///
    /// A reading plumbed through the shell and printed by nothing is the
    /// failure worth guarding, because it shows up as nothing at all — the
    /// overlay draws its usual rows, not one of them looks wrong, and the
    /// number someone went to the trouble of measuring is simply absent.
    ///
    /// `dead_code` said this for free while both types were `pub(crate)` in a
    /// private module of this crate, and cannot now they are a public crate's
    /// public API: a `pub` field of a reachable struct counts as read whether
    /// or not anything reads it. That moved the declarations into
    /// `harmonigraph-perf` and left the readers here, so this reads both files
    /// — the same shallow parse, one field per line and an identifier before
    /// the first `:`, that `harmonigraph-perf`'s own
    /// `every_frame_cost_reaches_a_stage` runs over [`FrameCosts`].
    ///
    /// What it cannot check is that a field reaches the RIGHT row;
    /// `every_breakdown_row_reports_the_cost_it_names` is what relates a
    /// reading to the label it prints under.
    #[test]
    fn every_reading_reaches_a_row() {
        // Code only, and only the code above the tests: a field named in a doc
        // comment, or bound under that name by a test below, would otherwise
        // answer for itself while no row prints it.
        fn code_of(src: &str) -> String {
            src.split_once("#[cfg(test)]")
                .map_or(src, |(above, _)| above)
                .lines()
                .map(str::trim)
                .filter(|line| !line.starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n")
        }
        // Declarations from the crate that holds them, readers from this one.
        let declared = code_of(include_str!("../../harmonigraph-perf/src/lib.rs"));
        let readers = code_of(include_str!("perf.rs"));

        fn fields<'a>(code: &'a str, decl: &str) -> Vec<&'a str> {
            let body = code
                .split_once(decl)
                .unwrap_or_else(|| panic!("{decl} is declared here"))
                .1
                .split_once('{')
                .expect("it has a body")
                .1
                .split_once('}')
                .expect("its body ends")
                .0;
            body.lines()
                .map(str::trim)
                .filter_map(|line| line.strip_prefix("pub "))
                .map(|field| field.split_once(':').expect("a field line has a `:`").0)
                .collect()
        }

        // `receiver.field` and not a longer name that merely starts with it —
        // `prims` must not be answered for by a later `prims_uploaded`.
        fn reads(code: &str, receiver: &str, field: &str) -> bool {
            let needle = format!("{receiver}.{field}");
            code.match_indices(&needle).any(|(at, _)| {
                !matches!(
                    code[at + needle.len()..].chars().next(),
                    Some(c) if c.is_alphanumeric() || c == '_'
                )
            })
        }

        let workload = fields(&declared, "pub struct Workload");
        let stats = fields(&declared, "pub struct PerfStats");
        // A shallow parse fails by finding nothing, which would pass every
        // assertion below, so it has to show it found both lists first.
        assert!(
            workload.contains(&"animating") && stats.contains(&"rss_bytes"),
            "the field lists did not parse: {workload:?} / {stats:?}",
        );

        for field in workload {
            assert!(
                reads(&readers, "workload", field),
                "`Workload::{field}` is measured every frame and printed by nothing — \
                 give it a row in `overlay_rows`",
            );
        }
        for field in stats {
            assert!(
                reads(&readers, "perf", field),
                "`PerfStats::{field}` is measured every frame and printed by nothing — \
                 give it a row in `overlay_rows`",
            );
        }
    }
}
