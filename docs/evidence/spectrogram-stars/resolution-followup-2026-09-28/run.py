from pathlib import Path
import subprocess,os,re,json,sys
R=Path(os.environ['RESEARCH_WORKTREE']); D=Path(os.environ['RES_OUTPUT']);D.mkdir(exist_ok=True)
exe=next(p for p in (R/'target/release/deps').glob('harmonigraph_render-*') if p.suffix=='')
cases={'A':('0.75','1'),'B':('0.5','1'),'C':('0.5','0.75'),'D':('0.5','0.75'),'control':('0.75','1')}
rows=[]
mode=sys.argv[1]
for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
 order=['A','B','C','D','D','C','B','A'] if mode=='timing' else ['A','B','C','D','control']
 for i,name in enumerate(order):
  far,near=cases[name]
  env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_CASE='stars, defaults',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_FRAMES='240',PROBE_FILLS='1',RES_FAR=far,RES_NEAR=near,RES_CASE='stars, defaults')
  if mode=='capture':env.update(RES_CAPTURE=str(D/f'{size}-{name}'),RES_INPUT='take',PROBE_SLABS='960',PROBE_SPAN_SEMITONES=str(12*__import__('math').log2(9000/70)))
  if name=='control': env['RES_EXTRA']='1'
  if name=='D': env['RES_HALOS']='1'
  log=D/f'{mode}-{size}-{i}-{name}.log'
  with log.open('w') as f:
   p=subprocess.run([str(exe),'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
  if p.returncode:print(log.read_text(),flush=True);raise SystemExit(p.returncode)
  m=re.search(r'stars, defaults / 1.00 / 10.0: ([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)',log.read_text());assert m
  row=dict(size=size,variant=name,index=i,stats=list(map(float,m.groups())));rows.append(row);print(row,flush=True)
  (D/f'{mode}.json').write_text(json.dumps(rows,indent=2))
