#!/usr/bin/env python3
"""Owned source/author protocol fixtures; no models, capture or public writes."""
import json
import hashlib
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
D = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/docs_run.py'))
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))
RENDERER = Path(os.environ.get('CODECLEW_MARKDOWN_RENDERER', ROOT / 'skills/codeclew/examples/markdown-renderer'))


class DocsRunTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve(); self.run = self.root / 'run'; self.seed = self.root / 'seed'; self.seed.mkdir()
        self.service = 'reservations'; self.mode = self.root / 'mode'; self.mode.write_text('normal')
        self.source = F['source'](); self.source.update(service=self.service, revision='b' * 40, startLine=1, endLine=1, authority='SOURCE_SYNTAX')
        raw = self.source['text'].encode()
        self.source['occurrence'] = {'snapshot': 'owned-blob-snapshot', 'blob': hashlib.sha1(('blob ' + str(len(raw)) + '\0').encode() + raw).hexdigest(), 'startByte': 0, 'endByte': len(raw)}
        self.backend = self.root / 'author'
        self.backend.write_text('#!' + sys.executable + '\n' + f'''
import json,shutil,sys,time
from pathlib import Path
seed=Path({str(self.seed)!r});mode=Path({str(self.mode)!r}).read_text()
sys.stdin.read()
if mode!='missing-thread':
 print(json.dumps({{'type':'thread.started','thread_id':'other-test-session' if mode=='thread-switch' and 'resume' in sys.argv else 'owned-test-session'}}),flush=True)
if mode=='ambiguous-thread':print(json.dumps({{'type':'thread.started','thread_id':'second-test-session'}}),flush=True)
if mode=='interrupt':time.sleep(10)
draft=Path.cwd()/'draft'
if mode!='empty':
 for source in seed.iterdir():
  target=draft/source.name
  if target.is_dir():shutil.rmtree(target)
  elif target.exists():target.unlink()
  if source.is_dir():shutil.copytree(source,target)
  else:shutil.copyfile(source,target)
(draft/'report.json').write_text(json.dumps({{'status':'PASS','semanticTruthAssessed':True}}))
if mode=='repair-on-resume' and 'resume' in sys.argv:
 path=draft/'original.json';path.write_text(path.read_text().replace('[Missing](absent.html)','Repaired local link.'))
usage={{'input_tokens':5000 if mode=='high-cache' else 20,'cached_input_tokens':4990 if mode=='high-cache' else 10,'output_tokens':2000000 if mode=='budget' else 10}}
counter=Path.cwd()/'owned-cumulative-usage.json'
if 'resume' in sys.argv and counter.exists():
 previous=json.loads(counter.read_text());usage={{key:value+previous.get(key,0) for key,value in usage.items()}}
counter.write_text(json.dumps(usage))
print(json.dumps({{'type':'turn.completed','usage':usage}}))
sys.exit(23 if mode=='exit-failed' else 0)
''')
        self.backend.chmod(0o700)
        self.clew = self.root / 'clew'
        self.clew.write_text('#!' + sys.executable + '\n' + f'''
import json,sys
args=sys.argv[1:]
source={self.source!r}
if args==['--version']:print('clew 0.15.5')
elif args==['capabilities']:print(json.dumps({{'runtimeMode':'RELEASE','runtimeKey':'owned-test-only'}}))
elif 'prepare' in args:print(json.dumps({{'snapshot':{F['SNAPSHOT']!r},'work':'owned-work'}}))
else:
 identity=args[args.index('--source')+1]
 print(json.dumps({{'snapshot':{F['SNAPSHOT']!r},'items':[{{'kind':'SOURCE','id':identity,'record':source}}] if identity==source['id'] else []}}))
''')
        self.clew.chmod(0o700)
        self.job = {'schema': D['SCHEMA'], 'request': 'Document the reservation service', 'clew': str(self.clew),
                    'root': str(self.root / 'retained'), 'snapshot': F['SNAPSHOT'], 'services': [self.service], 'language': 'en',
                    'renderer': str(RENDERER), 'node': str(Path(shutil.which('node')).absolute()),
                    'author': {'executable': str(self.backend), 'model': 'owned-test-backend', 'reasoningEffort': 'max'},
                    'budget': {'wallSeconds': 30, 'maxModelTokens': 1000, 'maxRepairs': 0}}
        (self.root / 'retained').mkdir()
        self.job_path = self.root / 'job.json'; D['save'](self.job_path, self.job)
        coverage = {'kind': 'COVERAGE', 'revision': self.source['revision'], 'coverage': 'SYNTAX', 'boundaries': []}
        entry = {'kind': 'THREAD_ENTRY', 'record': {'classification': 'DISCOVERED_TRIGGER',
                 'sourceIds': [self.source['id']], 'rootDeclarations': ['owned-symbol'],
                 'entrypoint': {'id': 'reserve', 'kind': 'HTTP_ENDPOINT', 'sourceIds': [self.source['id']],
                 'trigger': {'authority': 'OWNED_TEST_DECLARATION', 'sourceRoutes': [{'method': 'POST', 'path': '/reservations'}]}}}}
        file = {'kind': 'THREAD_FILE', 'record': {'path': self.source['file'], 'input': {'coverage': 'SYNTAX', 'digest': 'sha256:owned-syntax-fingerprint-not-a-text-hash'}}}
        def collect(launcher, root, work, output, **kwargs):
            output.mkdir(); D['save'](output / 'items.json', [entry, file] if kwargs.get('selection') else [coverage])
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}
        with patch.dict(D['T'], {'collect': collect}), patch.dict(D['prepare'].__globals__, {
                'prepare_sources': lambda job, inventory, directory: directory.mkdir()}):
            prepared = D['prepare'](self.job_path, self.run)
        self.assertEqual(prepared['status'], 'PREPARED')
        self.inventory = D['load'](self.run / 'inputs/inventory.json')
        self.entry = next(r for r in self.inventory['requirements'] if r['kind'] == 'ENTRY')
        (self.seed / 'reads').mkdir()
        self.manifest, view, _ = F['fixture'](self.seed / 'reads', service=self.service, work='owned-work', record=self.source)
        self.handle = view['records'][0]['cite']
        diagram = {'title': 'Reserve an item', 'participants': [{'id': 'caller', 'title': 'Caller'},
                   {'id': self.entry['handlerId'], 'title': 'Reservation handler', 'target': self.entry['page'] + '#handler'}],
                   'steps': [{'from': 'caller', 'to': self.entry['handlerId'], 'label': 'POST /reservations', 'kind': 'call', 'target': self.entry['page'] + '#handler'},
                             {'from': self.entry['handlerId'], 'to': 'caller', 'label': 'Return result', 'kind': 'return'}]}
        self.original = {'pages': [{'page': self.entry['page'], 'fragments': [{'text': '# Reserve\n\n<a id="handler"></a>\n\n'},
                         {'id': 'entry', 'text': 'Accept a reservation.\n\n```clew-sequence\n' + json.dumps(diagram) + '\n```\n', 'evidence': [self.handle]}]},
                         {'page': self.service + '.md', 'fragments': [{'text': '# Reservations\n\n'},
                          {'id': 'overview', 'text': 'Retained reservation data and effects.\n', 'evidence': [self.handle]}]}]}
        self.navigation = {'schema': 'codeclew-markdown-navigation/1', 'home': {'title': 'Reservations', 'target': self.service + '.md'},
                           'services': [{'id': self.service, 'title': 'Reservations', 'overview': self.service + '.md',
                                         'pages': [self.entry['page'], self.service + '.md'], 'starts': [{'title': 'Reserve', 'target': self.entry['page'] + '#handler', 'kind': 'http', 'summary': 'Accept a reservation'}], 'data': []}], 'routes': []}
        self.review = {'schema': D['REVIEW'], 'requirements': [], 'claims': [], 'unresolved': [], 'visual': {'status': 'UNAVAILABLE', 'reason': 'Owned non-browser unit fixture'}}
        for r in self.inventory['requirements']:
            account = {'requirement': r['id'], 'page': r['page'], 'section': 'handler' if r['kind'] == 'ENTRY' else 'reservations',
                       'disposition': 'EXPLAINED', 'evidence': [self.handle]}
            if r['kind'] == 'ENTRY':account['diagram'] = 0
            if r['kind'] == 'DISCOVERY':account.update(discoveryDisposition='NO_ADDITIONAL_STARTS', discoveredEntries=[])
            self.review['requirements'].append(account)
        self.review['claims'] = [{'page': p['page'], 'claim': f['id'], 'evidence': f['evidence'], 'disposition': 'SUPPORTED'}
                                 for p in self.original['pages'] for f in p['fragments'] if 'id' in f]
        self.seed_files()

    def seed_files(self):
        for name, value in [('original.json', self.original), ('navigation.json', self.navigation), ('self-review.json', self.review)]:
            D['save'](self.seed / name, value)

    def generated(self):
        return D['author'](self.run)

    def check(self, debug=False):
        return D['verify'](self.run, debug=debug)

    def projection_input(self, rows):
        outcomes, entries, texts = [], {}, {}
        for number, (text, first, blob) in enumerate(rows):
            source = {k:v for k,v in self.source.items() if k != 'text'}
            source.update(id='source-' + str(number), textDigest=D['sha'](text.encode()),
                          occurrence={'snapshot': 'owned-occurrence', 'blob': blob,
                                      'startByte': first, 'endByte': first + len(text.encode())})
            handle = 'owned:e' + str(number)
            entries[handle] = {'source': source}
            outcomes.append({'source': source, 'textKey': source['textDigest'],
                             'sourceRecordDigest': D['digest'](source)})
            texts[source['textDigest']] = text
        return {'outcomes': outcomes, 'texts': texts, 'scope': {'snapshot': F['SNAPSHOT']}}, entries

    def test_prepared_projection_preserves_all_cites_and_exact_overlaps(self):
        packet, entries = self.projection_input([('abcdef\n', 0, 'a' * 40), ('cde', 2, 'a' * 40)])
        regions, bindings, _ = D['source_projection'](packet, entries)
        self.assertEqual(len(regions), 1)
        self.assertEqual(set(bindings), set(entries))
        for handle, binding in bindings.items():
            self.assertEqual(binding['source'], entries[handle]['source'])
            span = binding['byteSlice']
            raw = regions[binding['region']]['text'].encode()[span['startByte']:span['endByte']]
            self.assertEqual(D['sha'](raw), binding['source']['textDigest'])

    def test_prepared_projection_keeps_same_file_distinct_occurrences_and_conflicts(self):
        for rows in [[('abc', 0, 'a' * 40), ('abc', 0, 'b' * 40)],
                     [('abc', 0, 'a' * 40), ('xyz', 0, 'a' * 40)]]:
            packet, entries = self.projection_input(rows)
            regions, bindings, _ = D['source_projection'](packet, entries)
            self.assertEqual(len(regions), 2)
            self.assertEqual(set(bindings), set(entries))
            for handle, binding in bindings.items():
                self.assertEqual(binding['source'], entries[handle]['source'])

    def test_prepared_projection_rejects_changed_text_and_ambiguous_reference_lists(self):
        packet, entries = self.projection_input([('abc', 0, 'a' * 40)])
        packet['texts'][next(iter(packet['texts']))] = 'changed'
        with self.assertRaises(D['Stop']):
            D['source_projection'](packet, entries)
        with self.assertRaises(D['Stop']):
            D['compiler_source_pairs']([{'id':'symbol', 'sourceIds':['s1', 's2'], 'sourceReferences':['e1']}])
        with self.assertRaises(D['Stop']):
            D['compiler_source_pairs']([{'id':'one', 'sourceIds':['s1'], 'sourceReferences':['e1']},
                                        {'id':'two', 'sourceIds':['s1'], 'sourceReferences':['e2']}])

    def test_prepare_keeps_discovery_when_native_has_no_triggers(self):
        scope = self.inventory['services'][self.service]
        def collect(launcher, root, work, output, **kwargs):
            output.mkdir()
            D['save'](output / 'items.json', scope['sourceInventory'] if kwargs.get('selection') else [scope['coverage']])
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}
        run = self.root / 'discovery-only'
        with patch.dict(D['T'], {'collect': collect}), patch.dict(D['prepare'].__globals__, {
                'prepare_sources': lambda job, inventory, directory: directory.mkdir()}):
            self.assertEqual(D['prepare'](self.job_path, run)['status'], 'PREPARED')
        rows = D['load'](run / 'inputs/inventory.json')['requirements']
        self.assertEqual([(r['kind'], r['page']) for r in rows],
                         [('DISCOVERY', self.service + '.md'), ('SERVICE', self.service + '.md')])
        self.assertEqual(rows[0]['file'], self.source['file'])

    def test_required_page_limit_matches_compose_before_body_reads(self):
        scope = self.inventory['services'][self.service]
        limit = D['M']['MAX_PAGES']
        calls = []
        for routes in (limit - 1, limit):
            entry = {'kind': 'THREAD_ENTRY', 'record': {'classification': 'DISCOVERED_TRIGGER',
                       'entrypoint': {'id': 'many', 'kind': 'HTTP_ENDPOINT', 'sourceIds': [self.source['id']],
                                      'trigger': {'sourceRoutes': [{'method': 'GET', 'path': '/route-' + str(i)}
                                                                   for i in range(routes)]}}}}
            def collect(launcher, root, work, output, **kwargs):
                output.mkdir()
                D['save'](output / 'items.json', [entry] if kwargs.get('selection') else [scope['coverage']])
                return {'status': 'COMPLETE', 'omittedRecords': 0,
                        'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}
            run = self.root / ('route-limit-' + str(routes))
            with patch.dict(D['T'], {'collect': collect}), patch.dict(D['prepare'].__globals__, {
                    'prepare_sources': lambda job, inventory, directory: calls.append(directory.mkdir())}):
                if routes < limit:
                    self.assertEqual(D['prepare'](self.job_path, run)['status'], 'PREPARED')
                else:
                    with self.assertRaises(D['Stop']) as raised:
                        D['prepare'](self.job_path, run)
                    self.assertEqual(raised.exception.code, 'REQUIRED_PAGE_LIMIT')
                    self.assertEqual(raised.exception.details['requiredPages'], limit + 1)
                    self.assertEqual(raised.exception.details['limit'], limit)
                    self.assertIn('entry pages', raised.exception.details['nextAction'])
            rows = D['load'](run / 'inputs/inventory.json')['requirements']
            self.assertEqual(len({r['page'] for r in rows}), routes + 1)
        self.assertEqual(len(calls), 1)

    def test_prepare_reads_executable_and_indexes_128_file_only_resources(self):
        file = self.source['file']
        files = ['resources/file-' + str(i) + '.txt' for i in range(128)]
        scope = self.inventory['services'][self.service]
        scope['sourceInventory'] += [{'kind': 'THREAD_SCOPE'}] + [
            {'kind': 'THREAD_FILE', 'record': {'path': path, 'input': {'coverage': 'FILE_ONLY'}}}
            for path in files]
        calls = []
        def command(argv, directory, label, **kwargs):
            calls.append(label)
            out = Path(argv[argv.index('--output-dir') + 1])
            if label == 'discover':
                out.mkdir()
                D['save'](out / 'packet.json', {'queryComplete': True, 'choices': [{'file': path} for path in [file] + files],
                                              'scope': {'snapshot': F['SNAPSHOT']}})
            else:
                self.assertEqual(D['load'](argv[argv.index('--selection') + 1]), [{'file': file}])
                self.assertEqual(argv[argv.index('--packet') + 1], str(directory / 'discovery/packet.json'))
                shutil.copytree(self.manifest.parent, out)
        directory = self.root / 'source-preparation'
        with patch.dict(D['_prepare_sources'].__globals__, {'command': command}):
            result = D['prepare_sources'](self.job, self.inventory, directory)
        self.assertEqual(calls, ['discover', 'read'])
        self.assertEqual(result['sourceFiles'], 1)
        self.assertEqual(result['services'][self.service]['deferredResourceFiles'], sorted(files))
        self.assertIn('COMPLETE_RETAINED_GIT_BLOB', (directory / 'index.json').read_text())
        index = (directory / self.service / 'index.md').read_text()
        self.assertIn('read-files with discovery/packet.json', index)
        self.assertTrue(all('- ' + file + '\n' in index for file in files))

    def test_preparation_file_limit_refuses_before_body_reads(self):
        calls = []
        def command(argv, directory, label, **kwargs):
            calls.append(label)
            out = Path(argv[argv.index('--output-dir') + 1]); out.mkdir()
            D['save'](out / 'packet.json', {'queryComplete': True, 'choices': [{'file':str(i)} for i in range(129)]})
        inventory = json.loads(json.dumps(self.inventory))
        inventory['services'][self.service]['sourceInventory'] = [{'kind':'THREAD_SCOPE'}]
        with patch.dict(D['_prepare_sources'].__globals__, {'command':command}):
            with self.assertRaises(D['Stop']) as raised:
                D['prepare_sources'](self.job, inventory, self.root / 'bounded-prepare')
        self.assertEqual(raised.exception.code, 'SOURCE_PREPARATION_FILE_LIMIT')
        self.assertEqual(calls, ['discover'])

    def test_preparation_wall_limit_and_command_descendants_stop(self):
        import signal
        timer = signal.setitimer
        def short_timer(kind, seconds, interval=0):
            return timer(kind, 0.02 if seconds == 300 else seconds, interval)
        with patch.object(signal, 'setitimer', short_timer), patch.dict(D['prepare_sources'].__globals__, {
                '_prepare_sources': lambda *args: time.sleep(1)}):
            with self.assertRaises(D['Stop']) as raised:
                D['prepare_sources'](None, None, None)
        self.assertEqual(raised.exception.code, 'SOURCE_PREPARATION_WALL_LIMIT')
        marker, ready = self.root / 'late-child', self.root / 'child-ready'
        script = 'import os,time;from pathlib import Path\nif os.fork()==0:\n Path(' + repr(str(ready)) + ').write_text("ready");time.sleep(1);Path(' + repr(str(marker)) + ').write_text("orphan")\nelse:time.sleep(5)'
        with self.assertRaises(D['Stop']):
            D['command']([sys.executable, '-c', script], self.root / 'timeout', 'child', timeout=0.5)
        self.assertTrue(ready.exists())
        time.sleep(0.6)
        self.assertFalse(marker.exists())

    def test_valid_owned_case_promotes_with_stable_html_and_keeps_debug_decisions(self):
        self.generated(); normal = self.check(); self.assertEqual(normal['status'], 'READY', normal)
        debug = self.check(debug=True); self.assertEqual(debug['status'], 'READY', debug)
        self.assertEqual(normal['deliverableFiles'], debug['deliverableFiles'])
        self.assertEqual({k:v['status'] for k,v in normal['checks'].items()}, {k:v['status'] for k,v in debug['checks'].items()})
        promoted = D['promote'](self.run)
        self.assertEqual(promoted['status'], 'READY_PROMOTED')
        self.assertFalse(normal['semanticTruthAssessed']); self.assertTrue(normal['semanticReviewRecorded'])
        self.assertEqual(normal['authorVisualReview']['status'], 'UNAVAILABLE')

    def test_no_native_entry_supports_evidenced_discovery_with_or_without_starts(self):
        self.inventory['requirements'].remove(self.entry)
        D['save'](self.run / 'inputs/inventory.json', self.inventory)
        D['save'](self.run / 'workspace/inventory.json', self.inventory)
        pins = D['load'](self.run / 'pins.json')
        pins.update(inputFiles=D['tree'](self.run / 'inputs'), inventoryDigest=D['digest'](self.inventory))
        D['save'](self.run / 'pins.json', pins)

        for has_start in (False, True):
            with self.subTest(has_start=has_start):
                page = self.service + '.md'
                target = page + '#handler'
                fragments = [{'text': '# Reservations\n\n<a id="handler"></a>\n\n'},
                             {'id': 'coverage', 'text': 'Scope explained here.\n', 'evidence': [self.handle]}]
                accounts = [{**row, 'section': 'handler'} for row in self.review['requirements'] if row['page'] == page]
                discovery = next(row for row in accounts if 'discoveryDisposition' in row)
                starts = []
                if has_start:
                    diagram = {'title': 'On invoice.ready', 'participants': [{'id': 'event', 'title': 'Event'},
                        {'id': 'discovered-invoice', 'title': 'Listener', 'target': target}],
                        'steps': [{'from': 'event', 'to': 'discovered-invoice', 'label': 'invoice.ready',
                                   'kind': 'async', 'target': target}]}
                    fragments.append({'text': '\n```clew-sequence\n' + json.dumps(diagram) + '\n```\n'})
                    starts.append({'title': 'Invoice ready', 'target': target, 'kind': 'other',
                                   'summary': 'Source shows a registered callback.'})
                    discovery.update(discoveryDisposition='ADDITIONAL_STARTS', discoveredEntries=[
                        {'id': 'invoice', 'kind': 'OTHER', 'page': page, 'section': 'handler', 'diagram': 0,
                         'evidence': [self.handle], 'disposition': 'EXPLAINED'}])
                D['save'](self.seed / 'original.json', {'pages': [{'page': page, 'fragments': fragments}]})
                navigation = {**self.navigation, 'services': [{**self.navigation['services'][0],
                                                               'pages': [page], 'starts': starts}]}
                D['save'](self.seed / 'navigation.json', navigation)
                D['save'](self.seed / 'self-review.json', {**self.review, 'requirements': accounts,
                    'claims': [{'page': page, 'claim': 'coverage', 'evidence': [self.handle], 'disposition': 'SUPPORTED'}]})
                D['save'](self.run / 'state.json', {'attempts': [], 'authorStartedAt': None})
                shutil.rmtree(self.run / 'attempts', ignore_errors=True)
                self.generated()
                result = self.check()
                self.assertEqual(result['status'], 'READY', result)
                self.assertEqual(len(result['checks']['entrySequences']['result']['entries']), int(has_start))

    def test_frozen_prepared_manifests_bind_without_author_copy_or_extra_reads(self):
        prepared = self.run / 'inputs/author-source/read'
        shutil.move(str(self.manifest.parent), prepared)
        pins = D['load'](self.run / 'pins.json')
        pins['inputFiles'] = D['tree'](self.run / 'inputs')
        D['save'](self.run / 'pins.json', pins)
        self.generated()
        self.assertFalse(list((self.run / 'workspace/draft/reads').rglob('citations.json')))
        result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertTrue(prepared.joinpath('citations.json').is_file())

    def test_high_cached_input_uses_uncached_budget_in_author_and_verify(self):
        self.mode.write_text('high-cache')
        state = self.generated()
        self.assertEqual(state['modelTokens'], 20)
        self.assertEqual(state['tokenAccounting']['totalInputOutputFootprint'], 5010)
        result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertEqual(result['checks']['author']['reportedModelTokens'], 20)

    def compact_assessment(self):
        compact = D['review_template'](self.inventory, self.original)
        for account in self.review['requirements']:
            row = compact['requirements'][account['requirement']]
            row['disposition'] = account['disposition']
            row['section'] = '#' + account['section']
            if account.get('discoveryDisposition'):
                row.update(discoveryDisposition=account['discoveryDisposition'], discoveredEntries=account['discoveredEntries'])
        for account in self.review['claims']:
            compact['claims'][account['page']][account['claim']] = account['disposition']
        compact['unresolved'] = self.review['unresolved']
        compact['visual'] = self.review['visual']
        return compact

    def test_compact_assessment_derives_only_redundant_ids_pages_and_evidence(self):
        original_review = json.dumps(self.review, sort_keys=True)
        self.review = self.compact_assessment()
        self.assertFalse(any('evidence' in row or 'page' in row for row in self.review['requirements'].values()))
        self.seed_files(); self.generated()
        result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        resolved = (self.run / result['output']).parent / 'resolved-self-review.json'
        canonical = D['load'](resolved)
        self.assertEqual(json.dumps(canonical, sort_keys=True), original_review)
        self.assertTrue(all(row['section'].startswith('#') for row in self.review['requirements'].values()))

    def test_starter_supplies_working_pages_navigation_and_http_entry_for_authored_review(self):
        draft = self.run / 'workspace/draft'
        starter = self.run / 'inputs/starter'
        original = D['load'](starter / 'original.json')
        navigation = D['load'](starter / 'navigation.json')
        pages = {page['page']: page['fragments'] for page in original['pages']}
        entry = self.entry['page']
        overview = self.service + '.md'
        self.assertEqual(set(pages), {overview, entry})
        self.assertIn(entry.removesuffix('.md') + '.html#handler', pages[overview][1]['text'])
        sequence = json.loads(pages[entry][-1]['text'].split('\n', 1)[1].rsplit('```', 1)[0])
        first = sequence['steps'][0]
        self.assertEqual((first['label'], first['to'], first['target']),
                         ('POST /reservations', self.entry['handlerId'], entry + '#handler'))
        self.assertEqual(navigation['services'][0]['starts'][0]['target'], entry + '#handler')
        self.assertFalse(any('id' in item for fragments in pages.values() for item in fragments))

        pages[overview].append({'id': 'overview', 'text': '\nA bounded overview.\n', 'evidence': [self.handle]})
        pages[entry].insert(1, {'id': 'operation', 'text': 'Accept a reservation.\n\n', 'evidence': [self.handle]})
        sequence['steps'].append({'from': self.entry['handlerId'], 'to': 'caller',
                                  'label': 'Return a result', 'kind': 'return'})
        pages[entry][-1]['text'] = '```clew-sequence\n' + json.dumps(sequence) + '\n```\n'
        D['save'](draft / 'original.json', original)
        review = D['review_for_run'](self.run)
        self.assertEqual(review['requirements'][self.entry['id']]['section'], 'handler')
        for row in self.inventory['requirements']:
            assessment = review['requirements'][row['id']]
            self.assertIsNone(assessment['disposition'])
            if row['kind'] != 'ENTRY':
                self.assertEqual(assessment['section'], 'service-scope')
            if row['kind'] == 'DISCOVERY':
                self.assertIsNone(assessment['discoveryDisposition'])
                assessment.update(discoveryDisposition='NO_ADDITIONAL_STARTS', discoveredEntries=[])
            assessment['disposition'] = 'EXPLAINED'
        self.assertIsNone(review['visual']['status'])
        for assessments in review['claims'].values():
            for key in assessments:
                self.assertIsNone(assessments[key])
                assessments[key] = 'SUPPORTED'
        review.update(unresolved=[], visual={'status': 'UNAVAILABLE', 'reason': 'Owned non-browser fixture'})
        for name, value in [('original.json', original), ('navigation.json', navigation), ('self-review.json', review)]:
            D['save'](self.seed / name, value)
        self.generated()
        self.assertEqual(self.check()['status'], 'READY')

    def test_template_has_no_semantic_pass_or_discovery_outcome(self):
        review = D['review_template'](self.inventory, self.original)
        self.assertTrue(all(row['disposition'] is None for row in review['requirements'].values()))
        self.assertTrue(all(value is None for claims in review['claims'].values() for value in claims.values()))
        self.assertIsNone(review['unresolved'])
        self.assertIsNone(review['visual']['status'])
        for requirement in self.inventory['requirements']:
            if requirement['kind'] == 'DISCOVERY':
                self.assertIsNone(review['requirements'][requirement['id']]['discoveryDisposition'])
                self.assertIsNone(review['requirements'][requirement['id']]['discoveredEntries'])
        self.review = review; self.seed_files(); self.generated()
        self.assertEqual(self.check()['status'], 'FAILED')

    def test_compact_contract_rejects_foreign_source_ids_and_cross_page_sections(self):
        baseline = json.loads(json.dumps(self.review))
        for mutation in ('raw-source-id', 'foreign-cite', 'cross-page-anchor', 'missing-answer', 'missing-claim'):
            self.review = baseline
            compact = self.compact_assessment()
            account = compact['requirements'][self.entry['id']]
            if mutation == 'raw-source-id':account['evidence'] = [self.source['id']]
            if mutation == 'foreign-cite':account['evidence'] = ['unselected:e1']
            if mutation == 'cross-page-anchor':account['section'] = 'other.md#handler'
            if mutation == 'missing-answer':account['disposition'] = None
            if mutation == 'missing-claim':compact['claims'][self.entry['page']].clear()
            self.review = compact; self.seed_files(); self.generated_for_test_reset()
            self.assertEqual(self.check()['status'], 'FAILED', mutation)

    def test_compact_source_overrides_and_noncanonical_mapping_stay_author_choices(self):
        compact = self.compact_assessment()
        account = compact['requirements'][self.entry['id']]
        account.update(evidence=[self.handle], section='#chosen-section', diagram=2)
        entries = {self.handle:{'source':self.source}}
        canonical = D['resolve_review'](compact, self.inventory, entries, self.original)
        selected = next(row for row in canonical['requirements'] if row['requirement']==self.entry['id'])
        self.assertEqual(selected['evidence'], [self.handle])
        self.assertEqual(selected['section'], 'chosen-section')
        self.assertEqual(selected['diagram'], 2)
        self.assertEqual(canonical['claims'][0]['evidence'], self.original['pages'][0]['fragments'][1]['evidence'])

    def test_template_command_is_read_only_and_works_under_active_author_lock(self):
        import fcntl
        import subprocess
        D['save'](self.run / 'workspace/draft/original.json', self.original)
        with (self.run / '.runner.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            before = D['tree'](self.run)
            result = subprocess.run([sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'),
                                     'review-template', '--run-dir', str(self.run)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        template = json.loads(result.stdout)
        self.assertEqual(template['schema'], D['COMPACT_REVIEW'])
        self.assertTrue(all(row['disposition'] is None for row in template['requirements'].values()))
        self.assertEqual(D['tree'](self.run), before)
        self.assertFalse((self.run / 'ready').exists())

    def test_whole_document_initial_author_and_same_thread_repair_remain_the_default(self):
        self.mode.write_text('repair-on-resume')
        self.original['pages'][0]['fragments'][1]['text']+='\n[Missing](absent.html)\n'
        self.seed_files()
        job=D['load'](self.run/'inputs/job.json');job['budget']['maxRepairs']=1
        D['save'](self.run/'inputs/job.json',job)
        pins=D['load'](self.run/'pins.json');pins['jobDigest']=D['digest'](job);pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        result=D['execute'](self.run);self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json');self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['modelTokens'],40)
        self.assertEqual([a['threadId'] for a in state['attempts']],['owned-test-session']*2)
        self.assertEqual([a['budgetTokens'] for a in state['tokenAccounting']['attemptUsage']],[20,20])
        self.assertNotIn('resume',state['attempts'][0]['argv'])
        args=state['attempts'][1]['argv'];self.assertEqual(args[args.index('resume')+1],'owned-test-session')
        self.assertEqual((self.run/'attempts/0/prompt.txt').read_bytes(),(self.run/'prompt.txt').read_bytes())
        self.assertEqual(len(D['load'](self.run/'attempts/0/draft/original.json')['pages']),2)
        self.assertIn('[Missing]',(self.run/'attempts/0/draft/original.json').read_text())
        self.assertNotIn('[Missing]',(self.run/'attempts/1/draft/original.json').read_text())
        self.assertEqual(D['load'](self.run/'attempts/0/draft/original.json')['pages'][1],D['load'](self.run/'attempts/1/draft/original.json')['pages'][1])

    def test_resume_thread_mismatch_preserves_the_report_but_cannot_qualify(self):
        self.generated();self.mode.write_text('thread-switch')
        job=D['load'](self.run/'inputs/job.json');job['budget']['maxRepairs']=1;D['save'](self.run/'inputs/job.json',job)
        pins=D['load'](self.run/'pins.json');pins['jobDigest']=D['digest'](job);pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        with self.assertRaises(D['Stop']) as error:self.generated()
        self.assertEqual(error.exception.code,'AUTHOR_THREAD_CHANGED')
        state=D['load'](self.run/'state.json')
        self.assertEqual(state['attempts'][1]['threadId'],'other-test-session')
        self.assertEqual(state['attempts'][1]['usage'],{'input_tokens':40,'cached_input_tokens':20,'output_tokens':20})
        self.assertNotEqual(self.check()['status'],'READY')

    def test_missing_or_ambiguous_new_thread_attribution_cannot_qualify(self):
        for mode in ('missing-thread','ambiguous-thread'):
            self.mode.write_text(mode)
            with self.assertRaises(D['Stop']) as error:self.generated_for_test_reset()
            self.assertEqual(error.exception.code,'AUTHOR_THREAD_REQUIRED')
            state=D['load'](self.run/'state.json')
            self.assertEqual(state['attempts'][0]['usage'],{'input_tokens':20,'cached_input_tokens':10,'output_tokens':10})
            self.assertNotEqual(self.check()['status'],'READY')

    def test_review_template_never_probes_native_but_verify_and_promote_probe_parser(self):
        self.generated();accepted=self.check();self.assertEqual(accepted['status'],'READY')
        before=D['tree'](self.run)
        with patch.dict(D['review_for_run'].__globals__,{'command':lambda *a,**k:self.fail('read-only template must not probe runtime')}):
            self.assertEqual(D['review_for_run'](self.run)['schema'],D['COMPACT_REVIEW'])
        self.assertEqual(before,D['tree'](self.run))
        from types import SimpleNamespace
        with patch.object(D['subprocess'],'run',return_value=SimpleNamespace(returncode=2,stdout=b'')):
            failed=self.check();self.assertEqual(failed['checks']['inputs']['code'],'RENDERER_PARSER_CHANGED')
            D['save'](self.run/'report.json',accepted)
            with self.assertRaises(D['Stop']) as error:D['promote'](self.run)
            self.assertEqual(error.exception.code,'PROMOTION_RECHECK_FAILED')
        self.assertFalse((self.run/'ready').exists())

    def test_recorded_v2_usage_still_exceeds_budget_after_metric_correction(self):
        usage = {'input_tokens':5089868, 'cached_input_tokens':4811904,
                 'output_tokens':167582, 'reasoning_output_tokens':96885, 'cache_write_input_tokens':0}
        result = D['token_accounting']([{'usage':usage}])
        self.assertEqual(result['status'], 'MEASURED')
        self.assertEqual(result['budgetTokens'], 445546)
        self.assertGreater(D['budget_tokens'](result), 250000)
        self.assertEqual(result['totalInputOutputFootprint'], 5257450)
        self.assertEqual(result['rawTotals']['reasoning_output_tokens'], 96885)
        self.assertEqual(usage['cached_input_tokens'], 4811904)

    def test_usage_accumulates_author_and_repairs_without_double_counting_subsets(self):
        result = D['token_accounting']([
            {'threadId':'one','usage':{'input_tokens':5000,'cached_input_tokens':4990,'output_tokens':10,'reasoning_output_tokens':4}},
            {'threadId':'two','usage':{'input_tokens':30,'cached_input_tokens':5,'output_tokens':7,'reasoning_output_tokens':2}}])
        self.assertEqual(result['budgetTokens'], 52)
        self.assertEqual(result['rawTotals'], {'input_tokens':5030,'cached_input_tokens':4995,
                                             'output_tokens':17,'reasoning_output_tokens':6})
        self.assertEqual(result['totalInputOutputFootprint'], 5047)

    def test_missing_cache_counter_stays_explicitly_unavailable_across_repairs(self):
        result = D['token_accounting']([
            {'threadId':'one','usage':{'input_tokens':100,'output_tokens':20}},
            {'threadId':'two','usage':{'input_tokens':50,'cached_input_tokens':45,'output_tokens':10}}])
        self.assertEqual(result['status'], 'UNAVAILABLE')
        self.assertIsNone(result['budgetTokens'])
        self.assertIsNone(result['rawTotals']['cached_input_tokens'])
        self.assertIsNone(result['rawTotals']['reasoning_output_tokens'])
        self.assertEqual(result['missing'], [{'attempt':0,'fields':['cached_input_tokens']}])
        self.assertEqual(D['budget_tokens'](result), 35)

    def test_missing_cache_cannot_pass_budget_ready_or_start_another_author(self):
        state = self.generated()
        state['attempts'][0]['usage'].pop('cached_input_tokens')
        D['save'](self.run / 'state.json', state)
        result = self.check()
        self.assertEqual(result['status'], 'BLOCKED', result)
        self.assertEqual(result['checks']['author']['code'], 'AUTHOR_USAGE_UNAVAILABLE')
        with self.assertRaises(D['Stop']) as raised:
            D['author'](self.run)
        self.assertEqual(raised.exception.code, 'AUTHOR_USAGE_UNAVAILABLE')
        self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']), 1)
        self.assertFalse((self.run / 'ready').exists())

    def test_missing_cache_with_known_over_budget_lower_bound_still_fails(self):
        state = self.generated()
        state['attempts'][0]['usage'].pop('cached_input_tokens')
        state['attempts'][0]['usage']['output_tokens'] = 1001
        D['save'](self.run / 'state.json', state)
        result = self.check()
        self.assertEqual(result['status'], 'FAILED')
        self.assertEqual(result['checks']['author']['code'], 'AUTHOR_TOKEN_BUDGET_EXCEEDED')

    def test_invalid_usage_cannot_be_reported_as_measured_uncached_tokens(self):
        for cache in [-1, 101, True, '90', None]:
            result = D['token_accounting']([{'usage':{'input_tokens':100,'cached_input_tokens':cache,'output_tokens':10}}])
            self.assertEqual(result['status'], 'INVALID', cache)
            self.assertIsNone(result['budgetTokens'])
        for usage in ['invalid', {'input_tokens':100,'cached_input_tokens':90,'output_tokens':10,'reasoning_output_tokens':11}]:
            self.assertEqual(D['token_accounting']([{'usage':usage}])['status'], 'INVALID')

    def test_empty_exit_zero_and_author_pass_report_never_replace_ready(self):
        ready = self.run / 'ready'; ready.mkdir(); (ready / 'current.json').write_text('previous accepted output')
        self.mode.write_text('empty'); self.generated(); result = self.check()
        self.assertEqual(result['status'], 'FAILED'); self.assertEqual(result['checks']['output']['code'], 'MISSING_REQUIRED_OUTPUT')
        self.assertEqual((ready / 'current.json').read_text(), 'previous accepted output')

    def test_broken_link_is_not_overridden_by_author_pass(self):
        self.original['pages'][0]['fragments'][1]['text'] += '\n[Missing](absent.html#lost)\n'
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['status'], 'FAILED'); self.assertEqual(result['checks']['finalize']['status'], 'FAIL')

    def test_heading_empty_or_unrelated_diagram_refuses_ready(self):
        for replacement in ['## Handler-started sequence\n\n1. Accept\n', '```clew-sequence\n{}\n```\n']:
            self.original['pages'][0]['fragments'][1]['text'] = replacement
            self.seed_files(); self.generated_for_test_reset(); result = self.check()
            self.assertEqual(result['status'], 'FAILED', result)
            self.assertEqual(result['checks']['entrySequences']['status'], 'FAIL', result)

    def generated_for_test_reset(self):
        state = D['load'](self.run / 'state.json'); state.update(attempts=[], authorStartedAt=None, status='PREPARED', modelTokens=0, usageAvailable=True)
        D['save'](self.run / 'state.json', state)
        shutil.rmtree(self.run / 'attempts', ignore_errors=True)
        return self.generated()

    def test_exit_zero_with_missing_required_page_is_rejected(self):
        self.original['pages'] = [p for p in self.original['pages'] if p['page'] != self.entry['page']]
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['checks']['output']['code'], 'MISSING_REQUIRED_PAGE')

    def test_deleted_requirement_and_foreign_snapshot_are_refused(self):
        self.review['requirements'].pop(); self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['checks']['selfReview']['code'], 'REQUIREMENT_ACCOUNTING_MISMATCH')
        job = D['load'](self.run / 'inputs/job.json'); job['snapshot'] = 'sha256:other/2'; D['save'](self.run / 'inputs/job.json', job)
        self.assertEqual(self.check()['checks']['inputs']['code'], 'FROZEN_INPUTS_CHANGED')

    def test_changed_bytes_after_check_cannot_be_promoted(self):
        self.generated(); result = self.check(); self.assertEqual(result['status'], 'READY', result)
        path = self.run / result['output'] / 'html' / str(Path(self.entry['page']).with_suffix('.html'))
        path.write_text(path.read_text() + '<p>Changed after check</p>')
        with self.assertRaises(D['Stop']) as caught:D['promote'](self.run)
        self.assertEqual(caught.exception.code, 'ACCEPTED_OUTPUT_CHANGED')

    def test_nonzero_exit_and_token_budget_cannot_be_resumed_as_completed(self):
        for mode in ['exit-failed', 'budget']:
            self.mode.write_text(mode)
            with self.assertRaises(D['Stop']):self.generated_for_test_reset()
            result = self.check(); self.assertNotEqual(result['status'], 'READY')
            self.assertNotEqual(D['load'](self.run / 'state.json')['status'], 'AUTHOR_COMPLETED')
            with self.assertRaises(D['Stop']):D['promote'](self.run)
            before = len(D['load'](self.run / 'state.json')['attempts'])
            with self.assertRaises(D['Stop']):D['execute'](self.run)
            self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']), before)

    def test_interruption_preserves_previous_ready_and_is_not_complete(self):
        job = D['load'](self.run / 'inputs/job.json'); job['budget']['wallSeconds'] = 1
        D['save'](self.run / 'inputs/job.json', job)
        pins = D['load'](self.run / 'pins.json'); pins['jobDigest'] = D['digest'](job); pins['inputFiles'] = D['tree'](self.run / 'inputs'); D['save'](self.run / 'pins.json', pins)
        self.mode.write_text('interrupt')
        with self.assertRaises(D['Stop']):self.generated()
        self.assertEqual(D['load'](self.run / 'state.json')['status'], 'INTERRUPTED')
        self.assertNotEqual(self.check()['status'], 'READY')
        self.assertFalse((self.run / 'ready').exists())

    def test_bare_starter_cannot_pass_on_formal_self_review(self):
        original = self.run / 'inputs/starter/original.json'
        shutil.copyfile(original, self.seed / 'original.json')
        self.review['claims'] = []
        D['save'](self.seed / 'self-review.json', self.review)
        self.generated()
        result = self.check()
        detail = result['checks']['output']
        self.assertEqual(detail['code'], 'INVALID_PAGE_CLAIMS')
        self.assertIn('cited reader explanation', detail['details']['nextAction'])

    def test_literal_guard_with_the_same_claim_and_cite_rejects_a_changed_inline_result(self):
        previous = self.source
        text = 'fun change(size: Int): Int {\n    if (size < -3) { return -12 }\n    return size\n}\n'
        raw = text.encode()
        blob = hashlib.sha1(('blob ' + str(len(raw)) + '\0').encode() + raw).hexdigest()
        self.source = {**previous, 'file': 'src/Change.kt', 'text': text, 'textDigest': D['sha'](raw), 'endLine': 4,
                       'occurrence': {**previous['occurrence'], 'blob': blob, 'endByte': len(raw)}}
        self.clew.write_text(self.clew.read_text().replace(repr(previous), repr(self.source)))
        shutil.rmtree(self.manifest.parent)
        self.manifest, view, _ = F['fixture'](self.seed / 'reads', service=self.service, work='owned-work', record=self.source)
        old_handle, self.handle = self.handle, view['records'][0]['cite']

        scope = self.inventory['services'][self.service]
        scope['sourceInventory'][0]['record']['path'] = self.source['file']
        scope['sourceInventory'].append({'kind': 'THREAD_SCOPE'})
        discovery = next(row for row in self.inventory['requirements'] if row['kind'] == 'DISCOVERY')
        discovery['file'] = self.source['file']

        def source_command(argv, directory, label, **kwargs):
            out = Path(argv[argv.index('--output-dir') + 1])
            if label == 'discover':
                out.mkdir()
                D['save'](out / 'packet.json', {'queryComplete': True, 'choices': [{'file': self.source['file']}],
                                              'scope': {'snapshot': F['SNAPSHOT']}})
            else:
                shutil.copytree(self.manifest.parent, out)
        prepared = self.run / 'inputs/author-source'
        prepared.rmdir()
        with patch.dict(D['_prepare_sources'].__globals__, {'command': source_command}):
            D['prepare_sources'](self.job, self.inventory, prepared)
        scope['sourceInventory'].pop()  # The verify fixture supplies the compiler membership transport.
        shutil.rmtree(self.manifest.parent)
        pins = D['load'](self.run / 'pins.json')
        for path in ('inputs/inventory.json', 'workspace/inventory.json'):
            D['save'](self.run / path, self.inventory)
        pins.update(inputFiles=D['tree'](self.run / 'inputs'), inventoryDigest=D['digest'](self.inventory))
        D['save'](self.run / 'pins.json', pins)

        operation = self.original['pages'][0]
        claim = {'id': 'guard', 'text': '\r\nWhen `size < -3`, the result is `-12`.\r\n', 'evidence': [old_handle]}
        operation['fragments'].extend([{'text': '\n## Examples π\n' + claim['text']},
                                       {'text': '\n## Checked return\n'}, claim])
        self.review['claims'].append({'page': operation['page'], 'claim': claim['id'],
                                      'evidence': [old_handle], 'disposition': 'SUPPORTED'})
        for row in self.review['requirements'] + self.review['claims'] + [
                item for page in self.original['pages'] for item in page['fragments'] if 'evidence' in item]:
            row['evidence'] = [self.handle if handle == old_handle else handle for handle in row['evidence']]
        D['save'](self.run / 'workspace/draft/original.json', self.original)
        template = D['review_for_run'](self.run)
        self.assertEqual(list(template['returnClaims'].values()), [None])
        check = next(iter(template['returnClaims']))
        self.assertIn(check, (prepared / self.service / 'index.md').read_text())
        self.review['returnClaims'] = {check: 'guard'}
        self.seed_files(); self.generated()
        result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertFalse(result['semanticTruthAssessed'])
        check_result = result['checks']['entrySequences']['result']['literalReturns']
        self.assertEqual(check_result[check], {'claim': 'guard', 'page': operation['page']})
        html = (self.run / result['output'] / 'html' / operation['page'].replace('.md', '.html')).read_text()
        self.assertIn('When <code>size &lt; -3</code>, the result is <code>-12</code>.', html)

        claim['text'] = claim['text'].replace('`-12`', '`6`')
        self.seed_files(); self.generated_for_test_reset()
        result = self.check()
        self.assertEqual(result['status'], 'FAILED')
        detail = result['checks']['entrySequences']
        self.assertEqual(detail['code'], 'LITERAL_RETURN_VALUE_MISMATCH')
        self.assertEqual(detail['details']['response']['details']['documented'], '6')
        self.assertEqual(detail['details']['response']['details']['expected'], '-12')

    def test_literal_guard_scan_skips_unsupported_ambiguous_and_commented_source(self):
        def candidates(body, extension='.kt'):
            source = {'readText': body, 'source': {'file': 'Example' + extension, 'id': 'exact', 'startLine': 1}}
            return D['literal_return_candidates'](self.entry, [self.handle], {self.handle: source})
        statement = '  if (amount != 18) return -2'
        function = lambda body: 'fun adjust(amount: Int): Int {\n' + body + '\n  return amount\n}'
        self.assertEqual(candidates(function(statement))[0]['literal'], '-2')
        self.assertEqual(candidates(function(statement), '.java'), [])
        self.assertEqual(candidates(function('/*\n' + statement + '\n*/')), [])
        self.assertEqual(candidates(function('val text = """\n' + statement + '\n"""')), [])
        self.assertEqual(candidates(function('// ' + statement)), [])
        self.assertEqual(candidates(function(statement + '\n' + statement.replace('-2', '5'))), [])
        self.assertEqual(candidates(function('if (amount != 18) return run { -2 }')), [])
        self.assertEqual(candidates(function(statement + '\n  + adjustment')), [])
        nested = function('  fun unused(other: Int): Int {\n    if (other < -2) return 17\n    return other\n  }\n' + statement)
        self.assertEqual(candidates(nested), [])
        self.assertEqual(candidates(function(statement) + '\n\n' + function(statement.replace('-2', '5'))), [])
        self.assertEqual(candidates('class Example {\n' + function(statement) + '\n}'), [])

    def test_overlapping_exact_function_reads_produce_one_literal_obligation(self):
        function = 'fun route(value: Int): Int {\n    if (value > 42) return 9\n    return value\n}'
        fragment = {'readText': function, 'source': {'file': 'Example.kt', 'id': 'method', 'startLine': 11}}
        annotated = {'readText': '@GetMapping("/somewhere")\n' + function,
                     'source': {'file': 'Example.kt', 'id': 'mapped-method', 'startLine': 10}}
        entries = {'first': fragment, 'second': annotated}
        checks = D['literal_return_candidates'](self.entry, list(entries), entries)
        self.assertEqual(len(checks), 1)
        self.assertEqual(checks[0]['sourceIds'], ['method', 'mapped-method'])
        self.assertEqual(checks[0]['line'], 12)

    def test_source_backed_false_assertion_is_not_a_semantic_certificate(self):
        self.original['pages'][0]['fragments'][1]['text'] += '\nDelivery is guaranteed.\n'
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertFalse(result['semanticTruthAssessed'])

    def test_verification_resume_does_not_launch_another_author(self):
        self.generated()
        result = D['execute'](self.run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']), 1)

    def test_genuine_foreign_source_proof_is_rejected(self):
        shutil.rmtree(self.seed / 'reads'); (self.seed / 'reads').mkdir()
        manifest, view, _ = F['fixture'](self.seed / 'reads', service='unselected-service', record=self.source)
        foreign = view['records'][0]['cite']
        for page in self.original['pages']:
            for fragment in page['fragments']:
                if 'evidence' in fragment:fragment['evidence'] = [foreign]
        for account in self.review['requirements'] + self.review['claims']:account['evidence'] = [foreign]
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['checks']['provenance']['code'], 'PROVENANCE_MISMATCH')

    def test_locally_rebound_occurrence_cannot_replace_native_occurrence(self):
        shutil.rmtree(self.seed / 'reads'); (self.seed / 'reads').mkdir()
        forged = {**self.source, 'occurrence': {**self.source['occurrence'], 'snapshot': 'forged-source-snapshot'}}
        _, view, _ = F['fixture'](self.seed / 'reads', service=self.service, record=forged)
        handle = view['records'][0]['cite']
        for page in self.original['pages']:
            for fragment in page['fragments']:
                if 'evidence' in fragment:fragment['evidence'] = [handle]
        for account in self.review['requirements'] + self.review['claims']:account['evidence'] = [handle]
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['checks']['provenance']['code'], 'PROVENANCE_MISMATCH')

    def test_parseable_other_entry_diagram_is_rejected(self):
        fragment = self.original['pages'][0]['fragments'][1]
        fragment['text'] = fragment['text'].replace(self.entry['handlerId'], 'different-handler')
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['checks']['entrySequences']['code'], 'SEQUENCE_ENTRY_MISMATCH')

    def test_complete_agreeing_byte_union_and_digest_domains(self):
        raw = self.source['text'].encode(); split = len(raw) // 2
        req = next(r for r in self.inventory['requirements'] if r['kind'] == 'DISCOVERY')
        entries = {}
        for name, start, end in [('first', 0, split), ('second', split, len(raw))]:
            part = raw[start:end].decode()
            source = {**self.source, 'textDigest': D['sha'](raw[start:end]),
                      'occurrence': {**self.source['occurrence'], 'startByte': start, 'endByte': end}}
            entries[name] = {'source': source, 'readText': part}
        proof = D['full_file_proof'](req, list(entries), entries, F['SNAPSHOT'])
        self.assertEqual(proof['gitBlob'], self.source['occurrence']['blob'])
        self.assertNotEqual(proof['textDigest'], req['fileFingerprint'])
        with self.assertRaises(D['Stop']):D['full_file_proof'](req, ['first'], entries, F['SNAPSHOT'])
        entries['second']['source']['occurrence']['startByte'] += 1
        entries['second']['source']['occurrence']['endByte'] += 1
        with self.assertRaises(D['Stop']):D['full_file_proof'](req, list(entries), entries, F['SNAPSHOT'])

    def test_unknown_job_gate_override_is_rejected(self):
        job = dict(self.job); job['skipChecks'] = ['entrySequences']
        with self.assertRaises(D['Stop']):D['validate_job'](job)


    def test_verify_and_promotion_still_refuse_a_runtime_version_change(self):
        self.generated();accepted = self.check()
        self.assertEqual(accepted['status'], 'READY')
        original_command = D['inputs_unchanged'].__globals__['command']
        def changed(argv, *args, **kwargs):
            if argv == [str(self.clew), '--version']:return 'clew changed-runtime'
            return original_command(argv, *args, **kwargs)
        with patch.dict(D['inputs_unchanged'].__globals__, {'command': changed}):
            failed = self.check()
            self.assertEqual(failed['status'], 'BLOCKED')
            self.assertEqual(failed['checks']['inputs']['code'], 'ADMITTED_RUNTIME_CHANGED')
            D['save'](self.run / 'report.json', accepted)
            with self.assertRaises(D['Stop']) as error:D['promote'](self.run)
            self.assertEqual(error.exception.code, 'PROMOTION_RECHECK_FAILED')
            self.assertFalse((self.run / 'ready').exists())

    def test_cumulative_same_thread_resume_uses_deltas_and_preserves_raw_reports(self):
        # Public counters observed across real CLI 0.159.2 resume boundaries.
        first = {'input_tokens':1771674, 'cached_input_tokens':1688320,
                 'output_tokens':36956, 'reasoning_output_tokens':24378}
        second = {'input_tokens':2793295, 'cached_input_tokens':2670976,
                  'output_tokens':62544, 'reasoning_output_tokens':43098}
        attempts = [{'threadId':'same', 'usage': first}, {'threadId':'same', 'usage': second}]
        before = json.dumps(attempts)
        result = D['token_accounting'](attempts)
        self.assertEqual(result['status'], 'MEASURED')
        self.assertEqual(result['budgetTokens'], 184863)
        self.assertEqual(result['rawTotals'], second)
        self.assertEqual([a['budgetTokens'] for a in result['attemptUsage']], [120310, 64553])
        self.assertEqual(result['reportedUsage'][1]['usage'], second)
        self.assertEqual(json.dumps(attempts), before)
        self.assertNotEqual(result['budgetTokens'], 305173)

    def test_cumulative_counter_reset_and_inconsistent_delta_refuse_exact_usage(self):
        first = {'input_tokens':100, 'cached_input_tokens':80, 'output_tokens':20, 'reasoning_output_tokens':10}
        for second in [dict(first, input_tokens=99), dict(first, output_tokens=19),
                       dict(first, reasoning_output_tokens=9),
                       dict(first, input_tokens=110, cached_input_tokens=95)]:
            result = D['token_accounting']([{'threadId':'same','usage':first}, {'threadId':'same','usage':second}])
            self.assertEqual(result['status'], 'INVALID', second)
            self.assertIsNone(result['budgetTokens'])
            self.assertGreaterEqual(result['knownBudgetTokensLowerBound'],40)

    def test_missing_thread_or_counters_never_invent_an_exact_resume_cost(self):
        raw = {'input_tokens':100, 'cached_input_tokens':80, 'output_tokens':20}
        for attempts in [[{'usage':raw}, {'usage':raw}],
                         [{'threadId':'same','usage':raw}, {'usage':raw}],
                         [{'threadId':'same','usage':raw}, {'threadId':'same','usage':None}],
                         [{'threadId':'same','usage':{'input_tokens':100,'output_tokens':20}},
                          {'threadId':'same','usage':dict(raw,input_tokens=150,cached_input_tokens=100)}]]:
            result = D['token_accounting'](attempts)
            self.assertEqual(result['status'], 'UNAVAILABLE')
            self.assertIsNone(result['budgetTokens'])
            self.assertEqual(result['knownBudgetTokensLowerBound'],40 if len(attempts[-1].get('usage') or {})<3 or 'threadId' not in attempts[-1] else 70)


class SourceMembershipTests(unittest.TestCase):
    """Identity metadata supplements complete body proof; never replaces it."""
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.calls=0
        self.job={'clew':'trusted-test-engine','root':'retained','snapshot':'frozen'}
        self.inventory={'services':{'owned':{'sourceInventory':[{'kind':'THREAD_SCOPE'}]}}}
        self.source={'id':'owned-large','service':'owned','file':'large.json','revision':'same',
                     'startLine':1,'endLine':5000,'textDigest':'sha256:complete-body',
                     'authority':'EXACT_SNAPSHOT_TEXT','occurrence':{'startByte':0,'endByte':72092}}
        self.entry={'work':'immutable-work','source':self.source,'sourceRecordDigest':'sha256:full-record'}
        self.rows=[{'id':self.source['id'],'kind':'SOURCE_NAVIGATION','record':{
            'authority':'NAVIGATION_ONLY','textDelivered':False,'recordDigest':self.entry['sourceRecordDigest'],
            'source':dict(self.source),'fullRecordReference':'s7'}}]
        self.response={'status':'COMPLETE','omittedRecords':0,'identity':{'snapshot':'frozen'}}

    def collect(self,clew,root,work,output,**kwargs):
        self.calls+=1;self.assertEqual(work,'immutable-work')
        output.mkdir();D['save'](output/'items.json',self.rows)
        return self.response

    def native_page(self, **fields):
        return {'schema':'codeclew-documentation-work-page/1.0','work':self.entry['work'],
                'snapshot':self.job['snapshot'],'subject':'service:owned','contextDigest':'sha256:context',
                'inputDigest':'sha256:input','membershipDigest':'sha256:members','receiptDigest':'sha256:receipt',
                'influenceCoverage':'RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED',
                'status':'READY_WITH_LIMITATIONS','items':self.rows,'omitted':[],'total':1,'nextCursor':None,**fields}

    def native_driver(self, payload, exit_code=0, raw=None):
        config=self.root/'native-response.json';D['save'](config,{'payload':payload,'exitCode':exit_code,'raw':raw})
        engine=self.root/'native-clew';counter=self.root/'native-count'
        engine.write_text('#!'+sys.executable+'\n'+
            'import json,sys\nfrom pathlib import Path\n'+
            f'config=Path({str(config)!r});counter=Path({str(counter)!r})\n'+
            'counter.write_text(str(int(counter.read_text())+1 if counter.exists() else 1))\n'+
            "args=sys.argv[1:];assert args[:3]==['docs','work','read']\n"+
            "selection=json.loads(Path(args[args.index('--input')+1]).read_text())\n"+
            "assert selection=={'query':{'kind':'SOURCE','projection':'NAVIGATION','file':'large.json'}}\n"+
            "value=json.loads(config.read_text());print(value['raw'] if value['raw'] is not None else json.dumps(value['payload']))\n"+
            "sys.stderr.write('DO_NOT_EMIT_PRIVATE_LOG_OR_SECRET')\n"+
            "sys.exit(value['exitCode'])\n")
        engine.chmod(0o700);self.job['clew']=str(engine)
        return counter

    def failure_after_cleanup(self):
        cache={}
        with tempfile.TemporaryDirectory(dir=self.root) as temporary:
            with self.assertRaises(D['Stop']) as error:
                D['check_source_membership'](self.job,self.inventory,self.entry,Path(temporary),cache)
            self.assertEqual(error.exception.code,'SOURCE_MEMBERSHIP_NAVIGATION_INCOMPLETE')
            details=error.exception.details
        self.assertFalse(Path(temporary).exists());self.assertEqual(cache,{})
        self.assertEqual(details['service'],'owned');self.assertEqual(details['file'],'large.json')
        self.assertEqual(details['work'],'immutable-work');self.assertEqual(details['expectedSnapshot'],'frozen')
        self.assertLess(len(json.dumps(details)),1800)
        self.assertNotIn('DO_NOT_EMIT',json.dumps(details))
        self.assertNotIn('sourceRecordDigest',json.dumps(details))
        return details

    def test_native_process_failure_keeps_structured_cause_after_cleanup_without_payload_or_logs(self):
        count=self.native_driver({'error':{'code':'CACHE_BUSY','status':'FAILED','message':'DO_NOT_EMIT_MESSAGE',
                                          'source':'DO_NOT_EMIT_SOURCE','auth':'DO_NOT_EMIT_AUTH'}},23)
        details=self.failure_after_cleanup();transport=details['transport']
        self.assertEqual(transport['reason'],'NATIVE_PROCESS_FAILED');self.assertEqual(transport['status'],'INCOMPLETE')
        self.assertEqual(transport['nativeProcess'],{'exitCode':23,'failure':None,'code':'CACHE_BUSY','status':'FAILED'})
        self.assertIsNone(details['actualSnapshot']);self.assertEqual(count.read_text(),'1')

    def test_native_start_failure_and_oversize_error_remain_bounded_without_retry(self):
        self.job['clew']=str(self.root/'missing-engine')
        details=self.failure_after_cleanup()
        self.assertEqual(details['transport']['reason'],'PROCESS_START_FileNotFoundError')
        self.assertEqual(details['transport']['nativeProcess']['failure'],'PROCESS_START_FileNotFoundError')
        self.assertIsNone(details['transport']['nativeProcess']['exitCode'])
        count=self.native_driver({'code':'CACHE_BUSY','message':'DO_NOT_EMIT_'*4000},23)
        details=self.failure_after_cleanup();self.assertIsNone(details['transport']['nativeProcess']['code'])
        self.assertEqual(details['transport']['nativeProcess']['exitCode'],23);self.assertEqual(count.read_text(),'1')
        for raw in ('{invalid native response', '['*2000+'0'+']'*2000):
            count=self.native_driver({},23,raw);before=int(count.read_text())
            details=self.failure_after_cleanup()
            self.assertEqual(details['transport']['reason'],'NATIVE_PROCESS_FAILED')
            self.assertEqual(details['transport']['nativeProcess']['exitCode'],23)
            self.assertIsNone(details['transport']['nativeProcess']['code'])
            self.assertEqual(int(count.read_text())-before,1)

    def test_native_incomplete_omissions_and_snapshot_mismatch_keep_exact_refusal_causes(self):
        cases=[({'total':2},'INCOMPLETE','TOTAL_COUNT_MISMATCH',0,'frozen'),
               ({'omitted':[{'reason':'DO_NOT_EMIT_OMITTED_BODY'}]},'INCOMPLETE','OMITTED_RECORDS_REQUIRE_NATIVE_INSPECTION',1,'frozen'),
               ({'snapshot':'foreign'},'COMPLETE',None,0,'foreign')]
        for fields,status,reason,omissions,snapshot in cases:
            with self.subTest(fields=fields):
                count=self.native_driver(self.native_page(**fields))
                before=int(count.read_text()) if count.exists() else 0
                details=self.failure_after_cleanup();transport=details['transport']
                self.assertEqual((transport['status'],transport['reason'],transport['omittedRecords']),
                                 (status,reason,omissions))
                self.assertEqual(transport['nativeStatus'],'READY_WITH_LIMITATIONS')
                self.assertEqual(details['actualSnapshot'],snapshot)
                self.assertEqual(transport['nativeProcess']['exitCode'],0)
                self.assertEqual(int(count.read_text())-before,1)

    def test_native_success_keeps_membership_check_and_single_read_cache(self):
        count=self.native_driver(self.native_page());cache={}
        with patch.dict(D['check_source_membership'].__globals__,{
                'source_membership_failure':lambda *a,**k:self.fail('Successful reads need no failure payload')}):
            with tempfile.TemporaryDirectory(dir=self.root) as temporary:
                for _ in range(2):D['check_source_membership'](self.job,self.inventory,self.entry,Path(temporary),cache)
        self.assertEqual(count.read_text(),'1');self.assertEqual(cache[(self.entry['work'],self.source['file'])][self.source['id']],self.rows[0]['record'])

    def test_large_saved_body_validates_by_native_record_identity_and_reuses_file_metadata(self):
        with patch.dict(D['T'],{'collect':self.collect}),patch.dict(D['check_source_membership'].__globals__,{
            'command':lambda *a,**k:self.fail('Source file scope must not request a stdout-sized body')}):
            cache={};D['check_source_membership'](self.job,self.inventory,self.entry,self.root,cache)
            D['check_source_membership'](self.job,self.inventory,self.entry,self.root,cache)
        self.assertEqual(self.calls,1)

    def test_foreign_digest_metadata_missing_member_and_incomplete_inventory_refuse(self):
        changes=[lambda:setattr(self,'rows',[]),
                 lambda:self.rows[0]['record'].update(recordDigest='sha256:foreign'),
                 lambda:self.rows[0]['record']['source'].update(revision='other'),
                 lambda:self.rows[0]['record'].update(textDelivered=True),
                 lambda:self.response.update(omittedRecords=1),
                 lambda:self.response['identity'].update(snapshot='other')]
        for i,change in enumerate(changes):
            with self.subTest(case=i):
                original_rows=json.loads(json.dumps(self.rows));original_response=json.loads(json.dumps(self.response))
                change();path=self.root/str(i);path.mkdir()
                with patch.dict(D['T'],{'collect':self.collect}):
                    with self.assertRaises(D['Stop']):D['check_source_membership'](self.job,self.inventory,self.entry,path,{})
                self.rows=original_rows;self.response=original_response

    def test_compiler_scope_keeps_supported_source_context_route(self):
        self.inventory['services']['owned']['sourceInventory']=[]
        response={'snapshot':'frozen','items':[{'kind':'SOURCE','id':self.source['id'],'record':self.source}]}
        with patch.dict(D['T'],{'collect':lambda *a,**k:self.fail('No SOURCE inventory for compiler Work')}),patch.dict(
            D['check_source_membership'].__globals__,{'command':lambda *a,**k:response}):
            D['check_source_membership'](self.job,self.inventory,self.entry,self.root,{})


if __name__ == '__main__':
    unittest.main()
