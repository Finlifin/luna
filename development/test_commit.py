"""Verify explicit and partial commits only in disposable repositories."""
import contextlib
import difflib
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('commit_entry', Path(__file__).with_name('commit.py'))
entry = importlib.util.module_from_spec(spec)
spec.loader.exec_module(entry)


class CommitTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='luna-commit-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.override = patch.object(entry, 'ROOT', self.root)
        self.override.start()
        self.addCleanup(self.override.stop)
        self.git('init', '-b', 'dev')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.original = ''.join(f'line {i}\n' for i in range(40))
        (self.root / 'shared').write_text(self.original)
        self.git('add', 'shared')
        self.git('commit', '-m', 'base')
        self.head = self.git('rev-parse', 'HEAD')
        self.manifest = self.root / 'manifest'
        self.manifest.write_text('shared\n')
        self.message = self.root / 'message'
        self.message.write_text('selected feature\n')
        self.patch_file = self.root / 'selection.patch'
        self.selected = self.original.replace('line 2\n', 'selected\n')
        self.working = self.selected.replace('line 35\n', 'other feature\n')
        (self.root / 'shared').write_text(self.working)
        self.patch_file.write_text(''.join(difflib.unified_diff(
            self.original.splitlines(True), self.selected.splitlines(True),
            fromfile='a/shared', tofile='b/shared')))

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, text=True,
                              capture_output=True, check=True).stdout

    def run_entry(self, *extra, partial=True):
        args = ['commit.py', '--manifest', str(self.manifest),
                '--message-file', str(self.message)]
        if partial:
            args += ['--patch-file', str(self.patch_file)]
        with patch.object(sys, 'argv', [*args, *extra]), contextlib.redirect_stdout(io.StringIO()), \
                contextlib.redirect_stderr(io.StringIO()):
            return entry.main()

    def test_partial_commit_preserves_other_hunks_and_working_files(self):
        self.assertEqual(self.run_entry(), 0)
        self.assertEqual(self.git('show', 'HEAD:shared'), self.selected)
        self.assertEqual((self.root / 'shared').read_text(), self.working)
        self.assertIn('other feature', self.git('diff'))
        self.assertEqual(self.git('diff', '--cached'), '')

    def test_preview_does_not_change_head_index_or_worktree(self):
        self.assertEqual(self.run_entry('--dry-run'), 0)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.head)
        self.assertEqual(self.git('diff', '--cached'), '')
        self.assertEqual((self.root / 'shared').read_text(), self.working)

    def test_patch_outside_manifest_is_rejected_before_staging(self):
        self.manifest.write_text('different\n')
        with self.assertRaises(SystemExit):
            self.run_entry()
        self.assertEqual(self.git('diff', '--cached'), '')
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.head)

    def test_patch_must_describe_existing_working_content(self):
        (self.root / 'shared').write_text(self.original)
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_entry()
        self.assertEqual(self.git('diff', '--cached'), '')

    def test_existing_staged_work_is_preserved_and_rejected(self):
        self.git('add', 'shared')
        staged = self.git('diff', '--cached')
        with self.assertRaises(SystemExit):
            self.run_entry()
        self.assertEqual(self.git('diff', '--cached'), staged)
        self.assertEqual(self.git('rev-parse', 'HEAD'), self.head)

    def test_whole_file_mode_remains_available(self):
        self.assertEqual(self.run_entry(partial=False), 0)
        self.assertEqual(self.git('show', 'HEAD:shared'), self.working)
        self.assertEqual(self.git('diff'), '')

    def test_partial_commit_includes_new_file_without_staging_other_files(self):
        added = 'new feature\n'
        (self.root / 'new').write_text(added)
        self.manifest.write_text('shared\nnew\n')
        with self.patch_file.open('a') as output:
            output.write(''.join(difflib.unified_diff([], added.splitlines(True),
                                                    fromfile='/dev/null', tofile='b/new')))
        self.run_entry()
        self.assertEqual(self.git('show', 'HEAD:new'), added)
        self.assertEqual(self.git('show', 'HEAD:shared'), self.selected)
        self.assertNotIn('manifest', self.git('ls-tree', '--name-only', 'HEAD'))


if __name__ == '__main__':
    unittest.main()
