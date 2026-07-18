// Composite the offscreen scene texture into the egui render pass: one
// viewport-filling quad, premultiplied-alpha blended over the pane
// background. Deliberately not hot-reloadable — it is plumbing, not an
// iteration surface; effects belong in lattice.wgsl or future passes.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_samp: sampler;

struct BlitOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Triangle-strip quad over the whole viewport (egui-wgpu sets the viewport
// to the callback's rect). vertex_index 0..3 -> (0,0) (1,0) (0,1) (1,1).
@vertex
fn vs_blit(@builtin(vertex_index) vi: u32) -> BlitOut {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var out: BlitOut;
    out.pos = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    // NDC y points up, texture v points down.
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

@fragment
fn fs_blit(in: BlitOut) -> @location(0) vec4<f32> {
    // The offscreen texture already holds premultiplied alpha (the scene
    // pass clears to transparent black and blends premultiplied), so its
    // texels pass straight through to the premultiplied blend state.
    return textureSample(scene_tex, scene_samp, in.uv);
}
