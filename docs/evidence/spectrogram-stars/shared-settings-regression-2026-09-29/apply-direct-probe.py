"""Temporary spectrogram-only shader probe; run from the pinned worktree root."""
from pathlib import Path
shared = Path('crates/harmonigraph-render/src/shaders/stars.wgsl')
probe = shared.with_name('stars-direct-probe.wgsl')
consumer = Path('crates/harmonigraph-render/src/spectrogram.rs')
assert not probe.exists(), probe
source = shared.read_text()
assert 'star_settings().' in source
text = consumer.read_text()
needle = 'include_str!("shaders/stars.wgsl")'
assert text.count(needle) == 1
probe.write_text(source.replace('star_settings().', 'cloud.'))
consumer.write_text(text.replace(needle, 'include_str!("shaders/stars-direct-probe.wgsl")'))
print('Replaced', source.count('star_settings().'), 'settings-struct accesses in spectrogram-only copy')
