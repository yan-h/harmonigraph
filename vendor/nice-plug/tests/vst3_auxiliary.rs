//! Exercise auxiliary descriptor bounds through the VST3 COM interface.
use nice_plug::prelude::*;
use nice_plug::wrapper::vst3::{Wrapper, vst3};
use std::{ptr, sync::Arc};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentTrait, ProcessData, ProcessModes_, ProcessSetup, SymbolicSampleSizes_,
};
use vst3::Steinberg::{IPluginBaseTrait, kResultOk};
use vst3::{ComPtr, ComWrapper};

#[derive(Default, Params)]
struct Parameters {}

#[derive(Default)]
struct AuxiliaryFixture<const AUX_OUTPUT: bool>;

impl<const AUX_OUTPUT: bool> Plugin for AuxiliaryFixture<AUX_OUTPUT> {
    const NAME: &'static str = "VST3 auxiliary fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "1";
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: Some(new_nonzero_u32(2)),
        main_output_channels: Some(new_nonzero_u32(2)),
        aux_input_ports: &[new_nonzero_u32(2), new_nonzero_u32(2)],
        aux_output_ports: if AUX_OUTPUT { &[new_nonzero_u32(2)] } else { &[] },
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
        for input in auxiliary.inputs.iter_mut() {
            if input.as_slice_immutable().iter().any(|channel| channel.len() != buffer.samples()) {
                return ProcessStatus::Error("auxiliary input length differs from callback length");
            }
            for (main, sidechain) in buffer.as_slice().iter_mut().zip(input.as_slice()) {
                for (sample, sidechain) in main.iter_mut().zip(sidechain.iter()) {
                    *sample += sidechain;
                }
            }
        }
        for output in auxiliary.outputs.iter_mut() {
            for channel in output.as_slice() {
                channel.fill(0.75);
            }
        }
        ProcessStatus::Normal
    }
}

impl<const AUX_OUTPUT: bool> Vst3Plugin for AuxiliaryFixture<AUX_OUTPUT> {
    const VST3_CLASS_ID: [u8; 16] = *b"AuxiliaryFixture";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

// These enum constants have platform-dependent integer types in the bindings.
#[allow(clippy::unnecessary_cast)]
const SAMPLE_32: i32 = SymbolicSampleSizes_::kSample32 as i32;
#[allow(clippy::unnecessary_cast)]
const REALTIME: i32 = ProcessModes_::kRealtime as i32;

struct Device {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
}

impl Device {
    fn new<const AUX_OUTPUT: bool>() -> Self {
        let wrapper = ComWrapper::new(Wrapper::<AuxiliaryFixture<AUX_OUTPUT>>::new());
        let component = wrapper.to_com_ptr::<IComponent>().unwrap();
        let processor = wrapper.to_com_ptr::<IAudioProcessor>().unwrap();
        let mut setup = ProcessSetup {
            processMode: REALTIME,
            symbolicSampleSize: SAMPLE_32,
            maxSamplesPerBlock: 64,
            sampleRate: 48000.0,
        };
        unsafe {
            assert_eq!(component.initialize(ptr::null_mut()), kResultOk);
            assert_eq!(processor.setupProcessing(&mut setup), kResultOk);
            assert_eq!(component.setActive(1), kResultOk);
            assert_eq!(processor.setProcessing(1), kResultOk);
        }
        Self { component, processor }
    }

    fn check_block(&self, frames: usize, num_inputs: i32, num_outputs: i32, expected: f32) {
        let mut main_input = [[0.25; 64]; 2];
        let mut first_input = [[0.5; 64]; 2];
        let mut second_input = [[0.125; 64]; 2];
        let mut main_output = [[-1.0; 64]; 2];
        let mut auxiliary_output = [[-1.0; 64]; 2];
        let mut main_in = main_input.each_mut().map(|c| c.as_mut_ptr());
        let mut first_in = first_input.each_mut().map(|c| c.as_mut_ptr());
        let mut second_in = second_input.each_mut().map(|c| c.as_mut_ptr());
        let mut main_out = main_output.each_mut().map(|c| c.as_mut_ptr());
        let mut aux_out = auxiliary_output.each_mut().map(|c| c.as_mut_ptr());
        // Keep descriptors beyond the declared counts valid as canaries. An
        // unintended access then fails on signal values, without UB or a crash.
        let mut inputs =
            [descriptor(&mut main_in), descriptor(&mut first_in), descriptor(&mut second_in)];
        let mut outputs = [descriptor(&mut main_out), descriptor(&mut aux_out)];
        let mut process = ProcessData {
            processMode: REALTIME,
            symbolicSampleSize: SAMPLE_32,
            numSamples: frames as i32,
            numInputs: num_inputs,
            numOutputs: num_outputs,
            inputs: inputs.as_mut_ptr(),
            outputs: outputs.as_mut_ptr(),
            inputParameterChanges: ptr::null_mut(),
            outputParameterChanges: ptr::null_mut(),
            inputEvents: ptr::null_mut(),
            outputEvents: ptr::null_mut(),
            processContext: ptr::null_mut(),
        };
        assert_eq!(unsafe { self.processor.process(&mut process) }, kResultOk);
        let mut expected_main = [-1.0; 64];
        expected_main[..frames].fill(expected);
        assert_eq!(main_output, [expected_main; 2], "inputs={num_inputs}, outputs={num_outputs}");
        let mut expected_auxiliary = [-1.0; 64];
        if num_outputs == 2 {
            expected_auxiliary[..frames].fill(0.75);
        }
        assert_eq!(auxiliary_output, [expected_auxiliary; 2], "outputs={num_outputs}");
        assert_eq!(main_input, [[0.25; 64]; 2], "host input must stay unchanged");
        assert_eq!(first_input, [[0.5; 64]; 2], "host input must stay unchanged");
        assert_eq!(second_input, [[0.125; 64]; 2], "host input must stay unchanged");
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            self.processor.setProcessing(0);
            self.component.setActive(0);
            self.component.terminate();
        }
    }
}

fn descriptor(channels: &mut [*mut f32; 2]) -> AudioBusBuffers {
    AudioBusBuffers {
        numChannels: 2,
        silenceFlags: 0,
        __field0: AudioBusBuffers__type0 { channelBuffers32: channels.as_mut_ptr() },
    }
}

#[test]
fn omitted_auxiliary_inputs_are_silent() {
    let device = Device::new::<true>();
    // Alternating counts also checks that missing buses do not retain pointers
    // or samples from the previous callback.
    for (frames, inputs, expected) in
        [(32, 3, 0.875), (64, 1, 0.25), (16, 2, 0.75), (64, 3, 0.875), (32, 1, 0.25)]
    {
        device.check_block(frames, inputs, 2, expected);
    }
}

#[test]
fn omitted_auxiliary_outputs_are_not_written() {
    let device = Device::new::<true>();
    // Missing output storage skips Plugin::process() after the main input copy.
    for (frames, outputs, expected) in [(32, 1, 0.25), (64, 2, 0.875), (16, 1, 0.25)] {
        device.check_block(frames, 3, outputs, expected);
    }
}

#[test]
fn auxiliary_inputs_can_outnumber_outputs() {
    let device = Device::new::<false>();
    device.check_block(64, 3, 1, 0.875);
}
