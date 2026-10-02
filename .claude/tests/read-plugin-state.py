#!/usr/bin/env python3
"""The extractor must retain distinct instances and decode their actual JSON."""
import importlib.util
import json
from pathlib import Path
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
