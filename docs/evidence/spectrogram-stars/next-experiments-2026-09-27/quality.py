from pathlib import Path
import json,numpy as np,sys
p=Path(sys.argv[1]);report=[]
for f in p.glob('full-a-*.rgba'):
 suffix=f.name[len('full-a'):];ref=np.fromfile(f,np.uint8).reshape(-1,4)
 for n in ['p3-a','blur-a','blur-fused']:
  c=p/(n+suffix)
  if not c.exists():continue
  b=np.fromfile(c,np.uint8).reshape(-1,4);d=b[:,:3].astype(np.int16)-ref[:,:3]
  report.append({'case':n,'frame':f.stem.split('-')[-1],'input':f.stem.split('-')[-2],'mae':float(np.abs(d).mean()),'mse':float((d.astype(float)**2).mean()),'max':int(np.abs(d).max())})
(p/'quality-vs-full.json').write_text(json.dumps(report,indent=2))
for n in ['p3-a','blur-a','blur-fused']:
 q=[r for r in report if r['case']==n];print(n,'MSE',np.mean([r['mse'] for r in q]),'max',max([r['max'] for r in q],default=0))
