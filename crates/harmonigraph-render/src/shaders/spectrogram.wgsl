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
    /// Which picture this draws: 0 the resampled heatmap, 1 the peak strokes
    /// over the dimmed cloud. A uniform and not a second pipeline, so the two
    /// share every buffer and switching costs nothing.
    detail: u32,
    /// Sigma of a stroke's Gaussian across pitch, in BUCKETS — the pane
    /// converts its cents through `bins_per_semitone`, so a stroke zooms with
    /// the pitch axis rather than holding a width in pixels.
    stroke_sigma: f32,
    /// How far above its own local mean a peak must stand to be drawn, in
    /// stored steps.
    prominence_steps: f32,
    /// `vec4`s one slot occupies in `peaks`: a header, then the entries.
    peak_stride: u32,
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
/// Each slot's peak list, `peak_stride` entries apart on the SAME slot mapping
/// as `grid` — written by the same scatter, out of the same bytes, so a slot's
/// peaks and its slab can never disagree. Slot `s` holds its count in
/// `peaks[s * peak_stride].x` and its entries from `s * peak_stride + 1`, each
/// `(x on the bucket axis, stored byte, local mean, 0)`.
@group(0) @binding(3) var<storage, read> peaks: array<vec4<f32>>;

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
fn bucket_level(slot: u32, b: u32) -> f32 {
    let midi = locals.spectrum_min_midi + (f32(b) + 0.5) / locals.bins_per_semitone;
    let v = f32(stored(slot, b));
    let level = locals.level0 + locals.level_per_step * v + locals.level_per_midi * midi;
    return clamp(level, 0.0, 1.0);
}

/// The level one fragment reads out of the slab in slot `slot`: an image
/// resample of [`bucket_level`] over the pitch this fragment covers.
///
/// The footprint is the fragment's own — exactly one pane pixel of the pitch
/// axis, so the footprints TILE it — and which of it and the bucket grid is
/// finer picks the arm.
///
/// MINIFYING (a pixel wider than a bucket) it is the AREA-WEIGHTED MEAN of the
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
/// MAGNIFYING (a pixel narrower than a bucket) the grid is being asked for
/// more than it holds, so it is read BETWEEN the two bucket centres this
/// fragment sits between. A bucket's centre is half a bucket above where the
/// floor divides them, which is the 0.5; the clamp keeps the upper tap inside
/// the spectrum.
fn read_level(slot: u32, t: f32) -> f32 {
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
            sum = sum + w * bucket_level(slot, b);
            total = total + w;
        }
        // A run of two or more whose overlap has been clamped to nothing — the
        // degenerate answered rather than one the picture arrives at.
        if total <= 0.0 {
            return bucket_level(slot, idx);
        }
        return sum / total;
    }
    let x = bucket_x(t) - 0.5;
    let b = u32(clamp(floor(x), 0.0, f32(locals.bins) - 2.0));
    let f = clamp(x - f32(b), 0.0, 1.0);
    return mix(bucket_level(slot, b), bucket_level(slot, b + 1u), f);
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
fn heatmap_level(in: VertexOut) -> f32 {
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

    return mix(read_level(s0, in.t), read_level(s1, in.t), fx);
}

/// The level one PEAK is drawn at, by the same affine a bucket takes in
/// [`bucket_level`] — level0 + level_per_step * byte + level_per_midi * midi,
/// clamped there and not after.
///
/// The midi is the peak's own centroid rather than its bucket's centre, which
/// is the whole reason the centroid is carried: a stroke is drawn where the
/// partial is, and the tilt is evaluated there too.
fn peak_level(peak: vec4<f32>) -> f32 {
    let midi = locals.spectrum_min_midi + peak.x / locals.bins_per_semitone;
    let level = locals.level0 + locals.level_per_step * peak.y + locals.level_per_midi * midi;
    return clamp(level, 0.0, 1.0);
}

/// The brightest stroke slab `slot` lays down at bucket position `x`.
///
/// A MAX over the slab's peaks, because two partials a stroke's width apart
/// are two lines that overlap, not one line twice as bright — the same reason
/// the slab fold takes a max over its columns.
fn slab_stroke(slot: u32, x: f32) -> f32 {
    let base = slot * locals.peak_stride;
    let count = u32(peaks[base].x);
    // Three sigma either side: past it the Gaussian is under 1.1%, which is
    // below one slice of a 4096-entry gradient at any level the ramp reaches.
    let reach = 3.0 * locals.stroke_sigma;
    let falloff = 1.0 / (2.0 * locals.stroke_sigma * locals.stroke_sigma);
    var best = 0.0;
    for (var i = 0u; i < count; i = i + 1u) {
        let peak = peaks[base + 1u + i];
        let d = x - peak.x;
        // The prominence gate is per PEAK, against the peak's own local mean:
        // a partial standing over a busy region is judged against that region
        // rather than against the column's average level.
        if abs(d) > reach || peak.y - peak.z < locals.prominence_steps {
            continue;
        }
        best = max(best, peak_level(peak) * exp(-d * d * falloff));
    }
    return best;
}

/// The stroke picture at this fragment: a MAX within each slab, a weighted
/// MEAN across the three slabs around it.
///
/// The two operators are not interchangeable and the asymmetry is the point.
/// Within a column the peaks are one measurement of one moment, so the
/// brightest line under the pixel is the reading. Across time they are
/// separate measurements, and a mean is what keeps a vibrato from aliasing
/// into a ragged edge at a coarse zoom — a max along time would draw the
/// envelope of the wobble instead of the wobble.
///
/// THREE taps and not five. The gather is the whole cost of this mode — up to
/// 48 peaks per tap per fragment — and five of them measured 4.1 to 4.6 ms a
/// frame over the heatmap at 1600x1300, where three measure 2.3 to 2.5 (see
/// `what_partials_detail_costs_a_frame`). What the outer pair bought was the
/// tail of the time smoothing, and a slab is already a fold over several
/// overlapping columns, so the mean across three of them is a wider window
/// than the count suggests.
///
/// Clamped into the run at both ends, the same `ClampToEdge` [`heatmap_level`]
/// takes past the newest slab, and off the same run-index-to-slot rule.
fn stroke_level(in: VertexOut) -> f32 {
    let n = i32(locals.run_slabs);
    // `in.slab` runs 0..n across the run, so the slab under this fragment is
    // its floor — slab j covering [j, j + 1) with its centre at j + 0.5.
    let jc = i32(clamp(floor(in.slab), 0.0, f32(n) - 1.0));
    let x = bucket_x(in.t);
    var sum = 0.0;
    var total = 0.0;
    for (var k = -1; k <= 1; k = k + 1) {
        // 1, 2, 1 as arithmetic: WGSL has no const array a loop variable may
        // index, and a three-tap triangle is one subtraction.
        let weight = 2.0 - abs(f32(k));
        let j = clamp(jc + k, 0, n - 1);
        let slot = (locals.first_slot + u32(j)) % locals.capacity;
        sum = sum + weight * slab_stroke(slot, x);
        total = total + weight;
    }
    return sum / total;
}

/// The level this fragment draws, whichever picture the pane is set to.
fn shown_level(in: VertexOut) -> f32 {
    if locals.detail == 1u {
        return stroke_level(in);
    }
    return heatmap_level(in);
}

fn heatmap_color(in: VertexOut) -> vec4<f32> {
    let level = shown_level(in);
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
    diffusion: f32,
    ppp: f32,
    /// How much of the diffused spectrum stands behind the Partials strokes,
    /// as a multiple of its own level. Heatmap detail never reads it.
    cloud: f32,
    /// Written rather than left to the layout rule, so the Rust struct beside
    /// it can be `Pod` and the two sizes are one number in both places.
    _pad0: f32,
    _pad1: f32,
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
    // Pitch already averages the reduced pixel's bucket footprint. Average
    // time too: a single center read can turn fine on/off slabs into a solid
    // bright or dark source depending on the scrolling phase. Four stratified
    // taps cover this quarter-resolution pixel at a fixed, bounded cost.
    let width = fwidth(in.slab);
    var level = 0.0;
    for (var i = 0u; i < 4u; i += 1u) {
        var tap = in;
        tap.slab += (f32(i) * 0.25 - 0.375) * width;
        level += heatmap_level(tap);
    }
    return vec4<f32>(level * 0.25, 0.0, 0.0, 1.0);
}
@fragment
fn fs_cloud_light(in: VertexOut) -> @location(0) vec4<f32> {
    // Combine the two smoothing scales at quarter resolution so the final
    // full-resolution pass needs only one filtered read per pixel.
    let uv = in.position.xy / vec2<f32>(textureDimensions(wide_light));
    let close = textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
    let wide = textureSampleLevel(wide_light, cloud_sampler, uv, 0.0).r;
    return vec4<f32>(close * 0.75 + wide * 0.25, 0.0, 0.0, 1.0);
}
fn baked_density(position: vec2<f32>) -> f32 {
    let uv = (position / cloud.ppp - cloud.origin) / cloud.size;
    // The source texture is reused for the finished scalar material only
    // after both filters have consumed it. No attachment samples itself.
    return textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r;
}
fn diffused_level(core: f32, material: f32) -> f32 {
    // Diffusion removes raw detail at every brightness. Its upper endpoint
    // is entirely filtered; restoring bright peaks here also restores grain.
    // Ease out the raw contribution so the default 70% leaves only 9% detail.
    let raw = (1.0 - cloud.diffusion) * (1.0 - cloud.diffusion);
    return mix(material, core, raw);
}
fn density_color(level: f32) -> vec4<f32> {
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
/// The soft field Partials draws BEHIND its strokes: the same diffused
/// spectrum the heatmap's filtered path composes, dimmed by the Cloud bar.
///
/// It goes through `diffused_level` rather than straight to the material so
/// the Diffusion bar keeps its meaning here — at 100% the cloud is the blurred
/// field alone, and at 0% it is the raw spectrum simply turned down, which is
/// the setting to reach for when the strokes should stand on a dark bed.
fn cloud_bed(core: f32, material: f32) -> f32 {
    return cloud.cloud * diffused_level(core, material);
}
/// Empty history uses the same field and palette with a zero measured core.
/// This quad never samples the grid, so the oldest column cannot be smeared —
/// and, in Partials, nothing strokes over it either.
fn backdrop_level(in: VertexOut) -> f32 {
    let material = baked_density(in.position.xy);
    if locals.detail == 1u {
        return cloud_bed(0.0, material);
    }
    return diffused_level(0.0, material);
}
@fragment
fn fs_cloud_backdrop_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return density_color(backdrop_level(in));
}
@fragment
fn fs_cloud_backdrop_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = density_color(backdrop_level(in));
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), 1.0);
}
fn cloud_color(in: VertexOut) -> vec4<f32> {
    let material = baked_density(in.position.xy);
    if locals.detail == 1u {
        // A max and not a sum: the cloud is what the strokes are seen
        // against, so a stroke over a lit region is drawn at its own level
        // rather than added to whatever is behind it.
        return density_color(max(stroke_level(in), cloud_bed(heatmap_level(in), material)));
    }
    return density_color(diffused_level(heatmap_level(in), material));
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
