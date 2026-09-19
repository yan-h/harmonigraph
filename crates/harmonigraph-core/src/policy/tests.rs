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
    /// Each held voice with the millisecond it was struck on.
    held: Vec<(String, ContextPitch, i64)>,
    scratch: PolicyScratch,
    /// Milliseconds, advanced by fixture waits: the clock context decays on.
    now: i64,
}
impl Harness {
    fn new() -> Self {
        Self {
            config: just(),
            memory: Memory::default(),
            held: Vec::new(),
            scratch: PolicyScratch::default(),
            now: 0,
        }
    }
    fn on(&mut self, id: &str, pitch: i64) -> ContextPitch {
        let newest = self.held.iter().map(|&(_, _, t)| t).max().unwrap_or(0);
        let mut context: Vec<_> = self
            .held
            .iter()
            .rev()
            .map(|&(_, v, t)| ContextPitch {
                weight: decay(self.config.policy, newest - t, 1000.0),
                ..v
            })
            .collect();
        self.memory.append(&mut context, self.config.policy, 1000.0);
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
        let struck = self.memory.attack(
            pitch,
            d.assignment.correction_microcents(),
            self.config.policy.tolerance,
            d.assignment.node(),
            self.now,
        );
        self.held.push((id.into(), v, struck));
        v
    }
    fn off(&mut self, id: &str) {
        while let Some(i) = self.held.iter().position(|(name, _, _)| id == "*" || name == id) {
            let (_, v, struck) = self.held.remove(i);
            self.memory.release(v, 1, self.config.policy, struck);
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
                // Every setting the example declared, in the plugin's units.
                // Replaying an example at anything but its own profile agrees
                // for the wrong reason, so an unknown name is a failure here
                // rather than a value quietly left at the default.
                for setting in &w[2..] {
                    let (name, value) =
                        setting.split_once('=').unwrap_or_else(|| panic!("{case}: {line}"));
                    let value = value.parse::<i64>().unwrap();
                    let policy = &mut h.config.policy;
                    match name {
                        "radius" => policy.radius = value.try_into().unwrap(),
                        "axes" => policy.axes = value.try_into().unwrap(),
                        "flexibility" => policy.pitch_flexibility = value.try_into().unwrap(),
                        "half-life-ms" => policy.half_life_ms = value.try_into().unwrap(),
                        "register" => policy.register = value.try_into().unwrap(),
                        "tolerance" => policy.tolerance = value.try_into().unwrap(),
                        // Elapsed silence and transport are the sequencer's,
                        // and this harness replays onsets alone.
                        "silence-ms" | "reset-stop" | "reset-loop" => {
                            assert_eq!(value, 0, "{case}: {setting} is not replayed here")
                        }
                        other => panic!("{case}: unhandled fixture setting {other}"),
                    }
                }
                // The simulator's bounds are not the plugin's: a profile the
                // plugin would clamp is numbers the plugin never runs.
                assert_eq!(h.config.policy, h.config.policy.sanitize(), "{case}");
            }
            "seed" => h.held.push((
                w[1].into(),
                ContextPitch {
                    pitch: num(2),
                    node: Some(LatticePos::new(num(3) as i32, num(4) as i32, num(5) as i32)),
                    weight: 1.0,
                },
                h.now,
            )),
            "on" => {
                let v = h.on(w[1], num(2));
                let node = (w[3] != "none")
                    .then(|| LatticePos::new(num(3) as i32, num(4) as i32, num(5) as i32));
                assert_eq!(v.node, node, "{case}: {line}");
                let output = num(if node.is_some() { 6 } else { 4 });
                // Production axes are fixed microcents, whereas the simulator
                // uses log2 doubles. Accumulated axis rounding is below .001c.
                assert!(v.pitch.abs_diff(output) < 1000, "{case}: {line}: {}", v.pitch);
            }
            "off" => h.off(w[1]),
            // Waiting advances the clock context decays on; bending never
            // changes onset policy context.
            "wait" => h.now += (w[1].parse::<f64>().unwrap() * 1000.0).round() as i64,
            "bend" => {}
            other => panic!("unhandled fixture event {other}"),
        }
    }
}
#[test]
fn repeated_controller_chords_cross_one_hundred_dieses_without_wrapping() {
    let mut h = Harness::new();
    for i in 0..304 {
        // Half a second apart, one default half-life: released memory fades
        // by time alone, and chords closer than that pull each root back.
        h.now += 500;
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
fn a_schismatic_third_chooses_just_e_or_pythagorean_e_without_doubling_c() {
    for (radius, keys, expected) in [
        (3, [0, 4, 1], LatticePos::new(0, 1, 0)),
        (3, [0, 1, 4], LatticePos::new(4, 0, 0)),
        (3, [0, -8, 1], LatticePos::new(0, 1, 0)),
        (4, [0, 4, 1], LatticePos::new(4, 0, 0)),
    ] {
        let mut h = Harness::new();
        h.config.policy.keyboard = [701_720_000, 386_250_000, 975_950_000];
        h.config.policy.radius = radius;
        h.config.policy.pitch_flexibility = 100;
        for fifths in keys {
            let pitch = sent(&h, fifths, 0, 0);
            let voice = h.on(&fifths.to_string(), pitch);
            if fifths == 4 || fifths == -8 {
                assert_eq!(voice.node, Some(expected), "radius {radius}, keys {keys:?}");
            }
        }
    }
}

#[test]
fn exponential_pitch_flexibility_sets_the_tradeoff_without_a_fixed_cutoff() {
    let mut config = just();
    let mut scratch = PolicyScratch::default();
    prepare(config, &[], &mut scratch).unwrap();
    scratch.candidates.retain(|&n| n == LatticePos::ORIGIN);
    assert_eq!(pitch_cost(50, 0.0), 0.0);
    assert_eq!(pitch_cost(50, 50.0), 1.0);
    assert!(pitch_cost(50, 100.0) > 30.0);
    for (flexibility, assigned) in [(50, false), (100, true)] {
        config.policy.pitch_flexibility = flexibility;
        let assignment = select_prepared(
            config,
            2 * OCTAVE + 60_000_000,
            OrderedOnset { pitch: 4_800_000_000 },
            &scratch,
        )
        .unwrap()
        .assignment;
        assert_eq!(assignment.node(), assigned.then_some(LatticePos::ORIGIN));
        assert_eq!(
            assignment.correction_microcents(),
            2 * OCTAVE + if assigned { 0 } else { 60_000_000 }
        );
    }
}

#[test]
fn an_unprofitable_keyboard_match_keeps_the_note_unsnapped() {
    let mut config = just();
    config.policy.axes = 1;
    config.policy.radius = 1;
    let reference = i64::from(config.axes[0]);
    let mut scratch = PolicyScratch::default();
    prepare(config, &[], &mut scratch).unwrap();
    for (input, expected) in
        [(4_800_000_000, None), (4_794_000_000, Some(LatticePos::new(1, 0, 0)))]
    {
        let assignment =
            select_prepared(config, reference, OrderedOnset { pitch: input }, &scratch)
                .unwrap()
                .assignment;
        assert_eq!(assignment.node(), expected);
    }
}

#[test]
fn an_unsnapped_attack_preserves_drift_and_interrupts_a_repeated_node() {
    let mut h = Harness::new();
    h.config.policy.axes = 1;
    h.config.policy.radius = 1;
    h.memory.reference = 2 * OCTAVE;
    h.on("c", 4_800_000_000);
    h.now = 100;
    let voice = h.on("e", 5_200_000_000);
    assert_eq!(voice.node, None);
    assert_eq!(voice.pitch, 5_200_000_000 + 2 * OCTAVE);
    assert_eq!(h.memory.reference, 2 * OCTAVE);
    h.now = 200;
    assert_eq!(h.on("c-again", 4_800_000_000).node, Some(LatticePos::ORIGIN));
    assert_eq!(h.held.last().unwrap().2, 200, "E interrupted the consecutive C attacks");
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
fn memory_refreshes_actual_register_pitch_and_orders_by_attack() {
    let mut h = Harness::new();
    let c = h.on("c", 4_800_000_000);
    h.now += 1000;
    let e = h.on("e", 5_200_000_000);
    h.off("e");
    h.off("c");
    // Struck first and let go last, C is still the older of the two.
    assert_eq!([h.memory.recent[0].pitch, h.memory.recent[1].pitch], [e, c]);
    // A full memory has no room for a note struck before every entry in it.
    let mut full = Memory::default();
    for i in 0..MAX_MEMORY as i64 {
        full.release(ContextPitch { pitch: c.pitch + i * 100_000_000, ..c }, 1, h.config.policy, i);
    }
    full.release(ContextPitch { pitch: c.pitch - 100_000_000, ..c }, 1, h.config.policy, -1);
    assert_eq!(full.len, MAX_MEMORY);
    assert!(full.recent.iter().all(|r| r.at >= 0));
    h.on("c", 4_800_000_000);
    h.off("c");
    assert_eq!(h.memory.len, 2);
    let shifted = ContextPitch { pitch: c.pitch - 41_059_000, ..c };
    h.memory.release(shifted, 1, h.config.policy, h.now);
    h.memory.release(
        ContextPitch { pitch: c.pitch + 1_200_000_000, ..c },
        1,
        h.config.policy,
        h.now,
    );
    assert_eq!(h.memory.len, 4);
    h.memory.forget_source(1);
    assert_eq!(h.memory.len, 0);
}
#[test]
fn octave_copies_of_a_note_add_no_vote() {
    // C two and three octaves below C3 votes less than C3 does for a G by G3,
    // so adding those copies changes nothing; each used to add a vote of its own.
    let config = just();
    let voice = |pitch, node| ContextPitch { pitch, node: Some(node), weight: 1.0 };
    let (c, e) = (LatticePos::ORIGIN, LatticePos::new(0, 1, 0));
    let cost = |context: &[ContextPitch]| {
        let mut scratch = PolicyScratch::default();
        prepare(config, context, &mut scratch).unwrap();
        harmonic_benefit(config, LatticePos::new(1, 0, 0), 5501.955, &scratch.context)
    };
    let alone = cost(&[voice(4_800_000_000, c), voice(5_186_313_714, e)]);
    let doubled = cost(&[
        voice(2_400_000_000, c),
        voice(4_800_000_000, c),
        voice(5_186_313_714, e),
        voice(3_600_000_000, c),
    ]);
    assert_eq!(alone, doubled);
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
