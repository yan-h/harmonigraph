//! The backdrop cut round the spiral's sounding-note dots: the roll's own
//! surround (`crate::surround`), read off each dot's circle.
//!
//! The sharp colored dots remain egui shapes. This callback is inserted just
//! before them: `prepare` draws the dots' cut and fill into targets over the
//! spiral's rect, and `paint` lays them over the picture, so the discs land on
//! a surround that has already taken the spectrum strip away beside them.
//!
//! One set of buffers, for the one copy: the Spiral is an editor tab that is
//! never exported (#974), so only its docked copy draws. Like the halo's chain
//! in [`crate::glow`], they stand once built, hidden tab included.

use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};

use crate::surround::{SurroundComposite, SurroundStages, SurroundTargets, SurroundUniform};
use crate::uniforms::{uniform_group, Float2};
use crate::{create_vertex_buffer, wgpu};

const SRC: &str = include_str!("shaders/dot_surround.wgsl");
const INITIAL_CAPACITY: usize = 16;

#[cfg(any(test, feature = "hot-reload"))]
pub(crate) const ENTRY_POINTS: &[&str] = &["vs_dot_cut", "fs_dot_surround"];

/// dot_surround.wgsl after the surround's arithmetic it calls into. It names
/// nothing in common.wgsl, so it takes none of it.
pub(crate) fn source() -> String {
    format!("{}\n{SRC}", crate::surround::SRC)
}

/// One sounding note's dot, as its surround needs it: where it stands, how big
/// it is, and its fill color — straight sRGB bytes, with the dot's own opacity
/// in alpha, which is how strongly it cuts.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SurroundDot {
    /// Centre, in screen points.
    pub center: [f32; 2],
    /// Radius, in points — the disc's painted extent, filled to its edge.
    pub radius: f32,
    pub aura: [u8; 4],
}

impl SurroundDot {
    const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<SurroundDot>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x2, // center
            1 => Float32,   // radius
            2 => Unorm8x4,  // aura
        ],
    };
}

/// Draw the backdrop cut round `dots`, which stand inside `rect`. The callback
/// belongs immediately before the colored egui discs.
///
/// Safe to add on every frame the pane draws, including the frames with no
/// dots and the frames whose cut draws nothing: the skipping is this
/// callback's to do, and a declining frame allocates nothing here.
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
    /// The spiral's rect in points, which the targets cover. `prepare` is
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
    surround: wgpu::RenderPipeline,
    composite: SurroundComposite,
    locals_layout: wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    #[cfg(feature = "hot-reload")]
    generation: u64,
    /// The one copy's buffers, built on its first cutting frame.
    pane: Option<Pane>,
}

struct Pane {
    locals: wgpu::Buffer,
    locals_bind: wgpu::BindGroup,
    dots: wgpu::Buffer,
    dot_capacity: usize,
    count: u32,
    /// The surround over the spiral's rect, and the size in device pixels it
    /// was built for. Dropped when the cut draws nothing.
    targets: Option<(SurroundTargets, [u32; 2])>,
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
        let locals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spiral_dot_surround_locals_layout"),
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
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spiral_dot_surround_shader"),
            source: wgpu::ShaderSource::Wgsl(source().into()),
        });
        Resources {
            surround: crate::surround::surround_pipeline(
                device,
                &shader,
                &[Some(&locals_layout)],
                SurroundStages {
                    label: "spiral_dot_surround",
                    vertex: "vs_dot_cut",
                    fragment: "fs_dot_surround",
                    buffers: &[SurroundDot::LAYOUT],
                },
            ),
            composite: SurroundComposite::new(device, format),
            locals_layout,
            format,
            #[cfg(feature = "hot-reload")]
            generation: crate::reload::generation(),
            pane: None,
        }
    }
}

impl Pane {
    /// The one copy's buffers, built on first use.
    fn get<'a>(
        pane: &'a mut Option<Pane>,
        device: &wgpu::Device,
        locals_layout: &wgpu::BindGroupLayout,
    ) -> &'a mut Pane {
        pane.get_or_insert_with(|| {
            let locals = crate::gpu_memory::buffer(
                device,
                &wgpu::BufferDescriptor {
                    label: Some("spiral_dot_surround_locals"),
                    size: std::mem::size_of::<DotLocals>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                },
            );
            let locals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("spiral_dot_surround_locals_bind"),
                layout: locals_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: locals.as_entire_binding(),
                }],
            });
            Pane {
                locals,
                locals_bind,
                dots: create_vertex_buffer::<SurroundDot>(
                    device,
                    "spiral_surround_dots",
                    INITIAL_CAPACITY,
                ),
                dot_capacity: INITIAL_CAPACITY,
                count: 0,
                targets: None,
            }
        })
    }
}

impl CallbackTrait for DotSurroundCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let ppp = screen.pixels_per_point.max(f32::EPSILON);
        // Never exported, so drawn at the editor's own scale.
        let surround = self.cut.uniform(1.0);
        let viewport =
            egui::epaint::ViewportInPixels::from_points(&self.rect, ppp, screen.size_in_pixels);
        let size = [viewport.width_px.max(0) as u32, viewport.height_px.max(0) as u32];
        let wants = surround.draws() && !self.dots.is_empty() && size.iter().all(|&d| d > 0);

        // Nothing to cut and nothing built: the pipelines are not built either,
        // so a reader whose cut is off never pays for them.
        if !wants && callback_resources.get::<Resources>().is_none() {
            return Vec::new();
        }
        let stale = callback_resources
            .get::<Resources>()
            .is_none_or(|resources| resources.is_stale(self.target_format));
        if stale {
            callback_resources.insert(Resources::new(device, self.target_format));
        }
        let Resources { surround: pipeline, composite, locals_layout, pane, .. } =
            callback_resources.get_mut::<Resources>().expect("inserted above");
        let pane = Pane::get(pane, device, locals_layout);

        // Held through a quiet stretch rather than dropped, since a dot comes
        // and goes with every note: what a frame with none skips is the work,
        // not the buffers. The count goes to 0 so `paint` declines; the targets
        // go with a cut that draws nothing, or a rect of another size.
        if !wants {
            pane.count = 0;
            if !surround.draws() {
                pane.targets = None;
            }
            return Vec::new();
        }
        if self.dots.len() > pane.dot_capacity {
            pane.dot_capacity = self.dots.len().next_power_of_two();
            pane.dots = create_vertex_buffer::<SurroundDot>(
                device,
                "spiral_surround_dots",
                pane.dot_capacity,
            );
        }
        pane.count = self.dots.len() as u32;
        queue.write_buffer(&pane.dots, 0, bytemuck::cast_slice(&self.dots));
        let locals = DotLocals {
            surround,
            origin_points: Float2([viewport.left_px as f32 / ppp, viewport.top_px as f32 / ppp]),
            viewport_points: Float2([size[0] as f32 / ppp, size[1] as f32 / ppp]),
            feather: 1.0 / ppp,
            _feather_pad: 0.0,
            _tail_pad: Float2([0.0; 2]),
        };
        queue.write_buffer(&pane.locals, 0, bytemuck::bytes_of(&locals));
        if pane.targets.as_ref().is_some_and(|(_, built)| *built != size) {
            pane.targets = None;
        }
        let (targets, _) = pane
            .targets
            .get_or_insert_with(|| (SurroundTargets::new(device, composite, size), size));
        let (locals_bind, dots, count) = (&pane.locals_bind, &pane.dots, pane.count);
        targets.draw(queue, encoder, pipeline, surround, |pass| {
            pass.set_bind_group(0, locals_bind, &[]);
            pass.set_vertex_buffer(0, dots.slice(..));
            pass.draw(0..4, 0..count);
        });
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
        let Some(pane) = &resources.pane else {
            return;
        };
        let Some((targets, _)) = pane.targets.as_ref().filter(|_| pane.count > 0) else {
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
        targets.paint(pass, &resources.composite);
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
    /// whole thing: no pipelines are built, and so no targets.
    ///
    /// Asked of the RESOURCES rather than of the frame, because both draw the
    /// same bytes however much work went into them. Read off a FIRST prepare,
    /// since anything already built is deliberately held through a quiet
    /// stretch.
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
        let resources = prepared(&device, &queue, &callback(vec![dot()], CUT), &screen);
        assert!(
            resources
                .get::<Resources>()
                .and_then(|resources| resources.pane.as_ref())
                .is_some_and(|pane| pane.targets.is_some()),
            "a dot under a cut that draws built no targets",
        );
    }

    /// A dot's surround takes the picture beside it as the cut's fade says, in
    /// points at every display density, leaves its own disc and everything
    /// past the reach alone, and fills what it took with the dot's own color.
    ///
    /// Read once with a black fill, where what is left is the picture alone,
    /// and once with a red one, where the white the cut took comes back red.
    #[test]
    fn a_dots_cut_takes_the_picture_beside_it_and_fills_with_its_color() {
        let Some((device, queue)) = headless_device() else {
            return;
        };
        for ppp in [1.0f32, 2.0] {
            let side = (64.0 * ppp) as u32;
            let size = [side.div_ceil(64) * 64, side];
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(64.0, 64.0));
            let resources = std::cell::RefCell::new(CallbackResources::default());
            let draw = |aura| {
                let cb = DotSurroundCallback {
                    rect,
                    dots: vec![SurroundDot { aura, ..dot() }],
                    cut: CUT,
                    target_format: wgpu::TextureFormat::Rgba8Unorm,
                };
                let screen = ScreenDescriptor { size_in_pixels: size, pixels_per_point: ppp };
                let mut encoder = device.create_command_encoder(&Default::default());
                let buffers =
                    cb.prepare(&device, &queue, &screen, &mut encoder, &mut resources.borrow_mut());
                queue.submit(buffers.into_iter().chain([encoder.finish()]));
                let resources = resources.borrow();
                let texture = render_to_texture(
                    &device,
                    &queue,
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
                            &resources,
                        );
                    },
                );
                let frame = readback(&device, &queue, &texture, size);
                move |x: f32, y: f32| {
                    let (x, y) = ((x * ppp) as u32, (y * ppp) as u32);
                    let i = ((y * size[0] + x) * 4) as usize;
                    [frame[i], frame[i + 1], frame[i + 2]]
                }
            };
            // About two and a half points past the edge, the straight fade
            // leaves `d / reach` of the white, `d` measured to the centre of
            // the pixel sampled.
            let black = draw([0, 0, 0, 255]);
            let d = ((38.5 * ppp).floor() + 0.5) / ppp - 36.0;
            let left = 255.0 * d / CUT.reach;
            let beside = black(38.5, 32.0);
            assert!(
                beside.iter().all(|&c| (f32::from(c) - left).abs() <= 3.0),
                "at {ppp} ppp {d} points out read {beside:?}, not {left}",
            );
            assert_eq!(black(32.0, 32.0), [255; 3], "at {ppp} ppp the cut reached under its dot");
            assert_eq!(black(4.0, 4.0), [255; 3], "at {ppp} ppp the cut escaped its reach");
            let filled = draw([255, 0, 0, 255])(38.5, 32.0);
            assert!(filled[0] >= 250 && filled[1] < 100, "at {ppp} ppp the fill read {filled:?}");
        }
    }
}

#[cfg(all(test, target_os = "macos", feature = "shader-assets-tools"))]
pub(super) fn asset_catalog(device: &wgpu::Device, format: wgpu::TextureFormat) {
    drop(Resources::new(device, format));
}
