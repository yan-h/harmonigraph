#!/usr/bin/env python3
"""Extract the proven two-query timing path, keeping only production case settings."""
from pathlib import Path
import difflib
import hashlib
import json
import subprocess

out = Path(__file__).resolve().parent
repo = Path('/Users/yan/.codex/worktrees/stars-investigation/harmonigraph')
frozen = Path('/private/tmp/stars-investigation/frozen-research/research.rs')
source = frozen.read_text()
body = source[source.index("struct Case<'a>"):source.index('\nthread_local! {')]
def replace(old, new):
    global body
    assert body.count(old) == 1, (old[:80], body.count(old))
    body = body.replace(old, new)
def section(begin, end, new):
    global body
    a = body.index(begin)
    b = body.index(end, a)
    body = body[:a] + new + body[b:]

replace('fn stars_research_timings()', 'fn stars_live_production_timings()')
replace('    let profile = research_profile();\n', '')
replace('std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(60);',
        'std::env::var("PROBE_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(240);')
replace('    assert!(frames > 0, "PROBE_FRAMES must be positive");',
        '    assert!(frames > 0 && frames <= 240 && frames % 12 == 0, "PROBE_FRAMES must be12..240, divisible by12 for balanced order");\n'
        '    let raw_path = std::env::var("PROBE_RAW").expect("PROBE_RAW required");\n'
        '    assert!(std::env::var_os("PROBE_STAGES").is_none(), "stage timing is excluded");\n'
        '    assert!(std::env::var_os("RESEARCH_CASES").is_none(), "all six production cases are always measured");')
replace('.unwrap_or_else(|| FILLS.to_vec());', '.unwrap_or_else(|| vec![1.0]);')
replace('wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::SHADER_F16', 'wgpu::Features::TIMESTAMP_QUERY')
replace('        eprintln!("no GPU adapter; nothing timed");\n        return;', '        panic!("GPU required for explicit production timing");')
replace('        eprintln!("the adapter carries no timestamps; nothing timed");\n        return;', '        panic!("timestamp queries required for explicit production timing");')
section('    // Extra timestamps can perturb scheduling.', '    let set = device.create_query_set',
        '    // Same production two-query bracket; no stage instrumentation.\n'
        '    let query_count = 2;\n    let query_bytes = 16;\n')
section('                timestamp_writes: stage_writes(', '                ..Default::default()',
        '                timestamp_writes: Some(wgpu::RenderPassTimestampWrites {\n'
        '                    query_set: &set,\n'
        '                    beginning_of_pass_write_index: (!source_stamped).then_some(0),\n'
        '                    end_of_pass_write_index: Some(1),\n'
        '                }),\n')
section('    let requested = std::env::var("RESEARCH_CASES")', '        .flat_map(|&(name, turn)|',
        "    let mut cases: Vec<Case<'_>> = CASES.iter()\n")
section('    let mut raw = String::from(', '    for frame in 0..frames + warmup {',
        '    assert_eq!(warmup, 60, "production comparison retains60 warmup rounds");\n'
        '    assert_eq!(orders.len(), 12);\n'
        '    let mut raw = String::from("frame,slot,case,gpu_ms,wall_ms\\n");\n')
replace('            activate(case.name);\n', '')
replace('.unwrap_or(1.0);\n                apply_research_profile(&mut settings, &profile);', '.unwrap_or(0.5);')
replace('            STAGE_QUERY.with_borrow_mut(|query| *query = stages.then(|| set.clone()));\n            STAGE_WRITTEN.set(0);\n', '')
replace('            STAGE_QUERY.with_borrow_mut(|query| *query = None);\n', '')
replace('            let source_stamped = SOURCE_QUERY.with_borrow_mut(|query| query.take().is_none());',
        '            let source_stamped = SOURCE_QUERY.with_borrow_mut(|query| query.take().is_none());\n'
        '            assert!(source_stamped, "Stars timing must begin in its real source pass");')
replace('                SpectrogramAtmosphere {\n',
        '                if frame == 0 { eprintln!("{name} settings: {settings:?}"); }\n'
        '                SpectrogramAtmosphere {\n')
section('                if stages {', "                raw.push('\\n');", '')
section('    if let Ok(path) = std::env::var("PROBE_RAW") {', '    for case in &mut cases {',
        '    std::fs::write(&raw_path, raw).unwrap();\n'
        '    eprintln!("production settings only; saved {raw_path};60 warmup,{frames} measured,order offset{offset}");\n')
section('        let baseline =', '\n    }\n}', '')

header = '''//! Temporary production timing evidence. Remove after capture.
//! Reuses the existing production SOURCE_QUERY timestamp hook only.
//! All pipelines, grouped resources and shader sources remain production paths.
use super::*;
use harmonigraph_scene::{CloudStyle, SpectralAtmosphere, StarHaloProfile};
type Turn = fn(&mut SpectralAtmosphere);
fn half(s: &mut SpectralAtmosphere) {
    s.cloud_style = CloudStyle::Stars;
    s.star_halo_profile = StarHaloProfile::Uniform;
    s.star_halo_resolution = 0.5;
    memory(s);
}
fn full(s: &mut SpectralAtmosphere) {
    half(s);
    s.star_halo_resolution = 1.0;
}
fn p2(s: &mut SpectralAtmosphere) { full(s); s.star_halo_profile = StarHaloProfile::P2; }
fn p3(s: &mut SpectralAtmosphere) { full(s); s.star_halo_profile = StarHaloProfile::P3; }
fn memory(s: &mut SpectralAtmosphere) {
    let defaults = SpectralAtmosphere::default();
    s.color_pickup = defaults.color_pickup;
    s.color_release = defaults.color_release;
}
const CASES: &[(&str, Option<Turn>)] = &[
    ("half-a", Some(half)), ("half-b", Some(half)),
    ("full-a", Some(full)), ("full-b", Some(full)),
    ("p2", Some(p2)), ("p3", Some(p3)),
];

'''
orders = source[source.index('fn orders(n:'):source.index('fn research_device()')]
result = header + body + '\n' + orders
for forbidden in ['ACTIVE', 'static SOURCE:', 'activate(', 'stage_writes(', 'STAGE_QUERY', 'research_profile(', 'research::', 'SHADER_F16']:
    assert forbidden not in result, forbidden
assert result.count('SOURCE_QUERY.with_borrow_mut') == 2
assert result.count('beginning_of_pass_write_index: (!source_stamped).then_some(0)') == 1
assert result.count('end_of_pass_write_index: Some(1)') == 1
(out/'live_timing.rs').write_text(result)
subprocess.run(['rustfmt','--edition','2021',str(out/'live_timing.rs')],check=True)
result = (out/'live_timing.rs').read_text()
rel = 'crates/harmonigraph-render/src/spectrogram.rs'
old = (repo/rel).read_text()
assert old.count('    mod timing;') == 1
assert 'mod live_timing;' not in old
new = old.replace('    mod timing;', '    mod live_timing;\n    mod timing;')
testrel = 'crates/harmonigraph-render/src/spectrogram/tests/live_timing.rs'
patch = ''.join(difflib.unified_diff(old.splitlines(True),new.splitlines(True),fromfile='a/'+rel,tofile='b/'+rel))
patch += ''.join(difflib.unified_diff([],result.splitlines(True),fromfile='/dev/null',tofile='b/'+testrel))
(out/'live-timing.patch').write_text(patch)
subprocess.run(['git','apply','--check',str(out/'live-timing.patch')],cwd=repo,check=True)
(out/'provenance.json').write_text(json.dumps(dict(
    frozen_source=str(frozen), frozen_sha256=hashlib.sha256(source.encode()).hexdigest(),
    extracted_sha256=hashlib.sha256(result.encode()).hexdigest(),
    timing='existing production SOURCE_QUERY: source pass BEGIN0 to final composite END1',
    cases=['half-a','half-b','full-a','full-b','p2','p3'],warmup=60,frames=240,
    excludes=['shader source specialization','prototype target selection','extra stage timestamps'],
),indent=2)+'\n')
print('Wrote unapplied live-timing.patch; existing production timing hooks suffice.')
