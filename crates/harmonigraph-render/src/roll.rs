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
//! distance field in the fragment shader ([`shaders/roll.wgsl`]). The note's
//! solid body, the white keyline beside it and the black shade beyond that are
//! bands of that distance, so a rim band costs a compare rather than a second
//! and third shape. Four vertices per note against several hundred: the upload
//! stops mattering rather than getting cheaper.
//!
//! **Why the buffer is still rewritten every frame.** The obvious next step
//! is an append-and-evict ring — settled notes never change, so they could
//! be uploaded once. They are not, deliberately. At 36 bytes per note a busy
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

/// One note segment: a solid box in the pane's (pitch, depth) plane, its
/// color, and the two rim bands standing outside its long edges.
///
/// Screen geometry, in egui POINTS, already resolved through the pane's
/// `Axes` — this crate never learns which way the pane is turned. Lengths
/// are along the pane's two axes rather than x/y for the same reason.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RollInstance {
    /// Center of the segment, in screen points.
    pub center: [f32; 2],
    /// Half extents of the note's solid body along (pitch, depth). This IS
    /// the note's painted extent: it is filled to its edge and nothing
    /// straddles the boundary.
    ///
    /// No corner radius: a note is a rectangle, always. Rounding was a setting
    /// and is gone — on the notes short enough for it to show at all, it only
    /// ever rounded a tapped key into a bead.
    pub half_extent: [f32; 2],
    /// The center line's pitch drift per point of depth: 0 for a held note,
    /// non-zero for a glide, which makes the box a parallelogram rather than
    /// needing a second shape.
    pub shear: f32,
    /// Width of EACH rim band in points, and 0 when the rim is turned off.
    /// One width for both: [`glow`](Self::glow) runs from the note's edge out
    /// to it, [`shade`](Self::shade) from there out to twice it.
    ///
    /// The rim rides the note's two LONG edges only, and never its ends — see
    /// `rail_mask` in the shader for why that is the shape rather than a
    /// choice.
    pub keyline: f32,
    /// Premultiplied sRGB bytes, straight out of [`egui::Color32`].
    pub core: [u8; 4],
    /// The keyline standing against the note's edge, white in this pane.
    pub glow: [u8; 4],
    /// The outer band, black in this pane: the dark backing that separates a
    /// note's keyline from whatever the spectrogram is doing behind it.
    pub shade: [u8; 4],
}

impl RollInstance {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<RollInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32x2, // half_extent
            2 => Float32,   // shear
            3 => Float32,   // keyline
            4 => Unorm8x4,  // core
            5 => Unorm8x4,  // glow
            6 => Unorm8x4,  // shade
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

    /// The four axis pairs the pane hands this shader, named as
    /// `SpectralOrientation` names them: for the side the now-line sits on, so
    /// `depth_dir` points away from it. Pitch reads low-to-high the conventional
    /// way in each pair, which is why only `depth_dir` differs within one.
    ///
    /// [`TOP`] is what [`draw`] uses, so a test that says nothing about
    /// orientation is drawn pitch across (x) and time down (y).
    const TOP: RollAxes = RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, 1.0] };
    const BOTTOM: RollAxes = RollAxes { pitch_dir: [1.0, 0.0], depth_dir: [0.0, -1.0] };
    /// Pitch climbs the screen (low at the bottom), time runs along it.
    const LEFT: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [1.0, 0.0] };
    const RIGHT: RollAxes = RollAxes { pitch_dir: [0.0, -1.0], depth_dir: [-1.0, 0.0] };

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
        draw_turned(device, queue, instances, TOP, clear)
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

    /// A straight note centered in the frame: 24 points thick, 120 long, with
    /// a 2-point white keyline standing against its long edges and a 2-point
    /// black shade standing outside that. Wide bands so a sample lands well
    /// inside each.
    fn centered_note() -> RollInstance {
        RollInstance {
            center: [128.0, 128.0],
            half_extent: [12.0, 60.0],
            shear: 0.0,
            keyline: 2.0,
            core: [255, 0, 0, 255],
            glow: [255, 255, 255, 255],
            shade: [0, 0, 0, 255],
        }
    }

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

    /// A note is a SOLID rectangle of its own color with both rim bands
    /// standing outside it: reading outward from the middle — the note's color
    /// right to its edge, the white keyline, the black shade, nothing.
    ///
    /// The rim standing outside is the flood invariant, and the reason the
    /// bands are read off a distance rather than drawn as a stroke of the
    /// note's path: a centered stroke grows inward exactly as much as outward,
    /// and on a ribbon a few points thick the two long edges met in the middle
    /// and painted the interior white. A band at distance 0..2 cannot reach
    /// inside a box whose interior is at negative distance.
    ///
    /// The ORDER is the other half of it. The dark band is the backing the
    /// keyline is read against, so it goes outside the light one; inside, it
    /// would be a dark line eating the note's own color at every ribbon width.
    #[test]
    fn a_note_is_solid_and_its_rim_bands_stand_outside_it() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        // Distances outward from the note's edge (x = 140): the keyline 0..2,
        // the shade 2..4.
        let at = |x: u32| pixel(&frame, x, 128);
        const RED: [u8; 4] = [255, 0, 0, 255];
        assert!(near(at(128), RED), "the note's middle is not painted: {:?}", at(128));
        assert!(near(at(138), RED), "the fill stops short of the note's edge: {:?}", at(138));
        assert!(
            near(at(141), [255, 255, 255, 255]),
            "no white keyline standing against the note's edge: {:?}",
            at(141),
        );
        assert!(
            near(at(143), [0, 0, 0, 255]),
            "no black shade standing outside the keyline: {:?}",
            at(143),
        );
        assert!(near(at(145), BG), "the rim reaches further than it should: {:?}", at(145));
    }

    /// BOTH rim bands ride the note's two long edges and are cut at its ends.
    /// The note's own body is untouched either way — it is the shape, not the
    /// rim.
    ///
    /// This is what stops repeats of one key painting their halos over each
    /// other: the rim stands outside the note, and along the time axis a
    /// note's outside is the next note. A shade capped at the ends would be
    /// worse than a keyline capped there — it would draw a dark seam across a
    /// run of one key that was played as a run.
    #[test]
    fn both_rim_bands_ride_the_long_edges_and_stop_at_the_ends() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Reading out from the center: the note spans +-12 across pitch (x)
        // and +-60 along time (y), the keyline the 2 beyond each and the shade
        // the 2 beyond that.
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        // (the note's body, the keyline, the shade) on a side, then on an end.
        let side = [pixel(&frame, 138, 128), pixel(&frame, 141, 128), pixel(&frame, 143, 128)];
        let end = [pixel(&frame, 128, 186), pixel(&frame, 128, 189), pixel(&frame, 128, 191)];
        const RED: [u8; 4] = [255, 0, 0, 255];
        const WHITE: [u8; 4] = [255, 255, 255, 255];
        const BLACK: [u8; 4] = [0, 0, 0, 255];
        assert!(near(side[0], RED), "the note's body went missing: {:?}", side[0]);
        assert!(near(side[1], WHITE), "the keyline rail is missing: {:?}", side[1]);
        assert!(near(side[2], BLACK), "the shade rail is missing: {:?}", side[2]);
        assert!(near(end[0], RED), "the note's body was cut at its end: {:?}", end[0]);
        assert!(near(end[1], BG), "the keyline wrapped the end: {:?}", end[1]);
        assert!(near(end[2], BG), "the shade wrapped the end: {:?}", end[2]);
    }

    /// A rail runs the FULL length of the note it edges — corner to corner,
    /// at its OUTER edge as much as its inner one.
    ///
    /// The keyline is a band of the box distance, so past the note's end it
    /// curves with the field and pulls away; where that band is cut is what
    /// decides how much rail survives. It used to sit at distance
    /// `half_outline .. half_outline + keyline`, and the corner rounding at
    /// the outer radius then took the rail's outer edge off half an outline
    /// early — the rails read as visibly shorter than the note between them.
    /// With the outline gone the band starts at the note's own edge, and both
    /// of its corners land exactly where the note's ink stops.
    #[test]
    fn a_rail_runs_the_whole_length_of_its_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let frame = draw(&device, &queue, vec![centered_note()], bg_color());
        // The note's box ends at half_extent 60, so row 187 is its last full
        // one along time and 190 is clear of it.
        let lit = |x: u32, y: u32| pixel(&frame, x, y)[1] > (BG[1] + 40);
        let dark = |x: u32, y: u32| pixel(&frame, x, y)[1] < (BG[1] - 40);
        // Both edges of the 2-point keyline band, which starts at the note's
        // own edge (x = 140): column 140 is its inner half, 141 its OUTER half —
        // the half the band's corner rounding used to take off early.
        for x in [140, 141] {
            assert!(lit(x, 128), "the rail is missing at the note's middle (x = {x})");
            assert!(lit(x, 187), "the rail stops short of the note's end (x = {x})");
            assert!(!lit(x, 190), "the rail runs past the note's end (x = {x})");
        }
        // And the shade band beyond it (x = 142, 143), which is cut by the same
        // mask and so runs exactly as far: a dark rail one band short of the
        // note's end would read as the note tapering.
        for x in [142, 143] {
            assert!(dark(x, 128), "the shade rail is missing at the note's middle (x = {x})");
            assert!(dark(x, 187), "the shade rail stops short of the note's end (x = {x})");
            assert!(!dark(x, 190), "the shade rail runs past the note's end (x = {x})");
        }
    }

    /// A note is a rectangle: its corners are square, right out to them.
    ///
    /// Nothing rounds a note, and this samples the corner a radius would
    /// take off. On the notes short enough for rounding to show at all it
    /// only hurts — a tap is a few points long, the radius clamps to its own
    /// half-length, and the note comes out a bead. A run of them comes out as
    /// a string of beads.
    #[test]
    fn a_notes_corners_are_square() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 40 points across pitch, 6 along time — the shape of a tapped key on
        // a thick ribbon. No keyline, so the sample reads the shape alone.
        let tap = RollInstance {
            half_extent: [20.0, 3.0],
            keyline: 0.0,
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

    /// The rim must still stand OUTSIDE a note floored to its minimum
    /// thickness: the note's own color at the middle, the white band beyond it,
    /// the black band beyond that. This is the same invariant at the width
    /// where it actually bit — a hairline is all edge, so a band that grew
    /// inward would simply paint over the note.
    #[test]
    fn the_rim_does_not_paint_over_a_hairline_note() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // 1.5 points across pitch: what `panes::spectral::roll` floors a ribbon too thin
        // to see to (MIN_RIBBON_PX). Narrower than a pixel is wide, so no
        // sample here is purely one thing — what matters is that the middle
        // still reads as the NOTE rather than as the keyline having flooded
        // it, which is [255, 255, 255].
        let note = RollInstance { half_extent: [0.75, 60.0], ..centered_note() };
        let frame = draw(&device, &queue, vec![note], bg_color());
        let at = |x: u32| pixel(&frame, x, 128);
        let middle = at(128);
        assert_eq!(middle[0], 255, "the note's own color is missing: {middle:?}");
        assert!(middle[1] < 128, "the keyline flooded the note's middle: {middle:?}");
        assert!(near(at(129), [255, 255, 255, 255]), "no keyline beside it: {:?}", at(129));
        assert!(near(at(131), [0, 0, 0, 255]), "no shade outside the keyline: {:?}", at(131));
        assert!(near(at(134), BG), "the rim reaches further than it should: {:?}", at(134));
    }

    /// A note two pixels along the depth axis holds its brightness as it
    /// scrolls sub-pixel; one pixel does not. That threshold is what
    /// `panes::spectral::roll::MIN_LENGTH_DEVICE_PX` floors a brief note's length to, and
    /// this is the measurement it is quoted from.
    ///
    /// The `band`/`inside` box filter is one pixel wide, so a shape's coverage
    /// profile is a trapezoid with one-pixel ramps: its flat top is
    /// `length - 1` pixels across, and every sub-pixel offset lands a sample on
    /// the top only once that top is a pixel wide. Under it, some offsets catch
    /// only the ramps and the note reads dimmer — which, on something scrolling,
    /// is a flicker at the scroll rate rather than a static difference.
    ///
    /// Total ink is not the thing to measure: the filter conserves it to a
    /// fraction of a percent well past the point where the peak has collapsed,
    /// so a test on ink alone passes while the artifact is at its worst.
    #[test]
    fn a_two_pixel_note_holds_its_brightness_as_it_scrolls() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // White, no rim: every painted byte is the fill's own coverage.
        let bare = RollInstance {
            keyline: 0.0,
            core: [255, 255, 255, 255],
            glow: [0, 0, 0, 0],
            shade: [0, 0, 0, 0],
            ..centered_note()
        };
        // The brightest pixel anywhere, over a sweep of sub-pixel scroll
        // offsets along depth — which is y under [`TOP`]. One point is one
        // pixel on this surface.
        let peak_spread = |half_depth: f32| {
            let peaks: Vec<u8> = (0..8)
                .map(|step| {
                    let note = RollInstance {
                        center: [128.0, 128.0 + step as f32 / 8.0],
                        half_extent: [bare.half_extent[0], half_depth],
                        ..bare
                    };
                    let frame = draw(&device, &queue, vec![note], wgpu::Color::BLACK);
                    (0..SIZE[1])
                        .flat_map(|y| (0..SIZE[0]).map(move |x| (x, y)))
                        .map(|(x, y)| pixel(&frame, x, y)[0])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let (lo, hi) = (*peaks.iter().min().unwrap(), *peaks.iter().max().unwrap());
            f32::from(hi - lo) / f32::from(hi.max(1))
        };

        // Two pixels long: the floor. Nothing may move.
        let floored = peak_spread(1.0);
        assert!(
            floored < 0.02,
            "a two-pixel note pulsed by {:.0}% as it scrolled",
            floored * 100.0,
        );

        // One pixel: what an unfloored brief note gets, and what the floor is
        // for. Asserted so the check above cannot pass by measuring nothing.
        let hairline = peak_spread(0.5);
        assert!(
            hairline > 0.3,
            "a one-pixel note only pulsed by {:.0}%, so the floor above is guarding nothing",
            hairline * 100.0,
        );
    }

    /// The pane's orientation lives entirely in the uniform: turning the axes
    /// turns the picture, and nothing in the instances or the shader names a
    /// screen side.
    ///
    /// [`TOP`] against [`LEFT`], which is a rotation AND a flip. The same note
    /// drawn through each must come out as the same picture, turned:
    /// `left(x, y)` is `top(255 - y, x)`.
    #[test]
    fn turning_the_axes_turns_the_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let top = draw(&device, &queue, vec![centered_note()], bg_color());
        let left = draw_turned(&device, &queue, vec![centered_note()], LEFT, bg_color());

        // A window around the note, wide enough to hold its full length.
        let mut painted = 0;
        for y in 60..200u32 {
            for x in 60..200u32 {
                let (a, b) = (pixel(&left, x, y), pixel(&top, 255 - y, x));
                assert!(near(a, b), "the turned pane differs at ({x}, {y}): {a:?} vs {b:?}");
                if !near(a, BG) {
                    painted += 1;
                }
            }
        }
        assert!(painted > 500, "the note barely drew ({painted} pixels); the comparison is thin");
    }

    /// A REVERSED depth direction mirrors the picture and changes nothing else.
    ///
    /// The pane has four orientations, and the two whose now-line is on the
    /// right or the bottom hand this shader a negated `depth_dir` — `[-1, 0]`
    /// and `[0, -1]`, which the other two can never produce. Nothing here reads
    /// a screen side, so the claim is that a negated direction is just a mirror:
    /// a note drawn through `RIGHT` is the `LEFT` picture reflected in x, to the
    /// byte. A sign dropped anywhere between the uniform and `across` would
    /// show as the mirror failing, most likely by the keyline landing on the
    /// wrong flank.
    #[test]
    fn a_reversed_depth_direction_only_mirrors_the_picture() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // A note with a glide, so the picture is not its own mirror image and
        // the comparison can actually fail.
        let bent = RollInstance { shear: 0.6, ..centered_note() };
        let left = draw_turned(&device, &queue, vec![bent], LEFT, bg_color());
        let right = draw_turned(&device, &queue, vec![bent], RIGHT, bg_color());

        // `LEFT` and `RIGHT` share a centre column, so the reflection is about
        // x = 255 - x with the note's own centre fixed.
        let mut painted = 0;
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let (a, b) = (pixel(&right, x, y), pixel(&left, SIZE[0] - 1 - x, y));
                assert!(near(a, b), "the mirrored pane differs at ({x}, {y}): {a:?} vs {b:?}");
                if !near(a, BG) {
                    painted += 1;
                }
            }
        }
        assert!(painted > 500, "the note barely drew ({painted} pixels); the comparison is thin");

        // And the vertical pair, so a flip that only reached the x axis is
        // caught too.
        let top = draw_turned(&device, &queue, vec![bent], TOP, bg_color());
        let bottom = draw_turned(&device, &queue, vec![bent], BOTTOM, bg_color());
        for y in 0..SIZE[1] {
            for x in 0..SIZE[0] {
                let (a, b) = (pixel(&bottom, x, y), pixel(&top, x, SIZE[1] - 1 - y));
                assert!(near(a, b), "the mirrored upright pane differs at ({x}, {y})");
            }
        }
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
    /// `sqrt(1 + slope^2)` times its true thickness: 5.66 points for the two
    /// 2-point flanks at 45 degrees, against 4.0 held. An unnormalized
    /// distance would read 4.0 for both.
    ///
    /// Both bands are in the measurement, since both are cut from the same
    /// distance and the shade is the one that would have to stretch furthest.
    #[test]
    fn a_glides_rim_keeps_its_thickness_instead_of_thinning_with_the_angle() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        // Only the rim paints, and in black: over a white background its
        // coverage is then exactly `1 - r/255` in every pixel it touched.
        let bare = RollInstance {
            keyline: 1.0,
            core: [0, 0, 0, 0],
            glow: [0, 0, 0, 255],
            shade: [0, 0, 0, 255],
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
        assert!((held - 4.0).abs() < 0.3, "a held note's two 2-point flanks measured {held}");

        let glide = ink(RollInstance { shear: 1.0, ..bare });
        let expected = 4.0 * f32::sqrt(2.0);
        assert!(
            (glide - expected).abs() < 0.5,
            "a 45-degree glide's rim measured {glide} across the scanline, not {expected} — \
             the band thins with the angle instead of keeping its thickness",
        );
    }
}
