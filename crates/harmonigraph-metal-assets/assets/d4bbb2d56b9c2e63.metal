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

struct vs_ink_blurOutput {
    metal::float4 member [[position]];
};
struct vb_15_type { metal::uchar data[120]; };
vertex vs_ink_blurOutput vs_ink_blur(
  uint vertex_index [[vertex_id]]
, constant Uniforms& u [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
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
    float scale = {};
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
        scale = unpackFloat32_(vb_15_elem.data[96], vb_15_elem.data[97], vb_15_elem.data[98], vb_15_elem.data[99]);
        ring = unpackFloat32_(vb_15_elem.data[100], vb_15_elem.data[101], vb_15_elem.data[102], vb_15_elem.data[103]);
        glow = unpackFloat32x3_(vb_15_elem.data[104], vb_15_elem.data[105], vb_15_elem.data[106], vb_15_elem.data[107], vb_15_elem.data[108], vb_15_elem.data[109], vb_15_elem.data[110], vb_15_elem.data[111], vb_15_elem.data[112], vb_15_elem.data[113], vb_15_elem.data[114], vb_15_elem.data[115]);
    }
    const Instance inst = { world_pos, color, params, octaves, cents, marks, {}, melody_color, bass_color, scale, ring, {}, glow };
    if (inst.glow.x <= 0.0) {
        return vs_ink_blurOutput { metal::float4(0.0, 0.0, 0.0, 1.0) };
    }
    metal::float2 corner = metal::float2(static_cast<float>(vertex_index & 1u), static_cast<float>(vertex_index >> 1u));
    float _e21 = u.glow.row_capacity;
    float rows = metal::max(_e21, 1.0);
    float v = (inst.glow.y + corner.y) / rows;
    return vs_ink_blurOutput { metal::float4((corner.x * 2.0) - 1.0, 1.0 - (2.0 * v), 0.0, 1.0) };
}
