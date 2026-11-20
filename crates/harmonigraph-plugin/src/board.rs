//! Lights the lattice-board's keys for the notes Harmonigraph draws.
//!
//! While Decode fifths is on, every held note the lattice draws, from any
//! source and from the tape, lights the board key nearest its sounding pitch
//! in the board's own tuning (the Decoded fifth, the lattice's C offset):
//! [`nearest_fifths_key`], in the `(channel, key)` encoding the board reads
//! in its Fifths mode. So a note a Lattice Map moved lights the key it
//! sounds on, not the one played. There is no setting of its own; Decode
//! fifths is already the board's.
//!
//! The input worker runs it once a poll, after its step: it reads the tracker
//! under the editor lock it takes anyway and the configuration from its
//! lock-free published copy, and sends with no lock held. Nothing runs on the
//! audio thread or the host's main thread. The board is found by its USB
//! product name, and looked for again at most once a second while there is
//! something to light; a failed send drops the port, which is then looked for
//! again the same way.
//!
//! The board drops a remote key at its first Note Off and holds 32, so what
//! is sent is the set of keys rather than the notes: two notes on one key
//! send one Note On, and the Note Off goes when the last of them is released.
//! Opening the port and going dark send All Notes Off, which the board's
//! firmware reads as clearing every remote key.
//!
//! Only macOS has a port to send to, so elsewhere the board is never made and
//! all but its tests are dead code.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::time::{Duration, Instant};

use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::tuning::nearest_fifths_key;
use harmonigraph_core::NoteTracker;

/// A `(channel, key)` the board reads, channel `0..=15`.
pub(crate) type Key = (u8, u8);

/// How often a missing board is looked for while there is something to light.
const RETRY: Duration = Duration::from_secs(1);

/// All Notes Off on channel 1; the firmware clears every remote key on it,
/// whatever the channel.
const ALL_NOTES_OFF: [u8; 3] = [0xb0, 123, 0];

/// The sounding pitches of the held notes the lattice draws, copied under
/// the editor lock so the keys are found after it is let go.
pub(crate) fn sounding(tracker: &NoteTracker) -> Vec<f32> {
    tracker
        .held_voices()
        .filter(|voice| !tracker.hides(voice.source))
        .map(|voice| voice.pitch)
        .collect()
}

/// The keys the board should show for `pitches`, sorted: `None` while Decode
/// fifths is off, when the board is left dark.
pub(crate) fn wanted(pitches: &[f32], config: &ResolvedConfig) -> Option<Vec<Key>> {
    if !config.policy.decode_fifths {
        return None;
    }
    let (c_offset, fifth) = (config.tuning.c_offset, config.policy.decoded_fifth);
    let mut keys: Vec<Key> =
        pitches.iter().filter_map(|&pitch| nearest_fifths_key(pitch, c_offset, fifth)).collect();
    keys.sort_unstable();
    keys.dedup();
    Some(keys)
}

/// Where the board's messages go: CoreMIDI on macOS, a recorder in tests.
pub(crate) trait Wire {
    /// Opens the board's port, or says why it could not.
    fn open(&mut self) -> Result<(), String>;
    fn send(&mut self, message: [u8; 3]) -> Result<(), String>;
    fn close(&mut self);
}

/// What the board has been sent, and the port it went through.
pub(crate) struct Board<W: Wire> {
    wire: W,
    open: bool,
    /// The keys the board was sent a Note On for and no Note Off since, sorted.
    lit: Vec<Key>,
    /// The earliest a missing board is looked for again.
    retry: Option<Instant>,
    /// The board's absence was logged since it was last open.
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
            retry: None,
            missing_logged: false,
            logs: Vec::new(),
        }
    }

    /// Nothing lit, no port open and no Console line owed: a poll with
    /// decoding off has nothing to do.
    fn idle(&self) -> bool {
        !self.open && self.logs.is_empty()
    }

    /// Shows `wanted` on the board, or leaves it dark for `None`.
    pub(crate) fn update(&mut self, wanted: Option<Vec<Key>>, now: Instant) {
        let Some(wanted) = wanted else {
            self.go_dark();
            return;
        };
        if !self.open {
            if wanted.is_empty() || self.retry.is_some_and(|at| now < at) {
                return;
            }
            if let Err(why) = self.wire.open() {
                self.retry = Some(now + RETRY);
                if !std::mem::replace(&mut self.missing_logged, true) {
                    self.logs.push(format!(
                        "lattice-board not lit: {why}. Looked for again once a second while notes sound.",
                    ));
                }
                return;
            }
            self.open = true;
            self.retry = None;
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

    /// A send failed: the port is dropped and looked for again.
    fn lose(&mut self, why: &str, now: Instant) {
        self.wire.close();
        self.open = false;
        self.lit.clear();
        self.retry = Some(now + RETRY);
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
        self.retry = None;
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
/// gives its USB device.
#[cfg(target_os = "macos")]
#[derive(Default)]
pub(crate) struct Midi {
    connection: Option<midir::MidiOutputConnection>,
}

#[cfg(target_os = "macos")]
impl Midi {
    const PRODUCT: &'static str = "LatticeBoard";
}

#[cfg(target_os = "macos")]
impl Wire for Midi {
    fn open(&mut self) -> Result<(), String> {
        let output = midir::MidiOutput::new("Harmonigraph").map_err(|error| error.to_string())?;
        let port = output
            .ports()
            .into_iter()
            .find(|port| output.port_name(port).is_ok_and(|name| name.contains(Self::PRODUCT)))
            .ok_or_else(|| format!("no MIDI port named {}", Self::PRODUCT))?;
        let connection =
            output.connect(&port, "Harmonigraph lights").map_err(|error| error.to_string())?;
        self.connection = Some(connection);
        Ok(())
    }

    fn send(&mut self, message: [u8; 3]) -> Result<(), String> {
        let connection = self.connection.as_mut().ok_or("the port is closed")?;
        connection.send(&message).map_err(|error| error.to_string())
    }

    fn close(&mut self) {
        if let Some(connection) = self.connection.take() {
            connection.close();
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
    /// `None` off macOS, the one platform the plugin runs on, and in tests,
    /// which must not reach a board plugged into the machine running them.
    pub(crate) fn live(settings: std::sync::Arc<crate::configuration::Settings>) -> Option<Self> {
        #[cfg(all(target_os = "macos", not(test)))]
        return Some(Self { settings, board: Board::new(Midi::default()) });
        #[cfg(not(all(target_os = "macos", not(test))))]
        {
            let _ = settings;
            None
        }
    }

    /// One poll: the pitches under the lock, the keys and messages without
    /// it. With decoding off and the board dark it does not take the lock.
    pub(crate) fn poll(&mut self, shared: &crate::editor::EditorLock) {
        let config = self.settings.applied();
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

    /// Records what was sent; `present` decides whether the port opens.
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
        fn send(&mut self, message: [u8; 3]) -> Result<(), String> {
            if !*self.present.borrow() {
                return Err("unplugged".to_string());
            }
            self.sent.borrow_mut().push(message);
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
        let mut tracker = NoteTracker::new();
        let source = SourceId(1);
        // A B♭ key retuned to 7/4 above C sounds an A♯ (+10 fifths, Ch 2), and
        // lights it rather than the B♭ its raw key would; a second note is released.
        tracker.handle_event(NoteEvent::on(0.0, source, 0, 70, 1.0));
        let semitones = (12.0 * 1.75f64.log2()) as f32 - 10.0;
        let kind = NoteEventKind::Tuning { semitones };
        tracker.handle_event(NoteEvent { time: 0.0, source, channel: 0, note: 70, kind });
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
        assert_eq!(tracker.held_voices().count(), 2, "the hidden G is held");
        let pitches = sounding(&tracker);
        assert_eq!(wanted(&pitches, &config), None, "Decode fifths is off by default");
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
        board.update(Some(vec![(0, 60)]), start + RETRY / 2);
        assert!(wire.sent.borrow().is_empty(), "not looked for within the second");
        board.update(Some(vec![(0, 60)]), start + RETRY);
        assert_eq!(*wire.sent.borrow(), [ALL_NOTES_OFF, [0x90, 60, 100]]);
    }

    #[test]
    fn turning_decoding_off_darkens_the_board() {
        let (mut board, wire) = board(true);
        let now = Instant::now();
        board.update(Some(vec![(0, 60)]), now);
        board.update(None, now);
        assert_eq!(wire.sent.borrow().last(), Some(&ALL_NOTES_OFF));
        board.update(Some(vec![(0, 62)]), now);
        assert_eq!(wire.sent.borrow()[3..], [ALL_NOTES_OFF, [0x90, 62, 100]], "relit whole");
    }
}
