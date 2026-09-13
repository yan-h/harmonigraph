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
typedef ShadowCaster type_7[1];
struct SplitOut {
    metal::float4 other;
    metal::float4 ink;
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
    float padding;
};
struct MarkerParams {
    float half_width;
    float taper_start;
    float world_unit;
    float padding;
};
struct type_12 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_12 bounds;
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
struct type_13 {
    metal::float4 inner[64];
};
struct type_15 {
    metal::uint4 inner[240];
};
struct Uniforms {
    CompositeParams composite;
    CameraParams camera;
    NodeParams node;
    MarkerParams marker;
    OctaveParams octave;
    SpectralParams spectral;
    GlowParams glow;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    type_13 pitch_lut;
    type_13 spectral_lut;
    type_15 spectrum_color;
};
struct ShadowThrough {
    float seen;
    float bloom;
};
struct VsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float3 params;
    metal::packed_uint3 octaves;
    float cents;
    float strip_row;
    char _pad7[4];
    metal::uint2 marks;
    metal::float4 melody_color;
    metal::float4 bass_color;
    float rim;
    float ring;
    float ink_carry;
    char _pad13[4];
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
struct Painted {
    metal::packed_float3 rgb;
    float seen;
    float bloom;
    float ink_alpha;
    char _pad4[8];
};
constant float DISTANCE_KIND = 1.0;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_FALLOFF_FLOOR = 0.35;
constant float SHADOW_TAIL = 4.0;
constant float SHADOW_FALLOFF_FREE = 0.64025325;
constant float SHADOW_STOP = 2.0;
constant float SHADOW_INVISIBLE_FOLDS = 1.5586027;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float GLOW_BASE = 0.8;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant uint GLOW_TILE_SIZE = 32u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float AA_SOFTNESS_PX = 2.0;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float GLOW_LOBE_KAPPA = 4.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

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
    metal::float3 w_2 = light * share;
    return (w_2 * alpha) + (ink * (metal::float3(1.0) - w_2));
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
    float u_1 = metal::max(d, 0.0) / metal::max(w, 0.000001);
    float f_3 = metal::max(falloff_1, SHADOW_FALLOFF_FLOOR);
    t = u_1;
    if (f_3 != 1.0) {
        t = metal::pow(u_1, f_3);
    }
    float _e15 = t;
    float _e18 = shadow_stop(f_3);
    return metal::exp(-4.0 * _e15) * (1.0 - metal::smoothstep(1.0, _e18, u_1));
}

float shadow_kernel(
    uint who,
    metal::float2 points,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_7 const& shadow_casters,
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
    metal::float2 atlas = static_cast<metal::float2>(metal::uint2(shadow_atlas.get_width(), shadow_atlas.get_height()));
    metal::float4 map = shadow_casters[metal::min(unsigned(who), (_buffer_sizes.size3 - 0 - 64) / 64)].map;
    metal::float2 texel = metal::clamp(map.xy + (points * map.z), cell_1.xy + metal::float2(0.5), (cell_1.xy + cell_1.zw) - metal::float2(0.5));
    metal::float4 _e39 = shadow_atlas.sample(shadow_sampler, texel / atlas, metal::level(0.0));
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
    device type_7 const& shadow_casters,
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
            float level_5 = metal::clamp(caster.shade.x, 0.0, 1.0);
            hidden = level_5 * _e61;
            if (caster.shade.y < 0.5) {
                float _e74 = shadow_transmittance(_e61, 1.0, level_5);
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
    device type_7 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    if (level_1 <= 0.0) {
        return ShadowThrough {1.0, 1.0};
    }
    float _e14 = shadow_kernel(naga_f2u32(metal::max(who_2, 0.0)), points_2, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
    float coverage_1 = metal::clamp(level_1, 0.0, 1.0) * _e14;
    float _e16 = glow_shadow_depth(u);
    return ShadowThrough {1.0 - (_e16 * coverage_1), 1.0 - coverage_1};
}

bool shadow_is_distance(
    float who_3,
    device type_7 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint caster_1 = naga_f2u32(metal::max(who_3, 0.0));
    if (caster_1 >= (1 + (_buffer_sizes.size3 - 0 - 64) / 64)) {
        return false;
    }
    float _e12 = shadow_casters[metal::min(unsigned(caster_1), (_buffer_sizes.size3 - 0 - 64) / 64)].shade.y;
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
    bool local_3 = {};
    if (!((in_1.marks.x != 0u))) {
        local_3 = in_1.marks.y != 0u;
    } else {
        local_3 = true;
    }
    bool _e16 = local_3;
    if (_e16) {
        float _e17 = reach;
        reach = metal::max(_e17, QUAD_MARGIN);
    }
    float _e20 = reach;
    metal::float2 _e22 = spectral_radii(u);
    return metal::max(_e20, metal::max(in_1.rim, _e22.y) + aa);
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
    float d_3 = ((nearest * 12.0) + off) - _e15;
    low = naga_div(naga_neg(as_type<int>(as_type<uint>(span) - as_type<uint>(1))), 2);
    if (naga_mod(span, 2) == 0) {
        low = (d_3 < 0.0) ? as_type<int>(as_type<uint>(1) - as_type<uint>(naga_div(span, 2))) : naga_div(naga_neg(span), 2);
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
    uint i_4 = static_cast<uint>(metal::clamp(as_type<int>(as_type<uint>(s_1) - as_type<uint>(ring_1.base)), 0, as_type<int>(as_type<uint>(static_cast<int>(_e4)) - as_type<uint>(1))));
    float _e12 = oct_bound(i_4, u);
    float _e17 = oct_bound(i_4 + 1u, u);
    return metal::float2(ring_1.seam - _e12, ring_1.seam - _e17);
}

float oct_slot_level(
    metal::uint3 octaves_1,
    int s_2
) {
    bool local_4 = {};
    if (!((s_2 < 0))) {
        local_4 = s_2 >= 11;
    } else {
        local_4 = true;
    }
    bool _e10 = local_4;
    if (_e10) {
        return 0.0;
    }
    float _e13 = octave_level(octaves_1, static_cast<uint>(s_2));
    return _e13;
}

float oct_arc_coverage(
    metal::float2 edges,
    metal::float2 uv,
    float aa_1
) {
    metal::float2 b1_ = metal::float2(metal::cos(edges.x), metal::sin(edges.x));
    metal::float2 b2_ = metal::float2(metal::cos(edges.y), metal::sin(edges.y));
    float c1_ = (uv.x * b1_.y) - (uv.y * b1_.x);
    float c2_ = (uv.x * b2_.y) - (uv.y * b2_.x);
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
    NodeLayer layer_1
) {
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

metal::float3 pitch_lut_color(
    float pitch,
    constant Uniforms& u
) {
    float _e4 = u.composite.darkest_pitch;
    float _e9 = u.composite.brightest_pitch;
    float _e13 = u.composite.darkest_pitch;
    float t_1 = metal::clamp((pitch - _e4) / metal::max(_e9 - _e13, 0.01), 0.0, 1.0);
    float f_4 = t_1 * 63.0;
    uint i0_ = naga_f2u32(metal::floor(f_4));
    uint i1_ = metal::min(i0_ + 1u, 63u);
    metal::float4 _e32 = u.pitch_lut.inner[metal::min(unsigned(i0_), 63u)];
    metal::float4 _e37 = u.pitch_lut.inner[metal::min(unsigned(i1_), 63u)];
    return metal::mix(_e32.xyz, _e37.xyz, f_4 - metal::floor(f_4));
}

metal::float4 oct_slot_lit(
    VsOut in_2,
    int slot,
    constant Uniforms& u
) {
    float _e3 = oct_slot_pitch(slot, in_2.cents);
    metal::float3 _e4 = pitch_lut_color(_e3, u);
    float _e6 = oct_slot_level(in_2.octaves, slot);
    return metal::float4(_e4, _e6);
}

metal::float4 oct_slot_ink(
    VsOut in_3,
    int slot_1,
    constant Uniforms& u
) {
    float presence = in_3.params.x;
    metal::float4 _e4 = oct_slot_lit(in_3, slot_1, u);
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

SectorFold sector_fold(
    metal::float2 uv_1,
    metal::float2 edges_1
) {
    float mid_1 = 0.5 * (edges_1.x + edges_1.y);
    float half_ = metal::clamp(0.5 * (edges_1.x - edges_1.y), 0.0, 3.1415927);
    float c_1 = metal::cos(mid_1);
    float s_4 = metal::sin(mid_1);
    return SectorFold {metal::float2(metal::abs((uv_1.y * c_1) - (uv_1.x * s_4)), (uv_1.x * c_1) + (uv_1.y * s_4)), metal::float2(metal::sin(half_), metal::cos(half_))};
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
    float inner_1,
    float outer_1,
    float gap
) {
    float distance = {};
    bool local_5 = {};
    bool local_6 = {};
    if (outer_1 <= inner_1) {
        return EMPTY_DISTANCE;
    }
    metal::float2 normal = metal::float2(f_2.e.y, -(f_2.e.x));
    float radius = metal::length(f_2.q);
    float axis_t = (gap * f_2.e.y) / metal::max(f_2.e.x, 0.000001);
    float inner_t = metal::sqrt(metal::max((inner_1 * inner_1) - (gap * gap), 0.0));
    float outer_t = metal::sqrt(metal::max((outer_1 * outer_1) - (gap * gap), 0.0));
    float segment_start = metal::max(axis_t, inner_t);
    if (segment_start > outer_t) {
        return EMPTY_DISTANCE;
    }
    float side_t = metal::clamp(metal::dot(f_2.q, f_2.e), segment_start, outer_t);
    distance = metal::length(f_2.q - ((side_t * f_2.e) - (gap * normal)));
    metal::float2 direction = f_2.q / metal::float2(metal::max(radius, 0.000001));
    metal::float2 outer_at = direction * outer_1;
    if ((metal::dot(normal, outer_at) + gap) <= 0.0) {
        float _e59 = distance;
        distance = metal::min(_e59, metal::abs(radius - outer_1));
    }
    metal::float2 inner_at = direction * inner_1;
    if ((metal::dot(normal, inner_at) + gap) <= 0.0) {
        float _e68 = distance;
        distance = metal::min(_e68, metal::abs(radius - inner_1));
    }
    if (radius >= inner_1) {
        local_5 = radius <= outer_1;
    } else {
        local_5 = false;
    }
    bool _e77 = local_5;
    if (_e77) {
        local_6 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_6 = false;
    }
    bool inside = local_6;
    float _e87 = distance;
    float _e88 = distance;
    return inside ? -(_e88) : _e87;
}

NodeLayer outer_glyph(
    int s_3,
    OctRing ring_2,
    metal::float2 uv_2,
    NodeLayer band,
    float inner_2,
    float outer_2,
    float aa_3,
    constant Uniforms& u
) {
    float sd = {};
    metal::float2 _e7 = oct_sector(s_3, ring_2, u);
    SectorFold _e8 = sector_fold(uv_2, _e7);
    float _e9 = slice_gap_half(u);
    float gap_1 = (metal::dot(_e8.q, _e8.e) > 0.0) ? _e9 : 0.0;
    float _e17 = sector_side(_e8);
    float side = _e17 + gap_1;
    float _e19 = sector_pie(_e8, outer_2);
    float width = _e7.x - _e7.y;
    if (width > 3.1415927) {
        sd = metal::max(metal::max(band.sd, _e19), side);
    } else {
        float _e29 = slice_gap_half(u);
        float _e30 = annular_sector_distance(_e8, inner_2, outer_2, _e29);
        sd = _e30;
    }
    metal::float2 b1_1 = metal::float2(metal::cos(_e7.x), metal::sin(_e7.x));
    metal::float2 b2_1 = metal::float2(metal::cos(_e7.y), metal::sin(_e7.y));
    float c1_1 = (uv_2.x * b1_1.y) - (uv_2.y * b1_1.x);
    float c2_1 = (uv_2.x * b2_1.y) - (uv_2.y * b2_1.x);
    float _e55 = oct_arc_coverage(_e7, uv_2, aa_3);
    float _e56 = slice_gap_half(u);
    float _e58 = aa_inside(_e56, metal::abs(c1_1), aa_3);
    float _e66 = aa_inside(_e56, metal::abs(c2_1), aa_3);
    float gaps = (1.0 - (_e58 * metal::smoothstep(-(aa_3), aa_3, metal::dot(uv_2, b1_1)))) * (1.0 - (_e66 * metal::smoothstep(-(aa_3), aa_3, metal::dot(uv_2, b2_1))));
    float _e74 = sd;
    return NodeLayer {_e74, band.level, (band.coverage * _e55) * gaps};
}

metal::float3 spectral_lut_color(
    float level_3,
    constant Uniforms& u
) {
    float f_5 = metal::clamp(level_3, 0.0, 1.0) * 63.0;
    uint i0_1 = naga_f2u32(metal::floor(f_5));
    uint i1_1 = metal::min(i0_1 + 1u, 63u);
    metal::float4 _e15 = u.spectral_lut.inner[metal::min(unsigned(i0_1), 63u)];
    metal::float4 _e20 = u.spectral_lut.inner[metal::min(unsigned(i1_1), 63u)];
    return metal::mix(_e15.xyz, _e20.xyz, f_5 - metal::floor(f_5));
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
    uint i_5 = metal::min(b, 3827u);
    uint word_1 = u.spectrum_color.inner[metal::min(unsigned(naga_div(i_5, 16u)), 239u)][metal::min(unsigned(naga_mod(naga_div(i_5, 4u), 4u)), 3u)];
    return static_cast<float>((word_1 >> (naga_mod(i_5, 4u) * 8u)) & 255u) / 255.0;
}

float spectrum_color_at(
    float pitch_1,
    constant Uniforms& u
) {
    bool local_7 = {};
    float x_2 = ((pitch_1 - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE) - 0.5;
    if (!((x_2 < 0.0))) {
        local_7 = x_2 > 3827.0;
    } else {
        local_7 = true;
    }
    bool _e15 = local_7;
    if (_e15) {
        return 0.0;
    }
    uint i_6 = naga_f2u32(metal::floor(x_2));
    float _e19 = spectrum_color_level(i_6, u);
    float _e22 = spectrum_color_level(i_6 + 1u, u);
    return metal::mix(_e19, _e22, x_2 - metal::floor(x_2));
}

float wedge_fraction(
    metal::float2 edges_2,
    metal::float2 uv_3
) {
    float mid_2 = 0.5 * (edges_2.x + edges_2.y);
    float half_1 = metal::max(0.5 * (edges_2.x - edges_2.y), 0.00001);
    float c_2 = (uv_3.x * metal::cos(mid_2)) + (uv_3.y * metal::sin(mid_2));
    float s_5 = (-(uv_3.x) * metal::sin(mid_2)) + (uv_3.y * metal::cos(mid_2));
    float delta = metal::atan2(s_5, c_2);
    return metal::clamp(0.5 - (delta / (2.0 * half_1)), 0.0, 1.0);
}

RingInk spectral_ring(
    VsOut in_4,
    OctRing oct,
    metal::float2 uv_4,
    NodeLayer band_1,
    float aa_4,
    bool analytic,
    constant Uniforms& u
) {
    bool local_8 = {};
    bool local_9 = {};
    float cov = 0.0;
    int owner = {};
    float sd_1 = EMPTY_DISTANCE;
    uint i_1 = 0u;
    float pitch_2 = {};
    metal::float2 _e6 = spectral_radii(u);
    if (_e6.y <= _e6.x) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (in_4.ring <= 0.0) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (EARLY_OUT) {
        local_8 = !(analytic);
    } else {
        local_8 = false;
    }
    bool _e36 = local_8;
    if (_e36) {
        float _e39 = layer_coverage(band_1);
        local_9 = _e39 <= 0.0;
    } else {
        local_9 = false;
    }
    bool _e43 = local_9;
    if (_e43) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    owner = oct.base;
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
        if (!loop_init) {
            uint _e77 = i_1;
            i_1 = _e77 + 1u;
        }
        loop_init = false;
        uint _e61 = i_1;
        uint _e62 = oct_span(u);
        if (_e61 < _e62) {
        } else {
            break;
        }
        {
            uint _e65 = i_1;
            int slot_2 = as_type<int>(as_type<uint>(oct.base) + as_type<uint>(static_cast<int>(_e65)));
            NodeLayer _e70 = outer_glyph(slot_2, oct, uv_4, band_1, _e6.x, _e6.y, aa_4, u);
            float _e71 = layer_coverage(_e70);
            float _e72 = sd_1;
            sd_1 = metal::min(_e72, _e70.sd);
            float _e75 = cov;
            if (_e71 > _e75) {
                cov = _e71;
                owner = slot_2;
            }
        }
    }
    float _e80 = cov;
    if (_e80 <= 0.0) {
        float _e85 = sd_1;
        float _e87 = cov;
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {_e85, in_4.ring, _e87}};
    }
    int _e92 = owner;
    float _e94 = oct_slot_pitch(_e92, in_4.cents);
    pitch_2 = _e94;
    bool _e96 = folded(u);
    if (!(_e96)) {
        int _e98 = owner;
        metal::float2 _e99 = oct_sector(_e98, oct, u);
        float _e100 = wedge_fraction(_e99, uv_4);
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
    return RingInk {_e114, _e115 * in_4.ring, metal::clamp(_e113, 0.0, 1.0), NodeLayer {_e121, in_4.ring, _e123}};
}

NodeLayer mark_extension(
    uint slots,
    OctRing ring_3,
    metal::float2 uv_5,
    NodeLayer strip,
    float inner_3,
    float outer_3,
    float aa_5,
    bool analytic_1,
    constant Uniforms& u
) {
    bool local_10 = {};
    bool local_11 = {};
    float sd_2 = EMPTY_DISTANCE;
    float coverage = 0.0;
    uint i_2 = 0u;
    bool local_12 = {};
    bool local_13 = {};
    if (EARLY_OUT) {
        local_10 = !(analytic_1);
    } else {
        local_10 = false;
    }
    bool _e13 = local_10;
    if (_e13) {
        float _e16 = layer_coverage(strip);
        local_11 = _e16 <= 0.0;
    } else {
        local_11 = false;
    }
    bool _e20 = local_11;
    if (_e20) {
        return NodeLayer {EMPTY_DISTANCE, strip.level, 0.0};
    }
    uint _e26 = oct_span(u);
    int top = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_3.base) + as_type<uint>(static_cast<int>(_e26)))) - as_type<uint>(1));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_1) {
            uint _e66 = i_2;
            i_2 = _e66 + 1u;
        }
        loop_init_1 = false;
        uint _e37 = i_2;
        if (_e37 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e40 = i_2;
            int s_6 = static_cast<int>(_e40);
            uint _e43 = i_2;
            if ((slots & (1u << _e43)) != 0u) {
                local_12 = s_6 >= ring_3.base;
            } else {
                local_12 = false;
            }
            bool _e53 = local_12;
            if (_e53) {
                local_13 = s_6 <= top;
            } else {
                local_13 = false;
            }
            bool _e58 = local_13;
            if (_e58) {
                NodeLayer _e59 = outer_glyph(s_6, ring_3, uv_5, strip, inner_3, outer_3, aa_5, u);
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
    VsOut in_5,
    bool analytic_2,
    constant Uniforms& u
) {
    bool local_14 = {};
    bool local_15 = {};
    bool local_16 = {};
    bool local_17 = {};
    bool local_18 = {};
    bool local_19 = {};
    bool local_20 = {};
    bool local_21 = {};
    bool local_22 = {};
    bool local_23 = {};
    float d_4 = metal::length(in_5.uv);
    float _e6 = metal::fwidth(in_5.uv.x);
    float _e9 = aa_width(_e6, in_5.shadow_at.w, u);
    if (EARLY_OUT) {
        local_14 = !(analytic_2);
    } else {
        local_14 = false;
    }
    bool _e15 = local_14;
    if (_e15) {
        float _e18 = paint_reach(in_5, _e9, u);
        local_15 = d_4 > _e18;
    } else {
        local_15 = false;
    }
    bool _e21 = local_15;
    if (_e21) {
        return NodeGeom {d_4, _e9, OctRing {0, 0.0}, false};
    }
    metal::float2 _e27 = spectral_radii(u);
    bool ring_draws = _e27.y > _e27.x;
    if (ring_draws) {
        local_16 = d_4 >= (_e27.x - _e9);
    } else {
        local_16 = false;
    }
    bool _e37 = local_16;
    if (_e37) {
        local_17 = d_4 <= (_e27.y + _e9);
    } else {
        local_17 = false;
    }
    bool in_audio_ring = local_17;
    if (EARLY_OUT) {
        local_18 = !(analytic_2);
    } else {
        local_18 = false;
    }
    bool _e50 = local_18;
    if (_e50) {
        local_19 = !(in_audio_ring);
    } else {
        local_19 = false;
    }
    bool _e55 = local_19;
    if (_e55) {
        local_20 = in_5.params.x <= 0.0;
    } else {
        local_20 = false;
    }
    bool _e63 = local_20;
    if (_e63) {
        local_21 = in_5.params.y <= 0.0;
    } else {
        local_21 = false;
    }
    bool _e71 = local_21;
    if (_e71) {
        local_22 = in_5.params.z <= 0.0;
    } else {
        local_22 = false;
    }
    bool _e79 = local_22;
    if (_e79) {
        local_23 = ((in_5.octaves[0] | in_5.octaves[1]) | in_5.octaves[2]) == 0u;
    } else {
        local_23 = false;
    }
    bool _e93 = local_23;
    if (_e93) {
        return NodeGeom {d_4, _e9, OctRing {0, 0.0}, false};
    }
    OctRing _e100 = oct_ring(in_5.cents, u);
    return NodeGeom {d_4, _e9, _e100, true};
}

float mask_level(
    float level_4
) {
    return metal::clamp(level_4 / INK_FLOOR, 0.0, 1.0);
}

NodeInk node_ink(
    VsOut in_6,
    float d_2,
    float aa_6,
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
    bool local_24 = {};
    bool local_25 = {};
    bool local_26 = {};
    bool local_27 = {};
    NodeLayer mark_strip = NodeLayer {65504.0, 0.0, 0.0};
    float mark = {};
    float mark_mask = {};
    float presence_1 = in_6.params.x;
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
        NodeLayer _e54 = glyph_band(d_2, band_in, band_out, 1.0, aa_6);
        band_2 = _e54;
    }
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e111 = i_3;
            i_3 = _e111 + 1u;
        }
        loop_init_2 = false;
        uint _e57 = i_3;
        uint _e58 = oct_span(u);
        if (_e57 < _e58) {
            if (true) {
                local_25 = analytic_3;
            } else {
                local_25 = true;
            }
            bool _e66 = local_25;
            if (!(_e66)) {
                NodeLayer _e70 = band_2;
                float _e71 = layer_coverage(_e70);
                local_26 = _e71 > 0.0;
            } else {
                local_26 = true;
            }
            bool _e75 = local_26;
            local_24 = _e75;
        } else {
            local_24 = false;
        }
        bool _e77 = local_24;
        if (_e77) {
        } else {
            break;
        }
        {
            uint _e79 = i_3;
            int slot_3 = as_type<int>(as_type<uint>(oct_1.base) + as_type<uint>(static_cast<int>(_e79)));
            float _e83 = oct_slot_level(in_6.octaves, slot_3);
            if (_e83 <= 0.0) {
                local_27 = presence_1 <= 0.0;
            } else {
                local_27 = false;
            }
            bool _e91 = local_27;
            if (_e91) {
                continue;
            }
            NodeLayer _e93 = band_2;
            NodeLayer _e94 = outer_glyph(slot_3, oct_1, in_6.uv, _e93, band_in, band_out, aa_6, u);
            float _e95 = layer_coverage(_e94);
            float _e96 = glyph_mask;
            glyph_mask = metal::max(_e96, _e94.coverage);
            metal::float4 _e99 = oct_slot_ink(in_6, slot_3, u);
            float opacity_1 = _e99.w;
            float _e101 = node_sd;
            float _e105 = layer_distance(_e101, NodeLayer {_e94.sd, opacity_1, _e94.coverage});
            node_sd = _e105;
            metal::float3 slot_rgb = _e99.xyz;
            float cov_1 = _e95 * opacity_1;
            float _e108 = glyph;
            if (cov_1 > _e108) {
                glyph = cov_1;
                glyph_rgb = slot_rgb;
                glyph_lit = _e95 * _e83;
            }
        }
    }
    float glyph_taper = 1.0 - metal::smoothstep(1.0, GLYPH_FADE_LIMIT, d_2);
    float _e119 = glyph;
    glyph = _e119 * glyph_taper;
    float _e121 = glyph_lit;
    glyph_lit = _e121 * glyph_taper;
    float _e123 = glyph_mask;
    glyph_mask = _e123 * glyph_taper;
    metal::float2 _e125 = spectral_radii(u);
    NodeLayer _e130 = glyph_band(d_2, _e125.x, _e125.y, 1.0, aa_6);
    RingInk _e131 = spectral_ring(in_6, oct_1, in_6.uv, _e130, aa_6, analytic_3, u);
    float _e132 = node_sd;
    float _e134 = layer_distance(_e132, _e131.layer);
    node_sd = _e134;
    metal::float3 _e138 = glyph_rgb;
    float _e139 = glyph;
    float _e147 = glyph;
    glyph_rgb = ((_e131.color * _e131.cov) + ((_e138 * _e139) * (1.0 - _e131.cov))) / metal::float3(metal::max(_e131.cov + (_e147 * (1.0 - _e131.cov)), 0.0001));
    float _e160 = glyph_lit;
    glyph_lit = (_e131.lit * _e131.cov) + (_e160 * (1.0 - _e131.cov));
    float _e167 = glyph;
    glyph = _e131.cov + (_e167 * (1.0 - _e131.cov));
    float _e176 = mask_level(in_6.ring);
    float audio_mask = _e131.layer.coverage * _e176;
    float _e178 = glyph_mask;
    glyph_mask = audio_mask + (_e178 * (1.0 - audio_mask));
    if (mark_out > mark_in) {
        NodeLayer _e190 = glyph_band(d_2, mark_in, mark_out, 1.0, aa_6);
        mark_strip = _e190;
    }
    float _e195 = mark_strip.sd;
    float _e199 = mark_strip.coverage;
    NodeLayer _e201 = mark_extension(in_6.marks.x, oct_1, in_6.uv, NodeLayer {_e195, in_6.params.y, _e199}, mark_in, mark_out, aa_6, analytic_3, u);
    float _e206 = mark_strip.sd;
    float _e210 = mark_strip.coverage;
    NodeLayer _e212 = mark_extension(in_6.marks.y, oct_1, in_6.uv, NodeLayer {_e206, in_6.params.z, _e210}, mark_in, mark_out, aa_6, analytic_3, u);
    float _e213 = layer_coverage(_e201);
    float _e214 = layer_coverage(_e212);
    float _e217 = mask_level(_e201.level);
    float melody_mask = _e201.coverage * _e217;
    float _e221 = mask_level(_e212.level);
    float bass_mask = _e212.coverage * _e221;
    float _e223 = node_sd;
    float _e224 = layer_distance(_e223, _e201);
    node_sd = _e224;
    float _e225 = node_sd;
    float _e226 = layer_distance(_e225, _e212);
    node_sd = _e226;
    mark = metal::max(_e213, _e214);
    mark_mask = metal::max(melody_mask, bass_mask);
    metal::float3 mark_rgb = (_e213 > _e214) ? in_6.melody_color.xyz : in_6.bass_color.xyz;
    float mark_taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_2);
    float _e242 = mark;
    mark = _e242 * mark_taper;
    float _e244 = mark_mask;
    mark_mask = _e244 * mark_taper;
    float _e246 = mark;
    metal::float3 _e248 = glyph_rgb;
    float _e249 = glyph;
    float _e251 = mark;
    float _e256 = mark;
    float _e257 = glyph;
    float _e258 = mark;
    glyph_rgb = ((mark_rgb * _e246) + ((_e248 * _e249) * (1.0 - _e251))) / metal::float3(metal::max(_e256 + (_e257 * (1.0 - _e258)), 0.0001));
    float _e267 = mark;
    float _e268 = glyph_lit;
    float _e269 = mark;
    glyph_lit = _e267 + (_e268 * (1.0 - _e269));
    float _e274 = mark;
    float _e275 = glyph;
    float _e276 = mark;
    glyph = _e274 + (_e275 * (1.0 - _e276));
    float _e281 = mark_mask;
    float _e282 = glyph_mask;
    float _e283 = mark_mask;
    glyph_mask = _e281 + (_e282 * (1.0 - _e283));
    float _e288 = glyph;
    float _e289 = base_alpha;
    float _e290 = glyph;
    float active_alpha = _e288 + (_e289 * (1.0 - _e290));
    metal::float3 _e295 = glyph_rgb;
    float _e296 = glyph;
    metal::float3 _e298 = base_rgb;
    float _e299 = glyph;
    metal::float3 active_rgb = (_e295 * _e296) + (_e298 * (1.0 - _e299));
    float _e304 = glyph_lit;
    float _e308 = glyph_mask;
    float _e309 = node_sd;
    return NodeInk {active_rgb, active_alpha, _e304 / metal::max(active_alpha, 0.0001), _e308, _e309};
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::int2 light_coord(
    metal::float2 frag_pos,
    metal::texture2d<float, metal::access::sample> glow_tex
) {
    metal::int2 edge_2 = as_type<metal::int2>(as_type<metal::uint2>(static_cast<metal::int2>(metal::uint2(glow_tex.get_width(), glow_tex.get_height()))) - as_type<metal::uint2>(metal::int2(1, 1)));
    return metal::min(naga_f2i32(frag_pos), edge_2);
}

Painted node_paint(
    VsOut in_7,
    metal::texture2d<float, metal::access::sample> glow_tex,
    metal::texture2d<float, metal::access::sample> shadow_atlas,
    metal::sampler shadow_sampler,
    device type_7 const& shadow_casters,
    constant Uniforms& u,
    constant _mslBufferSizes& _buffer_sizes
) {
    NodeInk ink_1 = {};
    float visibility_1 = 1.0;
    NodeGeom _e2 = node_geom(in_7, false, u);
    ShadowThrough _e9 = node_shadow_through(in_7.shadow_box.x, in_7.shadow_at.xy, in_7.shadow_at.z, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    if (!(_e2.paints)) {
        float shadow = 1.0 - _e9.seen;
        float bloom = 1.0 - _e9.bloom;
        if (bloom <= 0.0) {
            metal::discard_fragment();
        }
        return Painted {metal::float3(0.0), shadow, bloom, 0.0};
    }
    NodeInk _e28 = node_ink(in_7, _e2.d, _e2.aa, _e2.oct, false, u);
    ink_1 = _e28;
    float _e31 = ink_1.alpha;
    if (_e31 < INK_FLOOR) {
        float _e37 = ink_1.sd;
        ink_1 = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, _e37};
    }
    float _e43 = ink_1.mask;
    bool _e48 = shadow_is_distance(in_7.shadow_box.x, shadow_casters, _buffer_sizes);
    float shadow_exposure = _e48 ? 1.0 : (1.0 - _e43);
    float seen_through = 1.0 - ((1.0 - _e9.seen) * shadow_exposure);
    float bloom_through = 1.0 - ((1.0 - _e9.bloom) * shadow_exposure);
    float _e66 = ink_1.alpha;
    if (_e66 > 0.0) {
        float _e76 = u.geometry_shadow.occlusion;
        float _e77 = node_visibility(in_7.shadow_box.x, in_7.shadow_at.xy, _e76, shadow_atlas, shadow_sampler, shadow_casters, _buffer_sizes);
        visibility_1 = _e77;
    }
    float _e79 = ink_1.alpha;
    float _e80 = visibility_1;
    float visible_alpha = _e79 * _e80;
    float final_alpha = 1.0 - ((1.0 - visible_alpha) * seen_through);
    float bloom_alpha = 1.0 - ((1.0 - visible_alpha) * bloom_through);
    if (bloom_alpha <= 0.0) {
        metal::discard_fragment();
    }
    metal::int2 _e96 = light_coord(in_7.clip_pos.xy, glow_tex);
    metal::float4 _e97 = glow_light(_e96, glow_tex);
    metal::float3 _e99 = ink_1.rgb;
    float _e101 = ink_1.alpha;
    float _e103 = glow_wash(u);
    float _e105 = ink_1.lit;
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

struct fs_main_splitInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float3 params [[user(loc2), center_perspective]];
    metal::uint3 octaves [[user(loc3), flat]];
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
struct fs_main_splitOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
};
fragment fs_main_splitOutput fs_main_split(
  fs_main_splitInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, metal::texture2d<float, metal::access::sample> glow_tex [[texture(0)]]
, metal::texture2d<float, metal::access::sample> shadow_atlas [[texture(1)]]
, metal::sampler shadow_sampler [[sampler(1)]]
, device type_7 const& shadow_casters [[buffer(1)]]
, constant Uniforms& u [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.color, varyings.params, varyings.octaves, varyings.cents, varyings.strip_row, {}, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.ring, varyings.ink_carry, {}, varyings.shadow_box, varyings.shadow_at };
    Painted _e1 = node_paint(in, glow_tex, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    SplitOut _e3 = node_split(_e1, _e1.seen, u);
    const auto _tmp = _e3;
    return fs_main_splitOutput { _tmp.other, _tmp.ink };
}
