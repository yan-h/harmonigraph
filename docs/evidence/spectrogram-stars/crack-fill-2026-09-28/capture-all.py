from pathlib import Path
import subprocess,os,numpy as np,json
out=Path('/tmp/crack-fill');binary='/Users/yan/.codex/worktrees/stars-near-quads/harmonigraph/target/release/harmonigraph-offline'
for name,mode,debug in [('A',0,0),('B',1,0),('C',2,0),('far-A',0,1),('far-B',1,1),('far-C',2,1)]:
 subprocess.run(['python3',str(out/'render.py'),name,binary,str(mode)],env=dict(os.environ,HG_FILL_DEBUG=str(debug),HG_FILL_DUMP=str(out/'shaders')),check=True)
 if name=='A':
  a=np.memmap('/tmp/far-coverage/pristine.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4));b=np.memmap(out/'A.rgba',dtype=np.uint8,mode='r',shape=a.shape)
  total=changed=worst=0
  for x,y in zip(a,b):
   d=np.abs(x.astype(np.int16)-y.astype(np.int16));total+=int(d.sum());changed+=int(np.count_nonzero(d));worst=max(worst,int(d.max()))
  result=dict(max=worst,mean=total/a.size,changed_channels=changed,channels=a.size);(out/'baseline-parity.json').write_text(json.dumps(result,indent=2)+'\n');print('BASELINE',result,flush=True);assert worst==0,result
