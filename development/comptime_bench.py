#!/usr/bin/env python3
"""Build and measure isolated VM workloads, retaining raw evidence."""
import argparse
import contextlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
from cargo import ROOT, TARGET, Runner
from process_tools import run


def build_benchmark(bench, destination):
    # Capture Cargo's chosen executable under the same lock as cache cleanup.
    # Cargo still sends rendered build diagnostics to the outer report.
    with tempfile.TemporaryFile(mode='w+', encoding='utf-8') as messages:
        def publish():
            messages.seek(0)
            executable = None
            for line in messages:
                event = json.loads(line)
                target = event.get('target', {})
                if (event['reason'] == 'compiler-artifact' and target.get('name') == bench
                        and 'bench' in target.get('kind', []) and event.get('executable')):
                    executable = Path(event['executable'])
            if executable is None:
                raise RuntimeError(f'Cargo did not publish the {bench} benchmark executable')
            with executable.open('rb') as source, destination.open('xb') as output:
                shutil.copyfileobj(source, output)
            destination.chmod(0o700)

        with contextlib.redirect_stdout(messages):
            return Runner(ROOT, TARGET).run(
                ['--no-fmt', 'build', '--release', '-p', 'comptime_interpretor',
                 '--bench', bench, '--message-format=json-render-diagnostics'],
                on_success=publish)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repeat', type=int, default=5)
    parser.add_argument('--bench', choices=('invocation', 'type_list_index', 'list_length', 'list_append', 'borrowed_aggregate', 'object_queries', 'source_session'), default='invocation')
    parser.add_argument('--iterations', type=int, help='invocations per workload and process')
    parser.add_argument('--binary', type=Path, help='measure a previously saved executable')
    parser.add_argument('--prepare', type=Path, help=argparse.SUPPRESS)
    args = parser.parse_args()
    if args.prepare is not None:
        return build_benchmark(args.bench, args.prepare)
    if args.repeat < 1:
        parser.error('--repeat must be positive')
    if args.iterations is not None and args.iterations < 1:
        parser.error('--iterations must be positive')
    environment = os.environ.copy()
    if args.iterations is not None:
        environment['LUNA_BENCH_ITERATIONS'] = str(args.iterations)
    with contextlib.ExitStack() as resources:
        binary = args.binary
        if binary is None:
            directory = resources.enter_context(tempfile.TemporaryDirectory(prefix='luna-bench-'))
            binary = Path(directory) / args.bench
            code, _ = run([sys.executable, '-B', str(Path(__file__).resolve()), '--bench',
                           args.bench, '--prepare', str(binary)],
                          cwd=ROOT, timeout=900, label='comptime-benchmark-build')
            if code:
                return code
        for _ in range(args.repeat):
            code, _ = run([str(binary.resolve())], cwd=ROOT, timeout=120,
                          label=f'comptime-benchmark-{args.bench}', env=environment)
            if code:
                return code
    return 0


if __name__ == '__main__':
    sys.exit(main())
