"""Attach bounded one-off probes to a clean owner-managed baseline or candidate worktree.
Run from the target worktree; remove probes before committing production changes.
"""
import pathlib, subprocess, sys
root=pathlib.Path.cwd(); evidence=pathlib.Path(__file__).resolve().parent
scratch=pathlib.Path('/tmp/realfft-adoption')
# Reuse the evaluation's pinned generators and coefficient helper, not copied FFTs.
setup=evidence.parent/'fft-backend-evaluation/setup.py'
subprocess.run([sys.executable,str(setup),str(root),str(scratch/'evaluation')],check=True)
bench=(evidence.parent/'fft-backend-evaluation/bench.rs').read_text()
for module in ['ballistics','golden_audio']:
 bench=bench.replace(f'mod {module};',f'#[path="{scratch}/evaluation/baseline/src/{module}.rs"]\nmod {module};')
bench=bench.replace('mod spectrogram;','use harmonigraph_core::spectrogram;')
analyzer='harmonigraph_analysis::ChannelBank' if (root/'crates/harmonigraph-analysis').exists() else 'harmonigraph_core::spectrum::ChannelBank'
bench=bench.replace('mod spectrum;\nuse spectrum::{ChannelBank, SPECTRUM_BINS};',f'use {analyzer};\nuse harmonigraph_core::spectrum::SPECTRUM_BINS;')
bench=bench.replace('    println!("n,tapers,input,dft_peak_scaled_error,dft_normalized_absolute_error");','')
bench=bench.replace('                let (scaled, abs) = spectrum::reference_error(&b);\n                println!("{n},{k},{kind},{scaled:.9e},{abs:.9e}");','')
p=root/'crates/harmonigraph-ui/examples/adoption_analyzer.rs';p.parent.mkdir(exist_ok=True);p.write_text(bench)
p=root/'crates/harmonigraph-ui/src/panes/spectral_fold.rs';text=p.read_text()
assert 'adoption_gate_histories' not in text and 'realfft-adoption/gates.rs' not in text
end=text.rfind('\n}')
p.write_text(text[:end]+f'\n    include!("{evidence}/gates.rs");\n'+text[end:])
p=root/'crates/harmonigraph-offline/src/main.rs';text=p.read_text();assert 'mod adoption_evidence' not in text
p.write_text(text+f'\n#[cfg(test)]\n#[path="{evidence}/fixtures.rs"]\nmod adoption_evidence;\n')
