import os
from pathlib import Path
import numpy as np,sys,json
sys.path.insert(0,str(Path(__file__).resolve().parent));import png
src=Path(os.environ.get('FAR_OUTPUT','/tmp/far-coverage'));out=Path(os.environ.get('FAR_MEDIA','/Users/yan/.codex/visualizations/2026/09/28/01a0e620-6ae2-7832-944f-8c4534649f7f/far-coverage'))
out.mkdir(parents=True,exist_ok=True)
labels={'A':'A  THREE FAR LAYERS AT 75%','B':'B  THREE FAR LAYERS AT 50%','C':'C  FIVE FAR LAYERS AT 50%'}
whole=[];crops=[]
for name in labels:
 raw=np.memmap(src/f'{name}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4));frame=raw[72,:,:,:3].copy()
 png.write_png(str(out/f'{name}-full.png'),frame)
 small=np.round(frame.reshape(432,2,768,2,3).mean(axis=(1,3))).astype(np.uint8)
 cell=np.zeros((small.shape[0]+50,small.shape[1],3),np.uint8)+22;cell[50:]=small;png.draw_text(cell,8,10,labels[name],2,(245,245,245));whole.append(cell)
 crop=frame[448:768,192:704].repeat(2,axis=0).repeat(2,axis=1)
 cell=np.zeros((crop.shape[0]+50,crop.shape[1],3),np.uint8)+22;cell[50:]=crop;png.draw_text(cell,8,10,labels[name],2,(245,245,245));crops.append(cell)
 png.write_png(str(out/f'{name}-crop-2x.png'),cell)
png.write_png(str(out/'overview.png'),png.sheet(whole,cols=3,title='FAR STAR COVERAGE - SAME TAKE AND FOREGROUND',tscale=2))
png.write_png(str(out/'crops-2x.png'),png.sheet(crops,cols=3,title='2X PIXEL CROPS - SAME FRAME AND LOCATION',tscale=2))
print(out)
