#!/usr/bin/env python3
"""Produce an unapplied temporary test patch, asserting inherited fixture parity."""
import difflib
import hashlib
import json
from pathlib import Path
import subprocess
import sys

out = Path(__file__).resolve().parent
repo = Path(sys.argv[1] if len(sys.argv) > 1 else '/Users/yan/.codex/worktrees/stars-investigation/harmonigraph')
rel = 'crates/harmonigraph-render/src/spectrogram.rs'
old = (repo / rel).read_text()
frozen = Path('/private/tmp/stars-investigation/evidence-bundle/source-snapshot')
reference = (frozen / rel).read_text()
def function(src, name):
    start = src.index(f'fn {name}(')
    opening = src.index('{', start)
    depth = 1
    i = opening + 1
    while depth:
        depth += (src[i] == '{') - (src[i] == '}')
        i += 1
    return src[start:i]
helper_names = ['cloud_fixture', 'grid_of', 'relay_quad', 'read_of', 'callback', 'full_quad', 'shades', 'ramp_lut']
for name in helper_names:
    assert function(old, name) == function(reference, name), f'fixture helper changed: {name}'
assert old.count('    mod star_split;') == 1
assert 'mod live_parity;' not in old
new = old.replace('    mod star_split;', '    mod live_parity;\n    mod star_split;')
testrel = 'crates/harmonigraph-render/src/spectrogram/tests/live_parity.rs'
test = (out / 'live_parity.rs').read_text()
patch = ''.join(difflib.unified_diff(old.splitlines(True), new.splitlines(True), fromfile='a/'+rel, tofile='b/'+rel))
patch += ''.join(difflib.unified_diff([], test.splitlines(True), fromfile='/dev/null', tofile='b/'+testrel))
(out / 'live-parity.patch').write_text(patch)
paths = [Path('/private/tmp/stars-full-halo-compare') / f for f in ['palette.rgba', 'take-levels.u8', 'flat-levels.u8']]
paths += [out/'live_parity.rs', Path('/private/tmp/stars-investigation/frozen-research/research.rs')]
manifest = dict(base='fd7c8f9f9fccb2a55f79f8ee87c25e2d5afd8d40', helpers_equal_to_frozen=helper_names,
               sha256={str(p): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths})
(out / 'fixture-provenance.json').write_text(json.dumps(manifest, indent=2)+'\n')
subprocess.run(['git', 'apply', '--check', str(out/'live-parity.patch')], cwd=repo, check=True)
print('Wrote unapplied patch and fixture hashes; inherited helper bodies match frozen capture.')
