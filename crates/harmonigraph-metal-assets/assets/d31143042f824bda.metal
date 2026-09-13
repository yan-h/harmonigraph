// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size3;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_6[1];
struct SplitOut {
    metal::float4 other;
    metal::float4 ink;
};
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
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant uint SHEET_MARK = 1u;
constant float PATCH_MARGIN = 0.5;
constant float FILTER_TAP = 0.25;
constant float SDF_NEAR_PAD = 32.0;
constant float SDF_COARSE_PAD = 48.0;
constant float SDF_NEAR_BLEND = 8.0;

metal::float4 glow_light(
    metal::int2 coord,
    metal::texture2d<float, metal::access::sample> glow_tex
) {
    uint clamped_lod_e3 = metal::min(uint(0), glow_tex.get_num_mip_levels() - 1);
    metal::float4 _e3 = glow_tex.read(metal::min(metal::uint2(coord), metal::uint2(glow_tex.get_width(clamped_lod_e3), glow_tex.get_height(clamped_lod_e3)) - 1), clamped_lod_e3);
    return _e3;
}

metal::float3 wash_over(
    metal::float3 ink,
    float alpha,
    metal::float3 light,
    float share
) {
    metal::float3 w_1 = light * share;
    return (w_1 * alpha) + (ink * (metal::float3(1.0) - w_1));
}

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

float shadow_stop(
    float falloff
) {
    if (falloff >= SHADOW_FALLOFF_FREE) {
        return SHADOW_STOP;
    }
    return metal::max(SHADOW_STOP, metal::pow(SHADOW_INVISIBLE_FOLDS, 1.0 / falloff));
}

float standoff_coverage(
    float d,
    float w,
    float falloff_1
) {
    float t = {};
    float u = metal::max(d, 0.0) / metal::max(w, 0.000001);
    float f = metal::max(falloff_1, SHADOW_FALLOFF_FLOOR);
    t = u;
    if (f != 1.0) {
        t = metal::pow(u, f);
    }
    float _e15 = t;
    float _e18 = shadow_stop(f);
    return metal::exp(-4.0 * _e15) * (1.0 - metal::smoothstep(1.0, _e18, u));
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64)) {
        return 0.0;
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].cell;
    bool _e10 = cell_packed(cell_1);
    if (!(_e10)) {
        return 0.0;
    }
    metal::float2 atlas_1 = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].map;
    metal::float2 texel_4 = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel_4 / atlas_1, metal::level(0.0));
    float held = _e39.x;
    float _e45 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.y;
    if (_e45 >= 0.5) {
        float _e52 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.z;
        float _e59 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.w;
        float _e60 = standoff_coverage(held, 2.0 * _e52, _e59);
        return metal::clamp(_e60, 0.0, 1.0);
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
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint at = {};
    float visibility = 1.0;
    bool local_1 = {};
    bool local_2 = {};
    float hidden = {};
    float strength = metal::clamp(occlusion, 0.0, 1.0);
    if (strength == 0.0) {
        return 1.0;
    }
    at = naga_f2u32(metal::max(who_1, 0.0));
    uint _e13 = at;
    if (_e13 >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64)) {
        return 1.0;
    }
    uint2 loop_bound = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        uint _e21 = at;
        float _e25 = shadow_casters[metal::min(unsigned(_e21), (_buffer_sizes.size3 - 0 - 64) / 64)].map.w;
        uint next = naga_f2u32(_e25);
        if (next == 0u) {
            break;
        }
        uint candidate = next - 1u;
        uint _e31 = at;
        if (!((candidate <= _e31))) {
            local_1 = candidate >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64);
        } else {
            local_1 = true;
        }
        bool _e40 = local_1;
        if (_e40) {
            break;
        }
        at = candidate;
        uint _e42 = at;
        ShadowCaster caster = shadow_casters[metal::min(unsigned(_e42), (_buffer_sizes.size3 - 0 - 64) / 64)];
        if (metal::all(points_1 >= caster.rect.xy)) {
            local_2 = metal::all(points_1 <= (caster.rect.xy + caster.rect.zw));
        } else {
            local_2 = false;
        }
        bool _e59 = local_2;
        if (_e59) {
            uint _e60 = at;
            float _e61 = shadow_kernel(_e60, points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
            float level_1 = metal::clamp(caster.shade.x, 0.0, 1.0);
            hidden = level_1 * _e61;
            if (caster.shade.y < 0.5) {
                float _e74 = shadow_transmittance(_e61, 1.0, level_1);
                hidden = 1.0 - _e74;
            }
            float _e77 = visibility;
            float _e78 = hidden;
            visibility = _e77 * (1.0 - (strength * _e78));
        }
        float _e83 = visibility;
        if (_e83 == 0.0) {
            break;
        }
    }
    float _e86 = visibility;
    return _e86;
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
    bool local_3 = {};
    bool local_4 = {};
    bool local_5 = {};
    if (!((texel_2.x < (in_3.uv_min.x - PATCH_MARGIN)))) {
        local_3 = texel_2.y < (in_3.uv_min.y - PATCH_MARGIN);
    } else {
        local_3 = true;
    }
    bool _e18 = local_3;
    if (!(_e18)) {
        local_4 = texel_2.x > (in_3.uv_max.x + PATCH_MARGIN);
    } else {
        local_4 = true;
    }
    bool _e29 = local_4;
    if (!(_e29)) {
        local_5 = texel_2.y > (in_3.uv_max.y + PATCH_MARGIN);
    } else {
        local_5 = true;
    }
    bool _e40 = local_5;
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

metal::float4 glyph_light(
    metal::int2 coord_1,
    metal::texture2d<float, metal::access::sample> glow_tex
) {
    metal::int2 edge = as_type<metal::int2>(as_type<metal::uint2>(static_cast<metal::int2>(metal::uint2(glow_tex.get_width(), glow_tex.get_height()))) - as_type<metal::uint2>(metal::int2(1, 1)));
    metal::float4 _e12 = glow_light(metal::clamp(coord_1, metal::int2(0, 0), edge), glow_tex);
    return _e12;
}
metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}


struct fs_fill_litInput {
    metal::float2 texel [[user(loc0), center_perspective]];
    metal::float2 uv_min [[user(loc1), flat]];
    metal::float2 uv_max [[user(loc2), flat]];
    metal::float4 fill [[user(loc3), flat]];
    uint sheet [[user(loc5), flat]];
    metal::float2 sheet_size [[user(loc6), flat]];
    metal::float2 points [[user(loc7), center_perspective]];
    float who [[user(loc8), flat]];
};
struct fs_fill_litOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
};
fragment fs_fill_litOutput fs_fill_lit(
  fs_fill_litInput varyings [[stage_in]]
, metal::float4 position [[position]]
, metal::texture2d<float, metal::access::sample> glow_tex [[texture(3)]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(4)]]
, metal::sampler shadow_sampler [[sampler(2)]]
, device type_6 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> atlas [[texture(0)]]
, metal::sampler atlas_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> mark_atlas [[texture(1)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const VertexOut in = { position, varyings.texel, varyings.uv_min, varyings.uv_max, {}, varyings.fill, varyings.sheet, {}, varyings.sheet_size, varyings.points, varyings.who };
    float _e2 = coverage(in, in.texel, locals, atlas, atlas_sampler, mark_atlas);
    if (_e2 <= 0.0) {
        metal::discard_fragment();
    }
    float _e9 = locals.node_occlusion;
    float _e10 = node_visibility(in.who, in.points, _e9, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    metal::float4 ink_1 = in.fill * _e2;
    float alpha_1 = ink_1.w * _e10;
    metal::int2 coord_2 = naga_f2i32(in.position.xy);
    metal::float4 _e18 = glyph_light(coord_2, glow_tex);
    metal::float3 _e27 = wash_over(ink_1.xyz, ink_1.w, _e18.xyz, 1.0);
    const auto _tmp = SplitOut {metal::float4(0.0, 0.0, 0.0, alpha_1), metal::float4(_e27 * _e10, alpha_1)};
    return fs_fill_litOutput { _tmp.other, _tmp.ink };
}
