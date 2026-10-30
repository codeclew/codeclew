#!/usr/bin/env python3
"""Owned write-time fragments, current byte requests and unchanged native gates."""
import contextlib
import copy
import io
import json
import os
from pathlib import Path
import runpy
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
M = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/page_citations.py'))
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))


class PageComposeTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.serial = 0
        self.manifest, view, _ = F['fixture'](self.root)
        self.handle = view['records'][0]['cite']
        self.original = {'pages': [
            {'page': 'guide.md', 'fragments': [
                {'text': '# Guide π🙂\r\n\r\n'},
                {'id': 'prose', 'text': 'Repeated.\r\n', 'evidence': [self.handle]},
                {'text': '\r\n[Other](other.html)\r\n\r\n'},
                {'id': 'table', 'text': '| Field | Meaning |\r\n| --- | --- |\r\n| π | `x <= 10` |\r\n', 'evidence': [self.handle]},
                {'text': '\r\n'},
                {'id': 'code', 'text': '```text\r\n[[claim-not-a-marker]] {text: "π🙂"}\r\n```\r\n', 'evidence': [self.handle]},
                {'text': '\r\n'},
                {'id': 'again', 'text': 'Repeated.\r\n', 'evidence': [self.handle]}]},
            {'page': 'other.md', 'fragments': [{'text': '# Other\n\n'},
                {'id': 'prose', 'text': '- First\n- Second\n', 'evidence': [self.handle]}]}]}

    def compose(self, original=None):
        self.serial += 1; self.input = self.root / f'original-{self.serial}.json'; self.output = self.root / f'composed-{self.serial}'
        self.input.write_text(json.dumps(self.original if original is None else original, ensure_ascii=False))
        with patch('subprocess.run', side_effect=AssertionError('Producer cannot render, acquire proof or call models')):
            result = M['compose'](self.input, self.output)
        return result

    def refused(self, original, status=None):
        with self.assertRaises(M['Stop']) as error: self.compose(original)
        if status: self.assertEqual(error.exception.status, status)
        self.assertFalse(self.output.exists())

    def test_exact_bytes_gaps_scopes_ids_and_native_maps_equal_manual_route(self):
        result = self.compose(); output = self.output
        self.assertEqual((result['status'], result['pages'], result['claims']), ('PAGES_COMPOSED', 2, 5))
        self.assertFalse(result['nativeProofChecked']); self.assertFalse(result['semanticTruthAssessed'])
        self.assertEqual((output / 'original.json').read_bytes(), self.input.read_bytes())
        request = json.loads((output / 'request.json').read_bytes())
        manual_pages = self.root / 'manual'; manual_pages.mkdir(); manual_specs = []
        for page, spec in zip(self.original['pages'], request['pages']):
            raw = ''.join(f['text'] for f in page['fragments']).encode()
            self.assertEqual((output / 'markdown' / page['page']).read_bytes(), raw)
            (manual_pages / page['page']).write_bytes(raw)
            # Independent manual span control: find the known literal occurrence.
            claims = []; offset = 0
            for fragment in page['fragments']:
                body = fragment['text'].encode()
                if 'id' in fragment:
                    position = raw.index(body, offset)
                    claims.append({'id': fragment['id'], 'evidence': fragment['evidence'], 'startByte': position, 'endByte': position + len(body)})
                offset += len(body)
            expected = {'page': page['page'], 'markdownDigest': M['sha'](raw), 'claims': claims}
            self.assertEqual(spec, expected); manual_specs.append(expected)
        manual = self.root / 'manual.json'; manual.write_text(json.dumps({'schema': M['REQUEST'], 'pages': manual_specs}))
        left, right = self.root / 'left', self.root / 'right'
        M['bind']([self.manifest], output / 'markdown', output / 'request.json', left)
        M['bind']([self.manifest], manual_pages, manual, right)
        for folder in ('markdown', 'maps', 'selectors', 'checks'):
            a = {str(p.relative_to(left)): p.read_bytes() for p in (left / folder).rglob('*') if p.is_file()}
            b = {str(p.relative_to(right)): p.read_bytes() for p in (right / folder).rglob('*') if p.is_file()}
            self.assertEqual(a, b)
        self.assertEqual(M['check']([self.manifest], left / 'index.json')['claims'], 5)

    def test_malformed_annotations_ids_paths_and_empty_spans_publish_nothing(self):
        bad = [({}, 'INVALID_COMPOSE_INPUT'), ({'pages': []}, 'INVALID_COMPOSE_INPUT')]
        for fragment, status in [({'text': 'Claim', 'id': 'c'}, 'UNPAIRED_COMPOSE_ANNOTATION'),
                                 ({'text': 'Claim', 'evidence': [self.handle]}, 'UNPAIRED_COMPOSE_ANNOTATION'),
                                 ({'text': ' ', 'id': 'c', 'evidence': [self.handle]}, 'EMPTY_LOCATION_CLAIM'),
                                 ({'text': '', 'id': 'c', 'evidence': [self.handle]}, 'INVALID_BYTE_RANGE'),
                                 ({'text': 'Claim', 'id': [], 'evidence': [self.handle]}, 'INVALID_OR_DUPLICATE_PAGE_CLAIM'),
                                 ({'text': 'Claim', 'id': 'c', 'evidence': [self.handle, self.handle]}, 'EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE'),
                                 ({'text': 'Claim', 'id': 'c', 'evidence': 'wrong'}, 'EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE'),
                                 ({'text': 'Claim', 'id': 'c', 'evidence': [' ']}, 'EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE'),
                                 ({'text': 3}, 'INVALID_COMPOSE_FRAGMENT'),
                                 ({'text': 'Claim', 'children': []}, 'INVALID_COMPOSE_FRAGMENT')]:
            value = copy.deepcopy(self.original); value['pages'][1]['fragments'] = [fragment]; bad.append((value, status))
        for path in ('../bad.md', '/bad.md', 'a\\b.md', 'x/../bad.md', 'C:/bad.md', 'name:stream.md'):
            value = copy.deepcopy(self.original); value['pages'][1]['page'] = path; bad.append((value, 'INVALID_PAGE_PATH'))
        for original, status in bad:
            with self.subTest(status=status): self.refused(original, status)
        value = copy.deepcopy(self.original); value['pages'][0]['fragments'][-1]['id'] = 'prose'
        self.refused(value, 'INVALID_OR_DUPLICATE_PAGE_CLAIM')
        for name in ('guide.md', 'GUIDE.md', 'guide.md/other.md'):
            value = copy.deepcopy(self.original); value['pages'][1]['page'] = name; self.refused(value, 'PAGE_PATH_COLLISION')

    def test_selected_boundary_cannot_split_crlf_and_limits_refuse_before_write(self):
        value = {'pages': [{'page': 'a.md', 'fragments': [
            {'text': 'selected\r', 'id': 'c', 'evidence': [self.handle]}, {'text': '\n'}]}]}
        self.refused(value, 'SELECTION_SPLITS_CRLF')
        with patch.dict(M['compose'].__globals__, {'MAX_FILE': 10}): self.refused(self.original, 'PAGE_BYTES_LIMIT')
        with patch.dict(M['compose'].__globals__, {'MAX_PAGES': 1}): self.refused(self.original, 'INVALID_COMPOSE_INPUT')
        with patch.dict(M['compose'].__globals__, {'MAX_TOTAL': 10}): self.refused(self.original, 'PAGE_BYTES_LIMIT')
        with patch.dict(M['compose'].__globals__, {'MAX_CLAIMS': 3}): self.refused(self.original, 'PAGE_CLAIMS_LIMIT')

    def test_existing_human_output_input_and_failure_are_preserved(self):
        self.compose(); output = self.output; edited = output / 'markdown/guide.md'; edited.write_text('Human edit')
        before = {str(p): p.read_bytes() for p in output.rglob('*') if p.is_file()}
        with self.assertRaises(M['Stop']) as error: M['compose'](self.input, output)
        self.assertEqual(error.exception.status, 'OUTPUT_ALREADY_EXISTS')
        self.assertEqual(before, {str(p): p.read_bytes() for p in output.rglob('*') if p.is_file()})
        empty = self.root / 'already-empty'; empty.mkdir()
        with self.assertRaises(M['Stop']) as error: M['compose'](self.input, empty)
        self.assertEqual(error.exception.status, 'OUTPUT_ALREADY_EXISTS'); self.assertTrue(empty.is_dir())
        original_bytes = self.input.read_bytes(); fresh = self.root / 'failed'
        with patch('os.rename', side_effect=OSError('owned publication failure')), self.assertRaises(OSError):
            M['compose'](self.input, fresh)
        self.assertFalse(fresh.exists()); self.assertEqual(self.input.read_bytes(), original_bytes)
        self.assertFalse(list(self.root.glob('.clew-compose-*')))
        def human_race(source, target):
            target.mkdir(exist_ok=True)
            (target / 'human.md').write_bytes(b'human-created'); raise OSError('concurrent edit')
        with patch('os.rename', side_effect=human_race), self.assertRaises(OSError): M['compose'](self.input, fresh)
        self.assertEqual(list(p.name for p in fresh.iterdir()), ['human.md'])

    def test_stale_markdown_and_corrupt_proof_still_refuse_native_binding(self):
        self.compose(); output = self.output; page = output / 'markdown/guide.md'; original = page.read_bytes()
        page.write_bytes(original + b'Human edit')
        with self.assertRaises(M['Stop']) as error:
            M['bind']([self.manifest], output / 'markdown', output / 'request.json', self.root / 'stale')
        self.assertEqual(error.exception.status, 'STALE_PAGE_DIGEST'); page.write_bytes(original)
        proof = self.manifest.parent / 'part.stdout.json'; proof.write_bytes(proof.read_bytes() + b' ')
        with self.assertRaises(M['Stop']) as error:
            M['bind']([self.manifest], output / 'markdown', output / 'request.json', self.root / 'corrupt')
        self.assertEqual(error.exception.status, 'REUSE_NATIVE_STREAM_DIGEST_MISMATCH')

    def test_publication_obeys_documented_windows_and_posix_rename_contracts(self):
        rename = os.rename
        calls = []
        def windows_rename(source, target):
            calls.append(('nt', target.exists()))
            if target.exists() or target.is_symlink():
                raise FileExistsError('Windows rename refuses any existing target')
            return rename(source, target)
        def posix_rename(source, target):
            calls.append(('posix', target.exists()))
            self.assertTrue(target.is_dir())
            self.assertEqual(list(target.iterdir()), [])
            if os.name == 'nt':  # Emulate empty-directory replacement on an NT host.
                target.rmdir()
            return rename(source, target)
        for platform, operation in [('nt', windows_rename), ('posix', posix_rename)]:
            with self.subTest(platform=platform), patch.dict(M['compose'].__globals__, {'os': SimpleNamespace(name=platform, rename=operation)}):
                result = self.compose()
                self.assertEqual(result['status'], 'PAGES_COMPOSED')
                self.assertTrue((self.output / 'request.json').is_file())
                self.assertEqual((self.output / 'markdown/guide.md').read_bytes(),
                                 ''.join(f['text'] for f in self.original['pages'][0]['fragments']).encode())
        self.assertEqual(calls, [('nt', False), ('posix', True)])

    def test_windows_concurrent_target_refusal_preserves_empty_human_and_symlink_outputs(self):
        self.compose(); original = self.input
        for kind in ('empty', 'human', 'symlink'):
            target = self.root / ('windows-race-' + kind)
            outside = self.root / ('external-' + kind); outside.mkdir()
            def race(source, destination):
                if kind == 'symlink': destination.symlink_to(outside, target_is_directory=True)
                else:
                    destination.mkdir()
                    if kind == 'human': (destination / 'human.md').write_bytes(b'Human bytes')
                raise FileExistsError('Windows rename refuses concurrent target')
            with self.subTest(kind=kind), patch.dict(M['compose'].__globals__, {'os': SimpleNamespace(name='nt', rename=race)}):
                with self.assertRaises(FileExistsError): M['compose'](original, target)
            if kind == 'symlink': self.assertTrue(target.is_symlink()); self.assertEqual(list(outside.iterdir()), [])
            elif kind == 'human': self.assertEqual((target / 'human.md').read_bytes(), b'Human bytes')
            else: self.assertEqual(list(target.iterdir()), [])
            self.assertFalse((target / 'request.json').exists())
        def failed(source, destination): raise OSError('before atomic publication')
        target = self.root / 'windows-failure'
        with patch.dict(M['compose'].__globals__, {'os': SimpleNamespace(name='nt', rename=failed)}), self.assertRaises(OSError):
            M['compose'](original, target)
        self.assertFalse(target.exists()); self.assertFalse(list(self.root.glob('.clew-compose-*')))

    def test_previous_index_removal_and_rebinding_guards_are_not_bypassed(self):
        self.compose(); old = self.output; bound = self.root / 'bound'
        M['bind']([self.manifest], old / 'markdown', old / 'request.json', bound)
        value = copy.deepcopy(self.original); value['pages'][0]['fragments'][-1] = {'text': 'Repeated.\r\n'}
        self.compose(value)
        with self.assertRaises(M['Stop']) as error:
            M['bind']([self.manifest], self.output / 'markdown', self.output / 'request.json', self.root / 'missing-removal', previous=bound / 'index.json')
        self.assertEqual(error.exception.status, 'EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED')
        value['pages'][0]['remove'] = ['again']; self.compose(value)
        updated = self.root / 'updated'
        M['bind']([self.manifest], self.output / 'markdown', self.output / 'request.json', updated, previous=bound / 'index.json')
        self.assertEqual(M['check']([self.manifest], updated / 'index.json')['claims'], 4)
        other, view, _ = F['fixture'](self.root, 'foreign', service='other', work='work-b')
        value = copy.deepcopy(self.original); value['pages'][0]['fragments'][1]['evidence'] = [view['records'][0]['cite']]
        self.compose(value)
        with self.assertRaises(M['Stop']) as error:
            M['bind']([self.manifest, other], self.output / 'markdown', self.output / 'request.json', self.root / 'missing-rebind', previous=bound / 'index.json')
        self.assertEqual(error.exception.status, 'EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED')
        value['pages'][0]['replace'] = ['prose']; self.compose(value)
        M['bind']([self.manifest, other], self.output / 'markdown', self.output / 'request.json', self.root / 'rebound', previous=bound / 'index.json')

    def test_duplicate_json_keys_cli_errors_and_summary_do_not_echo_prose(self):
        path = self.root / 'bad.json'; path.write_text('{"pages":[],"pages":[]}')
        output = self.root / 'bad-output'; stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            self.assertEqual(M['main'](['compose', '--input', str(path), '--output-dir', str(output)]), 2)
        self.assertEqual(json.loads(stream.getvalue())['status'], 'INVALID_COMPOSE_INPUT'); self.assertFalse(output.exists())
        result = self.compose(); self.assertNotIn('Repeated.', M['encode'](result)); self.assertNotIn('markdownDigest', result)

    def test_unpaired_annotation_locates_missing_field_without_echo_or_repair(self):
        private_text = '[Private next page](private-domain.md) π🙂\n\n'
        for missing in ('id', 'evidence'):
            with self.subTest(missing=missing):
                value = copy.deepcopy(self.original)
                fragment = {'id': 'private-claim-id', 'text': private_text, 'evidence': [self.handle]}
                del fragment[missing]
                value['pages'][1]['fragments'].append(fragment)
                path = self.root / f'unpaired-{missing}.json'
                before = json.dumps(value, ensure_ascii=False).encode(); path.write_bytes(before)
                output = self.root / f'unpaired-{missing}-output'; stream = io.StringIO()
                with contextlib.redirect_stdout(stream), patch('subprocess.run', side_effect=AssertionError('No acquisition or models')):
                    code = M['main'](['compose', '--input', str(path), '--output-dir', str(output)])
                self.assertEqual(code, 2)
                result = json.loads(stream.getvalue()); details = result['details']
                self.assertEqual(result['status'], 'UNPAIRED_COMPOSE_ANNOTATION')
                self.assertEqual(details['page'], 'other.md')
                self.assertEqual(details['fragmentIndex'], 2)
                self.assertEqual(details['fragmentPointer'], '/pages/1/fragments/2')
                self.assertEqual(details['missingFields'], [missing])
                self.assertIn('Plain headings/navigation: use {text}', details['nextAction'])
                self.assertIn('provide {id,text,evidence}', details['nextAction'])
                for private in (private_text, 'private-domain.md', 'private-claim-id', self.handle):
                    self.assertNotIn(private, stream.getvalue())
                self.assertEqual(path.read_bytes(), before)
                self.assertFalse(output.exists())

    def test_plain_navigation_omits_both_annotation_fields_and_stays_unbound(self):
        value = copy.deepcopy(self.original)
        navigation = {'text': '[Next page](other.md)\n\n'}
        value['pages'][0]['fragments'].insert(1, navigation)
        before = copy.deepcopy(value); result = self.compose(value)
        self.assertEqual(result['claims'], 5)
        request = json.loads((self.output / 'request.json').read_bytes())
        self.assertEqual([c['id'] for c in request['pages'][0]['claims']], ['prose', 'table', 'code', 'again'])
        self.assertIn(navigation['text'].encode(), (self.output / 'markdown/guide.md').read_bytes())
        self.assertEqual(value, before)


class OriginalUpdateTest(unittest.TestCase):
    setUp = PageComposeTest.setUp
    compose = PageComposeTest.compose

    def update(self, edits, original=None, version=None):
        self.compose(original); self.base = self.input; self.base_bytes = self.base.read_bytes()
        self.edits = self.root / f'edits-{self.serial}.json'
        self.edits.write_text(json.dumps({'baseSHA256': version or M['sha'](self.base_bytes)[7:], 'edits': edits}))
        self.updated = self.root / f'updated-{self.serial}'
        with patch('subprocess.run', side_effect=AssertionError('No proof, source, renderer or models in an original edit')):
            return M['update_original'](self.base, self.edits, self.updated)

    def refused(self, edits, status, **options):
        with self.assertRaises(M['Stop']) as error: self.update(edits, **options)
        self.assertEqual(error.exception.status, status); self.assertFalse(self.updated.exists())
        self.assertEqual(self.base.read_bytes(), self.base_bytes)

    def test_simultaneous_splices_preserve_exact_fragments_metadata_and_native_maps(self):
        original = copy.deepcopy(self.original)
        original['pages'][0].update(remove=['historical'], replace=['historical-rebind'])
        revised = {**original['pages'][0]['fragments'][1], 'text': 'Revised π🙂\r\n'}
        new = {'id': 'added', 'text': 'Explicit new claim.\r\n', 'evidence': [self.handle]}
        edits = [
            {'page': 'guide.md', 'start': 6, 'delete': 1, 'fragments': []},
            {'page': 'guide.md', 'start': 2, 'delete': 1, 'fragments': [{'text': '\r\n## Reordered\r\n\r\n'}, new]},
            {'page': 'guide.md', 'start': 1, 'delete': 1, 'fragments': [revised]},
            {'page': 'guide.md', 'start': 8, 'delete': 0, 'fragments': [{'text': '\r\n'}]}]
        result = self.update(edits, original); updated = json.loads((self.updated / 'original.json').read_bytes())
        expected = copy.deepcopy(original)
        for edit in sorted(edits, key=lambda e: e['start'], reverse=True):
            a = edit['start']; expected['pages'][0]['fragments'][a:a + edit['delete']] = edit['fragments']
        self.assertEqual(updated, expected); self.assertEqual(updated['pages'][1], original['pages'][1])
        for page in expected['pages']:
            self.assertEqual((self.updated / 'markdown' / page['page']).read_bytes(), ''.join(f['text'] for f in page['fragments']).encode())
        self.assertEqual(result['originalSHA256'], M['sha']((self.updated / 'original.json').read_bytes())[7:])
        self.assertEqual((self.updated / 'edits.json').read_bytes(), self.edits.read_bytes())
        self.assertEqual(len(json.loads((self.updated / 'update.json').read_bytes())['changes']), 4)
        # Metadata is retained, not silently consumed. Native bind still owns it.
        self.assertEqual(json.loads((self.updated / 'request.json').read_bytes())['pages'][0]['remove'], ['historical'])
        expected['pages'][0].pop('remove'); expected['pages'][0].pop('replace')
        self.compose(expected); clean = self.output
        direct = self.root / 'direct'; M['bind']([self.manifest], clean / 'markdown', clean / 'request.json', direct)
        original['pages'][0].pop('remove'); original['pages'][0].pop('replace')
        self.compose(original); base = self.output; old = self.root / 'old'
        M['bind']([self.manifest], base / 'markdown', base / 'request.json', old)
        request = json.loads((self.updated / 'request.json').read_bytes()); request['pages'][0].pop('remove'); request['pages'][0].pop('replace')
        request_path = self.root / 'selected.json'; request_path.write_text(json.dumps(request))
        bound = self.root / 'bound'; M['bind']([self.manifest], self.updated / 'markdown', request_path, bound, previous=old / 'index.json')
        self.assertEqual(M['check']([self.manifest], bound / 'index.json')['claims'], 6)
        for folder in ('markdown', 'maps', 'checks'):
            self.assertEqual({str(p.relative_to(bound)): p.read_bytes() for p in (bound / folder).rglob('*') if p.is_file()},
                             {str(p.relative_to(direct)): p.read_bytes() for p in (direct / folder).rglob('*') if p.is_file()})

    def test_stale_ambiguous_overlapping_and_invalid_edits_publish_nothing(self):
        valid = {'page': 'guide.md', 'start': 0, 'delete': 1, 'fragments': [{'text': '# Revised\r\n\r\n'}]}
        self.refused([valid], 'STALE_ORIGINAL_VERSION', version='0' * 64)
        for edits, status in [([], 'INVALID_ORIGINAL_EDITS'),
            ([valid, valid], 'OVERLAPPING_ORIGINAL_EDITS'),
            ([{**valid, 'delete': 3}, {**valid, 'start': 2}], 'OVERLAPPING_ORIGINAL_EDITS'),
            ([{**valid, 'page': 'missing.md'}], 'UNKNOWN_ORIGINAL_PAGE'),
            ([{**valid, 'page': '../bad.md'}], 'INVALID_PAGE_PATH'),
            ([{**valid, 'start': True}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'start': -1}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'start': 9}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'delete': 9}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'delete': -1}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'delete': 0, 'fragments': []}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'fragments': None}], 'INVALID_ORIGINAL_EDIT'),
            ([{**valid, 'fragments': [None]}], 'INVALID_COMPOSE_FRAGMENT'),
            ([{**valid, 'text': 'unknown field'}], 'INVALID_ORIGINAL_EDIT')]:
            with self.subTest(edits=edits): self.refused(edits, status)

    def test_existing_bindings_cannot_disappear_or_change_and_new_ids_are_explicit(self):
        target = self.original['pages'][0]['fragments'][1]
        edit = {'page': 'guide.md', 'start': 1, 'delete': 1, 'fragments': []}
        self.refused([edit], 'ORIGINAL_BINDINGS_CHANGED')
        self.refused([{**edit, 'fragments': [{**target, 'evidence': ['different-explicit-handle']}]}], 'ORIGINAL_BINDINGS_CHANGED')
        self.refused([{**edit, 'fragments': [{'text': 'Unbound replacement'}]}], 'ORIGINAL_BINDINGS_CHANGED')
        self.refused([{**edit, 'fragments': [{**target, 'id': 'renamed'}]}], 'ORIGINAL_BINDINGS_CHANGED')
        self.refused([{**edit, 'delete': 0, 'fragments': [target]}], 'INVALID_OR_DUPLICATE_PAGE_CLAIM')
        self.refused([{**edit, 'fragments': [{**target, 'text': '  '}]}], 'EMPTY_LOCATION_CLAIM')

    def test_invalid_base_duplicate_json_keys_size_and_symlinks_refuse(self):
        self.compose(); base = self.input; edits = self.root / 'edits.json'; output = self.root / 'absent'
        invalid = copy.deepcopy(self.original); invalid['pages'][0]['fragments'][1]['evidence'] = []
        base.write_text(json.dumps(invalid)); edits.write_text(json.dumps({'baseSHA256': M['sha'](base.read_bytes())[7:],
            'edits': [{'page': 'guide.md', 'start': 1, 'delete': 1, 'fragments': []}]}))
        with self.assertRaises(M['Stop']) as error: M['update_original'](base, edits, output)
        self.assertEqual(error.exception.status, 'EMPTY_OR_AMBIGUOUS_CLAIM_EVIDENCE')
        base.write_text('{"pages":[],"pages":[]}')
        with self.assertRaises(M['Stop']): M['update_original'](base, edits, output)
        base.write_text(json.dumps(self.original))
        edits.write_text('{"baseSHA256":"x","baseSHA256":"y","edits":[]}')
        with self.assertRaises(M['Stop']): M['update_original'](base, edits, output)
        edits.write_text(json.dumps({'baseSHA256': M['sha'](base.read_bytes())[7:], 'edits': []}))
        with patch.dict(M['update_original'].__globals__, {'MAX_MAP': 10}), self.assertRaises(M['Stop']):
            M['update_original'](base, edits, output)
        link = self.root / 'symlink.json'; link.symlink_to(base)
        with self.assertRaises(M['Stop']): M['update_original'](link, edits, output)
        self.assertFalse(output.exists())

    def test_fresh_atomic_output_and_human_input_preservation(self):
        edit = {'page': 'guide.md', 'start': 0, 'delete': 1, 'fragments': [{'text': '# Revised\r\n\r\n'}]}
        self.update([edit]); (self.updated / 'human.md').write_bytes(b'human')
        before = {str(p): p.read_bytes() for p in self.updated.rglob('*') if p.is_file()}
        with self.assertRaises(M['Stop']): M['update_original'](self.base, self.edits, self.updated)
        self.assertEqual(before, {str(p): p.read_bytes() for p in self.updated.rglob('*') if p.is_file()})
        output = self.root / 'failed'
        with patch('os.rename', side_effect=OSError('publication failed')), self.assertRaises(OSError):
            M['update_original'](self.base, self.edits, output)
        self.assertFalse(output.exists()); self.assertFalse(list(self.root.glob('.clew-compose-*')))
        self.assertEqual(self.base.read_bytes(), self.base_bytes)

    def test_cli_summary_is_bounded_and_full_receipt_is_local(self):
        original = copy.deepcopy(self.original)
        original['pages'][0]['fragments'] = [{'text': '# Gap\n'} for _ in range(20)] + original['pages'][0]['fragments'][1:]
        edits = [{'page': 'guide.md', 'start': i, 'delete': 1, 'fragments': [{'text': '# Private new heading\n'}]} for i in range(20)]
        self.update(edits, original); output = self.root / 'cli'; stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            code = M['main'](['update-original', '--input', str(self.base), '--edits', str(self.edits), '--output-dir', str(output)])
        self.assertEqual(code, 0); result = json.loads(stream.getvalue())
        self.assertEqual((len(result['changes']), result['omittedChanges']), (16, 4))
        self.assertNotIn('Private new heading', stream.getvalue())
        self.assertEqual(len(json.loads((output / 'update.json').read_bytes())['changes']), 20)
        # Raw-file identity, including whitespace, is the version contract.
        self.base.write_bytes(self.base_bytes + b' '); failed = self.root / 'stale-cli'; stream = io.StringIO()
        with contextlib.redirect_stdout(stream):
            code = M['main'](['update-original', '--input', str(self.base), '--edits', str(self.edits), '--output-dir', str(failed)])
        self.assertEqual(code, 2); self.assertEqual(json.loads(stream.getvalue())['status'], 'STALE_ORIGINAL_VERSION')
        self.assertFalse(failed.exists())


if __name__ == '__main__': unittest.main()
