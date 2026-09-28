from pathlib import Path
import numpy as np,json,sys,subprocess
from png import write_png,label_cell,sheet
A=Path(__file__).parent;D=A/'movies';O=A/'comparisons';O.mkdir(exist_ok=True)
labels={'base-a':'A Current P3','base-b':'A2 Current P3 repeat','direct4':'D Direct four neighbours','residual4':'R Four-neighbour halos','group3-75':'G3-75 Far three at 75%','group3-50':'G3-50 Far three at 50%','group5-75':'G5-75 All five at 75%','group5-50':'G5-50 All five at 50%'}
labels['group3-short100']='N Far three short glow native'
labels['group3-short75']='S Far three short glow 75%'
labels['group3-short50']='S50 Far three short glow 50%'
for m in [1,2,3,4,8,12,16,24,28]:labels[f'hybrid-{m}']='H'+str(m)+' Wide glow on depths '+','.join(str(i+1) for i in range(5) if m & (1<<i))
names=sys.argv[2:];outname=sys.argv[1];crops=[];frames=[];metrics={}
base=np.load(D/'base-a.npy',mmap_mode='r')
for name in names:
 arr=np.load(D/f'{name}.npy',mmap_mode='r');frame=arr[72]
 frames.append(label_cell(frame.copy(),[labels.get(name,name)],scale=2))
 crop=frame[130:330,220:554];crop=np.repeat(np.repeat(crop,2,0),2,1);crops.append(label_cell(crop,[labels.get(name,name)],scale=2))
 # Pixel differences are descriptive only, never a visual acceptance score.
 total=0;maxd=0
 for f in range(len(arr)):
  delta=np.abs(arr[f,6:426,2:702].astype(np.int16)-base[f,6:426,2:702].astype(np.int16));total+=int(delta.sum());maxd=max(maxd,int(delta.max()))
 metrics[name]={'mean_abs_rgb_255':total/(len(arr)*420*700*3),'max_abs_rgb_255':maxd}
write_png(O/f'{outname}-native.png',sheet(frames,cols=min(3,len(frames)),title='Native frames: same take and instant'))
write_png(O/f'{outname}-2x.png',sheet(crops,cols=min(3,len(crops)),title='Same crop enlarged 2x without smoothing'))
(O/f'{outname}-metrics.json').write_text(json.dumps(metrics,indent=2));print(json.dumps(metrics),flush=True)
