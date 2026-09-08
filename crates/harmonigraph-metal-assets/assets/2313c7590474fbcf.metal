// language: metal3.2
#include <metal_stdlib>
#include <simd/simd.h>

using metal::uint;

struct _mslBufferSizes {
    uint size1;
};

struct type_1 {
    uint inner[6];
};
typedef uint type_2[1];
struct OffsetPc {
    uint inner;
};

kernel void main_(
  device type_1& dst [[buffer(1)]]
, device type_2 const& src [[buffer(2)]]
, constant OffsetPc& offset [[buffer(0)]]
, constant _mslBufferSizes& _buffer_sizes [[buffer(3)]]
) {
    bool local = {};
    bool local_1 = {};
    uint _e3 = offset.inner;
    uint _e5 = src[_e3];
    uint _e9 = offset.inner;
    uint _e13 = src[_e9 + 1u];
    uint _e17 = offset.inner;
    uint _e21 = src[_e17 + 2u];
    metal::uint3 src_1 = metal::uint3(_e5, _e13, _e21);
    if (!((src_1.x > 65535u))) {
        local = src_1.y > 65535u;
    } else {
        local = true;
    }
    bool _e32 = local;
    if (!(_e32)) {
        local_1 = src_1.z > 65535u;
    } else {
        local_1 = true;
    }
    bool _e39 = local_1;
    if (_e39) {
        dst = type_1 {{0u, 0u, 0u, 0u, 0u, 0u}};
        return;
    } else {
        dst = type_1 {{src_1.x, src_1.y, src_1.z, src_1.x, src_1.y, src_1.z}};
        return;
    }
}
