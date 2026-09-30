#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
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


if __name__ == "__main__":
    unittest.main()
