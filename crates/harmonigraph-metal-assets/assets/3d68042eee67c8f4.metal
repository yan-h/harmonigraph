// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size2;
    uint size3;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_5[1];
typedef uint type_7[1];
struct Locals {
    metal::float2 screen_points;
    metal::float2 atlas_size;
    metal::float2 mark_atlas_size;
    metal::float2 filter_axis;
    float pixels_per_point;
    float shadow_depth;
    metal::float2 shadow_atlas_size;
    float node_occlusion;
    float _pad0_;
    metal::float2 _pad1_;
};
struct VertexOut {
    metal::float4 position;
    metal::float2 texel;
    metal::float2 uv_min;
    metal::float2 uv_max;
    char _pad4[8];
    metal::float4 fill;
    uint sheet;
    char _pad6[4];
    metal::float2 sheet_size;
    metal::float2 points;
    float who;
    char _pad9[4];
};
constant float DISTANCE_KIND = 1.0;
constant float DISTANCE_COVERAGE_KIND = 2.0;
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_STOP = 1.0;
constant float SHADOW_FALLOFF_MIN = -6.0;
constant float SHADOW_FALLOFF_MAX = 6.0;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float SDF_NEAR_PAD = 32.0;
constant float SDF_COARSE_PAD = 48.0;
constant float SDF_NEAR_BLEND = 8.0;

bool cell_packed(
    metal::float4 cell
) {
    bool local = {};
    if (cell.z > 0.0) {
        local = cell.w > 0.0;
    } else {
        local = false;
    }
    bool _e10 = local;
    return _e10;
}

float standoff_coverage(
    float d,
    float w,
    float falloff
) {
    float u = metal::clamp((metal::max(d, 0.0) / metal::max(w, 0.000001)) / SHADOW_STOP, 0.0, 1.0);
    float remaining = 1.0 - u;
    float shape = -(metal::clamp(falloff, SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX));
    if (metal::abs(shape) < 0.05) {
        return remaining * ((1.0 - ((shape * u) * 0.5)) + ((((shape * shape) * u) * ((2.0 * u) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size2 - 0 - 64) / 64)) {
        return 0.0;
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].cell;
    bool _e10 = cell_packed(cell_1);
    if (!(_e10)) {
        return 0.0;
    }
    metal::float2 atlas_1 = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].map;
    metal::float2 texel_4 = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel_4 / atlas_1, metal::level(0.0));
    float held = _e39.x;
    float _e45 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.y;
    if (_e45 == DISTANCE_COVERAGE_KIND) {
        return metal::clamp(held, 0.0, 1.0);
    }
    float _e55 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.y;
    if (_e55 >= 0.5) {
        float _e62 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.z;
        float _e69 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size2 - 0 - 64) / 64)].shade.w;
        float _e70 = standoff_coverage(held, 2.0 * _e62, _e69);
        return metal::clamp(_e70, 0.0, 1.0);
    }
    return metal::min(GAUSSIAN_GAIN * metal::clamp(held, 0.0, 1.0), 1.0);
}

float shadow_transmittance(
    float full,
    float depth,
    float level
) {
    float keep = metal::max(1.0 - metal::clamp(depth, 0.0, 1.0), SHADOW_KEEP_FLOOR);
    float through = metal::pow(keep, metal::clamp(full, 0.0, 1.0));
    return 1.0 - (metal::clamp(level, 0.0, 1.0) * (1.0 - through));
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

float node_visibility(
    float who_1,
    metal::float2 points_1,
    float occlusion,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_5 const& shadow_casters,
    device type_7 const& node_occluders,
    constant _mslBufferSizes& _buffer_sizes
) {
    bool local_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    float visibility = 1.0;
    uint k = {};
    bool local_4 = {};
    bool local_5 = {};
    float hidden = {};
    float strength = metal::clamp(occlusion, 0.0, 1.0);
    if (strength == 0.0) {
        return 1.0;
    }
    uint receiver = naga_f2u32(metal::max(who_1, 0.0));
    uint casters = 1 + (_buffer_sizes.size2 - 0 - 64) / 64;
    uint words = 1 + (_buffer_sizes.size3 - 0 - 4) / 4;
    if (!((receiver >= casters))) {
        local_1 = words < OCCLUDER_HEADER;
    } else {
        local_1 = true;
    }
    bool _e23 = local_1;
    if (_e23) {
        return 1.0;
    }
    uint columns = node_occluders[metal::min(unsigned(0), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint rows = node_occluders[metal::min(unsigned(1), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint _e33 = node_occluders[metal::min(unsigned(2), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint _e37 = node_occluders[metal::min(unsigned(3), (_buffer_sizes.size3 - 0 - 4) / 4)];
    metal::float2 origin = metal::float2(as_type<float>(_e33), as_type<float>(_e37));
    uint _e43 = node_occluders[metal::min(unsigned(4), (_buffer_sizes.size3 - 0 - 4) / 4)];
    metal::float2 bin = metal::floor((points_1 - origin) * as_type<float>(_e43));
    if (metal::all(bin >= metal::float2(0.0))) {
        local_2 = bin.x < static_cast<float>(columns);
    } else {
        local_2 = false;
    }
    bool _e57 = local_2;
    if (_e57) {
        local_3 = bin.y < static_cast<float>(rows);
    } else {
        local_3 = false;
    }
    bool _e64 = local_3;
    if (!(_e64)) {
        return 1.0;
    }
    uint slot = (OCCLUDER_HEADER + (naga_f2u32(bin.y) * columns)) + naga_f2u32(bin.x);
    if ((slot + 1u) >= words) {
        return 1.0;
    }
    uint _e83 = node_occluders[metal::min(unsigned(slot + 1u), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint end = metal::min(_e83, words);
    uint _e89 = node_occluders[metal::min(unsigned(slot), (_buffer_sizes.size3 - 0 - 4) / 4)];
    k = _e89;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e148 = k;
            k = _e148 + 1u;
        }
        loop_init = false;
        uint _e91 = k;
        if (_e91 < end) {
        } else {
            break;
        }
        {
            uint _e94 = k;
            uint at = node_occluders[metal::min(unsigned(_e94), (_buffer_sizes.size3 - 0 - 4) / 4)];
            if (!((at <= receiver))) {
                local_4 = at >= casters;
            } else {
                local_4 = true;
            }
            bool _e103 = local_4;
            if (_e103) {
                continue;
            }
            ShadowCaster caster = shadow_casters[metal::min(unsigned(at), (_buffer_sizes.size2 - 0 - 64) / 64)];
            if (metal::all(points_1 >= caster.rect.xy)) {
                local_5 = metal::all(points_1 <= (caster.rect.xy + caster.rect.zw));
            } else {
                local_5 = false;
            }
            bool _e121 = local_5;
            if (_e121) {
                float _e122 = shadow_kernel(at, points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
                float level_1 = metal::clamp(caster.shade.x, 0.0, 1.0);
                hidden = level_1 * _e122;
                if (caster.shade.y < 0.5) {
                    float _e135 = shadow_transmittance(_e122, 1.0, level_1);
                    hidden = 1.0 - _e135;
                }
                float _e138 = visibility;
                float _e139 = hidden;
                visibility = _e138 * (1.0 - (strength * _e139));
            }
            float _e144 = visibility;
            if (_e144 == 0.0) {
                break;
            }
        }
    }
    float _e150 = visibility;
    return _e150;
}

float outside_atlas(
    VertexOut in_1,
    metal::float2 texel
) {
    metal::float2 low = metal::clamp(texel + metal::float2(0.5), metal::float2(0.0), metal::float2(1.0));
    metal::float2 high = metal::clamp((in_1.sheet_size + metal::float2(0.5)) - texel, metal::float2(0.0), metal::float2(1.0));
    metal::float2 weight = low * high;
    return weight.x * weight.y;
}

float sheet_alpha(
    VertexOut in_2,
    metal::float2 texel_1,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    metal::float2 uv = texel_1 / in_2.sheet_size;
    if (in_2.sheet == SHEET_MARK) {
        metal::float4 _e10 = mark_atlas.sample(atlas_sampler, uv, metal::level(0.0));
        return _e10.w;
    }
    metal::float4 _e15 = atlas.sample(atlas_sampler, uv, metal::level(0.0));
    return _e15.w;
}

float tap(
    VertexOut in_3,
    metal::float2 texel_2,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    bool local_6 = {};
    bool local_7 = {};
    bool local_8 = {};
    if (!((texel_2.x < (in_3.uv_min.x - PATCH_MARGIN)))) {
        local_6 = texel_2.y < (in_3.uv_min.y - PATCH_MARGIN);
    } else {
        local_6 = true;
    }
    bool _e18 = local_6;
    if (!(_e18)) {
        local_7 = texel_2.x > (in_3.uv_max.x + PATCH_MARGIN);
    } else {
        local_7 = true;
    }
    bool _e29 = local_7;
    if (!(_e29)) {
        local_8 = texel_2.y > (in_3.uv_max.y + PATCH_MARGIN);
    } else {
        local_8 = true;
    }
    bool _e40 = local_8;
    if (_e40) {
        return 0.0;
    }
    float _e42 = sheet_alpha(in_3, texel_2, atlas, atlas_sampler, mark_atlas);
    float _e43 = outside_atlas(in_3, texel_2);
    return _e42 * _e43;
}

float coverage(
    VertexOut in_4,
    metal::float2 texel_3,
    constant Locals& locals,
    metal::texture2d<float, metal::access::sample> atlas,
    metal::sampler atlas_sampler,
    metal::texture2d<float, metal::access::sample> mark_atlas
) {
    metal::float2 _e4 = locals.filter_axis;
    if (metal::all(_e4 > metal::float2(0.0))) {
        metal::float2 x = metal::float2(0.25, 0.0);
        metal::float2 y = metal::float2(0.0, 0.25);
        float _e17 = tap(in_4, (texel_3 - x) - y, atlas, atlas_sampler, mark_atlas);
        float _e20 = tap(in_4, (texel_3 + x) - y, atlas, atlas_sampler, mark_atlas);
        float _e24 = tap(in_4, (texel_3 - x) + y, atlas, atlas_sampler, mark_atlas);
        float _e28 = tap(in_4, (texel_3 + x) + y, atlas, atlas_sampler, mark_atlas);
        return 0.25 * (((_e17 + _e20) + _e24) + _e28);
    }
    metal::float2 _e35 = locals.filter_axis;
    metal::float2 off = FILTER_TAP * _e35;
    float _e38 = tap(in_4, texel_3 - off, atlas, atlas_sampler, mark_atlas);
    float _e40 = tap(in_4, texel_3 + off, atlas, atlas_sampler, mark_atlas);
    return 0.5 * (_e38 + _e40);
}

struct fs_glyph_transmittanceInput {
    metal::float2 texel [[user(loc0), center_perspective]];
    metal::float2 uv_min [[user(loc1), flat]];
    metal::float2 uv_max [[user(loc2), flat]];
    metal::float4 fill [[user(loc3), flat]];
    uint sheet [[user(loc5), flat]];
    metal::float2 sheet_size [[user(loc6), flat]];
    metal::float2 points [[user(loc7), center_perspective]];
    float who [[user(loc8), flat]];
};
struct fs_glyph_transmittanceOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glyph_transmittanceOutput fs_glyph_transmittance(
  fs_glyph_transmittanceInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(4)]]
, metal::sampler shadow_sampler [[sampler(2)]]
, device type_5 const& shadow_casters [[buffer(1)]]
, device type_7 const& node_occluders [[buffer(2)]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
, metal::sampler atlas_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> mark_atlas [[texture(1)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VertexOut in = { position, varyings.texel, varyings.uv_min, varyings.uv_max, {}, varyings.fill, varyings.sheet, {}, varyings.sheet_size, varyings.points, varyings.who };
    float _e2 = coverage(in, in.texel, locals, atlas, atlas_sampler, mark_atlas);
    float _e10 = locals.node_occlusion;
    float _e11 = node_visibility(in.who, in.points, _e10, shadow_atlas, shadow_sampler, shadow_casters, node_occluders, _buffer_sizes);
    float a = (_e2 * in.fill.w) * _e11;
    return fs_glyph_transmittanceOutput { metal::float4(a, 0.0, 0.0, a) };
}
