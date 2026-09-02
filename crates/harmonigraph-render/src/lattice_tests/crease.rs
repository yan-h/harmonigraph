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
    facing, facing_at, pocket, smoothstep, spend, standoff_coverage, taper_length, union_distance,
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
    /// The offset from `x` to this feature's nearest point, for `x` outside it,
    /// and that foot's CLEARANCE: how far along its own segment it stands from
    /// the nearest convex end of it, zero where the foot IS that end.
    ///
    /// The one fact the facing plane alone cannot carry. Seen from outside a
    /// gap between two pieces of one caster both feet are corners, and the
    /// exterior bisector puts each within the ramp of the other at every ramp
    /// width — the same reading a concave junction gives, whose feet stand on
    /// segment INTERIORS. What tells the two apart is the clearance.
    fn offset(self, x: Vec2) -> (Vec2, f32) {
        match self {
            Feature::Bar { centre, axis, half } => {
                let perp = axis.perp();
                let d = x - centre;
                let q = Vec2::new(d.dot(axis), d.dot(perp)).clamp(-half, half);
                // A rectangle's foot stands on the face whose own axis was
                // CLAMPED, and that face runs along the other one — so the
                // clearance is the larger of the two slacks, a clamped axis
                // having none. Both clamped is a corner and leaves neither.
                let slack = half - q.abs();
                (centre + q.x * axis + q.y * perp - x, slack.max_element().max(0.0))
            }
            Feature::Sector { inner, outer, cut, half, .. } => {
                let mut best = (Vec2::splat(f32::INFINITY), 0.0f32);
                let mut keep = |p: Vec2, clear: f32| {
                    let o = p - x;
                    if o.length_squared() < best.0.length_squared() {
                        best = (o, clear);
                    }
                };
                // Each cut is a segment running from where it leaves the inner
                // circle — or where the two cuts meet, whichever stands farther
                // out — to where it meets the outer one. Both ends are convex
                // corners of the slice, so the clearance is the margin to the
                // nearer of them and a clamped `t` leaves none.
                let end = (outer * outer - cut * cut).max(0.0).sqrt();
                let start = (cut / half.tan()).max((inner * inner - cut * cut).max(0.0).sqrt());
                let (lo, hi) = (start.min(end), end);
                let ((e1, m1), (e2, m2)) = self.cut_frames();
                for (e, m) in [(e1, m1), (e2, m2)] {
                    let base = cut * m;
                    let t = (x - base).dot(e).clamp(lo, hi);
                    keep(base + t * e, (t - lo).min(hi - t).max(0.0));
                }
                // A radial projection reaches an arc only while it stays inside
                // the cuts; where it does not, that arc's nearest point is its
                // endpoint, which the cut segments already carry.
                let r = x.length();
                if r > 1.0e-6 {
                    for radius in [inner, outer] {
                        let p = x / r * radius;
                        if self.within_cuts(p) {
                            keep(p, self.arc_clearance(p, radius));
                        }
                    }
                }
                best
            }
        }
    }

    /// How far along its own arc a foot on the circle of `radius` stands from
    /// the cut that ends that arc, as a LENGTH.
    ///
    /// A cut ends the arc `acos(cut/radius)` of angle off its own inward
    /// normal, so the margin is that angle less the foot's own — a difference
    /// of two `acos`, with no branch to wrap. The cuts bound an INTERSECTION
    /// where the wedge is convex and a UNION where it is reflex, and the margin
    /// follows: the smaller of the two, or the larger.
    ///
    /// The arc LENGTH rather than the chord to the cut line the shader takes
    /// (`annular_sector_distance`): the two agree to a fraction of a percent
    /// wherever the cut meets the circle near-squarely, which the octave ring's
    /// does, and only the neighbourhood of the end decides a taper.
    fn arc_clearance(self, p: Vec2, radius: f32) -> f32 {
        let Feature::Sector { half, cut, .. } = self else {
            unreachable!("only a sector has arcs")
        };
        let ((_, m1), (_, m2)) = self.cut_frames();
        let r = radius.max(1.0e-6);
        let beta = (cut / r).clamp(-1.0, 1.0).acos();
        let margin = |m: Vec2| beta - (p.dot(m) / r).clamp(-1.0, 1.0).acos();
        let (a, b) = (margin(m1), margin(m2));
        let angle = if half > std::f32::consts::FRAC_PI_2 { a.max(b) } else { a.min(b) };
        r * angle.max(0.0)
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

/// The two gates a second foot passes before it is unioned in.
///
/// Both are parameters and not constants so that one test evaluates the shipped
/// rule beside the rule without it: a mouth claim is only measuring something
/// if the sweep reads a different field under a rule that admits the mouth,
/// and a sweep stopping short of one reads the same field under every rule
/// here (#450).
#[derive(Clone, Copy)]
struct Rule {
    /// How far behind the facing plane a foot still counts, as a cosine. Zero
    /// is the hard predicate #568's first comment measures the step of.
    ramp: f32,
    /// The length a foot's clearance tapers over, in points. Zero counts every
    /// foot whole however close it stands to the end of its own segment.
    taper: f32,
}

/// The rule the picture is drawn with.
const TAPERED: Rule = Rule { ramp: BEYOND_RAMP, taper: taper_length(W) };

/// The facing ramp with every clearance counted whole: the rule #578 shipped,
/// and what the taper is measured against.
const RAMP_ONLY: Rule = Rule { ramp: BEYOND_RAMP, taper: 0.0 };

/// The hard half-plane predicate, ungated either way.
const HARD: Rule = Rule { ramp: 0.0, taper: 0.0 };

/// What the rule reads at one point of one shape.
struct Reading {
    /// Distance to the nearest ink.
    d1: f32,
    /// The second feature's distance, and how much of its foot counts once both
    /// gates are applied.
    d2: f32,
    k: f32,
    /// The taper's own share of that weight, so a claim can name which gate
    /// moved a reading.
    pocket: f32,
    /// The cosine the ramp is applied to — what a sweep has to carry across the
    /// ramp's band for its smoothness claim to be measuring anything.
    cos_phi: f32,
    /// The coverage the consumer reads: the union, carried as one distance.
    s: f32,
}

/// Read `ink` at `x` under `rule`.
///
/// The second feature is the RUNNER-UP — the nearest of the caster's others,
/// weighted by how squarely its foot faces and by how far both feet stand from
/// the ends of their own segments. §2 defines it as the nearest among those
/// beyond the plane, which differs only where a nearer feature is excluded and
/// a farther one is not; the producers keep a top-2, so the model keeps the
/// same approximation rather than a better one.
fn read(ink: &[Feature], x: Vec2, rule: Rule) -> Reading {
    let feet: Vec<(Vec2, f32)> = ink.iter().map(|f| f.offset(x)).collect();
    let mut win = 0;
    for (i, o) in feet.iter().enumerate() {
        if o.0.length_squared() < feet[win].0.length_squared() {
            win = i;
        }
    }
    let near = feet[win];
    let mut foot = (Vec2::splat(f32::INFINITY), 0.0);
    for (i, &o) in feet.iter().enumerate() {
        if i != win && o.0.length_squared() < foot.0.length_squared() {
            foot = o;
        }
    }
    let (d1, d2) = (near.0.length(), foot.0.length());
    // A taper of zero is no taper at all rather than a step at zero clearance:
    // `smoothstep` over an empty interval answers NaN exactly at its edge, and
    // a corner's clearance sits exactly there.
    let pocket = if rule.taper > 0.0 { pocket(near.1, foot.1, rule.taper) } else { 1.0 };
    let k = facing_at(near.0, foot.0, rule.ramp) * pocket;
    Reading {
        d1,
        d2,
        k,
        pocket,
        cos_phi: foot.0.dot(-near.0 / d1) / d2,
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
    ink.iter().map(|f| f.offset(x).0.length()).fold(f32::INFINITY, f32::min)
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
///
/// The two arms OVERLAP at the junction, so the segment ends that meet there
/// are buried in ink and no reading stands off them: the clearance's
/// convex-only rule needs no case here. A glyph bake, whose contours meet
/// rather than overlap, does — its terminals have to be told from its
/// junctions.
fn ell() -> Vec<Feature> {
    vec![
        Feature::Bar { centre: Vec2::new(12.5, -2.5), axis: Vec2::X, half: Vec2::new(17.5, 2.5) },
        Feature::Bar { centre: Vec2::new(-2.5, 12.5), axis: Vec2::X, half: Vec2::new(2.5, 17.5) },
    ]
}

/// A V of 5 point bars meeting at the origin with an interior angle of
/// `degrees`, opening along +y — overlapping at the vertex like [`ell`]'s arms,
/// for the same reason.
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
    let mid = read(&ink, Vec2::ZERO, TAPERED);
    let exact = 1.0 - (1.0 - spend(gap / 2.0, W)).powi(2);
    assert_eq!(mid.k, 1.0, "the two feet face squarely, so neither is weighted down");
    assert!(
        (mid.s - exact).abs() < 1.0e-6,
        "the midline reads {} where the union of two {} point feet is {exact}",
        mid.s,
        gap / 2.0,
    );
    let crease = max_second_difference(&ink, Vec2::ZERO, 3.0, |x| read(&ink, x, TAPERED).s);
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
        let r = read(ink, x, TAPERED);
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
    let r = read(&ink, Vec2::splat(arm), TAPERED);
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
    let filled = max_second_difference(&ink, inner, 3.0, |x| read(&ink, x, TAPERED).s);
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
        let (near, foot) = (ink[0].offset(x).0, ink[1].offset(x).0);
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
                read(&ink, x, TAPERED).s,
                nearest_only(&ink, x),
                "the {degrees}° V moved off the nearest-ink reading",
            );
        } else {
            assert!(k > 0.0, "the {degrees}° V is not filled at all");
        }
    }
}

/// The sliced ring fills its gaps, at the weight its own SHORTNESS allows, and
/// leaves its outside where it was.
///
/// The trap this is against is a union taken over PRIMITIVES, which counts two
/// collinear slices twice along their shared edge line and lifts the shadow
/// outside every gap.
///
/// The exact union is out of reach here, and by design: the cut segment is 4.01
/// points long between its two convex ends against a taper of
/// `taper_length(W)`, so the two tapers overlap and the middle of the gap
/// never counts a whole pair. A gap that short is what an octave ring HAS —
/// capping the taper at half a segment would buy the middle back and hand the
/// mouth its bulb again, since the cap is largest exactly where the segment is
/// shortest. So the claim is the arithmetic at the weight the pair carries,
/// plus the lift over the min-distance row that says the union reached the
/// fixture at all.
#[test]
fn the_ring_fills_its_gaps_and_leaves_its_outside_alone() {
    let ink = sliced_ring();
    let (inner, outer, gap) = RING;
    let centre = gap_direction() * (inner + outer) / 2.0;
    let r = read(&ink, centre, TAPERED);
    let p = spend(gap / 2.0, W);
    let held = 1.0 - (1.0 - p) * (1.0 - r.pocket * p);
    assert!(
        r.pocket > 0.7 && r.pocket < 1.0,
        "the gap centre carries {} of its pair, so the fixture is not measuring a taper that \
         overlaps itself",
        r.pocket,
    );
    assert!(
        (r.s - held).abs() < 1.0e-6,
        "the gap centre reads {} where two feet {} points off, at {} of a pair, union to {held}",
        r.s,
        gap / 2.0,
        r.pocket,
    );
    assert!(
        r.s > nearest_only(&ink, centre) + 0.15,
        "the gap centre reads {} against the min-distance row's {}, so the union is not reaching \
         this fixture",
        r.s,
        nearest_only(&ink, centre),
    );
    let round = |out: f32| {
        (0..3600)
            .map(|i| Vec2::from_angle(i as f32 * std::f32::consts::TAU / 3600.0) * (outer + out))
            .map(|x| read(&ink, x, TAPERED).s - nearest_only(&ink, x))
            .fold(0.0f32, f32::max)
    };
    // What the taper leaves outside the ring, which is
    // `a_mouths_exterior_reads_the_nearest_field_exactly`'s subject: nothing at
    // all over a mouth, and a residual thousandths of a level deep over the
    // slice's own arc, where an ADJACENT slice's cut foot stands a fraction
    // short of its end and so is not a corner.
    assert!(
        round(1.0) < 5.0e-4,
        "one point out the ring gains {} over the min-distance row",
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
///
/// The FACING gate on its own, with every clearance counted whole: what the
/// ramp is worth is a separate quantity from what the taper over it is worth,
/// and `a_gaps_taper_carries_no_step_to_its_mouth` makes the same measurement
/// of the rule the picture is drawn with.
#[test]
fn the_ring_mouth_is_a_step_under_a_hard_test_and_smooth_under_the_ramp() {
    let ink = sliced_ring();
    let (inner, outer, _) = RING;
    let g = gap_direction();
    let centre = g * (inner + outer) / 2.0;
    for (edge, out, radius) in [("outer", outer + 3.0, outer), ("inner", inner - 3.0, inner)] {
        let midline = sweep(centre, g * out);
        let hard = largest_step(&midline, |x| read(&ink, x, HARD).s);
        let ramped = largest_step(&midline, |x| read(&ink, x, RAMP_ONLY).s);
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
            |rule: Rule| max_second_difference(&ink, g * radius, 3.0, |x| read(&ink, x, rule).s);
        let flat = max_second_difference(&ink, g * radius, 3.0, |x| nearest_only(&ink, x));
        assert!(
            patch(HARD) > 2.0 * flat,
            "the hard test reads {} over the {edge} mouth patch against the min-distance row's \
             {flat}, so the patch does not hold the step",
            patch(HARD),
        );
        assert!(
            patch(RAMP_ONLY) <= flat,
            "the ramp reads {} over the {edge} mouth patch, above the min-distance row's own \
             {flat}",
            patch(RAMP_ONLY),
        );
    }
}

/// The taper carries the gap's fill to nothing BEFORE its mouth, so nothing
/// crosses the mouth to step there — and it does it without leaving a plug.
///
/// The claim the clearance was written for. Half a gap outside either radius
/// the nearest ink is a convex CORNER, and a corner is where a segment ends:
/// the pair is worth nothing there whatever angle its feet stand at, so the
/// field outside is the min-distance field and there is no switch-on to smooth.
///
/// A PLUG is the failure this build can produce in exchange, and it is the
/// second reading here: a fill that rises through the gap and then drops within
/// a taper of the mouth would leave a bright knot ending in mid-gap, which is a
/// different artifact rather than a smaller one. So the profile is required to
/// fall the whole way out from its own deepest point, and that point is
/// required to stand at the gap's middle rather than pressed against a mouth.
#[test]
fn a_gaps_taper_carries_no_step_to_its_mouth() {
    let ink = sliced_ring();
    let (inner, outer, _) = RING;
    let g = gap_direction();
    let centre = g * (inner + outer) / 2.0;
    for (edge, out, radius) in [("outer", outer + 3.0, outer), ("inner", inner - 3.0, inner)] {
        let midline = sweep(centre, g * out);
        let hard = largest_step(&midline, |x| read(&ink, x, HARD).s);
        let tapered = largest_step(&midline, |x| read(&ink, x, TAPERED).s);
        let today = largest_step(&midline, |x| nearest_only(&ink, x));
        assert!(
            hard > 0.2,
            "the hard test steps {hard} at the {edge} mouth, so the sweep does not reach it",
        );
        assert!(
            tapered < 0.05 && tapered * 5.0 < hard,
            "the taper steps {tapered} at the {edge} mouth against the hard test's {hard} and \
             the min-distance row's {today}",
        );
        let patch =
            |rule: Rule| max_second_difference(&ink, g * radius, 3.0, |x| read(&ink, x, rule).s);
        let flat = max_second_difference(&ink, g * radius, 3.0, |x| nearest_only(&ink, x));
        // The min-distance row's own curvature, and not a fraction of it: the
        // taper hands the mouth patch back to that row rather than smoothing
        // it, so what is left over is the exterior residual
        // `a_mouths_exterior_reads_the_nearest_field_exactly` bounds — 2e-5 of
        // the 0.117 the corner's own field carries.
        assert!(
            patch(TAPERED) <= flat * 1.001,
            "the taper reads {} over the {edge} mouth patch, above the min-distance row's own \
             {flat}",
            patch(TAPERED),
        );

        // No plug: the deepest reading stands at the gap's middle, and from
        // there the field only falls out through the mouth.
        let deepest = midline
            .iter()
            .copied()
            .enumerate()
            .max_by(|a, b| read(&ink, a.1, TAPERED).s.total_cmp(&read(&ink, b.1, TAPERED).s))
            .expect("the midline has samples");
        let (from_centre, to_mouth) =
            ((deepest.1 - centre).length(), (deepest.1 - g * radius).length());
        assert!(
            from_centre < 0.5 * to_mouth,
            "the {edge} midline is deepest {from_centre} points off the gap's centre and \
             {to_mouth} off its mouth, so the fill has moved out to the mouth",
        );
        let rise = midline[deepest.0..]
            .windows(2)
            .map(|x| read(&ink, x[1], TAPERED).s - read(&ink, x[0], TAPERED).s)
            .fold(0.0f32, f32::max);
        assert!(
            rise <= 0.0,
            "the {edge} midline rises {rise} on its way out to the mouth, so the fill ends in a \
             plug rather than a taper",
        );
    }
}

/// Outside a gap's mouth the taper hands the field back to the min-distance
/// row EXACTLY, which is what says the bulb is gone rather than smaller.
///
/// Exactly and not nearly: both feet at a mouth are corners, `pocket` is zero
/// with zero slope there, and `union_distance` returns `d1` itself at zero
/// weight — so the cell outside a mouth is the same number the row before the
/// union wrote, bit for bit.
///
/// What is NOT exact, and the second claim here, is the rest of the ring's
/// outside. Over a slice's own arc the runner-up is the adjacent slice's cut
/// foot, which stands a fraction inside its own end rather than at it: a pair
/// at about a hundredth of full weight, worth three ten-thousandths of a level
/// and confined to a band a point and a half deep. That is under the frame's
/// own quantiser and it is the leak a topology-aware successor removes; the
/// bound is here so a change that widens it is visible.
#[test]
fn a_mouths_exterior_reads_the_nearest_field_exactly() {
    let ink = sliced_ring();
    let (inner, outer, gap) = RING;
    let g = gap_direction();
    // Every radius from just outside the ink out past the standoff's reach,
    // read on both sides of the ring.
    let radii = |i: u32| 0.05 + i as f32 * (4.0 - 0.05) / 200.0;
    // The mouth's own wedge: from the gap's midline round to the slice CORNER,
    // which is where the nearest ink stops being that corner.
    let corner = |radius: f32| (gap / 2.0 / radius).asin();
    let mut worst_mouth = 0.0f32;
    let mut worst_round = 0.0f32;
    for i in 0..=200 {
        let out = radii(i);
        for (radius, r) in [(outer, outer + out), (inner, inner - out)] {
            for j in -100..=100 {
                let x = Vec2::from_angle(g.to_angle() + j as f32 * corner(radius) / 100.0) * r;
                worst_mouth =
                    worst_mouth.max((read(&ink, x, TAPERED).s - nearest_only(&ink, x)).abs());
            }
            for j in 0..720 {
                let x = Vec2::from_angle(j as f32 * std::f32::consts::TAU / 720.0) * r;
                worst_round =
                    worst_round.max((read(&ink, x, TAPERED).s - nearest_only(&ink, x)).abs());
            }
        }
    }
    // One ulp of a coverage near a half, which is what a `d1` round trip costs.
    assert!(
        worst_mouth < 1.0e-6,
        "a mouth's exterior moves {worst_mouth} off the min-distance row",
    );
    assert!(
        worst_round < 5.0e-4,
        "the ring's outside moves {worst_round} off the min-distance row, past the residual an \
         adjacent slice's near-corner foot accounts for",
    );
    // And the same sweep under the ramp alone, which is what says the sweep
    // passes a mouth at all rather than measuring an exterior nothing reaches
    // (#450).
    let lift = (0..=200)
        .flat_map(|i| {
            let out = radii(i);
            (-100..=100).map(move |j| {
                Vec2::from_angle(g.to_angle() + j as f32 * corner(outer) / 100.0) * (outer + out)
            })
        })
        .map(|x| read(&ink, x, RAMP_ONLY).s - nearest_only(&ink, x))
        .fold(0.0f32, f32::max);
    assert!(
        lift > 0.2,
        "the ramp lifts the same wedge only {lift} over the min-distance row, so the sweep never \
         passes a mouth",
    );
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
///
/// At that terminal the taper hands the field back to the min-distance row
/// whole rather than smoothing it: the bar's foot there is a CORNER, its
/// clearance is zero, and no pair survives. So the patch matches the row it
/// gave back rather than beating it, and the fill the taper does keep lives
/// where the sweep runs along the bar's flank.
#[test]
fn a_bar_end_in_a_bowl_crosses_the_ramp_without_a_step() {
    let ink = bar_end_in_a_bowl();
    // A point and a half off the bar's flank, from inside the pocket between
    // its tip and the wall to well past its far end.
    let line = sweep(Vec2::new(11.0, 4.0), Vec2::new(-16.0, 4.0));
    let (lo, hi) = line.iter().fold((1.0f32, -1.0f32), |(lo, hi), &x| {
        let c = read(&ink, x, TAPERED).cos_phi;
        (lo.min(c), hi.max(c))
    });
    assert!(
        hi > 0.5 && lo < -0.25,
        "the sweep carries the second foot over cos {lo}..{hi}, which does not cross the ramp",
    );
    let fill = line
        .iter()
        .map(|&x| read(&ink, x, TAPERED).s - nearest_only(&ink, x))
        .fold(0.0f32, f32::max);
    assert!(
        fill > 0.02,
        "the taper lifts the sweep only {fill} over the min-distance row, so the claims below are \
         about a field the rule never touched",
    );
    let (hard, at) = largest_step_at(&line, |x| read(&ink, x, HARD).s);
    let ramped = largest_step(&line, |x| read(&ink, x, TAPERED).s);
    let today = largest_step(&line, |x| nearest_only(&ink, x));
    assert!(hard > 0.1, "the hard test steps {hard}, so the sweep does not reach the terminal");
    assert!(
        ramped <= today,
        "the ramp steps {ramped} where the min-distance row steps {today} (hard test: {hard})",
    );
    let patch = |rule: Rule| max_second_difference(&ink, at, 3.0, |x| read(&ink, x, rule).s);
    let flat = max_second_difference(&ink, at, 3.0, |x| nearest_only(&ink, x));
    assert!(
        patch(HARD) > 2.0 * flat,
        "the hard test reads {} over the terminal's patch against the min-distance row's {flat}, \
         so the patch does not hold the step",
        patch(HARD),
    );
    assert!(
        patch(TAPERED) <= flat,
        "the taper reads {} over the terminal's patch, above the min-distance row's own {flat}",
        patch(TAPERED),
    );
}

/// A G's counter deepens over its crossbar, and the crease it stood on is
/// gone — at the price of the taper's own curvature over the crossbar's free
/// END.
///
/// The reading #490 is about: three points above the crossbar, inside the bowl,
/// where the min-distance row spends the crossbar's distance alone and the wall
/// on the far side of the point contributes nothing. Over the crossbar's middle
/// the taper is saturated and the fill is the full union.
///
/// The patch reaches three points either side of that, and the crossbar's free
/// end stands a point from its edge — so the fill FADES across the patch, which
/// is curvature a saturated fill does not have. What the reading says is that
/// the fade costs about half of the crease it replaced and stays a third
/// under it: the taper is not free here, and buying its share back by widening
/// `L` would readmit the bulb at every mouth, `L` being one rule for both.
#[test]
fn a_counter_deepens_over_its_crossbar_without_a_crease() {
    let ink = c_ring_with_a_crossbar();
    let x = Vec2::new(0.0, 4.5);
    let r = read(&ink, x, TAPERED);
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
    let filled = max_second_difference(&ink, x, 3.0, |p| read(&ink, p, TAPERED).s);
    let saturated = max_second_difference(&ink, x, 3.0, |p| read(&ink, p, RAMP_ONLY).s);
    let flat = max_second_difference(&ink, x, 3.0, |p| nearest_only(&ink, p));
    assert!(
        flat > 2.0 * SMOOTH,
        "the min-distance field reads {flat} in the counter, so the fixture has no crease in it \
         for the rule to fill",
    );
    assert!(
        saturated <= SMOOTH,
        "the fill itself reads {saturated} against a smooth field's {SMOOTH}, so the curvature \
         below is not the taper's (min-distance: {flat})",
    );
    assert!(
        filled < 0.6 * flat,
        "the tapered counter reads {filled} against the crease it replaced, {flat}, and the \
         saturated fill's own {saturated}",
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
    let ramps =
        [HARD, Rule { ramp: 0.3, taper: 0.0 }, RAMP_ONLY, Rule { ramp: 0.6, taper: 0.0 }, TAPERED];
    let head = |what: &str| {
        println!(
            "| {what} | min-distance | hard | ramp 0.3 | ramp 0.5 | ramp 0.6 | ramp 0.5 + taper |"
        );
        println!("|---|---|---|---|---|---|---|");
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
