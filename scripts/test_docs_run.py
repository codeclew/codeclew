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
print(json.dumps({{'type':'thread.started','thread_id':'different-test-session' if mode=='successor-thread-switch' else 'owned-test-session'}}),flush=True)
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
 path=draft/'original.json'
 path.write_text(path.read_text().replace('[Missing](absent.html)','Repaired local link.'))
usage={{'input_tokens':5000 if mode=='high-cache' else 20,'cached_input_tokens':4990 if mode=='high-cache' else 10,'output_tokens':2000000 if mode=='budget' else 10}}
if mode=='uneven-units':usage={{'input_tokens':2000 if 'resume' in sys.argv else 130000,'cached_input_tokens':0,'output_tokens':0}}
counter=Path.cwd()/'backend-usage.json'
if mode in ('successor-accounting','successor-decreasing'):
 counter=seed.parent/'session-usage.json'
 usage={{'input_tokens':5 if mode=='successor-decreasing' else 3 if 'resume' in sys.argv else 8,'cached_input_tokens':0,'output_tokens':0}}
if mode=='recorded-weather':
 prior=json.loads(counter.read_text()) if 'resume' in sys.argv and counter.exists() else None
 if prior is None:usage={{'input_tokens':1771674,'cached_input_tokens':1688320,'output_tokens':36956,'reasoning_output_tokens':24378}}
 elif prior['input_tokens']==1771674:usage={{'input_tokens':1021621,'cached_input_tokens':982656,'output_tokens':25588,'reasoning_output_tokens':18720}}
 else:usage={{'input_tokens':20,'cached_input_tokens':10,'output_tokens':10,'reasoning_output_tokens':0}}
if mode!='successor-decreasing' and 'resume' in sys.argv and counter.exists():
 previous=json.loads(counter.read_text())
 usage={{key:value+previous.get(key,0) for key,value in usage.items()}}
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
            output.mkdir(); D['save'](output / 'items.json', [entry, file] if kwargs else [coverage])
            return {'status': 'COMPLETE', 'omittedRecords': 0,
                    'identity': {'snapshot': F['SNAPSHOT'], 'contextDigest': 'owned-context', 'inputDigest': 'owned-input'}}
        self.collect = collect
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
        overview = self.original['pages'][1]['fragments'][1]
        for row in self.review['requirements']:
            requirement = next(r for r in self.inventory['requirements'] if r['id'] == row['requirement'])
            if requirement['kind'] == 'SERVICE':
                row['topics'] = {topic: {'disposition':'EXPLAINED','page':requirement['page'],
                    'claim':'overview','evidence':[self.handle],'fragmentDigest':D['digest'](overview)}
                    for topic in requirement['topics']}
                row['sourceNotes'] = {'id':'source-notes','text':'Source block: ' + self.source['text'],'evidence':[self.handle]}
        self.navigation['services'][0]['data'] = [{'title':'Reservation data','target':self.service + '.md'}]
        self.seed_files()
        # These controls start from authored labels; scaffold rejection has its
        # own positive/negative controls below and is never bypassed in product.
        D['W']['page_info'](self.run,self.entry['page'],self.service,'Reserve an item',False,D,summary='Accept a reservation')
        D['W']['page_info'](self.run,self.service+'.md',self.service,'Reservation data',True,D)

    def seed_files(self):
        for name, value in [('original.json', self.original), ('navigation.json', self.navigation), ('self-review.json', self.review)]:
            D['save'](self.seed / name, value)

    def test_author_projection_preserves_scope_and_addresses_without_proof_metadata(self):
        before = D['digest'](self.inventory)
        projected = D['load'](self.run / 'workspace/inventory.json')
        self.assertEqual(projected, D['author_inventory'](self.inventory))
        self.assertEqual(projected['services'][self.service]['coverage'], self.inventory['services'][self.service]['coverage'])
        for actual, original in zip(projected['requirements'], self.inventory['requirements']):
            for field in ('id', 'kind', 'service', 'page', 'handlerId', 'method', 'path', 'entryKind', 'file', 'topics', 'authority', 'boundaries'):
                if field in original:self.assertEqual(actual[field], original[field])
            self.assertNotIn('sourceIds', actual)
        self.assertNotIn('work', projected['services'][self.service])
        self.assertNotIn('sourceInventory', projected['services'][self.service])
        self.assertEqual(D['digest'](self.inventory), before)
        D['inputs_unchanged'](self.run)
        projected['requirements'][0]['page'] = 'forged.md'
        D['save'](self.run / 'workspace/inventory.json', projected)
        with self.assertRaises(D['Stop']) as error:D['inputs_unchanged'](self.run)
        self.assertEqual(error.exception.code, 'FROZEN_INPUTS_CHANGED')

    def test_navigation_starter_keeps_native_start_kinds_and_exact_targets(self):
        inventory = json.loads(json.dumps(self.inventory))
        for number, kind in enumerate(('KAFKA_LISTENER', 'SCHEDULED_JOB', 'OTHER')):
            row = {**self.entry, 'id': 'extra-' + str(number), 'page': 'extra-' + str(number) + '.md', 'entryKind': kind}
            row.pop('method'); row.pop('path')
            inventory['requirements'].append(row)
        nav = D['navigation_starter'](inventory)
        service = nav['services'][0]
        entries = [r for r in inventory['requirements'] if r['kind'] == 'ENTRY']
        self.assertEqual([s['kind'] for s in service['starts']], ['http', 'kafka', 'scheduled', 'other'])
        self.assertEqual([s['target'] for s in service['starts']], [r['page'] + '#handler' for r in entries])
        self.assertEqual(len(service['pages']), len(set(service['pages'])))
        self.assertTrue(all(r['page'] in service['pages'] for r in inventory['requirements']))
        self.assertEqual(nav['routes'], [])

    def test_review_template_is_read_only_but_runtime_gate_still_runs(self):
        D['save'](self.run / 'workspace/draft/original.json', self.original)
        before = D['tree'](self.run)
        def blocked(*args, **kwargs):raise PermissionError('Native receipts are outside author writable roots')
        with patch.dict(D['review_for_run'].__globals__, {'command': blocked}), patch.object(subprocess, 'run', blocked):
            review = D['review_for_run'](self.run)
            self.assertEqual(review['schema'], D['COMPACT_REVIEW'])
            self.assertEqual(set(review['requirements']), {r['id'] for r in self.inventory['requirements']})
            with self.assertRaisesRegex(PermissionError, 'Native receipts'):
                D['inputs_unchanged'](self.run)
        self.assertEqual(D['tree'](self.run), before)
        self.assertFalse((self.run / 'runtime-checks').exists())
        self.assertTrue(all(answer['disposition'] is None for answer in review['requirements'].values()))
        changed = D['load'](self.run / 'workspace/inventory.json');changed['requirements'].pop()
        D['save'](self.run / 'workspace/inventory.json', changed)
        with self.assertRaises(D['Stop']) as error:D['review_for_run'](self.run)
        self.assertEqual(error.exception.code, 'FROZEN_INPUTS_CHANGED')

    def test_start_scaffold_local_final_parity_and_supported_label_update_preserves_identity(self):
        self.navigation=D['navigation_starter'](self.inventory);self.seed_files();self.generated()
        local=self.local_check(self.entry['page']);final=D['verify'](self.run)
        self.assertEqual(local['code'],'NAVIGATION_SCAFFOLD_UNFINISHED')
        self.assertEqual(final['checks']['output']['code'],local['code'])
        self.assertIn('--summary',local['recovery'])
        before=D['load'](self.run/'workspace/draft/navigation.json')
        hashes={self.entry['page']:'old',self.service+'.md':'preserved'}
        D['save'](self.run/'workspace/draft/reviewed-pages.json',hashes)
        result=subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
            'page-info','--run-dir',str(self.run),'--page',self.entry['page'],'--service',self.service,
            '--title','Reserve an item','--summary','Validate the reservation and return the stored result.'],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        after=D['load'](self.run/'workspace/draft/navigation.json')
        expected=json.loads(json.dumps(before));expected['services'][0]['starts'][0].update(
            title='Reserve an item',summary='Validate the reservation and return the stored result.')
        self.assertEqual(after,expected)
        self.assertEqual(D['load'](self.run/'workspace/draft/reviewed-pages.json'),{self.service+'.md':'preserved'})
        D['W']['validate_start_labels'](self.run,D)
        # Only exact starter fields are rejected, not an ordinary mention.
        after['services'][0]['starts'][0]['summary']='The old message "Describe this entry from its retained source." is returned only in the documented compatibility case.'
        D['save'](self.run/'workspace/draft/navigation.json',after);D['W']['validate_start_labels'](self.run,D)
        with self.assertRaises(D['Stop']):D['W']['page_info'](self.run,self.entry['page'],self.service,'HTTP_ENDPOINT',False,D,summary='Action')

    def test_fragment_targets_are_stable_idempotent_and_preserve_text_handles_and_legacy_headings(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        original=D['load'](self.run/'workspace/draft/original.json')
        untouched={p['page']:D['digest'](p) for p in original['pages']}
        target=None
        for heading in ['Database and broker effects','Persistence consequences']:
            text='## '+heading+'\n\nStore the result.\n\n## Stable heading\n\nRead the result.'
            result=D['W']['put'](self.run,'details.md','effects',text,[self.handle],False,D)
            self.assertEqual(result['fragmentTarget'],'details.md#clew-fragment-effects')
            self.assertEqual(D['W']['summary'](result)['fragmentTarget'],result['fragmentTarget'])
            target=result['fragmentTarget']
            pages=D['load'](self.run/'workspace/draft/original.json')
            page=next(p for p in pages['pages'] if p['page']=='details.md')
            fragment=next(f for f in page['fragments'] if f.get('id')=='effects')
            self.assertEqual(fragment['text'].strip(),text);self.assertEqual(fragment['evidence'],[self.handle])
            self.assertEqual(sum('id="clew-fragment-effects"' in f['text'] for f in page['fragments']),1)
            raw,request,_=D['M']['compose_pages']({'pages':[page]})
            claim=request['pages'][0]['claims'][0]
            self.assertEqual(raw['details.md'][claim['startByte']:claim['endByte']],fragment['text'].encode())
            self.assertEqual({p['page']:D['digest'](p) for p in pages['pages'] if p['page'] in untouched},untouched)
            body=self.local_page()
            sequence=json.loads(body['fragments'][1]['text'].split('```clew-sequence\n')[1].split('```')[0])
            sequence['steps']+=[{'from':self.entry['handlerId'],'to':self.entry['handlerId'],'kind':'note','label':'Effects','target':target},
                               {'from':self.entry['handlerId'],'to':self.entry['handlerId'],'kind':'note','label':'Stable section','target':'details.md#stable-heading'}]
            D['W']['put'](self.run,self.entry['page'],'entry',json.dumps(sequence),[self.handle],True,D)
            checked=self.local_check(self.entry['page']);self.assertEqual(checked['status'],'UNIT_CHECKED',checked)
            untouched={p['page']:D['digest'](p) for p in D['load'](self.run/'workspace/draft/original.json')['pages'] if p['page']!='details.md'}
        shown=self.show_cli('--page','details.md','--fragment','effects','--limit','0')
        self.assertIn('Reader target: '+target,shown)

    def test_preserved_fragment_address_cli_is_readable_and_rejects_orphan_duplicate_and_missing(self):
        self.readmit_budget(100,1);self.generated()
        name=self.service+'.md';before=D['load'](self.run/'workspace/draft/original.json')
        claim=next(f for p in before['pages'] if p['page']==name for f in p['fragments'] if f.get('id')=='overview')
        for attempt in range(2):
            snapshot=D['tree'](self.run) if attempt else None
            done=subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
                'page-info','--run-dir',str(self.run),'--page',name,'--fragment','overview'],capture_output=True,text=True)
            self.assertEqual(done.returncode,0,done.stdout+done.stderr)
            self.assertEqual(json.loads(done.stdout)['fragmentTarget'],name+'#clew-fragment-overview')
            if attempt:self.assertEqual(snapshot,D['tree'](self.run))
        after=D['load'](self.run/'workspace/draft/original.json')
        selected=next(p for p in after['pages'] if p['page']==name)
        self.assertEqual(next(f for f in selected['fragments'] if f.get('id')=='overview'),claim)
        self.assertEqual(sum(f['text'].strip()=='<a id="clew-fragment-overview"></a>' for f in selected['fragments']),1)
        self.assertEqual(next(p for p in before['pages'] if p['page']==self.entry['page']),next(p for p in after['pages'] if p['page']==self.entry['page']))
        with self.assertRaises(D['Stop']) as error:D['W']['address_fragment'](self.run,name,'missing',D)
        self.assertEqual(error.exception.code,'FRAGMENT_ADDRESS_NOT_FOUND')
        inline=[{'text':'Prefix '},{'id':'inline','text':'phrase','evidence':[self.handle]},{'text':' suffix'}]
        with self.assertRaises(D['Stop']) as error:D['W']['address_body'](inline,name,'inline',D)
        self.assertEqual(error.exception.code,'FRAGMENT_ADDRESS_REQUIRES_BLOCK')
        self.assertEqual(inline[1]['text'],'phrase')
        broken=json.loads(json.dumps(after));page=next(p for p in broken['pages'] if p['page']==name)
        page['fragments']=[f for f in page['fragments'] if f.get('id')!='overview']
        page['fragments'].append({'id':'retained','text':'Retained explanation.','evidence':[self.handle]})
        before_tree=D['tree'](self.run/'workspace/draft')
        path=self.root/'orphan.json';D['save'](path,{'pages':[page]})
        with self.assertRaises(D['Stop']) as error:D['W']['validate_fragment_addresses'](broken,D)
        self.assertEqual(error.exception.details['issues'][0]['code'],'FRAGMENT_ADDRESS_ORPHANED')
        page['fragments']=[f for f in page['fragments'] if not (set(f)=={'text'} and f['text'].strip()=='<a id="clew-scope"></a>')]
        D['save'](path,{'pages':[page]})
        with self.assertRaises(D['Stop']) as error:D['W']['save_pages'](self.run,path,D)
        self.assertEqual(error.exception.code,'INVALID_FRAGMENT_ADDRESS')
        self.assertEqual(before_tree,D['tree'](self.run/'workspace/draft'))
        duplicate=json.loads(json.dumps(after));page=next(p for p in duplicate['pages'] if p['page']==name)
        page['fragments'].append({'text':'<a id="clew-fragment-overview"></a>'})
        with self.assertRaises(D['Stop']) as error:D['W']['validate_fragment_addresses'](duplicate,D)
        self.assertEqual(error.exception.details['issues'][0]['code'],'DUPLICATE_FRAGMENT_ADDRESS')
        self.original=broken;self.seed_files();self.generated()
        local=self.local_check(self.entry['page']);final=D['verify'](self.run)
        self.assertEqual(local['code'],'INVALID_FRAGMENT_ADDRESS')
        self.assertEqual(final['checks']['output']['code'],local['code'])

    def test_fragment_target_collision_and_missing_keep_actual_local_and_final_renderer_gates(self):
        self.readmit_budget(100,2);self.generated();self.save_local(self.local_page());name=self.service+'.md'
        result=D['W']['address_fragment'](self.run,name,'overview',D);target=result['fragmentTarget']
        diagram=json.loads(self.original['pages'][0]['fragments'][1]['text'].split('```clew-sequence\n')[1].split('```')[0])
        diagram['steps'].append({'from':self.entry['handlerId'],'to':self.entry['handlerId'],'kind':'note','label':'Details','target':target})
        D['W']['put'](self.run,self.entry['page'],'entry',json.dumps(diagram),[self.handle],True,D)
        self.original=D['load'](self.run/'workspace/draft/original.json');self.seed_files();self.generated()
        self.assertEqual(self.local_check(self.entry['page'])['status'],'UNIT_CHECKED')
        self.assertEqual(D['verify'](self.run)['status'],'READY')
        D['W']['put'](self.run,name,'extra','## Clew fragment overview\n\nAdditional explanation.',[self.handle],False,D)
        self.original=D['load'](self.run/'workspace/draft/original.json')
        self.review['claims'].append({'page':name,'claim':'extra','evidence':[self.handle],'disposition':'SUPPORTED'})
        self.seed_files();self.generated()
        local=self.local_check(self.entry['page']);final=D['verify'](self.run)
        self.assertEqual(local['code'],'FRAGMENT_TARGET_INVALID',local)
        self.assertEqual(local['details']['response']['details']['actual'],2)
        self.assertIn('./docs show',local['recovery'])
        self.assertEqual(final['checks']['finalize']['status'],'FAIL',final)
        # Removing an address also leaves the canonical reference invalid; it is
        # never silently reinterpreted as a differently named heading or claim.
        current=D['load'](self.run/'workspace/draft/original.json')
        for page in current['pages']:
            if page['page']==name:
                page['fragments']=[f for f in page['fragments'] if not (set(f)=={'text'} and 'id="clew-fragment-overview"' in f['text'])]
                next(f for f in page['fragments'] if f.get('id')=='extra')['text']='## Other heading\n\nAdditional explanation.'
        D['save'](self.run/'workspace/draft/original.json',current)
        missing=self.local_check(self.entry['page'])
        self.assertEqual(missing['code'],'FRAGMENT_TARGET_INVALID',missing)
        self.assertEqual(missing['details']['response']['details']['actual'],0)

    def test_service_topic_closure_references_absence_evidence_and_staleness_are_structural_only(self):
        entries={self.handle:{'source':self.source}}
        requirement=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        accounts=json.loads(json.dumps(self.review['requirements']))
        row=next(r for r in accounts if r['requirement']==requirement['id'])
        def validate(rows=accounts,original=None,navigation=None,record=False):
            return D['P']['validate'](self.inventory,rows,original or self.original,navigation or self.navigation,entries,D,record=record)
        self.assertIsNone(validate())  # No semantic PASS is produced.
        topic='configuration';row['topics'][topic].update(disposition='EXPLICIT_UNKNOWN',reason='The retained block contains no configuration declaration; deployment configuration is outside this source.')
        self.assertIsNone(validate())
        cases=[('SERVICE_TOPICS_REQUIRED',lambda r:r['topics'].pop('contracts')),
            ('SERVICE_TOPIC_REFERENCE_REQUIRED',lambda r:r['topics']['contracts'].update(claim='missing')),
            ('SERVICE_TOPIC_REFERENCE_REQUIRED',lambda r:r['topics']['contracts'].update(claim='clew-scope')),
            ('SERVICE_TOPIC_ABSENCE_BOUND_REQUIRED',lambda r:r['topics']['configuration'].pop('reason')),
            ('SERVICE_SOURCE_NOTES_REQUIRED',lambda r:r.pop('sourceNotes')),
            ('SERVICE_TOPIC_REVIEW_STALE',lambda r:r['topics']['contracts'].update(fragmentDigest='old'))]
        for code,change in cases:
            bad=json.loads(json.dumps(accounts));change(next(r for r in bad if r['requirement']==requirement['id']))
            with self.assertRaises(D['Stop']) as error:validate(bad)
            self.assertIn(code,{i['code'] for i in error.exception.details['issues']})
        foreign=json.loads(json.dumps(self.navigation));foreign['services'][0]['id']='another-service'
        with self.assertRaises(D['Stop']) as error:validate(navigation=foreign)
        self.assertIn('SERVICE_TOPIC_REFERENCE_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        nav=json.loads(json.dumps(self.navigation));nav['services'][0]['data']=[]
        with self.assertRaises(D['Stop']) as error:validate(navigation=nav)
        self.assertIn('SERVICE_DATA_NAVIGATION_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        stale=json.loads(json.dumps(self.original));stale['pages'][1]['fragments'][1]['text']+='Changed meaning.'
        with self.assertRaises(D['Stop']) as error:validate(original=stale)
        self.assertIn('SERVICE_TOPIC_REVIEW_STALE',{i['code'] for i in error.exception.details['issues']})
        validate(original=stale,record=True);validate(original=stale)

    def test_missing_service_topic_local_final_parity_and_source_notes_supported_path(self):
        requirement=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        bad=json.loads(json.dumps(self.review));row=next(r for r in bad['requirements'] if r['requirement']==requirement['id'])
        row.pop('topics');D['save'](self.seed/'self-review.json',bad);self.generated()
        local=self.local_check(requirement['page']);final=D['verify'](self.run)
        self.assertEqual(local['code'],'SERVICE_TOPIC_CLOSURE_REQUIRED',local)
        self.assertEqual(final['checks']['selfReview']['code'],local['code'])
        notes=self.root/'source-notes.md';notes.write_text('Source reservation block: '+self.source['text'])
        done=subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
            'review-page','--run-dir',str(self.run),'--page',requirement['page'],'--text-file',str(notes),'--evidence',self.handle],capture_output=True,text=True)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        self.assertEqual(json.loads(done.stdout)['status'],'SOURCE_NOTES_SAVED')
        shell=D['W']['review_page'](self.run,requirement['page'],None,D)
        self.assertEqual(shell['requirements'][requirement['id']]['sourceNotes']['text'],notes.read_text())
        self.assertTrue(all(v['disposition'] is None for v in shell['requirements'][requirement['id']]['topics'].values()))
        for identity,account in shell['requirements'].items():
            account.update(disposition='EXPLAINED',section='reservations')
            kind=next(r['kind'] for r in self.inventory['requirements'] if r['id']==identity)
            if kind=='DISCOVERY':account.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
            if kind=='SERVICE':
                for topic in account['topics']:
                    account['topics'][topic]={'disposition':'EXPLAINED','page':requirement['page'],'claim':'overview'}
        shell['claims']={key:'SUPPORTED' for key in shell['claims']}
        recorded=D['W']['review_page'](self.run,requirement['page'],shell,D)
        self.assertFalse(recorded['semanticTruthAssessed'])
        stored=D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]
        self.assertEqual(stored['topics']['contracts']['evidence'],[self.handle])
        self.assertEqual(stored['topics']['contracts']['fragmentDigest'],D['digest'](self.original['pages'][1]['fragments'][1]))

    def test_minimal_review_decisions_reuse_saved_own_notes_and_refuse_absent_invalid_or_unjudged(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        name=self.service+'.md';requirement=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        def call(*args):
            return subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
                'review-page','--run-dir',str(self.run),'--page',name,*args],capture_output=True,text=True)
        text=self.root/'outline.md';text.write_text('Source reservation block: '+self.source['text'])
        saved=call('--text-file',str(text),'--evidence',self.handle);self.assertEqual(saved.returncode,0,saved.stdout)
        shell=json.loads(call().stdout);payload={'claims':{key:'SUPPORTED' for key in shell['claims']},'requirements':{}}
        for identity,row in shell['requirements'].items():
            kind=next(r['kind'] for r in self.inventory['requirements'] if r['id']==identity)
            if kind=='SERVICE':
                payload['requirements'][identity]={'disposition':'EXPLAINED','topics':{
                    topic:{'disposition':'EXPLAINED','page':name,'claim':'overview'} for topic in row['topics']}}
            else:
                payload['requirements'][identity]={'disposition':'EXPLAINED','section':row['section'],
                    'discoveryDisposition':'NO_ADDITIONAL_STARTS','discoveredEntries':[]}
        path=self.root/'decisions.json';D['save'](path,payload);before=path.read_bytes()
        done=call('--input',str(path));self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        recorded=D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]
        self.assertEqual(recorded['sourceNotes'],shell['requirements'][requirement['id']]['sourceNotes'])
        self.assertEqual(recorded['section'],shell['requirements'][requirement['id']]['section'])
        self.assertEqual(before,path.read_bytes())
        review_path=self.run/'workspace/draft/self-review.json';review_before=review_path.read_bytes()
        note=self.run/'workspace/notes'/(name+'.json');raw=note.read_bytes()
        note.unlink();failed=call('--input',str(path));self.assertNotEqual(failed.returncode,0)
        self.assertIn('SERVICE_SOURCE_NOTES_REQUIRED',failed.stdout);note.write_bytes(raw)
        invalid=D['load'](note);invalid['fragments'][0]['text']='';D['save'](note,invalid)
        failed=call('--input',str(path));self.assertNotEqual(failed.returncode,0)
        self.assertIn('SERVICE_SOURCE_NOTES_REQUIRED',failed.stdout);note.write_bytes(raw)
        explicit=json.loads(json.dumps(payload));explicit['requirements'][requirement['id']]['sourceNotes']=None
        D['save'](path,explicit);failed=call('--input',str(path));self.assertNotEqual(failed.returncode,0)
        self.assertIn('SERVICE_SOURCE_NOTES_REQUIRED',failed.stdout)
        unjudged=json.loads(json.dumps(payload));unjudged['requirements'][requirement['id']]['topics']['contracts']['disposition']=None
        D['save'](path,unjudged);failed=call('--input',str(path));self.assertNotEqual(failed.returncode,0)
        self.assertIn('SERVICE_TOPIC_ASSESSMENT_REQUIRED',failed.stdout)
        self.assertEqual(review_before,review_path.read_bytes())
        unjudged=json.loads(json.dumps(payload));unjudged['claims'][next(iter(unjudged['claims']))]=None
        D['save'](path,unjudged);failed=call('--input',str(path));self.assertNotEqual(failed.returncode,0)
        self.assertIn('CLAIM_ASSESSMENT_REQUIRED',failed.stdout)
        manifest,view,_=F['fixture'](self.root,'foreign-note',service='another-service',work='other-work')
        foreign_handle=view['records'][0]['cite'];foreign=D['M']['C']['load'](manifest)[3]
        cached=D['load'](note);cached['fragments'][0]['evidence']=[foreign_handle];D['save'](note,cached)
        with patch.dict(D['P'],{'sources':lambda *a:{self.handle:{'source':self.source},**foreign}}):
            with self.assertRaises(D['Stop']) as error:D['W']['review_page'](self.run,name,json.loads(json.dumps(payload)),D)
        self.assertIn('SERVICE_SOURCE_NOTES_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        self.assertEqual(review_before,review_path.read_bytes());note.write_bytes(raw)
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

    def generated(self):
        return D['author'](self.run)

    def local_page(self):
        page = json.loads(json.dumps(self.original['pages'][0]))
        page['fragments'][0]['text'] = page['fragments'][0]['text'].replace('<a id="handler"></a>', '')
        return page

    def save_local(self, page):
        path = self.run / 'workspace/pages' / (page['page'] + '.json')
        D['save'](path, {'pages': [page]})
        return D['W']['save_pages'](self.run, path, D)

    def local_check(self, page):
        return D['W']['check'](self.run, [page], False, D)

    def local_reads(self):
        shutil.copytree(self.seed / 'reads', self.run / 'workspace/draft/reads')

    def test_registered_discovery_uses_same_local_final_binding_and_scoped_repair(self):
        self.local_reads()
        parent = next(r for r in self.inventory['requirements'] if r['kind'] == 'DISCOVERY')
        declaration = {'id': 'events', 'kind': 'KAFKA', 'page': 'events.md', 'title': 'Apply reservation events',
                       'evidence': ['invented-handle'], 'disposition': None}
        registered = D['W']['register_entry'](self.run, parent['id'], declaration, D)
        row = registered['entry'];self.assertEqual(row['handlerId'], 'discovered-events')
        other = (self.run / 'workspace/pages' / (self.entry['page'] + '.json')).read_bytes()
        D['W']['put'](self.run, 'events.md', 'meaning', 'Apply an incoming reservation event.', [self.handle], False, D)
        self.assertIn('events.md', D['W']['unfinished'](D['load'](self.run / 'workspace/draft/original.json')))
        sequence = {'title': 'Apply event', 'participants': [{'id': 'broker', 'title': 'Broker'}, {'id': 'wrong', 'title': 'Application'}],
                    'steps': [{'from': 'broker', 'to': 'wrong', 'kind': 'call', 'label': 'Reservation record'}]}
        D['W']['put'](self.run, 'events.md', 'flow', json.dumps(sequence), [self.handle], True, D)
        failed = self.local_check('events.md');self.assertEqual(failed['code'], 'SEQUENCE_ENTRY_MISMATCH', failed)
        details = failed['details']['response']['details']
        self.assertEqual(details['expected']['handlerId'], 'discovered-events')
        self.assertEqual(details['actual']['firstTo'], 'wrong');self.assertIn('./docs sequence', details['recovery'])
        self.assertIn('./docs sequence', failed['recovery']);self.assertNotIn('call save-pages', failed['recovery'])
        sequence['participants'][1]['id'] = 'handler';sequence['steps'][0]['to'] = 'handler'
        D['W']['put'](self.run, 'events.md', 'flow', json.dumps(sequence), [self.handle], True, D)
        wrong_evidence = self.local_check('events.md')
        self.assertEqual(wrong_evidence['code'], 'INVALID_DISCOVERED_ENTRY_ACCOUNT', wrong_evidence)
        content_before = D['load'](self.run / 'workspace/draft/original.json')
        declaration['evidence'] = [self.handle]
        repair = subprocess.run([str(self.run / 'workspace/docs'), 'register-entry', '--requirement', parent['id'],
                                 '--page', 'events.md', '--evidence', self.handle], capture_output=True, text=True)
        self.assertEqual(repair.returncode, 0, repair.stdout + repair.stderr)
        repaired_registration = json.loads(repair.stdout)
        self.assertEqual(repaired_registration['entry'], registered['entry'])
        self.assertEqual(content_before, D['load'](self.run / 'workspace/draft/original.json'))
        checked = self.local_check('events.md');self.assertEqual(checked['status'], 'UNIT_CHECKED', checked)
        self.assertEqual(other, (self.run / 'workspace/pages' / (self.entry['page'] + '.json')).read_bytes())
        assembled = D['load'](self.run / 'workspace/draft/original.json')
        new_page = next(p for p in assembled['pages'] if p['page'] == 'events.md')
        self.assertEqual(sum('<a id="handler"></a>' in f['text'] for f in new_page['fragments']), 1)
        self.original['pages'].append(new_page)
        self.navigation = D['load'](self.run / 'workspace/draft/navigation.json')
        for account in self.review['requirements']:
            if account['requirement'] == parent['id']:
                account.update(discoveryDisposition='ADDITIONAL_STARTS', discoveredEntries=[{
                    **D['W']['registered_accounts'](self.run, D)[0]['discoveredEntries'][0], 'disposition': 'EXPLAINED'}])
        self.review['claims'] += [{'page': 'events.md', 'claim': f['id'], 'evidence': f['evidence'], 'disposition': 'SUPPORTED'}
                                 for f in new_page['fragments'] if 'id' in f]
        self.seed_files();self.generated();final = D['verify'](self.run)
        self.assertEqual(final['status'], 'READY', final)
        self.assertIn(row['id'], final['checks']['entrySequences']['result']['entries'])

    def test_fragment_commands_serialize_text_and_page_reviews_survive_other_edits(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        for page in (self.entry['page'], self.service + '.md'):
            shell = D['W']['review_page'](self.run, page, None, D)
            for row in shell['requirements'].values():
                row['disposition'] = 'EXPLAINED'
                if 'discoveryDisposition' in row:row.update(discoveryDisposition='NO_ADDITIONAL_STARTS', discoveredEntries=[])
                if 'topics' in row:
                    fixture=next(a for a in self.review['requirements'] if 'topics' in a)
                    row.update(topics=json.loads(json.dumps(fixture['topics'])),sourceNotes=fixture['sourceNotes'])
            shell['claims'] = {key: 'SUPPORTED' for key in shell['claims']}
            D['W']['review_page'](self.run, page, shell, D)
        before = D['review_for_run'](self.run)
        self.assertEqual(before['claims'][self.service + '.md']['overview'], 'SUPPORTED')
        body = self.run / 'workspace/claim.md';body.write_text('## Input "value"\n\nA backslash \\ and `code` remain literal.\n')
        wrapper = self.run / 'workspace/docs'
        done = subprocess.run([str(wrapper), 'put', '--page', self.entry['page'], '--fragment', 'entry',
                               '--text-file', str(body), '--evidence', self.handle], capture_output=True, text=True)
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        compact = json.loads(done.stdout);self.assertNotIn('unchanged', compact);self.assertNotIn('unfinishedPages', compact)
        self.assertIn('unchangedCount', compact);self.assertTrue((self.run / 'workspace/last-save-pages.json').is_file())
        after = D['review_for_run'](self.run)
        self.assertIsNone(after['claims'][self.entry['page']]['entry'])
        self.assertEqual(after['claims'][self.service + '.md'], before['claims'][self.service + '.md'])
        page = next(p for p in D['load'](self.run / 'workspace/draft/original.json')['pages'] if p['page'] == self.entry['page'])
        self.assertEqual(next(f['text'].strip() for f in page['fragments'] if f.get('id') == 'entry'), body.read_text().strip())

    def prepare_show(self, *, cover_matches_candidates=True, requirement_views=None):
        cover_handle = self.handle
        if not cover_matches_candidates:
            (self.root / 'cover-reads').mkdir()
            _, cover_view, _ = F['fixture'](self.root / 'cover-reads', service=self.service,
                record={**self.source, 'id': self.source['id'] + '-union'})
            cover_handle = cover_view['records'][0]['cite']
            self.assertNotEqual(cover_handle, self.handle)
        def sources(job, inventory, directory):
            folder = directory / self.service;folder.mkdir(parents=True)
            paths = ['views/u16.md'] if requirement_views is None else requirement_views
            D['save'](folder / 'views.json', {Path(path).stem: {'coveringCitations': [cover_handle] if index == 0 else [],
                'view': path} for index, path in enumerate(paths)})
            D['save'](folder / 'requirements.json', {self.entry['id']: {'kind': 'ENTRY', 'page': self.entry['page'],
                'meaningReviewed': False, 'evidenceCandidates': [self.handle],
                'views': paths}})
            for path in paths:
                (folder / path).parent.mkdir(exist_ok=True);(folder / path).write_text(self.source['text'])
        self.run = self.root / 'show-run'
        with patch.dict(D['T'], {'collect': self.collect}), patch.dict(D['prepare'].__globals__, {'prepare_sources': sources}):
            D['prepare'](self.job_path, self.run)
        D['save'](self.run / 'workspace/draft/original.json', self.original)

    def show_cli(self, *args, success=True):
        done = subprocess.run(['./docs', 'show', *args], cwd=self.run / 'workspace', capture_output=True, text=True)
        self.assertEqual(done.returncode, 0 if success else 2, done.stdout + done.stderr)
        return done.stdout

    def test_show_page_fragment_sources_and_assessments_are_read_only(self):
        self.prepare_show()
        before = D['tree'](self.run)
        whole = self.show_cli('--page', self.entry['page'], '--limit', '0')
        for f in self.original['pages'][0]['fragments']:self.assertIn(f['text'], whole)
        self.assertIn('Fragment: (unannotated structure)', whole)
        self.assertIn('Cites: ' + self.handle, whole)
        self.assertIn('../inputs/author-source/reservations/views/u16.md', whole)
        selected = self.show_cli('--page', self.entry['page'], '--fragment', 'entry')
        self.assertIn(self.original['pages'][0]['fragments'][1]['text'], selected)
        self.assertNotIn('unannotated structure', selected)
        self.assertEqual(before, D['tree'](self.run))
        self.assertFalse((self.run / '.runner.lock').exists())
        self.assertFalse((self.run / 'workspace/.edit.lock').exists())
        assessment = D['W']['review_page'](self.run, self.entry['page'], None, D)
        self.assertEqual(set(assessment['claims']), {'entry'})
        self.assertEqual(before, D['tree'](self.run))
        bad = json.loads(self.show_cli('--page', self.entry['page'], '--fragment', 'invented', success=False))
        self.assertEqual(bad['code'], 'SHOW_FRAGMENT_NOT_FOUND')
        self.assertEqual(bad['details']['available'], ['entry'])
        missing = json.loads(self.show_cli('--page', 'missing.md', success=False))
        self.assertEqual(set(missing['details']['available']), {p['page'] for p in self.original['pages']})
        self.assertEqual(before, D['tree'](self.run))

    def test_show_large_unicode_content_has_lossless_explicit_continuation(self):
        import shlex
        self.prepare_show()
        fragment = self.original['pages'][0]['fragments'][1]
        fragment['text'] = 'First line\n' + ('Weather → città 🌦\n' * 650) + '\nLAST LINE\n'
        D['save'](self.run / 'workspace/draft/original.json', self.original)
        before = D['tree'](self.run)
        args = ['--page', self.entry['page'], '--fragment', 'entry']
        all_text = self.show_cli(*args, '--limit', '0').rsplit('\n[Characters ', 1)[0]
        chunks = [];continuations = []
        while True:
            output = self.show_cli(*args)
            content, footer = output.rsplit('\n[Characters ', 1)
            chunks.append(content)
            if 'Continue: ' not in footer:break
            command = footer.split('Continue: ', 1)[1].strip();continuations.append(command)
            args = shlex.split(command)[2:]
        self.assertTrue(continuations)
        self.assertEqual(''.join(chunks), all_text)
        self.assertIn(fragment['text'], all_text)
        self.assertIn('LAST LINE', chunks[-1])
        self.assertEqual(before, D['tree'](self.run))
        for flags in (('--offset', '-1'), ('--limit', '-1'), ('--offset', '999999')):
            self.show_cli('--page', self.entry['page'], *flags, success=False)

    def test_show_native_candidates_use_explicit_requirement_views_not_union_handles(self):
        self.prepare_show(cover_matches_candidates=False)
        before = D['tree'](self.run)
        requirements = D['load'](self.run / 'inputs/author-source/reservations/requirements.json')[self.entry['id']]
        cover = D['load'](self.run / 'inputs/author-source/reservations/views.json')['u16']['coveringCitations']
        self.assertTrue(set(requirements['evidenceCandidates']).isdisjoint(cover))
        selected = self.show_cli('--entry', self.entry['id'], '--limit', '0')
        item = json.loads(selected.split('\n', 1)[1].split('\nSource views:', 1)[0])
        self.assertEqual(item['evidence'], [self.handle]);self.assertEqual(item['authority'], self.entry['authority'])
        self.assertIn('../inputs/author-source/reservations/views/u16.md', selected)
        for args in (('--page', self.entry['page']), ('--page', self.entry['page'], '--fragment', 'entry')):
            output = self.show_cli(*args, '--limit', '0')
            self.assertIn('../inputs/author-source/reservations/views/u16.md', output)
            self.assertIn(self.original['pages'][0]['fragments'][1]['text'], output)
            self.assertIn('Cites: ' + self.handle, output)
        self.assertEqual(before, D['tree'](self.run))

    def test_show_does_not_guess_fragment_view_from_multiview_requirement(self):
        self.prepare_show(cover_matches_candidates=False, requirement_views=['views/u16.md', 'views/u20.md'])
        before = D['tree'](self.run)
        selected = self.show_cli('--entry', self.entry['id'], '--limit', '0')
        whole = self.show_cli('--page', self.entry['page'], '--limit', '0')
        for output in (selected, whole):
            self.assertIn('../inputs/author-source/reservations/views/u16.md', output)
            self.assertIn('../inputs/author-source/reservations/views/u20.md', output)
        fragment = self.show_cli('--page', self.entry['page'], '--fragment', 'entry', '--limit', '0')
        self.assertIn('no exact prepared link for these citations', fragment)
        self.assertNotIn('views/u16.md', fragment);self.assertNotIn('views/u20.md', fragment)
        self.assertIn('Cites: ' + self.handle, fragment)
        self.assertEqual(before, D['tree'](self.run))

    def test_show_entries_list_shape_and_routes_use_real_ids_and_complete_rows(self):
        self.prepare_show()
        parent = next(r for r in self.inventory['requirements'] if r['kind'] == 'DISCOVERY')
        declaration = {'id': 'events', 'kind': 'KAFKA', 'page': 'events.md', 'title': 'Apply reservation events',
                       'evidence': [self.handle], 'disposition': None}
        registered = D['W']['register_entry'](self.run, parent['id'], declaration, D)['entry']
        self.assertIsInstance(D['load'](self.run / 'workspace/draft/entries.json'), list)
        route = {'id': 'reserve-process', 'title': 'Reserve an item', 'narrative': {'page': 'reserve.html'},
                 'occurrences': [{'id': 'first', 'label': 'Start'}], 'outcomes': [{'id': 'last', 'label': 'Complete'}]}
        D['save'](self.run / 'workspace/draft/routes.json', {'routes': [route]})
        before = D['tree'](self.run)
        entries = self.show_cli('--entries', '--service', self.service)
        self.assertIn(registered['id'], entries);self.assertIn(self.entry['id'], entries)
        self.assertNotIn('sourceIds', entries)
        for identity in (registered['id'], self.entry['id']):
            selected = self.show_cli('--entry', identity, '--limit', '0')
            body = selected.split('\n', 1)[1].split('\nSource views:', 1)[0]
            row = json.loads(body)
            self.assertEqual(row['id'], identity)
            self.assertIn(self.handle, row['evidence'])
            self.assertEqual(row['target'], row['page'] + '#handler')
            self.assertNotIn('sourceIds', row)
            self.assertIn('views/u16.md', selected)
        routes = self.show_cli('--routes')
        self.assertIn('reserve-process | Reserve an item | reserve.html', routes)
        selected = self.show_cli('--route', route['id'], '--limit', '0')
        self.assertEqual(json.loads(selected.split('\n', 1)[1].rsplit('\n[Characters ', 1)[0]), route)
        for flag, code, choices in (('--entry', 'SHOW_ENTRY_NOT_FOUND', [self.entry['id'], registered['id']]),
                                     ('--route', 'SHOW_ROUTE_NOT_FOUND', [route['id']])):
            bad = json.loads(self.show_cli(flag, 'invented', success=False))
            self.assertEqual(bad['code'], code);self.assertEqual(bad['details']['available'], choices)
        self.assertEqual(before, D['tree'](self.run))

    def test_local_entry_check_honors_recorded_diagram_and_discovered_source_evidence(self):
        self.local_reads();page = self.local_page()
        diagram = json.loads(page['fragments'][1]['text'].split('```clew-sequence\n')[1].split('```')[0])
        unrelated = {'title': 'Other operation', 'participants': [{'id': 'x', 'title': 'Other'}, {'id':'y','title':'Peer'}],
                     'steps': [{'from': 'x', 'to': 'x', 'kind': 'note', 'label': 'Different operation'}]}
        page['fragments'][1]['text'] = '```clew-sequence\n' + json.dumps(unrelated) + '\n```\n\n```clew-sequence\n' + json.dumps(diagram) + '\n```\n'
        self.save_local(page)
        review = json.loads(json.dumps(self.review))
        next(r for r in review['requirements'] if r['requirement'] == self.entry['id'])['diagram'] = 1
        D['save'](self.run / 'workspace/draft/self-review.json', review)
        result = self.local_check(page['page']);self.assertEqual(result['status'], 'UNIT_CHECKED', result)
        parent = next(r for r in self.inventory['requirements'] if r['kind'] == 'DISCOVERY')
        D['W']['register_entry'](self.run, parent['id'], {'id':'events','kind':'OTHER','page':'events.md','title':'Apply event',
                                    'evidence':['invented-handle'],'disposition':None}, D)
        seq = {'title':'Apply event','participants':[{'id':'caller','title':'Caller'},{'id':'handler','title':'Application'}],
               'steps':[{'from':'caller','to':'handler','kind':'call','label':'Event'}]}
        D['W']['put'](self.run,'events.md','flow',json.dumps(seq),[self.handle],True,D)
        bad = self.local_check('events.md')
        self.assertEqual(bad['code'],'INVALID_DISCOVERED_ENTRY_ACCOUNT',bad)
        self.assertEqual(bad['details']['actual'],['invented-handle'])

    def test_registration_evidence_repair_invalidates_only_affected_reviews(self):
        parent = next(r for r in self.inventory['requirements'] if r['kind'] == 'DISCOVERY')
        declaration = {'id':'events','kind':'OTHER','page':'events.md','title':'Apply event',
                       'evidence':['invented-handle'],'disposition':None}
        D['W']['register_entry'](self.run,parent['id'],declaration,D)
        before = D['load'](self.run / 'workspace/draft/original.json')
        review = D['review_for_run'](self.run)
        row = review['requirements'][parent['id']];row['disposition']='EXPLAINED'
        row['discoveredEntries'][0]['disposition']='EXPLAINED'
        D['save'](self.run / 'workspace/draft/self-review.json',review)
        D['save'](self.run / 'workspace/draft/reviewed-pages.json',{
            parent['page']:'old-parent','events.md':'old-entry',self.entry['page']:'unaffected'})
        declaration['evidence']=[self.handle]
        D['W']['register_entry'](self.run,parent['id'],declaration,D)
        self.assertEqual(before,D['load'](self.run / 'workspace/draft/original.json'))
        hashes=D['load'](self.run / 'workspace/draft/reviewed-pages.json')
        self.assertEqual(hashes,{self.entry['page']:'unaffected'})
        repaired=D['load'](self.run / 'workspace/draft/self-review.json')['requirements'][parent['id']]
        self.assertIsNone(repaired['disposition']);self.assertIsNone(repaired['discoveredEntries'][0]['disposition'])
        self.assertEqual(repaired['discoveredEntries'][0]['evidence'],[self.handle])

    def test_small_route_and_navigation_updates_keep_unrelated_declarations(self):
        self.save_local({'page':'processes.md','fragments':[{'id':'catalog','text':'# Processes\nSource-backed reservation work.','evidence':[self.handle]}]})
        first = {'catalog': 'processes.html', 'route': {'title': 'First process', 'narrative': {'page': 'first.md'},
                 'origin': {'status': 'unknown', 'reason': 'Not bound'}, 'occurrences': [], 'branches': [], 'outcomes': [], 'coverage': {'outcomeInventory':'partial'}}}
        second = {'route': {'title': 'Second process', 'narrative': {'page': 'second.md'},
                  'origin': {'status': 'unknown', 'reason': 'Not bound'}, 'occurrences': [], 'branches': [], 'outcomes': [], 'coverage': {'outcomeInventory':'partial'}}}
        D['W']['route'](self.run, first, D)
        before = D['load'](self.run / 'workspace/draft/routes.json')['routes'][0]
        D['W']['route'](self.run, second, D)
        registry = D['load'](self.run / 'workspace/draft/routes.json')
        self.assertEqual(registry['routes'][0], before)
        self.assertEqual(registry['routes'][1]['narrative'], {'docset': 'docs', 'page': 'second.html'})
        first['catalog']='processes.md';first['route']['title']='Updated first process'
        D['W']['route'](self.run,first,D)
        updated_routes=D['load'](self.run / 'workspace/draft/routes.json')
        self.assertEqual([r['id'] for r in updated_routes['routes']],['first','second'])
        self.assertEqual(updated_routes['catalog']['page'],'processes.html')
        self.assertEqual(updated_routes['routes'][1],registry['routes'][1])
        self.assertEqual(D['W']['process_pages'](self.run, D), ['first.md', 'second.md'])
        navigation = D['load'](self.run / 'workspace/draft/navigation.json')
        starts = navigation['services'][0]['starts']
        D['W']['page_info'](self.run, 'data.md', self.service, 'Reservation data', True, D)
        updated = D['load'](self.run / 'workspace/draft/navigation.json')['services'][0]
        self.assertEqual(updated['starts'], starts)
        self.assertIn({'title': 'Reservation data', 'target': 'data.md'}, updated['data'])

    def test_route_origin_uses_registered_anchor_and_rejects_wrong_addresses_before_save(self):
        self.save_local({'page':'processes.md','fragments':[{'id':'catalog','text':'Source-backed catalog.','evidence':[self.handle]}]})
        value={'catalog':'processes.html','route':{'id':'reserve-route','title':'Reserve','narrative':{'page':'thread.md'},
            'origin':{'status':'known','occurrence':'start','kind':'http','title':'Reserve','summary':'Owned fixture'},
            'occurrences':[{'id':'start','actor':{'kind':'service','service':self.service},'role':'start','target':{'page':self.entry['page']}}],
            'branches':[],'outcomes':[],'coverage':{'outcomeInventory':'partial'}}}
        original=json.loads(json.dumps(value))
        D['W']['route'](self.run,value,D)
        self.assertEqual(value,original)
        path=self.run / 'workspace/draft/routes.json'
        stored=D['load'](path)
        self.assertEqual(stored['routes'][0]['occurrences'][0]['target'],
                         {'docset':'docs','page':self.entry['page'].removesuffix('.md')+'.html','anchor':'handler'})
        for change in ({'anchor':'wrong'}, {'anchor':''}, {'page':'missing.md'}, {'docset':'foreign'}):
            bad=json.loads(json.dumps(original));bad['route']['occurrences'][0]['target'].update(change)
            before=D['tree'](self.run / 'workspace/draft')
            with self.assertRaises(D['Stop']) as error:D['W']['route'](self.run,bad,D)
            issue=error.exception.details['issues'][0]
            self.assertEqual(issue['code'],'ROUTE_ORIGIN_START_MISMATCH')
            self.assertEqual(issue['routeId'],'reserve-route');self.assertEqual(issue['occurrenceId'],'start')
            self.assertEqual(issue['inputField'],'/route/occurrences/0/target')
            self.assertEqual(issue['expected'][0]['target']['anchor'],'handler')
            self.assertIn('./docs route',issue['recovery'])
            self.assertEqual(before,D['tree'](self.run / 'workspace/draft'))

        # Legacy stored declarations must fail locally with route-specific repair,
        # rather than reaching an opaque whole-site renderer error.
        stored['routes'][0]['occurrences'][0]['target'].pop('anchor')
        D['save'](path,stored)
        self.local_reads();self.save_local(self.local_page())
        before=D['tree'](self.run / 'workspace/draft')
        local=self.local_check(self.entry['page'])
        self.assertEqual(local['code'],'ROUTE_DECLARATION_INVALID')
        self.assertTrue(any(x.get('routeId')=='reserve-route' for x in local['locations']))
        self.assertIn('./docs route',local['recovery'])
        self.assertEqual(before,D['tree'](self.run / 'workspace/draft'))

    def test_route_preflight_rejects_bad_metadata_before_persisting_and_names_catalog(self):
        value={'catalog':'processes.html','route':{'id':'reserve-route','title':'Reserve','narrative':{'page':'thread.md'},
            'origin':{'status':'known','occurrence':'start','kind':'HTTP_ENDPOINT','title':'Reserve','summary':'Owned fixture'},
            'occurrences':[{'id':'start','actor':{'kind':'service','service':self.service},'role':'start','target':{'page':self.entry['page']}},
                           {'id':'finish','actor':{'kind':'service','service':self.service},'role':'finish','target':{'page':self.entry['page']}}],
            'branches':[{'id':'normal','title':'Normal','condition':'Owned branch','target':{'page':'thread.md'}}],'outcomes':[{'id':'done','branch':'normal','kind':'normal','title':'Done','target':{'page':'thread.md'},
                'internal':{'coverage':'partial','occurrences':['finish']},'end':{'status':'known','ownerOccurrence':'missing','boundary':'return','recipient':{'kind':'none'}}}],
            'coverage':{'outcomeInventory':'partial'}}}
        before=D['tree'](self.run / 'workspace/draft')
        with self.assertRaises(D['Stop']) as error:D['W']['route'](self.run,value,D)
        issues=error.exception.details['issues'];codes={i['code'] for i in issues}
        self.assertTrue({'ROUTE_TRIGGER_KIND_INVALID','ROUTE_TERMINAL_UNKNOWN','ROUTE_CATALOG_TARGET_REQUIRED'}<=codes)
        self.assertEqual(before,D['tree'](self.run / 'workspace/draft'))
        trigger=next(i for i in issues if i['code']=='ROUTE_TRIGGER_KIND_INVALID')
        self.assertEqual(trigger['routeId'],'reserve-route');self.assertEqual(trigger['inputField'],'/route/origin/kind')
        self.assertIn('./docs route',trigger['recovery'])
        self.save_local({'page':'processes.md','fragments':[{'id':'catalog','text':'Source-backed catalog.','evidence':[self.handle]}]})
        value['route']['origin']['kind']='http';value['route']['outcomes'][0]['end']['ownerOccurrence']='finish'
        with self.assertRaises(D['Stop']) as error:D['W']['route'](self.run,value,D)
        self.assertIn('ROUTE_TERMINAL_INTERNAL',{i['code'] for i in error.exception.details['issues']})
        value['route']['outcomes'][0]['internal']['occurrences']=[]
        self.assertEqual(D['W']['route'](self.run,value,D)['status'],'ROUTE_SAVED')
        registry=D['load'](self.run / 'workspace/draft/routes.json');registry['routes'][0]['origin']['kind']='HTTP_ENDPOINT'
        D['save'](self.run / 'workspace/draft/routes.json',registry)
        self.local_reads();self.save_local(self.local_page())
        local=self.local_check(self.entry['page'])
        self.assertEqual(local['code'],'ROUTE_DECLARATION_INVALID');self.assertTrue(local['locations'])
        self.assertIn('./docs route',local['recovery'])

    def test_discovery_contract_is_shared_at_page_local_finish_and_final(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        for page in [self.entry['page'],self.service+'.md']:
            shell=D['W']['review_page'](self.run,page,None,D)
            for row in shell['requirements'].values():
                row['disposition']='EXPLAINED'
                if 'discoveryDisposition' in row:row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
                if 'topics' in row:
                    fixture=next(a for a in self.review['requirements'] if 'topics' in a)
                    row.update(topics=json.loads(json.dumps(fixture['topics'])),sourceNotes=fixture['sourceNotes'])
            shell['claims']={key:'SUPPORTED' for key in shell['claims']}
            D['W']['review_page'](self.run,page,shell,D)
        parent=next(r for r in self.inventory['requirements'] if r['kind']=='DISCOVERY')
        correct=D['load'](self.run / 'workspace/draft/self-review.json')
        original=D['load'](self.run / 'workspace/draft/original.json')
        job,inventory,_=D['frozen_inputs'](self.run)
        _,entries=D['cite_entries'](job,inventory,self.run / 'workspace/draft',self.root / 'review-source-check')
        global_values={'unresolved':[],'visual':{'status':'UNAVAILABLE','reason':'Owned metadata control'}}
        correct.update(global_values);D['save'](self.run / 'workspace/draft/self-review.json',correct)
        for tag,found in [('EXPLAINED',[]),('ADDITIONAL_STARTS',[]),('NO_ADDITIONAL_STARTS',[{'id':'new'}]),(None,[])]:
            shell=D['W']['review_page'](self.run,parent['page'],None,D)
            shell['requirements'][parent['id']].update(discoveryDisposition=tag,discoveredEntries=found)
            before=D['tree'](self.run / 'workspace/draft')
            with self.assertRaises(D['Stop']) as error:D['W']['review_page'](self.run,parent['page'],shell,D)
            self.assertEqual(error.exception.code,'DISCOVERY_DISPOSITION_REQUIRED')
            issue=error.exception.details['issues'][0];self.assertEqual(issue['requirement'],parent['id']);self.assertEqual(issue['actual'],tag)
            self.assertEqual(before,D['tree'](self.run / 'workspace/draft'))
            corrupt=json.loads(json.dumps(correct));corrupt['requirements'][parent['id']].update(discoveryDisposition=tag,discoveredEntries=found)
            # Legacy/corrupt saved metadata: finish/local/final must reject it too.
            D['save'](self.run / 'workspace/draft/self-review.json',corrupt)
            with self.assertRaises(D['Stop']):D['W']['finish_review'](self.run,global_values,D)
            local=self.local_check(parent['page']);self.assertEqual(local['code'],'DISCOVERY_DISPOSITION_REQUIRED',local)
            resolved=D['resolve_review'](corrupt,self.inventory,entries,original)
            with self.assertRaises(D['Stop']) as error:D['review_accounts'](resolved,self.inventory,{p['page']:b'content' for p in original['pages']},entries,original)
            self.assertEqual(error.exception.details['issues'][0]['actual'],tag)
            D['save'](self.run / 'workspace/draft/self-review.json',correct)
        before=D['tree'](self.run / 'workspace/draft');self.assertEqual(D['W']['finish_review'](self.run,global_values,D)['status'],'GLOBAL_REVIEW_RECORDED')
        self.assertEqual(D['load'](self.run / 'workspace/draft/self-review.json')['claims'],correct['claims'])
        resolved=D['resolve_review'](D['load'](self.run / 'workspace/draft/self-review.json'),self.inventory,entries,original)
        D['review_accounts'](resolved,self.inventory,{p['page']:b'content' for p in original['pages']},entries,original)
        inv=json.loads(json.dumps(self.inventory));extra={**parent,'id':parent['id']+':another'};inv['requirements'].append(extra)
        with self.assertRaises(D['Stop']) as error:D['W']['validate_discovery_review'](inv,{},D)
        self.assertEqual({i['requirement'] for i in error.exception.details['issues']},{parent['id'],extra['id']})

    def test_repair_context_keeps_only_failed_checks_and_help_is_current(self):
        report={'status':'FAILED','checks':{'author':{'status':'PASS','tokenAccounting':{'rawTotals':'large preserved ledger'}},
            'selfReview':{'status':'FAIL','code':'DISCOVERY_DISPOSITION_REQUIRED','details':{'issues':[{'field':'/requirements/r/discoveryDisposition','actual':'EXPLAINED','expected':['NO_ADDITIONAL_STARTS'],'recovery':'./docs review-page --page service.md'}]}},
            'entrySequences':{'status':'NOT_RUN'},'provenance':{'status':'PASS','handles':785}}}
        before=json.loads(json.dumps(report));facts=D['repair_facts'](report)
        self.assertEqual(set(facts['checks']),{'selfReview'});self.assertEqual(report,before)
        self.assertNotIn('tokenAccounting',json.dumps(facts));self.assertEqual(facts['usageAndAttemptHistory'],'state.json')
        wrapper=self.run / 'workspace/docs'
        help_result=subprocess.run([str(wrapper),'--help'],capture_output=True,text=True)
        self.assertEqual(help_result.returncode,0,help_result.stdout+help_result.stderr)
        for term in ['route','review-page','finish-review','ADDITIONAL_STARTS','NO_ADDITIONAL_STARTS']:
            self.assertIn(term,help_result.stdout)
        self.readmit_budget(1000,1);self.generated()
        D['save'](self.run / 'report.json',report);D['author'](self.run,repair=True)
        prompt=(self.run / 'attempts/1/prompt.txt').read_text()
        sent,_=json.JSONDecoder().raw_decode(prompt)
        self.assertEqual(sent['failures'],facts)
        self.assertNotIn('large preserved ledger',prompt)
        self.assertEqual(D['load'](self.run / 'report.json'),report)
        self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']),2)

    def successor_parent(self, *, ready=False):
        self.readmit_budget(8,0);self.mode.write_text('successor-accounting')
        prepared=self.run / 'inputs/author-source' / self.service / 'read'
        shutil.copytree(self.seed / 'reads',prepared)
        (prepared.parent / 'view.md').write_text(self.source['text'])
        shutil.rmtree(self.seed / 'reads')  # Prepared carrier is authoritative; do not duplicate its cite handle.
        pins=D['load'](self.run / 'pins.json');pins['inputFiles']=D['tree'](self.run / 'inputs');D['save'](self.run / 'pins.json',pins)
        if not ready:self.original['pages'][0]['fragments'][1]['text']+='\n[Missing](absent.md)\n'
        self.seed_files();self.generated();failed=D['verify'](self.run)
        self.assertEqual(failed['status'],'READY' if ready else 'FAILED');self.assertEqual(failed['checks']['provenance']['status'],'PASS')
        self.assertEqual(failed['checks']['finalize']['status'],'PASS' if ready else 'FAIL');self.assertEqual(D['load'](self.run / 'state.json')['modelTokens'],8)
        if ready:self.assertEqual(D['promote'](self.run)['status'],'READY_PROMOTED')
        (self.run / '.runner.lock').touch()
        job=json.loads(json.dumps(self.job));job['budget']={'maxModelTokens':3,'maxRepairs':0,'wallSeconds':30}
        path=self.root / 'successor-job.json';D['save'](path,job)
        return path

    def prepare_owned_successor(self,job_path,child,mutate=None,previous=None,**options):
        def sources(job,inventory,directory):
            shutil.copytree(self.run / 'inputs/author-source',directory)
            if mutate:mutate(directory)
        with patch.dict(D['T'],{'collect':self.collect}),patch.dict(D['prepare'].__globals__,{'prepare_sources':sources}):
            return D['S']['prepare'](job_path,previous or self.run,D['SKILL'],child,D,**options)

    def test_ready_successor_requires_caller_revision_and_preserves_parent_ready_and_cost(self):
        job_path=self.successor_parent(ready=True)
        frozen=self.root / 'ready-freeze';shutil.copytree(self.run,frozen)
        before=D['tree'](frozen)
        with self.assertRaises(D['Stop']):self.prepare_owned_successor(job_path,self.root/'not-requested',previous=frozen)
        with (frozen / '.runner.lock').open('rb') as lock:
            D['fcntl'].flock(lock,D['fcntl'].LOCK_EX|D['fcntl'].LOCK_NB)
            with self.assertRaises(D['Stop']) as error:
                self.prepare_owned_successor(job_path,self.root/'live-ready',previous=frozen,revise_ready=True)
            self.assertEqual(error.exception.code,'SUCCESSOR_PREDECESSOR_BUSY')
        current=frozen/'ready/current.json';saved=current.read_bytes();changed=D['load'](current)
        changed['reportDigest']='changed';D['save'](current,changed)
        with self.assertRaises(D['Stop']) as error:
            self.prepare_owned_successor(job_path,self.root/'bad-ready',previous=frozen,revise_ready=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_READY_UNVERIFIABLE');current.write_bytes(saved)
        raw=frozen/'attempts/0/stdout.jsonl';saved=raw.read_bytes()
        raw.write_bytes(saved.replace(b'"input_tokens": 8',b'"input_tokens": 7'))
        with self.assertRaises(D['Stop']) as error:
            self.prepare_owned_successor(job_path,self.root/'bad-ready-history',previous=frozen,revise_ready=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_HISTORY_UNVERIFIABLE');raw.write_bytes(saved)
        child=self.root/'ready-successor'
        with patch.dict(D,{'author':lambda *a,**k:self.fail('prepare must not launch any author')}):
            result=self.prepare_owned_successor(job_path,child,previous=frozen,revise_ready=True)
        self.assertEqual(result['modelCalls'],0);self.assertEqual(result['inheritedModelTokens'],8)
        self.assertEqual(before,D['tree'](frozen))
        self.assertEqual(D['load'](frozen/'report.json')['status'],'READY')
        self.assertEqual(D['load'](child/'history/report.json')['status'],'READY')
        receipt=D['load'](child/'inputs/successor.json')
        self.assertEqual(receipt['callerDecision'],{'status':'REVISION_REQUESTED','reason':'INDEPENDENT_QUALITY_REJECTED','parentNativeStatus':'READY'})
        self.assertEqual(D['tree'](child/'history/ready'),D['tree'](frozen/'ready'))
        self.assertFalse((child/'ready').exists())
        self.assertEqual(D['load'](child/'state.json')['threadId'],D['load'](frozen/'state.json')['threadId'])
        self.assertNotEqual((child/'workspace/draft/original.json').stat().st_ino,(frozen/'workspace/draft/original.json').stat().st_ino)
        state=D['load'](child/'state.json');state['status']='AUTHOR_COMPLETED';D['save'](child/'state.json',state)
        self.assertEqual(D['verify'](child)['checks']['author']['code'],'AUTHOR_COMPLETION_REQUIRED')

    def test_successor_prepare_inherits_history_without_alias_or_model_and_requires_new_full_review(self):
        job_path=self.successor_parent();child=self.root / 'successor'
        (self.run / 'critic').mkdir();(self.run / 'critic/answers.txt').write_text('Must not enter successor workspace')
        before={name:D['sha']((self.run / name).read_bytes()) for name in ['state.json','pins.json','report.json']}
        parent_draft=D['tree'](self.run / 'workspace/draft')
        frozen=self.root / 'frozen-parent';shutil.copytree(self.run,frozen)
        with patch.dict(D,{'author':lambda *a,**k:self.fail('prepare must not invoke an author')}):
            prepared=self.prepare_owned_successor(job_path,child,previous=frozen)
        self.assertEqual(prepared['status'],'SUCCESSOR_PREPARED');self.assertEqual(prepared['modelCalls'],0)
        self.assertFalse((child / 'critic').exists());self.assertFalse((child / 'workspace/critic').exists())
        state=D['load'](child / 'state.json');self.assertEqual(state['modelTokens'],8);self.assertEqual(len(state['attempts']),1)
        self.assertEqual(state['threadId'],D['load'](self.run / 'state.json')['threadId'])
        self.assertEqual(parent_draft,D['tree'](child / 'workspace/draft'))
        self.assertNotEqual((child / 'workspace/draft/original.json').stat().st_ino,(self.run / 'workspace/draft/original.json').stat().st_ino)
        # Copying AUTHOR_COMPLETED cannot import qualification from the predecessor.
        state['status']='AUTHOR_COMPLETED';D['save'](child / 'state.json',state)
        checked=D['verify'](child);self.assertEqual(checked['checks']['author']['code'],'AUTHOR_COMPLETION_REQUIRED')
        state['status']='SUCCESSOR_PREPARED';D['save'](child / 'state.json',state)
        result=D['execute'](child)
        self.assertEqual(result['status'],'FAILED',result)  # Broken link remains; full final gate is mandatory.
        state=D['load'](child / 'state.json');self.assertEqual(state['modelTokens'],11)
        self.assertEqual(state['attempts'][1]['kind'],'successor-review')
        self.assertEqual(state['attempts'][1]['threadId'],state['attempts'][0]['threadId'])
        self.assertIn('resume',state['attempts'][1]['argv'])
        self.assertEqual(D['scoped_budget_tokens'](state,D['token_accounting'](state['attempts'])),3)
        self.assertEqual(result['checks']['author']['reportedModelTokens'],11)
        self.assertEqual(result['checks']['author']['inheritedModelTokens'],8)
        self.assertEqual(result['checks']['author']['continuationModelTokens'],3)
        prompt=json.loads((child / 'attempts/1/prompt.txt').read_text())
        self.assertEqual(prompt['request'],self.job['request']);self.assertIn('standalone full-docset review',prompt['instruction'])
        receipt=Path(prompt['historyReceipt'])
        if not receipt.is_absolute():receipt=child / 'workspace' / receipt
        self.assertTrue(receipt.is_file(), 'Receipt link must resolve from the author workspace')
        self.assertEqual(D['load'](receipt)['threadId'],state['threadId'])
        self.assertEqual(D['load'](child / 'history/report.json')['status'],'FAILED')
        self.assertEqual(parent_draft,D['tree'](self.run / 'workspace/draft'))
        for name,value in before.items():
            self.assertEqual(D['sha']((self.run / name).read_bytes()),value)
            self.assertEqual(D['sha']((frozen / name).read_bytes()),value)

    def test_successor_valid_same_context_review_can_reach_full_gate_without_hiding_inherited_cost(self):
        job_path=self.successor_parent();child=self.root / 'successor'
        self.prepare_owned_successor(job_path,child)
        self.original['pages'][0]['fragments'][1]['text']=self.original['pages'][0]['fragments'][1]['text'].replace('\n[Missing](absent.md)\n','')
        self.seed_files()
        result=D['execute'](child)
        self.assertEqual(result['status'],'READY_PROMOTED',result)
        report=D['load'](child / 'report.json');self.assertTrue(all(row['status']=='PASS' for row in report['checks'].values()))
        self.assertEqual(report['checks']['author']['reportedModelTokens'],11)
        self.assertEqual(report['checks']['author']['continuationModelTokens'],3)
        self.assertEqual(D['load'](self.run / 'report.json')['status'],'FAILED')

    def test_successor_rejects_changed_request_source_nonterminal_and_unverifiable_history(self):
        job_path=self.successor_parent()
        with (self.run / '.runner.lock').open('rb') as lock:
            D['fcntl'].flock(lock,D['fcntl'].LOCK_EX|D['fcntl'].LOCK_NB)
            with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root / 'busy-parent')
            self.assertEqual(error.exception.code,'SUCCESSOR_PREDECESSOR_BUSY')
        job=D['load'](job_path);job['request']='Different scope';D['save'](job_path,job)
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root / 'changed-request')
        self.assertEqual(error.exception.code,'SUCCESSOR_REQUEST_OR_SCOPE_CHANGED')
        job['request']=self.job['request'];D['save'](job_path,job)
        state=D['load'](self.run / 'state.json');saved=json.loads(json.dumps(state));state['status']='AUTHOR_RUNNING';D['save'](self.run / 'state.json',state)
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root / 'nonterminal')
        self.assertEqual(error.exception.code,'SUCCESSOR_TERMINAL_FAILED_REQUIRED');D['save'](self.run / 'state.json',saved)
        raw=self.run / 'attempts/0/stdout.jsonl';original=raw.read_bytes();raw.write_bytes(original.replace(b'"input_tokens": 8',b'"input_tokens": 7'))
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root / 'bad-accounting')
        self.assertEqual(error.exception.code,'SUCCESSOR_HISTORY_UNVERIFIABLE');raw.write_bytes(original)
        def changed(directory):
            path=directory / self.service / 'view.md';path.write_text(path.read_text()+'changed source')
        child=self.root / 'changed-source'
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,child,changed)
        self.assertEqual(error.exception.code,'SUCCESSOR_SOURCE_CHANGED')
        with self.assertRaises(D['Stop']) as error:D['execute'](child)
        self.assertEqual(error.exception.code,'SUCCESSOR_NOT_ADMITTED')
        self.assertFalse((child / 'attempts').exists())

    def test_successor_decreasing_cumulative_usage_is_preserved_and_rejected(self):
        job_path=self.successor_parent();child=self.root / 'successor';self.prepare_owned_successor(job_path,child)
        self.mode.write_text('successor-decreasing')
        with self.assertRaises(D['Stop']) as error:D['execute'](child)
        self.assertEqual(error.exception.code,'AUTHOR_USAGE_INVALID')
        state=D['load'](child / 'state.json')
        self.assertEqual(len(state['attempts']),2);self.assertEqual(state['attempts'][0]['usage']['input_tokens'],8)
        self.assertEqual(state['attempts'][1]['usage']['input_tokens'],5)
        self.assertEqual(state['tokenAccounting']['status'],'INVALID')
        self.assertFalse((child / 'ready').exists())

    def test_successor_rejects_thread_switch_and_preserves_the_failed_attempt_cost(self):
        job_path=self.successor_parent();child=self.root / 'successor';self.prepare_owned_successor(job_path,child)
        self.mode.write_text('successor-thread-switch')
        with self.assertRaises(D['Stop']) as error:D['execute'](child)
        self.assertEqual(error.exception.code,'AUTHOR_THREAD_CHANGED')
        state=D['load'](child / 'state.json')
        self.assertEqual(len(state['attempts']),2);self.assertEqual(state['modelTokens'],28)
        self.assertEqual(state['attempts'][0]['threadId'],'owned-test-session')
        self.assertEqual(state['attempts'][1]['threadId'],'different-test-session')
        self.assertFalse((child / 'ready').exists())

    def test_process_page_without_full_sequence_is_explicitly_rejected(self):
        request = self.root / 'sequences.json'
        D['save'](request, {'pages': {'process.md': '# Process\nOnly links.'}, 'requirements': [], 'accounts': [], 'processPages': ['process.md']})
        with self.assertRaises(D['Stop']) as error:
            D['command']([self.job['node'], str(ROOT / 'skills/codeclew/scripts/docs_run_sequences.mjs'), str(RENDERER), str(request)], self.root / 'sequence-check', 'check')
        self.assertEqual(error.exception.code, 'PROCESS_SEQUENCE_MISSING')
        self.assertEqual(error.exception.details['response']['details']['actual'], 0)

    def test_inventory_scaffold_renders_but_is_explicitly_unfinished(self):
        result = D['W']['check'](self.run, [], True, D)
        self.assertEqual(result['status'], 'SCAFFOLD_CHECKED', result)
        self.assertEqual(set(result['unfinishedPages']), {r['page'] for r in self.inventory['requirements']})
        self.assertFalse(result['ready']);self.assertFalse(result['sourceBindingsChecked'])
        self.assertTrue((Path(result['directory']) / 'html' / self.entry['page'].replace('.md','.html')).exists())

    def test_malformed_admitted_entry_kind_has_pre_author_field_diagnostic(self):
        for value in (None, '', 'http', {'kind':'HTTP_ENDPOINT'}):
            inventory = json.loads(json.dumps(self.inventory));inventory['requirements'][0]['entryKind'] = value
            with self.assertRaises(D['Stop']) as error:D['W']['build'](inventory, D)
            self.assertEqual(error.exception.code, 'INVALID_ENTRY_KIND')
            self.assertEqual(error.exception.details['field'], 'entryKind')
            self.assertEqual(error.exception.details['requirement'], self.entry['id'])

    def test_local_repair_keeps_other_pages_reads_and_last_valid_preview(self):
        self.local_reads();page = self.local_page()
        other = self.run / 'workspace/pages' / (self.service + '.md.json')
        other_bytes = other.read_bytes();reads = D['tree'](self.run / 'workspace/draft/reads')
        saved = self.save_local(page)
        self.assertEqual(saved['changed'], [self.entry['page']]);self.assertIn(self.service+'.md', saved['unchanged'])
        checked = self.local_check(page['page']);self.assertEqual(checked['status'], 'UNIT_CHECKED', checked)
        self.assertTrue(checked['sourceBindingsChecked']);self.assertFalse(checked['fullSiteReady'])
        pointer = (self.run / 'workspace/last-valid-preview.json').read_bytes()
        page['fragments'][1]['text'] += '\n[Missing](reservations.md#absent)\n'
        self.save_local(page);failed = self.local_check(page['page'])
        self.assertEqual(failed['status'], 'FAILED', failed)
        self.assertIn('reservations.md#absent',failed['pendingTargets'])
        self.assertTrue(any(x['page']==page['page'] and x['fragment']=='entry' for x in failed['locations']), failed)
        self.assertEqual((self.run / 'workspace/last-valid-preview.json').read_bytes(), pointer)
        page['fragments'][1]['text'] = page['fragments'][1]['text'].replace('\n[Missing](reservations.md#absent)\n','')
        self.save_local(page);self.assertEqual(self.local_check(page['page'])['status'],'UNIT_CHECKED')
        self.assertEqual(other.read_bytes(),other_bytes);self.assertEqual(D['tree'](self.run / 'workspace/draft/reads'),reads)

    def test_local_component_and_source_errors_never_become_ready(self):
        self.local_reads();page = self.local_page()
        page['fragments'][1]['text'] = page['fragments'][1]['text'].replace('"kind": "call"','"kind": "invented"')
        self.save_local(page);bad = self.local_check(page['page'])
        self.assertEqual(bad['code'],'INVALID_REQUIRED_SEQUENCE',bad)
        self.assertEqual(bad['locations'][0]['page'],page['page'])
        self.assertIn('/steps/0/kind',bad['locations'][0]['componentField'])
        page = self.local_page();page['fragments'][1]['evidence'] = ['invented-cite']
        self.save_local(page);bad = self.local_check(page['page'])
        self.assertEqual(bad['status'],'FAILED');self.assertFalse(bad['sourceBindingsChecked']);self.assertFalse(bad['ready'])

    def test_standalone_anchor_block_after_table_is_saved_safely(self):
        self.local_reads();page = self.local_page()
        page['fragments'] += [{'id':'table','text':'\n\n| Field | Meaning |\n| --- | --- |\n| value | Value |','evidence':[self.handle]},
                              {'text':'<a id="request-event"></a>'},
                              {'text':'\n[Input](#request-event)\n'}]
        self.save_local(page);result = self.local_check(page['page'])
        self.assertEqual(result['status'],'UNIT_CHECKED',result)
        original = D['load'](self.run / 'workspace/draft/original.json')
        content = next(p for p in original['pages'] if p['page']==page['page'])
        self.assertIn('\n\n<a id="request-event"></a>\n\n',[f['text'] for f in content['fragments']])

    def test_duplicate_anchor_is_located_on_the_affected_saved_page(self):
        self.local_reads();page = self.local_page()
        page['fragments'] += [{'text':'<a id="duplicate"></a>'},{'text':'<a id="duplicate"></a>'}]
        self.save_local(page);result = self.local_check(page['page'])
        self.assertEqual(result['code'],'AMBIGUOUS_EXPORT_ANCHOR',result)
        self.assertTrue(all(x['page']==page['page'] for x in result['locations']),result)

    def test_local_check_rejects_changed_frozen_input(self):
        raw = D['load'](self.run / 'workspace/inventory.json');raw['snapshot']='foreign'
        D['save'](self.run / 'workspace/inventory.json',raw)
        with self.assertRaises(D['Stop']) as error:D['W']['check'](self.run,[],True,D)
        self.assertEqual(error.exception.code,'FROZEN_INPUTS_CHANGED')

    def readmit_budget(self, maximum, repairs):
        job=D['load'](self.run/'inputs/job.json');job['budget'].update(maxModelTokens=maximum,maxRepairs=repairs)
        D['save'](self.run/'inputs/job.json',job)
        pins=D['load'](self.run/'pins.json');pins['jobDigest']=D['digest'](job);pins['inputFiles']=D['tree'](self.run/'inputs')
        D['save'](self.run/'pins.json',pins)

    def test_service_unit_gate_includes_new_owned_pages_before_global_stage(self):
        self.original['pages'].append({'page':'data.md','fragments':[{'text':'# Data\n\n' + D['W']['MARKER'] + '\n'}]})
        self.navigation['services'][0]['pages'].append('data.md')
        self.seed_files()
        result = D['execute'](self.run)
        self.assertEqual(result['status'],'FAILED',result)
        self.assertEqual(result['unit'],self.service)
        self.assertEqual(result['checks']['local']['code'],'UNFINISHED_SCAFFOLD')
        self.assertIn('data.md',result['checks']['local']['selectedPages'])
        self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']),1)

    def test_related_units_share_one_author_context_and_aggregate_budget(self):
        result=D['execute'](self.run)
        self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json')
        self.assertEqual([a['kind'] for a in state['attempts']],['unit','unit'])
        self.assertEqual(state['modelTokens'],40)
        self.assertTrue('resume' in state['attempts'][1]['argv'])
        self.assertEqual(state['threadId'],'owned-test-session')

    def test_early_failure_repairs_same_unit_before_advancing(self):
        self.readmit_budget(1000,1);self.mode.write_text('repair-on-resume')
        self.original['pages'][0]['fragments'][1]['text']+='\n[Missing](absent.html)\n';self.seed_files()
        result=D['execute'](self.run);self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json')
        self.assertEqual([a['kind'] for a in state['attempts']],['unit','repair','unit'])
        self.assertEqual(state['attempts'][0]['unit'],state['attempts'][1]['unit'])
        self.assertEqual(state['modelTokens'],60)
        self.assertTrue((self.run/'attempts/0/draft/original.json').exists())

    def test_advisory_reserve_does_not_replace_the_actual_aggregate_cap(self):
        self.readmit_budget(35,0)
        with self.assertRaises(D['Stop']) as error:D['execute'](self.run)
        self.assertEqual(error.exception.code,'AUTHOR_TOKEN_BUDGET_EXCEEDED')
        state=D['load'](self.run/'state.json');self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['budgetPlan']['repairReserveEstimate'],20)
        self.assertEqual(state['modelTokens'],40)
        self.assertFalse((self.run/'ready').exists())

    def test_uneven_units_are_not_stopped_by_unproven_repair_proxy(self):
        self.readmit_budget(250000,0);self.mode.write_text('uneven-units')
        result=D['execute'](self.run);self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json')
        self.assertEqual(state['modelTokens'],132000)
        self.assertEqual(state['budgetPlan']['remainingTokens'],120000)
        self.assertEqual(state['budgetPlan']['repairReserveEstimate'],130000)
        self.assertTrue(state['budgetPlan']['advisoryOnly'])

    def test_no_token_cutoff_still_requires_measured_usage_and_full_export(self):
        self.readmit_budget(None,0);self.mode.write_text('budget')
        result=D['execute'](self.run);self.assertEqual(result['status'],'READY_PROMOTED',result)
        state=D['load'](self.run/'state.json')
        self.assertGreater(state['modelTokens'],250000)
        self.assertIsNone(state['budgetPlan']['remainingTokens'])
        state['attempts'][-1]['usage'].pop('cached_input_tokens');D['save'](self.run/'state.json',state)
        failed=self.check();self.assertEqual(failed['checks']['author']['code'],'AUTHOR_USAGE_UNAVAILABLE')
        with self.assertRaises(D['Stop']) as error:D['author'](self.run)
        self.assertEqual(error.exception.code,'AUTHOR_USAGE_UNAVAILABLE')

    def test_short_entry_compact_success_full_receipts_and_actionable_errors(self):
        page=self.local_page();self.save_local(page);self.local_reads()
        wrapper=self.run/'workspace/docs'
        def call(*args):
            return subprocess.run([str(wrapper),*args],cwd=self.run/'workspace',capture_output=True,text=True)
        saved=call('save-pages','--input','pages/'+page['page']+'.json')
        self.assertEqual(saved.returncode,0,saved.stdout+saved.stderr)
        compact=json.loads(saved.stdout);full=D['load'](self.run/'workspace/last-save-pages.json')
        self.assertEqual(compact['changed'],full['changed'])
        self.assertLess(len(saved.stdout),len(D['encode'](full)))
        checked=call('check','--page',page['page']);self.assertEqual(checked.returncode,0,checked.stdout+checked.stderr)
        compact=json.loads(checked.stdout);full=D['load'](self.run/'workspace/last-local-check.json')
        self.assertTrue(compact['sourceBindingsChecked']);self.assertFalse(compact['ready'])
        self.assertLess(len(checked.stdout),len(D['encode'](full)))
        complete=call('check','--page',page['page'],'--full')
        self.assertEqual(complete.returncode,0,complete.stdout+complete.stderr)
        self.assertEqual(json.loads(complete.stdout),D['load'](self.run/'workspace/last-local-check.json'))
        page['fragments'][1]['text']+='\n[Wrong](missing.md#anchor)\n';self.save_local(page)
        failed=call('check','--page',page['page']);self.assertEqual(failed.returncode,2)
        diagnostic=json.loads(failed.stdout);self.assertTrue(diagnostic['code'])
        self.assertEqual(diagnostic['locations'][0]['page'],page['page'])
        self.assertEqual(diagnostic['locations'][0]['fragment'],'entry')
        self.assertTrue(diagnostic['locations'][0]['field'])

    def test_managed_author_entry_and_local_skill_are_pinned(self):
        for name in ['AGENTS.md','.agents/skills/codeclew/SKILL.md','docs']:
            path=self.run/'workspace'/name;raw=path.read_bytes();path.write_bytes(raw+b'\nchanged')
            with self.assertRaises(D['Stop']) as error:D['frozen_inputs'](self.run)
            self.assertEqual(error.exception.code,'PREPARED_AUTHOR_ENTRY_CHANGED')
            path.write_bytes(raw)
        pins=D['load'](self.run/'pins.json');original=pins['authorSkillsConfig'];pins['authorSkillsConfig']='skills.config=[]'
        D['save'](self.run/'pins.json',pins)
        with self.assertRaises(D['Stop']) as error:D['frozen_inputs'](self.run)
        self.assertEqual(error.exception.code,'PREPARED_AUTHOR_ENTRY_CHANGED')
        pins['authorSkillsConfig']=original;D['save'](self.run/'pins.json',pins)
        wrapper=self.run/'workspace/docs'
        result=subprocess.run([str(wrapper),'run'],cwd=self.run/'workspace',capture_output=True,text=True)
        self.assertNotEqual(result.returncode,0)
        self.assertEqual(D['load'](self.run/'state.json')['attempts'],[])

    def test_short_entry_executes_pinned_python_isolated_without_site_and_preserves_arguments(self):
        run=self.root/"isolated wrapper's run";(run/'workspace').mkdir(parents=True)
        skill=self.root/'skill with spaces';(skill/'scripts').mkdir(parents=True)
        (skill/'scripts/docs_run.py').write_text('import json,sys\nprint(json.dumps({"isolated":sys.flags.isolated,"noSite":sys.flags.no_site,"argv":sys.argv[1:]}))\n')
        D['W']['install_author_entry'](run,skill,sys.executable)
        result=subprocess.run([str(run/'workspace/docs'),'save-pages','--input','pages/a file.json'],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        observed=json.loads(result.stdout)
        self.assertEqual(observed['isolated'],1);self.assertEqual(observed['noSite'],1)
        self.assertEqual(observed['argv'],['save-pages','--input','pages/a file.json','--run-dir',str(run)])

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

    def test_source_preparation_keeps_discovery_proof_separate_from_readable_views(self):
        inventory = json.loads(json.dumps(self.inventory))
        inventory['services'][self.service].update(work='work-a', sourceInventory=[{'kind':'THREAD_SCOPE'}])
        destination = self.root / 'prepared' / 'author-source'; destination.parent.mkdir()
        calls = []
        def command(argv, directory, label, **kwargs):
            calls.append(label)
            out = Path(argv[argv.index('--output-dir') + 1])
            if label == 'discover':
                out.mkdir()
                D['save'](out / 'packet.json', {'queryComplete': True, 'choices': [{'file': self.source['file']}]})
            else:
                shutil.copytree(self.manifest.parent, out)
        with patch.dict(D['_prepare_sources'].__globals__, {'command':command}):
            result = D['prepare_sources'](self.job, inventory, destination)
        self.assertEqual(calls, ['discover', 'read'])
        self.assertEqual(result['sourceBindings'], 1)
        scope = result['services'][self.service]
        discovery = next(r for r in inventory['requirements'] if r['kind'] == 'DISCOVERY')
        self.assertEqual(scope['discoveryProofs'][discovery['id']]['status'], 'COMPLETE_RETAINED_GIT_BLOB')
        self.assertTrue(Path(scope['citationIndex']).is_file())
        self.assertIn('/source-proof/', scope['citationIndex'])
        self.assertFalse(list(destination.rglob('citations.json')))
        self.assertFalse(list(destination.rglob('receipts.json')))
        self.assertEqual(D['load'](destination / self.service / 'requirements.json')[discovery['id']]['evidenceCandidates'], [self.handle])
        self.assertEqual(len(list(destination.rglob('views/*.md'))), 1)

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
            for field in ('topics','sourceNotes'):
                if field in account:row[field]=json.loads(json.dumps(account[field]))
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
            {'threadId':'first', 'usage':{'input_tokens':5000,'cached_input_tokens':4990,'output_tokens':10,'reasoning_output_tokens':4}},
            {'threadId':'second', 'usage':{'input_tokens':30,'cached_input_tokens':5,'output_tokens':7,'reasoning_output_tokens':2}}])
        self.assertEqual(result['budgetTokens'], 52)
        self.assertEqual(result['rawTotals'], {'input_tokens':5030,'cached_input_tokens':4995,
                                             'output_tokens':17,'reasoning_output_tokens':6})
        self.assertEqual(result['totalInputOutputFootprint'], 5047)

    def test_missing_cache_counter_stays_explicitly_unavailable_across_repairs(self):
        result = D['token_accounting']([
            {'threadId':'first', 'usage':{'input_tokens':100,'output_tokens':20}},
            {'threadId':'second', 'usage':{'input_tokens':50,'cached_input_tokens':45,'output_tokens':10}}])
        self.assertEqual(result['status'], 'UNAVAILABLE')
        self.assertIsNone(result['budgetTokens'])
        self.assertIsNone(result['rawTotals']['cached_input_tokens'])
        self.assertIsNone(result['rawTotals']['reasoning_output_tokens'])
        self.assertEqual(result['missing'], [{'attempt':0,'fields':['cached_input_tokens']}])
        self.assertEqual(D['budget_tokens'](result), 35)

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

    def recorded_false_stop(self):
        self.mode.write_text('recorded-weather');self.readmit_budget(250000,2)
        scaffold = D['load'](self.run / 'inputs/scaffold.json')
        pages = list(scaffold['pages'])
        scaffold['units'] = [{'id':name,'pages':pages} for name in ('dashboard','places','forecasts','complete-docset')]
        D['save'](self.run / 'inputs/scaffold.json', scaffold)
        pins = D['load'](self.run / 'pins.json');pins['inputFiles'] = D['tree'](self.run / 'inputs');D['save'](self.run / 'pins.json',pins)
        D['author'](self.run,unit=scaffold['units'][0]);state=D['author'](self.run,unit=scaffold['units'][1])
        self.assertEqual(state['modelTokens'],184863)
        for attempt in state['attempts']:attempt.pop('threadId')
        state.update(status='AUTHOR_BUDGET_EXCEEDED',modelTokens=305173,unitCursor=1)
        D['save'](self.run / 'state.json',state)
        old = self.root / 'old-skill'
        shutil.copytree(ROOT / 'skills/codeclew',old,ignore=shutil.ignore_patterns('node_modules','__pycache__','.git'))
        with (old / 'scripts/docs_run.py').open('a') as stream:stream.write('\n# Owned previous-admission fixture.\n')
        pins['skillFiles'] = D['tree'](old,skill=True);D['save'](self.run / 'pins.json',pins)
        return old

    def test_recorded_false_stop_recovers_same_run_without_duplicate_places_author(self):
        old = self.recorded_false_stop()
        state_before = (self.run/'state.json').read_bytes();pins_before=(self.run/'pins.json').read_bytes()
        draft_before=D['tree'](self.run/'workspace/draft');input_before=D['tree'](self.run/'inputs');prompt_before=(self.run/'prompt.txt').read_bytes()
        result = subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
            'recover-accounting','--run-dir',str(self.run),'--previous-skill',str(old)],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stdout+result.stderr)
        receipt=json.loads(result.stdout);self.assertEqual(receipt['status'],'ACCOUNTING_RECOVERED')
        self.assertEqual(receipt['correctedUsage']['budgetTokens'],184863)
        self.assertEqual(receipt['oldUsage']['budgetTokens'],305173)
        folder=Path(receipt['receipt']).parent
        self.assertEqual((folder/'original-state.json').read_bytes(),state_before)
        self.assertEqual((folder/'original-pins.json').read_bytes(),pins_before)
        state=D['load'](self.run/'state.json');original=json.loads(state_before)
        self.assertEqual(state['authorStartedAt'],original['authorStartedAt'])
        self.assertEqual(state['unitCursor'],1);self.assertEqual(state['completedUnit'],'places')
        self.assertEqual([a['usage'] for a in state['attempts']],[a['usage'] for a in original['attempts']])
        self.assertEqual(len(state['attempts']),2)
        self.assertEqual(D['tree'](self.run/'workspace/draft'),draft_before)
        self.assertEqual(D['tree'](self.run/'inputs'),input_before)
        self.assertEqual((self.run/'prompt.txt').read_bytes(),prompt_before)
        final=D['execute'](self.run);self.assertEqual(final['status'],'READY_PROMOTED',final)
        resumed=D['load'](self.run/'state.json')
        self.assertEqual([a['unit'] for a in resumed['attempts']],['dashboard','places','forecasts','complete-docset'])
        self.assertEqual(resumed['modelTokens'],184903)
        self.assertEqual(resumed['authorStartedAt'],original['authorStartedAt'])
        continuation=(self.run/'attempts/2/prompt.txt').read_text()
        self.assertIn(str(ROOT/'skills/codeclew/scripts/docs_run.py'),continuation)
        self.assertIn('original time/token/repair caps are unchanged',continuation)

    def test_recovery_refuses_actual_over_cap_and_expired_original_wall_budget(self):
        old=self.recorded_false_stop();state=D['load'](self.run/'state.json');state['authorStartedAt']-=31
        D['save'](self.run/'state.json',state)
        with self.assertRaises(D['Stop']) as error:D['recover_accounting'](self.run,old)
        self.assertEqual(error.exception.code,'AUTHOR_WALL_BUDGET_EXHAUSTED')
        state['authorStartedAt']+=31;D['save'](self.run/'state.json',state)
        job=D['load'](self.run/'inputs/job.json');job['budget']['maxModelTokens']=180000
        D['save'](self.run/'inputs/job.json',job)
        pins=D['load'](self.run/'pins.json');pins.update(jobDigest=D['digest'](job),inputFiles=D['tree'](self.run/'inputs'))
        D['save'](self.run/'pins.json',pins)
        before=(self.run/'state.json').read_bytes()
        with self.assertRaises(D['Stop']) as error:D['recover_accounting'](self.run,old)
        self.assertEqual(error.exception.code,'FALSE_BUDGET_STOP_NOT_PROVEN')
        self.assertEqual((self.run/'state.json').read_bytes(),before)
        self.assertFalse((self.run/'accounting-recoveries').exists())

    def test_recovery_refuses_changed_sources_raw_counters_and_thread_mismatch(self):
        old=self.recorded_false_stop()
        targets=[self.run/'state.json',self.run/'attempts/1/stdout.jsonl',self.run/'inputs/inventory.json']
        originals=[p.read_bytes() for p in targets]
        badstate=json.loads(originals[0]);badstate['attempts'][1]['usage']['input_tokens']+=1
        badthread=originals[1].replace(b'owned-test-session',b'foreign-session')
        variants=[(targets[0],D['encode'](badstate),'AUTHOR_HISTORY_MISMATCH'),
                  (targets[1],badthread,'AUTHOR_HISTORY_MISMATCH'),
                  (targets[2],originals[2]+b'\n','FROZEN_INPUTS_CHANGED')]
        for path,bad,code in variants:
            path.write_bytes(bad);before=(self.run/'state.json').read_bytes()
            with self.assertRaises(D['Stop']) as error:D['recover_accounting'](self.run,old)
            self.assertEqual(error.exception.code,code)
            self.assertEqual((self.run/'state.json').read_bytes(),before)
            for target,raw in zip(targets,originals):target.write_bytes(raw)
        self.assertFalse((self.run/'accounting-recoveries').exists())

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
        self.assertEqual(result['status'], 'FAILED'); self.assertEqual(result['checks']['output']['code'], 'UNFINISHED_SCAFFOLD')
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
