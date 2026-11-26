#!/usr/bin/env python3
"""Check retained Java dispatch wiring and unrelated source that resembles it."""

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
ANNOTATED = FIXTURE.parent / "java-annotation" / "receiver"
ANNOTATED_HANDLER = "src/neutral/receiver/Sink.java"
ANNOTATED_WIRING = "src/neutral/receiver/Assembly.java"
PACKAGE = "src/neutral/assembly/"
HANDLER = PACKAGE + "ReceiptHandler.java"
WIRING = PACKAGE + "DispatchWiring.java"
METHOD = "this::accept"
MAKE = 'factory.make("receipts-reader", handler.handlers())'
NO_REGISTRAR = [
    (ANNOTATED_WIRING, "import io.eventuate.tram.spring.events.subscriber.TramEventSubscriberConfiguration;\n", ""),
    (ANNOTATED_WIRING, "@Import(TramEventSubscriberConfiguration.class)\n", ""),
]


def java_files(root):
    return {file.relative_to(root).as_posix(): {"text": file.read_text()} for file in sorted(root.rglob("*.java"))}


class JavaTypedDispatchTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        java = shutil.which("java")
        version = subprocess.run([java, "-version"], capture_output=True, text=True) if java else None
        release = re.search(r'version "(\d+)', version.stderr) if version else None
        if not release or int(release[1]) < 21:
            raise unittest.SkipTest("The Java dispatcher parser requires JDK 21 or newer")
        cls.fixture = java_files(FIXTURE)
        cls.annotated = java_files(ANNOTATED)

    def prepared(self, edits=(), additions=None, *, fixture=None):
        files = {file: dict(record) for file, record in (fixture or self.fixture).items()}
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
                self.assertEqual(row["subscriberId"], "receipts-reader")
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


    def test_annotated_bean_keeps_each_exact_channel_and_event_type(self):
        rows, files = self.prepared(fixture=self.annotated)
        self.assertEqual({(row["name"], row["channel"], row["eventType"], row["subscriberId"]) for row in rows}, {
            ("neutral.receiver.Sink.accept", "neutral.signal", "neutral.contract.Signal", "signal-reader"),
            ("neutral.receiver.Sink.acceptBackup", "neutral.backup", "neutral.contract.Signal", "signal-reader"),
            ("neutral.receiver.Sink.acceptOtherType", "neutral.signal", "neutral.foreign.Signal", "signal-reader"),
        })
        row = next(row for row in rows if row["name"].endswith(".accept"))
        self.assertEqual((row["kind"], row["adapter"]), ("JAVA_TYPED_DISPATCH", "EVENTUATE_TRAM_DOMAIN_EVENT"))
        excerpts = [files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                    for span in [row["registration"], row["callback"], *row["assemblySources"]]]
        for witness in ('@EventuateDomainEventHandler(subscriberId = "signal-reader", channel = "neutral.signal")',
                        "public void accept(DomainEventEnvelope<Signal> message)", "@Bean", "new Sink()",
                        "@Import(TramEventSubscriberConfiguration.class)"):
            self.assertTrue(any(witness in excerpt for excerpt in excerpts), witness)

    def test_renamed_annotated_handler_resolves_retained_channel_alias(self):
        constants = "src/neutral/receiver/Routes.java"
        edits = [
            (ANNOTATED_HANDLER, "import neutral.contract.Signal;", "import neutral.contract.Signal;\n"
             "import static neutral.receiver.Routes.SELECTED;"),
            (ANNOTATED_HANDLER, 'channel = "neutral.signal"', "channel = SELECTED"),
            (ANNOTATED_HANDLER, "public void accept(", "public void renamed("),
            (ANNOTATED_WIRING, " sink()", " renamedBean()"),
        ]
        rows, files = self.prepared(edits, {constants: 'package neutral.receiver; interface Routes {'
                                                      ' String PREFIX = "neutral."; String SELECTED = PREFIX + "signal"; }'},
                                    fixture=self.annotated)
        row = next(row for row in rows if row["name"].endswith(".renamed"))
        self.assertEqual(row["channel"], "neutral.signal")
        excerpts = [files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                    for span in row["assemblySources"]]
        self.assertTrue(any("import static neutral.receiver.Routes.SELECTED" in excerpt for excerpt in excerpts))
        self.assertTrue(any('String PREFIX = "neutral."' in excerpt for excerpt in excerpts))

    def test_exact_bean_declaration_survives_an_unseen_registrar(self):
        wired, _ = self.prepared(fixture=self.annotated)
        declared, files = self.prepared(NO_REGISTRAR, fixture=self.annotated)
        self.assertEqual([row["name"] for row in wired], [row["name"] for row in declared])
        self.assertTrue(all(not row["registrarOpen"] for row in wired))
        self.assertTrue(all(row["registrarOpen"] for row in declared))
        row = next(row for row in declared if row["name"].endswith(".accept"))
        excerpts = [files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                    for span in row["assemblySources"]]
        self.assertIn("@Bean", excerpts)
        self.assertIn("return handler;", excerpts)
        self.assertFalse(any("TramEventSubscriberConfiguration" in excerpt for excerpt in excerpts))

    def test_boot_auto_configuration_marker_preserves_the_exact_bean_declaration(self):
        imports = "import org.springframework.boot.autoconfigure.EnableAutoConfiguration;"
        changes = [(ANNOTATED_WIRING, "import org.springframework.context.annotation.Bean;",
                    imports + "\nimport org.springframework.context.annotation.Bean;"),
                   (ANNOTATED_WIRING, "@Configuration", "@Configuration\n@EnableAutoConfiguration")]
        rows, files = self.prepared([*NO_REGISTRAR, *changes], fixture=self.annotated)
        self.assertEqual(len(rows), 3)
        for row in rows:
            self.assertTrue(row["registrarOpen"])
            self.assertFalse(row["beanActivationOpen"])
            excerpts = [files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                        for span in row["assemblySources"]]
            self.assertIn("@EnableAutoConfiguration", excerpts)
            self.assertIn(imports, excerpts)

    def test_unknown_or_conditional_annotations_preserve_only_the_bean_declaration(self):
        cases = {
            "foreign Boot marker": ("@EnableAutoConfiguration",
                                    "import neutral.foreign.EnableAutoConfiguration;"),
            "real Spring profile": ('@Profile("test")', "import org.springframework.context.annotation.Profile;"),
            "unresolved configuration decorator": ("@EnvironmentGate", ""),
            "unresolved factory decorator": ("@FactoryGate", ""),
        }
        for label, (decorator, imported) in cases.items():
            with self.subTest(case=label):
                if label == "unresolved factory decorator":
                    member, decorated = "    @Bean", f"    {decorator}\n    @Bean"
                else:
                    member, decorated = "@Configuration", f"@Configuration\n{decorator}"
                changes = [(ANNOTATED_WIRING, member, decorated)]
                if imported:
                    changes.append((ANNOTATED_WIRING, "import org.springframework.context.annotation.Bean;",
                                    imported + "\nimport org.springframework.context.annotation.Bean;"))
                rows, files = self.prepared([*NO_REGISTRAR, *changes], fixture=self.annotated)
                self.assertEqual(len(rows), 3)
                for row in rows:
                    self.assertTrue(row["registrarOpen"] and row["beanActivationOpen"])
                    excerpts = [files[span["file"]]["text"].encode()[span["start"]:span["end"]].decode()
                                for span in row["assemblySources"]]
                    self.assertIn(decorator, excerpts)

    def test_partial_registration_requires_a_retained_exact_spring_bean(self):
        foreign = [(ANNOTATED_WIRING, "org.springframework.context.annotation.Bean", "neutral.foreign.Bean")]
        self.assertEqual(self.prepared([*NO_REGISTRAR, *foreign], fixture=self.annotated)[0], [])
        without_handler = {file: row for file, row in self.annotated.items() if file != ANNOTATED_HANDLER}
        self.assertEqual(self.prepared(NO_REGISTRAR, fixture=without_handler)[0], [])

    def test_annotated_registration_requires_exact_annotations_and_known_operands(self):
        cases = {
            "foreign framework configuration": [(ANNOTATED_WIRING,
                "io.eventuate.tram.spring.events.subscriber.TramEventSubscriberConfiguration",
                "neutral.foreign.TramEventSubscriberConfiguration")],
            "absent bean": [(ANNOTATED_WIRING, "    @Bean\n", "")],
            "opaque bean": [(ANNOTATED_WIRING, "new Sink()", "createSink()")],
            "foreign envelope": [(ANNOTATED_HANDLER, "io.eventuate.tram.events.subscriber.DomainEventEnvelope",
                                  "neutral.foreign.DomainEventEnvelope")],
            "unknown subscriber": [(ANNOTATED_HANDLER, 'subscriberId = "signal-reader"', "subscriberId = Unseen.KEY")],
        }
        for label, changes in cases.items():
            with self.subTest(case=label):
                self.assertEqual(self.prepared(changes, fixture=self.annotated)[0], [])

    def test_explicit_import_registers_component_without_factory_method(self):
        changes = [(ANNOTATED_WIRING, "@Import(TramEventSubscriberConfiguration.class)",
                    "@Import({TramEventSubscriberConfiguration.class, Sink.class})"),
                   (ANNOTATED_WIRING, "    @Bean\n", "")]
        rows, _ = self.prepared(changes, fixture=self.annotated)
        self.assertEqual(len(rows), 3)

    def test_duplicate_annotation_registration_does_not_select_first_match(self):
        changes = [(ANNOTATED_HANDLER, 'channel = "neutral.backup"', 'channel = "neutral.signal"')]
        for activation in ([], NO_REGISTRAR):
            rows, _ = self.prepared([*activation, *changes], fixture=self.annotated)
            self.assertEqual([row["name"] for row in rows], ["neutral.receiver.Sink.acceptOtherType"])


if __name__ == "__main__":
    unittest.main()
