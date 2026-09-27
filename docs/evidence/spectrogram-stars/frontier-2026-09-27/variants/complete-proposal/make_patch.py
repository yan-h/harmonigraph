#!/usr/bin/env python3
"""Read research.rs and emit an unapplied source-only native-complete prototype."""
from pathlib import Path
import difflib
import sys
root = Path(sys.argv[1])
out = Path(__file__).parent
path = "crates/harmonigraph-render/src/spectrogram/tests/research.rs"
original = (root/path).read_text()
s = original
def replace(before, after):
    global s
    assert s.count(before) == 1, (before[:90], s.count(before))
    s = s.replace(before, after)
cases = ""
for name in ["full-complete", "full-complete-mix4", "full-complete-mix5", "full-complete-v50_50_80_100_50", "full-complete-v50_50_100_100_60", "full-grouped-complete-v100_100_100_100_100", "full-grouped-complete-v50_50_80_100_50", "full-grouped-complete-v50_50_100_100_60", "full-separate-complete-v50_50_80_100_50", "full-separate-complete-v50_50_100_100_60"]:
    cases += f'    ("{name}", Some(|s| {{ s.cloud_style = CloudStyle::Stars; s.star_halo_resolution = 1.0; memory(s); }})),\n'
replace('const CASES: &[(&str, Option<Turn>)] = &[', 'const CASES: &[(&str, Option<Turn>)] = &[\n' + cases)
replace('fn activate(name: &str) {', (out/'source_hook.rs').read_text() + '\nfn activate(name: &str) {')
replace('    SOURCE.with_borrow_mut(|s| *s = Some(source));', '''    if name.starts_with("full-complete") {
        source = complete_source(source, name);
    }
    SOURCE.with_borrow_mut(|s| *s = Some(source));''')
replace('    if name.starts_with("full-complete") {\n        source = complete_source', '    if complete_mode(name) {\n        source = complete_source')
replace('ACTIVE.with_borrow(|name| name.starts_with("full-grouped-v"))', 'ACTIVE.with_borrow(|name| name.starts_with("full-grouped-v") || name.starts_with("full-grouped-complete-v"))')
replace('assert!(name.starts_with("full-grouped-v"));', 'assert!(name.starts_with("full-grouped-v") || name.starts_with("full-grouped-complete-v"));')
replace('ACTIVE.with_borrow(|name| name.starts_with("full-separate-v"))', 'ACTIVE.with_borrow(|name| name.starts_with("full-separate-v") || name.starts_with("full-separate-complete-v"))')
# The latest harness permits arbitrary vector cases not listed in CASES.
needle = '|| name.starts_with("full-grouped-v"), "unknown research case {name}"'
if needle in s:
    replace(needle, '|| name.starts_with("full-grouped-v") || complete_mode(name), "unknown research case {name}"')
(out/'native-complete.patch').write_text(''.join(difflib.unified_diff(original.splitlines(True), s.splitlines(True), fromfile='a/'+path, tofile='b/'+path)))
print(out/'native-complete.patch')
