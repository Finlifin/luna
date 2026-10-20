"""Verify this run's regular-file baseline; does not mutate source or Git."""
import hashlib
import json
import subprocess
from pathlib import Path

repo = Path(__file__).resolve().parents[3]
run = Path(__file__).resolve().parent
graph = json.loads((run / 'graph.json').read_text())
baseline = json.loads(Path(graph['task']['baseline']['manifest']).read_text())
owned = [path for unit in graph['units'] if unit['id'] != 'grader'
         for path in unit['write_boundary']]


def permitted(name):
    return any(name == path or (path.endswith('/') and name.startswith(path)) for path in owned)


changed, errors = [], []
for name, expected in baseline.items():
    path = repo / name
    actual = hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None
    if actual != expected:
        changed.append(name)
        if not permitted(name):
            errors.append(name)

names = subprocess.check_output(
    ['git', 'ls-files', '-co', '--exclude-standard', '-z'], cwd=repo
).decode().split('\0')
new = []
for name in sorted(set(filter(None, names)) - baseline.keys()):
    path = repo / name
    if not path.is_file() or path.is_symlink() or 'target' in path.parts:
        continue
    new.append(name)
    if not permitted(name) and not name.startswith('development/runs/002-literal-foundations/'):
        errors.append(name)

branch = subprocess.check_output(['git', 'branch', '--show-current'], cwd=repo).decode().strip()
head = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=repo).decode().strip()
result = dict(branch=branch, head=head, changed_baseline_paths=changed, new_paths=new,
              unowned_changes=errors, checked_regular_files=len(baseline))
result['pass'] = not errors and branch == 'dev' and head == graph['task']['baseline']['commit']
print(json.dumps(result, indent=2))
raise SystemExit(0 if result['pass'] else 1)
