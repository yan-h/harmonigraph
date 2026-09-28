from pathlib import Path
import os,subprocess,json,math,numpy as np
R=Path(os.environ['RESEARCH_WORKTREE']);D=Path('/private/tmp/stars-low-port');D.mkdir(exist_ok=True)
items=[json.loads(l) for l in Path('/private/tmp/low-parity-build.json').read_text().splitlines() if l.startswith('{')]
exe=next(x['executable'] for x in items if x.get('reason')=='compiler-artifact' and x.get('executable') and x['target']['name']=='harmonigraph_render')
results={}
for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
 out=D/size;env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_FRAMES='2',PROBE_FILLS='1',RES_CASE='L1',RES_CAPTURE=str(out),RES_INPUT='/private/tmp/stars-full-halo-compare',PROBE_SLABS='960',PROBE_SPAN_SEMITONES=str(12*math.log2(9000/70)))
 with (D/f'{size}.log').open('w') as f:p=subprocess.run([exe,'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
 assert p.returncode==0,(D/f'{size}.log').read_text()
 for frame in ['0010.rgba','0011.rgba']:
  a=np.fromfile(Path('/private/tmp/stars-low-resolution')/f'{size}-L1'/frame,np.uint8)
  b=np.fromfile(out/frame,np.uint8);assert a.shape==b.shape
  d=np.abs(a.astype(np.int16)-b.astype(np.int16));results[size+'/'+frame]={'max_channel_error':int(d.max()),'mean_channel_error_255':float(d.mean()),'changed_channels':int(np.count_nonzero(d)),'total_channels':int(d.size)}
  assert d.max()<=1,results
print(json.dumps(results,indent=2));(D/'parity.json').write_text(json.dumps(results,indent=2)+'\n')
