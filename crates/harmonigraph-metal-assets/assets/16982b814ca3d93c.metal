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
