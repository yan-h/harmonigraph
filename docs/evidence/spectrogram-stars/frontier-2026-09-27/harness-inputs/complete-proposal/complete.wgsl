// Scratch native-complete mode. Reduced or misaligned depths keep production's
// residual halo plus analytic native core. The host substitutes five booleans.
const RESEARCH_COMPLETE_DEPTH: array<bool, 5> = array<bool, 5>(__COMPLETE_DEPTHS__);

fn research_complete_layer(k: u32) -> bool {
    let pixel_size = cloud.size * cloud.ppp;
    let pixel_origin = cloud.origin * cloud.ppp;
    // Target size alone does not establish matching sample centers. Conservative
    // power-of-two scales keep point<->pixel arithmetic exact on normal windows.
    return RESEARCH_COMPLETE_DEPTH[k]
        && (cloud.ppp == 1.0 || cloud.ppp == 2.0 || cloud.ppp == 4.0)
        && all(pixel_size == cloud.star_halo_size)
        && all(pixel_origin == round(pixel_origin));
}

fn research_complete_load(pt: vec2<f32>, k: u32) -> vec4<f32> {
    let texel = vec2<i32>(pt * cloud.ppp);
    __LOAD_RESPONSE__
}

fn research_complete_star(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    let t = textureLoad(star_atlas, atlas_texel(index), 0);
    if t.w == 0u { return vec4<f32>(0.0); }
    let dist = length(f - vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))) * s.cell;
    let outer = STAR_HALO_REACH * s.cell;
    if dist >= outer { return vec4<f32>(0.0); }
    let colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    let shape = unpack2x16float(t.w);
    let d = dist * shape.x;
    var full = exp(-0.5 * d * d);
    if s.fringe > 0.0 { full += s.fringe * exp(-0.4 * d); }
    full = min(full, 1.0) * (1.0 - smoothstep(STAR_HALO_FADE * outer, outer, dist));
    let cover = full * shape.y;
    return vec4<f32>(colour * cover, cover);
}
