"""Create native and 2x-crop GPU comparison sheets; no image resampling except nearest 2x."""
from pathlib import Path
import argparse,json,sys
import numpy as np
parser=argparse.ArgumentParser(__doc__)
parser.add_argument('--repo',type=Path,required=True)
parser.add_argument('--input',type=Path,default=Path('/private/tmp/stars-full-halo-compare'))
parser.add_argument('--output',type=Path,required=True)
a=parser.parse_args()
sys.path.insert(0,str(a.repo/'.claude/skills/look-prototype/kit'))
from png import write_png,draw_text
out=a.output;out.mkdir(exist_ok=True,parents=True)
labels=['A  HALOS 100%','B  HALOS 50%','C  GATHERED BLUR']
metrics={}
for field in ['take','flat']:
    imgs=[np.frombuffer((a.input/f'{name}-{field}.rgba').read_bytes(),np.uint8).reshape(1080,1920,4)[:,:,:3] for name in ['full','half','blur']]
    metrics[field]={}
    for name,img in zip(['full','half','blur'],imgs):
        error=np.abs(img.astype(float)-imgs[0].astype(float))
        metrics[field][name]={'mean_rgb':img.mean(axis=(0,1)).tolist(),'mean_abs_rgb_vs_full':float(error.mean()),'p95_abs_rgb_vs_full':float(np.percentile(error,95))}
    for crop in [False,True]:
        images=[img[340:640,700:1100].repeat(2,axis=0).repeat(2,axis=1) for img in imgs] if crop else imgs
        h,w=images[0].shape[:2]
        sheet=np.full((h+50,w*3,3),[16,21,27],np.uint8)
        for i,(label,img) in enumerate(zip(labels,images)):
            draw_text(sheet,i*w+14,14,label,scale=3)
            sheet[50:,i*w:(i+1)*w]=img
        write_png(out/f'full-halo-{field}{"-2x" if crop else ""}.png',sheet)
(a.input/'image-metrics.json').write_text(json.dumps(metrics,indent=2)+'\n')
print(json.dumps(metrics,indent=2))
