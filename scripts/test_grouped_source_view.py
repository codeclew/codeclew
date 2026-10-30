#!/usr/bin/env python3
"""Verbatim grouped retained views and unchanged binding/export contracts."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import runpy
import shutil
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))
A = F['A']
REFRESH = runpy.run_path(str(ROOT / 'scripts/test_citation_evidence.py'))['refresh']
TEXT = 'first\r\nπ🙂\u2028literal\r\nthird\r\nfourth\r\nfifth\r\nsixth\r\nseventh\r\nlast'


class GroupedSourceViewTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.serial = 0
        self.manifest, self.legacy, _ = F['fixture'](self.root, record={**F['source'](TEXT), 'endLine': 10},
                                                   heavy=True, body='PARTIAL_BODY')

    def request(self, rows):
        self.serial += 1; path = self.root / f'views-{self.serial}.json'
        path.write_text(json.dumps(rows)); return path

    def inspect(self, rows, manifest=None):
        with patch('subprocess.run', side_effect=AssertionError('Inspection cannot acquire source or run models')):
            return A['inspect_grouped'](manifest or self.manifest, self.request(rows))

    def reconstruct(self, value):
        groups = {group['id']: group for group in value['groups']}
        return [(groups[v['group']], groups[v['group']]['fragments'][v['fragment']], v['cite']) for v in value['views']]

    def multi(self, variants):
        directory = self.root / 'combined'; directory.mkdir()
        outcomes, choices, texts, receipts = [], [], {}, []
        for number, (file, revision, authority) in enumerate(variants):
            manifest, _, _ = F['fixture'](self.root, f'part-{number}', authority=authority,
                record={**F['source'](TEXT), 'file': file, 'revision': revision, 'endLine': 10})
            origin = manifest.parent; prefix = f'source-{number}/'; shutil.copytree(origin, directory / prefix)
            part = json.loads((origin / 'packet.json').read_bytes()); outcome = copy.deepcopy(part['outcomes'][0])
            discovery = json.loads((origin / 'discovery.input.json').read_bytes())
            native = json.loads((origin / 'page.stdout.json').read_bytes())
            identity = f'declaration-{number}'
            native['items'][0]['id'] = native['items'][0]['record']['id'] = identity
            (directory / prefix / 'page.stdout.json').write_text(A['encode'](native))
            outcome['declaration']['id'] = identity; outcome['declaration']['raw']['stdout'] = prefix + 'page.stdout.json'
            for entry in outcome['sourceParts']: entry['stdout'] = prefix + entry['stdout']
            for entry in outcome['nativePages']: entry['raw'] = prefix + entry['raw']
            choices.append({**discovery['choices'][0], 'id': identity}); outcomes.append(outcome); texts.update(part['texts'])
            for receipt in json.loads((origin / 'receipts.json').read_bytes()):
                receipts.append({**receipt, 'stdout': prefix + receipt['stdout'], 'stderr': prefix + receipt['stderr']})
        packet = {**part, 'outcomes': outcomes, 'texts': texts}; discovery['choices'] = choices
        (directory / 'receipts.json').write_text(A['encode'](receipts)); REFRESH(directory, packet, discovery)
        manifest = directory / 'citations.json'; A['load'](manifest)
        return manifest

    def test_whole_disjoint_overlap_repeat_order_unicode_crlf_and_full_caveats(self):
        rows = [{'file': 'Ledger.cs', 'startLine': 9, 'endLine': 10},
                {'file': 'Ledger.cs', 'startLine': 4, 'endLine': 5},
                {'file': 'Ledger.cs', 'startLine': 5, 'endLine': 7},
                {'file': 'Ledger.cs', 'startLine': 4, 'endLine': 5}, {'file': 'Ledger.cs'}]
        before = {str(p): (p.stat().st_mtime_ns, p.read_bytes()) for p in self.manifest.parent.rglob('*') if p.is_file()}
        value = self.inspect(rows); reconstructed = self.reconstruct(value)
        self.assertEqual(len(value['groups']), 1); self.assertEqual(len(value['groups'][0]['fragments']), 4)
        self.assertEqual([v['request'] for v in value['views']], list(range(5)))
        expected = ['seventh\r\nlast', 'π🙂\u2028literal\r\nthird\r\n', 'third\r\nfourth\r\nfifth\r\n',
                    'π🙂\u2028literal\r\nthird\r\n', TEXT]
        for (_, fragment, cite), text in zip(reconstructed, expected):
            self.assertEqual(fragment['text'].encode(), text.encode())
            self.assertEqual(cite, self.legacy['records'][0]['cite'])
        self.assertEqual(reconstructed[0][0]['sourceLines'], [3, 10])
        self.assertEqual(reconstructed[0][0]['citations'][0]['bodyStatus'], 'PARTIAL_BODY')
        for key in ('manifest', 'limitations', 'coverageWarnings', 'failures', 'discoveryQueryComplete'):
            self.assertEqual(value[key], self.legacy[key])
        self.assertEqual(before, {str(p): (p.stat().st_mtime_ns, p.read_bytes()) for p in self.manifest.parent.rglob('*') if p.is_file()})

    def test_equal_text_distinct_file_revision_or_authority_never_share_identity(self):
        manifest = self.multi([('Unit.cs', 'r1', 'SOURCE_SYNTAX'), ('Other.cs', 'r1', 'SOURCE_SYNTAX'),
                               ('Unit.cs', 'r2', 'SOURCE_SYNTAX'), ('Unit.cs', 'r1', 'COMPILER')])
        _, packet, discovery, _ = A['load'](manifest)
        legacy = A['project'](packet, discovery, manifest.parent, include_reused=True, save=False)
        requests = [{'file': o['source']['file'], 'cite': r['cite']} for o, r in zip(packet['outcomes'], legacy['records'])]
        value = self.inspect(requests, manifest)
        self.assertEqual(len(value['groups']), 4)
        self.assertEqual([r[2] for r in self.reconstruct(value)], [r['cite'] for r in legacy['records']])
        self.assertEqual([g['revision'] for g in value['groups']], ['r1', 'r1', 'r2', 'r1'])
        self.assertEqual(value['groups'][-1]['authority'], 'COMPILER')
        with self.assertRaises(A['Stop']) as error: self.inspect([{'file': 'Unit.cs'}], manifest)
        self.assertEqual(error.exception.status, 'AMBIGUOUS_INSPECT_FILE')

    def test_compact_wire_reduction_does_not_drop_requested_text(self):
        rows = [{'file': 'Ledger.cs', 'startLine': a, 'endLine': b} for a, b in [(3, 4), (6, 6), (9, 10), (6, 6)]]
        value = self.inspect(rows)
        baseline = [A['inspect_sources'](self.manifest, files=['Ledger.cs'], start_line=r['startLine'], end_line=r['endLine']) for r in rows]
        for (_, fragment, cite), legacy in zip(self.reconstruct(value), baseline):
            self.assertEqual(fragment['text'], next(iter(legacy['texts'].values())))
            self.assertEqual(cite, legacy['records'][0]['cite'])
        old = ''.join(A['encode'](v) for v in baseline); new = A['encode'](value)
        self.assertLess(len(new), len(old)); self.assertLess(len(new.encode()), len(old.encode()))

    def test_shared_source_identity_keeps_distinct_citation_roles_and_partial_coverage(self):
        manifest = self.multi([('Unit.cs', 'r1', 'SOURCE_SYNTAX'), ('Unit.cs', 'r1', 'SOURCE_SYNTAX')])
        directory = manifest.parent; packet = json.loads((directory / 'packet.json').read_bytes())
        path = directory / 'source-1/page.stdout.json'; page = json.loads(path.read_bytes())
        normalized = page['items'][0]['record']['normalized']; normalized['name'] = 'secondary role'
        normalized['documentation']['bodyStatus'] = 'PARTIAL_BODY'
        path.write_text(A['encode'](page))
        declaration = packet['outcomes'][1]['declaration']
        declaration['descriptor']['name'] = 'secondary role'; declaration['bodyStatus'] = 'PARTIAL_BODY'
        declaration['limits'] = A['B']['limitations'](normalized)
        packet['discoveryQueryComplete'] = False; packet['coverage'] = {'boundaries': ['OWNED_PARTIAL']}
        REFRESH(directory, packet)
        _, checked, discovery, _ = A['load'](manifest)
        original = A['project'](checked, discovery, directory, include_reused=True, save=False)
        value = self.inspect([{'file': 'Unit.cs', 'cite': r['cite']} for r in original['records']], manifest)
        self.assertEqual(len(value['groups']), 1); self.assertEqual(len(value['groups'][0]['fragments']), 1)
        self.assertEqual(len(value['groups'][0]['citations']), 2)
        self.assertEqual(value['groups'][0]['citations'][1]['role'], 'secondary role')
        self.assertEqual(value['groups'][0]['citations'][1]['bodyStatus'], 'PARTIAL_BODY')
        self.assertFalse(value['discoveryQueryComplete'])
        self.assertEqual(value['coverageWarnings'], original['coverageWarnings'])
        self.assertEqual(value['limitations'], original['limitations'])

    def test_unknown_foreign_cite_invalid_ranges_and_malformed_requests_fail_atomically(self):
        invalid = [[], {}, [None], [{}], [{'file': 'Ledger.cs'}] * 257, [{'file': '*.cs'}], [{'file': './Ledger.cs'}],
                   [{'file': 'Ledger.cs', 'cite': 'foreign:e1'}], [{'file': 'Ledger.cs', 'startLine': 3}],
                   [{'file': 'Ledger.cs', 'startLine': None, 'endLine': 3}],
                   [{'file': 'Ledger.cs', 'startLine': True, 'endLine': 3}],
                   [{'file': 'Ledger.cs', 'startLine': 4, 'endLine': 3}],
                   [{'file': 'Ledger.cs', 'startLine': 0, 'endLine': 3}],
                   [{'file': 'Ledger.cs', 'startLine': 3, 'endLine': 11}],
                   [{'file': 'Ledger.cs', 'extra': 'ignored'}], [{'file': 'Ledger.cs'}, {'file': 'missing'}]]
        for rows in invalid:
            with self.subTest(rows=rows), self.assertRaises(A['Stop']): self.inspect(rows)
        for raw in ['[{"file":"missing","file":"Ledger.cs"}]', '[not JSON']:
            path = self.request([]); path.write_text(raw)
            stream = io.StringIO()
            with contextlib.redirect_stdout(stream):
                self.assertEqual(A['main'](['inspect-grouped', '--manifest', str(self.manifest), '--request', str(path)]), 2)
            self.assertNotIn('groups', json.loads(stream.getvalue()))
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            A['main'](['inspect-grouped', '--manifest', str(self.manifest), '--request', str(self.request([])), '--file', 'Ledger.cs'])
        self.assertEqual(error.exception.code, 2)

    def test_corrupt_unselected_proof_stale_packet_and_manifest_refuse_before_success(self):
        manifest = self.multi([('First.cs', 'r1', 'SOURCE_SYNTAX'), ('Other.cs', 'r2', 'SOURCE_SYNTAX')])
        proof = manifest.parent / 'source-1/part.stdout.json'; raw = proof.read_bytes(); proof.write_bytes(raw + b' ')
        with self.assertRaises(A['Stop']) as error: self.inspect([{'file': 'First.cs'}], manifest)
        self.assertEqual(error.exception.status, 'REUSE_NATIVE_STREAM_DIGEST_MISMATCH'); proof.write_bytes(raw)
        for name in ('packet.json', 'citations.json'):
            path = manifest.parent / name; raw = path.read_bytes(); value = json.loads(raw)
            value['id' if name == 'citations.json' else 'status'] = 'stale'; path.write_text(json.dumps(value))
            with self.assertRaises(A['Stop']): self.inspect([{'file': 'First.cs'}], manifest)
            path.write_bytes(raw)

    def test_compact_handles_roundtrip_unchanged_page_binding_and_export_and_reject_document_mutation(self):
        fixture = runpy.run_path(str(ROOT / 'scripts/test_reader_export_check.py'))['ReaderExportCheckTest']()
        fixture.setUp(); self.addCleanup(fixture.doCleanups)
        value = self.inspect([{'file': 'Ledger.cs'}], fixture.manifest)
        fixture.handle = value['views'][0]['cite']; fixture.prepare()
        result = fixture.verify(); self.assertEqual(result['status'], 'EXPORT_CHECKED')
        index = json.loads((fixture.package / 'index.json').read_bytes())
        mapped = json.loads((fixture.package / index['pages'][0]['map']).read_bytes())
        self.assertEqual(mapped['claims'][0]['evidence'], [fixture.handle])
        (fixture.authored / 'guide.md').write_text('Changed document')
        fixture.refused('AUTHORED_MARKDOWN_MISMATCH')


if __name__ == '__main__':
    unittest.main()
