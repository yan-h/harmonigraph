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
//! is named by) and the export queue's vocabulary — `ExportJob` and
//! `RenderProgress`, what the Video pane reads back — live in
//! `harmonigraph-take`. The Video pane's settings live in `harmonigraph-ui`
//! beside the appearance document: a take's look reaches this crate only as
//! that document's opaque string (#797), and its trigger only as the plain
//! values `Control` is handed each frame, so neither the code nor the tests
//! name their types. So this crate needs no harmonigraph crate but core
//! and the format — the rest of the list is `rtrb` and `parking_lot`, the two
//! audio-thread primitives. That is what makes the manifest's "nothing here
//! reaches egui or wgpu" a promise the dependency list can keep rather than
//! only an intention.
//!
//! The manifest's OTHER promise — that nothing on the record path allocates
//! or blocks — is not one a dependency list can make either way: `parking_lot`
//! is a blocking mutex and is in it. That one is kept by the code, and the
//! rings are where to check it.
//!
//! Nothing here touches the plugin API. That is what lets the whole
//! record-and-render path — rings, transport handling, subprocess driver,
//! stderr parsing — be tested without building a CLAP bundle, and reached
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
//! Everything is recorded on the plugin's own presentation clock, the one
//! the display draws on, timed where it sounds. Song time is chosen by the
//! reader (`harmonigraph_take`), from `Run` records that say where each
//! stretch the take recorded without a pause sits in the take's audio: at the
//! take's first block that is the **transport position**, and after it, the
//! end of the audio already recorded. So a take's notes share one time base
//! with its own WAV, and a pause resumes them where the audio got to rather
//! than where the host says it is (#1760). A take only records while the
//! transport is rolling, so arming the toggle while stopped records nothing
//! until you hit play, which is what the status line is for.
//!
//! Song time used to be the host's position per item, so that a take lined
//! up with a separate bounce of the same song; the take's own WAV replaced
//! that bounce (#903, #972, #1196, #1342).
//!
//! A block with no song position records nothing (Bitwig always reports
//! one): the status line reads "waiting for playback" before the take first
//! rolls, and afterwards what the last block with a position left it at.
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
pub mod transport;

#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use recorder::testing;
pub use recorder::{
    channel, default_renderer_path, header_for, take_dir, Control, Entry, Recorder, TAKE_CHANNELS,
};
