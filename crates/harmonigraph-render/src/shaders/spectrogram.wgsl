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
    /// How far past the visible range the edge rows reach, in pitch fraction.
    margin: f32,
    /// MIDI of bucket 0's lower edge, and how many buckets one semitone holds.
    spectrum_min_midi: f32,
    bins_per_semitone: f32,
    /// The level mapping, affine in the stored byte and in MIDI: a row's own
    /// offset is `level0 + level_per_midi * midi`, and a stored step is worth
    /// `level_per_step` of the 0..1 the gradient is indexed by.
    level0: f32,
    level_per_step: f32,
    level_per_midi: f32,
    /// Stored steps the power mean falls per halving of the summed weight.
    mean_steps: f32,
    /// Rows the picture is read at along pitch. Sets both the row geometry and
    /// the pitch axis' own filtering, which is the whole of what makes the
    /// read resolution-dependent.
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
    _pad: u32,
};

@group(0) @binding(0) var<uniform> locals: Locals;
/// The grid, packed four stored bytes to a word. Slot `s` bucket `b` is byte
/// `s * stride + b`.
@group(0) @binding(1) var<storage, read> grid: array<u32>;
/// The weight a bucket carries in the power mean, indexed by how many stored
/// steps below its run's loudest it sits.
@group(0) @binding(2) var<storage, read> weight: array<f32, 256>;
/// The gradient sampled at `textureDimensions(lut).x` equal level slices,
/// opaque and in gamma space — the bytes `Color32` carries.
@group(0) @binding(3) var lut: texture_2d<f32>;

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

/// Nearest whole number, ties AWAY from zero — which WGSL's own `round` is
/// not: it takes ties to even. Both callers round a value the Rust read rounds
/// with `f32::round`, and both pass a value that is never negative.
///
/// `x - floor(x)` is exact at these magnitudes, so the comparison is the tie
/// rule itself rather than an approximation of it.
fn round_half_away(x: f32) -> f32 {
    let f = floor(x);
    return select(f, f + 1.0, x - f >= 0.5);
}

/// One stored byte out of the grid. The buffer is words, so a byte costs a
/// shift and a mask; a slab is padded to `stride` bytes and the padding is
/// never addressed.
fn stored(slot: u32, bucket: u32) -> u32 {
    let i = slot * locals.stride + bucket;
    return (grid[i >> 2u] >> ((i & 3u) * 8u)) & 0xffu;
}

/// The bucket a pitch fraction falls in, clamped into the spectrum.
fn bucket_of(t: f32) -> u32 {
    let midi = locals.min_midi + t * locals.span;
    let b = floor((midi - locals.spectrum_min_midi) * locals.bins_per_semitone);
    return u32(clamp(b, 0.0, f32(locals.bins) - 1.0));
}

/// One row of the picture: the slice of pitch fraction it covers, its centre,
/// and the MIDI there.
///
/// Rows tile `[-margin, 1 + margin]` rather than `[0, 1]`, so the edge rows
/// reach past the visible range by the width of a bucket and the filtering
/// carries cleanly to the picture's own edges.
struct Row {
    lo_t: f32,
    hi_t: f32,
    t: f32,
    midi: f32,
};

fn row_of(r: u32) -> Row {
    let m = locals.margin;
    let reach = 1.0 + 2.0 * m;
    let rows = f32(locals.rows);
    var row: Row;
    row.lo_t = -m + reach * f32(r) / rows;
    row.hi_t = -m + reach * f32(r + 1u) / rows;
    row.t = 0.5 * (row.lo_t + row.hi_t);
    row.midi = locals.min_midi + row.t * locals.span;
    return row;
}

/// The stored value row `row` reads out of the slab in slot `slot`, in the dB
/// the buckets hold — which is also the domain the ramp reads, so combining
/// buckets here combines exactly what will be drawn.
///
/// Which of the row and the bucket grid is finer picks the arm: a power mean
/// where the row is wider than what it reads, an interpolation where it is
/// narrower, since the grid is then being asked for more than it holds.
fn read_row(slot: u32, row: Row) -> f32 {
    let idx = bucket_of(row.lo_t);
    let last = bucket_of(row.hi_t);
    if last > idx {
        let to = min(last + 1u, locals.bins);
        var top = 0u;
        for (var b = idx; b < to; b = b + 1u) {
            top = max(top, stored(slot, b));
        }
        let n = to - idx;
        // One bucket IS its own mean; the arm is reached only from a run of
        // two or more, so this is the degenerate case answered rather than
        // one the picture arrives at.
        if n < 2u {
            return f32(top);
        }
        // Denominated against `top` (hence the subtraction, never negative),
        // so the sum runs from 1 up to the run's length and the answer can
        // only come DOWN from the loudest bucket. An absolute weight at this
        // order spans the stored range far enough to flush to zero in an f32.
        var sum = 0.0;
        for (var b = idx; b < to; b = b + 1u) {
            sum = sum + weight[top - stored(slot, b)];
        }
        let steps = -log2(sum / f32(n)) * locals.mean_steps;
        // The Rust read subtracts saturating, so a run long enough to fall
        // past the bottom lands on 0 rather than wrapping.
        return max(f32(top) - round_half_away(steps), 0.0);
    }
    // Narrower than a bucket: read between the two whose centres straddle this
    // row's centre. A bucket's centre sits half a bucket above where
    // `bucket_of` divides them, which is the 0.5; the clamp keeps the upper
    // tap inside the spectrum.
    let x = (row.midi - locals.spectrum_min_midi) * locals.bins_per_semitone - 0.5;
    let lo = u32(clamp(floor(x), 0.0, f32(locals.bins) - 2.0));
    let f = clamp(x - f32(lo), 0.0, 1.0);
    let a = f32(stored(slot, lo));
    let b = f32(stored(slot, lo + 1u));
    return round_half_away(a + (b - a) * f);
}

/// The colour one texel of the picture would have carried: the row's read,
/// mapped to a 0..1 level and looked up in the gradient.
///
/// The lookup TRUNCATES into the table, matching the level's own quantization
/// — the table is sampled at the centre of each slice, so the entry a level
/// falls into is the one nearest it.
fn shade(slot: u32, row: Row) -> vec4<f32> {
    let value = read_row(slot, row);
    let row0 = locals.level0 + locals.level_per_midi * row.midi;
    let level = clamp(row0 + locals.level_per_step * value, 0.0, 1.0);
    let levels = textureDimensions(lut).x;
    let i = min(u32(level * f32(levels)), levels - 1u);
    return textureLoad(lut, vec2<u32>(i, 0u), 0);
}

/// The heatmap's colour at this fragment: four texel reads blended in GAMMA
/// space, which is the filtering an `Rgba8Unorm` egui texture gets — the
/// sampler blends raw bytes and egui's shader encodes afterwards.
///
/// The clamp at both ends of each axis is the sampler's `ClampToEdge`: past
/// the newest slab and past the outermost row the picture holds its edge
/// rather than reading a slot the run does not own.
fn heatmap_color(in: VertexOut) -> vec4<f32> {
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

    // Row centres sit at `(r + 0.5) / rows` of the span between the first and
    // last row's own centres, which is what the picture's pitch axis was
    // stretched across.
    let t0c = row_of(0u).t;
    let tnc = row_of(locals.rows - 1u).t;
    let denom = tnc - t0c;
    let v = select((in.t - t0c) / denom, 0.0, denom == 0.0);
    let rows = f32(locals.rows);
    let y = v * rows - 0.5;
    let ry = clamp(floor(y), 0.0, rows - 1.0);
    let r0 = u32(ry);
    let r1 = min(r0 + 1u, locals.rows - 1u);
    let fy = clamp(y - ry, 0.0, 1.0);

    let low = row_of(r0);
    let high = row_of(r1);
    let c = mix(
        mix(shade(s0, low), shade(s1, low), fx),
        mix(shade(s0, high), shade(s1, high), fx),
        fy,
    );
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
