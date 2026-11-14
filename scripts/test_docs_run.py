#!/usr/bin/env python3
"""Owned source/author protocol fixtures; no models, capture or public writes."""
import json
import hashlib
import os
from pathlib import Path
import runpy
import shutil
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
print(json.dumps({{'type':'thread.started','thread_id':'owned-test-session'}}),flush=True)
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
print(json.dumps({{'type':'turn.completed','usage':{{'input_tokens':5000 if mode=='high-cache' else 20,'cached_input_tokens':4990 if mode=='high-cache' else 10,'output_tokens':2000000 if mode=='budget' else 10}}}}))
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
            output.mkdir(); D['save'](output / 'items.json', [entry, file] if kwargs else [coverage])
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}
        with patch.dict(D['T'], {'collect': collect}), patch.dict(D['prepare'].__globals__, {
                'prepare_sources': lambda job, inventory, directory: directory.mkdir()}):
            prepared = D['prepare'](self.job_path, self.run)
        self.assertEqual(prepared['status'], 'PREPARED')
        self.inventory = D['load'](self.run / 'inputs/inventory.json')
        self.entry = next(r for r in self.inventory['requirements'] if r['kind'] == 'ENTRY')
        (self.seed / 'reads').mkdir()
        self.manifest, view, _ = F['fixture'](self.seed / 'reads', service=self.service, record=self.source)
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
        before = D['load'](self.run / 'state.json')
        with (self.run / '.runner.lock').open('w') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            result = subprocess.run([sys.executable, '-I', '-S', str(ROOT / 'skills/codeclew/scripts/docs_run.py'),
                                     'review-template', '--run-dir', str(self.run)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        template = json.loads(result.stdout)
        self.assertEqual(template['schema'], D['COMPACT_REVIEW'])
        self.assertTrue(all(row['disposition'] is None for row in template['requirements'].values()))
        self.assertEqual(D['load'](self.run / 'state.json'), before)
        self.assertFalse((self.run / 'ready').exists())

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
            {'usage':{'input_tokens':5000,'cached_input_tokens':4990,'output_tokens':10,'reasoning_output_tokens':4}},
            {'usage':{'input_tokens':30,'cached_input_tokens':5,'output_tokens':7,'reasoning_output_tokens':2}}])
        self.assertEqual(result['budgetTokens'], 52)
        self.assertEqual(result['rawTotals'], {'input_tokens':5030,'cached_input_tokens':4995,
                                             'output_tokens':17,'reasoning_output_tokens':6})
        self.assertEqual(result['totalInputOutputFootprint'], 5047)

    def test_missing_cache_counter_stays_explicitly_unavailable_across_repairs(self):
        result = D['token_accounting']([
            {'usage':{'input_tokens':100,'output_tokens':20}},
            {'usage':{'input_tokens':50,'cached_input_tokens':45,'output_tokens':10}}])
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


if __name__ == '__main__':
    unittest.main()
