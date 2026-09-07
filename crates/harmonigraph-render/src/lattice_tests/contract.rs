//! What the baked WGSL must still say: its entry points, and the constants
//! it has to agree with Rust about.

use crate::*;

#[test]
fn baked_shader_validates() {
    validate_wgsl(
        "lattice.wgsl",
        &with_common(SHADER_SRC),
        common_lines(COMMON_SRC),
        LATTICE_ENTRY_POINTS,
    )
    .expect("baked lattice.wgsl must parse, validate, and keep its entry points");
}

/// The `const _: () = assert!(PITCH_LUT_N == 64)` in `lib.rs` ties one Rust
/// literal to another; the two that decide what the GPU actually reads are in
/// WGSL, where no compiler is checking them against the scene's constant. This
/// is the half that catches a one-sided bump.
///
/// Both slips are silent otherwise. Raise the array but not the const and the
/// shader walks 63 entries of a longer table, painting a top-of-range glyph the
/// color of a pitch halfway down the ramp while the disc under it takes the
/// top — the mismatch #165 closed, reopened at several times the width. Raise
/// the const but not the array and the surplus indices clamp at runtime, which
/// is not a validation error either. naga sees a well-formed shader both ways,
/// `min_binding_size: None` means an over-long buffer never complains, and the
/// scene tests read PITCH_LUT_N symbolically, so they pass at any value.
///
/// Each table is named in its own needle rather than looked for by shape.
/// There are TWO of that length now — `pitch_lut`, walked by pitch, and the
/// `spectral_lut` the audio ring walks by level — and an unnamed
/// `array<vec4<f32>, N>` is satisfied by whichever of them still matches, so
/// it would pass with one table bumped and the other left behind. That is the
/// worse half of the mismatch, not a lesser one: the two sit in one uniform
/// block, so a length that disagrees with the CPU's upload moves every field
/// after it — `spectrum` included — to an offset the shader does not read it
/// at, and the picture that comes back is wrong everywhere rather than in one
/// ramp.
#[test]
fn the_shaders_pitch_luts_are_the_length_the_scene_says() {
    let n = harmonigraph_scene::PITCH_LUT_N;
    let needles = [
        format!("pitch_lut: array<vec4<f32>, {n}>"),
        format!("spectral_lut: array<vec4<f32>, {n}>"),
        format!("const PITCH_LUT_N: u32 = {n}u;"),
    ];
    for needle in &needles {
        assert!(
            SHADER_SRC.contains(needle),
            "lattice.wgsl must declare `{needle}` to match harmonigraph_scene::PITCH_LUT_N \
             ({n}); the CPU uploads that many entries and the GPU would index a different table",
        );
    }
    // And no third table of that shape has appeared without a needle of its
    // own, which is how the two got down to one check in the first place.
    assert_eq!(
        SHADER_SRC.matches(&format!("array<vec4<f32>, {n}>")).count(),
        2,
        "lattice.wgsl declares a table of {n} vec4s that this test does not name; give it a \
         needle, or a one-sided bump to it passes here",
    );
}

/// The shader spends the scene's signed shape in the same normalized
/// exponential `GlowCurve::sample` presents in the UI.
/// A two-power family here would put the low-right S back in the rendered light
/// while the bar continued to show a single-bend curve.
#[test]
fn the_shader_uses_the_scenes_global_glow_curve() {
    for formula in [
        "abs(shape) < 0.05",
        "shape2 * p * (2.0 * p - 1.0) / 12.0",
        "(exp(shape * remaining) - 1.0) / (exp(shape) - 1.0)",
    ] {
        assert!(
            SHADER_SRC.contains(formula),
            "lattice.wgsl must spend the scene's glow shape as `{formula}`",
        );
    }
}

/// Resolve the actual bound types, including every nested offset, scalar kind,
/// vector/matrix shape, array count/stride, size and alignment. The Rust field
/// metadata comes from the declarations themselves (uniforms.rs).
#[test]
fn uniform_transport_matches_nagas_resolved_layout() {
    use crate::uniforms::layout::check_binding;
    check_binding::<Uniforms>(&with_common(SHADER_SRC), 0, 0);
    check_binding::<CompositeParams>(BLIT_SRC, 0, 3);
    // Deliberately shortened from 128 bytes / bloom at 124: blit now binds
    // the first named group only, with no camera/node packing dependency.
    assert_eq!(std::mem::offset_of!(Uniforms, composite), 0);
    assert_eq!(std::mem::size_of::<CompositeParams>(), 16);
    assert_eq!(std::mem::offset_of!(CompositeParams, bloom_strength), 12);
}

/// blit.wgsl has no hot-reload path, so a broken edit would otherwise
/// first surface as a pipeline panic inside a DAW.
#[test]
fn baked_blit_shader_validates() {
    let module = naga::front::wgsl::parse_str(BLIT_SRC)
        .map_err(|e| e.emit_to_string(BLIT_SRC))
        .expect("blit.wgsl must parse");
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .expect("blit.wgsl must validate");
    for required in [
        "vs_blit",
        "fs_blit",
        "fs_bright",
        "fs_blur_h",
        "fs_blur_v",
        "fs_composite",
        "fs_bloom_add",
    ] {
        assert!(
            module.entry_points.iter().any(|ep| ep.name == required),
            "missing entry point `{required}`"
        );
    }
}

/// The ink strip is as wide as the texture it is drawn into.
///
/// Two constants, one number: [`INK_STRIP_N`] sizes the texture and every
/// index into it on the Rust side, and lattice.wgsl's own `INK_STRIP_N` is
/// what the shader walks, samples and wraps at. Nothing checks them against
/// each other — the strip is a colour attachment, so no binding size is
/// validated, and both halves are well-formed at any value.
///
/// A one-sided bump is silent and disfigures every node. Raise the shader's
/// alone and the read pass writes columns past the target's edge, which are
/// dropped, so the blur averages the ink of a partial turn and calls it the
/// whole; raise the Rust one alone and the columns past the shader's idea of a
/// turn are never written, and the light takes its colour from a cleared texel
/// wherever the fragment's angle lands in them.
#[test]
fn the_shaders_ink_strip_is_as_wide_as_the_texture_it_is_drawn_into() {
    let needle = format!("const INK_STRIP_N: u32 = {INK_STRIP_N}u;");
    assert!(
        SHADER_SRC.contains(&needle),
        "lattice.wgsl must declare `{needle}` to match INK_STRIP_N here ({INK_STRIP_N}); the \
         strip is allocated that wide and the shader would walk a different turn",
    );
}

#[test]
fn octave_packing_matches_the_documented_layout() {
    let mut levels = [0.0f32; harmonigraph_scene::OCTAVE_SLOTS];
    levels[0] = 1.0; // lowest byte of word 0
    levels[3] = 0.5; // highest byte of word 0
    levels[4] = 1.0; // lowest byte of word 1
    levels[9] = 1.0; // second byte of word 2
    let words = pack_octaves(&levels);
    assert_eq!(words[0] & 0xFF, 255);
    assert_eq!((words[0] >> 24) & 0xFF, 128);
    assert_eq!(words[1] & 0xFF, 255);
    assert_eq!(words[2] & 0xFF, 0);
    assert_eq!((words[2] >> 8) & 0xFF, 255);
    // Out-of-range levels clamp instead of corrupting neighbors.
    let words = pack_octaves(&[2.0; harmonigraph_scene::OCTAVE_SLOTS]);
    assert_eq!(words[0], 0xFFFF_FFFF);
}
