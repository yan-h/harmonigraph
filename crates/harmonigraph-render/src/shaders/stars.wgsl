// Shared star geometry and premultiplied composition. Consumers supply light,
// floor, and field-level settings adapters. Keep arrays behind indexed getters:
// returning all of StarUniforms by value made Metal's spectrogram fragment path
// about 5x slower at 4K Medium (#1282).
struct StarHaloSample {
    size: vec2<f32>,
    group: u32,
    layer: u32,
};
struct StarSlice {
    offset: vec2<f32>,
    cell: f32,
    // The stars' outer radius in star pixels before each star's size draw, and
    // their shape: the solid share of the radius, 1 / (1 - solid), and the
    // glow's bend.
    radius: f32,
    solid: f32,
    ramp: f32,
    bend: f32,
    // The atlas texel, counted along its rows, this slice's first cell is
    // baked into; the cell that is; and how many it holds across and down.
    base: i32,
    origin: vec2<i32>,
    grid: vec2<i32>,
    // The band a centre is drawn from, and how far a star reaches inside its
    // own cell wherever its centre is drawn, in cells.
    width: f32,
    inner: f32,
    // 0 not drawn, 1 the whole star from its own cell, 2 the whole star from a
    // 2x2 read, 3 its inner part plus the rest from a 3x3 halo image.
    gather: u32,
    pad: u32,
};
struct StarUniforms {
    origin: vec2<f32>,
    size: vec2<f32>,
    ppp: f32,
    star_randomness: f32,
    star_life: f32,
    star_size_variation: f32,
    star_far: vec4<f32>,
    star_near: vec4<f32>,
    star_slices: array<StarSlice, 5>,
    star_halo_samples: array<StarHaloSample, 5>,
};
fn atlas_texel(index: i32) -> vec2<i32> {
    return vec2<i32>(index & (STAR_ATLAS_WIDTH - 1), index >> STAR_ATLAS_SHIFT);
}
const STAR_SLICES: u32 = 5u;
const STAR_PANE: f32 = 540.0;
// The star atlas's width in texels, a power of two (`STAR_ATLAS_WIDTH` in
// stars.rs), and its log.
const STAR_ATLAS_WIDTH: i32 = 2048;
const STAR_ATLAS_SHIFT: u32 = 11u;
// The hash's period in each slice's cells; the CPU reduces each drift by it
// (and says there why it is this wide).
const STAR_HASH_PERIOD: i32 = 65536;
// The life clock is reduced by this many lives on the CPU; a power of two, so
// masking the life index by it wraps with the clock and no life is cut short
// where the clock wraps.
const STAR_LIFE_PERIOD: u32 = 4096u;
// The share of its life a star spends fading in, and again fading out, each
// as a smoothstep.
//
// ONE star a cell, and so a dip while it turns over, rather than two half a
// life apart fading as `sin²` — which sum to exactly one and never dip, and
// were built and measured: the field's frame-mean brightness held just as
// still either way over a flat input (a standard deviation of 0.08% of it
// against 0.11% here), because the cells turn over at hashed times, and the
// second star cost 70% more starfield at 4K (88 ms against 52). One star
// fading as `sin²` over its whole life left the field a fifth darker than
// two; with the fade kept to its ends it is 5% darker.
const STAR_FADE: f32 = 0.2;
// How far up the palette the brightest-ranked star is lifted past its level.
const STAR_LIFT: f32 = 0.18;
// Where a 3x3 star's inner part starts fading, as a share of its reach.
const STAR_INNER_FADE: f32 = 0.7;
// `wash_hash`'s mixer cut into four eight-bit draws, each centred in its
// step so none is 0 or 1: fine enough for anything about a star, and a star's
// four draws take two hashes.
fn star_hash(cell: vec2<i32>, salt: u32) -> vec4<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = n ^ (salt * 0x27d4eb2du);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    let bytes = vec4<u32>(n, n >> 8u, n >> 16u, n >> 24u) & vec4<u32>(0xffu);
    return (vec4<f32>(bytes) + 0.5) / 256.0;
}

// One cell's star this frame, packed for `star_atlas`, or zero where the cell
// holds none. `cell` is the slice's cell, `salt` the slice's.
//
// x and y: the centre, from the cell's corner, in cells, as f32 bits — a
// near cell can be hundreds of pixels wide, too wide for a half float's
// thousandth of one to hold still. z: the colour, ten bits a channel, which
// is finer than any target this draws into. w: the inverse of the star's
// outer radius in star pixels and life fade times source opacity as two half
// floats. The reciprocal is baked once per
// star rather than divided out at every pixel in reach. It is never zero,
// so w is zero exactly where there is no star.
fn star_bake(s: StarSlice, cell: vec2<i32>, salt: u32, index: i32) -> vec4<u32> {
    // The period is a power of two, so a mask IS the Euclidean wrap, negative
    // cells included, without `wrap_cell`'s integer divisions.
    let hashed = cell & vec2<i32>(STAR_HASH_PERIOD - 1);
    // How far through its lives this cell is, staggered per cell. Every hash
    // below is keyed on the life, so each is a new star. The high half of the
    // key is the life plus one: the stagger hashes at zero there, and the
    // slices' salts all sit in the low half.
    let age = star_life() + star_hash(hashed, salt + 2u).x;
    let life = u32(floor(age)) & (STAR_LIFE_PERIOD - 1u);
    let key = salt + ((life + 1u) << 16u);
    // Position variation. Every life holds a star, so a depth's count is its cell size
    // alone.
    let a = star_hash(hashed, key);
    let through = fract(age);
    let centre = 0.5 + s.width * (a.xy - 0.5);
    let at = (vec2<f32>(cell) + centre + s.offset) * s.cell
        * (star_size().y / STAR_PANE) + star_size() * 0.5;
    let c = star_hash(hashed, key + 1u);
    let randomness = star_randomness();
    let paint = star_source(at, pow(c.x, 1.0 + 6.0 * randomness) * (2.0 + 6.0 * randomness), index);
    if paint.a <= 0.0 { return vec4<u32>(0u); }
    let colour = paint.rgb;
    // Its own draw, shrinking from its depth's size: a star that grew would
    // reach past what its depth's read holds.
    let radius = s.radius * exp(-2.4 * star_size_variation() * c.y);
    // It fades in over the start of its life and out over the end.
    var fade = smoothstep(0.0, STAR_FADE, through) * smoothstep(0.0, STAR_FADE, 1.0 - through);
    var tens = vec3<u32>(round(clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0)) * 1023.0));
    if paint.a != 1.0 { fade *= paint.a; }
    return vec4<u32>(
        bitcast<u32>(centre.x),
        bitcast<u32>(centre.y),
        (tens.r << 20u) | (tens.g << 10u) | tens.b,
        pack2x16float(vec2<f32>(1.0 / radius, fade)),
    );
}

// Every slice's cells on screen into the atlas: each slice's grid row after
// row from its `base`, counted along the atlas's rows, and the next slice
// straight after it. Texels past the last slice hold no star.
@fragment
fn fs_star_bake(in: TileVertex) -> @location(0) vec4<u32> {
    let texel = vec2<i32>(floor(in.position.xy));
    let index = texel.y * STAR_ATLAS_WIDTH + texel.x;
    for (var k = 0u; k < STAR_SLICES; k += 1u) {
        let s = star_slice(k);
        let at = index - s.base;
        if at >= 0 && at < s.grid.x * s.grid.y {
            let local = vec2<i32>(at % s.grid.x, at / s.grid.x);
            return star_bake(s, s.origin + local, 1000u + 3u * k, index);
        }
    }
    return vec4<u32>(0u);
}

// One star's coverage at `t`, its distance over its own outer radius: full
// out to `solid`, then the glow, eased out of the solid edge and into the
// star's edge and bent by `bend`, so the star ends at its radius whatever its
// shape. `harmonigraph_scene::star_plan::star_profile` is the same curve.
fn star_profile(s: StarSlice, t: f32) -> f32 {
    if t >= 1.0 { return 0.0; }
    let u = saturate((t - s.solid) * s.ramp);
    let x = u * u * (3.0 - 2.0 * u);
    return (1.0 - x) / (1.0 + s.bend * x);
}

// A 3x3 star's premultiplied palette color and coverage, in two parts. The
// native path draws its inner part, which stays inside its own cell; the halo
// path draws the whole star MINUS that part, so their sum is the whole star.
fn star_texel(s: StarSlice, f: vec2<f32>, index: i32, halo: bool) -> vec4<f32> {
    let t = textureLoad(
        star_atlas,
        vec2<i32>(index & (STAR_ATLAS_WIDTH - 1), index >> STAR_ATLAS_SHIFT),
        0,
    );
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let reach = s.inner * s.cell;
    if !halo && dist >= reach { return vec4<f32>(0.0); }
    let shape = unpack2x16float(t.w);
    let full = star_profile(s, dist * shape.x);
    if full <= 0.0 { return vec4<f32>(0.0); }
    let colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    let inner = full * (1.0 - smoothstep(STAR_INNER_FADE * reach, reach, dist));
    let cover = select(inner, max(full - inner, 0.0), halo) * shape.y;
    return vec4<f32>(colour * cover, cover);
}

// The low-resolution target stores the unnormalized weighted color and
// coverage of ONE slice. They must join that slice's native core before the
// usual far-to-near over; flattening all halos would change the depth order.
@fragment
fn fs_star_halo(in: TileVertex) -> @location(0) vec4<f32> {
    let step = star_size() / star_halo_sample(in.layer).size;
    let pt = in.position.xy * step;
    let sp = (pt - star_size() * 0.5) * (STAR_PANE / star_size().y);
    let s = star_slice(in.layer);
    // Keep the fractional coordinate small across drift wraps so the two
    // passes do not round differently while subtracting an offset near 65536.
    let r = sp / s.cell - fract(s.offset);
    let o = floor(r);
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var halo = vec4<f32>(0.0);
    for (var y = -1; y <= 1; y += 1) {
        let row = index + y * s.grid.x;
        let fy = f.y - f32(y);
        halo += star_texel(s, vec2<f32>(f.x + 1.0, fy), row - 1, true);
        halo += star_texel(s, vec2<f32>(f.x, fy), row, true);
        halo += star_texel(s, vec2<f32>(f.x - 1.0, fy), row + 1, true);
    }
    return halo;
}

// Each array has its own actual size and edge clamp. The depth index is
// uniform across fragments, so selecting its array introduces no spatially
// divergent branch. Uniform sampling retains the original first-array lookup.
fn star_halo_at(pt: vec2<f32>, k: u32) -> vec4<f32> {
    let sample = star_halo_sample(k);
    let uv = pt / star_size();
    switch sample.group {
        case 0u: { return textureSampleLevel(star_halos, cloud_sampler, uv, i32(sample.layer), 0.0); }
        case 1u: { return textureSampleLevel(star_halos_b, cloud_sampler, uv, i32(sample.layer), 0.0); }
        default: { return textureSampleLevel(star_halos_c, cloud_sampler, uv, i32(sample.layer), 0.0); }
    }
}

// One star whole: the 2x2 read's, and the 1x1 read's, whose stars the plan
// holds inside their own cell. A read sees every centre within its bound, so
// a star the plan holds to it is never cut.
fn star_far_texel(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    let t = textureLoad(star_atlas, atlas_texel(index), 0);
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let shape = unpack2x16float(t.w);
    let cover = star_profile(s, dist * shape.x) * shape.y;
    if cover <= 0.0 { return vec4<f32>(0.0); }
    let colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    return vec4<f32>(colour * cover, cover);
}

fn star_far_gather(s: StarSlice, r: vec2<f32>) -> vec4<f32> {
    let o = floor(r - 0.5);
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var result = vec4<f32>(0.0);
    result += star_far_texel(s, f, index);
    result += star_far_texel(s, f - vec2<f32>(1.0, 0.0), index + 1);
    result += star_far_texel(s, f - vec2<f32>(0.0, 1.0), index + s.grid.x);
    result += star_far_texel(s, f - vec2<f32>(1.0, 1.0), index + s.grid.x + 1);
    return result;
}

// The consumer's floor, then every slice laid over it far to near: within a slice
// the stars' coverages add and their colours average by coverage, and the slice
// covers what is under it by its summed coverage, capped at one. The salts
// (`fs_star_bake`) are three apart: a star hashes at its salt and the one past
// it, a cell's stagger at the second.
//
// One native core per slice, with the remaining coverage gathered into the
// halo array. Both paths use this same per-slice composition.
fn star_layers(pt: vec2<f32>, first: u32, last: u32, under: vec4<f32>) -> vec4<f32> {
    var out = under;
    let sp = (pt - star_size() * 0.5) * (STAR_PANE / star_size().y);
    for (var k = first; k < last; k += 1u) {
        let s = star_slice(k);
        let r = sp / s.cell - fract(s.offset);
        let o = floor(r);
        let f = r - o;
        let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
        let index = s.base + local.y * s.grid.x + local.x;
        var slice = vec4<f32>(0.0);
        if s.gather == 2u {
            slice = star_far_gather(s, r);
        } else if s.gather == 1u {
            slice = star_far_texel(s, f, index);
        } else if s.gather == 3u {
            slice = star_texel(s, f, index, false);
            slice += star_halo_at(pt, k);
        }
        if slice.w > 0.0 {
            let cover = min(slice.w, 1.0);
            // Preserve the spectral RGB arithmetic; alpha independently follows over.
            out = vec4<f32>(mix(out.rgb, slice.rgb / slice.w, cover), out.a + (1.0 - out.a) * cover);
        }
    }
    return out;
}

// All profiles share the far-three partition. Uniform preserves native texel
// addressing; High, Medium and Low filter smaller complete far-layer images.
override STAR_SPLIT: bool = false;
const STAR_FAR_LAYERS: u32 = 3u;

fn star_near_color(pt: vec2<f32>) -> vec4<f32> {
    var far = vec4<f32>(0.0);
    if star_far().z > 0.0 {
        far = textureSampleLevel(cloud_tone, cloud_sampler, pt / star_size(), 0.0);
    } else {
        far = textureLoad(cloud_tone, vec2<i32>(pt * star_ppp()), 0);
    }
    return star_layers(pt, STAR_FAR_LAYERS, STAR_SLICES, far);
}

fn star_color(pt: vec2<f32>) -> vec4<f32> {
    if STAR_SPLIT {
        if star_near().x > 0.0 {
            return textureSampleLevel(cloud_tone, cloud_sampler, pt / star_size(), 0.0);
        }
        return star_near_color(pt);
    }
    return star_layers(pt, 0u, STAR_SLICES, star_floor());
}

@fragment
fn fs_star_near(in: TileVertex) -> @location(0) vec4<f32> {
    // Use actual rounded dimensions, including odd panes at fractional scale.
    // This pass binds the far image; final painting binds this pass's output
    // at the same slot. Texture mix and output color conversion stay native.
    let pt = in.position.xy / star_near().xy * star_size();
    return star_near_color(pt);
}

@fragment
fn fs_star_far(in: TileVertex) -> @location(0) vec4<f32> {
    // Repeat clouded's global-pixel-to-pane-point arithmetic, including its
    // rounding at fractional display scales and nonzero pane origins.
    let position = in.position.xy + round(star_origin() * star_ppp());
    var pt = position / star_ppp() - star_origin();
    if star_far().z > 0.0 {
        pt = in.position.xy / star_far().xy * star_size();
    }
    // Layer compositing remains gamma-coded here. Depth mixing and the final
    // target's color conversion are applied once, in the final composite.
    return star_layers(pt, 0u, STAR_FAR_LAYERS, star_floor());
}

