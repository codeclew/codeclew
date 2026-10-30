#!/usr/bin/env python3
"""Owned saved-record review fixtures; no compiler, native acquisition or models."""
import argparse
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT/'scripts/test_computation_view.py'))
M = F['M']


def fixture(text=None):
    item = F['fixture'](); source = item['source']; doc = item['record']['normalized']['documentation']
    if text is not None:
        source.update(text=text, textDigest='sha256:'+hashlib.sha256(text.encode()).hexdigest(), endLine=2+text.count('\n'))
        source['occurrence']['endByte'] = 9+len(text.encode())
        root = copy.deepcopy(doc['sourceStructure']['nodes'][0])
        root['sourceSpan'].update(byteEnd=9+len(text.encode()), ownerByteEnd=9+len(text.encode()))
        doc['sourceStructure']['nodes'] = [root]
    doc['events'] = []
    span = doc['sourceStructure']['nodes'][0]['sourceSpan']
    doc['semanticInputs'] = {**span, 'calls': [], 'returns': [], 'variables': []}
    item['record'].update(service='example', sourceIds=['source-example'])
    item['source'].update(service='example', id='source-example')
    return item


def add(item, kind, parent, role, text, after=0):
    doc = item['record']['normalized']['documentation']; nodes = doc['sourceStructure']['nodes']
    raw = item['source']['text'].encode(); literal = text.encode(); at = raw.index(literal, after)
    node = {'id': len(nodes), 'parent': parent, 'role': role, 'kind': kind, 'origin': 'SOURCE_DECLARED',
            'sourceSpan': {**nodes[0]['sourceSpan'], 'byteStart': 9+at, 'byteEnd': 9+at+len(literal)}}
    nodes.append(node); return node


def saved(item):
    return {'schema':'codeclew-connected-computation/1.0','work':'work','snapshot':'snapshot','service':'example','callables':[item]}


class StructuralReviewTest(unittest.TestCase):
    def review(self, item, **kwargs): return M['review_project'](saved(item), **kwargs)

    def test_same_line_distinct_branches_exact_utf8_and_no_callable_body_repeat(self):
        item = fixture(); nodes = item['record']['normalized']['documentation']['sourceStructure']['nodes']
        then = add(item,'KtCallExpression',1,'THEN','send("π🙂")')
        other = add(item,'KtCallExpression',1,'ELSE','error("bad")')
        before = copy.deepcopy(item); text, meta = self.review(item)
        self.assertIn('then A.n3[3]; else A.n4[3]', text)
        self.assertNotIn(item['source']['text'], text); self.assertEqual(item,before)
        row = next(r for r in meta['rows'] if r['id']=='A.n1')
        self.assertEqual(row['sourceSpan'], nodes[1]['sourceSpan'])
        self.assertTrue(any(p['pointer'].endswith('/nodes/3') for p in row['evidence']))
        self.assertNotEqual(then['sourceSpan']['byteStart'],other['sourceSpan']['byteStart'])

    def test_nested_conditions_loop_try_catch_finally_and_deferred_context(self):
        inside='if (other) return@run'; callback='{ '+inside+' }'; outer='if (flag) run '+callback+' else fail()'
        loop='for (v in values) { '+outer+' }'; attempt='try { '+loop+' } catch (e: E) { fail() } finally { done() }'
        item=fixture('fun example() { '+attempt+' }\n')
        tr=add(item,'KtTryExpression',0,'CHILD',attempt)
        tb=add(item,'KtBlockExpression',tr['id'],'TRY_BODY','{ '+loop+' }')
        lo=add(item,'KtForExpression',tb['id'],'CHILD',loop)
        add(item,'KtNameReferenceExpression',lo['id'],'LOOP_RANGE','values')
        lb=add(item,'KtBlockExpression',lo['id'],'LOOP_BODY','{ '+outer+' }')
        branch=add(item,'KtIfExpression',lb['id'],'CHILD',outer)
        add(item,'KtNameReferenceExpression',branch['id'],'CONDITION','flag')
        call=add(item,'KtCallExpression',branch['id'],'THEN','run '+callback)
        lam=add(item,'KtLambdaExpression',call['id'],'CHILD',callback)
        body=add(item,'KtBlockExpression',lam['id'],'CALLBACK_BODY',callback)
        inner=add(item,'KtIfExpression',body['id'],'CHILD',inside)
        add(item,'KtNameReferenceExpression',inner['id'],'CONDITION','other')
        ret=add(item,'KtReturnExpression',inner['id'],'THEN','return@run')
        add(item,'KtCallExpression',branch['id'],'ELSE','fail()')
        caught=add(item,'KtCatchClause',tr['id'],'CATCH','catch (e: E) { fail() }')
        add(item,'KtParameter',caught['id'],'CATCH_PARAMETER','e: E')
        add(item,'KtBlockExpression',caught['id'],'CATCH_BODY','{ fail() }')
        add(item,'KtBlockExpression',tr['id'],'FINALLY_BODY','{ done() }')
        sem=item['record']['normalized']['documentation']['semanticInputs']
        sem['returns']=[{'byteStart':ret['sourceSpan']['byteStart'],'byteEnd':ret['sourceSpan']['byteEnd'],
            'resolution':'COMPILER_EXACT','targetRegion':{'kind':'ANONYMOUS_FUNCTION',**lam['sourceSpan']}}]
        text,meta=self.review(item)
        returned=next(r for r in meta['rows'] if r['id']=='A.n'+str(ret['id']))
        self.assertTrue(returned['text'].endswith('return => A.n'+str(lam['id'])))
        inner_row=next(r for r in meta['rows'] if r['id']=='A.n'+str(inner['id']))
        self.assertIn('in A.n'+str(lam['id']),inner_row['text'])
        for marker in ['for values','catch e: E','finally A.n','callback body']: self.assertIn(marker,text)
        self.assertFalse(meta['semanticCoverageAssessed'])

    def test_same_line_two_callbacks_resolve_exact_region_not_line(self):
        item=fixture('fun example() { val first = { 1 }; run { return@run } }\n')
        first=add(item,'KtLambdaExpression',0,'CHILD','{ 1 }')
        second=add(item,'KtLambdaExpression',0,'CHILD','{ return@run }')
        ret=add(item,'KtReturnExpression',second['id'],'CALLBACK_BODY','return@run')
        sem=item['record']['normalized']['documentation']['semanticInputs']
        sem['returns']=[{'byteStart':ret['sourceSpan']['byteStart'],'byteEnd':ret['sourceSpan']['byteEnd'],
            'resolution':'COMPILER_EXACT','targetRegion':{'kind':'ANONYMOUS_FUNCTION',**second['sourceSpan']}}]
        _,meta=self.review(item); row=next(r for r in meta['rows'] if r['id']=='A.n3')
        self.assertTrue(row['text'].endswith('return => A.n2'));self.assertNotEqual(first['id'],second['id'])

    def test_lexical_identity_and_reassignment_are_not_reaching_value_proof(self):
        item=fixture('fun example() { var x = fetch(); x = other(); if (x != null) send(x) }\n')
        branch=add(item,'KtIfExpression',0,'CHILD','if (x != null) send(x)')
        cond=add(item,'KtBinaryExpression',branch['id'],'CONDITION','x != null')
        add(item,'KtCallExpression',branch['id'],'THEN','send(x)')
        raw=item['source']['text'].encode(); call_at=9+raw.index(b'fetch()'); write_at=9+raw.index(b'x = other')
        sem=item['record']['normalized']['documentation']['semanticInputs']
        sem['calls']=[{'byteStart':call_at,'byteEnd':call_at+7,'resolution':'COMPILER_EXACT','targetIdentity':'callable:api.fetch'}]
        sem['variables']=[{'kind':'VARIABLE_DECLARATION','resolution':'COMPILER_EXACT','variableIdentity':'x-one','name':'x',
            'initializerSpan':{'byteStart':call_at,'byteEnd':call_at+7}},
            {'kind':'VARIABLE_ACCESS','resolution':'COMPILER_EXACT','variableIdentity':'x-one','name':'x','accessMode':'WRITE','byteStart':write_at,'byteEnd':write_at+1},
            {'kind':'VARIABLE_ACCESS','resolution':'COMPILER_EXACT','variableIdentity':'x-one','name':'x','accessMode':'READ','byteStart':cond['sourceSpan']['byteStart'],'byteEnd':cond['sourceSpan']['byteStart']+1}]
        text,meta=self.review(item);link=next(r for r in meta['rows'] if r['kind']=='lexical')
        self.assertEqual(len(link['writes']),1);self.assertEqual(link['linkAuthority'],'SAME_COMPILER_VARIABLE_NOT_REACHING_VALUE')
        self.assertIn('writes retained',text)
        sem['variables'][-1]['variableIdentity']='different-x'
        _,meta=self.review(item);self.assertFalse(any(r['kind']=='lexical' for r in meta['rows']))

    def test_no_else_missing_semantics_scan_gaps_and_unknown_exit_remain_explicit(self):
        item=fixture('fun example() { if (flag) return }\n');branch=add(item,'KtIfExpression',0,'CHILD','if (flag) return')
        add(item,'KtNameReferenceExpression',branch['id'],'CONDITION','flag');add(item,'KtReturnExpression',branch['id'],'THEN','return')
        d=item['record']['normalized']['documentation'];del d['semanticInputs']
        d['sourceStructure'].update(complete=False,omittedScans=1,gaps=[{'reason':'TRAVERSAL_BUDGET'}])
        text,meta=self.review(item)
        for phrase in ['else not recorded','return => unestablished','compiler call/variable/return metadata missing','structure scan gap']:self.assertIn(phrase,text)
        self.assertTrue(meta['gapEvidence']);del d['sourceStructure']
        with self.assertRaises(M['Stop']):self.review(item)

    def test_synthetic_gate_is_not_an_extra_business_branch_and_semantic_scope_mismatch_refuses(self):
        item=fixture();d=item['record']['normalized']['documentation'];d['sourceStructure']['nodes'][2]['origin']='SYNTHETIC_SHORT_CIRCUIT_GATE'
        wrong=saved(copy.deepcopy(item));wrong['service']='other'
        with self.assertRaises(M['Stop']):M['review_project'](wrong)
        _,meta=self.review(item);self.assertEqual(meta['displayedNodeKinds']['KtIfExpression'],1)
        self.assertNotIn('KtNameReferenceExpression',meta['displayedNodeKinds'])
        self.assertEqual(d['sourceStructure']['nodes'][2]['origin'],'SYNTHETIC_SHORT_CIRCUIT_GATE')
        d['semanticInputs']['fullCompilationSourceDigest']='foreign'
        with self.assertRaises(M['Stop']):self.review(item)

    def page(self,item):
        return {'schema':'codeclew-documentation-work-page/1.0','work':'work','snapshot':'snapshot','subject':'service:example','items':[
            {'record':{'rootDeclarations':[item['record']['id']],'sourceIds':[item['source']['id']],
             'entrypoint':{'sourceIds':[item['source']['id']],'trigger':{'authority':'OWNED_RULE','configuration':{'mode':'on'},'timeUnit':'SECONDS'}}}}]}

    def test_annotation_trigger_consolidation_retains_both_origins_unique_facts_and_rejects_mismatch(self):
        item=fixture();item['record']['normalized']['jvmAnnotations']={'declaration':'callable:example','callables':[{'method':{
            'identity':'callable:example','annotations':[{'typeName':'sample.Mode','origin':{'kind':'SOURCE'},'arguments':{'mode':{'kind':'CONSTANT','value':'on'}}}]}}]}
        page=self.page(item);text,meta=self.review(item,entrypoints=[('page',page)])
        config=next(r for r in meta['rows'] if r['kind']=='configuration')
        self.assertEqual(config['text'].count('"on"'),1);self.assertIn('SECONDS',config['text'])
        self.assertEqual(len(config['contributors']),2);self.assertEqual({p['input'] for p in config['evidence']},{'metadata','page'})
        for key,value in [('work','other'),('snapshot','other'),('subject','service:other')]:
            wrong=copy.deepcopy(page);wrong[key]=value
            with self.subTest(key=key),self.assertRaises(M['Stop']):self.review(item,entrypoints=[('page',wrong)])
        wrong=copy.deepcopy(page);wrong['items'][0]['record']['entrypoint']['symbol']='foreign-symbol'
        with self.assertRaises(M['Stop']):self.review(item,entrypoints=[('page',wrong)])
        page['items'][0]['record']['sourceIds']=['foreign']
        with self.assertRaises(M['Stop']):self.review(item,entrypoints=[('page',page)])

    def test_budget_stays_open_when_target_body_is_selected(self):
        item=fixture();page=self.page(item);page.update(total=1,membershipDigest='membership',contextDigest='context',nextCursor=None)
        page['items']=[{'id':'gap','record':{'frontier':{'kind':'BUDGET','code':'EVENT_BUDGET','methodId':item['record']['id'],'semanticClosure':'UNKNOWN'}}}]
        text,meta=self.review(item,contexts=[('context',page,item['record']['id'])])
        self.assertIn('EVENT_BUDGET at A; OPEN',text);self.assertEqual(meta['contextRoots'],[item['record']['id']])
        self.assertTrue(all(r['evidence'] for r in meta['rows']))

    def test_when_and_while_body_relationships_and_else_native_pointer(self):
        expression='when (v) { 1 -> take() else -> skip() }';loop='while (flag) { tick() }'
        item=fixture('fun example() { '+expression+'; '+loop+' }\n')
        wh=add(item,'KtWhenExpression',0,'CHILD',expression)
        add(item,'KtNameReferenceExpression',wh['id'],'SUBJECT','v',after=item['source']['text'].encode().index(b'when'))
        arm=add(item,'KtWhenEntry',wh['id'],'WHEN_ENTRY','1 -> take()')
        add(item,'KtWhenConditionWithExpression',arm['id'],'CONDITION','1')
        add(item,'KtCallExpression',arm['id'],'WHEN_BODY','take()')
        other=add(item,'KtWhenEntry',wh['id'],'WHEN_ENTRY','else -> skip()')
        add(item,'KtCallExpression',other['id'],'WHEN_BODY','skip()')
        wl=add(item,'KtWhileExpression',0,'CHILD',loop)
        add(item,'KtNameReferenceExpression',wl['id'],'CONDITION','flag')
        add(item,'KtBlockExpression',wl['id'],'LOOP_BODY','{ tick() }')
        item['record']['normalized']['documentation']['events']=[{'kind':'ELSE','sourceStructureId':other['id']}]
        text,meta=self.review(item)
        for phrase in ['when v','case 1','case else (native event)','while flag']: self.assertIn(phrase,text)
        row=next(r for r in meta['rows'] if r['id']=='A.n'+str(other['id']))
        self.assertTrue(any(p['pointer'].endswith('/events/0') for p in row['evidence']))

    def test_implicit_result_event_and_missing_compiler_identity_are_not_invented_targets(self):
        item=fixture('fun example() = read()\n');d=item['record']['normalized']['documentation']
        d['events']=[{'kind':'RETURN','sourceStructureId':0}];d['semanticInputs'].pop('compilationScope')
        text,meta=self.review(item)
        self.assertIn('result expression A.n0',text);self.assertIn('compiler return target not retained',text)
        self.assertTrue(any('identity metadata incomplete' in gap for gap in meta['gaps']))

    def test_saved_context_receipt_scope_hash_and_root_are_checked_without_execution(self):
        item=fixture();meta=saved(item)
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);source=root/'metadata.json';source.write_text(json.dumps(meta))
            page=self.page(item);page.update(total=1,membershipDigest='m',contextDigest='c',nextCursor=None)
            page['items']=[{'id':'budget','record':{'frontier':{'kind':'BUDGET','code':'EVENT_BUDGET','methodId':item['record']['id']}}}]
            stdout=root/'page.json';stdout.write_text(json.dumps(page));request=root/'input.json'
            request.write_text(json.dumps({'query':{'kind':'THREAD_CONTEXT','symbolContains':item['record']['id']}}))
            receipt=root/'command.json';value={'command':['/supported/clew','docs','work','read','--work','work','--input',str(request)],
                'exitCode':0,'stdout':'page.json','stdoutSha256':hashlib.sha256(stdout.read_bytes()).hexdigest()}
            receipt.write_text(json.dumps(value))
            args=argparse.Namespace(input=source,entrypoints=[],context_receipt=[receipt],output_dir=root/'ok')
            with patch('subprocess.run',side_effect=AssertionError('No command execution')):text=M['review_saved'](args)
            self.assertIn('EVENT_BUDGET at A; OPEN',text)
            value['stdoutSha256']='0'*64;receipt.write_text(json.dumps(value));args.output_dir=root/'bad'
            with self.assertRaises(M['Stop']):M['review_saved'](args)
            self.assertFalse(args.output_dir.exists())

    def test_cli_saved_only_deterministic_sidecar_roundtrip_and_unchanged_compact(self):
        item=fixture();before=copy.deepcopy(item);base_compact=M['compact_project']([item])
        with tempfile.TemporaryDirectory() as name:
            root=Path(name);path=root/'metadata.json';raw=json.dumps(saved(item)).encode();path.write_bytes(raw)
            outputs=[]
            for i in range(2):
                out=root/str(i);capture=io.StringIO()
                with patch('subprocess.run',side_effect=AssertionError('No native calls')),contextlib.redirect_stdout(capture):
                    code=M['main'](['review-saved','--input',str(path),'--output-dir',str(out)])
                self.assertEqual(code,0);self.assertEqual(capture.getvalue().encode(),(out/'review.txt').read_bytes())
                side=json.loads((out/'review.json').read_bytes());self.assertEqual((out/'metadata.json').read_bytes(),raw)
                for row in side['rows']:
                    for point in row['evidence']:
                        value=json.loads((out/side['inputs'][point['input']]['copy']).read_bytes())
                        for part in point['pointer'].split('/')[1:]:value=value[int(part)] if isinstance(value,list) else value[part]
                        self.assertIsNotNone(value)
                outputs.append((out/'review.json').read_bytes())
            self.assertEqual(*outputs);self.assertEqual(path.read_bytes(),raw)
        self.assertEqual(item,before);self.assertEqual(M['compact_project']([item]),base_compact)


if __name__=='__main__':unittest.main()
