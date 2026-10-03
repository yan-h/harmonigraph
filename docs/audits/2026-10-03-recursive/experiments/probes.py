"""Apply/restore test-only probes in the dedicated audit worktree.

Refuses other source changes. Does not modify the main checkout.
"""
from pathlib import Path
import subprocess,sys,hashlib,json
here=Path(__file__).resolve().parent; root=here.parents[3]
base='ad1c6e6b9474dd80098703ae15cac241bba0eb41'
mapping={'hub_probe.rs':'crates/harmonigraph-plugin/src/tuning/hub.rs','naming_probe.rs':'crates/harmonigraph-ui/src/panes/spectral/names.rs','targets_probe.rs':'crates/harmonigraph-render/src/spectrogram/tests/star_split.rs','nonfinite_probe.rs':'crates/harmonigraph-ui/src/tests/spectrum.rs'}
assert (root/'AGENTS.md').exists()
for probe,source in mapping.items():
    original=subprocess.check_output(['git','show',f'{base}:{source}'],cwd=root)
    expected=original+b'\n'+(here/probe).read_bytes()
    path=root/source
    if sys.argv[1]=='apply':
        if path.read_bytes()!=original: raise SystemExit('Source differs; refusing: '+source)
        path.write_bytes(expected)
    elif sys.argv[1]=='restore':
        if path.read_bytes()!=expected: raise SystemExit('Probe source differs; refusing: '+source)
        path.write_bytes(original)
    else:raise SystemExit('choose apply or restore')
if sys.argv[1]=='apply':
    (here/'test-probes.patch').write_bytes(subprocess.check_output(['git','diff','--',*mapping.values()],cwd=root))
print(sys.argv[1], '4 test-only probes')
