#!/usr/bin/env python3
"""Compare real production outputs to frozen-reference bytes; never hides missing pairs."""
import argparse
import hashlib
import json
from pathlib import Path
import numpy as np

p = argparse.ArgumentParser()
p.add_argument('production', type=Path)
p.add_argument('--reference', type=Path, help='defaults from captured PPP')
p.add_argument('--output', type=Path)
a = p.parse_args()
m = json.loads((a.production/'manifest.json').read_text())
assert m['fixture'] == 'frozen-research-default' and m['shader'] == 'production'
assert (m['warmup'], m['fps'], m['time'], m['jitter'], m['frames']) == (60,30,3,.5,1)
root = Path('/private/tmp/stars-investigation')
default_ref = {2.0:'images-grouped', 4.0:'images-final-4k', 1.001:'images-grouped-odd'}
reference = a.reference or root/default_ref[m['ppp']]
rm = json.loads((reference/'manifest.json').read_text())
assert (rm['ppp'], rm['frames'], rm['jitter']) == (m['ppp'],1,.5), 'reference settings mismatch'
mapping = {'Half':['half-a','full-grouped-v50_50_50_50_50'],
           'Full':['full-a','full-grouped-v100_100_100_100_100'],
           'P2':['full-grouped-v50_50_80_100_50'],
           'P3':['full-grouped-v50_50_100_100_60']}
report = dict(production=str(a.production), reference=str(reference), pairs=[], missing=[])
size = m['size'][0] * m['size'][1] * 4
for name, variants in mapping.items():
    for input_name in ['take','flat']:
        target = a.production/f'{name}-{input_name}.rgba'
        raw = target.read_bytes()
        assert len(raw) == size, f'production byte count: {target}'
        actual = np.frombuffer(raw, np.uint8).reshape(-1,4)
        for variant in variants:
            ref = reference/f'{variant}-{input_name}.rgba'
            if not ref.exists():
                report['missing'].append(str(ref))
                continue
            expect_raw = ref.read_bytes()
            assert len(expect_raw) == size, f'reference byte count: {ref}'
            expected = np.frombuffer(expect_raw, np.uint8).reshape(-1,4)
            delta = np.abs(actual.astype(np.int16)-expected.astype(np.int16))
            rgb = delta[:,:3]
            report['pairs'].append(dict(case=name, input=input_name, reference=variant,
                exact=raw==expect_raw, different_bytes=int(np.count_nonzero(delta)),
                different_rgb_pixels=int(np.count_nonzero(rgb.max(axis=1))),
                rgb_pixels_over1=int(np.count_nonzero(rgb.max(axis=1)>1)),
                max_byte_difference=int(delta.max()), mean_rgb_difference=float(rgb.mean()),
                alpha_different_bytes=int(np.count_nonzero(delta[:,3])),
                production_sha256=hashlib.sha256(raw).hexdigest(),
                reference_sha256=hashlib.sha256(expect_raw).hexdigest()))
report['all_available_pairs_exact'] = bool(report['pairs']) and all(v['exact'] for v in report['pairs'])
report['required_cases_covered'] = all(any(v['case']==c and v['input']==i for v in report['pairs'])
    for c in mapping for i in ['take','flat'])
report['complete_exact_parity'] = report['all_available_pairs_exact'] and report['required_cases_covered']
output = a.output or a.production/'comparison.json'
output.write_text(json.dumps(report, indent=2)+'\n')
print(json.dumps({k:v for k,v in report.items() if k not in ['pairs','missing']}, indent=2))
for row in report['pairs']:
    print(row['case'], row['input'], row['reference'], 'exact' if row['exact'] else
          f"DIFF {row['different_bytes']} bytes, max{row['max_byte_difference']}, {row['rgb_pixels_over1']} pixels >1LSB")
print(f"Missing reference files: {len(report['missing'])}; details: {output}")
raise SystemExit(0 if report['complete_exact_parity'] else 1)
