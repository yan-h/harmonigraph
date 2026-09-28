from pathlib import Path
import numpy as np,json,sys,subprocess
sys.path.insert(0,str(Path(__file__).parent));import png
src=Path('/tmp/crack-fill');out=Path('/Users/yan/.codex/visualizations/2026/09/28/01a0e620-6ae2-7832-944f-8c4534649f7f/crack-fill')
raw=[np.memmap(src/f'far-{n}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4)) for n in 'ABC']
diag=np.memmap('/tmp/far-coverage/diag-A.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4))[72]
lum=np.einsum('...c,c->...',raw[0][72,:,:,:3].astype(np.float32),np.array([.2126,.7152,.0722],np.float32));local=sum(np.roll(np.roll(lum,dy,0),dx,1) for dy in range(-3,4) for dx in range(-3,4))/49
score=np.maximum(local-lum,0)*(1-diag[...,0].astype(np.float32)/255)*(diag[...,1]>76)
best=(-1,None)
for y in range(64,760,8):
 for x in range(16,1320,8):
  value=float(score[y:y+64,x:x+96].sum())
  if value>best[0]:best=(value,[x,y,96,64])
x,y,w,h=best[1];meta={'frame':72,'xywh':best[1],'score':best[0],'selection':'maximum summed positive deficit against 7x7 mean far-only weighted RGB, times far transmittance, where source>76/255; all from A; candidate pixels not used'};(src/'detail-crop.json').write_text(json.dumps(meta,indent=2)+'\n')
labels=['A CURRENT','B GENTLE','C STRONG'];scale=6;cells=[]
for k,r in enumerate(raw):
 cell=np.full((h*scale+48,w*scale,3),22,np.uint8);cell[48:]=r[72,y:y+h,x:x+w,:3].repeat(scale,0).repeat(scale,1);png.draw_text(cell,8,10,labels[k],2,(245,245,245));cells.append(cell)
png.write_png(str(out/'far-notches-6x.png'),png.sheet(cells,cols=3,title='FAR-ONLY DARK NOTCH DETAIL - 6X PIXELS',tscale=2))
size=(w*scale*3,h*scale+48);p=subprocess.Popen(['/opt/homebrew/bin/ffmpeg','-y','-v','error','-f','rawvideo','-pix_fmt','rgb24','-s',f'{size[0]}x{size[1]}','-r','24','-i','-','-an','-c:v','libx264','-crf','12','-preset','slow','-pix_fmt','yuv420p','-movflags','+faststart',str(out/'far-notches-motion-6x.mp4')],stdin=subprocess.PIPE)
base=np.full((size[1],size[0],3),22,np.uint8)
for k,l in enumerate(labels):png.draw_text(base,k*w*scale+8,10,l,2,(245,245,245))
for i in range(144):
 frame=base.copy()
 for k,r in enumerate(raw):frame[48:,k*w*scale:(k+1)*w*scale]=r[i,y:y+h,x:x+w,:3].repeat(scale,0).repeat(scale,1)
 p.stdin.write(frame.tobytes())
p.stdin.close();assert p.wait()==0;print(meta)
