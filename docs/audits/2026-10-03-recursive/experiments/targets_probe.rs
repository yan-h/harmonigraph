#[test]
fn audit_halo_only_resize_prepare_cpu_probe() {
    let (device, queue) = headless_device().expect("GPU required for allocation probe");
    let _split = SplitOverride::set(None);
    let mut cb = star_fixture([1920, 1080], egui::Pos2::ZERO);
    let a = cb.atmosphere.as_mut().unwrap();
    a.settings.stars.star_halo_profile = harmonigraph_scene::StarHaloProfile::Uniform;
    a.settings.stars.star_halo_resolution = 0.5;
    a.settings.color_pickup = 0.6;
    a.settings.color_release = 0.6;
    let mut resources = CallbackResources::default();
    let screen = ScreenDescriptor { size_in_pixels: [1920, 1080], pixels_per_point: 1.0 };
    let prepare_cpu_us = |cb: &SpectrogramCallback, resources: &mut CallbackResources| {
        let mut encoder = device.create_command_encoder(&Default::default());
        let start = std::time::Instant::now();
        let buffers = cb.prepare(&device, &queue, &screen, &mut encoder, resources);
        let cpu_us = start.elapsed().as_secs_f64() * 1_000_000.0;
        queue.submit(buffers.into_iter().chain([encoder.finish()]));
        device.poll(wgpu::PollType::wait_indefinitely()).expect("drain probe frame");
        cpu_us
    };
    for _ in 0..3 { cb.pass_nr += 1; prepare_cpu_us(&cb, &mut resources); }
    let old_source = target(&resources).source_view.clone();
    let old_shapes = { let t = target(&resources); (t.size, t.tone_size(), t.near_size(), t.tile_texels(), t.star_size(), t.memory_size()) };
    let old_halos = target(&resources).halo_layout();
    assert!(old_halos.is_some() && old_shapes.5.is_some());
    cb.atmosphere.as_mut().unwrap().settings.stars.star_halo_resolution = 0.25;
    cb.pass_nr += 1;
    prepare_cpu_us(&cb, &mut resources);
    let t = target(&resources);
    assert_ne!(t.halo_layout(), old_halos);
    assert_eq!((t.size, t.tone_size(), t.near_size(), t.tile_texels(), t.star_size(), t.memory_size()), old_shapes, "another allocation shape changed");
    assert_ne!(t.source_view, old_source, "halo-only change did not rebuild source");
    let mut stable = Vec::with_capacity(30);
    let mut changing = Vec::with_capacity(30);
    for pair in 0..30 {
        for change in if pair % 2 == 0 { [false, true] } else { [true, false] } {
            let source = target(&resources).source_view.clone();
            if change {
                let s = &mut cb.atmosphere.as_mut().unwrap().settings.stars;
                s.star_halo_resolution = if s.star_halo_resolution == 0.25 { 0.5 } else { 0.25 };
            }
            cb.pass_nr += 1;
            let us = prepare_cpu_us(&cb, &mut resources);
            if change { changing.push(us); assert_ne!(target(&resources).source_view, source); }
            else { stable.push(us); assert_eq!(target(&resources).source_view, source); }
        }
    }
    eprintln!("stable_samples_us={stable:?}\nchanging_samples_us={changing:?}");
    let percentiles = |samples: &mut Vec<f64>| { samples.sort_by(f64::total_cmp); ((samples[14]+samples[15])/2.0, samples[28]) };
    let stable_us = percentiles(&mut stable);
    let changing_us = percentiles(&mut changing);
    eprintln!("halo-only prepare CPU us stable median/p95={stable_us:?}; changing median/p95={changing_us:?}; median_delta={}", changing_us.0-stable_us.0);
}
