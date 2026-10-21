//! `golden-compare <base> <head> <again|-> <out>`: what the head's golden
//! frames did against the base's, for `tools/golden-diff.py`.
//!
//! Writes `<out>/sheets/` (a contact sheet per moved frame), `<out>/report.md`
//! (the table a PR body states) and `<out>/changed` (`true` or `false`, which
//! Full CI's summary job reads), and prints the table.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [base, head, again, out] = args.as_slice() else {
        eprintln!("usage: golden-compare <base> <head> <again|-> <out>");
        std::process::exit(2);
    };
    let again = (again.as_os_str() != "-").then_some(again.as_path());
    let report = harmonigraph_golden::compare(base, head, again, &out.join("sheets"));

    let mut table = String::new();
    if report.changed() {
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
    } else {
        table.push_str("No golden frame moved against the base.\n");
    }
    if !report.unstable.is_empty() {
        writeln!(
            table,
            "\nUnstable, drawn differently twice by the same build (not a picture change): {}",
            report.unstable.iter().map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
        )
        .unwrap();
    }
    std::fs::create_dir_all(out).expect("output directory");
    std::fs::write(out.join("report.md"), &table).expect("write report.md");
    std::fs::write(out.join("changed"), report.changed().to_string()).expect("write changed");
    print!("{table}");
}
