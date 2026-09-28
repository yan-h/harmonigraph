from pathlib import Path
import subprocess,os,json,statistics,re,sys,hashlib
root=Path(os.environ.get('QUAD_REPLAY_ROOT', Path.cwd())).resolve()
out=Path(os.environ.get('QUAD_RESULTS', '/tmp/near-quads-replay'));out.mkdir(parents=True,exist_ok=True)
binary=max((p for p in (root/'target/release/deps').glob('harmonigraph_render-*') if p.is_file() and os.access(p,os.X_OK)), key=lambda p:p.stat().st_mtime)
phase=sys.argv[1] if len(sys.argv)>1 else 'screen'
cases={
 'screen': [('small','926x720','1','10',None,'1'),('1080','1920x1080','2','10',None,'1'),('4k','3840x2160','4','10',None,'1'),('sparse1080','1920x1080','2','0.5',None,'1')],
 'repeat': [('4k-repeat','3840x2160','4','10',None,'1'),('1080-repeat','1920x1080','2','10',None,'1'),('recorded1080','1920x1080','2','10','/private/tmp/stars-full-halo-compare/take-levels.u8','1'),('no-memory1080','1920x1080','2','10',None,'0')],
}[phase]
for name,size,ppp,density,recording,memory in cases:
 env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_CASE='quad',PROBE_FRAMES='240',PROBE_WARMUP='120',PROBE_FILLS='1',PROBE_HISTORY_SECONDS='39.149967',PROBE_SPAN_SEMITONES='90.02282',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_STAR_DENSITY=density,PROBE_MEMORY=memory)
 if recording:env['PROBE_RECORDING']=recording
 log=out/f'{name}.log'
 with log.open('w') as f: subprocess.run([str(binary),'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
 samples={}
 for line in log.read_text().splitlines():
  if line.startswith('RAW '):
   label,data=line[4:].split(' [',1);samples[label]=json.loads('['+data)
 a,b=samples['quad A'],samples['quad A2'];control=[(x+y)/2 for x,y in zip(a,b)]
 row=dict(size=size,ppp=ppp,density=density,memory=memory,recording=recording,medians={k:statistics.median(v) for k,v in samples.items()},aa_percent=100*(statistics.median(b)/statistics.median(a)-1),paired_savings={k:100*statistics.median([(x-y)/x for x,y in zip(control,v)]) for k,v in samples.items() if k not in ['quad A','quad A2']})
 (out/f'{name}.json').write_text(json.dumps(row,indent=2)+'\n')
 print(name,json.dumps(row),flush=True)
