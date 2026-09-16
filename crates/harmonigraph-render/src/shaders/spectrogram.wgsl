// The spectrogram's heatmap, read per fragment out of the aggregator's slab
// grid rather than sampled out of a picture something else composed.
//
// One quad's worth of geometry carries two coordinates: where the fragment
// sits along the run of slabs, and where it sits across the visible pitch
// range. Everything else — which buckets are under this pixel, how they
// combine, what colour that is — is worked out here, from the uniforms and
// the grid.
//
// Coordinates arrive in egui POINTS, exactly as egui's own vertex shader takes
// them, and `vs_heatmap` does the same screen->clip mapping.

struct Locals {
    /// Where the viewport being drawn into starts, in egui points, and how big
    /// it is. The draw into the egui pass takes the whole surface (origin 0).
    origin_points: vec2<f32>,
    viewport_points: vec2<f32>,
    /// The visible pitch range: MIDI at pitch fraction 0, and semitones across.
    min_midi: f32,
    span: f32,
    /// MIDI of bucket 0's lower edge, and how many buckets one semitone holds.
    spectrum_min_midi: f32,
    bins_per_semitone: f32,
    /// The level mapping, affine in the stored byte and in MIDI: a bucket's
    /// level is `level0 + level_per_step * byte + level_per_midi * midi`,
    /// clamped, which is the 0..1 the gradient is indexed by.
    level0: f32,
    level_per_step: f32,
    level_per_midi: f32,
    /// Pixels the pane spends on the pitch axis, which sets how wide one
    /// fragment's footprint is and so which arm of the resample it takes.
    rows: u32,
    /// Buckets in one slab, and the bytes one slab occupies in `grid` — the
    /// latter padded to a multiple of 4 so a slab can be written on its own.
    bins: u32,
    stride: u32,
    /// Slots the ring holds, and the slot the run's first slab sits in.
    capacity: u32,
    first_slot: u32,
    /// Slabs in the visible run.
    run_slabs: u32,
    /// Scalars, not a `vec3`: a vector here would align to 16 and shift itself
    /// off the offset the Rust struct writes.
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
};

@group(0) @binding(0) var<uniform> locals: Locals;
/// The grid, packed four stored bytes to a word. Slot `s` bucket `b` is byte
/// `s * stride + b`.
@group(0) @binding(1) var<storage, read> grid: array<u32>;
/// The gradient sampled at `textureDimensions(lut).x` equal level slices,
/// opaque and in gamma space — the bytes `Color32` carries.
@group(0) @binding(2) var lut: texture_2d<f32>;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    /// Position along the run in SLABS from the first visible slab's left
    /// edge: `n - 0.5` is the newest slab's centre.
    @location(0) slab: f32,
    /// Pitch fraction across the visible range, 0 at `min_midi`.
    @location(1) t: f32,
};

@vertex
fn vs_heatmap(
    @location(0) pos: vec2<f32>,
    @location(1) slab: f32,
    @location(2) t: f32,
) -> VertexOut {
    let in_viewport = pos - locals.origin_points;
    var out: VertexOut;
    out.position = vec4<f32>(
        2.0 * in_viewport.x / locals.viewport_points.x - 1.0,
        1.0 - 2.0 * in_viewport.y / locals.viewport_points.y,
        0.0,
        1.0,
    );
    out.slab = slab;
    out.t = t;
    return out;
}

/// One stored byte out of the grid. The buffer is words, so a byte costs a
/// shift and a mask; a slab is padded to `stride` bytes and the padding is
/// never addressed.
fn stored(slot: u32, bucket: u32) -> u32 {
    let i = slot * locals.stride + bucket;
    return (grid[i >> 2u] >> ((i & 3u) * 8u)) & 0xffu;
}

/// Where a pitch fraction sits on the bucket axis: bucket `b` spans
/// `[b, b + 1)`, so this is continuous and the floor of it is a bucket index.
fn bucket_x(t: f32) -> f32 {
    let midi = locals.min_midi + t * locals.span;
    return (midi - locals.spectrum_min_midi) * locals.bins_per_semitone;
}

/// The IMAGE this fragment resamples: one virtual row per bucket, each holding
/// the level that bucket alone would be drawn at — the ramp's 0..1, tilted at
/// the bucket's own pitch and clamped there.
///
/// Clamped per bucket and not after the combine, which is what makes the
/// picture an image of the spectrum rather than of a mean of it: a partial
/// standing above the window's ceiling contributes a full-bright bucket to
/// whatever covers it, and a floor below the window contributes black, so a
/// feature narrower than a pixel dims in proportion to its share of that pixel
/// instead of being dragged off the ramp by its neighbours.
// Artistic brightness weighting of display intensity, not audio power or
// an RGB gamma transfer. The linear toe keeps quiet values representable in
// R16Float. Apply before either source footprint can average away a ridge.
fn density_encode(level: f32) -> f32 {
    return level * (0.1 + 0.9 * level);
}
fn density_decode(value: f32) -> f32 {
    let y = max(value, 0.0);
    return 2.0 * y / (0.1 + sqrt(0.01 + 3.6 * y));
}
fn bucket_level(slot: u32, b: u32, density: bool) -> f32 {
    let midi = locals.spectrum_min_midi + (f32(b) + 0.5) / locals.bins_per_semitone;
    let v = f32(stored(slot, b));
    let level = locals.level0 + locals.level_per_step * v + locals.level_per_midi * midi;
    let mapped = clamp(level, 0.0, 1.0);
    if density { return density_encode(mapped); }
    return mapped;
}

/// The level one fragment reads out of the slab in slot `slot`: an image
/// resample of [`bucket_level`] over the pitch this fragment covers.
///
/// The footprint is the fragment's own — exactly one pane pixel of the pitch
/// axis, so the footprints TILE it — and which of it and the bucket grid is
/// finer picks the arm.
///
/// For Plain, MINIFYING (a pixel wider than a bucket) is the AREA-WEIGHTED MEAN of the
/// levels under `[x0, x1)`: fractional weights where the footprint cuts its
/// first and last bucket, unit weights between. That is what a GPU does to a
/// texture it draws small, and it is the whole of why the pane's pixel height
/// no longer decides the picture's brightness: the operator is LINEAR in the
/// quantity the ramp is indexed by, footprints tile the axis, and every
/// footprint covers the same number of buckets — so the pane-integrated level
/// is the average over the buckets on screen at any pixel height. A power mean
/// over the same run is not linear and no order of one is: a feature narrower
/// than a pixel is attenuated as the pixel widens while its share of the pane
/// grows, and the two do not cancel.
///
/// The density source uses the same footprint in the encoded display domain.
///
/// MAGNIFYING (a pixel narrower than a bucket) the grid is being asked for
/// more than it holds, so it is read BETWEEN the two bucket centres this
/// fragment sits between. A bucket's centre is half a bucket above where the
/// floor divides them, which is the 0.5; the clamp keeps the upper tap inside
/// the spectrum.
fn read_level(slot: u32, t: f32, density: bool) -> f32 {
    let half = 0.5 / f32(locals.rows);
    let x0 = bucket_x(t - half);
    let x1 = bucket_x(t + half);
    let top = f32(locals.bins) - 1.0;
    let idx = u32(clamp(floor(x0), 0.0, top));
    let last = u32(clamp(floor(x1), 0.0, top));
    if last > idx {
        let lo = clamp(x0, 0.0, f32(locals.bins));
        let hi = clamp(x1, 0.0, f32(locals.bins));
        var sum = 0.0;
        var total = 0.0;
        for (var b = idx; b <= last; b = b + 1u) {
            let w = max(min(hi, f32(b) + 1.0) - max(lo, f32(b)), 0.0);
            sum = sum + w * bucket_level(slot, b, density);
            total = total + w;
        }
        // A run of two or more whose overlap has been clamped to nothing — the
        // degenerate answered rather than one the picture arrives at.
        if total <= 0.0 {
            return bucket_level(slot, idx, density);
        }
        return sum / total;
    }
    let x = bucket_x(t) - 0.5;
    let b = u32(clamp(floor(x), 0.0, f32(locals.bins) - 2.0));
    let f = clamp(x - f32(b), 0.0, 1.0);
    return mix(bucket_level(slot, b, density), bucket_level(slot, b + 1u, density), f);
}

/// The heatmap's colour at this fragment: the two slabs either side of it read
/// at this fragment's own footprint, blended, and looked up once.
///
/// The blend is in LEVEL space, the space the pitch resample already works in,
/// so one operator spans both axes and the gradient is applied to the answer
/// rather than to each tap. Against blending the two COLOURS it differs only
/// by the ramp's own curvature across one slab boundary, which is the whole of
/// what reading time this way costs.
///
/// The clamp on the slab axis is a sampler's `ClampToEdge`: past the newest
/// slab the picture holds its edge rather than reading a slot the run does not
/// own. The pitch axis needs none — a footprint that runs off the spectrum is
/// clamped bucket by bucket inside [`read_level`].
///
/// The lookup TRUNCATES into the table, matching the level's own quantization
/// — the table is sampled at the centre of each slice, so the entry a level
/// falls into is the one nearest it.
fn field_level(in: VertexOut, density: bool) -> f32 {
    // Slab centres sit at half-integers, so the taps straddle `slab - 0.5`.
    let n = f32(locals.run_slabs);
    let jx = clamp(floor(in.slab - 0.5), 0.0, n - 1.0);
    let j0 = u32(jx);
    let j1 = min(j0 + 1u, locals.run_slabs - 1u);
    let fx = clamp(in.slab - 0.5 - jx, 0.0, 1.0);
    // Run index to slot. The scatter that filled the buffer walks the same
    // rule from the same `first_slot` (`slot_of` in spectrogram.rs), and
    // `slab_keys_before_zero_and_a_wrapping_run_land_where_the_shader_reads`
    // is what holds the two together.
    let s0 = (locals.first_slot + j0) % locals.capacity;
    let s1 = (locals.first_slot + j1) % locals.capacity;

    return mix(read_level(s0, in.t, density), read_level(s1, in.t, density), fx);
}

// Literal domain arguments specialize the shared resampler for each entry
// point; Plain keeps its original display-level area mean exactly.
fn heatmap_level(in: VertexOut) -> f32 {
    return field_level(in, false);
}

fn heatmap_color(in: VertexOut) -> vec4<f32> {
    let level = heatmap_level(in);
    let levels = textureDimensions(lut).x;
    let i = min(u32(level * f32(levels)), levels - 1u);
    let c = textureLoad(lut, vec2<u32>(i, 0u), 0);
    // Opaque: silence is the ramp's dark end, so the plane is filled rather
    // than see-through, and every entry of the table is opaque already.
    return vec4<f32>(c.rgb, 1.0);
}

// 0-1 linear from 0-1 sRGB gamma. Lifted from egui's own shader, and used for
// the same reason: on an sRGB-aware target egui hands the hardware linear
// values and lets it encode.
fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_heatmap_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return heatmap_color(in);
}

@fragment
fn fs_heatmap_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = heatmap_color(in);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), gamma.a);
}

struct Cloud {
    origin: vec2<f32>,
    size: vec2<f32>,
    step: vec2<f32>,
    ppp: f32,
    spread: f32,
    contours: f32,
    contour_softness: f32,
    style: u32,
    _pad: u32,
    // Scale clouds. `drift` is the cloud material's offset in cloud units and
    // `time` a bounded clock for the scales' slow turning; the rest are the
    // sanitized settings. The filter shader declares only the head of this
    // struct, which is why these are appended rather than interleaved.
    drift: vec2<f32>,
    time: f32,
    cloud_depth: f32,
    cloud_scale: f32,
    cloud_cover: f32,
    scale_size: f32,
    scale_overlap: f32,
    scale_glint: f32,
    cloud_ambient: f32,
    _pad2: f32,
    _pad3: f32,
};
@group(1) @binding(0) var close_light: texture_2d<f32>;
@group(1) @binding(1) var wide_light: texture_2d<f32>;
@group(1) @binding(2) var cloud_sampler: sampler;
@group(1) @binding(3) var<uniform> cloud: Cloud;

// Scalar display intensity has no gamma transfer function. In particular,
// the float source target must not take fs_heatmap_linear's RGB conversion.
@fragment
fn fs_density_source(in: VertexOut) -> @location(0) vec4<f32> {
    // Integrate the piecewise-linear encoded field exactly between slab centers.
    // A large musical blur reduces the source width, so four fixed samples
    // would skip columns and alias periodic broadband energy before filtering.
    let width = fwidth(in.slab);
    if width < 0.0001 { return vec4<f32>(field_level(in, true), 0.0, 0.0, 1.0); }
    let high = in.slab + width * 0.5;
    var at = in.slab - width * 0.5;
    let covered = high - at;
    if covered <= 0.0 { return vec4<f32>(field_level(in, true), 0.0, 0.0, 1.0); }
    var tap = in;
    tap.slab = at;
    var left = field_level(tap, true);
    var integral = 0.0;
    let last_center = f32(locals.run_slabs) - 0.5;
    // Outside the run the read holds its edge; skip that constant interval in
    // one step. Inside, each original slab contributes to this footprint.
    for (var i = 0u; i < locals.run_slabs + 2u; i += 1u) {
        if at >= high { break; }
        var next = min(high, max(0.5, floor(at - 0.5) + 1.5));
        if at >= last_center { next = high; }
        tap.slab = next;
        let right = field_level(tap, true);
        integral += (left + right) * 0.5 * (next - at);
        left = right;
        at = next;
    }
    return vec4<f32>(integral / covered, 0.0, 0.0, 1.0);
}
@fragment
fn fs_cloud_light(in: VertexOut) -> @location(0) vec4<f32> {
    // Combine the two smoothing scales in the scalar image so the final
    // full-resolution pass needs only one filtered read per pixel.
    let uv = in.position.xy / vec2<f32>(textureDimensions(wide_light));
    let close = textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
    let wide = textureSampleLevel(wide_light, cloud_sampler, uv, 0.0).r;
    // Both scales stay in the encoded domain until their final combination.
    // Decode here once per reduced pixel; the composite linearly upsamples
    // this display-intensity field without another full-resolution sqrt.
    return vec4<f32>(density_decode(mix(close, wide, cloud.spread)), 0.0, 0.0, 1.0);
}
fn baked_density(position: vec2<f32>) -> f32 {
    let uv = (position / cloud.ppp - cloud.origin) / cloud.size;
    // The source texture is reused for the finished scalar material only
    // after both filters have consumed it. No attachment samples itself.
    return textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
}
fn smoothed_level(core: f32, material: f32) -> f32 {
    if all(cloud.step == vec2<f32>(0.0)) { return core; }
    return material;
}
// Local style transfer. No history, upload, smoothing or palette work is
// duplicated when adding a display style here. A residual slope preserves
// quiet fields below the first terrace; the zero input remains exactly zero.
fn style_level(level: f32) -> f32 {
    if cloud.style != 2u { return level; }
    let x = clamp(level, 0.0, 1.0) * cloud.contours;
    let edge = min(0.5, max(cloud.contour_softness, fwidth(x) * 0.5));
    let terraces = (floor(x) + smoothstep(0.5 - edge, 0.5 + edge, fract(x))) / cloud.contours;
    let strength = 0.9 * smoothstep(0.0, 1.0, x)
        * (1.0 - smoothstep(0.5, 1.5, fwidth(x)));
    return mix(level, terraces, strength);
}
// Interpolate the authored palette's center samples only after diffusion.
// The first half-slice joins true black smoothly, even for an edited ramp
// whose first sample is nonblack; there is no separate halo color curve.
fn palette_color(level: f32) -> vec3<f32> {
    let levels = textureDimensions(lut).x;
    let x = clamp(level, 0.0, 1.0) * f32(levels) - 0.5;
    let i = u32(clamp(floor(x), 0.0, f32(levels - 1u)));
    let a = textureLoad(lut, vec2<u32>(i, 0u), 0).rgb;
    if x < 0.0 {
        return a * (x + 0.5) * 2.0;
    }
    let b = textureLoad(lut, vec2<u32>(min(i + 1u, levels - 1u), 0u), 0).rgb;
    return mix(a, b, fract(x));
}
fn density_color(raw_level: f32) -> vec4<f32> {
    return vec4<f32>(palette_color(style_level(raw_level)), 1.0);
}

// Scale clouds (prototype).
//
// A drifting cloud layer over the finished picture, lit by the wide blur of
// the spectrogram under it. The layer has no shape of its own apart from its
// scales: it is puffs on jittered grids at three sizes, and a cloud is where
// enough of them pile up. There is no noise field anywhere, so one grain
// covers the whole pane and there is no separate cloud texture for the scales
// to read inconsistently against.
//
// Three things here are what separate a pile of puffs from a slab of gel, and
// each of them was measured as the difference rather than guessed at.
//
// **Every scale leaves gaps, and every scale reaches the silhouette.** One
// puff per cell on a jittered grid is a blue-noise point set — the most
// UNIFORM arrangement there is — so with each puff reaching past its own cell
// the layer covers the pane by construction and can never open a gap wider
// than a cell. A layer with no gaps has no outline, and a cloud is mostly
// outline. Cells are therefore empty at a rate `cloud_cover` sets. Leaving
// only the coarsest scale sparse is not enough either: its outline is then an
// arc of one circle, and a pile of equal circles reads as grapes. A cloud edge
// is bumps on bumps, so the finer octaves are in the silhouette too, at a
// weight that falls as they get finer.
//
// **The relief is a soft UNION of the puffs, not their SUM.** Adding two
// overlapping puffs makes one taller smooth mound — their slopes cancel
// exactly where the near one's rim should be — so a summed pile has no lobes
// however many puffs are in it. `log(sum exp(k*h))/k` keeps both tops and
// creases between them, and it is order-independent, so it costs one `exp` a
// puff and no sorting. The same weights average the puffs' own sphere normals,
// which is what gives each lobe its own terminator.
//
// **The light has to arrive from somewhere other than straight behind.** This
// is the one that made the old cut read as gel and it is not a tuning value: a
// backlit uniform slab has view path and light path of the SAME length at
// every point, so its brightness is a function of thickness alone, which is
// exactly what a sheet of jelly looks like. Marching the cloud-scale thickness
// a couple of steps toward the light gives the two paths different lengths,
// and the falloff across a mass is what reads as a body with a lit side.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI: five
// cloud units across the pane's height at size 1, like the lattice nebula.
// Scrolling never moves the clouds; they are in front of the picture.

// Enough bits for one puff, sliced into ten-bit fractions.
fn cloud_bits(cell: vec2<i32>, salt: u32) -> u32 {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u ^ bitcast<u32>(cell.y)) + salt;
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    return n ^ (n >> 16u);
}
fn cloud_slice(bits: u32, shift: u32) -> f32 {
    return f32((bits >> shift) & 1023u) / 1023.0;
}
// A puff's slow wander, -1..1 and smooth at its turns. `rate` picks a whole
// number of crossings per wrap of the pane clock, so the wrap is invisible.
fn puff_wave(phase: f32, rate: f32) -> f32 {
    let turns = 5.0 + floor(rate * 36.0);
    let t = fract(cloud.time * turns * 0.001 + phase);
    let ramp = abs(t * 2.0 - 1.0);
    return ramp * ramp * (3.0 - 2.0 * ramp) * 2.0 - 1.0;
}

// How far a puff's centre strays from its cell's, as a fraction of a cell.
// Together with the radius ceiling this keeps every puff that can carry weight
// to a point inside the 3x3 ring below. A puff that reaches past the ring pops
// in and out at a cell edge, which draws straight blocky patches across the
// clouds — the failure that a wider radius buys.
const PUFF_JITTER: f32 = 0.7;
const PUFF_RADIUS_MAX: f32 = 1.1;
const PUFF_OCTAVES: i32 = 3;
const PUFF_LIFT: f32 = 0.35;
const PUFF_UNION: f32 = 8.0;
// The second and third hashes of a puff: its lift, occupancy and wander.
const PUFF_SALT_B: u32 = 0xc2b2ae35u;
const PUFF_SALT_C: u32 = 0x27d4eb2fu;

// How many of an octave's cells hold a puff at all. The finer scales are
// denser, because a lone fine puff in clear sky is a speck while a lone coarse
// one is a small cloud.
fn cloud_occupancy(octave: i32) -> f32 {
    return clamp(0.12 + 0.62 * cloud.cloud_cover + 0.10 * f32(octave), 0.0, 1.0);
}
// What one scale puts into the silhouette. The coarsest carries the shapes;
// the finer ones are the fringe on their edges, which is what a cloud has and
// a circle does not.
fn cloud_octave_weight(octave: i32) -> f32 {
    return pow(0.5, f32(octave));
}

struct Puff {
    // Whether this cell holds a puff that covers the point at all.
    hit: bool,
    // The point's place in the puff: `cap` is the sphere's own height over it,
    // 1 at the centre and 0 at the rim, and `offset` is where it sits across
    // the disc, which with `cap` is the sphere's normal.
    cap: f32,
    offset: vec2<f32>,
    radius: f32,
    lift: f32,
};

// One cell's puff against a point, both in this octave's grid units.
fn cloud_puff(cell: vec2<i32>, at: vec2<f32>, salt: u32, occupancy: f32,
              radius_lo: f32, radius_hi: f32, wander: bool) -> Puff {
    var out = Puff(false, 0.0, vec2<f32>(0.0), 1.0, 0.0);
    let a = cloud_bits(cell, salt);
    let b = cloud_bits(cell, salt ^ PUFF_SALT_B);
    // An empty cell is sky. This is the only thing that opens one.
    if cloud_slice(b, 20u) >= occupancy {
        return out;
    }
    var stray = vec2<f32>(cloud_slice(a, 0u), cloud_slice(a, 10u)) - 0.5;
    if wander {
        // The wander is what makes the pile change shape rather than only
        // slide: puffs move inside their cells, so overlaps open and close.
        // Only the coarsest scale wanders, since it is the one whose overlaps
        // are a shape, and it is a third hash on nine puffs rather than on all
        // twenty-seven.
        let c = cloud_bits(cell, salt ^ PUFF_SALT_C);
        let rate = cloud_slice(c, 20u);
        stray += 0.18 * vec2<f32>(
            puff_wave(cloud_slice(c, 0u), rate),
            puff_wave(cloud_slice(c, 10u), rate),
        );
    }
    let centre = vec2<f32>(cell) + 0.5 + stray * PUFF_JITTER;
    let x = at - centre;
    let radius = mix(radius_lo, radius_hi, cloud_slice(a, 20u));
    let u = 1.0 - dot(x, x) / (radius * radius);
    if u <= 0.0 {
        return out;
    }
    out.hit = true;
    out.cap = sqrt(u);
    out.offset = x / radius;
    out.radius = radius;
    out.lift = cloud_slice(b, 0u);
    return out;
}

// The cloud-scale octave's thickness on its own, which is what the light march
// walks. The mass is all at this scale, and marching the finer ones as well
// would double the shader again for a gradient they do not change.
fn cloud_mass(p: vec2<f32>, radius_lo: f32, radius_hi: f32) -> f32 {
    let home = vec2<i32>(floor(p));
    let occupancy = cloud_occupancy(0);
    var total = 0.0;
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let puff = cloud_puff(home + vec2<i32>(i, j), p, 0u, occupancy,
                                  radius_lo, radius_hi, true);
            if puff.hit {
                total += puff.cap * puff.cap * puff.cap;
            }
        }
    }
    return total;
}

struct Pile {
    // Optical thickness through the whole pile, every scale in it: what
    // decides how much of the picture the layer hides and how far light gets
    // through it.
    density: f32,
    // The soft union's surface normal. Not the pile's summed slope, which is
    // the surface of a mound rather than of a heap of lobes.
    normal: vec3<f32>,
};

fn cloud_pile(p: vec2<f32>, lacunarity: f32, radius_lo: f32, radius_hi: f32) -> Pile {
    var union_acc = 0.0;
    var normal_acc = vec3<f32>(0.0);
    var density = 0.0;
    var freq = 1.0;
    for (var octave = 0; octave < PUFF_OCTAVES; octave += 1) {
        // Each octave's grid is displaced as well as finer, so two of them
        // never share a corner however their sizes land.
        let shift = vec2<f32>(f32(octave) * 31.7, f32(octave) * -17.3);
        let at = p * freq + shift;
        let home = vec2<i32>(floor(at));
        let salt = u32(octave) * 0x9e3779b9u;
        let occupancy = cloud_occupancy(octave);
        let weight = cloud_octave_weight(octave);
        for (var j = -1; j <= 1; j += 1) {
            for (var i = -1; i <= 1; i += 1) {
                let puff = cloud_puff(home + vec2<i32>(i, j), at, salt, occupancy,
                                      radius_lo, radius_hi, octave == 0);
                if !puff.hit {
                    continue;
                }
                density += puff.cap * puff.cap * puff.cap * weight;
                // The union, in cloud units: a puff's own height plus where it
                // rests, so the puffs stack into a pile instead of all sitting
                // on one plane.
                let height = (puff.radius * puff.cap + PUFF_LIFT * puff.lift) / freq;
                let share = exp(PUFF_UNION * height) - 1.0;
                union_acc += share;
                // The puff's own sphere normal, weighted by how much of the
                // union it wins here. Deep inside one lobe its neighbours
                // weigh nothing and the normal is that sphere's alone, which
                // is the terminator that makes a lobe read as round.
                normal_acc += share * vec3<f32>(puff.offset * freq,
                                                max(puff.cap, 0.05));
            }
        }
        freq *= lacunarity;
    }
    var out: Pile;
    out.density = density;
    out.normal = normalize(normal_acc + vec3<f32>(0.0, 0.0, 0.0001));
    return out;
}

// The light under a pane point, display intensity 0..1: mostly the wide blur,
// which is what reaches a cloud beside a ridge, lifted so a lit cloud glows
// brighter than the halo it sits in.
//
// Mostly, and not floored by the finished material as it was, because the
// material is SHARP: taking it at full weight printed the spectrogram's own
// band edges inside the cloud body, and a cloud showing a crisp edge that
// belongs to what is behind it is a film over the picture rather than
// something in front of it. What that cost is a cloud over a narrow ridge,
// which the wide blur spreads too thin to light, so a quarter of the material
// is added back — enough to brighten the cloud there, too little to draw with.
fn cloud_light(pt: vec2<f32>) -> f32 {
    let uv = pt / cloud.size;
    let wide = density_decode(textureSampleLevel(wide_light, cloud_sampler, uv, 0.0).r);
    let material = textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
    return 1.3 * wide + 0.25 * material;
}

// Compress rather than clip. At a deep cover the shading runs past what the
// palette can hold, and a channel that clips is a flat patch with a hard edge
// on it. Compressing the MAGNITUDE and keeping the direction keeps the hue: a
// clipped channel shifts the colour as well as flattening it, which is what
// makes the patch look like metal rather than like a bright cloud.
fn softened(colour: vec3<f32>) -> vec3<f32> {
    let m = max(max(colour.r, colour.g), colour.b);
    if m <= 0.75 {
        return colour;
    }
    return colour * ((0.75 + 0.25 * (1.0 - exp((0.75 - m) * 4.0))) / m);
}

struct Backlight {
    // The light behind the layer here, gathered over a disc a cloud wide
    // rather than read under the point. That width is the whole difference
    // between a lit cloud and a tinted one: a bright band lights the
    // neighbourhood of cloud around it, several clouds at once, and the
    // falloff away from it is what says where the light is.
    glow: f32,
    // Which way, across the pane, the light is coming from, and how much of a
    // direction there is to have.
    toward: vec2<f32>,
    aimed: f32,
};

// Six taps over a DISC and not a ring, on the golden-angle spiral that spaces
// them evenly at every radius. A ring gathers from one distance, so a band
// just inside or outside it is missed entirely and the ring's own shape prints
// into a smooth field; a disc of the same six taps is a wide blur, which is
// what a glow through cloud is.
fn backlight(pt: vec2<f32>, reach: f32) -> Backlight {
    let d0 = vec2<f32>(0.2887, 0.0000);
    let d1 = vec2<f32>(-0.3687, 0.3377);
    let d2 = vec2<f32>(0.0564, -0.6430);
    let d3 = vec2<f32>(0.4647, 0.6061);
    let d4 = vec2<f32>(-0.8528, -0.1508);
    let d5 = vec2<f32>(0.8078, -0.5139);
    let v0 = cloud_light(pt + d0 * reach);
    let v1 = cloud_light(pt + d1 * reach);
    let v2 = cloud_light(pt + d2 * reach);
    let v3 = cloud_light(pt + d3 * reach);
    let v4 = cloud_light(pt + d4 * reach);
    let v5 = cloud_light(pt + d5 * reach);
    let ring = v0 + v1 + v2 + v3 + v4 + v5;
    let here = cloud_light(pt);
    var out: Backlight;
    // The point counts for a fifth, so a cloud over a bright place is still
    // brighter than its neighbours while the disc carries where the light is.
    out.glow = here * 0.2 + ring * 0.1333;
    // The taps pull toward the brighter side of the disc, which is a direction
    // over a whole neighbourhood rather than a difference between two pixels.
    let pull = normalize(d0) * v0 + normalize(d1) * v1 + normalize(d2) * v2
        + normalize(d3) * v3 + normalize(d4) * v4 + normalize(d5) * v5;
    let lean = length(pull) / max(ring, 0.0001);
    // Held back hard, so the sound LEANS the light rather than aiming it. A
    // spectrogram of bands has a brighter side that reverses between every
    // pair of them; a light that follows it turns over as often, and the
    // shadow march then walks a direction that flips band to band and prints
    // the bands back into the clouds as horizontal streaks. The march is why
    // this is a quarter and not the two thirds it was: a shading term reads a
    // turned-over light as flat, but a march draws it.
    let followed = 0.25 * smoothstep(0.02, 0.25, lean);
    out.toward =
        normalize(mix(vec2<f32>(0.55, -0.83), pull / max(length(pull), 0.00001), followed));
    out.aimed = 0.45 + 0.55 * smoothstep(0.0, 0.2, lean);
    return out;
}

fn scale_clouds(base: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    // No blur means no light field to light the clouds from.
    if cloud.cloud_depth <= 0.0 || all(cloud.step == vec2<f32>(0.0)) {
        return base;
    }
    let pt = position / cloud.ppp - cloud.origin;
    let units = 5.0 / cloud.cloud_scale;
    let q = (pt - cloud.size * 0.5) / cloud.size.y * units + cloud.drift;
    // The octaves run geometrically from the cloud's own size down to the
    // scale size, so that knob sets how small the smallest puffs get rather
    // than laying a second grid over the first.
    let lacunarity = max(1.2, sqrt(4.0 / cloud.scale_size));
    // A wide spread of radii inside one octave, because equal circles read as
    // equal circles however they are piled.
    let radius_hi = min(cloud.scale_overlap, PUFF_RADIUS_MAX);
    let radius_lo = radius_hi * 0.48;
    let pile = cloud_pile(q, lacunarity, radius_lo, radius_hi);

    // Opacity is absorption through the pile rather than a threshold on it. A
    // threshold is an EDGE however wide its ramp — it has a place where the
    // cloud starts — and an edge is the one thing a fuzzy shape does not have.
    // This has none: every puff fades in from nothing over its own width.
    let alpha = 1.0 - exp(-2.4 * pow(max(pile.density, 0.0), 1.15));
    if alpha <= 0.002 {
        return base;
    }

    // The gather is a fifth of the pane, and at least two clouds wide: "lights
    // up several clouds at once" means it has to be wider than one of them,
    // and at a small cloud size a gather scaled to the cloud is narrower than
    // the bands it is meant to be spreading.
    let cloud_points = cloud.size.y / units;
    let sky = backlight(pt, max(cloud_points * 2.0, cloud.size.y * 0.2));

    // March the cloud-scale thickness toward the light. Two steps, the second
    // nearly twice as far, so the pair reaches across a whole mass rather than
    // sampling one rim of it — a single probe inside a mass wider than its own
    // step is a constant, and a constant is no gradient at all.
    var walked = 0.0;
    var travelled = 0.0;
    for (var step = 0; step < 2; step += 1) {
        travelled += 0.95 * (1.0 + f32(step) * 0.8);
        walked += cloud_mass(q + sky.toward * travelled, radius_lo, radius_hi)
            / (1.0 + f32(step));
    }
    let shadow = exp(-1.35 * walked);

    // The light stands behind the layer and off to the side it leans toward.
    // Straight behind is the degenerate case this whole pass exists to avoid.
    let sun = normalize(vec3<f32>(sky.toward * 0.97, -0.25));
    // Wrapped rather than clamped at the terminator: light crosses a cloud
    // instead of stopping at its surface, and a hard clamp at zero draws the
    // steep side of a lobe as a black edge with a visible boundary.
    let key = pow(max(dot(pile.normal, sun) * 0.5 + 0.5, 0.0), 2.0);
    // Forward scatter: the light is behind, so a wisp passes nearly all of it
    // and a deep body almost none. This is what makes the thin places and the
    // edges the bright ones.
    let through = exp(-0.9 * pile.density);
    // Scaled from zero, not from a floor: 0 has to mean a flat backlit sheet
    // with no side light at all, both because that is the knob's whole range
    // and because the seam test turns the relief off through it — a lit lobe
    // is a legitimate fast edge, and it would bury the edge that test looks
    // for.
    let sculpt = 2.6 * cloud.scale_glint;
    let shading = (cloud.cloud_ambient
        + key * shadow * sculpt
        + 0.95 * through * shadow * sky.aimed) * 1.3;
    // The sound picks the colour and the cloud's shape scales it, rather than
    // the shape being folded into the palette's level where it would run off
    // the top of the ramp. Under a root, so the gaps between bands come up
    // toward the bands and the sound stays a tint.
    let tint = pow(clamp(sky.glow, 0.0, 1.0), 0.7);
    let level = clamp(tint * 0.85 + 0.15, 0.0, 1.0);
    let body = palette_color(level) * shading;
    return mix(base, softened(body), cloud.cloud_depth * alpha);
}
fn clouded(level: f32, position: vec2<f32>) -> vec4<f32> {
    let base = density_color(level);
    return vec4<f32>(scale_clouds(base.rgb, position), 1.0);
}
// Empty history uses the same field and palette with a zero measured core.
// This quad never samples the grid, so the oldest column cannot be smeared.
@fragment
fn fs_cloud_backdrop_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return clouded(smoothed_level(0.0, baked_density(in.position.xy)), in.position.xy);
}
@fragment
fn fs_cloud_backdrop_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = clouded(smoothed_level(0.0, baked_density(in.position.xy)), in.position.xy);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), 1.0);
}
fn cloud_color(in: VertexOut) -> vec4<f32> {
    return clouded(smoothed_level(heatmap_level(in), baked_density(in.position.xy)), in.position.xy);
}
@fragment
fn fs_cloud_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return cloud_color(in);
}
@fragment
fn fs_cloud_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let color = cloud_color(in);
    return vec4<f32>(linear_from_gamma_rgb(color.rgb), color.a);
}
