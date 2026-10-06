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
    float edge_softness_pixels;
    float padding;
    metal::float2 padding2_;
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
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
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
struct TextureParams {
    float depth;
    float scale;
    metal::float2 drift;
    metal::float2 target_size;
    metal::float2 glow_size;
};
struct PickupParams {
    float intensity;
    float width;
    float softness;
    float color;
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
struct type_7 {
    metal::float4 inner[64];
};
struct type_9 {
    metal::uint4 inner[240];
};
struct type_10 {
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
    TextureParams texture;
    PickupParams pickup;
    ShadowParams geometry_shadow;
    ShadowParams marker_shadow;
    ShadowTargetParams shadow_target;
    MarkerCellParams marker_cell;
    metal::float4 lattice_ground;
    metal::float4 lut_spacing;
    type_7 pitch_lut;
    type_7 spectral_lut;
    type_9 spectrum_color;
    type_10 ink_kernel;
};
struct OctRing {
    int base;
    float seam;
};
struct SectorFold {
    metal::float2 q;
    metal::float2 e;
};
struct PickupOut {
    metal::float4 position;
    metal::float2 uv;
    float level;
    char _pad3[4];
    metal::packed_uint3 octaves;
    float cents;
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
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant bool EARLY_OUT = true;
constant float GAUSSIAN_SPREAD_KIND = 3.0;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant float THICKNESS_STEPS = 64.0;
constant uint MAX_SPAN = 11u;
constant uint PITCH_LUT_N = 64u;
constant float BEND_ROUNDING = 0.4;
constant uint SPECTRUM_BUCKETS = 3828u;
constant float BUCKETS_PER_SEMITONE = 32.0;
constant float SPECTRUM_MIN_MIDI = 15.48682;
constant float OCT_UP = -4.712389;
constant float DISTANCE_LEVEL_FLOOR = 0.5;
constant float EMPTY_DISTANCE = 65504.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);
constant metal::float2 NEBULA_DETAIL_FADE = metal::float2(0.3, 1.1);

float node_rim(
    bool marked,
    constant Uniforms& u
) {
    float rim = {};
    bool local = {};
    float _e4 = u.node.rings_outer;
    rim = metal::max(_e4, 0.0);
    if (marked) {
        float _e13 = u.node.mark_thickness;
        local = _e13 > 0.0;
    } else {
        local = false;
    }
    bool _e17 = local;
    if (_e17) {
        float _e18 = rim;
        float _e22 = u.node.mark_inner;
        float _e26 = u.node.mark_thickness;
        rim = metal::max(_e18, _e22 + _e26);
    }
    float _e29 = rim;
    return _e29;
}

uint naga_div(uint lhs, uint rhs) {
    return lhs / metal::select(rhs, 1u, rhs == 0u);
}

uint naga_mod(uint lhs, uint rhs) {
    return lhs % metal::select(rhs, 1u, rhs == 0u);
}

uint slot_byte(
    metal::uint3 words,
    uint i_1
) {
    return (words[metal::min(unsigned(naga_div(i_1, 4u)), 2u)] >> (naga_mod(i_1, 4u) * 8u)) & 255u;
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

float oct_walk(
    float x,
    constant Uniforms& u
) {
    uint _e1 = oct_span(u);
    float span = static_cast<float>(_e1);
    return (TAU * metal::clamp(x, 0.0, span)) / span;
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
    int span_1 = static_cast<int>(_e3);
    float _e5 = oct_center(u);
    float nearest = metal::floor(((_e5 - off) / 12.0) + 0.5);
    float _e15 = oct_center(u);
    float d = ((nearest * 12.0) + off) - _e15;
    low = naga_div(naga_neg(as_type<int>(as_type<uint>(span_1) - as_type<uint>(1))), 2);
    if (naga_mod(span_1, 2) == 0) {
        low = (d < 0.0) ? as_type<int>(as_type<uint>(1) - as_type<uint>(naga_div(span_1, 2))) : naga_div(naga_neg(span_1), 2);
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
    float _e13 = oct_walk(static_cast<float>(i_3), u);
    float _e19 = oct_walk(static_cast<float>(i_3 + 1u), u);
    return metal::float2(ring_1.seam - _e13, ring_1.seam - _e19);
}

float oct_slot_level(
    metal::uint3 octaves_1,
    int s_2
) {
    bool local_1 = {};
    if (!((s_2 < 0))) {
        local_1 = s_2 >= 11;
    } else {
        local_1 = true;
    }
    bool _e10 = local_1;
    if (_e10) {
        return 0.0;
    }
    float _e13 = octave_level(octaves_1, static_cast<uint>(s_2));
    return _e13;
}

float lut_position(
    float t,
    metal::float2 corner
) {
    float w = {};
    float x_1 = corner.x;
    float y = corner.y;
    if (x_1 == y) {
        return t;
    }
    float low_1 = y / x_1;
    float high = (1.0 - y) / (1.0 - x_1);
    float x0_ = x_1 * 0.6;
    float x2_ = x_1 + (BEND_ROUNDING * (1.0 - x_1));
    if (t <= x0_) {
        w = low_1 * t;
    } else {
        if (t >= x2_) {
            w = y + (high * (t - x_1));
        } else {
            float y0_ = low_1 * x0_;
            float y2_ = y + (high * (x2_ - x_1));
            float a = (x0_ - (2.0 * x_1)) + x2_;
            float b = 2.0 * (x_1 - x0_);
            float c = x0_ - t;
            float s_3 = (-2.0 * c) / (b + metal::sqrt(metal::max((b * b) - ((4.0 * a) * c), 0.0)));
            w = ((((1.0 - s_3) * (1.0 - s_3)) * y0_) + (((2.0 * s_3) * (1.0 - s_3)) * y)) + ((s_3 * s_3) * y2_);
        }
    }
    float _e65 = w;
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
    float f_1 = _e25 * 63.0;
    uint i0_ = naga_f2u32(metal::floor(f_1));
    uint i1_ = metal::min(i0_ + 1u, 63u);
    metal::float4 _e37 = u.pitch_lut.inner[metal::min(unsigned(i0_), 63u)];
    metal::float4 _e42 = u.pitch_lut.inner[metal::min(unsigned(i1_), 63u)];
    return metal::mix(_e37.xyz, _e42.xyz, f_1 - metal::floor(f_1));
}

metal::float4 oct_slot_lit(
    float cents_2,
    metal::uint3 octaves_2,
    int slot,
    constant Uniforms& u
) {
    float _e3 = oct_slot_pitch(slot, cents_2);
    metal::float3 _e4 = pitch_lut_color(_e3, u);
    float _e5 = oct_slot_level(octaves_2, slot);
    return metal::float4(_e4, _e5);
}

SectorFold sector_fold(
    metal::float2 uv,
    metal::float2 edges
) {
    float mid = 0.5 * (edges.x + edges.y);
    float half_ = metal::clamp(0.5 * (edges.x - edges.y), 0.0, 3.1415927);
    float c_1 = metal::cos(mid);
    float s_4 = metal::sin(mid);
    return SectorFold {metal::float2(metal::abs((uv.y * c_1) - (uv.x * s_4)), (uv.x * c_1) + (uv.y * s_4)), metal::float2(metal::sin(half_), metal::cos(half_))};
}

float sector_side(
    SectorFold f
) {
    return (f.e.y * f.q.x) - (f.e.x * f.q.y);
}

float pickup_arc_distance(
    metal::float2 p,
    metal::float2 edges_1,
    float radius
) {
    SectorFold _e3 = sector_fold(p, edges_1);
    float _e4 = sector_side(_e3);
    if (_e4 <= 0.0) {
        return metal::abs(metal::length(_e3.q) - radius);
    }
    return metal::length(_e3.q - (radius * _e3.e));
}

metal::float3 pickup_bloom_color(
    metal::float4 lit,
    constant Uniforms& u
) {
    float lum = metal::dot(lit.xyz * lit.w, metal::float3(0.2126, 0.7152, 0.0722));
    float keep = metal::smoothstep(0.1, 0.6, lum);
    float _e16 = u.composite.bloom_strength;
    return metal::min(lit.xyz * (1.0 + (keep * _e16)), metal::float3(1.0));
}

struct fs_source_shadowInput {
    metal::float2 uv [[user(loc0), center_perspective]];
    float level [[user(loc1), flat]];
    metal::uint3 octaves [[user(loc2), flat]];
    float cents [[user(loc3), flat]];
};
struct fs_source_shadowOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_source_shadowOutput fs_source_shadow(
  fs_source_shadowInput varyings [[stage_in]]
, metal::float4 position [[position]]
, constant Uniforms& u [[buffer(0)]]
) {
    const PickupOut in = { position, varyings.uv, varyings.level, {}, varyings.octaves, varyings.cents };
    metal::float4 pigment = metal::float4(0.0);
    float weight = 0.0;
    float coverage = 0.0;
    uint i = 0u;
    float _e4 = u.pickup.width;
    float half_width = (0.5 * _e4) / 1.8;
    float _e11 = metal::fwidth(in.uv.x);
    float aa = metal::max(_e11, 0.00001);
    float _e17 = u.pickup.softness;
    float feather = metal::max(_e17 / 1.8, aa);
    float _e24 = node_rim(false, u);
    if (metal::abs(metal::length(in.uv) - _e24) > (half_width + feather)) {
        metal::discard_fragment();
    }
    OctRing _e30 = oct_ring(in.cents, u);
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e72 = i;
            i = _e72 + 1u;
        }
        loop_init = false;
        uint _e40 = i;
        uint _e41 = oct_span(u);
        if (_e40 < _e41) {
        } else {
            break;
        }
        {
            uint _e44 = i;
            int slot_1 = as_type<int>(as_type<uint>(_e30.base) + as_type<uint>(static_cast<int>(_e44)));
            metal::float2 _e48 = oct_sector(slot_1, _e30, u);
            float _e50 = node_rim(false, u);
            float _e51 = pickup_arc_distance(in.uv, _e48, _e50);
            float arc = 1.0 - metal::smoothstep(half_width - aa, half_width + feather, _e51);
            metal::float4 _e59 = oct_slot_lit(in.cents, in.octaves, slot_1, u);
            metal::float4 _e60 = pigment;
            metal::float3 _e61 = pickup_bloom_color(_e59, u);
            pigment = _e60 + (metal::float4(_e61 * _e59.w, _e59.w) * arc);
            float _e68 = weight;
            weight = _e68 + arc;
            float _e70 = coverage;
            coverage = metal::max(_e70, arc);
        }
    }
    metal::float4 _e75 = pigment;
    float _e76 = weight;
    pigment = _e75 / metal::float4(metal::max(_e76, 0.00001));
    float _e81 = coverage;
    float amount = _e81 * in.level;
    float _e87 = u.pickup.intensity;
    float _e89 = pigment.w;
    float _e96 = u.pickup.color;
    float _e98 = pigment.w;
    float opacity = (_e87 * (1.0 - _e89)) + (_e96 * _e98);
    metal::float4 _e101 = pigment;
    float _e106 = u.pickup.color;
    return fs_source_shadowOutput { metal::float4((_e101.xyz * _e106) * amount, opacity * amount) };
}
