//! An opt-in experiment, not a production shader path. See tools/metal-precompile/.
//! Export uses Naga without a GPU; measurement loads Apple's compiled output
//! through wgpu's public passthrough API. Only horizontal shadow blur changes.

use std::path::{Path, PathBuf};
use std::time::Instant;

use naga::back::msl;

use super::fixtures::{differing_pixels, one_shadow, parity_scene, Shooter};
use crate::*;

const SOURCE: &str = include_str!("../shaders/shadow.wgsl");
const STAGES: [(naga::ShaderStage, &str); 2] =
    [(naga::ShaderStage::Vertex, "vs_cell"), (naga::ShaderStage::Fragment, "fs_blur_x")];

fn directory() -> PathBuf {
    std::env::var_os("HARMONIGRAPH_METAL_PROBE_DIR")
        .map(PathBuf::from)
        .expect("set HARMONIGRAPH_METAL_PROBE_DIR to the exported/compiled library directory")
}

/// Match wgpu-hal 29's Metal translation for this one layout: texture 0,
/// sampler 0, vertex buffer 15, and vertex-length buffer 0. No storage arrays,
/// dynamic offsets, constants or workgroup storage occur in this shader.
/// The explicit mapping is a prototype cost, not a portable backend contract.
fn generated() -> [(String, String); 2] {
    let module = naga::front::wgsl::parse_str(SOURCE).unwrap();
    let info = naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::all(),
    )
    .validate(&module)
    .unwrap();
    STAGES.map(|(stage, entry)| {
        let (module, info) = naga::back::pipeline_constants::process_overrides(
            &module,
            &info,
            Some((stage, entry)),
            &Default::default(),
        )
        .unwrap();
        let resources = msl::EntryPointResources {
            resources: [
                (
                    naga::ResourceBinding { group: 0, binding: 0 },
                    msl::BindTarget { texture: Some(0), ..Default::default() },
                ),
                (
                    naga::ResourceBinding { group: 0, binding: 1 },
                    msl::BindTarget {
                        sampler: Some(msl::BindSamplerTarget::Resource(0)),
                        ..Default::default()
                    },
                ),
            ]
            .into(),
            sizes_buffer: (stage == naga::ShaderStage::Vertex).then_some(0),
            ..Default::default()
        };
        let options = msl::Options {
            lang_version: (3, 2),
            per_entry_point_map: [(entry.to_owned(), resources)].into(),
            fake_missing_bindings: false,
            bounds_check_policies: naga::proc::BoundsCheckPolicies {
                index: naga::proc::BoundsCheckPolicy::Restrict,
                buffer: naga::proc::BoundsCheckPolicy::Restrict,
                image_load: naga::proc::BoundsCheckPolicy::Restrict,
                binding_array: naga::proc::BoundsCheckPolicy::Unchecked,
            },
            ..Default::default()
        };
        let layout = shadow::ShadowBox::LAYOUT;
        let pipeline_options = msl::PipelineOptions {
            entry_point: Some((stage, entry.to_owned())),
            allow_and_force_point_size: false,
            vertex_pulling_transform: true,
            vertex_buffer_mappings: if stage == naga::ShaderStage::Vertex {
                vec![msl::VertexBufferMapping {
                    id: 15,
                    stride: layout.array_stride as u32,
                    step_mode: msl::VertexBufferStepMode::ByInstance,
                    attributes: layout
                        .attributes
                        .iter()
                        .map(|a| {
                            assert_eq!(a.format, wgpu::VertexFormat::Float32x4);
                            msl::AttributeMapping {
                                shader_location: a.shader_location,
                                offset: a.offset as u32,
                                format: msl::VertexFormat::Float32x4,
                            }
                        })
                        .collect(),
                }]
            } else {
                vec![]
            },
        };
        let (source, translation) =
            msl::write_string(&module, &info, &options, &pipeline_options).unwrap();
        (source, translation.entry_point_names[0].as_ref().unwrap().clone())
    })
}

#[test]
#[ignore = "exports MSL for tools/metal-precompile/compile.py; no GPU required"]
fn export_shadow_blur_metal() {
    let dir = directory();
    std::fs::create_dir_all(&dir).unwrap();
    for ((_, entry), (source, translated)) in STAGES.into_iter().zip(generated()) {
        std::fs::write(dir.join(format!("{entry}.metal")), source).unwrap();
        std::fs::write(dir.join(format!("{entry}.entry")), translated).unwrap();
    }
}

fn pipeline(
    device: &wgpu::Device,
    atlas: &wgpu::BindGroupLayout,
    vertex: (&wgpu::ShaderModule, &str),
    fragment: (&wgpu::ShaderModule, &str),
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("precompiled_shadow_probe"),
        bind_group_layouts: &[Some(atlas)],
        ..Default::default()
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fs_blur_x"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: vertex.0,
            entry_point: Some(vertex.1),
            compilation_options: Default::default(),
            buffers: &[shadow::ShadowBox::LAYOUT],
        },
        fragment: Some(wgpu::FragmentState {
            module: fragment.0,
            entry_point: Some(fragment.1),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: shadow::ATLAS_FORMAT,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: Default::default(),
        multiview_mask: None,
        cache: None,
    })
}

fn compiled(
    device: &wgpu::Device,
    atlas: &wgpu::BindGroupLayout,
    dir: &Path,
) -> wgpu::RenderPipeline {
    // Reject stale generated input before using the unsafe passthrough API.
    // compile.py records hashes tying the binaries to these exact sources.
    for ((_, entry), (source, name)) in STAGES.into_iter().zip(generated()) {
        assert_eq!(std::fs::read_to_string(dir.join(format!("{entry}.metal"))).unwrap(), source);
        assert_eq!(std::fs::read_to_string(dir.join(format!("{entry}.entry"))).unwrap(), name);
    }
    let started = Instant::now();
    let shaders = STAGES.map(|(_, entry)| {
        let bytes = std::fs::read(dir.join(format!("{entry}.metallib"))).unwrap();
        // SAFETY: the experiment loads only the Naga-generated, build-validated
        // shader pair above. The explicit layout and vertex mapping match the
        // pinned Metal backend, with its vertex bounds checks retained. There
        // are no runtime storage arrays requiring missing reflection metadata.
        let module = unsafe {
            device.create_shader_module_passthrough(wgpu::ShaderModuleDescriptorPassthrough {
                label: Some(entry),
                metallib: Some(bytes.into()),
                ..Default::default()
            })
        };
        (module, std::fs::read_to_string(dir.join(format!("{entry}.entry"))).unwrap())
    });
    let libraries = started.elapsed();
    let result =
        pipeline(device, atlas, (&shaders[0].0, &shaders[0].1), (&shaders[1].0, &shaders[1].1));
    eprintln!(
        "METAL_PROBE mode=metallib library_ms={:.3} total_ms={:.3}",
        libraries.as_secs_f64() * 1000.0,
        started.elapsed().as_secs_f64() * 1000.0
    );
    result
}

fn source_pipeline(
    device: &wgpu::Device,
    atlas: &wgpu::BindGroupLayout,
    source: &str,
) -> wgpu::RenderPipeline {
    let started = Instant::now();
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("shadow_shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });
    let result = pipeline(device, atlas, (&shader, "vs_cell"), (&shader, "fs_blur_x"));
    eprintln!("METAL_PROBE mode=wgsl total_ms={:.3}", started.elapsed().as_secs_f64() * 1000.0);
    result
}

#[test]
#[ignore = "manual Metal A/B and pixel parity; requires compiled library artifacts"]
fn precompiled_shadow_blur_probe() {
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("Metal GPU required");
    assert_eq!(adapter.get_info().backend, wgpu::Backend::Metal);
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features: wgpu::Features::PASSTHROUGH_SHADERS,
        ..Default::default()
    }))
    .unwrap();
    let atlas = shadow::read_layout(&device);
    let mode = std::env::var("HARMONIGRAPH_METAL_PROBE_MODE").unwrap_or_else(|_| "parity".into());
    if mode == "wgsl" {
        let _ = source_pipeline(&device, &atlas, SOURCE);
        return;
    }
    let precompiled = compiled(&device, &atlas, &directory());
    if mode == "metallib" {
        return;
    }
    assert_eq!(mode, "parity");
    let mut shooter = Shooter {
        device,
        queue,
        resources: Default::default(),
        format: wgpu::TextureFormat::Rgba8Unorm,
        size: [256, 256],
        pane: 1,
        clear: wgpu::Color::BLACK,
    };
    let without_blur = SOURCE.replace(
        "return vec4<f32>(blur(in, vec2<i32>(1, 0)), 0.0, 0.0, 1.0);",
        "return vec4<f32>(0.0);",
    );
    assert_ne!(without_blur, SOURCE);
    let erased = source_pipeline(&shooter.device, &atlas, &without_blur);
    for width in [0.25, 1.0] {
        let mut scene = parity_scene();
        scene.shadow = one_shadow(width, 1.0, harmonigraph_scene::ShadowKernel::Gaussian);
        // Prepare the production resources, then keep all pipelines except
        // blur_x fixed. A fresh pane for each shot avoids temporal history.
        let _ = shooter.shot(&scene);
        let original = shooter
            .resources
            .get::<LatticeResources>()
            .unwrap()
            .compiled
            .shadow_cell_pipelines
            .blur_x
            .clone();
        let expected = shooter.shot(&scene);
        shooter
            .resources
            .get_mut::<LatticeResources>()
            .unwrap()
            .compiled
            .shadow_cell_pipelines
            .blur_x = precompiled.clone();
        let actual = shooter.shot(&scene);
        assert_eq!(
            differing_pixels(&expected, &actual),
            0,
            "precompiled blur changed pixels at width {width}"
        );
        shooter
            .resources
            .get_mut::<LatticeResources>()
            .unwrap()
            .compiled
            .shadow_cell_pipelines
            .blur_x = erased.clone();
        let missing = shooter.shot(&scene);
        let changed = differing_pixels(&expected, &missing);
        assert!(changed > 100, "fixture must visibly exercise the replaced blur pipeline");
        eprintln!(
            "METAL_PROBE parity width={width} changed_pixels=0 erased_control_pixels={changed}"
        );
        shooter
            .resources
            .get_mut::<LatticeResources>()
            .unwrap()
            .compiled
            .shadow_cell_pipelines
            .blur_x = original;
    }
}
