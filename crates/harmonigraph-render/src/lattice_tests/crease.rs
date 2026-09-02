//! The two-distance rule of #568 §2 as arithmetic, on analytic ink: what a
//! Distance shadow reads where two pieces of one caster face each other.
//!
//! A distance shadow spends `p(d) = exp(-SHADOW_TAIL·d/w)` of the distance to
//! the NEAREST ink, and nearest is a `min`, so between two facing pieces the
//! second one contributes nothing and the medial axis carries a crease. The
//! rule unions a SECOND distance — the caster's next feature, weighted by how
//! squarely its foot faces the point — and carries the pair as one effective
//! distance, so the consumer still reads one exponential.
//!
//! No GPU and no shader: the rule's arithmetic is
//! [`harmonigraph_scene::crease`] and what this module adds is the ink to
//! evaluate it over. It is the oracle the producers are written against, so the
//! ramp that decides how squarely a foot has to face is settled against
//! measured shapes before a pipeline is touched.

use glam::Vec2;
use harmonigraph_scene::crease::{
    facing, facing_at, smoothstep, spend, standoff_coverage, union_distance,
};
use harmonigraph_scene::{BEYOND_RAMP, SHADOW_TAIL};

/// The Shadow width every reading here is taken at, in points.
///
/// Ten, so the ring's 2.5 point gap is a quarter of it and a pair facing across
/// half of it lands where the exponential is still steep: the readings sit in
/// the part of the curve a crease is visible in.
const W: f32 = 10.0;

/// The grid pitch a second difference is taken on, in points: the cell a
/// Distance shadow is drawn into at the renderer's quality floor.
const PX: f32 = 0.25;

/// The pitch a midline sweep is read at, in points — fine enough that a step in
/// the field reads as a step rather than as the sweep's own slope.
const SWEEP: f32 = 0.05;

/// What a smooth field's second difference comes to at [`PX`]: the curvature of
/// `p` itself, `(SHADOW_TAIL·PX/W)²`, at coverage 1.
///
/// A field that only decays cannot beat it, so it is the bound a filled crease
/// has to reach; a crease or a step reads multiples of it.
const SMOOTH: f32 = (SHADOW_TAIL * PX / W) * (SHADOW_TAIL * PX / W);

/// How near the ink a second difference is still taken, in points.
///
/// A distance field's own curvature next to a convex corner is `1/d`, which
/// runs away as the corner is approached — §9's "ink-adjacent pixels
/// legitimately reach ~0.04". A clearance keeps the measurement on the part of
/// the field the rule is answerable for, and one point still reaches the mouth
/// of the ring's gap, where the nearest ink stands 1.8 points off.
const CLEARANCE: f32 = 1.0;

// ---------------------------------------------------------------- the ink

/// One piece of ink, which answers with exactly ONE nearest point.
///
/// Per feature and not per boundary point, which is #568 §2's second fact: a
/// wall's own neighbouring points are legitimately beyond the plane and would
/// add a curvature term inside every bowl, and boundary samples along one face
/// leak through the plane test as false second features.
#[derive(Clone, Copy)]
enum Feature {
    /// An oriented rectangle: `half` extents along `axis` and its perpendicular.
    Bar { centre: Vec2, axis: Vec2, half: Vec2 },
    /// A gap-cut annular sector, `annular_sector_distance`'s geometry: the
    /// annulus between `inner` and `outer`, cut by two lines parallel to the
    /// wedge's own edges at `mid ± half` and standing `cut` in from them, so
    /// two slices of one pitch leave a band `2·cut` wide between them.
    Sector { inner: f32, outer: f32, mid: f32, half: f32, cut: f32 },
}

impl Feature {
    /// The offset from `x` to this feature's nearest point, for `x` outside it.
    fn offset(self, x: Vec2) -> Vec2 {
        match self {
            Feature::Bar { centre, axis, half } => {
                let perp = axis.perp();
                let d = x - centre;
                let q = Vec2::new(d.dot(axis), d.dot(perp)).clamp(-half, half);
                centre + q.x * axis + q.y * perp - x
            }
            Feature::Sector { inner, outer, cut, half, .. } => {
                let mut best = Vec2::splat(f32::INFINITY);
                let mut keep = |p: Vec2| {
                    let o = p - x;
                    if o.length_squared() < best.length_squared() {
                        best = o;
                    }
                };
                // Each cut is a segment running from where it leaves the inner
                // circle — or where the two cuts meet, whichever stands farther
                // out — to where it meets the outer one.
                let end = (outer * outer - cut * cut).max(0.0).sqrt();
                let start = (cut / half.tan()).max((inner * inner - cut * cut).max(0.0).sqrt());
                let ((e1, m1), (e2, m2)) = self.cut_frames();
                for (e, m) in [(e1, m1), (e2, m2)] {
                    let base = cut * m;
                    let t = (x - base).dot(e).clamp(start.min(end), end);
                    keep(base + t * e);
                }
                // A radial projection reaches an arc only while it stays inside
                // the cuts; where it does not, that arc's nearest point is its
                // endpoint, which the cut segments already carry.
                let r = x.length();
                if r > 1.0e-6 {
                    for radius in [inner, outer] {
                        let p = x / r * radius;
                        if self.within_cuts(p) {
                            keep(p);
                        }
                    }
                }
                best
            }
        }
    }

    /// Whether `x` is ink.
    fn covers(self, x: Vec2) -> bool {
        match self {
            Feature::Bar { centre, axis, half } => {
                let perp = axis.perp();
                let d = x - centre;
                d.dot(axis).abs() <= half.x && d.dot(perp).abs() <= half.y
            }
            Feature::Sector { inner, outer, .. } => {
                let r = x.length();
                r >= inner && r <= outer && self.within_cuts(x)
            }
        }
    }

    /// Each cut's outward edge direction, and the inward normal `cut` is
    /// measured along.
    fn cut_frames(self) -> ((Vec2, Vec2), (Vec2, Vec2)) {
        let Feature::Sector { mid, half, .. } = self else {
            unreachable!("only a sector has cuts")
        };
        let e1 = Vec2::from_angle(mid + half);
        let e2 = Vec2::from_angle(mid - half);
        ((e1, Vec2::new(e1.y, -e1.x)), (e2, Vec2::new(-e2.y, e2.x)))
    }

    /// Whether a point stands on the ink side of the cuts.
    ///
    /// A wedge wider than a half turn is REFLEX, and a reflex wedge is not an
    /// intersection of half-planes: past that width a point belongs to it by
    /// being inside EITHER cut rather than inside both.
    fn within_cuts(self, x: Vec2) -> bool {
        let Feature::Sector { half, cut, .. } = self else {
            unreachable!("only a sector has cuts")
        };
        let ((_, m1), (_, m2)) = self.cut_frames();
        let (a, b) = (x.dot(m1) >= cut, x.dot(m2) >= cut);
        if half > std::f32::consts::FRAC_PI_2 {
            a || b
        } else {
            a && b
        }
    }
}

/// What the rule reads at one point of one shape.
struct Reading {
    /// Distance to the nearest ink.
    d1: f32,
    /// The second feature's distance, and how squarely its foot faces.
    d2: f32,
    k: f32,
    /// The cosine the ramp is applied to — what a sweep has to carry across the
    /// ramp's band for its smoothness claim to be measuring anything.
    cos_phi: f32,
    /// The coverage the consumer reads: the union, carried as one distance.
    s: f32,
}

/// Read `ink` at `x` with a facing ramp of `ramp` (0 is the hard predicate).
///
/// The second feature is the RUNNER-UP — the nearest of the caster's others,
/// weighted by how squarely its foot faces. §2 defines it as the nearest among
/// those beyond the plane, which differs only where a nearer feature is
/// excluded and a farther one is not; the producers keep a top-2, so the model
/// keeps the same approximation rather than a better one.
fn read(ink: &[Feature], x: Vec2, ramp: f32) -> Reading {
    let offsets: Vec<Vec2> = ink.iter().map(|f| f.offset(x)).collect();
    let mut win = 0;
    for (i, o) in offsets.iter().enumerate() {
        if o.length_squared() < offsets[win].length_squared() {
            win = i;
        }
    }
    let near = offsets[win];
    let mut foot = Vec2::splat(f32::INFINITY);
    for (i, &o) in offsets.iter().enumerate() {
        if i != win && o.length_squared() < foot.length_squared() {
            foot = o;
        }
    }
    let (d1, d2) = (near.length(), foot.length());
    let k = facing_at(near, foot, ramp);
    Reading {
        d1,
        d2,
        k,
        cos_phi: foot.dot(-near / d1) / d2,
        s: standoff_coverage(union_distance(d1, d2, k, W), W),
    }
}

/// What today's row reads at `x`: the standoff of the nearest ink alone.
fn nearest_only(ink: &[Feature], x: Vec2) -> f32 {
    standoff_coverage(nearest(ink, x), W)
}

/// Distance from `x` to the nearest ink, zero where `x` is ink.
///
/// The covered case is its own branch because [`Feature::offset`] answers with
/// the nearest BOUNDARY point, which inside a sector is a positive distance:
/// without it a patch walks over ink and measures a field no shadow reads.
fn nearest(ink: &[Feature], x: Vec2) -> f32 {
    if ink.iter().any(|f| f.covers(x)) {
        return 0.0;
    }
    ink.iter().map(|f| f.offset(x).length()).fold(f32::INFINITY, f32::min)
}

/// The largest jump between adjacent readings of `f` along `line`, and where it
/// sits.
fn largest_step_at(line: &[Vec2], f: impl Fn(Vec2) -> f32) -> (f32, Vec2) {
    line.windows(2).fold((0.0f32, line[0]), |worst, p| {
        let d = (f(p[1]) - f(p[0])).abs();
        if d > worst.0 {
            (d, p[0])
        } else {
            worst
        }
    })
}

/// [`largest_step_at`] without the location.
fn largest_step(line: &[Vec2], f: impl Fn(Vec2) -> f32) -> f32 {
    largest_step_at(line, f).0
}

/// The largest second difference of `f` over a square patch of [`PX`] samples,
/// taken along each axis, skipping any stencil that comes within [`CLEARANCE`]
/// of the ink.
fn max_second_difference(
    ink: &[Feature],
    centre: Vec2,
    reach: f32,
    f: impl Fn(Vec2) -> f32,
) -> f32 {
    let n = (reach / PX) as i32;
    let clear = |x: Vec2| {
        [Vec2::ZERO, Vec2::X * PX, -Vec2::X * PX, Vec2::Y * PX, -Vec2::Y * PX]
            .iter()
            .all(|o| nearest(ink, x + *o) > CLEARANCE)
    };
    let mut worst: f32 = 0.0;
    for iy in -n..=n {
        for ix in -n..=n {
            let x = centre + Vec2::new(ix as f32, iy as f32) * PX;
            if !clear(x) {
                continue;
            }
            let mid = f(x);
            for axis in [Vec2::X, Vec2::Y] {
                worst = worst.max((f(x + axis * PX) - 2.0 * mid + f(x - axis * PX)).abs());
            }
        }
    }
    worst
}

/// The points from `a` to `b` inclusive, [`SWEEP`] apart.
fn sweep(a: Vec2, b: Vec2) -> Vec<Vec2> {
    let n = (a.distance(b) / SWEEP).round() as usize + 1;
    (0..n).map(|i| a.lerp(b, i as f32 / (n - 1) as f32)).collect()
}

// ---------------------------------------------------------------- the shapes

/// Two slabs 6 points wide facing each other across `gap`.
fn facing_slabs(gap: f32) -> Vec<Feature> {
    let bar = |x: f32| Feature::Bar {
        centre: Vec2::new(x, 0.0),
        axis: Vec2::X,
        half: Vec2::new(3.0, 20.0),
    };
    vec![bar(-gap / 2.0 - 3.0), bar(gap / 2.0 + 3.0)]
}

/// An L of 5 point bars, its inner corner at the origin, its free quadrant the
/// positive one and its outer corner at `(-5, -5)`.
fn ell() -> Vec<Feature> {
    vec![
        Feature::Bar { centre: Vec2::new(12.5, -2.5), axis: Vec2::X, half: Vec2::new(17.5, 2.5) },
        Feature::Bar { centre: Vec2::new(-2.5, 12.5), axis: Vec2::X, half: Vec2::new(2.5, 17.5) },
    ]
}

/// A V of 5 point bars meeting at the origin with an interior angle of
/// `degrees`, opening along +y.
fn vee(degrees: f32) -> Vec<Feature> {
    let a = degrees.to_radians() / 2.0;
    let arm = |sx: f32| {
        let axis = Vec2::new(sx * a.sin(), a.cos());
        let inward = Vec2::new(-sx * a.cos(), a.sin());
        Feature::Bar { centre: 15.0 * axis - 2.5 * inward, axis, half: Vec2::new(20.0, 2.5) }
    };
    vec![arm(1.0), arm(-1.0)]
}

/// The inner radius, outer radius and gap width of the sliced ring, in points.
const RING: (f32, f32, f32) = (14.0, 18.0, 2.5);

/// How many slices it is cut into.
const SLICES: usize = 8;

/// An annulus cut into [`SLICES`] slices by parallel cuts, leaving gaps
/// [`RING`]`.2` wide — the octave ring of #490's seam.
fn sliced_ring() -> Vec<Feature> {
    let (inner, outer, gap) = RING;
    let pitch = std::f32::consts::TAU / SLICES as f32;
    (0..SLICES)
        .map(|i| Feature::Sector {
            inner,
            outer,
            mid: i as f32 * pitch,
            half: pitch / 2.0,
            cut: gap / 2.0,
        })
        .collect()
}

/// The direction the middle of one of the sliced ring's gaps lies along: the
/// pitch edge its two slices are cut back from.
fn gap_direction() -> Vec2 {
    Vec2::from_angle(std::f32::consts::PI / SLICES as f32)
}

/// A C ring of radius 12–16 open 70°, with a 3 point crossbar across it: the G
/// whose counter carries #490's crease.
fn c_ring_with_a_crossbar() -> Vec<Feature> {
    vec![
        Feature::Sector {
            inner: 12.0,
            outer: 16.0,
            mid: std::f32::consts::PI,
            half: (360.0f32 - 70.0).to_radians() / 2.0,
            cut: 0.0,
        },
        Feature::Bar { centre: Vec2::new(6.0, 0.0), axis: Vec2::X, half: Vec2::new(10.0, 1.5) },
    ]
}

/// A 5 point bar ending 6 points short of a concave wall: a crossbar's tip
/// inside its bowl, with the bar's other end reaching out through the bowl's
/// opening.
fn bar_end_in_a_bowl() -> Vec<Feature> {
    vec![
        Feature::Sector {
            inner: 12.0,
            outer: 16.0,
            mid: 0.0,
            half: (360.0f32 - 70.0).to_radians() / 2.0,
            cut: 0.0,
        },
        Feature::Bar { centre: Vec2::new(-2.0, 0.0), axis: Vec2::X, half: Vec2::new(8.0, 2.5) },
    ]
}

// ---------------------------------------------------------------- the claims

/// The union of two distances is what the midline of a facing pair reads, and
/// the crease that stood there is gone.
///
/// The identity is exact rather than approached: two slabs facing across `g`
/// put both feet at `g/2` on the plane's own side, so the rule's answer is
/// `1 − (1 − p(g/2))²` with nothing left over. The second claim is the one the
/// picture cares about — a filled crease has to be as flat as the field around
/// it, not merely deeper than it was.
#[test]
fn a_facing_pair_reads_the_union_of_its_two_distances() {
    let gap = W / 2.0;
    let ink = facing_slabs(gap);
    let mid = read(&ink, Vec2::ZERO, BEYOND_RAMP);
    let exact = 1.0 - (1.0 - spend(gap / 2.0, W)).powi(2);
    assert_eq!(mid.k, 1.0, "the two feet face squarely, so neither is weighted down");
    assert!(
        (mid.s - exact).abs() < 1.0e-6,
        "the midline reads {} where the union of two {} point feet is {exact}",
        mid.s,
        gap / 2.0,
    );
    let crease = max_second_difference(&ink, Vec2::ZERO, 3.0, |x| read(&ink, x, BEYOND_RAMP).s);
    let today = max_second_difference(&ink, Vec2::ZERO, 3.0, |x| nearest_only(&ink, x));
    assert!(
        today > 5.0 * SMOOTH,
        "the min-distance field reads {today} across the axis, so the fixture has no crease in it \
         for the rule to fill",
    );
    assert!(
        crease <= SMOOTH,
        "the filled crease reads {crease} against a smooth field's {SMOOTH} (min-distance: \
         {today})",
    );
}

/// A lone edge and a convex corner read the nearest distance and nothing else.
///
/// Both are the case the rule must not touch: on the open side of a pair the
/// far slab stands behind the plane, and outside an L's outer corner both arms
/// answer with the same foot. Equality is exact rather than toleranced because
/// the facing weight is exactly zero there and `union_distance` returns `d1`
/// itself, which is what keeps
/// `a_distance_row_darkens_a_corner_as_deeply_as_an_edge_where_a_blur_retreats`
/// green.
#[test]
fn a_lone_edge_and_a_convex_corner_read_the_nearest_distance_alone() {
    let gap = W / 2.0;
    let slabs = facing_slabs(gap);
    let l = ell();
    for (what, ink, x) in [
        ("the open side of a facing pair", &slabs, Vec2::new(-gap / 2.0 - 6.0 - 2.5, 0.0)),
        ("outside an L's outer corner", &l, Vec2::new(-8.0, -8.0)),
    ] {
        let r = read(ink, x, BEYOND_RAMP);
        assert_eq!(r.k, 0.0, "{what} weights its second foot {} at cos {}", r.k, r.cos_phi);
        assert_eq!(
            r.s,
            nearest_only(ink, x),
            "{what} reads {} where the nearest ink alone gives {}",
            r.s,
            nearest_only(ink, x),
        );
    }
}

/// A 90° junction fills to the exact union, and does it without a crease.
///
/// The L is a crossbar meeting its bowl. On the diagonal both arms stand at the
/// same distance and the second arm's foot lands exactly ON the plane, so the
/// junction is the boundary case the ramp has to leave at full weight — the
/// union is `1 − (1 − p(a))²` with nothing shaved off it.
#[test]
fn a_right_angle_junction_fills_to_the_exact_union() {
    let ink = ell();
    let arm = 5.0;
    let r = read(&ink, Vec2::splat(arm), BEYOND_RAMP);
    let exact = 1.0 - (1.0 - spend(arm, W)).powi(2);
    assert!(
        r.cos_phi.abs() < 1.0e-6,
        "the second foot stands at cos {}, not on the plane",
        r.cos_phi,
    );
    assert_eq!(r.k, 1.0, "a foot on the plane is weighted {} rather than counted whole", r.k);
    assert!(
        (r.s - exact).abs() < 1.0e-6,
        "the diagonal reads {} where the union of two {arm} point arms is {exact}",
        r.s,
    );
    let inner = Vec2::splat(4.0);
    let filled = max_second_difference(&ink, inner, 3.0, |x| read(&ink, x, BEYOND_RAMP).s);
    let today = max_second_difference(&ink, inner, 3.0, |x| nearest_only(&ink, x));
    assert!(
        today > 5.0 * SMOOTH,
        "the min-distance field reads {today} inside the corner, so the fixture has no crease in \
         it for the rule to fill",
    );
    assert!(
        filled <= SMOOTH,
        "the filled corner reads {filled} against a smooth field's {SMOOTH} (min-distance: \
         {today})",
    );
}

/// The ramp shuts exactly at a 120° junction, which is what fixes its width.
///
/// On the bisector of a concave junction the second arm's foot stands at
/// `cos θ` of the plane, θ being the interior angle: the geometry names the
/// constant, so `BEYOND_RAMP` is the widest junction filled at all rather than
/// a number tuned in the abstract. At a half that is 120°, leaving the 90°
/// junction whole and giving an obtuse one nothing — §2's "same as today, no
/// worse".
#[test]
fn the_ramp_shuts_exactly_at_a_120_degree_junction() {
    for degrees in [60.0f32, 90.0, 105.0, 120.0, 150.0] {
        let ink = vee(degrees);
        let x = Vec2::new(0.0, 6.0);
        let (near, foot) = (ink[0].offset(x), ink[1].offset(x));
        let cos = foot.dot(-near.normalize()) / foot.length();
        assert!(
            (cos - degrees.to_radians().cos()).abs() < 1.0e-5,
            "the {degrees}° V puts its second foot at cos {cos}, not at the cosine of its own \
             angle",
        );
        let k = facing(near, foot);
        let want = smoothstep(-BEYOND_RAMP, 0.0, degrees.to_radians().cos());
        assert!((k - want).abs() < 1.0e-6, "the {degrees}° V is weighted {k} rather than {want}");
        if degrees >= 120.0 {
            assert_eq!(k, 0.0, "the {degrees}° V is filled at weight {k}");
            assert_eq!(
                read(&ink, x, BEYOND_RAMP).s,
                nearest_only(&ink, x),
                "the {degrees}° V moved off the nearest-ink reading",
            );
        } else {
            assert!(k > 0.0, "the {degrees}° V is not filled at all");
        }
    }
}

/// The sliced ring fills its gaps and leaves its outside exactly where it was.
///
/// The trap this is against is a union taken over PRIMITIVES, which counts two
/// collinear slices twice along their shared edge line and lifts the shadow
/// outside every gap. The half-space test excludes a co-facing neighbour by
/// construction, so the outside stays the min-distance row's — and the sweep
/// one point out, where the gap itself is in view, is what says the same sweep
/// would have seen a lift if there were one.
#[test]
fn the_ring_fills_its_gaps_and_leaves_its_outside_alone() {
    let ink = sliced_ring();
    let (inner, outer, gap) = RING;
    let centre = gap_direction() * (inner + outer) / 2.0;
    let r = read(&ink, centre, BEYOND_RAMP);
    let exact = 1.0 - (1.0 - spend(gap / 2.0, W)).powi(2);
    assert!(
        r.s >= exact - 1.0e-6,
        "the gap centre reads {} where two feet {} points off union to {exact}",
        r.s,
        gap / 2.0,
    );
    let round = |out: f32| {
        (0..3600)
            .map(|i| Vec2::from_angle(i as f32 * std::f32::consts::TAU / 3600.0) * (outer + out))
            .map(|x| read(&ink, x, BEYOND_RAMP).s - nearest_only(&ink, x))
            .fold(0.0f32, f32::max)
    };
    assert!(
        round(1.0) > 0.2,
        "one point out the ring gains only {}, so the sweep never passes a gap",
        round(1.0),
    );
    assert!(
        round(4.0) < 1.0e-6,
        "four points out the ring gains {} over the min-distance row",
        round(4.0),
    );
}

/// The mouth of a gap is a STEP under a hard half-space test, and the ramp is
/// what takes it out.
///
/// Where the nearest ink is a convex CORNER the plane rotates about a fixed
/// foot, so the far slice switches on along a curve rather than sliding in: on
/// the gap's midline that is a quarter of the depth in one sample, at exactly
/// half a gap outside each radius. The hard predicate is evaluated here too and
/// asserted to reproduce that step — a sweep stopping short of the mouth reads
/// a smooth field under either rule and proves nothing (#450).
#[test]
fn the_ring_mouth_is_a_step_under_a_hard_test_and_smooth_under_the_ramp() {
    let ink = sliced_ring();
    let (inner, outer, _) = RING;
    let g = gap_direction();
    let centre = g * (inner + outer) / 2.0;
    for (edge, out, radius) in [("outer", outer + 3.0, outer), ("inner", inner - 3.0, inner)] {
        let midline = sweep(centre, g * out);
        let hard = largest_step(&midline, |x| read(&ink, x, 0.0).s);
        let ramped = largest_step(&midline, |x| read(&ink, x, BEYOND_RAMP).s);
        let today = largest_step(&midline, |x| nearest_only(&ink, x));
        assert!(
            hard > 0.2,
            "the hard test steps {hard} at the {edge} mouth, so the sweep does not reach it",
        );
        assert!(today < 0.01, "the min-distance row steps {today} along the {edge} midline");
        // A twentieth of the depth, against the quarter the hard test steps.
        // What is left under the ramp is the field's own slope through it:
        // `p` falls the whole way from the gap's reading to the lone corner's
        // over about half a point, so a sample carries a share of that.
        assert!(
            ramped < 0.05 && ramped * 5.0 < hard,
            "the ramp steps {ramped} at the {edge} mouth against the hard test's {hard} and the \
             min-distance row's {today}",
        );
        // The patch is centred on the corner the plane pivots about, and three
        // points of reach carries the crossing at half a gap out.
        let patch =
            |ramp: f32| max_second_difference(&ink, g * radius, 3.0, |x| read(&ink, x, ramp).s);
        let flat = max_second_difference(&ink, g * radius, 3.0, |x| nearest_only(&ink, x));
        assert!(
            patch(0.0) > 2.0 * flat,
            "the hard test reads {} over the {edge} mouth patch against the min-distance row's \
             {flat}, so the patch does not hold the step",
            patch(0.0),
        );
        assert!(
            patch(BEYOND_RAMP) <= flat,
            "the ramp reads {} over the {edge} mouth patch, above the min-distance row's own \
             {flat}",
            patch(BEYOND_RAMP),
        );
    }
}

/// A bar ending inside a bowl carries the same pair of claims, over a sweep
/// that crosses the ramp.
///
/// The tip end alone does not cross it: a concave wall faces a convex corner
/// from everywhere inside it, so the cosine never leaves 1 there and the ramp
/// is never exercised. What crosses it is the bar's OTHER end, a stroke
/// terminal standing against the bowl's own terminal — so the sweep runs the
/// length of the bar, past the tip's pocket and out through the opening, and
/// the hard test's step over it is what says it arrived.
#[test]
fn a_bar_end_in_a_bowl_crosses_the_ramp_without_a_step() {
    let ink = bar_end_in_a_bowl();
    // A point and a half off the bar's flank, from inside the pocket between
    // its tip and the wall to well past its far end.
    let line = sweep(Vec2::new(11.0, 4.0), Vec2::new(-16.0, 4.0));
    let (lo, hi) = line.iter().fold((1.0f32, -1.0f32), |(lo, hi), &x| {
        let c = read(&ink, x, BEYOND_RAMP).cos_phi;
        (lo.min(c), hi.max(c))
    });
    assert!(
        hi > 0.5 && lo < -0.25,
        "the sweep carries the second foot over cos {lo}..{hi}, which does not cross the ramp",
    );
    let (hard, at) = largest_step_at(&line, |x| read(&ink, x, 0.0).s);
    let ramped = largest_step(&line, |x| read(&ink, x, BEYOND_RAMP).s);
    let today = largest_step(&line, |x| nearest_only(&ink, x));
    assert!(hard > 0.1, "the hard test steps {hard}, so the sweep does not reach the terminal");
    assert!(
        ramped <= today,
        "the ramp steps {ramped} where the min-distance row steps {today} (hard test: {hard})",
    );
    let patch = |ramp: f32| max_second_difference(&ink, at, 3.0, |x| read(&ink, x, ramp).s);
    let flat = max_second_difference(&ink, at, 3.0, |x| nearest_only(&ink, x));
    assert!(
        patch(0.0) > 2.0 * flat,
        "the hard test reads {} over the terminal's patch against the min-distance row's {flat}, \
         so the patch does not hold the step",
        patch(0.0),
    );
    assert!(
        patch(BEYOND_RAMP) < 0.5 * flat,
        "the ramp reads {} over the terminal's patch against the min-distance row's {flat}",
        patch(BEYOND_RAMP),
    );
}

/// A G's counter deepens over its crossbar, and stays as flat as it was.
///
/// The reading #490 is about: three points above the crossbar, inside the bowl,
/// where the min-distance row spends the crossbar's distance alone and the wall
/// on the far side of the point contributes nothing.
#[test]
fn a_counter_deepens_over_its_crossbar_without_a_crease() {
    let ink = c_ring_with_a_crossbar();
    let x = Vec2::new(0.0, 4.5);
    let r = read(&ink, x, BEYOND_RAMP);
    let today = nearest_only(&ink, x);
    assert_eq!(r.k, 1.0, "the wall stands at cos {} of the plane", r.cos_phi);
    assert!(
        r.s > today + 0.03,
        "the counter reads {} against the min-distance row's {today}, with the crossbar {} points \
         off and the wall {}",
        r.s,
        r.d1,
        r.d2,
    );
    let filled = max_second_difference(&ink, x, 3.0, |p| read(&ink, p, BEYOND_RAMP).s);
    let flat = max_second_difference(&ink, x, 3.0, |p| nearest_only(&ink, p));
    assert!(
        flat > 2.0 * SMOOTH,
        "the min-distance field reads {flat} in the counter, so the fixture has no crease in it \
         for the rule to fill",
    );
    assert!(
        filled <= SMOOTH,
        "the filled counter reads {filled} against a smooth field's {SMOOTH} (min-distance: \
         {flat})",
    );
}

/// The ramp's trade, as the table #568's first comment reports it: what each
/// width leaves at the mouth against what it readmits outside the ring.
#[test]
#[ignore = "a probe: prints measurements, asserts nothing"]
fn the_ramp_trade_table() {
    let ink = sliced_ring();
    let (inner, outer, gap) = RING;
    let g = gap_direction();
    let ramps = [0.0f32, 0.3, 0.5, 0.6];
    let head = |what: &str| {
        println!("| {what} | min-distance | hard | ramp 0.3 | ramp 0.5 | ramp 0.6 |");
        println!("|---|---|---|---|---|---|");
    };

    head("h (pt) outside the outer radius");
    for h in [-1.0f32, 0.0, 1.0, 1.20, 1.25, 1.30, 1.5, 2.0, 4.0] {
        let x = g * (outer + h);
        let mut row = format!("| {h:.2} | {:.3} |", nearest_only(&ink, x));
        for r in ramps {
            row += &format!(" {:.3} |", read(&ink, x, r).s);
        }
        println!("{row}");
    }

    println!();
    head("over the ring");
    let centre = g * (inner + outer) / 2.0;
    for (what, line) in [
        ("largest step on the outer midline", sweep(centre, g * (outer + 3.0))),
        ("largest step on the inner midline", sweep(centre, g * (inner - 3.0))),
    ] {
        let mut row = format!("| {what} | {:.4} |", largest_step(&line, |x| nearest_only(&ink, x)));
        for r in ramps {
            row += &format!(" {:.4} |", largest_step(&line, |x| read(&ink, x, r).s));
        }
        println!("{row}");
    }
    for (what, at) in [("outer", g * outer), ("inner", g * inner)] {
        let mut row = format!(
            "| max 2nd difference over the {what} mouth patch (smooth = {SMOOTH:.3}) | {:.4} |",
            max_second_difference(&ink, at, 3.0, |x| nearest_only(&ink, x)),
        );
        for r in ramps {
            row +=
                &format!(" {:.4} |", max_second_difference(&ink, at, 3.0, |x| read(&ink, x, r).s));
        }
        println!("{row}");
    }
    // Outside the ring and over the SLICE rather than over the gap: what a
    // wider ramp readmits from the collinear neighbour.
    for out in [1.0f32, 4.0] {
        let beside: Vec<Vec2> = (0..3600)
            .map(|i| Vec2::from_angle(i as f32 * std::f32::consts::TAU / 3600.0) * (outer + out))
            .filter(|x| {
                (0..SLICES).all(|i| {
                    let m = Vec2::from_angle(
                        std::f32::consts::PI / SLICES as f32
                            + i as f32 * std::f32::consts::TAU / SLICES as f32,
                    );
                    x.perp_dot(m).abs() >= gap
                })
            })
            .collect();
        let peak = |f: &dyn Fn(Vec2) -> f32| beside.iter().map(|&x| f(x)).fold(0.0f32, f32::max);
        let mut row = format!(
            "| S {out} pt outside, over the slice beside the gap | {:.3} |",
            peak(&|x| nearest_only(&ink, x)),
        );
        for r in ramps {
            row += &format!(" {:.3} |", peak(&|x| read(&ink, x, r).s));
        }
        println!("{row}");
    }
}
