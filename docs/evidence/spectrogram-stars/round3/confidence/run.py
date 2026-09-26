"""Run balanced Stars comparisons sequentially; optionally include the local recording input."""
from pathlib import Path
import argparse
import json
import os
import subprocess
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--output", type=Path, default=Path("/private/tmp/stars-confidence"))
parser.add_argument("--recording", type=Path, help="1344 by 1024 bytes of slab-major preview light input")
parser.add_argument("--binary", type=Path, help="Optional prebuilt harmonigraph-render test executable")
args = parser.parse_args()
repo = Path(__file__).resolve().parent.parents[4]
out = args.output.resolve()
out.mkdir(parents=True, exist_ok=True)
base = {k: v for k, v in os.environ.items() if not k.startswith("PROBE_")}
base.update(HARMONIGRAPH_SHADER_ASSETS="source", PROBE_COMPARE="base_a,base_b,dust100,dust075", PROBE_CASE="stars", PROBE_FILLS="1", PROBE_FRAMES="600", PROBE_WARMUP="120")
runs = [("synthetic-4k-17", "3840x2160", 17, False), ("recording-4k-29", "3840x2160", 29, True), ("synthetic-1080-41", "1920x1080", 41, False), ("synthetic-4k-53", "3840x2160", 53, False), ("recording-4k-67", "3840x2160", 67, True), ("recording-1080-79", "1920x1080", 79, True), ("recording-1080-83", "1920x1080", 83, True), ("synthetic-1080-97", "1920x1080", 97, False)]
for name, size, seed, recording in runs:
    if recording and args.recording is None:
        print("Skipping recording input:", name, flush=True)
        continue
    env = base.copy()
    env.update(PROBE_SIZE=size, PROBE_SEED=str(seed), PROBE_RAW=str(out / f"{name}.csv"), PROBE_HISTORY_SECONDS="20" if recording else "10", PROBE_PPP="1" if recording else "2")
    if recording:
        env["PROBE_TAKE"] = str(args.recording.resolve())
    (out / f"{name}-env.json").write_text(json.dumps({k: v for k, v in env.items() if k.startswith(("PROBE_", "HARMONIGRAPH_"))}, indent=2) + "\n")
    command = ([str(args.binary.resolve()), "cloud_costs_by_style_and_dial"] if args.binary else ["cargo", "test", "--release", "-p", "harmonigraph-render", "cloud_costs_by_style_and_dial", "--"])
    command += ["--ignored", "--nocapture", "--test-threads=1"]
    print("Starting", name, flush=True)
    start = time.monotonic()
    with (out / f"{name}.log").open("w") as log:
        result = subprocess.run(command, cwd=repo, env=env, stdout=log, stderr=subprocess.STDOUT)
    print("Finished", name, "status", result.returncode, "seconds", round(time.monotonic() - start, 1), flush=True)
    if result.returncode:
        raise SystemExit(result.returncode)
