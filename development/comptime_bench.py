#!/usr/bin/env python3
"""Build and repeatedly measure invocation-only workloads, retaining raw evidence."""
import argparse
import os
from pathlib import Path
import sys
from process_tools import run

ROOT = Path(__file__).resolve().parent.parent
TARGET = Path('/tmp/luna-dev-002/target')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repeat', type=int, default=5)
    parser.add_argument('--bench', choices=('invocation', 'type_list_index', 'list_length', 'list_append', 'borrowed_aggregate'), default='invocation')
    parser.add_argument('--iterations', type=int, help='invocations per workload and process')
    parser.add_argument('--binary', type=Path, help='measure a previously saved executable')
    args = parser.parse_args()
    if args.repeat < 1:
        parser.error('--repeat must be positive')
    if args.iterations is not None and args.iterations < 1:
        parser.error('--iterations must be positive')
    environment = os.environ.copy()
    if args.iterations is not None:
        environment['LUNA_BENCH_ITERATIONS'] = str(args.iterations)
    binary = args.binary
    if binary is None:
        code, _ = run([sys.executable, '-B', str(ROOT / 'development/cargo.py'),
                       'build', '--release', '-p', 'comptime_interpretor', '--bench', args.bench],
                      cwd=ROOT, timeout=900, label='comptime-benchmark-build')
        if code:
            return code
        candidates = [p for p in (TARGET / 'release').rglob(f'{args.bench}-*')
                      if p.is_file() and not p.suffix and p.stat().st_mode & 0o111]
        if not candidates:
            raise RuntimeError(f'Cargo completed without a {args.bench} benchmark executable')
        binary = max(candidates, key=lambda p: p.stat().st_mtime_ns)
    for _ in range(args.repeat):
        code, _ = run([str(binary.resolve())], cwd=ROOT, timeout=120,
                      label=f'comptime-benchmark-{args.bench}', env=environment)
        if code:
            return code
    return 0


if __name__ == '__main__':
    sys.exit(main())
