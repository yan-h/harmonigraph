//! Recording a take: capturing everything the visualization is a
//! function of, so it can be re-rendered offline into a video.
//!
//! # Why this is a crate of its own
//!
//! `harmonigraph-take` is the take FORMAT, and stays tiny — serde, ron and
//! `harmonigraph-core` — so anything that reads or writes a take can have it
//! without a GUI stack. Recording one is a different job: rings on the audio
//! thread, transport handling, and a subprocess driver, none of which the
//! format should have to carry to be read. So the recorder sits BESIDE the
//! format crate rather than inside it.
//!
//! What it does NOT need is the editor. `ParamKey` (what an automation record
//! is named by) and `RenderConfig`/`RenderProgress` (what the Video pane asks
//! for and reads back) live in `harmonigraph-take`, so this crate needs no
//! harmonigraph crate but core and the format — the rest of the list is
//! `rtrb` and `parking_lot`, the two audio-thread primitives. That is what
//! makes the manifest's "nothing here reaches egui or wgpu" a promise the
//! dependency list can keep rather than only an intention.
//!
//! The manifest's OTHER promise — that nothing on the record path allocates
//! or blocks — is not one a dependency list can make either way: `parking_lot`
//! is a blocking mutex and is in it. That one is kept by the code, and the
//! rings are where to check it.
//!
//! Nothing here touches the plugin API. That is what lets the whole
//! record-and-render path — rings, transport handling, subprocess driver,
//! stderr parsing — be tested without building a CLAP/VST3 bundle, and reached
//! from any shell that wants it rather than only from the plugin.
//!
//! # Why this is a button and not automatic
//!
//! The first version armed itself when nice-plug reported
//! `ProcessMode::Offline`, on the theory that "exporting audio also
//! exports a take". Bitwig disproved it: an export produced a take
//! containing parameter values and 37 `AllOff`s and no notes at all,
//! while the lattice visibly lit up throughout. The only way both are
//! true is that the pass carrying the notes was **not** the pass flagged
//! offline — so the host runs some short offline probe and then renders
//! in realtime mode.
//!
//! Rather than reverse-engineer which pass is the real one, recording is
//! now explicit: a toggle in the Video pane, armed by the user, working
//! in any process mode. That also makes the good workflow possible —
//! play the piece once, as you would anyway, and render the video from
//! the take afterwards. The export never has to cooperate.
//!
//! # Time
//!
//! Events are stamped with **transport position**, not a plugin-local
//! clock, and only recorded while the transport is rolling. That means a
//! take lines up with a bounce of the same song with no offset to work
//! out — the two are measured from the same zero. It also means arming
//! the toggle while stopped records nothing until you hit play, which is
//! what the status line is for.
//!
//! Hosts that report no transport fall back to a plugin-local sample
//! count, so the standalone-style "just record what you play" case still
//! works; the take then starts at zero whenever recording was armed.
//!
//! # Threading
//!
//! The audio thread must not open files, allocate, or lock. It only
//! pushes plain `Copy` records into a ring and reads one atomic. A writer
//! thread — started once, for the plugin's lifetime — drains the ring and
//! takes open/close commands over a channel from the GUI thread.
//!
//! Unlike the note ring feeding the GUI, which drops on backpressure by
//! design (a stalled meter must never stall audio), **a dropped take
//! record is a silently wrong video**, so overflow is counted and
//! surfaced in the UI rather than ignored.

// The public paths stay here; implementation boundaries remain private.
pub mod configuration;
pub mod publication;
mod recorder;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use recorder::testing;
pub use recorder::{
    channel, default_renderer_path, header_for, interleaved_reservation, AudioSpec, Control, Entry,
    Recorder, RenderRequest, TAKE_CHANNELS,
};
