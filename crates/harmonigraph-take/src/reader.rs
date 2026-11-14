//! Reading one pass out of a take: routing, translation, the floor and the
//! warnings, as one pure function over the file's records in file order.
//!
//! The audio thread writes what it sees on the display's clock and says where
//! each input arrived. Everything that decides song time is here, over a
//! complete file, rather than in an ordering protocol between the audio thread
//! and the writer (#1760).
use std::collections::BTreeMap;

use crate::canonical::{GapReasonRecord, StampRecord};
use crate::{
    CanonicalRecord, ConfigurationRecord, Header, IncompleteRecord, ParamRecord, ReadError, Record,
    Take,
};

/// One run, as the file states it.
struct Run {
    pass: u32,
    start: StampRecord,
    /// The first sample past the run, once its `RunEnd` is read.
    end: Option<i64>,
    song: f64,
    presentation: f64,
    audio_file: Option<String>,
}

impl Run {
    /// Presentation plus this is song time, for every record the run holds.
    fn offset(&self) -> f64 {
        self.song - self.presentation
    }
    fn holds(&self, at: StampRecord) -> bool {
        at.clock() == self.start.clock()
            && self.start.input <= at.input
            && self.end.is_none_or(|end| at.input < end)
    }
    /// Whether the inputs `from..=to` overlap this run's span.
    fn touches(&self, from: StampRecord, to: StampRecord) -> bool {
        let epoch = self.start.clock_epoch;
        self.start.order() <= to.order() && self.end.is_none_or(|end| from.order() < (epoch, end))
    }
    /// Presentation time of `input` on the Hub's mapping the run was opened on.
    fn presentation(&self, input: i64, rate: f64) -> f64 {
        self.presentation + (input - self.start.input) as f64 / rate
    }
}

/// The running maximum of one source's song times in one pass, in file order,
/// and how far the record that set it sounded after its own input.
///
/// File order per source is lane order, which is the order `CanonicalOrder`
/// accepts, so a record that sorts below the maximum is lifted to it and the
/// stable sort keeps it in place. The lift is bounded: with song time taken
/// from the audio origin, a run's opening snapshot sits at or after the end of
/// the audio before it, and a delta played inside that audio sounds at most
/// its own lateness past it. Past the bound is a defect in the stamps, not a
/// pause; the record still lifts, so the file reads, and the pass warns.
#[derive(Default)]
struct Floor(BTreeMap<u64, (f64, f64)>);

impl Floor {
    /// Lift a record of `source` at `time` to the floor, and answer whether
    /// that broke the bound. Only its time moves: a lifted snapshot keeps its
    /// voices' onsets.
    fn lift(&mut self, source: u64, time: &mut f64, lateness: f64) -> bool {
        match self.0.get(&source) {
            Some(&(floor, bound)) if *time < floor => {
                let breach = floor - *time > bound;
                *time = floor;
                breach
            }
            _ => {
                self.0.insert(source, (*time, lateness));
                false
            }
        }
    }
}

pub(crate) fn assemble(
    header: Header,
    records: Vec<(usize, Record)>,
    truncated: bool,
    pass: Option<u32>,
) -> Result<Take, ReadError> {
    let mut runs: Vec<Run> = Vec::new();
    // The run open at each line, for parameters and configuration: they ride
    // the ring `Run` and `RunEnd` ride, so their place is their stamp.
    let mut placed = Vec::with_capacity(records.len());
    let mut open = None;
    for (_, record) in &records {
        match record {
            Record::Run(run) => {
                if let Some(index) = open.take() {
                    let previous: &mut Run = &mut runs[index];
                    if previous.start.clock() == run.at.clock() {
                        previous.end = Some(run.at.input);
                    }
                }
                runs.push(Run {
                    pass: run.pass,
                    start: run.at,
                    end: None,
                    song: run.song,
                    presentation: run.presentation,
                    audio_file: run.audio_file.clone(),
                });
                open = Some(runs.len() - 1);
            }
            Record::RunEnd(end) => {
                if let Some(index) = open.take() {
                    runs[index].end = Some(end.at.input);
                }
            }
            _ => {}
        }
        placed.push(open);
    }
    let mut take = Take { header, truncated, ..Take::default() };
    if runs.is_empty() {
        if pass.is_some_and(|asked| asked != 1) {
            return Err(ReadError::NoPass { asked: pass.unwrap_or(1), passes: vec![1] });
        }
        return unrouted(take, records);
    }
    take.passes = runs.iter().map(|run| run.pass).collect();
    take.passes.sort_unstable();
    take.passes.dedup();
    let chosen = match pass {
        None => *take.passes.last().expect("a run names its pass"),
        Some(asked) if take.passes.contains(&asked) => asked,
        Some(asked) => return Err(ReadError::NoPass { asked, passes: take.passes }),
    };
    take.pass = Some(chosen);
    let first = runs.iter().position(|run| run.pass == chosen).expect("the chosen pass has a run");
    take.audio_file = runs[first].audio_file.clone();
    take.audio_start = Some(runs[first].song);
    let rate = f64::from(take.header.sample_rate);

    let mut events = Vec::new();
    let mut floor = Floor::default();
    let mut closed_through = 0;
    let mut seed_params = BTreeMap::<String, ParamRecord>::new();
    let mut seed_configuration: Option<ConfigurationRecord> = None;
    let mut started = false;
    let mut run_index = 0;
    for ((line, record), placed) in records.into_iter().zip(placed) {
        let run = placed.map(|index| &runs[index]).filter(|run| run.pass == chosen);
        match record {
            Record::Run(_) => {
                started |= run_index == first;
                run_index += 1;
            }
            Record::PassClosed(pass) => closed_through = closed_through.max(pass),
            Record::Incomplete(marker) if chosen > closed_through => {
                take.incomplete.get_or_insert(marker);
            }
            Record::Param(mut param) => match run {
                Some(run) => {
                    param.t += run.offset();
                    take.params.push(param);
                }
                None if !started => {
                    seed_params.insert(param.id.clone(), param);
                }
                None => {}
            },
            Record::Configuration(mut configuration) => match run {
                Some(run) => {
                    configuration.t += run.offset();
                    take.configurations.push(configuration);
                }
                None if !started => seed_configuration = Some(configuration),
                None => {}
            },
            Record::Canonical(CanonicalRecord::Gap(mut gap)) => {
                // A gap marks the passes its inputs touch, and no other.
                let Some([from, to]) = gap.inputs else {
                    return Err(ReadError::InvalidCanonical(line));
                };
                let mut touched =
                    runs.iter().filter(|run| run.pass == chosen && run.touches(from, to));
                let Some(first) = touched.next() else { continue };
                let last = touched.next_back().unwrap_or(first);
                take.incomplete.get_or_insert(IncompleteRecord {
                    first_publication: gap.first,
                    last_publication: gap.last,
                    reason: gap.reason,
                });
                // Each end moves by the run it falls in, kept inside the runs
                // it touched, so a gap reaching into another pass lengthens
                // this one by nothing.
                let t = gap.t.max(first.presentation);
                let end = last.end.map_or(f64::INFINITY, |end| last.presentation(end, rate));
                let through = gap.through.min(end).max(t);
                gap.t = t + first.offset();
                gap.through = (through + last.offset()).max(gap.t);
                events.push((CanonicalRecord::Gap(gap), line));
            }
            Record::Canonical(mut record) => {
                // Nothing the recorder writes lacks a stamp, and in a file that
                // routes by them a record without one has no place.
                let at = record.stamp().ok_or(ReadError::InvalidCanonical(line))?;
                let Some(run) = runs.iter().find(|run| run.pass == chosen && run.holds(at)) else {
                    continue;
                };
                let lateness = record.time() - run.presentation(at.input, rate);
                record.translate(run.offset());
                let start = run.presentation + run.offset();
                let (source, time) = match &mut record {
                    CanonicalRecord::Delta(d) => (d.event.source, &mut d.event.t),
                    CanonicalRecord::Baseline(b) => {
                        // A voice struck where nothing recorded, in a pause or
                        // before the arm, is restated here; it draws from the
                        // run's start, not from a song time the run never had.
                        for voice in &mut b.voices {
                            voice.actual_onset = voice.actual_onset.max(start);
                            voice.input_onset = voice.input_onset.max(start);
                        }
                        (b.source, &mut b.t)
                    }
                    CanonicalRecord::Note(_) | CanonicalRecord::Gap(_) => continue,
                };
                if floor.lift(source, time, lateness) {
                    take.incomplete.get_or_insert(IncompleteRecord {
                        reason: GapReasonRecord::InvalidRecord,
                        ..IncompleteRecord::default()
                    });
                }
                events.push((record, line));
            }
            // The standalone's plain note carries no stamp, so it reads only
            // in a file with no run.
            Record::Note(_) => return Err(ReadError::InvalidCanonical(line)),
            Record::RunEnd(_) | Record::Header(_) | Record::Incomplete(_) => {}
        }
    }
    // A pass opens with what was in force before it: change detection runs per
    // arm, so a later pass writes only what changed.
    let start = runs[first].presentation + runs[first].offset();
    let seeds = seed_params.into_values().map(|param| ParamRecord { t: start, ..param });
    take.params.splice(0..0, seeds);
    if let Some(configuration) = seed_configuration {
        take.configurations.insert(0, ConfigurationRecord { t: start, ..configuration });
    }
    finish(take, events)
}

/// A file with no `Run`: one pass at offset zero. What carries a stamp was
/// played where no run recorded, an armed take whose transport never rolled,
/// and is dropped like any record in no run; the standalone's unstamped
/// history reads as written.
fn unrouted(mut take: Take, records: Vec<(usize, Record)>) -> Result<Take, ReadError> {
    take.passes = vec![1];
    take.pass = Some(1);
    let mut events = Vec::new();
    for (line, record) in records {
        match record {
            Record::Note(note) => events.push((CanonicalRecord::Note(note), line)),
            Record::Canonical(record)
                if record.stamp().is_some()
                    || matches!(&record, CanonicalRecord::Gap(gap) if gap.inputs.is_some()) => {}
            Record::Canonical(record) => {
                if let CanonicalRecord::Gap(gap) = &record {
                    take.incomplete.get_or_insert(IncompleteRecord {
                        first_publication: gap.first,
                        last_publication: gap.last,
                        reason: gap.reason,
                    });
                }
                events.push((record, line));
            }
            Record::Incomplete(marker) => {
                take.incomplete.get_or_insert(marker);
            }
            Record::Param(param) => take.params.push(param),
            Record::Configuration(configuration) => take.configurations.push(configuration),
            Record::Header(_) | Record::Run(_) | Record::RunEnd(_) | Record::PassClosed(_) => {}
        }
    }
    finish(take, events)
}

/// Sort by time, and refuse what no stable sort can make a valid history.
///
/// Stable, so simultaneous events keep the order they were played in
/// (note-off before the note-on that replaces it, and so on), and a record the
/// floor lifted level with another stays behind it.
fn finish(mut take: Take, mut events: Vec<(CanonicalRecord, usize)>) -> Result<Take, ReadError> {
    events.sort_by(|(a, _), (b, _)| a.time().total_cmp(&b.time()));
    let mut validation = harmonigraph_core::canonical::CanonicalOrder::default();
    for (record, line) in events {
        record.check_order(&mut validation).map_err(|_| ReadError::InvalidCanonical(line))?;
        take.events.push(record);
    }
    take.params.sort_by(|a, b| a.t.total_cmp(&b.t));
    take.configurations.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(take)
}

#[cfg(test)]
mod tests;
