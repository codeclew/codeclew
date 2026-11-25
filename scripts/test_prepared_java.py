#!/usr/bin/env python3
"""Check Java projections and Work citation binding through the packaged adapter."""
import hashlib
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1] / 'skills/codeclew/scripts'
JAVA = runpy.run_path(str(SCRIPTS / 'prepared_facts/java.py'))
TRANSPORT = runpy.run_path(str(SCRIPTS / 'docs_run_prepared.py'))
TRANSPORT['bound_facts'].__globals__.update(
    sha=lambda raw: 'sha256:' + hashlib.sha256(raw).hexdigest(), digest=repr,
)
JDK = shutil.which('java')
HAS_JDK = JDK and subprocess.run([JDK, '--source', '21', '-version'], capture_output=True).returncode == 0
PATH = 'src/catalog/Entry.java'
HELPER_PATH = 'src/catalog/Helper.java'
BODY = '''String change(String input, boolean permitted) {
        if (!permitted) return "blocked";
        final String stored = input.trim();
        final Helper helper = new Helper();
        helper.save(stored);
        return stored;
    }'''
FILE = f'package catalog;\nfinal class Entry {{\n    {BODY}\n}}\n'
HELPER = '''package catalog;
final class Helper {
    String latest;
    void save(String value) { this.latest = value; }
}
'''


def digest(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()


def blob(raw):
    return hashlib.sha1(f'blob {len(raw)}\0'.encode() + raw).hexdigest()


def source(file, full, text, *, revision='a' * 40, ref='s1', corrupt_blob=False):
    raw, payload = full.encode(), text.encode()
    start = raw.index(payload)
    end = start + len(payload)
    first = raw[:start].count(b'\n') + 1
    return {'file': file, 'revision': revision, 'service': 'catalog', 'text': text,
            'authority': 'EXACT_SNAPSHOT_TEXT', 'textDigest': digest(payload),
            'references': [ref], 'startByte': start, 'endByte': end, 'startLine': first,
            'endLine': first + payload.removesuffix(b'\n').count(b'\n'),
            'occurrence': {'snapshot': 'sha256:' + 'b' * 64, 'blob': '0' * 40 if corrupt_blob else blob(raw),
                           'startByte': start, 'endByte': end}}


def fixture(*, discovery=False, subranges=False, helper_cited=True, revision='a' * 40, repository=None,
            corrupt_blob=False, legacy=False, committed=True):
    selected = BODY if subranges else FILE
    exact = {'C1': source(PATH, FILE, selected, revision=revision, corrupt_blob=corrupt_blob)}
    if helper_cited:
        exact['C2'] = source(HELPER_PATH, HELPER, HELPER, revision=revision, ref='s2')
    if legacy:
        exact = {handle: {key: value for key, value in row.items()
                          if key not in ('occurrence', 'startByte', 'endByte')}
                 for handle, row in exact.items()}
    requirement = {'kind': 'DISCOVERY' if discovery else 'ENTRY', 'file': PATH if discovery else None,
                   'id': 'sample', 'rootDeclarations': [] if discovery else ['root'], 'sourceReferences': ['s1'],
                   'service': 'catalog', 'page': 'Change catalog'}
    units = TRANSPORT['full_files'](exact)
    operation = [{'id': 'BODY:root', 'obligationKind': 'BODY',
                  'record': {'declaration': 'root', 'sourceIds': ['body']}, 'sourceReferences': ['s1']}]
    process, context = TRANSPORT['operation'](requirement, [] if discovery else operation, exact, units)
    context.update(_revision=revision, _snapshot='sha256:' + 'c' * 64 + '/1', _repository=repository,
                   _allowLegacyPinned=committed)
    rows, ranges = JAVA['project'](process, context, node=None)

    def read_pinned(file, selected_revision):
        return subprocess.check_output(['git', '-C', repository, 'show', f'{selected_revision}:{file}']).decode()

    facts, rejected = TRANSPORT['bound_facts'](rows, ranges, exact, {site['id'] for site in context['sourceRanges']},
                                               read_pinned if repository and committed else None)
    return facts, ranges, rejected


def publication_fixture(body, *, receiver='DomainEventPublisher', files=(), compiler=()):
    main = f"""package catalog;
import java.util.List;
import io.eventuate.tram.events.publisher.DomainEventPublisher;
import io.eventuate.tram.events.common.DomainEvent;
final class Entry {{
    private final {receiver} publisher;
    Entry({receiver} publisher) {{ this.publisher = publisher; }}
    {body}
}}
record Added(String id) implements DomainEvent {{}}
final class LocalPublisher {{
    void publish(Class<?> key, String id, List<?> items) {{}}
}}
"""
    exact = {'file': source(PATH, main, main, ref='file-root'),
             'declaration': source(PATH, main, body, ref='root')}
    exact.update({file: source(file, text, text, ref=file) for file, text in files})
    work = [{'id': 'BODY:root', 'obligationKind': 'BODY', 'record': {'declaration': 'root', 'sourceIds': ['body']},
             'sourceReferences': ['root']}]
    for index, (literal, category, member) in enumerate(compiler):
        identity = f'event-{index}'
        exact[identity] = source(PATH, main, literal, ref=identity)
        work.append({'id': f'EVENT:{identity}', 'obligationKind': 'EVENT', 'sourceReferences': [identity],
                     'record': {'declaration': 'root', 'sourceIds': [identity], 'event': {
                         'kind': category, 'target': member, 'resolution': 'COMPILER_EXACT'}}})
    requirement = {'kind': 'ENTRY', 'id': 'entry', 'entryKind': 'HTTP_ENDPOINT', 'rootDeclarations': ['root'],
                   'service': 'catalog', 'page': 'entry.md', 'sourceReferences': ['root']}
    process, context = TRANSPORT['operation'](requirement, work, exact, TRANSPORT['full_files'](exact))
    context.update(_revision='a' * 40, _snapshot='sha256:' + 'b' * 64)
    facts, added = JAVA['project'](process, context, node=None)
    accepted, rejected = TRANSPORT['bound_facts'](facts, added, exact, {row['id'] for row in context['sourceRanges']})
    return [row for row in accepted if row['kind'] == 'java-typed-publication'], rejected


@unittest.skipUnless(HAS_JDK, 'JDK 21 or newer required')
class PackagedJava(unittest.TestCase):
    def test_typed_java_publication_uses_admitted_qualified_event_and_api(self):
        call = 'publisher.publish(Entry.class, id, List.of(added))'
        body = f'void add(String id) {{ var added = new Added(id); {call}; }}'
        records = [
            (call, 'CALL', 'method:class:io.eventuate.tram.events.publisher.DomainEventPublisher#publish(Ljava/lang/Class;Ljava/lang/Object;Ljava/util/List;)V'),
            ('List.of(added)', 'CALL', 'method:class:java.util.List#of(Ljava/lang/Object;)Ljava/util/List;'),
            ('new Added(id)', 'CONSTRUCT', 'method:class:catalog.Added#<init>(Ljava/lang/String;)V'),
        ]
        for compiler, authority in (((), 'RETAINED_JAVA_PUBLICATION_CANDIDATE'),
                                    (records, 'COMPILER_SELECTED_JAVA_PUBLICATION')):
            with self.subTest(authority=authority):
                publications, rejected = publication_fixture(body, compiler=compiler)
                self.assertEqual(rejected, 0)
                row, = publications
                self.assertEqual(row['authority'], authority)
                self.assertEqual(row['_edge']['adapter'], 'EVENTUATE_TRAM_DOMAIN_EVENT')
                self.assertEqual(row['_edge']['channel'], 'catalog.Entry')
                self.assertEqual(row['_edge']['eventType'], 'catalog.Added')
                self.assertTrue(any(site['textDigest'] == digest(b'new Added(id)') for site in row['witnesses']))
                self.assertIn('activation and delivery remain open', row['uncertainty'])

    def test_typed_java_publication_rejects_name_collisions_and_leaves_dynamic_channel_open(self):
        known = 'void add(String id) { publisher.publish(Entry.class, id, List.of(new Added(id))); }'
        foreign = 'void add(String id) { publisher.publish(Entry.class, id, List.of(new foreign.Added(id))); }'
        dynamic = 'void add(String id, String topic) { publisher.publish(topic, id, List.of(new Added(id))); }'
        for source_text in (known, foreign):
            self.assertFalse(publication_fixture(source_text, receiver='LocalPublisher')[0])
        foreign_unit = ("src/foreign/Added.java", """package foreign;
record Added(String id) implements io.eventuate.tram.events.common.DomainEvent { }
""")
        row, = publication_fixture(foreign, files=[foreign_unit])[0]
        self.assertEqual(row['_edge']['eventType'], 'foreign.Added')
        row, = publication_fixture(dynamic)[0]
        self.assertIsNone(row['_edge']['channel'])
        self.assertEqual(row['_edge']['eventType'], 'catalog.Added')
        self.assertIn('remains open', row['text'])

    def test_typed_java_aggregate_publication_requires_constructor_or_unique_bean(self):
        prefix = 'io.eventuate.tram.events.publisher'
        wrapper = ("src/catalog/ConfiguredPublisher.java", f"""package catalog;
import {prefix}.AbstractDomainEventPublisherForAggregateImpl;
import {prefix}.DomainEventPublisher;
final class ConfiguredPublisher extends AbstractDomainEventPublisherForAggregateImpl<Entry, String, Added> implements Port {{
    ConfiguredPublisher(DomainEventPublisher raw) {{
        super(Entry.class, entry -> "entry", raw, Added.class);
    }}
}}
""")
        port = ("src/catalog/Port.java", f"""package catalog;
import {prefix}.DomainEventPublisherForAggregate;
interface Port extends DomainEventPublisherForAggregate<Entry, String, Added> {{ }}
""")
        bean = ("src/catalog/Config.java", f"""package catalog;
import org.springframework.context.annotation.Bean;
import {prefix}.DomainEventPublisher;
final class Config {{
    @Bean Port route(DomainEventPublisher raw) {{ return new ConfiguredPublisher(raw); }}
}}
""")
        other = ("src/catalog/Ambiguous.java", bean[1].replace('class Config', 'class Ambiguous'))
        call = 'publisher.publishById(id, new Added(id))'
        body = f'void add(String id) {{ {call}; }}'
        target = f'method:class:{prefix}.DomainEventPublisherForAggregate#publishById(Ljava/lang/Object;Lio/eventuate/tram/events/common/DomainEvent;)V'
        for receiver, support, expected in (
            ("ConfiguredPublisher", [wrapper, port], "catalog.Entry"),
            ("Port", [wrapper, port, bean], "catalog.Entry"),
            ("Port", [wrapper, port], None),
            ("Port", [wrapper, port, bean, other], None),
        ):
            with self.subTest(receiver=receiver, support=len(support)):
                rows, rejected = publication_fixture(body, receiver=receiver, files=support,
                                                     compiler=[(call, 'CALL', target)])
                self.assertEqual(rejected, 0)
                row, = rows
                self.assertEqual(row['_edge']['channel'], expected)
                self.assertEqual(row['_edge']['eventType'], 'catalog.Added')
                if expected:
                    self.assertIn('retained constructor', row['uncertainty'])
                    self.assertIn(wrapper[0], {w['file'] for w in row['witnesses']})
                    if receiver == "Port":
                        self.assertIn(bean[0], {w['file'] for w in row['witnesses']})

    def test_compiler_selected_event_collection_is_bound_to_constructor_and_accessor(self):
        prefix = 'io.eventuate.tram.events.publisher'
        bundle = f'new {prefix}.ResultWithTypedEvents<>(id, new Added(id))'
        accessor = f'{bundle}.getEvents()'
        call = f'publisher.publish(Entry.class, id, {accessor})'
        body = f'void add(String id) {{ {call}; }}'
        compiler = [
            (call, 'CALL', f'method:class:{prefix}.DomainEventPublisher#publish(Ljava/lang/Class;Ljava/lang/Object;Ljava/util/List;)V'),
            (accessor, 'CALL', f'method:class:{prefix}.ResultWithTypedEvents#getEvents()Ljava/util/List;'),
            (bundle, 'CONSTRUCT', f'method:class:{prefix}.ResultWithTypedEvents#<init>(Ljava/lang/Object;Lio/eventuate/tram/events/common/DomainEvent;)V'),
            ('new Added(id)', 'CONSTRUCT', 'method:class:catalog.Added#<init>(Ljava/lang/String;)V'),
        ]
        good, = publication_fixture(body, compiler=compiler)[0]
        self.assertEqual(good['_edge']['eventType'], 'catalog.Added')
        self.assertEqual(good['_edge']['channel'], 'catalog.Entry')
        self.assertTrue(any(site['textDigest'] == digest(bundle.encode()) for site in good['witnesses']))
        foreign = (accessor, 'CALL', 'method:class:other.ResultWithTypedEvents#getEvents()Ljava/util/List;')
        open_event, = publication_fixture(body, compiler=[compiler[0], foreign, *compiler[2:]])[0]
        self.assertIsNone(open_event['_edge']['eventType'])

    def test_native_body_without_full_blob_still_projects_lexical_guards(self):
        rows, ranges, rejected = fixture(subranges=True)
        paths = [row for row in rows if row['kind'] == 'java-guarded-outcome']
        self.assertTrue(any('`permitted` is false' in row['condition'] and 'bypasses later' in row['text'] for row in paths))
        self.assertTrue(any(row.get('status') == 'SNAPSHOT_SOURCE_UNAVAILABLE' for row in rows))
        self.assertEqual(rejected, 0)
        self.assertEqual(ranges, {})

    def test_discovery_projects_only_covered_declarations_and_callee_witnesses(self):
        rows, ranges, rejected = fixture(discovery=True)
        path = next(row for row in rows if row['kind'] == 'java-guarded-outcome')
        self.assertIn('source declaration `catalog.Entry.change`', path['condition'])
        self.assertEqual(path['sources'], ['C1'])
        mutation = next(row for row in rows if row['kind'] == 'java-callee-path')
        self.assertIn('this.latest', mutation['text'])
        self.assertIn('C2', mutation['sources'])
        self.assertTrue(any(row['file'] == HELPER_PATH for row in ranges.values()))
        self.assertEqual(rejected, 0)

    def test_pinned_calculation_cannot_publish_uncovered_helper(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            for relative, code in ((PATH, FILE), (HELPER_PATH, HELPER)):
                file = repo / relative
                file.parent.mkdir(parents=True, exist_ok=True)
                file.write_text(code)

            def git(*args):
                return subprocess.check_output(['git', '-C', directory, *args], stderr=subprocess.DEVNULL)

            git('init', '-q')
            git('add', '.')
            git('-c', 'user.name=Test', '-c', 'user.email=test@example.invalid', 'commit', '-qm', 'Fixture')
            revision = git('rev-parse', 'HEAD').decode().strip()
            (repo / PATH).write_text('class Changed {}')
            visible, _, rejected = fixture(discovery=True, helper_cited=False, revision=revision, repository=directory)
            self.assertTrue(any(row['kind'] == 'java-guarded-outcome' for row in visible))
            self.assertTrue(any(row.get('status') == 'SNAPSHOT_SOURCE_UNAVAILABLE' for row in visible))
            self.assertFalse(any('this.latest' in row['text'] for row in visible))
            self.assertEqual(rejected, 0)
            wrong, _, rejected = fixture(discovery=True, subranges=True, helper_cited=False, revision=revision,
                                         repository=directory, corrupt_blob=True)
            self.assertEqual([row.get('status') for row in wrong], ['SNAPSHOT_SOURCE_UNAVAILABLE'])
            self.assertEqual(rejected, 0)
            legacy, _, rejected = fixture(subranges=True, revision=revision, repository=directory, legacy=True)
            mutation = next(row for row in legacy if row['kind'] == 'java-callee-path')
            self.assertIn('C2', mutation['sources'])
            self.assertEqual(rejected, 0)
            working_tree, _, rejected = fixture(subranges=True, revision=revision, repository=directory,
                                                legacy=True, committed=False)
            self.assertFalse(any(row['kind'] == 'java-callee-path' for row in working_tree))
            self.assertTrue(any(row.get('status') == 'SNAPSHOT_SOURCE_UNAVAILABLE' for row in working_tree))
            self.assertEqual(rejected, 0)


if __name__ == '__main__':
    unittest.main()
