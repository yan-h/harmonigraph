from pathlib import Path
from PIL import Image,ImageDraw
import numpy as np,json,sys
root=Path('/private/tmp/stars-next');d=root/sys.argv[1];meta=json.loads((d/'manifest.json').read_text());w,h=meta['width'],meta['height'];names=sys.argv[2:]
# Fixed central crop at native pixels, no resizing, with labelled full-picture thumbnails above.
sheet=Image.new('RGB',(640*len(names),650),(20,20,24));draw=ImageDraw.Draw(sheet)
for k,n in enumerate(names):
 a=np.fromfile(d/f'{n}-take-0.rgba',np.uint8).reshape(h,w,4);im=Image.fromarray(a,'RGBA').convert('RGB')
 thumb=im.copy();thumb.thumbnail((640,360));sheet.paste(thumb,(k*640,35));draw.text((k*640+12,12),n,fill='white')
 crop=im.crop((w//2-310,h//2-120,w//2+310,h//2+120));sheet.paste(crop,(k*640+10,395));draw.text((k*640+12,375),'Native-pixel central crop',fill='white')
sheet.save(d/'comparison.png')
