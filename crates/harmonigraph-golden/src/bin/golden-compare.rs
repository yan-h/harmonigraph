//! `golden-compare <base> <head> <again|-> <out>`: what the head's golden
//! frames did against the base's, for `tools/golden-diff.py`.
//!
//! Writes `<out>/sheets/` (a contact sheet per moved frame), `<out>/report.md`
//! (the table a PR body states) and `<out>/changed` (the table's rows, empty
//! when nothing changed: every row Full CI's summary job requires the PR body
//! to carry), and prints the table.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [base, head, again, out] = args.as_slice() else {
        eprintln!("usage: golden-compare <base> <head> <again|-> <out>");
        std::process::exit(2);
    };
    let again = (again.as_os_str() != "-").then_some(again.as_path());
    // A frame a second draw reclassifies as unstable must not keep the sheet
    // the first comparison wrote for it.
    let _ = std::fs::remove_dir_all(out.join("sheets"));
    let report = harmonigraph_golden::compare(base, head, again, &out.join("sheets"));

    // One row per change; `changed` is these rows, which the PR body has to
    // carry verbatim, so a frame that moves again on a later push, by a
    // different amount, needs stating again.
    let mut rows = Vec::new();
    for (frame, mean, max) in &report.moved {
        rows.push(format!("| `{frame}` | moved | {mean:.3} | {max} |"));
    }
    for frame in &report.new {
        rows.push(format!("| `{frame}` | new | | |"));
    }
    for frame in &report.removed {
        rows.push(format!("| `{frame}` | removed | | |"));
    }
    for (frame, [bw, bh], [hw, hh]) in &report.resized {
        rows.push(format!("| `{frame}` | resized {bw}x{bh} → {hw}x{hh} | | |"));
    }
    for frame in &report.unstable {
        rows.push(format!("| `{frame}` | unstable: drawn differently twice by this build | | |"));
    }
    let table = if rows.is_empty() {
        "No golden frame moved against the base.\n".to_owned()
    } else {
        let mut table = "| Frame | Change | Mean /255 | Max /255 |\n|---|---|---|---|\n".to_owned();
        for row in &rows {
            writeln!(table, "{row}").unwrap();
        }
        table
    };
    std::fs::create_dir_all(out).expect("output directory");
    std::fs::write(out.join("report.md"), &table).expect("write report.md");
    let changed: String = rows.iter().map(|row| format!("{row}\n")).collect();
    std::fs::write(out.join("changed"), changed).expect("write changed");
    print!("{table}");
}
