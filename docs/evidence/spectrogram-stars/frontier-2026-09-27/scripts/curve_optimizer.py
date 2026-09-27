#!/usr/bin/env python3
"""Linearized five-layer resolution search from matched 1920x1080 RGBA stills.

For each available complete curve-p0, curve-p1, ... directory, measure signed RGB
changes from full-a for both take and flat. Average their Gram matrices by
scene and phase, then predict RGB MSE of combined profiles as sum of one-layer changes.
This is a linearized image-space
approximation; actual combined renders must validate selected profiles.

Run: python3 curve_optimizer.py [--root /private/tmp/stars-investigation]
Requires NumPy and Pillow. No GPU work. Outputs curve-proposals.{json,csv,png}.
"""
from __future__ import annotations

import argparse
import csv
import itertools
import json
import math
import re
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

WIDTH, HEIGHT = 1920, 1080
PIXELS = WIDTH * HEIGHT
CHANNELS = 3 * PIXELS
LEVELS = (50, 60, 70, 80, 90, 100)
MEASURED = LEVELS[:-1]
LAYERS = range(5)
CHUNK_PIXELS = 16384


def path_for(directory: Path, layer: int, percent: int, scene: str) -> Path:
    return directory / f'full-l{layer}r{percent}-{scene}.rgba'


def ensure_rgba(path: Path) -> np.memmap:
    if path.stat().st_size != PIXELS * 4:
        raise ValueError(f'{path}: expected {PIXELS * 4} RGBA bytes, got {path.stat().st_size}')
    return np.memmap(path, dtype=np.uint8, mode='r', shape=(PIXELS, 4))


def phase_gram(directory: Path, scene: str) -> np.ndarray:
    reference = ensure_rgba(directory / f'full-a-{scene}.rgba')
    paths = [path_for(directory, k, r, scene) for k in LAYERS for r in MEASURED]
    variants = [ensure_rgba(path) for path in paths]
    gram = np.zeros((len(paths), len(paths)), dtype=np.float64)
    for start in range(0, PIXELS, CHUNK_PIXELS):
        end = min(start + CHUNK_PIXELS, PIXELS)
        base = reference[start:end, :3].astype(np.float32)
        # Signed deltas retain cancellations and reinforcement across layers.
        deltas = np.stack([(im[start:end, :3].astype(np.float32) - base).reshape(-1)
                           for im in variants])
        gram += (deltas @ deltas.T).astype(np.float64)
    gram /= CHANNELS  # RGB channel mean squared error, units: byte-value squared.
    return (gram + gram.T) / 2


def profile_name(levels: tuple[int, ...]) -> str:
    return 'full-v' + '_'.join(map(str, levels))


def profile_metrics(levels: tuple[int, ...], gram: np.ndarray,
                    scene_grams: dict[str, np.ndarray]) -> dict:
    indices = [k * len(MEASURED) + MEASURED.index(r)
               for k, r in enumerate(levels) if r != 100]
    additive = float(sum(gram[i, i] for i in indices))
    predicted = float(gram[np.ix_(indices, indices)].sum()) if indices else 0.0
    if predicted < -1e-5:
        raise ValueError(f'negative predicted MSE {predicted} for {levels}')
    predicted = max(0.0, predicted)
    scene_errors = {scene: float(g[np.ix_(indices, indices)].sum()) if indices else 0.0
                    for scene, g in scene_grams.items()}
    return {
        'name': profile_name(levels), 'levels_percent': list(levels),
        'distinct_resolution_values': len(set(levels)),
        'cost_proxy_percent_of_full': sum(r * r for r in levels) / (5 * 10000) * 100,
        'predicted_rgb_mse': predicted, 'predicted_rgb_rmse': math.sqrt(predicted),
        'predicted_take_rgb_mse': max(0.0, scene_errors['take']),
        'predicted_flat_rgb_mse': max(0.0, scene_errors['flat']),
        'simple_additive_rgb_mse': additive,
        'interaction_rgb_mse': predicted - additive,
    }


def nondominated(profiles: list[dict]) -> list[dict]:
    # For a fixed cost, retain the lower-error allocation; then sweep increasing cost.
    by_cost = {}
    for p in profiles:
        key = sum(r * r for r in p['levels_percent'])
        if key not in by_cost or p['predicted_rgb_mse'] < by_cost[key]['predicted_rgb_mse']:
            by_cost[key] = p
    best_error = float('inf')
    curve = []
    for key in sorted(by_cost):
        p = by_cost[key]
        if p['predicted_rgb_mse'] < best_error - 1e-8:
            curve.append(p)
            best_error = p['predicted_rgb_mse']
    return curve


def select_spaced(curve: list[dict], count: int = 8,
                  cost_key: str = 'cost_proxy_percent_of_full') -> list[dict]:
    if len(curve) <= count:
        return curve
    costs = np.array([p[cost_key] for p in curve])
    targets = np.linspace(costs[0], costs[-1], count)
    chosen = {int(np.argmin(abs(costs - target))) for target in targets}
    while len(chosen) < count:
        # Fill the largest cost gap until all requested profiles are distinct.
        available = [i for i in range(len(curve)) if i not in chosen]
        i = max(available, key=lambda j: min(abs(costs[j] - costs[k]) for k in chosen))
        chosen.add(i)
    return [curve[i] for i in sorted(chosen)]


def fit_empirical_cost(root: Path, size: str, profiles: list[dict]) -> tuple[dict | None, list[str]]:
    """Fit first-order GPU savings from five complete single-layer timing runs."""
    measurements = []
    missing = []
    for layer in LAYERS:
        path = root / 'timings' / f'layer-cost-{layer}' / f'{size}.analysis.json'
        if not path.exists():
            missing.append(str(path))
            continue
        report = json.loads(path.read_text())
        cases = report['cases']
        if not {'full-a', 'full-b'} <= cases.keys():
            missing.append(f'{path}: full-a/full-b controls')
            continue
        control = (cases['full-a']['mean_gpu_ms'] + cases['full-b']['mean_gpu_ms']) / 2
        for percent in (50, 70, 90):
            name = f'full-l{layer}r{percent}'
            if name not in cases:
                missing.append(f'{path}: {name}')
                continue
            x = 1 - (percent / 100) ** 2
            observed_drop = 1 - cases[name]['mean_gpu_ms'] / control
            measurements.append({'layer': layer, 'percent': percent, 'case': name,
                                 'source': str(path), 'x_area_drop': x,
                                 'observed_normalized_gpu_drop': observed_drop})
    if missing:
        return None, missing
    coeffs = []
    for layer in LAYERS:
        rows = [m for m in measurements if m['layer'] == layer]
        numerator = sum(m['x_area_drop'] * m['observed_normalized_gpu_drop'] for m in rows)
        denominator = sum(m['x_area_drop'] ** 2 for m in rows)
        coeffs.append(max(0.0, numerator / denominator))
        for m in rows:
            m['fitted_normalized_gpu_drop'] = coeffs[-1] * m['x_area_drop']
            m['fit_residual_percentage_points'] = 100 * (
                m['observed_normalized_gpu_drop'] - m['fitted_normalized_gpu_drop'])
    # Shared surcharge for activating any reduced-resolution profile. This is
    # fitted separately; all-100 remains the exact full-control baseline.
    design = np.zeros((len(measurements), 6), dtype=np.float64)
    observed = np.array([m['observed_normalized_gpu_drop'] for m in measurements])
    for i,m in enumerate(measurements):
        design[i,m['layer']] = m['x_area_drop']
        design[i,5] = -1
    best = None
    # Nonnegative least squares over six small parameters; enumerate active sets.
    for mask in range(1,1 << 6):
        indices=[i for i in range(6) if mask & (1 << i)]
        candidate=np.zeros(6)
        candidate[indices]=np.linalg.lstsq(design[:,indices],observed,rcond=None)[0]
        if np.any(candidate < -1e-12):continue
        residual=design @ candidate-observed
        score=float(residual @ residual)
        if best is None or score < best[0]:best=(score,candidate)
    assert best is not None
    intercept_coeffs=best[1][:5].tolist()
    surcharge=float(best[1][5])
    for i,m in enumerate(measurements):
        m['shared_intercept_fitted_drop']=float(design[i] @ best[1])
        m['shared_intercept_fit_residual_percentage_points']=100*(observed[i]-m['shared_intercept_fitted_drop'])
    estimated = []
    for p in profiles:
        levels = p['levels_percent']
        cost = 100 * (1 - sum(a * (1 - (r / 100) ** 2) for a, r in zip(coeffs, levels)))
        estimated.append({**p, 'estimated_gpu_cost_percent_of_full': cost})
    # Use the fitted cost, retaining best predicted image error at tied cost.
    def frontier(items):
        by_cost = {}
        for p in items:
            key = round(p['estimated_gpu_cost_percent_of_full'], 8)
            if key not in by_cost or p['predicted_rgb_mse'] < by_cost[key]['predicted_rgb_mse']:
                by_cost[key] = p
        best = float('inf')
        result = []
        for key in sorted(by_cost):
            p = by_cost[key]
            if p['predicted_rgb_mse'] < best - 1e-8:
                result.append(p)
                best = p['predicted_rgb_mse']
        return result
    model = {
        'size': size, 'definition': 'First-order fitted normalized source-to-composite GPU cost: 100*(1-sum(a_k*(1-r_k^2))).',
        'limitations': 'Single-layer timing fit; mixed-profile GPU interactions and 4K image error are unmeasured.',
        'nonnegative_coefficients': coeffs, 'single_layer_fit_rows': measurements,
        'fit_residual_rmse_percentage_points': math.sqrt(sum(m['fit_residual_percentage_points']**2 for m in measurements)/len(measurements)),
        'profile_count': len(estimated),
        'nondominated_curve': frontier(estimated),
        'constrained_curves': {f'at_most_{n}': frontier([p for p in estimated if p['distinct_resolution_values'] <= n]) for n in (2,3)},
    }
    model['suggested_spaced_frontier_points'] = select_spaced(
        model['nondominated_curve'], 8, 'estimated_gpu_cost_percent_of_full')
    intercept_estimated=[]
    for p in profiles:
        levels=p['levels_percent']
        active=any(r != 100 for r in levels)
        cost=100*(1+(surcharge if active else 0)-sum(
            a*(1-(r/100)**2) for a,r in zip(intercept_coeffs,levels)))
        intercept_estimated.append({**p,'estimated_gpu_cost_percent_of_full':cost})
    intercept_frontier=frontier(intercept_estimated)
    model['shared_intercept_model']={
        'definition':'For any reduced profile, GPU cost = 100*(1+b-sum(a_k*(1-r_k^2))); all-100 is exactly 100.',
        'nonnegative_coefficients':intercept_coeffs,'nonnegative_surcharge_b':surcharge,
        'single_layer_fit_rmse_percentage_points':math.sqrt(best[0]/len(measurements))*100,
        'nondominated_curve':intercept_frontier,
        'constrained_curves':{f'at_most_{n}':frontier([p for p in intercept_estimated if p['distinct_resolution_values']<=n]) for n in (2,3)},
        'suggested_spaced_frontier_points':select_spaced(intercept_frontier,8,'estimated_gpu_cost_percent_of_full'),
    }
    # Benchmarks not used to fit the model. Match each observation to its own
    # run's A/B controls; the take input is a different workload from fit runs.
    known=[('allocated-area','full-v50_50_50_50_50',(50,)*5,'synthetic'),
           ('take-confirm','full-mix4',(50,50,50,100,100),'take'),
           ('take-confirm','full-mix5',(50,50,100,100,100),'take')]
    cross=[]
    for run,case,levels,input_kind in known:
        path=root/'timings'/run/f'{size}.analysis.json'
        if not path.exists():continue
        report=json.loads(path.read_text())
        if case not in report['cases'] or not {'full-a','full-b'} <= report['cases'].keys():continue
        control=(report['cases']['full-a']['mean_gpu_ms']+report['cases']['full-b']['mean_gpu_ms'])/2
        actual=100*report['cases'][case]['mean_gpu_ms']/control
        x=sum(a*(1-(r/100)**2) for a,r in zip(coeffs,levels))
        xi=sum(a*(1-(r/100)**2) for a,r in zip(intercept_coeffs,levels))
        cross.append({'run':run,'input':input_kind,'case':case,'observed_gpu_cost_percent_of_full':actual,
                      'zero_intercept_predicted_gpu_cost_percent_of_full':100*(1-x),
                      'shared_intercept_predicted_gpu_cost_percent_of_full':100*(1+surcharge-xi),
                      'shared_intercept_error_percentage_points':100*(1+surcharge-xi)-actual,
                      'note':'take is a different input from the synthetic fit' if input_kind=='take' else 'same synthetic input class'})
    model['held_out_mixed_timing_comparisons']=cross
    return model, []


def validate_actual_combined(phase: str, folder: Path, scene: str,
                             phase_scene_gram: np.ndarray) -> list[dict]:
    ref = ensure_rgba(folder / f'full-a-{scene}.rgba')
    pattern = re.compile(r'^full-v(\d+)_(\d+)_(\d+)_(\d+)_(\d+)-' + scene + r'\.rgba$')
    actuals = {}
    for path in folder.glob(f'full-v*-{scene}.rgba'):
        match = pattern.fullmatch(path.name)
        if not match:
            continue
        levels = tuple(map(int, match.groups()))
        if len(levels) == 5 and all(r in LEVELS for r in levels):
            actuals[profile_name(levels)] = (levels, path)
    aliases = {
        'half-a': (50,50,50,50,50),
        'full-mix4': (50,50,50,100,100),
        'full-mix5': (50,50,100,100,100),
    }
    for alias, levels in aliases.items():
        name = profile_name(levels)
        path = folder / f'{alias}-{scene}.rgba'
        if name not in actuals and path.exists():
            actuals[name] = (levels, path)
    results = []
    for name, (levels, path) in sorted(actuals.items()):
        actual = ensure_rgba(path)
        variant_paths = [path_for(folder,k,r,scene) for k,r in enumerate(levels) if r != 100]
        variants = [ensure_rgba(p) for p in variant_paths]
        sse, residual_sse = 0.0, 0.0
        for start in range(0, PIXELS, CHUNK_PIXELS):
            end = min(start+CHUNK_PIXELS, PIXELS)
            base = ref[start:end,:3].astype(np.float32)
            difference = actual[start:end,:3].astype(np.float32)-base
            predicted_delta = sum((v[start:end,:3].astype(np.float32)-base for v in variants),
                                  np.zeros_like(base))
            sse += float(np.square(difference,dtype=np.float32).sum(dtype=np.float64))
            residual_sse += float(np.square(difference-predicted_delta,dtype=np.float32).sum(dtype=np.float64))
        predicted = profile_metrics(levels,phase_scene_gram,{scene:phase_scene_gram,
                    'flat' if scene=='take' else 'take':phase_scene_gram})
        results.append({
            'phase':phase,'scene':scene,'name':name,'actual_file':str(path),
            'predicted_rgb_mse':predicted['predicted_rgb_mse'],
            'simple_additive_rgb_mse':predicted['simple_additive_rgb_mse'],
            'actual_rgb_mse':sse/CHANNELS,
            'linearized_image_residual_rgb_mse':residual_sse/CHANNELS,
            'predicted_minus_actual_rgb_mse':predicted['predicted_rgb_mse']-sse/CHANNELS,
        })
    return results


def draw_plot(out: Path, phases: list[str], gram: np.ndarray, all_profiles: list[dict],
              frontier: list[dict], constrained: dict[str,list[dict]],
              proposed: list[dict], anchors: list[dict], validations: list[dict]) -> None:
    W, H = 1900, 1120
    im = Image.new('RGB', (W, H), '#f8fafc')
    d = ImageDraw.Draw(im)
    fontpath = '/System/Library/Fonts/Supplemental/Arial.ttf'
    boldpath = '/System/Library/Fonts/Supplemental/Arial Bold.ttf'
    f = lambda n, b=False: ImageFont.truetype(boldpath if b else fontpath, n)
    ink, muted, grid = '#243141', '#5e6c7a', '#dfe5ec'
    d.text((65, 35), 'Linearized per-layer resolution search', font=f(35, True), fill=ink)
    d.text((65, 86), f'Mean take + flat RGB MSE from signed one-layer differences · phases: {", ".join(phases)}', font=f(19), fill=muted)
    d.text((65, 117), 'Lower cost and lower MSE are preferable as diagnostics; combined renders are still required.', font=f(18), fill=muted)

    # Left: search and Pareto frontier.
    L, T, R, B = 95, 225, 930, 925
    xmax = 102
    ymax = max(p['predicted_rgb_mse'] for p in all_profiles) * 1.05
    ymax = max(ymax, .001)
    xp = lambda v: L + v / xmax * (R - L)
    yp = lambda v: B - v / ymax * (B - T)
    d.text((L, 178), 'Predicted combined-image error vs cost proxy', font=f(23, True), fill=ink)
    for tick in range(0, 101, 20):
        x = round(xp(tick)); d.line((x,T,x,B), fill=grid)
        d.text((x-10,B+12), str(tick), font=f(15), fill=muted)
    for j in range(5):
        v = ymax*j/4; y = round(yp(v)); d.line((L,y,R,y), fill=grid)
        d.text((L-72,y-9), f'{v:.2f}', font=f(15), fill=muted)
    d.line((L,T,L,B,R,B), fill='#8492a0', width=2)
    d.text((L+220,B+49), 'Cost proxy (% full layer area)', font=f(18), fill=ink)
    for p in all_profiles:
        x,y = xp(p['cost_proxy_percent_of_full']),yp(p['predicted_rgb_mse'])
        d.ellipse((x-1,y-1,x+1,y+1),fill='#bac5d0')
    coords = [(xp(p['cost_proxy_percent_of_full']),yp(p['predicted_rgb_mse'])) for p in frontier]
    if len(coords)>1:d.line(coords,fill='#176b86',width=4)
    for key,color in [('at_most_3','#79549e'),('at_most_2','#bb6b47')]:
        c=constrained[key]
        pts=[(xp(p['cost_proxy_percent_of_full']),yp(p['predicted_rgb_mse'])) for p in c]
        if len(pts)>1:d.line(pts,fill=color,width=3)
    d.text((L+18,T+20),'All values',font=f(17,True),fill='#176b86')
    d.text((L+160,T+20),'≤3 values',font=f(17,True),fill='#79549e')
    d.text((L+310,T+20),'≤2 values',font=f(17,True),fill='#bb6b47')
    for p in proposed:
        x,y=xp(p['cost_proxy_percent_of_full']),yp(p['predicted_rgb_mse'])
        d.ellipse((x-6,y-6,x+6,y+6),fill='#db773a',outline='white',width=2)
    for p in anchors:
        x,y=xp(p['cost_proxy_percent_of_full']),yp(p['predicted_rgb_mse'])
        d.rectangle((x-5,y-5,x+5,y+5),fill='#3f6b57',outline='white',width=2)
    profile_by_name={p['name']:p for p in all_profiles}
    actual_by_name={}
    for v in validations:
        actual_by_name.setdefault(v['name'],[]).append(v['actual_rgb_mse'])
    for name,values in actual_by_name.items():
        p=profile_by_name.get(name)
        if p is None:continue
        x=xp(p['cost_proxy_percent_of_full'])
        predicted_y=yp(p['predicted_rgb_mse'])
        actual_y=yp(sum(values)/len(values))
        d.line((x,predicted_y,x,actual_y),fill='#222d3a',width=2)
        d.ellipse((x-5,actual_y-5,x+5,actual_y+5),fill='#f8fafc',outline='#222d3a',width=2)
    d.text((L+457,T+20),'○ actual mixed stills',font=f(17,True),fill='#222d3a')

    # Right: one-layer marginal error curves, clearly measured values.
    L2,T2,R2,B2=1095,225,1815,925
    diag = [float(gram[k*5+j,k*5+j]) for k in LAYERS for j in range(5)]
    ymax2=max(diag)*1.08 if diag else 1
    x2=lambda v:L2+(v-48)/54*(R2-L2)
    floor=0.001
    y2=lambda v:B2-(math.log10(v+floor)-math.log10(floor))/(math.log10(ymax2+floor)-math.log10(floor))*(B2-T2)
    d.text((L2,178),'Measured one-layer marginal error',font=f(23,True),fill=ink)
    for tick in LEVELS:
        x=round(x2(tick));d.line((x,T2,x,B2),fill=grid)
        d.text((x-11,B2+12),str(tick),font=f(15),fill=muted)
    for v in (0,.01,.1,1,5):
        if v>ymax2:continue
        y=round(y2(v));d.line((L2,y,R2,y),fill=grid)
        d.text((L2-68,y-9),f'{v:g}',font=f(15),fill=muted)
    d.line((L2,T2,L2,B2,R2,B2),fill='#8492a0',width=2)
    d.text((L2+220,B2+49),'Layer resolution (%)',font=f(18),fill=ink)
    d.text((L2-65,T2-31),'RGB MSE (byte², log scale)',font=f(17),fill=ink)
    colors=['#23649b','#d17a2f','#8a5194','#339264','#ba4d61']
    for k in LAYERS:
        values=[float(gram[k*5+j,k*5+j]) for j in range(5)]+[0.0]
        pts=[(x2(r),y2(v)) for r,v in zip(LEVELS,values)]
        d.line(pts,fill=colors[k],width=3)
        for x,y in pts:d.ellipse((x-4,y-4,x+4,y+4),fill=colors[k],outline='white')
        d.text((L2+20+k*130,T2+20),f'Layer {k}',font=f(17,True),fill=colors[k])
    d.line((65,1015,1835,1015),fill=grid,width=2)
    d.text((65,1032),'Interaction = Gram-predicted MSE − sum of isolated MSEs. Signed RGB changes can reinforce or cancel.',font=f(17),fill=muted)
    d.text((65,1061),'Cost is Σ(resolution fraction²)/5, an area proxy—not measured GPU time. RGB MSE is not perceptual quality.',font=f(17),fill=muted)
    im.save(out,optimize=True)


def main() -> int:
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root',type=Path,default=Path('/private/tmp/stars-investigation'))
    parser.add_argument('--phases',default='p0,p1,p2',help='comma-separated phases to use when complete')
    args=parser.parse_args()
    root=args.root
    complete=[]; missing={}
    for phase in [p.strip() for p in args.phases.split(',') if p.strip()]:
        folder=root/f'curve-{phase}'
        required=[folder/f'full-a-{scene}.rgba' for scene in ('take','flat')]
        required += [path_for(folder,k,r,scene) for scene in ('take','flat') for k in LAYERS for r in MEASURED]
        absent=[str(p) for p in required if not p.exists()]
        if absent:missing[phase]=absent
        else:complete.append((phase,folder))
    if not complete:
        print('No complete phase yet. Need full-a plus all 25 one-layer RGBA files for both take and flat in one curve-{phase} directory.')
        for phase, paths in missing.items():print(f'{phase}: {len(paths)} missing; first: {paths[0]}')
        return 2
    grams={phase:{scene:phase_gram(folder,scene) for scene in ('take','flat')}
           for phase,folder in complete}
    scene_grams={scene:sum(grams[phase][scene] for phase,_ in complete)/len(complete)
                 for scene in ('take','flat')}
    gram=(scene_grams['take']+scene_grams['flat'])/2
    profiles=[profile_metrics(levels,gram,scene_grams) for levels in itertools.product(LEVELS,repeat=5)]
    frontier=nondominated(profiles)
    constrained={f'at_most_{n}':nondominated([p for p in profiles if p['distinct_resolution_values']<=n])
                 for n in (2,3)}
    proposed=select_spaced(frontier,8)
    grouped_alternatives={
        p['name']:{key:min(curve,key=lambda q:abs(q['cost_proxy_percent_of_full']-p['cost_proxy_percent_of_full']))
                   for key,curve in constrained.items()}
        for p in proposed
    }
    # Include prior named choices and uniform resolutions for direct comparisons.
    anchor_specs=[('D far 3 half',(50,50,50,100,100)),('F far 2 half',(50,50,100,100,100))]
    anchor_specs += [(f'Uniform {r}',(r,)*5) for r in LEVELS]
    anchors=[{'anchor_label':label,**profile_metrics(levels,gram,scene_grams)} for label,levels in anchor_specs]
    validations=[v for phase,folder in complete for scene in ('take','flat')
                 for v in validate_actual_combined(phase,folder,scene,grams[phase][scene])]
    validation_groups={}
    for v in validations:
        validation_groups.setdefault((v['name'],v['scene']),[]).append(v)
    validation_summary=[]
    for (name,scene),rows in sorted(validation_groups.items()):
        validation_summary.append({
            'name':name,'scene':scene,'phases':[r['phase'] for r in rows],
            'phase_count':len(rows),
            'mean_predicted_rgb_mse':sum(r['predicted_rgb_mse'] for r in rows)/len(rows),
            'mean_actual_rgb_mse':sum(r['actual_rgb_mse'] for r in rows)/len(rows),
            'mean_linearized_image_residual_rgb_mse':sum(r['linearized_image_residual_rgb_mse'] for r in rows)/len(rows),
        })
    empirical_models={}
    empirical_missing={}
    for size in ('1920x1080','3840x2160'):
        model, needed=fit_empirical_cost(root,size,profiles)
        if model is not None:empirical_models[size]=model
        else:empirical_missing[size]=needed
    document={
        'method':'Linearized final-image signed RGB perturbation Gram prediction, equally averaging take and flat; actual combined renders must validate.',
        'dimensions':[WIDTH,HEIGHT], 'mse_unit':'mean squared byte RGB channel difference',
        'cost_proxy':'sum of five squared resolution fractions divided by five; not GPU time',
        'used_phases':[phase for phase,_ in complete], 'missing_phase_file_counts':{k:len(v) for k,v in missing.items()},
        'gram_matrix':gram.tolist(),'scene_gram_matrices':{k:v.tolist() for k,v in scene_grams.items()},
        'per_phase_gram_matrices':{k:{scene:g.tolist() for scene,g in v.items()} for k,v in grams.items()},
        'gram_row_labels':[f'l{k}r{r}' for k in LAYERS for r in MEASURED],
        'candidate_count':len(profiles),'nondominated_count':len(frontier),
        'nondominated_curve':frontier,'constrained_curves':constrained,
        'proposed_profiles':proposed,'grouped_alternatives_nearest_cost':grouped_alternatives,
        'anchors':anchors,
        'actual_combined_validations':validations,
        'actual_combined_validation_summary':validation_summary,
        'empirical_gpu_cost_models':empirical_models,
        'empirical_gpu_cost_missing_inputs':empirical_missing,
    }
    (root/'curve-proposals.json').write_text(json.dumps(document,indent=2)+'\n')
    with (root/'curve-proposals.csv').open('w',newline='') as f:
        writer=csv.DictWriter(f,fieldnames=['group','name','anchor_label','levels_percent','distinct_resolution_values',
            'cost_proxy_percent_of_full','predicted_rgb_mse','predicted_rgb_rmse',
            'predicted_take_rgb_mse','predicted_flat_rgb_mse','simple_additive_rgb_mse','interaction_rgb_mse'])
        writer.writeheader()
        for group,items in [('proposed',proposed),('anchor',anchors),('frontier',frontier),
                            ('at_most_2',constrained['at_most_2']),('at_most_3',constrained['at_most_3'])]:
            for p in items:
                writer.writerow({'group':group,**{k:(json.dumps(v) if isinstance(v,list) else v) for k,v in p.items()}})
    draw_plot(root/'curve-proposals.png',[p for p,_ in complete],gram,profiles,frontier,constrained,proposed,anchors,validations)
    print(f'Used {len(complete)} complete phase(s): {", ".join(p for p,_ in complete)}; {len(profiles)} candidates, {len(frontier)} nondominated, ≤2 values {len(constrained["at_most_2"])}, ≤3 values {len(constrained["at_most_3"])}.')
    print('Suggested combined renders (cost proxy %, mean/take/flat predicted RGB MSE):')
    for p in proposed:print(f"  {p['name']}: {p['cost_proxy_percent_of_full']:.1f}%, {p['predicted_rgb_mse']:.4f}/{p['predicted_take_rgb_mse']:.4f}/{p['predicted_flat_rgb_mse']:.4f}")
    print('Grouped alternatives nearest proposed cost (2 values / 3 values):')
    for p in proposed[1:-1]:
        a=grouped_alternatives[p['name']]
        print(f"  {p['name']}: {a['at_most_2']['name']} ({a['at_most_2']['cost_proxy_percent_of_full']:.1f}%, {a['at_most_2']['predicted_rgb_mse']:.4f}) / {a['at_most_3']['name']} ({a['at_most_3']['cost_proxy_percent_of_full']:.1f}%, {a['at_most_3']['predicted_rgb_mse']:.4f})")
    if validations:
        print('Actual combined render validation (phase scene profile: predicted → actual MSE; image residual MSE):')
        for v in validations:print(f"  {v['phase']} {v['scene']} {v['name']}: {v['predicted_rgb_mse']:.4f} → {v['actual_rgb_mse']:.4f}; residual {v['linearized_image_residual_rgb_mse']:.4f}")
    for size,model in empirical_models.items():
        print(f"Empirical cost {size}: coefficients {[round(a,5) for a in model['nonnegative_coefficients']]}; fit RMSE {model['fit_residual_rmse_percentage_points']:.3f} percentage points; frontier {len(model['nondominated_curve'])}.")
        intercept=model['shared_intercept_model']
        print(f"  Shared-intercept fit: a={[round(a,5) for a in intercept['nonnegative_coefficients']]}, b={intercept['nonnegative_surcharge_b']:.5f}, RMSE {intercept['single_layer_fit_rmse_percentage_points']:.3f} pp.")
        for row in model['held_out_mixed_timing_comparisons']:
            print(f"  Held out {row['run']} {row['case']}: observed {row['observed_gpu_cost_percent_of_full']:.1f}%, zero-intercept {row['zero_intercept_predicted_gpu_cost_percent_of_full']:.1f}%, shared-intercept {row['shared_intercept_predicted_gpu_cost_percent_of_full']:.1f}% ({row['input']}).")
        for p in intercept['suggested_spaced_frontier_points']:
            print(f"  {p['name']}: shared-intercept GPU {p['estimated_gpu_cost_percent_of_full']:.1f}% of full, predicted 1080 RGB MSE {p['predicted_rgb_mse']:.4f}")
    for size,needed in empirical_missing.items():
        print(f'Empirical cost {size} pending: {len(needed)} missing inputs.')
    print('Outputs: curve-proposals.json, curve-proposals.csv, curve-proposals.png')
    return 0

if __name__=='__main__':
    raise SystemExit(main())
