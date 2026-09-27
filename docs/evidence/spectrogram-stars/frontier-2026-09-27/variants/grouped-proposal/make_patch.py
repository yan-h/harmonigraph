#!/usr/bin/env python3
"""Emit an unapplied follow-up to the five-separate-texture research patch."""
from pathlib import Path
import difflib
import sys
root = Path(sys.argv[1])
out = Path(__file__).parent
patch = []
def once(text, before, after):
    assert text.count(before) == 1, (before[:100], text.count(before))
    return text.replace(before, after)
def finish(path, original, changed):
    patch.extend(difflib.unified_diff(original.splitlines(True), changed.splitlines(True),
                                    fromfile="a/"+path, tofile="b/"+path))

path = "crates/harmonigraph-render/src/spectrogram/tests/research.rs"
original = (root / path).read_text()
s = original
cases = ""
for values in ["100_100_100_100_100", "50_50_50_50_50", "50_50_80_100_50", "50_50_100_100_60", "50_50_100_100_100", "25_25_50_50_50"]:
    cases += f'    ("full-grouped-v{values}", Some(|s| {{ s.cloud_style = CloudStyle::Stars; s.star_halo_resolution = 1.0; memory(s); }})),\n'
s = once(s, 'const CASES: &[(&str, Option<Turn>)] = &[', 'const CASES: &[(&str, Option<Turn>)] = &[\n' + cases)
s = once(s, 'pub(in crate::spectrogram) fn separate_halos() -> bool {', (out / "plan.rs").read_text() + '\npub(in crate::spectrogram) fn separate_halos() -> bool {')
s = once(s,
    'if factors(name) != [1.0; 5] || name == "full-identity" || separate_halos() {',
    'if factors(name) != [1.0; 5] || name == "full-identity" || separate_halos() || grouped_halos() {')
s = once(s, 'if !separate_halos() { source = source.replace(',
    'if !separate_halos() && !grouped_halos() { source = source.replace(')
source = r'''
    if grouped_halos() {
        assert_eq!(compute_mode(), 0, "grouped textures are fragment-only");
        assert!(!fused() && u8_range(name).is_none());
        let plan = grouped_halo_plan();
        let groups = plan.group.iter().map(|g| format!("{g}u")).collect::<Vec<_>>().join(",");
        let layers = plan.layer.iter().map(|k| format!("{k}i")).collect::<Vec<_>>().join(",");
        let mut helper = std::fs::read_to_string(
            "/private/tmp/stars-investigation/grouped-proposal/grouped.wgsl"
        ).unwrap();
        assert_eq!(helper.matches("__GROUPS__").count(), 1);
        assert_eq!(helper.matches("__LAYERS__").count(), 1);
        helper = helper.replace("__GROUPS__", &groups).replace("__LAYERS__", &layers);
        let old = "textureSampleLevel(star_halos, cloud_sampler, pt / cloud.size, i32(k), 0.0)";
        assert_eq!(source.matches(old).count(), 1, "halo sample moved");
        source = source.replace(old, "research_grouped_halo(pt / cloud.size, k)");
        source.push_str(&helper);
    }
'''
s = once(s, '    if fused() {\n        source.push_str(', source + '\n    if fused() {\n        source.push_str(')
finish(path, original, s)

path = "crates/harmonigraph-render/src/spectrogram/atmosphere.rs"
original = (root / path).read_text()
s = original
layout = r'''
        #[cfg(test)]
        if super::tests::research::grouped_halos() {
            // Eight original group1 textures + three arrays + group0 LUT.
            assert!(device.limits().max_sampled_textures_per_shader_stage >= 12);
            composite_entries.extend((11..14).map(|binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }));
        }
'''
s = once(s, '        let composite_layout = device.create_bind_group_layout(', layout + '\n        let composite_layout = device.create_bind_group_layout(')
constructor = r'''
        #[cfg(test)]
        if super::tests::research::grouped_halos() {
            let plan = super::tests::research::grouped_halo_plan();
            let arrays = plan.factors.iter().enumerate().map(|(group, &factor)| {
                let extent = size.map(|n| (n as f32 * factor).ceil().max(1.0) as u32);
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("research_grouped_halos"),
                    size: wgpu::Extent3d {
                        width: extent[0], height: extent[1], depth_or_array_layers: plan.counts[group],
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: STAR_FAR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
            }).collect::<Vec<_>>();
            let layers = std::array::from_fn(|k| {
                arrays[plan.group[k]].create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: plan.layer[k],
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            });
            let dummy = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("research_grouped_dummy_array"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: STAR_SLICES as u32 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: STAR_FAR_FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            return Self {
                view: dummy.create_view(&wgpu::TextureViewDescriptor {
                    dimension: Some(wgpu::TextureViewDimension::D2Array),
                    ..Default::default()
                }),
                layers,
                size,
            };
        }
'''
anchor = 'impl StarHalos {\n    fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {'
s = once(s, anchor, anchor + constructor)

# These local array views live until create_bind_group captures their resources.
# The existing halo tuple chooses real or scratch layer textures consistently.
groups = r'''
            #[cfg(test)]
            let grouped_views: Option<[wgpu::TextureView; 3]> =
                super::tests::research::grouped_halos().then(|| {
                    let plan = super::tests::research::grouped_halo_plan();
                    std::array::from_fn(|slot| {
                        // Unused fixed bindings alias group zero. The shader's
                        // constant map cannot select them, but bindings are valid.
                        let group = if slot < plan.counts.len() { slot } else { 0 };
                        let representative = plan.group.iter().position(|&g| g == group).unwrap();
                        let texture = halos.1[representative].texture();
                        assert_eq!(texture.size().depth_or_array_layers, plan.counts[group]);
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            dimension: Some(wgpu::TextureViewDimension::D2Array),
                            base_array_layer: 0,
                            array_layer_count: Some(plan.counts[group]),
                            ..Default::default()
                        })
                    })
                });
            #[cfg(test)]
            if let Some(views) = grouped_views.as_ref() {
                entries.extend(views.iter().enumerate().map(|(group, view)| wgpu::BindGroupEntry {
                    binding: 11 + group as u32,
                    resource: wgpu::BindingResource::TextureView(view),
                }));
            }
'''
anchor = '            device.create_bind_group(&wgpu::BindGroupDescriptor {\n                label: Some("spectral_cloud_composite_group"),'
s = once(s, anchor, groups + '\n' + anchor)
s = once(s,
    'let extent = if super::tests::research::separate_halos() {',
    'let extent = if super::tests::research::separate_halos() || super::tests::research::grouped_halos() {')
finish(path, original, s)
(out / "grouped-halos.patch").write_text("".join(patch))
print(out / "grouped-halos.patch")
