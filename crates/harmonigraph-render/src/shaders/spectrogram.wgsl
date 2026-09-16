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
// scales: it is a field of soft round puffs on jittered grids at three sizes,
// each puff wide enough to overlap its neighbours, and a cloud is nothing but
// where enough of them pile up. So one grain covers the whole pane — a thin
// veil and a thick body are the same material at the same size — and there is
// no separate cloud texture for the scales to read inconsistently against.
//
// The pile is also a height field, and its slope is what the light reads: a
// scale is lit on the side facing the light and glints where its slope
// bisects the light and the eye, and it shows the light under the puffs
// covering it rather than under itself, weighted toward whichever covers it
// most, so the light is softly quantized to the puffs. That shifts
// as the clouds drift across the light, as the light scrolls under them, and
// as each puff wanders inside its own cell.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI: five
// cloud units across the pane's height at size 1, like the lattice nebula.
// Scrolling never moves the clouds; they are in front of the picture.

// Enough bits for one puff, sliced into ten-bit fractions. Two of these cover
// a puff's place, weight and wander, which is a third of what a scalar hash
// per value would cost at twenty-seven puffs a pixel.
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

// Diffuse falloff over a body light passes into rather than off: 1 head on,
// and still lit a little where the surface turns away.
fn wrapped_light(cosine: f32) -> f32 {
    return pow(max(cosine * 0.5 + 0.5, 0.0), 1.5);
}

// How far a puff's centre strays from its cell's, as a fraction of a cell.
// Together with the overlap's own ceiling this keeps every puff that can carry
// weight to a point inside the 3x3 ring below; a wider one would pop puffs in
// and out at the ring's edge.
const PUFF_JITTER: f32 = 0.6;
const PUFF_OCTAVES: i32 = 3;

struct Puffs {
    // What every octave's puffs pile up over this point, and the slope of that
    // pile. The slope points into the pile, so the surface's outward tilt is
    // its negation.
    depth: f32,
    slope: vec2<f32>,
    // The largest octave alone, which shades the cloud's body rather than its
    // grain.
    body: f32,
    body_slope: vec2<f32>,
    // Where this point reads its light: the smallest octave's puff centres,
    // averaged by how far into each of them the point lies, and how much puff
    // there was to average. Deep inside one puff its neighbours weigh nothing
    // and the average is its centre alone, so a whole puff shows a single
    // reading of the sound; between two the average slides from one centre to
    // the other. Picking the nearest instead would be the same quantization
    // with a Voronoi seam drawn through it, and that seam is visible: it is a
    // straight edge across a cloud, which is the mosaic this is not.
    lit_centre: vec2<f32>,
    lit_weight: f32,
};

fn puff_field(p: vec2<f32>, lacunarity: f32, radius: f32) -> Puffs {
    var out = Puffs(0.0, vec2<f32>(0.0), 0.0, vec2<f32>(0.0), p, 0.0);
    let inv_r2 = 1.0 / (radius * radius);
    var freq = 1.0;
    var amp = 1.0;
    var broad = 1.0;
    var lit_acc = vec2<f32>(0.0);
    for (var octave = 0; octave < PUFF_OCTAVES; octave += 1) {
        // Each octave's grid is displaced as well as finer, so two of them
        // never share a corner however their sizes land.
        let shift = vec2<f32>(f32(octave) * 31.7, f32(octave) * -17.3);
        let r = p * freq + shift;
        let home = vec2<i32>(floor(r));
        let salt = u32(octave) * 0x9e3779b9u;
        for (var j = -1; j <= 1; j += 1) {
            for (var i = -1; i <= 1; i += 1) {
                let c = home + vec2<i32>(i, j);
                let a = cloud_bits(c, salt);
                let b = cloud_bits(c, salt ^ 0xc2b2ae35u);
                let place = vec2<f32>(cloud_slice(a, 0u), cloud_slice(a, 10u)) - 0.5;
                // The wander is what makes the pile change shape rather than
                // only slide: puffs move inside their cells, so overlaps open
                // and close. One rate shares a slice with the weight, which
                // nothing on screen can correlate.
                let wander = vec2<f32>(
                    puff_wave(cloud_slice(b, 0u), cloud_slice(b, 20u)),
                    puff_wave(cloud_slice(b, 10u), cloud_slice(a, 20u)),
                );
                let centre = vec2<f32>(c) + 0.5 + (place + wander * 0.18) * PUFF_JITTER;
                let x = r - centre;
                let u = 1.0 - dot(x, x) * inv_r2;
                if u <= 0.0 {
                    continue;
                }
                // Puffs differ in weight, biased low, and that difference is
                // the only thing that makes a cloud: where heavy ones land
                // close together the pile is a body, and elsewhere a veil.
                let pick = cloud_slice(a, 20u);
                let weight = pick * pick * amp;
                // Cubed and not squared: the softer a scale's own rim is, the
                // less the pile has anything an eye can call an edge, and the
                // more its weight sits in the middle where the lumps are.
                let lump = u * u * u * weight;
                // The same puff differentiated, which is the relief's slope.
                let lump_slope = (-6.0 * u * u * weight * freq * inv_r2) * x;
                out.depth += lump;
                // Weighted down as the scales get smaller, which the true
                // slope of the pile is not. The smallest scales belong in the
                // pile's DEPTH, where they are soft; carried into its shading
                // at full strength they are a crisp ripple across the surface,
                // and a crisp ripple lit from the side is water.
                out.slope += lump_slope * broad;
                if octave == 0 {
                    out.body += lump;
                    out.body_slope += lump_slope;
                }
                if octave == PUFF_OCTAVES - 1 {
                    // Weighted by the puff's shape alone and not by its
                    // weight, so the grain the light is read on is the same
                    // everywhere rather than following the heavy puffs. The
                    // steep power is what keeps the reading a plateau per
                    // puff: a plain average slides continuously from centre to
                    // centre and the light stops being quantized at all, which
                    // reads as rippled glass rather than as scales.
                    let u2 = u * u;
                    let reach = u2 * u2;
                    lit_acc += reach * (centre - shift) / freq;
                    out.lit_weight += reach;
                }
            }
        }
        freq *= lacunarity;
        amp *= 0.5;
        broad *= 0.5;
    }
    if out.lit_weight > 0.0 {
        out.lit_centre = lit_acc / out.lit_weight;
    }
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
    let field = puff_field(q, lacunarity, cloud.scale_overlap);

    // Opacity is absorption through the pile rather than a threshold on it. A
    // threshold is an EDGE however wide its ramp — it has a place where the
    // cloud starts — and an edge is the one thing a fuzzy shape does not have.
    // This has none: every scale fades in from nothing over its own width, and
    // cover is only how much of the light one scale's depth takes.
    // Against the SQUARE of the depth, so a pile twice as deep is much more
    // than twice as opaque: absorption straight off the depth is so nearly
    // uniform over the pane that the clouds stop having shapes and the whole
    // thing is one haze.
    let take = mix(0.5, 6.5, cloud.cloud_cover);
    let alpha = 1.0 - exp(-take * field.depth * field.depth);
    if alpha <= 0.002 {
        return base;
    }

    // Which way the light grows, from taps about a cloud apart. A flat field
    // has no direction, and the terms that need one fade out there.
    let cloud_points = cloud.size.y / units;
    let reach = vec2<f32>(cloud_points * 0.5, 0.0);
    let grad = vec2<f32>(
        cloud_light(pt + reach.xy) - cloud_light(pt - reach.xy),
        cloud_light(pt + reach.yx) - cloud_light(pt - reach.yx),
    );
    let magnitude = length(grad);
    // Capped, so the sound LEANS the light rather than aiming it. A
    // spectrogram of bands has a gradient that reverses between every pair of
    // them, and a light that follows it turns over as often: neighbouring
    // clouds end up lit from opposite sides, their shading cancels across the
    // pane and every one of them reads flat. One light everywhere is what
    // makes them bodies.
    let directed = 0.55 * smoothstep(0.0, 0.03, magnitude);
    // A fixed fill light carries it, so a cloud always has a lit face; the
    // sound's own gradient leans it wherever it has one, and that is what
    // moves the sheen as the picture scrolls.
    let toward =
        normalize(mix(vec2<f32>(0.55, -0.83), grad / max(magnitude, 0.00001), directed));
    let aimed = 0.5 + 0.5 * directed;

    // The light this point shows is the light under the puff that covers it
    // most, faded back to its own where no puff dominates. That is the whole
    // quantization: a puff carries one reading of the sound across its width,
    // and nothing anywhere draws the boundary between two of them.
    let centre_pt = (field.lit_centre - cloud.drift) / units * cloud.size.y + cloud.size * 0.5;
    // Faded out where barely any puff covers the point, which is the only
    // place the average of their centres says nothing.
    // Held down deliberately. Reading the light a scale away is what quantizes
    // it, and it is also what slides the picture under the cloud sideways —
    // much of it at all stops reading as a scale catching its own light and
    // starts reading as refraction through water, which is what it took to
    // keep out of the picture.
    let quantized = 0.2 * smoothstep(0.0, 0.1, field.lit_weight);
    let light = mix(cloud_light(pt), cloud_light(centre_pt), quantized);

    // The pile's slope is the relief, at every octave at once and at the same
    // strength wherever the layer reaches. Diffuse is 1 on a flat scale, so a
    // relief of 0 leaves the light alone, and the glint is the lobe's excess
    // over what a flat scale returns. The light stands 45 degrees over the
    // plane, on the side it grows toward.
    let normal = normalize(vec3<f32>(-field.slope * (3.2 * cloud.scale_glint), 1.0));
    let sun = normalize(vec3<f32>(toward * 0.7, 0.7));
    // Wrapped rather than clamped at the terminator: light crosses a cloud
    // instead of stopping at its surface, and a hard clamp at zero draws the
    // steep side of a scale as a black edge with a visible boundary.
    let diffuse = wrapped_light(dot(normal, sun)) / wrapped_light(sun.z);
    // A broad sheen and not a specular highlight: a tight lobe puts hard
    // bright specks on the scales, which is exactly what light on water looks
    // like. Wide enough that what it adds is a sheen across a whole scale.
    let half = normalize(sun + vec3<f32>(0.0, 0.0, 1.0));
    let flat_glint = pow(half.z, 6.0);
    let glint = max(pow(max(dot(normal, half), 0.0), 6.0) - flat_glint, 0.0)
        / (1.0 - flat_glint) * aimed;

    // The largest puffs shade the body under the grain: the face they turn
    // toward the light is its bright rim. Deep cloud carries more of the light
    // than a one-scale veil does, which is what makes the pile read as a body
    // rather than as an even haze — at a cover high enough to reach the whole
    // pane the opacity barely varies, so the shapes have to be in the shading.
    let rim = clamp(dot(-field.body_slope, toward) * 0.8, 0.0, 1.0) * aimed;
    let thickness = clamp(field.depth * 1.1, 0.0, 1.4);
    let shading = diffuse * (0.3 + 0.8 * thickness) * (0.85 + 0.45 * rim);
    // The sound picks the colour and the cloud's own shape scales it, rather
    // than the shape being folded into the palette's level: a lit body over a
    // ridge would otherwise clip flat white at the top of the ramp and lose
    // exactly the form this layer is for. Ambient is a floor under the level,
    // so a cloud over silence still has a colour to be shaded.
    // Compressed hard before it becomes a colour. The "wide" blur is wide in
    // CENTS and milliseconds, not in pixels, so it still carries the
    // spectrogram's bands at nearly full contrast — and a cloud whose own
    // colour steps at every band edge is a sheet of water with the picture
    // showing through it, whatever its shape is doing. Under a root the gaps
    // between bands come up nearly to the bands, the sound stays a tint, and
    // what is left to vary across the cloud is the cloud.
    let tint = pow(clamp(light, 0.0, 1.0), 0.55);
    let level = clamp(tint * 0.8 + cloud.cloud_ambient, 0.0, 1.0);
    let body = palette_color(level) * shading
        + vec3<f32>(glint * cloud.scale_glint * level * 0.25);
    return mix(base, body, cloud.cloud_depth * alpha);
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
