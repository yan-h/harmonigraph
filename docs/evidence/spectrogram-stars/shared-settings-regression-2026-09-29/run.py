import json, os, pathlib, re, subprocess, time
root = pathlib.Path(__file__).resolve().parent
size = os.environ.get('BENCH_SIZE', '3840x2160')
ppp = '4' if size == '3840x2160' else '2'
order = os.environ.get('BENCH_ORDER', 'before,before,current,current,before,current,current,before').split(',')
tag = os.environ.get('BENCH_TAG', 'baseline')
results = []
for i, label in enumerate(order):
    env = os.environ | {'HARMONIGRAPH_SHADER_ASSETS': os.environ.get('BENCH_ASSETS', 'source'), 'PROBE_CASE': 'medium', 'PROBE_SIZE': size, 'PROBE_PPP': ppp, 'PROBE_FRAMES': '240', 'PROBE_FILLS': '1'}
    start = time.time()
    p = subprocess.run([str(root / label), 'cloud_costs_by_style_and_dial', '--ignored', '--nocapture', '--test-threads=1'], env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    log = root / f'{tag}-{size}-{i+1:02d}-{label}.log'
    log.write_text(p.stdout)
    rows = [line for line in p.stdout.splitlines() if line.startswith('stars, medium /')]
    if p.returncode or not rows or '1 passed' not in p.stdout:
        print(p.stdout, flush=True)
        raise RuntimeError(f'{label}: invalid probe run {p.returncode}')
    row = rows[0]
    timings = re.search(r': ([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+)/([\d.]+); wall ([\d.]+); CPU ([\d.]+)', row)
    if not timings:
        raise RuntimeError(row)
    keys = ['gpu_min', 'gpu_p10', 'gpu_median', 'gpu_p90', 'gpu_max', 'wall_median', 'cpu_median']
    result = {'label': label, 'size': size, 'ppp': ppp, 'run': i + 1, 'start': start, 'log': str(log)} | dict(zip(keys, map(float, timings.groups())))
    results.append(result)
    print(json.dumps(result), flush=True)
    (root / f'results-{tag}-{size}.json').write_text(json.dumps(results, indent=2) + '\n')
