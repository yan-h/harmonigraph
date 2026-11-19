//! Missing inputs are silent; a missing output leaves the callback without audio.
use clap_sys::{
    audio_buffer::clap_audio_buffer,
    factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory},
    host::clap_host,
    plugin::clap_plugin,
    process::{CLAP_PROCESS_CONTINUE, clap_process},
    version::CLAP_VERSION,
};
use nice_plug::prelude::*;
use std::{
    ffi::{c_char, c_void},
    ptr,
    sync::Arc,
};

#[derive(Default)]
struct AuxiliaryFixture;

impl Plugin for AuxiliaryFixture {
    const NAME: &'static str = "Auxiliary fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const VERSION: &'static str = "1";
    const CLAP_ID: &'static str = "fixture.auxiliary";
    const CLAP_DESCRIPTION: Option<&'static str> = None;
    const CLAP_MANUAL_URL: Option<&'static str> = None;
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::AudioEffect];
    const PORTS: Ports = Ports { main_input: 2, main_output: 2, aux_inputs: &[(2, "Sidechain")] };

    fn params(&self) -> Arc<Table> {
        Arc::new(Table::empty())
    }

    /// Adds the sidechain into the main output.
    fn process(&mut self, block: &mut Block<'_>) {
        let Some(Audio { main, aux }) = block.audio.as_mut() else { return };
        let frames = block.boundary.frames as usize;
        // Report a malformed slice outside the FFI callback.
        assert!(
            main.iter().chain(aux[0].iter()).all(|channel| channel.len() == frames),
            "a channel's length differs from the callback's"
        );
        for (main, input) in main.iter_mut().zip(aux[0].iter()) {
            for (sample, sidechain) in main.iter_mut().zip(input.iter()) {
                *sample += sidechain;
            }
        }
    }
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

/// The sidechain the host gives, port by port and channel by channel, and
/// silence for what it leaves out, without reading a descriptor past the
/// count it declares (#638); no main output leaves the callback without
/// audio, and the output untouched.
#[test]
fn omitted_auxiliary_inputs_are_silent_and_omitted_outputs_untouched() {
    let device = Device::new();
    for (block, (frames, auxiliary, channels, output)) in [
        (32, true, 2, true),
        // After a full port, so a channel left as it was fails.
        (16, true, 1, true),
        (64, false, 2, true),
        (64, true, 2, false),
        (64, true, 2, true),
        (32, false, 2, true),
    ]
    .into_iter()
    .enumerate()
    {
        let mut main_input = [[0.25; 64]; 2];
        let mut auxiliary_input = [[0.5; 64]; 2];
        let mut main_output = [[-1.0; 64]; 2];
        let mut main_in = main_input.each_mut().map(|c| c.as_mut_ptr());
        let mut aux_in = auxiliary_input.each_mut().map(|c| c.as_mut_ptr());
        let mut main_out = main_output.each_mut().map(|c| c.as_mut_ptr());
        // The second descriptor is outside the DECLARED array when the port is
        // omitted. Keep backing memory valid so a one-past read fails on signal
        // values rather than invoking undefined behaviour or relying on a crash.
        let inputs = [
            descriptor(&mut main_in),
            clap_audio_buffer { channel_count: channels, ..descriptor(&mut aux_in) },
        ];
        let mut outputs = [descriptor(&mut main_out)];
        let process = clap_process {
            steady_time: block as i64 * 64,
            frames_count: frames as u32,
            transport: ptr::null(),
            audio_inputs: inputs.as_ptr(),
            audio_outputs: outputs.as_mut_ptr(),
            audio_inputs_count: 1 + u32::from(auxiliary),
            audio_outputs_count: u32::from(output),
            in_events: ptr::null(),
            out_events: ptr::null(),
        };
        assert_eq!(
            unsafe { ((*device.plugin).process.unwrap())(device.plugin, &process) },
            CLAP_PROCESS_CONTINUE,
        );
        let expected = |sidechain: bool| {
            let mut expected = [-1.0; 64];
            if output {
                expected[..frames].fill(if auxiliary && sidechain { 0.75 } else { 0.25 });
            }
            expected
        };
        assert_eq!(
            main_output,
            [expected(true), expected(channels > 1)],
            "auxiliary input present={auxiliary}, channels {channels}, output present={output}"
        );
        assert_eq!(auxiliary_input, [[0.5; 64]; 2], "host input must stay unchanged");
    }
}
