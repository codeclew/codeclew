#!/usr/bin/env python3
"""Owned source/author protocol fixtures; no models, capture or public writes."""
import json
import hashlib
import os
from pathlib import Path
import runpy
import shutil
import signal
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
REQUEST_MARKER = 'AUTHOR-INSTRUCTIONS-7C5B'
LARGE_USAGE = {'input_tokens': 600000, 'cached_input_tokens': 200000, 'output_tokens': 33917, 'reasoning_output_tokens': 12000}


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
prompt=sys.stdin.read()
page='docs-run-page-author.schema.json' in ' '.join(sys.argv)
slot=json.loads(prompt.split('\\n\\n')[-1])['slot'] if page else None
thread=('owned-page-'+slot) if page else 'owned-test-session'
if mode=='duplicate-page-thread':thread='owned-shared-page'
if mode!='missing-thread':
 print(json.dumps({{'type':'thread.started','thread_id':'other-test-session' if mode=='thread-switch' and 'resume' in sys.argv else thread}}),flush=True)
if mode=='ambiguous-thread':print(json.dumps({{'type':'thread.started','thread_id':'second-test-session'}}),flush=True)
if mode=='interrupt':time.sleep(10)
if mode=='controlled-page':
 control=json.loads((seed/'queue-control.json').read_text()).get(slot,{{}})
 (seed/(slot+'.started')).write_text(str(time.monotonic()))
draft=Path.cwd()/'draft'
if '--output-schema' in sys.argv:
 name=slot if page else 'inline'
 source=seed/(name+'-repair.json' if 'resume' in sys.argv and (seed/(name+'-repair.json')).exists() else name+'-response.json')
 Path(sys.argv[sys.argv.index('-o')+1]).write_bytes(source.read_bytes())
else:
 if mode!='empty':
  for source in seed.iterdir():
   if source.name.startswith('inline-'):continue
   target=draft/source.name
   if target.is_dir():shutil.rmtree(target)
   elif target.exists():target.unlink()
   if source.is_dir():shutil.copytree(source,target)
   else:shutil.copyfile(source,target)
 (draft/'report.json').write_text(json.dumps({{'status':'PASS','semanticTruthAssessed':True}}))
 if mode=='repair-on-resume' and 'resume' in sys.argv:
  path=draft/'original.json';path.write_text(path.read_text().replace('[Missing](absent.html)','Repaired local link.'))
usage={{'input_tokens':5000 if mode=='high-cache' else 20,'cached_input_tokens':4990 if mode=='high-cache' else 10,'output_tokens':10}}
if mode in ('high-usage','high-usage-missing-cache','high-usage-invalid-cache','repair-on-resume'):
 usage={LARGE_USAGE!r}
if mode=='high-usage-missing-cache':
 usage.pop('cached_input_tokens')
 usage['output_tokens']=433917
if mode=='high-usage-invalid-cache': usage['cached_input_tokens']=usage['input_tokens']+1
counter=(seed/('usage-'+slot+'.json')) if page else Path.cwd()/'owned-cumulative-usage.json'
if 'resume' in sys.argv and counter.exists():
 previous=json.loads(counter.read_text());usage={{key:value+previous.get(key,0) for key,value in usage.items()}}
counter.write_text(json.dumps(usage))
if mode=='controlled-page':
 if control.get('report', True):
  print(json.dumps({{'type':'turn.completed','usage':usage}}),flush=True)
 (seed/(slot+'.waiting')).touch()
 expires=time.monotonic()+6
 for target in control.get('until', []):
  while not (seed/target).exists():
   if time.monotonic()>=expires:sys.exit(30)
   time.sleep(0.01)
 (seed/(slot+'.finished')).write_text(str(time.monotonic()))
else:
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
        self.job = {'schema': D['SCHEMA'], 'request': 'Document the reservation service. ' + REQUEST_MARKER, 'clew': str(self.clew),
                    'root': str(self.root / 'retained'), 'snapshot': F['SNAPSHOT'], 'services': [self.service], 'language': 'en',
                    'renderer': str(RENDERER), 'node': str(Path(shutil.which('node')).absolute()),
                    'author': {'executable': str(self.backend), 'model': 'owned-test-backend', 'reasoningEffort': 'max'},
                    'budget': {'wallSeconds': 30, 'maxRepairs': 0}}
        (self.root / 'retained').mkdir()
        self.job_path = self.root / 'job.json'; D['save'](self.job_path, self.job)
        coverage = {'kind': 'COVERAGE', 'revision': self.source['revision'], 'coverage': 'SYNTAX', 'boundaries': []}
        entry = {'kind': 'THREAD_ENTRY', 'record': {'classification': 'DISCOVERED_TRIGGER',
                 'sourceIds': [self.source['id']], 'rootDeclarations': ['owned-symbol'],
                 'entrypoint': {'id': 'reserve', 'kind': 'HTTP_ENDPOINT', 'sourceIds': [self.source['id']],
                 'trigger': {'authority': 'OWNED_TEST_DECLARATION', 'sourceRoutes': [{'method': 'POST', 'path': '/reservations'}]}}}}
        file = {'kind': 'THREAD_FILE', 'record': {'path': self.source['file'], 'input': {'coverage': 'SYNTAX', 'digest': 'sha256:owned-syntax-fingerprint-not-a-text-hash'}}}
        self.native_entry = entry
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

    def test_reader_chrome_uses_navigation_home_title(self):
        self.navigation['home']['title'] = '--reserve & fulfil'
        self.seed_files()
        self.generated()
        result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertIn(REQUEST_MARKER, D['load'](self.run / 'inputs/job.json')['request'])
        html_root = self.run / result['output'] / 'html'
        pages = sorted(html_root.glob('*.html'))
        self.assertEqual(len(pages), 2)
        for page in pages:
            with self.subTest(page=page.name):
                html = page.read_text()
                self.assertRegex(html, r'<span class="reader-project">\s*--reserve &amp; fulfil</span>')
                self.assertRegex(html, r' —\s+--reserve &amp; fulfil</title>')
                self.assertNotIn(REQUEST_MARKER, html)

    def prepare_inline_fixture(self, body=None, *, repairs=0, extra_resource=False, project=None):
        text = body or ('fun reserve(count: Int): Int {\n'
                        '    if (count < 0) return 0\n'
                        '    accepted.add(count)\n'
                        '    events.on("note.ready") { accept(it) }\n'
                        '    return accepted.size\n}\n')
        raw = text.encode()
        blob = hashlib.sha1(('blob ' + str(len(raw)) + '\0').encode() + raw).hexdigest()
        source = {**self.source, 'file': 'src/Reservations.kt', 'text': text,
                  'textDigest': D['sha'](raw), 'endLine': len(text.splitlines()),
                  'occurrence': {**self.source['occurrence'], 'blob': blob, 'endByte': len(raw)}}
        if project is not None:
            self.job['evidenceMode'] = 'PREPARED_FACTS'
            source['authority'] = 'EXACT_SNAPSHOT_TEXT'
            self.native_entry['sourceReferences'] = ['s9']
        shutil.rmtree(self.manifest.parent)
        self.manifest, _, _ = F['fixture'](self.seed / 'reads', service=self.service, work='owned-work',
                                           record=source, authority=source['authority'])
        scope = self.inventory['services'][self.service]
        files = [source['file']]
        inventory_rows = [self.native_entry, {'kind': 'THREAD_SCOPE'},
            {'kind': 'THREAD_FILE', 'record': {'path': source['file'], 'input': {'coverage': 'SYNTAX',
              'digest': 'sha256:owned-syntax-fingerprint'}}}]
        if extra_resource:
            files.append('settings.properties')
            inventory_rows.append({'kind': 'THREAD_FILE', 'record': {'path': files[-1], 'input': {'coverage': 'FILE_ONLY'}}})
        member_checks = []

        def collect(launcher, root, work, output, **kwargs):
            selected = (kwargs.get('selection') or {}).get('query', {}).get('kind')
            output.mkdir(parents=True)
            if selected == 'SOURCE':
                assert kwargs['selection']['query']['file'] == source['file']
                member_checks.append(source['id'])
                rows = [{'id': source['id'], 'kind': 'SOURCE_NAVIGATION', 'record': {
                    'source': {key: value for key, value in source.items() if key != 'text'},
                    'recordDigest': F['digest'](source), 'textDelivered': False, 'authority': 'NAVIGATION_ONLY'}}]
            elif selected == 'DECLARED_INTERACTION':
                rows = []
            elif selected == 'THREAD_CONTEXT':
                rows = [{'kind': 'THREAD_OBLIGATION', 'id': 'BODY:owned-symbol', 'obligationKind': 'BODY',
                         'record': {'declaration': 'owned-symbol', 'sourceIds': [source['id']]},
                         'sourceReferences': ['s9']}]
            else:
                rows = inventory_rows if selected == 'THREAD_INVENTORY' else [scope['coverage']]
            D['save'](output / 'items.json', rows)
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}

        native = D['command']
        def command(argv, directory, label, **kwargs):
            if label == 'local-binding':
                raise D['Stop']('OWNED_MISSING_LOCAL_BINDING')
            if label not in ('discover', 'read'):
                return native(argv, directory, label, **kwargs)
            output = Path(argv[argv.index('--output-dir') + 1])
            if label == 'discover':
                output.mkdir()
                D['save'](output / 'packet.json', {'queryComplete': True, 'choices': [{'file': name} for name in files],
                           'scope': {'snapshot': F['SNAPSHOT']}})
            else:
                shutil.copytree(self.manifest.parent, output)

        self.job['budget']['maxRepairs'] = repairs
        D['save'](self.job_path, self.job)
        run = self.root / 'inline-run'
        self.enterContext(patch.dict(D['T'], {'collect': collect}))
        actual_run_path = runpy.run_path

        def run_path(filename, **kwargs):
            if project is not None and filename.endswith('/managed_languages.py'):
                return {'project': project}
            return actual_run_path(filename, **kwargs)

        with patch.dict(D['prepare'].__globals__, {'command': command}), patch.object(runpy, 'run_path', run_path):
            receipt = D['prepare'](self.job_path, run)
        shutil.rmtree(self.manifest.parent)
        return run, receipt, member_checks, source

    def inline_response(self, *, added=True, literal='0'):
        def claim(text, *, returns=(), assessment='SUPPORTED'):
            return {'heading': '', 'text': text, 'cites': ['S1'], 'assessment': assessment, 'returns': list(returns)}
        def step(kind, label, condition=None, to='caller'):
            return {'from': 'handler', 'to': to, 'kind': kind, 'label': label, 'condition': condition}
        entry = {'ref': 'E1', 'disposition': 'EXPLAINED', 'participants': [],
                 'claims': [claim('When `count < 0`, the method returns `' + literal + '` without adding to the list.', returns=['R1']),
                            claim('Otherwise it appends the count to `accepted` and returns the list size.'),
                            claim('The captured source gives no HTTP binding or serialized response.', assessment='EXPLICIT_UNKNOWN')],
                 'steps': [step('return', 'Return ' + literal + ' without changing the list', 'count < 0'),
                           step('note', 'Append the input', 'count >= 0', to='handler'),
                           step('return', 'Return the updated list size', 'count >= 0')]}
        start = {'kind': 'OTHER', 'title': 'Note arrival', 'initiator': 'Event hub',
                 'method': None, 'path': None, 'startLabel': 'Receive note.ready', 'startKind': 'async',
                 'disposition': 'EXPLAINED', 'claims': [claim('The callback passes the payload to `accept`.')],
                 'participants': [], 'steps': [step('note', 'Pass the payload to accept', to='handler')]}
        return {'status': 'DOCUMENTED', 'reason': '', 'reviewed': True,
                'overview': {'title': 'Reservation behavior', 'disposition': 'EXPLAINED',
                             'claims': [claim('The handler appends accepted reservations to `accepted`.')]},
                'entries': [entry], 'discovery': [{'ref': 'F1', 'disposition': 'EXPLAINED',
                    'discovery': 'ADDITIONAL_STARTS' if added else 'NO_ADDITIONAL_STARTS', 'starts': [start] if added else []}],
                'unresolved': ['Wire response and registration remain unknown.'],
                'visual': {'status': 'UNAVAILABLE', 'reason': 'This author turn has no browser.'}}

    def isolated_fixture(self, *, repairs=0, wrong_literal=False, language='en', project=None, wall_seconds=30):
        self.job.update(authorMode='ISOLATED_PAGES', language=language)
        self.job['budget']['wallSeconds'] = wall_seconds
        run, receipt, _, source = self.prepare_inline_fixture(repairs=repairs, project=project)
        self.assertEqual(receipt['authorMode'], 'ISOLATED_PAGES')
        plan = D['load'](run / 'inputs/author-plan.json')
        packets = {row['kind']: json.loads((run / 'inputs' / row['prompt']).read_text().split('\n\n')[-1])
                   for row in plan['slots']}
        self.assertEqual(set(packets), {'GUIDE', 'CONTRACTS', 'OPERATION', 'DISCOVERY', 'PROCESS_MAP'})
        localized = language == 'ru'
        for slot in plan['slots']:
            kind = slot['kind']
            packet = packets[kind]
            alias = next(iter(packet['citations']))
            text = '\u0411\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u0435 \u0434\u043e\u0431\u0430\u0432\u043b\u044f\u0435\u0442 \u044d\u043b\u0435\u043c\u0435\u043d\u0442.' if localized else 'The handler adds the accepted reservation.'
            claim = {'heading': '', 'text': text, 'cites': [alias], 'assessment': 'SUPPORTED', 'returns': []}
            answer = {'slot': slot['key'], 'reviewed': True, 'disposition': 'EXPLAINED', 'title': None,
                      'claims': [claim], 'participants': [], 'steps': [], 'discovery': None, 'starts': [],
                      'contracts': [], 'routes': [], 'limitations': []}
            if kind == 'GUIDE':
                answer['title'] = '\u0411\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u0435' if localized else 'Reservations'
            if kind == 'CONTRACTS':
                answer['contracts'] = [{'scope': 'LOCAL', 'name': 'Reservation', 'cites': [alias],
                    'fields': [{'name': 'count', 'type': 'Int', 'meaning': '\u041a\u043e\u043b\u0438\u0447\u0435\u0441\u0442\u0432\u043e' if localized else 'Item count'}]}]
            if kind == 'DISCOVERY':
                answer.update(claims=[], discovery='NO_ADDITIONAL_STARTS')
            if kind == 'OPERATION':
                ref, = packet['returns']
                cited = packet['returns'][ref]['cite']
                literal_text = '\u0415\u0441\u043b\u0438 `count < 0`, \u043c\u0435\u0442\u043e\u0434 \u0432\u043e\u0437\u0432\u0440\u0430\u0449\u0430\u0435\u0442 `0`.' if localized else 'When `count < 0`, the method returns `0`.'
                answer['claims'].append({**claim, 'text': literal_text, 'cites': [cited], 'returns': [ref]})
                answer['steps'] = [
                    {'from': 'handler', 'to': 'caller', 'kind': 'return', 'label': '0', 'condition': 'count < 0'},
                    {'from': 'handler', 'to': 'handler', 'kind': 'note', 'label': 'accepted.add(count)', 'condition': None},
                ]
                if wrong_literal:
                    D['save'](self.seed / (slot['key'] + '-repair.json'), answer)
                    answer['claims'][-1]['text'] = literal_text.replace('`0`', '`9`')
            if kind == 'PROCESS_MAP':
                operation, = packet['context']['operations']
                answer['routes'] = [{'origin': operation['id'], 'participants': [], 'result': 'normal',
                    'title': '\u041f\u0440\u0438\u043d\u044f\u0442\u0438\u0435 \u0431\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u044f' if localized else 'Accept a reservation', 'condition': 'count >= 0',
                    'summary': text, 'outcome': '\u0420\u0430\u0437\u043c\u0435\u0440 \u0441\u043f\u0438\u0441\u043a\u0430' if localized else 'Updated list size',
                    'terminal': operation['id'], 'boundary': 'return', 'recipient': 'Caller contract is not captured.',
                    'steps': [{'from': operation['id'], 'to': 'caller', 'kind': 'return', 'label': 'accepted.size',
                               'condition': 'count >= 0'}], 'cites': [alias]}]
            D['save'](self.seed / (slot['key'] + '-response.json'), answer)
        return run, plan, packets, source

    def test_isolated_normal_entry_uses_distinct_authors_and_native_reader(self):
        run, plan, packets, source = self.isolated_fixture(language='ru')
        operation = packets['OPERATION']
        ranges = list(operation['sourceRanges'].values())
        self.assertEqual(len(ranges), 1)
        self.assertEqual(ranges[0]['text'], source['text'])
        self.assertNotIn('operations', operation['context'])
        self.assertEqual(operation['context']['operation']['id'], self.entry['handlerId'])
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        self.assertEqual(len({row['threadId'] for row in state['attempts']}), len(plan['slots']))
        self.assertEqual(state['modelTokens'], 20 * len(plan['slots']))
        html = Path(result['ready']) / 'html'
        operation_html = (html / self.entry['page'].replace('.md', '.html')).read_text()
        self.assertIn('POST /reservations', operation_html)
        self.assertIn('sequence-message return', operation_html)
        self.assertIn('\u041a\u0430\u0440\u0442\u0430 \u043f\u0440\u043e\u0446\u0435\u0441\u0441\u043e\u0432', (html / 'process-map.html').read_text())
        self.assertIn('<th>\u041f\u043e\u043b\u0435</th>', (html / 'reservations-contracts.html').read_text())
        route, = html.glob('route-*.html')
        self.assertIn('\u041f\u0440\u0438\u043d\u044f\u0442\u0438\u0435 \u0431\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u044f', route.read_text())
        self.assertIn('sequence-message return', route.read_text())

    def control_isolated_slots(self, rules):
        self.mode.write_text('controlled-page')
        D['save'](self.seed / 'queue-control.json', rules)

    def test_isolated_queue_refills_while_first_slot_still_waits(self):
        run, plan, _, _ = self.isolated_fixture()
        slow, other, fast, fourth, fifth = (row['key'] for row in plan['slots'])
        initial = [f'{slot}.waiting' for slot in (slow, other, fast)]
        self.control_isolated_slots({
            slow: {'until': [f'{fourth}.started']},
            other: {'until': [f'{slow}.finished']},
            fast: {'until': initial},
        })
        state = D['author'](run)
        self.assertEqual(state['status'], 'AUTHOR_COMPLETED')
        started_fourth = float((self.seed / f'{fourth}.started').read_text())
        self.assertLess(started_fourth, float((self.seed / f'{slow}.finished').read_text()))
        self.assertGreater(started_fourth, float((self.seed / f'{fast}.finished').read_text()))
        self.assertCountEqual([row['slot'] for row in state['attempts']], [slow, other, fast, fourth, fifth])
        self.assertEqual(state['modelTokens'], 100)
        intervals = sorted((at, delta) for row in state['attempts'] for at, delta in
                           ((row['launchedAt'], 1), (row['completedAt'], -1)))
        active = peak = 0
        for _, delta in intervals:
            active += delta
            peak = max(peak, active)
        self.assertEqual(peak, 3)
        for slot, attempt in zip(plan['slots'], state['attempts']):
            self.assertEqual(slot['key'], attempt['slot'])
            self.assertEqual((D['PX']['attempt_dir'](run, attempt) / 'prompt.txt').read_bytes(),
                             (run / 'inputs' / slot['prompt']).read_bytes())

    def test_isolated_queue_terminal_cancels_and_accounts_for_already_started_slots(self):
        run, plan, _, _ = self.isolated_fixture()
        slow, invalid, fast, fourth, fifth = (row['key'] for row in plan['slots'])
        path = self.seed / f'{invalid}-response.json'
        response = D['load'](path)
        response['claims'][0]['cites'] = ['Unbound']
        D['save'](path, response)
        self.control_isolated_slots({
            slow: {'until': ['never-release']},
            invalid: {'until': [f'{fourth}.waiting']},
            fast: {'until': [f'{slow}.waiting', f'{invalid}.waiting']},
            fourth: {'until': ['never-release'], 'report': False},
        })
        with self.assertRaises(D['Stop']) as raised:
            D['author'](run)
        self.assertEqual(raised.exception.code, 'PAGE_AUTHOR_REPAIRS_EXHAUSTED')
        self.assertEqual(raised.exception.details['slot'], invalid)
        state = D['load'](run / 'state.json')
        self.assertEqual(state['status'], 'AUTHOR_REPAIRS_EXHAUSTED')
        self.assertEqual(state['terminalFailure'], raised.exception.details)
        attempts = {row['slot']: row for row in state['attempts']}
        self.assertEqual(set(attempts), {slow, invalid, fast, fourth})
        self.assertFalse((self.seed / f'{fifth}.started').exists())
        self.assertFalse((run / 'page-attempts' / fifth).exists())
        for row in (attempts[slow], attempts[fourth]):
            self.assertIn('launchedAt', row)
            self.assertEqual(row['failure']['code'], 'PAGE_AUTHOR_INTERRUPTED')
            self.assertEqual(D['load'](D['PX']['attempt_dir'](run, row) / 'result.json'), row)
        self.assertEqual(attempts[slow]['usage']['output_tokens'], 10)
        self.assertIsNone(attempts[fourth]['usage'])
        self.assertEqual(state['tokenAccounting']['status'], 'UNAVAILABLE')
        self.assertEqual(state['tokenAccounting']['knownBudgetTokensLowerBound'], 60)
        with self.assertRaises(D['Stop']) as again:
            D['author'](run)
        self.assertEqual(again.exception.code, raised.exception.code)
        self.assertEqual(D['load'](run / 'state.json'), state)

    def test_isolated_queue_shared_deadline_and_receipt_recovery(self):
        run, plan, _, _ = self.isolated_fixture(wall_seconds=1)
        first = [row['key'] for row in plan['slots'][:3]]
        self.control_isolated_slots({slot: {'until': ['never-release']} for slot in first})
        with self.assertRaises(D['Stop']) as raised:
            D['author'](run)
        self.assertEqual(raised.exception.code, 'AUTHOR_BUDGET_EXHAUSTED')
        state = D['load'](run / 'state.json')
        self.assertEqual([row['slot'] for row in state['attempts']], first)
        self.assertEqual(state['modelTokens'], 60)
        self.assertTrue(all(row['failure']['code'] == 'PAGE_AUTHOR_INTERRUPTED' for row in state['attempts']))
        # Simulate a crash between child receipt and the coordinator's journal.
        last = state['attempts'][-1]
        reservation = {field: last[field] for field in ('slot', 'number', 'argv', 'resumeThread', 'startedAt')}
        reservation.update(status='RUNNING', exitCode=None)
        journal = {**state, 'attempts': state['attempts'][:-1] + [reservation]}
        D['save'](run / 'state.json', journal)
        with self.assertRaises(D['Stop']) as again:
            D['author'](run)
        self.assertEqual(again.exception.code, raised.exception.code)
        self.assertEqual(D['load'](run / 'state.json'), state)
        # A missing receipt cannot authorize replay of a possibly paid attempt.
        D['save'](run / 'state.json', journal)
        (D['PX']['attempt_dir'](run, last) / 'result.json').unlink()
        with self.assertRaises(D['Stop']) as indeterminate:
            D['author'](run)
        self.assertEqual(indeterminate.exception.code, 'PAGE_AUTHOR_ATTEMPT_INDETERMINATE')
        self.assertTrue(indeterminate.exception.blocked)
        self.assertEqual(D['load'](run / 'state.json'), journal)

    def test_isolated_queue_interruption_resumes_only_the_measured_owning_threads(self):
        run, plan, _, _ = self.isolated_fixture(repairs=1)
        first = [row['key'] for row in plan['slots'][:3]]
        self.control_isolated_slots({slot: {'until': ['never-release']} for slot in first})
        argv = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'),
                'run', '--run-dir', str(run)]
        with subprocess.Popen(argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as process:
            try:
                expires = time.monotonic() + 4
                while not all((self.seed / f'{slot}.waiting').exists() for slot in first):
                    self.assertLess(time.monotonic(), expires, 'Authors did not start')
                    time.sleep(.01)
                process.send_signal(signal.SIGINT)
                process.communicate(timeout=5)
            finally:
                if process.poll() is None:
                    process.kill()
        interrupted = D['load'](run / 'state.json')
        self.assertEqual(interrupted['status'], 'INTERRUPTED')
        self.assertEqual(interrupted['modelTokens'], 60)
        self.assertEqual([row['slot'] for row in interrupted['attempts']], first)
        self.assertTrue(all(row['failure']['code'] == 'PAGE_AUTHOR_INTERRUPTED' for row in interrupted['attempts']))
        self.mode.write_text('normal')
        resumed = D['author'](run)
        self.assertEqual(resumed['status'], 'AUTHOR_COMPLETED')
        self.assertEqual(resumed['attempts'][:3], interrupted['attempts'])
        self.assertEqual(resumed['modelTokens'], 160)
        self.assertCountEqual([row['slot'] for row in resumed['attempts'][3:]], [row['key'] for row in plan['slots']])
        for before in interrupted['attempts']:
            repair, = [row for row in resumed['attempts'][3:] if row['slot'] == before['slot']]
            self.assertEqual((repair['number'], repair['resumeThread'], repair['threadId']),
                             (1, before['threadId'], before['threadId']))

    def test_isolated_literal_repair_reuses_only_operation_thread(self):
        run, plan, packets, _ = self.isolated_fixture(repairs=1, wrong_literal=True)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        self.assertEqual(len(state['attempts']), len(plan['slots']) + 1)
        attempt = state['attempts'][-1]
        self.assertEqual(packets['OPERATION']['slot'], attempt['slot'])
        self.assertEqual(attempt['resumeThread'], attempt['threadId'])
        self.assertEqual(state['modelTokens'], 20 * (len(plan['slots']) + 1))
        repair = (run / 'page-attempts' / attempt['slot'] / '1/prompt.txt').read_text()
        self.assertIn('LITERAL_RETURN_VALUE_MISMATCH', repair)
        self.assertNotIn('"expected"', repair)
        self.assertEqual(json.loads(repair.split('\n\n')[-1]), packets['OPERATION'])

    def test_isolated_response_schema_and_normalization_keep_owned_reader_content(self):
        run, plan, _, _ = self.isolated_fixture()
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        source = self.seed / (operation['key'] + '-response.json')
        response = D['load'](source)
        alias = next(iter(operation['citations']))
        region, first, last = operation['citationSpans'][alias][0]
        self.assertEqual(operation['sourceRegions'][region], [first, last])
        response['title'] = 'Reader title supplied on a fixed-title operation'
        response['participants'] = [{'id': 'caller', 'title': 'Starter caller metadata'},
                                    {'id': 'RowReader', 'title': 'Row reader'}]
        response['steps'] = [
            {'from': 'handler', 'to': 'RowReader', 'kind': 'await', 'label': 'Read rows', 'condition': ''},
            {'from': 'RowReader', 'to': 'handler', 'kind': 'return', 'label': 'Return selected rows', 'condition': 'Read succeeds'},
        ]
        response['claims'][0]['cites'] = [
            f'{alias}:{first}-{last}', f'{alias}#L{first}-L{last}', operation['citations'][alias], alias, region,
        ]
        original = D['digest'](response)
        canonical = D['P']['validate_response'](operation, response)
        self.assertEqual(D['digest'](response), original)
        self.assertEqual(canonical['claims'][0]['cites'], [alias])
        self.assertEqual(canonical['claims'][0]['text'], response['claims'][0]['text'])
        self.assertEqual(canonical['participants'], [{'id': 'rowreader', 'title': 'Row reader'}])
        self.assertEqual([step['kind'] for step in canonical['steps']], ['await', 'return'])
        self.assertEqual(canonical['steps'][0]['to'], canonical['steps'][1]['from'])
        self.assertIsNone(canonical['steps'][0]['condition'])
        self.assertIsNone(canonical['title'])

        schema = D['load'](run / 'inputs' / operation['schema'])
        self.assertNotIn('title', schema['properties'])
        self.assertNotIn('routes', schema['properties'])
        self.assertEqual(schema['properties']['slot']['enum'], [operation['key']])
        self.assertEqual(set(schema['$defs']['citation']['enum']), set(operation['citations']))
        self.assertEqual(set(schema['$defs']['claim']['properties']['returns']['items']['enum']), set(operation['returns']))
        prompt = (run / 'inputs' / operation['prompt']).read_text()
        self.assertIn('add the return edge from', prompt)
        self.assertNotIn('CONTRACTS |', prompt)
        guide = next(slot for slot in plan['slots'] if slot['kind'] == 'GUIDE')
        guide_schema = D['load'](run / 'inputs' / guide['schema'])
        self.assertNotIn('returns', guide_schema['$defs']['claim']['properties'])
        self.assertIn('title', guide_schema['required'])

        # Exercise schema-minimal shape through the assembled native reader.
        for field in ('discovery', 'starts', 'contracts', 'routes', 'title'):
            canonical.pop(field)
        D['save'](source, canonical)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = (Path(result['ready']) / 'html' / self.entry['page'].replace('.md', '.html')).read_text()
        self.assertIn('Return selected rows', html)
        self.assertIn('sequence-message return', html)

    def test_isolated_diagnostics_reject_outside_or_ambiguous_evidence_in_one_pass(self):
        run, plan, _, _ = self.isolated_fixture()
        slot = next(row for row in plan['slots'] if row['kind'] == 'OPERATION')
        answer = D['load'](self.seed / (slot['key'] + '-response.json'))
        alias = next(iter(slot['citations']))
        region, first, last = slot['citationSpans'][alias][0]
        slot['citations']['Sother'] = 'different-admitted-handle'
        slot['citationSpans']['Sother'] = [[region, first, last]]
        answer['claims'][0]['cites'] = [f'{alias}:{last + 1}', region, 'Sunknown']
        answer['claims'][1]['cites'] = []
        answer['claims'][1]['returns'] = ['Rother']
        answer['steps'][0]['to'] = 'undeclared'
        answer['steps'][0]['condition'] = 'two\nlines'
        with self.assertRaises(D['Stop']) as raised:
            D['P']['validate_response'](slot, answer)
        issues = {row['path']: row for row in raised.exception.details['issues']}
        self.assertEqual(raised.exception.code, 'PAGE_RESPONSE_INVALID')
        self.assertEqual(issues['/claims/0/cites/0']['received'], f'{alias}:{last + 1}')
        self.assertCountEqual(issues['/claims/0/cites/1']['matchingAliases'], [alias, 'Sother'])
        self.assertEqual(issues['/claims/1/cites']['code'], 'PAGE_CITATION_REQUIRED')
        self.assertEqual(issues['/claims/1/returns/0']['allowed'], list(slot['returns']))
        self.assertEqual(issues['/claims']['missing'], list(slot['returns']))
        self.assertIn('caller', issues['/steps/0/to']['allowed'])
        self.assertIn('/steps/0/condition', issues)

    def test_isolated_format_repair_reuses_verified_packet_without_resending_source(self):
        run, plan, _, _ = self.isolated_fixture(repairs=1)
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        path = self.seed / (operation['key'] + '-response.json')
        response = D['load'](path)
        D['save'](self.seed / (operation['key'] + '-repair.json'), response)
        response['claims'][0]['cites'] = ['Unbound']
        D['save'](path, response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        self.assertEqual(len(state['attempts']), len(plan['slots']) + 1)
        retry, = [attempt for attempt in state['attempts'] if attempt['number'] == 1]
        self.assertEqual(retry['slot'], operation['key'])
        self.assertEqual(retry['threadId'], retry['resumeThread'])
        prompt = D['load'](D['PX']['attempt_dir'](run, retry) / 'prompt.txt')
        self.assertNotIn('sourceRanges', prompt)
        self.assertEqual(prompt['formatRepair']['issues'][0]['path'], '/claims/0/cites/0')
        self.assertEqual(prompt['formatRepair']['issues'][0]['received'], 'Unbound')

    def test_isolated_repair_exhaustion_is_terminal_and_spends_no_more_attempts(self):
        run, plan, _, _ = self.isolated_fixture()
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        path = self.seed / (operation['key'] + '-response.json')
        response = D['load'](path)
        response['claims'][0]['cites'] = []
        D['save'](path, response)
        with self.assertRaises(D['Stop']) as error:
            D['execute'](run)
        state = D['load'](run / 'state.json')
        self.assertEqual(state['status'], 'AUTHOR_REPAIRS_EXHAUSTED')
        self.assertEqual(state['terminalFailure'], error.exception.details)
        with self.assertRaises(D['Stop']) as again:
            D['execute'](run)
        self.assertEqual(again.exception.code, 'PAGE_AUTHOR_REPAIRS_EXHAUSTED')
        self.assertEqual(D['load'](run / 'state.json'), state)

    def test_isolated_reused_thread_cannot_qualify(self):
        run, _, _, _ = self.isolated_fixture()
        self.mode.write_text('duplicate-page-thread')
        with self.assertRaises(D['Stop']) as error:
            D['execute'](run)
        self.assertEqual(error.exception.code, 'PAGE_AUTHOR_THREAD_REUSED_ACROSS_SLOTS')
        self.assertFalse((run / 'ready').exists())

    def test_isolated_prepared_peer_requires_selected_handle_and_matching_work_revision(self):
        pages = D['P']
        home = self.root / 'page-source'
        home.mkdir()
        D['save'](home / 'requirements.json', {})
        peer_manifest = home / 'peer' / 'citations.json'
        proof = {'file': 'peer.ts', 'startLine': 3, 'endLine': 3, 'service': 'peer', 'revision': 'c' * 40,
                 'manifest': 'peer/citations.json'}
        selected = {**proof, 'id': 'chosen'}
        unrelated = {**proof, 'id': 'other', 'file': 'unrelated.ts'}
        packet = {'scope': {'snapshot': F['SNAPSHOT']}, 'work': 'peer-work',
                  'outcomes': [{'source': source, 'textKey': source['id']} for source in (selected, unrelated)],
                  'texts': {'chosen': 'Read peer', 'other': 'Unrelated peer'}}
        entries = {'peer:cite': {'source': selected}, 'peer:other': {'source': unrelated}}
        source_scope = {'sourceIndex': str(home / 'index.md'), 'manifest': None,
                        'coverage': {}, 'deferredResourceFiles': [],
                        'preparedFacts': {'citations': {'peer:cite': proof}}}
        scopes = {self.service: {'revision': self.source['revision']}, 'peer': {'revision': 'c' * 40, 'work': 'peer-work'}}

        def citation_load(path):
            self.assertEqual(path, peer_manifest)
            return {}, packet, {}, entries

        with patch.dict(pages['service_evidence'].__globals__, {'citation_load': citation_load}):
            loaded = pages['service_evidence'](source_scope, self.service, scopes, F['SNAPSHOT'])
            self.assertEqual(set(loaded['sources']), {'peer:cite'})
            packet['work'] = 'different-work'
            with self.assertRaisesRegex(D['Stop'], 'PAGE_PEER_SOURCE_MEMBERSHIP_CHANGED'):
                pages['service_evidence'](source_scope, self.service, scopes, F['SNAPSHOT'])

    def test_isolated_declared_peer_packet_and_route_keep_exact_service_ownership(self):
        reservation = self.entry | {'entrySymbol': 'demo.Reservations.reserve'}
        warehouse = reservation | {'id': 'warehouse:entry', 'handlerId': 'entry-warehouse', 'service': 'warehouse',
                                   'page': 'warehouse-get-stock.md', 'method': 'GET', 'path': '/stock',
                                   'entrySymbol': 'demo.Stock.find'}
        unrelated = warehouse | {'id': 'warehouse:other', 'handlerId': 'entry-other', 'page': 'warehouse-get-other.md',
                                  'path': '/other', 'entrySymbol': 'demo.Stock.other'}
        declaration = {'id': 'stock-check', 'title': 'Declared stock lookup',
                       'from': {'service': self.service, 'selector': {'owner': 'demo.Reservations', 'name': 'reserve'}},
                       'to': {'service': 'warehouse'}, 'transport': {'kind': 'HTTP', 'method': 'GET', 'path': '/stock'}}
        inventory = {'snapshot': F['SNAPSHOT'], 'requirements': [reservation, warehouse, unrelated, *(
            {'id': service + ':service-scope', 'service': service, 'kind': 'SERVICE', 'page': service + '.md'}
            for service in (self.service, 'warehouse'))], 'services': {
                self.service: {'peerDeclarations': [{'kind': 'DEPENDENCY', 'record': {
                    'kind': 'DECLARED_INTERACTION', 'normalized': declaration}}]}, 'warehouse': {}}}
        by_service = {}
        for row, code in zip((reservation, warehouse, unrelated), ('CALL_SITE', 'PEER_BODY', 'UNRELATED_BODY'), strict=True):
            sources = by_service.setdefault(row['service'], {}).setdefault('sources', {})
            cite = 'cite:' + row['handlerId']
            sources[cite] = {'file': 'src/' + row['handlerId'] + '.kt', 'service': row['service'], 'startLine': 1,
                             'endLine': 1, 'revision': 'owned', 'textDigest': D['sha'](code.encode()), 'text': code}
            by_service[row['service']].setdefault('ready', {})[row['id']] = {'evidenceCandidates': [cite]}
            by_service[row['service']].setdefault('facts', {})[row['id']] = {
                'facts': [{'text': code + ' fact', 'kind': 'BODY', 'authority': 'SOURCE', 'sources': [cite]}]}
        for index in by_service.values():
            index.update(returns=[], coverage={'coverage': 'SYNTAX'}, deferred=[])
        pages = D['P']
        with patch.dict(pages['page_jobs'].__globals__, {'service_evidence': lambda service, *_: by_service[service]}):
            job = dict(self.job, services=[self.service, 'warehouse'])
            directory = self.root / 'page-prompts'
            plan = pages['page_jobs'](job, inventory, {'services': {key: key for key in by_service}},
                                      directory, D['M']['MAX_PAGES'], 'Owned instructions.')
        own = next(slot for slot in plan['slots'] if slot['requirement'] == reservation['id'])
        packet = json.loads((self.root / own['prompt']).read_text().split('\n\n')[-1])
        self.assertEqual({row['text'] for row in packet['sourceRanges'].values()}, {'CALL_SITE', 'PEER_BODY'})
        self.assertEqual(set(packet['facts']), {reservation['id'], warehouse['id']})
        interaction, = packet['context']['declaredInteractions']
        self.assertEqual(interaction['peer'], warehouse['handlerId'])
        self.assertEqual(interaction['authority'], 'RETAINED_DECLARATION_NOT_EXECUTED')
        D['starter'](inventory, 'en', self.root / 'starter', all_entries=True)
        original = D['load'](self.root / 'starter/original.json')
        navigation = D['load'](self.root / 'starter/navigation.json')
        process = next(slot for slot in plan['slots'] if slot['kind'] == 'PROCESS_MAP')
        aliases = [alias for alias, cite in process['citations'].items()
                   if cite in ('cite:' + reservation['handlerId'], 'cite:' + warehouse['handlerId'])]
        response = {'claims': [{'heading': '', 'text': 'Stock lookup follows the reservation request.',
                                'cites': aliases, 'assessment': 'SUPPORTED', 'returns': []}],
                    'routes': [{'title': 'Check stock', 'summary': 'Inspect stock before accepting a request.',
                                'origin': reservation['handlerId'], 'participants': [{'entry': warehouse['handlerId'], 'role': 'Read stock'}],
                                'condition': 'A reservation requests stock.', 'outcome': 'Read completes.',
                                'result': 'normal', 'terminal': warehouse['handlerId'], 'boundary': 'return',
                                'recipient': 'The observed receiver returns to its caller.',
                                'steps': [{'from': reservation['handlerId'], 'to': warehouse['handlerId'],
                                           'kind': 'await', 'label': 'Wait for stock', 'condition': None}], 'cites': aliases}]}
        catalog = pages['add_processes'](original, {'claims': {}}, navigation, plan, process, response,
                                         job, pages['LABELS']['en'], plan['citationOwners'])
        route, = catalog['routes']
        self.assertEqual(route['outcomes'][0]['internal']['occurrences'], [])
        draft = self.root / 'isolated-multi-draft'
        D['save'](draft / 'navigation.json', navigation)
        D['save'](draft / 'routes.json', catalog)
        _, checked = D['route_preflight'](job, draft, [row['page'] for row in original['pages']])
        self.assertTrue(checked['localNarratives'])
        response['routes'][0]['cites'] = list(aliases)[:1]
        with self.assertRaises(D['Stop']) as error:
            pages['add_processes']({'pages': []}, {'claims': {}}, navigation, plan, process, response,
                                    job, pages['LABELS']['en'], plan['citationOwners'])
        self.assertEqual(error.exception.code, 'PAGE_ROUTE_PARTICIPANT_EVIDENCE_REQUIRED')

    def test_isolated_operation_receives_only_reached_discovery_facts_with_narrow_source(self):
        pages = D['P']
        reach = pages['REACHED']
        helper = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/docs_run_prepared.py'),
                                init_globals={'digest': D['digest'], 'sha': D['sha']})
        root_text = 'fun handle() {\n    primary.probe()\n}\n'
        target_text = 'class Primary {\n    fun probe() {\n        require(valid)\n        repository.recheck()\n    }\n}\n'
        neighbor_text = 'class Archive { fun probe() { throw IllegalStateException() } }\n'
        deeper_text = 'class Repository {\n    fun recheck() {\n        if (cached) return\n    }\n}\n'
        service = self.service

        def source(file, text, full=None, offset=0):
            before = (full or text)[:offset]
            return {'service': service, 'revision': 'owned', 'file': file, 'text': text,
                    'startByte': len(before.encode()), 'endByte': len((before + text).encode()),
                    'startLine': 1 + before.count('\n'), 'endLine': 1 + (before + text.rstrip('\n')).count('\n'),
                    'textDigest': D['sha'](text.encode()), 'authority': 'EXACT_SNAPSHOT_TEXT'}

        def sub(file, text, token):
            return source(file, token, text, text.index(token))

        combined = target_text + neighbor_text
        all_sources = {
            'root': source('Handler.kt', root_text),
            'unit': source('Probes.kt', combined),
            'duplicate-unit': source('Probes.kt', combined),
            'repository-unit': source('Repository.kt', deeper_text),
        }
        root = all_sources['root']
        site = sub('Handler.kt', root_text, 'primary.probe()')
        target = sub('Probes.kt', combined, 'fun probe() {\n        require(valid)\n        repository.recheck()\n    }')
        other = sub('Probes.kt', combined, 'fun probe() { throw IllegalStateException() }')
        deeper = sub('Repository.kt', deeper_text, 'fun recheck() {\n        if (cached) return\n    }')
        deeper_site = sub('Probes.kt', combined, 'repository.recheck()')
        source_map = {'root': root, 'site': site, 'a': target, 'b': other, 'deeper': deeper, 'deeper-site': deeper_site}
        bodies = [{'kind': 'BODY', 'declaration': ref, 'sources': [ref]} for ref in ('root', 'a', 'b', 'deeper')]
        exact_edges = [
            {'kind': 'CALL', 'declaration': 'root', 'targetMethodId': 'a', 'sources': ['site'],
             'resolution': 'SAME_SCOPE_UNIQUE', 'relationshipAuthority': 'RETAINED_K2_COMPILER_SELECTED_SYMBOL_NOT_RUNTIME_DISPATCH'},
            {'kind': 'CALL', 'declaration': 'a', 'targetMethodId': 'deeper', 'sources': ['deeper-site'],
             'resolution': 'SAME_SCOPE_SELECTED_DECLARATION', 'relationshipAuthority': 'JAVAC_SELECTED_DECLARATION_NOT_CONTROL_FLOW'},
        ]
        unqualified = {'kind': 'CALL', 'declaration': 'root', 'targetMethodId': 'b', 'sources': ['site'],
                       'resolution': 'SOURCE_NAME_CANDIDATE', 'relationshipAuthority': 'NAVIGATION_ONLY'}
        process = {'root': 'root', 'facts': [*bodies, *exact_edges, unqualified], 'sources': source_map}
        bound_targets = {'Handler.kt': 'root', 'Probes.kt': 'unit', 'Repository.kt': 'repository-unit'}

        def bind(row):
            handle = bound_targets[row['file']]
            return handle if reach['relative_slice'](all_sources[handle], row) is not None else None

        def proposition(kind, text, site, handle):
            return {'kind': kind, 'authority': 'RETAINED_SOURCE', 'text': text, 'sources': [handle],
                    'witnesses': [reach['witness'](all_sources[handle], site, handle)]}

        pulled = [
            proposition('guard-return', 'Validation can fail before lookup.', sub('Probes.kt', combined, 'require(valid)'), 'unit'),
            proposition('return', 'Cached result bypasses the next effect.', sub('Repository.kt', deeper_text, 'if (cached) return'), 'repository-unit'),
            proposition('throw', 'Archive probe throws.', sub('Probes.kt', combined, 'throw IllegalStateException()'), 'duplicate-unit'),
        ]
        discovery = reach['index_discovery']([pulled], all_sources)
        selected, limited = reach['select'](process, discovery, all_sources, bind)
        self.assertEqual(limited, 0)
        self.assertEqual([row['kind'] for row in selected], ['guard-return', 'return'])
        self.assertTrue(all('If control reaches' in row['condition'] for row in selected))
        self.assertEqual(selected[1]['sources'], ['root', 'unit', 'repository-unit'])
        callback = {**process, 'facts': bodies,
                    '_background': {'calls': [('site', 'a'), ('deeper-site', 'deeper'), ('site', 'missing')]}}
        callback_rows, _ = reach['select'](callback, discovery, all_sources, bind)
        self.assertEqual(callback_rows, selected)
        aliases, _, regions = pages['source_payload'](['unit', 'duplicate-unit'], all_sources)
        self.assertEqual(len(aliases), 2)
        self.assertEqual(len(regions), 1)

        operation = self.entry
        discovery_id = service + ':discover:Probes.kt'
        scope = {'snapshot': F['SNAPSHOT'], 'requirements': [operation,
                 {'id': service + ':service-scope', 'service': service, 'kind': 'SERVICE', 'page': service + '.md'},
                 {'id': discovery_id, 'service': service, 'kind': 'DISCOVERY', 'page': service + '.md', 'file': 'Probes.kt'}],
                 'services': {service: {}}}
        for row in selected:
            row.pop('_reachedDeclaration')
        prepared = {operation['id']: {'propositions': selected, 'omittedForLimit': 2},
                    discovery_id: {'propositions': pulled}}
        exposed = helper['author_entries'](prepared, all_sources)
        index = {'sources': all_sources, 'facts': exposed, 'coverage': {'coverage': 'SYNTAX'}, 'returns': [], 'deferred': [],
                 'ready': {operation['id']: {'evidenceCandidates': ['root']},
                           discovery_id: {'evidenceCandidates': ['unit']}}}
        with patch.dict(pages['page_jobs'].__globals__, {'service_evidence': lambda *_: index}):
            plan = pages['page_jobs'](self.job, scope, {'services': {service: {}}},
                                      self.root / 'page-prompts', D['M']['MAX_PAGES'], 'Owned instructions.')
        slot = next(row for row in plan['slots'] if row['kind'] == 'OPERATION')
        packet = pages['expand_fact_context'](json.loads((self.root / slot['prompt']).read_text().split('\n\n')[-1]))
        self.assertEqual(list(packet['facts']), [operation['id']])
        self.assertEqual(packet['omittedPreparedFacts'], 0)
        self.assertEqual(packet['preparationOmissions'][operation['id']]['facts'], 2)
        self.assertEqual({row['text'] for row in packet['sourceRanges'].values()}, {root_text, target['text'], deeper['text']})
        self.assertNotIn('Archive', json.dumps(packet))
        self.assertNotIn('_sourceWitnesses', json.dumps(packet))
        citations = slot['citations']
        self.assertEqual({citations[alias] for alias in packet['facts'][operation['id']]['facts'][1]['sources']},
                         {'root', 'unit', 'repository-unit'})

        exposed[operation['id']]['facts'][1]['text'] += ' Additional retained detail.' * 650
        with patch.dict(pages['page_jobs'].__globals__, {
            'service_evidence': lambda *_: index, 'MAX_PACKET_BYTES': 16975,
        }):
            reduced = pages['page_jobs'](self.job, scope, {'services': {service: {}}},
                                         self.root / 'reduced-prompts', D['M']['MAX_PAGES'], 'Owned instructions.')
        slot = next(row for row in reduced['slots'] if row['kind'] == 'OPERATION')
        bounded = json.loads((self.root / slot['prompt']).read_text().split('\n\n')[-1])
        self.assertGreater(bounded['omittedPreparedFacts'], 0)
        self.assertGreater(bounded['omittedOptionalRanges'], 0)

    def test_prepared_facts_deliver_only_bound_witnesses_and_use_the_normal_renderer(self):
        def project(process, context, *, node):
            proof, = context['sourceRanges']
            self.assertEqual(proof['references'], ['s9'])
            text = 'return 0'
            start = proof['text'].encode().find(text.encode())
            witness = {**proof, 'text': text, 'startByte': start, 'endByte': start + len(text),
                       'startLine': 2, 'endLine': 2, 'textDigest': D['sha'](text.encode())}
            accepted = {'kind': 'return', 'authority': 'SOURCE_EVIDENCE', 'status': 'SUPPORTED',
                        'text': 'The handler returns zero.', 'condition': [{'text': 'The count is negative.'}],
                        'uncertainty': 'Serialization remains open.', 'sources': ['exact-witness']}
            if process['root'] is None:
                return [accepted, {'kind': 'CALL_BOUNDARY', 'authority': 'SOURCE_EVIDENCE',
                                   'text': 'Callback invocation remains open.', 'sources': [proof['id']]}], {
                    'exact-witness': witness}
            return [accepted, {**accepted, 'text': 'Foreign body.', 'sources': ['wrong-revision']}], {
                'exact-witness': witness, 'wrong-revision': {**witness, 'revision': '0' * 40}}

        run, prepared, _, _ = self.prepare_inline_fixture(project=project)
        self.assertEqual(prepared['authorMode'], 'INLINE')
        self.assertEqual(prepared['preparedFacts'][self.service]['facts'], 2)
        self.assertEqual(prepared['preparedFacts'][self.service]['statusCounts'], {'PREPARED': 2})
        prompt = (run / 'prompt.txt').read_text()
        packet = json.loads(prompt.split('\n\n')[-1])
        self.assertNotIn('preparedFacts', packet)
        fact, = packet['entries']['E1']['facts']
        self.assertEqual(fact['sources'], ['S1'])
        self.assertEqual(fact['lines'], {'S1': ['2']})
        self.assertEqual(fact['alsoFor'], ['F1'])
        self.assertNotIn('facts', packet['files']['F1'])
        boundary, = packet['files']['F1']['boundaries']
        self.assertEqual(boundary['text'], 'Callback invocation remains open.')
        self.assertNotIn('witnesses', fact)
        self.assertEqual(fact['condition'], 'The count is negative.')
        home = run / 'inputs/author-source' / self.service
        reviewer = D['load'](home / 'prepared-facts.json')['entries'][self.entry['id']]
        self.assertEqual(reviewer['unboundPropositions'], 1)
        witness, = reviewer['propositions'][0]['witnesses']
        self.assertEqual(witness['textDigest'], D['sha'](b'return 0'))
        self.assertEqual(witness['cite'], reviewer['propositions'][0]['sources'][0])
        self.assertNotIn('Foreign body.', prompt)
        response = self.inline_response()
        D['save'](self.seed / 'inline-response.json', response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = Path(result['ready']) / 'html/reservations-post-reservations.html'
        self.assertIn('Return the updated list size', html.read_text())

    def test_prepared_facts_workspace_includes_page_scoped_evidence_before_verbatim_source(self):
        def project(process, context, *, node):
            proof, = context['sourceRanges']
            fact = {'kind': 'RETURN', 'authority': 'SOURCE_EVIDENCE', 'status': 'SUPPORTED',
                    'text': 'The method can return zero.', 'condition': 'The count is negative.',
                    'sources': [proof['id']]}
            boundary = {'kind': 'CALL_BOUNDARY', 'authority': 'SOURCE_EVIDENCE',
                        'text': 'The dynamic call target remains open.', 'sources': [proof['id']]}
            return ([fact, boundary] if process['root'] else [fact]), {}

        self.job['language'] = 'ru'
        run, prepared, _, source = self.prepare_inline_fixture(project=project, extra_resource=True)
        self.assertEqual(prepared['authorMode'], 'WORKSPACE')
        summary = prepared['preparedFacts'][self.service]
        self.assertEqual((summary['facts'], summary['boundaries']), (2, 1))
        home = run / 'inputs/author-source' / self.service
        reviewer = D['load'](home / 'prepared-facts.json')['entries'][self.entry['id']]
        requirements = D['load'](home / 'requirements.json')
        view, = requirements[self.entry['id']]['views']
        text = (home / view).read_text()
        evidence, body = text[text.index('## Prepared source evidence'):].split('```', 1)
        self.assertIn('Cite ' + reviewer['propositions'][1]['sources'][0], evidence)
        self.assertIn('Facts for Page ' + self.entry['page'] + ', Discovery file ' + source['file'], evidence)
        self.assertIn('Boundaries for Page ' + self.entry['page'], evidence)
        self.assertIn('[RETURN, SOURCE_EVIDENCE, SUPPORTED]', evidence)
        self.assertIn('Condition The count is negative.', evidence)
        self.assertIn('The dynamic call target remains open.', evidence)
        self.assertEqual(evidence.count('The method can return zero.'), 1)
        self.assertEqual(body, '\n' + source['text'] + '\n```\n')
        self.assertEqual(summary['authorViewBytes'], len(evidence.encode()))
        self.assertEqual(summary['omittedAuthorViews'], 0)
        self.assertNotIn('fact-views', (home / 'index.md').read_text())
        self.assertFalse((home / 'fact-views').exists())
        prompt = (run / 'prompt.txt').read_text()
        self.assertIn('at the top of the source views', prompt)
        self.assertIn('Write the reader pages in ru', prompt)

    def test_prepared_workspace_bounds_shared_prefix_and_keeps_all_anchored_facts_in_view(self):
        def project(process, context, *, node):
            scope = 'Entry' if process['root'] else 'File'
            proof, = context['sourceRanges']
            anchor = {'authority': 'SOURCE_EVIDENCE', 'sources': [proof['id']]}
            return [
                {'kind': 'RETURN', 'text': f'{scope} observation {i:02d}. ' + 'A retained detail. ' * 25, **anchor}
                for i in range(12)
            ] + [{'kind': 'CALL_BOUNDARY', 'text': scope + ' dynamic call target remains open.', **anchor}], {}

        run, prepared, _, source = self.prepare_inline_fixture(project=project, extra_resource=True)
        home = run / 'inputs/author-source' / self.service
        view, = D['load'](home / 'requirements.json')[self.entry['id']]['views']
        prefix, tail = (home / view).read_bytes().split(b'```\n' + source['text'].encode() + b'\n```\n')
        summary = prepared['preparedFacts'][self.service]
        self.assertLessEqual(summary['maxAuthorPrefixBytes'], 6 * 1024)
        self.assertIn(b'Entry observation 00.', prefix)
        self.assertIn(b'File observation 00.', prefix)
        self.assertIn(b'Entry dynamic call target remains open.', prefix)
        self.assertIn(b'File dynamic call target remains open.', prefix)
        self.assertIn(b'## Additional prepared source evidence', tail)
        self.assertIn(f"{summary['additionalAuthorItems']} additional prepared items".encode(), prefix)
        self.assertEqual(summary['frontAuthorItems'] + summary['additionalAuthorItems'], 26)
        for scope in ('Entry', 'File'):
            for i in range(12):
                self.assertEqual((prefix + tail).count(f'{scope} observation {i:02d}.'.encode()), 1)

    def test_prepared_csharp_context_requires_the_whole_typed_cas_original(self):
        helper = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/docs_run_prepared.py'), init_globals={'sha': D['sha']})
        raw = b'class Ledger { int Post() => 1; }\n'
        digest = D['sha'](raw)
        blob = D['sha'](b'codeclew-cas/v2\0codeclew-repository-input-blob/2.0\0' + raw)
        occurrence = {'snapshot': 'owned-snapshot', 'blob': blob, 'startByte': 0, 'endByte': len(raw)}
        source = {'file': 'Ledger.cs', 'revision': 'a' * 40, 'startLine': 1, 'text': raw.decode(),
                  'textDigest': digest, 'occurrence': occurrence, 'references': ['s2']}
        pin = {'snapshot': 'owned-snapshot', 'blob': {'schema': 'codeclew-cas-object/2.0',
               'objectSchema': 'codeclew-repository-input-blob/2.0', 'digest': blob, 'size': len(raw)}}
        fact = {'file': 'Ledger.cs', 'sourceContentDigest': digest,
                'sourceRange': {'byteStart': 0, 'byteEnd': len(raw)}, 'snapshotSource': pin}
        unit = {'record': {'normalized': fact, 'sourceIds': ['source']}, 'sourceReferences': ['s2']}
        admitted = helper['csharp_files']([unit], {'cite': source})
        self.assertEqual(admitted['Ledger.cs']['text'], raw.decode())
        self.assertEqual(admitted['Ledger.cs']['authority'], 'RETAINED_CSHARP_COMPILATION_UNIT')
        fact['snapshotSource']['blob']['digest'] = digest
        self.assertEqual(helper['csharp_files']([unit], {'cite': source}), {})

    def test_prepared_acquisition_readmits_successful_references_after_a_partial_batch(self):
        selected = [{'sourceId': 'read', 'sourceReference': 's1'}, {'sourceId': 'missing', 'sourceReference': 's2'}]
        calls = []
        result_dir = self.root / 'optional-sources'
        result_dir.mkdir()

        def command(argv, home, label, **kwargs):
            choices = D['load'](Path(argv[argv.index('--selection') + 1]))
            calls.append(choices)
            folder = home / label
            folder.mkdir()
            (folder / 'citations.json').write_text('{}')
            if len(choices) == 2:
                D['save'](folder / 'packet.json', {'outcomes': [
                    {'selection': choices[0], 'status': 'RETAINED_SOURCE_RETURNED'},
                    {'selection': choices[1], 'status': 'OWNED_SOURCE_NOT_AVAILABLE'}]})
                raise D['Stop']('SELECTED_SOURCES_INCOMPLETE')

        source = {**self.source, 'authority': 'EXACT_SNAPSHOT_TEXT'}
        text = source.pop('text')
        source['textDigest'] = D['sha'](text.encode())

        def citation_load(path):
            self.assertEqual(path.parent.name, 'read-sources-recovered')
            packet = {'scope': {'snapshot': F['SNAPSHOT']}, 'work': 'owned-work', 'texts': {'digest': text},
                      'outcomes': [{'source': source, 'textKey': 'digest', 'sourceBinding': {'reference': 's1'}}]}
            return {}, packet, {}, {'cite': {'source': source}}

        helper = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/docs_run_prepared.py'), init_globals={
            'sha': D['sha'], 'save': D['save'], 'load': D['load'], 'command': command,
            'Stop': D['Stop'], 'CitationStop': D['M']['C']['Stop'], 'need': D['need'], 'citation_load': citation_load,
        })
        rows = [[{'record': {'sourceIds': ['read', 'missing']}, 'sourceReferences': ['s1', 's2']}]]
        acquired = helper['acquire_sources'](self.job, self.service, {'work': 'owned-work', 'revision': source['revision']},
                                             result_dir, rows, {}, time.monotonic() + 30)
        self.assertEqual(calls, [selected, selected[:1]])
        self.assertEqual(acquired['cite']['references'], ['s1'])
        self.assertTrue((result_dir / 'read-sources/unadmitted-citations.json').is_file())
        self.assertFalse((result_dir / 'read-sources/citations.json').exists())

    def test_prepared_facts_no_adapter_output_retains_inline_source_authoring(self):
        run, prepared, _, source = self.prepare_inline_fixture(project=lambda *args, **kwargs: ([], {}))
        self.assertEqual(prepared['authorMode'], 'INLINE')
        self.assertEqual(prepared['preparedFacts'][self.service]['facts'], 0)
        self.assertEqual(prepared['preparedFacts'][self.service]['statusCounts'], {'NO_CITABLE_FACTS': 2})
        prompt = (run / 'prompt.txt').read_text()
        packet = json.loads(prompt.split('\n\n')[-1])
        self.assertNotIn('facts', packet['entries']['E1'])
        self.assertNotIn('boundaries', packet['entries']['E1'])
        home = run / 'inputs/author-source' / self.service
        view, = D['load'](home / 'requirements.json')[self.entry['id']]['views']
        self.assertNotIn('Prepared source evidence', (home / view).read_text())
        self.assertEqual(prepared['preparedFacts'][self.service]['authorViewBytes'], 0)
        self.assertIn(json.dumps(source['text'], ensure_ascii=False), prompt)

    def test_prepared_facts_projector_failure_preserves_retained_source_and_reason(self):
        def project(process, context, *, node):
            if process['root']:
                raise AttributeError('Private literal from an unsupported language structure')
            return [], {}

        run, prepared, _, source = self.prepare_inline_fixture(project=project)
        self.assertEqual(prepared['authorMode'], 'INLINE')
        self.assertEqual(prepared['preparedFacts'][self.service]['statusCounts'],
                         {'PROJECTOR_FAILED': 1, 'NO_CITABLE_FACTS': 1})
        sidecar = run / 'inputs/author-source' / self.service / 'prepared-facts.json'
        entries = D['load'](sidecar)['entries']
        self.assertEqual(entries[self.entry['id']]['failedProjectors'], {'managed_languages': 'AttributeError'})
        self.assertNotIn('Private literal', sidecar.read_text())
        self.assertIn(json.dumps(source['text'], ensure_ascii=False), (run / 'prompt.txt').read_text())

    def test_inline_normal_entry_point_assembles_explicit_content_and_discovered_start(self):
        run, prepared, membership, source = self.prepare_inline_fixture()
        self.assertEqual(prepared['authorMode'], 'INLINE')
        prompt = (run / 'prompt.txt').read_text()
        self.assertIn(json.dumps(source['text'], ensure_ascii=False), prompt)
        self.assertIn('"path":"/reservations"', prompt)
        self.assertNotIn('citation-index.json', prompt)
        self.assertNotIn('preparedFacts', prepared)
        self.assertNotIn('Use the facts and boundaries', prompt)
        self.assertNotIn('compose-original.md', prompt)
        self.assertNotIn(str(self.root), prompt)
        response = self.inline_response()
        response['entries'][0]['participants'] = [
            {'id': 'caller', 'title': 'Calling code'},
            {'id': 'handler', 'title': 'Reservations.reserve'},
        ]
        D['save'](self.seed / 'inline-response.json', response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        attempt, = state['attempts']
        self.assertEqual(attempt['authorMode'], 'INLINE')
        self.assertIn('--output-schema', attempt['argv'])
        self.assertIn('shell_tool', attempt['argv'])
        self.assertIn('unified_exec', attempt['argv'])
        self.assertIn('read-only', attempt['argv'])
        self.assertNotIn('--add-dir', attempt['argv'])
        self.assertEqual(attempt['usage'], {'input_tokens': 20, 'cached_input_tokens': 10, 'output_tokens': 10})
        self.assertEqual(attempt['argv'][attempt['argv'].index('-m') + 1], self.job['author']['model'])
        self.assertIn('model_reasoning_effort="max"', attempt['argv'])
        self.assertGreater(len(membership), 0)
        report = D['load'](run / 'report.json')
        self.assertFalse(report['semanticTruthAssessed'])
        self.assertEqual(report['authorVisualReview']['status'], 'UNAVAILABLE')
        entry_html = Path(result['ready']) / 'html/reservations-post-reservations.html'
        self.assertIn('Return the updated list size', entry_html.read_text())
        self.assertIn('HTTP binding or serialized response', entry_html.read_text())
        discovery_html = Path(result['ready']) / 'html/reservations-discovered-f1-1.html'
        self.assertIn('Pass the payload to accept', discovery_html.read_text())
        starter = D['load'](run / 'inputs/starter/original.json')
        built = D['load'](run / 'workspace/draft/original.json')
        def diagram(original):
            page = next(page for page in original['pages'] if page['page'] == 'reservations-post-reservations.md')
            return json.loads(page['fragments'][-1]['text'].split('\n', 1)[1].rsplit('```', 1)[0])
        self.assertEqual(diagram(built)['participants'], diagram(starter)['participants'])
        review = D['load'](run / 'workspace/draft/self-review.json')
        self.assertEqual(review['claims']['reservations-post-reservations.md']['claim-3'], 'EXPLICIT_UNKNOWN')

    def test_inline_rejects_duplicate_or_invalid_actor_ids_and_unknown_step_endpoints(self):
        run, _, _, _ = self.prepare_inline_fixture()
        plan = D['load'](run / 'inputs/author-plan.json')
        inventory = D['load'](run / 'inputs/inventory.json')
        cases = (
            (['caller', 'caller'], None, 'INVALID_INLINE_PARTICIPANT'),
            (['handler', 'handler'], None, 'INVALID_INLINE_PARTICIPANT'),
            (['worker', 'worker'], None, 'INVALID_INLINE_PARTICIPANT'),
            (['unexpected/id'], None, 'INVALID_INLINE_PARTICIPANT'),
            ([], 'from', 'INVALID_INLINE_STEP_PARTICIPANT'),
            ([], 'to', 'INVALID_INLINE_STEP_PARTICIPANT'),
        )
        for roles, endpoint, code in cases:
            with self.subTest(roles=roles, endpoint=endpoint), self.assertRaises(D['Stop']) as raised:
                response = self.inline_response()
                entry = response['entries'][0]
                entry['participants'] = [{'id': role, 'title': 'Declared participant'} for role in roles]
                if endpoint:
                    entry['steps'][0][endpoint] = 'unknown'
                D['I']['assemble'](response, plan, inventory, D['load'](run / 'inputs/starter/original.json'),
                                   D['load'](run / 'inputs/starter/navigation.json'))
            self.assertEqual(raised.exception.code, code)

    def test_inline_refuses_wrong_literal_then_repairs_in_same_thread(self):
        text = 'fun reserve(count: Int): Int {\n    if (count < 0) return 0\n    accepted.add(count)\n    return accepted.size\n}\n'
        run, _, _, _ = self.prepare_inline_fixture(text, repairs=1)
        D['save'](self.seed / 'inline-response.json', self.inline_response(added=False, literal='7'))
        D['save'](self.seed / 'inline-repair.json', self.inline_response(added=False))
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        one, two = D['load'](run / 'state.json')['attempts']
        self.assertEqual((one['authorMode'], two['authorMode']), ('INLINE', 'INLINE'))
        self.assertEqual(one['threadId'], two['threadId'])
        self.assertEqual(two['argv'][two['argv'].index('resume') + 1], one['threadId'])
        prompt = D['load'](run / 'attempts/1/prompt.txt')
        self.assertEqual(prompt['failures']['entrySequences']['code'], 'LITERAL_RETURN_VALUE_MISMATCH')
        self.assertIn('`7`', (run / 'attempts/0/draft/original.json').read_text())
        self.assertIn('`0`', (run / 'attempts/1/draft/original.json').read_text())
        review = D['load'](run / 'workspace/draft/self-review.json')
        self.assertIn('NO_ADDITIONAL_STARTS', json.dumps(review))
        self.assertEqual(D['load'](run / 'state.json')['modelTokens'], 40)

    def test_inline_explicit_need_for_evidence_recovers_in_workspace_within_same_repair_budget(self):
        run, _, _, _ = self.prepare_inline_fixture(repairs=1)
        plan = D['load'](run / 'inputs/author-plan.json')
        response = self.inline_response()
        D['save'](self.seed / 'inline-response.json', {**response, 'status': 'NEEDS_WORKSPACE',
            'reason': 'The requested diagram requires the extra retained dependency.'})
        # A normal workspace author writes the established draft contract.
        workspace_draft = D['I']['assemble'](response, plan, D['load'](run / 'inputs/inventory.json'),
            D['load'](run / 'inputs/starter/original.json'), D['load'](run / 'inputs/starter/navigation.json'))
        for name, value in workspace_draft.items():
            D['save'](self.seed / name, value)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        one, two = D['load'](run / 'state.json')['attempts']
        self.assertEqual(one['assemblyFailure']['code'], 'INLINE_AUTHOR_NEEDS_WORKSPACE')
        self.assertEqual((one['authorMode'], two['authorMode']), ('INLINE', 'WORKSPACE'))
        self.assertEqual(one['threadId'], two['threadId'])
        self.assertNotIn('--output-schema', two['argv'])
        self.assertIn('--add-dir', two['argv'])
        prompt = (run / 'attempts/1/prompt.txt').read_text()
        self.assertIn('Retained root and snapshot', prompt)
        self.assertIn('requires the extra retained dependency', prompt)
        self.assertEqual(D['load'](run / 'state.json')['modelTokens'], 40)

    def test_inline_requires_the_authors_explicit_discovery(self):
        run, _, _, _ = self.prepare_inline_fixture()
        D['save'](self.seed / 'inline-response.json', {**self.inline_response(), 'discovery': []})
        result = D['execute'](run)
        self.assertEqual(result['status'], 'FAILED')
        self.assertEqual(result['checks']['output']['code'], 'INLINE_SCOPE_ACCOUNTING_MISMATCH')
        self.assertEqual(result['checks']['output']['details']['field'], 'ref')
        self.assertFalse((run / 'ready').exists())

    def test_retained_resource_selects_workspace(self):
        root, receipt, _, _ = self.prepare_inline_fixture(extra_resource=True)
        self.assertEqual((receipt['authorMode'], receipt['authorModeReason']),
                         ('WORKSPACE', 'RETAINED_RESOURCES_ON_REQUEST'))
        self.assertIn('Prepared source index', (root / 'prompt.txt').read_text())

    def test_large_fully_captured_source_selects_workspace(self):
        source = '// ' + 'retained detail ' * 850 + '\nfun inspect() = 1\n'
        root, receipt, _, _ = self.prepare_inline_fixture(source)
        self.assertGreater(len(source.encode()), D['I']['MAX_SOURCE_BYTES'])
        self.assertEqual((receipt['authorMode'], receipt['authorModeReason']),
                         ('WORKSPACE', 'INLINE_SOURCE_BYTE_LIMIT'))
        self.assertIn('Prepared source index', (root / 'prompt.txt').read_text())

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

    def test_prepared_compiler_work_with_large_source_part_promotes_and_rechecks_live_identity(self):
        text = '// captured original ' + 'x' * 80 + '\n'
        large = F['source'](text * 650)
        large.update(id='source-original-unit', service=self.service, revision=self.source['revision'],
                     file='src/OriginalUnit.cs', startLine=1, endLine=650)
        body = large['text'].encode()
        ref = 's10'
        scope = self.inventory['services'][self.service]
        first = F['part'](large, 0, 32000, 'second', ref)
        rest = F['part'](large, 32000, len(body), None, ref)
        for part in (first, rest):
            part['work'] = scope['work']
            part['receiptDigest'] = F['digest']({key: value for key, value in part.items() if key != 'receiptDigest'})
        parts = self.root / 'native-parts.json'
        D['save'](parts, {'first': first, 'second': rest})

        work_page = F['page']({**scope['coverage'], 'id': self.service}, omitted=[{
            'kind': 'SOURCE', 'id': large['id'], 'reference': ref, 'reason': 'ITEM_EXCEEDS_WORK_BYTE_BUDGET'}])
        work_page.update(work=scope['work'], subject='service:' + self.service, total=2,
                         contextDigest='owned-context', inputDigest='owned-input',
                         membershipDigest='owned-members', authority='IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED')
        work_page['receiptDigest'] = F['digest'](work_page)
        compact = {'snapshot': self.job['snapshot'], 'items': [], 'omitted': [
            {'index': 0, 'reason': 'ITEM_EXCEEDS_STDOUT_BUDGET', 'id': large['id']}]}
        driver = self.clew.read_text()
        native = f"""
from pathlib import Path
if args[:3] == ['docs', 'work', 'read']:
    request = json.loads(Path(args[args.index('--input')+1]).read_text())
    assert request == {{'references': [{ref!r}]}}
    print(json.dumps({work_page!r}))
    sys.exit(0)
if args[:3] == ['docs', 'work', 'read-part']:
    request = json.loads(Path(args[args.index('--input')+1]).read_text())
    assert request['schema'] == 'codeclew-documentation-source-part-request/1.0'
    assert request['reference'] == {ref!r}
    print(json.dumps(json.loads(Path({str(parts)!r}).read_text())[request.get('cursor', 'first')]))
    sys.exit(0)
if args[:2] == ['docs', 'context'] and args[args.index('--source')+1] == {large['id']!r}:
    print(json.dumps({compact!r}))
    sys.exit(0)
"""
        self.clew.write_text(driver.replace('args=sys.argv[1:]\n', 'args=sys.argv[1:]\n' + native, 1))

        def collect(launcher, root, work, output, **kwargs):
            output.mkdir()
            D['save'](output / 'items.json', [self.native_entry] if kwargs.get('selection') else [scope['coverage']])
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}

        def prepare_sources(job, inventory, folder):
            shutil.copytree(self.manifest.parent, folder / 'read')
            shutil.rmtree(self.manifest.parent)
            fact_reads = folder / self.service / 'fact-evidence'
            selection = fact_reads / 'selection.json'
            D['save'](selection, [{'sourceId': large['id'], 'sourceReference': ref}])
            sub = fact_reads / 'read-sources'
            acquired = D['command']([sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/evidence_packet.py'),
                'read-sources', '--clew', job['clew'], '--root', job['root'], '--service', self.service,
                '--snapshot', job['snapshot'], '--work', scope['work'], '--selection', str(selection),
                '--source-batch-size', '1', '--output-format', 'metadata', '--output-dir', str(sub)], fact_reads, 'prepare-exact')
            self.assertEqual(acquired['status'], 'SELECTED_SOURCES_READY')
            outcome, = D['load'](sub / 'packet.json')['outcomes']
            self.assertEqual(len(outcome['sourceParts']), 2)

        self.review['requirements'] = [row for row in self.review['requirements'] if 'discoveryDisposition' not in row]
        self.seed_files()
        run = self.root / 'compiler-run'
        with patch.dict(D['T'], {'collect': collect}), patch.dict(D['prepare'].__globals__,
                                                                 {'prepare_sources': prepare_sources}):
            self.assertEqual(D['prepare'](self.job_path, run)['status'], 'PREPARED')
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual(D['load'](run / 'report.json')['checks']['provenance']['handles'], 2)
        checks = list((run / 'checks').rglob('source-*-part.command.json'))
        self.assertTrue(checks)
        self.assertEqual(len(checks), len({path.parent for path in checks}))

        first['recordDigest'] = 'sha256:' + '9' * 64
        first['receiptDigest'] = F['digest']({key: value for key, value in first.items() if key != 'receiptDigest'})
        D['save'](parts, {'first': first, 'second': rest})
        result = D['verify'](run)
        self.assertEqual(result['checks']['provenance']['code'], 'PROVENANCE_MISMATCH')

    def test_job_without_token_cap_promotes_and_reports_large_author_usage(self):
        self.assertNotIn('maxModelTokens', D['load'](self.run / 'inputs/job.json')['budget'])
        self.mode.write_text('high-usage')
        result = D['execute'](self.run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](self.run / 'state.json')
        self.assertEqual(state['modelTokens'], 433917)
        self.assertEqual(len(state['attempts']), 1)
        self.assertEqual(state['attempts'][0]['usage'], LARGE_USAGE)
        report = D['load'](self.run / 'report.json')
        self.assertEqual(report['checks']['author']['reportedModelTokens'], 433917)
        self.assertEqual(report['checks']['author']['tokenAccounting']['rawTotals'], LARGE_USAGE)
        self.assertTrue((Path(result['ready']) / 'html' / 'reservations.html').is_file())

    def test_numeric_legacy_token_cap_is_ignored_in_inline_run(self):
        self.job['budget']['maxModelTokens'] = 1
        run, receipt, _, _ = self.prepare_inline_fixture()
        self.assertEqual(receipt['authorMode'], 'INLINE')
        self.assertEqual(D['load'](run / 'inputs/job.json')['budget']['maxModelTokens'], 1)
        self.mode.write_text('high-usage')
        D['save'](self.seed / 'inline-response.json', self.inline_response())
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        self.assertEqual(state['modelTokens'], 433917)
        self.assertEqual(len(state['attempts']), 1)
        self.assertEqual(D['load'](run / 'report.json')['checks']['author']['reportedModelTokens'], 433917)

    def test_null_legacy_token_cap_is_unlimited(self):
        self.job['budget']['maxModelTokens'] = None
        run, receipt, _, _ = self.prepare_inline_fixture()
        self.assertEqual(receipt['status'], 'PREPARED')
        self.assertIsNone(D['load'](run / 'inputs/job.json')['budget']['maxModelTokens'])
        self.mode.write_text('high-usage')
        D['save'](self.seed / 'inline-response.json', self.inline_response())
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual(D['load'](run / 'state.json')['modelTokens'], 433917)

    def test_high_cached_input_uses_uncached_metric_in_author_and_verify(self):
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

    def multi_service_route_draft(self):
        job = D['load'](self.run / 'inputs/job.json')
        job['services'].append('warehouse')
        D['save'](self.run / 'inputs/job.json', job)
        pins = D['load'](self.run / 'pins.json')
        pins.update(jobDigest=D['digest'](job), inputFiles=D['tree'](self.run / 'inputs'))
        D['save'](self.run / 'pins.json', pins)
        self.original['pages'].extend({'page': name + '.md', 'fragments': [
                                      {'text': text + '\n'}, {'id': 'description', 'text': 'Fixture description.\n',
                                                            'evidence': [self.handle]}]}
                                      for name, text in [('warehouse', '# Warehouse\n'),
                                                         ('process-one', '# First process\n\n## First\n\n## Other\n'),
                                                         ('process-two', '# Second process\n\n## Second\n')])
        self.navigation['services'].append({'id': 'warehouse', 'title': 'Warehouse', 'overview': 'warehouse.md',
                                            'pages': ['warehouse.md'], 'starts': [], 'data': []})
        target = {'docset': 'local', 'page': self.entry['page'].removesuffix('.md') + '.html', 'anchor': 'handler'}
        actor = {'kind': 'service', 'docset': 'local', 'service': self.service}
        catalog = {'schema': 'codeclew-cross-docset-routes/1.1', 'revision': 'r1',
                   'catalog': {'docset': 'local', 'page': 'process-one.html'}, 'externalActors': [],
                   'docsets': [{'id': 'local', 'title': 'Local services', 'version': 'r1',
                                'baseURL': 'https://docs.example.invalid/local/', 'services': [
                                    {'id': s['id'], 'title': s['title'], 'overview': {
                                        'docset': 'local', 'page': s['overview'].removesuffix('.md') + '.html'}}
                                    for s in self.navigation['services']]}], 'routes': []}
        for name, anchor in [('process-one', 'first'), ('process-two', 'second')]:
            narrative = {'docset': 'local', 'page': name + '.html', 'anchor': anchor}
            catalog['routes'].append({'id': name, 'version': 'r1', 'title': name, 'narrative': narrative,
                                      'origin': {'status': 'known', 'occurrence': 'request', 'kind': 'http',
                                                 'title': 'Request', 'summary': 'Start the process.'},
                                      'occurrences': [{'id': 'request', 'actor': actor, 'role': 'Accept request',
                                                       'target': dict(target)}], 'branches': [], 'outcomes': [],
                                      'coverage': {'outcomeInventory': 'unknown', 'scope': 'Fixture scope',
                                                   'reference': dict(narrative)}})
        self.seed_files()
        D['save'](self.seed / 'routes.json', catalog)
        return catalog

    def test_multi_service_review_template_reports_route_conflicts_before_review(self):
        import fcntl
        catalog = self.multi_service_route_draft()
        draft = self.run / 'workspace/draft'
        for name in ('original.json', 'navigation.json', 'routes.json'):
            shutil.copy(self.seed / name, draft / name)
        argv = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'),
                'review-template', '--run-dir', str(self.run)]
        with (self.run / '.runner.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            original = D['load'](self.seed / 'routes.json')
            catalog['routes'][1]['narrative'] = {'docset': 'local', 'page': 'process-one.html', 'anchor': 'other'}
            D['save'](draft / 'routes.json', catalog)
            before = D['tree'](self.run)
            result = subprocess.run(argv, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            failure = json.loads(result.stdout)
            self.assertEqual(failure['code'], 'INVALID_ROUTE_DECLARATIONS')
            detail = failure['details']
            self.assertEqual((detail['file'], detail['path'], detail['route'], detail['destination']['anchor']),
                             ('routes.json', '/routes/1/narrative', 'process-two', 'other'))
            self.assertEqual(detail['conflict'], {'route': 'process-one', 'file': 'routes.json', 'path': '/routes/0/narrative',
                                                  'destination': original['routes'][0]['narrative']})
            self.assertEqual(D['tree'](self.run), before)

            original['routes'][0]['occurrences'][0]['target']['anchor'] = 'wrong-anchor'
            original['routes'][0]['origin']['kind'] = 'scheduled'
            D['save'](draft / 'routes.json', original)
            result = subprocess.run(argv, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            detail = json.loads(result.stdout)['details']
            self.assertEqual((detail['path'], detail['occurrence'], detail['kind'], detail['destination']['anchor']),
                             ('/routes/0/origin', 'request', 'scheduled', 'wrong-anchor'))
            self.assertEqual(detail['owner']['service'], self.service)
            self.assertEqual(detail['navigation']['starts'], [{'kind': 'http', 'path': '/services/0/starts/0',
                                                              'target': self.entry['page'] + '#handler'}])

            shutil.copy(self.seed / 'routes.json', draft / 'routes.json')
            self.navigation['services'][0]['starts'][0]['target'] = self.entry['page'] + '#%68andler'
            D['save'](draft / 'navigation.json', self.navigation)
            before = D['tree'](self.run)
            result = subprocess.run(argv, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertEqual(json.loads(result.stdout)['schema'], D['COMPACT_REVIEW'])
            self.assertEqual(D['tree'](self.run), before)

    def test_multi_service_verify_surfaces_navigation_ownership_before_binding_or_render(self):
        self.multi_service_route_draft()
        section = 'process-one.md#other'
        start = self.navigation['services'][0]['starts'][0]['target']
        self.navigation['routes'] = [{'id': 'legacy-process', 'title': 'Legacy route', 'target': section,
                                      'origin': {'service': self.service, 'title': 'Request', 'kind': 'http', 'target': start},
                                      'participants': [{'service': self.service, 'title': 'Request', 'role': 'Start',
                                                        'target': start, 'section': section}]}]
        self.seed_files()
        self.generated()
        result = self.check()
        detail = result['checks']['output']['details']
        self.assertEqual(result['checks']['output']['code'], 'INVALID_ROUTE_DECLARATIONS')
        self.assertEqual((detail['route'], detail['path'], detail['conflict']['target'], detail['conflict']['route']),
                         ('process-one', '/routes/0/narrative', section, 'legacy-process'))
        self.assertEqual((detail['conflict']['file'], detail['conflict']['path']), ('navigation.json', '/routes/0/target'))
        self.assertEqual(result['checks']['provenance']['status'], 'NOT_RUN')
        self.assertEqual(result['checks']['finalize']['status'], 'NOT_RUN')

    def test_whole_document_initial_author_and_same_thread_repair_remain_the_default(self):
        self.mode.write_text('repair-on-resume')
        self.original['pages'][0]['fragments'][1]['text']+='\n[Missing](absent.html)\n'
        self.seed_files()
        job=D['load'](self.run/'inputs/job.json');job['budget']['maxRepairs']=1
        D['save'](self.run/'inputs/job.json',job)
        pins=D['load'](self.run/'pins.json');pins['jobDigest']=D['digest'](job);pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        result=D['execute'](self.run);self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json');self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['modelTokens'], 867834)
        self.assertEqual([a['threadId'] for a in state['attempts']], ['owned-test-session'] * 2)
        self.assertEqual([a['budgetTokens'] for a in state['tokenAccounting']['attemptUsage']], [433917, 433917])
        self.assertEqual([a['usage'] for a in state['attempts']],
                         [LARGE_USAGE, {key: value * 2 for key, value in LARGE_USAGE.items()}])
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

    def test_recorded_v2_usage_keeps_exact_uncached_accounting(self):
        usage = {'input_tokens':5089868, 'cached_input_tokens':4811904,
                 'output_tokens':167582, 'reasoning_output_tokens':96885, 'cache_write_input_tokens':0}
        result = D['token_accounting']([{'usage':usage}])
        self.assertEqual(result['status'], 'MEASURED')
        self.assertEqual(result['budgetTokens'], 445546)
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
        self.assertEqual(result['knownBudgetTokensLowerBound'], 35)

    def test_missing_or_invalid_cache_blocks_ready_and_another_author(self):
        for mode, code, status in (
            ('high-usage-missing-cache', 'AUTHOR_USAGE_UNAVAILABLE', 'BLOCKED'),
            ('high-usage-invalid-cache', 'AUTHOR_USAGE_INVALID', 'FAILED'),
        ):
            with self.subTest(mode=mode):
                self.mode.write_text(mode)
                with self.assertRaises(D['Stop']) as raised:
                    self.generated_for_test_reset()
                self.assertEqual(raised.exception.code, code)
                state_path = self.run / 'state.json'
                state = D['load'](state_path)
                self.assertFalse(state['usageAvailable'])
                self.assertIsNone(state['modelTokens'])
                self.assertEqual(len(state['attempts']), 1)
                if mode == 'high-usage-missing-cache':
                    self.assertEqual(state['tokenAccounting']['knownBudgetTokensLowerBound'], 433917)
                failure = self.check()
                self.assertEqual(failure['status'], status)
                self.assertEqual(failure['checks']['author']['code'], code)
                with self.assertRaises(D['Stop']) as raised:
                    D['execute'](self.run)
                self.assertEqual(raised.exception.code, code)
                self.assertEqual(D['load'](state_path), state)
                self.assertFalse((self.run / 'attempts/1').exists())
                self.assertFalse((self.run / 'ready').exists())

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

    def test_broken_link_still_refuses_readiness_after_high_usage(self):
        self.original['pages'][0]['fragments'][1]['text'] += '\n[Missing](absent.html#lost)\n'
        self.mode.write_text('high-usage')
        self.seed_files()
        result = D['execute'](self.run)
        self.assertEqual(result['status'], 'FAILED')
        self.assertEqual(result['checks']['author']['reportedModelTokens'], 433917)
        self.assertEqual(result['checks']['finalize']['status'], 'FAIL')
        self.assertFalse((self.run / 'ready').exists())

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

    def test_transient_author_failure_can_resume_with_valid_usage_and_an_allowed_repair(self):
        job = D['load'](self.run / 'inputs/job.json')
        job['budget'].update(maxRepairs=1, maxModelTokens=1000)
        D['save'](self.run / 'inputs/job.json', job)
        pins = D['load'](self.run / 'pins.json')
        pins.update(jobDigest=D['digest'](job), inputFiles=D['tree'](self.run / 'inputs'))
        D['save'](self.run / 'pins.json', pins)
        self.mode.write_text('exit-failed')
        with self.assertRaises(D['Stop']) as raised:
            self.generated()
        self.assertEqual(raised.exception.code, 'AUTHOR_NOT_COMPLETED')
        initial = D['load'](self.run / 'state.json')
        self.assertEqual(initial['status'], 'AUTHOR_FAILED')
        self.assertEqual(self.check()['checks']['author']['code'], 'AUTHOR_COMPLETION_REQUIRED')
        with self.assertRaises(D['Stop']) as raised:
            D['promote'](self.run)
        self.assertEqual(raised.exception.code, 'RUN_NOT_READY')

        self.mode.write_text('normal')
        result = D['execute'](self.run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](self.run / 'state.json')
        first, retry = state['attempts']
        self.assertEqual(first['exitCode'], 23)
        self.assertEqual(retry['threadId'], first['threadId'])
        self.assertEqual(retry['argv'][retry['argv'].index('resume') + 1], first['threadId'])
        self.assertEqual(state['modelTokens'], 40)
        self.assertEqual([row['budgetTokens'] for row in state['tokenAccounting']['attemptUsage']], [20, 20])

    def test_legacy_token_stopped_attempt_is_neither_resumed_nor_accepted(self):
        job = D['load'](self.run / 'inputs/job.json')
        job['budget'].update(maxRepairs=1, maxModelTokens=250000)
        D['save'](self.run / 'inputs/job.json', job)
        pins = D['load'](self.run / 'pins.json')
        pins.update(jobDigest=D['digest'](job), inputFiles=D['tree'](self.run / 'inputs'))
        D['save'](self.run / 'pins.json', pins)
        self.mode.write_text('high-usage')
        state = self.generated()
        state['status'] = 'AUTHOR_BUDGET_EXCEEDED'
        D['save'](self.run / 'state.json', state)
        before = D['tree'](self.run / 'attempts')
        verified = self.check()
        self.assertEqual(verified['checks']['author']['code'], 'AUTHOR_COMPLETION_REQUIRED')
        with self.assertRaises(D['Stop']) as raised:
            D['promote'](self.run)
        self.assertEqual(raised.exception.code, 'RUN_NOT_READY')
        with self.assertRaises(D['Stop']) as raised:
            D['execute'](self.run)
        self.assertEqual(raised.exception.code, 'AUTHOR_NOT_COMPLETED')
        self.assertEqual(raised.exception.details['previousStatus'], 'AUTHOR_BUDGET_EXCEEDED')
        self.assertEqual(D['tree'](self.run / 'attempts'), before)
        self.assertEqual(D['load'](self.run / 'state.json'), state)
        self.assertFalse((self.run / 'ready').exists())

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

    def test_literal_guard_accepts_only_an_exact_covering_occurrence(self):
        method = 'fun guard(delta: Int): Int {\n    if (delta < -9) return 23\n    return delta\n}'
        prefix = 'class Charges {\n    // π🙂\n    '
        body = prefix + method + '\n}\n'
        raw = body.encode()
        start, end = len(prefix.encode()), len((prefix + method).encode())
        snapshot = 'sha256:owned-captured-file'
        blob = hashlib.sha1(('blob ' + str(len(raw)) + '\0').encode() + raw).hexdigest()

        def evidence(identity, text, a, b):
            return {'readText': text, 'source': {'id': identity, 'file': 'src/Charges.kt', 'service': self.service,
                'revision': self.source['revision'], 'startLine': 3 if a else 1,
                'occurrence': {'snapshot': snapshot, 'blob': blob, 'startByte': a, 'endByte': b}}}

        exact = evidence('method', method, start, end)
        covering = evidence('class', body, 0, len(raw))
        candidates = D['literal_return_candidates'](self.entry, ['exact'], {'exact': exact})
        self.assertEqual(len(candidates), 1)
        self.assertEqual(D['kotlin_literal_guards'](covering), [])
        check = candidates[0]['id']
        page = self.entry['page']
        claim = {'id': 'guard', 'text': 'When `delta < -9`, the result is `23`.\n', 'evidence': ['provided']}
        original = {'pages': [{'page': page, 'fragments': [claim]}]}
        _, composed, _ = D['M']['compose_pages'](original)
        review = {'claims': [{'page': page, 'claim': claim['id'], 'disposition': 'SUPPORTED'}]}

        def validate(reference):
            return D['literal_return_claims'](candidates, {check: claim['id']}, review, original, composed,
                                              {'exact': exact, 'provided': reference})

        self.assertEqual(validate(covering)[0]['literal'], '23')
        self.assertEqual(claim['evidence'], ['provided'])
        partial = evidence('partial', raw[start + 1:end].decode(), start + 1, end)
        conflicting = evidence('conflict', body.replace('return 23', 'return 93'), 0, len(raw))
        malformed = evidence('malformed', body, 0, len(raw) + 1)
        wrong = {'partial': partial, 'conflicting overlap': conflicting, 'bad byte width': malformed}
        for field in ('service', 'file', 'revision', 'occurrence'):
            wrong[field] = {**covering, 'source': {**covering['source'], field: None if field == 'occurrence' else 'foreign'}}
        for field in ('snapshot', 'blob'):
            occurrence = {**covering['source']['occurrence'], field: '1' * 40}
            wrong[field] = {**covering, 'source': {**covering['source'], 'occurrence': occurrence}}
        for case, reference in wrong.items():
            with self.subTest(case=case), self.assertRaises(D['Stop']) as raised:
                validate(reference)
            self.assertEqual(raised.exception.code, 'LITERAL_RETURN_CITATION_REQUIRED')

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

    def test_http_first_call_checks_exact_request_before_optional_details(self):
        page = 'place-entry.md'
        target = page + '#handler'
        requirement = {'id': 'create-place', 'kind': 'ENTRY', 'page': page,
                       'handlerId': 'handler', 'method': 'POST', 'path': '/places'}
        step = {'from': 'client', 'to': 'handler', 'kind': 'call', 'target': target}
        diagram = {'title': 'Create a place', 'participants': [
            {'id': 'client', 'title': 'Client'},
            {'id': 'handler', 'title': 'Handler', 'target': target}],
            'steps': [step]}
        cases = [
            ('POST /places', 'call', 'client', True),
            ('POST /places with the city ID', 'call', 'client', True),
            ('GET /places with the city ID', 'call', 'client', False),
            ('POST /places-old with the city ID', 'call', 'client', False),
            ('POST /places/extra with the city ID', 'call', 'client', False),
            ('POST /places?city=one', 'call', 'client', False),
            ('Send POST /places', 'call', 'client', False),
            ('POST /places with the city ID', 'return', 'client', False),
            ('POST /places with the city ID', 'call', 'handler', False),
        ]
        request = self.root / 'sequence-requests.json'
        for label, kind, sender, accepted in cases:
            with self.subTest(label=label, kind=kind, sender=sender):
                step.update(label=label, kind=kind, **{'from': sender})
                D['save'](request, {'requirements': [requirement],
                    'accounts': [{'requirement': requirement['id'], 'diagram': 0}],
                    'pages': {page: '```clew-sequence\n' + json.dumps(diagram) + '\n```\n'}})
                result = subprocess.run([self.job['node'], str(ROOT / 'skills/codeclew/scripts/docs_run_sequences.mjs'),
                                         str(RENDERER), str(request)], capture_output=True, text=True)
                reply = json.loads(result.stdout)
                self.assertEqual(result.returncode, 0 if accepted else 2, reply)
                if accepted:
                    self.assertEqual(reply['status'], 'ENTRY_SEQUENCES_CHECKED')
                else:
                    self.assertEqual(reply['code'], 'SEQUENCE_HTTP_ENTRY_MISMATCH')
                    self.assertEqual(reply['details']['expectedRequest'], 'POST /places')

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


class PreparedKotlinSourceTests(unittest.TestCase):
    def test_installed_source_prepare_qualifies_local_helpers_and_focused_writes(self):
        launcher = os.environ.get('CODECLEW_TEST_INSTALLED_SOURCE_LAUNCHER')
        if not launcher:
            self.skipTest('An installed release launcher was not configured.')
        contents = '''package demo
import java.io.ByteArrayInputStream
import java.util.zip.GZIPInputStream
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.kafka.annotation.KafkaListener
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.transaction.support.TransactionTemplate

class Intake(private val transaction: TransactionTemplate, private val db: JdbcTemplate, private val parser: Parser) {
    @KafkaListener(topics = ["parcel.received"])
    fun receive(packet: Packet) {
        val decoded = try {
            parser.parse(packet.value()).also { validate(it) }
        } catch (bad: Exception) {
            return
        }
        transaction.executeWithoutResult {
            for (item in decoded.items) {
                if (!item.enabled) continue
                persist(item)
            }
        }
    }

    @KafkaListener(topics = ["parcel.shadowed"])
    fun shadowed(packet: Packet) {
        val validate = parser.validator
        parser.parse(packet.value()).also { validate(it) }
    }

    @KafkaListener(topics = ["parcel.ambiguous"])
    fun ambiguous(packet: Packet) {
        evaluate(packet.value())
    }

    @Scheduled(fixedDelay = 500)
    fun poll() {
        for (item in parser.pending()) {
            refresh(item)
        }
    }

    @Scheduled(fixedDelay = 800)
    fun checkpoint() {
        gather("daily", 20)
    }

    private fun gather(prefix: String, size: Int) {
        kotlin.require(prefix.isNotEmpty())
        kotlin.check(size > 0)
        while (true) {
            val outcome = try { parser.next(prefix, size) } catch (busy: Retry) { continue }
            if (outcome != ScanResult.Again) break
            parser.collect()
            parser.remember()
        }
        parser.finish(prefix)
    }

    @Scheduled(fixedDelay = 900)
    fun recount() {
        var collected = 0
        var outcome: ScanResult
        do {
            outcome = parser.next()
            if (false) throw IllegalStateException()
            collected += outcome.size
        } while (outcome.status == ScanResult.Again)
        parser.finish(collected)
    }

    private fun validate(value: Message) {
        require(value.kind == "selected") { "selected kind required" }
    }

    private fun evaluate(value: String) {
        require(value != "ambiguous string marker")
    }

    private fun evaluate(value: Int) {
        require(value != 98765)
    }

    private fun refresh(item: Item) {
        if (item.cached) return
        persist(item)
    }

    private fun persist(item: Item) {
        if (!item.enabled) return
        val text = GZIPInputStream(ByteArrayInputStream(item.bytes)).reader().readText()
        val message = parser.parse(text)
        if (message.kind != "parcel") return
        val first = message.items.first()
        val id = first.external
        db.update("INSERT INTO parcel (id) VALUES (?)", id)
        db.update("INSERT INTO incoming_record (id) VALUES (?)", id)
        db.update("INSERT INTO visible_record (id) VALUES (?)", id)
        db.update("INSERT INTO revisions (id) VALUES (?)", id)
        if (message.live) {
            db.update("INSERT INTO relay_queue (id) VALUES (?)", id)
        } else {
            db.update("INSERT INTO relay_queue (id) VALUES (?)", "cached")
        }
    }
}

class Unrelated {
    private fun validate(value: Message) {
        require(value.kind == "unrelated owner marker")
    }
}
'''
        with tempfile.TemporaryDirectory(prefix='clew-source-helper-test-') as tmp:
            directory = Path(tmp).resolve()
            repo, docs = directory / 'source', directory / 'docs'
            repo.mkdir()
            (repo / 'Intake.kt').write_text(contents)
            backend = directory / 'unavailable-author'
            backend.write_text('#!/bin/sh\nexit 81\n')
            backend.chmod(0o700)

            def git(*args):
                subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True)
            git('init', '-q')
            git('remote', 'add', 'origin', 'https://example.invalid/intake')
            git('add', '.')
            git('-c', 'user.name=Owned Test', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'Fixture')

            def clew(*args):
                result = subprocess.run([launcher, 'docs', *args, '--root', str(docs)],
                                        capture_output=True, text=True, timeout=30)
                self.assertIn(result.returncode, (0, 3) if args[0] == "check" else (0,), result.stdout + result.stderr)
                return json.loads(result.stdout)

            clew('init', '--title', 'Intake test')
            baseline = clew('service', 'list')
            record = directory / 'service.json'
            D['save'](record, {'schema': 'codeclew-documentation-service/1.0', 'id': 'intake',
                                'title': 'Intake', 'repositoryId': 'intake', 'repository': 'https://example.invalid/intake',
                                'language': 'kotlin', 'profile': 'source-syntax', 'targetRef': 'HEAD',
                                'source': {'roots': ['.'], 'dialect': '1.9'}})
            clew('service', 'add', '--input', str(record), '--expected-input-digest', baseline['inputDigest'])
            clew('bind', '--service', 'intake', '--repo', str(repo))
            check = clew('check')
            self.assertEqual(check['status'], 'CHECKED')
            self.assertEqual(check['services']['intake']['coverage'], 'SYNTAX')
            job = directory / 'job.json'
            D['save'](job, {'schema': D['SCHEMA'], 'request': 'Document the retained intake operations.',
                           'clew': str(Path(launcher).absolute()), 'root': str(docs), 'snapshot': check['snapshot'],
                           'services': ['intake'], 'language': 'en', 'renderer': str(RENDERER),
                           'node': str(Path(shutil.which('node')).absolute()), 'evidenceMode': 'PREPARED_FACTS',
                           'authorMode': 'ISOLATED_PAGES', 'author': {'executable': str(backend),
                              'model': 'unused-test-author', 'reasoningEffort': 'high'},
                           'budget': {'wallSeconds': 30, 'maxRepairs': 0}})
            run = directory / 'run'
            records = {}
            load_script = runpy.run_path

            def inspect_record(path, **kwargs):
                script = load_script(path, **kwargs)
                if Path(path).name != 'clew_service_edges.py':
                    return script

                def record(requirement, process, context, facts, exact):
                    value = script['record'](requirement, process, context, facts, exact)
                    records[requirement['entrySymbol']] = value
                    return value

                return {**script, 'record': record}

            with patch.object(runpy, 'run_path', inspect_record):
                result = D['prepare'](job, run)
            self.assertEqual(result['authorMode'], 'ISOLATED_PAGES')
            plan = D['load'](run / 'inputs/author-plan.json')
            packets = [json.loads((run / 'inputs' / row['prompt']).read_text().split('\n\n')[-1])
                       for row in plan['slots'] if row['kind'] == 'OPERATION']
            by_entry = {item['requirement']['entrySymbol']: item for item in packets}

            def packet_for(member):
                found = [item for symbol, item in by_entry.items() if member in symbol]
                self.assertEqual(len(found), 1, list(by_entry))
                return found[0]

            received = D['P']['expand_fact_context'](packet_for('receive'))
            output = json.dumps(received)
            self.assertIn('selected kind required', output)
            self.assertIn('suffix selects Kotlin', output)
            self.assertNotIn('unrelated owner marker', output)
            self.assertIn('write_commit', output)
            self.assertIn('write_guard', output)
            facts = [fact for scope in received['facts'].values() for fact in scope.get('facts', ())]
            lineage = [fact for fact in facts if fact['kind'] == 'write_lineage']
            self.assertTrue(any('parser.parse' in fact['text'] and 'GZIPInputStream' in fact['text']
                                and 'message.kind' in fact.get('condition', '') for fact in lineage),
                            [(fact['kind'], fact.get('condition')) for fact in facts])
            self.assertIn('INSERT INTO parcel', output)
            self.assertIn('INSERT INTO relay_queue', output)
            entry = received['requirement']['entrySymbol']
            writes = [write for write in records[entry]['writes'] if write['table'] == 'relay_queue']
            self.assertGreaterEqual(len(writes), 2, writes)
            polarities = set()
            for write in writes:
                rows = [row for row in write['focus'].get(None, ()) if row['kind'] == 'write_request']
                self.assertTrue(rows, write)
                requests = [row for row in facts if row['kind'] == 'write_request'
                            and any(row['text'] == request['text'] and row['condition'] == request['condition'] for request in rows)]
                self.assertTrue(requests, rows)
                signs = {row['condition'].endswith('`message.live`') for row in rows}
                self.assertEqual(len(signs), 1, rows)
                self.assertTrue(all(row['condition'].endswith(('`message.live`', '`message.live` is false')) for row in rows), rows)
                polarities.update(signs)
                self.assertTrue(any('relay_queue' in received['sourceRanges'][region]['text']
                                    for row in requests for alias in row['sources']
                                    for region in received['citations'][alias].get('regions', [received['citations'][alias]['region']])))
            self.assertEqual(polarities, {False, True})
            shadow = packet_for('shadowed')['requirement']['entrySymbol']
            self.assertFalse(records[shadow]['writes'])
            self.assertNotIn('selected kind required', json.dumps(packet_for('shadowed')))
            self.assertNotIn('ambiguous string marker', json.dumps(packet_for('ambiguous')))
            self.assertNotIn('98765', json.dumps(packet_for('ambiguous')))
            poll = packet_for('poll')
            self.assertIn('item.cached', json.dumps(poll))
            self.assertIn('INSERT INTO parcel', json.dumps(poll))
            self.assertEqual(received['omittedPreparedFacts'], 0)
            checkpoint = D['P']['expand_fact_context'](packet_for('checkpoint'))
            facts = [fact for scope in checkpoint['facts'].values() for fact in scope.get('facts', ())]
            relation, = (fact for fact in facts if fact['kind'] == 'RESULT_CONTROLLED_LOOP')
            self.assertIn('If `outcome != ScanResult.Again` is true, `break` exits', relation['text'])
            self.assertIn('The normal after-loop continuation reaches `parser.finish(prefix)`', relation['text'])
            guards = {fact['condition'] for fact in facts if fact['kind'] == 'LOOP_ENTRY_VALIDATION'}
            self.assertEqual(guards, {'`prefix.isNotEmpty()` is true', '`size > 0` is true'})
            recovery, = (fact for fact in facts if fact['kind'] == 'LOOP_FAILURE_PATH')
            self.assertIn('`continue` skips the remaining body', recovery['text'])
            scoped_regions = [checkpoint['sourceRanges'][region]['text'] for alias in relation['sources']
                              for region in checkpoint['citations'][alias].get('regions', [checkpoint['citations'][alias]['region']])]
            for source in ('parser.next(prefix, size)', 'outcome != ScanResult.Again', 'parser.collect()',
                           'parser.remember()', 'parser.finish(prefix)'):
                self.assertTrue(any(source in text for text in scoped_regions), scoped_regions)

            recount = D['P']['expand_fact_context'](packet_for('recount'))
            facts = [fact for scope in recount['facts'].values() for fact in scope.get('facts', ())]
            loop, = (fact for fact in facts if fact['kind'] == 'RESULT_CONTROLLED_LOOP')
            self.assertIn('outcome.status == ScanResult.Again', loop['text'])
            self.assertIn('exits normally', loop['text'])
            self.assertTrue({'LOOP_RESULT_SOURCE', 'LOOP_LOCAL_ACCUMULATION'} <= {fact['kind'] for fact in facts})
            self.assertFalse({'LOOP_ENTRY_VALIDATION', 'LOOP_FAILURE_PATH'} & {fact['kind'] for fact in facts})
            sidecar = D['load'](run / 'inputs/author-source/intake/prepared-facts.json')
            raw = contents.encode()
            for packet, literals in ((checkpoint, ('prefix.isNotEmpty()', 'size > 0', 'outcome != ScanResult.Again', 'continue',
                                                   'parser.next(prefix, size)', 'parser.collect()', 'parser.remember()', 'parser.finish(prefix)')),
                                     (recount, ('var collected = 0', 'parser.next()', 'collected += outcome.size',
                                                'outcome.status == ScanResult.Again', 'parser.finish(collected)'))):
                checked = sidecar['entries'][packet['requirement']['id']]['propositions']
                anchors = {(proof['startByte'], proof['endByte'], proof['textDigest'])
                           for fact in checked if fact['kind'] == 'RESULT_CONTROLLED_LOOP' or fact['kind'].startswith('LOOP_')
                           for proof in fact['witnesses']}
                for literal in literals:
                    encoded = literal.encode()
                    self.assertTrue(any(stop - start == len(encoded) and raw[start:stop] == encoded
                                        and digest == D['sha'](encoded) for start, stop, digest in anchors), literal)
                self.assertEqual(packet['omittedPreparedFacts'], 0)


class PreparedServiceEdgeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        scripts = ROOT / "skills/codeclew/scripts"
        cls.edges = runpy.run_path(str(scripts / "prepared_facts/clew_service_edges.py"))
        cls.prepared = runpy.run_path(str(scripts / "docs_run_prepared.py"), init_globals={
            name: D[name] for name in ("digest", "encode", "sha", "save")})

    def retained(self, service, *, name=None, verb="GET", path="/units", entry_kind="HTTP_ENDPOINT", trigger=None, proof=True,
                 events=(), facts=(), write_sites=()):
        identity = service + ":" + (name or "entry")
        name = name or service
        registration = name + "-registration"
        body = name + "-body"
        exact = {}

        def source(handle, text, start):
            offset = sum(len(row["text"].encode()) + 1 for row in exact.values())
            exact[handle] = {"id": handle, "file": "Operations.java", "service": service, "revision": D["sha"](service.encode()),
                             "startByte": offset, "endByte": offset + len(text.encode()),
                             "startLine": start, "endLine": start, "text": text, "textDigest": D["sha"](text.encode()),
                             "authority": "EXACT_SNAPSHOT_TEXT", "references": [handle]}

        source(body, "Object handle() { return value; }", 2)
        if proof:
            source(registration, "@Mapping", 1)
        requirement = {"id": identity, "kind": "ENTRY", "service": service, "entrypoint": "entry", "entryKind": entry_kind,
                       "rootDeclarations": ["root"], "method": verb if entry_kind == "HTTP_ENDPOINT" else None,
                       "path": path, "page": name + ".md", "trigger": trigger,
                       "sourceReferences": [registration, body], "registrationSourceIds": [registration]}

        def obligation(kind, cite, event=None):
            record = {"declaration": "root", "sourceIds": [cite]}
            if event is not None:
                record["event"] = event
            return {"obligationKind": kind, "record": record, "id": kind + ":" + cite, "sourceReferences": [cite]}

        rows = [obligation("BODY", body)]
        for number, (label, text, event) in enumerate(events, 3):
            cite = name + "-" + label
            source(cite, text, number)
            rows.append(obligation("EVENT", cite, event))
        process, context = self.prepared["operation"](requirement, rows, exact, {})
        lifted = {site: exact[name + "-" + site] for site in write_sites}
        accepted, rejected = self.prepared["bound_facts"](facts, lifted, exact, set(exact))
        self.assertEqual(rejected, 0)
        row = self.edges["record"](requirement, process, context, accepted, exact)
        return requirement, row, exact

    def project(self, *retained, isolated=False):
        return self.edges["project"]([row[0] for row in retained], {row[0]["id"]: row[1] for row in retained},
                                     isolated=isolated)

    def test_kotlin_replays_direct_and_reached_declaration_write_aliases(self):
        text = """import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.transaction.support.TransactionTemplate

class Inventory(private val tx: TransactionTemplate, private val db: JdbcTemplate) {
    @Scheduled(fixedDelay = 300)
    fun apply(entries: List<Parcel>) {
        tx.executeWithoutResult {
            for (entry in entries) {
                if (entry.ready) persist(entry)
            }
        }
    }

    private fun persist(entry: Parcel) {
        kotlin.require(entry.valid)
        if (entry.changed) {
            db.update("INSERT INTO delivery (id) VALUES (?)", entry.id)
        } else {
            db.update("INSERT INTO delivery (id) VALUES (?)", "unchanged")
        }
    }
}
"""
        file, service, revision = 'Inventory.kt', 'inventory', 'a' * 40
        raw = text.encode()
        blob = 'b' * 40

        def source(handle, literal):
            first = raw.index(literal.encode())
            end = first + len(literal.encode())
            line = 1 + raw[:first].count(b'\n')
            return {'id': handle, 'service': service, 'file': file, 'revision': revision,
                    'text': literal, 'textDigest': D['sha'](literal.encode()), 'authority': 'EXACT_SNAPSHOT_TEXT',
                    'startByte': first, 'endByte': end, 'startLine': line,
                    'endLine': line + literal.removesuffix('\n').count('\n'), 'references': [handle],
                    'occurrence': {'snapshot': F['SNAPSHOT'], 'blob': blob, 'startByte': first, 'endByte': end}}

        declaration = text[text.index('@Scheduled'):text.index('\n\n    private fun persist')]
        exact = {'unit': source('unit', text), 'root': source('root', declaration)}
        captured = {file: exact['unit']}
        binder = lambda witness: self.prepared['extra_binding'](witness, exact)
        project = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/prepared_facts/managed_languages.py'))['project']

        def derive(requirement, rows):
            process, context = self.prepared['operation'](requirement, rows, exact, captured)
            context.update(_sourceCallSelections=[], _deferredWriteFocus=[], _bindWitness=binder)
            propositions, narrow = project(process, context, node=None)
            local = {site['id'] for site in context['sourceRanges']}
            facts, unbound = self.prepared['bound_facts'](propositions, narrow, exact, local)
            self.assertEqual(unbound, 0)
            return process, context, facts

        base = {'service': service, 'page': 'inventory.md'}
        discovery = {'id': 'discovery', 'kind': 'DISCOVERY', 'file': file, **base}
        requirement = {'id': 'entry', 'entrySymbol': 'apply', 'kind': 'ENTRY', 'entryKind': 'SCHEDULED_JOB',
                       'rootDeclarations': ['apply'], 'sourceReferences': ['root'], **base}
        root = {'id': 'BODY:apply', 'obligationKind': 'BODY', 'record': {'declaration': 'apply', 'sourceIds': ['root']},
                'sourceReferences': ['root']}
        _, _, discovery_facts = derive(discovery, ())
        process, context, local = derive(requirement, [root])
        reached = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/docs_run_reached.py'))
        index = reached['index_discovery']([discovery_facts], exact)
        joined, limited = reached['select'](process, index, exact, binder, context['_sourceCallSelections'])
        self.assertEqual(limited, 0)
        local_writes = [fact for fact in local if fact.get('_edge', {}).get('kind') == 'INSERT']
        reached_writes = [fact for fact in joined if fact.get('_edge', {}).get('kind') == 'INSERT']
        self.assertEqual(len(local_writes), 2)
        self.assertEqual(len(reached_writes), 2)
        self.assertTrue(all(fact['_reachedDeclaration'] for fact in reached_writes))
        for pair in zip(local_writes, reached_writes):
            self.assertEqual(len({fact['_writeSite']['startByte'] for fact in pair}), 1)
            self.assertEqual(len({fact['_edge']['occurrence'] for fact in pair}), 2)
        composed = [*local, *joined]
        focused, rejected = self.prepared['replay_reached_writes'](context, composed, exact, None)
        self.assertEqual(rejected, 0)
        result = self.edges['record'](requirement, process, context, [*composed, *focused], exact)
        guarded = {}
        for write in result['writes']:
            requests = [fact for fact in write['focus'].get(None, ()) if fact['kind'] == 'write_request']
            self.assertTrue(requests, write)
            exits = [fact for fact in write['focus'].get(None, ()) if fact['kind'] == 'write_skip'
                     and 'guard throws' in fact['text']]
            self.assertTrue(exits, write)
            self.assertTrue(all('entry.ready' in fact['condition'] and '`entry.valid` is false' in fact['condition']
                                for fact in exits), exits)
            self.assertEqual({fact['_writeSite']['startByte'] for fact in exits},
                             {fact['_writeSite']['startByte'] for fact in requests})
            for request in requests:
                # The same physical call may have two retained aliases, with identical predicates.
                guarded.setdefault(request['_writeSite']['startByte'], set()).add(request['condition'])
        self.assertEqual(len(guarded), 2)
        self.assertEqual(sorted(map(len, guarded.values())), [1, 1])
        self.assertTrue(all('entry.ready' in condition for conditions in guarded.values() for condition in conditions))
        self.assertEqual({condition.endswith('`entry.changed` is false')
                          for conditions in guarded.values() for condition in conditions}, {False, True})

    def test_typed_java_publication_requires_wired_matching_protocol_channel_and_class(self):
        config = {"adapter": "EVENTUATE_TRAM_DOMAIN_EVENT", "channel": "library.Reservation",
                  "eventType": "library.events.Placed"}
        qualified = {"kind": "java-typed-publication", "authority": "COMPILER_SELECTED_JAVA_PUBLICATION",
                     "text": "The publisher submits the retained typed event.", "sources": ["origin-body"],
                     "_edge": {"kind": "JAVA_TYPED_PUBLISH", "site": "origin-body", **config}}

        assembly = ["assembly", *(f"alias-{index}" for index in range(10))]

        def listener(name, configuration, *, registered=True, declaration=True, callback=True, complete=True):
            references = [f"{name}-{piece}" for piece in assembly]
            return self.retained(name, entry_kind="JAVA_TYPED_DISPATCH", proof=registered,
                                 trigger={"kind": "JAVA_TYPED_DISPATCH",
                                          "authority": "RETAINED_JAVA_DISPATCHER_WIRING" if declaration else "INFERRED",
                                          "configuration": configuration, "registration": name + "-registration",
                                          "callback": name + ("-body" if callback else "-absent"),
                                          "assemblySources": references if complete else [*references, name + "-missing"]},
                                 events=[(piece, f'final var {piece.replace("-", "_")} = builder;', {"kind": "CALL"})
                                         for piece in assembly])

        owner = self.retained("orders", name="origin", facts=[qualified])
        eligible = listener("ledger", config)
        peers = [listener("different-package", {**config, "eventType": "other.events.Placed"}),
                 listener("different-channel", {**config, "channel": "library.Archive"}),
                 listener("untyped", {**config, "eventType": "Placed"}),
                 listener("wrong-adapter", {**config, "adapter": "KAFKA"}),
                 listener("unregistered", config, registered=False),
                 listener("missing-callback-source", config, callback=False),
                 listener("missing-assembly-source", config, complete=False),
                 listener("compatible-function", config, declaration=False)]
        result = self.project(owner, eligible, *peers)
        row, = result[owner[0]["id"]]
        self.assertEqual(row["kind"], "typed-message-peer")
        self.assertIn("compiler selected", row["text"])
        self.assertIn("adapter event mapping", row["uncertainty"])
        self.assertEqual(row["sources"], ["origin-body", "ledger-registration", *(f"ledger-{piece}" for piece in assembly), "ledger-body"])
        self.assertTrue(all(peer[0]["id"] not in result for peer in peers))
        nominal = self.retained("orders", name="origin",
                                facts=[{**qualified, "authority": "RETAINED_JAVA_PUBLICATION_CANDIDATE"}])
        local, = self.project(nominal, eligible)[nominal[0]["id"]]
        self.assertIn("receiver and operands corroborate", local["text"])
        open_call = {**qualified, "_edge": {**qualified["_edge"], "channel": None}}
        owner = self.retained("orders", name="origin", facts=[open_call])
        row, = self.project(owner, eligible)[owner[0]["id"]]
        self.assertEqual((row["kind"], row["status"]), ("typed-message-peer-boundary", "UNKNOWN"))

    def test_http_peer_return_and_failure_are_source_qualified_in_the_normal_author_view(self):
        call = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units/42"}}
        effect = {"kind": "error-path", "authority": "RETAINED_PATH", "sources": ["front-call", "front-body"],
                  "text": "After the earlier write request returns, the call can throw and enter the local catch."}
        front = self.retained("front", path="/start", events=[("call", "client.get()", call)], facts=[effect])
        depot = self.retained("depot", path="/units/{key}",
                              events=[("return", "return available;", {"kind": "RETURN"}),
                                      ("throw", "throw missing;", {"kind": "THROW"})])
        projected = self.project(front, depot)
        outcomes = {row["kind"] for row in projected[front[0]["id"]]}
        self.assertEqual(outcomes, {"http-peer", "http-peer-outcome", "http-caller-continuation"})
        peer_outcomes = [row for row in projected[front[0]["id"]] if row["kind"] == "http-peer-outcome"]
        self.assertTrue(any("return available" in row["text"] for row in peer_outcomes))
        self.assertTrue(any("throw missing" in row["text"] for row in peer_outcomes))
        self.assertTrue(all("wire mapping remain unknown" in row["uncertainty"] for row in peer_outcomes))
        self.assertTrue(any("rollback of any earlier effects remain unknown" in row["uncertainty"]
                            for row in projected[front[0]["id"]]))
        self.assertIn("front-call", projected[depot[0]["id"]][0]["sources"])
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory) / "front"
            (home / "views").mkdir(parents=True)
            path = home / "views/source.md"
            path.write_text("Source header\n\n```java\nsource body\n```\n")
            citations = {handle: {**row, "manifest": Path(directory) / row["service"] / "read/citations.json"}
                         for _, _, exact in (front, depot) for handle, row in exact.items()}
            selected = {"entries": {front[0]["id"]: {"status": "NO_CITABLE_FACTS"}}, "citations": {}, "bytes": 0}
            self.prepared["merge_edges"](selected, projected, citations, home)
            supported = selected["entries"][front[0]["id"]]
            self.assertEqual(supported["status"], "PREPARED")
            self.assertEqual(selected["citations"]["depot-return"]["manifest"], "../depot/read/citations.json")
            rows = self.prepared["author_entries"](selected["entries"], selected["citations"])
            self.prepared["workspace_views"](home, {front[0]["id"]: ("views/source.md", "Page front.md")},
                                             rows, selected["citations"], {"views/source.md": "Operations.java"}, 8192)
            text = path.read_text()
            self.assertIn("[depot@", text)
            self.assertIn("Cite front-call, depot-registration, depot-return", text)
            self.assertIn("```java\nsource body\n```\n", text)

    def test_preparation_preserves_late_operation_and_peer_evidence_under_discovery_pressure(self):
        dispatch = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units/42"}}
        front, terminal = (
            self.retained("intake", name="receive", entry_kind="SCHEDULED_JOB",
                          events=[("fetch", "client.get()", dispatch)]),
            self.retained("catalog", name="lookup", path="/units/{key}",
                          events=[("return", "return selectedUnit;", {"kind": "RETURN"})]),
        )
        helper_lines = (
            'val key = item.ref', 'val expiry = seen[key]', 'if (expiry?.isAfter(now) == true) return',
            'seen[key] = expiresAt', 'transaction.executeWithoutResult {',
            '    try {', '        client.get()', '        store.update(key)',
            '    } catch (error: IOException) {', '        throw error', '    }', '}',
        )
        helper_body = '\n'.join(helper_lines)
        premise = (
            'When the pending query has returned a row and its key selects a still-current parcel '
            'and validation reaches the callback after checking the cached expiry and the transaction '
            'is active and the external response has returned to this callback'
        )
        uncertainty = (
            'The source establishes this argument on the indicated normal continuation. The later '
            'committed store contents and delivery to another process depend on runtime outcomes.'
        )
        essentials = {
            'ENTRY': 'The retained schedule uses fixedDelay = 1000.',
            'SQL_LITERAL': 'The callback selects pending parcels before starting the loop.',
            'CALLBACK_BOUNDARY': 'The selected helper executes its store update inside the transaction callback.',
            'EARLY_EXIT': 'A still-valid expiry returns before sending or updating the store.',
            'HEAP_WRITE_BEFORE_CALL': 'The helper writes the expiry under item.ref before sending.',
            'SAME_KEY_HEAP_RETRY': 'A failed send can leave the same-key expiry in the map despite database rollback.',
            'EXCEPTION_BOUNDARY': 'The catch rethrows IOException before the store update completes.',
            'guarded-outcome': 'If the expiry has passed, the helper proceeds to the call.',
        }
        requirements, inventories, exact = [], {}, {}
        for side, _, sources in (front, terminal):
            service = side["service"]
            side.update(handlerId="entry-" + service, entrySymbol=side["entrypoint"],
                        sourceIds=list(sources), rootDeclarations=["root"])
            for source in sources.values():
                source["file"] = "Operations.kt"
            if service == 'intake':
                for handle, literal, line in (
                    ('receive-registration', '@Scheduled(fixedDelay = 1000)', 1),
                    ('receive-body', 'val items = store.query("SELECT ref FROM pending")\nfor (item in items) apply(item)', 2),
                    ('receive-helper', helper_body, 5),
                ):
                    sources[handle] = {**sources['receive-body'], 'id': handle, 'references': [handle],
                                       'text': literal, 'textDigest': D['sha'](literal.encode()),
                                       'startLine': line, 'endLine': line + literal.count('\n'),
                                       'startByte': line * 128, 'endByte': line * 128 + len(literal.encode()),
                                       'occurrence': {'snapshot': F['SNAPSHOT'], 'blob': 'a' * 40,
                                                      'startByte': line * 128, 'endByte': line * 128 + len(literal.encode())}}
            exact[service] = sources
            inventories[service] = {"coverage": {"coverage": "SYNTAX"}, "work": service + "-work",
                                    "revision": next(iter(sources.values()))["revision"]}
            if service == "intake":
                for i in range(3):
                    handle = "discovery-" + str(i)
                    source = {**sources["receive-body"], "id": handle, "file": f"Support{i}.kt",
                              "references": [handle]}
                    sources[handle] = source
                    requirements.append({"id": handle, "kind": "DISCOVERY", "service": service, "page": "intake.md",
                                         "file": source["file"], "sourceIds": [handle], "sourceReferences": [handle]})
            requirements.append(side)
        inventory = {"snapshot": F["SNAPSHOT"], "requirements": requirements, "services": inventories}
        projectors = runpy.run_path

        def projection(process, context, **_):
            handle = context["sourceRanges"][0]["id"]

            def fact(kind, text, **details):
                return {"kind": kind, "authority": "RETAINED_SOURCE", "text": text, "sources": [handle], **details}

            if not process["root"]:
                return [fact("validation_predicate", f"Discovery clause {i}. " + "A retained description. " * 55)
                        for i in range(96)], {}
            if process["service"] != "intake":
                return [fact("return", "Returns the selected catalog unit.")], {}
            helper = exact['intake']['receive-helper']

            def narrower(start, stop):
                lines = helper_lines[start:stop]
                site = '\n'.join(lines)
                byte = helper['startByte'] + sum(len(line.encode()) + 1 for line in helper_lines[:start])
                return {**helper, 'text': site, 'textDigest': D['sha'](site.encode()),
                        'startLine': helper['startLine'] + start, 'endLine': helper['startLine'] + stop - 1,
                        'startByte': byte, 'endByte': byte + len(site.encode())}

            lifted = {key: narrower(start, stop) for key, start, stop in (
                ('pointer', 0, 2), ('guard', 1, 3), ('heap', 2, 4), ('tx', 3, 6),
                ('send', 5, 7), ('write', 6, 8), ('catch', 8, 10),
            )}
            common = fact('sql-write', 'The operation reaches a local write.', sources=['tx', 'write'])
            essential_sources = [('receive-registration',), ('receive-body',), ('tx', 'write'), ('guard',),
                                 ('heap', 'send'), ('pointer', 'heap', 'catch'), ('catch',), ('guard', 'send')]
            reached = [fact(kind, text, sources=cites, **({'condition': 'otherwise'} if kind == 'guarded-outcome' else {}))
                       for (kind, text), cites in zip(essentials.items(), essential_sources)]
            bulk = [fact('write_lineage', f'Projection {i}. ' + 'A retained value witness. ' * 17,
                         sources=['pointer', 'guard', 'tx', 'send', 'write'],
                         condition=f'{premise} and value position `{i}` is available', uncertainty=uncertainty,
                         status='SOURCE_QUALIFIED_CANDIDATE',
                         _writeFocus={'occurrence': 'single-store-update'}) for i in range(235)]
            inventory = fact('composed-effect-inventory', 'Composes the reachable call steps. ' + 'A retained call. ' * 1200,
                             sources=['tx', 'send', 'write', 'catch'])
            return [inventory, *bulk, *(common if i % 2 else {**common, 'sources': ['write', 'tx']} for i in range(50)),
                    *reached], lifted

        def contexts(job, scope, home, selected, deadline):
            if intake := next((row for row in selected if row['kind'] == 'ENTRY'), None):
                body = intake['sourceReferences'][1]
                witness, event = (('receive-fetch', dispatch) if intake['service'] == 'intake'
                                  else ('lookup-return', {'kind': 'RETURN'}))
                rows = [('BODY', 'root', body, None), ('EVENT', 'root', witness, event)]
                if intake['service'] == 'intake':
                    rows.append(('BODY', 'helper', 'receive-helper', None))
                return {'root': [{'id': kind + ':' + handle, 'obligationKind': kind,
                                  'record': {'declaration': declaration, 'sourceIds': [handle], 'event': evidence},
                                  'sourceReferences': [handle]}
                                 for kind, declaration, handle, evidence in rows]}, {}
            return {}, {}

        def scripted_projectors(filename, **kwargs):
            name = Path(filename).name
            if name == "managed_languages.py":
                return {"project": projection}
            if name == "docs_run_contract_sources.py":
                return {"prepare": lambda *args: ({}, {})}
            loaded = projectors(filename, **kwargs)
            if name == "docs_run_prepared.py":
                loaded["prepare"].__globals__.update(
                    collect_contexts=contexts,
                    csharp_background_units=lambda *args: [], csharp_units=lambda *args: [],
                    acquire_sources=lambda *args: {}, local_repository=lambda *args: None,
                )
            return loaded

        def citation_load(path):
            side = Path(path).resolve().parent.name
            known = exact[side]
            records = {handle: {key: value for key, value in row.items() if key != "text"}
                       for handle, row in known.items()}
            packet = {"scope": {"snapshot": F["SNAPSHOT"]}, "work": inventories[side]["work"],
                      "texts": {handle: row["text"] for handle, row in known.items()},
                      "outcomes": [{"source": source, "sourceBinding": {"reference": known[handle]["references"][0]},
                                    "textKey": handle} for handle, source in records.items()]}
            return {}, packet, {}, {handle: {"source": row} for handle, row in records.items()}

        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            service_preparation = {}
            for service, scope in inventories.items():
                folder = home / service
                folder.mkdir()
                ready = {row["id"]: {"evidenceCandidates": row["sourceReferences"], "views": ["source.md"]}
                         for row in requirements if row["service"] == service}
                D["save"](folder / "requirements.json", ready)
                (folder / "source.md").write_text("# Retained source\n\nAn owned excerpt.\n")
                service_preparation[service] = {"sourceIndex": str(folder / "index.md"),
                    "manifest": str(folder / "citations.json"), "coverage": scope["coverage"],
                    "deferredResourceFiles": [], "views": {"source": {"view": "source.md", "file": "Operations.kt"}}}
            prepared = {"bounds": {"visibleBytes": 256 * 1024}, "visibleBytes": 0,
                        "requirements": requirements, "services": service_preparation}
            job = {"authorMode": "ISOLATED_PAGES", "services": list(inventories), "snapshot": F["SNAPSHOT"],
                   "request": "Describe the source behavior.", "language": "en", "node": "allocated-fixture"}
            with patch.object(runpy, "run_path", scripted_projectors), patch.dict(D["M"]["C"], {"load": citation_load}):
                D["prepare_facts"](job, inventory, prepared)
            intake = D["load"](home / "intake/prepared-facts.json")["entries"]
            large = intake["discovery-0"]
            self.assertGreater(large["factOmissions"]["DISCOVERY_FACT_BYTES"], 0)
            operation = intake[front[0]["id"]]
            kinds = [fact["kind"] for fact in operation["propositions"]]
            self.assertIn("http-peer-outcome", kinds)
            self.assertIn("write_lineage", kinds)
            self.assertEqual(kinds.count("sql-write"), 1)
            self.assertEqual(operation['omittedForLimit'], 0)
            self.assertEqual(kinds.count('write_lineage'), 235)
            catalog = prepared['services']['intake']['preparedFacts']['witnessCatalog']
            self.assertEqual(len(catalog), 7)
            self.assertEqual(len({json.dumps(proof, sort_keys=True) for proof in catalog}), 7)
            self.assertTrue(any(fact.get("condition") == "otherwise" for fact in operation["propositions"]))
            pages = D["P"]
            context_codec = pages['FACT_CONTEXT']
            plain = lambda instructions, packet: instructions + '\n\n' + context_codec['compact_json'](packet) + '\n'
            for name, size in (('plain', 192 * 1024), ('normal', 192 * 1024), ('limited', 18 * 1024)):
                settings = {'citation_load': citation_load, 'MAX_PACKET_BYTES': size,
                            'FACT_CONTEXT': {**context_codec, 'author_prompt': plain} if name == 'plain' else context_codec}
                with patch.dict(pages['page_jobs'].__globals__, settings):
                    plan = pages["page_jobs"](job, inventory, prepared, home / name, D["M"]["MAX_PAGES"], "Owned instructions.")
                slot = next(item for item in plan["slots"] if item["requirement"] == front[0]["id"])
                path = home / slot["prompt"]
                encoded = json.loads(path.read_text().split('\n\n')[-1])
                packet = pages['expand_fact_context'](encoded)
                facts = packet["facts"][front[0]["id"]]["facts"]
                self.assertTrue(any(item["kind"] == "http-peer-outcome" and "return selectedUnit" in item["text"]
                                    for item in facts), facts)
                self.assertTrue(any(item["kind"] == "write_lineage" for item in facts), facts)
                self.assertEqual(set(packet["facts"]), {front[0]["id"]})
                self.assertNotIn(front[0]['id'], packet['preparationOmissions'])
                delivered = {item['kind']: item for group in ('facts', 'boundaries')
                             for item in packet['facts'][front[0]['id']].get(group, ())}
                self.assertTrue(essentials.keys() <= delivered.keys(), delivered.keys())
                self.assertEqual(delivered['guarded-outcome']['condition'], 'otherwise')
                self.assertTrue(delivered['SAME_KEY_HEAP_RETRY']['lines'])
                self.assertEqual(len(packet['sourceRanges']), len({(row['file'], row['startLine'], row['endLine'], row['text'])
                                                                 for row in packet['sourceRanges'].values()}))
                self.assertNotIn('_sourceWitnessIds', json.dumps(packet))
                self.assertLessEqual(path.stat().st_size, size)
                if name != 'normal':
                    self.assertGreater(packet['omittedPreparedFacts'], 0)
                    if name == 'limited':
                        self.assertNotIn('composed-effect-inventory', delivered)
                else:
                    self.assertEqual(packet['omittedPreparedFacts'], 0)
                    self.assertIn('composed-effect-inventory', delivered)
                    chosen = [item for item in facts if item['kind'] == 'write_lineage']
                    self.assertEqual(len(chosen), 235)
                    self.assertIn('factContext', encoded)
                    self.assertIn('conditionTail', json.dumps(encoded))
                    self.assertLess(path.stat().st_size, len(plain('Owned instructions.', packet).encode()))
                    for item in chosen:
                        number = int(item['text'].split('.')[0].split()[-1])
                        self.assertEqual(item['condition'], f'{premise} and value position `{number}` is available')
                        self.assertEqual(item['uncertainty'], uncertainty)
                        self.assertEqual(item['authority'], 'RETAINED_SOURCE')
                        self.assertEqual(item['status'], 'SOURCE_QUALIFIED_CANDIDATE')
                        handles = [slot['citations'][alias] for alias in item['sources']]
                        self.assertEqual(handles, ['receive-helper'])
                        witnesses = packet['citations'][item['sources'][0]]['regions']
                        literal = '\n'.join(packet['sourceRanges'][region]['text'] for region in witnesses)
                        self.assertIn('transaction.executeWithoutResult', literal)
                        self.assertIn('store.update(key)', literal)

    def test_response_decoding_failure_does_not_establish_upstream_rollback(self):
        site = "front-body"
        cause = {"_edgeCause": "call-1", "_edgePhase": "RESPONSE_DECODING"}
        def fact(kind, text, handle, **metadata):
            return {"kind": kind, "authority": "RETAINED_PATH", "text": text, "sources": [handle], **metadata}
        route = fact("http_request", "The operation awaits fetch.", site,
                     _edge={"kind": "HTTP", "method": "GET", "uri": "/units", "site": site, "occurrence": "call-1"})
        fragments = [route,
                     fact("response_body_read", "Awaits decoding the fetched value.", site, **cause),
                     fact("exception_route", "If decoding rejects, control enters the catch.", site, **cause),
                     fact("status_body", 'reply.status receives 502. The body says "Unavailable".', site, **cause)]
        commit = fact("http-commit-before-response", "The handler commits its write before constructing HTTP 200.", "peer-body")
        peer = self.retained("peer", facts=[commit])
        unrelated = self.retained("front", path="/front", facts=fragments)
        self.assertFalse(any(row["kind"] == "http-failure-after-peer-commit"
                             for row in self.project(unrelated, peer)[unrelated[0]["id"]]))
        matched = fact("response_decode_failure_outcome", 'reply.status receives 502. The body says "Unavailable".',
                       site, **cause)
        front = self.retained("front", path="/front", facts=[*fragments, matched])
        pivot, = (row for row in self.project(front, peer)[front[0]["id"]] if row["kind"] == "http-failure-after-peer-commit")
        self.assertIn("502", pivot["text"])
        self.assertIn("Unavailable", pivot["text"])
        self.assertIn("may already be committed", pivot["uncertainty"])
        self.assertIn("does not establish upstream rollback", pivot["uncertainty"])
        self.assertEqual(pivot["sources"], [site, "peer-registration", "peer-body"])
        no_commit = self.retained("peer", facts=[fact("http-outcome", "Returns HTTP 200.", "peer-body")])
        self.assertFalse(any(row["kind"] == "http-failure-after-peer-commit"
                             for row in self.project(front, no_commit)[front[0]["id"]]))

    def test_full_preparation_keeps_private_cause_until_both_services_are_projected(self):
        node = shutil.which("node")
        if not node or subprocess.run([node, "-e", 'require("typescript")'], capture_output=True).returncode:
            self.skipTest("The locked TypeScript projector is unavailable")
        front = """async function claim(reply: Reply, host: string, id: string) {
    await fetch("/archives/one", {method: "POST"});
    await fetch("/archives/two", {method: "POST"});
    await fetch("/archives/three", {method: "POST"});
    await fetch("/archives/four", {method: "POST"});
    try {
        const res = await fetch(`${host}/books/${encodeURIComponent(id)}/claim`, {method: "POST"});
        reply.code(res.status).write(await res.json());
    } catch {
        reply.code(502).write({error: "REMOTE_UNAVAILABLE"});
    }
}
"""
        catalogue = """async Task<int> Claim(DbConnection db, bool notify, string slug) {
    if (slug.Length == 0) return 400;
    var claimId = Guid.NewGuid();
    await db.ExecuteAsync("INSERT INTO claims (id) VALUES (@id)", new { id = claimId });
    if (notify) {
        var body = JsonSerializer.Serialize(new ClaimCreated { Slug = slug });
        await db.ExecuteAsync("INSERT INTO notices (body) VALUES (@body)", new { body });
    }
    return 201;
}
"""
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory).resolve()
            services, scopes, requirements, work_rows = {}, {}, [], {}
            for name, file, body, route in (
                ("gateway", "Gateway.ts", front, "/claim"),
                *((word, f"{word}.kt", "fun archive() = 202\n", f"/archives/{word}")
                  for word in ("one", "two", "three", "four")),
                ("catalogue", "Catalogue.cs", catalogue, "/books/{id}/claim"),
            ):
                home = root / name
                home.mkdir()
                work = name + "-work"
                source = F["source"]()
                source.update(file=file, text=body, textDigest=D["sha"](body.encode()), startLine=1,
                              endLine=len(body.splitlines()))
                manifest, _, _ = F["fixture"](home, service=name, work=work, record=source,
                                               authority="EXACT_SNAPSHOT_TEXT")
                _, _, _, cites = D["M"]["C"]["load"](manifest)
                scope = {"work": work, "revision": source["revision"], "coverage": {"coverage": "SYNTAX", "boundaries": []},
                         "inputDigest": work + "-input", "contextDigest": work + "-context"}
                entry = {"id": name + ":entry", "kind": "ENTRY", "entryKind": "HTTP_ENDPOINT",
                         "service": name, "entrypoint": "claim", "method": "POST", "path": route,
                         "handlerId": name + "-claim", "page": name + "-claim.md",
                         "sourceIds": [source["id"]], "sourceReferences": ["s9"],
                         "registrationSourceIds": [source["id"]], "rootDeclarations": [name + "-root"]}
                work_rows[work] = {"kind": "THREAD_OBLIGATION", "id": "BODY:" + name, "obligationKind": "BODY",
                                   "record": {"declaration": name + "-root", "sourceIds": [source["id"]]},
                                   "sourceReferences": ["s9"]}
                D["save"](home / "requirements.json", {entry["id"]: {"views": ["view.md"],
                                                                    "evidenceCandidates": list(cites)}})
                (home / "view.md").write_text("# " + file + "\n\n```\n" + body + "```\n")
                services[name] = {"manifest": str(manifest), "sourceIndex": str(home / "index.md"),
                                  "coverage": scope["coverage"], "deferredResourceFiles": [],
                                  "views": {file: {"file": file, "view": "view.md"}}}
                scopes[name] = scope
                requirements.append(entry)

            def collect(launcher, docs_root, work, output, **kwargs):
                output.mkdir(parents=True)
                kind = kwargs["selection"]["query"]["kind"]
                D["save"](output / "items.json", [work_rows[work]] if kind == "THREAD_CONTEXT" else [])
                return {"status": "COMPLETE", "identity": {"snapshot": F["SNAPSHOT"],
                        "inputDigest": work + "-input", "contextDigest": work + "-context"}}

            contract_text = json.dumps({
                "openapi": "3.1.0",
                "paths": {"/books/{key}/claim": {"post": {"responses": {
                    "202": {"description": "Pending", "content": {"application/json": {
                        "schema": {"type": "object", "properties": {"ticket": {"type": "string"}}}}}}}}},
                          "/unrelated": {"post": {"responses": {"418": {"description": "Separate operation"}}}}},
            }, indent=2)
            revision = scopes["catalogue"]["revision"]
            manifest = (root / "catalogue/fact-evidence/contracts-native-citations/citations.json").resolve()
            spec = {key: value for key, value in F["source"](contract_text).items() if key != "text"}
            spec.update(id="spec", service="catalogue", bindingService="catalogue", revision=revision,
                        file="contracts/api.json", startLine=1, endLine=len(contract_text.splitlines()),
                        authority="REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS",
                        occurrence={"snapshot": revision, "startByte": 0, "endByte": len(contract_text.encode())})
            native = {"kind": "SOURCE_NATIVE", "binding": {"revision": revision},
                      "evidenceSet": {"service": "catalogue", "snapshot": F["SNAPSHOT"]}}
            native_entries = {"catalogue:api": {"source": spec, "nativeBinding": native}}
            packet = {"kind": "NATIVE", "scope": native["evidenceSet"],
                      "outcomes": [{"source": spec, "textKey": spec["textDigest"]}],
                      "texts": {spec["textDigest"]: contract_text}}
            load_citation = D["M"]["C"]["load"]

            def citation_load(path):
                if Path(path).resolve() == manifest:
                    return {}, packet, {}, native_entries
                return load_citation(path)

            native_run_path = runpy.run_path

            def run_path(filename, **kwargs):
                module = native_run_path(filename, **kwargs)
                if filename.endswith("/docs_run_contract_sources.py"):
                    def pinned_sources(*args):
                        return (module["packet_sources"](native_entries, packet, manifest)
                                if args[1] == "catalogue" else {})
                    module["prepare"].__globals__["pinned_sources"] = pinned_sources
                if not filename.endswith("/managed_languages.py"):
                    return module
                original = module["project"]
                def adapt(process, context, *, node):
                    if process["service"] == "gateway":
                        rows, witnesses = original(process, context, node=node)
                        self.assertTrue(any(row["kind"] == "response_decode_failure_outcome" for row in rows), rows)
                        return rows, witnesses
                    proof, = context["sourceRanges"]
                    def facet(kind, text, **extra):
                        return {"kind": kind, "text": text, "authority": "RETAINED_PATH", "sources": [proof["id"]], **extra}
                    outcomes = [facet("http-outcome", f"Success alternative {alias} yields HTTP 200.", _httpStatus=200)
                                for alias in range(6)]
                    outcomes.extend(facet("http-outcome", f"The handler can return HTTP {status}.", _httpStatus=status)
                                    for status in (400, 409, 500))
                    outcomes.extend([
                        facet("http-commit-before-response", "The handler commits the transaction before returning HTTP 201."),
                        facet("conditional-no-write", "If validation rejects the book, this path bypasses the INSERT."),
                        facet("conditional-no-write", "If the user cancels, this path bypasses the notice."),
                        facet("conditional-no-write", "If the nested lookup returns no list entry, this path bypasses the enqueue.",
                              condition="the collection lookup misses"),
                        facet("helper-return", "The nested lookup returns its retained list of candidate books."),
                        facet("awaited-list-selection", "The awaited result supplies the list searched by book ID.",
                              condition="the list is returned normally"),
                        facet("lookup-guard-return", "When lookup yields no book, the handler returns HTTP 400 with INVALID_BOOK.",
                              _httpStatus=400, condition="lookup yields no book"),
                        facet("guard-return-bypass", "The missing lookup's early return bypasses the enqueue."),
                        facet("argument-projection", "The selected book row supplies id and title to the persisted object.",
                              condition="lookup matches the book"),
                        facet("sql-write", "The source supplies an INSERT on claims without a selected slot.",
                              _edge={"kind": "INSERT", "table": "claims", "site": proof["id"], "occurrence": "claim-write"}),
                        facet("sql-write", "The INSERT INTO claims binds @id from Guid.NewGuid().", _writeBinding=True,
                              condition="slug is nonempty",
                              _edge={"kind": "INSERT", "table": "claims", "site": proof["id"], "occurrence": "claim-write"}),
                        facet("sql-write", "The INSERT INTO notices binds @body from JsonSerializer.Serialize over ClaimCreated.",
                              _writeBinding=True, condition="notify is true",
                              _edge={"kind": "INSERT", "table": "notices", "site": proof["id"], "occurrence": "notice-write"}),
                    ])
                    return outcomes, {}
                return {**module, "project": adapt}

            job = {"clew": "/fixture/clew", "root": str(root), "snapshot": F["SNAPSHOT"],
                   "node": node, "services": list(services), "request": "Document book claims.", "language": "en",
                   "authorMode": "ISOLATED_PAGES"}
            prepared = {"bounds": {"visibleBytes": 100_000}, "visibleBytes": 0,
                        "requirements": requirements.copy(), "services": services}
            inventory = {"requirements": requirements, "services": scopes, "snapshot": F["SNAPSHOT"]}
            with patch.dict(D["T"], {"collect": collect}), patch.object(runpy, "run_path", run_path):
                with patch.dict(D["prepare_facts"].__globals__, {"command": lambda *a, **kw: {}}):
                    D["prepare_facts"](job, inventory, prepared)
            caller_facts = [row for row in D["load"](root / "gateway/prepared-facts.json")["entries"]["gateway:entry"]["propositions"]
                            if row["kind"].startswith("http-")]
            self.assertTrue(any(row["kind"] == "http-failure-after-peer-commit" and "REMOTE_UNAVAILABLE" in row["text"]
                                for row in caller_facts), [row["kind"] for row in caller_facts])
            self.assertGreater(len(caller_facts), 48)
            authored = D["load"](root / "gateway/prepared-facts.json")["entries"]["gateway:entry"]["propositions"]
            self.assertTrue(all(not any(key.startswith("_") for key in row) for row in authored))

            pages = D["P"]
            with patch.dict(pages["page_jobs"].__globals__, {"citation_load": citation_load}):
                plan = pages["page_jobs"](job, inventory, prepared, root / "page-prompts", 20, "Fixture instructions.")
                slot = next(row for row in plan["slots"] if row["requirement"] == "gateway:entry")
                issue = pages["expand_fact_context"](json.loads((root / slot["prompt"]).read_text().split("\n\n")[-1]))
                self.assertEqual(set(issue["facts"]), {"gateway:entry"})
                self.assertEqual(issue["omittedPreparedFacts"], 0)
                facts = issue["facts"]["gateway:entry"]["facts"]
                facts = [row for row in facts if "catalogue-claim.md" in row["text"]]
                declared, = (row for row in facts if row["kind"] == "http-peer-declared-response")
                self.assertEqual(declared["authority"], "DECLARED_OPENAPI")
                self.assertIn("`202`", declared["text"])
                self.assertIn("`ticket`", declared["text"])
                self.assertNotIn("418", declared["text"])
                self.assertIn("runtime enforcement", declared["uncertainty"])
                ranges = [issue["sourceRanges"][issue["citations"][alias]["region"]] for alias in declared["sources"]]
                selected, = (row for row in ranges if row["file"] == "contracts/api.json")
                self.assertEqual(selected["service"], "catalogue")
                self.assertNotIn("/unrelated", selected["text"])
                self.assertTrue(all(any(str(status) in row["text"] for row in facts if row["kind"] == "http-peer-outcome")
                                    for status in (200, 400, 409, 500)))
                for kind, value in (("http-peer-commit-before-response", "HTTP 201"),
                                    ("http-peer-guarded-return", "INVALID_BOOK"),
                                    ("http-peer-return-bypass", "bypasses the enqueue"),
                                    ("http-peer-value-lineage", "id and title")):
                    row, = (row for row in facts if row["kind"] == kind)
                    self.assertIn(value, row["text"])
                writes = [row for row in facts if row["kind"] == "http-peer-sql-write"]
                claims, = (row for row in writes if "Guid.NewGuid()" in row["text"])
                notices, = (row for row in writes if "ClaimCreated" in row["text"])
                self.assertIn("claims binds @id from Guid.NewGuid()", claims["text"])
                self.assertIn("slug is nonempty", claims["condition"])
                self.assertNotIn("ClaimCreated", claims["text"])
                self.assertIn("notices binds @body from JsonSerializer.Serialize over ClaimCreated", notices["text"])
                self.assertIn("notify is true", notices["condition"])
                self.assertNotIn("Guid.NewGuid", notices["text"])
                for write in (claims, notices):
                    files = {issue["sourceRanges"][issue["citations"][handle]["region"]]["file"] for handle in write["sources"]}
                    self.assertEqual(files, {"Gateway.ts", "Catalogue.cs"})
                    self.assertIn("commit, visibility and timing remain unknown", write["uncertainty"])
                negative = [row for row in facts if row["kind"] == "http-peer-conditional-no-write"]
                self.assertEqual(len(negative), 3)
                self.assertTrue(any("nested lookup returns no list entry" in row["text"]
                                    and row.get("condition") == "the collection lookup misses" for row in negative))
                self.assertTrue(any(row["kind"] == "http-peer-helper-result" and "list of candidate books" in row["text"]
                                    for row in facts))
                self.assertTrue(any(row["kind"] == "http-peer-list-selection" and "list searched by book ID" in row["text"]
                                    for row in facts))
                self.assertTrue(any(row["kind"] == "http-failure-after-peer-commit" and "REMOTE_UNAVAILABLE" in row["text"]
                                    for row in facts))
                for bad_binding, bad_text in (({"revision": "stale"}, contract_text), (native["binding"], "different bytes")):
                    with patch.dict(native, {"binding": bad_binding}), patch.dict(packet["texts"], {spec["textDigest"]: bad_text}):
                        with self.assertRaisesRegex(D["Stop"], "PAGE_PEER_SOURCE_MEMBERSHIP_CHANGED"):
                            pages["service_evidence"](services["gateway"], "gateway", scopes, F["SNAPSHOT"])

    def test_http_peer_ambiguity_does_not_select_an_outcome_and_missing_proof_stays_open(self):
        call = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_TEMPLATE_LITERAL_SUFFIX/1.0", "method": "GET", "path": "/units"}}
        front = self.retained("front", path="/start", events=[("call", "client.get()", call)])
        first = self.retained("first")
        unproven = self.retained("unproven", proof=False)
        candidates = (front, first, unproven)
        declaration = {"kind": "openapi-declared-response", "authority": "DECLARED_OPENAPI",
                       "status": "SOURCE_ROUTE_MATCH_ONLY", "text": "Declares HTTP 202.",
                       "uncertainty": "Runtime enforcement remains unknown.", "sources": ["spec"]}
        row, = self.edges["project"]([row[0] for row in candidates], {row[0]["id"]: row[1] for row in candidates},
                                     contracts={first[0]["id"]: [declaration]})[front[0]["id"]]
        self.assertEqual((row["kind"], row["status"]), ("http-peer-boundary", "AMBIGUOUS"))
        self.assertIn("first-registration", row["sources"])
        candidates = (front, first)
        conflict = {**declaration, "kind": "openapi-overlay-boundary", "status": "AMBIGUOUS",
                    "text": "Multiple declarations remain possible.", "sources": ["spec-one", "spec-two"]}
        qualified = self.edges["project"]([row[0] for row in candidates], {row[0]["id"]: row[1] for row in candidates},
                                          contracts={first[0]["id"]: [conflict]})[front[0]["id"]]
        boundary, = (row for row in qualified if row["kind"] == "http-peer-declaration-boundary")
        self.assertEqual(boundary["status"], "AMBIGUOUS")
        self.assertEqual(boundary["sources"], ["front-call", "first-registration", "spec-one", "spec-two"])
        self.assertFalse(any(row["kind"] == "http-peer-declared-response" for row in qualified))
        for others in ((), (unproven,), (self.retained("wrong-method", verb="POST"),)):
            row, = self.project(front, *others)[front[0]["id"]]
            self.assertEqual((row["kind"], row["status"], row["sources"]),
                             ("http-peer-boundary", "UNKNOWN", ["front-call"]))

    def test_templates_leave_literal_overlaps_and_traversal_open(self):
        def call(uri, encoded=True):
            return {"kind": "http_request", "authority": "RETAINED_PATH", "sources": ["front-body"],
                    "text": "The request uses an encoded identifier.",
                    "uncertainty": "The identifier must encode as one non-dot segment.",
                    "_edge": {"kind": "HTTP", "site": "front-body", "method": "POST", "uri": uri,
                              "encodedSegment": encoded}}

        peer = self.retained("peer", verb="POST", path="/units/{id}/action")
        special = self.retained("special", verb="POST", path="/units/current/action")
        front = self.retained("front", path="/start", facts=[call("/units/{segment}/action")])
        self.assertEqual(self.project(front, peer)[front[0]["id"]][0]["kind"], "http-peer")
        ambiguous, = self.project(front, peer, special)[front[0]["id"]]
        self.assertEqual(ambiguous["status"], "AMBIGUOUS")
        for uri, encoded in (("/units/{segment}/action", False), ("/units/../{segment}/action", True),
                             ("/units/%2e/{segment}/action", True)):
            front = self.retained("front", path="/start", facts=[call(uri, encoded)])
            boundary, = self.project(front, peer)[front[0]["id"]]
            self.assertEqual(boundary["status"], "UNKNOWN")

    def test_subscriber_delete_links_to_observed_absence_without_selecting_deleted_row_for_a_poll(self):
        def operation(site, kind, **identity):
            return {"kind": "effect", "authority": "RETAINED_PATH", "text": "The source requests the cited operation.",
                    "sources": [site + "-body"], "_edge": {"kind": kind, "site": site + "-body", **identity}}

        sender = self.retained("catalog", facts=[operation("catalog", "PUBLISH", topic="book.removed")])
        listener = self.retained("stock", name="listener", entry_kind="KAFKA_LISTENER",
                                 trigger={"configuration": {"topics": ["book.removed"]}},
                                 facts=[operation("listener", "DELETE", table="record")])
        reader = self.retained("stock", name="read", facts=[operation("read", "SELECT", table="record")])
        relay = self.retained("stock", name="relay", entry_kind="SCHEDULED_JOB",
                              facts=[operation("relay", "SELECT", table="record"),
                                     operation("relay", "PUBLISH", topic="record.available")])
        projected = self.project(sender, listener, reader, relay)
        readbacks = [fact for fact in projected[sender[0]["id"]] if fact["kind"] == "message-storage-reader"]
        self.assertTrue(readbacks)
        self.assertTrue(all("absence" in fact["text"] and "remains deleted" in fact["text"] for fact in readbacks))
        self.assertFalse(any(fact["kind"].startswith("storage-message") for fact in projected[listener[0]["id"]]))

    def poll_proof(self, name, publication="send"):
        return {"kind": "poll-row-publication", "authority": "RETAINED_PATH", "sources": [name + "-body"],
                "text": "The selected row supplies the publication argument.",
                "_pollPublication": {"readOccurrence": "select", "publicationOccurrence": publication}}

    def test_chain_prioritizes_a_public_projection_and_keeps_a_guarded_terminal_path(self):
        def operation(site, kind, **identity):
            condition = identity.pop("condition", None)
            return {"kind": "source-effect", "authority": "RETAINED_PATH", "text": f"The source reaches {site}.",
                    "condition": condition, "sources": [site + "-body"],
                    "_edge": {"kind": kind, "site": site + "-body", **identity}}

        def consumer(service, name, topic, effects):
            return self.retained(service, name=name, entry_kind="KAFKA_LISTENER",
                                 trigger={"configuration": {"topics": [topic]}}, facts=effects)

        origin = self.retained("orders", name="request", facts=[operation("request", "INSERT", table="booking_events")])
        first = self.retained("orders", name="booking-poll", entry_kind="SCHEDULED_JOB", facts=[
            operation("booking-poll", "SELECT", table="booking_events", occurrence="select"),
            operation("booking-poll", "PUBLISH", topic="booking.requested", occurrence="send"),
            self.poll_proof("booking-poll")])
        validator = consumer("allocation", "validator", "booking.requested", [
            operation("validator", "INSERT", table="received"),
            operation("validator", "INSERT", table="declined", condition="payload lacks a required member"),
            operation("validator", "INSERT", table="changes", condition="payload passes validation"),
        ])
        second = self.retained("allocation", name="change-poll", entry_kind="HOSTED_BACKGROUND_WORKER", facts=[
            operation("change-poll", "SELECT", table="changes", occurrence="select"),
            operation("change-poll", "PUBLISH", topic="booking.allocated", occurrence="send"),
            operation("change-poll", "PUBLISH", topic="booking.requested", occurrence="repeat"),
            self.poll_proof("change-poll"), self.poll_proof("change-poll", "repeat"),
        ])
        index = consumer("reader", "index", "booking.allocated", [
            operation("index", "INSERT", table="incoming"),
            operation("index", "INSERT", table="bookkeeping"),
            operation("index", "UPDATE", table="availability", condition="revision passes the stale-input test"),
            operation("index", "INSERT", table="retained_views", condition="visibility is requested"),
        ])
        query = self.retained("reader", name="availability", path="/availability", facts=[
            operation("availability", "SELECT", table="availability")])
        third = self.retained("reader", name="view-poll", entry_kind="SCHEDULED_JOB", facts=[
            operation("view-poll", "SELECT", table="retained_views", occurrence="select"),
            operation("view-poll", "PUBLISH", topic="booking.visible", occurrence="send"),
            self.poll_proof("view-poll"),
        ])
        viewer = consumer("public", "receive", "booking.visible", [
            operation("receive", "UPDATE", table="visible_records", condition="the projection key is accepted")])
        public_read = self.retained("public", name="read", path="/visible", facts=[
            operation("read", "SELECT", table="visible_records")])
        call = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units"}}
        request = self.retained("gateway", path="/start", events=[("call", "client.get()", call)])
        result = self.project(request, origin, first, validator, second, index, query, third, viewer, public_read)
        facts = result[origin[0]["id"]]
        hop, = (row for row in result[request[0]["id"]] if row["kind"] == "http-further-subscriber-outbox")
        self.assertIn("INSERT on `changes` in `validator.md`", hop["text"])
        self.assertIn("independently registered poller in `change-poll.md`", hop["text"])
        self.assertIn("payload passes validation", hop["condition"])
        self.assertIn("validator-body", hop["sources"])
        reached = next(row for row in result[first[0]["id"]]
                       if row["kind"] == "further-subscriber-readback" and "availability.md" in row["text"])
        self.assertIn("booking.allocated", reached["text"])
        self.assertIn("availability.md", reached["text"])
        self.assertIn("payload passes validation", reached["condition"])
        self.assertIn("revision passes the stale-input test", reached["condition"])
        self.assertEqual(reached["sources"], ["booking-poll-body",
                                              "validator-registration", "validator-body", "change-poll-registration",
                                              "change-poll-body", "index-registration", "index-body",
                                              "availability-registration", "availability-body"])
        self.assertTrue(any(row["kind"] == "storage-message-effect" and
                            "payload lacks a required member" in row.get("condition", "") for row in facts))
        terminal = next(row for row in facts if row["kind"] == "further-subscriber-readback"
                        and "booking.visible" in row["text"])
        self.assertTrue({"request-body", "view-poll-registration", "receive-registration", "read-body"} <= set(terminal["sources"]))
        self.assertIn("visibility is requested", terminal["condition"])
        self.assertIn("projection key is accepted", terminal["condition"])
        self.assertTrue(any(row["kind"] == "further-outbox-boundary" and
                            "previously considered callback" in row["text"] for row in facts))
        self.assertLessEqual(len(facts), self.edges["MAX_LINKS"])

    def test_isolated_outbox_preserves_distinct_reached_write_guards_by_topic(self):
        def fact(name, kind, **metadata):
            return {"kind": kind, "authority": "RETAINED_PATH", "text": "Reached " + kind + ".",
                    "sources": [name + "-body"], **metadata}

        def relay(service, name, table, topic):
            return self.retained(service, name=name, entry_kind="SCHEDULED_JOB", facts=[
                fact(name, "sql-read", _edge={"kind": "SELECT", "site": name + "-body", "table": table, "occurrence": "select"}),
                fact(name, "broker-send", _edge={"kind": "PUBLISH", "site": name + "-body", "topic": topic, "occurrence": "send"}),
                self.poll_proof(name),
            ])

        client = self.retained("gateway", path="/start", events=[("call", "client.get()", {
            "kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
                "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units"}})])
        origin = self.retained("intake", name="reserve", facts=[
            fact("reserve", "sql-write", _edge={"kind": "INSERT", "site": "reserve-body", "table": "drafts"})])
        first = relay("intake", "relay", "drafts", "draft.queued")
        second = relay("processor", "flush", "edits", "draft.saved")
        subscriber = self.retained("catalogue", name="view", entry_kind="KAFKA_LISTENER",
                                   trigger={"configuration": {"topics": ["draft.saved"]}})
        operation = {"kind": "UPDATE", "site": "apply-body", "table": "edits", "occurrence": "one-write"}
        focus = {"occurrence": "one-write", "topic": "draft.queued", "compatibility": "COMPATIBLE"}
        paths = [
            fact("apply", "sql-write", _edge=operation, condition="deduplication allows the event"),
            fact("apply", "sql-write", _edge=operation, _writeSite="store", sources=["apply-body", "store"],
                 condition="deduplication allows the event"),
            *[fact("apply", "write_request", condition=guard,
                   _writeFocus={"occurrence": "one-write", "topic": topic, "compatibility": "COMPATIBLE"})
              for guard, topic in (
                  ("deduplication allows the event and the cache has no version", "draft.queued"),
                  ("deduplication allows the event and durable replay mode is on", "draft.queued"),
                  ("an unrelated schema version is present", "draft.old"),
              )],
        ]
        paths.extend([
            fact("apply", "write_lineage", text="The UPDATE binds revision from the local cache.",
                 condition="validation succeeds and the local cache contains a version",
                 sources=["apply-body", "store", "apply-cache"], _writeFocus=focus, _writeSite="store"),
            fact("apply", "write_lineage", text="The UPDATE binds revision from the decoded provider reply.",
                 condition="validation succeeds and the local cache is empty and the provider reply decodes",
                 sources=["apply-body", "store", "apply-external"], _writeFocus=focus, _writeSite="store"),
            fact("apply", "write_skip", text="The validation exit bypasses the selected UPDATE.",
                 condition="validation rejects the decoded revision", sources=["apply-body", "store", "apply-validation"],
                 _writeFocus=focus, _writeSite="store"),
            fact("apply", "sql-write", _edge={**operation, "occurrence": "unrelated-write"},
                 sources=["apply-body", "other"], _writeSite="other"),
            fact("apply", "write_compatibility", sources=["apply-body", "other"], _writeSite="other",
                 _writeFocus={**focus, "occurrence": "unrelated-write", "compatibility": "INCOMPATIBLE"}),
            fact("apply", "write_lineage", text="The other write binds revision from an unrelated source.",
                 sources=["apply-body", "other"], _writeFocus=focus, _writeSite="other"),
            fact("apply", "write_skip", text="An old topic refuses the update.",
                 sources=["apply-body", "store"], _writeFocus={**focus, "topic": "draft.old"}, _writeSite="store"),
        ])

        def callback(facts):
            return self.retained("processor", name="apply", entry_kind="KAFKA_LISTENER",
                                 trigger={"configuration": {"topics": ["draft.queued", "draft.old"]}},
                                 events=[
                                     ("store", 'db.update("UPDATE edits SET revision = ?", value)', {"kind": "CALL"}),
                                     ("other", 'db.update("UPDATE edits SET revision = ?", 100)', {"kind": "CALL"}),
                                     ("cache", "value = cache.current()", {"kind": "CALL"}),
                                     ("external", "value = decode(provider.read())", {"kind": "CALL"}),
                                     ("validation", "require(value > 0)", {"kind": "CALL"}),
                                 ], facts=facts, write_sites=("store", "other"))

        def first_hops(consumer):
            rows = self.project(client, origin, first, consumer, second, subscriber, isolated=True)[client[0]["id"]]
            return [row for row in rows if row["kind"] == "http-further-subscriber-outbox"]

        consumer = callback(paths)
        rows = self.project(client, origin, first, consumer, second, subscriber, isolated=True)[client[0]["id"]]
        lineage = [row for row in rows if row["kind"] == "http-further-subscriber-write-lineage"]
        cached, = (row for row in lineage if "local cache." in row["text"])
        decoded, = (row for row in lineage if "decoded provider reply" in row["text"])
        self.assertEqual(len(lineage), 2)
        self.assertEqual(cached["condition"], "validation succeeds and the local cache contains a version")
        self.assertEqual(decoded["condition"],
                         "validation succeeds and the local cache is empty and the provider reply decodes")
        self.assertIn("apply-cache", cached["sources"])
        self.assertNotIn("apply-cache", decoded["sources"])
        self.assertIn("apply-external", decoded["sources"])
        bypass, = (row for row in rows if row["kind"] == "http-further-subscriber-write-bypass")
        self.assertIn("bypasses the selected UPDATE", bypass["text"])
        self.assertIn("apply-validation", bypass["sources"])
        self.assertEqual(bypass["condition"], "validation rejects the decoded revision")
        for facet in (*lineage, bypass):
            self.assertIn("separate poller `flush.md`", facet["text"])
            self.assertTrue({"gateway-call", "apply-store", "flush-registration"} <= set(facet["sources"]))
            self.assertNotIn("apply-other", facet["sources"])
        hops = first_hops(consumer)
        self.assertEqual({row["condition"] for row in hops}, {
            "deduplication allows the event and the cache has no version",
            "deduplication allows the event and durable replay mode is on",
        })
        self.assertTrue(all("UPDATE on `edits` in `apply.md`" in row["text"] for row in hops))
        ambiguous, = first_hops(callback([*paths, fact("apply", "sql-write",
                                                      _edge={**operation, "table": "other_table"})]))
        self.assertEqual(ambiguous["condition"], "deduplication allows the event")
        conflicting_site = callback([*paths, fact("apply", "sql-write", _edge=operation,
                                                    sources=["apply-body", "other"], _writeSite="other")])
        conflicts = self.project(client, origin, first, conflicting_site, second, subscriber,
                                 isolated=True)[client[0]["id"]]
        self.assertFalse(any(row["kind"] in {"http-further-subscriber-write-lineage",
                                            "http-further-subscriber-write-bypass"} for row in conflicts))

    def test_continuation_budget_retains_a_cited_frontier(self):
        def fact(site, kind, **identity):
            return {"kind": "effect", "authority": "RETAINED_PATH", "text": "The source reaches " + site + ".",
                    "sources": [site + "-body"], "_edge": {"kind": kind, "site": site + "-body", **identity}}

        first = self.retained("coordinator", name="start", facts=[fact("start", "PUBLISH", topic="step.0")])
        operations = [first]
        for index in range(12):
            service, consumer, poller = f"service-{index}", f"write-{index}", f"poll-{index}"
            table = "outgoing"
            operations.append(self.retained(service, name=consumer, entry_kind="KAFKA_LISTENER",
                              trigger={"configuration": {"topics": [f"step.{index}"]}},
                              facts=[fact(consumer, "INSERT", table=table)]))
            operations.append(self.retained(service, name=poller, entry_kind="SCHEDULED_JOB",
                              facts=[fact(poller, "SELECT", table=table, occurrence="select"),
                                     fact(poller, "PUBLISH", topic=f"step.{index + 1}", occurrence="send"),
                                     self.poll_proof(poller)]))
        projected = self.project(*operations)[first[0]["id"]]
        boundary, = (row for row in projected if row["kind"] == "further-outbox-limit-boundary")
        self.assertEqual(boundary["status"], "PREPARATION_LIMIT")
        self.assertRegex(boundary["text"], r"cited poll root in `poll-[0-9]+\.md`")
        self.assertTrue(any(source.startswith("poll-") and source.endswith("-registration")
                            for source in boundary["sources"]))
        self.assertLessEqual(len(projected), self.edges["MAX_LINKS"])

    def test_focused_write_guards_apply_to_the_matching_topic_only(self):
        def fact(kind, sources, text, **metadata):
            return {"kind": kind, "sources": sources, "text": text, "authority": "RETAINED_PATH", **metadata}
        first = self.retained("publisher", facts=[fact(
            "publisher", ["publisher-body"], "Publish one message.",
            _edge={"kind": "PUBLISH", "site": "publisher-body", "topic": "books.ready"})])
        second = self.retained("alternate", facts=[fact(
            "publisher", ["alternate-body"], "Publish a different message.",
            _edge={"kind": "PUBLISH", "site": "alternate-body", "topic": "books.old"})])
        callback = self.retained("library", name="apply", entry_kind="KAFKA_LISTENER",
            trigger={"configuration": {"topics": ["books.ready", "books.old"]}}, facts=[
                fact("sql-write", ["apply-body"], "The source reaches a write.",
                     _edge={"kind": "UPDATE", "site": "apply-body", "table": "visible_books", "occurrence": "apply-write"}),
                fact("write_compatibility", ["apply-body"], "The old topic cannot reach this write in the retained paths.",
                     _writeFocus={"occurrence": "apply-write", "topic": "books.old", "compatibility": "INCOMPATIBLE"}),
                fact("write_request", ["apply-body"], "The qualified path reaches the write.", condition="revision is current",
                     _writeFocus={"occurrence": "apply-write", "topic": "books.ready"}),
                fact("write_lineage", ["apply-body"], "The write supplies the validated title.",
                     _writeFocus={"occurrence": "apply-write", "topic": "books.ready"}),
            ])
        read = self.retained("library", name="read", facts=[fact(
            "sql-read", ["read-body"], "Read the visible record.",
            _edge={"kind": "SELECT", "table": "visible_books", "site": "read-body"})])
        rows = self.project(first, second, callback, read)
        current = rows[first[0]["id"]]
        checked, = (row for row in current if row["kind"] == "message-storage-reader")
        self.assertIn("revision is current", checked["condition"])
        self.assertTrue(any(row["kind"] == "message-peer-write-detail" and "validated title" in row["text"]
                            for row in current))
        prior = rows[second[0]["id"]]
        self.assertTrue(any(row["status"] == "INCOMPATIBLE_IN_RETAINED_PATHS" for row in prior))
        self.assertFalse(any(row["kind"] in {"message-peer-effect", "message-storage-reader"} for row in prior))

    def test_registered_reader_publication_and_subscriber_require_matching_retained_keys(self):
        def effect(service, operation, **identity):
            site = service + "-body"
            return {"kind": "source-effect", "authority": "RETAINED_PATH", "sources": [site],
                    "text": "The retained source reaches the cited request.",
                    "_edge": {"kind": operation, "site": site, **identity}}

        request_write = effect("writer", "INSERT", table="pending_jobs", occurrence="insert")
        completion = {"kind": "http-local-completion", "authority": "RETAINED_PATH", "sources": ["writer-body"],
                      "text": "The handler resumes after the local write and returns HTTP 202.",
                      "condition": "the handler accepts the job",
                      "_httpCompletion": {"status": 202, "writeOccurrences": ["insert"],
                                          "writes": [{"occurrence": "insert", "table": "pending_jobs", "verb": "INSERT"}]}}
        writer = self.retained("catalog", name="writer", facts=[request_write, completion])
        operations = [effect("relay", "SELECT", table="pending_jobs", occurrence="select"),
                      effect("relay", "PUBLISH", topic="jobs.ready", occurrence="send")]
        relay = self.retained("catalog", name="relay", entry_kind="SCHEDULED_JOB",
                              facts=[*operations, {**self.poll_proof("relay"), "condition": "the poll observes a ready row"}])
        trigger = {"configuration": {"topics": ["jobs.ready"]}}
        listener = self.retained("index", name="listener", entry_kind="KAFKA_LISTENER", trigger=trigger,
                                 facts=[effect("listener", "INSERT", table="inbox")])
        reader = self.retained("index", name="reader", path="/indexed", facts=[effect("reader", "SELECT", table="inbox")])
        collision = self.retained("another-store", path="/other", facts=[effect("another-store", "SELECT", table="pending_jobs")])
        call = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units"}}
        front = self.retained("front", path="/start", events=[("call", "client.get()", call)])
        wrong = self.retained("wrong", entry_kind="KAFKA_LISTENER", trigger={"configuration": {"topics": ["jobs.other"]}})
        disabled = self.retained("disabled", entry_kind="KAFKA_LISTENER",
                                 trigger={"configuration": {"topics": ["jobs.ready"], "autoStartup": False}})
        result = self.project(front, writer, relay, listener, reader, collision, wrong, disabled)
        thread, = (fact for fact in result[writer[0]["id"]] if fact["kind"] == "storage-message-continuation")
        self.assertEqual(thread["sources"], ["writer-body", "relay-registration", "relay-body",
                                             "listener-registration", "listener-body"])
        self.assertIn("The selected row supplies the publication argument.", thread["text"])
        self.assertTrue(any(row["kind"] == "storage-message-effect" for row in result[writer[0]["id"]]))
        stop, = (row for row in result[writer[0]["id"]] if row["kind"] == "storage-peer-boundary")
        self.assertEqual(stop["status"], "UNKNOWN")
        self.assertIn("another-store-body", stop["sources"])
        complete, = (row for row in result[front[0]["id"]] if row["kind"] == "http-message-storage-reader")
        self.assertEqual(complete["sources"], ["front-call", "writer-registration", "writer-body",
                                             "relay-registration", "relay-body", "listener-registration",
                                             "listener-body", "reader-registration", "reader-body"])
        self.assertIn("Other payload fields and wire mapping remain open", complete["uncertainty"])
        activation, = (row for row in result[front[0]["id"]] if row["kind"] == "http-storage-poll-activation")
        self.assertIn("independently registered poller", activation["text"])
        self.assertIn("Any wait for downstream completion", activation["uncertainty"])
        self.assertIn("relative to an original HTTP response remain open", activation["uncertainty"])
        self.assertIn("relay-registration", activation["sources"])
        local, = (row for row in result[front[0]["id"]] if row["kind"] == "http-local-response-independent-poll")
        self.assertIn("returns HTTP 202", local["text"])
        self.assertIn("can complete independently", local["text"])
        self.assertEqual(local["condition"], "the handler accepts the job")
        self.assertTrue({"front-call", "writer-body", "relay-registration", "relay-body"} <= set(local["sources"]))
        proof = completion["_httpCompletion"]
        for update in ({"table": "unrelated"}, {"verb": "UPDATE"}, {"occurrence": "another-call"}):
            stale = {**completion, "_httpCompletion": {**proof, "writes": [{**proof["writes"][0], **update}]}}
            mismatch = self.retained("catalog", name="writer", facts=[request_write, stale])
            unqualified = self.project(front, mismatch, relay, listener, reader)[front[0]["id"]]
            self.assertTrue(any(row["kind"] == "http-storage-poll-activation" for row in unqualified))
            self.assertFalse(any(row["kind"] == "http-local-response-independent-poll" for row in unqualified))
        self.assertTrue(any(row["kind"] == "message-storage-reader" for row in result[relay[0]["id"]]))
        self.assertNotIn(wrong[0]["id"], result)
        self.assertEqual(result[disabled[0]["id"]][0]["status"], "DISABLED_IN_SOURCE")
        self.assertIn("message-peer-boundary", {row["kind"] for row in self.project(relay, wrong)[relay[0]["id"]]})
        unlinked = self.retained("catalog", name="relay", entry_kind="SCHEDULED_JOB",
                                 facts=[*operations, self.poll_proof("relay", "unrelated-send")])
        unrelated = self.project(writer, unlinked, listener, reader)
        kinds = {row["kind"] for row in unrelated[writer[0]["id"]]}
        self.assertEqual(kinds, {"storage-reader", "storage-message-boundary"})
        self.assertIn("message-peer", {row["kind"] for row in unrelated[unlinked[0]["id"]]})


class PreparedContractTests(unittest.TestCase):
    service = "registry"
    revision = "a" * 40

    @classmethod
    def setUpClass(cls):
        scripts = ROOT / "skills/codeclew/scripts"
        cls.contracts = runpy.run_path(str(scripts / "prepared_facts/clew_prepared_openapi.py"))
        cls.runner = runpy.run_path(str(scripts / "docs_run_prepared.py"), init_globals={
            name: D[name] for name in ("digest", "encode", "sha", "save")})

    def requirements(self):
        return [{"id": "entry", "entrypoint": "add", "entrySymbol": "Registry.Add",
                 "kind": "ENTRY", "entryKind": "HTTP_ENDPOINT", "method": "POST", "path": "/units/:name"}]

    def contract(self):
        return {
            "openapi": "3.1.0",
            "paths": {"/units/{unitName}": {
                "get": {"responses": {"404": {"description": "Unknown"}}},
                "post": {"responses": {
                    "202": {"description": "Accepted", "content": {"application/json": {
                        "schema": {"type": "object", "properties": {"ticketId": {"type": "string"}}}}}},
                    "409": {"description": "Conflict"},
                }},
            }},
        }

    def source(self, value, *, file="docs/registry.json", revision=None, authority="EXACT_SNAPSHOT_TEXT"):
        text = value if isinstance(value, str) else json.dumps(value, indent=2) + "\n"
        return {"id": "spec", "file": file, "service": self.service, "revision": revision or self.revision,
                "text": text, "startLine": 1, "endLine": len(text.splitlines()), "textDigest": D["sha"](text.encode()),
                "authority": authority, "references": ["spec-reference"],
                "occurrence": {"startByte": 0, "endByte": len(text.encode()), "blob": "owned-blob"}}

    def project(self, sources, *, operations=(), requirements=None):
        return self.contracts["project"](self.service, self.revision, requirements or self.requirements(), sources, operations)

    def test_declared_openapi_response_and_implementation_status_are_separate_author_facts(self):
        source = self.source(self.contract())
        overlay = self.project({"spec": source})
        declared, = overlay["entry"]
        self.assertIn("`POST /units/{unitName}`", declared["text"])
        self.assertIn("`202` (`application/json` object with declared fields `ticketId`)", declared["text"])
        self.assertIn("`409`", declared["text"])
        self.assertNotIn("`404`", declared["text"])
        span, = declared["witnesses"]
        method_line = next(index for index, line in enumerate(source["text"].splitlines(), 1) if '"post"' in line)
        self.assertEqual(span["startLine"], method_line)
        self.assertLess(span["endLine"], source["endLine"])
        self.assertEqual(span["textDigest"], D["sha"](source["text"].encode()[span["startByte"]:span["endByte"]]))
        metadata = {"file": "Registry.cs", "startLine": 5, "endLine": 9}
        implementation = {"kind": "http-outcome", "authority": "SUPPORTED_SOURCE", "text": "Returns an HTTP 201 result.",
                          "sources": ["handler"]}
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            (home / "view.md").write_text("Source header\n\n```csharp\nhandler body\n```\n")
            prepared = {"entries": {"entry": {"propositions": [implementation]}}, "citations": {"handler": metadata}, "bytes": 0}
            self.runner["merge_edges"](prepared, overlay, {"spec": {**source, "manifest": home / "spec/citations.json"}}, home)
            self.assertEqual([row["kind"] for row in prepared["entries"]["entry"]["propositions"]],
                             ["openapi-declared-response", "http-outcome"])
            author = self.runner["author_entries"](prepared["entries"], prepared["citations"])
            self.runner["workspace_views"](home, {"entry": ("view.md", "Page registry.md")}, author,
                                            prepared["citations"], {"view.md": "Registry.cs"}, 8192)
            delivered = (home / "view.md").read_text()
            self.assertIn("`202`", delivered)
            self.assertIn("HTTP 201", delivered)
            self.assertIn("runtime enforcement", delivered)
            self.assertIn("spec/citations.json", delivered)
            self.assertIn("handler body", delivered)

    def test_unrelated_dynamic_ambiguous_and_stale_contracts_do_not_choose_a_response(self):
        value = self.contract()
        invalid = [
            self.source({**value, "paths": {"/unrelated/{name}": value["paths"]["/units/{unitName}"]}}),
            self.source({**value, "paths": {"/units/{*name}": value["paths"]["/units/{unitName}"]}}),
            self.source({**value, "paths": {"/units/{name}": {"get": value["paths"]["/units/{unitName}"]["get"]}}}),
            self.source({"paths": value["paths"]}),
            self.source('{ "openapi": "3.1.0", "openapi": "3.1.1", "paths": {} }'),
            self.source(value, revision="b" * 40),
            {**self.source(value), "textDigest": D["sha"](b"stale spec")},
        ]
        for source in invalid:
            self.assertEqual(self.project({"spec": source}), {})
        ambiguous, = self.project({"first": self.source(value), "other": self.source(value, file="docs/other.json")})["entry"]
        self.assertEqual(ambiguous["status"], "AMBIGUOUS")
        self.assertNotIn("`202`", ambiguous["text"])

    def test_work_contract_uses_only_scoped_references(self):
        source = self.source(
            'openapi: 3.1.0\npaths:\n  /units/{name}:\n    post:\n      responses:\n        "204": {description: No body}\n',
            file="api/registered.yaml", authority="DECLARED_OPENAPI")
        operation = {"kind": "DEPENDENCY", "sourceReferences": ["spec-reference"], "record": {
            "kind": "CONTRACT_OPERATION", "service": self.service, "normalized": {
                "schema": "codeclew-documentation-contract-facts/1.0", "authority": "DECLARED_OPENAPI",
                "runtimeEnforcement": "UNVERIFIED", "declaredSource": source["file"], "method": "POST",
                "path": "/units/{name}", "entrypoint": "add", "sourceMapping": "SOURCE_ROUTE_MATCH_ONLY",
                "operation": {"responses": {"204": {"description": "No body"}}},
            },
        }}
        fact, = self.project({"spec": source}, operations=[operation])["entry"]
        self.assertIn("`204`", fact["text"])
        self.assertEqual(self.project({"spec": source}, operations=[{**operation, "sourceReferences": ["foreign-reference"]}]), {})

    def test_compiler_resource_fallback_uses_committed_regular_files_and_native_citations(self):
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            repo = home / "repository"
            repo.mkdir()
            contract = repo / "docs" / "api.json"
            contract.parent.mkdir()
            text = self.source(self.contract())["text"]
            contract.write_text(text)
            (repo / "docs" / "ignored.json").symlink_to(contract)
            for arguments in (["init", "--quiet"], ["add", "docs"],
                              ["-c", "user.name=Fixture", "-c", "user.email=fixture@example.invalid",
                               "commit", "--quiet", "-m", "Retain API declaration"]):
                subprocess.run(["git", "-C", str(repo), *arguments], check=True, capture_output=True)
            revision = subprocess.check_output(["git", "-C", str(repo), "rev-parse", "HEAD"], text=True).strip()
            contract.write_text('{"openapi":"3.1.0","paths":{}}')
            source = self.source(text, file="docs/api.json", revision=revision,
                                 authority="REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS")
            job = {"clew": "/trusted/clew", "root": str(home), "snapshot": "frozen"}
            calls = []

            def command(argv, output, label, **_):
                calls.append(label)
                if label == "contracts-native":
                    self.assertEqual(argv[argv.index("--snapshot") + 1], "frozen")
                    request = D["load"](Path(argv[argv.index("--native-inputs") + 1]))
                    self.assertEqual(request, [{"service": self.service, "files": ["docs/api.json"]}])
                    return {"snapshot": "frozen", "items": [{"kind": "SOURCE_NATIVE", "id": "spec", "record": source}]}
                self.assertEqual(label, "bind-native-contracts")
                self.assertIn("import-native", argv)
                self.assertEqual(argv[argv.index("--source") + 1], "spec")
                return {"status": "SELECTED_NATIVE_SOURCES_READY"}

            def citation_load(path):
                self.assertEqual(path.name, "citations.json")
                return {}, {"scope": {"service": self.service, "snapshot": "frozen"},
                            "outcomes": [{"source": source, "textKey": "digest"}], "texts": {"digest": text}}, {}, {
                                "native-spec": {"source": {key: value for key, value in source.items() if key != "text"}}}

            helper = runpy.run_path(str(ROOT / "skills/codeclew/scripts/docs_run_contract_sources.py"), init_globals={
                "save": D["save"], "command": command, "citation_load": citation_load,
                "Stop": D["Stop"], "CitationStop": D["M"]["C"]["Stop"]})
            retained = helper["pinned_sources"](job, self.service, revision, self.requirements(), str(repo), {},
                                                 {"Registry.cs"}, self.contracts, home, time.monotonic() + 15)
            self.assertEqual(calls, ["contracts-native", "bind-native-contracts"])
            declared, = self.contracts["project"](self.service, revision, self.requirements(), retained, ())["entry"]
            self.assertEqual(declared["sources"], ["native-spec"])
            self.assertIn("`202`", declared["text"])


if __name__ == '__main__':
    unittest.main()
