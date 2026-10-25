#!/usr/bin/env python3
"""Owned native Work schema, bounded text and authority regressions."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import tempfile
import unittest

ROOT=Path(__file__).resolve().parent.parent
E=runpy.run_path(str(ROOT/'skills/codeclew/scripts/work_evidence.py'))
P=runpy.run_path(str(ROOT/'scripts/test_work_session.py'))
page=P['work_page']


def source(text='AUTHORITY = "operator"\n'):
    return {'id':'source-1','service':'sample','file':'src/Owned.kt','startLine':10,'endLine':12,
            'revision':'owned-revision','authority':'EXACT_SNAPSHOT_TEXT','text':text,
            'textDigest':'sha256:'+hashlib.sha256(text.encode()).hexdigest(),'evidenceDigest':'sha256:captured',
            'occurrence':{'snapshot':'sha256:source','startByte':10,'endByte':10+len(text.encode())},'url':None}


def source_row(record=None,reference='s1'):
    return {'kind':'SOURCE','id':'source-1','reference':reference,'record':source() if record is None else record}


def symbol_row():
    return {'kind':'DEPENDENCY','id':'symbol-1','reference':'d1','sourceReferences':['s2'],
            'record':{'kind':'SYMBOL','id':'symbol-1','symbol':'method:OwnedBuilder.build','sourceIds':['source-2'],
                      'normalized':{'authority':'SYNTAX','name':'build','sourceTokens':['fun','build','(',')']}}}


def part(record,start,end,cursor):
    text=record['text'].encode()[start:end].decode()
    return {'schema':'codeclew-documentation-source-part/1.0','work':'work-1','snapshot':'sha256:owned/123',
            'reference':'s1','sourceId':record['id'],'recordDigest':E['digest'](record),
            'source':{k:v for k,v in record.items() if k!='text'},'text':text,'startByte':start,'endByte':end,
            'totalTextBytes':len(record['text'].encode()),'fragmentDigest':'sha256:'+hashlib.sha256(text.encode()).hexdigest(),
            'nextCursor':cursor,'authority':'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED'}


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.names=[]

    def write(self,name,value):
        p=self.root/name;p.parent.mkdir(parents=True,exist_ok=True);p.write_text(json.dumps(value));self.names.append(Path(name));return p

    def view(self,**options):
        return E['view'](self.root,'work-1',pages=self.names,**options)

    def test_literal_search_returns_matching_text_with_exact_binding_and_no_model_read_claim(self):
        record=source('prefix\n'*400+'AUTHORITY = "operator"\n'+'suffix\n'*50);p=self.write('native.stdout',page([source_row(record)]))
        before=p.read_bytes();result=self.view(references=['s1'],query='AUTHORITY',max_bytes=64)
        item=result['items'][0];self.assertIn('AUTHORITY',item['view']['text'])
        self.assertLessEqual(len(item['view']['text'].encode()),64);self.assertGreater(item['view']['remainingAvailableBytes'],0)
        self.assertEqual(item['native']['revision'],'owned-revision');self.assertEqual(item['native']['startLine'],10)
        self.assertEqual(item['native']['authority'],'EXACT_SNAPSHOT_TEXT');self.assertEqual(result['identity']['authority'],'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED')
        self.assertEqual(item['origins'][0]['rawDigest'],'sha256:'+hashlib.sha256(before).hexdigest())
        self.assertEqual(item['origins'][0]['recordDigest'],E['digest'](record))
        self.assertEqual(item['modelDelivery'],'UNATTESTED');self.assertEqual(item['modelRead'],'UNATTESTED')
        self.assertEqual(p.read_bytes(),before)

    def test_complete_selected_absence_is_distinct_from_unknown_or_unread_source(self):
        self.write('page.stdout',page([source_row(),symbol_row()]))
        complete=self.view(references=['s1'],query='NOT_PRESENT')
        self.assertEqual(complete['status'],'ABSENT_FROM_COMPLETE_SELECTED_SOURCE_TEXT')
        unread=self.view(references=['s2'],query='NOT_PRESENT')
        self.assertEqual(unread['status'],'NOT_FOUND_IN_AVAILABLE_TEXT');self.assertEqual(unread['textNotYetArchived'],['s2'])
        unknown=self.view(references=['s404'],query='NOT_PRESENT')
        self.assertEqual(unknown['unknownReferences'],['s404']);self.assertNotIn('ABSENT',unknown['status'])

    def test_symbol_is_navigation_to_source_not_a_full_body_or_compiler_upgrade(self):
        self.write('page.stdout',page([symbol_row()]))
        result=self.view(query='build');item=result['items'][0]
        self.assertEqual(item['kind'],'SYMBOL');self.assertEqual(item['reference'],'d1')
        self.assertEqual(item['sourceReferences'],['s2']);self.assertEqual(item['native']['authority'],'SYNTAX')
        self.assertEqual(item['textAvailability'],'SYMBOL_RECORD_ONLY')
        self.assertEqual(self.view(references=['s2'])['items'][0]['textAvailability'],'NOT_YET_ARCHIVED_TEXT')

    def test_source_metadata_and_retained_prose_are_not_source_text_delivery(self):
        metadata=source();del metadata['text']
        self.write('page.stdout',page([source_row(metadata),{'kind':'RETAINED_OPERATION','id':'prior','record':{'summary':{'text':'AUTHORITY = "retained"'}}}]))
        result=self.view(references=['s1']);self.assertEqual(result['items'][0]['textAvailability'],'SOURCE_METADATA_ONLY')
        self.assertEqual(self.view(query='retained')['totalMatches'],0)
        self.assertEqual(self.view(query='AUTHORITY')['status'],'NOT_FOUND_IN_AVAILABLE_TEXT')

    def test_partial_parts_preserve_remaining_cursor_and_do_not_establish_absence(self):
        record=source('prefix AUTHORITY then suffix')
        self.write('context.stdout',page())
        self.write('part-1.stdout',part(record,0,7,'part-next'))
        result=self.view(references=['s1']);item=result['items'][0]
        self.assertEqual(item['textAvailability'],'PARTIAL_SOURCE_TEXT');self.assertEqual(item['nativePartNextCursor'],'part-next')
        self.assertEqual(item['totalSourceTextBytes'],len(record['text'].encode()))
        self.assertEqual(self.view(references=['s1'],query='AUTHORITY')['status'],'NOT_FOUND_IN_AVAILABLE_TEXT')
        self.write('part-2.stdout',part(record,7,len(record['text'].encode()),None))
        complete=self.view(references=['s1'],query='AUTHORITY')
        self.assertEqual(complete['items'][0]['textAvailability'],'COMPLETE_SOURCE_TEXT')
        self.assertIn('AUTHORITY',complete['items'][0]['view']['text'])

    def test_part_digest_foreign_snapshot_gaps_and_overlaps_fail_closed(self):
        for defect in ['digest','snapshot','gap','record']:
            with self.subTest(defect=defect),tempfile.TemporaryDirectory() as directory:
                self.root=Path(directory);self.names=[];record=source('0123456789')
                self.write('context',page());first=part(record,0,5,'next');second=part(record,5,10,None)
                if defect=='digest':second['fragmentDigest']='forged'
                if defect=='snapshot':second['snapshot']='foreign'
                if defect=='gap':second=part(record,6,10,None)
                if defect=='record':second['recordDigest']='forged'
                self.write('one',first);self.write('two',second)
                with self.assertRaises(E['Incomplete']):self.view()

    def test_work_identity_reference_and_docs_page_schema_are_not_interchangeable(self):
        self.write('one',page([source_row()]));self.write('two',page(snapshot='foreign'))
        with self.assertRaisesRegex(E['Incomplete'],'IDENTITY_CHANGED'):self.view()
        self.names=[];self.write('proposal',P['proposal_page']())
        with self.assertRaisesRegex(E['Incomplete'],'UNSUPPORTED_PAGE_SCHEMA'):self.view()
        self.names=[];changed=source();changed['id']='foreign';self.write('wrong-id',page([source_row(changed)]))
        with self.assertRaisesRegex(E['Incomplete'],'SOURCE_RECORD_ID_CHANGED'):self.view()

    def test_changed_source_for_same_reference_is_never_silently_merged(self):
        self.write('one',page([source_row()]));self.write('two',page([source_row(source('changed'))]))
        with self.assertRaisesRegex(E['Incomplete'],'REFERENCE_RECORD_CHANGED'):self.view()

    def test_omissions_page_continuations_and_view_offsets_are_explicit(self):
        record=source('λ'*50)
        self.write('page',page([source_row(record)],cursor='next-page',total=2,omitted=[{'kind':'SOURCE','id':'pending','reference':'s3'}]))
        first=self.view(references=['s1'],max_bytes=10);item=first['items'][0]
        self.assertEqual(first['omittedCount'],1);self.assertEqual(first['savedPageContinuations'][0]['nextCursor'],'next-page')
        self.assertEqual(item['view']['endByte'],10)
        second=self.view(references=['s1'],max_bytes=10,offset=10)
        self.assertEqual(second['items'][0]['view']['startByte'],10);self.assertEqual(second['items'][0]['view']['remainingAvailableBytes'],80)
        self.assertEqual(first['modelRead'],'UNATTESTED')
        pending=self.view(references=['s3'])['items'][0];self.assertTrue(pending['nativeSourceOmitted']);self.assertEqual(pending['nextNativeRead']['input']['reference'],'s3')

    def test_partial_tail_reports_missing_leading_bytes_instead_of_complete_text(self):
        record=source('prefix AUTHORITY suffix')
        self.write('context',page());self.write('tail',part(record,7,len(record['text'].encode()),None))
        result=self.view(references=['s1'],query='AUTHORITY');item=result['items'][0]
        self.assertEqual(item['textAvailability'],'PARTIAL_SOURCE_TEXT')
        self.assertEqual(item['sourceWindow']['missingLeadingBytes'],7)
        self.assertEqual(item['sourceWindow']['missingTrailingBytes'],0)
        self.assertNotIn('ABSENT',self.view(references=['s1'],query='missing')['status'])

    def test_metadata_can_be_refined_by_same_exact_source_text_and_symbol_location(self):
        record=source();record['id']='source-2';metadata={k:v for k,v in record.items() if k!='text'}
        first=source_row(metadata,'s2');first['id']='source-2'
        second=source_row(record,'s2');second['id']='source-2'
        self.write('inventory',page([first,symbol_row()]));self.write('body',page([second]))
        item=self.view(references=['d1'],query='build')['items'][0]
        self.assertEqual(item['sourceBindings'][0]['reference'],'s2')
        self.assertEqual(item['sourceBindings'][0]['file'],'src/Owned.kt')
        self.assertEqual(item['sourceBindings'][0]['revision'],'owned-revision')
        self.assertTrue(item['sourceBindings'][0]['sourceTextInArchive'])
        source_item=self.view(references=['s2'])['items'][0]
        self.assertEqual(source_item['textAvailability'],'COMPLETE_SOURCE_TEXT')
        self.assertEqual(len(source_item['origins']),2)

    def test_collection_manifest_itself_is_anchored_before_read_with_regular_file_control(self):
        self.write('collection/raw.stdout',page([source_row()]))
        manifest={'schema':'codeclew-work-selection-collection/1.0','status':'INCOMPLETE',
                  'reason':'OUTSIDE_OWNED_MARKER','identity':{'work':'work-1'},'nextCursor':None,
                  'artifacts':{'pages':[{'stdout':'raw.stdout'}]}}
        with tempfile.TemporaryDirectory() as outside:
            external=Path(outside)/'manifest.json';external.write_text(json.dumps(manifest))
            link=self.root/'collection/manifest.json';link.symlink_to(external)
            original=E['S']['read_json'];reads=[]
            def observed(path):
                reads.append(path)
                return original(path)
            from unittest.mock import patch
            with patch.dict(E['S'],{'read_json':observed}):
                with self.assertRaisesRegex(E['Incomplete'],'INPUT_PATH_OUTSIDE_ASSIGNED_ROOT'):
                    E['view'](self.root,'work-1',collections=[Path('collection')],query='AUTHORITY')
            self.assertEqual(reads,[],'outside manifest must be blocked before any file read')
            self.assertTrue(link.is_symlink());self.assertEqual(json.loads(external.read_text()),manifest)
            link.unlink();manifest['reason']='INSIDE_CONTROL';link.write_text(json.dumps(manifest))
            with patch.dict(E['S'],{'read_json':observed}):
                result=E['view'](self.root,'work-1',collections=[Path('collection')],query='AUTHORITY')
            self.assertEqual(result['status'],'FOUND_IN_ARCHIVED_EVIDENCE')
            self.assertEqual(result['collectionGaps'][0]['reason'],'INSIDE_CONTROL')
            self.assertTrue(all(path.resolve().is_relative_to(self.root.resolve()) for path in reads))

    def test_collection_reads_only_declared_saved_native_files_and_preserves_incomplete_status(self):
        self.write('collection/raw.stdout',page([source_row()]))
        self.write('collection/manifest.json',{'schema':'codeclew-work-selection-collection/1.0','status':'INCOMPLETE','reason':'OMITTED_RECORDS',
            'identity':{'work':'work-1'},'nextCursor':None,'artifacts':{'pages':[{'stdout':'raw.stdout'}]}})
        result=E['view'](self.root,'work-1',collections=[Path('collection')],query='AUTHORITY')
        self.assertEqual(result['totalMatches'],1);self.assertEqual(result['collectionGaps'][0]['status'],'INCOMPLETE')


if __name__=='__main__':unittest.main()
