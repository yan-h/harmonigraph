"""Serialize audit measurements. Invoke only after all audit compilation finishes.

Uses one advisory local lock shared with an Otonal audit that follows this recipe.
Desktop applications stay running; their load is logged, not controlled.
"""
from pathlib import Path
import argparse, datetime, fcntl, json, os, subprocess, time
root=Path(__file__).resolve().parents[4]
audit=Path(__file__).resolve().parents[1]
logs=audit/'logs'; inputs=audit/'experiments/inputs'; output=audit/'experiments/output'
output.mkdir(exist_ok=True)
p=argparse.ArgumentParser(); p.add_argument('kind',choices=['ui','export','gpu']); p.add_argument('--rounds',type=int,default=3); p.add_argument('--case',choices=['all','chord720','chord1080','historical1080'],default='all'); p.add_argument('--tag',default=''); p.add_argument('--test-binary',type=Path); a=p.parse_args()
if a.rounds < 1 or any(c not in 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_' for c in a.tag): raise SystemExit('positive rounds and a simple tag required')
def binary(name):
    if a.test_binary: return str(a.test_binary.resolve())
    matches=[p for p in (root/'target/release/deps').glob(name+'-*') if p.is_file() and not p.suffix and os.access(p,os.X_OK)]
    if len(matches)!=1: raise SystemExit(f'Choose exact test binary; found {matches}')
    return str(matches[0])
def snapshot():
    r=subprocess.run(['ps','-axo','pid,ppid,%cpu,%mem,comm'],capture_output=True,text=True,check=True)
    rows=sorted(r.stdout.splitlines()[1:],key=lambda l:float(l.split()[2]),reverse=True)
    return {'utc':datetime.datetime.now(datetime.timezone.utc).isoformat(),'loadavg':os.getloadavg(),'processes':rows[:15]}
def run(label,cmd,env=None):
    before=snapshot()
    # Avoid accidental overlap with another compiler/export; game/DAW remain untouched.
    for l in subprocess.check_output(['ps','-axo','comm'],text=True).splitlines():
        if Path(l.strip()).name in {'rustc','cargo','ffmpeg','harmonigraph-offline'}:
            raise SystemExit('Competing build/export process: '+l)
    path=logs/(label+'.log')
    if path.exists(): raise SystemExit('Preserve prior result; choose a fresh --tag: '+str(path))
    e=os.environ.copy(); e.update({'WGPU_BACKEND':'metal','HARMONIGRAPH_REQUIRE_GPU':'1'}); e.update(env or {})
    t=time.perf_counter()
    with path.open('w') as f: r=subprocess.run(['/usr/bin/time','-l',*cmd],cwd=root,stdout=f,stderr=subprocess.STDOUT,env=e)
    record={'label':label,'command':cmd,'env':{k:e[k] for k in ['WGPU_BACKEND','HARMONIGRAPH_REQUIRE_GPU',*(env or {})]},'before':before,'after':snapshot(),'wall_seconds':time.perf_counter()-t,'exit':r.returncode,'log':str(path.relative_to(audit))}
    with (logs/'measurements.jsonl').open('a') as f:f.write(json.dumps(record)+'\n')
    print(label,record['wall_seconds'],r.returncode,flush=True)
    if r.returncode: raise SystemExit(r.returncode)
with open('/private/tmp/yan-deep-audits-expensive.lock','w') as lock:
    fcntl.flock(lock,fcntl.LOCK_EX)
    if a.kind=='ui':
        exe=binary('harmonigraph_ui')
        for i in range(a.rounds): run((a.tag+'-' if a.tag else '')+f'ui-runtime-{i}',[exe,'tests::profile::profile_visual_runtime','--exact','--ignored','--nocapture','--test-threads=1'])
    elif a.kind=='export':
        exe=str(root/'target/release/harmonigraph-offline')
        cases={'chord720':('chord','1280x720','0','8'), 'chord1080':('chord','1920x1080','0','8'), 'historical1080':('historical','1920x1080','0.66','4.66')}
        cases=list(cases.values()) if a.case=='all' else [cases[a.case]]
        for i in range(a.rounds):
            for name,size,start,end in (cases if i%2==0 else list(reversed(cases))):
                label=(a.tag+'-' if a.tag else '')+f'export-{name}-{size}-{i}'
                run(label,[exe,str(inputs/(name+'.take')),'--out',str(output/(label+'.mp4')),'--size',size,'--fps','60','--start',start,'--end',end,'--tail','0'])
    elif a.kind=='gpu':
        exe=binary('harmonigraph_render')
        for i in range(a.rounds):
            run((a.tag+'-' if a.tag else '')+f'lattice-gpu-{i}',[exe,'lattice_tests::timing::atmosphere_costs_by_polyphony','--exact','--ignored','--nocapture','--test-threads=1'],{'PROBE_SIZE':'1536','PROBE_NOTES':'12','PROBE_CASE':'all','PROBE_FRAMES':'60','PROBE_BLOOM':'1','PROBE_TIMER':'1'})
