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
    // ...and each is still its own module, not the same one twice.
    assert!(reloaded.lattice.contains("lattice one"));
    assert!(reloaded.text.contains("text one"));
}

/// The other two files are watched on their own account, and each produces the
/// pair: text.wgsl was reachable from no reload at all before this, and an edit
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
    reload::publish(format!("{}{marker}", with_common(text::TEXT_SRC)));

    assert!(
        reload::generation() > before,
        "a publish the count does not move is a rebuild nobody asks for"
    );
    assert!(reload::text_source().ends_with(marker), "the source did not cross");
    // Still a whole module: what a reader gets has to be something that
    // compiles, since the next `TextResources::new` builds four pipelines
    // out of it without looking.
    validate_wgsl("text.wgsl", &reload::text_source(), text::TEXT_ENTRY_POINTS)
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

/// A module missing an entry point is named in the message. The two modules
/// keep different lists, so a rejection has to say which one it is about — a
/// list checked against the wrong module reports every entry point missing and
/// reads as a broken shader rather than a broken call.
#[test]
fn a_rejection_names_the_module_it_is_about() {
    let err = validate_wgsl("text.wgsl", &with_common(text::TEXT_SRC), LATTICE_ENTRY_POINTS)
        .expect_err("text.wgsl declares none of the lattice's entry points");
    assert!(err.contains("text.wgsl"), "{err}");
}
