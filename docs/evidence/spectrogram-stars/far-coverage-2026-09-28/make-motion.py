import os
from pathlib import Path
import numpy as np,sys,subprocess
sys.path.insert(0,str(Path(__file__).resolve().parent));import png
src=Path(os.environ.get('FAR_OUTPUT','/tmp/far-coverage'));out=Path(os.environ.get('FAR_MEDIA','/Users/yan/.codex/visualizations/2026/09/28/01a0e620-6ae2-7832-944f-8c4534649f7f/far-coverage'))
raw=[np.memmap(src/f'{n}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4)) for n in 'ABC']
labels=['A  3 FAR AT 75%','B  3 FAR AT 50%','C  5 FAR AT 50%']
# Native pixels: one 640 x 432 source crop per candidate, no resize.
cmd=['/opt/homebrew/bin/ffmpeg','-y','-v','error','-f','rawvideo','-pix_fmt','rgb24','-s','1920x480','-r','24','-i','-','-an','-c:v','libx264','-crf','12','-preset','slow','-pix_fmt','yuv420p','-movflags','+faststart',str(out/'motion-native-crops.mp4')]
p=subprocess.Popen(cmd,stdin=subprocess.PIPE)
base=np.full((480,1920,3),22,np.uint8)
for k,l in enumerate(labels):png.draw_text(base,k*640+12,12,l,2,(245,245,245))
for i in range(144):
 frame=base.copy()
 for k,r in enumerate(raw):frame[48:,k*640:(k+1)*640]=r[i,384:816,128:768,:3]
 p.stdin.write(frame.tobytes())
p.stdin.close();assert p.wait()==0
print(out/'motion-native-crops.mp4')
