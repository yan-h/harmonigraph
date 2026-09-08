// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size0;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_2[1];
struct Locals {
    metal::float2 screen_points;
    metal::float2 atlas_size;
    metal::float2 mark_atlas_size;
    metal::float2 filter_axis;
    float pixels_per_point;
    float shadow_depth;
    metal::float2 shadow_atlas_size;
    metal::float4 _pad;
};
struct BoxOut {
    metal::float4 position;
    metal::float2 at;
    float level;
    uint who;
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

metal::float4 on_screen(
    metal::float2 pos,
    constant Locals& locals
) {
    float _e7 = locals.screen_points.x;
    float _e17 = locals.screen_points.y;
    return metal::float4(((2.0 * pos.x) / _e7) - 1.0, 1.0 - ((2.0 * pos.y) / _e17), 0.0, 1.0);
}

struct vs_shadow_boxInput {
};
struct vs_shadow_boxOutput {
    metal::float4 position [[position]];
    metal::float2 at [[user(loc0), center_perspective]];
    float level [[user(loc1), flat]];
    uint who [[user(loc2), flat]];
};
vertex vs_shadow_boxOutput vs_shadow_box(
  uint vertex_ [[vertex_id]]
, uint who [[instance_id]]
, device type_2 const& shadow_casters [[buffer(1)]]
, constant Locals& locals [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    BoxOut out = {};
    metal::float2 corner = metal::float2(((vertex_ & 1u) == 1u) ? 1.0 : 0.0, ((vertex_ & 2u) == 2u) ? 1.0 : 0.0);
    ShadowCaster caster = shadow_casters[metal::min(unsigned(metal::min(who, (1 + (_buffer_sizes.size0 - 0 - 64) / 64) - 1u)), (_buffer_sizes.size0 - 0 - 64) / 64)];
    metal::float2 at = caster.rect.xy + (corner * caster.rect.zw);
    metal::float4 _e33 = on_screen(at, locals);
    out.position = _e33;
    out.at = at;
    out.level = caster.shade.x;
    out.who = who;
    BoxOut _e39 = out;
    const auto _tmp = _e39;
    return vs_shadow_boxOutput { _tmp.position, _tmp.at, _tmp.level, _tmp.who };
}
