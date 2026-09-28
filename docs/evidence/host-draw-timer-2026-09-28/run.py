import csv, hashlib, json, os, platform, statistics, subprocess, sys
from pathlib import Path
root = Path.cwd()
out = Path(sys.argv[1] if len(sys.argv) > 1 else '/tmp/harmonigraph-host-timer')
out.mkdir(parents=True, exist_ok=True)
binary = root / 'target/release/examples/editor-startup'
manifest = {'baseline': subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(), 'platform': platform.platform(), 'cpu': subprocess.check_output(['sysctl','-n','machdep.cpu.brand_string'],text=True).strip(), 'binary_sha256': hashlib.sha256(binary.read_bytes()).hexdigest(), 'probe_patch_sha256': hashlib.sha256(Path(__file__).with_name('probe.patch').read_bytes()).hexdigest(), 'warmup':120, 'measured':240, 'order':['on','on','off','off','on','off','off','on'], 'mode_order':[0,1]}
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
all_rows=[]
summaries=[]
def fields(line):
    return dict(word.split('=',1) for word in line.split()[1:])
for wait in manifest['mode_order']:
    for run,timer in enumerate(manifest['order']):
        name=f'wait-{wait}-{run}-{timer}'
        logfile=out/(name+'.log')
        with logfile.open('w') as log:
            result=subprocess.run([str(binary)], stdout=log, stderr=subprocess.STDOUT, timeout=100, env={**os.environ,'HARMONIGRAPH_PROBE_TIMER':timer,'HARMONIGRAPH_PROBE_WAIT':str(wait),'HARMONIGRAPH_SHADER_ASSETS':'strict'})
        assert result.returncode == 0, (name,result.returncode)
        lines=logfile.read_text().splitlines()
        assert any('NATIVE_LIFECYCLE openings=1 devices=1 dropped=1' in line and 'source: 0, load_failed: 0, rejected: 0' in line for line in lines), name
        draws={int(d['frame']):d for line in lines if line.startswith('PROBE_DRAW ') for d in [fields(line)]}
        ui=[fields(line) for line in lines if line.startswith('PROBE_UI ')]
        assert len(ui)==360 and len({u['frame'] for u in ui})==360, name
        rows=[]
        for u in ui[120:]:
            d=draws[int(u['frame'])]
            row={'wait':wait,'run':run,'timer':timer, 'sample':int(u['n']), 'frame':int(u['frame']), 'armed':int(d['armed']=='true')}
            row.update({k:float(u[k]) for k in ['render','acquire','upload','encode','submit','tick','prims','verts']})
            row.update({k:float(d[k]) for k in ['completion','width','height','ppp']})
            row['render_less_acquire']=row['render']-row['acquire']
            rows.append(row)
        assert len(rows)==240
        assert {(r['width'],r['height'],r['ppp'],r['prims'],r['verts']) for r in rows}=={(2000.,1400.,2.,23.,4164.)},name
        assert all(r['armed']==0 for r in rows) if timer=='off' else sum(r['armed'] for r in rows)>50,name
        summary={'name':name,'timer':timer,'wait':wait,'armed':sum(r['armed'] for r in rows)}
        for key in ['render_less_acquire','acquire','upload','encode','submit','tick','completion']:
            summary[key]={'median_ms':statistics.median(r[key] for r in rows),'mean_ms':statistics.mean(r[key] for r in rows)}
        summaries.append(summary)
        all_rows.extend(rows)
        (out/'summary.json').write_text(json.dumps(summaries,indent=2)+'\n')
        with (out/'samples.csv').open('w',newline='') as f:
            writer=csv.DictWriter(f,fieldnames=list(all_rows[0]),lineterminator="\n");writer.writeheader();writer.writerows(all_rows)
        print(name,'armed',summary['armed'],'cpu',round(summary['render_less_acquire']['median_ms'],4),'completion',round(summary['completion']['median_ms'],4),flush=True)
