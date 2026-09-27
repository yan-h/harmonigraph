//! Read the production history targets: image-only comparisons cannot tell a
//! carried star from a newly lit one or expose repeated subpixel diffusion.
use super::*;
use crate::gpu_harness::headless_device;
use crate::spectrogram::tests::{prepare_once, refracted_fixture};
use crate::spectrogram::{SpectrogramCallback, SpectrogramResources};
use egui_wgpu::CallbackResources;
use harmonigraph_scene::CloudStyle;

fn memory(resources: &CallbackResources) -> &Memory {
    resources
        .get::<SpectrogramResources>()
        .unwrap()
        .panes
        .get(0)
        .unwrap()
        .cloud
        .as_ref()
        .unwrap()
        .memory
        .as_ref()
        .unwrap()
}

fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, memory: &Memory) -> Vec<[f32; 4]> {
    let [width, height] = memory.extent;
    let stride = (width * 16).next_multiple_of(256);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("color_memory_readback"),
        size: u64::from(stride * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        memory.views[memory.index].texture().as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit([encoder.finish()]);
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = slice.get_mapped_range();
    mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| {
            row[..width as usize * 16].chunks_exact(16).map(|rgba| {
                std::array::from_fn(|c| {
                    f32::from_le_bytes(rgba[c * 4..c * 4 + 4].try_into().unwrap())
                })
            })
        })
        .collect()
}

fn fixture(style: CloudStyle) -> SpectrogramCallback {
    let mut cb = refracted_fixture();
    let a = cb.atmosphere.as_mut().unwrap();
    a.settings.cloud_style = style;
    a.settings.cloud_depth = 1.0;
    a.settings.color_pickup = 0.08;
    a.settings.color_release = 0.6;
    a.settings.cloud_speed = 20.0;
    a.settings.cloud_direction = 37.0;
    a.settings.star_lifetime = 0.5;
    // Explicit motion ensures every slice crosses a cell regardless of look defaults.
    a.settings.star_speed_min = 0.2;
    a.settings.star_speed_max = 1.0;
    a.settings.star_speed_curve = 1.0;
    // Start away from zero to exercise initialization at an export's crop.
    a.now = 100.0;
    cb
}

fn floor(cb: &SpectrogramCallback) -> [f32; 4] {
    std::array::from_fn(|c| {
        if c == 3 {
            0.0
        } else {
            let gamma = f32::from(cb.shades.lut[0][c]) / 255.0;
            if gamma < 0.04045 {
                gamma / 12.92
            } else {
                ((gamma + 0.055) / 1.055).powf(2.4)
            }
        }
    })
}

fn close(actual: [f32; 4], expected: [f32; 4]) {
    for c in 0..4 {
        assert!((actual[c] - expected[c]).abs() < 1e-5, "channel {c}: {actual:?} != {expected:?}");
    }
}

#[test]
fn material_color_memory_carries_exact_texels_in_both_orientations() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    for style in [CloudStyle::Mosaic, CloudStyle::Watercolor] {
        for vertical in [false, true] {
            let mut cb = fixture(style);
            cb.atmosphere.as_mut().unwrap().pitch_vertical = vertical;
            let mut resources = CallbackResources::default();
            prepare_once(&device, &queue, &mut resources, &cb);
            let prior = pixels(&device, &queue, memory(&resources));
            let origin = memory(&resources).frame.as_ref().unwrap().origin;
            cb.grid.fill(0);
            cb.atmosphere.as_mut().unwrap().now += 0.25;
            prepare_once(&device, &queue, &mut resources, &cb);
            let m = memory(&resources);
            let shift: [i32; 2] =
                std::array::from_fn(|a| m.frame.as_ref().unwrap().origin[a] - origin[a]);
            assert!(
                shift.iter().all(|v| *v != 0) && shift.iter().any(|v| v.abs() > 1),
                "fixture never crosses a material texel: {shift:?}"
            );
            let held = pixels(&device, &queue, m);
            let [width, height] = m.extent.map(|v| v as i32);
            let floor = floor(&cb);
            let decay = (-0.25f32 / 0.6).exp();
            let mut carried = 0;
            for y in 0..height {
                for x in 0..width {
                    let [px, py] = [x + shift[0], y + shift[1]];
                    if px >= 0 && px < width && py >= 0 && py < height {
                        let old = prior[(py * width + px) as usize];
                        close(
                            held[(y * width + x) as usize],
                            std::array::from_fn(|c| floor[c] + (old[c] - floor[c]) * decay),
                        );
                        carried += usize::from(old[3] > 0.1);
                    } else {
                        close(held[(y * width + x) as usize], floor);
                    }
                }
            }
            assert!(carried > 1000, "fixture failed to carry structured light: {carried}");
            prepare_once(&device, &queue, &mut resources, &cb);
            assert_eq!(
                pixels(&device, &queue, memory(&resources)),
                held,
                "same time advanced memory"
            );
        }
    }
}

fn stagger(cell: [i32; 2], salt: u32) -> f32 {
    let cell = cell.map(|n| (n & (STAR_HASH_PERIOD as i32 - 1)) as u32);
    let mut n = cell[0].wrapping_mul(0x9e3779b9)
        ^ cell[1].wrapping_mul(0x85ebca6b)
        ^ salt.wrapping_mul(0x27d4eb2d);
    n = (n ^ (n >> 16)).wrapping_mul(0x7feb352d);
    n = (n ^ (n >> 15)).wrapping_mul(0x846ca68b);
    n ^= n >> 16;
    ((n & 255) as f32 + 0.5) / 256.0
}

#[test]
fn star_color_memory_follows_cells_and_resets_each_new_life() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    for wrap in [false, true] {
        let mut cb = fixture(CloudStyle::Stars);
        if wrap {
            let a = cb.atmosphere.as_mut().unwrap();
            a.settings.cloud_direction = 0.0;
            let layout = star_layout(a.settings, cb.rect.width() / cb.rect.height());
            a.now = STAR_HASH_PERIOD * f64::from(layout.cells[STAR_SLICES - 1])
                / star_px_per_second()
                - 0.125;
        }
        let mut resources = CallbackResources::default();
        prepare_once(&device, &queue, &mut resources, &cb);
        let prior = pixels(&device, &queue, memory(&resources));
        let old_slices = memory(&resources).frame.as_ref().unwrap().slices;
        let old_life = memory(&resources).frame.as_ref().unwrap().life;
        cb.grid.fill(0);
        cb.atmosphere.as_mut().unwrap().now += 0.25;
        prepare_once(&device, &queue, &mut resources, &cb);
        let m = memory(&resources);
        let held = pixels(&device, &queue, m);
        let frame = m.frame.as_ref().unwrap();
        if wrap {
            assert!(
                old_slices[STAR_SLICES - 1].offset.0[0] > 65530.0
                    && frame.slices[STAR_SLICES - 1].offset.0[0] < 5.0,
                "fixture did not cross the star hash period"
            );
        }
        let floor = floor(&cb);
        let decay = (-0.25f32 / 0.6).exp();
        let (mut carried, mut new_lives) = (0, 0);
        for (k, s) in frame.slices.iter().enumerate() {
            assert_ne!(s.origin, old_slices[k].origin, "slice {k} did not cross a cell");
            for y in 0..s.grid.0[1] {
                for x in 0..s.grid.0[0] {
                    let cell = [s.origin.0[0] + x, s.origin.0[1] + y];
                    let previous = old_slices[k];
                    let local: [i32; 2] = std::array::from_fn(|a| {
                        ((cell[a] - previous.origin.0[a] + 32768) & 65535) - 32768
                    });
                    let hash = stagger(cell, 1002 + 3 * k as u32);
                    let same_life = (frame.life + hash).floor() as u32 & 4095
                        == (old_life + hash).floor() as u32 & 4095;
                    let actual = held[(s.base + y * s.grid.0[0] + x) as usize];
                    if same_life
                        && (0..previous.grid.0[0]).contains(&local[0])
                        && (0..previous.grid.0[1]).contains(&local[1])
                    {
                        let old = prior
                            [(previous.base + local[1] * previous.grid.0[0] + local[0]) as usize];
                        close(
                            actual,
                            std::array::from_fn(|c| floor[c] + (old[c] - floor[c]) * decay),
                        );
                        carried += usize::from(old[3] > 0.1);
                    } else {
                        close(actual, floor);
                        new_lives += usize::from(!same_life);
                    }
                }
            }
        }
        assert!(carried > 1000 && new_lives > 1000, "carry={carried}, new lives={new_lives}");
    }
}

/// A low-rate export and a high-rate display integrate the same elapsed time.
/// Also exercises pickup, paused redraws, interpretation edits, seeks and off.
#[test]
fn color_memory_uses_elapsed_time_and_resets_invalid_history() {
    let Some((device, queue)) = headless_device() else {
        return;
    };
    let mut cb = fixture(CloudStyle::Mosaic);
    cb.atmosphere.as_mut().unwrap().settings.cloud_speed = 0.0;
    cb.atmosphere.as_mut().unwrap().settings.color_pickup = 5.0;
    let mut answers = Vec::new();
    for fps in [1, 120] {
        let mut resources = CallbackResources::default();
        cb.atmosphere.as_mut().unwrap().now = 100.0;
        cb.grid.fill(0);
        prepare_once(&device, &queue, &mut resources, &cb);
        let first = pixels(&device, &queue, memory(&resources))[5000];
        cb.grid.fill(255);
        for i in 1..=fps {
            cb.atmosphere.as_mut().unwrap().now = 100.0 + f64::from(i) / f64::from(fps);
            prepare_once(&device, &queue, &mut resources, &cb);
        }
        let held = pixels(&device, &queue, memory(&resources))[5000];
        assert!(
            (held[3] - (1.0 - (-0.2f32).exp())).abs() < 1e-5,
            "pickup ignored elapsed time: {fps}fps {held:?}"
        );
        assert_ne!(held, first);
        answers.push(held);
        // A changed source at the identical clock, and an unrelated style dial,
        // must neither advance nor invalidate the active material's history.
        cb.grid.fill(0);
        cb.atmosphere.as_mut().unwrap().settings.star_fringe += 0.1;
        prepare_once(&device, &queue, &mut resources, &cb);
        assert_eq!(pixels(&device, &queue, memory(&resources))[5000], held);
        // Palette interpretation changes are immediate even on a paused frame.
        cb.shades.lut = std::sync::Arc::new(vec![[64, 128, 192, 255]; 256]);
        prepare_once(&device, &queue, &mut resources, &cb);
        close(pixels(&device, &queue, memory(&resources))[5000], floor(&cb));
        // Seed again, then a backwards seek must initialize from the dark input.
        cb.grid.fill(255);
        cb.atmosphere.as_mut().unwrap().now = 140.0;
        prepare_once(&device, &queue, &mut resources, &cb);
        assert_eq!(pixels(&device, &queue, memory(&resources))[5000][3], 1.0);
        cb.grid.fill(0);
        cb.atmosphere.as_mut().unwrap().now = 139.0;
        prepare_once(&device, &queue, &mut resources, &cb);
        close(pixels(&device, &queue, memory(&resources))[5000], floor(&cb));
        // A degenerate callback must end the old history before drawing resumes.
        cb.grid.fill(255);
        cb.atmosphere.as_mut().unwrap().now = 180.0;
        prepare_once(&device, &queue, &mut resources, &cb);
        assert_eq!(pixels(&device, &queue, memory(&resources))[5000][3], 1.0);
        let rows = cb.read.rows;
        cb.read.rows = 0;
        prepare_once(&device, &queue, &mut resources, &cb);
        cb.read.rows = rows;
        cb.grid.fill(0);
        cb.atmosphere.as_mut().unwrap().now = 180.1;
        prepare_once(&device, &queue, &mut resources, &cb);
        close(pixels(&device, &queue, memory(&resources))[5000], floor(&cb));
        // Both zero releases the buffers and uses the old rendering path.
        let s = &mut cb.atmosphere.as_mut().unwrap().settings;
        s.color_pickup = 0.0;
        s.color_release = 0.0;
        prepare_once(&device, &queue, &mut resources, &cb);
        assert!(resources
            .get::<SpectrogramResources>()
            .unwrap()
            .panes
            .get(0)
            .unwrap()
            .cloud
            .as_ref()
            .unwrap()
            .memory
            .is_none());
        cb = fixture(CloudStyle::Mosaic);
        cb.atmosphere.as_mut().unwrap().settings.cloud_speed = 0.0;
        cb.atmosphere.as_mut().unwrap().settings.color_pickup = 5.0;
    }
    close(answers[0], answers[1]);
}

thread_local! {
    // The same production shaders and logical grid, with the pre-bucketing
    // physical allocation, provide the image reference for resize coverage.
    pub(super) static EXACT_ALLOCATION: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[test]
fn bucketed_memory_preserves_images_across_resize_and_sampling_changes() {
    use crate::spectrogram::tests::frame_with;
    let Some((device, queue)) = headless_device() else { return };
    for style in [CloudStyle::Mosaic, CloudStyle::Watercolor] {
        let mut cb = fixture(style);
        let mut bucketed = CallbackResources::default();
        let mut exact = CallbackResources::default();
        let mut prior = None;
        let mut reused = 0;
        let mut replaced = 0;
        for (frame, (width, pixel_points)) in [
            (100.0, 0.5),
            (100.0, 0.5),
            (101.0, 0.5),
            (140.0, 0.5),
            (140.0, 0.5),
            (145.0, 0.5),
            (100.0, 0.5),
            (100.0, 1.1),
            (100.0, 1.1),
        ]
        .into_iter()
        .enumerate()
        {
            cb.rect = egui::Rect::from_min_size(egui::pos2(11.0, 7.0), egui::vec2(width, 90.0));
            let a = cb.atmosphere.as_mut().unwrap();
            a.region = cb.rect;
            a.now = 100.0 + frame as f64 / 60.0;
            cb.grid.fill(if frame % 2 == 0 { 255 } else { 0 });
            for resources in [&mut bucketed, &mut exact] {
                resources.insert(CloudSampling { pixel_points, ..Default::default() });
            }
            let actual = frame_with(&device, &queue, &mut bucketed, &cb);
            EXACT_ALLOCATION.set(true);
            let expected = frame_with(&device, &queue, &mut exact, &cb);
            EXACT_ALLOCATION.set(false);
            assert_eq!(actual, expected, "{style:?} frame {frame} changed displayed pixels");
            let held = memory(&bucketed);
            assert_eq!(
                pixels(&device, &queue, held),
                pixels(&device, &queue, memory(&exact)),
                "{style:?} frame {frame} changed history's logical texels"
            );
            assert!(held.extent.iter().zip(held.size).all(|(&n, size)| n <= size && size - n < 64));
            let texture = held.views[0].texture().clone();
            if let Some((old_size, old_extent, old_rect, old_texture)) = prior {
                if frame == 7 {
                    assert_eq!(old_size, held.size, "sampling fixture changed physical bucket");
                    assert_eq!(old_rect, cb.rect, "sampling fixture changed pane geometry");
                    assert_ne!(old_extent, held.extent, "sampling fixture kept its logical grid");
                }
                if old_size == held.size {
                    assert_eq!(texture, old_texture, "same bucket replaced its allocation");
                    reused += 1;
                } else {
                    assert_ne!(texture, old_texture, "bucket boundary did not replace allocation");
                    replaced += 1;
                }
            }
            prior = Some((held.size, held.extent, cb.rect, texture));
        }
        assert!(
            reused >= 4 && replaced >= 2,
            "fixture missed allocation transitions: {reused} reused, {replaced} replaced"
        );
    }
    assert_eq!(memory_allocation_size([16383, 16384], 16384), [16384; 2]);
    assert_eq!(memory_allocation_size([998, 1000], 1000), [1000; 2]);
}
