from pathlib import Path
import os,subprocess,json,hashlib,sys,argparse,csv,statistics
A=Path(__file__).parent; R=Path(os.environ['RESEARCH_WORKTREE'])
p=argparse.ArgumentParser();p.add_argument('label');p.add_argument('cases');p.add_argument('--frames',default=120,type=int);p.add_argument('--sizes',default='1920x1080:2,3840x2160:4');p.add_argument('--input',default='take');p.add_argument('--profile',default='default');p.add_argument('--jitter',default='0.5');p.add_argument('--offset',default='0');args=p.parse_args()
exe=Path(os.environ['RESEARCH_TEST_BINARY']); dest=A/'timings'/args.label;dest.mkdir(parents=True,exist_ok=False)
env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',HG_ARCH_DIR=str(A/'shaders'),HG_FIXTURE=os.environ.get('HG_FIXTURE','/private/tmp/stars-full-halo-compare'),RESEARCH_CASES=args.cases,PROBE_FRAMES=str(args.frames),PROBE_WARMUP='120',PROBE_INPUT=args.input,PROBE_HISTORY_SECONDS='39.149967',PROBE_SPAN_SEMITONES='90.02282',PROBE_ORDER_OFFSET=args.offset,PROBE_PROFILE=args.profile,PROBE_JITTER=args.jitter)
meta={'base':subprocess.check_output(['git','rev-parse','HEAD'],cwd=R,text=True).strip(),'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'shaders':{n:hashlib.sha256((A/'shaders'/f'{n}.wgsl').read_bytes()).hexdigest() for n in args.cases.split(',')},'environment':{k:v for k,v in env.items() if k.startswith(('PROBE_','HG_','RESEARCH_'))}}
(dest/'manifest.json').write_text(json.dumps(meta,indent=2))
for item in args.sizes.split(','):
 size,ppp=item.split(':'); csvpath=dest/f'{size}.csv'
 running=subprocess.check_output(['ps','-axo','pid,etime,%cpu,command'],text=True);(dest/f'{size}-processes.txt').write_text('\n'.join(line for line in running.splitlines() if any(word in line for word in ['harmonigraph','cargo ','rustc ','ffmpeg','hg-arch'])))
 print('start',args.label,size,flush=True)
 with (dest/f'{size}.log').open('w') as f:
  subprocess.run([str(exe),'stars_live_production_timings','--ignored','--nocapture','--test-threads=1'],cwd=R,env=dict(env,PROBE_SIZE=size,PROBE_PPP=ppp,PROBE_RAW=str(csvpath)),stdout=f,stderr=subprocess.STDOUT,check=True)
 rows=list(csv.DictReader(csvpath.open()));cases=args.cases.split(','); by={n:{} for n in cases}
 for row in rows:by[row['case']][int(row['frame'])]=float(row['gpu_ms'])
 refs=cases[:2];control={i:(by[refs[0]][i]+by[refs[1]][i])/2 for i in by[refs[0]]}
 summary={n:{'median_ms':statistics.median(v.values()),'mean_ms':statistics.mean(v.values()),'median_paired_saving_pct':100*(1-statistics.median(v[i]/control[i] for i in v)),'mean_paired_saving_pct':100*(1-statistics.mean(v[i]/control[i] for i in v))} for n,v in by.items()}
 summary['aa_mean_pct']=100*(statistics.mean(by[refs[0]].values())/statistics.mean(by[refs[1]].values())-1)
 (dest/f'{size}.summary.json').write_text(json.dumps(summary,indent=2));print(size,json.dumps(summary),flush=True)
