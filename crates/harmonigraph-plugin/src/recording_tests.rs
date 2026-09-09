//! Ordinary recording through the actual exported VST3 factory and callback.
use nice_plug::prelude::Plugin;
use nice_plug::wrapper::vst3::vst3;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, IAudioProcessor, IAudioProcessorTrait, IComponent,
    IComponentTrait, ProcessData, ProcessModes_, ProcessSetup, SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    kResultOk, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait, PClassInfo,
};
use vst3::{ComPtr, Interface};

#[allow(clippy::unnecessary_cast)]
const SAMPLE_32: i32 = SymbolicSampleSizes_::kSample32 as i32;
#[allow(clippy::unnecessary_cast)]
const REALTIME: i32 = ProcessModes_::kRealtime as i32;

struct Device {
    component: ComPtr<IComponent>,
    processor: ComPtr<IAudioProcessor>,
}
impl Device {
    fn new() -> Self {
        unsafe {
            let factory =
                ComPtr::<IPluginFactory>::from_raw(crate::GetPluginFactory().cast()).unwrap();
            let mut info: PClassInfo = std::mem::zeroed();
            assert_eq!(factory.getClassInfo(0, &mut info), kResultOk);
            let mut component = ptr::null_mut();
            assert_eq!(
                factory.createInstance(
                    info.cid.as_ptr(),
                    IComponent::IID.as_ptr().cast(),
                    &mut component
                ),
                kResultOk
            );
            let component = ComPtr::<IComponent>::from_raw(component.cast()).unwrap();
            let processor = component.cast::<IAudioProcessor>().unwrap();
            assert_eq!(component.initialize(ptr::null_mut()), kResultOk);
            let mut setup = ProcessSetup {
                processMode: REALTIME,
                symbolicSampleSize: SAMPLE_32,
                maxSamplesPerBlock: 4,
                sampleRate: 48000.0,
            };
            assert_eq!(processor.setupProcessing(&mut setup), kResultOk);
            assert_eq!(component.setActive(1), kResultOk);
            assert_eq!(processor.setProcessing(1), kResultOk);
            Self { component, processor }
        }
    }
    fn block(&self) {
        let mut input = [[0.25, 0.5, 0.75, 1.0], [-0.25, -0.5, -0.75, -1.0]];
        let mut output = [[0.0; 4]; 2];
        let mut inputs = input.each_mut().map(|c| c.as_mut_ptr());
        let mut outputs = output.each_mut().map(|c| c.as_mut_ptr());
        let mut input_bus = descriptor(&mut inputs);
        let mut output_bus = descriptor(&mut outputs);
        let mut data = ProcessData {
            processMode: REALTIME,
            symbolicSampleSize: SAMPLE_32,
            numSamples: 4,
            numInputs: 1,
            numOutputs: 1,
            inputs: &mut input_bus,
            outputs: &mut output_bus,
            inputParameterChanges: ptr::null_mut(),
            outputParameterChanges: ptr::null_mut(),
            inputEvents: ptr::null_mut(),
            outputEvents: ptr::null_mut(),
            processContext: ptr::null_mut(),
        };
        // The dev-enabled assert_process_allocs guards the exported wrapper,
        // including this plugin's actual callback, on every test run.
        assert_eq!(unsafe { self.processor.process(&mut data) }, kResultOk);
        assert_eq!(input, output);
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
fn wait(ready: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready() {
        assert!(std::time::Instant::now() < deadline, "VST3 recording worker stalled");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
struct Resume<'a>(&'a harmonigraph_record::testing::WorkerProbe);
impl Drop for Resume<'_> {
    fn drop(&mut self) {
        self.0.pause_boundary(false);
    }
}

#[test]
fn vst3_stop_during_callback_keeps_the_observed_audio_without_another_callback() {
    // Ordinary Harmonigraph has no configuration owner and does not split a
    // VST3 callback for parameter automation. Its process entry is the boundary.
    const { assert!(!crate::Harmonigraph::SAMPLE_ACCURATE_AUTOMATION) };
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-vst3-stop-{}", std::process::id()));
    let (recorder, control) = harmonigraph_record::channel();
    let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    control.start(48000.0, String::new(), true);
    probe.pause_boundary(true);
    let callback_done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (release, retained) = std::sync::mpsc::channel::<()>();
        let callback_done = &callback_done;
        scope.spawn(move || {
            crate::configuration::inject_recorder(recorder);
            let device = Device::new();
            device.block();
            callback_done.store(true, Ordering::Release);
            // Keep the live plugin: neither destruction nor a second callback
            // may rescue the producer's Stop acknowledgment.
            let _ = retained.recv();
            drop(device);
        });
        let resume = Resume(&probe);
        wait(|| probe.boundary_entered());
        control.stop(None);
        drop(resume);
        wait(|| callback_done.load(Ordering::Acquire));
        wait(|| control.last_take().is_some());
        assert!(!probe.failed());
        let take = harmonigraph_take::Take::read(control.last_take().unwrap()).unwrap();
        assert_eq!(take.header.audio_start, Some(0.0));
        let bytes = std::fs::read(control.last_take().unwrap().with_extension("wav")).unwrap();
        assert_eq!(bytes.len(), 44 + 8 * 4);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 32);
        let samples: Vec<_> = bytes[44..]
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        assert_eq!(samples, [0.25, -0.25, 0.5, -0.5, 0.75, -0.75, 1.0, -1.0]);
        drop(release);
    });
    drop(control);
    wait(|| probe.finished());
    std::fs::remove_dir_all(directory).unwrap();
}
