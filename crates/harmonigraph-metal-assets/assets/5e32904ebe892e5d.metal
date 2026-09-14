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
    float transition;
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
    type_8 pitch_lut;
    type_8 spectral_lut;
    type_10 spectrum_color;
    type_11 ink_kernel;
};
struct VsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float4 params;
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
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);

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
    float _e23 = u.node.transition;
    if (_e23 != 0.0) {
        float _e26 = reach;
        reach = metal::max(_e26, metal::max(in_1.rim, 1.0) * 1.25);
    }
    float _e33 = reach;
    metal::float2 _e35 = spectral_radii(u);
    return metal::max(_e33, metal::max(in_1.rim, _e35.y) + aa);
}

float aa_inside(
    float edge,
    float x,
    float w
) {
    return 1.0 - metal::smoothstep(edge - w, edge + w, x);
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
    bool local_2 = {};
    if (!((s_2 < 0))) {
        local_2 = s_2 >= 11;
    } else {
        local_2 = true;
    }
    bool _e10 = local_2;
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
    float d,
    float inner,
    float outer,
    float level,
    float aa_2
) {
    float mid = 0.5 * (inner + outer);
    float _e14 = aa_inside(outer, d, aa_2);
    float _e15 = aa_inside(inner, d, aa_2);
    return NodeLayer {metal::abs(d - mid) - (0.5 * (outer - inner)), level, _e14 * (1.0 - _e15)};
}

metal::float3 pitch_lut_color(
    float pitch,
    constant Uniforms& u
) {
    float _e4 = u.composite.darkest_pitch;
    float _e9 = u.composite.brightest_pitch;
    float _e13 = u.composite.darkest_pitch;
    float t = metal::clamp((pitch - _e4) / metal::max(_e9 - _e13, 0.01), 0.0, 1.0);
    float f_3 = t * 63.0;
    uint i0_ = naga_f2u32(metal::floor(f_3));
    uint i1_ = metal::min(i0_ + 1u, 63u);
    metal::float4 _e32 = u.pitch_lut.inner[metal::min(unsigned(i0_), 63u)];
    metal::float4 _e37 = u.pitch_lut.inner[metal::min(unsigned(i1_), 63u)];
    return metal::mix(_e32.xyz, _e37.xyz, f_3 - metal::floor(f_3));
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
    bool local_3 = {};
    bool local_4 = {};
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
        local_3 = radius <= outer_1;
    } else {
        local_3 = false;
    }
    bool _e77 = local_3;
    if (_e77) {
        local_4 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_4 = false;
    }
    bool inside = local_4;
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
    float level_1,
    constant Uniforms& u
) {
    float f_4 = metal::clamp(level_1, 0.0, 1.0) * 63.0;
    uint i0_1 = naga_f2u32(metal::floor(f_4));
    uint i1_1 = metal::min(i0_1 + 1u, 63u);
    metal::float4 _e15 = u.spectral_lut.inner[metal::min(unsigned(i0_1), 63u)];
    metal::float4 _e20 = u.spectral_lut.inner[metal::min(unsigned(i1_1), 63u)];
    return metal::mix(_e15.xyz, _e20.xyz, f_4 - metal::floor(f_4));
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
    bool local_5 = {};
    float x_2 = ((pitch_1 - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE) - 0.5;
    if (!((x_2 < 0.0))) {
        local_5 = x_2 > 3827.0;
    } else {
        local_5 = true;
    }
    bool _e15 = local_5;
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
    bool local_6 = {};
    bool local_7 = {};
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
        local_6 = !(analytic);
    } else {
        local_6 = false;
    }
    bool _e36 = local_6;
    if (_e36) {
        float _e39 = layer_coverage(band_1);
        local_7 = _e39 <= 0.0;
    } else {
        local_7 = false;
    }
    bool _e43 = local_7;
    if (_e43) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    owner = oct.base;
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
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
    bool local_8 = {};
    bool local_9 = {};
    float sd_2 = EMPTY_DISTANCE;
    float coverage = 0.0;
    uint i_2 = 0u;
    bool local_10 = {};
    bool local_11 = {};
    if (EARLY_OUT) {
        local_8 = !(analytic_1);
    } else {
        local_8 = false;
    }
    bool _e13 = local_8;
    if (_e13) {
        float _e16 = layer_coverage(strip);
        local_9 = _e16 <= 0.0;
    } else {
        local_9 = false;
    }
    bool _e20 = local_9;
    if (_e20) {
        return NodeLayer {EMPTY_DISTANCE, strip.level, 0.0};
    }
    uint _e26 = oct_span(u);
    int top = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_3.base) + as_type<uint>(static_cast<int>(_e26)))) - as_type<uint>(1));
    uint2 loop_bound_1 = uint2(4294967295u);
    bool loop_init_1 = true;
    while(true) {
        if (metal::all(loop_bound_1 == uint2(0u))) { break; }
        loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
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
                local_10 = s_6 >= ring_3.base;
            } else {
                local_10 = false;
            }
            bool _e53 = local_10;
            if (_e53) {
                local_11 = s_6 <= top;
            } else {
                local_11 = false;
            }
            bool _e58 = local_11;
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

float transition_scale(
    float phase,
    constant Uniforms& u
) {
    float _e4 = u.node.transition;
    if (_e4 != 1.0) {
        return 1.0;
    }
    float p = metal::abs(phase);
    if (phase < 0.0) {
        return 0.55 + (0.45 * p);
    }
    float t_1 = p - 1.0;
    return (1.0 + (((2.1 * t_1) * t_1) * t_1)) + ((1.1 * t_1) * t_1);
}

VsOut transition_input(
    VsOut src,
    constant Uniforms& u
) {
    VsOut result = {};
    result = src;
    metal::float2 _e3 = result.uv;
    float _e6 = transition_scale(src.params.w, u);
    result.uv = _e3 / metal::float2(metal::max(_e6, 0.05));
    VsOut _e11 = result;
    return _e11;
}

NodeGeom node_geom(
    VsOut src_1,
    bool analytic_2,
    constant Uniforms& u
) {
    bool local_12 = {};
    bool local_13 = {};
    bool local_14 = {};
    bool local_15 = {};
    bool local_16 = {};
    bool local_17 = {};
    bool local_18 = {};
    bool local_19 = {};
    bool local_20 = {};
    bool local_21 = {};
    bool local_22 = {};
    VsOut _e2 = transition_input(src_1, u);
    float d_4 = metal::length(_e2.uv);
    float _e7 = metal::fwidth(_e2.uv.x);
    float _e10 = aa_width(_e7, _e2.shadow_at.w, u);
    if (EARLY_OUT) {
        local_12 = !(analytic_2);
    } else {
        local_12 = false;
    }
    bool _e16 = local_12;
    if (_e16) {
        float _e22 = u.node.transition;
        local_13 = _e22 == 0.0;
    } else {
        local_13 = false;
    }
    bool _e26 = local_13;
    if (_e26) {
        float _e29 = paint_reach(_e2, _e10, u);
        local_14 = d_4 > _e29;
    } else {
        local_14 = false;
    }
    bool _e32 = local_14;
    if (_e32) {
        return NodeGeom {d_4, _e10, OctRing {0, 0.0}, false};
    }
    metal::float2 _e38 = spectral_radii(u);
    bool ring_draws = _e38.y > _e38.x;
    if (ring_draws) {
        local_15 = metal::length(src_1.uv) >= (_e38.x - _e10);
    } else {
        local_15 = false;
    }
    bool _e50 = local_15;
    if (_e50) {
        local_16 = metal::length(src_1.uv) <= (_e38.y + _e10);
    } else {
        local_16 = false;
    }
    bool in_audio_ring = local_16;
    if (EARLY_OUT) {
        local_17 = !(analytic_2);
    } else {
        local_17 = false;
    }
    bool _e65 = local_17;
    if (_e65) {
        local_18 = !(in_audio_ring);
    } else {
        local_18 = false;
    }
    bool _e70 = local_18;
    if (_e70) {
        local_19 = _e2.params.x <= 0.0;
    } else {
        local_19 = false;
    }
    bool _e78 = local_19;
    if (_e78) {
        local_20 = _e2.params.y <= 0.0;
    } else {
        local_20 = false;
    }
    bool _e86 = local_20;
    if (_e86) {
        local_21 = _e2.params.z <= 0.0;
    } else {
        local_21 = false;
    }
    bool _e94 = local_21;
    if (_e94) {
        local_22 = ((_e2.octaves[0] | _e2.octaves[1]) | _e2.octaves[2]) == 0u;
    } else {
        local_22 = false;
    }
    bool _e108 = local_22;
    if (_e108) {
        return NodeGeom {d_4, _e10, OctRing {0, 0.0}, false};
    }
    OctRing _e115 = oct_ring(_e2.cents, u);
    return NodeGeom {d_4, _e10, _e115, true};
}

float mask_level(
    float level_2
) {
    return metal::clamp(level_2 / INK_FLOOR, 0.0, 1.0);
}

NodeInk base_node_ink(
    VsOut in_5,
    float d_1,
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
    bool local_23 = {};
    bool local_24 = {};
    bool local_25 = {};
    bool local_26 = {};
    NodeLayer mark_strip = NodeLayer {65504.0, 0.0, 0.0};
    float mark = {};
    float mark_mask = {};
    float presence_1 = in_5.params.x;
    float _e15 = u.node.transition;
    float focus = (_e15 == 4.0) ? (in_5.params.w * in_5.params.w) : 1.0;
    metal::float4 _e29 = u.lattice_ground;
    glyph_rgb = _e29.xyz;
    float band_in = u.node.band_inner;
    float band_out = u.node.band_outer;
    float mark_thick = u.node.mark_thickness;
    float mark_w = metal::max(mark_thick, 0.0);
    float _e56 = u.node.mark_inner;
    float mark_in = metal::min(_e56, 1.58);
    float mark_out = metal::min(mark_in + mark_w, 1.58);
    if (band_out > band_in) {
        NodeLayer _e67 = glyph_band(d_1, band_in, band_out, 1.0, aa_6);
        band_2 = _e67;
    }
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e126 = i_3;
            i_3 = _e126 + 1u;
        }
        loop_init_2 = false;
        uint _e70 = i_3;
        uint _e71 = oct_span(u);
        if (_e70 < _e71) {
            if (true) {
                local_24 = analytic_3;
            } else {
                local_24 = true;
            }
            bool _e79 = local_24;
            if (!(_e79)) {
                NodeLayer _e83 = band_2;
                float _e84 = layer_coverage(_e83);
                local_25 = _e84 > 0.0;
            } else {
                local_25 = true;
            }
            bool _e88 = local_25;
            local_23 = _e88;
        } else {
            local_23 = false;
        }
        bool _e90 = local_23;
        if (_e90) {
        } else {
            break;
        }
        {
            uint _e92 = i_3;
            int slot_3 = as_type<int>(as_type<uint>(oct_1.base) + as_type<uint>(static_cast<int>(_e92)));
            float _e96 = oct_slot_level(in_5.octaves, slot_3);
            if (_e96 <= 0.0) {
                local_26 = presence_1 <= 0.0;
            } else {
                local_26 = false;
            }
            bool _e104 = local_26;
            if (_e104) {
                continue;
            }
            NodeLayer _e106 = band_2;
            NodeLayer _e107 = outer_glyph(slot_3, oct_1, in_5.uv, _e106, band_in, band_out, aa_6, u);
            float _e108 = layer_coverage(_e107);
            float _e109 = glyph_mask;
            glyph_mask = metal::max(_e109, _e107.coverage);
            metal::float4 _e112 = oct_slot_ink(in_5, slot_3, u);
            float opacity_1 = _e112.w * focus;
            float _e115 = node_sd;
            float _e119 = layer_distance(_e115, NodeLayer {_e107.sd, opacity_1, _e107.coverage});
            node_sd = _e119;
            metal::float3 slot_rgb = _e112.xyz;
            float cov_1 = _e108 * opacity_1;
            float _e122 = glyph;
            if (cov_1 > _e122) {
                glyph = cov_1;
                glyph_rgb = slot_rgb;
                glyph_lit = (_e108 * _e96) * focus;
            }
        }
    }
    float glyph_taper = 1.0 - metal::smoothstep(1.0, GLYPH_FADE_LIMIT, d_1);
    float _e134 = glyph;
    glyph = _e134 * glyph_taper;
    float _e136 = glyph_lit;
    glyph_lit = _e136 * glyph_taper;
    float _e138 = glyph_mask;
    glyph_mask = _e138 * glyph_taper;
    float _e143 = u.node.transition;
    if (_e143 == 0.0) {
        metal::float2 _e146 = spectral_radii(u);
        NodeLayer _e151 = glyph_band(d_1, _e146.x, _e146.y, 1.0, aa_6);
        RingInk _e152 = spectral_ring(in_5, oct_1, in_5.uv, _e151, aa_6, analytic_3, u);
        float _e153 = node_sd;
        float _e155 = layer_distance(_e153, _e152.layer);
        node_sd = _e155;
        metal::float3 _e159 = glyph_rgb;
        float _e160 = glyph;
        float _e168 = glyph;
        glyph_rgb = ((_e152.color * _e152.cov) + ((_e159 * _e160) * (1.0 - _e152.cov))) / metal::float3(metal::max(_e152.cov + (_e168 * (1.0 - _e152.cov)), 0.0001));
        float _e181 = glyph_lit;
        glyph_lit = (_e152.lit * _e152.cov) + (_e181 * (1.0 - _e152.cov));
        float _e188 = glyph;
        glyph = _e152.cov + (_e188 * (1.0 - _e152.cov));
        float _e197 = mask_level(in_5.ring);
        float audio_mask = _e152.layer.coverage * _e197;
        float _e199 = glyph_mask;
        glyph_mask = audio_mask + (_e199 * (1.0 - audio_mask));
    }
    if (mark_out > mark_in) {
        NodeLayer _e211 = glyph_band(d_1, mark_in, mark_out, 1.0, aa_6);
        mark_strip = _e211;
    }
    float _e216 = mark_strip.sd;
    float _e221 = mark_strip.coverage;
    NodeLayer _e223 = mark_extension(in_5.marks.x, oct_1, in_5.uv, NodeLayer {_e216, in_5.params.y * focus, _e221}, mark_in, mark_out, aa_6, analytic_3, u);
    float _e228 = mark_strip.sd;
    float _e233 = mark_strip.coverage;
    NodeLayer _e235 = mark_extension(in_5.marks.y, oct_1, in_5.uv, NodeLayer {_e228, in_5.params.z * focus, _e233}, mark_in, mark_out, aa_6, analytic_3, u);
    float _e236 = layer_coverage(_e223);
    float _e237 = layer_coverage(_e235);
    float _e240 = mask_level(_e223.level);
    float melody_mask = _e223.coverage * _e240;
    float _e244 = mask_level(_e235.level);
    float bass_mask = _e235.coverage * _e244;
    float _e246 = node_sd;
    float _e247 = layer_distance(_e246, _e223);
    node_sd = _e247;
    float _e248 = node_sd;
    float _e249 = layer_distance(_e248, _e235);
    node_sd = _e249;
    mark = metal::max(_e236, _e237);
    mark_mask = metal::max(melody_mask, bass_mask);
    metal::float3 mark_rgb = (_e236 > _e237) ? in_5.melody_color.xyz : in_5.bass_color.xyz;
    float mark_taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_1);
    float _e265 = mark;
    mark = _e265 * mark_taper;
    float _e267 = mark_mask;
    mark_mask = _e267 * mark_taper;
    float _e269 = mark;
    metal::float3 _e271 = glyph_rgb;
    float _e272 = glyph;
    float _e274 = mark;
    float _e279 = mark;
    float _e280 = glyph;
    float _e281 = mark;
    glyph_rgb = ((mark_rgb * _e269) + ((_e271 * _e272) * (1.0 - _e274))) / metal::float3(metal::max(_e279 + (_e280 * (1.0 - _e281)), 0.0001));
    float _e290 = mark;
    float _e291 = glyph_lit;
    float _e292 = mark;
    glyph_lit = _e290 + (_e291 * (1.0 - _e292));
    float _e297 = mark;
    float _e298 = glyph;
    float _e299 = mark;
    glyph = _e297 + (_e298 * (1.0 - _e299));
    float _e304 = mark_mask;
    float _e305 = glyph_mask;
    float _e306 = mark_mask;
    glyph_mask = _e304 + (_e305 * (1.0 - _e306));
    float _e311 = glyph;
    float _e312 = base_alpha;
    float _e313 = glyph;
    float active_alpha = _e311 + (_e312 * (1.0 - _e313));
    metal::float3 _e318 = glyph_rgb;
    float _e319 = glyph;
    metal::float3 _e321 = base_rgb;
    float _e322 = glyph;
    metal::float3 active_rgb = (_e318 * _e319) + (_e321 * (1.0 - _e322));
    float _e327 = glyph_lit;
    float _e331 = glyph_mask;
    float _e332 = node_sd;
    return NodeInk {active_rgb, active_alpha, _e327 / metal::max(active_alpha, 0.0001), _e331, _e332};
}

NodeInk node_ink(
    VsOut src_2,
    float d_2,
    float aa_7,
    OctRing oct_2,
    bool analytic_4,
    constant Uniforms& u
) {
    NodeInk ink = {};
    bool local_27 = {};
    float accent = 0.0;
    float accent_sd = EMPTY_DISTANCE;
    bool local_28 = {};
    bool local_29 = {};
    bool local_30 = {};
    bool local_31 = {};
    bool local_32 = {};
    VsOut _e5 = transition_input(src_2, u);
    NodeInk _e6 = base_node_ink(_e5, d_2, aa_7, oct_2, analytic_4, u);
    ink = _e6;
    float mode = u.node.transition;
    if (mode == 0.0) {
        NodeInk _e14 = ink;
        return _e14;
    }
    float phase_1 = src_2.params.w;
    float p_1 = metal::abs(phase_1);
    if (mode == 1.0) {
        float _e21 = ink.sd;
        float _e22 = transition_scale(phase_1, u);
        ink.sd = _e21 * metal::max(_e22, 0.05);
    }
    if (mode == 2.0) {
        local_27 = p_1 < 1.0;
    } else {
        local_27 = false;
    }
    bool _e33 = local_27;
    if (_e33) {
        float angle = metal::fract((metal::atan2(_e5.uv.x, _e5.uv.y) / TAU) + 1.0);
        float end_delta = metal::abs(angle - p_1);
        float nearest_1 = metal::min(metal::min(angle, 1.0 - angle), metal::min(end_delta, 1.0 - end_delta));
        float edge_2 = (((angle <= p_1) ? -1.0 : 1.0) * d_2) * metal::sin(metal::min(nearest_1 * TAU, 1.5707964));
        float coverage_1 = (p_1 > 0.0) ? (1.0 - metal::smoothstep(-(aa_7), aa_7, edge_2)) : 0.0;
        metal::float3 _e73 = ink.rgb;
        ink.rgb = _e73 * coverage_1;
        float _e76 = ink.alpha;
        ink.alpha = _e76 * coverage_1;
        float _e79 = ink.mask;
        ink.mask = _e79 * coverage_1;
        float _e83 = ink.sd;
        ink.sd = metal::max(_e83, edge_2);
    }
    if (mode == 4.0) {
        float _e88 = ink.mask;
        ink.mask = _e88 * (p_1 * p_1);
    }
    float rim = metal::max(src_2.rim, 0.4);
    if (mode == 3.0) {
        local_28 = src_2.params.x > 0.0;
    } else {
        local_28 = false;
    }
    bool _e107 = local_28;
    if (_e107) {
        local_29 = phase_1 >= 0.0;
    } else {
        local_29 = false;
    }
    bool _e113 = local_29;
    if (_e113) {
        local_30 = p_1 < 1.0;
    } else {
        local_30 = false;
    }
    bool _e119 = local_30;
    if (_e119) {
        float radius_1 = metal::mix(0.12, metal::max(rim, 1.0) * 1.2, p_1);
        accent_sd = metal::abs(metal::length(src_2.uv) - radius_1) - 0.018;
        float _e133 = accent_sd;
        accent = (1.0 - metal::smoothstep(-(aa_7), aa_7, _e133)) * metal::sin(p_1 * 3.1415927);
    }
    if (mode == 5.0) {
        local_31 = src_2.params.x > 0.0;
    } else {
        local_31 = false;
    }
    bool _e150 = local_31;
    if (_e150) {
        local_32 = p_1 < 1.0;
    } else {
        local_32 = false;
    }
    bool _e156 = local_32;
    if (_e156) {
        float travel = (phase_1 >= 0.0) ? p_1 : (1.0 - p_1);
        float angle_1 = metal::fract((metal::atan2(src_2.uv.x, src_2.uv.y) / TAU) + 1.0);
        float behind = metal::fract((travel - angle_1) + 1.0);
        float tail = metal::max(0.0, 1.0 - (behind / 0.18));
        accent_sd = metal::max(metal::abs(metal::length(src_2.uv) - (rim * 0.97)) - 0.045, ((behind - 0.18) * rim) * TAU);
        float _e197 = accent_sd;
        accent = (((1.0 - metal::smoothstep(-(aa_7), aa_7, _e197)) * tail) * tail) * metal::sin(p_1 * 3.1415927);
    }
    float _e207 = accent;
    if (_e207 > 0.0) {
        metal::float3 rgb = metal::mix(src_2.color.xyz, metal::float3(1.0), 0.5);
        float _e217 = accent;
        metal::float3 _e220 = ink.rgb;
        float _e221 = accent;
        ink.rgb = (rgb * _e217) + (_e220 * (1.0 - _e221));
        float _e227 = accent;
        float _e229 = ink.alpha;
        float _e230 = accent;
        ink.alpha = _e227 + (_e229 * (1.0 - _e230));
        float _e237 = ink.mask;
        float _e238 = accent;
        ink.mask = metal::max(_e237, _e238);
        float _e240 = accent;
        if (_e240 >= 0.5) {
            float _e245 = ink.sd;
            float _e246 = accent_sd;
            ink.sd = metal::min(_e245, _e246);
        }
    }
    metal::float2 _e248 = spectral_radii(u);
    float _e249 = transition_scale(phase_1, u);
    float audio_aa = aa_7 * metal::max(_e249, 0.05);
    NodeLayer _e259 = glyph_band(metal::length(src_2.uv), _e248.x, _e248.y, 1.0, audio_aa);
    RingInk _e260 = spectral_ring(src_2, oct_2, src_2.uv, _e259, audio_aa, analytic_4, u);
    float _e265 = ink.lit;
    float _e267 = ink.alpha;
    float lit = (_e260.lit * _e260.cov) + ((_e265 * _e267) * (1.0 - _e260.cov));
    metal::float3 _e279 = ink.rgb;
    ink.rgb = (_e260.color * _e260.cov) + (_e279 * (1.0 - _e260.cov));
    float _e288 = ink.alpha;
    ink.alpha = _e260.cov + (_e288 * (1.0 - _e260.cov));
    float _e296 = ink.alpha;
    ink.lit = lit / metal::max(_e296, 0.0001);
    float _e303 = mask_level(src_2.ring);
    float mask = _e260.layer.coverage * _e303;
    float _e307 = ink.mask;
    ink.mask = mask + (_e307 * (1.0 - mask));
    float _e314 = ink.sd;
    float _e316 = layer_distance(_e314, _e260.layer);
    ink.sd = _e316;
    NodeInk _e317 = ink;
    return _e317;
}

struct fs_node_cellInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float4 params [[user(loc2), center_perspective]];
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
struct fs_node_cellOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_node_cellOutput fs_node_cell(
  fs_node_cellInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, constant Uniforms& u [[buffer(0)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.color, varyings.params, varyings.octaves, varyings.cents, varyings.strip_row, {}, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.ring, varyings.ink_carry, {}, varyings.shadow_box, varyings.shadow_at };
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
    bool analytic_5 = in.shadow_at.z < 0.0;
    NodeGeom _e21 = node_geom(in, analytic_5, u);
    if (!(_e21.paints)) {
        return fs_node_cellOutput { metal::float4(0.0) };
    }
    NodeInk _e29 = node_ink(in, _e21.d, _e21.aa, _e21.oct, analytic_5, u);
    if (analytic_5) {
        float points = metal::clamp(_e29.sd * metal::abs(in.shadow_at.z), -65504.0, EMPTY_DISTANCE);
        float stable = metal::rint(points * 32.0) / 32.0;
        return fs_node_cellOutput { metal::float4(stable, 0.0, 0.0, 0.0) };
    }
    return fs_node_cellOutput { metal::float4((_e29.alpha < INK_FLOOR) ? 0.0 : _e29.alpha, 0.0, 0.0, 0.0) };
}
