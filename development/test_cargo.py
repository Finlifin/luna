import importlib.util
import io
import errno
import os
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("cargo_runner", Path(__file__).with_name("cargo.py"))
cargo = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = cargo
spec.loader.exec_module(cargo)

rust_fmt = cargo.load_rust_fmt()


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

    def test_cargo_arguments_allow_parallel_builds_and_runner_sets_default_jobs(self):
        args = cargo.cargo_arguments(["test", "-p", "luna"], Path("/tmp/test-target"))
        self.assertNotIn("-j", args)
        self.assertNotIn("1", args)
        self.assertIn("--locked", args)
        self.assertIn("--offline", args)
        with patch.object(cargo, "total_memory_bytes", return_value=30 * 1024**3), patch.object(
            cargo.os, "cpu_count", return_value=32
        ):
            self.assertEqual(cargo.default_build_jobs(), 4)
            self.assertEqual(cargo.default_test_threads(), 2)
            self.assertEqual(cargo.default_codegen_units(), 4)
        with patch.object(cargo, "total_memory_bytes", return_value=12 * 1024**3), patch.object(
            cargo.os, "cpu_count", return_value=32
        ):
            self.assertEqual(cargo.default_build_jobs(), 2)
            self.assertEqual(cargo.default_test_threads(), 2)
            self.assertEqual(cargo.default_codegen_units(), 2)
        explicit = cargo.cargo_arguments(["test", "-j", "4", "-p", "luna"], Path("/tmp/test-target"))
        self.assertEqual(explicit[1:4], ["test", "-j", "4"])
        with patch.dict(os.environ, {}, clear=True):
            runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
            self.assertEqual(runner.env["CARGO_BUILD_JOBS"], str(cargo.default_build_jobs()))
            self.assertEqual(runner.env["RUST_TEST_THREADS"], str(cargo.default_test_threads()))
            self.assertEqual(
                runner.env["CARGO_PROFILE_DEV_CODEGEN_UNITS"],
                str(cargo.default_codegen_units()),
            )
            self.assertEqual(
                runner.env["CARGO_PROFILE_TEST_CODEGEN_UNITS"],
                str(cargo.default_codegen_units()),
            )
        with patch.dict(
            os.environ,
            {
                "CARGO_BUILD_JOBS": "3",
                "RUST_TEST_THREADS": "2",
                "CARGO_PROFILE_DEV_CODEGEN_UNITS": "8",
                "CARGO_PROFILE_TEST_CODEGEN_UNITS": "4",
            },
            clear=True,
        ):
            runner = cargo.Runner(Path("/repo"), Path("/tmp/test-target"))
            self.assertEqual(runner.env["CARGO_BUILD_JOBS"], "3")
            self.assertEqual(runner.env["RUST_TEST_THREADS"], "2")
            self.assertEqual(runner.env["CARGO_PROFILE_DEV_CODEGEN_UNITS"], "8")
            self.assertEqual(runner.env["CARGO_PROFILE_TEST_CODEGEN_UNITS"], "4")

    def test_memory_headroom_refuses_launch_when_available_ram_is_low(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.dict(os.environ, {}, clear=True), patch.object(
                cargo, "available_memory_bytes", return_value=1024 * 1024**2
            ):
                runner = cargo.Runner(Path(temporary), Path(temporary) / "target")
                runner.reclaim = Mock()
                runner.execute = Mock(return_value=(0, False))
                stderr = io.StringIO()
                with patch.object(cargo.sys, "stderr", stderr):
                    status = runner.run(["--no-fmt", "check", "-p", "example"])
                self.assertEqual(status, 75)
                runner.execute.assert_not_called()
                self.assertIn("refusing to start cargo", stderr.getvalue())

            with patch.dict(os.environ, {"LUNA_MIN_AVAILABLE_MEM_MIB": "0"}, clear=True), patch.object(
                cargo, "available_memory_bytes", return_value=1024 * 1024**2
            ):
                runner = cargo.Runner(Path(temporary), Path(temporary) / "target")
                runner.reclaim = Mock()
                runner.execute = Mock(return_value=(0, False))
                status = runner.run(["--no-fmt", "check", "-p", "example"])
                self.assertEqual(status, 0)
                runner.execute.assert_called_once()

            with patch.dict(os.environ, {}, clear=True), patch.object(
                cargo, "available_memory_bytes", return_value=8 * 1024**3
            ):
                runner = cargo.Runner(Path(temporary), Path(temporary) / "target")
                runner.reclaim = Mock()
                runner.execute = Mock(return_value=(0, False))
                status = runner.run(["--no-fmt", "check", "-p", "example"])
                self.assertEqual(status, 0)
                runner.execute.assert_called_once()

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


class RustFmtItemSpacingTests(unittest.TestCase):
    def test_inserts_blank_lines_between_top_level_and_nested_items(self):
        source = "\n".join([
            "use a::One;",
            "use b::Two;",
            "struct Foo {",
            "    x: i32,",
            "    y: i32,",
            "}",
            "impl Foo {",
            "    fn first(&self) -> i32 {",
            "        let a = 1;",
            "        let b = 2;",
            "        a + b",
            "    }",
            "    fn second(&self) -> i32 {",
            "        42",
            "    }",
            "}",
            "const LIMIT: usize = {",
            "    let n = 4;",
            "    n * 2",
            "};",
            "type Pair = (i32, i32);",
            "",
        ])
        expected = "\n".join([
            "use a::One;",
            "use b::Two;",
            "",
            "struct Foo {",
            "    x: i32,",
            "    y: i32,",
            "}",
            "",
            "impl Foo {",
            "    fn first(&self) -> i32 {",
            "        let a = 1;",
            "        let b = 2;",
            "        a + b",
            "    }",
            "",
            "    fn second(&self) -> i32 {",
            "        42",
            "    }",
            "}",
            "",
            "const LIMIT: usize = {",
            "    let n = 4;",
            "    n * 2",
            "};",
            "",
            "type Pair = (i32, i32);",
            "",
        ])
        formatted = rust_fmt.ensure_blank_lines_between_items(source)
        self.assertEqual(formatted, expected)
        self.assertEqual(rust_fmt.ensure_blank_lines_between_items(formatted), formatted)

    def test_keeps_docs_attributes_mod_decls_and_macro_bodies_intact(self):
        source = "\n".join([
            "//! Crate header.",
            "#![allow(unused)]",
            "mod alpha;",
            "mod beta;",
            "/// Docs stay attached.",
            "#[derive(Debug)]",
            "enum Kind {",
            "    One,",
            "    Two { inner: i32 },",
            "}",
            "// Leading note.",
            "#[inline]",
            "extern \"C\" fn c_entry() {",
            "    let raw = r#\"}\nfn fake() {}\"#;",
            "    /* /* } */ */",
            "    let ch = '}';",
            "}",
            "trait Ops {",
            "    type Out;",
            "    fn apply<'a>(&'a self, x: i32) -> i32;",
            "}",
            "macro_rules! mk {",
            "    ($name:ident) => { fn $name() {} };",
            "}",
            "fn after_macro() {}",
            "",
        ])
        expected = "\n".join([
            "//! Crate header.",
            "#![allow(unused)]",
            "mod alpha;",
            "mod beta;",
            "",
            "/// Docs stay attached.",
            "#[derive(Debug)]",
            "enum Kind {",
            "    One,",
            "    Two { inner: i32 },",
            "}",
            "",
            "// Leading note.",
            "#[inline]",
            "extern \"C\" fn c_entry() {",
            "    let raw = r#\"}\nfn fake() {}\"#;",
            "    /* /* } */ */",
            "    let ch = '}';",
            "}",
            "",
            "trait Ops {",
            "    type Out;",
            "",
            "    fn apply<'a>(&'a self, x: i32) -> i32;",
            "}",
            "",
            "macro_rules! mk {",
            "    ($name:ident) => { fn $name() {} };",
            "}",
            "",
            "fn after_macro() {}",
            "",
        ])
        formatted = rust_fmt.ensure_blank_lines_between_items(source)
        self.assertEqual(formatted, expected)
        self.assertEqual(rust_fmt.ensure_blank_lines_between_items(formatted), formatted)

    def test_const_generics_inline_const_and_attributed_mods(self):
        source = "\n".join([
            "mod first;",
            "#[cfg(test)]",
            "mod second;",
            "impl<const N: usize> Wrapper<const { 1 + 1 }, N> {",
            "    const unsafe fn tag(&self) -> usize { N }",
            "    fn next(&self) -> usize { N }",
            "}",
            "",
        ])
        expected = "\n".join([
            "mod first;",
            "",
            "#[cfg(test)]",
            "mod second;",
            "",
            "impl<const N: usize> Wrapper<const { 1 + 1 }, N> {",
            "    const unsafe fn tag(&self) -> usize { N }",
            "",
            "    fn next(&self) -> usize { N }",
            "}",
            "",
        ])
        self.assertEqual(rust_fmt.ensure_blank_lines_between_items(source), expected)


class CargoAutoFmtTests(unittest.TestCase):
    def _init_git_repo(self, root: Path) -> None:
        env = {
            **os.environ,
            "GIT_AUTHOR_NAME": "Test",
            "GIT_AUTHOR_EMAIL": "test@example.com",
            "GIT_COMMITTER_NAME": "Test",
            "GIT_COMMITTER_EMAIL": "test@example.com",
        }
        cargo.subprocess.run(["git", "init"], cwd=root, env=env, check=True, capture_output=True)
        (root / "rustfmt.toml").write_text(
            'edition = "2024"\nignore = ["src/ignored.rs"]\n',
            encoding="utf-8",
        )
        (root / "src").mkdir()
        (root / "src/clean.rs").write_text("fn clean() {}\n", encoding="utf-8")
        (root / "src/deleted.rs").write_text("fn gone() {}\n", encoding="utf-8")
        cargo.subprocess.run(["git", "add", "."], cwd=root, env=env, check=True, capture_output=True)
        cargo.subprocess.run(
            ["git", "commit", "-m", "init"], cwd=root, env=env, check=True, capture_output=True
        )

    def test_cargo_arguments_accept_leading_no_fmt_and_reject_fmt_as_cargo_subcmd(self):
        args = cargo.cargo_arguments(["--no-fmt", "test", "-p", "luna"], Path("/tmp/test-target"))
        self.assertEqual(args[:2], ["cargo", "test"])
        with self.assertRaises(ValueError):
            cargo.cargo_arguments(["fmt"], Path("/tmp/test-target"))

    def test_modified_rust_files_skips_ignored_clean_and_deleted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self._init_git_repo(root)
            (root / "src/dirty.rs").write_text("fn a() {}\nfn b() {}\n", encoding="utf-8")
            (root / "src/ignored.rs").write_text("concat!(a\n", encoding="utf-8")
            (root / "src/deleted.rs").unlink()
            runner = cargo.Runner(root, root / "target")
            files = runner.modified_rust_files()
            self.assertEqual(files, [(root / "src/dirty.rs").resolve()])

    def test_run_formats_modified_files_before_build_and_respects_opt_outs(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self._init_git_repo(root)
            dirty = root / "src/dirty.rs"
            raw = "fn a() {}\nfn b() {}\n"
            dirty.write_text(raw, encoding="utf-8")

            # Opt-out via env var leaves modified file untouched.
            with patch.dict(os.environ, {"LUNA_SKIP_AUTO_FMT": "1"}):
                runner_env = cargo.Runner(root, root / "target")
                runner_env.reclaim = Mock()
                runner_env.execute = Mock(return_value=(0, False))
                self.assertEqual(runner_env.run(["check", "-p", "foo"]), 0)
            self.assertEqual(dirty.read_text(encoding="utf-8"), raw)

            # Opt-out via --no-fmt flag leaves modified file untouched.
            runner_flag = cargo.Runner(root, root / "target")
            runner_flag.reclaim = Mock()
            runner_flag.execute = Mock(return_value=(0, False))
            self.assertEqual(runner_flag.run(["--no-fmt", "check", "-p", "foo"]), 0)
            self.assertEqual(dirty.read_text(encoding="utf-8"), raw)

            # Without opt-out, running `check` auto-formats dirty.rs before executing Cargo.
            observed = []
            runner = cargo.Runner(root, root / "target")
            runner.reclaim = Mock()
            runner.execute = lambda cmd: (observed.append(dirty.read_text(encoding="utf-8")) or 0, False)
            self.assertEqual(runner.run(["check", "-p", "foo"]), 0)
            self.assertEqual(len(observed), 1)
            self.assertEqual(observed[0], "fn a() {}\n\nfn b() {}\n")
            self.assertEqual(dirty.read_text(encoding="utf-8"), "fn a() {}\n\nfn b() {}\n")
            self.assertEqual((root / "src/clean.rs").read_text(encoding="utf-8"), "fn clean() {}\n")

    def test_auto_fmt_syntax_error_is_nonfatal(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self._init_git_repo(root)
            broken = root / "src/broken.rs"
            broken.write_text("fn broken( {", encoding="utf-8")
            runner = cargo.Runner(root, root / "target")
            runner.reclaim = Mock()
            runner.execute = Mock(return_value=(101, False))
            stderr_buf = io.StringIO()
            with patch.object(cargo.sys, "stderr", stderr_buf):
                self.assertEqual(runner.run(["build"]), 101)
            runner.execute.assert_called_once()
            self.assertIn("rustfmt-auto: warning", stderr_buf.getvalue())

    def test_fmt_subcommand_check_and_fix(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self._init_git_repo(root)
            dirty = root / "src/dirty.rs"
            dirty.write_text("fn a() {}\nfn b() {}\n", encoding="utf-8")
            runner = cargo.Runner(root, root / "target")
            stderr_buf = io.StringIO()
            with patch.object(cargo.sys, "stderr", stderr_buf):
                self.assertNotEqual(runner.run(["fmt", "--check"]), 0)
            self.assertEqual(runner.run(["fmt"]), 0)
            self.assertEqual(dirty.read_text(encoding="utf-8"), "fn a() {}\n\nfn b() {}\n")
            self.assertEqual(runner.run(["fmt", "--check"]), 0)


class CargoSourceRootTests(unittest.TestCase):
    def _write_workspace(self, root: Path, members: dict[str, str]) -> None:
        member_list = ", ".join(f'"{rel}"' for rel in members)
        (root / "Cargo.toml").write_text(
            f"[workspace]\nmembers = [{member_list}]\n",
            encoding="utf-8",
        )
        for rel, pkg_name in members.items():
            pkg_dir = root / rel
            pkg_dir.mkdir(parents=True, exist_ok=True)
            (pkg_dir / "Cargo.toml").write_text(
                f'[package]\nname = "{pkg_name}"\nversion = "0.1.0"\n',
                encoding="utf-8",
            )

    def test_manifest_root_parsing_ignores_arguments_after_double_dash(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary).resolve()
            self.assertEqual(cargo.manifest_root(base, ["check", "-p", "luna"]), base)
            self.assertEqual(
                cargo.manifest_root(base, ["run", "-p", "luna", "--", "--manifest-path", "/other/Cargo.toml"]),
                base,
            )
            snap_dir = base / "snap"
            snap_dir.mkdir()
            self.assertEqual(
                cargo.manifest_root(base, ["test", "--manifest-path", "snap/Cargo.toml"]),
                snap_dir.resolve(),
            )
            self.assertEqual(
                cargo.manifest_root(base, ["--no-fmt", "build", f"--manifest-path={snap_dir / 'Cargo.toml'}"]),
                snap_dir.resolve(),
            )

    def test_same_root_reuses_cache_and_cross_root_cleans_workspace_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            temp = Path(temporary).resolve()
            primary = temp / "primary"
            snapshot = temp / "snapshot"
            target = temp / "target"
            primary.mkdir()
            snapshot.mkdir()
            (target / "debug").mkdir(parents=True)
            self._write_workspace(primary, {"compiler/elab": "elaboration", "compiler/luna": "luna"})
            self._write_workspace(
                snapshot,
                {"compiler/elab": "elaboration", "compiler/luna": "luna", "compiler/extra": "snap_only"},
            )

            runner = cargo.Runner(primary, target)
            runner.reclaim = Mock()
            runner.execute = Mock(return_value=(0, False))
            clean_calls = []

            def fake_clean(*, full=False, packages=cargo.REBUILDABLE_PACKAGES, manifest_root=None):
                clean_calls.append((full, packages, manifest_root))

            runner.clean = fake_clean

            # Pre-populated target without a stamp cleans once for primary and records stamp.
            stderr_buf = io.StringIO()
            with patch.object(cargo.sys, "stderr", stderr_buf):
                self.assertEqual(runner.run(["--no-fmt", "test", "-p", "luna"]), 0)
            self.assertEqual(clean_calls, [(False, ("elaboration", "luna"), None)])
            self.assertEqual(runner.source_root_state_path().read_text(encoding="utf-8").strip(), str(primary))

            # Subsequent run in the same root reuses the cache with zero cleans.
            clean_calls.clear()
            self.assertEqual(runner.run(["--no-fmt", "check", "-p", "luna"]), 0)
            self.assertEqual(clean_calls, [])

            # Switching to snapshot via --manifest-path cleans snapshot workspace packages without full wipe.
            with patch.object(cargo.sys, "stderr", stderr_buf):
                self.assertEqual(
                    runner.run(["--no-fmt", "test", "--manifest-path", str(snapshot / "Cargo.toml")]),
                    0,
                )
            self.assertEqual(
                clean_calls,
                [(False, ("elaboration", "luna", "snap_only"), snapshot)],
            )
            self.assertEqual(runner.source_root_state_path().read_text(encoding="utf-8").strip(), str(snapshot))

            # Switching back to primary cleans primary packages plus extra snapshot-only package.
            clean_calls.clear()
            with patch.object(cargo.sys, "stderr", stderr_buf):
                self.assertEqual(runner.run(["--no-fmt", "check", "-p", "luna"]), 0)
            self.assertEqual(
                clean_calls,
                [
                    (False, ("elaboration", "luna"), None),
                    (False, ("snap_only",), snapshot),
                ],
            )
            self.assertEqual(runner.source_root_state_path().read_text(encoding="utf-8").strip(), str(primary))

            # Full `clean` removes the source-root stamp, whereas `-p` clean retains it.
            self.assertEqual(runner.run(["clean", "-p", "luna"]), 0)
            self.assertTrue(runner.source_root_state_path().exists())
            self.assertEqual(runner.run(["clean"]), 0)
            self.assertFalse(runner.source_root_state_path().exists())

    def test_auto_fmt_skipped_when_manifest_path_points_to_external_root(self):
        with tempfile.TemporaryDirectory() as temporary:
            temp = Path(temporary).resolve()
            primary = temp / "primary"
            snapshot = temp / "snapshot"
            primary.mkdir()
            snapshot.mkdir()
            CargoAutoFmtTests()._init_git_repo(primary)
            self._write_workspace(snapshot, {"compiler/luna": "luna"})
            dirty = primary / "src/dirty.rs"
            raw = "fn a() {}\nfn b() {}\n"
            dirty.write_text(raw, encoding="utf-8")

            runner = cargo.Runner(primary, temp / "target")
            runner.reclaim = Mock()
            runner.execute = Mock(return_value=(0, False))
            self.assertEqual(
                runner.run(["test", f"--manifest-path={snapshot / 'Cargo.toml'}"]),
                0,
            )
            self.assertEqual(dirty.read_text(encoding="utf-8"), raw)


if __name__ == "__main__":
    unittest.main()
