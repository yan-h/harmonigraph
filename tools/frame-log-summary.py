#!/usr/bin/env python3
"""Summarise a frame log: how often the editor missed the glass, and why.

The log comes from the plugin's `--features frame-log` build
(`vendor/egui-baseview/src/frame_log.rs`), one CSV per host process in
`~/Library/Logs/Harmonigraph/`. With no path, the newest log there is read.

The run is split into phases at every change of the chosen Editor frame rate
(the `chosen` column) and at every frame the display link did not ask for, so a
minimized editor's watchdog frames and the silence around them count in no
phase. A log written before the `chosen` column existed is read as chosen at
its `divisor`, which holds for one recorded before #1692 slowed a still
editor's link; one recorded between the two cannot be told apart, so record
it again. A log written before the `divisor` column existed is read as all
Display rate.

A tick row's `divisor` is the pacing of the interval that led to that frame:
the chosen rate, or a multiple of it while a still editor's link is slowed to
its poll (#1692). So each glass gap is measured against its own frame's
divisor, not one per phase: the frame after a first note was paced at the
chosen rate though the note's own frame was paced at the poll's.

Per phase, counted in the display's refreshes:
- missed refreshes: a glass gap longer than the divisor that paced it
  (a stall) misses each refresh beyond it. An input event restores the chosen
  rate between frames and the next refresh draws, so a frame the log marks
  `woken` is held to the link's own gap instead;
- jumps: a glass gap of the expected spacing whose link timestamps are
  further apart, so the content stepped further than the glass did;
- the p99 gap between presents paced at the chosen rate, in ms;
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

`--events` reports instead on the moments the editor's own `event` rows mark
(#1670), across every window and every time the editor was opened:
- still: each stretch where nothing kept the editor animating, so nothing on
  screen could move, and how often it drew, against the rate it draws at when
  animating. A transport left stopped reaches one once the last sound has
  scrolled out of the Span, since silence does not count as audible audio;
- first note: the frames in the second and a half after the first note of
  each opening, with any stall and the costliest frames;
- exports: missed refreshes in each export's first 30 s against the 30 s
  before it.

usage: python3 -I tools/frame-log-summary.py [LOG] [--window ID]
           [--from SECONDS] [--to SECONDS] [--misses] [--events]
"""

import argparse
import bisect
import csv
import glob
import os
import statistics
import sys
from collections import Counter

# baseview's `LINK_SILENCE`: past this, the watchdog draws instead of the link.
LINK_SILENCE = 0.1
# A still stretch shorter than this says little about idling.
SILENCE_MIN = 60.0
# How long after a first note its late frames are looked for.
NOTE_AFTER = 1.5
# How much of an export, and of the time before it, is compared.
EXPORT_COMPARE = 30.0
# A shown editor draws at least its 50 ms idle poll, so a longer gap between
# drawn frames is a window that could not present: covered, or minimized.
UNSHOWN_GAP = 1.0
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
    """Tick rows by window, each frame's GPU end and presented time, and the
    editor's events as (frame, t, text)."""
    ticks, gpu_end, presented, events = {}, {}, {}, []
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
            elif row['kind'] == 'event':
                events.append((frame, float(row['t']), row['event']))
    return ticks, gpu_end, presented, events, has_divisor


def frame(row, gpu_end, presented):
    f = int(row['frame'])
    return {
        'frame': f,
        't': float(row['t']),
        'fired': float(row['fired']),
        'timestamp': float(row['timestamp']),
        'target': float(row['target']),
        'refresh': float(row['refresh']),
        'tick_ms': float(row['tick_ms']),
        'render_ms': float(row['render_ms']),
        'acquire_ms': float(row['acquire_ms']),
        'submitted': float(row['submitted']),
        'divisor': int(row.get('divisor') or 1),
        'chosen': int(row.get('chosen') or row.get('divisor') or 1),
        'woken': row.get('woken') == '1',
        'gpu_end': gpu_end.get(f, 0.0),
        'presented': presented.get(f),
    }


def phases(frames):
    """Runs of link-paced frames at one chosen rate, with no silence in
    between."""
    out, current = [], []
    for fr in frames:
        if fr['timestamp'] == 0.0:
            # The watchdog's frame, or the one painted as the window is revealed.
            if current:
                out.append(current)
            current = []
            continue
        if current and (
            fr['chosen'] != current[-1]['chosen']
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
    divisor = phase[0]['chosen']
    refresh = statistics.median(fr['refresh'] for fr in phase)
    m = {
        'divisor': divisor,
        'refresh': refresh,
        'start': phase[0]['t'],
        'end': phase[-1]['t'] + phase[-1]['divisor'] * refresh,
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
            expected = link if fr['woken'] else fr['divisor']
            if expected == fr['chosen']:
                # An idle poll's gap is its pacing, not a slow present.
                m['gaps_ms'].append((fr['presented'] - previous['presented']) * 1000)
            if glass > expected:
                m['missed'] += glass - expected
                why = 'dropped before it' if dropped else cause(fr, refresh)
                m['stalls'].append((fr, glass, why))
            elif link > expected:
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
    gaps = sorted(m['gaps_ms']) or [float('nan')]
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


def stretch(frames, start, end):
    """Refreshes, missed refreshes, stalls and drawn frames per second
    between two times."""
    inside = [fr for fr in frames if start <= fr['t'] < end]
    runs = phases(inside)
    ms = [m for m in map(measure, runs) if m['refreshes']]
    drawn = sum(1 for fr in inside if fr['submitted'])
    # Visible: the time the display link paced frames, less every gap of
    # over UNSHOWN_GAP between drawn ones. A minimized editor draws only the
    # watchdog's frames, and a covered one keeps its link but refuses every
    # present, so neither says how often it asks to draw while it is seen.
    visible, paced = 0.0, 0
    for p in runs:
        edges = [p[0]['t']] + [fr['t'] for fr in p if fr['submitted']] + [p[-1]['t']]
        visible += sum(b - a for a, b in zip(edges, edges[1:]) if b - a <= UNSHOWN_GAP)
        paced += len(edges) - 2
    return {
        'refreshes': sum(m['refreshes'] for m in ms),
        'missed': sum(m['missed'] for m in ms),
        'stalls': [s for m in ms for s in m['stalls']],
        'fps': drawn / (end - start) if end > start else 0.0,
        'visible': visible,
        'visible_fps': paced / visible if visible else 0.0,
        'frames': [fr for fr in inside if fr['submitted']],
    }


def median(values):
    values = list(values)
    return statistics.median(values) if values else float('nan')


def full_rate(frames, evs):
    """The rate the link offers frames at while something animates, from the
    frames it paced then: a still editor's link is slowed to its poll (#1676),
    so its frames would read as the poll's rate."""
    busy = [(a, b) for a, b, reasons in timeline(evs, frames[-1]['t'], 'animating') if reasons]
    starts = [a for a, _ in busy]

    def animating(t):
        i = bisect.bisect_right(starts, t) - 1
        return i >= 0 and t < busy[i][1]

    paced = [fr for fr in frames if fr['timestamp'] and animating(fr['t'])]
    if not paced:
        return float('nan')
    return 1 / median(fr['refresh'] * fr['divisor'] for fr in paced)


def openings(ticks, gpu_end, presented, events):
    """Each time an editor was opened: its frames and its events, split at
    the `open` row its first frame logs."""
    window_of = {int(row['frame']): w for w, rows in ticks.items() for row in rows}
    start_of = {int(row['frame']): float(row['t']) for rows in ticks.values() for row in rows}
    by_window = {}
    for f, _, text in events:
        # Timed by the frame it was logged in rather than the moment it was
        # written, which comes after that frame's drawing: the frame a note
        # first lands on starts before its own event.
        if f in window_of:
            by_window.setdefault(window_of[f], []).append((f, start_of[f], text))
    for w, evs in by_window.items():
        frames = sorted((frame(row, gpu_end, presented) for row in ticks[w]),
                        key=lambda fr: fr['frame'])
        starts = [f for f, _, text in evs if text.startswith('open')]
        for k, first in enumerate(starts):
            last = starts[k + 1] if k + 1 < len(starts) else float('inf')
            run = [fr for fr in frames if first <= fr['frame'] < last]
            if run:
                yield w, run, [e for e in evs if first <= e[0] < last]


def timeline(evs, end, kind):
    """(start, end, words) for each stretch between this kind's events."""
    marks = [(t, text.split()[1:]) for _, t, text in evs if text.split()[0] == kind]
    return [(t, marks[i + 1][0] if i + 1 < len(marks) else end, words)
            for i, (t, words) in enumerate(marks)]


def report_silence(run, evs, t0):
    end = run[-1]['t']
    full = full_rate(run, evs)
    found = False
    for start, stop, reasons in timeline(evs, end, 'animating'):
        if reasons or stop - start < SILENCE_MIN:
            continue
        found = True
        s = stretch(run, start, stop)
        print(f'  still {start - t0:.0f}-{stop - t0:.0f} s, nothing animating')
        if s['visible'] < 10:
            print('    the editor was hidden for nearly all of it')
            continue
        drawn = s['frames']
        print(f"    drew {s['visible_fps']:.0f} frames/s over {s['visible']:.0f} s visible, "
              f"against {full:.0f} when animating; "
              f"tick {median(fr['tick_ms'] for fr in drawn):.2f} ms, "
              f"render {median(fr['render_ms'] for fr in drawn):.2f} ms, "
              f"submit to GPU done "
              f"{median((fr['gpu_end'] - fr['submitted']) * 1000 for fr in drawn if fr['gpu_end']):.2f} ms"
              f" (medians)")
        if s['visible_fps'] > full / 2:
            print('    -> draws at full rate with nothing animating: something else asks for frames')
        elif s['visible_fps'] < 30:
            print('    -> idles at the 20 Hz poll')
    if not found:
        print(f'  still: no stretch of {SILENCE_MIN:.0f} s or more with nothing animating')


def report_first_note(run, evs, t0):
    stretches = timeline(evs, run[-1]['t'], 'animating')
    note = next((start for start, _, reasons in stretches if 'voices' in reasons), None)
    if note is None:
        print('  first note: none played while open')
        return
    at_open = stretches[0][2] if stretches else []
    warm = [r for r in at_open if r in ('voices', 'roll', 'releasing')]
    print(f'  first note at {note - t0:.2f} s, {note - run[0]["t"]:.1f} s after opening'
          + (f'; opened with {" ".join(warm)} already animating, so not a cold first note'
             if warm else ''))
    s = stretch(run, note, note + NOTE_AFTER)
    for fr, glass, why in s['stalls']:
        print(f"    stall at +{(fr['t'] - note) * 1000:.0f} ms: {glass} refreshes, {why}; "
              f"tick {fr['tick_ms']:.2f} ms")
    if not s['stalls']:
        print(f'    no stall in the {NOTE_AFTER} s after it')
    # `render_ms` is the part of `tick_ms` spent rendering, so the tick is the
    # frame's whole main-thread cost. A costly frame can still miss nothing,
    # when the frames queued ahead of it cover the time it took.
    usual = median(fr['tick_ms'] for fr in run if fr['submitted'])
    print(f'    costliest frames, main-thread ms (this opening\'s median {usual:.2f}):')
    for fr in sorted(s['frames'], key=lambda fr: -fr['tick_ms'])[:3]:
        print(f"      +{(fr['t'] - note) * 1000:.0f} ms: tick {fr['tick_ms']:.2f} "
              f"(render {fr['render_ms']:.2f}), submitted "
              f"{(fr['submitted'] - fr['target']) * 1000:+.2f} ms from its target")


def report_exports(run, evs, t0):
    # One stretch per run of time with any export running, however many
    # overlap in it, so the time before it is never inside another export.
    busy = []
    for start, stop, words in timeline(evs, run[-1]['t'], 'exports'):
        if words[0] == '0':
            continue
        if busy and busy[-1]['stop'] == start:
            busy[-1]['stop'] = stop
            busy[-1]['most'] = max(busy[-1]['most'], int(words[0]))
        else:
            # A log from a build that named the pane's Encoder after the count
            # still reads: the word is ignored, as it was not what ran.
            busy.append({'start': start, 'stop': stop, 'most': int(words[0])})
    quiet_since = run[0]['t']
    for b in busy:
        start, stop = b['start'], b['stop']
        length = min(EXPORT_COMPARE, stop - start, start - quiet_since)
        quiet_since = stop
        if length <= 0:
            print(f'  export {start - t0:.0f}-{stop - t0:.0f} s already running when the editor '
                  f'opened: no time before it to compare')
            continue
        print(f"  export {start - t0:.0f}-{stop - t0:.0f} s, up to {b['most']} running; "
              f"{length:.0f} s each side:")
        for name, a in (('before', start - length), ('during', start)):
            s = stretch(run, a, a + length)
            print(f"    {name:6} drew {s['fps']:.0f} frames/s, missed {s['missed']} of "
                  f"{s['refreshes']} refreshes ({pct(s['missed'], s['refreshes'])}) "
                  f"in {len(s['stalls'])} stalls")


def report_events(ticks, gpu_end, presented, events):
    t0 = min(float(rows[0]['t']) for rows in ticks.values())
    found = False
    for w, run, evs in openings(ticks, gpu_end, presented, events):
        found = True
        print(f"window {w} opened at {run[0]['t'] - t0:.0f} s, "
              f"{run[-1]['t'] - run[0]['t']:.0f} s open")
        report_silence(run, evs, t0)
        report_first_note(run, evs, t0)
        report_exports(run, evs, t0)
        print()
    if not found:
        sys.exit('no event rows: the log predates them')


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('log', nargs='?', help='frames-*.csv (default: the newest)')
    parser.add_argument('--window', help='window id (default: the one with most frames)')
    parser.add_argument('--from', dest='start', type=float, default=float('-inf'),
                        help="seconds after the window's first frame")
    parser.add_argument('--to', dest='end', type=float, default=float('inf'))
    parser.add_argument('--misses', action='store_true', help='list every stall')
    parser.add_argument('--events', action='store_true',
                        help='report on silence, first notes and exports instead')
    args = parser.parse_args()

    path = args.log or newest_log()
    ticks, gpu_end, presented, events, has_divisor = read(path)
    if not ticks:
        sys.exit(f'{path}: no tick rows')
    print(path)
    if args.events:
        report_events(ticks, gpu_end, presented, events)
        return
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
