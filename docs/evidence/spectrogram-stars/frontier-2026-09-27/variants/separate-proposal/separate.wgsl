// Test-only bindings for five independently sized, filterable halo textures.
@group(1) @binding(11) var research_halo_0: texture_2d<f32>;
@group(1) @binding(12) var research_halo_1: texture_2d<f32>;
@group(1) @binding(13) var research_halo_2: texture_2d<f32>;
@group(1) @binding(14) var research_halo_3: texture_2d<f32>;
@group(1) @binding(15) var research_halo_4: texture_2d<f32>;

fn research_separate_halo(uv: vec2<f32>, k: u32) -> vec4<f32> {
    // k is the uniform depth-loop index: no spatially divergent selection.
    // Each texture applies its own true ClampToEdge in normalized coordinates.
    switch k {
        case 0u: { return textureSampleLevel(research_halo_0, cloud_sampler, uv, 0.0); }
        case 1u: { return textureSampleLevel(research_halo_1, cloud_sampler, uv, 0.0); }
        case 2u: { return textureSampleLevel(research_halo_2, cloud_sampler, uv, 0.0); }
        case 3u: { return textureSampleLevel(research_halo_3, cloud_sampler, uv, 0.0); }
        default: { return textureSampleLevel(research_halo_4, cloud_sampler, uv, 0.0); }
    }
}
