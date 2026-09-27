#!/usr/bin/env python3
"""Produce a scratch compute shader from production's unchanged f32 response.

Only reads the repository. No compilation, device creation or execution.
The generated full module reuses production uniform layout and star_texel.
"""
from pathlib import Path
import sys

root = Path(sys.argv[1])
out = Path(__file__).parent
shader_dir = root / "crates/harmonigraph-render/src/shaders"
source = (shader_dir / "atmosphere_geometry.wgsl").read_text() + "\n" + (shader_dir / "spectrogram.wgsl").read_text()
start = source.index("fn star_texel(")
body = source.index("    if t.w == 0u", start)
end = source.index("\n}\n", body) + 2
record = "fn compute_star_record(s: StarSlice, f: vec2<f32>, t: vec4<u32>) -> vec4<f32> {\n    let halo = true;\n" + source[body:end]
decoded = record.replace("compute_star_record(s: StarSlice, f: vec2<f32>, t: vec4<u32>)", "compute_star_decoded(s: StarSlice, f: vec2<f32>, t: ComputeStar)")
replacements = {
    "t.w == 0u": "t.active == 0u",
    "vec2<f32>(bitcast<f32>(t.x), bitcast<f32>(t.y))": "t.centre",
    "vec3<f32>(vec3<u32>(t.z >> 20u, t.z >> 10u, t.z) & vec3<u32>(1023u)) / 1023.0": "t.colour",
    "unpack2x16float(t.w)": "t.shape",
}
for old, new in replacements.items():
    assert decoded.count(old) == 1, f"star response changed: {old}"
    decoded = decoded.replace(old, new)
tail = (out / "compute_tail.wgsl").read_text()
(out / "compute.wgsl").write_text(source + "\n" + record + "\n" + decoded + "\n" + tail)
print(out / "compute.wgsl")
