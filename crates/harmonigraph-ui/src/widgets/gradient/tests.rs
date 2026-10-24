use super::*;
use crate::widgets::mesh::{band_bounds, band_colors, band_columns, bands, fades_out_at_its_edges};
use crate::widgets::probe::{
    after_passes, filled_rects, grips, handles, knockouts, painted, painted_in, painted_text,
    press, text_boxes,
};
use harmonigraph_scene::ViewConfig;

/// A [`SpectrumBar`] under a [`GradientPreview`] in a 300pt context,
/// driven one frame at a time.
///
/// Real events through a real context, because nothing less reaches the
/// widget: what a gesture has hold of is decided on the first frame egui
/// calls the press a drag and then remembered in context data, so a single
/// synthetic call would exercise neither the decision nor the memory. A
/// bare context is the design scale, 1.0, which is why the geometry below
/// reads the constants unmultiplied.
///
/// The preview is drawn as well as the bar, which is not scene-setting: it
/// senses hover alone, so a press on it is handed to the nearest widget
/// that takes drags — the track, a row's spacing below — and only the bar's
/// own position check declines it. A harness holding the bar by itself
/// would leave that check unreachable
/// (`the_preview_is_a_picture_and_not_a_control` is what walks it).
struct Spectrum {
    ctx: egui::Context,
    screen: egui::Rect,
    rect: egui::Rect,
    /// The preview above the bar, read back off the frame just drawn.
    preview: egui::Rect,
    t: f64,
    /// What the bar is told to reset to: the lattice's, as the pane
    /// hands it, unless a test says otherwise.
    home: Gradient,
    /// The chrome scale the bar is drawn at, put back in force on every
    /// frame because that is how a shell holds it — see
    /// [`crate::theme::set_ui_scale`].
    scale: f32,
}

impl Spectrum {
    /// Laid out once before anything is aimed at it: egui resolves the
    /// pointer against the PREVIOUS pass's rects, so a press cannot land on
    /// a bar that has never been drawn.
    fn settled(g: &mut Gradient) -> Spectrum {
        Spectrum::settled_at(g, 300.0)
    }

    /// The same, in a column of a named width — for the questions whose
    /// answer is arithmetic against the width rather than against the
    /// gradient. 173pt is the narrowest column the dock gives a pane, so a
    /// sweep that reaches it has reached everything a reader can drag to.
    fn settled_at(g: &mut Gradient, width: f32) -> Spectrum {
        Spectrum::settled_scaled(g, width, 1.0)
    }

    /// The same, with the chrome dialled somewhere other than the design
    /// size — for the questions whose answer is a length that the scale
    /// either does or does not multiply. A bare context reads 1.0, so
    /// every other harness here asks the bar the same question at one
    /// scale only.
    fn settled_scaled(g: &mut Gradient, width: f32, scale: f32) -> Spectrum {
        let ctx = crate::tests::probe::themed();
        let mut h = Spectrum {
            ctx,
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(width, 100.0)),
            rect: egui::Rect::NOTHING,
            preview: egui::Rect::NOTHING,
            t: 0.0,
            home: ViewConfig::default().pitch_gradient,
            scale,
        };
        // Twice: `set_ui_scale` rebuilds the style, and a `Ui` built from
        // the old one is a frame behind — so the pass that settles the
        // rects has to be the second.
        h.frame(g, vec![]);
        h.frame(g, vec![]);
        h
    }

    /// The same bar, told to reset somewhere other than the lattice's
    /// gradient — the Spectral pane's own case.
    fn settled_with_home(g: &mut Gradient, home: Gradient) -> Spectrum {
        let mut h = Spectrum::settled(g);
        h.home = home;
        // Laid out again under the new home, for the reason `settled`
        // lays out at all: egui resolves a press against the previous
        // pass's rects.
        h.frame(g, vec![]);
        h
    }

    fn frame(&mut self, g: &mut Gradient, events: Vec<egui::Event>) -> Vec<egui::Shape> {
        self.t += 1.0 / 60.0;
        crate::theme::set_ui_scale(&self.ctx, self.scale);
        let rect = std::cell::Cell::new(egui::Rect::NOTHING);
        let preview = std::cell::Cell::new(egui::Rect::NOTHING);
        let out = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(self.screen),
                time: Some(self.t),
                events,
                ..Default::default()
            },
            |ui| {
                // The group's own order: the preview's row taken first, its
                // colors read after the bar has written — see
                // [`GradientPreview`]. A harness that painted it where it
                // stands would model a pane this repo does not have and
                // would pass `a_spectrum_drag_draws_the_preview_it_just_set`
                // against a picture that lags.
                let slot = GradientPreview::reserve(ui);
                rect.set(SpectrumBar::new(g, self.home).show(ui).rect);
                preview.set(slot.show(ui, g).rect);
            },
        );
        self.rect = rect.get();
        self.preview = preview.get();
        out.shapes.into_iter().map(|s| s.shape).collect()
    }

    /// The stretch a handle travels. The bar hands back the TRACK's rect
    /// rather than the whole row it allocated — the flip button beside it
    /// carries its own tooltip — so the inset at either end is all that
    /// separates the two.
    fn track(&self) -> egui::Rect {
        self.rect.shrink2(Vec2::new(HANDLE_INSET * self.scale, 0.0))
    }

    /// The middle of the flip button, in the gutter past the track's right
    /// edge.
    fn on_flip(&self) -> egui::Pos2 {
        egui::pos2(self.rect.right() + FLIP_W * 0.5, self.rect.center().y)
    }

    /// A point on the preview, `across` of the way along it and `up` of the
    /// way through it — 0 at the edge nearest the track below, which is the
    /// depth that matters. egui hands a press that hits nothing to the
    /// nearest widget within `interact_radius`, so the bottom of the
    /// preview is the part the row spacing alone does not protect, and a
    /// probe at the preview's middle sits clear of the only place the
    /// geometry can fail.
    fn on_preview(&self, across: f32, up: f32) -> egui::Pos2 {
        egui::pos2(
            self.preview.left() + self.preview.width() * across,
            self.preview.bottom() - self.preview.height() * up,
        )
    }

    /// Where a span of `span` degrees stands the handle.
    fn at_span(&self, span: f32) -> egui::Pos2 {
        let track = self.track();
        let across = (span / FULL_TURN).abs().clamp(0.0, 1.0);
        egui::pos2(track.left() + track.width() * across, track.center().y)
    }

    /// Press and release at one spot, answering what the frame carrying
    /// the release painted — which is the frame a click lands on.
    fn click(&mut self, g: &mut Gradient, at: egui::Pos2) -> Vec<egui::Shape> {
        self.frame(g, vec![egui::Event::PointerMoved(at)]);
        self.frame(g, vec![press(at, true)]);
        self.frame(g, vec![press(at, false)])
    }

    /// Press at `from` and drag to `to`, answering what the arriving frame
    /// painted.
    ///
    /// A step clear of egui's drag threshold comes first, and it is the
    /// part of this that models a real hand: egui calls the press a drag
    /// only once the pointer has left that threshold, so the first frame
    /// the widget sees is ALWAYS some way along the drag and never at
    /// `from`. A harness that jumped straight to `to` would hand the widget
    /// a first frame at its destination and never exercise the gap.
    fn drag(&mut self, g: &mut Gradient, from: egui::Pos2, to: egui::Pos2) -> Vec<egui::Shape> {
        self.frame(g, vec![egui::Event::PointerMoved(from)]);
        self.frame(g, vec![egui::Event::PointerMoved(from), press(from, true)]);
        let live_at = from + (to - from).normalized() * 12.0;
        self.frame(g, vec![egui::Event::PointerMoved(live_at)]);
        self.frame(g, vec![egui::Event::PointerMoved(to)])
    }

    /// Two clicks at one spot, close enough together to be one gesture.
    fn double_click(&mut self, g: &mut Gradient, at: egui::Pos2) {
        self.frame(g, vec![egui::Event::PointerMoved(at)]);
        for _ in 0..2 {
            self.frame(g, vec![press(at, true)]);
            self.frame(g, vec![press(at, false)]);
        }
    }
}

/// The text runs a spectrum bar draws, each with the box it fills and the
/// color it was painted in — the color being a claim of its own here, since
/// the name and the readout stand on different halves of the track and are
/// drawn from opposite ends of the palette for it.
///
/// Knockouts are passed over, as [`text_boxes`] passes them: a second pass
/// over a run the handle stands in, not a run of its own.
fn spectrum_texts(shapes: &[egui::Shape]) -> Vec<(egui::Rect, String, Color32)> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if t.override_text_color.is_none() => Some((
                egui::Rect::from_min_size(t.pos, t.galley.size()),
                // The GLYPHS, not `Galley::text()`, which answers with the
                // string the galley was laid out from and so reads the same
                // whether or not the name fitted — see [`painted_text`].
                painted_text(&t.galley),
                t.galley.job.sections.first().map_or(t.fallback_color, |s| s.format.color),
            )),
            _ => None,
        })
        .collect()
}

/// The span a spectrum bar reads out, as against the name it writes at the
/// left. Both are asserted, so a name that has stopped being drawn is a
/// failure rather than a readout read off the wrong run.
fn spectrum_readout(shapes: &[egui::Shape]) -> String {
    let texts: Vec<String> = spectrum_texts(shapes).into_iter().map(|(_, s, _)| s).collect();
    assert_eq!(texts.len(), 2, "a spectrum bar draws its name and one readout, not {texts:?}",);
    assert_eq!(texts[0], SPAN_LABEL, "the bar's first text run is not its name");
    texts.into_iter().nth(1).expect("checked just above")
}

/// The same two, named: `(preview, circle)`.
///
/// Told apart by WHERE they are rather than by the order they are drawn in.
/// The preview is painted last on purpose (see [`GradientPreview`]), so
/// draw order says nothing about which band is which, and a helper that
/// read it would have quietly swapped the two the day that changed.
fn spectrum_bands(shapes: &[egui::Shape]) -> (egui::Mesh, egui::Mesh) {
    let mut bands = bands(shapes);
    assert_eq!(bands.len(), 2, "a preview and a spectrum bar paint two bands, not {}", bands.len(),);
    bands.sort_by(|a, b| band_bounds(a).top().total_cmp(&band_bounds(b).top()));
    let mut bands = bands.into_iter();
    let preview = bands.next().expect("checked just above");
    let circle = bands.next().expect("checked just above");
    assert!(
        band_bounds(&preview).bottom() <= band_bounds(&circle).top(),
        "the two bands overlap, so neither is the picture above the other",
    );
    (preview, circle)
}

/// The track is hue and nothing else: the brightness and chroma bars move
/// the preview above it and leave the arc alone.
///
/// That division is the whole of what the two bands are for, and it is
/// invisible to every other test here — a track painted out of the pitch
/// ramp draws the same rounded mesh in the same place at the same size, and
/// passes all of them. What it costs is the control: a grey ramp is one drag
/// of the Saturation bar, and a track that answered the chroma pair would be a
/// hue picker drawn in grey.
#[test]
fn the_track_is_hue_alone_and_the_preview_is_the_gradient() {
    let base = Gradient {
        hue_start: 30.0,
        hue_span: 180.0,
        lightness: 60.0,
        lightness_ramp: 0.0,
        chroma: 0.6,
        chroma_ramp: 0.0,
        ..Gradient::default()
    };
    let painted = |g: Gradient| {
        let mut g = g;
        let mut h = Spectrum::settled(&mut g);
        let shapes = h.frame(&mut g, vec![]);
        let (preview, circle) = spectrum_bands(&shapes);
        (band_colors(&circle), band_colors(&preview))
    };
    let (track, preview) = painted(base);
    for (what, dialled) in [
        ("a steep brightness ramp", Gradient { lightness: 50.0, lightness_ramp: 100.0, ..base }),
        ("a steep chroma ramp", Gradient { chroma: 0.5, chroma_ramp: 1.0, ..base }),
        ("a dark picture", Gradient { lightness: 12.0, ..base }),
        ("a grey ramp", Gradient { lightness: 50.0, lightness_ramp: 100.0, chroma: 0.0, ..base }),
    ] {
        let (moved, drawn) = painted(dialled);
        assert_eq!(moved, track, "{what} moved the hue track");
        // Without this half the claim above is satisfied by a bar that
        // draws nothing at all: the preview is where those two knobs live,
        // and a gradient that changed neither band would prove nothing.
        assert_ne!(drawn, preview, "{what} left the preview alone too");
    }
    // And the track does answer the two knobs it IS: the equality above is
    // a track holding still under the other four, not a picture that never
    // moves.
    for (what, dialled) in [
        ("turning the circle", Gradient { hue_start: base.hue_start + 40.0, ..base }),
        ("widening the arc", Gradient { hue_span: base.hue_span + 60.0, ..base }),
    ] {
        assert_ne!(painted(dialled).0, track, "{what} left the track alone");
    }
}

/// Past the handle the track carries on in the hues the arc WOULD take if
/// it reached them, held back — one circle at two strengths, so the two
/// halves meet flush at every setting.
///
/// Measured against the same bar with its arc opened to a whole turn, which
/// is the only way to ask the question without writing the color out a
/// second time here: every column past the handle must be the color that
/// column is painted when the arc claims it, at [`UNCLAIMED_ALPHA`].
#[test]
fn the_dimmed_remainder_is_the_same_circle_held_back() {
    let arc = Gradient {
        hue_start: 200.0,
        hue_span: 120.0,
        lightness: 50.0,
        lightness_ramp: 60.0,
        chroma: 0.5,
        chroma_ramp: 0.6,
        ..Gradient::default()
    };
    let track_of = |g: Gradient| {
        let mut g = g;
        let mut h = Spectrum::settled(&mut g);
        let shapes = h.frame(&mut g, vec![]);
        band_colors(&spectrum_bands(&shapes).1)
    };
    let part = track_of(arc);
    let whole = track_of(Gradient { hue_span: FULL_TURN, ..arc });
    assert_eq!(part.len(), whole.len(), "the two bars drew different columns");
    let dim = (UNCLAIMED_ALPHA * 255.0) as u8;
    // Past the handle first, and the two halves in separate passes: this is
    // the claim, and it is the one a track drawn out of the gradient misses
    // by a mile, where the lit half below it would miss by a byte or two at
    // the left edge — the two arcs starting on the same color — and report
    // a difference no reader could tell from rounding.
    let mut held = 0;
    for (i, (drawn, lit)) in part.iter().zip(&whole).enumerate() {
        if drawn.a() == 255 {
            continue;
        }
        held += 1;
        assert_eq!(drawn.a(), dim, "column {i} is past the handle at some other strength");
        // Premultiplied, so the stored bytes are the lit ones scaled by the
        // alpha. Within a byte or two of it: the color is quantized once on
        // the way out of the table and again by the multiply.
        for (channel, (d, l)) in
            [("r", (drawn.r(), lit.r())), ("g", (drawn.g(), lit.g())), ("b", (drawn.b(), lit.b()))]
        {
            let want = f32::from(l) * UNCLAIMED_ALPHA;
            assert!(
                (f32::from(d) - want).abs() <= 2.0,
                "column {i}'s {channel} is {d}, not the {want} a held-back {l} would be",
            );
        }
    }
    assert!(held > 0, "the arc claimed the whole track, so nothing was compared");
    // And the stretch both arcs claim is one color at one strength, the
    // circle not caring how far round it the gradient reaches.
    for (i, (drawn, lit)) in part.iter().zip(&whole).enumerate() {
        if drawn.a() == 255 {
            assert_eq!(drawn, lit, "column {i} is inside both arcs and must be one color");
        }
    }
    // A span of nothing claims nothing, which is what the `claimed > 0.0`
    // half of the guard is for and the only thing left holding it up: it
    // stopped a divide by zero while the claimed stretch was the ramp
    // squeezed into it, and now it stops `p <= claimed` from lighting the
    // one column at `p == 0`. A lit column at the left edge of a track
    // claiming nothing is a picture that says the gradient reaches the
    // first hue, and a grey ramp is one drag of the Saturation bar away.
    let nothing = track_of(Gradient { hue_span: 0.0, ..arc });
    let lit: Vec<usize> =
        nothing.iter().enumerate().filter(|(_, c)| c.a() != dim).map(|(i, _)| i).collect();
    assert!(lit.is_empty(), "a span of nothing lit columns {lit:?} of {}", nothing.len());
}

/// Both bands fade out at their edges, the way epaint fades the rounded
/// rect every bar beside them fills with.
///
/// A mesh arrives at the tessellator already triangulated, so nothing there
/// softens it: without the ring [`gradient_strip`] builds, the two bands are
/// the only shapes in a settings pane with a hard edge, and a picture with a
/// stair along its top is what a reader sees. Nothing else in the suite
/// would notice, because every other reading goes through [`band_columns`],
/// which averages the ring away on purpose so what it reports is the
/// geometry the caller asked for.
///
/// Four claims, and they fail apart. The ring stands OUTSIDE the band on
/// all four sides, which is what catches an offset that ignores which way
/// its edge faces and pushes every column straight up instead. A triangle
/// is DRAWN across each of the two ends, which is a separate failure and
/// the quieter one: the ring's vertices are emitted per column either way,
/// so an end left untriangulated keeps every other reading here — the
/// bounds included, `calc_bounds` reading vertices and not triangles —
/// exactly as it is, and hands back the hard edge this all exists to
/// remove. The colors stand inside the band, which is what keeps it the
/// size of the well beneath it rather than half a pixel fatter all round.
/// And along the straight run the two are a whole feather apart and square
/// to the edge, which is the fade itself and not just a ring of something
/// drawn near it.
#[test]
fn both_colour_bands_fade_out_at_their_edges() {
    let mut g = ViewConfig::default().pitch_gradient;
    let mut h = Spectrum::settled(&mut g);
    let shapes = h.frame(&mut g, vec![]);
    let (preview, circle) = spectrum_bands(&shapes);
    for (which, mesh) in [("preview", &preview), ("circle", &circle)] {
        fades_out_at_its_edges(which, mesh);
    }
}

/// A band lands on the pixel grid, wherever in a point the layout leaves
/// it.
///
/// epaint puts a rect there before it tessellates one
/// (`TessellationOptions::round_rects_to_pixels`), so the well under a band
/// and every bar fill beside it have their edges on whole physical pixels.
/// A mesh is not touched, and half a pixel of offset is the difference
/// between a fade covering one pixel and one smeared across two — which is
/// a blurred edge standing beside sharp ones, the same complaint as a hard
/// edge and a harder one to name.
///
/// The whole rest of the suite runs at one pixel per point on fixtures that
/// are already whole, so the rounding is a no-op in every one of them and
/// can be deleted with all of them staying green. Hence a context that puts
/// the band a quarter of a point down: half a physical pixel, the worst
/// offset there is and the one a snap has to move.
#[test]
fn a_band_lands_on_the_pixel_grid_the_bars_beside_it_do() {
    const PPP: f32 = 2.0;
    let ctx = crate::tests::probe::themed_at(PPP);
    let asked = std::cell::Cell::new(egui::Rect::NOTHING);
    let out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(300.0, 100.0),
            )),
            ..Default::default()
        },
        |ui| {
            ui.add_space(0.25);
            let preview = GradientPreview::reserve(ui);
            asked.set(preview.show(ui, &Gradient::default()).rect);
        },
    );
    // The scale has to have taken, or the rounding under test is being
    // asked for whole points and the fixture is back on the grid it is
    // meant to be off.
    assert_eq!(ctx.pixels_per_point(), PPP, "the context ignored the scale");
    let shapes: Vec<egui::Shape> = out.shapes.into_iter().map(|s| s.shape).collect();
    let band = band_bounds(bands(&shapes).first().expect("a preview paints one band"));
    let asked = asked.get();
    // And the row has to land off the grid, or a band snapped by doing
    // nothing passes this for the wrong reason.
    let fraction = |edge: f32| edge * PPP - (edge * PPP).round();
    assert!(
        fraction(asked.top()).abs() > 0.1,
        "the fixture put the preview at {asked:?}, already on the grid",
    );
    for (side, edge) in [
        ("top", band.top()),
        ("bottom", band.bottom()),
        ("left", band.left()),
        ("right", band.right()),
    ] {
        assert!(
            fraction(edge).abs() < 1e-4,
            "the band's {side} edge is at {edge}pt, {} of a pixel off the grid",
            fraction(edge),
        );
    }
    // Onto the NEAREST boundary, which is the whole of what the well under
    // a band does: a band that snapped somewhere else would be square with
    // the pixels and half a row from the layout.
    assert!(
        (band.top() - asked.top()).abs() <= 0.5 / PPP + 1e-4,
        "the band's top moved from {} to {}, further than the grid is apart",
        asked.top(),
        band.top(),
    );
}

/// Both bands are rounded by their own mesh, on the corner circle, and
/// sampled through the arc rather than chamfered across it.
///
/// Nothing else in the suite looks at a mesh, so without this the entire
/// rounding mechanism — [`corner_inset`], [`CORNER_SAMPLES`], the radius
/// handed to [`gradient_strip`] — could be deleted and every test would
/// stay green. What it holds is the reason the bands are drawn edge to edge
/// at all: a square mesh inside a rounded well needs a ring of well showing
/// round it to look rounded, and that ring is a border no other bar in a
/// settings pane wears.
///
/// Three claims, because they fail apart. Pinning the corner vertices to
/// the arc catches a chamfer and an inset that has stopped following the
/// circle, but not a corner drawn from its two endpoints alone — the
/// endpoints are ON the arc. The sample count catches that. And the
/// straight run catches a radius that has grown to swallow the band.
///
/// [`corner_inset`]: crate::widgets::mesh::corner_inset
/// [`CORNER_SAMPLES`]: crate::widgets::mesh::CORNER_SAMPLES
#[test]
fn both_colour_bands_are_rounded_by_their_own_mesh() {
    let mut g = ViewConfig::default().pitch_gradient;
    let mut h = Spectrum::settled(&mut g);
    let shapes = h.frame(&mut g, vec![]);
    let (preview, circle) = spectrum_bands(&shapes);
    let radius = f32::from(bar_radius(1.0));
    for (which, mesh) in [("preview", &preview), ("circle", &circle)] {
        // How round the 14pt preview's ends are is a shape, not a limit —
        // what the radius may not do is eat the band's LENGTH, which the
        // straight-run count below is what catches.
        let box_ = band_bounds(mesh);
        let (mut near, mut far, mut full_height) = (0, 0, 0);
        for (top, bottom, _) in band_columns(mesh) {
            assert!((top.x - bottom.x).abs() < 1e-3, "{which}: a column is not vertical");
            let from_end = (top.x - box_.left()).min(box_.right() - top.x);
            if from_end >= radius - 1e-3 {
                full_height += 1;
                assert!(
                    (top.y - box_.top()).abs() < 1e-3 && (bottom.y - box_.bottom()).abs() < 1e-3,
                    "{which}: a column {from_end} from the end, past the corner, is pinched",
                );
                continue;
            }
            let cx = if top.x - box_.left() < radius {
                near += 1;
                box_.left() + radius
            } else {
                far += 1;
                box_.right() - radius
            };
            for (y, cy) in [(top.y, box_.top() + radius), (bottom.y, box_.bottom() - radius)] {
                let reach = ((top.x - cx).powi(2) + (y - cy).powi(2)).sqrt();
                assert!(
                    (reach - radius).abs() < 0.05,
                    "{which}: a corner vertex sits {reach} from the arc's centre, not {radius}",
                );
            }
        }
        // Each corner counted on its own, against a flat four rather than
        // against anything derived from [`CORNER_SAMPLES`]. A floor read
        // off the constant it is meant to pin goes to zero with it and
        // passes on a chamfer; four is the claim itself — fewer than four
        // columns through a quarter turn reads as steps at any radius this
        // control uses.
        for (end, count) in [("near", near), ("far", far)] {
            assert!(
                count >= 4,
                "{which}: {count} columns through the {end} corner — a chamfer, not an arc",
            );
        }
        assert!(full_height > 0, "{which}: the radius swallowed the whole band");
    }
}

/// A column with no room for both gives the row to the flip button, and
/// gives it the row rather than a sliver of one.
///
/// The branch is a deliberate choice — `spectrum_track_width` floors at
/// zero, so past that point the track stops shrinking and the button stops
/// moving — and no sweep reaches it: `no_settings_pane_overruns_a_narrow_column`
/// bottoms out at the settings floor and this needs 20. Which leaves the arithmetic
/// under it unexercised, and it is the arithmetic most likely to put the
/// button's own left edge past the right edge of the row.
#[test]
fn a_column_too_narrow_for_both_gives_the_row_to_the_button() {
    for column in [FLIP_W + FLIP_GAP + 30.0, FLIP_W + FLIP_GAP, FLIP_W, 6.0, 1.0] {
        let mut g = ViewConfig::default().pitch_gradient;
        let seen = std::cell::Cell::new(egui::Rect::NOTHING);
        let out = painted_in(egui::vec2(column, 80.0), |ui| {
            seen.set(SpectrumBar::new(&mut g, Gradient::default()).show(ui).rect)
        });
        let track = seen.get();
        let shapes: Vec<egui::Shape> = out.into_iter().map(|s| s.shape).collect();
        // The button is the one thing painted in the theme's resting widget
        // fill; the well under the track is `well()` and the handle is
        // `text()`. Finding none would mean the paint no longer reads that
        // fill, which is its own failure.
        let buttons: Vec<egui::Rect> = filled_rects(&shapes)
            .into_iter()
            .filter(|(_, fill)| *fill == theme::widget())
            .map(|(r, _)| r)
            .collect();
        assert_eq!(buttons.len(), 1, "at {column}pt the bar drew {buttons:?} buttons");
        let button = buttons[0];

        assert!(track.width() >= 0.0, "at {column}pt the track came out {track:?}");
        assert!(
            button.left() >= track.right() - 0.01,
            "at {column}pt the button {button:?} runs into the track {track:?}",
        );
        // Below the threshold the track is gone and the button holds the
        // row: everything except the gap it would have kept clear.
        if column <= FLIP_W + FLIP_GAP {
            assert_eq!(track.width(), 0.0, "at {column}pt the track kept {}", track.width());
            assert!(
                button.width() >= button.right() - track.left() - FLIP_GAP - 0.01,
                "at {column}pt the button shrank to {} of a {}pt row",
                button.width(),
                button.right() - track.left(),
            );
        } else {
            assert!(track.width() > 0.0, "at {column}pt the track vanished early");
            assert!(
                (button.width() - FLIP_W).abs() < 0.01,
                "at {column}pt the button is {}, not its full {FLIP_W}",
                button.width(),
            );
        }
    }
}

/// Where the bar drew its handle.
fn spectrum_handle_x(shapes: &[egui::Shape]) -> f32 {
    let hs = handles(shapes);
    assert_eq!(hs.len(), 1, "a spectrum bar draws one handle, not {hs:?}");
    hs[0].center().x
}

/// The hue the bar paints at `p`, worked out the way the bar lays a circle
/// on a track: cut at the arc's own start, one whole turn across.
fn hue_under(g: Gradient, track: egui::Rect, p: egui::Pos2) -> f32 {
    let across = ((p.x - track.left()) / track.width()).clamp(0.0, 1.0);
    let winding = if g.hue_span < 0.0 { -1.0 } else { 1.0 };
    (g.hue_start + across * FULL_TURN * winding).rem_euclid(FULL_TURN)
}

/// The track stands for the span knob's whole travel, which holds only
/// while the two numbers are the same one.
#[test]
fn the_spectrum_bar_track_is_a_whole_turn_of_the_span_knob() {
    assert_eq!(
        FULL_TURN,
        Gradient::MAX_HUE_SPAN,
        "the track draws one turn while the span reaches {}, so the handle \
             parks at the right edge with the value still growing",
        Gradient::MAX_HUE_SPAN,
    );
}

/// The handle stands at the fraction of the turn the arc claims, and says
/// so in the same picture.
#[test]
fn the_spectrum_handle_stands_where_its_readout_says() {
    for span in [0.0f32, 45.0, 190.0, 360.0, -190.0, -360.0] {
        let mut g = Gradient { hue_span: span, ..Gradient::default() };
        let mut h = Spectrum::settled(&mut g);
        let shapes = h.frame(&mut g, vec![]);
        let track = h.track();
        let want = track.left() + track.width() * (span / FULL_TURN).abs();
        let drawn = spectrum_handle_x(&shapes);
        assert!(
            (drawn - want).abs() < 0.51,
            "a span of {span} put the handle at {drawn} rather than {want}",
        );
        assert_eq!(spectrum_readout(&shapes), format!("{span:+.0}°"));
    }

    // Both limits are places the handle can STAND rather than edges it
    // merges into, which is the whole of what the inset buys: at neither
    // one does it hang off the bar or disappear under the rounding.
    let mut nothing = Gradient { hue_span: 0.0, ..Gradient::default() };
    let mut h = Spectrum::settled(&mut nothing);
    let shapes = h.frame(&mut nothing, vec![]);
    assert!(handles(&shapes)[0].left() >= h.rect.left(), "a zero span hangs the handle off");
    let mut whole = Gradient { hue_span: 360.0, ..Gradient::default() };
    let mut h = Spectrum::settled(&mut whole);
    let shapes = h.frame(&mut whole, vec![]);
    assert!(handles(&shapes)[0].right() <= h.rect.right(), "a whole turn hangs the handle off");
}

/// The name rides the circle — which runs the track end to end — drawn in
/// the ground color rather than the theme's text, and the readout never
/// reaches it.
///
/// Three claims that fail apart. The circle runs the WHOLE track, so the
/// arc still reads from the bar's own left edge and nothing has quietly
/// taken a gutter back. The name is DARK, which is what makes it legible
/// where it stands — see [`span_name_color`]; a name drawn in the color the
/// readout uses sits at about 2.5:1 on the claimed circle, and no test of
/// position can see that. And the readout keeps off the name at every span
/// — swept to a whole turn, where it crosses to the near side of the
/// handle, which is the one arrangement that aims it at the name.
#[test]
fn the_name_rides_the_circle_and_the_number_keeps_off_it() {
    // Swept across the column as well as the arc: the reserve the name is
    // elided against, the stretch it leaves the readout and the clamp that
    // holds the readout there are all arithmetic against the WIDTH, and one
    // width exercises one answer.
    //
    // Past the dock's own 173pt floor on purpose. Above it the name is
    // drawn whole and the reserve never binds — a sweep that stopped there
    // passes with the reserve deleted, which is a test measuring nothing.
    // The pane sweeps reach this bar at 80pt
    // (`every_bar_in_a_settings_pane_is_the_width_of_the_pane`), and what
    // has to hold down there is not the name but the arrangement: elided,
    // and still clear of the number.
    const SPANS: [f32; 6] = [0.0, 45.0, 190.0, 330.0, 360.0, -360.0];
    /// The narrowest column the dock gives a pane, and so the width at
    /// which the name is still owed in full.
    const DOCK_FLOOR: f32 = 173.0;
    for (width, span) in [300.0f32, 240.0, 200.0, DOCK_FLOOR, 140.0, 110.0, 80.0]
        .into_iter()
        .flat_map(|width| SPANS.map(move |span| (width, span)))
    {
        let aimed = format!("a span of {span} in a {width}pt column");
        let mut g = Gradient { hue_span: span, ..Gradient::default() };
        let mut h = Spectrum::settled_at(&mut g, width);
        let shapes = h.frame(&mut g, vec![]);
        let texts = spectrum_texts(&shapes);
        assert_eq!(texts.len(), 2, "{aimed} drew {texts:?} rather than a name and a readout",);
        let (name, written, name_color) = texts[0].clone();
        let (readout, _, readout_color) = texts[1].clone();
        // The name in full at every width a reader can drag the column to,
        // and below that a truthful elision of it — the tail eaten, never
        // the head, so what survives still names the bar. Where the elision
        // STARTS is left to the font: the claim is that it cannot start
        // above the floor.
        if written != SPAN_LABEL {
            let kept = written.trim_end_matches('\u{2026}');
            assert!(
                width < DOCK_FLOOR && written.ends_with('\u{2026}') && SPAN_LABEL.starts_with(kept),
                "{aimed}: the bar wrote {written:?} where {SPAN_LABEL:?} was owed",
            );
        }
        // The circle end to end under it, no gutter anywhere.
        let circle = band_bounds(&spectrum_bands(&shapes).1);
        assert!(
            (circle.left() - h.rect.left()).abs() < 0.01
                && (circle.right() - h.rect.right()).abs() < 0.01,
            "{aimed}: the circle covers {circle:?} of a track that runs {:?}",
            h.rect,
        );
        assert!(
            name.left() < circle.right() && name.right() > circle.left(),
            "{aimed}: the name at {name:?} is not on the circle {circle:?} at all",
        );
        // Dark, and darker than the number beside it: the two stand on
        // different halves of the track and take opposite ends of the
        // palette for it. Asked as a comparison rather than against one
        // named color, so a re-skin moves both together.
        let luma = |c: Color32| f32::from(c.r()) + f32::from(c.g()) + f32::from(c.b());
        assert!(
            luma(name_color) < luma(readout_color),
            "{aimed}: the name {name_color:?} is no darker than the readout {readout_color:?}",
        );
        assert!(
            readout.left() >= name.right(),
            "{aimed} put the readout at {readout:?}, back into the name {name:?}",
        );
        assert!(
            readout.right() <= h.rect.right() + 0.01,
            "{aimed} ran the readout past the track's end: {readout:?}",
        );
    }
}

/// The handle knocks out a run it stands in, as every other bar's thumbs
/// do, rather than covering it — and a short arc stands it in the name.
///
/// The arc is derived from where the name was measured, so the handle lands
/// on the name's middle whatever the font makes of it.
#[test]
fn the_spectrum_handle_knocks_out_the_name_it_stands_in() {
    let home = ViewConfig::default().pitch_gradient;
    let paint = |span: f32| {
        let mut g = Gradient { hue_span: span, ..Gradient::default() };
        let track = std::cell::Cell::new(egui::Rect::NOTHING);
        let shapes = painted(300.0, |ui| track.set(SpectrumBar::new(&mut g, home).show(ui).rect));
        (shapes, track.get())
    };
    let (shapes, track) = paint(180.0);
    let flat: Vec<_> = shapes.into_iter().map(|s| s.shape).collect();
    let (name, written) = text_boxes(&flat).into_iter().next().expect("the bar names itself");
    assert_eq!(written, SPAN_LABEL);
    let travel = track.shrink2(Vec2::new(HANDLE_INSET, 0.0));
    let span = (name.center().x - travel.left()) / travel.width() * FULL_TURN;

    let (shapes, _) = paint(span);
    let flat: Vec<_> = shapes.iter().map(|s| s.shape.clone()).collect();
    let grip = handles(&flat)[0];
    assert!(grip.intersects(name), "an arc of {span} did not stand the handle in the name");
    let knocked = knockouts(&shapes);
    assert!(
        knocked.iter().any(|(clip, at, text, colour)| {
            *clip == grip && *at == name && text == SPAN_LABEL && *colour == Some(theme::panel())
        }),
        "the handle stands in the name and knocks out {knocked:?} instead",
    );
}

/// The frame that moves the arc is the frame that DRAWS it moved.
///
/// A bar that snapshots its value before the interaction block and paints
/// from the snapshot is right in the end and wrong the whole way through:
/// the handle, the arc, the strip and the readout all show the previous
/// frame's value for the length of the gesture. Only a live drag catches
/// it — a settled bar draws the same picture either way, which is why every
/// other test here would pass against the lag.
#[test]
fn a_spectrum_drag_draws_the_arc_it_just_set() {
    let mut g = Gradient { hue_span: 90.0, ..Gradient::default() };
    let mut h = Spectrum::settled(&mut g);
    let (from, to) = (h.at_span(90.0), h.at_span(270.0));
    let shapes = h.drag(&mut g, from, to);
    assert!(
        (g.hue_span - 270.0).abs() < 2.0,
        "the drag left the span at {} rather than near 270",
        g.hue_span,
    );
    let drawn = spectrum_handle_x(&shapes);
    assert!(
        (drawn - to.x).abs() < 1.0,
        "the pointer is at {} and the handle was drawn at {drawn}, a frame behind",
        to.x,
    );
    assert_eq!(
        spectrum_readout(&shapes),
        format!("{:+.0}°", g.hue_span),
        "the readout names a span other than the one the drag just set",
    );
}

/// The preview draws the arc the drag just SET, exactly as the bar under it
/// does.
///
/// A settings pane draws top-down, so a picture drawn WHERE IT STANDS is
/// drawn from the value the bars below it were handed rather than the one
/// they just wrote — `harmonigraph_scene::color`'s `LUT_SLOTS` counts that
/// very frame ("everything above the bar in a frame reads the value the bar
/// wrote LAST frame"), and it is the reason the bar itself re-reads. The
/// preview is above all three bars, so it is the group's one piece that can
/// spend a whole gesture a frame behind the control being dragged, and the
/// only fix is to take its space first and paint it last.
///
/// Measured against the picture the SAME gradient draws with nothing in
/// flight, rather than against a color written out here: the claim is that
/// the frame is not stale, and the settled bar is what "not stale" means.
#[test]
fn a_spectrum_drag_draws_the_preview_it_just_set() {
    let mut g = Gradient { hue_span: 90.0, ..Gradient::default() };
    let mut h = Spectrum::settled(&mut g);
    let shapes = h.drag(&mut g, h.at_span(90.0), h.at_span(270.0));
    let live = band_colors(&spectrum_bands(&shapes).0);

    let mut landed = g;
    let mut settled = Spectrum::settled(&mut landed);
    let shapes = settled.frame(&mut landed, vec![]);
    let want = band_colors(&spectrum_bands(&shapes).0);
    assert_eq!(
        live, want,
        "the preview drew a gradient other than the one the drag left at \
             {landed:?} — a frame behind the bar it stands over",
    );
}

/// The flip button reverses the arc, in the frame it is clicked.
///
/// The value half is what says the button is wired to the gradient's own
/// flip and not to a second spelling of it here; the picture half is the
/// same claim `a_spectrum_drag_draws_the_arc_it_just_set` makes about the
/// handle, and it fails the same way — a click handled after the snapshot
/// the paint reads leaves the bar a frame behind the button.
#[test]
fn the_flip_button_reads_the_arc_backwards() {
    let mut g = Gradient { hue_start: 260.0, hue_span: 190.0, ..Gradient::default() };
    let before = g.sanitized();
    let mut h = Spectrum::settled(&mut g);
    let shapes = h.click(&mut g, h.on_flip());
    assert_eq!(g, before.flipped(), "the click left the arc at {g:?}");
    assert_eq!(
        spectrum_readout(&shapes),
        format!("{:+.0}°", g.hue_span),
        "the readout names a direction other than the one the click just set",
    );
}

/// The button is a button and the track beside it is a track: a sideways
/// drag begun on the button turns nothing.
///
/// They share a row, and what keeps them apart is that each is interacted
/// with over its OWN rectangle — which works HERE, where the strip below
/// needs a position check as well, because the button senses clicks and not
/// drags: a press on it produces no drag hit for the track to inherit, at
/// any distance. Sensed together, the track takes that press and the only
/// thing left standing between it and the rotate branch is the same
/// `bar::drag` check the strip is declined by — a position test doing the
/// work a rectangle does for free, and doing it a few frames late.
#[test]
fn a_drag_begun_on_the_flip_button_turns_nothing() {
    let before = Gradient { hue_start: 0.0, hue_span: 90.0, ..Gradient::default() };
    let mut g = before;
    let mut h = Spectrum::settled(&mut g);
    let to = h.at_span(120.0);
    h.drag(&mut g, h.on_flip(), to);
    assert_eq!(g, before, "a drag begun on the flip button moved the arc to {g:?}");

    // The same drag from just inside the track does move it, so the harness
    // is delivering something the widget can act on.
    let mut g = before;
    let mut h = Spectrum::settled(&mut g);
    let from = egui::pos2(h.track().right(), h.track().center().y);
    h.drag(&mut g, from, to);
    assert_ne!(g, before, "the same drag on the track moved nothing, so this proves nothing");
}

/// The preview is a picture, and a press anywhere on it — including hard
/// against the track below — starts nothing.
///
/// Swept from the preview's bottom edge up, because the row spacing is not
/// by itself a barrier: egui's hit test collects every widget within
/// `interact_radius` of the pointer and, when the press hits nothing that
/// takes drags, hands it to the nearest one. At the default radius of 5
/// that reaches five points past the track's top edge — past the 4pt of
/// spacing between them — so the preview's own bottom is inside the track's
/// reach and only the position check in `show` keeps it out. A probe at the
/// preview's middle alone would sit clear of the one region where this can
/// fail.
#[test]
fn the_preview_is_a_picture_and_not_a_control() {
    // The arc the harness hands the bar as its home, so the double-click
    // half below reads the reset rather than a constant that merely used to
    // match it.
    let home = ViewConfig::default().pitch_gradient;
    let dialled = Gradient { hue_start: 12.0, hue_span: 33.0, ..home };

    for up in [0.0f32, 0.05, 0.25, 0.5, 1.0] {
        // A drag begun on the preview and run down into the track.
        let mut g = home;
        let mut h = Spectrum::settled(&mut g);
        let from = h.on_preview(0.2, up);
        let to = egui::pos2(h.on_preview(0.8, up).x, h.track().center().y);
        h.drag(&mut g, from, to);
        assert_eq!(g, home, "a drag begun {up} up the preview turned the circle");

        // The same gesture one row lower does move it, so the harness is
        // delivering something the widget can act on.
        let mut g = home;
        let mut h = Spectrum::settled(&mut g);
        let from = egui::pos2(from.x.max(h.track().left()), h.track().center().y);
        h.drag(&mut g, from, to);
        assert_ne!(g, home, "the same drag on the track moved nothing, so this proves nothing");

        // And a double-click on the preview does not reset the arc. A fresh
        // bar for each pair: run back to back on one, the second lands
        // inside the first's double-click window and arrives as the third
        // click of a sequence, which is a different gesture.
        let mut g = dialled;
        let mut h = Spectrum::settled(&mut g);
        let at = h.on_preview(0.5, up);
        h.double_click(&mut g, at);
        assert_eq!(g, dialled, "a double-click {up} up the preview reset the arc");
    }
}

/// A double-click on any of the three bars goes home to the gradient the bar
/// was HANDED, which is what lets one set of bars serve two gradients.
///
/// Every caller names its home ([`SpectrumBar::new`] says why no default
/// could serve), so what is left to hold is that the reset branch reads the
/// one it was given. The heatmap's is the fixture because it is nothing like
/// the type's own default — the one value a bar that ignored its home could
/// plausibly land on instead — and both halves are asserted: the gradient
/// that WAS reached, and that it is not that default. Without the second, a
/// bar that quietly ignored `home` would still pass whenever the two agreed.
#[test]
fn a_double_click_goes_home_to_the_gradient_the_bar_was_handed() {
    let home = crate::SpectrumConfig::default().spectrogram_gradient;
    let stray = Gradient::default();
    assert_ne!(
        (home.hue_start, home.hue_span),
        (stray.hue_start, stray.hue_span),
        "the home is the type's default, so this test cannot tell whether it was read",
    );

    let mut g = Gradient { hue_start: 12.0, hue_span: 33.0, ..home };
    let mut h = Spectrum::settled_with_home(&mut g, home);
    let at = h.track().center();
    h.double_click(&mut g, at);
    assert_eq!(
        (g.hue_start, g.hue_span),
        (home.hue_start, home.hue_span),
        "the reset ignored the home it was handed",
    );

    // And the pairs the two spread bars carry, which reset the same way.
    for spread in [Spread::Brightness, Spread::Chroma] {
        assert_ne!(
            spread.of(stray),
            spread.of(home.sanitized()),
            "{spread:?}: the home's pair is the type's default, so this cannot tell",
        );
        let dialled = holding(spread, spread.snapped((30.0 / spread.per_unit(), 0.0)));
        assert_ne!(
            spread.of(dialled),
            spread.of(home.sanitized()),
            "{spread:?}: the bar already holds the pair it would reset to",
        );
        assert_eq!(
            double_click_spread(spread, dialled, home),
            spread.of(home.sanitized()),
            "{spread:?}: the reset ignored the home it was handed",
        );
    }
}

/// A turn slides the circle under a fixed left edge, and the hue the
/// gesture took hold of stays under the pointer for the length of it.
///
/// The hue it took hold of is the one under the PRESS, so the turn is the
/// pointer's whole travel rather than what is left of it after egui has
/// spent six points deciding the press was a drag. Anchoring on the first
/// live frame instead leaves the circle behind the hand by that much for
/// the rest of the gesture, and nothing on screen ever catches it up.
#[test]
fn turning_the_spectrum_keeps_the_grabbed_hue_under_the_pointer() {
    let mut g = Gradient { hue_start: 0.0, hue_span: 90.0, ..Gradient::default() };
    let before = g;
    let mut h = Spectrum::settled(&mut g);
    // Well clear of the handle, which a quarter-turn arc stands at 0.25.
    let (from, to) = (h.at_span(300.0), h.at_span(200.0));
    h.drag(&mut g, from, to);
    let track = h.track();
    let held = hue_under(before, track, from);
    let now = hue_under(g, track, to);
    // Within a degree either side, the far side being the seam: two hues a
    // whisker apart across 0 are 359 apart by subtraction.
    let apart = (now - held).abs();
    assert!(
        !(1.0..=FULL_TURN - 1.0).contains(&apart),
        "the hue under the pointer went from {held} to {now} during the turn",
    );
    assert_ne!(g.hue_start, before.hue_start, "the turn moved nothing");
    assert_eq!(g.hue_span, before.hue_span, "a turn changed how wide the arc is");
}

/// A press within the handle's reach drags the HANDLE, whichever way the
/// drag then runs.
///
/// The reach is what the cursor promises — a `ResizeHorizontal` under the
/// pointer says a press here resizes the arc — and the promise is made at
/// the moment of the press. egui calls a press a drag only once it has left
/// a six-point threshold, so a gesture that runs OUTWARD from a press near
/// the edge of the reach is already clear of it by the first live frame,
/// and settling the question there turns the circle under a hand that aimed
/// at the handle. Swept across the reach and both ways, because the middle
/// of it survives either rule.
#[test]
fn a_press_within_the_handles_reach_drags_the_handle_whichever_way_it_runs() {
    let before = Gradient { hue_start: 40.0, hue_span: 180.0, ..Gradient::default() };
    for offset in [-GRAB_PX, -GRAB_PX * 0.5, 0.0, GRAB_PX * 0.5, GRAB_PX] {
        for run in [-40.0f32, 40.0] {
            let mut g = before;
            let mut h = Spectrum::settled(&mut g);
            let from = h.at_span(before.hue_span) + Vec2::new(offset, 0.0);
            h.drag(&mut g, from, from + Vec2::new(run, 0.0));
            let aimed = format!("pressed {offset} from the handle and dragged {run}");
            assert_eq!(
                g.hue_start, before.hue_start,
                "{aimed}: the circle turned under a press on the handle",
            );
            assert_ne!(g.hue_span, before.hue_span, "{aimed}: the arc did not move");
        }
    }
}

/// And that reach is the same length at every chrome scale, because it is
/// a reach rather than a drawn thing — [`GRAB_PX`]'s own contract, which
/// the `RangeBar` and the `SpreadBar` keep by reading the constant
/// unmultiplied.
///
/// A bar dialled smaller is already a smaller target; shrinking what it
/// answers to as well makes the pane's smallest handle the one that is
/// also hardest to catch. The sweep runs the full [`theme::UI_SCALE_RANGE`]
/// because the two ends fail in opposite directions — a scaled reach is
/// short of the promise at 0.7 and past it at 1.5 — and every other
/// harness in this file draws on a bare context, which reads 1.0.
#[test]
fn the_handles_reach_is_the_same_at_every_chrome_scale() {
    // Inside GRAB_PX, and outside what the smallest scale would leave of
    // it were the reach scaled.
    let offset = GRAB_PX * (1.0 + *theme::UI_SCALE_RANGE.start()) * 0.5;
    for scale in [1.0f32, 0.75, 1.6] {
        let before = Gradient { hue_start: 40.0, hue_span: 180.0, ..Gradient::default() };
        let mut g = before;
        let mut h = Spectrum::settled_scaled(&mut g, 300.0, scale);
        let from = h.at_span(before.hue_span) + Vec2::new(offset, 0.0);
        h.drag(&mut g, from, from + Vec2::new(40.0, 0.0));
        let aimed = format!("at chrome scale {scale}, pressed {offset} from the handle");
        assert_eq!(
            g.hue_start, before.hue_start,
            "{aimed}: the circle turned under a press on the handle",
        );
        assert_ne!(g.hue_span, before.hue_span, "{aimed}: the arc did not move");
    }
}

/// And a press CLEAR of that reach turns the circle, however the drag then
/// runs — the other half of the same rule, which without this is satisfied
/// by a bar whose handle has swallowed the whole track.
#[test]
fn a_press_clear_of_the_handle_turns_the_circle_whichever_way_it_runs() {
    let before = Gradient { hue_start: 40.0, hue_span: 180.0, ..Gradient::default() };
    for offset in [-GRAB_PX * 3.0, GRAB_PX * 3.0] {
        for run in [-20.0f32, 20.0] {
            let mut g = before;
            let mut h = Spectrum::settled(&mut g);
            let from = h.at_span(before.hue_span) + Vec2::new(offset, 0.0);
            h.drag(&mut g, from, from + Vec2::new(run, 0.0));
            let aimed = format!("pressed {offset} from the handle and dragged {run}");
            assert_ne!(g.hue_start, before.hue_start, "{aimed}: the circle did not turn");
            assert_eq!(g.hue_span, before.hue_span, "{aimed}: a turn resized the arc");
        }
    }
}

/// A flip lands on the same arc read backwards — the promise the Flip
/// button beside the bar makes, and the reason the bar draws the same
/// stretch of color either way round.
#[test]
fn a_flip_is_the_same_arc_read_backwards() {
    for (start, span) in [(260.0f32, 190.0f32), (0.0, 360.0), (95.0, -45.0), (12.0, 0.0)] {
        let before =
            Gradient { hue_start: start, hue_span: span, ..Gradient::default() }.sanitized();
        let after = before.flipped();
        let ends = |g: Gradient| (g.lightness_and_hue(0.0).1, g.lightness_and_hue(1.0).1);
        let (low, high) = ends(before);
        let (flipped_low, flipped_high) = ends(after);
        assert!(
            (flipped_low - high).abs() < 1e-3 && (flipped_high - low).abs() < 1e-3,
            "{start}/{span}: {low}..{high} came back as {flipped_low}..{flipped_high}",
        );
        assert_eq!(
            after.hue_span.abs(),
            before.hue_span.abs(),
            "{start}/{span}: a flip changed how much of the circle the arc claims",
        );
        assert_eq!(after.flipped(), before, "{start}/{span}: flipping twice is not the identity",);
    }
}

/// A span of nothing is written with the one sign that reads as no
/// direction.
///
/// `-0.0` is the sign that lies: it is not `< 0.0`, so everything asking
/// which way the arc runs takes it for rightward, while `{:+}` prints it as
/// running left — a bar that says one direction and behaves as the other.
/// Flipping a zero span makes one, and so does dragging a flipped arc down
/// to nothing.
#[test]
fn a_span_of_nothing_reads_out_with_no_direction() {
    let flipped = Gradient { hue_span: 0.0, ..Gradient::default() }.flipped();
    assert!(
        flipped.hue_span.is_sign_positive(),
        "flipping a span of nothing left it at {}",
        flipped.hue_span,
    );

    let mut g = Gradient { hue_span: -120.0, ..Gradient::default() };
    let mut h = Spectrum::settled(&mut g);
    let from = h.at_span(-120.0);
    let to = egui::pos2(h.track().left() - 40.0, h.track().center().y);
    let shapes = h.drag(&mut g, from, to);
    assert_eq!(g.hue_span, 0.0, "the drag left the span at {}", g.hue_span);
    assert!(
        g.sanitized().hue_span.is_sign_positive(),
        "a flipped arc dragged to nothing kept a direction it cannot have",
    );
    assert_eq!(spectrum_readout(&shapes), "+0°", "a span of nothing read out as running left",);
}

/// A reach in value units wide enough to reach a handle from well away
/// from it, standing in for the `GRAB_PX` a real bar converts.
const NEAR: f32 = 4.0;

/// A dragged end goes where the pointer is and leaves its partner exactly
/// where it stood. Which is the whole of a two-ended bar, and it is what
/// makes the readout's two numbers each settable on their own.
#[test]
fn a_dragged_end_moves_itself_and_leaves_its_partner() {
    // A 40-point ramp about 50 stands its ends at 30 and 70.
    let ramp = (50.0f32, 40.0);
    assert_eq!(
        SpreadGrab::High.apply(90.0, ramp, L_STAR_AXIS),
        (60.0, 60.0),
        "the high end to 90 leaves 30..90, which is a middle of 60 and a ramp of 60",
    );
    assert_eq!(
        SpreadGrab::Low.apply(10.0, ramp, L_STAR_AXIS),
        (40.0, 60.0),
        "and the low end to 10 leaves 10..70",
    );
}

/// Past its PARTNER an end inverts the ramp rather than stopping against
/// it, which is the whole of how the bright end reaches the bottom of the
/// pitch range. The gesture keeps hold of the end it grabbed, so the two
/// trade sides and the sign follows the pointer through zero without a
/// discontinuity.
///
/// A [`RangeBar`] refuses exactly this — see
/// `a_dragged_end_stops_at_the_minimum_span` — and is right to: its ends
/// bound a pitch axis, which inverted maps every pitch on it backwards.
///
/// [`RangeBar`]: crate::widgets::range::RangeBar
#[test]
fn an_end_dragged_past_its_partner_inverts_the_ramp() {
    // The low end walking up through its partner at 70.
    let walk: Vec<(f32, f32)> = [50.0, 70.0, 90.0]
        .into_iter()
        .map(|v| SpreadGrab::Low.apply(v, (50.0, 40.0), L_STAR_AXIS))
        .collect();
    assert_eq!(
        walk,
        vec![(60.0, 20.0), (70.0, 0.0), (80.0, -20.0)],
        "the low end crossing its partner went {walk:?}",
    );
}

/// An end stops at the axis, and nowhere short of it: black and white are
/// both settings, and the ramp that reaches from one to the other is the
/// widest the bar has.
#[test]
fn a_dragged_end_stops_at_the_axis() {
    let ramp = (50.0f32, 40.0);
    assert_eq!(SpreadGrab::High.apply(500.0, ramp, L_STAR_AXIS), (65.0, 70.0), "at white");
    assert_eq!(SpreadGrab::Low.apply(-500.0, ramp, L_STAR_AXIS), (35.0, 70.0), "and at black");
    // Both ends out: the whole axis, which is the steepest ramp there is.
    let full = SpreadGrab::Low.apply(-500.0, (50.0, 100.0), L_STAR_AXIS);
    assert_eq!(full, (50.0, 100.0), "black to white is the widest ramp the axis holds");
}

/// Sliding carries the ramp along at the width the gesture began with, so
/// making the picture brighter is one gesture and does not quietly restyle
/// how much brightness the pitch range spends.
#[test]
fn a_slid_ramp_keeps_its_grabbed_width() {
    let grab = SpreadGrab::Middle { offset: 4.0, spread: 40.0 };
    assert_eq!(grab.apply(64.0, (50.0, 40.0), L_STAR_AXIS), (60.0, 40.0));
    // And a negative ramp stays negative: its SIGN is not the slide's to
    // change, and a slide that flipped the picture would be a surprise
    // nothing on the bar announced.
    let inverted = SpreadGrab::Middle { offset: 4.0, spread: -40.0 };
    assert_eq!(inverted.apply(64.0, (50.0, -40.0), L_STAR_AXIS), (60.0, -40.0));
}

/// Slid into an end the ramp squishes rather than the drag jamming: the
/// leading end pins to the wall and the trailing one carries on with the
/// pointer, down to nothing. And it springs back out on the way home,
/// because it reads the width its own gesture began at and never the
/// squished pair it just wrote — a [`Grab::Span`]'s bargain, both halves.
///
/// [`Grab::Span`]: crate::widgets::range::Grab::Span
#[test]
fn a_slid_ramp_squishes_against_the_end_it_meets() {
    // Grabbed dead centre of a 40-point ramp at L* 50, so 30..70.
    let grab = SpreadGrab::Middle { offset: 0.0, spread: 40.0 };
    let start = (50.0f32, 40.0);
    assert_eq!(
        grab.apply(90.0, start, L_STAR_AXIS),
        (85.0, 30.0),
        "the bright end pins at white and the dark one carries on to 70",
    );
    assert_eq!(grab.apply(120.0, start, L_STAR_AXIS), (100.0, 0.0), "squishing to nothing");
    // Already squished, same pointer: the answer must not creep further.
    assert_eq!(grab.apply(90.0, (85.0, 30.0), L_STAR_AXIS), (85.0, 30.0));
    // And back down the axis, the ramp the gesture started with returns.
    assert_eq!(grab.apply(50.0, (85.0, 30.0), L_STAR_AXIS), start);
}

/// At a FLAT ramp all three grabs stand on the same point, and the middle
/// is the one that has to win: it is the only thing left to drag, and a
/// bar whose brightness could not be moved at exactly the isoluminant
/// setting would strand anyone who dialled their way into it.
///
/// Away from that point the ends take over, and WHICH end is the pointer's
/// own side — see `a_flat_ramp_opens_the_way_it_is_dragged` for what that
/// buys, which is a picture the right way round in either direction.
#[test]
fn the_middle_stays_grabbable_at_a_flat_ramp() {
    let flat = (50.0f32, 0.0);
    assert!(matches!(SpreadGrab::at(50.0, flat, NEAR), SpreadGrab::Middle { .. }));
    let out = SpreadGrab::at(20.0, flat, NEAR);
    assert!(matches!(out, SpreadGrab::Low), "a press out on the track took {out:?}");
    assert_eq!(
        out.apply(20.0, flat, L_STAR_AXIS),
        (35.0, 30.0),
        "and it opens the ramp dark-at-the-bottom",
    );
}

/// Opening a ramp out of a flat one runs the way it is DRAGGED, either
/// direction: up lifts the top of the pitch range, down darkens the bottom,
/// and both leave the picture the right way round.
///
/// A flat ramp is the one setting where nothing distinguishes the two ends
/// — they stand on the same point — so which one a press takes is a rule
/// rather than a measurement, and taking a FIXED one inverts the picture in
/// whichever direction it is not. At black or white that direction is the
/// only one there is: the axis runs one way from either, so a bar that
/// opened inverted on an up-drag would make an isoluminant black picture
/// impossible to open the right way round at all.
#[test]
fn a_flat_ramp_opens_the_way_it_is_dragged() {
    for (flat, to, want) in [
        ((50.0f32, 0.0f32), 70.0f32, (60.0f32, 20.0f32)),
        ((50.0, 0.0), 30.0, (40.0, 20.0)),
        // Parked on black, where up is the only way out.
        ((0.0, 0.0), 40.0, (20.0, 40.0)),
        // And on white.
        ((100.0, 0.0), 60.0, (80.0, 40.0)),
    ] {
        let grab = SpreadGrab::at(to, flat, NEAR);
        let got = grab.apply(to, flat, L_STAR_AXIS);
        assert_eq!(
            got, want,
            "flat at {} dragged to {to} gave a ramp of {}, and a negative one is the \
                 picture upside down",
            flat.0, got.1,
        );
    }
}

/// A wide ramp divides the bar the way a [`RangeBar`] does: a handle's
/// reach around each end, the whole inside between them, and the empty
/// track beyond falling to the nearer end. What that buys is that aiming at
/// a handle cannot land on the slide, which would move both ends instead of
/// the one aimed at.
///
/// [`RangeBar`]: crate::widgets::range::RangeBar
#[test]
fn a_wide_ramp_leaves_both_the_handles_and_the_slide_reachable() {
    let wide = (50.0f32, 80.0);
    assert!(matches!(SpreadGrab::at(10.0, wide, NEAR), SpreadGrab::Low), "on the low handle");
    assert!(matches!(SpreadGrab::at(90.0, wide, NEAR), SpreadGrab::High), "the high handle");
    assert!(matches!(SpreadGrab::at(50.0, wide, NEAR), SpreadGrab::Middle { .. }), "inside");
    assert!(matches!(SpreadGrab::at(2.0, wide, NEAR), SpreadGrab::Low), "off the end");
    // The reach is NEAR itself here, an 80-point ramp being far too wide
    // for the share to bite: 6 points inside the low end is a slide, 3 is
    // the handle.
    assert!(matches!(SpreadGrab::at(16.0, wide, NEAR), SpreadGrab::Middle { .. }));
    assert!(matches!(SpreadGrab::at(13.0, wide, NEAR), SpreadGrab::Low));
    // And inverted, where the low-pitch end stands on the RIGHT: the same
    // press takes the same pitch end, not the same side of the bar.
    let flipped = (50.0f32, -80.0);
    assert!(matches!(SpreadGrab::at(90.0, flipped, NEAR), SpreadGrab::Low), "still the low");
    assert!(matches!(SpreadGrab::at(10.0, flipped, NEAR), SpreadGrab::High));
}

/// Every pair the bar writes puts both ENDS on a whole readout unit inside
/// the axis, since the ends are what it reads out and a readout is worth
/// nothing once it is not the number the picture draws.
#[test]
fn the_pair_a_bar_writes_puts_both_ends_on_whole_readout_units() {
    let brightness = Spread::Brightness;
    // 43.4..83.8 rounds to 43..84, whose middle is a half.
    assert_eq!(brightness.snapped((63.6, 40.4)), (63.5, 41.0));
    // An odd ramp is exactly what snapping the PAIR could not keep honest:
    // 45 about 64 reaches 41.5 and 86.5, which round to 42 and 87 — the
    // ramp survives at 45 and the middle takes the half instead, which is
    // the right way round, since the middle is not what anyone reads.
    assert_eq!(brightness.snapped((64.0, 45.0)), (64.5, 45.0));
    // Past white, the bright end pins there and the ramp is what is left.
    assert_eq!(brightness.snapped((90.0, 40.0)), (85.0, 30.0));
    // A whole readout unit on the chroma axis is a hundredth of it, which
    // is the same statement about the same picture: the ends are read out
    // as percentages, so those are what land whole. To a tenth of a unit,
    // the resolution the readout itself claims — a hundredth is no binary
    // fraction, and `the_bar_can_only_reach_pairs_sanitize_leaves_alone`
    // covers what that costs the pair.
    for spread in [Spread::Brightness, Spread::Chroma] {
        let unit = spread.per_unit();
        let (min, max) = spread.axis();
        for centre in [0.0f32, 0.135, 0.49, 0.636, 0.896, 1.0].map(|v| min + v * (max - min)) {
            for spread_v in [0.0f32, 0.01, -0.07, 0.45, 0.999, -1.0, 4.0].map(|v| v * (max - min)) {
                let (c, s) = spread.snapped((centre, spread_v));
                for end in [c - s * 0.5, c + s * 0.5] {
                    let units = end * unit;
                    // A thousandth of a unit, which is a tolerance on the
                    // RECOMPOSITION and not on the snap: the pair is written
                    // as a middle and a ramp, so reading the ends back off
                    // it costs an ulp of the fraction — worst measured at
                    // 7.6e-6 of a percent, over every whole-percent pair of
                    // ends — where a snap that had actually missed the grid
                    // would miss by half a unit, five orders the other side
                    // of this.
                    assert!(
                        (units - units.round()).abs() < 1e-3,
                        "{spread:?}: {centre}/{spread_v} lands an end on {units} units",
                    );
                    assert!(
                        (min..=max).contains(&end),
                        "{spread:?}: {centre}/{spread_v} puts an end off the axis at {end}",
                    );
                }
            }
        }
    }
}

/// Every pair a gesture can settle on, put through the write path the bar
/// uses and then to `sanitized`.
///
/// The sweep runs in FRACTIONS of the axis so one set of positions means the
/// same thing on both, since the two axes are two orders of magnitude apart.
fn pairs_a_bar_can_write(spread: Spread, mut check: impl FnMut((f32, f32))) -> usize {
    let (min, max) = spread.axis();
    let of = |v: f32| min + v * (max - min);
    let mut checked = 0;
    for centre in [0.0f32, 0.01, 0.125, 0.5, 0.636, 0.896, 0.99, 1.0].map(of) {
        for width in [0.0f32, 0.07, -0.33, 1.0, -1.0].map(|v| v * (max - min)) {
            // Every grab the bar can settle on, against pointer positions
            // running the whole axis and a good way off both ends of it.
            // `value_at` clamps, so the widget itself never hands `apply` a
            // value off the axis; the extra range is aimed at `apply`'s OWN
            // clamps, which are what a pointer dragged past the bar's end
            // meets once that stops being true.
            let held = 0.135 * (max - min);
            let grabs = [
                SpreadGrab::Low,
                SpreadGrab::High,
                SpreadGrab::Middle { offset: 0.0, spread: width },
                SpreadGrab::Middle { offset: held, spread: width },
                SpreadGrab::Middle { offset: -held, spread: width },
            ];
            for grab in grabs {
                for step in -20..=120 {
                    check(grab.apply(of(step as f32 / 100.0), (centre, width), (min, max)));
                    checked += 1;
                }
            }
        }
    }
    checked
}

/// What the bar can reach is exactly what `sanitized` leaves alone. The two
/// say the same thing in different places — the bar because a handle off
/// the track is not a value it can express, the gradient because a pair out
/// of a hand-edited file never came through a bar — and nothing but this
/// stops them drifting into disagreeing about which pairs are legal. A bar
/// that could write a pair sanitize pulls in would draw one picture and
/// hold another.
#[test]
fn the_bar_can_only_reach_pairs_sanitize_leaves_alone() {
    for spread in [Spread::Brightness, Spread::Chroma] {
        let checked = pairs_a_bar_can_write(spread, |aimed| {
            let (c, s) = spread.legal(spread.snapped(aimed));
            let mut written = Gradient::default();
            spread.set(&mut written, (c, s));
            assert_eq!(
                written.sanitized(),
                written,
                "{spread:?}: the bar wrote a middle of {c} and a ramp of {s}, which \
                     sanitize does not accept as it stands",
            );
        });
        assert!(checked > 10_000, "only {checked} pairs — the sweep stopped covering it");
    }
}

/// And [`Spread::legal`] is load-bearing on the chroma axis rather than
/// belt-and-braces: snapping alone reaches pairs sanitize pulls in.
///
/// Both steps say both ends are on the axis — snapping by clamping the ends
/// it rounds, the gradient by bounding the ramp against what its middle
/// leaves — and the two are the same statement only in exact arithmetic. A
/// hundredth is no binary fraction, so a chroma pair recomposed from whole
/// percentages can land a ramp one ulp past the bound while whole `L*`
/// never does. Nothing about the picture turns on 6e-8 of chroma; what turns
/// on it is whether the number the bar reads out is the number the gradient
/// holds.
#[test]
fn snapping_alone_would_leave_a_chroma_pair_sanitize_pulls_in() {
    let over = |spread: Spread| {
        let mut over = 0;
        pairs_a_bar_can_write(spread, |aimed| {
            let (c, s) = spread.snapped(aimed);
            if spread.legal((c, s)) != (c, s) {
                over += 1;
            }
        });
        over
    };
    assert_eq!(over(Spread::Brightness), 0, "whole L* recomposes exactly, so this is a no-op");
    assert!(
        over(Spread::Chroma) > 0,
        "no snapped chroma pair needs pulling in, so `legal` is now untested here \
             and the sweep has stopped reaching the ends of the axis",
    );
}

/// One gradient carrying this pair on this spread and its own defaults
/// everywhere else.
fn holding(spread: Spread, pair: (f32, f32)) -> Gradient {
    let mut g = Gradient::default();
    spread.set(&mut g, pair);
    g
}

/// One bar of `spread`, built through the constructor that names it — which
/// is the only place the two differ to a caller.
fn spread_bar(spread: Spread, g: &mut Gradient, ui: &mut Ui) -> Response {
    match spread {
        Spread::Brightness => SpreadBar::brightness(g, Gradient::default()).show(ui),
        Spread::Chroma => SpreadBar::chroma(g, Gradient::default()).show(ui),
    }
}

/// Which of a brightness bar's two thumbs are lit, left then right, with the
/// pointer resting at `v` on the axis of a bar holding `pair`.
fn spread_lit_at(pair: (f32, f32), v: f32) -> Vec<bool> {
    let mut g = holding(Spread::Brightness, pair);
    let (min, max) = Spread::Brightness.axis();
    let shapes = after_passes(
        300.0,
        |bar| {
            let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
            let x = track.left() + track.width() * (v - min) / (max - min);
            vec![vec![egui::Event::PointerMoved(egui::pos2(x, bar.center().y))]; 2]
        },
        |ui| spread_bar(Spread::Brightness, &mut g, ui),
    );
    let mut thumbs = grips(&shapes);
    thumbs.sort_by(|a, b| a.0.left().total_cmp(&b.0.left()));
    thumbs.into_iter().map(|(_, lit)| lit).collect()
}

/// A pointer resting beside one end lights THAT end and dims the other, at
/// a rising ramp and at a falling one — and over the ramp's middle leaves
/// both lit, the slide being what a press there takes.
///
/// Both signs, because the ends are named for the PITCH they carry and
/// which thumb that is flips with the ramp: the left thumb is the low end
/// rising and the high end falling. A bar that mapped `Low` and `High` to
/// the wrong thumbs, or read the sign backwards, lights the far end at one
/// of the two and fails here; the fixture asserts the two ends really are
/// the two different grabs before it asks.
#[test]
fn a_resting_pointer_lights_the_spread_end_it_is_beside() {
    for spread in [40.0f32, -40.0] {
        let pair = (50.0, spread);
        let (left, right) = (30.0, 70.0);
        let grab = |v: f32| SpreadGrab::at(v, pair, 5.0);
        let low_is_left = matches!(grab(left - 3.0), SpreadGrab::Low);
        assert_eq!(
            low_is_left,
            spread > 0.0,
            "{spread}: the fixture's left end is not the grab it should be",
        );
        assert!(
            matches!(grab(right + 3.0), SpreadGrab::Low) != low_is_left,
            "{spread}: both ends of the fixture are the same grab",
        );
        assert_eq!(spread_lit_at(pair, left - 3.0), [true, false], "{spread}: beside the left");
        assert_eq!(spread_lit_at(pair, right + 3.0), [false, true], "{spread}: beside the right");
        assert_eq!(spread_lit_at(pair, 50.0), [true, true], "{spread}: over the middle");
    }
}

/// Paint one bar across a 300pt row and return what it emitted, each shape
/// still carrying the clip rect it was painted through — which is the only
/// thing that tells a knockout pass from the run it doubles.
fn paint_bar_clipped(spread: Spread, pair: (f32, f32)) -> Vec<egui::epaint::ClippedShape> {
    paint_bar_wide(300.0, spread, pair)
}

/// The same across a row of any width, for the sweeps that care where a
/// thumb falls relative to a run — which is a question about points, so it
/// moves with the row even at a fixed pair.
fn paint_bar_wide(width: f32, spread: Spread, pair: (f32, f32)) -> Vec<egui::epaint::ClippedShape> {
    let mut g = holding(spread, pair);
    painted(width, |ui| {
        spread_bar(spread, &mut g, ui);
    })
}

/// Paint one bar across a 300pt row and return what it emitted.
fn paint_bar(spread: Spread, pair: (f32, f32)) -> Vec<egui::Shape> {
    paint_bar_clipped(spread, pair).into_iter().map(|s| s.shape).collect()
}

/// BOTH runs on a spread bar are knocked out through a thumb standing in
/// them, where a [`RangeBar`] does it for its name alone. Neither of these
/// can move out of the way: the name is pinned left, and the two ends are
/// spelled into ONE readout parked at the right, which buys the pair a
/// single run to read but stands it exactly where a handle taken up the
/// axis comes to rest.
///
/// The pairs here are picked to walk the code — a thumb high enough to
/// reach the readout, one low enough to reach the name, and a flat ramp
/// that puts both grips at one x — and they are picked, so they say nothing
/// about where the shipped bars stand. That is
/// `the_bars_the_panes_build_are_knocked_out_wherever_they_rest_under_a_thumb`,
/// which reads the real defaults and is the one a retune can move.
///
/// [`RangeBar`]: crate::widgets::range::RangeBar
#[test]
fn a_spread_bar_knocks_out_both_its_runs_under_a_thumb() {
    let mut hit = Vec::new();
    for spread in [Spread::Brightness, Spread::Chroma] {
        let (min, max) = spread.axis();
        let of = |v: f32| min + v * (max - min);
        let span = |v: f32| v * (max - min);
        for (pair, what) in [
            ((of(0.64), span(0.44)), "a ramp across the middle of the axis"),
            ((of(0.9), span(0.18)), "a ramp up against the top of the axis"),
            ((of(0.08), span(0.14)), "a ramp down at the bottom, under the name"),
            // A FLAT ramp under the readout, which puts the two grips at
            // exactly the same x. That is the case `grip_over_text` names
            // as the reason it knocks out per grip and straight after that
            // grip's own fill: the second fill covers the first knockout,
            // and only a second knockout over the same ground repairs it.
            // Not a contrived pair — `chroma_ramp` is 0.0 in
            // `Gradient::default()`, and any chroma bar dragged flat lands
            // on it.
            ((of(0.9), 0.0), "a flat ramp parked under the readout"),
        ] {
            let shapes = paint_bar_clipped(spread, pair);
            let flat: Vec<_> = shapes.iter().map(|s| s.shape.clone()).collect();
            let (grips, runs) = (handles(&flat), text_boxes(&flat));
            let knocked = knockouts(&shapes);
            if pair.1 == 0.0 {
                assert_eq!(grips[0], grips[1], "{spread:?} {what}: grips are not coincident");
            }

            // The order the painter walks: each grip in turn, and under it
            // each run it stands in, name before readout. Derived from the
            // geometry so the expectation tracks the fixture rather than
            // pinning counts a font change could move.
            let want: Vec<_> = grips
                .iter()
                .flat_map(|g| {
                    runs.iter()
                        .filter(move |(r, _)| g.intersects(*r))
                        .map(move |(r, s)| (*g, *r, s.clone()))
                })
                .collect();
            assert_eq!(
                knocked.len(),
                want.len(),
                "{spread:?} {what}: {} knockouts for {} crossings, {knocked:?} vs {grips:?}",
                knocked.len(),
                want.len(),
            );
            for ((clip, at, text, colour), (grip, run, run_text)) in knocked.iter().zip(&want) {
                assert_eq!(text, run_text, "{spread:?} {what}: knocked out the wrong run");
                assert_eq!(at, run, "{spread:?} {what}: a knockout moved off its run");
                assert_eq!(
                    *colour,
                    Some(theme::panel()),
                    "{spread:?} {what}: a knockout is drawn in the panel colour",
                );
                assert_eq!(clip, grip, "{spread:?} {what}: a knockout escaped its thumb");
            }
            hit.extend(want.into_iter().map(|(_, _, s)| s));
        }
    }
    // Both runs are reached across the fixtures, and the readout — the run
    // that costs a DIGIT rather than a letter — by both bars.
    let readouts = hit.iter().filter(|s| s.contains('\u{2192}')).count();
    assert!(readouts >= 2, "the fixtures stopped standing a thumb in a readout: {hit:?}");
    assert!(
        hit.iter().any(|s| s == "Brightness" || s == "Saturation"),
        "the fixtures stopped standing a thumb in a name: {hit:?}",
    );
}

/// The four spread bars the panes actually build, at the pairs they
/// actually open with, rather than at a pair chosen to make the point.
///
/// This is the test that ties the knockout to something that ships: at the
/// shipped defaults every thumb standing in a run knocks it out, and no
/// knockout is painted where none does. It holds in both directions, so it
/// asserts something whether or not a default happens to rest a thumb on a
/// run today — and it carries no floor on how many do, since that depends
/// on the look a capture last put in the defaults, which a retune is free
/// to move. Reaching the knockout at all is the hand-picked fixtures' job
/// above.
#[test]
fn the_bars_the_panes_build_are_knocked_out_wherever_they_rest_under_a_thumb() {
    let nodes = harmonigraph_scene::view::ViewConfig::default().pitch_gradient;
    let spectral = crate::SpectrumConfig::default().spectrogram_gradient;
    for (pane, g) in [("nodes", nodes), ("spectral", spectral)] {
        for spread in [Spread::Brightness, Spread::Chroma] {
            let pair = match spread {
                Spread::Brightness => (g.lightness, g.lightness_ramp),
                Spread::Chroma => (g.chroma, g.chroma_ramp),
            };
            for width in [300.0f32, 423.0, 680.0] {
                let shapes = paint_bar_wide(width, spread, pair);
                let flat: Vec<_> = shapes.iter().map(|s| s.shape.clone()).collect();
                let (grips, runs) = (handles(&flat), text_boxes(&flat));
                let knocked = knockouts(&shapes);
                let want: Vec<_> = grips
                    .iter()
                    .flat_map(|gp| {
                        runs.iter().filter(move |(r, _)| gp.intersects(*r)).map(|(_, s)| s)
                    })
                    .collect();
                assert_eq!(
                    knocked.len(),
                    want.len(),
                    "{pane} {spread:?} at {width}pt: {} knockouts for {} crossings ({want:?})",
                    knocked.len(),
                    want.len(),
                );
                for (clip, _, text, colour) in &knocked {
                    assert_eq!(*colour, Some(theme::panel()), "{pane} {spread:?} {width}pt");
                    assert!(
                        grips.iter().any(|gp| gp == clip),
                        "{pane} {spread:?} {width}pt: a knockout of {text:?} is not on a thumb",
                    );
                }
            }
        }
    }
}

/// The bar draws the pair it holds: a handle at each end of the ramp, at its
/// own place along the whole axis. That is the whole claim the control
/// makes, and handles standing anywhere else would be a picture of some
/// other pair.
///
/// In FRACTIONS of the axis, which is what makes it one test of two bars:
/// the geometry is the same picture whether the ends are `L*` 42 and 86 or
/// 42% and 86% of the color available.
///
/// The inverted case is here because it is the one the picture CANNOT tell
/// apart: a ramp and its negative put the two handles in exactly the same
/// places, which is why the readout runs in pitch order instead.
#[test]
fn a_bar_stands_its_handles_where_its_numbers_say() {
    for spread in [Spread::Brightness, Spread::Chroma] {
        let (min, max) = spread.axis();
        let of = |v: f32| min + v * (max - min);
        for sign in [1.0f32, -1.0] {
            let ramp = sign * 0.44 * (max - min);
            let shapes = paint_bar(spread, (of(0.64), ramp));
            let bar = filled_rects(&shapes)[0].0;
            let hs = handles(&shapes);
            assert_eq!(hs.len(), 2, "{spread:?} at a ramp of {ramp} drew {} handles", hs.len());
            // The track a handle travels: the bar less the inset at either
            // end.
            let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
            for (want, h) in [(0.64 - 0.22, hs[0]), (0.64 + 0.22, hs[1])] {
                let at = track.left() + track.width() * want;
                assert!(
                    (h.center().x - at).abs() < 0.5,
                    "{spread:?} at a ramp of {ramp} puts an end {want} of the way up the \
                         axis, which is {at} across, and the handle stands at {}",
                    h.center().x,
                );
            }
        }
        // A flat ramp is one handle's worth of picture in the middle of an
        // empty track: none of the axis is spent on pitch, and there is
        // exactly one place the whole range is.
        let hs = handles(&paint_bar(spread, (of(0.3), 0.0)));
        assert_eq!(hs[0], hs[1], "{spread:?} drew a flat ramp's two handles apart");
    }
}

/// Two handles and nothing else standing on the track. A third mark on a
/// two-ended bar reads as a third handle whatever it is drawn like, and the
/// middle — the one thing that might have earned one — is not something a
/// gesture takes hold of.
#[test]
fn a_bar_stands_nothing_on_the_track_but_its_two_ends() {
    for spread in [Spread::Brightness, Spread::Chroma] {
        let (min, max) = spread.axis();
        let of = |v: f32| min + v * (max - min);
        let width = 0.44 * (max - min);
        for pair in [(of(0.64), width), (of(0.64), -width), (of(0.3), 0.0)] {
            let hs = handles(&paint_bar(spread, pair));
            assert_eq!(hs.len(), 2, "{spread:?} {pair:?} put {} marks on the track", hs.len());
        }
    }
}

/// The numbers one bar reads out, each parsed off the end of the readout
/// with whatever unit follows it stripped.
fn readout_ends(spread: Spread, pair: (f32, f32)) -> (String, Vec<f32>) {
    let shown = text_boxes(&paint_bar(spread, pair))
        .into_iter()
        .map(|(_, s)| s)
        .next_back()
        .expect("the bar draws a readout");
    let said = shown
        .split('\u{2192}')
        .map(|s| {
            s.trim().trim_end_matches(spread.suffix()).parse().expect("a readout is two numbers")
        })
        .collect();
    (shown, said)
}

/// The readout names the `L*` the curve actually draws at both ends of the
/// pitch range, at every pair the bar can be handed — not only at the ones
/// a drag leaves behind.
///
/// A drag snaps both ends to whole `L*`, so a bar that has been touched
/// reads out exactly whatever it does. Everything else arrives unsnapped:
/// the pair a fresh view opens on, the one a double-click goes home to, and
/// anything a saved blob or a hand-edited file carries. `ViewConfig`'s own
/// gradient is 53 over a ramp of 31, whose ends are 37.5 and 68.5 — the
/// case `snapped` is written to keep a DRAG off, arriving by the one road
/// that does not pass it.
///
/// A tenth of a point, because that is well under anything a viewer could
/// see and well over the half-point a whole-number readout costs at these
/// ends: the failure is a bar reading `38 → 68`, a span of 30, over a
/// gradient spending 31.
#[test]
fn a_brightness_readout_names_the_ends_the_curve_draws() {
    let fresh = ViewConfig::default().pitch_gradient;
    for pair in [(fresh.lightness, fresh.lightness_ramp), (64.0, 44.0), (64.0, -45.0), (20.0, 7.0)]
    {
        let g = holding(Spread::Brightness, pair);
        let (shown, said) = readout_ends(Spread::Brightness, pair);
        // In PITCH order, which is what the readout claims to be in: the
        // curve at t 0 and t 1, not the darker end and the brighter one.
        for (t, said) in [0.0, 1.0].into_iter().zip(said) {
            let drawn = g.lightness_and_hue(t).0 as f32;
            assert!(
                (said - drawn).abs() < 0.1,
                "{pair:?} reads out {shown:?}, saying L* {said} where the curve draws {drawn}",
            );
        }
    }
}

/// The same claim on the chroma axis, where the readout is a PERCENTAGE of
/// the curve's own fraction, so the two are a hundred apart and the
/// arithmetic between them is the thing that can be wrong.
///
/// A tenth of a percent, which is the resolution the readout claims — the
/// pairs below include the one a fresh view opens with, which arrives
/// without passing `snapped` and is not whole in percent either.
#[test]
fn a_chroma_readout_names_the_ends_the_curve_draws() {
    let fresh = ViewConfig::default().pitch_gradient;
    for pair in [(fresh.chroma, fresh.chroma_ramp), (0.5, 0.6), (0.5, -0.6), (0.2, 0.35)] {
        let g = holding(Spread::Chroma, pair);
        let (shown, said) = readout_ends(Spread::Chroma, pair);
        for (t, said) in [0.0, 1.0].into_iter().zip(said) {
            let drawn = g.chroma_at(t) as f32 * 100.0;
            assert!(
                (said - drawn).abs() < 0.1,
                "{pair:?} reads out {shown:?}, saying {said}% where the curve asks for \
                     {drawn}%",
            );
        }
    }
}

/// The two ends, in pitch order — the numbers the picture concretely draws,
/// each standing under its own handle, and each carrying the unit its own
/// axis is read in. Their ORDER is the sign: neither bar can show which end
/// of the pitch range carries the most, since the handles stand in the same
/// two places either way.
#[test]
fn a_bar_reads_out_its_two_ends_in_pitch_order() {
    let texts = |spread, pair| -> Vec<String> {
        text_boxes(&paint_bar(spread, pair)).into_iter().map(|(_, s)| s).collect()
    };
    let up = texts(Spread::Brightness, (64.0, 44.0));
    assert_eq!(up.len(), 2, "a name and one readout, not {up:?}");
    assert_eq!(up[0], "Brightness");
    assert_eq!(up[1], "42% \u{2192} 86%", "the bottom of the pitch range reads first");
    assert_eq!(
        texts(Spread::Brightness, (64.0, -44.0))[1],
        "86% \u{2192} 42%",
        "an inverted ramp draws the same two handles, so the readout is what says so",
    );
    let color = texts(Spread::Chroma, (0.64, 0.44));
    assert_eq!(color[0], "Saturation");
    assert_eq!(color[1], "42% \u{2192} 86%", "a share of the color reads out as one");
    assert_eq!(texts(Spread::Chroma, (0.64, -0.44))[1], "86% \u{2192} 42%");
}

/// Double-click one spread bar and answer the pair it wrote. `home` is what
/// the bar is told to reset to.
///
/// Through a real context for the reason [`drag_bar`] is: the reset is a
/// branch on a `Response`, and nothing synthetic reaches it.
fn double_click_spread(spread: Spread, start: Gradient, home: Gradient) -> (f32, f32) {
    let ctx = crate::tests::probe::themed();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
    let mut g = start;
    let bar = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut t = 0.0;
    let mut frame = |g: &mut Gradient, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(t),
                events,
                ..Default::default()
            },
            |ui| {
                let b = match spread {
                    Spread::Brightness => SpreadBar::brightness(g, home),
                    Spread::Chroma => SpreadBar::chroma(g, home),
                };
                bar.set(b.show(ui).rect)
            },
        );
    };
    frame(&mut g, vec![]);
    let at = bar.get().center();
    frame(&mut g, vec![egui::Event::PointerMoved(at)]);
    for _ in 0..2 {
        frame(&mut g, vec![press(at, true)]);
        frame(&mut g, vec![press(at, false)]);
    }
    spread.of(g)
}

/// Drag one bar across a 300pt row, from `from` to `to` as fractions of its
/// width, and answer the pair it wrote. A real gesture through a real
/// context, for the reason the range bar's is: what a gesture has hold of is
/// decided on the first frame egui calls the press a drag and then
/// remembered in context data, so a synthetic call exercises neither the
/// decision nor the memory.
fn drag_bar(spread: Spread, pair: (f32, f32), (from, to): (f32, f32)) -> (f32, f32) {
    let ctx = crate::tests::probe::themed();
    let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 100.0));
    let mut g = holding(spread, pair);
    let bar = std::cell::Cell::new(egui::Rect::NOTHING);
    let mut t = 0.0;
    let mut frame = |g: &mut Gradient, events: Vec<egui::Event>| {
        t += 1.0 / 60.0;
        let _ = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(t),
                events,
                ..Default::default()
            },
            |ui| bar.set(spread_bar(spread, g, ui).rect),
        );
    };
    // A frame with no input first: egui resolves the pointer against the
    // previous pass's rects, so a press cannot land on a bar that has never
    // been drawn.
    frame(&mut g, vec![]);
    let rect = bar.get();
    let at = |x: f32| egui::pos2(rect.left() + rect.width() * x, rect.center().y);
    frame(&mut g, vec![egui::Event::PointerMoved(at(from))]);
    frame(&mut g, vec![egui::Event::PointerMoved(at(from)), press(at(from), true)]);
    // A step clear of egui's drag threshold first, then the rest of the
    // way, for the reason the range bar's harness takes one: it is the gap
    // between where a press lands and where the gesture is first read, and
    // a jump straight to the target has no gap in it.
    let step = 12.0 / rect.width() * (to - from).signum();
    frame(&mut g, vec![egui::Event::PointerMoved(at(from + step))]);
    frame(&mut g, vec![egui::Event::PointerMoved(at(to))]);
    spread.of(g)
}

/// The two ends a pair draws, which is what the bar is really about and
/// what its readout says.
fn ends((centre, spread): (f32, f32)) -> (f32, f32) {
    (centre - spread * 0.5, centre + spread * 0.5)
}

/// The wiring, once, through a real pointer: a press on a handle moves that
/// end and leaves its partner standing, and a press between them slides
/// both without restyling the ramp. Every end lands on a whole `L*`, which
/// is what the readout claims of it.
#[test]
fn a_real_drag_on_a_brightness_bar_keeps_the_gesture_it_started() {
    // The default pair sits at 64 with a 44-point ramp, so its handles are
    // at L* 42 and 86 — a press at 0.86 of the way across is the bright
    // one, dragged out to the top of the axis.
    let dragged = ends(drag_bar(Spread::Brightness, (64.0, 44.0), (0.86, 1.0)));
    assert_eq!(dragged, (42.0, 100.0), "the low end moved, or the high one stopped short");

    // And a slide, from between the handles at 30 and 70: brighter by a
    // quarter of the axis, carrying its ramp.
    let pair = drag_bar(Spread::Brightness, (50.0, 40.0), (0.5, 0.75));
    assert!(pair.0 > 60.0, "the slide barely moved, landing at {}", pair.0);
    assert_eq!(pair.1, 40.0, "the slide restyled the ramp to {}", pair.1);
    let (low, high) = ends(pair);
    assert_eq!((low, high), (low.round(), high.round()), "{low}..{high} is not whole");
}

/// A press within an end's reach takes THAT end here too, whichever way the
/// drag then runs — the `bar::drag` rule on the third of the bars that
/// splits a handle from a middle.
///
/// Both bars, because the two work in axes two orders of magnitude apart
/// and the reach is converted into each: a conversion that dropped the
/// press position on one of them would leave the other passing.
///
/// The pairs above nearly reach this and stop short — a press ON a handle
/// dragged OUTWARD is still just inside the reach twelve points later,
/// which is what the live position leaves. Inward is where it runs out.
///
/// Asserted on the two ENDS rather than on the stored pair, which is the
/// only form of the claim that holds in both bars. "A slide leaves the ramp
/// alone" is false on the chroma bar: `snapped` recomposes the pair through
/// a hundredth-wide grid and `legal` runs it through `sanitize`, both in
/// f32, so a middle slide shifts `chroma_ramp` by an ulp at some runs and
/// not others — and a test that reads the ramp then passes against the live
/// position for exactly the runs where it does not. The ends say which
/// gesture ran: an end grab moves one and leaves the other standing, a
/// slide carries both. Held to a share of each bar's own axis so the one
/// assertion spans two units, and loose enough at the far end that an ulp
/// of recomposition is not a moved handle.
#[test]
fn a_press_within_a_spread_ends_reach_takes_that_end_whichever_way_it_runs() {
    let bars = [(Spread::Brightness, (64.0f32, 44.0f32)), (Spread::Chroma, (0.64, 0.44))];
    for (spread, pair) in bars {
        let (min, max) = spread.axis();
        let bar = filled_rects(&paint_bar(spread, pair))[0].0;
        let track = bar.shrink2(Vec2::new(HANDLE_INSET, 0.0));
        let was = ends(pair);
        let handle = track.left() + track.width() * (was.0 - min) / (max - min);
        let frac = |x: f32| (x - bar.left()) / bar.width();
        for reach in [-GRAB_PX * 0.8, GRAB_PX * 0.8] {
            for run in [-40.0f32, 40.0] {
                let from = handle + reach;
                let moved = ends(drag_bar(spread, pair, (frac(from), frac(from + run))));
                let aimed = format!("{spread:?}: pressed {reach} from the low end, ran {run}");
                let share = |a: f32, b: f32| (a - b).abs() / (max - min);
                assert!(
                    share(moved.1, was.1) < 1e-4,
                    "{aimed}: the high end came along, by {} of the axis",
                    share(moved.1, was.1),
                );
                assert!(
                    share(moved.0, was.0) > 0.02,
                    "{aimed}: the low end moved {} of the axis, which is nothing",
                    share(moved.0, was.0),
                );
            }
        }
    }
}

/// The same wiring on the chroma bar, which is where the units the widget
/// works in are actually at stake: the gesture arrives in pixels, the axis
/// is a fraction two orders smaller than the `L*` one, and the readout is a
/// percentage of it. A drag has to land on a whole PERCENT and leave a pair
/// the gradient accepts unchanged, which is what dragging an end all the way
/// out to the vivid end of the axis asks for.
#[test]
fn a_real_drag_on_a_chroma_bar_lands_on_whole_percentages() {
    // A 44% ramp about 64% stands its ends at 42% and 86%: the same picture
    // as the brightness case above, one axis over.
    let (low, high) = ends(drag_bar(Spread::Chroma, (0.64, 0.44), (0.86, 1.0)));
    for (end, want) in [(low, 42.0f32), (high, 100.0)] {
        assert!(
            (end * 100.0 - want).abs() < 0.05,
            "an end landed on {}% where {want}% is what the bar can say",
            end * 100.0,
        );
    }
    // And what the gradient makes of it: the pair is one it holds as it
    // stands, ends and all — the whole point of `Spread::legal` sitting on
    // the write path.
    let pair = drag_bar(Spread::Chroma, (0.64, 0.44), (0.86, 1.0));
    let written = holding(Spread::Chroma, pair);
    assert_eq!(written.sanitized(), written, "the drag wrote a pair sanitize pulls in");
}
