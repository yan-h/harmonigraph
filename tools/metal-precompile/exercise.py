#!/usr/bin/env python3
"""Build and exercise the copied-backend generated-asset experiment on macOS."""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess

import backend


def build(package, config, directory):
    command = ["cargo", "test", "--release", "--config", str(config), "-p", package,
               "--no-run", "--message-format=json"]
    if package == "harmonigraph-render":
        command.append("--lib")
    # Cargo removes the registry source/checksum for the temporary path patch.
    # Restore the caller's exact lockfile, including any pre-existing edits.
    lockfile = Path("Cargo.lock")
    original = lockfile.read_bytes()
    try:
        result = subprocess.run(command, text=True, stdout=subprocess.PIPE, check=True)
    finally:
        lockfile.write_bytes(original)
    (directory / f"{package}-build.jsonl").write_text(result.stdout)
    binaries = [item["executable"] for line in result.stdout.splitlines()
                if (item := json.loads(line)).get("executable")]
    binary, = binaries
    return binary


def run(binary, arguments, mode, name, directory, expected_failure=False):
    environment = dict(os.environ, HARMONIGRAPH_REQUIRE_GPU="1", WGPU_BACKEND="metal",
                       HARMONIGRAPH_METAL_ASSETS=mode,
                       HARMONIGRAPH_METAL_ASSET_DIR=str(directory / "assets"))
    result = subprocess.run([binary, *arguments, "--nocapture", "--test-threads=1"],
                            env=environment, text=True, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, timeout=300)
    (directory / f"{name}.log").write_text(result.stdout)
    hits = re.findall(r"METAL_ASSET hit ([0-9a-f]+)", result.stdout)
    sources = re.findall(r"METAL_ASSET source ([0-9a-f]+)", result.stdout)
    row = {"name": name, "mode": mode, "status": result.returncode,
           "hits": len(hits), "unique_hits": len(set(hits)), "sources": len(sources)}
    print(json.dumps(row), flush=True)
    if expected_failure:
        assert result.returncode != 0 and "missing, mismatched or invalid Metal asset" in result.stdout
    else:
        assert result.returncode == 0, f"{name} failed; see {directory / (name + '.log')}"
        assert hits or sources, f"{name} did not reach the instrumented Metal backend"
        if mode == "strict":
            assert hits and not sources, f"{name} silently compiled from source"
            assert "missing, mismatched or invalid Metal asset" not in result.stdout, \
                f"{name} caught an unexpected strict-mode miss"
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory.resolve()
    config = directory / "patch.toml"
    renderer = build("harmonigraph-render", config, directory)
    offline = build("harmonigraph-offline", config, directory)
    probe = ["generated_metal_assets_preserve_storage_binding_lengths", "--ignored"]
    cases = [("bindings", renderer, probe), ("renderer", renderer, []),
             ("offline-goldens", offline, ["golden"])]
    rows = []

    def record(*arguments, **keywords):
        rows.append(run(*arguments, directory=directory, **keywords))
        (directory / "results.json").write_text(json.dumps(rows, indent=2) + "\n")

    for name, binary, arguments in cases:
        record(binary, arguments, "export", f"export-{name}")
    assets = directory / "assets"
    backend.compile_assets(assets)
    backend.verify(assets)
    for name, binary, arguments in cases:
        record(binary, arguments, "strict", f"strict-{name}")

    # The diagnostic program has a unique named Input struct. Withhold its
    # library without touching compiler caches or any installed application.
    source, = [p for p in assets.glob("*.metal") if "struct Input" in p.read_text()]
    library = source.with_suffix(".metallib")
    saved = library.read_bytes()
    try:
        library.unlink()
        record(renderer, probe, "strict", "missing-strict", expected_failure=True)
        record(renderer, probe, "fallback", "missing-fallback")
        assert rows[-1]["sources"] == 1, "missing diagnostic must use exactly one source fallback"
    finally:
        library.write_bytes(saved)
    saved_source = source.read_bytes()
    try:
        source.write_bytes(saved_source + b"\n// mismatched artifact control\n")
        record(renderer, probe, "strict", "mismatch-strict", expected_failure=True)
        record(renderer, probe, "fallback", "mismatch-fallback")
        assert rows[-1]["sources"] == 1, "mismatched diagnostic must use one source fallback"
    finally:
        source.write_bytes(saved_source)
    backend.verify(assets)
    print(f"Generated-asset experiment passed; logs and results: {directory}")


if __name__ == "__main__":
    main()
