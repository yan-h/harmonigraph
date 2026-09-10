import subprocess,pathlib,time,csv
root=pathlib.Path('/tmp/realfft-adoption')
(root/'runtime-discard.rgba').symlink_to('/dev/null') if not (root/'runtime-discard.rgba').exists() else None
with open(root/'comparisons/runtime.csv','w') as f:
 w=csv.writer(f,lineterminator='\n');w.writerow(['round','case','variant','wall_seconds','maximum_rss_bytes'])
 for i in range(7):
  for case in ['default-fold','largest-fold']:
   for variant in (['baseline','candidate'] if i%2==0 else ['candidate','baseline']):
    log=root/'comparisons'/f'runtime-{i}-{case}-{variant}.log'
    with open(log,'w') as out:
     args=['/usr/bin/time','-l',str(root/variant/'harmonigraph-offline'),str(root/'fixtures'/f'{case}.take'),'--audio',str(root/'fixtures/audio.wav'),'--align','0','--out',str(root/'runtime-discard.rgba'),'--size','640x400','--fps','60','--start','0','--end','3.2']
     start=time.perf_counter();subprocess.run(args,stdout=out,stderr=out,check=True);wall=time.perf_counter()-start
    rss=next(int(line.split()[0]) for line in log.read_text().splitlines() if 'maximum resident set size' in line)
    w.writerow([i,case,variant,wall,rss]);f.flush()
