"""Materialize the preserved round-two shader variants without editing source."""
from pathlib import Path
import argparse
import hashlib
import math
import struct
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, default=Path("/private/tmp/stars-round2"))
args = parser.parse_args()
evidence = Path(__file__).resolve().parent
repo = evidence.parents[3]
source = repo / "crates/harmonigraph-render/src/shaders/spectrogram.wgsl"
expected = "ffd15f4423c9c355d7e98c64a27262a56f1e1415227e64cd9fd180a569a300a4"
if hashlib.sha256(source.read_bytes()).hexdigest() != expected:
    raise SystemExit("Shader differs from the measured base; use commit 3a3846fd plus this evidence.")
args.output.mkdir(parents=True, exist_ok=True)
(args.output / "base.wgsl").write_bytes(source.read_bytes())
for name in ["cleanup", "lookup", "lookup_zero", "combined", "no_falloff", "one_read"]:
    subprocess.run(["patch", "--batch", str(source), "-o", str(args.output / (name + ".wgsl")), "-i", str(evidence / (name + ".patch"))], check=True)
data = bytearray()
for i in range(4096):
    u = 32.0 * i / 4095.0
    data.extend(struct.pack("<ee", math.exp(-0.5 * u * u), math.exp(-0.4 * u)))
(args.output / "falloff.rg16").write_bytes(data)
print("Prepared", args.output)
