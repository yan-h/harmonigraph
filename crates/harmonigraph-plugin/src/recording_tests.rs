//! Ordinary recording through the actual exported VST3 factory and callback.
use nice_plug::prelude::Plugin;
use nice_plug::wrapper::vst3::vst3;
use parking_lot::Mutex;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use vst3::Steinberg::Vst::Event_::EventTypes_;
use vst3::Steinberg::Vst::ProcessContext_::StatesAndFlags_;
use vst3::Steinberg::Vst::{
    AudioBusBuffers, AudioBusBuffers__type0, Event, IAudioProcessor, IAudioProcessorTrait,
    IComponent, IComponentTrait, IEventList, IEventListTrait, NoteOffEvent, NoteOnEvent,
    ProcessContext, ProcessData, ProcessModes_, ProcessSetup, SymbolicSampleSizes_,
};
use vst3::Steinberg::{
    kInvalidArgument, kResultOk, tresult, IPluginBaseTrait, IPluginFactory, IPluginFactoryTrait,
    PClassInfo,
};
use vst3::{Class, ComPtr, ComWrapper, Interface};

#[allow(clippy::unnecessary_cast)]
const SAMPLE_32: i32 = SymbolicSampleSizes_::kSample32 as i32;
#[allow(clippy::unnecessary_cast)]
const REALTIME: i32 = ProcessModes_::kRealtime as i32;
#[allow(clippy::unnecessary_cast)]
const PLAYING: u32 = StatesAndFlags_::kPlaying as u32;

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
        self.block_with(ptr::null_mut(), ptr::null_mut());
    }
    /// One callback, optionally carrying the host's event lists. Both lists are
    /// built by the caller BEFORE the callback, so the fixture itself allocates
    /// nothing inside the guarded `process`.
    fn block_with(&self, input_events: *mut IEventList, output_events: *mut IEventList) {
        self.block_in(input_events, output_events, ptr::null_mut());
    }
    /// One callback with the transport playing at `position` samples.
    fn block_at(&self, input_events: *mut IEventList, position: i64) {
        let mut context: ProcessContext = unsafe { std::mem::zeroed() };
        context.state = PLAYING;
        context.sampleRate = 48000.0;
        context.projectTimeSamples = position;
        self.block_in(input_events, ptr::null_mut(), &mut context);
    }
    fn block_in(
        &self,
        input_events: *mut IEventList,
        output_events: *mut IEventList,
        process_context: *mut ProcessContext,
    ) {
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
            inputEvents: input_events,
            outputEvents: output_events,
            processContext: process_context,
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

/// A host event list. `queued` is what the plugin reads; `collected` is what it
/// writes back. One instance plays one role, and both vectors are sized before
/// the callback so neither COM method allocates inside the guard.
struct Events {
    queued: Vec<Event>,
    collected: Mutex<Vec<Event>>,
}
impl Events {
    fn queued(events: Vec<Event>) -> ComWrapper<Self> {
        ComWrapper::new(Self { queued: events, collected: Mutex::new(Vec::new()) })
    }
    fn collector() -> ComWrapper<Self> {
        ComWrapper::new(Self { queued: Vec::new(), collected: Mutex::new(Vec::with_capacity(16)) })
    }
}
impl Class for Events {
    type Interfaces = (IEventList,);
}
impl IEventListTrait for Events {
    unsafe fn getEventCount(&self) -> i32 {
        self.queued.len() as i32
    }
    unsafe fn getEvent(&self, index: i32, event: *mut Event) -> tresult {
        match usize::try_from(index).ok().and_then(|index| self.queued.get(index)) {
            Some(queued) => {
                unsafe { event.write(*queued) };
                kResultOk
            }
            None => kInvalidArgument,
        }
    }
    unsafe fn addEvent(&self, event: *mut Event) -> tresult {
        let mut collected = self.collected.lock();
        assert!(
            collected.len() < collected.capacity(),
            "the collector must not grow under the guard"
        );
        collected.push(unsafe { *event });
        kResultOk
    }
}
/// A raw pointer the callback can read, valid while `wrapper` is alive.
fn event_list(wrapper: &ComWrapper<Events>) -> *mut IEventList {
    wrapper.as_com_ref::<IEventList>().unwrap().as_ptr()
}
fn note_on(note: i16, sample_offset: i32) -> Event {
    let mut event: Event = unsafe { std::mem::zeroed() };
    event.sampleOffset = sample_offset;
    event.r#type = EventTypes_::kNoteOnEvent as u16;
    event.__field0.noteOn =
        NoteOnEvent { channel: 0, pitch: note, tuning: 0.0, velocity: 0.75, length: 0, noteId: -1 };
    event
}
fn note_off(note: i16, sample_offset: i32) -> Event {
    let mut event: Event = unsafe { std::mem::zeroed() };
    event.sampleOffset = sample_offset;
    event.r#type = EventTypes_::kNoteOffEvent as u16;
    event.__field0.noteOff =
        NoteOffEvent { channel: 0, pitch: note, velocity: 0.5, noteId: -1, tuning: 0.0 };
    event
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

/// The plain-MIDI arm of `process` — `mapped_note` → `publish_note` →
/// `take.note` → `send_event` — runs only where the plugin has no
/// configuration owner AND the host actually hands it events. The VST3 wrapper
/// is that shell; every guarded fixture through it passed `inputEvents: null`,
/// so the whole chain was allocation-guarded by nothing.
///
/// Reach is asserted from both ends of the arm rather than assumed: the take
/// file carries what `take.note` wrote, and the collector carries what
/// `send_event` handed back, which is the last statement of the same arm.
#[test]
fn vst3_notes_reach_the_take_and_the_host_through_the_guarded_callback() {
    const { assert!(!crate::Harmonigraph::SAMPLE_ACCURATE_AUTOMATION) };
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-vst3-notes-{}", std::process::id()));
    let (recorder, control) = harmonigraph_record::channel();
    let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    control.start(48000.0, String::new(), true);
    crate::configuration::inject_recorder(recorder);
    let device = Device::new();
    let queued = Events::queued(vec![note_on(60, 0), note_off(60, 3)]);
    let collected = Events::collector();
    device.block_with(event_list(&queued), event_list(&collected));
    control.stop(None);
    drop(device);
    wait(|| control.last_take().is_some());
    assert!(!probe.failed());

    // A transparent MIDI effect hands both events straight back, in order.
    let sent = collected.collected.lock();
    let sent: Vec<_> = sent
        .iter()
        .map(|event| {
            // Read the arm's own union member rather than trading on the two
            // note structs starting with the same two fields.
            let pitch = if event.r#type == EventTypes_::kNoteOnEvent as u16 {
                unsafe { event.__field0.noteOn.pitch }
            } else {
                unsafe { event.__field0.noteOff.pitch }
            };
            (event.r#type, event.sampleOffset, pitch)
        })
        .collect();
    assert_eq!(
        sent,
        vec![(EventTypes_::kNoteOnEvent as u16, 0, 60), (EventTypes_::kNoteOffEvent as u16, 3, 60),],
        "send_event must forward both notes, which is the end of the arm under test"
    );

    let take = harmonigraph_take::Take::read(control.last_take().unwrap()).unwrap();
    let notes: Vec<_> = take.notes().map(|note| (note.note, note.channel, note.kind)).collect();
    assert_eq!(
        notes,
        vec![
            (60, 0, harmonigraph_take::NoteKind::On { velocity: 0.75 }),
            (60, 0, harmonigraph_take::NoteKind::Off),
        ],
        "take.note must record both notes at their own offsets"
    );
    let times: Vec<_> = take.notes().map(|note| note.t).collect();
    assert_eq!(times, vec![0.0, 3.0 / 48000.0], "each note keeps its own sample offset");

    drop(control);
    wait(|| probe.finished());
    std::fs::remove_dir_all(directory).unwrap();
}

/// #1129 on the plain route: a take armed while a note is held. The note-on
/// arrives in a callback no take owns, so the only way the take learns of it
/// is the pass opening with what is already sounding.
#[test]
fn vst3_take_armed_mid_note_opens_with_the_held_note() {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-vst3-held-{}", std::process::id()));
    let (recorder, control) = harmonigraph_record::channel();
    let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    crate::configuration::inject_recorder(recorder);
    let device = Device::new();
    let struck = Events::queued(vec![note_on(60, 1)]);
    device.block_with(event_list(&struck), ptr::null_mut());
    control.start(48000.0, String::new(), false);
    device.block();
    let released = Events::queued(vec![note_off(60, 2)]);
    device.block_with(event_list(&released), ptr::null_mut());
    control.stop(None);
    drop(device);
    wait(|| control.last_take().is_some());
    assert!(!probe.failed());

    let take = harmonigraph_take::Take::read(control.last_take().unwrap()).unwrap();
    let notes: Vec<_> = take.notes().map(|note| (note.t, note.note, note.kind)).collect();
    // The first recorded callback is the second, 4 samples in: the held note
    // opens the pass at its first sample, and its release keeps its own time.
    assert_eq!(
        notes,
        vec![
            (4.0 / 48000.0, 60, harmonigraph_take::NoteKind::On { velocity: 0.75 }),
            (8.0 / 48000.0 + 2.0 / 48000.0, 60, harmonigraph_take::NoteKind::Off),
        ]
    );

    drop(control);
    wait(|| probe.finished());
    std::fs::remove_dir_all(directory).unwrap();
}

/// #1129, every pass rather than the first: a loop wraps while a note is
/// held, so the second pass begins with the note already sounding and holds
/// nothing of it but its release unless the split opens it again.
#[test]
fn vst3_a_pass_split_by_a_loop_opens_with_the_note_held_across_it() {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-vst3-split-{}", std::process::id()));
    let (recorder, control) = harmonigraph_record::channel();
    let probe = harmonigraph_record::testing::worker_probe(&control, directory.clone());
    control.start(48000.0, String::new(), false);
    crate::configuration::inject_recorder(recorder);
    let device = Device::new();
    let struck = Events::queued(vec![note_on(60, 1)]);
    device.block_at(event_list(&struck), 48000);
    device.block_at(ptr::null_mut(), 48004);
    // Back a second while playing — past the 50 ms a playing host may jitter
    // backwards — so the loop wraps and the take splits.
    device.block_at(ptr::null_mut(), 0);
    let released = Events::queued(vec![note_off(60, 2)]);
    device.block_at(event_list(&released), 4);
    control.stop(None);
    drop(device);
    wait(|| control.last_take().is_some());
    assert!(!probe.failed());

    let second = control.last_take().unwrap();
    assert!(
        second.to_string_lossy().ends_with("-2.take"),
        "the second pass is voiced, so it is the take: {second:?}"
    );
    let take = harmonigraph_take::Take::read(&second).unwrap();
    let notes: Vec<_> = take.notes().map(|note| (note.t, note.note, note.kind)).collect();
    assert_eq!(
        notes,
        vec![
            (0.0, 60, harmonigraph_take::NoteKind::On { velocity: 0.75 }),
            (4.0 / 48000.0 + 2.0 / 48000.0, 60, harmonigraph_take::NoteKind::Off),
        ]
    );

    drop(control);
    wait(|| probe.finished());
    std::fs::remove_dir_all(directory).unwrap();
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
