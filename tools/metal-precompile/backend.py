#!/usr/bin/env python3
"""Prepare the isolated wgpu hook, or compile/verify its exported shader assets."""

import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(directory):
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], text=True))
    package, = [p for p in metadata["packages"]
                if p["name"] == "wgpu-hal" and p["version"] == "29.0.4"]
    original = Path(package["manifest_path"]).parent
    copied = directory / "wgpu-hal"
    if copied.exists():
        raise SystemExit(f"refusing to overwrite existing backend: {copied}")
    shutil.copytree(original, copied)
    metal = copied / "src" / "metal"
    shutil.copyfile(Path(__file__).with_name("asset_hook.rs"), metal / "asset_hook.rs")
    with (metal / "mod.rs").open("a") as output:
        output.write("\nmod asset_hook;\n")
    path = metal / "device.rs"
    source = path.read_text()
    old = """let library = self
                    .shared
                    .device
                    .newLibraryWithSource_options_error(
                        &NSString::from_str(&source),
                        Some(&options),
                    )"""
    new = """let library = super::asset_hook::library(
                    &self.shared.device, &source, &options,
                )"""
    assert source.count(old) == 1, "pinned backend changed: inspect the hook boundary"
    source = source.replace(old, new)
    marker = "                Ok(CompiledShader {\n                    library,"
    assert source.count(marker) == 1, "pinned reflection boundary changed"
    diagnostic = """                if std::env::var_os("HARMONIGRAPH_METAL_ASSETS").is_some() {
                    std::eprintln!("METAL_REFLECTION entry={} sizes={:?} workgroup={:?} immutable={:x}",
                        stage.entry_point, sized_bindings, wg_memory_sizes, immutable_buffer_mask);
                }

"""
    path.write_text(source.replace(marker, diagnostic + marker))
    config = directory / "patch.toml"
    config.write_text(f"[patch.crates-io.wgpu-hal]\npath = {json.dumps(str(copied))}\n")
    print(f"Prepared copied backend; pass --config {config} to Cargo.")


def compile_assets(directory):
    sources = sorted(directory.glob("*.metal"))
    assert sources, "no backend-generated sources exported"
    compiler = subprocess.check_output(
        ["xcrun", "--sdk", "macosx", "metal", "--version"], text=True)
    flags = ["-std=metal3.2", "-mmacosx-version-min=15.0", "-ffast-math", "-fpreserve-invariance"]
    for source in sources:
        settings = dict(line.split("=", 1) for line in source.with_suffix(".options").read_text().splitlines())
        assert settings == {"wgpu-hal": "29.0.4", "language": str((3 << 16) + 2),
                            "fast_math": "true", "invariance": "true",
                            "math_mode": "2", "math_functions": "0"}, settings
        air = source.with_suffix(".air")
        subprocess.run(["xcrun", "--sdk", "macosx", "metal", *flags,
                        "-c", str(source), "-o", str(air)], check=True)
        subprocess.run(["xcrun", "--sdk", "macosx", "metallib", str(air),
                        "-o", str(source.with_suffix(".metallib"))], check=True)
        air.unlink()
    files = [source.with_suffix(extension) for source in sources
             for extension in (".metal", ".options", ".metallib")]
    manifest = {"compiler": compiler, "flags": flags,
                "sha256": {file.name: digest(file) for file in files}}
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"Compiled {len(sources)} libraries.")


def verify(directory):
    saved = json.loads((directory / "manifest.json").read_text())
    expected = {p.name for p in directory.iterdir()
                if p.suffix in (".metal", ".options", ".metallib")}
    assert expected and expected == set(saved["sha256"]), "artifact file set changed"
    for name, value in saved["sha256"].items():
        assert Path(name).name == name, "manifest paths must be simple filenames"
        assert digest(directory / name) == value, f"changed artifact: {name}"
    print(f"Verified {len(expected) // 3} generated shader assets.")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("prepare", "compile", "verify"))
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    directory = args.directory.resolve()
    directory.mkdir(parents=True, exist_ok=True)
    {"prepare": prepare, "compile": compile_assets, "verify": verify}[args.action](directory)


if __name__ == "__main__":
    main()
