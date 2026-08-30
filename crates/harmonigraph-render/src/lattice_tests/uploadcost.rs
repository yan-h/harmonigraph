//! TEMPORARY measurement: what one publication of the mirrored font atlas
//! costs on the GPU, at the sizes a zoom actually grows the atlas to.

use crate::gpu_harness::headless_device;
use crate::text::MirroredAtlas;
use crate::*;

fn atlas_of(size: [usize; 2], key: u64) -> FontAtlas {
    FontAtlas {
        image: std::sync::Arc::new(egui::ColorImage::filled(size, egui::Color32::WHITE)),
        key,
    }
}

#[test]
#[ignore = "a measurement, not an assertion: cargo test -- --ignored --nocapture measure_atlas_upload_cost"]
fn measure_atlas_upload_cost() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    println!(
        "adapter limit max_texture_dimension_2d = {}",
        device.limits().max_texture_dimension_2d
    );

    for (w, height) in
        [(512usize, 256usize), (512, 512), (8192, 512), (8192, 1024), (8192, 2048), (8192, 4096)]
    {
        // Built ONCE, outside the timed loop: a 67 MB `ColorImage::filled` is
        // itself a large memset, and timing it here would charge the upload
        // for an allocation the real path does not make.
        let atlas = atlas_of([w, height], 1);
        let mut mirror = MirroredAtlas::default();
        // The first upload creates the texture; every later one writes in
        // place, which is the case a zoom is in.
        mirror.upload(&device, &queue, &atlas);
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");

        let runs = 20;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            mirror.upload(&device, &queue, &atlas);
        }
        queue.submit(std::iter::empty());
        device.poll(wgpu::PollType::wait_indefinitely()).expect("poll");
        let ms = started.elapsed().as_secs_f64() * 1000.0 / runs as f64;
        println!(
            "{w:5} x {height:5} = {:6.2} MB: {ms:6.2} ms per upload ({:.0} MB/s)",
            w as f64 * height as f64 * 4.0 / 1e6,
            w as f64 * height as f64 * 4.0 / 1e6 / (ms / 1000.0),
        );
    }
}
