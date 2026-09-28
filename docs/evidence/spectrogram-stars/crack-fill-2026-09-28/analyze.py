from pathlib import Path
import numpy as np,json
src=Path('/tmp/crack-fill');roi=(slice(64,824),slice(16,1416));result={'flat':{},'pictures':{}}
for m,n in enumerate('ABC'):
 hist=np.zeros(256,np.int64)
 for p in (src/'flat').glob(f'mode-{m}-frame-*.rgba'):
  a=np.memmap(p,dtype=np.uint8,mode='r',shape=(865,1537,4));hist+=np.bincount(a[roi][...,0].ravel(),minlength=256)
 total=int(hist.sum());result['flat'][n]={'pixels':total,'mean_opacity':float(np.dot(hist,np.arange(256))/total/255),'below_90_pct':float(hist[:230].sum()/total*100),'below_50_pct':float(hist[:128].sum()/total*100),'zero_code_pct':float(hist[0]/total*100),'min_code':int(np.where(hist>0)[0][0])}
meta=json.loads((src/'crop.json').read_text());x,y,w,h=meta['xywh'];weights=np.array([.2126,.7152,.0722],np.float32)
def raw(name):return np.memmap(src/f'{name}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4))
diag=np.memmap('/tmp/far-coverage/diag-A.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4))
for prefix in ['', 'far-']:
 a=raw(prefix+'A');candidates={n:raw(prefix+n) for n in 'BC'}
 for n,b in candidates.items():
  changed=count=0;worst=0;sum_abs=sum_signed=0.;selected_count=0;selected_delta=0.;crop_delta=0.;crop_count=0
  for i in range(144):
   d=b[i][roi][...,:3].astype(np.int16)-a[i][roi][...,:3];ad=np.abs(d);count+=d.size;changed+=int(np.count_nonzero(ad));worst=max(worst,int(ad.max()));sum_abs+=float(ad.sum());sum_signed+=float(d.sum())
   q=diag[i][roi];mask=(q[...,0]<230)&(q[...,1]>76)
   if prefix=='':mask&=(q[...,2]<230)
   bright=np.einsum('...c,c->...',d,weights);selected_count+=int(mask.sum());selected_delta+=float(bright[mask].sum())
   cd=b[i,y:y+h,x:x+w,:3].astype(np.int16)-a[i,y:y+h,x:x+w,:3];crop_delta+=float(np.abs(cd).sum());crop_count+=cd.size
  result['pictures'][prefix+n]={'roi_mean_absolute_channel_delta_u8':sum_abs/count,'roi_mean_signed_channel_delta_u8':sum_signed/count,'max_absolute_channel_delta_u8':worst,'changed_channel_pct':100*changed/count,'selected_low_coverage_pixels':selected_count,'selected_mean_weighted_rgb_increase_u8':selected_delta/max(1,selected_count),'selected_condition':'A far opacity<230, source>76; full image additionally near opacity<230','crop_mean_absolute_channel_delta_u8':crop_delta/crop_count}
(src/'metrics.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
