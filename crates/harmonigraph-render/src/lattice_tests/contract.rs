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

/// The shader spends the two powers the CPU derives from the editable point in
/// the same global double-power family `GlowCurve::sample` presents in the UI.
/// A piecewise interpolant here would put the elbow back in the rendered light
/// while the editor continued to show the smooth curve.
#[test]
fn the_shader_uses_the_scenes_global_glow_curve() {
    let formula = "pow(max(1.0 - pow(p, u.glow_curve.x), 0.0), u.glow_curve.y)";
    assert!(
        SHADER_SRC.contains(formula),
        "lattice.wgsl must spend the scene's two glow powers as `{formula}`",
    );
}

/// The field names `struct {name} { ... }` declares in `src`, in order — a
/// `//` or `///` comment line is skipped, and each remaining non-blank line
/// contributes the identifier before its first `:`. Neither language's
/// struct is parsed for real; this reads both the same shallow way the
/// [`the_shaders_pitch_lut_is_the_length_the_scene_says`] needle check
/// does, which is enough to catch the two lists disagreeing.
///
/// Assumes one field per line, which every field in both of today's structs
/// is short enough to be. A field whose type needs wrapping to a second line
/// panics here instead of parsing wrong — loud, but a confusing place to
/// land for whoever adds it, since the message names no field and no line.
fn struct_field_names(src: &str, name: &str) -> Vec<String> {
    let after_kw = src.split_once(&format!("struct {name}")).expect("struct not found").1;
    let body_start = after_kw.find('{').expect("struct has no body") + 1;
    let mut depth = 1u32;
    let mut end = body_start;
    for (i, c) in after_kw[body_start..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = body_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    after_kw[body_start..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("//"))
        .map(|l| l.split_once(':').expect("field line has no `:`").0.trim().to_string())
        .collect()
}

/// `misc`..`misc8` carry the picture's knobs packed several to a vec4 (see
/// the doc comments on [`Uniforms`] and its WGSL twin), and nothing checks
/// the two structs against each other: naga validates the WGSL side against
/// itself, rustc the Rust side against itself, and a slot added, dropped,
/// renamed, or reordered on only one side still compiles and validates —
/// only the byte offsets downstream of it drift, so every read after the
/// mismatch lands on the wrong vec4's `.x`/`.y`/`.z`/`.w`. Comparing the
/// field-name lists is the cheap half of the guard; the doc comments above
/// each field are the other half; a `.w` typo'd for a `.z` within an
/// otherwise-correctly-paired slot is neither this test's job nor the
/// PITCH_LUT_N one's — see their doc comments.
#[test]
fn the_uniforms_slots_pair_up_between_rust_and_wgsl() {
    let rust_fields = struct_field_names(include_str!("../lib.rs"), "Uniforms");
    let wgsl_fields = struct_field_names(SHADER_SRC, "Uniforms");
    assert_eq!(
        rust_fields, wgsl_fields,
        "lib.rs's Uniforms and lattice.wgsl's Uniforms must declare the same fields in the \
         same order — they describe one GPU buffer from two ends, and every field here is a \
         multiple of 16 bytes, which is what lets Rust's #[repr(C)] layout match WGSL's without \
         either side spelling out padding; a name added, dropped, renamed, or reordered on only \
         one side is exactly what desyncs the offsets.",
    );
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
