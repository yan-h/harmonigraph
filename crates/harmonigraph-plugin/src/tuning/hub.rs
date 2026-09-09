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

use harmonigraph_core::canonical::{ClockId, EventTiming, NoteDelta, VoiceBaseline};
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_core::{policy, LatticePos, SourceId};
use harmonigraph_record::{publication, Recorder};
use nice_plug::wrapper::clap::configuration::OwnedInput;
use nice_plug::wrapper::clap::performance as api;

use super::event::Event;
use super::session::{self, HubEnds, Reply};
use super::state::{Stamp, State};
use super::tune::Tune;
use super::{setup, BATCH_EVENTS, DIRECT, HELD_PER_SOURCE, HELD_SESSION, TUNERS};

type Owner = crate::configuration::Owner;

/// `CLAP_TRANSPORT_HAS_SECONDS_TIMELINE`, which the wrapper hands over as a
/// bare flag word rather than as a clap-sys type.
const HAS_SECONDS: u32 = 1 << 2;

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
    source: u8,
    sample: i64,
    serial: u64,
    epoch: u64,
    event: Event,
}

impl Record {
    fn onset(&self) -> bool {
        self.event.attack().is_some()
    }
    /// Merge order inside one sample: every release and controller from every
    /// source applies before any onset, and onsets then run in the musical
    /// key/channel/source tie-break. Ties inside each half fall back to the
    /// per-source input order, which copying preserves.
    fn order(&self) -> (bool, u8, u8, u8, u64) {
        match self.event.attack() {
            Some((_, channel, key, _)) => (true, key, channel, self.source, self.serial),
            None => (false, 0, 0, self.source, self.serial),
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
    configuration: ResolvedConfig,
}

/// One scheduled delta, waiting for the callback's own recording segment to
/// exist. Sequencing runs at the input boundary, because that is where the
/// replies have to be minted; the recorder does not register this block until
/// `process`, so routing a delta before then would route it into nothing.
#[derive(Clone, Copy)]
struct Published {
    source: u8,
    delta: NoteDelta,
    /// The sample the input arrived at, which is the one inside this block's
    /// segment. The delta itself carries the time it is scheduled to sound.
    input: i64,
    timing: EventTiming,
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
}
impl Voice {
    fn context_pitch(self) -> policy::ContextPitch {
        policy::ContextPitch { pitch: self.onset_pitch, node: self.node, weight: 1.0 }
    }
}

/// The musical half, and nothing else. Everything in here is policy v2 state
/// as it was; the transport around it is what #786 replaced.
struct Sequencer {
    context: Box<[Option<Voice>]>,
    memory: policy::Memory,
    config: policy::MusicalConfig,
    scratch: Box<policy::PolicyScratch>,
    working: Vec<policy::ContextPitch>,
    published: Vec<policy::ContextPitch>,
    published_config: Option<policy::MusicalConfig>,
    published_reference: i64,
    last_release: Option<i64>,
    decision: u64,
    /// A loop or seek was detected and the next attack clears memory, if the
    /// control says so. Held notes keep their frozen assignments either way.
    loop_pending: bool,
}

impl Default for Sequencer {
    fn default() -> Self {
        Self {
            context: vec![None; HELD_SESSION].into_boxed_slice(),
            memory: policy::Memory::default(),
            config: harmonigraph_core::configuration::ConfigReducer::default().resolved().into(),
            scratch: Box::default(),
            working: Vec::with_capacity(policy::MAX_CONTEXT + policy::MAX_MEMORY),
            published: Vec::with_capacity(policy::MAX_CONTEXT + policy::MAX_MEMORY),
            published_config: None,
            published_reference: 0,
            last_release: None,
            decision: 0,
            loop_pending: false,
        }
    }
}

impl Sequencer {
    /// The context one assignment sees: every held voice newest first,
    /// deduplicated within the repetition tolerance, then released memory.
    fn fill(&mut self) {
        self.working.clear();
        let mut voices = [None; HELD_SESSION];
        let mut count = 0;
        for voice in self.context.iter().flatten() {
            voices[count] = Some(*voice);
            count += 1;
        }
        voices[..count].sort_unstable_by_key(|v| std::cmp::Reverse(v.unwrap().decision));
        for voice in voices[..count].iter().flatten() {
            if !self.working.iter().any(|v| {
                v.pitch.abs_diff(voice.onset_pitch) <= u64::from(self.config.policy.tolerance)
            }) {
                self.working.push(voice.context_pitch());
            }
        }
        self.memory.append(&mut self.working, self.config.policy);
    }
    /// Drop a context voice without contributing it to released memory: it
    /// never sounded, because the source's own state had no cell for it.
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
            self.memory.release(voice.context_pitch(), source, self.config.policy);
            self.last_release = Some(sample);
        }
    }
    /// Every voice of one source at once: a cut, a Stop, or a Tune leaving.
    fn release_source(&mut self, source: u8, sample: i64) {
        for cell in self.context.iter_mut() {
            if cell.is_some_and(|v| v.source == source) {
                let voice = cell.take().unwrap();
                self.memory.release(voice.context_pitch(), source, self.config.policy);
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
            self.memory.clear();
            self.last_release = None;
        }
    }
    fn publish_neighbourhood(&mut self, shared: &setup::Shared, config: policy::MusicalConfig) {
        self.config = config;
        self.fill();
        // Keyed only by values that decide the next assignment. Display
        // tolerance, callback time and camera movement must not restart work.
        if self.published_config != Some(config)
            || self.published_reference != self.memory.reference
            || self.published != self.working
        {
            shared.neighbourhood.publish(config, self.memory.reference, &self.working);
            self.published_config = Some(config);
            self.published_reference = self.memory.reference;
            self.published.clear();
            self.published.extend_from_slice(&self.working);
        }
    }
}

#[derive(Default)]
struct Row {
    /// A Tune holds this row. Read from the row's own owner each callback.
    live: bool,
    state: State,
    sequence: u64,
    applied: u64,
    delay: i64,
    repair: publication::Lanes<bool>,
    baseline_id: publication::Lanes<u64>,
}

pub struct Hub {
    pub shared: Arc<setup::Shared>,
    /// The Hub's own track. It is a Tune, so its notes are delayed by D and it
    /// is sequenced as a seventeenth source rather than observed as a special
    /// case. Its ring is a direct call.
    pub tune: Tune,
    id: u64,
    ends: Option<Box<[Option<HubEnds>; TUNERS]>>,
    epoch: u64,
    rows: Box<[Row]>,
    batch: Vec<Record>,
    pending: Vec<Published>,
    sequencer: Box<Sequencer>,
    rate: f64,
    callback: Option<api::Callback>,
    anchor: Option<(i64, f64)>,
    clock: ClockId,
    /// Host seconds against elapsed samples, for loop and seek detection.
    seconds: Option<f64>,
    status: u32,
    /// Which row the next collection starts from.
    rotation: usize,
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
            pending: Vec::with_capacity(BATCH_EVENTS),
            sequencer: Box::default(),
            rate: 0.0,
            callback: None,
            anchor: None,
            clock: ClockId::default(),
            seconds: None,
            status: 0,
            rotation: 0,
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
        self.anchor = None;
        self.seconds = None;
        // The Hub's own delay is the same knob every Tune has, at 1x: its
        // input is already at the Hub, so one buffer is all its own round trip
        // can need.
        self.tune.activate(rate, frames, 1);
    }

    pub fn begin(&mut self, callback: api::Callback, owner: &mut Owner, presentation: f64) {
        self.callback = Some(callback);
        // One clock for the life of this Harmonigraph. Nothing advances it any
        // more, because nothing can hold a record long enough for an old one
        // to reach the recorder: what this callback sequences, it publishes.
        if owner.recording.clock.runtime_session == 0 {
            owner.recording.clock.runtime_session = self.id;
        }
        self.clock = owner.recording.clock;
        if self.anchor.is_none() {
            self.anchor = Some((callback.steady_time, presentation));
        }
        // The Tune half runs first, because it is what turns an editor Reset
        // into the epoch bump this callback then adopts.
        self.tune.begin(callback);
        self.adopt();
        self.detect_loop(callback);
        self.sequencer.publish_neighbourhood(&self.shared, owner.reducer.resolved().into());
        // Silence expires released memory against the completed input
        // frontier, which is this callback's start: everything before it has
        // been sequenced, and nothing after it has arrived.
        self.sequencer.expire(callback.steady_time, self.rate);
    }

    /// One atomic load of the epoch. A different value is the cut: a Tune came
    /// or went, or somebody pressed Reset, and every row's voices are ending.
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
            // Every epoch change ends every voice. What it does to released
            // memory is the only thing that depends on which change it was —
            // and the question is what has happened since the epoch this Hub
            // was running under, not what the newest bump happens to be. A
            // Reset with an attach behind it is still a Reset.
            if session.is_reset(self.epoch)
                || session.is_stop(self.epoch) && self.sequencer.config.policy.reset_stop
            {
                self.sequencer.memory.clear();
                self.sequencer.last_release = None;
            }
            self.batch.clear();
            // Deltas scheduled before the cut describe notes it has just
            // ended. Every row is already owed a snapshot, which is what tells
            // the display what is actually sounding now.
            self.pending.clear();
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

    /// A greater-than-two-millisecond discontinuity between the host's seconds
    /// timeline and elapsed sample time is a loop or a seek. What it does then
    /// is the `reset_loop` control's business, at the next attack.
    fn detect_loop(&mut self, callback: api::Callback) {
        let Some(transport) = callback.transport else {
            return;
        };
        self.observe_seconds(
            transport.flags,
            transport.song_pos_seconds,
            Some(callback.steady_time),
        );
    }

    /// One observation of the host's seconds timeline against elapsed samples.
    /// `song_pos_seconds` is CLAP fixed point, 1/2^31 of a second.
    fn observe_seconds(&mut self, flags: u32, song_pos_seconds: i64, sample: Option<i64>) {
        let (Some(sample), true) = (sample, self.rate > 0.0 && flags & HAS_SECONDS != 0) else {
            return;
        };
        let offset = song_pos_seconds as f64 / 2_147_483_648.0 - sample as f64 / self.rate;
        if self.seconds.is_some_and(|previous| (offset - previous).abs() > 0.002) {
            self.sequencer.loop_pending = true;
        }
        self.seconds = Some(offset);
    }

    pub fn input(&mut self, input: OwnedInput) {
        // A host may supply the transport as the callback's own value, as an
        // input event, or as both. The seek this is looking for has to be
        // found in whichever one arrives.
        if let nice_plug::wrapper::clap::configuration::InputValue::Transport(transport) =
            input.value
        {
            self.observe_seconds(transport.flags, transport.song_pos_seconds, input.sample);
        }
        self.tune.input(input);
    }
    pub fn schedule(&mut self, block: api::Block, output: &mut api::Output<'_>) {
        self.tune.schedule(block, output);
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
        self.collect();
        if self.batch.is_empty() {
            return;
        }
        self.batch.sort_unstable_by_key(|record| (record.sample, record.order()));
        let Some(config) = owner.block_configuration(self.clock) else {
            // No configuration to assign against, so this batch is discarded
            // rather than held: every onset in it sounds uncorrected and the
            // Tunes that sent them count the misses. The one thing that must
            // not happen is that it goes unsaid, which is what the fault bit
            // is for — an evaluation that never ran is a refused one.
            self.status |= session::POLICY;
            self.batch.clear();
            return;
        };
        self.sequencer.config = config.into();
        let mut index = 0;
        while index < self.batch.len() {
            let sample = self.batch[index].sample;
            let mut end = index;
            while end < self.batch.len() && self.batch[end].sample == sample {
                end += 1;
            }
            for position in index..end {
                self.apply(position, index, end, config);
            }
            index = end;
        }
        self.batch.clear();
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
            if capture.epoch == direct_epoch {
                self.batch.push(Record {
                    source: DIRECT,
                    sample: capture.sample,
                    serial: capture.serial,
                    epoch: capture.epoch,
                    event: capture.event,
                });
            }
        }
        let epoch = self.epoch;
        // Start one row further along each callback. A batch that fills always
        // fills from the front, so a fixed order would make the same row wait
        // every time rather than each of them waiting in turn.
        let rotation = self.rotation;
        self.rotation = (self.rotation + 1) % TUNERS;
        let Some(ends) = self.ends.as_mut() else { return };
        for offset in 0..TUNERS {
            let slot = (rotation + offset) % TUNERS;
            let Some(end) = ends[slot].as_mut() else { continue };
            while self.batch.len() < BATCH_EVENTS {
                let Ok(capture) = end.captures.pop() else { break };
                if capture.epoch != epoch {
                    continue;
                }
                self.batch.push(Record {
                    source: slot as u8,
                    sample: capture.sample,
                    serial: capture.serial,
                    epoch: capture.epoch,
                    event: capture.event,
                });
            }
        }
    }

    /// One record, in order: its effect on the policy's context, its decision
    /// if it is an onset, and its place in the schedule the display and the
    /// take draw.
    fn apply(&mut self, position: usize, group: usize, group_end: usize, config: ResolvedConfig) {
        let record = self.batch[position];
        let source = usize::from(record.source);
        let scheduled = record.sample.saturating_add(self.rows[source].delay);
        if let Some((_, channel, key, _)) = record.event.attack() {
            // A missed release copy can leave the Hub fuller than the Tune.
            // Refuse before assignment mutates policy or sends a reply: this
            // locally admitted voice still sounds raw and is tracked by Tune.
            if self.rows[source].state.onset_cell(channel, key).is_none() {
                self.rows[source].state.complete = false;
                self.status |= session::DROPPED;
                return;
            }
        }
        let assignment =
            record.onset().then(|| self.assign(record, group, group_end, config)).flatten();
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

    fn release_addressed(&mut self, record: Record, channel: u8, key: u8) {
        let lifetime = self.rows[usize::from(record.source)]
            .state
            .voices()
            .find(|voice| voice.channel == channel && voice.note == key)
            .map(|voice| voice.lifetime);
        if let Some(lifetime) = lifetime {
            self.sequencer.release_voice(record.source, lifetime, record.sample);
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
    fn assign(
        &mut self,
        record: Record,
        group: usize,
        group_end: usize,
        config: ResolvedConfig,
    ) -> Option<Assigned> {
        let (_, channel, key, _) = record.event.attack()?;
        let source = usize::from(record.source);
        let player = self.initial_tuning(record, group, group_end);
        let channel_pitch = self.rows[source].state.channel_pitch(channel);
        if self.sequencer.loop_pending {
            if config.policy.reset_loop {
                self.sequencer.memory.clear();
                self.sequencer.last_release = None;
            }
            self.sequencer.loop_pending = false;
        }
        self.sequencer.fill();
        let onset = policy::OrderedOnset {
            pitch: i64::from(key) * 100_000_000
                + channel_pitch
                + (player * 100_000_000.0).round() as i64,
        };
        let count = self.sequencer.working.len();
        let selected = policy::assign_new_note(
            config.into(),
            &self.sequencer.working[..count],
            self.sequencer.memory.reference,
            onset,
            &mut self.sequencer.scratch,
        );
        let (correction, node, decision) = match selected {
            Ok(selection) => {
                self.sequencer.decision += 1;
                self.decisions += 1;
                (
                    selection.assignment.correction_microcents(),
                    selection.assignment.node(),
                    self.sequencer.decision,
                )
            }
            Err(_) => {
                // The one thing the policy can refuse. That onset sounds at raw
                // pitch, the status says so, and nothing is silenced for it.
                self.status |= session::POLICY;
                (0, None, 0)
            }
        };
        self.reply(record, correction);
        if decision != 0 {
            self.sequencer.memory.attack(
                onset.pitch + correction,
                correction,
                self.sequencer.config.policy,
            );
        }
        let voice = Voice {
            source: record.source,
            lifetime: record.serial,
            onset_pitch: onset.pitch + correction,
            node,
            decision,
        };
        if let Some(cell) = self.sequencer.context.iter_mut().find(|cell| cell.is_none()) {
            *cell = Some(voice);
        } else {
            self.status |= session::POLICY;
        }
        Some(Assigned { correction, node, decision, player, channel_pitch, configuration: config })
    }

    /// A per-note pitch expression at its own note's sample is the value that
    /// note starts from. The whole group is in hand, so this is a scan of one
    /// cohort rather than a stash that has to be retired.
    fn initial_tuning(&self, record: Record, group: usize, group_end: usize) -> f64 {
        let Some((id, channel, key, _)) = record.event.attack() else { return 0.0 };
        for other in &self.batch[group..group_end] {
            if other.source != record.source {
                continue;
            }
            let Event::Expression { kind: 2, value, .. } = other.event else { continue };
            if other.event.matches(id, channel, key) && value.is_finite() {
                return value;
            }
        }
        0.0
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
        let time = self.presentation(scheduled);
        let timing = EventTiming {
            clock: self.clock,
            input: record.sample,
            planned: Some(scheduled),
            sample: scheduled,
            sample_rate: self.rate,
        };
        self.rows[index].sequence += 1;
        let stamp = Stamp {
            source: identity,
            sequence: self.rows[index].sequence,
            lifetime: record.serial,
            time,
            input_time: self.presentation(record.sample),
            timing,
        };
        let delta = self.rows[index].state.apply(event, stamp);
        if let Some(assigned) = assignment {
            self.rows[index].state.assign(
                record.serial,
                assigned.correction,
                assigned.node,
                assigned.decision,
                assigned.configuration,
                assigned.player,
                assigned.channel_pitch,
            );
        }
        let Some(mut delta) = delta else {
            // Defensive cleanup if an onset cannot be retained. Admission
            // above normally refuses it before assignment; policy must not
            // keep a lifetime that the scheduled state cannot address.
            if event.attack().is_some() {
                self.sequencer.forget_voice(record.source, record.serial);
                self.status |= session::DROPPED;
            }
            self.rows[index].applied = self.rows[index].sequence;
            return;
        };
        // The voice now carries its frozen choice, so the delta states the
        // pitch this note will actually sound at rather than its raw key.
        if let Some(voice) = self.rows[index].state.voice(delta.lifetime) {
            delta.assignment = VoiceBaseline::metadata(voice);
            if delta.pitch_microcents.is_some() {
                delta.pitch_microcents = Some(voice.pitch_microcents);
            }
        }
        if self.rows[index].state.pitch_changed {
            self.rows[index].repair = publication::Lanes::both(true);
        }
        self.rows[index].applied = self.rows[index].sequence;
        if self.pending.len() < BATCH_EVENTS {
            self.pending.push(Published {
                source: record.source,
                delta,
                input: record.sample,
                timing,
            });
        } else {
            self.rows[index].repair = publication::Lanes::both(true);
            self.status |= session::PUBLICATION;
        }
    }

    /// Everything this callback scheduled, out to both lanes. It runs from
    /// `process`, after the recorder has registered that sub-block's own
    /// segment: routing asks which pass an event belongs to, and before that
    /// there is no pass for it to belong to.
    ///
    /// A callback's input is sequenced whole, before the first sub-block, so a
    /// delta can be scheduled from input the recorder has not reached yet. It
    /// waits for the sub-block that registers it, in order, and `force` at the
    /// callback's end is what stops one waiting forever.
    fn flush(&mut self, owner: &mut Owner, recorder: &mut Recorder, force: bool) {
        let mut published = 0;
        for position in 0..self.pending.len() {
            let item = self.pending[position];
            let index = usize::from(item.source);
            // Routing takes the sample the input ARRIVED at, which is inside
            // this block; the schedule runs D ahead of it and would fall in no
            // segment at all.
            let route = owner.recording_route(
                EventTiming { sample: item.input, ..item.timing },
                self.presentation(item.input),
            );
            let route = match route {
                Ok(route) => route,
                Err(()) if !force => break,
                Err(()) => {
                    recorder.fail_configuration();
                    Default::default()
                }
            };
            published += 1;
            let outcome = recorder.publish_note(item.delta, route);
            for lane in publication::Lane::ALL {
                if outcome[lane].is_err() {
                    self.rows[index].repair[lane] = true;
                    self.status |= session::PUBLICATION;
                }
            }
            if outcome.take.is_ok() && outcome.display.is_ok() {
                self.published += 1;
            }
            Self::confirm(&self.rows[index], identity(item.source), &mut owner.confirmed);
        }
        // Compact in place. Taking the vector would leave an empty one behind
        // and the reserve that refilled it would allocate, on audio.
        self.pending.copy_within(published.., 0);
        self.pending.truncate(self.pending.len() - published);
    }

    fn confirm(
        row: &Row,
        source: SourceId,
        confirmed: &mut harmonigraph_core::confirmed::ConfirmedPitches,
    ) {
        let _ = row.state.publish_confirmed(source, row.state.complete, confirmed);
    }

    fn presentation(&self, sample: i64) -> f64 {
        let (anchor, time) = self.anchor.unwrap_or((0, 0.0));
        if self.rate <= 0.0 {
            return time;
        }
        time + (sample as f64 - anchor as f64) / self.rate
    }

    /// The snapshot a display or take reads after a gap: what is sounding now,
    /// built from what the Hub itself scheduled. It reconstructs no history.
    pub fn publish(&mut self, owner: &mut Owner, recorder: &mut Recorder) {
        self.flush(owner, recorder, false);
        let outage = recorder.take_publication_outage();
        for lane in publication::Lane::ALL {
            if outage[lane] {
                for row in self.rows.iter_mut() {
                    row.repair[lane] = true;
                }
                self.status |= session::PUBLICATION;
            }
        }
        let Some(callback) = self.callback else { return };
        let sample = callback.steady_time;
        let time = self.presentation(sample);
        let timing = EventTiming {
            clock: self.clock,
            input: sample,
            planned: None,
            sample,
            sample_rate: self.rate,
        };
        let route = owner.recording_route(timing, time).unwrap_or_default();
        for index in 0..=TUNERS {
            if !self.rows[index].live || !self.rows[index].repair.any() {
                continue;
            }
            let identity = identity(index as u8);
            // One lane at a time, on its own free cells and its own next
            // identity. A display ring nobody is draining must not hold this
            // frame back from a healthy take.
            for lane in publication::Lane::ALL {
                if !self.rows[index].repair[lane] || recorder.publication_free()[lane] < 2 {
                    continue;
                }
                let Some(id) = self.rows[index].baseline_id[lane].checked_add(1) else { continue };
                let applied = self.rows[index].applied;
                let Some(frame) = self.rows[index].state.baseline(identity, id, applied, time)
                else {
                    continue;
                };
                if recorder.publish_baseline(lane, &frame, route).is_ok() {
                    self.rows[index].baseline_id[lane] = id;
                    self.rows[index].repair[lane] = false;
                }
            }
        }
        // Every record collected this callback has been published, and nothing
        // still to come is scheduled before this callback began, so this is a
        // sound and monotone frontier without a coverage protocol behind it.
        if owner.recording.source_frontier(self.clock, sample).is_err() {
            recorder.fail_configuration();
        }
    }

    /// The configuration owner refused an evaluation. Like every other fault
    /// here it is a status bit: nothing is silenced and nothing latches.
    pub fn configuration_exhausted(&mut self) {
        self.status |= session::POLICY;
    }

    pub fn end(&mut self, callback: api::Callback, owner: &mut Owner, recorder: &mut Recorder) {
        // Whatever no sub-block reached is published against whatever segment
        // the recorder does have, and says so if there is none.
        self.flush(owner, recorder, true);
        self.tune.end();
        self.status |= self.tune.status();
        self.shared.status.store(self.status, Ordering::Release);
        self.publish_diagnostics(callback);
        self.callback = None;
    }

    fn publish_diagnostics(&self, callback: api::Callback) {
        if !self.shared.diagnostics.due(callback.frames, self.rate) {
            return;
        }
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

    /// Destruction closes the recorder's own boundaries, here and now. There
    /// is no drain to wait for: everything this Hub sequenced was published in
    /// the callback it was sequenced in, so the only thing left to say is
    /// whether any voice was still sounding — which is a take the renderer
    /// warns about rather than a fact this can go and establish.
    pub fn retire_publication(
        &mut self,
        mut owner: Box<Owner>,
        mut recorder: Recorder,
        observation: f64,
    ) {
        recorder.hold_retired_publication();
        owner.recording.dispose_retired_configuration(&mut recorder);
        owner.finish_recording_publication(&mut recorder, observation);
        let held = self.rows.iter().any(|row| row.state.count() != 0);
        owner.recording.finish_retired_publication(&mut recorder, held);
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
        self.rows[usize::from(source)].state.count()
    }
    #[cfg(test)]
    pub fn test_context(&self) -> usize {
        self.sequencer.context.iter().flatten().count()
    }
    #[cfg(test)]
    pub fn test_next_context(&self) -> policy::reach::Snapshot {
        self.shared.neighbourhood.read().expect("published next-attack context")
    }
}

const _: () = assert!(std::mem::size_of::<Record>() <= 80);
