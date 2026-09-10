"""Run fixed full offline frames; timings are collected separately with no builds."""
import pathlib, subprocess, sys
root = pathlib.Path('/tmp/realfft-adoption')
variant = sys.argv[1]
binary = root / variant / 'harmonigraph-offline'
for name in ['default-fold', 'default-spectrum', 'largest-fold', 'largest-spectrum']:
    args = [str(binary), str(root/'fixtures'/f'{name}.take'), '--audio', str(root/'fixtures/audio.wav'), '--align', '0', '--out', str(root/variant/f'{name}.rgba'), '--size', '640x400', '--fps', '60', '--start', '0', '--end', '3.2']
    with open(root/variant/f'{name}-render.log','w') as log:
        subprocess.run(args, stdout=log, stderr=log, check=True)
