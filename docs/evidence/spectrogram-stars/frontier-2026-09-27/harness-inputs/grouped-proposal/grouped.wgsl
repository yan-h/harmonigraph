// Test-only groups follow factor identity, not their rounded texture extents.
@group(1) @binding(11) var research_halo_group_0: texture_2d_array<f32>;
@group(1) @binding(12) var research_halo_group_1: texture_2d_array<f32>;
@group(1) @binding(13) var research_halo_group_2: texture_2d_array<f32>;

const RESEARCH_HALO_GROUPS: array<u32, 5> = array<u32, 5>(__GROUPS__);
const RESEARCH_HALO_LAYERS: array<i32, 5> = array<i32, 5>(__LAYERS__);

fn research_grouped_halo(uv: vec2<f32>, k: u32) -> vec4<f32> {
    let layer = RESEARCH_HALO_LAYERS[k];
    // k and its group/layer are uniform across the fragment invocation group.
    switch RESEARCH_HALO_GROUPS[k] {
        case 0u: { return textureSampleLevel(research_halo_group_0, cloud_sampler, uv, layer, 0.0); }
        case 1u: { return textureSampleLevel(research_halo_group_1, cloud_sampler, uv, layer, 0.0); }
        default: { return textureSampleLevel(research_halo_group_2, cloud_sampler, uv, layer, 0.0); }
    }
}
