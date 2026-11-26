#!/usr/bin/env python3
"""Owned source/author protocol fixtures; no models, capture or public writes."""
import json
import hashlib
import os
import re
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


def sequence_rendering(html):
    figure = html.split('<figure class="clew-sequence">', 1)[1].split('</figure>', 1)[0]
    return figure.split('<details class="sequence-text" open>', 1)


class PageExecutionOrderTests(unittest.TestCase):
    def test_fresh_queue_prioritizes_large_packets_and_stably_orders_ties(self):
        execution = D['PX']
        specs = [('first', 'GUIDE', 24), ('light', 'OPERATION', 12), ('equal-one', 'OPERATION', 48),
                 ('equal-two', 'OPERATION', 48), ('map', 'PROCESS_MAP', 120)]
        slots = [{'key': key, 'kind': kind, 'page': key + '.md', 'prompt': key + '.txt'} for key, kind, _ in specs]
        plan = {'slots': slots}
        original = json.dumps(plan)
        job = {'budget': {'wallSeconds': 20, 'maxRepairs': 1},
               'author': {'executable': 'unused-fixture-author', 'model': 'fixture', 'reasoningEffort': 'low'}}
        payloads = {key: str(index).encode() * size for index, (key, _, size) in enumerate(specs)}
        keys = [slot['key'] for slot in slots]
        with tempfile.TemporaryDirectory() as directory:
            run = Path(directory)
            (run / 'inputs').mkdir()
            for key, payload in payloads.items():
                (run / 'inputs' / (key + '.txt')).write_bytes(payload)
            D['save'](run / 'state.json', {'attempts': [], 'authorStartedAt': None})

            def fake_launch(home, deadline, cancelled, attempt):
                key = attempt['slot']
                folder = execution['attempt_dir'](home, attempt)
                supplied = (folder / 'prompt.txt').read_bytes()
                if attempt['number']:
                    self.assertTrue(supplied.endswith(payloads[key]))
                    self.assertEqual(attempt['resumeThread'], 'thread-' + key)
                else:
                    self.assertEqual(supplied, payloads[key])
                output = json.dumps({'slot': key}).encode()
                execution['response_path'](home, attempt).write_bytes(output)
                turn = attempt['number'] + 1
                return {**attempt, 'status': 'COMPLETED', 'threadId': 'thread-' + key, 'exitCode': 0,
                        'usage': {'input_tokens': 20 * turn, 'cached_input_tokens': 0, 'output_tokens': turn},
                        'responseDigest': D['sha'](output)}

            def execute(repairs=None):
                return execution['execute_authors'](run, job, plan, None, lambda _: [], lambda _, value: value,
                                                    D['load'], repairs=repairs)

            with patch.dict(execution['execute_authors'].__globals__, {'launch': fake_launch}):
                state, responses = execute()
                self.assertEqual([turn['slot'] for turn in state['attempts']], ['map', 'equal-one', 'equal-two', 'first', 'light'])
                self.assertEqual(list(responses), keys)
                self.assertEqual(json.dumps(plan), original)
                self.assertEqual(execute()[0]['attempts'], state['attempts'])

                repairs = {key: {'code': 'FIXTURE_REPAIR'} for key in ('map', 'first')}
                resumed, responses = execute(repairs)
                self.assertEqual(resumed['attempts'][:len(keys)], state['attempts'])
                self.assertEqual([turn['slot'] for turn in resumed['attempts'][len(keys):]], ['first', 'map'])
                self.assertEqual(list(responses), keys)


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
control={{}}
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
failure=control.get('terminalFailure')
if not failure:counter.write_text(json.dumps(usage))
if mode=='controlled-page':
 if control.get('report', True) and not failure:
  print(json.dumps({{'type':'turn.completed','usage':usage}}),flush=True)
 (seed/(slot+'.waiting')).touch()
 expires=time.monotonic()+6
 for target in control.get('until', []):
  while not (seed/target).exists():
   if time.monotonic()>=expires:sys.exit(30)
   time.sleep(0.01)
 if control.get('delay'):time.sleep(control['delay'])
 if failure:
  print(json.dumps({{'type':'turn.failed','error':{{'message':failure}}}}),flush=True)
  sys.exit(1)
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

    def prepare_inline_fixture(self, body=None, *, repairs=0, extra_resource=False, project=None, run_name="inline-run",
                               extra_entries=()):
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
        if self.manifest.parent.exists():
            shutil.rmtree(self.manifest.parent)
        self.manifest, _, _ = F['fixture'](self.seed / 'reads', service=self.service, work='owned-work',
                                           record=source, authority=source['authority'])
        scope = self.inventory['services'][self.service]
        files = [source['file']]
        inventory_rows = [self.native_entry, *extra_entries, {'kind': 'THREAD_SCOPE'},
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
        run = self.root / run_name
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

    def isolated_fixture(self, *, repairs=0, wrong_literal=False, language='en', project=None, wall_seconds=30,
                         body=None, run_name="inline-run", extra_entries=(), map_routes=None):
        self.job.update(authorMode='ISOLATED_PAGES', language=language)
        self.job['budget']['wallSeconds'] = wall_seconds
        run, receipt, _, source = self.prepare_inline_fixture(body=body, repairs=repairs, project=project, run_name=run_name,
                                                               extra_entries=extra_entries)
        self.assertEqual(receipt['authorMode'], 'ISOLATED_PAGES')
        plan = D['load'](run / 'inputs/author-plan.json')
        packets = {row['kind']: json.loads((run / 'inputs' / row['prompt']).read_text().split('\n\n')[-1])
                   for row in plan['slots']}
        expected = {'CONTRACTS', 'OPERATION', 'DISCOVERY'}
        if 'GUIDE' in packets:
            expected.add('GUIDE')
        if not plan.get('operationOwnership') or map_routes is not None:
            expected.add('PROCESS_MAP')
        self.assertEqual(set(packets), expected)
        localized = language == 'ru'
        for slot in plan['slots']:
            kind = slot['kind']
            packet = json.loads((run / 'inputs' / slot['prompt']).read_text().split('\n\n')[-1])
            alias = next(iter(packet['citations']))
            text = '\u0411\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u0435 \u0434\u043e\u0431\u0430\u0432\u043b\u044f\u0435\u0442 \u044d\u043b\u0435\u043c\u0435\u043d\u0442.' if localized else 'The handler adds the accepted reservation.'
            claim = {'heading': '', 'text': text, 'cites': [alias], 'assessment': 'SUPPORTED', 'returns': []}
            answer = {'slot': slot['key'], 'reviewed': True, 'disposition': 'EXPLAINED', 'title': None,
                      'claims': [claim], 'participants': [], 'steps': [], 'discovery': None, 'starts': [],
                      'contracts': [], 'routes': [], 'limitations': []}
            if kind == 'GUIDE':
                if not slot.get('ownedContentOnly'):
                    answer['title'] = '\u0411\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u0435' if localized else 'Reservations'
                answer['claims'][0]['text'] = '\u0421\u0435\u0440\u0432\u0438\u0441 \u043f\u0440\u0438\u043d\u0438\u043c\u0430\u0435\u0442 \u0437\u0430\u044f\u0432\u043a\u0438.' if localized else 'The service accepts reservations.'
            if kind == 'CONTRACTS':
                answer['contracts'] = [{'scope': 'LOCAL', 'name': 'Reservation', 'cites': [alias],
                    'fields': [{'name': 'count', 'type': 'Int', 'meaning': '\u041a\u043e\u043b\u0438\u0447\u0435\u0441\u0442\u0432\u043e' if localized else 'Item count'}]}]
            if kind == 'DISCOVERY':
                answer.update(claims=[], discovery='NO_ADDITIONAL_STARTS')
            if kind == 'OPERATION':
                answer['steps'] = [
                    {'from': 'handler', 'to': 'caller', 'kind': 'return', 'label': '0', 'condition': 'count < 0'},
                    {'from': 'handler', 'to': 'handler', 'kind': 'note', 'label': 'accepted.add(count)', 'condition': None},
                ]
                if packet['returns'] or wrong_literal:
                    ref, = packet['returns']
                    cited = packet['returns'][ref]['cite']
                    literal_text = '\u0415\u0441\u043b\u0438 `count < 0`, \u043c\u0435\u0442\u043e\u0434 \u0432\u043e\u0437\u0432\u0440\u0430\u0449\u0430\u0435\u0442 `0`.' if localized else 'When `count < 0`, the method returns `0`.'
                    answer['claims'].append({**claim, 'text': literal_text, 'cites': [cited], 'returns': [ref]})
                    if wrong_literal:
                        D['save'](self.seed / (slot['key'] + '-repair.json'), answer)
                        answer['claims'][-1]['text'] = literal_text.replace('`0`', '`9`')
            if kind == 'PROCESS_MAP' and map_routes is not None:
                answer.update(claims=[], routes=map_routes(packet))
            elif kind == 'PROCESS_MAP':
                operation, = packet['context']['operations']
                answer['routes'] = [{'origin': operation['id'], 'participants': [], 'result': 'normal',
                    'title': '\u041f\u0440\u0438\u043d\u044f\u0442\u0438\u0435 \u0431\u0440\u043e\u043d\u0438\u0440\u043e\u0432\u0430\u043d\u0438\u044f' if localized else 'Accept a reservation', 'condition': 'count >= 0',
                    'summary': text, 'outcome': '\u0420\u0430\u0437\u043c\u0435\u0440 \u0441\u043f\u0438\u0441\u043a\u0430' if localized else 'Updated list size',
                    'terminal': operation['id'], 'boundary': 'return', 'recipient': 'Caller contract is not captured.',
                    'steps': [{'from': operation['id'], 'to': 'caller', 'kind': 'return', 'label': 'accepted.size',
                               'condition': 'count >= 0'}], 'cites': [alias]}]
            D['save'](self.seed / (slot['key'] + '-response.json'), answer)
        return run, plan, packets, source

    @staticmethod
    def planned_guide_sources(_owned, index, _service):
        return list(index['sources'])[:1]

    def legacy_local_route_fixture(self, *, local_route=True, once=False):
        """Freeze the former optional overview and route schema for compatibility tests."""
        current = D['P']['page_jobs']
        used = False

        def local_routes(requirements, indexes, *_args, **_kwargs):
            return {}, {row['service']: indexes[row['service']]['ready'][row['id']]['evidenceCandidates']
                        for row in requirements if row['kind'] == 'ENTRY'}, {}

        def plan(*args):
            nonlocal used
            if once and used:
                return current(*args)
            used = True
            hooks = {'guide_sources': self.planned_guide_sources}
            if local_route:
                hooks['route_evidence'] = local_routes
            with patch.dict(current.__globals__, hooks):
                result = current(*args)
            del result['navigationOverview']
            if local_route:
                del result['operationOwnership']
            schema = D['load'](ROOT / 'skills/codeclew/schemas/docs-run-page-author.schema.json')
            for slot in result['slots']:
                if slot.pop('ownedContentOnly', None):
                    if local_route:
                        slot.pop('transitionRoutes', None)
                    D['save'](args[3].parent / slot['schema'], D['P']['RESPONSE']['schema_for_slot'](schema, slot))
            return result

        self.enterContext(patch.dict(D['P'], {'page_jobs': plan}))

    def test_isolated_normal_entry_uses_distinct_authors_and_native_reader(self):
        run, plan, packets, source = self.isolated_fixture(language='ru')
        self.assertNotIn('GUIDE', packets)
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
        self.assertEqual({row['slot'] for row in state['attempts']}, {slot['key'] for slot in plan['slots']})
        html = Path(result['ready']) / 'html'
        operation_html = (html / self.entry['page'].replace('.md', '.html')).read_text()
        self.assertIn('POST /reservations', operation_html)
        self.assertIn('sequence-message return', operation_html)
        process = (html / 'process-map.html').read_text()
        self.assertIn('\u041a\u0430\u0440\u0442\u0430 \u043f\u0440\u043e\u0446\u0435\u0441\u0441\u043e\u0432', process)
        self.assertIn('href="' + self.entry['page'].replace('.md', '.html') + '#behavior">POST /reservations</a>', process)
        self.assertIn('id="behavior"', operation_html)
        self.assertIn('id="sequence"', operation_html)
        self.assertIn('<th>\u041f\u043e\u043b\u0435</th>', (html / 'reservations-contracts.html').read_text())
        overview = (html / 'reservations.html').read_text()
        self.assertIn('<h3 id="http">HTTP</h3>', overview)
        self.assertIn(self.entry['page'].replace('.md', '.html') + '#behavior', overview)
        self.assertIn('reservations-contracts.html', overview)
        self.assertIn('process-map.html', overview)
        self.assertEqual(list(html.glob('route-*.html')), [])

    def test_isolated_reuses_unchanged_slots_from_authored_overview_plan(self):
        self.legacy_local_route_fixture(local_route=False, once=True)
        old, target, prior = self.isolated_reuse_fixture()
        self.assertIn('GUIDE', {slot['kind'] for slot in prior['slots']})
        previous = {slot['key']: slot for slot in prior['slots']}
        plan = D['load'](target / 'inputs/author-plan.json')
        self.assertEqual({slot['kind'] for slot in plan['slots']}, {'CONTRACTS', 'OPERATION', 'DISCOVERY'})
        for slot in plan['slots']:
            self.assertEqual(slot, previous[slot['key']])
            self.assertEqual(D['PR']['slot_files'](old, previous[slot['key']]), D['PR']['slot_files'](target, slot))
        job, inventory, pins = D['frozen_inputs'](target)
        adoption = D['PR']['import_pages'](target, [old], job, inventory, pins, plan, D['load'](target / 'state.json'),
                                           D['inline_cli_flags'], D['P']['validate_response'])
        self.assertEqual((adoption['reusedPages'], adoption['remainingPages']), (3, 0))
        result = D['execute'](target)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual(result['reuse']['newAuthorInvocations'], 0)
        self.assertEqual(D['load'](target / 'state.json')['modelTokens'], 0)
        overview = (Path(result['ready']) / 'html/reservations.html').read_text()
        self.assertIn(self.entry['page'].replace('.md', '.html') + '#behavior', overview)
        self.assertNotIn('The service accepts reservations.', overview)

    def test_isolated_reader_displays_retained_start_name_without_rewriting_draft(self):
        opaque = self.service + '-0123456789abcdefabcd'
        entry = self.native_entry['record']['entrypoint']
        entry.update(id=opaque, kind='SCHEDULED_JOB')
        for language, symbol, trigger_name, title in (
            ('en', 'callable:example/jobs/QueueTimer.flushPending#jvm:()V', None, 'QueueTimer.flushPending'),
            ('ru', None, 'flushPending', 'flushPending'),
        ):
            with self.subTest(language=language):
                entry.update(symbol=symbol, trigger={'authority': 'OWNED_TEST_DECLARATION',
                                                     **({'name': trigger_name} if trigger_name else {})})
                run, plan, packets, _ = self.isolated_fixture(language=language, run_name='display-' + language)
                operation, = plan['entries']
                dest = operation['page'].replace('.md', '.html') + '#handler'
                starter = D['load'](run / 'inputs/starter/navigation.json')
                self.assertEqual(starter['services'][0]['starts'][0]['title'], opaque)
                self.assertEqual(packets['OPERATION']['context']['operation']['title'], symbol or opaque)
                inputs = run / 'inputs'
                inventory = D['load'](inputs / 'inventory.json')
                expected_labels = {operation['page'] + '#handler': {'original': opaque, 'title': title}}
                self.assertEqual(D['reader_entry_labels'](inventory, inputs), expected_labels)
                for row in inventory['requirements']:
                    row.pop('entrySymbol', None)
                    row.pop('trigger', None)
                self.assertEqual(D['reader_entry_labels'](inventory, inputs), expected_labels)
                result = D['execute'](run)
                self.assertEqual(result['status'], 'READY_PROMOTED', result)
                html = Path(result['ready']) / 'html'
                handler = (html / dest.split('#')[0]).read_text()
                overview = (html / (self.service + '.html')).read_text()
                process = (html / 'process-map.html').read_text()
                self.assertIn(f'<h1 id="{opaque}"><code>{title}</code></h1>', handler)
                self.assertIn('id="handler"', handler)
                self.assertIn(f'<figcaption>{title}</figcaption>', handler)
                self.assertIn(f'>{title}</a>', overview)
                self.assertIn(f'<a href="{dest.removesuffix("#handler")}#behavior">{title}</a>', process)
                self.assertIn('Scheduled' if language == 'en' else '\u041f\u043e \u0440\u0430\u0441\u043f\u0438\u0441\u0430\u043d\u0438\u044e', overview)
                draft = D['load'](run / 'workspace/draft/navigation.json')
                self.assertEqual(draft['services'][0]['starts'][0]['title'], opaque)
                self.assertEqual(draft['services'][0]['starts'][0]['target'], operation['page'] + '#handler')

    def isolated_reuse_fixture(self, *, official=False, routing=None, body=None):
        package = self.root / 'release'
        source = package / 'source/skills/codeclew'
        shutil.copytree(ROOT / 'skills/codeclew', source)
        kit_digest = D['PR']['skill_digest'](source, D['tree'](source, skill=True))
        caps = {'runtimeMode': 'RELEASE', 'runtimeKey': 'sha256:' + 'a' * 64,
                'runtimeManifestDigest': 'sha256:' + 'b' * 64, 'agentContract': {'skillDigest': kit_digest}}
        if official:
            self.seal_official_reuse_fixture(package, source, caps)
        else:
            D['save'](package / 'release.json', {'schema': 'codeclew-local-release-candidate/1.0',
                    **{key: caps[key] for key in ('runtimeKey', 'runtimeManifestDigest')}})
        launcher = package / 'bin/clew'
        launcher.parent.mkdir()
        navigation = self.root / 'source-navigation.json'
        launcher.write_text(self.clew.read_text().replace(
            "{'runtimeMode':'RELEASE','runtimeKey':'owned-test-only'}", repr(caps)).replace(
            "else:\n identity=", "elif args[:3]==['docs','work','read']:\n"
            f" from pathlib import Path\n print(Path({str(navigation)!r}).read_text())\nelse:\n identity="))
        launcher.chmod(0o700)
        self.job['clew'] = str(launcher)
        self.backend.write_text('#!' + sys.executable + '\n' + f"""
import hashlib,json,sys
from pathlib import Path
seed=Path({str(self.seed)!r})
out=Path(sys.argv[sys.argv.index('-o')+1])
packet=json.loads(sys.stdin.read().split('\\n\\n')[-1])
thread='fixture-'+hashlib.sha256(str(Path.cwd().parent).encode()).hexdigest()
if 'resume' in sys.argv:
    thread=sys.argv[sys.argv.index('resume')+1]
print(json.dumps({{'type':'thread.started','thread_id':thread}}))
print(json.dumps({{'type':'turn.started'}}))
routing=seed/'routing-slot.txt'
if routing.exists() and routing.read_text()==packet['slot'] and 'resume' not in sys.argv and 'selector' not in packet:
    message='workspace routing discovery failed'
    out.touch()
    for start in (2, 1):
        for index in range(start, 6):
            print(json.dumps({{'type':'error','message':f'Reconnecting... {{index}}/5 ({{message}})'}}))
        if start==2:
            print(json.dumps({{'type':'item.completed','item':{{'id':'item_0','type':'error',
                'message':'Falling back from WebSockets to HTTPS transport. '+message}}}}))
    print(json.dumps({{'type':'error','message':message}}))
    print(json.dumps({{'type':'turn.failed','error':{{'message':message}}}}))
    sys.exit(1)
repair='formatRepair' in packet
review_repair='reviewFormatRepair' in packet
initial=seed/(packet['slot']+'-review-first.json')
if 'selector' in packet and initial.is_file():
    suffix='-review-first.json'
elif 'selector' in packet or review_repair:
    suffix='-review.json'
else:
    suffix='-repair.json' if repair else '-response.json'
raw=(seed/(packet['slot']+suffix)).read_text()
out.write_text(raw)
multiplier=2 if repair or review_repair else 1
for row in (
    {{'type':'item.completed','item':{{'type':'agent_message','text':raw.removesuffix('\\n')}}}},
    {{'type':'turn.completed','usage':{{'input_tokens':20*multiplier,'cached_input_tokens':10*multiplier,
                                      'output_tokens':10*multiplier}}}},
): print(json.dumps(row))
""")
        old, plan, _, record = self.isolated_fixture(run_name='historical-run', repairs=int(routing == 'format-repair'),
                                                      body=body)
        D['save'](navigation, {'schema': 'codeclew-documentation-work-page/1.0', 'work': 'owned-work',
                              'snapshot': F['SNAPSHOT'], 'subject': 'service:' + self.service,
                              'contextDigest': 'owned-context', 'inputDigest': 'owned-input',
                              'membershipDigest': 'owned-membership', 'receiptDigest': 'owned-receipt',
                              'influenceCoverage': 'RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED',
                              'status': 'READY_WITH_LIMITATIONS', 'total': 1, 'nextCursor': None, 'omitted': [],
                              'items': [{'id': record['id'], 'kind': 'SOURCE_NAVIGATION', 'record': {
                                  'authority': 'NAVIGATION_ONLY', 'textDelivered': False,
                                  'recordDigest': F['digest'](record),
                                  'source': {key: value for key, value in record.items() if key != 'text'}}}]})
        if routing:
            slot = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
            key = slot['key']
            (self.seed / 'routing-slot.txt').write_text(key)
            if routing == 'format-repair':
                path = self.seed / (key + '-response.json')
                response = D['load'](path)
                D['save'](self.seed / (key + '-repair.json'), response)
                response['claims'][0]['cites'] = ['Unbound']
                D['save'](path, response)
            with patch.dict(D['PX']['execute_authors'].__globals__, {'SERVICE_RETRY_DELAY': 0}):
                with self.assertRaises(D['Stop']) as paused:
                    D['execute'](old)
                self.assertEqual(paused.exception.code, 'PAGE_AUTHOR_SERVICE_UNAVAILABLE')
                self.assertEqual(D['execute'](old)['status'], 'READY_PROMOTED')
        else:
            self.assertEqual(D['execute'](old)['status'], 'READY_PROMOTED')
        (old / '.runner.lock').touch()
        target, _, _, _ = self.isolated_fixture(run_name='new-run', body=body)
        return old, target, plan

    def seal_official_reuse_fixture(self, package, kit, caps):
        canonical = D['PR']['canonical']
        revision, tree_hash = 'e' * 40, 'f' * 40
        key = caps['runtimeKey']
        base = {'sourceRevision': revision, 'sourceTree': tree_hash}
        D['save'](package / 'release.json',
                  dict(base, schema='codeclew-release/2.0', status='PILOT_READY', version='v0.15.20',
                       architecture='arm64', operatingSystem='macos', profile='core'))

        def write(file, record):
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(canonical(record) + b'\n')
            file.chmod(0o400)

        files = [{'path': 'skills/codeclew/' + name, 'sha256': digest, 'size': (kit / name).stat().st_size,
                  'mode': 0o111 if (kit / name).stat().st_mode & 0o111 else 0}
                 for name, digest in D['tree'](kit, skill=True).items()]
        source = dict(base, schema='codeclew-release-source/1.0', manifestDigest='', files=files)
        source['manifestDigest'] = D['sha'](canonical(source))
        write(package / 'source/release-source.json', source)
        epoch = package / 'seed' / ('release-N-' + revision)
        capsule = epoch / 'parallel-state/v2/runtimes' / key.removeprefix('sha256:')
        runtime = dict(schema='codeclew-runtime-capsule/4.0', mode='RELEASE', runtimeKey=key, manifestDigest='',
                       artifacts={'clew': {'sha256': 'sha256:' + '1' * 64}},
                       workers={'java': {'treeHash': 'sha256:' + '2' * 64}})
        runtime['manifestDigest'] = D['sha'](canonical(runtime))
        write(capsule / 'runtime.json', runtime)
        (capsule / 'READY').write_text(key + '\n')
        caps['runtimeManifestDigest'] = runtime['manifestDigest']
        seed = dict(base, schema='codeclew-trusted-release-seed/2.0', mode='RELEASE', runtimeKey=key,
                    manifestDigest=runtime['manifestDigest'], sourcePayloadDigest=source['manifestDigest'],
                    stateEpoch=D['sha']((key + '\0' + revision).encode()),
                    artifactHashes={'clew': runtime['artifacts']['clew']['sha256']},
                    workerTreeHashes={'java': runtime['workers']['java']['treeHash']},
                    buildEvidenceDigests=['sha256:' + '3' * 64])
        seed['seedDigest'] = D['sha'](canonical(seed))
        write(epoch / 'seed.json', seed)

    def test_official_isolated_donor_uses_sealed_seed_and_rejects_tampering(self):
        old, target, plan = self.isolated_reuse_fixture(official=True)
        job, inventory, pins = D['frozen_inputs'](target)
        owner = D['PR']
        package = Path(job['clew']).parents[1]
        seed, = (package / 'seed').glob('release-N-*/seed.json')
        capsule, = seed.parent.glob('parallel-state/v2/runtimes/*/runtime.json')
        metadata = package / 'release.json'
        manifest = package / 'source/release-source.json'
        originals = {path: (path.read_bytes(), path.stat().st_mode & 0o777)
                     for path in (seed, capsule, manifest, metadata)}

        def import_pages():
            return owner['import_pages'](target, [old], job, inventory, pins, plan, D['load'](target / 'state.json'),
                                         D['inline_cli_flags'], D['P']['validate_response'])

        mutations = (
            (seed, lambda row: {**row, 'sourceTree': 'b' * 40}),
            (seed, lambda row: {**row, 'seedDigest': 'sha256:' + '0' * 64}),
            (capsule, lambda row: {**row, 'runtimeKey': 'sha256:' + '0' * 64}),
            (manifest, lambda row: {**row, 'manifestDigest': 'sha256:' + '0' * 64}),
            (metadata, lambda row: {**row, 'schema': 'unknown-release-layout'}),
        )
        for path, change in mutations:
            with self.subTest(path=path.name):
                original, mode = originals[path]
                path.chmod(0o600)
                path.write_bytes(owner['canonical'](change(json.loads(original))) + b'\n')
                path.chmod(mode)
                with self.assertRaises(D['Stop']) as rejected:
                    import_pages()
                self.assertEqual(rejected.exception.code, 'NO_ELIGIBLE_REUSED_PAGES')
                self.assertEqual(rejected.exception.details['sources'][0]['code'], 'REUSE_RELEASE_KIT_CHANGED')
                self.assertFalse((target / 'page-reuse.json').exists())
                path.chmod(0o600)
                path.write_bytes(original)
                path.chmod(mode)

        adopted = import_pages()
        self.assertEqual(adopted['reusedPages'], len(plan['slots']))
        proof = adopted['historicalRuns'][0]['releaseKit']
        self.assertEqual(proof['packageSchema'], 'codeclew-release/2.0')
        self.assertEqual(proof['seedDigest'], D['load'](seed)['seedDigest'])
        self.assertEqual(D['execute'](target)['status'], 'READY_PROMOTED')

    def test_reused_isolated_pages_mix_with_fresh_pages_and_reach_normal_reader(self):
        old, target, plan = self.isolated_reuse_fixture()
        source_files = D['tree'](old)
        changed = next(row for row in plan['slots'] if row['kind'] == 'OPERATION')
        schema = target / 'inputs' / changed['schema']
        D['save'](schema, {**D['load'](schema), 'title': 'Changed operation response schema'})
        pins = D['load'](target / 'pins.json')
        D['save'](target / 'pins.json', {**pins, 'inputFiles': D['tree'](target / 'inputs')})
        args = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'), 'reuse-pages',
                '--run-dir', str(target), '--reuse-from', str(old)]
        completed = subprocess.run(args, capture_output=True, text=True, check=True)
        adopted = json.loads(completed.stdout)
        self.assertEqual((adopted['reusedPages'], adopted['remainingPages']), (len(plan['slots'])-1, 1))
        self.assertEqual(adopted['rejected'], {'REUSE_PROMPT_OR_SCHEMA_CHANGED': 1})
        self.assertEqual(D['load'](target / 'state.json')['attempts'], [])
        result = D['execute'](target)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual((result['reuse']['reusedPages'], result['reuse']['newAuthorInvocations']),
                         (len(plan['slots'])-1, 1))
        report = D['load'](target / 'report.json')
        self.assertEqual(report['checks']['author']['reportedModelTokens'], 20)
        self.assertEqual(report['checks']['author']['reuse']['historicalRuns'][0]['reportedUsage']['budgetTokens'],
                         20 * len(plan['slots']))
        state = D['load'](target / 'state.json')
        self.assertEqual([row['slot'] for row in state['attempts']], [changed['key']])
        self.assertIn('POST /reservations', (Path(result['ready']) / 'html' / changed['page'].replace('.md', '.html')).read_text())
        self.assertEqual(source_files, D['tree'](old))

    def test_reuse_preserves_promoted_reviewed_output_across_a_smaller_plan(self):
        native_plan = D['P']['page_jobs']
        self.legacy_local_route_fixture()
        donor, reviewed, old_plan = self.isolated_reuse_fixture()
        slots = {slot['kind']: slot for slot in old_plan['slots']}
        operation = slots['OPERATION']
        key = operation['key']
        prior = next(row for row in D['load'](donor / 'state.json')['attempts'] if row['slot'] == key)
        response = D['load'](D['PX']['response_path'](donor, prior))
        claim = response['claims'][0]
        finding = {'slot': key, 'responseDigest': prior['responseDigest'],
                   'selector': {'field': 'claim', 'index': 0, 'action': 'replace'},
                   'issue': 'State the return value when the handler accepts a count.',
                   'sourceAliases': claim['cites']}
        findings = self.root / 'smaller-plan-findings.json'
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [finding]})
        prefix = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py')]

        def call(command, run, *arguments, status=0):
            finished = subprocess.run([*prefix, command, '--run-dir', str(run), *arguments], capture_output=True, text=True)
            self.assertEqual(finished.returncode, status, finished.stdout + finished.stderr)
            return json.loads(finished.stdout)

        call('reuse-pages', reviewed, '--reuse-from', str(donor), '--review-findings', str(findings))
        statement = 'For an accepted count, the handler returns the updated list size.'
        D['save'](self.seed / (key + '-review.json'), {'replacement': {**claim, 'text': statement}})
        D['author'](reviewed)

        def prepare_smaller(name):
            with patch.dict(D['P'], {'page_jobs': native_plan}):
                target, plan, _, _ = self.isolated_fixture(run_name=name)
            current = {slot['kind']: slot for slot in plan['slots']}
            self.assertEqual(set(slots) - set(current), {'GUIDE', 'PROCESS_MAP'})
            self.assertEqual(set(current), {'OPERATION', 'CONTRACTS', 'DISCOVERY'})
            for kind, slot in current.items():
                self.assertEqual(slot, slots[kind])
                self.assertEqual(D['PR']['slot_files'](target, slot), D['PR']['slot_files'](reviewed, slots[kind]))
            contracts = current['CONTRACTS']
            schema = target / 'inputs' / contracts['schema']
            D['save'](schema, {**D['load'](schema), 'title': 'Updated contract response schema'})
            pins = D['load'](target / 'pins.json')
            D['save'](target / 'pins.json', {**pins, 'inputFiles': D['tree'](target / 'inputs')})
            return target, plan

        target, plan = prepare_smaller('smaller-plan')
        args = ('--reuse-from', str(reviewed))
        rejected = call('reuse-pages', target, *args, status=2)
        self.assertEqual(rejected['code'], 'NO_ELIGIBLE_REUSED_PAGES')
        self.assertFalse((target / 'page-reuse.json').exists())
        accepted = call('run', reviewed)
        page = operation['page'].replace('.md', '.html')
        ready_page = Path(accepted['ready']) / 'html' / page
        original_bytes = ready_page.read_bytes()
        ready_page.write_bytes(original_bytes + b' ')
        rejected = call('reuse-pages', target, *args, status=2)
        self.assertEqual(rejected['details']['sources'][0]['code'], 'PAGE_REVIEW_DONOR_OUTPUT_CHANGED')
        ready_page.write_bytes(original_bytes)
        sealed = D['tree'](reviewed)

        adopted = call('reuse-pages', target, *args)
        self.assertEqual(adopted['reusedPages'], len(plan['slots']) - 1)
        self.assertEqual(adopted['rejected'], {'REUSE_PROMPT_OR_SCHEMA_CHANGED': 1})
        ledger = D['load'](target / 'page-reuse.json')
        self.assertEqual(set(ledger['pages']), {key, slots['DISCOVERY']['key']})
        proof = ledger['pages'][key]
        self.assertEqual(proof['responseDigest'], accepted['reuse']['reviewedSlots'][key]['responseDigest'])
        self.assertEqual(proof['correctionDigest'], D['load'](reviewed / 'state.json')['attempts'][0]['responseDigest'])
        contracts = slots['CONTRACTS']
        contract_response = self.seed / (contracts['key'] + '-response.json')
        revised = D['load'](contract_response)
        revised['contracts'][0]['fields'][0]['meaning'] = 'Number of items in this reservation'
        D['save'](contract_response, revised)
        result = call('run', target)
        self.assertEqual(result['reuse']['newAuthorInvocations'], 1)
        self.assertEqual([row['slot'] for row in D['load'](target / 'state.json')['attempts']], [contracts['key']])
        operation_html = (Path(result['ready']) / 'html' / page).read_text()
        self.assertIn(statement, operation_html)
        self.assertIn('id="behavior"', operation_html)
        self.assertEqual(sequence_rendering(operation_html)[1], sequence_rendering(original_bytes.decode())[1])
        self.assertIn('Number of items in this reservation',
                      (Path(result['ready']) / 'html' / Path(contracts['page']).with_suffix('.html')).read_text())
        self.assertEqual(D['tree'](reviewed), sealed)

        successor, _ = prepare_smaller('reuse-from-mixed-plan')
        adopted = call('reuse-pages', successor, '--reuse-from', str(target))
        self.assertEqual(adopted['reusedPages'], len(plan['slots']))
        self.assertEqual(D['load'](successor / 'page-reuse.json')['pages'][key]['responseDigest'], proof['responseDigest'])
        self.assertEqual(call('run', successor)['reuse']['newAuthorInvocations'], 0)

    def test_reuse_refuses_modified_seals_scope_and_stream_access(self):
        from collections import Counter
        old, target, plan = self.isolated_reuse_fixture()
        owner = D['PR']
        job, inventory, pins = D['frozen_inputs'](target)
        state = D['load'](old / 'state.json')
        attempt = state['attempts'][0]
        folder = D['PX']['attempt_dir'](old, attempt)
        key = attempt['slot']

        def selected(target_job=job, target_plan=plan):
            rejected = Counter()
            rows, _ = owner['candidates'](old, target, target_plan, target_job, inventory, pins,
                                         D['inline_cli_flags'], D['P']['validate_response'], {}, rejected)
            return rows, rejected

        for fields in ({'snapshot': 'another-snapshot'}, {'author': {**job['author'], 'model': 'another-author'}},
                       {'author': {**job['author'], 'reasoningEffort': 'low'}}):
            with self.subTest(fields=fields), self.assertRaises(D['Stop']) as caught:
                selected({**job, **fields})
            self.assertEqual(caught.exception.code, 'REUSE_RUN_SCOPE_MISMATCH')
        shifted = {**plan, 'slots': [{**row, 'sourceRegions': {}} if row['key'] == key else row for row in plan['slots']]}
        rows, rejected = selected(target_plan=shifted)
        self.assertNotIn(key, rows)
        self.assertEqual(rejected['REUSE_SLOT_SCOPE_CHANGED'], 1)
        response = folder / 'last-message.txt'
        raw = response.read_bytes()
        response.write_bytes(raw + b' ')
        rows, rejected = selected()
        self.assertNotIn(key, rows)
        self.assertEqual(rejected['REUSE_RESPONSE_CHANGED'], 1)
        response.write_bytes(raw)
        stream = folder / 'stdout.jsonl'
        events = stream.read_bytes().splitlines(keepends=True)
        stream.write_bytes(b''.join([*events[:2], b'{"type":"item.completed","item":{"type":"command_execution"}}\n', *events[2:]]))
        rows, rejected = selected()
        self.assertNotIn(key, rows)
        self.assertEqual(rejected['REUSE_AUTHOR_ACCESS_CHANGED'], 1)
        stream.write_bytes(b''.join(events))
        owner['import_pages'](target, [old], job, inventory, pins, plan, D['load'](target / 'state.json'),
                              D['inline_cli_flags'], D['P']['validate_response'])
        ledger = target / 'page-reuse.json'
        sealed = ledger.read_bytes()
        ledger.write_bytes(sealed + b' ')
        with self.assertRaises(D['Stop']) as caught:
            D['execute'](target)
        self.assertEqual(caught.exception.code, 'REUSE_LEDGER_CHANGED')
        self.assertEqual(D['load'](target / 'state.json')['attempts'], [])
        ledger.write_bytes(sealed)
        result = D['execute'](target)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        self.assertEqual((result['reuse']['reusedPages'], result['reuse']['newAuthorInvocations']), (len(plan['slots']), 0))
        self.assertEqual(D['load'](target / 'report.json')['checks']['author']['reportedModelTokens'], 0)

    def test_reuse_accepts_same_thread_routing_recovery_and_format_repair(self):
        old, target, plan = self.isolated_reuse_fixture(routing='format-repair')
        owner = D['PR']
        job, inventory, pins = D['frozen_inputs'](target)
        old_files = D['tree'](old)
        result = owner['import_pages'](target, [old], job, inventory, pins, plan, D['load'](target / 'state.json'),
                                      D['inline_cli_flags'], D['P']['validate_response'])
        self.assertEqual((result['reusedPages'], result['remainingPages'], result['rejected']),
                         (len(plan['slots']), 0, {}))
        key = (self.seed / 'routing-slot.txt').read_text()
        proof = D['load'](target / 'page-reuse.json')['pages'][key]
        self.assertEqual(proof['attempt'], 2)
        recovered = proof['routingRecovery']
        self.assertEqual([row['attempt'] for row in recovered['priorAttempts']], [0, 1])
        self.assertEqual(recovered['priorAttempts'][0]['attemptPromptDigest'],
                         recovered['priorAttempts'][1]['attemptPromptDigest'])
        accounting = D['load'](old / 'state.json')['tokenAccounting']
        for row in [*recovered['priorAttempts'], proof]:
            self.assertEqual(accounting['reportedUsage'][row['reportedUsageIndex']]['usage'],
                             row['historicalCumulativeUsage'])
            self.assertEqual(row['threadId'], proof['threadId'])
        self.assertIsNone(recovered['priorAttempts'][0]['historicalCumulativeUsage'])
        self.assertEqual(recovered['usage']['status'], 'UNAVAILABLE')
        self.assertIsNone(recovered['usage']['budgetTokens'])
        self.assertEqual(recovered['usage']['knownBudgetTokensLowerBound'], 40)
        self.assertEqual(D['execute'](target)['reuse']['newAuthorInvocations'], 0)
        self.assertEqual(D['tree'](old), old_files)

    def test_reuse_refuses_unproven_routing_recovery(self):
        from collections import Counter
        old, target, plan = self.isolated_reuse_fixture(routing='plain')
        owner = D['PR']
        job, inventory, pins = D['frozen_inputs'](target)
        key = (self.seed / 'routing-slot.txt').read_text()
        state_path = old / 'state.json'
        state = D['load'](state_path)
        failed, resumed = [row for row in state['attempts'] if row['slot'] == key]
        fail_dir, retry_dir = (D['PX']['attempt_dir'](old, row) for row in (failed, resumed))
        fail_stream, retry_stream = fail_dir / 'stdout.jsonl', retry_dir / 'stdout.jsonl'
        prompt = retry_dir / 'prompt.txt'
        output = retry_dir / 'last-message.txt'

        def with_attempt(row, fields, folder, *, refresh_usage=False):
            changed = dict(row, **fields)
            attempts = [changed if entry is row else entry for entry in state['attempts']]
            modified = dict(state, attempts=attempts)
            if refresh_usage:
                accounting = D['token_accounting'](attempts)
                modified.update(tokenAccounting=accounting, modelTokens=accounting['budgetTokens'],
                                usageAvailable=accounting['status'] == 'MEASURED')
            return {folder / 'result.json': D['encode'](changed), state_path: D['encode'](modified)}

        altered_prompt = b'A different packet.'
        forged = b'{"unproven":"output"}'
        altered_accounting = {**state['tokenAccounting'], 'reportedUsage': []}
        missing_usage = {'input_tokens': 20, 'output_tokens': 10}
        missing_usage_stream = retry_stream.read_bytes().splitlines(keepends=True)
        missing_usage_stream[-1] = json.dumps({'type': 'turn.completed', 'usage': missing_usage}).encode() + b'\n'
        mutations = (
            (with_attempt(failed, {'failure': {'code': 'PAGE_AUTHOR_PROCESS_FAILED', 'details': {}}}, fail_dir),
             'REUSE_ATTEMPT_NOT_ADMITTED'),
            ({fail_stream: fail_stream.read_bytes().replace(b'3/5', b'1/5', 1)}, 'REUSE_AUTHOR_STREAM_CHANGED'),
            ({fail_dir / 'last-message.txt': forged}, 'REUSE_RESPONSE_CHANGED'),
            ({retry_stream: retry_stream.read_bytes().replace(b'turn.completed', b'turn.failed')},
             'REUSE_AUTHOR_STREAM_CHANGED'),
            ({**with_attempt(resumed, {'usage': missing_usage}, retry_dir, refresh_usage=True),
              retry_stream: b''.join(missing_usage_stream)}, 'REUSE_AUTHOR_STREAM_CHANGED'),
            ({**with_attempt(resumed, {'responseDigest': D['sha'](forged)}, retry_dir), output: forged},
             'REUSE_AUTHOR_STREAM_CHANGED'),
            ({**with_attempt(resumed, {'promptDigest': D['sha'](altered_prompt)}, retry_dir), prompt: altered_prompt},
             'REUSE_ATTEMPT_PROMPT_CHANGED'),
            (with_attempt(resumed, {'resumeThread': 'another-session'}, retry_dir), 'REUSE_ATTEMPT_NOT_ADMITTED'),
            (with_attempt(resumed, {'argv': ['workspace-write' if arg == 'read-only' else arg
                                             for arg in resumed['argv']]}, retry_dir), 'REUSE_ATTEMPT_NOT_ADMITTED'),
            ({state_path: D['encode']({**state, 'tokenAccounting': altered_accounting})},
             'REUSE_HISTORICAL_COUNTERS_CHANGED'),
        )
        for index, (edits, expected) in enumerate(mutations):
            with self.subTest(index=index, expected=expected):
                originals = {file: file.read_bytes() for file in edits}
                try:
                    for file, raw in edits.items():
                        file.write_bytes(raw)
                    rejected = Counter()
                    rows, _ = owner['candidates'](old, target, plan, job, inventory, pins, D['inline_cli_flags'],
                                                  D['P']['validate_response'], {}, rejected)
                    self.assertNotIn(key, rows)
                    self.assertEqual(len(rows), len(plan['slots']) - 1)
                    self.assertEqual(rejected[expected], 1)
                finally:
                    for file, raw in originals.items():
                        file.write_bytes(raw)

    def test_source_bounded_review_after_mixed_recovery_and_routing_resume(self):
        historical, donor, plan = self.isolated_reuse_fixture()
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        key = operation['key']
        prior = next(row for row in D['load'](historical / 'state.json')['attempts'] if row['slot'] == key)
        stale_response = D['PX']['response_path'](historical, prior)
        raw = stale_response.read_bytes()
        stale_response.write_bytes(raw + b' ')
        prefix = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py')]

        def call(command, run, *arguments, status=0):
            completed = subprocess.run([*prefix, command, '--run-dir', str(run), *arguments], capture_output=True, text=True)
            self.assertEqual(completed.returncode, status, completed.stdout + completed.stderr)
            return json.loads(completed.stdout)

        reused = call('reuse-pages', donor, '--reuse-from', str(historical))
        self.assertEqual(reused['reusedPages'], len(plan['slots']) - 1)
        stale_response.write_bytes(raw)
        (self.seed / 'routing-slot.txt').write_text(key)
        self.assertEqual(call('run', donor, status=2)['code'], 'PAGE_AUTHOR_SERVICE_UNAVAILABLE')
        pending = D['load'](donor / 'state.json')
        pending['serviceRetryPending']['retryNotBefore'] = time.time() - 1
        D['save'](donor / 'state.json', pending)
        prior_site = call('run', donor)
        failure, success = D['load'](donor / 'state.json')['attempts']
        self.assertEqual(success['resumeThread'], failure['threadId'])
        self.assertTrue(success['serviceRetry'])
        sealed_donor = D['tree'](donor)
        target, _, packets, _ = self.isolated_fixture(run_name='successor')
        packet = packets['OPERATION']
        alias = next(iter(packet['citations']))
        old_response = D['load'](D['PX']['response_path'](donor, success))
        finding = {'slot': key, 'responseDigest': success['responseDigest'],
                   'selector': {'field': 'claim', 'index': 0, 'action': 'replace'},
                   'issue': 'The statement omits the successful return value.', 'sourceAliases': [alias]}
        findings = self.root / 'findings.json'
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [finding]})
        outage_stream = D['PX']['attempt_dir'](donor, failure) / 'stdout.jsonl'
        original_stream = outage_stream.read_bytes()
        outage_stream.write_bytes(original_stream.replace(b'3/5', b'1/5', 1))
        rejected = call('reuse-pages', target, '--reuse-from', str(donor), '--review-findings', str(findings), status=2)
        self.assertEqual(rejected['code'], 'REUSE_AUTHOR_STREAM_CHANGED')
        outage_stream.write_bytes(original_stream)
        adopted = call('reuse-pages', target, '--reuse-from', str(donor), '--review-findings', str(findings))
        self.assertEqual((adopted['reviewSlots'], adopted['reusedPages']), ([key], len(plan['slots']) - 1))
        origin, = adopted['historicalRuns']
        self.assertEqual(origin['priorReuse']['ledgerDigest'], pending['pageReuseDigest'])
        self.assertEqual(origin['reportedUsage']['knownBudgetTokensLowerBound'], 20)
        proof = D['load'](target / 'page-reuse.json')['pages'][key]
        self.assertEqual(proof['routingRecovery']['priorAttempts'][0]['threadId'], success['threadId'])
        sentence = 'On success, the handler returns the size of the accepted list.'
        authored = {**old_response['claims'][0], 'text': sentence, 'cites': [alias]}
        D['save'](self.seed / (key + '-review.json'), {'replacement': authored})
        result = call('run', target)
        self.assertEqual(result['status'], 'READY_PROMOTED')
        self.assertEqual(result['reuse']['newAuthorInvocations'], 1)
        self.assertEqual(result['reuse']['reviewedSlots'][key]['previousResponseDigest'], success['responseDigest'])
        state = D['load'](target / 'state.json')
        self.assertEqual(state['modelTokens'], 20)
        attempt, = state['attempts']
        self.assertEqual((attempt['slot'], attempt['resumeThread']), (key, None))
        self.assertNotEqual(attempt['threadId'], success['threadId'])
        folder = D['PX']['attempt_dir'](target, attempt)
        supplied = json.loads((folder / 'prompt.txt').read_text().split('\n\n')[-1])
        self.assertEqual(supplied['current'], old_response['claims'][0])
        self.assertEqual(supplied['citations'], {alias: packet['citations'][alias]})
        locations = packet['citations'][alias].get('regions', [packet['citations'][alias]['region']])
        self.assertEqual(supplied['sourceRanges'], {region: packet['sourceRanges'][region] for region in locations})
        self.assertEqual(D['tree'](donor), sealed_donor)
        old_html, new_html = [Path(r['ready']) / 'html' for r in (prior_site, result)]
        revised_page = operation['page'].replace('.md', '.html')
        self.assertIn(sentence, (new_html / revised_page).read_text())
        self.assertNotIn(sentence, (old_html / revised_page).read_text())
        self.assertEqual((new_html / 'process-map.html').read_bytes(), (old_html / 'process-map.html').read_bytes())
        old_pages, new_pages = [D['load'](run / 'workspace/draft/original.json')['pages'] for run in (donor, target)]
        self.assertEqual([page for page in new_pages if page['page'] != operation['page']],
                         [page for page in old_pages if page['page'] != operation['page']])

        next_run, _, _, _ = self.isolated_fixture(run_name='sequence-successor')
        revised_digest = result['reuse']['reviewedSlots'][key]['responseDigest']
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [{
            **finding, 'responseDigest': revised_digest,
            'selector': {'field': 'step', 'index': len(old_response['steps']), 'action': 'insert'},
            'issue': 'On the successful branch the trace omits the final return.',
        }]})
        next_adoption = call('reuse-pages', next_run, '--reuse-from', str(target), '--review-findings', str(findings))
        self.assertIn('priorReuse', next_adoption['historicalRuns'][0]['priorReuse']['origins'][0])
        D['save'](self.seed / (key + '-review.json'), {'replacement': {
            'from': 'handler', 'to': 'caller', 'kind': 'return', 'label': 'List size', 'condition': 'count >= 0',
        }})
        next_result = call('run', next_run)
        self.assertEqual(next_result['reuse']['reviewedSlots'][key]['previousResponseDigest'], revised_digest)
        revised = (Path(next_result['ready']) / 'html' / revised_page).read_text()
        self.assertIn(sentence, revised)
        self.assertIn('List size', revised)

    def test_source_bounded_review_refuses_unbound_selector_digest_alias_and_source(self):
        donor, target, plan = self.isolated_reuse_fixture()
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        key = operation['key']
        current = next(row for row in D['load'](donor / 'state.json')['attempts'] if row['slot'] == key)
        finding = {'slot': key, 'responseDigest': current['responseDigest'],
                   'selector': {'field': 'claim', 'index': 0, 'action': 'replace'},
                   'issue': 'Describe the condition for the returned value.',
                   'sourceAliases': [next(iter(operation['citations']))]}
        path = self.root / 'findings.json'
        args = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'), 'reuse-pages',
                '--run-dir', str(target), '--reuse-from', str(donor), '--review-findings', str(path)]

        def rejected(row, code):
            D['save'](path, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [row]})
            completed = subprocess.run(args, capture_output=True, text=True)
            self.assertEqual(completed.returncode, 2, completed.stdout + completed.stderr)
            self.assertEqual(json.loads(completed.stdout)['code'], code)
            self.assertFalse((target / 'page-reuse.json').exists())
            self.assertEqual(D['load'](target / 'state.json')['attempts'], [])

        for changes, code in (
                ({'slot': 'unassigned-slot'}, 'PAGE_REVIEW_SLOT_OUTSIDE_RUN'),
                ({'responseDigest': 'sha256:' + '0' * 64}, 'PAGE_REVIEW_RESPONSE_MISMATCH'),
                ({'selector': {'field': 'title'}}, 'PAGE_REVIEW_SELECTOR_OUTSIDE_SLOT'),
                ({'selector': {**finding['selector'], 'index': 99}}, 'PAGE_REVIEW_INDEX_OUTSIDE_SLOT'),
                ({'sourceAliases': ['Soutside']}, 'PAGE_REVIEW_SOURCE_OUTSIDE_SLOT')):
            with self.subTest(code=code):
                rejected({**finding, **changes}, code)
        packet = donor / 'inputs' / operation['prompt']
        text = packet.read_text()
        changed = text.replace('accepted.add(count)', 'accepted.remove(count)')
        self.assertNotEqual(changed, text)
        packet.write_text(changed)
        rejected(finding, 'REUSE_SOURCE_INPUTS_CHANGED')
        packet.write_text(text)

        D['save'](path, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [finding]})
        subprocess.run(args, capture_output=True, text=True, check=True)
        D['save'](self.seed / (key + '-review.json'), {'replacement': {
            'heading': '', 'text': 'A replacement with an unreviewed citation.',
            'cites': ['Soutside'], 'assessment': 'SUPPORTED', 'returns': [],
        }})
        completed = subprocess.run([*args[:4], 'run', '--run-dir', str(target)], capture_output=True, text=True)
        self.assertEqual(completed.returncode, 2, completed.stdout + completed.stderr)
        self.assertEqual(json.loads(completed.stdout)['details']['failure']['code'], 'PAGE_REVIEW_CORRECTION_OUTSIDE_SOURCE')
        self.assertEqual([row['slot'] for row in D['load'](target / 'state.json')['attempts']], [key])
        self.assertFalse((target / 'ready').exists())

    def test_recover_completed_review_replays_plan_order_and_preserves_original_seals(self):
        _, donor, plan = self.isolated_reuse_fixture()
        slots = [slot['key'] for slot in plan['slots']]
        self.assertNotEqual(slots, sorted(slots))
        for number, key in enumerate(slots):
            response = self.seed / (key + '-response.json')
            D['save'](response, {**D['load'](response), 'limitations': [f'Limitation {number}.']})
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        key = operation['key']
        prefix = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py')]

        def run_command(command, run, *arguments, status=0):
            completed = subprocess.run([*prefix, command, '--run-dir', str(run), *arguments], capture_output=True, text=True)
            self.assertEqual(completed.returncode, status, completed.stdout + completed.stderr)
            return json.loads(completed.stdout)

        self.assertEqual(run_command('run', donor)['status'], 'READY_PROMOTED')
        attempt = next(row for row in D['load'](donor / 'state.json')['attempts'] if row['slot'] == key)
        prior = D['load'](D['PX']['response_path'](donor, attempt))
        finding = {'slot': key, 'responseDigest': attempt['responseDigest'],
                   'selector': {'field': 'claim', 'index': 0, 'action': 'replace'},
                   'issue': 'State the successful return value.', 'sourceAliases': [prior['claims'][0]['cites'][0]]}
        findings = self.root / 'recovery-findings.json'
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [finding]})
        reviewed, _, _, _ = self.isolated_fixture(run_name='completed-review')
        recovered, _, _, _ = self.isolated_fixture(run_name='recovered-review')
        run_command('reuse-pages', reviewed, '--reuse-from', str(donor), '--review-findings', str(findings))
        sentence = 'On success, the handler returns the size of the accepted list.'
        D['save'](self.seed / (key + '-review.json'), {'replacement': {**prior['claims'][0], 'text': sentence}})
        state = D['author'](reviewed)
        draft = reviewed / 'workspace/draft'
        job, inventory, _ = D['frozen_inputs'](reviewed)
        _, review = D['isolated_authorship'](reviewed, job, inventory, state, draft)
        self.assertEqual(set(review['reviewedSlots']), {key})
        self.assertEqual(D['load'](draft / 'self-review.json')['unresolved'],
                         [f'Limitation {number}.' for number in range(len(slots))])

        # Reproduce the older verifier's ledger insertion order and failed report.
        assemble = D['P']['assemble_pages']

        def old_assemble(job, inventory, plan, values, *args):
            artifacts = assemble(job, inventory, plan, values, *args)
            artifacts['self-review.json']['unresolved'] = list(dict.fromkeys(
                limit for value in values.values() for limit in value['limitations']))
            return artifacts

        with patch.dict(D['P'], {'assemble_pages': old_assemble}):
            report = D['verify'](reviewed)
        self.assertEqual(report['checks']['author']['code'], 'PAGE_REVIEW_DRAFT_CHANGED')
        self.assertFalse((reviewed / 'ready').exists())
        source_files = D['tree'](reviewed)
        args = ('--reuse-from', str(reviewed), '--recover-reviewed')
        extra = draft / 'unexpected.txt'
        extra.write_text('Out of scope')
        self.assertEqual(run_command('reuse-pages', recovered, *args, status=2)['code'], 'PAGE_REVIEW_DRAFT_CHANGED')
        state_file = reviewed / 'state.json'
        original_state = state_file.read_bytes()
        D['save'](state_file, {**state, 'pageDraftFiles': D['tree'](draft)})
        self.assertEqual(run_command('reuse-pages', recovered, *args, status=2)['code'], 'PAGE_REVIEW_DONOR_HISTORY_CHANGED')
        state_file.write_bytes(original_state)
        extra.unlink()
        saved_attempt, = state['attempts']
        saved_response = D['PX']['response_path'](reviewed, saved_attempt)
        raw = saved_response.read_bytes()
        saved_response.write_bytes(raw + b' ')
        self.assertEqual(run_command('reuse-pages', recovered, *args, status=2)['code'], 'REUSE_RESPONSE_CHANGED')
        saved_response.write_bytes(raw)
        self.assertFalse((recovered / 'page-reuse.json').exists())

        adoption = run_command('reuse-pages', recovered, *args)
        self.assertEqual((adoption['reusedPages'], adoption['remainingPages']), (len(slots), 0))
        source, = adoption['historicalRuns']
        self.assertEqual(source['completedReview']['reviewedSlots'], [key])
        self.assertEqual(source['reportedUsage']['budgetTokens'], 20)
        with patch.dict(D['execute_isolated'].__globals__,
                        {'verify': lambda *args, **kwargs: {'status': 'FAILED', 'checks': {
                            'output': {'status': 'FAIL', 'code': 'MISSING_REQUIRED_PAGE', 'details': {'page': operation['page']}},
                        }}}):
            self.assertEqual(D['execute'](recovered)['status'], 'FAILED')
        self.assertEqual(D['load'](recovered / 'state.json')['attempts'], [])
        promotion = run_command('run', recovered)
        self.assertEqual(promotion['status'], 'READY_PROMOTED')
        self.assertEqual(promotion['reuse']['newAuthorInvocations'], 0)
        self.assertEqual(D['load'](recovered / 'state.json')['pageDraftFiles'], state['pageDraftFiles'])
        self.assertEqual(D['tree'](reviewed), source_files)
        self.assertIn(sentence, (Path(promotion['ready']) / 'html' / operation['page'].replace('.md', '.html')).read_text())

        successor, _, _, _ = self.isolated_fixture(run_name='review-after-recovery')
        ledger = D['load'](recovered / 'page-reuse.json')
        corrected = ledger['pages'][key]
        self.assertEqual(corrected['correctionDigest'], saved_attempt['responseDigest'])
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [
            {**finding, 'responseDigest': corrected['responseDigest']}]})
        next_review = run_command('reuse-pages', successor, '--reuse-from', str(recovered), '--review-findings', str(findings))
        self.assertEqual(next_review['reviewSlots'], [key])

    def test_review_updates_one_route_and_discovery_step_by_published_identity(self):
        self.legacy_local_route_fixture()
        body = ('fun reserve(count: Int): Int {\n'
                '    if (count < 0) return 0\n'
                '    accepted.add(count)\n'
                '    events.on("note.ready") { if (it.isReady) accept(it) }\n'
                '    events.on("note.closed") { if (it.isClosed) archive(it) }\n'
                '    return accepted.size\n}\n')
        _, donor, plan = self.isolated_reuse_fixture(body=body)
        slots = {slot['kind']: slot for slot in plan['slots']}
        process, discovery = [slots[kind] for kind in ('PROCESS_MAP', 'DISCOVERY')]
        process_path, discovery_path = [self.seed / (slot['key'] + '-response.json') for slot in (process, discovery)]
        map_response, discovery_response = [D['load'](path) for path in (process_path, discovery_path)]
        main, = map_response['routes']
        route_step = main['steps'][0]
        route_step['condition'] = 'count > 0'
        main['outcome'] = 'Every count changes the accepted list.'
        map_response['routes'].append({**main, 'title': 'Reject a negative count', 'condition': 'count < 0',
                                        'outcome': 'The handler returns zero without adding to the list.',
                                        'steps': [{**route_step, 'label': '0', 'condition': 'count < 0'}]})
        D['save'](process_path, map_response)
        discovery_alias = next(iter(discovery['citations']))
        start_step = {'from': 'handler', 'to': 'handler', 'kind': 'note', 'label': 'Accept the note',
                      'condition': 'it.isClosed'}
        def start(title, event, claim, step):
            return {'kind': 'OTHER', 'title': title, 'initiator': 'Event hub', 'method': None, 'path': None,
                    'startLabel': event, 'startKind': 'async', 'disposition': 'EXPLAINED',
                    'claims': [{'heading': '', 'text': claim, 'cites': [discovery_alias], 'assessment': 'SUPPORTED'}],
                    'participants': [], 'steps': [step]}

        discovery_response.update(discovery='ADDITIONAL_STARTS', starts=[
            start('Ready note', 'note.ready', 'The ready handler accepts a ready note.', start_step),
            start('Closed note', 'note.closed', 'The closed handler archives a closed note.',
                  {**start_step, 'label': 'Archive the note'}),
        ])
        D['save'](discovery_path, discovery_response)
        prefix = [sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py')]

        def call(command, run, *arguments, status=0):
            completed = subprocess.run([*prefix, command, '--run-dir', str(run), *arguments], capture_output=True, text=True)
            self.assertEqual(completed.returncode, status, completed.stdout + completed.stderr)
            return json.loads(completed.stdout)

        old_site = call('run', donor)
        old_draft = donor / 'workspace/draft'
        routes = D['load'](old_draft / 'routes.json')['routes']
        starts = D['load'](old_draft / 'self-review.json')['requirements'][discovery['requirement']]['discoveredEntries']
        route, neighbor_route = routes
        discovered, neighbor_start = starts
        responses = {row['slot']: row['responseDigest'] for row in D['load'](donor / 'state.json')['attempts']}
        target, _, _, _ = self.isolated_fixture(run_name='nested-successor', body=body, repairs=2)
        findings = self.root / 'nested-findings.json'
        route_finding = {'slot': process['key'], 'responseDigest': responses[process['key']],
                         'selector': {'field': 'routeStep', 'routeId': route['id'], 'index': 0, 'action': 'replace'},
                         'issue': 'The route step wrongly excludes zero.', 'sourceAliases': main['cites']}
        discovery_finding = {'slot': discovery['key'], 'responseDigest': responses[discovery['key']],
                             'selector': {'field': 'startStep', 'startId': discovered['id'], 'index': 0, 'action': 'replace'},
                             'issue': 'The ready callback step names the condition from the closed callback.',
                             'sourceAliases': [discovery_alias]}
        reuse = ['--reuse-from', str(donor), '--review-findings', str(findings)]
        for route_id, start_id in (('route-unavailable', discovered['id']), (route['id'], 'f99-1')):
            D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [
                {**route_finding, 'selector': {**route_finding['selector'], 'routeId': route_id}},
                {**discovery_finding, 'selector': {**discovery_finding['selector'], 'startId': start_id}},
            ]})
            rejected = call('reuse-pages', target, *reuse, status=2)
            self.assertEqual(rejected['code'], 'PAGE_REVIEW_TRACE_OUTSIDE_SLOT')
            self.assertEqual(D['load'](target / 'state.json')['attempts'], [])
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [route_finding, discovery_finding]})
        call('reuse-pages', target, *reuse)
        long_condition = 'count >= 0 and ' + ' and '.join(['the count is nonnegative'] * 12)
        D['save'](self.seed / (process['key'] + '-review-first.json'),
                  {'replacement': {**route_step, 'condition': long_condition}})
        D['save'](self.seed / (process['key'] + '-review.json'),
                  {'replacement': {**route_step, 'condition': 'count >= 0'}})
        D['save'](self.seed / (discovery['key'] + '-review.json'),
                  {'replacement': {**start_step, 'condition': 'it.isReady'}})
        reviewed = call('run', target)
        self.assertEqual(reviewed['reuse']['newAuthorInvocations'], 3)
        self.assertEqual(set(reviewed['reuse']['reviewedSlots']), {process['key'], discovery['key']})
        state = D['load'](target / 'state.json')
        self.assertEqual(state['modelTokens'], 60)
        first, repair = [row for row in state['attempts'] if row['slot'] == process['key']]
        sibling, = [row for row in state['attempts'] if row['slot'] == discovery['key']]
        self.assertLess(sibling['startedAt'], first['completedAt'])
        self.assertTrue(all(row['status'] == 'COMPLETED' for row in (first, sibling, repair)))
        self.assertEqual((repair['number'], repair['resumeThread'], repair['threadId']), (1, first['threadId'], first['threadId']))
        first_path = D['PX']['attempt_dir'](target, first)
        retry_path = D['PX']['attempt_dir'](target, repair)
        retry_packet = D['load'](retry_path / 'prompt.txt')
        self.assertEqual(retry_packet['reviewFormatRepair'][0]['path'], '/routes/0/steps/0/condition')
        self.assertEqual(retry_packet['reviewFormatRepair'][0]['maxCharacters'], 240)
        self.assertNotIn('sourceRanges', retry_packet)
        self.assertNotIn(long_condition, (retry_path / 'prompt.txt').read_text())
        schema_file = 'docs-run-page-author.schema.json'
        self.assertEqual((first_path / schema_file).read_bytes(), (retry_path / schema_file).read_bytes())
        self.assertIn('240 characters', D['load'](first_path / schema_file)['$defs']['step']['properties']['condition']['description'])
        for row in state['attempts']:
            if row is repair:
                continue
            folder = D['PX']['attempt_dir'](target, row)
            packet = json.loads((folder / 'prompt.txt').read_text().split('\n\n')[-1])
            if row['slot'] == process['key']:
                self.assertEqual(packet['current'], route_step)
                self.assertIn('240 characters', packet['replacementContract']['condition'])
                self.assertNotIn('Reject a negative count', json.dumps(packet))
            else:
                self.assertEqual(packet['current'], start_step)
                self.assertNotIn('Archive the note', json.dumps(packet))

        old_html, new_html = [Path(site['ready']) / 'html' for site in (old_site, reviewed)]
        for page, action, label, old_condition, new_condition in (
                (route['narrative']['page'], 'Return', 'accepted.size', 'count > 0', 'count >= 0'),
                (Path(discovered['page']).with_suffix('.html').name, 'Work', 'Accept the note', 'it.isClosed', 'it.isReady')):
            previous, corrected = [sequence_rendering((root / page).read_text()) for root in (old_html, new_html)]
            self.assertEqual(previous[0], corrected[0])
            guard = re.search(rf'href="#(clew-sequence-1-step-2)">2\. {action}</a><small>Conditional</small>', corrected[0])
            self.assertIsNotNone(guard)
            for (_, trace), condition in ((previous, old_condition), (corrected, new_condition)):
                step = re.search(rf'<li id="{guard[1]}">(.*?)</li>', trace)
                self.assertIsNotNone(step)
                self.assertIn(f'</span>{label}<p>{condition}</p>', step[1])
            self.assertNotIn(f'<p>{old_condition}</p>', corrected[1])
        for page in (neighbor_route['narrative']['page'], Path(neighbor_start['page']).with_suffix('.html').name):
            self.assertEqual((old_html / page).read_bytes(), (new_html / page).read_bytes())

        next_run, _, _, _ = self.isolated_fixture(run_name='format-successor', body=body)
        revised_digest = reviewed['reuse']['reviewedSlots'][process['key']]['responseDigest']
        outcome_finding = {**route_finding, 'responseDigest': revised_digest,
                           'selector': {'field': 'routeOutcome', 'routeId': route['id']},
                           'issue': 'The outcome attributes the successful effect to every count.'}
        reuse = ['--reuse-from', str(target), '--review-findings', str(findings)]
        for invalid, expected in (
                ({'selector': {'field': 'routeOutcome', 'routeId': 'route-foreign'}}, 'PAGE_REVIEW_TRACE_OUTSIDE_SLOT'),
                ({'sourceAliases': ['Sforeign']}, 'PAGE_REVIEW_SOURCE_OUTSIDE_SLOT')):
            D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [{**outcome_finding, **invalid}]})
            self.assertEqual(call('reuse-pages', next_run, *reuse, status=2)['code'], expected)
            self.assertEqual(D['load'](next_run / 'state.json')['attempts'], [])
        D['save'](findings, {'schema': D['PAGE_REVIEW']['SCHEMA'], 'findings': [outcome_finding]})
        prompt = retry_path / 'prompt.txt'
        original = prompt.read_bytes()
        prompt.write_bytes(original + b'\n')
        rejected = call('reuse-pages', next_run, *reuse, status=2)
        self.assertEqual(rejected['code'], 'REUSE_ATTEMPT_PROMPT_CHANGED')
        prompt.write_bytes(original)
        call('reuse-pages', next_run, *reuse)
        recovery = D['load'](next_run / 'page-reuse.json')['pages'][process['key']]['formatRecovery']
        proof, = recovery['priorAttempts']
        self.assertEqual(proof['responseDigest'], first['responseDigest'])
        self.assertEqual(recovery['usage']['budgetTokens'], 40)
        (self.seed / (process['key'] + '-review-first.json')).unlink()
        outcome = 'When the count is nonnegative, the handler returns the updated list size.'
        D['save'](self.seed / (process['key'] + '-review.json'), {'replacement': outcome})
        next_result = call('run', next_run)
        self.assertEqual(next_result['reuse']['newAuthorInvocations'], 1)
        next_html = Path(next_result['ready']) / 'html'
        route_page = route['narrative']['page']
        outcome_section = (next_html / route_page).read_text().split('id="outcome"', 1)[1]
        self.assertIn(outcome, outcome_section)
        self.assertNotIn(main['outcome'], outcome_section)
        self.assertEqual(sequence_rendering((next_html / route_page).read_text()),
                         sequence_rendering((new_html / route_page).read_text()))
        attempt, = D['load'](next_run / 'state.json')['attempts']
        schema = D['load'](D['PX']['attempt_dir'](next_run, attempt) / schema_file)
        value, unset = schema['properties']['replacement']['anyOf']
        self.assertEqual((value['type'], unset['type']), ('string', 'null'))
        self.assertIn('512 characters', value['description'])
        before_draft, after_draft = [run / 'workspace/draft' for run in (target, next_run)]
        before = D['load'](before_draft / 'original.json')
        for page in before['pages']:
            if page['page'] == route['id'] + '.md':
                fragment, = [part for part in page['fragments'] if part['text'] == main['outcome'] + '\n\n']
                fragment['text'] = outcome + '\n\n'
        self.assertDictEqual(D['load'](after_draft / 'original.json'), before)
        catalog = D['load'](before_draft / 'routes.json')
        catalog['routes'][0]['outcomes'][0]['title'] = outcome
        self.assertDictEqual(D['load'](after_draft / 'routes.json'), catalog)

    def isolated_decoding_outcome(self, language):
        body = ('fun reserve(count: Int): Int {\n'
                '    val receipt = reserveStock(count)\n'
                '    try { return parser.decode(receipt) } catch (error: DecodeException) { return 502 }\n'
                '}\n'
                'fun reserveStock(count: Int): String {\n'
                '    ledger.commit(count)\n'
                '    return receipt()\n'
                '}\n')
        fact_text = ('After `reserveStock` takes its commit-before-receipt path, '
                     'failing to decode that receipt can return `502` to the caller.')
        premise = 'The receipt from the committing branch is malformed.'
        limit = 'The decoder failure does not establish rollback of the ledger write.'

        def project(process, context, *, node):
            if process['root'] is None:
                return [], {}
            source, = context['sourceRanges']
            fact = {'kind': 'handled-exception-continuation', 'authority': 'BOUNDED_KOTLIN_SOURCE_SYNTAX',
                    'text': fact_text, 'condition': premise, 'uncertainty': limit, 'sources': [source['id']]}
            return [fact, dict(fact, status='AMBIGUOUS', text='An unrelated ledger committed.')], {}

        run, plan, packets, _ = self.isolated_fixture(language=language, project=project, body=body)
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        return run, operation, packets['OPERATION'], fact_text, premise, limit

    def test_isolated_reader_preserves_source_qualified_outcome_omitted_by_author(self):
        run, operation, _, fact_text, premise, limit = self.isolated_decoding_outcome('en')
        frozen, = operation['sourceOutcomes']
        self.assertEqual((frozen['text'], frozen['condition'], frozen['uncertainty']), (fact_text, premise, limit))
        path = self.seed / (operation['key'] + '-response.json')
        response = D['load'](path)
        response['claims'].extend([{**response['claims'][0], 'text': text} for text in (
            '`reserveStock` writes the reservation before preparing the receipt.',
            'The handler returns `502` on a decoding error.')])
        D['save'](path, response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        state = D['load'](run / 'state.json')
        attempt, = (row for row in state['attempts'] if row['slot'] == operation['key'])
        authored = D['load'](D['PX']['response_path'](run, attempt))
        self.assertEqual(authored, response)
        self.assertNotIn(fact_text, ' '.join(claim['text'] for claim in authored['claims']))
        html = (Path(result['ready']) / 'html' / self.entry['page'].replace('.md', '.html')).read_text()
        section = re.search(r'<details class="operation-outcomes">(.*?)</details>', html, re.S)[1]
        self.assertIn('Selected conditions and outcomes', section)
        self.assertIn('After <code>reserveStock</code> takes its commit-before-receipt path,', section)
        self.assertIn('failing to decode that receipt can return <code>502</code> to the caller.', section)
        self.assertIn(premise, section)
        self.assertIn(limit, section)
        self.assertNotIn('An unrelated ledger committed.', html)
        anchor, = re.findall(r'href="#(clew-source-\d+-\d+)"', section)
        self.assertIn('Sources', section)
        self.assertIn('reservations / Reservations.kt:', section)
        self.assertRegex(html, rf'<pre id="{anchor}"[^>]*><code>[^<]*ledger\.commit')
        self.assertLess(html.index('on a decoding error.'), html.index('class="operation-outcomes"'))
        self.assertIn('sequence-message return', html)
        self.assertIn('reservations-contracts.html', html)

    def test_isolated_russian_reader_leaves_english_projector_prose_in_the_packet(self):
        run, operation, packet, fact_text, _, limit = self.isolated_decoding_outcome('ru')
        expanded = D['P']['expand_fact_context'](packet)
        self.assertTrue(any(fact['text'] == fact_text for fact in
                            expanded['facts'][operation['requirement']]['facts']))
        self.assertNotIn('sourceOutcomes', operation)
        path = self.seed / (operation['key'] + '-response.json')
        response = D['load'](path)
        authored = '\u041f\u043e\u0441\u043b\u0435 \u0437\u0430\u043f\u0438\u0441\u0438 \u0440\u0435\u0437\u0435\u0440\u0432\u0430 \u043e\u0448\u0438\u0431\u043a\u0430 \u0447\u0442\u0435\u043d\u0438\u044f \u043e\u0442\u0432\u0435\u0442\u0430 \u043f\u0440\u0438\u0432\u043e\u0434\u0438\u0442 \u043a \u043a\u043e\u0434\u0443 502.'
        response['claims'].append({**response['claims'][0], 'text': authored})
        D['save'](path, response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = (Path(result['ready']) / 'html' / self.entry['page'].replace('.md', '.html')).read_text()
        self.assertIn('<html lang="ru">', html)
        self.assertIn(authored, html)
        self.assertIn('\u041a\u0430\u0440\u0442\u0430 \u043f\u0440\u043e\u0446\u0435\u0441\u0441\u043e\u0432', html)
        self.assertNotIn('class="operation-outcomes"', html)
        self.assertNotIn(limit, html)

    def test_isolated_source_outcomes_dedup_qualified_paths_without_merging_distinct_guards(self):
        module = D['P']['OUTCOMES']
        template = {'kind': 'http-failure-after-peer-commit', 'status': 'SOURCE_QUALIFIED_CANDIDATE',
                    'authority': 'RETAINED_SOURCE_EDGE', 'text': 'A response decoder can still fail.',
                    'condition': 'The receiver committed.', 'uncertainty': 'Rollback remains open.',
                    'sources': ['S1', 'S2']}
        packet = {'facts': {
            'owner': {'facts': [template, dict(template, sources=['S2', 'S3']),
                                dict(template, condition='The receiver was interrupted.'),
                                dict(template, status='AMBIGUOUS', text='Ambiguous'),
                                dict(template, uncertainty='', text='Unqualified'),
                                dict(template, sources=['S4'], text='Unbound'),
                                dict(template, sources=['S5'], text='Oversize retained source'),
                                dict(template, kind='ORDERED_EFFECTS', text='Routine steps')]},
            'other': {'facts': [dict(template, text='Wrong operation'),
                                dict(template, text='Explicitly shared', alsoFor=['owner'])]},
        }}
        rows = module['select'](packet, 'owner', {'S1': 200, 'S2': 230, 'S3': 250, 'S5': 300 * 1024})
        self.assertEqual([row['cites'] for row in rows], [['S1', 'S2', 'S3'], ['S1', 'S2'], ['S1', 'S2']])
        self.assertEqual([row['condition'] for row in rows], ['The receiver committed.', 'The receiver was interrupted.',
                                                             'The receiver committed.'])
        self.assertEqual([row['text'] for row in rows][-1:], ['Explicitly shared'])
        full = {'text': ' '.join(template[key] for key in ('text', 'condition', 'uncertainty')),
                'cites': ['S1', 'S2', 'S3'], 'assessment': 'SUPPORTED'}
        self.assertTrue(module['stated_in_author_text'](rows[0], [full]))
        self.assertFalse(module['stated_in_author_text'](rows[0], [dict(full, text=template['text'])]))
        self.assertFalse(module['stated_in_author_text'](rows[0], [dict(full, cites=['S1', 'S2'])]))

    def test_isolated_contracts_render_cited_fieldless_shape_as_prose(self):
        body = ('fun reserve(count: Int): Int {\n'
                '    if (count < 0) return 0\n'
                '    accepted.add(count)\n'
                '    return accepted.size\n}\n'
                'data class Reservation(val count: Int)\n'
                'typealias TagValues = Map<String, String>\n')
        run, plan, packets, _ = self.isolated_fixture(body=body)
        slot = next(row for row in plan['slots'] if row['kind'] == 'CONTRACTS')
        packet = packets['CONTRACTS']
        self.assertTrue(any('typealias TagValues = Map<String, String>' in source['text']
                            for source in packet['sourceRanges'].values()))
        path = self.seed / (slot['key'] + '-response.json')
        response = D['load'](path)
        alias = response['contracts'][0]['cites'][0]
        shape = {'scope': 'LOCAL', 'name': 'TagValues', 'fields': [], 'cites': [alias]}
        response['contracts'].append(shape)
        with self.assertRaises(D['Stop']) as raised:
            D['P']['validate_response'](slot, response)
        issue, = raised.exception.details['issues']
        self.assertEqual(issue['path'], '/contracts/1/fields')
        self.assertEqual(issue['code'], 'PAGE_CONTRACT_FIELDS_REQUIRED')
        response['contracts'].remove(shape)
        response['claims'].append({**response['claims'][0], 'cites': [alias],
                                   'text': '`TagValues` maps string keys to string values.'})
        D['save'](path, response)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = (Path(result['ready']) / 'html/reservations-contracts.html').read_text()
        self.assertIn('<code>TagValues</code> maps string keys to string values.', html)
        self.assertEqual(html.count('<table>'), 1)
        self.assertIn('<td>count</td>', html)
        attempts = D['load'](run / 'state.json')['attempts']
        self.assertEqual([row['number'] for row in attempts if row['slot'] == slot['key']], [0])

    def control_isolated_slots(self, rules):
        self.mode.write_text('controlled-page')
        D['save'](self.seed / 'queue-control.json', rules)

    @staticmethod
    def first_launch_order(run, plan):
        return sorted(plan['slots'], key=lambda slot: -(run / 'inputs' / slot['prompt']).stat().st_size)

    def test_isolated_queue_refills_while_first_slot_still_waits(self):
        self.enterContext(patch.dict(D['P']['page_jobs'].__globals__, {'guide_sources': self.planned_guide_sources}))
        run, plan, _, _ = self.isolated_fixture()
        launch_order = self.first_launch_order(run, plan)
        slow, other, fast, fourth = (row['key'] for row in launch_order)
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
        self.assertCountEqual([row['slot'] for row in state['attempts']], [slow, other, fast, fourth])
        self.assertEqual(state['modelTokens'], 80)
        intervals = sorted((at, delta) for row in state['attempts'] for at, delta in
                           ((row['launchedAt'], 1), (row['completedAt'], -1)))
        active = peak = 0
        for _, delta in intervals:
            active += delta
            peak = max(peak, active)
        self.assertEqual(peak, 3)
        for slot, attempt in zip(launch_order, state['attempts']):
            self.assertEqual(slot['key'], attempt['slot'])
            self.assertEqual((D['PX']['attempt_dir'](run, attempt) / 'prompt.txt').read_bytes(),
                             (run / 'inputs' / slot['prompt']).read_bytes())

    def author_service_outage(self, run, plan, message='workspace routing discovery failed'):
        failed, sibling_a, sibling_b, *later = (row['key'] for row in self.first_launch_order(run, plan))
        terminal = run / 'page-attempts' / failed / '0/result.json'
        self.control_isolated_slots({
            failed: {'terminalFailure': message, 'until': [f'{sibling_a}.waiting', f'{sibling_b}.waiting']},
            sibling_a: {'until': [str(terminal)], 'delay': .15},
            sibling_b: {'until': [str(terminal)], 'delay': .15},
        })
        return failed, sibling_a, sibling_b, later

    def test_isolated_service_failure_drains_siblings_then_resumes_with_honest_usage(self):
        self.enterContext(patch.dict(D['P']['page_jobs'].__globals__, {'guide_sources': self.planned_guide_sources}))
        run, plan, _, _ = self.isolated_fixture(wall_seconds=2)
        failed, sibling_a, sibling_b, later = self.author_service_outage(run, plan)
        execution = D['PX']['execute_authors'].__globals__
        with patch.dict(execution, {'SERVICE_RETRY_DELAY': 2.1}):
            with self.assertRaises(D['Stop']) as raised:
                D['execute'](run)
            self.assertEqual(raised.exception.code, 'PAGE_AUTHOR_SERVICE_UNAVAILABLE')
            self.assertTrue(raised.exception.blocked)
            waiting = D['load'](run / 'state.json')
            self.assertEqual(waiting['status'], 'AUTHOR_SERVICE_RETRY_PENDING')
            pause = waiting['serviceRetryPending']
            self.assertEqual(pause['slots'][failed]['details']['backendMessage'], 'workspace routing discovery failed')
            self.assertGreater(pause['remainingAuthorSeconds'], 1)
            by_slot = {row['slot']: row for row in waiting['attempts']}
            self.assertEqual(set(by_slot), {failed, sibling_a, sibling_b})
            self.assertEqual([by_slot[slot]['status'] for slot in (sibling_a, sibling_b)], ['COMPLETED'] * 2)
            self.assertIsNone(by_slot[failed]['usage'])
            original = {D['PX']['attempt_dir'](run, row) / name: (D['PX']['attempt_dir'](run, row) / name).read_bytes()
                        for row in by_slot.values() for name in ('prompt.txt', 'result.json')}
            with self.assertRaises(D['Stop']) as early:
                D['execute'](run)
            self.assertEqual(early.exception.code, raised.exception.code)
            self.assertEqual(D['load'](run / 'state.json'), waiting)
            self.mode.write_text('normal')
            time.sleep(max(0, pause['retryNotBefore'] - time.time()) + .02)
            result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        resumed = D['load'](run / 'state.json')
        self.assertGreaterEqual(resumed['authorPausedSeconds'], 2)
        self.assertEqual(resumed['attempts'][:3], waiting['attempts'])
        self.assertEqual(resumed['attempts'][3]['slot'], failed)
        self.assertCountEqual([attempt['slot'] for attempt in resumed['attempts'][3:]], [failed, *later])
        retry = resumed['attempts'][3]
        self.assertTrue(retry['serviceRetry'])
        self.assertEqual(retry['resumeThread'], by_slot[failed]['threadId'])
        self.assertEqual(retry['promptDigest'], by_slot[failed]['promptDigest'])
        self.assertEqual({path: path.read_bytes() for path in original}, original)
        accounting = D['load'](run / 'report.json')['checks']['author']
        self.assertFalse(accounting['usageAvailable'])
        self.assertIsNone(accounting['reportedModelTokens'])
        self.assertEqual(accounting['tokenAccounting']['knownBudgetTokensLowerBound'], 80)
        self.assertEqual(accounting['tokenAccounting']['status'], 'UNAVAILABLE')

    def test_isolated_service_recovery_refuses_a_second_outage_in_the_same_thread(self):
        run, plan, _, _ = self.isolated_fixture()
        failed, *_ = self.author_service_outage(run, plan)
        with patch.dict(D['PX']['execute_authors'].__globals__, {'SERVICE_RETRY_DELAY': 0}):
            with self.assertRaises(D['Stop']) as paused:
                D['author'](run)
            self.assertEqual(paused.exception.code, 'PAGE_AUTHOR_SERVICE_UNAVAILABLE')
            with self.assertRaises(D['Stop']) as terminal:
                D['author'](run)
        self.assertEqual(terminal.exception.code, 'PAGE_AUTHOR_SERVICE_RETRIES_EXHAUSTED')
        state = D['load'](run / 'state.json')
        history = [attempt for attempt in state['attempts'] if attempt['slot'] == failed]
        self.assertEqual(len(history), 2)
        self.assertEqual(history[1]['resumeThread'], history[0]['threadId'])
        with self.assertRaises(D['Stop']) as again:
            D['author'](run)
        self.assertEqual(again.exception.code, terminal.exception.code)
        self.assertEqual(D['load'](run / 'state.json'), state)

    def test_isolated_service_recovery_refuses_changed_failed_attempt_prompt(self):
        run, plan, _, _ = self.isolated_fixture()
        failed, *_ = self.author_service_outage(run, plan)
        with patch.dict(D['PX']['execute_authors'].__globals__, {'SERVICE_RETRY_DELAY': 0}):
            with self.assertRaises(D['Stop']):
                D['author'](run)
            receipt = D['load'](run / 'state.json')
            (run / 'page-attempts' / failed / '0/prompt.txt').write_text('Changed source input')
            with self.assertRaises(D['Stop']) as refused:
                D['author'](run)
        self.assertEqual(refused.exception.code, 'PAGE_AUTHOR_SERVICE_RETRY_EVIDENCE_CHANGED')
        state = D['load'](run / 'state.json')
        self.assertEqual(state['attempts'], receipt['attempts'])

    def test_isolated_service_recovery_does_not_retry_unknown_unmetered_failures(self):
        run, plan, _, _ = self.isolated_fixture()
        failed, *_ = self.author_service_outage(run, plan, 'request failed for an unclassified reason')
        with self.assertRaises(D['Stop']) as error:
            D['author'](run)
        self.assertEqual(error.exception.code, 'AUTHOR_USAGE_UNAVAILABLE')
        self.assertTrue(any(row['slot'] == failed and row['failure']['code'] == 'PAGE_AUTHOR_PROCESS_FAILED'
                            for row in error.exception.details['attemptFailures']))
        state = D['load'](run / 'state.json')
        self.assertNotIn('serviceRetryPending', state)
        self.assertTrue(all(row['number'] == 0 for row in state['attempts']))

    def test_isolated_queue_terminal_cancels_and_accounts_for_already_started_slots(self):
        self.legacy_local_route_fixture()
        run, plan, _, _ = self.isolated_fixture()
        slow, invalid, fast, fourth, fifth = (row['key'] for row in self.first_launch_order(run, plan))
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
        first = [row['key'] for row in self.first_launch_order(run, plan)[:3]]
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
        reservation = {field: last[field] for field in ('slot', 'number', 'argv', 'resumeThread', 'startedAt', 'promptDigest')}
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
        first = [row['key'] for row in self.first_launch_order(run, plan)[:3]]
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
        self.assertEqual(resumed['modelTokens'], 60 + 20 * len(plan['slots']))
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

    def test_source_paths_repair_only_the_operation_that_disagrees_with_retained_cases(self):
        body = """fun reserve(count: Int, override: String?): String {
    val name = override?.trim() ?: "default"
    val code = if (count > 10) "wide" else "narrow"
    store.put(code)
    return name
}
"""
        cases = [
            ('fallback', '(override?.trim()) != null', ['override?.trim()']),
            ('fallback', '(override?.trim()) == null', ['"default"']),
            ('write', 'count > 10', ['"wide"', 'store.put']),
            ('write', '!(count > 10)', ['"narrow"', 'store.put']),
        ]

        def project(process, context, *, node):
            if process['root'] is None:
                return [], {}
            return [{'kind': 'VALUE_LINEAGE', 'authority': 'BOUNDED_KOTLIN_SOURCE_SYNTAX',
                     'text': 'The source branches select a fallback and feed a common write.',
                     'sources': [context['sourceRanges'][0]['id']],
                     '_conditionCases': [dict(group=group, condition=condition, actions=actions, stepKind='note')
                                         for group, condition, actions in cases]}], {}

        run, plan, packets, _ = self.isolated_fixture(body=body, project=project, repairs=1)
        operation = next(slot for slot in plan['slots'] if slot['kind'] == 'OPERATION')
        paths = packets['OPERATION']['conditionPaths']
        self.assertEqual(paths, operation['conditionPaths'])
        self.assertEqual({row['condition'] for row in paths}, {condition for _, condition, _ in cases})
        response_path = self.seed / (operation['key'] + '-response.json')
        response = D['load'](response_path)
        response['claims'] = [
            {**response['claims'][0], 'cites': row['cites'],
             'text': 'On `' + row['condition'] + '`, use ' + ' and then '.join('`' + term + '`' for term in row['actions']) + '.'}
            for row in paths]
        response['steps'] = [
            {'from': 'handler', 'to': 'handler', 'kind': 'note',
             'condition': row['condition'], 'label': 'Use ' + ' then '.join(row['actions'])}
            for row in paths]
        D['save'](self.seed / (operation['key'] + '-repair.json'), response)
        response['claims'][1]['text'] = response['claims'][1]['text'].replace('`"default"`', '`override?.trim()`')
        response['steps'][3]['condition'] = 'count > 10'
        D['save'](response_path, response)

        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED')
        state = D['load'](run / 'state.json')
        attempt, repair = [row for row in state['attempts'] if row['slot'] == operation['key']]
        self.assertEqual(repair['resumeThread'], attempt['threadId'])
        self.assertEqual(len(state['attempts']), len(plan['slots']) + 1)
        result = D['load'](D['PX']['attempt_dir'](run, repair) / 'result.json')
        prompt = (D['PX']['attempt_dir'](run, repair) / 'prompt.txt').read_text()
        self.assertIn('PAGE_SOURCE_PATH_MISSING', prompt)
        self.assertIn('"path": "/claims"', prompt)
        self.assertIn('"path": "/steps"', prompt)
        self.assertEqual(result['status'], 'COMPLETED')

    def test_cold_process_map_features_outcomes_before_the_route_inventory(self):
        operation = self.native_entry['record']['entrypoint']
        peer = {'kind': 'THREAD_ENTRY', 'record': {**self.native_entry['record'], 'entrypoint': {
            **operation, 'id': 'dispatch', 'trigger': {**operation['trigger'],
                'sourceRoutes': [{'method': 'POST', 'path': '/dispatch'}]}}}}
        page_jobs = D['P']['page_jobs']

        def planning(job, inventory, prepared, directory, maximum, instructions):
            entries = {row['path']: row for row in inventory['requirements'] if row['kind'] == 'ENTRY'}
            sender, receiver = entries['/reservations'], entries['/dispatch']
            index = D['P']['service_evidence'](prepared['services'][self.service], self.service,
                                                inventory['services'], inventory['snapshot'])
            sources = list(dict.fromkeys(handle for entry in (sender, receiver)
                                         for handle in index['ready'][entry['id']]['evidenceCandidates']))
            hop = {'from': sender['id'], 'to': receiver['id'], 'protocol': 'message',
                   'qualifier': 'parcel.prepared', 'site': 'prepare-dispatch', 'condition': None}
            records = [{'hops': [hop], 'sources': sources, 'status': 'SOURCE_QUALIFIED_CANDIDATE',
                        'selection': {'owner': receiver['id'], 'site': str(status), 'effect': 'outcome',
                                      'httpStatus': status, 'condition': condition, 'sources': sources}}
                       for status, condition in [(201, 'count >= 0'), (422, 'count < 0'), (503, 'serviceOffline')]]
            boundaries = [{'owner': sender['id'], 'method': 'GET', 'endpoint': '/partner/capacity',
                           'sources': sources, 'condition': '!useCache'}]
            return page_jobs(job, inventory, {**prepared, 'routeRecords': records, 'routeBoundaries': boundaries},
                             directory, maximum, instructions)

        with patch.dict(D['P'], {'page_jobs': planning}):
            run, plan, _, _ = self.isolated_fixture(extra_entries=[peer])
        owner = next(entry for entry in plan['entries'] if entry['path'] == '/dispatch')
        response_path = self.seed / (next(slot['key'] for slot in plan['slots'] if slot['page'] == owner['page']) + '-response.json')
        response = D['load'](response_path)
        response['steps'].extend([
            {'from': 'handler', 'to': 'caller', 'kind': 'return', 'label': 'Issue a collection ticket', 'condition': 'count >= 0'},
            {'from': 'handler', 'to': 'caller', 'kind': 'return', 'label': 'Report delivery unavailable', 'condition': 'serviceOffline'}])
        D['save'](response_path, response)

        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = Path(result['ready']) / 'html'
        process_map = (html / 'process-map.html').read_text()
        visible = process_map.split('<main', 1)[1].partition('<details>')[0]
        for text in ('Successful outcomes', 'Refusals and early exits', 'Failures', 'Asynchronous handoffs',
                     'Outbound boundaries', 'Issue a collection ticket', 'HTTP 422', 'HTTP 503',
                     'parcel.prepared', 'GET /partner/capacity', '!useCache'):
            self.assertIn(text, visible)
        self.assertIn(f'href="{owner["page"].replace(".md", ".html")}#sequence"', visible)
        catalog = D['load'](run / 'workspace/draft/routes.json')
        route, = catalog['routes']
        for outcome in route['outcomes']:
            self.assertIn(f'href="{outcome["target"]["page"]}#{outcome["target"]["anchor"]}"', visible)
        self.assertIn('Observed route variants (1)', process_map)
        for later in ('id="documented-operations"', 'class="route-inventory"'):
            self.assertLess(process_map.index('Successful outcomes'), process_map.index(later))

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
        instructions = ' '.join(prompt.rsplit('\n\n', 1)[0].split())
        self.assertIn('return for a result to its actual recipient', instructions)
        self.assertIn('show its return from the producer after the successful call or wait', instructions)
        self.assertNotIn('CONTRACTS |', prompt)
        self.assertFalse(any(slot['kind'] == 'GUIDE' for slot in plan['slots']))

        # Exercise schema-minimal shape through the assembled native reader.
        for field in ('discovery', 'starts', 'contracts', 'routes', 'title'):
            canonical.pop(field)
        D['save'](source, canonical)
        result = D['execute'](run)
        self.assertEqual(result['status'], 'READY_PROMOTED', result)
        html = (Path(result['ready']) / 'html' / self.entry['page'].replace('.md', '.html')).read_text()
        diagram, trace = sequence_rendering(html)
        self.assertRegex(diagram, r'aria-label="[^"]+ → Row reader"[^>]*><span><a href="#clew-sequence-1-step-2">2\. Wait</a>')
        self.assertRegex(diagram, r'class="sequence-message return reverse"[^>]+aria-label="Row reader → [^"]+"[^>]*>'
                                  r'<span><a href="#clew-sequence-1-step-3">3\. Return</a><small>Conditional</small>')
        self.assertRegex(trace, r'<li id="clew-sequence-1-step-3"><div class="sequence-detail">'
                                r'<span class="sequence-direction">Row reader → .*? · Return</span>'
                                r'Return selected rows<p>Read succeeds</p></div></li>')

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
        configuration = 'reservation.clock = utc'
        by_service[self.service]['sources']['cite:configuration'] = {
            'file': 'settings.properties', 'service': self.service, 'startLine': 1, 'endLine': 1, 'revision': 'owned',
            'textDigest': D['sha'](configuration.encode()), 'text': configuration}
        pages = D['P']
        with patch.dict(pages['page_jobs'].__globals__, {'service_evidence': lambda service, *_: by_service[service]}):
            job = dict(self.job, services=[self.service, 'warehouse'])
            directory = self.root / 'page-prompts'
            plan = pages['page_jobs'](job, inventory, {'services': {key: key for key in by_service}},
                                      directory, D['M']['MAX_PAGES'], 'Owned instructions.')
        own = next(slot for slot in plan['slots'] if slot['requirement'] == reservation['id'])
        packet = json.loads((self.root / own['prompt']).read_text().split('\n\n')[-1])
        self.assertEqual({row['text'] for row in packet['sourceRanges'].values()}, {'CALL_SITE'})
        self.assertEqual(set(packet['facts']), {reservation['id']})
        self.assertEqual(own['relatedOperations'], [warehouse['handlerId']])
        interaction, = packet['context']['declaredInteractions']
        self.assertEqual(interaction['peer'], warehouse['handlerId'])
        self.assertEqual(interaction['authority'], 'RETAINED_DECLARATION_NOT_EXECUTED')
        # A peer's independent facts do not become this operation's outcomes.
        warehouse_facts = by_service['warehouse']['facts'][warehouse['id']]['facts']
        warehouse_facts.append({'text': 'The peer alone returns when stock is missing.', 'kind': 'EARLY_EXIT',
                                'authority': 'SOURCE', 'sources': ['cite:' + warehouse['handlerId']]})
        linked_fact = {'text': 'If warehouse commits before returning, decoding can fail after the write.',
                       'kind': 'http-failure-after-peer-commit', 'authority': 'RETAINED_SOURCE_EDGE',
                       'status': 'SOURCE_QUALIFIED_CANDIDATE', 'uncertainty': 'Persistence remains unresolved.',
                       'sources': ['cite:' + reservation['handlerId'], 'cite:' + warehouse['handlerId']]}
        link_basis = {**linked_fact, 'kind': 'http-peer', 'text': 'The retained lookup matches the warehouse entry.'}
        large_details = {**linked_fact, 'kind': 'http-peer-fields', 'text': 'Independent recipient details. ' * 250}
        by_service[self.service]['facts'][reservation['id']]['facts'].extend((linked_fact, link_basis, large_details))
        with patch.dict(pages['page_jobs'].__globals__, {'service_evidence': lambda service, *_: by_service[service]}):
            qualified = pages['page_jobs'](job, inventory, {'services': {key: key for key in by_service}},
                                           self.root / 'qualified-prompts', D['M']['MAX_PAGES'], 'Owned instructions.')
        linked = next(slot for slot in qualified['slots'] if slot['requirement'] == reservation['id'])
        outcome, = linked['sourceOutcomes']
        self.assertEqual(outcome['text'], linked_fact['text'])
        self.assertEqual([linked['citations'][alias] for alias in outcome['cites']], linked_fact['sources'])
        linked_packet = json.loads((self.root / linked['prompt']).read_text().split('\n\n')[-1])
        self.assertEqual({row['text'] for row in linked_packet['sourceRanges'].values()}, {'CALL_SITE', 'PEER_BODY'})
        self.assertEqual(set(linked_packet['facts']), {reservation['id']})
        selected_map = next(slot for slot in qualified['slots'] if slot['kind'] == 'PROCESS_MAP')
        map_prompt = (self.root / selected_map['prompt']).read_text()
        map_packet = pages['expand_fact_context'](json.loads(map_prompt.rsplit('\n\n', 1)[-1]))
        self.assertEqual([row['text'] for row in map_packet['facts'][reservation['id']]['facts']], [link_basis['text']])
        self.assertNotIn(linked_fact['text'], map_prompt)
        self.assertNotIn(large_details['text'], map_prompt)
        self.assertLess(len(map_prompt.encode()), (self.root / linked['prompt']).stat().st_size // 2)
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

        # The route owns its two transitions and route-specific ordering.
        # Both local sequence segments point at the accepted operation pages.
        process = next(slot for slot in qualified['slots'] if slot['kind'] == 'PROCESS_MAP')
        origin_id, peer_id = reservation['handlerId'], warehouse['handlerId']
        own_note = {'from': 'handler', 'to': 'handler', 'kind': 'note',
                    'label': 'Read the item identity.', 'condition': None}
        repeated_note = {**own_note, 'from': origin_id, 'to': origin_id}
        crossing = {'from': origin_id, 'to': peer_id, 'kind': 'await', 'label': 'Wait for the stock lookup.',
                    'condition': 'The reservation names a warehouse.'}
        linked_local = {'from': peer_id, 'to': peer_id, 'kind': 'reference', 'label': '',
                        'condition': 'The stock lookup reaches its retained branch.'}
        unique_note = {'from': origin_id, 'to': origin_id, 'kind': 'note',
                       'label': 'The stock outcome selects a caller continuation.', 'condition': None}
        returned = {'from': peer_id, 'to': origin_id, 'kind': 'return', 'label': 'Return the stock lookup result.',
                    'condition': None}
        route_response = {**response,
                          'slot': process['key'], 'reviewed': True, 'disposition': 'EXPLAINED', 'limitations': [],
                          'claims': [], 'routes': [{**response['routes'][0], 'cites': aliases,
                              'steps': [repeated_note, crossing, linked_local, returned, unique_note]}]}
        schema = D['load'](self.root / process['schema'])
        self.assertNotIn('claims', schema['properties'])
        self.assertNotIn('limitations', schema['properties'])
        unrelated_claim = {'heading': '', 'text': 'The service never validates a reservation.',
                           'cites': aliases, 'assessment': 'EXPLICIT_UNKNOWN'}
        with self.assertRaises(D['Stop']) as rejected_scope:
            pages['validate_response'](process, {**route_response, 'claims': [unrelated_claim],
                                                'limitations': ['The service has no retained validator.']})
        self.assertEqual({issue['path'] for issue in rejected_scope.exception.details['issues']},
                         {'/claims', '/limitations'})
        invalid = {**route_response, 'routes': [{**route_response['routes'][0], 'participants': [],
                    'terminal': origin_id, 'steps': [repeated_note]}]}
        with self.assertRaises(D['Stop']) as rejected:
            pages['validate_response'](process, invalid)
        self.assertIn('PAGE_ROUTE_TRANSITION_REQUIRED', [row['code'] for row in rejected.exception.details['issues']])
        values = {}
        for slot in qualified['slots']:
            if slot['kind'] == 'PROCESS_MAP':
                values[slot['key']] = pages['validate_response'](slot, route_response)
                continue
            claim = {'heading': '', 'text': 'The operation reads a retained value.', 'assessment': 'SUPPORTED',
                     'cites': [next(iter(slot['citations']))]}
            result = {'slot': slot['key'], 'reviewed': True, 'disposition': 'EXPLAINED', 'limitations': [],
                      'claims': [claim]}
            if slot['kind'] == 'OPERATION':
                result.update(participants=[], steps=[own_note])
            elif slot['kind'] == 'GUIDE':
                self.assertEqual(list(slot['citations'].values()), ['cite:configuration'])
                claim['text'] = 'Reservations use UTC in the retained service configuration.'
            elif slot['kind'] == 'CONTRACTS':
                result['contracts'] = []
            values[slot['key']] = pages['validate_response'](slot, result)
        output = pages['assemble_pages'](job, inventory, qualified, values,
            D['load'](self.root / 'starter/original.json'), D['load'](self.root / 'starter/navigation.json'),
            D['I']['assemble'], qualified['citationOwners'])
        registry = output['routes.json']
        narrative = registry['routes'][0]['narrative']['page']
        markdown, _, _ = D['M']['compose_pages'](output['original.json'])
        self.assertNotIn(own_note['label'].encode(), markdown[narrative.replace('.html', '.md')])
        inputs = self.root / 'mixed-markdown'
        inputs.mkdir()
        for name, data in markdown.items():
            (inputs / name).write_bytes(data)
        D['save'](draft / 'navigation.json', output['navigation.json'])
        D['save'](draft / 'routes.json', registry)
        rendered = self.root / 'mixed-html'
        run = subprocess.run([job['node'], str(RENDERER / 'render.mjs'), str(inputs), str(rendered), 'Catalog',
                              '--navigation', str(draft / 'navigation.json'), '--docset-routes', str(draft / 'routes.json'),
                              '--docset', 'retained', '--docset-version', registry['revision']],
                             capture_output=True, text=True)
        self.assertEqual(run.returncode, 0, run.stderr)
        html = (rendered / narrative).read_text()
        for operation in (reservation, warehouse):
            page = operation['page'].replace('.md', '.html')
            self.assertIn(f'href="{page}#sequence"', html)
            self.assertIn('id="sequence"', (rendered / page).read_text())
            self.assertIn(f'href="{page}#behavior"', (rendered / 'process-map.html').read_text())
        self.assertNotIn(own_note['label'], html)
        diagram, trace = sequence_rendering(html)
        self.assertIn(linked_local['condition'], trace)
        self.assertIn(unique_note['label'], trace)
        self.assertRegex(diagram, r'class="sequence-message return reverse"[^>]+aria-label="warehouse → reservations"[^>]*>'
                                  r'<span><a href="#clew-sequence-1-step-4">4\. Return</a>')
        self.assertRegex(trace, r'<li id="clew-sequence-1-step-4"><div class="sequence-detail"><span class="sequence-direction">'
                                r'<a href="warehouse-get-stock.html#handler">GET /stock</a> → '
                                r'<a href="reservations-post-reservations.html#handler">POST /reservations</a> · Return</span>'
                                r'Return the stock lookup result\.</div></li>')
        self.assertIn(f'href="{warehouse["page"].replace(".md", ".html")}#behavior"',
                      (rendered / reservation['page'].replace('.md', '.html')).read_text())
        overview = (rendered / (self.service + '.html')).read_text()
        self.assertIn('Reservations use UTC in the retained service configuration.', overview)
        self.assertIn(f'href="{reservation["page"].replace(".md", ".html")}#behavior"', overview)
        self.assertIn(f'href="{self.service}-contracts.html"', overview)
        self.assertNotIn('The service never validates', (rendered / 'process-map.html').read_text())

    def test_map_upgrade_reuses_owners_and_preserves_distinct_routes_across_promoted_donors(self):
        operation = self.native_entry['record']['entrypoint']
        extra = {'kind': 'THREAD_ENTRY', 'record': {**self.native_entry['record'], 'entrypoint': {
            **operation, 'id': 'stock', 'trigger': {**operation['trigger'],
                'sourceRoutes': [{'method': 'GET', 'path': '/stock'}]}}}}
        pages = D['P']
        plan_jobs = pages['page_jobs']
        active = ['historical-run']
        raw_records = []

        def planning(job, inventory, prepared, directory, maximum, instructions):
            if directory.parent.parent.name in active:
                def old_routes(requirements, indexes, *_args, **_kwargs):
                    return {}, {self.service: list(dict.fromkeys(
                        handle for row in requirements if row['kind'] == 'ENTRY'
                        for handle in indexes[self.service]['ready'][row['id']]['evidenceCandidates']))}, {}

                with patch.dict(plan_jobs.__globals__, {'route_evidence': old_routes}):
                    historical = plan_jobs(job, inventory, prepared, directory, maximum, instructions)
                if directory.parent.parent.name == 'earlier-b':
                    slot = next(slot for slot in historical['slots'] if slot['kind'] == 'PROCESS_MAP')
                    slot.pop('transitionRoutes')
                    schema = D['load'](ROOT / 'skills/codeclew/schemas/docs-run-page-author.schema.json')
                    D['save'](directory.parent / slot['schema'], pages['RESPONSE']['schema_for_slot'](schema, slot))
                return historical
            operations = {row['path']: row for row in inventory['requirements'] if row['kind'] == 'ENTRY'}
            origin, terminal = operations['/reservations']['id'], operations['/stock']['id']
            entry = operations['/reservations']
            index = pages['service_evidence'](prepared['services'][self.service], self.service,
                                              inventory['services'], inventory['snapshot'])
            sources = index['ready'][entry['id']]['evidenceCandidates']
            hop = {'from': origin, 'to': terminal, 'protocol': 'http', 'qualifier': 'GET /stock',
                   'site': 'request-site', 'condition': None}
            raw_records[:] = [
                {'hops': [hop], 'selection': {'owner': terminal, 'site': site, 'effect': kind,
                                              'condition': condition, 'sources': sources},
                 'status': 'SOURCE_QUALIFIED_CANDIDATE', 'sources': sources}
                for site, kind, condition in (('early-return', 'no-write', 'count < 0'),
                                              ('success-write', 'write', 'count >= 0'))]
            records = raw_records
            if directory.parent.parent.name == 'rerun-map':
                records = records[:1]
            if directory.parent.parent.name == 'dense-map':
                records = [{**row, 'hops': [{**hop, 'qualifier': (f'GET /stock/{site % 5}' if site else
                            'GET /stock/[optional] with `quoted [key](nested)` and ) punctuation ' * 4), 'site': str(site)}]}
                           for site in range(75) for row in raw_records]
                edge = records[0]['hops'][0]
                positive, negative = 'eligible', '!eligible && hasStock'
                for guard, choice_condition in ((positive, None), (negative, None), (positive, negative)):
                    selection = ({'owner': origin, 'effect': 'write', 'site': 'independent-choice',
                                  'condition': choice_condition, 'sources': sources} if choice_condition else None)
                    records.append({**records[0], 'hops': [{**edge, 'condition': guard}], 'selection': selection})
            boundaries = [{'owner': origin, 'method': 'GET', 'endpoint': '/partner/availability', 'sources': sources,
                           'condition': None}]
            return plan_jobs(job, inventory, {**prepared, 'routeRecords': records, 'routeBoundaries': boundaries},
                             directory, maximum, instructions)

        original_fixture = self.isolated_fixture
        answers = {
            'historical-run': [('Check stock', 'count < 0'), ('Confirm stock', 'count >= 0')],
            'earlier-a': [('Check stock', 'count < 0'), ('Confirm stock', 'count < 0')],
            'earlier-b': [('Confirm stock', 'count >= 0'), ('Verify availability', 'count >= 0'),
                          ('Check outside the reservation service', 'count < 0')],
        }

        def fixture(*args, **kwargs):
            pairs = answers.get(kwargs.get('run_name'))
            kwargs['extra_entries'] = [extra]
            if pairs is not None:
                def map_routes(packet):
                    entries = {entry['path']: entry for entry in packet['context']['operations']}
                    start, peer = entries['/reservations']['id'], entries['/stock']['id']
                    routes = [{'title': title, 'summary': 'Reservation looks up stock.', 'origin': start,
                             'participants': [{'entry': peer, 'role': 'Warehouse'}], 'condition': condition,
                             'outcome': 'This outdated outcome is replaced.', 'result': 'normal', 'terminal': peer,
                             'boundary': 'return', 'recipient': 'This outdated recipient is replaced.',
                             'steps': [{'from': start, 'to': peer, 'kind': 'call',
                                        'label': 'This outdated step is replaced.', 'condition': None}],
                             'cites': list(packet['citations'])}
                            for title, condition in pairs]
                    if kwargs.get('run_name') == 'earlier-b':
                        last = routes[-1]
                        last.update(participants=[], terminal=None, boundary=None, result='unknown',
                                    steps=[{'from': start, 'to': start, 'kind': 'note',
                                            'label': 'This outdated external action is replaced.', 'condition': None}])
                    return routes

                kwargs['map_routes'] = map_routes
            run, plan, packets, source = original_fixture(*args, **kwargs)
            for slot in plan['slots']:
                if slot['kind'] == 'OPERATION':
                    answer_path = self.seed / (slot['key'] + '-response.json')
                    answer = D['load'](answer_path)
                    answer['steps'][-1]['condition'] = 'count >= 0'
                    D['save'](answer_path, answer)
            return run, plan, packets, source

        def call(command, run, *arguments):
            completed = subprocess.run([sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'),
                                        command, '--run-dir', str(run), *arguments], capture_output=True, text=True)
            self.assertEqual(completed.returncode, 0, completed.stdout + completed.stderr)
            return json.loads(completed.stdout)

        with patch.dict(pages, {'page_jobs': planning}), patch.object(self, 'isolated_fixture', fixture):
            first, target, old_plan = self.isolated_reuse_fixture()
            donors = [first]
            active.extend(('earlier-a', 'earlier-b'))
            for name in active[1:]:
                author = self.job['author']
                if name == 'earlier-b':
                    self.job['author'] = {**author, 'model': 'earlier-fixture-author'}
                try:
                    previous, _, _, _ = self.isolated_fixture(run_name=name)
                finally:
                    self.job['author'] = author
                result = D['execute'](previous)
                self.assertEqual(result['status'], 'READY_PROMOTED')
                (previous / '.runner.lock').touch()
                donors.append(previous)
        new_plan = D['load'](target / 'inputs/author-plan.json')
        old_slots = {slot['key']: slot for slot in old_plan['slots'] if slot['kind'] != 'PROCESS_MAP'}
        self.assertEqual({slot['key']: slot for slot in new_plan['slots']}, old_slots)
        self.assertEqual(len(new_plan['routeRecords']), 2)
        for slot in new_plan['slots']:
            self.assertEqual(D['PR']['slot_files'](target, slot), D['PR']['slot_files'](first, old_slots[slot['key']]))
        seals = {run: D['tree'](run) for run in donors}
        adopted = call('reuse-pages', target, *(arg for donor in donors for arg in ('--reuse-from', str(donor))))
        self.assertEqual((adopted['reusedPages'], adopted['remainingPages']), (len(old_slots), 0), adopted)
        self.assertEqual(adopted['historicalRouteRecords'], 7, adopted)
        result = call('run', target)
        self.assertEqual(result['reuse']['newAuthorInvocations'], 0)
        self.assertEqual(D['load'](target / 'state.json')['modelTokens'], 0)
        self.assertEqual({run: D['tree'](run) for run in donors}, seals)
        route_catalog = D['load'](target / 'workspace/draft/routes.json')
        projected, *historic = route_catalog['routes']
        self.assertEqual(len(historic), 5)
        self.assertEqual(len({row['id'] for row in historic}), 5)
        self.assertEqual({row['title'].split(' · ')[0] for row in historic}, {
            'Check stock', 'Confirm stock', 'Verify availability', 'Check outside the reservation service'})
        self.assertEqual(len({row['title'] for row in historic}), len(historic))
        self.assertEqual(len({row['narrative']['page'] for row in route_catalog['routes']}), 1)
        self.assertTrue(all(row['narrative']['anchor'] == row['id'] for row in route_catalog['routes']))
        external, = [row for row in historic if len(row['occurrences']) == 1]
        self.assertEqual(external['outcomes'][0]['end']['status'], 'unknown')
        self.assertFalse(route_catalog['externalActors'])
        self.assertEqual({row['condition'] for row in projected['branches']}, {'count < 0', 'count >= 0'})
        self.assertEqual(len(projected['outcomes']), 2)
        html = Path(result['ready']) / 'html'
        draft = 'workspace/draft/original.json'
        unchanged = {slot['page'] for slot in new_plan['slots']} | {self.service + '.md'}
        self.assertEqual([row for row in D['load'](target / draft)['pages'] if row['page'] in unchanged],
                         [row for row in D['load'](first / draft)['pages'] if row['page'] in unchanged])
        ledger = D['load'](target / 'page-reuse.json')
        old_attempts = {row['slot']: row for row in D['load'](first / 'state.json')['attempts']}
        self.assertEqual({key: proof['responseDigest'] for key, proof in ledger['pages'].items()},
                         {key: old_attempts[key]['responseDigest'] for key in old_slots})
        for entry in new_plan['entries']:
            file = entry['page'].replace('.md', '.html')
            owner = (html / file).read_text()
            for route in route_catalog['routes']:
                narrative = (html / route['narrative']['page']).read_text()
                for anchor in ('behavior', 'sequence'):
                    self.assertIn(f'href="{file}#{anchor}"', narrative)
                    self.assertIn(f'id="{anchor}"', owner)
                self.assertNotIn('outdated', narrative)
        narrative = (html / projected['narrative']['page']).read_text()
        self.assertIn('count &lt; 0', narrative)
        self.assertIn('accepted.add(count)', narrative)
        self.assertIn('The complete end-to-end outcome', narrative)
        self.assertIn('See the originating operation for the retained boundary.', narrative)
        self.assertIn('Retained HTTP request GET /partner/availability.', narrative)
        self.assertIn('Receiving service and deployment are unresolved', narrative)
        for route in route_catalog['routes']:
            self.assertIn(f'id="{route["id"]}"', narrative)
            self.assertIn(f'href="{route["narrative"]["page"]}#{route["id"]}"', (html / 'process-map.html').read_text())
        exported = {path.name: path.read_bytes() for path in html.glob('*.html')}
        operation_file = new_plan['entries'][0]['page'].replace('.md', '.html')
        exported[operation_file] = exported[operation_file].replace(b'id="sequence"', b'id="broken-sequence"', 1)
        with self.assertRaises(D['E']['Stop']) as missing:
            D['E']['check_links'](exported, (html / 'favicon.svg').read_bytes())
        self.assertEqual(missing.exception.status, 'EXPORT_FRAGMENT_MISSING')

        with patch.dict(pages, {'page_jobs': planning}), patch.object(self, 'isolated_fixture', fixture):
            next_run, _, _, _ = self.isolated_fixture(run_name='rerun-map')
        carried = call('reuse-pages', next_run, '--reuse-from', str(target))
        self.assertEqual(carried['historicalRouteRecords'], 6, carried)
        replayed = call('run', next_run)
        self.assertEqual(replayed['reuse']['newAuthorInvocations'], 0)
        self.assertEqual(D['load'](next_run / 'workspace/draft/routes.json'), route_catalog)

        with patch.dict(pages, {'page_jobs': planning}), patch.object(self, 'isolated_fixture', fixture):
            localized, localized_plan, _, _ = self.isolated_fixture(run_name='localized-map', language='ru')
        localized_output = D['execute'](localized)
        self.assertEqual(localized_output['status'], 'READY_PROMOTED')
        self.assertEqual(len(D['load'](localized / 'state.json')['attempts']), len(localized_plan['slots']))
        directory = Path(localized_output['ready']) / 'html'
        narrative, = directory.glob('process-routes-*.html')
        localized_text = narrative.read_text()
        self.assertIn('\u0423\u0441\u043b\u043e\u0432\u0438\u044f \u0438 \u0438\u0441\u0445\u043e\u0434\u044b', localized_text)
        self.assertIn('\u0414\u0438\u0430\u0433\u0440\u0430\u043c\u043c\u0430 \u043e\u043f\u0435\u0440\u0430\u0446\u0438\u0438 GET /stock', localized_text)

        with patch.dict(pages, {'page_jobs': planning}), patch.object(self, 'isolated_fixture', fixture):
            dense, dense_plan, _, _ = self.isolated_fixture(run_name='dense-map')
        adopted = call('reuse-pages', dense, '--reuse-from', str(target))
        self.assertEqual(adopted['remainingPages'], 0, adopted)
        dense_result = call('run', dense)
        self.assertEqual(dense_result['reuse']['newAuthorInvocations'], 0)
        registry = D['load'](dense / 'workspace/draft/routes.json')
        # Seventy-five structural paths alone exceed the 64-page reader limit.
        self.assertEqual(len(dense_plan['routeRecords']), 153)
        self.assertEqual(len(registry['routes']), 81)
        self.assertEqual(len({row['narrative']['page'] for row in registry['routes']}), 1)
        self.assertEqual(len({row['narrative']['anchor'] for row in registry['routes']}), 81)
        self.assertEqual(len({row['title'] for row in registry['routes']}), 81)
        self.assertEqual(sum(len(row['branches']) for row in registry['routes']), 160)
        dense_html = Path(dense_result['ready']) / 'html'
        self.assertLess(len(list(dense_html.glob('*.html'))), 10)
        alternative, = [route for route in registry['routes'] if len(route['branches']) == 5]
        conditions = {branch['condition'] for branch in alternative['branches']}
        self.assertIn('eligible', conditions)
        self.assertIn('!eligible && hasStock', conditions)
        self.assertIn('See the participating operations for the conditions of each local path.', conditions)
        self.assertFalse(any('eligible · !eligible' in condition for condition in conditions))
        route_html = (dense_html / alternative['narrative']['page']).read_text()
        self.assertNotIn('eligible · !eligible', route_html)
        self.assertNotIn('Retained path condition <code>!eligible &amp;&amp; hasStock</code> · Retained path condition', route_html)
        self.assertNotIn('>Write path<', route_html)
        map_html = (dense_html / 'process-map.html').read_text()
        self.assertIn('<details class="route-inventory"><summary>Search all routes (81)</summary>', map_html)
        disclosure = re.search(r'<details>\s*<summary>Observed route variants \(76\)</summary>(.*?)</details>',
                               map_html, re.S)
        self.assertIsNotNone(disclosure)
        self.assertEqual(len(re.findall(r'<a href="[^"]+#route-[^"]+">', disclosure[1])), 76)
        self.assertIn('[optional]', disclosure[1])
        self.assertLess(map_html.index('>Check outside the reservation service</a>'),
                        map_html.index('<h2 id="documented-operations">'))
        fixture_markdown = D['load'](dense / 'workspace/draft/original.json')
        map_page = next(page for page in fixture_markdown['pages'] if page['page'] == 'process-map.md')
        overview = next(row['text'] for row in map_page['fragments'] if 'Observed route variants' in row['text'])
        self.assertIn('Check outside the reservation service', overview.partition('<details>')[0])
        self.assertNotIn('GET /stock/1', overview.partition('<details>')[0])


    def test_isolated_guides_and_discovery_share_only_owned_source_occurrences(self):
        service, operation = self.service, self.entry
        discovery = next(row for row in self.inventory['requirements'] if row['kind'] == 'DISCOVERY')
        raw = 'fun submit() {\n' + '    recordRequest()\n' * 300 + '}\n'
        def source(name, text):
            return {'id': name, 'service': service, 'revision': 'owned', 'file': name,
                    'startLine': 1, 'endLine': len(text.splitlines()), 'text': text, 'textDigest': D['sha'](text.encode())}

        code, setting, contained = 'cite:source', 'cite:setting', 'cite:contained'
        owned = {'kind': 'guard', 'text': 'Skip a missing item.', 'condition': 'The item is missing.',
                 'authority': 'RETAINED_PATH', 'sources': [code], 'lines': {code: [(4, 4)]}}
        distinct = {**owned, 'lines': {code: [(224, 224)]}}
        contract = {'kind': 'shape', 'text': 'The entry type is a local request.', 'authority': 'DECLARED_SOURCE_CONTRACT',
                    'sources': [code]}
        config = 'service.enabled = true'
        index = {'sources': {code: source('Source.kt', raw), setting: source('settings.properties', config),
                             contained: source('Source.kt', 'fun submit() {\n')},
                 'ready': {row['id']: {'evidenceCandidates': [code]} for row in (operation, discovery)},
                 'facts': {operation['id']: {'facts': [owned, contract]}, discovery['id']: {'facts': [owned, distinct]}},
                 'returns': [], 'coverage': {'coverage': 'SYNTAX'}, 'deferred': []}
        pages = D['P']
        with patch.dict(pages['page_jobs'].__globals__, {'service_evidence': lambda *_: index}):
            plan = pages['page_jobs'](self.job, self.inventory, {'services': {service: {}}},
                                      self.root / 'ownership', D['M']['MAX_PAGES'], 'Owned instructions.')
        by_kind = {slot['kind']: (self.root / slot['prompt']).read_text() for slot in plan['slots']}
        self.assertNotIn('PROCESS_MAP', by_kind)
        packets = {kind: pages['expand_fact_context'](json.loads(prompt.rsplit('\n\n', 1)[-1]))
                   for kind, prompt in by_kind.items()}
        self.assertLess(len(by_kind['GUIDE'].encode()), len(by_kind['OPERATION'].encode()) // 2)
        guide_ranges = [row['text'] for row in packets['GUIDE']['sourceRanges'].values()]
        self.assertEqual(guide_ranges, [config])
        self.assertEqual(packets['GUIDE']['facts'], {})
        guide = next(slot for slot in plan['slots'] if slot['kind'] == 'GUIDE')
        guide_schema = D['load'](self.root / guide['schema'])
        self.assertNotIn('title', guide_schema['required'])
        self.assertNotIn('limitations', guide_schema['required'])
        self.assertEqual(guide_schema['$defs']['claim']['properties']['assessment']['enum'], ['SUPPORTED'])
        overview_claim = {'heading': '', 'text': 'The service flag is enabled in the retained configuration.',
                          'cites': [next(iter(guide['citations']))], 'assessment': 'SUPPORTED'}
        overview_response = {'slot': guide['key'], 'reviewed': True, 'disposition': 'EXPLAINED', 'claims': [overview_claim]}
        self.assertEqual(pages['validate_response'](guide, overview_response)['claims'][0]['text'], overview_claim['text'])
        with self.assertRaises(D['Stop']):
            pages['validate_response'](guide, {**overview_response, 'claims': [
                {**overview_claim, 'assessment': 'EXPLICIT_UNKNOWN'}]})
        self.assertEqual([row['text'] for row in packets['CONTRACTS']['facts'][operation['id']]['facts']],
                         [contract['text']])
        discovered, = packets['DISCOVERY']['facts'][discovery['id']]['facts']
        self.assertEqual(list(discovered['lines'].values()), [[[224, 224]]])
        self.assertIn(raw, [row['text'] for row in packets['DISCOVERY']['sourceRanges'].values()])

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
    def test_installed_prepare_delivers_nested_condition_paths_and_checks_both_reader_views(self):
        launcher = os.environ.get('CODECLEW_TEST_INSTALLED_SOURCE_LAUNCHER')
        if not launcher:
            self.skipTest('An installed release launcher was not configured.')
        source = """package demo

import java.net.URI
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.transaction.support.TransactionTemplate
import org.springframework.web.bind.annotation.PostMapping
import org.springframework.web.bind.annotation.RestController

@RestController
class ParcelLabels(private val transaction: TransactionTemplate, private val store: JdbcTemplate, private val client: HttpClient) {
    @PostMapping("/labels")
    fun submit(raw: String?, query: String, approved: Boolean, enabled: Boolean, usePreset: Boolean): Int {
        if (raw == "ignored") return 409
        return planLabel(raw, query, approved, enabled, usePreset)
    }

    private fun planLabel(raw: String?, query: String, approved: Boolean, enabled: Boolean, usePreset: Boolean): Int {
        val message = raw?.trim() ?: "untitled"
        val label = normalize(raw)
        if (usePreset) storeLabel(label, false) else storeLabel(message, true)
        return sendLabel(query, approved, enabled)
    }

    private fun normalize(raw: String?): String {
        val normalized = raw?.let { raw -> decode(raw) } ?: kotlin.error("unusable")
        audit(normalized)
        return normalized
    }

    private fun storeLabel(label: String, preset: Boolean) {
        transaction.executeWithoutResult {
            val value = if (preset) "existing" else label
            store.update("INSERT INTO journal (label) VALUES (?)", value)
        }
    }

    private fun sendLabel(query: String, approved: Boolean, enabled: Boolean): Int {
        val builder = HttpRequest.newBuilder(URI.create("https://labels.example.invalid/api/labels?k=$query"))
        if (!approved || !enabled) return 422
        if (query.isNotBlank()) {
            builder.header("Accept", "application/json")
        }
        return client.send(builder.GET().build(), HttpResponse.BodyHandlers.ofString()).statusCode()
    }

    @PostMapping("/preview")
    fun preview(raw: String, enabled: Boolean, decorated: Boolean): String {
        if (!enabled) return "blocked"
        return planPreview(raw, decorated)
    }

    private fun planPreview(raw: String, decorated: Boolean): String {
        val preview = if (decorated) raw.uppercase() else raw.lowercase()
        audit(preview)
        return preview
    }

    private fun decode(raw: String): String? = raw.trim().ifEmpty { null }
    private fun audit(raw: String) = Unit

    @PostMapping("/simple")
    fun simple(name: String?): String {
        val resolved = name ?: "any"
        return resolved
    }

    @PostMapping("/health")
    fun health(): String = "ready"
}
"""
        with tempfile.TemporaryDirectory(prefix='clew-condition-test-', dir=Path.home()) as temporary:
            home = Path(temporary)
            repo, docs, run = (home / name for name in ('src', 'docs', 'run'))
            repo.mkdir()
            (repo / 'ParcelLabels.kt').write_text(source)
            author = home / 'unused-author'
            author.write_text('#!/bin/sh\nexit 81\n')
            author.chmod(0o700)
            for args in (('init', '-q'), ('remote', 'add', 'origin', 'https://example.invalid/labels'),
                         ('add', '.'), ('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
                          'commit', '-qm', 'Start fixture')):
                subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True)

            def clew(*args):
                result = subprocess.run([launcher, 'docs', *args, '--root', str(docs)], text=True,
                                        capture_output=True, timeout=30)
                self.assertIn(result.returncode, (0, 3) if args[0] == 'check' else (0,), result.stdout + result.stderr)
                return json.loads(result.stdout)

            with patch.dict(os.environ, {'CODECLEW_HOME': str(home / 'state')}):
                clew('init', '--title', 'Parcel labels')
                service = home / 'service.json'
                D['save'](service, {'schema': 'codeclew-documentation-service/1.0', 'id': 'labels',
                                   'title': 'Labels', 'repositoryId': 'labels', 'repository': 'https://example.invalid/labels',
                                   'language': 'kotlin', 'profile': 'source-syntax', 'targetRef': 'HEAD',
                                   'source': {'roots': ['.'], 'dialect': '1.9'}})
                clew('service', 'add', '--input', str(service),
                     '--expected-input-digest', clew('service', 'list')['inputDigest'])
                clew('bind', '--service', 'labels', '--repo', str(repo))
                job = home / 'job.json'
                D['save'](job, {'schema': D['SCHEMA'], 'request': 'Document conditional paths in the label service.',
                              'clew': str(Path(launcher).absolute()), 'root': str(docs), 'snapshot': clew('check')['snapshot'],
                              'services': ['labels'], 'language': 'en', 'renderer': str(RENDERER),
                              'node': str(Path(shutil.which('node')).absolute()), 'evidenceMode': 'PREPARED_FACTS',
                              'authorMode': 'ISOLATED_PAGES', 'author': {'executable': str(author),
                                  'model': 'unused-test-author', 'reasoningEffort': 'high'},
                              'budget': {'wallSeconds': 45, 'maxRepairs': 0}})
                receipt = D['prepare'](job, run)
            plan = D['load'](run / 'inputs/author-plan.json')
            coverage = receipt['conditionPathCoverage']
            self.assertEqual((coverage['operations'], coverage['operationsWithChecks'], coverage['paths']), (4, 3, 15))
            self.assertGreater(coverage['omissions'], 0)
            slots = {slot['page']: slot for slot in plan['slots'] if slot['kind'] == 'OPERATION'}
            self.assertEqual(slots['labels-post-health.md']['conditionPathCoverage'],
                             {'decisions': 0, 'paths': 0, 'omissions': {'NO_QUALIFIED_BRANCH': 1}})
            slot = slots['labels-post-labels.md']
            packet = json.loads((run / 'inputs' / slot['prompt']).read_text().rsplit('\n\n', 1)[-1])
            paths = packet['conditionPaths']
            self.assertEqual(slot['conditionPaths'], paths)
            by_scope = {}
            for path in paths:
                by_scope.setdefault(path['scope'], []).append(path)
                self.assertEqual(len(path['cites']), 1)
            self.assertEqual(set(by_scope), {'planLabel', 'normalize', 'storeLabel', 'sendLabel'})
            self.assertEqual({row['condition'] for row in by_scope['storeLabel']}, {'preset', '!(preset)'})
            self.assertTrue(all('store.update' in row['actions'] for row in by_scope['storeLabel']))
            self.assertEqual({row['condition'] for row in by_scope['sendLabel']},
                             {'!approved', 'approved && !enabled', 'approved && enabled'})
            self.assertEqual({row['endpoint'] for row in plan['routeBoundaries']},
                             {'https://labels.example.invalid/api/labels'})

            def make_response(slot):
                ordered = sorted(slot['conditionPaths'], key=lambda row: (row['decision'], not row.get('skip')))
                response = {'slot': slot['key'], 'reviewed': True, 'disposition': 'EXPLAINED',
                            'participants': [{'id': 'remote', 'title': 'Label receiver'}], 'steps': [], 'claims': [], 'limitations': []}
                for path in ordered:
                    skip = set(path.get('skip', ()))
                    scope = path.get('scope')
                    premise = (f"In `{scope}`, " if scope else '') + f"When `{path['condition']}`,"
                    actions = ' and '.join(('skip ' if action in skip else '') + f'`{action}`' for action in path['actions'])
                    response['claims'].append({'heading': '', 'text': f'{premise} {actions}.',
                                               'assessment': 'SUPPORTED', 'cites': path['cites']})
                    kind = path.get('stepKind', 'note')
                    response['steps'].append({'from': 'handler', 'kind': kind, 'label': ' '.join(filter(None, (scope, actions))),
                                              'to': {'call': 'remote', 'return': 'caller', 'note': 'handler'}[kind],
                                              'condition': path['condition']})
                return ordered, response

            ordered, response = make_response(slot)
            validate = D['P']['validate_response']
            try:
                validate(slot, response)
            except D['Stop'] as error:
                self.fail(error.details)
            wrong = next(i for i, path in enumerate(ordered) if path['scope'] == 'normalize' and path.get('skip'))
            response['claims'][wrong]['text'] = response['claims'][wrong]['text'].replace('skip ', 'still call ')
            outbound = next(step for step in response['steps'] if step['kind'] == 'call')
            outbound['condition'] = 'approved || enabled'
            with self.assertRaisesRegex(D['Stop'], 'PAGE_RESPONSE_INVALID') as refused:
                validate(slot, response)
            issues = refused.exception.details['issues']
            self.assertEqual({i['path'] for i in issues if i['code'] == 'PAGE_SOURCE_PATH_MISSING'}, {'/claims', '/steps'})

            preview = slots['labels-post-preview.md']
            ordered, response = make_response(preview)
            self.assertTrue(all(path['partial'] for path in ordered))
            arms = {path['condition']: path['actions'] for path in ordered if path.get('scope') == 'planPreview'}
            self.assertEqual(arms['decorated'], ['raw.uppercase()'])
            self.assertEqual(arms['!(decorated)'], ['raw.lowercase()'])
            validate(preview, response)
            index = next(i for i, path in enumerate(ordered) if path['condition'] == '!(decorated)')
            response['claims'][index]['text'] = response['claims'][index]['text'].replace('raw.lowercase()', 'raw.uppercase()')
            response['steps'][index]['condition'] = 'decorated'
            with self.assertRaisesRegex(D['Stop'], 'PAGE_RESPONSE_INVALID') as refused:
                validate(preview, response)
            self.assertEqual({row['path'] for row in refused.exception.details['issues']}, {'/claims', '/steps'})

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
        parser.finish(collected) { completionCallback() }
        laterContinuation()
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
            self.assertIn('`parser.finish(collected)` with a trailing lambda argument', loop['text'])
            self.assertIn('Whether the lambda runs depends on the callee.', loop['text'])
            self.assertNotIn('completionCallback()', loop['text'])
            self.assertNotIn('laterContinuation()', loop['text'])
            regions = [recount['sourceRanges'][region]['text'] for alias in loop['sources']
                       for region in recount['citations'][alias].get('regions', [recount['citations'][alias]['region']])]
            self.assertTrue(any('parser.finish(collected) {' in text for text in regions), regions)
            self.assertTrue({'LOOP_RESULT_SOURCE', 'LOOP_LOCAL_ACCUMULATION'} <= {fact['kind'] for fact in facts})
            self.assertFalse({'LOOP_ENTRY_VALIDATION', 'LOOP_FAILURE_PATH'} & {fact['kind'] for fact in facts})
            sidecar = D['load'](run / 'inputs/author-source/intake/prepared-facts.json')
            raw = contents.encode()
            for packet, literals in ((checkpoint, ('prefix.isNotEmpty()', 'size > 0', 'outcome != ScanResult.Again', 'continue',
                                                   'parser.next(prefix, size)', 'parser.collect()', 'parser.remember()', 'parser.finish(prefix)')),
                                     (recount, ('var collected = 0', 'parser.next()', 'collected += outcome.size',
                                                'outcome.status == ScanResult.Again', 'parser.finish(collected) {'))):
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

    def project(self, *retained, isolated=False, route_records=None, route_boundaries=None):
        return self.edges["project"]([row[0] for row in retained], {row[0]["id"]: row[1] for row in retained},
                                     isolated=isolated, route_records=route_records, route_boundaries=route_boundaries)

    def test_installed_plan_keeps_typed_publication_and_receiver_effect_in_their_scopes(self):
        launcher = os.environ.get('CODECLEW_TEST_INSTALLED_SOURCE_LAUNCHER')
        if not launcher:
            self.skipTest('An installed release launcher was not configured.')
        controller = '''package neutral.inbound;
import neutral.domain.Dispatch;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;
@RestController
public final class Intake {
    private final Dispatch dispatch;
    public Intake(Dispatch dispatch) { this.dispatch = dispatch; }
    @PostMapping("/parcels")
    public Reply accept(@RequestBody Long key) {
        if (key == null) return new Reply(null);
        if (key == 0L) return new Reply(0L);
        if (key == 1L) return new Reply(1L);
        if (key == 2L) return new Reply(2L);
        var parcel = dispatch.execute(key);
        return new Reply(parcel.getId());
    }
    @PostMapping("/signals")
    public Long signal(@RequestBody Long key) {
        dispatch.signal(key);
        return key;
    }
}
'''
        factory = '''package neutral.domain;
import io.eventuate.tram.events.publisher.ResultWithTypedEvents;
import java.math.BigDecimal;
import java.util.HashMap;
import java.util.Map;
import neutral.contract.Changed;
import neutral.contract.ParcelEvent;
public final class Parcel {
    private final Long id;
    private final BigDecimal total = BigDecimal.TEN;
    private final Map<Long, BigDecimal> ledger = new HashMap<>();
    Parcel(Long id) { this.id = id; }
    public Long getId() { return id; }
    public void apply(Long key) {
        var request = BigDecimal.valueOf(key);
        if (available().compareTo(request) >= 0) {
            ledger.put(key, request);
        } else {
            throw new LimitExceeded();
        }
    }
    private BigDecimal available() {
        return total.subtract(ledger.values().stream().reduce(BigDecimal.ZERO, BigDecimal::add));
    }
    static ResultWithTypedEvents<Parcel, ParcelEvent> create(Long key) {
        var parcel = new Parcel(key);
        var placement = new Placement(new Container(key), new Dock(key), new Shelf(key), new Zone(key));
        var event = new Changed(key, placement.tag());
        return new ResultWithTypedEvents<>(parcel, event);
    }
}
'''
        dispatch = '''package neutral.domain;
import java.util.Optional;
import neutral.contract.Approved;
import neutral.contract.Changed;
import neutral.contract.Missing;
import neutral.contract.Refused;
public final class Dispatch {
    private final Port publisher;
    private final Store store;
    public Dispatch(Port publisher, Store store) {
        this.publisher = publisher;
        this.store = store;
    }
    public Parcel execute(Long key) {
        var packet = Parcel.create(key);
        var parcel = packet.getResult();
        store.save(parcel);
        publisher.publish(parcel, packet.getEvents());
        return parcel;
    }
    public void signal(Long key) {
        Optional<Parcel> found = store.find(key);
        if (found.isEmpty()) {
            publisher.publishById(key, new Missing(key));
            return;
        }
        Parcel parcel = found.get();
        try {
            parcel.apply(key);
            Approved event = new Approved(key);
            publisher.publish(parcel, event);
        } catch (LimitExceeded failure) {
            Refused event = new Refused(key);
            publisher.publish(parcel, event);
        }
    }
}
interface Store {
    void save(Parcel parcel);
    Optional<Parcel> find(Long key);
}
'''
        port = '''package neutral.domain;
import io.eventuate.tram.events.publisher.DomainEventPublisherForAggregate;
import neutral.contract.ParcelEvent;
public interface Port extends SourcePort<ParcelEvent> {}
interface SourcePort<E extends ParcelEvent> extends DomainEventPublisherForAggregate<Parcel, Long, E> {}
'''
        publisher_assembly = '''package neutral.assembly;
import io.eventuate.tram.events.publisher.AbstractDomainEventPublisherForAggregateImpl;
import io.eventuate.tram.events.publisher.DomainEventPublisher;
import neutral.contract.ParcelEvent;
import neutral.domain.Parcel;
import neutral.domain.Port;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
@Configuration
public class PublisherAssembly {
    @Bean Port port(DomainEventPublisher delegate) {
        return new Implementation(delegate);
    }
}
final class Implementation extends AbstractDomainEventPublisherForAggregateImpl<Parcel, Long, ParcelEvent> implements Port {
    Implementation(DomainEventPublisher delegate) {
        super(Parcel.class, Parcel::getId, delegate, ParcelEvent.class);
    }
}
'''
        event = '''package neutral.contract;
public record Changed(Long key, String mode) implements ParcelEvent {}
'''
        receiver_event = event.replace(', String mode', '')
        base_event = '''package neutral.contract;
import io.eventuate.tram.events.common.DomainEvent;
public interface ParcelEvent extends DomainEvent {}
'''
        receiver = '''package neutral.subscriber;
import io.eventuate.tram.events.subscriber.DomainEventEnvelope;
import io.eventuate.tram.events.subscriber.annotations.EventuateDomainEventHandler;
import neutral.contract.Approved;
import neutral.contract.Changed;
import neutral.contract.Missing;
import neutral.contract.Refused;
public class Storage {
    private Long latest;
    @EventuateDomainEventHandler(subscriberId = "remote-storage", channel = "neutral.domain.Parcel")
    public void receive(DomainEventEnvelope<Changed> message) {
        if (message.getAggregateId() != null) latest = message.getEvent().key();
    }
    @EventuateDomainEventHandler(subscriberId = "remote-storage", channel = "neutral.domain.Parcel")
    public void approved(DomainEventEnvelope<Approved> message) { latest = message.getEvent().key(); }
    @EventuateDomainEventHandler(subscriberId = "remote-storage", channel = "neutral.domain.Parcel")
    public void refused(DomainEventEnvelope<Refused> message) { latest = message.getEvent().key(); }
    @EventuateDomainEventHandler(subscriberId = "remote-storage", channel = "neutral.domain.Parcel")
    public void missing(DomainEventEnvelope<Missing> message) { latest = message.getEvent().key(); }
    @EventuateDomainEventHandler(subscriberId = "remote-storage", channel = "neutral.other")
    public void elsewhere(DomainEventEnvelope<Changed> message) { latest = 0L; }
}
'''
        assembly = '''package neutral.subscriber;
import org.springframework.context.annotation.Bean;
import org.springframework.context.annotation.Configuration;
@Configuration
public class Assembly {
    @Bean public Storage storage() { return new Storage(); }
}
'''
        alternatives = {f'contract/{event}.java':
                        f'package neutral.contract; public record {event}(Long key) implements ParcelEvent {{}}\n'
                        for event in ('Approved', 'Refused', 'Missing')}
        placement = {f'domain/{part}.java': f'package neutral.domain; record {part}(Long key) {{}}\n'
                     for part in ('Container', 'Dock', 'Shelf', 'Zone')}
        placement['domain/Placement.java'] = '''package neutral.domain;
record Placement(Container container, Dock dock, Shelf shelf, Zone zone) {
    String tag() { return "new"; }
}
'''
        services = {'sender': {'ingress': {'inbound/Intake.java': controller,
                                          'inbound/Reply.java': 'package neutral.inbound; record Reply(Long identifier) {}\n'},
                               'domain': {'domain/Parcel.java': factory, 'domain/Dispatch.java': dispatch, 'domain/Port.java': port,
                                          'domain/LimitExceeded.java': 'package neutral.domain; class LimitExceeded extends RuntimeException {}\n',
                                          'contract/Changed.java': event, 'contract/ParcelEvent.java': base_event,
                                          **alternatives, **placement},
                               'publisher': {'assembly/PublisherAssembly.java': publisher_assembly}},
                    'receiver': {'consumer': {'subscriber/Storage.java': receiver, 'subscriber/Assembly.java': assembly,
                                              'contract/Changed.java': receiver_event, 'contract/ParcelEvent.java': base_event,
                                              **alternatives}}}
        with tempfile.TemporaryDirectory(prefix='clew-java-typed-peers-') as raw:
            base = Path(raw).resolve()
            docs = base / 'docs'
            repo = base / 'workspace'
            environment = {**os.environ, 'CODECLEW_HOME': str(base / 'state')}

            def clew(*args):
                reply = subprocess.run([launcher, 'docs', *args, '--root', str(docs)], env=environment,
                                       capture_output=True, text=True, timeout=30)
                self.assertIn(reply.returncode, (0, 3) if args[0] == 'check' else (0,), reply.stdout + reply.stderr)
                return json.loads(reply.stdout)

            clew('init', '--title', 'Parcel flow')
            registered_roots = {}
            for service, modules in services.items():
                registered_roots[service] = roots = []
                for module, files in modules.items():
                    source = module + '/src/main/java'
                    roots.append(source)
                    for name, text in files.items():
                        path = repo / source / 'neutral' / name
                        path.parent.mkdir(parents=True, exist_ok=True)
                        path.write_text(text)
            remote = 'https://example.invalid/parcel-workspace.git'

            def git(*args):
                subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True)

            git('init', '-q', '-b', 'main')
            git('remote', 'add', 'origin', remote)
            git('add', '.')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'Fixture')
            for service, roots in registered_roots.items():
                spec = base / (service + '.json')
                D['save'](spec, {'schema': 'codeclew-documentation-service/1.0', 'id': service, 'title': service,
                                 'repositoryId': 'parcel-workspace', 'repository': remote, 'language': 'java',
                                 'targetRef': 'main', 'profile': 'source-syntax', 'source': {'roots': roots, 'dialect': '17'}})
                clew('service', 'add', '--input', str(spec), '--expected-input-digest', clew('service', 'list')['inputDigest'])
                clew('bind', '--service', service, '--repo', str(repo))
            snapshot = clew('check')
            self.assertEqual(snapshot['status'], 'CHECKED')
            run = base / 'run'
            job = {'schema': D['SCHEMA'], 'clew': launcher, 'root': str(docs), 'snapshot': snapshot['snapshot'],
                   'services': list(services), 'request': 'Document parcel processing.', 'language': 'en',
                   'renderer': str(RENDERER), 'node': str(Path(shutil.which('node')).absolute()),
                   'evidenceMode': 'PREPARED_FACTS', 'authorMode': 'ISOLATED_PAGES',
                   'author': {'executable': sys.executable, 'model': 'fixture-author', 'reasoningEffort': 'low'},
                   'budget': {'wallSeconds': 60, 'maxRepairs': 0}}
            job_path = base / 'job.json'
            D['save'](job_path, job)
            with patch.dict(os.environ, environment):
                D['prepare'](job_path, run)
            plan = D['load'](run / 'inputs/author-plan.json')
            expanded = [D['P']['expand_fact_context'](json.loads((run / 'inputs' / slot['prompt']).read_text()
                                                                .split('\n\n')[-1]))
                        for slot in plan['slots'] if slot['kind'] == 'OPERATION']
            sender, = [packet for packet in expanded if packet['requirement'].get('path') == '/parcels']
            direct, = [packet for packet in expanded if packet['requirement'].get('path') == '/signals']
            direct_group = direct['facts'][direct['requirement']['id']]['facts']
            routes = [row for row in direct_group if row['kind'] == 'typed-message-peer']
            self.assertEqual(len(routes), 3, [row['text'] for row in direct_group])
            for event_type, premise in (('Missing', '`found.isEmpty()` is true'),
                                        ('Approved', 'execution enters the cited try body'),
                                        ('Refused', 'the catch for `LimitExceeded failure` handles an exception')):
                edge, = [row for row in routes if f'`neutral.contract.{event_type}`' in row['text']]
                self.assertIn(premise, edge['condition'])
                self.assertIn('external Eventuate registrar', edge['condition'])
                self.assertIn('callback completion and runtime execution order remain open', edge['uncertainty'])
                effects = [row for row in direct_group if row['kind'] == 'typed-message-peer-effect'
                           and premise in row['condition']]
                effect, = effects
                citations = [direct['citations'][alias] for alias in effect['sources']]
                views = [direct['sourceRanges'][ref] for cite in citations for ref in cite.get('regions', [cite['region']])]
                self.assertTrue(any(f'void {event_type.lower()}(' in row['text'] for row in views))
                if event_type == 'Refused':
                    self.assertNotIn('parcel.apply(key)', edge['condition'])
                if event_type == 'Approved':
                    self.assertIn('`BigDecimal::add` is formed normally', edge['condition'])
                    self.assertIn('returns normally', edge['condition'])
                    self.assertIn('`available().compareTo(request) >= 0` is true', edge['condition'])
                    self.assertTrue(any('BigDecimal::add' in row['text'] for row in views))
            group = sender['facts'][sender['requirement']['id']]
            facts = {row['kind']: row for key in ('facts', 'boundaries') for row in group.get(key, ())}
            self.assertIn('typed-message-peer', facts, facts)
            joined = facts['typed-message-peer']
            self.assertIn('neutral.domain.Parcel', joined['text'])
            self.assertIn('neutral.contract.Changed', joined['text'])
            self.assertIn('external Eventuate registrar', joined['condition'])
            self.assertIn('wire payload compatibility', joined['uncertainty'])
            rows = sender['sourceRanges']
            def owners(fact):
                citations = [sender['citations'][alias] for alias in fact['sources']]
                return {rows[ref]['service'] for cite in citations for ref in cite.get('regions', [cite['region']])}

            self.assertEqual(owners(joined), {'sender', 'receiver'})
            effect = facts['typed-message-peer-effect']
            self.assertIn('latest = message.getEvent().key()', effect['text'])
            self.assertIn('message.getAggregateId() != null', effect['condition'])
            attachment = facts['java-typed-event-attachment']
            self.assertIn('new ResultWithTypedEvents<>(parcel, event)', attachment['text'])
            self.assertIn('new Changed(key, placement.tag())', attachment['text'])
            self.assertEqual(owners(attachment), {'sender'})
            refs = [sender['citations'][alias] for alias in attachment['sources']]
            attested = [rows[ref] for cite in refs for ref in cite.get('regions', [cite['region']])]
            self.assertTrue(any(row['file'].startswith('domain/') and row['file'].endswith('/Changed.java') for row in attested))
            effects = [row['text'] for row in group['facts'] if row['kind'] == 'java-ordered-effects']
            self.assertTrue(any('store.save(parcel)' in text and 'publisher.publish(parcel, packet.getEvents())' in text
                                and text.index('store.save(parcel)') < text.index('publisher.publish(') for text in effects))
            reply = facts['java-typed-publication-reply']
            self.assertIn('return new Reply(parcel.getId());', reply['text'])
            self.assertIn('no explicit wait', reply['text'])
            self.assertEqual(owners(reply), {'sender'})
            self.assertFalse(any('elsewhere' in row['text'] for row in rows.values()))
            constructions = [row for row in group['facts'] if row['kind'] == 'java-construction']
            body, = [row for row in constructions if 'new Reply(parcel.getId())' in row['text']]
            self.assertIn('record component `identifier`', body['text'])
            self.assertIn('The component receives the value of `key`', body['text'])
            refs = [sender['citations'][alias] for alias in body['sources']]
            self.assertTrue(any('Long identifier' in rows[ref]['text'] for cite in refs
                                for ref in cite.get('regions', [cite['region']])))

    def test_kotlin_source_path_cases_keep_fallback_throw_and_shared_write_distinct(self):
        source = """package demo
import org.springframework.jdbc.core.JdbcTemplate
import org.springframework.transaction.support.TransactionTemplate
import org.springframework.scheduling.annotation.Scheduled

class Entry(private val tx: TransactionTemplate, private val db: JdbcTemplate) {
    @Scheduled(fixedDelay = 200)
    fun save(raw: String?, exists: Boolean) {
        val message = raw?.trim() ?: "untitled"
        val checked = raw ?: kotlin.error("missing")
        log(checked)
        tx.executeWithoutResult {
            val key = if (exists) "existing" else "new"
            db.update("INSERT INTO journal (name) VALUES (?)", key)
        }
    }
}
"""
        name, revision = 'Entry.kt', 'a' * 40
        declaration = source[source.index('@Scheduled'):source.rindex('}')].rstrip()
        def retained(ref, content):
            first = source.index(content)
            start_line = source[:first].count('\n') + 1
            raw = content.encode()
            start = len(source[:first].encode())
            return {'id': ref, 'file': name, 'service': 'notes', 'revision': revision,
                    'text': content, 'textDigest': D['sha'](raw), 'authority': 'EXACT_SNAPSHOT_TEXT',
                    'startLine': start_line, 'endLine': start_line + content.removesuffix('\n').count('\n'),
                    'startByte': start, 'endByte': start + len(raw), 'references': [ref],
                    'occurrence': {'snapshot': F['SNAPSHOT'], 'blob': 'b' * 40,
                                   'startByte': start, 'endByte': start + len(raw)}}
        exact = {'file': retained('file', source), 'body': retained('body', declaration)}
        requirement = {'id': 'entry', 'entrySymbol': 'save', 'kind': 'ENTRY', 'entryKind': 'SCHEDULED_JOB',
                       'rootDeclarations': ['save'], 'sourceReferences': ['body'],
                       'service': 'notes', 'page': 'notes.md'}
        body = {'id': 'BODY:save', 'obligationKind': 'BODY', 'sourceReferences': ['body'],
                'record': {'declaration': 'save', 'sourceIds': ['body']}}
        process, context = self.prepared['operation'](requirement, [body], exact, {name: exact['file']})
        context.update(_sourceCallSelections=[], _deferredWriteFocus=[],
                       _bindWitness=lambda witness: self.prepared['extra_binding'](witness, exact))
        project = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/prepared_facts/managed_languages.py'))['project']
        rows, witnesses = project(process, context, node=None)
        refs = {site['id'] for site in context['sourceRanges']}
        facts, unbound = self.prepared['bound_facts'](rows, witnesses, exact, refs)
        self.assertEqual(unbound, 0)
        paths = {row['condition']: row for fact in facts for row in fact.get('_conditionCases', ())}
        self.assertEqual(paths['(raw?.trim()) == null']['actions'], ['"untitled"'])
        self.assertEqual(paths['(raw?.trim()) != null']['actions'], ['raw?.trim()'])
        self.assertEqual(paths['(raw) == null']['actions'][0], 'IllegalStateException')
        self.assertEqual(paths['(raw) == null']['skip'], paths['(raw) != null']['actions'])
        self.assertTrue(any(fact['kind'] == 'write_lineage' for fact in facts), facts)
        self.assertEqual(paths['exists']['actions'], ['"existing"', 'db.update'])
        self.assertEqual(paths['!(exists)']['actions'], ['"new"', 'db.update'])

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

        def listener(name, configuration, *, registered=True, callback=True, complete=True, gated=False,
                     authority="RETAINED_JAVA_DISPATCHER_WIRING"):
            references = [f"{name}-{piece}" for piece in assembly]
            return self.retained(name, entry_kind="JAVA_TYPED_DISPATCH", proof=registered,
                                 trigger={"kind": "JAVA_TYPED_DISPATCH", "authority": authority,
                                          "beanActivationOpen": gated,
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
                 listener("compatible-function", config, authority="INFERRED")]
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
        declared = "RETAINED_JAVA_HANDLER_BEAN_DECLARATION"
        possible = listener("declared", config, authority=declared)
        missing = listener("missing-declaration-source", config, authority=declared, complete=False)
        gated = listener("gated-declaration", config, authority=declared, gated=True)
        partial = self.project(owner, possible, missing, gated, *peers)
        self.assertNotIn(missing[0]["id"], partial)
        self.assertNotIn(gated[0]["id"], partial)
        self.assertEqual(self.project(owner, gated)[owner[0]["id"]][0]["status"], "UNKNOWN")
        for entry in (owner, possible):
            edge, = partial[entry[0]["id"]]
            self.assertEqual((edge["kind"], edge["status"]), ("typed-message-peer", "POSSIBLE"))
            self.assertEqual(edge["condition"], "If the external Eventuate registrar is activated and the message is delivered.")
            self.assertIn("registrar and its activation unresolved", edge["uncertainty"])
            self.assertIn("acknowledgement", edge["uncertainty"])
            self.assertEqual(edge["sources"], ["origin-body", "declared-registration",
                                               *(f"declared-{piece}" for piece in assembly), "declared-body"])
        also_possible = listener("another-declaration", config, authority=declared)
        ambiguous = self.project(owner, possible, also_possible)
        self.assertEqual(set(ambiguous), {owner[0]["id"]})
        unresolved, = ambiguous[owner[0]["id"]]
        self.assertEqual((unresolved["kind"], unresolved["status"]), ("typed-message-peer-boundary", "AMBIGUOUS"))
        stronger = self.project(owner, possible, eligible)
        self.assertNotIn(possible[0]["id"], stronger)
        self.assertEqual(stronger[owner[0]["id"]][0]["status"], "SOURCE_QUALIFIED_CANDIDATE")

        audit = listener("audit", {**config, "subscriberId": "audit-reader"}, authority=declared)
        refresh = listener("refresh", {**config, "subscriberId": "refresh-reader"}, authority=declared)
        fanout = self.project(owner, audit, refresh)
        self.assertEqual([row["status"] for row in fanout[owner[0]["id"]]], ["POSSIBLE", "POSSIBLE"])
        collision = listener("collision", {**config, "subscriberId": "audit-reader"}, authority=declared)
        mixed = self.project(owner, audit, refresh, collision)
        self.assertEqual({row["status"] for row in mixed[owner[0]["id"]]}, {"POSSIBLE", "AMBIGUOUS"})
        self.assertNotIn(audit[0]["id"], mixed)
        self.assertNotIn(collision[0]["id"], mixed)

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

    def test_process_evidence_keeps_distinct_calls_returns_and_unrelated_operations(self):
        call = {"kind": "CALL", "resolution": "COMPILER_EXACT", "http": {
            "adapter": "SPRING_REST_CLIENT_URI/2.0", "method": "GET", "path": "/units/42"}}
        origin = self.retained("front", path="/start", events=[
            ("primary", "primary.get()", call), ("fallback", "fallback.get()", call)])
        peer = self.retained("depot", path="/units/{key}", events=[
            ("return", "return available;", {"kind": "RETURN"}), ("throw", "throw missing;", {"kind": "THROW"})])
        other = self.retained("other", path="/unrelated")
        records = []
        plain = self.project(origin, peer, other, isolated=True)
        self.assertEqual(self.project(origin, peer, other, isolated=True, route_records=records), plain)
        self.assertEqual(len(records), 6)
        self.assertEqual({row["hops"][0]["site"] for row in records}, {"front-primary", "front-fallback"})
        self.assertEqual({(row["hops"][0]["from"], row["hops"][0]["to"]) for row in records},
                         {(origin[0]["id"], peer[0]["id"])})
        selected = [row["selection"] for row in records if row["selection"]]
        self.assertTrue(all(row["owner"] == peer[0]["id"] for row in selected))
        self.assertEqual({source for row in selected for source in row["sources"]}, {"depot-return", "depot-throw"})

    def test_same_source_call_keeps_alternative_guards_and_qualified_outgoing_boundary(self):
        def request(condition, path="/units/2", occurrence="request"):
            return {"kind": "http_request", "authority": "RETAINED_PATH", "text": "Send the request.",
                    "condition": condition, "sources": ["front-body"], "_edge": {
                         "kind": "HTTP", "site": "front-body", "occurrence": occurrence, "method": "GET", "uri": path}}

        front = self.retained("front", path="/start", facts=[
            request("eligible"), request("!eligible && hasStock"),
            request("ordered", "/partner/availability?token=ignored", "outside")])
        peer = self.retained("depot", path="/units/{id}")
        records, boundaries = [], []
        baseline = self.project(front, peer, isolated=True)
        self.assertEqual(self.project(front, peer, isolated=True, route_records=records, route_boundaries=boundaries),
                         baseline)
        self.assertEqual({row["hops"][0]["condition"] for row in records}, {"eligible", "!eligible && hasStock"})
        self.assertTrue(all(row["hops"][0]["sources"] == ["front-body"] for row in records))
        boundary, = boundaries
        self.assertEqual((boundary["owner"], boundary["method"], boundary["endpoint"], boundary["sources"]),
                         (front[0]["id"], "GET", "/partner/availability", ["front-body"]))

    def test_jdk_http_builder_boundary_binds_only_settled_reached_requests(self):
        root = ROOT / 'skills/codeclew/scripts'
        project = runpy.run_path(str(root / 'prepared_facts/clew_prepared_jdk_http.py'))['project']
        reached = D['P']['REACHED']

        def records(file, source, root_text, helper=None):
            def site(text, handle):
                index = source.index(text)
                offset = len(source[:index].encode())
                first = 1 + source.count('\n', 0, index)
                data = text.encode()
                return {'file': file, 'service': 'neutral', 'revision': 'kept', 'text': text, 'textDigest': D['sha'](data),
                        'startByte': offset, 'endByte': offset + len(data), 'startLine': first,
                        'endLine': first + text.count('\n'), 'handle': handle}

            sources = {'root': site(root_text, 'root-body')}
            facts = [{'kind': 'BODY', 'declaration': 'entry', 'sources': ['root']}]
            if helper:
                sources.update(detail=site(helper, 'helper-body'), selection=site('return sendLater(query)', 'helper-call'))
                facts.extend([{'kind': 'BODY', 'declaration': 'helper', 'sources': ['detail']},
                              {'kind': 'CALL', 'declaration': 'entry', 'targetMethodId': 'helper',
                               'resolution': 'SAME_SCOPE_SELECTED_DECLARATION',
                               'relationshipAuthority': 'RETAINED_K2_COMPILER_SELECTED_SYMBOL_NOT_RUNTIME_DISPATCH',
                               'sources': ['selection']}])

            def bind(witness):
                matched = [row for row in sources.values() if reached['relative_slice'](row, witness) is not None]
                return min(matched, key=lambda row: len(row['text']))['handle'] if matched else None

            process = {'root': 'entry', 'sources': sources, 'facts': facts}
            context = {'_bindWitness': bind, '_sourceCallSelections': [],
                       '_nativeFiles': {file: {'text': source, 'textDigest': D['sha'](source.encode()), 'revision': 'kept'}}}
            return project(process, context, reached, {})

        imports = "import java.net.URI\nimport java.net.http.HttpRequest\nimport java.net.http.HttpClient\n"
        method = """fun check(fixture: Boolean, query: String, client: HttpClient): Int {
    if (fixture) return 200
    val builder = HttpRequest.newBuilder(URI.create("https://stock.example.invalid/v2/items?q=$query"))
    return client.send(builder.GET().build(), HttpResponse.BodyHandlers.ofString()).statusCode()
}"""
        direct, = records('Offers.kt', imports + method, method)
        self.assertEqual({key: direct[key] for key in ('method', 'endpoint', 'condition', 'sources')}, {
            'method': 'GET', 'endpoint': 'https://stock.example.invalid/v2/items', 'condition': '!fixture',
            'sources': ['root-body']})
        self.assertGreater(direct['site'][1], 3)
        self.assertNotIn('query', str(direct))
        conditional = method.replace('    return client.send',
            '    if (query.isNotBlank()) builder.header("X-Mode", query)\n'
            '    return client.send')
        conditioned, = records('Offers.kt', imports + conditional, conditional)
        self.assertEqual((conditioned['method'], conditioned['endpoint']), ('GET', direct['endpoint']))
        with_options = method.replace('    return client.send',
            '    if (query.isNotBlank()) {\n'
            '        builder.header("Accept", "application/json").expectContinue(true)\n'
            '    }\n'
            '    builder.headers("X-A", "1", "X-B", "2")\n'
            '    return client.send')
        compound = conditional.replace('    val builder', '    val result = if (query.isEmpty()) { 204 } else {\n        val builder')
        compound = compound.replace('    return client.send', '        client.send').removesuffix('}') + '    }\n    return result\n}'
        nested, = records('Offers.kt', imports + compound, compound)
        self.assertEqual((nested['method'], nested['endpoint']), ('GET', direct['endpoint']))
        configured, = records('Offers.kt', imports + with_options, with_options)
        self.assertEqual({key: configured[key] for key in ('method', 'endpoint', 'condition', 'sources')},
                         {key: direct[key] for key in ('method', 'endpoint', 'condition', 'sources')})
        owner, edge_record, _ = self.retained('neutral', path='/inside')
        baseline = self.project((owner, edge_record))
        boundaries = []
        mapped = (owner, {**edge_record, 'jdkHttpBoundaries': [direct]})
        self.assertEqual(self.project(mapped, isolated=True, route_boundaries=boundaries), baseline)
        self.assertEqual(boundaries, [{**direct, 'owner': owner['id']}])
        helper = method.replace('fun check(fixture: Boolean, query: String, client: HttpClient): Int {',
                                'private fun sendLater(query: String): Int {').replace('    if (fixture) return 200\n', '')
        caller = 'fun check(query: String): Int { return sendLater(query) }'
        source = imports + 'class Offers(private val client: HttpClient) {\n' + caller + '\n' + helper + '\n}'
        reached_request, = records('Offers.kt', source, caller, helper)
        self.assertEqual(reached_request['endpoint'], direct['endpoint'])
        self.assertEqual(reached_request['sources'], ['root-body', 'helper-body', 'helper-call'])
        self.assertEqual(records('Offers.kt', source, caller), [])
        java = """int check(HttpClient client, String query) {
    var builder = HttpRequest.newBuilder(URI.create("https://stock.example.invalid/v2/items" + "?q=" + query));
    return client.send(builder.POST(BodyPublishers.noBody()).build(), BodyHandlers.ofString()).statusCode();
}"""
        java_imports = imports.replace('\n', ';\n')
        java_request, = records('Offers.java', java_imports + 'class Offers { ' + java + ' }', java)
        self.assertEqual((java_request['method'], java_request['endpoint']), ('POST', direct['endpoint']))
        for extension, header, compound in (
            ('kt', imports, method.replace('fixture: Boolean,', 'approved: Boolean, enabled: Boolean,')
             .replace('if (fixture) return 200', 'if (!approved || !enabled) return 403')),
            ('java', java_imports, java.replace('HttpClient client,', 'HttpClient client, boolean approved, boolean enabled,')
             .replace('    var builder', '    if (!approved || !enabled) { return 403; }\n    var builder')),
        ):
            declared = compound if extension == 'kt' else 'class Gate { ' + compound + ' }'
            selected, = records('Gate.' + extension, header + declared, compound)
            self.assertEqual(selected['condition'], 'approved && enabled')
            paths = selected['_conditionCases']
            self.assertEqual([row['condition'] for row in paths], ['approved && enabled', '!approved', 'approved && !enabled'])
            self.assertEqual([row['stepKind'] for row in paths], ['call', 'return', 'return'])
            self.assertEqual([row.get('skip') for row in paths], [None, ['send'], ['send']])
        checks = D['P']['RESPONSE']['CONDITION_PATHS']
        paths, _ = checks['select']([selected], {'root-body': 'S1'})
        class Review:
            slot = {'conditionPaths': paths}
            def __init__(self):
                self.issues = []

            def issue(self, code, path, message, **details):
                self.issues.append((path, message, details['sourcePath']['condition']))

        cases = paths[1:] + paths[:1]
        accepted = {'claims': [
            {'assessment': 'SUPPORTED', 'cites': ['S1'], 'text': (
                f"Under `{row['condition']}`, `return 403` before calling `send`." if row.get('skip') else
                f"Under `{row['condition']}`, call `send`.")} for row in cases],
            'steps': [
                {'condition': row['condition'], 'label': 'return 403 before calling send' if row.get('skip') else 'send',
                 'kind': row['stepKind']} for row in cases]}
        review = Review()
        checks['check'](review, accepted)
        self.assertEqual(review.issues, [])
        accepted['claims'][0]['text'] = 'When `!approved`, `return 403` and call `send`.'
        accepted['steps'].reverse()
        review = Review()
        checks['check'](review, accepted)
        self.assertTrue(any(path == '/claims' and condition == '!approved' for path, _, condition in review.issues))
        self.assertTrue(any(path == '/steps' and 'before the outbound request' in message
                            and condition == 'approved && !enabled' for path, message, condition in review.issues))
        java_options = java.replace('    return client.send',
            '    if (!query.isEmpty()) { builder.setHeader("X-Mode", "local"); }\n'
            '    return client.send')
        configured_java, = records('Offers.java', java_imports + 'class Offers { ' + java_options + ' }', java_options)
        self.assertEqual((configured_java['method'], configured_java['endpoint']), ('POST', direct['endpoint']))
        for mutation in (
            method.replace('val builder', 'var builder').replace('    return client.send',
                '    builder = HttpRequest.newBuilder(URI.create("https://other.example.invalid"))\n    return client.send'),
            with_options.replace('    return client.send', '    inspect(builder)\n    return client.send'),
            with_options.replace('    return client.send', '    val escaped = builder.header("X-Mode", "local")\n    return client.send'),
            with_options.replace('    return client.send', '    val alias = builder\n    return client.send'),
            with_options.replace('    return client.send', '    val alias = `builder`\n    return client.send'),
            with_options.replace('    return client.send', '    builder.DELETE()\n    return client.send'),
            with_options.replace('    return client.send', '    builder.customize()\n    return client.send'),
            with_options.replace('"application/json"', '"${inspect(builder)}"'),
            with_options.replace('    return client.send',
                '    builder.uri(URI.create("https://other.example.invalid"))\n    return client.send'),
            method.replace('    return client.send(builder.GET().build(),',
                '    val second = HttpRequest.newBuilder(URI.create("https://other.example.invalid"))\n'
                '    return client.send(if (fixture) second.GET().build() else builder.GET().build(),'),
            method.replace('    return client.send', '    builder.uri(URI.create("https://other.example.invalid"))\n    return client.send'),
            method.replace('https://stock.example.invalid/v2/items?q=', 'https://stock.example.invalid/$query?q='),
            method.replace('https://stock.example.invalid/v2/items?q=', 'https://name:secret' '@stock.example.invalid/v2/items?q='),
        ):
            self.assertEqual(records('Offers.kt', imports + mutation, mutation), [], mutation)
        for external_type in ('HttpRequest', 'HttpClient', 'URI'):
            lookalike = imports.replace(next(line for line in imports.splitlines() if line.endswith('.' + external_type)),
                                        'import neutral.client.' + external_type)
            self.assertEqual(records('Offers.kt', lookalike + with_options, with_options), [], external_type)

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

    def test_scoped_subscriber_verdicts_and_conditional_relay_survive_author_packets(self):
        def retained_fact(name, kind, text, **data):
            return {"kind": kind, "authority": "RETAINED_PATH", "text": text, "sources": [name + "-body"], **data}

        def operation(name, verb, occurrence, **identity):
            return retained_fact(name, "effect", "The source reaches the cited operation.",
                                 _edge={"kind": verb, "site": name + "-body", "occurrence": occurrence, **identity})

        def verdict(occurrence, topic, compatible, text, **conditions):
            return retained_fact("apply-change", "write_compatibility", text,
                                 _writeFocus={"occurrence": occurrence, "topic": topic,
                                              "compatibility": "COMPATIBLE" if compatible else "INCOMPATIBLE"},
                                 **conditions)

        inbound = self.retained("intake", name="submit", facts=[operation("submit", "PUBLISH", "start", topic="pickup.ready")])
        enqueue = self.retained("routing", name="enqueue", entry_kind="KAFKA_LISTENER",
                                trigger={"configuration": {"topics": ["pickup.ready"]}}, facts=[
                                    {**operation("enqueue", "INSERT", "queue", table="pending_changes"),
                                     "condition": "the request has a warehouse key"}])
        relay_facts = [operation("relay", "SELECT", "select", table="pending_changes"),
                       operation("relay", "PUBLISH", "send", topic="inventory.apply")]
        poller = self.retained("routing", name="relay", entry_kind="SCHEDULED_JOB",
                              facts=[*relay_facts, {**self.poll_proof("relay"), "condition": "the poll finds an eligible row"}])
        other = self.retained("retirement", name="cancel", facts=[operation("cancel", "PUBLISH", "retire", topic="inventory.retire")])
        recipient = self.retained("warehouse", name="apply-change", entry_kind="KAFKA_LISTENER",
            trigger={"configuration": {"topics": ["inventory.apply", "inventory.retire"]}}, facts=[
                operation("apply-change", "UPDATE", "apply", table="inventory"),
                operation("apply-change", "DELETE", "retire", table="inventory"),
                verdict("apply", "inventory.apply", True, 'Normal validation fixes `action` to `"apply"`.',
                        condition="the validator returns normally"),
                verdict("retire", "inventory.apply", False, 'The `action === "retire"` guard evaluates to `false`.'),
                verdict("apply", "inventory.retire", False, 'The `action === "apply"` guard evaluates to `false`.'),
                verdict("retire", "inventory.retire", True, 'Normal validation fixes `action` to `"retire"`.')])
        operations = (inbound, enqueue, poller, other, recipient)
        inbound_id, other_id = inbound[0]["id"], other[0]["id"]

        def verify(rows):
            hop, = (row for row in rows if row["kind"] == "further-subscriber-outbox")
            for part in ("pickup.ready", "pending_changes", "enqueue.md", "relay.md", "same key", "inventory.apply"):
                self.assertIn(part, hop["text"])
            self.assertIn("the request has a warehouse key", hop["condition"])
            self.assertIn("the poll finds an eligible row", hop["condition"])
            positive, = (row for row in rows if row["kind"] == "message-peer-write-compatibility")
            for part in ("relay.md", "inventory.apply", "UPDATE", "apply-change.md", "validator returns normally"):
                self.assertIn(part, positive["text"] + " " + positive["condition"])
            negative, = (row for row in rows if row["kind"] == "message-peer-write-boundary")
            self.assertEqual(negative["status"], "INCOMPATIBLE_IN_RETAINED_PATHS")
            for part in ("relay.md", "inventory.apply", "DELETE", "apply-change.md", 'action === "retire"'):
                self.assertIn(part, negative["text"])
            self.assertNotIn("enqueue.md", negative["text"])
            self.assertNotIn("condition", negative)
            self.assertTrue(all("delivery" in row["uncertainty"] for row in (hop, positive, negative)))

        for isolated in (False, True):
            with self.subTest(isolated=isolated):
                projected = self.project(*operations, isolated=isolated)
                verify(projected[inbound_id])
                positives = [row["text"] for row in projected[other_id] if row["kind"] == "message-peer-write-compatibility"]
                self.assertTrue(any("inventory.retire" in text and "DELETE" in text for text in positives))
                self.assertFalse(any("UPDATE" in text for text in positives))

        unmatched = self.retained("routing", name="relay", entry_kind="SCHEDULED_JOB", facts=relay_facts)
        missing = self.project(inbound, enqueue, unmatched, recipient, isolated=True)[inbound_id]
        frontier, = (row for row in missing if row["kind"] == "further-outbox-poll-activation")
        self.assertIn("the independently registered poller in `relay.md` can select the row", frontier["text"])
        self.assertIn("same database and row key", frontier["text"])
        boundary, = (row for row in missing if row["kind"] == "further-outbox-boundary")
        self.assertIn("leaves the row-to-publication link open", boundary["text"])
        self.assertEqual(boundary["status"], "UNKNOWN")
        self.assertFalse(any(row["status"] == "INCOMPATIBLE_IN_RETAINED_PATHS" for row in missing))

        requirements, services = [], {}
        projected = self.project(*operations, isolated=True)
        citations = {handle: source for _, _, exact in operations for handle, source in exact.items()}
        entries = {row[0]["id"]: {"propositions": projected[row[0]["id"]]} for row in operations}
        author_entries = self.prepared["author_entries"](entries, citations)
        for requirement, _, exact in operations:
            service, identity = requirement["service"], requirement["id"]
            requirements.append({**requirement, "handlerId": identity + "-handler"})
            scope = services.setdefault(service, {"ready": {}, "sources": {}, "facts": {}, "returns": [],
                  "coverage": {"coverage": "SYNTAX"}, "deferred": []})
            scope["ready"][identity] = {"evidenceCandidates": list(exact)}
            scope["sources"].update(exact)
            if identity in author_entries:
                scope["facts"][identity] = author_entries[identity]
            for fact in projected[identity]:
                scope["sources"].update({handle: citations[handle] for handle in fact["sources"]})

        job = {"services": list(services), "request": "Describe retained behavior.", "language": "en"}
        inventory = {"services": {service: {} for service in services}, "requirements": requirements,
                     "snapshot": F["SNAPSHOT"]}
        pages = D["P"]
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            with patch.dict(pages["page_jobs"].__globals__, {"service_evidence": lambda scope, *_: scope}):
                plan = pages["page_jobs"](job, inventory, {"services": services}, home / "prompts",
                                          D["M"]["MAX_PAGES"], "Owned instructions.")
            slots = [slot for slot in plan["slots"] if slot["requirement"] == inbound_id or slot["kind"] == "PROCESS_MAP"]
            self.assertEqual(len(slots), 2)
            for slot in slots:
                with self.subTest(slot=slot["kind"]):
                    prompt = (home / slot["prompt"]).read_text()
                    packet = pages["expand_fact_context"](json.loads(prompt.rsplit("\n\n", 1)[-1]))
                    scoped = packet["facts"][inbound_id]
                    visible = [*scoped.get("facts", ()), *scoped.get("boundaries", ())]
                    if slot["kind"] == "OPERATION":
                        verify(visible)
                    else:
                        bridge, = (row for row in visible if row["kind"] == "further-subscriber-outbox")
                        self.assertIn("inventory.apply", bridge["text"])
                        self.assertIn("the poll finds an eligible row", bridge["condition"])
                        self.assertFalse(any(row["kind"] == "message-peer-write-compatibility" for row in visible))

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
