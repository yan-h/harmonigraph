// Scratch-only: append to production module plus generated compute_star_record.
// GROUP_SHARED=false is the gather control; true enables bounded shared records.
override GROUP_SHARED: bool = false;
override GROUP_DECODED: bool = false;
@group(1) @binding(11)
var compute_halos: texture_storage_2d_array<rgba16float, write>;

// 256 raw records = 4096 bytes. Shader semantics and atlas decoding stay f32.
var<workgroup> group_records: array<vec4<u32>, 256>;
// 32 bytes/record = 8 KiB. Even if unused arrays survive specialization, the
// combined workgroup allocation is 12 KiB, below WebGPU's 16 KiB default.
struct ComputeStar {
    centre: vec2<f32>,
    shape: vec2<f32>,
    colour: vec3<f32>,
    active: u32,
};
var<workgroup> decoded_records: array<ComputeStar, 256>;

fn compute_decode(t: vec4<u32>) -> ComputeStar {
    var record: ComputeStar;
    record.centre = vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y));
    record.shape = unpack2x16float(t.w);
    record.colour = vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0;
    record.active = select(0u, 1u, t.w != 0u);
    return record;
}

fn compute_shared_record(s: StarSlice, f: vec2<f32>, at: i32) -> vec4<f32> {
    if GROUP_DECODED { return compute_star_decoded(s, f, decoded_records[u32(at)]); }
    return compute_star_record(s, f, group_records[u32(at)]);
}

// Repeat fs_star_halo's expressions in its original order. The caller supplies
// the equivalent fragment center, including the half-pixel offset.
fn compute_star_r(pixel: vec2<u32>, s: StarSlice) -> vec2<f32> {
    let step = cloud.size / cloud.star_halo_size;
    let pt = (vec2<f32>(pixel) + vec2<f32>(0.5)) * step;
    let sp = (pt - cloud.size * 0.5) * (STAR_PANE / cloud.size.y);
    return sp / s.cell - fract(s.offset);
}

fn compute_star_gather(s: StarSlice, f: vec2<f32>, index: i32) -> vec4<f32> {
    var halo = vec4<f32>(0.0);
    for (var y = -1; y <= 1; y += 1) {
        let row = index + y * s.grid.x;
        let fy = f.y - f32(y);
        halo += star_texel(s, vec2<f32>(f.x + 1.0, fy), row - 1, true);
        halo += star_texel(s, vec2<f32>(f.x, fy), row, true);
        halo += star_texel(s, vec2<f32>(f.x - 1.0, fy), row + 1, true);
    }
    return halo;
}

@compute @workgroup_size(8, 8, 1)
fn cs_star_halo(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(local_invocation_index) lane: u32,
) {
    let size = textureDimensions(compute_halos);
    let s = cloud.star_slices[wid.z];
    let base_pixel = wid.xy * vec2<u32>(8u);
    let last_pixel = min(base_pixel + vec2<u32>(7u), size - vec2<u32>(1u));

    // Positive axis scale means these two corners bound every nominal cell.
    // Border cells are exactly those already reached by the production 3x3.
    let first_cell = vec2<i32>(floor(compute_star_r(base_pixel, s)));
    let last_cell = vec2<i32>(floor(compute_star_r(last_pixel, s)));
    let patch_first = first_cell - vec2<i32>(1);
    let patch_size = vec2<u32>(last_cell - first_cell + vec2<i32>(3));
    let count = patch_size.x * patch_size.y;
    let cached = GROUP_SHARED && count <= 256u;

    // A uniform workgroup predicate. Every lane participates even in the
    // partially covered right/bottom workgroups. Never return before barrier.
    if cached {
        let offset = vec2<i32>(floor(s.offset)) + s.origin;
        for (var n = lane; n < count; n += 64u) {
            let cell = patch_first + vec2<i32>(i32(n % patch_size.x), i32(n / patch_size.x));
            let local = cell - offset;
            let index = s.base + local.y * s.grid.x + local.x;
            let t = textureLoad(star_atlas, atlas_texel(index), 0);
            if GROUP_DECODED { decoded_records[n] = compute_decode(t); }
            else { group_records[n] = t; }
        }
    }
    if GROUP_SHARED { workgroupBarrier(); }
    if any(gid.xy >= size) { return; }

    let r = compute_star_r(gid.xy, s);
    let o = floor(r);
    let f = r - o;
    let local = vec2<i32>(o) - vec2<i32>(floor(s.offset)) - s.origin;
    let index = s.base + local.y * s.grid.x + local.x;
    var halo = vec4<f32>(0.0);
    if cached {
        let local_patch = vec2<i32>(o) - patch_first;
        let center = local_patch.y * i32(patch_size.x) + local_patch.x;
        for (var y = -1; y <= 1; y += 1) {
            let row = center + y * i32(patch_size.x);
            let fy = f.y - f32(y);
            halo += compute_shared_record(s, vec2<f32>(f.x + 1.0, fy), row - 1);
            halo += compute_shared_record(s, vec2<f32>(f.x, fy), row);
            halo += compute_shared_record(s, vec2<f32>(f.x - 1.0, fy), row + 1);
        }
    } else {
        halo = compute_star_gather(s, f, index);
    }
    textureStore(compute_halos, vec2<i32>(gid.xy), i32(wid.z), halo);
}
