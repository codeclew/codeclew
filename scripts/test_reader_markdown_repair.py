#!/usr/bin/env python3
"""Addressed reader changes over existing valid READ/citation fixtures."""
import contextlib
import copy
import io
import json
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
C=runpy.run_path(str(ROOT/'skills/codeclew/scripts/author_citations.py'))
R=runpy.run_path(str(ROOT/'skills/codeclew/scripts/reader_markdown_repair.py'),init_globals={'CITATIONS':C})
F=runpy.run_path(str(ROOT/'scripts/test_author_citations.py'))


class ReaderMarkdownRepairTest(unittest.TestCase):
    old='A saved response completes every background task.'
    new='A saved response does not establish background completion.'
    control='A duplicate event skips refresh and still reaches acknowledgement.'
    schedule='A scheduled scan refreshes expired places.'

    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.root=Path(self.tmp.name)
        self.manifest,self.view,_=F['fixture'](self.root);self.handle=self.view['records'][0]['cite'];self.counter=0
        self.text=self.old+'\r\n\r\n'+self.control+'\r\n\r\n'+self.schedule+'\r\n'
        self.md=self.root/'draft.md';self.md.write_bytes(self.text.encode())
        claims=[{'id':key,'text':value,'evidence':[self.handle]} for key,value in [('target',self.old),('control',self.control),('schedule',self.schedule)]]
        self.claims=self.root/'claims.json';self.claims.write_text(C['encode']({'claims':claims}))
        self.base=self.root/'base';C['compile_map']([self.manifest],self.md,self.claims,self.base)
        previous=json.loads((self.base/'sources.json').read_text())
        self.spec={'schema':'codeclew-reader-repair/1.0','baseMarkdownDigest':previous['markdownDigest'],
            'baseMapDigest':previous['mapDigest'],'replacements':[{'claim':'target','old':self.old,'new':self.new}]}
        self.patch=self.root/'patch.json';self.patch.write_text(C['encode'](self.spec))
        # Thin invocation of the existing shipped renderer, not another parser.
        self.renderer=self.root/'render.cjs'
        self.renderer.write_text('''const fs=require('node:fs'),path=require('node:path');
const md=require('''+json.dumps(str(ROOT/'crates/clew/assets/documentation/markdown.js'))+''');
const [input,output,mode,base,oldText,newText,control]=process.argv.slice(2);
let text=fs.readFileSync(path.join(input,'index.md'),'utf8'),html=md.render(text);
if(path.basename(input)==='candidate'){
 if(mode==='sidecar')html=md.render(base);
 if(mode==='append')html=md.render(base+'\\n\\n'+newText);
 if(mode==='comment')html=md.render(base)+'<!-- '+newText+' -->';
 if(mode==='hidden')html=html.replace(md.render(newText),md.render(newText).replace('<p>','<p hidden>'));
 if(mode==='flatten')html=html.replaceAll('<code>','').replaceAll('</code>','');
 if(mode==='unrelated')html=html.replace(control,'Unrelated changed behavior.');
}
fs.mkdirSync(output);fs.writeFileSync(path.join(output,'index.html'),'<main>'+html+'</main>');
''')

    def command(self,mode='good'):
        self.counter+=1;path=self.root/('renderer-'+str(self.counter)+'.json')
        path.write_text(json.dumps(['node',str(self.renderer),'{input}','{output}',mode,self.text,self.old,self.new,self.control]))
        return path

    def apply(self,*,mode='good',spec=None,manifests=None,mapping=None):
        self.counter+=1;out=self.root/('repair-'+str(self.counter));patch_path=self.patch
        if spec is not None:
            patch_path=self.root/('patch-'+str(self.counter)+'.json');patch_path.write_text(C['encode'](spec))
        command=self.command(mode);calls=[];native_run=subprocess.run
        def renderer_only(argv,**kw):
            self.assertEqual(Path(argv[0]).name,'node');calls.append(argv);return native_run(argv,**kw)
        with patch('subprocess.run',side_effect=renderer_only):
            result=R['repair'](manifests or [self.manifest],self.base,patch_path,command,out,mapping=mapping)
        return result,out,calls

    def formatted_case(self):
        # Same valid native fixture and explicit mapping as ordinary bind.
        _,_,_,entries=C['load'](self.manifest);entry=entries[self.handle]
        config={entry['source']['service']:{'repository':'https://example.invalid/repo',
            'urlTemplate':'https://example.invalid/repo/blob/{revision}/{path}#L{startLine}-L{endLine}'}}
        mapping=self.root/'repositories.json';mapping.write_text(C['encode'](config));url=C['mapped_source'](entry,config)['url']
        self.old='The `saved_value` completes every task. [Source]('+url+')'
        self.new='The `saved_value` does not establish background completion. [Source]('+url+')'
        self.text=self.old+'\r\n\r\n'+self.control+'\r\n\r\n'+self.schedule+'\r\n';self.md.write_bytes(self.text.encode())
        claims=json.loads(self.claims.read_text());claims['claims'][0]['text']=self.old;self.claims.write_text(C['encode'](claims))
        self.base=self.root/'formatted-base';C['compile_map']([self.manifest],self.md,self.claims,self.base,mapping=mapping)
        previous=json.loads((self.base/'sources.json').read_text());self.spec.update(baseMarkdownDigest=previous['markdownDigest'],baseMapDigest=previous['mapDigest'],
            replacements=[{'claim':'target','old':self.old,'new':self.new}]);self.patch.write_text(C['encode'](self.spec));return mapping,url

    def test_correct_replacement_preserves_markdown_bindings_and_rendered_controls(self):
        originals={p.name:p.read_bytes() for p in self.base.iterdir()}
        result,out,calls=self.apply()
        self.assertEqual(result['status'],'ADDRESSED_REPAIR_RENDERED');self.assertEqual(len(calls),2)
        self.assertFalse(result['semanticTruthAssessed']);self.assertEqual(result['sourceAcquisitionCommandsIssued'],0)
        self.assertEqual((out/'candidate/index.md').read_bytes(),self.text.replace(self.old,self.new).encode())
        before=json.loads((self.base/'sources.json').read_text());after=json.loads((out/'candidate/sources.json').read_text())
        for old,new in zip(before['claims'],after['claims']):
            self.assertEqual(old['sources'],new['sources']);self.assertEqual(old['evidence'],new['evidence'])
            if old['id']!='target':self.assertEqual(old['text'],new['text'])
        self.assertEqual({p.name:p.read_bytes() for p in self.base.iterdir()},originals)
        self.assertEqual(R['check'](out)['status'],'ADDRESSED_REPAIR_RENDERED')

    def test_hidden_main_ancestor_is_unsupported(self):
        before='<div hidden><main><p>'+self.old+'</p></main></div>'
        after=before.replace(self.old,self.new)
        with self.assertRaises(C['Stop']) as error:
            R['rendered_check'](before,after,[{'claim':'target','old':self.old,'new':self.new}])
        self.assertEqual(error.exception.status,'UNSUPPORTED_RENDERED_MAIN_ANCESTORS')

    def test_inline_code_and_bound_source_links_survive_at_original_position(self):
        mapping,url=self.formatted_case();original=(self.base/'index.md').read_bytes()
        result,out,calls=self.apply(mapping=mapping)
        self.assertEqual(len(calls),4);self.assertEqual(result['status'],'ADDRESSED_REPAIR_RENDERED')
        self.assertEqual((out/'candidate/index.md').read_bytes(),original.replace(self.old.encode(),self.new.encode()))
        html=(out/'rendered/index.html').read_text();self.assertIn('<code>saved_value</code>',html);self.assertIn('href="'+url+'"',html)
        checks=json.loads((out/'candidate/checks.json').read_text());self.assertEqual(checks['sourceLinks'],[{'url':url}]);self.assertEqual(checks['linkMapping'],'COMPLETE')
        self.assertEqual(R['check'](out)['status'],'ADDRESSED_REPAIR_RENDERED')
        reference=out/'targets/0-new/rendered/index.html';reference.write_text(reference.read_text()+'<!-- changed -->')
        with self.assertRaises(C['Stop']) as error:R['check'](out)
        self.assertEqual(error.exception.status,'RENDERED_TARGET_REFERENCE_CHANGED')

    def test_formatted_repair_preserves_existing_code_and_link_structure(self):
        mapping,url=self.formatted_case()
        for new,status in [(self.new.replace('`saved_value`','saved_value'),'EXISTING_INLINE_CODE_CHANGED'),
                           (self.new.replace('[Source]','[Other]'),'EXISTING_INLINE_LINKS_CHANGED'),
                           (self.new.replace('[Source]('+url+')',''),'EXISTING_INLINE_LINKS_CHANGED')]:
            spec=copy.deepcopy(self.spec);spec['replacements'][0]['new']=new.rstrip()
            with self.subTest(new=new),self.assertRaises(C['Stop']) as error:self.apply(spec=spec,mapping=mapping)
            self.assertEqual(error.exception.status,status)
        for mode in ['sidecar','append','comment','hidden','flatten','unrelated']:
            with self.subTest(mode=mode),self.assertRaises(C['Stop']):self.apply(mode=mode,mapping=mapping)

    def test_formatted_target_rejects_unbound_source_link_or_unsupported_markup(self):
        mapping,url=self.formatted_case();spec=copy.deepcopy(self.spec);spec['replacements'][0]['new']=self.new.replace(url,url+'-foreign')
        with patch('subprocess.run',side_effect=AssertionError('Must reject before rendering')):
            with self.assertRaises(C['Stop']) as error:self.apply(spec=spec,mapping=mapping)
        self.assertEqual(error.exception.status,'SOURCE_LINK_NOT_BOUND_TO_SELECTED_SPAN')
        spec['replacements'][0]['new']=self.new.replace('background','*background*')
        with self.assertRaises(C['Stop']) as error:self.apply(spec=spec,mapping=mapping)
        self.assertEqual(error.exception.status,'UNSUPPORTED_RENDERED_TARGET_LAYOUT')

    def test_full_existing_bind_accepts_append_and_comment_but_not_addressed_completion(self):
        source=json.loads(self.claims.read_text())
        source['claims'][0]['text']=self.new;request=self.root/'revised-claims.json';request.write_text(C['encode'](source))
        for label,text in [('append',self.text+'\n'+self.new),('comment',self.text+'\n<!-- '+self.new+' -->')]:
            md=self.root/(label+'.md');md.write_text(text);out=self.root/(label+'-bound')
            result=C['compile_map']([self.manifest],md,request,out,previous=self.base/'sources.json')
            self.assertEqual(result['status'],'MECHANICALLY_VALID');self.assertFalse(result['semanticTruthAssessed'])
            with self.assertRaises(C['Stop']) as caught:R['check'](out)
            self.assertEqual(caught.exception.status,'MISSING_ADDRESSED_REPAIR_STATE')

    def test_sidecar_append_comment_hidden_and_unrelated_renderer_outputs_fail(self):
        for mode in ['sidecar','append','comment','hidden','unrelated']:
            with self.subTest(mode=mode),self.assertRaises(C['Stop']):self.apply(mode=mode)

    def test_post_repair_sidecar_only_append_comment_and_out_of_scope_changes_fail(self):
        _,original,_=self.apply()
        for label,text in [('sidecar',self.text),('append',self.text+'\n'+self.new),
            ('comment',self.text+'\n<!-- '+self.new+' -->'),('unrelated',self.text.replace(self.old,self.new).replace(self.control,'Changed control.'))]:
            out=self.root/('tamper-'+label);shutil.copytree(original,out);(out/'candidate/index.md').write_text(text)
            with self.assertRaises(C['Stop']) as error:R['check'](out)
            self.assertEqual(error.exception.status,'UNADDRESSED_OR_OUT_OF_SCOPE_MARKDOWN_EDIT')

    def test_stale_unchanged_hidden_and_multiple_block_replacements_fail(self):
        cases=[]
        stale=copy.deepcopy(self.spec);stale['baseMarkdownDigest']='sha256:wrong';cases.append(stale)
        stale_map=copy.deepcopy(self.spec);stale_map['baseMapDigest']='sha256:wrong';cases.append(stale_map)
        for new in [self.old,'<!-- '+self.new+' -->',self.old+'\n\n'+self.new]:
            spec=copy.deepcopy(self.spec);spec['replacements'][0]['new']=new;cases.append(spec)
        for spec in cases:
            with self.subTest(spec=spec),self.assertRaises(C['Stop']):self.apply(spec=spec)

    def test_ambiguous_old_paragraph_and_overlapping_claim_targets_fail(self):
        ambiguous=self.text+'\r\n'+self.old
        md=self.root/'ambiguous.md';md.write_text(ambiguous);request=json.loads(self.claims.read_text());request['claims'][0]['occurrence']=1
        file=self.root/'ambiguous.json';file.write_text(C['encode'](request));base=self.root/'ambiguous-base'
        C['compile_map']([self.manifest],md,file,base);previous=json.loads((base/'sources.json').read_text())
        spec={**self.spec,'baseMarkdownDigest':previous['markdownDigest'],'baseMapDigest':previous['mapDigest']}
        with self.assertRaises(C['Stop']) as error:R['addressed'](ambiguous,previous,spec)
        self.assertEqual(error.exception.status,'CLAIM_LOCATION_MISSING_OR_AMBIGUOUS')
        request=json.loads(self.claims.read_text());request['claims'].append({**request['claims'][0],'id':'second-target'})
        file=self.root/'overlap.json';file.write_text(C['encode'](request));base=self.root/'overlap-base';C['compile_map']([self.manifest],self.md,file,base)
        previous=json.loads((base/'sources.json').read_text());spec={**self.spec,'baseMapDigest':previous['mapDigest'],
            'replacements':self.spec['replacements']+[{'claim':'second-target','old':self.old,'new':'A second supplied correction.'}]}
        with self.assertRaises(C['Stop']) as error:R['addressed'](self.text,previous,spec)
        self.assertEqual(error.exception.status,'OVERLAPPING_REPAIR_TARGETS')

    def test_foreign_or_missing_evidence_fails_before_rendering(self):
        foreign,_,_=F['fixture'](self.root,'foreign',service='other',work='work-b')
        with patch('subprocess.run',side_effect=AssertionError('No renderer before evidence validation')):
            with self.assertRaises(C['Stop']):R['repair']([foreign],self.base,self.patch,self.command(),self.root/'foreign-repair')
        missing=self.root/'missing.json'
        with self.assertRaises(OSError):R['repair']([missing],self.base,self.patch,self.command(),self.root/'missing-repair')

    def test_check_rejects_rendered_or_binding_tampering(self):
        _,out,_=self.apply();html=out/'rendered/index.html';original=html.read_bytes();html.write_bytes(original+b'<!-- changed -->')
        with self.assertRaises(C['Stop']) as error:R['check'](out)
        self.assertEqual(error.exception.status,'RENDERED_ARTIFACT_CHANGED');html.write_bytes(original)
        source=out/'candidate/sources.json';value=json.loads(source.read_text());value['claims'][0]['text']='Other.';source.write_text(C['encode'](value))
        with self.assertRaises(C['Stop']) as error:R['check'](out)
        self.assertEqual(error.exception.status,'REPAIR_BINDINGS_CHANGED')

    def test_cli_records_failures_and_never_overwrites_prior_artifacts(self):
        spec=copy.deepcopy(self.spec);spec['baseMarkdownDigest']='wrong';bad=self.root/'bad.json';bad.write_text(C['encode'](spec));out=self.root/'cli-out'
        argv=['repair','--manifest',str(self.manifest),'--base',str(self.base),'--patch',str(bad),
              '--renderer-command',str(self.command()),'--output-dir',str(out)]
        with contextlib.redirect_stdout(io.StringIO()):self.assertEqual(C['main'](argv),2)
        result=json.loads((out/'result.json').read_text());self.assertEqual(result['status'],'STALE_REPAIR_BASE')
        self.assertEqual((out/'patch.original.json').read_bytes(),bad.read_bytes());inventory={p.name:p.read_bytes() for p in out.iterdir()}
        with contextlib.redirect_stdout(io.StringIO()):self.assertEqual(C['main'](argv),2)
        self.assertEqual({p.name:p.read_bytes() for p in out.iterdir()},inventory)


if __name__=='__main__':unittest.main()
