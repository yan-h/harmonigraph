#!/usr/bin/env python3
"""The summary reads the log the frame log writes now: phases split at a
divisor change and at a reveal's frame, and a still editor's idle ticks start
no glass gap."""
import importlib.util
from pathlib import Path
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

    def test_an_idle_tick_starts_no_gap_and_a_stall_after_it_still_counts(self):
        idle = [(1, 1, 0, 'idle')] * 40
        # The first drawn frame after the idle ticks has no gap behind it.
        [m] = self.measured([drawn(1)] * 5 + idle + [drawn(1), drawn(1)])
        self.assertEqual(m['missed'], 0)
        # The next one reaching the glass three refreshes late is a stall.
        [m] = self.measured([drawn(1)] * 5 + idle + [drawn(1), drawn(1, late=3)])
        self.assertEqual(m['missed'], 3)

    def test_a_reveal_frame_ends_a_phase_and_counts_in_none(self):
        runs = summary.phases(frames([drawn(1)] * 5 + [(1, 1, 0, 'reveal')] + [drawn(1)] * 5))
        self.assertEqual([len(run) for run in runs], [5, 5])

    def test_a_divisor_change_starts_a_phase_measured_at_its_own_divisor(self):
        ms = self.measured([drawn(1)] * 10 + [drawn(2, 2)] * 10)
        self.assertEqual([(m['divisor'], m['missed']) for m in ms], [(1, 0), (2, 0)])

    def test_the_full_rate_is_read_off_the_frames_the_link_paced(self):
        paced = frames([drawn(2, 2)] * 10 + [(1, 2, 0, 'reveal')])
        self.assertAlmostEqual(summary.full_rate(paced), 72.0)


if __name__ == "__main__":
    unittest.main()
