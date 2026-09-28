import os
from pathlib import Path
import numpy as np,sys
sys.path.insert(0,str(Path(__file__).resolve().parent));import png
src=Path(os.environ.get('FAR_OUTPUT','/tmp/far-coverage'));out=Path(os.environ.get('FAR_MEDIA','/Users/yan/.codex/visualizations/2026/09/28/01a0e620-6ae2-7832-944f-8c4534649f7f/far-coverage'))
cells=[]
for n,label in [('A','CURRENT FULL IMAGE'),('far-A','SAME FRAME - FAR ONLY')]:
 raw=np.memmap(src/f'{n}.rgba',dtype=np.uint8,mode='r',shape=(144,864,1536,4));crop=raw[72,448:768,192:704,:3].repeat(2,axis=0).repeat(2,axis=1)
 cell=np.full((690,1024,3),22,np.uint8);cell[50:]=crop;png.draw_text(cell,8,10,label,2,(245,245,245));cells.append(cell)
png.write_png(str(out/'foreground-diagnostic.png'),png.sheet(cells,cols=2,title='DIAGNOSTIC ONLY - REMOVING NEAR STARS REVEALS THEIR OVERLAP',tscale=2))
