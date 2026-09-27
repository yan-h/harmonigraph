import os, subprocess, json
from pathlib import Path
root=Path('/Users/yan/.codex/worktrees/stars-investigation/harmonigraph')
scratch=Path('/private/tmp/stars-investigation')
binary=scratch/'frozen-research/research-render'
def images(label,cases,ppp=2,frames=1):
    out=scratch/label;out.mkdir(exist_ok=True)
    env={k:v for k,v in os.environ.items() if not k.startswith(('PROBE_','RESEARCH_'))}
    env.update(HARMONIGRAPH_REQUIRE_GPU='1',HARMONIGRAPH_SHADER_ASSETS='source',RESEARCH_CASES=','.join(cases),RESEARCH_OUTPUT=str(out),PROBE_JITTER='.5',PROBE_IMAGE_PPP=str(ppp),PROBE_VIDEO_FRAMES=str(frames))
    (out/'manifest.json').write_text(json.dumps({'cases':cases,'ppp':ppp,'frames':frames,'jitter':.5},indent=2))
    print(label,flush=True)
    with (scratch/(label+'.log')).open('w') as log:
        subprocess.run([str(binary),'stars_research_images','--ignored','--nocapture','--test-threads=1'],cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT,check=True)
if __name__=='__main__':
    import sys
    if sys.argv[1]=='stills':
        vectors=['100_100_100_100_100','50_50_50_50_50','50_50_80_100_50','50_50_100_100_60','50_50_100_100_100','25_25_50_50_50']
        images('images-grouped',['full-a','half-a']+['full-grouped-v'+v for v in vectors])
        images('images-grouped-odd',['full-a','full-grouped-v100_100_100_100_100','full-grouped-v50_50_80_100_50','full-separate-v50_50_80_100_50'],1.001)
    elif sys.argv[1]=='motion':
        images('motion-p2-p3',['full-a','half-a','full-grouped-v50_50_80_100_50','full-grouped-v50_50_100_100_60'],frames=180)
    elif sys.argv[1]=='final4k':
        images('images-final-4k',['full-a','half-a','r750','r875','full-grouped-v50_50_80_100_50','full-grouped-v50_50_100_100_60','full-grouped-v50_50_100_100_100','full-complete'],ppp=4)
