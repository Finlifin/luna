#!/usr/bin/env python3
"""Build the real std/sysroot println demo, then run its native executable."""
from pathlib import Path
import argparse
import subprocess
import sys

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=Path('/tmp/flurry-println'))
    args = parser.parse_args()
    output = args.output.resolve()
    build = subprocess.run([
        sys.executable, '-B', str(ROOT / 'development/check.py'),
        '--timeout', '900', '--', 'run', '-p', 'luna', '--', 'build',
        'development/examples/host-println.fl', '--entry', 'main',
        '--sysroot', 'library', '--fuel', '10000000', '--opt-level', '3',
        '--discard-result', '--output', str(output),
    ], cwd=ROOT, stdin=subprocess.DEVNULL, timeout=930)
    if build.returncode:
        return build.returncode
    print(f'Native executable: {output}', flush=True)
    return subprocess.run([str(output)], cwd=ROOT, stdin=subprocess.DEVNULL,
                          timeout=30).returncode


if __name__ == '__main__':
    raise SystemExit(main())
