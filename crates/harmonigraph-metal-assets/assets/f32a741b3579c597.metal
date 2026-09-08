// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size2;
    uint size3;
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
struct GlowNode {
    metal::float2 inv_x;
    metal::float2 inv_y;
    metal::float2 centre;
    metal::float2 light;
    metal::float2 mark;
};
typedef GlowNode type_14[1];
typedef uint type_15[1];
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

float node_rim(
    bool marked,
    constant Uniforms& u
) {
    float rim = {};
    bool local_3 = {};
    float _e4 = u.node.rings_outer;
    rim = metal::max(_e4, 0.0);
    if (marked) {
        float _e13 = u.node.mark_thickness;
        local_3 = _e13 > 0.0;
    } else {
        local_3 = false;
    }
    bool _e17 = local_3;
    if (_e17) {
        float _e18 = rim;
        float _e22 = u.node.mark_inner;
        float _e26 = u.node.mark_thickness;
        rim = metal::max(_e18, _e22 + _e26);
    }
    float _e29 = rim;
    return _e29;
}

float glow_rim(
    float marked_1,
    constant Uniforms& u
) {
    float _e2 = node_rim(false, u);
    float _e4 = node_rim(true, u);
    return metal::mix(_e2, _e4, metal::clamp(marked_1, 0.0, 1.0));
}

float glow_level(
    float carried
) {
    return metal::clamp(carried, 0.0, 1.0);
}

int naga_mod(int lhs, int rhs) {
    int divisor = metal::select(rhs, 1, (lhs == (-2147483647 - 1) & rhs == -1) | (rhs == 0));
    return lhs - (lhs / divisor) * divisor;
}

metal::float4 strip_texel(
    int col,
    int row,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    uint clamped_lod_e9 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
    metal::float4 _e9 = ink_strip.read(metal::min(metal::uint2(metal::int2(naga_mod(as_type<int>(as_type<uint>(naga_mod(col, 64)) + as_type<uint>(64)), 64), row)), metal::uint2(ink_strip.get_width(clamped_lod_e9), ink_strip.get_height(clamped_lod_e9)) - 1), clamped_lod_e9);
    return _e9;
}

int naga_f2i32(float value) {
    return static_cast<int>(metal::clamp(value, -2147483600.0, 2147483500.0));
}

metal::float4 glow_ink(
    float strip_row,
    float angle,
    float mix_out,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    int row_1 = naga_f2i32(strip_row);
    float x = ((angle / TAU) * 64.0) - 0.5;
    float base = metal::floor(x);
    metal::float4 _e12 = strip_texel(naga_f2i32(base), row_1, ink_strip);
    metal::float4 _e16 = strip_texel(as_type<int>(as_type<uint>(naga_f2i32(base)) + as_type<uint>(1)), row_1, ink_strip);
    metal::float4 lit = metal::mix(_e12, _e16, x - base);
    uint clamped_lod_e23 = metal::min(uint(0), ink_strip.get_num_mip_levels() - 1);
    metal::float4 mean = ink_strip.read(metal::min(metal::uint2(metal::int2(64, row_1)), metal::uint2(ink_strip.get_width(clamped_lod_e23), ink_strip.get_height(clamped_lod_e23)) - 1), clamped_lod_e23);
    return metal::float4(metal::mix(mean.xyz, lit.xyz, mix_out), mean.w);
}

float glow_curve_at(
    float d,
    float span,
    constant Uniforms& u
) {
    float p = metal::clamp(d / span, 0.0, 1.0);
    float shape = u.glow.curve;
    float remaining = 1.0 - p;
    if (metal::abs(shape) < 0.05) {
        float shape2_ = shape * shape;
        return remaining * ((1.0 - ((shape * p) * 0.5)) + (((shape2_ * p) * ((2.0 * p) - 1.0)) / 12.0));
    }
    return (metal::exp(shape * remaining) - 1.0) / (metal::exp(shape) - 1.0);
}

metal::float4 glow_layer(
    GlowNode node,
    metal::float2 uv,
    constant Uniforms& u,
    metal::texture2d<float, metal::access::sample> ink_strip
) {
    float _e4 = glow_level(node.light.x);
    float _e8 = u.glow.reach;
    float reach = metal::max(_e8, 0.0);
    float _e14 = u.glow.strength;
    float strength = metal::max(_e14, 0.0);
    float _e19 = glow_rim(node.mark.x, u);
    float d_1 = metal::length(uv);
    float span_1 = metal::max(_e19 + reach, 0.1);
    if (d_1 >= span_1) {
        return metal::float4(0.0);
    }
    float _e28 = glow_curve_at(d_1, span_1, u);
    float skirt = GLOW_BASE * _e28;
    float seam = metal::max(_e19, 0.1);
    float mix_out_1 = metal::min(1.0, (d_1 * d_1) / (seam * seam));
    float alpha = metal::clamp((skirt * _e4) * strength, 0.0, 1.0);
    if (alpha <= 0.0) {
        return metal::float4(0.0);
    }
    metal::float4 _e51 = glow_ink(node.light.y, metal::atan2(uv.y, uv.x), mix_out_1, ink_strip);
    if (_e51.w <= 0.0) {
        return metal::float4(0.0);
    }
    return metal::float4(_e51.xyz, alpha);
}

metal::float3 glow_linear(
    metal::float3 rgb_1
) {
    return metal::select(metal::pow((rgb_1 + metal::float3(0.055)) / metal::float3(1.055), metal::float3(2.4)), rgb_1 / metal::float3(12.92), rgb_1 <= metal::float3(0.04045));
}

metal::float3 glow_gamma(
    metal::float3 rgb_2
) {
    return metal::select((1.055 * metal::pow(metal::max(rgb_2, metal::float3(0.0031308)), metal::float3(0.41666666))) - metal::float3(0.055), rgb_2 * 12.92, rgb_2 <= metal::float3(0.0031308));
}
metal::uint2 naga_f2u32(metal::float2 value) {
    return static_cast<metal::uint2>(metal::clamp(value, 0.0, 4294967000.0));
}

metal::uint2 naga_div(metal::uint2 lhs, metal::uint2 rhs) {
    return lhs / metal::select(rhs, 1u, rhs == 0u);
}


struct fs_glow_gatherInput {
};
struct fs_glow_gatherOutput {
    metal::float4 member [[color(0)]];
};
fragment fs_glow_gatherOutput fs_glow_gather(
  metal::float4 pos [[position]]
, constant Uniforms& u [[buffer(0)]]
, metal::texture2d<float, metal::access::sample> ink_strip [[texture(0)]]
, device type_14 const& glow_nodes [[buffer(1)]]
, device type_15 const& glow_tiles [[buffer(2)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    float screen = 0.0;
    float coverage = 0.0;
    metal::float3 rgb = metal::float3(0.0);
    metal::float4 accumulated = metal::float4(0.0);
    metal::float4 sole = metal::float4(0.0);
    uint count = 0u;
    uint local = {};
    uint global = {};
    bool local_1 = {};
    uint i = {};
    bool local_2 = {};
    metal::float3 linear = {};
    float _e4 = u.glow.lit;
    if (_e4 <= 0.0) {
        return fs_glow_gatherOutput { metal::float4(0.0) };
    }
    float _e12 = u.glow.accumulation;
    float accumulation = metal::clamp(_e12, 0.0, 1.0);
    float _e20 = u.glow.strength;
    float peak = metal::clamp(GLOW_BASE * _e20, 0.0, 1.0);
    if (peak <= 0.0) {
        return fs_glow_gatherOutput { metal::float4(0.0) };
    }
    metal::float3 _e30 = glow_linear(metal::float3(peak));
    float peak_luminance = _e30.x;
    if (peak_luminance <= 0.0) {
        return fs_glow_gatherOutput { metal::float4(0.0) };
    }
    metal::uint2 tile_xy = naga_div(naga_f2u32(pos.xy), metal::uint2(32u));
    uint _e59 = glow_tiles[metal::min(unsigned(0), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint tile = (tile_xy.y * _e59) + tile_xy.x;
    uint _e67 = glow_tiles[metal::min(unsigned(3u + tile), (_buffer_sizes.size3 - 0 - 4) / 4)];
    local = _e67;
    uint local_end = glow_tiles[metal::min(unsigned(4u + tile), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint _e76 = glow_tiles[metal::min(unsigned(1), (_buffer_sizes.size3 - 0 - 4) / 4)];
    global = _e76;
    uint global_end = glow_tiles[metal::min(unsigned(2), (_buffer_sizes.size3 - 0 - 4) / 4)];
    uint2 loop_bound = uint2(4294967295u);
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        uint _e81 = local;
        if (!((_e81 < local_end))) {
            uint _e86 = global;
            local_1 = _e86 < global_end;
        } else {
            local_1 = true;
        }
        bool _e89 = local_1;
        if (_e89) {
        } else {
            break;
        }
        {
            i = 4294967295u;
            uint _e92 = local;
            if (_e92 < local_end) {
                uint _e95 = local;
                uint _e97 = glow_tiles[metal::min(unsigned(_e95), (_buffer_sizes.size3 - 0 - 4) / 4)];
                i = _e97;
            }
            uint _e98 = global;
            if (_e98 < global_end) {
                uint _e103 = global;
                uint _e105 = glow_tiles[metal::min(unsigned(_e103), (_buffer_sizes.size3 - 0 - 4) / 4)];
                uint _e106 = i;
                local_2 = _e105 < _e106;
            } else {
                local_2 = false;
            }
            bool _e109 = local_2;
            if (_e109) {
                uint _e111 = global;
                uint _e113 = glow_tiles[metal::min(unsigned(_e111), (_buffer_sizes.size3 - 0 - 4) / 4)];
                i = _e113;
                uint _e114 = global;
                global = _e114 + 1u;
            } else {
                uint _e117 = local;
                local = _e117 + 1u;
            }
            uint _e121 = i;
            GlowNode node_1 = glow_nodes[metal::min(unsigned(_e121), (_buffer_sizes.size2 - 0 - 40) / 40)];
            metal::float2 delta = pos.xy - node_1.centre;
            metal::float2 uv_1 = metal::float2(metal::dot(node_1.inv_x, delta), metal::dot(node_1.inv_y, delta));
            metal::float4 _e132 = glow_layer(node_1, uv_1, u, ink_strip);
            if (_e132.w <= 0.0) {
                continue;
            }
            metal::float4 incoming = metal::float4(_e132.xyz * _e132.w, _e132.w);
            sole = incoming;
            uint _e141 = count;
            count = _e141 + 1u;
            if (accumulation > 0.0) {
                metal::float4 _e146 = accumulated;
                accumulated = incoming + (_e146 * (metal::float4(1.0) - incoming));
            }
            if (accumulation < 1.0) {
                metal::float3 _e155 = glow_linear(incoming.xyz);
                float share = metal::clamp(metal::dot(_e155, GLOW_LUMINANCE) / peak_luminance, 0.0, 1.0);
                float _e162 = screen;
                float _e163 = screen;
                screen = _e162 + (share * (1.0 - _e163));
                float _e168 = coverage;
                float _e171 = coverage;
                coverage = _e168 + ((_e132.w / peak) * (1.0 - _e171));
                metal::float3 _e176 = rgb;
                rgb = _e176 + _e155;
            }
        }
    }
    uint _e178 = count;
    if (_e178 <= 1u) {
        metal::float4 _e181 = sole;
        return fs_glow_gatherOutput { _e181 };
    }
    if (accumulation >= 1.0) {
        metal::float4 _e184 = accumulated;
        return fs_glow_gatherOutput { _e184 };
    }
    metal::float3 _e185 = rgb;
    float total = metal::dot(_e185, GLOW_LUMINANCE);
    float _e188 = screen;
    float light = metal::min(_e188, 1.0) * peak_luminance;
    if (total <= 0.0) {
        float _e194 = coverage;
        metal::float4 _e200 = accumulated;
        return fs_glow_gatherOutput { metal::mix(metal::float4(0.0, 0.0, 0.0, peak * _e194), _e200, accumulation) };
    }
    metal::float3 _e202 = rgb;
    linear = _e202 * (light / total);
    float _e207 = linear.x;
    float _e209 = linear.y;
    float _e212 = linear.z;
    float largest = metal::max(metal::max(_e207, _e209), _e212);
    if (largest > peak_luminance) {
        float chroma = metal::clamp((peak_luminance - light) / (largest - light), 0.0, 1.0);
        metal::float3 _e222 = linear;
        linear = metal::mix(metal::float3(light), _e222, chroma);
    }
    metal::float3 _e224 = linear;
    metal::float3 _e228 = glow_gamma(metal::max(_e224, metal::float3(0.0)));
    metal::float3 colour = metal::min(_e228, metal::float3(peak));
    float _e231 = coverage;
    float alpha_1 = metal::max(peak * _e231, metal::max(metal::max(colour.x, colour.y), colour.z));
    metal::float4 _e241 = accumulated;
    return fs_glow_gatherOutput { metal::mix(metal::float4(colour, metal::min(alpha_1, peak)), _e241, accumulation) };
}
