#!/usr/bin/env python3
"""Run Cargo through the shared cache manager, with bounded time and saved logs."""
import argparse
import math
import sys
from process_tools import ROOT, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=float, default=900)
    parser.add_argument('cargo_args', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    tail = args.cargo_args
    if tail[:1] == ['--']:
        tail = tail[1:]
    if not tail or tail[0] not in {'test', 'check', 'build', 'run', 'clean'}:
        parser.error('supply -- test|check|build|run|clean and Cargo arguments')
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('--timeout must be positive')
    code, _ = run([sys.executable, '-B', str(ROOT / 'development/cargo.py'), *tail],
                  timeout=args.timeout, label='cargo')
    return code


if __name__ == '__main__':
    sys.exit(main())
