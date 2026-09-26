// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
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
struct Instance {
    metal::float3 world_pos;
    metal::float4 params;
    metal::uint3 octaves;
    metal::uint3 thickness;
    metal::uint4 motion;
    float cents;
    char _pad6[4];
    metal::uint2 marks;
    metal::float4 melody_color;
    metal::float4 bass_color;
    float scale;
    float ring;
    char _pad11[8];
    metal::float4 glow;
};
struct type_14 {
    metal::float2 inner[4];
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
metal::float3 unpackFloat32x3_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11) {
    return metal::float3(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8));
}
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
uint3 unpackUint32x3_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11) {
    return uint3((b3 << 24 | b2 << 16 | b1 << 8 | b0), (b7 << 24 | b6 << 16 | b5 << 8 | b4), (b11 << 24 | b10 << 16 | b9 << 8 | b8));
}
metal::uint4 unpackUint32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::uint4((b3 << 24 | b2 << 16 | b1 << 8 | b0), (b7 << 24 | b6 << 16 | b5 << 8 | b4), (b11 << 24 | b10 << 16 | b9 << 8 | b8), (b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}
uint2 unpackUint32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return uint2((b3 << 24 | b2 << 16 | b1 << 8 | b0), (b7 << 24 | b6 << 16 | b5 << 8 | b4));
}

float glow_shadow(
    constant Uniforms& u
) {
    float _e3 = u.geometry_shadow.width;
    return metal::max(_e3, 0.0);
}

float glow_shadow_reach(
    constant Uniforms& u
) {
    float _e3 = u.geometry_shadow.reach_sigmas;
    return metal::max(_e3, SHADOW_REACH_SIGMAS);
}

float shadow_reach_uv(
    float scale,
    constant Uniforms& u
) {
    float _e1 = glow_shadow(u);
    float _e4 = glow_shadow_reach(u);
    return ((0.5 * _e1) * _e4) / (1.8 * metal::max(scale, 0.05));
}

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

float quad_margin(
    float rim_1,
    float g
) {
    return metal::max(QUAD_MARGIN, (rim_1 + g) + 0.05);
}

metal::float2 spectral_radii(
    constant Uniforms& u
) {
    float _e3 = u.spectral.inner;
    float _e7 = u.spectral.outer;
    return metal::float2(_e3, _e7);
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
    float reach = inner + ((outer_1 - inner) * t);
    if (t < 1.0) {
        return reach;
    }
    float _e10 = swell_cap(outer_1, u);
    return metal::max(outer_1, metal::min(reach, _e10));
}

float slices_outer(
    metal::uint3 thickness_1,
    constant Uniforms& u
) {
    float t_1 = 1.0;
    uint i_2 = 0u;
    float inner_1 = u.node.band_inner;
    float outer_2 = u.node.band_outer;
    if (outer_2 <= inner_1) {
        return outer_2;
    }
    uint2 loop_bound = uint2(4294967295u);
    bool loop_init = true;
    while(true) {
        if (metal::all(loop_bound == uint2(0u))) { break; }
        loop_bound -= uint2(loop_bound.y == 0u, 1u);
        if (!loop_init) {
            uint _e21 = i_2;
            i_2 = _e21 + 1u;
        }
        loop_init = false;
        uint _e14 = i_2;
        if (_e14 < OCTAVE_SLOTS) {
        } else {
            break;
        }
        {
            float _e17 = t_1;
            uint _e18 = i_2;
            float _e19 = slice_thickness(thickness_1, _e18);
            t_1 = metal::max(_e17, _e19);
        }
    }
    float _e24 = t_1;
    float _e25 = reach_at(_e24, inner_1, outer_2, u);
    return _e25;
}

VsOut node_vertex(
    uint vertex_index_1,
    Instance inst_1,
    constant Uniforms& u
) {
    type_14 corners = type_14 {{metal::float2(-1.0, -1.0), metal::float2(1.0, -1.0), metal::float2(-1.0, 1.0), metal::float2(1.0, 1.0)}};
    float rim_2 = {};
    bool local_1 = {};
    bool local_2 = {};
    bool local_3 = {};
    VsOut out_1 = {};
    metal::float2 corner = corners.inner[metal::min(unsigned(vertex_index_1), 3u)];
    float scale_1 = metal::max(inst_1.scale, 0.05);
    bool marked_1 = (inst_1.marks.x | inst_1.marks.y) != 0u;
    float _e28 = node_rim(marked_1, u);
    rim_2 = _e28;
    float _e31 = slices_outer(inst_1.thickness, u);
    float _e35 = u.node.band_outer;
    if (_e31 > _e35) {
        float _e37 = rim_2;
        rim_2 = metal::max(_e37, _e31);
        if (marked_1) {
            float _e44 = u.node.mark_thickness;
            local_1 = _e44 > 0.0;
        } else {
            local_1 = false;
        }
        bool _e48 = local_1;
        if (_e48) {
            float _e52 = u.node.mark_inner;
            float _e56 = u.node.mark_thickness;
            float mark_out = _e52 + _e56;
            float _e58 = rim_2;
            float _e62 = u.node.band_outer;
            rim_2 = metal::max(_e58, mark_out + (_e31 - _e62));
        }
    }
    float _e66 = rim_2;
    float _e70 = u.node.band_outer;
    float _e74 = u.node.band_inner;
    if (!((_e70 > _e74))) {
        if ((inst_1.marks.x | inst_1.marks.y) != 0u) {
            float _e91 = u.node.mark_thickness;
            local_3 = _e91 > 0.0;
        } else {
            local_3 = false;
        }
        bool _e95 = local_3;
        local_2 = _e95;
    } else {
        local_2 = true;
    }
    bool _e97 = local_2;
    float midi_rim = _e97 ? _e66 : 0.0;
    float _e100 = rim_2;
    float _e101 = rim_2;
    float _e106 = u.node.pose.z;
    metal::float2 _e108 = spectral_radii(u);
    float _e115 = u.node.animation;
    float bounds = (_e115 != 0.0) ? metal::max(_e101, metal::max(midi_rim * _e106, _e108.y)) : _e100;
    float _e119 = shadow_reach_uv(scale_1, u);
    float _e120 = quad_margin(bounds, _e119);
    float _e124 = u.node.radius;
    float radius = (((_e124 * 0.9) * 2.0) * _e120) * scale_1;
    metal::float4 _e135 = u.camera.right;
    metal::float4 _e142 = u.camera.up;
    metal::float3 world = inst_1.world_pos + (((_e135.xyz * corner.x) + (_e142.xyz * corner.y)) * radius);
    metal::float4x4 _e154 = u.camera.view_proj;
    out_1.clip_pos = _e154 * metal::float4(world, 1.0);
    out_1.uv = corner * _e120;
    out_1.params = inst_1.params;
    out_1.octaves = inst_1.octaves;
    out_1.thickness = inst_1.thickness;
    out_1.motion = inst_1.motion;
    out_1.cents = inst_1.cents;
    out_1.strip_row = inst_1.glow.y;
    out_1.ink_carry = inst_1.glow.z;
    out_1.marks = inst_1.marks;
    out_1.melody_color = inst_1.melody_color;
    out_1.bass_color = inst_1.bass_color;
    float _e183 = rim_2;
    out_1.rim = _e183;
    out_1.swell = _e31;
    out_1.ring = inst_1.ring;
    out_1.shadow_box = metal::float4(0.0);
    out_1.shadow_at = metal::float4(0.0, 0.0, 0.0, 1.0);
    VsOut _e196 = out_1;
    return _e196;
}

struct vs_ink_stripOutput {
    metal::float4 clip_pos [[position]];
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
struct vb_15_type { metal::uchar data[136]; };
vertex vs_ink_stripOutput vs_ink_strip(
  uint vertex_index [[vertex_id]]
, constant Uniforms& u [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float3 world_pos = {};
    metal::float4 params = {};
    metal::uint3 octaves = {};
    metal::uint4 motion = {};
    float cents = {};
    metal::uint2 marks = {};
    metal::float4 melody_color = {};
    metal::float4 bass_color = {};
    float scale_2 = {};
    float ring = {};
    metal::float4 glow = {};
    metal::uint3 thickness_2 = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 136)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        world_pos = unpackFloat32x3_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        params = unpackFloat32x4_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15], vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27]);
        octaves = unpackUint32x3_(vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31], vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        motion = unpackUint32x4_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43], vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47], vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51], vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55]);
        cents = unpackFloat32_(vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59]);
        marks = unpackUint32x2_(vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63], vb_15_elem.data[64], vb_15_elem.data[65], vb_15_elem.data[66], vb_15_elem.data[67]);
        melody_color = unpackFloat32x4_(vb_15_elem.data[68], vb_15_elem.data[69], vb_15_elem.data[70], vb_15_elem.data[71], vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79], vb_15_elem.data[80], vb_15_elem.data[81], vb_15_elem.data[82], vb_15_elem.data[83]);
        bass_color = unpackFloat32x4_(vb_15_elem.data[84], vb_15_elem.data[85], vb_15_elem.data[86], vb_15_elem.data[87], vb_15_elem.data[88], vb_15_elem.data[89], vb_15_elem.data[90], vb_15_elem.data[91], vb_15_elem.data[92], vb_15_elem.data[93], vb_15_elem.data[94], vb_15_elem.data[95], vb_15_elem.data[96], vb_15_elem.data[97], vb_15_elem.data[98], vb_15_elem.data[99]);
        scale_2 = unpackFloat32_(vb_15_elem.data[100], vb_15_elem.data[101], vb_15_elem.data[102], vb_15_elem.data[103]);
        ring = unpackFloat32_(vb_15_elem.data[104], vb_15_elem.data[105], vb_15_elem.data[106], vb_15_elem.data[107]);
        glow = unpackFloat32x4_(vb_15_elem.data[108], vb_15_elem.data[109], vb_15_elem.data[110], vb_15_elem.data[111], vb_15_elem.data[112], vb_15_elem.data[113], vb_15_elem.data[114], vb_15_elem.data[115], vb_15_elem.data[116], vb_15_elem.data[117], vb_15_elem.data[118], vb_15_elem.data[119], vb_15_elem.data[120], vb_15_elem.data[121], vb_15_elem.data[122], vb_15_elem.data[123]);
        thickness_2 = unpackUint32x3_(vb_15_elem.data[124], vb_15_elem.data[125], vb_15_elem.data[126], vb_15_elem.data[127], vb_15_elem.data[128], vb_15_elem.data[129], vb_15_elem.data[130], vb_15_elem.data[131], vb_15_elem.data[132], vb_15_elem.data[133], vb_15_elem.data[134], vb_15_elem.data[135]);
    }
    const Instance inst = { world_pos, params, octaves, thickness_2, motion, cents, {}, marks, melody_color, bass_color, scale_2, ring, {}, glow };
    VsOut out = {};
    VsOut _e2 = node_vertex(vertex_index, inst, u);
    out = _e2;
    metal::float2 corner_1 = metal::float2(static_cast<float>(vertex_index & 1u), static_cast<float>(vertex_index >> 1u));
    float _e14 = u.glow.row_capacity;
    float rows = metal::max(_e14, 1.0);
    float _e18 = out.strip_row;
    float v = (_e18 + corner_1.y) / rows;
    out.clip_pos = metal::float4((corner_1.x * 2.0) - 1.0, 1.0 - (2.0 * v), 0.0, 1.0);
    out.uv = metal::float2(corner_1.x, 0.0);
    if (inst.glow.x <= 0.0) {
        out.clip_pos = metal::float4(0.0, 0.0, 0.0, 1.0);
    }
    VsOut _e49 = out;
    const auto _tmp = _e49;
    return vs_ink_stripOutput { _tmp.clip_pos, _tmp.uv, _tmp.params, _tmp.octaves, _tmp.thickness, _tmp.motion, _tmp.cents, _tmp.strip_row, _tmp.marks, _tmp.melody_color, _tmp.bass_color, _tmp.rim, _tmp.swell, _tmp.ring, _tmp.ink_carry, _tmp.shadow_box, _tmp.shadow_at };
}
