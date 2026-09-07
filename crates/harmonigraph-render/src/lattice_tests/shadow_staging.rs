//! Exercise the actual prepare sites against the parent's collected uploads.
use super::fixtures::*;
use crate::gpu_harness::{readback, render_to_texture};
use crate::*;

const SIZE: [u32; 2] = [256, 256];
const CELL_BYTES: usize = std::mem::size_of::<shadow::ShadowBox>();

fn callback(nodes: usize, labels: usize, atlas: bool, pane: u64) -> LatticeCallback {
    let mut scene = parity_scene();
    scene.glow_strength = 0.0;
    scene.pluses.clear();
    let seed = scene.nodes[0];
    scene.nodes = (0..nodes)
        .map(|i| {
            let mut node = seed;
            node.world_pos =
                glam::vec3((i % 13) as f32 * 0.1 - 0.6, (i % 17) as f32 * 0.1 - 0.8, 0.0);
            node
        })
        .collect();
    let mut named = names(
        (0..labels)
            .map(|i| {
                (
                    i as u32,
                    (0..3 + i % 5)
                        .map(|j| name_glyph(&scene, [100.0 + j as f32 * 3.0, 100.0, 2.0, 6.0]))
                        .collect(),
                )
            })
            .collect(),
    );
    if !atlas {
        named.atlas = None;
    }
    let cb = LatticeCallback::from_scene(
        &scene,
        named,
        egui::vec2(256.0, 256.0),
        wgpu::TextureFormat::Rgba8Unorm,
        pane,
        None,
    );
    assert_eq!(cb.instances.len(), nodes);
    assert_eq!(cb.glyphs.len(), (0..labels).map(|i| 3 + i % 5).sum::<usize>());
    cb
}

fn collected(cb: &LatticeCallback, has_atlas: bool, max_dim: u32) -> [Vec<shadow::ShadowBox>; 2] {
    let packed = shadow::pack(&cb.casters, cb.render_scale, max_dim);
    let nodes = cb
        .node_cells
        .iter()
        .map(|&i| packed.boxes.get(i as usize).copied().unwrap_or(shadow::NO_CELL))
        .collect();
    let mut glyphs = Vec::new();
    if has_atlas && !packed.boxes.is_empty() {
        for draw in &cb.draws {
            if let Draw::Label(a, b, l) = *draw {
                assert_eq!(a as usize, glyphs.len(), "exact contiguous label range");
                glyphs.extend(std::iter::repeat_n(packed.boxes[l as usize], (b - a) as usize));
            }
        }
        assert_eq!(glyphs.len(), cb.glyphs.len());
    }
    [nodes, glyphs]
}

fn read_cells(shooter: &Shooter, buffer: &wgpu::Buffer, bytes: usize) -> Vec<u8> {
    let out = shooter.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shadow_cell_readback"),
        size: bytes as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = shooter.device.create_command_encoder(&Default::default());
    encoder.copy_buffer_to_buffer(buffer, 0, &out, 0, bytes as u64);
    shooter.queue.submit([encoder.finish()]);
    out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    shooter.device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    out.slice(..).get_mapped_range().to_vec()
}

fn picture(shooter: &Shooter, cb: &LatticeCallback) -> Vec<u8> {
    let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(256.0, 256.0));
    let texture = render_to_texture(
        &shooter.device,
        &shooter.queue,
        SIZE,
        shooter.format,
        wgpu::Color::BLACK,
        |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &shooter.resources,
            );
        },
    );
    readback(&shooter.device, &shooter.queue, &texture, SIZE)
}

#[test]
fn prepare_shadow_uploads_preserve_exact_cells_and_picture_across_panes() {
    let Some(mut shooter) = Shooter::new(SIZE) else { return };
    let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
    // Missing font first: a later callback without a snapshot can still use
    // the renderer's cached font. Both panes then grow and shrink independently.
    for (pane, nodes, labels, atlas, no_boxes) in [
        (101, 273, 15, false, false),
        (202, 6, 2, true, false),
        (101, 273, 15, true, false),
        (202, 6, 0, true, true),
        (101, 0, 0, true, false),
        (202, 273, 15, true, false),
        (101, 6, 2, true, false),
        (202, 273, 0, true, false),
    ] {
        let mut cb = callback(nodes, labels, atlas, pane);
        if no_boxes {
            cb.casters.clear();
        }
        // Prime normal buffer growth, then enable readback only on these two
        // test buffers. Their production capacity metadata remains unchanged.
        let mut encoder = shooter.device.create_command_encoder(&Default::default());
        let commands = cb.prepare(
            &shooter.device,
            &shooter.queue,
            &screen,
            &mut encoder,
            &mut shooter.resources,
        );
        shooter.queue.submit(commands.into_iter().chain([encoder.finish()]));
        let resources = shooter.resources.get_mut::<LatticeResources>().unwrap();
        let has_atlas = !resources.atlas.is_empty();
        assert_eq!(has_atlas, atlas);
        let expected = collected(&cb, has_atlas, shooter.device.limits().max_texture_dimension_2d);
        let gpu_pane = resources.panes.get_mut(&pane).unwrap();
        assert_eq!(gpu_pane.instance_count as usize, nodes);
        assert_eq!(gpu_pane.glyph_count as usize, if atlas { cb.glyphs.len() } else { 0 });
        for (buffer, capacity) in [
            (&mut gpu_pane.node_cell_buffer, gpu_pane.node_cell_capacity),
            (&mut gpu_pane.cell_buffer, gpu_pane.glyph_capacity),
        ] {
            if !buffer.usage().contains(wgpu::BufferUsages::COPY_SRC) {
                *buffer = shooter.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("readable_shadow_upload"),
                    size: ((capacity + 1) * CELL_BYTES) as u64,
                    usage: wgpu::BufferUsages::VERTEX
                        | wgpu::BufferUsages::COPY_DST
                        | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });
            }
            shooter.queue.write_buffer(buffer, 0, &vec![0xA5; buffer.size() as usize]);
        }
        let buffers = [gpu_pane.node_cell_buffer.clone(), gpu_pane.cell_buffer.clone()];
        let mut images = Vec::new();
        for reference in [false, true] {
            let mut encoder = shooter.device.create_command_encoder(&Default::default());
            let commands = cb.prepare(
                &shooter.device,
                &shooter.queue,
                &screen,
                &mut encoder,
                &mut shooter.resources,
            );
            if reference {
                // The parent's collect/write_buffer bytes replace direct
                // staging before any of this frame's encoded draws execute.
                for (buffer, cells) in buffers.iter().zip(&expected) {
                    shooter.queue.write_buffer(buffer, 0, bytemuck::cast_slice(cells));
                }
            }
            shooter.queue.submit(commands.into_iter().chain([encoder.finish()]));
            for (buffer, cells) in buffers.iter().zip(&expected) {
                let bytes: &[u8] = bytemuck::cast_slice(cells);
                let got = read_cells(&shooter, buffer, bytes.len() + CELL_BYTES);
                assert_eq!(&got[..bytes.len()], bytes, "pane {pane}, reference {reference}");
                assert_eq!(&got[bytes.len()..], &[0xA5; CELL_BYTES], "untouched tail");
            }
            images.push(picture(&shooter, &cb));
        }
        assert_eq!(images[0], images[1], "pane {pane}, nodes {nodes}, labels {labels}");
        if nodes > 0 {
            assert!(
                images[0].chunks_exact(4).any(|pixel| pixel[..3] != images[0][..3]),
                "a drawable frame must contain more than a flat background",
            );
        }
    }
}
