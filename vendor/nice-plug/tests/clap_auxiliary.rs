//! Missing inputs are silent; missing outputs leave the callback unprocessed.
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory},
    host::clap_host,
    plugin::clap_plugin,
    process::{CLAP_PROCESS_CONTINUE_IF_NOT_QUIET, clap_process},
    version::CLAP_VERSION,
};
use nice_plug::prelude::*;
use std::{
    ffi::{c_char, c_void},
    ptr,
    sync::Arc,
};

#[derive(Default, Params)]
struct Parameters {}

#[derive(Default)]
struct AuxiliaryFixture;

impl Plugin for AuxiliaryFixture {
    const NAME: &'static str = "Auxiliary fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "1";
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: Some(new_nonzero_u32(2)),
        main_output_channels: Some(new_nonzero_u32(2)),
        aux_input_ports: &[new_nonzero_u32(2)],
        aux_output_ports: &[new_nonzero_u32(2)],
        ..AudioIOLayout::const_default()
    }];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        Arc::new(Parameters::default())
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        auxiliary: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Report a malformed slice outside the FFI callback, before using any
        // iterator whose safety relies on samples() matching the slice length.
        if auxiliary.inputs[0]
            .as_slice_immutable()
            .iter()
            .any(|channel| channel.len() != buffer.samples())
        {
            return ProcessStatus::Error("auxiliary input length differs from callback length");
        }
        for (main, input) in buffer.as_slice().iter_mut().zip(auxiliary.inputs[0].as_slice()) {
            for (sample, sidechain) in main.iter_mut().zip(input.iter()) {
                *sample += sidechain;
            }
        }
        for channel in auxiliary.outputs[0].as_slice() {
            channel.fill(0.75);
        }
        ProcessStatus::Normal
    }
}

impl ClapPlugin for AuxiliaryFixture {
    const CLAP_ID: &'static str = "fixture.auxiliary";
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect];
}
nice_export_clap!(AuxiliaryFixture);

unsafe extern "C" fn extension(_: *const clap_host, _: *const c_char) -> *const c_void {
    ptr::null()
}
unsafe extern "C" fn request(_: *const clap_host) {}

struct Device {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
}

impl Device {
    fn new() -> Self {
        let host = Box::new(clap_host {
            clap_version: CLAP_VERSION,
            host_data: ptr::null_mut(),
            name: c"fixture".as_ptr(),
            vendor: c"fixture".as_ptr(),
            url: c"".as_ptr(),
            version: c"1".as_ptr(),
            get_extension: Some(extension),
            request_restart: Some(request),
            request_process: Some(request),
            request_callback: Some(request),
        });
        let factory = unsafe { (clap_entry.get_factory.unwrap())(CLAP_PLUGIN_FACTORY_ID.as_ptr()) }
            .cast::<clap_plugin_factory>();
        let plugin = unsafe {
            ((*factory).create_plugin.unwrap())(factory, &*host, c"fixture.auxiliary".as_ptr())
        };
        assert!(!plugin.is_null());
        assert!(unsafe { ((*plugin).init.unwrap())(plugin) });
        assert!(unsafe { ((*plugin).activate.unwrap())(plugin, 48000.0, 1, 64) });
        assert!(unsafe { ((*plugin).start_processing.unwrap())(plugin) });
        Self { plugin, _host: host }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            ((*self.plugin).stop_processing.unwrap())(self.plugin);
            ((*self.plugin).deactivate.unwrap())(self.plugin);
            ((*self.plugin).destroy.unwrap())(self.plugin);
        }
    }
}

fn descriptor(channels: &mut [*mut f32; 2]) -> clap_audio_buffer {
    clap_audio_buffer {
        data32: channels.as_mut_ptr(),
        data64: ptr::null_mut(),
        channel_count: 2,
        latency: 0,
        constant_mask: 0,
    }
}

#[test]
fn omitted_auxiliary_descriptors_are_not_read_or_written() {
    let device = Device::new();
    for (block, (frames, input_present, output_present)) in [
        (32, true, true),
        (64, false, true),
        (16, true, false),
        (64, false, false),
        (64, true, true),
        (32, false, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut main_input = [[0.25; 64]; 2];
        let mut auxiliary_input = [[0.5; 64]; 2];
        let mut main_output = [[-1.0; 64]; 2];
        let mut auxiliary_output = [[-1.0; 64]; 2];
        let mut main_in = main_input.each_mut().map(|c| c.as_mut_ptr());
        let mut aux_in = auxiliary_input.each_mut().map(|c| c.as_mut_ptr());
        let mut main_out = main_output.each_mut().map(|c| c.as_mut_ptr());
        let mut aux_out = auxiliary_output.each_mut().map(|c| c.as_mut_ptr());
        // The second descriptor is outside the DECLARED array when the port is
        // omitted. Keep backing memory valid so a one-past read fails on signal
        // values rather than invoking undefined behaviour or relying on a crash.
        let inputs = [descriptor(&mut main_in), descriptor(&mut aux_in)];
        let mut outputs = [descriptor(&mut main_out), descriptor(&mut aux_out)];
        let process = clap_process {
            steady_time: block as i64 * 64,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: inputs.as_ptr(),
            audio_outputs: outputs.as_mut_ptr(),
            audio_inputs_count: 1 + u32::from(input_present),
            audio_outputs_count: 1 + u32::from(output_present),
            in_events: ptr::null(),
            out_events: ptr::null(),
        };
        assert_eq!(
            unsafe { ((*device.plugin).process.unwrap())(device.plugin, &process) },
            CLAP_PROCESS_CONTINUE_IF_NOT_QUIET,
        );
        // Missing output storage makes the wrapper skip Plugin::process(),
        // after copying main input to output, just as for a missing main bus.
        let mut expected_main = [-1.0; 64];
        expected_main[..frames].fill(if input_present && output_present { 0.75 } else { 0.25 });
        assert_eq!(main_output, [expected_main; 2], "auxiliary input present={input_present}");
        let mut expected_auxiliary = [-1.0; 64];
        expected_auxiliary[..frames].fill(if output_present { 0.75 } else { -1.0 });
        assert_eq!(
            auxiliary_output, [expected_auxiliary; 2],
            "auxiliary output present={output_present}"
        );
        assert_eq!(auxiliary_input, [[0.5; 64]; 2], "host input must stay unchanged");
    }
}
