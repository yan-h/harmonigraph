use super::*;
use crate::configuration::{ConfigReducer, TuningModes};
use crate::Tuning;

fn just() -> MusicalConfig {
    ConfigReducer::new(
        Tuning::from_cents(
            0.0,
            crate::tuning::THREE_JUST,
            crate::tuning::FIVE_JUST,
            crate::tuning::SEVEN_JUST,
            0.0,
        ),
        TuningModes {
            tempered: Tempered { syntonic: false, septimal_kleisma: false },
            auto: [false; 2],
            learning: false,
        },
    )
    .resolved()
    .into()
}
struct Harness {
    config: MusicalConfig,
    memory: Memory,
    held: Vec<(String, ContextPitch)>,
    scratch: PolicyScratch,
}
impl Harness {
    fn new() -> Self {
        Self {
            config: just(),
            memory: Memory::default(),
            held: Vec::new(),
            scratch: PolicyScratch::default(),
        }
    }
    fn on(&mut self, id: &str, pitch: i64) -> ContextPitch {
        let mut context: Vec<_> = self.held.iter().rev().map(|(_, v)| *v).collect();
        self.memory.append(&mut context, self.config.policy);
        let d = assign_new_note(
            self.config,
            &context,
            &self.memory,
            OrderedOnset { pitch },
            &mut self.scratch,
        )
        .unwrap();
        let v = ContextPitch {
            pitch: pitch + d.assignment.correction_microcents(),
            node: d.assignment.node(),
            weight: 1.0,
        };
        self.memory.attack(pitch, d.assignment, self.config.policy);
        self.held.push((id.into(), v));
        v
    }
    fn off(&mut self, id: &str) {
        while let Some(i) = self.held.iter().position(|(name, _)| id == "*" || name == id) {
            let (_, v) = self.held.remove(i);
            self.memory.release(v, 1, self.config.policy);
        }
    }
}
#[test]
fn simulator_fixture_parity_includes_register_memory_and_precision() {
    let mut h = Harness::new();
    let mut case = "";
    for line in include_str!("fixtures.txt").lines().filter(|l| !l.starts_with('#')) {
        let w: Vec<_> = line.split_whitespace().collect();
        let num = |i: usize| w[i].parse::<i64>().unwrap();
        match w[0] {
            "case" => {
                h = Harness::new();
                case = w[1];
                h.config.policy.harmonic = num(2) as u16;
            }
            "seed" => h.held.push((
                w[1].into(),
                ContextPitch {
                    pitch: num(2),
                    node: Some(LatticePos::new(num(3) as i32, num(4) as i32, num(5) as i32)),
                    weight: 1.0,
                },
            )),
            "on" => {
                let v = h.on(w[1], num(2));
                assert_eq!(
                    v.node,
                    Some(LatticePos::new(num(3) as i32, num(4) as i32, num(5) as i32)),
                    "{case}: {line}"
                );
                // Production axes are fixed microcents, whereas the simulator
                // uses log2 doubles. Accumulated axis rounding is below .001c.
                assert!(v.pitch.abs_diff(num(6)) < 1000, "{case}: {line}: {}", v.pitch);
            }
            "off" => h.off(w[1]),
            "wait" | "bend" => {} // Neither waiting nor bending changes onset policy context.
            other => panic!("unhandled fixture event {other}"),
        }
    }
}
#[test]
fn repeated_controller_chords_cross_one_hundred_dieses_without_wrapping() {
    let mut h = Harness::new();
    for i in 0..304 {
        h.off("*");
        let p = 4_800_000_000 + (i % 3) * 400_000_000;
        let v = h.on("root", p);
        assert_eq!(v.node, Some(LatticePos::new(0, i as i32, 0)));
        h.on("third", p + 400_000_000);
        h.on("fifth", p + 700_000_000);
    }
    assert!(h.memory.reference < -4_000_000_000);
}
#[test]
fn keep_tuning_holds_each_pitch_class_until_the_context_resets() {
    fn chord(h: &mut Harness, root: i64) -> [ContextPitch; 3] {
        h.off("*");
        [h.on("root", root), h.on("third", root + 400_000_000), h.on("fifth", root + 700_000_000)]
    }
    // The travelling cycle above, run both ways so the fixture is shown to
    // drift before the control is shown to stop it.
    for keep in [false, true] {
        let mut h = Harness::new();
        h.config.policy.keep_tuning = keep;
        let home = chord(&mut h, 4_800_000_000);
        for _ in 0..3 {
            chord(&mut h, 5_200_000_000);
            // A♭ major's third is C an octave up: B♯ a diesis low while the
            // context travels, the pinned C an exact octave up while it keeps.
            let third = chord(&mut h, 5_600_000_000)[1];
            assert_eq!(third.pitch == home[0].pitch + 1_200_000_000, keep);
            assert_eq!(chord(&mut h, 4_800_000_000) == home, keep);
        }
    }
    let mut h = Harness::new();
    h.config.policy.keep_tuning = true;
    chord(&mut h, 4_800_000_000);
    assert!(h.memory.pinned(6_000_000_000, h.config.policy).is_some());
    h.memory.clear();
    assert_eq!(h.memory.pinned(6_000_000_000, h.config.policy), None);
    // An attack with the control off forgets the pins too.
    chord(&mut h, 4_800_000_000);
    h.config.policy.keep_tuning = false;
    h.on("d", 5_000_000_000);
    h.config.policy.keep_tuning = true;
    assert_eq!(h.memory.pinned(6_000_000_000, h.config.policy), None);
}
#[test]
fn memory_refreshes_actual_register_pitch_and_new_release_order() {
    let mut h = Harness::new();
    let c = h.on("c", 4_800_000_000);
    h.on("e", 5_200_000_000);
    h.off("e");
    h.off("c");
    assert_eq!(h.memory.recent[0].pitch, c);
    h.on("c", 4_800_000_000);
    h.off("c");
    assert_eq!(h.memory.len, 2);
    let shifted = ContextPitch { pitch: c.pitch - 41_059_000, ..c };
    h.memory.release(shifted, 1, h.config.policy);
    h.memory.release(ContextPitch { pitch: c.pitch + 1_200_000_000, ..c }, 1, h.config.policy);
    assert_eq!(h.memory.len, 4);
    h.memory.forget_source(1);
    assert_eq!(h.memory.len, 0);
}
#[test]
fn hard_boundary_and_configured_axes_ignore_exact_remote_pitch() {
    let mut c = just();
    c.policy.axes = 1;
    let mut s = PolicyScratch::default();
    let remote = LatticePos::new(10, 0, 0);
    let d = assign_new_note(
        c,
        &[],
        &Memory::default(),
        OrderedOnset { pitch: (c.cents(remote) * 1e6) as i64 },
        &mut s,
    )
    .unwrap();
    assert!(s.candidates.iter().all(|n| n.fives == 0 && n.sevens == 0 && n.threes.abs() <= 3));
    assert_ne!(d.assignment.node(), Some(remote));
}
#[test]
fn resource_exhaustion_is_explicit_and_never_truncates_to_a_winner() {
    let mut c = just();
    c.policy.axes = 3;
    c.policy.radius = 5;
    let voices: Vec<_> = (0..100)
        .map(|i| ContextPitch {
            pitch: i64::from(i) * 100_000_000,
            node: Some(LatticePos::new(i * 20, 0, 0)),
            weight: 1.0,
        })
        .collect();
    assert_eq!(
        assign_new_note(
            c,
            &voices,
            &Memory::default(),
            OrderedOnset { pitch: 4_800_000_000 },
            &mut PolicyScratch::default()
        ),
        Err(InputError::TooManyCandidates)
    );
}

#[test]
fn analytic_reachability_covers_direct_selection_across_registers() {
    let mut h = Harness::new();
    h.on("c", 4_800_000_000);
    h.on("g", 5_500_000_000);
    h.on("d", 6_200_000_000);
    let context: Vec<_> = h.held.iter().map(|(_, v)| *v).collect();
    let snapshot = reach::Snapshot {
        config: h.config,
        reference: h.memory.reference,
        context: context.clone(),
    };
    let reachable = reach::reachable(&snapshot, 3600.0, 9600.0, || false).unwrap();
    for cents in (3600..=9600).step_by(7) {
        let selected = assign_new_note(
            h.config,
            &context,
            &h.memory,
            OrderedOnset { pitch: i64::from(cents) * 1_000_000 },
            &mut h.scratch,
        )
        .unwrap();
        assert!(reachable.contains(&selected.assignment.node().unwrap()));
    }
    let shifted = reach::Snapshot {
        context: context
            .iter()
            .map(|v| ContextPitch { pitch: v.pitch + 1_200_000_000, ..*v })
            .collect(),
        ..snapshot
    };
    assert_eq!(reachable, reach::reachable(&shifted, 4800.0, 10800.0, || false).unwrap());
}
#[test]
fn channel_bend_and_rpn_sensitivity_are_resolved_before_attack() {
    let mut channel = channel::ChannelPitch::default();
    channel.apply([0xe0, 0, 96]);
    assert_eq!(channel.microcents(), 100_000_000);
    channel.apply([0xb0, 101, 0]);
    channel.apply([0xb0, 100, 0]);
    channel.apply([0xb0, 6, 12]);
    channel.apply([0xb0, 38, 50]);
    assert_eq!(channel.microcents(), 625_000_000);
    channel.apply([0xb0, 121, 0]);
    assert_eq!(channel.microcents(), 0);
}
