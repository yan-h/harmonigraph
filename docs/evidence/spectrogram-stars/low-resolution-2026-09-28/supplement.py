from pathlib import Path
import subprocess,os,json,re
D=Path(os.environ.get('RES_OUTPUT','/private/tmp/stars-low-resolution'));R=Path(os.environ['RESEARCH_WORKTREE']);exe=os.environ['RES_EXE']
mode=__import__('sys').argv[1];rows=json.loads((D/'confirmation.json').read_text()) if (D/'confirmation.json').exists() else []
if mode=='confirm':
 for size,ppp in [('1920x1080','2'),('3840x2160','4')]:
  for case in os.environ.get('RES_CONFIRM_CASES','L1,L2,V,X').split(','):
   env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_FRAMES='600',PROBE_FILLS='1',PROBE_CASE='M,'+case)
   log=D/f'confirm-{size}-{case}.log'
   with log.open('w') as f:p=subprocess.run([exe,'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
   assert p.returncode==0,log.read_text()
   for name,*stats in re.findall(r'^(H|M|M2|L1|L2|V|X) / 1.00 / 10.0: ([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)',log.read_text(),re.M):
    row=dict(size=size,variant=name,run=case,stats=list(map(float,stats)));rows.append(row);print(row,flush=True)
   (D/'confirmation.json').write_text(json.dumps(rows,indent=2)+'\n')
else:
 import math
 fdir=Path('/private/tmp/stars-low-flat');fdir.mkdir(exist_ok=True)
 for dst,src in [('take-levels.u8','flat-levels.u8'),('palette.rgba','palette.rgba')]:
  if not (fdir/dst).exists():(fdir/dst).symlink_to(Path('/private/tmp/stars-full-halo-compare')/src)
 for size,ppp,case,fixture in [('1920x1080','2','M2','take'),('3840x2160','4','M2','take')]+[('1920x1080','2',c,'flat') for c in ['H','M','L1','X']]:
  out=D/f'{size}-{fixture}-{case}'
  env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_FRAMES='2',PROBE_FILLS='1',RES_CASE=case,RES_CAPTURE=str(out),RES_INPUT=str(fdir) if fixture=='flat' else '/private/tmp/stars-full-halo-compare',PROBE_SLABS='960',PROBE_SPAN_SEMITONES=str(12*math.log2(9000/70)))
  log=D/f'control-{size}-{fixture}-{case}.log'
  with log.open('w') as f:p=subprocess.run([exe,'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=R,env=env,stdout=f,stderr=subprocess.STDOUT)
  assert p.returncode==0,log.read_text()
  if fixture=='take':
   for frame in ['0010.rgba','0011.rgba']:assert (out/frame).read_bytes()==(D/f'{size}-M'/frame).read_bytes()
   print(size,'M/M2 exact pixel parity',flush=True)
