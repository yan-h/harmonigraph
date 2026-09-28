from pathlib import Path
import os,subprocess,json,hashlib,sys
out=Path('/private/tmp/stars-next'); root=Path(os.environ.get('RESEARCH_WORKTREE',os.getcwd()))
exe=max((p for p in (root/'target/release/deps').glob('harmonigraph_render-*') if p.is_file() and os.access(p,os.X_OK) and p.suffix==''),key=lambda p:p.stat().st_mtime)
label=sys.argv[1]; dest=out/label;dest.mkdir(exist_ok=False)
base=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1')
meta={'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=root,text=True).strip(),'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'shaders':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.glob('*.wgsl')},'env':{k:v for k,v in base.items() if k.startswith(('PROBE_','RESEARCH_','IMAGE_'))}}
(dest/'manifest.json').write_text(json.dumps(meta,indent=2))
for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
 env=dict(base,PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_RAW=str(dest/(size+'.csv')),PROBE_FRAMES=base.get('PROBE_FRAMES','240'),PROBE_WARMUP='60',PROBE_INPUT=base.get('PROBE_INPUT','take'))
 print('start',label,size,flush=True)
 with (dest/(size+'.log')).open('w') as f: subprocess.run([str(exe),'stars_live_production_timings','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
 subprocess.run([sys.executable,str(out/'analyze.py'),str(dest/(size+'.csv'))],stdout=(dest/(size+'.summary.json')).open('w'),check=True)
 print('done',label,size,flush=True)
