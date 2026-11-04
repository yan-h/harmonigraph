#!/usr/bin/env python3
"""Validate a pushed PR locally; merge its receipt only during a proven runner outage."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import tempfile
from urllib.parse import urlsplit

from tools.golden_policy import check_golden

ROOT = Path(__file__).resolve().parent
ACQUISITION_FAILURE = "The job was not acquired by Runner of type hosted even after multiple attempts"
STEPS = {
    "full-ci": ["./ci.sh"],
    "metal-check": ["python3", "tools/shader-assets.py", "check"],
    "metal-controls": ["python3", "tools/shader-assets.py", "controls"],
}


def output(*command, env=None):
    environment = dict(os.environ) if env is None else dict(env)
    environment.pop("GH_REPO", None)
    return subprocess.check_output(command, cwd=ROOT, env=environment, text=True).strip()


def gh_json(*arguments):
    return json.loads(output("gh", *arguments))


def snapshot(number, repository):
    pr = gh_json("pr", "view", str(number), "--repo", repository, "--json",
                 "number,url,state,isDraft,headRefOid,baseRefName,body,mergeable,statusCheckRollup")
    if pr["state"] != "OPEN":
        raise ValueError("PR is not open")
    base = gh_json("api", f"repos/{repository}/git/ref/heads/{pr['baseRefName']}")["object"]["sha"]
    return dict(repository=repository, pr=number, head=pr["headRefOid"], base=base), pr


def clean_head(identity):
    if output("git", "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("Commit and push all source changes before local validation or merge")
    if output("git", "rev-parse", "HEAD") != identity["head"]:
        raise ValueError("Checkout does not match the pushed PR head")
    subprocess.run(["git", "merge-base", "--is-ancestor", identity["base"], "HEAD"],
                   cwd=ROOT, check=True)


def same_identity(tested, current):
    if tested != current:
        raise ValueError("PR head or base changed; run local validation again")


def evidence_root():
    common = Path(output("git", "rev-parse", "--git-common-dir"))
    return (ROOT / common / "local-ci").resolve()


def strict_environment(base):
    env = {key: value for key, value in os.environ.items()
           if not key.startswith("HARMONIGRAPH_") and key not in (
               "GOLDEN_BASE", "GOLDEN_FRAMES", "GITHUB_STEP_SUMMARY", "PYTHONOPTIMIZE",
               "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR", "RUSTUP_TOOLCHAIN", "SCCACHE_GHA_ENABLED",
           )}
    env.update(HARMONIGRAPH_REQUIRE_GPU="1", HARMONIGRAPH_REQUIRE_FFMPEG="1",
               WGPU_BACKEND="metal", GOLDEN_BASE=base, RUSTC_WRAPPER="sccache")
    return env


def preflight():
    if platform.system() != "Darwin":
        raise ValueError("Local outage validation requires macOS and Metal")
    for program in ("cargo", "rustc", "node", "sccache", "ffmpeg", "ffprobe", "gh", "xcrun"):
        if not shutil.which(program):
            raise ValueError(f"Required program is missing from PATH: {program}")
    if "libsvtav1" not in output("ffmpeg", "-hide_banner", "-encoders"):
        raise ValueError("ffmpeg must include libsvtav1 for the export tests")
    env = strict_environment("")
    return dict(os=output("sw_vers"), architecture=platform.machine(),
                rust=output("rustc", "--version", env=env),
                metal=output("xcrun", "--sdk", "macosx", "metal", "--version", env=env))


def logged(command, env, path):
    print("▶ " + " ".join(command) + f"\n  log: {path}", flush=True)
    with path.open("w") as log:
        subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def validate(number, repository):
    machine = preflight()
    identity, pr = snapshot(number, repository)
    output("git", "fetch", "origin", pr["baseRefName"])
    clean_head(identity)
    parent = evidence_root()
    parent.mkdir(parents=True, exist_ok=True)
    directory = Path(tempfile.mkdtemp(prefix=identity["head"][:12] + "-", dir=parent))
    env = strict_environment(identity["base"])
    for name, command in STEPS.items():
        logged(command, env, directory / (name + ".log"))
    # Fetch again: the body may have been edited while the suite ran.
    current, pr = snapshot(number, repository)
    same_identity(identity, current)
    clean_head(current)
    goldens = ROOT / "target/golden-diff"
    report = check_golden(goldens, pr["body"])
    shutil.copytree(goldens, directory / "golden", ignore=shutil.ignore_patterns("base", "head", "again"))
    # Preserve Metal's detailed assertion logs, including controls, outside target/.
    shutil.copytree(ROOT / "target/shader-asset-logs", directory / "metal",
                    ignore=shutil.ignore_patterns("frames", "*-build.jsonl"))
    receipt = dict(schema=1, identity=identity, machine=machine,
                   completed=datetime.now(timezone.utc).isoformat(),
                   steps=list(STEPS),
                   files={str(p.relative_to(directory)): digest(p)
                          for p in sorted(directory.rglob("*")) if p.is_file()})
    path = directory / "receipt.json"
    path.write_text(json.dumps(receipt, indent=2) + "\n")
    comment = directory / "comment.md"
    comment.write_text(
        f"Local outage validation passed for `{identity['head']}` against `{identity['base']}`.\n\n"
        "Full `ci.sh`, mandatory Metal GPU/ffmpeg tests, committed Metal corpus/source parity "
        "and fallback controls all passed; the live PR body declares every changed golden row.\n\n"
        f"```text\n{machine['os']}\n{machine['architecture']}\n{machine['rust']}\n{machine['metal']}\n```\n\n"
        f"{report}\nOwner-local logs and checksums: `{path}`.\n"
        f"Receipt SHA-256: `{digest(path)}`. This is local evidence, not a hosted CI success.\n"
    )
    subprocess.run(["gh", "pr", "comment", str(number), "--repo", repository,
                    "--body-file", str(comment)], cwd=ROOT, check=True)
    print(f"Local validation receipt: {path}")
    print(f"Merge during a confirmed outage: ./local-ci.py {number} --merge {path}")
    return path


def outage_job(job, annotations, head):
    return (job.get("head_sha") == head and job.get("runner_id") in (None, 0)
            and not job.get("steps")
            and any(label.startswith("macos-") for label in job.get("labels", []))
            and any(a.get("annotation_level") == "failure" and a.get("message") == ACQUISITION_FAILURE
                    for a in annotations))


def require_outage(repository, head, checks):
    """Waive only Actions jobs with no executed code and a runner-acquisition error.

    Full CI's dependent summary may fail solely because those groups were
    abandoned. A real test failure, another workflow, or an unproven queue blocks.
    """
    affected = []
    if not any(c.get("workflowName") == "Security audit" and c.get("conclusion") == "SUCCESS"
               for c in checks):
        raise ValueError("Security audit must pass normally; the exception replaces macOS execution only")
    for check in checks:
        if check.get("conclusion") == "SUCCESS" or check.get("state") == "SUCCESS":
            continue
        workflow = check.get("workflowName")
        url = urlsplit(check.get("detailsUrl", ""))
        parts = url.path.strip("/").split("/")
        if (workflow not in ("Full CI", "Metal shader assets") or url.netloc != "github.com"
                or len(parts) != 7 or "/".join(parts[:2]) != repository
                or parts[2:4] != ["actions", "runs"] or parts[5] != "job"):
            raise ValueError(f"Non-outage check blocks merge: {check.get('name', check)}")
        job = gh_json("api", f"repos/{repository}/actions/jobs/{int(parts[6])}")
        if job.get("workflow_name") != workflow:
            raise ValueError("Check does not belong to the expected workflow")
        annotations = gh_json("api", f"repos/{repository}/check-runs/{job['id']}/annotations?per_page=100")
        if outage_job(job, annotations, head):
            affected.append(check["detailsUrl"])
            continue
        failed = [s["name"] for s in job.get("steps", []) if s.get("conclusion") == "failure"]
        if (workflow == "Full CI" and job.get("name") == "Full CI"
                and failed == ["Did every group pass?"] and job.get("head_sha") == head):
            # Check the whole run, not just whichever group first reported a
            # check: an actual failure in the other group must remain blocking.
            jobs = gh_json("api", f"repos/{repository}/actions/runs/{job['run_id']}/jobs?per_page=100")["jobs"]
            groups = [j for j in jobs if j["name"] != "Full CI"]
            if groups and all(j.get("conclusion") == "success" or outage_job(
                    j, gh_json("api", f"repos/{repository}/check-runs/{j['id']}/annotations?per_page=100"), head)
                    for j in groups):
                affected.append(check["detailsUrl"])
                continue
        raise ValueError(f"Check has no proven runner-only failure: {check['name']}")
    if not affected:
        raise ValueError("No runner outage needs an owner exception; use the normal merge procedure")
    return affected


def merge(number, repository, path, method):
    directory = path.resolve().parent
    if directory.parent != evidence_root():
        raise ValueError("Receipt must belong to this repository's common Git local-ci directory")
    receipt = json.loads(path.read_text())
    required = {name + ".log" for name in STEPS} | {"golden/report.md", "golden/changed"}
    if (receipt.get("schema") != 1 or receipt.get("steps") != list(STEPS)
            or not required.issubset(receipt.get("files", {}))):
        raise ValueError("Incomplete local validation receipt")
    for name, expected in receipt["files"].items():
        file = directory / name
        if not file.resolve().is_relative_to(directory) or digest(file) != expected:
            raise ValueError(f"Local validation evidence changed: {name}")
    current, pr = snapshot(number, repository)
    same_identity(receipt["identity"], current)
    clean_head(current)
    if pr["mergeable"] != "MERGEABLE":
        raise ValueError("GitHub has not established that this PR merges without conflicts")
    if gh_json("api", "user")["login"] != repository.split("/")[0]:
        raise ValueError("Only the repository owner's account may use the outage exception")
    check_golden(directory / "golden", pr["body"])
    affected = require_outage(repository, current["head"], pr["statusCheckRollup"])
    note = directory / "merge.md"
    note.write_text(f"Owner outage exception for `{current['head']}` against `{current['base']}`.\n\n"
                    f"Local receipt: `{path.resolve()}` (SHA-256 `{digest(path)}`).\n\n"
                    "Hosted checks waived solely for runner acquisition failures:\n"
                    + "\n".join(f"- {url}" for url in affected) + "\n")
    subprocess.run(["gh", "pr", "comment", str(number), "--repo", repository,
                    "--body-file", str(note)], cwd=ROOT, check=True)
    if pr["isDraft"]:
        subprocess.run(["gh", "pr", "ready", str(number), "--repo", repository], cwd=ROOT, check=True)
    latest, latest_pr = snapshot(number, repository)
    same_identity(current, latest)
    clean_head(latest)
    check_golden(directory / "golden", latest_pr["body"])
    require_outage(repository, latest["head"], latest_pr["statusCheckRollup"])
    subprocess.run(["gh", "pr", "merge", str(number), "--repo", repository, "--admin",
                    "--match-head-commit", current["head"], "--" + method], cwd=ROOT, check=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pr", type=int)
    parser.add_argument("--merge", type=Path, metavar="RECEIPT", help="merge previously validated evidence")
    parser.add_argument("--method", choices=("squash", "merge"), default="squash")
    args = parser.parse_args()
    if not args.merge and os.environ.get("AGENT_LIFECYCLE_ACTIVE") != str(ROOT):
        os.execv(str(ROOT / "session-lifecycle.sh"),
                 [str(ROOT / "session-lifecycle.sh"), "run", "--", sys.executable, str(Path(__file__).resolve()), *sys.argv[1:]])
    repository = gh_json("repo", "view", "--json", "nameWithOwner")["nameWithOwner"]
    try:
        if args.merge:
            merge(args.pr, repository, args.merge, args.method)
        else:
            validate(args.pr, repository)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"Local CI refused: {error}")


if __name__ == "__main__":
    main()
