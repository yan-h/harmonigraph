#!/usr/bin/env python3
"""Read the plugin's live settings back out of a saved Bitwig project.

Answers "what is the plugin actually set to right now" — for copying a
dialed-in look into `ViewConfig::default()`, or for debugging a persist
bug against real state rather than a guess.

    ./read-plugin-state.py                 # newest Bitwig project
    ./read-plugin-state.py --rust          # ...as an impl Default body
    ./read-plugin-state.py --appearance project.bwproject > appearance.ron
    ./read-plugin-state.py path/to.bwproject

THE ONE THING THAT WILL WASTE YOUR TIME
---------------------------------------
The UI state (dock layout, camera, spiral framing, ViewConfig) is written
into the plugin state ONLY when the editor window is CLOSED — see `impl Drop for
LatticeEditorHandle` in crates/harmonigraph-plugin/src/editor.rs. Saving a
project with the plugin window open stores whatever was there before.

So the procedure is, in this order:

    1. Close the Harmonigraph plugin WINDOW in Bitwig.
    2. Save the project (Cmd+S).
    3. Run this.

If you skip step 1 you get stale values, or none at all, and nothing warns
you. (Host-automatable params — tuning, fade, color range — are not
affected; those live in the param system and are always current.)

WHERE THE BYTES ACTUALLY ARE
----------------------------
A .bwproject is Bitwig's own "BtWg" tagged binary container. Plugin state
sits inside a raw-DEFLATE section of it (no zlib header, so wbits=-15).
Inside that is nice-plug's plugin state as plain JSON:

    {"version":..,"params":{..},"fields":{"editor-state":..,"ui-state":..}}

`fields["ui-state"]` is the editor RON `SharedState::save_persist` wrote,
JSON-quoted a second time (a persisted field is stored serialized), so it
arrives as `"(version:..)"` and is decoded once more before parsing as RON.
Its `appearance` member holds camera, view, spectrum, spiral and video settings;
--appearance prints that complete document for the offline renderer.
nice-plug can also zstd-compress that JSON (see its wrapper/state.rs), so
this tries zstd too, and plaintext, before giving up.
"""

import argparse
import json
import pathlib
import re
import subprocess
import sys
import zlib

STATE_START = b'{"version"'


def newest_project() -> pathlib.Path:
    """The most recently modified .bwproject, auto-backups included — an
    autosave is as good a source as a manual save.

    Two sources, because Spotlight does not index the cloud-synced folder the
    real projects live in: `mdfind` alone found only Bitwig's own temp-project
    backups, a month stale, while the project saved a minute ago sat under
    Google Drive unseen."""
    out = subprocess.run(
        ["mdfind", "-name", ".bwproject"], capture_output=True, text=True
    ).stdout.split("\n")
    paths = [pathlib.Path(p) for p in out if p.strip().endswith(".bwproject")]
    cloud = pathlib.Path.home() / "Library/CloudStorage"
    for drive in cloud.glob("GoogleDrive-*/My Drive/music"):
        paths.extend(drive.rglob("*.bwproject"))
    if not paths:
        sys.exit("No .bwproject found by mdfind or under Google Drive. Pass one explicitly.")
    return max(paths, key=lambda p: p.stat().st_mtime)


def candidate_bytes(data: bytes):
    """Every decompression of `data` that might contain the state JSON,
    plus `data` itself in case it was never compressed."""
    yield data
    seen = set()
    for i in range(len(data) - 2):
        # Raw deflate (Bitwig's own sections) and zlib-wrapped, both cheap
        # to attempt and both observed in the wild.
        for wbits in (-15, 15):
            if wbits == 15 and data[i] != 0x78:
                continue
            try:
                out = zlib.decompressobj(wbits).decompress(data[i:])
            except Exception:
                continue
            if len(out) > 200 and STATE_START in out and out[:32] not in seen:
                seen.add(out[:32])
                yield out


def json_blobs(buf: bytes):
    """Every brace-balanced nice-plug state object in `buf`."""
    for m in re.finditer(re.escape(STATE_START), buf):
        depth, start = 0, m.start()
        for j in range(start, len(buf)):
            if buf[j] == 0x7B:
                depth += 1
            elif buf[j] == 0x7D:
                depth -= 1
                if depth == 0:
                    try:
                        yield json.loads(buf[start : j + 1])
                    except Exception:
                        pass
                    break


def find_states(path: pathlib.Path):
    """Every plugin instance's state in the project.

    The whole file, not the first section that yields one: adaptive tuning
    puts Harmonigraph Tune instances in a project (each one `tuning_delay`
    param and no `ui-state`), and they sit in separate compressed sections
    from the editor's. Stopping at the first productive one reported a Tune
    alone and read as "the editor was never closed".
    A full scan is about 13 s on a 600 KB project."""
    data = path.read_bytes()
    states, seen = [], set()
    for buf in candidate_bytes(data):
        for st in json_blobs(buf):
            # The strict RON split below reads a still-quoted blob as having
            # no top-level members at all, which prints as an unsupported format.
            ui = st.get("fields", {}).get("ui-state")
            if isinstance(ui, str) and ui.startswith('"'):
                st["fields"]["ui-state"] = json.loads(ui)
            key = json.dumps(st, sort_keys=True)
            if key not in seen and "fields" in st:
                seen.add(key)
                states.append(st)
    return states


def split_ron(body: str) -> "list[tuple[str, str]]":
    """Split a flat RON struct body into (name, value) on top-level commas."""
    out, depth, cur = [], 0, ""
    quoted, escaped = False, False
    for c in body:
        if quoted:
            cur += c
            if escaped:
                escaped = False
            elif c == "\\":
                escaped = True
            elif c == '"':
                quoted = False
            continue
        if c == '"':
            quoted = True
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        if c == "," and depth == 0:
            out.append(cur)
            cur = ""
        else:
            cur += c
    if cur.strip():
        out.append(cur)
    return [(f.split(":", 1)[0].strip(), f.split(":", 1)[1].strip()) for f in out if ":" in f]


def block(ui: str, name: str) -> "str | None":
    """The body of a top-level `name:(...)` block in the persist RON."""
    body = ui.strip().removeprefix("(").removesuffix(")")
    value = dict(split_ron(body)).get(name)
    if value and value.startswith("(") and value.endswith(")"):
        return value[1:-1]
    return None


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("project", nargs="?", help="a .bwproject (default: newest)")
    output = ap.add_mutually_exclusive_group()
    output.add_argument("--appearance", action="store_true",
                        help="print one complete appearance RON for offline --appearance")
    output.add_argument(
        "--rust",
        action="store_true",
        help="print the view fields as an impl Default body to paste into view.rs",
    )
    args = ap.parse_args()

    path = pathlib.Path(args.project).expanduser() if args.project else newest_project()
    print(f"# {path}", file=sys.stderr)

    states = find_states(path)
    if not states:
        sys.exit(
            "No plugin state found.\n"
            "If the plugin IS in this project, the likely cause is the trap in\n"
            "this script's header: the UI state is only written when the editor\n"
            "WINDOW is closed. Close it, save the project, and re-run."
        )

    if args.appearance:
        appearances = [
            body for st in states
            if (ui := st.get("fields", {}).get("ui-state"))
            and (body := block(ui, "appearance")) is not None
        ]
        if len(appearances) != 1:
            sys.exit(f"Expected one editor appearance, found {len(appearances)}; "
                     "close its window and save with the current plugin format.")
        print(f"({appearances[0]})")
        return

    if args.rust:
        # Only the editor's instance carries a view; a Harmonigraph Tune beside
        # it has no ui-state at all, and is not a failure.
        bodies = [
            (n, body)
            for n, st in enumerate(states, 1)
            if (ui := st.get("fields", {}).get("ui-state")) and (appearance := block(ui, "appearance"))
            and (body := block(f"({appearance})", "view"))
        ]
        if not bodies:
            sys.exit("No view block in any instance's ui-state blob.")
        # Numbered where there is more than one, so two runs of fields cannot be
        # read as one: the bodies print back to back, and a paste of the wrong
        # one is a look nobody dialled.
        for n, body in bodies:
            where = f" instance {n}" if len(bodies) > 1 else ""
            print(f"// From a live Bitwig session{where}; see read-plugin-state.py.")
            for name, value in split_ron(body):
                print(f"    {name}: {value},")
        return

    for n, st in enumerate(states, 1):
        ui = st.get("fields", {}).get("ui-state")
        if len(states) > 1:
            print(f"\n=== instance {n} ===")
        print("\n--- params (host-automatable; always current) ---")
        for k, v in sorted(st.get("params", {}).items()):
            print(f"  {k}: {list(v.values())[0]}")
        if not ui:
            # A Tune has no editor, so its missing ui-state is not the trap.
            if set(st.get("params", {})) == {"tuning_delay"}:
                print("\n(a Harmonigraph Tune — no editor, so no ui-state)")
            else:
                print("\n(no ui-state field — editor never closed before the save)")
            continue
        # The spiral's framing is persisted beside the camera and for the same
        # reason — a take renders from the blob, so a disc dialled in on its
        # inner turns has to export the picture it was dialled to. Left out,
        # a capture silently drops half the framing of one of the two pictures.
        appearance = block(ui, "appearance")
        if appearance is None:
            print("\n(no appearance document — older editor format is unsupported)")
            continue
        for name in ("camera", "spiral", "view", "spectrum", "render"):
            body = block(f"({appearance})", name)
            if body is None:
                continue
            print(f"\n--- {name} ---")
            for field, value in split_ron(body):
                print(f"  {field}: {value}")


if __name__ == "__main__":
    main()
