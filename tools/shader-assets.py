#!/usr/bin/env python3
"""Generate, validate or import the embedded production Metal shader corpus."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parent.parent
EMBEDDED = ROOT / "crates/harmonigraph-metal-assets/assets"
FLAGS = ["-std=metal3.2", "-mmacosx-version-min=15.0", "-ffast-math", "-fpreserve-invariance"]


def verify(directory):
    manifest = json.loads((directory / "manifest.json").read_text())
    assert manifest["schema"] == 1
    files = {p.name for p in directory.iterdir() if p.name != "manifest.json"}
    assert files and files == set(manifest["sha256"]), "artifact file set changed"
    for name, digest in manifest["sha256"].items():
        assert Path(name).name == name, "artifact filenames must be simple"
        assert hashlib.sha256((directory / name).read_bytes()).hexdigest() == digest, name
    sources = list(directory.glob("*.metal"))
    assert len(files) == 3 * len(sources), "unpaired artifacts"
    for source in sources:
        assert source.with_suffix(".options").is_file()
        assert source.with_suffix(".metallib").is_file()
    print(f"Verified {len(sources)} libraries", flush=True)


def compile_assets(directory):
    compiler = subprocess.check_output(["xcrun", "--sdk", "macosx", "metal", "--version"], text=True)
    sources = sorted(directory.glob("*.metal"))
    assert sources, "catalog exported no sources"
    for source in sources:
        settings = dict(line.split("=", 1) for line in source.with_suffix(".options").read_text().splitlines())
        assert settings == {
            "schema": "1", "wgpu-hal": "29.0.4", "language": "196610",
            "fast_math": "true", "invariance": "true", "math_mode": "2", "math_functions": "0",
        }, f"unsupported compiler options: {settings}"
        air = source.with_suffix(".air")
        subprocess.run(["xcrun", "--sdk", "macosx", "metal", *FLAGS, "-c", str(source), "-o", str(air)], check=True)
        subprocess.run(["xcrun", "--sdk", "macosx", "metallib", str(air), "-o", str(source.with_suffix(".metallib"))], check=True)
        air.unlink()
    files = [source.with_suffix(extension) for source in sources for extension in (".metal", ".options", ".metallib")]
    manifest = {"schema": 1, "compiler": compiler, "flags": FLAGS,
                "sha256": {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in files}}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    verify(directory)


def build(package, logs):
    command = ["cargo", "test", "--locked", "--release", "-p", package,
               "--no-run", "--message-format=json"]
    if package == "harmonigraph-render":
        command += ["--lib", "--features", "shader-assets-tools"]
    result = subprocess.run(command, cwd=ROOT, text=True, stdout=subprocess.PIPE, check=True)
    (logs / f"{package}-build.jsonl").write_text(result.stdout)
    binaries = [item["executable"] for line in result.stdout.splitlines()
                if (item := json.loads(line)).get("executable")]
    binary, = binaries
    return binary


def run(binary, arguments, mode, name, logs, export=None):
    environment = dict(os.environ, HARMONIGRAPH_REQUIRE_GPU="1", WGPU_BACKEND="metal",
                       HARMONIGRAPH_SHADER_ASSETS=mode)
    environment.pop("HARMONIGRAPH_BLESS", None)
    if export is not None:
        environment["HARMONIGRAPH_SHADER_EXPORT_DIR"] = str(export)
    result = subprocess.run([binary, *arguments, "--nocapture", "--test-threads=1"], cwd=ROOT,
                            env=environment, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=300)
    (logs / f"{name}.log").write_text(result.stdout)
    assert result.returncode == 0, f"{name} failed; see {logs / (name + '.log')}"
    assert "test result: ok." in result.stdout and "0 passed;" not in result.stdout
    assert "missing Metal asset" not in result.stdout, "a test caught a strict coverage error"
    print(f"Passed {name}", flush=True)


def check(logs):
    verify(EMBEDDED)
    renderer = build("harmonigraph-render", logs)
    run(renderer, ["production_metal_asset_catalog", "--ignored"], "strict", "strict-catalog", logs)
    run(renderer, ["golden"], "strict", "strict-renderer-goldens", logs)
    offline = build("harmonigraph-offline", logs)
    run(offline, ["golden"], "strict", "strict-offline-goldens", logs)


def install(directory):
    verify(directory)
    assert directory.resolve() != EMBEDDED.resolve()
    shutil.rmtree(EMBEDDED)
    shutil.copytree(directory, EMBEDDED)


def generate(directory, logs):
    assert not directory.exists(), "export requires a fresh directory"
    directory.mkdir(parents=True)
    renderer = build("harmonigraph-render", logs)
    run(renderer, ["production_metal_asset_catalog", "--ignored"], "export", "export-catalog", logs, directory)
    compile_assets(directory)
    # Validate the actual embedded path, not a runtime sidecar override. Restore
    # the caller's corpus even on failure; only the explicit import mutates it.
    with tempfile.TemporaryDirectory(prefix="harmonigraph-assets-") as backup:
        saved = Path(backup) / "assets"
        shutil.copytree(EMBEDDED, saved)
        try:
            install(directory)
            check(logs)
        finally:
            shutil.rmtree(EMBEDDED)
            shutil.copytree(saved, EMBEDDED)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("generate", "check", "verify", "import"))
    parser.add_argument("directory", nargs="?", type=Path)
    parser.add_argument("--logs", type=Path, default=ROOT / "target/shader-asset-logs")
    args = parser.parse_args()
    args.logs = args.logs.resolve()
    args.logs.mkdir(parents=True, exist_ok=True)
    directory = args.directory.resolve() if args.directory else EMBEDDED
    if args.action == "generate":
        assert args.directory is not None
        generate(directory, args.logs)
    elif args.action == "check":
        check(args.logs)
    elif args.action == "import":
        assert args.directory is not None
        install(directory)
    else:
        verify(directory)


if __name__ == "__main__":
    main()
