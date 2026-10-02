#!/usr/bin/env python3
"""The extractor must retain distinct instances and decode their actual JSON."""
import importlib.util
import json
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
