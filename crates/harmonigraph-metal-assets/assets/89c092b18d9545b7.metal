// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size4;
    uint size5;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_6[1];
typedef uint type_8[1];
struct SplitOut {
    metal::float4 other;
    metal::float4 ink;
};
struct SceneOut {
    metal::float4 other;
    metal::float4 ink;
    metal::float4 bloom_other;
    metal::float4 bloom_ink;
};
struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
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
struct type_11 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_11 bounds;
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
    float blend;
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
    metal::float2 padding;
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
    float padding;
};
struct type_12 {
    metal::float4 inner[64];
};
struct type_14 {
    metal::uint4 inner[240];
};
struct type_15 {
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
    type_12 pitch_lut;
    type_12 spectral_lut;
    type_14 spectrum_color;
    type_15 ink_kernel;
};
struct ShadowThrough {
    float seen;
    float bloom;
};
struct VsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 params;
    metal::uint3 octaves;
    metal::uint3 thickness;
    metal::uint4 motion;
    float cents;
    float strip_row;
    metal::uint2 marks;
    metal::float4 melody_color;
    metal::float4 bass_color;
    float rim;
    float ring;
    float ink_carry;
    char _pad14[4];
    metal::float4 shadow_box;
    metal::float4 shadow_at;
};
struct OctRing {
    int base;
    float seam;
};
struct NodeLayer {
    float sd;
    float level;
    float coverage;
};
struct SliceZones {
    metal::float4 ink;
    NodeLayer lit;
    char _pad2[4];
};
struct SectorFold {
    metal::float2 q;
    metal::float2 e;
};
struct RingInk {
    metal::packed_float3 color;
    float cov;
    float lit;
    NodeLayer layer;
};
struct NodeGeom {
    float d;
    float aa;
    OctRing oct;
    bool paints;
    char _pad4[3];
};
struct NodeInk {
    metal::packed_float3 rgb;
    float alpha;
    float lit;
    float mask;
    float sd;
    char _pad5[4];
};
struct AnimatedInk {
    NodeInk body;
    NodeInk marks;
};
struct Painted {
    metal::packed_float3 rgb;
    float seen;
    float bloom;
    float ink_alpha;
    char _pad4[8];
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

metal::float4 glow_light(
    metal::float2 uv,
    metal::texture2d<float, metal::access::sample> glow_tex,
    metal::sampler glow_sampler
) {
    metal::float4 _e4 = glow_tex.sample(glow_sampler, uv, metal::level(0.0));
    return _e4;
}

metal::float3 wash_over(
    metal::float3 ink,
    float alpha,
    metal::float3 light,
    float share
) {
    metal::float3 w_3 = light * share;
    return (w_3 * alpha) + (ink * (metal::float3(1.0) - w_3));
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

float standoff_coverage(
    float d,
    float w,
    float falloff
) {
    float u_1 = metal::clamp((metal::max(d, 0.0) / metal::max(w, 0.000001)) / SHADOW_STOP, 0.0, 1.0);
    float remaining = 1.0 - u_1;
    float shape = -(metal::clamp(falloff, SHADOW_FALLOFF_MIN, SHADOW_FALLOFF_MAX));
    if (metal::abs(shape) < 0.05) {
        return remaining * ((1.0 - ((shape * u_1) * 0.5)) + ((((shape * shape) * u_1) * ((2.0 * u_1) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (who >= (1 + (_buffer_sizes.size4 - 0 - 64) / 64)) {
        return 0.0;
    }
    metal::float4 cell_1 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].cell;
    bool _e10 = cell_packed(cell_1);
    if (!(_e10)) {
        return 0.0;
    }
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].map;
    metal::float2 texel = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel / atlas, metal::level(0.0));
    float held = _e39.x;
    float _e45 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].shade.y;
    if (_e45 == DISTANCE_COVERAGE_KIND) {
        return metal::clamp(held, 0.0, 1.0);
    }
    float _e55 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].shade.y;
    if (_e55 >= 0.5) {
        float _e62 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].shade.z;
        float _e69 = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size4 - 0 - 64) / 64)].shade.w;
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
    device type_6 const& shadow_casters,
    device type_8 const& node_occluders,
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
    uint casters = 1 + (_buffer_sizes.size4 - 0 - 64) / 64;
    uint words = 1 + (_buffer_sizes.size5 - 0 - 4) / 4;
    if (!((receiver >= casters))) {
        local_1 = words < OCCLUDER_HEADER;
    } else {
        local_1 = true;
    }
    bool _e23 = local_1;
    if (_e23) {
        return 1.0;
    }
    uint columns = node_occluders[metal::min(unsigned(0), (_buffer_sizes.size5 - 0 - 4) / 4)];
    uint rows = node_occluders[metal::min(unsigned(1), (_buffer_sizes.size5 - 0 - 4) / 4)];
    uint _e33 = node_occluders[metal::min(unsigned(2), (_buffer_sizes.size5 - 0 - 4) / 4)];
    uint _e37 = node_occluders[metal::min(unsigned(3), (_buffer_sizes.size5 - 0 - 4) / 4)];
    metal::float2 origin = metal::float2(as_type<float>(_e33), as_type<float>(_e37));
    uint _e43 = node_occluders[metal::min(unsigned(4), (_buffer_sizes.size5 - 0 - 4) / 4)];
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
    uint slot_2 = (OCCLUDER_HEADER + (naga_f2u32(bin.y) * columns)) + naga_f2u32(bin.x);
    if ((slot_2 + 1u) >= words) {
        return 1.0;
    }
    uint _e83 = node_occluders[metal::min(unsigned(slot_2 + 1u), (_buffer_sizes.size5 - 0 - 4) / 4)];
    uint end = metal::min(_e83, words);
    uint _e89 = node_occluders[metal::min(unsigned(slot_2), (_buffer_sizes.size5 - 0 - 4) / 4)];
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
            uint at = node_occluders[metal::min(unsigned(_e94), (_buffer_sizes.size5 - 0 - 4) / 4)];
            if (!((at <= receiver))) {
                local_4 = at >= casters;
            } else {
                local_4 = true;
            }
            bool _e103 = local_4;
            if (_e103) {
                continue;
            }
            ShadowCaster caster = shadow_casters[metal::min(unsigned(at), (_buffer_sizes.size4 - 0 - 64) / 64)];
            if (metal::all(points_1 >= caster.rect.xy)) {
                local_5 = metal::all(points_1 <= (caster.rect.xy + caster.rect.zw));
            } else {
                local_5 = false;
            }
            bool _e121 = local_5;
            if (_e121) {
                float _e122 = shadow_kernel(at, points_1, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
                float level_5 = metal::clamp(caster.shade.x, 0.0, 1.0);
                hidden = level_5 * _e122;
                if (caster.shade.y < 0.5) {
                    float _e135 = shadow_transmittance(_e122, 1.0, level_5);
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

float glow_shadow_depth(
    constant Uniforms& u
) {
    float _e3 = u.geometry_shadow.depth;
    return metal::clamp(_e3, 0.0, 1.0);
}

ShadowThrough node_shadow_through(
    float who_2,
    metal::float2 points_2,
    float level_1,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (level_1 <= 0.0) {
        return ShadowThrough {1.0, 1.0};
    }
    float _e14 = shadow_kernel(naga_f2u32(metal::max(who_2, 0.0)), points_2, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float coverage_2 = metal::clamp(level_1, 0.0, 1.0) * _e14;
    float _e16 = glow_shadow_depth(u);
    return ShadowThrough {1.0 - (_e16 * coverage_2), 1.0 - coverage_2};
}

bool shadow_is_distance(
    float who_3,
    device type_6 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint caster_1 = naga_f2u32(metal::max(who_3, 0.0));
    if (caster_1 >= (1 + (_buffer_sizes.size4 - 0 - 64) / 64)) {
        return false;
    }
    float _e12 = shadow_casters[metal::min(unsigned(caster_1), (_buffer_sizes.size4 - 0 - 64) / 64)].shade.y;
    return _e12 >= 0.5;
}

float glow_wash(
    constant Uniforms& u
) {
    float _e3 = u.glow.wash;
    return metal::clamp(_e3, 0.0, 1.0);
}

metal::float2 spectral_radii(
    constant Uniforms& u
) {
    float _e3 = u.spectral.inner;
    float _e7 = u.spectral.outer;
    return metal::float2(_e3, _e7);
}

float paint_reach(
    VsOut in_1,
    float aa,
    constant Uniforms& u
) {
    float reach = GLYPH_FADE_LIMIT;
    bool local_6 = {};
    if (!((in_1.marks.x != 0u))) {
        local_6 = in_1.marks.y != 0u;
    } else {
        local_6 = true;
    }
    bool _e16 = local_6;
    if (_e16) {
        float _e17 = reach;
        reach = metal::max(_e17, QUAD_MARGIN);
    }
    float _e23 = u.node.animation;
    if (_e23 != 0.0) {
        float _e26 = reach;
        float _e32 = u.node.pose.z;
        reach = metal::max(_e26, (in_1.rim * _e32) + aa);
    }
    float _e36 = reach;
    metal::float2 _e38 = spectral_radii(u);
    return metal::max(_e36, metal::max(in_1.rim, _e38.y) + aa);
}

float aa_inside(
    float edge,
    float x,
    float w_1
) {
    return 1.0 - metal::smoothstep(edge - w_1, edge + w_1, x);
}

float aa_width(
    float coord_fwidth,
    float surface_scale,
    constant Uniforms& u
) {
    float _e6 = u.composite.render_scale;
    float knob = (AA_SOFTNESS_PX * metal::max(_e6, 0.01)) * metal::clamp(surface_scale, 0.0, 1.0);
    return metal::max(coord_fwidth, 0.0001) * metal::max(knob, 1.0);
}

uint naga_div(uint lhs, uint rhs) {
    return lhs / metal::select(rhs, 1u, rhs == 0u);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

float octave_level(
    metal::uint3 octaves,
    uint i
) {
    uint word = octaves[metal::min(unsigned(naga_div(i, 4u)), 2u)];
    return static_cast<float>((word >> (naga_mod(i, 4u) * 8u)) & 255u) / 255.0;
}

uint oct_span(
    constant Uniforms& u
) {
    float _e3 = u.octave.span;
    return metal::clamp(naga_f2u32(_e3), 1u, MAX_SPAN);
}

float oct_center(
    constant Uniforms& u
) {
    float _e3 = u.octave.center;
    return _e3;
}

float oct_bound(
    uint j,
    constant Uniforms& u
) {
    float _e10 = u.octave.bounds.inner[metal::min(unsigned(naga_div(j, 4u)), 2u)][metal::min(unsigned(naga_mod(j, 4u)), 3u)];
    return _e10;
}

float oct_walk(
    float x_1,
    constant Uniforms& u
) {
    uint _e1 = oct_span(u);
    float c = metal::clamp(x_1, 0.0, static_cast<float>(_e1));
    uint _e9 = oct_span(u);
    uint j_1 = metal::min(naga_f2u32(metal::max(metal::floor(c), 0.0)), _e9 - 1u);
    float _e13 = oct_bound(j_1, u);
    float _e16 = oct_bound(j_1 + 1u, u);
    return metal::mix(_e13, _e16, c - static_cast<float>(j_1));
}

float oct_slot_pitch(
    int s,
    float cents
) {
    return (static_cast<float>(s) * 12.0) + (cents / 100.0);
}

int naga_neg(int val) {
    return as_type<int>(-as_type<uint>(val));
}

int naga_div(int lhs, int rhs) {
    return lhs / metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

OctRing oct_ring(
    float cents_1,
    constant Uniforms& u
) {
    int low = {};
    OctRing ring = {};
    float off = cents_1 / 100.0;
    uint _e3 = oct_span(u);
    int span = static_cast<int>(_e3);
    float _e5 = oct_center(u);
    float nearest = metal::floor(((_e5 - off) / 12.0) + 0.5);
    float _e15 = oct_center(u);
    float d_5 = ((nearest * 12.0) + off) - _e15;
    low = naga_div(naga_neg(as_type<int>(as_type<uint>(span) - as_type<uint>(1))), 2);
    if (naga_mod(span, 2) == 0) {
        low = (d_5 < 0.0) ? as_type<int>(as_type<uint>(1) - as_type<uint>(naga_div(span, 2))) : naga_div(naga_neg(span), 2);
    }
    int _e40 = low;
    ring.base = as_type<int>(as_type<uint>(naga_f2i32(nearest)) + as_type<uint>(_e40));
    float _e42 = oct_center(u);
    int _e44 = ring.base;
    float _e45 = oct_slot_pitch(_e44, cents_1);
    float along = ((_e42 - _e45) / 12.0) + 0.5;
    float _e53 = oct_walk(along, u);
    ring.seam = OCT_UP + _e53;
    OctRing _e55 = ring;
    return _e55;
}

metal::float2 oct_sector(
    int s_1,
    OctRing ring_1,
    constant Uniforms& u
) {
    uint _e4 = oct_span(u);
    uint i_6 = static_cast<uint>(metal::clamp(as_type<int>(as_type<uint>(s_1) - as_type<uint>(ring_1.base)), 0, as_type<int>(as_type<uint>(static_cast<int>(_e4)) - as_type<uint>(1))));
    float _e12 = oct_bound(i_6, u);
    float _e17 = oct_bound(i_6 + 1u, u);
    return metal::float2(ring_1.seam - _e12, ring_1.seam - _e17);
}

float oct_mid(
    int s_2,
    OctRing ring_2,
    constant Uniforms& u
) {
    metal::float2 _e2 = oct_sector(s_2, ring_2, u);
    return 0.5 * (_e2.x + _e2.y);
}

float oct_slot_level(
    metal::uint3 octaves_1,
    int s_3
) {
    bool local_7 = {};
    if (!((s_3 < 0))) {
        local_7 = s_3 >= 11;
    } else {
        local_7 = true;
    }
    bool _e10 = local_7;
    if (_e10) {
        return 0.0;
    }
    float _e13 = octave_level(octaves_1, static_cast<uint>(s_3));
    return _e13;
}

float oct_arc_coverage(
    metal::float2 edges,
    metal::float2 uv_1,
    float aa_1
) {
    metal::float2 b1_ = metal::float2(metal::cos(edges.x), metal::sin(edges.x));
    metal::float2 b2_ = metal::float2(metal::cos(edges.y), metal::sin(edges.y));
    float c1_ = (uv_1.x * b1_.y) - (uv_1.y * b1_.x);
    float c2_ = (uv_1.x * b2_.y) - (uv_1.y * b2_.x);
    float s1_ = metal::smoothstep(-(aa_1), aa_1, c1_);
    float s2_ = metal::smoothstep(-(aa_1), aa_1, -(c2_));
    return ((edges.x - edges.y) > 3.1415927) ? (1.0 - ((1.0 - s1_) * (1.0 - s2_))) : (s1_ * s2_);
}

float slice_gap_half(
    constant Uniforms& u
) {
    float _e3 = u.node.angular_gap;
    return metal::max(_e3, 0.0) * 0.5;
}

float layer_coverage(
    NodeLayer layer
) {
    return layer.coverage * layer.level;
}

float layer_distance(
    float field,
    NodeLayer layer_1,
    VsOut in_2
) {
    if (in_2.params.w > 1.5) {
        float _e20 = standoff_coverage(layer_1.sd * metal::abs(in_2.shadow_at.z), 2.0 * in_2.strip_row, in_2.ink_carry);
        float coverage_3 = metal::clamp(layer_1.level, 0.0, 1.0) * _e20;
        return metal::min(field, -(coverage_3));
    }
    return (layer_1.level >= DISTANCE_LEVEL_FLOOR) ? metal::min(field, layer_1.sd) : field;
}

NodeLayer glyph_band(
    float d_1,
    float inner,
    float outer,
    float level_2,
    float aa_2
) {
    float mid = 0.5 * (inner + outer);
    float _e14 = aa_inside(outer, d_1, aa_2);
    float _e15 = aa_inside(inner, d_1, aa_2);
    return NodeLayer {metal::abs(d_1 - mid) - (0.5 * (outer - inner)), level_2, _e14 * (1.0 - _e15)};
}

float lut_position(
    float t,
    metal::float2 corner
) {
    float w_2 = {};
    float x_2 = corner.x;
    float y = corner.y;
    if (x_2 == y) {
        return t;
    }
    float low_1 = y / x_2;
    float high = (1.0 - y) / (1.0 - x_2);
    float x0_ = x_2 * 0.6;
    float x2_ = x_2 + (BEND_ROUNDING * (1.0 - x_2));
    if (t <= x0_) {
        w_2 = low_1 * t;
    } else {
        if (t >= x2_) {
            w_2 = y + (high * (t - x_2));
        } else {
            float y0_ = low_1 * x0_;
            float y2_ = y + (high * (x2_ - x_2));
            float a = (x0_ - (2.0 * x_2)) + x2_;
            float b_1 = 2.0 * (x_2 - x0_);
            float c_1 = x0_ - t;
            float s_7 = (-2.0 * c_1) / (b_1 + metal::sqrt(metal::max((b_1 * b_1) - ((4.0 * a) * c_1), 0.0)));
            w_2 = ((((1.0 - s_7) * (1.0 - s_7)) * y0_) + (((2.0 * s_7) * (1.0 - s_7)) * y)) + ((s_7 * s_7) * y2_);
        }
    }
    float _e65 = w_2;
    return 0.5 * (t + metal::clamp(_e65, 0.0, 1.0));
}

metal::float3 pitch_lut_color(
    float pitch,
    constant Uniforms& u
) {
    float _e4 = u.composite.darkest_pitch;
    float _e9 = u.composite.brightest_pitch;
    float _e13 = u.composite.darkest_pitch;
    float t_1 = metal::clamp((pitch - _e4) / metal::max(_e9 - _e13, 0.01), 0.0, 1.0);
    metal::float4 _e23 = u.lut_spacing;
    float _e25 = lut_position(t_1, _e23.xy);
    float f_3 = _e25 * 63.0;
    uint i0_ = naga_f2u32(metal::floor(f_3));
    uint i1_ = metal::min(i0_ + 1u, 63u);
    metal::float4 _e37 = u.pitch_lut.inner[metal::min(unsigned(i0_), 63u)];
    metal::float4 _e42 = u.pitch_lut.inner[metal::min(unsigned(i1_), 63u)];
    return metal::mix(_e37.xyz, _e42.xyz, f_3 - metal::floor(f_3));
}

metal::float4 oct_slot_lit(
    VsOut in_3,
    int slot,
    constant Uniforms& u
) {
    float _e3 = oct_slot_pitch(slot, in_3.cents);
    metal::float3 _e4 = pitch_lut_color(_e3, u);
    float _e6 = oct_slot_level(in_3.octaves, slot);
    return metal::float4(_e4, _e6);
}

metal::float4 oct_slot_ink(
    VsOut in_4,
    int slot_1,
    constant Uniforms& u
) {
    float presence = in_4.params.x;
    metal::float4 _e4 = oct_slot_lit(in_4, slot_1, u);
    float level_6 = _e4.w;
    if (level_6 <= 0.0) {
        metal::float4 _e10 = u.lattice_ground;
        return metal::float4(_e10.xyz, presence);
    }
    float ghost_rest = metal::max(presence - level_6, 0.0);
    float opacity = level_6 + ghost_rest;
    metal::float4 _e19 = u.lattice_ground;
    metal::float3 ground = _e19.xyz;
    return metal::float4(((_e4.xyz * level_6) + (ground * ghost_rest)) / metal::float3(opacity), opacity);
}

float slice_reach(
    VsOut in_5,
    int s_4,
    float inner_1,
    float outer_1
) {
    bool local_8 = {};
    if (!((s_4 < 0))) {
        local_8 = s_4 >= 11;
    } else {
        local_8 = true;
    }
    bool _e12 = local_8;
    if (_e12) {
        return outer_1;
    }
    float _e15 = octave_level(in_5.thickness, static_cast<uint>(s_4));
    return (_e15 >= 1.0) ? outer_1 : (inner_1 + ((outer_1 - inner_1) * _e15));
}

SectorFold sector_fold(
    metal::float2 uv_2,
    metal::float2 edges_1
) {
    float mid_1 = 0.5 * (edges_1.x + edges_1.y);
    float half_ = metal::clamp(0.5 * (edges_1.x - edges_1.y), 0.0, 3.1415927);
    float c_2 = metal::cos(mid_1);
    float s_8 = metal::sin(mid_1);
    return SectorFold {metal::float2(metal::abs((uv_2.y * c_2) - (uv_2.x * s_8)), (uv_2.x * c_2) + (uv_2.y * s_8)), metal::float2(metal::sin(half_), metal::cos(half_))};
}

float sector_side(
    SectorFold f
) {
    return (f.e.y * f.q.x) - (f.e.x * f.q.y);
}

float sector_pie(
    SectorFold f_1,
    float r
) {
    float arc = metal::length(f_1.q) - r;
    float edge_1 = metal::length(f_1.q - (f_1.e * metal::clamp(metal::dot(f_1.q, f_1.e), 0.0, r)));
    float _e15 = sector_side(f_1);
    return metal::max(arc, edge_1 * metal::sign(_e15));
}

float annular_sector_distance(
    SectorFold f_2,
    float inner_2,
    float outer_2,
    float gap
) {
    float distance = {};
    bool local_9 = {};
    bool local_10 = {};
    if (outer_2 <= inner_2) {
        return EMPTY_DISTANCE;
    }
    metal::float2 normal = metal::float2(f_2.e.y, -(f_2.e.x));
    float radius = metal::length(f_2.q);
    float axis_t = (gap * f_2.e.y) / metal::max(f_2.e.x, 0.000001);
    float inner_t = metal::sqrt(metal::max((inner_2 * inner_2) - (gap * gap), 0.0));
    float outer_t = metal::sqrt(metal::max((outer_2 * outer_2) - (gap * gap), 0.0));
    float segment_start = metal::max(axis_t, inner_t);
    if (segment_start > outer_t) {
        return EMPTY_DISTANCE;
    }
    float side_t = metal::clamp(metal::dot(f_2.q, f_2.e), segment_start, outer_t);
    distance = metal::length(f_2.q - ((side_t * f_2.e) - (gap * normal)));
    metal::float2 direction = f_2.q / metal::float2(metal::max(radius, 0.000001));
    metal::float2 outer_at = direction * outer_2;
    if ((metal::dot(normal, outer_at) + gap) <= 0.0) {
        float _e59 = distance;
        distance = metal::min(_e59, metal::abs(radius - outer_2));
    }
    metal::float2 inner_at = direction * inner_2;
    if ((metal::dot(normal, inner_at) + gap) <= 0.0) {
        float _e68 = distance;
        distance = metal::min(_e68, metal::abs(radius - inner_2));
    }
    if (radius >= inner_2) {
        local_9 = radius <= outer_2;
    } else {
        local_9 = false;
    }
    bool _e77 = local_9;
    if (_e77) {
        local_10 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_10 = false;
    }
    bool inside = local_10;
    float _e87 = distance;
    float _e88 = distance;
    return inside ? -(_e88) : _e87;
}

NodeLayer outer_glyph(
    int s_5,
    OctRing ring_3,
    metal::float2 uv_3,
    NodeLayer band,
    float inner_3,
    float outer_3,
    float aa_3,
    constant Uniforms& u
) {
    float sd = {};
    metal::float2 _e7 = oct_sector(s_5, ring_3, u);
    SectorFold _e8 = sector_fold(uv_3, _e7);
    float _e9 = slice_gap_half(u);
    float gap_1 = (metal::dot(_e8.q, _e8.e) > 0.0) ? _e9 : 0.0;
    float _e17 = sector_side(_e8);
    float side = _e17 + gap_1;
    float _e19 = sector_pie(_e8, outer_3);
    float width = _e7.x - _e7.y;
    if (width > 3.1415927) {
        sd = metal::max(metal::max(band.sd, _e19), side);
    } else {
        float _e29 = slice_gap_half(u);
        float _e30 = annular_sector_distance(_e8, inner_3, outer_3, _e29);
        sd = _e30;
    }
    metal::float2 b1_1 = metal::float2(metal::cos(_e7.x), metal::sin(_e7.x));
    metal::float2 b2_1 = metal::float2(metal::cos(_e7.y), metal::sin(_e7.y));
    float c1_1 = (uv_3.x * b1_1.y) - (uv_3.y * b1_1.x);
    float c2_1 = (uv_3.x * b2_1.y) - (uv_3.y * b2_1.x);
    float _e55 = oct_arc_coverage(_e7, uv_3, aa_3);
    float _e56 = slice_gap_half(u);
    float _e58 = aa_inside(_e56, metal::abs(c1_1), aa_3);
    float _e66 = aa_inside(_e56, metal::abs(c2_1), aa_3);
    float gaps = (1.0 - (_e58 * metal::smoothstep(-(aa_3), aa_3, metal::dot(uv_3, b1_1)))) * (1.0 - (_e66 * metal::smoothstep(-(aa_3), aa_3, metal::dot(uv_3, b2_1))));
    float _e74 = sd;
    return NodeLayer {_e74, band.level, (band.coverage * _e55) * gaps};
}

SliceZones slice_zones(
    VsOut in_6,
    int s_6,
    OctRing ring_4,
    metal::float2 uv_4,
    float d_2,
    NodeLayer full_1,
    metal::float4 ink_1,
    float inner_4,
    float reach_1,
    float aa_4,
    constant Uniforms& u
) {
    NodeLayer lit = NodeLayer {65504.0, 0.0, 0.0};
    if (reach_1 > inner_4) {
        NodeLayer _e17 = glyph_band(d_2, inner_4, reach_1, 1.0, aa_4);
        NodeLayer _e18 = outer_glyph(s_6, ring_4, uv_4, _e17, inner_4, reach_1, aa_4, u);
        lit = _e18;
    }
    NodeLayer _e19 = lit;
    float _e20 = layer_coverage(_e19);
    float lit_cov = _e20 * ink_1.w;
    float _e23 = layer_coverage(full_1);
    NodeLayer _e24 = lit;
    float _e25 = layer_coverage(_e24);
    float ghost_cov = metal::max(_e23 - _e25, 0.0) * in_6.params.x;
    float cov_2 = lit_cov + ghost_cov;
    metal::float4 _e37 = u.lattice_ground;
    metal::float3 rgb_1 = ((ink_1.xyz * lit_cov) + (_e37.xyz * ghost_cov)) / metal::float3(metal::max(cov_2, 0.0001));
    NodeLayer _e46 = lit;
    return SliceZones {metal::float4(rgb_1, cov_2), _e46};
}

metal::float3 spectral_lut_color(
    float level_3,
    constant Uniforms& u
) {
    metal::float4 _e6 = u.lut_spacing;
    float _e8 = lut_position(metal::clamp(level_3, 0.0, 1.0), _e6.zw);
    float f_4 = _e8 * 63.0;
    uint i0_1 = naga_f2u32(metal::floor(f_4));
    uint i1_1 = metal::min(i0_1 + 1u, 63u);
    metal::float4 _e20 = u.spectral_lut.inner[metal::min(unsigned(i0_1), 63u)];
    metal::float4 _e25 = u.spectral_lut.inner[metal::min(unsigned(i1_1), 63u)];
    return metal::mix(_e20.xyz, _e25.xyz, f_4 - metal::floor(f_4));
}

bool folded(
    constant Uniforms& u
) {
    float _e3 = u.spectral.folded;
    return _e3 > 0.5;
}

float spectrum_color_level(
    uint b,
    constant Uniforms& u
) {
    uint i_7 = metal::min(b, 3827u);
    uint word_1 = u.spectrum_color.inner[metal::min(unsigned(naga_div(i_7, 16u)), 239u)][metal::min(unsigned(naga_mod(naga_div(i_7, 4u), 4u)), 3u)];
    return static_cast<float>((word_1 >> (naga_mod(i_7, 4u) * 8u)) & 255u) / 255.0;
}

float spectrum_color_at(
    float pitch_1,
    constant Uniforms& u
) {
    bool local_11 = {};
    float x_3 = ((pitch_1 - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE) - 0.5;
    if (!((x_3 < 0.0))) {
        local_11 = x_3 > 3827.0;
    } else {
        local_11 = true;
    }
    bool _e15 = local_11;
    if (_e15) {
        return 0.0;
    }
    uint i_8 = naga_f2u32(metal::floor(x_3));
    float _e19 = spectrum_color_level(i_8, u);
    float _e22 = spectrum_color_level(i_8 + 1u, u);
    return metal::mix(_e19, _e22, x_3 - metal::floor(x_3));
}

float wedge_fraction(
    metal::float2 edges_2,
    metal::float2 uv_5
) {
    float mid_2 = 0.5 * (edges_2.x + edges_2.y);
    float half_1 = metal::max(0.5 * (edges_2.x - edges_2.y), 0.00001);
    float c_3 = (uv_5.x * metal::cos(mid_2)) + (uv_5.y * metal::sin(mid_2));
    float s_9 = (-(uv_5.x) * metal::sin(mid_2)) + (uv_5.y * metal::cos(mid_2));
    float delta = metal::atan2(s_9, c_3);
    return metal::clamp(0.5 - (delta / (2.0 * half_1)), 0.0, 1.0);
}

RingInk spectral_ring(
    VsOut in_7,
    OctRing oct,
    metal::float2 uv_6,
    NodeLayer band_1,
    float aa_5,
    bool analytic,
    constant Uniforms& u
) {
    bool local_12 = {};
    bool local_13 = {};
    float cov = 0.0;
    int owner = {};
    float sd_1 = EMPTY_DISTANCE;
    uint i_1 = 0u;
    float pitch_2 = {};
    metal::float2 _e6 = spectral_radii(u);
    if (_e6.y <= _e6.x) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (in_7.ring <= 0.0) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (EARLY_OUT) {
        local_12 = !(analytic);
    } else {
        local_12 = false;
    }
    bool _e36 = local_12;
    if (_e36) {
        float _e39 = layer_coverage(band_1);
        local_13 = _e39 <= 0.0;
    } else {
        local_13 = false;
    }
    bool _e43 = local_13;
    if (_e43) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    owner = oct.base;
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init_1) {
            uint _e77 = i_1;
            i_1 = _e77 + 1u;
        }
        loop_init_1 = false;
        uint _e61 = i_1;
        uint _e62 = oct_span(u);
        if (_e61 < _e62) {
        } else {
            break;
        }
        {
            uint _e65 = i_1;
            int slot_3 = as_type<int>(as_type<uint>(oct.base) + as_type<uint>(static_cast<int>(_e65)));
            NodeLayer _e70 = outer_glyph(slot_3, oct, uv_6, band_1, _e6.x, _e6.y, aa_5, u);
            float _e71 = layer_coverage(_e70);
            float _e72 = sd_1;
            sd_1 = metal::min(_e72, _e70.sd);
            float _e75 = cov;
            if (_e71 > _e75) {
                cov = _e71;
                owner = slot_3;
            }
        }
    }
    float _e80 = cov;
    if (_e80 <= 0.0) {
        float _e85 = sd_1;
        float _e87 = cov;
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {_e85, in_7.ring, _e87}};
    }
    int _e92 = owner;
    float _e94 = oct_slot_pitch(_e92, in_7.cents);
    pitch_2 = _e94;
    bool _e96 = folded(u);
    if (!(_e96)) {
        int _e98 = owner;
        metal::float2 _e99 = oct_sector(_e98, oct, u);
        float _e100 = wedge_fraction(_e99, uv_6);
        float _e101 = pitch_2;
        float _e107 = u.spectral.range_cents;
        pitch_2 = _e101 + (((_e100 - 0.5) * _e107) / 100.0);
    }
    float _e112 = pitch_2;
    float _e113 = spectrum_color_at(_e112, u);
    metal::float3 _e114 = spectral_lut_color(_e113, u);
    float _e115 = cov;
    float _e121 = sd_1;
    float _e123 = cov;
    return RingInk {_e114, _e115 * in_7.ring, metal::clamp(_e113, 0.0, 1.0), NodeLayer {_e121, in_7.ring, _e123}};
}

NodeLayer mark_extension(
    uint slots,
    OctRing ring_5,
    metal::float2 uv_7,
    NodeLayer strip,
    float inner_5,
    float outer_4,
    float aa_6,
    bool analytic_1,
    constant Uniforms& u
) {
    bool local_14 = {};
    bool local_15 = {};
    float sd_2 = EMPTY_DISTANCE;
    float coverage = 0.0;
    uint i_2 = 0u;
    bool local_16 = {};
    bool local_17 = {};
    if (EARLY_OUT) {
        local_14 = !(analytic_1);
    } else {
        local_14 = false;
    }
    bool _e13 = local_14;
    if (_e13) {
        float _e16 = layer_coverage(strip);
        local_15 = _e16 <= 0.0;
    } else {
        local_15 = false;
    }
    bool _e20 = local_15;
    if (_e20) {
        return NodeLayer {EMPTY_DISTANCE, strip.level, 0.0};
    }
    uint _e26 = oct_span(u);
    int top = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_5.base) + as_type<uint>(static_cast<int>(_e26)))) - as_type<uint>(1));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e66 = i_2;
            i_2 = _e66 + 1u;
        }
        loop_init_2 = false;
        uint _e37 = i_2;
        if (_e37 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e40 = i_2;
            int s_10 = static_cast<int>(_e40);
            uint _e43 = i_2;
            if ((slots & (1u << _e43)) != 0u) {
                local_16 = s_10 >= ring_5.base;
            } else {
                local_16 = false;
            }
            bool _e53 = local_16;
            if (_e53) {
                local_17 = s_10 <= top;
            } else {
                local_17 = false;
            }
            bool _e58 = local_17;
            if (_e58) {
                NodeLayer _e59 = outer_glyph(s_10, ring_5, uv_7, strip, inner_5, outer_4, aa_6, u);
                float _e60 = sd_2;
                sd_2 = metal::min(_e60, _e59.sd);
                float _e63 = coverage;
                coverage = metal::max(_e63, _e59.coverage);
            }
        }
    }
    float _e69 = sd_2;
    float _e71 = coverage;
    return NodeLayer {_e69, strip.level, _e71};
}

NodeGeom node_geom(
    VsOut src,
    bool analytic_2,
    constant Uniforms& u
) {
    bool local_18 = {};
    bool local_19 = {};
    bool local_20 = {};
    bool local_21 = {};
    bool local_22 = {};
    bool local_23 = {};
    bool local_24 = {};
    bool local_25 = {};
    bool local_26 = {};
    bool local_27 = {};
    float d_6 = metal::length(src.uv);
    float _e6 = metal::fwidth(src.uv.x);
    float _e9 = aa_width(_e6, src.shadow_at.w, u);
    if (EARLY_OUT) {
        local_18 = !(analytic_2);
    } else {
        local_18 = false;
    }
    bool _e15 = local_18;
    if (_e15) {
        float _e18 = paint_reach(src, _e9, u);
        local_19 = d_6 > _e18;
    } else {
        local_19 = false;
    }
    bool _e21 = local_19;
    if (_e21) {
        return NodeGeom {d_6, _e9, OctRing {0, 0.0}, false};
    }
    metal::float2 _e27 = spectral_radii(u);
    bool ring_draws = _e27.y > _e27.x;
    if (ring_draws) {
        local_20 = metal::length(src.uv) >= (_e27.x - _e9);
    } else {
        local_20 = false;
    }
    bool _e39 = local_20;
    if (_e39) {
        local_21 = metal::length(src.uv) <= (_e27.y + _e9);
    } else {
        local_21 = false;
    }
    bool in_audio_ring = local_21;
    if (EARLY_OUT) {
        local_22 = !(analytic_2);
    } else {
        local_22 = false;
    }
    bool _e54 = local_22;
    if (_e54) {
        local_23 = !(in_audio_ring);
    } else {
        local_23 = false;
    }
    bool _e59 = local_23;
    if (_e59) {
        local_24 = src.params.x <= 0.0;
    } else {
        local_24 = false;
    }
    bool _e67 = local_24;
    if (_e67) {
        local_25 = src.params.y <= 0.0;
    } else {
        local_25 = false;
    }
    bool _e75 = local_25;
    if (_e75) {
        local_26 = src.params.z <= 0.0;
    } else {
        local_26 = false;
    }
    bool _e83 = local_26;
    if (_e83) {
        local_27 = ((src.octaves.x | src.octaves.y) | src.octaves.z) == 0u;
    } else {
        local_27 = false;
    }
    bool _e97 = local_27;
    if (_e97) {
        return NodeGeom {d_6, _e9, OctRing {0, 0.0}, false};
    }
    OctRing _e104 = oct_ring(src.cents, u);
    return NodeGeom {d_6, _e9, _e104, true};
}

float mask_level(
    float level_4
) {
    return metal::clamp(level_4 / INK_FLOOR, 0.0, 1.0);
}

NodeInk base_node_ink(
    VsOut in_8,
    float d_3,
    float aa_7,
    OctRing oct_1,
    bool analytic_3,
    constant Uniforms& u
) {
    float base_alpha = 0.0;
    metal::float3 base_rgb = metal::float3(0.0);
    float glyph = 0.0;
    metal::float3 glyph_rgb = {};
    float glyph_mask = 0.0;
    float glyph_lit = 0.0;
    float node_sd = EMPTY_DISTANCE;
    NodeLayer band_2 = NodeLayer {65504.0, 0.0, 0.0};
    uint i_3 = 0u;
    bool local_28 = {};
    bool local_29 = {};
    bool local_30 = {};
    bool local_31 = {};
    metal::float3 slot_rgb = {};
    float cov_1 = {};
    float lit_shape = {};
    NodeLayer mark_strip = NodeLayer {65504.0, 0.0, 0.0};
    float mark = {};
    float mark_mask = {};
    float presence_1 = in_8.params.x;
    metal::float4 _e16 = u.lattice_ground;
    glyph_rgb = _e16.xyz;
    float band_in = u.node.band_inner;
    float band_out = u.node.band_outer;
    float mark_thick = u.node.mark_thickness;
    float mark_w = metal::max(mark_thick, 0.0);
    float _e43 = u.node.mark_inner;
    float mark_in = metal::min(_e43, 1.58);
    float mark_out = metal::min(mark_in + mark_w, 1.58);
    if (band_out > band_in) {
        NodeLayer _e54 = glyph_band(d_3, band_in, band_out, 1.0, aa_7);
        band_2 = _e54;
    }
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_3 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_3) {
            uint _e140 = i_3;
            i_3 = _e140 + 1u;
        }
        loop_init_3 = false;
        uint _e57 = i_3;
        uint _e58 = oct_span(u);
        if (_e57 < _e58) {
            if (true) {
                local_29 = analytic_3;
            } else {
                local_29 = true;
            }
            bool _e66 = local_29;
            if (!(_e66)) {
                NodeLayer _e70 = band_2;
                float _e71 = layer_coverage(_e70);
                local_30 = _e71 > 0.0;
            } else {
                local_30 = true;
            }
            bool _e75 = local_30;
            local_28 = _e75;
        } else {
            local_28 = false;
        }
        bool _e77 = local_28;
        if (_e77) {
        } else {
            break;
        }
        {
            uint _e79 = i_3;
            int slot_4 = as_type<int>(as_type<uint>(oct_1.base) + as_type<uint>(static_cast<int>(_e79)));
            float _e83 = oct_slot_level(in_8.octaves, slot_4);
            if (_e83 <= 0.0) {
                local_31 = presence_1 <= 0.0;
            } else {
                local_31 = false;
            }
            bool _e91 = local_31;
            if (_e91) {
                continue;
            }
            NodeLayer _e93 = band_2;
            NodeLayer _e94 = outer_glyph(slot_4, oct_1, in_8.uv, _e93, band_in, band_out, aa_7, u);
            float _e95 = layer_coverage(_e94);
            float _e96 = glyph_mask;
            glyph_mask = metal::max(_e96, _e94.coverage);
            metal::float4 _e99 = oct_slot_ink(in_8, slot_4, u);
            float opacity_1 = _e99.w;
            slot_rgb = _e99.xyz;
            cov_1 = _e95 * opacity_1;
            lit_shape = _e95;
            float _e106 = slice_reach(in_8, slot_4, band_in, band_out);
            if (_e106 >= band_out) {
                float _e108 = node_sd;
                float _e112 = layer_distance(_e108, NodeLayer {_e94.sd, opacity_1, _e94.coverage}, in_8);
                node_sd = _e112;
            } else {
                SliceZones _e114 = slice_zones(in_8, slot_4, oct_1, in_8.uv, d_3, _e94, _e99, band_in, _e106, aa_7, u);
                slot_rgb = _e114.ink.xyz;
                cov_1 = _e114.ink.w;
                float _e120 = layer_coverage(_e114.lit);
                lit_shape = _e120;
                float _e121 = node_sd;
                float _e127 = layer_distance(_e121, NodeLayer {_e114.lit.sd, opacity_1, _e114.lit.coverage}, in_8);
                node_sd = _e127;
                float _e128 = node_sd;
                float _e132 = layer_distance(_e128, NodeLayer {_e94.sd, presence_1, _e94.coverage}, in_8);
                node_sd = _e132;
            }
            float _e133 = cov_1;
            float _e134 = glyph;
            if (_e133 > _e134) {
                float _e136 = cov_1;
                glyph = _e136;
                metal::float3 _e137 = slot_rgb;
                glyph_rgb = _e137;
                float _e138 = lit_shape;
                glyph_lit = _e138 * _e83;
            }
        }
    }
    float glyph_taper = 1.0 - metal::smoothstep(1.0, GLYPH_FADE_LIMIT, d_3);
    float _e148 = glyph;
    glyph = _e148 * glyph_taper;
    float _e150 = glyph_lit;
    glyph_lit = _e150 * glyph_taper;
    float _e152 = glyph_mask;
    glyph_mask = _e152 * glyph_taper;
    {
        metal::float2 _e154 = spectral_radii(u);
        NodeLayer _e159 = glyph_band(d_3, _e154.x, _e154.y, 1.0, aa_7);
        RingInk _e160 = spectral_ring(in_8, oct_1, in_8.uv, _e159, aa_7, analytic_3, u);
        float _e161 = node_sd;
        float _e163 = layer_distance(_e161, _e160.layer, in_8);
        node_sd = _e163;
        metal::float3 _e167 = glyph_rgb;
        float _e168 = glyph;
        float _e176 = glyph;
        glyph_rgb = ((_e160.color * _e160.cov) + ((_e167 * _e168) * (1.0 - _e160.cov))) / metal::float3(metal::max(_e160.cov + (_e176 * (1.0 - _e160.cov)), 0.0001));
        float _e189 = glyph_lit;
        glyph_lit = (_e160.lit * _e160.cov) + (_e189 * (1.0 - _e160.cov));
        float _e196 = glyph;
        glyph = _e160.cov + (_e196 * (1.0 - _e160.cov));
        float _e205 = mask_level(in_8.ring);
        float audio_mask = _e160.layer.coverage * _e205;
        float _e207 = glyph_mask;
        glyph_mask = audio_mask + (_e207 * (1.0 - audio_mask));
    }
    if (mark_out > mark_in) {
        NodeLayer _e219 = glyph_band(d_3, mark_in, mark_out, 1.0, aa_7);
        mark_strip = _e219;
    }
    float _e224 = mark_strip.sd;
    float _e228 = mark_strip.coverage;
    NodeLayer _e230 = mark_extension(in_8.marks.x, oct_1, in_8.uv, NodeLayer {_e224, in_8.params.y, _e228}, mark_in, mark_out, aa_7, analytic_3, u);
    float _e235 = mark_strip.sd;
    float _e239 = mark_strip.coverage;
    NodeLayer _e241 = mark_extension(in_8.marks.y, oct_1, in_8.uv, NodeLayer {_e235, in_8.params.z, _e239}, mark_in, mark_out, aa_7, analytic_3, u);
    float _e242 = layer_coverage(_e230);
    float _e243 = layer_coverage(_e241);
    float _e246 = mask_level(_e230.level);
    float melody_mask = _e230.coverage * _e246;
    float _e250 = mask_level(_e241.level);
    float bass_mask = _e241.coverage * _e250;
    float _e252 = node_sd;
    float _e253 = layer_distance(_e252, _e230, in_8);
    node_sd = _e253;
    float _e254 = node_sd;
    float _e255 = layer_distance(_e254, _e241, in_8);
    node_sd = _e255;
    mark = metal::max(_e242, _e243);
    mark_mask = metal::max(melody_mask, bass_mask);
    metal::float3 mark_rgb = (_e242 > _e243) ? in_8.melody_color.xyz : in_8.bass_color.xyz;
    float mark_taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_3);
    float _e271 = mark;
    mark = _e271 * mark_taper;
    float _e273 = mark_mask;
    mark_mask = _e273 * mark_taper;
    float _e275 = mark;
    metal::float3 _e277 = glyph_rgb;
    float _e278 = glyph;
    float _e280 = mark;
    float _e285 = mark;
    float _e286 = glyph;
    float _e287 = mark;
    glyph_rgb = ((mark_rgb * _e275) + ((_e277 * _e278) * (1.0 - _e280))) / metal::float3(metal::max(_e285 + (_e286 * (1.0 - _e287)), 0.0001));
    float _e296 = mark;
    float _e297 = glyph_lit;
    float _e298 = mark;
    glyph_lit = _e296 + (_e297 * (1.0 - _e298));
    float _e303 = mark;
    float _e304 = glyph;
    float _e305 = mark;
    glyph = _e303 + (_e304 * (1.0 - _e305));
    float _e310 = mark_mask;
    float _e311 = glyph_mask;
    float _e312 = mark_mask;
    glyph_mask = _e310 + (_e311 * (1.0 - _e312));
    float _e317 = glyph;
    float _e318 = base_alpha;
    float _e319 = glyph;
    float active_alpha = _e317 + (_e318 * (1.0 - _e319));
    metal::float3 _e324 = glyph_rgb;
    float _e325 = glyph;
    metal::float3 _e327 = base_rgb;
    float _e328 = glyph;
    metal::float3 active_rgb = (_e324 * _e325) + (_e327 * (1.0 - _e328));
    float _e333 = glyph_lit;
    float _e337 = glyph_mask;
    float _e338 = node_sd;
    return NodeInk {active_rgb, active_alpha, _e333 / metal::max(active_alpha, 0.0001), _e337, _e338};
}

float slice_progress(
    VsOut in_9,
    uint i_4
) {
    return static_cast<float>((in_9.motion[metal::min(unsigned(naga_div(i_4, 3u)), 3u)] >> (naga_mod(i_4, 3u) * 10u)) & 1023u) / 1023.0;
}

AnimatedInk animated_slice_ink(
    VsOut in_10,
    float aa_8,
    OctRing oct_2,
    constant Uniforms& u
) {
    NodeInk result = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, 65504.0};
    NodeInk marks = {};
    uint i_5 = 0u;
    float coverage_1 = {};
    metal::float3 rgb = {};
    float lit_1 = {};
    bool local_32 = {};
    bool local_33 = {};
    NodeInk _e11 = result;
    marks = _e11;
    float band_in_1 = u.node.band_inner;
    float band_out_1 = u.node.band_outer;
    float _e24 = u.node.mark_inner;
    float mark_in_1 = metal::min(_e24, 1.58);
    float _e30 = u.node.mark_thickness;
    float mark_out_1 = metal::min(mark_in_1 + metal::max(_e30, 0.0), 1.58);
    float anchor_radius = (band_out_1 > band_in_1) ? (0.5 * (band_in_1 + band_out_1)) : (0.5 * (mark_in_1 + mark_out_1));
    uint2 loop_bound_4 = uint2(4294967295u);
    bool loop_init_4 = true;
    while(true) {
        if (metal::all(loop_bound_4 == uint2(0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        if (!loop_init_4) {
            uint _e263 = i_5;
            i_5 = _e263 + 1u;
        }
        loop_init_4 = false;
        uint _e46 = i_5;
        uint _e47 = oct_span(u);
        if (_e46 < _e47) {
        } else {
            break;
        }
        {
            uint _e50 = i_5;
            int slot_5 = as_type<int>(as_type<uint>(oct_2.base) + as_type<uint>(static_cast<int>(_e50)));
            uint _e53 = i_5;
            float _e54 = slice_progress(in_10, _e53);
            if (_e54 <= 0.0) {
                continue;
            }
            float _e61 = u.node.pose.x;
            float scale = metal::mix(_e61, 1.0, _e54);
            if (_e54 < INK_FLOOR) {
                continue;
            }
            if (scale <= 0.001) {
                continue;
            }
            float _e68 = oct_mid(slot_5, oct_2, u);
            metal::float2 anchor = anchor_radius * metal::float2(metal::cos(_e68), metal::sin(_e68));
            float _e77 = u.node.pose.y;
            metal::float2 start = anchor * (1.0 + (_e77 * (1.0 - _e54)));
            metal::float2 uv_8 = anchor + ((in_10.uv - start) / metal::float2(scale));
            float d_7 = metal::length(uv_8);
            float soft = aa_8 / scale;
            if (band_out_1 > band_in_1) {
                NodeLayer _e93 = glyph_band(d_7, band_in_1, band_out_1, 1.0, soft);
                NodeLayer _e94 = outer_glyph(slot_5, oct_2, uv_8, _e93, band_in_1, band_out_1, soft, u);
                metal::float4 _e95 = oct_slot_ink(in_10, slot_5, u);
                float taper = 1.0 - metal::smoothstep(1.0, GLYPH_FADE_LIMIT, d_7);
                float _e102 = oct_slot_level(in_10.octaves, slot_5);
                coverage_1 = ((_e94.coverage * taper) * _e95.w) * _e54;
                rgb = _e95.xyz;
                lit_1 = _e102 / metal::max(_e95.w, 0.0001);
                float _e116 = slice_reach(in_10, slot_5, band_in_1, band_out_1);
                if (_e116 >= band_out_1) {
                    float _e120 = result.sd;
                    float _e127 = layer_distance(_e120, NodeLayer {_e94.sd * scale, _e95.w * _e54, _e94.coverage}, in_10);
                    result.sd = _e127;
                } else {
                    SliceZones _e128 = slice_zones(in_10, slot_5, oct_2, uv_8, d_7, _e94, _e95, band_in_1, _e116, soft, u);
                    rgb = _e128.ink.xyz;
                    coverage_1 = (_e128.ink.w * taper) * _e54;
                    lit_1 = (_e102 * _e128.lit.coverage) / metal::max(_e128.ink.w, 0.0001);
                    float _e145 = result.sd;
                    float _e154 = layer_distance(_e145, NodeLayer {_e128.lit.sd * scale, _e95.w * _e54, _e128.lit.coverage}, in_10);
                    result.sd = _e154;
                    float _e157 = result.sd;
                    float _e165 = layer_distance(_e157, NodeLayer {_e94.sd * scale, in_10.params.x * _e54, _e94.coverage}, in_10);
                    result.sd = _e165;
                }
                float _e166 = coverage_1;
                float _e168 = result.alpha;
                if (_e166 > _e168) {
                    metal::float3 _e171 = rgb;
                    float _e172 = coverage_1;
                    result.rgb = _e171 * _e172;
                    float _e175 = coverage_1;
                    result.alpha = _e175;
                    float _e177 = lit_1;
                    result.lit = _e177;
                }
                float _e180 = result.mask;
                float _e185 = mask_level(_e95.w * _e54);
                result.mask = metal::max(_e180, (_e94.coverage * taper) * _e185);
            }
            if (slot_5 >= 0) {
                local_32 = slot_5 < 11;
            } else {
                local_32 = false;
            }
            bool _e195 = local_32;
            if (_e195) {
                local_33 = mark_out_1 > mark_in_1;
            } else {
                local_33 = false;
            }
            bool _e200 = local_33;
            if (_e200) {
                uint bit = 1u << static_cast<uint>(slot_5);
                float melody = ((in_10.marks.x & bit) != 0u) ? in_10.params.y : 0.0;
                float bass = ((in_10.marks.y & bit) != 0u) ? in_10.params.z : 0.0;
                float level_7 = metal::max(melody, bass) * _e54;
                metal::float3 color = (melody > bass) ? in_10.melody_color.xyz : in_10.bass_color.xyz;
                NodeLayer _e230 = glyph_band(d_7, mark_in_1, mark_out_1, level_7, soft);
                NodeLayer _e231 = outer_glyph(slot_5, oct_2, uv_8, _e230, mark_in_1, mark_out_1, soft, u);
                float taper_1 = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_7);
                float _e237 = layer_coverage(_e231);
                float coverage_4 = _e237 * taper_1;
                float _e240 = marks.alpha;
                if (coverage_4 > _e240) {
                    marks.rgb = color * coverage_4;
                    marks.alpha = coverage_4;
                    marks.lit = 1.0;
                }
                float _e249 = marks.mask;
                float _e252 = mask_level(level_7);
                marks.mask = metal::max(_e249, (_e231.coverage * taper_1) * _e252);
                float _e257 = marks.sd;
                float _e262 = layer_distance(_e257, NodeLayer {_e231.sd * scale, level_7, _e231.coverage}, in_10);
                marks.sd = _e262;
            }
        }
    }
    NodeInk _e266 = result;
    NodeInk _e267 = marks;
    return AnimatedInk {_e266, _e267};
}

NodeInk node_ink(
    VsOut src_1,
    float d_4,
    float aa_9,
    OctRing oct_3,
    bool analytic_4,
    constant Uniforms& u
) {
    bool local_34 = {};
    bool local_35 = {};
    NodeInk ink_2 = {};
    float _e8 = u.node.animation;
    if (_e8 == 0.0) {
        NodeInk _e11 = base_node_ink(src_1, d_4, aa_9, oct_3, analytic_4, u);
        return _e11;
    }
    bool settled = (src_1.motion.w & 2147483648u) != 0u;
    float _e21 = u.node.band_outer;
    float _e25 = u.node.band_inner;
    if (_e21 <= _e25) {
        float _e32 = u.node.mark_thickness;
        local_34 = _e32 <= 0.0;
    } else {
        local_34 = false;
    }
    bool only_audio = local_34;
    if (!(settled)) {
        local_35 = only_audio;
    } else {
        local_35 = true;
    }
    bool _e41 = local_35;
    if (_e41) {
        NodeInk _e42 = base_node_ink(src_1, d_4, aa_9, oct_3, analytic_4, u);
        return _e42;
    }
    AnimatedInk _e43 = animated_slice_ink(src_1, aa_9, oct_3, u);
    ink_2 = _e43.body;
    metal::float2 _e46 = spectral_radii(u);
    NodeLayer _e53 = glyph_band(metal::length(src_1.uv), _e46.x, _e46.y, 1.0, aa_9);
    RingInk _e54 = spectral_ring(src_1, oct_3, src_1.uv, _e53, aa_9, analytic_4, u);
    float _e59 = ink_2.lit;
    float _e61 = ink_2.alpha;
    float lit_2 = (_e54.lit * _e54.cov) + ((_e59 * _e61) * (1.0 - _e54.cov));
    metal::float3 _e73 = ink_2.rgb;
    ink_2.rgb = (_e54.color * _e54.cov) + (_e73 * (1.0 - _e54.cov));
    float _e82 = ink_2.alpha;
    ink_2.alpha = _e54.cov + (_e82 * (1.0 - _e54.cov));
    float _e90 = ink_2.alpha;
    ink_2.lit = lit_2 / metal::max(_e90, 0.0001);
    float _e97 = mask_level(src_1.ring);
    float mask = _e54.layer.coverage * _e97;
    float _e101 = ink_2.mask;
    ink_2.mask = mask + (_e101 * (1.0 - mask));
    float _e108 = ink_2.sd;
    float _e110 = layer_distance(_e108, _e54.layer, src_1);
    ink_2.sd = _e110;
    float _e114 = ink_2.lit;
    float _e116 = ink_2.alpha;
    float mark_lit = _e43.marks.alpha + ((_e114 * _e116) * (1.0 - _e43.marks.alpha));
    metal::float3 _e128 = ink_2.rgb;
    ink_2.rgb = _e43.marks.rgb + (_e128 * (1.0 - _e43.marks.alpha));
    float _e139 = ink_2.alpha;
    ink_2.alpha = _e43.marks.alpha + (_e139 * (1.0 - _e43.marks.alpha));
    float _e148 = ink_2.alpha;
    ink_2.lit = mark_lit / metal::max(_e148, 0.0001);
    float _e156 = ink_2.mask;
    ink_2.mask = _e43.marks.mask + (_e156 * (1.0 - _e43.marks.mask));
    float _e165 = ink_2.sd;
    ink_2.sd = metal::min(_e165, _e43.marks.sd);
    NodeInk _e169 = ink_2;
    return _e169;
}

metal::float2 light_coord(
    metal::float2 frag_pos,
    constant Uniforms& u
) {
    metal::float2 _e4 = u.nebula.target_size;
    return frag_pos / metal::max(_e4, metal::float2(1.0));
}

Painted node_paint(
    VsOut in_11,
    metal::texture2d<float, metal::access::sample> glow_tex,
    metal::sampler glow_sampler,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_6 const& shadow_casters,
    device type_8 const& node_occluders,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    NodeInk ink_3 = {};
    float visibility_1 = 1.0;
    NodeGeom _e2 = node_geom(in_11, false, u);
    ShadowThrough _e9 = node_shadow_through(in_11.shadow_box.x, in_11.shadow_at.xy, in_11.shadow_at.z, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    if (!(_e2.paints)) {
        float shadow = 1.0 - _e9.seen;
        float bloom = 1.0 - _e9.bloom;
        if (bloom <= 0.0) {
            metal::discard_fragment();
        }
        return Painted {metal::float3(0.0), shadow, bloom, 0.0};
    }
    NodeInk _e28 = node_ink(in_11, _e2.d, _e2.aa, _e2.oct, false, u);
    ink_3 = _e28;
    float _e31 = ink_3.alpha;
    if (_e31 < INK_FLOOR) {
        float _e37 = ink_3.sd;
        ink_3 = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, _e37};
    }
    float _e43 = ink_3.mask;
    bool _e48 = shadow_is_distance(in_11.shadow_box.x, shadow_casters, _buffer_sizes);
    float shadow_exposure = _e48 ? 1.0 : (1.0 - _e43);
    float seen_through = 1.0 - ((1.0 - _e9.seen) * shadow_exposure);
    float bloom_through = 1.0 - ((1.0 - _e9.bloom) * shadow_exposure);
    float _e66 = ink_3.alpha;
    if (_e66 > 0.0) {
        float _e76 = u.geometry_shadow.occlusion;
        float _e77 = node_visibility(in_11.shadow_box.x, in_11.shadow_at.xy, _e76, shadow_atlas, shadow_sampler, shadow_casters, node_occluders, _buffer_sizes);
        visibility_1 = _e77;
    }
    float _e79 = ink_3.alpha;
    float _e80 = visibility_1;
    float visible_alpha = _e79 * _e80;
    float final_alpha = 1.0 - ((1.0 - visible_alpha) * seen_through);
    float bloom_alpha = 1.0 - ((1.0 - visible_alpha) * bloom_through);
    if (bloom_alpha <= 0.0) {
        metal::discard_fragment();
    }
    metal::float2 _e96 = light_coord(in_11.clip_pos.xy, u);
    metal::float4 _e97 = glow_light(_e96, glow_tex, glow_sampler);
    metal::float3 _e99 = ink_3.rgb;
    float _e101 = ink_3.alpha;
    float _e103 = glow_wash(u);
    float _e105 = ink_3.lit;
    metal::float3 _e108 = wash_over(_e99, _e101, _e97.xyz, metal::mix(1.0, _e103, _e105));
    float _e109 = visibility_1;
    return Painted {_e108 * _e109, final_alpha, bloom_alpha, visible_alpha};
}

SplitOut node_split(
    Painted paint,
    float shadow_alpha,
    constant Uniforms& u
) {
    float _e6 = u.geometry_shadow.occlusion;
    float alpha_1 = metal::mix(shadow_alpha, paint.ink_alpha, metal::clamp(_e6, 0.0, 1.0));
    return SplitOut {metal::float4(0.0, 0.0, 0.0, shadow_alpha), metal::float4(paint.rgb, alpha_1)};
}

struct fs_main_sceneInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 params [[user(loc2), center_perspective]];
    metal::uint3 octaves [[user(loc3), flat]];
    metal::uint3 thickness [[user(loc1), flat]];
    metal::uint4 motion [[user(loc9), flat]];
    float cents [[user(loc4), flat]];
    float strip_row [[user(loc5), flat]];
    metal::uint2 marks [[user(loc6), flat]];
    metal::float4 melody_color [[user(loc7), flat]];
    metal::float4 bass_color [[user(loc8), flat]];
    float rim [[user(loc11), flat]];
    float ring [[user(loc14), flat]];
    float ink_carry [[user(loc15), flat]];
    metal::float4 shadow_box [[user(loc10), flat]];
    metal::float4 shadow_at [[user(loc12), center_no_perspective]];
};
struct fs_main_sceneOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
    metal::float4 bloom_other [[color(2)]];
    metal::float4 bloom_ink [[color(3)]];
};
fragment fs_main_sceneOutput fs_main_scene(
  fs_main_sceneInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, metal::texture2d<float, metal::access::sample> glow_tex [[texture(0)]]
, metal::sampler glow_sampler [[sampler(0)]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(1)]]
, metal::sampler shadow_sampler [[sampler(1)]]
, device type_6 const& shadow_casters [[buffer(1)]]
, device type_8 const& node_occluders [[buffer(2)]]
, constant Uniforms& u [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.params, varyings.octaves, varyings.thickness, varyings.motion, varyings.cents, varyings.strip_row, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.ring, varyings.ink_carry, {}, varyings.shadow_box, varyings.shadow_at };
    Painted _e1 = node_paint(in, glow_tex, glow_sampler, shadow_atlas, shadow_sampler, shadow_casters, node_occluders, u, _buffer_sizes);
    SplitOut _e3 = node_split(_e1, _e1.seen, u);
    SplitOut _e5 = node_split(_e1, _e1.bloom, u);
    const auto _tmp = SceneOut {_e3.other, _e3.ink, _e5.other, _e5.ink * (1.0 - in.params.w)};
    return fs_main_sceneOutput { _tmp.other, _tmp.ink, _tmp.bloom_other, _tmp.bloom_ink };
}
