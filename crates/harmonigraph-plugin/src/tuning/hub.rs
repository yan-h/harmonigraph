//! The Hub: one ordered pass over every source's copied input, one policy
//! decision per onset, one reply back.
//!
//! It runs to completion in the callback it starts in. There is no coverage
//! watermark and no complete-interval wait: a record that arrives after its own
//! sample was sequenced is assigned when it arrives, and the Tune it belongs to
//! has already decided what to do without an answer. The Hub also tunes its own
//! track, through the same [`Tune`] every other row runs, so there is no second
//! observation path to keep in step with this one.
use std::sync::atomic::Ordering;
use std::sync::Arc;

use harmonigraph_clap::events::{Input, InputValue, Output, Transport};
use harmonigraph_clap::plugin::Boundary;
use harmonigraph_core::canonical::{NoteDelta, VoiceBaseline};
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::lattice_map::{Follow, LatticeMap, TuningEngine, FOLLOW_GROUP};
use harmonigraph_core::tuning::{fifths_for_midi, fifths_offset};
use harmonigraph_core::{policy, LatticePos, SourceId};
use harmonigraph_record::{publication, Recorder};

use crate::presentation::Mapping;

use super::event::Event;
use super::session::{self, Copied, HubEnds, Reply};
use super::state::{Stamp, State};
use super::tune::Tune;
use super::{setup, BATCH_EVENTS, DIRECT, HELD_PER_SOURCE, TUNERS};

type Owner = crate::configuration::Owner;

/// One observation of the host's seconds timeline: where the song was, at
/// which steady sample.
#[derive(Clone, Copy)]
struct Seconds {
    song: f64,
    sample: i64,
}

/// The identity one row's notes carry downstream. The Hub's own track keeps
/// the reserved DIRECT id every consumer already knows it by; a Tune row is
/// one past its own index, so no paired track can collide with it.
fn identity(source: u8) -> SourceId {
    if source == DIRECT {
        SourceId::DIRECT
    } else {
        SourceId(u64::from(source) + 1)
    }
}

/// One copied record staged for the ordering pass, with the row it came from.
#[derive(Clone, Copy)]
struct Record {
    retune: u64,
    source: u8,
    sample: i64,
    serial: u64,
    epoch: u64,
    event: Event,
    player: f64,
}

impl Record {
    fn onset(&self) -> bool {
        self.event.attack().is_some()
    }
    /// Merge order inside one sample: every release and controller from every
    /// source applies before any onset, and onsets then run in the musical
    /// key/channel/source tie-break. Pressure, gain and timbre come LAST: they
    /// decide nothing about pitch, and a host states a note's starting value
    /// at its onset sample, which applied ahead of the onset addresses a voice
    /// that does not exist yet. Ties inside each rank fall back to the
    /// per-source input order, which copying preserves.
    fn order(&self) -> (u8, u8, u8, u8, u64) {
        match self.event.attack() {
            Some((_, channel, key, _)) => (1, key, channel, self.source, self.serial),
            None if self.event.expression().is_some() => (2, 0, 0, self.source, self.serial),
            None => (0, 0, 0, self.source, self.serial),
        }
    }
}

/// What one onset's decision produced, on its way to the source's state.
#[derive(Clone, Copy)]
struct Assigned {
    correction: i64,
    node: Option<LatticePos>,
    decision: u64,
    player: f64,
    channel_pitch: i64,
}

/// One scheduled delta, held from the input boundary, where the replies have
/// to be minted, to this callback's first publication in `process`, which is
/// where the recorder is in hand.
#[derive(Clone, Copy)]
struct Published {
    source: u8,
    delta: NoteDelta,
}

/// A note the Hub believes is sounding, as far as the policy is concerned.
#[derive(Clone, Copy)]
struct Voice {
    source: u8,
    lifetime: u64,
    /// The tuned onset pitch. Later expression moves what sounds and never
    /// this, which is what "the adaptive choice is frozen" means.
    onset_pitch: i64,
    node: Option<LatticePos>,
    decision: u64,
    /// The input sample its decay counts from: the one it was struck on, or for
    /// a repeat of the node struck last, that strike's (`Memory::attack`).
    onset: i64,
}
impl Voice {
    fn context_pitch(self) -> policy::ContextPitch {
        policy::ContextPitch { pitch: self.onset_pitch, node: self.node, weight: 1.0 }
    }
}

/// The musical half, and nothing else. Everything in here is policy state; the
/// transport around it is what #786 replaced.
struct Sequencer {
    engine: TuningEngine,
    engine_revision: u64,
    map: Option<LatticeMap>,
    /// Lattice Map's Position, every engine's: under Adaptive its fifths
    /// place the strip a decoded onset is tuned in.
    position: LatticePos,
    context: Box<[Option<Voice>]>,
    memory: policy::Memory,
    config: policy::MusicalConfig,
    scratch: Box<policy::PolicyScratch>,
    working: Vec<policy::ContextPitch>,
    last_release: Option<i64>,
    decision: u64,
    /// A loop or seek was detected and the next attack clears memory, if the
    /// control says so. Held notes keep their frozen assignments either way.
    loop_pending: bool,
    reference_source: Option<u8>,
    /// Lattice Map following: the mode the current attack group runs under,
    /// how far the Hub has moved the map beyond its automated place, and the
    /// input sample of the attack group that offset was last decided for.
    follow: Follow,
    followed: LatticePos,
    /// Adaptive's strip following the spellings in use (`Follow spellings`).
    follower: policy::strip::Follower,
    /// This callback's ordering pass, and the attack group (its sample, in a
    /// pass) last scanned: a group is scanned once in each pass it has
    /// onsets in, so a Tune's onset drained a pass after its sample's first
    /// is still seen.
    pass: u64,
    scanned: Option<(i64, u64)>,
    /// The attack sample whose following was last decided, map's or strip's:
    /// one decision per attack group.
    decided: Option<i64>,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self {
            engine: Default::default(),
            engine_revision: 0,
            map: None,
            position: LatticePos::ORIGIN,
            context: vec![None; policy::MAX_CONTEXT].into_boxed_slice(),
            memory: policy::Memory::default(),
            config: harmonigraph_core::configuration::ConfigReducer::default().resolved().into(),
            scratch: Box::default(),
            working: Vec::with_capacity(policy::MAX_CONTEXT + policy::MAX_MEMORY),
            last_release: None,
            decision: 0,
            loop_pending: false,
            reference_source: None,
            follow: Follow::Off,
            followed: LatticePos::ORIGIN,
            follower: policy::strip::Follower::default(),
            pass: 0,
            scanned: None,
            decided: None,
        }
    }
}

impl Sequencer {
    /// Released memory and everything the moving context carries forward: the
    /// Adaptive memory, the map's follow offset and the strip's alike. Held
    /// voices stay.
    fn forget_history(&mut self) {
        self.memory.clear();
        self.last_release = None;
        self.followed = LatticePos::ORIGIN;
        self.follower.clear();
    }
    /// The next attack after a loop or seek clears the history, if the control
    /// says so. Held notes keep their frozen assignments either way.
    fn take_loop(&mut self) {
        if self.loop_pending {
            if self.config.policy.reset_loop {
                self.forget_history();
            }
            self.loop_pending = false;
        }
    }
    /// A newly assigned voice joins the context, or `false` when the context
    /// has no cell left for it.
    fn hold(&mut self, voice: Voice) -> bool {
        let cell = self.context.iter_mut().find(|cell| cell.is_none());
        cell.map(|cell| *cell = Some(voice)).is_some()
    }
    /// Participation ends without releasing anything onto the wire or into
    /// musical memory. The moving reference must not retain this source either.
    fn forget_source(&mut self, source: u8) {
        for cell in self.context.iter_mut() {
            if cell.is_some_and(|voice| voice.source == source) {
                *cell = None;
            }
        }
        self.memory.forget_source(source);
        if self.reference_source == Some(source) {
            self.memory.reference = 0;
            self.reference_source = None;
        }
    }
    /// The context one assignment sees (`policy::context`), into `working`.
    fn fill(&mut self, rate: f64) {
        let mut held = [policy::Held::default(); policy::MAX_CONTEXT];
        let mut count = 0;
        for voice in self.context.iter().flatten() {
            held[count] = policy::Held {
                pitch: voice.context_pitch(),
                decision: voice.decision,
                struck: voice.onset,
            };
            count += 1;
        }
        policy::context(
            &mut held[..count],
            &self.memory,
            self.config.policy,
            rate,
            &mut self.working,
        );
    }
    /// Drop a context voice without contributing it to released memory. Two
    /// callers, and neither is a note ending: an onset the scheduled state
    /// could not retain never sounded at all, and a voice a same-key onset
    /// displaces is taken over rather than released. Both leave a lifetime no
    /// release will ever address, and neither leaves one for `Memory` to
    /// remember — which is the whole of the difference from `release_voice`.
    fn forget_voice(&mut self, source: u8, lifetime: u64) {
        if let Some(cell) = self
            .context
            .iter_mut()
            .find(|cell| cell.is_some_and(|v| v.source == source && v.lifetime == lifetime))
        {
            *cell = None;
        }
    }
    fn release_voice(&mut self, source: u8, lifetime: u64, sample: i64) {
        if let Some(cell) = self
            .context
            .iter_mut()
            .find(|cell| cell.is_some_and(|v| v.source == source && v.lifetime == lifetime))
        {
            let voice = cell.take().unwrap();
            self.memory.release(voice.context_pitch(), source, self.config.policy, voice.onset);
            self.last_release = Some(sample);
        }
    }
    /// Every voice of one source at once: a cut, a Stop, or a Tune leaving.
    fn release_source(&mut self, source: u8, sample: i64) {
        for cell in self.context.iter_mut() {
            if cell.is_some_and(|v| v.source == source) {
                let voice = cell.take().unwrap();
                self.memory.release(voice.context_pitch(), source, self.config.policy, voice.onset);
                self.last_release = Some(sample);
            }
        }
    }
    fn expire(&mut self, sample: i64, rate: f64) {
        let timeout = self.config.policy.silence_ms;
        if timeout != 0
            && self.context.iter().all(Option::is_none)
            && self.last_release.is_some_and(|t| {
                sample.saturating_sub(t) as f64 >= f64::from(timeout) * rate / 1000.0
            })
        {
            self.forget_history();
        }
    }
}

#[derive(Default)]
struct Row {
    retune: u64,
    show: bool,
    /// A Tune holds this row. Read from the row's own owner each callback.
    live: bool,
    state: State,
    sequence: u64,
    applied: u64,
    /// Includes deltas lost before publication; a repair cannot predate them.
    applied_time: Option<f64>,
    delay: i64,
    repair: publication::Lanes<bool>,
}

pub struct Hub {
    pub shared: Arc<setup::Shared>,
    /// The Hub's own track. It is a Tune, at zero delay, so it is sequenced as
    /// a seventeenth source rather than observed as a special case. Its ring
    /// is a direct call.
    pub tune: Tune,
    id: u64,
    ends: Option<Box<[Option<HubEnds>; TUNERS]>>,
    epoch: u64,
    rows: Box<[Row]>,
    batch: Vec<Record>,
    /// Onsets that left their Tune without a correction, `(source, serial)`,
    /// withdrawn after this callback's batch is applied.
    missed: Vec<(u8, u64)>,
    pending: Vec<Published>,
    /// The earliest time among deltas lost before reaching either
    /// publication lane.
    pending_loss: Option<f64>,
    sequencer: Box<Sequencer>,
    rate: f64,
    callback: Option<Boundary>,
    /// This callback's mapping from steady samples to presentation time,
    /// which the plugin's one clock owns (`presentation`).
    mapping: Mapping,
    /// The last observation of the host's seconds timeline, for loop and
    /// seek detection.
    seconds: Option<Seconds>,
    /// The last recording run every row was told to open with a take-lane
    /// snapshot. See [`Hub::publish`].
    opened: Option<u64>,
    status: u32,
    decisions: u64,
    published: u64,
}

impl Hub {
    pub fn new() -> Box<Self> {
        let shared = setup::Shared::hub();
        Box::new(Self {
            tune: Tune::direct(shared.clone()),
            shared,
            id: 0,
            ends: None,
            epoch: 0,
            rows: (0..=TUNERS).map(|_| Row::default()).collect::<Vec<_>>().into_boxed_slice(),
            batch: Vec::with_capacity(BATCH_EVENTS),
            missed: Vec::with_capacity(BATCH_EVENTS),
            pending: Vec::with_capacity(BATCH_EVENTS),
            pending_loss: None,
            sequencer: Box::default(),
            rate: 0.0,
            callback: None,
            mapping: Mapping::default(),
            seconds: None,
            opened: None,
            status: 0,
            decisions: 0,
            published: 0,
        })
    }

    /// Main thread. The first Harmonigraph in the process owns the session; a
    /// second one gets no rings and both say so. There is no choice to offer,
    /// so there is nothing here that can be ambiguous.
    pub fn register(&mut self) {
        if self.id != 0 {
            return;
        }
        self.id = session::session().register();
        self.tune.register();
        self.ends = session::session().attach_hub(self.id);
    }
    /// Main thread. Giving the rings back is what lets a reloaded Harmonigraph
    /// take the session over without a handoff protocol.
    pub fn retire(&mut self) {
        self.tune.retire();
        session::session().detach_hub(self.id, self.ends.take());
    }

    pub fn activate(&mut self, rate: f64, frames: u32) {
        self.rate = rate;
        self.seconds = None;
        // No delay at all: the Hub's own input is already at the Hub, and its
        // correction is queued at the input boundary before the same
        // callback is scheduled, so the round trip needs none.
        self.tune.activate(rate, frames, 0);
        self.shared.publish_format(rate, frames);
    }

    pub fn begin(&mut self, callback: &Boundary, owner: &mut Owner, mapping: Mapping) {
        self.callback = Some(*callback);
        self.mapping = mapping;
        // A run states where it opens on the clock every delta is timed on.
        owner.recording.presentation = mapping;
        // Reset all voices is the Hub's own: a cut, which this callback's own
        // row and every Tune adopt like any other, plus forgetting the history
        // once the cut has released its voices into it.
        let forget = session::session().take_reset();
        if forget {
            session::session().cut();
        }
        self.tune.begin(callback);
        self.adopt();
        if forget {
            self.sequencer.forget_history();
        }
        self.detect_loop(callback);
        // Adopt even on callbacks with no note input, so silence expiry below
        // uses the policy rather than the last onset's configuration. It is
        // the policy the previous callback left: the configuration owner reads
        // the document's latest configuration after `begin`, so an edit
        // governs expiry from the callback after it arrives.
        self.sequencer.config = owner.current().into();
        // Silence expires released memory against the completed input
        // frontier, which is this callback's start: everything before it has
        // been sequenced, and nothing after it has arrived.
        self.sequencer.expire(callback.steady_time, self.rate);
    }

    /// An epoch change ends every voice.
    fn adopt(&mut self) {
        let session = session::session();
        let sample = self.callback.map_or(0, |callback| callback.steady_time);
        let epoch = session.epoch();
        if epoch != self.epoch {
            for source in 0..=TUNERS {
                self.sequencer.release_source(source as u8, sample);
                self.rows[source].state.clear();
                self.rows[source].repair = publication::Lanes::both(true);
            }
            if session.is_stop(self.epoch) && self.sequencer.config.policy.reset_stop {
                self.sequencer.forget_history();
            }
            self.batch.clear();
            // Deltas scheduled before the cut describe notes it has just
            // ended. Every row is already owed a snapshot, which is what tells
            // the display what is actually sounding now.
            self.pending.clear();
            self.pending_loss = None;
            self.status = 0;
            self.epoch = epoch;
        }
        for slot in 0..TUNERS {
            let row = session.row(slot as u8);
            // A second Harmonigraph holds no rings, so a row a Tune holds is
            // not one it sequences. Calling it live would publish an empty
            // snapshot for a source this instance has never seen.
            let live = self.ends.is_some() && row.held();
            if !live && self.rows[slot].live {
                self.sequencer.release_source(slot as u8, sample);
                self.rows[slot].state.clear();
            }
            self.rows[slot].live = live;
            self.rows[slot].delay = row.delay.load(Ordering::Acquire);
        }
        self.rows[usize::from(DIRECT)].live = true;
        self.rows[usize::from(DIRECT)].delay = self.tune.delay();
        for source in 0..=TUNERS {
            let (retune, show) = if source == usize::from(DIRECT) {
                (self.shared.retuning(), self.shared.show.load(Ordering::Acquire))
            } else {
                let row = session.row(source as u8);
                (row.retune.load(Ordering::Acquire), row.show.load(Ordering::Acquire))
            };
            if self.rows[source].retune != retune {
                self.sequencer.forget_source(source as u8);
                self.rows[source].retune = retune;
            }
            if self.rows[source].show != show {
                self.rows[source].show = show;
                self.rows[source].repair = publication::Lanes::both(true);
            }
        }
        // Sticky until the cut. A policy refusal or a lost report inside one
        // callback would otherwise be gone before the once-a-second summary
        // that is meant to show it.
        let mut status = self.status & (session::PUBLICATION | session::POLICY | session::DROPPED);
        if session.hubs() > 1 {
            status |= session::SECOND_HUB;
        }
        if session.hub() != self.id {
            status |= session::NO_HUB;
        }
        self.status = status;
    }

    /// A loop or a seek is a song position that neither held still nor
    /// advanced with the samples since the last observation, by more than two
    /// milliseconds either way. That one rule covers a playing transport, a
    /// stopped one a host holds still, a stop that returns to where play began,
    /// and an export that reports not playing while its position climbs. What
    /// a jump does then is the `reset_loop` control's business, at the next
    /// attack.
    fn detect_loop(&mut self, callback: &Boundary) {
        if let Some(transport) = callback.transport {
            self.observe_seconds(&transport, callback.steady_time);
        }
    }

    /// One observation of the host's seconds timeline against elapsed samples.
    fn observe_seconds(&mut self, transport: &Transport, sample: i64) {
        let (Some(song), true) = (transport.seconds(), self.rate > 0.0) else {
            return;
        };
        if let Some(previous) = self.seconds {
            let moved = song - previous.song;
            let elapsed = (sample - previous.sample) as f64 / self.rate;
            if moved.abs().min((moved - elapsed).abs()) > 0.002 {
                self.sequencer.loop_pending = true;
            }
        }
        self.seconds = Some(Seconds { song, sample });
    }

    pub fn input(&mut self, input: Input) {
        // A host may supply the transport as the callback's own value, as an
        // input event, or as both. The seek this is looking for has to be
        // found in whichever one arrives.
        if let InputValue::Transport(transport) = input.value {
            self.observe_seconds(&transport, input.sample);
        }
        self.tune.input(input);
    }
    pub fn schedule(&mut self, output: &mut Output<'_>) {
        self.tune.schedule(output);
    }

    /// The audio engine stopping, or a host reset. Both are a session cut,
    /// which is the same mechanism a transport Stop takes — but neither IS a
    /// transport Stop, so released memory stays. One trigger, one cut: the
    /// Hub's own row takes it and every other row adopts its epoch.
    pub fn stop(&mut self) {
        self.tune.stop();
    }

    /// THE ordering pass. Drain every row, sort by sample, apply releases and
    /// controllers, assign onsets, reply. It runs to completion here.
    pub fn input_boundary(&mut self, owner: &mut Owner) {
        // Every callback, so a silence or Stop reset reaches the editor too;
        // what this pass decides is drawn from the next one. A mode change not
        // yet reached by an attack has already reset following in effect.
        let current = owner.maps.engine_revision() == self.sequencer.engine_revision;
        owner.maps.publish_followed(if current {
            self.sequencer.followed
        } else {
            LatticePos::ORIGIN
        });
        owner.maps.publish_strip(if current { self.sequencer.follower.offset() } else { 0 });
        self.sequencer.pass += 1;
        self.collect();
        self.batch.sort_unstable_by_key(|record| (record.sample, record.order()));
        let mut index = 0;
        while index < self.batch.len() {
            let sample = self.batch[index].sample;
            let mut end = index;
            while end < self.batch.len() && self.batch[end].sample == sample {
                end += 1;
            }
            // The state in force at the group's own sample, or, where the
            // history does not reach it, the live one: a sample past this
            // callback's end, whose automation nobody has seen yet, or one
            // older than what the history keeps or a host reset left of it (a
            // song-time locate clears nothing). Every engine then assigns from
            // what it holds now, as #920 asked of Adaptive, rather than
            // leaving the onset raw.
            let attack = owner.in_force(sample);
            if self.sequencer.engine_revision != attack.engine_revision {
                self.sequencer.forget_history();
                self.sequencer.context.fill(None);
                self.sequencer.engine = attack.playback.engine;
                self.sequencer.engine_revision = attack.engine_revision;
            }
            self.sequencer.map = attack.playback.map;
            self.sequencer.position = attack.playback.offset;
            self.sequencer.follow = attack.playback.follow;
            self.sequencer.config = attack.config.into();
            for position in index..end {
                self.apply(position, attack.config);
            }
            index = end;
        }
        self.batch.clear();
        self.withdraw_misses();
    }

    /// Each onset that sounded without the correction this Hub chose for it
    /// is drawn from here on at the raw pitch it sounds at: the voice loses
    /// its correction, and a `Tuning` delta states the raw pitch at the row's
    /// latest scheduled time, as a channel move does (#1151). The usual miss
    /// is collected in the same pass as its onset, since the Tune's whole
    /// callback ran before this one, so the note is drawn corrected only up
    /// to the row's latest scheduled time in the pass. One collected a pass
    /// later, after a race or a lost reply, stays corrected for up to about a
    /// delay plus one callback. After the batch, so an onset collected in the
    /// same pass is held by now; a voice already released, or replaced on its
    /// key, is left alone.
    ///
    /// Never before this callback's start: a snapshot an earlier callback
    /// published for the row, timed later in its callback, covers the voice
    /// with its correction, and a withdrawal timed before it would sort ahead
    /// of it on read.
    fn withdraw_misses(&mut self) {
        let start = self.callback.map(|callback| self.presentation(callback.steady_time));
        for position in 0..self.missed.len() {
            let (source, lifetime) = self.missed[position];
            let row = &mut self.rows[usize::from(source)];
            let Some(applied) = row.applied_time else { continue };
            let time = start.map_or(applied, |start| applied.max(start));
            let sequence = row.sequence + 1;
            let stamp = Stamp { source: identity(source), sequence, lifetime, time };
            if let Some(delta) = row.state.sounded_raw(lifetime, stamp) {
                row.sequence = sequence;
                row.applied = sequence;
                self.hold(source, delta);
            }
        }
        self.missed.clear();
    }

    /// Take what each row has copied over. A record from an old epoch belongs
    /// to a session that has been cut away and is dropped where it is found.
    fn collect(&mut self) {
        let direct_epoch = self.tune.epoch();
        // A full batch stops the drain rather than emptying it: what stays in
        // the ring is sequenced next callback, and a ring that then fills is
        // the Tune's own RING_FULL rather than a record lost without a word.
        while self.batch.len() < BATCH_EVENTS {
            let Some(capture) = self.tune.take_direct() else { break };
            if capture.epoch != direct_epoch {
                continue;
            }
            match capture.copied {
                Copied::Input { retune, sample, event } => Self::retain(
                    &mut self.batch,
                    Record {
                        retune,
                        source: DIRECT,
                        sample,
                        serial: capture.serial,
                        epoch: capture.epoch,
                        event,
                        player: 0.0,
                    },
                ),
                Copied::Miss => Self::miss(&mut self.missed, &mut self.status, DIRECT, capture),
            }
        }
        let epoch = self.epoch;
        let Some(ends) = self.ends.as_mut() else { return };
        for (slot, end) in ends.iter_mut().enumerate() {
            let Some(end) = end.as_mut() else { continue };
            // Discarded epochs do not fill the batch. Bound visits by what is
            // here now so another instance cannot keep this drain running by
            // replenishing rejected captures. New arrivals wait one callback.
            let available = end.captures.slots();
            for _ in 0..available {
                if self.batch.len() == BATCH_EVENTS {
                    break;
                }
                let Ok(capture) = end.captures.pop() else { break };
                if capture.epoch != epoch {
                    continue;
                }
                match capture.copied {
                    Copied::Input { retune, sample, event } => Self::retain(
                        &mut self.batch,
                        Record {
                            retune,
                            source: slot as u8,
                            sample,
                            serial: capture.serial,
                            epoch: capture.epoch,
                            event,
                            player: 0.0,
                        },
                    ),
                    Copied::Miss => {
                        Self::miss(&mut self.missed, &mut self.status, slot as u8, capture)
                    }
                }
            }
        }
    }

    /// Keep a miss for after the batch. It never ranks as an onset or binds an
    /// expression; more than one pass's worth is dropped.
    fn miss(missed: &mut Vec<(u8, u64)>, status: &mut u32, source: u8, capture: session::Capture) {
        if missed.len() < BATCH_EVENTS {
            missed.push((source, capture.serial));
        } else {
            *status |= session::DROPPED;
        }
    }

    /// Bind in each source's input order, before the musical sort moves
    /// expressions ahead of onsets. The derived value follows its onset serial.
    fn retain(batch: &mut Vec<Record>, record: Record) {
        if let Some(mut tuning) = record.event.initial_tuning() {
            for other in batch.iter_mut().rev().take_while(|other| {
                other.source == record.source
                    && other.sample == record.sample
                    && other.epoch == record.epoch
            }) {
                if let Some(value) = tuning.bind(other.event) {
                    other.player = value;
                }
            }
        }
        batch.push(record);
    }

    /// One record, in order: its effect on the policy's context, its decision
    /// if it is an onset, and its place in the schedule the display and the
    /// take draw.
    fn apply(&mut self, position: usize, config: ResolvedConfig) {
        let record = self.batch[position];
        let source = usize::from(record.source);
        let scheduled = record.sample.saturating_add(self.rows[source].delay);
        if let Some((_, channel, key, _)) = record.event.attack() {
            // A missed release copy can leave the Hub fuller than the Tune.
            // Refuse before assignment mutates policy or sends a reply: this
            // locally admitted voice still sounds raw and is tracked by Tune.
            if self.rows[source].state.onset_cell(channel, key).is_none() {
                self.status |= session::DROPPED;
                return;
            }
        }
        let assignment = record.onset().then(|| self.assign(position, config)).flatten();
        // A channel termination is one controller that ends every voice on its
        // channel. The instrument performs those endings, so the schedule owes
        // them as note-offs rather than as one opaque controller.
        if record.event.channel_termination().is_some() {
            let channel = record.event.channel().unwrap_or(0);
            let mut ended = [(0i32, 0u8, 0u8); HELD_PER_SOURCE];
            let count = self.rows[source].state.on_channel(channel, &mut ended);
            for (id, channel, key) in ended.into_iter().take(count) {
                self.release_addressed(record, channel, key);
                let off = Event::note_off(id, channel, key, false);
                self.schedule_delta(off, record, scheduled, None);
            }
        } else if !record.onset() && record.event.release() {
            self.release_matching(record);
        }
        self.schedule_delta(record.event, record, scheduled, assignment);
    }

    /// The lifetime one source's state holds for a `(channel, key)`: the voice
    /// a termination ends, and the one a same-key onset takes over.
    fn addressed_lifetime(&self, source: u8, channel: u8, key: u8) -> Option<u64> {
        self.rows[usize::from(source)]
            .state
            .voices()
            .find(|voice| voice.channel == channel && voice.note == key)
            .map(|voice| voice.lifetime)
    }
    fn release_addressed(&mut self, record: Record, channel: u8, key: u8) {
        if let Some(lifetime) = self.addressed_lifetime(record.source, channel, key) {
            self.sequencer.release_voice(record.source, lifetime, record.sample);
        }
    }
    /// Let go of whatever an onset is about to displace, before it is scored.
    fn forget_replaced(&mut self, source: u8, channel: u8, key: u8) {
        if let Some(lifetime) = self.addressed_lifetime(source, channel, key) {
            self.sequencer.forget_voice(source, lifetime);
        }
    }
    fn release_matching(&mut self, record: Record) {
        let lifetime = self.rows[usize::from(record.source)]
            .state
            .voices()
            .find(|voice| record.event.matches(voice.host_note_id, voice.channel, voice.note))
            .map(|voice| voice.lifetime);
        if let Some(lifetime) = lifetime {
            self.sequencer.release_voice(record.source, lifetime, record.sample);
        }
    }

    /// One onset, one decision. Everything the policy sees was folded in by an
    /// earlier record in this same pass, which is what makes a chord spread
    /// across three tracks one chord rather than three independent guesses.
    fn assign(&mut self, position: usize, config: ResolvedConfig) -> Option<Assigned> {
        let record = self.batch[position];
        let (_, channel, key, _) = record.event.attack()?;
        let source = usize::from(record.source);
        // A same-key onset takes over the cell admission already found for
        // that identity, so no release will ever address the lifetime it
        // displaces: it leaves policy context here or never.
        //
        // Forgotten rather than released, and those are NOT the same context.
        // A real note-off feeds `Memory`, and with nothing else held `fill`
        // scores from that memory, so an off-then-on IS scored with its
        // predecessor's pitch present where a replacement is not.
        // What that difference is worth was measured rather than assumed: over
        // a harmony change, a clip-loop chain of repeats, a lone repeated note
        // and a note held until a drifted context respells it, the entry never
        // moved the chosen spelling. With anything held it is not in the
        // context at all, and when the retrigger keeps its predecessor's
        // spelling `Memory::attack` deletes it again on that same decision.
        // Forgetting is still the answer to want: nothing ended, so released
        // memory owes this no entry.
        //
        // Before the score rather than after, because after `fill` the
        // displaced voice would still be there at HELD weight — a second
        // full-weight copy of the very pitch being scored.
        self.forget_replaced(record.source, channel, key);
        self.scan_group(position, config);
        let player = record.player;
        let channel_pitch = self.rows[source].state.channel_pitch(channel);
        // Captures remain visible with Retune off. Only a still-current,
        // enabled arrival may make a decision or enter the musical context.
        if !self.current(&record) {
            if record.retune & 1 != 0 {
                self.reply(record, 0);
            }
            return Some(Assigned {
                correction: 0,
                node: None,
                decision: 0,
                player,
                channel_pitch,
            });
        }
        let incoming = self.arriving(record, channel, key);
        // With Decode fifths on, every source with Retune on declares its
        // tuning: a chain of fifths at the decoded fifth, its C at the
        // lattice's. A map never reads it, since both its slot and its follow
        // are chosen by the key.
        let decoded = config.policy.decode_fifths;
        let stage = decoded.then(|| {
            let fifths = fifths_for_midi(channel, key);
            fifths_offset(fifths, config.tuning.c_offset, config.policy.decoded_fifth)
        });
        // Adaptive tunes a decoded onset by the strip, a node chosen by its
        // spelling alone (`policy::strip`), and searches only the rest.
        let adaptive = self.sequencer.engine == TuningEngine::Adaptive;
        if !adaptive || stage.is_some() {
            let assigned = if adaptive {
                let fifths = fifths_for_midi(channel, key);
                let flat = self.strip_place(position, config);
                let stage = stage.unwrap_or(0);
                let (node, correction) =
                    policy::strip::assignment(fifths, flat, incoming + stage, config.tuning);
                Some((node, stage + correction))
            } else {
                (self.sequencer.engine == TuningEngine::LatticeMap)
                    .then_some(self.sequencer.map)
                    .flatten()
                    .map(|map| self.follow(position, map).assignment(incoming, config.tuning))
            };
            // With no node to assign, a decoded onset sounds as declared.
            let correction = assigned.map_or(stage.unwrap_or(0), |(_, correction)| correction);
            let node = assigned.map(|(node, _)| node);
            let decision = if assigned.is_some() || stage.is_some() {
                self.sequencer.decision += 1;
                self.decisions += 1;
                self.sequencer.decision
            } else {
                0
            };
            self.reply(record, correction);
            // A mapped or strip voice is context too: following weighs what is
            // held, and what was released, through the same memory Adaptive
            // keeps.
            if let Some(node) = node {
                let voice = Voice {
                    source: record.source,
                    lifetime: record.serial,
                    onset_pitch: incoming + correction,
                    node: Some(node),
                    decision,
                    onset: record.sample,
                };
                if !self.sequencer.hold(voice) {
                    self.status |= session::POLICY;
                }
            }
            return Some(Assigned { correction, node, decision, player, channel_pitch });
        }
        self.sequencer.fill(self.rate);
        let musical: policy::MusicalConfig = config.into();
        let onset = policy::OrderedOnset { pitch: incoming };
        let count = self.sequencer.working.len();
        let selected = policy::assign_new_note(
            musical,
            &self.sequencer.working[..count],
            &self.sequencer.memory,
            onset,
            &mut self.sequencer.scratch,
        );
        let (assignment, decision) = match selected {
            Ok(selection) => {
                self.sequencer.decision += 1;
                self.decisions += 1;
                (selection.assignment, self.sequencer.decision)
            }
            Err(_) => {
                // The one thing the policy can refuse. That onset sounds at raw
                // (or declared) pitch, the status says so, and nothing is
                // silenced for it.
                self.status |= session::POLICY;
                (policy::Assignment::NoCandidate { correction_microcents: 0 }, 0)
            }
        };
        let (correction, node) = (assignment.correction_microcents(), assignment.node());
        self.reply(record, correction);
        let mut struck = record.sample;
        if decision != 0 {
            struck = self.sequencer.memory.attack(
                onset.pitch,
                correction,
                self.sequencer.config.policy.tolerance,
                node,
                record.sample,
            );
            if node.is_some() {
                self.sequencer.reference_source = Some(record.source);
            }
        }
        let voice = Voice {
            source: record.source,
            lifetime: record.serial,
            onset_pitch: onset.pitch + correction,
            node,
            decision,
            onset: struck,
        };
        if !self.sequencer.hold(voice) {
            self.status |= session::POLICY;
        }
        Some(Assigned { correction, node, decision, player, channel_pitch })
    }

    /// Whether `record` may decide and enter the musical context: a
    /// still-current arrival from a source with Retune on. Captures stay
    /// visible with Retune off.
    fn current(&self, record: &Record) -> bool {
        record.retune & 1 != 0 && record.retune == self.rows[usize::from(record.source)].retune
    }

    /// The current onsets of the attack group at `position`'s sample, from
    /// `position` on, with their channel and key.
    fn group(&self, position: usize) -> impl Iterator<Item = (Record, u8, u8)> + '_ {
        let sample = self.batch[position].sample;
        self.batch[position..]
            .iter()
            .take_while(move |record| record.sample == sample)
            .filter(|record| self.current(record))
            .filter_map(|&record| {
                record.event.attack().map(|(_, channel, key, _)| (record, channel, key))
            })
    }

    /// The attack group at `position`'s sample, scanned at its first onset in
    /// each pass, before any of it is scored: a pending loop reset applies,
    /// what each same-key onset
    /// displaces leaves the context, so neither the map's following nor the
    /// strip's weighs a pitch against the voice it replaces, and under
    /// Adaptive's strip every current onset drained this pass is counted
    /// once (`Follower::strike`).
    fn scan_group(&mut self, position: usize, config: ResolvedConfig) {
        let sample = self.batch[position].sample;
        let pass = self.sequencer.pass;
        if self.sequencer.scanned == Some((sample, pass)) {
            return;
        }
        self.sequencer.scanned = Some((sample, pass));
        // A loop or seek's reset goes first, so the group it clears for is
        // counted after it, not wiped with what came before.
        self.sequencer.take_loop();
        let mut index = position;
        while index < self.batch.len() && self.batch[index].sample == sample {
            let record = self.batch[index];
            index += 1;
            if let Some((_, channel, key, _)) = record.event.attack() {
                self.forget_replaced(record.source, channel, key);
            }
        }
        let policy = config.policy;
        if self.sequencer.engine == TuningEngine::Adaptive
            && policy.decode_fifths
            && policy.follow_spellings
        {
            let mut strikes = [0i32; BATCH_EVENTS];
            let mut count = 0;
            for (_, channel, key) in self.group(position).take(BATCH_EVENTS) {
                strikes[count] = fifths_for_midi(channel, key);
                count += 1;
            }
            for &fifths in &strikes[..count] {
                self.sequencer.follower.strike(fifths, sample, self.rate);
            }
        }
    }

    /// The flat column the strip stands at for the onset at `position`:
    /// where the Position fifths set it, moved by the follower when Follow
    /// spellings is on. The follower decides each attack group once, at its
    /// first decoded onset, before any of it is tuned (`Follower::decide`).
    fn strip_place(&mut self, position: usize, config: ResolvedConfig) -> i32 {
        let sample = self.batch[position].sample;
        let set = self.sequencer.position.threes;
        if !config.policy.follow_spellings {
            self.sequencer.follower.clear();
            return policy::strip::flat_column(set);
        }
        if self.sequencer.decided != Some(sample) {
            self.sequencer.decided = Some(sample);
            let mut sounding = policy::strip::Spellings::default();
            let mut held = [(0, LatticePos::ORIGIN); policy::MAX_CONTEXT];
            let mut count = 0;
            for node in self.sequencer.context.iter().flatten().filter_map(|voice| voice.node) {
                let fifths = node.threes + 4 * node.fives;
                sounding.insert(fifths);
                held[count] = (fifths, node);
                count += 1;
            }
            for (_, channel, key) in self.group(position) {
                sounding.insert(fifths_for_midi(channel, key));
            }
            let sequencer = &mut self.sequencer;
            sequencer.follower.decide(sample, self.rate, &sounding, &held[..count], set);
        }
        self.sequencer.follower.flat(set)
    }

    /// What actually arrived, key number and every tuning the source already
    /// applied to it. Both engines decide from this one pitch: whatever a
    /// keyboard's own temperament, an MTS scale or a bend held at the attack
    /// has done is part of the note here, not something to correct on top of.
    /// A decoded source's declared tuning is not in it: Adaptive adds that
    /// itself, and Lattice Map reads the key alone.
    fn arriving(&self, record: Record, channel: u8, key: u8) -> i64 {
        let channel_pitch = self.rows[usize::from(record.source)].state.channel_pitch(channel);
        i64::from(key) * 100_000_000
            + channel_pitch
            + (record.player * 100_000_000.0).round() as i64
    }

    /// The selected map where it sounds for the onset at `position`: moved by
    /// the follow offset, which the first onset of each attack group decides
    /// for the whole group. Releases at that sample have all applied by then,
    /// because a sample's onsets sort after its releases, so a chord change
    /// on one sample is weighed against what is actually still held.
    fn follow(&mut self, position: usize, map: LatticeMap) -> LatticeMap {
        let sample = self.batch[position].sample;
        if self.sequencer.decided != Some(sample) {
            self.sequencer.decided = Some(sample);
            self.sequencer.followed = if self.sequencer.follow == Follow::Off {
                LatticePos::ORIGIN
            } else {
                self.decide_follow(position, map)
            };
        }
        LatticeMap { position: map.position + self.sequencer.followed, ..map }
    }

    /// The follow offset for the attack group starting at `position`, whose
    /// replaced voices `scan_group` has already let go.
    fn decide_follow(&mut self, position: usize, map: LatticeMap) -> LatticePos {
        let mut keys = [0i64; FOLLOW_GROUP];
        let mut struck = 0;
        for (record, channel, key) in self.group(position) {
            // One per pitch class, so octave doublings never crowd a class out.
            let key = LatticeMap::rounded_key(self.arriving(record, channel, key));
            if struck < FOLLOW_GROUP
                && !keys[..struck].iter().any(|k| (k - key).rem_euclid(12) == 0)
            {
                keys[struck] = key;
                struck += 1;
            }
        }
        // Adaptive's own context: the held voices, or with nothing held the
        // released ones, each decayed on the half-life.
        let sequencer = &mut self.sequencer;
        sequencer.fill(self.rate);
        let held = sequencer.context.iter().any(Option::is_some);
        map.follow(sequencer.follow, sequencer.followed, &sequencer.working, held, &keys[..struck])
    }

    fn reply(&mut self, record: Record, correction: i64) {
        let reply = Reply { epoch: record.epoch, serial: record.serial, correction };
        if record.source == DIRECT {
            self.tune.reply_direct(reply);
            return;
        }
        let Some(ends) = self.ends.as_mut() else { return };
        if let Some(end) = ends[usize::from(record.source)].as_mut() {
            // A full reply ring means that onset emits uncorrected, which its
            // own Tune counts. There is nothing to retry and nobody to tell.
            let _ = end.replies.push(reply);
        }
    }

    /// Fold one scheduled event into the source's state, and hold the delta it
    /// produced for this callback's publication pass.
    fn schedule_delta(
        &mut self,
        event: Event,
        record: Record,
        scheduled: i64,
        assignment: Option<Assigned>,
    ) {
        let index = usize::from(record.source);
        let identity = identity(record.source);
        // A row's times never go back, so its deltas read in the order it
        // published them: a record read late across a closed gap or a lost
        // callback, or one scheduled at a lowered delay, lands at the row's
        // latest time instead (Yan, #1885).
        let time = self.presentation(scheduled);
        let time = self.rows[index].applied_time.map_or(time, |applied| applied.max(time));
        self.rows[index].sequence += 1;
        let stamp = Stamp {
            source: identity,
            sequence: self.rows[index].sequence,
            lifetime: record.serial,
            time,
        };
        let delta = self.rows[index].state.apply(event, stamp);
        if let Some(assigned) = assignment {
            self.rows[index].state.assign(
                record.serial,
                assigned.correction,
                assigned.node,
                assigned.decision,
                assigned.player,
                assigned.channel_pitch,
            );
        }
        if let Some(mut delta) = delta {
            // The voice now carries its frozen choice, so the delta states
            // the pitch this note sounds at if its correction reaches the Tune
            // in time, rather than its raw key. If it does not, the Tune's
            // miss withdraws the correction (`withdraw_misses`).
            if let Some(voice) = self.rows[index].state.voice(delta.lifetime) {
                delta.assignment = VoiceBaseline::metadata(voice);
                if delta.pitch_microcents.is_some() {
                    delta.pitch_microcents = Some(voice.pitch_microcents);
                }
            }
            self.hold(record.source, delta);
        }
        // A channel bend moves every voice held on its channel and has no
        // delta of its own, so each moved voice gets one: its new pitch as a
        // bend, sequenced after the event that moved it (#1151).
        if let Some(channel) = self.rows[index].state.moved_channel {
            let mut moved = [0; HELD_PER_SOURCE];
            let mut count = 0;
            for voice in self.rows[index].state.voices().filter(|v| v.channel == channel) {
                moved[count] = voice.lifetime;
                count += 1;
            }
            for &lifetime in &moved[..count] {
                self.rows[index].sequence += 1;
                let stamp = Stamp { sequence: self.rows[index].sequence, ..stamp };
                if let Some(delta) = self.rows[index].state.retuned(lifetime, stamp) {
                    self.hold(record.source, delta);
                }
            }
        }
        self.rows[index].applied = self.rows[index].sequence;
    }

    /// Queue one delta for this callback's publication pass. A full queue
    /// owes an explicit gap and a snapshot: current state cannot replace history.
    fn hold(&mut self, source: u8, delta: NoteDelta) {
        let time = delta.event.time;
        let row = &mut self.rows[usize::from(source)];
        row.applied_time = Some(row.applied_time.map_or(time, |old| old.max(time)));
        if self.pending.len() < BATCH_EVENTS {
            self.pending.push(Published { source, delta });
        } else {
            self.pending_loss = Some(self.pending_loss.map_or(time, |old| old.min(time)));
            row.repair = publication::Lanes::both(true);
            self.status |= session::PUBLICATION;
        }
    }

    /// Everything this callback scheduled, out to both lanes, each delta timed
    /// where it sounds. The take places it by that time, so nothing here waits
    /// for a later block or asks which run a delta belongs to.
    fn flush(&mut self, recorder: &mut Recorder) {
        for position in 0..self.pending.len() {
            let item = self.pending[position];
            let index = usize::from(item.source);
            let outcome = recorder.publish_note(item.delta);
            for lane in publication::Lane::ALL {
                if outcome[lane].is_err() {
                    self.rows[index].repair[lane] = true;
                    self.status |= session::PUBLICATION;
                }
            }
            if outcome.take.is_ok() && outcome.display.is_ok() {
                self.published += 1;
            }
        }
        // Clear in place. Taking the vector would leave an empty one behind
        // and the reserve that refilled it would allocate, on audio.
        self.pending.clear();
        if let Some(time) = self.pending_loss.take() {
            // Let the ordinary all-source snapshots repair each lane.
            recorder.publication_lost(time);
        }
        let outage = recorder.take_publication_outage();
        for lane in publication::Lane::ALL {
            if outage[lane] {
                for row in self.rows.iter_mut() {
                    row.repair[lane] = true;
                }
                self.status |= session::PUBLICATION;
            }
        }
    }

    fn presentation(&self, sample: i64) -> f64 {
        self.mapping.at(sample)
    }

    /// Everything this callback sequenced, then the snapshots owed: what is
    /// sounding now, built from what the Hub itself scheduled, which a display
    /// or take reads after a gap and a take reads at the start of every run.
    /// It reconstructs no history.
    ///
    /// A run that ends at this callback's start pushed its end before this,
    /// but what this callback collected was played in it or a callback before
    /// by a Tune that lags, and sounds a delay later, at or after that end.
    pub fn publish(&mut self, owner: &mut Owner, recorder: &mut Recorder) {
        self.flush(recorder);
        if self.callback.is_some() {
            self.snapshots(owner, recorder);
        }
    }

    /// Pay every snapshot a row owes and can state now, one lane at a time.
    ///
    /// Both callers reach the run latch below, and `end` never re-opens what
    /// `publish` opened: it reads the same callback's run, which `opened`
    /// already holds.
    fn snapshots(&mut self, owner: &Owner, recorder: &mut Recorder) {
        // Snapshots are timed at the block's start, where a run the block
        // opens begins.
        let block = owner.recording.block_start;
        let time = self.presentation(block);
        // Every recording run — the take's first, and each resume — opens with
        // a take-lane snapshot of every row (#1129). A voice that began where
        // nothing recorded — before arming, or in a pause — has its attack in
        // no run, and the take would otherwise play its release and
        // expressions against nothing.
        if let Some(run) = recorder.open_run() {
            if self.opened != Some(run) {
                self.opened = Some(run);
                for row in self.rows.iter_mut() {
                    row.repair.take = true;
                }
            }
        }
        for index in 0..=TUNERS {
            // A row whose Tune has left still pays what it owes, with the empty
            // snapshot its cleared state makes: a consumer that missed its last
            // releases in a gap hears it holds nothing, where silence would
            // leave those notes drawn for the life of the instance (#1195). A
            // row that never
            // published has nothing to answer for, and a consumer that never
            // heard of it must not start now.
            let departed = !self.rows[index].live;
            if !self.rows[index].repair.any() || (departed && self.rows[index].sequence == 0) {
                continue;
            }
            // A snapshot cuts at every delta this row applied, and has to
            // FOLLOW all of them. On the lane: the cut is `applied`, which
            // counts every delta this row has scheduled, and both callers have
            // flushed every one of those before asking, since the display and
            // the take both hold that no history falls at or below a cut they
            // have adopted, checked in debug builds (#1127).
            //
            // And in time, which is what a take is sorted by on read: a delta
            // the cut covers that sorted after its snapshot would replay after
            // the snapshot that already restated it. Those deltas sound up to D
            // after this callback, so the frame is timed at the latest of them
            // rather than at the block's start, which also leaves no voice
            // whose onset lies after the frame that claims it is sounding.
            let at = self.rows[index].applied_time.map_or(time, |applied| applied.max(time));
            let identity = identity(index as u8);
            // One lane at a time, on its own free cells. A display ring nobody
            // is draining must not hold this frame back from a healthy take.
            for lane in publication::Lane::ALL {
                if !self.rows[index].repair[lane] || recorder.publication_free()[lane] < 2 {
                    continue;
                }
                let applied = self.rows[index].applied;
                let Some(mut frame) = self.rows[index].state.baseline(identity, applied, at) else {
                    continue;
                };
                frame.shown = self.rows[index].show;
                if recorder.publish_baseline(lane, &frame).is_ok() {
                    self.rows[index].repair[lane] = false;
                }
            }
        }
    }

    pub fn end(&mut self, owner: &mut Owner, recorder: &mut Recorder) {
        // A callback whose audio did not run still publishes what it
        // sequenced.
        self.flush(recorder);
        // A snapshot owed and refused for room is owed now, not a callback
        // later that may never come.
        if self.callback.is_some() {
            self.snapshots(owner, recorder);
        }
        let diagnostics_due = self.tune.end();
        self.status |= self.tune.status();
        self.shared.status.store(self.status, Ordering::Release);
        if diagnostics_due {
            self.publish_diagnostics();
        }
        self.callback = None;
    }

    fn publish_diagnostics(&self) {
        self.shared.diagnostics.publish_hub(super::diagnostics::HubReport {
            epoch: self.epoch,
            rows: self.rows.iter().take(TUNERS).filter(|row| row.live).count() as u64,
            context: self.sequencer.context.iter().flatten().count() as u64,
            decisions: self.decisions,
            published: self.published,
            reference: self.sequencer.memory.reference,
            status: self.status,
            direct_held: self.tune.held() as u64,
            direct_pending: self.tune.pending() as u64,
        });
        self.shared.request_main();
    }

    /// The one authority on what is sounding. There is no second copy to
    /// agree with it any more, which is most of why this file is short.
    #[cfg(test)]
    pub fn test_voice(&self, source: u8, channel: u8, key: u8) -> Option<VoiceBaseline> {
        self.rows[usize::from(source)]
            .state
            .voices()
            .find(|voice| voice.channel == channel && voice.note == key)
            .copied()
    }
    #[cfg(test)]
    pub fn test_held(&self, source: u8) -> usize {
        self.rows[usize::from(source)].state.voices().count()
    }
    #[cfg(test)]
    pub fn test_context(&self) -> usize {
        self.sequencer.context.iter().flatten().count()
    }
    /// Observe retained musical state after the callback, including any silence
    /// expiry.
    #[cfg(test)]
    pub fn test_next_context(&self) -> TestContext {
        let mut observed = Sequencer {
            context: self.sequencer.context.clone(),
            memory: self.sequencer.memory.clone(),
            config: self.sequencer.config,
            ..Default::default()
        };
        observed.fill(self.rate);
        TestContext { reference: observed.memory.reference, context: observed.working }
    }
    /// Released notes remembered, whatever is held: the next-attack context
    /// shows memory only when nothing is.
    #[cfg(test)]
    pub fn test_memory(&self) -> usize {
        let mut context = Vec::new();
        self.sequencer.memory.append(&mut context, self.sequencer.config.policy, self.rate);
        context.len()
    }
}

#[cfg(test)]
#[derive(Debug)]
pub struct TestContext {
    pub reference: i64,
    pub context: Vec<policy::ContextPitch>,
}

const _: () = assert!(std::mem::size_of::<Record>() <= 96);
