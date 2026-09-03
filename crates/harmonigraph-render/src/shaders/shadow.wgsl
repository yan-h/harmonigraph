// What turns every caster's ink, drawn into a cell of its own (`vs_glyph_cell`
// in text.wgsl), into what that cell HOLDS — which is one of two things.
//
// A BLUR cell holds the ink convolved with a Gaussian of that cell's σ, once
// along x into the atlas's second texture and once along y back into the first.
// A DISTANCE cell holds how far each texel stands from the caster's nearest
// ink, in pane points. Nodes write that field analytically and names MIN-blend
// their fixed glyph SDFs.
// Either way the scene pass samples one texel and spends it (`fs_shadow_box`).
//
// A pass here is a draw over the CELLS rather than one quad over the atlas, and
// that is what lets every cell carry its own σ, its own resolution and its own
// kind: a cell's quad reads them off its instance, a tap that falls outside the
// cell's own rect reads nothing, and a cell of the wrong kind collapses its
// quad off the viewport. The padding a cell is packed with
// (`pack` in shadow.rs) is what puts the whole of its blur — or the whole of
// the reach its curve is windowed to — inside the rect the taps are clamped to.

@group(0) @binding(0) var src: texture_2d<f32>;
// The layout's sampler slot, which the scene pass's one tap takes and nothing
// here does: every tap in this module is a texel by construction
// (`textureLoad`).
@group(0) @binding(1) var src_sampler: sampler;

/// What `ShadowBox::who.y` holds for a cell that is a DISTANCE rather than
/// blurred ink — `shadow::DISTANCE_KIND`, and spelled again in common.wgsl
/// because there is no linkage between shader modules here.
const DISTANCE_KIND: f32 = 1.0;

struct CellOut {
    @builtin(position) position: vec4<f32>,
    /// The cell's own rect in atlas texels — min, then max, the max exclusive.
    @location(0) @interpolate(flat) bounds: vec4<f32>,
    /// σ in atlas texels. Distance cells collapse before this is read.
    @location(1) @interpolate(flat) sigma: f32,
};

/// A quad with no area, off the viewport: what a pass emits for a cell of the
/// other kind. `no_quad` in common.wgsl is the same value, spelled there for
/// the draws that FILL a cell.
fn no_quad() -> vec4<f32> {
    return vec4<f32>(2.0, 2.0, 0.0, 1.0);
}

/// One cell's quad, over exactly the texels its packer gave it — or nothing at
/// all where `draws` is false. The attributes are `ShadowBox`'s four rows
/// (shadow.rs).
fn cell_quad(
    vertex: u32,
    cell: vec4<f32>,
    terms: vec4<f32>,
    draws: bool,
) -> CellOut {
    let corner = vec2<f32>(
        select(0.0, 1.0, (vertex & 1u) == 1u),
        select(0.0, 1.0, (vertex & 2u) == 2u),
    );
    // The atlas's size, off the texture at group 0 — which every pass here
    // binds.
    let atlas = vec2<f32>(textureDimensions(src));
    let texel = cell.xy + corner * cell.zw;
    var out: CellOut;
    out.position = vec4<f32>(
        texel.x / atlas.x * 2.0 - 1.0,
        1.0 - texel.y / atlas.y * 2.0,
        0.0,
        1.0,
    );
    if !draws {
        out.position = no_quad();
    }
    out.bounds = vec4<f32>(cell.xy, cell.xy + cell.zw);
    out.sigma = terms.y;
    return out;
}

/// The BLUR cells alone: the x pass. A distance cell holds its final field in
/// the target this reads and no draw here writes one, which is why the y pass
/// LOADS the atlas rather than clearing it (`ShadowTarget::blur`).
@vertex
fn vs_cell(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
    @location(3) who: vec4<f32>,
) -> CellOut {
    return cell_quad(vertex, cell, terms, who.y < 0.5 * DISTANCE_KIND);
}

/// The BLUR cells alone: the y pass.
@vertex
fn vs_cell_blur(
    @builtin(vertex_index) vertex: u32,
    @location(0) rect: vec4<f32>,
    @location(1) cell: vec4<f32>,
    @location(2) terms: vec4<f32>,
    @location(3) who: vec4<f32>,
) -> CellOut {
    return cell_quad(vertex, cell, terms, who.y < 0.5 * DISTANCE_KIND);
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

/// What every weight is dropped by, so the kernel arrives at [`REACH`] σ on
/// zero: the Gaussian's own value there. Past that distance the drop takes a
/// weight below zero and the clamp holds it, so the kernel's support is exactly
/// ±REACH σ however far `ceil` put the loop's last tap.
///
/// Below σ 0.35 texels every tap but the centre clamps away and the blur is a
/// pass-through. That is a blur a third of a texel wide — narrower than the
/// thing it would be blurring — and it is reachable only at a Shadow the eye
/// cannot see at all, the packer holding σ at the cap for every width past
/// about 0.14 of the bar.
const PEDESTAL: f32 = 0.011109;

/// The Gaussian along `axis` at this fragment, over this cell's texels and no
/// others.
///
/// Normalised by the whole kernel and not by the taps that landed inside the
/// cell: outside the rect the ink IS zero, so a tap there contributes nothing
/// and still counts, and the blur of a half-plane comes out at exactly half at
/// its edge. Normalising over the taps that landed would lift every cell's
/// edge back toward its interior.
///
/// The kernel is LOWERED onto zero at [`REACH`] rather than cut there. A
/// Gaussian truncated at three σ still carries `½erfc(3/√2)` ≈ 1.3e-3 of the
/// light at the cut, and a caster's quad ends on that same distance — so the
/// shadow stopped at a step, which `shadow_transmittance` spends as 5.9/255 at
/// the top of the depth bar, in a straight line along the caster's BOX. Every
/// weight is dropped by [`PEDESTAL`] and clamped at zero, which is the standard
/// treatment for a truncated kernel: same taps, same padding, and the blur of a
/// half-plane reaches exactly zero where the quad ends.
///
/// It costs 4.3% of the effective σ — the pedestal comes out of the kernel's
/// own width — so the Shadow bar reads a hair narrower than it did, and that is
/// the whole of what the goldens' re-baseline pays for. Taken against the
/// constant and not against the loop's own last tap, which is where it wants to
/// be written: `radius` is a CEILING, so a pedestal read off it would step
/// every time `ceil` did and swing the effective σ by ±3% as the bar moves,
/// where this holds it at 0.955..0.963 of σ across the whole range.
fn blur(in: CellOut, axis: vec2<i32>) -> f32 {
    let sigma = max(in.sigma, 1.0e-3);
    let radius = min(i32(ceil(REACH * sigma)), MAX_RADIUS);
    let at = vec2<i32>(in.position.xy);
    var sum = 0.0;
    var weight = 0.0;
    for (var i = -radius; i <= radius; i = i + 1) {
        let w = max(exp(-0.5 * f32(i * i) / (sigma * sigma)) - PEDESTAL, 0.0);
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
