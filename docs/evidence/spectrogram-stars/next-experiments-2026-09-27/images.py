from pathlib import Path
import os,subprocess,json,sys,numpy as np
root=Path(os.environ.get('RESEARCH_WORKTREE',os.getcwd())); out=Path('/private/tmp/stars-next');dest=out/sys.argv[1];dest.mkdir(exist_ok=False)
exe=max((p for p in (root/'target/release/deps').glob('harmonigraph_render-*') if p.is_file() and os.access(p,os.X_OK) and p.suffix==''),key=lambda p:p.stat().st_mtime)
env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',IMAGE_OUTPUT=str(dest))
with (dest/'capture.log').open('w') as f: subprocess.run([str(exe),'stars_next_images','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
meta=json.loads((dest/'manifest.json').read_text());meta['env']={k:v for k,v in env.items() if k.startswith(('PROBE_','RESEARCH_','IMAGE_'))};(dest/'manifest.json').write_text(json.dumps(meta,indent=2))
report=[]
for f in dest.glob('*.rgba'):
 name,inp,frame=f.stem.rsplit('-',2)
 if name in ['p3-a','full-a','blur-a']:continue
 refname='full-a' if name.startswith('full') else 'blur-a' if name.startswith('blur') else 'p3-a'
 ref=dest/f'{refname}-{inp}-{frame}.rgba'
 if not ref.exists():continue
 a=np.fromfile(ref,np.uint8).reshape(-1,4);b=np.fromfile(f,np.uint8).reshape(-1,4);d=b.astype(np.int16)-a
 report.append({'case':name,'input':inp,'frame':int(frame),'max':int(np.abs(d).max()),'mae':float(np.abs(d[:,:3]).mean()),'mse':float((d[:,:3].astype(float)**2).mean()),'changed_channels':int(np.count_nonzero(d)),'alpha_changed':int(np.count_nonzero(d[:,3]))})
(dest/'metrics.json').write_text(json.dumps(report,indent=2))
print(sys.argv[1], 'worst',max((r['max'] for r in report),default=None),'rows',len(report))
