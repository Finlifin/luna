#!/usr/bin/env python3
"""Build and execute a small set of Flurry examples through the LLVM host pipeline."""
import argparse
from dataclasses import dataclass
import json
import math
from pathlib import Path
import sys
import tempfile

from process_tools import ROOT, run, save_summary


@dataclass(frozen=True)
class Example:
    expected: str
    options: tuple[str, ...] = ()
    dependencies: tuple[tuple[str, str], ...] = ()
    # Additional library-to-library edges; app already depends on every input.
    dependency_links: tuple[tuple[str, str], ...] = ()

    def inputs(self, source):
        if not self.dependencies:
            return [str(source)]
        arguments = ['--packages-v1', '--package', '1', '101', 'app', str(source)]
        for index, (name, path) in enumerate(self.dependencies, start=2):
            arguments.extend(['--package', str(index), str(index * 101), name,
                              str(ROOT / path), '--dependency', '1', name, str(index)])
        identities = {name: index for index, (name, _) in enumerate(self.dependencies, start=2)}
        for owner, dependency in self.dependency_links:
            arguments.extend(['--dependency', str(identities[owner]), dependency,
                              str(identities[dependency])])
        return arguments + ['--primary', '1']


RESOURCE_OPTIONS = ('--copy-protocol', 'Duplicate', '--drop-protocol', 'Retire')
MATH_OPERATIONS = (
    'fma', 'sqrt', 'copysign', 'floor', 'ceil', 'trunc', 'roundeven', 'round',
    'fabs', 'sin', 'cos', 'exp', 'exp2', 'log', 'log2', 'log10', 'pow', 'powi', 'ldexp',
    'minimum', 'maximum', 'bitreverse', 'ctpop', 'ctlz', 'cttz', 'bswap', 'fshl', 'fshr', 'minimumnum', 'maximumnum', 'isnan', 'isinf', 'isfinite', 'isnormal', 'issubnormal', 'signbit', 'add_sat', 'sub_sat', 'min', 'max', 'abs', 'bit_and', 'bit_or', 'bit_xor',
)
MATH_OPTIONS = tuple(argument for operation in MATH_OPERATIONS
                     for argument in ('--host-intrinsic',
                                      f'{operation}=math.target.Scalar.{operation}'))
EXAMPLES = {
    'object-queries': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'builder-code-generator': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'comptime-scalar-index': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'effectful-reflection-index': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'mirrored-enum': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'generated-enum': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'generated-nested-struct': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'generated-struct-methods': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'),
         ('primitives', 'library/builtin/primitives.fl'))),
    'staged-bitvec': Example('(bool true)'),
    'list-clauses': Example('(bool true)',
        ('--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('meta', 'library/meta/lib.fl'), ('resources', 'library/builtin/resources.fl'))),
    'staged-default-forwarding': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('resources', 'library/builtin/resources.fl'),)),
    'library-conversion': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('conversion', 'library/builtin/conversion.fl'), ('resources', 'library/builtin/resources.fl'))),
    'math-ldexp': Example('(bool true)',
        MATH_OPTIONS + ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('math', 'library/builtin/math_intrinsics.fl'), ('resources', 'library/builtin/resources.fl'))),
    'math-powi': Example('(bool true)',
        MATH_OPTIONS + ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('math', 'library/builtin/math_intrinsics.fl'), ('resources', 'library/builtin/resources.fl'))),
    'quote-contract-patterns': Example('(bool true)',
        ('--tuple-type', 'tuples.Tuple', '--meta-list-type', 'meta.List',
         '--copy-protocol', 'Duplicate', '--drop-protocol', 'resources.Drop',
         '--fuel', '1000000'),
        (('tuples', 'library/builtin/tuple.fl'), ('meta', 'library/meta/lib.fl'),
         ('resources', 'library/builtin/resources.fl')),
        (('tuples', 'meta'),)),
    'library-tuple': Example('(bool true)',
        ('--tuple-type', 'tuples.Tuple', '--meta-list-type', 'meta.List',
         '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('tuples', 'library/builtin/tuple.fl'), ('meta', 'library/meta/lib.fl'),
         ('resources', 'library/builtin/resources.fl')),
        (('tuples', 'meta'),)),
    'memory-result-contracts': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop', '--fuel', '1000000'),
        (('resources', 'library/builtin/resources.fl'),)),
    'library-option-callback': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('optional', 'library/builtin/option.fl'), ('resources', 'library/builtin/resources.fl'))),
    'library-optional-combinators': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop', '--fuel', '1000000'),
        (('optional', 'library/builtin/option.fl'), ('resources', 'library/builtin/resources.fl'))),
    'library-result': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('fallible', 'library/builtin/result.fl'), ('resources', 'library/builtin/resources.fl'))),
    'library-iteration': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop',
         '--inline-functions', '--fuel', '1000000'),
        (('iteration', 'library/builtin/iterator.fl'), ('resources', 'library/builtin/resources.fl'))),
    'library-native-once': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('resources', 'library/builtin/resources.fl'),)),
    'library-option': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('optional', 'library/builtin/option.fl'), ('resources', 'library/builtin/resources.fl'))),
    'field-closure-state': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-record-callback': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-record-state': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-owned-data': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-restored-owner': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-enum-callback': Example('(bool true)', RESOURCE_OPTIONS),

    'owned-field-callback': Example('(bool true)', RESOURCE_OPTIONS),
    'handler-parameter-control': Example('(bool true)', RESOURCE_OPTIONS),
    'entry-polynomials': Example('(bool true)', RESOURCE_OPTIONS + ('--fuel', '1000000')),
    'local-generics': Example('(bool true)'),
    'const-match': Example('(bool true)', RESOURCE_OPTIONS),
    'type-packets': Example('(bool true)',
        ('--tuple-type', 'input.Tuple', '--meta-list-type', 'input.Elements')),
    'local-declarations': Example('(bool true)'),
    'linked-list': Example('(bool true)', RESOURCE_OPTIONS),
    'recursive-data': Example('(bool true)', RESOURCE_OPTIONS),
    'arithmetic': Example('(unsigned-integer 42)'),
    'number-theory': Example('(unsigned-integer 1121)'),
    'generic-integers': Example('(bool true)',
        tuple(argument for operation, trait in (
            ('add', 'Add'), ('sub', 'Sub'), ('mul', 'Mul'), ('div', 'Div'),
            ('rem', 'Rem'), ('neg', 'Neg'), ('not', 'Not'))
            for argument in (f'--{operation}-protocol', f'operators.{trait}')),
        (('operators', 'library/builtin/ops.fl'),)),
    'owned-result': Example('(unit)', RESOURCE_OPTIONS + ('--discard-result',)),
    'floating': Example('(bool true)'),
    'generic-iteration': Example('(bool true)'),
    'generic-order': Example('(bool true)'),
    'intrinsic-contract': Example(
        '(bool true)', RESOURCE_OPTIONS + ('--host-intrinsic', 'ctlz=input.Ops.leading_zeros')),
    'intrinsic-predicate': Example(
        '(bool true)', RESOURCE_OPTIONS + ('--host-intrinsic', 'fma=input.Ops.fma')),
    'string': Example('(string "你好，Flurry！")'),
    'product-string-result': Example(
        '(anonymous-record (fields (field (string "label") (string "中文 100%")) '
        '(field (string "data") (tuple (string "") (string "a\\u0000b\\n\\\"\\\\"))) '
        '(field (string "seen") (bool true))))',
        ('--tuple-type', 'input.Tuple', '--meta-list-type', 'input.Elements')),
    'bitwise': Example('(bool true)', MATH_OPTIONS, (('math', 'library/builtin/math_intrinsics.fl'),)),
    'saturating-integers': Example('(bool true)', MATH_OPTIONS, (('math', 'library/builtin/math_intrinsics.fl'),)),
    'integer-minmax': Example('(bool true)', MATH_OPTIONS, (('math', 'library/builtin/math_intrinsics.fl'),)),
    'wrapping-abs': Example('(bool true)', RESOURCE_OPTIONS + MATH_OPTIONS + ('--fuel', '10000000'), (('math', 'library/builtin/math_intrinsics.fl'),)),
    'float-classification': Example('(bool true)', MATH_OPTIONS, (('math', 'library/builtin/math_intrinsics.fl'),)),
    'math-fma': Example('(bool true)', MATH_OPTIONS, (('math', 'library/builtin/math_intrinsics.fl'),)),
    'owned-dispatch': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('resources', 'library/builtin/resources.fl'),)),
    'owned-variants': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('resources', 'library/builtin/resources.fl'),)),
    'owned-callback': Example('(bool true)',
        ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('resources', 'library/builtin/resources.fl'),)),
    'owned-selection': Example('(bool true)', ('--ord-protocol', 'comparison.Ord', '--eq-protocol', 'comparison.Eq',
        '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('comparison', 'library/builtin/comparison.fl'), ('resources', 'library/builtin/resources.fl'))),
    'custom-order': Example('(bool true)', ('--ord-protocol', 'comparison.Ord', '--eq-protocol', 'comparison.Eq',
        '--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('comparison', 'library/builtin/comparison.fl'), ('resources', 'library/builtin/resources.fl'))),
    'library-order': Example(
        '(bool true)', ('--copy-protocol', 'resources.Copy', '--drop-protocol', 'resources.Drop'),
        (('comparison', 'library/builtin/comparison.fl'),
         ('resources', 'library/builtin/resources.fl'))),
    'error-product': Example('(bool true)', RESOURCE_OPTIONS),
    'staged-callback-order': Example('(bool true)', RESOURCE_OPTIONS),
    'pattern-pointer-cleanup': Example('(bool true)', RESOURCE_OPTIONS),
    'effect-callback-cleanup': Example('(bool true)', RESOURCE_OPTIONS),
    'template': Example('(bool true)'),
    'callback': Example('(bool true)'),
    'wide-integer': Example('(unsigned-integer 36893488147419103232)'),
    'destructor': Example('(bool true)', RESOURCE_OPTIONS),
    'direct-effect-cleanup': Example('(bool true)', RESOURCE_OPTIONS),
    'direct-calls': Example('(bool true)', RESOURCE_OPTIONS),
    'direct-return': Example('(bool true)', RESOURCE_OPTIONS),
    'direct-loop': Example('(bool true)', RESOURCE_OPTIONS),
    'passing-calls': Example('(bool true)', RESOURCE_OPTIONS),
    'passing-aggregates': Example('(bool true)', RESOURCE_OPTIONS),
    'inline-effect-cleanup': Example('(bool true)', RESOURCE_OPTIONS + ('--inline-functions',)),
}



def execution_result(code, report, expected):
    """Compare complete UTF-8 output; malformed output remains reviewable evidence."""
    stdout = report.get('stdout', {})
    result = {'execute_exit_code': code, 'expected': expected, 'passed': False}
    try:
        if 'path' not in stdout:
            raise OSError('stdout log is missing')
        raw = Path(stdout['path']).read_bytes()
    except OSError as error:
        result.update(actual='', failure=f'cannot read stdout: {error}')
        return result
    try:
        actual = raw.decode('utf-8')
        valid_utf8 = True
    except UnicodeDecodeError:
        actual = raw.decode('utf-8', errors='replace')
        valid_utf8 = False
    result['actual'] = actual
    if code != 0:
        result['failure'] = f'native execution exited with status {code}'
    elif stdout.get('truncated', False):
        result['failure'] = 'stdout was truncated'
    elif not valid_utf8:
        result['failure'] = 'stdout is not valid UTF-8'
    elif raw != (expected + '\n').encode('utf-8'):
        result['failure'] = 'stdout differs from the expected result and newline'
    else:
        result['passed'] = True
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('examples', nargs='*', help='default: all examples')
    optimization = parser.add_mutually_exclusive_group()
    optimization.add_argument('--opt-level', choices=['0', '1', '2', '3'], default='0')
    optimization.add_argument('--compare-optimization', action='store_true',
                              help='run each example at O0 and O3, sharing one driver build')
    parser.add_argument('--timeout', type=float, default=300,
                        help='timeout in seconds for each build (execution: at most 10s)')
    parser.add_argument('--keep-going', action='store_true',
                        help='finish all selected cases and report every failure')
    parser.add_argument('--release-driver', action='store_true',
                        help='build luna in release mode; independent of generated-code --opt-level')
    args = parser.parse_args()
    if not math.isfinite(args.timeout) or args.timeout <= 0:
        parser.error('--timeout must be positive and finite')
    selected = args.examples or list(EXAMPLES)
    for name in selected:
        if name not in EXAMPLES:
            parser.error(f'unknown example {name!r}; choose from {", ".join(EXAMPLES)}')
    directory = Path(tempfile.mkdtemp(prefix='luna-host-examples-'))
    print(f'Artifacts and logs: {directory}', flush=True)
    driver = directory / 'luna'
    try:
        return run_examples(args, selected, directory, driver)
    finally:
        # This unique run owns the private compiler copy, not the generated
        # programs or reports. Release it on failures and interruptions too.
        driver.unlink(missing_ok=True)


def run_examples(args, selected, directory, driver):
    results = []
    code, _ = run(
        [sys.executable, '-B', str(ROOT / 'development/build_host_driver.py'), str(driver),
         *(['--release'] if args.release_driver else [])],
        timeout=args.timeout, label='build-driver', directory=directory / 'driver-build')
    if code != 0:
        return code
    levels = ('0', '3') if args.compare_optimization else (args.opt_level,)
    cases = [(index, name, level) for index, name in enumerate(selected) for level in levels]
    for index, name, level in cases:
        example = EXAMPLES[name]
        suffix = f'-O{level}' if args.compare_optimization else ''
        case = directory / f'{index}-{name}{suffix}'
        case.mkdir()
        executable = case / 'program'
        source = ROOT / 'development/examples' / f'host-{name}.fl'
        code, build_logs = run(
            [str(driver), *example.inputs(source), '--build', 'run',
             '--opt-level', level, '--output', str(executable), *example.options],
            timeout=args.timeout, label=f'build-{name}', directory=case / 'build')
        result = {'example': name, 'build_exit_code': code,
                  'driver_profile': 'release' if args.release_driver else 'debug',
                  'build_report': str(build_logs / 'result.json'),
                  'executable': str(executable), 'options': list(example.options),
                  'opt_level': level, 'inputs': example.inputs(source), 'passed': False}
        if code == 0:
            code, logs = run([str(executable)], timeout=min(args.timeout, 10),
                             label=f'execute-{name}', directory=case / 'execute')
            result['execute_report'] = str(logs / 'result.json')
            try:
                report = json.loads((logs / 'result.json').read_text(encoding='utf-8'))
                result.update(execution_result(code, report, example.expected))
            except (OSError, ValueError) as error:
                result.update(execute_exit_code=code, failure=f'cannot read execution evidence: {error}')
        else:
            result['failure'] = f'build exited with status {code}'
        results.append(result)
        if not save_summary(directory / 'summary.json', results):
            return 125
        print(f'{name} O{level}: {"PASS" if result["passed"] else "FAIL"}', flush=True)
        if not result['passed'] and not args.keep_going:
            return 1
    return int(any(not result['passed'] for result in results))


if __name__ == '__main__':
    sys.exit(main())
