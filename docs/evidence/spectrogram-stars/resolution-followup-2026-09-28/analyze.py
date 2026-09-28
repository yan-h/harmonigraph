from pathlib import Path
import numpy as np,sys,json,os
sys.path.insert(0,str(Path(os.environ['RESEARCH_WORKTREE'])/'.claude/skills/look-prototype/kit'))
from png import write_png,sheet,label_cell
D=Path(os.environ['RES_OUTPUT']);O=D/'visuals';O.mkdir(exist_ok=True)
metrics={}
for size in ['1920x1080','3840x2160']:
 w,h=map(int,size.split('x'));ref=sorted((D/f'{size}-A').glob('*.rgba'));assert len(ref)==12
 for case in ['B','C','D','control']:
  diffs=[];squared=0;count=0;hist=np.zeros(256,np.int64);worst=0;ex5=0;edgea=edgeb=0;temporal=0;prev_a=prev_b=None
  for f in ref:
   a=np.fromfile(f,np.uint8).reshape(h,w,4)[:,:,:3].astype(np.int16);b=np.fromfile(D/f'{size}-{case}'/f.name,np.uint8).reshape(h,w,4)[:,:,:3].astype(np.int16)
   d=np.abs(a-b);diffs.append(float(d.mean()));hist+=np.bincount(d.ravel(),minlength=256);squared+=np.square(d.astype(np.float64)).sum();count+=d.size;worst=max(worst,int(d.max()));ex5+=np.count_nonzero(d.max(2)>5)
   edgea+=np.abs(np.diff(a,axis=0)).sum()+np.abs(np.diff(a,axis=1)).sum();edgeb+=np.abs(np.diff(b,axis=0)).sum()+np.abs(np.diff(b,axis=1)).sum()
   if prev_a is not None:temporal+=np.abs((b-prev_b)-(a-prev_a)).mean()
   prev_a,prev_b=a,b
  metrics[size+'-'+case]={'mae_rgb_255':float(np.mean(diffs)),'rmse_rgb_255':float(np.sqrt(squared/count)),'p99_channel_error':int(np.searchsorted(np.cumsum(hist),count*.99)),'max_channel_error':worst,'pixels_any_channel_gt5_percent':100*ex5/(12*w*h),'gradient_absolute_sum_change_percent':100*(edgeb/edgea-1),'frame_delta_error_mae':float(temporal/11),'frames':12}
 frames={c:np.fromfile(D/f'{size}-{c}'/'0130.rgba',np.uint8).reshape(h,w,4)[:,:,:3] for c in ['A','B','C','D']}
 for c,a in frames.items():write_png(O/f'{size}-{c}.png',a)
 # Shared crop centered on brightest spatial block, excludes no changed pixels from metrics.
 a=frames['A'];bh,bw=180,320
 candidates=[(float(a[y:y+bh,x:x+bw].mean()),x,y) for y in range(0,h-bh+1,bh) for x in range(0,w-bw+1,bw)]
 _,x,y=max(candidates); cells=[]
 labels={'A':'A CURRENT FAR75 FRONT100','B':'B FAR50 FRONT100','C':'C FAR50 FRONT75 HALOS UNCHANGED','D':'D FAR50 FRONT75 HALOS75'}
 for c in ['A','B','C','D']:
  crop=frames[c][y:y+bh,x:x+bw].repeat(2,0).repeat(2,1)
  cells.append(label_cell(crop, [labels[c]]))
 # png sheet interface checked below
 try: out=sheet(cells,cols=2,title=size+' - SAME CROP AT 2X')
 except Exception as e: print('sheet',e);continue
 write_png(O/f'{size}-crops-2x.png',out)
 metrics[size+'-crop']={'x':x,'y':y,'width':bw,'height':bh,'frame':130}
(D/'visual-metrics.json').write_text(json.dumps(metrics,indent=2));print(json.dumps(metrics,indent=2))
