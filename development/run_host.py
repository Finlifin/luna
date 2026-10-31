#!/usr/bin/env python3
"""Build and run a Flurry source or package entry with a private driver and retained logs."""
import argparse
from contextlib import redirect_stdout
import json
import math
from pathlib import Path
import sys
import tempfile

from process_tools import ROOT, run, save_summary


def relay(report, *, program):
    """Keep runner diagnostics off program stdout; preserve captured bytes."""
    complete = True
    for name in ('stdout', 'stderr'):
        stream = report[name]
        data = Path(stream['path']).read_bytes()
        destination = sys.stdout if program and name == 'stdout' else sys.stderr
        destination.buffer.write(data)
        destination.buffer.flush()
        if stream.get('truncated', False):
            print(f'run-host: {name} log was truncated; see the stage report', file=sys.stderr)
            complete = False
    return complete


def main(argv=None):
    argv = list(sys.argv[1:] if argv is None else argv)
    compiler_args = []
    if '--' in argv:
        separator = argv.index('--')
        argv, compiler_args = argv[:separator], argv[separator + 1:]
    parser = argparse.ArgumentParser(description=__doc__, epilog=
        'Pass compiler options after --, for example -- --copy-protocol Duplicate --drop-protocol Retire.')
    parser.add_argument('source', type=Path, nargs='?',
                        help='source file; omit when passing --packages-v1 after --')
    parser.add_argument('--entry', default='run')
    parser.add_argument('--opt-level', choices=['0', '1', '2', '3'], default='0')
    parser.add_argument('--build-timeout', type=float, default=300)
    parser.add_argument('--run-timeout', type=float, default=10)
    parser.add_argument('--release-driver', action='store_true',
                        help='build luna in release mode; independent of generated-code --opt-level')
    args = parser.parse_args(argv)
    for name in ('build_timeout', 'run_timeout'):
        value = getattr(args, name)
        if not math.isfinite(value) or value <= 0:
            parser.error(f'--{name.replace("_", "-")} must be positive and finite')
    source = args.source.resolve() if args.source is not None else None
    if source is not None:
        if '--packages-v1' in compiler_args:
            parser.error('omit the source argument when using --packages-v1')
        if not source.is_file():
            parser.error(f'source file does not exist: {source}')
        inputs = ['build', str(source)]
    elif compiler_args[:1] == ['--packages-v1']:
        # Keep package identities, dependency aliases, and paths verbatim.
        # luna owns validation of this existing versioned input protocol.
        inputs, compiler_args = ['packages', 'build', *compiler_args[1:]], []
    else:
        parser.error('provide a source file or -- followed by --packages-v1 inputs')
    directory = Path(tempfile.mkdtemp(prefix='luna-run-host-'))
    print(f'Artifacts and logs: {directory}', file=sys.stderr, flush=True)
    driver, executable = directory / 'luna', directory / 'program'
    summary = {'source': str(source) if source else None, 'inputs': inputs, 'entry': args.entry, 'executable': str(executable),
               'stages': [], 'exit_code': None,
               'driver_profile': 'release' if args.release_driver else 'debug'}

    def save():
        return save_summary(directory / 'summary.json', summary, stream=sys.stderr)

    stages = [
        ('driver-build', [sys.executable, '-B', str(ROOT / 'development/build_host_driver.py'),
                          str(driver), *(['--release'] if args.release_driver else [])], args.build_timeout),
        ('build', [str(driver), *inputs, '--entry', args.entry, '--output', str(executable),
                   '--opt-level', args.opt_level, *compiler_args], args.build_timeout),
        ('execute', [str(executable)], args.run_timeout),
    ]
    try:
        if not save():
            return 125
        for stage, command, timeout in stages:
            # Existing process tooling closes stdin, bounds capture, and terminates
            # the process group on timeout. Do not add a second subprocess runner.
            with redirect_stdout(sys.stderr):
                code, logs = run(command, cwd=Path.cwd(), timeout=timeout,
                                 label=stage, directory=directory / stage)
            report_path = logs / 'result.json'
            summary['stages'].append({'stage': stage, 'report': str(report_path), 'exit_code': code})
            try:
                report = json.loads(report_path.read_text(encoding='utf-8'))
                if stage == 'execute' or code != 0:
                    complete = relay(report, program=stage == 'execute')
                    if not complete and code == 0:
                        code = 1
            except (OSError, KeyError, ValueError) as error:
                print(f'run-host: cannot read stage evidence: {error}', file=sys.stderr)
                summary['evidence_error'] = str(error)
                code = code or 1
            summary['exit_code'] = code
            if code != 0:
                summary['failed_stage'] = stage
            if not save():
                return 125
            if code != 0:
                return code
        return 0
    finally:
        # The generated program and reports remain available for inspection.
        driver.unlink(missing_ok=True)


if __name__ == '__main__':
    sys.exit(main())
