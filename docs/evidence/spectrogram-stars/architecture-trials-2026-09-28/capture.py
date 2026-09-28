from pathlib import Path
import os,subprocess,sys,json,hashlib
import numpy as np
from png import write_png,label_cell,sheet
A=Path(__file__).parent;R=Path(os.environ['RESEARCH_WORKTREE']);D=A/'movies';D.mkdir(exist_ok=True)
appearance=A/'appearance.ron'
assert appearance.exists()
env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HG_ARCH_DIR=str(A/'shaders'))
exe=R/'target/release/harmonigraph-offline'; W,H,FPS=768,432,24
for name in sys.argv[1:]:
 if (D/f'{name}.mp4').exists():raise RuntimeError('refusing overwrite '+name)
 raw=Path('/private/tmp')/f'hg-arch-{name}.rgba'
 print('render',name,flush=True)
 with (D/f'{name}.log').open('w') as f:
  subprocess.run([str(exe),os.environ['TAKE_FILE'],'--appearance',str(appearance),'--start','46.766945','--end','92.766945','--fps',str(FPS),'--size',f'{W}x{H}','--out',str(raw)],cwd=R,env=dict(env,HG_ARCH=name),stdout=f,stderr=subprocess.STDOUT,check=True)
 count=raw.stat().st_size//(W*H*4);assert count>=1103,count
 data=np.memmap(raw,dtype=np.uint8,mode='r',shape=(count,H,W,4))
 # Exclude common analyzer/lattice borders exactly as in the accepted actual-renderer reference.
 keep=np.array(data[40*FPS:,:,:, :3]);np.save(D/f'{name}.npy',keep)
 for i in [0,len(keep)//2,len(keep)-1]:write_png(D/f'{name}-{i}.png',keep[i])
 subprocess.run(['ffmpeg','-v','error','-y','-f','rawvideo','-pixel_format','rgba','-video_size',f'{W}x{H}','-framerate',str(FPS),'-i',str(raw),'-ss','40','-an','-c:v','libx264','-crf','15','-pix_fmt','yuv420p','-movflags','+faststart',str(D/f'{name}.mp4')],check=True)
 (D/f'{name}.json').write_text(json.dumps({'variant':name,'width':W,'height':H,'fps':FPS,'rendered_frames':count,'trim_lead_seconds':40,'retained_frames':len(keep),'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'shader_sha256':hashlib.sha256((A/'shaders'/f'{name}.wgsl').read_bytes()).hexdigest(),'appearance_sha256':hashlib.sha256(appearance.read_bytes()).hexdigest()},indent=2))
 del data;raw.unlink();print('captured',name,flush=True)
