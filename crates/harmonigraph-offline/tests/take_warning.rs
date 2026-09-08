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
//! (`a_take_missing_note_history_is_exported_with_a_warning` in the binary
//! covers that): that the loop RUNS at all, and that it runs **before** the
//! command line is judged. The second is a contract with `harmonigraph-record`'s
//! `follow`, which takes the FIRST `warning:` line for the Video pane's status
//! line — so a complaint about flags reaching stderr first would take the
//! status line away from the take's missing history.

use harmonigraph_take::{CanonicalRecord, Header, NoteKind, NoteRecord, Record};

/// A readable take with a hole in it, written by hand: the renderer is the
/// thing under test here, not the recorder that would normally produce one.
fn take_with_a_gap(path: &std::path::Path) {
    let note = |t: f64, note: u8| {
        Record::Note(NoteRecord {
            t,
            source: 1,
            channel: 0,
            note,
            kind: NoteKind::On { velocity: 0.8 },
        })
    };
    let gap = Record::Canonical(CanonicalRecord::Gap(
        harmonigraph_core::canonical::PublicationGap {
            source: Some(harmonigraph_core::SourceId(1)),
            time: 0.4,
            through: 0.9,
            first: 6,
            last: 8,
            reason: harmonigraph_core::canonical::GapReason::PublicationFull,
        }
        .into(),
    ));
    let lines = [Record::Header(Header::default()), note(0.2, 60), gap, note(1.2, 64)];
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

    // `--layout` is the first thing after the warning loop that can refuse, so
    // an unusable one stops the run without any of the rendering below it —
    // and puts a second, LATER message on the same stderr to order against.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_harmonigraph-offline"))
        .arg(&take)
        .arg("--layout")
        .arg("definitely-not-a-preset")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "the unusable layout still refuses: {stderr}");

    let first = stderr.lines().next().unwrap_or_default();
    assert!(first.starts_with("warning:"), "the take's own trouble comes first: {stderr}");
    assert!(first.contains("6..=8"), "and names the lost range: {stderr}");
    let complaint = stderr
        .lines()
        .position(|line| line.contains("definitely-not-a-preset"))
        .expect("the layout complaint reached stderr too");
    assert!(complaint > 0, "the command line is judged after the take is read: {stderr}");
    std::fs::remove_dir_all(directory).unwrap();
}
