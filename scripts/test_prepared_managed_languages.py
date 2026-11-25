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


def captured_csharp(text, file="ScoreApi.cs"):
    raw = text.encode()
    blob = sha256(b"codeclew-cas/v2\0codeclew-repository-input-blob/2.0\0" + raw).hexdigest()
    return {"file": file, "revision": REVISION, "text": text, "textDigest": digest(text),
            "authority": "RETAINED_CSHARP_COMPILATION_UNIT",
            "occurrence": {"snapshot": "sha256:" + "b" * 64, "blob": "sha256:" + blob,
                           "startByte": 0, "endByte": len(raw)}}


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

SCHEDULED_ROOT = """@Scheduled(fixedDelay = 250)
    fun dispatch() {
        for (parcel in parcels) {
            transactions.executeWithoutResult {
                transmit(parcel.key)
            }
        }
    }"""
SCHEDULED = f"""package example
import java.net.http.HttpClient
import java.net.http.HttpRequest
import java.net.http.HttpResponse
import org.springframework.scheduling.annotation.Scheduled
import org.springframework.transaction.support.TransactionTemplate

class ParcelDispatcher(
    private val parcels: List<Parcel>,
    private val transactions: TransactionTemplate,
    private val relay: HttpClient,
    private val request: HttpRequest
) {{
    private val attempts = mutableMapOf<String, Long>()
    private val otherAttempts = mutableMapOf<String, Long>()

    {SCHEDULED_ROOT}

    private fun transmit(key: String) {{
        val prior = attempts[key]
        attempts[key] = 4L
        relay.send(request, HttpResponse.BodyHandlers.discarding())
        remember(key, prior)
    }}

    private fun remember(key: String, previous: Long?) {{ }}
}}

data class Parcel(val key: String, val token: String)
"""


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


    def test_typescript_retains_a_specific_fetch_failure_after_commit(self):
        self.require_typescript()
        text = """async function score() {
    await store.query("COMMIT");
    try { return await fetch("/jobs", { method: "POST" }); }
    catch (error) { return 503; }
}
"""
        process, context = selected("Jobs.ts", text)
        rows, _ = self.project(process, context)
        request, = (row for row in rows if row["kind"] == "http_request")
        edge = request["_edge"]
        self.assertEqual((edge["method"], edge["uri"], edge["site"]), ("POST", "/jobs", HANDLE))
        failure = {row["kind"]: row for row in rows if row.get("_edgeCause") == edge["occurrence"]}
        self.assertEqual(set(failure), {"exception_route", "request_after_commit_failure", "return"})
        self.assertIn("COMMIT fulfills", failure["request_after_commit_failure"]["text"])
        self.assertEqual(failure["return"]["text"], "Returns 503.")
        self.assertIn("Durability", failure["request_after_commit_failure"]["uncertainty"])

        process, context = selected("Jobs.ts", text.replace("score()", "score(fetch: LocalLookup)"))
        rows, _ = self.project(process, context)
        self.assertFalse(any("_edge" in row for row in rows))

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

    def test_kotlin_reports_nested_write_only_under_its_own_reached_guard(self):
        text = """fun apply(skip: Boolean, quick: Boolean, allowed: Boolean) {
    if (skip) return
    if (quick) {
        readLocal()
    } else {
        if (allowed) {
            persist()
        }
    }
}
"""
        process, context = selected("action.kt", text)
        facts, _ = self.project(process, context)
        arms = [row for row in facts if row["kind"] == "GUARDED_EFFECTS"]
        writes = [row for row in arms if "arm contains" in row["text"] and "`persist()`" in row["text"]]
        self.assertEqual(len(writes), 1, arms)
        self.assertIn("If execution reaches this decision", writes[0]["text"])
        self.assertEqual(writes[0]["condition"], "`quick` is false and `allowed`")
        self.assertEqual(writes[0]["sources"], [HANDLE])

    def scheduled_facts(self, text):
        root = SCHEDULED_ROOT
        if root not in text:
            root = text[text.index("@Scheduled"):text.index("\n\n    private fun transmit")]
        first = text[:text.index(root)].count("\n") + 1
        process, context = selected("ParcelDispatcher.kt", text, standalone=root, start=first)
        process.update(entryKind="SCHEDULED_JOB", entrySymbol="dispatch")
        raw = text.encode()

        def bind(witness):
            start, end = witness["startByte"], witness["endByte"]
            if raw[start:end] == witness["text"].encode() and witness["textDigest"] == digest(witness["text"]):
                return HANDLE

        context["_bindWitness"] = bind
        return self.project(process, context)

    def test_kotlin_scheduled_key_and_heap_write_reach_a_transaction_helper(self):
        facts, witnesses = self.scheduled_facts(SCHEDULED)
        indexed = {row["kind"]: row for row in facts}
        expected = {"SCHEDULED_ARGUMENT", "HEAP_WRITE_BEFORE_CALL", "HEAP_FAILURE_BYPASS", "SAME_KEY_HEAP_RETRY"}
        self.assertLessEqual(expected, indexed.keys(), facts)
        handoffs = [row["text"] for row in facts if row["kind"] == "SCHEDULED_ARGUMENT"]
        self.assertTrue(any("parcel.key" in text and "transmit" in text for text in handoffs), handoffs)
        self.assertIn("remember", indexed["HEAP_FAILURE_BYPASS"]["text"])
        self.assertIn("rollback", indexed["SAME_KEY_HEAP_RETRY"]["text"])
        self.assertIn("same receiver", indexed["SAME_KEY_HEAP_RETRY"]["uncertainty"])
        proof = [witnesses[ref]["text"] for ref in indexed["SAME_KEY_HEAP_RETRY"]["sources"] if ref in witnesses]
        self.assertTrue(any("attempts[key] = 4L" in text for text in proof), proof)
        self.assertTrue(any("val attempts" in text for text in proof), proof)
        self.assertTrue(any("transmit(parcel.key)" in text for text in proof), proof)

    def test_kotlin_scheduled_state_excludes_unbound_receiver_key_and_order(self):
        cases = [
            (SCHEDULED.replace("        attempts[key] = 4L", "        attempts[key] = 4L\n        attempts.clear()"),
             "SAME_KEY_HEAP_RETRY"),
            (SCHEDULED.replace("transmit(parcel.key)", "transmit(parcel.key, parcel.token)").replace(
                "transmit(key: String)", "transmit(key: String, otherKey: String)").replace(
                "val prior = attempts[key]", "val prior = attempts[otherKey]"), "SAME_KEY_HEAP_RETRY"),
            (SCHEDULED.replace("val prior = attempts[key]", "val prior = otherAttempts[key]"), "SAME_KEY_HEAP_RETRY"),
            (SCHEDULED.replace("attempts[key] = 4L", "val alias = attempts\n        alias[key] = 4L"), "HEAP_WRITE_BEFORE_CALL"),
            (SCHEDULED.replace("        attempts[key] = 4L\n", "").replace(
                "        remember(key, prior)", "        attempts[key] = 4L\n        remember(key, prior)"), "HEAP_WRITE_BEFORE_CALL"),
            (SCHEDULED.replace("transmit(parcel.key)", "try { transmit(parcel.key) } finally { attempts.clear() }"),
             "SAME_KEY_HEAP_RETRY"),
        ]
        for source, reaction in cases:
            with self.subTest(reaction=reaction):
                facts, _ = self.scheduled_facts(source)
                self.assertNotIn(reaction, {row["kind"] for row in facts}, facts)

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

    def test_typescript_keeps_native_navigation_candidate_open(self):
        self.require_typescript()
        caller = "function score(amount: number) { return check(amount); }"
        helper = "function check(amount: number) { return { status: 418 }; }"
        text = caller + "\n" + helper
        process, context = selected("score.ts", text, standalone=caller)
        offset = len(caller.encode()) + 1
        source = {**process["sources"]["s1"], "text": helper, "textDigest": digest(helper), "startLine": 2,
                  "endLine": 2, "startByte": offset, "endByte": offset + len(helper.encode())}
        helper_cite = "m123456789abc:e2"
        process["sources"]["s2"] = source
        process["facts"].extend([
            {"kind": "CANDIDATE", "declaration": "decl", "sources": ["s2"], "event": "CALL",
             "targetMethodId": "helper", "relationshipAuthority": "NAVIGATION_ONLY"},
            {"kind": "BODY", "declaration": "helper", "sources": ["s2"]},
        ])
        context["sourceRanges"].append({**source, "id": helper_cite, "references": ["s2"]})
        context["_projectionUnavailable"] = []
        facts, extra = self.project(process, context)
        self.assertTrue(facts)
        self.assertTrue(all("418" not in row["text"] and row["sources"] == [HANDLE] for row in facts))
        self.assertEqual(extra, {})
        self.assertEqual(context["_projectionUnavailable"], ["TYPESCRIPT_CALL_TARGETS_OPEN"])

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

    def test_roslyn_uses_exact_retained_declaration_and_captured_unit(self):
        self.require_roslyn()
        declaration = CSHARP.split("\n", 4)[4].rsplit("\n}", 1)[0]
        process, context = selected("ScoreApi.cs", CSHARP, standalone=declaration, start=5)
        context["_nativeFiles"]["ScoreApi.cs"] = captured_csharp(CSHARP)
        facts, extra = self.project(process, context)
        self.assertTrue(any("400" in row["text"] and "amount < 0" in row["text"] for row in facts), facts)
        self.assertTrue(all(row["sources"] == [HANDLE] for row in facts))
        self.assertEqual(extra, {})

    def test_roslyn_conflict_command_completion_does_not_assert_inserted_row(self):
        self.require_roslyn()
        header = """using Microsoft.AspNetCore.Http;
using System.Data.Common;
using System.Threading.Tasks;
public static class ScoreApi
{
"""
        declaration = """    public static async Task<IResult> Submit(DbConnection connection)
    {
        var transaction = await connection.BeginTransactionAsync();
        var command = connection.CreateCommand();
        command.Transaction = transaction;
        command.CommandText = "INSERT INTO batch (id) VALUES (1) ON CONFLICT DO NOTHING RETURNING id";
        if (await command.ExecuteScalarAsync() is not null)
            return Results.BadRequest();
        await transaction.CommitAsync();
        return Results.Ok();
    }"""
        text = header + declaration + "\n}\n"
        process, context = selected("ScoreApi.cs", text, standalone=declaration, start=6)
        context["_nativeFiles"]["ScoreApi.cs"] = captured_csharp(text)
        facts, _ = self.project(process, context)
        found = [row for row in facts if row["kind"] == "http-commit-before-response"
                 and "is not null` is false" in row.get("condition", "")]
        self.assertTrue(found, facts)
        for row in found:
            self.assertIn("command execution", row["text"])
            self.assertIn("leave the existing row unchanged", row["text"])
            self.assertEqual(row["sources"], [HANDLE])

    def test_roslyn_stored_row_delivery_uses_matching_key_receiver_and_order(self):
        self.require_roslyn()
        header = """using System.Threading.Tasks;
using Confluent.Kafka;
using Dapper;
using Npgsql;

sealed record Candidate(int Id, string Payload);
sealed class Relay
{
"""
        root = """    public async Task RunAsync(NpgsqlConnection connection, NpgsqlConnection other, IProducer<int, string> broker)
    {
        await PublishAsync(connection, other, broker);
    }"""
        helper = """    private static async Task PublishAsync(NpgsqlConnection connection, NpgsqlConnection other, IProducer<int, string> broker)
    {
        var tx = await connection.BeginTransactionAsync();
        var row = await connection.QuerySingleOrDefaultAsync<Candidate>(
            "SELECT id, payload FROM pending WHERE ready = true LIMIT 1", transaction: tx);
        if (row is null) return;
        await broker.ProduceAsync("parcel.available", new Message<int, string> { Key = row.Id, Value = row.Payload });
        await connection.ExecuteAsync("UPDATE pending SET ready = false WHERE id = @id", new { id = row.Id }, transaction: tx);
        await tx.CommitAsync();
    }"""
        for label, routine in {
            "matched": helper,
            "different receiver": helper.replace('await connection.ExecuteAsync', 'await other.ExecuteAsync'),
            "different key": helper.replace('new { id = row.Id }', 'new { id = 100 }'),
            "prior commit": helper.replace('        await tx.CommitAsync();\n', '').replace(
                '        await broker.ProduceAsync', '        await tx.CommitAsync();\n        await broker.ProduceAsync'),
        }.items():
            with self.subTest(label=label):
                text = header + root + "\n" + routine + "\n}\n"
                process, context = selected("Relay.cs", text, standalone=root, start=header.count("\n") + 1)
                process.update(entryKind="HOSTED_BACKGROUND_WORKER", entrySymbol="Relay.RunAsync")
                other, _ = selected("Relay.cs", text, standalone=routine,
                                    start=header.count("\n") + root.count("\n") + 2)
                source = other["sources"]["s1"]
                process["sources"]["s2"] = source
                process["facts"].append({"kind": "BODY", "declaration": "helper", "sources": ["s2"]})
                context["sourceRanges"].append({**source, "id": "m123456789abc:e2", "references": ["s2"]})
                context["_nativeFiles"]["Relay.cs"] = captured_csharp(text, "Relay.cs")
                facts, _ = self.project(process, context)
                deliveries = [row for row in facts if row["kind"] == "worker-row-delivery"]
                retries = [row for row in facts if row["kind"] == "worker-reselection" and "broker may" in row["text"]]
                if label == "matched":
                    self.assertTrue(deliveries and retries, facts)
                    self.assertIn("CommitAsync", deliveries[0]["text"])
                    self.assertIn("ProduceAsync", retries[0]["text"])
                else:
                    self.assertFalse(deliveries or retries, facts)

    def test_roslyn_rejects_same_body_in_differently_declared_git_unit(self):
        declaration = CSHARP.split("\n", 4)[4].rsplit("\n}", 1)[0]
        captured = CSHARP.replace("using Microsoft.AspNetCore.Http;", "using Results = Other.Results;")
        process, context = selected("ScoreApi.cs", captured, standalone=declaration, start=5)
        self.assertIn(declaration, CSHARP)
        self.assertIn(declaration, captured)
        context["_nativeFiles"] = {}
        context["_loadPinnedFile"] = lambda _file, _revision: CSHARP
        context["_projectionUnavailable"] = []
        self.assertEqual(self.project(process, context), ([], {}))
        self.assertEqual(context["_projectionUnavailable"], ["CSHARP_CAPTURED_COMPILATION_UNIT_UNAVAILABLE"])

    def test_roslyn_refuses_body_that_differs_from_captured_unit(self):
        declaration = CSHARP.split("\n", 4)[4].rsplit("\n}", 1)[0]
        process, context = selected("ScoreApi.cs", CSHARP, standalone=declaration, start=5)
        context["_nativeFiles"]["ScoreApi.cs"] = captured_csharp(CSHARP.replace("Ok(amount)", "Ok(3)"))
        context["_projectionUnavailable"] = []
        self.assertEqual(self.project(process, context), ([], {}))
        self.assertEqual(context["_projectionUnavailable"], ["CSHARP_CAPTURED_BODY_MISMATCH"])


if __name__ == "__main__":
    unittest.main()
