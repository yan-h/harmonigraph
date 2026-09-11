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
        self.memory.attack(
            pitch,
            d.assignment.correction_microcents(),
            self.config.policy.tolerance,
        );
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
/// The pitch the keyboard sends for the key it spells `threes`, `fives`,
/// `octave` octaves above the one starting at C 4800¢.
fn sent(h: &Harness, threes: i32, fives: i32, octave: i64) -> i64 {
    4_800_000_000
        + octave * OCTAVE
        + keyboard_class(h.config.policy.keyboard, LatticePos::new(threes, fives, 0))
}
fn play(h: &mut Harness, keys: &[(&str, i32, i32, i64)]) -> Option<LatticePos> {
    let mut node = None;
    for &(id, threes, fives, octave) in keys {
        let pitch = sent(h, threes, fives, octave);
        node = h.on(id, pitch).node;
    }
    node
}
fn meantone() -> [i32; 3] {
    crate::tuning::fifth_generated(crate::tuning::microcents(696.578))
}
#[test]
fn a_meantone_keyboards_c_key_cannot_become_b_sharp() {
    // E spells the G♯ as a third above it; with E released, the held G♯
    // pulls the next C key a third above itself harder than the diesis
    // between B♯ and the key's pitch pulls back.
    fn phrase(h: &mut Harness) -> Option<LatticePos> {
        play(h, &[("e", 0, 1, 0), ("g#", 0, 2, 0)]);
        h.off("e");
        play(h, &[("c", 0, 0, 0)])
    }
    let b_sharp = Some(LatticePos::new(0, 3, 0));
    let mut h = Harness::new();
    assert_eq!(phrase(&mut h), b_sharp, "a 12-TET keyboard cannot tell C from B♯");
    let mut h = Harness::new();
    h.config.policy.keyboard = meantone();
    assert_eq!(phrase(&mut h), Some(LatticePos::ORIGIN));
    h.off("c");
    assert_eq!(play(&mut h, &[("b#", 0, 3, 0)]), b_sharp);
}
#[test]
fn a_schismatic_keyboards_two_a_keys_are_two_as() {
    let mut h = Harness::new();
    h.config.policy.keyboard = crate::tuning::fifth_generated(crate::tuning::microcents(701.711));
    play(&mut h, &[("f", -1, 0, 0), ("c", 0, 0, 1), ("g", 1, 0, 1), ("d", 2, 0, 2)]);
    assert_eq!(play(&mut h, &[("a", 3, 0, 2)]), Some(LatticePos::new(3, 0, 0)));
    h.off("a");
    assert_eq!(play(&mut h, &[("a", -1, 1, 2)]), Some(LatticePos::new(-1, 1, 0)));
}
#[test]
fn a_slightly_mislearned_fifth_still_keeps_the_b_sharp_key() {
    // A fifth learned 0.2¢ sharp, as pitch-bend steps can leave it, renders
    // B♯ twelve fifths out 2.4¢ from the pitch the key sends. The held C pulls
    // that key to C unless the filter still recognises it.
    let mut h = Harness::new();
    h.config.policy.keyboard = crate::tuning::fifth_generated(crate::tuning::microcents(696.778));
    play(&mut h, &[("c", 0, 0, 0)]);
    let b_sharp = LatticePos::new(0, 3, 0);
    let pitch = 4_800_000_000 + keyboard_class(meantone(), b_sharp);
    assert_eq!(h.on("b#", pitch).node, Some(b_sharp));
}
#[test]
fn an_attack_bent_off_every_key_is_scored_against_every_node() {
    // The phrase whose in-tune C key the meantone keyboard keeps off B♯. Bent
    // 15¢ flat it is off every key, so B♯ competes again and wins.
    let mut h = Harness::new();
    h.config.policy.keyboard = meantone();
    play(&mut h, &[("e", 0, 1, 0), ("g#", 0, 2, 0)]);
    h.off("e");
    let bent = sent(&h, 0, 0, 0) - 15_000_000;
    assert_eq!(h.on("c", bent).node, Some(LatticePos::new(0, 3, 0)));
}
#[test]
fn reachability_plays_the_keys_the_unfiltered_sweep_never_reaches() {
    // With F and C held, the Pythagorean A costs 12 more than the 5-limit one
    // and never wins by pitch alone; a schismatic keyboard's key three fifths
    // up admits it and nothing else.
    let mut h = Harness::new();
    h.config.policy.keyboard = crate::tuning::fifth_generated(crate::tuning::microcents(701.711));
    play(&mut h, &[("f", -1, 0, 0), ("c", 0, 0, 1)]);
    let snapshot = reach::Snapshot {
        config: h.config,
        reference: h.memory.reference,
        context: h.held.iter().map(|(_, v)| *v).collect(),
    };
    let reachable = reach::reachable(&snapshot, 3600.0, 9600.0, || false).unwrap();
    assert!(reachable.contains(&LatticePos::new(3, 0, 0)), "{reachable:?}");
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
