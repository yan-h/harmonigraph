//! What a shader reload has to reach.
//!
//! The reload is the one path here with no picture behind it to catch a
//! mistake. A rebuild it SKIPS draws the previous build's arithmetic and puts
//! nothing on screen saying so, and the half most likely to be skipped is the
//! half that lives in another entry of `CallbackResources` — so the claims are
//! made against files a test owns rather than against the crate's own sources,
//! which a test may not edit.

use crate::*;

/// A directory of this test's own, named as every other temp-using test here
/// names one. Removed and remade, so a previous run's files are never what a
/// claim is measured against.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("harmonigraph-reload-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// Three files standing in for the three the watcher reads, and a watcher over
/// them. The contents only have to be distinguishable — nothing here compiles
/// them, which `baked_shader_validates` and `baked_text_shader_validates` are
/// for.
fn watched(
    name: &str,
) -> (ShaderWatcher, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let dir = scratch(name);
    let (common, lattice, text) =
        (dir.join("common.wgsl"), dir.join("lattice.wgsl"), dir.join("text.wgsl"));
    std::fs::write(&common, "// common one\n").unwrap();
    std::fs::write(&lattice, "// lattice one\n").unwrap();
    std::fs::write(&text, "// text one\n").unwrap();
    let watcher = ShaderWatcher::watching(lattice.clone(), text.clone(), common.clone());
    (watcher, common, lattice, text)
}

/// `poll` holds a 500 ms debounce; a test is not waiting it out to ask a
/// question about mtimes.
fn poll_now(watcher: &mut ShaderWatcher) -> Option<ReloadedShaders> {
    watcher.next_check = std::time::Instant::now();
    watcher.poll()
}

/// Editing the file BOTH modules are compiled against has to produce both of
/// them, carrying the edit. This is the whole of #510: the watcher saw
/// common.wgsl move and rebuilt the lattice alone, so every name kept drawing
/// against the arithmetic on the previous build.
#[test]
fn an_edit_to_the_common_half_reaches_the_text_module_and_the_lattice_one() {
    let (mut watcher, common, ..) = watched("common-edit");
    assert!(poll_now(&mut watcher).is_none(), "the first sighting is a baseline, not a reload");

    // A distinct mtime, whatever the filesystem's stamp resolution.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&common, "// common TWO\n").unwrap();

    let reloaded = poll_now(&mut watcher).expect("an edited common half is a reload");
    assert!(reloaded.lattice.contains("common TWO"), "the lattice module kept the old common half");
    assert!(reloaded.text.contains("common TWO"), "the text module kept the old common half");
    assert_eq!(reloaded.common, "// common TWO\n");
    // ...and each is still its own module, not the same one twice.
    assert!(reloaded.lattice.contains("lattice one"));
    assert!(reloaded.text.contains("text one"));
}

/// The other two files are watched on their own account, and each produces the
/// pair: an edit to text.wgsl has to rebuild the lattice's own glyph pipelines,
/// which read the same common half, and an edit
/// to lattice.wgsl still has to rebuild the names, which read the same common
/// half.
#[test]
fn an_edit_to_either_module_is_a_reload_of_both() {
    for (which, marker) in [("lattice", "lattice TWO"), ("text", "text TWO")] {
        let (mut watcher, _common, lattice, text) = watched(which);
        assert!(poll_now(&mut watcher).is_none());
        std::thread::sleep(std::time::Duration::from_millis(20));
        let edited = if which == "lattice" { &lattice } else { &text };
        std::fs::write(edited, format!("// {marker}\n")).unwrap();

        let reloaded = poll_now(&mut watcher).unwrap_or_else(|| panic!("{which} edit is a reload"));
        let carried = if which == "lattice" { &reloaded.lattice } else { &reloaded.text };
        assert!(carried.contains(marker), "{which}: the edit did not arrive");
        assert!(reloaded.lattice.contains("common one") && reloaded.text.contains("common one"));
    }
}

/// A poll with nothing moved is not a reload. The pipelines are rebuilt from
/// scratch on every one that answers, so a watcher that answered each time
/// would rebuild nine pipelines twice a second for the life of the session.
#[test]
fn a_poll_over_files_that_did_not_move_rebuilds_nothing() {
    let (mut watcher, ..) = watched("still");
    assert!(poll_now(&mut watcher).is_none(), "baseline");
    assert!(poll_now(&mut watcher).is_none(), "nothing has moved");
    assert!(poll_now(&mut watcher).is_none(), "still nothing");
}

/// A read that fails leaves the stamp where it was, so the edit is picked up on
/// the next poll rather than swallowed. Reachable through an editor that saves
/// by writing a temp file and renaming: for a moment the metadata is the new
/// one and the file is not there to read.
#[test]
fn an_edit_whose_read_fails_is_not_swallowed() {
    let (mut watcher, common, ..) = watched("torn");
    assert!(poll_now(&mut watcher).is_none(), "baseline");

    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&common, "// common TWO\n").unwrap();
    let text = watcher.text.clone();
    std::fs::remove_file(&text).unwrap();
    assert!(poll_now(&mut watcher).is_none(), "a missing half is not a reload");

    // The file is back; the edit is still owed.
    std::fs::write(&text, "// text one\n").unwrap();
    let reloaded = poll_now(&mut watcher).expect("the edit survived the failed read");
    assert!(reloaded.lattice.contains("common TWO") && reloaded.text.contains("common TWO"));
}

/// What crosses to the text callback, which the reload cannot reach directly:
/// the source, and a count that says the pipelines in hand are stale.
///
/// The published module is the baked one with a comment on the end — this is a
/// process-wide global, and every other test in this binary that compiles the
/// text module goes on reading it.
#[test]
fn a_published_reload_raises_the_count_and_hands_over_the_source() {
    let _guard = reload::test_lock();
    let before = reload::generation();
    let marker = "// a_published_reload_raises_the_count_and_hands_over_the_source\n";
    reload::publish(format!("{}{marker}", with_common(text::TEXT_SRC)), COMMON_SRC.to_owned());

    assert!(
        reload::generation() > before,
        "a publish the count does not move is a rebuild nobody asks for"
    );
    assert!(reload::text_source().ends_with(marker), "the source did not cross");
    // Still a whole module: what a reader gets has to be something that
    // compiles, since the next `TextResources::new` builds four pipelines
    // out of it without looking.
    validate_wgsl(
        "text.wgsl",
        &reload::text_source(),
        common_lines(COMMON_SRC),
        text::TEXT_ENTRY_POINTS,
    )
    .expect("a published module must still be one");
}

/// The seam a naga diagnostic's line number has to be read against. Off by one
/// and every rejected edit points at the wrong line, which is worse than
/// pointing at an impossible one.
#[test]
fn the_stated_seam_is_where_the_module_actually_starts() {
    for common in ["a\nb\n", "a\nb", "", "\n"] {
        let joined = module_source(common, "MODULE_LINE_ONE\nMODULE_LINE_TWO");
        let seam = common_lines(common);
        let lines: Vec<&str> = joined.lines().collect();
        assert_eq!(
            lines.get(seam).copied(),
            Some("MODULE_LINE_ONE"),
            "common {common:?}: line {} of the join is not the module's first",
            seam + 1
        );
    }
}

/// And against the real common half, since that is the number a message
/// actually quotes.
#[test]
fn the_baked_common_half_seams_where_it_says_it_does() {
    let joined = with_common("MODULE_LINE_ONE");
    let seam = common_lines(COMMON_SRC);
    assert_eq!(joined.lines().nth(seam), Some("MODULE_LINE_ONE"));
}

/// And the seam a REJECTION states is the one its own module was joined at.
///
/// The two tests above measure `common_lines` on its own; this is the pairing
/// the reload path actually makes, and the only one that can be wrong — a
/// module built against the common half on DISK, checked after an edit there
/// has changed how many lines it takes. Reading the seam off `COMMON_SRC` at
/// that point quotes the BAKED half's count over a module joined at a
/// different line, so every diagnostic from then on points that many lines
/// away from the error, in a file where the number is already the one thing a
/// reader cannot check by eye.
#[test]
fn a_rejection_states_the_seam_its_own_module_was_joined_at() {
    // A common half two lines longer than the baked one, which is what saving
    // an edit to common.wgsl produces.
    let common = format!("{COMMON_SRC}\n// one\n// two\n");
    let seam = common_lines(&common);
    assert!(
        seam > common_lines(COMMON_SRC),
        "the fixture's common half is no longer than the baked one, so a seam \
         taken from either would pass",
    );

    let source = module_source(&common, "fn broken( {");
    let err = validate_wgsl("lattice.wgsl", &source, seam, LATTICE_ENTRY_POINTS)
        .expect_err("`fn broken( {` does not parse");

    assert!(
        err.contains(&format!("lines 1-{seam} below are common.wgsl")),
        "the banner states a seam this module was not joined at: {err}",
    );
    // ...and the stated seam is where the module really starts, so subtracting
    // it off a diagnostic's line lands in the file the banner names.
    assert_eq!(
        source.lines().nth(seam),
        Some("fn broken( {"),
        "line {} of the join is not the module's first",
        seam + 1,
    );
}

/// A module missing an entry point is named in the message. The two modules
/// keep different lists, so a rejection has to say which one it is about — a
/// list checked against the wrong module reports every entry point missing and
/// reads as a broken shader rather than a broken call.
#[test]
fn a_rejection_names_the_module_it_is_about() {
    let err = validate_wgsl(
        "text.wgsl",
        &with_common(text::TEXT_SRC),
        common_lines(COMMON_SRC),
        LATTICE_ENTRY_POINTS,
    )
    .expect_err("text.wgsl declares none of the lattice's entry points");
    assert!(err.contains("text.wgsl"), "{err}");
}

/// Reload both attachment choices with real modules, then draw their labels,
/// shadows and glow. Merely validating WGSL cannot catch a pass/pipeline
/// attachment mismatch or a variant left on the old generation.
#[cfg(feature = "hot-reload")]
#[test]
fn a_reload_rebuilds_and_draws_both_bloom_variants() {
    use super::fixtures::*;
    let _guard = reload::test_lock();
    let Some(mut shooter) = Shooter::new([256, 256]) else { return };
    let mut scene = parity_scene();
    scene.glow_reach = 0.8;
    let labels =
        |scene: &Scene| names(vec![(0, vec![name_glyph(scene, [112.0, 110.0, 24.0, 36.0])])]);
    let plain = shooter.shot_with(&scene, labels(&scene));
    scene.bloom_strength = 1.0;
    let bloomed = shooter.shot_with(&scene, labels(&scene));
    let dir = scratch("real-pipelines");
    let (common, lattice, text_path) =
        (dir.join("common.wgsl"), dir.join("lattice.wgsl"), dir.join("text.wgsl"));
    std::fs::write(&common, COMMON_SRC).unwrap();
    std::fs::write(&lattice, SHADER_SRC).unwrap();
    std::fs::write(&text_path, text::TEXT_SRC).unwrap();
    let old = {
        let resources = shooter.resources.get_mut::<LatticeResources>().unwrap();
        resources.watcher = ShaderWatcher::watching(lattice, text_path, common.clone());
        assert!(poll_now(&mut resources.watcher).is_none());
        resources.scenes.each_ref().map(|s| s.nodes.clone())
    };
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&common, format!("{COMMON_SRC}\n// reload both attachments\n")).unwrap();
    shooter.resources.get_mut::<LatticeResources>().unwrap().watcher.next_check =
        std::time::Instant::now();
    assert_eq!(bloomed, shooter.draw(&scene, labels(&scene)));
    let resources = shooter.resources.get::<LatticeResources>().unwrap();
    for (before, after) in old.iter().zip(&resources.scenes) {
        assert_ne!(*before, after.nodes, "both pipeline variants must rebuild");
    }
    scene.bloom_strength = 0.0;
    assert_eq!(plain, shooter.draw(&scene, labels(&scene)));
    std::fs::remove_dir_all(dir).unwrap();
}
