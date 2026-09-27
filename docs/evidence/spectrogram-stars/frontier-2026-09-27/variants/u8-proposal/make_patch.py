#!/usr/bin/env python3
"""Emit an unapplied scratch RGBA8 halo experiment. Never edits repository."""
from pathlib import Path
import difflib
import sys
root = Path(sys.argv[1])
out = Path(__file__).parent
patch = []
def edit(relative, replacements):
    original = (root / relative).read_text()
    changed = original
    for old, new in replacements:
        assert changed.count(old) == 1, (relative, old[:100], changed.count(old))
        changed = changed.replace(old, new)
    patch.extend(difflib.unified_diff(original.splitlines(True), changed.splitlines(True),
                                    fromfile="a/"+relative, tofile="b/"+relative))

helper = '''
// Test-only format experiments preserve the native far-layer intermediate.
fn star_halo_format() -> wgpu::TextureFormat {
    #[cfg(test)]
    { super::tests::research::halo_format() }
    #[cfg(not(test))]
    { STAR_FAR_FORMAT }
}
'''
edit("crates/harmonigraph-render/src/spectrogram/atmosphere.rs", [
    ("const STAR_FAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;",
     "const STAR_FAR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;\n" + helper),
    ('                "fs_star_halo",\n                &[Some(STAR_FAR_FORMAT)],',
     '                "fs_star_halo",\n                &[Some(star_halo_format())],'),
    ('            format: STAR_FAR_FORMAT,\n            usage,',
     '            format: star_halo_format(),\n            usage,'),
])

cases = ""
for res, value in [("half", 0.5), ("full", 1.0)]:
    for limit in [4, 9]:
        cases += f'''    ("{res}-u8r{limit}", Some(|s| {{ s.cloud_style = CloudStyle::Stars; s.star_halo_resolution = {value}; memory(s); }})),\n'''
# Diagnostic names are image-only; deliberately not benchmark cases.
helpers = r'''
pub(in crate::spectrogram) fn halo_format() -> wgpu::TextureFormat {
    ACTIVE.with_borrow(|name| {
        if name.contains("-u8r") { wgpu::TextureFormat::Rgba8Unorm }
        else { wgpu::TextureFormat::Rgba16Float }
    })
}
fn u8_range(name: &str) -> Option<f32> {
    if name.contains("-u8r4") { Some(4.0) }
    else if name.contains("-u8r9") { Some(9.0) }
    else { None }
}
'''
source_change = r'''
    if let Some(range) = u8_range(name) {
        assert!(!name.contains("compute") && !fused(), "u8 experiment is fragment-only");
        let original_return = "    return halo;\n}";
        assert_eq!(source.matches(original_return).count(), 1, "halo return changed");
        let clipping_diagnostic = name.ends_with("-clip");
        let write = if clipping_diagnostic {
            // Binary R8 mask of any channel outside the chosen legal range.
            // RGB<=coverage analytically; checking all four also catches a bug.
            format!("    let clipped = any(halo > vec4<f32>({range:.1}));\n    return vec4<f32>(select(0.0, 1.0, clipped), 0.0, 0.0, 1.0);\n}}")
        } else {
            format!("    return halo / {range:.1};\n}}")
        };
        source = source.replace(original_return, &write);
        let read = "slice += textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0);";
        assert_eq!(source.matches(read).count(), 1, "halo read changed");
        source = source.replace(read, &format!("slice += {range:.1} * textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0);"));
        if clipping_diagnostic {
            // A dedicated image diagnostic, not a timing variant or usable look.
            // Disable split so its independent native target cannot hide a mask.
            STAR_SPLIT_OVERRIDE.set(Some(false));
            let read = "return star_layers(pt, 0u, STAR_SLICES, palette_color(0.0));";
            assert_eq!(source.matches(read).count(), 1, "star_color changed");
            source = source.replace(read, "return research_u8_clipping(pt);");
            source.push_str(r#"
fn research_u8_clipping(pt: vec2<f32>) -> vec3<f32> {
    var peak = 0.0;
    for (var k = 0u; k < STAR_SLICES; k += 1u) {
        peak = max(peak, textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0).r);
    }
    return vec3<f32>(peak, 0.0, 0.0);
}
"#);
        }
    }
'''
edit("crates/harmonigraph-render/src/spectrogram/tests/research.rs", [
    ('const CASES: &[(&str, Option<Turn>)] = &[', 'const CASES: &[(&str, Option<Turn>)] = &[\n' + cases),
    ('fn activate(name: &str) {', helpers + '\nfn activate(name: &str) {'),
    ('    SOURCE.with_borrow_mut(|s| *s = Some(source));', source_change + '\n    SOURCE.with_borrow_mut(|s| *s = Some(source));'),
])
(out / "rgba8-halos.patch").write_text("".join(patch))
print(out / "rgba8-halos.patch")
