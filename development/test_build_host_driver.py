from pathlib import Path
import errno
import tempfile
import unittest
from unittest.mock import patch

import build_host_driver as driver


class HostDriverTests(unittest.TestCase):
    def test_release_snapshot_never_uses_stale_debug_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for profile in ('debug', 'release'):
                (root / profile).mkdir()
                (root / profile / 'luna').write_bytes(profile.encode())
            destination = root / 'snapshot'

            def build(runner, arguments, *, on_success):
                self.assertEqual(arguments, ['build', '-p', 'luna', '--bin', 'luna', '--release'])
                on_success()
                return 0

            with patch.object(driver, 'TARGET', root), patch.object(driver.Runner, 'run', build):
                self.assertEqual(driver.build(destination, release=True), 0)
                self.assertEqual(destination.read_bytes(), b'release')
                destination.unlink()
                (root / 'release/luna').unlink()
                with self.assertRaises(FileNotFoundError):
                    driver.build(destination, release=True)
                self.assertFalse(destination.exists())
                self.assertEqual((root / 'debug/luna').read_bytes(), b'debug')

    def test_copy_storage_failure_reclaims_cache_and_reuses_open_artifact(self):
        for code in (errno.ENOSPC, errno.EDQUOT):
            with self.subTest(errno=code), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / 'debug/luna'
                source.parent.mkdir()
                source.write_bytes(b'complete compiler image')
                destination = root / 'snapshot'
                original_copy = driver.shutil.copyfileobj
                copies = []

                def copy(input_file, output_file):
                    copies.append(input_file.tell())
                    if len(copies) == 1:
                        output_file.write(input_file.read(5))
                        raise OSError(code, 'storage exhausted')
                    original_copy(input_file, output_file)

                def reclaim(runner, *, force):
                    self.assertTrue(force)
                    self.assertFalse(destination.exists())
                    source.unlink()

                with patch.object(driver, 'TARGET', root), patch.object(
                        driver.Runner, 'run', lambda self, args, *, on_success: on_success() or 0), \
                        patch.object(driver.Runner, 'reclaim', reclaim), \
                        patch.object(driver.shutil, 'copyfileobj', side_effect=copy):
                    self.assertEqual(driver.build(destination), 0)
                self.assertEqual(copies, [0, 0])
                self.assertFalse(source.exists())
                self.assertEqual(destination.read_bytes(), b'complete compiler image')
                self.assertEqual(destination.stat().st_mode & 0o777, 0o700)

    def test_persistent_quota_and_other_copy_errors_leave_no_partial_driver(self):
        for code, attempts, cleanups in ((errno.EDQUOT, 2, 1), (errno.EIO, 1, 0),
                                        (errno.EACCES, 1, 0)):
            with self.subTest(errno=code), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                source = root / 'debug/luna'
                source.parent.mkdir()
                source.write_bytes(b'compiler')
                destination = root / 'snapshot'

                def copy(input_file, output_file):
                    output_file.write(input_file.read(1))
                    raise OSError(code, 'injected failure')

                with patch.object(driver, 'TARGET', root), patch.object(
                        driver.Runner, 'run', lambda self, args, *, on_success: on_success()), \
                        patch.object(driver.Runner, 'reclaim') as reclaim, \
                        patch.object(driver.shutil, 'copyfileobj', side_effect=copy) as copying:
                    with self.assertRaises(OSError) as raised:
                        driver.build(destination)
                self.assertEqual(raised.exception.errno, code)
                self.assertEqual(copying.call_count, attempts)
                self.assertEqual(reclaim.call_count, cleanups)
                self.assertFalse(destination.exists())

    def test_dangling_destination_symlink_is_preserved_without_build(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'snapshot'
            destination.symlink_to('missing')
            with patch.object(driver.Runner, 'run') as build:
                with self.assertRaises(FileExistsError):
                    driver.build(destination)
                build.assert_not_called()
            self.assertTrue(destination.is_symlink())

    def test_snapshot_survives_cache_reclamation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = root / 'debug/luna'
            source.parent.mkdir()
            source.write_bytes(b'compiled executable')
            destination = root / 'snapshot'

            def build(runner, arguments, *, on_success):
                self.assertEqual(arguments, ['build', '-p', 'luna', '--bin', 'luna'])
                on_success()
                source.unlink()
                return 0

            with patch.object(driver, 'TARGET', root), patch.object(driver.Runner, 'run', build):
                self.assertEqual(driver.build(destination), 0)
            self.assertEqual(destination.read_bytes(), b'compiled executable')
            self.assertEqual(destination.stat().st_mode & 0o777, 0o700)

    def test_existing_destination_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            destination = Path(temporary) / 'snapshot'
            destination.write_bytes(b'old executable')
            with patch.object(driver.Runner, 'run',
                              lambda self, args, *, on_success: on_success()):
                with self.assertRaises(FileExistsError):
                    driver.build(destination)
            self.assertEqual(destination.read_bytes(), b'old executable')

    def test_missing_artifact_leaves_no_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / 'snapshot'
            with patch.object(driver, 'TARGET', root), patch.object(
                    driver.Runner, 'run', lambda self, args, *, on_success: on_success()):
                with self.assertRaises(FileNotFoundError):
                    driver.build(destination)
            self.assertFalse(destination.exists())


if __name__ == '__main__':
    unittest.main()
