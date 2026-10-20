#!/usr/bin/env python3
"""Run the existing Luna binary noninteractively; retain command, logs and status."""
import argparse
import math
import sys
from pathlib import Path
from process_tools import LUNA, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--binary', type=Path, default=LUNA)
    parser.add_argument('luna_args', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    tail = args.luna_args
    if tail[:1] == ['--']:
        tail = tail[1:]
    if not tail or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('supply -- <input.fl> [Luna options] and a positive timeout')
    code, _ = run([str(args.binary.resolve()), *tail], cwd=Path.cwd(),
                  timeout=args.timeout, label='luna-reproduction')
    return code


if __name__ == '__main__':
    sys.exit(main())
