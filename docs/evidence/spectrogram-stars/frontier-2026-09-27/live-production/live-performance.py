#!/usr/bin/python3
"""Plot benchmark JSON using Pillow. Run with /usr/bin/python3 on this Mac."""
from pathlib import Path
import json
import shutil
from PIL import Image, ImageDraw, ImageFont

ROOT = Path('/private/tmp/stars-investigation')
OUTPUT = ROOT / 'live-performance.png'
DISPLAY = Path('/Users/yan/.codex/visualizations/2026/09/27/01a0e477-3f39-7370-aa91-6e50cd112eb6/stars-research/live-performance.png')
image = Image.new('RGB', (1500, 850), '#fafbfc')
d = ImageDraw.Draw(image)
regular = '/System/Library/Fonts/Supplemental/Arial.ttf'
bold = '/System/Library/Fonts/Supplemental/Arial Bold.ttf'
def font(size, strong=False):
    return ImageFont.truetype(bold if strong else regular, size)
ink, muted, grid = '#243144', '#536170', '#dce3e9'
d.text((65, 42), 'Live Stars profiles', font=font(42, True), fill=ink)
d.text((65, 102), 'Measured on M1 Pro / Metal · recorded scrolling fixture', font=font(24), fill=muted)
colors = ['#80939f', '#207b91', '#4268a5', '#495469']
summary = {}
for panel, (pixels, title, axis_max, ticks) in enumerate([
    ('1920x1080', '1080p · 1920 × 1080 pixels', 10, [0, 2, 4, 6, 8, 10]),
    ('3840x2160', '4K · 3840 × 2160 pixels', 32, [0, 10, 20, 30]),
]):
    data = json.loads((ROOT / f'live-take-{pixels}-analysis.json').read_text())
    c = data['cases']
    vals = [(c['half-a']['mean_gpu_ms'] + c['half-b']['mean_gpu_ms']) / 2,
            c['p2']['mean_gpu_ms'], c['p3']['mean_gpu_ms'],
            (c['full-a']['mean_gpu_ms'] + c['full-b']['mean_gpu_ms']) / 2]
    savings = [100 * (1 - v / vals[-1]) for v in vals]
    summary[pixels] = dict(zip(['Half', 'P2', 'P3', 'Full'],
                               [{'mean_gpu_ms': v, 'savings_vs_full_percent': s}
                                for v, s in zip(vals, savings)]))
    x0 = 65 + panel * 740
    left, width = x0 + 78, 390
    d.text((x0, 194), title, font=font(27, True), fill=ink)
    for tick in ticks:
        x = left + tick / axis_max * width
        d.line((x, 274, x, 646), fill=grid, width=1)
        text = str(tick)
        d.text((x - d.textlength(text, font=font(19)) / 2, 657), text, font=font(19), fill=muted)
    for row, (name, value, saved, color) in enumerate(zip(['Half', 'P2', 'P3', 'Full'], vals, savings, colors)):
        y = 304 + row * 91
        d.text((x0, y + 7), name, font=font(24, True), fill=ink)
        end = left + value / axis_max * width
        d.rectangle((left, y, end, y + 43), fill=color)
        d.text((end + 12, y - 4), f'{value:.2f} ms', font=font(23, True), fill=ink)
        label = f'{saved:.1f}% less than Full' if name != 'Full' else 'reference'
        d.text((end + 12, y + 27), label, font=font(18), fill=muted)
    d.text((left - 10, 697), 'Mean source → composite GPU time (ms)', font=font(20), fill=muted)
d.text((65, 761), 'Same 960 × 540 pt pane · 60 warm-up rounds + 240 measured rounds per case', font=font(22), fill=muted)
d.text((65, 803), 'Half and Full average two A/A controls; P2 and P3 are measured directly. Scope excludes the rest of the DAW frame.', font=font(19), fill=muted)
image.save(OUTPUT)
DISPLAY.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(OUTPUT, DISPLAY)
(ROOT / 'live-performance-summary.json').write_text(json.dumps(summary, indent=2) + '\n')
print(OUTPUT)
print(DISPLAY)
