from pathlib import Path
import os,subprocess,threading,sys,json,hashlib
out=Path('/tmp/crack-fill');out.mkdir(exist_ok=True)
name=sys.argv[1];binary=sys.argv[2];mode=sys.argv[3] if len(sys.argv)>3 else '0'
w,h=1536,864;frame_bytes=w*h*4
fifo=out/(name+'-pipe.rgba')
if fifo.exists():fifo.unlink()
os.mkfifo(fifo)
counts=[]
def read():
 n=0
 with fifo.open('rb',buffering=0) as src,(out/(name+'.rgba')).open('wb') as dst:
  while True:
   b=bytearray()
   while len(b)<frame_bytes:
    part=src.read(frame_bytes-len(b))
    if not part:break
    b.extend(part)
   if not b:break
   assert len(b)==frame_bytes,len(b)
   if n>=960:dst.write(b)
   n+=1
 counts.append(n)
reader=threading.Thread(target=read,daemon=True);reader.start()
cmd=[binary,'/Users/yan/Music/Harmonigraph Takes/take-2026-09-11_03-16-17.take','--appearance','/Users/yan/.codex/visualizations/2026/09/27/01a0e495-2447-7061-8516-168df98e4784/stars-1242/actual-stars-appearance.ron','--start','46.766945','--end','92.766945','--fps','24','--size',f'{w}x{h}','--scale','2','--out',str(fifo)]
with (out/(name+'.log')).open('w') as log:
 proc=subprocess.run(cmd,env=dict(os.environ,HARMONIGRAPH_SHADER_ASSETS='source',HARMONIGRAPH_REQUIRE_GPU='1',HG_FILL_MODE=mode),stdout=log,stderr=subprocess.STDOUT)
if proc.returncode:raise RuntimeError(f'render failed {name}; see log')
reader.join();fifo.unlink();assert counts==[1104],counts
meta=dict(command=cmd,mode=mode,debug=int(os.environ.get("HG_FILL_DEBUG","0")),flat=os.environ.get("HG_FILL_FLAT")=="1",width=w,height=h,fps=24,warmup_frames=960,frames=144,binary_sha256=hashlib.sha256(Path(binary).read_bytes()).hexdigest())
(out/(name+'.json')).write_text(json.dumps(meta,indent=2)+'\n')
print(name,'complete',flush=True)
