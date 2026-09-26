"""Materialize split-layer shader variants without editing production source."""
from pathlib import Path
import argparse
import hashlib
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, default=Path("/private/tmp/stars-round3"))
args = parser.parse_args()
evidence = Path(__file__).resolve().parent
repo = evidence.parents[3]
source = repo / "crates/harmonigraph-render/src/shaders/spectrogram.wgsl"
expected = "ffd15f4423c9c355d7e98c64a27262a56f1e1415227e64cd9fd180a569a300a4"
if hashlib.sha256(source.read_bytes()).hexdigest() != expected:
    raise SystemExit("Shader differs from measured base; use commit 3a3846fd plus this evidence.")
args.output.mkdir(parents=True, exist_ok=True)
for name in ["base", "dust100", "dust075"]:
    subprocess.run(["patch", "--batch", str(source), "-o", str(args.output / (name + ".wgsl")), "-i", str(evidence / (name + ".patch"))], check=True)
print("Prepared", args.output)
