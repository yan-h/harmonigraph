#!/usr/bin/env python3
"""Bundle the already-built macOS probe without touching Harmonigraph's slot."""
import pathlib
import plistlib
import shutil
import subprocess

root = pathlib.Path(__file__).resolve().parent
bundle = root / "target" / "Harmonigraph Offset Range Probe.clap"
contents = bundle / "Contents"
binary = contents / "MacOS" / "offset-range-probe"
binary.parent.mkdir(parents=True, exist_ok=True)
shutil.copy2(root / "target/release/libharmonigraph_offset_range_probe.dylib", binary)
with (contents / "Info.plist").open("wb") as stream:
    plistlib.dump({
        "CFBundleExecutable": binary.name,
        "CFBundleIdentifier": "org.harmonigraph.offset-range-probe",
        "CFBundleName": "Harmonigraph Offset Range Probe",
        "CFBundlePackageType": "BNDL",
        "CFBundleVersion": "1",
        "CFBundleShortVersionString": "0.1.0",
    }, stream)
subprocess.run(["codesign", "--force", "--sign", "-", str(bundle)], check=True)
print(bundle)
