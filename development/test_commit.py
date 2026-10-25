import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from development import commit


class CommitManifestTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='luna-commit-test-')
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name) / 'repo'
        self.root.mkdir()
        self.git('init', '-b', 'dev')
        self.git('config', 'user.name', 'Commit Test')
        self.git('config', 'user.email', 'test@example.invalid')
        # Do not inherit signing or hook configuration from the user's setup.
        self.git('config', 'commit.gpgsign', 'false')
        self.git('config', 'core.hooksPath', str(Path(self.directory.name) / 'no-hooks'))
        (self.root / 'base').write_text('baseline\n')
        self.git('add', 'base')
        self.git('commit', '-m', 'baseline')
        self.manifest = Path(self.directory.name) / 'manifest'
        self.message = Path(self.directory.name) / 'message'
        self.message.write_text('test: commit explicitly selected work\n')

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, check=True,
                              capture_output=True, text=True, timeout=15).stdout

    def invoke(self, paths):
        self.manifest.write_text('\n'.join(paths) + '\n')
        argv = ['commit.py', '--manifest', str(self.manifest), '--message-file', str(self.message)]
        with patch.object(commit, 'ROOT', self.root), patch('sys.argv', argv):
            return commit.main()

    def test_only_selected_files_are_committed(self):
        (self.root / 'base').write_text('changed\n')
        (self.root / 'keep-untracked').write_text('unrelated\n')
        self.assertEqual(self.invoke(['base']), 0)
        self.assertEqual(self.git('show', 'HEAD:base'), 'changed\n')
        self.assertIn('?? keep-untracked', self.git('status', '--short'))

    def test_existing_index_and_invalid_manifest_are_not_modified(self):
        (self.root / 'base').write_text('staged by someone else\n')
        self.git('add', 'base')
        previous = self.git('diff', '--cached')
        head = self.git('rev-parse', 'HEAD')
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.invoke(['base'])
        self.assertEqual(self.git('diff', '--cached'), previous)
        self.assertEqual(self.git('rev-parse', 'HEAD'), head)
        self.git('commit', '-m', 'save independently staged work')
        for paths in [['../outside'], ['.git/config'], ['base', 'base'], ['.']]:
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                self.invoke(paths)
            self.assertEqual(self.git('diff', '--cached'), '')


if __name__ == '__main__':
    unittest.main()
