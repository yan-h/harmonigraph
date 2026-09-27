"""Build the private real-take and flat controls for the scratch GPU harness."""
from pathlib import Path
import shutil
import sys
import numpy as np

repo = Path(__file__).resolve().parents[4]
out = Path('/private/tmp/stars-gather-blur')
kit = out / 'kit'
shutil.copytree(repo / '.claude/skills/look-prototype/kit', kit, dirs_exist_ok=True)
sys.path.insert(0, str(kit))
import pane
import spectro

light, _ = pane.pane_light(960, 1024)
np.clip(np.round(light[::-1].T * 255), 0, 255).astype(np.uint8).tofile(out / 'take-levels.u8')
np.full((960, 1024), 128, np.uint8).tofile(out / 'flat-levels.u8')
colour = spectro.to_srgb8(spectro.palette(np.linspace(0, 1, 4096, dtype=np.float32)))
np.column_stack([colour, np.full(4096, 255, np.uint8)]).astype(np.uint8).tofile(out / 'palette.rgba')
