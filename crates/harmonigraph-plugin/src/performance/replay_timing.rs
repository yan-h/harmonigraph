//! Called only by the existing excluded timing test. Rehearsals assert reach
//! without publishing elapsed-time results; neither profile establishes WCET.
//! Exported callback durations include the preallocated test host's accepted
//! and rejected event bookkeeping, but exclude snapshots and capture draining.
use super::*;
use harmonigraph_take::{CanonicalRecord, NoteKind};

const FRAMES: u32 = 512;
const PRELUDE: [[u8; 3]; 13] = [
    [0xb0, 99, 1],
    [0xb0, 98, 2],
    [0xb0, 6, 3],
    [0xb0, 38, 4],
    [0xb0, 96, 0],
    [0xb0, 97, 0],
    [0xb0, 101, 127],
    [0xb0, 100, 127],
    [0xb0, 0, 1],
    [0xb0, 32, 2],
    [0xc0, 3, 0],
    [0xb0, 0, 4],
    [0xb0, 32, 5],
];
const GESTURES: [(u32, [u8; 3]); 6] = [
    (20, [0xb0, 7, 91]),
    (24, [0xe0, 64, 64]),
    (28, [0xd0, 72, 0]),
    (32, [0xb0, 66, 127]),
    (40, [0x80, 64, 3]),
    (60, [0xb0, 66, 0]),
];

struct Observation {
    variant: &'static str,
    phase: &'static str,
    nanos: [u128; 17],
    attempts: [usize; 17],
    accepted: [usize; 17],
}

struct Episode {
    sources: Vec<Device>,
    hub: Device,
    capture: harmonigraph_record::testing::Capture,
    records: Vec<CanonicalRecord>,
    session: std::sync::Arc<protocol::SessionControl>,
    block: i64,
    variant: &'static str,
    peak: [usize; 5],
}

impl Episode {
    fn new(variant: &'static str) -> Self {
        let uuid = SavedUuid::default();
        let calibration =
            Calibration { offset: 0, sample_rate: 44100.0, max_frames: FRAMES, validated: true };
        let (mut hub, capture) = Device::recorded_aggregation_hub();
        hub.configure_format(uuid, true, calibration);
        hub.activate_format(44100.0, FRAMES);
        let sources = (0..16)
            .map(|index| {
                let mut source = Device::aggregation(true);
                source.configure_format(uuid, index != 0, calibration);
                source.activate_format(44100.0, FRAMES);
                source
            })
            .collect();
        let session = registry::global().lock().unwrap().test_session(uuid);
        capture.arm();
        Self {
            sources,
            hub,
            capture,
            records: Vec::new(),
            session,
            block: 0,
            variant,
            peak: [0; 5],
        }
    }

    // peer_action: 1 starts three64-voice peers, -1 releases the first,
    // -2 releases the remaining two. Twelve additional Tunes stay enrolled.
    fn round(
        &mut self,
        phase: &'static str,
        target: Vec<Input>,
        peer_action: i8,
        direct: Vec<Input>,
        observations: &mut Vec<Observation>,
    ) -> Sink {
        let raw = self.block * i64::from(FRAMES);
        let mut sample = Observation {
            variant: self.variant,
            phase,
            nanos: [0; 17],
            attempts: [0; 17],
            accepted: [0; 17],
        };
        let output = self.sources[0].run_format(raw, target, None, None, FRAMES);
        sample.nanos[0] = output.callback_nanos;
        sample.attempts[0] = output.attempts;
        sample.accepted[0] = output.values.len();
        let state = self.sources[0].source_snapshot();
        for (peak, now) in self.peak.iter_mut().zip([
            state.pending,
            state.references,
            state.journal,
            state.emergency,
            state.held,
        ]) {
            *peak = (*peak).max(now);
        }
        for (index, source) in self.sources.iter().enumerate().skip(1) {
            let on = peer_action == 1 && index <= 3;
            let off =
                peer_action == -1 && index == 1 || peer_action == -2 && (2..=3).contains(&index);
            let events = if on || off {
                (0..64).map(|key| note(key + 1, 0, key as i16, 0, on)).collect()
            } else {
                vec![]
            };
            let peer = source.run_format(raw, events, None, None, FRAMES);
            sample.nanos[index] = peer.callback_nanos;
            sample.attempts[index] = peer.attempts;
            sample.accepted[index] = peer.values.len();
            if on || off {
                assert_eq!(
                    peer.values
                        .iter()
                        .filter(|(_, event)| {
                            if on {
                                event.attack().is_some()
                            } else {
                                event.release()
                            }
                        })
                        .count(),
                    64,
                    "{phase} Source{index} must retain every real voice output"
                );
            }
        }
        let hub = self.hub.run_format(raw, direct, None, None, FRAMES);
        sample.nanos[16] = hub.callback_nanos;
        sample.attempts[16] = hub.attempts;
        sample.accepted[16] = hub.values.len();
        self.records.extend(self.capture.drain_canonical());
        observations.push(sample);
        self.block += 1;
        output
    }

    fn empty(&mut self, phase: &'static str, observations: &mut Vec<Observation>) -> Sink {
        self.round(phase, vec![], 0, vec![], observations)
    }

    fn target_lease(&self) -> protocol::Lease {
        let wrapper = unsafe {
            &*((*self.sources[0].plugin)
                .plugin_data
                .cast::<nice_plug::wrapper::clap::Wrapper<tune::HarmonigraphTune>>())
        };
        wrapper.test_inspect_plugin(|plugin| {
            plugin.source.as_ref().unwrap().test_stream_status().0.unwrap()
        })
    }

    fn finish(mut self, observations: &mut Vec<Observation>) {
        for source in &self.sources {
            let shared = source.shared();
            shared.apply(shared.value().routing, true).unwrap();
        }
        let direct = (0..63).map(|key| note(key + 1, 0, key as i16, 0, false)).collect();
        self.round("cleanup", vec![], -2, direct, observations);
        for _ in 0..15 {
            self.empty("cleanup", observations);
        }
        assert_eq!(self.session.credits.load(Ordering::Acquire), 0);
        let mut history = [0; 2];
        for source in &self.sources {
            let snapshot = source.source_snapshot();
            assert_eq!((snapshot.journal, snapshot.emergency, snapshot.held), (0, 0, 0));
            history[0] += snapshot.pending;
            history[1] += snapshot.references;
        }
        // Reconstructable transaction history deliberately survives musical
        // idle/Reset. Actual producer retirement releases those remaining pins.
        self.capture.stop();
        drop(self.sources);
        drop(self.hub);
        assert_eq!(registry::global().lock().unwrap().test_counts(), (0, 0, 0));
        println!("REPLAY_CLEANUP {} before_retirement_pending_refs={history:?} retained_output=0 credits=0 after_retirement_registry=0", self.variant);
    }
}

fn raw(data: [u8; 3], time: u32) -> Input {
    midi(0, data[0], data[1], data[2], time)
}
fn event(data: [u8; 3]) -> Event {
    Event::Midi { port: 0, data, flags: 0 }
}

fn episode(reject: bool, observations: &mut Vec<Observation>) {
    let variant = if reject { "reject-repair" } else { "success" };
    let mut fixture = Episode::new(variant);
    fixture.empty("enrollment", observations);
    let mut seed = known_seed();
    seed.extend(PRELUDE.map(|data| raw(data, 3)));
    seed.extend([raw([0xb0, 88, 37], 3), raw([0x90, 60, 64], 4)]);
    let initial = fixture.round(
        "onset256",
        seed,
        1,
        (0..63).map(|key| note(key + 1, 0, key as i16, 0, true)).collect(),
        observations,
    );
    assert_eq!(initial.values.len(), 19);
    assert_eq!(initial.values.last(), Some(&(4, event([0x90, 60, 64]))));
    assert_eq!(fixture.sources[0].source_snapshot().velocity_prefix[0], Some(0));
    assert_eq!(fixture.session.credits.load(Ordering::Acquire), 256);
    let lease = fixture.target_lease();
    let mut expected_setup = vec![[0xb0, 7, 40], [0xb0, 64, 0], [0xb0, 66, 0], [0xb0, 69, 0]];
    expected_setup.extend(PRELUDE);
    for _ in 2..=44 {
        let headers: Vec<_> = (0..48).map(|value| [0xb0, 7, value]).collect();
        let output = fixture.round(
            "history",
            headers.iter().map(|data| raw(*data, 0)).collect(),
            0,
            vec![],
            observations,
        );
        assert_eq!(output.values, headers.iter().map(|data| (0, event(*data))).collect::<Vec<_>>());
        expected_setup.extend(headers);
    }
    assert_eq!(expected_setup.len(), 2081);
    assert!(fixture.sources[0].source_snapshot().pending > 2048);
    let prefix = fixture.round("prefix", vec![raw([0xb0, 88, 55], 8)], 0, vec![], observations);
    assert_eq!(prefix.values, [(8, event([0xb0, 88, 55]))]);
    let input_on = fixture.block * i64::from(FRAMES) + 4;
    let mut young = vec![raw([0x90, 64, 88], 4)];
    young.extend(GESTURES.map(|(time, data)| raw(data, time)));
    let old = fixture.round("younger-capture", young, 0, vec![], observations);
    assert_eq!(
        old.values,
        GESTURES
            .iter()
            .filter(|(_, data)| data[0] != 0x80)
            .map(|(time, data)| (*time, event(*data)))
            .collect::<Vec<_>>()
    );
    let release = fixture.round(
        "old-release",
        vec![raw([0xa0, 60, 73], 8), raw([0x80, 60, 2], 16)],
        -1,
        vec![],
        observations,
    );
    assert_eq!(
        release.values,
        [(8, event([0xa0, 60, 73])), (16, event([0xb0, 88, 0])), (16, event([0x80, 60, 2]))]
    );

    let mut replay = Vec::new();
    let mut setup_count = 0;
    let mut max_setup_attempts = 0;
    let mut rejected_attempts = 0;
    let mut rejected_wire = Vec::new();
    let mut attempted_rejection = false;
    let mut handed_off = false;
    for _ in 0..16 {
        let remaining = expected_setup.len() - setup_count;
        let handoff = remaining < 500;
        if reject && handoff && !attempted_rejection {
            let mut acceptance = vec![true; remaining + 4];
            acceptance[remaining + 1] = false; // exact prefix is accepted; B rejects
            acceptance[remaining + 2] = false; // first actual neutral repair rejects
            ACCEPTANCE_SCRIPT.with(|script| *script.borrow_mut() = acceptance);
            attempted_rejection = true;
        }
        let raw_start = fixture.block * i64::from(FRAMES);
        let output = fixture.empty(
            if handoff {
                if reject {
                    "rejection-repair"
                } else {
                    "handoff"
                }
            } else {
                "setup"
            },
            observations,
        );
        max_setup_attempts = max_setup_attempts.max(output.attempts);
        rejected_attempts += output.attempts - output.values.len();
        rejected_wire.extend(
            output.rejected.iter().map(|(time, value)| (raw_start + i64::from(*time), *value)),
        );
        for (time, value) in output.values {
            if setup_count < expected_setup.len() {
                assert_eq!(
                    value,
                    event(expected_setup[setup_count]),
                    "exact setup offset{setup_count}"
                );
                setup_count += 1;
            } else {
                replay.push((raw_start + i64::from(time), value));
            }
        }
        handed_off = if reject {
            fixture.sources[0].source_snapshot().faults != 0
        } else {
            replay.iter().any(|(_, value)| *value == event([0x80, 64, 3]))
        };
        if handed_off {
            break;
        }
    }
    assert!(handed_off);
    assert_eq!(setup_count, 2081);
    assert_eq!(max_setup_attempts, 512);
    if reject {
        assert!(attempted_rejection);
        assert_eq!(rejected_attempts, 2);
        assert_eq!(replay.len(), 2);
        assert_eq!(replay[0].1, event([0xb0, 88, 55]));
        assert_eq!(replay[1].1, event([0xb0, 88, 0]));
        assert_eq!(
            rejected_wire.iter().map(|(_, value)| *value).collect::<Vec<_>>(),
            [event([0x90, 64, 88]), event([0xb0, 88, 0])]
        );
        assert_eq!(rejected_wire[0].0, replay[0].0);
        assert!(replay[1].0 >= replay[0].0);
        assert_eq!(fixture.sources[0].source_snapshot().held, 0);
        assert_eq!(fixture.sources[0].source_snapshot().faults, source::OUTPUT_FAULT);
    } else {
        let on = replay[1].0;
        assert!(on > input_on);
        let mut expected = vec![(on, event([0xb0, 88, 55])), (on, event([0x90, 64, 88]))];
        expected.extend(GESTURES.map(|(time, data)| (on + i64::from(time) - 4, event(data))));
        assert_eq!(replay, expected);
        assert_eq!(rejected_attempts, 0);
    }
    for _ in 0..8 {
        fixture.empty("reporting", observations);
    }
    let snapshot = fixture.sources[0].source_snapshot();
    let (prefix, held, received, applied) = receiver(&fixture.hub, usize::from(lease.slot - 1));
    assert_eq!(
        (prefix, held, received, applied),
        (Some(0), 0, snapshot.sequence, snapshot.sequence)
    );
    assert_eq!(snapshot.acknowledged, snapshot.sequence);
    assert_eq!((snapshot.journal, snapshot.emergency, snapshot.held), (0, 0, 0));
    let canonical: Vec<_> = fixture
        .records
        .iter()
        .filter_map(|record| match record {
            CanonicalRecord::Delta(delta)
                if delta.event.source == lease.source.0
                    && delta.event.note == 64
                    && matches!(delta.event.kind, NoteKind::On { .. } | NoteKind::Off) =>
            {
                Some((delta.timing.unwrap().sample, delta.event.kind))
            }
            _ => None,
        })
        .collect();
    if reject {
        assert!(canonical.is_empty());
    } else {
        assert_eq!(canonical.len(), 2);
        assert!(matches!(canonical[0].1, NoteKind::On { .. }));
        assert_eq!(canonical[0].0, replay[1].0);
        assert_eq!(canonical[1], (replay[1].0 + 36, NoteKind::Off));
    }
    println!("REPLAY_REACH {variant} history=2064 setup={setup_count} max_setup_attempts={max_setup_attempts} rejected_attempts={rejected_attempts} peak_pending_refs_journal_emergency_held={:?} retained_cut={} acknowledged={} canonical_young={} owners=16+Hub rate=44100 frames=512", fixture.peak, snapshot.sequence, snapshot.acknowledged, canonical.len());
    fixture.finish(observations);
}

pub(in super::super) fn observe_replay_callbacks(rehearsal: bool) {
    let mut observations = Vec::new();
    for reject in [false, true] {
        for _ in 0..if rehearsal { 1 } else { 16 } {
            episode(reject, &mut observations);
        }
    }
    if rehearsal {
        println!("REPLAY_REHEARSAL success=1 rejection_repair=1 registry=0 elapsed_results_suppressed=true");
        return;
    }
    for variant in ["success", "reject-repair"] {
        for phase in [
            "enrollment",
            "onset256",
            "history",
            "prefix",
            "younger-capture",
            "old-release",
            "setup",
            "handoff",
            "rejection-repair",
            "reporting",
            "cleanup",
        ] {
            let samples: Vec<_> =
                observations.iter().filter(|v| v.variant == variant && v.phase == phase).collect();
            if samples.is_empty() {
                continue;
            }
            for owner in 0..18 {
                let mut times: Vec<_> =
                    samples
                        .iter()
                        .map(|sample| {
                            if owner == 17 {
                                sample.nanos.iter().sum()
                            } else {
                                sample.nanos[owner]
                            }
                        })
                        .collect();
                times.sort_unstable();
                let mean = times.iter().sum::<u128>() as f64 / times.len() as f64;
                let attempts: usize = samples
                    .iter()
                    .map(|sample| {
                        if owner == 17 {
                            sample.attempts.iter().sum()
                        } else {
                            sample.attempts[owner]
                        }
                    })
                    .sum();
                let accepted: usize = samples
                    .iter()
                    .map(|sample| {
                        if owner == 17 {
                            sample.accepted.iter().sum()
                        } else {
                            sample.accepted[owner]
                        }
                    })
                    .sum();
                let name = match owner {
                    16 => "Hub".into(),
                    17 => "sum16Sources+Hub".into(),
                    index => format!("Source{index:02}"),
                };
                println!("CALLBACK replay/{variant}/{phase}/{name} n={} mean_ns={mean:.0} p50_ns={} p95_ns={} max_ns={} attempts={attempts} accepted={accepted} debug_assertions={}", times.len(), times[times.len()/2], times[times.len()*95/100], times[times.len()-1], cfg!(debug_assertions));
            }
        }
    }
    println!("CALLBACK replay scope=targeted_known_state elapsed_sum_excludes_host_scheduling_jitter=true full616D512_validation=false WCET=false");
}
