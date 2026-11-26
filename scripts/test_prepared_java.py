#!/usr/bin/env python3
"""Check Java projections and Work citation binding through the packaged adapter."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch


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


def publication_fixture(body, *, receiver='DomainEventPublisher', files=(), compiler=(), all_facts=False,
                        source_members=None):
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
    context.update(_revision='a' * 40, _snapshot='sha256:' + 'b' * 64, _sourceMembers=source_members)
    facts, added = JAVA['project'](process, context, node=None)
    accepted, rejected = TRANSPORT['bound_facts'](facts, added, exact, {row['id'] for row in context['sourceRanges']})
    return (accepted if all_facts else [row for row in accepted if row['kind'] == 'java-typed-publication']), rejected


def operation_fixture(body, local, helper):
    main = f'package catalog;\nfinal class Entry {{\n    {body}\n    {local}\n}}\n'
    helper = f'package catalog;\n{helper}\n'
    sources = {
        'entry-unit': source(PATH, main, main, ref='entry-unit'),
        'root': source(PATH, main, body, ref='root'),
        'helper': source(HELPER_PATH, helper, helper, ref='helper'),
    }
    requirement = {'kind': 'ENTRY', 'id': 'catalog:choose', 'entryKind': 'HTTP_ENDPOINT', 'rootDeclarations': ['root'],
                   'service': 'catalog', 'page': 'choose.md', 'sourceReferences': ['root'], 'handlerId': 'choose',
                   'method': 'GET', 'path': '/choose'}
    work = [{'id': 'BODY:root', 'obligationKind': 'BODY', 'record': {'declaration': 'root', 'sourceIds': ['body']},
             'sourceReferences': ['root']}]
    process, context = TRANSPORT['operation'](requirement, work, sources, TRANSPORT['full_files'](sources))
    context.update(_revision='a' * 40, _snapshot='sha256:' + 'b' * 64)
    rows, ranges = JAVA['project'](process, context, node=None)
    facts, rejected = TRANSPORT['bound_facts'](rows, ranges, sources, {row['id'] for row in context['sourceRanges']})
    return requirement, sources, facts, rejected


def witness_texts(fact, sources):
    for witness in fact['witnesses']:
        source = sources[witness['cite']]
        lower = witness['startByte'] - source['startByte']
        upper = witness['endByte'] - source['startByte']
        yield source['text'].encode()[lower:upper].decode()


class InstalledJava(unittest.TestCase):
    def __init__(self, launcher, skill, renderer, scratch_parent):
        super().__init__('test_installed_source_admits_cross_file_java_values_into_operation_prompt')
        self.launcher = launcher
        self.skill = skill
        self.renderer = renderer
        self.scratch_parent = scratch_parent

    def test_installed_source_admits_cross_file_java_values_into_operation_prompt(self):
        self.assertTrue(HAS_JDK, 'Installed Java qualification requires JDK 21 or newer.')
        launcher = str(self.launcher)
        main = '''package catalog;
import java.util.Map;
import org.springframework.web.bind.annotation.PostMapping;
import org.springframework.web.bind.annotation.RequestBody;
import org.springframework.web.bind.annotation.RestController;

@RestController public final class Entry {
    private Reader reader;

    @PostMapping("/choose")
    public String choose(@RequestBody Parcel parcel) {
        if (!parcel.ready()) return "blocked";
        Item selected = forward(parcel, true);
        return selected != null ? "selected" : "unavailable";
    }

    @PostMapping("/inclusive")
    public String inclusive(@RequestBody Parcel parcel) {
        Item selected = forward(parcel, false);
        return selected != null ? "included" : "unavailable";
    }

    @PostMapping("/shadow")
    public Object shadow(Item item, @RequestBody Parcel parcel, boolean hold, boolean retry) {
        if (!item.isNew()) return new Receipt("existing", item);
        Item selected = forward(parcel, true);
        if (selected != null) return selected;
        if (hold) return new Receipt("holding", parcel.item());
        try {
            reader.refresh(parcel);
        } catch (IllegalArgumentException failure) {
            if (retry) return new Receipt("try again", parcel.item());
            throw failure;
        }
        return completed();
    }

    private String completed() { return "queued"; }

    private Item forward(Parcel parcel, boolean ignoreNew) {
        return parcel.rules().select(parcel.item().getLabel(), ignoreNew);
    }

    @PostMapping("/display")
    public String display(int requested, @RequestBody Parcel parcel, Map<String, Object> view) {
        String key = parcel.label();
        if (key == null) key = "";
        else key = key.strip();
        Slice<Item> page = query(requested, key);
        if (requested < 1 || requested > Math.max(page.pages(), 1)) {
            view.put("page", 1);
            if (!key.isEmpty()) view.put("key", key);
            return "bad-request";
        }
        if (page.empty()) return "no-results";
        if (page.count() == 1) return "single:" + page.entries().get(0).size;
        return finish(requested, view, page);
    }

    private Slice<Item> query(int requested, String key) {
        Window window = new Window(requested);
        return reader.find(key, window);
    }

    private String finish(int requested, Map<String, Object> view, Slice<Item> page) {
        var items = page.entries();
        view.put("items", items);
        view.put("requested", requested);
        view.put("count", page.count());
        view.put("total", page.pages());
        return "display";
    }
}
record Receipt(String status, Item item) {}
'''
        parcel = '''package catalog;
public record Parcel(boolean ready, String label, Helper rules, Item item) {}
'''
        helper = '''package catalog;
public class Helper {
    private Item[] items;

    Item select(String label) { return null; }
    Item select(Integer number) { return null; }
    Item select(String label, boolean ignoreNew) {
        for (Item item : items) {
            String name = item.getLabel();
            if (name.equalsIgnoreCase(label)) {
                if (!ignoreNew || !item.isNew()) return item;
            }
        }
        return null;
    }
}
'''
        item = '''package catalog;
import catalog.model.Named;
public class Item extends Named {
    int size;
}
'''
        named = '''package catalog.model;
import catalog.model.ids.Identity;
public class Named extends Identity {
    private String label;
    public String getLabel() { return this.label; }
}
'''
        identity = '''package catalog.model.ids;
public class Identity implements java.io.Serializable {
    private Integer id;
    public boolean isNew() { return this.id == null; }
}
'''
        reader = '''package catalog;
interface Reader extends java.io.Serializable {
    /** Selects items with a label starting with the supplied prefix. */
    Slice<Item> find(String key, Window window);
    void refresh(Parcel parcel);
}
record Window(int requested) {}
interface Slice<T> {
    int pages();
    boolean empty();
    long count();
    java.util.List<T> entries();
}
'''
        docs = runpy.run_path(str(self.skill / 'scripts/docs_run.py'))
        with tempfile.TemporaryDirectory(prefix='clew-java-values-', dir=self.scratch_parent) as directory:
            base = Path(directory).resolve()
            repo, root, run = (base / name for name in ('catalog', 'docs', 'run'))
            environment = {**os.environ, 'CODECLEW_HOME': str(base / 'state')}
            for name, text in (('Entry', main), ('Parcel', parcel), ('Helper', helper), ('Item', item),
                               ('model/Named', named), ('model/ids/Identity', identity), ('Reader', reader)):
                path = repo / 'src' / 'catalog' / (name + '.java')
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(text)
            remote = 'https://example.invalid/catalog.git'

            def git(*args):
                subprocess.run(['git', '-C', str(repo), *args], check=True, capture_output=True)

            def clew(*args):
                result = subprocess.run([launcher, 'docs', *args, '--root', str(root)], env=environment,
                                        capture_output=True, text=True, timeout=30)
                self.assertIn(result.returncode, (0, 3) if args[0] == 'check' else (0,), result.stdout + result.stderr)
                return json.loads(result.stdout)

            git('init', '-q', '-b', 'main')
            git('remote', 'add', 'origin', remote)
            git('add', '.')
            git('-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qm', 'Fixture')
            clew('init', '--title', 'Catalog selection')
            spec = base / 'service.json'
            docs['save'](spec, {'schema': 'codeclew-documentation-service/1.0', 'id': 'catalog', 'title': 'Catalog',
                                'repositoryId': 'catalog', 'repository': remote, 'language': 'java',
                                'targetRef': 'main', 'profile': 'source-syntax', 'source': {'roots': ['src'], 'dialect': '17'}})
            clew('service', 'add', '--input', str(spec), '--expected-input-digest', clew('service', 'list')['inputDigest'])
            clew('bind', '--service', 'catalog', '--repo', str(repo))
            checked = clew('check')
            self.assertEqual(checked['status'], 'CHECKED')
            snapshot = checked['snapshot']
            job = base / 'job.json'
            docs['save'](job, {'schema': docs['SCHEMA'], 'clew': launcher, 'root': str(root), 'snapshot': snapshot,
                               'services': ['catalog'], 'request': 'Document catalog selection.', 'language': 'en',
                               'renderer': str(self.renderer),
                               'node': str(Path(shutil.which('node')).absolute()), 'evidenceMode': 'PREPARED_FACTS',
                               'authorMode': 'ISOLATED_PAGES',
                               'author': {'executable': sys.executable, 'model': 'fixture-author', 'reasoningEffort': 'low'},
                               'budget': {'wallSeconds': 60, 'maxRepairs': 0}})
            with patch.dict(os.environ, environment):
                docs['prepare'](job, run)
            plan = docs['load'](run / 'inputs' / 'author-plan.json')
            packets = {}
            for slot in plan['slots']:
                if slot['kind'] == 'OPERATION':
                    prompt = (run / 'inputs' / slot['prompt']).read_text()
                    packet = docs['P']['expand_fact_context'](json.loads(prompt.rsplit('\n\n', 1)[-1]))
                    packets[packet['requirement']['path']] = packet
            for route, literal in (('/choose', 'selected'), ('/inclusive', 'included'), ('/display', 'display')):
                with self.subTest(route=route):
                    packet = packets[route]
                    group = packet['facts'][packet['requirement']['id']]
                    rows = [row for row in group.get('facts', ()) if row['kind'] == 'java-return-lineage']
                    fact = next((row for row in rows if f'returns `"{literal}"`' in row['text']), None)
                    self.assertIsNotNone(fact, [(row['kind'], row['text']) for row in group.get('facts', ())][:20])
                    if route == '/display':
                        self.assertIn('`page.empty()` is false', fact['condition'])
                        self.assertIn('An opaque call can still fail', fact['uncertainty'])
                        contracts = [row for row in group.get('facts', ()) if row['kind'] == 'java-contract']
                        self.assertEqual({truth for row in contracts for truth in ('true', 'false')
                                          if f'`key == null` is {truth}' in row['condition']}, {'true', 'false'}, contracts)
                        self.assertTrue(all('Selects items with a label starting' in row['text'] for row in contracts))
                        queries = [row for row in group['facts'] if row['kind'] == 'java-ordered-effects'
                                   and 'reader.find(key, window)' in row['text']]
                        self.assertEqual({truth for row in queries for truth in ('true', 'false')
                                          if f'`key == null` is {truth}' in row['condition']}, {'true', 'false'}, queries)
                    else:
                        result = next(row for row in group['facts'] if row['kind'] == 'java-iteration-alternatives')
                        self.assertIn('receives the value of `parcel.item().getLabel()` for `label`', result['condition'])
                        self.assertIn('`name.equalsIgnoreCase(label)` is true', result['condition'] + result['text'])
                        self.assertIn('`selected != null` is true', fact['condition'])
                        if route == '/choose':
                            self.assertIn('receives `true` for `ignoreNew`', result['condition'])
                            self.assertIn('`item.isNew()` is false, the current iteration returns', result['text'])
                            self.assertIn('`item.isNew()` is true, the current iteration reaches the loop body end',
                                          result['text'])
                        else:
                            self.assertIn('receives `false` for `ignoreNew`', result['condition'])
                            self.assertNotIn('`item.isNew()`', result['condition'] + result['text'])
                    cited = [packet['citations'][alias] for alias in fact['sources']]
                    excerpts = [packet['sourceRanges'][ref]['text'] for row in cited
                                for ref in row.get('regions', [row.get('region')])]
                    self.assertTrue(any(f'"{literal}"' in text for text in excerpts))
                    if route == '/choose':
                        self.assertTrue(any('return item;' in text for text in excerpts))
                        self.assertTrue(any('return this.id == null;' in text for text in excerpts))
            shadow = packets['/shadow']
            facts = shadow['facts'][shadow['requirement']['id']]['facts']
            self.assertTrue(any(row['kind'] == 'java-return-lineage' and 'returns `"queued"`' in row['text']
                                for row in facts), facts)
            self.assertTrue(any(row['kind'] == 'java-construction' and 'supplies `"existing"`' in row['text']
                                and 'record component `status`' in row['text'] for row in facts), facts)
            pair, = [row for row in facts if row['kind'] == 'java-iteration-alternatives']
            self.assertIn('`item.isNew()` is true in `Entry.shadow`', pair['condition'])
            self.assertIn('`item.isNew()` is false in `Helper.select`, the current iteration returns', pair['text'])
            self.assertIn('`item.isNew()` is true in `Helper.select`, the current iteration reaches the loop body end',
                          pair['text'])
            citations = [shadow['citations'][alias] for alias in pair['sources']]
            excerpts = [region['text'] for row in citations
                        for ref in row.get('regions', [row.get('region')])
                        if (region := shadow['sourceRanges'][ref])['file'].endswith(('/Entry.java', '/Helper.java'))]
            self.assertTrue(any('!item.isNew()' in text for text in excerpts), excerpts)
            self.assertTrue(any('!ignoreNew || !item.isNew()' in text and 'return item;' in text for text in excerpts),
                            excerpts)

@unittest.skipUnless(HAS_JDK, 'JDK 21 or newer required')
class PackagedJava(unittest.TestCase):
    def test_interprocedural_values_reach_only_the_owning_operation_author(self):
        body = '''String choose(boolean enabled, Helper helper, Item[] items) {
        if (!enabled) return "off";
        final int threshold = 3;
        return forward(helper, items, threshold);
    }'''
        local = '''private String forward(Helper helper, Item[] items, int requested) {
        return helper.select(items, requested);
    }'''
        helper = '''class Helper {
    String select(Item[] items, boolean flag) { return "unselected"; }
    String select(Item[] items, int requested) {
        while (requested < 2) return "below";
        for (Item item : items) {
            if (item.disabled) continue;
            if (!(item.size != requested)) return "selected";
        }
        return "empty";
    }
}
class Item {
    int size;
    boolean disabled;
}'''
        requirement, sources, facts, rejected = operation_fixture(body, local, helper)
        self.assertEqual(rejected, 0)
        returns = [row for row in facts if row['kind'] == 'java-return-lineage']
        match = next(row for row in returns if '`"selected"`' in row['text'])
        self.assertEqual(match['status'], 'CONDITIONAL')
        self.assertIn('if runtime dispatch enters the cited body', match['uncertainty'])
        self.assertIn('`enabled` is true', match['condition'])
        self.assertIn('receives `3` for `requested`', match['condition'])
        self.assertIn('binds its first element', match['condition'])
        self.assertIn('`item.disabled` is false', match['condition'])
        self.assertIn('`item.size != requested` is false', match['condition'])
        self.assertTrue(any('!(item.size != requested)' in text for text in witness_texts(match, sources)))
        continued = next(row for row in facts if row['kind'] == 'java-iteration-skip' and 'cited continue' in row['text'])
        self.assertIn('`item.disabled` is true', continued['condition'])
        self.assertTrue(any('continue;' in text for text in witness_texts(continued, sources)))
        self.assertFalse(any(value in row['text'] for row in returns for value in ('"unselected"', '"below"')))
        decision = next(row for row in facts if row['kind'] == 'java-value-decision' and 'loop entry test' in row['text'])
        self.assertIn('`requested < 2` is false', decision['text'])

        docs = runpy.run_path(str(SCRIPTS / 'docs_run.py'))
        pages = docs['P']
        sibling = {**requirement, 'id': 'catalog:other', 'handlerId': 'other', 'page': 'other.md', 'path': '/other'}
        identity = requirement['id']
        exposed = TRANSPORT['author_entries']({identity: {'propositions': facts}}, sources)
        index = {'sources': sources, 'facts': exposed, 'coverage': {'coverage': 'SYNTAX'}, 'returns': [], 'deferred': [],
                 'ready': {row['id']: {'evidenceCandidates': ['root']} for row in (requirement, sibling)}}
        inventory = {'snapshot': 'fixture', 'requirements': [requirement, sibling], 'services': {'catalog': {}}}
        job = {'services': ['catalog'], 'language': 'en', 'request': 'Describe the selected operation.'}
        with tempfile.TemporaryDirectory() as directory, patch.dict(pages['page_jobs'].__globals__, {
            'service_evidence': lambda *_: index,
        }):
            root = Path(directory)
            plan = pages['page_jobs'](job, inventory, {'services': {'catalog': {}}}, root / 'prompts', 12, 'Write a page.')
            slots = {slot['requirement']: slot for slot in plan['slots'] if slot['kind'] == 'OPERATION'}

            def packet(entry):
                prompt = (root / slots[entry]['prompt']).read_text()
                return pages['expand_fact_context'](json.loads(prompt.rsplit('\n\n', 1)[-1]))

            owned = packet(identity)
            scoped, = owned['facts'].values()
            rendered = next(row for row in scoped['facts'] if row['text'] == match['text'])
            citations = {slots[identity]['citations'][alias] for alias in rendered['sources']}
            self.assertEqual(citations, {'root', 'entry-unit', 'helper'})
            excerpts = [region['text'] for region in owned['sourceRanges'].values()]
            self.assertTrue(any('forward(helper, items, threshold)' in text for text in excerpts))
            self.assertTrue(any('helper.select(items, requested)' in text for text in excerpts))
            self.assertTrue(any('return "selected";' in text for text in excerpts))
            self.assertEqual(packet(sibling['id'])['facts'], {})

    def test_interprocedural_virtual_dispatch_keeps_a_conditional_result(self):
        body = 'int choose(Helper helper) { return helper.select(3); }'
        helper = 'class Helper { int select(int requested) { return 29; } }'
        _, _, facts, rejected = operation_fixture(body, '', helper)
        self.assertEqual(rejected, 0)
        result, = [row for row in facts if row['kind'] == 'java-return-lineage']
        self.assertIn('returns `29`', result['text'])
        self.assertIn('`helper.select(3)` enters `Helper.select`', result['condition'])
        self.assertIn('if runtime dispatch enters the cited body', result['uncertainty'])
        stopped, = [row for row in facts if row['kind'] == 'java-value-boundary']
        self.assertEqual(stopped['status'], 'OPEN')
        self.assertIn('runtime dispatch', stopped['text'])
        self.assertEqual({site['file'] for site in stopped['witnesses']}, {PATH, HELPER_PATH})

    def test_interprocedural_value_stops_at_an_unselected_overload(self):
        body = '''int choose(Helper helper, java.util.function.IntSupplier values) {
        return helper.select(values.getAsInt());
    }'''
        helper = '''class Helper {
    int select(int requested) { return 29; }
    int select(boolean requested) { return 13; }
}'''
        _, _, facts, rejected = operation_fixture(body, '', helper)
        self.assertEqual(rejected, 0)
        self.assertFalse(any(row['kind'] == 'java-return-lineage' for row in facts))
        stopped, = [row for row in facts if row['kind'] == 'java-value-boundary']
        self.assertIn('leaves the selected declaration open', stopped['text'])

    def test_interprocedural_boolean_result_selects_the_caller_return(self):
        body = 'int choose() { if (!Helper.blocked(3)) return 8; return 9; }'
        helper = '''final class Helper {
    static boolean blocked(int count) { return count >= 5 && probe(); }
    static boolean blocked(boolean enabled) { return enabled; }
    private static boolean probe() { return true; }
}'''
        _, sources, facts, rejected = operation_fixture(body, '', helper)
        self.assertEqual(rejected, 0)
        returned, = [row for row in facts if row['kind'] == 'java-return-lineage']
        self.assertIn('operation returns `8`', returned['text'])
        self.assertIn('`Helper.blocked(3)` is false', returned['condition'])
        self.assertIn('`count >= 5` is false', returned['condition'])
        self.assertNotIn('`probe()`', returned['condition'])
        self.assertTrue(any('return count >= 5 && probe();' in text for text in witness_texts(returned, sources)))

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
        opaque = (other[0], other[1].replace('new ConfiguredPublisher(raw)', 'load(raw)'))
        call = 'publisher.publishById(id, new Added(id))'
        body = f'void add(String id) {{ {call}; }}'
        target = f'method:class:{prefix}.DomainEventPublisherForAggregate#publishById(Ljava/lang/Object;Lio/eventuate/tram/events/common/DomainEvent;)V'
        for receiver, support, expected in (
            ("ConfiguredPublisher", [wrapper, port], "catalog.Entry"),
            ("Port", [wrapper, port, bean], "catalog.Entry"),
            ("Port", [wrapper, port], None),
            ("Port", [wrapper, port, bean, other], None),
            ("Port", [wrapper, port, bean, opaque], None),
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

    def test_generic_factory_result_keeps_each_constructed_event_and_its_guard(self):
        helper = ('src/catalog/Factory.java', '''package catalog;
import io.eventuate.tram.events.common.DomainEvent;
import io.eventuate.tram.events.publisher.ResultWithTypedEvents;
import java.util.List;
final class Factory {
    static <E extends DomainEvent> ResultWithTypedEvents<String, E> pack(String id, E event) {
        return new ResultWithTypedEvents<>(id, List.of(event));
    }
}
''')
        removed = ('src/catalog/Removed.java', '''package catalog;
import io.eventuate.tram.events.common.DomainEvent;
record Removed(String id) implements DomainEvent {}
''')
        body = '''String change(String id, boolean add) {
    var packet = add ? Factory.pack(id, new Added(id)) : Factory.pack(id, new Removed(id));
    publisher.publish(Entry.class, id, packet.getEvents());
    return id;
}'''
        facts, rejected = publication_fixture(body, files=[helper, removed], all_facts=True)
        self.assertEqual(rejected, 0)
        publications = {row['_edge']['eventType']: row for row in facts if row['kind'] == 'java-typed-publication'}
        self.assertEqual(set(publications), {'catalog.Added', 'catalog.Removed'})
        self.assertEqual(len({row['_edge']['occurrence'] for row in publications.values()}), 2)
        for event, guard in (('catalog.Added', 'true'), ('catalog.Removed', 'false')):
            publication = publications[event]
            self.assertIn(f'`add` is {guard}', publication['condition'])
            self.assertIn('The event constructor receives `id`', publication['text'])
            self.assertTrue(any(witness['file'] == helper[0] for witness in publication['witnesses']))
            reply, = [row for row in facts if row.get('_edgeCause') == publication['_edge']['occurrence']]
            self.assertIn('return id;', reply['text'])
            self.assertIn('no explicit wait', reply['text'])

    def test_opaque_collection_and_reply_wait_remain_open(self):
        body = '''String update(String id, List<Added> opaque, java.util.concurrent.Future<String> wait) {
    publisher.publish(Entry.class, id, opaque);
    return wait.get();
}'''
        facts, rejected = publication_fixture(body, all_facts=True)
        self.assertEqual(rejected, 0)
        publication, = [row for row in facts if row['kind'] == 'java-typed-publication']
        self.assertIsNone(publication['_edge']['eventType'])
        reply, = [row for row in facts if row.get('_edgeCause') == publication['_edge']['occurrence']]
        self.assertIn('leave any wait for a dispatcher result open', reply['text'])

    def test_local_event_attachment_survives_open_channel_and_in_scope_ambiguity_is_refused(self):
        factory = ('src/catalog/Factory.java', '''package catalog;
import io.eventuate.tram.events.common.DomainEvent;
import io.eventuate.tram.events.publisher.ResultWithTypedEvents;
final class Factory {
    static <E extends DomainEvent> ResultWithTypedEvents<String, E> pack(String id, E event) {
        return new ResultWithTypedEvents<>(id, event);
    }
}
''')
        duplicate = ('duplicate/src/catalog/Added.java', '''package catalog;
import io.eventuate.tram.events.common.DomainEvent;
record Added(String different) implements DomainEvent {}
''')
        body = '''void add(String id, String destination) {
    var packet = Factory.pack(id, new Added(id));
    publisher.publish(destination, id, packet.getEvents());
}'''
        members = [PATH, factory[0]]
        facts, rejected = publication_fixture(body, files=[factory, duplicate], all_facts=True, source_members=members)
        self.assertEqual(rejected, 0)
        publication, = [row for row in facts if row['kind'] == 'java-typed-publication']
        self.assertIsNone(publication['_edge']['channel'])
        self.assertEqual(publication['_edge']['eventType'], 'catalog.Added')
        attachment, = [row for row in facts if row['kind'] == 'java-typed-event-attachment']
        self.assertIn('new Added(id)', attachment['text'])
        self.assertIn('new ResultWithTypedEvents<>(id, event)', attachment['text'])
        self.assertTrue(all(witness['file'] in members for witness in attachment['witnesses']))

        ambiguous, rejected = publication_fixture(body, files=[factory, duplicate], all_facts=True,
                                                   source_members=[*members, duplicate[0]])
        self.assertEqual(rejected, 0)
        publication, = [row for row in ambiguous if row['kind'] == 'java-typed-publication']
        self.assertIsNone(publication['_edge']['eventType'])
        self.assertFalse(any(row['kind'] == 'java-typed-event-attachment' for row in ambiguous))

    def test_typed_route_representatives_precede_repeated_paths_to_the_same_event(self):
        removed = ('src/catalog/Removed.java', '''package catalog;
import io.eventuate.tram.events.common.DomainEvent;
record Removed(String id) implements DomainEvent {}
''')
        body = '''void add(String id, boolean first, boolean second, boolean third) {
    if (first) { }
    if (second) { }
    if (third) { }
    publisher.publish(Entry.class, id, List.of(new Added(id)));
    publisher.publish(Entry.class, id, List.of(new Removed(id)));
}'''
        publications, rejected = publication_fixture(body, files=[removed])
        self.assertEqual(rejected, 0)
        self.assertEqual({row['_edge']['eventType'] for row in publications[:2]}, {'catalog.Added', 'catalog.Removed'})

    def test_typed_catch_does_not_reuse_the_previous_value_of_a_reassigned_local(self):
        body = '''void add(String id, String unknown) {
    String destination = "original";
    try {
        destination = unknown;
        throw new IllegalStateException();
    } catch (IllegalStateException failure) {
        publisher.publish(destination, id, List.of(new Added(id)));
    }
}'''
        publications, rejected = publication_fixture(body)
        self.assertEqual(rejected, 0)
        row, = publications
        self.assertIsNone(row['_edge']['channel'])
        self.assertEqual(row['_edge']['eventType'], 'catalog.Added')
        self.assertIn('the catch for `IllegalStateException failure`', row['condition'])

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
    unittest.main(defaultTest='PackagedJava')
