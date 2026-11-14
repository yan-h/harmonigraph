//! What the take needs from the audio thread: where each input arrived, under
//! which arm intent, and which stretches recorded. The take's file routes,
//! translates and picks passes itself (#1760); this side states the runs and
//! proves when a take may close.
use harmonigraph_core::canonical::ClockId;
use harmonigraph_core::configuration::ResolvedConfig;
use harmonigraph_record::{configuration::RecordIntent, publication, Recorder};

/// The Hub's mapping from its samples to the take's presentation clock,
/// anchored once at activation. A run states where it opens on it, and every
/// delta is timed on it, so the two must be one function.
#[derive(Clone, Copy, Default)]
pub(crate) struct Presentation {
    pub anchor: Option<(i64, f64)>,
    pub rate: f64,
}

impl Presentation {
    pub fn at(self, sample: i64) -> f64 {
        let (anchor, time) = self.anchor.unwrap_or((0, 0.0));
        if self.rate <= 0.0 {
            return time;
        }
        time + (sample as f64 - anchor as f64) / self.rate
    }
}

/// A run open on the current clock.
#[derive(Clone, Copy)]
struct Run {
    epoch: u64,
    pass: u32,
    /// The recorder's run serial it opened under, which moves at a resume and
    /// at a split.
    serial: u64,
    start: i64,
}

pub(crate) struct Recording {
    pub clock: ClockId,
    /// Continuous presentation clock for publication controls and loss markers.
    pub observation_time: f64,
    pub presentation: Presentation,
    source_prefix: Option<i64>,
    registered_through: i64,
    /// The end of the last sub-block that recorded on this clock.
    recorded_through: Option<i64>,
    /// The intent captured at this callback's boundary, and where it began.
    pub captured_intent: RecordIntent,
    callback_start: Option<i64>,
    /// The previous callback's start and intent. The Hub's own row, and a Tune
    /// the host runs after the Hub, deliver their input a callback late, so
    /// their notes are stamped with the intent they were played under.
    previous: Option<(i64, RecordIntent)>,
    run: Option<Run>,
    /// The input span of the last run closed on this clock. With the open run,
    /// the recorded samples a late record can still name: two runs cannot
    /// start within one callback.
    closed: Option<(i64, i64)>,
    /// The epoch whose take has opened a run since the clock was cut. Before
    /// its first run a take has recorded nothing, so it owes no proof.
    opened: Option<u64>,
    /// The last run's end, `(epoch, pass, end)`, and the pass that ended
    /// before the run open now, until both frontiers pass its end.
    ended: Option<(u64, u32, i64)>,
    closing: Option<(u64, u32, i64)>,
    /// The configuration last written for this arm, so a stable block writes
    /// nothing and a changed one writes once.
    last: Option<(u64, ResolvedConfig)>,
    pub prefix: i64,
    pub block_start: i64,
    pub block_frames: u32,
    /// Terminal wrapper ownership, separate from successful configuration and
    /// source prefixes.
    pub retired_configuration: Option<bool>,
    pub retirement_finished: bool,
}
impl Default for Recording {
    fn default() -> Self {
        Self {
            clock: ClockId::default(),
            observation_time: 0.0,
            presentation: Presentation::default(),
            source_prefix: None,
            registered_through: i64::MIN,
            recorded_through: None,
            captured_intent: RecordIntent::default(),
            callback_start: None,
            previous: None,
            run: None,
            closed: None,
            opened: None,
            ended: None,
            closing: None,
            last: None,
            prefix: 0,
            block_start: 0,
            block_frames: 0,
            retired_configuration: None,
            retirement_finished: false,
        }
    }
}
impl Recording {
    /// A callback boundary: what is captured now stamps this callback's
    /// input, and what was captured before still stamps the previous one's.
    pub fn begin(&mut self, start: i64, intent: RecordIntent) {
        if let Some(previous) = self.callback_start {
            self.previous = Some((previous, self.captured_intent));
        }
        self.callback_start = Some(start);
        self.captured_intent = intent;
    }

    /// Whether this sub-block records.
    pub fn recording(&self) -> bool {
        self.run.is_some()
    }

    /// Whether `epoch`'s take may not close yet: it has recorded, and either
    /// it is still armed or what it recorded is not yet proven on both
    /// frontiers.
    pub fn owes_closure(&self, recorder: &Recorder, epoch: u64) -> bool {
        self.opened == Some(epoch)
            && (recorder.armed_pass().is_some_and(|(armed, _)| armed == epoch)
                || self.recorded_through.is_some_and(|end| !self.proven(end)))
    }

    fn proven(&self, end: i64) -> bool {
        self.prefix >= end && self.source_prefix.is_some_and(|through| through >= end)
    }

    /// Where an input arrived, and the intent of the callback that held it.
    /// A stale clock, or an input below the proven frontier, is one the take
    /// can no longer place: the caller fails the take.
    pub fn stamp(&self, clock: ClockId, input: i64) -> Result<publication::Span, ()> {
        if clock != self.clock || self.source_prefix.is_some_and(|through| input < through) {
            return Err(());
        }
        let intent = match (self.callback_start, self.previous) {
            (Some(start), _) if input >= start => self.captured_intent,
            (_, Some((start, intent))) if input >= start => intent,
            _ => return Err(()),
        };
        Ok(publication::Span::at(clock, input, intent))
    }

    /// Whether `sample` was recorded, as far as a record still in a ring can
    /// name it: inside the open run or the last one closed.
    pub fn recorded(&self, sample: i64) -> bool {
        self.run.is_some_and(|run| run.start <= sample)
            || self.closed.is_some_and(|(start, end)| start <= sample && sample < end)
    }

    fn end_run(&mut self, recorder: &mut Recorder, at: i64) {
        let Some(run) = self.run.take() else { return };
        recorder.run_end(run.epoch, publication::Stamp { clock: self.clock, input: at });
        self.closed = Some((run.start, at));
        self.ended = Some((run.epoch, run.pass, at));
    }

    /// Close or open this sub-block's run. It records when it has an origin
    /// and the take armed at this callback's boundary is the one the
    /// lifecycle holds; `origin` is where it sits in its pass's audio.
    pub fn record(&mut self, recorder: &mut Recorder, origin: Option<f64>) {
        let start = self.block_start;
        let recorded = origin.and(recorder.armed_pass()).filter(|(epoch, _)| {
            *epoch == self.captured_intent.epoch() && self.captured_intent.is_armed()
        });
        let serial = recorder.recording_run();
        if self
            .run
            .is_some_and(|run| recorded != Some((run.epoch, run.pass)) || serial != run.serial)
        {
            self.end_run(recorder, start);
        }
        if let (Some((epoch, pass)), None, Some(song)) = (recorded, self.run, origin) {
            // A new pass ends the one before it, whose proof then closes it.
            if let Some(ended) = self.ended.filter(|&(e, p, _)| e == epoch && p != pass) {
                self.closing = Some(ended);
            }
            if self.opened != Some(epoch) {
                self.opened = Some(epoch);
                self.last = None;
            }
            let at = publication::Stamp { clock: self.clock, input: start };
            recorder.run(epoch, pass, at, song, self.presentation.at(start));
            self.run = Some(Run { epoch, pass, serial, start });
        }
    }

    /// The configuration in force from `sample`, written when it changed.
    pub fn configuration(&mut self, recorder: &mut Recorder, sample: i64, config: ResolvedConfig) {
        let Some(run) = self.run else { return };
        if self.last != Some((run.epoch, config)) {
            recorder.configuration(run.epoch, self.presentation.at(sample), config);
            self.last = Some((run.epoch, config));
        }
    }

    /// This sub-block is registered through `end`.
    pub fn registered(&mut self, end: i64) {
        self.registered_through = end;
        if self.run.is_some() {
            self.recorded_through = Some(end);
        }
    }

    /// A joined callback boundary can observe Stop: no callback still owns an
    /// earlier armed intent. Close only the span actually registered, leaving
    /// both existing frontiers to prove it whole.
    pub fn observe_joined_disarm(&mut self, recorder: &mut Recorder) {
        let intent = recorder.capture_recording_intent();
        if intent.is_armed() {
            return;
        }
        self.captured_intent = intent;
        recorder.is_armed_at(intent);
        self.end_run(recorder, self.registered_through);
    }
    pub fn dispose_retired_configuration(&mut self, recorder: &mut Recorder) {
        let unfinished = self.retired_configuration.expect("joined configuration producer");
        // Retain every already-applied change in its run.
        self.finish(recorder);
        if unfinished && self.recorded_through.is_some_and(|end| end > self.prefix) {
            recorder.fail_configuration();
        }
    }
    /// All original source producers have joined/sealed and every final actual
    /// cut has been published or explicitly lost. No synthetic prefix or seed.
    pub fn finish_retired_publication(&mut self, recorder: &mut Recorder, unknown_held: bool) {
        assert!(self.retired_configuration.is_some());
        if unknown_held || self.recorded_through.is_some_and(|end| end > self.prefix) {
            recorder.fail_configuration();
        }
        self.run = None;
        self.closed = None;
        self.retirement_finished = true;
        recorder.retired_publication_complete();
    }
    /// A host reset cuts the clock. A run open across it ends here and opens
    /// again on the new clock; what was recorded on the old one must already
    /// be proven, since nothing can publish on it any more.
    pub fn reset(&mut self, recorder: &mut Recorder) {
        self.end_run(recorder, self.registered_through);
        if self.recorded_through.is_some_and(|end| !self.proven(end)) {
            recorder.fail_configuration();
        }
        self.close_pass(recorder);
        let Some(epoch) = self.clock.epoch.checked_add(1) else {
            recorder.fail_configuration();
            return;
        };
        // The pass that ended is proven with everything else on the old
        // clock, so a new pass after the cut may close it at once.
        let ended = self.ended.map(|(epoch, pass, _)| (epoch, pass, i64::MIN));
        let clock = ClockId { epoch, ..self.clock };
        *self = Self { clock, ended, ..Self::default() };
    }

    /// ONLY the canonical audio owner may supply this proof: every actual
    /// output strictly before `through` has been published or explicitly lost.
    /// Source receipt, held baseline and configuration progress are
    /// insufficient.
    ///
    /// A sample here is the Hub's own steady timeline, the same one the
    /// runs are stated on, so there is no offset between them to apply.
    pub fn source_frontier(&mut self, clock: ClockId, through: i64) -> Result<(), ()> {
        if clock != self.clock {
            return Err(());
        }
        if self.source_prefix.is_some_and(|old| through < old) {
            return Err(());
        }
        self.source_prefix = Some(through);
        Ok(())
    }

    /// The end of the last sub-block registered, if any has been since the
    /// clock was last cut.
    pub fn registered_through(&self) -> Option<i64> {
        (self.registered_through != i64::MIN).then_some(self.registered_through)
    }

    fn close_pass(&mut self, recorder: &mut Recorder) {
        if let Some((epoch, pass, end)) = self.closing {
            if end == i64::MIN || self.proven(end) {
                recorder.pass_closed(epoch, pass, self.observation_time);
                self.closing = None;
            }
        }
    }

    pub fn finish(&mut self, recorder: &mut Recorder) {
        self.close_pass(recorder);
        // A concurrent GUI stop cannot close an epoch whose armed callback
        // may still open another run later in this same block.
        let epoch = self.captured_intent.epoch();
        if !self.captured_intent.is_armed()
            && self.run.is_none()
            && !self.owes_closure(recorder, epoch)
        {
            recorder.configuration_epoch_complete(epoch);
            recorder.source_epoch_complete(epoch, self.observation_time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harmonigraph_core::configuration::ConfigReducer;

    /// A run is stated on the Hub's own steady timeline, the one the frontiers
    /// prove, and the take's closure waits for both to pass what it recorded.
    #[test]
    fn a_run_on_the_adopted_hub_clock_closes_its_take_once_both_frontiers_pass_it() {
        let (mut recorder, mut capture) = harmonigraph_record::testing::channel();
        capture.arm();
        let intent = recorder.capture_recording_intent();
        recorder.is_armed_at(intent);
        let origin = recorder.observe_transport(20.0, true, 64.0 / 48_000.0);
        let clock = ClockId { runtime_session: 7, epoch: 3 };
        let mut recording = Recording {
            clock,
            prefix: 1040,
            block_start: 1000,
            block_frames: 40,
            ..Default::default()
        };
        recording.presentation = Presentation { anchor: Some((1000, 5.0)), rate: 48_000.0 };
        recording.begin(1000, intent);
        recording.record(&mut recorder, origin);
        recording.configuration(&mut recorder, 1000, ConfigReducer::default().resolved());
        recording.registered(1040);
        assert!(recording.recorded(1039) && !recording.recorded(999));
        let entries = capture.drain_entries();
        let Some(harmonigraph_record::Entry::Run { pass, at, song, presentation, .. }) =
            entries.first().copied()
        else {
            panic!("the first recorded block opens a run");
        };
        assert_eq!((pass, at.input, song, presentation), (1, 1000, 20.0, 5.0));
        // An input of this callback is stamped armed; a stale clock is not
        // stamped at all.
        assert!(recording.stamp(clock, 1010).unwrap().intent.is_armed());
        assert!(recording.stamp(ClockId { epoch: 4, ..clock }, 1010).is_err());

        capture.stop();
        let disarmed = recorder.capture_recording_intent();
        recording.begin(1040, disarmed);
        recording.observe_joined_disarm(&mut recorder);
        recording.source_frontier(clock, 1024).unwrap();
        assert!(recording.owes_closure(&recorder, 1), "the source has not passed the run");
        recording.source_frontier(clock, 1040).unwrap();
        assert!(!recording.owes_closure(&recorder, 1));
        assert!(recording.stamp(clock, 1030).is_err(), "below the proven frontier");
    }
}
