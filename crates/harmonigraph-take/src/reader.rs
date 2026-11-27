//! Reading a take: every record goes to the run its own time falls in, and
//! moves by that run's one offset into song time.
//!
//! The audio thread writes what it sees on the display's clock, each record
//! timed where it sounds. A `Run` says where a stretch the take recorded
//! without a pause sits on that clock and in the take's audio, so placing a
//! record is a lookup of its own time, with no ordering between the records
//! and nothing kept from one to the next (#1760).

use harmonigraph_core::canonical::NoteDelta;
use harmonigraph_core::NoteEvent;

use crate::{CanonicalRecord, Header, IncompleteRecord, ReadError, Record, Take};

/// One run, as the file states it: `[start, end)` on the presentation clock,
/// and the offset that moves what it holds into song time.
struct Run {
    start: f64,
    end: f64,
    offset: f64,
}

impl Run {
    fn holds(&self, time: f64) -> bool {
        self.start <= time && time < self.end
    }
}

/// The run `time` falls in.
fn holding(runs: &[Run], time: f64) -> Option<&Run> {
    runs.iter().find(|run| run.holds(time))
}

pub(crate) fn assemble(
    header: Header,
    records: Vec<(usize, Record)>,
    truncated: bool,
) -> Result<Take, ReadError> {
    let mut runs: Vec<Run> = Vec::new();
    let mut audio_start = None;
    for (_, record) in &records {
        match record {
            Record::Run(run) => {
                if let Some(open) = runs.last_mut() {
                    open.end = open.end.min(run.at);
                }
                audio_start.get_or_insert(run.song);
                runs.push(Run { start: run.at, end: f64::INFINITY, offset: run.song - run.at });
            }
            Record::RunEnd(end) => {
                if let Some(open) = runs.last_mut() {
                    open.end = open.end.min(end.at);
                }
            }
            _ => {}
        }
    }
    let audio_file = header.audio_file.clone();
    let mut take = Take { header, truncated, audio_file, audio_start, ..Take::default() };

    let mut events = Vec::new();
    for (line, record) in records {
        match record {
            Record::Incomplete(marker) => {
                take.incomplete.get_or_insert(marker);
            }
            Record::Param(mut param) => {
                if let Some(run) = holding(&runs, param.t) {
                    param.t += run.offset;
                    take.params.push(param);
                }
            }
            Record::Configuration(mut configuration) => {
                if let Some(run) = holding(&runs, configuration.t) {
                    configuration.t += run.offset;
                    take.configurations.push(configuration);
                }
            }
            Record::Canonical(CanonicalRecord::Gap(mut gap)) => {
                // A loss warns if it overlaps what the take recorded. It is
                // drawn inside the first run it overlaps, from no earlier than
                // that run's start to no later than its end.
                let Some(run) = runs.iter().find(|run| run.start <= gap.through && gap.t < run.end)
                else {
                    continue;
                };
                take.incomplete.get_or_insert(IncompleteRecord {
                    first_publication: gap.first,
                    last_publication: gap.last,
                    reason: crate::canonical::GapReasonRecord::PublicationFull,
                });
                gap.t = gap.t.max(run.start) + run.offset;
                gap.through = (gap.through.min(run.end) + run.offset).max(gap.t);
                events.push((CanonicalRecord::Gap(gap), line));
            }
            Record::Canonical(CanonicalRecord::Delta(mut delta)) => {
                if let Some(run) = holding(&runs, delta.event.t) {
                    delta.event.t += run.offset;
                    events.push((CanonicalRecord::Delta(delta), line));
                }
            }
            Record::Canonical(CanonicalRecord::Baseline(mut frame)) => {
                let Some(run) = holding(&runs, frame.t) else { continue };
                frame.t += run.offset;
                // A restated voice moves by the run its onset sounded in, as
                // its attack did. One that began where nothing recorded (in a
                // pause, or before the arm) starts with this frame's run.
                for voice in &mut frame.voices {
                    let shift = match holding(&runs, voice.actual_onset) {
                        Some(onset) => onset.offset,
                        None => run.start + run.offset - voice.actual_onset,
                    };
                    voice.actual_onset += shift;
                }
                events.push((CanonicalRecord::Baseline(frame), line));
            }
            Record::RunEnd(_) | Record::Header(_) | Record::Run(_) => {}
        }
    }
    let end = runs.last().filter(|run| run.end.is_finite()).map(|run| run.end + run.offset);
    finish(take, events, end)
}

/// Sort by time, and refuse what no stable sort can make a valid history: a
/// take whose records break their source's order (`CanonicalOrder`), as one
/// recorded before #1885 did when a Tune's delay was lowered while it recorded.
///
/// Stable, so simultaneous events keep the order they were played in
/// (note-off before the note-on that replaces it, and so on). Within a source
/// the file holds its records in the order they sound, and a snapshot is timed
/// at the latest record its cut covers, so the sort keeps a snapshot after
/// what it covers.
///
/// `end` is the song time the take's last run ended at, when the file says it
/// did. If a shown source still holds a note there, everything held is
/// released at it, as a session reset releases it: a release that sounded
/// after the last block that recorded,
/// as a host's do when the transport stops, was dropped, and without this a
/// Tail would draw its note held to the last frame (#1803). A take with
/// nothing held at its end gains no record, so its length is unchanged.
fn finish(
    mut take: Take,
    mut events: Vec<(CanonicalRecord, usize)>,
    end: Option<f64>,
) -> Result<Take, ReadError> {
    events.sort_by(|(a, _), (b, _)| a.time().total_cmp(&b.time()));
    let mut order = harmonigraph_core::canonical::CanonicalOrder::default();
    let mut held = harmonigraph_core::NoteTracker::new();
    for (record, line) in events {
        if !record.ordered(&mut order) {
            return Err(ReadError::OutOfOrder(line));
        }
        record.apply(&mut held).map_err(|_| ReadError::InvalidCanonical(line))?;
        take.events.push(record);
    }
    if let Some(end) = end.filter(|_| held.held_count() > 0) {
        let release = NoteDelta::from(NoteEvent::session_reset(end));
        take.events.push(CanonicalRecord::Delta(release.into()));
    }
    take.params.sort_by(|a, b| a.t.total_cmp(&b.t));
    take.configurations.sort_by(|a, b| a.t.total_cmp(&b.t));
    Ok(take)
}

#[cfg(test)]
mod tests;
