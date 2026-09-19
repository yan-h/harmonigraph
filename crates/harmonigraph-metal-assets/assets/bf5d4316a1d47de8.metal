// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint buffer_size15;
};

struct Locals {
    metal::float2 origin_points;
    metal::float2 viewport_points;
    float min_midi;
    float span;
    float spectrum_min_midi;
    float bins_per_semitone;
    float level0_;
    float level_per_step;
    float level_per_midi;
    uint rows;
    uint bins;
    uint stride;
    uint capacity;
    uint first_slot;
    uint run_slabs;
    uint _pad0_;
    uint _pad1_;
    uint _pad2_;
};
struct VertexOut {
    metal::float4 position;
    float slab;
    float t;
    char _pad3[8];
};
constant float CLOUD_UNITS = 10.0;
constant float SCALE_CELLS = 2.7272727;
constant float DOME_RADIUS = 1.15;
constant float DOME_JITTER = 0.3;
constant float DOME_RADIUS_MIN = 0.95;
constant float DOME_RADIUS_MAX = 1.32;
constant float DOME_UNION = 9.0;
constant float DOME_VARIETY_GAIN = 5.0;
constant float DOME_FACE = 1.5122874;
constant float ROCK_TILT = 0.3;
constant float SUN_LEAN = 1.0;
constant float SUN_KNEE = 0.03;
constant float RELIEF_FLOOR_FALL = 3.22;
constant float DOME_LACUNARITY = 2.1;
constant float DOME_FINE_GAIN = 0.22;
constant float CLOUD_SHADE = 0.64;
constant int WASH_RING = 2;
constant float WASH_JITTER = 0.4;
constant float WASH_RAGGED = 0.3;
constant float WASH_RADIUS_MIN = 1.02;
constant float WASH_RADIUS_MAX = 1.66;
constant float WASH_CELLS = 5.25;
constant float WASH_LACUNARITY = 2.1;
constant float WASH_FINE_OCCUPANCY = 0.2;
constant float WASH_WARP = 0.45;
constant float WASH_WARP_SCALE = 0.9;
constant float WASH_RAGGED_SCALE = 2.8;
constant float WASH_POOL = 0.44;
constant float WASH_GRAIN = 0.1;
constant float WASH_POOL_WIDTH = 0.55;
constant float WASH_SURF = 0.07;
constant float WASH_PIG_DEPTH = 0.35;
constant float WASH_TONE_FLOOR = 0.05;
constant float WASH_PIVOT = 0.45;
constant float WASH_LIFT_A = 1.15;
constant float WASH_LIFT_B = 0.16;
constant float WASH_BLACK_KNEE = 0.175;
metal::float2 unpackFloat32x2_(uint b0, uint b1, uint b2, uint b3, uint b4, uint b5, uint b6, uint b7) {
    return metal::float2(as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0), as_type<float>(b7 << 24 | b6 << 16 | b5 << 8 | b4));
}
float unpackFloat32_(uint b0, uint b1, uint b2, uint b3) {
    return as_type<float>(b3 << 24 | b2 << 16 | b1 << 8 | b0);
}

struct vs_heatmapOutput {
    metal::float4 position [[position]];
    float slab [[user(loc0), center_perspective]];
    float t [[user(loc1), center_perspective]];
};
struct vb_15_type { metal::uchar data[16]; };
vertex vs_heatmapOutput vs_heatmap(
  constant Locals& locals [[buffer(0)]]
, uint v_id [[vertex_id]]
, const device vb_15_type* vb_15_in [[buffer(15)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(1)]]
) {
    metal::float2 pos = {};
    float slab = {};
    float t = {};
    if (v_id < (_buffer_sizes.buffer_size15 / 16)) {
        const vb_15_type vb_15_elem = vb_15_in[v_id];
        pos = unpackFloat32x2_(vb_15_elem.data[0], vb_15_elem.data[1], vb_15_elem.data[2], vb_15_elem.data[3], vb_15_elem.data[4], vb_15_elem.data[5], vb_15_elem.data[6], vb_15_elem.data[7]);
        slab = unpackFloat32_(vb_15_elem.data[8], vb_15_elem.data[9], vb_15_elem.data[10], vb_15_elem.data[11]);
        t = unpackFloat32_(vb_15_elem.data[12], vb_15_elem.data[13], vb_15_elem.data[14], vb_15_elem.data[15]);
    }
    VertexOut out = {};
    metal::float2 _e5 = locals.origin_points;
    metal::float2 in_viewport = pos - _e5;
    float _e15 = locals.viewport_points.x;
    float _e25 = locals.viewport_points.y;
    out.position = metal::float4(((2.0 * in_viewport.x) / _e15) - 1.0, 1.0 - ((2.0 * in_viewport.y) / _e25), 0.0, 1.0);
    out.slab = slab;
    out.t = t;
    VertexOut _e34 = out;
    const auto _tmp = _e34;
    return vs_heatmapOutput { _tmp.position, _tmp.slab, _tmp.t };
}
