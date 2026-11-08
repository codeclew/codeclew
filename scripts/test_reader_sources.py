"""Cited-subset footer materialization over owned retained receipt fixtures."""
import json
from pathlib import Path
import runpy
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
S=runpy.run_path(str(ROOT/'skills/codeclew/scripts/reader_sources.py'))
F=runpy.run_path(str(ROOT/'scripts/test_author_citations.py'))


class SourcesTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.manifests=[];self.handles=[];self.records=[];self.pool={}
        for i,(service,file,revision,first) in enumerate([
            ('one','src/OrderService.kt','rev-a',40),('one','src/OrderService.kt','rev-a',91),
            ('two','src/OrderService.kt','rev-a',40),('one','other/OrderService.kt','rev-b',40),
            ('one','Unused.kt','rev-a',1)]):
            text=f'// source {i} with unicode \u03c0\nval value = {i}\n'
            record={**F['source'](text),'file':file,'revision':revision,'startLine':first,'endLine':first+1}
            manifest,view,_=F['fixture'](self.root,str(i),service=service,record=record)
            self.manifests.append(manifest);h=view['records'][0]['cite'];self.handles.append(h)
            prefix='\u03c0 prefix outside selected bytes\n';raw=(prefix+text+'unused tail').encode()
            key='u'+str(i);self.pool[key]=raw.decode()
            self.records.append({**view['records'][0],'text':key,'byteSlice':{'startByte':len(prefix.encode()),'endByte':len(prefix.encode())+len(text.encode())}})
        original={'pages':[{'page':'a.md','fragments':[{'id':'claim','text':'# A\n\nBound explanation.\n','evidence':self.handles[:4]+[self.handles[0]]}]},
                           {'page':'b.md','fragments':[{'id':'claim','text':'# B\n\nOwn subset.\n','evidence':[self.handles[2]]}]}]}
        # Duplicate evidence within one claim is forbidden; repeat via a distinct claim.
        original['pages'][0]['fragments'][0]['evidence']=self.handles[:4]
        original['pages'][0]['fragments'].append({'id':'again','text':'\nRepeated source.\n','evidence':[self.handles[0]]})
        (self.root/'original.json').write_text(json.dumps(original))
        S['M']['compose'](self.root/'original.json',self.root/'compose')
        S['M']['bind'](self.manifests,self.root/'compose/markdown',self.root/'compose/request.json',self.root/'bound')
        self.index=self.root/'bound/index.json'
        self.view=self.root/'union.json';self.view.write_text(json.dumps({'records':self.records,'texts':self.pool}))

    def materialize(self,views=True):
        output=self.root/'sources.json'
        result=S['materialize'](self.index,self.manifests,output,[self.view] if views else [])
        return result,json.loads(output.read_text())

    def test_saved_utf8_slices_group_ranges_dedup_and_keep_page_subsets_without_query(self):
        with patch.dict(S['materialize'].__globals__['C'],{'grouped_sources':lambda *a: (_ for _ in ()).throw(AssertionError('No repeated inspector'))}):
            result,bundle=self.materialize()
        self.assertEqual(result['citations'],4)
        groups=bundle['pages'][0]['groups'];self.assertEqual(len(groups),3)
        group=next(g for g in groups if g['service']=='one' and g['file']=='src/OrderService.kt')
        self.assertEqual([(r['startLine'],r['endLine']) for r in group['ranges']],[(40,41),(91,92)])
        self.assertEqual(len(bundle['pages'][1]['groups']),1)
        for page in bundle['pages']:
            for g in page['groups']:
                for r in g['ranges']:self.assertNotIn('outside selected bytes',r['text']);self.assertNotIn('unused tail',r['text'])
        self.assertNotIn('Unused.kt',json.dumps(bundle))

    def test_no_view_uses_bounded_grouped_projection_and_matches_saved_views(self):
        _,first=self.materialize(False)
        (self.root/'sources.json').unlink();_,second=self.materialize(True)
        self.assertEqual(first,second)

    def test_missing_or_changed_saved_excerpt_refuses_without_output(self):
        for kind in ['missing','text','range']:
            value={'records':json.loads(json.dumps(self.records)),'texts':dict(self.pool)}
            if kind=='missing':value['records']=value['records'][1:]
            elif kind=='text':value['texts']['u0']=value['texts']['u0'].replace('value','wrong')
            else:value['records'][0]['location']='one:src/OrderService.kt:1-2'
            self.view.write_text(json.dumps(value))
            with self.subTest(kind=kind),self.assertRaises(S['M']['Stop']):self.materialize()
            self.assertFalse((self.root/'sources.json').exists())


if __name__=='__main__':unittest.main()
