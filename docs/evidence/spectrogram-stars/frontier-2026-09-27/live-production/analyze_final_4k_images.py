#!/usr/bin/env python3
"""CPU-only same-kind RGB comparisons for frozen final 4K and 1080 captures."""
from __future__ import annotations

import csv
import hashlib
import json
from pathlib import Path

import numpy as np

ROOT=Path('/private/tmp/stars-investigation')
SCENES=('take','flat')
CASES=[
    ('Full','full-a','full-a',None),
    ('Half','half-a','half-a','images-grouped'),
    ('Uniform 75','r750','r750','images-reversed'),
    ('Uniform 87.5','r875','r875','images-reversed'),
    ('P2','full-grouped-v50_50_80_100_50','full-grouped-v50_50_80_100_50','images-grouped'),
    ('P3','full-grouped-v50_50_100_100_60','full-grouped-v50_50_100_100_60','images-grouped'),
    ('F','full-grouped-v50_50_100_100_100','full-grouped-v50_50_100_100_100','images-grouped'),
    ('Complete','full-complete',None,None),
]

def image(path:Path,w:int,h:int):
    assert path.stat().st_size==w*h*4,(path,path.stat().st_size,w*h*4)
    return np.memmap(path,dtype=np.uint8,mode='r',shape=(w*h,4))

def metrics(reference:Path,candidate:Path,w:int,h:int)->dict:
    a,b=image(reference,w,h),image(candidate,w,h)
    absolute_sum=square_sum=different_channels=different_pixels=different_rgba=max_diff=0
    for start in range(0,w*h,32768):
        end=min(start+32768,w*h)
        raw_a=a[start:end];raw_b=b[start:end]
        difference=raw_b[:,:3].astype(np.int16)-raw_a[:,:3].astype(np.int16)
        absolute=np.abs(difference).astype(np.uint16)
        absolute_sum+=int(absolute.sum(dtype=np.uint64))
        square_sum+=int(np.square(difference.astype(np.int32)).sum(dtype=np.int64))
        different_channels+=int(np.count_nonzero(difference))
        different_pixels+=int(np.count_nonzero(np.any(difference!=0,axis=1)))
        different_rgba+=int(np.count_nonzero(raw_a!=raw_b))
        max_diff=max(max_diff,int(absolute.max(initial=0)))
    n=w*h*3
    return {'rgb_mae_0_255':absolute_sum/n,'rgb_mse_byte_squared':square_sum/n,
            'max_absolute_rgb_0_255':max_diff,'different_rgb_channels':different_channels,
            'different_rgb_pixels':different_pixels,'different_rgba_bytes':different_rgba,
            'rgb_channel_count':n}

def sha256(path:Path)->str:
    return hashlib.sha256(path.read_bytes()).hexdigest()

def main():
    d4=ROOT/'images-final-4k'; rows=[]
    manifest=json.loads((d4/'manifest.json').read_text())
    assert manifest['frames']==1 and manifest['jitter']==0.5 and manifest['ppp']==4
    for label,case4,case1080,source1080 in CASES:
        for scene in SCENES:
            reference4=d4/f'full-a-{scene}.rgba'
            candidate4=d4/f'{case4}-{scene}.rgba'
            assert candidate4.exists(),candidate4
            m4=metrics(reference4,candidate4,3840,2160)
            rows.append({'resolution':'3840x2160','scene':scene,'profile':label,'case':case4,
                         'source_dir':'images-final-4k','reference':str(reference4),
                         'candidate':str(candidate4),**m4})
            if case1080 is None:continue
            source=ROOT/(source1080 or 'images-grouped')
            reference=source/f'full-a-{scene}.rgba';candidate=source/f'{case1080}-{scene}.rgba'
            if label=='Full':reference=ROOT/'images-grouped'/f'full-a-{scene}.rgba';candidate=reference
            assert candidate.exists(),candidate
            m=metrics(reference,candidate,1920,1080)
            rows.append({'resolution':'1920x1080','scene':scene,'profile':label,'case':case1080,
                         'source_dir':source1080 or 'images-grouped','reference':str(reference),
                         'candidate':str(candidate),**m})
    # The 1080 grouped reference is byte-identical to p0; older uniform 75/87.5
    # captures use their own reference and must be labeled as a different phase.
    reference_identity={scene:sha256(ROOT/'images-grouped'/f'full-a-{scene}.rgba')==sha256(ROOT/'curve-p0'/f'full-a-{scene}.rgba')
                        for scene in SCENES}
    assert all(reference_identity.values())
    document={'metric_definition':'Unsigned 8-bit final RGBA captures; MAE/MSE/max over RGB only, candidate minus same-directory full-a of same kind.',
              'not_perception':'RGB distance is a numerical diagnostic, not perceptual quality.',
              'not_performance':'These are still-image differences, not GPU timings.',
              'four_k_capture':'One frozen phase at jitter 0.5, clock 3, take and flat; no three-phase 4K model.',
              'uniform_1080_caveat':'1080 Uniform 75/87.5 images come from images-reversed and use that directory\'s own full-a; its reference differs from p0/images-grouped.',
              'grouped_1080_reference_matches_curve_p0_sha256':reference_identity,
              'four_k_manifest':manifest,'rows':rows}
    out=ROOT/'images-final-4k'/'final-image-metrics.json';out.write_text(json.dumps(document,indent=2)+'\n')
    bundle=ROOT/'evidence-bundle'/'images-final-4k';bundle.mkdir(parents=True,exist_ok=True)
    (bundle/'final-image-metrics.json').write_text(out.read_text())
    (bundle/'manifest.json').write_text((d4/'manifest.json').read_text())
    cross=ROOT/'final-image-cross-resolution.csv'
    with cross.open('w',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=['profile','scene','resolution','source_dir','rgb_mae_0_255','rgb_mse_byte_squared','max_absolute_rgb_0_255','different_rgba_bytes'])
        writer.writeheader()
        for row in rows:writer.writerow({k:row[k] for k in writer.fieldnames})
    print('Profile           1080 MAE/MSE (take,flat)        4K MAE/MSE (take,flat)')
    for label,*_ in CASES:
        def fmt(size):
            a=[r for r in rows if r['profile']==label and r['resolution']==size]
            if len(a)!=2:return '—'
            a=sorted(a,key=lambda r:SCENES.index(r['scene']))
            return ' / '.join(f"{r['rgb_mae_0_255']:.4f},{r['rgb_mse_byte_squared']:.4f}" for r in a)
        print(f'{label:16} {fmt("1920x1080"):31} {fmt("3840x2160")}')
    complete=[r for r in rows if r['profile']=='Complete']
    print('Complete-vs-full 4K quantization:',[(r['scene'],r['max_absolute_rgb_0_255'],r['different_rgba_bytes']) for r in complete])
    print(f'Wrote {out}, {cross}, and compact evidence-bundle copies')

if __name__=='__main__':main()
