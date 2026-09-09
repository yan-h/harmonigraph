"""Materialize isolated baseline/candidate crates from pinned production source."""
import pathlib
import subprocess
import sys

BASE = "0c32443a5f23f59c5a4ad7fee21ec49628f8e436"
here = pathlib.Path(__file__).resolve().parent
repo, out = map(pathlib.Path, sys.argv[1:])
source = subprocess.check_output(
    ["git", "-C", str(repo), "show", f"{BASE}:crates/harmonigraph-core/src/spectrum.rs"], text=True
)
for name in ["baseline", "candidate"]:
    root = out / name
    (root / "src").mkdir(parents=True, exist_ok=True)
    (root / "src/spectrum.rs").write_text(source)
    if name == "candidate":
        subprocess.run(["patch", "-p0", "-i", str(here / "candidate.patch")], cwd=root, check=True)
    with (root / "src/spectrum.rs").open("a") as f:
        f.write((here / "probe.rs").read_text())
    (root / "src/main.rs").write_text((here / "bench.rs").read_text())
    for module in ["spectrogram"]:
        content = subprocess.check_output(["git", "-C", str(repo), "show",
            f"{BASE}:crates/harmonigraph-core/src/{module}.rs"], text=True)
        (root / f"src/{module}.rs").write_text(content)
    golden = subprocess.check_output(["git", "-C", str(repo), "show",
        f"{BASE}:crates/harmonigraph-offline/src/golden.rs"], text=True)
    golden = golden[golden.index("struct Tone {"):golden.index("/// One golden frame:")]
    golden = golden.replace("fn probe_audio() -> Audio", "pub fn probe_audio() -> Vec<f32>")
    golden = golden.replace("Audio::from_samples(SAMPLE_RATE, samples, 1)", "samples")
    (root / "src/golden_audio.rs").write_text(
        "const SECONDS: f64 = 2.0;\nconst SAMPLE_RATE: f32 = 48_000.0;\n" + golden)
    ui = subprocess.check_output(["git", "-C", str(repo), "show",
        f"{BASE}:crates/harmonigraph-ui/src/spectrum.rs"], text=True)
    start = ui.index("pub(crate) fn hop_alpha(")
    (root / "src/ballistics.rs").write_text(ui[start:ui.index("\n}", start) + 2])
    deps = '\nrealfft = "=3.5.0"\nrustfft = "=6.4.1"\n' if name == "candidate" else ""
    (root / "Cargo.toml").write_text(f'''[package]
name = "fft-evaluation-{name}"
version = "0.0.0"
edition = "2021"
publish = false
[workspace]
[dependencies]
{deps}
[profile.release]
lto = false
strip = "debuginfo"
''')
    lock = here / f"{name}.lock"
    if lock.exists():
        (root / "Cargo.lock").write_bytes(lock.read_bytes())
    (root / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.92"\n')
print(out)
