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
    cloud_billow: f32,
    cloud_fringe: f32,
    cloud_grain: f32,
    cloud_wash: f32,
    cloud_tide: f32,
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

// Watercolour clouds (prototype).
//
// The clouds ARE the spectrogram. Nothing here has a shape of its own: the
// silhouette is an iso-surface of the sound, so with silence under the pane
// the layer draws nothing at all, and a cloud cannot slide independently of
// what it is made of. That is the whole difference from the pile of puffs this
// replaces, which drew the same shapes in silence and only took its COLOUR
// from the sound — a procedural layer over a static picture, which is exactly
// what it looked like.
//
// There is also no lighting model. Diffuse, terminator, specular and a shadow
// march were four rounds of trying to make a lit body out of a field that had
// no body, and each of them read as gel or as metal. Watercolour has no light
// source: what makes a wash read as paint is where the PIGMENT went, and the
// three cues are all pigment cues. None of them can clip a channel or throw a
// highlight, because none of them is a highlight.
//
// **The field is BAND-PASSED, not blurred.** A blurred spectrogram is high
// wherever the music is loud, so a threshold on it gives overcast in a loud
// passage and clear sky in a quiet one — the layer would track the volume
// knob instead of the music. A difference of two blurs is high only where
// sound is locally CONCENTRATED. It dissolves over a flat wash of energy
// however loud that wash is, it needs no normalisation against the level, and
// it leaves sky between the masses at every loudness.
//
// **The wash is a BODY, not a coverage.** Painting a flat alpha wherever the
// field passes a threshold gives a stain: an outline with nothing inside. The
// opacity runs off how far PAST the threshold the field is, so a mass is thick
// in the middle and thin at the rim.
//
// **Each wash leaves a tide line where it dried.** That is the one cue that
// says watercolour rather than airbrush, and it is free: it is the delta at
// the contour, which `a * (1 - a)` already is. It is weighted toward the side
// the field falls away on, because pigment settles to the low side of a wash —
// an evenly weighted rim around a closed contour is a contour LINE, and a
// stack of them is a topographic map, which is what an early cut drew.
//
// **The warp is a SHEAR field and the fringe is not a warp.** The billow comes
// from displacing the lookup by two noise fields. The bumps on it come from
// adding noise to the FIELD instead, where they roughen the interior as well
// as the outline and cannot chop the silhouette into fragments. Displacing by
// the gradient of one scalar was tried — it is curl-free, so in principle it
// inflates where a shear field shears — and it fails in practice for a reason
// worth writing down: differentiating an fbm multiplies every octave by its
// own frequency, so the gradient is dominated by the FINEST octave however
// small its amplitude, and the contour comes apart into confetti.
//
// **The paper never moves.** Granulation is pigment settling into the tooth of
// the sheet, so it is fixed in pane space while the wash drifts over it. That
// inversion is the point: the old layer was a moving texture over a static
// picture, and this is a moving picture over a static substrate.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI: five
// cloud units across the pane's height at size 1, like the lattice nebula.
// Scrolling never moves the clouds; they are in front of the picture.

// A unit gradient for one lattice corner. Gradient noise rather than value
// noise because value noise is zero-mean only ACROSS a cell, not at its
// corners, so its lattice prints as brightness — visible rectangles at the low
// cell counts a cloud warp runs at.
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
fn cloud_close(uv: vec2<f32>) -> f32 {
    // NOT decoded. This attachment holds the finished scalar material that
    // `fs_cloud_light` already decoded on its way out, not an encoded blur —
    // which is why `baked_density` reads it raw too, and why the old
    // `cloud_light` decoded only its wide tap. Decoding it a second time lifts
    // a flat 0.59 to 0.75 and hands the band-pass a contrast of a quarter out
    // of a picture with no structure in it at all, which is cloud over
    // silence. `clouds_are_cut_from_the_sound_and_not_from_a_field_of_their_own`
    // is what caught it.
    return textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
}
fn cloud_wide(uv: vec2<f32>) -> f32 {
    return density_decode(textureSampleLevel(wide_light, cloud_sampler, uv, 0.0).r);
}

// Six taps over a DISC and not a ring, on the golden-angle spiral that spaces
// them evenly at every radius: a ring gathers from one distance, so its own
// shape prints into a smooth field. This is the band-pass's SURROUND — the
// blur the centre is measured against — and it is the only gather the layer
// takes.
fn cloud_surround(uv: vec2<f32>, reach: vec2<f32>) -> f32 {
    let d0 = vec2<f32>(0.2887, 0.0000);
    let d1 = vec2<f32>(-0.3687, 0.3377);
    let d2 = vec2<f32>(0.0564, -0.6430);
    let d3 = vec2<f32>(0.4647, 0.6061);
    let d4 = vec2<f32>(-0.8528, -0.1508);
    let d5 = vec2<f32>(0.8078, -0.5139);
    return (cloud_wide(uv + d0 * reach) + cloud_wide(uv + d1 * reach)
        + cloud_wide(uv + d2 * reach) + cloud_wide(uv + d3 * reach)
        + cloud_wide(uv + d4 * reach) + cloud_wide(uv + d5 * reach)) / 6.0;
}

// A contrast, not a difference. Dividing by the surround is what makes the
// layer independent of how loud the passage is: the same shape of
// concentration draws the same cloud whether it stands at a tenth of the
// ramp or at the top of it. The floor is what stops near-silence dividing a
// noise floor by itself and drawing weather out of nothing.
const CLOUD_FLOOR: f32 = 0.05;

fn cloud_contrast(centre: f32, around: f32) -> f32 {
    return (centre - around) / max(around, CLOUD_FLOOR);
}

struct Wash {
    // How much pigment this wash laid down here, 0 outside it.
    body: f32,
    // The tide line: the delta at the wash's own contour, weighted to the side
    // the field falls away on.
    tide: f32,
};

// `soft` is in FIELD units, so the edge's width on screen is `soft` over the
// field's own slope — narrow where a mass rises fast and wide where it fades,
// which is what a wash does and what one fixed screen width is not.
fn cloud_wash(field: f32, threshold: f32, soft: f32, tide_width: f32,
              thickness: f32, fall: f32, pool: f32) -> Wash {
    let past = field - threshold;
    let a = smoothstep(-1.0, 1.0, past / max(soft, 0.0001));
    var out: Wash;
    // Depth past the threshold, not coverage of it: thick in the middle, thin
    // at the rim, which is a body rather than a stain.
    out.body = a * (1.0 - exp(-thickness * max(past, 0.0)));
    let edge = smoothstep(-1.0, 1.0, past / max(tide_width, 0.0001));
    // `fall` is +1 where the field falls toward the bottom of the pane, so a
    // mass deposits along its lower edge the way a wash on a tilted board does.
    let weight = 1.0 - pool + pool * clamp(0.5 + 0.5 * fall, 0.0, 1.0);
    out.tide = 4.0 * edge * (1.0 - edge) * weight;
    // No concentration, no pigment — at ANY setting of the dials. Wound to the
    // top, cover pulls the threshold down far enough that the tide line's own
    // width reaches past zero, and a band centred on nothing draws a wash over
    // a pane with no sound in it. The body cannot do this because it runs off
    // `past` directly; the tide can, because it is a band rather than a step.
    let present = smoothstep(0.0, 0.35 * max(threshold, 0.0001), field);
    out.body *= present;
    out.tide *= present;
    return out;
}

// Water in the pigment: paler, and it carries less of its own colour. The one
// dial that runs the layer from a flat tinted wash to a dry brush loaded with
// pigment, and it is a pigment dial rather than a brightness one, so it cannot
// push a channel over the top the way the old sheen did.
fn cloud_dilute(colour: vec3<f32>, water: f32) -> vec3<f32> {
    return mix(colour, colour * 0.30 + 0.70, water);
}

fn scale_clouds(base: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    // No blur means no field to cut the clouds out of.
    if cloud.cloud_depth <= 0.0 || all(cloud.step == vec2<f32>(0.0)) {
        return base;
    }
    let pt = position / cloud.ppp - cloud.origin;
    let units = 5.0 / cloud.cloud_scale;
    let q = (pt - cloud.size * 0.5) / cloud.size.y * units + cloud.drift;
    // One cloud unit, in the uv the light textures are sampled by.
    let unit = vec2<f32>(cloud.size.y / units) / cloud.size;

    // The billow. Two independent fields, so the displacement shears — which
    // is what bends a smooth contour into a mass with lobes. Three octaves:
    // the fine detail belongs on the field, not on the displacement.
    let billow = 1.6 * cloud.cloud_billow;
    let warp = billow * vec2<f32>(
        cloud_fbm(q * 0.62, 3, 0.5),
        cloud_fbm(q * 0.62 + vec2<f32>(19.3, 7.1), 3, 0.5),
    );
    let uv = (pt / cloud.size) + warp * unit;

    // Bumps on bumps, MULTIPLIED into the field rather than added to it. Four
    // octaves at a steep gain, so the silhouette carries detail at every scale
    // down to a pixel or two — a cloud edge has that and a smooth contour does
    // not.
    //
    // Multiplied because an added fringe is a field of its own and draws cloud
    // out of nothing: where the sound is flat the contrast is zero, and noise
    // added to zero still crosses the threshold wherever it happens to peak.
    // That is the exact defect this layer exists to remove, reintroduced one
    // line lower down, and it is what
    // `clouds_are_cut_from_the_sound_and_not_from_a_field_of_their_own`
    // caught. A factor cannot do it: no field, no fringe, and near the contour
    // it still displaces the outline by a share of the threshold.
    let fringe = 1.0 + 1.1 * cloud.cloud_fringe
        * cloud_fbm(q * 2.6 + vec2<f32>(5.7, 12.9), 4, 0.55);

    // The two washes. The coarse one is the cloud-scale concentration, its
    // surround gathered a cloud and a half out; the fine one is the analyser's
    // own two blur scales against each other, which costs nothing because both
    // are already sampled.
    let close = cloud_close(uv);
    let wide = cloud_wide(uv);
    let around = cloud_surround(uv, unit * 1.5);
    let coarse = cloud_contrast(wide, around) * fringe;
    let fine = cloud_contrast(close, wide) * fringe;

    // Which way the field falls, for the tide line's pooling. One extra tap of
    // the wide blur rather than a screen-space derivative: `scale_clouds`
    // returns early above, so a derivative here would sit in non-uniform
    // control flow.
    let below = cloud_wide(uv + vec2<f32>(0.0, unit.y * 0.35));
    let fall = clamp((wide - below) * 8.0, -1.0, 1.0);

    // Cover opens the sky. It is the THRESHOLD both washes stand on, so the
    // knob's whole range runs from a clear pane to an overcast whose thin
    // places the picture still shows through.
    let sky = 0.62 - 0.52 * cloud.cloud_cover;
    let pool = 0.75;
    let coarse_wash = cloud_wash(coarse, sky, 0.05, 0.16, 3.4, fall, pool);
    let fine_wash = cloud_wash(fine, sky * 1.25, 0.035, 0.11, 4.0, fall, pool);

    // The paper. Fixed in PANE space, never in cloud space: it is the sheet
    // the wash dried on, and a substrate that drifted with the paint would be
    // one more moving texture over a static picture.
    let sheet = pt / cloud.size.y;
    let tooth = 0.5 + 0.5 * (0.55 * cloud_fbm(sheet * 170.0, 2, 0.5)
        + 0.45 * cloud_fbm(sheet * 48.0 + vec2<f32>(31.7, 3.3), 3, 0.5));
    let grain = 1.0 - cloud.cloud_grain + cloud.cloud_grain * 2.0 * tooth;

    // The pigment's colour is the sound under the wash, read where the wash
    // was displaced from, so a mass carries the colour of what it is made of
    // rather than of whatever it happens to be over.
    let tint = pow(clamp(wide * 1.3, 0.0, 1.0), 0.7);
    let pigment = palette_color(clamp(tint * 0.80 + 0.18, 0.0, 1.0));

    // Paint back to front. Each wash lays its body down and then its own tide
    // line on top of it, undiluted, because a tide line is where the water
    // left and the pigment stayed.
    var out = base;
    let depth = cloud.cloud_depth;
    let water = cloud.cloud_wash;
    let bodies = array<f32, 2>(coarse_wash.body * 0.88, fine_wash.body * 0.74);
    let tides = array<f32, 2>(coarse_wash.tide, fine_wash.tide);
    let waters = array<f32, 2>(water, water * 0.62);
    for (var i = 0; i < 2; i += 1) {
        let a = clamp(bodies[i] * grain, 0.0, 1.0) * depth;
        out = mix(out, cloud_dilute(pigment, waters[i]), a);
        let t = clamp(tides[i] * cloud.cloud_tide * grain, 0.0, 1.0) * depth;
        out = mix(out, pigment, t);
    }
    return out;
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
