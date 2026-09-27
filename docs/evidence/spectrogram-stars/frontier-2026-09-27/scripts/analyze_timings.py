#!/usr/bin/env python3
"""Summarize paired GPU timing CSVs and block-bootstrap paired savings.

Usage: python3 analyze_timings.py run.csv [more.csv ...]
Each CSV must contain frame,slot,case,gpu_ms,wall_ms. Extra *_ms columns
are retained only as input; summaries are produced for gpu_ms.
"""
import argparse
import csv
import json
import math
import statistics
from pathlib import Path

import numpy as np

BLOCK_FRAMES = 12
BOOTSTRAP_SAMPLES = 2000
RNG_SEED = 0
CONTROL_PAIRS = (("half-a", "half-b"), ("full-a", "full-b"))


def read_csv(path):
    rows = []
    with path.open(newline="") as f:
        reader = csv.DictReader(f)
        required = {"frame", "slot", "case", "gpu_ms", "wall_ms"}
        if reader.fieldnames is None or not required.issubset(reader.fieldnames):
            raise ValueError(f"{path}: required columns: {', '.join(sorted(required))}")
        for line, row in enumerate(reader, 2):
            try:
                frame = int(row["frame"])
                gpu = float(row["gpu_ms"])
                wall = float(row["wall_ms"])
            except (TypeError, ValueError) as e:
                raise ValueError(f"{path}:{line}: invalid frame or timing") from e
            if not math.isfinite(gpu) or gpu <= 0:
                raise ValueError(f"{path}:{line}: gpu_ms must be finite and positive")
            if not math.isfinite(wall) or wall <= 0:
                raise ValueError(f"{path}:{line}: wall_ms must be finite and positive")
            rows.append({"frame": frame, "slot": row["slot"], "case": row["case"], "gpu_ms": gpu, "wall_ms": wall})
    if not rows:
        raise ValueError(f"{path}: no data rows")
    return rows


def analyze(path):
    rows = read_csv(path)
    seen = set()
    by_frame = {}
    by_case = {}
    slots_by_frame = {}
    slot_counts = {}
    for row in rows:
        key = (row["frame"], row["case"])
        if key in seen:
            raise ValueError(f"{path}: duplicate frame/case {key}")
        seen.add(key)
        slots = slots_by_frame.setdefault(row["frame"], set())
        if row["slot"] in slots:
            raise ValueError(f"{path}: duplicate slot {row['slot']!r} in frame {row['frame']}")
        slots.add(row["slot"])
        slot_counts.setdefault(row["case"], {}).setdefault(row["slot"], 0)
        slot_counts[row["case"]][row["slot"]] += 1
        by_frame.setdefault(row["frame"], {})[row["case"]] = row["gpu_ms"]
        by_case.setdefault(row["case"], []).append(row["gpu_ms"])

    cases = sorted(by_case)
    expected = set(cases)
    for frame, values in by_frame.items():
        if set(values) != expected:
            raise ValueError(f"{path}: frame {frame} has {len(values)} cases; expected {len(expected)}")
    counts = {case: len(vals) for case, vals in by_case.items()}
    if len(set(counts.values())) != 1:
        raise ValueError(f"{path}: case counts differ across rounds: {counts}")

    slot_histograms = {case: {str(slot): n for slot, n in sorted(hist.items())}
                       for case, hist in sorted(slot_counts.items())}
    positions_balanced = all(len(set(hist.values())) <= 1 for hist in slot_counts.values())
    summaries = {case: {"n": len(vals), "mean_gpu_ms": statistics.mean(vals), "median_gpu_ms": statistics.median(vals)}
                 for case, vals in sorted(by_case.items())}
    control_pairs = [pair for pair in CONTROL_PAIRS if set(pair).issubset(expected)]
    result = {"input": str(path), "frames": len(by_frame), "cases": summaries,
              "slot_histograms": slot_histograms, "slot_positions_balanced": positions_balanced,
              "paired_control_cases": [list(pair) for pair in control_pairs],
              "comparisons": {}, "bootstrap": {"block_frames": BLOCK_FRAMES,
              "samples": BOOTSTRAP_SAMPLES, "seed": RNG_SEED,
              "description": "descriptive block-bootstrap interval; not a calibrated guarantee"}}
    if not control_pairs:
        result["pairing_note"] = "No complete half-a/half-b or full-a/full-b control pair; paired comparisons omitted."
        return result

    frames = sorted(by_frame)
    blocks = [np.arange(i, min(i + BLOCK_FRAMES, len(frames))) for i in range(0, len(frames), BLOCK_FRAMES)]
    rng = np.random.default_rng(RNG_SEED)
    for a, b in control_pairs:
        a_vals = np.array([by_frame[f][a] for f in frames], dtype=float)
        b_vals = np.array([by_frame[f][b] for f in frames], dtype=float)
        control_name = f"{a}/{b}"
        control = (a_vals + b_vals) / 2
        aa_ratio = float(np.mean(b_vals) / np.mean(a_vals))
        result.setdefault("aa_noise", {})[control_name] = {
            "mean_savings_percent": float(100 * (1 - aa_ratio)),
            "mean_ratio_b_over_a": aa_ratio}
        all_control_cases = {name for pair in control_pairs for name in pair}
        variants = [c for c in cases if c not in all_control_cases]
        for variant in variants:
            values = np.array([by_frame[f][variant] for f in frames], dtype=float)
            point = float(1 - np.mean(values) / np.mean(control))
            boot = np.empty(BOOTSTRAP_SAMPLES)
            for sample in range(BOOTSTRAP_SAMPLES):
                chosen = rng.integers(0, len(blocks), size=len(blocks))
                idx = np.concatenate([blocks[i] for i in chosen])
                boot[sample] = 1 - np.mean(values[idx]) / np.mean(control[idx])
            result["comparisons"].setdefault(variant, {})[control_name] = {
                "paired_mean_savings_percent": 100 * point,
                "mean_variant_over_control_ratio": float(np.mean(values) / np.mean(control)),
                "block_bootstrap_95pct_interval_savings_percent": [float(100 * x) for x in np.quantile(boot, [0.025, 0.975])]}
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("csv", nargs="+", type=Path, help="timing CSV input(s)")
    args = parser.parse_args()
    for path in args.csv:
        result = analyze(path)
        out = path.with_suffix(".analysis.json")
        out.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
        print(json.dumps(result, sort_keys=True))


if __name__ == "__main__":
    main()
