#!/usr/bin/env python3
"""Owned path/transport regressions; no private sources or installed mutations."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
S = runpy.run_path(str(ROOT/'skills/codeclew/scripts/work_session.py'))


def work_page(items=(), cursor=None, total=None, **fields):
    return {'schema':'codeclew-documentation-work-page/1.0','work':'work-1','snapshot':'sha256:owned/123',
            'subject':'service:sample','audience':'Maintainers','contextDigest':'context','inputDigest':'inputs',
            'membershipDigest':'members','receiptDigest':'receipt','authority':'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED',
            'influenceCoverage':'RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED','items':list(items),'omitted':[],
            'nextCursor':cursor,'total':len(items) if total is None else total,**fields}


def proposal_page(status='READY_WITH_LIMITATIONS'):
    return {'schema':'codeclew-docs-page/1.0','reportSchema':'codeclew-documentation-proposal-result/1.0',
            'proposal':'proposal-1','work':'work-1','status':status,'meaningReview':'UNASSESSED',
            'readDigest':'reads','narrativeDigest':None,'total':0,'items':[],'omitted':[],'nextCursor':None}


def proposal(participants=()):
    return {'schema':'codeclew-documentation-proposal/1.0','operations':[{
        'entrypoint':'e1','title':'Owned operation','summary':{'text':'Native feedback first','evidence':['s1']},
        'participants':list(participants),'steps':[{'kind':'alt','meaning':{'text':'Owned guard','evidence':['s1']},
        'children':[{'kind':'note','meaning':{'text':'Owned effect','evidence':['s1']}}],
        'otherwise':[{'kind':'return','meaning':{'text':'Owned fallback','evidence':['s1']}}]}]},
        {'entrypoint':'e2','title':'Unrelated operation','summary':{'text':'Preserve this','evidence':['s2']},'steps':[]}]}


class SessionTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name);self.assigned=self.base/'assigned';self.assigned.mkdir()
        self.docs=self.base/'docs';self.docs.mkdir();self.launcher=self.base/'clew';self.calls=self.base/'calls.jsonl'
        self.write('work/page.stdout',work_page())
        self.write('draft.json',proposal());self.write('selection.json',{'query':{'kind':'SYMBOL','symbolContains':'OwnedService'}})
        self.responses={'read':{'FIRST':work_page()},'submit':proposal_page('NEEDS_REPAIR'),'show':proposal_page('NEEDS_REPAIR')}
        self.driver()

    def write(self,name,value):
        p=self.assigned/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(value));return p

    def driver(self):
        config=self.base/'responses.json';config.write_text(json.dumps(self.responses))
        self.launcher.write_text(f'#!{sys.executable}\n'+f'''import json,sys
from pathlib import Path
args=sys.argv[1:]
with Path({str(self.calls)!r}).open('a') as out:out.write(json.dumps(args)+'\\n')
cfg=json.loads(Path({str(config)!r}).read_text())
if args[:3]==['docs','work','read']:
 selection=json.loads(Path(args[args.index('--input')+1]).read_text())
 result=cfg['read'][selection.get('cursor','FIRST')]
elif args[:3]==['docs','proposal','submit']:
 result=cfg['submit']
else:
 assert args[:3]==['docs','proposal','show']
 result=cfg['show']
 if 'schema' not in result and 'processExit' not in result:result=result[args[args.index('--cursor')+1] if '--cursor' in args else 'FIRST']
if 'processExit' in result:
 sys.stdout.write(result['raw']);sys.stderr.write(result['stderr']);sys.exit(result['processExit'])
print(json.dumps(result))
''');self.launcher.chmod(0o700)

    def run_session(self,action='submit',input_name='draft.json',output='attempt-1',**options):
        return S['run'](action,self.launcher,self.docs,self.assigned,'work-1',Path(input_name),Path(output),
                        work_page=Path('work/page.stdout') if action!='read' else None,**options)

    def test_cli_relative_inputs_are_independent_of_cwd_and_preserve_query_cursor(self):
        row={'kind':'DEPENDENCY','id':'method-1','reference':'d1','record':{'kind':'SYMBOL','symbol':'OwnedService'}}
        self.responses['read']={'FIRST':work_page([],cursor='next',total=1),'next':work_page([row],total=1)};self.driver()
        process=subprocess.run([sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/work_session.py'),'read',
            '--launcher',str(self.launcher),'--root',str(self.docs),'--input-root',str(self.assigned),'--work','work-1',
            '--input','selection.json','--output','nested/read','--max-pages','2'],cwd=self.base,capture_output=True,text=True)
        self.assertEqual(process.returncode,0,process.stderr)
        self.assertTrue(json.loads(process.stdout)['selectionComplete'])
        second=json.loads((self.assigned/'nested/read/read-collection/page-0002.selection.json').read_text())
        self.assertEqual(second,{'query':{'kind':'SYMBOL','symbolContains':'OwnedService'},'cursor':'next'})

    def test_missing_foreign_and_symlink_inputs_have_zero_native_calls(self):
        foreign=self.base/'foreign.json';foreign.write_text('{}');(self.assigned/'link.json').symlink_to(foreign)
        for index,name in enumerate(['nested/missing.json',str(foreign),'../foreign.json','link.json']):
            result=self.run_session(input_name=name,output=f'bad-{index}')
            self.assertEqual(result['status'],'BLOCKED');self.assertEqual(result['nativeCallsAttempted'],0)
            self.assertIn('INPUT_',result['reason'])
        self.assertFalse(self.calls.exists())
        self.assertIn('same Work',self.run_session(input_name='missing.json',output='explain')['reason'])

    def test_output_and_context_failures_cannot_mutate_or_overwrite(self):
        existing=self.assigned/'existing';existing.mkdir();(existing/'result.json').write_text('KEEP')
        result=self.run_session(output='existing');self.assertEqual(result['status'],'BLOCKED')
        self.assertEqual((existing/'result.json').read_text(),'KEEP');self.assertFalse(self.calls.exists())
        self.write('work/page.stdout',work_page(work='foreign'))
        result=self.run_session(output='foreign-context');self.assertIn('WORK_IDENTITY_CHANGED',result['reason']);self.assertFalse(self.calls.exists())

    def test_all_reserved_participants_reported_together_and_external_never_removed(self):
        bad=[{'id':name,'label':name} for name in ['caller','sample','service-sample']]
        self.write('bad.json',proposal(bad));r=self.run_session(input_name='bad.json',output='bad')
        self.assertEqual([v['id'] for v in r['participantIssues']],['caller','sample','service-sample'])
        self.assertEqual(r['nativeCallsAttempted'],0)
        self.assertEqual(json.loads((self.assigned/'bad/input.original.json').read_text()),proposal(bad))
        external={'id':'provider','label':'External provider','externalService':'gateway'}
        self.write('external.json',proposal([external]));r=self.run_session('preflight',input_name='external.json',output='valid')
        self.assertEqual(r['status'],'LOCAL_PREFLIGHT_COMPLETE');self.assertFalse(self.calls.exists())
        self.assertEqual(json.loads((self.assigned/'valid/input.original.json').read_text())['operations'][0]['participants'],[external])

    def test_duplicate_additional_actor_is_blocked_without_removing_external_declaration(self):
        participant={'id':'provider','label':'Provider','externalService':'gateway'}
        value=proposal([participant,copy.deepcopy(participant)]);self.write('duplicate.json',value)
        r=self.run_session(input_name='duplicate.json')
        self.assertEqual(r['nativeCallsAttempted'],0)
        self.assertEqual(r['participantIssues'][0]['reason'],'DUPLICATE_ADDITIONAL_PARTICIPANT')
        self.assertEqual(json.loads((self.assigned/'attempt-1/input.original.json').read_text()),value)
        self.assertFalse(self.calls.exists())

    def test_same_work_resubmit_preserves_full_inputs_unrelated_branches_and_raw_feedback(self):
        before=(self.assigned/'draft.json').read_bytes();r=self.run_session()
        self.assertEqual((r['status'],r['nativeStatus']),('NATIVE_RESULT_RECORDED','NEEDS_REPAIR'))
        first_raw=(self.assigned/'attempt-1/submit.stdout').read_bytes()
        corrected=proposal();corrected['operations'][0]['title']='Caller corrected native field';self.write('draft-corrected.json',corrected)
        self.responses.update(submit=proposal_page(),show=proposal_page());self.driver()
        r=self.run_session(input_name='draft-corrected.json',output='attempt-2')
        self.assertEqual(r['nativeStatus'],'READY_WITH_LIMITATIONS');self.assertEqual(r['work'],'work-1')
        self.assertEqual((self.assigned/'attempt-1/submit.stdout').read_bytes(),first_raw)
        self.assertEqual((self.assigned/'draft.json').read_bytes(),before)
        self.assertEqual((self.assigned/'attempt-2/input.original.json').read_bytes(),(self.assigned/'draft-corrected.json').read_bytes())
        archived=json.loads((self.assigned/'attempt-2/input.original.json').read_text())
        self.assertEqual(archived['operations'][1],proposal()['operations'][1]);self.assertEqual(archived['operations'][0]['steps'],proposal()['operations'][0]['steps'])
        calls=[json.loads(line) for line in self.calls.read_text().splitlines()]
        self.assertTrue(all(args[args.index('--work')+1]=='work-1' for args in calls if '--work' in args))
        self.assertFalse(any('prepare' in args or 'publish' in args for args in calls))

    def test_raw_native_failure_preserved_without_new_work_or_retry(self):
        self.responses['submit']={'processExit':2,'raw':'native failure bytes','stderr':'owned error'};self.driver()
        r=self.run_session();self.assertEqual(r['status'],'BLOCKED');self.assertEqual(r['nativeCallsAttempted'],1)
        self.assertEqual((self.assigned/'attempt-1/submit.stdout').read_text(),'native failure bytes')
        self.assertEqual((self.assigned/'attempt-1/submit.stderr').read_text(),'owned error')
        self.assertIn('SAME --work',r['nextAction'])

    def test_cursor_tail_omissions_and_wrong_identity_stay_explicit(self):
        self.write('tail.json',{'query':{'kind':'SYMBOL','symbolContains':'OwnedService'},'cursor':'page-3'})
        row={'kind':'SOURCE','id':'body','reference':'s1','record':{'id':'body','text':'owned'}}
        self.responses['read']={'page-3':work_page([row],total=17)};self.driver()
        r=self.run_session('read',input_name='tail.json',output='tail')
        self.assertEqual(r['status'],'TAIL_RECORDED');self.assertFalse(r['selectionComplete'])
        self.responses['read']={'FIRST':work_page([],omitted=[{'kind':'SOURCE','id':'large','reference':'s2'}],total=1)};self.driver()
        r=self.run_session('read',input_name='selection.json',output='omitted');self.assertEqual(r['status'],'INCOMPLETE')
        self.responses['read']={'FIRST':work_page(work='foreign')};self.driver()
        r=self.run_session('read',input_name='selection.json',output='wrong');self.assertEqual(r['status'],'INCOMPLETE')
        self.assertTrue((self.assigned/'wrong/read-collection/page-0001.stdout').is_file())

    def test_invalid_json_and_selection_fail_before_native_calls(self):
        (self.assigned/'invalid.json').write_text('{"x":1,"x":2}')
        r=self.run_session('read',input_name='invalid.json');self.assertIn('DUPLICATE_JSON_KEY',r['reason'])
        self.write('bad-selection.json',{'references':['s1']*9});r=self.run_session('read',input_name='bad-selection.json',output='bad-selection')
        self.assertEqual(r['nativeCallsAttempted'],0);self.assertFalse(self.calls.exists())

    def test_discovery_defaults_to_one_page_and_explicit_budget_keeps_exact_continuation(self):
        selection={'query':{'kind':'*','symbolContains':''}}
        self.write('broad.json',selection)
        self.responses['read']={}
        for i in range(4):
            row={'kind':'DEPENDENCY','id':f'symbol-{i}','reference':f'd{i}','record':{'kind':'SYMBOL'}}
            self.responses['read']['FIRST' if i==0 else f'cursor-{i}']=work_page(
                [row],cursor=f'cursor-{i+1}' if i<3 else None,total=4)
        self.driver()
        command=[sys.executable,'-I','-S',str(ROOT/'skills/codeclew/scripts/work_session.py'),'read',
            '--launcher',str(self.launcher),'--root',str(self.docs),'--input-root',str(self.assigned),
            '--work','work-1','--input','broad.json','--output','first-probe']
        process=subprocess.run(command,cwd=self.base,capture_output=True,text=True)
        self.assertEqual(process.returncode,2,process.stderr)
        result=json.loads(process.stdout)
        self.assertEqual((result['status'],result['nativeCallsAttempted']),('INCOMPLETE',1))
        self.assertFalse(result['selectionComplete'])
        manifest=json.loads((self.assigned/'first-probe/read-collection/manifest.json').read_text())
        self.assertEqual(manifest['reason'],'HELPER_PAGE_LIMIT_BEFORE_TERMINAL')
        self.assertFalse(manifest['terminalCursorObserved'])
        self.assertEqual(manifest['continuationSelection'],{**selection,'cursor':'cursor-1'})
        self.assertEqual((self.assigned/'broad.json').read_text(),json.dumps(selection))
        # The programmatic default also probes one page, without implicit retries.
        probe=self.run_session('read',input_name='broad.json',output='api-probe')
        self.assertEqual((probe['status'],probe['nativeCallsAttempted']),('INCOMPLETE',1))
        limited=self.run_session('read',input_name='broad.json',output='two-pages',max_pages=2)
        self.assertEqual((limited['status'],limited['nativeCallsAttempted']),('INCOMPLETE',2))
        second=json.loads((self.assigned/'two-pages/read-collection/manifest.json').read_text())
        self.assertEqual(second['continuationSelection'],{**selection,'cursor':'cursor-2'})
        override=subprocess.run(command[:-1]+['explicit-four','--max-pages','4'],cwd=self.base,capture_output=True,text=True)
        self.assertEqual(override.returncode,0,override.stderr)
        complete=json.loads(override.stdout)
        self.assertEqual((complete['status'],complete['nativeCallsAttempted']),('COMPLETE',4))
        self.assertTrue(complete['selectionComplete']);self.assertIsNone(complete['nextCursor'])
        self.write('resume.json',manifest['continuationSelection'])
        resumed=self.run_session('read',input_name='resume.json',output='resume',max_pages=1)
        self.assertEqual((resumed['status'],resumed['nativeCallsAttempted']),('INCOMPLETE',1))
        resume_manifest=json.loads((self.assigned/'resume/read-collection/manifest.json').read_text())
        self.assertEqual(resume_manifest['continuationSelection'],{**selection,'cursor':'cursor-2'})
        calls=[json.loads(row) for row in self.calls.read_text().splitlines()]
        self.assertEqual(len(calls),9)
        self.assertTrue(all(args[:3]==['docs','work','read'] and args[args.index('--work')+1]=='work-1' for args in calls))

    def test_exact_references_initial_and_submit_keep_complete_default_collection(self):
        rows=[{'kind':'SOURCE','id':f'source-{i}','reference':f's{i}','record':{'text':'owned source'}} for i in (1,2)]
        self.responses['read']={'FIRST':work_page(rows[:1],cursor='next',total=2),'next':work_page(rows[1:],total=2)}
        self.driver()
        self.write('exact.json',{'references':['s1','s2']})
        exact=self.run_session('read',input_name='exact.json',output='exact')
        self.assertEqual((exact['status'],exact['nativeCallsAttempted']),('COMPLETE',2))
        self.write('initial.json',{})
        initial=self.run_session('read',input_name='initial.json',output='initial')
        self.assertEqual((initial['status'],initial['nativeCallsAttempted']),('COMPLETE',2))
        required=S['T']['collect'](self.launcher,self.docs,'work-1',self.assigned/'required-initial',kind='work-initial')
        self.assertEqual((required['status'],required['pages']),('COMPLETE',2))
        first=proposal_page('NEEDS_REPAIR');last=proposal_page('NEEDS_REPAIR')
        first.update(total=2,nextCursor='feedback-next',items=[{'kind':'DIAGNOSTIC','id':'one','record':{'code':'FIRST'}}])
        last.update(total=2,items=[{'kind':'DIAGNOSTIC','id':'two','record':{'code':'SECOND'}}])
        self.responses.update(submit=first,show={'FIRST':first,'feedback-next':last});self.driver()
        submitted=self.run_session(output='submit-feedback')
        self.assertEqual((submitted['status'],submitted['nativeStatus'],submitted['nativeCallsAttempted']),('NATIVE_RESULT_RECORDED','NEEDS_REPAIR',3))
        feedback=json.loads((self.assigned/'submit-feedback/submit-collection/manifest.json').read_text())
        self.assertEqual((feedback['status'],feedback['pages'],feedback['diagnosticRows']),('COMPLETE',2,2))

    def test_scenario_uses_existing_native_submit_collector_route(self):
        self.write('work/page.stdout',work_page(subject='scenario:owned-lineage'))
        scenario=proposal();scenario['operations']=[];self.write('scenario.json',scenario)
        preflight=self.run_session('preflight',input_name='scenario.json',output='scenario-preflight')
        self.assertEqual(preflight['status'],'BLOCKED');self.assertEqual(preflight['nativeCallsAttempted'],0)
        self.assertIn('PARTICIPANT_PREFLIGHT_REQUIRES_SERVICE_WORK',preflight['reason'])
        output=self.assigned/'scenario-native';output.mkdir()
        recorded=S['T']['submit_and_collect'](self.launcher,self.docs,'work-1',self.assigned/'scenario.json',output,max_pages=2)
        self.assertEqual(recorded['status'],'COMPLETE');self.assertEqual(recorded['nativeStatus'],'NEEDS_REPAIR')
        self.assertEqual((self.assigned/'scenario.json').read_text(),json.dumps(scenario))



if __name__=='__main__':unittest.main()
