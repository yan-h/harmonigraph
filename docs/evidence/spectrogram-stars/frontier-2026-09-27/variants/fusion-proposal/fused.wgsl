// Append to the complete production shader only for the scratch fused cases.
// The MRT pass binds previous memory; it writes current memory and packed stars.
struct ResearchFusedStar {
    @location(0) star: vec4<u32>,
    @location(1) memory: vec4<f32>,
}

fn research_fused_star(k: u32, cell: vec2<i32>) -> ResearchFusedStar {
    let s = cloud.star_slices[k];
    let salt = 1000u + 3u * k;
    let hashed = cell & vec2<i32>(STAR_HASH_PERIOD - 1);
    let stagger = star_hash(hashed, salt + 2u).x;
    let age = cloud.star_life + stagger;
    let life = u32(floor(age)) & (STAR_LIFE_PERIOD - 1u);
    let key = salt + ((life + 1u) << 16u);
    let a = star_hash(hashed, key);
    let through = fract(age);
    let centre = 0.5 + cloud.star_geometry.x * (a.xy - 0.5);
    let at = (vec2<f32>(cell) + centre + s.offset) * s.cell
        * (cloud.size.y / STAR_PANE) + cloud.size * 0.5;
    let level = star_level_at(at);
    let c = star_hash(hashed, key + 1u);
    let randomness = cloud.star_randomness;
    let colour = star_paint(level, pow(c.x, 1.0 + 6.0 * randomness) * (2.0 + 6.0 * randomness));
    let current = vec4<f32>(linear_from_gamma_rgb(colour), level);
    var held = current;
    if cloud.memory_valid != 0u {
        let old_life = u32(floor(cloud.previous_life + stagger)) & (STAR_LIFE_PERIOD - 1u);
        let previous = cloud.previous_slices[k];
        let local = ((cell - previous.origin + STAR_HASH_PERIOD / 2) & vec2<i32>(STAR_HASH_PERIOD - 1)) - STAR_HASH_PERIOD / 2;
        if old_life == life && all(local >= vec2<i32>(0)) && all(local < previous.grid) {
            let old_index = previous.base + local.y * previous.grid.x + local.x;
            held = remembered(current, textureLoad(color_memory, atlas_texel(old_index), 0));
        }
    }
    // A silent star still updates retained memory, exactly as the two-pass path.
    if held.a <= 0.0 {
        return ResearchFusedStar(vec4<u32>(0u), held);
    }
    let size = exp((0.3 + 0.9 * randomness) * (c.y - 0.5) * 2.0);
    let sigma = min(s.sigma * size, s.cap) * s.defocus;
    let fade = smoothstep(0.0, STAR_FADE, through) * smoothstep(0.0, STAR_FADE, 1.0 - through);
    let tens = vec3<u32>(round(gamma_from_linear_rgb(held.rgb) * 1023.0));
    let star = vec4<u32>(
        bitcast<u32>(centre.x),
        bitcast<u32>(centre.y),
        (tens.r << 20u) | (tens.g << 10u) | tens.b,
        pack2x16float(vec2<f32>(1.0 / sigma, fade)),
    );
    return ResearchFusedStar(star, held);
}

@fragment
fn fs_research_fused_star(in: TileVertex) -> ResearchFusedStar {
    let texel = vec2<i32>(floor(in.position.xy));
    let index = texel.y * STAR_ATLAS_WIDTH + texel.x;
    for (var k = 0u; k < STAR_SLICES; k += 1u) {
        let s = cloud.star_slices[k];
        let at = index - s.base;
        if at >= 0 && at < s.grid.x * s.grid.y {
            let local = vec2<i32>(at % s.grid.x, at / s.grid.x);
            return research_fused_star(k, s.origin + local);
        }
    }
    return ResearchFusedStar(vec4<u32>(0u), vec4<f32>(0.0));
}
