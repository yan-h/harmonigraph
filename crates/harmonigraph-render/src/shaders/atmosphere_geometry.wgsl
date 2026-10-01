// Pure material geometry, shared by scalar spectrogram light and RGBA lattice light.
const CLOUD_TILE_ROT_SIN: f32 = 0.6;
const CLOUD_TILE_ROT_COS: f32 = 0.8;
fn rotate_watercolor_tile_vector_for(v: vec2<f32>, pitch_vertical: u32) -> vec2<f32> {
    let semantic = select(vec2<f32>(v.y, v.x), v, pitch_vertical == 1u);
    let turned = vec2<f32>(
        CLOUD_TILE_ROT_COS * semantic.x - CLOUD_TILE_ROT_SIN * semantic.y,
        CLOUD_TILE_ROT_SIN * semantic.x + CLOUD_TILE_ROT_COS * semantic.y,
    );
    return select(vec2<f32>(turned.y, turned.x), turned, pitch_vertical == 1u);
}

fn watercolor_tile_uv_for(r: vec2<f32>, period: f32, pitch_vertical: u32) -> vec2<f32> {
    let semantic = select(vec2<f32>(r.y, r.x), r, pitch_vertical == 1u);
    return vec2<f32>(
        CLOUD_TILE_ROT_COS * semantic.x + CLOUD_TILE_ROT_SIN * semantic.y,
        -CLOUD_TILE_ROT_SIN * semantic.x + CLOUD_TILE_ROT_COS * semantic.y,
    ) / period;
}

// The cell a hash is taken at, folded onto the tile when one is being baked.
//
// The tile is square in its OWN coordinates. Rotating the already-periodic
// result where it is read keeps every lattice the walk uses exact; trying
// instead to wrap the world-cell hashes on the 3-4-5 vectors would leave the
// 2.1x and 0.9x octaves on fractional cells and draw a seam.
//
// WGSL's `%` truncates toward zero, so `-1 % 20` is `-1` and the second fold is
// what lands a negative cell in the range. A period of 0 returns the cell
// whole: no production pass asks for it, but it is the unwrapped walk the
// tests hold the tile against.
fn wrap_cell_for_tile(cell: vec2<i32>, period: i32) -> vec2<i32> {
    if period <= 0 {
        return cell;
    }
    return ((cell % vec2<i32>(period)) + vec2<i32>(period)) % vec2<i32>(period);
}

fn wrap_cell(cell: vec2<i32>, period: i32) -> vec2<i32> {
    return wrap_cell_for_tile(cell, period);
}

// The ring each octave walks, and the four numbers that decide whether walking
// it is enough. THESE ARE A PROOF and not four tastes, held against the shipped
// text of this file by `the_wash_grid_covers_the_plane_and_the_ring_holds_it`.
//
// **Coverage.** A centre sits at its cell's middle give or take `JITTER / 2` on
// each axis, so it can be `(JITTER / 2) * sqrt(2)` from that middle in any
// direction. The point hardest to reach is a lattice corner with all four
// cells touching it pushed away from it, `0.5 * sqrt(2) + (JITTER / 2) * sqrt(2)`
// from every one of them, and the SMALLEST radius a glob can draw has to clear
// that. An uncovered point is not a dim spot — it is a pixel that reads its own
// light with no glob's centre to borrow, so the lookup falls off a cliff from
// most of a radius to nothing.
//
// **Reach.** A cell `RING + 1` out can put its centre no nearer than
// `RING + 1.5 - (JITTER / 2) * sqrt(2)` from the pixel's own cell origin, and the
// pixel is at most 1 past that origin, so the LARGEST rim has to stay under
// `RING + 0.5 - (JITTER / 2) * sqrt(2)` or a glob the ring never visits can cover
// the pixel — which is a step on the cell grid every time `floor(r)` moves.
//
// The largest RIM is the largest radius, and it was not always. A retired
// `Ragged` dial pushed a rim OUTWARD by up to 30% of its own radius and never
// inward, so the reach bound carried `RADIUS_MAX * (1 + RAGGED)` while coverage
// read `RADIUS_MIN` untouched. One-sided also meant every rim sat about 15%
// OUTSIDE its radius on average at the setting that shipped, so the band was
// scaled by 1.15 when the wobble went, and a default glob is the size it always
// drew. What the reach bound stops carrying is slack: 1.91 against a bound of
// 2.217, where the wobble left 2.7% of a radius. Spending it on a wider band
// than 1.63:1 is a LOOK change and is deliberately not taken here.
//
// Both bounds are about globs that COVER the pixel, which is what the visible
// glob, the one beneath it and `cover` are all read off. The front used for
// bleed is chosen from the two nearest non-covering globs in `wash_scan`.
//
// A 5x5 ring rather than 3x3, and it is the jitter and the variety that buy it.
// At 3x3 these same inequalities leave a radius band of about 1.2:1 with a
// jitter of 0.20 — a nearly regular grid of nearly equal globs, which is the one
// thing this look cannot be, since a field of DIFFERENT SIZED globs is what was
// asked for. The wider ring costs a second pass over 25 cells instead of 9 and
// buys jitter 0.40 and a 1.63:1 radius band, with 15.4% of a radius spare on
// coverage and 16.1% on reach — the room the retired wobble used to spend.
const WASH_RING: i32 = 2;
const WASH_JITTER: f32 = 0.40;
const WASH_RADIUS_MIN: f32 = 1.17;
const WASH_RADIUS_MAX: f32 = 1.91;

// How many cells cross one cloud unit at `Patch size` 1x — see `wash_cloud_tone`,
// where it is chosen so a glob comes out the width the prototype's J2 drew
// rather than so the CELLS come out at J2's count.
const WASH_CELLS: f32 = 5.25;

// The finer octave: how much smaller its cells are, and how many of them carry a
// glob at all. It is sparse on purpose — a big wash sometimes carries a small one
// and sometimes sits beside it, and where two washes meet the tone steps, which
// is where the reference's tones come from. Only the BASE octave owes coverage;
// a pixel no fine glob reaches simply shows the coarse wash under it.
const WASH_LACUNARITY: f32 = 2.1;
const WASH_FINE_OCCUPANCY: f32 = 0.20;

// The domain warp's maximum displacement and noise frequency, in cells.
// The frequency must close over the tile period, like the finer octave.
const WASH_WARP: f32 = 0.45;
const WASH_WARP_SCALE: f32 = 0.9;

// Three 10-bit fractions off a salted cell hash. Two of these per cell: one for
// the paint order and the occupancy draw, one for the centre and the radius.
// Five channels is what the construction needs, and one word holds three.
fn wash_hash(cell: vec2<i32>, salt: u32) -> vec3<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = n ^ (salt * 0x27d4eb2du);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    return vec3<f32>(
        f32(n & 0x3ffu) / 1023.0,
        f32((n >> 10u) & 0x3ffu) / 1023.0,
        f32((n >> 20u) & 0x3ffu) / 1023.0,
    );
}

// Smooth value noise, two octaves. Used for the SHARED domain warp only —
// evaluated once per pixel and then read by every glob of every octave, which is
// what keeps neighbouring globs leaning together along a shared boundary instead
// of each wandering off on its own.
fn wash_noise(p: vec2<f32>, salt: u32, period: i32) -> f32 {
    let b = floor(p);
    let f = p - b;
    let t = f * f * (3.0 - 2.0 * f);
    let i = vec2<i32>(b);
    let n00 = wash_hash(wrap_cell(i, period), salt).x;
    let n10 = wash_hash(wrap_cell(i + vec2<i32>(1, 0), period), salt).x;
    let n01 = wash_hash(wrap_cell(i + vec2<i32>(0, 1), period), salt).x;
    let n11 = wash_hash(wrap_cell(i + vec2<i32>(1, 1), period), salt).x;
    return mix(mix(n00, n10, t.x), mix(n01, n11, t.x), t.y);
}
// Where this noise's second octave sits, and the one constant the TILE changes.
//
// `WASH_FBM_FINE` is an irrational-looking 2.07 exactly so the two octaves never
// line up, and no tile period makes `2.07 * P` a whole number of the finer
// lattice's cells — so a tiled walk runs it at exactly 2 instead, which doubles
// the period with it and tiles for every `P` the coarse lattice already does.
// A period of 0, the unwrapped walk that no production pass draws since #1100,
// keeps 2.07 bit for bit.
const WASH_FBM_FINE: f32 = 2.07;
const WASH_FBM_FINE_TILED: f32 = 2.0;
fn wash_fbm(p: vec2<f32>, salt: u32, period: i32) -> f32 {
    let lacunarity = select(WASH_FBM_FINE, WASH_FBM_FINE_TILED, period > 0);
    let coarse = wash_noise(p, salt, period);
    let fine = wash_noise(p * lacunarity + vec2<f32>(13.1, -7.3), salt + 31u, period * 2);
    return (coarse + 0.5 * fine) / 1.5;
}

struct Glob {
    centre: vec2<f32>,
    // Where the pixel sits on this glob's rim: under 1 is inside it. A rim
    // coordinate rather than a distance, so feather and bleed are measured
    // in fractions of each glob's own radius.
    edge: f32,
    order: f32,
}

// One cell's glob, at the pixel `r` — both in this octave's cell units. `period`
// folds the cell the two hashes are taken at and nothing else, so the centre
// below is still this cell's own (see `wrap_cell`).
fn wash_glob(cell: vec2<i32>, salt: u32, r: vec2<f32>, occupancy: f32, period: i32) -> Glob {
    let hashed = wrap_cell(cell, period);
    let g = wash_hash(hashed, salt + 77u);
    var out: Glob;
    out.order = g.x;
    // A cell the occupancy draw missed carries no glob, and that is four cells
    // in five of the finer octave, so it is answered before the geometry is
    // worked out: the second hash and the square root are most of what a cell
    // costs. Its rim is put out of reach, where `wash_scan` does exactly
    // nothing with it — `1 - 1e9` rounds to `-1e9` in an f32, which is the
    // value every running nearest starts at and no `>` passes, and it adds a
    // clamped zero to the cover. So the early return draws the picture the
    // uniform loop drew, bit for bit.
    //
    // The compare takes the boundary because `wash_hash` returns
    // `(n & 0x3ff) / 1023`, which is a CLOSED range: a channel can be exactly
    // 1.0, and the base octave is scanned at an occupancy of exactly 1.0. Under
    // a strict `>=` those two 1.0s met and the base octave dropped about one
    // cell in 1024 — a hole the coverage half of the proof above forbids, since
    // its hardest point is a lattice corner reached by all four cells touching
    // it. The finer octave cannot tell the two compares apart: `0.20 * 1023` is
    // 204.6, so no hash value lands on `WASH_FINE_OCCUPANCY` at all.
    if g.y > occupancy {
        out.centre = r;
        out.edge = 1.0e9;
        return out;
    }
    let h = wash_hash(hashed, salt);
    let centre = vec2<f32>(cell) + 0.5 + (h.xy - 0.5) * WASH_JITTER;
    // Each glob draws its own radius from the whole band the proof above
    // allows. That was the top of a `Variety` dial, which is where it shipped
    // and where it stays: the band is 1.63:1 and the paint order decides which
    // glob a pixel shows, so the dial moved a twentieth of the pane end to end,
    // and the size range the look is after comes from `Fine layer mix` instead.
    //
    // A `Wander` dial turned each centre about its own cell here, on a hashed
    // rate. It could only ever TURN the jitter — a travel would break the reach
    // bound — and a quarter of a cell swung round once a minute is a point or
    // two of movement under a whole field already drifting faster than that.
    let radius = mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, h.z);
    out.centre = centre;
    out.edge = length(r - centre) / radius;
    return out;
}

struct Wash {
    // The visible glob's centre, and the centre of the glob directly beneath it
    // — what its own rim dissolves INTO.
    centre: vec2<f32>,
    under: vec2<f32>,
    has_under: bool,
    // The nearest glob painted AFTER the visible one, which is the arc about to
    // take this pixel, and how near it is as `1 - edge` (never above 0).
    front: vec2<f32>,
    near: f32,
    // Where the pixel sits on the visible glob's rim, and whether any glob
    // covers it at all — the latter is what a finer wash is composited by.
    edge: f32,
    cover: f32,
}

// One octave, in ONE walk of the ring.
//
// Two things come out of it. The two highest-ordered globs COVERING the pixel
// are the one it shows and the one its rim dissolves into, and both are a plain
// running top-two. The FRONT — the glob painted after
// the visible one whose arc is about to take this pixel — is the awkward one: it
// is defined against an answer the same walk is still computing, since the
// visible glob's order is not known until the last cell.
//
// Keep the two nearest non-covering globs, then choose the higher-ordered
// neighbour for the lookup bleed. This retains the established single-walk
// geometry; a full second search would change which edges blend together.
fn wash_scan(r: vec2<f32>, salt: u32, occupancy: f32, period: i32) -> Wash {
    var out: Wash;
    // An uncovered pixel reads its own light, unmoved. Unreachable for the
    // base octave while the constants hold — see
    // the proof above — and the ordinary case for a sparse finer one, which is
    // composited by `cover` and so never shows it.
    out.centre = r;
    out.under = r;
    out.front = r;
    out.near = -1.0e9;
    out.edge = 1.0;
    out.cover = 0.0;
    var best = -1.0e9;
    var second = -1.0e9;
    // The two nearest globs the pixel is OUTSIDE, with the order each was
    // painted at, so the front can be chosen once `best` has settled.
    var near_a = -1.0e9;
    var near_b = -1.0e9;
    var order_a = -1.0e9;
    var order_b = -1.0e9;
    var front_a = r;
    var front_b = r;
    let base = vec2<i32>(floor(r));
    for (var j = -WASH_RING; j <= WASH_RING; j += 1) {
        for (var i = -WASH_RING; i <= WASH_RING; i += 1) {
            let glob = wash_glob(base + vec2<i32>(i, j), salt, r, occupancy, period);
            let prox = 1.0 - glob.edge;
            out.cover = max(out.cover, clamp(prox / 0.05, 0.0, 1.0));
            if glob.edge < 1.0 {
                if glob.order > best {
                    second = best;
                    out.under = out.centre;
                    best = glob.order;
                    out.centre = glob.centre;
                    out.edge = glob.edge;
                } else if glob.order > second {
                    second = glob.order;
                    out.under = glob.centre;
                }
            } else if prox > near_a {
                near_b = near_a;
                order_b = order_a;
                front_b = front_a;
                near_a = prox;
                order_a = glob.order;
                front_a = glob.centre;
            } else if prox > near_b {
                near_b = prox;
                order_b = glob.order;
                front_b = glob.centre;
            }
        }
    }
    // The further of the two first, so the nearer one wins if both qualify.
    if order_b > best {
        out.near = near_b;
        out.front = front_b;
    }
    if order_a > best {
        out.near = near_a;
        out.front = front_a;
    }
    out.has_under = second >= 0.0;
    return out;
}

// Each octave stores its lookup offset, an independent signed brightness draw,
// and how far the pixel lies outside the arc of the glob painted over it: in
// that glob's radii, clamped to 0..1, and 1 where no later glob is near. The
// spectrogram's `Edge pooling` dials shape a tide line out of that distance
// after the bake, so dragging any of them is not a rebake.
struct Wet {
    offset: vec2<f32>,
    brightness: f32,
    gap: f32,
};

// The cell walk chooses the lookup. `Edge feathering` feathers and bleeds that lookup
// across glob boundaries. Brightness follows the same feathering, with a hash
// independent of paint order, occupancy, size and position jitter.
fn wash_brightness(centre: vec2<f32>, salt: u32, period: i32) -> f32 {
    return 2.0 * wash_hash(wrap_cell(vec2<i32>(floor(centre)), period), salt + 197u).x - 1.0;
}

fn wash_wet(f: Wash, r: vec2<f32>, fuzz: f32, salt: u32, period: i32) -> Wet {
    let feather = 0.10 + 0.80 * fuzz;
    let bleed = 0.12 + 0.78 * fuzz;

    var look = f.centre;
    // FEATHER: the visible wash dissolves at its own rim into whatever lies
    // beneath, reaching half and half exactly on the boundary so the two sides
    // meet. This is the whole of the fuzziness AND the whole of the
    // antialiasing — there is no supersampling anywhere in this path.
    var fa = clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    fa = fa * fa * (3.0 - 2.0 * fa) * 0.5;
    look = mix(look, f.under, fa);
    // BLEED: the reading crossfades toward the glob about to cover this pixel,
    // over a band at their shared edge. Still one tap, and dialled up it is what
    // makes neighbouring washes run into each other.
    var bl = clamp((f.near + bleed) / bleed, 0.0, 1.0);
    bl = bl * bl * (3.0 - 2.0 * bl) * 0.5;
    look = mix(look, f.front, bl);

    // Missing-glob lookups are the moving pixel coordinate, not a glob.
    // Keep them neutral: even uncovered texels filter into the painted rim.
    let centre = select(0.0, wash_brightness(f.centre, salt, period), f.cover > 0.0);
    let under = select(0.0, wash_brightness(f.under, salt, period), f.has_under);
    let brightness = mix(
        mix(centre, under, fa),
        wash_brightness(f.front, salt, period), bl);
    // Where the tide line lies: on the OVERLAPPED glob, measured out from the
    // front glob's arc. `near` is never above 0, since the front is a glob
    // that does not cover the pixel. Not feathered, as before #1038.
    //
    // #909's surface pigment (each glob darkening toward its own rim) is not
    // here: without that version's paper lift it measured as a near-uniform
    // dim over 99% of the pane rather than a shape, and Yan dropped it.
    return Wet(look - r, brightness, clamp(-f.near, 0.0, 1.0));
}

// The whole of the wash's geometry at a point, in cells: what each octave
// carries and how much of the pixel the finer one covers.
//
// Nine numbers, none of which reads the light, the sound or the clock —
// which is exactly why `fs_cloud_tile` can bake them into three tile targets
// and the per-frame shader can read them back. The bake always walks
// both octaves, because `Fine layer mix` is a mix over channels the tile already holds
// and so is deliberately not in the tile's key.
struct WashField {
    coarse: Wet,
    fine: Wet,
    cover: f32,
};

fn wash_field(r: vec2<f32>, period: i32, fuzz: f32, lobe: f32) -> WashField {
    // One shared field, evaluated once per pixel and then read by every glob of
    // every octave: a domain warp of glob space, which is what stops a glob
    // being a circle. It is read at the UNWARPED point and stays small — a heavy
    // warp draws flames — and it sits on a lattice `WASH_WARP_SCALE` cells
    // across, which is the period it tiles at.
    var warped = r;
    if lobe > 0.0 {
        let amp = WASH_WARP * lobe;
        let warp_period = i32(round(WASH_WARP_SCALE * f32(period)));
        warped += amp * 2.0 * vec2<f32>(
            wash_fbm(r * WASH_WARP_SCALE, 71u, warp_period) - 0.5,
            wash_fbm(r * WASH_WARP_SCALE + vec2<f32>(37.0, -19.0), 73u, warp_period) - 0.5,
        );
    }
    var out: WashField;
    out.coarse = wash_wet(wash_scan(warped, 1u, 1.0, period), warped, fuzz, 1u, period);
    // Coarse to fine, the finer octave a translucent wash over the one below and
    // sparse, so a big wash sometimes carries a small one and sometimes sits
    // beside it.
    let fine_r = warped * WASH_LACUNARITY + vec2<f32>(17.3, 5.9);
    let fine =
        wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, i32(round(WASH_LACUNARITY * f32(period))));
    out.fine = wash_wet(fine, fine_r, fuzz, 2u, i32(round(WASH_LACUNARITY * f32(period))));
    out.cover = fine.cover;
    return out;
}

// Apply a zero-mean draw AFTER coloring, in linear light. One gain preserves
// hue; symmetric headroom preserves expected RGB without clipping bright draws.
// A finite view of a random field fluctuates around that mean. `ceiling` is
// alpha for premultiplied light and 1 for an opaque spectral color.
fn wash_vary_brightness(color: vec3<f32>, ceiling: f32, draw: f32, amount: f32) -> vec3<f32> {
    let peak = max(color.r, max(color.g, color.b));
    let headroom = clamp((ceiling - peak) / max(peak, 1.0e-6), 0.0, 1.0);
    return color * (1.0 + amount * draw * headroom);
}

// Velvet Scales: the S1 prototype's normalized mixture of body light.
// Each body samples its own center BEFORE weighting. Sampling the source at
// the weighted average center would recreate raw spectrogram edges instead.
// Geometry is a fixed, nonperiodic field; drift translates it without morphing.
fn velvet_hash(cell: vec2<i32>, seed: u32) -> f32 {
    var h = bitcast<u32>(cell.x) * 1597334677u ^ bitcast<u32>(cell.y) * 3812015801u ^ seed * 2798796415u;
    h = (h ^ (h >> 16u)) * 2246822519u;
    h = (h ^ (h >> 13u)) * 3266489917u;
    h = h ^ (h >> 16u);
    return f32(h >> 8u) / 16777216.0;
}
fn velvet_warp(p: vec2<f32>, irregularity: f32) -> vec2<f32> {
    return vec2<f32>(0.43 * sin(p.y * 0.61 + sin(p.x * 0.24)),
        0.40 * sin(p.x * 0.53 + sin(p.y * 0.31))) * (irregularity / 0.8);
}
struct VelvetBody { center: vec2<f32>, weight: f32 };
// The body's radius under a superellipse norm: exponent 2 is the circle, and
// `Squareness` raises it exponentially toward 12, where the scale is a square
// with barely rounded corners. Divided through by the larger axis first so the
// powers stay near one.
fn velvet_square_radius(v: vec2<f32>, square: f32) -> f32 {
    let a = abs(v);
    let m = max(max(a.x, a.y), 1e-6);
    let p = 2.0 * pow(6.0, square);
    return m * pow(pow(a.x / m, p) + pow(a.y / m, p), 1.0 / p);
}
// `form` is x `Squareness`, y `Tilt`.
fn velvet_body(q: vec2<f32>, cell: vec2<i32>, dials: vec4<f32>, form: vec2<f32>) -> VelvetBody {
    let a = velvet_hash(cell, 0u);
    let b = velvet_hash(cell, 1u);
    let center = vec2<f32>(cell) + 0.5 + (vec2<f32>(a, b) - 0.5) * dials.y;
    let delta = q - center;
    // Compact support is exactly zero here. Reject before trigonometry,
    // radius, priority and source-center warp; no contributor is truncated.
    if any(abs(delta) >= vec2<f32>(1.9)) {
        return VelvetBody(vec2<f32>(0.0), 0.0);
    }
    let c = velvet_hash(cell, 2u);
    let d = velvet_hash(cell, 3u);
    let angle = (0.2 + (c - 0.5) * 0.85) * form.y;
    let ca = cos(angle); let sa = sin(angle);
    let size = 0.875 + 0.9 * dials.w * (d - 0.5);
    let rotated = vec2<f32>(delta.x * ca + delta.y * sa, -delta.x * sa + delta.y * ca) / size;
    let rx = rotated.x / mix(1.0, clamp(1.0 - 0.38 * rotated.y, 0.52, 1.3), dials.z);
    let shaped = vec2<f32>(rx, rotated.y + dials.z * 0.17 * rx * rx);
    // Branched so Squareness 0 pays for no powers and keeps the round scale's own `length`.
    var r = length(shaped);
    if form.x > 0.0 {
        r = velvet_square_radius(shaped, form.x);
    }
    let support = (1.0 - smoothstep(1.5, 1.9, abs(delta.x))) * (1.0 - smoothstep(1.5, 1.9, abs(delta.y)));
    let body = 1.0 - smoothstep(0.85 - dials.x, 0.85 + dials.x, r);
    let weight = (body + 0.025 * exp(-2.0 * r * r)) * exp(2.8 * (c - 0.5)) * support;
    return VelvetBody(center - velvet_warp(center, dials.y), weight);
}
fn velvet_light(source: texture_2d<f32>, source_sampler: sampler, uv: vec2<f32>, radius: vec2<f32>) -> vec4<f32> {
    return 0.4 * textureSampleLevel(source, source_sampler, uv, 0.0)
        + 0.15 * (textureSampleLevel(source, source_sampler, uv + vec2<f32>(radius.x, 0.0), 0.0)
        + textureSampleLevel(source, source_sampler, uv - vec2<f32>(radius.x, 0.0), 0.0)
        + textureSampleLevel(source, source_sampler, uv + vec2<f32>(0.0, radius.y), 0.0)
        + textureSampleLevel(source, source_sampler, uv - vec2<f32>(0.0, radius.y), 0.0));
}
fn velvet_material(source: texture_2d<f32>, source_sampler: sampler, pt: vec2<f32>, size: vec2<f32>, cell_size: f32, drift: vec2<f32>, dials: vec4<f32>, form: vec2<f32>) -> vec4<f32> {
    let p = pt / cell_size + drift;
    let q = p + velvet_warp(p, dials.y);
    let base = vec2<i32>(floor(q));
    var light = vec4<f32>(0.0);
    var weight = 0.0;
    for (var j = -2; j <= 2; j++) {
        for (var i = -2; i <= 2; i++) {
            let body = velvet_body(q, base + vec2<i32>(i, j), dials, form);
            if body.weight > 0.0 {
                let uv = (body.center - drift) * cell_size / size;
                light += body.weight * velvet_light(source, source_sampler, uv, vec2<f32>(0.14 * cell_size) / size);
                weight += body.weight;
            }
        }
    }
    // The soft skirts cover the entire field. No raw-source fallback: a flat
    // source stays flat, and full material depth contains only body light.
    return light / max(weight, 1e-20);
}
