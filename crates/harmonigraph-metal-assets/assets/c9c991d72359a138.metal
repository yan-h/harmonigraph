// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct StarSlice {
    metal::float2 offset;
    float cell;
    float radius;
    float solid;
    float ramp;
    float bend;
    int base;
    metal::int2 origin;
    metal::int2 grid;
    float width;
    float inverse_floor;
    uint gather;
    float twinkle;
};
struct type_10 {
    StarSlice inner[5];
};
struct StarUniforms {
    metal::float2 size;
    metal::float2 star_image_size;
    float star_randomness;
    float star_life;
    float star_size_variation;
    float pad;
    type_10 star_slices;
};
struct Settings {
    StarUniforms stars;
    float depth;
    float _pad0_;
    float _pad1_;
    float _pad2_;
};
struct StarDraw {
    metal::float2 centre;
    metal::float2 own;
    metal::float2 other;
    float blend;
    float fade;
    float stagger;
    uint life;
    bool held;
    char _pad8[7];
};
struct TileVertex {
    metal::float4 position;
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;
constant int STAR_ATLAS_WIDTH = 2048;
constant uint STAR_ATLAS_SHIFT = 11u;
constant uint STAR_SLICES = 5u;
constant float STAR_PANE = 540.0;
constant int STAR_HASH_PERIOD = 65536;
constant uint STAR_LIFE_PERIOD = 4096u;
constant float STAR_FADE = 0.2;
constant float STAR_LIFT = 0.18;

metal::float4 star_hash(
    metal::int2 cell,
    uint salt
) {
    uint n = {};
    n = (as_type<uint>(cell.x) * 2654435769u) ^ (as_type<uint>(cell.y) * 2246822507u);
    uint _e12 = n;
    n = _e12 ^ (salt * 668265261u);
    uint _e16 = n;
    uint _e17 = n;
    n = (_e16 ^ (_e17 >> 16u)) * 2146121005u;
    uint _e23 = n;
    uint _e24 = n;
    n = (_e23 ^ (_e24 >> 15u)) * 2221713035u;
    uint _e30 = n;
    uint _e31 = n;
    n = _e30 ^ (_e31 >> 16u);
    uint _e35 = n;
    uint _e36 = n;
    uint _e39 = n;
    uint _e42 = n;
    metal::uint4 bytes = metal::uint4(_e35, _e36 >> 8u, _e39 >> 16u, _e42 >> 24u) & metal::uint4(255u);
    return (static_cast<metal::float4>(bytes) + metal::float4(0.5)) / metal::float4(256.0);
}

float star_randomness(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_randomness;
    return _e3;
}

float star_rank(
    float draw,
    constant Settings& settings
) {
    float _e1 = star_randomness(settings);
    return metal::pow(draw, 1.0 + (6.0 * _e1)) * (2.0 + (6.0 * _e1));
}

float star_life(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_life;
    return _e3;
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

StarDraw star_draw(
    StarSlice s,
    metal::int2 hashed,
    uint salt_1,
    constant Settings& settings
) {
    StarDraw d = {};
    bool local_1 = {};
    bool local_2 = {};
    metal::float4 _e7 = star_hash(hashed, salt_1 + 2u);
    d.stagger = _e7.x;
    float _e9 = star_life(settings);
    float _e11 = d.stagger;
    float age = _e9 + _e11;
    uint life = naga_f2u32(metal::floor(age)) & 4095u;
    d.life = life;
    uint key = salt_1 + ((life + 1u) << 16u);
    float through = metal::fract(age);
    bool held = s.twinkle < 1.0;
    d.held = held;
    metal::float4 _e29 = star_hash(hashed, held ? salt_1 : key);
    d.centre = metal::float2(0.5) + (s.width * (_e29.xy - metal::float2(0.5)));
    metal::float4 _e43 = star_hash(hashed, key + 1u);
    d.own = _e43.xy;
    metal::float2 _e47 = d.own;
    d.other = _e47;
    d.blend = 0.0;
    float dip = metal::smoothstep(0.0, STAR_FADE, through) * metal::smoothstep(0.0, STAR_FADE, 1.0 - through);
    d.fade = 1.0 - (s.twinkle * (1.0 - dip));
    bool early = through < STAR_FADE;
    if (held) {
        if (!(early)) {
            local_2 = through > 0.8;
        } else {
            local_2 = true;
        }
        bool _e76 = local_2;
        local_1 = _e76;
    } else {
        local_1 = false;
    }
    bool _e78 = local_1;
    if (_e78) {
        uint other = (early ? (life - 1u) : (life + 1u)) & 4095u;
        metal::float4 _e94 = star_hash(hashed, (salt_1 + ((other + 1u) << 16u)) + 1u);
        d.other = _e94.xy;
        float later = metal::smoothstep(-0.2, STAR_FADE, early ? through : (through - 1.0));
        d.blend = early ? (1.0 - later) : later;
    }
    StarDraw _e106 = d;
    return _e106;
}

float star_half_width(
    uint gather
) {
    return (gather == 0u) ? 0.5 : (0.5 * static_cast<float>(gather));
}

float star_reach(
    StarSlice s_1,
    metal::float2 centre
) {
    metal::float2 stray = metal::abs(centre - metal::float2(0.5));
    float _e7 = star_half_width(s_1.gather);
    return (_e7 - metal::max(stray.x, stray.y)) * s_1.cell;
}

metal::float2 star_size(
    constant Settings& settings
) {
    metal::float2 _e3 = settings.stars.size;
    return _e3;
}

metal::float4 star_source(
    metal::float2 pt,
    float rank,
    int index,
    constant Settings& settings,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler light_sampler
) {
    metal::float2 _e8 = settings.stars.size;
    metal::float4 light = source.sample(light_sampler, pt / _e8, metal::level(0.0));
    if (light.w <= 0.0) {
        return metal::float4(0.0);
    }
    float randomness = settings.stars.star_randomness;
    float spread = (1.0 - randomness) + (randomness * (0.35 + (0.65 * rank)));
    float lift = (0.09 * rank) * metal::smoothstep(0.0, 0.15, light.w);
    float level = metal::clamp((light.w * spread) + lift, 0.0, 1.0);
    return metal::float4((light.xyz / metal::float3(light.w)) * level, 1.0);
}

float star_size_variation(
    constant Settings& settings
) {
    float _e3 = settings.stars.star_size_variation;
    return _e3;
}

metal::uint3 naga_f2u32(metal::float3 value) {
    return static_cast<metal::uint3>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::uint4 star_bake(
    StarSlice s_2,
    metal::int2 cell_1,
    uint salt_2,
    int index_1,
    constant Settings& settings,
    metal::texture2d<float, metal::access::sample> source,
    metal::sampler light_sampler
) {
    metal::float3 colour = {};
    float radius = {};
    metal::uint3 tens = {};
    metal::int2 hashed_1 = cell_1 & metal::int2(65535);
    StarDraw _e7 = star_draw(s_2, hashed_1, salt_2, settings);
    metal::float2 _e15 = star_size(settings);
    metal::float2 _e20 = star_size(settings);
    metal::float2 at = ((((static_cast<metal::float2>(cell_1) + _e7.centre) + s_2.offset) * s_2.cell) * (_e15.y / STAR_PANE)) + (_e20 * 0.5);
    float _e26 = star_rank(_e7.own.x, settings);
    metal::float4 _e27 = star_source(at, _e26, index_1, settings, source, light_sampler);
    if (_e27.w <= 0.0) {
        return metal::uint4(0u);
    }
    colour = _e27.xyz;
    float _e36 = star_size_variation(settings);
    radius = s_2.radius * metal::exp((-2.4 * _e36) * _e7.own.y);
    if (_e7.blend > 0.0) {
        metal::float3 _e48 = colour;
        float _e51 = star_rank(_e7.other.x, settings);
        metal::float4 _e52 = star_source(at, _e51, index_1, settings, source, light_sampler);
        colour = metal::mix(_e48, _e52.xyz, _e7.blend);
        float _e56 = radius;
        float _e58 = star_size_variation(settings);
        radius = metal::mix(_e56, s_2.radius * metal::exp((-2.4 * _e58) * _e7.other.y), _e7.blend);
    }
    float _e68 = radius;
    float _e74 = star_reach(s_2, _e7.centre);
    float drawn = metal::min(metal::max(_e68, 1.0 / s_2.inverse_floor), _e74);
    float _e77 = radius;
    float _e79 = radius;
    float gain = _e7.fade * metal::min(1.0, (_e77 / drawn) * (_e79 / drawn));
    metal::float3 _e85 = colour;
    tens = naga_f2u32(metal::rint(metal::clamp(_e85, metal::float3(0.0), metal::float3(1.0)) * 1023.0));
    uint _e103 = tens.x;
    uint _e107 = tens.y;
    uint _e112 = tens.z;
    return metal::uint4(as_type<uint>(_e7.centre.x), as_type<uint>(_e7.centre.y), ((_e103 << 20u) | (_e107 << 10u)) | _e112, as_type<uint>(half2(metal::float2(1.0 / drawn, gain))));
}

StarSlice star_slice(
    uint k_1,
    constant Settings& settings
) {
    StarSlice _e5 = settings.stars.star_slices.inner[metal::min(unsigned(k_1), 4u)];
    return _e5;
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

int naga_div(int lhs, int rhs) {
    return lhs / metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
}


struct fs_star_bakeInput {
};
struct fs_star_bakeOutput {
    metal::uint4 member [[color(0)]];
};
fragment fs_star_bakeOutput fs_star_bake(
  metal::float4 position [[position]]
, constant Settings& settings [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> source [[texture(0)]]
, metal::sampler light_sampler [[sampler(0)]]
) {
    const TileVertex in = { position };
    uint k = 0u;
    bool local = {};
    metal::int2 texel = naga_f2i32(metal::floor(in.position.xy));
    int index_2 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(texel.y) * as_type<uint>(STAR_ATLAS_WIDTH))) + as_type<uint>(texel.x));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e46 = k;
            k = _e46 + 1u;
        }
        loop_init = false;
        uint _e12 = k;
        if (_e12 < STAR_SLICES) {
        } else {
            break;
        }
        {
            uint _e15 = k;
            StarSlice _e16 = star_slice(_e15, settings);
            int at_1 = as_type<int>(as_type<uint>(index_2) - as_type<uint>(_e16.base));
            if (at_1 >= 0) {
                local = at_1 < as_type<int>(as_type<uint>(_e16.grid.x) * as_type<uint>(_e16.grid.y));
            } else {
                local = false;
            }
            bool _e30 = local;
            if (_e30) {
                metal::int2 local_3 = metal::int2(naga_mod(at_1, _e16.grid.x), naga_div(at_1, _e16.grid.x));
                uint _e42 = k;
                metal::uint4 _e45 = star_bake(_e16, as_type<metal::int2>(as_type<metal::uint2>(_e16.origin) + as_type<metal::uint2>(local_3)), 1000u + (3u * _e42), index_2, settings, source, light_sampler);
                return fs_star_bakeOutput { _e45 };
            }
        }
    }
    return fs_star_bakeOutput { metal::uint4(0u) };
}
