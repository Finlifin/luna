#!/usr/bin/env python3
"""Show one saved tool result and bounded log tails; no polling or rerunning."""
import argparse
import json
from pathlib import Path

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('result', type=Path, help='result.json or its directory')
args = parser.parse_args()
path = args.result / 'result.json' if args.result.is_dir() else args.result
report = json.loads(path.read_text())
print(json.dumps(report, ensure_ascii=False, indent=2))
for channel in ['stdout', 'stderr']:
    log = path.parent / (channel + '.log')
    if log.exists():
        with log.open('rb') as source:
            source.seek(max(0, log.stat().st_size - 8192))
            tail = source.read(8192).decode(errors='replace')
        print(f'\n{channel} (last 8192 bytes):\n{tail}')
