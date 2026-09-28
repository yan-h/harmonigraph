from pathlib import Path
import numpy as np,json,subprocess,sys
sys.path.insert(0,str(Path(__file__).parent));import png
src=Path('/tmp/crack-fill');out=Path('/Users/yan/.codex/visualizations/2026/09/28/01a0e620-6ae2-7832-944f-8c4534649f7f/crack-fill');out.mkdir(parents=True,exist_ok=True)
meta=json.loads((src/'crop.json').read_text());x,y,w,h=meta['xywh'];idx=meta['frame'];labels=['A  CURRENT','B  GENTLE COVERAGE','C  STRONG COVERAGE']
for prefix,stem in [('', 'full'),('far-','far-only')]:
 raw=[np.memmap(src/f'{prefix}{n}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4)) for n in 'ABC']
 for scale in [2,4]:
  cells=[]
  for k,r in enumerate(raw):
   crop=r[idx,y:y+h,x:x+w,:3].repeat(scale,axis=0).repeat(scale,axis=1)
   cell=np.full((h*scale+48,w*scale,3),22,np.uint8);cell[48:]=crop;png.draw_text(cell,8,10,labels[k],2,(245,245,245));cells.append(cell)
  png.write_png(str(out/f'{stem}-crops-{scale}x.png'),png.sheet(cells,cols=3,title=f'{stem.upper()} - {scale}X PIXELS - SAME CRACK REGION',tscale=2))
 for k,r in enumerate(raw):png.write_png(str(out/f'{prefix}{"ABC"[k]}-full.png'),r[idx,:,:,:3])
 # The movie uses nearest-neighbor 2x pixels, preserving the source geometry.
 size=(w*2*3,h*2+48);dest=out/f'{stem}-motion-2x.mp4'
 proc=subprocess.Popen(['/opt/homebrew/bin/ffmpeg','-y','-v','error','-f','rawvideo','-pix_fmt','rgb24','-s',f'{size[0]}x{size[1]}','-r','24','-i','-','-an','-c:v','libx264','-crf','12','-preset','slow','-pix_fmt','yuv420p','-movflags','+faststart',str(dest)],stdin=subprocess.PIPE)
 base=np.full((size[1],size[0],3),22,np.uint8)
 for k,l in enumerate(labels):png.draw_text(base,k*w*2+8,10,l,2,(245,245,245))
 for frame in range(144):
  image=base.copy()
  for k,r in enumerate(raw):image[48:,k*w*2:(k+1)*w*2]=r[frame,y:y+h,x:x+w,:3].repeat(2,axis=0).repeat(2,axis=1)
  proc.stdin.write(image.tobytes())
 proc.stdin.close();assert proc.wait()==0
 print(dest,flush=True)
