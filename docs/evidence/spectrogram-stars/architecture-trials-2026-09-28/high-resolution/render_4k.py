from pathlib import Path
import os,subprocess,json,hashlib,time,sys
import numpy as np
A=Path(__file__).resolve().parent.parent;D=Path(__file__).resolve().parent
R=Path(os.environ['RESEARCH_WORKTREE'])
exe=R/'target/release/harmonigraph-offline'
take=Path(os.environ['TAKE_FILE'])
W,H,FPS=3840,2160,24; frame_bytes=W*H*4
for name in ['base-a','group3-short75']:
 out=D/f'{name}.rgb'
 if out.exists():raise RuntimeError('refusing overwrite '+str(out))
 fifo=D/f'{name}.rgba';os.mkfifo(fifo)
 args=[str(exe),str(take),'--appearance',str(A/'appearance.ron'),'--start','46.766945','--end','92.766945','--fps',str(FPS),'--size',f'{W}x{H}','--out',str(fifo)]
 print('render',name,flush=True);start=time.monotonic();count=0;kept=0
 try:
  with (D/f'{name}.render.log').open('w') as log:
   proc=subprocess.Popen(args,cwd=R,env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HG_ARCH_DIR=str(A/'shaders'),HG_ARCH=name),stdout=log,stderr=subprocess.STDOUT)
   with fifo.open('rb') as source,out.open('wb') as dest:
    while True:
     data=source.read(frame_bytes)
     if not data:break
     assert len(data)==frame_bytes,(name,count,len(data))
     if count>=40*FPS:
      rgba=np.frombuffer(data,np.uint8).reshape(H,W,4)
      dest.write(rgba[:,:,:3].tobytes());kept+=1
     count+=1
     if count%120==0:print(name,'frames',count,flush=True)
   assert proc.wait()==0,(name,proc.returncode)
  assert count==1104 and kept==144,(count,kept)
  meta={'variant':name,'width':W,'height':H,'fps':FPS,'rendered_frames':count,'retained_frames':kept,'start':46.766945,'end':92.766945,'trim_lead_seconds':40,'elapsed_seconds':time.monotonic()-start,'binary_sha256':hashlib.sha256(exe.read_bytes()).hexdigest(),'shader_sha256':hashlib.sha256((A/'shaders'/f'{name}.wgsl').read_bytes()).hexdigest(),'appearance_sha256':hashlib.sha256((A/'appearance.ron').read_bytes()).hexdigest()}
  (D/f'{name}.json').write_text(json.dumps(meta,indent=2)+'\n')
  print('complete',name,meta['elapsed_seconds'],flush=True)
 finally:
  fifo.unlink(missing_ok=True)
