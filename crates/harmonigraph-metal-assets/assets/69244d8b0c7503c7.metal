// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float min_midi;
    float span;
    float spectrum_min_midi;
    float bins_per_semitone;
    float level0_;
    float level_per_step;
    float level_per_midi;
    uint rows;
    uint bins;
    uint stride;
    uint capacity;
    uint first_slot;
    uint run_slabs;
    uint _pad0_;
    uint _pad1_;
    uint _pad2_;
};
struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
};
struct Cloud {
    metal::float2 origin;
    metal::float2 size;
    metal::float2 step;
    float glow;
    float texture;
    metal::float2 time;
    float ppp;
    float time_scale;
    metal::float2 time_direction;
    metal::float2 pitch_direction;
};
struct type_8 {
    metal::float2 inner[8];
};

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

uint cloud_hash(
    metal::int2 cell
) {
    uint n = {};
    uint x = static_cast<uint>(naga_mod(as_type<int>(as_type<uint>(naga_mod(cell.x, 128)) + as_type<uint>(128)), 128));
    n = (x * 374761393u) + (as_type<uint>(cell.y) * 668265263u);
    uint _e17 = n;
    uint _e18 = n;
    n = (_e17 ^ (_e18 >> 13u)) * 1274126177u;
    uint _e24 = n;
    uint _e25 = n;
    return _e24 ^ (_e25 >> 16u);
}

metal::float2 cloud_gradient(
    metal::int2 cell_1
) {
    type_8 directions = type_8 {{metal::float2(1.0, 0.0), metal::float2(0.7071068, 0.7071068), metal::float2(0.0, 1.0), metal::float2(-0.7071068, 0.7071068), metal::float2(-1.0, 0.0), metal::float2(-0.7071068, -0.7071068), metal::float2(0.0, -1.0), metal::float2(0.7071068, -0.7071068)}};
    uint _e26 = cloud_hash(cell_1);
    return directions.inner[metal::min(unsigned(_e26 & 7u), 7u)];
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float cloud_noise(
    metal::float2 p
) {
    metal::int2 cell_2 = naga_f2i32(metal::floor(p));
    metal::float2 f = metal::fract(p);
    metal::float2 w = ((f * f) * f) * ((f * ((f * 6.0) - metal::float2(15.0))) + metal::float2(10.0));
    metal::float2 _e16 = cloud_gradient(cell_2);
    metal::float2 _e22 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 0))));
    metal::float2 _e34 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(0, 1))));
    metal::float2 _e44 = cloud_gradient(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 1))));
    return 0.5 + metal::mix(metal::mix(metal::dot(_e16, f), metal::dot(_e22, f - metal::float2(1.0, 0.0)), w.x), metal::mix(metal::dot(_e34, f - metal::float2(0.0, 1.0)), metal::dot(_e44, f - metal::float2(1.0, 1.0)), w.x), w.y);
}

metal::float3 gamma_from_linear_rgb(
    metal::float3 linear
) {
    metal::float3 c = metal::max(linear, metal::float3(0.0));
    return metal::select((1.055 * metal::pow(c, metal::float3(0.41666666))) - metal::float3(0.055), c * 12.92, c <= metal::float3(0.0031308));
}

float cloud_edge(
    metal::float2 uv,
    metal::texture2d<float, metal::access::sample> wide_light
) {
    metal::float2 feather = metal::float2(2.0) / static_cast<metal::float2>(metal::uint2(wide_light.get_width(), wide_light.get_height()));
    metal::float2 coverage = metal::smoothstep(metal::float2(0.0), feather, uv) * metal::smoothstep(metal::float2(0.0), feather, metal::float2(1.0) - uv);
    return coverage.x * coverage.y;
}

struct fs_cloud_lightInput {
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct fs_cloud_lightOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_cloud_lightOutput fs_cloud_light(
  fs_cloud_lightInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> close_light [[texture(1)]]
, metal::texture2d<float, metal::access::sample> wide_light [[texture(2)]]
, metal::sampler cloud_sampler [[sampler(0)]]
, constant Cloud& cloud [[buffer(2)]]
) {
    const VertexOut in = { position, varyings.slab, varyings.t };
    metal::float2 close_uv = in.position.xy / static_cast<metal::float2>(metal::uint2(wide_light.get_width(), wide_light.get_height()));
    float _e10 = cloud.time.x;
    float _e14 = cloud.time.x;
    float origin = _e10 - (metal::floor(_e14 / 4096.0) * 4096.0);
    float _e25 = cloud.time.y;
    float _e30 = cloud.time_scale;
    float _e34 = locals.min_midi;
    float _e38 = locals.span;
    metal::float2 p_1 = metal::float2((origin + (in.slab * _e25)) * _e30, (_e34 + (in.t * _e38)) * 0.18);
    float _e48 = cloud_noise(p_1 + metal::float2(3.1, 7.4));
    float _e53 = cloud_noise(p_1 + metal::float2(8.3, 2.7));
    metal::float2 q = metal::float2(_e48, _e53) - metal::float2(0.5);
    float _e67 = cloud_noise(((p_1 * 2.0) + (q * 1.2)) + metal::float2(1.7, 9.2));
    float _e77 = cloud_noise(((p_1 * 2.0) + (q * 1.2)) + metal::float2(6.8, 3.5));
    metal::float2 r = metal::float2(_e67, _e77) - metal::float2(0.5);
    metal::float2 folded = (p_1 + (q * 1.6)) + (r * 0.35);
    float _e88 = cloud_noise(folded);
    float _e95 = cloud_noise((folded * 2.0) + metal::float2(5.2, 1.3));
    float _e104 = cloud_noise((folded * 4.0) + metal::float2(2.8, 6.1));
    float wisps = (0.65 * _e95) + (0.35 * _e104);
    float _e110 = cloud.texture;
    float plain = 1.0 - _e110;
    float amount = 1.0 - (((plain * plain) * plain) * plain);
    float density = 0.12 + (1.2 * metal::smoothstep(0.25, 0.75, (_e88 * 0.7) + (wisps * 0.3)));
    metal::float2 flow = q + (r * 0.15);
    metal::float2 _e135 = cloud.time_direction;
    metal::float2 _e140 = cloud.pitch_direction;
    metal::float2 _e146 = cloud.step;
    metal::float2 displacement = (((_e135 * flow.x) + (_e140 * flow.y)) * _e146) * (12.0 * amount);
    metal::float2 wide_uv = close_uv + displacement;
    metal::float4 _e155 = close_light.sample(cloud_sampler, close_uv, metal::level(0.0));
    float _e157 = cloud_edge(close_uv, wide_light);
    metal::float3 close = _e155.xyz * _e157;
    metal::float4 _e162 = wide_light.sample(cloud_sampler, wide_uv, metal::level(0.0));
    float _e164 = cloud_edge(wide_uv, wide_light);
    metal::float3 wide = _e162.xyz * _e164;
    float modulation = metal::mix(1.0, density, amount);
    metal::float3 _e173 = gamma_from_linear_rgb((close * 0.35) + (wide * 0.75));
    float _e176 = cloud.glow;
    metal::float3 light = (_e173 * _e176) * modulation;
    return fs_cloud_lightOutput { metal::float4(metal::clamp(light, metal::float3(0.0), metal::float3(1.0)), 1.0) };
}
