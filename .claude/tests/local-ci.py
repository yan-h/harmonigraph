#!/usr/bin/env python3
"""Local validation must not turn missing evidence or real CI failures into a merge."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT))
spec = importlib.util.spec_from_file_location("local_ci", ROOT / "local-ci.py")
local = importlib.util.module_from_spec(spec)
spec.loader.exec_module(local)
from tools.golden_policy import check_golden

IDENTITY = dict(repository="owner/repo", pr=1, head="a" * 40, base="b" * 40)
FAILURE = [dict(annotation_level="failure", message=local.ACQUISITION_FAILURE)]
SECURITY = dict(name="Security audit", workflowName="Security audit", conclusion="SUCCESS")


def abandoned(identifier=10):
    return dict(id=identifier, run_id=1, name="workspace", workflow_name="Full CI",
                head_sha=IDENTITY["head"], runner_id=None, labels=["macos-26"], steps=[],
                status="completed", conclusion="cancelled")


def check(identifier=10, name="workspace"):
    return dict(name=name, workflowName="Full CI", conclusion="FAILURE",
                detailsUrl=f"https://github.com/owner/repo/actions/runs/1/job/{identifier}")


class LocalCITests(unittest.TestCase):
    def test_summary_checkout_cannot_erase_the_downloaded_golden_evidence(self):
        workflow = (ROOT / ".github/workflows/ci.yml").read_text().split("  full-ci-summary:", 1)[1]
        checkout = workflow.index("uses: actions/checkout@")
        download = workflow.index("uses: actions/download-artifact@")
        policy = workflow.index("run: python3 tools/golden_policy.py golden-diff")
        self.assertLess(checkout, download, "checkout clears a non-Git workspace before cloning")
        self.assertLess(download, policy)

    def test_missing_golden_artifacts_cannot_read_as_no_changes(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            with self.assertRaises(FileNotFoundError):
                check_golden(directory, "")
            (directory / "report.md").write_text("report")
            with self.assertRaises(FileNotFoundError):
                check_golden(directory, "")
            (directory / "changed").write_text("")
            self.assertEqual(check_golden(directory, ""), "report")

    def test_golden_rows_require_the_current_body_and_exact_metrics(self):
        with tempfile.TemporaryDirectory() as tmp:
            directory = Path(tmp)
            (directory / "report.md").write_text("report")
            (directory / "changed").write_text("| frame | 0.2 | 4 |\n")
            for body in ("", "| frame | 0.3 | 4 |"):
                with self.assertRaises(ValueError):
                    check_golden(directory, body)
            self.assertEqual(check_golden(directory, "Explanation\n| frame | 0.2 | 4 |"), "report")

    def test_inherited_overrides_cannot_skip_gpu_encoder_or_golden_work(self):
        overrides = dict(HARMONIGRAPH_REQUIRE_GPU="0", HARMONIGRAPH_REQUIRE_FFMPEG="0",
                         HARMONIGRAPH_METAL_ASSET_BUILD_DIR="other-corpus", GOLDEN_BASE="none",
                         GOLDEN_FRAMES="cached", PYTHONOPTIMIZE="1", WGPU_BACKEND="",
                         SCCACHE_GHA_ENABLED="true", CARGO_TARGET_DIR="elsewhere")
        with patch.dict(os.environ, overrides):
            env = local.strict_environment(IDENTITY["base"])
        self.assertEqual(env["HARMONIGRAPH_REQUIRE_GPU"], "1")
        self.assertEqual(env["HARMONIGRAPH_REQUIRE_FFMPEG"], "1")
        self.assertEqual(env["WGPU_BACKEND"], "metal")
        self.assertEqual(env["GOLDEN_BASE"], IDENTITY["base"])
        for key in ("HARMONIGRAPH_METAL_ASSET_BUILD_DIR", "GOLDEN_FRAMES", "PYTHONOPTIMIZE",
                    "SCCACHE_GHA_ENABLED", "CARGO_TARGET_DIR"):
            self.assertNotIn(key, env)

    def test_head_base_or_repository_changes_invalidate_the_receipt(self):
        local.same_identity(IDENTITY, dict(IDENTITY))
        for key in IDENTITY:
            with self.assertRaises(ValueError):
                local.same_identity(IDENTITY, dict(IDENTITY, **{key: "changed"}))

    def test_only_a_proven_unexecuted_macos_job_is_an_outage(self):
        job = abandoned()
        self.assertTrue(local.outage_job(job, FAILURE, IDENTITY["head"]))
        for changes in (dict(steps=[dict(name="Run full CI", conclusion="failure")]),
                        dict(runner_id=123), dict(head_sha="other"), dict(labels=["ubuntu-latest"])):
            self.assertFalse(local.outage_job(dict(job, **changes), FAILURE, IDENTITY["head"]))
        self.assertFalse(local.outage_job(job, [], IDENTITY["head"]), "an ordinary queue is not proof")

    def test_summary_cannot_hide_a_real_failure_in_another_group(self):
        group = abandoned()
        summary = dict(group, id=11, name="Full CI", runner_id=5,
                       steps=[dict(name="Did every group pass?", conclusion="failure")])
        other = dict(abandoned(12), name="isolated")
        def api(*args):
            route = args[1]
            if "/actions/jobs/11" in route:
                return summary
            if "/actions/runs/1/jobs" in route:
                return dict(jobs=[group, other, summary])
            return FAILURE
        with patch.object(local, "gh_json", side_effect=api):
            self.assertTrue(local.require_outage("owner/repo", IDENTITY["head"], [SECURITY, check(11, "Full CI")]))
            other["steps"] = [dict(name="Run full CI", conclusion="failure")]
            with self.assertRaises(ValueError):
                local.require_outage("owner/repo", IDENTITY["head"], [SECURITY, check(11, "Full CI")])

    def test_other_checks_and_unproven_queues_cannot_be_waived(self):
        with patch.object(local, "gh_json", side_effect=[abandoned(), []]):
            with self.assertRaises(ValueError):
                local.require_outage("owner/repo", IDENTITY["head"], [SECURITY, check()])
        with self.assertRaises(ValueError):
            local.require_outage("owner/repo", IDENTITY["head"], [SECURITY, dict(check(), workflowName="Other")])
        with self.assertRaisesRegex(ValueError, "Security audit must pass"):
            local.require_outage("owner/repo", IDENTITY["head"], [check()])

    def test_a_gate_failure_leaves_logs_but_no_success_receipt_or_pr_comment(self):
        with tempfile.TemporaryDirectory() as tmp, \
                patch.object(local, "preflight", return_value={}), \
                patch.object(local, "snapshot", return_value=(IDENTITY, dict(baseRefName="main"))), \
                patch.object(local, "clean_head"), patch.object(local, "output"), \
                patch.object(local, "evidence_root", return_value=Path(tmp)), \
                patch.object(local.subprocess, "run") as run:
            def fail(command, **kwargs):
                kwargs["stdout"].write("gate output\n")
                if "check" in command:
                    raise subprocess.CalledProcessError(1, command)
                return subprocess.CompletedProcess(command, 0)
            run.side_effect = fail
            with self.assertRaises(subprocess.CalledProcessError):
                local.validate(1, "owner/repo")
            self.assertEqual(list(Path(tmp).rglob("receipt.json")), [])
            self.assertEqual(len(list(Path(tmp).rglob("*.log"))), 2)
            self.assertTrue(all(call.args[0][0] != "gh" for call in run.call_args_list))

    def test_missing_or_modified_receipt_logs_block_before_any_merge_action(self):
        with tempfile.TemporaryDirectory() as tmp:
            parent = Path(tmp).resolve()
            directory = parent / "run"
            directory.mkdir()
            path = directory / "receipt.json"
            log = directory / "full-ci.log"
            log.write_text("passed")
            receipt = dict(schema=1, steps=list(local.STEPS), identity=IDENTITY,
                           files={"full-ci.log": local.digest(log)})
            path.write_text(json.dumps(receipt))
            with patch.object(local, "evidence_root", return_value=parent), \
                    patch.object(local, "snapshot") as remote:
                with self.assertRaisesRegex(ValueError, "Incomplete"):
                    local.merge(1, "owner/repo", path, "squash")
                remote.assert_not_called()
                for name in ("metal-check.log", "metal-controls.log", "golden/report.md", "golden/changed"):
                    file = directory / name
                    file.parent.mkdir(exist_ok=True)
                    file.write_text("fixture")
                    receipt["files"][name] = local.digest(file)
                path.write_text(json.dumps(receipt))
                log.write_text("modified")
                with self.assertRaisesRegex(ValueError, "evidence changed"):
                    local.merge(1, "owner/repo", path, "squash")
                remote.assert_not_called()

    def test_successful_validation_and_outage_merge_use_the_recorded_head(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp).resolve()
            golden = root / "target/golden-diff"
            golden.mkdir(parents=True)
            row = "| frame | 0.2 | 4 |"
            (golden / "report.md").write_text(row + "\n")
            (golden / "changed").write_text(row + "\n")
            metal = root / "target/shader-asset-logs"
            metal.mkdir()
            (metal / "strict-catalog.log").write_text("1 passed")
            pr = dict(baseRefName="main", body=row, isDraft=True, mergeable="MERGEABLE",
                      statusCheckRollup=[SECURITY, check()])
            def api(*args):
                if args == ("api", "user"):
                    return dict(login="owner")
                if "/actions/jobs/" in args[1]:
                    return abandoned()
                return FAILURE
            def success(command, **kwargs):
                if "stdout" in kwargs:
                    kwargs["stdout"].write("passed\n")
                return subprocess.CompletedProcess(command, 0)
            with patch.object(local, "ROOT", root), \
                    patch.object(local, "preflight", return_value=dict(os="Mac", architecture="arm64", rust="pinned", metal="compiler")), \
                    patch.object(local, "snapshot", return_value=(IDENTITY, pr)), \
                    patch.object(local, "clean_head") as clean, patch.object(local, "output"), \
                    patch.object(local, "gh_json", side_effect=api), \
                    patch.object(local, "evidence_root", return_value=root / "evidence"), \
                    patch.object(local.subprocess, "run", side_effect=success) as run:
                path = local.validate(1, "owner/repo")
                self.assertTrue(path.is_file())
                for change, error in ((dict(body=""), "Golden rows"),
                                      (dict(statusCheckRollup=[check()]), "Security audit")):
                    with patch.object(local, "snapshot", side_effect=[(IDENTITY, pr), (IDENTITY, dict(pr, **change))]):
                        with self.assertRaisesRegex(ValueError, error):
                            local.merge(1, "owner/repo", path, "merge")
                        self.assertFalse(any(call.args[0][:3] == ["gh", "pr", "merge"]
                                             for call in run.call_args_list))
                local.merge(1, "owner/repo", path, "merge")
                command = run.call_args.args[0]
                self.assertIn("--admin", command)
                self.assertEqual(command[command.index("--match-head-commit") + 1], IDENTITY["head"])
                self.assertEqual(command[-1], "--merge")
                clean.assert_called_with(IDENTITY)


if __name__ == "__main__":
    unittest.main()
