// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
    metal::float4 background;
    float edge_softness_pixels;
    float padding;
    metal::float2 padding2_;
};
struct BlitOut {
    metal::float4 pos;
    metal::float2 uv;
    char _pad2[8];
};
constant float BLOOM_THRESHOLD = 0.35;
constant float BLOOM_KNEE = 0.25;
constant float BLUR_W0_ = 0.227027;
constant metal::float4 BLUR_W = metal::float4(0.1945946, 0.1216216, 0.054054, 0.016216);

float interleaved_gradient_noise(
    metal::float2 pixel
) {
    float f = (0.06711056 * pixel.x) + (0.00583715 * pixel.y);
    return metal::fract(52.982918 * metal::fract(f));
}

metal::float3 dither_to_unorm8_(
    metal::float3 rgb,
    metal::float2 pixel_1
) {
    float _e2 = interleaved_gradient_noise(pixel_1);
    float noise = (_e2 - 0.5) * 0.85;
    return rgb + metal::float3(noise / 255.0);
}

metal::float4 bloom_sample(
    metal::texture2d<float, metal::access::sample> tex,
    metal::float2 uv,
    metal::sampler scene_samp
) {
    metal::float2 size = static_cast<metal::float2>(metal::uint2(tex.get_width(), tex.get_height()));
    metal::float2 p = (uv * size) - metal::float2(0.5);
    metal::float2 i = metal::floor(p);
    metal::float2 f_1 = p - i;
    metal::float2 w0_ = (((metal::float2(1.0) - f_1) * (metal::float2(1.0) - f_1)) * (metal::float2(1.0) - f_1)) / metal::float2(6.0);
    metal::float2 w1_ = ((metal::float2(4.0) - ((6.0 * f_1) * f_1)) + (((3.0 * f_1) * f_1) * f_1)) / metal::float2(6.0);
    metal::float2 w3_ = ((f_1 * f_1) * f_1) / metal::float2(6.0);
    metal::float2 w2_ = ((metal::float2(1.0) - w0_) - w1_) - w3_;
    metal::float2 g0_ = w0_ + w1_;
    metal::float2 g1_ = w2_ + w3_;
    metal::float2 lo = ((i + (w1_ / g0_)) - metal::float2(0.5)) / size;
    metal::float2 hi = ((i + metal::float2(1.5)) + (w3_ / g1_)) / size;
    metal::float4 _e68 = tex.sample(scene_samp, metal::float2(lo.x, lo.y));
    metal::float4 _e75 = tex.sample(scene_samp, metal::float2(hi.x, lo.y));
    metal::float4 _e85 = tex.sample(scene_samp, metal::float2(lo.x, hi.y));
    metal::float4 _e92 = tex.sample(scene_samp, metal::float2(hi.x, hi.y));
    return (g0_.y * ((g0_.x * _e68) + (g1_.x * _e75))) + (g1_.y * ((g0_.x * _e85) + (g1_.x * _e92)));
}

struct fs_compositeInput {
    metal::float2 uv [[user(loc0), center_perspective]];
};
struct fs_compositeOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_compositeOutput fs_composite(
  fs_compositeInput varyings [[stage_in]]
, metal::float4 pos [[position]]
, metal::texture2d<float, metal::access::sample> scene_tex [[texture(0)]]
, metal::sampler scene_samp [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> bloom_tex [[texture(1)]]
, constant CompositeParams& bu [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> ink_tex [[texture(2)]]
, metal::texture2d<float, metal::access::sample> local_shadow_tex [[texture(3)]]
) {
    const BlitOut in = { pos, varyings.uv };
    metal::float4 scene = scene_tex.sample(scene_samp, in.uv);
    metal::float4 _e7 = bloom_sample(bloom_tex, in.uv, scene_samp);
    metal::float4 ink = ink_tex.sample(scene_samp, in.uv);
    metal::float4 _e15 = local_shadow_tex.sample(scene_samp, in.uv);
    float t = metal::clamp(_e15.x, 0.0, 1.0);
    float _e26 = bu.bloom_strength;
    metal::float3 light = (scene.xyz + ink.xyz) + ((_e7.xyz * _e26) * t);
    metal::float4 _e32 = bu.background;
    metal::float3 ground = _e32.xyz * (1.0 - scene.w);
    metal::float3 capped = metal::max(metal::min(light + ground, metal::float3(t)) - ground, metal::float3(0.0));
    metal::float3 rgb_1 = (t >= 1.0) ? light : capped;
    bool has_light = metal::any(rgb_1 > metal::float3(0.0));
    metal::float3 _e54 = dither_to_unorm8_(rgb_1, in.pos.xy);
    metal::float3 dithered = has_light ? _e54 : rgb_1;
    return fs_compositeOutput { metal::float4(dithered, scene.w) };
}
