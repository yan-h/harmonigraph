
// Lossless transition checks; this is a scratch probe, not a timing benchmark.
fn fusion_parity_fixture(profile: &str) -> SpectrogramCallback {
    let root = "/private/tmp/stars-full-halo-compare";
    let mut cb = cloud_fixture();
    cb.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(960.0, 540.0));
    let levels = std::fs::read(format!("{root}/take-levels.u8")).unwrap();
    assert_eq!(levels.len(), 960 * 1024);
    cb.grid = grid_of(Arc::new(levels), 1024, 960, 0);
    relay_quad(&mut cb, 960);
    let span = 12.0 * (9000.0_f32 / 70.0).log2();
    cb.read = read_of(35.0, span, 540);
    cb.read.spectrum_min_midi = 35.0;
    cb.read.bins_per_semitone = 1024.0 / span;
    cb.read.level_per_midi = 0.0;
    cb.read.level_per_step = 1.0 / 255.0;
    cb.shades.lut = Arc::new(std::fs::read(format!("{root}/palette.rgba")).unwrap()
        .chunks_exact(4).map(|p| p.try_into().unwrap()).collect());
    let a = cb.atmosphere.as_mut().unwrap();
    a.region = cb.rect;
    a.points_per_cent = 540.0 / (span * 100.0);
    a.points_per_ms = 960.0 / 20000.0;
    a.settings = SpectralAtmosphere {
        cloud_style: CloudStyle::Stars, cloud_depth: 1.0,
        star_halo_resolution: 1.0, star_jitter: 1.0, ..Default::default()
    };
    apply_research_profile(&mut a.settings, profile);
    assert!(a.settings.color_pickup > 0.0 || a.settings.color_release > 0.0,
        "fusion parity requires retained memory; memory-off only exercises fallback");
    cb
}

#[test]
#[ignore = "scratch lossless fusion parity, requires GPU and private take fixture"]
fn stars_research_fusion_parity() {
    let profile = research_profile();
    let (device, queue) = research_device();
    let out = std::env::var("RESEARCH_OUTPUT")
        .unwrap_or_else(|_| format!("/private/tmp/stars-investigation/fusion-parity/{profile}"));
    std::fs::create_dir_all(&out).unwrap();
    // Native 960x540 RGBA frames. The atlas geometry is pane-relative and unchanged.
    let mut cases = [
        ("full-a", fusion_parity_fixture(&profile), CallbackResources::default()),
        ("full-fused", fusion_parity_fixture(&profile), CallbackResources::default()),
    ];
    let lifetime = f64::from(cases[0].1.atmosphere.unwrap().settings.star_lifetime);
    assert!(lifetime < 3.25, "the bounded silent-life fixture must cross every cell's lifetime");
    assert!(31.0 > 6.0 * f64::from(harmonigraph_scene::atmosphere::COLOR_MEMORY_MAX));
    let mut schedule = Vec::new();
    for i in 0..=20 {
        schedule.push((1.0 + f64::from(i) * 0.1,
            if i == 0 { "cold" } else if i == 20 { "warm" } else { "" }, false));
    }
    for i in 1..=12 {
        schedule.push((3.0 + f64::from(i) * 0.25, if i == 12 { "life" } else { "" }, false));
    }
    schedule.extend([(37.0, "reset", false), (37.1, "recovery", false), (37.2, "release", true)]);
    for i in 1..=13 {
        schedule.push((37.2 + f64::from(i) * 0.25, if i == 13 { "silent-life" } else { "" }, false));
    }
    let mut failures = Vec::new();
    let mut log = String::from("frame,time,checkpoint,different_bytes,max_byte_difference\n");
    let mut release = Vec::new();
    let mut last = Vec::new();
    for (frame, (now, checkpoint, silence)) in schedule.iter().copied().enumerate() {
        let mut rendered = Vec::new();
        for (name, cb, resources) in &mut cases {
            activate(name);
            if silence {
                // New grid identity triggers an upload; same dimensions and source
                // end preserve memory validity, so this exercises release, not reset.
                cb.grid = grid_of(Arc::new(vec![0; 960 * 1024]), 1024, 960, 0);
            }
            cb.pass_nr = frame as u64 + 1;
            cb.atmosphere.as_mut().unwrap().now = now;
            let rgba = scratch_frame_at_ppp(&device, &queue, resources, cb, 1.0);
            if silence {
                assert_eq!(cb.grid.full_uploads.load(Ordering::Relaxed), 1,
                    "the silence transition must actually replace the uploaded light grid");
            }
            if !checkpoint.is_empty() {
                std::fs::write(format!("{out}/{checkpoint}-{name}.rgba"), &rgba).unwrap();
            }
            rendered.push(rgba);
        }
        let differences = rendered[0].iter().zip(&rendered[1]).filter(|(a,b)| a != b).count();
        let maximum = rendered[0].iter().zip(&rendered[1]).map(|(a,b)| a.abs_diff(*b)).max().unwrap();
        log.push_str(&format!("{frame},{now:.9},{checkpoint},{differences},{maximum}\n"));
        if differences != 0 { failures.push(format!("frame {frame} t={now:.3}: {differences} bytes, max {maximum}")); }
        if checkpoint == "reset" {
            // The 31s jump reaches update()'s invalid-history branch. Fresh
            // resources provide an independent cold-start image at the same time.
            activate("full-a");
            let mut fresh = fusion_parity_fixture(&profile);
            fresh.pass_nr = 1;
            fresh.atmosphere.as_mut().unwrap().now = now;
            let cold = scratch_frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &fresh, 1.0);
            std::fs::write(format!("{out}/reset-cold-reference.rgba"), &cold).unwrap();
            if cold != rendered[0] { failures.push("reset baseline differs from cold reference".into()); }
            if cold != rendered[1] { failures.push("reset fused differs from cold reference".into()); }
        }
        if checkpoint == "release" { release = rendered[0].clone(); }
        last = rendered;
    }
    activate("full-a");
    let mut silent = fusion_parity_fixture(&profile);
    silent.grid = grid_of(Arc::new(vec![0; 960 * 1024]), 1024, 960, 0);
    silent.pass_nr = 1;
    silent.atmosphere.as_mut().unwrap().now = schedule.last().unwrap().0;
    let floor = scratch_frame_at_ppp(&device, &queue, &mut CallbackResources::default(), &silent, 1.0);
    std::fs::write(format!("{out}/silent-cold-reference.rgba"), &floor).unwrap();
    if release == floor { failures.push("silent transition never reached retained visible color".into()); }
    for (i, image) in last.iter().enumerate() {
        if image != &floor { failures.push(format!("{} retained stars after every silent life reset", cases[i].0)); }
    }
    std::fs::write(format!("{out}/parity.csv"), log).unwrap();
    activate("");
    assert!(failures.is_empty(), "fusion parity failed:\n{}", failures.join("\n"));
}
