"""Run the scratch harness after applying harness.patch; Metal/M1 Pro evidence."""
import argparse
import json
import os
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(__doc__)
parser.add_argument('--binary', required=True, type=Path, help='compiled release harmonigraph-render test executable')
parser.add_argument('--recording', type=Path, help='optional recording-derived 1024-bin u8 input')
parser.add_argument('--output', type=Path, default=Path('/private/tmp/stars-ordered-split'))
args = parser.parse_args()
if args.output.exists() and any(args.output.glob('*.csv*')):
    parser.error('output contains earlier measurements; choose a fresh --output directory')
args.output.mkdir(parents=True, exist_ok=True)
reference = subprocess.check_output(['git', 'show', '5d05abfd:crates/harmonigraph-render/src/shaders/spectrogram.wgsl'], text=True)
# The reference pipeline never selects the split; the declaration allows its
# unused specialized pipeline to be created by the common constructor.
Path('/private/tmp/stars-original.wgsl').write_text(reference + '\noverride STAR_SPLIT: bool = false;\n')
runs = [
    ('recording-1080-113', '1920x1080', 113, True),
    ('recording-4k-127', '3840x2160', 127, True),
    ('recording-1296-101', '2304x1296', 101, True),
    ('synthetic-1440-103', '2560x1440', 103, False),
    ('recording-1440-107', '2560x1440', 107, True),
    ('recording-1800-109', '3200x1800', 109, True),
    ('sparse-1440-131', '2560x1440', 131, False),
    ('small-stars-1440-137', '2560x1440', 137, False),
]
for name, size, seed, recorded in runs:
    if recorded and args.recording is None:
        continue
    env = {k: v for k, v in os.environ.items() if not k.startswith('PROBE_')}
    env.update(HARMONIGRAPH_SHADER_ASSETS='source', PROBE_COMPARE='base_a,base_b,split,unsplit',
               PROBE_CASE='stars', PROBE_FILLS='1', PROBE_FRAMES='240', PROBE_WARMUP='120',
               PROBE_SIZE=size, PROBE_SEED=str(seed), PROBE_RAW=str(args.output / f'{name}.csv'),
               PROBE_HISTORY_SECONDS='20' if recorded else '10', PROBE_PPP='1' if recorded else '2')
    if recorded:
        env['PROBE_TAKE'] = str(args.recording.resolve())
    if name.startswith('sparse'):
        env['PROBE_DENSITY'] = '0.5'
    if name.startswith('small-stars'):
        env['PROBE_STAR_SIZE'] = '0.5'
    (args.output / f'{name}-env.json').write_text(json.dumps({k: v for k, v in env.items() if k.startswith(('PROBE_', 'HARMONIGRAPH_'))}, indent=2) + '\n')
    print(name, flush=True)
    with (args.output / f'{name}.log').open('w') as log:
        subprocess.run([str(args.binary.resolve()), 'cloud_costs_by_style_and_dial', '--ignored', '--nocapture', '--test-threads=1'], env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
