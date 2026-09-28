from pathlib import Path
import subprocess,numpy as np
from png import draw_text
A=Path(__file__).parent;D=A/'movies';O=A/'comparisons'
base=np.load(D/'base-a.npy',mmap_mode='r')
for name,label in [('group3-short75','Far 3 - shorter glow - 75%'),('group3-50','Far 3 - wide glow - 50%'),('direct4','All 5 - shorter glow - native'),('group5-50','All 5 - wide glow - 50%')]:
 other=np.load(D/f'{name}.npy',mmap_mode='r')
 out=O/f'current-vs-{name}.mp4';h,w=base.shape[1:3]
 process=subprocess.Popen(['ffmpeg','-v','error','-y','-f','rawvideo','-pixel_format','rgb24','-video_size',f'{2*w}x{h+28}','-framerate','24','-i','-','-an','-c:v','libx264','-threads','1','-crf','15','-pix_fmt','yuv420p','-movflags','+faststart',str(out)],stdin=subprocess.PIPE)
 for left,right in zip(base,other):
  canvas=np.zeros((h+28,2*w,3),np.uint8);canvas[28:,:w]=left;canvas[28:,w:]=right
  draw_text(canvas,8,6,'Current P3',scale=2);draw_text(canvas,w+8,6,label,scale=2)
  process.stdin.write(canvas.tobytes())
 process.stdin.close();assert process.wait()==0
 print(out,flush=True)
