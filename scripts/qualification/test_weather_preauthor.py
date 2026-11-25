"""Negative controls for the installed Weather qualification."""
import json
from pathlib import Path
import runpy
import tarfile
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


class WeatherReleaseEvidenceTests(unittest.TestCase):
    def test_exact_archive_runtime_install_and_bundled_skill(self):
        payload = runpy.run_path(str(Path(__file__).with_name('weather-release-payload.py')))
        with TemporaryDirectory() as directory:
            base = Path(directory)
            package = base / 'codeclew'
            skill = package / 'source/skills/codeclew'
            skill.mkdir(parents=True)
            (skill / 'SKILL.md').write_text('fixture skill\n')
            content = (skill / 'SKILL.md').read_bytes()
            revision, tree = 'a' * 40, 'b' * 40
            common = {'sourceRevision': revision, 'sourceTree': tree}
            bytes_for = payload['canonical']
            sha = payload['sha']
            release = {**common, 'schema': 'codeclew-release/2.0', 'status': 'PILOT_READY',
                       'profile': 'core', 'version': 'v1.1.0', 'operatingSystem': 'linux', 'architecture': 'x86_64'}
            source = {**common, 'schema': 'codeclew-release-source/1.0', 'manifestDigest': '',
                      'files': [{'path': 'skills/codeclew/SKILL.md', 'sha256': sha(content), 'size': len(content), 'mode': 0}]}
            source['manifestDigest'] = sha(bytes_for(source))
            seed = {**common, 'schema': 'codeclew-trusted-release-seed/2.0', 'mode': 'RELEASE',
                    'sourcePayloadDigest': source['manifestDigest'], 'runtimeKey': sha(b'key'), 'manifestDigest': sha(b'manifest')}
            seed['seedDigest'] = sha(bytes_for(seed))
            launcher = package / 'bin/clew'
            launcher.parent.mkdir()
            launcher.write_text('fixture launcher\n')
            seed_path = package / 'seed' / ('release-N-' + revision) / 'seed.json'
            seed_path.parent.mkdir(parents=True)
            for path, value in [(package / 'release.json', release), (package / 'source/release-source.json', source),
                                (seed_path, seed)]:
                path.write_bytes(bytes_for(value) + b'\n')
            archive = base / 'codeclew-linux-x86_64.tar.gz'
            with tarfile.open(archive, 'w:gz') as item:
                item.add(package, arcname='codeclew')
            proof, files = payload['read_archive'](archive, revision)
            self.assertEqual(proof['sourcePayloadDigest'], source['manifestDigest'])
            pins = {'skillFiles': files, 'clewVersion': 'clew 1.1.0',
                    'clewCapabilities': {'runtimeMode': 'RELEASE', 'runtimeKey': seed['runtimeKey'],
                                         'runtimeManifestDigest': seed['manifestDigest']}}
            payload['check_pins'](proof, pins)
            payload['check_install'](proof, files, {'clew': str(launcher)}, skill)
            pins['clewCapabilities']['runtimeManifestDigest'] = sha(b'stock runtime')
            with self.assertRaisesRegex(ValueError, 'not from the target runtime'):
                payload['check_pins'](proof, pins)
            with self.assertRaisesRegex(ValueError, 'archive metadata|payload member'):
                payload['read_archive'](archive, 'c' * 40)
            (skill / 'SKILL.md').write_text('substituted skill')
            with self.assertRaisesRegex(ValueError, 'skill differs'):
                payload['check_install'](proof, files, {'clew': str(launcher)}, skill)

    def test_recomputed_release_report_rejects_mismatched_archive_and_old_review(self):
        with TemporaryDirectory() as directory:
            base = Path(directory)
            review = base / 'review.json'
            review.write_text('{"criteria":{"test":"independent original"}}')
            (base / 'pins.json').write_text(json.dumps({'inputFiles': {'facts.json': 'digest'},
                                                       'inventoryDigest': 'inventory', 'promptDigest': 'prompt'}))
            report_file = base / 'report.json'
            payload = {'archiveSha256': 'first archive'}
            inventory = {'snapshot': 'snapshot'}
            witness = {'mode': 'ISOLATED_PAGES'}
            decision = {'status': 'PASS', 'full': 45, 'required': 45, 'materialFalseClaims': 0, 'verifiedAnchors': 30}
            computed = Q['release_report'](payload, base, inventory, witness, review, decision)
            report_file.write_text(json.dumps(computed))
            Q['validate_release_report'](report_file, computed)
            changed = Q['release_report']({'archiveSha256': 'another archive'}, base, inventory, witness, review, decision)
            with self.assertRaisesRegex(ValueError, 'differs from exact archive'):
                Q['validate_release_report'](report_file, changed)
            review.write_text('{"criteria":{"test":"edited"}}')
            changed = Q['release_report'](payload, base, inventory, witness, review, decision)
            with self.assertRaisesRegex(ValueError, 'differs from exact archive'):
                Q['validate_release_report'](report_file, changed)
            computed['full'] = 44
            with self.assertRaisesRegex(ValueError, '45/45'):
                Q['validate_release_report'](report_file, computed)


if __name__ == '__main__':
    unittest.main()
