from pathlib import Path
import subprocess
root=Path('/private/tmp/stars-investigation/motion-p2-p3')
items=[('full-a','Full reference'),('half-a','Current half'),('full-grouped-v50_50_80_100_50','P2 - 50 50 80 100 50'),('full-grouped-v50_50_100_100_60','P3 - 50 50 100 100 60')]
cmd=['ffmpeg','-v','error','-y']
for name,_ in items:cmd.extend(['-i',str(root/(name+'.mp4'))])
filters=[]
for i,(_,label) in enumerate(items):
    filters.append(f'[{i}:v]crop=800:450:400:300,pad=800:500:0:50:color=0x10131a,drawtext=text={label}:fontcolor=white:fontsize=24:x=16:y=12[v{i}]')
filters.append('[v0][v1][v2][v3]xstack=inputs=4:layout=0_0|800_0|0_500|800_500[out]')
out=root/'comparison.mp4'
cmd.extend(['-filter_complex',';'.join(filters),'-map','[out]','-an','-c:v','libx264','-crf','16','-pix_fmt','yuv420p',str(out)])
subprocess.run(cmd,check=True)
subprocess.run(['ffmpeg','-v','error','-y','-i',str(out),'-frames:v','1',str(root/'poster.png')],check=True)
print(out)
