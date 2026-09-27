"""Check balanced run structure and report mean paired GPU reductions."""
import collections
import csv
import gzip
import itertools
from pathlib import Path
import statistics
import sys

root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).parent
print('| Run | Original ms | Production saving | Forced native saving | A/A difference |')
print('|---|---:|---:|---:|---:|')
for path in sorted(root.glob('*.csv*')):
    opening = gzip.open if path.suffix == '.gz' else open
    with opening(path, 'rt') as source:
        rows = list(csv.DictReader(source))
    frames = collections.defaultdict(dict)
    for row in rows:
        frame, slot = int(row['frame']), int(row['slot'])
        assert slot not in frames[frame]
        frames[frame][slot] = row
    assert len(frames) == 240
    orders = set(itertools.permutations(['base_a', 'base_b', 'split', 'unsplit']))
    for begin in range(0, len(frames), 24):
        actual = {tuple(frames[f][slot]['case'] for slot in range(4)) for f in range(begin, begin + 24)}
        assert actual == orders, (path, begin)
    samples = collections.defaultdict(list)
    for row in rows:
        samples[row['case']].append(float(row['gpu_ms']))
    means = {name: statistics.mean(values) for name, values in samples.items()}
    base = (means['base_a'] + means['base_b']) / 2
    save = lambda case: 100 * (1 - means[case] / base)
    aa = 100 * (means['base_b'] / means['base_a'] - 1)
    print(f"| {path.name.split('.')[0]} | {base:.3f} | {save('split'):+.1f}% | {save('unsplit'):+.1f}% | {aa:+.1f}% |")
