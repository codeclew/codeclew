#!/usr/bin/env python3
"""Owned native-protocol XML/SQL/Java file discovery, exact parts and citations."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
M=runpy.run_path(str(ROOT/'skills/codeclew/scripts/evidence_packet.py'))
A=runpy.run_path(str(ROOT/'skills/codeclew/scripts/author_citations.py'))
F=runpy.run_path(str(ROOT/'scripts/test_work_bound_source.py'))
encode,digest=M['encode'],M['digest']


class RetainedFilesTest(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.root=Path(self.tmp.name)
        self.sources=[];self.commands=[];self.requests=[];self.bad=None
        for i,(file,text) in enumerate([('config/app.xml','<config>\n  <value>π🙂</value>\n</config>\n'),
            ('db/schema.sql','CREATE TABLE sample (id INTEGER);\n'),('src/Main.java','class Main { int read() { return 1; } }\n')]):
            source=F['source'](text);source.update(id='source-'+str(i),file=file,startLine=1,endLine=text.count('\n'))
            self.sources.append(source)

    def native(self,command,**kwargs):
        self.commands.append(command);self.assertIn(command[1:4],[['docs','work','read'],['docs','work','read-part']])
        request=json.loads(Path(command[command.index('--input')+1]).read_text());self.requests.append(request)
        if command[3]=='read-part':
            index=int(request['reference'][1:])-1;source=self.sources[index];raw=source['text'].encode()
            start=int(request.get('cursor','offset:0').split(':')[1]);end=min(len(raw),start+40000)
            while end<len(raw) and raw[end]&0xc0==0x80:end-=1
            value=F['part'](source,start,end,'offset:'+str(end) if end<len(raw) else None,request['reference'])
            if self.bad=='part':value['recordDigest']='bad'
        else:
            query=request.get('query',{});projection=query.get('projection')
            if 'references' in request:
                rows=[{'kind':'COVERAGE','id':'owned','revision':'rev-a'}]
                rows += [{'kind':'SOURCE','id':self.sources[int(ref[1:])-1]['id'],'reference':ref,
                          'record':self.sources[int(ref[1:])-1]} for ref in request['references']]
            elif projection=='FILES':
                files={s['file'] for s in self.sources if query.get('fileContains','') in s['file']}
                rows=[{'kind':'SOURCE_FILE','id':file,'record':{'file':file,'service':'owned','revision':'rev-a',
                    'sourceRecords':sum(s['file']==file for s in self.sources),'authorities':['EXACT_SNAPSHOT_TEXT'],
                    'authority':'NAVIGATION_ONLY','wholeFileCapture':'NOT_ESTABLISHED_BY_GROUPING'}} for file in sorted(files)]
            else:
                self.assertIn(projection,['NAVIGATION','WHOLE_FILE']);rows=[]
                if projection=='WHOLE_FILE' and self.bad=='legacy':
                    return subprocess.CompletedProcess(command,2,encode({'error':{'message':'WHOLE_FILE_NOT_RETAINED'}}).encode(),b'')
                for i,s in enumerate(self.sources):
                    if s['file']!=query['file']:continue
                    rows.append({'kind':'SOURCE_NAVIGATION','id':s['id'],'record':{'source':{k:v for k,v in s.items() if k!='text'},
                        'recordDigest':digest(s),'fullRecordReference':'s'+str(i+1),'textDelivered':False,'authority':'NAVIGATION_ONLY'}})
                if projection=='WHOLE_FILE':
                    rows=rows[:1]
                    if rows and self.bad!='missing-marker':rows[0]['record']['wholeFileCapture']='EXACT_RETAINED_FILE'
            capacity=getattr(self,'page_size',2)
            start=int(request.get('cursor','page:0').split(':')[1]);selected=rows[start:start+capacity]
            value={**F['page'](*selected,cursor='page:'+str(start+capacity) if start+capacity<len(rows) else None),
                   'total':len(rows),'membershipDigest':digest([r['id'] for r in rows]),'contextDigest':'ctx','inputDigest':'input','authority':'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED'}
            if request.get('projection')=='SOURCE_ROWS':value['selectionProjection']='SOURCE_ROWS'
            if self.bad=='foreign':value['work']='other'
            if self.bad=='omitted':value['omitted']=[{'id':'missing','kind':'SOURCE_NAVIGATION'}]
            value['receiptDigest']=digest(value)
        return subprocess.CompletedProcess(command,0,encode(value).encode(),b'')

    def packet(self,action,name,*,discovery=None,files=None,reuse=None,contains='',whole=False,output_format='author',batch=1):
        out=self.root/name;out.mkdir();selection=self.root/(name+'.selection.json')
        if files is not None:selection.write_text(encode([{'file':f} for f in files]))
        args=argparse.Namespace(action=action,clew='/supported/clew',root='/disposable/docs',service='owned',snapshot=F['SNAPSHOT'],
            work='work-a' if action=='discover-files' else None,prepare_work=False,path_contains=contains,contains='',page_items=2,
            packet=discovery,selection=selection,output_dir=out,reuse_read=reuse,output_format=output_format,whole_files=whole,
            source_batch_size=batch)
        self.commands=[];self.requests=[]
        with patch('subprocess.run',side_effect=self.native):value,code=M['Packet'](args).run()
        return out,value,code

    def prepare(self):return self.packet('discover-files','discovery')[0]/'packet.json'

    def test_batched_overlapping_file_sources_keep_old_and_reusable_proof_routes(self):
        for source in self.sources:source.update(file='Same.java',evidenceDigest='sha256:original')
        discovery=self.prepare()
        old,_,code=self.packet('read-files','old',discovery=discovery,files=['Same.java']);self.assertEqual(code,0)
        fresh,_,code=self.packet('read-files','batch',discovery=discovery,files=['Same.java'],batch=8);self.assertEqual(code,0)
        self.assertFalse(any(c[3]=='read-part' for c in self.commands));A['load'](fresh/'citations.json')
        before=json.loads((old/'packet.json').read_text());after=json.loads((fresh/'packet.json').read_text())
        self.assertEqual(before['texts'],after['texts'])
        self.assertEqual([o['source'] for o in before['outcomes']],[o['source'] for o in after['outcomes']])
        for name,prior in [('reuse',fresh),('again',self.root/'reuse')]:
            reused,_,code=self.packet('read-files',name,discovery=discovery,files=['Same.java'],reuse=prior/'packet.json',batch=8)
            self.assertEqual(code,0);self.assertEqual(self.commands,[]);A['load'](reused/'citations.json')

    def test_metadata_delivery_keeps_full_packet_bindings_and_grouped_inspection(self):
        for source in self.sources:
            source['authority']='EXACT_SNAPSHOT_TEXT'
            source['occurrence']={'snapshot':'sha256:'+'a'*64,'blob':'b'*40,'startByte':0,'endByte':len(source['text'].encode())}
        discovery=self.prepare();files=[s['file'] for s in self.sources]
        full,view,code=self.packet('read-files','full',discovery=discovery,files=files,whole=True)
        meta,receipt,meta_code=self.packet('read-files','metadata',discovery=discovery,files=files,whole=True,output_format='metadata')
        self.assertEqual((code,meta_code),(0,0));self.assertEqual(receipt['texts'],{})
        self.assertFalse(receipt['sourceBodiesDisplayed'])
        self.assertEqual((full/'packet.json').read_bytes(),(meta/'packet.json').read_bytes())
        self.assertEqual((full/'citations.json').read_bytes(),(meta/'citations.json').read_bytes())
        for old,new in zip(view['records'],receipt['records']):
            self.assertEqual({k:v for k,v in new.items() if k!='retainedText'},old)
            self.assertTrue(new['retainedText']);self.assertTrue(new['wholeFile'])
        for source in self.sources:self.assertNotIn(source['text'],encode(receipt))
        request=self.root/'views.json';request.write_text(encode([{'file':f} for f in files]))
        grouped=A['inspect_grouped'](meta/'citations.json',request)
        self.assertEqual([g['fragments'][0]['text'] for g in grouped['groups']],[s['text'] for s in self.sources])
        self.assertEqual([v['cite'] for v in grouped['views']],[r['cite'] for r in receipt['records']])

    def test_metadata_delivery_keeps_failure_status_and_full_diagnostics(self):
        discovery=self.prepare();self.bad='part'
        out,value,code=self.packet('read-files','failed-metadata',discovery=discovery,files=[self.sources[0]['file']],whole=True,output_format='metadata')
        self.assertNotEqual(code,0);self.assertTrue(value['failures']);self.assertEqual(value['texts'],{})
        self.assertTrue((out/'failure-details.json').is_file());self.assertFalse(value['sourceBodiesDisplayed'])

    def test_strict_whole_file_ignores_nested_fragments_and_reuses_exact_alias(self):
        source=self.sources[2];source['text']='package owned\r\nimport kotlin.String\r\n// π☀\r\n'+('val x = "λ"\r\n'*5000)
        source['textDigest']='sha256:'+hashlib.sha256(source['text'].encode()).hexdigest()
        source['endLine']=len(source['text'].split('\n'))-1
        source['occurrence']={'snapshot':'sha256:'+'a'*64,'blob':'b'*40,'startByte':0,'endByte':len(source['text'].encode())}
        nested=copy.deepcopy(source);nested.update(id='nested',text='val x = "λ"')
        self.sources.append(nested)
        discovery=self.prepare()
        out,view,code=self.packet('read-files','whole',discovery=discovery,files=[source['file']],whole=True)
        self.assertEqual(code,0);self.assertEqual(len(view['records']),1)
        self.assertEqual(self.requests[0]['query']['projection'],'WHOLE_FILE')
        packet=json.loads((out/'packet.json').read_text());self.assertEqual(len(packet['outcomes']),1)
        self.assertGreater(len(packet['outcomes'][0]['sourceParts']),1)
        self.assertEqual(packet['texts'][source['textDigest']].encode(),source['text'].encode())
        A['load'](out/'citations.json')
        reused,_,code=self.packet('read-files','whole-reused',discovery=discovery,files=[source['file']],whole=True,reuse=out/'packet.json')
        self.assertEqual(code,0);self.assertEqual(self.commands,[]);A['load'](reused/'citations.json')

    def test_strict_whole_file_refuses_legacy_marker_range_and_authority(self):
        source=self.sources[0];source['occurrence']={'snapshot':'sha256:'+'a'*64,'blob':'b'*40,'startByte':0,'endByte':len(source['text'].encode())}
        discovery=self.prepare()
        for bad in ['legacy','missing-marker','range','authority']:
            self.bad=bad
            if bad=='range':source['occurrence']['startByte']=1
            if bad=='authority':source['authority']='SOURCE_SYNTAX'
            out,view,code=self.packet('read-files','strict-'+bad,discovery=discovery,files=[source['file']],whole=True)
            self.assertNotEqual(code,0);self.assertTrue(view['failures'])
            if bad=='legacy':self.assertEqual(view['failures'][0]['status'],'WHOLE_FILE_NOT_RETAINED')
            source['occurrence']['startByte']=0;source['authority']='EXACT_SNAPSHOT_TEXT'
        self.bad=None

    def test_unfiltered_and_known_path_discovery_never_deliver_bodies(self):
        out,value,code=self.packet('discover-files','all')
        self.assertEqual(code,0);self.assertEqual(len(value['choices']),3);self.assertEqual(len(self.commands),2)
        self.assertNotIn('<config>',encode(value));self.assertFalse(value['sourceBodiesRequested'])
        _,view,code=self.packet('discover-files','filtered',contains='.xml')
        self.assertEqual(code,0);self.assertEqual([r['file'] for r in view['choices']],['config/app.xml'])
        self.assertEqual(view['choices'][0]['wholeFileCapture'],'NOT_ESTABLISHED_BY_GROUPING')

    def test_file_only_xml_sql_and_java_bind_without_any_declaration(self):
        discovery=self.prepare();out,view,code=self.packet('read-files','read',discovery=discovery,files=[s['file'] for s in self.sources])
        self.assertEqual(code,0);packet=json.loads((out/'packet.json').read_text())
        self.assertTrue(all('declaration' not in o for o in packet['outcomes']))
        self.assertEqual(sorted(view['texts'].values()),sorted(s['text'] for s in self.sources))
        manifest,_,_,entries=A['load'](out/'citations.json');self.assertEqual(len(entries),3)
        md=self.root/'guide.md';md.write_text('An authored statement.\n');claims=self.root/'claims.json'
        claims.write_text(encode({'claims':[{'id':'one','text':'An authored statement.','evidence':list(entries)}]}))
        result=A['compile_map']([out/'citations.json'],md,claims,self.root/'bound')
        self.assertFalse(result['semanticTruthAssessed']);bound=json.loads((self.root/'bound/sources.json').read_text())
        self.assertTrue(all(s['evidenceKind']=='RETAINED_SOURCE' and 'declarationId' not in s for s in bound['claims'][0]['sources']))

    def test_multiple_records_one_file_and_oversized_unicode_parts_stay_distinct(self):
        s=copy.deepcopy(self.sources[0]);s.update(id='different-source');self.sources.append(s)
        large=self.sources[0];large['text']='π🙂\n'*13000;large['textDigest']='sha256:'+hashlib.sha256(large['text'].encode()).hexdigest();large['endLine']=13000
        discovery=self.prepare();out,view,code=self.packet('read-files','read',discovery=discovery,files=['config/app.xml'])
        self.assertEqual(code,0);self.assertEqual(len(view['records']),2);self.assertEqual(len(view['texts']),2)
        summary=json.loads((out/'read-summary.json').read_text());self.assertEqual(summary['selectedRequests'],1);self.assertEqual(summary['verifiedSourceRecords'],2)
        packet=json.loads((out/'packet.json').read_text());self.assertGreater(len(packet['outcomes'][0]['sourceParts']),1)
        A['load'](out/'citations.json')

    def test_same_input_repair_zero_native_calls_and_mixed_only_missing_file(self):
        discovery=self.prepare();out,_,code=self.packet('read-files','first',discovery=discovery,files=['config/app.xml'])
        self.assertEqual(code,0);original={str(p):p.read_bytes() for p in out.iterdir()}
        repaired,view,code=self.packet('read-files','repair',discovery=discovery,files=['config/app.xml'],reuse=out/'packet.json')
        self.assertEqual(code,0);self.assertEqual(self.commands,[]);self.assertEqual(view['texts'],{})
        A['load'](repaired/'citations.json')
        mixed,_,code=self.packet('read-files','mixed',discovery=discovery,files=['config/app.xml','db/schema.sql'],reuse=out/'packet.json')
        self.assertEqual(code,0);self.assertEqual(len(self.commands),2)
        self.assertEqual(self.requests[0]['query']['file'],'db/schema.sql')
        self.assertEqual({str(p):p.read_bytes() for p in out.iterdir()},original);A['load'](mixed/'citations.json')

    def test_foreign_work_omissions_and_corrupt_parts_never_become_success(self):
        discovery=self.prepare()
        for bad in ['foreign','omitted','part']:
            with self.subTest(bad=bad):
                self.bad=bad
                out,view,code=self.packet('read-files',bad,discovery=discovery,files=['config/app.xml'])
                self.assertNotEqual(code,0);self.assertTrue(view['failures'])
                with self.assertRaises(A['Stop']):A['load'](out/'citations.json')
                self.bad=None

    def test_unknown_file_and_stale_snapshot_fail_without_native_reads(self):
        discovery=self.prepare();out,view,code=self.packet('read-files','unknown',discovery=discovery,files=['private/unknown.xml'])
        self.assertNotEqual(code,0);self.assertEqual(self.commands,[])
        old=json.loads(discovery.read_text());old['scope']['snapshot']='changed';discovery.write_text(encode(old))
        _,view,code=self.packet('read-files','stale',discovery=discovery,files=['config/app.xml'])
        self.assertEqual(code,2);self.assertEqual(self.commands,[])

    def test_reuse_rejects_changed_revision_or_reference_and_corrupt_native_proof(self):
        discovery=self.prepare();out,_,code=self.packet('read-files','first',discovery=discovery,files=['config/app.xml'])
        self.assertEqual(code,0);original=json.loads((out/'packet.json').read_text());summary=json.loads((out/'read-summary.json').read_text())
        for field in ['revision','reference','proof']:
            with self.subTest(field=field):
                p=copy.deepcopy(original)
                if field=='revision':p['outcomes'][0]['source']['revision']='wrong'
                elif field=='reference':p['outcomes'][0]['sourceBinding']['reference']='s99'
                else:
                    receipt=json.loads((out/'receipts.json').read_text())[-1];(out/receipt['stdout']).write_text('{}')
                (out/'packet.json').write_text(encode(p));(out/'read-summary.json').write_text(encode({**summary,'packetDigest':digest(p)}))
                _,view,code=self.packet('read-files','bad-'+field,discovery=discovery,files=['config/app.xml'],reuse=out/'packet.json')
                self.assertNotEqual(code,0);self.assertEqual(self.commands,[])


if __name__=='__main__':unittest.main()
