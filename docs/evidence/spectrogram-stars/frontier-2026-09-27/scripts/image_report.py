#!/usr/bin/env python3
"""Compare RGBA take/flat buffers and emit a labelled crop sheet.

Example: python3 image_report.py --dir /path/to/rgba --width 1920 --height 1080
Recognizes <case>-take.rgba and <case>-flat.rgba files.
"""
import argparse
import importlib.util
import json
import re
from pathlib import Path

import numpy as np

PNG_KIT = Path('/Users/yan/projects/harmonigraph/.claude/skills/look-prototype/kit/png.py')


def case_of(path):
    return re.sub(r'-(take|flat)\.rgba$', '', path.name)


def load_rgb(path, width, height):
    raw = np.fromfile(path, dtype=np.uint8)
    expected = width * height * 4
    if raw.size != expected:
        raise ValueError(f'{path}: expected {expected} RGBA bytes, got {raw.size}')
    return raw.reshape(height, width, 4)[:, :, :3]


def metrics(image, baseline):
    delta = np.abs(image.astype(np.int16) - baseline.astype(np.int16))
    return {"mean_absolute_rgb": float(delta.mean()), "max_absolute_rgb": int(delta.max()),
            "p99_absolute_rgb": float(np.percentile(delta, 99)),
            "different_fraction": float(np.mean(np.any(delta != 0, axis=2)))}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--dir', required=True, type=Path)
    parser.add_argument('--width', required=True, type=int)
    parser.add_argument('--height', required=True, type=int)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--sheet', type=Path)
    args = parser.parse_args()
    if args.width <= 0 or args.height <= 0:
        parser.error('--width and --height must be positive')
    files = sorted(set(args.dir.glob('*-take.rgba')) | set(args.dir.glob('*-flat.rgba')))
    data = {}
    for path in files:
        data.setdefault(case_of(path), {})[path.name.rsplit('-', 1)[-1].split('.')[0]] = load_rgb(path, args.width, args.height)
    if 'full-a' not in data or 'take' not in data['full-a']:
        raise ValueError(f'{args.dir}: required baseline full-a-take.rgba not found')
    baseline_cases = ['full-a'] + (['half-a'] if 'half-a' in data else [])
    report = {"directory": str(args.dir), "width": args.width, "height": args.height, "comparisons": {}}
    for name, channels in sorted(data.items()):
        report['comparisons'][name] = {}
        for image_kind, image in sorted(channels.items()):
            report['comparisons'][name][image_kind] = {}
            for baseline_name in baseline_cases:
                baseline = data[baseline_name].get(image_kind)
                if baseline is None:
                    continue
                if name == baseline_name:
                    continue
                report['comparisons'][name][image_kind][f'vs_{baseline_name}'] = metrics(image, baseline)

    out_json = args.output or args.dir / 'image-report.json'
    out_json.write_text(json.dumps(report, indent=2, sort_keys=True) + '\n')

    spec = importlib.util.spec_from_file_location('look_png', PNG_KIT)
    kit = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(kit)
    scale = args.width / 1920.0
    x, y, w, h = (int(round(v * scale)) for v in (800, 350, 300, 220))
    x, y = min(max(0, x), args.width - 1), min(max(0, y), args.height - 1)
    w, h = min(w, args.width - x), min(h, args.height - y)
    names = ['full-a'] + (['half-a'] if 'half-a' in data else []) + [n for n in sorted(data) if n not in ('full-a', 'half-a')]
    cells = []
    for name in names:
        image = data[name].get('take', data[name].get('flat'))
        if image is None:
            continue
        crop = image[y:y+h, x:x+w].copy()
        crop = np.repeat(np.repeat(crop, 2, axis=0), 2, axis=1)
        kit.label_cell(crop, [name.upper(), 'TAKE CROP 2X'], scale=2)
        cells.append(crop)
    out_sheet = args.sheet or args.dir / 'image-crops.png'
    kit.write_png(out_sheet, kit.sheet(cells, cols=2, title='RGBA TAKE COMPARISON', tscale=2))
    print(json.dumps({"json": str(out_json), "sheet": str(out_sheet), "cases": names}, sort_keys=True))


if __name__ == '__main__':
    main()
