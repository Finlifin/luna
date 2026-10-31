import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('rebuild', Path(__file__).with_name('rustc_rebuild.py'))
rebuild = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rebuild)


class RebuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='luna-rustc-rebuild-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest = self.root / 'compiler/elaboration'
        self.manifest.mkdir(parents=True)
        self.source = self.manifest / 'lib.rs'
        self.source.write_text('pub fn answer() -> u32 { 41 }')
        self.target = self.root / 'target'
        self.output = self.target / 'release/build/elaboration/hash/out'
        self.output.mkdir(parents=True)
        self.env = {'LUNA_MANAGED_RUSTC_PRUNE': '1', 'CARGO_PKG_NAME': 'elaboration',
                    'CARGO_MANIFEST_DIR': str(self.manifest)}
        self.args = ['rustc', '--crate-name', 'elaboration', '--crate-type', 'lib',
                     '--emit=metadata,link', '-C', 'extra-filename=-12345678',
                     '--out-dir', str(self.output), str(self.source)]
        self.paths = {'root': self.root, 'target': self.target, 'cwd': self.root}

    def artifact(self, name):
        path = self.output / name
        path.write_bytes(b'previous artifact')
        return path

    def test_only_exact_rebuilt_library_outputs_are_removed(self):
        old = [self.artifact('libelaboration-12345678' + suffix) for suffix in ('.rlib', '.rmeta')]
        preserved = [self.artifact(name) for name in ('libelaboration-87654321.rlib',
                     'libnir-12345678.rlib', 'elaboration-12345678.d', 'notes.txt')]
        self.assertEqual(rebuild.prune(self.args, self.env, **self.paths), 34)
        self.assertTrue(all(not p.exists() for p in old))
        self.assertTrue(all(p.exists() for p in preserved))
        self.assertTrue(self.source.exists())

    def test_test_harness_is_separate_from_library(self):
        executable = self.artifact('elaboration-12345678')
        library = self.artifact('libelaboration-12345678.rlib')
        args = [*self.args, '--test']
        rebuild.prune(args, self.env, **self.paths)
        self.assertFalse(executable.exists())
        self.assertTrue(library.exists())

    def test_queries_metadata_only_foreign_sources_and_packages_are_untouched(self):
        old = self.artifact('libelaboration-12345678.rlib')
        cases = [([*self.args, '--print=file-names'], self.env),
                 ([a.replace('metadata,link', 'metadata') for a in self.args], self.env),
                 ([*self.args, '-oelsewhere'], self.env),
                 ([*self.args, '@options.rsp'], self.env),
                 ([*self.args, '--target=x86_64-unknown-linux-gnu'], self.env),
                 (['custom-wrapper', *self.args], self.env),
                 (self.args, {**self.env, 'LUNA_MANAGED_RUSTC_PRUNE': '0'}),
                 (self.args, {**self.env, 'CARGO_PKG_NAME': 'other'}),
                 (self.args, {**self.env, 'CARGO_MANIFEST_DIR': str(self.root)})]
        foreign = self.root / 'foreign.rs'
        foreign.write_text('')
        cases.append(([str(foreign) if a == str(self.source) else a for a in self.args], self.env))
        for args, env in cases:
            with self.subTest(args=args, env=env):
                self.assertEqual(rebuild.prune(args, env, **self.paths), 0)
                self.assertTrue(old.exists())

    def test_escaping_output_and_symlinks_are_not_followed(self):
        old = self.artifact('libelaboration-12345678.rlib')
        outside = self.root / 'outside'
        outside.mkdir()
        victim = outside / old.name
        victim.write_bytes(b'keep')
        for path in (outside, self.target / '../outside'):
            args = [str(path) if a == str(self.output) else a for a in self.args]
            self.assertEqual(rebuild.prune(args, self.env, **self.paths), 0)
        old.unlink()
        old.symlink_to(victim)
        self.assertEqual(rebuild.prune(self.args, self.env, **self.paths), 0)
        self.assertEqual(victim.read_bytes(), b'keep')
        linked = self.target / 'linked'
        linked.symlink_to(outside, target_is_directory=True)
        args = [str(linked) if a == str(self.output) else a for a in self.args]
        self.assertEqual(rebuild.prune(args, self.env, **self.paths), 0)
        self.assertEqual(victim.read_bytes(), b'keep')

    def test_duplicate_output_options_and_unsafe_suffix_are_not_pruned(self):
        old = self.artifact('libelaboration-12345678.rlib')
        for tail in (['--out-dir', str(self.output)], ['-Cextra-filename=/../../escape']):
            self.assertEqual(rebuild.prune([*self.args, *tail], self.env, **self.paths), 0)
        self.assertTrue(old.exists())

    def test_compiler_command_is_forwarded_even_when_cleanup_fails(self):
        with patch.object(sys, 'argv', ['wrapper', *self.args]), \
             patch.object(rebuild, 'prune', side_effect=PermissionError('denied')), \
             patch.object(rebuild.os, 'execvp') as execute:
            rebuild.main()
        execute.assert_called_once_with(self.args[0], self.args)

    @unittest.skipUnless(shutil.which('rustc'), 'rustc unavailable')
    def test_real_recompile_failure_leaves_no_old_success_and_retry_rebuilds(self):
        first = subprocess.run(self.args, capture_output=True, text=True, check=True)
        old = self.output / 'libelaboration-12345678.rlib'
        self.assertTrue(old.exists(), first.stderr)
        self.source.write_text('pub fn answer() -> u32 { invalid syntax }')
        self.assertGreater(rebuild.prune(self.args, self.env, **self.paths), 0)
        failed = subprocess.run(self.args, capture_output=True, text=True)
        self.assertNotEqual(failed.returncode, 0)
        self.assertFalse(old.exists())
        self.source.write_text('pub fn answer() -> u32 { 42 }')
        subprocess.run(self.args, capture_output=True, text=True, check=True)
        self.assertTrue(old.exists())


if __name__ == '__main__':
    unittest.main()
