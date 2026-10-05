// Shared star geometry and premultiplied composition. Consumers supply light,
// floor, and field-level settings adapters. Keep arrays behind indexed getters:
// returning all of StarUniforms by value made Metal's spectrogram fragment path
// about 5x slower at 4K Medium (#1282).
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
    // The band a centre is drawn from, in cells.
    width: f32,
    // The inverse of the narrowest radius a star is drawn at: a texel of the
    // star image, or what a 3x3 read holds where that is less (`star_slices`).
    inverse_floor: f32,
    // 0 not drawn, 1 the whole star from its own cell, 2 from a 2x2 read, 3
    // from a 3x3 read.
    gather: u32,
    // How far its stars fade between lives; below 1 a star keeps its place
    // across them (`star_draw`).
    twinkle: f32,
};
struct StarUniforms {
    // The pane, and the star image's actual size, in device pixels.
    size: vec2<f32>,
    star_image_size: vec2<f32>,
    star_randomness: f32,
    star_life: f32,
    star_size_variation: f32,
    pad: f32,
    star_slices: array<StarSlice, 5>,
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

// A star's brightness rank from its draw, normalised to a mean of ONE: most
// stars below it, a rare bright one far above, steeper with `Brightness
// variation`.
fn star_rank(draw: f32) -> f32 {
    let randomness = star_randomness();
    return pow(draw, 1.0 + 6.0 * randomness) * (2.0 + 6.0 * randomness);
}

// One cell's star this frame, as the bake draws it and the spectrogram's
// colour memory remembers it.
struct StarDraw {
    // Its centre, from the cell's corner, in cells.
    centre: vec2<f32>,
    // This life's brightness and size draws, the neighbouring life's, and how
    // far the star has turned into that one: above zero only for a star that
    // keeps its place, within `STAR_FADE` of the turnover.
    own: vec2<f32>,
    other: vec2<f32>,
    blend: f32,
    // Its coverage this far through its life.
    fade: f32,
    // The cell's stagger and the life it is in, and whether its star keeps its
    // place across lives: the colour memory carries one through them.
    stagger: f32,
    life: u32,
    held: bool,
};

// `hashed` is the slice's cell wrapped to the hash's period, `salt` the slice's.
fn star_draw(s: StarSlice, hashed: vec2<i32>, salt: u32) -> StarDraw {
    // How far through its lives this cell is, staggered per cell. Every hash
    // keyed on the life is a new star. The high half of the key is the life
    // plus one: the stagger hashes at zero there, and the slices' salts all
    // sit in the low half.
    var d: StarDraw;
    d.stagger = star_hash(hashed, salt + 2u).x;
    let age = star_life() + d.stagger;
    let life = u32(floor(age)) & (STAR_LIFE_PERIOD - 1u);
    d.life = life;
    let key = salt + ((life + 1u) << 16u);
    let through = fract(age);
    let held = s.twinkle < 1.0;
    d.held = held;
    // Position variation. Every life holds a star, so a depth's count is its
    // cell size alone. A star that keeps its place hashes it at the salt
    // alone, which no life's key is.
    let a = star_hash(hashed, select(key, salt, held));
    d.centre = 0.5 + s.width * (a.xy - 0.5);
    d.own = star_hash(hashed, key + 1u).xy;
    d.other = d.own;
    d.blend = 0.0;
    // It fades in over the start of its life and out over the end, as far
    // as its slice twinkles: all the way at 1, as before the dial.
    let dip = smoothstep(0.0, STAR_FADE, through) * smoothstep(0.0, STAR_FADE, 1.0 - through);
    d.fade = 1.0 - s.twinkle * (1.0 - dip);
    let early = through < STAR_FADE;
    if held && (early || through > 1.0 - STAR_FADE) {
        // Where it keeps its place it turns into the next life's star over
        // the window it would have faded through: from the previous life
        // early in this one, toward the next late in it. The life clock
        // wraps with the mask, so life 0's previous is the period's last.
        let other = select(life + 1u, life - 1u, early) & (STAR_LIFE_PERIOD - 1u);
        d.other = star_hash(hashed, salt + ((other + 1u) << 16u) + 1u).xy;
        let later = smoothstep(-STAR_FADE, STAR_FADE, select(through - 1.0, through, early));
        d.blend = select(later, 1.0 - later, early);
    }
    return d;
}

// One cell's star this frame, packed for `star_atlas`, or zero where the cell
// holds none. `cell` is the slice's cell, `salt` the slice's.
//
// x and y: the centre, from the cell's corner, in cells, as f32 bits — a
// near cell can be hundreds of pixels wide, too wide for a half float's
// thousandth of one to hold still. z: the colour, ten bits a channel, which
// is finer than any target this draws into. w: the inverse of the star's
// outer radius in star pixels and its life fade as two half floats. A source
// gives a star whole or not at all. The reciprocal is baked once per
// star rather than divided out at every pixel in reach. It is never zero,
// so w is zero exactly where there is no star.
fn star_bake(s: StarSlice, cell: vec2<i32>, salt: u32, index: i32) -> vec4<u32> {
    // The period is a power of two, so a mask IS the Euclidean wrap, negative
    // cells included, without `wrap_cell`'s integer divisions.
    let hashed = cell & vec2<i32>(STAR_HASH_PERIOD - 1);
    let d = star_draw(s, hashed, salt);
    let at = (vec2<f32>(cell) + d.centre + s.offset) * s.cell
        * (star_size().y / STAR_PANE) + star_size() * 0.5;
    let paint = star_source(at, star_rank(d.own.x), index);
    if paint.a <= 0.0 { return vec4<u32>(0u); }
    var colour = paint.rgb;
    // Its own draw, shrinking from its depth's size: a star that grew would
    // reach past what its depth's read holds.
    var radius = s.radius * exp(-2.4 * star_size_variation() * d.own.y);
    if d.blend > 0.0 {
        // The same place in both lives, so the same source: only the draws
        // differ.
        colour = mix(colour, star_source(at, star_rank(d.other.x), index).rgb, d.blend);
        radius = mix(radius, s.radius * exp(-2.4 * star_size_variation() * d.other.y), d.blend);
    }
    var tens = vec3<u32>(round(clamp(colour, vec3<f32>(0.0), vec3<f32>(1.0)) * 1023.0));
    return vec4<u32>(
        bitcast<u32>(d.centre.x),
        bitcast<u32>(d.centre.y),
        (tens.r << 20u) | (tens.g << 10u) | tens.b,
        pack2x16float(vec2<f32>(1.0 / radius, d.fade)),
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

// One cell's star, premultiplied colour and coverage, at `f`, the pixel's
// place from that cell's corner in cells.
fn star_texel(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    let t = textureLoad(star_atlas, atlas_texel(index), 0);
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let shape = unpack2x16float(t.w);
    // A star narrower than a texel would show only where a texel centre fell
    // inside it: one under the floor's radius is drawn at it instead, dimmed
    // by the ratio of the areas so it keeps its light.
    let inverse = min(shape.x, s.inverse_floor);
    let dim = inverse / shape.x;
    let cover = star_profile(s, dist * inverse) * shape.y * dim * dim;
    if cover <= 0.0 { return vec4<f32>(0.0); }
    let colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    return vec4<f32>(colour * cover, cover);
}

// The four cells whose centres surround the pixel. A read sees every centre
// within its bound, so a star the plan holds to it is never cut.
fn star_gather2(s: StarSlice, r: vec2<f32>) -> vec4<f32> {
    let o = floor(r - 0.5);
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var result = vec4<f32>(0.0);
    result += star_texel(s, f, index);
    result += star_texel(s, f - vec2<f32>(1.0, 0.0), index + 1);
    result += star_texel(s, f - vec2<f32>(0.0, 1.0), index + s.grid.x);
    result += star_texel(s, f - vec2<f32>(1.0, 1.0), index + s.grid.x + 1);
    return result;
}

// The pixel's own cell and its eight neighbours.
fn star_gather3(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    var result = vec4<f32>(0.0);
    for (var y = -1; y <= 1; y += 1) {
        let row = index + y * s.grid.x;
        let fy = f.y - f32(y);
        result += star_texel(s, vec2<f32>(f.x + 1.0, fy), row - 1);
        result += star_texel(s, vec2<f32>(f.x, fy), row);
        result += star_texel(s, vec2<f32>(f.x - 1.0, fy), row + 1);
    }
    return result;
}

// The consumer's floor, then every slice laid over it far to near: within a slice
// the stars' coverages add and their colours average by coverage, and the slice
// covers what is under it by its summed coverage, capped at one. The salts
// (`fs_star_bake`) are three apart: a star hashes at its salt and the one past
// it, a cell's stagger at the second.
fn star_layers(pt: vec2<f32>) -> vec4<f32> {
    var out = star_floor();
    let sp = (pt - star_size() * 0.5) * (STAR_PANE / star_size().y);
    for (var k = 0u; k < STAR_SLICES; k += 1u) {
        let s = star_slice(k);
        let r = sp / s.cell - fract(s.offset);
        let o = floor(r);
        let f = r - o;
        let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
        let index = s.base + local.y * s.grid.x + local.x;
        var slice = vec4<f32>(0.0);
        if s.gather == 1u {
            slice = star_texel(s, f, index);
        } else if s.gather == 2u {
            slice = star_gather2(s, r);
        } else if s.gather == 3u {
            slice = star_gather3(s, f, index);
        }
        if slice.w > 0.0 {
            let cover = min(slice.w, 1.0);
            // Preserve the spectral RGB arithmetic; alpha independently follows over.
            out = vec4<f32>(mix(out.rgb, slice.rgb / slice.w, cover), out.a + (1.0 - out.a) * cover);
        }
    }
    return out;
}

// Every slice into the star image, at `Stars resolution` of the pane. Its
// actual rounded dimensions place each texel, so odd panes at fractional
// scale stay aligned. Layer compositing remains gamma-coded here; depth mixing
// and the final target's colour conversion are applied once, in the composite.
@fragment
fn fs_stars(in: TileVertex) -> @location(0) vec4<f32> {
    return star_layers(in.position.xy / star_image_size() * star_size());
}

// The star image under the pane point `pt`, filtered. The host reads its own
// binding in `star_image_at`, so neither host has to name it for the other.
fn star_color(pt: vec2<f32>) -> vec4<f32> {
    return star_image_at(pt / star_size());
}
