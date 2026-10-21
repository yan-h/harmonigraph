//! The shipping corpus enumerates actual production constructors. Synthetic
//! reference shaders from other tests do not become shipping build assets.

use crate::wgpu;

#[test]
#[ignore = "manual Metal asset generation/strict coverage"]
fn production_metal_asset_catalog() {
    super::initialize();
    let (_, adapter) = crate::test_gpu_adapter().expect("catalog requires Metal");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    // The one device every shell creates, which includes the backend's
    // internal shaders before any pane is created.
    let (device, queue) =
        pollster::block_on(adapter.request_device(&crate::device_descriptor(&adapter))).unwrap();
    for format in [
        wgpu::TextureFormat::Bgra8Unorm,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Bgra8UnormSrgb,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let mut callbacks = crate::CallbackResources::default();
        let layouts = crate::spectral_shadow::layouts(&device, &mut callbacks);
        drop(crate::LatticeResources::new(&device, &queue, format));
        crate::text::asset_catalog(&device, &queue, format, &layouts);
        crate::roll::asset_catalog(&device, format);
        crate::spectrogram::asset_catalog(&device, format);
        crate::glow::asset_catalog(&device, format);
        crate::dot_shadow::asset_catalog(&device, format, &layouts);
        // The editor's egui renderer and the offline one differ only in filtering.
        for native in [false, true] {
            drop(egui_wgpu::Renderer::new(
                &device,
                format,
                egui_wgpu::RendererOptions {
                    predictable_texture_filtering: !native,
                    ..Default::default()
                },
            ));
        }
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    let stats = super::statistics();
    eprintln!("METAL_ASSET_CATALOG {stats:?}");
    match std::env::var("HARMONIGRAPH_SHADER_ASSETS").as_deref() {
        Ok("strict") => {
            assert!(stats.loaded > 0, "catalog never used generated libraries");
            assert_eq!((stats.source, stats.load_failed, stats.rejected), (0, 0, 0));
        }
        Ok("export" | "source") => {
            assert!(stats.source > 0);
            assert_eq!(stats.loaded, 0);
        }
        _ => panic!("catalog requires an explicit strict, export or source mode"),
    }
}
