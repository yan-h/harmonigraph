// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
    uint buffer_size14;
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
struct type_8 {
    metal::float4 inner[64];
};
struct type_10 {
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
    type_8 pitch_lut;
    type_8 spectral_lut;
    type_10 spectrum_color;
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
struct Instance {
    metal::float3 world_pos;
    metal::float4 color;
    metal::float3 params;
    metal::packed_uint3 octaves;
    float cents;
    metal::uint2 marks;
    char _pad6[8];
    metal::float4 melody_color;
    metal::float4 bass_color;
    float scale;
    float ring;
    char _pad10[8];
    metal::float3 glow;
};
struct ShadowCell {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 cell_map;
    metal::float4 who;
};
struct type_13 {
    metal::float2 inner[4];
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
metal::float3 unpackFloat32x3_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11) {
    return metal::float3(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8));
}
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
uint3 unpackUint32x3_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11) {
    return uint3((b3 << 24 | b2 << 16 | b1 << 8 | b0), (b7 << 24 | b6 << 16 | b5 << 8 | b4), (b11 << 24 | b10 << 16 | b9 << 8 | b8));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}
uint2 unpackUint32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return uint2((b3 << 24 | b2 << 16 | b1 << 8 | b0), (b7 << 24 | b6 << 16 | b5 << 8 | b4));
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

metal::float2 cell_texel(
    metal::float2 points,
    metal::float4 rect,
    metal::float4 cell_1,
    float k
) {
    return cell_1.xy + ((points - rect.xy) * k);
}

metal::float4 no_quad(
) {
    return metal::float4(2.0, 2.0, 0.0, 1.0);
}

metal::float4 cell_clip(
    metal::float2 texel,
    metal::float2 size,
    float w
) {
    metal::float2 extent = metal::max(size, metal::float2(1.0));
    return metal::float4((((2.0 * texel.x) / extent.x) - 1.0) * w, (1.0 - ((2.0 * texel.y) / extent.y)) * w, 0.0, w);
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

metal::float2 pane_points(
    metal::float4 clip,
    constant Uniforms& u
) {
    metal::float2 ndc = clip.xy / metal::float2(clip.w);
    float _e14 = u.shadow_target.pane_points.x;
    float _e25 = u.shadow_target.pane_points.y;
    return metal::float2(((ndc.x * 0.5) + 0.5) * _e14, (0.5 - (ndc.y * 0.5)) * _e25);
}

float node_rim(
    bool marked,
    constant Uniforms& u
) {
    float rim = {};
    bool local_1 = {};
    float _e4 = u.node.rings_outer;
    rim = metal::max(_e4, 0.0);
    if (marked) {
        float _e13 = u.node.mark_thickness;
        local_1 = _e13 > 0.0;
    } else {
        local_1 = false;
    }
    bool _e17 = local_1;
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

VsOut node_vertex(
    uint vertex_index_1,
    Instance inst_1,
    constant Uniforms& u
) {
    type_13 corners = type_13 {{metal::float2(-1.0, -1.0), metal::float2(1.0, -1.0), metal::float2(-1.0, 1.0), metal::float2(1.0, 1.0)}};
    VsOut out_1 = {};
    metal::float2 corner = corners.inner[metal::min(unsigned(vertex_index_1), 3u)];
    float scale_1 = metal::max(inst_1.scale, 0.05);
    float _e28 = node_rim((inst_1.marks.x | inst_1.marks.y) != 0u, u);
    float _e29 = shadow_reach_uv(scale_1, u);
    float _e30 = quad_margin(_e28, _e29);
    float _e34 = u.node.radius;
    float radius = (((_e34 * 0.9) * 2.0) * _e30) * scale_1;
    metal::float4 _e45 = u.camera.right;
    metal::float4 _e52 = u.camera.up;
    metal::float3 world = inst_1.world_pos + (((_e45.xyz * corner.x) + (_e52.xyz * corner.y)) * radius);
    metal::float4x4 _e64 = u.camera.view_proj;
    out_1.clip_pos = _e64 * metal::float4(world, 1.0);
    out_1.uv = corner * _e30;
    out_1.color = inst_1.color;
    out_1.params = inst_1.params;
    out_1.octaves = inst_1.octaves;
    out_1.cents = inst_1.cents;
    out_1.strip_row = inst_1.glow.y;
    out_1.ink_carry = inst_1.glow.z;
    out_1.marks = inst_1.marks;
    out_1.melody_color = inst_1.melody_color;
    out_1.bass_color = inst_1.bass_color;
    out_1.rim = _e28;
    out_1.ring = inst_1.ring;
    out_1.shadow_box = metal::float4(0.0);
    out_1.shadow_at = metal::float4(0.0, 0.0, 0.0, 1.0);
    VsOut _e102 = out_1;
    return _e102;
}

struct vs_node_cellOutput {
    metal::float4 clip_pos [[position]];
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
struct vb_15_type { metal::uchar data[120]; };
struct vb_14_type { metal::uchar data[64]; };
vertex vs_node_cellOutput vs_node_cell(
  uint vertex_index [[vertex_id]]
, constant Uniforms& u [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, const device vb_14_type* vb_14_in [[buffer(14)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float3 world_pos = {};
    metal::float4 color = {};
    metal::float3 params = {};
    metal::uint3 octaves = {};
    float cents = {};
    metal::uint2 marks = {};
    metal::float4 melody_color = {};
    metal::float4 bass_color = {};
    float scale_2 = {};
    float ring = {};
    metal::float3 glow = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 120)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        world_pos = unpackFloat32x3_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        color = unpackFloat32x4_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15], vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27]);
        params = unpackFloat32x3_(vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31], vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35], vb_15_elem.data[36], vb_15_elem.data[37], vb_15_elem.data[38], vb_15_elem.data[39]);
        octaves = unpackUint32x3_(vb_15_elem.data[40], vb_15_elem.data[41], vb_15_elem.data[42], vb_15_elem.data[43], vb_15_elem.data[44], vb_15_elem.data[45], vb_15_elem.data[46], vb_15_elem.data[47], vb_15_elem.data[48], vb_15_elem.data[49], vb_15_elem.data[50], vb_15_elem.data[51]);
        cents = unpackFloat32_(vb_15_elem.data[52], vb_15_elem.data[53], vb_15_elem.data[54], vb_15_elem.data[55]);
        marks = unpackUint32x2_(vb_15_elem.data[56], vb_15_elem.data[57], vb_15_elem.data[58], vb_15_elem.data[59], vb_15_elem.data[60], vb_15_elem.data[61], vb_15_elem.data[62], vb_15_elem.data[63]);
        melody_color = unpackFloat32x4_(vb_15_elem.data[64], vb_15_elem.data[65], vb_15_elem.data[66], vb_15_elem.data[67], vb_15_elem.data[68], vb_15_elem.data[69], vb_15_elem.data[70], vb_15_elem.data[71], vb_15_elem.data[72], vb_15_elem.data[73], vb_15_elem.data[74], vb_15_elem.data[75], vb_15_elem.data[76], vb_15_elem.data[77], vb_15_elem.data[78], vb_15_elem.data[79]);
        bass_color = unpackFloat32x4_(vb_15_elem.data[80], vb_15_elem.data[81], vb_15_elem.data[82], vb_15_elem.data[83], vb_15_elem.data[84], vb_15_elem.data[85], vb_15_elem.data[86], vb_15_elem.data[87], vb_15_elem.data[88], vb_15_elem.data[89], vb_15_elem.data[90], vb_15_elem.data[91], vb_15_elem.data[92], vb_15_elem.data[93], vb_15_elem.data[94], vb_15_elem.data[95]);
        scale_2 = unpackFloat32_(vb_15_elem.data[96], vb_15_elem.data[97], vb_15_elem.data[98], vb_15_elem.data[99]);
        ring = unpackFloat32_(vb_15_elem.data[100], vb_15_elem.data[101], vb_15_elem.data[102], vb_15_elem.data[103]);
        glow = unpackFloat32x3_(vb_15_elem.data[104], vb_15_elem.data[105], vb_15_elem.data[106], vb_15_elem.data[107], vb_15_elem.data[108], vb_15_elem.data[109], vb_15_elem.data[110], vb_15_elem.data[111], vb_15_elem.data[112], vb_15_elem.data[113], vb_15_elem.data[114], vb_15_elem.data[115]);
    }
    metal::float4 rect_1 = {};
    metal::float4 cell_2 = {};
    metal::float4 cell_map = {};
    metal::float4 who = {};
    if (i_id < (_buffer_sizes.buffer_size14 / 64)) {
        const vb_14_type vb_14_elem = vb_14_in[i_id];
        rect_1 = unpackFloat32x4_(vb_14_elem.data[0], vb_14_elem.data[1], vb_14_elem.data[2], vb_14_elem.data[3], vb_14_elem.data[4], vb_14_elem.data[5], vb_14_elem.data[6], vb_14_elem.data[7], vb_14_elem.data[8], vb_14_elem.data[9], vb_14_elem.data[10], vb_14_elem.data[11], vb_14_elem.data[12], vb_14_elem.data[13], vb_14_elem.data[14], vb_14_elem.data[15]);
        cell_2 = unpackFloat32x4_(vb_14_elem.data[16], vb_14_elem.data[17], vb_14_elem.data[18], vb_14_elem.data[19], vb_14_elem.data[20], vb_14_elem.data[21], vb_14_elem.data[22], vb_14_elem.data[23], vb_14_elem.data[24], vb_14_elem.data[25], vb_14_elem.data[26], vb_14_elem.data[27], vb_14_elem.data[28], vb_14_elem.data[29], vb_14_elem.data[30], vb_14_elem.data[31]);
        cell_map = unpackFloat32x4_(vb_14_elem.data[32], vb_14_elem.data[33], vb_14_elem.data[34], vb_14_elem.data[35], vb_14_elem.data[36], vb_14_elem.data[37], vb_14_elem.data[38], vb_14_elem.data[39], vb_14_elem.data[40], vb_14_elem.data[41], vb_14_elem.data[42], vb_14_elem.data[43], vb_14_elem.data[44], vb_14_elem.data[45], vb_14_elem.data[46], vb_14_elem.data[47]);
        who = unpackFloat32x4_(vb_14_elem.data[48], vb_14_elem.data[49], vb_14_elem.data[50], vb_14_elem.data[51], vb_14_elem.data[52], vb_14_elem.data[53], vb_14_elem.data[54], vb_14_elem.data[55], vb_14_elem.data[56], vb_14_elem.data[57], vb_14_elem.data[58], vb_14_elem.data[59], vb_14_elem.data[60], vb_14_elem.data[61], vb_14_elem.data[62], vb_14_elem.data[63]);
    }
    const Instance inst = { world_pos, color, params, octaves, cents, marks, {}, melody_color, bass_color, scale_2, ring, {}, glow };
    const ShadowCell box = { rect_1, cell_2, cell_map, who };
    VsOut out = {};
    VsOut _e3 = node_vertex(vertex_index, inst, u);
    out = _e3;
    metal::float4x4 _e8 = u.camera.view_proj;
    metal::float4 centre_clip = _e8 * metal::float4(inst.world_pos, 1.0);
    float _e16 = u.node.radius;
    float uv_world = ((_e16 * 0.9) * 2.0) * metal::max(inst.scale, 0.05);
    metal::float4x4 _e28 = u.camera.view_proj;
    metal::float4 _e33 = u.camera.right;
    metal::float4 right_clip = _e28 * metal::float4(inst.world_pos + (_e33.xyz * uv_world), 1.0);
    metal::float2 _e40 = pane_points(centre_clip, u);
    metal::float2 _e41 = pane_points(right_clip, u);
    metal::float2 right = _e41 - _e40;
    float uv_points = metal::length(right);
    if (box.who.y < 0.5) {
        metal::float4 _e49 = out.clip_pos;
        metal::float2 _e50 = pane_points(_e49, u);
        metal::float2 _e55 = cell_texel(_e50, box.rect, box.cell, box.cell_map.x);
        metal::float4 _e57 = no_quad();
        metal::float2 _e61 = u.shadow_target.atlas_texels;
        float _e64 = out.clip_pos.w;
        metal::float4 _e65 = cell_clip(_e55, _e61, _e64);
        bool _e67 = cell_packed(box.cell);
        out.clip_pos = _e67 ? _e65 : _e57;
        out.shadow_box = box.cell;
        out.shadow_at = metal::float4(_e55, uv_points, box.cell_map.w);
        VsOut _e75 = out;
        const auto _tmp = _e75;
        return vs_node_cellOutput { _tmp.clip_pos, _tmp.uv, _tmp.color, _tmp.params, _tmp.octaves, _tmp.cents, _tmp.strip_row, _tmp.marks, _tmp.melody_color, _tmp.bass_color, _tmp.rim, _tmp.ring, _tmp.ink_carry, _tmp.shadow_box, _tmp.shadow_at };
    }
    metal::float2 corner_1 = metal::float2(((vertex_index & 1u) == 1u) ? 1.0 : 0.0, ((vertex_index & 2u) == 2u) ? 1.0 : 0.0);
    metal::float2 texel_1 = box.cell.xy + (corner_1 * box.cell.zw);
    metal::float2 points_1 = box.rect.xy + ((texel_1 - box.cell.xy) / metal::float2(metal::max(box.cell_map.x, 0.000001)));
    metal::float4x4 _e112 = u.camera.view_proj;
    metal::float4 _e117 = u.camera.up;
    metal::float4 up_clip = _e112 * metal::float4(inst.world_pos + (_e117.xyz * uv_world), 1.0);
    metal::float2 _e124 = pane_points(up_clip, u);
    metal::float2 up = _e124 - _e40;
    metal::float2 delta = points_1 - _e40;
    float det = (right.x * up.y) - (right.y * up.x);
    float stable_det = (det >= 0.0) ? metal::max(metal::abs(det), 0.000001) : -(metal::max(metal::abs(det), 0.000001));
    out.uv = metal::float2((delta.x * up.y) - (delta.y * up.x), (right.x * delta.y) - (right.y * delta.x)) / metal::float2(stable_det);
    metal::float4 _e163 = no_quad();
    metal::float2 _e167 = u.shadow_target.atlas_texels;
    metal::float4 _e169 = cell_clip(texel_1, _e167, 1.0);
    bool _e171 = cell_packed(box.cell);
    out.clip_pos = _e171 ? _e169 : _e163;
    out.shadow_box = box.cell;
    out.shadow_at = metal::float4(texel_1, -(metal::max(uv_points, 0.000001)), box.cell_map.w);
    VsOut _e182 = out;
    const auto _tmp = _e182;
    return vs_node_cellOutput { _tmp.clip_pos, _tmp.uv, _tmp.color, _tmp.params, _tmp.octaves, _tmp.cents, _tmp.strip_row, _tmp.marks, _tmp.melody_color, _tmp.bass_color, _tmp.rim, _tmp.ring, _tmp.ink_carry, _tmp.shadow_box, _tmp.shadow_at };
}
