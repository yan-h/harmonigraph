//! The backdrop cut round the spiral's sounding-note dots: the roll's own
//! surround (`crate::surround`), read off each dot's circle.
//!
//! The sharp colored dots remain egui shapes. This callback is inserted just
//! before them: `prepare` uploads the dots, and `paint` draws each one's cut
//! straight over the picture, the strongest dot's landing wherever two meet
//! (dot_surround.wgsl), so the discs land on a surround that has already taken
//! the spectrum strip away beside them.
//!
//! Straight onto the picture, unlike the roll, which draws into targets over
//! its rect and composites them: those cost the whole rect a frame whatever
//! the dots cover (0.14 ms at 1600x1600 px on an M5 Pro), where the dots are
//! a dozen discs. Picking the strongest in the shader costs what the dots
//! cover instead, so long as each fragment asks only the dots whose quads
//! overlap its own: asking every dot cost 0.40 ms for 48 dots spread over a
//! spiral, where the candidates lists cost 0.02 (`timing` below).
//!
//! One set of buffers, for the one copy: the Spiral is an editor tab that is
//! never exported (#974), so only its docked copy draws. Like the halo's chain
//! in [`crate::glow`], they stand once built, hidden tab included.

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::surround::SurroundUniform;
use crate::uniforms::{uniform_group, Float2};
use crate::wgpu;

const SRC: &str = include_str!("shaders/dot_surround.wgsl");
const INITIAL_CAPACITY: usize = 16;

#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ENTRY_POINTS: &[&str] =
    &["vs_dot_cut", "fs_dot_surround_gamma", "fs_dot_surround_linear"];

/// dot_surround.wgsl after the surround's arithmetic it calls into. It names
/// nothing in common.wgsl, so it takes none of it.
pub(crate) fn source() -> String {
    format!("{}\n{SRC}", crate::surround::SRC)
}

/// One sounding note's dot, as its surround needs it: where it stands, how big
/// it is, and its fill color — straight sRGB bytes, with the dot's own opacity
/// in alpha, which is how strongly it cuts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurroundDot {
    /// Centre, in screen points.
    pub center: [f32; 2],
    /// Radius, in points — the disc's painted extent, filled to its edge.
    pub radius: f32,
    pub aura: [u8; 4],
}

/// `Dot` in dot_surround.wgsl: a [`SurroundDot`] and its run of the
/// candidates list, itself and the dots whose quads overlap its own.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuDot {
    center: [f32; 2],
    radius: f32,
    aura: [u8; 4],
    first: u32,
    candidates: u32,
}

/// Draw the backdrop cut round `dots`, which stand inside `rect`. The callback
/// belongs immediately before the colored egui discs.
///
/// A cut that draws nothing is this callback's to decline, and a declining
/// frame allocates nothing here. A frame with no dots need not add it at all:
/// what it built stands through a quiet stretch without being asked.
pub fn dot_surround_paint_callback(
    rect: egui::Rect,
    dots: Vec<SurroundDot>,
    cut: crate::SurroundCut,
    target_format: wgpu::TextureFormat,
) -> egui::PaintCallback {
    egui_wgpu::Callback::new_paint_callback(
        rect,
        DotSurroundCallback { rect, dots, cut, target_format },
    )
}

struct DotSurroundCallback {
    /// The spiral's rect in points, which the dots are placed in. `prepare` is
    /// handed no viewport, so it rides here.
    rect: egui::Rect,
    dots: Vec<SurroundDot>,
    cut: crate::SurroundCut,
    target_format: wgpu::TextureFormat,
}

uniform_group! {
/// `DotLocals` in dot_surround.wgsl.
struct DotLocals {
    surround: SurroundUniform,
    origin_points: Float2,
    viewport_points: Float2,
    feather: f32,
    _feather_pad: f32,
    _tail_pad: Float2,
}
}

struct Resources {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    #[cfg(feature = "hot-reload")]
    generation: u64,
    /// The one copy's buffers, built on its first cutting frame.
    pane: Option<Pane>,
}

struct Pane {
    buffers: Buffers,
    /// The dots this frame draws, empty while the cut or the dots are absent,
    /// which `paint` declines; and their candidates. Both kept for their
    /// allocations.
    dots: Vec<GpuDot>,
    candidates: Vec<u32>,
}

/// The buffers the shader reads, and the bind group over them, rebuilt
/// together when either list outgrows its buffer.
struct Buffers {
    locals: wgpu::Buffer,
    dots: wgpu::Buffer,
    candidates: wgpu::Buffer,
    capacity: [usize; 2],
    bind: wgpu::BindGroup,
}

impl Resources {
    fn is_stale(&self, format: wgpu::TextureFormat) -> bool {
        #[cfg(feature = "hot-reload")]
        if self.generation != crate::reload::generation() {
            return true;
        }
        self.format != format
    }

    fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let entry = |binding, ty| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spiral_dot_surround_layout"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, wgpu::BufferBindingType::Storage { read_only: true }),
                entry(2, wgpu::BufferBindingType::Storage { read_only: true }),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spiral_dot_surround_shader"),
            source: wgpu::ShaderSource::Wgsl(source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spiral_dot_surround"),
            bind_group_layouts: &[Some(&layout)],
            ..Default::default()
        });
        let pipeline = crate::surround::landing_pipeline(
            device,
            "spiral_dot_surround",
            &shader,
            &pipeline_layout,
            "vs_dot_cut",
            "fs_dot_surround",
            format,
        );
        Resources {
            pipeline,
            layout,
            format,
            #[cfg(feature = "hot-reload")]
            generation: crate::reload::generation(),
            pane: None,
        }
    }
}

impl Buffers {
    /// Room for `capacity` dots and candidate entries.
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, capacity: [usize; 2]) -> Self {
        let buffer = |label, size, usage| {
            crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some(label),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            )
        };
        let locals = buffer(
            "spiral_dot_surround_locals",
            std::mem::size_of::<DotLocals>() as u64,
            wgpu::BufferUsages::UNIFORM,
        );
        let dots = buffer(
            "spiral_surround_dots",
            (capacity[0] * std::mem::size_of::<GpuDot>()) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let candidates = buffer(
            "spiral_surround_candidates",
            (capacity[1] * std::mem::size_of::<u32>()) as u64,
            wgpu::BufferUsages::STORAGE,
        );
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spiral_dot_surround_bind"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: locals.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: dots.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: candidates.as_entire_binding() },
            ],
        });
        Buffers { locals, dots, candidates, capacity, bind }
    }
}

/// Each of `dots` with its run of `candidates`: itself and every other dot
/// whose quad, its disc grown by `grow` points, overlaps its own, in index
/// order. Anything else cuts nothing in that quad, so the shader need not ask
/// it, and a crowd of dots costs a pixel only the dots near it.
fn list_candidates(
    dots: &[SurroundDot],
    grow: f32,
    out: &mut Vec<GpuDot>,
    candidates: &mut Vec<u32>,
) {
    out.clear();
    candidates.clear();
    for (i, a) in dots.iter().enumerate() {
        let first = candidates.len() as u32;
        candidates.extend(dots.iter().enumerate().filter_map(|(j, b)| {
            let apart = a.radius + b.radius + 2.0 * grow;
            let overlaps = (a.center[0] - b.center[0]).abs() <= apart
                && (a.center[1] - b.center[1]).abs() <= apart;
            // Its own entry too, which a non-finite centre would fail.
            (overlaps || i == j).then_some(j as u32)
        }));
        out.push(GpuDot {
            center: a.center,
            radius: a.radius,
            aura: a.aura,
            first,
            candidates: candidates.len() as u32 - first,
        });
    }
}

impl CallbackTrait for DotSurroundCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let ppp = screen.pixels_per_point.max(f32::EPSILON);
        // Never exported, so drawn at the editor's own scale.
        let surround = self.cut.uniform(1.0);
        let viewport =
            egui::epaint::ViewportInPixels::from_points(&self.rect, ppp, screen.size_in_pixels);
        let size = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
        let wants = surround.draws() && !self.dots.is_empty() && size.iter().all(|&d| d > 0);

        // Nothing to cut builds nothing, so a reader whose cut is off never
        // pays for the pipeline. What is built is held through a quiet stretch
        // rather than dropped, since a dot comes and goes with every note: a
        // frame with none skips the work, not the buffers, and with no dots
        // `paint` declines.
        if !wants {
            if let Some(pane) =
                callback_resources.get_mut::<Resources>().and_then(|r| r.pane.as_mut())
            {
                pane.dots.clear();
            }
            return Vec::new();
        }
        let stale = callback_resources
            .get::<Resources>()
            .is_none_or(|resources| resources.is_stale(self.target_format));
        if stale {
            callback_resources.insert(Resources::new(device, self.target_format));
        }
        let Resources { layout, pane, .. } =
            callback_resources.get_mut::<Resources>().expect("inserted above");
        let pane = pane.get_or_insert_with(|| Pane {
            buffers: Buffers::new(device, layout, [INITIAL_CAPACITY; 2]),
            dots: Vec::new(),
            candidates: Vec::new(),
        });

        // The quad's own growth, past the disc: the reach and a feather.
        let grow = surround.cut.0[1] + 1.0 / ppp;
        list_candidates(&self.dots, grow, &mut pane.dots, &mut pane.candidates);
        let needs = [pane.dots.len(), pane.candidates.len()];
        if needs.iter().zip(pane.buffers.capacity).any(|(&need, has)| need > has) {
            let capacity =
                [0, 1].map(|i| needs[i].next_power_of_two().max(pane.buffers.capacity[i]));
            pane.buffers = Buffers::new(device, layout, capacity);
        }
        queue.write_buffer(&pane.buffers.dots, 0, bytemuck::cast_slice(&pane.dots));
        queue.write_buffer(&pane.buffers.candidates, 0, bytemuck::cast_slice(&pane.candidates));
        let locals = DotLocals {
            surround,
            origin_points: Float2([viewport.left_px as f32 / ppp, viewport.top_px as f32 / ppp]),
            viewport_points: Float2([size[0] as f32 / ppp, size[1] as f32 / ppp]),
            feather: 1.0 / ppp,
            _feather_pad: 0.0,
            _tail_pad: Float2([0.0; 2]),
        };
        queue.write_buffer(&pane.buffers.locals, 0, bytemuck::bytes_of(&locals));
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<Resources>() else {
            return;
        };
        let Some(pane) = resources.pane.as_ref().filter(|pane| !pane.dots.is_empty()) else {
            return;
        };
        let vp = info.viewport_in_pixels();
        pass.set_viewport(
            vp.left_px as f32,
            vp.top_px as f32,
            vp.width_px as f32,
            vp.height_px as f32,
            0.0,
            1.0,
        );
        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &pane.buffers.bind, &[]);
        pass.draw(0..4, 0..pane.dots.len() as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_harness::{headless_device, readback, render_to_texture};

    #[test]
    fn baked_dot_surround_shader_validates() {
        crate::validate_wgsl("dot_surround.wgsl", &source(), 0, ENTRY_POINTS)
            .expect("baked dot_surround.wgsl must parse and validate");
    }

    #[test]
    fn uniforms_match_the_bound_shader_layout() {
        crate::uniforms::layout::check_binding::<DotLocals>(&source(), 0, 0);
    }

    /// The cut the tests below draw under: whole at the dot's edge, a straight
    /// fade over eight points, strongest-wins.
    const CUT: crate::SurroundCut = crate::SurroundCut { amount: 1.0, reach: 8.0, shape: 0.0 };

    fn dot() -> SurroundDot {
        SurroundDot { center: [32.0, 32.0], radius: 4.0, aura: [255, 0, 0, 255] }
    }

    fn prepared(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        cb: &DotSurroundCallback,
        screen: &ScreenDescriptor,
    ) -> CallbackResources {
        let mut resources = CallbackResources::default();
        let mut encoder = device.create_command_encoder(&Default::default());
        let buffers = cb.prepare(device, queue, screen, &mut encoder, &mut resources);
        queue.submit(buffers.into_iter().chain([encoder.finish()]));
        resources
    }

    /// A cut that draws nothing, and a frame with no dots in it, each skip the
    /// whole thing: no pipeline is built. Once built, a cut switched off draws
    /// nothing.
    ///
    /// Asked of the RESOURCES rather than of the frame, because both draw the
    /// same bytes however much work went into them. The skips are read off a
    /// FIRST prepare, since anything already built is deliberately held through
    /// a quiet stretch.
    #[test]
    fn nothing_to_cut_builds_no_pipelines() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let screen = ScreenDescriptor { size_in_pixels: [64, 64], pixels_per_point: 1.0 };
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
        let callback = |dots, cut| DotSurroundCallback {
            rect,
            dots,
            cut,
            target_format: wgpu::TextureFormat::Rgba8Unorm,
        };
        for (cb, what) in [
            (callback(vec![dot()], crate::SurroundCut { amount: 0.0, ..CUT }), "a cut of nothing"),
            (callback(Vec::new(), CUT), "an empty frame"),
        ] {
            let resources = prepared(&device, &queue, &cb, &screen);
            assert!(resources.get::<Resources>().is_none(), "{what} built the pipelines anyway");
        }
        // The control, and what makes the two above claims about the DECISION
        // rather than about a fixture too small to reach the build.
        let mut resources = prepared(&device, &queue, &callback(vec![dot()], CUT), &screen);
        let draws = |resources: &CallbackResources| {
            resources
                .get::<Resources>()
                .and_then(|resources| resources.pane.as_ref())
                .is_some_and(|pane| !pane.dots.is_empty())
        };
        assert!(draws(&resources), "a dot under a cut that draws drew nothing");
        let off = callback(vec![dot()], crate::SurroundCut { amount: 0.0, ..CUT });
        let mut encoder = device.create_command_encoder(&Default::default());
        off.prepare(&device, &queue, &screen, &mut encoder, &mut resources);
        assert!(!draws(&resources), "the cut switched off still draws");
    }

    /// `dots` cut under `cut` over a white 64-point square at `origin` on the
    /// screen, at `ppp`, prepared into `resources` (held across calls, as a
    /// pane's are) and painted: the frame's RGB at a point of the screen.
    fn drawn(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &mut CallbackResources,
        ppp: f32,
        origin: egui::Pos2,
        dots: Vec<SurroundDot>,
        cut: crate::SurroundCut,
    ) -> impl Fn(f32, f32) -> [u8; 3] {
        let [w, h] = [origin.x + 64.0, origin.y + 64.0].map(|v| (v * ppp).ceil() as u32);
        let size = [w.div_ceil(64) * 64, h];
        let rect = egui::Rect::from_min_size(origin, egui::vec2(64.0, 64.0));
        let cb =
            DotSurroundCallback { rect, dots, cut, target_format: wgpu::TextureFormat::Rgba8Unorm };
        let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
        let mut encoder = device.create_command_encoder(&Default::default());
        let buffers = cb.prepare(device, queue, &screen, &mut encoder, resources);
        queue.submit(buffers.into_iter().chain([encoder.finish()]));
        let texture = render_to_texture(
            device,
            queue,
            size,
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::Color::WHITE,
            |pass| {
                cb.paint(
                    egui::PaintCallbackInfo {
                        viewport: rect,
                        clip_rect: rect,
                        pixels_per_point: ppp,
                        screen_size_px: size,
                    },
                    pass,
                    resources,
                );
            },
        );
        let frame = readback(device, queue, &texture, size);
        move |x: f32, y: f32| {
            let (x, y) = ((x * ppp) as u32, (y * ppp) as u32);
            let i = ((y * size[0] + x) * 4) as usize;
            [frame[i], frame[i + 1], frame[i + 2]]
        }
    }

    /// A dot's surround takes the picture beside it as the cut's fade says, in
    /// points at every display density, leaves its own disc and everything
    /// past the reach alone, and fills what it took with the dot's own color.
    ///
    /// Read with a black fill, where what is left is the picture alone — at the
    /// whole amount and at half of it — and with a red one, where the white the
    /// cut took comes back red.
    #[test]
    fn a_dots_cut_takes_the_picture_beside_it_and_fills_with_its_color() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut resources = CallbackResources::default();
        for ppp in [1.0f32, 2.0] {
            let mut draw = |aura, cut| {
                drawn(
                    &device,
                    &queue,
                    &mut resources,
                    ppp,
                    egui::Pos2::ZERO,
                    vec![SurroundDot { aura, ..dot() }],
                    cut,
                )
            };
            // About two and a half points past the edge, the straight fade
            // leaves `d / reach` of the white, `d` measured to the centre of
            // the pixel sampled.
            let black = draw([0, 0, 0, 255], CUT);
            let d = ((38.5 * ppp).floor() + 0.5) / ppp - 36.0;
            let left = 255.0 * d / CUT.reach;
            let beside = black(38.5, 32.0);
            assert!(
                beside.iter().all(|&c| (f32::from(c) - left).abs() <= 3.0),
                "at {ppp} ppp {d} points out read {beside:?}, not {left}",
            );
            // At half the amount, half as much is taken.
            let half = draw([0, 0, 0, 255], crate::SurroundCut { amount: 0.5, ..CUT })(38.5, 32.0);
            let left_half = 255.0 * (1.0 - 0.5 * (1.0 - d / CUT.reach));
            assert!(
                half.iter().all(|&c| (f32::from(c) - left_half).abs() <= 3.0),
                "at {ppp} ppp half the cut read {half:?}, not {left_half}",
            );
            assert_eq!(black(32.0, 32.0), [255; 3], "at {ppp} ppp the cut reached under its dot");
            assert_eq!(black(4.0, 4.0), [255; 3], "at {ppp} ppp the cut escaped its reach");
            let filled = draw([255, 0, 0, 255], CUT)(38.5, 32.0);
            assert!(filled[0] >= 250 && filled[1] < 100, "at {ppp} ppp the fill read {filled:?}");
        }
    }

    /// A crowd larger than the buffers were first built for draws once they
    /// grow: twenty dots on a 12-point grid, after a frame of one, outgrow
    /// both the dots and the candidates buffers (sixteen entries each). Read
    /// between two dots near a corner, off the rect's middle in both
    /// directions, where the nearer cuts.
    #[test]
    fn a_crowd_outgrowing_the_buffers_still_cuts() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut resources = CallbackResources::default();
        drop(drawn(&device, &queue, &mut resources, 1.0, egui::Pos2::ZERO, vec![dot()], CUT));
        let crowd: Vec<SurroundDot> = (0..20)
            .map(|i| SurroundDot {
                center: [8.0 + 12.0 * (i % 5) as f32, 8.0 + 12.0 * (i / 5) as f32],
                radius: 2.0,
                aura: [0, 0, 0, 255],
            })
            .collect();
        let frame = drawn(&device, &queue, &mut resources, 1.0, egui::Pos2::ZERO, crowd, CUT);
        let capacity = resources
            .get::<Resources>()
            .and_then(|resources| resources.pane.as_ref())
            .map(|pane| pane.buffers.capacity);
        assert!(
            capacity.is_some_and(|[dots, candidates]| dots >= 20 && candidates > INITIAL_CAPACITY),
            "the crowd never outgrew the buffers: {capacity:?}",
        );
        // 2.5 points past the edge of the dot at (8, 20), the nearest by far.
        let d = 12.5 - 10.0;
        let left = 255.0 * d / CUT.reach;
        let read = frame(12.5, 20.5);
        assert!(
            read.iter().all(|&c| (f32::from(c) - left).abs() <= 6.0),
            "between two dots of the crowd read {read:?}, not about {left}",
        );
    }

    /// Where two dots' cuts meet, the stronger lands alone, with its color:
    /// neither laid over the other, nor a hole between them.
    ///
    /// A full red dot and a half-strength blue one, 16 points apart under an
    /// 8-point reach, so their cuts overlap between them. Read at two points
    /// there: one nearer the red, where red's cut is stronger, and one near
    /// enough the blue that its weaker opacity still wins on distance. Drawn
    /// with the pane at the screen's corner and again off it, since a fragment
    /// reads its pixel in screen points and the dots stand in them.
    #[test]
    fn where_two_dots_cuts_meet_the_stronger_lands_alone() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        let mut resources = CallbackResources::default();
        for (ppp, origin) in [(1.0f32, egui::pos2(0.0, 0.0)), (2.0, egui::pos2(37.0, 21.0))] {
            let [ox, oy] = [origin.x, origin.y];
            let dot = |x: f32, aura| SurroundDot { center: [ox + x, oy + 32.0], radius: 4.0, aura };
            let (red, blue) = (dot(24.0, [255, 0, 0, 255]), dot(40.0, [0, 0, 255, 128]));
            let frame = drawn(&device, &queue, &mut resources, ppp, origin, vec![red, blue], CUT);
            let frame = |x: f32, y: f32| frame(ox + x, oy + y);
            let fade = |d: f32| 1.0 - d / CUT.reach;
            let at = |x: f32| ((x * ppp).floor() + 0.5) / ppp;
            // White under a cut of `cut` filled with `rgb`.
            let over = |rgb: [f32; 3], cut: f32| rgb.map(|c| 255.0 * (1.0 - cut) + c * cut);
            for (x, expected) in [
                (31.0, over([255.0, 0.0, 0.0], fade(at(31.0) - 28.0))),
                (34.5, over([0.0, 0.0, 255.0], 128.0 / 255.0 * fade(36.0 - at(34.5)))),
            ] {
                let read = frame(x, 32.0);
                assert!(
                    read.iter().zip(expected).all(|(&c, e)| (f32::from(c) - e).abs() <= 3.0),
                    "at {ppp} ppp, x {x} read {read:?}, not {expected:?}",
                );
            }
        }
    }
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    drop(Resources::new(device, format));
}

/// A GPU-timer probe: what a frame of the Spiral's dot surround costs, over a
/// control with no dots. `#[ignore]`d — it prints figures and asserts nothing.
///
/// ```text
/// cargo test --release -p harmonigraph-render a_frame_of_spiral_dots \
///   -- --ignored --nocapture --test-threads=1
/// ```
/// Three workloads at the default cut, and again at the longest reach: a
/// six-note chord spread over the turns; a chromatic run over the two
/// innermost turns, where every dot's quad overlaps its neighbours'; and 48
/// notes over four turns of a spiral zoomed far out, the ceiling.
///
/// The figure scales with how many dots' reaches cover a pixel, squared, and
/// not with the pane. On an M5 Pro, at the default reach: the chord 0.01 ms,
/// the run 0.05 and the ceiling 0.43, where the targets this replaced cost
/// 0.14 at any crowding. At the longest reach the run costs 0.28 and the
/// ceiling 2.54. That ceiling was accepted (#1545) as the price of the
/// ordinary frame's saving, rather than bounded by falling back to targets.
#[cfg(test)]
mod timing {
    use super::*;

    #[test]
    #[ignore = "a probe: prints timings and asserts nothing"]
    fn a_frame_of_spiral_dots_costs_this_much() {
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let Some(timer) = crate::frame_timer::FrameTimer::new(format) else { return };
        for (size, reach) in [([1600u32, 1600], 15.0), ([2400, 1800], 15.0), ([1600, 1600], 30.0)] {
            let cut = crate::SurroundCut { amount: 1.0, reach, shape: 0.0 };
            let pane = timer.pane(size, 2.0);
            let (rect, c) = (pane.rect, pane.rect.center());
            let time = |dots: Vec<SurroundDot>| {
                let cb = DotSurroundCallback { rect, dots, cut, target_format: format };
                let mut resources = CallbackResources::default();
                let mut samples: Vec<f64> = timer
                    .time_frames(120, |_| {
                        timer.frame(
                            &pane,
                            &mut resources,
                            crate::frame_timer::Opening::AheadOfPrepare,
                            |encoder, resources| {
                                cb.prepare(
                                    &timer.device,
                                    &timer.queue,
                                    &pane.screen,
                                    encoder,
                                    resources,
                                )
                            },
                            |pass, resources| cb.paint(pane.paint_info(), pass, resources),
                        )
                    })
                    .into_iter()
                    .map(|sample| sample.gpu_ms)
                    .collect();
                samples.sort_by(f64::total_cmp);
                samples[samples.len() / 2]
            };
            // A dot `k` semitones round a spiral of `turn` points a turn,
            // starting `inner` points out.
            let on_spiral = |k: usize, inner: f32, turn: f32| {
                let a = std::f32::consts::TAU * k as f32 / 12.0;
                let r = inner + turn * k as f32 / 12.0;
                SurroundDot {
                    center: [c.x + r * a.cos(), c.y + r * a.sin()],
                    radius: 4.0,
                    aura: [200, 80, 40, 255],
                }
            };
            let control = time(Vec::new());
            eprintln!("spiral dots {size:?} at 2 ppp, reach {reach}: control {control:.3} ms");
            for (name, dots) in [
                (
                    "six-note chord",
                    [0, 4, 7, 16, 23, 31].map(|k| on_spiral(k, 60.0, 60.0)).to_vec(),
                ),
                ("chromatic run, 24", (0..24).map(|k| on_spiral(k, 40.0, 20.0)).collect()),
                (
                    "zoomed out, 48 over four turns",
                    (0..48).map(|k| on_spiral(k, 20.0, 10.0)).collect(),
                ),
            ] {
                eprintln!("  {name}: {:.3} ms over the control", time(dots) - control);
            }
        }
    }
}
