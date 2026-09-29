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
    metal::float4 transmission;
};
struct CompositeParams {
    float darkest_pitch;
    float brightest_pitch;
    float render_scale;
    float bloom_strength;
    metal::float4 background;
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
    uint material_style;
    float material_roughness;
    float quiet_visibility;
    float guide_width;
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
    uint style;
    float padding;
};
struct MaterialParams {
    float amount;
    float scale;
    metal::float2 drift;
    uint style;
    float fuzz;
    float lobe;
    float variety;
    float refract;
    float layers;
    float randomness;
    float padding;
    metal::float4 velvet;
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
    TextureParams texture;
    MaterialParams material;
    PickupParams pickup;
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
constant float GLOW_BASE = 0.8;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
constant float SHADOW_REACH_SIGMAS = 3.0;
constant uint INK_STRIP_N = 64u;
constant bool EARLY_OUT = true;
constant float INK_FLOOR = 0.01;
constant uint OCTAVE_SLOTS = 11u;
constant float THICKNESS_STEPS = 64.0;
constant float GAUSSIAN_SPREAD_KIND = 3.0;
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
    uint words_1 = 1 + (_buffer_sizes.size5 - 0 - 4) / 4;
    if (!((receiver >= casters))) {
        local_1 = words_1 < OCCLUDER_HEADER;
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
    uint slot_3 = (OCCLUDER_HEADER + (naga_f2u32(bin.y) * columns)) + naga_f2u32(bin.x);
    if ((slot_3 + 1u) >= words_1) {
        return 1.0;
    }
    uint _e83 = node_occluders[metal::min(unsigned(slot_3 + 1u), (_buffer_sizes.size5 - 0 - 4) / 4)];
    uint end = metal::min(_e83, words_1);
    uint _e89 = node_occluders[metal::min(unsigned(slot_3), (_buffer_sizes.size5 - 0 - 4) / 4)];
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
    float coverage_3 = metal::clamp(level_1, 0.0, 1.0) * _e14;
    float _e16 = glow_shadow_depth(u);
    return ShadowThrough {1.0 - (_e16 * coverage_3), 1.0 - coverage_3};
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

float pigment_fringe(
    constant Uniforms& u
) {
    float _e3 = u.node.material_roughness;
    uint _e9 = u.node.material_style;
    return (_e9 == 0u) ? 0.0 : (0.3 * _e3);
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
    uint i_8 = static_cast<uint>(metal::clamp(as_type<int>(as_type<uint>(s_1) - as_type<uint>(ring_1.base)), 0, as_type<int>(as_type<uint>(static_cast<int>(_e4)) - as_type<uint>(1))));
    float _e12 = oct_bound(i_8, u);
    float _e17 = oct_bound(i_8 + 1u, u);
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
    VsOut in_2,
    constant Uniforms& u
) {
    if (in_2.params.w > 2.5) {
        float _e10 = aa_width(in_2.strip_row, in_2.shadow_at.w, u);
        float _e17 = aa_inside(in_2.ink_carry, layer_1.sd, _e10);
        float coverage_4 = metal::clamp(layer_1.level, 0.0, 1.0) * _e17;
        return metal::min(field, -(coverage_4));
    }
    if (in_2.params.w > 1.5) {
        float _e38 = standoff_coverage(layer_1.sd * metal::abs(in_2.shadow_at.z), 2.0 * in_2.strip_row, in_2.ink_carry);
        float coverage_5 = metal::clamp(layer_1.level, 0.0, 1.0) * _e38;
        return metal::min(field, -(coverage_5));
    }
    return (layer_1.level >= DISTANCE_LEVEL_FLOOR) ? metal::min(field, layer_1.sd) : field;
}

NodeLayer glyph_band(
    float d_1,
    float inner_1,
    float outer_2,
    float level_2,
    float aa_2
) {
    float mid = 0.5 * (inner_1 + outer_2);
    float _e14 = aa_inside(outer_2, d_1, aa_2);
    float _e15 = aa_inside(inner_1, d_1, aa_2);
    return NodeLayer {metal::abs(d_1 - mid) - (0.5 * (outer_2 - inner_1)), level_2, _e14 * (1.0 - _e15)};
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
    float low_3 = y / x_2;
    float high_2 = (1.0 - y) / (1.0 - x_2);
    float x0_ = x_2 * 0.6;
    float x2_ = x_2 + (BEND_ROUNDING * (1.0 - x_2));
    if (t_1 <= x0_) {
        w_2 = low_3 * t_1;
    } else {
        if (t_1 >= x2_) {
            w_2 = y + (high_2 * (t_1 - x_2));
        } else {
            float y0_ = low_3 * x0_;
            float y2_ = y + (high_2 * (x2_ - x_2));
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

metal::float4 oct_slot_ink(
    VsOut in_3,
    int slot_1,
    constant Uniforms& u
) {
    float presence = in_3.params.x;
    metal::float4 _e6 = oct_slot_lit(in_3.cents, in_3.octaves, slot_1, u);
    float level_6 = _e6.w;
    if (level_6 <= 0.0) {
        metal::float4 _e12 = u.lattice_ground;
        return metal::float4(_e12.xyz, presence);
    }
    float ghost_rest = metal::max(presence - level_6, 0.0);
    float opacity = level_6 + ghost_rest;
    metal::float4 _e21 = u.lattice_ground;
    metal::float3 ground = _e21.xyz;
    return metal::float4(((_e6.xyz * level_6) + (ground * ghost_rest)) / metal::float3(opacity), opacity);
}

float slice_reach(
    VsOut in_4,
    int s_4,
    float inner_2,
    float outer_3,
    constant Uniforms& u
) {
    bool local_8 = {};
    if (!((s_4 < 0))) {
        local_8 = s_4 >= 11;
    } else {
        local_8 = true;
    }
    bool _e12 = local_8;
    if (_e12) {
        return outer_3;
    }
    float _e15 = slice_thickness(in_4.thickness, static_cast<uint>(s_4));
    float _e16 = reach_at(_e15, inner_2, outer_3, u);
    return _e16;
}

metal::float2 mark_radii(
    VsOut in_5,
    int s_5,
    float inner_3,
    float outer_4,
    constant Uniforms& u
) {
    bool local_9 = {};
    float band_in = u.node.band_inner;
    float band_out = u.node.band_outer;
    float _e12 = slice_reach(in_5, s_5, band_in, band_out, u);
    if (!((band_out <= band_in))) {
        local_9 = _e12 == band_out;
    } else {
        local_9 = true;
    }
    bool _e19 = local_9;
    if (_e19) {
        return metal::float2(inner_3, outer_4);
    }
    float _e25 = u.node.mark_inner;
    float start = metal::min(_e25 + (_e12 - band_out), 1.58);
    float _e32 = u.node.mark_thickness;
    return metal::float2(start, metal::min(start + metal::max(_e32, 0.0), 1.58));
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
    metal::float2 uv_2,
    metal::float2 edges_1
) {
    float mid_1 = 0.5 * (edges_1.x + edges_1.y);
    float half_ = metal::clamp(0.5 * (edges_1.x - edges_1.y), 0.0, 3.1415927);
    float c_2 = metal::cos(mid_1);
    float s_9 = metal::sin(mid_1);
    return SectorFold {metal::float2(metal::abs((uv_2.y * c_2) - (uv_2.x * s_9)), (uv_2.x * c_2) + (uv_2.y * s_9)), metal::float2(metal::sin(half_), metal::cos(half_))};
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
    bool local_10 = {};
    bool local_11 = {};
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
        local_10 = radius <= outer_5;
    } else {
        local_10 = false;
    }
    bool _e77 = local_10;
    if (_e77) {
        local_11 = (metal::dot(normal, f_2.q) + gap) <= 0.0;
    } else {
        local_11 = false;
    }
    bool inside = local_11;
    float _e87 = distance;
    float _e88 = distance;
    return inside ? -(_e88) : _e87;
}

NodeLayer outer_glyph(
    int s_6,
    OctRing ring_3,
    metal::float2 uv_3,
    NodeLayer band,
    float inner_5,
    float outer_6,
    float aa_3,
    constant Uniforms& u
) {
    float sd = {};
    metal::float2 _e7 = oct_sector(s_6, ring_3, u);
    SectorFold _e8 = sector_fold(uv_3, _e7);
    float _e9 = slice_gap_half(u);
    float gap_1 = (metal::dot(_e8.q, _e8.e) > 0.0) ? _e9 : 0.0;
    float _e17 = sector_side(_e8);
    float side_1 = _e17 + gap_1;
    float _e19 = sector_pie(_e8, outer_6);
    float width = _e7.x - _e7.y;
    if (width > 3.1415927) {
        sd = metal::max(metal::max(band.sd, _e19), side_1);
    } else {
        float _e29 = slice_gap_half(u);
        float _e30 = annular_sector_distance(_e8, inner_5, outer_6, _e29);
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
    int s_7,
    OctRing ring_4,
    metal::float2 uv_4,
    float d_3,
    NodeLayer band_1,
    metal::float4 ink_1,
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
        NodeLayer _e19 = outer_glyph(s_7, ring_4, uv_4, _e18, inner_6, reach_1, aa_4, u);
        slice = _e19;
    }
    float _e21 = oct_slot_level(in_6.octaves, s_7);
    near = band_1;
    NodeLayer _e23 = slice;
    far = _e23;
    metal::float4 _e27 = oct_slot_lit(in_6.cents, in_6.octaves, s_7, u);
    far_rgb = _e27.xyz;
    far_level = _e21;
    if (reach_1 < outer_7) {
        NodeLayer _e32 = slice;
        near = _e32;
        far = band_1;
        metal::float4 _e35 = u.lattice_ground;
        far_rgb = _e35.xyz;
        far_level = metal::max(in_6.params.x - _e21, 0.0);
    }
    NodeLayer _e42 = near;
    float _e43 = layer_coverage(_e42);
    NodeLayer _e44 = far;
    float _e45 = layer_coverage(_e44);
    float rest = metal::max(_e45 - _e43, 0.0);
    float near_ink = _e43 * ink_1.w;
    float _e51 = far_level;
    float far_ink = rest * _e51;
    float cov_2 = near_ink + far_ink;
    metal::float3 _e56 = far_rgb;
    metal::float3 rgb_1 = ((ink_1.xyz * near_ink) + (_e56 * far_ink)) / metal::float3(metal::max(cov_2, 0.0001));
    NodeLayer _e64 = slice;
    float _e65 = layer_coverage(_e64);
    float _e67 = near.sd;
    float _e70 = near.coverage;
    float _e73 = far.sd;
    float _e74 = far_level;
    float _e76 = far.coverage;
    return SliceZones {metal::float4(rgb_1, cov_2), _e65, NodeLayer {_e67, ink_1.w, _e70}, NodeLayer {_e73, _e74, _e76}, rest};
}

float pigment_hash(
    metal::int2 q,
    uint seed
) {
    uint h = {};
    h = ((as_type<uint>(q.x) * 374761393u) + (as_type<uint>(q.y) * 668265263u)) + (seed * 144269u);
    uint _e15 = h;
    uint _e16 = h;
    h = (_e15 ^ (_e16 >> 13u)) * 1274126177u;
    uint _e22 = h;
    uint _e23 = h;
    return static_cast<float>(_e22 ^ (_e23 >> 16u)) / 4294967300.0;
}

metal::int2 naga_f2i32(metal::float2 value) {
    return static_cast<metal::int2>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

float pigment_noise(
    metal::float2 p,
    uint seed_1
) {
    metal::int2 cell_2 = naga_f2i32(metal::floor(p));
    metal::float2 f_4 = metal::fract(p);
    metal::float2 t_3 = (f_4 * f_4) * (metal::float2(3.0) - (2.0 * f_4));
    float _e12 = pigment_hash(cell_2, seed_1);
    float _e17 = pigment_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 0))), seed_1);
    float _e24 = pigment_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(0, 1))), seed_1);
    float _e29 = pigment_hash(as_type<metal::int2>(as_type<metal::uint2>(cell_2) + as_type<metal::uint2>(metal::int2(1, 1))), seed_1);
    return metal::mix(metal::mix(_e12, _e17, t_3.x), metal::mix(_e24, _e29, t_3.x), t_3.y);
}

float pigment_field(
    metal::float2 p_1,
    uint seed_2
) {
    float _e2 = pigment_noise(p_1, seed_2);
    float _e11 = pigment_noise((p_1 * 2.07) + metal::float2(13.1, -7.3), seed_2 + 31u);
    return (_e2 + (0.5 * _e11)) / 1.5;
}

NodeLayer pigment_layer(
    metal::float2 uv_5,
    float inner_7,
    float outer_8,
    float aa_5,
    float side,
    float drift,
    float fine,
    bool trace,
    float low_1,
    float high,
    constant Uniforms& u
) {
    float sd_1 = {};
    float coverage = {};
    if (outer_8 <= inner_7) {
        return NodeLayer {65504.0, 0.0, 0.0};
    }
    float d_8 = metal::length(uv_5);
    float rough = u.node.material_roughness;
    float _e29 = u.node.guide_width;
    float half_width = trace ? metal::min(0.5 * (outer_8 - inner_7), 0.04027778 * _e29) : (0.5 * (outer_8 - inner_7));
    float radial = metal::abs((d_8 - (0.5 * (inner_7 + outer_8))) - drift) - half_width;
    float soft = metal::max(aa_5 * 0.5, 0.023611112);
    float core_sd = radial - 0.0069444445;
    float limits = metal::max(low_1 - d_8, d_8 - high);
    sd_1 = metal::max(metal::max(core_sd, side), limits);
    coverage = 1.0 - metal::smoothstep(-(soft), soft, core_sd);
    if (trace) {
        float holes = (0.3 - fine) * 0.08;
        float _e62 = sd_1;
        sd_1 = metal::max(_e62, holes);
        float _e64 = coverage;
        coverage = _e64 * (metal::smoothstep(0.22, 0.38, fine) * (0.48 + (0.52 * fine)));
    } else {
        if (rough > 0.0) {
            float bleed = (1.0 - metal::smoothstep(0.0, 0.14722222 * rough, radial)) * (0.25 + (0.23 * fine));
            float _e87 = coverage;
            coverage = metal::max(_e87, bleed);
        }
    }
    float _e89 = coverage;
    coverage = _e89 * (1.0 - metal::smoothstep(-(metal::max(aa_5 * 0.5, 0.036111113)), 0.0, side));
    float _e100 = coverage;
    coverage = _e100 * (1.0 - metal::smoothstep(-(metal::max(aa_5 * 0.5, 0.01)), 0.0, limits));
    float _e111 = sd_1;
    float _e112 = coverage;
    return NodeLayer {_e111, 1.0, _e112};
}

SliceZones pigment_zones(
    VsOut in_7,
    int slot_2,
    OctRing ring_5,
    metal::float2 uv_6,
    float aa_6,
    constant Uniforms& u
) {
    bool local_12 = {};
    bool local_13 = {};
    bool local_14 = {};
    float low_2 = 0.0;
    bool local_15 = {};
    float high_1 = {};
    bool local_16 = {};
    bool local_17 = {};
    NodeLayer live = {};
    NodeLayer quiet = {};
    float inner_9 = u.node.band_inner;
    float outer_10 = u.node.band_outer;
    float _e13 = slice_reach(in_7, slot_2, inner_9, outer_10, u);
    metal::float2 _e14 = oct_sector(slot_2, ring_5, u);
    SectorFold _e15 = sector_fold(uv_6, _e14);
    float _e16 = sector_side(_e15);
    float _e17 = slice_gap_half(u);
    float _e22 = sector_pie(_e15, EMPTY_DISTANCE);
    float side_2 = metal::max(_e16 + metal::max(_e17, 0.019444445), _e22);
    if (in_7.shadow_at.z >= 0.0) {
        local_12 = in_7.params.w < 1.5;
    } else {
        local_12 = false;
    }
    bool _e35 = local_12;
    if (_e35) {
        if (!((side_2 >= 0.0))) {
            float _e45 = pigment_fringe(u);
            local_14 = metal::length(uv_6) > ((metal::max(outer_10, _e13) + _e45) + aa_6);
        } else {
            local_14 = true;
        }
        bool _e50 = local_14;
        local_13 = _e50;
    } else {
        local_13 = false;
    }
    bool _e52 = local_13;
    if (_e52) {
        NodeLayer empty = NodeLayer {65504.0, 0.0, 0.0};
        return SliceZones {metal::float4(0.0), 0.0, empty, empty, 0.0};
    }
    float _e63 = oct_slot_pitch(slot_2, in_7.cents);
    uint seed_3 = naga_f2u32(metal::max(metal::rint(_e63), 0.0));
    metal::float2 p_2 = metal::float2(uv_6.x, -(uv_6.y)) * 36.0;
    float _e79 = pigment_noise(p_2 / metal::float2(1.1), seed_3 + 209u);
    float _e85 = pigment_field(p_2 / metal::float2(4.4), seed_3 + 73u);
    float _e100 = u.node.material_roughness;
    float drift_1 = ((((_e85 - 0.5) * 9.5) + ((_e79 - 0.5) * 1.4)) / 36.0) * _e100;
    if (in_7.ring > 0.0) {
        float _e112 = u.spectral.outer;
        float _e116 = u.spectral.inner;
        local_15 = _e112 > _e116;
    } else {
        local_15 = false;
    }
    bool _e119 = local_15;
    if (_e119) {
        float _e123 = u.spectral.outer;
        float _e127 = u.node.angular_gap;
        low_2 = metal::min(inner_9, _e123 + (0.5 * _e127));
    }
    float _e134 = pigment_fringe(u);
    high_1 = metal::min(1.5600001, metal::max(outer_10, _e13) + _e134);
    if (slot_2 >= 0) {
        local_16 = slot_2 < 11;
    } else {
        local_16 = false;
    }
    bool _e145 = local_16;
    if (_e145) {
        float _e151 = u.node.mark_thickness;
        local_17 = _e151 > 0.0;
    } else {
        local_17 = false;
    }
    bool _e155 = local_17;
    if (_e155) {
        uint bit = 1u << static_cast<uint>(slot_2);
        float melody = ((in_7.marks.x & bit) != 0u) ? in_7.params.y : 0.0;
        float bass = ((in_7.marks.y & bit) != 0u) ? in_7.params.z : 0.0;
        float mark_level = metal::clamp(metal::max(melody, bass), 0.0, 1.0);
        if (mark_level > 0.0) {
            float _e186 = u.node.mark_inner;
            float _e190 = u.node.mark_inner;
            float _e194 = u.node.mark_thickness;
            metal::float2 _e196 = mark_radii(in_7, slot_2, _e186, _e190 + _e194, u);
            float _e197 = high_1;
            float _e198 = high_1;
            float _e203 = u.node.angular_gap;
            high_1 = metal::mix(_e197, metal::min(_e198, _e196.x - (0.5 * _e203)), mark_level);
        }
    }
    float _e210 = low_2;
    float _e211 = high_1;
    NodeLayer _e212 = pigment_layer(uv_6, inner_9, _e13, aa_6, side_2, drift_1, _e79, false, _e210, _e211, u);
    live = _e212;
    uint _e217 = u.node.material_style;
    float _e220 = low_2;
    float _e221 = high_1;
    NodeLayer _e222 = pigment_layer(uv_6, inner_9, outer_10, aa_6, side_2, drift_1, _e79, _e217 == 2u, _e220, _e221, u);
    quiet = _e222;
    metal::float4 _e226 = oct_slot_lit(in_7.cents, in_7.octaves, slot_2, u);
    live.level = _e226.w;
    float _e239 = u.node.quiet_visibility;
    quiet.level = metal::max(in_7.params.x - _e226.w, 0.0) * _e239;
    NodeLayer _e241 = live;
    float _e242 = layer_coverage(_e241);
    NodeLayer _e243 = quiet;
    float _e244 = layer_coverage(_e243);
    float coverage_6 = _e242 + _e244;
    float _e251 = pigment_field(p_2 / metal::float2(6.8), seed_3 + 11u);
    float _e264 = pigment_noise(p_2 / metal::float2(2.1), seed_3 + 119u);
    float density = (0.55 + (0.72 * metal::smoothstep(0.15, 0.8, _e251))) + (0.35 * (_e264 - 0.5));
    float quiet_density = 0.68 + (0.52 * _e85);
    metal::float4 _e276 = u.lattice_ground;
    metal::float3 ground_1 = _e276.xyz * quiet_density;
    metal::float3 rgb_2 = (((_e226.xyz * density) * _e242) + (ground_1 * _e244)) / metal::float3(metal::max(coverage_6, 0.0001));
    float _e290 = live.coverage;
    NodeLayer _e291 = live;
    NodeLayer _e292 = quiet;
    float _e294 = quiet.coverage;
    return SliceZones {metal::float4(rgb_2, coverage_6), _e290, _e291, _e292, _e294};
}

metal::float3 spectral_lut_color(
    float level_3,
    constant Uniforms& u
) {
    metal::float4 _e6 = u.lut_spacing;
    float _e8 = lut_position(metal::clamp(level_3, 0.0, 1.0), _e6.zw);
    float f_5 = _e8 * 63.0;
    uint i0_1 = naga_f2u32(metal::floor(f_5));
    uint i1_1 = metal::min(i0_1 + 1u, 63u);
    metal::float4 _e20 = u.spectral_lut.inner[metal::min(unsigned(i0_1), 63u)];
    metal::float4 _e25 = u.spectral_lut.inner[metal::min(unsigned(i1_1), 63u)];
    return metal::mix(_e20.xyz, _e25.xyz, f_5 - metal::floor(f_5));
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
    uint i_9 = metal::min(b, 3827u);
    uint word = u.spectrum_color.inner[metal::min(unsigned(naga_div(i_9, 16u)), 239u)][metal::min(unsigned(naga_mod(naga_div(i_9, 4u), 4u)), 3u)];
    return static_cast<float>((word >> (naga_mod(i_9, 4u) * 8u)) & 255u) / 255.0;
}

float spectrum_color_at(
    float pitch_1,
    constant Uniforms& u
) {
    bool local_18 = {};
    float x_3 = ((pitch_1 - SPECTRUM_MIN_MIDI) * BUCKETS_PER_SEMITONE) - 0.5;
    if (!((x_3 < 0.0))) {
        local_18 = x_3 > 3827.0;
    } else {
        local_18 = true;
    }
    bool _e15 = local_18;
    if (_e15) {
        return 0.0;
    }
    uint i_10 = naga_f2u32(metal::floor(x_3));
    float _e19 = spectrum_color_level(i_10, u);
    float _e22 = spectrum_color_level(i_10 + 1u, u);
    return metal::mix(_e19, _e22, x_3 - metal::floor(x_3));
}

float wedge_fraction(
    metal::float2 edges_2,
    metal::float2 uv_7
) {
    float mid_2 = 0.5 * (edges_2.x + edges_2.y);
    float half_1 = metal::max(0.5 * (edges_2.x - edges_2.y), 0.00001);
    float c_3 = (uv_7.x * metal::cos(mid_2)) + (uv_7.y * metal::sin(mid_2));
    float s_10 = (-(uv_7.x) * metal::sin(mid_2)) + (uv_7.y * metal::cos(mid_2));
    float delta = metal::atan2(s_10, c_3);
    return metal::clamp(0.5 - (delta / (2.0 * half_1)), 0.0, 1.0);
}

RingInk spectral_ring(
    VsOut in_8,
    OctRing oct,
    metal::float2 uv_8,
    NodeLayer band_2,
    float aa_7,
    bool analytic,
    constant Uniforms& u
) {
    bool local_19 = {};
    bool local_20 = {};
    float cov = 0.0;
    int owner = {};
    float sd_2 = EMPTY_DISTANCE;
    uint i_3 = 0u;
    float pitch_2 = {};
    metal::float2 _e6 = spectral_radii(u);
    if (_e6.y <= _e6.x) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (in_8.ring <= 0.0) {
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {65504.0, 0.0, 0.0}};
    }
    if (EARLY_OUT) {
        local_19 = !(analytic);
    } else {
        local_19 = false;
    }
    bool _e36 = local_19;
    if (_e36) {
        float _e39 = layer_coverage(band_2);
        local_20 = _e39 <= 0.0;
    } else {
        local_20 = false;
    }
    bool _e43 = local_20;
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
            uint _e77 = i_3;
            i_3 = _e77 + 1u;
        }
        loop_init_1 = false;
        uint _e61 = i_3;
        uint _e62 = oct_span(u);
        if (_e61 < _e62) {
        } else {
            break;
        }
        {
            uint _e65 = i_3;
            int slot_4 = as_type<int>(as_type<uint>(oct.base) + as_type<uint>(static_cast<int>(_e65)));
            NodeLayer _e70 = outer_glyph(slot_4, oct, uv_8, band_2, _e6.x, _e6.y, aa_7, u);
            float _e71 = layer_coverage(_e70);
            float _e72 = sd_2;
            sd_2 = metal::min(_e72, _e70.sd);
            float _e75 = cov;
            if (_e71 > _e75) {
                cov = _e71;
                owner = slot_4;
            }
        }
    }
    float _e80 = cov;
    if (_e80 <= 0.0) {
        float _e85 = sd_2;
        float _e87 = cov;
        return RingInk {metal::float3(0.0), 0.0, 0.0, NodeLayer {_e85, in_8.ring, _e87}};
    }
    int _e92 = owner;
    float _e94 = oct_slot_pitch(_e92, in_8.cents);
    pitch_2 = _e94;
    bool _e96 = folded(u);
    if (!(_e96)) {
        int _e98 = owner;
        metal::float2 _e99 = oct_sector(_e98, oct, u);
        float _e100 = wedge_fraction(_e99, uv_8);
        float _e101 = pitch_2;
        float _e107 = u.spectral.range_cents;
        pitch_2 = _e101 + (((_e100 - 0.5) * _e107) / 100.0);
    }
    float _e112 = pitch_2;
    float _e113 = spectrum_color_at(_e112, u);
    metal::float3 _e114 = spectral_lut_color(_e113, u);
    float _e115 = cov;
    float _e121 = sd_2;
    float _e123 = cov;
    return RingInk {_e114, _e115 * in_8.ring, metal::clamp(_e113, 0.0, 1.0), NodeLayer {_e121, in_8.ring, _e123}};
}

NodeLayer drawn_marks(
    VsOut in_9,
    uint slots,
    OctRing ring_6,
    metal::float2 uv_9,
    float d_4,
    NodeLayer strip,
    float inner_8,
    float outer_9,
    float aa_8,
    bool analytic_1,
    constant Uniforms& u
) {
    float sd_3 = EMPTY_DISTANCE;
    float coverage_1 = 0.0;
    uint i_4 = 0u;
    bool local_21 = {};
    bool local_22 = {};
    NodeLayer own = {};
    bool local_23 = {};
    bool local_24 = {};
    bool local_25 = {};
    uint _e11 = oct_span(u);
    int top = as_type<int>(as_type<uint>(as_type<int>(as_type<uint>(ring_6.base) + as_type<uint>(static_cast<int>(_e11)))) - as_type<uint>(1));
    uint2 loop_bound_2 = uint2(4294967295u);
    bool loop_init_2 = true;
    while(true) {
        if (metal::all(loop_bound_2 == uint2(0u))) { break; }
        loop_bound_2 -= uint2(loop_bound_2.y == 0u, 1u);
        if (!loop_init_2) {
            uint _e88 = i_4;
            i_4 = _e88 + 1u;
        }
        loop_init_2 = false;
        uint _e22 = i_4;
        if (_e22 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            uint _e25 = i_4;
            int s_11 = static_cast<int>(_e25);
            uint _e28 = i_4;
            if (!(((slots & (1u << _e28)) == 0u))) {
                local_21 = s_11 < ring_6.base;
            } else {
                local_21 = true;
            }
            bool _e39 = local_21;
            if (!(_e39)) {
                local_22 = s_11 > top;
            } else {
                local_22 = true;
            }
            bool _e45 = local_22;
            if (_e45) {
                continue;
            }
            metal::float2 _e46 = mark_radii(in_9, s_11, inner_8, outer_9, u);
            own = strip;
            if (!((_e46.x != inner_8))) {
                local_23 = _e46.y != outer_9;
            } else {
                local_23 = true;
            }
            bool _e56 = local_23;
            if (_e56) {
                if (_e46.y <= _e46.x) {
                    continue;
                }
                NodeLayer _e63 = glyph_band(d_4, _e46.x, _e46.y, 1.0, aa_8);
                own = _e63;
            }
            if (EARLY_OUT) {
                local_24 = !(analytic_1);
            } else {
                local_24 = false;
            }
            bool _e69 = local_24;
            if (_e69) {
                NodeLayer _e72 = own;
                float _e73 = layer_coverage(_e72);
                local_25 = _e73 <= 0.0;
            } else {
                local_25 = false;
            }
            bool _e77 = local_25;
            if (_e77) {
                continue;
            }
            NodeLayer _e78 = own;
            NodeLayer _e81 = outer_glyph(s_11, ring_6, uv_9, _e78, _e46.x, _e46.y, aa_8, u);
            float _e82 = sd_3;
            sd_3 = metal::min(_e82, _e81.sd);
            float _e85 = coverage_1;
            coverage_1 = metal::max(_e85, _e81.coverage);
        }
    }
    float _e91 = sd_3;
    float _e93 = coverage_1;
    return NodeLayer {_e91, strip.level, _e93};
}

NodeGeom node_geom(
    VsOut src,
    bool analytic_2,
    constant Uniforms& u
) {
    bool local_26 = {};
    bool local_27 = {};
    bool local_28 = {};
    bool local_29 = {};
    bool local_30 = {};
    bool local_31 = {};
    bool local_32 = {};
    bool local_33 = {};
    bool local_34 = {};
    bool local_35 = {};
    float d_9 = metal::length(src.uv);
    float _e6 = metal::fwidth(src.uv.x);
    float _e9 = aa_width(_e6, src.shadow_at.w, u);
    if (EARLY_OUT) {
        local_26 = !(analytic_2);
    } else {
        local_26 = false;
    }
    bool _e15 = local_26;
    if (_e15) {
        float _e18 = paint_reach(src, _e9, u);
        local_27 = d_9 > _e18;
    } else {
        local_27 = false;
    }
    bool _e21 = local_27;
    if (_e21) {
        return NodeGeom {d_9, _e9, OctRing {0, 0.0}, false};
    }
    metal::float2 _e27 = spectral_radii(u);
    bool ring_draws = _e27.y > _e27.x;
    if (ring_draws) {
        local_28 = metal::length(src.uv) >= (_e27.x - _e9);
    } else {
        local_28 = false;
    }
    bool _e39 = local_28;
    if (_e39) {
        local_29 = metal::length(src.uv) <= (_e27.y + _e9);
    } else {
        local_29 = false;
    }
    bool in_audio_ring = local_29;
    if (EARLY_OUT) {
        local_30 = !(analytic_2);
    } else {
        local_30 = false;
    }
    bool _e54 = local_30;
    if (_e54) {
        local_31 = !(in_audio_ring);
    } else {
        local_31 = false;
    }
    bool _e59 = local_31;
    if (_e59) {
        local_32 = src.params.x <= 0.0;
    } else {
        local_32 = false;
    }
    bool _e67 = local_32;
    if (_e67) {
        local_33 = src.params.y <= 0.0;
    } else {
        local_33 = false;
    }
    bool _e75 = local_33;
    if (_e75) {
        local_34 = src.params.z <= 0.0;
    } else {
        local_34 = false;
    }
    bool _e83 = local_34;
    if (_e83) {
        local_35 = ((src.octaves.x | src.octaves.y) | src.octaves.z) == 0u;
    } else {
        local_35 = false;
    }
    bool _e97 = local_35;
    if (_e97) {
        return NodeGeom {d_9, _e9, OctRing {0, 0.0}, false};
    }
    OctRing _e104 = oct_ring(src.cents, u);
    return NodeGeom {d_9, _e9, _e104, true};
}

float mask_level(
    float level_4
) {
    return metal::clamp(level_4 / INK_FLOOR, 0.0, 1.0);
}

NodeInk base_node_ink(
    VsOut in_10,
    float d_5,
    float aa_9,
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
    NodeLayer band_3 = NodeLayer {65504.0, 0.0, 0.0};
    bool local_36 = {};
    uint i_5 = 0u;
    bool local_37 = {};
    bool local_38 = {};
    bool local_39 = {};
    bool local_40 = {};
    bool local_41 = {};
    bool local_42 = {};
    bool local_43 = {};
    bool local_44 = {};
    metal::float3 slot_rgb = {};
    float cov_1 = {};
    float lit_shape = {};
    NodeLayer mark_strip = NodeLayer {65504.0, 0.0, 0.0};
    float mark = {};
    float mark_mask = {};
    float presence_1 = in_10.params.x;
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
        NodeLayer _e54 = glyph_band(d_5, band_in_1, band_out_1, 1.0, aa_9);
        band_3 = _e54;
    }
    float swell_out = in_10.swell;
    bool swells_1 = swell_out > band_out_1;
    if (swells_1) {
        local_36 = d_5 < (swell_out + aa_9);
    } else {
        local_36 = false;
    }
    bool in_swell = local_36;
    uint2 loop_bound_3 = uint2(4294967295u);
    bool loop_init_3 = true;
    while(true) {
        if (metal::all(loop_bound_3 == uint2(0u))) { break; }
        loop_bound_3 -= uint2(loop_bound_3.y == 0u, 1u);
        if (!loop_init_3) {
            uint _e221 = i_5;
            i_5 = _e221 + 1u;
        }
        loop_init_3 = false;
        uint _e65 = i_5;
        uint _e66 = oct_span(u);
        if (_e65 < _e66) {
            if (true) {
                local_38 = analytic_3;
            } else {
                local_38 = true;
            }
            bool _e74 = local_38;
            if (!(_e74)) {
                NodeLayer _e78 = band_3;
                float _e79 = layer_coverage(_e78);
                local_39 = _e79 > 0.0;
            } else {
                local_39 = true;
            }
            bool _e83 = local_39;
            if (!(_e83)) {
                local_40 = in_swell;
            } else {
                local_40 = true;
            }
            bool _e88 = local_40;
            if (!(_e88)) {
                uint _e95 = u.node.material_style;
                if (_e95 != 0u) {
                    local_42 = band_out_1 > band_in_1;
                } else {
                    local_42 = false;
                }
                bool _e102 = local_42;
                if (_e102) {
                    float _e106 = pigment_fringe(u);
                    local_43 = d_5 < ((metal::max(band_out_1, swell_out) + _e106) + aa_9);
                } else {
                    local_43 = false;
                }
                bool _e111 = local_43;
                local_41 = _e111;
            } else {
                local_41 = true;
            }
            bool _e113 = local_41;
            local_37 = _e113;
        } else {
            local_37 = false;
        }
        bool _e115 = local_37;
        if (_e115) {
        } else {
            break;
        }
        {
            uint _e117 = i_5;
            int slot_5 = as_type<int>(as_type<uint>(oct_1.base) + as_type<uint>(static_cast<int>(_e117)));
            float _e121 = oct_slot_level(in_10.octaves, slot_5);
            if (_e121 <= 0.0) {
                local_44 = presence_1 <= 0.0;
            } else {
                local_44 = false;
            }
            bool _e129 = local_44;
            if (_e129) {
                continue;
            }
            uint _e133 = u.node.material_style;
            if (_e133 != 0u) {
                if (band_out_1 <= band_in_1) {
                    continue;
                }
                SliceZones _e138 = pigment_zones(in_10, slot_5, oct_1, in_10.uv, aa_9, u);
                float _e139 = glyph_mask;
                float _e144 = mask_level(_e138.near.level);
                float _e150 = mask_level(_e138.far.level);
                glyph_mask = metal::max(_e139, metal::max(_e138.near.coverage * _e144, _e138.far.coverage * _e150));
                float _e154 = node_sd;
                float _e156 = layer_distance(_e154, _e138.near, in_10, u);
                node_sd = _e156;
                float _e157 = node_sd;
                float _e159 = layer_distance(_e157, _e138.far, in_10, u);
                node_sd = _e159;
                float _e162 = glyph;
                if (_e138.ink.w > _e162) {
                    glyph = _e138.ink.w;
                    glyph_rgb = _e138.ink.xyz;
                    glyph_lit = _e138.lit * _e121;
                }
                continue;
            }
            NodeLayer _e171 = band_3;
            NodeLayer _e172 = outer_glyph(slot_5, oct_1, in_10.uv, _e171, band_in_1, band_out_1, aa_9, u);
            float _e173 = layer_coverage(_e172);
            metal::float4 _e174 = oct_slot_ink(in_10, slot_5, u);
            float opacity_1 = _e174.w;
            slot_rgb = _e174.xyz;
            cov_1 = _e173 * opacity_1;
            lit_shape = _e173;
            float _e181 = slice_reach(in_10, slot_5, band_in_1, band_out_1, u);
            if (_e181 == band_out_1) {
                float _e183 = glyph_mask;
                glyph_mask = metal::max(_e183, _e172.coverage);
                float _e186 = node_sd;
                float _e190 = layer_distance(_e186, NodeLayer {_e172.sd, opacity_1, _e172.coverage}, in_10, u);
                node_sd = _e190;
            } else {
                SliceZones _e192 = slice_zones(in_10, slot_5, oct_1, in_10.uv, d_5, _e172, _e174, band_in_1, band_out_1, _e181, aa_9, u);
                slot_rgb = _e192.ink.xyz;
                cov_1 = _e192.ink.w;
                lit_shape = _e192.lit;
                float _e198 = glyph_mask;
                float _e204 = mask_level(_e192.far.level);
                glyph_mask = metal::max(_e198, _e192.near.coverage + (_e192.rest * _e204));
                float _e208 = node_sd;
                float _e210 = layer_distance(_e208, _e192.near, in_10, u);
                node_sd = _e210;
                float _e211 = node_sd;
                float _e213 = layer_distance(_e211, _e192.far, in_10, u);
                node_sd = _e213;
            }
            float _e214 = cov_1;
            float _e215 = glyph;
            if (_e214 > _e215) {
                float _e217 = cov_1;
                glyph = _e217;
                metal::float3 _e218 = slot_rgb;
                glyph_rgb = _e218;
                float _e219 = lit_shape;
                glyph_lit = _e219 * _e121;
            }
        }
    }
    float _e224 = glyph_taper(d_5, swells_1);
    float _e225 = glyph;
    glyph = _e225 * _e224;
    float _e227 = glyph_lit;
    glyph_lit = _e227 * _e224;
    float _e229 = glyph_mask;
    glyph_mask = _e229 * _e224;
    {
        metal::float2 _e231 = spectral_radii(u);
        NodeLayer _e236 = glyph_band(d_5, _e231.x, _e231.y, 1.0, aa_9);
        RingInk _e237 = spectral_ring(in_10, oct_1, in_10.uv, _e236, aa_9, analytic_3, u);
        float _e238 = node_sd;
        float _e240 = layer_distance(_e238, _e237.layer, in_10, u);
        node_sd = _e240;
        metal::float3 _e244 = glyph_rgb;
        float _e245 = glyph;
        float _e253 = glyph;
        glyph_rgb = ((_e237.color * _e237.cov) + ((_e244 * _e245) * (1.0 - _e237.cov))) / metal::float3(metal::max(_e237.cov + (_e253 * (1.0 - _e237.cov)), 0.0001));
        float _e266 = glyph_lit;
        glyph_lit = (_e237.lit * _e237.cov) + (_e266 * (1.0 - _e237.cov));
        float _e273 = glyph;
        glyph = _e237.cov + (_e273 * (1.0 - _e237.cov));
        float _e282 = mask_level(in_10.ring);
        float audio_mask = _e237.layer.coverage * _e282;
        float _e284 = glyph_mask;
        glyph_mask = audio_mask + (_e284 * (1.0 - audio_mask));
    }
    if (mark_out > mark_in) {
        NodeLayer _e296 = glyph_band(d_5, mark_in, mark_out, 1.0, aa_9);
        mark_strip = _e296;
    }
    float _e301 = mark_strip.sd;
    float _e305 = mark_strip.coverage;
    NodeLayer _e307 = drawn_marks(in_10, in_10.marks.x, oct_1, in_10.uv, d_5, NodeLayer {_e301, in_10.params.y, _e305}, mark_in, mark_out, aa_9, analytic_3, u);
    float _e312 = mark_strip.sd;
    float _e316 = mark_strip.coverage;
    NodeLayer _e318 = drawn_marks(in_10, in_10.marks.y, oct_1, in_10.uv, d_5, NodeLayer {_e312, in_10.params.z, _e316}, mark_in, mark_out, aa_9, analytic_3, u);
    float _e319 = layer_coverage(_e307);
    float _e320 = layer_coverage(_e318);
    float _e323 = mask_level(_e307.level);
    float melody_mask = _e307.coverage * _e323;
    float _e327 = mask_level(_e318.level);
    float bass_mask = _e318.coverage * _e327;
    float _e329 = node_sd;
    float _e330 = layer_distance(_e329, _e307, in_10, u);
    node_sd = _e330;
    float _e331 = node_sd;
    float _e332 = layer_distance(_e331, _e318, in_10, u);
    node_sd = _e332;
    mark = metal::max(_e319, _e320);
    mark_mask = metal::max(melody_mask, bass_mask);
    metal::float3 mark_rgb = (_e319 > _e320) ? in_10.melody_color.xyz : in_10.bass_color.xyz;
    float mark_taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_5);
    float _e348 = mark;
    mark = _e348 * mark_taper;
    float _e350 = mark_mask;
    mark_mask = _e350 * mark_taper;
    float _e352 = mark;
    metal::float3 _e354 = glyph_rgb;
    float _e355 = glyph;
    float _e357 = mark;
    float _e362 = mark;
    float _e363 = glyph;
    float _e364 = mark;
    glyph_rgb = ((mark_rgb * _e352) + ((_e354 * _e355) * (1.0 - _e357))) / metal::float3(metal::max(_e362 + (_e363 * (1.0 - _e364)), 0.0001));
    float _e373 = mark;
    float _e374 = glyph_lit;
    float _e375 = mark;
    glyph_lit = _e373 + (_e374 * (1.0 - _e375));
    float _e380 = mark;
    float _e381 = glyph;
    float _e382 = mark;
    glyph = _e380 + (_e381 * (1.0 - _e382));
    float _e387 = mark_mask;
    float _e388 = glyph_mask;
    float _e389 = mark_mask;
    glyph_mask = _e387 + (_e388 * (1.0 - _e389));
    float _e394 = glyph;
    float _e395 = base_alpha;
    float _e396 = glyph;
    float active_alpha = _e394 + (_e395 * (1.0 - _e396));
    metal::float3 _e401 = glyph_rgb;
    float _e402 = glyph;
    metal::float3 _e404 = base_rgb;
    float _e405 = glyph;
    metal::float3 active_rgb = (_e401 * _e402) + (_e404 * (1.0 - _e405));
    float _e410 = glyph_lit;
    float _e414 = glyph_mask;
    float _e415 = node_sd;
    return NodeInk {active_rgb, active_alpha, _e410 / metal::max(active_alpha, 0.0001), _e414, _e415};
}

float slice_progress(
    VsOut in_11,
    uint i_6
) {
    return static_cast<float>((in_11.motion[metal::min(unsigned(naga_div(i_6, 3u)), 3u)] >> (naga_mod(i_6, 3u) * 10u)) & 1023u) / 1023.0;
}

AnimatedInk animated_slice_ink(
    VsOut in_12,
    float aa_10,
    OctRing oct_2,
    constant Uniforms& u
) {
    NodeInk result = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, 65504.0};
    NodeInk marks = {};
    uint i_7 = 0u;
    bool local_45 = {};
    float coverage_2 = {};
    metal::float3 rgb = {};
    float lit = {};
    bool local_46 = {};
    bool local_47 = {};
    NodeInk _e11 = result;
    marks = _e11;
    float band_in_2 = u.node.band_inner;
    float band_out_2 = u.node.band_outer;
    float _e24 = u.node.mark_inner;
    float mark_in_1 = metal::min(_e24, 1.58);
    float _e30 = u.node.mark_thickness;
    float mark_out_1 = metal::min(mark_in_1 + metal::max(_e30, 0.0), 1.58);
    float anchor_radius = (band_out_2 > band_in_2) ? (0.5 * (band_in_2 + band_out_2)) : (0.5 * (mark_in_1 + mark_out_1));
    bool swells_2 = in_12.swell > band_out_2;
    uint2 loop_bound_4 = uint2(4294967295u);
    bool loop_init_4 = true;
    while(true) {
        if (metal::all(loop_bound_4 == uint2(0u))) { break; }
        loop_bound_4 -= uint2(loop_bound_4.y == 0u, 1u);
        if (!loop_init_4) {
            uint _e354 = i_7;
            i_7 = _e354 + 1u;
        }
        loop_init_4 = false;
        uint _e48 = i_7;
        uint _e49 = oct_span(u);
        if (_e48 < _e49) {
        } else {
            break;
        }
        {
            uint _e52 = i_7;
            int slot_6 = as_type<int>(as_type<uint>(oct_2.base) + as_type<uint>(static_cast<int>(_e52)));
            uint _e55 = i_7;
            float _e56 = slice_progress(in_12, _e55);
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
            float _e70 = oct_mid(slot_6, oct_2, u);
            metal::float2 anchor = anchor_radius * metal::float2(metal::cos(_e70), metal::sin(_e70));
            float _e79 = u.node.pose.y;
            metal::float2 start_1 = anchor * (1.0 + (_e79 * (1.0 - _e56)));
            metal::float2 uv_10 = anchor + ((in_12.uv - start_1) / metal::float2(scale));
            float d_10 = metal::length(uv_10);
            float soft_1 = aa_10 / scale;
            if (band_out_2 > band_in_2) {
                uint _e99 = u.node.material_style;
                local_45 = _e99 != 0u;
            } else {
                local_45 = false;
            }
            bool _e103 = local_45;
            if (_e103) {
                SliceZones _e104 = pigment_zones(in_12, slot_6, oct_2, uv_10, soft_1, u);
                float _e105 = glyph_taper(d_10, swells_2);
                float coverage_7 = (_e104.ink.w * _e105) * _e56;
                NodeLayer near_1 = _e104.near;
                NodeLayer far_1 = _e104.far;
                float _e114 = result.sd;
                float _e121 = layer_distance(_e114, NodeLayer {_e104.near.sd * scale, _e104.near.level * _e56, _e104.near.coverage}, in_12, u);
                result.sd = _e121;
                float _e124 = result.sd;
                float _e131 = layer_distance(_e124, NodeLayer {_e104.far.sd * scale, _e104.far.level * _e56, _e104.far.coverage}, in_12, u);
                result.sd = _e131;
                float _e134 = result.mask;
                float _e138 = mask_level(_e104.near.level * _e56);
                float _e143 = mask_level(_e104.far.level * _e56);
                result.mask = metal::max(_e134, metal::max(_e104.near.coverage * _e138, _e104.far.coverage * _e143) * _e105);
                float _e149 = result.alpha;
                if (coverage_7 > _e149) {
                    result.rgb = _e104.ink.xyz * coverage_7;
                    result.alpha = coverage_7;
                    result.lit = (_e104.lit * _e104.near.level) / metal::max(_e104.ink.w, 0.0001);
                }
            } else {
                if (band_out_2 > band_in_2) {
                    NodeLayer _e167 = glyph_band(d_10, band_in_2, band_out_2, 1.0, soft_1);
                    NodeLayer _e168 = outer_glyph(slot_6, oct_2, uv_10, _e167, band_in_2, band_out_2, soft_1, u);
                    metal::float4 _e169 = oct_slot_ink(in_12, slot_6, u);
                    float _e170 = glyph_taper(d_10, swells_2);
                    float _e172 = oct_slot_level(in_12.octaves, slot_6);
                    coverage_2 = ((_e168.coverage * _e170) * _e169.w) * _e56;
                    rgb = _e169.xyz;
                    lit = _e172 / metal::max(_e169.w, 0.0001);
                    float _e186 = slice_reach(in_12, slot_6, band_in_2, band_out_2, u);
                    if (_e186 == band_out_2) {
                        float _e190 = result.sd;
                        float _e197 = layer_distance(_e190, NodeLayer {_e168.sd * scale, _e169.w * _e56, _e168.coverage}, in_12, u);
                        result.sd = _e197;
                        float _e200 = result.mask;
                        float _e205 = mask_level(_e169.w * _e56);
                        result.mask = metal::max(_e200, (_e168.coverage * _e170) * _e205);
                    } else {
                        SliceZones _e208 = slice_zones(in_12, slot_6, oct_2, uv_10, d_10, _e168, _e169, band_in_2, band_out_2, _e186, soft_1, u);
                        rgb = _e208.ink.xyz;
                        coverage_2 = (_e208.ink.w * _e170) * _e56;
                        lit = (_e172 * _e208.lit) / metal::max(_e208.ink.w, 0.0001);
                        NodeLayer near_2 = _e208.near;
                        NodeLayer far_2 = _e208.far;
                        float _e226 = result.sd;
                        float _e233 = layer_distance(_e226, NodeLayer {_e208.near.sd * scale, _e208.near.level * _e56, _e208.near.coverage}, in_12, u);
                        result.sd = _e233;
                        float _e236 = result.sd;
                        float _e243 = layer_distance(_e236, NodeLayer {_e208.far.sd * scale, _e208.far.level * _e56, _e208.far.coverage}, in_12, u);
                        result.sd = _e243;
                        float _e247 = mask_level(_e208.near.level * _e56);
                        float _e252 = mask_level(_e208.far.level * _e56);
                        float footprint = (_e208.near.coverage * _e247) + (_e208.rest * _e252);
                        float _e257 = result.mask;
                        result.mask = metal::max(_e257, footprint * _e170);
                    }
                    float _e260 = coverage_2;
                    float _e262 = result.alpha;
                    if (_e260 > _e262) {
                        metal::float3 _e265 = rgb;
                        float _e266 = coverage_2;
                        result.rgb = _e265 * _e266;
                        float _e269 = coverage_2;
                        result.alpha = _e269;
                        float _e271 = lit;
                        result.lit = _e271;
                    }
                }
            }
            metal::float2 _e272 = mark_radii(in_12, slot_6, mark_in_1, mark_out_1, u);
            if (slot_6 >= 0) {
                local_46 = slot_6 < 11;
            } else {
                local_46 = false;
            }
            bool _e280 = local_46;
            if (_e280) {
                local_47 = _e272.y > _e272.x;
            } else {
                local_47 = false;
            }
            bool _e287 = local_47;
            if (_e287) {
                uint bit_1 = 1u << static_cast<uint>(slot_6);
                float melody_1 = ((in_12.marks.x & bit_1) != 0u) ? in_12.params.y : 0.0;
                float bass_1 = ((in_12.marks.y & bit_1) != 0u) ? in_12.params.z : 0.0;
                float level_7 = metal::max(melody_1, bass_1) * _e56;
                metal::float3 color = (melody_1 > bass_1) ? in_12.melody_color.xyz : in_12.bass_color.xyz;
                NodeLayer _e319 = glyph_band(d_10, _e272.x, _e272.y, level_7, soft_1);
                NodeLayer _e322 = outer_glyph(slot_6, oct_2, uv_10, _e319, _e272.x, _e272.y, soft_1, u);
                float taper = 1.0 - metal::smoothstep(1.5600001, QUAD_MARGIN, d_10);
                float _e328 = layer_coverage(_e322);
                float coverage_8 = _e328 * taper;
                float _e331 = marks.alpha;
                if (coverage_8 > _e331) {
                    marks.rgb = color * coverage_8;
                    marks.alpha = coverage_8;
                    marks.lit = 1.0;
                }
                float _e340 = marks.mask;
                float _e343 = mask_level(level_7);
                marks.mask = metal::max(_e340, (_e322.coverage * taper) * _e343);
                float _e348 = marks.sd;
                float _e353 = layer_distance(_e348, NodeLayer {_e322.sd * scale, level_7, _e322.coverage}, in_12, u);
                marks.sd = _e353;
            }
        }
    }
    NodeInk _e357 = result;
    NodeInk _e358 = marks;
    return AnimatedInk {_e357, _e358};
}

NodeInk node_ink(
    VsOut src_1,
    float d_6,
    float aa_11,
    OctRing oct_3,
    bool analytic_4,
    constant Uniforms& u
) {
    bool local_48 = {};
    bool local_49 = {};
    NodeInk ink_2 = {};
    float _e8 = u.node.animation;
    if (_e8 == 0.0) {
        NodeInk _e11 = base_node_ink(src_1, d_6, aa_11, oct_3, analytic_4, u);
        return _e11;
    }
    bool settled = (src_1.motion.w & 2147483648u) != 0u;
    float _e21 = u.node.band_outer;
    float _e25 = u.node.band_inner;
    if (_e21 <= _e25) {
        float _e32 = u.node.mark_thickness;
        local_48 = _e32 <= 0.0;
    } else {
        local_48 = false;
    }
    bool only_audio = local_48;
    if (!(settled)) {
        local_49 = only_audio;
    } else {
        local_49 = true;
    }
    bool _e41 = local_49;
    if (_e41) {
        NodeInk _e42 = base_node_ink(src_1, d_6, aa_11, oct_3, analytic_4, u);
        return _e42;
    }
    AnimatedInk _e43 = animated_slice_ink(src_1, aa_11, oct_3, u);
    ink_2 = _e43.body;
    metal::float2 _e46 = spectral_radii(u);
    NodeLayer _e53 = glyph_band(metal::length(src_1.uv), _e46.x, _e46.y, 1.0, aa_11);
    RingInk _e54 = spectral_ring(src_1, oct_3, src_1.uv, _e53, aa_11, analytic_4, u);
    float _e59 = ink_2.lit;
    float _e61 = ink_2.alpha;
    float lit_1 = (_e54.lit * _e54.cov) + ((_e59 * _e61) * (1.0 - _e54.cov));
    metal::float3 _e73 = ink_2.rgb;
    ink_2.rgb = (_e54.color * _e54.cov) + (_e73 * (1.0 - _e54.cov));
    float _e82 = ink_2.alpha;
    ink_2.alpha = _e54.cov + (_e82 * (1.0 - _e54.cov));
    float _e90 = ink_2.alpha;
    ink_2.lit = lit_1 / metal::max(_e90, 0.0001);
    float _e97 = mask_level(src_1.ring);
    float mask = _e54.layer.coverage * _e97;
    float _e101 = ink_2.mask;
    ink_2.mask = mask + (_e101 * (1.0 - mask));
    float _e108 = ink_2.sd;
    float _e110 = layer_distance(_e108, _e54.layer, src_1, u);
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
    metal::float2 _e4 = u.texture.target_size;
    return frag_pos / metal::max(_e4, metal::float2(1.0));
}

Painted node_paint(
    VsOut in_13,
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
    float final_alpha = {};
    float bloom_alpha = {};
    NodeGeom _e2 = node_geom(in_13, false, u);
    ShadowThrough _e9 = node_shadow_through(in_13.shadow_box.x, in_13.shadow_at.xy, in_13.shadow_at.z, shadow_atlas, shadow_sampler, shadow_casters, u, _buffer_sizes);
    if (!(_e2.paints)) {
        float shadow = 1.0 - _e9.seen;
        float bloom = 1.0 - _e9.bloom;
        if (bloom <= 0.0) {
            metal::discard_fragment();
        }
        return Painted {metal::float3(0.0), shadow, bloom, 0.0};
    }
    NodeInk _e28 = node_ink(in_13, _e2.d, _e2.aa, _e2.oct, false, u);
    ink_3 = _e28;
    float _e31 = ink_3.alpha;
    if (_e31 < INK_FLOOR) {
        float _e37 = ink_3.sd;
        ink_3 = NodeInk {metal::float3(0.0), 0.0, 0.0, 0.0, _e37};
    }
    float _e45 = ink_3.alpha;
    if (_e45 > 0.0) {
        float _e55 = u.geometry_shadow.occlusion;
        float _e56 = node_visibility(in_13.shadow_box.x, in_13.shadow_at.xy, _e55, shadow_atlas, shadow_sampler, shadow_casters, node_occluders, _buffer_sizes);
        visibility_1 = _e56;
    }
    float _e58 = ink_3.alpha;
    float _e59 = visibility_1;
    float visible_alpha = _e58 * _e59;
    bool _e65 = shadow_is_distance(in_13.shadow_box.x, shadow_casters, _buffer_sizes);
    if (_e65) {
        float _e69 = glow_shadow_depth(u);
        float _e71 = ink_3.alpha;
        final_alpha = visible_alpha + metal::max(0.0, (1.0 - _e9.seen) - (_e69 * _e71));
        float _e81 = ink_3.alpha;
        bloom_alpha = visible_alpha + metal::max(0.0, (1.0 - _e9.bloom) - _e81);
    } else {
        float _e90 = ink_3.mask;
        float seen_through = 1.0 - ((1.0 - _e9.seen) * (1.0 - _e90));
        float _e100 = ink_3.mask;
        float bloom_through = 1.0 - ((1.0 - _e9.bloom) * (1.0 - _e100));
        final_alpha = 1.0 - ((1.0 - visible_alpha) * seen_through);
        bloom_alpha = 1.0 - ((1.0 - visible_alpha) * bloom_through);
    }
    float _e116 = final_alpha;
    float _e117 = bloom_alpha;
    if (metal::max(_e116, _e117) <= 0.0) {
        metal::discard_fragment();
    }
    metal::float2 _e123 = light_coord(in_13.clip_pos.xy, u);
    metal::float4 _e124 = glow_light(_e123, glow_tex, glow_sampler);
    metal::float3 _e126 = ink_3.rgb;
    float _e128 = ink_3.alpha;
    float _e130 = glow_wash(u);
    float _e132 = ink_3.lit;
    metal::float3 _e135 = wash_over(_e126, _e128, _e124.xyz, metal::mix(1.0, _e130, _e132));
    float _e136 = visibility_1;
    float _e138 = final_alpha;
    float _e139 = bloom_alpha;
    return Painted {_e135 * _e136, _e138, _e139, visible_alpha};
}

SplitOut node_split(
    Painted paint,
    float shadow_alpha,
    constant Uniforms& u
) {
    float _e6 = u.geometry_shadow.occlusion;
    float alpha_1 = metal::mix(shadow_alpha, paint.ink_alpha, metal::clamp(_e6, 0.0, 1.0));
    return SplitOut {metal::float4(0.0, 0.0, 0.0, shadow_alpha), metal::float4(paint.rgb, alpha_1), metal::float4(paint.ink_alpha, 0.0, 0.0, paint.ink_alpha)};
}

struct fs_main_splitInput {
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
struct fs_main_splitOutput {
    metal::float4 other [[color(0)]];
    metal::float4 ink [[color(1)]];
    metal::float4 transmission [[color(2)]];
};
fragment fs_main_splitOutput fs_main_split(
  fs_main_splitInput varyings [[stage_in]]
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
    const VsOut in = { clip_pos, varyings.uv, {}, varyings.params, varyings.octaves, varyings.thickness, varyings.motion, varyings.cents, varyings.strip_row, varyings.marks, varyings.melody_color, varyings.bass_color, varyings.rim, varyings.swell, varyings.ring, varyings.ink_carry, varyings.shadow_box, varyings.shadow_at };
    Painted _e1 = node_paint(in, glow_tex, glow_sampler, shadow_atlas, shadow_sampler, shadow_casters, node_occluders, u, _buffer_sizes);
    SplitOut _e3 = node_split(_e1, _e1.seen, u);
    const auto _tmp = _e3;
    return fs_main_splitOutput { _tmp.other, _tmp.ink, _tmp.transmission };
}
