// What lattice.wgsl and text.wgsl both spell, spelled once.
//
// Not a module of its own. WGSL has no include and naga takes a string, so this
// text is prepended to another module's own source at pipeline creation
// (`with_common` in lib.rs) and compiled as one file with it. Its two callers
// are the lattice's module and the text module; every other shader here stands
// alone and is compiled as it is.
//
// What belongs here: arithmetic two modules run identically, and the resource
// declarations that arithmetic reads — which every including module therefore
// carries at the same group, binding, name and type, whether or not its own
// entry points reach them.
//
// What does not:
//
//  - Anything that reads a UNIFORM. `Uniforms` and `Locals` are different
//    structs at the same slot, so a function reaching for either can only live
//    in the module that declares it; what such a function needs out of one is
//    passed in as a parameter (`shadow_transmittance`'s `depth`, `cell_clip`'s
//    `size`).
//  - Anything only ONE module uses. A second home for a single caller is a file
//    to keep in sync for nothing.
//  - Anything that DIFFERS between the two, however close. A parameter is the
//    right answer where one number parts them; a near-copy moved here anyway is
//    a picture change dressed as a deduplication.
//
// Nothing here may call into an including module: this text comes first and has
// to compile against every one of them.

// The finished light: every node's halo screened together into one field
// ([`glow_light`] is how it is read).
//
// Read with `textureLoad` and never a sampler: the target is created at the
// scene's own pixel size, so it is 1:1 with the attachment being written and any
// filtering would be a blur nobody asked for. It also takes no derivative, which
// is what keeps it out of the early-out parity test's way.
//
// An including module may declare a SECOND texture at group 1 binding 0 for an
// entry point that reads that one instead (lattice.wgsl's `ink_strip`): a
// binding collision is diagnosed per entry point, so the two share the slot for
// as long as no entry point reads both, and one that ever wanted both would fail
// to compile, loudly, at pipeline creation.
@group(1) @binding(0) var glow_tex: texture_2d<f32>;

// The finished light at a pixel of the glow's target, premultiplied as the
// pass wrote it.
//
// Every reader of the target takes it through here, so that ink and the
// picture it stands in cannot come to read the light differently. The
// composite that lays the field down keeps its own copy (blit.wgsl's
// `fs_glow_over`) and is the one exception, that pass SAMPLING the target
// where this loads it.
//
// `coord` is taken as given. Where the pixel comes from is the caller's — a
// scene fragment's own position (lattice.wgsl's `light_coord`), a glyph's
// rounded one (text.wgsl's `glyph_light`) — and the two hold it inside the
// texture on their own terms.
fn glow_light(coord: vec2<i32>) -> vec4<f32> {
    return textureLoad(glow_tex, coord, 0);
}

// `share` of `light`, laid over ink already premultiplied by `alpha`. Every
// piece of ink in the picture takes the light through here — a node's rings,
// marks and glyphs (`node_paint`), a resting marker (`plus_paint`) and a name
// (`fs_fill_lit`) — so the lattice wears it in one operation rather than in
// several that have to agree.
//
// A SCREEN, where the ground under the ink takes an over, and the difference is
// what a NEIGHBOUR's light is allowed to do. The field is one layer, so the
// light at a piece of ink carries every sheet's halo; an over
// (`ink * (1 - light.a)`)
// lets a saturated halo from behind take the ink's other channels DOWN — a
// white name under a red one comes out red — which is ink losing its colour to
// something it stands in front of. `w + ink * (1 - w)` per channel can only
// brighten, whatever reaches the pixel. The ground keeps the over instead: that
// is `fs_glow_over`'s own blend state.
//
// Premultiplied, so the ink's own term carries `alpha`: this is the screen of
// the ink over `w`, scaled by the coverage the ink has, and whoever fills the
// `(1 - alpha)` it leaves takes its own light separately.
//
// A `share` of 1 is the whole field, which is what every RESTING piece of ink
// takes — a cross, a name, a silent slice — those being ground laid over lit
// ground, so unwashed they read as holes punched exactly where the light is
// brightest. Below 1 is the lit slice's own setting (`glow_wash`), a slice
// already being the colour of the halo around it.
//
// What this does NOT give ink is a way to hold off a NEIGHBOUR's light: the
// field is one layer, so washed ink is tinted by a far sheet's halo as well as
// by its own, light reaching through it from behind. Interleaving the sheets is
// the only answer to that and is the thing this design exists to not do; a
// node's own halo is the maximum at its own pixel, the falloff being measured
// from its centre, so the far share is small unless a lit node sits directly
// behind.
fn wash_over(ink: vec3<f32>, alpha: f32, light: vec3<f32>, share: f32) -> vec3<f32> {
    let w = light * share;
    return w * alpha + ink * (1.0 - w);
}

// The shadow atlas: every caster's own ink, blurred into a cell of its own
// (`shadow.rs`). A caster's draw reads the ONE cell that is a picture of its own
// ink and multiplies whatever is already in the frame under it by what that blur
// leaves. No receiver carries any shadow code: the light is composited under
// everything and takes every shadow by being there first.
//
// Bound on the SCENE pipelines alone. The draws that FILL a cell bind none of it
// — a texture cannot be read while it is the target being written — and take the
// atlas's size from a uniform of their own instead.
@group(2) @binding(0) var shadow_atlas: texture_2d<f32>;
@group(2) @binding(1) var shadow_sampler: sampler;

// The most terms a caster's kernel is built out of — `SHADOW_TERMS_MAX` in
// harmonigraph_scene, pinned to it by `the_shaders_term_count_is_the_scenes`.
const SHADOW_TERMS: u32 = 4u;

// One caster's whole kernel, as the scene pass reads it (`ShadowCaster` in
// shadow.rs).
struct ShadowCaster {
    // The union of every term's padded box, in the pane's points: min, then
    // size. The quad a caster's shadow is drawn over.
    rect: vec4<f32>,
    // x: how much of this caster's shadow lands, 0..=1. y/z/w unused.
    level: vec4<f32>,
    // What each term's cell HOLDS: 0 blurred ink, `DISTANCE_KIND` a distance.
    kind: array<f32, SHADOW_TERMS>,
    // Each term's σ in the pane's POINTS, which is what a distance read out of
    // a cell is measured against — one Shadow width is 2σ.
    sigma: array<f32, SHADOW_TERMS>,
    // Each term's cell in atlas texels: origin, then size. Zeroed past the
    // kernel's own term count, and zeroed whole where nothing was packed.
    cell: array<vec4<f32>, SHADOW_TERMS>,
    // Each term's map from a point of the pane to a texel of that cell —
    // `xy + points * z` — and w its share of the mixture, already normalized
    // over the BLUR terms and 0 on a distance term.
    map: array<vec4<f32>, SHADOW_TERMS>,
};

// What `ShadowCaster::kind` holds for a term whose cell is a DISTANCE —
// `shadow::DISTANCE_KIND`, and spelled again in shadow.wgsl because there is no
// linkage between shader modules here.
const DISTANCE_KIND: f32 = 1.0;

// Every caster's kernel, indexed by the caster's own index in the frame
// (`pack`'s order).
//
// A storage buffer and a group of its own, which is what a MIXTURE costs. One
// term rode beside the instance; four cannot — a node's rows reach location 15
// and leave five free, against the eight the cells need — and a term's cell is
// read by a node, a marker and a name alike, so one array they all index is
// also one place the shape is written down.
@group(3) @binding(0) var<storage, read> shadow_casters: array<ShadowCaster>;

// What a caster's kernel comes to at `points` of the pane, 0..=1 — the EXPONENT
// `shadow_transmittance` then spends the depth over.
//
// The whole row is mixed HERE and not after the transmittance, because the depth
// is spent as an exponent on the result: a sum of transmittances is a different
// picture from the transmittance of a sum, and the second is the one a kernel
// means.
//
// Each term's cell is at its OWN resolution — a narrow term is drawn finer than
// a wide one (`pack`) — so this is N bilinear taps into N pictures rather than
// N taps into one, and the kernel's core keeps the sharpness that is the whole
// reason a mixture is worth drawing.
//
// One loop across both FAMILIES, branching per term on what its cell holds. The
// gain enters here rather than at the transmittance because whether it applies
// is a property of the ROW: a blur row is gained and a distance row is not, and
// a function taking the finished exponent cannot tell them apart.
//
// `terms` is the kernel's own count, off a uniform: a term past it carries
// weight 0 and would cost a tap for nothing.
fn shadow_kernel(who: u32, points: vec2<f32>, terms: u32, gain: f32, shape: f32) -> f32 {
    if who >= arrayLength(&shadow_casters) {
        return 0.0;
    }
    let atlas = vec2<f32>(textureDimensions(shadow_atlas));
    // The blur terms, summed by weight; and the distance term's own coverage.
    var blur = 0.0;
    var cov = 0.0;
    var flooded = false;
    for (var t = 0u; t < min(terms, SHADOW_TERMS); t = t + 1u) {
        let cell = shadow_casters[who].cell[t];
        if !cell_packed(cell) {
            continue;
        }
        let map = shadow_casters[who].map[t];
        // Held inside the cell, so a quad reaching a hair past its own box
        // takes that cell's own empty border rather than the neighbour packed
        // beside it.
        let texel = clamp(map.xy + points * map.z, cell.xy + 0.5, cell.xy + cell.zw - 0.5);
        // One bilinear tap per term. A cell is drawn at a fraction of the
        // target's pixels once its σ is past `shadow::SIGMA_CELL_MAX`, and the
        // tap is what makes a blur wider than its own texels read smooth. A tap
        // of a DISTANCE field is a valid interpolation for the same reason a
        // tap of coverage is — the field is smooth almost everywhere and its
        // creases are where two answers are equally right.
        let held = textureSampleLevel(shadow_atlas, shadow_sampler, texel / atlas, 0.0).r;
        if shadow_casters[who].kind[t] >= 0.5 * DISTANCE_KIND {
            flooded = true;
            cov = standoff_coverage(held, 2.0 * shadow_casters[who].sigma[t], shape);
        } else {
            blur = blur + map.w * held;
        }
    }
    if flooded {
        // A distance row does NOT see the gain. The gain exists to push a
        // hairline's blur up to full depth, and a distance field already gives
        // a hairline full depth at its own edge by construction — a gain on top
        // of that is the plateau over the whole padded box that the family is
        // here to not draw.
        return clamp(cov, 0.0, 1.0);
    }
    // The GAIN, which is how much of the depth a caster thin against σ is
    // worth: a hairline's blur peaks at a fraction of 1, and without this its
    // shadow would land at a fraction of the depth the bar names. The
    // `min(…, 1)` is what keeps the depth a FLOOR — a caster wide against σ
    // saturates there rather than overshooting.
    return min(max(gain, 0.0) * clamp(blur, 0.0, 1.0), 1.0);
}

// How much of a shadow stands `d` points out from the ink, 0..=1, for a caster
// whose Shadow is `w` points wide — the standoff's own decay, windowed to
// exactly nothing at [`SHADOW_STOP`] widths.
//
// `exp(-TAIL u^shape)` and not a ramp: a ramp ending at the width ends at a
// closed contour of one radius, and a closed contour is the shape the eye picks
// out of a smooth field best however gently the ramp meets it. What ends the
// tail instead is the window, a couple of widths out, where the decay is under
// the eye's own threshold rather than at a fiftieth of its depth.
//
// `pow` is 0 at 0 for every exponent above it, so the coverage is exactly 1 at
// the ink whatever the shape bar says — the bar bends the profile between the
// two ends and moves neither.
fn standoff_coverage(d: f32, w: f32, shape: f32) -> f32 {
    let u = max(d, 0.0) / max(w, 1.0e-6);
    return exp(-SHADOW_TAIL * pow(u, max(shape, SHADOW_SHAPE_FLOOR)))
        * (1.0 - smoothstep(1.0, SHADOW_STOP, u));
}

// How many e-folds the decay has spent by one Shadow width, and how many widths
// out its window has shut — `SHADOW_TAIL` and `SHADOW_STOP` in
// harmonigraph_scene, pinned to them by `the_standoffs_window_is_the_scenes`.
const SHADOW_TAIL: f32 = 4.0;
const SHADOW_STOP: f32 = 2.0;

// The steepest the decay may be bent to, whatever a caller asks for.
//
// `pow` is an `exp2(y · log2(x))` on the hardware, so an exponent of 0 against
// the distance's own 0 at the ink is `0 · -inf` — a NaN coverage, and a NaN
// multiplying the frame is a pane gone with nothing on screen to say why. A
// floor here rather than only in the bar, so a hand-edited blob cannot reach it
// either.
const SHADOW_SHAPE_FLOOR: f32 = 0.05;

// The flattest a shadow's falloff may be bent to, whatever a caller asks for.
//
// The exponent acts on a number in 0..=1, so as it approaches zero every
// blurred fragment with any ink at all in it goes to `pow(x, 0)` = 1 and the
// shadow is a solid rectangle over the caster's whole padded box — the one
// value of the curve that draws a shape no caster has. A floor here rather than
// only in the bar, so a hand-edited blob cannot reach it either.
const SHADOW_CURVE_FLOOR: f32 = 0.05;

// What the Shadow depth's own bar bottoms out at: the share of the frame left
// under a caster's solid middle at the top of that bar.
//
// A floor and not a clamp on the depth, because the depth is spent as an
// EXPONENT over the blur — a number of stops — and zero has no number of stops
// under it. The top of the bar is a shadow ten stops deep rather than a hole to
// black.
//
// A 1024th and not a 255th, so that top is exactly black under a solid caster:
// the scene target is 8-bit, so a factor under half a code value of anything it
// can hold rounds the frame away, which is what the top of that bar says it does.
const SHADOW_KEEP_FLOOR: f32 = 0.0009765625;

// What a caster's kernel leaves of the frame under one fragment, 0..=1: `keep`
// raised to the exponent `shadow_kernel` came to, with the caster's LEVEL spent
// as a share of the result rather than inside the exponent.
//
// The whole of the arithmetic a caster's draw multiplies by, whatever the caster
// is — a node's rings, a resting cross, a name's box — so that one Shadow bar is
// one darkness. What differs between them is where the kernel is SAMPLED and
// which uniform the depth arrives in, and both stay with the caller.
//
// `full` arrives already in 0..=1 and already spent through whatever its own
// family owes — the gain on a blur row, the standoff's decay on a distance one
// (`shadow_kernel`). What is left here is the pair every row shares: how dark
// the shadow lands, and where along its width that darkness sits.
//
// The level is spent as a SHARE because `1 - level * (1 - T)` is a caster of
// opacity `level` letting the rest of the light straight through: a marker a
// tenth of the way in at the top of the depth bar casts a tenth of a shadow,
// where the same level inside the exponent would have it cast half
// (`SHADOW_KEEP_FLOOR` to the 0.1 is 0.5) — a shadow snapping on while its ink
// is barely there.
fn shadow_transmittance(full: f32, depth: f32, level: f32, curve: f32) -> f32 {
    let keep = max(1.0 - clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    // The CURVE, which is where along the shadow's width that depth sits. An
    // exponent on a number in 0..=1 holds both ends still — a saturated middle
    // stays at the depth, and a fragment the blur left at nothing stays at
    // nothing — and moves everything between them, so the bar bends the profile
    // without moving where the shadow starts or stops.
    let through = pow(keep, pow(full, max(curve, SHADOW_CURVE_FLOOR)));
    return 1.0 - clamp(level, 0.0, 1.0) * (1.0 - through);
}

// Where a point of the pane reads its caster's cell, in atlas texels: the cell's
// origin plus how far into the box the point stands, at the scale the packer
// related the two by (`pack` in shadow.rs).
fn cell_texel(points: vec2<f32>, rect: vec4<f32>, cell: vec4<f32>, k: f32) -> vec2<f32> {
    return cell.xy + (points - rect.xy) * k;
}

// Whether a caster's cell was packed at all. A cell the atlas had no room for is
// zeroed (`fits` in shadow.rs), and a zeroed cell sits at the ORIGIN — so a draw
// that filled it anyway would paint its ink over whatever cell IS packed there.
// Every fill draw asks this first and collapses its quad instead.
fn cell_packed(cell: vec4<f32>) -> bool {
    return cell.z > 0.0 && cell.w > 0.0;
}

// A quad with no area, off the viewport: what a fill draw emits for a cell the
// atlas had no room for.
fn no_quad() -> vec4<f32> {
    return vec4<f32>(2.0, 2.0, 0.0, 1.0);
}

// A cell texel as a clip position, for the draws that FILL the atlas, on an
// atlas `size` texels across.
//
// The caller's own `w` is carried through rather than replaced by 1: screen
// barycentrics survive an affine remap of the screen, so a quad remapped this
// way interpolates every varying exactly as the pane's own draw does, and the
// cell holds the caster's picture rather than a sheared copy of it. A caller
// drawing in plain screen space hands over the 1 it already has.
//
// The size is floored at one texel, an atlas being at least that wide wherever
// anything is packed at all: it is the divisor, and the branch that would have
// answered for a zero is [`cell_packed`], which every caller takes first.
fn cell_clip(texel: vec2<f32>, size: vec2<f32>, w: f32) -> vec4<f32> {
    let extent = max(size, vec2<f32>(1.0));
    return vec4<f32>(
        (2.0 * texel.x / extent.x - 1.0) * w,
        (1.0 - 2.0 * texel.y / extent.y) * w,
        0.0,
        w,
    );
}

// The two attachments the offscreen scene pass carries.
//
// `picture` is everything, and is what the composite puts on screen. `nodes` is
// the same picture with the node LABELS left out — the bright pass reads it, so
// a name neither glows nor takes a bite out of the halo of the node it covers.
//
// A label's ink reaches `picture` alone, its pipeline writing that attachment
// and no other; every other draw writes the same INK to both, so the ink of a
// name is the one thing the two pictures hold different amounts of. What they
// are otherwise allowed to differ in is a caster's SHADOW: a premultiplied
// fragment's alpha is what it takes off the frame UNDER it, so a deeper alpha
// here is the same item over a darker copy of the frame rather than a different
// item (lattice.wgsl's `Painted`, text.wgsl's `fs_shadow_box`) — `nodes`
// always at a whole shadow (1), whatever `picture`'s own depth is.
struct SceneOut {
    @location(0) picture: vec4<f32>,
    @location(1) nodes: vec4<f32>,
};
