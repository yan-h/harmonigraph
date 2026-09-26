#!/usr/bin/env python3
"""One-off audit witnesses; run.py MODE REPO [SCRATCH_DIRECTORY]. Not a test suite."""
import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('mode', choices=('confirmed', 'motion', 'png', 'fixtures'))
parser.add_argument('repo', type=Path)
parser.add_argument('scratch', nargs='?', type=Path)
args = parser.parse_args()
if args.mode in ('confirmed', 'motion') and sys.platform != 'darwin':
    parser.error('CPU timing uses Darwin clock() microsecond ticks')
repo = args.repo.resolve()
scratch = (args.scratch or Path(tempfile.mkdtemp(prefix='harmonigraph-audit-'))).resolve()
scratch.mkdir(parents=True, exist_ok=True)
if (scratch / 'Cargo.toml').exists():
    parser.error('choose an empty scratch directory; the witness will create a Cargo project')
(scratch / 'src').mkdir(exist_ok=True)
output = scratch / 'output'
output.mkdir(exist_ok=True)
source = Path(__file__).with_name(args.mode + '.rs').read_text()
source = source.replace('@REPO@', str(repo)).replace('@OUTPUT@', str(output))
(scratch / 'src' / 'main.rs').write_text(source)
manifest = '[package]\nname = "harmonigraph-audit-' + args.mode + '"\nversion = "0.0.0"\nedition = "2021"\n\n[dependencies]\n'
if args.mode == 'png':
    manifest += 'image = { version = "0.25", default-features = false, features = ["png"] }\n'
else:
    manifest += 'harmonigraph-core = { path = "' + str(repo / 'crates/harmonigraph-core') + '" }\n'
    if args.mode in ('motion', 'fixtures'):
        manifest += 'harmonigraph-scene = { path = "' + str(repo / 'crates/harmonigraph-scene') + '" }\n'
(scratch / 'Cargo.toml').write_text(manifest)
# Seed resolution from the audited checkout, not newer cached crate versions.
shutil.copyfile(repo / 'Cargo.lock', scratch / 'Cargo.lock')
env = dict(os.environ, RUSTC_WRAPPER='')
print(f'Scratch project: {scratch}', flush=True)
subprocess.run(['cargo', 'run', '--release', '--offline', '--manifest-path', str(scratch / 'Cargo.toml')], env=env, check=True)
