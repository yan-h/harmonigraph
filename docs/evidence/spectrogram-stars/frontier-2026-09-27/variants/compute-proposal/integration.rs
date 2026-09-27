// SCRATCH PROPOSAL ONLY. Intended in atmosphere.rs, all behind cfg(test).
// No repository edit, compilation or GPU validation has been performed.
// Constructor source is the generator's compute.wgsl snapshot, not an f16 or
// per-layer-resolution research source. Build each case's resources under its
// stable research mode; do not flip modes on existing Targets without a key.

#[cfg(test)]
pub(super) struct ResearchHaloCompute {
    pipeline: wgpu::ComputePipeline,
    empty: wgpu::BindGroup,
    group: wgpu::BindGroup,
}

#[cfg(test)]
impl ResearchHaloCompute {
    // mode: 1=gather, 2=shared raw, 3=shared decoded. 0 is original render pass.
    pub fn new(
        device: &wgpu::Device,
        uniform: &wgpu::Buffer,
        atlas: &wgpu::TextureView,
        output: &wgpu::TextureView,
        mode: u32,
    ) -> Self {
        assert!((1..=3).contains(&mode));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("research_star_halo_compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!(
                "/private/tmp/stars-investigation/compute-proposal/compute.wgsl"
            ).into()),
        });
        let constants = [
            ("GROUP_SHARED", if mode >= 2 { 1.0 } else { 0.0 }),
            ("GROUP_DECODED", if mode == 3 { 1.0 } else { 0.0 }),
        ];
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("research_star_halo_compute"),
            layout: None,
            module: &module,
            entry_point: Some("cs_star_halo"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            },
            cache: None,
        });
        let empty = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("research_star_halo_compute_empty"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("research_star_halo_compute_group"),
            layout: &pipeline.get_bind_group_layout(1),
            entries: &[
                wgpu::BindGroupEntry { binding: 3, resource: uniform.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(atlas) },
                wgpu::BindGroupEntry { binding: 11, resource: wgpu::BindingResource::TextureView(output) },
            ],
        });
        Self { pipeline, empty, group }
    }

    pub fn draw(&self, encoder: &mut wgpu::CommandEncoder, size: [u32; 2]) {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("research_star_halo_compute"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.empty, &[]);
        pass.set_bind_group(1, &self.group, &[]);
        pass.dispatch_workgroups(size[0].div_ceil(8), size[1].div_ceil(8), STAR_SLICES as u32);
    }
}

// Required wiring, sequentially:
// 1. Add #[cfg(test)] halo_compute: Option<ResearchHaloCompute> to Targets.
// 2. Extend StarHalos::new with an extra usage argument, or a private helper:
//    actual active compute output adds STORAGE_BINDING to its usage;
//    render cases and halo_scratch retain original usage. No format change.
// 3. After uniform and star_view exist in Targets::new:
//    let halo_compute = (research_mode != 0).then(|| {
//        ResearchHaloCompute::new(device, &uniform, star_view,
//            &halos.as_ref().unwrap().view, research_mode)
//    });
//    Only construct when Stars+halos exist. Store in Self initializer.
// 4. At start of draw_halos, after obtaining halos:
//    #[cfg(test)] if let Some(compute) = &self.halo_compute {
//        self.encoded_passes.fetch_add(1, Ordering::Relaxed);
//        compute.draw(encoder, halos.size);
//        return;
//    }
// 5. Select ordinary f32 source for these three research modes. Run full and
//    half-resolution separately; no per-layer halo_extent overrides apply.
// 6. Existing outer source/full timestamp interval needs no new query slots.
//    It already encloses source -> atlas -> compute -> far/composite dependency.
//    If adding inner timestamps, give this ONE pass its OWN fresh begin/end pair,
//    not five reused render-layer pairs. Optional instrumentation is not needed
//    to compare whole frame totals.
