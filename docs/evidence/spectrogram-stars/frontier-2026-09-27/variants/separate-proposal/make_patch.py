#!/usr/bin/env python3
"""Read current research checkout and emit an unapplied separate-target patch."""
from pathlib import Path
import difflib
import sys
root = Path(sys.argv[1])
out = Path(__file__).parent
patch = []

def once(text, before, after):
    assert text.count(before) == 1, (before[:100], text.count(before))
    return text.replace(before, after)

path = "crates/harmonigraph-render/src/spectrogram/atmosphere.rs"
original = (root / path).read_text()
s = original

# Preserve baseline entries, extending only the active research layout.
start = s.index("        let composite_layout = device.create_bind_group_layout(")
end = s.index("        let shader = device.create_shader_module(", start)
block = s[start:end]
entries = block[block.index("            entries: &[") + len("            entries: &["):block.rindex("            ],")]
replacement = "        let mut composite_entries = vec![" + entries + r'''
        ];
        #[cfg(test)]
        if super::tests::research::separate_halos() {
            // Existing group1 has eight textures; group0 has one LUT. Five
            // added textures make fourteen total, not nineteen array layers.
            assert!(device.limits().max_sampled_textures_per_shader_stage >= 14);
            composite_entries.extend((11..16).map(texture));
        }
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectral_cloud_composite_layout"),
            entries: &composite_entries,
        });
'''
s = once(s, block, replacement)

# Avoid allocating the full-size array: leave only a tiny compatibility binding.
constructor = r'''
        #[cfg(test)]
        if super::tests::research::separate_halos() {
            let dummy = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("research_halo_dummy_array"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: STAR_SLICES as u32 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: STAR_FAR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let layers = std::array::from_fn(|layer| {
                let extent = super::tests::research::halo_extent(size, layer);
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("research_separate_halo"),
                    size: wgpu::Extent3d { width: extent[0], height: extent[1], depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: STAR_FAR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }).create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    ..Default::default()
                })
            });
            return Self {
                view: dummy.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                }),
                layers,
                // Base dimensions still feed research_halo_size's constants.
                size,
            };
        }
'''
anchor = 'impl StarHalos {\n    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {'
s = once(s, anchor, anchor + constructor)

# Carry the appropriate extra views along with the old array view. This keeps
# the halo write pass on scratch for all five new bindings, not just binding10.
start = s.index("        let cloud_group = |front:")
end = s.index("        let scratch_tile =", start)
block = s[start:end]
signature = block[:block.index("            device.create_bind_group")]
signature = once(signature, "halos: &wgpu::TextureView|", "halos: (&wgpu::TextureView, &[wgpu::TextureView; STAR_SLICES])|")
entries = block[block.index("                entries: &[") + len("                entries: &["):block.rindex("                ],")]
entries = once(entries, "wgpu::BindingResource::TextureView(halos)", "wgpu::BindingResource::TextureView(halos.0)")
replacement = signature + "            let mut entries = vec![" + entries + r'''
            ];
            #[cfg(test)]
            if super::tests::research::separate_halos() {
                entries.extend(halos.1.iter().enumerate().map(|(layer, view)| wgpu::BindGroupEntry {
                    binding: 11 + layer as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                }));
            }
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spectral_cloud_composite_group"),
                layout: &pipelines.composite_layout,
                entries: &entries,
            })
        };
'''
s = once(s, block, replacement)
s = once(s,
    "        let halo_view = halos.as_ref().map_or(&halo_scratch.view, |halo| &halo.view);",
    "        let halo_view = halos.as_ref().map_or((&halo_scratch.view, &halo_scratch.layers), |halo| (&halo.view, &halo.layers));")
s = once(s,
    "                &halo_scratch.view,\n            )",
    "                (&halo_scratch.view, &halo_scratch.layers),\n            )")
# For real textures use their captured physical dimensions. This also witnesses
# that these are not subrects of the maximum allocation at draw time.
s = once(s,
    "                let extent = super::tests::research::halo_extent(halos.size, layer);",
    '''                let extent = if super::tests::research::separate_halos() {
                    let actual = view.texture().size();
                    let expected = super::tests::research::halo_extent(halos.size, layer);
                    assert_eq!([actual.width, actual.height], expected);
                    [actual.width, actual.height]
                } else {
                    super::tests::research::halo_extent(halos.size, layer)
                };''')
patch.extend(difflib.unified_diff(original.splitlines(True), s.splitlines(True), fromfile="a/"+path, tofile="b/"+path))

path = "crates/harmonigraph-render/src/spectrogram/tests/research.rs"
original = (root / path).read_text()
s = original
cases = ""
for values in ["100_100_100_100_100", "50_50_50_50_50", "50_50_50_100_100", "50_50_100_100_100", "50_50_80_100_50"]:
    cases += f'    ("full-separate-v{values}", Some(|s| {{ s.cloud_style = CloudStyle::Stars; s.star_halo_resolution = 1.0; memory(s); }})),\n'
s = once(s, 'const CASES: &[(&str, Option<Turn>)] = &[', 'const CASES: &[(&str, Option<Turn>)] = &[\n' + cases)
s = once(s, 'fn activate(name: &str) {', '''pub(in crate::spectrogram) fn separate_halos() -> bool {
    ACTIVE.with_borrow(|name| name.starts_with("full-separate-v"))
}
fn activate(name: &str) {''')
start = s.index('    if factors(name) != [1.0; 5]')
end = s.index('    if fused() {', start)
block = s[start:end]
condition_end = block.index('{')
body = block[condition_end+1:]
body = once(body,
    '        source = source.replace("pt / cloud.size, i32(k), 0.0)",',
    '        if !separate_halos() { source = source.replace("pt / cloud.size, i32(k), 0.0)",')
line_start = body.index('        if !separate_halos() {')
line_end = body.index('\n', line_start)
body = body[:line_end] + ' }' + body[line_end:]
replacement = '    if factors(name) != [1.0; 5] || name == "full-identity" || separate_halos() {' + body
replacement += r'''
    if separate_halos() {
        assert_eq!(compute_mode(), 0, "separate textures are fragment-only");
        assert!(!fused() && u8_range(name).is_none());
        let old = "textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0)";
        assert_eq!(source.matches(old).count(), 1, "halo sample moved");
        source = source.replace(old, "research_separate_halo(pt / cloud.size, k)");
        source.push_str(&std::fs::read_to_string(
            "/private/tmp/stars-investigation/separate-proposal/separate.wgsl"
        ).unwrap());
    }
'''
s = once(s, block, replacement)
patch.extend(difflib.unified_diff(original.splitlines(True), s.splitlines(True), fromfile="a/"+path, tofile="b/"+path))
(out / "separate-halos.patch").write_text("".join(patch))
print(out / "separate-halos.patch")
