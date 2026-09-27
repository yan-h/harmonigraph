#!/usr/bin/env python3
"""Bounded CPU-only opacity screen of complete analytic Stars layers 3/4.

Inputs are actual packed atlas records and frame uniforms. Explicit fringe is
required because the original JSON exporter omitted it. This computes the full
analytic response, not current half-resolution residual interpolation, and is
an opportunity estimate rather than bit-exact emulation of Metal arithmetic.
"""
import argparse
import json
from pathlib import Path
import time
import numpy as np

p = argparse.ArgumentParser()
p.add_argument("metadata", type=Path)
p.add_argument("--fringe", type=float, action="append", required=True)
p.add_argument("--fringe-provenance", required=True)
p.add_argument("--width", type=int, default=1920)
p.add_argument("--height", type=int, default=1080)
p.add_argument("--rows", type=int, default=32)
p.add_argument("--budget-seconds", type=float, default=25)
p.add_argument("--output", type=Path, required=True)
args = p.parse_args()
started = time.monotonic()
meta = json.loads(args.metadata.read_text())
assert all(0 <= v <= .5 for v in args.fringe)
assert args.width == 1920 and args.height == 1080, "this screen is bounded to one 1080p frame"
assert 1 <= args.rows <= 64
f = np.float32
atlas = np.memmap(args.metadata.with_suffix(".rgba32u"), dtype="<u4", mode="r", shape=(meta["atlas_size"][0] * meta["atlas_size"][1], 4))
assert atlas.shape[0] == meta["atlas_size"][0] * meta["atlas_size"][1]
pane = np.asarray(meta["pane_size"], dtype=np.float32)
step = pane / np.asarray([args.width, args.height], dtype=np.float32)
x_sp = ((np.arange(args.width, dtype=np.float32) + f(.5)) * step[0] - pane[0] * f(.5)) * f(540.0 / pane[1])
results = {fringe: {} for fringe in args.fringe}

for k in [3, 4]:
    s = meta["slices"][k]
    cell = f(s["cell"])
    offset = np.asarray(s["offset"], dtype=np.float32)
    offset_floor = np.floor(offset).astype(np.int64)
    offset_fraction = offset - offset_floor.astype(np.float32)
    rx = x_sp / cell - offset_fraction[0]
    ox = np.floor(rx).astype(np.int64)
    fx = (rx - ox.astype(np.float32))[None, :]
    local_x = ox - offset_floor[0] - s["origin"][0]
    outer = f(1.2) * cell
    start = f(.7) * outer
    width = outer - start
    sums = {v: np.empty((args.height, args.width), dtype=np.float32) for v in args.fringe}
    for first in range(0, args.height, args.rows):
        if time.monotonic() - started > args.budget_seconds:
            raise SystemExit("CPU wall budget exceeded; no incomplete whole-frame counts reported")
        last = min(args.height, first + args.rows)
        y_sp = ((np.arange(first, last, dtype=np.float32) + f(.5)) * step[1] - pane[1] * f(.5)) * f(540.0 / pane[1])
        ry = y_sp / cell - offset_fraction[1]
        oy = np.floor(ry).astype(np.int64)
        fy = (ry - oy.astype(np.float32))[:, None]
        local_y = oy - offset_floor[1] - s["origin"][1]
        base_index = s["base"] + local_y[:, None] * s["grid"][0] + local_x[None, :]
        block = {v: np.zeros((last-first, args.width), dtype=np.float32) for v in args.fringe}
        for dy in [-1, 0, 1]:
            for dx in [-1, 0, 1]:
                index = base_index + dy * s["grid"][0] + dx
                assert index.min() >= s["base"] and index.max() < s["base"] + s["grid"][0]*s["grid"][1]
                t = np.asarray(atlas[index])
                center_x = t[..., 0].copy().view("<f4")
                center_y = t[..., 1].copy().view("<f4")
                inverse_sigma = (t[..., 3] & np.uint32(65535)).astype("<u2").view("<f2").astype(np.float32)
                life = (t[..., 3] >> np.uint32(16)).astype("<u2").view("<f2").astype(np.float32)
                dist_x = fx - f(dx) - center_x
                dist_y = fy - f(dy) - center_y
                distance = np.sqrt(dist_x * dist_x + dist_y * dist_y) * cell
                d = distance * inverse_sigma
                gaussian = np.exp(f(-.5) * d * d)
                fringe_shape = np.exp(f(-.4) * d)
                fade_position = np.clip((distance - start) / width, f(0), f(1))
                outer_fade = f(1) - fade_position * fade_position * (f(3) - f(2) * fade_position)
                live = (t[..., 3] != 0) & (distance < outer)
                for fringe in args.fringe:
                    full = np.minimum(gaussian + f(fringe) * fringe_shape, f(1))
                    cover = (full * outer_fade) * life
                    cover[~live] = f(0)
                    block[fringe] += cover
        for fringe in args.fringe:
            sums[fringe][first:last] = block[fringe]
    for fringe in args.fringe:
        assert np.isfinite(sums[fringe]).all() and (sums[fringe] >= 0).all()
        results[fringe][k] = sums[fringe]

def mask_stats(mask):
    # 1080 and1920 are both divisible by8; no partial tiles in this screen.
    tiles = mask.reshape(args.height//8, 8, args.width//8, 8).all(axis=(1, 3))
    return dict(pixels=int(mask.sum()), pixel_fraction=float(mask.mean()),
                all_pixels_tiles8=int(tiles.sum()), tiles8_fraction=float(tiles.mean()),
                total_pixels=int(mask.size), total_tiles8=int(tiles.size))

report = dict(metadata=str(args.metadata), resolution=[args.width,args.height],
    fringe_provenance=args.fringe_provenance,
    model="f32 NumPy complete analytic response from actual packed atlas; no RGBA16 target quantization or half-resolution interpolation",
    exact_zero_definition="sum coverage >=1 in at least one covering near layer; no epsilon opacity threshold and no transmittance underflow test",
    limits=["Metal arithmetic may differ near thresholds", "final image is not reconstructed; only complete-response alpha", "mask footprint erosion and mask-generation costs are excluded"],
    cases=[])
for fringe in args.fringe:
    raw3, raw4 = results[fringe][3], results[fringe][4]
    alpha3, alpha4 = np.minimum(raw3, f(1)), np.minimum(raw4, f(1))
    trans4 = 1 - alpha4.astype(np.float64)
    trans34 = (1 - alpha3.astype(np.float64)) * trans4
    for name, trans, exact in [("layer4", trans4, raw4 >= f(1)), ("layers3+4", trans34, (raw3 >= f(1)) | (raw4 >= f(1)))]:
        row = dict(fringe=fringe, coverage=name, exactly_opaque=mask_stats(exact),
                   mean_transmittance=float(trans.mean()),
                   merely_small_nonzero={str(v): mask_stats((trans > 0) & (trans <= v)) for v in [.001,.01,.05,.1]},
                   opaque_or_small={str(v): mask_stats(trans <= v) for v in [.001,.01,.05,.1]})
        report["cases"].append(row)
    report.setdefault("threshold_diagnostics", []).append(dict(fringe=fringe,
        layer3_sum_exactly1=int((raw3 == f(1)).sum()), layer4_sum_exactly1=int((raw4 == f(1)).sum()),
        layer3_sum_within_1e_5_of1=int((np.abs(raw3-f(1)) <= f(1e-5)).sum()),
        layer4_sum_within_1e_5_of1=int((np.abs(raw4-f(1)) <= f(1e-5)).sum())))
report["elapsed_seconds"] = time.monotonic() - started
args.output.write_text(json.dumps(report, indent=2))
for row in report["cases"]:
    exact = row["exactly_opaque"]
    print(f"fringe={row['fringe']} {row['coverage']}: opaque pixels={exact['pixel_fraction']:.4%}, opaque8x8={exact['tiles8_fraction']:.4%}, meanT={row['mean_transmittance']:.5f}")
print(f"elapsed {report['elapsed_seconds']:.3f}s; saved {args.output}")
