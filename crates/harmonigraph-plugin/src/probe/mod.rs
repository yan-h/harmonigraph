//! Opt-in, disposable #615 apparatus. Candidate clocks/delay are not host findings.

use std::path::PathBuf;

use nice_plug::prelude::*;
use serde::{Deserialize, Serialize};

pub mod session;
mod trace;
mod tuner;

pub use session::Hub;
pub use tuner::HarmonigraphTune;

pub const MAX_SOURCES: usize = 8;
pub const MAX_EVENTS: usize = 2048;
pub const QUEUE_CAPACITY: usize = 4096;

pub fn directory() -> std::io::Result<PathBuf> {
    // Read only off the callback. The CLI and DAW must share the default,
    // but another local user must not be able to pre-create it under /tmp.
    if let Some(path) = std::env::var_os("HARMONIGRAPH_PROBE_DIR").filter(|p| !p.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    std::env::var_os("HOME")
        .filter(|p| !p.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE").filter(|p| !p.is_empty()))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .map(|p| p.join(".cache/harmonigraph/tuning-probe"))
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "probe home directory unavailable; set HARMONIGRAPH_PROBE_DIR",
            )
        })
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub delay_samples: u32,
    pub expected_sources: usize,
    pub hold_source: usize,
    pub hold_request: u64,
    pub hold_extra_samples: u32,
    pub keep_alive: bool,
    pub hub_clock_offset: i64,
    pub source_clock_offsets: [i64; MAX_SOURCES],
}

impl Default for Config {
    fn default() -> Self {
        Self {
            delay_samples: 2048,
            expected_sources: 3,
            hold_source: 0,
            hold_request: 1,
            hold_extra_samples: 0,
            keep_alive: true,
            hub_clock_offset: 0,
            source_clock_offsets: [0; MAX_SOURCES],
        }
    }
}

impl Config {
    pub fn read() -> Result<Self, String> {
        let path = directory().map_err(|e| e.to_string())?.join("config.json");
        let config: Self = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => return Err(e.to_string()),
        };
        if !(1..=MAX_SOURCES).contains(&config.expected_sources)
            || config.delay_samples == 0
            || config.delay_samples > 1_048_576
            || config.hold_extra_samples > 1_048_576
            || config.hub_clock_offset.unsigned_abs() > 1_000_000_000
            || config.source_clock_offsets.iter().any(|v| v.unsigned_abs() > 1_000_000_000)
        {
            return Err("probe configuration outside bounded experiment limits".into());
        }
        Ok(config)
    }
}

pub fn event_key(event: &NoteEvent<()>) -> Option<u8> {
    match event {
        NoteEvent::NoteOn { note, .. }
        | NoteEvent::NoteOff { note, .. }
        | NoteEvent::Choke { note, .. }
        | NoteEvent::VoiceTerminated { note, .. }
        | NoteEvent::PolyPressure { note, .. }
        | NoteEvent::PolyVolume { note, .. }
        | NoteEvent::PolyPan { note, .. }
        | NoteEvent::PolyTuning { note, .. }
        | NoteEvent::PolyVibrato { note, .. }
        | NoteEvent::PolyExpression { note, .. }
        | NoteEvent::PolyBrightness { note, .. } => Some(*note),
        _ => None,
    }
}

pub fn event_name(event: &NoteEvent<()>) -> &'static str {
    match event {
        NoteEvent::NoteOn { .. } => "note_on",
        NoteEvent::NoteOff { .. } => "note_off",
        NoteEvent::Choke { .. } => "choke",
        NoteEvent::PolyTuning { .. } => "tuning",
        NoteEvent::MidiCC { .. } => "cc",
        NoteEvent::MidiPitchBend { .. } => "pitch_bend",
        _ => "expression_or_midi",
    }
}

pub fn retime(event: &mut NoteEvent<()>, sample: u32) -> bool {
    match event {
        NoteEvent::NoteOn { timing, .. }
        | NoteEvent::NoteOff { timing, .. }
        | NoteEvent::Choke { timing, .. }
        | NoteEvent::VoiceTerminated { timing, .. }
        | NoteEvent::PolyModulation { timing, .. }
        | NoteEvent::MonoAutomation { timing, .. }
        | NoteEvent::PolyPressure { timing, .. }
        | NoteEvent::PolyVolume { timing, .. }
        | NoteEvent::PolyPan { timing, .. }
        | NoteEvent::PolyTuning { timing, .. }
        | NoteEvent::PolyVibrato { timing, .. }
        | NoteEvent::PolyExpression { timing, .. }
        | NoteEvent::PolyBrightness { timing, .. }
        | NoteEvent::MidiChannelPressure { timing, .. }
        | NoteEvent::MidiPitchBend { timing, .. }
        | NoteEvent::MidiCC { timing, .. }
        | NoteEvent::MidiProgramChange { timing, .. }
        | NoteEvent::MidiSysEx { timing, .. } => {
            *timing = sample;
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests;
