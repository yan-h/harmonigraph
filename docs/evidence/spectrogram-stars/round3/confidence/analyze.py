from pathlib import Path
import csv,json,argparse,gzip
import numpy as np
parser=argparse.ArgumentParser(description="Summarize paired GPU comparisons with a 24-round block bootstrap.")
parser.add_argument("root", type=Path, nargs="?", default=Path("/private/tmp/stars-confidence"))
parser.add_argument("--synthetic-only", action="store_true", help="Analyze only the four synthetic runs")
args=parser.parse_args()
root=args.root
expected={"synthetic-4k-17","synthetic-1080-41","synthetic-4k-53","synthetic-1080-97"}
if not args.synthetic_only:expected.update({"recording-4k-29","recording-4k-67","recording-1080-79","recording-1080-83"})
paths=sorted(p for p in root.iterdir() if (p.name.endswith(".csv") or p.name.endswith(".csv.gz")) and (not args.synthetic_only or p.name.startswith("synthetic-")))
run_name=lambda p:p.name.removesuffix(".gz").removesuffix(".csv")
assert len(paths)==len(expected) and {run_name(p) for p in paths} == expected, "Missing or unexpected comparison CSVs"
results={}
for path in paths:
 with (gzip.open(path,"rt") if path.suffix==".gz" else path.open()) as stream:
  rows=list(csv.DictReader(stream))
 names=['base_a','base_b','dust100','dust075'];frames=len(rows)//4
 assert len(rows)==2400 and frames==600
 arrays={metric:np.full((frames,4),np.nan) for metric in ['gpu_ms','begin_gpu_ms','wall_ms','cpu_prepare_ms']}
 slots=np.zeros((4,4),dtype=int)
 seen=set(); orders=np.full((frames,4),-1,dtype=int)
 for row in rows:
  f=int(row['frame']);c=names.index(row['case']);slot=int(row['slot'])
  assert 0<=f<frames and 0<=slot<4 and (f,c) not in seen and orders[f,slot]==-1
  seen.add((f,c)); orders[f,slot]=c;slots[c,slot]+=1
  for key,values in arrays.items():values[f,c]=float(row[key])
 assert np.all(slots==150),slots
 assert all(len({tuple(order) for order in orders[start:start+24]})==24 for start in range(0,frames,24))
 assert all(np.isfinite(v).all() for v in arrays.values())
 entry={'frames':frames,'slots':slots.tolist(),'metrics':{}}
 for metric,values in arrays.items():
  ref=values[:,:2].mean(axis=1)
  blocks=values.reshape(25,24,4).mean(axis=1)
  ref_blocks=blocks[:,:2].mean(axis=1)
  rng=np.random.default_rng(12345)
  indices=rng.integers(0,25,(10000,25))
  m={'median_ms':dict(zip(names,np.median(values,axis=0).tolist())), 'mean_ms':dict(zip(names,values.mean(axis=0).tolist())), 'p10_p90_ms':dict(zip(names,np.percentile(values,[10,90],axis=0).T.tolist()))}
  for c in [1,2,3]:
   base=values[:,0] if c==1 else ref
   base_blocks=blocks[:,0] if c==1 else ref_blocks
   diff=base-values[:,c]
   benefit=(1-values[:,c].mean()/base.mean())*100
   bootstrap=(1-blocks[indices,c].mean(axis=1)/base_blocks[indices].mean(axis=1))*100
   b=(1-blocks[:,c]/base_blocks)*100
   m[names[c]]={'mean_saving_percent':float(benefit),'paired_median_saving_ms':float(np.median(diff)),'block_bootstrap_95_percent':np.percentile(bootstrap,[2.5,97.5]).tolist(),'block_saving_percent_min_median_max':[float(b.min()),float(np.median(b)),float(b.max())],'positive_blocks':int((b>0).sum())}
  entry['metrics'][metric]=m
 results[run_name(path)]=entry
(root/'analysis.json').write_text(json.dumps(results,indent=2)+'\n')
for name,entry in results.items():
 m=entry['metrics']['begin_gpu_ms'];end=entry['metrics']['gpu_ms'];wall=entry['metrics']['wall_ms']
 print(name, 'median GPU', {k:round(v,2) for k,v in m['median_ms'].items()})
 for c in ['base_b','dust100','dust075']:
  v=m[c];print(' ',c,'mean saving',round(v['mean_saving_percent'],2),'95% blocks',*[round(x,2) for x in v['block_bootstrap_95_percent']], 'positive blocks',v['positive_blocks'],'end saving',round(end[c]['mean_saving_percent'],2),'wall saving',round(wall[c]['mean_saving_percent'],2))
