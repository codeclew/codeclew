#!/usr/bin/env python3
"""Focused helper contracts with a caller-trusted engine test double.

Real engine reload/mixed export is a separate recorded neutral qualification;
these fixtures do not claim engine admission or test producer authenticity.
"""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))


class NativeCitationsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.C = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/author_citations.py'))
        self.M = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/page_citations.py'))
        self.clew = self.root / 'trusted-clew'; self.clew.write_text('caller-trusted test double')
        self.docs = self.root / 'docs'; self.docs.mkdir()
        for module in (self.C, self.M): module['configure_engine'](self.clew, self.docs)
        self.config = {'id': 'owned', 'repositoryId': 'neutral', 'repository': 'https://example.invalid/neutral',
                       'language': 'csharp', 'profile': 'csharp-dotnet-msbuild-read-only', 'targetRef': 'main',
                       'title': 'Owned', 'compilations': ['csproj:Owned.csproj@net10.0'], 'contractFiles': []}
        self.ticket = {'schema': 'codeclew-prepared-source-evidence/1.0',
                       'reference': {'schema': 'codeclew-prepared-source-evidence/1.0', 'digest': 'sha256:' + 'd'*64, 'size': 100},
                       'service': 'owned', 'snapshot': 'saved-snapshot', 'symbol': 'actual-method',
                       'nativeFiles': 1, 'nativeMembershipScopes': 0}
        self.record = {'id': 'native-example', 'service': 'owned', 'bindingService': 'owned',
                       'file': 'config.xml', 'revision': 'a'*40, 'startLine': 1, 'endLine': 2,
                       'text': '\ufeff<value>π🙂</value>\r\n<!-- original -->\r\n',
                       'authority': 'REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS', 'url': None,
                       'membershipCoverage': 'EXPLICIT_REGISTERED_FILES_ONLY'}
        raw = self.record['text'].encode()
        import hashlib
        self.record.update(textDigest='sha256:'+hashlib.sha256(raw).hexdigest(),
                           occurrence={'snapshot': 'a'*40, 'blob': 'b'*40, 'startByte': 0, 'endByte': len(raw)})
        self.packet = {'schema': 'codeclew-docs-page/1.0', 'snapshot': 'saved-snapshot',
                       'evidenceSet': self.ticket, 'items': [{'kind': 'SOURCE_NATIVE', 'id': 'native-example', 'record': self.record}]}
        self.binding = {'snapshot': 'saved-snapshot', 'service': 'owned', 'symbol': 'actual-method', 'revision': 'a'*40,
                        'analysisConfigurationDigest': self.C['digest']({k:v for k,v in self.config.items() if k not in ('title','targetRef')})}
        self.actual = {**copy.deepcopy(self.packet), 'comparison': {'status': 'UNKNOWN',
                       'integrity': 'BOTH_IMMUTABLE_SNAPSHOT_CLOSURES_ADMITTED',
                       'baselineSnapshot': 'saved-snapshot', 'targetSnapshot': 'saved-snapshot',
                       'baselineBinding': self.binding, 'targetBinding': self.binding},
                       'registeredInputLimits': {'native': []}}
        self.input = self.root / 'prepared.json'; self.write(self.input, self.packet)
        self.calls = []
        self.mock = patch('subprocess.run', side_effect=self.engine); self.mock.start(); self.addCleanup(self.mock.stop)

    def write(self, path, value): path.write_text(self.C['encode'](value))

    def engine(self, argv, **kwargs):
        self.calls.append(argv)
        self.assertEqual(Path(argv[0]), self.clew)
        self.assertIn(str(self.docs), argv)
        self.assertNotIn('--refresh', argv)
        if argv[2:4] == ['evidence', 'compare-source']:
            self.assertEqual(argv[argv.index('--snapshot')+1], 'saved-snapshot')
            value = self.actual
        else:
            self.assertEqual(argv[2:4], ['service', 'show']); value = {'record': self.config}
        return subprocess.CompletedProcess(argv, 0, self.C['encode'](value).encode(), b'')

    def register(self, name='native'):
        out = self.root / name
        value = self.C['native_module']()['register'](self.input, out, (self.clew, self.docs))
        return out / 'citations.json', value

    def test_engine_reloaded_bytes_remain_native_in_compact_views_and_mixed_page_maps(self):
        manifest, view = self.register(); self.assertEqual(view['texts'], {})
        handle = view['records'][0]['cite']
        shown = self.C['inspect_sources'](manifest, files=['config.xml'], start_line=1, end_line=1)
        self.assertEqual(list(shown['texts'].values()), ['\ufeff<value>π🙂</value>\r\n'])
        work, work_view, _ = F['fixture'](self.root, 'work')
        page = self.root/'pages'; page.mkdir(); body='Native and Work source.\n'; (page/'one.md').write_text(body)
        request=self.root/'request.json'; self.write(request, {'schema': self.M['REQUEST'], 'pages': [
            {'page':'one.md', 'markdownDigest': self.M['sha'](body.encode()), 'claims': [
                {'id':'mixed', 'startByte':0, 'endByte':len(body), 'evidence':[handle,work_view['records'][0]['cite']]}]}]})
        out=self.root/'bound'; result=self.M['bind']([manifest,work],page,request,out)
        self.assertEqual(self.M['check']([manifest,work],out/'index.json')['status'],'PAGE_MAPS_CHECKED')
        source=json.loads((out/'maps/one.md.json').read_text())['claims'][0]['sources'][0]
        self.assertEqual(source['evidenceKind'],'SOURCE_NATIVE'); self.assertEqual(source['evidenceSet'],self.ticket)
        self.assertEqual(source['authority'],self.record['authority']); self.assertEqual(source['occurrence'],self.record['occurrence'])
        self.assertEqual(source['bindingRepository'],self.config['repository']); self.assertEqual(source['bindingService'],'owned')
        self.assertNotIn('work',source); self.assertNotIn('sourceReference',source)
        self.assertEqual(source['engineBinding']['reloadComparison']['status'],'UNKNOWN')
        self.assertEqual(result['pages'],1)

    def test_changed_source_metadata_or_text_cannot_become_engine_attestation(self):
        mutations={'text':'forged','file':'foreign.xml','revision':'c'*40,'bindingService':'other',
                   'service':'other','textDigest':'sha256:'+'0'*64,'authority':'COMPILER_EXACT','id':'foreign',
                   'occurrence':{**self.record['occurrence'],'endByte':999},'url':'https://forged.invalid'}
        for key,value in mutations.items():
            with self.subTest(key=key):
                packet=copy.deepcopy(self.packet);packet['items'][0]['record'][key]=value;self.write(self.input,packet)
                with self.assertRaises(self.C['Stop']):self.register('bad-'+key)
        packet=copy.deepcopy(self.packet);packet['items'][0]['record']['occurrence']['startByte']=False
        self.write(self.input,packet)
        with self.assertRaises(self.C['Stop']):self.register('bool-range')

    def test_changed_ticket_or_native_membership_refuses(self):
        for key,value in [('reference',{'digest':'fake'}),('snapshot','other'),('service','other'),('nativeFiles',2),('symbol','foreign')]:
            with self.subTest(key=key):
                packet=copy.deepcopy(self.packet);packet['evidenceSet'][key]=value;self.write(self.input,packet)
                with self.assertRaises(self.C['Stop']):self.register('bad-ticket-'+key)
        packet=copy.deepcopy(self.packet);packet['items'].append(packet['items'][0]);self.write(self.input,packet)
        with self.assertRaises(self.C['Stop']):self.register('duplicates')

    def test_rehashed_saved_json_and_receipts_still_require_live_caller_trusted_engine(self):
        manifest,_=self.register();prepared=manifest.parent/'prepared-native.json'
        packet=copy.deepcopy(self.packet);packet['items'][0]['record']['text']='forged';self.write(prepared,packet)
        module=self.C['native_module']();raw=prepared.read_bytes()
        self.write(manifest,module['manifest_value'](raw,packet,['native-example']))
        self.write(manifest.parent/'engine-reload.receipts.json',[{'exitCode':0,'stdout':self.C['encode'](packet)}])
        with self.assertRaises(self.C['Stop']):self.C['load'](manifest)
        self.C['configure_engine']();before=len(self.calls)
        with self.assertRaises(self.C['Stop']) as error:self.C['load'](manifest)
        self.assertEqual(error.exception.status,'NATIVE_ENGINE_VALIDATION_REQUIRED');self.assertEqual(len(self.calls),before)

    def test_repository_configuration_must_match_snapshot_and_cannot_be_reassigned(self):
        self.config['repository']='https://different.invalid/neutral'
        with self.assertRaises(self.C['Stop']) as error:self.register()
        self.assertEqual(error.exception.status,'NATIVE_REPOSITORY_CONFIGURATION_MISMATCH')

    def test_explicit_link_mapping_preserves_bound_repository_and_original_url(self):
        manifest,view=self.register();entry=self.C['load'](manifest)[3][view['records'][0]['cite']]
        template='{repository}/blob/{revision}/{path}#L{startLine}-L{endLine}'
        mapping={'owned':{'repository':self.config['repository'],
                          'urlTemplate':template.replace('{repository}',self.config['repository'])}}
        source=self.C['mapped_source'](entry,mapping)
        self.assertEqual(source['bindingRepository'],self.config['repository'])
        self.assertEqual(source['originalUrl'],None);self.assertIn('/blob/'+'a'*40+'/config.xml',source['url'])
        mapping['owned']['repository']='https://foreign.invalid'
        with self.assertRaises(self.C['Stop']) as error:self.C['mapped_source'](entry,mapping)
        self.assertEqual(error.exception.status,'NATIVE_REPOSITORY_IDENTITY_OVERRIDE')

    def test_malformed_packets_refuse_without_selecting_an_executable_from_json(self):
        for value in [[], None, {'evidenceSet': None}, {**self.packet, 'items': [None]}]:
            with self.subTest(value=value):
                self.write(self.input,value)
                with self.assertRaises(self.C['Stop']):self.register()

    def test_engine_retained_empty_file_inspection_does_not_invent_bytes(self):
        import hashlib
        record={**self.record,'text':'','textDigest':'sha256:'+hashlib.sha256(b'').hexdigest(),
                'endLine':1,'occurrence':{**self.record['occurrence'],'endByte':0}}
        self.packet['items'][0]['record']=record;self.actual['items'][0]['record']=record
        self.write(self.input,self.packet);manifest,_=self.register()
        view=self.C['inspect_sources'](manifest,files=['config.xml'])
        self.assertEqual(list(view['texts'].values()),[''])


if __name__ == '__main__': unittest.main()
