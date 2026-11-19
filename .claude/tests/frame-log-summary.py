#!/usr/bin/env python3
"""The summary reads the log the frame log writes now: phases split at a
divisor change and at a reveal's frame, and a still editor's idle ticks start
no glass gap."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "summary", Path(__file__).resolve().parents[2] / "tools/frame-log-summary.py"
)
summary = importlib.util.module_from_spec(spec)
spec.loader.exec_module(summary)

REFRESH = 1 / 144


def frames(paced):
    """Tick rows from (refreshes since the last, divisor, refreshes late to
    the glass, kind): kind 'drawn' submits and presents, 'idle' is a link frame
    that ran no UI pass, 'reveal' a frame the link did not ask for."""
    out, k = [], 0
    for n, (step, divisor, late, kind) in enumerate(paced, 1):
        k += step
        at = 100 + k * REFRESH
        link = 0.0 if kind == 'reveal' else at
        drawn = kind != 'idle'
        out.append({
            'frame': n, 't': at, 'fired': link, 'timestamp': link, 'target': link + REFRESH,
            'refresh': REFRESH, 'tick_ms': 1.0, 'render_ms': 0.5 if drawn else 0.0,
            'acquire_ms': 0.0, 'submitted': at + 0.002 if drawn else 0.0, 'divisor': divisor,
            'gpu_end': 0.0, 'presented': at + (2 + late) * REFRESH if drawn else None,
        })
    return out


def drawn(step, divisor=1, late=0):
    return (step, divisor, late, 'drawn')


class Pacing(unittest.TestCase):
    def measured(self, paced):
        return [summary.measure(phase) for phase in summary.phases(frames(paced))]

    def test_a_gap_misses_each_refresh_past_the_divisor_that_paced_it(self):
        [m] = self.measured([drawn(1)] * 10 + [drawn(3)] + [drawn(1)] * 10)
        self.assertEqual(m['missed'], 2)
        [m] = self.measured([drawn(2, 2)] * 10 + [drawn(4, 2)] + [drawn(2, 2)] * 10)
        self.assertEqual(m['missed'], 2)
        self.assertEqual(len(m['stalls']), 1)

    def test_idle_ticks_are_no_miss_and_a_frame_late_after_them_is(self):
        idle = [(1, 1, 0, 'idle')] * 40
        # Drawn on the refresh after the last idle tick: on time.
        [m] = self.measured([drawn(1)] * 5 + idle + [drawn(1), drawn(1)])
        self.assertEqual(m['missed'], 0)
        # Reaching the glass three refreshes late, first after the idle ticks
        # or second, is a stall either way.
        [m] = self.measured([drawn(1)] * 5 + idle + [drawn(1, late=3), drawn(1, late=3)])
        self.assertEqual(m['missed'], 3)
        [m] = self.measured([drawn(1)] * 5 + idle + [drawn(1), drawn(1, late=3)])
        self.assertEqual(m['missed'], 3)

    def test_frames_drawn_at_a_chrome_cues_30_hz_are_still_measured(self):
        # A frame every fifth refresh, four idle ticks between, as the armed
        # Record button or an export's stripes ask with nothing animating.
        def cadence(late):
            return ([(1, 1, 0, 'idle')] * 4 + [drawn(1)]) * 10 + \
                [(1, 1, 0, 'idle')] * 4 + [drawn(1, late=late)]
        [m] = self.measured(cadence(0))
        self.assertEqual(m['missed'], 0)
        [m] = self.measured(cadence(3))
        self.assertEqual(m['missed'], 3)

    def test_a_reveal_frame_ends_a_phase_and_counts_in_none(self):
        runs = summary.phases(frames([drawn(1)] * 5 + [(1, 1, 0, 'reveal')] + [drawn(1)] * 5))
        self.assertEqual([len(run) for run in runs], [5, 5])

    def test_a_divisor_change_starts_a_phase_measured_at_its_own_divisor(self):
        ms = self.measured([drawn(1)] * 10 + [drawn(2, 2)] * 10)
        self.assertEqual([(m['divisor'], m['missed']) for m in ms], [(1, 0), (2, 0)])

    def test_the_full_rate_is_each_phases_own(self):
        paced = frames([drawn(1)] * 10 + [drawn(2, 2)] * 10 + [(1, 2, 0, 'reveal')])
        self.assertEqual([round(r) for r in summary.full_rates(paced)], [144, 72])

    def test_a_log_from_before_1747_is_refused(self):
        header = 'kind,frame,t,fired,timestamp,target,refresh,tick_ms,acquire_ms,render_ms,' \
                 'submitted,drawables,gpu_start,gpu_end,presented,window,divisor,chosen,woken,event\n'
        with tempfile.NamedTemporaryFile('w', suffix='.csv') as log:
            log.write(header)
            log.flush()
            with self.assertRaises(SystemExit) as refused:
                summary.read(log.name)
        self.assertIn('record a fresh one', str(refused.exception.code))
        # Or one whose link a still editor slowed to its poll.
        with tempfile.NamedTemporaryFile('w', suffix='.csv') as log:
            log.write(header.replace(',chosen,woken', ''))
            log.write('tick,1,100,100,100,100.007,0.0069,1,0,0,0,,,,,w,8,\n')
            log.flush()
            with self.assertRaises(SystemExit):
                summary.read(log.name)


if __name__ == "__main__":
    unittest.main()
