// The notes' surround laid over the picture: one draw over the producer's
// rect, after surround.wgsl, reading the two targets its producer drew in
// `prepare` (`crate::surround::SurroundTargets`).
//
// What is under the surround keeps `1 - cut` of itself, and gains the
// strongest note's fill color at `fill × cut`: premultiplied over, so the
// picture loses exactly what the cut takes and the fill puts back only as much.

@group(0) @binding(0) var<uniform> surround: SurroundCut;

/// What [`surround_share`] summed (or kept) per texel, and the strongest
/// note's straight gamma-space fill color, each the size of the rect it is
/// laid over, so a pixel reads its own texel.
@group(1) @binding(0) var surround_cut: texture_2d<f32>;
@group(1) @binding(1) var surround_color: texture_2d<f32>;

struct CoverOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

/// The whole viewport, as `vs_blit` covers it.
@vertex
fn vs_cover(@builtin(vertex_index) vi: u32) -> CoverOut {
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    var out: CoverOut;
    out.position = vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(corner.x, 1.0 - corner.y);
    return out;
}

/// This pixel's surround: the strongest note's fill color, straight, how much
/// of it goes on (`fill × cut`), and how much of the picture the cut takes.
/// The color is read only while the cut fills: a cut to black leaves its color
/// target undrawn.
struct Over {
    color: vec3<f32>,
    weight: f32,
    cut: f32,
};

fn surround_over(in: CoverOut) -> Over {
    let size = vec2<i32>(textureDimensions(surround_cut));
    let texel = min(vec2<i32>(in.uv * vec2<f32>(size)), size - vec2<i32>(1));
    let cut = surround_strength(surround, textureLoad(surround_cut, texel, 0).r);
    var color = vec3<f32>(0.0);
    if surround.fill.x > 0.0 {
        color = textureLoad(surround_color, texel, 0).rgb;
    }
    return Over(color, surround.fill.x * cut, cut);
}

@fragment
fn fs_surround_gamma(in: CoverOut) -> @location(0) vec4<f32> {
    let over = surround_over(in);
    return vec4<f32>(over.color * over.weight, over.cut);
}

/// The same onto a target that encodes sRGB for itself, which wants linear
/// color, as the roll's `fs_core_linear` does for the bodies.
@fragment
fn fs_surround_linear(in: CoverOut) -> @location(0) vec4<f32> {
    let over = surround_over(in);
    return vec4<f32>(linear_from_gamma_rgb(over.color) * over.weight, over.cut);
}
