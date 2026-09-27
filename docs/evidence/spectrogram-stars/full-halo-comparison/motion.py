"""Assemble labeled native-pixel crops from the scratch GPU motion renders."""
from pathlib import Path
import argparse,subprocess,sys
import numpy as np
p=argparse.ArgumentParser(__doc__)
p.add_argument('--repo',type=Path,required=True)
p.add_argument('--input',type=Path,default=Path('/private/tmp/stars-full-halo-compare'))
p.add_argument('--output',type=Path,required=True)
a=p.parse_args()
sys.path.insert(0,str(a.repo/'.claude/skills/look-prototype/kit'))
from png import write_png,draw_text
header=np.full((40,1920,3),[16,21,27],np.uint8)
for i,label in enumerate(['A  HALOS 100%','B  HALOS 50%','C  GATHERED BLUR']):
    draw_text(header,640*i+12,10,label,scale=3)
write_png(a.input/'motion-header.png',header)
filters=';'.join(f'[{i}:v]crop=640:480:640:300,pad=640:520:0:40:color=0x10151b[{label}]' for i,label in enumerate('abc'))
filters+=';[a][b][c]hstack=inputs=3[stack];[stack][3:v]overlay=0:0[v]'
cmd=['ffmpeg','-v','error','-y']
for name in ['full.mp4','half.mp4','blur.mp4','motion-header.png']:cmd+=['-i',str(a.input/name)]
cmd+=['-filter_complex',filters,'-map','[v]','-c:v','libx264','-crf','12','-pix_fmt','yuv420p',str(a.output)]
subprocess.run(cmd,check=True)
