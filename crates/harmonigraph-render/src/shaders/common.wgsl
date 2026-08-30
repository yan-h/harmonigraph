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

// How much blurred ink is a whole shadow.
//
// The blur of a caster's coverage is at most 1, and only deep inside a caster
// far wider than σ; a hairline ring or a stroke of type is a few pixels against
// a σ of several, so its blur peaks at a fraction of that and its shadow, spent
// as an exponent on `keep`, would land at a fraction of the depth the bar names.
// This is the factor that fraction is multiplied up by, and the `min(…, 1)`
// under it keeps the Shadow depth a true FLOOR: a caster wide against σ
// saturates there rather than overshooting, and the gain only deepens the thin
// ones.
//
// One constant and not a bar, calibrated by eye on a name at the fresh view
// (#498, PR B): at 1 a fresh name's shadow is a faint tint beside the ring's, at
// 4 a hairline casts as a block. A ring and a cross take the same number, which
// is what makes one Shadow bar one darkness across the picture.
const SHADOW_GAIN: f32 = 2.5;

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

// What a caster's blurred ink leaves of the frame under one fragment, 0..=1:
// `keep` raised to the blur, with the caster's LEVEL spent as a share of the
// result rather than inside the exponent.
//
// The whole of the arithmetic a caster's draw multiplies by, whatever the caster
// is — a node's rings, a resting cross, a name's box — so that one Shadow bar is
// one darkness. What differs between them is where the blur is SAMPLED and which
// uniform the depth arrives in, and both stay with the caller.
//
// Spent as a SHARE because `1 - level * (1 - T)` is a caster of opacity `level`
// letting the rest of the light straight through: a marker a tenth of the way in
// at the top of the depth bar casts a tenth of a shadow, where the same level
// inside the exponent would have it cast half (`SHADOW_KEEP_FLOOR` to the 0.1 is
// 0.5) — a shadow snapping on while its ink is barely there.
fn shadow_transmittance(blur: f32, depth: f32, level: f32) -> f32 {
    let keep = max(1.0 - clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    let through = pow(keep, min(SHADOW_GAIN * clamp(blur, 0.0, 1.0), 1.0));
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
// Every draw writes the same INK to both, so a label is the one thing the two
// pictures hold different amounts of. What they are otherwise allowed to differ
// in is a caster's SHADOW: a premultiplied fragment's alpha is what it takes
// off the frame UNDER it, so a deeper alpha here is the same item over a darker
// copy of the frame rather than a different item (`glow_shadow_bloom` —
// lattice.wgsl's `Painted`, text.wgsl's `fs_shadow_box`). With that bar at 0
// every fragment in the pass is identical in both.
struct SceneOut {
    @location(0) picture: vec4<f32>,
    @location(1) nodes: vec4<f32>,
};
