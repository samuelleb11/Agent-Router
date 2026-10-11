"""Run repository gates and save actual Northstar measurements as CI artifacts.

Northstar never runs checks. CI does not need access to the local tracker: the
agent imports these measured results, with the recorded host, argv and git trees.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

repo = Path(__file__).resolve().parent.parent
artifacts = repo / ".ci/codex-measurements"
artifacts.mkdir(parents=True, exist_ok=True)
baseline = subprocess.check_output(["git", "rev-parse", "105b456^{commit}"], cwd=repo, text=True).strip()
host = subprocess.check_output(["hostname", "-s"], text=True).strip()
environment = os.environ.copy()
environment["CARGO_TARGET_DIR"] = str(repo / "target")


def measure(worktree, mode, label):
    argv = ["bash", "scripts/check-primary-codex.sh", mode]
    dirty = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=worktree, text=True).strip())
    assert not dirty, "Qualification must run from a clean tree"
    started = time.monotonic()
    result = subprocess.run(argv, cwd=worktree, env=environment, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    output = result.stdout
    (artifacts / f"{mode}-{label}.log").write_text(output)
    print(output, flush=True)
    measurement = {
        "run": argv, "exit_code": result.returncode, "host": host,
        "at_tree_sha": subprocess.check_output(["git", "rev-parse", "HEAD^{tree}"], cwd=worktree, text=True).strip(),
        "at_commit_sha": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=worktree, text=True).strip(),
        "tree_dirty": dirty or bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=worktree, text=True).strip()),
        "duration_ms": round((time.monotonic() - started) * 1000),
        "stdout_digest": hashlib.sha256(output.encode()).hexdigest(),
    }
    (artifacts / f"{mode}-{label}.json").write_text(json.dumps(measurement, indent=2))
    return result.returncode, output


with tempfile.TemporaryDirectory(prefix="ar-codex-baseline-") as scratch:
    worktree = Path(scratch) / "baseline"
    subprocess.run(["git", "worktree", "add", "--detach", str(worktree), baseline], cwd=repo, check=True)
    try:
        for mode in sys.argv[1:]:
            assert mode in ("gateway", "catalogue")
            code, output = measure(worktree, mode, "red")
            assert code not in (0, 124, 126, 127), f"{mode}: no meaningful failing baseline"
            assert "models::tests::" in output and "--- FAILED" in output, "Baseline must fail on behavior, not setup"
            code, _ = measure(repo, mode, "green")
            if code:
                raise SystemExit(code)
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(worktree)], cwd=repo, check=True)
