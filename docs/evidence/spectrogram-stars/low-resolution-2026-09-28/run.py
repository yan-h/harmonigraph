from pathlib import Path
import subprocess,os,re,json,sys,hashlib,math
R=Path(os.environ['RESEARCH_WORKTREE'])
D=Path(os.environ.get('RES_OUTPUT','/private/tmp/stars-low-resolution'));D.mkdir(exist_ok=True)
exe=Path(os.environ['RES_EXE'])
mode=sys.argv[1]
rows=[]
for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
 for case in (['forward','reverse'] if mode=='timing' else ['H','M','L1','L2','V','X']):
  env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_FRAMES='240',PROBE_FILLS='1')
  if case=='reverse':env['RES_REVERSE']='1'
  if mode=='capture':env.update(RES_CASE=case,RES_CAPTURE=str(D/f'{size}-{case}'),RES_INPUT='/private/tmp/stars-full-halo-compare',PROBE_SLABS='960',PROBE_SPAN_SEMITONES=str(12*math.log2(9000/70)),PROBE_FRAMES='360')
  log=D/f'{mode}-{size}-{case}.log'
  with log.open('w') as f:
   p=subprocess.run([str(exe),'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
  if p.returncode:print(log.read_text(),flush=True);raise SystemExit(p.returncode)
  for name,*stats in re.findall(r'^(H|M|M2|L1|L2|V|X) / 1.00 / 10.0: ([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)',log.read_text(),re.M):
   row=dict(size=size,variant=name,run=case,stats=list(map(float,stats))); rows.append(row)
   print(row,flush=True)
  (D/f'{mode}.json').write_text(json.dumps(rows,indent=2)+'\n')
print('done '+mode,flush=True)
