import importlib.util
import io
import errno
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("cargo_runner", Path(__file__).with_name("cargo.py"))
cargo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo)


class CargoCacheTests(unittest.TestCase):
    def test_quota_cleanup_learns_limit_after_freeing_storage(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            runner.cache_size = Mock(return_value=512 * 1024**2)
            events = []
            runner.clean = lambda **kwargs: events.append(runner.cache_limit())
            runner.reclaim(force=True)
            self.assertEqual(events, [cargo.MAX_CACHE, cargo.MAX_CACHE])
            restarted = cargo.Runner(runner.root, runner.target)
            self.assertEqual(restarted.cache_limit(), 384 * 1024**2)
            restarted.cache_size = Mock(return_value=400 * 1024**2)
            with patch.object(cargo.shutil, 'disk_usage', return_value=Mock(free=10 * cargo.MIN_FREE)):
                self.assertTrue(restarted.pressure())
                restarted.cache_size.return_value = 300 * 1024**2
                self.assertFalse(restarted.pressure())

    def test_learned_limit_never_grows_and_has_a_floor(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            runner.remember_pressure(512 * 1024**2)
            limit = runner.cache_limit()
            runner.remember_pressure(cargo.MAX_CACHE * 2)
            self.assertEqual(runner.cache_limit(), limit)
            runner.remember_pressure(1)
            self.assertEqual(runner.cache_limit(), cargo.MIN_CACHE)
            runner.remember_pressure(0)
            self.assertEqual(runner.cache_limit(), cargo.MIN_CACHE)

    def test_invalid_cache_state_falls_back_and_write_failure_is_nonfatal(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            state = runner.target.with_name('target-cache-limit')
            self.assertEqual(runner.cache_limit(), cargo.MAX_CACHE)
            for text in ['broken', '-1', '0', str(cargo.MAX_CACHE + 1)]:
                state.write_text(text)
                self.assertEqual(runner.cache_limit(), cargo.MAX_CACHE)
            with patch.object(Path, 'write_text', side_effect=OSError(errno.EDQUOT, 'full')):
                runner.remember_pressure(512 * 1024**2)
            self.assertEqual(runner.cache_limit(), cargo.MAX_CACHE)

    def test_failed_size_probe_does_not_prevent_quota_cleanup(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            runner.cache_size = Mock(side_effect=OSError(errno.EDQUOT, 'full'))
            runner.clean = Mock()
            runner.reclaim(force=True)
            self.assertEqual(runner.clean.call_count, 2)
            self.assertEqual(runner.cache_limit(), cargo.MAX_CACHE)

    def test_application_arguments_are_not_reinterpreted(self):
        args = cargo.cargo_arguments(["run", "-p", "luna", "--", "--target-dir=application-data"], Path("/tmp/test-target"))
        self.assertEqual(args[-2:], ["--", "--target-dir=application-data"])
        self.assertIn("--offline", args)
        self.assertIn("--locked", args)
        with self.assertRaises(ValueError):
            cargo.cargo_arguments(["test", "--target-dir", "/source"], Path("/tmp/test-target"))

    def test_cleanup_is_scoped_to_managed_target(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        with patch.object(cargo.subprocess, "run") as execute:
            runner.clean()
            command = ["cargo", "clean", "--target-dir", "/tmp/test-target", "-p", "elaboration", "-p", "luna"]
            self.assertEqual([call.args[0] for call in execute.call_args_list],
                             [command, command + ["--release"]])
            execute.reset_mock()
            runner.clean(full=True)
            self.assertEqual(execute.call_count, 1)
            self.assertEqual(execute.call_args.args[0], ["cargo", "clean", "--target-dir", "/tmp/test-target"])

    def test_pressure_cleans_packages_before_full_cache(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(side_effect=[True, True, True])
        runner.clean = Mock()
        runner.reclaim()
        self.assertEqual(runner.clean.call_args_list, [unittest.mock.call(packages=("luna",)), unittest.mock.call(packages=("elaboration",)), unittest.mock.call(full=True)])

    def test_cleanup_still_runs_when_its_status_message_cannot_be_written(self):
        runner = cargo.Runner(Path('/repo'), Path('/tmp/test-target'))
        broken = Mock()
        broken.write.side_effect = OSError(errno.EDQUOT, 'log is full')
        with patch.object(cargo.sys, 'stderr', broken), patch.object(cargo.subprocess, 'run') as execute:
            runner.clean(packages=('luna',))
        self.assertEqual(execute.call_count, 2)
        self.assertIn('--release', execute.call_args.args[0])

    def run_fake_build(self, outcomes):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        runner = cargo.Runner(Path(temporary.name), Path(temporary.name) / "target")
        runner.reclaim = Mock()
        runner.execute = Mock(side_effect=outcomes)
        status = runner.run(["test", "-p", "example"])
        return runner, status

    def test_hidden_quota_failure_reclaims_full_target(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(return_value=False)
        runner.clean = Mock()
        runner.reclaim(force=True)
        self.assertEqual(runner.clean.call_args_list,
                         [unittest.mock.call(), unittest.mock.call(full=True)])

    def test_normal_pressure_keeps_dependencies_when_package_cleanup_suffices(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(side_effect=[True, False])
        runner.clean = Mock()
        runner.reclaim()
        runner.clean.assert_called_once_with(packages=("luna",))

    def test_second_cleanup_keeps_dependency_cache(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(side_effect=[True, True, False])
        runner.clean = Mock()
        runner.reclaim()
        self.assertEqual(runner.clean.call_args_list,
                         [unittest.mock.call(packages=("luna",)),
                          unittest.mock.call(packages=("elaboration",))])

    def test_no_pressure_keeps_all_build_outputs(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(return_value=False)
        runner.clean = Mock()
        runner.reclaim()
        runner.clean.assert_not_called()

    def test_storage_failure_recovers_in_the_running_process(self):
        runner, status = self.run_fake_build([(101, True), (0, False)])
        self.assertEqual(status, 0)
        self.assertEqual(runner.execute.call_count, 2)
        self.assertEqual(runner.reclaim.call_args_list, [unittest.mock.call(), unittest.mock.call(force=True), unittest.mock.call()])

    def test_real_child_preserves_streams_and_reports_storage_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / "target")
            stdout, stderr = io.BytesIO(), io.BytesIO()
            with patch.object(cargo.sys, "stdout", Mock(buffer=stdout)), patch.object(cargo.sys, "stderr", Mock(buffer=stderr)):
                status, quota = runner.execute([sys.executable, "-c", "import sys; print('(artifact)'); print('disk quota exceeded', file=sys.stderr); sys.exit(101)"])
            self.assertEqual((status, quota), (101, True))
            self.assertEqual(stdout.getvalue(), b"(artifact)\n")
            self.assertEqual(stderr.getvalue(), b"disk quota exceeded\n")

    def test_real_test_failure_is_not_retried_or_hidden(self):
        runner, status = self.run_fake_build([(101, False)])
        self.assertEqual(status, 101)
        self.assertEqual(runner.execute.call_count, 1)

    def test_large_newline_free_output_is_streamed_and_split_quota_is_detected(self):
        class Recording(io.BytesIO):
            largest = 0
            def write(self, data):
                self.largest = max(self.largest, len(data))
                return super().write(data)
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            stdout, stderr = Recording(), Recording()
            payload = b'x' * (64 * 1024 - 4) + b'QUOTA EXCEEDED' + b'y' * (1024 * 1024)
            script = "import sys; sys.stdout.buffer.write(b'x'*(65536-4)+b'QUOTA EXCEEDED'+b'y'*1048576); sys.stderr.buffer.write(b'z'*1048576); sys.exit(101)"
            with patch.object(cargo.sys, 'stdout', Mock(buffer=stdout)), patch.object(cargo.sys, 'stderr', Mock(buffer=stderr)):
                self.assertEqual(runner.execute([sys.executable, '-c', script]), (101, True))
            self.assertEqual(stdout.getvalue(), payload)
            self.assertEqual(stderr.getvalue(), b'z' * 1048576)
            self.assertLessEqual(max(stdout.largest, stderr.largest), 64 * 1024)

    def test_forwarding_failure_drains_child_and_never_reports_success(self):
        for number in (errno.EDQUOT, errno.ENOSPC, errno.EPIPE):
            for stream in ('stdout', 'stderr'):
                with self.subTest(errno=number, stream=stream), tempfile.TemporaryDirectory() as temporary:
                    runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
                    failed = Mock()
                    failed.write.side_effect = OSError(number, 'injected forwarding failure')
                    other = io.BytesIO()
                    outputs = {stream: failed, 'stderr' if stream == 'stdout' else 'stdout': other}
                    # More than the pipe capacity, on both streams. A stopped
                    # reader would deadlock this otherwise successful child.
                    script = "import sys; sys.stdout.buffer.write(b'a'*1048576); sys.stderr.buffer.write(b'b'*1048576)"
                    with patch.object(cargo.sys, 'stdout', Mock(buffer=outputs['stdout'])), patch.object(cargo.sys, 'stderr', Mock(buffer=outputs['stderr'])):
                        with self.assertRaises(OSError) as caught:
                            runner.execute([sys.executable, '-c', script])
                    self.assertEqual(caught.exception.errno, number)
                    self.assertIn(stream, str(caught.exception))
                    failed.write.assert_called_once()
                    self.assertEqual(len(other.getvalue()), 1048576)

    def test_evidence_quota_failure_reclaims_without_repeating_or_publishing(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            runner.reclaim = Mock()
            runner.execute = Mock(side_effect=OSError(errno.EDQUOT, 'evidence unavailable'))
            publish = Mock()
            with self.assertRaises(OSError):
                runner.run(['run', '--', 'application'], on_success=publish)
            runner.execute.assert_called_once()
            publish.assert_not_called()
            self.assertEqual(runner.reclaim.call_args_list,
                             [unittest.mock.call(), unittest.mock.call(force=True), unittest.mock.call()])

    def test_repeated_storage_failure_cannot_loop(self):
        runner, status = self.run_fake_build([(101, True), (101, True)])
        self.assertEqual(status, 101)
        self.assertEqual(runner.execute.call_count, 2)
        self.assertEqual(runner.reclaim.call_args_list[-2:], [unittest.mock.call(force=True), unittest.mock.call()])

    def test_success_publication_precedes_cleanup_and_failed_build_cannot_publish(self):
        for status in (0, 101):
            with tempfile.TemporaryDirectory() as temporary:
                runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
                events = []
                runner.reclaim = lambda: events.append('reclaim')
                runner.execute = lambda command: (events.append('execute') or status, False)
                result = runner.run(['build'], on_success=lambda: events.append('publish'))
                self.assertEqual(result, status)
                self.assertEqual(events, ['reclaim', 'execute'] +
                                 (['publish'] if status == 0 else []) + ['reclaim'])

    def test_failed_publication_still_reclaims_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            runner = cargo.Runner(Path(temporary), Path(temporary) / 'target')
            runner.reclaim = Mock()
            runner.execute = Mock(return_value=(0, False))
            with self.assertRaises(OSError):
                runner.run(['build'], on_success=Mock(side_effect=OSError('disk full')))
            self.assertEqual(runner.reclaim.call_count, 2)


if __name__ == "__main__":
    unittest.main()
