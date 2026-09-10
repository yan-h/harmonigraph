"""Compare every RGBA byte; export the most affected frame without blessing."""
import csv, pathlib, sys
import numpy as np
from PIL import Image, ImageDraw
root=pathlib.Path('/tmp/realfft-adoption')
out=root/'comparisons';out.mkdir(exist_ok=True)
w=csv.writer(sys.stdout,lineterminator="\n");w.writerow(['case','frames','changed_frames','changed_pixels','changed_channels','max_delta','mean_abs_channel_delta','max_frame'])
for name in ['default-fold','default-spectrum','largest-fold','largest-spectrum','boundary-8192-3-fold','boundary-16384-5-fold','boundary-16384-1-spectrum']:
 a=np.memmap(root/'baseline'/f'{name}.rgba',dtype=np.uint8,mode='r').reshape(-1,400,640,4)
 b=np.memmap(root/'candidate'/f'{name}.rgba',dtype=np.uint8,mode='r').reshape(a.shape)
 changed=[]; pixels=channels=total=maximum=0;largest=(0,0)
 for i in range(len(a)):
  d=np.abs(a[i].astype(np.int16)-b[i].astype(np.int16)); count=int(np.count_nonzero(d))
  if count:changed.append(i)
  pixels+=int(np.count_nonzero(np.any(d,axis=2)));channels+=count;total+=int(d.sum());maximum=max(maximum,int(d.max()))
  if d.sum()>largest[1]:largest=(i,int(d.sum()))
 i=largest[0];w.writerow([name,len(a),len(changed),pixels,channels,maximum,total/a.size,i])
 if changed:
  sheet=Image.new('RGB',(1920,428),'#202020'); draw=ImageDraw.Draw(sheet)
  diff=np.minimum(np.abs(a[i].astype(np.int16)-b[i].astype(np.int16))*32,255).astype(np.uint8)
  for j,(label,frame) in enumerate([('Baseline',a[i]),('RealFFT',b[i]),('Absolute difference x32',diff)]):
   sheet.paste(Image.fromarray(frame[:,:,:3]),(640*j,28));draw.text((640*j+10,8),f'{label} — {name}, frame {i}',fill='white')
  sheet.save(out/f'{name}.png')
  Image.fromarray(b[i]).save(out/f'{name}-candidate.png')
