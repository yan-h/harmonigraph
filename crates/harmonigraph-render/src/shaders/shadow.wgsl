// The shadow atlas's blur: every caster's ink, drawn into a cell of its own
// (`vs_glyph_cell` in text.wgsl), convolved with a Gaussian of that cell's σ —
// once along x into the atlas's second texture, once along y back into the
// first, which the scene pass then samples (`fs_shadow_box`).
//
// A pass here is a draw over the CELLS rather than one quad over the atlas,
// and that is what lets every cell carry its own σ: a cell's quad reads σ off
// its instance, and a tap that falls outside the cell's own rect reads nothing,
// so a cell never sees its neighbour's ink whatever either is blurred by. The
// padding a cell is packed with (`pack` in shadow.rs) is what puts the whole of
// its blur inside the rect the taps are clamped to.

@group(0) @binding(0) var src: texture_2d<f32>;
// The layout's sampler slot, which the scene pass's one tap takes and the blur
// does not: a blur tap is a texel by construction (`textureLoad`).
@group(0) @binding(1) var src_sampler: sampler;

struct CellOut {
    @builtin(position) position: vec4<f32>,
    /// The cell's own rect in atlas texels — min, then max, the max exclusive.
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    /// σ in atlas texels.
    @location(1) @interpolate(flat) sigma: f32,
};

/// One cell's quad, over exactly the texels its packer gave it. The
/// attributes are `ShadowBox`'s three rows (shadow.rs).
@vertex
fn vs_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
) -> CellOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    // The texture being read is the atlas's other half, at the same size as
    // the one being written.
    let atlas = vec2<f32>(textureDimensions(src));
    let texel = cell.xy + corner * cell.zw;
    var out: CellOut;
    out.position = vec4<f32>(
        texel.x / atlas.x * 2.0 - 1.0,
        1.0 - texel.y / atlas.y * 2.0,
        0.0,
        1.0,
    );
    out.bounds = vec4<f32>(cell.xy, cell.xy + cell.zw);
    out.sigma = terms.y;
    return out;
}

/// How many σ out the kernel reaches. Three, where 0.3% of a Gaussian's mass is
/// left: the padding a cell is packed with reaches the same distance
/// (`REACH_SIGMAS` in shadow.rs), so the kernel and the cell agree on where a
/// shadow stops.
const REACH: f32 = 3.0;

/// The widest kernel a cell can ask for, in taps either side of the centre —
/// `ceil(REACH * SIGMA_CELL_MAX)` for shadow.rs's `SIGMA_CELL_MAX`, which the
/// packer holds every cell's σ under. A bound on the LOOP rather than a second
/// copy of the packer's rule: a cell arriving over it draws wrong, not forever.
const MAX_RADIUS: i32 = 9;

/// The Gaussian along `axis` at this fragment, over this cell's texels and no
/// others.
///
/// Normalised by the whole kernel and not by the taps that landed inside the
/// cell: outside the rect the ink IS zero, so a tap there contributes nothing
/// and still counts, and the blur of a half-plane comes out at exactly half at
/// its edge. Normalising over the taps that landed would lift every cell's
/// edge back toward its interior.
fn blur(in: CellOut, axis: vec2<i32>) -> f32 {
    let sigma = max(in.sigma, 1.0e-3);
    let radius = min(i32(ceil(REACH * sigma)), MAX_RADIUS);
    let at = vec2<i32>(in.position.xy);
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -radius; i <= radius; i = i + 1) {
        let w = exp(-0.5 * f32(i * i) / (sigma * sigma));
        weight = weight + w;
        let tap = at + axis * i;
        let centre = vec2<f32>(tap) + vec2<f32>(0.5);
        if centre.x < in.bounds.x || centre.y < in.bounds.y
            || centre.x >= in.bounds.z || centre.y >= in.bounds.w {
            continue;
        }
        sum = sum + w * textureLoad(src, tap, 0).r;
    }
    return sum / weight;
}

@fragment
fn fs_blur_x(in: CellOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in, vec2<i32>(1, 0)), 0.0, 0.0, 1.0);
}

@fragment
fn fs_blur_y(in: CellOut) -> @location(0) vec4<f32> {
    return vec4<f32>(blur(in, vec2<i32>(0, 1)), 0.0, 0.0, 1.0);
}
