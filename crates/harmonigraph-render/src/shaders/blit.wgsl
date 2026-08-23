// Post-processing for the offscreen scene texture, and the final
// composite into the egui render pass. Deliberately not hot-reloadable —
// this is plumbing plus a fixed bloom chain; scene-look iteration belongs
// in lattice.wgsl.
//
// Every pass draws the same viewport-filling quad (vs_blit):
//   fs_bright     scene -> half res, soft-knee luminance threshold
//   fs_blit       plain copy (half -> quarter downsample)
//   fs_blur_h/v   separable 9-tap Gaussian over the quarter-res texture
//   fs_composite  scene + bloom * strength, premultiplied over the pane
//   fs_bloom_add  bloom * strength alone, over a picture already in the pass
//   fs_glow_over  the lattice's node glow, at the bottom of the scene pass
//
// The bloom chain runs at fractions of the pane's SCREEN size, not the
// (possibly supersampled) scene size, so the halo's screen width does not
// change with the render-scale setting.
//
// The lattice takes all of it; the piano roll (`crate::roll`) and the spiral's
// dots (`crate::glow`) take the threshold, the blurs and `fs_bloom_add` over
// marks they render themselves, so one bloom strength means the same halo in
// every picture.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_samp: sampler;
// Composite-only bindings (declared module-wide; pipelines whose entry
// points don't reference them omit them from their layout).
@group(0) @binding(2) var bloom_tex: texture_2d<f32>;
// The glow target's max-blended half, at the composite-only slot above: no
// entry point reads both, so the two share a binding and the pipeline layout
// for each stays the length its own pass needs. An entry point that ever wanted
// both would fail to compile, loudly, at pipeline creation.
@group(0) @binding(2) var glow_max_tex: texture_2d<f32>;
// The standoff cut into that light, at the composite-only uniform's slot and on
// the same rule: `fs_composite` reads `bu` there and `fs_glow_over` reads this,
// and no entry point wants both — this pass takes the scene uniforms at a group
// of its own (`gu`) precisely because its group 0 is the glow target's. What it
// holds is coverage in x; see lattice.wgsl's `glow_shade_tex`, which is the
// same layer read from the other side.
@group(0) @binding(3) var glow_shade_tex: texture_2d<f32>;
// Head of the scene pass's Uniforms buffer (same binding, shorter view):
// only misc2.w (bloom strength; 0 = off) is read here.
struct BlitUniforms {
    view_proj: mat4x4<f32>,
    cam_right: vec4<f32>,
    cam_up: vec4<f32>,
    misc: vec4<f32>,
    misc2: vec4<f32>,
};
@group(0) @binding(3) var<uniform> bu: BlitUniforms;
// The same head again, at a group of its own, for `fs_glow_over`: its group 0
// is the glow target's two textures, which is a different bind group from the
// composite's, so the one buffer is reached through two slots rather than the
// two passes being made to share a layout that fits neither.
//
// It reads misc.z, how much two nodes' overlapping light adds up. That this
// pass can see only the HEAD of the scene's uniforms is why that dial lives up
// there rather than among the glow's own rows.
@group(1) @binding(0) var<uniform> gu: BlitUniforms;
// The strength on its own, for a caller with no scene uniforms to take the
// head of — the roll, which draws its notes straight into the egui pass and
// wants only the halo laid over them. Its own binding rather than a second
// view of 3: the two buffers have nothing else in common, and a layout that
// fits both would be a coincidence to maintain.
//
// The strength is x. A uniform block is 16 bytes wide whatever it declares,
// so the vector is what is there rather than what is used.
struct AddUniforms {
    strength: vec4<f32>,
};
@group(0) @binding(4) var<uniform> add: AddUniforms;

struct BlitOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Triangle-strip quad over the whole viewport (egui-wgpu sets the viewport
// to the callback's rect). vertex_index 0..3 -> (0,0) (1,0) (0,1) (1,1).
// The NDC-vs-texture y-flip is applied in every pass, so texture-to-
// texture hops cancel it out and the final composite lands upright.
@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> BlitOut {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var out: BlitOut;
    out.pos = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

// Plain copy; used for the half -> quarter downsample (the linear sampler
// does the filtering).
@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    return textureSample(scene_tex, scene_samp, in.uv);
}

// Luminance soft knee: keep what glows, drop the faint idle grid so the
// whole background doesn't haze. Values are premultiplied, so unlit
// (transparent) texels are already black and fall out naturally.
const BLOOM_THRESHOLD: f32 = 0.35;
const BLOOM_KNEE: f32 = 0.25;

@fragment
fn fs_bright(in: BlitOut) -> @location(0) vec4<f32> {
    let c = textureSample(scene_tex, scene_samp, in.uv);
    let lum = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let keep = smoothstep(BLOOM_THRESHOLD - BLOOM_KNEE, BLOOM_THRESHOLD + BLOOM_KNEE, lum);
    return vec4<f32>(c.rgb * keep, 0.0);
}

// Separable 9-tap Gaussian (sigma ~1.75 in taps; the quarter-res target
// stretches that to a wide, cheap halo on screen).
const BLUR_W0: f32 = 0.227027;
const BLUR_W: vec4<f32> = vec4<f32>(0.1945946, 0.1216216, 0.054054, 0.016216);

fn blur(uv: vec2<f32>, dir: vec2<f32>) -> vec4<f32> {
    let texel = dir / vec2<f32>(textureDimensions(scene_tex));
    var acc = textureSample(scene_tex, scene_samp, uv) * BLUR_W0;
    for (var i = 1; i <= 4; i++) {
        let offset = texel * f32(i);
        acc += textureSample(scene_tex, scene_samp, uv + offset) * BLUR_W[i - 1];
        acc += textureSample(scene_tex, scene_samp, uv - offset) * BLUR_W[i - 1];
    }
    return acc;
}

@fragment
fn fs_blur_h(in: BlitOut) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(1.0, 0.0));
}

@fragment
fn fs_blur_v(in: BlitOut) -> @location(0) vec4<f32> {
    return blur(in.uv, vec2<f32>(0.0, 1.0));
}

// Final composite into the egui pass. Bloom is added as pure light with
// zero alpha: under premultiplied blending the halo brightens whatever is
// behind it (pane background included) without occluding anything. With
// strength 0 this reduces exactly to the plain scene blit.
@fragment
fn fs_composite(in: BlitOut) -> @location(0) vec4<f32> {
    let scene = textureSample(scene_tex, scene_samp, in.uv);
    let bloom = textureSample(bloom_tex, scene_samp, in.uv);
    return scene + vec4<f32>(bloom.rgb * bu.misc2.w, 0.0);
}

// The halo alone, over a picture the caller has already drawn into this pass.
// Same light, same zero alpha, and `scene_tex` is the blurred quarter here —
// there is no second texture to composite because the sharp layer is already
// in the target. Drawn AFTER it, so a note's own body is brightened by its
// halo exactly as the lattice's nodes are by theirs.
@fragment
fn fs_bloom_add(in: BlitOut) -> @location(0) vec4<f32> {
    let bloom = textureSample(scene_tex, scene_samp, in.uv);
    return vec4<f32>(bloom.rgb * add.strength.x, 0.0);
}

// The lattice's node glow, laid down at the BOTTOM of the scene pass, before
// any node, marker or label (`crate::LatticeCallback::prepare`). `scene_tex` is
// the glow's own target here: light where the nodes put it, cleared to
// transparent everywhere else, so this is a plain premultiplied-over blit and
// every decision about the shape was taken in lattice.wgsl.
//
// TWO of them, mixed by the Meld bar: the light screened and the light taken at
// its brightest, which is a dial between blends that could not be one blend
// (`create_glow_pipeline`). The node pipelines mix the same pair the same way,
// `node_paint` reading this target back for what its clearing paints.
//
// TWO attachments, because the pass it draws into carries two. The second is
// the picture without the LABELS, which the bloom's bright pass reads — and the
// glow belongs in it: it is light the nodes emit, so it blooms exactly as the
// rest of the node does. Once, from here; nothing else writes it, so there is
// no path by which the same light reaches the threshold twice.
struct GlowOverOut {
    @location(0) color: vec4<f32>,
    @location(1) nodes: vec4<f32>,
};

@fragment
fn fs_glow_over(in: BlitOut) -> GlowOverOut {
    let screened = textureSample(scene_tex, scene_samp, in.uv);
    let brightest = textureSample(glow_max_tex, scene_samp, in.uv);
    // A mix of two premultiplied colours is premultiplied, so what reaches the
    // pass below is the same kind of value either end of the bar hands it.
    let melded = mix(brightest, screened, clamp(gu.misc.z, 0.0, 1.0));
    // The STANDOFF, applied here and nowhere earlier: the light's own target
    // holds the field whole, and every reader of it takes this same multiply.
    // The other reader is lattice.wgsl's `node_paint`, which is what a node's
    // clearing repaints its ground from — so the two must agree exactly, on the
    // same terms the Meld above is mixed identically in both.
    //
    // One factor on premultiplied light takes the colour and the coverage
    // together: a shade of 1 leaves nothing, which is that pixel with no glow
    // in it at all, and a shade of 0 is the light untouched.
    //
    // This is also where a node's dark pool stops blooming. The second
    // attachment is what the bright pass reads, so the light reaching the
    // threshold is the light AFTER the standoff rather than before it, and a
    // pool cannot be filled back in by the halo of the ring standing in it.
    let shade = clamp(textureSample(glow_shade_tex, scene_samp, in.uv).x, 0.0, 1.0);
    let light = melded * (1.0 - shade);
    return GlowOverOut(light, light);
}
