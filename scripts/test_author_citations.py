#!/usr/bin/env python3
"""Mechanical citation contracts; no model, compiler, native build or source access."""
from __future__ import annotations
import argparse
import contextlib
import hashlib
import json
import io
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
A = runpy.run_path(str(ROOT / "skills/codeclew/scripts/author_citations.py"))
encode, digest = A["encode"], A["digest"]
P = runpy.run_path(str(ROOT / "skills/codeclew/scripts/citation_evidence.py"))
SNAPSHOT = "sha256:owned/1"


def source(text="public class Ledger { /* π🙂 */ }"):
    return {"id": "source-b", "service": "owned", "revision": "rev-a", "file": "Ledger.cs",
            "startLine": 3, "endLine": 3, "authority": "EXACT_SNAPSHOT_TEXT", "text": text,
            "textDigest": "sha256:" + hashlib.sha256(text.encode()).hexdigest()}

def declaration(ids=None, references=None):
    return {"kind": "DEPENDENCY", "id": "decl-b", "reference": "d9",
            "sourceReferences": ["s9"] if references is None else references,
            "record": {"id": "decl-b", "kind": "SYMBOL", "service": "owned", "symbol": "class:Ledger",
                "sourceIds": ["source-b"] if ids is None else ids, "digest": "sha256:descriptor",
                "normalized": {"documentation": {"bodyStatus": "DECLARATION_ONLY"}}}}

def page(*items, cursor=None, omitted=None):
    return {"schema": "codeclew-documentation-work-page/1.0", "work": "work-a",
            "subject": "service:owned", "snapshot": SNAPSHOT, "items": list(items),
            "omitted": omitted or [], "nextCursor": cursor}

def part(record=None, start=0, end=None, cursor=None, reference="s9"):
    record = record or source()
    text = record["text"].encode(); end = len(text) if end is None else end
    fragment = text[start:end].decode()
    value = {"schema": "codeclew-documentation-source-part/1.0", "work": "work-a",
        "snapshot": SNAPSHOT, "reference": reference, "sourceId": record["id"],
        "authority": "IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED", "recordDigest": digest(record),
        "source": {k: v for k, v in record.items() if k != "text"}, "startByte": start, "endByte": end,
        "totalTextBytes": len(text), "text": fragment, "fragmentDigest": "sha256:" + hashlib.sha256(fragment.encode()).hexdigest(),
        "nextCursor": cursor, "receiptType": "SOURCE_PART"}
    return {**value, "receiptDigest": digest(value)}

def fixture(root, label="read", *, service="owned", work="work-a", authority="SOURCE_SYNTAX", body=None, heavy=False, record=None):
    """Owned native protocol fixture; no acquisition helper or subprocess is loaded."""
    directory = root / label; directory.mkdir()
    selected = declaration(); record = source() if record is None else dict(record)
    selected['record']['service'] = record['service'] = service
    record['authority'] = authority
    normalized = selected['record']['normalized']; normalized['authority'] = authority
    if authority == 'COMPILER': normalized['scope'] = 'compilation:' + work
    normalized['documentation'] = {} if body is None else {'bodyStatus': body}
    if heavy:
        normalized['documentation']['observations'] = [
            {'authority': authority, 'boundaries': ['CALL_TARGETS_UNRESOLVED', 'ORDER_LEXICAL_ONLY'],
             'resolution': 'UNRESOLVED'} for _ in range(40)]
    row = {'kind': 'DECLARATION', 'id': selected['id'], 'reference': selected['reference'],
        'service': service, 'symbol': selected['record']['symbol'], 'revision': record['revision'],
        'sourceIds': selected['record']['sourceIds'], 'sourceReferences': selected['sourceReferences'],
        'observationDigest': selected['record']['digest']}
    discovery = {'schema': 'codeclew-auto-doc-evidence/1.0', 'kind': 'DISCOVERY',
        'scope': {'service': service, 'snapshot': SNAPSHOT}, 'work': work,
        'coverage': {}, 'choices': [row], 'queryComplete': True}
    discovery_bytes = encode(discovery).encode()
    (directory / 'discovery.input.json').write_bytes(discovery_bytes)
    native_page = page(selected); native_page.update(work=work, subject='service:' + service)
    native_part = part(record); native_part['work'] = work
    native_part['receiptDigest'] = digest({k:v for k,v in native_part.items() if k != 'receiptDigest'})
    receipts = []
    for label, verb, value in [('page', 'read', native_page), ('part', 'read-part', native_part)]:
        stdout, stderr = label + '.stdout.json', label + '.stderr.log'
        raw = encode(value).encode(); (directory / stdout).write_bytes(raw); (directory / stderr).write_bytes(b'')
        receipts.append({'command': ['/supported/clew', 'docs', 'work', verb, '--work', work],
            'exitCode': 0, 'stdout': stdout, 'stderr': stderr,
            'stdoutSha256': hashlib.sha256(raw).hexdigest(),
            'stderrSha256': hashlib.sha256(b'').hexdigest()})
    bound = {'id': row['id'], 'symbol': row['symbol'], 'service': service,
        'sourceIds': row['sourceIds'], 'sourceReferences': row['sourceReferences'],
        'selectedSourceId': record['id'], 'selectedSourceReference': 's9',
        'observationDigest': row['observationDigest'], 'bodyStatus': normalized['documentation'].get('bodyStatus'),
        'descriptor': {k: normalized[k] for k in ('scope', 'kind', 'name', 'ownerIdentity', 'declarationKind', 'syntaxKind') if k in normalized},
        'limits': P['limitations'](normalized), 'raw': {'stdout': 'page.stdout.json', 'pointer': '/items/0'}}
    outcome = {'selection': {'declarationId': row['id']}, 'status': 'DECLARATION_SOURCE_RETURNED',
        'declaration': bound, 'source': {k:v for k,v in record.items() if k != 'text'},
        'textKey': record['textDigest'], 'sourceRecordDigest': digest(record),
        'sourceParts': [{'stdout':'part.stdout.json', **{k:native_part[k] for k in
            ('startByte','endByte','recordDigest','receiptDigest','authority','nextCursor')}}],
        'nativePages': [{'raw':'page.stdout.json', 'metadata':{k:v for k,v in native_page.items() if k != 'items'}, 'coverage':[]}],
        'sourceReadReused': False, 'evidenceReused': False}
    packet = {'schema': 'codeclew-auto-doc-evidence/1.0', 'kind':'READ', 'status':'SELECTED_SOURCES_READY',
        'scope':discovery['scope'], 'work':work, 'coverage':{}, 'discoveryQueryComplete':True,
        'outcomes':[outcome], 'texts':{record['textDigest']:record['text']},
        'inputPacketDigest':'sha256:' + hashlib.sha256(discovery_bytes).hexdigest()}
    (directory/'packet.json').write_text(encode(packet))
    (directory/'receipts.json').write_text(encode(receipts))
    (directory/'read-summary.json').write_text(encode({'packetDigest':digest(packet)}))
    view = A['project'](packet, discovery, directory)
    return directory/'citations.json', view, argparse.Namespace(output_dir=directory)


class AuthorCitationsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.serial = 0

    def bind(self, manifests, claims, markdown, *, repositories=None, previous=None, remove=None, replace=None):
        self.serial += 1; stem = self.root / ('bind-' + str(self.serial)); out = stem.with_suffix('.out')
        md = stem.with_suffix('.md'); md.write_bytes(markdown.encode())
        request = stem.with_suffix('.json'); request.write_text(encode({'claims': claims,
            **({'remove':remove} if remove is not None else {}), **({'replace':replace} if replace is not None else {})}))
        mapping = None
        if repositories is not None:
            mapping = stem.with_suffix('.repos.json'); mapping.write_text(encode(repositories))
        with patch('subprocess.run', side_effect=AssertionError('No native or external commands allowed')):
            result = A['compile_map'](manifests, md, request, out, mapping=mapping, previous=previous)
        return result, json.loads((out / 'sources.json').read_text()), out

    def assert_failure(self, manifests, claims, markdown, status=None, **kwargs):
        with self.assertRaises(A['Stop']) as error:self.bind(manifests, claims, markdown, **kwargs)
        if status:self.assertEqual(error.exception.status, status)
        self.assertFalse((self.root / ('bind-' + str(self.serial) + '.out')).exists())

    def test_multiple_services_works_equal_text_and_authorities_keep_exact_provenance(self):
        a, av, _ = fixture(self.root, authority='COMPILER', body='PARTIAL_BODY')
        b, bv, _ = fixture(self.root, 'second', service='other', work='work-b', authority='SOURCE_SYNTAX')
        self.assertNotEqual(av['records'][0]['cite'], bv['records'][0]['cite'])
        self.assertEqual(av['records'][0]['scope'],'compilation:work-a')
        self.assertEqual(av['texts'], bv['texts'])
        self.assertIsNone(bv['records'][0]['bodyStatus']); self.assertEqual(av['records'][0]['bodyStatus'], 'PARTIAL_BODY')
        text = 'This unsupported sentence says every request always succeeds.'
        result, value, out = self.bind([a,b], [{'id':'claim','text':text,'evidence':[av['records'][0]['cite'],bv['records'][0]['cite']]}], text + '\r\n')
        self.assertFalse(result['semanticTruthAssessed']);self.assertEqual(result['status'], 'MECHANICALLY_VALID')
        self.assertFalse(value['nativePublicationApproval']);self.assertFalse(value['runtimeVerified'])
        self.assertEqual((out/'index.md').read_bytes(), (text+'\r\n').encode())
        refs = value['claims'][0]['sources'];self.assertEqual({s['service'] for s in refs},{'owned','other'})
        self.assertEqual({s['authority'] for s in refs},{'COMPILER','SOURCE_SYNTAX'})
        self.assertEqual({s['work'] for s in refs},{'work-a','work-b'})
        self.assertEqual(len(result['missingLinkMappings']),2)

    def test_native_metadata_edits_are_rejected_even_with_coedited_packet_checksum(self):
        for field, bad in [('id','wrong'),('revision','other'),('startLine',1),('endLine',100),('file','other.cs'),('textDigest','bad')]:
            with self.subTest(field=field):
                manifest, view, _ = fixture(self.root, field)
                directory = manifest.parent; packet = json.loads((directory/'packet.json').read_text())
                packet['outcomes'][0]['source'][field] = bad
                (directory/'packet.json').write_text(encode(packet))
                summary=json.loads((directory/'read-summary.json').read_text());summary['packetDigest']=digest(packet)
                (directory/'read-summary.json').write_text(encode(summary))
                discovery=json.loads((directory/'discovery.input.json').read_text())
                manifest.write_text(encode(A['manifest_value'](packet,discovery)))
                current=json.loads(manifest.read_text());handle='m'+current['id'].split(':')[1][:12]+':e1'
                self.assert_failure([manifest],[{'id':'c','text':'Sentence.','evidence':[handle]}],'Sentence.')

    def test_scope_reference_missing_and_corrupt_proofs_fail_without_outputs(self):
        for field in ['service','snapshot','work','sourceReference','proofMissing','proofCorrupt','manifest']:
            with self.subTest(field=field):
                manifest, view, _ = fixture(self.root, field);directory=manifest.parent
                packet=json.loads((directory/'packet.json').read_text())
                if field in ['proofMissing','proofCorrupt']:
                    receipt=json.loads((directory/'receipts.json').read_text())[-1];p=directory/receipt['stdout']
                    if field=='proofMissing':p.unlink()
                    else:p.write_text('corrupt')
                elif field=='manifest':manifest.write_text('{}')
                else:
                    if field=='work':packet['work']='other'
                    elif field=='sourceReference':packet['outcomes'][0]['declaration']['selectedSourceReference']='s99'
                    else:packet['scope'][field]='other'
                    (directory/'packet.json').write_text(encode(packet))
                self.assert_failure([manifest],[{'id':'c','text':'Sentence.','evidence':[view['records'][0]['cite']]}],'Sentence.')

    def test_unknown_foreign_empty_and_ambiguous_handles_fail(self):
        manifest, view, _ = fixture(self.root);handle=view['records'][0]['cite']
        for handles in [[],['unknown'],[handle,handle]]:
            self.assert_failure([manifest],[{'id':'c','text':'Sentence.','evidence':handles}],'Sentence.')
        other, foreign, _ = fixture(self.root, 'foreign', service='other',work='work-c')
        self.assert_failure([manifest],[{'id':'c','text':'Sentence.','evidence':[foreign['records'][0]['cite']]}],'Sentence.')
        self.assert_failure([manifest,manifest],[{'id':'c','text':'Sentence.','evidence':[handle]}],'Sentence.','AMBIGUOUS_CITATION_MANIFEST')

    def test_locations_and_explicit_source_links_preserve_other_bytes(self):
        manifest, view, _ = fixture(self.root);handle=view['records'][0]['cite'];claim={'id':'c','text':'Sentence.','evidence':[handle]}
        self.assert_failure([manifest],[claim],'Other.','CLAIM_LOCATION_MISSING_OR_AMBIGUOUS')
        self.assert_failure([manifest],[claim],'Sentence. Sentence.','CLAIM_LOCATION_MISSING_OR_AMBIGUOUS')
        result,value,_=self.bind([manifest],[{**claim,'occurrence':2}],'Sentence. Sentence.')
        self.assertEqual(value['claims'][0]['location']['startByte'],10)
        mapping={'owned':{'repository':'https://example.invalid/owned','urlTemplate':'https://example.invalid/owned/blob/{revision}/{path}#L{startLine}-L{endLine}'}}
        markdown='Sentence. [source](cite:'+handle+')\r\nOther bytes: π🙂\r\n'
        self.assert_failure([manifest],[claim],markdown,'SOURCE_LINK_MAPPING_MISSING')
        result,value,out=self.bind([manifest],[claim],markdown,repositories=mapping)
        expected=markdown.replace('cite:'+handle,'https://example.invalid/owned/blob/rev-a/Ledger.cs#L3-L3')
        self.assertEqual((out/'index.md').read_bytes(),expected.encode());self.assertFalse(result['markdownBytesUnchanged'])
        self.assert_failure([manifest],[claim],'Sentence. [source](https://example.invalid/owned/blob/rev-a/Ledger.cs#L1-L90)',
                            'SOURCE_LINK_NOT_BOUND_TO_SELECTED_SPAN',repositories=mapping)
        self.assert_failure([manifest],[claim],'Sentence. [source][ref]\n[ref]: cite:'+handle,'UNSUPPORTED_CITATION_LINK_SYNTAX',repositories=mapping)

    def test_repair_requires_explicit_removal_or_rebind_and_retains_originals(self):
        manifest,view,_=fixture(self.root);other,ov,_=fixture(self.root,'other',service='other',work='work-b')
        h=view['records'][0]['cite'];foreign=ov['records'][0]['cite']
        claims=[{'id':'one','text':'First.','evidence':[h]},{'id':'two','text':'Second.','evidence':[h]}]
        _,_,original=self.bind([manifest],claims,'First. Second.');previous=original/'sources.json'
        inventory=lambda:{str(p):p.read_bytes() for p in original.iterdir()};before=inventory()
        self.assert_failure([manifest],claims[:1],'First.','EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED',previous=previous)
        self.bind([manifest],claims[:1],'First.',previous=previous,remove=['two'])
        new=[{**claims[0],'evidence':[foreign]},claims[1]]
        self.assert_failure([manifest,other],new,'First. Second.','EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED',previous=previous)
        self.bind([manifest,other],new,'First. Second.',previous=previous,replace=['one'])
        self.assertEqual(inventory(),before)

    def test_metadata_heavy_contract_reduces_non_source_payload_without_losing_text(self):
        manifest,view,args=fixture(self.root,heavy=True)
        packet=json.loads((manifest.parent/'packet.json').read_text())
        legacy={k:packet[k] for k in ('schema','kind','status','scope','work','texts')}
        legacy['outcomes']=[{k:v for k,v in o.items() if k not in ('nativePages','sourceParts','nativeOutput')} for o in packet['outcomes']]
        source_bytes=lambda v:sum(len(encode(t).rstrip('\n').encode()) for t in v['texts'].values())
        old=len(encode(legacy).encode())-source_bytes(legacy);new=len(encode(view).encode())-source_bytes(view)
        self.assertLessEqual(new,old*0.5,(old,new));self.assertEqual(list(packet['texts'].values()),list(view['texts'].values()))
        self.assertTrue(any('CALL_TARGETS_UNRESOLVED' in w for group in view['limitations'].values() for w in group))
        A['load'](manifest)

    def test_unchanged_inspection_has_zero_native_commands(self):
        manifest,view,_=fixture(self.root);before={str(p):p.read_bytes() for p in manifest.parent.iterdir()}
        with patch('subprocess.run',side_effect=AssertionError('No native or external command')):
            _,packet,discovery,_=A['load'](manifest)
            inspected=A['project'](packet,discovery,manifest.parent,include_reused=True,save=False)
            self.assertEqual(inspected['texts'],view['texts'])
        self.assertEqual({str(p):p.read_bytes() for p in manifest.parent.iterdir()},before)

    def test_cli_inspection_preserves_original_artifacts(self):
        manifest,view,_=fixture(self.root)
        before={str(p):(p.stat().st_mtime_ns,p.read_bytes()) for p in manifest.parent.iterdir()}
        with patch('subprocess.run',side_effect=AssertionError('No native commands')),contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(A['main'](['inspect','--manifest',str(manifest)]),0)
        self.assertEqual({str(p):(p.stat().st_mtime_ns,p.read_bytes()) for p in manifest.parent.iterdir()},before)

    def inspect_cli(self, manifest, *arguments):
        output = io.StringIO(); errors = io.StringIO()
        with patch('subprocess.run', side_effect=AssertionError('Inspection cannot acquire source')):
            with contextlib.redirect_stdout(output), contextlib.redirect_stderr(errors):
                try:
                    code = A['main'](['inspect', '--manifest', str(manifest), *arguments])
                except SystemExit as error:
                    code = error.code
        return code, json.loads(output.getvalue()) if output.getvalue() else None, errors.getvalue()

    def test_bounded_inspect_preserves_whole_text_identity_and_unfiltered_behavior(self):
        text = 'first\r\nπ🙂\u2028literal\r\n\r\nlast'
        record = {**source(text), 'endLine': 6}
        manifest, original, _ = fixture(self.root, record=record)
        before = {str(p): (p.stat().st_mtime_ns, p.read_bytes()) for p in manifest.parent.iterdir()}
        code, whole, _ = self.inspect_cli(manifest, '--file', 'Ledger.cs')
        self.assertEqual(code, 0)
        self.assertEqual(list(whole['texts'].values()), [text])
        self.assertEqual(len(whole['records']), 1)
        selected = whole['records'][0]
        self.assertEqual(selected['cite'], original['records'][0]['cite'])
        self.assertEqual(selected['source'], {k: v for k, v in {**record, 'authority': 'SOURCE_SYNTAX'}.items() if k != 'text'})
        self.assertEqual(whole['manifest'], original['manifest'])
        self.assertEqual(whole['view']['startLine'], 3)
        self.assertEqual(whole['view']['endLine'], 6)
        code, unfiltered, _ = self.inspect_cli(manifest)
        self.assertEqual(code, 0); self.assertEqual(unfiltered, original)
        self.assertEqual({str(p): (p.stat().st_mtime_ns, p.read_bytes()) for p in manifest.parent.iterdir()}, before)

    def test_bounded_inspect_range_keeps_unicode_crlf_and_original_line_offsets(self):
        text = 'first\r\nπ🙂\u2028literal\r\n\r\nlast'
        record = {**source(text), 'endLine': 6}
        manifest, original, _ = fixture(self.root, record=record)
        code, view, _ = self.inspect_cli(manifest, '--file', 'Ledger.cs', '--start-line', '4', '--end-line', '5')
        self.assertEqual(code, 0)
        self.assertEqual(list(view['texts'].values()), ['π🙂\u2028literal\r\n\r\n'])
        self.assertEqual(view['view'], {'file': 'Ledger.cs', 'startLine': 4, 'endLine': 5, 'originalEvidenceUnchanged': True})
        self.assertEqual(view['records'][0]['location'], 'owned:Ledger.cs:4-5')
        self.assertEqual(view['records'][0]['cite'], original['records'][0]['cite'])
        self.assertEqual(view['records'][0]['source']['textDigest'], record['textDigest'])
        self.assertEqual(view['records'][0]['source']['startLine'], 3)
        self.assertEqual(view['records'][0]['source']['endLine'], 6)

    def test_bounded_inspect_refuses_unknown_multiple_and_invalid_ranges(self):
        manifest, _, _ = fixture(self.root)
        cases = [
            (['--file', 'Ledger'], 'UNKNOWN_INSPECT_FILE'),
            (['--file', '*.cs'], 'UNKNOWN_INSPECT_FILE'),
            (['--file', './Ledger.cs'], 'UNKNOWN_INSPECT_FILE'),
            (['--file', 'Ledger.cs', '--file', 'Other.cs'], 'SINGLE_INSPECT_FILE_REQUIRED'),
            (['--file', 'Ledger.cs', '--file', 'Ledger.cs'], 'SINGLE_INSPECT_FILE_REQUIRED'),
            (['--start-line', '3', '--end-line', '3'], 'INSPECT_FILE_REQUIRED_FOR_RANGE'),
        ]
        for bounds in [['--start-line', '3'], ['--end-line', '3'],
                       ['--start-line', '0', '--end-line', '3'],
                       ['--start-line', '4', '--end-line', '3'],
                       ['--start-line', '3', '--end-line', '4'],
                       ['--start-line', '2', '--end-line', '3']]:
            cases.append((['--file', 'Ledger.cs', *bounds], 'INVALID_INSPECT_LINE_RANGE'))
        for arguments, status in cases:
            with self.subTest(arguments=arguments):
                code, result, _ = self.inspect_cli(manifest, *arguments)
                self.assertEqual((code, result['status']), (2, status))
                self.assertNotIn('texts', result)
                self.assertFalse(result['sourceFallbackAllowed'])
        code, result, error = self.inspect_cli(manifest, '--file', 'Ledger.cs', '--start-line', 'bad', '--end-line', '3')
        self.assertEqual(code, 2); self.assertIsNone(result); self.assertIn('invalid int value', error)

    def test_bounded_inspect_refuses_ambiguous_file_after_full_proof_validation(self):
        import copy
        refresh = runpy.run_path(str(ROOT / 'scripts/test_citation_evidence.py'))['refresh']
        manifest, _, _ = fixture(self.root); directory = manifest.parent
        packet = json.loads((directory / 'packet.json').read_bytes())
        discovery = json.loads((directory / 'discovery.input.json').read_bytes())
        native = json.loads((directory / 'page.stdout.json').read_bytes())
        other = copy.deepcopy(native['items'][0]); other['id'] = other['record']['id'] = 'decl-c'
        native['items'].append(other); (directory / 'page.stdout.json').write_text(encode(native))
        outcome = copy.deepcopy(packet['outcomes'][0]); outcome['declaration']['id'] = 'decl-c'
        outcome['declaration']['raw']['pointer'] = '/items/1'; packet['outcomes'].append(outcome)
        discovery['choices'].append({**discovery['choices'][0], 'id': 'decl-c'})
        refresh(directory, packet, discovery)
        A['load'](manifest)
        code, result, _ = self.inspect_cli(manifest, '--file', 'Ledger.cs')
        self.assertEqual((code, result['status']), (2, 'AMBIGUOUS_INSPECT_FILE'))
        self.assertNotIn('texts', result)

    def test_bounded_inspect_rejects_corrupt_packet_and_native_proof_before_selection(self):
        for label, name in [('packet', 'packet.json'), ('proof', 'part.stdout.json')]:
            with self.subTest(label=label):
                manifest, _, _ = fixture(self.root, label)
                path = manifest.parent / name; path.write_bytes(path.read_bytes() + b' ')
                if label == 'packet':
                    value = json.loads(path.read_bytes()); value['texts'][next(iter(value['texts']))] += 'changed'
                    path.write_text(encode(value))
                code, result, _ = self.inspect_cli(manifest, '--file', 'Ledger.cs')
                self.assertEqual(code, 2)
                self.assertIn(result['status'], ['REUSE_PACKET_DIGEST_MISMATCH', 'REUSE_NATIVE_STREAM_DIGEST_MISMATCH'])
                self.assertNotIn('texts', result)

    def test_bounded_inspect_refuses_empty_or_unavailable_source_line_metadata(self):
        for label, record in [('empty', source('')), ('inconsistent', {**source('one\n'), 'endLine': 4})]:
            with self.subTest(label=label):
                manifest, _, _ = fixture(self.root, label, record=record)
                code, result, _ = self.inspect_cli(manifest, '--file', 'Ledger.cs')
                self.assertEqual((code, result['status']), (2, 'INSPECT_SOURCE_LINES_UNAVAILABLE'))
                self.assertNotIn('texts', result)

    def test_incomplete_selection_is_visible_and_cannot_compile_success_map(self):
        manifest,view,args=fixture(self.root);directory=manifest.parent;packet=json.loads((directory/'packet.json').read_text())
        packet['status']='SELECTED_SOURCES_INCOMPLETE';packet['discoveryQueryComplete']=False
        packet['outcomes'].append({'selection':{'declarationId':'missing'},'status':'MISSING_DECLARATION_SOURCE','exitCode':3})
        (directory/'packet.json').write_text(encode(packet));summary=json.loads((directory/'read-summary.json').read_text())
        summary['packetDigest']=digest(packet);(directory/'read-summary.json').write_text(encode(summary))
        discovery=json.loads((directory/'discovery.input.json').read_text());projected=A['project'](packet,discovery,directory)
        self.assertEqual(projected['failures'][0]['status'],'MISSING_DECLARATION_SOURCE');self.assertFalse(projected['discoveryQueryComplete'])
        self.assert_failure([manifest],[{'id':'c','text':'Sentence.','evidence':[projected['records'][0]['cite']]}],'Sentence.','CITATION_EVIDENCE_INCOMPLETE')

    def test_equal_warning_groups_retain_all_paths_and_distinct_values(self):
        value={'one':{'boundaries':['SCOPE_AMBIGUOUS:A','NO_RUNTIME']},
               'two':{'boundaries':['SCOPE_AMBIGUOUS:A','NO_RUNTIME']},
               'three':{'boundaries':['SCOPE_AMBIGUOUS:B']}}
        groups=A['grouped_warnings'](value)
        restored={(path,json.dumps(v,ensure_ascii=False,separators=(',',':'))) for g in groups for path in g['paths'] for v in g['values']}
        self.assertEqual(restored,A['warning_leaves'](value));self.assertEqual(len(groups),2)
        self.assertEqual(len(A['grouped_warnings']({'flag':True,'count':1})),2)


if __name__=='__main__':unittest.main()
