#!/usr/bin/env python3
"""Protocol tests for automatic scoped discovery and explicitly selected batch reads."""
from __future__ import annotations
import argparse
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
M = runpy.run_path(str(ROOT / "skills/codeclew/scripts/evidence_packet.py"))
Packet, digest, encode = (M[k] for k in ("Packet", "digest", "encode"))
F = runpy.run_path(str(ROOT / "scripts/test_work_bound_source.py"))
SNAPSHOT = F["SNAPSHOT"]


def page(*items, cursor=None, omitted=None, total=None):
    return {**F["page"](*items, cursor=cursor, omitted=omitted), "total": len(items) if total is None else total,
            "membershipDigest": "sha256:membership", "contextDigest": "sha256:context", "inputDigest": "sha256:input",
            "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED"}


def coverage():
    return {"schema": "codeclew-docs-page/1.0", "subject": "service:owned", "snapshot": SNAPSHOT,
            "items": [{"kind": "COVERAGE", "id": "owned", "revision": "rev-a", "coverage": "SYNTAX"}],
            "omitted": [], "nextCursor": "unused-entrypoints", "authority": "PINNED_SNAPSHOT_NOT_REVERIFIED"}


def dependency_part(record, start=0, end=None, cursor=None):
    text = encode(record)[:-1].encode(); end = len(text) if end is None else end
    value = {"schema": "codeclew-documentation-dependency-part/1.0", "work": "work-a", "snapshot": SNAPSHOT,
        "kind": "DEPENDENCY", "id": record["id"], "recordDigest": digest(record), "startByte": start, "endByte": end,
        "totalRecordBytes": len(text), "text": text[start:end].decode(), "fragmentDigest":
        "sha256:" + __import__('hashlib').sha256(text[start:end]).hexdigest(), "nextCursor": cursor,
        "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED", "receiptType": "DEPENDENCY_PART"}
    return {**value, "receiptDigest": digest(value)}


def navigation(record, references):
    return {"kind": "CALLABLE_SUMMARY", "id": record["id"], "record": {"service": "owned", "symbol": record["symbol"],
        "sourceReferences": references, "provenance": {"dependencyId": record["id"], "observationDigest": record["digest"]}}}


class EvidencePacketTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.out = self.root / "out"; self.out.mkdir()
        self.args = argparse.Namespace(action="discover", clew="/supported/clew", root="/disposable/docs", service="owned",
            snapshot=SNAPSHOT, work="work-a", prepare_work=False, contains="", page_items=1,
            output_dir=self.out, packet=None, selection=None)
        self.commands, self.inputs = [], []

    def execute(self, packets):
        self.commands, self.inputs = [], []
        sequence = iter(packets)
        def native(command, **kwargs):
            self.commands.append(command)
            self.assertEqual(command[0], self.args.clew)
            self.assertNotIn('--refresh',command); self.assertNotIn('--symbol',command)
            self.assertIn(command[1:4], [['docs','context','--root'],['docs','work','read'],
                ['docs','work','prepare'],['docs','work','read-part'],['docs','work','read-dependency-part']])
            if '--input' in command:self.inputs.append(json.loads(Path(command[command.index('--input')+1]).read_text()))
            value=next(sequence)
            return value if isinstance(value,subprocess.CompletedProcess) else subprocess.CompletedProcess(command,0,json.dumps(value,ensure_ascii=False).encode(),b'diagnostic\xff\n')
        with patch('subprocess.run',side_effect=native): value,code=Packet(self.args).run()
        packet=json.loads((self.out/'packet.json').read_text());summary=json.loads((self.out/'read-summary.json').read_text())
        self.assertFalse(packet['sourceFallbackAllowed']);self.assertFalse(packet['completeDeclarationClaimed'])
        self.assertFalse(packet['wholeProcessOrRuntimeCompleteClaimed'])
        self.assertEqual(summary['observedNativeCommands'],len(self.commands))
        self.assertFalse(summary['agentTranscriptCountsAttested'])
        self.assertEqual(summary['packetDigest'],digest(packet))
        return value,code,packet,summary

    def read_input(self, rows, selections):
        self.args.action='read';self.args.packet=self.root/'discover.json';self.args.selection=self.root/'selection.json'
        self.args.packet.write_text(encode({'schema':M['SCHEMA'],'kind':'DISCOVERY','scope':{'service':'owned','snapshot':SNAPSHOT},
            'work':'work-a','coverage':{'record':{'revision':'rev-a'}},'choices':rows,'queryComplete':True}))
        self.args.selection.write_text(json.dumps(selections))

    def row(self,item):
        return {'kind':'DECLARATION','id':item['id'],'reference':item['reference'],'symbol':item['record']['symbol'],
            'service':'owned','revision':'rev-a','sourceIds':item['record']['sourceIds'],'observationDigest':item['record']['digest']}

    def test_batch_read_selected_and_reuse_keep_each_declaration_binding(self):
        sources=[];declarations=[];rows=[]
        for index in range(2):
            source=F['source']('if (ready) send(); // π🙂\n')
            source.update(id='source-'+str(index),evidenceDigest='sha256:original');sources.append(source)
            item=F['declaration']([source['id']],['s'+str(index)])
            item['id']=item['record']['id']='decl-'+str(index);item['reference']='d'+str(index);declarations.append(item)
            rows.append({**self.row(item),'sourceReferences':item['sourceReferences']})
        shared=json.loads(json.dumps(declarations[0]));shared['id']=shared['record']['id']='shared-declaration'
        shared['reference']='d-shared';declarations.append(shared)
        rows.append({**self.row(shared),'sourceReferences':shared['sourceReferences']})
        self.read_input(rows,[{'declarationId':r['id']} for r in rows]);self.args.source_batch_size=8
        self.args.output_format='author'
        batch=page({'kind':'COVERAGE','id':'owned','revision':'rev-a'},
                   *[{'kind':'SOURCE','id':s['id'],'reference':'s'+str(i),'record':s} for i,s in enumerate(sources)])
        batch['receiptDigest']=digest(batch)
        _,code,packet,summary=self.execute([batch,*[F['page'](d) for d in declarations]])
        self.assertEqual(code,0);self.assertEqual(len(self.commands),4)
        self.assertFalse(any(c[3]=='read-part' for c in self.commands))
        self.assertEqual(summary['verifiedSourceReadsReused'],1)
        A=runpy.run_path(str(ROOT/'skills/codeclew/scripts/author_citations.py'));A['load'](self.out/'citations.json')
        prior=self.out/'packet.json';self.args.output_dir=self.root/'reuse';self.args.output_dir.mkdir();self.out=self.args.output_dir
        self.args.reuse_read=prior
        _,code,reused,summary=self.execute([]);self.assertEqual(code,0);self.assertEqual(self.commands,[])
        self.assertEqual(packet['texts'],reused['texts']);A['load'](self.out/'citations.json')
        fresh_source=F['source']('throw error; // π🙂\n')
        fresh_source.update(id='fresh-source',evidenceDigest='sha256:original')
        fresh=F['declaration'](['fresh-source'],['s-fresh']);fresh['id']=fresh['record']['id']='fresh-declaration'
        fresh['reference']='d-fresh';rows.append({**self.row(fresh),'sourceReferences':fresh['sourceReferences']})
        self.read_input(rows,[{'declarationId':r['id']} for r in rows]);self.args.reuse_read=prior
        self.args.output_dir=self.root/'mixed';self.args.output_dir.mkdir();self.out=self.args.output_dir
        batch=page({'kind':'COVERAGE','id':'owned','revision':'rev-a'},
                   {'kind':'SOURCE','id':'fresh-source','reference':'s-fresh','record':fresh_source})
        batch['receiptDigest']=digest(batch)
        _,code,_,summary=self.execute([batch,F['page'](fresh)])
        self.assertEqual(code,0);self.assertEqual(summary['priorEvidenceBindingsReused'],3)
        self.assertEqual(self.inputs[0]['references'],['s-fresh']);self.assertEqual(len(self.commands),2)
        A['load'](self.out/'citations.json')

    def test_discovery_drains_pages_and_recovers_oversized_record_without_source_reads(self):
        first=F['declaration'](); first['id']=first['record']['id']='decl-a';first['reference']='d8'
        second=F['declaration']();second['record']['normalized']['largeDescriptor']='x'*70000
        record=second['record'];cut=40000
        replies=[coverage(),page(first,cursor='next',total=2),page(total=2,omitted=[{'kind':'DEPENDENCY','id':'decl-b','reference':'d9','reason':'ITEM_EXCEEDS_WORK_BYTE_BUDGET'}]),
                 dependency_part(record,end=cut,cursor='part-next'),dependency_part(record,start=cut),page(navigation(record,['s9']))]
        _,code,packet,summary=self.execute(replies)
        self.assertEqual(code,0);self.assertTrue(packet['queryComplete']);self.assertEqual(len(packet['choices']),2)
        self.assertEqual(summary['recoveredOmissions'],1)
        self.assertFalse(packet['sourceBodiesRequested']);self.assertFalse(any(c[3:4]==['read-part'] for c in self.commands))
        self.assertEqual(packet['choices'][1]['sourceIds'],['source-b'])

    def test_source_validation_reused_only_within_same_sealed_work_binding(self):
        a=F['declaration']();b=F['declaration']();b['id']=b['record']['id']='decl-c';b['reference']='d10'
        self.read_input([self.row(a),self.row(b)],[{'declarationId':'decl-b'},{'declarationId':'decl-c'}])
        _,code,packet,summary=self.execute([page(a),F['part'](),page(b)])
        self.assertEqual(code,0);self.assertEqual(len(packet['texts']),1)
        self.assertEqual(summary['verifiedSourceReadsReused'],1);self.assertEqual(summary['observedNativeCommands'],3)
        self.assertEqual(len(packet['outcomes']),2)

    def test_equal_text_different_source_ids_keep_both_provenances_and_full_validation(self):
        a=F['declaration']();b=F['declaration'](['source-c'],['s10']);b['id']=b['record']['id']='decl-c';b['reference']='d10'
        source=F['source']();source['id']='source-c'
        self.read_input([self.row(a),self.row(b)],[{'declarationId':'decl-b'},{'declarationId':'decl-c'}])
        _,code,packet,summary=self.execute([page(a),F['part'](),page(b),F['part'](source,reference='s10')])
        self.assertEqual(code,0);self.assertEqual(len(packet['texts']),1)
        self.assertEqual({o['source']['id'] for o in packet['outcomes']},{'source-b','source-c'})
        self.assertEqual(summary['verifiedSourceRecords'],2);self.assertEqual(summary['verifiedSourceReadsReused'],0)

    def test_genuine_multiple_bindings_stay_explicit(self):
        item=F['declaration'](['one','two'],['s1','s2']);self.read_input([self.row(item)],[{'declarationId':'decl-b'}])
        _,code,packet,_=self.execute([page(item)])
        self.assertEqual(code,3);self.assertEqual(packet['outcomes'][0]['status'],'EXPLICIT_SOURCE_CHOICE_REQUIRED')
        self.assertEqual(packet['texts'],{})

    def test_batch_recovers_selected_oversized_declaration_before_source_parts(self):
        item=F['declaration']();self.read_input([self.row(item)],[{'declarationId':'decl-b'}])
        _,code,packet,_=self.execute([page(omitted=[{'kind':'DEPENDENCY','id':'decl-b','reference':'d9'}]),
            dependency_part(item['record']),page(navigation(item['record'],['s9'])),F['part']()])
        self.assertEqual(code,0);self.assertEqual(packet['outcomes'][0]['source']['id'],'source-b')

    def test_nonprogressing_cursor_and_changed_membership_are_errors(self):
        _,code,packet,_=self.execute([coverage(),page(cursor='same',total=0),page(cursor='same',total=0)])
        self.assertEqual(code,2);self.assertFalse(packet['queryComplete'])
        changed=page(total=2);changed['membershipDigest']='different'
        _,code,packet,_=self.execute([coverage(),page(cursor='next',total=2),changed])
        self.assertEqual(code,2);self.assertEqual(packet['status'],'NATIVE_QUERY_MEMBERSHIP_CHANGED')

    def test_unrecovered_omission_does_not_complete_query(self):
        _,code,packet,summary=self.execute([coverage(),page(omitted=[{'kind':'UNKNOWN','id':'gap'}],total=1)])
        self.assertEqual(code,3);self.assertFalse(packet['queryComplete']);self.assertEqual(summary['unresolvedOmissions'],1)

    def test_changed_dependency_part_digest_keeps_scope_incomplete(self):
        item=F['declaration']();bad=dependency_part(item['record']);bad['fragmentDigest']='bad'
        _,code,packet,_=self.execute([coverage(),page(omitted=[{'kind':'DEPENDENCY','id':'decl-b','reference':'d9'}],total=1),bad])
        self.assertEqual(code,3);self.assertFalse(packet['queryComplete'])
        self.assertEqual(packet['omissions'][0]['failure']['status'],'DEPENDENCY_PART_RANGE_OR_DIGEST_MISMATCH')

    def test_unknown_choice_and_missing_source_are_separate_batch_outcomes(self):
        item=F['declaration']([],[]);self.read_input([self.row(item)],[{'declarationId':'unknown'},{'declarationId':'decl-b'}])
        _,code,packet,_=self.execute([page(item)])
        self.assertEqual(code,3);self.assertEqual([o['status'] for o in packet['outcomes']],['DECLARATION_NOT_IN_PACKET','MISSING_DECLARATION_SOURCE'])

    def test_batch_scope_mismatch_stops_before_native_read(self):
        self.read_input([], [{'declarationId':'unknown'}]);packet=json.loads(self.args.packet.read_text())
        packet['scope']['snapshot']='other';self.args.packet.write_text(encode(packet))
        _,code,packet,_=self.execute([]);self.assertEqual(code,2);self.assertEqual(self.commands,[])

    def test_native_errors_and_complete_streams_remain_on_disk(self):
        stdout=b'{"error":{"code":"STATE_CORRUPT","message":"native failure"}}\n';stderr=b'error\xff\n'*20000
        _,code,packet,summary=self.execute([subprocess.CompletedProcess([],7,stdout,stderr)])
        self.assertEqual(code,7);self.assertFalse(packet['queryComplete'])
        self.assertEqual(summary['nativeStderrBytes'],len(stderr))
        receipt=json.loads((self.out/'receipts.json').read_text())[0]
        self.assertEqual((self.out/receipt['stdout']).read_bytes(),stdout)
        self.assertEqual((self.out/receipt['stderr']).read_bytes(),stderr)


if __name__=='__main__':unittest.main()
