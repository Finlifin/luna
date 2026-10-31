"""Storage failures must not erase prior reports or masquerade as successful runs."""
import contextlib
import errno
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import host_examples
import library_check
import process_tools
import run_host


class SummaryEvidenceTests(unittest.TestCase):
    def test_write_and_replace_failures_preserve_previous_snapshot_and_terminal_evidence(self):
        for operation in ('write_text', 'replace'):
            with self.subTest(operation=operation), tempfile.TemporaryDirectory() as directory:
                path = Path(directory) / 'summary.json'
                previous = '{"status":"previous"}\n'
                path.write_text(previous)
                payload = {'exit_code': 0, 'result': '你好\n"quoted"'}
                output = io.StringIO()
                with patch.object(Path, operation, side_effect=OSError(errno.EDQUOT, 'quota')):
                    self.assertFalse(process_tools.save_summary(path, payload, stream=output))
                self.assertEqual(path.read_text(), previous)
                self.assertFalse(path.with_name('summary.json.tmp').exists())
                terminal = json.loads(output.getvalue())
                self.assertEqual(terminal['wrapper_exit_code'], 125)
                self.assertFalse(terminal['summary_saved'])
                self.assertEqual(terminal['evidence'], payload)
                self.assertTrue(process_tools.save_summary(path, payload))
                self.assertEqual(json.loads(path.read_text()), payload)

    def test_library_stops_before_more_builds_when_summary_publication_fails(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'summary.json'
            path.write_text('{"old":true}')
            calls = []

            def run(command, **kwargs):
                calls.append(kwargs['label'])
                logs = kwargs['directory']
                logs.mkdir()
                (logs / 'stdout.log').write_text('')
                return 0, logs

            output = io.StringIO()
            with patch.object(library_check.tempfile, 'mkdtemp', return_value=str(root)), \
                    patch.object(library_check, 'run', side_effect=run), \
                    patch.object(Path, 'replace', side_effect=OSError(errno.EDQUOT, 'quota')), \
                    patch('sys.argv', ['library_check.py', '--stage', 'declarations', '--stage', 'native-math']), \
                    contextlib.redirect_stdout(output):
                self.assertEqual(library_check.main(), 125)
            self.assertEqual(calls, ['declarations'])
            self.assertEqual(json.loads(path.read_text()), {'old': True})
            terminal = json.loads(output.getvalue())
            self.assertEqual(terminal['evidence']['stages'][0]['exit_code'], 0)
            self.assertFalse(terminal['evidence']['passed'])
            self.assertEqual(terminal['evidence']['selected_stages'], ['declarations', 'native-math'])

    def test_example_summary_failure_stops_even_keep_going_and_releases_private_driver(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            calls = []

            def run(command, **kwargs):
                calls.append(kwargs['label'])
                logs = kwargs['directory']
                logs.mkdir(parents=True)
                (root / 'luna').write_bytes(b'private driver')
                output = logs / 'stdout.log'
                output.write_text('(unsigned-integer 42)\n')
                (logs / 'result.json').write_text(json.dumps({'stdout': {'path': str(output)}}))
                return 0, logs

            output = io.StringIO()
            with patch.object(host_examples.tempfile, 'mkdtemp', return_value=str(root)), \
                    patch.object(host_examples, 'run', side_effect=run), \
                    patch.object(Path, 'replace', side_effect=OSError(errno.EDQUOT, 'quota')), \
                    patch('sys.argv', ['host_examples.py', 'arithmetic', 'arithmetic', '--keep-going']), \
                    contextlib.redirect_stdout(output):
                self.assertEqual(host_examples.main(), 125)
            self.assertEqual(calls, ['build-driver', 'build-arithmetic', 'execute-arithmetic'])
            self.assertFalse((root / 'luna').exists())
            terminal = json.loads(output.getvalue().splitlines()[-1])
            self.assertTrue(terminal['evidence'][0]['passed'])
            self.assertEqual(terminal['wrapper_exit_code'], 125)

    def test_run_host_summary_failure_does_not_pollute_program_stdout_or_repeat_execution(self):
        for failing_save in (1, 2, 4):
            with self.subTest(failing_save=failing_save), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source = root / 'input.fl'
                source.write_text('fn run() {}')
                calls = []
                attempts = 0
                replace = Path.replace

                def fail_publication(path, destination):
                    nonlocal attempts
                    if path.name == 'summary.json.tmp':
                        attempts += 1
                        if attempts == failing_save:
                            raise OSError(errno.EDQUOT, 'quota')
                    return replace(path, destination)

                def run(command, **kwargs):
                    label = kwargs['label']
                    calls.append(label)
                    logs = kwargs['directory']
                    logs.mkdir()
                    if label == 'driver-build':
                        (root / 'luna').write_bytes(b'driver')
                    report = {}
                    for channel in ('stdout', 'stderr'):
                        path = logs / (channel + '.log')
                        path.write_bytes(b'program\x00\xff\n' if label == 'execute' and channel == 'stdout' else b'')
                        report[channel] = {'path': str(path)}
                    (logs / 'result.json').write_text(json.dumps(report))
                    return 0, logs

                stdout = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
                stderr = io.TextIOWrapper(io.BytesIO(), encoding='utf-8')
                with patch.object(run_host.tempfile, 'mkdtemp', return_value=str(root)), \
                        patch.object(run_host, 'run', side_effect=run), \
                        patch.object(Path, 'replace', new=fail_publication), \
                        patch('sys.stdout', stdout), patch('sys.stderr', stderr):
                    self.assertEqual(run_host.main([str(source)]), 125)
                stdout.flush()
                stderr.flush()
                self.assertEqual(len(calls), failing_save - 1)
                self.assertEqual(stdout.buffer.getvalue(), b'program\x00\xff\n' if failing_save == 4 else b'')
                terminal = json.loads(stderr.buffer.getvalue().decode().splitlines()[-1])
                self.assertEqual(len(terminal['evidence']['stages']), failing_save - 1)
                self.assertEqual(terminal['evidence']['exit_code'], 0 if failing_save > 1 else None)
                self.assertFalse((root / 'luna').exists())
                stdout.close()
                stderr.close()

    def test_example_missing_or_truncated_execution_report_preserves_failure_summary(self):
        for report_text in (None, '{"stdout":'):
            with self.subTest(report_text=report_text), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                calls = []

                def run(command, **kwargs):
                    calls.append(kwargs['label'])
                    logs = kwargs['directory']
                    logs.mkdir(parents=True)
                    if kwargs['label'].startswith('execute-') and report_text is not None:
                        (logs / 'result.json').write_text(report_text)
                    return 0, logs

                with patch.object(host_examples.tempfile, 'mkdtemp', return_value=str(root)), \
                        patch.object(host_examples, 'run', side_effect=run), \
                        patch('sys.argv', ['host_examples.py', 'arithmetic']), \
                        contextlib.redirect_stdout(io.StringIO()):
                    self.assertEqual(host_examples.main(), 1)
                result, = json.loads((root / 'summary.json').read_text())
                self.assertFalse(result['passed'])
                self.assertEqual(result['execute_exit_code'], 0)
                self.assertIn('cannot read execution evidence', result['failure'])
                self.assertEqual(len(calls), 3)
