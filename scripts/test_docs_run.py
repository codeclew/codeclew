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
import json,shutil,sys,time,subprocess
from pathlib import Path
seed=Path({str(self.seed)!r});mode=Path({str(self.mode)!r}).read_text()
raw=sys.stdin.read()
try:packet=json.loads(raw)
except ValueError:packet={{}}
fresh=packet.get('schema')=='codeclew-author-handoff/1'
number=packet.get('accounting',{{}}).get('historicalAttempts',0)
thread='owned-test-session' if mode=='context-reused' or not fresh else 'owned-fresh-'+str(number)
print(json.dumps({{'type':'thread.started','thread_id':thread}}),flush=True)
if mode=='interrupt':time.sleep(10)
draft=Path.cwd()/'draft'
if mode!='empty':
 for source in seed.iterdir():
  if source.name in ('review-current','finish-current'):continue
  target=draft/source.name
  if target.is_dir():shutil.rmtree(target)
  elif target.exists():target.unlink()
  if source.is_dir():shutil.copytree(source,target)
  else:shutil.copyfile(source,target)
if mode=='repair-on-handoff' and fresh and number:
 path=draft/'original.json'
 path.write_text(path.read_text().replace('[Missing](absent.html)','Repaired local link.'))
if fresh and (seed/'review-current').exists():
 page=packet['currentPage']['page']
 shell=json.loads(subprocess.check_output(['./docs','review-page','--page',page],text=True))
 for row in shell['requirements'].values():
  row['disposition']='EXPLAINED'
  row['section']='handler' if 'diagram' in row else Path(page).stem
  if 'topics' in row:
   row['sourceNotes']={{'references':[{{'page':page,'claim':'overview'}}]}}
   row['topics']={{k:{{'disposition':'EXPLAINED','page':page,'claim':'overview'}} for k in row['topics']}}
  if 'discoveryDisposition' in row:row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
 shell['claims']={{k:'SUPPORTED' for k in shell['claims']}}
 assessment=Path.cwd()/'fixture-review.json';assessment.write_text(json.dumps(shell))
 subprocess.run(['./docs','review-page','--page',page,'--input',str(assessment)],check=True,stdout=subprocess.DEVNULL)
 if (seed/'finish-current').exists():
  assessment.write_text(json.dumps({{'unresolved':[],'visual':{{'status':'UNAVAILABLE','reason':'Owned deterministic backend'}}}}))
  subprocess.run(['./docs','finish-review','--input',str(assessment)],check=True,stdout=subprocess.DEVNULL)
 for name in ('self-review.json','reviewed-pages.json'):shutil.copyfile(draft/name,seed/name)
(draft/'report.json').write_text(json.dumps({{'status':'PASS','semanticTruthAssessed':True}}))
usage={{'input_tokens':5000 if mode=='high-cache' else 20,'cached_input_tokens':4990 if mode=='high-cache' else 10,'output_tokens':2000000 if mode=='budget' else 10}}
if mode=='uneven-units':usage={{'input_tokens':2000 if number else 130000,'cached_input_tokens':0,'output_tokens':0}}
counter=Path.cwd()/'backend-usage.json'
if mode in ('successor-accounting','successor-decreasing'):
 counter=seed.parent/'session-usage.json'
 usage={{'input_tokens':5 if mode=='successor-decreasing' else 3 if fresh else 8,'cached_input_tokens':0,'output_tokens':0}}
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
                          {'text': '\n\n<a id="clew-fragment-overview"></a>\n\n'},
                          {'id': 'overview', 'text': 'Retained reservation data and effects.\n\n| Concept | Meaning |\n| --- | --- |\n| Retained declaration | Source-defined domain item |\n', 'evidence': [self.handle]}]}]}
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
        overview = next(f for f in self.original['pages'][1]['fragments'] if f.get('id') == 'overview')
        for row in self.review['requirements']:
            requirement = next(r for r in self.inventory['requirements'] if r['id'] == row['requirement'])
            if requirement['kind'] == 'SERVICE':
                row['topics'] = {topic: {'disposition':'EXPLAINED','page':requirement['page'],
                    'claim':'overview','evidence':[self.handle],'fragmentDigest':D['digest'](overview)}
                    for topic in requirement['topics']}
                row['sourceNotes'] = {'id':'source-notes','text':'Source block: ' + self.source['text'],'evidence':[self.handle]}
        self.navigation['services'][0]['data'] = [{'title':'Reservation data','target':self.service + '.md#clew-fragment-overview'}]
        self.seed_files()
        # These controls start from authored labels; scaffold rejection has its
        # own positive/negative controls below and is never bypassed in product.
        D['W']['page_info'](self.run,self.entry['page'],self.service,'Reserve an item',False,D,summary='Accept a reservation')
        self.save_local(self.original['pages'][1])
        navigation = D['load'](self.run / 'workspace/draft/navigation.json')
        navigation['services'][0]['data'] = self.navigation['services'][0]['data']
        D['save'](self.run / 'workspace/draft/navigation.json', navigation)

    def seed_files(self):
        for name, value in [('original.json', self.original), ('navigation.json', self.navigation), ('self-review.json', self.review)]:
            D['save'](self.seed / name, value)

    def prepare_review_context(self):
        proof=self.run/'inputs/source-proof'/self.service
        shutil.copytree(self.manifest.parent,proof/'read')
        _,packet,_,entries=D['M']['C']['load'](proof/'read/citations.json')
        regions,bindings,_=D['source_projection'](packet,entries)
        home=self.run/'inputs/author-source'/self.service;home.mkdir(parents=True,exist_ok=True)
        views={}
        for key,region in regions.items():
            path='views/'+key+'.md';(home/path).parent.mkdir(exist_ok=True)
            (home/path).write_text(region['text'])
            views[key]={'file':region['file'],'view':path,'codeBytes':len(region['text'].encode()),'coveringCitations':region['citations']}
        D['save'](home/'views.json',views)
        D['save'](home/'requirements.json',{self.entry['id']:{'kind':'ENTRY','page':self.entry['page'],'views':[r['view'] for r in views.values()],'evidenceCandidates':list(entries)}})
        D['save'](proof/'citation-index.json',bindings)
        pins=D['load'](self.run/'pins.json');pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        return self.service+'/'+next(iter(views))

    def fixture_overview(self, text, *, unavailable=False):
        fragment=next(f for f in self.original['pages'][1]['fragments'] if f.get('id')=='overview')
        fragment['text']=text
        for row in self.review['requirements']:
            for topic,value in row.get('topics',{}).items():
                if value.get('claim')=='overview':value['fragmentDigest']=D['digest'](fragment)
                if topic=='data fields and domain distinctions' and unavailable:
                    value.update(disposition='EXPLICIT_UNKNOWN',reason='The retained empty declaration contains no domain fields; runtime-only state is outside this source.')
        self.seed_files();self.generated()

    def test_raw_literal_loss_has_exact_local_final_diagnostic_while_code_and_intentional_html_pass(self):
        self.readmit_budget(1000,1);name=self.service+'.md'
        base=next(f['text'] for f in self.original['pages'][1]['fragments'] if f.get('id')=='overview')
        self.fixture_overview(base+'\nHandle <record-key>:<stamp>; outcome OUTCOME_<category>.\n')
        local=self.local_check(name);final=D['verify'](self.run)
        self.assertEqual(local['code'],'READER_PRESENTATION_REQUIRED',local)
        self.assertEqual(final['checks']['selfReview']['code'],local['code'])
        self.assertEqual(local['details']['issues'],final['checks']['selfReview']['details']['issues'])
        issues=local['details']['issues'];self.assertEqual({i['actual'] for i in issues},{'<record-key>','<stamp>','<category>'})
        self.assertTrue(all(i['page']==name and i['fragment']=='overview' and './docs put' in i['recovery'] for i in issues))
        fixed=base+'\nHandle `<record-key>:<stamp>`; outcome `OUTCOME_<category>`.\n\n'
        fixed+='```text\n<literal-code>\n```\n\n<a id="intentional"></a>\n\n<details><summary>Details</summary>\n<p>Intentional disclosure.</p>\n</details>\n\n'
        fixed+='<domain-marker>Intentional paired HTML</domain-marker>\n\n<span data-token="<attribute-value>">Visible</span>\n\n&lt;escaped-literal&gt;\n'
        self.fixture_overview(fixed)
        local=self.local_check(name);self.assertEqual(local['status'],'UNIT_CHECKED',local)
        final=D['verify'](self.run);self.assertEqual(final['status'],'READY',final);self.assertFalse(final['semanticTruthAssessed'])
        from html.parser import HTMLParser
        class Visible(HTMLParser):
            def __init__(self):super().__init__();self.text=[]
            def handle_data(self,text):self.text.append(text)
        parser=Visible();parser.feed((Path(local['directory'])/'html'/name.replace('.md','.html')).read_text())
        visible=''.join(parser.text)
        for literal in ('<record-key>:<stamp>','OUTCOME_<category>','<literal-code>','<escaped-literal>'):self.assertIn(literal,visible)

    def test_data_topic_requires_rendered_table_but_bounded_absence_and_prose_topics_remain_valid(self):
        self.readmit_budget(1000,2);name=self.service+'.md'
        self.fixture_overview('## Retained domain\n\nA prose-only account of retained fields.\n')
        local=self.local_check(name);final=D['verify'](self.run)
        self.assertEqual(local['code'],'READER_PRESENTATION_REQUIRED',local)
        self.assertEqual(final['checks']['selfReview']['code'],local['code'])
        self.assertEqual(local['details']['issues'],final['checks']['selfReview']['details']['issues'])
        self.assertEqual(local['details']['issues'][0]['code'],'DATA_FIELDS_TABLE_REQUIRED')
        self.assertEqual(local['details']['issues'][0]['fragment'],'overview')
        table='## Retained domain\n\n| Concept | Meaning |\n| --- | --- |\n| Retained declaration | Source-defined domain item |\n'
        self.fixture_overview(table);self.assertEqual(self.local_check(name)['status'],'UNIT_CHECKED')
        final=D['verify'](self.run);self.assertEqual(final['status'],'READY',final);self.assertFalse(final['semanticTruthAssessed'])
        self.fixture_overview('## Retained boundary\n\nNo domain fields are declared by this retained empty class; runtime-only state is outside this source.\n',unavailable=True)
        local=self.local_check(name);self.assertEqual(local['status'],'UNIT_CHECKED',local)
        final=D['verify'](self.run);self.assertEqual(final['status'],'READY',final)
        # Intentional HTML tables are rendered tables too. Fenced examples and
        # comments containing table markup are not actual reader field tables.
        accounts=json.loads(json.dumps(self.review['requirements']))
        for row in accounts:
            if 'topics' in row:row['topics']['data fields and domain distinctions']['disposition']='EXPLAINED'
        for text,valid in (('<table><tr><th>Concept</th><th>Meaning</th></tr><tr><td>Declaration</td><td>Retained item</td></tr></table>',True),
                           ('```html\n<table><tr><td>Example</td></tr></table>\n```',False),
                           ('<!-- <table><tr><td>Comment</td></tr></table> -->',False),
                           ('<template><table><tr><td>Template</td></tr></table></template>',False)):
            original=json.loads(json.dumps(self.original));next(f for f in original['pages'][1]['fragments'] if f.get('id')=='overview')['text']=text
            if valid:D['W']['validate_reader_presentation'](original,accounts,self.job,D)
            else:
                with self.assertRaises(D['Stop']) as error:D['W']['validate_reader_presentation'](original,accounts,self.job,D)
                self.assertIn('DATA_FIELDS_TABLE_REQUIRED',{i['code'] for i in error.exception.details['issues']})

    def test_empty_and_explicitly_hidden_field_tables_fail_the_same_local_final_predicate(self):
        self.readmit_budget(1000,1);name=self.service+'.md'
        for text in ('<table></table>',
                     '<table hidden><tr><td>Identifier</td><td>Retained identifier</td></tr></table>'):
            self.fixture_overview(text)
            local=self.local_check(name);final=D['verify'](self.run)
            self.assertEqual(local.get('code'),'READER_PRESENTATION_REQUIRED',local)
            self.assertEqual(final['checks']['selfReview']['code'],local['code'])
            self.assertEqual(local['details']['issues'],final['checks']['selfReview']['details']['issues'])
            issue=local['details']['issues'][0]
            self.assertEqual(issue['code'],'DATA_FIELDS_TABLE_REQUIRED');self.assertEqual(issue['fragment'],'overview')
            self.assertIn('./docs put',issue['recovery']);self.assertFalse(final['semanticTruthAssessed'])

    def test_field_tables_need_nonempty_data_cells_outside_explicit_hidden_markup(self):
        accounts=self.review['requirements']
        def check(text, valid):
            original=json.loads(json.dumps(self.original))
            next(f for f in original['pages'][1]['fragments'] if f.get('id')=='overview')['text']=text
            if valid:D['W']['validate_reader_presentation'](original,accounts,self.job,D)
            else:
                with self.assertRaises(D['Stop']) as error:D['W']['validate_reader_presentation'](original,accounts,self.job,D)
                self.assertIn('DATA_FIELDS_TABLE_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        table='<table><tr><th>Field</th><th>Meaning</th></tr><tr><td>Key</td><td>Retained identifier</td></tr></table>'
        for text in (table,'<details><summary>Fields</summary>'+table+'</details>',
                     '| Field | Meaning |\n| --- | --- |\n| Key | Retained identifier |',
                     '<table><tr><th scope="row">Key</th><td>Retained identifier</td></tr></table>'):
            check(text,True)
        for text in ('<table></table>','<table><tr><td> </td></tr></table>',
                     '<table><tr><th>Field</th><th>Meaning</th></tr></table>',
                     table.replace('<table>','<table hidden>'),'<div hidden>'+table+'</div>',
                     table.replace('<tr><td>','<tr hidden><td>'),
                     table.replace('Retained identifier','<span hidden>Retained identifier</span>').replace('>Key<','><span hidden>Key</span><'),
                     '<div style="display: none">'+table+'</div>',
                     '| Field | Meaning |\n| --- | --- |',
                     '| Field | Meaning |\n| --- | --- |\n| <span hidden>Key</span> | <span hidden>Value</span> |'):
            check(text,False)
        original=json.loads(json.dumps(self.original));page=original['pages'][1]
        page['fragments'].insert(0,{'text':'<div hidden>\n\n'})
        page['fragments'].append({'text':'\n\n</div>\n'})
        with self.assertRaises(D['Stop']) as error:D['W']['validate_reader_presentation'](original,accounts,self.job,D)
        self.assertIn('DATA_FIELDS_TABLE_REQUIRED',{i['code'] for i in error.exception.details['issues']})

    def test_review_context_pairs_one_genuine_source_with_current_reader_and_own_notes_without_writes(self):
        view=self.prepare_review_context();self.save_local(self.local_page())
        D['P']['save_notes'](self.run,self.service+'.md','Previous own reading: inspect the retained declaration.',[self.handle],D)
        before=D['tree'](self.run)
        inventory=self.show_cli('--review-context','--page',self.entry['page'],'--limit','0')
        self.assertIn(view,inventory);self.assertNotIn(self.source['text'],inventory)
        context=self.show_cli('--review-context','--page',self.entry['page'],'--fragment','entry','--view',view,'--limit','0')
        self.assertEqual(context.count(self.source['text']),1)
        self.assertIn(self.local_page()['fragments'][1]['text'],context)
        self.assertNotIn('Previous own reading',context);self.assertIn(self.handle,context)
        with_notes=self.show_cli('--review-context','--page',self.entry['page'],'--fragment','entry','--view',view,'--include-notes','--limit','0')
        self.assertIn('Previous own reading',with_notes)
        hint=next(line for line in context.splitlines() if line.startswith('Own notes available: '))
        note_path=hint.removeprefix('Own notes available: ').split(' (',1)[0]
        self.assertEqual((self.run/'workspace'/note_path).read_bytes(),
                         (self.run/'workspace/notes'/(self.service+'.md.json')).read_bytes())
        self.assertIn('source to reader for omissions',context);self.assertIn('reader to source for unsupported',context)
        self.assertIn('not complete dependencies',context);self.assertIn(self.source['revision'],context)
        self.assertEqual(before,D['tree'](self.run))
        oversized=json.loads(self.show_cli('--review-context','--page',self.entry['page'],'--view',view,'--limit','20',success=False))
        self.assertEqual(oversized['code'],'REVIEW_CONTEXT_TOO_LARGE')
        self.assertEqual(oversized['details']['sourceBytes'],len(self.source['text'].encode()))
        self.assertNotIn(self.source['text'],json.dumps(oversized))
        self.assertIn('--limit',oversized['details']['recovery']);self.assertEqual(before,D['tree'](self.run))
        cropped=json.loads(self.show_cli('--review-context','--page',self.entry['page'],'--view',view,'--offset','1',success=False))
        self.assertEqual(cropped['code'],'REVIEW_CONTEXT_REQUIRES_WHOLE_VIEW')

    def test_review_context_inventory_includes_uncited_views_and_missing_foreign_or_stale_cannot_pair(self):
        view=self.prepare_review_context();self.save_local(self.local_page())
        home=self.run/'inputs/author-source'/self.service
        rows=D['load'](home/'views.json');rows['uncited']={'file':'uncited.fixture','view':'views/uncited.md','codeBytes':42,'coveringCitations':[]}
        D['save'](home/'views.json',rows)
        pins=D['load'](self.run/'pins.json');pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        text=self.show_cli('--review-context','--page',self.entry['page'],'--limit','0')
        self.assertIn(self.service+'/uncited',text)
        for flags,code in ((['--view',self.service+'/uncited'],'REVIEW_SOURCE_VIEW_UNVERIFIED'),
                           (['--view','foreign/'+view.split('/')[1]],'REVIEW_SOURCE_VIEW_NOT_FOUND'),
                           (['--view',self.service+'/missing'],'REVIEW_SOURCE_VIEW_NOT_FOUND'),
                           (['--service','foreign'],'REVIEW_CONTEXT_SERVICE_REQUIRED')):
            failed=json.loads(self.show_cli('--review-context','--page',self.entry['page'],*flags,success=False))
            self.assertEqual(failed['code'],code)
        manifest=self.run/'inputs/source-proof'/self.service/'read/citations.json';manifest.unlink()
        failed=json.loads(self.show_cli('--review-context','--page',self.entry['page'],'--view',view,success=False))
        self.assertEqual(failed['code'],'FROZEN_INPUTS_CHANGED')

    def test_review_context_explicit_group_exact_text_remaining_ids_and_source_only(self):
        view=self.prepare_review_context();self.save_local(self.local_page())
        page=json.loads(json.dumps(self.original['pages'][1]))
        page['fragments'] += [{'id':identity,'text':'\n\nUnique '+identity+' reader body.','evidence':[self.handle]}
                              for identity in ('extra','unselected')]
        self.save_local(page)
        selection=self.run/'workspace/selection.json'
        claims={self.entry['page']:{'entry':None},page['page']:{'extra':None}}
        D['save'](selection,{'claims':claims});before=D['tree'](self.run)
        text=self.show_cli('--review-context','--input',str(selection),'--service',self.service,'--view',view,'--limit','0')
        self.assertEqual(text.count(self.source['text']),1)
        for fragment in (self.local_page()['fragments'][1],page['fragments'][-2]):self.assertEqual(text.count(fragment['text']),1)
        self.assertNotIn(page['fragments'][-1]['text'],text)
        self.assertIn(page['page']+'#unselected',text)
        self.assertEqual(text.count(page['page']+'#extra'),1)
        self.assertEqual(before,D['tree'](self.run))
        source=self.show_cli('--review-context','--service',self.service,'--view',view,'--limit','0')
        self.assertEqual(source.count(self.source['text']),1)
        self.assertNotIn(page['fragments'][-2]['text'],source)
        for bad,code in (({'foreign.md':{'entry':None}},'SHOW_PAGE_NOT_FOUND'),
                         ({page['page']:{'deleted':None}},'SHOW_FRAGMENT_NOT_FOUND'),
                         ({page['page']:{str(i):None for i in range(9)}},'REVIEW_CONTEXT_SELECTION_TOO_LARGE')):
            D['save'](selection,{'claims':bad})
            result=json.loads(self.show_cli('--review-context','--input',str(selection),'--service',self.service,'--view',view,'--limit','0',success=False))
            self.assertEqual(result['code'],code)

    def test_review_context_projection_deduplicates_exact_bindings_but_never_files_or_revisions(self):
        for change in ({'file':'Other.cs'},{'revision':'c'*40}):
            packet,entries=self.projection_input([('abc',0,'a'*40),('abc',0,'a'*40)])
            regions,bindings,_=D['source_projection'](packet,entries)
            self.assertEqual(len(regions),1);self.assertEqual(len(bindings),2)
            packet['outcomes'][1]['source'].update(change)
            regions,bindings,_=D['source_projection'](packet,entries)
            self.assertEqual(len(regions),2);self.assertEqual(len(bindings),2)

    def test_context_display_citations_verify_full_coverage_instead_of_truncating(self):
        context=runpy.run_path(str(ROOT/'skills/codeclew/scripts/docs_review_context.py'))
        packet,entries=self.projection_input([('cde',2,'a'*40),('abcdef',0,'a'*40)])
        regions,bindings,_=D['source_projection'](packet,entries);key=next(iter(regions));region=regions[key]
        partial,full=list(bindings)
        self.assertEqual(context['display_citations'](key,region,bindings,[full],D),[full])
        self.assertEqual(context['display_citations'](key,region,bindings,[],D),[full])
        with self.assertRaises(D['Stop']) as error:context['display_citations'](key,region,bindings,[partial],D)
        self.assertEqual(error.exception.code,'REVIEW_SOURCE_COVERING_REFERENCE_INVALID')
        packet,entries=self.projection_input([('abc',0,'a'*40),('cde',2,'a'*40)])
        regions,bindings,_=D['source_projection'](packet,entries);key=next(iter(regions));region=regions[key]
        self.assertEqual(context['display_citations'](key,region,bindings,[],D),list(bindings))
        incomplete={**region,'citations':region['citations'][:1]}
        with self.assertRaises(D['Stop']) as error:context['display_citations'](key,incomplete,bindings,[],D)
        self.assertEqual(error.exception.code,'REVIEW_SOURCE_COVERAGE_INCOMPLETE')

    def test_compact_source_header_keeps_full_membership_candidates_and_alias_notes(self):
        # Controlled decoder/projection aliases isolate display from proof I/O.
        # The original retained source and real native-membership check remain.
        view=self.prepare_review_context();self.save_local(self.local_page())
        manifest=self.run/'inputs/source-proof'/self.service/'read/citations.json'
        loaded=D['M']['C']['load'](manifest);packet=loaded[1];entries=loaded[3]
        regions,bindings,_=D['source_projection'](packet,entries);key=view.split('/')[1]
        region=regions[key];handle=self.handle
        aliases=[handle.rsplit(':e',1)[0]+':e'+str(i) for i in range(2,25)]
        for alias in aliases:
            entries[alias]=json.loads(json.dumps(entries[handle]))
            bindings[alias]=json.loads(json.dumps(bindings[handle]))
        region['citations'] += aliases
        D['save'](manifest.parent.parent/'citation-index.json',bindings)
        pins=D['load'](self.run/'pins.json');pins['inputFiles']=D['tree'](self.run/'inputs');D['save'](self.run/'pins.json',pins)
        extra=json.loads(json.dumps(self.original['pages'][1]))
        extra['fragments'].append({'id':'alias-reader','text':'\n\nAn existing reader claim through another source binding.','evidence':[aliases[-1]]})
        self.save_local(extra)
        member=D['check_source_membership'];checked=[]
        def membership(*args):checked.append(args[2]);return member(*args)
        with patch.dict(D['M']['C'],{'load':lambda *a:loaded}),patch.dict(D,{'source_projection':lambda *a:(regions,bindings,{}),'check_source_membership':membership}):
            D['P']['save_notes'](self.run,self.service+'.md','Own notes through a non-displayed binding.',[aliases[-1]],D)
            from argparse import Namespace
            args=Namespace(review_context=True,page=[self.entry['page']],fragment='entry',service=None,view=view,
                           entries=False,entry=None,routes=False,route=None,offset=0,limit=0,include_notes=True)
            checked.clear();before=D['tree'](self.run)
            result=D['H']['show'](self.run,args,D);text=result['text']
            self.assertEqual(next(line for line in text.splitlines() if line.startswith('Cites: ')),'Cites: '+handle)
            self.assertIn('Other source bindings: 23',text)
            self.assertIn('citation-index.json',text);self.assertIn(self.service+'.md#alias-reader',text)
            self.assertIn('Own notes through a non-displayed binding.',text)
            self.assertEqual(len(checked),24);self.assertEqual(len(region['citations']),24)
            self.assertEqual(text.count(self.source['text']),1);self.assertEqual(before,D['tree'](self.run))

    def test_canonical_review_coexists_with_old_notes_without_stale_fallback_and_can_refresh_legacy(self):
        self.readmit_budget(200,1);self.local_reads();self.save_local(self.local_page())
        name=self.service+'.md';requirement=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        D['P']['save_notes'](self.run,name,'Previous source outline: '+self.source['text'],[self.handle],D)
        note=self.run/'workspace/notes'/(name+'.json');old_notes=note.read_bytes()
        D['W']['put'](self.run,'phase.md','capacity','## Capacity\n\nInspect retained reservation capacity.',[self.handle],False,D)
        D['W']['page_info'](self.run,'phase.md',self.service,'Capacity phase',False,D)
        shell=D['W']['review_page'](self.run,name,None,D)
        self.assertIn('text',shell['requirements'][requirement['id']]['sourceNotes'])
        for identity,row in shell['requirements'].items():
            row.update(disposition='EXPLAINED',section='reservations')
            kind=next(r['kind'] for r in self.inventory['requirements'] if r['id']==identity)
            if kind=='SERVICE':
                row['sourceNotes']={'references':[{'page':'phase.md','claim':'capacity'}]}
                for topic in row['topics']:row['topics'][topic]={'disposition':'EXPLAINED','page':name,'claim':'overview'}
            if kind=='DISCOVERY':row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
        shell['claims']={key:'SUPPORTED' for key in shell['claims']}
        D['W']['review_page'](self.run,name,shell,D)
        selected=D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]['sourceNotes']
        getter=D['W']['review_page'](self.run,name,None,D)
        self.assertEqual(getter['requirements'][requirement['id']]['sourceNotes'],selected)
        shell['requirements'][requirement['id']].pop('sourceNotes')
        D['W']['review_page'](self.run,name,shell,D)
        for page in (self.entry['page'],'phase.md'):
            assessment=D['W']['review_page'](self.run,page,None,D)
            for row in assessment['requirements'].values():row['disposition']='EXPLAINED'
            assessment['claims']={key:'SUPPORTED' for key in assessment['claims']}
            D['W']['review_page'](self.run,page,assessment,D)
        D['W']['finish_review'](self.run,{'unresolved':[],'visual':{'status':'UNAVAILABLE','reason':'Owned non-browser coexistence control'}},D)
        self.assertEqual(selected,D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]['sourceNotes'])
        self.assertEqual(old_notes,note.read_bytes())
        self.assertEqual(self.local_check(name)['status'],'UNIT_CHECKED')
        def completed_owned_output():
            self.original=D['load'](self.run/'workspace/draft/original.json')
            self.navigation=D['load'](self.run/'workspace/draft/navigation.json')
            self.review=D['load'](self.run/'workspace/draft/self-review.json')
            self.seed_files();self.generated();return D['verify'](self.run)
        final=completed_owned_output();self.assertEqual(final['status'],'READY',final)
        D['W']['put'](self.run,'phase.md','capacity','## Capacity\n\nChanged retained capacity explanation.',[self.handle],False,D)
        getter=D['W']['review_page'](self.run,name,None,D)
        self.assertEqual(getter['requirements'][requirement['id']]['sourceNotes'],selected)
        before=(self.run/'workspace/draft/self-review.json').read_bytes()
        with self.assertRaises(D['Stop']) as error:D['W']['review_page'](self.run,name,shell,D)
        self.assertIn('SERVICE_SOURCE_REVIEW_STALE',{i['code'] for i in error.exception.details['issues']})
        self.assertEqual(before,(self.run/'workspace/draft/self-review.json').read_bytes())
        phase_review=D['W']['review_page'](self.run,'phase.md',None,D);phase_review['claims']={'capacity':'SUPPORTED'}
        D['W']['review_page'](self.run,'phase.md',phase_review,D)
        local=self.local_check('phase.md');final=completed_owned_output()
        self.assertEqual(local['code'],'SERVICE_TOPIC_CLOSURE_REQUIRED',local)
        self.assertEqual(final['checks']['selfReview']['code'],local['code'])
        self.assertIn('SERVICE_SOURCE_REVIEW_STALE',{i['code'] for i in final['checks']['selfReview']['details']['issues']})
        self.assertEqual(old_notes,note.read_bytes())
        D['P']['save_notes'](self.run,name,'Explicitly refreshed source outline: '+self.source['text'],[self.handle],D)
        refreshed=D['W']['review_page'](self.run,name,None,D)['requirements'][requirement['id']]
        self.assertNotIn('references',refreshed['sourceNotes']);self.assertIsNone(refreshed['disposition'])
        self.assertIn('Explicitly refreshed',refreshed['sourceNotes']['text']);self.assertTrue(note.exists())

    def test_canonical_source_review_records_references_without_second_body_and_reuses_them(self):
        self.local_reads();self.save_local(self.local_page())
        name = self.service+'.md';requirement = next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        shell = D['W']['review_page'](self.run,name,None,D)
        for identity,row in shell['requirements'].items():
            row.update(disposition='EXPLAINED',section='reservations')
            kind = next(r['kind'] for r in self.inventory['requirements'] if r['id']==identity)
            if kind=='SERVICE':
                row['sourceNotes'] = {'references':[{'page':name,'claim':'overview'}]}
                for topic in row['topics']:row['topics'][topic]={'disposition':'EXPLAINED','page':name,'claim':'overview'}
            if kind=='DISCOVERY':row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
        shell['claims'] = {key:'SUPPORTED' for key in shell['claims']}
        original = D['load'](self.run/'workspace/draft/original.json')
        recorded = D['W']['review_page'](self.run,name,shell,D)
        self.assertFalse(recorded['semanticTruthAssessed'])
        row = D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]
        reference = row['sourceNotes']['references'][0]
        fragment = next(f for p in original['pages'] if p['page']==name for f in p['fragments'] if f.get('id')=='overview')
        self.assertEqual(reference,{'page':name,'claim':'overview','fragmentDigest':D['digest'](fragment),'evidence':[self.handle]})
        self.assertNotIn('text',row['sourceNotes']);self.assertFalse((self.run/'workspace/notes').exists())
        shell['requirements'][requirement['id']].pop('sourceNotes')
        D['W']['review_page'](self.run,name,shell,D)
        self.assertEqual(D['load'](self.run/'workspace/draft/self-review.json')['requirements'][requirement['id']]['sourceNotes'],row['sourceNotes'])
        self.assertEqual(original,D['load'](self.run/'workspace/draft/original.json'))
        entry_review=D['W']['review_page'](self.run,self.entry['page'],None,D)
        for account in entry_review['requirements'].values():account['disposition']='EXPLAINED'
        entry_review['claims']={key:'SUPPORTED' for key in entry_review['claims']}
        D['W']['review_page'](self.run,self.entry['page'],entry_review,D)
        self.original=original
        next(r for r in self.review['requirements'] if r['requirement']==requirement['id'])['sourceNotes']=row['sourceNotes']
        self.seed_files();self.generated();final=D['verify'](self.run)
        self.assertEqual(final['status'],'READY',final);self.assertFalse(final['semanticTruthAssessed'])

    def test_canonical_source_review_rejects_missing_foreign_stale_unbacked_and_explicit_bad_notes(self):
        name=self.service+'.md';requirement=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        accounts=json.loads(json.dumps(self.review['requirements']));row=next(r for r in accounts if r['requirement']==requirement['id'])
        fragment=next(f for f in self.original['pages'][1]['fragments'] if f.get('id')=='overview')
        reference={'page':name,'claim':'overview','fragmentDigest':D['digest'](fragment),'evidence':[self.handle]}
        row['sourceNotes']={'references':[reference]}
        def validate(rows=accounts,original=None,navigation=None,record=False,pages=None):
            return D['P']['validate'](self.inventory,rows,original or self.original,navigation or self.navigation,{self.handle:{'source':self.source}},D,record=record,pages=pages)
        self.assertIsNone(validate())
        bad_values=[(None,'SERVICE_SOURCE_NOTES_REQUIRED'),({},'SERVICE_SOURCE_NOTES_REQUIRED'),
            ({'references':[]},'SERVICE_SOURCE_NOTES_REQUIRED'),
            ({'text':'','evidence':[self.handle],'references':[reference]},'SERVICE_SOURCE_NOTES_REQUIRED'),
            ({'references':[None]},'SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED'),
            ({'references':[{'page':[],'claim':'overview'}]},'SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED'),
            ({'references':[{**reference,'claim':'missing'}]},'SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED'),
            ({'references':[{**reference,'fragmentDigest':'stale'}]},'SERVICE_SOURCE_REVIEW_STALE'),
            ({'references':[{**reference,'evidence':[]}]},'SERVICE_SOURCE_REVIEW_STALE')]
        for value,code in bad_values:
            changed=json.loads(json.dumps(accounts));next(r for r in changed if r['requirement']==requirement['id'])['sourceNotes']=value
            for record in (False,True):
                with self.assertRaises(D['Stop']) as error:validate(changed,record=record)
                self.assertIn(code,{i['code'] for i in error.exception.details['issues']})
        foreign=json.loads(json.dumps(self.navigation));foreign['services'][0]['pages'].remove(name)
        with self.assertRaises(D['Stop']) as error:validate(navigation=foreign)
        self.assertIn('SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        unbacked=json.loads(json.dumps(self.original));next(f for f in unbacked['pages'][1]['fragments'] if f.get('id')=='overview')['evidence']=[]
        with self.assertRaises(D['Stop']) as error:validate(original=unbacked)
        self.assertIn('SERVICE_SOURCE_REVIEW_EVIDENCE_REQUIRED',{i['code'] for i in error.exception.details['issues']})
        # Changed referenced pages are checked even if only their owning SERVICE
        # overview was reviewed earlier. No matching overview hash can hide this.
        shared={'id':'phase','text':'Capacity is checked before any hold.','evidence':[self.handle]}
        self.original['pages'].append({'page':'phase.md','fragments':[shared]})
        self.navigation['services'][0]['pages'].append('phase.md')
        row['sourceNotes']={'references':[{'page':'phase.md','claim':'phase','fragmentDigest':D['digest'](shared),'evidence':[self.handle]}]}
        shared['text']+=' Changed.'
        with self.assertRaises(D['Stop']) as error:validate(pages=['phase.md'])
        self.assertIn('SERVICE_SOURCE_REVIEW_STALE',{i['code'] for i in error.exception.details['issues']})
        # Preserve the same stale review in a genuinely completed owned backend
        # output to compare local/final diagnostics without bypassing completion.
        self.review['requirements']=accounts
        self.review['claims'].append({'page':'phase.md','claim':'phase','evidence':[self.handle],'disposition':'SUPPORTED'})
        self.seed_files();self.generated();local=self.local_check('phase.md');final=D['verify'](self.run)
        self.assertEqual(local['code'],'SERVICE_TOPIC_CLOSURE_REQUIRED',local)
        self.assertEqual(final['checks']['selfReview']['code'],local['code'])
        local_issue=next(i for i in local['details']['issues'] if i['code']=='SERVICE_SOURCE_REVIEW_STALE')
        final_issue=next(i for i in final['checks']['selfReview']['details']['issues'] if i['code']=='SERVICE_SOURCE_REVIEW_STALE')
        self.assertEqual(local_issue,final_issue)

    def test_data_section_cli_exact_target_local_final_parity_and_idempotence(self):
        self.readmit_budget(1000,2)
        name = self.service + '.md'
        fields = {'id':'fields', 'text':'\n\n## Reservation fields\n\n| Field | Meaning |\n| --- | --- |\n| identifier | The reservation identifier |\n', 'evidence':[self.handle]}
        self.original['pages'][1]['fragments'].append(fields)
        requirement = next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        row = next(r for r in self.review['requirements'] if r['requirement']==requirement['id'])
        row['topics']['data fields and domain distinctions'].update(claim='fields',fragmentDigest=D['digest'](fields))
        self.review['claims'].append({'page':name,'claim':'fields','evidence':[self.handle],'disposition':'SUPPORTED'})
        self.navigation['services'][0]['data'] = [{'title':'Reservation data','target':name}]
        for target in (name, name+'#clew-fragment-overview'):
            self.navigation['services'][0]['data'][0]['target'] = target
            self.seed_files();self.generated()
            local = self.local_check(name);final = D['verify'](self.run)
            self.assertEqual(local['code'],'SERVICE_TOPIC_CLOSURE_REQUIRED',local)
            self.assertEqual(final['checks']['selfReview']['code'],local['code'])
            local_issue = next(i for i in local['details']['issues'] if i['code']=='SERVICE_DATA_NAVIGATION_REQUIRED')
            final_issue = next(i for i in final['checks']['selfReview']['details']['issues'] if i['code']=='SERVICE_DATA_NAVIGATION_REQUIRED')
            self.assertEqual(local_issue,final_issue)
            self.assertEqual(local_issue['actual']['fragment'],'fields')
            self.assertIn('--data --fragment fields',local_issue['recovery'])
        before = D['load'](self.run/'workspace/draft/original.json')
        review = (self.run/'workspace/draft/self-review.json').read_bytes()
        navigation = D['load'](self.run/'workspace/draft/navigation.json')
        starts = navigation['services'][0]['starts']
        # Retain a different Data section; registration only replaces page top
        # and the exact section, never unrelated fragment destinations.
        navigation['services'][0]['data'].append({'title':'Old page target','target':name})
        D['save'](self.run/'workspace/draft/navigation.json',navigation)
        command = [sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
            'page-info','--run-dir',str(self.run),'--page',name,'--service',self.service,
            '--title','Reservation fields','--data','--fragment','fields']
        done = subprocess.run(command,capture_output=True,text=True)
        self.assertEqual(done.returncode,0,done.stdout+done.stderr)
        result = json.loads(done.stdout);target = name+'#clew-fragment-fields'
        self.assertEqual(result['fragmentTarget'],target)
        self.assertEqual(result['routeTarget']['anchor'],'clew-fragment-fields')
        after = D['load'](self.run/'workspace/draft/original.json')
        self.assertEqual([f for p in before['pages'] for f in p['fragments'] if 'id' in f],
                         [f for p in after['pages'] for f in p['fragments'] if 'id' in f])
        self.assertEqual(before['pages'][0],after['pages'][0])
        updated = D['load'](self.run/'workspace/draft/navigation.json')
        self.assertEqual(updated['services'][0]['starts'],starts)
        self.assertEqual(updated['services'][0]['data'],[
            {'title':'Reservation data','target':name+'#clew-fragment-overview'},
            {'title':'Reservation fields','target':target}])
        self.assertEqual(review,(self.run/'workspace/draft/self-review.json').read_bytes())
        stable = {path:D['tree'](self.run/path) for path in ('workspace/draft','workspace/pages')}
        repeated = subprocess.run(command,capture_output=True,text=True)
        self.assertEqual(repeated.returncode,0,repeated.stdout+repeated.stderr)
        self.assertEqual(json.loads(repeated.stdout),result)
        self.assertEqual(stable,{path:D['tree'](self.run/path) for path in stable})
        local = self.local_check(name);self.assertEqual(local['status'],'UNIT_CHECKED',local)
        html = (Path(local['directory'])/'html'/name.replace('.md','.html')).read_text()
        self.assertEqual(html.count('id="clew-fragment-fields"'),1)
        # The owned backend completes with this output; verification must still
        # require its real completion hash rather than accepting post-run edits.
        self.original = after;self.navigation = updated;self.seed_files();self.generated()
        final = D['verify'](self.run);self.assertEqual(final['status'],'READY',final)

    def test_data_section_invalid_flags_missing_foreign_and_unsafe_refuse_before_mutation(self):
        self.generated();name = self.service+'.md'
        (self.run/'workspace/.edit.lock').touch()
        command = [sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/docs_run.py'),
            'page-info','--run-dir',str(self.run),'--page',name,'--fragment','overview']
        for flags,code in ((['--data'],'DATA_FRAGMENT_METADATA_REQUIRED'),
                           (['--data','--service',self.service],'DATA_FRAGMENT_METADATA_REQUIRED'),
                           (['--data','--service',self.service,'--title','Fields','--summary','Summary'],'DATA_FRAGMENT_METADATA_REQUIRED'),
                           (['--title','Fields'],'SEPARATE_FRAGMENT_ADDRESS_AND_PAGE_METADATA')):
            before = D['tree'](self.run)
            failed = subprocess.run(command+flags,capture_output=True,text=True)
            self.assertEqual(json.loads(failed.stdout)['code'],code,failed.stdout)
            self.assertEqual(before,D['tree'](self.run))
        for fragment,owner,code in (('missing',self.service,'FRAGMENT_ADDRESS_NOT_FOUND'),
                                    ('overview','unknown','UNKNOWN_PAGE_OWNER')):
            before = D['tree'](self.run)
            with self.assertRaises(D['Stop']) as error:
                D['W']['page_info'](self.run,name,owner,'Fields',True,D,fragment=fragment)
            self.assertEqual(error.exception.code,code);self.assertEqual(before,D['tree'](self.run))
        for evidence in ({}, {self.handle:{'source':{**self.source,'service':'foreign'}}}):
            before = D['tree'](self.run)
            with patch.dict(D['P'],{'sources':lambda *a:evidence}):
                with self.assertRaises(D['Stop']) as error:
                    D['W']['page_info'](self.run,name,self.service,'Fields',True,D,fragment='overview')
            self.assertEqual(error.exception.code,'DATA_FRAGMENT_EVIDENCE_REQUIRED')
            self.assertEqual(before,D['tree'](self.run))
        navigation = D['load'](self.run/'workspace/draft/navigation.json')
        navigation['services'][0]['pages'].remove(name)
        navigation['services'].append({'id':'other','pages':[name]})
        D['save'](self.run/'workspace/draft/navigation.json',navigation)
        before = D['tree'](self.run)
        with self.assertRaises(D['Stop']) as error:
            D['W']['page_info'](self.run,name,self.service,'Fields',True,D,fragment='overview')
        self.assertEqual(error.exception.code,'PAGE_ALREADY_OWNED');self.assertEqual(before,D['tree'](self.run))
        D['save'](self.run/'workspace/draft/navigation.json',self.navigation)
        current = D['load'](self.run/'workspace/draft/original.json')
        page = next(p for p in current['pages'] if p['page']==name)
        page['fragments'] = [{'text':'```text\n\n'}, {'id':'unsafe','text':'literal source\n\n','evidence':[self.handle]}, {'text':'```\n'}]
        D['save'](self.run/'workspace/draft/original.json',current)
        before = D['tree'](self.run)
        with self.assertRaises(D['Stop']) as error:
            D['W']['page_info'](self.run,name,self.service,'Fields',True,D,fragment='unsafe')
        self.assertEqual(error.exception.code,'INVALID_FRAGMENT_ADDRESS');self.assertEqual(before,D['tree'](self.run))

    def test_dedicated_data_page_keeps_page_target_and_rejects_wrong_section(self):
        requirement = next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        fields = {'id':'fields','text':'# Reservation fields\n\n| Field | Meaning |\n| --- | --- |\n| identifier | The reservation identifier |','evidence':[self.handle]}
        self.original['pages'].append({'page':'data.md','fragments':[fields]})
        self.navigation['services'][0]['pages'].append('data.md')
        self.navigation['services'][0]['data'] = [{'title':'Reservation data','target':'data.md'}]
        row = next(r for r in self.review['requirements'] if r['requirement']==requirement['id'])
        row['topics']['data fields and domain distinctions'].update(page='data.md',claim='fields',fragmentDigest=D['digest'](fields))
        self.review['claims'].append({'page':'data.md','claim':'fields','evidence':[self.handle],'disposition':'SUPPORTED'})
        self.seed_files();self.generated()
        before = D['tree'](self.run/'workspace/draft')
        D['W']['page_info'](self.run,'data.md',self.service,'Reservation data',True,D)
        self.assertEqual(before,D['tree'](self.run/'workspace/draft'))
        final = D['verify'](self.run);self.assertEqual(final['status'],'READY',final)
        self.navigation['services'][0]['data'][0]['target']='data.md#wrong'
        with self.assertRaises(D['Stop']) as error:
            D['P']['validate'](self.inventory,self.review['requirements'],self.original,self.navigation,{self.handle:{'source':self.source}},D)
        self.assertIn('SERVICE_DATA_NAVIGATION_REQUIRED',{i['code'] for i in error.exception.details['issues']})

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

    def test_fragment_address_actual_parser_refuses_literal_context_and_show_never_claims_false_target(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        cases=[('backtick','```text\n\n','alpha\n\n','```\n'),
               ('tilde','~~~text\n\n','alpha\n\n','~~~\n'),
               ('indent','    prefix\n\n','    alpha\n\n',''),
               ('raw-pre','<pre>\n\n','alpha\n\n','</pre>\n'),
               ('raw-comment','<!--\n\n','alpha\n\n','-->\n')]
        for label,prefix,text,suffix in cases:
            name='literal-'+label+'.md'
            fragments=[{'text':prefix},{'id':'snippet','text':text,'evidence':[self.handle]},{'text':suffix}]
            self.save_local({'page':name,'fragments':fragments})
            before=D['tree'](self.run)
            with self.assertRaises(D['Stop']) as error:D['W']['address_fragment'](self.run,name,'snippet',D)
            self.assertEqual(error.exception.code,'INVALID_FRAGMENT_ADDRESS',label)
            self.assertEqual(error.exception.details['issues'][0]['code'],'FRAGMENT_ADDRESS_CONTEXT_UNSAFE')
            self.assertEqual(before,D['tree'](self.run))
            # A legacy caller may already have stored the old unsafe assembly.
            current=D['load'](self.run/'workspace/draft/original.json')
            page=next(p for p in current['pages'] if p['page']==name)
            page['fragments']=D['W']['address_body'](page['fragments'],name,'snippet',D)
            D['save'](self.run/'workspace/draft/original.json',current)
            before=D['tree'](self.run)
            shown=self.show_cli('--page',name,'--fragment','snippet','--limit','0')
            self.assertIn('Reader target unavailable:',shown,label)
            self.assertNotIn('Reader target: '+name+'#clew-fragment-snippet',shown)
            self.assertEqual(before,D['tree'](self.run))
            local=self.local_check(self.entry['page'])
            self.assertEqual(local['code'],'INVALID_FRAGMENT_ADDRESS',local)
        self.original=D['load'](self.run/'workspace/draft/original.json');self.seed_files();self.generated()
        final=D['verify'](self.run)
        self.assertEqual(final['checks']['output']['code'],'INVALID_FRAGMENT_ADDRESS')

    def test_fragment_address_actual_parser_preserves_complete_literal_blocks_unicode_and_crlf(self):
        self.local_reads();self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        cases=['```text\nalpha\n```','~~~text\nalpha\n~~~','    alpha','<pre>\nalpha\n</pre>','## Δ\r\n\r\nAlpha π.']
        for index,text in enumerate(cases):
            name='complete-'+str(index)+'.md'
            self.save_local({'page':name,'fragments':[{'id':'complete','text':text,'evidence':[self.handle]}]})
            result=D['W']['address_fragment'](self.run,name,'complete',D)
            page=next(p for p in D['load'](self.run/'workspace/draft/original.json')['pages'] if p['page']==name)
            claim=next(f for f in page['fragments'] if f.get('id')=='complete')
            self.assertEqual(claim['text'],text);self.assertEqual(claim['evidence'],[self.handle])
            D['W']['validate_fragment_addresses']({'pages':[page]},D,job=self.job)
            self.assertEqual(self.local_check(name)['status'],'UNIT_CHECKED')
            self.assertIn('Reader target: '+result['fragmentTarget'],self.show_cli('--page',name,'--fragment','complete','--limit','0'))

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
        before=D['tree'](self.run/'workspace/draft')
        with self.assertRaises(D['Stop']) as error:
            D['W']['put'](self.run,name,'extra','## Clew fragment overview\n\nAdditional explanation.',[self.handle],False,D)
        self.assertEqual(error.exception.code,'INVALID_FRAGMENT_ADDRESS')
        self.assertEqual(error.exception.details['issues'][0]['actual']['renderedTargets'],2)
        self.assertEqual(before,D['tree'](self.run/'workspace/draft'))
        self.original=D['load'](self.run/'workspace/draft/original.json')
        next(p for p in self.original['pages'] if p['page']==name)['fragments'].append(
            {'id':'extra','text':'\n\n## Clew fragment overview\n\nAdditional explanation.\n\n','evidence':[self.handle]})
        self.review['claims'].append({'page':name,'claim':'extra','evidence':[self.handle],'disposition':'SUPPORTED'})
        self.seed_files();self.generated()
        local=self.local_check(self.entry['page']);final=D['verify'](self.run)
        self.assertEqual(local['code'],'INVALID_FRAGMENT_ADDRESS',local)
        self.assertEqual(local['details']['issues'][0]['actual']['renderedTargets'],2)
        self.assertIn('./docs put',local['recovery'])
        self.assertEqual(final['checks']['output']['code'],local['code'],final)
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
        stale=json.loads(json.dumps(self.original));next(f for f in stale['pages'][1]['fragments'] if f.get('id')=='overview')['text']+='Changed meaning.'
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
        self.assertEqual(stored['topics']['contracts']['fragmentDigest'],D['digest'](next(f for f in self.original['pages'][1]['fragments'] if f.get('id')=='overview')))

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
        # Initial authored-output fixtures also exercise malformed author output.
        # Further turns must use the supported explicit context, never resume.
        if D['load'](self.run/'state.json')['attempts']:
            self.run_page(self.service+'.md')
            return D['load'](self.run/'state.json')
        return D['author'](self.run)

    def handoff(self, page, run=None):
        run=run or self.run
        if not (self.run/'inputs/source-proof'/self.service/'read/citations.json').exists():
            self.prepare_review_context()
        # Move the fixture's single proof carrier into frozen inputs. Keeping
        # another copy in the authored draft would duplicate its cite ID.
        for path in (self.seed/'reads',self.run/'workspace/draft/reads'):
            if path.exists():shutil.rmtree(path)
        views=D['load'](run/'inputs/author-source'/self.service/'views.json')
        D['DH']['prepare'](run,page,[self.service+'/'+v for v in views],D,renew=True)
        return D['load'](run/'workspace/author-context.json')

    def run_page(self, page, run=None):
        run=run or self.run
        self.handoff(page,run)
        return D['execute'](run)

    def author_page(self, page, run=None):
        run=run or self.run
        self.run_page(page,run)
        return D['load'](run/'state.json')

    def final_gate(self, run=None):
        run=run or self.run
        report=D['verify'](run)
        return D['promote'](run) if report['status']=='READY' else report

    def page_fixture(self):
        # The deterministic backend writes these owned authored outputs. Start
        # with reader addresses available so either page can be selected first.
        for name in ('original.json','navigation.json','self-review.json'):
            shutil.copyfile(self.seed/name,self.run/'workspace/draft'/name)
        (self.seed/'review-current').touch()

    def record_historical_attempt(self, run, usage, thread, *, kind='unit', unit=None):
        """Materialize an owned saved report, not the removed resume execution."""
        state=D['load'](run/'state.json');number=len(state['attempts'])
        folder=run/'attempts'/str(number);folder.mkdir()
        shutil.copytree(run/'workspace/draft',folder/'draft')
        attempt={'argv':['owned-historical-fixture','resume',thread], 'kind':kind,'unit':unit,
            'status':'COMPLETED','exitCode':0,'threadId':thread,'usage':usage,
            'draftFiles':D['tree'](run/'workspace/draft')}
        events=[{'type':'thread.started','thread_id':thread},{'type':'turn.completed','usage':usage}]
        (folder/'stdout.jsonl').write_text(''.join(json.dumps(e)+'\n' for e in events))
        (folder/'prompt.txt').write_text('Owned historical cumulative-counter fixture.\n')
        (folder/'stderr.log').write_text('');D['save'](folder/'result.json',attempt)
        state['attempts'].append(attempt)
        tokens=D['token_accounting'](state['attempts'])
        state.update(threadId=thread,status='AUTHOR_COMPLETED',modelTokens=tokens['budgetTokens'],
                     tokenAccounting=tokens,usageAvailable=tokens['status']=='MEASURED',completedUnit=unit)
        D['save'](run/'state.json',state)
        return state

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

    def test_route_occurrence_copies_prepared_target_and_reports_same_local_final_failure(self):
        self.readmit_budget(100,1);self.local_reads()
        self.save_local(self.local_page());self.save_local(self.original['pages'][1])
        info=D['W']['put'](self.run,self.service+'.md','effects','## Storage effects\n\nStore the reservation result.',[self.handle],False,D)
        self.assertEqual(info['routeTarget'],{'docset':'docs','page':self.service+'.html','anchor':'clew-fragment-effects'})
        self.assertEqual(D['W']['summary'](info)['routeTarget'],info['routeTarget'])
        self.assertIn('Route target: '+json.dumps(info['routeTarget'],separators=(',',':')),
                      self.show_cli('--page',self.service+'.md','--fragment','effects','--limit','0'))
        self.save_local({'page':'processes.md','fragments':[{'id':'catalog','text':'# Reservation processes\n','evidence':[self.handle]}]})
        diagram={'title':'Reserve','participants':[{'id':'caller','title':'Caller'},
            {'id':'reserve','title':'Reserve','target':self.entry['page']+'#handler'}],
            'steps':[{'from':'caller','to':'reserve','kind':'call','label':'Reserve','target':self.entry['page']+'#handler'}]}
        self.save_local({'page':'reserve-process.md','fragments':[{'id':'flow','text':'# Reserve\n\n```clew-sequence\n'+json.dumps(diagram)+'\n```\n','evidence':[self.handle]}]})
        value={'catalog':'processes.md','route':{'id':'reserve-route','version':'v1','title':'Reserve',
            'narrative':{'page':'reserve-process.md'},
            'origin':{'status':'known','occurrence':'request','kind':'http','title':'Reserve','summary':'Accept input'},
            'occurrences':[{'id':'request','actor':{'kind':'service','service':self.service},'role':'start','target':{'page':self.entry['page']}},
                {'id':'storage','actor':{'kind':'service','service':self.service},'role':'store','target':info['routeTarget']}],
            'branches':[],'outcomes':[],'coverage':{'outcomeInventory':'partial','scope':'Owned reservation','reference':{'page':'reserve-process.md'}}}}
        D['W']['route'](self.run,value,D)
        self.assertEqual(self.local_check(self.entry['page'])['status'],'UNIT_CHECKED')
        self.original=D['load'](self.run/'workspace/draft/original.json')
        self.review['claims']=[{'page':p['page'],'claim':f['id'],'evidence':f['evidence'],'disposition':'SUPPORTED'}
            for p in self.original['pages'] for f in p['fragments'] if 'id' in f]
        self.seed_files();self.generated();positive=D['verify'](self.run)
        self.assertEqual(positive['status'],'READY',{k:v for k,v in positive['checks'].items() if v['status']!='PASS'})
        value['route']['occurrences'][1]['target']={**info['routeTarget'],'anchor':'missing'}
        D['W']['route'](self.run,value,D)
        local=self.local_check(self.entry['page'])
        self.assertEqual(local['code'],'ROUTE_DECLARATION_INVALID',local)
        issue=local['details']['response']['details']['issues'][0]
        self.assertEqual(issue['routeId'],'reserve-route');self.assertEqual(issue['occurrenceId'],'storage')
        self.assertEqual(issue['page'],self.service+'.md')
        self.assertEqual(issue['field'],'/routes/0/occurrences/1/target')
        self.assertEqual(issue['actual']['anchor'],'missing')
        self.assertIn('./docs route',local['recovery'])
        self.assertTrue(any(x.get('routeId')=='reserve-route' and x.get('occurrenceId')=='storage' for x in local['locations']))
        # Finalization uses the same renderer predicate and retains typed detail
        # through the real CLI and checked-export failure receipts.
        shutil.copyfile(self.run/'workspace/draft/routes.json',self.seed/'routes.json')
        self.generated();final=D['verify'](self.run)
        self.assertEqual(final['checks']['finalize']['status'],'FAIL',final)
        failure=json.dumps(final['checks']['finalize'])
        for expected in ['reserve-route','storage','ROUTE_OCCURRENCE_TARGET_INVALID','missing','./docs route']:
            self.assertIn(expected,failure)

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
        D['save'](self.run / 'report.json',report)
        result=self.run_page(self.service+'.md');self.assertEqual(result['status'],'FAILED')
        sent=self.handoff(self.service+'.md');prompt=D['encode'](sent).decode()
        self.assertEqual(sent['localFailure']['code'],'PAGE_REVIEW_REQUIRED')
        self.assertNotIn('large preserved ledger',prompt)
        self.assertEqual(D['load'](self.run / 'report.json'),report)
        self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']),2)

    def successor_parent(self, *, ready=False):
        self.readmit_budget(8,0);self.mode.write_text('successor-accounting')
        self.prepare_review_context()
        (self.run/'inputs/author-source'/self.service/'view.md').write_text(self.source['text'])
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
            proof=self.run/'inputs/source-proof'
            if proof.exists():shutil.copytree(proof,directory.parent/'source-proof')
            if mutate:mutate(directory)
        with patch.dict(D['T'],{'collect':self.collect}),patch.dict(D['prepare'].__globals__,{'prepare_sources':sources}):
            return D['S']['prepare'](job_path,previous or self.run,D['SKILL'],child,D,**options)

    def compact_successor_parent(self):
        generated=self.generated
        def generate():
            self.save_local(self.local_page());self.save_local(self.original['pages'][1])
            for page in D['load'](self.run/'workspace/draft/original.json')['pages']:
                shell=D['W']['review_page'](self.run,page['page'],None,D)
                for identity,row in shell['requirements'].items():
                    row['disposition']='EXPLAINED'
                    kind=next(r['kind'] for r in self.inventory['requirements'] if r['id']==identity)
                    if kind=='SERVICE':
                        row['sourceNotes']={'references':[{'page':page['page'],'claim':'overview'}]}
                        row['topics']={topic:{'disposition':'EXPLAINED','page':page['page'],'claim':'overview'} for topic in row['topics']}
                    if kind=='DISCOVERY':row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
                shell['claims']={identity:'SUPPORTED' for identity in shell['claims']}
                D['W']['review_page'](self.run,page['page'],shell,D)
            D['W']['finish_review'](self.run,{'unresolved':[],'visual':{'status':'UNAVAILABLE','reason':'Owned protocol control'}},D)
            self.original=D['load'](self.run/'workspace/draft/original.json')
            self.navigation=D['load'](self.run/'workspace/draft/navigation.json')
            self.review=D['load'](self.run/'workspace/draft/self-review.json');self.seed_files()
            return generated()
        self.generated=generate
        try:return self.successor_parent(ready=True)
        finally:self.generated=generated

    def test_rejected_review_requires_current_page_decisions_even_after_old_files_and_completed_turn(self):
        job_path=self.compact_successor_parent()
        job=D['load'](job_path);job['budget'].update(maxModelTokens=6,maxRepairs=1);D['save'](job_path,job)
        draft=self.run/'workspace/draft';before_draft=D['tree'](draft)
        old={name:(draft/name).read_bytes() for name in ('self-review.json','reviewed-pages.json')}
        global_decision={k:self.review[k] for k in ('unresolved','visual')}
        self.assertEqual(D['review_for_run'](self.run),self.review)
        self.assertEqual(D['W']['finish_review'](self.run,global_decision,D)['status'],'GLOBAL_REVIEW_RECORDED')
        self.assertEqual(before_draft,D['tree'](draft));before=D['tree'](self.run)
        child=self.root/'rejected';self.prepare_owned_successor(job_path,child,revise_ready=True)
        for name,raw in old.items():self.assertEqual((child/'history'/name).read_bytes(),raw)
        self.assertEqual(D['load'](child/'workspace/draft/reviewed-pages.json'),{})
        self.assertEqual(D['load'](child/'workspace/draft/original.json'),self.original)
        self.assertEqual(D['S']['prepared_bindings'](child,D),D['S']['prepared_bindings'](self.run,D))
        for name,raw in old.items():
            (child/'workspace/draft'/name).write_bytes(raw)
            (self.seed/name).write_bytes(raw)
        getter=D['review_for_run'](child)
        self.assertTrue(all(v is None for claims in getter['claims'].values() for v in claims.values()))
        service=next(r for r in self.inventory['requirements'] if r['kind']=='SERVICE')
        row=getter['requirements'][service['id']]
        self.assertEqual(row['sourceNotes'],self.review['requirements'][service['id']]['sourceNotes'])
        self.assertTrue(all(t['disposition'] is None and t['claim']=='overview' for t in row['topics'].values()))
        with self.assertRaises(D['Stop']) as error:D['W']['finish_review'](child,global_decision,D)
        self.assertEqual(error.exception.code,'QUALITY_REJECTED_REVIEW_PENDING')
        pending=error.exception.details
        local=D['W']['check'](child,[],False,D)
        self.assertEqual(local['pendingReview']['pages'],pending['pages'])
        self.author_page(self.service+'.md',child)  # Copy-only backend restores old reviews, not their validity.
        final=D['verify'](child)
        self.assertEqual(final['checks']['selfReview']['code'],'QUALITY_REJECTED_REVIEW_PENDING',final)
        self.assertEqual(final['checks']['selfReview']['details']['claims'],pending['claims'])
        approved={'requirements':{key:json.loads(json.dumps(value)) for key,value in self.review['requirements'].items()
            if any(r['id']==key and r['page']==service['page'] for r in self.inventory['requirements'])},
            'claims':self.review['claims'][service['page']]}
        for reference,code in (({'page':'foreign.md','claim':'overview'},'SERVICE_SOURCE_REVIEW_REFERENCE_REQUIRED'),
                               ({'page':service['page'],'claim':'overview','fragmentDigest':'sha256:stale'},'SERVICE_SOURCE_REVIEW_STALE')):
            value=json.loads(json.dumps(approved));value['requirements'][service['id']]['sourceNotes']={'references':[reference]}
            with self.assertRaises(D['Stop']) as error:D['W']['review_page'](child,service['page'],value,D)
            self.assertIn(code,[i['code'] for i in error.exception.details['issues']])
            self.assertEqual(D['S']['pending_review'](child,D)['pages'],pending['pages'])
        for page in self.original['pages']:
            shell=D['W']['review_page'](child,page['page'],None,D)
            for identity,row in shell['requirements'].items():
                row['disposition']='EXPLAINED'
                for topic in (row.get('topics') or {}).values():topic['disposition']='EXPLAINED'
                if 'discoveryDisposition' in row:row['discoveryDisposition']='NO_ADDITIONAL_STARTS'
            shell['claims']={identity:'SUPPORTED' for identity in shell['claims']}
            D['W']['review_page'](child,page['page'],shell,D)
            remaining=D['S']['pending_review'](child,D)
            self.assertNotIn(page['page'],remaining['pages'])
            if page==self.original['pages'][0]:self.assertEqual(len(remaining['pages']),1)
        frozen=self.root/'reviewed-freeze';shutil.copytree(child,frozen)
        self.assertEqual(D['S']['pending_review'](frozen,D),D['S']['pending_review'](child,D))
        self.assertEqual(D['W']['finish_review'](child,global_decision,D)['status'],'GLOBAL_REVIEW_RECORDED')
        for name in old:shutil.copyfile(child/'workspace/draft'/name,self.seed/name)
        self.author_page(self.service+'.md',child)  # Fresh turn pins the newly submitted decisions.
        final=D['verify'](child);self.assertEqual(final['status'],'READY',final)
        self.assertEqual(D['load'](child/'state.json')['modelTokens'],14)
        self.assertEqual(before,D['tree'](self.run))

    def test_rejected_page_omitted_decisions_stop_without_dispatching_another_page(self):
        job_path=self.compact_successor_parent()
        job=D['load'](job_path);job['budget'].update(maxModelTokens=6,maxRepairs=0);D['save'](job_path,job)
        child=self.root/'page-omitted';self.prepare_owned_successor(job_path,child,revise_ready=True)
        page=self.service+'.md';result=self.run_page(page,child)
        self.assertEqual(result['status'],'FAILED');self.assertEqual(result['page'],page)
        local=result['checks']['local'];self.assertEqual(local['code'],'PAGE_REVIEW_REQUIRED')
        self.assertEqual(local['selectedPages'],[page]);self.assertIn(page,local['pendingReview']['pages'])
        state=D['load'](child/'state.json')
        self.assertEqual([a['unit'] for a in state['attempts'][1:]],['page:'+page])
        pending=D['S']['pending_review'](child,D)
        self.assertEqual(pending['pages'],[p['page'] for p in self.original['pages']])
        self.assertEqual(pending['claims'],[{'page':p['page'],'claim':f['id']}
            for p in self.original['pages'] for f in p['fragments'] if 'id' in f])
        self.assertEqual(D['verify'](child)['checks']['selfReview']['code'],'QUALITY_REJECTED_REVIEW_PENDING')
        self.assertFalse((child/'ready').exists());self.assertFalse(local['semanticTruthAssessed'])

    def test_rejected_pages_preserve_unchanged_text_accounting_and_explicit_full_gate(self):
        job_path=self.compact_successor_parent()
        job=D['load'](job_path);job['budget'].update(maxModelTokens=15,maxRepairs=1);D['save'](job_path,job)
        child=self.root/'page-rejected';self.prepare_owned_successor(job_path,child,revise_ready=True)
        receipt=D['load'](child/'inputs/successor.json');self.assertTrue(receipt['fullFinalGateRequired'])
        D['W']['put'](child,'dynamic.md','details','## Retained details\n\nAdditional retained declaration details.',[self.handle],False,D)
        D['W']['page_info'](child,'dynamic.md',self.service,'Retained details',False,D)
        D['W']['put'](child,'later.md','shared','## Shared result\n\nShared retained result.',[self.handle],False,D)
        for name in ('original.json','navigation.json'):shutil.copyfile(child/'workspace/draft'/name,self.seed/name)
        (self.seed/'review-current').touch()
        for page in (self.entry['page'],self.service+'.md'):
            self.assertEqual(self.run_page(page,child)['status'],'PAGE_COMPLETED')
        stable={p['page']:D['digest'](p) for p in D['load'](child/'workspace/draft/original.json')['pages'] if p['page']!='dynamic.md'}
        (self.seed/'review-current').unlink()
        failed=self.run_page('dynamic.md',child);self.assertEqual(failed['status'],'FAILED')
        self.assertEqual(failed['checks']['local']['selectedPages'],['dynamic.md'])
        self.assertIn('dynamic.md',failed['checks']['local']['pendingReview']['pages'])
        self.assertEqual(D['S']['pending_review'](child,D)['pages'],['dynamic.md','later.md'])
        (self.seed/'review-current').touch()
        self.assertEqual(self.run_page('dynamic.md',child)['status'],'PAGE_COMPLETED')
        self.assertEqual(D['S']['pending_review'](child,D)['pages'],['later.md'])
        for page in D['load'](child/'workspace/draft/original.json')['pages']:
            if page['page'] in stable:self.assertEqual(D['digest'](page),stable[page['page']])
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page('later.md',child)['status'],'PAGE_COMPLETED')
        self.assertEqual(self.final_gate(child)['status'],'READY_PROMOTED')
        state=D['load'](child/'state.json')
        self.assertEqual([a['kind'] for a in state['attempts']],['full','successor-review','unit','unit','repair','unit'])
        self.assertEqual(state['modelTokens'],23)
        self.assertEqual(D['scoped_budget_tokens'](state,D['token_accounting'](state['attempts'])),15)
        self.assertEqual(D['load'](child/'report.json')['checks']['author']['inheritedModelTokens'],8)

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

    def budget_stopped_parent(self):
        job_path=self.compact_successor_parent()
        job=D['load'](job_path);job['budget']['maxModelTokens']=2;D['save'](job_path,job)
        previous=self.root/'budget-stopped';self.prepare_owned_successor(job_path,previous,revise_ready=True)
        with self.assertRaises(D['Stop']) as error:self.author_page(self.service+'.md',previous)  # Fresh deterministic turn, genuine 3 > 2 stop.
        self.assertEqual(error.exception.code,'AUTHOR_TOKEN_BUDGET_EXCEEDED')
        state=D['load'](previous/'state.json');self.assertEqual(state['status'],'AUTHOR_BUDGET_EXCEEDED')
        self.assertEqual(state['modelTokens'],11);self.assertEqual(state['successor']['inheritedModelTokens'],8)
        self.assertFalse((previous/'report.json').exists());(previous/'.runner.lock').touch()
        job['budget']['maxModelTokens']=3;D['save'](job_path,job)
        return previous,job_path

    def test_budget_stopped_successor_prepare_preserves_absence_history_scope_and_pending_review(self):
        previous,job_path=self.budget_stopped_parent()
        frozen=self.root/'budget-freeze';shutil.copytree(previous,frozen)
        before=D['tree'](frozen);child=self.root/'after-budget-stop'
        with patch.dict(D,{'author':lambda *a,**k:self.fail('prepare must not launch author')}):
            result=self.prepare_owned_successor(job_path,child,previous=frozen,revise_budget_stopped=True)
        self.assertEqual(result['status'],'SUCCESSOR_PREPARED');self.assertEqual(result['modelCalls'],0)
        self.assertEqual(result['inheritedModelTokens'],11);self.assertEqual(result['continuationModelTokens'],0)
        self.assertEqual(before,D['tree'](frozen))
        state=D['load'](child/'state.json');old=D['load'](frozen/'state.json')
        self.assertEqual(state['attempts'],old['attempts']);self.assertEqual(state['threadId'],old['threadId'])
        self.assertEqual(D['tree'](child/'attempts'),D['tree'](frozen/'attempts'))
        self.assertEqual(state['successor']['inheritedAttemptCount'],2);self.assertEqual(state['unitCursor'],0)
        self.assertEqual(D['scoped_budget_tokens'](state,D['token_accounting'](state['attempts'])),0)
        self.assertEqual(D['load'](child/'workspace/draft/original.json'),D['load'](frozen/'workspace/draft/original.json'))
        self.assertEqual(D['S']['prepared_bindings'](child,D),D['S']['prepared_bindings'](frozen,D))
        receipt=D['load'](child/'inputs/successor.json')
        self.assertEqual(receipt['predecessorTerminal'],{'status':'AUTHOR_BUDGET_EXCEEDED','nativeReportPresent':False,
            'budgetScope':'NEW_ATTEMPTS_ONLY','scopedModelTokens':3,'originalLimit':2})
        self.assertEqual(receipt['callerDecision']['reason'],'INDEPENDENT_QUALITY_REJECTED')
        self.assertEqual(receipt['callerDecision']['parentNativeStatus'],'UNAVAILABLE')
        self.assertNotIn('report.json',receipt['predecessorFiles']);self.assertNotIn('report.json',receipt['archiveFiles'])
        self.assertFalse((child/'history/report.json').exists());self.assertFalse((child/'report.json').exists())
        self.assertEqual(D['load'](child/'history/state.json'),old)
        self.assertEqual(receipt['originalLimits']['maxModelTokens'],2);self.assertEqual(receipt['continuationAllowance']['maxModelTokens'],3)
        self.assertFalse(receipt['standaloneFullDocsetReviewRequired']);self.assertTrue(receipt['fullFinalGateRequired'])
        self.assertTrue(D['S']['pending_review'](child,D)['pages'])
        self.assertTrue(all(v is None for claims in D['review_for_run'](child)['claims'].values() for v in claims.values()))
        self.assertEqual(D['S']['prompt_facts'](child,D),{'predecessorMachineFailure':None,'predecessorTerminal':receipt['predecessorTerminal']})
        self.assertEqual(D['S']['validate'](child,state,D),2)
        self.assertEqual(len(state['attempts']),2);self.assertFalse((child/'ready').exists())

    def test_budget_stopped_successor_rejects_live_incomplete_unknown_or_unproven_stop(self):
        previous,job_path=self.budget_stopped_parent()
        immutable={name:(previous/name).read_bytes() for name in ('state.json','pins.json','inputs/job.json')}
        with self.assertRaises(D['Stop']) as error:D['recover_accounting'](previous,D['SKILL'])
        # A fresh-context budget stop is not the historical same-thread false
        # stop targeted by recover-accounting. Its truthful chain is retained.
        self.assertEqual(error.exception.code,'AUTHOR_HISTORY_MISMATCH')
        self.assertEqual(immutable,{name:(previous/name).read_bytes() for name in immutable})
        before=D['tree'](previous)  # Recovery's ordinary native input probes may leave diagnostic receipts.
        with (previous/'.runner.lock').open('rb') as lock:
            D['fcntl'].flock(lock,D['fcntl'].LOCK_EX|D['fcntl'].LOCK_NB)
            with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/'live-budget',previous=previous,revise_budget_stopped=True)
            self.assertEqual(error.exception.code,'SUCCESSOR_PREDECESSOR_BUSY')
        state_path=previous/'state.json';saved=state_path.read_bytes();state=D['load'](state_path)
        for change,code in (({'status':'AUTHOR_RUNNING'},'SUCCESSOR_BUDGET_STOP_REQUIRED'),
                            ({'attemptStatus':'RUNNING'},'SUCCESSOR_HISTORY_UNVERIFIABLE'),
                            ({'exitCode':23},'SUCCESSOR_HISTORY_UNVERIFIABLE'),
                            ({'modelTokens':12},'SUCCESSOR_ACCOUNTING_UNVERIFIABLE')):
            value=json.loads(json.dumps(state))
            if 'attemptStatus' in change:value['attempts'][-1]['status']=change['attemptStatus']
            elif 'exitCode' in change:value['attempts'][-1]['exitCode']=change['exitCode']
            else:value.update(change)
            D['save'](state_path,value)
            with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/('bad-'+code),previous=previous,revise_budget_stopped=True)
            self.assertEqual(error.exception.code,code);state_path.write_bytes(saved)
        raw=previous/'attempts/1/stdout.jsonl';original=raw.read_bytes();value=json.loads(json.dumps(state))
        value['attempts'][-1]['usage'].pop('cached_input_tokens');D['save'](state_path,value)
        events=[json.loads(line) for line in raw.read_text().splitlines()]
        for event in events:
            if event['type']=='turn.completed':event['usage']=value['attempts'][-1]['usage']
        raw.write_text(''.join(json.dumps(e)+'\n' for e in events))
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/'unknown-budget',previous=previous,revise_budget_stopped=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_ACCOUNTING_UNVERIFIABLE');state_path.write_bytes(saved);raw.write_bytes(original)
        D['save'](previous/'report.json',{'status':'FAILED'})
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/'has-report',previous=previous,revise_budget_stopped=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_BUDGET_STOP_REQUIRED');(previous/'report.json').unlink()
        with self.assertRaises(D['Stop']):self.prepare_owned_successor(job_path,self.root/'implicit-budget',previous=previous)
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/'conflict-budget',previous=previous,revise_ready=True,revise_budget_stopped=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_REVISION_MODE_CONFLICT')
        # Owned invalid fixture: its total is above 8, but its NEW_ATTEMPTS_ONLY use 3 is below a claimed limit 4.
        job=D['load'](previous/'inputs/job.json');job['budget']['maxModelTokens']=4;D['save'](previous/'inputs/job.json',job)
        pins_path=previous/'pins.json';pins_raw=pins_path.read_bytes();pins=D['load'](pins_path)
        pins['jobDigest']=D['digest'](job);pins['inputFiles']=D['tree'](previous/'inputs');D['save'](pins_path,pins)
        with self.assertRaises(D['Stop']) as error:self.prepare_owned_successor(job_path,self.root/'false-budget',previous=previous,revise_budget_stopped=True)
        self.assertEqual(error.exception.code,'SUCCESSOR_GENUINE_BUDGET_STOP_REQUIRED')
        job['budget']['maxModelTokens']=2;D['save'](previous/'inputs/job.json',job);pins_path.write_bytes(pins_raw)
        self.assertEqual(before,D['tree'](previous))

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
        self.author_page(self.service+'.md',child)
        result=D['verify'](child)
        self.assertEqual(result['status'],'FAILED',result)  # Broken link remains; full final gate is mandatory.
        state=D['load'](child / 'state.json');self.assertEqual(state['modelTokens'],11)
        self.assertEqual(state['attempts'][1]['kind'],'successor-review')
        self.assertNotEqual(state['attempts'][1]['threadId'],state['attempts'][0]['threadId'])
        self.assertNotIn('resume',state['attempts'][1]['argv'])
        self.assertEqual(D['scoped_budget_tokens'](state,D['token_accounting'](state['attempts'])),3)
        self.assertEqual(result['checks']['author']['reportedModelTokens'],11)
        self.assertEqual(result['checks']['author']['inheritedModelTokens'],8)
        self.assertEqual(result['checks']['author']['continuationModelTokens'],3)
        prompt=json.loads((child / 'attempts/1/prompt.txt').read_text())
        self.assertEqual(prompt['request'],self.job['request'])
        self.assertEqual(prompt['previousThreadId'],state['attempts'][0]['threadId'])
        self.assertEqual(prompt['scope']['pages'],[p['page'] for p in self.original['pages']])
        self.assertEqual(D['load'](child / 'history/report.json')['status'],'FAILED')
        self.assertEqual(parent_draft,D['tree'](self.run / 'workspace/draft'))
        for name,value in before.items():
            self.assertEqual(D['sha']((self.run / name).read_bytes()),value)
            self.assertEqual(D['sha']((frozen / name).read_bytes()),value)

    def test_successor_fresh_page_reviews_reach_full_gate_without_hiding_inherited_cost(self):
        job_path=self.successor_parent();child=self.root/'successor'
        job=D['load'](job_path);job['budget']['maxModelTokens']=6;D['save'](job_path,job)
        self.prepare_owned_successor(job_path,child)
        self.original['pages'][0]['fragments'][1]['text']=self.original['pages'][0]['fragments'][1]['text'].replace('\n[Missing](absent.md)\n','')
        self.seed_files();(self.seed/'review-current').touch()
        self.assertEqual(self.run_page(self.entry['page'],child)['status'],'PAGE_COMPLETED')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md',child)['status'],'PAGE_COMPLETED')
        result=self.final_gate(child);self.assertEqual(result['status'],'READY_PROMOTED',result)
        report=D['load'](child/'report.json');self.assertTrue(all(row['status']=='PASS' for row in report['checks'].values()))
        self.assertEqual(report['checks']['author']['reportedModelTokens'],14)
        self.assertEqual(report['checks']['author']['continuationModelTokens'],6)
        self.assertEqual(D['load'](self.run/'report.json')['status'],'FAILED')

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
        job_path=self.successor_parent();child=self.root/'successor';self.prepare_owned_successor(job_path,child)
        # Historical cumulative reports remain accepted inputs to accounting,
        # without redispatching the removed same-thread continuation path.
        usage={'input_tokens':5,'cached_input_tokens':0,'output_tokens':0}
        self.record_historical_attempt(child,usage,'owned-test-session',kind='successor-review')
        result=D['verify'](child);self.assertEqual(result['checks']['author']['code'],'AUTHOR_USAGE_INVALID')
        state=D['load'](child/'state.json')
        self.assertEqual(state['attempts'][0]['usage']['input_tokens'],8)
        self.assertEqual(state['attempts'][1]['usage'],usage)
        self.assertEqual(D['token_accounting'](state['attempts'])['status'],'INVALID')
        self.assertFalse((child/'ready').exists())

    def test_successor_rejects_reused_context_and_preserves_the_failed_attempt_usage(self):
        job_path=self.successor_parent();child=self.root/'successor';self.prepare_owned_successor(job_path,child)
        self.mode.write_text('context-reused')
        with self.assertRaises(D['Stop']) as error:self.run_page(self.service+'.md',child)
        self.assertEqual(error.exception.code,'AUTHOR_CONTEXT_NOT_FRESH')
        state=D['load'](child/'state.json')
        self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['attempts'][1]['usage'],{'input_tokens':20,'cached_input_tokens':10,'output_tokens':10})
        self.assertEqual(state['attempts'][0]['threadId'],'owned-test-session')
        self.assertEqual(state['attempts'][1]['threadId'],'owned-test-session')
        self.assertFalse((child/'ready').exists())

    def test_process_page_without_full_sequence_is_explicitly_rejected(self):
        request = self.root / 'sequences.json'
        D['save'](request, {'pages': {'process.md': '# Process\nOnly links.'}, 'requirements': [], 'accounts': [], 'processPages': ['process.md']})
        with self.assertRaises(D['Stop']) as error:
            D['command']([self.job['node'], str(ROOT / 'skills/codeclew/scripts/docs_run_sequences.mjs'), str(RENDERER), str(request)], self.root / 'sequence-check', 'check')
        self.assertEqual(error.exception.code, 'PROCESS_SEQUENCE_MISSING')
        self.assertEqual(error.exception.details['response']['details']['actual'], 0)

    def test_inventory_scaffold_renders_but_is_explicitly_unfinished(self):
        D['W']['install'](self.inventory, self.run, D)
        D['save'](self.run / 'workspace/draft/navigation.json', D['navigation_starter'](self.inventory))
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
        D['W']['install'](self.inventory, self.run, D)
        navigation = json.loads(json.dumps(self.navigation))
        navigation['services'][0]['data'][0]['target'] = self.service + '.md'
        D['save'](self.run / 'workspace/draft/navigation.json', navigation)
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

    def test_explicit_new_owned_page_is_checked_before_final_gate(self):
        self.original['pages'].append({'page':'data.md','fragments':[{'text':'# Data\n\n' + D['W']['MARKER'] + '\n'}]})
        self.navigation['services'][0]['pages'].append('data.md');self.seed_files();self.page_fixture()
        result=self.run_page('data.md')
        self.assertEqual(result['status'],'FAILED',result)
        self.assertEqual(result['page'],'data.md')
        self.assertEqual(result['checks']['local']['code'],'UNFINISHED_SCAFFOLD')
        self.assertIn('data.md',result['checks']['local']['selectedPages'])
        self.assertEqual(len(D['load'](self.run/'state.json')['attempts']),1)
        self.assertNotEqual(self.final_gate()['status'],'READY_PROMOTED')

    def test_explicit_pages_use_fresh_contexts_aggregate_budget_and_full_promotion(self):
        self.page_fixture()
        self.assertEqual(self.run_page(self.entry['page'])['status'],'PAGE_COMPLETED')
        self.assertFalse((self.run/'ready').exists())
        first=D['tree'](self.run/'workspace/draft')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md')['status'],'PAGE_COMPLETED')
        self.assertFalse((self.run/'ready').exists())
        self.assertEqual(self.final_gate()['status'],'READY_PROMOTED')
        state=D['load'](self.run/'state.json')
        self.assertEqual([a['kind'] for a in state['attempts']],['unit','unit'])
        self.assertEqual(state['modelTokens'],40)
        self.assertNotEqual(state['attempts'][0]['threadId'],state['attempts'][1]['threadId'])
        self.assertTrue(all('resume' not in a['argv'] for a in state['attempts']))
        self.assertEqual(state['attempts'][0]['draftFiles'],first)
        report=D['load'](self.run/'report.json')
        self.assertTrue(all(row['status']=='PASS' for row in report['checks'].values()),report)

    def test_early_page_failure_requires_explicit_local_repair_before_advancing(self):
        self.readmit_budget(1000,1);self.mode.write_text('repair-on-handoff')
        self.original['pages'][0]['fragments'][1]['text']+='\n[Missing](absent.html)\n';self.seed_files();self.page_fixture()
        first=self.run_page(self.entry['page']);self.assertEqual(first['status'],'FAILED',first)
        self.assertEqual(len(D['load'](self.run/'state.json')['attempts']),1)
        repaired=self.run_page(self.entry['page']);self.assertEqual(repaired['status'],'PAGE_COMPLETED',repaired)
        # Carry the repaired output forward, preserving the other page.
        shutil.copyfile(self.run/'workspace/draft/original.json',self.seed/'original.json')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md')['status'],'PAGE_COMPLETED')
        self.assertEqual(self.final_gate()['status'],'READY_PROMOTED')
        state=D['load'](self.run/'state.json')
        self.assertEqual([a['kind'] for a in state['attempts']],['unit','repair','unit'])
        self.assertEqual(state['attempts'][0]['unit'],state['attempts'][1]['unit'])
        self.assertEqual(state['modelTokens'],60)
        self.assertIn('[Missing]',(self.run/'attempts/0/draft/original.json').read_text())

    def test_advisory_reserve_does_not_replace_the_actual_aggregate_cap(self):
        self.readmit_budget(35,0);self.page_fixture()
        self.assertEqual(self.run_page(self.entry['page'])['status'],'PAGE_COMPLETED')
        with self.assertRaises(D['Stop']) as error:self.run_page(self.service+'.md')
        self.assertEqual(error.exception.code,'AUTHOR_TOKEN_BUDGET_EXCEEDED')
        state=D['load'](self.run/'state.json');self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['modelTokens'],40)
        self.assertNotEqual(self.final_gate()['status'],'READY_PROMOTED')
        self.assertFalse((self.run/'ready').exists())

    def test_uneven_pages_are_not_stopped_by_unproven_repair_proxy(self):
        self.readmit_budget(250000,0);self.mode.write_text('uneven-units');self.page_fixture()
        self.assertEqual(self.run_page(self.entry['page'])['status'],'PAGE_COMPLETED')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md')['status'],'PAGE_COMPLETED')
        self.assertEqual(self.final_gate()['status'],'READY_PROMOTED')
        state=D['load'](self.run/'state.json');self.assertEqual(state['modelTokens'],132000)
        self.assertEqual([a['usage']['input_tokens'] for a in state['attempts']],[130000,2000])

    def test_no_token_cutoff_still_requires_measured_usage_and_full_export(self):
        self.readmit_budget(None,0);self.mode.write_text('budget');self.page_fixture()
        self.assertEqual(self.run_page(self.entry['page'])['status'],'PAGE_COMPLETED')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md')['status'],'PAGE_COMPLETED')
        self.assertEqual(self.final_gate()['status'],'READY_PROMOTED')
        state=D['load'](self.run/'state.json');self.assertGreater(state['modelTokens'],250000)
        state['attempts'][-1]['usage'].pop('cached_input_tokens');D['save'](self.run/'state.json',state)
        failed=self.check();self.assertEqual(failed['checks']['author']['code'],'AUTHOR_USAGE_UNAVAILABLE')
        with self.assertRaises(D['Stop']) as error:self.author_page(self.service+'.md')
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
        assessment=D['W']['review_page'](self.run,page['page'],None,D)
        for row in assessment['requirements'].values():row['disposition']='EXPLAINED'
        assessment['claims']={key:'SUPPORTED' for key in assessment['claims']}
        D['W']['review_page'](self.run,page['page'],assessment,D)
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
        self.prepare_review_context();shutil.rmtree(self.seed/'reads')
        D['author'](self.run,unit=scaffold['units'][0])
        state=self.record_historical_attempt(self.run,{'input_tokens':2793295,'cached_input_tokens':2670976,
            'output_tokens':62544,'reasoning_output_tokens':43098},'owned-test-session',unit='places')
        self.assertEqual(state['modelTokens'],184863)
        for attempt in state['attempts']:attempt.pop('threadId')
        state.update(status='AUTHOR_BUDGET_EXCEEDED',modelTokens=305173,unitCursor=1)
        D['save'](self.run / 'state.json',state)
        old = self.root / 'old-skill'
        shutil.copytree(ROOT / 'skills/codeclew',old,ignore=shutil.ignore_patterns('node_modules','__pycache__','.git'))
        with (old / 'scripts/docs_run.py').open('a') as stream:stream.write('\n# Owned previous-admission fixture.\n')
        pins=D['load'](self.run/'pins.json');pins['skillFiles'] = D['tree'](old,skill=True);D['save'](self.run / 'pins.json',pins)
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
        self.mode.write_text('normal');(self.seed/'review-current').touch()
        self.assertEqual(self.run_page(self.entry['page'])['status'],'PAGE_COMPLETED')
        (self.seed/'finish-current').touch()
        self.assertEqual(self.run_page(self.service+'.md')['status'],'PAGE_COMPLETED')
        final=self.final_gate();self.assertEqual(final['status'],'READY_PROMOTED',final)
        continued=D['load'](self.run/'state.json')
        self.assertEqual([a['unit'] for a in continued['attempts'][:2]],['dashboard','places'])
        self.assertEqual(continued['modelTokens'],184903)
        self.assertEqual(continued['authorStartedAt'],original['authorStartedAt'])
        self.assertTrue(all('resume' not in a['argv'] for a in continued['attempts'][2:]))
        continuation=D['load'](self.run/'attempts/2/context.json')
        self.assertEqual(continuation['previousThreadId'],'owned-test-session')
        self.assertNotIn('OLD_TOOL_HISTORY',D['encode'](continuation).decode())

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
            self.author_page(self.service+'.md')
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

    def generated_for_test_reset(self, *, explicit_page=None):
        state = D['load'](self.run / 'state.json'); state.update(attempts=[], authorStartedAt=None, status='PREPARED', modelTokens=0, usageAvailable=True, handoff=None, threadId=None)
        D['save'](self.run / 'state.json', state)
        shutil.rmtree(self.run / 'attempts', ignore_errors=True)
        if explicit_page:
            self.page_fixture()
            return self.run_page(explicit_page)
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
            with self.assertRaises(D['Stop']):self.generated_for_test_reset(explicit_page=self.entry['page'])
            result = self.check(); self.assertNotEqual(result['status'], 'READY')
            self.assertNotEqual(D['load'](self.run / 'state.json')['status'], 'AUTHOR_COMPLETED')
            with self.assertRaises(D['Stop']):D['promote'](self.run)
            before = len(D['load'](self.run / 'state.json')['attempts'])
            with self.assertRaises(D['Stop']) as stopped:D['execute'](self.run)
            self.assertEqual(stopped.exception.code,'AUTHOR_HANDOFF_ATTEMPT_EXISTS')
            self.assertEqual(len(D['load'](self.run / 'state.json')['attempts']), before)

    def test_interruption_preserves_previous_ready_and_is_not_complete(self):
        job = D['load'](self.run / 'inputs/job.json'); job['budget']['wallSeconds'] = 1
        D['save'](self.run / 'inputs/job.json', job)
        pins = D['load'](self.run / 'pins.json'); pins['jobDigest'] = D['digest'](job); pins['inputFiles'] = D['tree'](self.run / 'inputs'); D['save'](self.run / 'pins.json', pins)
        self.mode.write_text('interrupt')
        self.page_fixture()
        with self.assertRaises(D['Stop']):self.run_page(self.entry['page'])
        self.assertEqual(D['load'](self.run / 'state.json')['status'], 'INTERRUPTED')
        self.assertNotEqual(self.check()['status'], 'READY')
        self.assertFalse((self.run / 'ready').exists())
        attempts=D['load'](self.run/'state.json')['attempts']
        with self.assertRaises(D['Stop']) as stopped:D['execute'](self.run)
        self.assertEqual(stopped.exception.code,'AUTHOR_HANDOFF_ATTEMPT_EXISTS')
        self.assertEqual(D['load'](self.run/'state.json')['attempts'],attempts)

    def test_source_backed_false_assertion_is_not_a_semantic_certificate(self):
        self.original['pages'][0]['fragments'][1]['text'] += '\nDelivery is guaranteed.\n'
        self.seed_files(); self.generated(); result = self.check()
        self.assertEqual(result['status'], 'READY', result)
        self.assertFalse(result['semanticTruthAssessed'])

    def test_explicit_verification_and_promotion_do_not_launch_another_author(self):
        self.generated()
        with patch.dict(D['verify'].__globals__,{'author':lambda *a,**k:self.fail('verify/promote must not launch an author')}):
            result=self.final_gate()
        self.assertEqual(result['status'],'READY_PROMOTED',result)
        self.assertEqual(len(D['load'](self.run/'state.json')['attempts']),1)

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


class HandoffTests(unittest.TestCase):
    """Recording backend only: inputs, page loop, lineage and no implicit resume."""
    def setUp(self):
        record=F['source']()
        record['text']='public class Ledger { string schema="SCHEMA_SENTINEL"; string Run(bool allowed) { if (!allowed) return "BRANCH_SENTINEL"; Publish("EFFECT_SENTINEL"); return "REPLY_SENTINEL"; } }'
        record['textDigest']=D['sha'](record['text'].encode())
        self.f=DocsRunTests()
        with patch.dict(F,{'source':lambda:json.loads(json.dumps(record))}):self.f.setUp()
        self.addCleanup(self.f.doCleanups);f=self.f
        f.readmit_budget(1000,1)
        f.prepare_review_context();shutil.rmtree(f.seed/'reads')
        f.save_local(f.local_page());f.save_local(f.original['pages'][1])
        f.original=D['load'](f.run/'workspace/draft/original.json')
        f.original['pages'][0]['fragments'][1]['text']+='\nUNRELATED_COMPLETED_BODY_SENTINEL\n'
        overview=next(x for x in f.original['pages'][1]['fragments'] if x.get('id')=='overview')
        overview['text']+='\nCURRENT_DRAFT_SENTINEL\n'
        for row in f.review['requirements']:
            for topic in row.get('topics',{}).values():topic['fragmentDigest']=D['digest'](overview)
        f.navigation=D['load'](f.run/'workspace/draft/navigation.json')
        f.seed_files();f.generated()
        history=f.run/'attempts/0/stdout.jsonl'
        with history.open('a') as stream:stream.write(json.dumps({'type':'item.completed','item':{
            'type':'command_execution','aggregated_output':'OLD_TOOL_HISTORY_SENTINEL'+'x'*500000}})+'\n')
        D['P']['save_notes'](f.run,f.service+'.md','OWN_NOTE_SENTINEL',[f.handle],D)
        path=f.run/'inputs/author-source'/f.service/'requirements.json';rows=D['load'](path)
        rows['scope-fixture']={'page':f.service+'.md','views':[v['view'] for v in D['load'](path.parent/'views.json').values()]}
        D['save'](path,rows)
        f.backend.write_text('#!'+sys.executable+'\n'+'''
import json,sys,subprocess
from pathlib import Path
raw=sys.stdin.read();packet=json.loads(raw);workspace=Path.cwd()
n=packet['accounting']['historicalAttempts']
(workspace/('recorded-'+str(n)+'.json')).write_text(json.dumps({'stdin':raw,'argv':sys.argv}))
page=packet['currentPage']['page']
shell=json.loads(subprocess.check_output(['./docs','review-page','--page',page],text=True))
for row in shell['requirements'].values():
 row['disposition']='EXPLAINED'
 if 'topics' in row:
  row['sourceNotes']={'references':[{'page':page,'claim':'overview'}]}
  row['topics']={k:{'disposition':'EXPLAINED','page':page,'claim':'overview'} for k in row['topics']}
 if 'discoveryDisposition' in row:row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
shell['claims']={k:'SUPPORTED' for k in shell['claims']}
assessment=workspace/'recording-review.json';assessment.write_text(json.dumps(shell))
subprocess.run(['./docs','review-page','--page',page,'--input',str(assessment)],check=True,stdout=subprocess.DEVNULL)
print(json.dumps({'type':'thread.started','thread_id':'recording-context-'+str(n)}))
print(json.dumps({'type':'turn.completed','usage':{'input_tokens':100,'cached_input_tokens':20,'output_tokens':5}}))
''')
        pins=D['load'](f.run/'pins.json');pins['authorExecutableDigest']=D['sha'](f.backend.read_bytes())
        pins['inputFiles']=D['tree'](f.run/'inputs');D['save'](f.run/'pins.json',pins)
        self.page=f.service+'.md'

    def test_explicit_check_and_caller_require_rereview_after_page_metadata_changes(self):
        f=self.f;page=f.entry['page'];other=self.page
        shell=D['W']['review_page'](f.run,other,None,D)
        for row in shell['requirements'].values():
            row['disposition']='EXPLAINED'
            if 'topics' in row:
                row['sourceNotes']={'references':[{'page':other,'claim':'overview'}]}
                row['topics']={k:{'disposition':'EXPLAINED','page':other,'claim':'overview'} for k in row['topics']}
            if 'discoveryDisposition' in row:row.update(discoveryDisposition='NO_ADDITIONAL_STARTS',discoveredEntries=[])
        shell['claims']={key:'SUPPORTED' for key in shell['claims']}
        D['W']['review_page'](f.run,other,shell,D)
        other_review=D['W']['review_page'](f.run,other,None,D)
        other_stamp=D['load'](f.run/'workspace/draft/reviewed-pages.json')[other]
        other_body=next(p for p in D['load'](f.run/'workspace/draft/original.json')['pages'] if p['page']==other)
        code='''
before=subprocess.run(['./docs','check','--page',page],capture_output=True,text=True)
subprocess.run(['./docs','page-info','--page',page,'--service','reservations',
 '--title','Reserve an item','--summary','Validate and accept a reservation.'],check=True,stdout=subprocess.DEVNULL)
missing=subprocess.run(['./docs','check','--page',page],capture_output=True,text=True)
repeat=subprocess.run(['./docs','check','--page',page],capture_output=True,text=True)
scaffold=subprocess.run(['./docs','check','--page',page,'--scaffold'],capture_output=True,text=True)
subprocess.run(['./docs','review-page','--page',page,'--input',str(assessment)],check=True,stdout=subprocess.DEVNULL)
after=subprocess.run(['./docs','check','--page',page],capture_output=True,text=True)
(workspace/'metadata-check-trace.json').write_text(json.dumps({
 key:{'exit':value.returncode,'result':json.loads(value.stdout)}
 for key,value in [('before',before),('missing',missing),('repeat',repeat),('scaffold',scaffold),('after',after)]}))
'''
        text=f.backend.read_text();marker="print(json.dumps({'type':'thread.started'"
        f.backend.write_text(text.replace(marker,code+'\n'+marker))
        pins=D['load'](f.run/'pins.json');pins['authorExecutableDigest']=D['sha'](f.backend.read_bytes());D['save'](f.run/'pins.json',pins)
        D['DH']['prepare'](f.run,page,[],D)
        result=D['execute'](f.run);self.assertEqual(result['status'],'PAGE_COMPLETED',result)
        trace=D['load'](f.run/'workspace/metadata-check-trace.json')
        for key in ('before','after'):
            self.assertEqual(trace[key]['exit'],0,trace)
            self.assertEqual(trace[key]['result']['status'],'UNIT_CHECKED')
            self.assertTrue(trace[key]['result']['sourceBindingsChecked'])
        for key in ('missing','repeat'):
            self.assertEqual(trace[key]['exit'],2)
            self.assertEqual(trace[key]['result']['code'],'PAGE_REVIEW_REQUIRED')
            self.assertEqual(trace[key]['result']['details']['pages'],[page])
            self.assertIn('./docs review-page --page '+page+' --input FILE',trace[key]['result']['recovery'])
        self.assertEqual(trace['scaffold']['result']['status'],'SCAFFOLD_CHECKED')
        self.assertEqual(other_review,D['W']['review_page'](f.run,other,None,D))
        self.assertEqual(other_stamp,D['load'](f.run/'workspace/draft/reviewed-pages.json')[other])
        self.assertEqual(other_body,next(p for p in D['load'](f.run/'workspace/draft/original.json')['pages'] if p['page']==other))
        self.assertEqual(D['load'](f.run/'state.json')['modelTokens'],105)

    def test_fresh_page_request_preserves_materials_and_omits_history_with_idempotent_run(self):
        f=self.f
        prepared=D['DH']['prepare'](f.run,self.page,[],D)
        packet=D['load'](f.run/'workspace/author-context.json');wire=D['encode'](packet)
        self.assertEqual(D['DH']['prepare'](f.run,self.page,[],D)['context'],prepared['context'])
        for marker in ('SCHEMA_SENTINEL','BRANCH_SENTINEL','EFFECT_SENTINEL','REPLY_SENTINEL',
                       'CURRENT_DRAFT_SENTINEL','OWN_NOTE_SENTINEL',f.handle):self.assertIn(marker,wire.decode())
        for marker in ('OLD_TOOL_HISTORY_SENTINEL','UNRELATED_COMPLETED_BODY_SENTINEL'):self.assertNotIn(marker,wire.decode())
        self.assertIn(f.entry['page'],packet['scope']['pages'])
        self.assertEqual(packet['snapshot'],f.job['snapshot']);self.assertEqual(packet['model'],f.job['author']['model'])
        self.assertEqual(packet['sources'][0]['text'],f.source['text'])
        self.assertIn(f.handle,packet['sources'][0]['citations'])
        result=D['execute'](f.run);self.assertEqual(result['status'],'PAGE_COMPLETED',result)
        recorded=D['load'](f.run/'workspace/recorded-1.json')
        self.assertEqual(recorded['stdin'].encode(),wire);self.assertNotIn('resume',recorded['argv'])
        state=D['load'](f.run/'state.json');self.assertEqual(len(state['attempts']),2)
        self.assertEqual(state['attempts'][0]['threadId'],'owned-test-session')
        self.assertEqual(state['attempts'][1]['threadId'],'recording-context-1')
        self.assertEqual(state['modelTokens'],105)
        self.assertEqual(D['S']['history'](f.run,state,D)[1]['budgetTokens'],105)
        again=D['execute'](f.run,debug=True)
        self.assertEqual(again['status'],result['status']);self.assertFalse(again['observability']['historyReplayed'])
        self.assertEqual(D['load'](f.run/'state.json')['attempts'],state['attempts'])
        self.assertEqual(D['DH']['prepare'](f.run,self.page,[],D)['context'],prepared['context'])
        self.assertEqual(len(D['load'](f.run/'state.json')['attempts']),2)
        renewed=D['DH']['prepare'](f.run,self.page,[],D,renew=True)
        self.assertNotEqual(renewed['context'],prepared['context'])
        self.assertEqual(D['execute'](f.run)['status'],'PAGE_COMPLETED')
        state=D['load'](f.run/'state.json');self.assertEqual(state['modelTokens'],190)
        self.assertEqual(state['threadId'],'recording-context-2')
        self.assertEqual(D['S']['history'](f.run,state,D)[1]['budgetTokens'],190)

    def test_failed_page_check_prepares_same_runtime_successor_without_an_author(self):
        f=self.f;page=f.entry['page']
        marker="print(json.dumps({'type':'thread.started'"
        edit="subprocess.run(['./docs','page-info','--page',page,'--service','reservations','--title','Reserve an item','--summary','Validate and accept a reservation.'],check=True,stdout=subprocess.DEVNULL)\n"
        f.backend.write_text(f.backend.read_text().replace(marker,edit+marker))
        pins=D['load'](f.run/'pins.json');pins['authorExecutableDigest']=D['sha'](f.backend.read_bytes());D['save'](f.run/'pins.json',pins)
        D['DH']['prepare'](f.run,page,[],D)
        result=D['execute'](f.run);self.assertEqual(result['status'],'FAILED')
        self.assertEqual(result['checks']['local']['code'],'PAGE_REVIEW_REQUIRED')
        report=D['verify'](f.run);self.assertEqual(report['status'],'FAILED')
        self.assertEqual(report['checks']['author']['status'],'PASS')
        (f.run/'.runner.lock').touch();before=D['tree'](f.run)
        job=f.root/'continuation-job.json';D['save'](job,f.job);child=f.root/'same-campaign-successor'
        def sources(job,inventory,directory):
            shutil.copytree(f.run/'inputs/author-source',directory)
            shutil.copytree(f.run/'inputs/source-proof',directory.parent/'source-proof')
        with patch.dict(D['T'],{'collect':f.collect}),patch.dict(D['prepare'].__globals__,{'prepare_sources':sources}),\
             patch.dict(D,{'author':lambda *a,**k:self.fail('prepare/handoff must not launch an author')}):
            prepared=D['S']['prepare'](job,f.run,D['SKILL'],child,D)
            handoff=D['DH']['prepare'](child,page,[],D)
        self.assertEqual(prepared['modelCalls'],0);self.assertEqual(handoff['modelCalls'],0)
        self.assertEqual(prepared['inheritedModelTokens'],105)
        self.assertEqual(before,D['tree'](f.run))
        self.assertEqual(D['tree'](child/'workspace/draft'),D['tree'](f.run/'workspace/draft'))
        self.assertEqual(D['load'](child/'state.json')['attempts'],D['load'](f.run/'state.json')['attempts'])
        self.assertEqual(D['load'](child/'inputs/job.json')['clew'],f.job['clew'])
        self.assertEqual(D['load'](child/'inputs/job.json')['snapshot'],f.job['snapshot'])
        self.assertEqual(D['S']['prepared_bindings'](child,D),D['S']['prepared_bindings'](f.run,D))
        self.assertEqual(D['load'](child/'pins.json')['clewCapabilities'],D['load'](f.run/'pins.json')['clewCapabilities'])
        self.assertFalse((child/'attempts/2').exists())

    def test_restart_after_recorded_turn_checks_it_once_without_redispatch(self):
        f=self.f;D['DH']['prepare'](f.run,self.page,[],D)
        def interrupted_check(*a,**k):raise OSError('Owned caller interruption after recorded author completion')
        with patch.dict(D['W'],{'check':interrupted_check}):
            with self.assertRaises(OSError):D['execute'](f.run)
        state=D['load'](f.run/'state.json')
        self.assertEqual(len(state['attempts']),2);self.assertEqual(state['handoff']['status'],'PREPARED')
        self.assertEqual(D['execute'](f.run)['status'],'PAGE_COMPLETED')
        self.assertEqual(len(D['load'](f.run/'state.json')['attempts']),2)
        self.assertFalse((f.run/'workspace/recorded-2.json').exists())

    def test_successor_preserves_context_lineage_and_requires_its_own_handoff(self):
        f=self.f;D['DH']['prepare'](f.run,self.page,[],D)
        self.assertEqual(D['execute'](f.run)['status'],'PAGE_COMPLETED')
        report=D['verify'](f.run);self.assertEqual(report['status'],'FAILED')
        (f.run/'.runner.lock').touch();before=D['tree'](f.run)
        job=f.root/'next-job.json';D['save'](job,f.job);child=f.root/'next-context'
        def sources(job,inventory,directory):
            shutil.copytree(f.run/'inputs/author-source',directory)
            shutil.copytree(f.run/'inputs/source-proof',directory.parent/'source-proof')
        with patch.dict(D['T'],{'collect':f.collect}),patch.dict(D['prepare'].__globals__,{'prepare_sources':sources}):
            prepared=D['S']['prepare'](job,f.run,D['SKILL'],child,D)
        self.assertEqual(prepared['inheritedModelTokens'],105)
        self.assertIsNone(D['load'](child/'state.json')['handoff'])
        self.assertEqual(before,D['tree'](f.run))
        with self.assertRaises(D['Stop']) as error:D['execute'](child)
        self.assertEqual(error.exception.code,'AUTHOR_HANDOFF_REQUIRED')
        D['DH']['prepare'](child,self.page,[],D)
        self.assertEqual(D['execute'](child)['status'],'PAGE_COMPLETED')
        state=D['load'](child/'state.json');self.assertEqual(state['modelTokens'],190)
        self.assertEqual(D['scoped_budget_tokens'](state,D['token_accounting'](state['attempts'])),85)
        self.assertEqual(D['S']['validate'](child,state,D),2)
        self.assertEqual(state['attempts'][-1]['kind'],'successor-review')

    def test_inflight_handoff_is_not_rearmed_or_dispatched(self):
        f=self.f;prepared=D['DH']['prepare'](f.run,self.page,[],D)
        state=D['load'](f.run/'state.json')
        state['attempts'].append({'contextHandoff':prepared['context'],'status':'RUNNING','usage':None})
        state['status']='AUTHOR_RUNNING';D['save'](f.run/'state.json',state)
        with self.assertRaises(D['Stop']) as error:D['execute'](f.run)
        self.assertEqual(error.exception.code,'AUTHOR_HANDOFF_ATTEMPT_EXISTS')
        with self.assertRaises(D['Stop']) as error:D['DH']['prepare'](f.run,self.page,[],D,renew=True)
        self.assertEqual(error.exception.code,'AUTHOR_HANDOFF_BUSY')
        self.assertFalse((f.run/'workspace/recorded-1.json').exists())

    def test_legacy_continuation_refuses_before_backend_and_unknown_usage_is_not_erased(self):
        f=self.f
        with self.assertRaises(D['Stop']) as error:D['execute'](f.run)
        self.assertEqual(error.exception.code,'AUTHOR_HANDOFF_REQUIRED')
        self.assertFalse((f.run/'workspace/recorded-1.json').exists())
        state=D['load'](f.run/'state.json');state['attempts'][0]['usage']=None
        state['modelTokens']=None;D['save'](f.run/'state.json',state)
        D['DH']['prepare'](f.run,self.page,[],D)
        packet=D['load'](f.run/'workspace/author-context.json')
        self.assertEqual(packet['accounting']['status'],'UNAVAILABLE')
        with self.assertRaises(D['Stop']) as error:D['execute'](f.run)
        self.assertEqual(error.exception.code,'AUTHOR_USAGE_UNAVAILABLE')
        self.assertEqual(len(D['load'](f.run/'state.json')['attempts']),1)
        self.assertFalse((f.run/'workspace/recorded-1.json').exists())

    def test_required_uncited_source_survives_empty_draft_and_stale_handoff_does_not_launch(self):
        f=self.f
        original=D['load'](f.run/'workspace/draft/original.json')
        next(p for p in original['pages'] if p['page']==self.page)['fragments']=[{'text':'# Unwritten current page'}]
        D['save'](f.run/'workspace/draft/original.json',original)
        D['DH']['prepare'](f.run,self.page,[],D)
        packet=D['load'](f.run/'workspace/author-context.json')
        self.assertIn('BRANCH_SENTINEL',packet['sources'][0]['text'])
        self.assertTrue(packet['scope']['sourceViews']);self.assertTrue(packet['decisions']['requirements'])
        original['pages'][1]['fragments'][0]['text']+='\nChanged after handoff.'
        D['save'](f.run/'workspace/draft/original.json',original)
        with self.assertRaises(D['Stop']) as error:D['execute'](f.run)
        self.assertEqual(error.exception.code,'AUTHOR_HANDOFF_STALE')
        self.assertFalse((f.run/'workspace/recorded-1.json').exists())


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
