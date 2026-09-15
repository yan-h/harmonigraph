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
    motion: vec4<f32>,
    breath: vec4<f32>,
    field: vec4<f32>,
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
fn nebula_hash(cell: vec2<i32>) -> f32 {
    var n = bitcast<u32>(cell.x) * 0x9e3779b9u ^ bitcast<u32>(cell.y);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    return f32(n >> 8u) / 16777216.0;
}

fn nebula_noise(p: vec2<f32>) -> f32 {
    let cell = vec2<i32>(floor(p));
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(nebula_hash(cell), nebula_hash(cell + vec2<i32>(1, 0)), w.x),
        mix(nebula_hash(cell + vec2<i32>(0, 1)), nebula_hash(cell + vec2<i32>(1, 1)), w.x),
        w.y,
    );
}

// Read the audio field through a moving, folded coordinate map. Sampling
// beyond its real region returns silence rather than stretching an edge texel.
fn cloud_sample(point: vec2<f32>) -> f32 {
    let uv = (point - cloud.origin) / cloud.size;
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0))
        || any(point < cloud.field.xy) || any(point > cloud.field.xy + cloud.field.zw) {
        return 0.0;
    }
    return textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
}

fn cloud_density(position: vec2<f32>) -> f32 {
    let point = position / cloud.ppp;
    let cell_size = max(cloud.field.w, 1.0) * cloud.motion.z / 5.0;
    let p = (point - cloud.field.xy - cloud.field.zw * 0.5) / cell_size;
    let drift = cloud.motion.xy;
    let bend = vec2<f32>(nebula_noise(p + drift), nebula_noise(p + vec2<f32>(8.3, 2.7) - drift)) - 0.5;
    let folded = p + bend * 2.4;
    let curl = vec2<f32>(
        nebula_noise(folded * 2.3 - drift + vec2<f32>(3.1, 7.4)),
        nebula_noise(folded * 2.3 + drift + vec2<f32>(6.7, 1.2)),
    ) - 0.5;
    let phase = p.x * 0.8 + p.y * 0.5;
    let wave = 0.5 + sin(cloud.breath.x + phase) / 3.0
        + sin(cloud.breath.y + phase * 1.7) / 6.0;
    let breath = 1.0 - cloud.breath.z * (1.0 - wave);
    // The broad bend carries whole bands; the smaller fold pulls their edges
    // into tendrils. Breathing changes their reach as well as their intensity.
    let reach = cell_size * cloud.motion.w * (0.65 + 0.35 * breath);
    let carried = point + (bend * 1.5 + curl * 0.65) * reach;
    let strand = vec2<f32>(curl.y, -curl.x) * reach * 0.35;
    let body = cloud_sample(carried);
    let filament = (cloud_sample(carried + strand) + cloud_sample(carried - strand)) * 0.5;
    let billow = nebula_noise(folded + drift);
    let density = 0.22 + 0.96 * smoothstep(0.20, 0.78, billow * 0.7 + (curl.x + 0.5) * 0.3);
    // Shape intensity BEFORE the palette lookup: colors migrate through the
    // authored ramp as the material folds and breathes. No RGB veil remains.
    return mix(body, filament, 0.35 * cloud.motion.w)
        * mix(1.0, density, cloud.motion.w) * breath;
}
fn density_color(raw_level: f32) -> vec4<f32> {
    let level = style_level(raw_level);
    // Interpolate the authored palette's center samples only after diffusion.
    // The first half-slice joins true black smoothly, even for an edited ramp
    // whose first sample is nonblack; there is no separate halo color curve.
    let levels = textureDimensions(lut).x;
    let x = clamp(level, 0.0, 1.0) * f32(levels) - 0.5;
    let i = u32(clamp(floor(x), 0.0, f32(levels - 1u)));
    let a = textureLoad(lut, vec2<u32>(i, 0u), 0).rgb;
    if x < 0.0 {
        return vec4<f32>(a * (x + 0.5) * 2.0, 1.0);
    }
    let b = textureLoad(lut, vec2<u32>(min(i + 1u, levels - 1u), 0u), 0).rgb;
    return vec4<f32>(mix(a, b, fract(x)), 1.0);
}
fn backdrop_color(position: vec2<f32>) -> vec4<f32> {
    if cloud.style == 3u { return density_color(cloud_density(position)); }
    return density_color(smoothed_level(0.0, baked_density(position)));
}
// The full region shares one scalar material. Clouds may carry measured
// color into neighboring empty history; zero beyond the source bounds keeps
// this from becoming an indefinitely held oldest/newest column.
@fragment
fn fs_cloud_backdrop_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return backdrop_color(in.position.xy);
}
@fragment
fn fs_cloud_backdrop_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = backdrop_color(in.position.xy);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), 1.0);
}
fn cloud_color(in: VertexOut) -> vec4<f32> {
    if cloud.style == 3u { return density_color(cloud_density(in.position.xy)); }
    return density_color(smoothed_level(heatmap_level(in), baked_density(in.position.xy)));
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
