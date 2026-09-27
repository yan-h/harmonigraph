#!/usr/bin/env python3
"""Run the research timing test with explicit checkout, scratch and binary paths.

This script performs GPU work. Read README.md before invoking it.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import subprocess
import sys

HERE = Path(__file__).resolve().parent
DEFAULT_SHADER_ROOT = HERE.parent / 'variants'
DEFAULT_ANALYZER = HERE / 'analyze_timings.py'
DEFAULT_HARNESS = (
    'crates/harmonigraph-render/src/spectrogram.rs',
    'crates/harmonigraph-render/src/spectrogram/atmosphere.rs',
    'crates/harmonigraph-render/src/spectrogram/tests/research.rs',
)


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open('rb') as f:
        for block in iter(lambda: f.read(1024 * 1024), b''):
            h.update(block)
    return h.hexdigest()


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('label')
    p.add_argument('cases', help='comma-separated RESEARCH_CASES names')
    p.add_argument('--worktree', type=Path, default=os.environ.get('HARMONIGRAPH_RESEARCH_WORKTREE', Path.cwd()))
    p.add_argument('--scratch', type=Path, default=os.environ.get('HARMONIGRAPH_RESEARCH_SCRATCH'))
    p.add_argument('--binary', type=Path, default=os.environ.get('HARMONIGRAPH_RESEARCH_BINARY'))
    p.add_argument('--shader-root', type=Path, default=DEFAULT_SHADER_ROOT,
                   help='directory whose child directories group WGSL variants')
    p.add_argument('--frames', default=120, type=int)
    p.add_argument('--warmup', default=60, type=int)
    p.add_argument('--sizes', default='1920x1080,3840x2160')
    p.add_argument('--offset', default=0, type=int)
    p.add_argument('--jitter', default='1')
    p.add_argument('--input', default='synthetic')
    p.add_argument('--profile', default='default')
    p.add_argument('--test-name', default='stars_research_timings')
    args = p.parse_args()
    if args.scratch is None or args.binary is None:
        p.error('--scratch and --binary are required (or set their HARMONIGRAPH_RESEARCH_* environment variables)')
    worktree = args.worktree.resolve()
    scratch = args.scratch.resolve()
    binary = args.binary.resolve()
    if not worktree.is_dir(): p.error(f'worktree does not exist: {worktree}')
    if not binary.is_file(): p.error(f'test binary does not exist: {binary}')
    if not DEFAULT_ANALYZER.is_file(): p.error(f'analyzer missing from bundle: {DEFAULT_ANALYZER}')
    out = scratch / 'timings' / args.label
    out.mkdir(parents=True, exist_ok=False)

    try:
        base = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=worktree, text=True).strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        base = None
    shader_hashes = {}
    if args.shader_root.is_dir():
        for group in sorted(x for x in args.shader_root.iterdir() if x.is_dir()):
            shader_hashes[group.name] = {
                str(f.relative_to(group)): sha256(f)
                for f in sorted(group.rglob('*.wgsl'))
            }
    harness_hashes = {}
    for rel in DEFAULT_HARNESS:
        f = worktree / rel
        harness_hashes[rel] = sha256(f) if f.is_file() else None
    bundled_helpers = {}
    for f in sorted((HERE.parent / 'variants').rglob('*')):
        if f.is_file() and f.suffix in ('.rs', '.py'):
            bundled_helpers[str(f.relative_to(HERE.parent))] = sha256(f)

    meta = {
        'label': args.label, 'cases': args.cases, 'frames': args.frames,
        'warmup': args.warmup, 'sizes': args.sizes, 'offset': args.offset,
        'jitter': args.jitter, 'input': args.input, 'profile': args.profile,
        'worktree': str(worktree), 'binary': str(binary), 'base': base,
        'shader_hashes_by_group': shader_hashes,
        'harness_source_hashes': harness_hashes,
        'bundled_helper_source_hashes': bundled_helpers,
    }
    (out / 'manifest.json').write_text(json.dumps(meta, indent=2, sort_keys=True) + '\n')
    for size in args.sizes.split(','):
        env = {k: v for k, v in os.environ.items()
               if not k.startswith(('PROBE_', 'RESEARCH_', 'STARS_DIAG_'))}
        env.update(
            HARMONIGRAPH_REQUIRE_GPU='1', HARMONIGRAPH_SHADER_ASSETS='source',
            RESEARCH_CASES=args.cases, PROBE_FILLS='1',
            PROBE_FRAMES=str(args.frames), PROBE_WARMUP=str(args.warmup),
            PROBE_SIZE=size, PROBE_ORDER_OFFSET=str(args.offset),
            PROBE_RAW=str(out / f'{size}.csv'), PROBE_JITTER=args.jitter,
            PROBE_INPUT=args.input, PROBE_PROFILE=args.profile,
            RESEARCH_OUTPUT=str(out / 'images'),
        )
        print('Starting', args.label, size, flush=True)
        with (out / f'{size}.log').open('w') as log:
            subprocess.run([str(binary), args.test_name, '--ignored', '--nocapture',
                            '--test-threads=1'], cwd=worktree, env=env,
                           stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run([sys.executable, str(DEFAULT_ANALYZER), str(out / f'{size}.csv')], check=True)
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
