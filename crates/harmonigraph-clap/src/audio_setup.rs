//! A plugin's audio ports, the host's buffer configuration, and one
//! callback's audio as the plugin reads and writes it.

use clap_sys::process::clap_process;

/// The most channels one port may declare, and the most auxiliary inputs: the
/// bounds of the arrays a callback's audio is built in, on the stack.
pub const MAX_CHANNELS: usize = 8;
pub const MAX_AUX_INPUTS: usize = 4;

/// The plugin's audio ports: one layout, which the host cannot change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ports {
    /// The main input's channels, 0 for no main input.
    pub main_input: u32,
    /// The main output's channels, 0 for no main output.
    pub main_output: u32,
    /// Each auxiliary input's channels and name.
    pub aux_inputs: &'static [(u32, &'static str)],
}

impl Ports {
    /// No audio at all.
    pub const NONE: Ports = Ports { main_input: 0, main_output: 0, aux_inputs: &[] };

    /// The layout's name, as nice-plug generated it.
    pub fn name(&self) -> &'static str {
        match (self.main_input, self.main_output, self.aux_inputs.len()) {
            (0, 0, 0) => "Empty",
            (_, 1, 0) | (1, 0, _) => "Mono",
            (_, 2, 0) | (2, 0, _) => "Stereo",
            (_, 1, _) => "Mono with sidechain",
            (_, 2, _) => "Stereo with sidechain",
            _ => "Multichannel",
        }
    }

    /// The main input, if the plugin has one, then each auxiliary input.
    pub(crate) fn inputs(&self) -> u32 {
        u32::from(self.main_input > 0) + self.aux_inputs.len() as u32
    }

    pub(crate) fn outputs(&self) -> u32 {
        u32::from(self.main_output > 0)
    }

    /// The bounds a layout must stay inside, checked once per plugin type.
    pub(crate) fn check(&self) {
        let channels = [self.main_input, self.main_output]
            .into_iter()
            .chain(self.aux_inputs.iter().map(|&(channels, _)| channels));
        assert!(
            channels.into_iter().all(|channels| channels as usize <= MAX_CHANNELS)
                && self.aux_inputs.len() <= MAX_AUX_INPUTS
                && self.aux_inputs.iter().all(|&(channels, _)| channels > 0),
            "an audio layout past the shell's bounds"
        );
    }
}

/// Configuration for (the host's) audio buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BufferConfig {
    /// The current sample rate.
    pub sample_rate: f32,
    /// The maximum buffer size the host will use. The plugin should be able to accept variable
    /// sized buffers up to this size, or between the minimum and the maximum buffer size if both
    /// are set.
    pub max_buffer_size: u32,
    /// The current processing mode. The host reactivates the plugin any time this changes.
    pub process_mode: ProcessMode,
}

/// The plugin's current processing mode. Exposed through [`BufferConfig::process_mode`]. The host
/// will reactivate the plugin whenever this changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessMode {
    /// The plugin is processing audio in real time at a fixed rate.
    Realtime,
    /// The plugin is rendering audio offline, potentially faster than realtime ('freewheeling').
    /// The host will continuously call the process function back to back until all audio has been
    /// processed.
    Offline,
}

/// One callback's audio. Every channel holds the callback's frames.
pub struct Audio<'a> {
    /// The main output's channels, holding the main input: copied there
    /// when the host does not process in place, and silence in a channel
    /// the input does not have. The plugin writes its output over it.
    pub main: &'a mut [&'a mut [f32]],
    /// Each auxiliary input's channels, in the order [`Ports`] declares them:
    /// the host's audio, read where the host left it, or silence for a port
    /// or a channel the host left out (#638).
    ///
    /// Read-only, and borrowed rather than copied (#1864): CLAP lets a host
    /// share a buffer between an input and an output only for a port's
    /// declared in-place pair, and an auxiliary input declares none
    /// (`ext_audio_ports_get`), so nothing the plugin writes can alias these.
    pub aux: &'a [&'a [&'a [f32]]],
}

/// The silence a callback reads for an auxiliary port or channel the host
/// left out: one plane of the activation's maximum frames, sized there so a
/// callback allocates nothing.
pub(crate) struct Silence(Box<[f32]>);

impl Silence {
    pub(crate) fn new(max_frames: u32) -> Self {
        Self(vec![0.0; max_frames as usize].into_boxed_slice())
    }
}

/// The host's channel pointers for buffer `index` of `buffers`, if the host
/// counted that many and the buffer has 32-bit data.
///
/// # Safety
/// `buffers` must be null or hold `count` valid descriptors.
unsafe fn channels<'a>(
    buffers: *const clap_sys::audio_buffer::clap_audio_buffer,
    count: u32,
    index: usize,
) -> Option<&'a [*mut f32]> {
    if buffers.is_null() || index >= count as usize {
        return None;
    }
    let buffer = unsafe { &*buffers.add(index) };
    (!buffer.data32.is_null()).then(|| unsafe {
        std::slice::from_raw_parts(buffer.data32, buffer.channel_count as usize)
    })
}

/// The arrays one callback's [`Audio`] is built in, on the caller's stack.
#[derive(Default)]
pub(crate) struct Slices<'a> {
    main: [&'a mut [f32]; MAX_CHANNELS],
    aux_channels: [[&'a [f32]; MAX_CHANNELS]; MAX_AUX_INPUTS],
    aux: [&'a [&'a [f32]]; MAX_AUX_INPUTS],
}

/// `process`'s audio for `ports`, built in `slices`, with `silence` for an
/// auxiliary port or channel the host left out: `None` when the layout's main
/// output is
/// missing, or has fewer channels than the layout declares, so there is
/// nothing to write the audio to.
///
/// # Safety
/// `process` and every buffer it points to must be valid for `'a`, each
/// channel `frames_count` long, `silence` at least that long, and no auxiliary
/// input's buffer one the host also handed in as an output.
pub(crate) unsafe fn audio<'a>(
    ports: &Ports,
    process: &clap_process,
    silence: &'a Silence,
    slices: &'a mut Slices<'a>,
) -> Option<Audio<'a>> {
    let frames = process.frames_count as usize;
    let main_channels = ports.main_output as usize;
    let output = unsafe { channels(process.audio_outputs, process.audio_outputs_count, 0) }
        .filter(|output| ports.main_output > 0 && output.len() >= main_channels)?;
    let input = unsafe { channels(process.audio_inputs, process.audio_inputs_count, 0) }
        .filter(|_| ports.main_input > 0)
        .unwrap_or(&[]);
    let Slices { main, aux_channels, aux } = slices;

    for (channel, slot) in main.iter_mut().enumerate().take(main_channels) {
        let out = output[channel];
        match input.get(channel).copied().filter(|_| channel < ports.main_input as usize) {
            // In place: the input is already there.
            Some(input) if input == out => {}
            Some(input) => unsafe { std::ptr::copy(input, out, frames) },
            None => unsafe { std::ptr::write_bytes(out, 0, frames) },
        }
        *slot = unsafe { std::slice::from_raw_parts_mut(out, frames) };
    }

    let first_aux = usize::from(ports.main_input > 0);
    let silence = &silence.0[..frames];
    for (port, (&(declared, _), slots)) in
        ports.aux_inputs.iter().zip(aux_channels.iter_mut()).enumerate()
    {
        // Stop at the host's declared count, before reading the one-past
        // descriptor (#638): an omitted port is silence.
        let host =
            unsafe { channels(process.audio_inputs, process.audio_inputs_count, first_aux + port) }
                .unwrap_or(&[]);
        for (channel, slot) in slots.iter_mut().take(declared as usize).enumerate() {
            // No host is known to hand an input the buffer of an output it
            // has no in-place pair with; one that did would alias the main
            // output the plugin writes (see `Audio::aux`).
            debug_assert!(
                host.get(channel).is_none_or(|input| !output[..main_channels].contains(input)),
                "an auxiliary input shares a main output's buffer"
            );
            *slot = match host.get(channel) {
                Some(&input) => unsafe { std::slice::from_raw_parts(input, frames) },
                None => silence,
            };
        }
    }
    for ((port, slots), slot) in ports.aux_inputs.iter().zip(&*aux_channels).zip(&mut *aux) {
        *slot = &slots[..port.0 as usize];
    }
    Some(Audio { main: &mut main[..main_channels], aux: &aux[..ports.aux_inputs.len()] })
}
