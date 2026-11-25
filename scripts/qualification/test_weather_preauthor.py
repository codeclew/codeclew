"""Negative controls for the installed Weather qualification."""
import json
from pathlib import Path
import runpy
from tempfile import TemporaryDirectory
import unittest

Q = runpy.run_path(str(Path(__file__).with_name('weather-preauthor.py')))


class WeatherQualificationTests(unittest.TestCase):
    def setUp(self):
        self.criteria = Q['rubric']()
        self.scopes = {row['route']: {} for row in self.criteria.values()}
        self.statements = {}
        self.report = {'runDigest': 'bound-kit', 'rubricSha256': Q['RUBRIC_SHA'], 'criteria': self.criteria,
                       'scopes': self.scopes, 'statements': self.statements}
        self.review = {'runDigest': 'bound-kit', 'rubricSha256': Q['RUBRIC_SHA'], 'reviewer': 'independent',
                       'materialFalseClaims': [], 'criteria': {}}
        for atom, row in self.criteria.items():
            witnesses = {}
            for part, component in row['components'].items():
                key = atom + '/' + str(part)
                self.statements[key] = {'scopes': [row['route']], 'text': component, 'anchors': ['m123:a']}
                witnesses[str(part)] = [key]
            self.review['criteria'][atom] = {'status': 'FULL', 'witnesses': witnesses}

    def test_positive_and_deleted_atom(self):
        self.assertEqual((45, {'m123:a'}), Q['coverage'](self.report, self.review))
        self.review['criteria'].pop(next(iter(self.criteria)))
        with self.assertRaisesRegex(ValueError, 'every frozen atom'):
            Q['coverage'](self.report, self.review)

    def test_compound_atom_requires_each_component(self):
        self.review['criteria']['route-add-place.html/core/2']['witnesses'].pop('2')
        with self.assertRaisesRegex(ValueError, 'Missing component'):
            Q['coverage'](self.report, self.review)

    def test_wrong_scope_rejected_even_with_cited_statement(self):
        key = 'route-scheduled-refresh.html/core/2/1'
        self.statements[key]['scopes'] = ['operation-602619abba89']
        with self.assertRaisesRegex(ValueError, 'Wrong operation scope'):
            Q['coverage'](self.report, self.review)

    def test_file_fact_requires_visible_connection_from_eligible_entry(self):
        atom = 'route-add-place.html/core/1'
        direct = atom + '/1'
        answer = self.review['criteria'][atom]
        self.statements[direct]['text'] = 'The route calls Places.Save.'
        self.statements['file-fact'] = {'scopes': [], 'files': ['places:Repository.cs'],
                                        'text': 'Places.Save validates this branch.', 'anchors': ['m123:b']}
        answer['witnesses']['1'].append('file-fact')
        with self.assertRaisesRegex(ValueError, 'Wrong operation scope'):
            Q['coverage'](self.report, self.review)
        answer['scopeLinks'] = {'file-fact': {'via': direct, 'symbol': 'Places.Save'}}
        self.assertEqual((45, {'m123:a', 'm123:b'}), Q['coverage'](self.report, self.review))
        self.statements['file-fact']['text'] = 'OtherPlaces.Save validates a different branch.'
        with self.assertRaisesRegex(ValueError, 'Wrong operation scope'):
            Q['coverage'](self.report, self.review)

    def test_old_grade_or_changed_guard_cannot_be_reused(self):
        key = 'route-scheduled-refresh.html/core/2/1'
        original = self.statements.pop(key)
        Q['record'](self.statements, original['scopes'], 'view.md:11',
                    'A past per-place marker blocks a send.', original['anchors'])
        with self.assertRaisesRegex(ValueError, 'Stale or unavailable'):
            Q['coverage'](self.report, self.review)
        self.review['runDigest'] = 'different-kit'
        with self.assertRaisesRegex(ValueError, 'changed kit'):
            Q['coverage'](self.report, self.review)

    def test_source_only_and_sidecar_rows_never_reach_the_witness_index(self):
        with TemporaryDirectory() as directory:
            path = Path(directory) / 'view.md'
            path.write_text(
                '# Loaded file\n\n'
                '## Prepared source evidence\nFacts for Page current.md [SOURCE]\n'
                '- [conditional] Authentic front assertion. | Cite m123:c (lines 1, 4-6)\n\n'
                '```\n## Prepared source evidence\nFacts for Page current.md\n'
                '- [conditional] This row is raw source. | Cite m123:a\n```\n'
                '## Additional prepared source evidence\nBoundaries for Page current.md\n'
                '- [scope] Later assertion. | Cite m123:b\n'
                '## Unrelated\nFacts for Page current.md\n'
                '- [conditional] Unrelated assertion. | Cite m123:a\n')
            result = list(Q['visible_view'](path))
            self.assertEqual(2, len(result))
            self.assertEqual(['m123:c (lines 1, 4-6)'], result[0][3])
            self.assertIn('Later assertion', result[1][2])

    def test_byte_and_line_anchor_reject_changed_body_or_range(self):
        blob = b'int local;\nif (active)\n  send();\n'
        raw = b'if (active)\n  send();'
        occurrence = {'startByte': 11, 'endByte': 11 + len(raw)}
        self.assertTrue(Q['matches_source'](blob, raw, 2, 3, occurrence))
        self.assertTrue(Q['matches_source'](blob, raw, 2, 3, None))
        self.assertFalse(Q['matches_source'](blob, raw, 1, 2, occurrence))
        self.assertFalse(Q['matches_source'](blob, b'if (!active)', 2, 3, None))

    def test_route_resolves_native_page_without_historical_identity(self):
        row = {'kind': 'ENTRY', 'service': 'dashboard', 'id': 'fresh-entry',
               'method': 'POST', 'path': '/api/places', 'page': 'native-page.md'}
        scoped = Q['select_scopes']({'requirements': [row]})
        self.assertEqual({'operation-a7d8675103bc': row}, scoped)

    def test_only_declared_inline_facts_and_their_qualified_scopes(self):
        row = {'kind': 'ENTRY', 'service': 'dashboard', 'id': 'native-entry',
               'method': 'POST', 'path': '/api/places', 'page': 'actual-page.md'}
        plan = {'mode': 'INLINE', 'entries': {'E1': row['id']},
                'citations': {'S1': 'm123:c'}}
        packet = {'entries': {'E1': {'facts': [{
            'kind': 'guard', 'text': 'Scoped fact', 'uncertainty': 'Activation conditional.', 'sources': ['S1'],
        }]}}, 'files': {'F1': {'source': 'Unqualified raw source'}},
                  'citations': {'S1': {'file': 'source.ts', 'startLine': 1, 'endLine': 1}}}
        with TemporaryDirectory() as directory:
            run = Path(directory)
            (run / 'prompt.txt').write_text('Use prepared evidence.\n\n' + json.dumps(packet) + '\n')
            _, result = Q['collect'](run, {'requirements': [row]}, plan)
            self.assertEqual(1, len(result))
            statement = next(iter(result.values()))
            self.assertEqual(['operation-a7d8675103bc'], statement['scopes'])
            self.assertIn('Activation conditional.', statement['text'])

    def test_isolated_page_reads_actual_body_and_cite_location(self):
        requirement = {'id': 'actual-entry', 'kind': 'ENTRY', 'service': 'dashboard',
                       'method': 'POST', 'path': '/api/places', 'page': 'actual-page.md'}
        slot = {'key': 'op', 'kind': 'OPERATION', 'page': 'actual-page.md',
                'requirement': requirement['id'], 'prompt': 'op.txt', 'citations': {'S1': 'm123:c'}}
        packet = {'slot': 'op', 'type': 'OPERATION', 'page': slot['page'], 'requirement': requirement,
                  'facts': {requirement['id']: {'facts': [{'text': 'Local condition',
                                                          'sources': ['S1'], 'lines': {'S1': ['2']}}]}},
                  'citations': {'S1': {'region': 'V1', 'authority': 'EXACT_SNAPSHOT_TEXT'}},
                  'sourceRanges': {'V1': {'service': 'dashboard', 'file': 'source.ts',
                                           'startLine': 1, 'endLine': 2, 'text': 'retained body'}}}
        with TemporaryDirectory() as directory:
            run = Path(directory)
            (run / 'inputs').mkdir()
            path = run / 'inputs/op.txt'
            path.write_text('Prepare one page.\n\n' + json.dumps(packet))
            inventory = {'requirements': [requirement]}
            plan = {'mode': 'ISOLATED_PAGES', 'slots': [slot]}
            _, result = Q['collect'](run, inventory, plan)
            fact = next(iter(result.values()))
            self.assertEqual(['m123:c (lines 2)'], fact['anchors'])
            self.assertEqual(Q['sha'](b'retained body'), fact['locations']['m123:c']['bodySha256'])
            packet['page'] = 'foreign-page.md'
            path.write_text('Prepare one page.\n\n' + json.dumps(packet))
            with self.assertRaisesRegex(ValueError, 'differs from its frozen slot'):
                Q['collect'](run, inventory, plan)


if __name__ == '__main__':
    unittest.main()
