#!/usr/bin/env python3
"""Mandatory native components survive current compose/bind/export contracts."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
E=runpy.run_path(str(ROOT/'skills/codeclew/scripts/reader_export_check.py'));M=E['M']
F=runpy.run_path(str(ROOT/'scripts/test_author_citations.py'))


class DeclarationComponentsTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.addCleanup(E['configure_engine'])
        self.manifest,view,_=F['fixture'](self.root);self.handle=view['records'][0]['cite']
        self.html='<section id="clew-component-record" data-clew-component="record"><h2>Record</h2><table><tr><td>key</td><td>uuid</td><td>Primary key</td></tr><tr><td>label</td><td>text</td><td>Default expression</td></tr></table><p>Partial projection (1)</p><details><summary>Limits</summary><p>A constraint is retained without interpretation.</p></details></section>\n'
        self.packet={'schema':'codeclew-declaration-components/1.0','components':[{'id':'record','title':'Record','html':self.html,'htmlDigest':M['sha'](self.html.encode()),'binding':{'source':{'file':'neutral.sql','revision':'neutral','textDigest':'sha256:neutral'}},'projection':{'frontier':[{'code':'UNSUPPORTED'}]}}]}
        self.ticket={'schema':'codeclew-declaration-components-ticket/1.0','id':'sha256:selected','reference':{'digest':'sha256:selected'},'components':[{'id':'record'}]}
        self.ticket_path=self.root/'ticket.json';self.ticket_path.write_text(json.dumps(self.ticket))
        response={'schema':'codeclew-declaration-components-read/1.0','ticket':self.ticket,'packet':self.packet}
        self.engine=self.root/'engine';self.engine.write_text('#!'+sys.executable+'\nimport json\nprint('+repr(json.dumps(response))+')\n');self.engine.chmod(0o700)
        E['configure_engine'](self.engine,self.root,self.ticket_path,'sha256:selected')
        self.original={'pages':[{'page':'index.md','fragments':[{'id':'purpose','text':'The record carries a retained declaration.\n\n','evidence':[self.handle]},{'component':'record'}]},
            {'page':'guide.md','fragments':[{'id':'use','text':'Use the declared record.\n\n','evidence':[self.handle]},{'componentRef':'record','text':'Record declaration'}]}]}

    def composed(self,value=None):
        value=self.original if value is None else value
        return M['compose_pages'](value)

    def test_one_canonical_table_and_reference_preserve_prose_and_source_binding(self):
        before=copy.deepcopy(self.original);pages,request,_=self.composed()
        self.assertEqual(sum(raw.count(self.html.encode()) for raw in pages.values()),1)
        self.assertIn(b'index.md#clew-component-record',pages['guide.md'])
        self.assertIn(b'The record carries a retained declaration.',pages['index.md'])
        self.assertEqual(self.original,before)
        M['component_call']('validate_markdown',request['components'],pages)
        html={name.replace('.md','.html'):body.replace(b'index.md#',b'index.html#') for name,body in pages.items()}
        M['component_call']('validate_html',request['components'],html)

    def test_shared_component_links_resolve_from_nested_page_directories(self):
        value=copy.deepcopy(self.original)
        value['pages'][0]['page']='declarations/record.md'
        value['pages'][1]['page']='guides/deep/use.md'
        pages,_,_=self.composed(value)
        self.assertIn(b'../../declarations/record.md#clew-component-record',pages['guides/deep/use.md'])

    def test_omission_duplication_and_author_cell_override_refuse(self):
        for change in ('omit','duplicate','values','unknown','untyped'):
            value=copy.deepcopy(self.original)
            if change=='omit':value['pages'][0]['fragments'].pop()
            elif change=='duplicate':value['pages'][1]['fragments'].append({'component':'record'})
            elif change=='values':value['pages'][0]['fragments'][1]['text']='changed row'
            elif change=='unknown':value['pages'][0]['fragments'][1]['component']='other'
            else:value['pages'][0]['fragments'][1]={'text':self.html}
            with self.subTest(change=change),self.assertRaises(M['Stop']):self.composed(value)

    def test_row_frontier_and_binding_changes_refuse_even_with_rewritten_digests(self):
        pages,request,_=self.composed()
        for old,new in (('<td>uuid</td>','<td>text</td>'),('<tr><td>label</td><td>text</td><td>Default expression</td></tr>',''),('<p>Partial projection (1)</p>','')):
            changed={**pages,'index.md':pages['index.md'].replace(old.encode(),new.encode())}
            with self.subTest(old=old),self.assertRaises(M['Stop']):M['component_call']('validate_markdown',request['components'],changed)
            with self.subTest(export=old),self.assertRaises(M['Stop']):M['component_call']('validate_html',request['components'],{'index.html':changed['index.md'],'guide.html':changed['guide.md']})
        for key in ('ticket','packetDigest'):
            bad=copy.deepcopy(request['components']);bad['binding'][key]='changed'
            with self.subTest(binding=key),self.assertRaises(M['Stop']):M['component_call']('validate_markdown',bad,pages)
        with self.assertRaises(M['Stop']):M['component_call']('validate_markdown',None,pages)

    def test_substituted_ticket_cannot_narrow_selected_scope(self):
        bad=copy.deepcopy(self.ticket);bad['id']='sha256:narrowed';bad['reference']['digest']='sha256:narrowed';self.ticket_path.write_text(json.dumps(bad))
        with self.assertRaises(M['Stop']) as error:E['configure_engine'](self.engine,self.root,self.ticket_path,'sha256:selected')
        self.assertEqual(error.exception.status,'EXPECTED_COMPONENT_SET_CHANGED')

    def test_current_bind_and_export_check_protect_native_table_independently(self):
        original=self.root/'original.json';original.write_text(json.dumps(self.original));composed=self.root/'composed';M['compose'](original,composed)
        bound=self.root/'bound';M['bind']([self.manifest],composed/'markdown',composed/'request.json',bound)
        self.assertEqual(M['component_call']('native_commands'),1)
        self.assertTrue((bound/'components.json').is_file());M['check']([self.manifest],bound/'index.json')
        html=self.root/'html';html.mkdir()
        for path in (bound/'markdown').glob('*.md'):
            (html/path.with_suffix('.html').name).write_bytes(path.read_bytes().replace(b'index.md#',b'index.html#'))
        renderer=self.root/'renderer.py';renderer.write_text('import sys\nfrom pathlib import Path\na,b=map(Path,sys.argv[1:]);b.mkdir()\nfor p in a.glob("*.md"):(b/p.with_suffix(".html").name).write_bytes(p.read_bytes().replace(b"index.md#",b"index.html#"))\n')
        command=self.root/'renderer.json';command.write_text(json.dumps([sys.executable,'-I','-S',str(renderer),'{input}','{output}']))
        result=E['verify']([self.manifest],bound/'index.json',composed/'markdown',html,command,self.root/'check')
        self.assertEqual(result['status'],'EXPORT_CHECKED')
        path=html/'index.html';path.write_bytes(path.read_bytes().replace(b'<td>uuid</td>',b'<td>text</td>'))
        with self.assertRaises(M['Stop']) as error:E['verify']([self.manifest],bound/'index.json',composed/'markdown',html,command,self.root/'bad-check')
        self.assertEqual(error.exception.status,'EXPORTED_COMPONENT_CHANGED')

    def test_finalize_forwards_expected_inventory_through_child_processes(self):
        original=self.root/'original.json';original.write_text(json.dumps(self.original));composed=self.root/'composed';M['compose'](original,composed)
        renderer=self.root/'renderer.py';renderer.write_text('import sys\nfrom pathlib import Path\na,b=map(Path,sys.argv[1:]);b.mkdir()\nfor p in a.glob("*.md"):(b/p.with_suffix(".html").name).write_bytes(p.read_bytes().replace(b"index.md#",b"index.html#"))\n')
        command=self.root/'renderer.json';command.write_text(json.dumps([sys.executable,'-I','-S',str(renderer),'{input}','{output}']))
        result=E['finalize'](composed,[self.manifest],command,self.root/'final')
        self.assertEqual(result['status'],'CHECKED_PAGES_FINALIZED')
        self.assertIn(self.html,(self.root/'final/html/index.html').read_text())
        receipt=json.loads((self.root/'final/receipts/export-check.command.json').read_bytes())
        self.assertIn('--component-set-digest',receipt['argv'])

    def test_dropping_index_inventory_is_rejected_after_rehashing(self):
        original=self.root/'original.json';original.write_text(json.dumps(self.original));composed=self.root/'composed';M['compose'](original,composed)
        bound=self.root/'bound';M['bind']([self.manifest],composed/'markdown',composed/'request.json',bound)
        index=json.loads((bound/'index.json').read_bytes());del index['components'];index['indexDigest']=M['digest']({k:v for k,v in index.items() if k!='indexDigest'});(bound/'index.json').write_text(M['encode'](index))
        with self.assertRaises(M['Stop']) as error:M['check']([self.manifest],bound/'index.json')
        self.assertEqual(error.exception.status,'COMPONENT_BINDING_CHANGED')


if __name__=='__main__':unittest.main()
