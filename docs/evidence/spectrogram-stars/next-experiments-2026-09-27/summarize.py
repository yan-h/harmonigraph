from pathlib import Path
import json
for d in Path('/private/tmp/stars-next').iterdir():
 if not d.is_dir():continue
 for p in d.glob('*.analysis.json'):
  j=json.loads(p.read_text());print(d.name,p.stem)
  print('means', {k:round(v['mean_gpu_ms'],3) for k,v in j['cases'].items()})
  print('A/A', {k:round(v['mean_savings_percent'],2) for k,v in j['aa_noise'].items()})
  for k,v in j['comparisons'].items():
   ref=('full-a/full-b' if k.startswith('full') else 'blur-a/blur-b' if k.startswith('blur') else 'p3-a/p3-b')
   if ref in v:print(k,round(v[ref]['paired_mean_savings_percent'],2),[round(x,2) for x in v[ref]['block_bootstrap_95pct_interval_savings_percent']])
