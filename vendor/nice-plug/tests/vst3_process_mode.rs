//! The VST3 setup mode handed to `Plugin::initialize` is the mode from that
//! setup call, not the wrapper's previous mode.

use nice_plug::prelude::*;
use nice_plug::wrapper::vst3::{Wrapper, vst3};
use std::sync::atomic::{AtomicU8, Ordering};
use std::{ptr, sync::Arc};
use vst3::Steinberg::Vst::{
    IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentTrait, ProcessModes_,
    ProcessSetup, SymbolicSampleSizes_,
};
use vst3::Steinberg::{IPluginBaseTrait, kResultOk};
use vst3::ComWrapper;

static INITIALIZED_MODE: AtomicU8 = AtomicU8::new(u8::MAX);

#[derive(Default, Params)]
struct Parameters {}

#[derive(Default)]
struct ProcessModeFixture;

impl Plugin for ProcessModeFixture {
    const NAME: &'static str = "VST3 process mode fixture";
    const VENDOR: &'static str = "fixture";
    const URL: &'static str = "";
    const EMAIL: &'static str = "";
    const VERSION: &'static str = "1";
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: Some(new_nonzero_u32(2)),
        main_output_channels: Some(new_nonzero_u32(2)),
        ..AudioIOLayout::const_default()
    }];
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        Arc::new(Parameters::default())
    }

    fn initialize(
        &mut self,
        _: &AudioIOLayout,
        config: &BufferConfig,
        _: &mut impl InitContext<Self>,
    ) -> bool {
        let mode = match config.process_mode {
            ProcessMode::Realtime => 0,
            ProcessMode::Buffered => 1,
            ProcessMode::Offline => 2,
        };
        INITIALIZED_MODE.store(mode, Ordering::SeqCst);
        true
    }

    fn process(
        &mut self,
        _: &mut Buffer,
        _: &mut AuxiliaryBuffers,
        _: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        ProcessStatus::Normal
    }
}

impl Vst3Plugin for ProcessModeFixture {
    const VST3_CLASS_ID: [u8; 16] = *b"ProcessModeTest_";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Fx];
}

#[test]
fn initialize_receives_the_mode_from_the_current_setup_call() {
    #[allow(clippy::unnecessary_cast)]
    let offline = ProcessModes_::kOffline as i32;
    #[allow(clippy::unnecessary_cast)]
    let sample_32 = SymbolicSampleSizes_::kSample32 as i32;
    let wrapper = ComWrapper::new(Wrapper::<ProcessModeFixture>::new());
    let component = wrapper.to_com_ptr::<IComponent>().unwrap();
    let processor = wrapper.to_com_ptr::<IAudioProcessor>().unwrap();
    let mut setup = ProcessSetup {
        processMode: offline,
        symbolicSampleSize: sample_32,
        maxSamplesPerBlock: 64,
        sampleRate: 48_000.0,
    };

    unsafe {
        assert_eq!(component.initialize(ptr::null_mut()), kResultOk);
        assert_eq!(processor.setupProcessing(&mut setup), kResultOk);
        assert_eq!(component.setActive(1), kResultOk);
    }
    assert_eq!(INITIALIZED_MODE.load(Ordering::SeqCst), 2);
    unsafe {
        assert_eq!(component.setActive(0), kResultOk);
        assert_eq!(component.terminate(), kResultOk);
    }
}
