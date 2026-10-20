#!/usr/bin/env python3
"""Run a diagnostic command with no stdin, a deadline and persistent evidence."""
import argparse
import math
from pathlib import Path
import sys

from process_tools import run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=float, default=60)
    parser.add_argument('--cwd', type=Path, default=Path.cwd())
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command
    if command[:1] == ['--']:
        command = command[1:]
    if not command or not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('supply a positive --timeout and -- command [arguments]')
    code, _ = run(command, cwd=args.cwd.resolve(), timeout=args.timeout,
                  label='diagnostic')
    return code


if __name__ == '__main__':
    sys.exit(main())
