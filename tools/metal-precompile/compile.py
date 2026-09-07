#!/usr/bin/env python3
"""Compile the probe's exported MSL, or verify a downloaded artifact's hashes."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--verify", action="store_true")
    args = parser.parse_args()
    directory = args.directory.resolve()
    manifest = directory / "manifest.json"
    names = [f"{entry}.{ext}" for entry in ("vs_cell", "fs_blur_x")
             for ext in ("metal", "entry", "metallib")]
    if args.verify:
        saved = json.loads(manifest.read_text())
        assert set(saved["sha256"]) == set(names), "incomplete artifact"
        for name in names:
            assert digest(directory / name) == saved["sha256"][name], f"changed artifact: {name}"
        print("All exported sources, entry names and compiled libraries match the artifact manifest.")
        return

    flags = ["-std=metal3.2", "-mmacosx-version-min=15.0", "-ffast-math", "-fpreserve-invariance"]
    compiler = subprocess.check_output(["xcrun", "--sdk", "macosx", "metal", "--version"], text=True)
    for entry in ("vs_cell", "fs_blur_x"):
        air = directory / f"{entry}.air"
        subprocess.run(["xcrun", "--sdk", "macosx", "metal", *flags,
                        "-c", str(directory / f"{entry}.metal"), "-o", str(air)], check=True)
        subprocess.run(["xcrun", "--sdk", "macosx", "metallib", str(air),
                        "-o", str(directory / f"{entry}.metallib")], check=True)
        air.unlink()
    manifest.write_text(json.dumps({"compiler": compiler, "flags": flags,
                                   "sha256": {name: digest(directory / name) for name in names}}, indent=2) + "\n")
    print(compiler.strip())
    print(f"Compiled libraries and manifest: {directory}")


if __name__ == "__main__":
    main()
