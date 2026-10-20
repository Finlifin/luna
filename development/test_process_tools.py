import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import time
import unittest

from process_tools import run


class ProcessToolsTests(unittest.TestCase):
    def test_wrapper_termination_cleans_children_and_records_result(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            marker = directory / 'escaped'
            ready = directory / 'ready'
            child = ("import time;from pathlib import Path;"
                     f"Path({str(ready)!r}).touch();time.sleep(1.2);"
                     f"Path({str(marker)!r}).touch()")
            wrapper = ("from process_tools import run;import sys;"
                       f"sys.exit(run([sys.executable,'-c',{child!r}],"
                       f"directory={str(directory)!r},timeout=5)[0])")
            with subprocess.Popen([sys.executable, '-B', '-c', wrapper],
                                  cwd=Path(__file__).parent,
                                  stdout=subprocess.DEVNULL) as process:
                try:
                    deadline = time.monotonic() + 3
                    while not ready.exists() and time.monotonic() < deadline:
                        time.sleep(.01)
                    self.assertTrue(ready.exists())
                finally:
                    process.terminate()
                    process.wait(timeout=4)
                self.assertEqual(process.returncode, 143)
            report = json.loads((directory / 'result.json').read_text())
            self.assertEqual(report['status'], 'interrupted')
            self.assertEqual(report['wrapper_exit_code'], 143)
            time.sleep(1.3)
            self.assertFalse(marker.exists())

    def test_generic_command_preserves_arguments_without_shell_expansion(self):
        import subprocess
        result = subprocess.run(
            [sys.executable, '-B', str(Path(__file__).with_name('command.py')),
             '--timeout', '3', '--', sys.executable, '-c',
             'import sys;print(sys.argv[1]);sys.exit(7)', '$(false); literal'],
            capture_output=True, text=True, timeout=5)
        self.assertEqual(result.returncode, 7)
        report_path = Path(json.loads(result.stdout.splitlines()[-1])['report'])
        self.assertEqual((report_path.parent / 'stdout.log').read_text(),
                         '$(false); literal\n')

    def test_logs_are_visible_before_child_finishes(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            release = directory / 'release'
            child = ("import time;from pathlib import Path;"
                     "print('ready',flush=True);"
                     f"p=Path({str(release)!r});"
                     "\nwhile not p.exists(): time.sleep(.01)")
            wrapper = ("from process_tools import run;import sys;"
                       f"run([sys.executable,'-c',{child!r}],"
                       f"directory={str(directory)!r},timeout=5)")
            with subprocess.Popen([sys.executable, '-B', '-c', wrapper],
                                  cwd=Path(__file__).parent,
                                  stdout=subprocess.DEVNULL) as process:
                try:
                    log = directory / 'stdout.log'
                    deadline = time.monotonic() + 3
                    while time.monotonic() < deadline:
                        if log.exists() and log.read_text() == 'ready\n':
                            break
                        time.sleep(.01)
                    self.assertEqual(log.read_text(), 'ready\n')
                    self.assertIsNone(process.poll())
                finally:
                    release.touch()
                    process.wait(timeout=7)
                self.assertEqual(process.returncode, 0)

    def test_report_location_is_published_before_command_completes(self):
        import subprocess
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            release = directory / 'release'
            child = ("import time;from pathlib import Path;"
                     f"p=Path({str(release)!r});"
                     "\nwhile not p.exists(): time.sleep(.01)")
            wrapper = ("from process_tools import run;import sys;"
                       f"run([sys.executable,'-c',{child!r}],"
                       f"directory={str(directory)!r},timeout=3)")
            with subprocess.Popen([sys.executable, '-B', '-c', wrapper],
                                  cwd=Path(__file__).parent,
                                  stdout=subprocess.PIPE, text=True) as process:
                # Bound the read even if the early announcement regresses.
                import selectors
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    self.assertTrue(selector.select(timeout=2))
                report = json.loads(process.stdout.readline())
                self.assertEqual(report['status'], 'running')
                self.assertEqual(Path(report['report']), directory / 'result.json')
                self.assertIsNone(process.poll())
                release.touch()
                output, _ = process.communicate(timeout=5)
                self.assertEqual(json.loads(output)['status'], 'completed')
                self.assertEqual(process.returncode, 0)

    def execute(self, source, **options):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        directory = Path(temporary.name)
        with contextlib.redirect_stdout(io.StringIO()):
            code, _ = run([sys.executable, '-c', source], directory=directory,
                          timeout=options.pop('timeout', 3), **options)
        return code, directory, json.loads((directory / 'result.json').read_text())

    def test_no_input_and_exact_failure_status(self):
        code, directory, report = self.execute(
            "import sys; assert sys.stdin.read()==''; print('artifact'); "
            "print('diagnostic',file=sys.stderr);sys.exit(7)")
        self.assertEqual(code, 7)
        self.assertEqual(report['exit_code'], 7)
        self.assertEqual(report['wrapper_exit_code'], 7)
        self.assertEqual((directory / 'stdout.log').read_text(), 'artifact\n')
        self.assertEqual((directory / 'stderr.log').read_text(), 'diagnostic\n')

    def test_output_is_drained_but_storage_is_bounded(self):
        code, directory, report = self.execute(
            "import os; os.write(1,b'x'*200000);os.write(2,b'y'*200000)", log_limit=1234)
        self.assertEqual(code, 0)
        for channel in ['stdout', 'stderr']:
            self.assertEqual((directory / (channel + '.log')).stat().st_size, 1234)
            self.assertEqual(report[channel]['bytes'], 200000)
            self.assertTrue(report[channel]['truncated'])

    def test_timeout_kills_descendants_even_after_leader_exits(self):
        with tempfile.TemporaryDirectory() as temporary:
            marker = Path(temporary) / 'escaped'
            child = f"import time;from pathlib import Path;time.sleep(1.2);Path({str(marker)!r}).write_text('bad')"
            code, _, report = self.execute(
                f"import subprocess,sys;subprocess.Popen([sys.executable,'-c',{child!r}])", timeout=.15)
            self.assertEqual(code, 124)
            self.assertEqual(report['status'], 'timeout')
            self.assertEqual(report['wrapper_exit_code'], 124)
            time.sleep(1.3)
            self.assertFalse(marker.exists())

    def test_signal_is_not_a_success(self):
        code, _, report = self.execute('import os,signal;os.kill(os.getpid(),signal.SIGTERM)')
        self.assertEqual(code, 143)
        self.assertEqual(report['exit_code'], -15)
        self.assertEqual(report['wrapper_exit_code'], 143)

    def test_missing_tool_records_unavailable(self):
        with tempfile.TemporaryDirectory() as temporary, contextlib.redirect_stdout(io.StringIO()):
            code, directory = run(['/nonexistent/luna-test-tool'], directory=temporary)
            self.assertEqual(code, 125)
            self.assertEqual(json.loads((directory / 'result.json').read_text())['status'], 'unavailable')


if __name__ == '__main__':
    unittest.main()
