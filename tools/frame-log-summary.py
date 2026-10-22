#!/usr/bin/env python3
"""Summarise a frame log: how often the editor missed the glass, and why.

The log comes from the plugin's `--features frame-log` build
(`vendor/egui-baseview/src/frame_log.rs`), one CSV per host process in
`~/Library/Logs/Harmonigraph/`. With no path, the newest log there is read.

The run is split into phases at every change of frame rate (the `divisor`
column) and at every frame the display link did not ask for, so a minimized
editor's watchdog frames and the silence around them count in no phase. A log
written before the `divisor` column existed is read as all Display rate.

Per phase, counted in the display's refreshes:
- missed refreshes: a glass gap longer than the divisor's own spacing
  (a stall) misses each refresh beyond it;
- jumps: a glass gap of the divisor's spacing whose link timestamps are
  further apart, so the content stepped further than the glass did;
- the p99 gap between presents, in ms;
- depth: `presented - target`, how many refreshes after the link's target
  each frame reached the glass;
- main-thread blocking: `acquire_ms` summed per second, the time the host's
  main thread waited for a drawable.

Each stall is put down to the frame that arrived late, checked in order:
dropped before it (a rendered frame in between never reached the glass),
late fire (the link callback ran over half a refresh late), late latch (its
GPU work was done by the link's target), waited for drawable (it submitted
after the target having waited over half a refresh in acquire), late CPU (it
submitted after the target anyway) or late GPU.

usage: python3 -I tools/frame-log-summary.py [LOG] [--window ID]
           [--from SECONDS] [--to SECONDS] [--misses]
"""

import argparse
import csv
import glob
import os
import statistics
import sys
from collections import Counter

# baseview's `LINK_SILENCE`: past this, the watchdog draws instead of the link.
LINK_SILENCE = 0.1
RATES = {1: 'Display', 2: 'Half', 3: 'Third', 4: 'Quarter'}
CAUSES = ('dropped before it', 'late fire', 'waited for drawable', 'late CPU', 'late GPU', 'late latch')


def rate(divisor):
    return RATES.get(divisor, f'1/{divisor}')


def newest_log():
    logs = glob.glob(os.path.expanduser('~/Library/Logs/Harmonigraph/frames-*.csv'))
    if not logs:
        sys.exit('no frames-*.csv in ~/Library/Logs/Harmonigraph; pass a path')
    return max(logs, key=os.path.getmtime)


def read(path):
    """Tick rows by window, and each frame's GPU end and presented time."""
    ticks, gpu_end, presented = {}, {}, {}
    with open(path, newline='') as file:
        reader = csv.DictReader(file)
        has_divisor = 'divisor' in (reader.fieldnames or ())
        for row in reader:
            frame = int(row['frame'])
            if row['kind'] == 'tick':
                ticks.setdefault(row['window'], []).append(row)
            elif row['kind'] == 'gpu':
                gpu_end[frame] = float(row['gpu_end'])
            elif row['kind'] == 'present':
                presented[frame] = float(row['presented'])
    return ticks, gpu_end, presented, has_divisor


def frame(row, gpu_end, presented):
    f = int(row['frame'])
    return {
        'frame': f,
        't': float(row['t']),
        'fired': float(row['fired']),
        'timestamp': float(row['timestamp']),
        'target': float(row['target']),
        'refresh': float(row['refresh']),
        'acquire_ms': float(row['acquire_ms']),
        'submitted': float(row['submitted']),
        'divisor': int(row.get('divisor') or 1),
        'gpu_end': gpu_end.get(f, 0.0),
        'presented': presented.get(f),
    }


def phases(frames):
    """Runs of link-paced frames at one divisor, with no silence in between."""
    out, current = [], []
    for fr in frames:
        if fr['timestamp'] == 0.0:
            # The watchdog's frame, or the one painted as the window is revealed.
            if current:
                out.append(current)
            current = []
            continue
        if current and (
            fr['divisor'] != current[-1]['divisor']
            or fr['timestamp'] - current[-1]['timestamp'] > LINK_SILENCE
        ):
            out.append(current)
            current = []
        current.append(fr)
    if current:
        out.append(current)
    return out


def cause(fr, refresh):
    if fr['fired'] - fr['timestamp'] > refresh / 2:
        return 'late fire'
    if fr['gpu_end'] and fr['gpu_end'] <= fr['target']:
        return 'late latch'
    if fr['submitted'] > fr['target']:
        if fr['acquire_ms'] / 1000 > refresh / 2:
            return 'waited for drawable'
        return 'late CPU'
    return 'late GPU'


def measure(phase):
    """One phase's counts, in a form that sums across phases."""
    divisor = phase[0]['divisor']
    refresh = statistics.median(fr['refresh'] for fr in phase)
    m = {
        'divisor': divisor,
        'refresh': refresh,
        'start': phase[0]['t'],
        'end': phase[-1]['t'] + divisor * refresh,
        'acquire_ms': sum(fr['acquire_ms'] for fr in phase),
        'unshown': 0,
        'refreshes': 0,
        'missed': 0,
        'stalls': [],
        'jumps': 0,
        'gaps_ms': [],
        'depth': Counter(),
    }
    previous, dropped = None, False
    for fr in phase:
        if not fr['submitted']:
            # Nothing to draw: the next glass gap is idle time, not a miss.
            previous, dropped = None, False
            continue
        if not fr['presented']:
            m['unshown'] += 1
            dropped = True
            continue
        m['depth'][round((fr['presented'] - fr['target']) / refresh)] += 1
        if previous is not None:
            glass = round((fr['presented'] - previous['presented']) / refresh)
            link = round((fr['timestamp'] - previous['timestamp']) / refresh)
            m['refreshes'] += glass
            m['gaps_ms'].append((fr['presented'] - previous['presented']) * 1000)
            if glass > divisor:
                m['missed'] += glass - divisor
                why = 'dropped before it' if dropped else cause(fr, refresh)
                m['stalls'].append((fr, glass, why))
            elif link > divisor:
                m['jumps'] += 1
        previous = fr
        dropped = False
    return m


def combine(ms):
    total = dict(ms[0])
    for key in ('acquire_ms', 'unshown', 'refreshes', 'missed', 'jumps'):
        total[key] = sum(m[key] for m in ms)
    total['seconds'] = sum(m['end'] - m['start'] for m in ms)
    total['stalls'] = [s for m in ms for s in m['stalls']]
    total['gaps_ms'] = [g for m in ms for g in m['gaps_ms']]
    total['depth'] = sum((m['depth'] for m in ms), Counter())
    return total


def pct(n, of):
    return f'{100 * n / of:.2f}%' if of else '-'


def report(m, title, t0):
    seconds = m.get('seconds', m['end'] - m['start'])
    span = f"{m['start'] - t0:.1f}-{m['end'] - t0:.1f} s" if 'seconds' not in m else ''
    print(f"{title} {span}  {seconds:.1f} s, {m['refreshes']} refreshes at "
          f"{1 / m['refresh']:.0f} Hz")
    gaps = sorted(m['gaps_ms'])
    p99 = gaps[int(0.99 * (len(gaps) - 1))]
    print(f"  missed refreshes {m['missed']} ({pct(m['missed'], m['refreshes'])}) "
          f"in {len(m['stalls'])} stalls; jumps {m['jumps']} "
          f"({pct(m['jumps'], m['refreshes'])})")
    print(f'  present gap p99 {p99:.2f} ms, max {gaps[-1]:.2f} ms')
    depth = '  '.join(f'{d}: {n}' for d, n in sorted(m['depth'].items()))
    print(f'  depth (presented - target, refreshes)  {depth}')
    blocked = m['acquire_ms'] / seconds if seconds else 0.0
    print(f'  main thread blocked in acquire {blocked:.0f} ms/s')
    causes = Counter(c for _, _, c in m['stalls'])
    if causes:
        print('  stalls: ' + ', '.join(f'{c} {causes[c]}' for c in CAUSES if causes[c]))
    if m['unshown']:
        print(f"  rendered but never shown: {m['unshown']}")


def list_misses(stalls, t0):
    print('\n    s    frame gap  fire  submit  acquire  gpu_end-target  presented-target  cause')
    for fr, glass, why in stalls:
        def ms(x):
            return (x - fr['timestamp']) * 1000
        gpu = (fr['gpu_end'] - fr['target']) * 1000 if fr['gpu_end'] else float('nan')
        print(f"{fr['t'] - t0:6.2f} {fr['frame']:7d} {glass:3d} {ms(fr['fired']):5.2f} "
              f"{ms(fr['submitted']):7.2f} {fr['acquire_ms']:8.2f} {gpu:15.2f} "
              f"{(fr['presented'] - fr['target']) * 1000:17.2f}  {why}")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('log', nargs='?', help='frames-*.csv (default: the newest)')
    parser.add_argument('--window', help='window id (default: the one with most frames)')
    parser.add_argument('--from', dest='start', type=float, default=float('-inf'),
                        help="seconds after the window's first frame")
    parser.add_argument('--to', dest='end', type=float, default=float('inf'))
    parser.add_argument('--misses', action='store_true', help='list every stall')
    args = parser.parse_args()

    path = args.log or newest_log()
    ticks, gpu_end, presented, has_divisor = read(path)
    if not ticks:
        sys.exit(f'{path}: no tick rows')
    print(path)
    if len(ticks) > 1 or args.window:
        counts = ', '.join(f'{w} ({len(rows)})' for w, rows in ticks.items())
        print(f'windows: {counts}')
    window = args.window or max(ticks, key=lambda w: len(ticks[w]))
    if window not in ticks:
        sys.exit(f'no window {window}')
    if not has_divisor:
        print('no divisor column: every link frame is read as Display rate')

    frames = [frame(row, gpu_end, presented) for row in ticks[window]]
    frames.sort(key=lambda fr: fr['frame'])
    t0 = frames[0]['t']
    frames = [fr for fr in frames if args.start <= fr['t'] - t0 <= args.end]
    measured = [measure(p) for p in phases(frames)]
    # A phase with no two consecutive presents (a lone frame between watchdog
    # frames) has no glass gap to measure.
    measured = [m for m in measured if m['refreshes']]
    print(f'window {window}: {len(frames)} frames, {len(measured)} phases\n')

    for i, m in enumerate(measured, 1):
        report(m, f"phase {i}  {rate(m['divisor'])}", t0)
        if args.misses and m['stalls']:
            list_misses(m['stalls'], t0)
        print()
    by_divisor = {}
    for m in measured:
        by_divisor.setdefault(m['divisor'], []).append(m)
    for divisor, ms in sorted(by_divisor.items()):
        if len(ms) > 1:
            report(combine(ms), f'all {rate(divisor)} ({len(ms)} phases)', t0)
            print()


if __name__ == '__main__':
    main()
