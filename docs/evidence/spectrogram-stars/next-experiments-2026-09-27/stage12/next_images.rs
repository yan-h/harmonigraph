//! Temporary ignored capture of production rendering, using the frozen research fixture.
//! No source substitutions, render-path overrides, or experimental resource hooks.
use super::*;
use harmonigraph_scene::{CloudStyle, MaterialSettings, SpectralAtmosphere, StarHaloProfile};

// Pin the defaults compiled into frozen-research/research-render (base fd7c8f9),
// rather than letting unrelated future appearance defaults move the reference.
fn captured_settings(profile: StarHaloProfile, resolution: f32) -> SpectralAtmosphere {
    SpectralAtmosphere {
        pitch_softness: 6.726_529_6,
        time_softness: 0.0,
        spread: 0.0,
        blur_time_step: 1.0,
        contour_strength: 1.0,
        contours: 17.0,
        contour_softness: 0.492_202_6,
        cloud_depth: 1.0,
        color_pickup: 0.043_984_346,
        color_release: 0.711_714_74,
        cloud_speed: 4.242_738_7,
        cloud_direction: 174.0,
        cloud_style: CloudStyle::Stars,
        material_settings: MaterialSettings::default(), // Inert in Stars.
        star_density: 10.0,
        star_randomness: 0.080912866,
        star_jitter: 0.5,
        star_size_min: 2.315533,
        star_size_max: 14.752405,
        star_size_curve: 2.1178954,
        star_speed_min: 0.08931082,
        star_speed_max: 0.16860056,
        star_speed_curve: 3.179647,
        star_lifetime: 2.9719827,
        star_fringe: 0.5,
        star_halo_resolution: resolution,
        star_halo_profile: profile,
        star_defocus: 0.35391274,
    }
}

#[test]
#[ignore]
fn stars_next_images() {
 let (device,queue)=headless_device().expect("GPU required");
 let out=std::env::var("IMAGE_OUTPUT").unwrap(); std::fs::create_dir_all(&out).unwrap();
 let ppp:f32=std::env::var("PROBE_PPP").unwrap_or("2".into()).parse().unwrap();
 let frames:u64=std::env::var("IMAGE_FRAMES").unwrap_or("3".into()).parse().unwrap();
 let offset:f32=std::env::var("IMAGE_ORIGIN").unwrap_or("0".into()).parse().unwrap();
 let names=std::env::var("RESEARCH_CASES").unwrap_or("p3-a,p3-complete,full-a,full-complete".into());
 let fixture="/private/tmp/stars-full-halo-compare";
 let palette=std::fs::read(format!("{fixture}/palette.rgba")).unwrap();
 for name in names.split(',') {
  next_timing::activate(name);
  for input in ["take","flat"] {
   let mut cb=cloud_fixture();
   cb.rect=egui::Rect::from_min_size(egui::pos2(offset,offset),egui::vec2(960.0,540.0));
   let bytes=std::fs::read(format!("{fixture}/{input}-levels.u8")).unwrap();
   cb.grid=grid_of(Arc::new(bytes),1024,960,0); relay_quad(&mut cb,960);
   let span=12.0*(9000.0_f32/70.0).log2(); cb.read=read_of(35.0,span,540);
   cb.read.spectrum_min_midi=35.0; cb.read.bins_per_semitone=1024.0/span;
   cb.read.level_per_midi=0.0; cb.read.level_per_step=1.0/255.0;
   cb.shades.lut=Arc::new(palette.chunks_exact(4).map(|p|p.try_into().unwrap()).collect());
   let a=cb.atmosphere.as_mut().unwrap(); a.region=cb.rect;
   a.points_per_cent=540.0/(span*100.0); a.points_per_ms=960.0/20000.0;
   a.settings=captured_settings(if name.starts_with("full") {StarHaloProfile::Uniform} else {StarHaloProfile::P3},1.0);
   next_timing::apply_profile(&mut a.settings);
   a.settings.star_jitter=std::env::var("PROBE_JITTER").unwrap_or("0.5".into()).parse().unwrap();
   let mut resources=CallbackResources::default();
   for frame in 0..60+frames {
    cb.pass_nr=frame; cb.atmosphere.as_mut().unwrap().now=1.0+frame as f64/30.0;
    let bytes=capture_frame(&device,&queue,&mut resources,&cb,ppp);
    if frame>=60 { std::fs::write(format!("{out}/{name}-{input}-{}.rgba",frame-60),bytes).unwrap(); }
   }
  }
 }
 std::fs::write(format!("{out}/manifest.json"),format!("{{\"ppp\":{ppp},\"origin\":{offset},\"frames\":{frames},\"width\":{},\"height\":{}}}",( (960.0+offset)*ppp).ceil() as u32,((540.0+offset)*ppp).ceil() as u32)).unwrap();
 next_timing::activate("");
}

// Copied verbatim in behavior from the frozen research scratch_frame_at_ppp.
fn capture_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    resources: &mut CallbackResources,
    cb: &SpectrogramCallback,
    ppp: f32,
) -> Vec<u8> {
    let size = [(cb.rect.max.x * ppp).ceil() as u32, (cb.rect.max.y * ppp).ceil() as u32];
    let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
    let mut encoder = device.create_command_encoder(&Default::default());
    let bufs = cb.prepare(device, queue, &screen, &mut encoder, resources);
    queue.submit(bufs.into_iter().chain([encoder.finish()]));
    let texture = render_to_texture(device, queue, size, cb.target_format, wgpu::Color::BLACK, |pass| {
        cb.paint(
            egui::PaintCallbackInfo {
                viewport: cb.rect,
                clip_rect: cb.rect,
                pixels_per_point: ppp,
                screen_size_px: size,
            },
            pass,
            resources,
        );
    });
    readback(device, queue, &texture, size)
}
