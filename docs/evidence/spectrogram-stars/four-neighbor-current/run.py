"""Paired current-default Stars comparison; apply harness.patch first."""
from pathlib import Path
import argparse,csv,json,os,statistics,subprocess
parser=argparse.ArgumentParser(__doc__)
parser.add_argument('--binary',type=Path,required=True)
parser.add_argument('--output',type=Path,required=True)
args=parser.parse_args()
root=Path(__file__).resolve().parents[4]
binary=args.binary.resolve()
out=args.output.resolve()
if out.exists() and any(out.iterdir()):parser.error('output must be empty')
out.mkdir(parents=True,exist_ok=True)
results={}
for label,size,offset in [('1080-a','1920x1080',0),('4k-a','3840x2160',0),('4k-b','3840x2160',3),('1080-b','1920x1080',3)]:
    env={k:v for k,v in os.environ.items() if not k.startswith(('PROBE_','STARS_DIAG_'))}
    env.update(HARMONIGRAPH_REQUIRE_GPU='1',HARMONIGRAPH_SHADER_ASSETS='source',PROBE_CASE='paired',PROBE_FILLS='1',PROBE_FRAMES='240',PROBE_SIZE=size,PROBE_ORDER_OFFSET=str(offset),PROBE_RAW=str(out/(label+'.csv')))
    with (out/(label+'.log')).open('w') as f:
        subprocess.run([str(binary),'cloud_costs_by_style_and_dial','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=f,stderr=subprocess.STDOUT,check=True)
    grouped={}
    for r in csv.DictReader((out/(label+'.csv')).open()):grouped.setdefault(r['case'],[]).append(float(r['gpu_ms']))
    means={k:statistics.mean(v) for k,v in grouped.items()}
    base=(means['paired baseline A']+means['paired baseline B'])/2
    results[label]={'means_ms':means,'reduction_percent':100*(1-means['paired four']/base),'aa_percent':100*(1-means['paired baseline B']/means['paired baseline A'])}
    print(label,json.dumps(results[label]),flush=True)
(out/'analysis.json').write_text(json.dumps(results,indent=2)+'\n')
