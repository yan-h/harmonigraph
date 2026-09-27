import os,subprocess,json,hashlib,time
from pathlib import Path
root=Path('/Users/yan/.codex/worktrees/stars-investigation/harmonigraph')
out=Path('/private/tmp/stars-investigation')
exe=root/'target/release/deps/harmonigraph_render-6209358007c8e797'
base=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1')
manifest={'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'started':time.time(),'runs':[]}
for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
    name='live-take-'+size
    env=dict(base,PROBE_INPUT='take',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_JITTER='0.5',PROBE_FILLS='1',PROBE_WARMUP='60',PROBE_FRAMES='240',PROBE_RAW=str(out/(name+'.csv')))
    for k in ('RESEARCH_CASES','PROBE_STAGES'): env.pop(k,None)
    print('starting',name,flush=True)
    with (out/(name+'.log')).open('w') as f:
        subprocess.run([str(exe),'spectrogram::tests::live_timing::stars_live_production_timings','--ignored','--exact','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
    with (out/(name+'-analysis.json')).open('w') as f:
        subprocess.run(['/opt/homebrew/bin/python3',str(out/'analyze_timings.py'),str(out/(name+'.csv'))],stdout=f,check=True)
    manifest['runs'].append({'name':name,'size':size,'ppp':ppp})
    (out/'live-validation-manifest.json').write_text(json.dumps(manifest,indent=2))
for name,ppp in [('1080','2'),('4k','4')]:
    dest=out/('images-live-'+name)
    print('starting images',name,flush=True)
    env=dict(base,LIVE_PARITY_OUTPUT=str(dest),LIVE_PARITY_PPP=ppp)
    with (out/('live-images-'+name+'.log')).open('w') as f:
        subprocess.run([str(exe),'spectrogram::tests::live_parity::stars_live_production_images','--ignored','--exact','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
    result=subprocess.run(['/opt/homebrew/bin/python3',str(out/'live-parity-proposal/compare.py'),str(dest)],capture_output=True,text=True)
    (out/('live-parity-'+name+'.log')).write_text(result.stdout+result.stderr)
    print('parity',name,'exit',result.returncode,flush=True)
print('all captures finished',flush=True)
