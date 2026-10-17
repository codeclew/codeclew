#!/usr/bin/env python3
"""Offline paired-run accounting. No agent/provider execution or token estimation."""
from __future__ import annotations

import argparse
import json
import math
from pathlib import Path
import statistics
import sys

SCHEMA = 'codeclew-agent-efficiency-run/1.0'
# A comparison must hold these fixed; implementation/skill revisions vary by arm.
GROUP = ('experiment', 'corpus', 'model', 'effort', 'harness', 'guarantees',
         'cache', 'stratum', 'evidenceKind')
IDENTITY = ('task', 'taskDigest', 'fixtureDigest', 'repeat')
METRICS = ('elapsedMs', 'firstJustifiedEditMs', 'verifiedPatchMs', 'inputTokens',
           'cachedInputTokens', 'uncachedInputTokens', 'outputTokens', 'toolCalls',
           'failedToolCalls', 'modelRounds', 'toolOutputBytes', 'sourceBytes',
           'repeatedSourceBytes', 'indexMs', 'validationMs')
COUNTS = set(METRICS) - {'elapsedMs', 'firstJustifiedEditMs', 'verifiedPatchMs',
                         'indexMs', 'validationMs'}
FIELDS = set(GROUP + IDENTITY) | {
    'schema', 'arm', 'implementationRevision', 'skillDigest', 'outcome',
    'criticalMisses', 'falseExactClaims', 'metrics',
}


def validate(row):
    if not isinstance(row, dict) or set(row) != FIELDS:
        raise ValueError('run fields must exactly match the versioned contract')
    if row['schema'] != SCHEMA:
        raise ValueError('unsupported run schema')
    for key in FIELDS - {'repeat', 'criticalMisses', 'falseExactClaims', 'metrics'}:
        if not isinstance(row[key], str) or not row[key].strip():
            raise ValueError(f'{key} must be a nonempty string')
    for key in ('repeat', 'criticalMisses', 'falseExactClaims'):
        if type(row[key]) is not int or row[key] < 0:
            raise ValueError(f'{key} must be a nonnegative integer')
    for key, allowed in (
        ('cache', {'cold', 'warm'}),
        ('evidenceKind', {'observed', 'synthetic'}),
        ('outcome', {'pass', 'fail', 'unresolved'}),
    ):
        if row[key] not in allowed:
            raise ValueError(f'invalid {key}')
    metrics = row['metrics']
    if not isinstance(metrics, dict) or set(metrics) != set(METRICS):
        raise ValueError('metrics must contain every documented key; use null if unknown')
    for key, value in metrics.items():
        if value is None:
            continue
        if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
            raise ValueError(f'{key} must be finite and nonnegative, or null')
        if key in COUNTS and type(value) is not int:
            raise ValueError(f'{key} must be an integer or null')
    for smaller, larger in (
        ('cachedInputTokens', 'inputTokens'), ('uncachedInputTokens', 'inputTokens'),
        ('failedToolCalls', 'toolCalls'), ('sourceBytes', 'toolOutputBytes'),
        ('repeatedSourceBytes', 'sourceBytes'), ('firstJustifiedEditMs', 'elapsedMs'),
        ('verifiedPatchMs', 'elapsedMs'),
    ):
        if metrics[smaller] is not None and metrics[larger] is not None:
            if metrics[smaller] > metrics[larger]:
                raise ValueError(f'{smaller} exceeds {larger}')
    total, cached, fresh = (metrics[k] for k in (
        'inputTokens', 'cachedInputTokens', 'uncachedInputTokens'))
    if all(value is not None for value in (total, cached, fresh)) and total != cached + fresh:
        raise ValueError('inputTokens must equal cached plus uncached input')
    if row['outcome'] != 'pass' and metrics['verifiedPatchMs'] is not None:
        raise ValueError('failed/unresolved runs cannot report a verified patch')
    return row


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f'duplicate JSON key: {key}')
        result[key] = value
    return result


def load(path):
    rows = []
    with path.open(encoding='utf-8') as stream:
        for number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            try:
                rows.append(validate(json.loads(line, object_pairs_hook=unique_object)))
            except (ValueError, TypeError) as error:
                raise ValueError(f'line {number}: {error}') from error
    if not rows:
        raise ValueError('no runs')
    return rows


def quality_ok(row):
    return row['outcome'] == 'pass' and not row['criticalMisses'] and not row['falseExactClaims']


def metric_summary(pairs, key):
    known = [(a['metrics'][key], b['metrics'][key]) for a, b in pairs
             if a['metrics'][key] is not None and b['metrics'][key] is not None]
    ratios = [b / a for a, b in known if a > 0]
    return {
        'knownPairs': len(known), 'totalPairs': len(pairs),
        'baselineSumKnown': sum(a for a, _ in known) if known else None,
        'candidateSumKnown': sum(b for _, b in known) if known else None,
        'medianCandidateOverBaseline': statistics.median(ratios) if ratios else None,
        'ratioPairs': len(ratios),
        'complete': bool(pairs) and len(known) == len(pairs),
    }


def report(rows, baseline, candidate):
    if baseline == candidate:
        raise ValueError('baseline and candidate must differ')
    groups = {}
    seen = set()
    for row in rows:
        validate(row)
        if row['arm'] not in (baseline, candidate):
            raise ValueError('unexpected arm; provide only the two requested arms')
        group = tuple(row[k] for k in GROUP)
        identity = tuple(row[k] for k in IDENTITY)
        unique = (group, row['arm'], identity)
        if unique in seen:
            raise ValueError('duplicate task/repeat within arm and comparison group')
        seen.add(unique)
        groups.setdefault(group, {baseline: {}, candidate: {}})[row['arm']][identity] = row
    if not groups:
        raise ValueError('no runs')
    output = []
    for group, arms in sorted(groups.items()):
        left, right = arms[baseline], arms[candidate]
        common = sorted(left.keys() & right.keys())
        pairs = [(left[k], right[k]) for k in common]
        both_pass = [(a, b) for a, b in pairs if quality_ok(a) and quality_ok(b)]
        losses = [dict(zip(IDENTITY, k)) for k in common
                  if quality_ok(left[k]) and not quality_ok(right[k])]
        output.append({
            'group': dict(zip(GROUP, group)), 'pairedRuns': len(pairs),
            'unmatchedBaseline': [dict(zip(IDENTITY, k)) for k in sorted(left.keys() - right.keys())],
            'unmatchedCandidate': [dict(zip(IDENTITY, k)) for k in sorted(right.keys() - left.keys())],
            'allTasksPaired': bool(pairs) and left.keys() == right.keys(),
            'qualityLosses': losses,
            'arms': {name: {
                'runs': len(values),
                'outcomes': {outcome: sum(r['outcome'] == outcome for r in values.values())
                             for outcome in ('pass', 'fail', 'unresolved')},
                'qualityPasses': sum(quality_ok(r) for r in values.values()),
                'criticalMisses': sum(r['criticalMisses'] for r in values.values()),
                'falseExactClaims': sum(r['falseExactClaims'] for r in values.values()),
                'implementationRevisions': sorted({r['implementationRevision'] for r in values.values()}),
                'skillDigests': sorted({r['skillDigest'] for r in values.values()}),
            } for name, values in arms.items()},
            'allPairedMetrics': {key: metric_summary(pairs, key) for key in METRICS},
            'bothQualityPassMetrics': {key: metric_summary(both_pass, key) for key in METRICS},
        })
    return {
        'schema': 'codeclew-agent-efficiency-report/1.0',
        'baseline': baseline, 'candidate': candidate,
        'authority': 'OFFLINE_ACCOUNTING_NOT_RUNTIME_OR_PERFORMANCE_QUALIFICATION',
        'groups': output,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('input', type=Path)
    parser.add_argument('--baseline', required=True)
    parser.add_argument('--candidate', required=True)
    args = parser.parse_args()
    try:
        value = report(load(args.input), args.baseline, args.candidate)
    except (ValueError, OSError) as error:
        print(f'invalid measurement: {error}', file=sys.stderr)
        return 2
    print(json.dumps(value, indent=2, allow_nan=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
