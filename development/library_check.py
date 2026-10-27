#!/usr/bin/env python3
"""Check real library declarations, native modules and the sysroot smoke program."""
import argparse
import json
import math
from pathlib import Path
import sys
import tempfile

from process_tools import ROOT, run, save_summary
from host_examples import EXAMPLES, MATH_OPTIONS

STAGES = ('declarations', 'attributes', 'merged-attributes', 'meta-values', 'inline-iteration', 'manifest', 'native-resources', 'native-comparison',
          'native-math', 'native-option', 'native-result', 'native-tuple', 'native-conversion', 'sysroot', 'sysroot-iteration')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--timeout', type=float, default=300,
                        help='deadline for each check in seconds')
    parser.add_argument('--release', action='store_true',
                        help='reuse the release compiler and test cache')
    parser.add_argument('--stage', action='append', choices=STAGES,
                        help='run selected stages only; repeat to select more (default: all)')
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('--timeout must be positive and finite')
    directory = Path(tempfile.mkdtemp(prefix='luna-library-'))
    sysroot_executable = directory / 'sysroot-smoke'
    cargo = [sys.executable, '-B', str(ROOT / 'development/cargo.py')]
    stages = [
        ('declarations', ['test', '-p', 'luna', '--test', 'library_protocols']),
        ('attributes', ['run', '-p', 'luna', '--', '--packages-v1',
                        '--package', '1', '101', 'app',
                        str(ROOT / 'development/examples/library-attributes.fl'),
                        '--package', '2', '202', 'attrs', str(ROOT / 'library/attrs/lib.fl'),
                        '--dependency', '1', 'attrs', '2', '--primary', '1',
                        '--emit', 'attributes']),
        ('meta-values', ['run', '-p', 'luna', '--', '--packages-v1',
                         '--package', '1', '101', 'app',
                         str(ROOT / 'development/examples/library-meta-values.fl'),
                         '--package', '2', '202', 'meta', str(ROOT / 'library/meta/lib.fl'),
                         '--package', '3', '303', 'builtin', str(ROOT / 'library/builtin/primitives.fl'),
                         '--dependency', '1', 'meta', '2', '--dependency', '1', 'builtin', '3',
                         '--primary', '1', '--meta-list-type', 'meta.List', '--invoke-comptime', 'run']),
        ('inline-iteration', ['test', '-p', 'luna', '--test', 'library_iteration']),
        ('manifest', ['test', '-p', 'luna', '--lib', 'repository_sysroot']),
        ('native-resources', ['run', '-p', 'luna', '--', '--packages-v1',
                              '--package', '1', '101', 'app',
                              str(ROOT / 'development/examples/host-library-resources.fl'),
                              '--package', '2', '202', 'resources',
                              str(ROOT / 'library/builtin/resources.fl'),
                              '--dependency', '1', 'resources', '2', '--primary', '1',
                              '--copy-protocol', 'resources.Copy',
                              '--drop-protocol', 'resources.Drop', '--test-native']),
        ('native-comparison', ['run', '-p', 'luna', '--', '--packages-v1',
                               '--package', '1', '101', 'app',
                               str(ROOT / 'development/examples/host-library-comparison.fl'),
                               '--package', '2', '202', 'comparison', str(ROOT / 'library/builtin/comparison.fl'),
                               '--package', '3', '303', 'resources', str(ROOT / 'library/builtin/resources.fl'),
                               '--dependency', '1', 'comparison', '2',
                               '--dependency', '1', 'resources', '3', '--primary', '1',
                               '--copy-protocol', 'resources.Copy',
                               '--drop-protocol', 'resources.Drop', '--test-native']),
        ('native-math', ['run', '-p', 'luna', '--', '--packages-v1',
                        '--package', '1', '101', 'app',
                        str(ROOT / 'development/examples/host-math-tests.fl'),
                        '--package', '2', '202', 'math', str(ROOT / 'library/builtin/math_intrinsics.fl'),
                        '--package', '3', '303', 'resources', str(ROOT / 'library/builtin/resources.fl'),
                        '--dependency', '1', 'math', '2',
                        '--dependency', '1', 'resources', '3', '--primary', '1',
                        '--copy-protocol', 'resources.Copy',
                        '--drop-protocol', 'resources.Drop',
                        *MATH_OPTIONS,
                        '--test-native']),
        ('sysroot-iteration', ['run', '-p', 'luna', '--',
                               str(ROOT / 'development/examples/sysroot-iteration.fl'),
                               '--sysroot', str(ROOT / 'library'), '--test-native',
                               '--inline-functions', '--fuel', '200000']),
        ('sysroot', ['run', '-p', 'luna', '--',
                     str(ROOT / 'development/examples/library-smoke.fl'),
                     '--sysroot', str(ROOT / 'library'), '--build', 'run',
                     '--output', str(sysroot_executable)]),
    ]
    attribute_command = next(command for label, command in stages if label == 'attributes')
    stages.insert(2, ('merged-attributes', [*attribute_command[:-1], 'merged-attributes']))
    for name, alias, bindings in [
        ('option', 'optional', []),
        ('result', 'fallible', []),
    ]:
        stages.insert(-1, (f'native-{name}', [
            'run', '-p', 'luna', '--', '--packages-v1',
            '--package', '1', '101', 'app',
            str(ROOT / f'development/examples/host-library-{name}.fl'),
            '--package', '2', '202', alias, str(ROOT / f'library/builtin/{name}.fl'),
            '--package', '3', '303', 'resources', str(ROOT / 'library/builtin/resources.fl'),
            '--dependency', '1', alias, '2', '--dependency', '1', 'resources', '3',
            '--primary', '1', *bindings,
            '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop',
            '--test-native',
        ]))
    for name in ['tuple', 'conversion']:
        example = EXAMPLES[f'library-{name}']
        stages.insert(-1, (f'native-{name}', [
            'run', '-p', 'luna', '--',
            *example.inputs(ROOT / f'development/examples/host-library-{name}.fl'),
            *example.options, '--test-native',
        ]))
    if args.stage:
        stages = [(label, command) for label, command in stages if label in args.stage]
    native_counts = {'native-resources': 4, 'native-comparison': 6, 'native-math': 8,
                     'native-option': 1, 'native-result': 1, 'native-tuple': 4, 'native-conversion': 3, 'sysroot-iteration': 3}
    outcomes = []
    for label, command in stages:
        if args.release:
            command = [command[0], '--release', *command[1:]]
        code, evidence = run(cargo + command, cwd=ROOT, timeout=args.timeout,
                             label=label, directory=directory / label)
        build_report = None
        if label == 'sysroot':
            build_report = str(evidence / 'result.json')
            if code == 0:
                code, evidence = run([str(sysroot_executable)], cwd=ROOT,
                                     timeout=args.timeout, label='sysroot-run',
                                     directory=directory / 'sysroot-run')
        # A successful compiler or runner is insufficient: validate the value.
        matches = None
        if code == 0:
            output = (evidence / 'stdout.log').read_text().strip()
            if label in ('sysroot', 'meta-values'):
                matches = output == '(bool true)'
            elif label in ('attributes', 'merged-attributes'):
                matches = (output.count('(evaluated-attribute ') == 1
                           and '(property "abi")' in output
                           and '(symbol "c")' in output)
                if label == 'merged-attributes':
                    matches = (matches and output.startswith('(attribute-inspection (evaluated ')
                               and output.count('(merged-attributes ') == 1
                               and '(key "abi") (entry 0)' in output)
            elif label in native_counts:
                count = native_counts[label]
                matches = f'(count {count}) (failures 0)' in output
        outcomes.append({'stage': label, 'exit_code': code,
                         'value_matches': matches,
                         'passed': code == 0 and matches is not False,
                         'report': str(evidence / 'result.json'),
                         **({'build_report': build_report} if build_report else {})})
        summary = {'scope': 'selected' if args.stage else 'all',
                   'selected_stages': [label for label, _ in stages],
                   'passed': len(outcomes) == len(stages)
                   and all(item['passed'] for item in outcomes),
                   'stages': outcomes}
        if not save_summary(directory / 'summary.json', summary):
            return 125
    print(json.dumps({'summary_saved': True, 'passed': summary['passed'], 'scope': summary['scope'],
                      'selected_stages': summary['selected_stages'],
                      'summary': str(directory / 'summary.json')}), flush=True)
    return 0 if summary['passed'] else 1


if __name__ == '__main__':
    sys.exit(main())
