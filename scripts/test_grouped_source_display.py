#!/usr/bin/env python3
"""Saved display contracts; no source acquisition, model or native runtime."""
import contextlib
import copy
import io
import json
from pathlib import Path
import runpy
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent
F = runpy.run_path(str(ROOT / 'scripts/test_author_citations.py'))
A = F['A']
TEXT = 'first\r\nπ🙂\u2028literal\r\nthird ```tick```\r\nfourth\r\nfifth\r\nsixth\r\nseventh\r\nlast'


class GroupedSourceDisplayTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.manifest, _, _ = F['fixture'](self.root, record={**F['source'](TEXT), 'endLine': 10})
        self.request = self.root / 'request.json'
        self.request.write_text(json.dumps([{'file': 'Ledger.cs'}]))
        self.value = A['inspect_grouped'](self.manifest, self.request)
        self.saved = self.root / 'grouped.json'; self.saved.write_text(A['encode'](self.value))
        self.prior = self.root / 'prior.txt'

    def display(self, **kwargs):
        return A['display_grouped'](self.saved, **kwargs)

    def test_default_plain_source_is_exact_with_short_identity_header_and_unchanged_sidecars(self):
        before = {p: p.read_bytes() for p in self.root.rglob('*') if p.is_file()}
        text = self.display()
        self.assertIn(TEXT, text)
        cite = self.value['views'][0]['cite']
        self.assertIn('Ledger.cs:3-10 · cite:' + cite, text)
        self.assertIn('````\n' + TEXT + '\n````', text)
        self.assertNotIn('Same exact body:', text)
        self.assertEqual(before, {p: p.read_bytes() for p in before})

    def test_explicit_complete_prior_only_and_explicit_reread_restores_default_exactly(self):
        full = self.display(); self.prior.write_bytes(full.encode())
        self.assertEqual(self.display(against=[self.prior]), full)
        compact = self.display(against=[self.prior], prior_complete=True)
        self.assertNotIn(TEXT, compact); self.assertIn('Same exact body: prior1 lines ', compact)
        self.assertIn(str(self.prior), compact)
        self.assertEqual(self.display(against=[self.prior], prior_complete=True, reread=True), full)

    def test_partial_altered_unavailable_loose_or_truncated_prior_never_hides_source(self):
        full = self.display()
        variants = [full[:full.index(TEXT)+len(TEXT)//2], full.replace('fourth', 'changed'),
                    TEXT, full + '\nWarning: truncated output (17 tokens truncated)\n']
        for prior in variants:
            self.prior.write_bytes(prior.encode())
            with self.subTest(prior=prior):
                self.assertIn(TEXT, self.display(against=[self.prior], prior_complete=True))
        self.prior.unlink()
        self.assertIn(TEXT, self.display(against=[self.prior], prior_complete=True))

    def test_equal_text_different_identity_and_changed_range_remain_full(self):
        other, _, _ = F['fixture'](self.root, 'other', authority='COMPILER',
                                  record={**F['source'](TEXT), 'revision': 'rev-b', 'endLine': 10})
        value = A['inspect_grouped'](other, self.request)
        path = self.root / 'other.json'; path.write_text(A['encode'](value))
        self.prior.write_bytes(A['display_grouped'](path).encode())
        self.assertIn(TEXT, self.display(against=[self.prior], prior_complete=True))
        self.prior.write_bytes(self.display().replace('Ledger.cs:3-10', 'Ledger.cs:4-10').encode())
        self.assertIn(TEXT, self.display(against=[self.prior], prior_complete=True))

    def test_existing_whole_source_block_requires_exact_cite_and_body_and_complete_fence(self):
        cite = self.value['views'][0]['cite']
        block = f'Ledger.cs · cite:{cite}\n```text\n' + TEXT + '\n```\n'
        self.prior.write_bytes(block.encode())
        self.assertNotIn(TEXT, self.display(against=[self.prior], prior_complete=True))
        for wrong in [block.replace(cite, 'foreign:e1'), block[:-4], block.replace('Ledger.cs', 'Other.cs')]:
            self.prior.write_bytes(wrong.encode())
            self.assertIn(TEXT, self.display(against=[self.prior], prior_complete=True))

    def test_existing_whole_file_trailing_lf_and_fence_separator_keep_original_bytes(self):
        record = {**F['source']('line one\nline two\n'), 'endLine': 4}
        manifest, view, _ = F['fixture'](self.root, 'with-lf', record=record)
        value = A['inspect_grouped'](manifest, self.request)
        saved = self.root/'with-lf.json'; saved.write_text(A['encode'](value))
        cite = view['records'][0]['cite']
        self.prior.write_text(f'Ledger.cs · cite:{cite}\n```\nline one\nline two\n\n```\n')
        shown = A['display_grouped'](saved, against=[self.prior], prior_complete=True)
        self.assertIn('Same exact body:', shown); self.assertNotIn(record['text'], shown)

    def test_corrupt_saved_body_authority_or_mapping_refuses_before_printing_and_cli_is_plain(self):
        for mutation in ['body', 'authority', 'cite']:
            value = copy.deepcopy(self.value)
            if mutation == 'body': value['groups'][0]['fragments'][0]['text'] += 'changed'
            if mutation == 'authority': value['groups'][0]['authority'] = 'FOREIGN'
            if mutation == 'cite': value['views'][0]['cite'] = 'foreign:e1'
            self.saved.write_text(A['encode'](value))
            with self.subTest(mutation=mutation), self.assertRaises(A['Stop']): self.display()
        self.saved.write_text(A['encode'](self.value))
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = A['main'](['display-grouped', '--input', str(self.saved)])
        self.assertEqual(code, 0); self.assertEqual(out.getvalue(), self.display())
        self.saved.write_text('[]'); out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(A['main'](['display-grouped', '--input', str(self.saved)]), 2)
        self.assertEqual(json.loads(out.getvalue())['status'], 'INVALID_GROUPED_DISPLAY')

    def test_nonempty_source_limits_remain_visible_and_full_metadata_retained(self):
        other, _, _ = F['fixture'](self.root, 'limited', heavy=True, body='PARTIAL_BODY')
        value = A['inspect_grouped'](other, self.request)
        path = self.root / 'limited.json'; path.write_text(A['encode'](value))
        shown = A['display_grouped'](path)
        self.assertIn('CALL_TARGETS_UNRESOLVED', shown)
        self.assertIn('ORDER_LEXICAL_ONLY', shown)
        self.assertNotIn('/authority=', shown)
        self.assertIn('PARTIAL_BODY', shown)

    def test_unknown_identity_never_suppresses_even_with_matching_header(self):
        manifest, _, _ = F['fixture'](self.root, 'unknown', record={**F['source'](TEXT), 'revision': 'UNKNOWN', 'endLine': 10})
        value = A['inspect_grouped'](manifest, self.request)
        path = self.root/'unknown.json'; path.write_text(A['encode'](value))
        self.prior.write_bytes(A['display_grouped'](path).encode())
        shown = A['display_grouped'](path, against=[self.prior], prior_complete=True)
        self.assertIn(TEXT, shown); self.assertNotIn('Same exact body:', shown)

    def test_incomplete_discovery_and_material_coverage_gaps_remain_visible(self):
        refresh = runpy.run_path(str(ROOT/'scripts/test_citation_evidence.py'))['refresh']
        packet = json.loads((self.manifest.parent/'packet.json').read_bytes())
        packet.update(discoveryQueryComplete=False, coverage={'boundaries': ['MISSING_DEPENDENCY']})
        refresh(self.manifest.parent, packet)
        value = A['inspect_grouped'](self.manifest, self.request)
        self.saved.write_text(A['encode'](value))
        shown = self.display()
        self.assertIn('discovery is incomplete', shown); self.assertIn('MISSING_DEPENDENCY', shown)
        self.assertIn(TEXT, shown)


if __name__ == '__main__':
    unittest.main()
