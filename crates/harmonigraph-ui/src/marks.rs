//! The vector-glyph rasterizer for a note name's marks -- the accidental and
//! comma signs, drawn rather than typeset, and the stacked-name layout that
//! places them around a letter. Consumed by the lattice's own labels, the
//! spectral roll's (`panes::spectral::names`), and `crate::text`'s mark
//! atlas, which packs the rasterized bitmaps into a sheet of their own.

/// The note name's letter, and the size every other measurement in this file
/// is quoted against.
///
/// A size at scale 1, as everything here is. What a label is actually drawn at
/// follows the camera — see
/// [draw_node_labels](crate::panes::lattice::draw_node_labels) — and the Labels
/// section's Name size bar on top of that, so this is NOT the size a fresh view
/// draws at: that bar opens at
/// [`label_scale`](harmonigraph_scene::ViewConfig::label_scale)'s default and
/// the letter lands there.
///
/// Which is exactly where the lattice's letter parts company with the spectral
/// pane's two text sizes, `axes::MARKING_PT` and `names::LABEL_PT`. Those are
/// requoted whenever their bar settles away from 1, so that the built-in is the
/// size the pane is read at and a saved view is not a standing correction of
/// it. Here the built-in is the PROPORTION BASE of a label that has parts:
/// [`CENTS_SIZE`], [`CENTS_GAP`] and [`LABEL_REACH`] are absolute points quoted
/// against this letter and only [`MARK_SIZE`] is derived from it, so moving it
/// means re-proportioning the label by hand and getting the same picture. A
/// marking and a ribbon name are each one point size with nothing hanging off
/// them, which is what makes the requote free there and not here — so the
/// lattice's retune goes on the bar, and this number holds the proportions.
pub(crate) const NAME_SIZE: f32 = 30.0;
/// The pane the sizes here are quoted against: 860 points tall, which is what
/// the lattice gets in the 1512x886 window they were dialled in. A pane half
/// as tall draws them half the size, because a pane half as tall draws the
/// LATTICE half the size — see [draw_node_labels](crate::panes::lattice::draw_node_labels).
///
/// Height, not width: the projection's window is a height, and the aspect
/// ratio spreads it sideways, so a node's size on screen is a function of the
/// pane's height whichever way the pane is stretched.
pub(crate) const REFERENCE_HEIGHT: f32 = 860.0;
/// The cents readout under it: subordinate to the name, so smaller, and
/// tucked right beneath it rather than floating free.
///
/// Under half the letter, which is what makes the pair read as a name with a
/// number under it rather than as two lines. It is a six-character number
/// against a one-character name, so at any size close to the letter's the
/// readout is the wider of the two and wins the eye.
pub(crate) const CENTS_SIZE: f32 = 14.0;
/// How far a label can reach from the node it belongs to, in points at
/// scale 1 — the name, its marks, the gap and the cents line under it, with
/// room to spare. Scaled with the label like everything else here, so the
/// cull leaves a zoomed-in label the room it now takes. Only used to decide
/// that a label is too far off the pane to be worth laying out, so it errs
/// generous: too small silently clips a label at the edge, too large only
/// costs the work this saves.
pub(crate) const LABEL_REACH: f32 = 96.0;

/// Air between the bottom of the name's glyphs and the top of the cents
/// readout's. Real pixels of gap, since both ends are measured as ink: the
/// two are one label, sitting together without crowding.
///
/// Small against the letter it hangs under — a fifth of the name's size. Ink
/// to ink is a tighter measure than it sounds: the same gap struck between
/// galley boxes would carry the leading of both fonts on top of it, which is
/// the spacing that had the two floating apart.
pub(crate) const CENTS_GAP: f32 = 3.0;
/// Every mark in the column, relative to the letter -- accidental and comma
/// alike, which is what keeps the two rows reading as one annotation rather
/// than as one row set smaller than the other. Small enough that the two
/// stacked still fit inside the letter's own height: a label that grows
/// taller than its letter reads as two lines rather than one name.
const MARK_SCALE: f32 = 0.55;
/// The size the marks are actually laid out at.
pub(crate) const MARK_SIZE: f32 = NAME_SIZE * MARK_SCALE;

/// How far the accidental rises and the comma sign drops, as a fraction of
/// the offset that would set each flush with the letter's own line box.
///
/// This is the ONLY lever on the air between the two rows, and it has to be
/// the only one: every mark in the column sets at [`MARK_SIZE`], so anything
/// that opens the gap by shrinking one row makes that row visibly the
/// smaller of the two. `♯`'s ink is the tallest in the column, but the pair
/// that gives the mismatch away is the two count digits — one directly over
/// the other, in the same column, where 10% off one of them reads as a
/// typo rather than as air.
///
/// Flush (1.0) is the loosest the rows can sit and still be one name: it
/// spends every point the letter's leading offers on the space between them,
/// about 6pt of clear air under `♯` at scale 1 and nearer nine under `♭`,
/// which is more than the marks' own heights and stops the column reading as
/// one stacked annotation. Pulling in tightens that without touching what
/// says which mark is which, since the cue is the ORDER of the two.
///
/// Measured at scale 1, the two clearances that bind — a count over the count
/// below it, and `♯` over the whole row under it. The counts are ink; the `♯`
/// is its drawn BITMAP, which is its ink rounded out to whole physical pixels
/// and so a twentieth of a point taller than the same glyph set as type:
///
/// | rise | count over count | `♯` over the row |
/// |------|------------------|------------------|
/// | 0.8  | 0.6pt            | −1.45pt (they interleave) |
/// | 0.9  | 2.3pt            | 0.25pt           |
/// | 1.0  | 4.0pt            | 1.95pt           |
///
/// 0.9 is the tightest that keeps the rows from interleaving at all.
const MARK_RISE: f32 = 0.9;

/// Iosevka Fixed's advance, as a fraction of the em: every cell is half an
/// em wide. A drawn mark claims exactly this, so it sits in the same column
/// grid as the counts typeset beside it.
pub(crate) const MARK_ADVANCE: f32 = 0.5;
/// How far a mark's COUNT is pulled back toward its sign, as a fraction of
/// the mark size — `♯2` and `+2` set tighter than one monospace cell each.
///
/// A count is not a second character of a word, it is a multiplier on the
/// sign beside it, and monospace advance sets the two as far apart as it
/// sets `♯` from the letter's own column. Iosevka leaves 0.11em of ink-to-ink
/// air there (0.13 after `♭`, which has the deeper side bearing), against
/// glyphs about 0.4em wide.
///
/// One constant covers both halves of the column because every drawn sign is
/// cut to the typeface's own ink width: `♯` and `+` share [`MARK_INK_W`]
/// exactly, so `♯2` and `+2` open identically, and tracking them by the same
/// amount keeps them matched rather than merely both tighter. `♭` is the
/// narrower glyph ([`FLAT_INK_W`]) and opens 0.022em wider, which is the air
/// the face itself gives it.
pub(crate) const MARK_TRACK: f32 = 0.06;
/// Iosevka's own stroke weight, measured off its outlines: 70/1000 em, as a
/// fraction of the mark's font size.
///
/// A constant rather than a setting, because the face gives no other answer
/// to weigh it against. It uses ONE weight for everything STRAIGHT — `♯`'s
/// verticals are 69 and its bars 70, the hyphen is 70, `+` is about 70, and
/// `♭`'s stem is 69 — and it does that across a glyph 878 units tall (`♯`)
/// and one 70 units tall (`-`) alike. So the typeface's own answer to
/// "should a smaller mark be drawn heavier?" is no, and an optical-sizing
/// argument for 0.10 or 0.12 is an argument against the face these marks sit
/// in.
///
/// The one curve in the set is the one exception, and it is a fraction of
/// this rather than a weight of its own: see [`FLAT_BOWL_WEIGHT`].
///
/// Heavier weights were also compensating for something since fixed: while
/// the marks were composited shapes their feathered joins read heavier than
/// the geometry measured, and while they were typeset a bar this thin really
/// did smear. Rasterized with a whole-pixel floor (see [`mark_key`]), 0.07 is
/// a clean line — and it is the line the rest of the label is drawn with.
pub(crate) const MARK_WEIGHT: f32 = 0.07;

/// The ink width Iosevka gives `+`, `-` and `♯` alike within that cell
/// (372/1000 em). Matching it is what keeps a drawn sign from reading as a
/// different size of mark than the one stacked over it.
pub(crate) const MARK_INK_W: f32 = 0.372;
/// And the height of `+`'s upright (386/1000 em).
const PLUS_INK_H: f32 = 0.386;

/// `♯`'s ink height (878/1000 em), in the width `+` has.
///
/// An accidental is a tall mark -- more than twice the `+`'s height -- and
/// it is the size the same glyph sets at, so drawing it changes what the
/// column is made OF and not what it measures.
const SHARP_INK_H: f32 = 0.878;
/// `♭`'s ink box (328 x 818/1000 em). Narrower than `♯`, as the face has it.
const FLAT_INK_W: f32 = 0.328;
const FLAT_INK_H: f32 = 0.818;

/// `♯`'s uprights, as a fraction of its ink width either side of the centre:
/// Iosevka centres them on 133.5 and 366.5 of a box running 64..436.
const SHARP_STEM_X: f32 = 0.313;
/// How much of the ink height one upright covers (818 of 878). Both are that
/// length, and they are OFFSET -- the left flush with the bottom of the box,
/// the right flush with the top. That stagger is the sharp's own, and it is
/// the cue that still reads when the mark is a dozen pixels tall.
const SHARP_STEM_H: f32 = 0.932;
/// The bars' MIDPOINTS, as a fraction of the ink height either side of the
/// centre: Iosevka runs them through 176.5 and 503.5 about an ink centre of
/// 340.
const SHARP_BAR_Y: f32 = 0.186;
/// How far a bar climbs across the glyph, as a fraction of the ink width:
/// Iosevka lifts each one 75 units over the 372 the mark is wide.
///
/// The slant is what makes a sharp a sharp rather than a hash, and it looks
/// like it ought to cost something: a bar this thick is one physical pixel at
/// the size the analyzer sets its names, and one that climbs a whole pixel
/// end to end is partial coverage the whole way, where a level bar is a row
/// of solid ink. That is the argument [`mark_key`] makes for taking the mark
/// out of type at all, so it is worth saying where it does NOT reach.
///
/// Levelling the bars is worth 0.1 of a point on the reading that matters --
/// 29.3% against 29.4%, where the same glyph set as type is 40.8% (the swing
/// `a_drawn_accidental_breathes_less_than_the_type_it_replaced` takes) -- and
/// 0.4 of a point at three times that size, 37.5% against 37.9%. The walk is
/// horizontal and the bars are what runs that way, so sliding one moves it
/// ALONG itself and lights very nearly the same pixels either way; it is the
/// UPRIGHTS that a horizontal walk smears, and they are vertical in both
/// designs. The slant is all but free, and what it buys is that the mark is a
/// sharp rather than a hash. Do not spend the glyph to buy back a fraction of
/// a point.
const SHARP_SLANT: f32 = 0.202;

/// `♭`'s bowl, as one cubic along the centre of Iosevka's own: control points
/// in fractions of the ink box, x rightward and y DOWN from its top, which is
/// the space a mark's pieces are built in.
///
/// One width the whole way, as every other drawn mark is. The middle two
/// points sit off the box to the right, which is ordinary for a cubic: the
/// curve itself bulges to 0.89 of the width, and the stroke around it reaches
/// the box's edge exactly.
///
/// Chosen against the face's own outline rather than by eye or by a midline
/// fit: the points that minimise the coverage difference between this mark
/// and a rasterization of Iosevka's `uni266D` contours, read over the range
/// of sizes a name is drawn at. That difference is 5.4% of the glyph's ink,
/// against 11.5% for a bowl fitted to the midline instead.
///
/// The midline is the tempting target and it is the wrong one at the bottom
/// of the glyph, where there is no bowl to have a midline: stem and bowl are
/// one mass below [`FLAT_MERGE`], so a midline sampled there is the MASS's,
/// and a cubic pulled onto it leaves the bowl arriving at the stem from the
/// wrong angle.
///
/// The fit is CONSTRAINED, and the constraint is what the eye actually reads:
/// the bowl's outer edge has to arrive along the foot's run rather than
/// across it. Coverage alone will not buy that -- a corner in the outline
/// costs almost no area, so an unconstrained fit is happy to let the bowl
/// bulge past the run and cut back onto it, which reads as a crease in the
/// silhouette however close the areas are. So the fit carries the tangency as
/// a constraint, and `a_flats_outline_has_no_crease_where_the_bowl_lands`
/// holds it.
///
/// The touch is INTERIOR to the curve. Requiring it at the curve's end
/// instead is the tidier-looking condition -- it reduces to putting the last
/// two control points on the run, offset inward by half a stroke -- and it is
/// much worse, 9.9% against 5.4%, because it straightens the whole last third
/// of the bowl.
const FLAT_BOWL: [[f32; 2]; 4] =
    [[0.2104, 0.4242], [1.0655, 0.2518], [1.2094, 0.7585], [0.1252, 0.9020]];
/// What the bowl weighs against [`MARK_WEIGHT`], which is the stem's.
///
/// The face's one weight covers everything STRAIGHT -- `♯`'s verticals and
/// bars, the hyphen, the `+`, and this mark's own stem -- and its bowl is the
/// exception. Measured perpendicular between Iosevka's two contours, the bowl
/// runs 70 units at its right extreme where the stroke is vertical, 63 across
/// the top where it is horizontal, and 59..61 for the whole lower-left run
/// into the stem. One number for a curve that carries three is a compromise,
/// and 0.861 is where it lands. A width that VARIES along the curve is the
/// obvious improvement and it is not one: fitted, it lands the counter's tip
/// exactly and is worth 0.01 of a point, which does not pay for giving
/// [`curve_arms`] a width function again.
///
/// It is not a nicety. A bowl at the stem's full weight is too fat on the
/// inside exactly where the counter is trying to close, so the counter tips
/// out early and the bowl reads as meeting the stem abruptly rather than
/// running into it. Nor can the shape be bought back by moving the curve: at
/// full weight, the placement that lengthens the counter is the placement
/// that pushes the outer edge across the foot's run, which is the crease
/// `a_flats_outline_has_no_crease_where_the_bowl_lands` exists to forbid. The
/// two are one trade, and the face buys out of it by thinning the bowl.
///
/// Against the face's own outline, over the sizes a name is drawn at: 7.0% of
/// the glyph's ink at the stem's weight, 5.4% at this one.
const FLAT_BOWL_WEIGHT: f32 = 0.861;
/// The straight run that closes the bowl into the foot of the upright, as the
/// width the glyph still carries per unit of height above the tip.
///
/// Iosevka's is straight to within a percent over its whole length: the glyph
/// is 0.326 of the ink width across at 0.916 of the way down, 0.231 at 0.940,
/// 0.136 at 0.965, and nothing at all at the bottom-left corner. That single
/// slope is the foot, and it is what makes the bottom of a `♭` a point rather
/// than a stem with a bowl stuck on the side of it.
const FLAT_FOOT_SLOPE: f32 = 3.86;
/// Where the bowl stops being a stroke and the foot takes over, as a fraction
/// of the ink height.
///
/// Two separate things happen at this height in the face, which is why one
/// constant does both jobs:
///
/// - Iosevka's counter bottoms out here (49 of a box running -69..749, so
///   0.85575). Below it there is no hole, so there are no longer two strokes
///   to see: stem and bowl are ONE mass, as wide as [`FLAT_FOOT_SLOPE`] says
///   and narrowing to the point at the corner. Two strokes carried down to
///   the corner separately never join, and the bowl reads as hung off the
///   stem rather than merged into it.
/// - It is also where [`FLAT_BOWL`]'s outer edge comes onto the foot's run --
///   TANGENT to it, so the outline changes hands without changing direction.
///
/// The second is the load-bearing one and this is set to it. It sits a little
/// below the first, which is what leaves the counter room to tip out on its
/// own rather than being cut off square: the hole closes at 0.849 and the
/// outline changes hands at 0.873, so the triangle's top edge is buried
/// inside the mark by the time it appears.
///
/// So the foot is a triangle whose top vertex sits ON the bowl's outer edge,
/// sharing its tangent -- and the silhouette below the bowl is one straight
/// run to the corner. Move this without refitting the bowl and the two part
/// company: the triangle's top corner pokes out of the outline, or the bowl
/// bulges past the run and creases back onto it.
const FLAT_MERGE: f32 = 0.8729;
/// Air between the accidental/comma column and the septimal mark, as a
/// fraction of the mark's font size. Small: enough that the mark is not
/// read as another row of the stack it sits beside, not so much that it
/// floats free of the name it belongs to.
pub(crate) const SEPTIMAL_GAP: f32 = 0.22;
/// The septimal mark's box, relative to the `+` box beside it. One, which
/// is to say: the same box.
///
/// It was 1.25 to keep a FILLED TRIANGLE from reading lighter than the `+`,
/// a triangle covering half its bounding box. The chevron that replaced it
/// has the opposite problem -- two arms spanning the full diagonal are
/// already more ink than the `+` at equal stroke -- so the compensation
/// went from justified to doubled.
const SEPTIMAL_BULK: f32 = 1.0;
/// One piece of a mark, in the mark bitmap's own pixel space.
///
/// These are never drawn to the screen. They describe a shape that gets
/// rasterized ONCE into a coverage bitmap, so pieces may abut or overlap
/// freely -- coverage is a max over pieces, not a composite of them, and
/// none of the artifacts of drawing them separately can arise.
enum MarkPiece {
    Bar(egui::Rect),
    /// Any convex four corners: a stroked segment with FLAT terminals, the
    /// `♯`'s slanted bars, or -- with a corner repeated -- the triangle that
    /// closes a `♭`'s foot, which the coverage test handles as the degenerate
    /// quad it is.
    ///
    /// Not a distance-to-segment test, which is the easy way to stroke a
    /// line and gives round caps and a round join. Iosevka cuts its
    /// terminals flat, and a chevron with rounded ends and a blunt apex
    /// reads as a different hand than the `♯` above it.
    Quad([egui::Pos2; 4]),
}

impl MarkPiece {
    /// Whether this piece covers a point, in bitmap pixel space.
    fn covers(&self, p: egui::Pos2) -> bool {
        match self {
            MarkPiece::Bar(rect) => rect.contains(p),
            MarkPiece::Quad(corners) => {
                // Convex and consistently wound: inside is the same side of
                // every edge.
                let (mut neg, mut pos) = (false, false);
                for i in 0..4 {
                    let (a, b) = (corners[i], corners[(i + 1) % 4]);
                    let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
                    neg |= cross < 0.0;
                    pos |= cross > 0.0;
                }
                !(neg && pos)
            }
        }
    }

    /// The box this piece lies inside, in the same space.
    ///
    /// What a scanline uses to skip the pieces it cannot touch (see
    /// [`Scanline`]), which is the difference between a mark costing its own
    /// area and costing its area times its piece count.
    fn bounds(&self) -> egui::Rect {
        match self {
            MarkPiece::Bar(rect) => *rect,
            MarkPiece::Quad(corners) => egui::Rect::from_points(corners),
        }
    }
}

/// The pieces a single row of a rasterization can touch, reused down the
/// bitmap.
///
/// Coverage is a union over pieces, so the naive reading tests every piece at
/// every sample -- fine for a `+`, which is two pieces at any size, and not
/// fine for the `♭`, whose bowl is a stroked curve flattened into as many as
/// forty arms. At the largest size a zoom can ask for, that reading costs
/// 34ms to build one bitmap against the `+`'s 1.9ms, and it is paid on the
/// frame a new size is first asked for -- which during a zoom drag is every
/// frame.
///
/// A piece spans a few rows out of hundreds, so filtering once per row leaves
/// a handful to test per sample and puts the `♭` back beside the marks around
/// it. The ANSWER is untouched: this drops only pieces whose own bounding box
/// excludes the row, which could not have covered anything on it.
struct Scanline<'a> {
    pieces: &'a [MarkPiece],
    bounds: Vec<egui::Rect>,
    row: Vec<usize>,
}

impl<'a> Scanline<'a> {
    fn new(pieces: &'a [MarkPiece]) -> Self {
        Scanline {
            bounds: pieces.iter().map(|piece| piece.bounds()).collect(),
            pieces,
            row: Vec::with_capacity(pieces.len()),
        }
    }

    /// Narrow to the pieces reaching into `top..bottom`.
    fn seek(&mut self, top: f32, bottom: f32) {
        self.row.clear();
        self.row.extend(
            (0..self.pieces.len())
                .filter(|&i| self.bounds[i].min.y < bottom && self.bounds[i].max.y > top),
        );
    }

    /// Whether the row's pieces cover a point -- the same union as testing
    /// them all, since the ones left out do not reach this row.
    fn covers(&self, p: egui::Pos2) -> bool {
        self.row.iter().any(|&i| self.pieces[i].covers(p))
    }
}

/// One arm of a stroked mark: the segment `a`-`b`, `thick` wide, with flat
/// terminals, extended past `b` by half that width so two arms meeting there
/// overlap into a clean point instead of leaving a notch in the outer corner.
/// Overlap is free -- coverage is a union.
///
/// One width and not two. Every drawn stroke here is of ONE weight along its
/// whole length -- the `♭`'s bowl carries less of it than the stem does (see
/// [`FLAT_BOWL_WEIGHT`]), but it carries the same amount the whole way round.
/// Where the face's own bowl stops being a stroke it is because it has MERGED
/// with the stem, and that mass is drawn as a mass (see [`FLAT_MERGE`])
/// rather than approximated by thinning the stroke into it -- a stroke that
/// thins is a stroke that loses its core at the far end, which is the same
/// argument that levels the sharp's bars.
fn arm(a: egui::Pos2, b: egui::Pos2, thick: f32) -> MarkPiece {
    let along = (b - a).normalized();
    let across = egui::vec2(-along.y, along.x) * (thick / 2.0);
    let tip = b + along * (thick / 2.0);
    MarkPiece::Quad([a + across, tip + across, tip - across, a - across])
}

/// Which mark, at what size in physical pixels -- the identity of one
/// rasterized bitmap, and its place in the mark sheet
/// ([`crate::text::MarkAtlas`]).
///
/// Nothing about the RIM is in here, and that is the whole shape of the mark
/// path now: the halo is the same per-fragment arithmetic that haloes the
/// letters (`fs_rim` in `harmonigraph_render`'s text shader), so a bitmap is a
/// function of the mark's own design and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MarkKey {
    kind: MarkKind,
    /// The mark's font size in whole physical pixels. Whole, because a
    /// bitmap has to be rasterized at SOME integer size, exactly as a glyph
    /// is; the on-screen size then steps by a pixel as the camera zooms,
    /// which is what a glyph atlas does too.
    size_px: u32,
    /// Stroke weight in physical pixels x16, so the cache key stays integral
    /// without quantizing the weight to something visible.
    weight_16: u32,
}

impl MarkKey {
    pub(crate) fn kind(self) -> MarkKind {
        self.kind
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MarkKind {
    Minus,
    Plus,
    /// The septimal mark -- a chevron -- and whether it points up.
    ///
    /// A chevron out of four designs built to be compared: it is the
    /// lightest of them, and the only one whose weight reads as belonging
    /// with the `+`/`-` rather than as a second, heavier kind of mark. The
    /// others (filled and hollow triangles, a stemmed arrow) are deleted
    /// rather than kept switchable -- a comparison that has been made is
    /// not a setting.
    Septimal(bool),
    /// The two accidentals. Separate variants rather than one flag, because
    /// unlike the septimal pair they are not one shape and its mirror.
    Sharp,
    Flat,
}

impl MarkKind {
    /// Every drawn shape a note name can emit, in stable atlas order.
    pub(crate) const ALL: [Self; 6] = [
        Self::Minus,
        Self::Plus,
        Self::Septimal(false),
        Self::Septimal(true),
        Self::Sharp,
        Self::Flat,
    ];
}

/// Clear pixels around every mark bitmap, so a mark's own ink never reaches
/// the quad that carries it. One is enough: a bilinear tap half a texel past
/// the ink already reads the margin whole. See [`mark_geometry`], which is
/// where the reasoning and the measurement live.
pub(crate) const MARK_BITMAP_PAD: usize = 1;

/// A mark's geometry in its bitmap's pixel space, with the bitmap size.
///
/// Built once per (design, size, weight) on a canonical grid at the origin,
/// which is what makes a mark's proportions IDENTICAL every time it is
/// drawn. Building it in screen space instead let the two arms of a `+`
/// land on different subpixel offsets and rasterize to different lengths --
/// the lopsidedness that survived every attempt to fix it by snapping,
/// because snapping the pieces is what made them disagree.
fn mark_geometry(key: MarkKey) -> (Vec<MarkPiece>, [usize; 2]) {
    let size = key.size_px as f32;
    let thick = key.weight_16 as f32 / 16.0;
    let (w, h) = match key.kind {
        MarkKind::Septimal(_) => {
            (MARK_INK_W * size * SEPTIMAL_BULK, PLUS_INK_H * size * SEPTIMAL_BULK)
        }
        MarkKind::Sharp => (MARK_INK_W * size, SHARP_INK_H * size),
        MarkKind::Flat => (FLAT_INK_W * size, FLAT_INK_H * size),
        _ => (MARK_INK_W * size, PLUS_INK_H * size),
    };
    // The bitmap is a whole number of pixels with the shape centered in it, so
    // a design and its mirror rasterize to mirror images, and it carries a
    // clear pixel on every side.
    //
    // That margin is what keeps a sliding mark from stepping, and it is the
    // shader that spends it. `paint_mark` hands the mark to the same batch
    // the type goes through, so a mark is a glyph of the mark sheet:
    // `vs_glyph` grows the quad past the ink, and `tap` reads up to
    // `PATCH_MARGIN` -- half a texel -- outside the patch. The coverage
    // therefore falls to zero across a texel that exists rather than stopping
    // dead at the bound, and the ink is interior at every phase. `ceil` alone
    // leaves under half a pixel of clearance, which is not enough for it to
    // have fallen to zero by then. It is the bargain epaint strikes by
    // padding every glyph in its atlas, bought here by hand.
    //
    // ONE texel is the whole of what the pad may be, and the PACKING is what
    // decides it rather than the ink: `MarkAtlas::claim` starts the next
    // patch at `x + w`, so marks sit touching on a shelf with no gutter, and
    // the clear texel each carries is the only thing that half-texel tap can
    // find out there. Take it away and the tap reaches the neighbour's
    // outermost column instead, so an accidental draws a ghost of whatever
    // was packed beside it, changing as a zoom repacks the shelf.
    // `text.wgsl` argues the same bound from the other side.
    //
    // The accidentals are where it shows, because they are the marks with a
    // full-height stroke standing at the edge of the box. Walking a name at
    // the roll's own 1.4286 physical pixels a frame, the drawn ink's centre
    // of mass advances by 0.86..2.14 pixels bare against 1.41..1.44 padded --
    // a mark lurching a pixel around its own motion, against one that glides.
    // `a_sliding_marks_ink_advances_with_the_quad_that_carries_it` is that
    // reading.
    let (bw, bh) = (
        w.ceil().max(1.0) + 2.0 * MARK_BITMAP_PAD as f32,
        h.ceil().max(1.0) + 2.0 * MARK_BITMAP_PAD as f32,
    );
    let c = egui::pos2(bw / 2.0, bh / 2.0);
    let (hw, hh) = (w / 2.0, h / 2.0);
    let pieces = match key.kind {
        MarkKind::Minus => {
            vec![MarkPiece::Bar(egui::Rect::from_center_size(c, egui::vec2(w, thick)))]
        }
        MarkKind::Plus => vec![
            MarkPiece::Bar(egui::Rect::from_center_size(c, egui::vec2(w, thick))),
            MarkPiece::Bar(egui::Rect::from_center_size(c, egui::vec2(thick, h))),
        ],
        MarkKind::Septimal(up) => {
            // Point-toward-the-tip: -1 draws upward, +1 downward, so the
            // shape is written once and mirrored by arithmetic.
            let dir = if up { -1.0 } else { 1.0 };
            let tip = egui::pos2(c.x, c.y + dir * hh);
            let base_l = egui::pos2(c.x - hw, c.y - dir * hh);
            let base_r = egui::pos2(c.x + hw, c.y - dir * hh);
            vec![arm(base_l, tip, thick), arm(base_r, tip, thick)]
        }
        MarkKind::Sharp => {
            let stem_h = SHARP_STEM_H * h;
            // The stagger: each upright reaches one end of the box, so the
            // pair is offset by exactly the height they do not fill.
            let stagger = (h - stem_h) / 2.0;
            let stem = |side: f32| {
                MarkPiece::Bar(egui::Rect::from_center_size(
                    egui::pos2(c.x + side * SHARP_STEM_X * w, c.y - side * stagger),
                    egui::vec2(thick, stem_h),
                ))
            };
            // The bars are parallelograms with VERTICAL ends, which is how
            // the face cuts them: the thickness is measured up the page
            // rather than across the slant, so a bar meets an upright along
            // the upright's own edge.
            let bar = |side: f32| {
                let mid = c.y + side * SHARP_BAR_Y * h;
                let (left, right) = (mid + SHARP_SLANT * w / 2.0, mid - SHARP_SLANT * w / 2.0);
                MarkPiece::Quad([
                    egui::pos2(c.x - hw, left - thick / 2.0),
                    egui::pos2(c.x + hw, right - thick / 2.0),
                    egui::pos2(c.x + hw, right + thick / 2.0),
                    egui::pos2(c.x - hw, left + thick / 2.0),
                ])
            };
            vec![stem(-1.0), stem(1.0), bar(-1.0), bar(1.0)]
        }
        MarkKind::Flat => {
            let (left, top) = (c.x - hw, c.y - hh);
            // The mark is a stem and a bowl down to where the counter closes,
            // and ONE mass below that -- so the merge is drawn as a piece of
            // its own rather than left to the two strokes meeting.
            //
            // Held no lower than the height at which the foot's own run is
            // still as wide as the stem: at a weight floored to a whole pixel
            // (see `mark_key`) the stroke can be a third of the ink width, and
            // a wedge narrower than the stem it continues would put a step in
            // the left edge where the face has none.
            let merge = FLAT_MERGE.min(1.0 - thick / (w * FLAT_FOOT_SLOPE));
            let (mouth, waist) = (top + merge * h, FLAT_FOOT_SLOPE * (1.0 - merge) * w);
            let mut pieces = vec![
                MarkPiece::Bar(egui::Rect::from_min_max(
                    egui::pos2(left, top),
                    egui::pos2(left + thick, mouth),
                )),
                // The foot, as the triangle its own straight run cuts: widest
                // where the counter runs out, nothing at the box's corner.
                MarkPiece::Quad([
                    egui::pos2(left, mouth),
                    egui::pos2(left + waist, mouth),
                    egui::pos2(left, top + h),
                    egui::pos2(left, top + h),
                ]),
            ];
            let bowl = FLAT_BOWL.map(|[u, v]| egui::pos2(left + u * w, top + v * h));
            // The bowl's own weight, held to a whole pixel like the stem's:
            // the floor is why these marks are drawn at all (see `mark_key`),
            // and a bowl scaled under it would be back to the sub-pixel stroke
            // it exists to escape. So the two meet at 1px on a mark small
            // enough for it to matter, and the face's ratio shows through
            // wherever there are pixels to carry it.
            pieces.extend(curve_arms(bowl, (thick * FLAT_BOWL_WEIGHT).max(1.0)));
            pieces
        }
    };
    (pieces, [bw as usize, bh as usize])
}

/// How far a flattened curve may sit from the curve it stands in for, in
/// bitmap pixels. Well inside one supersample cell (see
/// [`MARK_SUPERSAMPLE`]), so the flattening can never be what decides a
/// coverage sample.
const CURVE_TOLERANCE: f32 = 0.1;

/// A cubic stroked at `thick` with flat terminals: the curve flattened into a
/// chain of [`arm`]s, each overlapping the next, so coverage unions them into
/// one smooth stroke.
///
/// The count comes from the curve's own curvature rather than being fixed,
/// and that is a cost decision, not a fussy one: every arm is tested at every
/// coverage sample, so a count high enough for a mark two hundred pixels
/// across would be paid at the dozen-pixel sizes that are the ordinary case
/// and buy nothing there. A cubic's polyline sits within `max|B''| / 8n²` of
/// it, which is the bound solved for `n` here.
fn curve_arms(p: [egui::Pos2; 4], thick: f32) -> Vec<MarkPiece> {
    let bend = |a: egui::Pos2, b: egui::Pos2, c: egui::Pos2| {
        ((a.to_vec2() - b.to_vec2() * 2.0 + c.to_vec2()) * 6.0).length()
    };
    let worst = bend(p[0], p[1], p[2]).max(bend(p[1], p[2], p[3]));
    let n = ((worst / (8.0 * CURVE_TOLERANCE)).sqrt().ceil() as usize).clamp(2, 64);
    let at = |t: f32| {
        let m = 1.0 - t;
        ((p[0].to_vec2() * m * m * m)
            + (p[1].to_vec2() * 3.0 * m * m * t)
            + (p[2].to_vec2() * 3.0 * m * t * t)
            + (p[3].to_vec2() * t * t * t))
            .to_pos2()
    };
    (0..n)
        .map(|i| {
            let (t0, t1) = (i as f32 / n as f32, (i + 1) as f32 / n as f32);
            let (a, b) = (at(t0), at(t1));
            // The first arm backs off half a width, so its flat terminal is
            // buried in whatever the curve leaves rather than cutting across
            // that join at the angle the curve happens to depart at. Every
            // later one is already covered by its predecessor's overhang.
            let a = if i == 0 { a - (b - a).normalized() * (thick / 2.0) } else { a };
            arm(a, b, thick)
        })
        .collect()
}

/// Supersampling grid used to turn a mark's outline into coverage. 4x4 is
/// finer than the antialiasing a shape would have got from the tessellator,
/// and on a mark a dozen pixels square it is the difference between an edge
/// and a staircase.
///
/// What it costs is the mark's own area, once per size the camera asks for —
/// the whole of it and no more, since the halo is `fs_rim`'s arithmetic
/// rather than a second bitmap read off this grid at twenty offsets. At the
/// largest size a zoom can reach that is a couple of milliseconds for a `♭`,
/// paid on the frame that size is first asked for.
const MARK_SUPERSAMPLE: usize = 4;

/// Rasterize a mark to an alpha coverage image -- the same thing a font
/// rasterizer hands the atlas for a glyph, and packed into a sheet the same
/// way (see [`crate::text::MarkAtlas`]).
pub(crate) fn rasterize_mark(key: MarkKey) -> egui::ColorImage {
    let (pieces, [w, h]) = mark_geometry(key);
    let n = MARK_SUPERSAMPLE;
    let step = 1.0 / n as f32;
    let mut scanline = Scanline::new(&pieces);
    let mut pixels = Vec::with_capacity(w * h);
    for y in 0..h {
        scanline.seek(y as f32, y as f32 + 1.0);
        for x in 0..w {
            let mut hits = 0;
            for sy in 0..n {
                for sx in 0..n {
                    let p = egui::pos2(
                        x as f32 + (sx as f32 + 0.5) * step,
                        y as f32 + (sy as f32 + 0.5) * step,
                    );
                    // Coverage is a UNION over pieces, never a sum: two
                    // pieces meeting cannot darken or brighten their join.
                    if scanline.covers(p) {
                        hits += 1;
                    }
                }
            }
            let a = (255 * hits / (n * n)) as u8;
            pixels.push(egui::Color32::from_white_alpha(a));
        }
    }
    egui::ColorImage { size: [w, h], pixels, source_size: egui::vec2(w as f32, h as f32) }
}

/// Add a mark to the label's own run of glyphs, haloed like the letters
/// beside it -- the same rim, by the same arithmetic, over the same kind of
/// thing.
///
/// The mark is ONE quad of coverage cut from a sheet, which is exactly what a
/// glyph is, so every difference that came of drawing it as separate shapes is
/// gone by construction: no seam between pieces to feather twice, no join to
/// composite twice, no arm to rasterize at its own subpixel offset. The quad
/// lands wherever the label lands, and bilinear sampling resolves it the way
/// it resolves a glyph.
///
/// Two things follow from it being an instance of the glyph pass rather than a
/// pair of image quads on the painter, and both are the point:
///
///   - it is drawn where the LETTERS are. On the lattice that is inside the
///     scene pass at its node's place in the back-to-front order, so a node in
///     front covers a name and its marks together instead of cutting the name
///     and leaving the sign floating on the disc that covered it (#207);
///   - the rim is `fs_rim`'s arithmetic rather than a bitmap of its own, so
///     the stamped order -- every rim, then every fill -- holds ACROSS the
///     seam between a sign and the count typeset beside it. On the painter
///     that order breaks at every such seam, and breaks the opposite way in
///     each pane, depending on when the batch flushes.
///
/// Returns how far the mark reaches from its own center, which the caller
/// needs to know what the cents readout has to clear.
fn paint_mark(
    batch: &mut crate::text::TextBatch,
    ctx: &egui::Context,
    ppp: f32,
    key: MarkKey,
    center: egui::Pos2,
    color: egui::Color32,
    outline: egui::Color32,
) -> f32 {
    // The bitmap is rasterized on the same grid the type is (see `mark_key`),
    // and DRAWN at whatever size the label is actually at -- the two are the
    // same split, for the same reason, and they have to be the same split or a
    // name would glide while the `+` beside it stepped. Both halves are the
    // batch's now, which is what makes them the same split rather than two
    // that have to agree.
    let [_, h] = batch.mark(ctx, key, center, ppp, color, outline);
    // UNmagnified, because the caller is still laying out at the rasterized
    // size and this is one of its measurements — the magnification is applied
    // once, to the finished label, and a measurement that had it applied
    // already would carry it twice.
    //
    // Less the clear margin the bitmap carries on every side, which is quad
    // and not reach: counting it would hold the cents readout a pixel further
    // off than the ink it is clearing.
    (h as usize - 2 * MARK_BITMAP_PAD) as f32 / ppp / 2.0
}

/// The key for one mark at the size a label is drawing at.
///
/// `size` is the mark font size in points; the bitmap is rasterized in
/// physical pixels, so the size crosses into pixels here and is rounded --
/// a bitmap has an integer size or it has none.
pub(crate) fn mark_key(kind: MarkKind, size: f32, weight: f32, ppp: f32) -> MarkKey {
    let size_px = (size * ppp).round().max(2.0);
    // Floored at a whole physical pixel: the whole reason these marks are
    // not type is that Iosevka's own bars are 70/1000 em, which is 0.58px
    // at MARK_SIZE, and a stroke thinner than a pixel spends all of its
    // contrast on partial coverage.
    //
    // That is a claim about the STROKE and not about the glyph, which is why
    // it reaches the accidentals too. `♯` and `♭` carry 878 and 818 units of
    // ink against the hyphen's 70, and being tall buys them nothing: every
    // line in them is the same 69-70 units the hyphen is, so at the size this
    // column sets they are the same sub-pixel stroke (issue #292).
    //
    // What the floor buys is worth stating exactly, because it is less than
    // it sounds. At this size a drawn mark's pixels are still ALL partial
    // coverage, the same as the type's -- one whole pixel of stroke landing
    // at a sub-pixel offset is two pixels at half. What changes is how much
    // that varies as the mark slides: a stroke floored at a pixel is nearly
    // the same picture at every phase, where one at 0.58px is not, and the
    // swing drops from 49.0% to 30.8% for `♭` and 40.8% to 29.4% for `♯`.
    // The shimmer is the variation, not the softness, which is why this is
    // the fix and a sharper bitmap is not. The reading is the one
    // `a_drawn_accidental_breathes_less_than_the_type_it_replaced` takes.
    let thick = (weight * size * ppp).max(1.0);
    MarkKey { kind, size_px: size_px as u32, weight_16: (thick * 16.0).round() as u32 }
}

/// A note name centered on `anchor`: the letter, then a column carrying its
/// accidental above its syntonic-comma sign, then a column for the septimal
/// mark (`♯` riding high like a superscript, `+` low like a subscript).
/// Every mark is counted rather than repeated (see [`harmonigraph_core::NoteName`]),
/// so a name deep in the lattice -- or five modulations out along the sevens
/// axis -- stays a couple of characters wide instead of sprawling off its node.
///
/// Every sign in the column is DRAWN and only the counts are type: Iosevka
/// has no line thicker than 70/1000 em anywhere -- in the hyphen, in the
/// `+`, or in the accidentals -- and at the size this column sets, that is
/// a stroke narrower than a pixel. Digits are not lines and keep a core of
/// their own, so they stay typeset. See [`mark_geometry`] and [`mark_key`].
///
/// The septimal mark takes its direction twice over -- from the shape it is
/// drawn as, and from which end of the column it sits at. The second cue is
/// free (the column already offsets its marks) and it is the one that
/// survives when the node is small enough that the shape is four pixels.
///
/// Returns how far the lowest thing drawn reaches below the line the stack was
/// finally laid out on -- ink, not boxes -- which is what the cents readout
/// hangs off.
///
/// That line is `anchor.y` under [`NameLead::Centred`] and only there. A name
/// LED by its letter is slid before anything is emitted, and that slide has a y
/// in it for any direction that is not flat -- both vertical cardinals, and
/// every ray the spiral places a name on but two -- so the reach a caller gets
/// back is measured from a line the caller was not told about. Every consumer
/// of the VALUE is `Centred`: the cents readout is the only one, and the roll
/// and the spiral both discard it. So this is a contract to read before adding
/// the next one, not a defect to go looking for.
///
/// Monospace for in-lattice text: labels align across nodes and match the
/// technical feel of the readouts.
///
/// `scale` is the size the label is LAID OUT and rasterized at; `magnify` is
/// how much bigger it is finally drawn, which is what lets the size follow a
/// zoom continuously while the atlas still sees one size per rung. See
/// [`crate::text::ladder`], which hands the pair out together, and
/// [`crate::text::TextBatch::magnified`]; 1.0 is a label drawn at exactly the
/// size it was rasterized at.
///
/// `lead` decides what `anchor` MEANS -- see [`NameLead`]. The magnification
/// turns about that same point either way, so a name led by its letter keeps
/// the gap it was placed with at every rung of the ladder: the letter's ink is
/// at distance zero from the point everything is scaled about, and zero times
/// anything is zero.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: harmonigraph_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    magnify: f32,
    lead: NameLead,
) -> f32 {
    batch.magnified(anchor, magnify, |batch| {
        stacked_name(batch, painter, anchor, name, color, outline, scale, lead)
    })
}

/// What the point handed to [`draw_stacked_name`] is a point OF.
///
/// The two pictures want different answers, and the difference is what the name
/// is about. A lattice node's label sits ON its node, so the node is the middle
/// of it. A roll name stands OFF the end of the ribbon it belongs to, by a gap
/// a reader is supposed to see -- and a gap is only a gap to the INK. Placed by
/// its box a name carries two errors into that gap: the font's own side bearing,
/// and whatever the caller's estimate of the name's width was wrong by. Both
/// scale with the type, so both open as the pitch zoom does, and the name
/// drifts off the note it names exactly where the drift is easiest to see.
/// Issue #349 measured 5.5 points of it across the zoom with time running
/// across the pane and 12.2 with it running down.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum NameLead {
    /// The whole name centred on the point, box and all.
    Centred,
    /// The LETTER's ink flush against the point, the name running the way this
    /// direction points -- ANY screen direction, and the two callers use it
    /// both ways: the roll hands it a pitch axis
    /// ([`Axes::dir_depth`](crate::panes::spectral::axes::Axes)), which is one
    /// of the four cardinals, and the spiral hands it a pitch class's own ray,
    /// which is any angle on the circle.
    ///
    /// Nothing here has to name a screen side to do it. `stacked_name` slides
    /// the layout by the LEAST projection of the letter's ink onto this
    /// direction, taken over the ink box's four corners, which puts the ink
    /// against the plane through the anchor at right angles to it. On a
    /// cardinal that plane is a box edge and the slide reads the letter's side
    /// bearing; on a diagonal it is a corner that touches, so the gap a reader
    /// sees along the direction is the one asked for and the gap either side of
    /// it is wider.
    ///
    /// The letter and not the whole name, and that is the trade issue #151
    /// holds rather than an oversight. A reader lines a column of names up by
    /// their letters, so the letter is the piece that has to stand still
    /// against its ribbon end whatever the spelling costs; the mark column
    /// trails it, and is therefore free to reach PAST the end the name is
    /// anchored to. Measured on a 300pt pane, a `B♭↓` whose growth runs against
    /// the way its marks are set puts them 4.81 points past that end at the
    /// dialled size and 38.73 at the two-octave floor. Containing the name
    /// instead is one edit here -- the union of this ink with the marks' own
    /// quads, rather than this ink alone -- and it buys that back by letting
    /// every marked name's letter sit deeper than a bare one's.
    Letter(egui::Vec2),
}

/// [`draw_stacked_name`]'s layout, all of it at the rasterized size — and
/// nothing below here knows the label will be drawn at any other, marks
/// included, because everything it emits goes through the one batch that
/// applies the magnification.
#[allow(clippy::too_many_arguments)]
fn stacked_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    name: harmonigraph_core::NoteName,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    lead: NameLead,
) -> f32 {
    let name_font = egui::FontId::monospace(NAME_SIZE * scale);
    let mark_font = egui::FontId::monospace(MARK_SIZE * scale);
    let mark_size = MARK_SIZE * scale;
    let ppp = painter.ctx().pixels_per_point();
    let measure = |text: &str, font: &egui::FontId| {
        painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size()
    };
    // Each piece is drawn centered on its own line, so its ink reaches this
    // far below that line.
    let ink_below = |text: &str, font: &egui::FontId, size: egui::Vec2| {
        painter_ink(painter, text, font).max.y - size.y / 2.0
    };
    // Past one, a mark carries its count as a digit beside the sign. One is
    // common enough that the digit would be noise, so it stays bare.
    let count_text = |n: i32| if n.abs() > 1 { n.abs().to_string() } else { String::new() };

    let letter_text = name.letter.to_string();
    let letter = measure(&letter_text, &name_font);
    // Every mark sits on one line of the mark font, so both rows rise by the
    // same amount -- including the drawn ones, which have no galley to ask.
    let line = measure("0", &mark_font);
    // A drawn mark is centered on its line box, with NO correction toward
    // the font's own bar axis, and that is a measured decision rather than
    // an omission.
    //
    // Iosevka puts the ink of `-`, `+`, `♯` and `♭` alike at 340/1000 em
    // above the baseline: one axis, by design, and every mark this column
    // draws is on it. Its typo line box is centered on exactly that and its
    // hhea box 35 units above it, which looked like a 0.035 em correction
    // worth applying. But at MARK_SIZE egui rasterizes into whole-point
    // atlas cells, and those glyphs come back centered at -0.5pt, +0.0pt and
    // +0.5pt from the line box: a whole point of spread across glyphs the
    // font draws on one axis. The offset is below the size at which the text
    // beside it can be positioned, and reading it off any single glyph
    // measures that glyph's rounding.
    //
    // Centered is the mean of what the font actually renders, it is exactly
    // where `+` lands, and it is where every drawn mark's ink box now goes
    // by construction rather than by an atlas cell's luck.
    let rise = MARK_RISE * (letter.y - line.y) / 2.0;
    let cell = MARK_ADVANCE * mark_size;

    // Only the counts are laid out as text -- every sign in the column is
    // drawn, so which sign is a MarkKind rather than a character. That reads
    // the accidental's direction off `sharps` itself, exactly as the comma
    // row beside it reads its own, rather than off `NoteName`'s spelling.
    let accidental = count_text(name.sharps);
    let syntonic = count_text(name.syntonic_commas);
    let septimal = count_text(name.septimal_commas);
    let track = MARK_TRACK * mark_size;
    // A sign and its count read as ONE mark, so the count follows its sign's
    // cell tracked in rather than a clear cell away -- see MARK_TRACK.
    let tracked_width = |sign: f32, count: &str| {
        if count.is_empty() {
            sign
        } else {
            sign + measure(count, &mark_font).x - track
        }
    };
    // A drawn sign claims one cell; its count follows in the same column.
    let signed_width =
        |count: &str, present: bool| if present { tracked_width(cell, count) } else { 0.0 };
    // Both rows of the stack claim the same cell -- one column grid for the
    // pair, which is what keeps the two counts sharing a left edge.
    let column = signed_width(&accidental, name.sharps != 0)
        .max(signed_width(&syntonic, name.syntonic_commas != 0));
    let septimal_column = signed_width(&septimal, name.septimal_commas != 0);
    // Air between the accidental column and the septimal mark, so the mark
    // reads as its own thing rather than as a third row of the stack.
    let gap = if name.septimal_commas != 0 { SEPTIMAL_GAP * mark_size } else { 0.0 };
    let left = anchor.x - (letter.x + column + gap + septimal_column) / 2.0;

    // Everything below places off `left` and `anchor.y`, so leading the name by
    // its letter is a correction to those two and nothing else -- the stack is
    // laid out identically either way, then slid. Taken BEFORE the first glyph
    // is emitted, since a batch has no way to move what it has already been
    // handed.
    let (left, anchor) = match lead {
        NameLead::Centred => (left, anchor),
        NameLead::Letter(toward) => {
            // The letter's ink where the centred layout would have put it. Its
            // galley goes out LEFT_CENTER at (left, anchor.y), so the box
            // `painter_ink` measures hangs off that corner.
            let ink = painter_ink(painter, &letter_text, &name_font)
                .translate(egui::vec2(left, anchor.y - letter.y / 2.0));
            // How far the ink's own trailing edge sits from the point, against
            // the way the name runs. Projecting the corners answers all four
            // cardinals without naming a screen side, and for the two that run
            // along x it reads the letter's side bearing -- the term no
            // constant can remove, being per-glyph.
            let behind = [ink.left_top(), ink.right_top(), ink.left_bottom(), ink.right_bottom()]
                .iter()
                .map(|&corner| (corner - anchor).dot(toward))
                .fold(f32::INFINITY, f32::min);
            let shift = -toward * behind;
            (left + shift.x, anchor + shift)
        }
    };

    batch.text(
        painter,
        egui::pos2(left, anchor.y),
        egui::Align2::LEFT_CENTER,
        letter_text.clone(),
        name_font.clone(),
        color,
        outline,
    );
    let mut bottom = ink_below(&letter_text, &name_font, letter);

    // Drawn sign, then its count: same column, same line, so the pair reads
    // as one mark rather than as a glyph with a number after it.
    let mut draw_signed = |x: f32, direction: f32, count: &str, kind: MarkKind| -> f32 {
        let key = mark_key(kind, mark_size, MARK_WEIGHT, ppp);
        let center = egui::pos2(x + cell / 2.0, anchor.y + direction * rise);
        let half_height = paint_mark(batch, painter.ctx(), ppp, key, center, color, outline);
        if !count.is_empty() {
            batch.text(
                painter,
                egui::pos2(x + cell - track, anchor.y + direction * rise),
                egui::Align2::LEFT_CENTER,
                count.to_owned(),
                mark_font.clone(),
                color,
                outline,
            );
        }
        // Whichever reaches lower: the mark's own bitmap from its center, or
        // the count's digits from theirs.
        let ink = half_height.max(if count.is_empty() {
            0.0
        } else {
            ink_below(count, &mark_font, line)
        });
        direction * rise + ink
    };

    // The accidental rides high, just inside the top of the letter; the comma
    // sign drops by the same amount under it.
    if name.sharps != 0 {
        let kind = if name.sharps > 0 { MarkKind::Sharp } else { MarkKind::Flat };
        bottom = bottom.max(draw_signed(left + letter.x, -1.0, &accidental, kind));
    }
    if name.syntonic_commas != 0 {
        let kind = if name.syntonic_commas > 0 { MarkKind::Plus } else { MarkKind::Minus };
        bottom = bottom.max(draw_signed(left + letter.x, 1.0, &syntonic, kind));
    }
    if name.septimal_commas != 0 {
        // Centered on the letter's own line -- the seam between the
        // accidental riding above it and the comma sitting below -- rather
        // than in one slot or the other. It is not a third member of that
        // stack: it belongs to a different prime, and sitting across the
        // divide with a gap before it is what says so. The chevron carries
        // its own direction, so the slot is free to mean this instead.
        bottom = bottom.max(draw_signed(
            left + letter.x + column + gap,
            0.0,
            &septimal,
            MarkKind::Septimal(name.septimal_commas > 0),
        ));
    }
    bottom
}

/// One line of centered label text, measured like [`draw_stacked_name`] so
/// the two can be stacked against each other: returns how far its ink reaches
/// below `anchor.y`. Used for the label lines that are numbers rather than
/// note names — an off-sheet node's cents, and its comma (see
/// [SevensLabel](harmonigraph_scene::SevensLabel)).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_plain_name(
    batch: &mut crate::text::TextBatch,
    painter: &egui::Painter,
    anchor: egui::Pos2,
    text: &str,
    color: egui::Color32,
    outline: egui::Color32,
    scale: f32,
    magnify: f32,
) -> f32 {
    let font = egui::FontId::monospace(NAME_SIZE * scale);
    let size =
        painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).size();
    batch.magnified(anchor, magnify, |batch| {
        batch.text(
            painter,
            anchor,
            egui::Align2::CENTER_CENTER,
            text.to_owned(),
            font.clone(),
            color,
            outline,
        );
    });
    painter_ink(painter, text, &font).max.y - size.y / 2.0
}

/// The box the glyphs of `text` actually cover, relative to the galley's own
/// top-left. Distinct from the galley's size, which pads the glyphs out to a
/// full line box; laying two pieces of text out edge to edge by their boxes
/// leaves a visible gap that neither piece's ink accounts for.
pub(crate) fn painter_ink(painter: &egui::Painter, text: &str, font: &egui::FontId) -> egui::Rect {
    painter.layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::PLACEHOLDER).mesh_bounds
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The alpha at one pixel of a rasterized mark.
    fn coverage(img: &egui::ColorImage, x: usize, y: usize) -> u8 {
        img.pixels[y * img.size[0] + x].a()
    }

    /// A mark's bitmap with its clear margin cropped off, so the box a test
    /// scans is the box the DESIGN fills.
    ///
    /// [`MARK_BITMAP_PAD`] exists for the draw path -- it keeps a sliding
    /// mark's outermost column inside the quad that carries it -- and it says
    /// nothing about the shape. A test asking whether a stroke reaches its
    /// box's edge is asking about the design, so it reads this rather than
    /// the bitmap and stays a statement about the mark.
    fn mark_ink(key: MarkKey) -> egui::ColorImage {
        let img = rasterize_mark(key);
        let [w, h] = img.size;
        let pad = MARK_BITMAP_PAD;
        let (cw, ch) = (w - 2 * pad, h - 2 * pad);
        let pixels = (0..ch)
            .flat_map(|y| (0..cw).map(move |x| (x, y)))
            .map(|(x, y)| img.pixels[(y + pad) * w + x + pad])
            .collect();
        egui::ColorImage { size: [cw, ch], pixels, source_size: egui::vec2(cw as f32, ch as f32) }
    }

    /// A `+` rasterizes to its own mirror, both ways.
    ///
    /// The lopsidedness -- one arm visibly longer than the other -- came of
    /// building the arms in SCREEN space, where each landed on its own
    /// subpixel offset and rasterized to its own length. Built on the
    /// bitmap's canonical grid, symmetry is structural.
    #[test]
    fn a_plus_rasterizes_to_its_own_mirror() {
        for size in [6.0_f32, 8.25, 13.0, 21.0] {
            let img = rasterize_mark(mark_key(MarkKind::Plus, size, 0.12, 2.0));
            let [w, h] = img.size;
            for y in 0..h {
                for x in 0..w {
                    let a = coverage(&img, x, y);
                    assert_eq!(a, coverage(&img, x, h - 1 - y), "top/bottom at {x},{y} @{size}");
                    assert_eq!(a, coverage(&img, w - 1 - x, y), "left/right at {x},{y} @{size}");
                }
            }
        }
    }

    /// The square where a `+`'s bar and upright cross is no brighter than
    /// the bar itself.
    ///
    /// A mark is rasterized as COVERAGE, and coverage is a union over the
    /// pieces -- so a join cannot be composited twice. Drawn as separate
    /// translucent shapes it could, and did: the middle of a fading `+` lit
    /// up against its own arms however carefully the pieces were made not to
    /// overlap, because the tessellator feathers every edge and two abutting
    /// edges feather over each other.
    #[test]
    fn a_pluss_join_is_no_brighter_than_its_arms() {
        let img = rasterize_mark(mark_key(MarkKind::Plus, 16.0, 0.12, 2.0));
        let [w, h] = img.size;
        let peak = img.pixels.iter().map(|p| p.a()).max().expect("a + has ink");
        assert_eq!(coverage(&img, w / 2, h / 2), peak, "the join is not the brightest point");
        // And it is not the ONLY point at that level: the arms reach it too,
        // so the join does not read as a spot.
        let at_peak = img.pixels.iter().filter(|p| p.a() == peak).count();
        assert!(at_peak > w.min(h), "only {at_peak} pixels reach peak coverage of {w}x{h}");
    }

    /// A mark's bitmap is the same whatever subpixel position its node
    /// projects to: the key rounds into whole physical pixels, and the
    /// bitmap is placed rather than rebuilt.
    #[test]
    fn a_mark_is_one_bitmap_wherever_it_lands() {
        // All within one rounding bucket: 8.25..8.37 points is 17 physical
        // pixels at 2x. Sizes that straddle a bucket edge SHOULD differ --
        // that is the bitmap stepping by a pixel as the camera zooms, the
        // same thing a glyph atlas does.
        let a = mark_key(MarkKind::Minus, 8.25, 0.12, 2.0);
        for size in [8.25_f32, 8.26, 8.30, 8.36] {
            assert_eq!(mark_key(MarkKind::Minus, size, 0.12, 2.0), a, "{size}");
        }
        // A minus is a single bar, so it too is its own mirror.
        let img = rasterize_mark(a);
        let [w, h] = img.size;
        for y in 0..h {
            for x in 0..w {
                assert_eq!(coverage(&img, x, y), coverage(&img, x, h - 1 - y));
            }
        }
    }

    /// The septimal mark's direction lives in the shape: up and down are
    /// each other's mirror, which is the whole of what tells them apart now
    /// that both sit on the same line.
    #[test]
    fn the_septimal_chevron_mirrors_with_its_direction() {
        let up = rasterize_mark(mark_key(MarkKind::Septimal(true), 12.0, 0.10, 2.0));
        let down = rasterize_mark(mark_key(MarkKind::Septimal(false), 12.0, 0.10, 2.0));
        assert_eq!(up.size, down.size);
        let [w, h] = up.size;
        for y in 0..h {
            for x in 0..w {
                assert_eq!(
                    coverage(&up, x, y),
                    coverage(&down, x, h - 1 - y),
                    "up and down disagree at {x},{y}"
                );
            }
        }
        // And it is NOT its own vertical mirror, or the two would be
        // indistinguishable and the mark would say nothing.
        assert!(
            (0..h).any(|y| (0..w).any(|x| coverage(&up, x, y) != coverage(&up, x, h - 1 - y))),
            "a chevron that mirrors onto itself carries no direction"
        );
    }

    /// Every drawn mark is one stroke weight, and it is the one the font
    /// draws with.
    ///
    /// Iosevka uses a single weight across its whole set -- `♯`'s verticals
    /// are 69 units, its bars 70, the hyphen 70 -- and it does that over a
    /// glyph 878 units tall and one 70 units tall alike. These marks sit in
    /// that set, so they are drawn at 70/1000 em too, and the `-` and the
    /// `+`'s bar have to come out identical.
    #[test]
    fn every_drawn_mark_is_the_fonts_one_stroke_weight() {
        const WEIGHT: f32 = 0.07;
        let size = 24.0;
        let ppp = 2.0;
        let minus = rasterize_mark(mark_key(MarkKind::Minus, size, WEIGHT, ppp));
        let plus = rasterize_mark(mark_key(MarkKind::Plus, size, WEIGHT, ppp));
        // The bar's thickness: inked rows down a column, read a quarter of
        // the way across so the `+`'s upright is nowhere near it.
        let bar = |img: &egui::ColorImage| {
            let x = img.size[0] / 4;
            (0..img.size[1]).filter(|&y| coverage(img, x, y) > 127).count()
        };
        assert_eq!(bar(&minus), bar(&plus), "the - and the +'s bar disagree");
        // And that thickness is the font's own 0.07 em, to the pixel.
        let want = (WEIGHT * size * ppp).round() as usize;
        assert_eq!(bar(&minus), want, "{} px drawn, {want} px is 0.07 em", bar(&minus));
    }

    /// The stroke floor is one PHYSICAL pixel, not one point -- a point is
    /// two pixels here, so flooring in points drew every small mark at twice
    /// the minimum and made the whole set look heavy.
    #[test]
    fn the_weight_floor_is_a_physical_pixel() {
        assert_eq!(mark_key(MarkKind::Minus, 4.0, 0.0001, 2.0).weight_16, 16);
        assert_eq!(mark_key(MarkKind::Minus, 4.0, 0.0001, 1.0).weight_16, 16);
        // Above the floor the weight is what decides it: 0.1 * 20 * 2 = 4px.
        assert_eq!(mark_key(MarkKind::Minus, 20.0, 0.1, 2.0).weight_16, 64);
    }

    /// No mark this app can ask for is wider than a shelf of the sheet it is
    /// packed into.
    ///
    /// `crate::text::MarkAtlas` has no fallback for one that is: a shelf
    /// packer that cannot place something has to either grow sideways —
    /// moving every patch already handed out this pass, which is the one thing
    /// the sheet promises not to do — or write across the row into whatever is
    /// packed beside it. Neither is worth having when the bound is a constant,
    /// and this is where the constant is checked rather than asserted.
    ///
    /// Measured through the ladder rather than computed from the ink ratios,
    /// so it answers for what a label can actually reach: the ceiling on a
    /// name's raster (`crate::text::MAX_GLYPH_PX`) is the whole of what bounds
    /// a mark's size, and the walk below asks for far past it at every density
    /// the app is drawn at.
    #[test]
    fn a_mark_is_never_wider_than_the_sheet_it_is_packed_into() {
        for ppp in [1.0, 1.5, 2.0, 3.0] {
            // Past the ceiling, so `ladder` is clamping rather than following:
            // whatever this asks for, the raster stops at MAX_GLYPH_PX.
            let (raster, _) = crate::text::ladder(1e6, NAME_SIZE, ppp);
            for kind in [
                MarkKind::Minus,
                MarkKind::Plus,
                MarkKind::Septimal(true),
                MarkKind::Sharp,
                MarkKind::Flat,
            ] {
                let key = mark_key(kind, MARK_SIZE * raster, MARK_WEIGHT, ppp);
                let [w, _] = mark_geometry(key).1;
                assert!(
                    w as u32 <= crate::text::MARK_SHEET_WIDTH,
                    "{kind:?} at the raster ceiling is {w} texels wide, past the sheet's {}",
                    crate::text::MARK_SHEET_WIDTH,
                );
            }
        }
    }

    /// A piece never covers anything outside its own bounding box.
    ///
    /// The one thing [`Scanline`] rests on, and it is worth a test of its own
    /// because the way it fails is silent: a bound that is too tight drops a
    /// piece from the rows it really did reach, and the mark comes out with a
    /// bite missing rather than with an error. Swept over the whole box at a
    /// finer grid than the rasterizer's own samples, so a bound that is short
    /// by less than a pixel is still caught.
    #[test]
    fn a_piece_stays_inside_the_bounds_it_reports() {
        for kind in [
            MarkKind::Minus,
            MarkKind::Plus,
            MarkKind::Septimal(true),
            MarkKind::Sharp,
            MarkKind::Flat,
        ] {
            let key = mark_key(kind, 21.0, MARK_WEIGHT, 2.0);
            let (pieces, [w, h]) = mark_geometry(key);
            for piece in &pieces {
                let bounds = piece.bounds();
                for step_y in 0..=h * 8 {
                    for step_x in 0..=w * 8 {
                        let p = egui::pos2(step_x as f32 / 8.0, step_y as f32 / 8.0);
                        assert!(
                            !piece.covers(p) || bounds.contains(p),
                            "{kind:?} covers {p:?} outside its bounds {bounds:?}"
                        );
                    }
                }
            }
        }
    }

    /// A `♯` rasterizes to its own half turn.
    ///
    /// The sharp is NOT mirror-symmetric -- its uprights are staggered, one
    /// flush with each end of the box -- so the mirror this pins for the `+`
    /// says nothing here. Turned through half a circle it maps onto itself
    /// exactly: each upright onto the other, each bar onto the other. That is
    /// the whole of its geometry stated as one identity, and it fails if
    /// either the stagger or the bars stop being built about the centre.
    #[test]
    fn a_sharp_rasterizes_to_its_own_half_turn() {
        for size in [6.0_f32, 8.25, 13.0, 21.0] {
            let img = rasterize_mark(mark_key(MarkKind::Sharp, size, MARK_WEIGHT, 2.0));
            let [w, h] = img.size;
            for y in 0..h {
                for x in 0..w {
                    assert_eq!(
                        coverage(&img, x, y),
                        coverage(&img, w - 1 - x, h - 1 - y),
                        "the sharp should be its own half turn at {x},{y}, size {size}"
                    );
                }
            }
        }
    }

    /// A `♭` keeps an upright down its left edge and a bowl that stays OPEN.
    ///
    /// The bowl is the only stroked curve in the mark vocabulary, and the way
    /// it fails is by filling in: flatten the cubic too coarsely, or stroke it
    /// at a width the counter cannot survive, and the bowl becomes a blob that
    /// reads as a `b` at best and as a smudge at worst. So this pins the
    /// counter as a hole, not the outline as a shape.
    #[test]
    fn a_flat_keeps_its_bowl_open() {
        // From the size the analyzer's names set at on a Retina grid up to a
        // zoom that magnifies it twenty times.
        for size in [6.79_f32, 12.35, 33.0, 140.0] {
            let img = mark_ink(mark_key(MarkKind::Flat, size, MARK_WEIGHT, 2.0));
            let [w, h] = img.size;
            // The upright runs the whole height down the left edge.
            for y in 0..h {
                assert!(
                    coverage(&img, 0, y) > 0,
                    "the flat's upright should reach row {y} of {h} at size {size}"
                );
            }
            // The bowl reaches the right edge of the box, which is what its
            // stroke half-width past the curve's own bulge is cut to.
            assert!(
                (0..h).any(|y| coverage(&img, w - 1, y) > 0),
                "the bowl should reach the box's right edge at size {size}"
            );
            // And it is a counter, not a fill: some row through the bowl runs
            // ink, CLEAR, ink. Which row is left open, because at the smallest
            // size the whole counter is a pixel or two and the widest part of
            // the curve is not where it survives -- what has to hold is that
            // the bowl is a stroke around a hole at every size, not that the
            // hole is in the same place at all of them.
            let open = (0..h).any(|y| {
                let lit = |x: usize| coverage(&img, x, y) > 0;
                match ((0..w).find(|&x| lit(x)), (0..w).rev().find(|&x| lit(x))) {
                    (Some(first), Some(last)) => (first..last).any(|x| !lit(x)),
                    _ => false,
                }
            });
            assert!(open, "the flat's bowl should close around a counter at size {size}");
        }
    }

    /// A `♭`'s outline does not crease where the bowl lands on the foot.
    ///
    /// The way this fails is not a wrong shape but a wrong JOIN. Below
    /// [`FLAT_MERGE`] the outline is the foot's own straight run, and above it
    /// the bowl's outer edge; if the bowl arrives across that run rather than
    /// along it, the two meet at an angle and the eye reads a crease in the
    /// side of the mark. Area barely notices -- a bowl that bulges past the
    /// run and cuts back onto it is within a few percent of the right glyph on
    /// coverage -- so this is pinned on the OUTLINE instead.
    ///
    /// Below the merge every row is solid from the left edge (see
    /// `a_flats_stem_and_bowl_close_into_one_foot`), so a row's total coverage
    /// IS its right edge, to sub-pixel accuracy. Those edges have to fall on
    /// one line, and it has to be the foot's line: a bowl crossing the run
    /// steepens the fit measurably, which is the reading here.
    #[test]
    fn a_flats_outline_has_no_crease_where_the_bowl_lands() {
        // Per row of descent, which is size-independent: the ink box's own
        // aspect turns the foot's slope into pixels.
        let want = -FLAT_FOOT_SLOPE * FLAT_INK_W / FLAT_INK_H;
        for size in [33.0_f32, 60.0, 140.0] {
            let key = mark_key(MarkKind::Flat, size, MARK_WEIGHT, 2.0);
            let img = mark_ink(key);
            let [w, h] = img.size;
            let ink_h = FLAT_INK_H * key.size_px as f32;
            let merge = (h as f32 - ink_h) / 2.0 + FLAT_MERGE * ink_h;
            let edges: Vec<f32> = (merge.ceil() as usize..h)
                .map(|y| (0..w).map(|x| coverage(&img, x, y) as f32 / 255.0).sum())
                .collect();
            let n = edges.len() as f32;
            let mx = (n - 1.0) / 2.0;
            let my = edges.iter().sum::<f32>() / n;
            let sxy: f32 = edges.iter().enumerate().map(|(i, e)| (i as f32 - mx) * (e - my)).sum();
            let sxx: f32 = (0..edges.len()).map(|i| (i as f32 - mx).powi(2)).sum();
            let slope = sxy / sxx;
            assert!(
                (slope - want).abs() < 0.03 * want.abs(),
                "the outline below the merge falls {slope:.3}px a row against the foot's \
                 own {want:.3} at size {size} -- the bowl is crossing the run, not \
                 landing on it"
            );
            // And it is a line, not a curve that averages to one.
            let off = edges
                .iter()
                .enumerate()
                .map(|(i, e)| (e - (my + slope * (i as f32 - mx))).abs())
                .fold(0.0_f32, f32::max);
            assert!(
                off < 0.012 * w as f32,
                "the outline below the merge wanders {off:.2}px off its own line at size \
                 {size}, on a mark {w}px wide"
            );
        }
    }

    /// The `♭`'s bowl runs lighter than its stem, and still never goes under
    /// a physical pixel.
    ///
    /// Both halves matter and they pull against each other. The bowl is
    /// lighter because Iosevka's is ([`FLAT_BOWL_WEIGHT`]), and a bowl at the
    /// stem's weight closes the counter early. But the floor is why the mark
    /// is drawn rather than typeset at all (see [`mark_key`]), so scaling the
    /// bowl UNDER it would walk straight back into the sub-pixel stroke this
    /// whole path exists to escape. Below the size where 0.861 of a stroke is
    /// still a pixel, the two weights meet and the bowl is the stem's.
    #[test]
    fn a_flats_bowl_is_lighter_than_its_stem_and_never_under_a_pixel() {
        for size in [6.79_f32, 12.35, 33.0, 140.0] {
            let key = mark_key(MarkKind::Flat, size, MARK_WEIGHT, 2.0);
            let stem = key.weight_16 as f32 / 16.0;
            let (pieces, _) = mark_geometry(key);
            // Read off the geometry rather than the bitmap: a horizontal cut
            // through a stroke measures the stroke's own width only where it
            // crosses the row square, so pixels are the wrong instrument for a
            // ratio. What the pixels have to show is pinned by the three tests
            // around this one.
            let bars: Vec<f32> = pieces
                .iter()
                .filter_map(|p| match p {
                    MarkPiece::Bar(r) => Some(r.width()),
                    MarkPiece::Quad(_) => None,
                })
                .collect();
            assert_eq!(bars.len(), 1, "a flat has one straight piece, its stem");
            assert!(
                (bars[0] - stem).abs() < 1e-3,
                "the stem is {:.3}px, not the {stem:.3}px the key asked for, at size {size}",
                bars[0]
            );
            // Every quad past the foot is an arm of the bowl, and an arm's
            // width is the span across its own start.
            let want = (stem * FLAT_BOWL_WEIGHT).max(1.0);
            let arms: Vec<f32> = pieces
                .iter()
                .skip(2)
                .filter_map(|p| match p {
                    MarkPiece::Quad(c) => Some((c[0] - c[3]).length()),
                    MarkPiece::Bar(_) => None,
                })
                .collect();
            assert!(arms.len() >= 2, "a bowl is at least two arms, not {}", arms.len());
            for a in &arms {
                assert!(
                    (a - want).abs() < 1e-3,
                    "an arm of the bowl is {a:.3}px against the {want:.3}px it should carry \
                     at size {size}, where the stem is {stem:.3}px"
                );
                // The length is computed from corners, so it carries a little
                // float noise; a pixel short by 1e-7 is still a pixel.
                assert!(*a > 1.0 - 1e-3, "an arm is {a:.3}px at size {size}, under the floor");
            }
            if stem * FLAT_BOWL_WEIGHT >= 1.0 {
                assert!(want < stem, "the bowl should run lighter than the stem at size {size}");
            } else {
                assert_eq!(want, 1.0, "at the floor the bowl is one pixel, size {size}");
            }
        }
    }

    /// A `♭`'s stem and bowl are ONE mass where the face has them merged.
    ///
    /// The complement of the test above, and the failure it pins is the one
    /// that reads as a broken glyph rather than a wrong one: bring the two
    /// strokes down to the corner separately and the counter never closes, so
    /// the bowl reads as hung off the stem instead of joined to it. What
    /// makes that hard to catch by eye at the size names are drawn is that it
    /// is a sub-pixel gap -- the mark still looks like a `♭`, it just will not
    /// settle.
    ///
    /// So: the counter closes where [`FLAT_MERGE`] says the face closes it,
    /// every row below that is a single run of ink off the left edge, and the
    /// mark is already a MASS by then rather than two strokes in contact.
    #[test]
    fn a_flats_stem_and_bowl_close_into_one_foot() {
        for size in [6.79_f32, 12.35, 33.0, 140.0] {
            let key = mark_key(MarkKind::Flat, size, MARK_WEIGHT, 2.0);
            let img = mark_ink(key);
            let [w, h] = img.size;
            let thick = key.weight_16 as f32 / 16.0;
            let lit = |x: usize, y: usize| coverage(&img, x, y) > 0;
            // A row's ink as (first, last, unbroken), or None where it has none.
            let run = |y: usize| {
                let first = (0..w).find(|&x| lit(x, y))?;
                let last = (0..w).rev().find(|&x| lit(x, y))?;
                Some((first, last, (first..last).all(|x| lit(x, y))))
            };
            // The counter is the only hole in the mark, so the lowest row
            // with one is where it closes.
            let closes = (0..h)
                .rfind(|&y| matches!(run(y), Some((_, _, false))))
                .unwrap_or_else(|| panic!("a flat with no counter at all, size {size}"));
            // Never BELOW where the face closes it, which is the direction
            // that would mean the foot is eating a counter it should leave
            // open. Above is allowed and at the smallest sizes is what
            // happens: the hole runs out of pixels before it runs out of
            // glyph, which is what `a_flat_keeps_its_bowl_open` is for.
            let ink_h = FLAT_INK_H * key.size_px as f32;
            let merge = (h as f32 - ink_h) / 2.0 + FLAT_MERGE * ink_h;
            assert!(
                (closes as f32) <= merge + 1.0,
                "the counter closes at row {closes} of {h}, below the merge at {merge:.1}, \
                 size {size}"
            );
            for y in closes + 1..h {
                assert!(
                    matches!(run(y), Some((0, _, true)) | None),
                    "the flat's foot breaks in two at row {y} of {h}, size {size}"
                );
            }
            // A mass, not a touch: where the two meet, the mark is over twice
            // the stroke across.
            let (first, last, _) = run(merge as usize).expect("the merge row has ink");
            let across = last + 1 - first;
            assert!(
                across as f32 > 2.0 * thick,
                "the foot is {across}px across where the two meet, against a {thick:.1}px \
                 stroke at size {size} -- they are touching, not merged"
            );
        }
    }

    /// Coverage as a plain alpha grid, whatever drew it -- a mark's bitmap or
    /// a glyph's cell in egui's atlas, so the two can be read the same way.
    struct Grid {
        w: usize,
        h: usize,
        a: Vec<f32>,
    }

    impl Grid {
        /// The same coverage drawn one pixel along at sub-pixel phase `t`,
        /// which is what LINEAR sampling of a quad at its own bitmap's size
        /// does -- a blend of the two texels either side of each pixel.
        fn shifted(&self, t: f32) -> Grid {
            let at = |x: isize, y: usize| {
                if x < 0 || x >= self.w as isize {
                    0.0
                } else {
                    self.a[y * self.w + x as usize]
                }
            };
            let mut a = Vec::with_capacity((self.w + 2) * self.h);
            for y in 0..self.h {
                for x in 0..self.w as isize + 2 {
                    a.push(at(x - 2, y) * t + at(x - 1, y) * (1.0 - t));
                }
            }
            Grid { w: self.w + 2, h: self.h, a }
        }

        /// How much of this mark's weight breathes as it slides: the swing in
        /// `sum a(1-a)` across a walk of one pixel in sixteenths, as a
        /// fraction of that sum's own maximum.
        ///
        /// Issue #292's reading, and its normalisation as well as its sum.
        /// The sum is maximal at half coverage and zero for a pixel that is
        /// either ink or nothing, so its swing is the symbol's weight visibly
        /// changing as it slides; against its own peak, that is the share of
        /// the softness that is moving rather than sitting still.
        ///
        /// Not divided by the mark's ink, which is the reading that suggests
        /// itself and is the wrong one here: the drawn mark and the type do
        /// not carry the same ink (19.3 against 26.5 for `♭` at the size
        /// below), so per-ink flatters whichever is heavier and answers a
        /// question about weight rather than about shimmer.
        fn breathing(&self) -> f32 {
            let (mut lo, mut hi) = (f32::MAX, 0.0f32);
            for step in 0..16 {
                let shifted = self.shifted(step as f32 / 16.0);
                let smear: f32 = shifted.a.iter().map(|a| a * (1.0 - a)).sum();
                lo = lo.min(smear);
                hi = hi.max(smear);
            }
            (hi - lo) / hi.max(1e-6)
        }
    }

    fn drawn_coverage(kind: MarkKind, size: f32, ppp: f32) -> Grid {
        let img = rasterize_mark(mark_key(kind, size, MARK_WEIGHT, ppp));
        Grid {
            w: img.size[0],
            h: img.size[1],
            a: img.pixels.iter().map(|p| p.a() as f32 / 255.0).collect(),
        }
    }

    /// The same character as egui's own atlas rasterizes it, at the same size.
    fn typeset_coverage(ch: char, size: f32, ppp: f32) -> Grid {
        let ctx = crate::tests::probe::themed_at(ppp);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let galley = ctx.fonts_mut(|f| {
            f.layout_no_wrap(ch.to_string(), egui::FontId::monospace(size), egui::Color32::WHITE)
        });
        let cell = galley.rows[0].glyphs[0].uv_rect;
        ctx.fonts(|f| {
            let atlas = f.image();
            // Off `min`/`max`, which are TEXELS. `UvRect::size` is the glyph's
            // size in POINTS, and taking the cell from it reads a fraction of
            // the glyph -- a quarter of it at ppp 2 -- which is a crop that
            // looks like a plausible bitmap and measures nothing.
            let w = (cell.max[0] - cell.min[0]) as usize;
            let h = (cell.max[1] - cell.min[1]) as usize;
            let mut a = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let at = (cell.min[1] as usize + y) * atlas.size[0] + cell.min[0] as usize + x;
                    a.push(atlas.pixels[at].a() as f32 / 255.0);
                }
            }
            Grid { w, h, a }
        })
    }

    /// The accidentals breathe LESS drawn than they did typeset, which is the
    /// whole reason they are drawn.
    ///
    /// Issue #292 measured the symptom on the type: at the size the analyzer
    /// sets its names, essentially every lit pixel of a `♭` is partial
    /// coverage, so nothing in the symbol is decided by anything but
    /// sub-pixel phase and the whole mark shimmers rather than its outline.
    /// The reading here is the SWING rather than that count, because the
    /// count is what a drawn mark shares -- see [`mark_key`].
    ///
    /// Both paths are read HERE, in one test, off the same walk -- rather
    /// than the drawn one being pinned against a number copied out of that
    /// issue. A number would only say the mark is as good as the type was on
    /// the machine that measured it; taking both means the claim is the
    /// comparison itself, and it cannot go stale as the face, the size or the
    /// rasterizer move under it.
    ///
    /// At the size asserted here the readings are 30.8% against 49.0% for
    /// `♭` and 29.4% against 40.8% for `♯` -- a real margin rather than a
    /// hair either side of equal. The drawn `-` beside them sits at 4.3%,
    /// which is what a mark made of one straight bar can reach and neither
    /// accidental can.
    ///
    /// SMALL sizes, and only those, which is where the complaint was: a name
    /// scrolling slowly is where sub-pixel phase is watchable. At three times
    /// this size the `♯` still gains (37.9% against 52.9%) and the `♭` is a
    /// wash (40.6% either way) -- the type's strokes are over a pixel
    /// there, so the floor that makes the drawn mark different has nothing
    /// left to do. Asserting across the range would be asserting something
    /// this change does not claim.
    #[test]
    fn a_drawn_accidental_breathes_less_than_the_type_it_replaced() {
        // The analyzer's own mark size, on a Retina grid: `names::LABEL_PT`
        // through MARK_SCALE, which is where the symptom was reported.
        let size = 12.35 * MARK_SCALE;
        for (ch, kind) in [('\u{266D}', MarkKind::Flat), ('\u{266F}', MarkKind::Sharp)] {
            let typeset = typeset_coverage(ch, size, 2.0).breathing();
            let drawn = drawn_coverage(kind, size, 2.0).breathing();
            assert!(
                drawn < typeset,
                "{ch} breathes {:.1}% of its ink drawn against {:.1}% typeset -- \
                 drawing it is supposed to be the quieter of the two",
                100.0 * drawn,
                100.0 * typeset,
            );
        }
    }

    /// How a roll-sized mark reads on screen as it slides, and what the
    /// fill's reconstruction filter does to that.
    ///
    /// A MODEL of `text.wgsl` rather than the shader itself: it walks the
    /// same bitmaps through the same ring arithmetic in `f32`, which is what
    /// lets one run compare two filters against each other. The shader's own
    /// behaviour is pinned on the GPU, by `harmonigraph_render`'s
    /// `a_sliding_hairline_keeps_its_weight`. What lives here is the reason
    /// that filter has the shape it does, read off the marks that asked for
    /// it -- a claim the hairline fixture cannot make, since it is a stroke
    /// and these are symbols.
    mod sliding {
        use super::*;

        /// The halo as `crate::text::RINGS` sets it, in PHYSICAL pixels at
        /// ppp 2: (radius, per-stamp alpha, samples), outer ring first.
        const RINGS_PX: [(f32, f32, u32); 2] = [(4.0, 0.21, 8), (2.0, 1.0, 12)];

        /// `text.wgsl`'s `FILTER_TAP` laid along x, and the single tap it is
        /// read against. Mirrored rather than shared -- nothing links a WGSL
        /// constant to a Rust one, so the shader is where the value lives,
        /// this follows it, and the NAME is the whole of the thread between
        /// them.
        ///
        /// The AXIS is this module's own rather than the shader's. There the
        /// offset is `FILTER_TAP * locals.filter_axis`, a direction each pane
        /// chooses (`harmonigraph_render::SlideAxis`); here [`walk`] slides in
        /// x, so x is the axis to model it on. The two agree on what the
        /// filter is worth, since bilinear is separable and a pane sliding
        /// along its own named axis is this case turned to match. What this
        /// module therefore does NOT read is a mark travelling across the
        /// filter's axis, which is a whole different quantity and is
        /// `harmonigraph_render`'s to hold.
        const TWO_TAP: [(f32, f32); 2] = [(-0.25, 0.0), (0.25, 0.0)];
        const ONE_TAP: [(f32, f32); 1] = [(0.0, 0.0)];

        /// The size the spectral roll sets its marks at, in points: scale 1
        /// through the ladder anchored at `names::LABEL_PT`, which is where
        /// the shimmer was reported.
        fn roll_mark_size(ppp: f32) -> f32 {
            let (raster, _) = crate::text::ladder(1.0, 12.35, ppp);
            12.35 * raster * MARK_SCALE
        }

        /// A mark bitmap, and the screen it is sampled onto.
        struct Sheet(Grid);

        impl Sheet {
            /// One bilinear tap, addressed in screen pixels from the bitmap's
            /// own origin -- texels sit 1:1 with pixels, which is the whole
            /// arrangement `mark_key` and the size ladder exist to keep.
            /// Outside the bitmap reads as nothing, which [`MARK_BITMAP_PAD`]
            /// already guarantees the ink never needs.
            fn tap(&self, x: f32, y: f32) -> f32 {
                let (x0, y0) = (x.floor(), y.floor());
                let (fx, fy) = (x - x0, y - y0);
                let at = |ix: f32, iy: f32| {
                    if ix < 0.0 || iy < 0.0 || ix >= self.0.w as f32 || iy >= self.0.h as f32 {
                        0.0
                    } else {
                        self.0.a[iy as usize * self.0.w + ix as usize]
                    }
                };
                at(x0, y0) * (1.0 - fx) * (1.0 - fy)
                    + at(x0 + 1.0, y0) * fx * (1.0 - fy)
                    + at(x0, y0 + 1.0) * (1.0 - fx) * fy
                    + at(x0 + 1.0, y0 + 1.0) * fx * fy
            }

            /// Coverage through a filter given as tap offsets, averaged --
            /// which is `fs_fill`.
            fn cov(&self, x: f32, y: f32, taps: &[(f32, f32)]) -> f32 {
                taps.iter().map(|(dx, dy)| self.tap(x + dx, y + dy)).sum::<f32>()
                    / taps.len() as f32
            }

            /// And `fs_rim`: `1 - PRODUCT(1 - alpha * coverage)` over each
            /// ring's stamps, the outer ring accumulated first. Its stamps
            /// read through the filter the fill does, which is the shader --
            /// and `rim_taps` is separate here only so a test can hold one
            /// pass still and vary the other.
            fn rim(&self, x: f32, y: f32, rim_taps: &[(f32, f32)]) -> f32 {
                let mut acc = 0.0;
                for (radius, alpha, samples) in RINGS_PX {
                    let mut open = 1.0 - acc;
                    for i in 0..samples {
                        let angle = std::f32::consts::TAU * i as f32 / samples as f32;
                        let (ox, oy) = (angle.cos() * radius, angle.sin() * radius);
                        open *= 1.0 - alpha * self.cov(x - ox, y - oy, rim_taps);
                    }
                    acc = 1.0 - open;
                }
                acc
            }
        }

        /// What one sub-pixel phase of the walk puts on screen.
        struct Reading {
            /// Total alpha of the composite -- the halo with the fill over
            /// it. A bilinear resample conserves the fill's own sum exactly,
            /// so a swing here is the RING arithmetic's nonlinearity and
            /// nothing else: the mark pulsing rather than resampling.
            weight: f32,
            /// `sum a(1-a)` over the composite: how much of what is drawn is
            /// partial coverage, which is the quantity that varies with phase
            /// while the ink itself does not.
            smear: f32,
            /// The fill's own ink, constant across the walk, and so the
            /// denominator that makes two symbols comparable.
            ink: f32,
            /// The darkest the fill reaches -- the check that a filter is not
            /// merely blurring the mark until its swing has nothing to swing.
            peak: f32,
        }

        /// Walk one pixel in sixteenths, reading the composite at each phase.
        fn walk(sheet: &Sheet, taps: &[(f32, f32)], rim_taps: &[(f32, f32)]) -> Vec<Reading> {
            // The halo reaches four pixels past the ink, the filter a quarter
            // past that, and the phase up to one more. Six covers a MARK,
            // whose bitmap carries its own clear border (`MARK_BITMAP_PAD`).
            // A typeset cell is egui's exact glyph box with no border at all,
            // so its ink can sit on the edge and the rim's support runs a
            // fraction past this window -- which is why what is read off type
            // here is `peak`, the fill alone and nowhere near the edge, while
            // `weight` and `smear` are asserted on mark bitmaps only.
            let pad = 6.0;
            (0..16)
                .map(|step| {
                    let t = step as f32 / 16.0;
                    let mut r = Reading { weight: 0.0, smear: 0.0, ink: 0.0, peak: 0.0 };
                    let mut y = -pad;
                    while y < sheet.0.h as f32 + pad {
                        let mut x = -pad;
                        while x < sheet.0.w as f32 + pad {
                            let fill = sheet.cov(x - t, y, taps);
                            let a = fill + sheet.rim(x - t, y, rim_taps) * (1.0 - fill);
                            r.weight += a;
                            r.smear += a * (1.0 - a);
                            r.ink += fill;
                            r.peak = r.peak.max(fill);
                            x += 1.0;
                        }
                        y += 1.0;
                    }
                    r
                })
                .collect()
        }

        /// A reading's swing across the walk, as a share of the symbol's own
        /// ink: how much of what is drawn changes appearance between one
        /// phase and the next.
        ///
        /// Per INK, and the choice matters. [`Grid::breathing`] normalises a
        /// swing against its own peak, which answers "how much of this
        /// symbol's softness is moving" -- the right question for one symbol
        /// drawn two ways, and the wrong one for two symbols side by side. On
        /// its reading the LETTERS score worse than the accidentals at roll
        /// size (43.7% for `B` against 30.8% for the flat), which is the
        /// reverse of what is watchable, because a letter's swing is spread
        /// over an ink four times the size and its halo saturates into a
        /// plateau besides. Per ink the order comes out as reported: the flat
        /// at 12.6%, the sharp at 6.6%, `B` at 1.7%.
        fn per_ink(readings: &[Reading], of: impl Fn(&Reading) -> f32) -> f32 {
            let (mut lo, mut hi) = (f32::MAX, 0.0f32);
            for r in readings {
                lo = lo.min(of(r));
                hi = hi.max(of(r));
            }
            (hi - lo) / readings[0].ink.max(1e-6)
        }

        fn sheet_of(img: &egui::ColorImage) -> Sheet {
            Sheet(Grid {
                w: img.size[0],
                h: img.size[1],
                a: img.pixels.iter().map(|p| p.a() as f32 / 255.0).collect(),
            })
        }

        /// The accidentals hold their weight better through two taps than
        /// through the one they were drawn against.
        ///
        /// The complaint this answers is theirs specifically: a name gliding
        /// across the roll is quiet in its letters and shimmers in its
        /// accidental. Both marks are read here off the same walk, through
        /// both filters, rather than either being pinned to a number -- so
        /// the claim is the COMPARISON, and it cannot go stale as the face,
        /// the size or the mark designs move under it.
        ///
        /// Measured, the flat falls from 12.6% of its ink to 4.2% and the
        /// sharp from 6.6% to 1.5% -- a third of what it was, either way,
        /// against a bound of half that leaves room for the designs to shift
        /// without the test becoming a tripwire for something it is not
        /// about.
        ///
        /// Why the accidentals and not the `-` beside them: read against
        /// motion ACROSS the pane, these are the marks whose ink is mostly
        /// vertical strokes, which redistribute across columns as they slide.
        /// A horizontal bar has almost no vertical edge to redistribute, and
        /// sits at 2.5% before any of this.
        ///
        /// Across the pane is the default orientation's motion and not the
        /// only one -- the roll runs time down the pane in two of its four,
        /// and there it is the `♯`'s two horizontal bars that redistribute
        /// while its uprights sit still. The filter turns to follow the pane
        /// (`harmonigraph_render::SlideAxis`), so what that mirror case reads
        /// is this same number with the symbols exchanged, and the reading
        /// that holds it is the vertical half of
        /// `a_sliding_hairline_keeps_its_weight`.
        #[test]
        fn two_taps_calm_the_accidentals_that_one_could_not() {
            const PPP: f32 = 2.0;
            let size = roll_mark_size(PPP);
            for (what, kind) in [("flat", MarkKind::Flat), ("sharp", MarkKind::Sharp)] {
                let sheet = sheet_of(&rasterize_mark(mark_key(kind, size, MARK_WEIGHT, PPP)));
                let one = per_ink(&walk(&sheet, &ONE_TAP, &ONE_TAP), |r| r.smear);
                let two = per_ink(&walk(&sheet, &TWO_TAP, &TWO_TAP), |r| r.smear);
                assert!(
                    two < one * 0.5,
                    "the {what} swings {:.1}% of its ink through two taps against {:.1}% \
                     through one -- the wider filter is supposed to be the calmer of them",
                    100.0 * two,
                    100.0 * one,
                );
            }
        }

        /// And the type beside them pays nothing for it.
        ///
        /// The filter is one path: widening it for the marks widens it for
        /// every letter in the app. That is affordable only because type at
        /// these sizes has strokes over a pixel, where a quarter-texel spread
        /// falls inside ink that is already opaque -- so the darkest pixel of
        /// a letter is the same darkest pixel it was. A pair of taps up and
        /// down as well would NOT be affordable: it takes the digits to 0.86
        /// and the flat to 0.74, which is the trade this shape of filter
        /// exists to avoid.
        #[test]
        fn the_type_beside_them_keeps_its_contrast() {
            const PPP: f32 = 2.0;
            for (what, ch, size) in [
                ("the roll's letter", 'B', 12.35),
                ("the roll's letter", 'C', 12.35),
                ("a count digit", '2', 12.35 * MARK_SCALE),
                ("a count digit", '3', 12.35 * MARK_SCALE),
                ("the lattice's letter", 'B', NAME_SIZE),
            ] {
                let sheet = Sheet(typeset_coverage(ch, size, PPP));
                let one = walk(&sheet, &ONE_TAP, &ONE_TAP);
                let two = walk(&sheet, &TWO_TAP, &TWO_TAP);
                let peak = |r: &[Reading]| r.iter().map(|r| r.peak).fold(0.0, f32::max);
                // The floor first, because everything below it is a RELATIVE
                // reading and an empty grid satisfies every one of them: a
                // face that failed to load, or a `uv_rect` that came back
                // empty, would leave both peaks at zero and report contrast
                // preserved while measuring nothing. Type at these sizes has
                // strokes over a pixel, so a glyph that draws at all reaches
                // very near opaque somewhere.
                assert!(
                    peak(&one) > 0.9,
                    "{what} `{ch}` at {size}pt peaks at only {:.2} before any of this -- \
                     the fixture is not drawing the glyph it says it is",
                    peak(&one),
                );
                assert!(
                    peak(&two) >= peak(&one) - 0.02,
                    "{what} `{ch}` at {size}pt peaks at {:.2} through two taps against {:.2} \
                     through one: the filter is costing type its contrast",
                    peak(&two),
                    peak(&one),
                );
            }
        }

        /// The count digits are NOT what is left breathing -- issue #304's
        /// last suspect, measured rather than reasoned about.
        ///
        /// The suspicion was specific and worth taking seriously: a count digit
        /// is set at [`MARK_SIZE`], where Iosevka's 70/1000 em stroke is under
        /// a physical pixel at ppp 2 -- the same regime the drawn signs were
        /// pulled out of by [`mark_key`]'s whole-pixel floor. What answers it
        /// is that a digit is not a stroke: it carries a bowl or a stem several
        /// times that width, so its darkest pixel is opaque where a bar's is
        /// not, and it has an interior that no sub-pixel phase can empty.
        ///
        /// Read as the composite through the filter the tree ships, which is
        /// the only reading that says what a viewer sees today, and against the
        /// marks beside it in the same column -- a digit and the sign it counts
        /// are drawn together, so the sign is the fair bar. Every digit comes
        /// in under it.
        ///
        /// A typeset cell is read here exactly as a mark bitmap is, with no
        /// padding between them, and that IS the equal footing rather than a
        /// gap in it. A mark carries a clear border ([`MARK_BITMAP_PAD`]) and an
        /// atlas cell is the glyph box exactly, so the two look asymmetric --
        /// but [`Sheet::tap`] reads outside either grid as nothing, which is the
        /// clear space a glyph on a pane has around it, and [`walk`]'s window
        /// already runs six pixels past both. Padding a grid here adds only
        /// samples that contribute zero to `weight`, `smear` and `ink` alike:
        /// measured, every digit below reads bit-identically with six pixels of
        /// margin added. Worth stating because the asymmetry looks like it must
        /// matter, and a pad put in to answer it would be twenty lines that
        /// change no number.
        #[test]
        fn a_count_digit_breathes_less_than_the_sign_it_counts() {
            const PPP: f32 = 2.0;
            let size = roll_mark_size(PPP);
            let composite =
                |grid: Grid| per_ink(&walk(&Sheet(grid), &TWO_TAP, &TWO_TAP), |r| r.smear);
            let sign = |kind| {
                composite(sheet_of(&rasterize_mark(mark_key(kind, size, MARK_WEIGHT, PPP))).0)
            };
            // The louder of the two signs a count ever sits beside.
            let bar = sign(MarkKind::Flat).max(sign(MarkKind::Sharp));
            for digit in '0'..='9' {
                let swing = composite(typeset_coverage(digit, size, PPP));
                assert!(
                    swing <= bar,
                    "the count digit `{digit}` swings {:.1}% of its ink at {size:.2}pt against \
                     {:.1}% for the sign it is counted beside -- the digits would then be the \
                     shimmer that is left, which is issue #304's suspicion and not its finding",
                    100.0 * swing,
                    100.0 * bar,
                );
                println!("digit {digit}: {:.2}% (sign bar {:.2}%)", 100.0 * swing, 100.0 * bar);
            }
        }

        /// The swing is the RIM's, not the fill's -- which is why the filter
        /// lives in `coverage`, where both passes read it, and not in
        /// `fs_fill` where it looks like it belongs.
        ///
        /// This is the reading that decides the shape of the change, and it
        /// is worth pinning because it is genuinely surprising: the fill is
        /// the ink, the ink is what shimmers, and the obvious fix is to
        /// filter the ink. Measured on the flat at roll size, widening the
        /// fill alone takes its OWN swing from 15.9% of its weight to 4.0%
        /// and moves the composite from 12.6% to 12.5% -- which is to say it
        /// fixes the thing being measured and not the thing being seen.
        ///
        /// The halo is why. It is a dilation of the same sub-pixel stroke
        /// through `1 - PRODUCT(1 - a)`, it covers several times the area the
        /// ink does, and being nonlinear it does not conserve what a resample
        /// hands it. Its total WEIGHT is steady all the same -- under 2%
        /// across the walk, asserted below -- so what moves is where the
        /// halo's darkness sits rather than how much of it there is: an
        /// outline crisping and softening around a mark, once per pixel of
        /// travel.
        ///
        /// Widening the rim is what costs something real, twenty stamps a
        /// fragment becoming forty taps, so the case for spending it is
        /// exactly this measurement.
        #[test]
        fn the_swing_is_the_rims_and_not_the_fills() {
            const PPP: f32 = 2.0;
            let size = roll_mark_size(PPP);
            for (what, kind) in [("flat", MarkKind::Flat), ("sharp", MarkKind::Sharp)] {
                let sheet = sheet_of(&rasterize_mark(mark_key(kind, size, MARK_WEIGHT, PPP)));
                let composite = |fill, rim| per_ink(&walk(&sheet, fill, rim), |r| r.smear);
                let before = composite(&ONE_TAP, &ONE_TAP);
                let fill_only = composite(&TWO_TAP, &ONE_TAP);
                let rim_only = composite(&ONE_TAP, &TWO_TAP);
                assert!(
                    fill_only > before * 0.8,
                    "the {what}'s composite swings {:.1}% of its ink with the fill widened \
                     alone, against {:.1}% with neither -- if the fill carried this, the \
                     rim would not need the taps it is being given",
                    100.0 * fill_only,
                    100.0 * before,
                );
                assert!(
                    rim_only < before * 0.5,
                    "the {what}'s composite swings {:.1}% with the rim widened alone, \
                     against {:.1}% with neither: the rim is supposed to be where this is",
                    100.0 * rim_only,
                    100.0 * before,
                );
            }
        }

        /// The mark does not get heavier and lighter as it slides, at either
        /// filter -- what moves is how its weight is spread.
        ///
        /// A bilinear resample conserves the fill's own ink exactly, so this
        /// reads the composite, where the rim's `1 - PRODUCT(1 - a)` is free
        /// to invent and destroy weight. It does neither, to within 2%, and
        /// that is what makes the swing above a matter of distribution rather
        /// than of a symbol pulsing between two weights.
        #[test]
        fn a_sliding_mark_keeps_its_weight_whatever_the_filter() {
            const PPP: f32 = 2.0;
            let size = roll_mark_size(PPP);
            let column = [
                MarkKind::Flat,
                MarkKind::Sharp,
                MarkKind::Minus,
                MarkKind::Plus,
                MarkKind::Septimal(true),
            ];
            for kind in column {
                let sheet = sheet_of(&rasterize_mark(mark_key(kind, size, MARK_WEIGHT, PPP)));
                for (what, taps) in [("one tap", &ONE_TAP[..]), ("two", &TWO_TAP[..])] {
                    let readings = walk(&sheet, taps, taps);
                    let (mut lo, mut hi) = (f32::MAX, 0.0f32);
                    for r in &readings {
                        lo = lo.min(r.weight);
                        hi = hi.max(r.weight);
                    }
                    assert!(
                        (hi - lo) / hi < 0.02,
                        "{kind:?} weighs between {lo:.2} and {hi:.2} across one pixel of \
                         travel through {what}: the mark is pulsing, not resampling",
                    );
                }
            }
        }
    }

    /// A mark's INK advances with the quad that carries it, instead of
    /// lurching about inside it.
    ///
    /// [`a_drawn_accidental_breathes_less_than_the_type_it_replaced`] reads
    /// how much a mark's ink varies in AMOUNT as it slides; this reads where
    /// that ink is. They are different complaints and they had different
    /// causes: breathing is the stroke's sub-pixel width, and this is the
    /// quad's own bound. A mark can be perfectly steady in weight while
    /// stepping a pixel back and forth across its own motion, which is what
    /// the accidentals did on the roll.
    ///
    /// Read as the centre of MASS, sampled through the quad the way the GPU
    /// samples it -- bilinear, and only at pixel centres that fall inside the
    /// quad. That last part is the whole mechanism: a centre a hair outside
    /// takes nothing at all, so ink reaching the bitmap's edge is dropped and
    /// picked up once per pixel of travel, and the mark's apparent position
    /// jumps by the weight of a whole column. The centroid is what sees it --
    /// the quad's own corners are an exact straight line either way, and
    /// per-pixel alpha deltas read as smooth motion, so both miss it.
    ///
    /// Walked at the roll's own 1.4286 physical pixels a frame, which is
    /// deliberately not a whole one: at a whole pixel a step and a straight
    /// line are the same picture, and every phase would be the same phase.
    ///
    /// Every mark, not just the two that were reported: the bound belongs to
    /// [`paint_mark`]'s quad and reaches all of them, and an accidental is
    /// only where it shows worst. Measured, the spread is 0.025 pixels for
    /// every kind, against 1.27 for a `♭` and 0.70 for a `♯` drawn to a
    /// bitmap with no margin -- which is a mark lurching a pixel around its
    /// own motion. The bound here is far under the broken reading and well
    /// over the fixed one.
    #[test]
    fn a_sliding_marks_ink_advances_with_the_quad_that_carries_it() {
        const PPP: f32 = 2.0;
        const RATE: f32 = 1.4286;

        let ctx = crate::tests::probe::themed_at(PPP);
        let screen = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 400.0));
        // The analyzer's own label setting, split the way `names::draw` splits
        // it: the rung a mark is rasterized on, and the rest as magnification.
        let (raster, magnify) = crate::text::ladder(1.0, 12.35, PPP);
        let scale = 12.35 * raster / NAME_SIZE;

        let bare = |sharps, syntonic_commas, septimal_commas| harmonigraph_core::NoteName {
            letter: 'D',
            sharps,
            syntonic_commas,
            septimal_commas,
        };
        for (what, kind, name) in [
            ("flat", MarkKind::Flat, bare(-1, 0, 0)),
            ("sharp", MarkKind::Sharp, bare(1, 0, 0)),
            ("minus", MarkKind::Minus, bare(0, -1, 0)),
            ("plus", MarkKind::Plus, bare(0, 1, 0)),
            ("septimal", MarkKind::Septimal(true), bare(0, 0, 1)),
        ] {
            let bitmap = rasterize_mark(mark_key(kind, MARK_SIZE * scale, MARK_WEIGHT, PPP));
            let centroids: Vec<f32> = (0..40)
                .map(|frame| {
                    let anchor = egui::pos2(100.0 + frame as f32 * RATE / PPP, 100.0);
                    let mut batch = crate::text::TextBatch::default();
                    let _ = ctx.run_ui(
                        egui::RawInput { screen_rect: Some(screen), ..Default::default() },
                        |ui| {
                            draw_stacked_name(
                                &mut batch,
                                ui.painter(),
                                anchor,
                                name,
                                egui::Color32::WHITE,
                                egui::Color32::TRANSPARENT,
                                scale,
                                magnify,
                                // The reading is about how the ink MOVES, so
                                // the anchor is a place to move it from.
                                NameLead::Centred,
                            );
                        },
                    );
                    let [quad] = batch.marks()[..] else {
                        panic!("{what} should draw one mark, got {:?}", batch.marks())
                    };
                    ink_centroid(&bitmap, quad, PPP)
                })
                .collect();

            let steps: Vec<f32> = centroids.windows(2).map(|w| w[1] - w[0]).collect();
            let lo = steps.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = steps.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            assert!(
                hi - lo < 0.15,
                "{what}'s ink advances between {lo:.4} and {hi:.4} pixels a frame against a \
                 quad that advances by {RATE} every time: the mark steps inside its own quad",
            );
        }
    }

    /// Where a mark's ink sits on screen, in physical pixels: its bitmap
    /// sampled through `quad` the way the GPU samples it, reduced to a centre
    /// of mass.
    ///
    /// Bilinear against clamped taps, and a pixel counts only if its CENTRE
    /// falls inside the quad -- which is rasterization, not an approximation
    /// of it, for the axis-aligned quad a mark is drawn as.
    ///
    /// The quad the glyph pass actually rasterizes is GROWN by the rim's reach
    /// on every side, and reading only the mark's own box is deliberate: past
    /// it the shader is painting the halo, and the halo is not what a claim
    /// about the mark's position is about. What is left out is the half-texel
    /// margin the fill reads past its patch, which on a padded bitmap is
    /// transparent either way.
    fn ink_centroid(img: &egui::ColorImage, quad: egui::Rect, ppp: f32) -> f32 {
        let [w, h] = img.size;
        let tap = |x: isize, y: isize| {
            let (x, y) = (x.clamp(0, w as isize - 1), y.clamp(0, h as isize - 1));
            img.pixels[y as usize * w + x as usize].a() as f32
        };
        let (x0, x1) = (quad.left() * ppp, quad.right() * ppp);
        let (y0, y1) = (quad.top() * ppp, quad.bottom() * ppp);
        let (mut mass, mut moment) = (0.0f64, 0.0f64);
        for py in (y0.floor() as i32)..=(y1.ceil() as i32) {
            for px in (x0.floor() as i32)..=(x1.ceil() as i32) {
                let (cx, cy) = (px as f32 + 0.5, py as f32 + 0.5);
                if cx < x0 || cx > x1 || cy < y0 || cy > y1 {
                    continue;
                }
                // The quad's own uv runs 0..1, so a pixel centre maps to a
                // texel coordinate and the tap sits half a texel back of it.
                let tx = (cx - x0) / (x1 - x0) * w as f32 - 0.5;
                let ty = (cy - y0) / (y1 - y0) * h as f32 - 0.5;
                let (ix, iy) = (tx.floor(), ty.floor());
                let (fx, fy) = (tx - ix, ty - iy);
                let (ix, iy) = (ix as isize, iy as isize);
                let a = tap(ix, iy) * (1.0 - fx) * (1.0 - fy)
                    + tap(ix + 1, iy) * fx * (1.0 - fy)
                    + tap(ix, iy + 1) * (1.0 - fx) * fy
                    + tap(ix + 1, iy + 1) * fx * fy;
                mass += f64::from(a);
                moment += f64::from(a) * f64::from(cx);
            }
        }
        assert!(mass > 0.0, "a mark that samples to nothing has no position");
        (moment / mass) as f32
    }
}
