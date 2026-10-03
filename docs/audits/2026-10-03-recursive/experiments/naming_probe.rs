#[cfg(test)]
mod audit_naming_probe {
    use super::*;
    #[test]
    fn audit_fixed_just_pitch_uses_current_temperament_and_reaches_fallback() {
        let just = Tuning::just();
        let node = LatticePos::new(0, 1, 0);
        let midi = 60.0 + just.five_cents() / 100.0;
        let pitch = PitchClass::from_cents(midi.rem_euclid(12.0) * 100.0);
        let mut view = ViewConfig::default();
        view.meantone = false;
        view.marvel = false;
        let shown = view.reach();
        assert!(shown.positions().any(|pos| pos == node));
        let mut before = Namer::new(&view, shown, &just);
        assert!(before.shows_node(pitch));
        assert_eq!(before.name(midi).to_string(), "E-");
        view.meantone = true;
        view.marvel = true;
        let equal = Tuning::default();
        let shown = view.reach();
        assert!(!equal.matches(pitch, equal.pitch_class(node)));
        assert!(naming_node_from(shown.positions().map(|pos| (pos, equal.pitch_class(pos))), view.tempered(), &equal, pitch).is_none());
        let mut after = Namer::new(&view, shown, &equal);
        assert!(!after.shows_node(pitch));
        assert_eq!(after.name(midi).to_string(), "E");
        assert_eq!(after.name(midi), equal_tempered_name(midi));
        eprintln!("fixed Just E at MIDI {midi}: current Just E- => current equal fallback E; no equal node matches");
    }
}
