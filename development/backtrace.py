#!/usr/bin/env python3
"""Optional batch GDB diagnostic. Never escalates privileges or requests input."""
import argparse
import math
import json
import shutil
import sys
from pathlib import Path
from process_tools import LUNA, run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=float, default=30)
    parser.add_argument('--binary', type=Path, default=LUNA)
    parser.add_argument('--core', type=Path, help='inspect an existing core without launching a process')
    parser.add_argument('luna_args', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    tail = args.luna_args
    if tail[:1] == ['--']:
        tail = tail[1:]
    if not math.isfinite(args.timeout) or args.timeout <= 0 or (args.core and tail) or (not args.core and not tail):
        parser.error('use --core <file> or -- <input.fl> [Luna options]; timeout must be positive')
    command = [shutil.which('gdb') or 'gdb', '-nx', '-nh', '-batch',
               '-iex', 'set auto-load off', '-iex', 'set debuginfod enabled off',
               '-ex', 'set pagination off', '-ex', 'set confirm off']
    if not args.core:
        command += ['-ex', 'run']
    command += ['-ex', 'thread apply all bt 80', '-ex', 'info program']
    if args.core:
        command += ['-c', str(args.core.resolve()), str(args.binary.resolve())]
    else:
        command += ['--args', str(args.binary.resolve()), *tail]
    code, directory = run(command, cwd=Path.cwd(), timeout=args.timeout, label='gdb-diagnostic')
    # GDB's exit code is not the inferior's exit code. Mark evidence explicitly;
    # a successful debugger invocation is never a passing compiler test.
    evidence = ''.join((directory / name).read_text(errors='replace')
                       for name in ['stdout.log', 'stderr.log'] if (directory / name).exists())
    unavailable = any(marker in evidence.lower() for marker in
                      ['operation not permitted', 'could not trace', 'ptrace: permission denied'])
    path = directory / 'result.json'
    report = json.loads(path.read_text())
    report['debugger_only'] = True
    if unavailable:
        report['status'] = 'unavailable'
        report['reason'] = 'ptrace restricted; keep reproduction logs or inspect an existing core'
        code = 125
    report['wrapper_exit_code'] = code
    path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    print(json.dumps({'diagnostic_status': report['status'], 'report': str(path)}))
    return code


if __name__ == '__main__':
    sys.exit(main())
