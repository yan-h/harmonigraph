#!/usr/bin/env python3
"""The loader can install a published build after its source worktree is gone."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[2]
product = 'harmonigraph' if 'harmonigraph-plugin' in (root / '.agent-lifecycle.json').read_text() else 'otonal'
name = 'Harmonigraph' if product == 'harmonigraph' else 'otonal'
lib = f'lib{product}_plugin.dylib'
if not shutil.which('codesign') or not shutil.which('cc'):
    print('Skipped: macOS codesign and cc required')
    raise SystemExit(0)

with tempfile.TemporaryDirectory(prefix='preserved-build-') as tmp:
    tmp = Path(tmp).resolve()
    repo = tmp / 'repo'
    repo.mkdir()
    env = {k: v for k, v in os.environ.items() if not k.startswith('GIT_') and not k.startswith('AGENT_LIFECYCLE_')}
    # Only this disposable HOME receives the offline renderer or other installation.
    env['HOME'] = str(tmp / 'home')
    def run(*args):
        return subprocess.check_output(args, cwd=repo, env=env, text=True, stderr=subprocess.STDOUT)
    run('git', 'init', '-q', '-b', 'main')
    run('git', 'config', 'user.name', 'Fixture')
    run('git', 'config', 'user.email', 'fixture@example.test')
    (repo / '.gitignore').write_text('target/\n')
    (repo / '.claude').mkdir()
    for item in ('load-plugin.sh', 'session-lifecycle.sh', '.claude/build-handoffs.sh'):
        shutil.copy2(root / item, repo / item)
    run('git', 'add', '.')
    run('git', 'commit', '-qm', 'fixture')
    sha = run('git', 'rev-parse', 'HEAD').strip()
    branch = 'codex/preserved'
    wt = tmp / 'source'
    run('git', 'worktree', 'add', '-qb', branch, str(wt))
    publication = repo / '.git/agent-lifecycle/builds/fixture'
    release = publication / 'target/release'
    release.mkdir(parents=True)
    c = tmp / 'fixture.c'
    c.write_text(f'const char *tag = "{branch} @{sha[:7]}";\nint fixture_value(void) {{ return 7; }}\n')
    run('cc', '-dynamiclib', '-o', str(release / lib), str(c))
    files = {f'target/release/{lib}': hashlib.sha256((release / lib).read_bytes()).hexdigest()}
    if product == 'harmonigraph':
        offline = release / 'harmonigraph-offline'
        offline.write_text('#!/bin/sh\necho matched-renderer\n')
        offline.chmod(0o755)
        files['target/release/harmonigraph-offline'] = hashlib.sha256(offline.read_bytes()).hexdigest()
    (publication / 'handoff.json').write_text(json.dumps(dict(branch=branch, commit=sha, commit_time=0, files=files)))
    run('git', 'worktree', 'remove', str(wt))
    # Stub only the shared CLI boundary. Its publication/checksum/lock behavior
    # has separate integration tests in agent-config.
    fake = tmp / 'catalog.py'
    fake.write_text('''import os,subprocess,sys
from pathlib import Path
repo=Path(sys.argv[2]); action=sys.argv[3]
if action=='load':
 env=dict(os.environ,AGENT_LIFECYCLE_CATALOG_FD='fixture')
 raise SystemExit(subprocess.call(sys.argv[5:],env=env))
if action=='catalog':
 import json
 p=repo/'.git/agent-lifecycle/builds/fixture';m=json.loads((p/'handoff.json').read_text())
 print(str(p)+'\\t'+m['branch']+'\\t'+m['commit']+'\\t0')
''')
    env['AGENT_LIFECYCLE_TOOL'] = str(fake)
    bundle = repo / f'target/bundled/{name}.clap'
    if product == 'harmonigraph':
        (bundle / 'Contents/MacOS').mkdir(parents=True)
        shutil.copy2(release / lib, bundle / f'Contents/MacOS/{name}')
        (bundle / 'Contents/Info.plist').write_text(f'<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleExecutable</key><string>{name}</string><key>CFBundleIdentifier</key><string>test.preserved</string></dict></plist>')
        run('codesign', '--force', '--sign', '-', str(bundle))
        assert run('./load-plugin.sh', '--tag', branch).strip() == f'{branch} @{sha[:7]}'
    assert branch in run('./load-plugin.sh', '--list')
    output = run('./load-plugin.sh', branch)
    run('codesign', '--verify', str(bundle))
    assert 'worktree=' + str(publication) in (repo / 'target/bundled/.loaded').read_text()
    if product == 'harmonigraph':
        installed = Path(env['HOME']) / 'Library/Application Support/Harmonigraph/harmonigraph-offline'
        assert installed.read_bytes() == (release / 'harmonigraph-offline').read_bytes()
    print(f'{product}: preserved build loads after source removal; installed signature verifies')
