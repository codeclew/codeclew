#!/usr/bin/env python3
"""Same-input repair reuses only internally consistent earlier native evidence."""
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
F = runpy.run_path(str(ROOT / "scripts/test_work_bound_source.py"))
Packet, encode, digest = (M[k] for k in ("Packet", "encode", "digest"))


class ReusableEvidenceTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.commands = []
        self.discovery = self.root / "discovery.json"
        self.a = F['declaration'](); self.b = F['declaration'](['source-c'], ['s10'])
        self.b['id'] = self.b['record']['id'] = 'decl-c'; self.b['reference'] = 'd10'
        def row(item):
            return {'kind':'DECLARATION','id':item['id'],'reference':item['reference'],'symbol':item['record']['symbol'],
                'service':'owned','revision':'rev-a','sourceIds':item['record']['sourceIds'],'sourceReferences':item['sourceReferences'],
                'observationDigest':item['record']['digest']}
        self.discovery.write_text(encode({'schema':M['SCHEMA'],'kind':'DISCOVERY','scope':{'service':'owned','snapshot':F['SNAPSHOT']},
            'work':'work-a','coverage':{'record':{'revision':'rev-a'}},'choices':[row(self.a),row(self.b)],'queryComplete':True}))

    def run_packet(self, label, choices, responses=(), reuse=None):
        out=self.root/label;out.mkdir();selection=self.root/(label+'.selection.json');selection.write_text(json.dumps(choices))
        args=argparse.Namespace(action='read',clew='/supported/clew',root='/disposable/docs',service='owned',snapshot=F['SNAPSHOT'],
            work=None,prepare_work=False,contains='',page_items=20,packet=self.discovery,selection=selection,output_dir=out,reuse_read=reuse)
        sequence=iter(responses);self.commands=[]
        def native(command,**kwargs):
            self.commands.append(command)
            return subprocess.CompletedProcess(command,0,encode(next(sequence)).encode(),b'diagnostic\xff\n')
        with patch('subprocess.run',side_effect=native):stdout,code=Packet(args).run()
        return out,stdout,code,json.loads((out/'packet.json').read_text()),json.loads((out/'read-summary.json').read_text())

    def initial(self):
        return self.run_packet('initial',[{'declarationId':'decl-b'}],[F['page'](self.a),F['part']()])[0]

    def rewrite_packet(self, directory, mutate):
        path=directory/'packet.json';value=json.loads(path.read_text());mutate(value);path.write_text(encode(value))
        summary=json.loads((directory/'read-summary.json').read_text());summary['packetDigest']=digest(value)
        (directory/'read-summary.json').write_text(encode(summary))

    def test_same_input_repair_zero_native_calls_and_no_repeated_stdout_text(self):
        initial=self.initial()
        _,stdout,code,packet,summary=self.run_packet('repair',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,0);self.assertEqual(self.commands,[])
        self.assertEqual(stdout['texts'],{});self.assertEqual(len(stdout['reusedTextKeys']),1)
        self.assertEqual(summary['observedNativeCommands'],0);self.assertEqual(summary['priorEvidenceBindingsReused'],1)
        self.assertFalse(packet['freshSourceOrRuntimeVerificationClaimed']);self.assertFalse(packet['nativePublicationApprovalClaimed'])
        self.assertEqual(packet['texts'],json.loads((initial/'packet.json').read_text())['texts'])

    def test_mixed_existing_new_acquires_only_missing_binding(self):
        initial=self.initial();source=F['source']();source['id']='source-c'
        _,_,code,packet,summary=self.run_packet('mixed',[{'declarationId':'decl-b'},{'declarationId':'decl-c'}],
            [F['page'](self.b),F['part'](source,reference='s10')],initial/'packet.json')
        self.assertEqual(code,0);self.assertEqual(len(self.commands),2)
        self.assertEqual(summary['priorEvidenceBindingsReused'],1)
        self.assertEqual([o['evidenceReused'] for o in packet['outcomes']],[True,False])
        self.assertEqual(len(packet['texts']),1)

    def test_repair_packet_remains_self_contained_for_next_same_input_repair(self):
        initial=self.initial();second,_,code,_,_=self.run_packet('second',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,0)
        _,_,code,_,summary=self.run_packet('third',[{'declarationId':'decl-b'}],reuse=second/'packet.json')
        self.assertEqual(code,0);self.assertEqual(summary['observedNativeCommands'],0)

    def test_changed_snapshot_service_or_work_is_explicit_not_refetched(self):
        initial=self.initial()
        for index,(field,value) in enumerate([('snapshot','other'),('service','other'),('work','other')]):
            data=json.loads(self.discovery.read_text())
            if field=='work':data[field]=value
            else:data['scope'][field]=value
            self.discovery.write_text(encode(data))
            _,_,code,packet,_=self.run_packet('changed-'+str(index),[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
            self.assertEqual(code,2);self.assertEqual(self.commands,[])
            data['scope']={'service':'owned','snapshot':F['SNAPSHOT']};data['work']='work-a';self.discovery.write_text(encode(data))

    def test_changed_binding_is_not_silently_reused_or_refetched(self):
        initial=self.initial();data=json.loads(self.discovery.read_text());data['choices'][0]['sourceIds']=['other']
        self.discovery.write_text(encode(data))
        _,_,code,packet,_=self.run_packet('changed-binding',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,3);self.assertEqual(packet['outcomes'][0]['status'],'REUSE_BINDING_CHANGED');self.assertEqual(self.commands,[])

    def test_changed_source_reference_is_explicit_before_any_native_call(self):
        initial=self.initial();data=json.loads(self.discovery.read_text());data['choices'][0]['sourceReferences']=['s99']
        self.discovery.write_text(encode(data))
        _,_,code,packet,_=self.run_packet('changed-reference',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,3);self.assertEqual(packet['outcomes'][0]['status'],'REUSE_BINDING_CHANGED');self.assertEqual(self.commands,[])

    def test_corrupted_packet_digest_is_detected(self):
        initial=self.initial();data=json.loads((initial/'packet.json').read_text());data['texts'][next(iter(data['texts']))]='wrong'
        (initial/'packet.json').write_text(encode(data))
        _,_,code,packet,_=self.run_packet('bad-checksum',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,2);self.assertEqual(packet['status'],'REUSE_PACKET_DIGEST_MISMATCH')

    def test_coedited_packet_checksum_does_not_hide_source_metadata_corruption(self):
        initial=self.initial();self.rewrite_packet(initial,lambda p:p['outcomes'][0]['source'].update(file='wrong.cs'))
        _,_,code,packet,_=self.run_packet('bad-source',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,3);self.assertEqual(packet['outcomes'][0]['status'],'REUSE_SOURCE_METADATA_OR_TEXT_CHANGED')

    def test_unavailable_or_corrupted_native_proof_stays_explicit(self):
        initial=self.initial();receipts=json.loads((initial/'receipts.json').read_text());path=initial/receipts[-1]['stdout']
        path.write_bytes(b'corrupt')
        _,_,code,packet,_=self.run_packet('corrupt-stream',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,2);self.assertEqual(packet['status'],'REUSE_NATIVE_STREAM_DIGEST_MISMATCH')
        path.unlink()
        _,_,code,packet,_=self.run_packet('missing-stream',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,2);self.assertEqual(packet['status'],'REUSE_ARTIFACT_UNAVAILABLE')

    def test_duplicate_text_distinct_bindings_preserve_both_native_provenances(self):
        source=F['source']();source['id']='source-c'
        initial,_,code,_,_=self.run_packet('two-initial',[{'declarationId':'decl-b'},{'declarationId':'decl-c'}],
            [F['page'](self.a),F['part'](),F['page'](self.b),F['part'](source,reference='s10')])
        self.assertEqual(code,0)
        _,stdout,code,packet,summary=self.run_packet('two-repair',[{'declarationId':'decl-b'},{'declarationId':'decl-c'}],reuse=initial/'packet.json')
        self.assertEqual(code,0);self.assertEqual(summary['priorEvidenceBindingsReused'],2)
        self.assertEqual(len(packet['texts']),1);self.assertEqual(stdout['texts'],{})
        self.assertEqual({o['source']['id'] for o in packet['outcomes']},{'source-b','source-c'})

    def test_declared_provenance_edit_does_not_survive_native_proof_validation(self):
        initial=self.initial();self.rewrite_packet(initial,lambda p:p['outcomes'][0]['sourceParts'][0].update(endByte=1))
        _,_,code,packet,_=self.run_packet('bad-provenance',[{'declarationId':'decl-b'}],reuse=initial/'packet.json')
        self.assertEqual(code,3);self.assertEqual(packet['outcomes'][0]['status'],'REUSE_PART_PROOF_CHANGED')

    def test_durable_reader_policy_keeps_behavior_and_material_boundaries_visible(self):
        text=' '.join((ROOT/'skills/codeclew/references/compose-original.md').read_text().split())
        for requirement in ['behavior, data roles and consequences', 'confirmed errors, retries',
            'asynchronous or completion boundaries', 'machine evidence or the author report',
            'unknown/not-established inventories', 'rewrite the contradicted visible claim',
            'Never use prior prose as source evidence', 'not authenticate a bundle']:
            self.assertIn(requirement,text)


if __name__=='__main__':unittest.main()
