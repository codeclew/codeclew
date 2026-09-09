#!/usr/bin/env python3
"""Behavioral checks for deterministic initial-message preparation and explicit failures."""
import importlib.util
import json
from pathlib import Path
import stat
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("prepare_agent_context", Path(__file__).with_name("prepare_agent_context.py"))
prepare = importlib.util.module_from_spec(spec)
spec.loader.exec_module(prepare)


def packet(text="fun Root() = 1\n"):
    value = {
        "schema":"codeclew-source-packet/1.0", "status":"READY_WITH_LIMITS",
        "sessionId":"session:retained", "contextId":"context:retained", "baseRevision":"a" * 40,
        "snapshotId":"snapshot:retained", "admission":{"status":"PASS"},
        "preparation":{"modelCalls":0, "modelTokens":0, "durationMs":12},
        "generationAuthority":{"coverage":"PARTIAL", "certainty":"UNSURE", "obligations":["check-runtime"], "compilations":[]},
        "roots":[{"identifier":"Root", "status":"UNIQUE"}],
        "boundaries":{"RELATED_SOURCE_NOT_INCLUDED":1}, "analysisBoundaries":["CONDITIONAL_ANALYZER"],
        "selectionPolicy":{"maxHops":2},
        "sources":[{"file":"src/Root.kt", "startLine":1, "endLine":max(1, len(text.splitlines())),
                    "completeFile":True, "contentRef":{"schema":"codeclew-cas-object/2.0",
                        "objectSchema":"codeclew-repository-input-blob/2.0", "size":len(text.encode()),
                        "digest":prepare.digest(b"codeclew-cas/v2\0codeclew-repository-input-blob/2.0\0" + text.encode())}, "text":text,
                    "selections":[{"role":"ROOT", "authority":"COMPILER_DECLARATION", "identity":"Root", "compilation":":/main"}]}],
    }
    evidence = json.loads(json.dumps(value))
    evidence["preparation"].pop("durationMs")
    value["evidenceDigest"] = prepare.digest(prepare.canonical(evidence))
    return value


class PreparationTests(unittest.TestCase):
    def test_renderer_preserves_question_source_and_limit_evidence(self):
        value = packet()
        prompt = prepare.render("Explain Root.\n", value)
        self.assertTrue(prompt.startswith("Explain Root.\n"))
        self.assertIn("1: fun Root() = 1", prompt)
        self.assertIn("RELATED_SOURCE_NOT_INCLUDED", prompt)
        self.assertIn("CONDITIONAL_ANALYZER", prompt)
        self.assertIn("check-runtime", prompt)
        self.assertIn("PARTIAL/UNSURE", prompt)
        self.assertIn("Test companions are selected by filename and were not executed", prompt)

    def test_source_cannot_close_its_markdown_fence(self):
        prompt = prepare.render("Inspect the literal.", packet('val text = "```"\n'))
        self.assertIn("````text\n", prompt)
        self.assertTrue(prompt.endswith("````\n"))

    def test_tampered_source_is_not_preloaded(self):
        value = packet()
        value["sources"][0]["text"] = "different source"
        with self.assertRaisesRegex(ValueError, "evidence digest"):
            prepare.render("Explain", value)

    def test_raw_text_hash_is_not_mistaken_for_a_domain_bound_cas_digest(self):
        value = packet()
        value["sources"][0]["contentRef"]["digest"] = prepare.digest(value["sources"][0]["text"].encode())
        evidence = json.loads(json.dumps(value))
        evidence.pop("evidenceDigest")
        evidence["preparation"].pop("durationMs")
        value["evidenceDigest"] = prepare.digest(prepare.canonical(evidence))
        with self.assertRaisesRegex(ValueError, "source file digest"):
            prepare.render("Explain", value)

    def run_preparation(self, root, value, exit_code=0, allow_native=False):
        launcher = root / "clew"
        launcher.write_text("#!/usr/bin/env python3\nimport sys\nsys.stdout.write(" + repr(json.dumps(value)) + ")\nsys.stderr.write('private diagnostic\\n')\nraise SystemExit(" + str(exit_code) + ")\n")
        launcher.chmod(0o700)
        question = root / "question.txt"
        question.write_text("Explain Root with its edge cases.\n")
        arguments = ["--clew", str(launcher), "--repo", str(root), "--target-ref", "main",
                     "--profile", "kotlin-jvm-gradle-analysis", "--compilation", ":/main", "--identifier", "Root",
                     "--question-file", str(question), "--output-dir", str(root / "output")]
        if allow_native:
            arguments.append("--allow-native-on-failure")
        return prepare.prepare(prepare.parser().parse_args(arguments))

    def test_preparation_writes_first_message_without_running_a_model(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            result = self.run_preparation(root, packet())
            self.assertEqual(result["status"], "PREPARED")
            self.assertEqual((result["modelCalls"], result["modelTokens"]), (0, 0))
            self.assertEqual(result["command"][1:3], ["context", "packet"])
            self.assertNotIn("--file", result["command"])
            self.assertNotIn("--term", result["command"])
            prompt = Path(result["promptPath"])
            self.assertTrue(prompt.read_text().startswith("Explain Root with its edge cases."))
            self.assertEqual(stat.S_IMODE(prompt.stat().st_mode), 0o600)
            self.assertEqual(prepare.digest(prompt.read_bytes()), result["promptDigest"])
            with self.assertRaises(FileExistsError):
                self.run_preparation(root, packet())

    def test_admission_failure_blocks_by_default_and_preserves_the_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = {"schema":"codeclew-error/2.0", "error":{"code":"WORKER_PROTOCOL_MISMATCH", "message":"test refusal"}}
            result = self.run_preparation(root, value, exit_code=1)
            self.assertEqual(result["status"], "BLOCKED")
            self.assertFalse((root / "output/prompt.md").exists())
            self.assertEqual(json.loads((root / "output/packet.json").read_text()), value)
            self.assertEqual(result["failure"]["code"], "WORKER_PROTOCOL_MISMATCH")

    def test_explicit_native_continuation_keeps_managed_failure_visible(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = {"schema":"codeclew-error/2.0", "error":{"code":"WORKER_PROTOCOL_MISMATCH", "message":"test refusal"}}
            result = self.run_preparation(root, value, exit_code=1, allow_native=True)
            self.assertEqual(result["status"], "NATIVE_CONTINUATION")
            prompt = Path(result["promptPath"]).read_text()
            self.assertIn("WORKER_PROTOCOL_MISMATCH", prompt)
            self.assertIn("do not describe managed admission or source selection as successful", prompt)
            self.assertNotIn("Source evidence prepared before this request", prompt)

    def test_success_exit_with_non_object_packet_is_still_blocked(self):
        with tempfile.TemporaryDirectory() as temporary:
            result = self.run_preparation(Path(temporary), [], exit_code=0)
            self.assertEqual(result["status"], "BLOCKED")
            self.assertEqual(result["failure"]["code"], "UNUSABLE_SOURCE_PACKET")


if __name__ == "__main__":
    unittest.main()
