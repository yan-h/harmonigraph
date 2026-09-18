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
    scale_size: f32,
    scale_variety: f32,
    scale_refract: f32,
    scale_relief: f32,
    scale_shade_floor: f32,
    scale_facet: f32,
    scale_rock: f32,
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

// A refracting scale TEXTURE (prototype).
//
// It was clouds over sky until Yan saw one: *"I feel like in the current
// prototype there are gaps in cloud cover even at 100%. I don't want this. I
// want the texture everywhere, and the cloud cover slider to be gone."* — and,
// on a reference sheet of watercolour cumulus: *"I want a consistent texture at
// the macro level, but different lobe sizes at the micro level"*, *"I want the
// texture, not the exact shape of how clouds behave in real life. No macro level
// variation from bottom of spectrogram to the top."*
//
// So there is no cloud SHAPE here any more and no sky between: one continuous
// field of globs over the whole pane, statistically the same everywhere, and the
// variation Yan wants is in the globs' own sizes rather than in where they
// gather. The drifting billow that used to carve cloud bodies out of it is gone
// along with everything that only existed to serve it — the threshold, the
// per-pixel `density`, the bright rim on a cloud's leading edge, and the whole
// perlin chain underneath. What is left is the domes, which were always the
// thing being looked at.
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
// What is kept from round 1: the light is LIFTED so a glob over a ridge glows
// nearly as bright as the ridge rather than reading as a shadow on it, and the
// sun leans with the picture's own gradient, which is what moves the shading as
// the sound scrolls.
//
// What is NOT kept, both on Yan seeing the built thing: the specular glint and
// its exponent (*"I don't like glint or sparkle, we can remove them"*), and the
// ambient floor (*"'Ambient' slider is useless"*). Diffuse alone shapes the
// texture now, and over silence the layer draws the palette's own bottom.
//
// Two things that are NOT round 1:
//
// **The union is a union, not a sum.** Adding two overlapping domes makes one
// taller smooth mound and cancels the slopes exactly where the near dome's face
// should be — no faces, and the faces are the scales. `exp(k*h)` weights keep
// both, and the same weights carry each dome's own slope, so one pass gives the
// face and everything else keyed on it together.
//
// **The slope is normalised before it bends anything.** A dome's slope goes as
// one over its radius, so a raw slope would make Scale size silently a second
// refraction knob — halve the scale and the picture bends twice as far.
// `DOME_FACE` takes that out, and takes out the same effect WITHIN one field now
// that `Variety` gives each glob its own radius.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI: five
// cloud units across the pane's height at size 1, like the lattice nebula.
//
// Four qualities are on DIALS rather than decided here, because describing which
// of them Yan wants has failed in words repeatedly: `Facet` carries the lookup
// from this round's continuous slope onto round 1's flat per-glob patch, `Rock`
// is round 1's per-dome clock, `Variety` is how much the globs differ in size,
// and `Shade floor` is how dark a face turned away from the sun may get — the
// one quantity in the fix below that is taste rather than correctness.

// One cell's dome, as three 10-bit fractions: where its centre sits inside the
// cell, and how wide it is.
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

// How far a dome reaches past its own cell, how far its centre may wander
// inside it, and the band `Variety` draws each dome's own radius from.
//
// THESE FOUR ARE A PROOF, not four independent tastes, and the two inequalities
// they have to satisfy are held by `the_dome_grid_covers_the_plane_and_the_ring_holds_it`.
//
// **Coverage.** A centre sits at its cell's middle give or take `JITTER/2`, so
// the point hardest to reach is a lattice corner with all four cells touching it
// pushed diagonally away: `(0.5 + JITTER/2) * sqrt(2)` from every one of them.
// The SMALLEST radius a dome can draw has to clear that, or there is a pinhole
// in the layer where no dome reaches — and a pinhole is not a dim spot, it is a
// place where `to_centre` falls off a cliff from most of a radius to nothing,
// which is the hard edge this whole construction exists not to draw.
//
// **Reach.** The union only visits the 3x3 ring, so a dome outside it must not
// be able to touch this pixel. The nearest a cell two out can put its centre is
// `2.5 - JITTER/2` from the pixel's own cell origin, and the pixel is at most 1
// past that origin, so the LARGEST radius has to stay under `1.5 - JITTER/2`.
//
// Round 5's jitter of 0.75 satisfied NEITHER (it wanted a radius at once above
// 1.237 and below 1.125, which is empty), and both failures were live: cells
// were 22% empty then, so the pinholes were being drawn on purpose, and a dome
// two cells out reaching in is a step on the cell grid every time `floor(r)`
// moves. Dropping the jitter to 0.30 opens a band of [0.919, 1.350] and leaves
// room for `Variety` inside it.
const DOME_RADIUS: f32 = 1.15;
const DOME_JITTER: f32 = 0.30;
const DOME_RADIUS_MIN: f32 = 0.95;
const DOME_RADIUS_MAX: f32 = 1.32;
// Hardness of the soft union. Low is putty, high is a crease; this is where a
// pile of domes still has faces and does not yet have edges.
const DOME_UNION: f32 = 9.0;
// What turns a dome's analytic slope into the FACE the light is bent by.
//
// `h = q^1.5` gives `dh/dr = -3 * root * d / R`, whose steepest point is
// `1.5 / R` — so on a global normalisation a SMALLER dome bends the light
// FURTHER, which draws a little glob displacing a patch bigger than itself.
// Multiplying by `R^2 / (1.5 * DOME_RADIUS^2)` instead leaves `-2 * root * d *
// R / DOME_RADIUS^2`: a peak of `R / DOME_RADIUS`, so a glob carries the light
// as far as it is wide, and a dome at the base radius bends exactly what it did
// before `Variety` existed.
const DOME_FACE: f32 = 2.0 / (DOME_RADIUS * DOME_RADIUS);
// How far off its own face a scale's normal may be rocked, at the top of the
// dial. Round 1 rocked by 0.12 against a tilt vector that reached about 0.5, so
// round 1's whole wobble sits around 40% of the way up this one and the rest of
// the dial is past anything that has been seen.
const ROCK_TILT: f32 = 0.30;
// How far the sun may lean off vertical, and the gradient at which it has leant
// half that far. `SUN_LEAN` of 1 against a height of 1 is 45 degrees, which is
// where the sun stood before it was allowed to stand up.
const SUN_LEAN: f32 = 1.0;
const SUN_KNEE: f32 = 0.03;

struct Pile {
    // The face the scales here present to the light: each covering dome's own
    // slope, normalised by `DOME_FACE` and blended by the union's weights.
    face: vec2<f32>,
    // Where the domes covering this point keep their CENTRES, as an offset from
    // the point in cell units. Inside a dome one weight runs away with the
    // union, so this is `centre - r` and `r + to_centre` is the CONSTANT centre —
    // a flat facet. On a bisector the two weights are equal and it is their
    // mean, so the reading turns over continuously where round 1's nearest-cell
    // pick stepped. That is the whole difference between the two.
    to_centre: vec2<f32>,
    // Each dome's own slow wobble at unit amplitude, blended by the same
    // weights, so the scales rock past each other rather than together.
    rock: vec2<f32>,
};

// One octave of domes: a soft union over the 3x3 ring, with the union's own
// weights carrying each dome's normalised face out alongside the rest.
//
// EVERY cell has a dome. There used to be an `occupancy` draw that left 22% of
// them empty, which is where the sky between the clouds came from; Yan wants the
// texture everywhere, so the draw is gone and `h3.z` went with it — freed, and
// spent on the radius below.
fn dome_octave(r: vec2<f32>) -> Pile {
    let base = floor(r);
    var weight = 0.0;
    var face = vec2<f32>(0.0);
    var to_centre = vec2<f32>(0.0);
    var rock = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let cell = vec2<i32>(base) + vec2<i32>(i, j);
            let h3 = cloud_hash3(cell);
            let centre = base + vec2<f32>(f32(i), f32(j)) + 0.5
                + (h3.xy - 0.5) * DOME_JITTER;
            // Each dome's own width. `Variety` opens the band from the single
            // shared radius, never below `DOME_RADIUS_MIN`, so every step of the
            // dial is still a proof that the plane is covered.
            let radius = mix(
                DOME_RADIUS,
                mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, h3.z),
                cloud.scale_variety,
            );
            let d = (r - centre) / radius;
            let q = 1.0 - dot(d, d);
            if q <= 0.0 {
                continue;
            }
            let root = sqrt(q);
            let h = q * root;
            let w = exp(DOME_UNION * h);
            weight += w;
            face += w * (-(DOME_FACE * root * radius)) * d;
            to_centre += w * (centre - r);
            // Round 1's clock, rates and phases unchanged: each dome turns at
            // its own rate from its own offset, so no two scales beat together.
            // Behind the knob because a sine and a cosine per dome per pixel is
            // real work to do for an amplitude of zero, and the branch is on a
            // uniform, so no two lanes ever disagree about taking it.
            if cloud.scale_rock > 0.0 {
                rock += w * vec2<f32>(
                    sin(cloud.time * (0.2 + 0.3 * h3.x) + h3.y * 6.2831853),
                    cos(cloud.time * (0.25 + 0.2 * h3.y) + h3.x * 6.2831853),
                );
            }
        }
    }
    var out: Pile;
    // Unreachable while the constants hold — see the proof on `DOME_RADIUS` —
    // and kept as the divide's guard rather than as a case the picture has. It
    // is what an uncovered point WOULD draw: a flat unbent face, and beside it a
    // `to_centre` that has just fallen from most of a radius to nothing.
    if weight <= 0.0 {
        out.face = vec2<f32>(0.0);
        out.to_centre = vec2<f32>(0.0);
        out.rock = vec2<f32>(0.0);
        return out;
    }
    out.face = face / weight;
    out.to_centre = to_centre / weight;
    out.rock = rock / weight;
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

fn cloud_domes(r: vec2<f32>) -> Pile {
    let coarse = dome_octave(r);
    let fine = dome_octave(r * DOME_LACUNARITY + vec2<f32>(17.3, 5.9));
    var out: Pile;
    let norm = 1.0 + DOME_FINE_GAIN;
    // the finer octave's face arrives in ITS cell units, so it carries the
    // lacunarity back out with it
    out.face = (coarse.face + DOME_FINE_GAIN * DOME_LACUNARITY * fine.face) / norm;
    // The facet is the COARSE octave's alone, and that is not an omission. A
    // facet is flat because one dome's centre answers for its whole interior,
    // so mixing a second octave in puts a finer mosaic inside every patch and
    // takes the flatness back out. Worse, the fine octave's bisectors are
    // 2.1 times closer together and its swing between centres turns over inside
    // a pixel — a hard edge in miniature, everywhere, which is the one thing
    // this construction exists to avoid. The crinkle it carries still reaches
    // the picture through the SLOPE above, which is where it belongs: it is
    // surface, not a scale.
    out.to_centre = coarse.to_centre;
    // An amplitude rather than a derivative, so this takes the height's
    // combination and not the slope's — no lacunarity.
    out.rock = (coarse.rock + DOME_FINE_GAIN * fine.rock) / norm;
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


// How much of the light survives the layer, now that the layer is everywhere.
//
// It used to be `(1.2 - 0.4 * density) * (0.8 + 0.6 * rim)`: a body darker than
// its own lit edge, which is what made a cloud read as a thing with a front and
// a middle. With no sky to have an edge against, both terms are constants —
// `density` is 1 and `rim` is 0 — and this is what they multiply to. It dims the
// WHOLE pane now rather than only the parts a cloud covered, and `Cloud depth`
// is the dial that lets the picture back through.
const CLOUD_SHADE: f32 = 0.64;

fn scale_clouds(base: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    // No blur means no light field for the scales to bend.
    if cloud.cloud_depth <= 0.0 || all(cloud.step == vec2<f32>(0.0)) {
        return base;
    }
    let pt = position / cloud.ppp - cloud.origin;
    let units = 5.0 / cloud.cloud_scale;
    let q = (pt - cloud.size * 0.5) / cloud.size.y * units + cloud.drift;

    // The scales. `scale_size` is how many of them cross one cloud unit, so the
    // knob reads as a size rather than as a frequency.
    let scale_units = 6.0 / cloud.scale_size;
    let scale_points = cloud.size.y / units / scale_units;
    let pile = cloud_domes(q * scale_units);

    // THE REFRACTION. `DOME_FACE` has already put the offset in scale widths
    // whatever the scale size is, and in each glob's OWN width whatever
    // `Variety` has made of it.
    let face = pile.face;
    let bend = cloud.scale_refract * scale_points;
    // `scale_facet` swings that offset off the face the scale PRESENTS and onto
    // the scale's own CENTRE, which is round 1's reading: one value for the
    // whole scale, so the picture comes apart into flat quantized patches
    // instead of bending through them. `to_centre` is in cells and `scale_points`
    // is how many pane points a cell is, so `pile.to_centre * scale_points` lands
    // exactly on the dome's centre — the same arithmetic round 1 spelled out as
    // `centre_pt`.
    //
    // `scale_refract` scales the face half only. At facet 1 the reading is the
    // centre whatever Refraction says, because the two are then measuring
    // different things: Refraction is how far a FACE carries the light, and a
    // facet has stopped asking the face. Dialling one down to look at the other
    // is what this build is for, so they are kept from cancelling.
    let lookup = mix(-face * bend, pile.to_centre * scale_points, cloud.scale_facet);
    let bent = cloud_light(pt + lookup);

    // Which way the picture's light grows, from taps about a scale apart, so the
    // sun leans with the sound and the shading travels as it scrolls.
    //
    // **The lean is the gradient, NOT its direction, and that is the whole of
    // what stopped this tearing.** It used to be
    // `normalize(mix(fallback, grad / magnitude, smoothstep(0, 0.03, magnitude)))`
    // — a UNIT vector aimed along the gradient. A gradient reverses across every
    // crest of the picture, so the sun jumped to the opposite side of the sky
    // along the top of every band, and every face that had been lit turned away
    // in one pixel step. That is the row of near-black vertical tears Yan found
    // along a loud band, and no dial could reach it because the flip is in the
    // sun rather than in the scales. The `smoothstep` was meant to fade the
    // direction out where the field is flat, but its window is far too narrow to
    // matter: inside a loud band `magnitude` stays well above 0.03, so the
    // fallback never engaged and the raw flip ran at full strength.
    //
    // Scaling instead of normalising, the lean passes THROUGH zero at a crest —
    // the sun stands overhead there and comes back down the other side — which
    // is continuous, and it needs no fallback direction at all. That also
    // retires the fixed screen-space `(0.55, -0.83)`, which gave the layer an up
    // and a down that Yan has said he does not want.
    //
    // `SUN_KNEE` is where the lean reaches half of `SUN_LEAN`; a gradient well
    // past it leans the full 45 degrees the old sun always stood at.
    let reach = vec2<f32>(scale_points * 0.75, 0.0);
    let grad = vec2<f32>(
        cloud_light(pt + reach.xy) - cloud_light(pt - reach.xy),
        cloud_light(pt + reach.yx) - cloud_light(pt - reach.yx),
    );
    let lean = grad * (SUN_LEAN / (SUN_KNEE + length(grad)));

    // The scales' normal, from the same face that bent the light, flattened by
    // `scale_relief` so 0 is a smooth body with no faces at all.
    let relief = cloud.scale_relief;
    // Rocked off that face by `scale_rock`, on each dome's own clock, so the
    // shading on a face sways over a picture that is holding still. The LOOKUP
    // is left alone: a scale that rocked the light it refracts would swim, and
    // what round 1 had was the shading travelling over a lens that stayed put.
    var tilt = face;
    if cloud.scale_rock > 0.0 {
        tilt += pile.rock * (cloud.scale_rock * ROCK_TILT);
    }
    let normal = normalize(vec3<f32>(-tilt * relief, 1.0));
    // Diffuse is 1 on a flat face, so a relief of 0 leaves the light alone.
    // `scale_shade_floor` is how much a face turned right away still keeps: the
    // top of the range is untouched by it, so it darkens nothing that was not
    // already dark.
    let sun = normalize(vec3<f32>(lean, 1.0));
    let lambert = max(dot(normal, sun), 0.0) / sun.z;
    let diffuse = mix(cloud.scale_shade_floor, 1.0, lambert);

    let lit = bent * diffuse * CLOUD_SHADE;
    let body = palette_color(clamp(lit, 0.0, 1.0));
    // `softened` used to sit here, compressing anything whose brightest channel
    // ran past 0.75. It was holding back the ADDITIVE glint, and with the glint
    // gone `body` is `palette_color(clamp(lit, 0, 1))` — a colour the palette
    // itself chose, which cannot leave the ramp. Measured on the shipped look it
    // was a complete no-op, byte for byte, at the defaults and at every dial up;
    // the only input that still reached it was an EDITED bright palette, where
    // it dimmed 231k pixels by up to 24/255 — darkening colours Yan had asked
    // for to prevent a clipping that can no longer happen.
    return mix(base, body, cloud.cloud_depth);
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
