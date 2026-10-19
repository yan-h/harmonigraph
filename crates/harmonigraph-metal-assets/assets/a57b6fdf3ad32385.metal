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
struct type_7 {
    metal::float4 inner[12];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_7 turns;
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
    TextureParams texture;
    PickupParams pickup;
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
struct type_12 {
    metal::float2 inner[4];
};
struct PlusInstance {
    metal::float4 pos_radius;
    metal::float4 color;
    uint place;
    char _pad3[12];
};
struct PlusVsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float2 points;
    char _pad4[8];
    metal::float4 shadow_box;
    metal::float4 shadow_at;
};
constant uint OCCLUDER_HEADER = 5u;
constant float GAUSSIAN_GAIN = 2.5;
constant float SHADOW_KEEP_FLOOR = 0.0009765625;
constant float GLOW_BASE = 0.8;
constant float TAU = 6.2831855;
constant float QUAD_MARGIN = 1.6;
constant float GLYPH_FADE_LIMIT = 1.3;
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
constant float OCT_UP = -4.712389;
constant float EMPTY_DISTANCE = 65504.0;
constant float PLUS_QUAD_MARGIN = 1.6;
constant metal::float3 GLOW_LUMINANCE = metal::float3(0.2126, 0.7152, 0.0722);
constant metal::float2 NEBULA_DETAIL_FADE = metal::float2(0.3, 1.1);
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
}
uint unpackUint32_(uint b0, uint b1, uint b2, uint b3) {
    return (b3 << 24 | b2 << 16 | b1 << 8 | b0);
}

float plus_shadow_width(
    constant Uniforms& u
) {
    float _e3 = u.marker_shadow.width;
    return metal::max(_e3, 0.0);
}

float plus_shadow_reach(
    constant Uniforms& u
) {
    float _e3 = u.marker_shadow.reach_sigmas;
    return _e3;
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

float marker_unit(
    constant Uniforms& u
) {
    float _e3 = u.marker.world_unit;
    return metal::max(_e3, 0.000001);
}

struct vs_plusOutput {
    metal::float4 clip_pos [[position]];
    metal::float2 uv [[user(loc0), center_perspective]];
    metal::float4 color [[user(loc1), center_perspective]];
    metal::float2 points [[user(loc2), center_no_perspective]];
    metal::float4 shadow_box [[user(loc3), flat]];
    metal::float4 shadow_at [[user(loc4), center_no_perspective]];
};
struct vb_15_type { metal::uchar data[36]; };
vertex vs_plusOutput vs_plus(
  uint vertex_index [[vertex_id]]
, constant Uniforms& u [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    metal::float4 pos_radius = {};
    metal::float4 color = {};
    uint place = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 36)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        pos_radius = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        color = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
        place = unpackUint32_(vb_15_elem.data[32], vb_15_elem.data[33], vb_15_elem.data[34], vb_15_elem.data[35]);
    }
    const PlusInstance inst = { pos_radius, color, place };
    type_12 corners = type_12 {{metal::float2(-1.0, -1.0), metal::float2(1.0, -1.0), metal::float2(-1.0, 1.0), metal::float2(1.0, 1.0)}};
    PlusVsOut out = {};
    metal::float2 corner = corners.inner[metal::min(unsigned(vertex_index), 3u)];
    float _e20 = marker_unit(u);
    float _e25 = u.marker.world_unit;
    float arm = (_e25 > 0.0) ? (inst.pos_radius.w / _e20) : 0.0;
    float _e30 = plus_shadow_width(u);
    float _e33 = plus_shadow_reach(u);
    float plus_reach_uv = ((0.5 * _e30) * _e33) / 1.8;
    float stand = (arm > 0.0) ? (plus_reach_uv / arm) : 0.0;
    float margin = metal::max(PLUS_QUAD_MARGIN, 1.0 + stand);
    float reach = inst.pos_radius.w * margin;
    metal::float4 _e54 = u.camera.right;
    metal::float4 _e61 = u.camera.up;
    metal::float3 world = inst.pos_radius.xyz + (((_e54.xyz * corner.x) + (_e61.xyz * corner.y)) * reach);
    metal::float4x4 _e73 = u.camera.view_proj;
    out.clip_pos = _e73 * metal::float4(world, 1.0);
    out.uv = corner * margin;
    out.color = inst.color;
    out.shadow_box = metal::float4(0.0, 0.0, static_cast<float>(inst.place), 0.0);
    metal::float4 _e90 = out.clip_pos;
    metal::float2 _e91 = pane_points(_e90, u);
    out.points = _e91;
    float shared_arm_points = u.marker_cell.arm_points;
    metal::float2 _e98 = out.uv;
    out.shadow_at = metal::float4(_e98 * shared_arm_points, inst.color.w, 1.0);
    PlusVsOut _e104 = out;
    const auto _tmp = _e104;
    return vs_plusOutput { _tmp.clip_pos, _tmp.uv, _tmp.color, _tmp.points, _tmp.shadow_box, _tmp.shadow_at };
}
