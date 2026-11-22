import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import comptime_bench


class BenchmarkPublication(unittest.TestCase):
    def test_measurement_uses_reported_artifact_after_reclaim(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            built = root / 'artifact'
            built.write_bytes(b'current executable')
            saved = root / 'measurement'

            def build(runner, arguments, *, on_success):
                for kind, path in [('bench', built), ('lib', root / 'unrelated')]:
                    print(json.dumps({
                        'reason': 'compiler-artifact',
                        'target': {'kind': [kind], 'name': 'invocation'},
                        'executable': str(path),
                    }))
                on_success()
                built.unlink()
                return 0

            with patch.object(comptime_bench.Runner, 'run', build):
                self.assertEqual(comptime_bench.build_benchmark('invocation', saved), 0)
            self.assertEqual(saved.read_bytes(), b'current executable')
            self.assertTrue(saved.stat().st_mode & 0o100)
