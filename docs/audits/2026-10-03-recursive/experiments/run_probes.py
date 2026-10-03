"""Run compiled scratch tests, and query the real dependency's native layout.

Invoke through with_shared_lock.py and session-lifecycle.sh after the probe build.
"""
from pathlib import Path
import json, os, re, subprocess, time
here=Path(__file__).resolve().parent; root=here.parents[3]; logs=here.parent/'logs'
text=(logs/'build-probes.log').read_text()
def binary(name):
    matches=re.findall(r'\((target/release/deps/'+name+r'-[^)]+)\)',text)
    assert len(set(matches))==1, (name,matches)
    return str(root/matches[-1])
def library(name):
    matches=list((root/'target/release/deps').glob('lib'+name+'-*.rlib'))
    assert len(matches)==1, matches
    return str(matches[0])
context={'before':subprocess.check_output(['ps','-axo','pid,ppid,%cpu,%mem,comm'],text=True),'load_before':os.getloadavg()}
exe=here/'output/atomic-status-probe'
exe.parent.mkdir(exist_ok=True)
cmd=['rustc','--edition=2021',str(here/'atomic_status_probe.rs'),'-L','dependency='+str(root/'target/release/deps'),'--extern','crossbeam_utils='+library('crossbeam_utils'),'--extern','nice_plug_core='+library('nice_plug_core'),'-o',str(exe)]
with (logs/'build-atomic-status.log').open('w') as f:
    f.write(repr(cmd)+'\n');f.flush();subprocess.run(cmd,cwd=root,stdout=f,stderr=subprocess.STDOUT,check=True)
env=os.environ|{'WGPU_BACKEND':'metal','HARMONIGRAPH_REQUIRE_GPU':'1'}
results=[]
for name,cmd in [('atomic-status',[str(exe)]),('hub',[binary('harmonigraph_plugin'),'audit_epoch_rejection_does_not_spend_the_collection_budget']),('naming',[binary('harmonigraph_ui'),'audit_fixed_just_pitch_uses_current_temperament_and_reaches_fallback']),('nonfinite',[binary('harmonigraph_ui'),'audit_display_recovers_after_one_nonfinite_audio_sample']),('targets',[binary('harmonigraph_render'),'audit_halo_only_resize_prepare_cpu_probe'])]:
    if name!='atomic-status':cmd+=['--nocapture','--test-threads=1']
    before=time.perf_counter();path=logs/('probe-'+name+'.log')
    with path.open('w') as f:r=subprocess.run(cmd,cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT)
    results.append({'name':name,'command':cmd,'exit':r.returncode,'wall_seconds':time.perf_counter()-before,'log':str(path.relative_to(here.parent))})
    print(name,r.returncode,flush=True)
    if name!='atomic-status':assert 'running 1 test' in path.read_text(), 'fixture not reached'
context['after']=subprocess.check_output(['ps','-axo','pid,ppid,%cpu,%mem,comm'],text=True);context['load_after']=os.getloadavg()
(logs/'probe-context.json').write_text(json.dumps(context,indent=2)+'\n')
(logs/'probe-results.json').write_text(json.dumps(results,indent=2)+'\n')
