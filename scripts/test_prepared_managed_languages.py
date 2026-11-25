#!/usr/bin/env python3
"""Portable citation and fallback tests for managed documentation projectors."""
from hashlib import sha256
import json
from pathlib import Path
import runpy
import shutil
import time
import subprocess
import tempfile
import time
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


    def test_typescript_admits_an_encoded_path_shape_with_known_or_configured_origin(self):
        self.require_typescript()
        edges = runpy.run_path(str(MANAGED / "clew_service_edges.py"))
        for origin, expected in (('configuration.origin', True), ('"https://example.test"', False)):
            text = f"""async function score(configuration: Config, id: string) {{
    const origin = {origin};
    const method = "post";
    const endpoint = `${{origin}}/v1/units/${{encodeURIComponent(id)}}/action`;
    return await fetch(endpoint, {{method}});
}}
"""
            with self.subTest(origin=origin):
                process, context = selected("Units.ts", text)
                rows, _ = self.project(process, context)
                request, = (row for row in rows if row["kind"] == "http_request")
                edge = request["_edge"]
                self.assertEqual(edge["configuredOrigin"], expected)
                self.assertEqual(edges["uri_path"](edge["uri"]), "/v1/units/{segment}/action")
                self.assertEqual(edges["request_key"](edge["method"], edges["uri_path"](edge["uri"])),
                                 ("POST", ("v1", "units", None, "action")))
                self.assertIn("nonempty, non-dot", request["uncertainty"])

        for call in (
            'fetch(new Request("/units"))',
            'fetch(`/units/${id}`)',
            'fetch(`/units/${encodeURIComponent("..")}`)',
            'fetch("/units", { ...options })',
            'fetch("/units", { method: options.method })',
            'fetch("/units", { method: "TRACE" })',
            'fetch("/units", { method: "" })',
            'fetch("/units", { __proto__: { method: "POST" } })',
            'fetch("/units", { method: "GET", body: id })',
        ):
            with self.subTest(call=call):
                process, context = selected("Units.ts", f"async function score(id: string, options: Options) {{ {call}; }}")
                rows, _ = self.project(process, context)
                self.assertFalse(any(row["kind"] == "http_request" for row in rows))

    def test_typescript_response_decode_preserves_request_occurrence_and_local_status(self):
        self.require_typescript()
        text = """async function score(reply: Reply) {
    try {
        const response = await fetch("/units", {method: "POST"});
        const result = await response.json();
        return reply.status(200).json(result);
    } catch (error) {
        return reply.status(502).json({message: "Response unavailable"});
    }
}
"""
        process, context = selected("Units.ts", text)
        rows, _ = self.project(process, context)
        request, = (row for row in rows if row["kind"] == "http_request")
        origin = request["_edge"]["occurrence"]
        decoding = [row for row in rows if row.get("_edgePhase") == "RESPONSE_DECODING"]
        self.assertTrue(decoding)
        self.assertTrue(all(row["_edgeCause"] == origin for row in decoding))
        failure, = (row for row in decoding if row["kind"] == "status_body")
        self.assertIn("reply.status receives 502", failure["text"])
        self.assertIn("Response unavailable", failure["text"])
        self.assertTrue(any(row["kind"] == "response_body_read" for row in decoding))

        process, context = selected("Units.ts", text.replace("await response.json()", "await unrelated.json()"))
        rows, _ = self.project(process, context)
        self.assertFalse(any(row.get("_edgePhase") == "RESPONSE_DECODING" for row in rows))

    def test_qualified_origin_concatenation_survives_nested_await_and_reply_code(self):
        self.require_typescript()
        text = """async function score(reply: Reply, base: string, id: string) {
    const path = `/books/${encodeURIComponent(id)}/claim`;
    try {
        const res = await fetch(base + path, {method: "POST", signal: AbortSignal.timeout(500)});
        return reply.code(res.status).send(await res.json());
    } catch {
        return reply.code(502).send({error: "REMOTE_UNAVAILABLE"});
    }
}
"""
        process, context = selected("Books.ts", text)
        rows, _ = self.project(process, context)
        request, = (row for row in rows if row["kind"] == "http_request")
        edge = request["_edge"]
        self.assertEqual((edge["method"], edge["uri"]), ("POST", "/books/{segment}/claim"))
        self.assertTrue(edge["configuredOrigin"] and edge["encodedSegment"])
        decoding = [row for row in rows if row.get("_edgePhase") == "RESPONSE_DECODING"]
        self.assertTrue(all(row["_edgeCause"] == edge["occurrence"] for row in decoding))
        status, = (row for row in decoding if row["kind"] == "status_body")
        self.assertIn("reply.code receives 502", status["text"])
        self.assertIn("REMOTE_UNAVAILABLE", status["text"])
        self.assertIn("res.json", status["condition"])
        for unsupported in ("base + id", 'base + "//untrusted.example/action"', "base + new Request(path)"):
            process, context = selected("Books.ts", text.replace("base + path", unsupported))
            rows, _ = self.project(process, context)
            self.assertFalse(any(row["kind"] == "http_request" or row.get("_edgePhase") for row in rows))

    def test_kafkajs_topics_require_an_awaited_subscription_on_the_same_consumer_setup(self):
        self.require_typescript()
        text = """import { Kafka } from "kafkajs";
const kafka = new Kafka({ brokers: [] });
const consumer = kafka.consumer({ groupId: "a" });
const runner = consumer;
const other = kafka.consumer({ groupId: "b" });
const known = "unit.available";
const producer = kafka.producer();
async function receive() {
    const display = { send(input: {topic: string; messages: string[]}) {
        return input.topic.length + input.messages.length;
    } };
    display.send({topic: "not.published", messages: ["display text"]});
    await producer.send({topic: "unit.processed", messages: []});
}
async function main(dynamic: string, enabled: boolean) {
    if (enabled) await consumer.subscribe({ topics: ["branch.only"] });
    await other.subscribe({ topic: "other.only" });
    await consumer.subscribe({ topic: dynamic });
    consumer.subscribe({ topic: "unawaited.only" });
    await consumer.subscribe({ topics: [known, "unit.completed"] });
    await runner.run({ eachMessage: receive });
    await consumer.subscribe({ topic: "later.only" });
}
"""
        candidate, = self.adapter["discover_background"]({"Worker.ts": {"text": text}}, node=self.node,
                                                         deadline=time.monotonic() + 10, family="typescript")
        self.assertEqual(candidate["topics"], ["unit.available", "unit.completed"])
        self.assertTrue(candidate["topicCoverageOpen"])
        subscription, = candidate["subscriptions"]
        self.assertIn("subscribe({ topics: [known", text[subscription["start"]:subscription["end"]])
        origin, = candidate["topicSources"]
        self.assertEqual(text[origin["start"]:origin["end"]], 'known = "unit.available"')

        scripts = MANAGED.parent
        helper = runpy.run_path(str(scripts / "docs_run_prepared.py"), init_globals={
            "sha": lambda data: "sha256:" + sha256(data).hexdigest(),
            "encode": lambda value: json.dumps(value, sort_keys=True).encode()})
        _, context = selected("Worker.ts", text)
        record, = context["sourceRanges"]
        occurrence = {"snapshot": "worker-snapshot", "blob": "worker-blob"}
        origin = {**record, "occurrence": occurrence}
        requirement, process, witnesses, _, facts = helper["background_operation"](
            "listener", candidate, {"Worker.ts": origin}, {HANDLE: origin})
        self.assertEqual(requirement["trigger"]["configuration"]["topics"], ["unit.available", "unit.completed"])
        self.assertEqual(len(requirement["trigger"]["subscriptionSources"]), 1)
        self.assertTrue(any(fact["kind"] == "background-topic-boundary" for fact in facts))
        context = {"sourceRanges": list(witnesses.values()), "_sourceOccurrences": witnesses,
                   "entrySources": process["entrySources"]}
        facts, _ = self.project(process, context)
        publish, = (fact for fact in facts if fact.get("_edge", {}).get("kind") == "PUBLISH")
        self.assertEqual(publish["_edge"]["topic"], "unit.processed")
        bound, = process["_background"]["publications"]
        self.assertTrue(set(bound) <= set(publish["sources"]))
        self.assertIn("KafkaJS producer", publish["text"])

    def test_kotlin_publication_uses_qualified_receiver_and_literal_topic(self):
        body = """fun score(publisher: org.springframework.kafka.core.KafkaTemplate<String, String>, value: String) {
    publisher.send("parcel.available", value)
}
"""
        for label, text in (("typed", body), ("foreign", body.replace("org.springframework.kafka.core", "internal.broker")),
                            ("dynamic", body.replace('"parcel.available"', 'value')),
                            ("overload", body.replace('"parcel.available", value', 'value'))):
            with self.subTest(form=label):
                process, context = selected("Worker.kt", text)
                rows, _ = self.project(process, context)
                publications = [row for row in rows if row.get("_edge", {}).get("kind") == "PUBLISH"]
                if label != "typed":
                    self.assertEqual(publications, [])
                    continue
                row, = publications
                self.assertEqual(row["_edge"]["site"], HANDLE)
                self.assertEqual(row["_edge"]["topic"], "parcel.available")
                self.assertIn("assumed", row["uncertainty"])

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
        return self.kotlin_callback_facts(text, root, "SCHEDULED_JOB", "dispatch")

    def kotlin_callback_facts(self, text, root, kind, symbol):
        first = text[:text.index(root)].count("\n") + 1
        process, context = selected("ParcelDispatcher.kt", text, standalone=root, start=first)
        process.update(entryKind=kind, entrySymbol=symbol)
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


    def test_kotlin_scheduled_accessor_guard_and_after_call_overwrite(self):
        source = (SCHEDULED.replace("import java.net.http.HttpClient", """import java.time.Instant
import java.util.concurrent.ConcurrentHashMap
import java.net.http.HttpClient""")
                  .replace("transmit(parcel.key)", "transmit(parcel)")
                  .replace("private val attempts = mutableMapOf<String, Long>()",
                           "private val attempts = ConcurrentHashMap<String, Instant>()")
                  .replace("""private fun transmit(key: String) {
        val prior = attempts[key]
        attempts[key] = 4L
        relay.send(request, HttpResponse.BodyHandlers.discarding())
        remember(key, prior)
    }""", """private fun transmit(parcel: Parcel) {
        val key = parcel["lookup-key"].asText()
        val cached = parcel.token.takeIf { it.isNotEmpty() }
        if (cached != null) return
        val status = if (key.isEmpty()) {
            0
        } else {
            require(key.length < 200) { "label too long" }
            val option = parcel.optional()?.takeIf { it.value.isNotEmpty() } ?: error("missing option")
            if (attempts[key]?.isAfter(Instant.now()) == true) error("too early")
            attempts[key] = Instant.now()
            val reply = relay.send(request, HttpResponse.BodyHandlers.discarding())
            if (reply.statusCode() == 202) {
                attempts[key] = if (option.value.isEmpty()) Instant.EPOCH else Instant.now()
            }
            reply.statusCode()
        }
    }""")
                  .replace("data class Parcel(val key: String, val token: String)", """data class Parcel(val key: String, val token: String) {
    operator fun get(name: String): Label = Label(if (name == "lookup-key") key else token)
    fun optional(): Label? = Label(key)
}
data class Label(val value: String) {
    fun asText(): String = value
}"""))
        facts, witnesses = self.scheduled_facts(source)
        found = {row["kind"]: row for row in facts}
        expected = {"SCHEDULED_GUARD_EXIT", "HEAP_GUARD_EXIT", "HEAP_WRITE_BEFORE_CALL", "HEAP_FAILURE_BYPASS",
                    "HEAP_WRITE_AFTER_CALL", "SAME_KEY_HEAP_RETRY"}
        self.assertLessEqual(expected, found.keys(), facts)
        self.assertIn("cached != null", found["SCHEDULED_GUARD_EXIT"]["condition"])
        self.assertIn("relay.send", found["SCHEDULED_GUARD_EXIT"]["text"])
        self.assertIn("isAfter", found["HEAP_GUARD_EXIT"]["condition"])
        self.assertIn("passes the Kotlin require guard", found["HEAP_WRITE_BEFORE_CALL"]["condition"])
        self.assertIn("yields a non-null value", found["HEAP_WRITE_BEFORE_CALL"]["condition"])
        self.assertIn("statusCode", found["HEAP_WRITE_AFTER_CALL"]["condition"])
        self.assertIn("equal derived keys", found["SAME_KEY_HEAP_RETRY"]["uncertainty"])
        proof = [witnesses[ref]["text"] for ref in found["SAME_KEY_HEAP_RETRY"]["sources"] if ref in witnesses]
        self.assertTrue(any('parcel["lookup-key"].asText()' in text for text in proof), proof)

    def test_kotlin_scheduled_map_state_survives_only_intact_join_arms(self):
        source = (SCHEDULED.replace("import java.net.http.HttpClient", "import java.net.URI\nimport java.net.http.HttpClient")
                  .replace("val prior = attempts[key]", "val prior = attempts[key]\n        if (attempts[key] == 99L) error(\"paused\")")
                  .replace("relay.send(request, HttpResponse.BodyHandlers.discarding())", """val opts = HttpRequest.newBuilder(URI.create("https://example.test/dispatch"))
        val extra = request.headers().firstValue("X-Extra").orElse(null)
        BRANCH
        val response = relay.send(opts.build(), HttpResponse.BodyHandlers.discarding())""")
                  .replace("private fun remember(key: String, previous: Long?) { }", """private fun remember(key: String, previous: Long?) { }

    private fun opaque(target: MutableMap<String, Long>) { target.clear() }
    private fun clearOwner() { attempts.clear() }"""))
        cases = {
            "unrelated builder": ('if (extra != null) opts.header("X-Extra", extra)', True),
            "other map": ('if (extra != null) otherAttempts.clear()', True),
            "unchanged alias": ('var copy = otherAttempts\n        if (extra != null) copy = otherAttempts\n'
                                '        copy.clear()', True),
            "divergent alias": ('var copy = otherAttempts\n        if (extra != null) copy = attempts\n'
                                '        copy.clear()', False),
            "branch write": ('if (extra != null) attempts[key] = 7L', False),
            "opaque argument": ('val copy = attempts\n        if (extra != null) opaque(copy)', False),
            "implicit owner": ('if (extra != null) clearOwner()', False),
        }
        for label, (branch, survives) in cases.items():
            with self.subTest(label=label):
                facts, _ = self.scheduled_facts(source.replace("BRANCH", branch))
                repeats = [row for row in facts if row["kind"] == "SAME_KEY_HEAP_RETRY"]
                self.assertEqual(bool(repeats), survives, facts)
                if survives:
                    result = repeats[0]
                    self.assertIn("attempts[key] == 99L", result["condition"])
                    self.assertNotIn("extra != null", result["condition"])
                    self.assertIn("opts.build()", result["text"])

    def test_kotlin_native_listener_cites_later_same_key_invocation_through_transaction(self):
        root = """@KafkaListener(topics = ["parcel.received"])
    fun accept(parcel: Parcel) {
        transactions.executeWithoutResult {
            transmit(parcel.key)
        }
    }"""
        text = (SCHEDULED.replace(SCHEDULED_ROOT, root)
                .replace("org.springframework.scheduling.annotation.Scheduled", "org.springframework.kafka.annotation.KafkaListener"))
        for source, expected in ((text, True), (text.replace("attempts[key] = 4L", "otherAttempts[key] = 4L"), False)):
            with self.subTest(same_receiver=expected):
                facts, witnesses = self.kotlin_callback_facts(source, root, "KAFKA_LISTENER", "accept")
                repeats = [row for row in facts if row["kind"] == "SAME_KEY_HEAP_RETRY"]
                self.assertEqual(bool(repeats), expected, facts)
                self.assertTrue(any(row["kind"] == "LISTENER_ARGUMENT" for row in facts), facts)
                if expected:
                    result, = repeats
                    self.assertIn("A later message-listener invocation", result["text"])
                    self.assertIn("same receiver, equal derived keys", result["uncertainty"])
                    self.assertIn("transaction template invokes its callback", result["condition"])
                    proof = [witnesses[ref]["text"] for ref in result["sources"] if ref in witnesses]
                    self.assertTrue(any("parcel: Parcel" in excerpt for excerpt in proof), proof)


    def test_kotlin_http_client_builder_tracks_only_known_receiver_transitions(self):
        source = (SCHEDULED.replace("    private val relay: HttpClient,\n", "")
                  .replace("    private val attempts =", """    private val relay = java.net.http.HttpClient.newBuilder()
        .connectTimeout(java.time.Duration.ofSeconds(2))
        .followRedirects(HttpClient.Redirect.NORMAL)
        .build()
    private val attempts ="""))
        for variant, expected in (
            (source, True),
            (source.replace(".followRedirects(HttpClient.Redirect.NORMAL)", ".customStep()"), False),
            (source.replace(".connectTimeout(java.time.Duration.ofSeconds(2))", ".connectTimeout()"), False),
            (source.replace("java.net.http.HttpClient.newBuilder()", "LocalClient.newBuilder()"), False),
            (source.replace("    private val relay =", "    private val java = LocalJava()\n    private val relay ="), False),
        ):
            with self.subTest(supports_external=expected):
                facts, _ = self.scheduled_facts(variant)
                self.assertEqual(any(row["kind"] == "HEAP_WRITE_BEFORE_CALL" for row in facts), expected, facts)

    def test_kotlin_listener_maps_successful_decode_result_into_bound_helper(self):
        root = """@KafkaListener(topics = ["parcel.received"])
    fun accept(record: Message) {
        val decoded = try {
            codec.readTree(record.value()).also { validate(it) }
        } catch (failure: Exception) {
            return
        }
        transactions.executeWithoutResult {
            val key = decoded["identifier"]
            transmit(key)
        }
    }"""
        source = (SCHEDULED.replace(SCHEDULED_ROOT, root)
                  .replace("org.springframework.scheduling.annotation.Scheduled", "org.springframework.kafka.annotation.KafkaListener")
                  .replace("    private val attempts =", """    private val codec = Codec()
    private fun validate(tree: Tree) { }
    private val attempts =""")
                  + """
data class Message(private val parcel: Parcel) {
    fun value(): Parcel = parcel
}
class Tree(private val parcel: Parcel) {
    operator fun get(name: String): String = if (name == "identifier") parcel.key else parcel.token
}
class Codec {
    fun readTree(parcel: Parcel): Tree = Tree(parcel)
}
""")
        facts, witnesses = self.kotlin_callback_facts(source, root, "KAFKA_LISTENER", "accept")
        by_kind = {row["kind"]: row for row in facts}
        self.assertLessEqual({"LISTENER_ARGUMENT", "HEAP_WRITE_BEFORE_CALL", "SAME_KEY_HEAP_RETRY"}, by_kind.keys(), facts)
        retry = by_kind["SAME_KEY_HEAP_RETRY"]
        self.assertIn("completes normally", retry["condition"])
        self.assertIn("equal derived keys", retry["uncertainty"])
        proof = [witnesses[ref]["text"] for ref in retry["sources"] if ref in witnesses]
        self.assertTrue(any("readTree(record.value())" in excerpt for excerpt in proof), proof)
        self.assertTrue(any('decoded["identifier"]' in excerpt for excerpt in proof), proof)

        for modified in (
            root.replace("return", "Tree(Parcel(\"fallback\", \"token\"))"),
            root.replace("record.value()", "unrelated()"),
            root.replace(".also { validate(it) }", ".custom { validate(it) }"),
        ):
            with self.subTest(modified=modified):
                absent, _ = self.kotlin_callback_facts(source.replace(root, modified), modified,
                                                       "KAFKA_LISTENER", "accept")
                self.assertFalse(any(row["kind"] == "SAME_KEY_HEAP_RETRY" for row in absent), absent)

    def test_kotlin_scheduled_alias_mutations_invalidate_the_actual_map_only(self):
        for replacement in ("same.clear()", "same[key] = 8L", "same.put(key, 8L)"):
            with self.subTest(replacement=replacement):
                updated = SCHEDULED.replace("        attempts[key] = 4L",
                                            "        attempts[key] = 4L\n        val same = attempts\n        " + replacement)
                facts, _ = self.scheduled_facts(updated)
                self.assertNotIn("SAME_KEY_HEAP_RETRY", {row["kind"] for row in facts}, facts)
        unrelated = SCHEDULED.replace("        attempts[key] = 4L",
                                     "        attempts[key] = 4L\n        val same = otherAttempts\n        same.clear()")
        facts, _ = self.scheduled_facts(unrelated)
        self.assertIn("SAME_KEY_HEAP_RETRY", {row["kind"] for row in facts}, facts)

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


    def test_typescript_discovery_checks_callback_and_consumer_method_rebinding(self):
        self.require_typescript()
        text = """import { Kafka } from "kafkajs";
const kafka = new Kafka({ brokers: ["broker:9092"] });
const feed = kafka.consumer({ groupId: "parcel-feed" });
const alias = feed;
async function handle() { return "initial"; }
async function replacement() { return "replacement"; }
async function boot() {
    REBIND
    await feed.subscribe({ topic: "lot.arrived" });
    await alias.run({ eachMessage: handle });
}
"""
        def discover(change):
            files = {"boot.ts": {"text": text.replace("REBIND", change)}}
            return self.adapter["discover_background"](files, node=self.node, deadline=time.monotonic() + 8,
                                                       family="typescript")

        original, = discover("")
        self.assertEqual(original["topics"], ["lot.arrived"])
        self.assertEqual(discover("handle = replacement;"), [])
        self.assertEqual(discover("alias.run = replacement;"), [])
        for method_write in ('alias.subscribe = replacement;', 'feed["subscribe"] = replacement;'):
            replaced, = discover(method_write)
            self.assertEqual(replaced["topics"], [])
            self.assertTrue(replaced["topicCoverageOpen"])
        other, = discover("const other = kafka.consumer({ groupId: 'other' }); other.subscribe = replacement;")
        self.assertEqual(other["topics"], ["lot.arrived"])

    def test_typescript_early_exit_distinguishes_repeated_call_occurrences(self):
        self.require_typescript()
        text = """async function run(stop: boolean) {
    if (stop) {
        await record("FINISH");
        return;
    }
    await record("FINISH");
}
"""
        process, context = selected("worker.ts", text)
        facts, _ = self.project(process, context)
        exits = [row for row in facts if row["kind"] == "early_exit"]
        self.assertEqual(len(exits), 1, facts)
        text = exits[0]["text"]
        self.assertIn("reaches the earlier call at line 3", text)
        self.assertIn("before reaching the later call at line 6", text)
        self.assertIn(HANDLE, exits[0]["sources"])

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
            "dynamic topic": helper.replace('"parcel.available"', 'row.Payload'),
            "unbound receiver": helper.replace("IProducer<int, string> broker", "object broker"),
            "other overload": helper.replace('"parcel.available"', 'topicPartition: row.Id').replace(
                'topicPartition: row.Id, new Message', 'topicPartition: row.Id, message: new Message'),
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
                publications = [row for row in facts if row.get("_edge", {}).get("kind") == "PUBLISH"]
                if label in {"dynamic topic", "unbound receiver", "other overload"}:
                    self.assertEqual(publications, [])
                    continue
                self.assertTrue(publications, facts)
                self.assertEqual({row["_edge"]["topic"] for row in publications}, {"parcel.available"})
                self.assertTrue(all("declared receiver binds" in row["uncertainty"] and
                                    row["_edge"]["site"] == "m123456789abc:e2" for row in publications))
                if label == "matched":
                    self.assertTrue(deliveries and retries, facts)
                    self.assertIn("CommitAsync", deliveries[0]["text"])
                    self.assertIn("ProduceAsync", retries[0]["text"])
                else:
                    self.assertFalse(deliveries or retries, facts)

    def test_roslyn_replays_one_guarded_worker_invocation_through_returned_db_handle(self):
        self.require_roslyn()
        path = ROOT / "fixtures/documentation-background-processes/roslyn/DispatchWorker.cs"
        fixture = path.read_text()
        for label, text in {
            "matched": fixture,
            "different returned handle": fixture.replace('await db.ExecuteAsync',
                'var other = await OpenAsync(source);\n        await other.ExecuteAsync'),
            "different transaction": fixture.replace('await tx.CommitAsync();',
                'var other = await db.BeginTransactionAsync();\n        await other.CommitAsync();'),
            "independent sibling": fixture.replace('            if (enabled) await RelayAsync(source, broker);',
                '            if (enabled) await RelayAsync(source, broker);\n            else await MarkAsync(source);').replace(
                    '        await db.ExecuteAsync', '    }\n\n    private static async Task MarkAsync(NpgsqlDataSource source)\n    {\n'
                    '        var db = await OpenAsync(source);\n        var tx = await db.BeginTransactionAsync();\n        await db.ExecuteAsync').replace(
                        'new { id = row.Id },', 'new { id = 10 },'),
        }.items():
            with self.subTest(label=label):
                names = ["ExecuteAsync", "RelayAsync", "OpenAsync"]
                if label == "independent sibling":
                    names.append("MarkAsync")
                for index, name in enumerate(names, 1):
                    line = next(line for line in text.splitlines() if line.startswith("    ") and
                                "async Task" in line and f" {name}(" in line)
                    first = text.index(line)
                    end = text.index("\n    }", first) + len("\n    }")
                    declaration = text[first:end]
                    branch, current = selected(path.name, text, standalone=declaration,
                                               start=text[:first].count("\n") + 1)
                    if index == 1:
                        process, context = branch, current
                        process.update(entryKind="HOSTED_BACKGROUND_WORKER", entrySymbol="DispatchWorker.ExecuteAsync")
                    else:
                        ref = f"s{index}"
                        source = branch["sources"]["s1"]
                        process["sources"][ref] = source
                        process["facts"].append({"kind": "BODY", "declaration": name, "sources": [ref]})
                        context["sourceRanges"].append({**source, "id": f"m123456789abc:e{index}", "references": [ref]})
                context["_nativeFiles"][path.name] = captured_csharp(text, path.name)
                facts, _ = self.project(process, context)
                deliveries = [row for row in facts if row["kind"] == "worker-row-delivery"]
                commit_faults = [row for row in facts if row["kind"] == "worker-reselection" and
                                 "`CommitAsync` can fail" in row["text"]]
                if label == "matched":
                    self.assertTrue(deliveries and commit_faults, facts)
                    self.assertIn("`enabled` is true", commit_faults[0]["text"])
                    self.assertIn("`RelayAsync(source, broker)`", commit_faults[0]["text"])
                    self.assertIn("`row is null` is false", deliveries[0]["text"])
                    self.assertIn("row.Payload", deliveries[0]["text"])
                    self.assertIn("connection returned by `OpenAsync(source)`", deliveries[0]["text"])
                    self.assertIn(HANDLE, commit_faults[0]["sources"])
                    self.assertIn("m123456789abc:e3", commit_faults[0]["sources"])
                else:
                    self.assertFalse(deliveries or commit_faults, facts)

    def test_roslyn_worker_reader_keeps_returned_open_connection_and_row_data(self):
        self.require_roslyn()
        path = ROOT / "fixtures/documentation-background-processes/roslyn/ReaderWorker.cs"
        fixture = path.read_text()
        direct = fixture.replace('        using var document = JsonDocument.Parse(payload);\n'
                                 '        var routingKey = document.RootElement.GetProperty("routing").GetProperty("key").GetString();',
                                 '        var routingKey = payload;')
        for label, text in {
            "json key and direct payload": fixture,
            "json key alone": fixture.replace("Value = payload", 'Value = "fixed"'),
            "direct payload": direct,
            "unrelated connection": direct.replace('await using var update = db.CreateCommand();',
                'await using var alternate = await ConnectAsync(settings, token);\n'
                '        await using var update = alternate.CreateCommand();'),
            "unrelated publish": direct.replace('Key = routingKey, Value = payload', 'Key = "fixed", Value = "different"'),
        }.items():
            with self.subTest(label=label):
                for index, name in enumerate(("ExecuteAsync", "ShipAsync", "ConnectAsync"), 1):
                    line = next(line for line in text.splitlines() if "async Task" in line and f" {name}(" in line)
                    first = text.index(line)
                    end = text.index("\n    }", first) + len("\n    }")
                    branch, current = selected(path.name, text, standalone=text[first:end],
                                               start=text[:first].count("\n") + 1)
                    if index == 1:
                        process, context = branch, current
                        process.update(entryKind="HOSTED_BACKGROUND_WORKER", entrySymbol="ReaderWorker.ExecuteAsync")
                    else:
                        ref = f"s{index}"
                        body = branch["sources"]["s1"]
                        process["sources"][ref] = body
                        process["facts"].append({"kind": "BODY", "declaration": name, "sources": [ref]})
                        context["sourceRanges"].append({**body, "id": f"m123456789abc:e{index}", "references": [ref]})
                context["_nativeFiles"][path.name] = captured_csharp(text, path.name)
                facts, _ = self.project(process, context)
                deliveries = [row for row in facts if row["kind"] == "worker-row-delivery"]
                failures = [row for row in facts if row["kind"] == "worker-reselection" and "CommitAsync` can fail" in row["text"]]
                ack_failures = [row for row in facts if row["kind"] == "worker-reselection" and "already have accepted" in row["text"]]
                self.assertEqual(bool(deliveries), not label.startswith("unrelated"), facts)
                self.assertEqual(bool(failures), not label.startswith("unrelated"), facts)
                self.assertEqual(bool(ack_failures), not label.startswith("unrelated"), facts)
                if deliveries:
                    self.assertTrue(any("connection returned by `ConnectAsync(settings, token)`" in row["text"] for row in deliveries))
                    self.assertTrue(any("m123456789abc:e3" in row["sources"] for row in deliveries))
                    self.assertTrue(any("payload" in row["text"] and "id" in row["text"] for row in deliveries))
                    if label.startswith("json key"):
                        self.assertTrue(any('"routing"' in row["text"] and '"key"' in row["text"] for row in deliveries), deliveries)

    def test_roslyn_lookup_checks_later_rows_before_claiming_no_match(self):
        self.require_roslyn()
        header = """using System.Collections.Generic;
using System.Linq;
using Microsoft.AspNetCore.Http;
sealed record Candidate(int Id);
static class Finder
{
"""
        declaration = """    public static IResult Find(int id)
    {
        var candidates = new List<Candidate>();
        candidates.Add(new Candidate(1));
        candidates.Add(new Candidate(2));
        var found = candidates.FirstOrDefault(row => row.Id == TARGET);
        if (found is null) return Results.NotFound();
        return Results.Ok(found.Id);
    }"""
        for target, missing in (("2", False), ("3", True), ("id", None)):
            with self.subTest(target=target):
                body = declaration.replace("TARGET", target)
                text = header + body + "\n}\n"
                process, context = selected("Finder.cs", text, standalone=body, start=header.count("\n") + 1)
                context["_nativeFiles"]["Finder.cs"] = captured_csharp(text, "Finder.cs")
                facts, _ = self.project(process, context)
                labels = [row["text"] for row in facts]
                self.assertTrue(all("retained row fails" not in label for label in labels), labels)
                if missing is None:
                    self.assertFalse(any("tracked collection satisfies" in label for label in labels), labels)
                elif missing:
                    self.assertTrue(any("No row in the tracked collection satisfies" in label for label in labels), labels)
                else:
                    self.assertTrue(any("selects the first tracked row" in label for label in labels), labels)
                    self.assertFalse(any("No row in the tracked collection satisfies" in label or
                                         "HTTP 404" in label for label in labels), labels)

    def test_roslyn_switch_result_payload_stays_with_its_input_case(self):
        self.require_roslyn()
        header = """using Microsoft.AspNetCore.Http;
record Item(string Id, string Caption);
record Filter(string Id);
static class Reader
{
"""
        root = """    public static IResult Show(Filter input)
    {
        var item = Select(input.Id);
        if (item is null) return Results.BadRequest();
        return Results.Ok(item);
    }"""
        helper = """    private static Item? Select(string id) => id switch
    {
        "a" => new("a", "First"),
        "b" => new("b", "Second"),
        _ => null,
    };"""
        text = header + root + "\n" + helper + "\n}\n"
        process, context = selected("Reader.cs", text, standalone=root, start=header.count("\n") + 1)
        callee, _ = selected("Reader.cs", text, standalone=helper, start=header.count("\n") + root.count("\n") + 2)
        body = callee["sources"]["s1"]
        process["sources"]["s2"] = body
        process["facts"].append({"kind": "BODY", "declaration": "helper", "sources": ["s2"]})
        context["sourceRanges"].append({**body, "id": "m123456789abc:e2", "references": ["s2"]})
        context["_nativeFiles"]["Reader.cs"] = captured_csharp(text, "Reader.cs")
        facts, _ = self.project(process, context)
        success = [row["text"] for row in facts if row["kind"] == "http-outcome" and "HTTP 200" in row["text"]]
        self.assertTrue(any('`input.Id` matches `"a"`' in row and 'First' in row for row in success), success)
        self.assertTrue(any('`input.Id` matches `"b"`' in row and 'Second' in row for row in success), success)
        self.assertTrue(all('`input.Id` matches `"a"`' in row for row in success if 'First' in row), success)
        self.assertTrue(all('`input.Id` matches `"b"`' in row for row in success if 'Second' in row), success)
        invalid = [row["text"] for row in facts if row["kind"] == "http-outcome" and "HTTP 400" in row["text"]]
        self.assertTrue(any('`input.Id` differs from `"a"`' in row and
                            '`input.Id` differs from `"b"`' in row for row in invalid), invalid)

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
