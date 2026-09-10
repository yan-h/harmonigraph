//! The Tune: a delay line, a held set, and a copy of every input on its way to
//! the Hub. The Hub embeds one of these too, so its own track is tuned by the
//! same code that tunes every other one.
//!
//! Nothing here waits. An onset whose correction has not arrived by its own
//! deadline emits at raw pitch and increments a counter; a late reply is
//! dropped. That is the whole of the missed-deadline contract, and it is why
//! this file has no settle, ready or coverage predicate in it.
use nice_plug::wrapper::clap::configuration::OwnedInput;
use nice_plug::wrapper::clap::performance as api;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::event::Event;
use super::queue::Queue;
use super::session::{self, Attached, Capture, Reply};
use super::{setup, CAPTURE_RING, CUT_EVENTS, HELD_PER_SOURCE, PENDING_EVENTS, REPLY_RING};

/// Controllers the cut owes a neutral value for. Sustain, sostenuto and legato
/// outlive the note-offs beside them, so a cut that left one down would hold
/// the very voices it just released.
const PEDALS: [u8; 3] = [64, 66, 69];
/// `CLAP_TRANSPORT_IS_PLAYING`, which the wrapper hands over as a bare flag
/// word rather than as a clap-sys type.
const IS_PLAYING: u32 = 1 << 4;

/// One retained input, waiting for its own emission time.
#[derive(Clone, Copy)]
struct Pending {
    /// Input sample plus D. The line is in this order because input is.
    due: i64,
    serial: u64,
    event: Event,
    /// This onset's copy reached the Hub, so a correction may still arrive.
    /// Cleared when one does, and the reason an unanswered onset is a miss.
    awaiting: bool,
    correction: Option<i64>,
    /// The player's own per-note tuning at the attack. The emitted expression
    /// states this plus the frozen correction, because a CLAP note expression
    /// is the current value rather than a delta.
    player: f64,
}

/// A voice this Tune has actually emitted, and the correction frozen onto it.
#[derive(Clone, Copy)]
struct Voice {
    id: i32,
    channel: u8,
    key: u8,
    correction: i64,
}

/// How this Tune reaches the Hub. A paired Tune owns one row's ring pair; the
/// Hub's own copy hands its records over by a direct call.
enum Link {
    Detached,
    Row(Attached),
    Direct { out: Queue<Capture, CAPTURE_RING>, back: Queue<Reply, REPLY_RING> },
}

pub struct Tune {
    pub shared: Arc<setup::Shared>,
    /// Registration id, minted once per instance. Zero before registration.
    id: u64,
    link: Link,
    /// The Reset generation this Tune has already turned into a session cut.
    setup: u64,
    /// The epoch this Tune runs under. A different session epoch is the cut.
    epoch: u64,
    serial: u64,
    /// Input plus this is the emission time. Fixed for the activation.
    delay: i64,
    rate: f64,
    frames: u32,
    callback: Option<api::Callback>,
    line: Queue<Pending, PENDING_EVENTS>,
    /// Note-offs and pedal neutralisations a cut left owing, emitted at the
    /// first output position the next callback offers.
    cut: Queue<Event, CUT_EVENTS>,
    held: [Option<Voice>; HELD_PER_SOURCE],
    /// Per channel, which of [`PEDALS`] is currently down.
    pedals: [u8; 16],
    status: u32,
    /// The last offset emitted in this callback. A CLAP output list is sorted
    /// by time and the wrapper no longer holds the floor for us, so this is the
    /// whole of what keeps it sorted: every emission takes
    /// `max(self.cursor, block.start)` and moves it.
    cursor: u32,
    /// The transport was playing at the last callback. A falling edge is a
    /// Stop, and a Stop is a cut for the whole session.
    playing: bool,
    pub misses: u64,
    pub dropped: u64,
    pub notes_in: u64,
    pub notes_out: u64,
    pub captured: u64,
}

impl Tune {
    /// A paired Tune. Its ring pair arrives at activation, not here.
    pub fn new(shared: Arc<setup::Shared>) -> Box<Self> {
        Box::new(Self::with_link(shared, Link::Detached))
    }
    /// The Hub's own. Its records never touch a ring; the Hub drains them.
    pub fn direct(shared: Arc<setup::Shared>) -> Self {
        Self::with_link(shared, Link::Direct { out: Queue::default(), back: Queue::default() })
    }
    fn with_link(shared: Arc<setup::Shared>, link: Link) -> Self {
        Self {
            shared,
            id: 0,
            link,
            setup: 0,
            epoch: 0,
            serial: 0,
            delay: 0,
            rate: 0.0,
            frames: 0,
            callback: None,
            line: Queue::default(),
            cut: Queue::default(),
            held: [None; HELD_PER_SOURCE],
            pedals: [0; 16],
            status: 0,
            cursor: 0,
            playing: false,
            misses: 0,
            dropped: 0,
            notes_in: 0,
            notes_out: 0,
            captured: 0,
        }
    }

    pub fn register(&mut self) {
        if self.id == 0 {
            self.id = session::session().register();
        }
    }
    pub fn status(&self) -> u32 {
        self.status
    }
    pub fn slot(&self) -> Option<u8> {
        match &self.link {
            Link::Row(attached) => Some(attached.slot),
            _ => None,
        }
    }
    pub fn delay(&self) -> i64 {
        self.delay
    }
    pub fn held(&self) -> usize {
        self.held.iter().flatten().count()
    }
    pub fn pending(&self) -> usize {
        self.line.len()
    }

    /// Host activation. The delay is resolved here, from the format the host
    /// just advertised, and is fixed for the whole activation. Reactivation is
    /// a cut: the notes standing in the old line were scheduled against a
    /// delay that no longer exists.
    pub fn activate(&mut self, rate: f64, frames: u32, multiplier: u32) {
        self.rate = rate;
        self.frames = frames;
        self.delay = i64::from(multiplier) * i64::from(frames);
        self.callback = None;
        // The bump is what makes this the session's cut rather than only this
        // Tune's. The Note-Offs it stages go to the host, so without it the
        // Hub would still be holding every voice this just ended.
        self.take_cut(session::session().cut());
        self.claim();
        self.publish_delay();
    }

    /// Main thread. Give the row back so a reloaded Tune can take it. The
    /// records left in the ring carry the old epoch and the Hub refuses them,
    /// so there is nothing to drain and nobody to tell.
    pub fn retire(&mut self) {
        if let Link::Row(attached) = std::mem::replace(&mut self.link, Link::Detached) {
            session::session().detach_row(attached.slot, self.id, attached.ends);
        }
    }

    fn claim(&mut self) {
        if self.id == 0 || !matches!(self.link, Link::Detached) {
            return;
        }
        if let Some((slot, ends)) = session::session().try_attach_row(self.id) {
            self.link = Link::Row(Attached { slot, ends });
        }
    }
    fn publish_delay(&self) {
        if let Link::Row(attached) = &self.link {
            session::session().row(attached.slot).delay.store(self.delay, Ordering::Release);
        }
    }

    /// One atomic load of the Hub slot and one of the epoch. That is pairing
    /// and lifecycle in full; there is no third thing to agree with them.
    fn adopt(&mut self) {
        let session = session::session();
        self.claim();
        // The editor's Reset, which is the one setup action left. It bumps the
        // session epoch, so every paired track cuts, not just this one.
        let setup = self.shared.reset_generation();
        if setup != self.setup {
            self.setup = setup;
            session.reset();
        }
        let epoch = session.epoch();
        if epoch != self.epoch {
            self.take_cut(epoch);
        }
        let mut status = 0;
        if session.hub() == 0 {
            status |= session::NO_HUB;
        }
        if session.hubs() > 1 {
            status |= session::SECOND_HUB;
        }
        if matches!(self.link, Link::Detached) {
            status |= session::NO_ROW;
        }
        self.status =
            (self.status & (session::RING_FULL | session::CLOCK | session::DROPPED)) | status;
    }

    /// True while this Tune has somewhere to send a copy and someone to answer
    /// it. An unpaired Tune still delays and still forwards; it just never
    /// asks, so nothing it plays counts as a missed deadline.
    fn asking(&self) -> bool {
        !matches!(self.link, Link::Detached) && session::session().hub() != 0
    }

    /// The cut. Note-Off every held voice, neutralise the pedals, forget the
    /// line, adopt the new epoch. Nothing waits on anyone, and a fault never
    /// reaches here — a fault is a status, not a cut.
    fn take_cut(&mut self, epoch: u64) {
        // The queue is sized for one cut's worth: every held voice, and every
        // pedal that is down. It can still overflow, because a cut whose
        // Note-Offs did not all reach the wire leaves them queued and a second
        // cut over voices played in between arrives on top of them. What must
        // not happen is that a Note-Off goes missing in silence, so what did
        // not fit is carried past the status reset below and reported.
        let mut lost = 0;
        for cell in self.held.iter_mut() {
            if let Some(voice) = cell.take() {
                let off = Event::note_off(voice.id, voice.channel, voice.key, false);
                lost += u64::from(self.cut.push(off).is_err());
            }
        }
        for channel in 0..16u8 {
            for (bit, cc) in PEDALS.iter().enumerate() {
                if self.pedals[usize::from(channel)] & (1 << bit) != 0 {
                    let neutral = Event::Midi { port: 0, data: [0xb0 | channel, *cc, 0], flags: 0 };
                    lost += u64::from(self.cut.push(neutral).is_err());
                }
            }
            self.pedals[usize::from(channel)] = 0;
        }
        self.line.clear();
        // A fault is a status, and a status is about the context it happened
        // in. Carrying one past the cut that ended that context would report a
        // full ring against notes that were never in it.
        self.status = 0;
        self.misses = 0;
        self.epoch = epoch;
        if let Link::Direct { out, back } = &mut self.link {
            out.clear();
            back.clear();
        }
        // After the reset, because a Note-Off this cut could not queue is owed
        // by the context the cut starts rather than by the one it ended.
        if lost != 0 {
            self.dropped += lost;
            self.status |= session::DROPPED;
        }
    }

    /// The audio engine stopping, or a host reset on this instance. Neither is
    /// a transport Stop — that is the `IS_PLAYING` falling edge in
    /// [`Self::input`] — so released memory is not theirs to clear. What they
    /// owe is the cut every other trigger takes, announced so the Hub lets go
    /// of the voices this ends.
    pub fn stop(&mut self) {
        self.take_cut(session::session().cut());
    }

    pub fn begin(&mut self, callback: api::Callback) {
        self.callback = Some(callback);
        self.cursor = 0;
        self.adopt();
        if callback.steady_time < 0 {
            self.status |= session::CLOCK;
        }
    }

    /// Retain the input locally before exposing it to the Hub. A full local
    /// line refuses the event on both paths; a full copy ring costs only the
    /// correction, because the event already has its place in the line.
    pub fn input(&mut self, input: OwnedInput) {
        // `clap_performance_stop` is the audio engine stopping, not the
        // transport. A transport Stop reaches a plugin only as this flag, so
        // its falling edge is where the cut for one has to be taken.
        if let nice_plug::wrapper::clap::configuration::InputValue::Transport(transport) =
            input.value
        {
            let playing = transport.flags & IS_PLAYING != 0;
            if self.playing && !playing {
                // Every row sees the same falling edge in the same callback,
                // so exactly one of them announces it: the Hub's own. A bump
                // per Tune would make one Stop N+1 epochs, and every one of
                // them a further cut over whatever the other tracks played in
                // the callback that saw the edge. A plain Tune ends its own
                // voices here and adopts the Hub's epoch on its next callback,
                // like any other cut.
                let epoch = match self.link {
                    Link::Direct { .. } => session::session().stop(),
                    _ => self.epoch,
                };
                self.take_cut(epoch);
            }
            self.playing = playing;
            return;
        }
        let (Some(sample), Some(event)) = (input.sample, Event::from_input(input.value)) else {
            return;
        };
        // Admission may rely on a queued release freeing a cell. The output
        // boundary no longer validates anything at staging time, so the same
        // check has to happen here: a release that could never reach the wire
        // must not make room for an onset first.
        if !event.emittable() {
            self.status |= session::DROPPED;
            self.dropped += 1;
            return;
        }
        self.serial += 1;
        let serial = self.serial;
        let due = sample.saturating_add(self.delay);
        let onset = event.attack().is_some();
        if onset {
            self.notes_in += 1;
        }
        // Do not replay a full line for a note it cannot retain anyway.
        if self.line.len() == PENDING_EVENTS || onset && !self.can_hold(event) {
            self.status |= session::DROPPED;
            self.dropped += 1;
            return;
        }
        let pending =
            Pending { due, serial, event, awaiting: false, correction: None, player: 0.0 };
        if self.line.push(pending).is_err() {
            self.status |= session::DROPPED;
            self.dropped += 1;
            return;
        }
        let position = self.line.position(self.line.len() - 1).unwrap();
        let asking = self.asking();
        let copied = asking && self.copy(Capture { epoch: self.epoch, serial, sample, event });
        if asking && !copied {
            self.status |= session::RING_FULL;
            self.dropped += 1;
            if onset {
                // The Hub will never see this note, so it can neither correct
                // it nor hold it as context. It sounds raw, and says so.
                self.misses += 1;
            }
        }
        self.line.set(position, Pending { awaiting: onset && copied, ..pending });
        self.bind_initial_tuning(due, event);
    }

    /// Refuse an onset before either path sees it if its eventual wire
    /// position has no identity cell. Pending releases and channel endings
    /// free cells; retriggers replace them. This is scratch, not a second
    /// authority: it replays the actual held reducer in FIFO emission order.
    fn can_hold(&self, event: Event) -> bool {
        // Even if every pending event adds a voice, this cannot fill the set.
        // Ordinary phrases therefore do not need to replay their delay line.
        if self.held() + self.line.len() < HELD_PER_SOURCE {
            return true;
        }
        let mut held = self.held;
        for offset in 0..self.line.len() {
            let pending = self.line.at(self.line.position(offset).unwrap()).unwrap();
            Self::track_voice(&mut held, pending.event, 0);
        }
        Self::track_voice(&mut held, event, 0)
    }

    /// A per-note pitch expression at its own note's sample is the value that
    /// note starts from, not a change to it. Only the tail of the line can
    /// hold an onset at the same time, so this scan is bounded by one cohort.
    fn bind_initial_tuning(&mut self, due: i64, event: Event) {
        let Event::Expression { kind: 2, id, channel, key, value, .. } = event else {
            return;
        };
        if !value.is_finite() {
            return;
        }
        for offset in (0..self.line.len()).rev() {
            let Some(position) = self.line.position(offset) else { break };
            let Some(mut pending) = self.line.at(position) else { break };
            if pending.due != due {
                break;
            }
            let Some((voice, voice_channel, voice_key, _)) = pending.event.attack() else {
                continue;
            };
            let addressed = (id == -1 || voice == -1 || id == voice)
                && (channel == -1 || channel == i16::from(voice_channel))
                && (key == -1 || key == i16::from(voice_key));
            if addressed {
                pending.player = value;
                self.line.set(position, pending);
                return;
            }
        }
    }

    fn copy(&mut self, capture: Capture) -> bool {
        let copied = match &mut self.link {
            Link::Detached => false,
            Link::Row(attached) => attached.ends.captures.push(capture).is_ok(),
            Link::Direct { out, .. } => out.push(capture).is_ok(),
        };
        self.captured += u64::from(copied);
        copied
    }

    /// A reply reaches the onset with its serial or it reaches nothing. A
    /// wrong epoch is an answer to a session that has been cut away.
    fn drain_replies(&mut self) {
        // Serials increase along the line, because input order is what fills
        // it, and replies mostly arrive in that order too. So the scan carries
        // on from where the last one landed rather than starting at the front
        // for each reply, which is what makes a drain linear in the callback
        // rather than in replies times line length.
        //
        // Mostly, not always: onsets sharing a sample are sequenced by key, so
        // a reply can address a serial behind its predecessor's. The cursor is
        // an optimisation and never an assumption — it is dropped whenever the
        // entry under it is not one this reply could still be ahead of, which
        // covers a cursor run off the end of the line as well.
        let mut cursor = 0;
        loop {
            let reply = match &mut self.link {
                Link::Detached => None,
                Link::Row(attached) => attached.ends.replies.pop().ok(),
                Link::Direct { back, .. } => back.pop(),
            };
            let Some(reply) = reply else { break };
            if reply.epoch != self.epoch {
                continue;
            }
            if self.serial_at(cursor).is_none_or(|serial| serial > reply.serial) {
                cursor = 0;
            }
            while let Some(position) = self.line.position(cursor) {
                let Some(mut pending) = self.line.at(position) else { break };
                // An answer to a note that has already emitted is one
                // comparison rather than a walk to the end.
                if pending.serial > reply.serial {
                    break;
                }
                cursor += 1;
                if pending.serial != reply.serial {
                    continue;
                }
                if pending.awaiting {
                    pending.awaiting = false;
                    pending.correction = Some(reply.correction);
                    self.line.set(position, pending);
                }
                break;
            }
        }
    }

    fn serial_at(&self, offset: usize) -> Option<u64> {
        self.line.position(offset).and_then(|position| self.line.at(position)).map(|p| p.serial)
    }

    /// Emit everything whose time has come. An onset with a correction gets its
    /// tuning expression; one without emits uncorrected and counts the miss.
    pub fn schedule(&mut self, block: api::Block, output: &mut api::Output<'_>) {
        self.drain_replies();
        let Some(callback) = self.callback else { return };
        let base = callback.steady_time;
        let end =
            base.saturating_add(i64::from(block.start)).saturating_add(i64::from(block.frames));
        while let Some(event) = self.cut.front() {
            let time = self.cursor.max(block.start).min(callback.frames.saturating_sub(1));
            if !self.emit(output, time, event, None, 0) {
                return;
            }
            self.cut.pop();
        }
        while let Some(pending) = self.line.front() {
            if pending.due >= end {
                break;
            }
            let offset = pending.due.saturating_sub(base).clamp(0, i64::from(u32::MAX)) as u32;
            let time = offset.max(self.cursor).max(block.start);
            if time >= callback.frames {
                break;
            }
            let (event, tuning, correction) = self.resolve(pending);
            if !self.emit(output, time, event, tuning, correction) {
                break;
            }
            // Counted where the note actually leaves, not where it is
            // resolved: a staging failure re-resolves the same entry next
            // callback, and one note must not count as two misses.
            if pending.awaiting && pending.correction.is_none() {
                self.misses += 1;
            }
            self.line.pop();
        }
    }

    /// What actually goes on the wire for one retained input: the frozen
    /// correction composed into a later per-note expression, and the initial
    /// expression that states it for a new voice.
    fn resolve(&self, pending: Pending) -> (Event, Option<Event>, i64) {
        if let Some((id, channel, key, _)) = pending.event.attack() {
            let tuning = pending.correction.map(|correction| Event::Expression {
                kind: 2,
                id,
                port: 0,
                channel: i16::from(channel),
                key: i16::from(key),
                value: pending.player + correction as f64 / 100_000_000.0,
                flags: 0,
            });
            return (pending.event, tuning, pending.correction.unwrap_or(0));
        }
        // The same addressing rule the Hub applies to the same event, so the
        // pitch this composes and the pitch the Hub draws are one voice's.
        let correction = self
            .held
            .iter()
            .flatten()
            .find(|voice| pending.event.matches(voice.id, voice.channel, voice.key))
            .map_or(0, |voice| voice.correction);
        let mut event = pending.event;
        if let Event::Expression { kind: 2, value, .. } = &mut event {
            *value += correction as f64 / 100_000_000.0;
        }
        (event, None, 0)
    }

    fn emit(
        &mut self,
        output: &mut api::Output<'_>,
        time: u32,
        event: Event,
        tuning: Option<Event>,
        correction: i64,
    ) -> bool {
        if !event.emittable() {
            self.dropped += 1;
            self.status |= session::DROPPED;
            return true;
        }
        // A refused note is the host's whole answer: nothing left, nothing is
        // tracked, and `schedule` keeps the line entry for a later callback.
        if !output.push(event.input(), time) {
            return false;
        }
        self.cursor = time;
        // The tuning expression addresses the voice the note just created, so
        // it can only follow the note, and a refusal here costs the correction
        // rather than the note. Losing both would be the one thing this design
        // exists to stop, and re-emitting the note to recover the pair would
        // sound it twice.
        if let Some(tuning) = tuning {
            if !tuning.emittable() || !output.push(tuning.input(), time) {
                self.status |= session::DROPPED;
            }
        }
        self.track(event, correction);
        true
    }

    /// The held set and the pedal state, which is the whole of what the cut
    /// needs. It follows what was actually emitted, never what was planned.
    fn track(&mut self, event: Event, correction: i64) {
        let retained = Self::track_voice(&mut self.held, event, correction);
        debug_assert!(retained, "onsets reserve held capacity before admission");
        if event.attack().is_some() {
            self.notes_out += 1;
        }
        if let Event::Midi { port: 0, data, .. } = event {
            let channel = data[0] & 15;
            if data[0] & 0xf0 == 0xb0 {
                if let Some(bit) = PEDALS.iter().position(|cc| *cc == data[1]) {
                    let mask = 1 << bit;
                    if data[2] >= 64 {
                        self.pedals[usize::from(channel)] |= mask;
                    } else {
                        self.pedals[usize::from(channel)] &= !mask;
                    }
                }
            }
        }
    }

    /// The identity/correction reducer shared by emission and admission.
    /// False means an attack cannot be retained; no other event needs a cell.
    fn track_voice(
        held: &mut [Option<Voice>; HELD_PER_SOURCE],
        event: Event,
        correction: i64,
    ) -> bool {
        if let Some((id, channel, key, _)) = event.attack() {
            let cell = held
                .iter()
                .position(|held| held.is_some_and(|v| v.channel == channel && v.key == key))
                .or_else(|| held.iter().position(Option::is_none));
            if let Some(cell) = cell {
                held[cell] = Some(Voice { id, channel, key, correction });
            } else {
                return false;
            }
            return true;
        }
        if event.release() {
            if let Some(cell) = held.iter_mut().find(|cell| {
                cell.is_some_and(|voice| event.matches(voice.id, voice.channel, voice.key))
            }) {
                *cell = None;
            }
        }
        if let Event::Midi { port: 0, data, .. } = event {
            let channel = data[0] & 15;
            if event.channel_termination().is_some() {
                for cell in held.iter_mut() {
                    if cell.is_some_and(|voice| voice.channel == channel) {
                        *cell = None;
                    }
                }
            }
        }
        true
    }

    pub fn end(&mut self) {
        if let Some(callback) = self.callback.take() {
            self.shared.status.store(self.status, Ordering::Release);
            self.shared.misses.store(self.misses, Ordering::Release);
            if self.shared.diagnostics.due(callback.frames, self.rate) {
                self.shared.diagnostics.publish_tune(super::diagnostics::TuneReport {
                    epoch: self.epoch,
                    slot: self.slot().map_or(0, |slot| u64::from(slot) + 1),
                    delay: self.delay,
                    notes_in: self.notes_in,
                    notes_out: self.notes_out,
                    captured: self.captured,
                    misses: self.misses,
                    dropped: self.dropped,
                    held: self.held() as u64,
                    pending: self.line.len() as u64,
                    status: self.status,
                });
                self.shared.request_main();
            }
        }
    }

    /// The Hub's half of the direct link.
    pub fn take_direct(&mut self) -> Option<Capture> {
        match &mut self.link {
            Link::Direct { out, .. } => out.pop(),
            _ => None,
        }
    }
    pub fn reply_direct(&mut self, reply: Reply) {
        if let Link::Direct { back, .. } = &mut self.link {
            let _ = back.push(reply);
        }
    }
    pub fn epoch(&self) -> u64 {
        self.epoch
    }
}
