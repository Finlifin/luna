"""Output-contract checks for the native example runner; no compiler build required."""
import tempfile
import unittest
import json
from pathlib import Path
from unittest.mock import patch

import host_examples
from host_examples import execution_result


class NativeExampleOutputTests(unittest.TestCase):
    def test_library_dependency_edges_use_declared_package_identities(self):
        example = host_examples.EXAMPLES['library-tuple']
        arguments = example.inputs(Path('entry.fl'))
        edges = [tuple(arguments[index + 1:index + 4])
                 for index, value in enumerate(arguments) if value == '--dependency']
        self.assertEqual(edges, [('1', 'tuples', '2'), ('1', 'meta', '3'),
                                 ('1', 'resources', '4'), ('2', 'meta', '3'), ('2', 'primitives', '5'),
                                 ('3', 'builtin', '5'), ('4', 'primitives', '5')])
        self.assertEqual(arguments[-2:], ['--primary', '1'])
        self.assertIn(str(host_examples.ROOT / 'library/builtin/types/tuple.fl'), arguments)
        self.assertIn(str(host_examples.ROOT / 'library/meta/lib.fl'), arguments)

    def test_standalone_math_resolves_the_shared_primitive_package(self):
        arguments = host_examples.EXAMPLES['math-fma'].inputs(Path('entry.fl'))
        edges = [tuple(arguments[index + 1:index + 4])
                 for index, value in enumerate(arguments) if value == '--dependency']
        self.assertIn(('2', 'primitives', '3'), edges)
        self.assertEqual(arguments.count(str(host_examples.ROOT /
                                             'library/builtin/primitives.fl')), 1)

    def test_private_driver_is_released_but_artifacts_survive(self):
        for outcome in (0, 1, RuntimeError('report failed'), KeyboardInterrupt()):
            with self.subTest(outcome=outcome), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)

                def execute(args, selected, artifacts, driver):
                    driver.write_bytes(b'private compiler')
                    (artifacts / 'program').write_bytes(b'native example')
                    (artifacts / 'summary.json').write_text('[]\n')
                    if isinstance(outcome, BaseException):
                        raise outcome
                    return outcome

                with patch.object(host_examples.tempfile, 'mkdtemp', return_value=str(root)), \
                     patch.object(host_examples, 'run_examples', side_effect=execute), \
                     patch('sys.argv', ['host_examples.py', 'arithmetic']), \
                     patch('builtins.print'):
                    if isinstance(outcome, BaseException):
                        with self.assertRaises(type(outcome)):
                            host_examples.main()
                    else:
                        self.assertEqual(host_examples.main(), outcome)
                self.assertFalse((root / 'luna').exists())
                self.assertEqual((root / 'program').read_bytes(), b'native example')
                self.assertEqual((root / 'summary.json').read_text(), '[]\n')

    def test_keep_going_preserves_failures_and_runs_remaining_cases(self):
        for keep_going in (False, True):
            with self.subTest(keep_going=keep_going), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                builds = []

                def run(command, *, timeout, label, directory):
                    directory.mkdir(parents=True)
                    output = directory / 'stdout.log'
                    output.write_text('(unsigned-integer 42)\n', encoding='utf-8')
                    (directory / 'result.json').write_text(json.dumps({
                        'stdout': {'path': str(output)},
                    }), encoding='utf-8')
                    if label.startswith('build-') and label != 'build-driver':
                        builds.append(str(directory.parent.name))
                        # A build failure must survive later successful cases.
                        if len(builds) == 1:
                            return 7, directory
                    return 0, directory

                with patch.object(host_examples.tempfile, 'mkdtemp', return_value=str(root)), \
                     patch.object(host_examples, 'run', side_effect=run), \
                     patch('sys.argv', ['host_examples.py', 'arithmetic', 'arithmetic',
                                       '--compare-optimization',
                                       *(['--keep-going'] if keep_going else [])]), \
                     patch('builtins.print'):
                    self.assertEqual(host_examples.main(), 1)
                results = json.loads((root / 'summary.json').read_text(encoding='utf-8'))
                self.assertEqual(len(builds), 4 if keep_going else 1)
                self.assertEqual([r['passed'] for r in results],
                                 [False, True, True, True] if keep_going else [False])
                self.assertEqual(results[0]['build_exit_code'], 7)
                self.assertNotIn('execute_report', results[0])

    def evaluate(self, raw, *, code=0, truncated=False, expected='(bool true)'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'stdout.log'
            path.write_bytes(raw)
            return execution_result(code, {'stdout': {'path': str(path), 'truncated': truncated}}, expected)

    def test_exact_utf8_result(self):
        expected = '(string "你好")'
        result = self.evaluate((expected + '\n').encode('utf-8'), expected=expected)
        self.assertTrue(result['passed'])
        self.assertEqual(result['actual'], expected + '\n')
        self.assertNotIn('failure', result)

    def test_extra_or_missing_bytes_fail(self):
        for raw in [b'(bool true)', b' (bool true)\n', b'(bool true) \n',
                    b'(bool true)\n\n', b'(bool true)\r\n', b'(bool true)\nextra']:
            with self.subTest(raw=raw):
                result = self.evaluate(raw)
                self.assertFalse(result['passed'])
                self.assertIn('differs', result['failure'])
                self.assertEqual(result['actual'].encode('utf-8'), raw)

    def test_invalid_encoding_is_reported_without_losing_summary(self):
        result = self.evaluate(b'(bool true)\n\xff')
        self.assertFalse(result['passed'])
        self.assertEqual(result['failure'], 'stdout is not valid UTF-8')
        self.assertIn('\ufffd', result['actual'])

    def test_truncated_matching_prefix_and_nonzero_exit_fail(self):
        self.assertEqual(self.evaluate(b'(bool true)\n', truncated=True)['failure'], 'stdout was truncated')
        result = self.evaluate(b'(bool true)\n', code=7)
        self.assertFalse(result['passed'])
        self.assertEqual(result['execute_exit_code'], 7)

    def test_missing_log_is_reported(self):
        result = execution_result(0, {}, '(bool true)')
        self.assertFalse(result['passed'])
        self.assertIn('stdout log is missing', result['failure'])

    def test_optimized_failure_preserves_both_reports_and_builds_driver_once(self):
        for release in (False, True):
            with self.subTest(release=release):
                self.check_optimized_failure(release)

    def check_optimized_failure(self, release):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            calls = []

            def run(command, *, timeout, label, directory):
                calls.append((command, label))
                directory.mkdir(parents=True)
                stdout = directory / 'stdout.log'
                # A successful O0 must not mask a miscompiled O3 result.
                optimized = '-O3' in str(directory)
                stdout.write_text('(unsigned-integer 41)\n' if optimized
                                  else '(unsigned-integer 42)\n', encoding='utf-8')
                (directory / 'result.json').write_text(
                    json.dumps({'stdout': {'path': str(stdout)}}), encoding='utf-8')
                return 0, directory

            with patch.object(host_examples.tempfile, 'mkdtemp', return_value=str(root)), \
                 patch.object(host_examples, 'run', side_effect=run), \
                 patch('sys.argv', ['host_examples.py', 'arithmetic', '--compare-optimization',
                                    *(['--release-driver'] if release else [])]), \
                 patch('builtins.print'):
                self.assertEqual(host_examples.main(), 1)
            results = json.loads((root / 'summary.json').read_text(encoding='utf-8'))
            self.assertEqual([r['opt_level'] for r in results], ['0', '3'])
            self.assertEqual([r['passed'] for r in results], [True, False])
            self.assertIn('differs', results[1]['failure'])
            self.assertNotEqual(results[0]['executable'], results[1]['executable'])
            self.assertTrue(all(Path(r['execute_report']).is_file() for r in results))
            self.assertEqual(sum(label == 'build-driver' for _, label in calls), 1)
            self.assertEqual('--release' in calls[0][0], release)
            self.assertTrue(all('--release-driver' not in command for command, _ in calls))
            self.assertTrue(all(r['driver_profile'] == ('release' if release else 'debug')
                                for r in results))


if __name__ == '__main__':
    unittest.main()
