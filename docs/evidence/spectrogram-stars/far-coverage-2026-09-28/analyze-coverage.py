import os
from pathlib import Path
import numpy as np,json,sys
src=Path(os.environ.get('FAR_OUTPUT','/tmp/far-coverage'));out=src/'coverage-metrics.json'
roi=(slice(64,824),slice(16,1416))
def movie(n):return np.memmap(src/f'{n}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4))
def stats(hist):
 n=int(hist.sum());cdf=hist.cumsum();return dict(pixels=n,mean=float(np.dot(hist,np.arange(256))/n/255),p01=int(np.searchsorted(cdf,n*.01))/255,p10=int(np.searchsorted(cdf,n*.1))/255,below_90_pct=float(hist[:230].sum()/n*100),below_99_pct=float(hist[:253].sum()/n*100),uncovered_weight=float(np.dot(hist,1-np.arange(256)/255)/n))
result={'roi_xywh':[16,64,1400,760],'thresholds':'source G >= 51/255 in both A and C for actual opacity distributions; coverage <230/255 and <253/255; code-valued means not linear luminance','flat':{},'actual':{}}
for mode in range(3):
 hist=np.zeros(256,np.int64)
 for p in (src/'flat').glob(f'mode-{mode}-frame-*.rgba'):
  a=np.memmap(p,dtype=np.uint8,mode='r',shape=(865,1537,4));hist+=np.bincount(a[roi][...,0].ravel(),minlength=256)
 result['flat']['ABC'[mode]]=stats(hist)
a,c,full,far=map(movie,['diag-A','diag-C','A','far-A']);nearmax=0;nearchanged=0
for name,raw in [('A',a),('C',c)]:
 hist=np.zeros(256,np.int64);nhist=np.zeros(256,np.int64);count=0;visible_gap=0
 for i in range(144):
  frame=raw[i][roi];mask=(a[i][roi][...,1]>=51)&(c[i][roi][...,1]>=51)
  hist+=np.bincount(frame[...,0][mask],minlength=256);nhist+=np.bincount(frame[...,2][mask],minlength=256)
  count+=int(mask.sum());visible_gap+=int(((frame[...,0]<230)&(frame[...,2]<230)&mask).sum())
 result['actual'][name]={'far':stats(hist),'near':stats(nhist),'far_and_near_below_90_pct':100*visible_gap/count}
# Far-only comparison: both images gamma-coded, a descriptive brightness proxy.
bright=dim=0;dimnear=0;covsum=0
for i in range(144):
 near_delta=np.abs(a[i][roi][...,2].astype(np.int16)-c[i][roi][...,2]);nearmax=max(nearmax,int(near_delta.max()));nearchanged+=int(np.count_nonzero(near_delta))
 la=np.einsum('...c,c->...',full[i][roi][...,:3].astype(np.float32),[.2126,.7152,.0722]);lf=np.einsum('...c,c->...',far[i][roi][...,:3].astype(np.float32),[.2126,.7152,.0722]);d=a[i][roi]
 mask=(d[...,1]>=51)&(d[...,0]>=253)&(lf>=20)
 dark=mask&(la<.75*lf);bright+=int(mask.sum());dim+=int(dark.sum());dimnear+=int((dark&(d[...,2]>=230)).sum());covsum+=float(d[...,2][dark].sum())
result['near_identity']={'max_u8_delta':nearmax,'changed_samples':nearchanged}
result['near_darkening']={'eligible_pixels':bright,'eligible_condition':'G>=51, far coverage>=253, far-only weighted RGB>=20','darkened_by_25pct_pixels':dim,'darkened_pct':100*dim/bright,'darkened_with_near_coverage_at_least_230_pct':100*dimnear/max(dim,1),'mean_near_coverage_when_darkened':covsum/max(dim,1)/255}
out.write_text(json.dumps(result,indent=2)+'\n');print(out.read_text())
