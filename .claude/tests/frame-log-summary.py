#!/usr/bin/env python3
"""The summary measures each frame against the divisor that paced it."""
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
    """Drawn frames at a chosen Display rate, from (refreshes since the last,
    logged divisor, refreshes late to the glass, the divisor an input event
    cut short to wake it or 0)."""
    out, k = [], 0
    for n, (step, divisor, late, woken) in enumerate(paced, 1):
        k += step
        at = 100 + k * REFRESH
        out.append({
            'frame': n, 't': at, 'fired': at, 'timestamp': at, 'target': at + REFRESH,
            'refresh': REFRESH, 'tick_ms': 1.0, 'render_ms': 0.5, 'acquire_ms': 0.0,
            'submitted': at + 0.002, 'divisor': divisor, 'chosen': 1, 'woken': woken, 'gpu_end': 0.0,
            'presented': at + (2 + late) * REFRESH,
        })
    return out


class Pacing(unittest.TestCase):
    def missed(self, paced):
        [phase] = summary.phases(frames(paced))
        return summary.measure(phase)['missed']

    def test_the_frame_after_a_first_note_from_idle_is_held_to_the_chosen_rate(self):
        # Idle at the poll's divisor 8; the note's frame is paced at 8, and
        # the next at the chosen 1, which reaches the glass 3 refreshes late.
        idle = [(8, 8, 0, 0)] * 20
        note = (8, 8, 0, 0)
        self.assertEqual(self.missed(idle + [note, (1, 1, 3, 0), (1, 1, 3, 0)]), 3)
        # The note's frame blocks the main thread, so the link skips to the
        # fourth refresh: that is a stall, not an input event cutting a poll.
        self.assertEqual(self.missed(idle + [note, (4, 1, 0, 0), (1, 1, 0, 0)]), 3)

    def test_an_input_event_cutting_the_poll_short_is_no_stall(self):
        # The event restores divisor 1 between frames, five refreshes into
        # the poll's eight, and the next refresh draws on time.
        idle = [(8, 8, 0, 0)] * 20
        self.assertEqual(self.missed(idle + [(5, 1, 0, 8), (1, 1, 0, 0)]), 0)
        # A second sparse event wakes the poll its frame set going again.
        self.assertEqual(self.missed(idle + [(5, 1, 0, 8), (3, 1, 0, 8)]), 0)
        # One that lands during a hitch does not hide it: the frame was due
        # by the poll's eighth refresh and the link reached it at the 13th.
        self.assertEqual(self.missed(idle + [(13, 1, 0, 8), (1, 1, 0, 0)]), 5)

    def test_an_idle_polls_pacing_is_no_present_gap(self):
        [phase] = summary.phases(frames([(8, 8, 0, 0)] * 20 + [(1, 1, 0, 0)] * 20))
        self.assertEqual(round(max(summary.measure(phase)['gaps_ms']) / 1000 / REFRESH), 1)


if __name__ == "__main__":
    unittest.main()
