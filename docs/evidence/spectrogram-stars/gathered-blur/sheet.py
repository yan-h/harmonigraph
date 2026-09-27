from pathlib import Path
import sys
sys.path.insert(0,str(Path(__file__).resolve().parents[4]/'.claude/skills/look-prototype/kit'))
from png import write_png,draw_text
import numpy as np
r=Path('/private/tmp/stars-gather-blur')
o=r/'sheets';o.mkdir(exist_ok=True,parents=True)
for field in ['take','flat']:
 imgs=[]
 for name in ['halos','blur']:
  a=np.frombuffer((r/f'{name}-{field}.rgba').read_bytes(),np.uint8).reshape(540,960,4)[:,:,:3]
  imgs.append(a)
  print(name,field,'mean',np.mean(a,axis=(0,1)))
 for crop in [False,True]:
  w,h=(640,480) if crop else (960,540)
  sh=np.full((h+55,w*2,3),[16,21,27],np.uint8)
  for i,(label,img) in enumerate(zip(['A  Halos 33%, full jitter','B  Gathered blur'],imgs)):
   if crop:img=img[170:410,350:670].repeat(2,axis=0).repeat(2,axis=1)
   draw_text(sh,i*w+14,14,label,scale=3);sh[55:,i*w:(i+1)*w]=img
  write_png(o/f'stars-gather-{field}{"-2x" if crop else ""}.png',sh)
