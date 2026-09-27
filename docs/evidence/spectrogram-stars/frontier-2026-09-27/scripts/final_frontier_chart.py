#!/usr/bin/env python3
"""Render the final measured GPU comparison once final-frontier runs exist.

Reads timings/final-frontier/{1920x1080,3840x2160}.analysis.json. No GPU work.
Uses actual mean_gpu_ms per case and each run's mean full-a/full-b controls.
Writes final-frontier-performance.png and final-frontier-performance.json in scratch.

Image MSE is deliberately absent: the available three-phase images are 1080p
only, and the final timing cases may use a separate texture implementation.
A matched image panel needs exact same implementation, input, camera, time,
resolution, and controls for each compared profile. RGB MSE is diagnostic,
not a perceptual-quality score.
"""
from __future__ import annotations

import json
import math
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path('/private/tmp/stars-investigation')
SIZES = ('1920x1080', '3840x2160')
PROFILES = [
    ('Full', (100,100,100,100,100), 'reference', ('full-a','full-b')),
    ('Half', (50,50,50,50,50), 'anchor', ('half-a','half-b','full-v50_50_50_50_50')),
    ('Uniform 75', (75,75,75,75,75), 'anchor', ('r750','full-v75_75_75_75_75')),
    ('Uniform 87.5', (87.5,)*5, 'anchor', ('r875','full-v87.5_87.5_87.5_87.5_87.5')),
    ('P2', (50,50,80,100,50), 'accepted', ('full-grouped-v50_50_80_100_50','full-separate-v50_50_80_100_50','full-v50_50_80_100_50')),
    ('P3', (50,50,100,100,60), 'accepted', ('full-grouped-v50_50_100_100_60','full-separate-v50_50_100_100_60','full-v50_50_100_100_60')),
    ('F', (50,50,100,100,100), 'accepted', ('full-grouped-v50_50_100_100_100','full-separate-v50_50_100_100_100','full-v50_50_100_100_100','full-mix5')),
]


def halo_mib(size: str, levels: tuple[float,...]) -> float:
    width,height=map(int,size.split('x'))
    return sum(math.ceil(width*r/100)*math.ceil(height*r/100)*8 for r in levels)/2**20


def load_size(size: str) -> dict | None:
    path=ROOT/'timings'/'final-frontier'/f'{size}.analysis.json'
    if not path.exists():return None
    report=json.loads(path.read_text())
    cases=report['cases']
    if not {'full-a','full-b'}<=cases.keys():
        raise ValueError(f'{path}: missing full-a/full-b')
    full=(cases['full-a']['mean_gpu_ms']+cases['full-b']['mean_gpu_ms'])/2
    rows=[]
    for label,levels,status,aliases in PROFILES:
        if label=='Full':
            selected=['full-a','full-b']
            ms=full
        elif label=='Half' and 'half-a' in cases and 'half-b' in cases:
            selected=['half-a','half-b']
            ms=(cases['half-a']['mean_gpu_ms']+cases['half-b']['mean_gpu_ms'])/2
        else:
            selected=[next((name for name in aliases if name in cases),None)]
            ms=cases[selected[0]]['mean_gpu_ms'] if selected[0] else None
        rows.append({'label':label,'levels_percent':levels,'status':status,
                     'matched_timing_cases':selected if selected[0] else [],
                     'mean_gpu_ms':ms,'percent_of_same_run_full':100*ms/full if ms is not None else None,
                     'halo_texture_mib':halo_mib(size,levels)})
    return {'resolution':size,'timing_analysis':str(path),'full_control_mean_gpu_ms':full,
            'rows':rows,'source_measure':'GPU source-to-final-composite interval'}


def draw(data: dict[str,dict]) -> None:
    W,H=1940,1160
    im=Image.new('RGB',(W,H),'#f8fafc')
    d=ImageDraw.Draw(im)
    regular='/System/Library/Fonts/Supplemental/Arial.ttf'
    bold='/System/Library/Fonts/Supplemental/Arial Bold.ttf'
    font=lambda n,b=False:ImageFont.truetype(bold if b else regular,n)
    ink,muted,grid='#233142','#607083','#dce3eb'
    d.text((68,39),'Final profile performance',font=font(37,True),fill=ink)
    d.text((68,93),'Measured GPU source-to-final-composite time · 240-frame real-input runs · lower is faster',font=font(19),fill=muted)
    d.text((68,126),'Each resolution uses its own full-a/full-b mean as the 100% reference.',font=font(18),fill=muted)
    d.line((68,169,1872,169),fill=grid,width=2)
    colors={'reference':'#3d4d60','anchor':'#8391a2','accepted':'#087d82'}
    for panel,size in enumerate(SIZES):
        x0=78+panel*945
        panel_data=data[size]
        rows=panel_data['rows']
        d.text((x0,190),'1080p' if panel==0 else '4K',font=font(29,True),fill=ink)
        d.text((x0,229),f"Full control mean: {panel_data['full_control_mean_gpu_ms']:.2f} ms",font=font(17),fill=muted)
        chart_left=x0+190; chart_right=x0+800
        top=295;row_h=79
        valid=[r['mean_gpu_ms'] for r in rows if r['mean_gpu_ms'] is not None]
        max_ms=max(valid)*1.10 if valid else 1
        for tick in (0,.25,.5,.75,1):
            x=chart_left+tick*(chart_right-chart_left)
            d.line((x,top-15,x,top+row_h*len(rows)),fill=grid,width=1)
            d.text((x-12,top+row_h*len(rows)+15),f'{max_ms*tick:.0f}',font=font(15),fill=muted)
        for i,row in enumerate(rows):
            y=top+i*row_h
            d.text((x0,y+10),row['label'],font=font(21,row['status']=='accepted'),fill=colors[row['status']])
            d.text((x0,y+40),'/'.join(f'{v:g}' for v in row['levels_percent']),font=font(15),fill=muted)
            ms=row['mean_gpu_ms']
            if ms is None:
                d.text((chart_left+10,y+16),'pending in run',font=font(17),fill=muted)
                continue
            bar_w=(ms/max_ms)*(chart_right-chart_left)
            d.rounded_rectangle((chart_left,y+6,chart_left+bar_w,y+38),radius=6,fill=colors[row['status']])
            d.text((chart_left+8,y+10),f'{ms:.2f} ms',font=font(17,True),fill='white')
            d.text((chart_left+bar_w+9,y+13),f"{row['percent_of_same_run_full']:.1f}%",font=font(15),fill=ink)
        d.text((chart_left+220,top+row_h*len(rows)+48),'GPU time (ms)',font=font(18),fill=ink)
    d.line((68,943,1872,943),fill=grid,width=2)
    d.text((68,963),'4K halo texture memory · five RGBA16F layers · Σ ceil(3840r) × ceil(2160r) × 8 bytes',font=font(19,True),fill=ink)
    mem=[]
    for label in ('Full','Half','Uniform 75','Uniform 87.5','P2','P3','F'):
        row=next(r for r in data['3840x2160']['rows'] if r['label']==label)
        mem.append(f"{label} {row['halo_texture_mib']:.1f} MiB")
    d.text((68,998),'   ·   '.join(mem[:4]),font=font(17),fill=muted)
    d.text((68,1028),'   ·   '.join(mem[4:]),font=font(17),fill='#087d82')
    d.text((68,1071),'P2/P3: stills accepted, motion pending. F: stills and motion accepted. P1 rejected. MSE is not perceptual quality.',font=font(17),fill=muted)
    im.save(ROOT/'final-frontier-performance.png',optimize=True)


def main() -> int:
    data={size:load_size(size) for size in SIZES}
    missing=[size for size,result in data.items() if result is None]
    if missing:
        print('Final-frontier timing reports pending: '+', '.join(missing))
        for label,levels,status,_ in PROFILES:
            print(f'{label:14} {status:9} 4K halo {halo_mib("3840x2160",levels):.1f} MiB')
        return 2
    summary={'source':'same-run measured means from final-frontier analysis JSONs',
             'image_mse_note':'Not plotted: no matched 4K captures and no complete same-implementation image set yet. RGB MSE is not perceptual quality.',
             'accepted_in_stills_and_motion':['P2','P3','F'],'rejected':['P1'],
             'halo_memory_formula':'sum over five layers of ceil(width*r)*ceil(height*r)*8 bytes, RGBA16F',
             'resolutions':data}
    (ROOT/'final-frontier-performance.json').write_text(json.dumps(summary,indent=2)+'\n')
    draw(data)
    for size,report in data.items():
        print(size)
        for row in report['rows']:
            timing='pending' if row['mean_gpu_ms'] is None else f"{row['mean_gpu_ms']:.2f} ms ({row['percent_of_same_run_full']:.1f}% full)"
            print(f"  {row['label']:14} {timing}")
    print('Wrote final-frontier-performance.json and .png')
    return 0

if __name__=='__main__':
    raise SystemExit(main())
