// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

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
struct type_7 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_7 bounds;
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
struct type_8 {
    metal::float4 inner[64];
};
struct type_10 {
    metal::uint4 inner[240];
};
struct type_11 {
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
    type_8 pitch_lut;
    type_8 spectral_lut;
    type_10 spectrum_color;
    type_11 ink_kernel;
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
    float swell;
    float ring;
    float ink_carry;
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
    float lit;
    NodeLayer near;
    NodeLayer far;
    float rest;
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
    bool local_1 = {};
    if (!((in_1.marks.x != 0u))) {
        local_1 = in_1.marks.y != 0u;
    } else {
        local_1 = true;
    }
    bool _e16 = local_1;
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

uint naga_div(uint lhs, uint rhs) {
    return lhs / metal::select(rhs, 1u, rhs == 0u);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

uint slot_byte(
    metal::uint3 words,
    uint i
) {
    return (words[metal::min(unsigned(naga_div(i, 4u)), 2u)] >> (naga_mod(i, 4u) * 8u)) & 255u;
}

float slice_thickness(
    metal::uint3 thickness,
    uint i_1
) {
    uint _e2 = slot_byte(thickness, i_1);
    return static_cast<float>(_e2) / THICKNESS_STEPS;
}

float swell_cap(
    float outer,
    constant Uniforms& u
) {
    float _e4 = u.node.mark_thickness;
    float mark_w = metal::max(_e4, 0.0);
    float _e10 = u.node.mark_inner;
    float room = (mark_w > 0.0) ? (metal::max(_e10 - outer, 0.0) + mark_w) : 0.0;
    return 1.58 - room;
}

float reach_at(
    float t,
    float inner,
    float outer_1,
    constant Uniforms& u
) {
    if (t == 1.0) {
        return outer_1;
    }
    float reach_2 = inner + ((outer_1 - inner) * t);
    if (t < 1.0) {
        return reach_2;
    }
    float _e10 = swell_cap(outer_1, u);
    return metal::max(outer_1, metal::min(reach_2, _e10));
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

float octave_level(
    metal::uint3 octaves,
    uint i_2
) {
    uint _e2 = slot_byte(octaves, i_2);
    return static_cast<float>(_e2) / 255.0;
}

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
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
    float d_7 = ((nearest * 12.0) + off) - _e15;
    low = naga_div(naga_neg(as_type<int>(as_type<uint>(span) - as_type<uint>(1))), 2);
    if (naga_mod(span, 2) == 0) {
        low = (d_7 < 0.0) ? as_type<int>(as_type<uint>(1) - as_type<uint>(naga_div(span, 2))) : naga_div(naga_neg(span), 2);
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
    uint i_10 = static_cast<uint>(metal::clamp(as_type<int>(as_type<uint>(s_1) - as_type<uint>(ring_1.base)), 0, as_type<int>(as_type<uint>(static_cast<int>(_e4)) - as_type<uint>(1))));
    float _e12 = oct_bound(i_10, u);
    float _e17 = oct_bound(i_10 + 1u, u);
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
    bool local_2 = {};
    if (!((s_3 < 0))) {
        local_2 = s_3 >= 11;
    } else {
        local_2 = true;
    }
    bool _e10 = local_2;
    if (_e10) {
        return 0.0;
    }
    float _e13 = octave_level(octaves_1, static_cast<uint>(s_3));
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
    float inner_1,
    float outer_2,
    float level,
    float aa_2
) {
    float mid = 0.5 * (inner_1 + outer_2);
    float _e14 = aa_inside(outer_2, d_1, aa_2);
    float _e15 = aa_inside(inner_1, d_1, aa_2);
    return NodeLayer {metal::abs(d_1 - mid) - (0.5 * (outer_2 - inner_1)), level, _e14 * (1.0 - _e15)};
}

float lut_position(
    float t_1,
    metal::float2 corner
) {
    float w_2 = {};
    float x_2 = corner.x;
    float y = corner.y;
    if (x_2 == y) {
        return t_1;
    }
    float low_1 = y / x_2;
    float high = (1.0 - y) / (1.0 - x_2);
    float x0_ = x_2 * 0.6;
    float x2_ = x_2 + (BEND_ROUNDING * (1.0 - x_2));
    if (t_1 <= x0_) {
        w_2 = low_1 * t_1;
    } else {
        if (t_1 >= x2_) {
            w_2 = y + (high * (t_1 - x_2));
        } else {
            float y0_ = low_1 * x0_;
            float y2_ = y + (high * (x2_ - x_2));
            float a = (x0_ - (2.0 * x_2)) + x2_;
            float b_1 = 2.0 * (x_2 - x0_);
            float c_1 = x0_ - t_1;
            float s_8 = (-2.0 * c_1) / (b_1 + metal::sqrt(metal::max((b_1 * b_1) - ((4.0 * a) * c_1), 0.0)));
            w_2 = ((((1.0 - s_8) * (1.0 - s_8)) * y0_) + (((2.0 * s_8) * (1.0 - s_8)) * y)) + ((s_8 * s_8) * y2_);
        }
    }
    float _e65 = w_2;
    return 0.5 * (t_1 + metal::clamp(_e65, 0.0, 1.0));
}

metal::float3 pitch_lut_color(
    float pitch,
    constant Uniforms& u
) {
    float _e4 = u.composite.darkest_pitch;
    float _e9 = u.composite.brightest_pitch;
    float _e13 = u.composite.darkest_pitch;
    float t_2 = metal::clamp((pitch - _e4) / metal::max(_e9 - _e13, 0.01), 0.0, 1.0);
    metal::float4 _e23 = u.lut_spacing;
    float _e25 = lut_position(t_2, _e23.xy);
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
    float level_3 = _e4.w;
    if (level_3 <= 0.0) {
        metal::float4 _e10 = u.lattice_ground;
        return metal::float4(_e10.xyz, presence);
    }
    float ghost_rest = metal::max(presence - level_3, 0.0);
    float opacity = level_3 + ghost_rest;
    metal::float4 _e19 = u.lattice_ground;
    metal::float3 ground = _e19.xyz;
    return metal::float4(((_e4.xyz * level_3) + (ground * ghost_rest)) / metal::float3(opacity), opacity);
}

float slice_reach(
    VsOut in_5,
    int s_4,
    float inner_2,
    float outer_3,
    constant Uniforms& u
) {
    bool local_3 = {};
    if (!((s_4 < 0))) {
        local_3 = s_4 >= 11;
    } else {
        local_3 = true;
    }
    bool _e12 = local_3;
    if (_e12) {
        return outer_3;
    }
    float _e15 = slice_thickness(in_5.thickness, static_cast<uint>(s_4));
    float _e16 = reach_at(_e15, inner_2, outer_3, u);
    return _e16;
}

metal::float2 mark_radii(
    VsOut in_6,
    int s_5,
    float inner_3,
    float outer_4,
    constant Uniforms& u
) {
    bool local_4 = {};
    float band_in = u.node.band_inner;
    float band_out = u.node.band_outer;
    float _e12 = slice_reach(in_6, s_5, band_in, band_out, u);
    if (!((band_out <= band_in))) {
        local_4 = _e12 == band_out;
    } else {
        local_4 = true;
    }
    bool _e19 = local_4;
    if (_e19) {
        return metal::float2(inner_3, outer_4);
    }
    float _e25 = u.node.mark_inner;
    float start = metal::min(_e25 + (_e12 - band_out), 1.58);
    float _e32 = u.node.mark_thickness;
    return metal::float2(start, metal::min(start + metal::max(_e32, 0.0), 1.58));
}

bool marks_move(
    VsOut in_7,
    uint slots,
    constant Uniforms& u
) {
    uint i_3 = 0u;
    bool local_5 = {};
    float _e5 = u.node.band_outer;
    float _e9 = u.node.band_inner;
    if (_e5 <= _e9) {
        return false;
    }
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e33 = i_3;
            i_3 = _e33 + 1u;
        }
        loop_init = false;
        uint _e14 = i_3;
        if (_e14 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e18 = i_3;
            if ((slots & (1u << _e18)) != 0u) {
                uint _e26 = i_3;
                float _e27 = slice_thickness(in_7.thickness, _e26);
                local_5 = _e27 != 1.0;
            } else {
                local_5 = false;
            }
            bool _e31 = local_5;
            if (_e31) {
                return true;
            }
        }
    }
    return false;
}

float glyph_taper(
    float d_2,
    bool swells
) {
    if (swells) {
        return 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_2);
    }
    return 1.0 - metal::smoothstep(1.0, GLYPH_FADE_LIMIT, d_2);
}

SectorFold sector_fold(
    metal::float2 uv_1,
    metal::float2 edges_1
) {
    float mid_1 = 0.5 * (edges_1.x + edges_1.y);
    float half_ = metal::clamp(0.5 * (edges_1.x - edges_1.y), 0.0, 3.1415927);
    float c_2 = metal::cos(mid_1);
    float s_9 = metal::sin(mid_1);
    return SectorFold {metal::float2(metal::abs((uv_1.y * c_2) - (uv_1.x * s_9)), (uv_1.x * c_2) + (uv_1.y * s_9)), metal::float2(metal::sin(half_), metal::cos(half_))};
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
    float inner_4,
    float outer_5,
    float gap
) {
    float distance = {};
    bool local_6 = {};
    bool local_7 = {};
    if (outer_5 <= inner_4) {
        return EMPTY_DISTANCE;
    }
    metal::float2 normal = metal::float2(f_2.e.y, -(f_2.e.x));
    float radius = metal::length(f_2.q);
    float axis_t = (gap * f_2.e.y) / metal::max(f_2.e.x, 0.000001);
    float inner_t = metal::sqrt(metal::max((inner_4 * inner_4) - (gap * gap), 0.0));
    float outer_t = metal::sqrt(metal::max((outer_5 * outer_5) - (gap * gap), 0.0));
    float segment_start = metal::max(axis_t, inner_t);
    if (segment_start > outer_t) {
        return EMPTY_DISTANCE;
    }
    float side_t = metal::clamp(metal::dot(f_2.q, f_2.e), segment_start, outer_t);
    distance = metal::length(f_2.q - ((side_t * f_2.e) - (gap * normal)));
    metal::float2 direction = f_2.q / metal::float2(metal::max(radius, 0.000001));
    metal::float2 outer_at = direction * outer_5;
    if ((metal::dot(normal, outer_at) + gap) <= 0.0) {
        float _e59 = distance;
        distance = metal::min(_e59, metal::abs(radius - outer_5));
    }
    metal::float2 inner_at = direction * inner_4;
    if ((metal::dot(normal, inner_at) + gap) <= 0.0) {
        float _e68 = distance;
        distance = metal::min(_e68, metal::abs(radius - inner_4));
    }
    if (radius >= inner_4) {
        local_6 = radius <= outer_5;
    } else {
        local_6 = false;
    }
    bool _e77 = local_6;
    if (_e77) {
        local_7 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_7 = false;
    }
    bool inside = local_7;
    float _e87 = distance;
    float _e88 = distance;
    return inside ? -(_e88) : _e87;
}

NodeLayer outer_glyph(
    int s_6,
    OctRing ring_3,
    metal::float2 uv_2,
    NodeLayer band,
    float inner_5,
    float outer_6,
    float aa_3,
    constant Uniforms& u
) {
    float sd = {};
    metal::float2 _e7 = oct_sector(s_6, ring_3, u);
    SectorFold _e8 = sector_fold(uv_2, _e7);
    float _e9 = slice_gap_half(u);
    float gap_1 = (metal::dot(_e8.q, _e8.e) > 0.0) ? _e9 : 0.0;
    float _e17 = sector_side(_e8);
    float side = _e17 + gap_1;
    float _e19 = sector_pie(_e8, outer_6);
    float width = _e7.x - _e7.y;
    if (width > 3.1415927) {
        sd = metal::max(metal::max(band.sd, _e19), side);
    } else {
        float _e29 = slice_gap_half(u);
        float _e30 = annular_sector_distance(_e8, inner_5, outer_6, _e29);
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

SliceZones slice_zones(
    VsOut in_8,
    int s_7,
    OctRing ring_4,
    metal::float2 uv_3,
    float d_3,
    NodeLayer band_1,
    metal::float4 ink,
    float inner_6,
    float outer_7,
    float reach_1,
    float aa_4,
    constant Uniforms& u
) {
    NodeLayer slice = NodeLayer {65504.0, 0.0, 0.0};
    NodeLayer near = {};
    NodeLayer far = {};
    metal::float3 far_rgb = {};
    float far_level = {};
    if (reach_1 > inner_6) {
        NodeLayer _e18 = glyph_band(d_3, inner_6, reach_1, 1.0, aa_4);
        NodeLayer _e19 = outer_glyph(s_7, ring_4, uv_3, _e18, inner_6, reach_1, aa_4, u);
        slice = _e19;
    }
    float _e21 = oct_slot_level(in_8.octaves, s_7);
    near = band_1;
    NodeLayer _e23 = slice;
    far = _e23;
    metal::float4 _e25 = oct_slot_lit(in_8, s_7, u);
    far_rgb = _e25.xyz;
    far_level = _e21;
    if (reach_1 < outer_7) {
        NodeLayer _e30 = slice;
        near = _e30;
        far = band_1;
        metal::float4 _e33 = u.lattice_ground;
        far_rgb = _e33.xyz;
        far_level = metal::max(in_8.params.x - _e21, 0.0);
    }
    NodeLayer _e40 = near;
    float _e41 = layer_coverage(_e40);
    NodeLayer _e42 = far;
    float _e43 = layer_coverage(_e42);
    float rest = metal::max(_e43 - _e41, 0.0);
    float near_ink = _e41 * ink.w;
    float _e49 = far_level;
    float far_ink = rest * _e49;
    float cov_2 = near_ink + far_ink;
    metal::float3 _e54 = far_rgb;
    metal::float3 rgb_1 = ((ink.xyz * near_ink) + (_e54 * far_ink)) / metal::float3(metal::max(cov_2, 0.0001));
    NodeLayer _e62 = slice;
    float _e63 = layer_coverage(_e62);
    float _e65 = near.sd;
    float _e68 = near.coverage;
    float _e71 = far.sd;
    float _e72 = far_level;
    float _e74 = far.coverage;
    return SliceZones {metal::float4(rgb_1, cov_2), _e63, NodeLayer {_e65, ink.w, _e68}, NodeLayer {_e71, _e72, _e74}, rest};
}

metal::float3 spectral_lut_color(
    float level_1,
    constant Uniforms& u
) {
    metal::float4 _e6 = u.lut_spacing;
    float _e8 = lut_position(metal::clamp(level_1, 0.0, 1.0), _e6.zw);
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
    uint i_11 = metal::min(b, 3827u);
    uint word = u.spectrum_color.inner[metal::min(unsigned(naga_div(i_11, 16u)), 239u)][metal::min(unsigned(naga_mod(naga_div(i_11, 4u), 4u)), 3u)];
    return static_cast<float>((word >> (naga_mod(i_11, 4u) * 8u)) & 255u) / 255.0;
}

float spectrum_color_at(
    float pitch_1,
    constant Uniforms& u
) {
    bool local_8 = {};
    float x_3 = ((pitch_1 - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE) - 0.5;
    if (!((x_3 < 0.0))) {
        local_8 = x_3 > 3827.0;
    } else {
        local_8 = true;
    }
    bool _e15 = local_8;
    if (_e15) {
        return 0.0;
    }
    uint i_12 = naga_f2u32(metal::floor(x_3));
    float _e19 = spectrum_color_level(i_12, u);
    float _e22 = spectrum_color_level(i_12 + 1u, u);
    return metal::mix(_e19, _e22, x_3 - metal::floor(x_3));
}

float wedge_fraction(
    metal::float2 edges_2,
    metal::float2 uv_4
) {
    float mid_2 = 0.5 * (edges_2.x + edges_2.y);
    float half_1 = metal::max(0.5 * (edges_2.x - edges_2.y), 0.00001);
    float c_3 = (uv_4.x * metal::cos(mid_2)) + (uv_4.y * metal::sin(mid_2));
    float s_10 = (-(uv_4.x) * metal::sin(mid_2)) + (uv_4.y * metal::cos(mid_2));
    float delta = metal::atan2(s_10, c_3);
    return metal::clamp(0.5 - (delta / (2.0 * half_1)), 0.0, 1.0);
}

RingInk spectral_ring(
    VsOut in_9,
    OctRing oct,
    metal::float2 uv_5,
    NodeLayer band_2,
    float aa_5,
    bool analytic,
    constant Uniforms& u
) {
    bool local_9 = {};
    bool local_10 = {};
    float cov = 0.0;
    int owner = {};
    float sd_1 = EMPTY_DISTANCE;
    uint i_4 = 0u;
    float pitch_2 = {};
    metal::float2 _e6 = spectral_radii(u);
    if (_e6.y <= _e6.x) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (in_9.ring <= 0.0) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (EARLY_OUT) {
        local_9 = !(analytic);
    } else {
        local_9 = false;
    }
    bool _e36 = local_9;
    if (_e36) {
        float _e39 = layer_coverage(band_2);
        local_10 = _e39 <= 0.0;
    } else {
        local_10 = false;
    }
    bool _e43 = local_10;
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
            uint _e77 = i_4;
            i_4 = _e77 + 1u;
        }
        loop_init_1 = false;
        uint _e61 = i_4;
        uint _e62 = oct_span(u);
        if (_e61 < _e62) {
        } else {
            break;
        }
        {
            uint _e65 = i_4;
            int slot_2 = as_type<int>(as_type<uint>(oct.base) + as_type<uint>(static_cast<int>(_e65)));
            NodeLayer _e70 = outer_glyph(slot_2, oct, uv_5, band_2, _e6.x, _e6.y, aa_5, u);
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
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {_e85, in_9.ring, _e87}};
    }
    int _e92 = owner;
    float _e94 = oct_slot_pitch(_e92, in_9.cents);
    pitch_2 = _e94;
    bool _e96 = folded(u);
    if (!(_e96)) {
        int _e98 = owner;
        metal::float2 _e99 = oct_sector(_e98, oct, u);
        float _e100 = wedge_fraction(_e99, uv_5);
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
    return RingInk {_e114, _e115 * in_9.ring, metal::clamp(_e113, 0.0, 1.0), NodeLayer {_e121, in_9.ring, _e123}};
}

NodeLayer mark_extension(
    uint slots_1,
    OctRing ring_5,
    metal::float2 uv_6,
    NodeLayer strip,
    float inner_7,
    float outer_8,
    float aa_6,
    bool analytic_1,
    constant Uniforms& u
) {
    bool local_11 = {};
    bool local_12 = {};
    float sd_2 = EMPTY_DISTANCE;
    float coverage = 0.0;
    uint i_5 = 0u;
    bool local_13 = {};
    bool local_14 = {};
    if (EARLY_OUT) {
        local_11 = !(analytic_1);
    } else {
        local_11 = false;
    }
    bool _e13 = local_11;
    if (_e13) {
        float _e16 = layer_coverage(strip);
        local_12 = _e16 <= 0.0;
    } else {
        local_12 = false;
    }
    bool _e20 = local_12;
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
            uint _e66 = i_5;
            i_5 = _e66 + 1u;
        }
        loop_init_2 = false;
        uint _e37 = i_5;
        if (_e37 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e40 = i_5;
            int s_11 = static_cast<int>(_e40);
            uint _e43 = i_5;
            if ((slots_1 & (1u << _e43)) != 0u) {
                local_13 = s_11 >= ring_5.base;
            } else {
                local_13 = false;
            }
            bool _e53 = local_13;
            if (_e53) {
                local_14 = s_11 <= top;
            } else {
                local_14 = false;
            }
            bool _e58 = local_14;
            if (_e58) {
                NodeLayer _e59 = outer_glyph(s_11, ring_5, uv_6, strip, inner_7, outer_8, aa_6, u);
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

NodeLayer drawn_marks(
    VsOut in_10,
    uint slots_2,
    OctRing ring_6,
    metal::float2 uv_7,
    float d_4,
    NodeLayer strip_1,
    float inner_8,
    float outer_9,
    float aa_7,
    bool analytic_2,
    constant Uniforms& u
) {
    float sd_3 = EMPTY_DISTANCE;
    float coverage_1 = 0.0;
    uint i_6 = 0u;
    bool local_15 = {};
    bool local_16 = {};
    bool local_17 = {};
    bool local_18 = {};
    bool local_19 = {};
    bool local_20 = {};
    bool _e10 = marks_move(in_10, slots_2, u);
    if (!(_e10)) {
        NodeLayer _e12 = mark_extension(slots_2, ring_6, uv_7, strip_1, inner_8, outer_9, aa_7, analytic_2, u);
        return _e12;
    }
    uint _e14 = oct_span(u);
    int top_1 = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_6.base) + as_type<uint>(static_cast<int>(_e14)))) - as_type<uint>(1));
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_3 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_3) {
            uint _e92 = i_6;
            i_6 = _e92 + 1u;
        }
        loop_init_3 = false;
        uint _e25 = i_6;
        if (_e25 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e28 = i_6;
            int s_12 = static_cast<int>(_e28);
            uint _e31 = i_6;
            if (!(((slots_2 & (1u << _e31)) == 0u))) {
                local_15 = s_12 < ring_6.base;
            } else {
                local_15 = true;
            }
            bool _e42 = local_15;
            if (!(_e42)) {
                local_16 = s_12 > top_1;
            } else {
                local_16 = true;
            }
            bool _e48 = local_16;
            if (_e48) {
                continue;
            }
            metal::float2 _e49 = mark_radii(in_10, s_12, inner_8, outer_9, u);
            if (!((_e49.y <= _e49.x))) {
                if (EARLY_OUT) {
                    local_18 = !(analytic_2);
                } else {
                    local_18 = false;
                }
                bool _e61 = local_18;
                if (_e61) {
                    if (!((d_4 <= (_e49.x - aa_7)))) {
                        local_20 = d_4 >= (_e49.y + aa_7);
                    } else {
                        local_20 = true;
                    }
                    bool _e74 = local_20;
                    local_19 = _e74;
                } else {
                    local_19 = false;
                }
                bool _e76 = local_19;
                local_17 = _e76;
            } else {
                local_17 = true;
            }
            bool _e78 = local_17;
            if (_e78) {
                continue;
            }
            NodeLayer _e82 = glyph_band(d_4, _e49.x, _e49.y, 1.0, aa_7);
            NodeLayer _e85 = outer_glyph(s_12, ring_6, uv_7, _e82, _e49.x, _e49.y, aa_7, u);
            float _e86 = sd_3;
            sd_3 = metal::min(_e86, _e85.sd);
            float _e89 = coverage_1;
            coverage_1 = metal::max(_e89, _e85.coverage);
        }
    }
    float _e95 = sd_3;
    float _e97 = coverage_1;
    return NodeLayer {_e95, strip_1.level, _e97};
}

NodeGeom node_geom(
    VsOut src,
    bool analytic_3,
    constant Uniforms& u
) {
    bool local_21 = {};
    bool local_22 = {};
    bool local_23 = {};
    bool local_24 = {};
    bool local_25 = {};
    bool local_26 = {};
    bool local_27 = {};
    bool local_28 = {};
    bool local_29 = {};
    bool local_30 = {};
    float d_8 = metal::length(src.uv);
    float _e6 = metal::fwidth(src.uv.x);
    float _e9 = aa_width(_e6, src.shadow_at.w, u);
    if (EARLY_OUT) {
        local_21 = !(analytic_3);
    } else {
        local_21 = false;
    }
    bool _e15 = local_21;
    if (_e15) {
        float _e18 = paint_reach(src, _e9, u);
        local_22 = d_8 > _e18;
    } else {
        local_22 = false;
    }
    bool _e21 = local_22;
    if (_e21) {
        return NodeGeom {d_8, _e9, OctRing {0, 0.0}, false};
    }
    metal::float2 _e27 = spectral_radii(u);
    bool ring_draws = _e27.y > _e27.x;
    if (ring_draws) {
        local_23 = metal::length(src.uv) >= (_e27.x - _e9);
    } else {
        local_23 = false;
    }
    bool _e39 = local_23;
    if (_e39) {
        local_24 = metal::length(src.uv) <= (_e27.y + _e9);
    } else {
        local_24 = false;
    }
    bool in_audio_ring = local_24;
    if (EARLY_OUT) {
        local_25 = !(analytic_3);
    } else {
        local_25 = false;
    }
    bool _e54 = local_25;
    if (_e54) {
        local_26 = !(in_audio_ring);
    } else {
        local_26 = false;
    }
    bool _e59 = local_26;
    if (_e59) {
        local_27 = src.params.x <= 0.0;
    } else {
        local_27 = false;
    }
    bool _e67 = local_27;
    if (_e67) {
        local_28 = src.params.y <= 0.0;
    } else {
        local_28 = false;
    }
    bool _e75 = local_28;
    if (_e75) {
        local_29 = src.params.z <= 0.0;
    } else {
        local_29 = false;
    }
    bool _e83 = local_29;
    if (_e83) {
        local_30 = ((src.octaves.x | src.octaves.y) | src.octaves.z) == 0u;
    } else {
        local_30 = false;
    }
    bool _e97 = local_30;
    if (_e97) {
        return NodeGeom {d_8, _e9, OctRing {0, 0.0}, false};
    }
    OctRing _e104 = oct_ring(src.cents, u);
    return NodeGeom {d_8, _e9, _e104, true};
}

float mask_level(
    float level_2
) {
    return metal::clamp(level_2 / INK_FLOOR, 0.0, 1.0);
}

NodeInk base_node_ink(
    VsOut in_11,
    float d_5,
    float aa_8,
    OctRing oct_1,
    bool analytic_4,
    constant Uniforms& u
) {
    float base_alpha = 0.0;
    metal::float3 base_rgb = metal::float3(0.0);
    float glyph = 0.0;
    metal::float3 glyph_rgb = {};
    float glyph_mask = 0.0;
    float glyph_lit = 0.0;
    float node_sd = EMPTY_DISTANCE;
    NodeLayer band_3 = NodeLayer {65504.0, 0.0, 0.0};
    bool local_31 = {};
    uint i_7 = 0u;
    bool local_32 = {};
    bool local_33 = {};
    bool local_34 = {};
    bool local_35 = {};
    bool local_36 = {};
    metal::float3 slot_rgb = {};
    float cov_1 = {};
    float lit_shape = {};
    NodeLayer mark_strip = NodeLayer {65504.0, 0.0, 0.0};
    float mark = {};
    float mark_mask = {};
    float presence_1 = in_11.params.x;
    metal::float4 _e16 = u.lattice_ground;
    glyph_rgb = _e16.xyz;
    float band_in_1 = u.node.band_inner;
    float band_out_1 = u.node.band_outer;
    float mark_thick = u.node.mark_thickness;
    float mark_w_1 = metal::max(mark_thick, 0.0);
    float _e43 = u.node.mark_inner;
    float mark_in = metal::min(_e43, 1.58);
    float mark_out = metal::min(mark_in + mark_w_1, 1.58);
    if (band_out_1 > band_in_1) {
        NodeLayer _e54 = glyph_band(d_5, band_in_1, band_out_1, 1.0, aa_8);
        band_3 = _e54;
    }
    float swell_out = in_11.swell;
    bool swells_1 = swell_out > band_out_1;
    if (swells_1) {
        local_31 = d_5 < (swell_out + aa_8);
    } else {
        local_31 = false;
    }
    bool in_swell = local_31;
    uint2 loop_bound_4 = uint2(4294967295u);
    bool loop_init_4 = true;
    while(true) {
        if (metal::all(loop_bound_4 == uint2(0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        if (!loop_init_4) {
            uint _e156 = i_7;
            i_7 = _e156 + 1u;
        }
        loop_init_4 = false;
        uint _e65 = i_7;
        uint _e66 = oct_span(u);
        if (_e65 < _e66) {
            if (true) {
                local_33 = analytic_4;
            } else {
                local_33 = true;
            }
            bool _e74 = local_33;
            if (!(_e74)) {
                NodeLayer _e78 = band_3;
                float _e79 = layer_coverage(_e78);
                local_34 = _e79 > 0.0;
            } else {
                local_34 = true;
            }
            bool _e83 = local_34;
            if (!(_e83)) {
                local_35 = in_swell;
            } else {
                local_35 = true;
            }
            bool _e88 = local_35;
            local_32 = _e88;
        } else {
            local_32 = false;
        }
        bool _e90 = local_32;
        if (_e90) {
        } else {
            break;
        }
        {
            uint _e92 = i_7;
            int slot_3 = as_type<int>(as_type<uint>(oct_1.base) + as_type<uint>(static_cast<int>(_e92)));
            float _e96 = oct_slot_level(in_11.octaves, slot_3);
            if (_e96 <= 0.0) {
                local_36 = presence_1 <= 0.0;
            } else {
                local_36 = false;
            }
            bool _e104 = local_36;
            if (_e104) {
                continue;
            }
            NodeLayer _e106 = band_3;
            NodeLayer _e107 = outer_glyph(slot_3, oct_1, in_11.uv, _e106, band_in_1, band_out_1, aa_8, u);
            float _e108 = layer_coverage(_e107);
            metal::float4 _e109 = oct_slot_ink(in_11, slot_3, u);
            float opacity_1 = _e109.w;
            slot_rgb = _e109.xyz;
            cov_1 = _e108 * opacity_1;
            lit_shape = _e108;
            float _e116 = slice_reach(in_11, slot_3, band_in_1, band_out_1, u);
            if (_e116 == band_out_1) {
                float _e118 = glyph_mask;
                glyph_mask = metal::max(_e118, _e107.coverage);
                float _e121 = node_sd;
                float _e125 = layer_distance(_e121, NodeLayer {_e107.sd, opacity_1, _e107.coverage}, in_11);
                node_sd = _e125;
            } else {
                SliceZones _e127 = slice_zones(in_11, slot_3, oct_1, in_11.uv, d_5, _e107, _e109, band_in_1, band_out_1, _e116, aa_8, u);
                slot_rgb = _e127.ink.xyz;
                cov_1 = _e127.ink.w;
                lit_shape = _e127.lit;
                float _e133 = glyph_mask;
                float _e139 = mask_level(_e127.far.level);
                glyph_mask = metal::max(_e133, _e127.near.coverage + (_e127.rest * _e139));
                float _e143 = node_sd;
                float _e145 = layer_distance(_e143, _e127.near, in_11);
                node_sd = _e145;
                float _e146 = node_sd;
                float _e148 = layer_distance(_e146, _e127.far, in_11);
                node_sd = _e148;
            }
            float _e149 = cov_1;
            float _e150 = glyph;
            if (_e149 > _e150) {
                float _e152 = cov_1;
                glyph = _e152;
                metal::float3 _e153 = slot_rgb;
                glyph_rgb = _e153;
                float _e154 = lit_shape;
                glyph_lit = _e154 * _e96;
            }
        }
    }
    float _e159 = glyph_taper(d_5, swells_1);
    float _e160 = glyph;
    glyph = _e160 * _e159;
    float _e162 = glyph_lit;
    glyph_lit = _e162 * _e159;
    float _e164 = glyph_mask;
    glyph_mask = _e164 * _e159;
    {
        metal::float2 _e166 = spectral_radii(u);
        NodeLayer _e171 = glyph_band(d_5, _e166.x, _e166.y, 1.0, aa_8);
        RingInk _e172 = spectral_ring(in_11, oct_1, in_11.uv, _e171, aa_8, analytic_4, u);
        float _e173 = node_sd;
        float _e175 = layer_distance(_e173, _e172.layer, in_11);
        node_sd = _e175;
        metal::float3 _e179 = glyph_rgb;
        float _e180 = glyph;
        float _e188 = glyph;
        glyph_rgb = ((_e172.color * _e172.cov) + ((_e179 * _e180) * (1.0 - _e172.cov))) / metal::float3(metal::max(_e172.cov + (_e188 * (1.0 - _e172.cov)), 0.0001));
        float _e201 = glyph_lit;
        glyph_lit = (_e172.lit * _e172.cov) + (_e201 * (1.0 - _e172.cov));
        float _e208 = glyph;
        glyph = _e172.cov + (_e208 * (1.0 - _e172.cov));
        float _e217 = mask_level(in_11.ring);
        float audio_mask = _e172.layer.coverage * _e217;
        float _e219 = glyph_mask;
        glyph_mask = audio_mask + (_e219 * (1.0 - audio_mask));
    }
    if (mark_out > mark_in) {
        NodeLayer _e231 = glyph_band(d_5, mark_in, mark_out, 1.0, aa_8);
        mark_strip = _e231;
    }
    float _e236 = mark_strip.sd;
    float _e240 = mark_strip.coverage;
    NodeLayer _e242 = drawn_marks(in_11, in_11.marks.x, oct_1, in_11.uv, d_5, NodeLayer {_e236, in_11.params.y, _e240}, mark_in, mark_out, aa_8, analytic_4, u);
    float _e247 = mark_strip.sd;
    float _e251 = mark_strip.coverage;
    NodeLayer _e253 = drawn_marks(in_11, in_11.marks.y, oct_1, in_11.uv, d_5, NodeLayer {_e247, in_11.params.z, _e251}, mark_in, mark_out, aa_8, analytic_4, u);
    float _e254 = layer_coverage(_e242);
    float _e255 = layer_coverage(_e253);
    float _e258 = mask_level(_e242.level);
    float melody_mask = _e242.coverage * _e258;
    float _e262 = mask_level(_e253.level);
    float bass_mask = _e253.coverage * _e262;
    float _e264 = node_sd;
    float _e265 = layer_distance(_e264, _e242, in_11);
    node_sd = _e265;
    float _e266 = node_sd;
    float _e267 = layer_distance(_e266, _e253, in_11);
    node_sd = _e267;
    mark = metal::max(_e254, _e255);
    mark_mask = metal::max(melody_mask, bass_mask);
    metal::float3 mark_rgb = (_e254 > _e255) ? in_11.melody_color.xyz : in_11.bass_color.xyz;
    float mark_taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_5);
    float _e283 = mark;
    mark = _e283 * mark_taper;
    float _e285 = mark_mask;
    mark_mask = _e285 * mark_taper;
    float _e287 = mark;
    metal::float3 _e289 = glyph_rgb;
    float _e290 = glyph;
    float _e292 = mark;
    float _e297 = mark;
    float _e298 = glyph;
    float _e299 = mark;
    glyph_rgb = ((mark_rgb * _e287) + ((_e289 * _e290) * (1.0 - _e292))) / metal::float3(metal::max(_e297 + (_e298 * (1.0 - _e299)), 0.0001));
    float _e308 = mark;
    float _e309 = glyph_lit;
    float _e310 = mark;
    glyph_lit = _e308 + (_e309 * (1.0 - _e310));
    float _e315 = mark;
    float _e316 = glyph;
    float _e317 = mark;
    glyph = _e315 + (_e316 * (1.0 - _e317));
    float _e322 = mark_mask;
    float _e323 = glyph_mask;
    float _e324 = mark_mask;
    glyph_mask = _e322 + (_e323 * (1.0 - _e324));
    float _e329 = glyph;
    float _e330 = base_alpha;
    float _e331 = glyph;
    float active_alpha = _e329 + (_e330 * (1.0 - _e331));
    metal::float3 _e336 = glyph_rgb;
    float _e337 = glyph;
    metal::float3 _e339 = base_rgb;
    float _e340 = glyph;
    metal::float3 active_rgb = (_e336 * _e337) + (_e339 * (1.0 - _e340));
    float _e345 = glyph_lit;
    float _e349 = glyph_mask;
    float _e350 = node_sd;
    return NodeInk {active_rgb, active_alpha, _e345 / metal::max(active_alpha, 0.0001), _e349, _e350};
}

float slice_progress(
    VsOut in_12,
    uint i_8
) {
    return static_cast<float>((in_12.motion[metal::min(unsigned(naga_div(i_8, 3u)), 3u)] >> (naga_mod(i_8, 3u) * 10u)) & 1023u) / 1023.0;
}

AnimatedInk animated_slice_ink(
    VsOut in_13,
    float aa_9,
    OctRing oct_2,
    constant Uniforms& u
) {
    NodeInk result = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, 65504.0};
    NodeInk marks = {};
    uint i_9 = 0u;
    float coverage_2 = {};
    metal::float3 rgb = {};
    float lit = {};
    bool local_37 = {};
    bool local_38 = {};
    NodeInk _e11 = result;
    marks = _e11;
    float band_in_2 = u.node.band_inner;
    float band_out_2 = u.node.band_outer;
    float _e24 = u.node.mark_inner;
    float mark_in_1 = metal::min(_e24, 1.58);
    float _e30 = u.node.mark_thickness;
    float mark_out_1 = metal::min(mark_in_1 + metal::max(_e30, 0.0), 1.58);
    float anchor_radius = (band_out_2 > band_in_2) ? (0.5 * (band_in_2 + band_out_2)) : (0.5 * (mark_in_1 + mark_out_1));
    bool swells_2 = in_13.swell > band_out_2;
    uint2 loop_bound_5 = uint2(4294967295u);
    bool loop_init_5 = true;
    while(true) {
        if (metal::all(loop_bound_5 == uint2(0u))) { break; }
        loop_bound_5 -= uint2(loop_bound_5.y == 0u, 1u);
        if (!loop_init_5) {
            uint _e282 = i_9;
            i_9 = _e282 + 1u;
        }
        loop_init_5 = false;
        uint _e48 = i_9;
        uint _e49 = oct_span(u);
        if (_e48 < _e49) {
        } else {
            break;
        }
        {
            uint _e52 = i_9;
            int slot_4 = as_type<int>(as_type<uint>(oct_2.base) + as_type<uint>(static_cast<int>(_e52)));
            uint _e55 = i_9;
            float _e56 = slice_progress(in_13, _e55);
            if (_e56 <= 0.0) {
                continue;
            }
            float _e63 = u.node.pose.x;
            float scale = metal::mix(_e63, 1.0, _e56);
            if (_e56 < INK_FLOOR) {
                continue;
            }
            if (scale <= 0.001) {
                continue;
            }
            float _e70 = oct_mid(slot_4, oct_2, u);
            metal::float2 anchor = anchor_radius * metal::float2(metal::cos(_e70), metal::sin(_e70));
            float _e79 = u.node.pose.y;
            metal::float2 start_1 = anchor * (1.0 + (_e79 * (1.0 - _e56)));
            metal::float2 uv_8 = anchor + ((in_13.uv - start_1) / metal::float2(scale));
            float d_9 = metal::length(uv_8);
            float soft = aa_9 / scale;
            if (band_out_2 > band_in_2) {
                NodeLayer _e95 = glyph_band(d_9, band_in_2, band_out_2, 1.0, soft);
                NodeLayer _e96 = outer_glyph(slot_4, oct_2, uv_8, _e95, band_in_2, band_out_2, soft, u);
                metal::float4 _e97 = oct_slot_ink(in_13, slot_4, u);
                float _e98 = glyph_taper(d_9, swells_2);
                float _e100 = oct_slot_level(in_13.octaves, slot_4);
                coverage_2 = ((_e96.coverage * _e98) * _e97.w) * _e56;
                rgb = _e97.xyz;
                lit = _e100 / metal::max(_e97.w, 0.0001);
                float _e114 = slice_reach(in_13, slot_4, band_in_2, band_out_2, u);
                if (_e114 == band_out_2) {
                    float _e118 = result.sd;
                    float _e125 = layer_distance(_e118, NodeLayer {_e96.sd * scale, _e97.w * _e56, _e96.coverage}, in_13);
                    result.sd = _e125;
                    float _e128 = result.mask;
                    float _e133 = mask_level(_e97.w * _e56);
                    result.mask = metal::max(_e128, (_e96.coverage * _e98) * _e133);
                } else {
                    SliceZones _e136 = slice_zones(in_13, slot_4, oct_2, uv_8, d_9, _e96, _e97, band_in_2, band_out_2, _e114, soft, u);
                    rgb = _e136.ink.xyz;
                    coverage_2 = (_e136.ink.w * _e98) * _e56;
                    lit = (_e100 * _e136.lit) / metal::max(_e136.ink.w, 0.0001);
                    NodeLayer near_1 = _e136.near;
                    NodeLayer far_1 = _e136.far;
                    float _e154 = result.sd;
                    float _e161 = layer_distance(_e154, NodeLayer {_e136.near.sd * scale, _e136.near.level * _e56, _e136.near.coverage}, in_13);
                    result.sd = _e161;
                    float _e164 = result.sd;
                    float _e171 = layer_distance(_e164, NodeLayer {_e136.far.sd * scale, _e136.far.level * _e56, _e136.far.coverage}, in_13);
                    result.sd = _e171;
                    float _e175 = mask_level(_e136.near.level * _e56);
                    float _e180 = mask_level(_e136.far.level * _e56);
                    float footprint = (_e136.near.coverage * _e175) + (_e136.rest * _e180);
                    float _e185 = result.mask;
                    result.mask = metal::max(_e185, footprint * _e98);
                }
                float _e188 = coverage_2;
                float _e190 = result.alpha;
                if (_e188 > _e190) {
                    metal::float3 _e193 = rgb;
                    float _e194 = coverage_2;
                    result.rgb = _e193 * _e194;
                    float _e197 = coverage_2;
                    result.alpha = _e197;
                    float _e199 = lit;
                    result.lit = _e199;
                }
            }
            metal::float2 _e200 = mark_radii(in_13, slot_4, mark_in_1, mark_out_1, u);
            if (slot_4 >= 0) {
                local_37 = slot_4 < 11;
            } else {
                local_37 = false;
            }
            bool _e208 = local_37;
            if (_e208) {
                local_38 = _e200.y > _e200.x;
            } else {
                local_38 = false;
            }
            bool _e215 = local_38;
            if (_e215) {
                uint bit = 1u << static_cast<uint>(slot_4);
                float melody = ((in_13.marks.x & bit) != 0u) ? in_13.params.y : 0.0;
                float bass = ((in_13.marks.y & bit) != 0u) ? in_13.params.z : 0.0;
                float level_4 = metal::max(melody, bass) * _e56;
                metal::float3 color = (melody > bass) ? in_13.melody_color.xyz : in_13.bass_color.xyz;
                NodeLayer _e247 = glyph_band(d_9, _e200.x, _e200.y, level_4, soft);
                NodeLayer _e250 = outer_glyph(slot_4, oct_2, uv_8, _e247, _e200.x, _e200.y, soft, u);
                float taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_9);
                float _e256 = layer_coverage(_e250);
                float coverage_4 = _e256 * taper;
                float _e259 = marks.alpha;
                if (coverage_4 > _e259) {
                    marks.rgb = color * coverage_4;
                    marks.alpha = coverage_4;
                    marks.lit = 1.0;
                }
                float _e268 = marks.mask;
                float _e271 = mask_level(level_4);
                marks.mask = metal::max(_e268, (_e250.coverage * taper) * _e271);
                float _e276 = marks.sd;
                float _e281 = layer_distance(_e276, NodeLayer {_e250.sd * scale, level_4, _e250.coverage}, in_13);
                marks.sd = _e281;
            }
        }
    }
    NodeInk _e285 = result;
    NodeInk _e286 = marks;
    return AnimatedInk {_e285, _e286};
}

NodeInk node_ink(
    VsOut src_1,
    float d_6,
    float aa_10,
    OctRing oct_3,
    bool analytic_5,
    constant Uniforms& u
) {
    bool local_39 = {};
    bool local_40 = {};
    NodeInk ink_1 = {};
    float _e8 = u.node.animation;
    if (_e8 == 0.0) {
        NodeInk _e11 = base_node_ink(src_1, d_6, aa_10, oct_3, analytic_5, u);
        return _e11;
    }
    bool settled = (src_1.motion.w & 2147483648u) != 0u;
    float _e21 = u.node.band_outer;
    float _e25 = u.node.band_inner;
    if (_e21 <= _e25) {
        float _e32 = u.node.mark_thickness;
        local_39 = _e32 <= 0.0;
    } else {
        local_39 = false;
    }
    bool only_audio = local_39;
    if (!(settled)) {
        local_40 = only_audio;
    } else {
        local_40 = true;
    }
    bool _e41 = local_40;
    if (_e41) {
        NodeInk _e42 = base_node_ink(src_1, d_6, aa_10, oct_3, analytic_5, u);
        return _e42;
    }
    AnimatedInk _e43 = animated_slice_ink(src_1, aa_10, oct_3, u);
    ink_1 = _e43.body;
    metal::float2 _e46 = spectral_radii(u);
    NodeLayer _e53 = glyph_band(metal::length(src_1.uv), _e46.x, _e46.y, 1.0, aa_10);
    RingInk _e54 = spectral_ring(src_1, oct_3, src_1.uv, _e53, aa_10, analytic_5, u);
    float _e59 = ink_1.lit;
    float _e61 = ink_1.alpha;
    float lit_1 = (_e54.lit * _e54.cov) + ((_e59 * _e61) * (1.0 - _e54.cov));
    metal::float3 _e73 = ink_1.rgb;
    ink_1.rgb = (_e54.color * _e54.cov) + (_e73 * (1.0 - _e54.cov));
    float _e82 = ink_1.alpha;
    ink_1.alpha = _e54.cov + (_e82 * (1.0 - _e54.cov));
    float _e90 = ink_1.alpha;
    ink_1.lit = lit_1 / metal::max(_e90, 0.0001);
    float _e97 = mask_level(src_1.ring);
    float mask = _e54.layer.coverage * _e97;
    float _e101 = ink_1.mask;
    ink_1.mask = mask + (_e101 * (1.0 - mask));
    float _e108 = ink_1.sd;
    float _e110 = layer_distance(_e108, _e54.layer, src_1);
    ink_1.sd = _e110;
    float _e114 = ink_1.lit;
    float _e116 = ink_1.alpha;
    float mark_lit = _e43.marks.alpha + ((_e114 * _e116) * (1.0 - _e43.marks.alpha));
    metal::float3 _e128 = ink_1.rgb;
    ink_1.rgb = _e43.marks.rgb + (_e128 * (1.0 - _e43.marks.alpha));
    float _e139 = ink_1.alpha;
    ink_1.alpha = _e43.marks.alpha + (_e139 * (1.0 - _e43.marks.alpha));
    float _e148 = ink_1.alpha;
    ink_1.lit = mark_lit / metal::max(_e148, 0.0001);
    float _e156 = ink_1.mask;
    ink_1.mask = _e43.marks.mask + (_e156 * (1.0 - _e43.marks.mask));
    float _e165 = ink_1.sd;
    ink_1.sd = metal::min(_e165, _e43.marks.sd);
    NodeInk _e169 = ink_1;
    return _e169;
}

struct fs_node_cellInput {
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
    float swell [[user(loc13), flat]];
    float ring [[user(loc14), flat]];
    float ink_carry [[user(loc15), flat]];
    metal::float4 shadow_box [[user(loc10), flat]];
    metal::float4 shadow_at [[user(loc12), center_no_perspective]];
};
struct fs_node_cellOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_node_cellOutput fs_node_cell(
  fs_node_cellInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, constant Uniforms& u [[buffer(0)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.params, varyings.octaves, varyings.thickness, varyings.motion, varyings.cents, varyings.strip_row, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.swell, varyings.ring, varyings.ink_carry, varyings.shadow_box, varyings.shadow_at };
    bool local = {};
    metal::float4 cell = in.shadow_box;
    metal::float2 at = in.clip_pos.xy;
    if (!(metal::any(at < cell.xy))) {
        local = metal::any(at > (cell.xy + cell.zw));
    } else {
        local = true;
    }
    bool _e16 = local;
    if (_e16) {
        metal::discard_fragment();
    }
    bool analytic_6 = in.shadow_at.z < 0.0;
    NodeGeom _e21 = node_geom(in, analytic_6, u);
    if (!(_e21.paints)) {
        return fs_node_cellOutput { metal::float4(0.0) };
    }
    NodeInk _e29 = node_ink(in, _e21.d, _e21.aa, _e21.oct, analytic_6, u);
    if (in.params.w > 1.5) {
        return fs_node_cellOutput { metal::float4(metal::clamp(-(_e29.sd), 0.0, 1.0), 0.0, 0.0, 0.0) };
    }
    if (analytic_6) {
        float points = metal::clamp(_e29.sd * metal::abs(in.shadow_at.z), -65504.0, EMPTY_DISTANCE);
        float stable = metal::rint(points * 32.0) / 32.0;
        return fs_node_cellOutput { metal::float4(stable, 0.0, 0.0, 0.0) };
    }
    return fs_node_cellOutput { metal::float4((_e29.alpha < INK_FLOOR) ? 0.0 : _e29.alpha, 0.0, 0.0, 0.0) };
}
