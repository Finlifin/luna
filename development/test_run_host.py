"""Single-source native runner sequencing, output, and failure contracts."""
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import run_host


class RunHostTests(unittest.TestCase):
    def exercise(self, failure=None, truncated=False, extra=(), packages=False, release=False):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            source = root / 'source with spaces.fl'
            source.write_text('fn run()->bool {true}')
            calls = []
            stdout = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
            stderr = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')

            def run(command, *, cwd, timeout, label, directory):
                calls.append((command, cwd, timeout, label))
                directory.mkdir()
                if label == 'driver-build':
                    (root / 'luna').write_bytes(b'private compiler')
                elif label == 'build':
                    self.assertTrue((root / 'luna').is_file())
                    (root / 'program').write_bytes(b'native example')
                print('runner status')
                code = failure[1] if failure and label == failure[0] else 0
                report = {}
                for name in ('stdout', 'stderr'):
                    path = directory / f'{name}.log'
                    data = b'program\x00\xff\n' if label == 'execute' and name == 'stdout' else b''
                    if code and name == 'stderr':
                        data = b'failure diagnostic\n'
                    path.write_bytes(data)
                    report[name] = {'path': str(path), 'truncated': truncated and label == 'execute' and name == 'stdout'}
                (directory / 'result.json').write_text(json.dumps(report))
                return code, directory

            with patch.object(run_host.tempfile, 'mkdtemp', return_value=str(root)), \
                 patch.object(run_host, 'run', side_effect=run), \
                 patch('sys.stdout', stdout), patch('sys.stderr', stderr):
                inputs = ['--packages-v1', '--package', '7', '107', 'app', str(source),
                          '--package', '9', '109', 'dep', 'dependency with spaces.fl',
                          '--dependency', '7', 'alias', '9', '--primary', '7'] if packages else []
                code = run_host.main([*([str(source)] if not packages else []),
                                      '--entry', 'start', '--opt-level', '3',
                                      '--run-timeout', '0.25', *(['--release-driver'] if release else []),
                                      '--', *inputs, *extra])
            self.assertFalse((root / 'luna').exists())
            if any(c[3] == 'build' for c in calls):
                self.assertEqual((root / 'program').read_bytes(), b'native example')
            stdout.flush()
            stderr.flush()
            return code, calls, json.loads((root / 'summary.json').read_text()), stdout.buffer.getvalue(), stderr.buffer.getvalue()

    def test_private_build_then_execute_with_exact_forwarded_arguments(self):
        code, calls, summary, output, diagnostic = self.exercise(extra=['--copy-protocol', 'A.Copy'])
        self.assertEqual(code, 0)
        self.assertEqual([c[3] for c in calls], ['driver-build', 'build', 'execute'])
        self.assertEqual(calls[1][0][-2:], ['--copy-protocol', 'A.Copy'])
        self.assertEqual(calls[1][0][3:5], ['--entry', 'start'])
        self.assertEqual(calls[2][2], 0.25)
        self.assertEqual(output, b'program\x00\xff\n')
        self.assertIn(b'runner status', diagnostic)
        self.assertEqual(summary['exit_code'], 0)
        self.assertEqual(len(summary['stages']), 3)
        self.assertNotIn('failed_stage', summary)

    def test_release_driver_is_independent_of_native_optimization(self):
        for release in (False, True):
            with self.subTest(release=release):
                code, calls, summary, output, _ = self.exercise(release=release)
                self.assertEqual(code, 0)
                self.assertEqual('--release' in calls[0][0], release)
                self.assertNotIn('--release', calls[1][0])
                self.assertNotIn('--release-driver', calls[1][0])
                self.assertEqual(calls[1][0][-2:], ['--opt-level', '3'])
                self.assertEqual(summary['driver_profile'], 'release' if release else 'debug')
                self.assertEqual(output, b'program\x00\xff\n')

    def test_package_inputs_preserve_identities_aliases_and_paths(self):
        code, calls, summary, output, _ = self.exercise(packages=True, extra=['--copy-protocol', 'alias.Copy'])
        self.assertEqual(code, 0)
        self.assertIsNone(summary['source'])
        self.assertEqual(calls[1][0][1:1 + len(summary['inputs'])], summary['inputs'])
        self.assertEqual(summary['inputs'][:6], ['packages', 'build', '--package', '7', '107', 'app'])
        self.assertIn('dependency with spaces.fl', summary['inputs'])
        self.assertEqual(summary['inputs'][-8:], ['--dependency', '7', 'alias', '9', '--primary', '7', '--copy-protocol', 'alias.Copy'])
        self.assertEqual(output, b'program\x00\xff\n')

    def test_missing_input_and_mixed_modes_do_not_start_a_driver(self):
        for arguments in [[], ['--', '--packages-stream-v1'], ['input.fl', '--', '--packages-v1']]:
            with self.subTest(arguments=arguments), patch.object(run_host, 'run') as run, \
                 patch('sys.stderr', io.StringIO()), self.assertRaises(SystemExit) as error:
                run_host.main(arguments)
            self.assertEqual(error.exception.code, 2)
            run.assert_not_called()

    def test_failures_stop_dependent_stages_and_preserve_reports(self):
        for stage, count, status in [('driver-build', 1, 7), ('build', 2, 2), ('execute', 3, 124)]:
            with self.subTest(stage=stage):
                code, calls, summary, output, diagnostic = self.exercise(failure=(stage, status))
                self.assertEqual(code, status)
                self.assertEqual(len(calls), count)
                self.assertEqual(summary['failed_stage'], stage)
                self.assertEqual(summary['exit_code'], status)
                self.assertIn(b'failure diagnostic', diagnostic)
                if stage != 'execute':
                    self.assertEqual(output, b'')

    def test_successful_child_with_truncated_log_is_not_reported_as_complete(self):
        code, _, summary, _, diagnostic = self.exercise(truncated=True)
        self.assertEqual(code, 1)
        self.assertEqual(summary['failed_stage'], 'execute')
        self.assertEqual(summary['stages'][-1]['exit_code'], 0)
        self.assertIn(b'truncated', diagnostic)

    def test_invalid_timeout_does_not_start_build(self):
        for value in ['nan', 'inf', '0', '-1']:
            with self.subTest(value=value), patch.object(run_host, 'run') as run, \
                 patch('sys.stderr', io.StringIO()), self.assertRaises(SystemExit) as error:
                run_host.main(['input.fl', '--run-timeout', value])
            self.assertEqual(error.exception.code, 2)
            run.assert_not_called()


if __name__ == '__main__':
    unittest.main()
