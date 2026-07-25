//! The piano roll's notes, drawn as instanced quads through a wgpu paint
//! callback instead of through egui's tessellator.
//!
//! **Why this exists.** The roll was the frame's dominant cost, and not
//! where it looked. Broken down (the performance overlay's Frame breakdown)
//! tessellation was ~0.5 ms while the vertex UPLOAD was 4-5 ms: 20k vertices
//! idle, 100k+ with notes on screen, in only ~20 primitives. Batching was
//! fine; the volume was the problem. egui is immediate-mode and re-uploads
//! every vertex every frame, so a roll that merely SCROLLS was re-sending
//! six figures of geometry 144 times a second. A note was three stroked,
//! anti-aliased rounded rects — keyline, black outline, core — each a
//! couple of hundred vertices once its corners and AA ring were subdivided.
//!
//! **What this does instead.** One quad per note segment, with a box signed
//! distance field in the fragment shader ([`shaders/roll.wgsl`]). Fill, the
//! note's own outline and the white keyline are bands of that distance, so
//! they cost a compare rather than a second and third shape. Four vertices per
//! note against several hundred: the upload stops mattering rather than
//! getting cheaper.
//!
//! **Why the buffer is still rewritten every frame.** The obvious next step
//! is an append-and-evict ring — settled notes never change, so they could
//! be uploaded once. They are not, deliberately. At 48 bytes per note a busy
//! roll is tens of kilobytes a frame against the megabytes that were the
//! whole problem, so a ring would be optimizing three orders of magnitude
//! below the cost it was built for, and it would have to carry the far-edge
//! trap with it: a note crossing the window's oldest edge is TRUNCATED
//! (as is, at the other end, one whose tail the Gap setting is shaving)
//! there, rewriting its geometry every frame while it leaves (see
//! `panes/roll.rs`), so any cache has to retire chunks before they reach it.
//! Rebuilding per frame keeps the geometry a pure function of `now` — which
//! is also what keeps the offline render deterministic.

use std::collections::HashMap;

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::{create_vertex_buffer, wgpu};

const ROLL_SRC: &str = include_str!("shaders/roll.wgsl");

/// Entry points the roll shader must provide.
#[cfg(test)]
pub(crate) const ROLL_ENTRY_POINTS: &[&str] = &["vs_note", "fs_note_gamma", "fs_note_linear"];

/// One note segment: a box in the pane's (pitch, depth) plane, its colors,
/// and the keyline standing outside it.
///
/// Screen geometry, in egui POINTS, already resolved through the pane's
/// `Axes` — this crate never learns which way the pane is turned. Lengths
/// are along the pane's two axes rather than x/y for the same reason.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RollInstance {
    /// Center of the segment, in screen points.
    pub center: [f32; 2],
    /// Half extents of the note's own outline along (pitch, depth).
    ///
    /// The pitch half-extent is zero for a ribbon too thin to bound, which
    /// is drawn as a bare spine — the outline width alone then gives it its
    /// thickness, exactly as the hairline branch did with a line stroke.
    pub half_extent: [f32; 2],
    /// `(shear, outline width, interior fill, spare)`.
    ///
    /// Shear is the center line's pitch drift per point of depth: 0 for a
    /// held note, non-zero for a glide, which makes the box a parallelogram
    /// rather than needing a second shape. Fill is how solidly the inside of
    /// the outline is painted in the note's own color, 0 leaving it hollow.
    ///
    /// No corner radius: a note is a rectangle, always. Rounding was a setting
    /// and is gone — on the notes short enough for it to show at all, it only
    /// ever rounded a tapped key into a bead.
    pub shape: [f32; 4],
    /// `(keyline width, edge mode)`.
    ///
    /// The width is in points, and 0 when the keyline is turned off. The mode
    /// says which of the note's edges it rides — around (0), its long sides
    /// (1), or its ends (2); see `rim_mask` in the shader.
    pub rim: [f32; 2],
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub core: [u8; 4],
    pub glow: [u8; 4],
}

impl RollInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<RollInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32x2, // half_extent
            2 => Float32x4, // shape
            3 => Float32x2, // rim
            4 => Unorm8x4,  // core
            5 => Unorm8x4,  // glow
        ],
    };
}

/// Which way the pane's two axes run on screen: unit vectors for pitch (the
/// short side) and depth/time (the long side).
///
/// The pane rotates and flips, and rather than baking that into every
/// instance it rides in the uniform — one pair of vectors for the whole
/// roll, the same affine the egui path got from `Axes::at`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RollAxes {
    pub pitch_dir: [f32; 2],
    pub depth_dir: [f32; 2],
}

/// Draw `instances` into `rect`. `pane_id` must be unique per roll shown in
/// the same frame (each gets its own instance buffer; the pipeline is
/// shared).
pub fn roll_paint_callback(
    rect: egui::Rect,
    instances: Vec<RollInstance>,
    axes: RollAxes,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        RollCallback { instances, axes, target_format, pane_id },
    )
}

/// Per-frame, per-pane draw data, built on the UI thread.
struct RollCallback {
    instances: Vec<RollInstance>,
    axes: RollAxes,
    target_format: wgpu::TextureFormat,
    pane_id: u64,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RollUniforms {
    screen_points: [f32; 2],
    feather: f32,
    _pad: f32,
    pitch_dir: [f32; 2],
    depth_dir: [f32; 2],
}

/// GPU objects cached across frames in egui-wgpu's `CallbackResources`.
struct RollResources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    panes: HashMap<u64, RollPane>,
}

struct RollPane {
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    instance_buffer: wgpu::Buffer,
    capacity: usize,
    count: u32,
}

/// Starting size of a pane's instance buffer; it grows by
/// `next_power_of_two` when a frame overflows it. A roll holds a few
/// hundred notes at the spans this pane is used at.
const INITIAL_NOTE_CAPACITY: usize = 512;

impl RollResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("roll_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let pipeline = create_roll_pipeline(device, target_format, &layout);
        RollResources { pipeline, layout, target_format, panes: HashMap::new() }
    }

    fn pane(&mut self, device: &wgpu::Device, pane_id: u64) -> &mut RollPane {
        let layout = &self.layout;
        self.panes.entry(pane_id).or_insert_with(|| {
            let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("roll_uniforms"),
                size: std::mem::size_of::<RollUniforms>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("roll_bind_group"),
                layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            });
            RollPane {
                uniform_buffer,
                bind_group,
                instance_buffer: create_vertex_buffer::<RollInstance>(
                    device,
                    "roll_notes",
                    INITIAL_NOTE_CAPACITY,
                ),
                capacity: INITIAL_NOTE_CAPACITY,
                count: 0,
            }
        })
    }
}

/// The one pipeline: instanced quads, blended exactly the way egui blends
/// its own shapes so a note composites over the spectrogram identically to
/// the tessellated version it replaces.
fn create_roll_pipeline(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("roll_shader"),
        source: wgpu::ShaderSource::Wgsl(ROLL_SRC.into()),
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("roll_pipeline_layout"),
        bind_group_layouts: &[Some(layout)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("roll_notes"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_note"),
            compilation_options: Default::default(),
            buffers: &[RollInstance::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some(if target_format.is_srgb() {
                // Same fork egui makes, for the same reason: an sRGB-aware
                // target wants linear values and encodes them itself.
                "fs_note_linear"
            } else {
                "fs_note_gamma"
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                // egui's own blend state, verbatim (see egui-wgpu's
                // renderer): premultiplied color, and alpha accumulated so
                // the pass composites the same way over a transparent
                // framebuffer.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::OneMinusDstAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

impl CallbackTrait for RollCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let recreate = callback_resources
            .get::<RollResources>()
            .is_none_or(|r| r.target_format != self.target_format);
        if recreate {
            callback_resources.insert(RollResources::new(device, self.target_format));
        }
        let resources: &mut RollResources =
            callback_resources.get_mut().expect("inserted above when missing");

        let ppp = screen_descriptor.pixels_per_point.max(f32::EPSILON);
        let uniforms = RollUniforms {
            screen_points: [
                screen_descriptor.size_in_pixels[0] as f32 / ppp,
                screen_descriptor.size_in_pixels[1] as f32 / ppp,
            ],
            // One physical pixel, expressed in the points the geometry is
            // in. Derived rather than sampled from the fragment's
            // derivatives so coverage is a pure function of the uniforms,
            // which is what the offline render's byte-for-byte determinism
            // test rests on.
            feather: 1.0 / ppp,
            _pad: 0.0,
            pitch_dir: self.axes.pitch_dir,
            depth_dir: self.axes.depth_dir,
        };

        let pane = resources.pane(device, self.pane_id);
        if self.instances.len() > pane.capacity {
            pane.capacity = self.instances.len().next_power_of_two();
            pane.instance_buffer =
                create_vertex_buffer::<RollInstance>(device, "roll_notes", pane.capacity);
        }
        pane.count = self.instances.len() as u32;
        if !self.instances.is_empty() {
            queue.write_buffer(&pane.instance_buffer, 0, bytemuck::cast_slice(&self.instances));
        }
        queue.write_buffer(&pane.uniform_buffer, 0, bytemuck::bytes_of(&uniforms));

        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<RollResources>() else {
            return;
        };
        let Some(pane) = resources.panes.get(&self.pane_id) else {
            return;
        };
        if pane.count == 0 {
            return;
        }
        // Draw against the WHOLE surface rather than the viewport egui-wgpu
        // helpfully set to the callback rect: the geometry is in screen
        // points, so this shader's clip mapping is egui's own and there is
        // no second rounding of the pane rect into pixels to disagree with.
        // egui-wgpu resets the viewport after a callback, and the SCISSOR it
        // set from the clip rect is left alone — that is what keeps the roll
        // inside its pane.
        render_pass.set_viewport(
            0.0,
            0.0,
            info.screen_size_px[0] as f32,
            info.screen_size_px[1] as f32,
            0.0,
            1.0,
        );
        render_pass.set_pipeline(&resources.pipeline);
        render_pass.set_bind_group(0, &pane.bind_group, &[]);
        render_pass.set_vertex_buffer(0, pane.instance_buffer.slice(..));
        render_pass.draw(0..4, 0..pane.count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{headless_device, readback, render_to_texture};

    /// A 256x256 test surface at one point per pixel, so a distance in
    /// points is a distance in pixels and the band arithmetic below reads
    /// straight off the instance.
    const SIZE: [u32; 2] = [256, 256];
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    /// Pitch across (x), time down (y) — the pane's Upright layout.
    const UPRIGHT: RollAxes = RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, 1.0] };

    /// A background whose bytes are exact, so "nothing was painted here" is
    /// an equality rather than a tolerance.
    const BG: [u8; 4] = [64, 96, 128, 255];

    fn bg_color() -> wgpu::Color {
        wgpu::Color {
            r: f64::from(BG[0]) / 255.0,
            g: f64::from(BG[1]) / 255.0,
            b: f64::from(BG[2]) / 255.0,
            a: 1.0,
        }
    }

    /// Run the callback for real — `prepare` then `paint` — over `clear`,
    /// and read the frame back as RGBA8.
    fn draw(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        draw_turned(device, queue, instances, UPRIGHT, clear)
    }

    /// As [`draw`], with the pane turned whichever way `axes` says.
    fn draw_turned(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: Vec<RollInstance>,
        axes: RollAxes,
        clear: wgpu::Color,
    ) -> Vec<u8> {
        let cb = RollCallback {
            instances,
            axes,
            target_format: FORMAT,
            pane_id: 0,
        };
        let mut resources = CallbackResources::default();
        let screen = ScreenDescriptor { size_in_pixels: SIZE, pixels_per_point: 1.0 };
        let mut encoder = device.create_command_encoder(&Default::default());
        let bufs = cb.prepare(device, queue, &screen, &mut encoder, &mut resources);
        queue.submit(bufs.into_iter().chain([encoder.finish()]));

        let rect =
            egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(SIZE[0] as f32, SIZE[1] as f32));
        let texture = render_to_texture(device, queue, SIZE, FORMAT, clear, |pass| {
            cb.paint(
                egui::PaintCallbackInfo {
                    viewport: rect,
                    clip_rect: rect,
                    pixels_per_point: 1.0,
                    screen_size_px: SIZE,
                },
                pass,
                &resources,
            );
        });
        readback(device, queue, &texture, SIZE)
    }

    fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE[0] + x) * 4) as usize;
        [frame[i], frame[i + 1], frame[i + 2], frame[i + 3]]
    }

    /// 8-bit color comparison with room for the shader's arithmetic.
    fn near(got: [u8; 4], want: [u8; 4]) -> bool {
        got.iter().zip(want).all(|(&a, b)| a.abs_diff(b) <= 3)
    }

    /// A straight note centered in the frame: 24 points thick, 120 long, a
    /// 4-point outline with a 2-point white keyline standing outside it,
    /// hollow, and rimmed all the way around. A wide keyline so a sample lands
    /// well inside it.
    fn centered_note() -> RollInstance {
        RollInstance {
            center: [128.0, 128.0],
            half_extent: [12.0, 60.0],
            shape: [0.0, 4.0, 0.0, 0.0],
            rim: [2.0, EDGE_AROUND],
            core: [255, 0, 0, 255],
            glow: [255, 255, 255, 255],
        }
    }

    /// The edge modes, as `rim.y` carries them (see `rim_mask` in the shader).
    const EDGE_AROUND: f32 = 0.0;
    const EDGE_SIDES: f32 = 1.0;
    const EDGE_ENDS: f32 = 2.0;

    #[test]
    fn baked_roll_shader_validates() {
        let module = naga::front::wgsl::parse_str(ROLL_SRC)
            .map_err(|e| e.emit_to_string(ROLL_SRC))
            .expect("roll.wgsl must parse");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("roll.wgsl must validate");
        for required in ROLL_ENTRY_POINTS {
            assert!(
                module.entry_points.iter().any(|ep| ep.name == *required),
                "missing entry point `{required}`"
            );
        }
    }

    /// The vertex-layout <-> shader-input contract (attribute locations,
    /// formats, strides), which neither the naga check (shader only) nor the
    /// type system (Rust only) covers — a mismatch otherwise panics at first
    /// paint inside a host.
    #[test]
    fn the_pipeline_builds_against_a_headless_device() {
        let Some((device, _queue)) = headless_device() else {
            return;
        };
        let _resources = RollResources::new(&device, FORMAT);
    }

    /// At Fill 0 a note is a HOLLOW ribbon with its keyline standing outside
    /// it: reading outward from the middle — nothing (the spectrogram shows
    /// through), the note's own color, the white keyline, nothing.
    ///
    /// This is the flood invariant, and the reason the bands are read off a
    /// distance rather than drawn as two strokes of the same path: a centered
    /// stroke grows inward exactly as much as outward, and on a ribbon a few
    /// points thick the two long edges met in the middle and painted the
    /// interior white. A band at distance 2..4 cannot reach inside an outline
    /// that ends at 2.
    #[test]
    fn a_note_is_hollow_and_its_rim_stands_outside_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        // Distances outward from the outline's path (x = 140): the outline
        // spans +-2, the keyline 2..4.
        let at = |x: u32| pixel(&frame, x, 128);
        assert!(near(at(128), BG), "the note's interior is not hollow: {:?}", at(128));
        assert!(near(at(134), BG), "the interior floods up to the outline: {:?}", at(134));
        assert!(near(at(140), [255, 0, 0, 255]), "no note color on its outline: {:?}", at(140));
        assert!(
            near(at(143), [255, 255, 255, 255]),
            "no white keyline riding the outline's edge: {:?}",
            at(143),
        );
        assert!(near(at(146), BG), "the keyline reaches further than it should: {:?}", at(146));
    }

    /// Fill paints the note's INTERIOR in its own color, and only the
    /// interior: the keyline outside it is untouched, so a filled note is the
    /// hollow one with its middle painted in rather than a differently shaped
    /// note.
    ///
    /// Partway is a real wash, not a switch — the whole point of the setting
    /// is that a note can read as solid while a loud spectrogram cell still
    /// ghosts through it.
    #[test]
    fn fill_paints_the_interior_and_nothing_else() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let solid = RollInstance { shape: [0.0, 4.0, 1.0, 0.0], ..centered_note() };
        let frame = draw(&device, &queue, vec![solid], bg_color());
        let at = |x: u32| pixel(&frame, x, 128);
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert!(near(at(128), RED), "the note's middle is still hollow: {:?}", at(128));
        assert!(near(at(134), RED), "the fill stops short of the outline: {:?}", at(134));
        assert!(
            near(at(143), [255, 255, 255, 255]),
            "the fill ate the keyline: {:?}",
            at(143),
        );
        assert!(near(at(146), BG), "the fill reaches outside the note: {:?}", at(146));

        // Half fill over an opaque background is half the note's color and half
        // the picture behind it.
        let wash = RollInstance { shape: [0.0, 4.0, 0.5, 0.0], ..centered_note() };
        let frame = draw(&device, &queue, vec![wash], bg_color());
        let middle = pixel(&frame, 128, 128);
        let half = |a: u8, b: u8| ((u32::from(a) + u32::from(b)) / 2) as u8;
        let want = [half(255, BG[0]), half(0, BG[1]), half(0, BG[2]), 255];
        assert!(near(middle, want), "half fill read as {middle:?}, not {want:?}");
    }

    /// `Sides` keeps the rim on the note's long edges and cuts it at the ends;
    /// `Ends` is the mirror image. The note's OWN outline is untouched either
    /// way — it is the shape, not the rim.
    ///
    /// This is what stops repeats of one key painting their halos over each
    /// other: the rim stands outside the note, and along the time axis a
    /// note's outside is the next note.
    #[test]
    fn the_rim_can_be_held_to_one_pair_of_edges() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Reading out from the center: the outline spans 10..14 across pitch
        // (x) and 58..62 along time (y), the keyline the 2 beyond that.
        let look = |mode: f32| {
            let note = RollInstance { rim: [2.0, mode], ..centered_note() };
            let frame = draw(&device, &queue, vec![note], bg_color());
            // (its own outline, the keyline) on a side, then on an end.
            (
                [pixel(&frame, 140, 128), pixel(&frame, 143, 128)],
                [pixel(&frame, 128, 188), pixel(&frame, 128, 191)],
            )
        };
        const RED: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        let (side, end) = look(EDGE_AROUND);
        assert!(near(side[1], WHITE), "Around lost the keyline on a side: {:?}", side[1]);
        assert!(near(end[1], WHITE), "Around lost the keyline on an end: {:?}", end[1]);

        let (side, end) = look(EDGE_SIDES);
        assert!(near(side[0], RED), "Sides dropped the note's own outline: {:?}", side[0]);
        assert!(near(side[1], WHITE), "Sides lost the rail: {:?}", side[1]);
        assert!(near(end[0], RED), "Sides cut the note's own outline at its end: {:?}", end[0]);
        assert!(near(end[1], BG), "Sides still rimmed the end: {:?}", end[1]);

        let (side, end) = look(EDGE_ENDS);
        assert!(near(end[1], WHITE), "Ends lost the cap: {:?}", end[1]);
        assert!(near(side[0], RED), "Ends cut the note's own outline: {:?}", side[0]);
        assert!(near(side[1], BG), "Ends still rimmed the side: {:?}", side[1]);
    }

    /// A note is a rectangle: its corners are square, right out to them.
    ///
    /// Rounding used to be a setting, and on the notes short enough for it to
    /// show at all it only ever hurt — a tap is a few points long, the radius
    /// clamps to its own half-length, and the note comes out a bead. A run of
    /// them came out as a string of beads. Nothing rounds a note now, and this
    /// samples the corner a radius would have taken off.
    #[test]
    fn a_notes_corners_are_square() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 40 points across pitch, 6 along time — the shape of a tapped key on
        // a thick ribbon. Filled and unrimmed, so the sample reads the shape
        // alone.
        let tap = RollInstance {
            half_extent: [20.0, 3.0],
            shape: [0.0, 0.0, 1.0, 0.0],
            rim: [0.0, EDGE_AROUND],
            ..centered_note()
        };
        let frame = draw(&device, &queue, vec![tap], bg_color());
        // 19.5 points out along pitch and 2.5 along time: inside the square
        // note, and outside any rounding of it (a radius clamped to the note's
        // half-length would arc from 17 out, missing this by half a point).
        let corner = pixel(&frame, 147, 130);
        assert!(
            near(corner, [255, 0, 0, 255]),
            "the tap's corner is missing ({corner:?}) — something is rounding it off",
        );
    }

    /// A ribbon too thin to bound is drawn as a bare spine, and the keyline
    /// must still stand OUTSIDE it: the note's own color at the middle, the
    /// white band beyond. This is the same invariant at the thickness where it
    /// actually bit — a hairline has no interior to hollow out, so a rim that
    /// grew inward would simply paint over the note.
    #[test]
    fn the_rim_does_not_paint_over_a_hairline_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let note = RollInstance { half_extent: [0.0, 60.0], ..centered_note() };
        let frame = draw(&device, &queue, vec![note], bg_color());
        let at = |x: u32| pixel(&frame, x, 128);
        assert!(near(at(128), [255, 0, 0, 255]), "the rim covered the spine: {:?}", at(128));
        assert!(near(at(131), [255, 255, 255, 255]), "no keyline beside it: {:?}", at(131));
        assert!(near(at(134), BG), "the rim reaches further than it should: {:?}", at(134));
    }

    /// The pane's orientation lives entirely in the uniform: turning the axes
    /// turns the picture, and nothing in the instances or the shader names a
    /// screen side.
    ///
    /// Both layouts the pane actually uses — Upright (pitch across, time
    /// down) and Across (pitch climbing, time along), which is a rotation AND
    /// a flip. The same note drawn through each must come out as the same
    /// picture, turned: `across(x, y)` is `upright(255 - y, x)`.
    #[test]
    fn turning_the_axes_turns_the_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Across: pitch climbs the screen (low at the bottom), time runs
        // left to right.
        const ACROSS: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [1.0, 0.0] };
        let upright = draw(&device, &queue, vec![centered_note()], bg_color());
        let across =
            draw_turned(&device, &queue, vec![centered_note()], ACROSS, bg_color());

        // A window around the note, wide enough to hold its full length.
        let mut painted = 0;
        for y in 60..200u32 {
            for x in 60..200u32 {
                let (a, b) = (pixel(&across, x, y), pixel(&upright, 255 - y, x));
                assert!(near(a, b), "the turned pane differs at ({x}, {y}): {a:?} vs {b:?}");
                if !near(a, BG) {
                    painted += 1;
                }
            }
        }
        assert!(painted > 500, "the note barely drew ({painted} pixels); the comparison is thin");
    }

    /// A glide's rim keeps its thickness instead of thinning with the angle.
    ///
    /// The shear that turns the box into the parallelogram a bent note
    /// follows also stretches distances along the pitch axis, so the band has
    /// to be measured perpendicular to the edge it rides — that is the
    /// division by the shear's length. Without it a 45-degree glide's keyline
    /// comes out 1/sqrt(2) as thick as the same note held.
    ///
    /// Measured as total ink across one scanline, which for a slanted band is
    /// `sqrt(1 + slope^2)` times its true thickness: 2.83 points for the two
    /// 1-point flanks at 45 degrees, against 2.0 held. An unnormalized
    /// distance would read 2.0 for both.
    #[test]
    fn a_glides_rim_keeps_its_thickness_instead_of_thinning_with_the_angle() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Only the keyline paints, and in black: over a white background its
        // coverage is then exactly `1 - r/255` in every pixel it touched.
        let bare = RollInstance {
            shape: [0.0, 0.0, 0.0, 0.0],
            rim: [1.0, EDGE_AROUND],
            core: [0, 0, 0, 0],
            glow: [0, 0, 0, 255],
            ..centered_note()
        };
        let white = wgpu::Color::WHITE;
        let ink = |note: RollInstance| {
            let frame = draw(&device, &queue, vec![note], white);
            (0..SIZE[0])
                .map(|x| 1.0 - f32::from(pixel(&frame, x, 128)[0]) / 255.0)
                .sum::<f32>()
        };

        let held = ink(bare);
        assert!((held - 2.0).abs() < 0.2, "a held note's two 1-point flanks measured {held}");

        let glide = ink(RollInstance { shape: [1.0, 0.0, 0.0, 0.0], ..bare });
        let expected = 2.0 * f32::sqrt(2.0);
        assert!(
            (glide - expected).abs() < 0.3,
            "a 45-degree glide's rim measured {glide} across the scanline, not {expected} — \
             the band thins with the angle instead of keeping its thickness",
        );
    }
}
