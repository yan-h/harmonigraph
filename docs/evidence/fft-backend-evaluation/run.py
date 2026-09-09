"""Run only after the coordinator grants a quiet machine window.

Usage: python3 run.py SCRATCH_ROOT RESULTS_DIR [bench|build|verify]
Build timings disable compiler caches, but retain downloaded crate sources.
"""
import os
import pathlib
import subprocess
import sys
import time

root, results = map(pathlib.Path, sys.argv[1:3])
mode = sys.argv[3]
results.mkdir(parents=True, exist_ok=True)
def binary(name):
    return root / name / "target/release" / f"fft-evaluation-{name}"

if mode == "bench":
    for pair in range(7):
        order = ["baseline", "candidate"] if pair % 2 == 0 else ["candidate", "baseline"]
        for name in order:
            label = f"pair-{pair}-{name}"
            with (results / f"{label}.processes").open("w") as f:
                subprocess.run(["ps", "-axo", "pid,pcpu,comm"], stdout=f, check=True)
            with (results / f"{label}.csv").open("w") as f:
                subprocess.run([str(binary(name))], stdout=f, check=True)
elif mode == "build":
    with (results / "build.csv").open("w") as f:
        f.write("pair,backend,seconds,binary_bytes\n")
        for pair in range(3):
            order = ["baseline", "candidate"] if pair % 2 == 0 else ["candidate", "baseline"]
            for name in order:
                target = root / f"clean-{pair}-{name}"
                assert not target.exists(), f"clean build target exists: {target}"
                env = dict(os.environ, RUSTC_WRAPPER="", CARGO_TARGET_DIR=str(target))
                start = time.monotonic()
                with (results / f"build-{pair}-{name}.log").open("w") as log:
                    subprocess.run(["cargo", "build", "--release", "--locked", "--offline"],
                        cwd=root / name, env=env, stdout=log, stderr=log, check=True)
                elapsed = time.monotonic() - start
                size = (target / "release" / f"fft-evaluation-{name}").stat().st_size
                f.write(f"{pair},{name},{elapsed:.3f},{size}\n")
                f.flush()
elif mode == "verify":
    for name in ["baseline", "candidate"]:
        with (results / f"{name}-dft.csv").open("w") as f:
            subprocess.run([str(binary(name)), "dump", str(results / f"{name}.bin")], stdout=f, check=True)
        with (results / f"{name}-tests.log").open("w") as f:
            subprocess.run(["cargo", "test", "--release", "--locked", "--offline"],
                cwd=root / name, stdout=f, stderr=f, check=True)
    with (results / "numerical.csv").open("w") as f:
        subprocess.run([sys.executable, str(pathlib.Path(__file__).with_name("compare.py")), str(results)],
            stdout=f, check=True)
else:
    raise SystemExit(f"unknown mode: {mode}")
