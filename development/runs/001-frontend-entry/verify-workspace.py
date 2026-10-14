"""Check that this run preserved all files outside its declared write boundary."""

import hashlib
import json
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN = Path(__file__).resolve().parent
STATE = json.loads((RUN / "graph.json").read_text())
BASELINE = json.loads(Path(STATE["task"]["baseline"]["manifest"]).read_text())
OWNED = (
    "compiler/hir/src/lib.rs",
    "compiler/hir/src/display.rs",
    "compiler/hir/tests/",
    "compiler/luna/src/",
    "compiler/luna/tests/",
    "compiler/diagnostic/src/lib.rs",
    "compiler/ast_lowering/src/item.rs",
    "compiler/ast_lowering/src/expr.rs",
    "compiler/parse/src/parser.rs",
    "development/README.md",
    "development/runs/001-frontend-entry/",
)


def owned(name):
    return any(name == rule or (rule.endswith("/") and name.startswith(rule)) for rule in OWNED)


def fingerprint(path):
    if path.is_symlink():
        return {"symlink": str(path.readlink())}
    if path.is_file():
        return {"sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
    return {"missing": True}


issues = []
protected = 0
for name, original in BASELINE.items():
    if owned(name):
        continue
    protected += 1
    if fingerprint(ROOT / name) != original:
        issues.append(f"Unrelated file changed: {name}")

current_paths = subprocess.check_output(
    ["git", "ls-files", "-co", "--exclude-standard", "-z"], cwd=ROOT
).decode().split("\0")
for name in set(current_paths):
    if name and name not in BASELINE and not owned(name):
        issues.append(f"Unexpected new file: {name}")

branch = subprocess.check_output(["git", "branch", "--show-current"], cwd=ROOT, text=True).strip()
head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
if branch != "dev":
    issues.append(f"Expected dev, found {branch}")
if head != STATE["task"]["baseline"]["commit"]:
    issues.append("HEAD changed during an uncommitted implementation run")

print(json.dumps({"protected_files": protected, "branch": branch, "head": head, "issues": issues}, indent=2))
raise SystemExit(bool(issues))
