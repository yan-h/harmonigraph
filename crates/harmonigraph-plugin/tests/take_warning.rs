//! The renderer's own warning loop, run as a process.
//!
//! Stage 5B reported `run()`'s `eprintln!` loop as untestable "because it needs
//! a GPU and argv". The GPU half is wrong, and the review said so: the loop is
//! the statement after `Take::read`, hundreds of lines before an adapter is
//! ever requested, so nothing about it depends on there being one. What it does
//! need is a process — argv to parse and a stderr to read — which is what an
//! integration test over `CARGO_BIN_EXE_` is for.
//!
//! Two things worth pinning, and neither is `take_warnings`' own formatting
//! (`a_take_missing_note_history_is_exported_with_a_warning` in the library
//! covers that): that the loop RUNS at all, and that it runs **before** the
//! command line is judged, so the export job's row, which lists every warning in
//! the order printed, leads with the recording rather than the flags.

use harmonigraph_take::{CanonicalRecord, Header, NoteKind, NoteRecord, Record};

/// A readable take with a hole in it, written by hand: the renderer is the
/// thing under test here, not the recorder that would normally produce one.
fn take_with_a_gap(path: &std::path::Path) {
    let note = |t: f64, note: u8| {
        let event =
            NoteRecord { t, note, kind: NoteKind::On { velocity: 0.8 }, ..Default::default() };
        Record::Canonical(CanonicalRecord::Delta(event.into()))
    };
    let gap = Record::Canonical(CanonicalRecord::Gap(
        harmonigraph_core::canonical::PublicationGap { time: 0.4, through: 0.9, first: 6, last: 8 }
            .into(),
    ));
    let run = Record::Run(harmonigraph_take::RunRecord::default());
    // A look, as every take writer records one: without it the export refuses
    // before the command line is judged.
    let header = Header {
        appearance: Some(harmonigraph_ui::AppearanceDocument::default().serialize()),
        ..Default::default()
    };
    let lines = [Record::Header(header), run, note(0.2, 60), gap, note(1.2, 64)];
    let encoded =
        lines.iter().map(|r| ron::to_string(r).unwrap()).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(path, encoded).unwrap();
}

#[test]
fn a_take_with_a_hole_warns_before_the_command_line_is_judged() {
    let directory =
        std::env::temp_dir().join(format!("harmonigraph-offline-warning-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let take = directory.join("gapped.take");
    take_with_a_gap(&take);

    // A missing recorded audio file is harmless, but an invalid explicit
    // output aspect warns after the take's own gap, before rendering starts.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_harmonigraph-offline"))
        .arg(&take)
        .args(["--size", "160x120", "--out", "/dev/null/impossible.mp4"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the unusable output refuses: {stderr}");

    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.starts_with("warning:"), "the take's own trouble comes first: {stderr}");
    assert!(first.contains("6..=8"), "and names the lost range: {stderr}");
    let complaint = stderr
        .lines()
        .position(|line| line.contains("--size 160x120"))
        .expect("the size warning reached stderr too");
    assert!(complaint > 0, "the command line is judged after the take is read: {stderr}");
    std::fs::remove_dir_all(directory).unwrap();
}

/// A renderer told another build's tag says so first, naming both, even when
/// it then cannot read the take (#1928 B): the failure a mismatched pair
/// most plausibly meets is a take or look this build cannot read.
#[test]
fn a_renderer_of_another_build_says_so_before_anything_fails() {
    let missing = std::env::temp_dir()
        .join(format!("harmonigraph-offline-no-such-{}.take", std::process::id()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_harmonigraph-offline"))
        .args(["--plugin-build", "other @0000000"])
        .arg(&missing)
        .output()
        .unwrap();
    assert!(!out.status.success(), "a missing take rendered");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.starts_with(harmonigraph_take::WARNING_PREFIX), "{stderr}");
    assert!(first.contains("other @0000000"), "the plugin's build is unnamed: {stderr}");
}

/// The renderer carries the tag the plugin package stamps, so the plugin of
/// its own build queues an export with no warning (#1954).
#[test]
fn a_renderer_of_its_own_build_does_not_warn() {
    let missing = std::env::temp_dir()
        .join(format!("harmonigraph-offline-own-build-{}.take", std::process::id()));
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_harmonigraph-offline"))
        .args(["--plugin-build", env!("LATTICE_BUILD_TAG")])
        .arg(&missing)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("the plugin that queued"), "{stderr}");
}
