//! Lights the lattice-board's keys for the notes Harmonigraph draws.
//!
//! While Decode fifths is on, every held note the lattice draws, from any
//! source and from the tape, lights the board key nearest its sounding pitch
//! in the board's own tuning (the Decoded fifth, the lattice's C offset):
//! [`nearest_fifths_key`] over the fifths the board holds, in the
//! `(channel, key)` encoding its firmware reads. So a note a Lattice Map moved
//! lights the key it sounds on, not the one played. There is no setting of its
//! own; Decode fifths is already the board's.
//!
//! The input worker runs it once a poll, after its step. With decoding on, or
//! the board still owing a reset, it takes the editor lock once more to copy
//! the shown held pitches and write the Console lines owed; it reads the
//! configuration from its lock-free published copy, and finds keys and sends
//! with no lock held. Nothing runs on the audio thread, and the host's main
//! thread only makes the two CoreMIDI clients, once, at plugin creation.
//!
//! **The clients are made on the main thread** because CoreMIDI tells a
//! process about devices that come and go only through the run loop of the
//! thread its first client was made on, and the input worker runs none.
//! Measured on macOS (2026-11-20, a virtual port standing in for the board):
//! with the first client made on a thread with no run loop, a port that
//! appeared later was never listed, by that client or by a fresh one; with it
//! made on a thread running its run loop, a client on another thread listed
//! the port within a second of its appearing and dropped it within a second of
//! its going. A client is never disposed (coremidi leaves that to the
//! process's end), so the two are kept for the plugin's life rather than made
//! per look.
//!
//! The board is found by its USB product name, looked for again at most once
//! a second while there is something to light, and while it is lit checked
//! once a second to still be there, since CoreMIDI need not fail a send to an
//! unplugged device. A board that went is relit whole when it returns.
//!
//! The board drops a remote key at its first Note Off and holds 32, so what
//! is sent is the set of keys rather than the notes: two notes on one key
//! send one Note On, and the Note Off goes when the last of them is released.
//! Opening the port and going dark send All Notes Off, which the board's
//! firmware reads as clearing every remote key.
//!
//! Only macOS has a port to send to, so elsewhere the board is never made and
//! all but its tests are dead code; tests never make the CoreMIDI side.
#![cfg_attr(any(not(target_os = "macos"), test), allow(dead_code))]

use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::tuning::nearest_fifths_key;
use harmonigraph_core::NoteTracker;

/// A `(channel, key)` the board reads, channel `0..=15`.
pub(crate) type Key = (u8, u8);

/// The fifths the 5x25 board holds, E𝄫 to C𝄪, from its firmware's layout
/// (`layout_5x25.rs` in yan-h/lattice-board, keys 33 to 92). A note lights
/// the nearest key among these rather than a spelling the board does not
/// have; a board of another shape needs this changed.
const BOARD_FIFTHS: RangeInclusive<i32> = -10..=14;

/// How often a missing board is looked for, and a lit one checked, while
/// there is something to light.
const LOOK: Duration = Duration::from_secs(1);

/// All Notes Off on channel 1; the firmware clears every remote key on it,
/// whatever the channel.
const ALL_NOTES_OFF: [u8; 3] = [0xb0, 123, 0];

/// The sounding pitches of the held notes the lattice draws, copied under
/// the editor lock so the keys are found after it is let go.
pub(crate) fn sounding(tracker: &NoteTracker) -> Vec<f32> {
    tracker.held_shown().map(|voice| voice.pitch).collect()
}

/// The keys the board should show for `pitches`, sorted: `None` while Decode
/// fifths is off, when the board is left dark.
pub(crate) fn wanted(pitches: &[f32], config: &ResolvedConfig) -> Option<Vec<Key>> {
    if !config.policy.decode_fifths {
        return None;
    }
    let (c_offset, fifth) = (config.tuning.c_offset, config.policy.decoded_fifth);
    let mut keys: Vec<Key> = pitches
        .iter()
        .filter_map(|&pitch| nearest_fifths_key(pitch, c_offset, fifth, BOARD_FIFTHS))
        .collect();
    keys.sort_unstable();
    keys.dedup();
    Some(keys)
}

/// Where the board's messages go: CoreMIDI on macOS, a recorder in tests.
pub(crate) trait Wire {
    /// Opens the board's port, or says why it could not.
    fn open(&mut self) -> Result<(), String>;
    /// Whether the port opened is still listed.
    fn present(&self) -> bool;
    fn send(&mut self, message: [u8; 3]) -> Result<(), String>;
    fn close(&mut self);
}

/// What the board has been sent, and the port it went through.
pub(crate) struct Board<W: Wire> {
    wire: W,
    open: bool,
    /// The keys the board was sent a Note On for and no Note Off since, sorted.
    lit: Vec<Key>,
    /// The earliest the board is looked for again, or checked to be there.
    next_look: Option<Instant>,
    /// The board's absence was logged since it was last open or went dark.
    missing_logged: bool,
    /// Console lines owed, written at the worker's next hold of the lock.
    pub(crate) logs: Vec<String>,
}

impl<W: Wire> Board<W> {
    pub(crate) fn new(wire: W) -> Self {
        Self {
            wire,
            open: false,
            lit: Vec::new(),
            next_look: None,
            missing_logged: false,
            logs: Vec::new(),
        }
    }

    /// Dark with nothing owed: a poll with decoding off has nothing to do.
    fn idle(&self) -> bool {
        !self.open && self.logs.is_empty() && self.next_look.is_none() && !self.missing_logged
    }

    /// Shows `wanted` on the board, or leaves it dark for `None`.
    pub(crate) fn update(&mut self, wanted: Option<Vec<Key>>, now: Instant) {
        let Some(wanted) = wanted else {
            self.go_dark();
            return;
        };
        let due = self.next_look.is_none_or(|at| now >= at);
        if self.open {
            if due && !(wanted.is_empty() && self.lit.is_empty()) {
                self.next_look = Some(now + LOOK);
                if !self.wire.present() {
                    self.lose("unplugged", now);
                    return;
                }
            }
        } else {
            if wanted.is_empty() || !due {
                return;
            }
            self.next_look = Some(now + LOOK);
            if let Err(why) = self.wire.open() {
                if !std::mem::replace(&mut self.missing_logged, true) {
                    self.logs.push(format!(
                        "lattice-board not lit: {why}. Looked for again once a second while notes sound.",
                    ));
                }
                return;
            }
            self.open = true;
            self.missing_logged = false;
            self.logs.push("lattice-board found: lighting its keys".to_string());
            if let Err(why) = self.wire.send(ALL_NOTES_OFF) {
                self.lose(&why, now);
                return;
            }
        }
        let offs = self.lit.iter().filter(|key| wanted.binary_search(key).is_err());
        let ons = wanted.iter().filter(|key| self.lit.binary_search(key).is_err());
        let messages: Vec<[u8; 3]> = offs
            .map(|&(channel, key)| [0x80 | channel, key, 0])
            .chain(ons.map(|&(channel, key)| [0x90 | channel, key, 100]))
            .collect();
        for message in messages {
            if let Err(why) = self.wire.send(message) {
                self.lose(&why, now);
                return;
            }
        }
        self.lit = wanted;
    }

    /// The board went: the port is dropped and looked for again.
    fn lose(&mut self, why: &str, now: Instant) {
        self.wire.close();
        self.open = false;
        self.lit.clear();
        self.next_look = Some(now + LOOK);
        self.logs.push(format!("lattice-board lost ({why}): looking for it again"));
    }

    fn go_dark(&mut self) {
        if self.open {
            // Dark either way: a board that refuses this is gone.
            let _ = self.wire.send(ALL_NOTES_OFF);
            self.wire.close();
        }
        self.open = false;
        self.lit.clear();
        self.next_look = None;
        self.missing_logged = false;
    }
}

impl<W: Wire> Drop for Board<W> {
    /// A removed plugin leaves no key lit.
    fn drop(&mut self) {
        self.go_dark();
    }
}

/// The board's port over CoreMIDI, found by the product name its firmware
/// gives its USB device: one client that only looks, and one that connects.
#[cfg(target_os = "macos")]
pub(crate) struct Midi {
    looker: midir::MidiOutput,
    /// The connecting client while no connection holds it.
    spare: Option<midir::MidiOutput>,
    /// The connection, and the unique id of the port it went to.
    connection: Option<(midir::MidiOutputConnection, String)>,
}

#[cfg(target_os = "macos")]
impl Midi {
    const PRODUCT: &'static str = "LatticeBoard";

    /// On the host's main thread, for the reason the module says.
    fn new() -> Option<Self> {
        let looker = midir::MidiOutput::new("Harmonigraph").ok()?;
        let spare = midir::MidiOutput::new("Harmonigraph lights").ok()?;
        Some(Self { looker, spare: Some(spare), connection: None })
    }

    fn find(&self) -> Option<midir::MidiOutputPort> {
        let looker = &self.looker;
        looker
            .ports()
            .into_iter()
            .find(|port| looker.port_name(port).is_ok_and(|name| name.contains(Self::PRODUCT)))
    }
}

#[cfg(target_os = "macos")]
impl Wire for Midi {
    fn open(&mut self) -> Result<(), String> {
        let port = self.find().ok_or_else(|| format!("no MIDI port named {}", Self::PRODUCT))?;
        let output = self.spare.take().ok_or("its client is connected elsewhere")?;
        match output.connect(&port, "Harmonigraph lights") {
            Ok(connection) => {
                self.connection = Some((connection, port.id()));
                Ok(())
            }
            Err(error) => {
                let why = error.to_string();
                self.spare = Some(error.into_inner());
                Err(why)
            }
        }
    }

    fn present(&self) -> bool {
        let Some((_, id)) = &self.connection else { return false };
        self.looker.ports().iter().any(|port| port.id() == *id)
    }

    fn send(&mut self, message: [u8; 3]) -> Result<(), String> {
        let (connection, _) = self.connection.as_mut().ok_or("the port is closed")?;
        connection.send(&message).map_err(|error| error.to_string())
    }

    fn close(&mut self) {
        if let Some((connection, _)) = self.connection.take() {
            self.spare = Some(connection.close());
        }
    }
}

/// The board and the configuration that decides what it shows, as the input
/// worker holds them.
pub(crate) struct Lights {
    settings: std::sync::Arc<crate::configuration::Settings>,
    #[cfg(target_os = "macos")]
    board: Board<Midi>,
}

impl Lights {
    /// Called on the host's main thread, at plugin creation, which makes the
    /// CoreMIDI clients there. `None` off macOS, the one platform the plugin
    /// runs on; when CoreMIDI makes no client; and in tests, which must not
    /// reach a board plugged into the machine running them.
    pub(crate) fn live(settings: std::sync::Arc<crate::configuration::Settings>) -> Option<Self> {
        #[cfg(all(target_os = "macos", not(test)))]
        return Midi::new().map(|midi| Self { settings, board: Board::new(midi) });
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            let _ = settings;
            None
        }
    }

    /// One poll: the pitches under the lock, the keys and messages without
    /// it. With decoding off and the board idle it does not take the lock.
    pub(crate) fn poll(&mut self, shared: &crate::editor::EditorLock) {
        let config = self.settings.resolved();
        #[cfg(target_os = "macos")]
        {
            if !config.policy.decode_fifths && self.board.idle() {
                return;
            }
            let pitches = {
                let mut guard = shared.lock();
                let runtime = &mut guard.ui.picture.runtime;
                for line in self.board.logs.drain(..) {
                    runtime.console.log(line);
                }
                sounding(&runtime.tracker)
            };
            self.board.update(wanted(&pitches, &config), Instant::now());
        }
        #[cfg(not(target_os = "macos"))]
        let _ = (shared, config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// Records what was sent; `present` is whether the board is plugged in.
    #[derive(Clone, Default)]
    struct Recorder {
        sent: Rc<RefCell<Vec<[u8; 3]>>>,
        present: Rc<RefCell<bool>>,
    }

    impl Wire for Recorder {
        fn open(&mut self) -> Result<(), String> {
            if *self.present.borrow() {
                Ok(())
            } else {
                Err("absent".to_string())
            }
        }
        fn present(&self) -> bool {
            *self.present.borrow()
        }
        /// A send to an unplugged board succeeds, as CoreMIDI's may.
        fn send(&mut self, message: [u8; 3]) -> Result<(), String> {
            if *self.present.borrow() {
                self.sent.borrow_mut().push(message);
            }
            Ok(())
        }
        fn close(&mut self) {}
    }

    fn board(present: bool) -> (Board<Recorder>, Recorder) {
        let wire = Recorder::default();
        *wire.present.borrow_mut() = present;
        (Board::new(wire.clone()), wire)
    }

    #[test]
    fn held_notes_the_lattice_draws_light_their_sounding_keys_while_decoding() {
        use harmonigraph_core::canonical::{SourceBaseline, VoiceBaseline};
        use harmonigraph_core::notes::{NoteEvent, NoteEventKind, SourceId};
        let mut config = harmonigraph_core::configuration::ConfigReducer::default().resolved();
        config.tuning.c_offset = 0;
        config.policy.decoded_fifth = 696_578_428;
        config.policy.decode_fifths = false;
        let mut tracker = NoteTracker::new();
        let source = SourceId(1);
        // B♭ keys on two channels, each retuned to 7/4 above C, sound one A♯
        // (+10 fifths, Ch 2) and light it once, rather than the B♭ their raw
        // keys would; a third note is released.
        let semitones = (12.0 * 1.75f64.log2()) as f32 - 10.0;
        for channel in [0, 1] {
            tracker.handle_event(NoteEvent::on(0.0, source, channel, 70, 1.0));
            let kind = NoteEventKind::Tuning { semitones };
            tracker.handle_event(NoteEvent { time: 0.0, source, channel, note: 70, kind });
        }
        tracker.handle_event(NoteEvent::on(0.0, source, 0, 64, 1.0));
        tracker.handle_event(NoteEvent::off(0.1, source, 0, 64));
        // A G held by a source whose Show is off: held, but not drawn.
        let row = VoiceBaseline {
            note: 67,
            lifetime: 1,
            actual_onset: 0.0,
            pitch_microcents: 67 * 100_000_000,
            velocity: 0.8,
            ..Default::default()
        };
        let hidden = SourceBaseline::new(SourceId::DIRECT, 0.0, 0, false, &[row]).unwrap();
        tracker.replace_source(&hidden).unwrap();
        assert_eq!(tracker.held_voices().count(), 3, "the hidden G is held");
        let pitches = sounding(&tracker);
        assert_eq!(wanted(&pitches, &config), None, "dark with Decode fifths off");
        config.policy.decode_fifths = true;
        assert_eq!(wanted(&pitches, &config), Some(vec![(1, 70)]));
    }

    #[test]
    fn a_key_two_notes_share_is_released_with_the_last() {
        let (mut board, wire) = board(true);
        let now = Instant::now();
        board.update(Some(vec![(0, 60), (1, 61)]), now);
        board.update(Some(vec![(0, 60)]), now);
        board.update(Some(vec![(0, 60)]), now);
        board.update(Some(vec![]), now);
        assert_eq!(
            *wire.sent.borrow(),
            [ALL_NOTES_OFF, [0x90, 60, 100], [0x91, 61, 100], [0x81, 61, 0], [0x80, 60, 0]],
        );
    }

    #[test]
    fn a_missing_board_is_looked_for_once_a_second_and_relit_whole() {
        let (mut board, wire) = board(false);
        let start = Instant::now();
        board.update(Some(vec![(0, 60)]), start);
        assert_eq!(board.logs.len(), 1, "the absence is said once");
        *wire.present.borrow_mut() = true;
        board.update(Some(vec![(0, 60)]), start + LOOK / 2);
        assert!(wire.sent.borrow().is_empty(), "not looked for within the second");
        board.update(Some(vec![(0, 60)]), start + LOOK);
        assert_eq!(*wire.sent.borrow(), [ALL_NOTES_OFF, [0x90, 60, 100]]);
    }

    #[test]
    fn an_unplugged_board_is_noticed_within_a_second_and_relit_whole_on_return() {
        let (mut board, wire) = board(true);
        let start = Instant::now();
        board.update(Some(vec![(0, 60)]), start);
        *wire.present.borrow_mut() = false;
        board.update(Some(vec![(0, 60)]), start + LOOK);
        *wire.present.borrow_mut() = true;
        // The held chord has not changed, so only the check could relight it.
        board.update(Some(vec![(0, 60)]), start + LOOK * 2);
        assert_eq!(
            *wire.sent.borrow(),
            [ALL_NOTES_OFF, [0x90, 60, 100], ALL_NOTES_OFF, [0x90, 60, 100]],
        );
    }

    #[test]
    fn turning_decoding_off_darkens_the_board_and_owes_nothing() {
        let (mut board, wire) = board(true);
        let now = Instant::now();
        board.update(Some(vec![(0, 60)]), now);
        board.update(None, now);
        assert_eq!(wire.sent.borrow().last(), Some(&ALL_NOTES_OFF));
        board.update(Some(vec![(0, 62)]), now);
        assert_eq!(wire.sent.borrow()[3..], [ALL_NOTES_OFF, [0x90, 62, 100]], "relit whole");
        // A missing board: turning decoding off clears what it owes, so the
        // next look says again that it is missing.
        let (mut board, _) = self::board(false);
        board.update(Some(vec![(0, 60)]), now);
        board.logs.clear();
        assert!(!board.idle(), "a look is owed");
        board.update(None, now);
        assert!(board.idle());
        board.update(Some(vec![(0, 60)]), now);
        assert_eq!(board.logs.len(), 1, "the absence is said again");
    }
}
