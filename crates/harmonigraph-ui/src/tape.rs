//! What the Tape tab shows, and what a click in it asks (#1719).
//!
//! The plugin's input worker fills a [`TapeView`] under the editor lock once a
//! step and takes its request. A shell with no tape leaves it at its default,
//! which says there is none.

/// What a click in the Tape tab asks of the plugin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TapeRequest {
    /// Audition from these song seconds.
    Cue(f64),
    /// End the audition.
    Stop,
}

#[derive(Clone, Debug, Default)]
pub struct TapeView {
    /// A tape exists here: the plugin has one, other shells do not.
    pub present: bool,
    /// The analysis input is Main and the host is not rendering offline: the
    /// only times the tape records or sounds.
    pub enabled: bool,
    /// The transport rolls, so a cue would be cut at once.
    pub rolling: bool,
    /// The song position the host last reported, in seconds. While stopped
    /// it is where playback stopped: Bitwig does not report a playhead moved
    /// while stopped.
    pub position: Option<f64>,
    /// The song seconds sounding now, while an audition plays.
    pub heard: Option<f64>,
    /// Memory the recording holds.
    pub bytes: usize,
    /// A click the plugin has not taken yet. A newer click replaces it.
    pub request: Option<TapeRequest>,
}
