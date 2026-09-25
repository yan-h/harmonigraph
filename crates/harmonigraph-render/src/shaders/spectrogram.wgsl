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
    // How far the levels are gathered into terraces, 0 for none. This word was
    // a style enum: Plain, Blur and Lava are now the blur, the terraces and the
    // cloud each at zero or not, read off their own dials.
    contour_strength: f32,
    // 1 when the cloud's scalar tone has been drawn into `cloud_tone` at the
    // resolution `Cloud pixel size` asks for, so the composite reads it there
    // instead of walking the cells per pixel. 0 is the native path, which is
    // what every `cloud_pixel` at or under one device pixel takes.
    tone_baked: u32,
    // Watercolour clouds. `drift` is the wash's offset in cloud units; the rest
    // are the sanitized settings. The filter shader declares only the head of
    // this struct, which is why these are appended rather than interleaved.
    drift: vec2<f32>,
    cloud_depth: f32,
    scale_size: f32,
    scale_variety: f32,
    scale_refract: f32,
    // Which texture the layer draws: 0 the refracting scales above, 1 the
    // watercolour wash below. Nothing is shared between the two but the blurred
    // light, the palette, the clock and `cloud_depth`.
    cloud_style: u32,
    wash_size: f32,
    wash_fuzz: f32,
    wash_lobe: f32,
    wash_refract: f32,
    wash_layers: f32,
    // The tile's period in cells, above zero whenever a cloud is drawn. The
    // cell a hash is taken at is folded onto the square period described
    // beside `wrap_cell`, `fs_cloud_tile` bakes one period of it, and the two
    // paths below read that texture instead of walking the ring per pixel. The
    // wash rotates that read; the mosaic keeps the square tile's original axes.
    tile_cells: u32,
    // 1 when pitch is the pane's Y axis, 0 when it is X. The wash's 3-4-5
    // rotation is defined in (time, pitch), so its basis follows this
    // orientation. The unrotated mosaic does not read it.
    pitch_vertical: u32,
    _pad: vec2<u32>,
};
@group(1) @binding(0) var close_light: texture_2d<f32>;
@group(1) @binding(1) var wide_light: texture_2d<f32>;
@group(1) @binding(2) var cloud_sampler: sampler;
@group(1) @binding(3) var<uniform> cloud: Cloud;
/// The cloud's scalar tone, one texel per `cloud_pixel` of pane, as `fs_cloud_tone`
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
/// The tile's own sampler, and the only REPEATING one here: the mosaic divides
/// its cell coordinate by the period, while the wash first turns it into the
/// rotated basis; either texture coordinate wraps. `cloud_sampler` clamps,
/// which every other read wants —
/// a refracted lookup that ran off the pane must hold its edge rather than
/// return the light from the far side of the picture.
@group(1) @binding(7) var tile_sampler: sampler;

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
// one over its radius, so a raw slope would make Scale size silently a second
// refraction knob — halve the scale and the picture bends twice as far.
// `DOME_FACE` takes that out, and takes out the same effect WITHIN one field now
// that `Variety` gives each glob its own radius.
//
// Cloud space is the pane's, aspect-corrected and independent of DPI, and it is
// FIXED: ten cloud units across the pane's height, which is what the shipped
// `Cloud size` of 0.5x drew before the dial was retired.
//
// It was a dial, and it was a second copy of `Scale size` and `Glob size`. The
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
// 1's flat per-glob patch, and `Variety` is how much the globs differ in size.
const CLOUD_UNITS: f32 = 10.0;
// The tiled WASH is turned by the exact 3-4-5 rotation: cosine 4/5, sine 3/5,
// or 36.87 degrees. Its square walk is baked in its own coordinates and this
// rotation is applied only when it is read, so every octave still closes on
// the ordinary square period and no resampling stretch is introduced. Mosaic
// deliberately keeps the square tile's original axes: turning its scale pile
// changed the look rather than merely hiding its repetition.
const CLOUD_TILE_ROT_COS: f32 = 0.8;
const CLOUD_TILE_ROT_SIN: f32 = 0.6;
// How many dome cells cross one cloud unit at `Scale size` 1x. Carries the
// retired `Cloud size` default: the shipped picture was 6 cells per unit over a
// frame half this one's, and `6 / 2.2` at the old `Scale size` default is what
// puts the same scales on the pane with the dial reading a plain 1x.
const SCALE_CELLS: f32 = 6.0 / 2.2;

// The cell a hash is taken at, folded onto the tile when one is being baked.
//
// The tile is square in its OWN coordinates. Rotating the already-periodic
// result below keeps every lattice the walk uses exact; trying instead to wrap
// the world-cell hashes on the 3-4-5 vectors would leave the 2.1x and 0.9x
// octaves on fractional cells and draw a seam.
//
// WGSL's `%` truncates toward zero, so `-1 % 20` is `-1` and the second fold is
// what lands a negative cell in the range. A period of 0 returns the cell
// whole: no production pass asks for it, but it is the unwrapped walk the
// tests hold the tile against.
fn wrap_cell_for_tile(cell: vec2<i32>, period: i32) -> vec2<i32> {
    if period <= 0 {
        return cell;
    }
    return ((cell % vec2<i32>(period)) + vec2<i32>(period)) % vec2<i32>(period);
}

fn wrap_cell(cell: vec2<i32>, period: i32) -> vec2<i32> {
    return wrap_cell_for_tile(cell, period);
}

// Watercolor texture coordinates in the rotated basis. `semantic` is `(time, pitch)`
// whichever way the pane is oriented; multiplying by R^-1 turns the world
// point back into the square tile's coordinates. The repeating sampler then
// makes its two screen-space repeat vectors `(4P/5, 3P/5)` and
// `(-3P/5, 4P/5)` in `(time, pitch)`.
fn watercolor_tile_uv_for(r: vec2<f32>, period: f32, pitch_vertical: u32) -> vec2<f32> {
    let semantic = select(vec2<f32>(r.y, r.x), r, pitch_vertical == 1u);
    return vec2<f32>(
        CLOUD_TILE_ROT_COS * semantic.x + CLOUD_TILE_ROT_SIN * semantic.y,
        -CLOUD_TILE_ROT_SIN * semantic.x + CLOUD_TILE_ROT_COS * semantic.y,
    ) / period;
}

fn watercolor_tile_uv(r: vec2<f32>) -> vec2<f32> {
    return watercolor_tile_uv_for(r, f32(cloud.tile_cells), cloud.pitch_vertical);
}

// The baked channels carry directions as well as scalars. Sampling them at a
// rotated coordinate turns the geometry only if its vectors turn with it;
// otherwise refraction would still point along the unrotated
// field. Convert to `(time, pitch)`, apply R, and return to pane axes.
fn rotate_watercolor_tile_vector_for(v: vec2<f32>, pitch_vertical: u32) -> vec2<f32> {
    let semantic = select(vec2<f32>(v.y, v.x), v, pitch_vertical == 1u);
    let turned = vec2<f32>(
        CLOUD_TILE_ROT_COS * semantic.x - CLOUD_TILE_ROT_SIN * semantic.y,
        CLOUD_TILE_ROT_SIN * semantic.x + CLOUD_TILE_ROT_COS * semantic.y,
    );
    return select(vec2<f32>(turned.y, turned.x), turned, pitch_vertical == 1u);
}

fn rotate_watercolor_tile_vector(v: vec2<f32>) -> vec2<f32> {
    return rotate_watercolor_tile_vector_for(v, cloud.pitch_vertical);
}

// One cell's dome, as four 10-bit fractions: where its centre sits inside the
// cell, how wide it is, and how loudly it argues for its own territory.
//
// The first word is only good for three of them — the top two bits are too
// coarse to draw anything from — so the fourth comes from a second avalanche
// over the finished word rather than from bits the other three already spent.
fn cloud_hash4(cell: vec2<i32>) -> vec4<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    var m = (n ^ 0xb5297a4du) * 0x68e31da4u;
    m = m ^ (m >> 15u);
    return vec4<f32>(
        f32(n & 0x3ffu) / 1023.0,
        f32((n >> 10u) & 0x3ffu) / 1023.0,
        f32((n >> 20u) & 0x3ffu) / 1023.0,
        f32(m & 0x3ffu) / 1023.0,
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
// How many octaves of weight `Variety` may give or take from one dome, and the
// reason the dial is worth turning at all.
//
// **The radius band above is not what `Variety` reads as.** What the eye calls
// one scale here is the TERRITORY a dome wins from the soft union, and the grid
// that sets the territory is one dome per cell however wide each dome is drawn.
// Measured over an interior patch of the field, the shipped radius band moved
// the 10th-to-90th-percentile territory from 1.22:1 at `Variety` 0 to 1.51:1 at
// `Variety` 1 — a band already nearly uniform, opened by a quarter. That is the
// whole of what the dial used to buy, and it is why it read as doing nothing.
//
// A weight gain moves the BISECTORS instead, which is the same measurement's
// 4.2:1 at the constant below. It is outside the coverage proof entirely: the
// union is a weighted MEAN, every weight stays positive, and no radius changes,
// so neither inequality above is touched and a suppressed dome cannot open a
// hole — it can only lose its cell to a neighbour that already reached across
// it.
//
// The ceiling is smoothness, not coverage. The steepest single-pixel step in
// the face field is 9.3 per cell here, BELOW the 9.8 the shipped dial already
// drew at `Variety` 1; at 7 octaves it is 15.4 and at 8 it is 21.3, which is a
// swallowed dome's influence ending in a visible ring rather than fading.
const DOME_VARIETY_GAIN: f32 = 5.0;
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
};

// One octave of domes: a soft union over the 3x3 ring, with the union's own
// weights carrying each dome's normalised face out alongside the rest.
//
// EVERY cell has a dome. There used to be an `occupancy` draw that left 22% of
// them empty, which is where the sky between the clouds came from; Yan wants the
// texture everywhere, so the draw is gone and the hash word it spent went with
// it — freed, and spent on the radius below.
//
// A dome the WEIGHT gain suppresses is not that draw coming back. An empty cell
// left a hole, because a hole is what `occupancy` skipped the dome to make; a
// suppressed dome still covers its own cell and still has a face, it has just
// lost the argument about whose face this pixel reads. The union is continuous
// across the whole plane either way.
//
// `period` is the tile's own, in THIS octave's cells, and 0 for the unwrapped
// walk the tile test holds the bake against.
// Only the hash's cell is folded by it; the centre below is built from the
// unwrapped cell, so a dome at the tile's far edge still sits where it sits.
fn dome_octave(r: vec2<f32>, period: i32) -> Pile {
    let base = floor(r);
    var weight = 0.0;
    var face = vec2<f32>(0.0);
    var to_centre = vec2<f32>(0.0);
    for (var j = -1; j <= 1; j += 1) {
        for (var i = -1; i <= 1; i += 1) {
            let cell = vec2<i32>(base) + vec2<i32>(i, j);
            let h4 = cloud_hash4(wrap_cell(cell, period));
            let centre = base + vec2<f32>(f32(i), f32(j)) + 0.5
                + (h4.xy - 0.5) * DOME_JITTER;
            // Each dome's own width. `Variety` opens the band from the single
            // shared radius, never below `DOME_RADIUS_MIN`, so every step of the
            // dial is still a proof that the plane is covered.
            let radius = mix(
                DOME_RADIUS,
                mix(DOME_RADIUS_MIN, DOME_RADIUS_MAX, h4.z),
                cloud.scale_variety,
            );
            let d = (r - centre) / radius;
            let q = 1.0 - dot(d, d);
            if q <= 0.0 {
                continue;
            }
            let root = sqrt(q);
            let h = q * root;
            // Each dome's own say in the union, log-symmetric about the shared
            // weight so `Variety` gives one dome a neighbour's cell exactly as
            // often as it takes its own away. Behind a knob because an `exp2`
            // per dome per pixel is real work for a gain that is exactly 1, and
            // the branch is on a uniform, so no two lanes ever disagree about
            // taking it.
            var gain = 1.0;
            if cloud.scale_variety > 0.0 {
                gain = exp2(DOME_VARIETY_GAIN * cloud.scale_variety * (2.0 * h4.w - 1.0));
            }
            // `- 1.0` is what lets the gain exist. A dome ENTERS the ring at
            // `q = 0`, where `exp(0)` is 1 rather than 0 — a step, tiny against
            // a dominant dome's `exp(6.3)` and invisible while every dome
            // weighs the same, but multiplied by a gain of 32 it is a fifth of
            // the union arriving at once, which draws the hard ring this whole
            // construction exists not to draw. Subtracting the pedestal lets a
            // rim contribution fade to nothing however loud the dome is, and it
            // retires the old step at `Variety` 0 as well.
            let w = gain * (exp(DOME_UNION * h) - 1.0);
            weight += w;
            face += w * (-(DOME_FACE * root * radius)) * d;
            to_centre += w * (centre - r);
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
        return out;
    }
    out.face = face / weight;
    out.to_centre = to_centre / weight;
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

fn cloud_domes(r: vec2<f32>, period: i32) -> Pile {
    let coarse = dome_octave(r, period);
    // The finer octave counts in its OWN cells, `DOME_LACUNARITY` of them to
    // one coarse cell, so the tile closes on `DOME_LACUNARITY * period` of
    // them. Rounded because 2.1 is not exact in binary and this has to be the
    // whole number `the_tile_period_tiles_every_lattice` proves it is.
    let fine = dome_octave(
        r * DOME_LACUNARITY + vec2<f32>(17.3, 5.9),
        i32(round(DOME_LACUNARITY * f32(period))),
    );
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
    return out;
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
    // `Variety` has made of it.
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
// Each glob reads the displaced scalar level without a tone adjustment.
// Layers mixes those levels before the shared Contours and palette transfer.
//
// **Feather is the fuzziness.** A visible glob dissolves at its OWN rim into
// whatever lies beneath it, reaching half and half exactly on the boundary so
// both sides meet. That is the antialiasing as well: there is no supersampling
// here and the prototype's final renders had none either, precisely so they
// showed what a fragment shader would really draw.

// The ring each octave walks, and the four numbers that decide whether walking
// it is enough. THESE ARE A PROOF and not four tastes, held against the shipped
// text of this file by `the_wash_grid_covers_the_plane_and_the_ring_holds_it`.
//
// **Coverage.** A centre sits at its cell's middle give or take `JITTER / 2` on
// each axis, so it can be `(JITTER / 2) * sqrt(2)` from that middle in any
// direction. The point hardest to reach is a lattice corner with all four
// cells touching it pushed away from it, `0.5 * sqrt(2) + (JITTER / 2) * sqrt(2)`
// from every one of them, and the SMALLEST radius a glob can draw has to clear
// that. An uncovered point is not a dim spot — it is a pixel that reads its own
// light with no glob's centre to borrow, so the lookup falls off a cliff from
// most of a radius to nothing.
//
// **Reach.** A cell `RING + 1` out can put its centre no nearer than
// `RING + 1.5 - (JITTER / 2) * sqrt(2)` from the pixel's own cell origin, and the
// pixel is at most 1 past that origin, so the LARGEST rim has to stay under
// `RING + 0.5 - (JITTER / 2) * sqrt(2)` or a glob the ring never visits can cover
// the pixel — which is a step on the cell grid every time `floor(r)` moves.
//
// The largest RIM is the largest radius, and it was not always. A retired
// `Ragged` dial pushed a rim OUTWARD by up to 30% of its own radius and never
// inward, so the reach bound carried `RADIUS_MAX * (1 + RAGGED)` while coverage
// read `RADIUS_MIN` untouched. One-sided also meant every rim sat about 15%
// OUTSIDE its radius on average at the setting that shipped, so the band was
// scaled by 1.15 when the wobble went, and a default glob is the size it always
// drew. What the reach bound stops carrying is slack: 1.91 against a bound of
// 2.217, where the wobble left 2.7% of a radius. Spending it on a wider band
// than 1.63:1 is a LOOK change and is deliberately not taken here.
//
// Both bounds are about globs that COVER the pixel, which is what the visible
// glob, the one beneath it and `cover` are all read off. The front used for
// bleed is chosen from the two nearest non-covering globs in `wash_scan`.
//
// A 5x5 ring rather than 3x3, and it is the jitter and the variety that buy it.
// At 3x3 these same inequalities leave a radius band of about 1.2:1 with a
// jitter of 0.20 — a nearly regular grid of nearly equal globs, which is the one
// thing this look cannot be, since a field of DIFFERENT SIZED globs is what was
// asked for. The wider ring costs a second pass over 25 cells instead of 9 and
// buys jitter 0.40 and a 1.63:1 radius band, with 15.4% of a radius spare on
// coverage and 16.1% on reach — the room the retired wobble used to spend.
const WASH_RING: i32 = 2;
const WASH_JITTER: f32 = 0.40;
const WASH_RADIUS_MIN: f32 = 1.17;
const WASH_RADIUS_MAX: f32 = 1.91;

// How many cells cross one cloud unit at `Glob size` 1x — see `wash_cloud_tone`,
// where it is chosen so a glob comes out the width the prototype's J2 drew
// rather than so the CELLS come out at J2's count.
const WASH_CELLS: f32 = 5.25;

// The finer octave: how much smaller its cells are, and how many of them carry a
// glob at all. It is sparse on purpose — a big wash sometimes carries a small one
// and sometimes sits beside it, and where two washes meet the tone steps, which
// is where the reference's tones come from. Only the BASE octave owes coverage;
// a pixel no fine glob reaches simply shows the coarse wash under it.
const WASH_LACUNARITY: f32 = 2.1;
const WASH_FINE_OCCUPANCY: f32 = 0.20;

// The domain warp's maximum displacement and noise frequency, in cells.
// The frequency must close over the tile period, like the finer octave.
const WASH_WARP: f32 = 0.45;
const WASH_WARP_SCALE: f32 = 0.9;

// Three 10-bit fractions off a salted cell hash. Two of these per cell: one for
// the paint order and the occupancy draw, one for the centre and the radius.
// Five channels is what the construction needs, and one word holds three.
fn wash_hash(cell: vec2<i32>, salt: u32) -> vec3<f32> {
    var n = (bitcast<u32>(cell.x) * 0x9e3779b9u) ^ (bitcast<u32>(cell.y) * 0x85ebca6bu);
    n = n ^ (salt * 0x27d4eb2du);
    n = (n ^ (n >> 16u)) * 0x7feb352du;
    n = (n ^ (n >> 15u)) * 0x846ca68bu;
    n = n ^ (n >> 16u);
    return vec3<f32>(
        f32(n & 0x3ffu) / 1023.0,
        f32((n >> 10u) & 0x3ffu) / 1023.0,
        f32((n >> 20u) & 0x3ffu) / 1023.0,
    );
}

// Smooth value noise, two octaves. Used for the SHARED domain warp only —
// evaluated once per pixel and then read by every glob of every octave, which is
// what keeps neighbouring globs leaning together along a shared boundary instead
// of each wandering off on its own.
fn wash_noise(p: vec2<f32>, salt: u32, period: i32) -> f32 {
    let b = floor(p);
    let f = p - b;
    let t = f * f * (3.0 - 2.0 * f);
    let i = vec2<i32>(b);
    let n00 = wash_hash(wrap_cell(i, period), salt).x;
    let n10 = wash_hash(wrap_cell(i + vec2<i32>(1, 0), period), salt).x;
    let n01 = wash_hash(wrap_cell(i + vec2<i32>(0, 1), period), salt).x;
    let n11 = wash_hash(wrap_cell(i + vec2<i32>(1, 1), period), salt).x;
    return mix(mix(n00, n10, t.x), mix(n01, n11, t.x), t.y);
}
// Where this noise's second octave sits, and the one constant the TILE changes.
//
// `WASH_FBM_FINE` is an irrational-looking 2.07 exactly so the two octaves never
// line up, and no tile period makes `2.07 * P` a whole number of the finer
// lattice's cells — so a tiled walk runs it at exactly 2 instead, which doubles
// the period with it and tiles for every `P` the coarse lattice already does.
// A period of 0, the unwrapped walk that no production pass draws since #1100,
// keeps 2.07 bit for bit.
const WASH_FBM_FINE: f32 = 2.07;
const WASH_FBM_FINE_TILED: f32 = 2.0;
fn wash_fbm(p: vec2<f32>, salt: u32, period: i32) -> f32 {
    let lacunarity = select(WASH_FBM_FINE, WASH_FBM_FINE_TILED, period > 0);
    let coarse = wash_noise(p, salt, period);
    let fine = wash_noise(p * lacunarity + vec2<f32>(13.1, -7.3), salt + 31u, period * 2);
    return (coarse + 0.5 * fine) / 1.5;
}

struct Glob {
    centre: vec2<f32>,
    // Where the pixel sits on this glob's rim: under 1 is inside it. A rim
    // coordinate rather than a distance, so feather and bleed are measured
    // in fractions of each glob's own radius.
    edge: f32,
    order: f32,
}

// One cell's glob, at the pixel `r` — both in this octave's cell units. `period`
// folds the cell the two hashes are taken at and nothing else, so the centre
// below is still this cell's own (see `wrap_cell`).
fn wash_glob(cell: vec2<i32>, salt: u32, r: vec2<f32>, occupancy: f32, period: i32) -> Glob {
    let hashed = wrap_cell(cell, period);
    let g = wash_hash(hashed, salt + 77u);
    var out: Glob;
    out.order = g.x;
    // A cell the occupancy draw missed carries no glob, and that is four cells
    // in five of the finer octave, so it is answered before the geometry is
    // worked out: the second hash and the square root are most of what a cell
    // costs. Its rim is put out of reach, where `wash_scan` does exactly
    // nothing with it — `1 - 1e9` rounds to `-1e9` in an f32, which is the
    // value every running nearest starts at and no `>` passes, and it adds a
    // clamped zero to the cover. So the early return draws the picture the
    // uniform loop drew, bit for bit.
    //
    // The compare takes the boundary because `wash_hash` returns
    // `(n & 0x3ff) / 1023`, which is a CLOSED range: a channel can be exactly
    // 1.0, and the base octave is scanned at an occupancy of exactly 1.0. Under
    // a strict `>=` those two 1.0s met and the base octave dropped about one
    // cell in 1024 — a hole the coverage half of the proof above forbids, since
    // its hardest point is a lattice corner reached by all four cells touching
    // it. The finer octave cannot tell the two compares apart: `0.20 * 1023` is
    // 204.6, so no hash value lands on `WASH_FINE_OCCUPANCY` at all.
    if g.y > occupancy {
        out.centre = r;
        out.edge = 1.0e9;
        return out;
    }
    let h = wash_hash(hashed, salt);
    let centre = vec2<f32>(cell) + 0.5 + (h.xy - 0.5) * WASH_JITTER;
    // Each glob draws its own radius from the whole band the proof above
    // allows. That was the top of a `Variety` dial, which is where it shipped
    // and where it stays: the band is 1.63:1 and the paint order decides which
    // glob a pixel shows, so the dial moved a twentieth of the pane end to end,
    // and the size range the look is after comes from `Layers` instead.
    //
    // A `Wander` dial turned each centre about its own cell here, on a hashed
    // rate. It could only ever TURN the jitter — a travel would break the reach
    // bound — and a quarter of a cell swung round once a minute is a point or
    // two of movement under a whole field already drifting faster than that.
    let radius = mix(WASH_RADIUS_MIN, WASH_RADIUS_MAX, h.z);
    out.centre = centre;
    out.edge = length(r - centre) / radius;
    return out;
}

struct Wash {
    // The visible glob's centre, and the centre of the glob directly beneath it
    // — what its own rim dissolves INTO.
    centre: vec2<f32>,
    under: vec2<f32>,
    // The nearest glob painted AFTER the visible one, which is the arc about to
    // take this pixel, and how near it is as `1 - edge` (never above 0).
    front: vec2<f32>,
    near: f32,
    // Where the pixel sits on the visible glob's rim, and whether any glob
    // covers it at all — the latter is what a finer wash is composited by.
    edge: f32,
    cover: f32,
}

// One octave, in ONE walk of the ring.
//
// Two things come out of it. The two highest-ordered globs COVERING the pixel
// are the one it shows and the one its rim dissolves into, and both are a plain
// running top-two. The FRONT — the glob painted after
// the visible one whose arc is about to take this pixel — is the awkward one: it
// is defined against an answer the same walk is still computing, since the
// visible glob's order is not known until the last cell.
//
// Keep the two nearest non-covering globs, then choose the higher-ordered
// neighbour for the lookup bleed. This retains the established single-walk
// geometry; a full second search would change which edges blend together.
fn wash_scan(r: vec2<f32>, salt: u32, occupancy: f32, period: i32) -> Wash {
    var out: Wash;
    // An uncovered pixel reads its own light, unmoved. Unreachable for the
    // base octave while the constants hold — see
    // the proof above — and the ordinary case for a sparse finer one, which is
    // composited by `cover` and so never shows it.
    out.centre = r;
    out.under = r;
    out.front = r;
    out.near = -1.0e9;
    out.edge = 1.0;
    out.cover = 0.0;
    var best = -1.0e9;
    var second = -1.0e9;
    // The two nearest globs the pixel is OUTSIDE, with the order each was
    // painted at, so the front can be chosen once `best` has settled.
    var near_a = -1.0e9;
    var near_b = -1.0e9;
    var order_a = -1.0e9;
    var order_b = -1.0e9;
    var front_a = r;
    var front_b = r;
    let base = vec2<i32>(floor(r));
    for (var j = -WASH_RING; j <= WASH_RING; j += 1) {
        for (var i = -WASH_RING; i <= WASH_RING; i += 1) {
            let glob = wash_glob(base + vec2<i32>(i, j), salt, r, occupancy, period);
            let prox = 1.0 - glob.edge;
            out.cover = max(out.cover, clamp(prox / 0.05, 0.0, 1.0));
            if glob.edge < 1.0 {
                if glob.order > best {
                    second = best;
                    out.under = out.centre;
                    best = glob.order;
                    out.centre = glob.centre;
                    out.edge = glob.edge;
                } else if glob.order > second {
                    second = glob.order;
                    out.under = glob.centre;
                }
            } else if prox > near_a {
                near_b = near_a;
                order_b = order_a;
                front_b = front_a;
                near_a = prox;
                order_a = glob.order;
                front_a = glob.centre;
            } else if prox > near_b {
                near_b = prox;
                order_b = glob.order;
                front_b = glob.centre;
            }
        }
    }
    // The further of the two first, so the nearer one wins if both qualify.
    if order_b > best {
        out.near = near_b;
        out.front = front_b;
    }
    if order_a > best {
        out.near = near_a;
        out.front = front_a;
    }
    return out;
}

// A wash tile holds only the lookup offset, in its own octave's cell units.
struct Wet {
    offset: vec2<f32>,
};

// The cell walk chooses the lookup. Fuzz feathers and bleeds that lookup
// across glob boundaries; it never changes the sampled level.
fn wash_wet(f: Wash, r: vec2<f32>) -> Wet {
    let feather = 0.10 + 0.80 * cloud.wash_fuzz;
    let bleed = 0.12 + 0.78 * cloud.wash_fuzz;

    var look = f.centre;
    // FEATHER: the visible wash dissolves at its own rim into whatever lies
    // beneath, reaching half and half exactly on the boundary so the two sides
    // meet. This is the whole of the fuzziness AND the whole of the
    // antialiasing — there is no supersampling anywhere in this path.
    var fa = clamp((f.edge - (1.0 - feather)) / feather, 0.0, 1.0);
    fa = fa * fa * (3.0 - 2.0 * fa) * 0.5;
    look = mix(look, f.under, fa);
    // BLEED: the reading crossfades toward the glob about to cover this pixel,
    // over a band at their shared edge. Still one tap, and dialled up it is what
    // makes neighbouring washes run into each other.
    var bl = clamp((f.near + bleed) / bleed, 0.0, 1.0);
    bl = bl * bl * (3.0 - 2.0 * bl) * 0.5;
    look = mix(look, f.front, bl);

    return Wet(look - r);
}

fn wash_level(wet: Wet, pane_per_cell: f32, pt: vec2<f32>) -> f32 {
    return cloud_light(pt + wet.offset * pane_per_cell * cloud.wash_refract);
}

// The whole of the wash's geometry at a point, in cells: what each octave
// carries and how much of the pixel the finer one covers.
//
// Five numbers, not one of which reads the light, the sound or the clock —
// which is exactly why `fs_cloud_tile` can bake them into two `Rgba16Float`
// targets and the per-frame shader can read them back. The bake always walks
// both octaves, because `Layers` is a mix over channels the tile already holds
// and so is deliberately not in the tile's key.
struct WashField {
    coarse: Wet,
    fine: Wet,
    cover: f32,
};

fn wash_field(r: vec2<f32>, period: i32) -> WashField {
    // One shared field, evaluated once per pixel and then read by every glob of
    // every octave: a domain warp of glob space, which is what stops a glob
    // being a circle. It is read at the UNWARPED point and stays small — a heavy
    // warp draws flames — and it sits on a lattice `WASH_WARP_SCALE` cells
    // across, which is the period it tiles at.
    var warped = r;
    if cloud.wash_lobe > 0.0 {
        let amp = WASH_WARP * cloud.wash_lobe;
        let warp_period = i32(round(WASH_WARP_SCALE * f32(period)));
        warped += amp * 2.0 * vec2<f32>(
            wash_fbm(r * WASH_WARP_SCALE, 71u, warp_period) - 0.5,
            wash_fbm(r * WASH_WARP_SCALE + vec2<f32>(37.0, -19.0), 73u, warp_period) - 0.5,
        );
    }
    var out: WashField;
    out.coarse = wash_wet(wash_scan(warped, 1u, 1.0, period), warped);
    // Coarse to fine, the finer octave a translucent wash over the one below and
    // sparse, so a big wash sometimes carries a small one and sometimes sits
    // beside it.
    let fine_r = warped * WASH_LACUNARITY + vec2<f32>(17.3, 5.9);
    let fine =
        wash_scan(fine_r, 2u, WASH_FINE_OCCUPANCY, i32(round(WASH_LACUNARITY * f32(period))));
    out.fine = wash_wet(fine, fine_r);
    out.cover = fine.cover;
    return out;
}

// The same field out of the baked tile: the cell coordinate is turned into the
// wash's rotated basis and divided by the period. That repeating coordinate is
// the whole of what the drift does here — it slides a fixed field rather than
// changing one.
fn wash_tile_field(r: vec2<f32>) -> WashField {
    let uv = watercolor_tile_uv(r);
    let a = textureSampleLevel(cloud_tile_a, tile_sampler, uv, 0.0);
    var out: WashField;
    out.coarse = Wet(rotate_watercolor_tile_vector(a.xy));
    out.fine = Wet(vec2<f32>(0.0));
    out.cover = 0.0;
    if cloud.wash_layers > 0.0 {
        let b = textureSampleLevel(cloud_tile_b, tile_sampler, uv, 0.0);
        out.fine = Wet(rotate_watercolor_tile_vector(b.xy));
        out.cover = b.w;
    }
    return out;
}

// The wash's scalar tone at a pane-relative point — `scale_tone`'s counterpart,
// split out for the same reason.
fn wash_cloud_tone(pt: vec2<f32>) -> f32 {
    let q = (pt - cloud.size * 0.5) / cloud.size.y * CLOUD_UNITS + cloud.drift;

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
    let r = q * cells;

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
    if cloud.cloud_style == 1u {
        return wash_cloud_tone(pt);
    }
    return scale_tone(pt);
}

// The cloud's tone reduced to a target of its own, one texel per `Cloud pixel
// size` of pane. The coverage quad carries the pane-relative 0..1 fraction in
// its `slab`/`t`, which is what makes this the same `pt` the composite would
// have walked under each of its own pixels.
@fragment
fn fs_cloud_tone(in: VertexOut) -> @location(0) vec4<f32> {
    return vec4<f32>(cloud_tone_at(vec2<f32>(in.slab, in.t) * cloud.size), 0.0, 0.0, 1.0);
}

// ====================== ONE PERIOD OF THE CELL WALK ========================
//
// The tile, baked whenever a cloud is drawn and read by both paths above. It has
// no pane, no drift and no light in it: it is one square period of whichever
// walk the style selects. The Watercolor read turns that whole field by 36.87
// degrees; the Mosaic read leaves it square. That is why a resize, a drift or
// a note never touches it and `Scale size` reaches it only
// through how many texels the renderer spends on a cell.
struct TileVertex {
    @builtin(position) position: vec4<f32>,
    // Where this texel sits in the tile, 0 at one corner and 1 at the other.
    // The PERIOD is applied in the fragment stage rather than here, because the
    // cloud uniform is bound to the fragment stage alone and a vertex read of
    // it would widen every cloud pipeline's layout for this one entry point.
    @location(0) fraction: vec2<f32>,
};
@vertex
fn vs_cloud_tile(@builtin(vertex_index) vertex: u32) -> TileVertex {
    let uv = vec2<f32>(f32((vertex << 1u) & 2u), f32(vertex & 2u));
    var out: TileVertex;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    out.fraction = uv;
    return out;
}

struct TileBake {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
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
    if cloud.cloud_style == 1u {
        // Five channels of glob geometry, the fine octave whatever `Layers`
        // says, so turning that dial up is a mix and never a rebake.
        let field = wash_field(wash_cell, period);
        out.a = vec4<f32>(field.coarse.offset, 0.0, 0.0);
        out.b = vec4<f32>(field.fine.offset, 0.0, field.cover);
    } else {
        // The mosaic's whole walk is these two vectors, so its second target is
        // never read. It is still allocated and still written, which is what
        // keeps a change of style a rebake rather than a reallocation.
        let pile = cloud_domes(mosaic_cell, period);
        out.a = vec4<f32>(pile.face, pile.to_centre);
    }
    return out;
}

fn clouded(level: f32, position: vec2<f32>) -> vec4<f32> {
    // There is no gate on the blur here, and there used to be: the light field
    // was only built when a softness was above zero, so the cloud quietly
    // vanished with the blur. The field is built whenever a cloud is drawn now,
    // and at zero softness it holds the measured picture unblurred.
    if cloud.cloud_depth <= 0.0 {
        return density_color(level);
    }
    let pt = position / cloud.ppp - cloud.origin;
    // Either the walk under this pixel, or one bilinear tap into what
    // `fs_cloud_tone` already walked. The palette lookup and the mix stay HERE
    // whichever it was, so the base picture, its terraces and the gradient are
    // full resolution even where the texture over them is not.
    var tone: f32;
    if cloud.tone_baked == 1u {
        tone = textureSampleLevel(cloud_tone, cloud_sampler, pt / cloud.size, 0.0).r;
    } else {
        tone = cloud_tone_at(pt);
    }
    // Mix levels before the one shared style/palette lookup: Cloud depth and
    // Watercolor Layers cannot introduce RGB blends outside the authored ramp.
    return density_color(mix(level, tone, cloud.cloud_depth));
}
// Empty history uses the same field and palette with a zero measured core.
// This quad never samples the grid, so the oldest column cannot be smeared.
fn backdrop_color(position: vec2<f32>) -> vec4<f32> {
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
