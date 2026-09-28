from pathlib import Path
import sys,subprocess,json,hashlib
import numpy as np
sys.path.insert(0,str(Path(__file__).resolve().parent.parent))
from png import write_png,draw_text
D=Path(__file__).resolve().parent;W,H,FPS,N=3840,2160,24,144
names=['base-a','group3-short75'];arrays={}
for name in names:
 p=D/f'{name}.rgb';assert p.stat().st_size==W*H*3*N
 arrays[name]=np.memmap(p,np.uint8,'r',shape=(N,H,W,3))
 write_png(D/f'{name}-4k-frame72.png',arrays[name][72])
 subprocess.run(['ffmpeg','-v','error','-y','-f','rawvideo','-pixel_format','rgb24','-video_size',f'{W}x{H}','-framerate',str(FPS),'-i',str(p),'-an','-c:v','libx264','-threads','2','-preset','medium','-crf','10','-pix_fmt','yuv420p','-movflags','+faststart',str(D/f'{name}-4k.mp4')],check=True)
 print('encoded',name,flush=True)
# Native-pixel crops: the same central 1920x2100 region of each 4K render.
proc=subprocess.Popen(['ffmpeg','-v','error','-y','-f','rawvideo','-pixel_format','rgb24','-video_size',f'{W}x{H}','-framerate',str(FPS),'-i','-','-an','-c:v','libx264','-threads','2','-preset','medium','-crf','10','-pix_fmt','yuv420p','-movflags','+faststart',str(D/'comparison-4k-native-crops.mp4')],stdin=subprocess.PIPE)
maxd=0;total=0
for i in range(N):
 a,b=(arrays[name][i] for name in names)
 frame=np.zeros((H,W,3),np.uint8);frame[60:,:1920]=a[30:2130,960:2880];frame[60:,1920:]=b[30:2130,960:2880]
 draw_text(frame,20,15,'Original P3',scale=4);draw_text(frame,1940,15,'75% far-three + shorter glow',scale=4)
 if i==72:write_png(D/'comparison-4k-frame72.png',frame)
 proc.stdin.write(frame.tobytes())
 delta=np.abs(a.astype(np.int16)-b.astype(np.int16));maxd=max(maxd,int(delta.max()));total+=int(delta.sum())
proc.stdin.close();assert proc.wait()==0
files={}
for p in sorted(D.glob('*.mp4')):
 meta=json.loads(subprocess.check_output(['ffprobe','-v','error','-select_streams','v:0','-show_entries','stream=width,height,nb_frames,r_frame_rate:format=duration','-of','json',str(p)],text=True));stream=meta['streams'][0]
 assert (stream['width'],stream['height'],int(stream['nb_frames']))==(W,H,N),(p,meta)
 files[p.name]={'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest(),'probe':meta}
(D/'comparison.json').write_text(json.dumps({'frames':N,'fps':FPS,'size':[W,H],'pixels_per_point':3,'native_crop':[960,30,1920,2100],'comparison_header_height':60,'no_crop_rescaling':True,'mean_abs_rgb_255':total/(N*H*W*3),'max_abs_rgb_255':maxd,'video_encoding':'libx264 CRF10 yuv420p','files':files},indent=2)+'\n')
print('comparison complete',flush=True)
