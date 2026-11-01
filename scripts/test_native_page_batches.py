#!/usr/bin/env python3
"""Owned bulk transport fixtures, exact proof pointers and startup scaling."""
import argparse
import copy
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
R = runpy.run_path(str(ROOT / 'scripts/test_retained_files.py'))
M, A, F, encode, digest = (R[k] for k in ('M', 'A', 'F', 'encode', 'digest'))


class NativePageBatchesTest(unittest.TestCase):
    def setUp(self):
        self.fixture = R['RetainedFilesTest']()
        self.fixture.setUp(); self.addCleanup(self.fixture.doCleanups)
        for source in self.fixture.sources:
            source['evidenceDigest'] = 'sha256:owned-original'
        self.backend = self.fixture.native
        self.commands = []; self.bad = None

    def native(self, command, **kwargs):
        self.commands.append(command)
        if command[3] != 'read-batch':
            return self.backend(command, **kwargs)
        request = json.loads(Path(command[command.index('--input') + 1]).read_text())
        results = []
        for n, selection in enumerate(request['selections']):
            input_path = self.fixture.root / ('native-selection-' + str(n) + '.json')
            input_path.write_text(encode(selection))
            inner = [*command]; inner[3] = 'read'; inner[inner.index('--input') + 1] = str(input_path)
            response = self.backend(inner, **kwargs)
            value = json.loads(response.stdout)
            results.append({'selection':selection, 'error':value['error']} if response.returncode else {'selection':selection, 'page':value})
        value = {'schema':'codeclew-documentation-work-batch/1.0', 'work':'work-a', 'snapshot':F['SNAPSHOT'], 'results':results}
        if self.bad == 'selection': results[0]['selection'] = {'references':['foreign']}
        if self.bad == 'missing': results.pop()
        if self.bad == 'work': value['work'] = 'foreign'
        if self.bad == 'snapshot': value['snapshot'] = 'foreign'
        if self.bad == 'page': results[0]['page']['work'] = 'foreign'
        if self.bad == 'empty-next':
            page=results[0]['page'];page.update(items=[],omitted=[],nextCursor='new-cursor')
            page['receiptDigest']=digest({k:v for k,v in page.items() if k!='receiptDigest'})
        value['receiptDigest'] = digest(value)
        if self.bad == 'digest': value['receiptDigest'] = 'bad'
        raw = encode(value).encode()
        if self.bad == 'truncated': raw = raw[:-8]
        return subprocess.CompletedProcess(command, 0, raw, b'')

    def read(self, label, *, batch=64, reuse=None):
        f = self.fixture
        discovery = f.root / 'discovery'
        if not discovery.exists():
            f.packet('discover-files', 'discovery')
        out = f.root / label; out.mkdir()
        selection = f.root / (label + '.json')
        selection.write_text(encode([{'file':s} for s in sorted({v['file'] for v in f.sources})]))
        args = argparse.Namespace(action='read-files', clew='/supported/clew', root='/disposable/docs', service='owned',
            snapshot=F['SNAPSHOT'], work=None, packet=discovery / 'packet.json', selection=selection, output_dir=out,
            contains='', reuse_read=reuse, output_format='metadata', whole_files=False, source_union=False,
            source_batch_size=8, native_batch_size=batch)
        self.commands = []
        with patch('subprocess.run', side_effect=self.native): result, code = M['Packet'](args).run()
        return out, result, code

    def test_cross_file_pages_share_launches_and_preserve_reusable_citations(self):
        legacy, _, code = self.read('separate', batch=1); self.assertEqual(code, 0)
        fresh, _, code = self.read('bulk'); self.assertEqual(code, 0)
        self.assertEqual([c[3] for c in self.commands], ['read-batch'] * 3)  # two-row pages still drain fully
        before = json.loads((legacy / 'packet.json').read_text()); after = json.loads((fresh / 'packet.json').read_text())
        self.assertEqual(before['texts'], after['texts'])
        self.assertEqual([o['source'] for o in before['outcomes']], [o['source'] for o in after['outcomes']])
        self.assertTrue(all('#' in o['sourceBinding']['raw']['stdout'] for o in after['outcomes']))
        A['load'](fresh / 'citations.json')
        reused, _, code = self.read('reused', reuse=fresh / 'packet.json'); self.assertEqual(code, 0)
        self.assertEqual(self.commands, []); A['load'](reused / 'citations.json')

    def test_transport_refuses_changed_selection_scope_missing_digest_and_truncation(self):
        for bad in ['selection', 'missing', 'work', 'snapshot', 'page', 'digest', 'truncated', 'empty-next']:
            with self.subTest(bad=bad):
                self.bad = bad; out, _, code = self.read('bad-' + bad)
                self.assertNotEqual(code, 0)

    def test_virtual_pointer_and_edited_native_parent_fail_proof_validation(self):
        for bad in ['pointer', 'parent', 'error', 'occurrence', 'projection']:
            out, _, code = self.read('saved-' + bad); self.assertEqual(code, 0)
            A['load'](out / 'citations.json')  # previous validation must not lend trust to the next load
            packet = json.loads((out / 'packet.json').read_text()); outcome = packet['outcomes'][0]
            if bad == 'pointer': outcome['sourceBinding']['raw']['stdout'] += '0'
            else:
                proof=outcome['sourceBatch'] if bad=='projection' else outcome['sourceBinding']
                base, _, index = proof['raw']['stdout'].partition('#')
                path = out / base; value = json.loads(path.read_text()); item = value['results'][int(index)]
                if bad == 'parent': value['work'] = 'foreign'
                elif bad == 'error': item.pop('page'); item['error'] = {'message':'missing'}
                elif bad == 'projection': item['selection']['projection']='CONTEXT'
                else: item['page']['items'][0]['record']['source']['occurrence'] = {'startByte':999}
                value['receiptDigest'] = digest({k:v for k,v in value.items() if k != 'receiptDigest'})
                path.write_text(encode(value))
            # Refresh outer local hashes deliberately: exact inner row/page proof still refuses.
            C = runpy.run_path(str(ROOT / 'scripts/test_citation_evidence.py'))
            C['refresh'](out, packet)
            with self.assertRaises(A['Stop']) as error: A['load'](out / 'citations.json')
            if bad=='projection':self.assertEqual(error.exception.status,'SOURCE_BATCH_PROJECTION_CHANGED')

    def test_public_scale_keeps_all_bindings_with_bounded_native_invocations(self):
        f = self.fixture; f.page_size = 100; template = f.sources[0]; f.sources = []
        for n in range(2048):
            source = copy.deepcopy(template); text = 'class Owned' + str(n) + ' {}\n'
            source.update(id='owned-' + str(n), file='Owned' + str(n // 32) + '.java', text=text,
                textDigest='sha256:' + __import__('hashlib').sha256(text.encode()).hexdigest(), startLine=1, endLine=1)
            f.sources.append(source)
        out, _, code = self.read('scale'); self.assertEqual(code, 0)
        self.assertEqual(len(self.commands), 5)  # one navigation + four 64×8 SOURCE groups
        self.assertEqual(len(A['load'](out / 'citations.json')[3]), 2048)

    def test_reference_route_uses_bulk_rows_and_only_omitted_source_parts(self):
        module = runpy.run_path(str(ROOT / 'scripts/test_source_references.py'))
        f = module['SourceBatchTest'](); f.setUp(); self.addCleanup(f.doCleanups)
        self.fixture, self.backend = f, f.native
        f.page_size = 2; f.omit = {'s2'}
        selection = f.root / 'selection.json'
        selection.write_text(encode([{'sourceId':s['id'], 'sourceReference':r} for r,s in f.sources.items()]))
        out = f.root / 'bulk'; out.mkdir()
        args = argparse.Namespace(action='read-sources',clew='/supported/clew',root='/disposable/docs',service='owned',
            snapshot=F['SNAPSHOT'],work='work-a',contains='',selection=selection,output_dir=out,
            output_format='metadata',reuse_read=None,source_union=False,source_batch_size=8,native_batch_size=64)
        with patch('subprocess.run',side_effect=self.native): _,code=M['Packet'](args).run()
        self.assertEqual(code,0)
        _,packet,_,entries=A['load'](out/'citations.json')
        self.assertEqual(len(entries),9)
        self.assertEqual({o['sourceBinding']['reference'] for o in packet['outcomes'] if o['sourceParts']},{'s2'})
        self.assertTrue(all(c[3] in ['read-batch','read-part'] for c in self.commands))


if __name__ == '__main__':
    unittest.main()
