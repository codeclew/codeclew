#!/usr/bin/env python3
"""Check configured Java handler tables and unrelated source that resembles them."""

from pathlib import Path
import re
import runpy
import shutil
import subprocess
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "fixtures/documentation-background-processes/java-dispatch"
DISCOVER = runpy.run_path(str(ROOT / "skills/codeclew/scripts/prepared_facts/clew_java_dispatch.py"))["discover"]
PACKAGE = "src/neutral/assembly/"
HANDLER = PACKAGE + "ReceiptHandler.java"
WIRING = PACKAGE + "DispatchWiring.java"
METHOD = "this::accept"
MAKE = 'factory.make("receipts-reader", handler.handlers())'


class JavaTypedDispatchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        java = shutil.which("java")
        version = subprocess.run([java, "-version"], capture_output=True, text=True) if java else None
        release = re.search(r'version "(\d+)', version.stderr) if version else None
        if not release or int(release[1]) < 21:
            raise unittest.SkipTest("The Java dispatcher parser requires JDK 21 or newer")
        cls.fixture = {file.relative_to(FIXTURE).as_posix(): {"text": file.read_text()}
                       for file in FIXTURE.rglob("*.java")}

    def prepared(self, edits=(), additions=None):
        files = {file: dict(record) for file, record in self.fixture.items()}
        for file, before, after in edits:
            self.assertIn(before, files[file]["text"])
            files[file]["text"] = files[file]["text"].replace(before, after)
        for file, text in (additions or {}).items():
            files[file] = {"text": text}
        return DISCOVER(files, deadline=time.monotonic() + 8), files

    def test_retained_factory_selects_method_reference_or_single_delegate(self):
        for callback in (METHOD, "value -> this.accept(value)", "value -> { accept(value); }"):
            with self.subTest(callback=callback):
                rows, files = self.prepared([(HANDLER, METHOD, callback)])
                row, = rows
                self.assertEqual((row["kind"], row["name"], row["adapter"], row["channel"], row["eventType"]),
                                 ("JAVA_TYPED_DISPATCH", "neutral.assembly.ReceiptHandler.accept", "EVENTUATE_TRAM_DOMAIN_EVENT",
                                  "receipts.pending", "neutral.assembly.BatchArrived"))
                ranged = lambda span: files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                self.assertEqual(ranged(row["registration"]), MAKE)
                self.assertIn("public void accept", ranged(row["callback"]))
                self.assertTrue(any(callback in ranged(span) for span in row["assemblySources"]))
                self.assertTrue(any("new ReceiptHandler()" in ranged(span) for span in row["assemblySources"]))

    def test_alternate_channel_and_initialized_dispatcher_constructor(self):
        rows, _ = self.prepared([(HANDLER, ".build();", '.andForAggregateType("receipts.backup")\n'
                                   '            .onEvent(BatchArrived.class, this::accept).build();')])
        self.assertEqual({row["channel"] for row in rows}, {"receipts.pending", "receipts.backup"})
        constructor = 'new DomainEventDispatcher("receipts-reader", handler.handlers(), consumer, names)'
        changes = [(WIRING, "import org.springframework.context.annotation.Bean;",
                    "import org.springframework.context.annotation.Bean;\n"
                    "import io.eventuate.tram.events.common.DomainEventNameMapping;\n"
                    "import io.eventuate.tram.messaging.consumer.MessageConsumer;"),
                   (WIRING, "DomainEventDispatcherFactory factory, ReceiptHandler handler",
                    "MessageConsumer consumer, DomainEventNameMapping names, ReceiptHandler handler"),
                   (WIRING, MAKE, constructor)]
        rows, _ = self.prepared(changes)
        self.assertEqual(rows, [])
        rows, _ = self.prepared([*changes, (WIRING, "@Bean\n    public DomainEventDispatcher",
                                           '@Bean(initMethod = "initialize")\n    public DomainEventDispatcher')])
        self.assertEqual([row["name"] for row in rows], ["neutral.assembly.ReceiptHandler.accept"])

    def test_unwired_opaque_conflicting_or_unbound_candidates_do_not_create_roots(self):
        cases = {
            "unwired table": [(WIRING, "@Bean\n    public DomainEventDispatcher", "public DomainEventDispatcher")],
            "unknown factory": [(WIRING, "import io.eventuate.tram.events.subscriber.DomainEventDispatcherFactory;",
                                 "import neutral.unknown.DomainEventDispatcherFactory;")],
            "opaque factory": [(WIRING, MAKE, "unknown(handler.handlers())")],
            "opaque bean": [(WIRING, "new ReceiptHandler()", "loadReceiptHandler()")],
            "configured bean ambiguous": [(WIRING, "    @Bean\n    public DomainEventDispatcher",
                                            "    @Bean public ReceiptHandler other() { return new ReceiptHandler(); }\n"
                                            "    @Bean\n    public DomainEventDispatcher")],
            "conflicting message callbacks": [(HANDLER, ".build();", ".onEvent(BatchArrived.class, this::accept).build();")],
            "wrong event argument": [(HANDLER, "Envelope<BatchArrived>", "Envelope<UnseenEvent>")],
            "opaque lambda": [(HANDLER, METHOD, "value -> dispatchElsewhere(value)")],
            "instance callback from static table": [(HANDLER, "public DomainEventHandlers handlers()",
                                                    "public static DomainEventHandlers handlers()"),
                                                   (HANDLER, METHOD, "value -> accept(value)")],
            "reassigned factory receiver": [(WIRING, "return factory.make", "factory = opaque();\n        return factory.make")],
            "same spelling but different builder": [(HANDLER, "import io.eventuate.tram.events.subscriber.DomainEventHandlersBuilder;",
                                                    "import neutral.unknown.DomainEventHandlersBuilder;")],
        }
        for label, mutations in cases.items():
            with self.subTest(case=label):
                self.assertEqual(self.prepared(mutations)[0], [])
        self.assertEqual(self.prepared(additions={
            "src/io/eventuate/tram/events/subscriber/DomainEventHandlersBuilder.java":
                "package io.eventuate.tram.events.subscriber; public final class DomainEventHandlersBuilder {}",
        })[0], [])


if __name__ == "__main__":
    unittest.main()
