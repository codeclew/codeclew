#!/usr/bin/env python3
"""Synthetic accounting regressions, not Codeclew performance results."""
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name('agent_efficiency_report.py')
SPEC = importlib.util.spec_from_file_location('efficiency', SCRIPT)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def run(arm='bare', **changes):
    row = {
        'schema': MODULE.SCHEMA, 'experiment': 'synthetic-test', 'corpus': 'fixture-v1',
        'model': 'not-executed', 'effort': 'not-executed', 'harness': 'unit-test',
        'guarantees': 'same-native-tests', 'cache': 'warm', 'stratum': 'single-edit',
        'evidenceKind': 'synthetic', 'task': 'a', 'taskDigest': 'fixture-task-a',
        'fixtureDigest': 'fixture-revision-a', 'repeat': 0, 'arm': arm,
        'implementationRevision': 'synthetic', 'skillDigest': 'synthetic',
        'outcome': 'pass', 'criticalMisses': 0, 'falseExactClaims': 0,
        'metrics': dict.fromkeys(MODULE.METRICS),
    }
    row['metrics'].update(elapsedMs=100, inputTokens=1000, cachedInputTokens=700,
                          uncachedInputTokens=300, outputTokens=50)
    row.update(changes)
    return row


class AccountingTests(unittest.TestCase):
    def test_paired_ratio_does_not_double_count_cached_tokens(self):
        a, b = run(), run('compact')
        b['metrics'].update(inputTokens=800, cachedInputTokens=700, uncachedInputTokens=100)
        group = MODULE.report([a, b], 'bare', 'compact')['groups'][0]
        self.assertEqual(group['allPairedMetrics']['inputTokens']['medianCandidateOverBaseline'], 0.8)
        self.assertAlmostEqual(group['allPairedMetrics']['uncachedInputTokens']['medianCandidateOverBaseline'], 1/3)

    def test_cold_guarantees_and_synthetic_never_mix(self):
        for key, value in [('cache', 'cold'), ('guarantees', 'managed'), ('evidenceKind', 'observed')]:
            b = run('compact', **{key: value})
            groups = MODULE.report([run(), b], 'bare', 'compact')['groups']
            self.assertEqual(len(groups), 2)
            self.assertTrue(all(g['pairedRuns'] == 0 and not g['allTasksPaired'] for g in groups))

    def test_missing_tasks_and_revisions_are_not_silently_dropped(self):
        b = run('compact', fixtureDigest='changed-target')
        group = MODULE.report([run(), b], 'bare', 'compact')['groups'][0]
        self.assertEqual(group['pairedRuns'], 0)
        self.assertEqual(len(group['unmatchedBaseline']), 1)
        self.assertEqual(len(group['unmatchedCandidate']), 1)

    def test_fast_failure_cannot_be_counted_as_quality_win(self):
        a, b = run(), run('compact', outcome='fail')
        b['metrics']['elapsedMs'] = 1
        group = MODULE.report([a, b], 'bare', 'compact')['groups'][0]
        self.assertEqual(len(group['qualityLosses']), 1)
        self.assertEqual(group['bothQualityPassMetrics']['elapsedMs']['knownPairs'], 0)
        self.assertEqual(group['allPairedMetrics']['elapsedMs']['medianCandidateOverBaseline'], 0.01)

    def test_false_exact_claim_is_quality_failure_even_with_passing_tests(self):
        group = MODULE.report([run(), run('compact', falseExactClaims=1)], 'bare', 'compact')['groups'][0]
        self.assertEqual(len(group['qualityLosses']), 1)
        self.assertEqual(group['arms']['compact']['qualityPasses'], 0)

    def test_null_and_zero_are_not_fabricated_measurements(self):
        a, b = run(), run('compact')
        a['metrics']['elapsedMs'] = 0
        group = MODULE.report([a, b], 'bare', 'compact')['groups'][0]
        self.assertIsNone(group['allPairedMetrics']['elapsedMs']['medianCandidateOverBaseline'])
        self.assertEqual(group['allPairedMetrics']['elapsedMs']['ratioPairs'], 0)
        self.assertFalse(group['allPairedMetrics']['toolCalls']['complete'])

    def test_rejects_invalid_accounting(self):
        for key, value in [('inputTokens', 1001), ('toolCalls', True),
                           ('elapsedMs', float('nan')), ('outputTokens', -1)]:
            row = run()
            row['metrics'][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                MODULE.validate(row)
        row = run(outcome='unresolved')
        row['metrics']['verifiedPatchMs'] = 50
        with self.assertRaises(ValueError):
            MODULE.validate(row)

    def test_duplicate_runs_and_unexpected_arms_fail(self):
        a = run()
        with self.assertRaises(ValueError):
            MODULE.report([a, copy.deepcopy(a)], 'bare', 'compact')
        with self.assertRaises(ValueError):
            MODULE.report([run('third')], 'bare', 'compact')

    def test_cli_roundtrip_and_bad_json_return_nonzero(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'runs.jsonl'
            path.write_text('\n'.join(json.dumps(r) for r in [run(), run('compact')]), encoding='utf-8')
            command = [sys.executable, '-I', '-S', str(SCRIPT), str(path), '--baseline', 'bare', '--candidate', 'compact']
            result = subprocess.run(command, capture_output=True, text=True, check=True)
            self.assertEqual(json.loads(result.stdout)['groups'][0]['pairedRuns'], 1)
            path.write_text('{"schema":"a","schema":"b"}\n', encoding='utf-8')
            result = subprocess.run(command, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn('duplicate JSON key', result.stderr)
            self.assertEqual(result.stdout, '')


if __name__ == '__main__':
    unittest.main()
