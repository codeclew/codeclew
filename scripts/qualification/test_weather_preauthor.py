"""Negative controls for the installed Weather qualification."""
import json
from pathlib import Path
import runpy
import tarfile
from tempfile import TemporaryDirectory
import unittest
from unittest.mock import patch

Q = runpy.run_path(str(Path(__file__).with_name('weather-preauthor.py')))
SKILL = Path(__file__).resolve().parents[2] / 'skills/codeclew'


class WeatherQualificationTests(unittest.TestCase):
    def setUp(self):
        self.criteria = Q['rubric']()
        self.scopes = {row['route']: {} for row in self.criteria.values()}
        self.statements = {}
        self.report = {'runDigest': 'bound-kit', 'rubricSha256': Q['RUBRIC_SHA'], 'criteria': self.criteria,
                       'scopes': self.scopes, 'statements': self.statements}
        self.bind_counterevidence({})
        self.review = {'runDigest': 'bound-kit', 'rubricSha256': Q['RUBRIC_SHA'], 'reviewer': 'independent',
                       'materialFalseClaims': [], 'criteria': {}}
        for atom, row in self.criteria.items():
            witnesses = {}
            for part, component in row['components'].items():
                key = atom + '/' + str(part)
                self.statements[key] = {'scopes': [row['route']], 'text': component, 'anchors': ['m123:a']}
                witnesses[str(part)] = [key]
            self.review['criteria'][atom] = {'status': 'FULL', 'witnesses': witnesses}

    def bind_counterevidence(self, assertions):
        self.report.update(counterevidence=assertions, counterevidenceDigest=Q['sha'](Q['canonical'](assertions)))

    def decision(self):
        with patch.dict(Q['evaluate'].__globals__, check_sources=lambda *args: 1):
            return Q['evaluate'](None, None, None, {'snapshot': 'fixture'}, self.report, self.review)

    def test_positive_and_deleted_atom(self):
        self.assertEqual((45, {'m123:a'}), Q['coverage'](self.report, self.review))
        self.review['criteria'].pop(next(iter(self.criteria)))
        with self.assertRaisesRegex(ValueError, 'every frozen atom'):
            Q['coverage'](self.report, self.review)

    def test_compound_atom_requires_each_component(self):
        self.review['criteria']['route-add-place.html/core/2']['witnesses'].pop('2')
        with self.assertRaisesRegex(ValueError, 'Missing component'):
            Q['coverage'](self.report, self.review)

    def test_isolated_components_require_same_operation_packet_and_visible_source_bytes(self):
        self.report['mode'] = 'ISOLATED_PAGES'
        for fact in self.statements.values():
            fact.update(operationPacket=fact['scopes'][0],
                        locations={'m123:a': {'bodySha256': Q['sha'](b'visible retained bytes')}})
        self.assertEqual((45, {'m123:a'}), Q['coverage'](self.report, self.review))
        fact = next(iter(self.statements.values()))
        original = fact.pop('operationPacket')
        with self.assertRaisesRegex(ValueError, 'outside its isolated operation prompt'):
            Q['coverage'](self.report, self.review)
        fact['operationPacket'] = 'different-operation'
        with self.assertRaisesRegex(ValueError, 'outside its isolated operation prompt'):
            Q['coverage'](self.report, self.review)
        fact['operationPacket'] = original
        fact['locations'] = {}
        with self.assertRaisesRegex(ValueError, 'lacks visible source'):
            Q['coverage'](self.report, self.review)
        self.report['mode'] = 'WORKSPACE'
        self.assertEqual((45, {'m123:a'}), Q['coverage'](self.report, self.review))

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
            _, result, counterevidence = Q['collect'](run, {'requirements': [row]}, plan, SKILL)
            self.assertEqual(1, len(result))
            self.assertEqual({}, counterevidence)
            statement = next(iter(result.values()))
            self.assertEqual(['operation-a7d8675103bc'], statement['scopes'])
            self.assertIn('Activation conditional.', statement['text'])

    def test_isolated_page_reads_actual_body_and_cite_location(self):
        requirement = {'id': 'actual-entry', 'kind': 'ENTRY', 'service': 'dashboard',
                       'method': 'POST', 'path': '/api/places', 'page': 'actual-page.md'}
        slot = {'key': 'op', 'kind': 'OPERATION', 'page': 'actual-page.md',
                'requirement': requirement['id'], 'prompt': 'op.txt', 'citations': {'S1': 'm123:c'}}
        packet = {'slot': 'op', 'type': 'OPERATION', 'page': slot['page'], 'requirement': requirement,
                  'facts': {requirement['id']: {'facts': [{'text': 'Local condition', 'authority': 'RETAINED_PATH',
                      'condition': 'when `loaded` and `valid`', 'sources': ['S1'], 'lines': {'S1': ['2']}}]}},
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
            _, result, counterevidence = Q['collect'](run, inventory, plan, SKILL)
            self.assertEqual({}, counterevidence)
            fact = next(iter(result.values()))
            self.assertEqual(['m123:c (lines 2)'], fact['anchors'])
            self.assertIn('operationPacket', fact)
            self.assertEqual(Q['sha'](b'retained body'), fact['locations']['m123:c']['bodySha256'])
            packet['factContext'] = {'P1': {'condition': 'when `loaded`'}, 'S1': {'sources': ['S1']},
                                     'L1': {'lines': {'S1': ['2']}}, 'A1': {'authority': 'RETAINED_PATH'}}
            packet['facts'][requirement['id']]['facts'][0] = {
                'text': 'Local condition', 'contextRefs': ['P1', 'S1', 'L1', 'A1'], 'conditionTail': ' and `valid`'}
            path.write_text('Prepare one page.\n\n' + json.dumps(packet))
            self.assertEqual(Q['collect'](run, inventory, plan, SKILL)[1], result)
            packet['page'] = 'foreign-page.md'
            path.write_text('Prepare one page.\n\n' + json.dumps(packet))
            with self.assertRaisesRegex(ValueError, 'differs from its frozen slot'):
                Q['collect'](run, inventory, plan, SKILL)

    def opposing_packets(self, condition=None):
        with TemporaryDirectory() as directory:
            run = Path(directory)
            (run / 'inputs').mkdir()
            requirement = {'id': 'entry', 'kind': 'ENTRY', 'service': 'dashboard',
                           'method': 'POST', 'path': '/api/places', 'page': 'operation.md'}
            slots = [{'key': 'operation', 'kind': 'OPERATION', 'page': requirement['page'], 'requirement': 'entry',
                      'prompt': 'operation.txt', 'citations': {'S0': 'm1:writer'}}]
            source = {'service': 'writer', 'file': 'writer.txt', 'startLine': 1, 'endLine': 3,
                      'text': 'open\nwrite\nclose'}
            matching_operation = {'slot': 'operation', 'type': 'OPERATION', 'page': requirement['page'],
                                  'requirement': requirement, 'facts': {'entry': {'facts': [{
                                      'text': 'When writes are enabled, delivery reaches the write.',
                                      'sources': ['S0'], 'lines': {'S0': ['2']}}]}},
                                  'citations': {'S0': {'region': 'V1', 'authority': 'EXACT_SNAPSHOT_TEXT'}},
                                  'sourceRanges': {'V1': source}}
            (run / 'inputs/operation.txt').write_text('Frozen prompt\n\n' + json.dumps(matching_operation))
            text = 'Delivery cannot reach the write.'
            for category, kind, key, alias in (
                    ('facts', 'GUIDE', 'guide', 'S1'),
                    ('boundaries', 'PROCESS_MAP', 'map', 'S2')):
                slot = {'key': key, 'kind': kind, 'page': key + '.md', 'requirement': None,
                        'prompt': key + '.txt', 'citations': {alias: 'm1:writer'},
                        'citationSpans': {alias: [['V1', 2, 2]]}}
                slots.append(slot)
                context = {'T1': {'status': 'INCOMPATIBLE_IN_RETAINED_PATHS'}, 'S1': {'sources': [alias]}}
                visible = {'text': text, 'kind': 'effect-boundary', 'contextRefs': ['S1', 'T1'],
                           'alsoFor': ['another-owner']}
                if condition:
                    context['P1'] = {'condition': condition}
                    visible['contextRefs'].append('P1')
                packet = {'slot': key, 'type': kind, 'page': slot['page'],
                          'facts': {'outside-positive-rubric': {category: [visible, {
                              'text': 'A separate selected path persists the record.',
                              'status': 'SUPPORTED', 'sources': [alias]}]}},
                          'factContext': context,
                          'citations': {alias: {'region': 'V1', 'authority': 'EXACT_SNAPSHOT_TEXT'}},
                          'sourceRanges': {'V1': source}}
                (run / 'inputs' / slot['prompt']).write_text('Frozen prompt\n\n' + json.dumps(packet))
            _, statements, counterevidence = Q['collect'](
                run, {'requirements': [requirement]}, {'mode': 'ISOLATED_PAGES', 'slots': slots}, SKILL)
            self.assertEqual(1, len(statements))
            self.statements.update(statements)
            self.review['criteria']['route-add-place.html/core/2']['witnesses']['1'] = list(statements)
            self.bind_counterevidence(counterevidence)
            self.assertEqual(1, len(counterevidence))
            identity, row = next(iter(counterevidence.items()))
            self.assertEqual(('INCOMPATIBLE_IN_RETAINED_PATHS', text),
                             (row['assertion']['status'], row['assertion']['text']))
            self.assertEqual(['GUIDE', 'PROCESS_MAP'], [item['slot']['kind'] for item in row['occurrences']])
            for occurrence in row['occurrences']:
                self.assertEqual(['another-owner', 'outside-positive-rubric'], occurrence['owners'])
                self.assertEqual([[2, 2]], occurrence['sources'][0]['lines'])
            return identity

    def disposition(self, identity, status, reason):
        self.review['counterevidenceDigest'] = self.report['counterevidenceDigest']
        self.review['counterevidence'] = {identity: {
            'status': status, 'reason': reason,
            'sourceLinks': [{'file': 'writer.txt', 'startLine': 2, 'endLine': 2}]}}

    def test_full_positive_rubric_cannot_outvote_unqualified_negative_in_other_slots(self):
        identity = self.opposing_packets()
        self.assertEqual(96, sum(len(row['components']) for row in self.criteria.values()))
        self.assertEqual((45, {'m123:a', 'm1:writer (lines 2)'}), Q['coverage'](self.report, self.review))
        with self.assertRaisesRegex(ValueError, 'lacks its digest'):
            self.decision()
        self.review['counterevidenceDigest'] = self.report['counterevidenceDigest']
        with self.assertRaisesRegex(ValueError, 'Missing independent counterevidence dispositions'):
            self.decision()
        self.disposition(identity, 'MATERIAL_FALSE', 'Line 2 writes on the enabled path. The packet excludes it unconditionally.')
        decision = self.decision()
        self.assertEqual(('FAIL', 45, 1), (decision['status'], decision['full'], decision['counterevidence']['materialFalse']))

    def test_source_grounded_exclusion_and_stale_catalog(self):
        identity = self.opposing_packets(condition='Only in read-only mode, before starting the writer.')
        self.disposition(identity, 'SCOPED_EXCLUSION', 'The exclusion is confined to read-only mode. Line 2 is the enabled write.')
        self.assertEqual('PASS', self.decision()['status'])
        link = self.review['counterevidence'][identity]['sourceLinks'][0]
        link['endLine'] = 80
        with self.assertRaisesRegex(ValueError, 'Invalid counterevidence source range'):
            self.decision()
        link.update(startLine=3, endLine=3)
        with self.assertRaisesRegex(ValueError, 'must ground the delivered assertion'):
            self.decision()
        link.update(startLine=2, endLine=2)
        self.assertNotEqual(identity, self.opposing_packets(condition='Only after the first write completes.'))
        with self.assertRaisesRegex(ValueError, 'changed counterevidence catalog'):
            self.decision()

    def test_empty_counterevidence_needs_no_dispositions(self):
        decision = self.decision()
        self.assertEqual('PASS', decision['status'])
        self.assertEqual({'required': 0, 'reviewed': 0, 'materialFalse': 0, 'verifiedSourceLinks': 0},
                         decision['counterevidence'])
        self.disposition('undelivered assertion', 'SCOPED_EXCLUSION', 'A scoped exclusion.')
        with self.assertRaisesRegex(ValueError, 'Unknown counterevidence disposition'):
            self.decision()


class WeatherRetainedSourceTests(unittest.TestCase):
    def setUp(self):
        temporary = TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.run_dir = Path(temporary.name)
        self.author_source = self.run_dir / 'inputs/author-source'
        self.author_source.mkdir(parents=True)
        self.blob = b'first\nselected reply\nlast\n'
        self.row = {'id': 'source-1', 'service': 'dashboard', 'file': 'contracts/interface.json',
                    'revision': Q['REVISION'], 'startLine': 1, 'endLine': 3, 'textDigest': Q['sha'](self.blob),
                    'authority': 'DECLARED_OPENAPI'}
        self.authority = self.row['authority']
        self.fact = {'authority': 'DECLARED_OPENAPI', 'kind': 'openapi-declared-response', 'text': 'Declares a reply'}
        self.excerpt = {'service': 'dashboard', 'file': self.row['file'], 'startLine': 2, 'endLine': 2,
                        'text': 'selected reply\n'}
        self.manifest = {'id': Q['sha'](b'frozen fixture')}
        self.handle = 'm' + self.manifest['id'][7:19] + ':e1'
        self.packet_alias = self.handle
        self.anchor = f'{self.handle} (lines 2)'

    def verify(self, extra=None, negative_lines=None, review_links=()):
        (self.author_source / 'citations.json').write_text(json.dumps(self.manifest))
        plan = {'slots': [{'prompt': 'prompt.txt', 'citations': {'S1': self.packet_alias}}]}
        (self.run_dir / 'inputs/author-plan.json').write_text(json.dumps(plan))
        packet = {'citations': {'S1': {'region': 'V1', 'authority': self.authority}},
                  'sourceRanges': {'V1': self.excerpt}}
        if extra:
            packet['citations']['S1']['regions'] = ['V1', 'V2']
            packet['sourceRanges']['V2'] = extra
        (self.run_dir / 'inputs/prompt.txt').write_text('Frozen prompt\n\n' + json.dumps(packet))
        visible = {key: value for key, value in self.excerpt.items() if key != 'text'}
        visible.update(authority=self.authority, bodySha256=Q['sha'](self.excerpt['text'].encode()))
        fact = {'location': 'inputs/prompt.txt#entry/facts/0', 'anchors': [self.anchor],
                'locations': {self.handle: visible}, 'text': json.dumps(self.fact)}
        raw = {'outcomes': [{'source': self.row, 'textKey': 'only'}], 'texts': {'only': self.blob.decode()}}
        entry = {'source': self.row, 'manifestId': self.manifest['id'],
                 'scope': {'snapshot': 'saved', 'service': 'dashboard'}}
        loader = { 'load': lambda path: (self.manifest, raw, None, {self.handle: entry}) }
        negative = [] if negative_lines is None else [{'occurrences': [
            {'location': fact['location'], 'sources': [{**visible, 'handle': self.handle, 'lines': negative_lines}]}]}]
        with patch('runpy.run_path', return_value=loader), patch('subprocess.check_output', return_value=self.blob):
            return Q['check_sources'](self.run_dir, self.run_dir, self.run_dir, 'saved', {self.anchor}, [fact],
                                      negative, review_links)

    def test_declared_openapi_retains_exact_delivered_subrange(self):
        self.assertEqual(1, self.verify())

    def test_counterevidence_replays_packet_ranges_and_independent_source_links(self):
        self.assertEqual(1, self.verify(negative_lines=[[2, 2]], review_links=[(self.row['file'], 2, 3)]))
        with self.assertRaisesRegex(ValueError, 'Witness lines absent from its isolated packet'):
            self.verify(negative_lines=[[3, 3]])
        with self.assertRaisesRegex(ValueError, 'review source range unavailable'):
            self.verify(negative_lines=[[2, 2]], review_links=[(self.row['file'], 2, 4)])

    def test_exact_snapshot_can_deliver_an_anchor_in_a_secondary_region(self):
        self.row['authority'] = self.authority = 'EXACT_SNAPSHOT_TEXT'
        self.fact = {'authority': 'SOURCE_BODY', 'kind': 'return', 'text': 'Returns from implementation'}
        self.anchor = f'{self.handle} (lines 3)'
        extra = {**self.excerpt, 'startLine': 3, 'endLine': 3, 'text': 'last\n'}
        self.assertEqual(1, self.verify(extra))
        with self.assertRaisesRegex(ValueError, 'Witness lines absent from its isolated packet'):
            self.verify()

    def test_mutated_display_bytes_and_wrong_identity_are_refused(self):
        self.excerpt['text'] = 'changed reply\n'
        with self.assertRaisesRegex(ValueError, 'citation bytes differ from Git'):
            self.verify()
        self.excerpt.update(text='selected reply\n', file='contracts/another.json')
        with self.assertRaisesRegex(ValueError, 'citation source identity differs'):
            self.verify()

    def test_undelivered_handle_and_substituted_authority_are_refused(self):
        self.packet_alias = 'm123:another'
        with self.assertRaisesRegex(ValueError, 'citation not delivered to its packet'):
            self.verify()
        self.packet_alias = self.handle
        self.authority = 'EXACT_SNAPSHOT_TEXT'
        with self.assertRaisesRegex(ValueError, 'citation authority differs'):
            self.verify()

    def test_openapi_alone_cannot_qualify_implementation_and_retained_digest_is_required(self):
        self.fact['kind'] = 'return'
        with self.assertRaisesRegex(ValueError, 'cannot support an implementation statement'):
            self.verify()
        self.fact['kind'] = 'openapi-declared-response'
        self.row['textDigest'] = Q['sha'](b'different retained original')
        with self.assertRaisesRegex(ValueError, 'Digest mismatch'):
            self.verify()


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
            witness = {'mode': 'ISOLATED_PAGES', 'counterevidenceDigest': 'negative digest'}
            decision = {'status': 'PASS', 'full': 45, 'required': 45, 'materialFalseClaims': 0, 'verifiedAnchors': 30,
                        'counterevidence': {'required': 1, 'reviewed': 1, 'materialFalse': 0, 'verifiedSourceLinks': 1}}
            original_review = Q['sha'](review.read_bytes())
            computed = Q['release_report'](payload, base, inventory, witness, original_review, decision)
            report_file.write_text(json.dumps(computed))
            Q['validate_release_report'](report_file, computed)
            report_file.write_text(json.dumps(computed).replace('{', '{"status":"FAIL",', 1))
            with self.assertRaisesRegex(ValueError, 'Duplicate JSON key'):
                Q['validate_release_report'](report_file, computed)
            report_file.write_text(json.dumps(computed))
            changed = Q['release_report']({'archiveSha256': 'another archive'}, base, inventory, witness,
                                          original_review, decision)
            with self.assertRaisesRegex(ValueError, 'differs from exact archive'):
                Q['validate_release_report'](report_file, changed)
            review.write_text('{"criteria":{"test":"edited"}}')
            changed = Q['release_report'](payload, base, inventory, witness, Q['sha'](review.read_bytes()), decision)
            with self.assertRaisesRegex(ValueError, 'differs from exact archive'):
                Q['validate_release_report'](report_file, changed)
            changed = Q['release_report'](payload, base, inventory, {**witness, 'counterevidenceDigest': 'changed'},
                                          original_review, decision)
            with self.assertRaisesRegex(ValueError, 'differs from exact archive'):
                Q['validate_release_report'](report_file, changed)
            computed['counterevidence']['materialFalse'] = 1
            with self.assertRaisesRegex(ValueError, 'zero material false claims'):
                Q['validate_release_report'](report_file, computed)
            computed['counterevidence']['materialFalse'] = 0
            computed['full'] = 44
            with self.assertRaisesRegex(ValueError, '45/45'):
                Q['validate_release_report'](report_file, computed)


if __name__ == '__main__':
    unittest.main()
