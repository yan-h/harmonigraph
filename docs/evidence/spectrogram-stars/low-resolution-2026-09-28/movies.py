from pathlib import Path
import subprocess,json,os
D=Path(os.environ.get('RES_OUTPUT','/private/tmp/stars-low-resolution'));O=D/'visuals';m=json.loads((D/'visual-metrics.json').read_text());cases=['H','M','L1','L2','V','X']
labels=['H  High','M  Medium','L1  Background 33% / foreground 50%','L2  Background 25% / foreground 50%','V  Background 17% / foreground 33%','X  Background 12.5% / foreground 25%']
for size in os.environ.get('RES_SIZES','1920x1080,3840x2160').split(','):
 c=m[size+'-crop'];filters=[];args=['ffmpeg','-hide_banner','-loglevel','error','-y']
 for case in cases:args+=['-i',str(D/f'{size}-{case}'/'motion.mp4')]
 for i,label in enumerate(labels):
  filters.append(f"[{i}:v]crop=320:180:{c['x']}:{c['y']},scale=640:360:flags=neighbor,pad=640:400:0:40:color=0x17191d,drawtext=expansion=none:fontfile=/System/Library/Fonts/Supplemental/Arial.ttf:text='{label}':fontsize=22:fontcolor=white:x=12:y=10[v{i}]")
 filters.append(''.join(f'[v{i}]' for i in range(6))+'xstack=inputs=6:layout=0_0|640_0|1280_0|0_400|640_400|1280_400[out]')
 args+=['-filter_complex',';'.join(filters),'-map','[out]','-an','-c:v','libx264','-crf','16','-preset','fast','-pix_fmt','yuv420p','-movflags','+faststart',str(O/f'{size}-motion-crops.mp4')]
 subprocess.run(args,check=True)
 print('created',size,flush=True)
