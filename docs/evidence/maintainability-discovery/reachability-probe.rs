// Discovery probe: fixture snapshots, not a live Hub/Bitwig measurement.
// See README.md for source revision, commands and interpretation.
use harmonigraph_core::{
    configuration::{ConfigReducer, TuningModes},
    policy::{self, reach::Snapshot, ContextPitch, MusicalConfig, PolicyScratch},
    LatticePos, Tempered, Tuning,
};

fn voice(pitch: i64, t: i32, f: i32, s: i32) -> ContextPitch {
    ContextPitch { pitch, node: Some(LatticePos::new(t, f, s)), weight: 1.0 }
}

fn main() {
    let config: MusicalConfig = ConfigReducer::new(
        Tuning::just(),
        TuningModes {
            tempered: Tempered { syntonic: false, septimal_kleisma: false },
            auto: [false; 2],
            learning: false,
        },
    )
    .resolved()
    .into();
    let cases = [
        (
            "minor-before-bb",
            0,
            vec![
                voice(4_800_000_000, 0, 0, 0),
                voice(5_115_641_287, 1, -1, 0),
                voice(5_501_955_001, 1, 0, 0),
            ],
        ),
        (
            "fifths-high-before-e",
            5_865_003,
            vec![
                voice(4_800_000_000, 0, 0, 0),
                voice(5_501_955_001, 1, 0, 0),
                voice(6_203_910_002, 2, 0, 0),
                voice(6_905_865_003, 3, 0, 0),
            ],
        ),
    ];
    for (name, reference, context) in cases {
        let mut scratch = PolicyScratch::default();
        policy::prepare(config, &context, &mut scratch).unwrap();
        let snapshot = Snapshot { config, reference, context };
        let winners = policy::reach::reachable(&snapshot, 3600.0, 9600.0, || false).unwrap();
        assert!(winners.iter().all(|n| scratch.candidates.contains(n)));
        let extras: Vec<_> = scratch
            .candidates
            .iter()
            .filter(|n| !winners.contains(n))
            .map(|n| (n.threes, n.fives, n.sevens))
            .collect();
        println!(
            "{name}: eligible={} winners={} extras={} examples={:?}",
            scratch.candidates.len(),
            winners.len(),
            extras.len(),
            &extras[..extras.len().min(10)]
        );
    }
}
