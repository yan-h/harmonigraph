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
    // Watercolour clouds. `drift` is the wash's offset in cloud units and
    // `time` a bounded clock; the rest are the sanitized settings. The filter
    // shader declares only the head of this struct, which is why these are
    // appended rather than interleaved.
    drift: vec2<f32>,
    time: f32,
    cloud_depth: f32,
    cloud_scale: f32,
    cloud_cover: f32,
    scale_size: f32,
    scale_refract: f32,
    scale_relief: f32,
    scale_glint: f32,
    cloud_ambient: f32,
    _pad2: f32,
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

// Refracting scale clouds (prototype).
//
// Round 1 of #888 read the light at the nearest DOME'S CENTRE instead of under
// the pixel, so every scale showed the spectrogram sampled from somewhere else
// and the picture came apart into bent facets. That is the one thing Yan has
// asked for twice — *"it actually looked like the scales were refracting the
// light"* — and rounds 2 through 6 removed it, each for a locally good reason.
// Round 5's notes call it a defect in as many words: "the light was read at the
// NEAREST scale's centre; that steps the reading across the bisector between
// two scales — a straight edge through a cloud", and replaced it with an
// average over the covering scales. The SEAM was the bug. The DISPLACEMENT was
// the feature. They went out together, and everything after was paint laid over
// the picture rather than a lens in front of it.
//
// So the displaced lookup is back, and the cell pick is not. The relief is a
// pile of soft round domes joined by a soft union, and the light is bent by its
// SLOPE. A nearest-cell pick steps across a bisector; a slope turns
// continuously, so the picture bends where it used to break and there is no
// boundary left anywhere to draw. That is the whole of "keep the refraction,
// just make it softer" — the softness is a property of the construction rather
// than a blur applied to a hard thing afterwards.
//
// What is kept from round 1, deliberately and verbatim in structure, because it
// is what Yan liked: the light is LIFTED so a cloud over a ridge glows nearly as
// bright as the ridge rather than reading as a shadow on it; the sun leans with
// the picture's own gradient, which is what moves the glints as the sound
// scrolls; there is a specular glint, a rim on the edge facing the light, and an
// ambient floor so a cloud is visible over dark sky.
//
// Two things that are NOT round 1:
//
// **The union is a union, not a sum.** Adding two overlapping domes makes one
// taller smooth mound and cancels the slopes exactly where the near dome's face
// should be — no faces, and the faces are the scales. `log(sum exp(k*h))/k`
// keeps both, and its gradient is the same weights against each dome's own
// slope, so one pass gives the height and the normal together.
//
// **The slope is normalised before it bends anything.** A dome's slope goes as
// one over its radius, so a raw slope would make Scale size silently a second
// refraction knob — halve the scale and the picture bends twice as far. Divided
// by a dome's own peak slope, `scale_refract` is an offset in SCALE WIDTHS and
// means one thing at every size.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI: five
// cloud units across the pane's height at size 1, like the lattice nebula.

fn cloud_gradient(cell: vec2<i32>) -> vec2<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    let angle = f32(n) * (6.2831853 / 4294967296.0);
    return vec2<f32>(cos(angle), sin(angle));
}

fn cloud_fade(t: vec2<f32>) -> vec2<f32> {
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn cloud_noise(p: vec2<f32>) -> f32 {
    let base = floor(p);
    let f = p - base;
    let c = vec2<i32>(base);
    let g00 = cloud_gradient(c);
    let g10 = cloud_gradient(c + vec2<i32>(1, 0));
    let g01 = cloud_gradient(c + vec2<i32>(0, 1));
    let g11 = cloud_gradient(c + vec2<i32>(1, 1));
    let n00 = dot(g00, f);
    let n10 = dot(g10, f - vec2<f32>(1.0, 0.0));
    let n01 = dot(g01, f - vec2<f32>(0.0, 1.0));
    let n11 = dot(g11, f - vec2<f32>(1.0, 1.0));
    let u = cloud_fade(f);
    return mix(mix(n00, n10, u.x), mix(n01, n11, u.x), u.y) * 1.4;
}

// Rotated between octaves rather than only scaled: octaves on the same axes
// stack whatever alignment each one has left, and the sum shows a weave.
const CLOUD_TURN: mat2x2<f32> = mat2x2<f32>(0.80, 0.60, -0.60, 0.80);

fn cloud_fbm(point: vec2<f32>, octaves: i32, gain: f32) -> f32 {
    var p = point;
    var amplitude = 1.0;
    var total = 0.0;
    var norm = 0.0;
    for (var i = 0; i < octaves; i += 1) {
        total += amplitude * cloud_noise(p);
        norm += amplitude;
        p = CLOUD_TURN * p * 2.0;
        amplitude *= gain;
    }
    return total / max(norm, 0.0001);
}

// The light under the layer, decoded, at the two scales the filter chain
// already produced. `close` is the analyser's own softening and `wide` is that
// softened again, so the pair is a ready-made band-pass at no cost.

// Jitter and a liveness draw for one cell, as three 10-bit fractions.
fn cloud_hash3(cell: vec2<i32>) -> vec3<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    return vec3<f32>(
        f32(n & 0x3ffu) / 1023.0,
        f32((n >> 10u) & 0x3ffu) / 1023.0,
        f32((n >> 20u) & 0x3ffu) / 1023.0,
    );
}

// How far a dome reaches past its own cell, and how far its centre may wander
// inside it. Both are the round-5 values: a dome that reaches its neighbours is
// what makes the pile continuous, and the jitter is what stops the grid reading
// as a grid.
const DOME_RADIUS: f32 = 1.15;
const DOME_JITTER: f32 = 0.75;
// Hardness of the soft union. Low is putty, high is a crease; this is where a
// pile of domes still has faces and does not yet have edges.
const DOME_UNION: f32 = 9.0;
// The steepest a unit dome gets, which is what `scale_refract` is measured
// against. h = (1 - d^2)^1.5, so |dh/dd| peaks at d = 1/sqrt(2) and equals 1.5.
const DOME_PEAK_SLOPE: f32 = 1.5;

struct Pile {
    // Soft-union height of the domes covering this point, 0 where none do.
    height: f32,
    // Its slope, in cell units: the face the scales here present to the light.
    slope: vec2<f32>,
};

// One octave of domes: a soft union over the 3x3 ring, with the union's own
// weights carrying each dome's analytic slope out alongside its height.
//
// `occupancy` is what opens sky. One dome per cell on a jittered grid is a
// blue-noise point set — the most UNIFORM arrangement there is — so with each
// dome reaching past its cell the layer covers the pane by construction and can
// never open a gap. Cells are empty at a rate instead.
fn dome_octave(r: vec2<f32>, occupancy: f32) -> Pile {
    let base = floor(r);
    var weight = 0.0;
    var slope = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let cell = vec2<i32>(base) + vec2<i32>(i, j);
            let h3 = cloud_hash3(cell);
            if h3.z >= occupancy {
                continue;
            }
            let centre = base + vec2<f32>(f32(i), f32(j)) + 0.5
                + (h3.xy - 0.5) * DOME_JITTER;
            let d = (r - centre) / DOME_RADIUS;
            let q = 1.0 - dot(d, d);
            if q <= 0.0 {
                continue;
            }
            // h = q^1.5, so dh/dr = 1.5 * q^0.5 * (-2 d) / DOME_RADIUS
            let root = sqrt(q);
            let h = q * root;
            let w = exp(DOME_UNION * h);
            weight += w;
            slope += w * (-3.0 * root * d / DOME_RADIUS);
        }
    }
    var out: Pile;
    if weight <= 0.0 {
        out.height = 0.0;
        out.slope = vec2<f32>(0.0);
        return out;
    }
    out.height = log(weight) / DOME_UNION;
    out.slope = slope / weight;
    return out;
}

// Two octaves, the finer one damped hard.
//
// Not a taste setting: a finer octave's SLOPE is larger than a coarser one's at
// equal amplitude, by exactly the lacunarity, so an fbm that halves amplitude
// per octave still hands the gradient to its finest octave — and the gradient is
// what bends the light here. Carried at full strength the small scales are a
// crinkled terrain, which is round 3's "more like water with light cast on it
// than clouds" and Yan's "a bit too jagged" in one. Damped by the square of the
// lacunarity, each octave contributes about equally to the slope, which is what
// puts big faces carrying small ones into the same picture.
const DOME_LACUNARITY: f32 = 2.1;
const DOME_FINE_GAIN: f32 = 0.22;

fn cloud_domes(r: vec2<f32>, occupancy: f32) -> Pile {
    let coarse = dome_octave(r, occupancy);
    let fine = dome_octave(r * DOME_LACUNARITY + vec2<f32>(17.3, 5.9), occupancy);
    var out: Pile;
    let norm = 1.0 + DOME_FINE_GAIN;
    out.height = (coarse.height + DOME_FINE_GAIN * fine.height) / norm;
    // the finer octave's slope arrives in ITS cell units, so it carries the
    // lacunarity back out with it
    out.slope = (coarse.slope + DOME_FINE_GAIN * DOME_LACUNARITY * fine.slope) / norm;
    return out;
}

// The light under a pane point, display intensity 0..1: the wide blur, floored
// by most of the finished material so a cloud over a ridge is nearly as bright
// as the ridge. The wide blur alone spreads a narrow ridge's energy so thin
// that a cloud over it reads as a shadow, which is what makes the layer look
// like something laid ON the picture rather than lit BY it.
//
// `close_light` is NOT decoded: that attachment holds the finished scalar
// material `fs_cloud_light` already decoded on its way out, which is why
// `baked_density` reads it raw too. Decoding it twice lifts a flat 0.59 to 0.75
// and invents structure out of a picture that has none.
fn cloud_light(pt: vec2<f32>) -> f32 {
    let uv = pt / cloud.size;
    let wide = density_decode(textureSampleLevel(wide_light, cloud_sampler, uv, 0.0).r);
    let material = textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
    return 1.4 * max(wide, 0.85 * material);
}

// Compress rather than clip, keeping the hue. See where it is applied.
fn softened(colour: vec3<f32>) -> vec3<f32> {
    let m = max(max(colour.r, colour.g), colour.b);
    if m <= 0.75 {
        return max(colour, vec3<f32>(0.0));
    }
    return max(colour, vec3<f32>(0.0))
        * ((0.75 + 0.25 * (1.0 - exp((0.75 - m) * 4.0))) / m);
}

// A drifting cloud field with a flat interior behind a soft edge, so cover
// moves the threshold rather than the contrast.
fn cloud_billow(q: vec2<f32>) -> f32 {
    let warp = 0.7 * vec2<f32>(
        cloud_fbm(q * 0.7, 2, 0.5),
        cloud_fbm(q * 0.7 + vec2<f32>(8.3, 2.7), 2, 0.5),
    );
    return 0.5 + 0.5 * cloud_fbm(q + warp, 4, 0.5);
}

fn scale_clouds(base: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    // No blur means no light field for the scales to bend.
    if cloud.cloud_depth <= 0.0 || all(cloud.step == vec2<f32>(0.0)) {
        return base;
    }
    let pt = position / cloud.ppp - cloud.origin;
    let units = 5.0 / cloud.cloud_scale;
    let q = (pt - cloud.size * 0.5) / cloud.size.y * units + cloud.drift;

    // The cloud's own shape, and the sky between. Cover moves the threshold.
    let low = 0.62 - 0.40 * cloud.cloud_cover;
    let density = smoothstep(low, low + 0.30, cloud_billow(q));
    if density <= 0.003 {
        return base;
    }

    // The scales. `scale_size` is how many of them cross one cloud, so the knob
    // reads as a size rather than as a frequency.
    let scale_units = 6.0 / cloud.scale_size;
    let scale_points = cloud.size.y / units / scale_units;
    let pile = cloud_domes(q * scale_units, 0.78);

    // THE REFRACTION. Normalised to a dome's own peak slope, so the offset is in
    // scale widths whatever the scale size is, and scaled by the cloud's density
    // so a wisp bends the light less than a body does.
    let face = pile.slope / DOME_PEAK_SLOPE;
    let bend = cloud.scale_refract * scale_points * density;
    let bent = cloud_light(pt - face * bend);

    // Which way the picture's light grows, from taps about a scale apart, so the
    // sun leans with the sound and the glints travel as it scrolls. A flat field
    // has no direction and the terms that need one fade out there.
    let reach = vec2<f32>(scale_points * 0.75, 0.0);
    let grad = vec2<f32>(
        cloud_light(pt + reach.xy) - cloud_light(pt - reach.xy),
        cloud_light(pt + reach.yx) - cloud_light(pt - reach.yx),
    );
    let magnitude = length(grad);
    let directed = smoothstep(0.0, 0.03, magnitude);
    let toward =
        normalize(mix(vec2<f32>(0.55, -0.83), grad / max(magnitude, 0.00001), directed));
    let aimed = 0.5 + 0.5 * directed;

    // The scales' normal, from the same slope that bent the light, flattened by
    // `scale_relief` so 0 is a smooth body with no faces at all.
    let relief = cloud.scale_relief * density;
    let normal = normalize(vec3<f32>(-face * relief, 1.0));
    // The light stands 45 degrees over the plane, on the side it grows toward.
    // Diffuse is 1 on a flat face, so a relief of 0 leaves the light alone.
    let sun = normalize(vec3<f32>(toward * 0.7, 0.7));
    let diffuse = max(dot(normal, sun), 0.0) / sun.z;
    let half = normalize(sun + vec3<f32>(0.0, 0.0, 1.0));
    let flat_glint = pow(half.z, 14.0);
    let glint = max(pow(max(dot(normal, half), 0.0), 14.0) - flat_glint, 0.0)
        / (1.0 - flat_glint) * aimed;

    // The edge facing the light is the bright rim and the thick core is dimmer,
    // which is what makes a body of it rather than a flat patch.
    let ahead = smoothstep(low, low + 0.30, cloud_billow(q + toward * 0.18));
    let rim = clamp((density - ahead) * 2.5, 0.0, 1.0) * aimed;
    let shade = 1.2 - 0.4 * density;
    let lit = bent * diffuse * shade * (0.8 + 0.6 * rim) + cloud.cloud_ambient;
    let body = palette_color(clamp(lit, 0.0, 1.0))
        + vec3<f32>(glint * cloud.scale_glint * bent * 0.5);
    // The glint is ADDITIVE, so at a deep cover it runs past what the palette
    // can hold — which is round (4)'s bright metallic patches, and they got
    // worse as the glint was raised because the glint is what pushed it over.
    // Compressing the MAGNITUDE and keeping the direction keeps the hue: a
    // clipped channel shifts the colour as well as flattening it, and that
    // shift is what reads as metal rather than as a bright cloud. Kept from
    // round 6, which is the one thing in it that this round should not undo.
    return mix(base, softened(body), cloud.cloud_depth * density);
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
