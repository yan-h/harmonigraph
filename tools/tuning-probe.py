#!/usr/bin/env python3
"""Prepare and inspect #615 experiments. No Bitwig control or shared-slot writes."""

import argparse
from collections import Counter, defaultdict
import json
from pathlib import Path
import struct


def vlq(value):
    result = [value & 127]
    while value > 127:
        value >>= 7
        result.insert(0, (value & 127) | 128)
    return bytes(result)


def fixture(directory):
    """Type-1 MIDI: simultaneous separate-track notes, releases and silence."""
    tracks = []
    for name, key in [("Probe A - Polysynth", 60), ("Probe B - Vital VST3", 64),
                      ("Probe C - Instrument Layer", 67)]:
        events = [(0, b"\xff\x03" + vlq(len(name)) + name.encode()),
                  (0, b"\xff\x51\x03\x07\xa1\x20")]
        for beat in [4, 8, 12, 20]:
            events += [(beat * 960, bytes([0x90, key, 96])),
                       ((beat + 1) * 960, bytes([0xB0, 1, 64])),
                       ((beat + 2) * 960, bytes([0x80, key, 0]))]
        # A short note makes accidental late on/off collapse conspicuous.
        events += [(24 * 960, bytes([0x90, key, 96])),
                   (24 * 960 + 120, bytes([0x80, key, 0])),
                   (32 * 960, b"\xff\x2f\x00")]
        events.sort(key=lambda item: item[0])
        track, previous = bytearray(), 0
        for tick, event in events:
            track.extend(vlq(tick - previous) + event)
            previous = tick
        tracks.append(b"MTrk" + struct.pack(">I", len(track)) + track)
    path = directory / "three-tracks.mid"
    path.write_bytes(b"MThd" + struct.pack(">IHHH", 6, 1, 3, 960) + b"".join(tracks))
    print(path)


def prepare(args):
    args.directory.mkdir(parents=True, exist_ok=True)
    config = {"delay_samples": args.delay, "expected_sources": args.sources,
              "hold_source": 0, "hold_request": 1, "hold_extra_samples": args.late,
              "keep_alive": not args.allow_sleep, "hub_clock_offset": 0,
              "source_clock_offsets": [0] * 8}
    (args.directory / "config.json").write_text(json.dumps(config, indent=2) + "\n")
    fixture(args.directory)
    print("Candidate configuration only; activate/reload all probe instances to read it.")


def analyze(args):
    files = sorted(args.directory.glob("trace-*.jsonl"))
    if not files:
        raise SystemExit("No trace files found")
    summary = {"verdict": "UNMEASURED: this report cannot establish host viability",
               "instances": [], "faults": {}, "round_trips": []}
    faults = Counter()
    requests = defaultdict(dict)
    for path in files:
        records = []
        partial = False
        with path.open() as stream:
            for line in stream:
                try:
                    records.append(json.loads(line))
                except json.JSONDecodeError:
                    partial = True
        if not records or records[0].get("kind") != "header":
            faults["missing_header"] += 1
            continue
        header = records[0]
        source = header.get("source")
        row = {key: header.get(key) for key in ["pid", "instance", "class", "source", "build"]}
        row.update(file=str(path), callback_count=0, frames=[], sub_block_starts=[],
                   first_steady=None, last_steady=None, threads=[], activations=[],
                   accepted_notes=0, rejected_events=0, raw_note_ids=[],
                   latency_queries=0, reported_latencies=[], complete_file=False)
        frames, splits, threads, ids, latencies = set(), set(), set(), set(), set()
        generation = 0
        for record in records[1:]:
            kind, clock = record.get("kind"), record.get("clock", {})
            if "thread" in record:
                threads.add(record["thread"])
            if kind == "activation":
                row["activations"].append({key: record.get(key) for key in ["rate", "min", "max", "offline", "delay", "sources", "clock_offset"]})
            elif kind == "lifecycle" and record["name"] == "source_reset":
                # The reset record reports the generation being canceled. The
                # following progress/reply is authoritative for its successor.
                generation = record["generation"] + 1
            elif kind == "progress" and header["class"] == "tuner":
                generation = record["generation"]
            elif kind == "callback_enter":
                row["callback_count"] += 1
                frames.add(clock["frames"])
                if row["first_steady"] is None:
                    row["first_steady"] = clock["steady"]
                row["last_steady"] = clock["steady"]
                row["latency_queries"] = max(row["latency_queries"], record["latency_queries"])
                latencies.add(record["reported_latency"])
            elif kind == "sub_block" and record["enter"]:
                splits.add(clock["start"])
            elif kind in ("raw_input", "raw_output"):
                event = record["event"]
                if event["kind"] in (0, 1, 2, 4):
                    ids.add(event["note_id"])
                if kind == "raw_output":
                    row["accepted_notes"] += int(event["kind"] == 0 and record["accepted"])
                    row["rejected_events"] += int(not record["accepted"])
            elif kind == "fault":
                faults[record["reason"]] += 1
            elif kind == "trace_loss":
                faults["trace_loss"] += record["lost"]
            elif kind == "footer":
                row["complete_file"] = record["io_ok"] and record["lost"] == 0
            if kind in ("input", "reply_visible", "planned_output") and source is not None:
                key = (header["pid"], source, record.get("generation", generation), record.get("request", 0))
                if key[-1] and (kind != "input" or record["event_kind"] == "note_on"):
                    requests[key].setdefault(kind, record)
            elif kind in ("assignment", "reply_published"):
                key = (header["pid"], record["source"], record["generation"], record["request"])
                requests[key][kind] = record
        row.update(frames=sorted(frames), sub_block_starts=sorted(splits), threads=sorted(threads),
                   raw_note_ids=sorted(ids), reported_latencies=sorted(latencies), partial_line=partial)
        summary["instances"].append(row)
    for key, stages in sorted(requests.items()):
        output = stages.get("planned_output", {})
        trace = {"pid": key[0], "source": key[1], "generation": key[2], "request": key[3],
                 "stages": sorted(stages), "input": output.get("input"),
                 "deadline": output.get("deadline"), "requested_actual": output.get("actual"),
                 "extra_shift": output.get("extra_shift")}
        if "input" in stages and "reply_visible" in stages:
            trace["submission_to_visibility_ns"] = stages["reply_visible"]["ns"] - stages["input"]["ns"]
        summary["round_trips"].append(trace)
    summary["faults"] = dict(faults)
    summary["caution"] = "Requested output times require matching accepted raw_output records. Wall time and equal counters do not prove a common sample epoch or a future upper bound. Missing footer means still running or incomplete."
    result = json.dumps(summary, indent=2) + "\n"
    if args.output:
        args.output.write_text(result)
        print(args.output)
    else:
        print(result, end="")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, default=Path("/tmp/harmonigraph-tuning-probe"))
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("--delay", type=int, default=2048)
    prep.add_argument("--sources", type=int, default=3)
    prep.add_argument("--late", type=int, default=0)
    prep.add_argument("--allow-sleep", action="store_true")
    prep.set_defaults(run=prepare)
    inspect = sub.add_parser("analyze")
    inspect.add_argument("--output", type=Path)
    inspect.set_defaults(run=analyze)
    args = parser.parse_args()
    args.run(args)


if __name__ == "__main__":
    main()
