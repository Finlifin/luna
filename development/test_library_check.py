"""Library integration orchestration and output contracts, without a build."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import library_check


class LibraryCheckTests(unittest.TestCase):
    def check_stages(self, release=False, failing=None, wrong_value=False, native_output=None, native_stage='native-math', selected=None):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            calls = []

            def run(command, **kwargs):
                label = kwargs['label']
                calls.append((label, command))
                evidence = kwargs['directory']
                evidence.mkdir(parents=True)
                count = {'native-resources': 4, 'native-comparison': 6, 'native-math': 11, 'native-option': 1, 'native-result': 1, 'native-tuple': 4, 'native-conversion': 3, 'sysroot-iteration': 9, 'sysroot-resources': 3, 'sysroot-patterns': 3, 'sysroot-meta-patterns': 3, 'sysroot-hash': 3, 'sysroot-meta': 1, 'sysroot-builders': 2, 'sysroot-enum-builder': 2, 'sysroot-symbol-enum': 2, 'sysroot-fixed-enum': 2, 'sysroot-build-mode': 1, 'sysroot-build-target': 2}
                text = '(bool false)' if wrong_value else '(bool true)'
                if label in ('attributes', 'merged-attributes'):
                    text = '(evaluated-attribute (property "abi") (symbol "c"))'
                    if label == 'merged-attributes':
                        text = (f'(attribute-inspection (evaluated {text}) '
                                '(selected (merged-attributes (key "abi") (entry 0))))')
                    if native_output is not None and label == native_stage:
                        text = native_output
                if label in count:
                    text = f'(count {count[label]}) (failures 0)'
                    if native_output is not None and label == native_stage:
                        text = native_output
                (evidence / 'stdout.log').write_text(text)
                return (1 if failing == label else 0), evidence

            argv = ['library_check.py'] + (['--release'] if release else [])
            for stage in selected or []:
                argv.extend(['--stage', stage])
            with patch.object(library_check.tempfile, 'mkdtemp', return_value=str(root)), \
                    patch.object(library_check, 'run', side_effect=run), \
                    patch('sys.argv', argv), patch('builtins.print'):
                status = library_check.main()
            summary = json.loads((root / 'summary.json').read_text())
            cargo_calls = [(label, command) for label, command in calls if label != 'sysroot-run']
            self.assertEqual(len(cargo_calls), len(set(selected)) if selected else len(library_check.STAGES))
            self.assertEqual(summary['scope'], 'selected' if selected else 'all')
            self.assertEqual(summary['selected_stages'], [label for label, _ in cargo_calls])
            for _, command in cargo_calls:
                cargo_arguments = command[3:]
                self.assertIn(cargo_arguments[0], ('test', 'run'))
                self.assertEqual('--release' in cargo_arguments, release)
                if release:
                    self.assertEqual(cargo_arguments[1], '--release')
            for name, alias in [('option', 'optional'), ('result', 'fallible')]:
                if selected and f'native-{name}' not in selected:
                    continue
                command = next(command for label, command in calls if label == f'native-{name}')
                self.assertIn(str(library_check.ROOT / f'library/builtin/{name}.fl'), command)
                self.assertIn(str(library_check.ROOT / 'library/builtin/resources.fl'), command)
                self.assertIn('--test-native', command)
                self.assertIn('resources.Copy', command)
                self.assertIn('resources.Drop', command)
                if name == 'option':
                    self.assertNotIn('--optional-type', command)
                    self.assertIn('optional', command)
            if not selected or 'native-math' in selected:
                command = next(command for label, command in calls if label == 'native-math')
                self.assertIn('--sysroot', command)
                self.assertIn(str(library_check.ROOT / 'library'), command)
                self.assertNotIn('--packages-v1', command)
                self.assertNotIn('--host-intrinsic', command)
                self.assertNotIn('--copy-protocol', command)
                self.assertIn('--test-native', command)
            if not selected or 'sysroot' in selected:
                build = next(command for label, command in calls if label == 'sysroot')
                self.assertIn('--build', build)
                executable = build[build.index('--output') + 1]
                executions = [command for label, command in calls if label == 'sysroot-run']
                self.assertEqual(executions, [] if failing == 'sysroot' else [[executable]])
                stage = next(item for item in summary['stages'] if item['stage'] == 'sysroot')
                self.assertTrue(stage['build_report'].endswith('/sysroot/result.json'))
                if failing != 'sysroot':
                    self.assertTrue(stage['report'].endswith('/sysroot-run/result.json'))
            self.assertEqual(summary['passed'], status == 0)
            return status, summary

    def test_sysroot_suites_require_all_native_cases(self):
        for stage, count in [('sysroot-iteration', 9), ('sysroot-resources', 3), ('sysroot-patterns', 3), ('sysroot-meta-patterns', 3), ('sysroot-hash', 3), ('sysroot-meta', 1), ('sysroot-builders', 2), ('sysroot-enum-builder', 2), ('sysroot-symbol-enum', 2), ('sysroot-fixed-enum', 2), ('sysroot-build-mode', 1), ('sysroot-build-target', 2)]:
            for output in ['(count 0) (failures 0)', f'(count {count - 1}) (failures 0)',
                           '(count 99) (failures 0)', f'(count {count}) (failures 1)']:
                with self.subTest(stage=stage, output=output):
                    status, summary = self.check_stages(
                        selected=[stage], native_stage=stage, native_output=output)
                    self.assertEqual(status, 1)
                    self.assertFalse(summary['stages'][0]['value_matches'])

    def test_sysroot_build_and_run_failures_are_distinct(self):
        for failing in ['sysroot', 'sysroot-run']:
            with self.subTest(failing=failing):
                status, summary = self.check_stages(selected=['sysroot'], failing=failing)
                self.assertEqual(status, 1)
                stage = summary['stages'][0]
                self.assertFalse(stage['passed'])
                self.assertIsNone(stage['value_matches'])
                self.assertTrue(stage['report'].endswith(f'/{failing}/result.json'))

    def test_inline_iteration_failure_is_reported(self):
        status, summary = self.check_stages(release=True, failing='inline-iteration', selected=['inline-iteration'])
        self.assertEqual(status, 1)
        self.assertFalse(summary['stages'][0]['passed'])

    def test_meta_values_requires_the_expected_comptime_result(self):
        for wrong_value in [False, True]:
            status, summary = self.check_stages(selected=['meta-values'], wrong_value=wrong_value)
            self.assertEqual(status, int(wrong_value))
            self.assertEqual(summary['stages'][0]['value_matches'], not wrong_value)

    def test_attribute_stages_require_evaluation_and_merge_evidence(self):
        raw = '(evaluated-attribute (property "abi") (symbol "c"))'
        for label, output in [('attributes', ''), ('attributes', raw + raw),
                              ('attributes', raw.replace('"c"', '"other"')),
                              ('merged-attributes', raw),
                              ('merged-attributes', '(attribute-inspection (evaluated) (selected))')]:
            with self.subTest(label=label, output=output):
                status, summary = self.check_stages(
                    selected=[label], native_stage=label, native_output=output)
                self.assertEqual(status, 1)
                self.assertFalse(summary['stages'][0]['value_matches'])

    def test_profiles_apply_to_every_cargo_stage(self):
        for release in [False, True]:
            with self.subTest(release=release):
                status, _ = self.check_stages(release=release)
                self.assertEqual(status, 0)

    def test_failed_stage_does_not_skip_independent_checks(self):
        status, summary = self.check_stages(release=True, failing='declarations')
        self.assertEqual(status, 1)
        self.assertFalse(summary['stages'][0]['passed'])
        self.assertTrue(summary['stages'][-1]['passed'])

    def test_successful_process_with_wrong_result_is_failure(self):
        status, summary = self.check_stages(release=True, wrong_value=True)
        self.assertEqual(status, 1)
        self.assertFalse(summary['stages'][-1]['value_matches'])

    def test_native_math_requires_all_eleven_tests_and_zero_failures(self):
        for output in ['(count 8) (failures 0)', '(count 11) (failures 1)']:
            with self.subTest(output=output):
                status, summary = self.check_stages(release=True, native_output=output)
                self.assertEqual(status, 1)
                math = next(stage for stage in summary['stages'] if stage['stage'] == 'native-math')
                self.assertFalse(math['value_matches'])

    def test_value_type_stages_require_a_real_passing_native_test(self):
        for label in ['native-option', 'native-result']:
            for output in ['(count 0) (failures 0)', '(count 1) (failures 1)']:
                with self.subTest(label=label, output=output):
                    status, summary = self.check_stages(native_output=output, native_stage=label)
                    self.assertEqual(status, 1)
                    stage = next(item for item in summary['stages'] if item['stage'] == label)
                    self.assertFalse(stage['value_matches'])
                    self.assertTrue(summary['stages'][-1]['passed'])

    def test_selected_stages_are_explicit_and_deduplicated(self):
        status, summary = self.check_stages(release=True, selected=['native-tuple', 'native-math', 'native-tuple'])
        self.assertEqual(status, 0)
        self.assertEqual(summary['selected_stages'], ['native-math', 'native-tuple'])

    def test_tuple_suite_cannot_pass_with_an_empty_or_partial_test_set(self):
        for output in ['(count 0) (failures 0)', '(count 3) (failures 0)', '(count 4) (failures 1)']:
            status, summary = self.check_stages(native_stage='native-tuple', native_output=output, selected=['native-tuple'])
            self.assertEqual(status, 1)
            self.assertFalse(summary['stages'][0]['value_matches'])

    def test_conversion_suite_requires_all_three_tests(self):
        for output in ['(count 0) (failures 0)', '(count 2) (failures 0)', '(count 3) (failures 1)']:
            status, summary = self.check_stages(native_stage='native-conversion', native_output=output, selected=['native-conversion'])
            self.assertEqual(status, 1)
            self.assertFalse(summary['stages'][0]['value_matches'])
