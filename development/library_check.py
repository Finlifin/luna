#!/usr/bin/env python3
"""Check real library declarations, native modules and the sysroot smoke program."""
import argparse
import json
import math
from pathlib import Path
import sys
import tempfile

from process_tools import ROOT, run, save_summary
from host_examples import EXAMPLES, MATH_OPERATIONS

STAGES = ('declarations', 'attributes', 'merged-attributes', 'meta-values', 'inline-iteration', 'manifest', 'native-resources', 'native-comparison',
          'native-math', 'sysroot-meta-math', 'sysroot-meta-object', 'sysroot-qualified-methods', 'native-option', 'native-result', 'native-tuple', 'native-conversion', 'hash-protocols', 'sysroot-hash', 'sysroot-meta', 'sysroot-builders', 'sysroot-enum-builder', 'sysroot-symbol-enum', 'sysroot-fixed-enum', 'sysroot-build-mode', 'sysroot-build-target', 'build-context', 'sysroot', 'sysroot-iteration', 'sysroot-resources', 'sysroot-patterns', 'sysroot-meta-patterns')


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
        ('attributes', ['run', '-p', 'luna', '--', 'packages', 'dump',
                        '--package', '1', '101', 'app',
                        str(ROOT / 'development/examples/library-attributes.fl'),
                        '--package', '2', '202', 'attrs', str(ROOT / 'library/attrs/lib.fl'),
                        '--dependency', '1', 'attrs', '2', '--primary', '1',
                        '--emit', 'attributes']),
        ('meta-values', ['run', '-p', 'luna', '--', 'packages', 'comptime',
                         '--package', '1', '101', 'app',
                         str(ROOT / 'development/examples/library-meta-values.fl'),
                         '--package', '2', '202', 'meta', str(ROOT / 'library/meta/lib.fl'),
                         '--package', '3', '303', 'builtin', str(ROOT / 'library/builtin/primitives.fl'),
                         '--dependency', '1', 'meta', '2', '--dependency', '1', 'builtin', '3',
                         '--dependency', '2', 'builtin', '3', '--builtin-impl-package', '2',
                         *[arg for operation in MATH_OPERATIONS for arg in
                           ('--comptime-intrinsic', f'{operation}=meta.numeric.scalar_math.target.Scalar.{operation}')],
                         '--primary', '1', '--meta-list-type', 'meta.List', '--entry', 'run']),
        ('inline-iteration', ['test', '-p', 'luna', '--test', 'library_iteration']),
        ('manifest', ['test', '-p', 'luna', '--lib', 'repository_sysroot']),
        ('native-resources', ['run', '-p', 'luna', '--', 'packages', 'test',
                              '--package', '1', '101', 'app',
                              str(ROOT / 'development/examples/host-library-resources.fl'),
                              '--package', '2', '202', 'resources',
                              str(ROOT / 'library/builtin/protocols/resources.fl'),
                              '--package', '3', '303', 'primitives',
                              str(ROOT / 'library/builtin/primitives.fl'),
                              '--dependency', '2', 'primitives', '3',
                              '--dependency', '1', 'resources', '2', '--primary', '1',
                              '--copy-protocol', 'resources.Copy',
                              '--drop-protocol', 'resources.Drop']),
        ('native-comparison', ['run', '-p', 'luna', '--', 'packages', 'test',
                               '--package', '1', '101', 'app',
                               str(ROOT / 'development/examples/host-library-comparison.fl'),
                               '--package', '2', '202', 'comparison', str(ROOT / 'library/builtin/numeric/comparison.fl'),
                               '--package', '3', '303', 'resources', str(ROOT / 'library/builtin/protocols/resources.fl'),
                               '--package', '4', '404', 'attrs', str(ROOT / 'library/attrs/lib.fl'),
                               '--package', '5', '505', 'primitives',
                               str(ROOT / 'library/builtin/primitives.fl'),
                               '--dependency', '3', 'primitives', '5',
                               '--dependency', '2', 'primitives', '5',
                               '--dependency', '2', 'attrs', '4',
                               '--dependency', '1', 'comparison', '2',
                               '--dependency', '1', 'resources', '3', '--primary', '1',
                               '--copy-protocol', 'resources.Copy',
                               '--drop-protocol', 'resources.Drop']),
        ('sysroot-meta-math', ['run', '-p', 'luna', '--', 'comptime',
                               str(ROOT / 'development/examples/sysroot-meta-math.fl'),
                               '--sysroot', str(ROOT / 'library'),
                               '--build-target', 'x86_64', 'linux', '64',
                               '--entry', 'run']),
        ('native-math', ['run', '-p', 'luna', '--', 'test',
                        str(ROOT / 'development/examples/host-math-tests.fl'),
                        '--sysroot', str(ROOT / 'library'),
                        '--fuel', '2000000']),
        ('sysroot-build-target', ['run', '-p', 'luna', '--', 'test',
                                  str(ROOT / 'development/examples/sysroot-build-target.fl'),
                                  '--sysroot', str(ROOT / 'library'),
                                  '--build-target', 'configured_arch', 'configured_os', '64',
                                  '--build-feature', 'vector=true', '--build-feature', 'disabled=false']),
        ('build-context', ['test', '-p', 'luna', '--test', 'build_context']),
        ('sysroot-build-mode', ['run', '-p', 'luna', '--', 'test',
                                str(ROOT / 'development/examples/sysroot-build-mode.fl'),
                                '--sysroot', str(ROOT / 'library'), '--build-mode', 'debug']),
        ('sysroot-fixed-enum', ['run', '-p', 'luna', '--', 'test',
                                str(ROOT / 'development/examples/sysroot-fixed-enum.fl'),
                                '--sysroot', str(ROOT / 'library')]),
        ('sysroot-symbol-enum', ['run', '-p', 'luna', '--', 'test',
                                 str(ROOT / 'development/examples/sysroot-symbol-enum.fl'),
                                 '--sysroot', str(ROOT / 'library')]),
        ('sysroot-enum-builder', ['run', '-p', 'luna', '--', 'test',
                                  str(ROOT / 'development/examples/sysroot-edited-enum.fl'),
                                  '--sysroot', str(ROOT / 'library')]),
        ('sysroot-builders', ['run', '-p', 'luna', '--', 'test',
                              str(ROOT / 'development/examples/sysroot-edited-struct.fl'),
                              '--sysroot', str(ROOT / 'library')]),
        ('sysroot-meta', ['run', '-p', 'luna', '--', 'test',
                          str(ROOT / 'development/examples/sysroot-meta-lists.fl'),
                          '--sysroot', str(ROOT / 'library')]),
        ('hash-protocols', ['test', '-p', 'luna', '--test', 'library_hash']),
        ('sysroot-hash', ['run', '-p', 'luna', '--', 'test',
                          str(ROOT / 'development/examples/sysroot-hash.fl'),
                          '--sysroot', str(ROOT / 'library')]),
        ('sysroot-meta-patterns', ['run', '-p', 'luna', '--', 'test',
                                   str(ROOT / 'development/examples/sysroot-meta-list-patterns.fl'),
                                   '--sysroot', str(ROOT / 'library')]),
        ('sysroot-patterns', ['run', '-p', 'luna', '--', 'test',
                              str(ROOT / 'development/examples/sysroot-list-patterns.fl'),
                              '--sysroot', str(ROOT / 'library')]),
        ('sysroot-resources', ['run', '-p', 'luna', '--', 'test',
                               str(ROOT / 'development/examples/sysroot-native-wrapper-copy.fl'),
                               '--sysroot', str(ROOT / 'library')]),
        ('sysroot-iteration', ['run', '-p', 'luna', '--', 'test',
                               str(ROOT / 'development/examples/sysroot-iteration.fl'),
                                '--dispatch-fuel', '1000000',
                               '--sysroot', str(ROOT / 'library'),
                               '--inline-functions', '--fuel', '1000000']),
        ('sysroot', ['run', '-p', 'luna', '--', 'build',
                     str(ROOT / 'development/examples/library-smoke.fl'),
                     '--sysroot', str(ROOT / 'library'), '--entry', 'run',
                     '--output', str(sysroot_executable)]),
    ]
    stages[-1:-1] = [
        ('sysroot-meta-object', ['run', '-p', 'luna', '--', 'comptime',
                                str(ROOT / 'development/examples/sysroot-meta-object-methods.fl'),
                                '--sysroot', str(ROOT / 'library'),
                                '--build-target', 'x86_64', 'linux', '64',
                                '--entry', 'run']),
        ('sysroot-qualified-methods', ['run', '-p', 'luna', '--', 'test',
                                      str(ROOT / 'development/examples/sysroot-qualified-methods.fl'),
                                      '--sysroot', str(ROOT / 'library')]),
    ]
    attribute_command = next(command for label, command in stages if label == 'attributes')
    stages.insert(2, ('merged-attributes', [*attribute_command[:-1], 'merged-attributes']))
    for name in ['option', 'result', 'tuple', 'conversion']:
        example = EXAMPLES[f'library-{name}']
        stages.insert(-1, (f'native-{name}', [
            'run', '-p', 'luna', '--',
            *example.inputs(ROOT / f'development/examples/host-library-{name}.fl', 'test'),
            *example.options,
        ]))
    if args.stage:
        stages = [(label, command) for label, command in stages if label in args.stage]
    native_counts = {'native-resources': 4, 'native-comparison': 6, 'native-math': 13,
                     'sysroot-qualified-methods': 1, 'native-option': 1, 'native-result': 1, 'native-tuple': 4, 'native-conversion': 3, 'sysroot-iteration': 30, 'sysroot-resources': 3, 'sysroot-patterns': 3, 'sysroot-meta-patterns': 3, 'sysroot-hash': 3, 'sysroot-meta': 1, 'sysroot-builders': 4, 'sysroot-enum-builder': 3, 'sysroot-symbol-enum': 2, 'sysroot-fixed-enum': 2, 'sysroot-build-mode': 1, 'sysroot-build-target': 2}
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
            if label in ('sysroot', 'meta-values', 'sysroot-meta-math', 'sysroot-meta-object'):
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
