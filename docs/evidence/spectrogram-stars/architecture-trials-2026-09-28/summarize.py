from pathlib import Path
import json
A=Path(__file__).parent
out=['# Complete timing results','','Positive percentages mean less GPU time than the same-frame mean of the two controls.','Paired medians are the primary statistic; means and A/A are retained to show instability.','The disturbed `confirm-main-synthetic` 4K run is excluded from conclusions.','']
for d in sorted((A/'timings').iterdir()):
 files=sorted(d.glob('*.summary.json'))
 if not files:continue
 out += [f'## {d.name}','']
 for p in files:
  s=json.loads(p.read_text());size=p.name.split('.')[0]
  out += [f'### {size}', '',f'A/A difference of mean GPU times: {s["aa_mean_pct"]:.2f}%.','', '| Variant | Median ms | Mean ms | Paired median saving | Paired mean saving |','|---|---:|---:|---:|---:|']
  for n,v in s.items():
   if n=='aa_mean_pct':continue
   out += [f'| {n} | {v["median_ms"]:.3f} | {v["mean_ms"]:.3f} | {v["median_paired_saving_pct"]:.2f}% | {v["mean_paired_saving_pct"]:.2f}% |']
  out += ['']
(A/'RESULTS.md').write_text('\n'.join(out).rstrip()+'\n')
