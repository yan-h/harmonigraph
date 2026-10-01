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
/// whatever covers it, and a floor below the window contributes level 0, so a
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
/// On the plain path, MINIFYING (a pixel wider than a bucket) is the AREA-WEIGHTED MEAN of the
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
// point; the plain path keeps its original display-level area mean exactly.
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
    // How far the levels are gathered into terraces, 0 for none. This word was
    // a style enum: Plain, Blur and Lava are now the blur, the terraces and the
    // cloud each at zero or not, read off their own dials.
    contour_strength: f32,
    // 1 when `cloud_tone` holds a precomposite: a reduced scalar field for
    // clouds, or RGB of the three far Stars layers (75%, 50% or a third of the
    // pane at High, Medium and Low; native at Uniform on a large pane). 0 works
    // the texture out per pixel in the composite instead.
    tone_baked: u32,
    // Watercolour clouds. `drift` is the wash's offset in cloud units; the rest
    // are the sanitized settings. The filter shader declares only the head of
    // this struct, which is why these are appended rather than interleaved.
    drift: vec2<f32>,
    cloud_depth: f32,
    scale_size: f32,
    scale_variety: f32,
    scale_refract: f32,
    // Which texture the layer draws: 0 the refracting scales above (Mosaic), 1
    // the watercolour wash below, 2 the starfield after them, 3 Scales. Nothing is shared
    // between them but the blurred light, the palette, the clock and
    // `cloud_depth`.
    cloud_style: u32,
    wash_size: f32,
    wash_fuzz: f32,
    wash_lobe: f32,
    wash_refract: f32,
    wash_layers: f32,
    // The tile's period in cells, above zero whenever a mosaic or wash is
    // drawn. The cell a hash is taken at is folded onto the square period
    // described beside `wrap_cell`, `fs_cloud_tile` bakes one period of it, and
    // the two paths below read that texture instead of walking the ring per
    // pixel. The
    // wash rotates that read; the mosaic keeps the square tile's original axes.
    tile_cells: u32,
    // 1 when pitch is the pane's Y axis, 0 when it is X. The wash's 3-4-5
    // rotation is defined in (time, pitch), so its basis follows this
    // orientation. The unrotated mosaic does not read it.
    pitch_vertical: u32,
    // The starfield's `Brightness variation`, and its life clock in lives, already
    // reduced by `STAR_LIFE_PERIOD` (`star_life` in atmosphere.rs). Read by
    // none of the textures above.
    star_randomness: f32,
    star_life: f32,
    // The starfield's `Size variation`, then the rest of its row.
    star_size_variation: f32,
    star_pad0: u32,
    star_pad1: u32,
    star_pad2: u32,
    star_far: vec4<f32>,
    star_near: vec4<f32>,
    // Jitter width, compact-core reach, fade-start fraction and far fill, computed once
    // per frame by star_geometry in atmosphere.rs. The first two lengths are in cells.
    star_geometry: vec4<f32>,
    // One entry per depth, worked out on the CPU from the dials and the clock
    // (`star_slices` in atmosphere.rs, which says what each field is).
    star_slices: array<StarSlice, 5>,
    memory_enabled: u32,
    memory_valid: u32,
    pickup_alpha: f32,
    release_alpha: f32,
    memory_shift: vec2<i32>,
    memory_fraction: vec2<f32>,
    previous_life: f32,
    wash_randomness: f32,
    memory_extent: vec2<f32>,
    previous_slices: array<StarSlice, 5>,
    star_halo_samples: array<StarHaloSample, 5>,
    velvet: vec4<f32>,
    velvet_size: vec4<f32>,
    // The wash's `Edge pooling` (signed), its width in front-glob radii, and
    // the exponent its `Softness` makes of the fade; w is padding.
    wash_pigment: vec4<f32>,
};
@group(1) @binding(9) var color_memory: texture_2d<f32>;
@group(1) @binding(0) var close_light: texture_2d<f32>;
@group(1) @binding(1) var wide_light: texture_2d<f32>;
@group(1) @binding(2) var cloud_sampler: sampler;
@group(1) @binding(3) var<uniform> cloud: Cloud;
/// The cloud's scalar tone, one texel per cloud sample of pane, as `fs_cloud_tone`
/// drew it. Bound whether or not it holds anything — a pass that RENDERS into it
/// binds a stand-in here, since wgpu validates every resource in a bound group
/// against the attachments whether the shader reads it or not.
@group(1) @binding(4) var cloud_tone: texture_2d<f32>;
/// One period of the cell walk's OUTPUT, as `fs_cloud_tile` baked it, and bound
/// on the same terms as `cloud_tone` above — the pass that renders into these
/// two binds a stand-in here. The mosaic uses only the first (its `face` and
/// `to_centre`); the wash fills both (see `WashField`).
@group(1) @binding(5) var cloud_tile_a: texture_2d<f32>;
@group(1) @binding(6) var cloud_tile_b: texture_2d<f32>;
/// The wash's distances out from the front glob's arc, coarse then fine, read
/// only while `Edge pooling` is not zero.
@group(1) @binding(13) var cloud_tile_c: texture_2d<f32>;
/// The tile's own sampler, and the only REPEATING one here: the mosaic divides
/// its cell coordinate by the period, while the wash first turns it into the
/// rotated basis; either texture coordinate wraps. `cloud_sampler` clamps,
/// which every other read wants —
/// a refracted lookup that ran off the pane must hold its edge rather than
/// return the light from the far side of the picture.
@group(1) @binding(7) var tile_sampler: sampler;
/// Every star on screen this frame, one texel per cell, as `fs_star_bake` drew
/// it; bound on the same terms as `cloud_tone`. See `star_texel` for the
/// packing.
@group(1) @binding(8) var star_atlas: texture_2d<u32>;
@group(1) @binding(10) var star_halos: texture_2d_array<f32>;
@group(1) @binding(11) var star_halos_b: texture_2d_array<f32>;
@group(1) @binding(12) var star_halos_c: texture_2d_array<f32>;

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
    var integral = 0.0;
    let last_center = f32(locals.run_slabs) - 0.5;
    // The field is affine BETWEEN those centers, so a segment's exact integral
    // is its width times the level at its midpoint: one `field_level` per
    // segment, where a trapezoid rule reads both ends and so reads every
    // interior kink twice over. The same number, off only in the last
    // bit, for one read per segment fewer — and at full zoom-out a source
    // texel spans well under a slab, so a footprint is one or two segments and
    // that read is a third to a half of the whole integration.
    // Outside the run the read holds its edge; skip that constant interval in
    // one step. Inside, each original slab contributes to this footprint.
    for (var i = 0u; i < locals.run_slabs + 2u; i += 1u) {
        if at >= high { break; }
        var next = min(high, max(0.5, floor(at - 0.5) + 1.5));
        if at >= last_center { next = high; }
        tap.slab = (at + next) * 0.5;
        integral += field_level(tap, true) * (next - at);
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
// Whether the picture is the blurred field. At zero softness on both axes it is
// the measured core instead — the light field may still have been built, for a
// cloud to read, but it is then a copy of the core at the field's resolution
// and the core itself is the sharper of the two.
fn softened() -> bool {
    return any(cloud.step != vec2<f32>(0.0));
}
// Local style transfer. No history, upload, smoothing or palette work is
// duplicated when adding a display style here. A residual slope preserves
// quiet fields below the first terrace; the zero input remains exactly zero.
//
// `Contour strength` scales the blend and nothing else, so 100% is what the
// Lava style drew and 0 is the level untouched — behind the knob, because the
// `fwidth` pair and the smoothsteps are per-pixel work for a blend of nothing.
fn style_level(level: f32) -> f32 {
    if cloud.contour_strength <= 0.0 { return level; }
    let x = clamp(level, 0.0, 1.0) * cloud.contours;
    let edge = min(0.5, max(cloud.contour_softness, fwidth(x) * 0.5));
    let terraces = (floor(x) + smoothstep(0.5 - edge, 0.5 + edge, fract(x))) / cloud.contours;
    let strength = 0.9 * cloud.contour_strength * smoothstep(0.0, 1.0, x)
        * (1.0 - smoothstep(0.5, 1.5, fwidth(x)));
    return mix(level, terraces, strength);
}
// Interpolate the authored palette's center samples only after diffusion.
//
// **Level 0 is the gradient's own floor, not black.** The bottom of the range
// is what the scheme SAYS silence looks like — `Gradient`'s low end is silence
// here the way its low end is the darkest pitch on the lattice — so a ramp
// that does not start at black draws a quiet pane in its own colour. This
// slice used to fade to true black instead, which put a colour under the
// picture that the scheme never named and that no dial could reach: an
// isoluminant gradient is a documented setting, and at one the whole point is
// that the bottom of the range reads as bright as the top.
//
// Nothing is lost for a scheme that does want black down there, because how
// dark the bottom sits is already a gradient knob and only a gradient knob —
// `Lightness` with `Lightness ramp` place both ends on the `L*` axis, and the
// fresh Aurora spends the whole of it, so its floor is `L*` 0 and its quiet
// pane stays byte-for-byte black. Wanting it back is `Lightness ramp` up, not
// a tenth dial that would say a second time what those two already say.
//
// Below the first sample's centre the table is therefore FLAT, which is the
// answer the TOP has always given: the last entry pairs with itself at a lerp
// weight of 0, and this is that same rule at the other end.
fn palette_color(level: f32) -> vec3<f32> {
    let levels = textureDimensions(lut).x;
    let x = max(clamp(level, 0.0, 1.0) * f32(levels) - 0.5, 0.0);
    let i = u32(clamp(floor(x), 0.0, f32(levels - 1u)));
    let a = textureLoad(lut, vec2<u32>(i, 0u), 0).rgb;
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
// The geometry now displaces levels only; Contours and the palette are
// shared with the source picture. No material exposure or shading remains.
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
// one over its radius, so a raw slope would make `Cell size` silently a second
// refraction knob — halve the scale and the picture bends twice as far.
// `DOME_FACE` takes that out, and takes out the same effect WITHIN one field now
// that `Size variation` gives each glob its own radius.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI, and it is
// FIXED: ten cloud units across the pane's height, which is what the shipped
// `Cloud size` of 0.5x drew before the dial was retired.
//
// It was a dial, and it was a second copy of `Cell size` and `Patch size`. The
// texture's size on the pane came out as a PRODUCT — `cloud_scale * scale_size`
// reached the dome grid and nothing read either on its own — so the two dials
// named one number between them, and the picture could not tell which of them
// had set it. What `Cloud size` did own was the drift, since `drift` is measured
// in cloud units and a larger unit carries the texture further per second; but
// that is `Drift speed` again, one multiplication later. Three dials, two
// observables. Pinning the frame here leaves each texture one size dial that
// means its own size, and leaves the drift to the dial named after it.
//
// These qualities are on DIALS rather than decided here, because describing
// which of them Yan wants has failed in words repeatedly: the NEGATIVE half of
// `Refraction` carries the lookup from this round's continuous slope onto round
// 1's flat per-glob patch, and `Size variation` is how much the globs differ in
// size.
const CLOUD_UNITS: f32 = 10.0;
// The tiled WASH is turned by the exact 3-4-5 rotation: cosine 4/5, sine 3/5,
// or 36.87 degrees. Its square walk is baked in its own coordinates and this
// rotation is applied only when it is read, so every octave still closes on
// the ordinary square period and no resampling stretch is introduced. Mosaic
// deliberately keeps the square tile's original axes: turning its scale pile
// changed the look rather than merely hiding its repetition.

// How many dome cells cross one cloud unit at `Cell size` 1x. Carries the
// retired `Cloud size` default: the shipped picture was 6 cells per unit over a
// frame half this one's, and `6 / 2.2` at the old `Cell size` default is what
// puts the same scales on the pane with the dial reading a plain 1x.
const SCALE_CELLS: f32 = 6.0 / 2.2;

// Watercolor texture coordinates in the rotated basis. `semantic` is `(time, pitch)`
// whichever way the pane is oriented; multiplying by R^-1 turns the world
// point back into the square tile's coordinates. The repeating sampler then
// makes its two screen-space repeat vectors `(4P/5, 3P/5)` and
// `(-3P/5, 4P/5)` in `(time, pitch)`.
fn watercolor_tile_uv(r: vec2<f32>) -> vec2<f32> {
    return watercolor_tile_uv_for(r, f32(cloud.tile_cells), cloud.pitch_vertical);
}

// The baked channels carry directions as well as scalars. Sampling them at a
// rotated coordinate turns the geometry only if its vectors turn with it;
// otherwise refraction would still point along the unrotated
// field. Convert to `(time, pitch)`, apply R, and return to pane axes.
fn rotate_watercolor_tile_vector(v: vec2<f32>) -> vec2<f32> {
    return rotate_watercolor_tile_vector_for(v, cloud.pitch_vertical);
}

fn cloud_domes(r: vec2<f32>, period: i32) -> Pile {
    return mosaic_field(r, period, cloud.scale_variety);
}

// `close_light` already holds the decoded, Spread-combined scalar material.
// Both textures read the SAME Spread-combined scalar picture. Geometry only
// selects the lookup: no gain, lighting, paper, pigment or independent wide tap.
fn cloud_light(pt: vec2<f32>) -> f32 {
    return textureSampleLevel(close_light, cloud_sampler, pt / cloud.size, 0.0).r;
}

// The mosaic's displaced level, before Contours and the palette.
fn scale_tone(pt: vec2<f32>) -> f32 {
    let q = (pt - cloud.size * 0.5) / cloud.size.y * CLOUD_UNITS + cloud.drift;

    // The scales. `scale_size` DIVIDES how many of them cross one cloud unit,
    // so the knob reads as a size rather than as a frequency.
    let scale_units = SCALE_CELLS / cloud.scale_size;
    let scale_points = cloud.size.y / CLOUD_UNITS / scale_units;
    let r = q * scale_units;
    // One tap into the period of the ring `fs_cloud_tile` already walked. The
    // whole of the walk's output is the two vectors below, so the tile is one
    // `Rgba16Float` read and the rest of this function — the refraction — is
    // what runs per pixel. There is deliberately no live-walk arm here: even
    // never taken, it cost this full-resolution shader 16 to 21% (#1100).
    let tile = textureSampleLevel(cloud_tile_a, tile_sampler, r / f32(cloud.tile_cells), 0.0);
    var pile: Pile;
    pile.face = tile.xy;
    pile.to_centre = tile.zw;

    // THE REFRACTION. `DOME_FACE` has already put the offset in scale widths
    // whatever the scale size is, and in each glob's OWN width whatever
    // `Size variation` has made of it.
    let face = pile.face;
    let bend = max(cloud.scale_refract, 0.0) * scale_points;
    // The dial's NEGATIVE half swings the reading off the face the scale
    // PRESENTS and onto the scale's own CENTRE, which is round 1's reading: one
    // value for the whole scale, so the picture comes apart into flat quantized
    // patches instead of bending through them. `to_centre` is in cells and
    // `scale_points` is how many pane points a cell is, so at -1
    // `pile.to_centre * scale_points` lands exactly on the dome's centre — the
    // same arithmetic round 1 spelled out as `centre_pt` — and between 0 and -1
    // the reading is pulled a share of the way there, which is what the wash
    // below has always called its own refraction.
    //
    // One signed dial where there were two, `Refraction` and a `Facet` that
    // blended between the two readings. They were kept apart so one could be
    // dialled down to look at the other while the layer was being built, and
    // together they spanned a plane of which the blends in the middle — part
    // face, part centre — were neither look. Only one side is ever nonzero, so
    // the other term adds an exact zero and the positive half draws what
    // `Refraction` alone always drew, bit for bit.
    let gather = max(-cloud.scale_refract, 0.0) * scale_points;
    let lookup = -face * bend + pile.to_centre * gather;
    return cloud_light(pt + lookup);
}

// The second texture, beside the scales above and sharing nothing with them but
// the blurred light, the palette and the drift clock. It is what Yan asked for
// first, on a sheet of watercolour cumulus: *"this watercolor clouds example is
// roughly what I want - with additional movement and refraction, and different
// colors of course"*, *"I want the entire field to look like a field of
// different sized cloud globs, with some variation"*, *"I want the texture, not
// the exact shape of how clouds behave in real life"*. So again one continuous
// isotropic field with no sky, no up and no gaps — but a WATERCOLOUR one, where
// the shape comes from overlapping globs rather than a soft union of domes.
//
// Prototyped in numpy over a real recording across two contact sheets; Yan's
// pick was *"I like J1, J2 and J5 the most"*, which are one construction at
// three settings, so the settings are the dials below and the construction is
// this. The prototype's own negative results are why several obvious things are
// NOT here: a z-buffer of spheres instead of a paint order (cracked mud), an
// even outline on each glob (a contour map), transparency where the paper is
// lightest (the raw stripes show through as a screen door), any additive
// highlight (wet plastic), a separate paint quantizer (cel shading), and elongated globs
// (rice grains).
//
// **Paint order, not depth.** Every cell hashes a centre, a radius and a PAINT
// ORDER. The glob a pixel shows is the highest order among those covering it, so
// every boundary in the picture is one glob's own arc — a curve — and never the
// bisector between two, which is the straight crossing that made the z-buffered
// version read as cracked mud.
//
// Each glob reads the displaced scalar level, and pigment is the one tone
// adjustment: off by default, and only ever subtracted, so it can darken a glob
// but never lift one. `Fine layer mix` mixes the levels before the shared Contours and
// palette transfer.
//
// **Feather is the fuzziness.** A visible glob dissolves at its OWN rim into
// whatever lies beneath it, reaching half and half exactly on the boundary so
// both sides meet. That is the antialiasing as well: there is no supersampling
// here and the prototype's final renders had none either, precisely so they
// showed what a fragment shader would really draw.

// At 50% `Edge pooling` is the strength the first Watercolor shipped with (#909).
const WASH_POOL: f32 = 0.44;
// Pigment bites in proportion to the level under it, plus this much: one that
// took the same bite out of a dark tone as out of a light one turns every
// crevice black, which reads as mortar between stones rather than paint.
const WASH_PIG_DEPTH: f32 = 0.35;
// A negative `Edge pooling` lifts by `level * (1 - level)` times this, which
// matches the darkening's bite at mid level (`0.35 + 0.65 * 0.5 = 2.7 / 4`)
// and is zero at both ends: silence stays black and nothing is pushed past 1
// until the dial is well past 100%.
const WASH_BLOOM: f32 = 2.7;

fn wash_pigmented() -> bool {
    return cloud.wash_pigment.x != 0.0;
}

fn wash_level(wet: Wet, pane_per_cell: f32, pt: vec2<f32>) -> f32 {
    let level = cloud_light(pt + wet.offset * pane_per_cell * cloud.wash_refract);
    if !wash_pigmented() {
        return level;
    }
    // The tide line: full against the front glob's arc, gone `Width` radii
    // out, and the fade between them raised to the `Softness` exponent — a
    // flat hard-edged band at one end, a long soft tail at the other. The
    // fresh 0.55 and 2 are #909's crescent exactly.
    let shape = pow(clamp(1.0 - wet.gap / cloud.wash_pigment.y, 0.0, 1.0), cloud.wash_pigment.z);
    // The tide line has to fade as the edge dissolves: a crisp dark crescent
    // on a boundary that is no longer there reads as a line floating in fog.
    let fuzz = cloud.wash_fuzz;
    let pigment = WASH_POOL * cloud.wash_pigment.x * (1.0 - 0.75 * fuzz) * shape;
    if pigment < 0.0 {
        return min(level - pigment * WASH_BLOOM * level * (1.0 - level), 1.0);
    }
    // Over silence the bite is negative and clamps away: the floor stays the
    // palette's floor with no black point needed.
    return max(level - pigment * (WASH_PIG_DEPTH + (1.0 - WASH_PIG_DEPTH) * level), 0.0);
}

// The same field out of the baked tile: the cell coordinate is turned into the
// wash's rotated basis and divided by the period. That repeating coordinate is
// the whole of what the drift does here — it slides a fixed field rather than
// changing one.
fn wash_tile_field(r: vec2<f32>) -> WashField {
    let uv = watercolor_tile_uv(r);
    let a = textureSampleLevel(cloud_tile_a, tile_sampler, uv, 0.0);
    var out: WashField;
    var c = vec4<f32>(0.0);
    if wash_pigmented() {
        c = textureSampleLevel(cloud_tile_c, tile_sampler, uv, 0.0);
    }
    out.coarse = Wet(rotate_watercolor_tile_vector(a.xy), a.z, c.x);
    out.fine = Wet(vec2<f32>(0.0), 0.0, 0.0);
    out.cover = 0.0;
    if cloud.wash_layers > 0.0 {
        let b = textureSampleLevel(cloud_tile_b, tile_sampler, uv, 0.0);
        out.fine = Wet(rotate_watercolor_tile_vector(b.xy), b.z, c.y);
        out.cover = b.w;
    }
    return out;
}

// Shared coordinates keep brightness and displacement on the same globs.
fn wash_cell_at(pt: vec2<f32>) -> vec2<f32> {
    let q = (pt - cloud.size * 0.5) / cloud.size.y * CLOUD_UNITS + cloud.drift;
    return q * (WASH_CELLS / cloud.wash_size);
}

fn wash_cloud_tone(pt: vec2<f32>) -> f32 {
    // `wash_size` is how big one GLOB is, so the knob reads as a size.
    //
    // The number is set by the picture and not by the cell count, and the two
    // are not the same: a glob here was calibrated 1.18 cells wide where the
    // prototype's was `rad` 0.90, so matching J2's forty cells up the pane would
    // draw its globs 31% too big — measured against `j2.png`, a visibly blobbier
    // field with two or three fewer harmonic lines showing through it. Matching
    // the DIAMETER instead puts 40 * 1.18 / 0.90 ≈ 52 cells up the pane at the
    // fresh cloud size, which is `WASH_CELLS` per cloud unit.
    let cells = WASH_CELLS / cloud.wash_size;
    let pane_per_cell = cloud.size.y / CLOUD_UNITS / cells;
    let r = wash_cell_at(pt);

    // One or two taps into the period of the two ring walks `fs_cloud_tile`
    // already walked; no live arm, for the reason `scale_tone` gives.
    let field = wash_tile_field(r);
    var level = wash_level(field.coarse, pane_per_cell, pt);
    if cloud.wash_layers > 0.0 {
        let fine_level = wash_level(field.fine, pane_per_cell / WASH_LACUNARITY, pt);
        let over = cloud.wash_layers * field.cover;
        level = mix(level, fine_level, over);
    }

    return level;
}

// Whichever texture is selected, as one scalar. The branch is on a uniform, so
// no two lanes ever disagree about it.
fn cloud_tone_at(pt: vec2<f32>) -> f32 {
    if cloud.cloud_style == 3u {
        return textureSampleLevel(cloud_tone, cloud_sampler, pt / cloud.size, 0.0).r;
    }
    if cloud.cloud_style == 1u {
        return wash_cloud_tone(pt);
    }
    return scale_tone(pt);
}

// The cloud's tone reduced to a target of its own, one texel per cloud sample
// (a fixed 0.5 pt) of pane. The coverage quad carries the pane-relative 0..1 fraction in
// its `slab`/`t`, which is what makes this the same `pt` the composite would
// have read under each of its own pixels.
@fragment
fn fs_cloud_tone(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(cloud_tone_at(vec2<f32>(in.slab, in.t) * cloud.size), 0.0, 0.0, 1.0);
}

@fragment
fn fs_velvet_tone(in: VertexOut) -> @location(0) vec4<f32> {
    let cell = cloud.size.y * (24.0 / 405.0) * cloud.velvet_size.x;
    let drift = (cloud.drift - vec2<f32>(0.0, 0.6)) * cloud.size.y / (10.0 * cell);
    let level = velvet_material(close_light, cloud_sampler, vec2<f32>(in.slab, in.t) * cloud.size,
        cloud.size, cell, drift, cloud.velvet).r;
    return vec4<f32>(level, 0.0, 0.0, 1.0);
}

// ====================== ONE PERIOD OF THE CELL WALK ========================
//
// The tile, baked whenever a cloud is drawn and read by both paths above. It has
// no pane, no drift and no light in it: it is one square period of whichever
// walk the style selects. The Watercolor read turns that whole field by 36.87
// degrees; the Mosaic read leaves it square. That is why a resize, a drift or
// a note never touches it and `Cell size` reaches it only
// through how many texels the renderer spends on a cell.
struct TileVertex {
    @builtin(position) position: vec4<f32>,
    // Where this texel sits in the tile, 0 at one corner and 1 at the other.
    // The PERIOD is applied in the fragment stage rather than here, because the
    // cloud uniform is bound to the fragment stage alone and a vertex read of
    // it would widen every cloud pipeline's layout for this one entry point.
    @location(0) fraction: vec2<f32>,
    @location(1) @interpolate(flat) layer: u32,
};
@vertex
fn vs_cloud_tile(@builtin(vertex_index) vertex: u32, @builtin(instance_index) layer: u32) -> TileVertex {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var out: TileVertex;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.fraction = uv;
    out.layer = layer;
    return out;
}

struct TileBake {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) c: vec4<f32>,
};
@fragment
fn fs_cloud_tile(in: TileVertex) -> TileBake {
    let period = i32(cloud.tile_cells);
    // The wash's texel centre lands in the square semantic coordinates that
    // the inverse `watercolor_tile_uv` rotation reads back. The mosaic instead
    // uses the original physical X/Y square so its bake matches `r / period`.
    let period_f = f32(cloud.tile_cells);
    let time = in.fraction.x * period_f;
    let pitch = in.fraction.y * period_f;
    let wash_cell =
        select(vec2<f32>(pitch, time), vec2<f32>(time, pitch), cloud.pitch_vertical == 1u);
    let mosaic_cell = in.fraction * period_f;
    var out: TileBake;
    out.a = vec4<f32>(0.0);
    out.b = vec4<f32>(0.0);
    out.c = vec4<f32>(0.0);
    if cloud.cloud_style == 1u {
        // Seven channels of glob geometry and brightness and two of distance
        // out from the front glob's arc, the fine octave whatever `Fine layer mix` says, so turning that
        // dial up is a mix and never a rebake.
        let field = wash_field(wash_cell, period, cloud.wash_fuzz, cloud.wash_lobe);
        out.a = vec4<f32>(field.coarse.offset, field.coarse.brightness, 0.0);
        out.b = vec4<f32>(field.fine.offset, field.fine.brightness, field.cover);
        out.c = vec4<f32>(field.coarse.gap, field.fine.gap, 0.0, 0.0);
    } else {
        // The mosaic's whole walk is these two vectors, so its second target is
        // never read. It is still allocated and still written, which is what
        // keeps a change of style a rebake rather than a reallocation.
        let pile = cloud_domes(mosaic_cell, period);
        out.a = vec4<f32>(pile.face, pile.to_centre);
    }
    return out;
}

fn gamma_from_linear_rgb(linear: vec3<f32>) -> vec3<f32> {
    let bounded = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(bounded, vec3<f32>(1.0 / 2.4)) - 0.055, 12.92 * bounded, bounded <= vec3<f32>(0.0031308));
}


// RGB and scalar source level share one response coefficient. Releasing toward
// silence interpolates recent RGB toward the palette floor, never along the ramp.
fn remembered(current: vec4<f32>, previous: vec4<f32>) -> vec4<f32> {
    let alpha = select(cloud.release_alpha, cloud.pickup_alpha, current.a > previous.a);
    return mix(previous, current, alpha);
}

fn star_memory(k: u32, cell: vec2<i32>) -> vec4<f32> {
    let s = cloud.star_slices[k];
    let salt = 1000u + 3u * k;
    let hashed = cell & vec2<i32>(STAR_HASH_PERIOD - 1);
    let stagger = star_hash(hashed, salt + 2u).x;
    let life = u32(floor(cloud.star_life + stagger)) & (STAR_LIFE_PERIOD - 1u);
    let key = salt + ((life + 1u) << 16u);
    let a = star_hash(hashed, key);
    let centre = 0.5 + cloud.star_geometry.x * (a.xy - 0.5);
    let at = (vec2<f32>(cell) + centre + s.offset) * s.cell * (cloud.size.y / STAR_PANE) + cloud.size * 0.5;
    let level = star_level_at(at);
    let rank_draw = star_hash(hashed, key + 1u).x;
    let rank = pow(rank_draw, 1.0 + 6.0 * cloud.star_randomness) * (2.0 + 6.0 * cloud.star_randomness);
    let current = vec4<f32>(linear_from_gamma_rgb(star_paint(level, rank)), level);
    if cloud.memory_valid == 0u { return current; }
    let old_life = u32(floor(cloud.previous_life + stagger)) & (STAR_LIFE_PERIOD - 1u);
    let previous = cloud.previous_slices[k];
    // Signed nearest periodic cell difference carries identity across drift's wrap.
    let local = ((cell - previous.origin + STAR_HASH_PERIOD / 2) & vec2<i32>(STAR_HASH_PERIOD - 1)) - STAR_HASH_PERIOD / 2;
    if old_life != life || any(local < vec2<i32>(0)) || any(local >= previous.grid) { return current; }
    let index = previous.base + local.y * previous.grid.x + local.x;
    return remembered(current, textureLoad(color_memory, atlas_texel(index), 0));
}

@fragment
fn fs_color_memory(in: TileVertex) -> @location(0) vec4<f32> {
    let texel = vec2<i32>(floor(in.position.xy));
    if cloud.cloud_style == 2u {
        let index = texel.y * STAR_ATLAS_WIDTH + texel.x;
        for (var k = 0u; k < STAR_SLICES; k += 1u) {
            let s = cloud.star_slices[k];
            let at = index - s.base;
            if at >= 0 && at < s.grid.x * s.grid.y {
                return star_memory(k, s.origin + vec2<i32>(at % s.grid.x, at / s.grid.x));
            }
        }
        return vec4<f32>(0.0);
    }
    // The lattice is fixed in material coordinates. Only integer textureLoad
    // copies enter feedback; drift never repeatedly bilinear-blurs held color.
    let dimensions = vec2<i32>(cloud.memory_extent);
    let grid = vec2<f32>(dimensions - 2);
    let pt = (vec2<f32>(texel) + 0.5 - 1.0 - cloud.memory_fraction) / grid * cloud.size;
    let level = cloud_tone_at(pt);
    let current = vec4<f32>(linear_from_gamma_rgb(density_color(level).rgb), level);
    let previous = texel + cloud.memory_shift;
    if cloud.memory_valid == 0u || any(previous < vec2<i32>(0)) || any(previous >= dimensions) { return current; }
    return remembered(current, textureLoad(color_memory, previous, 0));
}

// RGBA32F history requires no optional float-filtering device feature. Only
// the final display resamples it; feedback always copies exact lattice texels.
fn memory_color(uv: vec2<f32>) -> vec3<f32> {
    let size = vec2<i32>(cloud.memory_extent);
    let p = uv * vec2<f32>(size) - 0.5;
    let lo = vec2<i32>(floor(p));
    let f = fract(p);
    let a = textureLoad(color_memory, clamp(lo, vec2<i32>(0), size - 1), 0).rgb;
    let b = textureLoad(color_memory, clamp(lo + vec2<i32>(1, 0), vec2<i32>(0), size - 1), 0).rgb;
    let c = textureLoad(color_memory, clamp(lo + vec2<i32>(0, 1), vec2<i32>(0), size - 1), 0).rgb;
    let d = textureLoad(color_memory, clamp(lo + vec2<i32>(1, 1), vec2<i32>(0), size - 1), 0).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn clouded_base(level: f32, position: vec2<f32>) -> vec4<f32> {
    // There is no gate on the blur here, and there used to be: the light field
    // was only built when a softness was above zero, so the cloud quietly
    // vanished with the blur. The field is built whenever a cloud is drawn now,
    // and at zero softness it holds the measured picture unblurred.
    if cloud.cloud_depth <= 0.0 {
        return density_color(level);
    }
    let pt = position / cloud.ppp - cloud.origin;
    // The starfield is colour, not a level: `Texture mix` blends the plain
    // picture toward it rather than feeding the palette a mixed level.
    if cloud.cloud_style == 2u {
        return vec4<f32>(mix(density_color(level).rgb, star_color(pt).rgb, cloud.cloud_depth), 1.0);
    }
    if cloud.memory_enabled != 0u {
        let dimensions = cloud.memory_extent;
        let uv = (pt / cloud.size * (dimensions - 2.0) + 1.0 + cloud.memory_fraction) / dimensions;
        let held = memory_color(uv);
        if cloud.cloud_depth >= 1.0 {
            return vec4<f32>(gamma_from_linear_rgb(held), 1.0);
        }
        // Memory is RGB, so its partial Texture mix is a bounded linear-light
        // blend. Both response times zero retain the original scalar mix below.
        let base = linear_from_gamma_rgb(density_color(level).rgb);
        return vec4<f32>(gamma_from_linear_rgb(mix(base, held, cloud.cloud_depth)), 1.0);
    }
    // Either the tone worked out under this pixel (a tile tap and its
    // refraction), or one bilinear tap into what `fs_cloud_tone` already worked
    // out. The palette lookup and the mix stay HERE whichever it was, so the
    // base picture, its terraces and the gradient are full resolution even
    // where the texture over them is not.
    var tone: f32;
    if cloud.tone_baked == 1u {
        tone = textureSampleLevel(cloud_tone, cloud_sampler, pt / cloud.size, 0.0).r;
    } else {
        tone = cloud_tone_at(pt);
    }
    // Mix levels before the one shared style/palette lookup: `Texture mix` and
    // Watercolor's `Fine layer mix` cannot introduce RGB blends outside the
    // authored ramp.
    return density_color(mix(level, tone, cloud.cloud_depth));
}
// Brightness is a display adjustment, after the palette and color memory.
// It reads the same globs at every blur resolution and never feeds back into
// held color, so turning the slider requires neither a rebake nor a reset.
fn clouded(level: f32, position: vec2<f32>) -> vec4<f32> {
    let color = clouded_base(level, position);
    if cloud.cloud_style != 1u || cloud.wash_randomness <= 0.0 || cloud.cloud_depth <= 0.0 {
        return color;
    }
    let pt = position / cloud.ppp - cloud.origin;
    let field = wash_tile_field(wash_cell_at(pt));
    let draw = mix(field.coarse.brightness, field.fine.brightness, cloud.wash_layers * field.cover);
    let varied = wash_vary_brightness(linear_from_gamma_rgb(color.rgb), 1.0, draw,
        cloud.wash_randomness * cloud.cloud_depth);
    return vec4<f32>(gamma_from_linear_rgb(varied), color.a);
}

// Full material memory replaces the base level, so neither its grid walk nor
// its blurred sample contributes. Stars keep their separate color composite.
fn full_material_memory() -> bool {
    return cloud.memory_enabled != 0u && cloud.cloud_depth >= 1.0 && cloud.cloud_style != 2u;
}

// Empty history uses the same field and palette with a zero measured core.
// This quad never samples the grid, so the oldest column cannot be smeared.
fn backdrop_color(position: vec2<f32>) -> vec4<f32> {
    if full_material_memory() { return clouded(0.0, position); }
    var level = 0.0;
    if softened() { level = baked_density(position); }
    return clouded(level, position);
}
@fragment
fn fs_cloud_backdrop_gamma(in: VertexOut) -> @location(0) vec4<f32> {
    return backdrop_color(in.position.xy);
}
@fragment
fn fs_cloud_backdrop_linear(in: VertexOut) -> @location(0) vec4<f32> {
    let gamma = backdrop_color(in.position.xy);
    return vec4<f32>(linear_from_gamma_rgb(gamma.rgb), 1.0);
}
// One of the two levels, and only that one is read. The measured core is a walk
// over every bucket under the pixel in two slabs, and the soft field REPLACES it
// rather than blending with it, so reading both and keeping one paid for the
// walk on every softened frame — which is nearly every frame drawn here.
fn cloud_color(in: VertexOut) -> vec4<f32> {
    if full_material_memory() { return clouded(0.0, in.position.xy); }
    var level: f32;
    if softened() {
        level = baked_density(in.position.xy);
    } else {
        level = heatmap_level(in);
    }
    return clouded(level, in.position.xy);
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

// The level a star sees at pane point `pt`: the Spread-combined light, so
// how loosely the stars follow the picture is `Wide blur mix` and the two
// softnesses, as it is for every texture. A Stars-only blur toward the wide
// light stood here and was that dial a second time.
// =============================== THE STARFIELD ===============================
//
// The third texture, and the one that is not a displacement. Yan asked for *"lots
// of dense pinpoints of varying brightness and color"*, then *"a parallax effect,
// ideally looking as if they're all independent, yet overall all drifting in the
// same direction"*, with stars that *"take on the color of the place they drift
// over as they move, but not look like a flat effect pasted on the spectrogram"*.
// Prototyped in numpy over a real take (round 4's `drift.py`).
//
// **Depth slices, not octaves.** Five jittered star grids from far (fine dust,
// cells of `Star size`'s far end, many and faint) to near (cells of its near
// end, few, bright, soft), each sliding at the shared drift times its own
// parallax factor. The CPU works out every slice's numbers and its drift
// (`star_slices`); native composition reads one core per slice and its halo image.
//
// **Each star is worked out once a frame, not once per pixel.** Everything about
// a star but its coverage — its life, jitter, the light under it, its
// colour and size — depends on the star alone, and nine cells a slice round
// every pixel took it again at every pixel in reach: about 500 times a frame for
// a far star at 4K and 6000 for a near one. So `fs_star_bake` draws every
// slice's cells on screen into `star_atlas`, a texel a cell, and the pixel's
// native walk reads one texel a slice. The separate halo pass gathers nine
// cells at the selected resolution and keeps each slice's color and coverage separate
// (#1142).
//
// **Lives.** A star lives `Star lifetime`, then its cell draws a new star,
// fading the old one out and the new one in over the ends of their lives. Cells
// turn over at hashed times, so the field never does at once. The life a star
// is in is hashed into everything about it. Each depth moves as one sheet: a
// per-star speed spread that let each star stray at its own speed paid for it
// out of every star's reach, and cut the soft edges off the whole field.
//
// **One light tap per star, at the star's CURRENT centre.** So a star is one
// colour and one brightness, never a smear of the pixels under it, and as it
// drifts it takes on the colour of what it crosses.
//
// **Paint, not light.** Yan: *"star color should change over time but each star
// should only have one color at any given moment"*. Summed light could not do
// that — a bright core tonemapped per channel went white while its rim kept the
// hue. So a star is ONE palette colour, and its brightness is spent as a
// POSITION on the palette rather than a multiply, which would turn an orange
// into brown mud: a dim star is the scheme's own lower colour and sinks into the
// sky, a bright one sits above what is behind it. Its shape is only coverage — a
// soft Gaussian point with an optional same-colour fringe — and the slices are
// laid far to near, each OVER what is under it, so no core can whiten and no rim
// can turn another colour. Round 8 of the prototype (`round8.py`, Yan's YB3).
//
// The ground under them is the scheme's floor, and contours never reach it.
// Silence is exactly the floor: a star over silence is not drawn.
//
// Every length is in STAR PIXELS, a 540th of the pane's height, because the
// prototype's pane was 540 pixels; the stars keep their size relative to the
// pane at any export resolution, like the other textures' `CLOUD_UNITS`.
fn star_level_at(pt: vec2<f32>) -> f32 {
    let uv = pt / cloud.size;
    return clamp(textureSampleLevel(close_light, cloud_sampler, uv, 0.0).r, 0.0, 1.0);
}
// A star's one colour, its brightness spent as a palette position. `rank` is
// the star's brightness draw normalised to a mean of ONE: most stars below it,
// a rare bright one far above, steeper with `Brightness variation`. Both the
// spread and the lift are linear in it with a mean of one and of
// `STAR_LIFT / 2`, so the field's average palette position is the light behind
// it plus that lift at every `Brightness variation` — the dial redistributes
// brightness, it does not darken (it did before, by about a quarter at 0.6). Only the palette's top clips.
// At 0 every star is the colour behind it lifted by its draw, as before. The
// lift fades in with the level, so a star over near-silence cannot climb the
// palette on its rank alone. The mean rank, 1, paints the mean star.
fn star_paint(level: f32, rank: f32) -> vec3<f32> {
    let randomness = cloud.star_randomness;
    let spread = (1.0 - randomness) + randomness * (0.35 + 0.65 * rank);
    let lift = 0.5 * STAR_LIFT * rank * smoothstep(0.0, 0.15, level);
    return palette_color(clamp(level * spread + lift, 0.0, 1.0));
}
fn star_origin() -> vec2<f32> { return cloud.origin; }
fn star_size() -> vec2<f32> { return cloud.size; }
fn star_ppp() -> f32 { return cloud.ppp; }
fn star_randomness() -> f32 { return cloud.star_randomness; }
fn star_life() -> f32 { return cloud.star_life; }
fn star_size_variation() -> f32 { return cloud.star_size_variation; }
fn star_far() -> vec4<f32> { return cloud.star_far; }
fn star_near() -> vec4<f32> { return cloud.star_near; }
fn star_geometry() -> vec4<f32> { return cloud.star_geometry; }
fn star_slice(k: u32) -> StarSlice { return cloud.star_slices[k]; }
fn star_halo_sample(k: u32) -> StarHaloSample { return cloud.star_halo_samples[k]; }
fn star_floor() -> vec4<f32> { return vec4<f32>(palette_color(0.0), 1.0); }
fn star_source(pt: vec2<f32>, rank: f32, index: i32) -> vec4<f32> {
    let level = star_level_at(pt);
    if cloud.memory_enabled != 0u {
        let held = textureLoad(color_memory, atlas_texel(index), 0);
        if held.a <= 0.0 { return vec4<f32>(0.0); }
        return vec4<f32>(gamma_from_linear_rgb(held.rgb), 1.0);
    }
    if level <= 0.0 { return vec4<f32>(0.0); }
    return vec4<f32>(star_paint(level, rank), 1.0);
}
