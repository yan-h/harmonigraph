// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size0;
    uint buffer_size15;
};

struct ShadowCaster {
    metal::float4 rect;
    metal::float4 cell;
    metal::float4 map;
    metal::float4 shade;
};
typedef ShadowCaster type_3[1];
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
struct type_8 {
    metal::float4 inner[3];
};
struct OctaveParams {
    float span;
    float center;
    metal::float2 padding;
    type_8 bounds;
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
struct type_9 {
    metal::float4 inner[64];
};
struct type_11 {
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
    type_9 pitch_lut;
    type_9 spectral_lut;
    type_11 spectrum_color;
};
struct type_12 {
    metal::float2 inner[4];
};
struct PlusInstance {
    metal::float4 pos_radius;
    metal::float4 color;
};
struct PlusVsOut {
    metal::float4 clip_pos;
    metal::float2 uv;
    char _pad2[8];
    metal::float4 color;
    metal::float4 shadow_box;
    metal::float4 shadow_at;
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
metal::float4 unpackFloat32x4_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7, uint b8, uint b9, uint b10, uint b11, uint b12, uint b13, uint b14, uint b15) {
    return metal::float4(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4), as_type<float>(b11 << 24 | b10 << 16 | b9 << 8 | b8), as_type<float>(b15 << 24 | b14 << 16 | b13 << 8 | b12));
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
    return metal::max(_e3, SHADOW_REACH_SIGMAS);
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

uint naga_f2u32(float value) {
    return static_cast<uint>(metal::clamp(value, 0.0, 4294967000.0));
}

bool shadow_is_distance(
    float who,
    device type_3 const& shadow_casters,
    constant _mslBufferSizes& _buffer_sizes
) {
    uint caster = naga_f2u32(metal::max(who, 0.0));
    if (caster >= (1 + (_buffer_sizes.size0 - 0 - 64) / 64)) {
        return false;
    }
    float _e12 = shadow_casters[metal::min(unsigned(caster), (_buffer_sizes.size0 - 0 - 64) / 64)].shade.y;
    return _e12 >= 0.5;
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
    metal::float4 shadow_box [[user(loc3), flat]];
    metal::float4 shadow_at [[user(loc4), center_no_perspective]];
};
struct vb_15_type { metal::uchar data[32]; };
vertex vs_plusOutput vs_plus(
  uint vertex_index [[vertex_id]]
, device type_3 const& shadow_casters [[buffer(1)]]
, constant Uniforms& u [[buffer(0)]]
, uint i_id [[instance_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(2)]]
) {
    metal::float4 pos_radius = {};
    metal::float4 color = {};
    if (i_id < (_buffer_sizes.buffer_size15 / 32)) {
        const vb_15_type vb_15_elem = vb_15_in[i_id];
        pos_radius = unpackFloat32x4_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7], vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11], vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
        color = unpackFloat32x4_(vb_15_elem.data[16], vb_15_elem.data[17], vb_15_elem.data[18], vb_15_elem.data[19], vb_15_elem.data[20], vb_15_elem.data[21], vb_15_elem.data[22], vb_15_elem.data[23], vb_15_elem.data[24], vb_15_elem.data[25], vb_15_elem.data[26], vb_15_elem.data[27], vb_15_elem.data[28], vb_15_elem.data[29], vb_15_elem.data[30], vb_15_elem.data[31]);
    }
    const PlusInstance inst = { pos_radius, color };
    type_12 corners = type_12 {{metal::float2(-1.0, -1.0), metal::float2(1.0, -1.0), metal::float2(-1.0, 1.0), metal::float2(1.0, 1.0)}};
    float stand = {};
    PlusVsOut out = {};
    metal::float2 corner = corners.inner[metal::min(unsigned(vertex_index), 3u)];
    float _e20 = marker_unit(u);
    float _e25 = u.marker.world_unit;
    float arm = (_e25 > 0.0) ? (inst.pos_radius.w / _e20) : 0.0;
    metal::float4x4 _e33 = u.camera.view_proj;
    metal::float4 centre_clip = _e33 * metal::float4(inst.pos_radius.xyz, 1.0);
    metal::float4x4 _e42 = u.camera.view_proj;
    metal::float4 _e48 = u.camera.right;
    metal::float4 arm_clip = _e42 * metal::float4(inst.pos_radius.xyz + (_e48.xyz * inst.pos_radius.w), 1.0);
    metal::float2 _e57 = pane_points(centre_clip, u);
    metal::float2 _e58 = pane_points(arm_clip, u);
    float arm_points = metal::distance(_e57, _e58);
    float _e60 = plus_shadow_width(u);
    float _e63 = plus_shadow_reach(u);
    float plus_reach_uv = ((0.5 * _e60) * _e63) / 1.8;
    stand = (arm > 0.0) ? (plus_reach_uv / arm) : 0.0;
    bool _e74 = shadow_is_distance(0.0, shadow_casters, _buffer_sizes);
    if (_e74) {
        float _e75 = plus_shadow_reach(u);
        float _e80 = shadow_casters[metal::min(unsigned(0), (_buffer_sizes.size0 - 0 - 64) / 64)].shade.z;
        stand = (_e75 * _e80) / metal::max(arm_points, 0.000001);
    }
    float _e86 = stand;
    float margin = metal::max(PLUS_QUAD_MARGIN, 1.0 + _e86);
    float reach = inst.pos_radius.w * margin;
    metal::float4 _e98 = u.camera.right;
    metal::float4 _e105 = u.camera.up;
    metal::float3 world = inst.pos_radius.xyz + (((_e98.xyz * corner.x) + (_e105.xyz * corner.y)) * reach);
    metal::float4x4 _e117 = u.camera.view_proj;
    out.clip_pos = _e117 * metal::float4(world, 1.0);
    out.uv = corner * margin;
    out.color = inst.color;
    out.shadow_box = metal::float4(0.0, arm_points, 0.0, 0.0);
    float shared_arm_points = u.marker_cell.arm_points;
    metal::float2 _e136 = out.uv;
    out.shadow_at = metal::float4(_e136 * shared_arm_points, inst.color.w, 1.0);
    PlusVsOut _e142 = out;
    const auto _tmp = _e142;
    return vs_plusOutput { _tmp.clip_pos, _tmp.uv, _tmp.color, _tmp.shadow_box, _tmp.shadow_at };
}
