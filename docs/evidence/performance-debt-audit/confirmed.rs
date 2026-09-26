extern crate self as nice_plug;
pub mod wrapper {
    pub mod clap {
        pub mod configuration {
            #[derive(Clone, Copy)]
            pub enum InputValue {
                Note {
                    kind: u16,
                    note_id: i32,
                    port: i16,
                    channel: i16,
                    key: i16,
                    velocity: f64,
                    flags: u32,
                },
                Expression {
                    expression: i32,
                    note_id: i32,
                    port: i16,
                    channel: i16,
                    key: i16,
                    value: f64,
                    flags: u32,
                },
                Midi {
                    port: u16,
                    data: [u8; 3],
                    flags: u32,
                },
                Other,
            }
        }
    }
}
#[path = "@REPO@/crates/harmonigraph-plugin/src/tuning/event.rs"]
mod event;
#[path = "@REPO@/crates/harmonigraph-plugin/src/tuning/state.rs"]
mod state;
use harmonigraph_core::{
    canonical::EventTiming,
    confirmed::{ConfirmedPitches, LearningState},
    SourceId,
};
use std::{hint::black_box, time::Instant};
fn populate(n: usize, source: SourceId) -> state::State {
    let mut s = state::State::default();
    for i in 0..n {
        s.apply(
            event::Event::Note {
                kind: 0,
                id: i as i32,
                port: 0,
                channel: 0,
                key: i as i16,
                velocity: 0.8,
                flags: 0,
            },
            state::Stamp {
                source,
                sequence: i as u64 + 1,
                lifetime: i as u64 + 1,
                time: 0.,
                input_time: 0.,
                timing: EventTiming {
                    clock: Default::default(),
                    input: 0,
                    planned: None,
                    sample: 0,
                    sample_rate: 48000.,
                },
            },
        );
    }
    s
}
extern "C" {
    fn clock() -> std::os::raw::c_ulong;
}
fn main() {
    capacity_turnover();
    let mut full = populate(64, SourceId(1));
    let mut confirmed = ConfirmedPitches::default();
    full.publish_confirmed(SourceId(1), full.complete, &mut confirmed);
    let good = populate(3, SourceId(2));
    good.publish_confirmed(SourceId(2), good.complete, &mut confirmed);
    println!(
        "before invalid: complete={} confirmed={}",
        confirmed.is_complete(),
        confirmed.rows().count()
    );
    full.apply(
        event::Event::Expression {
            kind: 2,
            id: 0,
            port: 0,
            channel: 0,
            key: 0,
            value: f64::MAX,
            flags: 0,
        },
        state::Stamp {
            source: SourceId(1),
            sequence: 65,
            lifetime: 1,
            time: 1.,
            input_time: 1.,
            timing: EventTiming {
                clock: Default::default(),
                input: 0,
                planned: None,
                sample: 0,
                sample_rate: 48000.,
            },
        },
    );
    full.publish_confirmed(SourceId(1), full.complete, &mut confirmed);
    println!(
        "after invalid: state.complete={} confirmed.complete={} confirmed={} infer={:?}",
        full.complete,
        confirmed.is_complete(),
        confirmed.rows().count(),
        LearningState::default().infer(&confirmed, true)
    );
    for (sources, voices, deltas, iterations) in [
        (1, 0, 0, 10000),
        (1, 8, 0, 10000),
        (1, 8, 8, 1000),
        (1, 10, 8, 1000),
        (1, 10, 32, 1000),
        (1, 16, 8, 1000),
        (1, 16, 32, 1000),
        (4, 16, 64, 100),
        (4, 64, 0, 100),
        (4, 64, 256, 10),
        (4, 64, 2048, 3),
    ] {
        let states: Vec<_> = (0..sources)
            .map(|s| populate(voices, SourceId(s as u64 + 1)))
            .collect();
        let empty = state::State::default();
        let mut confirmed = ConfirmedPitches::default();
        let start = Instant::now();
        let cpu = unsafe { clock() };
        for _ in 0..iterations {
            for s in 0..17 {
                let state = states.get(s).unwrap_or(&empty);
                black_box(state.publish_confirmed(
                    SourceId(s as u64 + 1),
                    true,
                    black_box(&mut confirmed),
                ));
            }
            for d in 0..deltas {
                let s = d % sources;
                black_box(states[s].publish_confirmed(
                    SourceId(s as u64 + 1),
                    true,
                    black_box(&mut confirmed),
                ));
            }
        }
        println!(
            "confirm sources={sources} voices/source={voices} deltas={deltas}: {:.2} us/callback",
            start.elapsed().as_secs_f64() * 1e6 / iterations as f64
        );
        println!(
            "cpu {:.2} us/callback",
            (unsafe { clock() } - cpu) as f64 / iterations as f64
        );
    }
}

fn capacity_turnover() {
    use std::collections::BTreeSet;
    fn keys(store: &ConfirmedPitches) -> BTreeSet<(u64, u8, u8)> {
        store
            .rows()
            .map(|v| (v.key.source.0, v.key.channel, v.key.note))
            .collect()
    }
    fn expected(sources: std::ops::RangeInclusive<u64>) -> BTreeSet<(u64, u8, u8)> {
        sources
            .flat_map(|source| (0..64).map(move |note| (source, 0, note)))
            .collect()
    }
    fn refresh(states: &[state::State], store: &mut ConfirmedPitches) {
        for (i, state) in states.iter().enumerate() {
            state.publish_confirmed(SourceId(i as u64 + 1), state.complete, store);
        }
    }
    let mut states: Vec<_> = (1..=17)
        .map(|source| {
            populate(
                if (2..=5).contains(&source) { 64 } else { 0 },
                SourceId(source),
            )
        })
        .collect();
    let seed = || {
        let mut store = ConfirmedPitches::default();
        refresh(&states, &mut store);
        assert!(store.is_complete());
        assert_eq!(keys(&store), expected(2..=5));
        store
    };
    let (mut retained, mut rebuilt, mut current) = (seed(), seed(), seed());
    // Source 5 releases all its notes before source 1's onsets. Hub sequencing
    // finishes before flush, so each per-delta confirmation sees final State.
    for note in 0..64 {
        assert!(states[4]
            .apply(
                event::Event::note_off(note, 0, note as u8, false),
                state::Stamp {
                    source: SourceId(5),
                    sequence: 65 + note as u64,
                    lifetime: note as u64 + 1,
                    time: 1.0,
                    input_time: 1.0,
                    timing: EventTiming {
                        clock: Default::default(),
                        input: 48_000,
                        planned: None,
                        sample: 48_000,
                        sample_rate: 48_000.0
                    },
                },
            )
            .is_some());
    }
    states[0] = populate(64, SourceId(1));
    assert!(states.iter().all(|state| state.complete));

    // Proposed removal alone: begin publishes the earlier row against stale
    // later-row occupancy, and does not retry it after clearing that later row.
    refresh(&states, &mut retained);
    assert!(!retained.is_complete());
    assert_eq!(keys(&retained), expected(2..=4));
    println!("turnover retained begin: complete=false voices=192 exact_sources=2,3,4");

    rebuilt.reset();
    refresh(&states, &mut rebuilt);
    assert!(rebuilt.is_complete());
    assert_eq!(keys(&rebuilt), expected(1..=4));
    println!("turnover reset then begin: complete=true voices=256 exact_sources=1,2,3,4");

    // Current release-first flush: 64 release deltas from source 5, then
    // 64 onset deltas from source 1; every publication sees its final row.
    for index in [4, 0] {
        for _ in 0..64 {
            assert!(states[index].publish_confirmed(
                SourceId(index as u64 + 1),
                states[index].complete,
                &mut current,
            ));
        }
    }
    refresh(&states, &mut current);
    assert!(current.is_complete());
    assert_eq!(keys(&current), expected(1..=4));
    assert_eq!(keys(&current), keys(&rebuilt));
    println!("turnover current release-first flush then begin: complete=true voices=256 exact_sources=1,2,3,4");
}
