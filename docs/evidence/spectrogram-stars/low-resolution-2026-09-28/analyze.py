from pathlib import Path
import numpy as np,sys,json,os
R=Path(os.environ['RESEARCH_WORKTREE'])
sys.path.insert(0,str(R/'.claude/skills/look-prototype/kit'))
from png import write_png,sheet,label_cell
D=Path(os.environ.get('RES_OUTPUT','/private/tmp/stars-low-resolution'));O=D/'visuals';O.mkdir(exist_ok=True)
metrics=json.loads((D/'visual-metrics.json').read_text()) if (D/'visual-metrics.json').exists() else {};cases=['H','M','L1','L2','V','X']
labels={'H':'H HIGH FAR75 FRONT100','M':'M MEDIUM FAR50 FRONT75','L1':'L1 FAR33 FRONT50','L2':'L2 FAR25 FRONT50','V':'V FAR17 FRONT33','X':'X FAR12.5 FRONT25'}
for size in os.environ.get('RES_SIZES','1920x1080,3840x2160').split(','):
 w,h=map(int,size.split('x'))
 def read(c,f):return np.fromfile(D/f'{size}-{c}'/f,np.uint8).reshape(h,w,4)[:,:,:3].astype(np.int16)
 ref=sorted((D/f'{size}-H').glob('*.rgba'));assert len(ref)==12
 masks={}
 for f in ref:
  lum=read('H',f.name).sum(2);masks[f.name]=lum>=np.percentile(lum,75)
 # Shared crop at t=3 s: brightest baseline block, all metrics also cover full frame.
 frame='0190.rgba';a=read('H',frame);bh,bw=180,320
 _,x,y=max((float(a[y:y+bh,x:x+bw].mean()),x,y) for y in range(0,h-bh+1,bh) for x in range(0,w-bw+1,bw))
 metrics[size+'-crop']={'x':x,'y':y,'width':bw,'height':bh,'frame':190}
 for baseline in ['H','M']:
  for case in cases:
   if case==baseline:continue
   abs_sum=squared=count=ex5=worst=edgea=edgeb=bright_sum=bright_count=0
   hist=np.zeros(256,np.int64);temporal=[];temporal_bright=[];prev=None
   for f in ref:
    a=read(baseline,f.name);b=read(case,f.name);d=np.abs(a-b)
    mask=masks[f.name]
    bright_sum+=d[mask].sum();bright_count+=d[mask].size
    hist+=np.bincount(d.ravel(),minlength=256);abs_sum+=d.sum();squared+=np.square(d.astype(np.float64)).sum();count+=d.size;worst=max(worst,int(d.max()));ex5+=np.count_nonzero(d.max(2)>5)
    edgea+=np.abs(np.diff(a,axis=0)).sum()+np.abs(np.diff(a,axis=1)).sum();edgeb+=np.abs(np.diff(b,axis=0)).sum()+np.abs(np.diff(b,axis=1)).sum()
    if int(f.stem)%60==11:
     pa,pb=prev;delta=np.abs((b-pb)-(a-pa));temporal.append(float(delta.mean()));temporal_bright.append(float(delta[mask].mean()))
    prev=(a,b)
   metrics[size+'-'+case+'-vs-'+baseline]={'mae_rgb_255':float(abs_sum/count),'rmse_rgb_255':float(np.sqrt(squared/count)),'p99_channel_error':int(np.searchsorted(np.cumsum(hist),count*.99)),'max_channel_error':worst,'pixels_any_channel_gt5_percent':100*ex5/(12*w*h),'gradient_absolute_sum_change_percent':100*(float(edgeb)/edgea-1),'bright_quartile_mae_rgb_255':float(bright_sum/bright_count),'adjacent_frame_delta_error_mae':float(np.mean(temporal)),'bright_adjacent_delta_error_mae':float(np.mean(temporal_bright)),'frames':12,'adjacent_pairs':len(temporal)}
   print(size,case,'vs',baseline,metrics[size+'-'+case+'-vs-'+baseline],flush=True)
 cells=[]
 for case in cases:
  a=read(case,frame).astype(np.uint8);write_png(O/f'{size}-{case}.png',a)
  crop=a[y:y+bh,x:x+bw].repeat(2,0).repeat(2,1)
  cells.append(label_cell(crop,[labels[case]]))
 write_png(O/f'{size}-crops-2x.png',sheet(cells,cols=3,title=size+' SAME CROP AT 2X - LOWER RESOLUTION ONLY'))
 (D/'visual-metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')

# A featureless field checks brightness and exposes coarser texture directly.
if (D/'1920x1080-flat-M/0010.rgba').exists():
 def flat_read(c):
  return np.fromfile(D/f'1920x1080-flat-{c}'/'0010.rgba',np.uint8).reshape(1080,1920,4)[:,:,:3].astype(np.int16)
 baseline=flat_read('M');flat={};cells=[]
 for c in ['H','M','L1','X']:
  a=flat_read(c);delta=np.abs(a-baseline)
  flat[c]={'mae_vs_medium_rgb_255':float(delta.mean()),'mean_rgb_255':float(a.mean()),'pixels_any_channel_gt5_percent':float(100*np.mean(delta.max(2)>5))}
  cells.append(label_cell(a[450:630,800:1120].repeat(2,0).repeat(2,1).astype(np.uint8),[c+' FLAT INPUT 2X']))
 write_png(O/'flat-crops-2x.png',sheet(cells,cols=2,title='FLAT INPUT - MATCHED 1080P CROP AT 2X'))
 (D/'flat-metrics.json').write_text(json.dumps(flat,indent=2)+'\n')
