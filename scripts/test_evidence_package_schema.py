#!/usr/bin/env python3
"""Portable legacy and historical evidence envelope contracts."""
import copy
import json
from pathlib import Path
import runpy
import unittest

ROOT = Path(__file__).resolve().parent.parent
VALIDATOR = runpy.run_path(str(ROOT / 'scripts/validate_nessy_acceptance.py'))
SCHEMA = json.loads((ROOT / 'schemas/documentation/evidence-package.schema.json').read_text())


def errors(value):
    result = []
    validator = VALIDATOR['MiniSchema'](SCHEMA, result)
    validator.validate(value, SCHEMA, '$', {})
    validator.finish()
    return result


def package():
    return {'schema': 'codeclew-documentation-evidence-package/1.0',
            'service': {'schema': 'codeclew-documentation-service/1.0', 'id': 'owned',
                        'title': 'Owned', 'repositoryId': 'owned', 'repository': 'https://example.invalid/owned',
                        'language': 'java', 'profile': 'java-17plus-maven-read-only',
                        'compilations': [':/main'], 'targetRef': 'HEAD'},
            'serviceDigest': 'sha256:' + 'a' * 64, 'revision': 'b' * 40,
            'producerVersion': 'owned-fixture', 'compatibilityDigest': 'sha256:' + 'c' * 64,
            'outcome': 'CAPTURED', 'parts': [{'kind': 'report', 'path': 'parts/' + 'd' * 64 + '.json',
            'digest': 'sha256:' + 'd' * 64, 'bytes': 100, 'records': 1, 'encoding': 'json'}]}


def origin():
    return {'schema': 'codeclew-documentation-retained-export/1.0', 'snapshot': 'sha256:' + 'a' * 64 + '/100',
            'inputDigest': 'sha256:' + 'b' * 64, 'contextDigest': 'sha256:' + 'c' * 64,
            'captureDigest': 'sha256:' + 'd' * 64, 'cacheability': 'NON_CACHEABLE',
            'cacheabilityReason': 'Owned external authority is incomplete',
            'sourceAuthority': 'RETAINED_SOURCE_NOT_REVERIFIED', 'automaticReuse': 'DISALLOWED_HISTORICAL_EXPORT'}


class EvidenceSchemaTest(unittest.TestCase):
    def test_legacy_and_explicit_historical_formats(self):
        value = package()
        self.assertFalse(errors(value))
        value['retained'] = origin()
        self.assertTrue(errors(value))
        value['schema'] = 'codeclew-documentation-evidence-package/1.1'
        self.assertFalse(errors(value))
        for field in ['retained']:
            missing = copy.deepcopy(value); del missing[field]
            self.assertTrue(errors(missing))
        value['outcome'] = 'DIAGNOSTICS_ONLY'
        self.assertTrue(errors(value))

    def test_historical_authority_and_cacheability_cannot_be_upgraded(self):
        value = package(); value.update(schema='codeclew-documentation-evidence-package/1.1', retained=origin())
        for field, replacement in [('sourceAuthority', 'CURRENT'), ('automaticReuse', 'REUSABLE'),
                                   ('cacheabilityReason', None), ('cacheabilityReason', ' '),
                                   ('snapshot', 'sha256:' + 'a' * 64 + '/01')]:
            bad = copy.deepcopy(value); bad['retained'][field] = replacement
            self.assertTrue(errors(bad), field)
        value['retained'].update(cacheability='REUSABLE', cacheabilityReason=None)
        self.assertFalse(errors(value))
        value['retained']['cacheabilityReason'] = 'Not reusable reason'
        self.assertTrue(errors(value))


if __name__ == '__main__':
    unittest.main()
