import os
from pathlib import Path
import subprocess,os,numpy as np,json
root=Path(os.environ.get('FAR_REPLAY_ROOT','/Users/yan/.codex/worktrees/stars-near-quads/harmonigraph'))
out=Path(os.environ.get('FAR_OUTPUT','/tmp/far-coverage'));binary=root/'target/release/harmonigraph-offline'
for name,mode,debug in [('A',0,0),('B',1,0),('C',2,0),('diag-A',0,3),('diag-C',2,3)]:
 subprocess.run(['python3',str(Path(__file__).with_name('render.py')),name,str(binary),str(mode)],env=dict(os.environ,HG_FAR_DEBUG=str(debug)),check=True)
 if name=='A':
  a=np.memmap(out/'pristine.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4))
  b=np.memmap(out/'A.rgba',dtype=np.uint8,mode='r',shape=a.shape)
  total=changed=worst=0
  for x,y in zip(a,b):
   d=np.abs(x.astype(np.int16)-y.astype(np.int16));total+=int(d.sum());changed+=int(np.count_nonzero(d));worst=max(worst,int(d.max()))
  result=dict(max=worst,mean=total/a.size,changed_channels=changed,channels=a.size)
  (out/'baseline-parity.json').write_text(json.dumps(result,indent=2)+'\n');print('BASELINE',result,flush=True)
  assert worst<=1 and result['mean']<=0.005,result
