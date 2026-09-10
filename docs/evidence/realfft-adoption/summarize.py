"""Summaries over measured rows; neither acceptance tolerance nor golden blessing."""
import csv,collections,pathlib,statistics
r=pathlib.Path('/tmp/realfft-adoption'); out=r/'comparisons'
with open(out/'gates-summary.csv','w') as f:
 w=csv.writer(f,lineterminator='\n');w.writerow(['variant','n','tapers','reading','node_observations','changed_observations','changed_nodes','first_phase','last_phase','max_fade_delta'])
 a=list(csv.DictReader(open(r/'baseline/nodes.csv')))
 for variant in ['candidate','scalar']:
  p=r/variant/'nodes.csv'
  if not p.exists():continue
  b=list(csv.DictReader(open(p)));assert len(a)==len(b)
  groups=collections.defaultdict(list)
  for x,y in zip(a,b):
   assert tuple(x[k] for k in ['n','tapers','reading','phase','node','cents'])==tuple(y[k] for k in ['n','tapers','reading','phase','node','cents'])
   groups[x['n'],x['tapers'],x['reading']].append((x,y))
  for key,pairs in groups.items():
   changed=[(x,y) for x,y in pairs if x['level']!=y['level']]
   phases=[int(x['phase']) for x,y in changed]
   w.writerow([variant,*key,len(pairs),len(changed),len({x['node'] for x,y in changed}),min(phases,default=''),max(phases,default=''),max((abs(float(x['level'])-float(y['level'])) for x,y in changed),default=0)])
with open(out/'analyzer-summary.csv','w') as f:
 w=csv.writer(f,lineterminator='\n');w.writerow(['n','tapers','input','baseline_ns','candidate_ns','saved_ns_median','saved_ns_min','saved_ns_max','retained_baseline','retained_candidate','candidate_warm_allocations'])
 groups=collections.defaultdict(lambda:collections.defaultdict(list))
 for i in range(7):
  for variant in ['baseline','candidate']:
   for row in csv.DictReader(open(out/f'analyzer-{i}-{variant}.csv')):
    groups[row['n'],row['tapers'],row['input']][variant].append(row)
 for key,rows in groups.items():
  a=rows['baseline'];b=rows['candidate'];d=[float(x['ns_per_column'])-float(y['ns_per_column']) for x,y in zip(a,b)]
  w.writerow([*key,statistics.median(float(x['ns_per_column']) for x in a),statistics.median(float(x['ns_per_column']) for x in b),statistics.median(d),min(d),max(d),a[0]['retained_bytes'],b[0]['retained_bytes'],max(int(x['steady_allocs']) for x in b)])
