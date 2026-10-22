//! `golden-compare <base> <head> <again|-> <out>`: what the head's golden
//! frames did against the base's, for `tools/golden-diff.py`.
//!
//! Writes `<out>/sheets/` (a contact sheet per moved frame), `<out>/report.md`
//! (the table a PR body states) and `<out>/changed` (one frame per line, empty
//! when nothing changed: every name Full CI's summary job requires the PR body
//! to mention), and prints the table.

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
    let changed = report.changed();

    let mut table = String::new();
    if !changed.is_empty() {
        table.push_str("| Frame | Change | Mean /255 | Max /255 |\n|---|---|---|---|\n");
        for (frame, mean, max) in &report.moved {
            writeln!(table, "| `{frame}` | moved | {mean:.3} | {max} |").unwrap();
        }
        for frame in &report.new {
            writeln!(table, "| `{frame}` | new | | |").unwrap();
        }
        for frame in &report.removed {
            writeln!(table, "| `{frame}` | removed | | |").unwrap();
        }
        for (frame, [bw, bh], [hw, hh]) in &report.resized {
            writeln!(table, "| `{frame}` | resized {bw}x{bh} → {hw}x{hh} | | |").unwrap();
        }
        for frame in &report.unstable {
            writeln!(table, "| `{frame}` | unstable: drawn differently twice by this build | | |")
                .unwrap();
        }
    } else {
        table.push_str("No golden frame moved against the base.\n");
    }
    std::fs::create_dir_all(out).expect("output directory");
    std::fs::write(out.join("report.md"), &table).expect("write report.md");
    let names: String = changed.iter().map(|f| format!("{f}\n")).collect();
    std::fs::write(out.join("changed"), names).expect("write changed");
    print!("{table}");
}
