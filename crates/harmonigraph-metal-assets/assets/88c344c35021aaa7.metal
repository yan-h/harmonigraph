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
};
struct CameraParams {
    metal::float4x4 view_proj;
    metal::float4 right;
    metal::float4 up;
};
struct NodeParams {
    float radius;
    float band_inner;
    float band_outer;
    float rings_outer;
    float mark_inner;
    float angular_gap;
    float mark_thickness;
    float animation;
    metal::float4 pose;
};
struct MarkerParams {
    float half_width;
    float taper_start;
    float world_unit;
    float padding;
};
struct type_8 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_8 bounds;
};
struct SpectralParams {
    float inner;
    float outer;
    float range_cents;
    float folded;
};
struct GlowParams {
    float reach;
    float strength;
    float padding;
    float curve;
    float wash;
    float row_capacity;
    float lit;
    float accumulation;
};
struct NebulaParams {
    float depth;
    float scale;
    metal::float2 drift;
    metal::float2 target_size;
    uint material;
    float padding;
};
struct ShadowParams {
    float width;
    float reach_sigmas;
    float depth;
    float occlusion;
};
struct ShadowTargetParams {
    metal::float2 pane_points;
    metal::float2 atlas_texels;
};
struct MarkerCellParams {
    metal::float4 rect;
    metal::float4 cell;
    float points_to_texels;
    float aa_scale;
    float arm_points;
    float spread_points;
};
struct type_9 {
    metal::float4 inner[64];
};
struct type_11 {
    metal::uint4 inner[240];
};
struct type_12 {
    metal::float4 inner[16];
};
struct Uniforms {
    CompositeParams composite;
    CameraParams camera;
    NodeParams node;
    MarkerParams marker;
    OctaveParams octave;
    SpectralParams spectral;
    GlowParams glow;
    NebulaParams nebula;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    metal::float4 lut_spacing;
    type_9 pitch_lut;
    type_9 spectral_lut;
    type_11 spectrum_color;
    type_12 ink_kernel;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float GLOW_BASE = 0.8;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant float THICKNESS_STEPS = 64.0;
constant float GAUSSIAN_SPREAD_KIND = 3.0;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant float BEND_ROUNDING = 0.4;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float AA_SOFTNESS_PX = 2.0;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

metal::float3 glow_linear(
    metal::float3 rgb
) {
    return metal::select(metal::pow((rgb + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4)), rgb / metal::float3(12.92), rgb <= metal::float3(0.04045));
}

metal::float3 glow_gamma(
    metal::float3 rgb_1
) {
    return metal::select((1.055 * metal::pow(metal::max(rgb_1, metal::float3(0.0031308)), metal::float3(0.41666666))) - metal::float3(0.055), rgb_1 * 12.92, rgb_1 <= metal::float3(0.0031308));
}

float nebula_hash(
    metal::int2 cell
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ as_type<uint>(cell.y);
    uint _e9 = n;
    uint _e10 = n;
    n = (_e9 ^ (_e10 >> 16u)) * 2146121005u;
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 15u)) * 2221713035u;
    uint _e23 = n;
    uint _e24 = n;
    n = _e23 ^ (_e24 >> 16u);
    uint _e28 = n;
    return static_cast<float>(_e28 >> 8u) / 16777216.0;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float nebula_noise(
    metal::float2 p
) {
    metal::int2 cell_1 = naga_f2i32(metal::floor(p));
    metal::float2 f = metal::fract(p);
    metal::float2 w = (f * f) * (metal::float2(3.0) - (2.0 * f));
    float _e11 = nebula_hash(cell_1);
    float _e16 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 0))));
    float _e23 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(0, 1))));
    float _e28 = nebula_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_1) + as_type<metal::uint2>(metal::int2(1, 1))));
    return metal::mix(metal::mix(_e11, _e16, w.x), metal::mix(_e23, _e28, w.x), w.y);
}

float lattice_material(
    metal::float2 p_1,
    metal::float2 drift,
    float alpha,
    float alpha_width,
    constant Uniforms& u,
    metal::texture2d<float, metal::access::sample> glow_sum
) {
    float _e5 = nebula_noise(p_1 + drift);
    float _e11 = nebula_noise((p_1 + metal::float2(8.3, 2.7)) - drift);
    metal::float2 warp = metal::float2(_e5, _e11);
    metal::float2 q = p_1 + ((warp - metal::float2(0.5)) * 1.2);
    float _e22 = u.nebula.scale;
    float footprint = 5.0 / (_e22 * static_cast<float>(metal::uint2(glow_sum.get_width(), glow_sum.get_height()).y));
    uint _e33 = u.nebula.material;
    if (_e33 == 1u) {
        float _e40 = u.nebula.scale;
        float phase = ((metal::sqrt(alpha) * (48.0 / _e40)) + ((warp.x - 0.5) * 2.0)) + (drift.x * 2.0);
        float terrace = 0.5 + (0.5 * metal::sin(phase));
        float _e64 = u.nebula.scale;
        float phase_width = (alpha_width * 24.0) / (_e64 * metal::sqrt(metal::max(alpha, 0.001)));
        return 0.15 + (0.85 * metal::mix(terrace * terrace, 0.375, metal::smoothstep(0.7, 2.5, phase_width)));
    }
    float a = metal::sin((metal::length(q - metal::float2(1.7, -0.8)) * 19.0) + (drift.x * 5.0));
    float b = metal::sin((metal::length(q - metal::float2(-2.1, 1.3)) * 23.0) - (drift.y * 5.0));
    float fringes = metal::pow(0.5 + (0.25 * (a + b)), 3.0);
    return 0.1 + (0.9 * metal::mix(fringes, 0.21875, metal::smoothstep(0.7, 2.5, footprint * 55.0)));
}

metal::float4 nebula_light(
    metal::float4 light,
    metal::float2 pixel,
    constant Uniforms& u
) {
    bool local = {};
    float _e5 = u.nebula.depth;
    if (!((_e5 <= 0.0))) {
        local = light.w <= 0.0;
    } else {
        local = true;
    }
    bool _e15 = local;
    if (_e15) {
        return light;
    }
    metal::float2 _e19 = u.nebula.target_size;
    float _e27 = u.nebula.target_size.y;
    float _e33 = u.nebula.scale;
    metal::float2 p_2 = ((pixel - (_e19 * 0.5)) / metal::float2(_e27)) * (5.0 / _e33);
    metal::float2 drift_1 = u.nebula.drift;
    float _e42 = nebula_noise(p_2 + drift_1);
    float _e48 = nebula_noise((p_2 + metal::float2(8.3, 2.7)) - drift_1);
    metal::float2 warp_1 = metal::float2(_e42, _e48);
    float _e54 = nebula_noise((p_2 + (warp_1 * 1.2)) + drift_1);
    float _e62 = nebula_noise(((p_2 * 2.3) - drift_1) + metal::float2(3.1, 7.4));
    float density = 0.08 + (0.92 * metal::smoothstep(0.25, 0.7, (_e54 * 0.75) + (_e62 * 0.25)));
    float _e78 = u.nebula.depth;
    return light * metal::mix(1.0, density, _e78);
}

metal::float4 material_light(
    metal::float4 light_1,
    metal::float2 pixel_1,
    metal::float2 contour_1,
    constant Uniforms& u,
    metal::texture2d<float, metal::access::sample> glow_sum
) {
    bool local_1 = {};
    uint _e6 = u.nebula.material;
    if (_e6 == 0u) {
        metal::float4 _e9 = nebula_light(light_1, pixel_1, u);
        return _e9;
    }
    float _e13 = u.nebula.depth;
    if (!((_e13 <= 0.0))) {
        local_1 = light_1.w <= 0.0;
    } else {
        local_1 = true;
    }
    bool _e23 = local_1;
    if (_e23) {
        return light_1;
    }
    metal::float2 _e27 = u.nebula.target_size;
    float _e35 = u.nebula.target_size.y;
    float _e41 = u.nebula.scale;
    metal::float2 p_3 = ((pixel_1 - (_e27 * 0.5)) / metal::float2(_e35)) * (5.0 / _e41);
    metal::float2 _e48 = u.nebula.drift;
    float _e51 = lattice_material(p_3, _e48, contour_1.x, contour_1.y, u, glow_sum);
    float _e55 = u.nebula.depth;
    return light_1 * metal::mix(1.0, _e51, _e55);
}

struct fs_glow_resolveInput {
};
struct fs_glow_resolveOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glow_resolveOutput fs_glow_resolve(
  metal::float4 pos [[position]]
, constant Uniforms& u [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> glow_sum [[texture(0)]]
, metal::texture2d<float, metal::access::sample> glow_screen [[texture(1)]]
, metal::texture2d<float, metal::access::sample> glow_accumulated [[texture(2)]]
) {
    metal::float2 contour = metal::float2(0.0);
    metal::float3 linear = {};
    float _e4 = u.glow.lit;
    if (_e4 <= 0.0) {
        return fs_glow_resolveOutput { metal::float4(0.0) };
    }
    metal::int2 pixel_2 = naga_f2i32(pos.xy);
    metal::float2 _e15 = u.nebula.target_size;
    metal::float2 scene_pixel = (pos.xy * _e15) / static_cast<metal::float2>(metal::uint2(glow_sum.get_width(), glow_sum.get_height()));
    uint clamped_lod_e23 = metal::min(uint(0), glow_sum.get_num_mip_levels() - 1);
    metal::float4 sum = glow_sum.read(metal::min(metal::uint2(pixel_2), metal::uint2(glow_sum.get_width(clamped_lod_e23), glow_sum.get_height(clamped_lod_e23)) - 1), clamped_lod_e23);
    uint clamped_lod_e26 = metal::min(uint(0), glow_screen.get_num_mip_levels() - 1);
    metal::float4 _e26 = glow_screen.read(metal::min(metal::uint2(pixel_2), metal::uint2(glow_screen.get_width(clamped_lod_e26), glow_screen.get_height(clamped_lod_e26)) - 1), clamped_lod_e26);
    metal::float2 bounded = _e26.xy;
    uint clamped_lod_e30 = metal::min(uint(0), glow_accumulated.get_num_mip_levels() - 1);
    metal::float4 accumulated = glow_accumulated.read(metal::min(metal::uint2(pixel_2), metal::uint2(glow_accumulated.get_width(clamped_lod_e30), glow_accumulated.get_height(clamped_lod_e30)) - 1), clamped_lod_e30);
    uint _e37 = u.nebula.material;
    if (_e37 == 1u) {
        float _e42 = metal::fwidth(accumulated.w);
        contour = metal::float2(accumulated.w, _e42);
    }
    if (sum.w <= 1.0) {
        metal::float2 _e47 = contour;
        metal::float4 _e48 = material_light(accumulated, scene_pixel, _e47, u, glow_sum);
        return fs_glow_resolveOutput { _e48 };
    }
    float _e52 = u.glow.accumulation;
    float accumulation = metal::clamp(_e52, 0.0, 1.0);
    if (accumulation >= 1.0) {
        metal::float2 _e58 = contour;
        metal::float4 _e59 = material_light(accumulated, scene_pixel, _e58, u, glow_sum);
        return fs_glow_resolveOutput { _e59 };
    }
    float _e64 = u.glow.strength;
    float peak = metal::clamp(GLOW_BASE * _e64, 0.0, 1.0);
    metal::float3 _e70 = glow_linear(metal::float3(peak));
    float peak_luminance = _e70.x;
    metal::float3 rgb_2 = sum.xyz;
    float screen = bounded.x;
    float coverage = bounded.y;
    float total = metal::dot(rgb_2, GLOW_LUMINANCE);
    float light_2 = metal::min(screen, 1.0) * peak_luminance;
    if (total <= 0.0) {
        metal::float2 _e88 = contour;
        metal::float4 _e89 = material_light(metal::mix(metal::float4(0.0, 0.0, 0.0, peak * coverage), accumulated, accumulation), scene_pixel, _e88, u, glow_sum);
        return fs_glow_resolveOutput { _e89 };
    }
    linear = rgb_2 * (light_2 / total);
    float _e94 = linear.x;
    float _e96 = linear.y;
    float _e99 = linear.z;
    float largest = metal::max(metal::max(_e94, _e96), _e99);
    if (largest > peak_luminance) {
        float chroma = metal::clamp((peak_luminance - light_2) / (largest - light_2), 0.0, 1.0);
        metal::float3 _e109 = linear;
        linear = metal::mix(metal::float3(light_2), _e109, chroma);
    }
    metal::float3 _e111 = linear;
    metal::float3 _e115 = glow_gamma(metal::max(_e111, metal::float3(0.0)));
    metal::float3 colour = metal::min(_e115, metal::float3(peak));
    float alpha_1 = metal::max(peak * coverage, metal::max(metal::max(colour.x, colour.y), colour.z));
    metal::float2 _e128 = contour;
    metal::float4 _e129 = material_light(metal::mix(metal::float4(colour, metal::min(alpha_1, peak)), accumulated, accumulation), scene_pixel, _e128, u, glow_sum);
    return fs_glow_resolveOutput { _e129 };
}
