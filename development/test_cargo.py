import importlib.util
import io
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("cargo_runner", Path(__file__).with_name("cargo.py"))
cargo = importlib.util.module_from_spec(spec)
spec.loader.exec_module(cargo)


class CargoCacheTests(unittest.TestCase):
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
            command = execute.call_args.args[0]
            self.assertEqual(command, ["cargo", "clean", "--target-dir", "/tmp/test-target", "-p", "elaboration", "-p", "luna"])
            runner.clean(full=True)
            self.assertEqual(execute.call_args.args[0], ["cargo", "clean", "--target-dir", "/tmp/test-target"])

    def test_pressure_cleans_packages_before_full_cache(self):
        runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
        runner.pressure = Mock(side_effect=[True, True, True])
        runner.clean = Mock()
        runner.reclaim()
        self.assertEqual(runner.clean.call_args_list, [unittest.mock.call(packages=("luna",)), unittest.mock.call(packages=("elaboration",)), unittest.mock.call(full=True)])

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

    def test_repeated_storage_failure_cannot_loop(self):
        runner, status = self.run_fake_build([(101, True), (101, True)])
        self.assertEqual(status, 101)
        self.assertEqual(runner.execute.call_count, 2)
        self.assertEqual(runner.reclaim.call_args_list[-2:], [unittest.mock.call(force=True), unittest.mock.call()])


if __name__ == "__main__":
    unittest.main()
