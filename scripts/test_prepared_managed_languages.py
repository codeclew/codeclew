#!/usr/bin/env python3
"""Portable citation and fallback tests for managed documentation projectors."""
from hashlib import sha256
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parent.parent
MANAGED = ROOT / "skills/codeclew/scripts/prepared_facts"
HANDLE = "m123456789abc:e1"
REVISION = "a" * 40
KOTLIN = """fun score(amount: Int): Int {
    if (amount < 0) return 400
    return 200
}
"""
TYPESCRIPT = """async function score(amount: number) {
    if (amount < 0) return { status: 400 };
    return { status: 200 };
}
"""
CSHARP = """using Microsoft.AspNetCore.Http;

public static class ScoreApi
{
    public static IResult Score(int amount)
    {
        if (amount < 0) return Results.BadRequest("negative");
        return Results.Ok(amount);
    }
}
"""


def digest(text):
    return "sha256:" + sha256(text.encode()).hexdigest()


def selected(file, text, *, standalone=None, start=1):
    standalone = text if standalone is None else standalone
    first = len(text[:text.index(standalone)].encode())
    metadata = {"file": file, "revision": REVISION, "text": standalone, "textDigest": digest(standalone),
                "authority": "EXACT_SNAPSHOT_TEXT", "startLine": start,
                "endLine": start + standalone.rstrip("\n").count("\n"),
                "startByte": first, "endByte": first + len(standalone.encode())}
    process = {"root": "decl", "entrySymbol": "score", "entryKind": "HTTP_ENDPOINT",
               "facts": [{"kind": "BODY", "declaration": "decl", "sources": ["s1"]}], "sources": {"s1": metadata}}
    context = {"sourceRanges": [{**metadata, "id": HANDLE, "references": ["s1"]}],
               "_sourceOccurrences": {"s1": metadata}, "entrySources": [HANDLE],
               "_nativeFiles": {file: {"file": file, "revision": REVISION, "text": text, "textDigest": digest(text)}}}
    return process, context


class PreparedManagedLanguages(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory(prefix="clew-prepared-language-test-")
        cls.addClassCleanup(cls.temp.cleanup)
        directory = Path(cls.temp.name) / "installed" / "prepared_facts"
        shutil.copytree(MANAGED, directory)
        cls.adapter = runpy.run_path(str(directory / "managed_languages.py"))
        cls.node = shutil.which("node")

    def project(self, process, context):
        return self.adapter["project"](process, context, node=self.node)

    def require_typescript(self):
        if not self.node:
            self.skipTest("Node is unavailable")
        probe = subprocess.run([self.node, "-p", "require('typescript').version"], capture_output=True, text=True)
        if probe.returncode or probe.stdout.strip() != "5.9.3":
            self.skipTest("The locked TypeScript dependency is unavailable")

    def require_roslyn(self):
        dotnet = shutil.which("dotnet")
        if not dotnet:
            self.skipTest(".NET SDK is unavailable")
        probe = subprocess.run([dotnet, "--list-sdks"], capture_output=True, text=True)
        if probe.returncode or not any(line.startswith("10.") for line in probe.stdout.splitlines()):
            self.skipTest(".NET 10 SDK is unavailable")

    def test_kotlin_cites_only_admitted_work_handle(self):
        process, context = selected("score.kt", KOTLIN)
        facts, extra = self.project(process, context)
        self.assertTrue(any("400" in row["text"] and "amount < 0" in row["text"] for row in facts), facts)
        self.assertEqual({handle for row in facts for handle in row["sources"]}, {HANDLE})
        self.assertEqual(extra, {})

    def test_conflicting_source_digest_cannot_reuse_a_work_handle(self):
        process, context = selected("score.kt", KOTLIN)
        other = context["sourceRanges"][0]["text"].replace("400", "401")
        context["sourceRanges"][0].update(text=other, textDigest=digest(other))
        self.assertEqual(self.project(process, context), ([], {}))

    def test_discovery_witness_preserves_exact_utf8_source_interval(self):
        text = "// café\n" + KOTLIN
        process, context = selected("score.kt", text)
        process.update(root=None, facts=[])
        context["file"] = "score.kt"
        facts, extra = self.project(process, context)
        self.assertTrue(facts)
        self.assertTrue(extra)
        for row in extra.values():
            raw = text.encode()[row["startByte"]:row["endByte"]]
            self.assertEqual(raw.decode(), row["text"])
            self.assertEqual(digest(raw.decode()), row["textDigest"])
            self.assertEqual(row["startLine"], 2)

    def test_kotlin_schedule_comes_from_exact_entry_annotation(self):
        text = '@Scheduled(fixedDelayString = "\\${INTERVAL:1000}")\n' + KOTLIN
        process, context = selected("score.kt", text)
        process["entryKind"] = "SCHEDULED_JOB"
        facts, _ = self.project(process, context)
        self.assertTrue(any(row["kind"] == "ENTRY" and "INTERVAL:1000" in row["text"]
                            and row["sources"] == [HANDLE] for row in facts), facts)

    def test_kotlin_declared_receiver_adds_conditional_wait_and_exact_witness(self):
        body = """fun produce() {
        try {
            transaction.executeWithoutResult {
                publisher.send().get(10, TimeUnit.SECONDS)
                database.save()
            }
        } catch (error: Exception) { notify(error) }
    }"""
        text = ("import org.springframework.transaction.support.TransactionTemplate\n"
                "class Job(private val transaction: TransactionTemplate) {\n    " + body + "\n}\n")
        process, context = selected("Job.kt", text, standalone=body, start=3)

        def bind(witness):
            candidate = text.encode()[witness["startByte"]:witness["endByte"]]
            self.assertEqual(candidate.decode(), witness["text"])
            return HANDLE

        context["_bindWitness"] = bind
        facts, extra = self.project(process, context)
        waiting = [row for row in facts if row["kind"] == "WAIT_TRANSACTION_BOUNDARY"]
        self.assertTrue(waiting, facts)
        self.assertIn("supplies a Java Future", waiting[0]["text"])
        field = next((ref for ref, row in extra.items() if row["text"] == "private val transaction: TransactionTemplate"), None)
        self.assertIsNotNone(field)
        self.assertIn(field, waiting[0]["sources"])

    def test_typescript_retained_body_is_projected_with_locked_compiler(self):
        self.require_typescript()
        process, context = selected("score.ts", TYPESCRIPT)
        facts, extra = self.project(process, context)
        self.assertTrue(any("400" in row["text"] for row in facts), facts)
        self.assertTrue(all(row["sources"] == [HANDLE] for row in facts))
        self.assertEqual(extra, {})

    def test_typescript_discovery_uses_only_actual_file_citation(self):
        self.require_typescript()
        process, context = selected("score.ts", "// café\n" + TYPESCRIPT)
        process.update(root=None, facts=[])
        context["file"] = "score.ts"
        facts, extra = self.project(process, context)
        self.assertTrue(any("400" in row["text"] for row in facts), facts)
        self.assertTrue(all(row["sources"] == [HANDLE] for row in facts))
        self.assertEqual(extra, {})

    def test_unavailable_typescript_does_not_create_a_behavior_fact(self):
        if not self.node:
            self.skipTest("Node is unavailable")
        process, context = selected("score.ts", TYPESCRIPT)
        context["_typescriptModule"] = str(Path(self.temp.name) / "missing.js")
        context["_projectionUnavailable"] = []
        self.assertEqual(self.project(process, context), ([], {}))
        self.assertEqual(context["_projectionUnavailable"], ["TYPESCRIPT_COMPILER_UNAVAILABLE_OR_MISMATCHED"])

    def test_roslyn_uses_exact_retained_declaration_and_full_unit(self):
        self.require_roslyn()
        declaration = CSHARP.split("\n", 4)[4].rsplit("\n}", 1)[0]
        process, context = selected("ScoreApi.cs", CSHARP, standalone=declaration, start=5)
        context["_nativeFiles"] = {}
        context["_loadPinnedFile"] = lambda file, revision: CSHARP if (file, revision) == (
            "ScoreApi.cs", REVISION) else None
        facts, extra = self.project(process, context)
        self.assertTrue(any("400" in row["text"] and "amount < 0" in row["text"] for row in facts), facts)
        self.assertTrue(all(row["sources"] == [HANDLE] for row in facts))
        self.assertEqual(extra, {})

    def test_roslyn_refuses_edited_or_missing_pinned_unit(self):
        process, context = selected("ScoreApi.cs", CSHARP, standalone="        return Results.Ok(amount);", start=8)
        context["_nativeFiles"] = {}
        self.assertEqual(self.project(process, context), ([], {}))
        context["_loadPinnedFile"] = lambda _file, _revision: CSHARP.replace("Ok(amount)", "Ok(3)")
        self.assertEqual(self.project(process, context), ([], {}))


if __name__ == "__main__":
    unittest.main()
