from pathlib import Path
import subprocess,os,sys,json
import numpy as np
from png import write_png
A=Path(__file__).parent;R=Path(os.environ['RESEARCH_WORKTREE'])
label,names=sys.argv[1:3];ppp=sys.argv[3] if len(sys.argv)>3 else '1';jitter=sys.argv[4] if len(sys.argv)>4 else '0.5';origin=sys.argv[5] if len(sys.argv)>5 else '0';D=A/'probes'/label;D.mkdir(parents=True,exist_ok=False)
env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',HG_ARCH_DIR=str(A/'shaders'),HG_FIXTURE=os.environ.get('HG_FIXTURE','/private/tmp/stars-full-halo-compare'),RESEARCH_CASES=names,IMAGE_OUTPUT=str(D),IMAGE_FRAMES='3',PROBE_PPP=ppp,PROBE_JITTER=jitter,IMAGE_ORIGIN=origin)
with (D/'capture.log').open('w') as log:subprocess.run([str(Path(os.environ['RESEARCH_TEST_BINARY'])),'stars_next_images','--ignored','--nocapture','--test-threads=1'],env=env,cwd=R,stdout=log,stderr=subprocess.STDOUT,check=True)
m=json.loads((D/'manifest.json').read_text());shape=(m['height'],m['width'],4);metrics={}
for input in ['take','flat']:
 for f in range(3):
  arrays={n:np.fromfile(D/f'{n}-{input}-{f}.rgba',np.uint8).reshape(shape) for n in names.split(',')}
  for n,a in arrays.items():
   if f==1:write_png(D/f'{n}-{input}.png',a[:,:,:3])
  for left,right in [('base-a','long-ref'),('base-a','micro'),('base-a','complete'),('direct4','direct4-ref9'),('direct4','full-residual4'),('full-a','group5-100')]:
   if left in arrays and right in arrays:
    delta=np.abs(arrays[left].astype(np.int16)-arrays[right].astype(np.int16));metrics[f'{left}:{right}:{input}:{f}']={'max':int(delta.max()),'mean_rgb':float(delta[:,:,:3].mean()),'changed_channels':int(np.count_nonzero(delta)),'alpha_max':int(delta[:,:,3].max())}
(D/'metrics.json').write_text(json.dumps(metrics,indent=2));print(label,json.dumps(metrics),flush=True)
