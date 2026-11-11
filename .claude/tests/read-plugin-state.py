#!/usr/bin/env python3
"""The extractor must retain distinct instances and decode their actual JSON."""
import importlib.util
import json
import re
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
import zlib

spec = importlib.util.spec_from_file_location(
    "extractor", Path(__file__).resolve().parents[2] / "read-plugin-state.py"
)
extractor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extractor)


def state(name):
    return {"version": "0.0.0", "params": {}, "fields": {
        "ui-state": json.dumps(
            f'(version:6,appearance:(name:{json.dumps(name, ensure_ascii=False)}))',
            ensure_ascii=False,
        )
    }}


class StateExtraction(unittest.TestCase):
    def test_capture_uses_host_camera_after_closed_editor_automation(self):
        appearance = ('(camera:(target:(0.25,-0.25,1.0),yaw:0.4,pitch:0.3,'
                      'distance:12.0,projection:Cabinet,cabinet_scale:0.6),'
                      'view:(center_fives:9,center_threes:8),spectrum:(enabled:true))')
        saved = {"version": "0.5.0", "params": {
            f"camera-{key}": {"f32": value} for key, value in
            [("yaw", 1.0), ("pitch", -0.5), ("distance", 6.0),
             ("pan-x", 2.5), ("pan-y", -3.5)]
        }, "fields": {"ui-state": json.dumps(f'(version:7,appearance:{appearance})')}}
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory) / "camera.bwproject"
            project.write_bytes(b"BtWg\0" + json.dumps(saved).encode())
            def capture(*args):
                return subprocess.check_output(
                    [sys.executable, extractor.__file__, *args, str(project)], text=True
                )
            captured = capture("--appearance")
            camera = dict(extractor.split_ron(extractor.block(captured, "camera")))
            self.assertEqual(float(camera["distance"]), 6.0)
            self.assertEqual(float(camera["yaw"]), 1.0)
            self.assertEqual(float(camera["pitch"]), -0.5)
            self.assertEqual(camera["target"], "(-0.5,0.5,0.0)")
            self.assertEqual(camera["projection"], "Cabinet")
            self.assertEqual(camera["cabinet_scale"], "0.6")
            self.assertEqual(extractor.block(captured, "spectrum"), "enabled:true")
            rust = capture("--rust")
            self.assertIn("center_fives: 3,", rust)
            self.assertIn("center_threes: -4,", rust)
            self.assertIn("distance: 6.0", capture())
            # A state without host camera channels still owns its snapshot.
            saved["params"] = {}
            project.write_bytes(b"BtWg\0" + json.dumps(saved).encode())
            self.assertEqual(capture("--appearance").strip(), appearance)

    def test_rust_output_drops_keys_view_config_no_longer_declares(self):
        """A project saved before #1698 still holds `meantone` in its view."""
        ui = '(version:7,appearance:(view:(center_fives:3,meantone:true,max_sevens:2)))'
        saved = {"version": "0.6.0", "params": {}, "fields": {"ui-state": json.dumps(ui)}}
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory) / "retired.bwproject"
            project.write_bytes(b"BtWg\0" + json.dumps(saved).encode())
            out = subprocess.check_output(
                [sys.executable, extractor.__file__, "--rust", str(project)], text=True,
                stderr=subprocess.DEVNULL,
            )
        self.assertIn("center_fives: 3,", out)
        self.assertIn("max_sevens: 2,", out)
        self.assertNotIn("meantone", out)

    def test_capture_overlays_exactly_the_fields_rust_names_host_owned(self):
        """The overlay is a copy of the Rust list, so the two are compared."""
        source = (Path(__file__).resolve().parents[2]
                  / "crates/harmonigraph-ui/src/camera_movement.rs").read_text()
        body = source.split("fn copy_host_owned(", 1)[1].split("\n    }\n", 1)[0]
        owned = set(re.findall(r"^\s*(camera|view)\.(\w+) = from_\1\.\2;", body, re.M))
        self.assertGreaterEqual(len(owned), 5, "the Rust list was not found")
        appearance = ('(camera:(target:(0.25,-0.25,1.0),yaw:0.4,pitch:0.3,distance:12.0,'
                      'projection:Cabinet,cabinet_angle:0.7,cabinet_scale:0.6),'
                      'view:(center_fives:9,center_threes:8,center_sevens:1,max_sevens:2))')
        state = {"params": {
            f"camera-{key}": {"f32": value} for key, value in
            [("yaw", 1.0), ("pitch", -0.5), ("distance", 6.0), ("pan-x", 2.5), ("pan-y", -3.5)]
        }, "fields": {"ui-state": f"(version:7,appearance:{appearance})"}}
        captured = extractor.captured_appearance(state)

        def fields(text):
            return {(name, key): value
                    for name in ("camera", "view")
                    for key, value in extractor.split_ron(extractor.block(f"({text})", name))}
        before, after = fields(appearance[1:-1]), fields(captured)
        self.assertEqual(before.keys(), after.keys())
        written = {field for field in before if before[field] != after[field]}
        self.assertEqual(written, owned)

    def test_the_spiral_framing_is_read_from_either_saved_shape(self):
        spiral = 'spiral:(zoom:2.75,look:(0.4,-0.6))'
        shapes = {
            # Current builds save it among the preferences.
            "prefs": ('(version:7,appearance:(spectrum:(enabled:true)),'
                      f'prefs:(ui_scale:1.0,{spiral},show_perf:false))'),
            # Earlier builds saved it at the top level, with no `prefs`.
            "top level": f'(version:7,appearance:(spectrum:(enabled:true)),{spiral})',
        }
        for shape, ui in shapes.items():
            with self.subTest(shape):
                saved = {"version": "0.6.0", "params": {}, "fields": {"ui-state": json.dumps(ui)}}
                with tempfile.TemporaryDirectory() as directory:
                    project = Path(directory) / "spiral.bwproject"
                    project.write_bytes(b"BtWg\0" + json.dumps(saved).encode())
                    out = subprocess.check_output(
                        [sys.executable, extractor.__file__, str(project)], text=True
                    )
                self.assertIn("--- spiral (editor framing) ---", out)
                self.assertIn("  zoom: 2.75", out)
                self.assertIn("  look: (0.4,-0.6)", out)

    def test_another_plugins_state_is_not_read_as_an_unopened_editor(self):
        """Every nice-plug plugin saves the same JSON shape, so a project's other
        plugins are found too; only an editor of ours earns the no-ui-state warning."""
        saved = [
            {"version": "0.5.0", "params": {"camera-yaw": {"f32": 0.4}}, "fields": {}},
            {"version": "0.5.0", "params": {"tuning_delay": {"f32": 0.0}}, "fields": {}},
            {"version": "1.0.0", "params": {"gain": {"f32": 0.25}, "macro01": {"f32": 0.0}},
             "fields": {"editor": "{}", "patch": "{}"}},
        ]
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory) / "mixed.bwproject"
            project.write_bytes(b"BtWg\0" + b"\0\xff".join(
                json.dumps(s).encode() for s in saved))
            out = subprocess.check_output(
                [sys.executable, extractor.__file__, str(project)], text=True,
                stderr=subprocess.DEVNULL,
            )
        self.assertNotIn("gain", out)
        self.assertEqual(out.count("=== instance"), 2)
        self.assertEqual(out.count("the editor was never open before a save"), 1)
        self.assertIn("a Harmonigraph Tune", out)

    def test_distinct_compressed_instances_share_a_prefix(self):
        states = [state("first"), state("second")]
        encoded = [json.dumps(s, separators=(",", ":")).encode() for s in states]
        self.assertEqual(encoded[0][:32], encoded[1][:32])
        sections = []
        for data in encoded:
            compressor = zlib.compressobj(wbits=-15)
            sections.append(compressor.compress(data) + compressor.flush())
        with tempfile.TemporaryDirectory() as directory:
            project = Path(directory) / "test.bwproject"
            project.write_bytes(b"BtWg\0" + b"\0\xff".join(sections))
            found = extractor.find_states(project)
        self.assertEqual(len(found), 2)
        self.assertEqual(
            [s["fields"]["ui-state"] for s in found],
            [json.loads(s["fields"]["ui-state"]) for s in states],
        )

    def test_braces_and_quotes_in_names_are_json_string_contents(self):
        for name in ['camera{', 'camera}', 'quoted "{é']:
            with self.subTest(name=name):
                expected = state(name)
                data = b"\xff\0" + json.dumps(expected, separators=(",", ":"), ensure_ascii=False).encode() + b"\0\xff"
                self.assertEqual(list(extractor.json_blobs(data)), [expected])


if __name__ == "__main__":
    unittest.main()
