#!/usr/bin/env python3
"""Lossless selected-source projection and unchanged citation contract controls."""
from __future__ import annotations
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
SCRIPTS = ROOT / 'skills/codeclew/scripts'
U = runpy.run_path(str(SCRIPTS/'source_union.py'))
A = runpy.run_path(str(SCRIPTS/'author_citations.py'))
F = runpy.run_path(str(ROOT/'scripts/test_author_citations.py'))


def row(text, start=0, *, file='File.kt', revision='rev-a', snapshot='source-snapshot', blob='blob-a', cite='m:e1', reused=False):
    digest = 'sha256:' + hashlib.sha256(text.encode()).hexdigest()
    source = {'file':file, 'revision':revision, 'service':'owned', 'textDigest':digest,
              'occurrence':{'snapshot':snapshot, 'blob':blob, 'startByte':start, 'endByte':start+len(text.encode())}}
    return ({'cite':cite, 'text':'old', 'authority':'EXACT_SNAPSHOT_TEXT', 'bodyStatus':None, 'limits':'l1'},
            {'source':source, 'textKey':digest, 'evidenceReused':reused}), {digest:text}


class SourceUnionTest(unittest.TestCase):
    def project(self, values, *, scope=None, include_reused=False):
        rows, texts = [], {}
        for pair, pool in values:rows.append(pair);texts.update(pool)
        before = copy.deepcopy((rows,texts))
        records, projected, info = U['compact'](rows,texts,scope or {'snapshot':'packet-snapshot'},include_reused=include_reused)
        self.assertEqual((rows,texts), before)
        for record, (_, outcome) in zip(records,rows):
            if record['text'] in projected:
                span = record['byteSlice'];raw = projected[record['text']].encode()[span['startByte']:span['endByte']]
                self.assertEqual(raw, texts[outcome['textKey']].encode())
            self.assertEqual(record['cite'], rows[records.index(record)][0]['cite'])
            self.assertIsNone(record['bodyStatus']);self.assertEqual(record['limits'],'l1')
        return records,projected,info

    def test_nested_containment_preserves_comments_sql_and_citations(self):
        values = [row('/* parent */\nSELECT * FROM data;\nreturn\n',100),row('SELECT * FROM data;',113,cite='m:e2')]
        records,texts,info = self.project(values)
        self.assertEqual(len(texts),1);self.assertEqual(info['metrics']['overlapBytesRemoved'],19)
        self.assertEqual(records[1]['byteSlice'],{'startByte':13,'endByte':32})

    def test_partial_agreeing_overlap_and_transitive_union(self):
        _,texts,info = self.project([row('abcdef',5),row('defghi',8,cite='m:e2'),row('hijk',12,cite='m:e3')])
        self.assertEqual(list(texts.values()),['abcdefghijk']);self.assertEqual(info['metrics']['overlapBytesRemoved'],5)

    def test_disjoint_and_adjacent_ranges_remain_distinct(self):
        for start in [3,8]:
            _,texts,_ = self.project([row('abc'),row('xyz',start,cite='m:e2')]);self.assertEqual(len(texts),2)

    def test_equal_text_different_source_identities_never_merge(self):
        for field in ['file','revision','snapshot','blob']:
            with self.subTest(field=field):
                _,texts,info = self.project([row('same'),row('same',cite='m:e2',**{field:'different'})])
                self.assertEqual(len(texts),2);self.assertEqual(info['metrics']['overlapBytesRemoved'],0)
                self.assertEqual(info['metrics']['canonicalToProjectedByteDelta'],-4)
        left,pool = row('same');right,other = row('same',cite='m:e2');right[1]['source']['service']='other'
        self.assertEqual(len(self.project([(left,pool),(right,other)])[1]),2)

    def test_conflicting_overlap_preserves_entire_connected_component(self):
        _,texts,info = self.project([row('abcdef'),row('XXghi',4,cite='m:e2'),row('hi!',7,cite='m:e3')])
        self.assertEqual(len(texts),3);self.assertEqual(len(info['fallbacks']),3)
        self.assertEqual({r['reason'] for r in info['fallbacks']},{'CONFLICTING_OVERLAP'})
        self.assertEqual(info['metrics']['overlapBytesRemoved'],0)

    def test_multibyte_utf8_offsets_are_bytes_not_characters(self):
        records,texts,_ = self.project([row('aé☀中z',10),row('☀中',13,cite='m:e2')])
        self.assertEqual(len(texts),1);self.assertEqual(records[1]['byteSlice'],{'startByte':3,'endByte':9})

    def test_missing_invalid_or_insufficient_coordinates_preserve_text(self):
        for field,value in [('occurrence',None),('startByte',None),('endByte',999),('startByte',True),('blob',None)]:
            pair,pool = row('abc',cite='m:e2')
            if field=='occurrence':pair[1]['source'][field]=value
            else:pair[1]['source']['occurrence'][field]=value
            _,texts,info = self.project([row('abc'),(pair,pool)])
            self.assertEqual(len(texts),2);self.assertEqual(len(info['fallbacks']),1)

    def test_unresolved_syntax_authority_is_not_upgraded(self):
        pair,pool = row('abc');pair[0]['authority']='SOURCE_SYNTAX';pair[0]['limits']='CALL_TARGETS_UNRESOLVED'
        records,_,_ = U['compact']([pair],pool,{'snapshot':'snap'})
        self.assertEqual(records[0]['authority'],'SOURCE_SYNTAX');self.assertEqual(records[0]['limits'],'CALL_TARGETS_UNRESOLVED')
        self.assertIsNone(records[0]['bodyStatus'])

    def test_corrupt_text_digest_fails_instead_of_guessing(self):
        pair,pool = row('abc');pair[1]['source']['textDigest']='sha256:bad'
        with self.assertRaisesRegex(ValueError,'TEXT_DIGEST_MISMATCH'):self.project([(pair,pool)])

    def test_reuse_and_mixed_new_reused_region_delivery(self):
        values = [row('abcdef',reused=True),row('cde',2,cite='m:e2',reused=True)]
        records,texts,info = self.project(values)
        self.assertEqual(texts,{});self.assertTrue(all(r['retainedText'] for r in records))
        self.assertEqual(info['metrics']['deliveredUniqueBytes'],0);self.assertEqual(info['metrics']['projectedUniqueBytes'],6)
        self.assertEqual(len(self.project(values,include_reused=True)[1]),1)
        mixed = [values[0],row('cde',2,cite='m:e2')]
        records,texts,_ = self.project(mixed);self.assertEqual(list(texts.values()),['abcdef'])
        self.assertFalse(any(r.get('retainedText') for r in records))

    def test_projection_keeps_canonical_packet_manifest_handles_and_bindings(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            manifest,view,_ = F['fixture'](directory)
            before = {name:(manifest.parent/name).read_bytes() for name in ['packet.json','citations.json','discovery.input.json']}
            _,packet,discovery,entries = A['load'](manifest)
            compact = A['project'](packet,discovery,manifest.parent,source_union=True,include_reused=True,save=False)
            self.assertEqual([r['cite'] for r in view['records']],[r['cite'] for r in compact['records']])
            self.assertEqual(before,{name:(manifest.parent/name).read_bytes() for name in before})
            result = subprocess.run([sys.executable,'-I','-S',str(SCRIPTS/'author_citations.py'),'inspect','--manifest',str(manifest),'--source-union'],capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr);self.assertEqual(json.loads(result.stdout),compact)
            draft=directory/'draft.md';draft.write_text('Owned citation fixture.\n');claims=directory/'claims.json'
            claims.write_text(json.dumps({'claims':[{'id':'one','text':'Owned citation fixture.','evidence':[compact['records'][0]['cite']]}]}))
            checked=A['compile_map']([manifest],draft,claims,directory/'bound')
            self.assertEqual(checked['status'],'MECHANICALLY_VALID');self.assertFalse(checked['semanticTruthAssessed'])
            actual=json.loads((directory/'bound/sources.json').read_text())['claims'][0]['sources'][0]
            self.assertEqual(actual['sourceRecordDigest'],next(iter(entries.values()))['sourceRecordDigest'])


if __name__=='__main__':unittest.main()
