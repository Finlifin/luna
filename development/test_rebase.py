"""Exercise history rewriting only in disposable repositories."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import rebase


class RebaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='luna-rebase-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.override = patch.object(rebase, 'ROOT', self.root)
        self.override.start()
        self.addCleanup(self.override.stop)
        self.git('init', '-b', 'dev')
        self.git('config', 'user.name', 'Test')
        self.git('config', 'user.email', 'test@example.invalid')
        self.git('config', 'commit.gpgsign', 'false')
        self.commit('base')
        self.base = rebase.oid('HEAD')
        self.commit('feature')
        self.commit('fix')
        self.head = rebase.oid('HEAD')
        self.plan = self.root / 'plan.json'
        self.run_command('plan', '--base', self.base, '--output', str(self.plan))
        data = json.loads(self.plan.read_text())
        data['commits'][1]['action'] = 'fixup'
        self.plan.write_text(json.dumps(data))

    def git(self, *args):
        return subprocess.run(['git', *args], cwd=self.root, check=True,
                              capture_output=True, text=True).stdout

    def commit(self, text):
        (self.root / 'file').write_text(text)
        self.git('add', 'file')
        self.git('commit', '-m', text)

    def run_command(self, *args):
        with patch.object(sys, 'argv', ['rebase.py', *args]), contextlib.redirect_stdout(io.StringIO()):
            rebase.main()

    def test_fixup_preserves_tree_untracked_and_backup(self):
        self.run_command('start', '--plan', str(self.plan))
        self.assertEqual(self.git('rev-list', '--count', self.base + '..HEAD').strip(), '1')
        self.assertEqual(self.git('rev-parse', 'HEAD^{tree}'), self.git('rev-parse', self.head + '^{tree}'))
        self.assertEqual(self.git('for-each-ref', '--format=%(objectname)', 'refs/backup/').strip(), self.head)
        self.assertTrue(self.plan.exists())
        self.assertFalse(rebase.git_path('luna-history-rebase.json').exists())

    def test_dirty_work_is_preserved(self):
        (self.root / 'file').write_text('unfinished')
        with self.assertRaisesRegex(ValueError, 'dirty'):
            self.run_command('start', '--plan', str(self.plan))
        self.assertEqual((self.root / 'file').read_text(), 'unfinished')
        self.assertEqual(rebase.oid('HEAD'), self.head)

    def test_stale_or_destructive_plans_rejected(self):
        data = json.loads(self.plan.read_text())
        data['commits'][1]['action'] = 'exec'
        with self.assertRaises(ValueError):
            rebase.validate(data)
        data['commits'].pop()
        with self.assertRaises(ValueError):
            rebase.validate(data)
        self.commit('new work')
        with self.assertRaisesRegex(ValueError, 'HEAD changed'):
            rebase.validate(json.loads(self.plan.read_text()))

    def test_preview_does_not_change_refs(self):
        refs = self.git('show-ref')
        self.run_command('preview', '--plan', str(self.plan))
        self.assertEqual(self.git('show-ref'), refs)
        self.assertFalse(rebase.git_path('luna-history-rebase.json').exists())


if __name__ == '__main__':
    unittest.main()
