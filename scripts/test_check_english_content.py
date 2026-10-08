#!/usr/bin/env python3
from __future__ import annotations

import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import unittest


MODULE_PATH = Path(__file__).resolve().with_name("check_english_content.py")
SPEC = importlib.util.spec_from_file_location("codeclew_english_content", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
english = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = english
SPEC.loader.exec_module(english)


LOCALIZED_SOURCE = "crates/clew/src/documentation/operation_answer.rs"
RUSSIAN = "".join(chr(codepoint) for codepoint in (0x041F, 0x0440, 0x0438, 0x0432, 0x0435, 0x0442))
RUSSIAN_CHAR = chr(0x044F)
RUSSIAN_IDENTIFIER = "".join(chr(codepoint) for codepoint in (0x0438, 0x043C, 0x044F))


class EnglishContentTest(unittest.TestCase):
    def rejected(self, content: str, path: str = LOCALIZED_SOURCE) -> list[int]:
        return english.rejected_cyrillic_line_numbers(path, content)

    def test_only_complete_strings_in_the_explicit_source_are_localized(self) -> None:
        source = r'''let ordinary = "@RU@ \" reader // /*";
let multiline = "first line
@RU@";
let raw = r##"@RU@ " # /* still inside */"##;
let byte_raw = br#"@RU@"#;
'''.replace("@RU@", RUSSIAN)
        self.assertEqual([], self.rejected(source))
        self.assertEqual([1, 3, 4, 5], self.rejected(source, "crates/clew/src/other.rs"))
        self.assertEqual([1, 3, 4, 5], self.rejected(source, "docs/guide.md"))

    def test_comments_doc_comments_characters_and_identifiers_remain_visible(self) -> None:
        source = '''let message = "English"; // "@RU@"
/* outer "@RU@" /* nested @RU@ */ still @RU@ */
/// @RU@ documentation for readers.
fn @IDENT@() {}
'''.replace("@RU@", RUSSIAN).replace("@IDENT@", RUSSIAN_IDENTIFIER)
        self.assertEqual([1, 2, 3, 4], self.rejected(source))

    def test_character_literals_and_lifetimes_are_not_string_exemptions(self) -> None:
        source = (
            "let marker = '" + RUSSIAN_CHAR + "';\n"
            'let text: &\'static str = "@RU@";\n'
        ).replace("@RU@", RUSSIAN)
        self.assertEqual([1], self.rejected(source))

    def test_doc_attribute_strings_remain_repository_prose(self) -> None:
        source = (
            '#[doc = /* English comment */ "@RU@ API"]\n'
            '#[doc /* English comment */ = "@RU@ module"]\n'
            '#[doc = concat!("@RU@", " English")]\n'
            '#![doc = "@RU@ module"]\n'
            '#[cfg_attr(anyflag, doc = "@RU@ conditional docs")]\n'
            '#[cfg_attr(anyflag, doc = concat!("@RU@", " conditional docs"))]\n'
            'let label = "@RU@";\n'
        ).replace("@RU@", RUSSIAN)
        self.assertEqual([1, 2, 3, 4, 5, 6], self.rejected(source))

    def test_masking_preserves_line_numbers_for_mixed_content(self) -> None:
        source = '''let label = "@RU@";
let marker = '@CHAR@';
let text: &'static str = r###"@RU@ "## remains raw"###;
let another = "@RU@";
let @IDENT@ = 1;
'''.replace("@RU@", RUSSIAN).replace("@CHAR@", RUSSIAN_CHAR).replace(
            "@IDENT@", RUSSIAN_IDENTIFIER
        )
        self.assertEqual([2, 5], self.rejected(source))

    def test_unterminated_ordinary_and_raw_strings_cannot_hide_later_text(self) -> None:
        ordinary = 'let broken = "@RU@\nfn @IDENT@() {}\n'.replace(
            "@RU@", RUSSIAN
        ).replace("@IDENT@", RUSSIAN_IDENTIFIER)
        raw = 'let broken = r#"open "@RU@"\nfn @IDENT@() {}\n'.replace(
            "@RU@", RUSSIAN
        ).replace("@IDENT@", RUSSIAN_IDENTIFIER)
        self.assertEqual([1, 2], self.rejected(ordinary))
        self.assertEqual([1, 2], self.rejected(raw))


class GeneratedPublicationEnglishTest(unittest.TestCase):
    bundle = "a" * 64
    prefix = "site/examples/codeclew-source/docs/"
    service_path = prefix + "generated/" + bundle + "/services/example.json"
    html_path = service_path.removesuffix(".json") + ".html"
    bindings_path = prefix + "generated/" + bundle + "/bindings.json"

    def source(self) -> dict:
        text = 'fn label() { let message = "' + RUSSIAN + '"; }\n'
        return {
            "id": "s1", "service": "example", "file": "src/example.rs",
            "revision": "frozen", "authority": "EXACT_SNAPSHOT_TEXT", "text": text,
            "textDigest": "sha256:" + hashlib.sha256(text.encode()).hexdigest(),
            "evidenceDigest": "sha256:" + "b" * 64,
            "occurrence": {"snapshot": "sha256:" + "c" * 64,
                           "startByte": 0, "endByte": len(text.encode())},
        }

    def document(self) -> dict:
        return {"renderer": "codeclew-documentation-html/1.16",
                "title": "Example", "operations": [{"summary": "English narrative"}],
                "sources": {"s1": self.source()},
                "operationSources": {"op1": {"s1": self.source()}}}

    def rejected(self, value: dict, path: str | None = None, escaped: bool = False) -> list[int]:
        content = json.dumps(value, ensure_ascii=escaped, indent=2)
        return english.rejected_cyrillic_line_numbers(path or self.service_path, content)

    def test_exact_retained_sources_allowed_without_exempting_narratives(self) -> None:
        value = self.document()
        self.assertEqual([], self.rejected(value))
        # Identical code quoted as narrative is still checked at its own path.
        value["operations"][0]["summary"] = value["sources"]["s1"]["text"]
        self.assertTrue(self.rejected(value))
        self.assertTrue(self.rejected(value, escaped=True))
        value["operations"][0]["summary"] = "English"
        value["sources"]["s1"]["file"] = RUSSIAN
        self.assertTrue(self.rejected(value))

    def test_current_publication_keeps_exact_sources_but_checks_prose(self) -> None:
        path = "site/examples/current-workflow/docs/generated/" + self.bundle + "/services/example.json"
        value = self.document()
        self.assertEqual([], self.rejected(value, path))
        value["operations"][0]["summary"] = RUSSIAN
        self.assertTrue(self.rejected(value, path))
        value["operations"][0]["summary"] = "English"
        value["sources"]["s1"]["textDigest"] = "sha256:" + "0" * 64
        self.assertTrue(self.rejected(value, path))

    def test_source_authority_identity_and_bytes_are_required(self) -> None:
        mutations = [
            lambda source: source.update(textDigest="sha256:" + "0" * 64),
            lambda source: source.update(authority="DECLARED"),
            lambda source: source.update(id="another"),
            lambda source: source["occurrence"].update(endByte=1),
            lambda source: source.update(evidenceDigest="unbound"),
            lambda source: source.update(text=source["text"] + "\ud800"),
        ]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                value = self.document()
                mutate(value["sources"]["s1"])
                self.assertTrue(self.rejected(value))
        value = self.document()
        value["narrative"] = {"text": RUSSIAN}
        self.assertTrue(self.rejected(value))
        value = self.document()
        value["renderer"] = "unknown"
        self.assertTrue(self.rejected(value))

    def test_bindings_retained_sources_only_and_no_broad_path_exemption(self) -> None:
        value = {"schema": "codeclew-documentation-bindings/1.4",
                 "retainedSources": {"s1": self.source()}}
        self.assertEqual([], self.rejected(value, self.bindings_path))
        value["narratives"] = [{"text": RUSSIAN}]
        self.assertTrue(self.rejected(value, self.bindings_path))
        for path in ("site/documentation.html", "site/examples/other/docs/example.json",
                     self.prefix + "other.json"):
            with self.subTest(path=path):
                self.assertTrue(self.rejected(self.document(), path))
        value = {"schema": "unknown", "retainedSources": {"s1": self.source()}}
        self.assertTrue(self.rejected(value, self.bindings_path))

    def test_actual_embedded_localization_assets_and_source_payload(self) -> None:
        app = (english.ROOT / "crates/clew/assets/documentation/app.js").read_text()
        reader = (english.ROOT / "crates/clew/assets/documentation/reader.js").read_text()
        payload = json.dumps(self.document(), ensure_ascii=False).replace("<", "\\u003c")
        content = ('<html><body><p>English narrative</p>\n'
                   '<script type="application/json" id="document-data">' + payload + '</script>\n'
                   '<script>' + app + '</script>\n<script>' + reader + '</script></body></html>')
        self.assertEqual([], english.rejected_cyrillic_line_numbers(self.html_path, content))
        # Extra script prose is not an approved renderer asset.
        added = content.replace('</body>', '<script>const summary="' + RUSSIAN + '";</script></body>')
        self.assertTrue(english.rejected_cyrillic_line_numbers(self.html_path, added))
        self.assertTrue(english.rejected_cyrillic_line_numbers("site/documentation.html", content))
        changed = content.replace('English narrative', RUSSIAN, 1)
        self.assertEqual([1], english.rejected_cyrillic_line_numbers(self.html_path, changed))
        payload = json.dumps({"summary": RUSSIAN}, ensure_ascii=True)
        self.assertTrue(english.rejected_cyrillic_line_numbers(
            self.html_path, '<script type="application/json" id="document-data">' + payload + '</script>'))

    def test_frozen_released_reader_survives_asset_updates_but_not_script_changes(self) -> None:
        paths = (
            "site/examples/codeclew-source/docs/generated/"
            "7c84c5220aa3bd456acf143043d657955a73bd4ff685f1f3322e4a258f89c86c/"
            "services/clew-public-workflow.html",
            "site/examples/current-workflow/docs/generated/"
            "bf1cf2b887bd2cd30062cc00d6da3ac18722ddd76f8310d1df1006f37f7310ab/"
            "services/cli-documentation.html",
            "site/examples/clew-starter/docs/generated/"
            "fe231260a788793c8125f84cd34c23eaac4a1bec360efd7fd570cf4126d109d0/"
            "services/clew-starter.html",
            "site/examples/current-workflow/docs/generated/"
            "55925743c99a25e7caedad2327b2998160fa0e8c1f2d9a3215f0a046faf9fd88/"
            "services/cli-documentation.html",
        )
        for relative in paths:
            with self.subTest(publication=relative):
                content = (english.ROOT / relative).read_text(encoding="utf-8")
                self.assertEqual([], english.rejected_cyrillic_line_numbers(relative, content))
                body = next(match[2] for match in english.SCRIPT.finditer(content)
                            if "application/json" not in match[1] and english.CYRILLIC.search(match[2]))
                altered = content.replace(body, body + '\nconst extraProse="' + RUSSIAN + '";', 1)
                self.assertTrue(english.rejected_cyrillic_line_numbers(relative, altered))
                changed = content.replace(body, body.replace("const", "var", 1), 1)
                self.assertTrue(english.rejected_cyrillic_line_numbers(relative, changed))

    def test_duplicate_keys_and_malformed_json_do_not_hide_prose(self) -> None:
        source = json.dumps(self.source(), ensure_ascii=False)
        content = ('{"renderer":"codeclew-documentation-html/1.16",'
                   '"sources":{"s1":' + source + '},"sources":{},"title":"English"}')
        self.assertTrue(english.rejected_cyrillic_line_numbers(self.service_path, content))
        self.assertTrue(english.rejected_cyrillic_line_numbers(self.service_path, '{"text":"' + RUSSIAN))

    def test_generated_masking_preserves_reported_line_numbers(self) -> None:
        content = json.dumps(self.document(), ensure_ascii=False, indent=2)
        content = content.replace('"English narrative"', '"' + RUSSIAN + '"')
        expected = [number for number, line in enumerate(content.splitlines(), 1)
                    if '"summary":' in line]
        self.assertEqual(expected, english.rejected_cyrillic_line_numbers(self.service_path, content))


if __name__ == "__main__":
    unittest.main()
