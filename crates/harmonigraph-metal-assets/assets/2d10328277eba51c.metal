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
    float padding;
};
struct MarkerParams {
    float half_width;
    float taper_start;
    float world_unit;
    float padding;
};
struct type_10 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_10 bounds;
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
    float padding;
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
struct type_11 {
    metal::float4 inner[64];
};
struct type_13 {
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
    type_11 pitch_lut;
    type_11 spectral_lut;
    type_13 spectrum_color;
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

float aa_inside(
    float edge,
    float x,
    float w
) {
    return 1.0 - metal::smoothstep(edge - w, edge + w, x);
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
    float d = ((nearest * 12.0) + off) - _e15;
    low = naga_div(naga_neg(as_type<int>(as_type<uint>(span) - as_type<uint>(1))), 2);
    if (naga_mod(span, 2) == 0) {
        low = (d < 0.0) ? as_type<int>(as_type<uint>(1) - as_type<uint>(naga_div(span, 2))) : naga_div(naga_neg(span), 2);
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
    uint i_3 = static_cast<uint>(metal::clamp(as_type<int>(as_type<uint>(s_1) - as_type<uint>(ring_1.base)), 0, as_type<int>(as_type<uint>(static_cast<int>(_e4)) - as_type<uint>(1))));
    float _e12 = oct_bound(i_3, u);
    float _e17 = oct_bound(i_3 + 1u, u);
    return metal::float2(ring_1.seam - _e12, ring_1.seam - _e17);
}

float oct_slot_level(
    metal::uint3 octaves_1,
    int s_2
) {
    bool local = {};
    if (!((s_2 < 0))) {
        local = s_2 >= 11;
    } else {
        local = true;
    }
    bool _e10 = local;
    if (_e10) {
        return 0.0;
    }
    float _e13 = octave_level(octaves_1, static_cast<uint>(s_2));
    return _e13;
}

float oct_arc_coverage(
    metal::float2 edges,
    metal::float2 uv,
    float aa
) {
    metal::float2 b1_ = metal::float2(metal::cos(edges.x), metal::sin(edges.x));
    metal::float2 b2_ = metal::float2(metal::cos(edges.y), metal::sin(edges.y));
    float c1_ = (uv.x * b1_.y) - (uv.y * b1_.x);
    float c2_ = (uv.x * b2_.y) - (uv.y * b2_.x);
    float s1_ = metal::smoothstep(-(aa), aa, c1_);
    float s2_ = metal::smoothstep(-(aa), aa, -(c2_));
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
    VsOut in_1,
    int slot,
    constant Uniforms& u
) {
    float _e3 = oct_slot_pitch(slot, in_1.cents);
    metal::float3 _e4 = pitch_lut_color(_e3, u);
    float _e6 = oct_slot_level(in_1.octaves, slot);
    return metal::float4(_e4, _e6);
}

SectorFold sector_fold(
    metal::float2 uv_1,
    metal::float2 edges_1
) {
    float mid = 0.5 * (edges_1.x + edges_1.y);
    float half_ = metal::clamp(0.5 * (edges_1.x - edges_1.y), 0.0, 3.1415927);
    float c_1 = metal::cos(mid);
    float s_4 = metal::sin(mid);
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
    float inner,
    float outer,
    float gap
) {
    float distance = {};
    bool local_1 = {};
    bool local_2 = {};
    if (outer <= inner) {
        return EMPTY_DISTANCE;
    }
    metal::float2 normal = metal::float2(f_2.e.y, -(f_2.e.x));
    float radius = metal::length(f_2.q);
    float axis_t = (gap * f_2.e.y) / metal::max(f_2.e.x, 0.000001);
    float inner_t = metal::sqrt(metal::max((inner * inner) - (gap * gap), 0.0));
    float outer_t = metal::sqrt(metal::max((outer * outer) - (gap * gap), 0.0));
    float segment_start = metal::max(axis_t, inner_t);
    if (segment_start > outer_t) {
        return EMPTY_DISTANCE;
    }
    float side_t = metal::clamp(metal::dot(f_2.q, f_2.e), segment_start, outer_t);
    distance = metal::length(f_2.q - ((side_t * f_2.e) - (gap * normal)));
    metal::float2 direction = f_2.q / metal::float2(metal::max(radius, 0.000001));
    metal::float2 outer_at = direction * outer;
    if ((metal::dot(normal, outer_at) + gap) <= 0.0) {
        float _e59 = distance;
        distance = metal::min(_e59, metal::abs(radius - outer));
    }
    metal::float2 inner_at = direction * inner;
    if ((metal::dot(normal, inner_at) + gap) <= 0.0) {
        float _e68 = distance;
        distance = metal::min(_e68, metal::abs(radius - inner));
    }
    if (radius >= inner) {
        local_1 = radius <= outer;
    } else {
        local_1 = false;
    }
    bool _e77 = local_1;
    if (_e77) {
        local_2 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_2 = false;
    }
    bool inside = local_2;
    float _e87 = distance;
    float _e88 = distance;
    return inside ? -(_e88) : _e87;
}

NodeLayer outer_glyph(
    int s_3,
    OctRing ring_2,
    metal::float2 uv_2,
    NodeLayer band,
    float inner_1,
    float outer_1,
    float aa_1,
    constant Uniforms& u
) {
    float sd = {};
    metal::float2 _e7 = oct_sector(s_3, ring_2, u);
    SectorFold _e8 = sector_fold(uv_2, _e7);
    float _e9 = slice_gap_half(u);
    float gap_1 = (metal::dot(_e8.q, _e8.e) > 0.0) ? _e9 : 0.0;
    float _e17 = sector_side(_e8);
    float side = _e17 + gap_1;
    float _e19 = sector_pie(_e8, outer_1);
    float width = _e7.x - _e7.y;
    if (width > 3.1415927) {
        sd = metal::max(metal::max(band.sd, _e19), side);
    } else {
        float _e29 = slice_gap_half(u);
        float _e30 = annular_sector_distance(_e8, inner_1, outer_1, _e29);
        sd = _e30;
    }
    metal::float2 b1_1 = metal::float2(metal::cos(_e7.x), metal::sin(_e7.x));
    metal::float2 b2_1 = metal::float2(metal::cos(_e7.y), metal::sin(_e7.y));
    float c1_1 = (uv_2.x * b1_1.y) - (uv_2.y * b1_1.x);
    float c2_1 = (uv_2.x * b2_1.y) - (uv_2.y * b2_1.x);
    float _e55 = oct_arc_coverage(_e7, uv_2, aa_1);
    float _e56 = slice_gap_half(u);
    float _e58 = aa_inside(_e56, metal::abs(c1_1), aa_1);
    float _e66 = aa_inside(_e56, metal::abs(c2_1), aa_1);
    float gaps = (1.0 - (_e58 * metal::smoothstep(-(aa_1), aa_1, metal::dot(uv_2, b1_1)))) * (1.0 - (_e66 * metal::smoothstep(-(aa_1), aa_1, metal::dot(uv_2, b2_1))));
    float _e74 = sd;
    return NodeLayer {_e74, band.level, (band.coverage * _e55) * gaps};
}

NodeLayer mark_extension(
    uint slots,
    OctRing ring_3,
    metal::float2 uv_3,
    NodeLayer strip,
    float inner_2,
    float outer_2,
    float aa_2,
    bool analytic,
    constant Uniforms& u
) {
    bool local_3 = {};
    bool local_4 = {};
    float sd_1 = EMPTY_DISTANCE;
    float coverage = 0.0;
    uint i_1 = 0u;
    bool local_5 = {};
    bool local_6 = {};
    if (EARLY_OUT) {
        local_3 = !(analytic);
    } else {
        local_3 = false;
    }
    bool _e13 = local_3;
    if (_e13) {
        float _e16 = layer_coverage(strip);
        local_4 = _e16 <= 0.0;
    } else {
        local_4 = false;
    }
    bool _e20 = local_4;
    if (_e20) {
        return NodeLayer {EMPTY_DISTANCE, strip.level, 0.0};
    }
    uint _e26 = oct_span(u);
    int top = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_3.base) + as_type<uint>(static_cast<int>(_e26)))) - as_type<uint>(1));
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e66 = i_1;
            i_1 = _e66 + 1u;
        }
        loop_init = false;
        uint _e37 = i_1;
        if (_e37 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e40 = i_1;
            int s_5 = static_cast<int>(_e40);
            uint _e43 = i_1;
            if ((slots & (1u << _e43)) != 0u) {
                local_5 = s_5 >= ring_3.base;
            } else {
                local_5 = false;
            }
            bool _e53 = local_5;
            if (_e53) {
                local_6 = s_5 <= top;
            } else {
                local_6 = false;
            }
            bool _e58 = local_6;
            if (_e58) {
                NodeLayer _e59 = outer_glyph(s_5, ring_3, uv_3, strip, inner_2, outer_2, aa_2, u);
                float _e60 = sd_1;
                sd_1 = metal::min(_e60, _e59.sd);
                float _e63 = coverage;
                coverage = metal::max(_e63, _e59.coverage);
            }
        }
    }
    float _e69 = sd_1;
    float _e71 = coverage;
    return NodeLayer {_e69, strip.level, _e71};
}

float ink_arc(
    float r_1
) {
    return (r_1 * TAU) / 64.0;
}

metal::float4 ink_at(
    VsOut in_2,
    OctRing oct,
    float angle,
    constant Uniforms& u
) {
    metal::float3 rgb = metal::float3(0.0);
    float wsum = 0.0;
    bool local_7 = {};
    float cov = 0.0;
    int owner = {};
    uint i_2 = 0u;
    bool local_8 = {};
    bool local_9 = {};
    metal::float2 dir = metal::float2(metal::cos(angle), metal::sin(angle));
    float band_in = u.node.band_inner;
    float band_out = u.node.band_outer;
    if (band_out > band_in) {
        local_7 = in_2.params.x > 0.0;
    } else {
        local_7 = false;
    }
    bool _e27 = local_7;
    if (_e27) {
        float mid_1 = 0.5 * (band_in + band_out);
        metal::float2 p = dir * mid_1;
        float _e32 = ink_arc(mid_1);
        owner = oct.base;
        uint2 loop_bound_1 = uint2(4294967295u);
        bool loop_init_1 = true;
        while(true) {
            if (metal::all(loop_bound_1 == uint2(0u))) { break; }
            loop_bound_1 -= uint2(loop_bound_1.y == 0u, 1u);
            if (!loop_init_1) {
                uint _e55 = i_2;
                i_2 = _e55 + 1u;
            }
            loop_init_1 = false;
            uint _e39 = i_2;
            uint _e40 = oct_span(u);
            if (_e39 < _e40) {
            } else {
                break;
            }
            {
                uint _e43 = i_2;
                int slot_1 = as_type<int>(as_type<uint>(oct.base) + as_type<uint>(static_cast<int>(_e43)));
                NodeLayer _e51 = outer_glyph(slot_1, oct, p, NodeLayer {-65504.0, 1.0, 1.0}, band_in, EMPTY_DISTANCE, _e32, u);
                float _e52 = layer_coverage(_e51);
                float _e53 = cov;
                if (_e52 > _e53) {
                    cov = _e52;
                    owner = slot_1;
                }
            }
        }
        float _e58 = cov;
        if (_e58 > 0.0) {
            int _e61 = owner;
            metal::float4 _e62 = oct_slot_lit(in_2, _e61, u);
            float _e63 = cov;
            float w_1 = (_e63 * _e62.w) * (band_out - band_in);
            metal::float3 _e68 = rgb;
            rgb = _e68 + (_e62.xyz * w_1);
            float _e72 = wsum;
            wsum = _e72 + w_1;
        }
    }
    float mark_thick = u.node.mark_thickness;
    if (mark_thick > 0.0) {
        local_8 = (in_2.marks.x | in_2.marks.y) != 0u;
    } else {
        local_8 = false;
    }
    bool _e90 = local_8;
    if (_e90) {
        float _e95 = u.node.mark_inner;
        float mark_in = metal::min(_e95, 1.58);
        float mark_out = metal::min(mark_in + mark_thick, 1.58);
        float mid_2 = 0.5 * (mark_in + mark_out);
        metal::float2 p_1 = dir * mid_2;
        float _e103 = ink_arc(mid_2);
        NodeLayer _e116 = mark_extension(in_2.marks.x, oct, p_1, NodeLayer {-65504.0, metal::clamp(in_2.params.y, 0.0, 1.0), 1.0}, mark_in, EMPTY_DISTANCE, _e103, false, u);
        float _e117 = layer_coverage(_e116);
        NodeLayer _e130 = mark_extension(in_2.marks.y, oct, p_1, NodeLayer {-65504.0, metal::clamp(in_2.params.z, 0.0, 1.0), 1.0}, mark_in, EMPTY_DISTANCE, _e103, false, u);
        float _e131 = layer_coverage(_e130);
        float cov_1 = metal::max(_e117, _e131);
        if (cov_1 > 0.0) {
            local_9 = mark_out > mark_in;
        } else {
            local_9 = false;
        }
        bool _e139 = local_9;
        if (_e139) {
            float w_2 = cov_1 * (mark_out - mark_in);
            metal::float3 _e142 = rgb;
            rgb = _e142 + (((_e117 > _e131) ? in_2.melody_color.xyz : in_2.bass_color.xyz) * w_2);
            float _e151 = wsum;
            wsum = _e151 + w_2;
        }
    }
    metal::float3 _e153 = rgb;
    float _e154 = wsum;
    return metal::float4(_e153, _e154);
}

struct fs_ink_stripInput {
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
struct fs_ink_stripOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_ink_stripOutput fs_ink_strip(
  fs_ink_stripInput varyings [[stage_in]]
, metal::float4 clip_pos [[position]]
, constant Uniforms& u [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> ink_strip [[texture(0)]]
) {
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.color, varyings.params, varyings.octaves, varyings.cents, varyings.strip_row, {}, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.ring, varyings.ink_carry, {}, varyings.shadow_box, varyings.shadow_at };
    OctRing _e2 = oct_ring(in.cents, u);
    metal::float4 _e7 = ink_at(in, _e2, in.uv.x * TAU, u);
    float carry = metal::clamp(in.ink_carry, 0.0, 1.0);
    if (carry >= 1.0) {
        return fs_ink_stripOutput { _e7 };
    }
    uint clamped_lod_e22 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
    metal::float4 held = ink_strip.read(metal::min(metal::uint2(metal::int2(naga_f2i32(in.clip_pos.x), naga_f2i32(in.strip_row))), metal::uint2(ink_strip.get_width(clamped_lod_e22), ink_strip.get_height(clamped_lod_e22)) - 1), clamped_lod_e22);
    return fs_ink_stripOutput { metal::mix(held, _e7, carry) };
}
