// Instanced billboard rendering of lattice nodes.
//
// Each instance is one node, expanded into a camera-facing quad in the
// vertex shader; the fragment shader draws the node's stack of rings, and a
// second pass lays that node's own light around them. Skins/effects iterate
// here: this file is the main thing to edit when trying a new look.

struct Uniforms {
    view_proj: mat4x4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    // x: unused — it carried the global clock in seconds, which nothing
    //    here reads: the shimmer was its one consumer, and it takes its
    //    travel pre-multiplied and pre-wrapped in misc8.x instead, an f32
    //    being unable to carry an hour-long song position finely enough to
    //    phase a band a fiftieth of a world unit wide. A second spelling of
    //    the same clock is how a later pattern would clock on the wrong one.
    // y: base node radius (world units),
    // z: how much two nodes' overlapping light adds up (`glow_meld`), 1
    //    screening the halos together and 0 leaving an overlap as bright as
    //    the brighter node alone. Away from the glow's own rows because
    //    blit.wgsl's composite mixes the same two blends and sees only the
    //    head of this buffer.
    // w: unused — it carried the node style, the paint of a disc at the
    //    node's centre, from when that disc had more than one. A retired
    //    slot rather than a repack, which would renumber the ones around it
    //    for nothing.
    misc: vec4<f32>,
    // x: darkest_pitch, y: brightest_pitch (MIDI notes); z: render scale
    // (offscreen pixels per screen pixel — converts the screen-pixel
    // softness knob to render pixels); w: bloom strength, which blit.wgsl
    // reads off this same buffer — NOT a free slot, whatever the fact that
    // nothing in this file touches it suggests. An octave glyph maps its
    // pitch through x/y to index pitch_lut.
    misc2: vec4<f32>,
    // x: unused — it carried the radius of a disc at the node's centre, the
    // one layer that was not a ring. Retired in place. y/z: the
    // outer layer's inner/outer band radii (same units; the scene guarantees
    // z > y where the band draws at all, and hands both as 0 where it does
    // not). w: the outer edge of the outermost RING the node draws — z, save
    // where the band is off and a ring inside it is the last one on — which is
    // what the marks stand off and what the billboard is sized on (`node_rim`).
    // It BOUNDS the clearing without describing it: `node_clearing` measures
    // each layer from that layer's own radii, so a hole is never read off w.
    misc3: vec4<f32>,
    // Pitch->color lookup for the octave glyphs. The disc is colored through
    // this same table on the CPU, so a glyph and the disc under it match
    // exactly rather than closely (length mirrors
    // harmonigraph_scene::PITCH_LUT_N).
    pitch_lut: array<vec4<f32>, 64>,
    // x: unused — it carried the solidity of a disc at the node's centre.
    // y: where the melody/bass strip starts. z/w: unused — they carried the
    // idle marker's radius and style, from when an unlit node drew a
    // placeholder of its own.
    misc4: vec4<f32>,
    // x: half a resting marker's arm thickness, as a share of one arm's
    // length; 1 is a filled square. y: where its arms start to taper, as a
    // share of one arm; 1 is a square end.
    // z: the node's ANGULAR padding, in quad UV units — the gap between two
    // neighbouring sectors, wherever sectors are drawn. The RADIAL padding is
    // a second setting and never arrives: every stand-off it buys is already
    // spent in the radii in misc3 and misc4.y, which is arithmetic done before
    // they get here. w: how deep the melody/bass marks reach past the ring
    // they stand off, same units; 0 = no marks.
    misc5: vec4<f32>,
    // x/y: unused — they carried the trail's mark style and strength, from
    //    when a memory was a change to the idle marker rather than a kept
    //    note name. z: the sevens knockout's fade width, read below by the
    //    vertex stage. w: the melody/bass marks' shimmer pattern
    //    (0 off, then one index per pattern; see Pulse::shader_index), read
    //    by mark_pulse — NOT a free slot.
    misc6: vec4<f32>,
    // The pane fill this pass is composited OVER: the surface BEHIND the
    // lattice, not the grey the lattice's own at-rest structure is drawn IN
    // (that is lattice_ground below, and the two are independent numbers).
    // Only the sevens knockout reads it: without it the gutter can knock out
    // only to black, which is darker than the pane and reads as a plate
    // sitting ON the picture rather than a hole THROUGH it.
    background: vec4<f32>,
    // The unlit ground a node's two rings stand on: one neutral grey, its
    // brightness the view's own Ground bar. A colour the lattice DRAWS,
    // where background above is only what it lands on — this grey is free to
    // sit either side of the pane's own fill, and at the bottom of the bar it
    // is black. Read by the OCTAVE band alone — a silent slice IS this
    // colour, and a sounding one's pitch is painted over it. The audio ring
    // beside it stands on the same grey by carrying it as entry 0 of
    // spectral_lut, baked on the CPU from the same L*.
    lattice_ground: vec4<f32>,
    // The wheel. x: octaves one turn is cut into; y: the MIDI pitch at the top
    // of every node.
    // Which SLOTS a node draws, and how far its ring is turned, are derived
    // per node from these — both depend on the node's pitch class, so there is
    // no one answer to send.
    // z/w: the audio ring's inner and outer radius, in quad UV units, both 0
    // when the ring is off. Beside the wheel because the ring IS the wheel at
    // smaller radii — same slices, same angles, same table of boundaries —
    // with the levels coming off the instance's second octave word instead of
    // its first.
    misc7: vec4<f32>,
    // The shimmer's knobs. x: how FAR the bands have travelled, in world
    // units, already reduced onto one cycle of the pattern — a clock and a
    // speed cannot be had here separately, and the reason is precision (see
    // `Scene::shimmer_slide`); y: how wide they are, in world units (the scene
    // floors it above zero — the band phase divides by it); z: how deep the
    // light they carry is, 0 none and 1 the tuned depth; w: how gradually it
    // arrives across the period, 0 a crest and 1 a cosine. See the Shimmer
    // section below.
    misc8: vec4<f32>,
    // The angle from a ring's own seam to each of its slice boundaries, four
    // to a row and read through oct_bound(): boundary j walking clockwise. One
    // table for every node, since the widths are the node's only in where they
    // are turned to. Computed on the CPU (harmonigraph_scene's
    // `octave_layout`) because it depends on settings alone — the alternative
    // is accumulating the same widths per pixel per sector.
    oct_bounds: array<vec4<f32>, 3>,
    // The audio ring's knobs. x: how many cents of the spectrum one wedge of
    // the ring spans, centered on that wedge's own octave, read only where y is
    // 0; y: 1 where each wedge is ONE reading taken at its own octave's pitch
    // (the FOLD) rather than a window of pitch spread across it; z/w unused.
    misc9: vec4<f32>,
    // The node glow's four dials. x: how far past a node's outermost drawn
    // edge its light spreads, in the node's own uv; y: how much light that is;
    // z: how flat the falloff is across that span, 0 the exponential heaped on
    // the node and 1 an even field across it (see `glow_layer`); w: how widely
    // a node's own ink is averaged into the colour of its light, 0 keeping each
    // layer's sectors distinct and 1 laying one tint over the whole halo.
    //
    // ZEROED WHOLE where the glow is off, by the CPU, so `u.misc10.x > 0.0` is
    // the one test anything here makes and no half-on state exists.
    //
    // Read by the light's own draw and the strip behind it (`vs_glow` sizes the
    // billboard, `glow_layer` shapes the light, `glow_blend_kappa` shapes its
    // colour). The light is written into a target of the glow's own and
    // composited at the BOTTOM of the scene pass, so a node's own picture — its
    // knockout included — is drawn over it rather than under it.
    misc10: vec4<f32>,
    // The STANDOFF's four dials. x: how far past each ring a node holds the
    // light off, in the node's own uv; y: how much of that is spent fading the
    // light back in, measured back from where the gap ends; z: how that fade is
    // skewed across its own width (see `standoff_coverage`), 0 giving the light
    // back closest to the ring and 1 holding the ring dark to the end of it;
    // w: how much of the light it takes where it stands, 1 clearing to the bare
    // ground. What the node's own INK takes of that same field is `misc13`, on
    // a bar of its own.
    //
    // Read by `fs_glow` alone, which writes the standoff into a layer of its
    // own beside the light (`glow_shade_tex`): the light is one field under the
    // whole lattice and this is a second field cut into it, so a node holds the
    // light off wherever that light reaches rather than only where the node
    // itself paints. Both readers of the light scale it by the same layer, so
    // nothing is ever erased out of the light target.
    //
    // ZEROED WHOLE with misc10, on the same rule and for the same reason:
    // there is one off switch for the glow and it is `u.misc10.x > 0.0`.
    misc11: vec4<f32>,
    // The node glow's plumbing row, which is not a dial. x: how many rows the
    // ink strip has — the row map's CAPACITY and not this frame's instance
    // count, rows being handed out per node and held for as long as that node's
    // light lasts. What `vs_ink_strip` and `vs_ink_blur` place a node's row
    // inside. y/z/w unused.
    //
    // A row of its own rather than the spare half of one above, because it
    // answers a different question: everything up there is a setting a person
    // dialled, and this is how tall a texture the renderer allocated. Zeroed
    // whole with them, on the same rule.
    misc12: vec4<f32>,
    // The WASH. x: how much of the light standing at a node's pixel washes over
    // the node's own INK (`glow_wash`), where `misc11.w` is the GROUND's share
    // of that same field. y: how brightly a resting marker lights the position
    // it stands at (`marker_light`); z: how far that light reaches from the
    // crossing, in WORLD units (`marker_span`); w: one quad uv as a world
    // length on the sheet the markers stand on (`marker_unit`), which is what
    // puts a marker's own draw and the Gap it stands the light off by into one
    // unit.
    //
    // A row of its own because it is not a term of the standoff, close as it
    // reads to the depth: the Gap bars shape the field the GROUND is painted
    // from, and this reads that same field raw, so a dial sitting among them
    // would carry the coupling it exists to break. Zeroed whole with misc10,
    // on the same rule.
    misc13: vec4<f32>,
    // The FREQUENCY color scheme's ramp: the analyzer's own gradient, the
    // table the spectrogram's cells and the Spiral pane's segments are read
    // off. Indexed by a LEVEL, where pitch_lut above is indexed by a pitch —
    // that is the whole difference between the plugin's two color schemes, and
    // the reason there are two tables here rather than one.
    spectral_lut: array<vec4<f32>, 64>,
    // The analyzer's loudness at every bucket of its pitch grid, a byte
    // apiece, sixteen buckets to a row. Read by the audio ring alone; see
    // `spectrum_at`.
    spectrum: array<vec4<u32>, 240>,
};

const TAU: f32 = 6.2831853;

// Billboard headroom past the octave band's outer edge (uv 1.0): the quad
// and its uv are both scaled by this, so the uv->world mapping is
// unchanged (disc, band, glyphs, glow all render identically) but there is
// margin out to this radius for things that live OUTSIDE the band -- the
// marks, which at the default band (outer 1.0) sit entirely out here.
// Costs a bit of fill (bigger quads, which alpha-blend and discard where
// empty).
const QUAD_MARGIN: f32 = 1.6;
// Where the octave layer's overflow past uv 1.0 -- the aa fringe of a band
// dialed right out to the edge -- finishes easing off, rather than being
// cut flat by the quad boundary. Pinned to what QUAD_MARGIN was when this
// fade was tuned, so widening the billboard for the marks doesn't
// quietly restyle the glyph edges.
const GLYPH_FADE_LIMIT: f32 = 1.3;
// The node glow's base amplitude at a node's centre, before the Strength bar
// scales it — what a strength of 1 lays down where the light is fullest.
//
// The amplitude is spent over a falloff spanning the node's outermost edge plus
// the whole Reach, so by the rim there is a fraction of it left. Tuned so the
// Strength bar reads: at 1 a node's middle is unmistakably lit and its halo
// plainly visible, at 2 — the bar's top — the middle saturates and the halo
// doubles.
const GLOW_BASE: f32 = 0.8;
// The two ends of the Feather bar, as the rate the light's exponential comes
// off at across one span (`glow_layer`). Mirrored in harmonigraph-scene, which
// draws the bar's own preview from its copy; the render crate's tests hold the
// two pairs to each other, so retune them here and there together.
//
// TIGHT is the accent: down to a fifth of its peak by half the span and to
// nothing well before the end of it, so the falloff is the whole shape and the
// node is plainly what the light belongs to.
//
// FLAT is the wash: a tenth off by half the span and a fifth by the far edge,
// where the window has the light near nothing anyway — a constant, as far as
// the eye is concerned. What shapes the light is then the window alone, full
// out to half the span and a smoothstep to nothing across the other half,
// zero-sloped at both ends and so with no rim in it at any width. A node in the
// middle of that is not lit more than the space around it, which is the whole
// difference between a light on a node and a field the node sits in.
const GLOW_FALLOFF_TIGHT: f32 = 3.0;
const GLOW_FALLOFF_FLAT: f32 = 0.25;
// The two ends of the Gap curve bar, as the exponent the standoff's decay is
// taken over across the Gap bar's fade (`standoff_coverage`). Mirrored in
// harmonigraph-scene on the Feather pair's terms above.
//
// RIND is the thin skin: the light is back almost the moment it leaves the ink
// and the rest of the width carries a haze, so the dark has no width of its
// own. PLAIN is the exponential, and it is the CEILING rather than a setting
// in the middle of a range, because an exponent of one is where this family
// stops being a decay and starts being a band.
//
// Exactly there, and the derivative says so. `exp(-T u^p)` has an interior
// inflection at `u = ((p-1)/(T p))^(1/p)`, which is real only for p above one:
// at or under one the coverage is steepest where it leaves the ring and
// shallower every step after, and over one it turns over at a radius that
// marches outward with the exponent — 0.07 of the fade at 1.2, a third of it
// at 2, two thirds at 4. That turn is a KNEE, and a knee at a fixed radius is
// the closed contour the eye picks out of a smooth field, which is the whole
// reason the standoff is a decay and not the ramp it replaced (see `GAP_TAIL`).
// Carry the exponent far enough and the family arrives back at that ramp
// exactly: the coverage goes to 1 across the whole fade and to `exp(-T)` at
// its end, which is a hard step at the outer handle.
//
// So the bar spends its whole length UNDER the exponential, a factor of four
// of it, and the light a node stands off never has an edge in it at any
// setting.
//
// RIND is a QUARTER rather than the 0 the family would allow. `pow` is an
// `exp2(y * log2(x))` on the hardware, so an exponent of 0 against the
// distance's own 0 at the solid end of the band is `0 * -inf` — a NaN
// coverage, and a NaN scaling the light a node clears to is a node whose
// clearing is gone with nothing on screen to say why. WHICH of the numbers
// above 0 is then a choice: the exponent has no floor of its own, and the
// picture goes on moving about as far per halving as the top of the bar does
// per quarter. A quarter of the ceiling is a range wide enough to take the
// shade off a ring altogether and short enough to stay a dial rather than a
// switch.
//
// What the low end costs is a slope: under an exponent of one the decay leaves
// the solid band vertically, and a vertical at a fixed radius is a crease —
// the one thing a fade laid over this light must not draw. It draws none at
// the pairing the bar ships in, where the fade is the whole width of its gap:
// the solid band has no width there, so what the vertical meets is the ring's
// own ink edge, and an edge laid along an edge is invisible. A fade NARROWER
// than its gap stands that crease out in the open field instead, and there the
// low end of the bar is a shape to avoid rather than one to reach for.
const GAP_SHAPE_RIND: f32 = 0.25;
const GAP_SHAPE_PLAIN: f32 = 1.0;
// How far down `standoff_coverage` is carried by the outer end of the Gap's
// fade, as the exponent of the decay there: four e-folds, so the outer handle
// stands at a fiftieth of the standoff and the rest of it is spent inside the
// band.
//
// The coverage under that handle is a DECAY and not a ramp landing on zero at
// it, which is what the number is for: a ramp ending AT the handle ends at a
// circle of one radius, and a closed contour is the shape the eye picks out of
// a smooth field best, however gently the ramp meets it — see `glow_layer`,
// where every other length in the light is an exponential and so has no such
// radius to find. What ends the tail instead is `GAP_STOP`, a couple of Gaps
// out, where the decay is under the eye's own threshold rather than at a
// fiftieth of the standoff.
//
// Four rather than the `ln(255)` that would put the handle under one code
// value of the target. A decay steep enough to be invisible at the handle is
// steep enough to spend the whole standoff in the first fifth of the band,
// which is the abrupt shadow this is here to not draw. What is left standing
// at the handle instead is a fiftieth, under the Gap depth, on a curve still
// falling — a gradient rather than an edge at any width.
const GAP_TAIL: f32 = 4.0;
// How many Gap widths out from a ring the standoff's own window has shut, and
// with it how far past every ring the two glow billboards reach
// (`glow_gap_stop`).
//
// A decay has no radius at which it stops and a quad does, so this is where
// that difference is settled rather than left to the billboard: the coverage is
// windowed to nothing at exactly the radius each quad is sized to, and there is
// no fragment where the two disagree. Left to the billboard the tail is cut at
// whatever value it happens to hold there, which is a hard step at a
// SCREEN-ALIGNED square — the standoff is a decay precisely to keep a closed
// contour out of a smooth field, and a square is a worse one than the circle it
// was avoiding.
//
// TWO, so the window is `glow_layer`'s exactly — full to half its span, shut by
// the end of it, zero-sloped at both — laid over the fade's outer handle at the
// half. Inside the Gap nothing moves at any exponent: the bar's two handles say
// what they have always said, and what this ends is the tail past them.
//
// Two rather than further out, and the EXPONENT is what decides that. The decay
// is under half a code value of the deepest light at `u = (ln(1/eps)/T)^(1/p)`,
// which for a shade of 0.5/255 at a depth of 1 is 2.0 at the plain exponential
// and 17.4 at GAP_SHAPE_RIND. So at the bar's ceiling the window falls entirely
// inside what was already invisible and the picture is unmoved, and at its
// floor a quad honest enough to hold the raw tail would be seventeen Gaps wide
// — most of a pane, at a Gap dialled anywhere near GLOW_GAP_MAX. The tail below
// the exponential is a shape worth having and its full support is not
// affordable; this is where the two are traded.
const GAP_STOP: f32 = 2.0;
// The darkest the standoff is allowed to leave the light, as the factor it
// keeps.
//
// A floor and not a clamp on the depth, because the factor is spent
// GEOMETRICALLY (`glow_shade`): the fade is a number of e-folds from here up to
// the light whole, and zero has no number of e-folds under it.
//
// A 1024th and not a 255th, so that the bar's top is still exactly the bare
// ground: the light is premultiplied and the target is 8-bit, so a factor under
// half a code value of the brightest light there can be rounds the whole pool
// to the frame with no glow in it — which is what the Gap depth's top says it
// is, and is asserted byte for byte
// (`the_gap_depth_says_how_much_light_a_ring_stands_off`). A 255th leaves that
// true only for a light dimmer than half of full.
const GAP_KEEP_FLOOR: f32 = 0.0009765625;
// How many angles a node's own ink is read at — the width of the strip that
// reading is kept in (`fs_ink_strip`), and the only rate at which anything
// about the colour of that node's light is resolved.
//
// Set against the TIGHTEST lobe the blur is ever asked for, GLOW_LOBE_KAPPA,
// whose angular spread is 1/sqrt(kappa): half a radian, near thirty degrees.
// One texel is 5.6 degrees, five of them to that spread, and a von Mises at
// that concentration has nothing left past the eighth harmonic — under a
// thousandth of its mean, and a millionth by the twelfth — so the blurred
// strip is resolved several times over.
//
// The other half of the headroom is for the ink BEFORE it is blurred: a node's
// sectors are crisp shapes and this is where they are sampled, so one texel is
// also the angular width their edges are softened over (`ink_at`). The rate
// and its prefilter are one number rather than two, which is what stops a wedge
// narrower than a texel from landing between two of them.
const INK_STRIP_N: u32 = 64u;

@group(0) @binding(0) var<uniform> u: Uniforms;

// A node's ink read round the node: one ROW per lit node, angle across it.
//
// Bound at THREE stages, and one declaration for all of them, the shape being
// the same every time; which strip a draw is looking at is the bind group it
// was recorded with:
//
//  - `fs_ink_strip` reads the raw strip IT wrote last frame, which is what a
//    node's colour is carried from (the pair ping-pongs — see `InkStrip`).
//  - `fs_ink_blur` reads the raw strip that pass has just written.
//  - the light's own draw reads the blurred one.
@group(1) @binding(0) var ink_strip: texture_2d<f32>;

// The finished light, as a node's own clearing reads it: the glow's target,
// composited at the bottom of the scene pass and sampled again here so that
// what a node clears to is the light standing at that pixel over the ground
// rather than the bare ground (`node_paint`).
//
// The SAME slot as the strip above, and that is what keeps both pipeline
// layouts as they are: no entry point reads both — the strip is the ink
// pass's and the light's own draw's, this is the node and marker draws' —
// and a binding collision is diagnosed per entry point, so the two share
// group 1's one texture slot instead of the node pipelines carrying a
// second, empty one. An entry point that ever wanted both would fail to
// compile, loudly, at pipeline creation.
//
// Read with `textureLoad` and never a sampler: the target is created at the
// scene's own pixel size, so it is 1:1 with the attachment being written and
// any filtering would be a blur nobody asked for. It also takes no
// derivative, which is what keeps it out of the early-out parity test's way.
@group(1) @binding(0) var glow_tex: texture_2d<f32>;

// The same light, blended with MAX rather than screened: an overlap here is
// exactly as bright as the brighter of the nodes making it. What a node's
// clearing actually reads is the two mixed by `glow_meld`, since a blend is
// fixed-function and a dial between two of them can only be had after both
// are written (`create_glow_pipeline` in harmonigraph-render).
//
// A slot of its own rather than the shared one above: this is read alongside
// `glow_tex` and not instead of it, which is exactly the case that trick does
// not cover.
@group(1) @binding(2) var glow_max_tex: texture_2d<f32>;

// The standoff field the light above is read THROUGH: how much of the light at
// this pixel the lattice's rings hold off, in x, melded across every sheet at
// once the way the light itself is (`fs_glow`).
//
// A layer of its own beside the light rather than a term inside it, because
// the two meld differently. Both blends the light is written under can only
// brighten and are commutative, so no order is legible in either; a shadow
// written into one of them would be an erase racing the light it is cut into,
// and what came out would depend on which node the instance buffer happened to
// hold first. Here the blend is `max` — the deepest standoff at a pixel wins,
// so two nodes' bands crossing do not compound into a pit — which is
// commutative in its own right and is the operator `glow_standoff` already
// combines ONE node's layers with.
//
// Read at both places the light reaches the picture, and both spell
// `light * (1 - shade)`: `node_paint` here, and blit.wgsl's `fs_glow_over`.
// That agreement is what makes a node's clearing repaint the field the
// composite laid down under it, on the same terms [`glow_light`] states for
// the Meld.
@group(1) @binding(3) var glow_shade_tex: texture_2d<f32>;

// The standoff: how far a node holds the light off every ring it draws, in the
// node's uv — 0 wherever the glow is off, `u.misc11` being zeroed whole there.
//
// A share of the node's radius, like the two gaps it sits with in the view, so
// it shrinks with a node off the home sheet. That is the right unit for it:
// what it stands off is the node's own layers, which shrink with the node too.
fn glow_gap() -> f32 {
    return max(u.misc11.x, 0.0);
}

// How far past a node's ink the standoff reaches AT ALL, in the same uv: the
// Gap, out to where `standoff_coverage`'s own window has shut it (`GAP_STOP`).
//
// What both glow billboards are sized to hold — `vs_glow` for a node's rings and
// `vs_plus_glow` for a marker's cross — so that every fragment the coverage is
// nonzero at is one the quad has, and the quad's own edge falls where the
// coverage is 0.
//
// Zero with the glow, `u.misc11` being zeroed whole there, and zero at a Gap
// DEPTH of 0, so it grows no quad where there is no standoff to hold. The depth
// is `glow_shade`'s own off switch and it is exact here for the same reason it
// is there — a depth of 0 keeps every scrap of the light by definition — which
// is what makes the depth the whole switch: with it at 0 the Gap moves not even
// a billboard, and the frame is byte for byte the one with no standoff in it at
// every width (`the_gap_depth_says_how_much_light_a_ring_stands_off`).
fn glow_gap_stop() -> f32 {
    return select(0.0, glow_gap() * GAP_STOP, glow_gap_depth() > 0.0);
}

// How much of the standoff is spent fading the light back in (`u.misc11.y`), in
// the node's uv and measured back from where the Gap ends — the same pair the
// view's Clearance is (`gutter_coverage`), and the same two-handle bar.
//
// A fade the whole width of the gap is the bar's low handle at 0: no solid
// band at all, the light coming off a ring at one steady rate from its edge.
// That is what a wide standoff wants, since a band of uniform width with a
// short edge, laid against a node's own dark rings, is what the eye reads as a
// painted black ring rather than as light that is not there.
//
// At 0 all that is left is the view's own Clearance fade under it, which is an
// edge a couple of screen pixels wide.
fn glow_gap_soft() -> f32 {
    return max(u.misc11.y, 0.0);
}

// How the standoff's fade is skewed across its own width (`u.misc11.z`), as the
// exponent `standoff_coverage` raises its decay to: 1 is a plain exponential
// and everything under it gives the light back nearer the ink, 0 handing it
// back inside the ink and leaving the rest of the width a haze.
//
// A geometric mix between the two ends, so equal drags are equal FACTORS: the
// bar's middle lands on half an exponent. A linear mix would spend most of the
// bar in the top quarter of the range, where the shapes are hardest to tell
// apart.
fn glow_gap_shape() -> f32 {
    let t = clamp(u.misc11.z, 0.0, 1.0);
    return GAP_SHAPE_RIND * pow(GAP_SHAPE_PLAIN / GAP_SHAPE_RIND, t);
}

// How much of the light the standoff takes away where it stands (`u.misc11.w`):
// 1 clears to the bare ground, and below it the rings sit in a dimmer pool of
// their own light.
//
// A share of the LIGHT and not of the picture — it scales what the node's
// clearing paints, and the node's own ink is `glow_wash`'s business. At 0 the
// clearing paints the field untouched, which is the picture with no standoff in
// it at all.
fn glow_gap_depth() -> f32 {
    return clamp(u.misc11.w, 0.0, 1.0);
}

// How much of the light standing at a pixel washes over the lattice's own INK
// there (`u.misc13.x`): 0 is ink drawn exactly as it is with the glow off, and
// 1 is the whole field laid over it. Every layer that draws ink over the light
// — a node's rings, marks and glyphs, and the resting markers between them —
// takes its share through `wash_over` below.
//
// The GROUND's share of that same field is `glow_gap_depth` above, and the two
// are independent by construction — this reads the field RAW, before the
// standoff's factor — so a ring can stand in a pool cleared to the bare ground
// and still wear the colour of the halo around it. One coupled dial cannot say
// that, which is the whole reason this is a second one.
fn glow_wash() -> f32 {
    return clamp(u.misc13.x, 0.0, 1.0);
}

// The Wash bar's share of `light`, laid over ink already premultiplied by
// `alpha`. Every piece of the lattice's ink takes the light through here — a
// node's rings, marks and glyphs (`node_paint`) and a resting marker
// (`plus_paint`) — so one bar means one operation and not two that agree.
//
// A SCREEN, where the ground under the ink takes an over, and the difference is
// what a NEIGHBOUR's light is allowed to do. The field is melded, so the light
// at a piece of ink carries every sheet's halo; an over (`ink * (1 - light.a)`)
// lets a saturated halo from behind take the ink's other channels DOWN — a
// white name under a red one comes out red — which is ink losing its colour to
// something it stands in front of. `w + ink * (1 - w)` per channel can only
// brighten, whatever reaches the pixel. The ground keeps the over: that is
// `fs_glow_over`'s own blend state, and the ground a clearing paints has to be
// the ground it meets at the clearing's edge.
//
// Premultiplied, so the ink's own term carries `alpha`: this is the screen of
// the ink over `w`, scaled by the coverage the ink has, and whoever fills the
// `(1 - alpha)` it leaves takes its own light separately.
//
// What this does NOT give ink is a way to hold off a NEIGHBOUR's light: the
// field is one layer, so washed ink is tinted by a far sheet's halo as well as
// by its own, light reaching through it from behind. Interleaving the sheets is
// the only answer to that and is the thing this design exists to not do; a
// node's own halo is the maximum at its own pixel, the falloff being measured
// from its centre, so the far share is small unless a lit node sits directly
// behind.
fn wash_over(ink: vec3<f32>, alpha: f32, light: vec3<f32>) -> vec3<f32> {
    let w = light * glow_wash();
    return w * alpha + ink * (1.0 - w);
}

// How brightly a resting marker lights the position it stands at
// (`u.misc13.y`): the pool it sits in, where the cross itself is drawn in the
// marker's own ink. 0 is a marker that is ink and nothing else.
//
// Its own bar and not a share of that ink, because the two are read at
// different distances: ink is a shape AT the position, and a pool is a presence
// around it that carries. A picture too dark to find a cross in can still be
// dialled to say where the cross is.
fn marker_light() -> f32 {
    return clamp(u.misc13.y, 0.0, 1.0);
}

// The node's own outermost feature in ANY direction: a MARK where this node is
// wearing one — both marks ride the strip just past the outermost ring — and
// the outermost ring's own edge (u.misc3.w, the stack's cursor) where it is not.
// The ring is ordinarily the octave band; on a node whose band is dialled off it
// is whichever layer inside it the stack ended on, which is why the edge is
// handed in rather than read off the band's radii.
//
// A circle the whole node fits inside, which is what a billboard and an early-
// out want. What SHAPE it fills that circle with is `node_clearing`, and the two
// answer differently on three things: a mark is one wedge, so a marked node
// reaches its strip in that wedge's direction alone; a layer this node draws
// nothing of clears nothing, where the circle holds room for it either way; and
// the circle is filled where the hole is a set of annuli, so the node's own
// middle is inside this bound and outside every term of that. All three are a
// bound being loose, which costs a little fill and no correctness.
fn node_rim(marked: bool) -> f32 {
    var rim = max(u.misc3.w, 0.0);
    if marked && u.misc5.w > 0.0 {
        rim = max(rim, u.misc4.y + u.misc5.w);
    }
    return rim;
}

// The rim a node's LIGHT is measured against: the same two answers `node_rim`
// chooses between, with the mark's share of the choice CARRIED (Instance::glow
// w, `panes::glow_fade` in harmonigraph-ui) rather than switched by the bit.
//
// The light's whole span is this plus the Reach, so reading the bit put a step
// in it: the bit is set while the marking voice exists and clear the frame it
// is pruned, one Fade after the key came up, and the halo — still near full,
// with seconds of its own release to run — jumped a size smaller in one frame.
// A light is the slow part of the picture in its size exactly as in its
// brightness, and this is where the two are made to agree.
//
// Interpolating the two rims rather than the mark's own drawn width, because
// what the light needs is one length to lay its falloff over, and a mark is a
// wedge: its width is a direction the node reaches in, not a circle it fills.
// The pair are the circle with the mark and the circle without, and the light
// eases between them.
fn glow_rim(inst: Instance) -> f32 {
    return mix(node_rim(false), node_rim(true), clamp(inst.glow.w, 0.0, 1.0));
}

// How far the billboard has to reach, in uv, for a clearing of reach `g` around
// a node reaching `rim` to finish inside it rather than being clipped square at
// the corners. Measured on the circle the node fits inside, so a clearing that
// bulges out over one wedge alone finishes inside it too. Never smaller than
// QUAD_MARGIN, so a node without a gutter — and every node on a lattice with no
// depth — is sized exactly as before.
fn quad_margin(rim: f32, g: f32) -> f32 {
    return max(QUAD_MARGIN, rim + g + 0.05);
}

// Whether the fragment shader may stop early where it can prove it would
// paint nothing (see `paint_reach` and the idle branch in `fs_main`). Only
// ever false in the parity test, which compiles a second pipeline with this
// flipped and requires the two to render the same pixels — the early-outs
// are an optimization, and the test is what keeps them one.
const EARLY_OUT: bool = true;

// Where a clearing of reach `reach` ends, as a distance from the footprint it
// is measured around — the reach itself, with a floor under it so that a
// smoothstep across it is a band rather than a step answering a half everywhere.
//
// One function because THREE places have to agree on it exactly: `paint_reach`
// just below, `gutter_coverage`, and the idle branch in `node_paint`, the two
// of which discard the fragments past this. A second spelling of the floor in
// either is a hairline of clearing dropped on one pipeline and kept on the
// other, which is precisely what the parity test compares.
//
// It is load-bearing beyond the fade, too: `gutter_coverage` clamps into
// [0, edge - 0.001], and a `clamp` whose low bound exceeds its high one is
// undefined in WGSL. This floor is what keeps that range from inverting.
fn clearing_edge(reach: f32) -> f32 {
    return max(reach, 0.001);
}

// How far from the node's center anything can paint, in its own uv.
//
// The billboard is deliberately bigger than the node: QUAD_MARGIN of
// headroom for the marks and a soft glyph's overflow, more when a
// gutter has to finish inside it. Between that circle of content and the
// square quad lies a lot of fragment — most of a quad, once the corners are
// counted — where every layer below computes its coverage, arrives at zero,
// and blends nothing. On a zoomed-in lattice, where one node can cover the
// pane, that is the frame's dominant cost.
//
// Every term here is the radius at which the corresponding layer's own
// smoothstep has reached zero, so the bound is exact rather than generous:
//
//   - the octave glyphs (and their eased-off fringe) end at
//     GLYPH_FADE_LIMIT;
//   - the marks taper off at QUAD_MARGIN, but only exist while a slot
//     is marked;
//   - the knockout clears to `clearing_edge` past the node's body, which fits
//     inside the circle of radius rim, so rim + that bounds it in every
//     direction and meets it in the one the marks reach. The edge rather than
//     the raw reach because a bound that stops short of what the coverage
//     paints is a discarded fragment the slow pipeline fills.
fn paint_reach(in: VsOut, aa: f32) -> f32 {
    var reach = GLYPH_FADE_LIMIT;
    if in.marks.x != 0u || in.marks.y != 0u {
        reach = max(reach, QUAD_MARGIN);
    }
    if in.gutter > 0.0 {
        reach = max(reach, in.rim + clearing_edge(in.gutter));
    }
    return reach;
}

struct Instance {
    @location(0) world_pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    // x: activation 0..1. y/z: the melody and bass marks' own levels, which
    // follow the marked voice rather than this node's activation — each
    // ring eases in over the scene layer's attack when its note takes that
    // end, and drops to 0 the frame the key comes up.
    @location(2) params: vec3<f32>,
    // Per-octave activation, 8 bits per slot, little-endian packed: how much
    // of that octave is HELD, and nothing else. The analyzer never writes here
    // — its reading is the audio ring's own channel (u.spectrum), a window
    // onto one grid the whole lattice shares rather than a level per node — so
    // this is the MIDI picture whole and is painted off pitch_lut throughout.
    @location(3) octaves: vec3<u32>,
    // The node's pitch class in cents (0..1200). It both PLACES the octave
    // indicators and COLORS them, off the one quantity: each indicator's
    // octave has a pitch, that octave's C plus this, and the indicator sits
    // at that pitch's angle on the shared axis (see oct_sector) in that
    // pitch's color.
    @location(4) cents: f32,
    // Melody/bass marks: x = melody slots, y = bass slots, one bit per
    // octave slot. Which SECTOR each mark continues outward (see
    // mark_extension); its fade level rides params.y/params.z.
    @location(6) marks: vec2<u32>,
    // Each mark's own color: its own sector's pitch off the ramp, with no lift
    // on top of it (see NodeInstance::melody_color), so a mark reads as the
    // indicator it extends rather than as a fixed livery.
    @location(7) melody_color: vec4<f32>,
    @location(8) bass_color: vec4<f32>,
    // The sevens layer: x = billboard size factor (1 on the home sheet,
    // smaller with every step off it), y = knockout gutter width, in the uv
    // units of a FULL-SIZE node — the vertex shader divides by the size
    // factor so the gap comes out the same width whatever the node's size.
    // 0 means no gutter. See ViewConfig::sevens_size / _gutter.
    @location(10) sevens: vec2<f32>,
    // How much of the audio ring this node wears, 0..1: the gate the view sets
    // answered against this node's own wedges, carried on the note Fade and
    // floored by the node's envelope. The CPU decides it
    // (harmonigraph_scene's RingGate and RingFade, against the same u.spectrum
    // this shader reads), so the ring's annulus is the layer's own off switch
    // and this is which NODES the layer is on at, and how far.
    @location(11) ring: f32,
    // The node's own light: x how bright it is, y which ROW of the ink strip
    // keeps its colour, z how much of this frame's reading the two of them
    // take, w how much of a MARK the light still has this node wearing. All
    // four are settled on the CPU, where a node has an identity that outlives a
    // frame (`panes::glow_fade` in harmonigraph-ui).
    //
    // The level is CARRIED and not the largest envelope on the node, which is
    // the whole point of it: a light runs on a clock of its own, so it is above
    // zero on a node whose every layer has gone silent, and such a node is
    // shipped for exactly that reason. The mix is 1 where the row is new — a
    // strip just built, or a row just handed over — and there is nothing to
    // carry from. The mark is carried for the same reason the level is: it is
    // the light's SIZE (`glow_rim`), and the bit it is carried from steps the
    // frame the marking voice is pruned.
    @location(12) glow: vec4<f32>,
    // WHICH HALF of the node this instance draws. A home node is drawn twice
    // with the cross standing at its position between: its knockout first, so
    // the hole hides the sheets behind and the nodes beside it without taking
    // that cross with them, then its ink (`from_scene`). Every other sheet
    // draws WHOLE.
    //
    // The split is EXACT rather than close, being one premultiplied over
    // factored into two: `ground*g` at alpha `g`, then `ink` at alpha `a`,
    // composites to the `ink + ground*g*(1-a)` at alpha `a + g*(1-a)` a single
    // draw writes.
    @location(13) paint: f32,
};

// What `Instance::paint` says. A float because it crosses as a vertex attribute
// either way.
const PAINT_WHOLE: f32 = 0.0;
const PAINT_INK: f32 = 1.0;
const PAINT_CLEARING: f32 = 2.0;

struct VsOut {
    @builtin(position) clip_pos: vec4<f32>,
    @location(0) uv: vec2<f32>, // -1..1 across the quad
    @location(1) color: vec4<f32>,
    @location(2) params: vec3<f32>,
    @location(3) @interpolate(flat) octaves: vec3<u32>,
    @location(4) @interpolate(flat) cents: f32,
    // Which ROW of the ink strip is this node's — the row the light's own clock
    // handed it, which it keeps for as long as its light lasts. NOT the index
    // in the instance buffer, which is sorted by depth and culled and so moves
    // under a node between frames; the row is where last frame's colour is read
    // back from, so a row that moved would be a node taking on a stranger's
    // ink.
    @location(5) @interpolate(flat) strip_row: f32,
    @location(6) @interpolate(flat) marks: vec2<u32>,
    @location(7) @interpolate(flat) melody_color: vec4<f32>,
    @location(8) @interpolate(flat) bass_color: vec4<f32>,
    // Already converted to THIS node's uv (see vs_main).
    @location(10) @interpolate(flat) gutter: f32,
    // The circle the node fits inside (`node_rim`) and the clearing's fade
    // width, both in this node's uv — computed once in the vertex shader
    // because the billboard is sized on the first and the second, like the
    // reach beside it, is a width on SCREEN and so depends on the node's size.
    // What the clearing is actually shaped like is settled per fragment
    // (`node_clearing`); this bounds it.
    @location(11) @interpolate(flat) rim: f32,
    @location(12) @interpolate(flat) soft: f32,
    // Where this fragment sits on the plane the billboards face, in world
    // units: its world position resolved onto the camera's own right/up
    // axes. Every billboard faces that same plane, so this is ONE coordinate
    // system spanning the whole lattice — which is what lets the shimmer be
    // a single sheet of bands crossing node after node instead of a copy per
    // node. Interpolated rather than flat for the same reason: a band has to
    // cross a node, not step from one to the next.
    @location(13) field: vec2<f32>,
    // How much of the audio ring this node wears (see Instance::ring), which
    // multiplies the ring's coverage and nothing else on the node.
    @location(14) @interpolate(flat) ring: f32,
    // The node's light: x how bright it is, y how much of this frame's ink its
    // row takes (see Instance::glow), z the rim the LIGHT is drawn against in
    // this node's uv (`glow_rim`). Read by the ink strip and the light's own
    // draw, and by nothing else on the node.
    //
    // The rim rides here rather than in a location of its own because there is
    // no location left: a vertex stage may hand on sixteen, and `rim` beside it
    // is the last of them. It belongs with these two anyway — all three are
    // what the light carries, and `rim` is what the NODE is measured against.
    @location(15) @interpolate(flat) glow: vec3<f32>,
    // Which half of the node this draw is painting (see `Instance::paint`).
    @location(9) @interpolate(flat) paint: f32,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, inst: Instance) -> VsOut {
    return node_vertex(vertex_index, inst, 0.0, false);
}

/// The GLOW's billboard: the same node, on a quad grown to hold everything this
/// pass writes. Two lengths reach past the node, and the quad is sized to the
/// larger — `glow_layer` shuts the window at the light's own rim ([`glow_rim`])
/// plus the Reach, and [`glow_shade`] stands the light off every ring out to
/// where the standoff's own window has shut ([`glow_gap_stop`]). The margin
/// below is that with room to spare.
///
/// The GAP is in it because the standoff is answered for the LIGHT AT A PIXEL
/// rather than for this node's own light: a node holds its rings off a
/// neighbour's halo out where its own light has shut, which is the whole reason
/// [`fs_glow`] cannot leave on `glow_level` alone. The two ceilings are
/// independent, so a Gap dialled past the Reach reaches past a quad sized to the
/// Reach, and what a bound cuts there is a fade partway down its ramp — a step
/// on a straight line, and a screen-aligned one, the quad being built from
/// `cam_right`/`cam_up`.
///
/// A second entry point rather than a wider `vs_main`, because the margin is
/// what every fragment of the node draw is measured against: growing that quad
/// would spend one more ring of discarded fragments per node for a reach only
/// the glow paints in. The uv is scaled with the quad, so uv 1.0 is the same
/// world distance either way and nothing inside the node moves.
@vertex
fn vs_glow(@builtin(vertex_index) vertex_index: u32, inst: Instance) -> VsOut {
    return node_vertex(vertex_index, inst, max(max(u.misc10.x, 0.0), glow_gap_stop()), true);
}

/// One node's billboard, with `extra` uv of headroom past what the node itself
/// needs — 0 for the node draw, and for the glow draw whichever of the Reach and
/// the Gap reaches further (see [`vs_glow`]). `light` says which of the two rims
/// sizes the quad ([`glow_rim`]); both are handed on either way, since the
/// fragment stages of one draw never read the other's.
fn node_vertex(vertex_index: u32, inst: Instance, extra: f32, light: bool) -> VsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];

    // Notes, hover, and distance from the camera all leave a node's size
    // alone, so a note changes only brightness and glow. The ONE thing that
    // sizes it is which sevens sheet it sits on (inst.sevens.x, 1 on the
    // home sheet): the home sheet is the ground the music is read against,
    // so sheets off it draw smaller — in both directions, since that is
    // distance from the ground and not depth toward the eye. The uv is
    // deliberately NOT scaled with it, so every layer inside the node keeps
    // its proportions and only the node's size on screen changes. (The quad
    // is twice the disc radius to leave room for the glow, plus QUAD_MARGIN
    // for the outer glyphs' soft edge — see QUAD_MARGIN.)
    let scale = max(inst.sevens.x, 0.05);
    // The gutter is a constant width ON SCREEN, not a share of the node.
    // uv is the node's own coordinate system, so it shrinks with the node —
    // which meant a half-size node cleared a half-size gap, and the gap read
    // as a property of the note rather than of the layer it sits on. Dividing
    // by the scale converts the setting from "of this node" back to "of a
    // full-size node", i.e. one fixed distance everywhere.
    let gutter_uv = max(inst.sevens.y, 0.0) / scale;
    let rim = node_rim((inst.marks.x | inst.marks.y) != 0u);
    let lit_rim = glow_rim(inst);
    // ...which can want more room than the standard billboard has, on the
    // smallest sheets. Only then does the quad grow: uv 1.0 still maps to
    // the same world distance either way, so nothing about the node's own
    // content moves.
    // The glow's quad is sized off the wider of the two rims. Its light is
    // measured against the one the LIGHT carries; taking the max with the
    // node's own is what keeps the quad a bound whichever way round the two
    // are, and a mark arriving on a node already lit puts the node's rim
    // ahead of the light's for the whole of the light's attack. What that
    // costs while it lasts is the ring of discarded fragments between them,
    // which is what a bound is for.
    let margin = quad_margin(select(rim, max(rim, lit_rim), light), max(gutter_uv, extra));
    let radius = u.misc.y * 0.90 * 2.0 * margin * scale;

    let world = inst.world_pos
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * radius;

    var out: VsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner * margin;
    out.color = inst.color;
    out.params = inst.params;
    out.octaves = inst.octaves;
    out.cents = inst.cents;
    out.strip_row = inst.glow.y;
    out.glow = vec3<f32>(inst.glow.x, inst.glow.z, lit_rim);
    out.marks = inst.marks;
    out.melody_color = inst.melody_color;
    out.bass_color = inst.bass_color;
    out.gutter = gutter_uv;
    out.rim = rim;
    out.ring = inst.ring;
    out.paint = inst.paint;
    // The shimmer's shared coordinate — see VsOut::field. Taken off the
    // CORNER's world position rather than the node's center, so the field
    // varies across the quad and the interpolator hands the fragment shader
    // the real plane position of every pixel.
    out.field = vec2<f32>(dot(world, u.cam_right.xyz), dot(world, u.cam_up.xyz));
    // The fade is a constant width on screen too, so it converts the
    // same way the reach does.
    out.soft = u.misc6.z / scale;
    return out;
}

// Octave indicator slots the packing carries: MIDI octaves -1..=9, so slot =
// octave + 1 and slot s is the octave whose C is MIDI 12*s. Which of them a
// node DRAWS is the span setting and its own pitch class — see oct_ring. A
// ring at the pitch limits reaches for slots outside this, which draw as
// backdrop and never light; oct_slot_level is what holds the lookup inside.
// (octave_level unpacks without that guard, and is not the one to reach for.)
const OCTAVE_SLOTS: u32 = 11u;

// Octaves one turn can be cut into (harmonigraph_scene::MAX_SPAN). One fewer
// than the boundary table's entries, since a slice needs a boundary each end.
const MAX_SPAN: u32 = 11u;

// Length of the pitch->color LUT (mirrors harmonigraph_scene::PITCH_LUT_N
// and the `pitch_lut` array in Uniforms).
const PITCH_LUT_N: u32 = 64u;

// The analyzer's pitch grid, as the audio ring reads it: how many buckets
// there are, how many of them a semitone spans, and the MIDI pitch the first
// one starts from (20 Hz). Mirrors harmonigraph_scene::SPECTRAL_BUCKETS,
// ::SPECTRAL_BUCKETS_PER_SEMITONE and ::SPECTRAL_AXIS, which
// harmonigraph-render asserts against these.
const SPECTRUM_BUCKETS: u32 = 3828u;
const BUCKETS_PER_SEMITONE: f32 = 32.0;
const SPECTRUM_MIN_MIDI: f32 = 15.486820;

// Coverage of `x` inside the threshold `edge`, with a screen-constant
// soft band: `w` is ~a pixel expressed in `x`'s units (from fwidth at
// the call site — taken at the top of the fragment fn, outside any
// non-uniform control flow). Fixed-width smoothstep edges blur as a
// quad grows on screen and alias as it shrinks; this keeps every shape
// edge equally soft at all zooms. The glow deliberately keeps its
// proportional falloff.
fn aa_inside(edge: f32, x: f32, w: f32) -> f32 {
    return 1.0 - smoothstep(edge - w, edge + w, x);
}

// Half-width of the soft band, in SCREEN pixels on each side of an edge
// (the one knob for how soft shape edges feel; softness stays
// screen-constant at every zoom). ~4px total on a Retina surface.
// aa_width() converts to render pixels via the render scale, so the
// super/sub-sampling view setting changes resolved detail without
// changing how soft edges look.
const AA_SOFTNESS_PX: f32 = 2.0;

// Soft-band width in the units of a coordinate whose per-render-pixel
// derivative is `coord_fwidth`: the softness knob, converted from screen
// pixels to render pixels.
fn aa_width(coord_fwidth: f32) -> f32 {
    return max(coord_fwidth, 1e-4) * AA_SOFTNESS_PX * max(u.misc2.z, 0.01);
}

// Activation level (0..1) of octave slot `i`, unpacked from 8-bit fields.
// Each octave carries its OWN envelope so indicators fade independently
// (a released C5 decays even while C4 holds the node fully lit).
fn octave_level(octaves: vec3<u32>, i: u32) -> f32 {
    let word = octaves[i / 4u];
    return f32((word >> ((i % 4u) * 8u)) & 0xFFu) / 255.0;
}

// ---- Where the octave indicators sit ---------------------------------------
// The wheel is SPAN octaves to the turn with the CENTER pitch straight up, and
// every node draws the span octaves of ITSELF nearest that center. Three
// things follow, and they are the whole layout:
//
//   - a slice is exactly one octave, so with no extras it is exactly a turn
//     over the span — on every node, whatever its pitch class;
//   - the center pitch is straight up on every node too, which is what makes
//     the top of the picture mean one pitch across the whole lattice;
//   - a node whose class sits d semitones from the center's has its whole ring
//     TURNED by d — left below, right above, never more than half a slice —
//     which is what puts each of its slices at the angle of the pitch it
//     stands for.
//
// What moves with the turn is the seam, the point where the ring's lowest
// slice meets its highest and the pitch wraps a whole span of octaves. It is
// at the bottom for the center's own pitch class and turns away from it with
// everything else.
//
// The widths are computed on the CPU (harmonigraph_scene's `octave_layout`)
// and read out of `oct_bounds` here — one table, shared by every node, giving
// the angle from a ring's own seam to each of its slice boundaries.

// The span, held inside the boundary table it indexes (oct_walk reads
// bound(span)), so a stale or oversized uniform draws a wrong sector rather
// than reading past the last row.
fn oct_span() -> u32 {
    return clamp(u32(u.misc7.x), 1u, MAX_SPAN);
}
// The MIDI pitch at the top of every node's wheel.
fn oct_center() -> f32 {
    return u.misc7.y;
}
// Straight up, in these angles: the bottom of a node is a quarter turn back
// from zero and clockwise — the direction pitch rises — subtracts.
const OCT_UP: f32 = -0.75 * TAU;
// Boundary `j` of a ring, four to a uniform row. j runs 0..span: 0 is the seam
// and span is the same seam a full turn on.
fn oct_bound(j: u32) -> f32 {
    return u.oct_bounds[j / 4u][j % 4u];
}
// Angle from a ring's seam to `x` slices along it, walking clockwise. Linear
// inside a slice, so a pitch stands at the same fraction of its own octave's
// wedge as it does of the octave.
fn oct_walk(x: f32) -> f32 {
    let c = clamp(x, 0.0, f32(oct_span()));
    let j = min(u32(max(floor(c), 0.0)), oct_span() - 1u);
    return mix(oct_bound(j), oct_bound(j + 1u), c - f32(j));
}
// MIDI pitch of octave slot `s` on a node whose pitch class is `cents`: slot
// s is the octave whose C is MIDI 12*s. Signed, since a ring at the pitch
// limits names slots the packing has no room for.
fn oct_slot_pitch(s: i32, cents: f32) -> f32 {
    return f32(s) * 12.0 + cents / 100.0;
}
// Where one node's ring sits: the slot of its LOWEST slice, and the angle of
// the seam the walk starts from.
struct OctRing {
    base: i32,
    seam: f32,
}
// Derived per node (harmonigraph_scene's `ring`) — where its class falls
// against the center is what decides both which octaves it draws and how far
// it is turned. Per node is also as often as it is worth deriving, so callers
// take it ONCE per fragment and hand it down; nothing below recomputes it per
// slot, per sector or per edge, which is what it would come to inside the
// loops.
fn oct_ring(cents: f32) -> OctRing {
    let off = cents / 100.0;
    let span = i32(oct_span());
    // The node's octave nearest the center, and how far above the center it
    // sits. Halves round up, so a node exactly a tritone away counts as the
    // half octave ABOVE.
    let nearest = floor((oct_center() - off) / 12.0 + 0.5);
    let d = nearest * 12.0 + off - oct_center();
    // The span octaves nearest the center: symmetric when the span is odd, and
    // when it is even one octave deeper on the side of the node's nearest
    // octave the center itself sits, with a tie going down.
    var low = -(span - 1) / 2;
    if (span % 2) == 0 {
        low = select(-span / 2, 1 - span / 2, d < 0.0);
    }
    var ring: OctRing;
    ring.base = i32(nearest) + low;
    // Turned so the CENTER pitch lands straight up. Solved for rather than
    // derived from the ring's middle: with extras the slice the center falls
    // in is not one span-th of the turn, and the pitch sits at its own
    // fraction of whatever width that slice has.
    let along = (oct_center() - oct_slot_pitch(ring.base, cents)) / 12.0 + 0.5;
    ring.seam = OCT_UP + oct_walk(along);
    return ring;
}
// The two angular edges of slot `s`'s indicator, in the order the wedge tests
// below want them: x the counter-clockwise edge, y the clockwise one. Exactly
// its own octave's ends, at every slot — nothing is cut to fit, which is what
// keeps the indicators meeting edge to edge and closing the ring.
fn oct_sector(s: i32, ring: OctRing) -> vec2<f32> {
    let i = u32(clamp(s - ring.base, 0, i32(oct_span()) - 1));
    return vec2<f32>(ring.seam - oct_bound(i), ring.seam - oct_bound(i + 1u));
}
// Where an indicator "points": the angle of its own pitch, which is the middle
// of its wedge — for anything that needs one angle for the whole of it rather
// than its two edges.
fn oct_mid(s: i32, ring: OctRing) -> f32 {
    let e = oct_sector(s, ring);
    return 0.5 * (e.x + e.y);
}
// The level of slot `s`, or nothing when the ring names an octave the packing
// has no room for: a ring near the pitch limits draws octaves no note can
// reach, and those are backdrop that never lights.
fn oct_slot_level(octaves: vec3<u32>, s: i32) -> f32 {
    if s < 0 || s >= i32(OCTAVE_SLOTS) {
        return 0.0;
    }
    return octave_level(octaves, u32(s));
}
// Whether a pixel's angle falls inside sector `edges`'s own arc (between its
// two boundaries), 0..1 with a soft edge over `aa`. `outer_glyph`'s test for
// which wedge owns a pixel, and so — through it — the octave indicator's arc
// and its mark extension's alike, which is what makes them one wedge rather
// than two constructions that could drift.
//
// A wedge under a half turn is the INTERSECTION of its two half-planes; one
// PAST a half turn is their union, and reading it as an intersection would
// empty the sector instead of filling it — see `outer_glyph`'s own note on
// `an_indicator_can_pass_a_half_turn_but_never_a_whole_one`.
fn oct_arc_coverage(edges: vec2<f32>, uv: vec2<f32>, aa: f32) -> f32 {
    let b1 = vec2<f32>(cos(edges.x), sin(edges.x));
    let b2 = vec2<f32>(cos(edges.y), sin(edges.y));
    let c1 = uv.x * b1.y - uv.y * b1.x;
    let c2 = uv.x * b2.y - uv.y * b2.x;
    let s1 = smoothstep(-aa, aa, c1);
    let s2 = smoothstep(-aa, aa, -c2);
    return select(s1 * s2, 1.0 - (1.0 - s1) * (1.0 - s2), edges.x - edges.y > TAU * 0.5);
}

// The melody/bass marks' shimmer pattern (u.misc6.w — see `Pulse`).
fn pulse_marks_mode() -> u32 {
    return u32(u.misc6.w + 0.5);
}

// ---- Shimmer: one sheet of soft light over the whole lattice --------------
// Every pulse mode but 0, and the same animation in each: a pattern of light
// laid over the layer, travelling. What the mode picks is its SHAPE.
//
// WHICH pixels it covers is not quite the same question. The sheet is the
// marks', and it takes the octave slices those marks extend (`mark_slice` in
// `fs_main`) as well as the extensions themselves, because a mark is one slice
// in two pieces rather than the outer piece alone. The rest of the octave layer
// draws steady.
//
// The field is SHARED. Every node samples the same sheet at its own place on
// `in.field` — the plane the billboards face, in world units — so the light
// crosses the lattice as one thing rather than each node running a copy of it
// inside its own uv, which would read as many small identical animations
// rather than one big one.
//
// There is no texture, for the same reason there is no per-node phase: every
// pattern here is a handful of dot products, sines and one power, so the
// shared sheet is a few lines of arithmetic instead of an upload, a sampler
// and a bind group. It is also seamless and resolution-free, which a scrolled
// image is not — and being built from sines rather than sampled is what lets
// it be band-limited honestly below.
//
// World units rather than screen pixels: the DAW window and a 1080p offline
// render then draw the same pattern at the same size ON THE LATTICE, where a
// period in pixels would lay a different number of cells across the picture
// in each — the same look in the plugin and in the exported video is worth
// more here than a pattern that holds still while the camera moves. Both the
// settings below are in those units for that reason.
//
// Distance from one bright peak to the next (u.misc8.y, the view's Width
// bar), and how far the sheet has travelled by now (u.misc8.x, the Speed bar
// against the clock, multiplied out on the CPU). The pair sizes and moves ONE
// shape: the softness below is what shares the period out between the lit part
// and the dark, so a wider setting widens both together rather than spacing out
// peaks of a fixed size. See `ViewConfig::shimmer_width` for what a setting
// under the node spacing costs.
fn shimmer_period() -> f32 {
    // The scene clamps this well clear of zero; the floor is here so a hand-
    // built Scene in a test cannot divide by it either.
    return max(u.misc8.y, 0.01);
}
// The exponent the raised cosine is taken to, from the Softness bar
// (u.misc8.w): 8 at 0, 1 at 1, log-spaced so equal drags are equal RATIOS of
// sharpness rather than equal steps of an exponent, which is not a scale
// anyone reads by eye.
//
// It is the whole of what the bar means, and the two ends are different
// pictures. High up, the pattern IS the cosine: every point of the period is
// on its way somewhere and the brightest part fades into the clearest across
// the whole gap, which is the gradual sweep. Low down the peak narrows to a
// crest on a layer otherwise at rest — a hard band with edges, and at a tight
// width a stripe laid on the layer rather than light crossing it.
//
// Clamped rather than trusted to the scene's clamp, because the range is what
// keeps the shape a shape: below 1 the exponent would widen the lit part past
// the dark and invert the pattern into holes.
const SHIMMER_SHARP_MAX: f32 = 3.0;
fn shimmer_sharpness() -> f32 {
    return exp2(SHIMMER_SHARP_MAX * (1.0 - clamp(u.misc8.w, 0.0, 1.0)));
}
// What a peak is worth at intensity 1, as the natural log of the gain from the
// sheet's trough to its crest: an EXPOSURE, not an amount of light to add.
//
// A multiply and not an addition, and that is what the sheet MEANS rather than
// how it is arithmetically spelled. The values here are gamma-encoded — the
// targets are UNORM and nothing decodes on the way in — so a multiply here is a
// multiply in linear light too, which is a pure luminance scale: while it fits,
// it slides a color along its own chromaticity, holding hue and saturation
// exactly at every phase. That is what light falling on a surface does, and it
// is the model that keeps the sheet ONE SIZE, because a ratio is the same ratio
// on every color where an amount of light is not.
//
// It does not always fit, and `shimmer_light` is where that is dealt with: a
// crest with nowhere left to go pales toward white rather than clipping. So the
// honest claim is that hue is held — 0.7 and 5.0 degrees across the ramp's two
// ends at Intensity 1 — and that chroma is what a bright crest spends, keeping
// 88% and 57%.
//
// The two other shapes spend it worse. A mix toward white drains chroma to 15%
// at EVERY point on the ramp, bright crest or dark trough alike — a ring under
// a peak is bleached rather than lit, and on a dark saturated color bleaching is
// most of what there is to see. An addition holds the channel GAPS instead,
// which is the same thing until a channel saturates: 0.4 of added light fits the
// headroom of exactly one color on the default ramp, so the other 63 clip at
// Intensity 1, and a clip is asymmetric — the channels with room go on rising
// and the one without does not. That costs chroma AND swings the hue 15 degrees
// at the ramp's bright end, which a lerp does not do.
//
// The SIZE is in contrast, which is the currency a texture this fine is seen
// in, and choosing it is the whole of what this constant decides. An addition
// is near-uniform in the `L*` it ADDS — 20.8 to 23.9 across the ramp at the
// fresh view's Intensity, a 13% spread, which is the property an addition is
// tuned to hold — but the ratio between a crest and its trough falls from
// 0.508 at the ramp's dark end to 0.367 at its bright one, a 29% decline. A
// moving texture is read by that ratio and not by the `L*` difference, which
// is why such a sheet reads weaker on the ramp's bright half however uniform
// its added light is. One exposure everywhere makes the ratio the constant
// instead, at a 13% spread of its own. The trade runs the other way — the
// `L*` a peak is worth varies so the ratio can hold — and that is the right
// way round for what the eye is doing here. Both halves are measured in
// `the_sweep_is_worth_the_same_contrast_on_a_dark_color_as_on_a_bright_one` and
// `a_ring_keeps_its_color_under_a_sweep_peak`.
//
// 0.873 sizes a peak at the fresh view's Intensity to what an added-light
// sheet is worth at mid-ramp, so a saved view's Intensity keeps meaning about
// the size it was dialled at.
const SHIMMER_EXPOSURE: f32 = 0.873;
// The most LIGHT a crest may ask for, as the luma of the layer under it. Where
// a swing would take a layer past this, it slides down until the crest lands
// here instead (see `shimmer_light`).
//
// This is the knob on a three-way trade with no free corner, because the sRGB
// gamut is what sets it: a sheet can be one size everywhere, leave the troughs
// alone, and keep the color — any two, never all three. The ramp's bright end
// carries a luma of 0.64 in the scale this arithmetic runs on, and one uniform
// peak wants half again more light than the display has — and what room there
// is near white leaves that hue almost no chroma to be at. Something gives,
// and this says what.
//
// Measured across the default ramp at the fresh view's Intensity, against an
// added light (which darkens 5.0, spreads 29%, keeps 80% of the chroma and
// swings 6.0 degrees). The trough figures model `lit` as the DECODED colors'
// linear luminance (bright end 0.38, which clears the slide threshold for one
// color); the shader dots the stored encoded values themselves (bright end
// 0.64), and its slide compounds through the display transfer into a deeper
// ratio in light — so every row's trough cost runs higher on screen than its
// figure says, and what the table carries is the comparison between shapes,
// not the prices. The first three rows are a fixed FRACTION
// of the shortfall taken as slide instead, which is the other shape this
// could have; the ceiling is the rest:
//
//                    darkens by   spread   worst chroma   worst hue
//   quarter-slide       4.3 L*      11%        29%           3.2
//   half-slide          8.3 L*       8%        65%           2.5
//   full slide         15.2 L*       3%       112%           0.3
//   ceiling 0.95        3.8 L*      13%        25%           3.6
//   ceiling 0.90       10.1 L*      15%        43%           3.6
//
// A ceiling and not a fraction, because a fraction does not know how big the
// swing is and this does. At the TOP of the Intensity bar the quarter-slide
// blows out — 0% of the chroma left and 41 degrees of hue, no better than the
// addition's 37 — where the ceiling still holds 12% and 6.4 degrees, because
// it goes on sliding exactly as far as the growing swing needs. One constant
// that behaves at both ends of a bar beats one tuned for the middle of it.
//
// 0.95 leaves the slide engaged over the ramp's upper half. The swing fits
// under the ceiling up to a luma of CEILING / e^swing — about 0.40 at
// Intensity 1 — so below that a trough IS the steady layer; mid-ramp (0.45)
// pays about 4 `L*` of trough for its crest, and the bright end (0.64) pays
// about 15, most of its swing arriving as shade.
// `between_peaks_the_layer_sits_at_its_own_color` pins those three measured
// readings. What the slide buys is the crest staying a color — what a bright
// crest still cannot take as light it gives up as chroma — and moving the
// ceiling trades the two across the board: lower is darker troughs and more
// chroma kept at the top of the ramp.
const SHIMMER_CEILING: f32 = 0.95;
// sRGB's own luminance row, for the ceiling above and the desaturation below.
// Weighted rather than a plain mean so a blown yellow pales toward the white
// its own light names, rather than toward a grey darker than it started.
const SHIMMER_LUMA: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);
// How much of that exposure this view asks for (u.misc8.z, the Intensity bar).
// One number scaling one thing, where an added-light model has a brightness
// and a coverage fade to keep in step: the sheet is one shape at every setting,
// and 0 leaves `shimmer_light` returning the layer exactly as it draws
// unshimmered — from the bar rather than from the mode.
fn shimmer_depth() -> f32 {
    return max(u.misc8.z, 0.0);
}
// Which way the sheet is laid and travels. A diagonal because the lattice's
// own structure is upright — its rows of fifths and thirds — so a pattern
// along either axis would run parallel to something already in the picture and
// read as part of it.
const SHIMMER_ANGLE: f32 = 0.375 * TAU;
// Where one fragment's shimmer stops being resolvable: how much of a period
// a single pixel spans. Below the first the sheet is drawn at full strength;
// past the second it is gone, and between them it fades.
//
// The upper bound is Nyquist — half a period to a pixel is the tightest a
// sampled sine can mean anything at all, and past it the pattern does not get
// finer, it gets WRONG: a moire of the sampling grid, which crawls as the
// camera moves and lands differently in the DAW window than in a render of
// another size. So the sheet is faded to nothing rather than aliased, and a
// width dialed past what the zoom can carry settles to the layer's own steady
// look. The lower bound sits well short of Nyquist so the fade is finishing
// where the moire would be starting, rather than racing it.
//
// Both are scaled by a ROOT TWO the Width bar never sees, because the period
// the bar sets is not every pattern's finest feature. Checker multiplies two
// crossed gratings into their sum and difference frequencies, and those run at
// k*sqrt(2) — so a Checker at the bar's Nyquist is already half a period past
// its own. The row is faded on its tightest member rather than per pattern:
// one fade for one sheet, and what Bands gives up for it is a slightly earlier
// finish at a width no shot is framed for anyway.
const SHIMMER_RESOLVE_FULL: f32 = 0.14;
const SHIMMER_RESOLVE_GONE: f32 = 0.35;
// Turn a unit vector by `angle`, for the gratings below.
fn rotated(v: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(v.x * c - v.y * s, v.x * s + v.y * c);
}
// The signed pattern at `p`, in -1..1 — mode by mode, and every one of them
// built out of gratings of the SAME period, so the Width bar means one thing
// across the row. `d` is the direction the sheet is laid along and `n` its
// perpendicular.
//
// The two that cross gratings are the tessellating family the checkerboard
// belongs to: multiply two and you get its cells, sum three at sixty degrees
// and the cells come out hexagonal — which lands on the lattice better than
// squares do, its rows running three ways rather than two.
fn shimmer_pattern(mode: u32, p: vec2<f32>, d: vec2<f32>, n: vec2<f32>) -> f32 {
    let k = TAU / shimmer_period();
    if mode == 2u {
        // Checker: two gratings at right angles, multiplied. The product is
        // +1 where both crests meet AND where both troughs do, which is the
        // checkerboard's two colors of cell; it crosses zero along the lines
        // between them.
        return sin(k * dot(p, d)) * sin(k * dot(p, n));
    }
    if mode == 3u {
        // Hex: three gratings sixty degrees apart, summed. Their wavevectors
        // close a triangle (the middle one is the sum of the outer two), so
        // the three crests can meet, and where they do the sum reaches 3
        // against a floor of -1.5 — bright cells rarer and sharper than the
        // dark between them, which IS the honeycomb. Mapped through that
        // range rather than divided by the count, which would leave the
        // pattern unable to reach either end.
        //
        // The outer two gratings take the travel through a cos of sixty
        // degrees, so along their own axes the sheet moves at HALF its own
        // rate and this pattern only closes a cycle over two periods. That
        // is what `Scene::shimmer_slide` reduces against, and it is the one
        // arm here that needs the two: changing these angles changes the
        // modulus, and reducing by one period flips this pattern's sign at
        // every wrap.
        //
        // COSINES, where every other pattern here takes sines, and the
        // asymmetry is the whole point: sin(A) + sin(C) + sin(A+C) is an odd
        // function, so it runs a symmetric ±3sqrt(3)/2 with as many dark
        // blobs as bright and no crest that ever reaches full. Only the
        // cosine form has the crest the honeycomb is made of, and it is the
        // one the -1.5..3 above is the range of.
        let a = cos(k * dot(p, d));
        let b = cos(k * dot(p, rotated(d, TAU / 6.0)));
        let c = cos(k * dot(p, rotated(d, TAU / 3.0)));
        return (a + b + c - 0.75) / 2.25;
    }
    // Bands (mode 1): one grating along the sheet's own direction.
    return sin(k * dot(p, d));
}
// The sheet at this fragment, as (swing, shape): the log gain from its trough
// to its crest, and where in that swing this fragment sits, 0 at a trough and 1
// at a crest. `shimmer_light` is what turns the pair into a color.
//
// ONE term, where an added-light sheet needs two — a brightness and a
// coverage scale. An added light clips at white, so on color alone a layer
// already near it would barely move and its dip would have to be an
// opacity's job; an exposure moves a near-white glyph as readily as the dark
// ground beside it, because it is a ratio rather than an amount.
// Coverage is the layer's own, untouched by the sheet, which is what keeps
// `paint_reach` exact: nothing here can make a layer cover more than it does
// steady, since the sheet never touches coverage at all.
//
// A swing of 0 in mode 0, which `shimmer_light` reads as the identity, so a
// caller applies it unconditionally and Off stays byte-for-byte the look it
// was.
//
// `footprint` is how much of the field one pixel spans, in the field's own
// world units, taken with the other derivatives at the top of the fragment
// body — derivatives have to be in uniform control flow, and by here the
// shader has already been free to discard.
//
// The direction is built from SHIMMER_ANGLE here rather than passed in, so the
// cos/sin that make it sit AFTER the early return. Handed in as a vector they
// would be evaluated at the call site in every mode, and Off would be free
// only if the backend inlined this and folded the constant — which is the sort
// of thing that holds on one driver and not the next.
fn shimmer_terms(mode: u32, field: vec2<f32>, footprint: f32) -> vec2<f32> {
    if mode == 0u {
        return vec2<f32>(0.0, 0.0);
    }
    let period = shimmer_period();
    let dir = vec2<f32>(cos(SHIMMER_ANGLE), sin(SHIMMER_ANGLE));
    let norm = vec2<f32>(-dir.y, dir.x);
    // How far the sheet has slid — arriving whole rather than as a clock
    // times a speed, and already reduced onto one cycle of the pattern. The
    // reduction is exact (every arm below is periodic in it) and it is done
    // in f64 on the CPU, where a song position still HAS the resolution to
    // phase a band with. See `Scene::shimmer_slide`.
    let slide = u.misc8.x;
    // The field slid along the sheet's own direction, which is the one
    // position every pattern is built on.
    let p = field - dir * slide;
    let pattern = shimmer_pattern(mode, p, dir, norm);
    // Clamped because the power below is `pow`, which is undefined for a
    // negative base — and sin is only promised to land NEAR its range, so a
    // wave of -1e-8 at a trough would put a NaN into the node's color.
    let wave = clamp(0.5 + 0.5 * pattern, 0.0, 1.0);
    let band = pow(wave, shimmer_sharpness());
    // Fade the sheet out as its period closes on the pixel — see
    // SHIMMER_RESOLVE_*. It rides on the DEPTH rather than on the pattern's
    // amplitude, so what a sheet running out of resolution settles onto is
    // the identity below — the layer's own steady look. Damping the amplitude
    // instead would leave `wave` at a half and the layer under a flat pale
    // haze at a flat coverage dip: the average of a sheet nobody can see, and
    // a picture that never returns to the one Off draws.
    let resolve = 1.0 - smoothstep(
        SHIMMER_RESOLVE_FULL,
        SHIMMER_RESOLVE_GONE,
        footprint / period,
    );
    let depth = shimmer_depth() * resolve;
    // No clamp on the swing, and none is needed: an exposure has no value that
    // stops meaning something the way an added light past 1 or a coverage below
    // 0 does, and `shimmer_light` fits whatever arrives to the layer's own
    // headroom. `ViewConfig::sanitize` checks this intensity for finiteness and
    // not for range, so what reaches here from a saved view can be larger than
    // the bar's own top of 2 — a very large swing is a very dark trough, which
    // is a picture, where a clamp would be a flat lid over one.
    return vec2<f32>(SHIMMER_EXPOSURE * depth, band);
}
// The layer's color under the sheet: `rgb` scaled by the gain the pair from
// `shimmer_terms` asks for at this fragment.
//
// The gamut handling, and the whole of it: a crest may not take a channel past
// 1, and this is what happens instead of the clip an addition takes there.
//
// Two things share the shortfall. The swing SLIDES DOWN, but only so far as
// `SHIMMER_CEILING` asks — which costs the troughs some light and nothing else,
// since a slid swing is the same RATIO between crest and trough and so still
// one size. And whatever crest still overflows a channel is DESATURATED toward
// the grey of its own light rather than clipped, which costs that crest some
// chroma and buys back the light the slide gave up.
//
// The ceiling is read against the layer's own light, never below it: a layer
// already brighter than the ceiling is not asked to be dimmer than it draws, it
// simply has nowhere up to go and takes the whole swing as shade. That is what
// keeps this continuous as the swing closes on zero, where a fixed ceiling
// would step a near-white layer down the moment a mode was switched on.
//
// The desaturation is the smaller move of the two and it is worth being exact
// about why it is not a clip. A per-channel clip stops the channel that is
// full and lets the others go on rising, so the color turns as it brightens —
// 15 degrees of hue at the ramp's bright end, which is #235. Mixing all three
// toward one grey moves them TOGETHER: the order of the channels is preserved,
// because mixing toward a constant is monotone, so the color pales toward white
// along its own hue instead of rotating away from it. `t` is the least mix that
// brings the top channel back to 1, so nothing is paled further than it must
// be, and the result reads as a highlight blowing out rather than as a ring
// changing color.
//
// `rgb` is read per LAYER rather than once per fragment: a near-white octave
// glyph and a ramp color have different room, and one fit for both would clip
// whichever it was not measured on. A glyph at the top of the pitch ramp has
// almost none — the case an added light cannot move at all, and the reason an
// additive sheet needs the coverage dip this model does without.
//
// The early return is the identity, and it has to be exact rather than nearly
// so: Off and Intensity 0 both arrive here as a swing of 0, and every layer
// below premultiplies its color by its coverage, so a color coming back a
// rounding under itself would show as a mark drawn dimmer than the bar says.
fn shimmer_light(rgb: vec3<f32>, terms: vec2<f32>) -> vec3<f32> {
    let a = terms.x;
    if a <= 0.0 {
        return rgb;
    }
    // The floor keeps the log finite on a black layer, which has no light to
    // lose and nowhere it needs sliding to.
    let lit = max(dot(rgb, SHIMMER_LUMA), 1e-4);
    let slide = min(0.0, log(max(SHIMMER_CEILING, lit) / lit) - a);
    let v = rgb * exp(slide + a * terms.y);
    let over = max(max(v.r, v.g), v.b);
    if over <= 1.0 {
        return v;
    }
    // The least mix toward grey that brings the full channel back to 1. Mixing
    // all three toward one value moves them TOGETHER, so their order — and with
    // it the hue — survives, where a per-channel clip stops the full one and
    // lets the others climb past it, which is the 15 degrees in #235.
    let grey = dot(v, SHIMMER_LUMA);
    let t = clamp((over - 1.0) / max(over - grey, 1e-4), 0.0, 1.0);
    // The slide keeps `grey` at or under the ceiling for any in-gamut layer
    // at any swing — the crest's luma is capped at max(SHIMMER_CEILING, lit)
    // by construction — and the mix lands the top channel at exactly 1. The
    // clamp is rounding insurance on the exp/log round trip, and the honest
    // end for a layer handed in already brighter than white, whose grey sits
    // past 1 before the sheet touches it.
    return min(mix(v, vec3<f32>(grey), t), vec3<f32>(1.0));
}

// ---- Outer octave layer ----------------------------------------------------
// Every outer style draws its glyphs inside the radial band
// [u.misc3.y, u.misc3.z] (quad UV units): the band IS the glyph set's
// radial footprint, so switching styles keeps the octave display the same
// size. The glyphs are drawn identically whatever the ring inside them does —
// the layers are independent.
//
// The backdrop is always on: it is the cohesion device that makes a note
// read as ONE whole shape even when a single octave sounds, so the SILENT
// octaves draw in the rings' own ground (u.lattice_ground), carrying the ring's
// shape around the bright one. It completes the circle, since the indicators
// tile the whole turn: the only breaks in it are the gaps between them.
//
// The glyphs are always crisp, too: `aa` alone is their edge width, which
// is what makes it screen-constant at every zoom.

// Neighboring sectors are separated by a CONSTANT-thickness gap: the slice
// edges are radial lines offset half the gap from the boundary the two
// sectors share, not constant-angle edges (those read as a V that widens
// outward). That matters more the more indicators there are, and their
// number is a setting. At band inner 0 the sectors become full pie wedges;
// near the center every wedge falls inside the gap band, leaving a small
// clear hub instead of an N-way mush point.
//
// The gap's full width is u.misc5.z (the view's Octave gap bar), in quad UV
// units. The same value cuts the sides of a melody/bass mark, so one number is
// every angular interruption on the node; how far the marks stand OFF the band
// is the radial gap instead, and it reaches here only as a radius.
fn slice_gap_half() -> f32 {
    return max(u.misc5.z, 0.0) * 0.5;
}
// The backdrop is OPAQUE where the note is fully present, and its color is
// u.lattice_ground — so a silent slice is that grey exactly, the same grey the
// audio ring a gap inside it reads at silence.
//
// A color and not an opacity, which is the whole of what makes the two rings
// agree. A ghost laid on at a fixed alpha instead lands on a blend of the
// ground it happens to be over, which is a near-grey that moves with whatever
// is behind it and with the note's own hue — no definite colour to share with
// the ring a gap inside it, so the two annuli reach their empty state by
// different routes and no one value dials both. As a color the grey IS one
// number in the view (Ground), and the fade to it rides the ACTIVATION, so the
// whole ring still fades in and out with the pitch class.

// How much of the glyph BAND this pixel is inside, which every slot's glyph
// is scaled by. It asks only about the radius, so it is the same answer for
// every slot on the node — hoisted out of [`outer_glyph`] so the caller can
// take it once and, where it is zero, skip the whole per-slot loop. The band
// is a narrow annulus (0.64..0.85 by default) and the glyph layer is done
// with by GLYPH_FADE_LIMIT, inside a billboard reaching QUAD_MARGIN or more,
// so most of a lit node's fragments are outside the band, and running the
// loop there is `span` sectors of work for an answer of zero.
// A collapsed band is the caller's to skip, and every caller does (the mark
// strip and the audio ring by a test of their own, the octave band by the
// `select` below). It cannot be answered here: at inner == outer the two soft
// edges do not cancel, they cross, and the product peaks at a quarter — a
// screen-constant hairline ring of a quarter coverage, which is exactly what a
// layer dialled to nothing must not draw.
fn glyph_band(d: f32, inner: f32, outer: f32, aa: f32) -> f32 {
    return aa_inside(outer, d, aa) * (1.0 - aa_inside(inner, d, aa));
}

// One slice of the octave band: the colour it paints and how opaque it is,
// `xyz` and `w`, before any of the wedge's own shape.
//
// A layer's colour is a CONTINUUM rather than a pair — a fully sounding glyph
// is its own pitch exactly, a silent one is the rings' ground, and a slot part
// way through its envelope is the two mixed by however much of it is lit, which
// is what a fade between them IS.
//
// Over in BOTH terms together, which is what makes the end of a release one
// continuous thing. Taking the opacity as a max() and the colour by a
// `level > 0` switch instead parts them — the opacity floors at the ghost while
// the colour is still the lit pitch, so the fade visibly stops, and then the
// colour steps to the ground in one frame at no change in opacity at all. That
// is visible only where a node's presence OUTLIVES this slot's level, which is
// another instance of the pitch class still held: a lone note drives both off
// one envelope, so the ghost never catches the fade and the switch lands at
// nothing.
//
// The ghost takes what is left of the node's PRESENCE after this slot's own
// level, rather than a share of the whole of it. Both releases are then
// straight lines: the opacity is `presence` throughout, so a slot fading under
// a held instance runs its COLOUR from the pitch to the ground evenly, and a
// slot whose node is going with it — `level` and `presence` one envelope — runs
// its opacity to nothing evenly. Scaling the ghost by `1 - level` instead,
// which is the same thing wherever presence is 1, counts the note's own
// presence twice in that second case and bulges the opacity above the straight
// line through the middle of the fade.
//
// Its own function because the GLOW is lit out of the same reading: a band
// slice lighting a halo in some second reading of "what colour is this octave"
// is exactly the drift this file spends its comments on. What the light takes
// is [`oct_slot_lit`] below — this without the ghost, since the ghost is a
// backdrop rather than anything the octave is doing.
fn oct_slot_ink(in: VsOut, slot: i32) -> vec4<f32> {
    let presence = in.params.x;
    let lit = oct_slot_lit(in, slot);
    let level = lit.w;
    if level <= 0.0 {
        return vec4<f32>(u.lattice_ground.rgb, presence);
    }
    let ghost_rest = max(presence - level, 0.0);
    let opacity = level + ghost_rest;
    // A glyph as lit as its node is present is exactly the colour that pitch
    // lights everywhere else, which `ghost_rest` is what holds: it is nothing
    // where `level` reaches `presence`, so a fully lit slot AND a lone note's
    // whole release wear the pitch alone. The LUT is the pitch ramp
    // (pitch_ramp_lch in harmonigraph-scene), and the node's own light and the
    // piano roll sample that same table — so all three read as one colour where
    // the note is sounding. A white mix there would be a second definition of
    // what a lit pitch looks like, and it would drift off the halo the moment
    // the gradient's brightness moved. Where the node OUTLIVES the slot, the mix
    // toward the ground is the ghost coming through as that one octave goes,
    // which is the fade itself rather than a second definition of anything.
    //
    // The divide un-premultiplies, and wants no floor under it: `level > 0`
    // here, the packing's smallest step is 1/255, and `ghost_rest` is never
    // negative, so `opacity >= level`.
    let ground = u.lattice_ground.rgb;
    return vec4<f32>((lit.xyz * level + ground * ghost_rest) / opacity, opacity);
}

// The LIGHT one slice of the octave band gives off: `xyz` the colour it wears
// where it is lit, `w` how much of it is — this slot's own level, and nothing
// of the node's presence.
//
// The two readings a slice has, and the whole of what parts them: the GROUND
// IS NOT A LIGHT. A silent slice is drawn in it ([`oct_slot_ink`], which is
// this with the ghost laid over it) because the backdrop is what carries the
// ring's shape around the bright slice — it says a node is there, not that
// this octave is sounding. So it weighs exactly 0 in [`ink_at`], and the halo
// round a note voiced in one octave is that octave's colour rather than a
// tenth of it under nine tenths of grey.
//
// The colour of a LIT slice is stated once, here, and the drawn ink is this
// mixed toward the ground by however much of the node's presence this slot's
// level does not account for.
fn oct_slot_lit(in: VsOut, slot: i32) -> vec4<f32> {
    // Slot s is MIDI octave s - 1, whose C is MIDI 12*s; add this node's pitch
    // class for the glyph's true pitch.
    let pitch = oct_slot_pitch(slot, in.cents);
    return vec4<f32>(pitch_lut_color(pitch), oct_slot_level(in.octaves, slot));
}

// Coverage (0..1) of the outer glyph for octave slot `s` on the node whose
// ring is `ring`, drawn in the uniform band. Reads nothing from the ring
// inside it — the outer glyphs are independent of it. `aa` is the caller's
// per-pixel soft-band width, giving the shape screen-constant edges.
//
// `band` is this pixel's radial coverage, from [`glyph_band`]: the caller
// holds it because it is the one term here that does not depend on `s`.
fn outer_glyph(
    s: i32, ring: OctRing,
    uv: vec2<f32>, band: f32, aa: f32,
) -> f32 {
    // Annular sectors, screen-constant edges. The directions b1/b2 are this
    // sector's two edges; the cross products against them give BOTH the
    // side-of-line tests (which wedge owns the pixel) and the Euclidean
    // distance to each edge line, thresholded at half the gap width, for a
    // gap of constant thickness at every radius.
    let edges = oct_sector(s, ring);
    let b1 = vec2<f32>(cos(edges.x), sin(edges.x));
    let b2 = vec2<f32>(cos(edges.y), sin(edges.y));
    let c1 = uv.x * b1.y - uv.y * b1.x;
    let c2 = uv.x * b2.y - uv.y * b2.x;
    // Ownership softened over `aa`: a hard step would show as a straight
    // cut down the slice's sides wherever the gap doesn't reach zero at the
    // wedge boundary — an Octave gap of 0, which closes the sectors into a
    // solid annulus, is exactly that case. Soft ownership lets adjacent slices
    // cross-fade (the loop keeps the max), so the sector edges stay clean.
    //
    // A wedge under a half turn is the INTERSECTION of its two half-planes;
    // one PAST a half turn is their union, and reading it as an intersection
    // would empty the sector instead of filling it. Both cases are real here:
    // one full-size octave flanked by an extra either side, at the thinnest
    // size, pins those two at a tenth of an even slice each and leaves the one
    // between them 336 degrees.
    // That extreme, and the floor that keeps it under a whole turn, are
    // `an_indicator_can_pass_a_half_turn_but_never_a_whole_one` and `MIN_SPAN`
    // in harmonigraph-scene. (The test itself is `oct_arc_coverage`.)
    let own = oct_arc_coverage(edges, uv, aa);
    // Each edge's gap is cut only on the side the edge actually runs to. The
    // boundary LINE passes just as close on the far side of the node, which
    // falls outside a narrow wedge and so does not matter there, but lands
    // inside a wide one — where it would read as a slit across an indicator
    // that has no boundary at all in that direction.
    let gap_half = slice_gap_half();
    let g = (1.0 - aa_inside(gap_half, abs(c1), aa) * smoothstep(-aa, aa, dot(uv, b1)))
        * (1.0 - aa_inside(gap_half, abs(c2), aa) * smoothstep(-aa, aa, dot(uv, b2)));
    return band * own * g;
}

// Color at absolute MIDI `pitch`, read from the pitch gradient LUT so an
// octave glyph is the same hue as the disc that pitch would light.
fn pitch_lut_color(pitch: f32) -> vec3<f32> {
    let t = clamp((pitch - u.misc2.x) / max(u.misc2.y - u.misc2.x, 0.01), 0.0, 1.0);
    let f = t * f32(PITCH_LUT_N - 1u);
    let i0 = u32(floor(f));
    let i1 = min(i0 + 1u, PITCH_LUT_N - 1u);
    return mix(u.pitch_lut[i0].rgb, u.pitch_lut[i1].rgb, f - floor(f));
}

// Color at loudness `level` (0..1), read from the FREQUENCY scheme's ramp —
// the analyzer's own gradient, the one the spectrogram's cells, the spectrum
// curve and the Spiral pane's segments are all drawn off, re-anchored for the
// LATTICE: the CPU rebuilds the table from that gradient with its silent end
// pinned onto u.lattice_ground — that L*, and no chroma at all — and its loud
// end left where it stands, so the two tables meet exactly at the top and a
// loud level is the same light here as there (see
// `harmonigraph_scene::ring_gradient`). The same walk pitch_lut_color does,
// over the other table and against the other quantity.
fn spectral_lut_color(level: f32) -> vec3<f32> {
    let f = clamp(level, 0.0, 1.0) * f32(PITCH_LUT_N - 1u);
    let i0 = u32(floor(f));
    let i1 = min(i0 + 1u, PITCH_LUT_N - 1u);
    return mix(u.spectral_lut[i0].rgb, u.spectral_lut[i1].rgb, f - floor(f));
}

// Whether each wedge of the audio ring is ONE reading taken at its own
// octave's pitch (u.misc9.y — the FOLD) rather than a window of pitch spread
// across the wedge. The two readings differ in what u.spectrum holds and in
// where in the wedge it is sampled, and nowhere else.
fn folded() -> bool {
    return u.misc9.y > 0.5;
}

// ---- The audio ring --------------------------------------------------------
// A second ring of the same wedges, INSIDE the octave band. The band says
// which octaves are HELD; this says what is actually SOUNDING around each of
// them, and it never replaces any part of the MIDI picture — the two readings
// are on one node at once, told apart by their radius and by their color
// scheme (pitch_lut by PITCH out there, spectral_lut by LEVEL in here).
//
// Which reading fills it is the one setting: harmonigraph_scene's
// `SpectralReading`, reaching the shader as `folded()` above.
//
// FOLD — one number for the whole wedge, read at the octave's own pitch, off a
// u.spectrum the CPU has already folded (a Gaussian kernel over a local noise
// floor). What survives is energy concentrated AT the node's pitch rather than
// energy near it, so a timbre draws as a constellation across the lattice: a
// partial sits at an exact rational multiple of its fundamental, and on a
// 7-limit lattice the first sixteen harmonics land on six nodes.
//
// SPECTRUM — a wedge is not one number. Angle within it runs linearly over a
// window of u.misc9.x cents centered on that octave's own pitch —
// counter-clockwise edge the bottom of the window, clockwise edge the top,
// since clockwise is the direction pitch rises everywhere else on the wheel.
// So a partial dead on the node paints down the middle of the wedge, and one a
// comma sharp paints to the clockwise side of it: the ring reads a DETUNING,
// which is the one thing a folded number per octave cannot say. Nothing is
// folded into u.spectrum for it either — no kernel and no noise floor, since
// both are a blur across the very axis the window exists to resolve, so a
// wedge is the picture the Spectral pane would draw of the same stretch of
// frequency.
//
// One cost is shared, and stated rather than hidden: with nothing sounding a
// wedge is not empty, it is the ramp's silent end. A range with nothing in it
// is a reading. That end is PINNED to u.lattice_ground, the same grey the
// octave band's unlit slices are drawn in, so the two rings read as empty in
// exactly one colour and one bar moves both — and what that colour is against
// the pane is that bar's own answer, from holes punched through the surface at
// the bottom of it to the whole resting picture vanishing into the pane at the
// panel's own L*.
//
// That grey is a thing DRAWN and never a thing lit: the glow weighs each
// wedge by the reading behind it (`ink_at`), so a ring of empty wedges is on
// screen without a halo round it.
//
// WHICH nodes wear one, and how much of it, is in.ring, decided on the CPU: a
// node draws its ring when one of its wedges reaches the view's Gate
// (harmonigraph_scene's RingGate), and a ring comes and goes on the note Fade
// rather than at the instant that answer changes (RingFade). A node the KEYS
// have lit wears its ring whatever the gate says, so the two pictures leave
// together. At the gate's floor every node in the window rings, silence
// included — the reading in full, and hundreds of rings saying only where the
// nodes are.
//
// The radius is its own (u.misc7.z/w, the innermost slot of the stack at the
// fresh view, reaching the node's own centre); the slices are the band's, off the
// same `OctRing`, so the two rings share one rhythm of wedges and one meaning
// for an angle.

// The ring's inner and outer radius in quad UV units, both 0 when it is off.
fn spectral_radii() -> vec2<f32> {
    return vec2<f32>(u.misc7.z, u.misc7.w);
}

// The analyzer's loudness at bucket `b`, unpacked from the byte it rides as.
fn spectrum_level(b: u32) -> f32 {
    let i = min(b, SPECTRUM_BUCKETS - 1u);
    let word = u.spectrum[i / 16u][(i / 4u) % 4u];
    return f32((word >> ((i % 4u) * 8u)) & 0xFFu) / 255.0;
}

// The analyzer's loudness at absolute MIDI `pitch`, interpolated between the
// two buckets either side of it, or 0 where the axis does not reach that pitch
// (under 20 Hz, over 20 kHz).
//
// Zero rather than the nearest end's value, which is what a clamp would give:
// the axis ending is the analyzer having nothing to say, and smearing the last
// bucket it does have across everything past it would draw a measurement where
// none was taken. A silent floor is the honest edge.
//
// A bucket stands for its own CENTRE, which is the half-step below — the grid
// is `SPECTRUM_MIN_MIDI + (b + 0.5) / BUCKETS_PER_SEMITONE`, and dropping the
// half puts every partial half a bucket flat of where it sounds.
fn spectrum_at(pitch: f32) -> f32 {
    let x = (pitch - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE - 0.5;
    if x < 0.0 || x > f32(SPECTRUM_BUCKETS - 1u) {
        return 0.0;
    }
    let i = u32(floor(x));
    return mix(spectrum_level(i), spectrum_level(i + 1u), x - floor(x));
}

// Where a fragment sits ACROSS one wedge, 0 at its counter-clockwise edge and
// 1 at its clockwise one — the fraction the pitch window is read at.
//
// Off the fragment's angle relative to the wedge's own middle rather than off
// an atan2 against a fixed zero, so nothing here has a seam: a wedge is under
// a whole turn by construction (MIN_SPAN), so every point inside it is within
// half a turn of its middle and the wrap below never reaches one.
fn wedge_fraction(edges: vec2<f32>, uv: vec2<f32>) -> f32 {
    let mid = 0.5 * (edges.x + edges.y);
    // Half the wedge, always positive: the sector's edges are given
    // counter-clockwise first and the walk from one to the other is clockwise,
    // which is decreasing angle.
    let half = max(0.5 * (edges.x - edges.y), 1e-5);
    // The fragment's angle about that middle, brought onto (-pi, pi].
    let c = uv.x * cos(mid) + uv.y * sin(mid);
    let s = -uv.x * sin(mid) + uv.y * cos(mid);
    let delta = atan2(s, c);
    return clamp(0.5 - delta / (2.0 * half), 0.0, 1.0);
}

// What the audio ring is doing at one point of it.
struct RingInk {
    /// The color the owning wedge paints there. NOT premultiplied — the caller
    /// composites it against the octave layer, and a layer picked BY coverage
    /// needs the color the coverage belongs to.
    color: vec3<f32>,
    /// How much of the point that wedge covers, the node's own ring presence
    /// taken out of it.
    cov: f32,
};

/// Coverage and color of the audio ring at this fragment.
///
/// `band` is the fragment's RADIAL coverage, held by the caller exactly as
/// [`outer_glyph`]'s is: the walk below is angular, so the one edge crossing a
/// radius is taken once, outside it, with [`glyph_band`].
fn spectral_ring(in: VsOut, oct: OctRing, uv: vec2<f32>, band: f32, aa: f32) -> RingInk {
    let radii = spectral_radii();
    // Off, or an annulus dialled inside out: nothing to draw either way.
    if radii.y <= radii.x {
        return RingInk(vec3<f32>(0.0), 0.0);
    }
    // ...and this node's own gate: the layer is on, and nothing this ring would
    // show reaches the level the view asks for — nor has the node been played,
    // and nor is either still fading out. Decided on the CPU against this same
    // u.spectrum (harmonigraph_scene's RingGate and RingFade) rather than
    // rediscovered here, since the question is about the node's whole ring and
    // this is one fragment of one wedge of it.
    if in.ring <= 0.0 {
        return RingInk(vec3<f32>(0.0), 0.0);
    }
    // The ring is a narrow annulus in a billboard reaching QUAD_MARGIN, so
    // most fragments are outside it — and the whole slot walk below answers
    // zero for every one of them. The same skip the band's own loop takes.
    if EARLY_OUT && band <= 0.0 {
        return RingInk(vec3<f32>(0.0), 0.0);
    }
    // Which wedge owns this pixel, and how much of it. The color is settled
    // AFTER the walk rather than inside it: one fragment is one reading of the
    // spectrum, and taking it per candidate slot would sample the grid `span`
    // times to throw all but one away.
    var cov = 0.0;
    var owner = oct.base;
    for (var i = 0u; i < oct_span(); i = i + 1u) {
        let slot = oct.base + i32(i);
        // `outer_glyph` whole: the sector's own edges and the same constant
        // gap between neighbours, so one rhythm of slices runs through both
        // rings instead of each drawing its own.
        let c = outer_glyph(slot, oct, uv, band, aa);
        if c > cov {
            cov = c;
            owner = slot;
        }
    }
    if cov <= 0.0 {
        return RingInk(vec3<f32>(0.0), 0.0);
    }
    // WHERE in the wedge the grid is read, which is the whole of what the two
    // readings differ by. The fold answers one number for the octave, so every
    // fragment of a wedge reads that octave's own pitch and the wedge comes out
    // flat; the raw spectrum spreads a window of `range` cents across it,
    // centered on the same pitch, read at wherever across the wedge this pixel
    // falls.
    var pitch = oct_slot_pitch(owner, in.cents);
    if !folded() {
        let across = wedge_fraction(oct_sector(owner, oct), uv);
        pitch = pitch + (across - 0.5) * u.misc9.x / 100.0;
    }
    // The node's own level taken out of the COVERAGE and not out of the colour:
    // a ring on its way in is the octave layer showing through it, where a
    // wedge mixed toward the bed would be a reading of a quieter spectrum. The
    // caller composites by coverage, so this is the one place it belongs.
    //
    // The reading itself goes out beside the colour it picks, un-scaled by
    // either: how loud this wedge is and how much of the node's ring is showing
    // are two questions, and the light asks the first one alone.
    let level = spectrum_at(pitch);
    return RingInk(spectral_lut_color(level), cov * in.ring);
}

// ---- How tightly a node's octaves pack -------------------------------------
// A node's light takes its colour from the INK STRIP: every layer the node
// draws contributes in proportion to the radial width it occupies TIMES how
// lit it is there, so a silent slice and a silent wedge — both of them the
// ground exactly — weigh nothing (`ink_at`, `fs_ink_strip`, then `glow_ink`).
// What is left here is the angular tightness that blend is laid out at.

// The TIGHTEST each octave's angular color lobe is drawn at (a von Mises-like
// falloff): higher is tighter, more separated arcs. Tuned so neighbouring
// octaves blend softly rather than banding at the widest span, where they sit
// closest together. A ceiling rather than the concentration itself — the seams
// are fixed in ANGLE and so converge to a cusp at the node's centre, and a
// caller holding them to one width asks for less there (`glow_layer`, which
// eases to the blend's mean instead; the rim width is 1/sqrt of this, so the
// two move together).
const GLOW_LOBE_KAPPA: f32 = 4.0;

// An unlit node's own billboard paints no disc, no trail mark and no
// placeholder. What says the position is there is the MARKER standing at it,
// which is a separate instance drawn under the home sheet (`fs_plus`) rather
// than anything this node paints. So every DISC on screen is a note.
//
// The audio ring is the one thing an idle node does paint, and it is not the
// node speaking — it is the analyzer, drawn on the node's own ground (see
// `node_paint`, which states the idle case in full).
//
// The trail rides no layer here at all. A node the music has been to is
// captioned by the LABEL layer, on the CPU (see harmonigraph_scene::trail) --
// which is what makes a memory unmistakable for a sounding note: the two are
// not the same kind of thing on screen, rather than the same kind kept apart
// by a ceiling on how loud a mark may get.

// ---- Melody / bass marks ---------------------------------------------------
// A mark is its octave's SLICE, continued outward: an annular sector in the
// strip just past the octave band, on exactly the angles the marked indicator
// spans, with its SIDES cut by the same padding that separates one indicator
// from the next. How far past the band the strip stands is the node's other
// padding, the radial one, and it arrives as `mark_inner` rather than as a gap
// this reads. Both ends draw in that one strip, and both draw in their own
// sector's color rather than a fixed livery.
//
// The shape is the whole of what says WHICH octave is the melody -- the
// question a full ring at its own radius cannot answer, and the one that is
// everything on a chord voiced inside a single pitch class, where every note
// lands on one node and differs only by slot. A ring bracketing the band and
// slit at the marked sector's boundaries points back at it; extending the
// slice itself is that link with nothing left over.
//
// What the shared strip gives up is telling the two ENDS apart by radius.
// Mostly that costs nothing: a mark names a slice, the slices are ordered by
// pitch round the node, so the higher of two marked slices is the melody. Where
// one note is both ends -- a lone held note, or a chord whose top and bottom
// share a pitch class -- the two marks are one slice and draw as one extension,
// in the one color they both carry (each takes its SECTOR's pitch, and it is
// the same sector).
//
// The ordering is not an invariant, and the exception is worth knowing because
// the strip is what gave up the other cue. A mark outlives its key, and a
// RELEASED voice claims each end from its own stamp, so a lone note that wore
// both can hold the melody on a low slice while a live note takes the bass on a
// higher one -- for the length of one release, the melody draws below the bass.
// `a_released_end_can_mark_a_lower_slice_than_the_live_one` builds that state.
// Telling them apart by radius drew it unambiguously; this does not, and the
// alternative is a second cue that would undo the shape above.

// Floor on the strip's depth (the view sets the rest, u.misc5.w), in
// soft-band widths — about a couple of render pixels, so a thin setting
// can't go sub-pixel on a densely packed lattice and read as nothing. A
// thickness of exactly 0 is the off state and skips the floor.
const MARK_RING_MIN_AA: f32 = 1.5;

// Coverage of the marks in `slots`, in the strip whose radial coverage at this
// pixel is `strip`.
//
// The sector geometry is `outer_glyph`'s, called on the mark strip's coverage
// where the octave layer calls it on the band's -- one body, so an extension
// lines up with the indicator it continues at every gap width, sector count and
// fringe size, rather than agreeing with it by two constructions that have to be
// kept in step.
//
// A slot mask can name more than one sector: releasing the top of a chord
// leaves the old melody fading on its slot while the new one takes another,
// and both are the melody for as long as that lasts. Slots the ring has no room
// for are skipped — a mark reaches for octaves the packing may not show, and
// `oct_sector` has no angle to put one at.
fn mark_extension(slots: u32, ring: OctRing, uv: vec2<f32>, strip: f32, aa: f32) -> f32 {
    // Off the strip entirely: the sectors below only ever scale this coverage,
    // so walking the slots for them would be an 11-iteration answer to a pixel
    // that is already zero. The strip is a thin annulus in a billboard reaching
    // QUAD_MARGIN — the margin it lives in is the one the marks are the reason
    // for — so that is nearly all of them.
    if EARLY_OUT && strip <= 0.0 {
        return 0.0;
    }
    let top = ring.base + i32(oct_span()) - 1;
    var cov = 0.0;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let s = i32(i);
        if (slots & (1u << i)) != 0u && s >= ring.base && s <= top {
            cov = max(cov, outer_glyph(s, ring, uv, strip, aa));
        }
    }
    return cov;
}

// Distance from `uv` to the filled wedge between `edges` (`oct_sector`'s pair,
// counter-clockwise edge first) out to radius `r`: negative inside it, and
// outside it the distance to the nearest point of it.
//
// A PIE — apex at the node's center, closed by an arc — where the layer that
// wedge belongs to is drawn as an annular sector. Its two callers each cut the
// pie back to the strip themselves, by intersecting this with everything past
// the strip's inner edge; what is shared is the ANGULAR half of the shape,
// which is the part with a case in it.
fn sector_distance(uv: vec2<f32>, edges: vec2<f32>, r: f32) -> f32 {
    return sector_pie(sector_fold(uv, edges), r);
}

// One wedge's own frame, which both readings below are taken in.
struct SectorFold {
    // The fragment, with the wedge's middle up the y axis and its two edges
    // folded onto one.
    q: vec2<f32>,
    // That edge's direction, half the wedge's width off the middle.
    e: vec2<f32>,
}

// `uv` into the wedge's own frame — its middle up the y axis — folded onto one
// side of that middle, which turns its two edges into one edge lying `half` off
// the fold. The wedge's edges arrive counter-clockwise first and the walk
// between them is clockwise, so their difference is its width.
//
// A step of its own because a mark asks TWO questions of one wedge — how far its
// pie is, and how far its gap-cut sides are — and a fold each would be two
// constructions to keep in step as well as a second `sin`/`cos` pair per marked
// slot per fragment.
fn sector_fold(uv: vec2<f32>, edges: vec2<f32>) -> SectorFold {
    let mid = 0.5 * (edges.x + edges.y);
    let half = clamp(0.5 * (edges.x - edges.y), 0.0, TAU * 0.5);
    let c = cos(mid);
    let s = sin(mid);
    return SectorFold(
        vec2<f32>(abs(uv.y * c - uv.x * s), uv.x * c + uv.y * s),
        vec2<f32>(sin(half), cos(half)),
    );
}

// Distance out of the pie `f` folds, out to radius `r`.
//
// The two features a pie has: its arc, and the straight edge running from the
// center out to it. Which one is nearest is which SIDE of that edge the point
// falls — inside the wedge the arc is the only way out, outside it the edge is.
// The sign is what says which, so a wedge past a half turn (which the extras can
// hand out — see `outer_glyph`) needs no case of its own: the fold puts the
// point on the far side of one edge either way.
fn sector_pie(f: SectorFold, r: f32) -> f32 {
    let arc = length(f.q) - r;
    let edge = length(f.q - f.e * clamp(dot(f.q, f.e), 0.0, r));
    return max(arc, edge * sign(sector_side(f)));
}

// Signed perpendicular distance from the folded point to its wedge's edge LINE:
// negative inside the wedge, positive outside it.
fn sector_side(f: SectorFold) -> f32 {
    return f.e.y * f.q.x - f.e.x * f.q.y;
}

// Distance out of ONE slice's ink across its SIDES, signed: the wedge `f` folds
// with half an Octave gap taken off each side, negative inside what is left and
// outside it the perpendicular distance to that cut.
//
// The angular half of the footprint of a layer drawn at SOME of a node's slices,
// where `slice_gap_distance` is that answer for a layer drawn at every one. The
// two agree wherever the nearest boundary is this wedge's own, and they part
// exactly where they must: the walk answers "how far out of the nearest SLICE",
// so at a boundary this wedge has no part in it hands back half a gap however
// far off this wedge's ink is. A mark reading the walk therefore stands the
// light off down boundaries it does not draw at, one shadow per boundary its pie
// is still within a Gap of — which is
// `a_mark_stands_no_light_off_a_boundary_it_does_not_draw_at`.
//
// Half a gap is ADDED rather than the wedge narrowed by an angle, because the
// gap is a band of constant thickness laid along the boundary ray and not a
// constant-angle wedge (see `slice_gap_half`). The cut is a perpendicular offset
// of the edge, and on a signed distance that is a constant.
//
// Cut only on the side the edge actually runs to, which is `outer_glyph`'s own
// gate and holds for the same reason: the boundary LINE passes just as close on
// the far side of the node, where a slice past a half turn has no edge at all. A
// hard step where the ink's is soft, the light taking no derivatives anywhere
// (see `fs_glow`); it stands where `dot(q, e)` changes sign, a quarter turn off
// the wedge's middle, so it is either far outside the ink (a narrow wedge) or
// deep enough inside it that the coverage is already solid (a wide one).
fn slice_side_distance(f: SectorFold) -> f32 {
    return sector_side(f) + select(0.0, slice_gap_half(), dot(f.q, f.e) > 0.0);
}

// How much of the destination one layer's knockout clears, `sd` out from that
// layer's own footprint.
//
// `reach` is where the clearing ENDS, measured past that footprint, and `soft`
// is how gradual that ending is — two numbers rather than one, because tying
// the fade to the reach means a wider gap is always a blurrier one. Solid
// across the footprint and out to `reach - soft`, gone by `reach`. A fade wider
// than the reach eats outward rather than inward: the footprint itself is the
// one part that always has to be cleared.
//
// `sd` is a distance out of a layer in EVERY direction, the inward one
// included, so the ramp that eases the hole off outside a ring is the same one
// that eases it off inside the ring's own middle. What the layer covers is what
// gets cleared; a ring does not own the circle it encloses.
fn gutter_coverage(sd: f32, reach: f32, soft: f32) -> f32 {
    // The inner end is floored AT the footprint, and 0 rather than something
    // smaller for a reason worth stating: a layer's own ink is cleared in full
    // at every distance under one, which is what lets `node_clearing` stop
    // walking the moment a pixel is answered.
    let edge = clearing_edge(reach);
    let inner = clamp(edge - soft, 0.0, edge - 0.001);
    return 1.0 - smoothstep(inner, edge, sd);
}

// How much of the destination a node's knockout clears at this fragment.
//
// ONE TERM PER LAYER, and each carries its own level: a layer clears its own
// footprint at the strength it is drawn at, and the hole is whichever of them
// claims the pixel most. That is the whole design, and three things fall out of
// it that a single shape times a single level cannot give.
//
// The clearing FOLLOWS THE NODE. A node's layers are dialled independently and
// worn independently — the audio ring is on where the view's Gate says, the
// marks on the octaves a chord's outer voices took, the band wherever
// a key is down — so what a node draws is a different shape node to node and
// moment to moment, and a circle sized to hold all of it clears a gap wider
// than the node nearly everywhere. Here every layer is read off the same radii
// that draw it.
//
// A layer NOBODY IS DRAWING clears nothing. That is what lets a node wearing an
// audio ring and no note clear at all — its own level is the ring's, where the
// note's is zero — and equally what stops it clearing the band-sized hole a
// node-wide level would give it.
//
// And the hole is CONTINUOUS in time, because every level here is an envelope
// rather than a switch. A layer arriving brings its own hole up from nothing at
// full size; a layer leaving takes it back down. Nothing steps, and no layer's
// fade drags another's hole with it.
//
// Each layer's shape is its own ANNULUS, dilated by the reach, and not that
// annulus filled to the node's centre. A node hides what stands behind the
// RINGS it draws and nothing else, so the middle a node leaves empty is empty
// to see through: the picture behind reads on through it, and what says which
// sheet is in front is the ring, which is the part of a node that has ink in
// it. Filling inward instead makes a node an opaque disc the size of its
// outermost ring, and the node behind it — however far off its centre stands —
// is erased down to the ground plus whatever light was standing there.
//
// The same shape `glow_standoff` measures, and for a reason the two share:
// the middle of a node is the one place its light is wanted whole, so a
// standoff filled inward would put the node's own halo out. The Shadow bars
// read as one family exactly to the degree these two agree.
fn node_clearing(in: VsOut, oct: OctRing, d: f32) -> f32 {
    let reach = in.gutter;
    let soft = in.soft;
    var cov = 0.0;
    // The octave band, on the node's own envelope: the annulus its slices and
    // the glyphs inside them are drawn between. Skipped where the view draws no
    // band at all, which is a node whose whole picture is its ring and its
    // marks.
    if u.misc3.z > u.misc3.y {
        cov = gutter_coverage(annulus_distance(d, u.misc3.y, u.misc3.z), reach, soft)
            * clamp(in.params.x, 0.0, 1.0);
    }
    // The audio ring, on ITS own: the layer's width bar says whether there is a
    // ring at all, and `Instance::ring` how much of one this node wears — which
    // is the view's Gate answered per node, carried on the note Fade. A node
    // nobody played wears one whenever the spectrum reaches that gate, and this
    // is the term that gives it a hole.
    if u.misc7.w > u.misc7.z {
        cov = max(
            cov,
            gutter_coverage(annulus_distance(d, u.misc7.z, u.misc7.w), reach, soft)
                * clamp(in.ring, 0.0, 1.0),
        );
    }
    // Every slot either mark names, each on the level of the voice that took
    // it. A mark reaches for octaves the packing may not show and the ring may
    // not draw, and `oct_sector` has no angle to put one of those at — the same
    // slots `mark_extension` skips.
    let slots = in.marks.x | in.marks.y;
    if slots == 0u || u.misc5.w <= 0.0 {
        return cov;
    }
    // Nothing below can raise a pixel already cleared in full — the same skip
    // `glow_standoff` takes, and exact for the same reason: coverage is
    // capped at one, which is what lets the parity test hold it.
    if EARLY_OUT && cov >= 1.0 {
        return cov;
    }
    let strip_in = u.misc4.y;
    let strip_out = u.misc4.y + u.misc5.w;
    let top = oct.base + i32(oct_span()) - 1;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let bit = 1u << i;
        let s = i32(i);
        if (slots & bit) == 0u || s < oct.base || s > top {
            continue;
        }
        // This slot's own level, not the strip's: where the two ends are on
        // different slices, a melody still fading out takes its own wedge's
        // hole with it while a bass arriving brings its own up.
        let level = max(
            select(0.0, in.params.y, (in.marks.x & bit) != 0u),
            select(0.0, in.params.z, (in.marks.y & bit) != 0u),
        );
        if level > 0.0 {
            // The mark is an annular SECTOR: the pie `sector_distance`
            // measures, intersected with everything past the strip's inner
            // edge. Filled to the centre instead, one mark would clear the
            // node's whole middle and every other layer's hole with it — the
            // one wedge the node reaches furthest in deciding the shape of a
            // hole the other layers are read off their own radii for.
            let pie = sector_distance(in.uv, oct_sector(s, oct), strip_out);
            let wedge = max(pie, strip_in - d);
            cov = max(cov, gutter_coverage(wedge, reach, soft) * clamp(level, 0.0, 1.0));
        }
    }
    return cov;
}

/// Distance to the annulus between `inner` and `outer`, signed: negative inside
/// the band, and the radial distance to the nearer edge outside it.
fn annulus_distance(d: f32, inner: f32, outer: f32) -> f32 {
    return abs(d - 0.5 * (inner + outer)) - 0.5 * (outer - inner);
}

/// Distance to the nearest SLICE across the gaps that cut every ring on the
/// node, signed: negative inside a slice, and out in a gap the distance to the
/// nearer slice's own edge.
///
/// The ANGULAR half of a ring's footprint, where [`annulus_distance`] is the
/// radial one; [`glow_standoff`] intersects the two with a `max`. One walk
/// answers it for every RING at once, since one number cuts every angular slice
/// on the node (see [`slice_gap_half`]) and a ring is drawn at all of them.
///
/// A layer drawn at only SOME of them is answered off its own wedge instead
/// ([`slice_side_distance`]): the nearest slice to a fragment is the wrong slice
/// to measure once the layer is not in every one, and the walk has no way to
/// know which it landed on.
///
/// Measured to the boundary RAYS rather than to each slice in turn, which is
/// what keeps it one walk instead of `span` distance fields: the gaps are
/// straight bands of constant thickness laid along those rays, so what a
/// fragment stands out of the ink is half the gap less its distance to the
/// nearest of them, and inside a slice that same difference is how deep in it
/// stands. A slice the gap has EATEN WHOLE needs no case of its own — every
/// point of one is within half a gap of an edge that cuts it, so the walk
/// answers "no ink" unprompted, which is the clear hub `outer_glyph` leaves
/// around the node's centre.
fn slice_gap_distance(uv: vec2<f32>, d: f32, oct: OctRing) -> f32 {
    let gap_half = slice_gap_half();
    // No gap is no cut: the slices meet edge to edge and every ring is a solid
    // annulus, which is the ring's own radial distance and nothing else. `-d`
    // is at or below every answer the walk can give — no ray stands further
    // off a fragment than the node's centre does — so the `max` at the caller
    // keeps that distance exactly.
    if EARLY_OUT && gap_half <= 0.0 {
        return -d;
    }
    // The centre is where every boundary meets, so it is inside the gaps at
    // every angle and has no angle of its own to take.
    if d <= 0.0 {
        return gap_half;
    }
    // The fragment's angle in the walk the boundary table is written in —
    // clockwise from the ring's seam — brought onto one turn, which is what
    // keeps the differences below under one and the wrap exact.
    let walk = oct.seam - atan2(uv.y, uv.x);
    let a = walk - TAU * floor(walk / TAU);
    // Boundary `span` is boundary 0 a turn on (`oct_bound`), so the walk stops
    // one short of the table's end and the wrap is what closes the ring.
    var near = TAU;
    for (var j = 0u; j < oct_span(); j = j + 1u) {
        let delta = abs(a - oct_bound(j));
        near = min(near, min(delta, TAU - delta));
    }
    // A ray past a quarter turn cuts nothing here. `outer_glyph` takes each gap
    // only on the side its own edge runs to, so a fragment BEHIND an edge is ink
    // however close that edge's own line passes on its way through the centre —
    // and a slice wider than a half turn has a middle made of exactly those
    // fragments, which is the wedge `oct_arc_coverage` carries a union branch
    // for. `-d` hands the answer back to the ring's own radial distance, which
    // is what stands the light off ink no edge cuts.
    if near >= TAU * 0.25 {
        return -d;
    }
    // Off the nearest ray, perpendicular.
    return gap_half - d * sin(near);
}

/// How wide the standoff's fade is, in the node's uv: the Gap bar's own fade,
/// under one floor.
///
/// The floor is what stops the standoff ending in a STEP with the bar's two
/// handles together, and a step cut across a wide soft light crawls as the
/// camera moves where a band does not. This band is the ONLY hard edge the
/// light has — every other length in it is an exponential ([`glow_layer`]) — so
/// there is nothing else in the field for a step here to hide behind.
///
/// `soft` is the width taken, and it is the view's Clearance fade. That is
/// a floor and not a coupling: what it buys is one soft width for every node,
/// taken in the uv of a full-size node and divided by the
/// sheet's scale once in the vertex stage (`out.soft`), so one width is the
/// same fraction of a node's own radius on every sheet it is drawn at.
///
/// The Gap's own fade is the larger of the two at the fresh view and
/// binds there; the floor is what a person dragging that fade to nothing lands
/// on.
///
/// A width `soft` and not one `aa`, which is what the node's shape edges are
/// taken over: `aa` is a derivative, and the light's draw takes none anywhere
/// (see [`fs_glow`]).
///
/// A float rather than the node it came off, because the markers' own standoff
/// asks the same question ([`plus_standoff`]) and has no node to ask it of.
fn standoff_soft(soft: f32) -> f32 {
    return max(soft, glow_gap_soft());
}

/// How much of the light standing `sd` out from a ring's own annulus that ring
/// holds off: solid across the ring and out to `gap - soft`, then a decay whose
/// rate the Gap curve sets, spending [`GAP_TAIL`] e-folds by the Gap's own end
/// and going on falling past it.
///
/// A DECAY and not [`gutter_coverage`]'s ramp, which is the one place the Gap
/// bar and the Clearance bar part company, and it is the difference between
/// what the two are laid over. A clearing is cut into the picture, where its
/// outer end is met by whatever the neighbouring pixel already held; the
/// standoff is laid over the light, whose every other length is an exponential
/// (see [`glow_layer`]) and which is therefore smooth for as far as it goes. A
/// ramp landing on nothing at one radius puts a closed contour in that field
/// — the one edge in the whole of the light — and a circle is what the eye
/// finds in a smooth field however gently the ramp meets it. What the Gap bar's
/// two handles say is unchanged all the same: solid to the inner one, and back
/// to a fiftieth of the standoff by the outer.
///
/// The floor on `inner` is what keeps a fade wider than its gap (which the view
/// never stores, and a hand-edited scene can) from starting inside the ring, so
/// the ring's own footprint is stood off in full at every pair of handles; it
/// is also what keeps the `clamp` range from inverting, which WGSL leaves
/// undefined, and the same job [`clearing_edge`]'s floor does for
/// [`gutter_coverage`].
///
/// Both SIDES of a ring at once — the halo outside it and the lit middle of the
/// node inside it — because [`annulus_distance`] hands the two of them one
/// `sd`, so one decay feathers both and the light comes off a ring at the same
/// rate it comes off anything else. That match is the point: the gap and the
/// glow are one blur or the gap reads as a cut.
///
/// WHERE the fade is and HOW the light is spent across it are then two
/// questions, and the exponent answers the second alone. Every curve is the
/// same decay over the same width: `pow` is 0 at 0, so the coverage is still
/// exactly solid inside `inner`, and [`GAP_TAIL`] is what it has spent by
/// `edge` whatever the exponent. The curve therefore moves no handle.
///
/// It is the one place the standoff's two sides are treated UNEVENLY, which is
/// deliberate: the decay still softens the node's middle and its halo together,
/// but a curve below the middle spends most of its width on the far side, where
/// the light being recovered is the halo's. A ring wants its dark close and its
/// edge nowhere, and a symmetric ramp cannot give both.
///
/// The decay is brought to an end by a window of its own out at [`GAP_STOP`],
/// which is what the tail is allowed to cost rather than a shape anyone dials.
/// A decay is unbounded and a billboard is not, so the alternative is not an
/// endless tail but a tail cut off at the quad — and THAT contour is a
/// screen-aligned square, which is the one shape a field like this must not
/// have. The window's own reach is what the two glow billboards are sized to
/// ([`glow_gap_stop`]), so the coverage is exactly 0 where each of them ends.
fn standoff_coverage(sd: f32, soft: f32) -> f32 {
    let edge = clearing_edge(glow_gap());
    let inner = clamp(edge - soft, 0.0, edge - 0.001);
    let u = max(sd - inner, 0.0) / (edge - inner);
    return exp(-GAP_TAIL * pow(u, glow_gap_shape())) * (1.0 - smoothstep(1.0, GAP_STOP, u));
}

/// What a ring at `level` holds off `sd` out from its own annulus: the Gap depth
/// spent over [`standoff_coverage`], then closed by the level of the ink casting
/// it.
///
/// The level lands on the SHADE and not on the coverage, and that is the whole
/// of it: the coverage is an EXPONENT ([`gap_shade`]), so a level spent there is
/// a number of stops rather than a share of the light — at a depth of 1 a ring
/// a tenth of the way through its release still holds off half of what it held
/// off whole, and one at a fiftieth still holds off a seventh. What that draws
/// is a node whose ink has faded out of a halo that is still bright, with the
/// node's own footprint standing in it as a black silhouette until the last
/// frames, when the exponential lets go all at once.
///
/// On the shade it is what [`node_clearing`]'s levels already are: the ink's
/// own alpha, so ink at half strength holds off half the light and ink that is
/// gone holds off none. That is also the one reading the picture can be held
/// to, a ring's shadow being the thing the ring casts.
///
/// It costs the depth's mapping once per term rather than once per fragment,
/// which is what buying the right domain costs here: the levels differ per term,
/// so there is no order in which one mapping can answer for all of them.
fn ring_shade(sd: f32, soft: f32, level: f32) -> f32 {
    return gap_shade(standoff_coverage(sd, soft)) * clamp(level, 0.0, 1.0);
}

/// How much of the light standing here this node holds off: every RING it
/// draws, dilated by the Gap.
///
/// Answered in the LIGHT's own draw ([`fs_glow`]) and written to a layer beside
/// it, so what a node holds off is held off wherever the light reaches — its
/// NEIGHBOURS' as much as its own, that being what the melded field holds. The
/// node's own quad is the bound on that, and it is a bound that has to be sized
/// for: the light a node stands off is not its own, so its own light's span says
/// nothing about how far this has to reach. What does is the Gap, and
/// [`vs_glow`] grows the billboard by it.
///
/// A RING — the octave band, the audio ring, a mark's sector — is measured from
/// its own ANNULUS. That is the whole of what makes the middle of a node glow:
/// with nothing inside the innermost ring claiming the pixel, the light runs in
/// to the centre and stops one Gap short of that ring's inner edge.
///
/// [`node_clearing`] measures the same annuli, which is what makes the two
/// Shadow bars one family: what a node holds the light off is what it hides,
/// and neither is the empty circle its innermost ring encloses. What is still
/// its own function is the FALLOFF — a decay here against that one's ramp — and
/// the slice walk below, which the knockout has no use for.
///
/// A ring that reaches the centre itself has no such middle, its own annulus
/// being a disc: the standoff then covers what it covers, and the light is a
/// halo outside the node alone.
///
/// It is measured ACROSS the ring as well as along it: a ring is slices with
/// gaps between them, not a closed annulus, so every term is that annulus
/// intersected with an angular one. Without it a node stands the light off the
/// whole turn at every gap width — a dark ring under a set of slices that may be
/// nowhere near each other — and the gaps the picture is drawn with would be the
/// one length on the node the light cannot see.
///
/// WHICH angular one turns on how many of the node's slices the layer is drawn
/// at. A ring is drawn at all of them, so one walk to the nearest boundary
/// answers for the whole ring at once ([`slice_gap_distance`]); a mark is drawn
/// at the slices two voices took, so it is answered off its own wedge instead
/// ([`slice_side_distance`]), a boundary it has no part in being no edge of its.
///
/// Each term carries its own level, exactly as [`node_clearing`]'s do: a layer
/// that is off, refused by the stack, attacking or releasing opens and closes
/// its own standoff in step with the ink it stands off, and a layer nobody is
/// drawing holds nothing off. WHERE that level lands is [`ring_shade`]'s
/// business, and it is the difference between a release and a black silhouette.
///
/// So what a term answers is already the SHADE, and the max is taken over
/// those: the mapping is monotone, so the deepest standoff at a pixel still
/// wins, and it is the one operator the levels leave commutative — a max over
/// coverages, with a level on each, is a max over nothing the picture means.
fn glow_standoff(in: VsOut, d: f32, oct: OctRing) -> f32 {
    let soft = standoff_soft(in.soft);
    // The one angular length shared by the layers drawn at every slice, taken
    // once for both of them. The marks have their own, per wedge.
    let gap = slice_gap_distance(in.uv, d, oct);
    var shade = 0.0;
    // The octave band, on the node's own envelope, between the two radii its
    // slices and the glyphs inside them are drawn in — every slice of it, since
    // a silent one is the backdrop and the backdrop is drawn at the node's
    // presence.
    if u.misc3.z > u.misc3.y {
        let ring = max(annulus_distance(d, u.misc3.y, u.misc3.z), gap);
        shade = ring_shade(ring, soft, in.params.x);
    }
    // The audio ring, on ITS own: the view's Gate answered per node and carried
    // on the note Fade. A node nobody played wears one whenever the spectrum
    // reaches that gate, and this is the term that stands the light off it.
    if u.misc7.w > u.misc7.z {
        let ring = max(annulus_distance(d, u.misc7.z, u.misc7.w), gap);
        shade = max(shade, ring_shade(ring, soft, in.ring));
    }
    let slots = in.marks.x | in.marks.y;
    if slots == 0u || u.misc5.w <= 0.0 {
        return shade;
    }
    // Nothing below can raise a pixel already held off in full — the same skip
    // [`node_clearing`] takes, and exact for the same reason: a coverage of one
    // at a level of one is the deepest any term can answer, and [`gap_shade`] is
    // where that lands.
    if EARLY_OUT && shade >= gap_shade(1.0) {
        return shade;
    }
    let strip_in = u.misc4.y;
    let strip_out = u.misc4.y + u.misc5.w;
    let top = oct.base + i32(oct_span()) - 1;
    for (var i = 0u; i < OCTAVE_SLOTS; i = i + 1u) {
        let bit = 1u << i;
        let s = i32(i);
        if (slots & bit) == 0u || s < oct.base || s > top {
            continue;
        }
        // This slot's own level, not the strip's: where the two ends are on
        // different slices, a melody still fading out keeps its own wedge's
        // standoff while a bass arriving opens one.
        let level = max(
            select(0.0, in.params.y, (in.marks.x & bit) != 0u),
            select(0.0, in.params.z, (in.marks.y & bit) != 0u),
        );
        if level > 0.0 {
            // The mark is an annular SECTOR, which is the pie `sector_pie`
            // measures intersected with everything past the strip's inner edge
            // and with the gap its own sides are cut back by — a max of the
            // three distances, the way an intersection of fields always is.
            // Filled to the centre instead, one mark would put the node's whole
            // middle in shadow.
            //
            // Its OWN sides, off the same fold the pie is taken in, and not the
            // walk above: a mark is drawn at some of the node's slices rather
            // than all of them, and the walk answers for whichever slice is
            // nearest whether this mark has any part in it (see
            // `slice_side_distance`).
            let fold = sector_fold(in.uv, oct_sector(s, oct));
            let pie = sector_pie(fold, strip_out);
            let sd = max(max(pie, strip_in - d), slice_side_distance(fold));
            shade = max(shade, ring_shade(sd, soft, level));
        }
    }
    return shade;
}

/// The three derivatives-and-geometry answers every layer of a node is drawn
/// against, and the two early-outs that decide there is no node here at all.
///
/// One function because the SCENE pass and the GLOW pass both start here, and
/// what they must agree on exactly is where a node paints nothing: the glow is
/// grown from the ink the scene drew, so a fragment one of them keeps and the
/// other drops is ink with no halo or a halo with no ink.
///
/// It `discard`s rather than returning a flag. Both callers want exactly that
/// — a fragment this rejects writes no attachment in either pass — and the
/// derivatives below have to be taken in uniform control flow, which they are
/// here and would not be behind a caller's branch.
struct NodeGeom {
    /// Distance from the node's center in its own uv (0 at center, 1 at the
    /// octave band's outer limit).
    d: f32,
    /// The screen-constant softness every shape edge is taken over, in uv.
    aa: f32,
    /// How much of the shimmer's shared field one pixel spans, in world units.
    field_step: f32,
    /// Where this node's ring sits — which octaves it draws and how far it is
    /// turned.
    oct: OctRing,
}

fn node_geom(in: VsOut) -> NodeGeom {
    let d = length(in.uv); // 0 at center, 1 at quad edge (2x disc radius)

    // Screen-constant soft-band width: uv units per pixel (uv.x is linear
    // across the billboard, so fwidth is uniform over the quad and safe to
    // take before any branching), scaled to the softness knob. Shape edges
    // below use this instead of fixed-uv smoothsteps.
    let aa = aa_width(fwidth(in.uv.x));
    // How much of the shimmer's shared field one pixel spans, in that field's
    // own world units — what tells `shimmer_terms` when a pattern has run out
    // of pixels to be drawn in. Taken here, beside `aa` and for the same
    // reason: it is a derivative, and by the time the sheet is wanted the
    // shader has already been free to discard. The field's axes are the
    // camera's own right and up, which the screen's x and y run along, so the
    // larger of the two steps IS the world size of a pixel.
    let field_fw = fwidth(in.field);
    let field_step = max(field_fw.x, field_fw.y);

    // Outside everything this node can paint. `fwidth` above is taken first
    // and in uniform control flow, as its comment requires; from here on the
    // shader is free to leave.
    if EARLY_OUT && d > paint_reach(in, aa) {
        discard;
    }

    // An idle node paints NOTHING but its audio ring and the hole that ring
    // clears — no glyphs (a ghost needs presence), no mark rings (their own
    // levels gate them), and no marker
    // of its own — the cross at the position is its own instance (`fs_plus`),
    // not this node's to paint. Everything below still computes all of that and multiplies it away,
    // which on a lattice where most nodes are idle most of the time is most of
    // the fragment work in the frame. The three levels and the octave word are
    // exactly the terms those gates read, so this branch discards what the full
    // path would have discarded, not an approximation of it.
    //
    // The RING is the exception, and what it reads is not a level a node
    // carries: it is a window onto one shared spectrum, so it draws wherever
    // the view's Gate lets it whatever the keys are doing — silence included, at
    // the ramp's silent end, which is PINNED to u.lattice_ground and so reads as
    // the same empty grey the octave band's unlit slices carry. What is left to
    // skip is therefore radial rather than per node, and it is most of the quad:
    // the ring is a narrow annulus in a billboard reaching QUAD_MARGIN.
    // `spectral_ring` skips the same band from the other side, and it is also
    // where a node wearing NONE of its ring leaves — an idle node the gate has
    // closed is culled on the CPU and never reaches this shader at all.
    //
    // Its CLEARING is the exception's own exception, and it is a wider band than
    // the ring: the hole a layer punches is that layer's own annulus dilated by
    // one gutter (`node_clearing`), so an idle node's fragments are worth
    // keeping over that band. Skipping them would leave a ringing node with no
    // note drawing its ring straight onto the sheets behind it, the hole it
    // owes them cut short at the ring's own edge.
    let audio_annulus = spectral_radii();
    let ring_draws = audio_annulus.y > audio_annulus.x;
    let in_audio_ring = ring_draws
        && d >= audio_annulus.x - aa
        && d <= audio_annulus.y + aa;
    let in_ring_clearing = ring_draws
        && in.gutter > 0.0
        && in.ring > 0.0
        && d >= audio_annulus.x - clearing_edge(in.gutter)
        && d <= audio_annulus.y + clearing_edge(in.gutter);
    if EARLY_OUT
        && !in_audio_ring
        && !in_ring_clearing
        && in.params.x <= 0.0
        && in.params.y <= 0.0
        && in.params.z <= 0.0
        && (in.octaves.x | in.octaves.y | in.octaves.z) == 0u
    {
        discard;
    }

    // Where THIS node's ring sits — which octaves it draws and how far it is
    // turned — derived once for the whole fragment and handed to everything
    // below that draws a sector or points at one. It depends on the wheel and
    // the node's pitch class and on nothing per-pixel, so deriving it inside a
    // loop (or inside oct_sector, or per edge) would be the same answer
    // computed dozens of times over. After the idle branch above, which paints
    // no sector at all.
    let oct = oct_ring(in.cents);
    return NodeGeom(d, aa, field_step, oct);
}

/// What a node paints OF ITSELF at this fragment: every layer's ink,
/// premultiplied, and nothing of the hole it knocks out of the picture behind
/// it.
///
/// Split from [`node_paint`] so that the node's own picture and the hole it
/// knocks in everyone else's are two readable pieces rather than one function
/// of three hundred lines: what a layer paints is decided here, and what it
/// CLEARS is decided once, uniformly, over every layer at the end. The two
/// answers are different shapes — a layer's ink is a set of slices cut out of
/// its annulus, its hole is that whole annulus dilated by the reach — and
/// reading them together is what made the second easy to get wrong.
fn node_ink(in: VsOut, d: f32, aa: f32, field_step: f32, oct: OctRing) -> vec4<f32> {
    let activation = in.params.x;

    // A node is its RINGS and nothing else: the stack starts at the node's own
    // centre, so the innermost layer left on fills the middle with its own
    // sectors rather than sitting a padding out from a disc. What lights that
    // middle is the node glow, drawn in a pass of its own (`glow_layer`), and
    // it is the one light a node has — nothing here paints a halo.
    //
    // The ground the rings composite onto is therefore empty, and that IS the
    // picture where every layer is off: the marker standing at the position is
    // what says a node is there, and it is drawn under this pass.
    let presence = activation;
    var base_alpha = 0.0;
    var base_rgb = vec3<f32>(0.0);

    // Octave indicators, composited over that ground. Each slot fades on
    // its own envelope. Whichever element covers a pixel most strongly owns
    // its color there, and within one slot the color is a CONTINUUM rather
    // than a pair: a fully sounding glyph is its own pitch exactly, a silent
    // one is the rings' ground, and a slot part way through its envelope is
    // the two mixed by however much of it is lit — which is what a fade
    // between them IS.
    // The octave layer always draws — one glyph shape, no on/off. Which
    // octaves it shows is the per-node bitmask, and how much of the band it
    // covers is the band radii; there is nothing left for a switch to say.
    var glyph = 0.0;
    var glyph_rgb = u.lattice_ground.rgb;

    // Melody/bass mark geometry: one strip outside the octave band, standing
    // off it by the node's RADIAL padding (already spent — the strip's inner
    // edge arrives as misc4.y) and cut down its sides by the ANGULAR one, the
    // same padding one indicator stands off the next. That second one is what
    // makes an extension read as its slice continued rather than as a second
    // thing stuck to the end of it, and it holds however far out the first
    // stands the strip.
    let band_in = u.misc3.y;
    let band_out = u.misc3.z;
    let mark_thick = u.misc5.w;
    let mark_w = select(max(mark_thick, aa * MARK_RING_MIN_AA), 0.0, mark_thick <= 0.0);
    // The slot the STACK gave the strip (u.misc4.y): a padding out from
    // whatever layer the stack ended on — which is the band on a node that
    // draws one and whatever is inside it on a node that does not — and the
    // stack's own START on a node with no rings at all, where a padding would
    // stand off nothing and open a hole the size of itself. That start is the
    // Inner handle's radius rather than the node's centre, so a node drawing
    // nothing but its marks still wears them where its rings would have been.
    //
    // Headroom: that edge can be dialed out to 1.0, so the strip lives in the
    // QUAD_MARGIN margin. Cap it inside the billboard (a circle of radius
    // QUAD_MARGIN fits the square quad) and ease it off there, rather than
    // letting the corner clip it flat.
    let lim = QUAD_MARGIN - 0.02;
    let mark_in = min(u.misc4.y, lim);
    let mark_out = min(mark_in + mark_w, lim);
    // Sounding slots draw bright, tinted by their own pitch, each fading on
    // its own envelope; the silent ones draw as the backdrop's ghosts in the
    // loop below, riding the note's own presence so the whole ring fades
    // with the pitch class rather than outliving it.
    //
    // Over the ring's own SLICES rather than over the packing's slots: a ring
    // near the pitch limits reaches for octaves the packing has no room for,
    // and walking the slots instead would drop those and leave a wedge of the
    // backdrop missing. They never light (oct_slot_level), which is all they
    // should never do.
    // The band coverage every slot's glyph is scaled by — the same answer for
    // all of them, so it is taken once here. Zero is most of the node: the
    // band is a narrow annulus in a billboard that reaches QUAD_MARGIN, so
    // outside it the loop below can be skipped entirely rather than run to
    // reach zero `span` times over.
    // An empty pair is the octave layer dialled off (its width bar at 0), and
    // it takes the whole layer with it: the slot loop below only ever scales
    // this coverage, the backdrop rides it, and the marks' shimmer reaches the
    // slices through it.
    let band = select(glyph_band(d, band_in, band_out, aa), 0.0, band_out <= band_in);
    // The slots a melody or bass mark is extending (in.marks, the same
    // bitmasks `mark_extension` reads below), which is where the MARK layer's
    // sheet reaches into this one.
    let extreme_slots = in.marks.x | in.marks.y;
    // How much of this pixel is a slice a melody or bass mark extends, and
    // how strongly that mark is drawing there: the weight the MARK layer's
    // shimmer reaches the octave glyphs with, below. The slice's own shape,
    // so the sweep fades in exactly with the wedge's edges instead of at a
    // boundary of its own, times the same mark level the extension itself is
    // scaled by -- a released melody's slice stops shimmering as its mark
    // goes, rather than outliving it.
    var mark_slice = 0.0;
    for (var i = 0u; i < oct_span() && (!EARLY_OUT || band > 0.0); i = i + 1u) {
        let slot = oct.base + i32(i);
        let level = oct_slot_level(in.octaves, slot);
        if level <= 0.0 && presence <= 0.0 {
            continue;
        }
        let shape = outer_glyph(slot, oct, in.uv, band, aa);
        // This slot's bit in the mark masks, or none at all where the ring
        // names an octave the packing has no room for. The shift is CLAMPED
        // into the word rather than guarded by the range test alone: `select`
        // evaluates both arms, and a shift past the width is undefined.
        let in_range = slot >= 0 && slot < i32(OCTAVE_SLOTS);
        let bit = select(0u, 1u << u32(clamp(slot, 0, i32(OCTAVE_SLOTS) - 1)), in_range);
        let is_extreme = (extreme_slots & bit) != 0u;
        if is_extreme {
            mark_slice = max(mark_slice, shape * max(
                select(0.0, in.params.y, (in.marks.x & bit) != 0u),
                select(0.0, in.params.z, (in.marks.y & bit) != 0u),
            ));
        }
        // Ghosts carry the ring's shape in the rings' own ground, and a lit
        // slot is that ghost with its pitch painted OVER it — never one in
        // place of the other.
        let ink = oct_slot_ink(in, slot);
        let opacity = ink.w;
        let slot_rgb = ink.xyz;
        // The wedge enters ONCE, after the two layers are resolved: they are
        // the same shape at different opacities, and compositing their COVERED
        // FRACTIONS instead would count the antialiased edge twice and leave a
        // lit slice a brighter fringe than the silent ones it meets.
        let cov = shape * opacity;
        if cov > glyph {
            glyph = cov;
            glyph_rgb = slot_rgb;
        }
    }
    // Ease the glyph layer off across the billboard's margin instead of
    // letting the quad boundary clip it flat. The fade starts at uv 1.0 —
    // the outer band's own limit — so it touches nothing but what reaches
    // past the band: the aa fringe of a band dialed right out to the edge,
    // eased to zero by GLYPH_FADE_LIMIT.
    glyph = glyph * (1.0 - smoothstep(1.0, GLYPH_FADE_LIMIT, d));
    // The mark layer's sheet, which the glyphs take too -- over the slices a
    // melody or bass mark extends, and nowhere else. A mark is the octave it
    // names TOGETHER with the extension that says so, one slice in two
    // pieces, so light crossing the one crosses the other; a sweep that
    // stopped at the extension's edge would cut the mark in half at the gap.
    // It is taken here rather than with the marks below because this is where
    // the glyph layer is finished, and it is the same terms the marks
    // themselves use -- one sheet, read twice, not two sweeps that could
    // disagree.
    //
    // After the slot loop and after the margin taper: the sheet is a plane
    // crossing the lattice rather than anything per slot, and a peak must not
    // push the layer back out past the fade the taper just closed.
    let mark_shimmer = shimmer_terms(pulse_marks_mode(), in.field, field_step);
    // Taken down by the slice weight rather than applied flat, which is what
    // keeps the sheet inside the mark: `mark_slice` is how much of this pixel
    // is a wedge some ring points at, so a plain slice arrives at a SWING of
    // zero and draws exactly as it does steady. Scaling the swing and not the
    // shape, because the shape is where in the sheet this fragment sits and a
    // half-lit slice sits in the same place as a fully lit one -- it is how far
    // the sheet moves it that the weight is about. The mode needs no guard of
    // its own here -- `shimmer_terms` returns a zero swing when the pattern is
    // Off, so the scale is a no-op either way.
    let glyph_shimmer = vec2<f32>(mark_shimmer.x * mark_slice, mark_shimmer.y);
    // No clamp at white here, where an added light needs one:
    // `shimmer_light` fits its crest to the layer's own headroom, so the top
    // channel lands AT 1 and never past it. That guarantee is what the layers
    // below rely on, and they rely on it exactly -- each one premultiplies its
    // color by its coverage, so a channel left at 1.2 would not come back to 1
    // but to 1.2 times whatever fraction of the pixel it covers, and the ring
    // would grow a bright fringe on its half-covered edges under every peak.
    glyph_rgb = shimmer_light(glyph_rgb, glyph_shimmer);

    // The audio ring, over the octave layer. A layer of its own, and radially
    // disjoint from the band above and the marks below, so the three bands
    // of a node — audio ring, octave band, marks — simply stack outward.
    // OVER rather than under so that a hand-dialled pair of radii that does
    // overlap the band still shows the measurement: the band's own reading is
    // drawn twice over in that case (its wedge and its ghost), and the
    // spectrum's is not drawn anywhere else.
    //
    // After the shimmer, and deliberately outside it: the sheet belongs to the
    // marks and the slices they point at, and light crossing a measurement
    // would be a brightness nobody asked the analyzer for.
    let audio_radii = spectral_radii();
    let audio =
        spectral_ring(in, oct, in.uv, glyph_band(d, audio_radii.x, audio_radii.y, aa), aa);
    glyph_rgb = (audio.color * audio.cov + glyph_rgb * glyph * (1.0 - audio.cov))
        / max(audio.cov + glyph * (1.0 - audio.cov), 1e-4);
    glyph = audio.cov + glyph * (1.0 - audio.cov);

    // Melody/bass marks: each one its own octave's slice, continued into the
    // strip past the band. Their own layer, composited over the glyphs — a
    // sector's color is its pitch, which is what the octave layer is FOR, so
    // nothing here repaints one.
    //
    // The strip's radial coverage is taken once for both ends, and guarded
    // rather than left to `glyph_band`: with no thickness to draw at all the
    // two radii meet, and an annulus of zero width is not a coverage of zero —
    // the edge in and the edge out are the same smoothstep, and the product of
    // one with the other's complement peaks at a quarter halfway through it.
    // That is a quarter-covered mark on a node whose marks are switched off.
    let mark_strip =
        select(0.0, glyph_band(d, mark_in, mark_out, aa), mark_out > mark_in);
    let melody_cov = mark_extension(in.marks.x, oct, in.uv, mark_strip, aa) * in.params.y;
    let bass_cov = mark_extension(in.marks.y, oct, in.uv, mark_strip, aa) * in.params.z;
    // The two ends share the strip, so where they name DIFFERENT slices they
    // are angularly disjoint and where they name the same one they are the
    // same wedge in the same color — either way the stronger owns the pixel,
    // and the crossfade between two adjacent extensions is the one the octave
    // layer already runs between two adjacent indicators.
    var mark = max(melody_cov, bass_cov);
    // The marks' own shimmer (`mark_shimmer`, taken with the glyph layer's
    // above). ONE direction for both, not one each: they lie in one strip and
    // never overlap, so a single sweep crossing both reads as light passing
    // over the node, where two would read as two unrelated animations. The
    // mark's own color is what the fit is measured on, which is why this is a
    // second call and not the glyph layer's result reused -- a mark and the
    // slice it continues are different colors with different room above them.
    let mark_rgb = shimmer_light(
        select(in.bass_color.rgb, in.melody_color.rgb, melody_cov > bass_cov),
        mark_shimmer,
    );
    // Safety taper only. The radii above are already capped inside the
    // billboard (a circle of radius QUAD_MARGIN fits the square quad), so
    // this just keeps a soft edge from ending on the boundary; starting it
    // any earlier eats the mark, which at the default band (outer 1.0)
    // lives entirely in this margin.
    mark = mark * (1.0 - smoothstep(QUAD_MARGIN - 0.04, QUAD_MARGIN, d));
    glyph_rgb = (mark_rgb * mark + glyph_rgb * glyph * (1.0 - mark))
        / max(mark + glyph * (1.0 - mark), 1e-4);
    glyph = mark + glyph * (1.0 - mark);

    // The active note: glyph over (disc + glow), premultiplied.
    let active_alpha = glyph + base_alpha * (1.0 - glyph);
    let active_rgb = glyph_rgb * glyph + base_rgb * (1.0 - glyph);
    return vec4<f32>(active_rgb, active_alpha);
}

/// What a node paints at this fragment, clearing and all. The scene pass's two
/// entry points below return exactly this; they differ only in how many
/// attachments they write it to.
fn node_paint(in: VsOut) -> vec4<f32> {
    let g = node_geom(in);
    // A clearing-only draw takes no ink, and skipping the call is the point of
    // the branch rather than a saving on top of it: `node_ink` is the whole of
    // what a node costs per fragment, and this instance is drawn over the same
    // quad a second time. Safe to branch on because every length the ink is cut
    // with arrives from `node_geom` (`aa`, `field_step`) — nothing under here
    // asks the rasterizer how big the node is, so the derivatives are all taken
    // outside.
    var ink = vec4<f32>(0.0);
    if in.paint != PAINT_CLEARING {
        ink = node_ink(in, g.d, g.aa, g.field_step, g.oct);
    }
    let active_alpha = ink.w;
    let active_rgb = ink.xyz;
    let d = g.d;
    let oct = g.oct;

    // The knockout gutter, which every node the scene ships carries — the
    // reach is the view's constant, and what decides whether a hole appears is
    // the per-layer level `node_clearing` scales each term by. This is what
    // lets the sevens layer overlap the home sheet instead of needing clearance
    // of its own: the node clears its own footprint out of whatever was drawn
    // before it and sits in the hole.
    //
    // THREE things make it read as a hole rather than a dark blob stuck on the
    // picture, and it needs all of them:
    //
    //  - It clears to the GROUND (u.background), not to black. With no color
    //    of its own a premultiplied layer knocks out to black, and black is
    //    several shades darker than this skin's panel, so the cleared disc
    //    announced itself everywhere — including over empty lattice, where
    //    it should be invisible. Against the real ground it disappears
    //    wherever it crosses nothing.
    //  - It FADES rather than ending at a rim. A hard edge cutting across
    //    a lit ring reads as a bite taken out of it; a gradient reads as the
    //    small node sitting in front. The clearing is solid across the
    //    node's own footprint and eases off over the reach the setting buys.
    //  - It is the node's own RINGS, one reach out, layer by layer
    //    (`node_clearing`). A circle big enough to hold the node holds a good
    //    deal else besides: a node wearing one mark reaches its strip in one
    //    wedge, and a node wearing nothing but an audio ring reaches only that
    //    ring, so a circle that fits either clears a gap wider than the node,
    //    and what a hole says about which sheet is in front it then says about
    //    empty space — the node's own empty middle included, which is the
    //    widest such space there is.
    //
    // Compositing it UNDER the node's own paint is what the `(1 - active_alpha)`
    // term says: the ground fills the part of the quad the node itself leaves
    // empty and no more of it. What the node's own paint then takes of the
    // light is the WASH's business, at the bottom of this function, and is a
    // separate question from where the hole is.
    //
    // Each layer's own ENVELOPE is what scales its part of the hole — the same
    // level that paints that layer — so a clearing fades out exactly as the ink
    // in it does instead of outliving it. The width stays put while it does: a
    // hole that shrinks as it fades reads as the node retreating, and a hole
    // that holds full strength to the last frame (which is what scaling the
    // width alone did) vanishes with an audible pop.
    var gutter_cov = 0.0;
    if in.gutter > 0.0 && in.paint != PAINT_INK {
        gutter_cov = node_clearing(in, oct, d);
    }
    let final_alpha = active_alpha + gutter_cov * (1.0 - active_alpha);
    if final_alpha < 0.01 {
        discard;
    }
    // What the hole is cleared TO: the finished light standing at this pixel,
    // over the ground. The glow is composited at the bottom of the scene pass,
    // so a clearing painting the bare ground would stamp the light out exactly
    // where it is most wanted — in the middle of the node the light belongs to
    // — and the feature would come out as a ring of haze round a hole. Reading
    // the same target the composite laid down puts the light back, so a node's
    // clearing hides what is BEHIND it and nothing of the light in front of it.
    //
    // A premultiplied over, the light being premultiplied: its alpha is how
    // much of the pixel the light claims and its colour already carries that
    // weight.
    let coord = light_coord(in.clip_pos.xy);
    // The SAME mix the composite laid down (`glow_light`), which is what keeps
    // the clearing from painting a brighter light than the picture it stands
    // in — that difference would read as a halo drawn round every node.
    let light = glow_light(coord, glow_meld());
    // The STANDOFF, which is where the Gap bars land: the light is dimmed around
    // every ring the lattice draws (`glow_shade_tex`), so a ring stands in a pool
    // that brightens outward instead of on the flat maximum of a falloff
    // measured from its own node's centre. Without it the light is at its
    // brightest exactly where the rings are, on both of their sides, and the
    // ring drawn over that is a grey silhouette on a bright field.
    //
    // Read from the same coordinate as the light, off a layer written beside it
    // in the same pass, and NOT computed here. What a node holds off is a shape
    // in the LIGHT, and answering it here would bound it at what this node
    // paints; the light's own draw is where it is asked (`fs_glow`), and what is
    // left here is the one
    // multiply, and the composite under this node spells it identically
    // (blit.wgsl's `fs_glow_over`). Those two agreeing is the same contract the
    // Meld above keeps, and for the same reason.
    //
    // Premultiplied, so one factor on the whole vector is less light and not a
    // darker light: where a depth of 1 covers a pixel in full `lit` is nothing
    // and the ground comes back bare, which is that pixel with no glow in it at
    // all.
    //
    // Exact at the ends rather than branched: a shade of 0 is the light whole,
    // which is the answer everywhere the feature is off, and no `EARLY_OUT`
    // stands in front of it because a `textureLoad` and a multiply are cheaper
    // than the test. At reach 0, and on the single-attachment path that has no
    // glow pass at all, this binding is a 1x1 zero, so the light comes through
    // whole with no branch to take.
    let shade = clamp(textureLoad(glow_shade_tex, coord, 0).x, 0.0, 1.0);
    let lit = light * (1.0 - shade);
    let ground = lit.rgb + u.background.rgb * (1.0 - lit.a);
    // The WASH: the same field over the node's own INK, on a bar of its own. At
    // 0 the ink is exactly what it is with the glow off, byte for byte; at 1 the
    // whole field runs over it and the node melts into its own light; between
    // them a silent slice's grey lifts toward the colour of the halo it stands
    // in, so the node reads as a shape inside its light rather than a silhouette
    // cut out of it.
    //
    // The RAW `light` and NOT the shaded `lit` the ground below takes,
    // which is the whole difference a second bar makes: with the standoff's
    // remainder here, a depth of 1 would leave nothing to wash with, and the
    // dark pool and the tint could never be asked for together. What that costs
    // is that the two can be dialled into an INVERSION — ink brighter than the
    // ground it stands on, the standoff having cleared that ground bare — which
    // is a picture worth being able to ask for rather than one to stumble into,
    // and the fresh wash is the tint the fresh depth alone would have left.
    //
    // The ground below fills the `(1 - active_alpha)` the ink leaves and takes
    // its own share of the light on the way (`lit`), so each share of the
    // fragment is lit exactly once and the two are allowed to differ — that is
    // what the two bars are. Where they do, the node's own antialiased edge
    // cross-fades between them, which is a gradient across a pixel rather than
    // a step.
    let washed = wash_over(active_rgb, active_alpha, light.rgb);
    let with_ground = washed + ground * gutter_cov * (1.0 - active_alpha);
    return vec4<f32>(with_ground, final_alpha);
}

// ---- Resting markers -------------------------------------------------------
// One cross per home-sheet lattice position, drawn under the nodes, and the
// whole of what an unplayed lattice draws. Lines between the positions are NOT
// drawn — the field's regularity is what the eye reads the rows and columns
// off, and the ink they would cost goes to the notes. A cross is that argument
// at its sharpest: it draws exactly what a pair of gridlines draws where they
// meet, and nothing of what they draw between one meeting and the next.

struct PlusInstance {
    // xyz: the position's world center, w: the length of one arm in world
    // units, crossing to tip.
    @location(0) pos_radius: vec4<f32>,
    // rgb: the marker's own ink, a: this marker's opacity — which is also what
    // its pool and its shadow are worth (`PlusInstance::strength`).
    @location(1) color: vec4<f32>,
};

struct PlusVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // Distance from the crossing as a fraction of one ARM, per axis: 1 is a
    // tip and the quad reaches PLUS_QUAD_MARGIN past it. Expressed in the
    // marker's own arm rather than in the quad, so the two proportions the
    // uniform carries — the thickness and where the taper starts — are read
    // against it directly and cut every marker the same way whatever size it
    // is drawn at.
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

// How far a marker's billboard reaches past the tips of its arms, in arms.
//
// A margin for the same reason a node's QUAD_MARGIN is one: the edge's soft
// band is CENTRED on the shape's boundary, as every ring's is, so its outer
// half stands outside the shape and needs quad to stand in. Without it a small
// marker — where the band is a large share of the arm — gets that half cut
// flat by the quad, and the marker reads square-shouldered at exactly the size
// it is hardest to see. Measured off the rendered picture at the bottom of the
// arm bar: a shape of radius 5.6px carried ink out to 8px, so it wants 1.4
// radii of quad and a margin under that cuts the faint end of the band flat.
//
// It is a share of the ARM rather than a length, so the headroom shrinks with
// the marker while the band, being screen-constant, does not — which is why
// `plus_paint` also caps the band at what this holds rather than trusting the
// margin alone.
const PLUS_QUAD_MARGIN: f32 = 1.6;

// How much of the marker this fragment is inside, `uv` measured in the arm's
// own length and `aa` the soft band the whole shape is cut with.
fn plus_coverage(uv: vec2<f32>, aa: f32) -> f32 {
    // The cross, as the union of two bars — but folded into one. Reflecting
    // into the octant where x >= y maps the upright bar onto the flat one, so
    // a single box's distance field answers for both and there is no union to
    // take (and so no seam where two soft edges would cross and the band would
    // double up on the diagonals).
    let p = abs(uv);
    let q = vec2<f32>(max(p.x, p.y), min(p.x, p.y));
    // `misc5.x` is HALF the arm's thickness, as a share of its length: the bar
    // sets a whole thickness across the arm and `derive_plus_half_width` halves
    // it, because what the fold measures is the distance out from the arm's
    // own centre line. At 1 the box covers the octant and the cross is a filled
    // square, which is the top of that bar and not an accident of the field.
    let corner = vec2<f32>(q.x - 1.0, q.y - u.misc5.x);
    // The exact signed distance to that box: outside, the distance to its
    // nearest point; inside, how far in. Exact rather than approximate is what
    // makes the arms' inner corners as clean as their ends — an approximation
    // rounds them off at exactly the four places the shape is doing its work.
    let sd = length(max(corner, vec2<f32>(0.0))) + min(max(corner.x, corner.y), 0.0);
    // The four ends taper: an arm is solid out to `misc5.y` of its length and
    // fades to nothing by its tip, the way a line drawn into a node arrives at
    // nothing rather than stopping at something. `q.x` is the distance along
    // whichever arm this fragment is on — the same fold that spares the union
    // spares this a branch, since folding puts every arm on one axis.
    //
    // ALPHA rather than width. A cross narrowed to a point is four spikes,
    // which reads as a drawn glyph; one that fades stops being there, which is
    // what a marker running out has to say.
    //
    // Inside the crossing the two arms' claims meet, and `q.x` is the
    // Chebyshev distance there — so a fully tapered plus is brightest at the
    // centre and eases off in every direction at once, with no seam where the
    // arms overlap.
    //
    // The sides do NOT taper. Only the ends are being softened — where the
    // marker stops — and fading the sides as well would blur the plus rather
    // than let it reach out of its crossing.
    //
    // `derive_plus_taper_start` already holds the start short of the tip; the
    // guard is here too because a zero-width `smoothstep` has no answer and
    // this is the one line that would have to give it.
    let start = min(u.misc5.y, 1.0 - 1e-3);
    let taper = 1.0 - smoothstep(start, 1.0, q.x);
    return aa_inside(0.0, sd, aa) * taper;
}

@vertex
fn vs_plus(@builtin(vertex_index) vertex_index: u32, inst: PlusInstance) -> PlusVsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];
    // Camera-facing, like every other billboard here: a marker is a mark ON
    // the lattice rather than an object standing in it, so it keeps its shape
    // under an orbit instead of foreshortening into an ellipse.
    let reach = inst.pos_radius.w * PLUS_QUAD_MARGIN;
    let world = inst.pos_radius.xyz
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * reach;

    var out: PlusVsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner * PLUS_QUAD_MARGIN;
    out.color = inst.color;
    return out;
}

/// The two attachments the offscreen scene pass carries.
///
/// `picture` is everything, and is what the composite puts on screen.
/// `nodes` is the same picture with the node LABELS left out — the bright
/// pass reads it, so a name neither glows nor takes a bite out of the halo
/// of the node it covers. Every draw that is not a label writes the same
/// fragment to both, so the two differ in exactly one thing.
struct SceneOut {
    @location(0) picture: vec4<f32>,
    @location(1) nodes: vec4<f32>,
};

/// The node pipelines. `fs_main` is the single-attachment form, which the
/// parity test's direct-to-egui-pass reference draws through; `fs_main_scene`
/// is the one the offscreen pass uses.
@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return node_paint(in);
}

@fragment
fn fs_main_scene(in: VsOut) -> SceneOut {
    let paint = node_paint(in);
    return SceneOut(paint, paint);
}

// ---- Node glow -------------------------------------------------------------
// A node's own light, drawn into a target of its own and composited UNDER the
// finished lattice.
//
// It is the ONLY light a node has: every sounding octave's hue laid round the
// node by angle over an exponential falloff, windowed to nothing before the
// quad's edge. The falloff spans the node's outermost drawn edge plus the
// Reach, and the window shuts at the end of that same span, so the Reach bar
// says exactly how far a note's light goes and there is no second skirt at a
// second size for it to fight with.
//
// The COLOUR is settled first, in a pass of its own: a node's ink is read round
// the node once per frame and kept as a strip (see The ink strip below), and
// the light's draw samples it.
//
// ONE DRAW over the whole instance buffer, into one transparent target.
// `fs_glow` lays every node's light down, SCREEN-blended — src + dst*(1-src),
// premultiplied. Two halos meld: an overlap is brighter than either alone and
// still bounded by white however many nodes reach the pixel, and the blend is
// commutative, so nothing about the order inside the call reaches the picture.
// Adding instead makes the COUNT of overlapping nodes, rather than any note,
// the brightest thing on screen.
//
// NOTHING in the target is subtractive, and that is what lets the sheets meld
// into one layer rather than being assembled one at a time: it is light and
// light only, so a node hidden behind a nearer sheet has nothing to cut with —
// what it may do to a node in front of it is BRIGHTEN it, and only that.
// What hides its SHAPE is the scene pass, which draws every node over the
// finished light: a ring, a mark and a name are drawn whole there, with nothing
// to be stood off. See `LatticeCallback::prepare` for the order, and
// `node_paint` for the two things that read this target back, both of them the
// node's own and each on its own bar: the clearing, which paints the light over
// the ground instead of the ground bare and scales it by the standoff, and the
// wash, which is the share of the raw field the node's own ink takes.

/// How lit this node is, for the purpose of the light it gives off — carried on
/// the glow's own attack and release, and handed over per instance.
///
/// Its TARGET is the largest of every level that puts ink on the node, and the
/// note's own envelope is only one of them: a mark rides the marked VOICE's
/// level rather than the node's, and the audio ring rides the analyzer through
/// the view's Gate, so a node with no key down and a ring showing is a node
/// with something on screen. But this is where that target has GOT to, not the
/// target — a light runs slower than every layer under it, which is what makes
/// it read as light, and it can stand above zero on a node that has gone
/// silent entirely.
///
/// Settled on the CPU because that is where a node has an identity that
/// outlives a frame (`panes::glow_fade` in harmonigraph-ui). A shader has this
/// frame's instances and nothing else.
fn glow_level(in: VsOut) -> f32 {
    return clamp(in.glow.x, 0.0, 1.0);
}

/// How much two nodes' overlapping light adds up (`u.misc.z`): 1 the halos
/// screened together, 0 an overlap exactly as bright as the brighter node
/// alone. See [`glow_light`], which is the only place it is spent.
fn glow_meld() -> f32 {
    return clamp(u.misc.z, 0.0, 1.0);
}

/// The finished light at a pixel of the glow's target: the two blends the
/// light was written under, mixed by the Meld bar.
///
/// Every reader of the target takes it through here — the draws that read it
/// back (at [`light_coord`]) and the composite that lays it down (blit.wgsl's
/// `fs_glow_over`) — because ink painting a different mix from the picture it
/// stands in is a halo drawn round every node.
///
/// A mix of two premultiplied colours, so the result is premultiplied too, and
/// the max is bounded above by the screen at every texel: neither end can put
/// more light in a pixel than the screen already had.
fn glow_light(coord: vec2<i32>, meld: f32) -> vec4<f32> {
    let screened = textureLoad(glow_tex, coord, 0);
    let brightest = textureLoad(glow_max_tex, coord, 0);
    return mix(brightest, screened, meld);
}

/// Where in the glow's target one fragment of the scene pass stands: the pixel
/// under it, clamped into the texture.
///
/// Shared by the draws that read the target back rather than write it — a
/// node's clearing, its ink and the shade over both (`node_paint`), and a
/// resting marker's ink (`plus_paint`). The COORDINATE and not the light,
/// because the layers written beside it are read at the same pixel and one
/// clamp answers for all of them.
///
/// Clamped rather than trusted to the backend's out-of-bounds rule: WGSL lets a
/// load past the edge answer (0,0,0,1) as readily as zero, and an alpha of 1
/// from the 1x1 stand-in would clear every node to black. On the real target it
/// is a no-op, the target being the attachment's own size.
///
/// At reach 0, and on the single-attachment path that has no glow pass at all,
/// every one of those bindings IS that 1x1 texture and it holds nothing, so
/// each reader gets no light and no shade with no branch to take.
fn light_coord(frag_pos: vec2<f32>) -> vec2<i32> {
    let edge = vec2<i32>(textureDimensions(glow_tex)) - vec2<i32>(1, 1);
    return min(vec2<i32>(frag_pos), edge);
}

/// How flat the light's falloff is across its own span (`u.misc10.z`): 0 the
/// exponential heaped on the node, 1 an even field across it.
fn glow_feather() -> f32 {
    return clamp(u.misc10.z, 0.0, 1.0);
}

/// How widely a node's own ink is averaged into the colour of its light, as the
/// concentration that average is taken at (`u.misc10.w`): the bar's bottom is
/// GLOW_LOBE_KAPPA, where each layer's sectors stay distinct, and its top is no
/// concentration at all, one tint over the halo. Read by [`fs_ink_blur`], which
/// is where the average is taken.
fn glow_blend_kappa() -> f32 {
    return GLOW_LOBE_KAPPA * (1.0 - clamp(u.misc10.w, 0.0, 1.0));
}

/// One layer's angular soft-band width at radius `r`, in the node's uv: the arc
/// one column of the ink strip spans there.
///
/// Every edge [`ink_at`] reads is angular — the strip is one radius per layer,
/// so nothing crosses a RADIAL edge — and the strip resolves angle to
/// [`INK_STRIP_N`] columns, so this is the width the sampling itself asks for:
/// a seam softened over its own texel and no wider. That is a prefilter rather
/// than an anti-alias, and the difference is what it does NOT depend on: the
/// screen. A node's light is a property of the node, so a wedge must not change
/// colour as the camera moves toward it — which is exactly what deriving this
/// from `fwidth` would do, and it is why the glow's stage has no derivative in
/// it at all.
fn ink_arc(r: f32) -> f32 {
    return r * TAU / f32(INK_STRIP_N);
}

/// The light a node's MIDI layers give off in the direction `angle`: `xyz`
/// every such layer's colour times the weight it carries there, and `w` those
/// weights summed. Not a colour on its own — [`fs_ink_blur`] is what
/// normalises it.
///
/// **A generalised reading of what is ON the node**, and that is the whole
/// design. The light is not assembled out of a formula naming its sources, so
/// there is no source to forget: every layer that shines states its colour here
/// through the SAME function that draws it, sampled on that layer's own radius,
/// and a layer added to the node is lit by adding a term rather than by a case
/// in a hue picker. It also moves as the picture does — a slice a key lights, a
/// mark arriving — because it is that picture, read.
///
/// **The AUDIO RING is not one of them.** A node's light says something is
/// being PLAYED here, and the analyzer's ring says something is being heard in
/// the room — one is the node's own voice and the other is a reading it wears,
/// so the ring is drawn and never shone. It is the one layer left out of this
/// walk, and out of the level beside it (`panes::glow_fade` in harmonigraph-ui,
/// where the same rule decides how bright a light is); a node whose every drawn
/// layer is a ring gives off nothing at all.
///
/// **Each layer's share is how much LIGHT it puts on the node**: how lit it is
/// at this angle times the radial WIDTH the ring stack handed it, which is 0
/// for a layer switched off or refused the room. So widening the octave band on
/// the Layers bar moves the light toward the band's colours and narrowing the
/// mark strip takes the melody's back out of it, with no knob of its own.
///
/// **What a layer is drawing in the GROUND weighs nothing**, and the octave
/// band reaches that state in the one colour the Ground bar sets: an unlit
/// slice is `u.lattice_ground` exactly. That is BACKDROP — it carries the
/// ring's shape around the bright part of it, which is a thing to draw and not
/// a thing to shine — so the halo round a note voiced in one octave is that
/// octave's own colour instead of a tenth of it under nine tenths of grey, and
/// a node whose every layer is resting gives off nothing at all rather than a
/// grey haze (see [`glow_layer`], which stops on a `w` of 0).
///
/// It costs the light a way of saying "there is something here" about a node
/// nothing is sounding on. That is the LATTICE's job — the markers say where
/// the positions are and the rings say a node is there — and a halo is what
/// says something is playing.
fn ink_at(in: VsOut, oct: OctRing, angle: f32) -> vec4<f32> {
    let dir = vec2<f32>(cos(angle), sin(angle));
    var rgb = vec3<f32>(0.0);
    var wsum = 0.0;

    // The octave band: whichever slice owns this direction, at the colour and
    // opacity that slice paints — a lit slot's pitch, a silent one's ground, a
    // fading one the two mixed. The same walk the layer itself makes, and the
    // same `oct_slot_ink` at the end of it.
    let band_in = u.misc3.y;
    let band_out = u.misc3.z;
    if band_out > band_in && in.params.x > 0.0 {
        let mid = 0.5 * (band_in + band_out);
        let p = dir * mid;
        let arc = ink_arc(mid);
        var cov = 0.0;
        var owner = oct.base;
        for (var i = 0u; i < oct_span(); i = i + 1u) {
            let slot = oct.base + i32(i);
            let shape = outer_glyph(slot, oct, p, 1.0, arc);
            if shape > cov {
                cov = shape;
                owner = slot;
            }
        }
        if cov > 0.0 {
            // [`oct_slot_lit`] and not the drawn ink: what a slice is lit at is
            // its own level, where the ink's opacity is the node's PRESENCE —
            // the ghost included, which is the backdrop and weighs nothing.
            let ink = oct_slot_lit(in, owner);
            let w = cov * ink.w * (band_out - band_in);
            rgb = rgb + ink.xyz * w;
            wsum = wsum + w;
        }
    }

    // The melody/bass marks, in the strip past the outermost ring. One strip
    // and two ends, so the stronger owns the direction exactly as it owns the
    // pixel in `node_ink` — and the depth the strip is dialled to, capped at
    // the quad the way the layer's own is, is what weighs it.
    let mark_thick = u.misc5.w;
    if mark_thick > 0.0 && (in.marks.x | in.marks.y) != 0u {
        let lim = QUAD_MARGIN - 0.02;
        let mark_in = min(u.misc4.y, lim);
        // The strip's own floor is a SCREEN width (`MARK_RING_MIN_AA` times the
        // fragment's soft band), which is nothing the strip has: what the light
        // reads is the depth the marks are dialled to, and a mark too thin to
        // draw at this zoom is a mark whose colour is not in the halo either.
        let mark_out = min(mark_in + mark_thick, lim);
        let mid = 0.5 * (mark_in + mark_out);
        let p = dir * mid;
        let arc = ink_arc(mid);
        let melody = mark_extension(in.marks.x, oct, p, 1.0, arc) * clamp(in.params.y, 0.0, 1.0);
        let bass = mark_extension(in.marks.y, oct, p, 1.0, arc) * clamp(in.params.z, 0.0, 1.0);
        let cov = max(melody, bass);
        if cov > 0.0 && mark_out > mark_in {
            let w = cov * (mark_out - mark_in);
            rgb = rgb + select(in.bass_color.rgb, in.melody_color.rgb, melody > bass) * w;
            wsum = wsum + w;
        }
    }
    return vec4<f32>(rgb, wsum);
}

// ---- The ink strip ---------------------------------------------------------
// A node's ink, read once per node per frame instead of once per lit fragment,
// and blurred there rather than under every one of them.
//
// [`ink_at`] depends on the NODE and an angle and on nothing else about a
// fragment — no uv, no field, no derivative — so evaluating it per fragment was
// answering one question a node's worth of pixels over. Two passes settle it
// instead, both of them pure functions of the frame's own instances and
// uniforms:
//
//  - `fs_ink_strip` lays the reading down, one ROW per lit node and
//    [`INK_STRIP_N`] columns across the turn, mixed into what that row already
//    held on the glow's own attack and release.
//  - `fs_ink_blur` convolves each row with the von Mises lobe the Color blend bar
//    asks for and normalises it, so what it leaves behind is the finished
//    colour per angle. Its last column is the same average at no concentration
//    at all — the mean, which [`glow_ink`] eases the middle of a node toward.
//
// The light's own draw then reads two texels. What that buys is exact rather
// than approximate: the blur is a convolution of the whole turn, so it has no
// tap count to fall between, and the ripple a fixed set of taps left round
// every node — one dip per tap, in the light and in nothing else on screen —
// is gone by construction rather than pushed under a threshold.

/// The strip's raw draw: one node's whole ring of ink, on a quad one row tall.
///
/// The instance's own vertex stage does everything but the geometry, so what a
/// node carries into `ink_at` is spelled once and the strip cannot come to
/// disagree with the billboard about it. What is replaced is the two things a
/// row is not: where it sits — the row this node was handed, of `u.misc12.x` —
/// and its uv, which carries the ANGLE across the row rather than a position on
/// a node. Column `i` therefore falls at `(i + 0.5)/N` of a turn, which is what
/// [`glow_ink`] reads it back at.
@vertex
fn vs_ink_strip(@builtin(vertex_index) vertex_index: u32, inst: Instance) -> VsOut {
    var out = node_vertex(vertex_index, inst, 0.0, false);
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let rows = max(u.misc12.x, 1.0);
    let v = (out.strip_row + corner.y) / rows;
    out.clip_pos = vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - 2.0 * v, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 0.0);
    // A node with no light was handed no ROW either, and the two facts are one
    // fact: `GlowFade` gives a row only to a node that has a light and hands
    // everything else `GlowStep::default()`, whose row is 0 and whose mix is 1.
    // Such a node is still SHIPPED whenever it draws anything at all — an audio
    // ring is enough — so writing its ink here would settle it whole into the
    // row belonging to whichever node lit first. Collapsed to a point instead,
    // which rasterises nothing.
    //
    // The level is the right test rather than the row, because a row is only
    // ever ITS OWN while the light is: the frame a light ends the node still
    // carries its row, ships at a level of exactly 0, and has that row taken
    // back the same frame — so there is nothing left in it to keep.
    if inst.glow.x <= 0.0 {
        out.clip_pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    return out;
}

/// The reading, mixed into the one this row already held.
///
/// This is the COLOUR half of the glow's own clock, and the whole of it: the
/// level is carried on the CPU, and the colour cannot be, because what a node
/// is drawing is stated in WGSL by the same functions that draw it and a mirror
/// of that in Rust is the one thing this design refuses. So it is carried
/// where it lives — a row of the strip, read back from the strip this pass
/// wrote last frame.
///
/// PREMULTIPLIED, which is what makes the mix a colour rather than a fade to
/// black: `ink_at` returns each layer's colour times the weight it carries and
/// those weights summed, so a node going silent takes both to zero together
/// and their RATIO — the hue — is untouched on the way. `fs_ink_blur`
/// normalises at the end, so what a fading node's light keeps is the colour it
/// had at full, dimming. Mixing a normalised colour instead would drag the hue
/// toward whatever the empty texel says, which is black.
///
/// A mix of 1 takes the reading whole and does not touch the other strip at
/// all — a row just handed to this node holds a stranger's ink, and a strip
/// just rebuilt holds nothing.
@fragment
fn fs_ink_strip(in: VsOut) -> @location(0) vec4<f32> {
    let ink = ink_at(in, oct_ring(in.cents), in.uv.x * TAU);
    let carry = clamp(in.glow.y, 0.0, 1.0);
    if carry >= 1.0 {
        return ink;
    }
    let held = textureLoad(
        ink_strip, vec2<i32>(i32(in.clip_pos.x), i32(in.strip_row)), 0,
    );
    return mix(held, ink, carry);
}

/// The blur pass's quad: one node's row, exactly as the reading pass laid it
/// out, one column wider (see [`fs_ink_blur`]).
///
/// Over the INSTANCES rather than over the whole target, because the strip is
/// as tall as the row map's capacity and this frame lights whatever share of
/// that it lights — the rows in between belong to nobody, and blurring them
/// would be the one cost that grew with how many nodes have ever glowed at once
/// rather than with how many are glowing.
@vertex
fn vs_ink_blur(
    @builtin(vertex_index) vertex_index: u32,
    inst: Instance,
) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32(vertex_index & 1u), f32(vertex_index >> 1u));
    let rows = max(u.misc12.x, 1.0);
    let v = (inst.glow.y + corner.y) / rows;
    return vec4<f32>(corner.x * 2.0 - 1.0, 1.0 - 2.0 * v, 0.0, 1.0);
}

/// Each row of the raw strip, convolved with the Color blend bar's lobe and
/// normalised by the weights the ink itself carried — so the hue is the ink's
/// hue and how bright the light is stays [`glow_level`]'s answer.
///
/// The WHOLE turn, every column, rather than a window narrowed with the lobe:
/// the widest average the bar asks for is no concentration at all, and the
/// turn is only [`INK_STRIP_N`] texels wide, so there is nothing to save by
/// stopping early and a `w` that stays positive for a node lighting any part
/// of itself to keep by not.
///
/// One column PAST the strip is the mean — the same accumulation with the lobe
/// left flat, which is what `kappa = 0` makes of it, so the two are one loop
/// rather than a second pass over the same texels.
@fragment
fn fs_ink_blur(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    let col = i32(pos.x);
    let row = i32(pos.y);
    let kappa = select(glow_blend_kappa(), 0.0, col >= i32(INK_STRIP_N));
    var rgb = vec3<f32>(0.0);
    var wsum = 0.0;
    var lobes = 0.0;
    for (var i = 0u; i < INK_STRIP_N; i = i + 1u) {
        let off = (f32(i) - f32(col)) * (TAU / f32(INK_STRIP_N));
        let lobe = exp(kappa * (cos(off) - 1.0));
        let ink = textureLoad(ink_strip, vec2<i32>(i32(i), row), 0);
        rgb = rgb + ink.xyz * lobe;
        wsum = wsum + ink.w * lobe;
        lobes = lobes + lobe;
    }
    // The colour normalised by the ink's own weights, and the weight itself by
    // the lobe's — the first is a hue, the second is "how much light is
    // there", and a sum of sixty-four of them would be neither.
    return vec4<f32>(rgb / max(wsum, 1e-5), wsum / max(lobes, 1e-5));
}

/// One column of a node's finished strip, wrapped: [`INK_STRIP_N`] holds the
/// blur and the column past it the mean (see [`fs_ink_blur`]).
fn strip_texel(col: i32, row: i32) -> vec4<f32> {
    let n = i32(INK_STRIP_N);
    return textureLoad(ink_strip, vec2<i32>(((col % n) + n) % n, row), 0);
}

/// The colour of a node's light at a fragment: this node's finished strip, read
/// in the fragment's own direction and eased toward the strip's mean by `mix`.
/// `xyz` is that colour, `w` how much light the node gives off anywhere at all.
///
/// The blur itself is not here — the strip arrives already blurred, once per
/// node per frame — so what a lit fragment costs is two texels and a lerp,
/// whatever any setting reads. Column `i` holds the angle `(i + 0.5)/N` of a
/// turn, which is where its own fragment sat in [`fs_ink_strip`], so the
/// interpolation below lands exactly on a column at exactly that angle and the
/// wrap between the last and the first is one more step of the same walk.
///
/// The MIX is the node's own seam argument, kept: the ink is
/// laid in angle, so an arc shrinks with the radius and every seam between two
/// layers' colours would converge to a cusp at the node's middle. The mean is
/// the same strip at no concentration at all — one flat tint — so easing toward
/// it as the radius falls holds the seams at the width they have out at the
/// light's own edge, and only ever loosens them. It is also the one honest
/// reading of the middle: there is no direction left to have a colour for.
///
/// A `w` of 0 is a node giving off NOTHING: every layer off, every level at
/// zero, or every layer resting in the ground, which weighs nothing
/// ([`ink_at`]). Read off the MEAN, which is the whole turn averaged and so is
/// above zero for a node lighting any part of itself. There is no colour to be
/// had there and none is invented — see [`glow_layer`], which stops rather than
/// lighting a black halo.
fn glow_ink(in: VsOut, angle: f32, mix_out: f32) -> vec4<f32> {
    let row = i32(in.strip_row);
    // The column this angle falls between, in the strip's own coordinate:
    // column i sits at angle (i + 0.5) * TAU / N.
    let x = angle / TAU * f32(INK_STRIP_N) - 0.5;
    let base = floor(x);
    let lit = mix(
        strip_texel(i32(base), row),
        strip_texel(i32(base) + 1, row),
        x - base,
    );
    let mean = textureLoad(ink_strip, vec2<i32>(i32(INK_STRIP_N), row), 0);
    return vec4<f32>(mix(mean.xyz, lit.xyz, mix_out), mean.w);
}

/// The node's light at this fragment, premultiplied, exactly as every other
/// layer here returns its ink.
fn glow_layer(in: VsOut, d: f32) -> vec4<f32> {
    let level = glow_level(in);
    let reach = max(u.misc10.x, 0.0);
    let strength = max(u.misc10.y, 0.0);
    // ONE length under the whole layer: the node's outermost drawn edge as the
    // LIGHT has it ([`glow_rim`]) plus the Reach. It is the falloff's domain,
    // so the halo is a field the node sits inside rather than a rim light on
    // its edge, and it is where the window shuts, so the Reach bar says exactly
    // how far the light goes.
    //
    // Not the quad's own margin, which is the tempting reading of "window it at
    // the edge": `quad_margin` floors at QUAD_MARGIN, so on a small reach the
    // billboard is wider than the light has any business being and every reach
    // under that floor would draw one width of halo. The guarantee runs the
    // other way instead — the quad is SIZED to hold this, with room to spare
    // (`node_vertex`), so the light is never clipped square at the corners.
    let span = max(in.glow.z + reach, 0.1);
    // Past the window there is no light, and saying so by returning nothing is
    // what lets [`fs_glow`] keep ONE exit for its three attachments: a
    // `discard` here would take the standoff's write with it, and a node's
    // rings can stand off a neighbour's light out where this node's own has
    // shut. Not an early-out either, and so needs no `EARLY_OUT` of its own —
    // `window` is exactly 0 out here, which carries `skirt` and the coverage
    // below it to 0 by the same arithmetic the slow path runs.
    if d >= span {
        return vec4<f32>(0.0);
    }
    // The window, which is where the light STOPS: full to half the span and
    // shut by the end of it, at every setting of every bar. What the Feather
    // moves is the falloff underneath it — how much light is left to be shut
    // off out there — so the Reach alone says how far the light goes.
    let window = 1.0 - smoothstep(span * 0.5, span, d);
    let rate = mix(GLOW_FALLOFF_TIGHT, GLOW_FALLOFF_FLAT, glow_feather());
    // The falloff, plain: one exponential from the node's exact centre outward,
    // with nothing shaping the middle apart from what the node draws over it.
    // A node's middle gets no treatment of its own — whatever the Reach and
    // the Feather do out in the halo, they do at radius 0 too.
    let skirt = GLOW_BASE * exp(-rate * d / span) * window;

    // How much of the strip's own DIRECTION this fragment gets, against the
    // flat tint of its mean. Every seam converges to a cusp at the node's
    // middle otherwise, the ink being laid in angle, so an arc shrinks with the
    // radius; easing to the mean instead carries the arc the seam has where the
    // ink ends inward unchanged.
    //
    // The reference length is the node's own RIM and NOT the glow's span, which
    // is the tempting reading of "ease out over the light" and takes the colour
    // out of the light altogether: the ramp is quadratic and the skirt is an
    // exponential over the same length, so measuring it against `span` leaves
    // the light about a fifth directional however far the Reach is dialled —
    // the halo is then the flat mean nearly everywhere, and the Color blend bar,
    // which is exactly the concentration that mean is NOT taken at, has almost
    // nothing to move. Against the rim the light is fully the node's own
    // colours from the ink's outer edge outward, which is all of the halo
    // proper, and the easing is left doing the one job it is for.
    //
    // Floored where the span is and for the same reason: a node drawn down to
    // almost nothing has almost no rim, and a ramp measured against that is
    // full direction at every radius it has — which is the cusp back again,
    // this time on the one node too small to hide it under its own ink.
    let seam = max(in.glow.z, 0.1);
    let mix_out = min(1.0, (d * d) / (seam * seam));

    // The level scales the COVERAGE, once, and not the blend as well: a note
    // halfway through its attack should lay down half as much of its colour,
    // not a quarter of a darker one.
    //
    // Ahead of the colour because a fragment laying down nothing has no use for
    // one, cheap as the strip has made it. Exact rather than a threshold, so it
    // is not an early-out and needs no EARLY_OUT of its own: a coverage of zero
    // returns the same nothing either way.
    let alpha = clamp(skirt * level * strength, 0.0, 1.0);
    if alpha <= 0.0 {
        return vec4<f32>(0.0);
    }
    // The colour: this node's own strip, read in this direction. Not a hue
    // assembled out of the voices — see `ink_at`, which is where a layer states
    // what it is putting on the node and how much of the node that is.
    let ink = glow_ink(in, atan2(in.uv.y, in.uv.x), mix_out);
    // A node lighting NOTHING gives off nothing, and the level above does not
    // say so: it is the largest envelope on the node, and a view with every
    // layer dialled off leaves a held note a full envelope with no ink under
    // it — as does a node the audio ring is showing on with nothing sounding
    // anywhere in it, whose every wedge is the ground. Lighting either would be
    // a black halo, or a grey one: a colour invented for a node that has none.
    if ink.w <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(ink.xyz * alpha, alpha);
}

/// What a fragment of the light's draw emits. The light goes to two of the
/// three attachments as ONE value — what differs between those is the blend
/// they are written under, which is fixed-function state and not something a
/// fragment can vary (see `create_glow_pipeline`) — and the standoff goes to
/// the third, being a different quantity rather than the same one blended
/// twice.
struct GlowOut {
    @location(0) screened: vec4<f32>,
    @location(1) brightest: vec4<f32>,
    /// Coverage in x; the target has no other channel to write.
    @location(2) shade: vec4<f32>,
};

/// What a standoff coverage of `cov` leaves of the light, as
/// [`glow_shade_tex`] carries it: the Gap depth, spent over that coverage.
///
/// The depth is spent GEOMETRICALLY — a coverage of `c` keeps `(1 - depth)` to
/// the power `c`, so the Gap depth is a number of stops and the coverage says
/// what share of them a pixel takes. A factor `1 - depth * c` is the tempting
/// reading and is the wrong domain: sight answers ratios, so a factor walked
/// evenly in VALUE spends most of what can be SEEN of it in the first fraction
/// of its width — at a depth of 0.85, half the visible swing is gone by a fifth
/// of the way out, and what is left over the remaining four fifths is under the
/// eye's own threshold. The pool then reads as a dark ring hugging the ink with
/// an edge on it, whatever width the Gap is dialled to.
///
/// The ends are unmoved by that: `pow` is 1 at 0 and `1 - depth` at 1, so a
/// pixel the standoff misses keeps the light whole and one it covers is the
/// depth's own floor. [`GAP_KEEP_FLOOR`] is what stands under that floor, a
/// depth of 1 having no ratio to walk.
///
/// It blends as it does: the mapping is one function of the coverage for every
/// emitter, and monotone in it, so a `max` over what each of them holds off is
/// still a `max` over the shade they write.
///
/// Its own function because every ring that writes the third attachment ends
/// here — [`ring_shade`] is the one caller, and both draws reach it through
/// that — and what makes the Gap bars ONE setting across a node's rings and a
/// marker's cross is that they meet at this mapping rather than each spending
/// the depth their own way.
fn gap_shade(cov: f32) -> f32 {
    return 1.0 - pow(max(1.0 - glow_gap_depth(), GAP_KEEP_FLOOR), clamp(cov, 0.0, 1.0));
}

/// How much of the light standing here this node's rings hold off:
/// [`glow_standoff`], which is already the depth spent per ring.
///
/// The exit a depth of 0 buys is stated here rather than inside the mapping,
/// which is where the walk it skips lives. It is EXACT rather than an
/// approximation, which is what lets it stand outside `EARLY_OUT`: a depth of 0
/// keeps every scrap of the light by definition, and `u.misc11` is zeroed whole
/// with the glow, so the whole feature being off is the same answer arrived at
/// by the same line — where [`glow_standoff`] is otherwise a walk over every
/// mark per fragment for nothing.
fn glow_shade(in: VsOut, d: f32) -> f32 {
    if glow_gap_depth() <= 0.0 {
        return 0.0;
    }
    return glow_standoff(in, d, oct_ring(in.cents));
}

/// The light draw, and the standoff cut into it. No depth in it: this is a pass
/// of its own ahead of the scene's, so every node's halo melds into one layer
/// before any node is drawn over it, and no sheet's light is legible as having
/// come first.
///
/// The STANDOFF rides here rather than in [`node_paint`] because it is a shape
/// in the light and not a shape on a node: a node holds the light off its own
/// rings, and the light it is holding off is the melded field — its neighbours'
/// as much as its own. Answered per node in the pass that already draws one
/// quad per node, melded by its attachment's own `max` blend, and read back by
/// both the composite and every clearing. What that gives up is the depth
/// ORDERING a shape painted on a node's body had for free: the field has no z,
/// so a ring on a sheet behind can put its band through a nearer node's halo.
/// The light has always been unordered in exactly this way; the difference is
/// that light can only brighten what stands over it and a standoff can only
/// darken it.
///
/// Its own early-out rather than `node_geom`'s, and this is the reason it does
/// not share that function: `paint_reach` bounds what a node PAINTS, which the
/// glow reaches past by the whole Reach, and the idle branch keeps fragments
/// this layer has no colour for. What the glow needs is narrower on both counts
/// — a node doing nothing at all emits no light, and neither does anything past
/// where its own window has shut.
///
/// It takes BOTH answers before it leaves, and that is what the third
/// attachment costs: a node with no light of its own still stands its rings off
/// a NEIGHBOUR's halo, so `glow_level` alone is no longer a reason to go. Exact
/// all the same — writing zero to any of the three is that attachment's own
/// identity, a screen blend and a `max` blend both leaving the destination
/// alone at 0.
///
/// No derivative anywhere in it, unlike every other fragment entry point here,
/// and that is the strip's doing: the shapes the light is coloured out of are
/// read in [`fs_ink_strip`] at the strip's own angular rate, so nothing in this
/// stage asks how big the node is on screen. The standoff keeps that property —
/// every length in it is read against `in.soft`, which the vertex stage
/// converted out of pixels once (see [`standoff_soft`]).
@fragment
fn fs_glow(in: VsOut) -> GlowOut {
    let d = length(in.uv);
    let shade = glow_shade(in, d);
    if EARLY_OUT && shade <= 0.0 && glow_level(in) <= 0.0 {
        discard;
    }
    let light = glow_layer(in, d);
    return GlowOut(light, light, vec4<f32>(shade, 0.0, 0.0, 0.0));
}

/// What a resting marker paints; see [`node_paint`] for why the entry points
/// are two.
fn plus_paint(in: PlusVsOut) -> vec4<f32> {
    // A marker's edge is a RING's edge: `aa_inside` at its boundary, carrying
    // the one screen-constant soft band the octave band and the audio ring are
    // cut with. A marker has no softness of its own to dial, so the resting
    // field and the layers that stand on it come to an end the same way, at
    // every zoom.
    //
    // The band's width is taken off `fwidth` of a quad AXIS, which is the
    // rings' choice too and is load-bearing rather than incidental: the
    // derivative of the RADIAL coordinate is half again as wide on the diagonal
    // as on the axes, so a band measured that way pinches the circle at 45° by
    // about a fifth of its radius. An axis has one derivative everywhere on the
    // quad, so the band closes evenly all the way round.
    //
    // Capped at the margin, which binds only where a marker is a few pixels
    // across and the band would otherwise want more quad than there is. One
    // that small trades softness it cannot show for a shape it can: the band
    // narrows, and the arms keep their ends instead of squaring off against
    // the quad at the bottom of the bar.
    let aa = min(aa_width(fwidth(in.uv.x)), PLUS_QUAD_MARGIN - 1.0);
    let alpha = in.color.a * plus_coverage(in.uv, aa);
    if alpha < 0.01 {
        discard;
    }
    // Premultiplied, as every draw in this pass is: the marker IS its own ink
    // rather than a brightness of it, so its colour is laid down flat and only
    // its coverage varies across the edge.
    let ink = in.color.rgb * alpha;
    // The WASH, on the same bar and out of the same field a node's ink takes it
    // from (`wash_over`). A marker is ink laid over ground the light is already
    // under, so at a wash of 0 a marker inside a halo comes out DARKER than the
    // ground to either side of it — the resting field reading as holes punched
    // in the light exactly where the light is brightest. What the bar says of a
    // node's rings it says here, and a lattice whose structure sits inside the
    // light is the whole of what it buys.
    //
    // The RAW light, as a node's ink takes it: the shade layer beside it is the
    // GROUND's share of the field and the Gap bars' answer, and a wash that read
    // the remainder could not be asked for alongside a dark pool at all.
    let light = glow_light(light_coord(in.clip_pos.xy), glow_meld());
    return vec4<f32>(wash_over(ink, alpha, light.rgb), alpha);
}

@fragment
fn fs_plus(in: PlusVsOut) -> @location(0) vec4<f32> {
    return plus_paint(in);
}

@fragment
fn fs_plus_scene(in: PlusVsOut) -> SceneOut {
    let paint = plus_paint(in);
    return SceneOut(paint, paint);
}

// ---- The markers' own light -------------------------------------------------
// A pool under every resting marker, written into the same target a node's halo
// is and under the same two blends, so the resting field is made of the same
// light the notes are rather than of a second kind standing beside it.
//
// The STANDOFF is written here too, off the same quad and into the same third
// attachment, on the Gap bars a node's rings are held off by. A marker is ink
// standing in the light exactly as a ring is, so what it holds off is a shape
// in the light rather than one on the marker — and a lattice where the notes
// are dark-rimmed and the resting field is not reads as two pictures laid over
// each other. Sharing the bars is what makes it one.

// How far a marker's pool reaches from its crossing, in WORLD units
// (`u.misc13.z`): the marker's own arm plus the Marker reach bar, resolved for
// the whole field on the CPU (`derive_marker_span` in harmonigraph-scene).
//
// A bar of its own and NOT the Reach, which is the tempting reading, the Reach
// being where every other light in this picture stops. What separates them is
// how many emitters each distance is spent on: the Reach is the distance ONE
// sounding node's light is given, and how few nodes sound at once is what bounds
// where it lands. There is a marker at every lattice position and all of them
// emit always, so the reach that makes a node's halo into a field makes the
// marker field into one flat wash with the structure gone out of it.
//
// Already a length rather than a uv, because the pool's own draw has no node to
// measure a uv against: `vs_plus_glow` sizes its billboard off this before it
// knows which marker it is drawing.
fn marker_span() -> f32 {
    return max(u.misc13.z, 0.0);
}

// One quad uv as a world length on the sheet the markers stand on
// (`u.misc13.w`, `Scene::marker_unit`), which is what puts the two halves of
// this draw into one unit: the pool's own lengths arrive in world, and the Gap
// it stands the light off by is a node's bar, in uv.
//
// Floored above zero because it divides. A scene at no spacing at all draws no
// markers, so what the floor is worth is finite arithmetic on the way to
// drawing none.
fn marker_unit() -> f32 {
    return max(u.misc13.w, 1e-6);
}

// The pool's span in that same uv: where `plus_glow_layer` shuts the light, and
// one of the two lengths the quad is sized to hold.
fn marker_span_uv() -> f32 {
    return marker_span() / marker_unit();
}

struct PlusGlowVsOut {
    @builtin(position) clip_pos: vec4<f32>,
    // Where this fragment stands, measured out from the crossing in the QUAD UV
    // every glow bar is dialled in — the unit a node's own `VsOut::uv` carries,
    // so one Gap is one length across the two draws.
    @location(0) uv: vec2<f32>,
    // This marker's arm, in that same uv: what [`plus_standoff`] measures its
    // cross out from. Carried per instance because the arm is, one bar setting
    // every marker's or not.
    @location(1) @interpolate(flat) arm: f32,
    // The marker's opacity, which the pool and the SHADOW take with the ink: a
    // position whose NAME is fading in hands all three over together
    // (`derive_pluses`), so neither the light standing in a cross nor the
    // standoff it writes can outlive the cross itself.
    @location(2) @interpolate(flat) strength: f32,
};

@vertex
fn vs_plus_glow(@builtin(vertex_index) vertex_index: u32, inst: PlusInstance) -> PlusGlowVsOut {
    var corners = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, -1.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex_index];
    let unit = marker_unit();
    // `misc13` is zeroed whole where the glow is off, so a unit of nothing is
    // the marker field having neither light nor shadow to draw — and the floor
    // `marker_unit` puts under the division would otherwise turn an arm into a
    // quad the width of the pane. The collapse is what the row means, said here
    // rather than left to the floor.
    let arm = select(0.0, inst.pos_radius.w / unit, u.misc13.w > 0.0);
    // Two lengths reach past the crossing and the quad is sized to the larger,
    // which is `vs_glow`'s rule for a node and holds here for the same reason:
    // the light shuts at the span, and the standoff holds the light off out to
    // where its own window has shut ([`glow_gap_stop`]). The two are independent
    // bars, so neither bounds the other, and a standoff outside a quad sized to
    // the span alone is a fade cut off partway down — a screen-aligned step, the
    // quad being built from `cam_right`/`cam_up`.
    //
    // Exact rather than generous at both: the light's window is 0 AT the span,
    // and the standoff's is 0 at the stop.
    let margin = max(marker_span_uv(), arm + glow_gap_stop());
    // Camera-facing on the same plane as the marker itself (`vs_plus`), so the
    // pool sits square on the cross under any orbit.
    let world = inst.pos_radius.xyz
        + (u.cam_right.xyz * corner.x + u.cam_up.xyz * corner.y) * margin * unit;

    var out: PlusGlowVsOut;
    out.clip_pos = u.view_proj * vec4<f32>(world, 1.0);
    out.uv = corner * margin;
    out.arm = arm;
    out.strength = inst.color.a;
    return out;
}

/// One marker's light at a fragment, premultiplied — `glow_layer`'s falloff with
/// the node's part taken out of it.
///
/// The SHAPE is shared deliberately, term for term: the same window shutting the
/// light at the span, the same exponential under it, the same Feather choosing
/// how flat that exponential is, and the same Strength over the whole thing.
/// What a light looks like is one answer in this picture, and a marker's pool
/// asking it separately would be a second kind of light rather than less of the
/// one there is.
///
/// What it does NOT share is the node's COLOUR path. A node's halo is the ink
/// the node is drawing, read round it and blurred (`glow_ink`); a marker draws
/// one flat grey and there is no direction in it to read. The pool is NEUTRAL
/// white, which is the same hue that grey has — none, the ground carrying none
/// by construction — and taking the colour off the ink instead would put back
/// exactly the coupling this bar exists to break: a ground dialled to black
/// would emit black, and the marker would go dark with the structure again.
fn plus_glow_layer(in: PlusGlowVsOut) -> vec4<f32> {
    let span = marker_span_uv();
    let d = length(in.uv);
    // Past the window there is no light, and returning nothing rather than
    // discarding is what lets [`fs_plus_glow`] keep ONE exit for its three
    // attachments — `glow_layer`'s reason exactly: the quad reaches past the
    // span whenever the Gap does, and a marker holds the light off out there.
    // It is also what makes the division below safe, `span` being strictly
    // above `d >= 0` past this line.
    if d >= span {
        return vec4<f32>(0.0);
    }
    // The window, as `glow_layer` spells it: full to half the span, shut by the
    // end of it, whatever the Feather does to the falloff underneath.
    let window = 1.0 - smoothstep(span * 0.5, span, d);
    let rate = mix(GLOW_FALLOFF_TIGHT, GLOW_FALLOFF_FLAT, glow_feather());
    let skirt = GLOW_BASE * exp(-rate * d / span) * window;
    let alpha = clamp(
        skirt * marker_light() * in.strength * max(u.misc10.y, 0.0),
        0.0,
        1.0,
    );
    return vec4<f32>(vec3<f32>(alpha), alpha);
}

/// How much of the light standing here this marker's cross holds off: its own
/// distance field under [`ring_shade`], which is the same Gap bars a node's
/// rings are stood off on and the same place the depth is spent.
///
/// The CROSS's own distance field, which is [`plus_coverage`]'s — the same fold
/// onto one box, so the shadow is the shape the marker draws rather than a disc
/// around it, and the four inner corners are as clean in the shadow as they are
/// in the ink. Measured in the LIGHT's uv rather than the arm's, which is the
/// one conversion the two fields differ by: a signed distance scales with the
/// coordinate it is taken in, so the box's half-extents carry the arm here
/// where there they are 1 and `misc5.x`.
///
/// The arm is taken at its SOLID length — out to where the taper starts, not
/// out to the tip. Past that the ink is running out (`plus_coverage` fades an
/// arm to nothing by its end), and ink that is running out casts no shadow of
/// its own; the alternative is a shadow standing off a tip that is barely
/// there. The tapered part is not left in the light either, the standoff
/// reaching one Gap past where it starts — at the fresh bars that is past the
/// tip and then some, and it takes a Gap narrower than the taper is long
/// before any of the faint end stands outside its own shadow.
///
/// Multiplying the coverage BY the taper is the tempting reading and is wrong
/// at the one place it matters: the taper is 0 at the tip by construction, so
/// every fragment outside an arm reads the ink it stands off as absent and a
/// square-ended marker gets no shadow at all.
///
/// Closed by the marker's own OPACITY, the one number a marker hands over: ink
/// that is not there holds nothing off, and ink half way in holds half. So a
/// position handing itself back as a name fades off it grows its cross and the
/// shadow under that cross on one clock, where a shadow closed against the
/// LIGHT instead arrives on the Glow release — seconds behind a cross that is
/// already whole, and for no reason the picture shows.
fn plus_standoff(in: PlusGlowVsOut) -> f32 {
    // An arm of 0 draws no markers (`derive_pluses`). The marker draw is left
    // to say so by its own quad collapsing; this one's does not, the Gap
    // holding it open, so the emptiness is stated here instead.
    if in.arm <= 0.0 {
        return 0.0;
    }
    let p = abs(in.uv);
    let q = vec2<f32>(max(p.x, p.y), min(p.x, p.y));
    let solid = clamp(u.misc5.y, 0.0, 1.0) * in.arm;
    // Never wider than it is long, which is what the fold above needs: it maps
    // the upright arm onto the flat one, and a box taller than wide in that
    // octant is the other arm's, nearer than the one being measured. The shape
    // agrees — an arm whose solid length is under its own half-thickness has no
    // arm left, only the square where the two cross.
    let half = min(u.misc5.x * in.arm, solid);
    let corner = vec2<f32>(q.x - solid, q.y - half);
    let sd = length(max(corner, vec2<f32>(0.0))) + min(max(corner.x, corner.y), 0.0);
    // The Clearance fade with no sheet scale on it, which is what `VsOut::soft`
    // carries for a node: markers stand on the home sheet alone
    // (`derive_pluses`), and the home sheet has no scale of its own.
    let soft = standoff_soft(u.misc6.z);
    return ring_shade(sd, soft, in.strength);
}

/// How much of the light standing here this marker holds off, as
/// [`glow_shade_tex`] carries it: [`plus_standoff`], which is already the depth
/// spent over the cross.
///
/// [`glow_shade`]'s twin, down to the exit a depth of 0 buys, which here skips
/// a distance field rather than a walk over every mark.
fn plus_shade(in: PlusGlowVsOut) -> f32 {
    if glow_gap_depth() <= 0.0 {
        return 0.0;
    }
    return plus_standoff(in);
}

@fragment
fn fs_plus_glow(in: PlusGlowVsOut) -> GlowOut {
    let shade = plus_shade(in);
    let light = plus_glow_layer(in);
    // Both answers before it leaves, and for [`fs_glow`]'s reason: a marker
    // whose own pool has shut — or was never lit at all — still holds a NODE's
    // halo off its cross, so the light alone is no reason to go. Exact all the
    // same, a zero being the identity of every one of the three blends.
    if EARLY_OUT && shade <= 0.0 && light.a <= 0.0 {
        discard;
    }
    return GlowOut(light, light, vec4<f32>(shade, 0.0, 0.0, 0.0));
}
