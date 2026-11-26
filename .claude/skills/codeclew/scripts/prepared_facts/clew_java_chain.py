"""Project bounded Java call witnesses from the operation's pinned source."""

from base64 import b64encode
from collections import Counter, defaultdict
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
from itertools import islice, zip_longest
import json
import operator
from pathlib import Path, PurePosixPath
import re
import runpy
import select
import shutil
import subprocess


AUTHORITY = "PINNED_JAVAC_SYNTAX"
MAX_FILES = 12
MAX_LOOKUPS = 36
MAX_FILE_BYTES = 96 * 1024
MAX_STATES = 12
MAX_STEPS = 1800
MAX_DEPTH = 4
MAX_PATH_EVENTS = 64
MAX_FACTS = 16
MAX_FACT_BYTES = 6500
MAX_LOOKUP_FACTS = 2
MAX_VALUE_FACTS = 8
MAX_VALUE_FACT_BYTES = 3600
VALUE_EVENT_KINDS = {"return-lineage", "iteration-result", "iteration-skip", "iteration-alternatives",
                     "value-decision", "value-boundary"}
FILTERED_LOOKUP = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_control.py")))["filtered_lookup"]
PUBLICATIONS = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_publication.py")))


@dataclass(frozen=True)
class Site:
    path: str
    start: int
    end: int


def proof_sites(*sites):
    return tuple(dict.fromkeys(site for site in sites if site))


@dataclass(frozen=True)
class Type:
    path: str
    package: str
    import_nodes: tuple
    node: list

    @property
    def name(self):
        return self.node[3]

    @property
    def qualified(self):
        return ".".join(part for part in (self.package, self.name) if part)

    @property
    def record(self):
        return self.node[4] == "RECORD"

    @property
    def members(self):
        return self.node[7]

    @property
    def imports(self):
        return tuple(node[3] for node in self.import_nodes if node[0] == "import")

    def fields(self, name=None):
        return [member for member in self.members if member[0] == "field"
                and (name is None or name == member[3])]

    def methods(self, name=None):
        return [member for member in self.members if member[0] == "method"
                and (name is None or name == member[3])]

    def site(self, node):
        return Site(self.path, node[1], node[2])

    def import_proof(self, written, qualified):
        if written == qualified or not qualified:
            return ()
        return tuple(self.site(node) for node in self.import_nodes if node[0] == "import" and node[3] == qualified)


@dataclass(frozen=True)
class Value:
    label: str | None = None
    site: Site | None = None
    type: str = ""
    exact: str = ""
    truth: bool | None = None
    nullable: bool | None = None
    optional: str | None = None
    identity: Site | None = None
    test: tuple[Site, str] | None = None
    entries: tuple = ()
    components: tuple = ()
    constructed: bool = False
    final_fields: tuple = ()
    provenance: tuple[Site, ...] = ()
    arguments: tuple = ()
    type_sources: tuple[Site, ...] = ()
    publication_operand: object = None
    number: int | None = None
    string: str | None = None
    flow: tuple[Site, ...] = ()
    loop_element: tuple[Site, ...] = ()
    transferred_result: bool = False


UNKNOWN = Value()
OPTIONAL = "java.util.Optional"
CRUD_REPOSITORY = "org.springframework.data.repository.CrudRepository"
CRUD_SIGNATURES = {f"method:class:{CRUD_REPOSITORY}#findById({descriptor})Ljava/util/Optional;"
                   for descriptor in ("Ljava/lang/Object;", "Ljava/io/Serializable;")}
CRUD_TYPES = {CRUD_REPOSITORY, "org.springframework.data.repository.ListCrudRepository",
              "org.springframework.data.jpa.repository.JpaRepository"}
COMPATIBLE_REPOSITORY_BASES = {
    "org.springframework.data.repository.Repository",
    "org.springframework.data.repository.PagingAndSortingRepository",
    "org.springframework.data.repository.ListPagingAndSortingRepository",
}
DESCRIPTORS = dict(boolean="Z", byte="B", char="C", short="S", int="I", long="J", float="F", double="D", void="V")
INTEGRAL_TYPES = {"byte", "char", "short", "int", "long"}
COMPARISONS = {"eq": operator.eq, "neq": operator.ne, "LESS_THAN": operator.lt,
               "LESS_THAN_EQUAL": operator.le, "GREATER_THAN": operator.gt, "GREATER_THAN_EQUAL": operator.ge}
JAVA_LANG = {"String", "Object", "Boolean", "Byte", "Character", "Short", "Integer", "Long", "Float", "Double", "Void"}


@dataclass(frozen=True)
class Decision:
    expression: str
    method: str
    site: Site
    parents: tuple[Site, ...]


@dataclass(frozen=True)
class Condition:
    text: str
    cites: tuple[Site, ...]
    lookup_scope: bool = False
    value_flow: Site | None = None
    dispatch_candidate: bool = False
    decision: Decision | None = None
    transferred_result: bool = False
    brief: str | None = None
    truth: bool | None = None


@dataclass(frozen=True)
class IterationScope:
    binding: tuple[Site, ...]
    premises: int


@dataclass(frozen=True)
class Event:
    kind: str
    text: str
    cites: tuple[Site, ...]
    depth: int
    parents: tuple[Site, ...]
    conditions: tuple[Condition, ...]
    bypass: Site | None = None
    transferred_result: bool = False
    opaque_calls: bool = False
    iteration: IterationScope | None = None


@dataclass(frozen=True)
class IterationPair:
    returned: Event
    skipped: Event
    fork: int


@dataclass(frozen=True)
class State:
    bindings: dict = field(default_factory=dict)
    events: tuple[Event, ...] = ()
    conditions: tuple[Condition, ...] = ()
    optionals: dict = field(default_factory=dict)
    initialized: dict = field(default_factory=dict)
    typed_submissions: tuple = ()
    exit: str | None = None
    result: Value = UNKNOWN
    return_site: Site | None = None
    value_events: tuple[Event, ...] = ()
    normal_receivers: dict[tuple[Site, ...], Site] = field(default_factory=dict)


@dataclass(frozen=True)
class Frame:
    owner: Type
    method: list
    this: Value
    parents: tuple[Site, ...] = ()
    value_flow: bool = True

    def site(self, node):
        return self.owner.site(node)


class Units:
    def __init__(self, snapshot, parser):
        self.snapshot = snapshot
        self.parser = parser
        self.units = {}
        self.reads = {False: 0, True: 0}
        self.lookups = {False: 0, True: 0}
        self.searches = {}

    def read(self, path, *, protocol=False):
        if path not in self.units:
            if self.lookups[protocol] >= MAX_LOOKUPS or self.reads[protocol] >= MAX_FILES:
                return ()
            self.lookups[protocol] += 1
            data = self.snapshot.read(path, protocol=True) if protocol else self.snapshot.read(path)
            self.units[path] = ()
            if data and len(data) <= MAX_FILE_BYTES:
                self.reads[protocol] += 1
                pipe = self.parser
                pipe.stdin.write(b64encode(data) + b"\n")
                pipe.stdin.flush()
                timeout = self.snapshot.remaining(8) if hasattr(self.snapshot, "remaining") else 8
                if not select.select([pipe.stdout], [], [], timeout)[0]:
                    raise TimeoutError("Java compilation unit parser did not respond")
                if unit := json.loads(pipe.stdout.readline()):
                    package, imports, declarations = unit
                    self.units[path] = tuple(Type(path, package, tuple(imports), node) for node in declarations)
        return self.units[path]

    @staticmethod
    def source_root(owner):
        path = PurePosixPath(owner.path)
        package = tuple(owner.package.split(".")) if owner.package else ()
        dirs = path.parts[:-1]
        if package and dirs[-len(package):] != package:
            return None
        return PurePosixPath(*dirs[:len(dirs) - len(package)])

    def qualify(self, owner, name, type_parameters=()):
        if name.endswith("[]"):
            element = self.qualify(owner, name[:-2], type_parameters)
            return element + "[]" if element else ""
        if name in DESCRIPTORS:
            return name
        if name in type_parameters or name in owner.node[9] or name in owner.node[10]:
            return ""
        if not name or name == "var" or "." in name:
            return "" if name == "var" else name
        imports = [path for path in owner.imports if path.endswith("." + name)]
        if imports:
            return imports[0] if len(imports) == 1 else ""
        local = ".".join(part for part in (owner.package, name) if part)
        return local

    def lookup(self, owner, name, *, protocol=False):
        if not name or name == OPTIONAL or name in DESCRIPTORS or not all(part.isidentifier() for part in name.split(".")):
            return None
        key = owner.path, name, protocol
        if key not in self.searches:
            suffix = name.replace(".", "/") + ".java"
            leaf = PurePosixPath(suffix).name
            root = self.source_root(owner)
            nearby = [owner.path, str(PurePosixPath(owner.path).with_name(leaf))]
            pinned = [path for path in self.snapshot.paths() if PurePosixPath(path).name == leaf]
            modules = self.snapshot.type_paths(name) if hasattr(self.snapshot, "type_paths") else ()
            if len(modules) > 1:
                self.searches[key] = None
                return None
            candidates = [*nearby, *((str(root / suffix),) if root is not None else ()), *pinned[:MAX_FILES], *modules]
            candidates = tuple(dict.fromkeys(candidates))
            if hasattr(self.snapshot, "existing_paths"):
                candidates = self.snapshot.existing_paths(candidates)
            matches = {(entry.path, entry.qualified): entry for path in candidates
                       for entry in self.read(path, protocol=protocol) if entry.qualified == name}
            self.searches[key] = next(iter(matches.values())) if len(matches) == 1 else None
        return self.searches[key]

    def method_at(self, site):
        matching = [(owner, method) for owner in self.read(site.path) for method in owner.methods()
                    if owner.site(method) == site]
        return matching[0] if len(matching) == 1 else (None, None)

    def code(self, site):
        source = self.snapshot.read(site.path)
        text = source[site.start:site.end].decode()
        if 0 < len(text) <= 155 and not any(char in text for char in "`\r\n"):
            return f"`{text}`"
        return None

    def descriptor(self, owner, name):
        if name.endswith("[]"):
            return "[" + self.descriptor(owner, name[:-2])
        if name in DESCRIPTORS:
            return DESCRIPTORS[name]
        qualified = self.qualify(owner, name)
        if name in JAVA_LANG and not any(item.endswith("." + name) for item in owner.imports):
            if (not self.lookup(owner, qualified) and not (hasattr(self.snapshot, "type_paths") and self.snapshot.type_paths(qualified))
                    and getattr(self.snapshot, "complete_tree", True)):
                qualified = "java.lang." + name
        return "L" + qualified.replace(".", "/") + ";" if qualified else ""

    def compiler_method(self, caller, target):
        if not target:
            return None, None
        match = re.fullmatch(r"method:class:([^#]+)#([^()]+)(\([^)]*\).+)", target)
        if not match or "$" in match[1] or not (owner := self.lookup(caller, match[1])):
            return None, None
        methods = []
        for method in owner.methods(match[2]):
            if method[12]:
                continue
            parameters = "".join(self.descriptor(owner, param[4]) for param in method[6])
            result = "V" if match[2] == "<init>" else self.descriptor(owner, method[4])
            if f"({parameters}){result}" == match[3]:
                methods.append(method)
        return (owner, methods[0]) if len(methods) == 1 else (None, None)

    def comparable_type(self, owner, name):
        if name.endswith("[]"):
            return self.comparable_type(owner, name[:-2]) + "[]"
        short = name.rpartition(".")[2]
        local = ".".join(part for part in (owner.package, short) if part)
        if (name == local and short in JAVA_LANG and not any(entry.endswith(".*") for entry in owner.imports)
                and not self.lookup(owner, name)
                and not (hasattr(self.snapshot, "type_paths") and self.snapshot.type_paths(name))
                and getattr(self.snapshot, "complete_tree", True)):
            return "java.lang." + short
        return name

    def nominal_type(self, owner, name, type_parameters=()):
        return self.comparable_type(owner, self.qualify(owner, name, type_parameters))

    def inherited_lookup(self, receiver, owner, seen=()):
        """Resolve every base before using a published Spring Data Optional signature."""
        if receiver in CRUD_TYPES | COMPATIBLE_REPOSITORY_BASES and owner is None:
            if ((hasattr(self.snapshot, "type_paths") and self.snapshot.type_paths(receiver))
                    or not getattr(self.snapshot, "complete_tree", True)):
                return None
            return receiver in CRUD_TYPES, ()
        if not owner or owner.qualified in seen or len(seen) == 3 or owner.methods("findById") or len(owner.node[11]) > 3:
            return None
        inherited = False
        anchors = []
        for parent in owner.node[11]:
            name = self.qualify(owner, parent[3])
            branch = self.inherited_lookup(name, self.lookup(owner, name), (*seen, owner.qualified))
            if branch is None:
                return None
            inherited |= branch[0]
            anchors.extend((owner.site(parent), *owner.import_proof(parent[3], name), *branch[1]))
        return inherited, tuple(anchors)

    def inherited_declarations(self, owner, name, caller, seen=()):
        """Find visible inherited declarations in a complete, bounded source walk."""
        if owner.qualified in seen or len(seen) >= MAX_DEPTH or len(owner.node[11]) > 3:
            return None
        local = [method for method in owner.methods(name) if "PRIVATE" not in method[5]
                 and (owner.package == caller.package or {"PUBLIC", "PROTECTED"}.intersection(method[5])
                      or owner.node[4] == "INTERFACE")]
        if local:
            return ((owner, ()),)
        declarations = []
        for parent in owner.node[11]:
            expected = self.qualify(owner, parent[3])
            actual = self.lookup(owner, expected)
            if actual is None:
                return None
            inherited = self.inherited_declarations(actual, name, caller, (*seen, owner.qualified))
            if inherited is None:
                return None
            anchors = (owner.site(parent), *owner.import_proof(parent[3], expected))
            declarations.extend((declaring, (*anchors, *path)) for declaring, path in inherited)
        return tuple(declarations)


class Projection:
    def __init__(self, units, selected, compiler_targets, *, lookup_only=False, publications=None,
                 value_flow=True):
        self.units = units
        self.selected = selected
        self.compiler_targets = compiler_targets
        self.lookup_only = lookup_only
        self.publications = publications
        self.value_flow = value_flow and not lookup_only
        self.steps = MAX_STEPS

    def code(self, frame, node):
        return self.units.code(frame.site(node)) or "the cited expression"

    @staticmethod
    def scalar(value):
        if value.truth is not None:
            return "`true`" if value.truth else "`false`"
        if value.number is not None and value.type == "char":
            return value.label if value.label and value.label.startswith("`'") else None
        if value.number is not None and value.type in INTEGRAL_TYPES:
            return f"`{value.number}`"
        if value.nullable is True:
            return "`null`"
        if value.string is not None and value.label and value.label.startswith("`"):
            return value.label
        return None

    def qualify(self, frame, name):
        return self.units.qualify(frame.owner, name, frame.method[12])

    def bind(self, state, name, value, declared_type="", type_sources=()):
        if declared_type:
            value = replace(value, type=declared_type, type_sources=proof_sites(*type_sources, *value.type_sources))
        return replace(state, bindings={**state.bindings, name: value})

    def event(self, frame, state, kind, text, *sites, transferred_result=False, opaque_calls=False, iteration=None):
        event = Event(kind, text, tuple(site for site in sites if site), len(frame.parents), frame.parents, state.conditions)
        if kind in VALUE_EVENT_KINDS:
            event = replace(event, transferred_result=transferred_result, opaque_calls=opaque_calls, iteration=iteration)
            if len(state.value_events) >= MAX_VALUE_FACTS:
                if kind not in {"return-lineage", "iteration-result", "iteration-skip"}:
                    return state
                for index in range(len(state.value_events) - 1, -1, -1):
                    if state.value_events[index].kind in {"value-decision", "value-boundary"}:
                        return replace(state, value_events=(*state.value_events[:index],
                                                            *state.value_events[index + 1:], event))
                return state
            return replace(state, value_events=(*state.value_events, event))
        if len(state.events) >= MAX_PATH_EVENTS:
            return replace(state, exit="open")
        return replace(state, events=(*state.events, event))

    def tick(self):
        self.steps -= 1
        return self.steps >= 0

    def block(self, frame, state, node):
        rows = [state]
        original = set(state.bindings)
        declared = set()
        for statement in node[3]:
            if statement[0] == "let":
                declared.add(statement[3])
            rows = [result for current in rows for result in self.statement(frame, current, statement)][:MAX_STATES]
        if declared:
            # Restore shadowed declarations and discard block-local names.
            def outer(row):
                bindings = {name: value for name, value in row.bindings.items() if name not in declared}
                bindings.update((name, state.bindings[name]) for name in declared & original)
                return replace(row, bindings=bindings)
            rows = [outer(row) for row in rows]
        return rows

    def statement(self, frame, state, node):
        if node is None:
            return [state]
        if state.exit:
            expression = ({"expr": 3, "let": 4, "return": 3}.get(node[0]))
            call = node[expression] if expression is not None else None
            return [self.mark_bypass(frame, state, call)]
        if not self.tick():
            return [replace(state, exit="open")]
        kind, data = node[0], node[3:]
        if kind == "block":
            return self.block(frame, state, node)
        if kind == "if":
            return [result for branch, truth in self.branch(frame, state, data[0])
                    for result in self.statement(frame, branch, data[1] if truth else data[2])]
        if kind == "pre-test-loop" and self.value_flow and frame.value_flow:
            return self.pre_test_loop(frame, state, node)
        if kind == "foreach" and self.value_flow and frame.value_flow:
            return self.each_loop(frame, state, node)
        if kind in {"break", "continue"} and self.value_flow and frame.value_flow and data[0] is None:
            return [replace(state, exit=kind, return_site=frame.site(node))]
        if kind == "try" and self.lookup_only:
            body, _, _, resources = data
            if not resources:
                entry = Site(frame.owner.path, node[1], node[1] + len(b"try"))
                premise = Condition("control enters the cited try body", (entry,), lookup_scope=True)
                entered = replace(state, conditions=(*state.conditions, premise))
                return [replace(row, exit="open") for row in self.statement(frame, entered, body)]
        if kind == "try" and self.publications:
            body, catches, final, resources = data
            if not final and not resources:
                return self.try_catches(frame, state, node, body, catches)
        if kind == "assert" and self.lookup_only:
            premise = Condition("the cited assertion allows evaluation to continue", (frame.site(node),), lookup_scope=True)
            return [replace(state, conditions=(*state.conditions, premise))]
        if kind == "let":
            name, initializer, declared_type = data
            type_name = self.qualify(frame, declared_type)
            imports = frame.owner.import_proof(declared_type, type_name)
            rows = []
            for row, value in self.evaluate(frame, state, initializer):
                if initializer and (type_name or value.type) == OPTIONAL and value.identity is None:
                    value = replace(value, identity=frame.site(initializer))
                if type_name == OPTIONAL:
                    value = replace(value, provenance=(*value.provenance, *imports))
                rows.append(self.bind(row, name, value, type_name, (frame.site(node), *imports)))
            return rows
        if kind in {"return", "throw"}:
            rows = []
            for row, value in self.evaluate(frame, state, data[0]):
                if self.publications and kind == "return" and not row.exit:
                    self.publications.reply(frame, row, node)
                    proof = self.publications.returned_operand(frame, row, node, value.publication_operand)
                    value = replace(value, publication_operand=proof)
                if kind == "throw" and frame.parents and not row.exit:
                    row = self.event(frame, row, "throw", f"Executes {self.code(frame, node)} in `{frame.owner.name}.{frame.method[3]}`.",
                                     frame.site(node), value.site)
                if kind == "return" and frame.parents and value.constructed and not row.exit:
                    name = f"`{frame.owner.name}.{frame.method[3]}`"
                    row = self.event(frame, row, "constructed-result", f"{name} returns the value constructed by {value.label}.",
                                     frame.site(node), value.site)
                if kind == "return" and self.value_flow and not frame.parents and not row.exit:
                    row = self.return_lineage(frame, row, node, data[0], value)
                rows.append(row if row.exit else replace(row, exit=kind, result=value,
                                                         return_site=frame.site(node)))
            return rows
        if kind == "expr":
            return [row for row, _ in self.evaluate(frame, state, data[0])]
        if kind == "empty":
            return [state]
        return [replace(state, exit="open")]

    def try_catches(self, frame, state, node, body, catches):
        entry = Site(frame.owner.path, node[1], node[1] + len(b"try"))
        entered = replace(state, conditions=(*state.conditions, Condition("execution enters the cited try body", (entry,))))
        paths = [self.statement(frame, entered, body)]
        try_nodes = tuple(syntax_nodes(body))
        if not any(part[0] in {"throw", "call", "new", "open", "boundary"} for part in try_nodes):
            return paths[0]

        # The selected handler may follow any throwing evaluation. Carry only
        # locals whose values did not change in the protected body.
        opaque = any(part[0] in {"open", "boundary"} for part in try_nodes)
        written = {part[3][3] for part in try_nodes if part[0] in {"set", "mutate"} and part[3][0] == "id"}
        stable = {name: Value(label=f"`{name}`", type=value.type, type_sources=value.type_sources)
                  if opaque or name in written else value for name, value in state.bindings.items()}
        for catch in catches[:MAX_STATES]:
            parameter, handler = catch[3:]
            name, declared = parameter[3:]
            caught = frame.site(parameter)
            label = self.units.code(caught) or "the cited exception parameter"
            condition = Condition(f"the catch for {label} handles an exception from the cited try body", (entry, caught))
            value = Value(f"`{name}`", caught, self.qualify(frame, declared), nullable=False)
            scoped = replace(state, conditions=(*state.conditions, condition), bindings={**stable, name: value})
            outputs = []
            for row in self.statement(frame, scoped, handler):
                bindings = row.bindings.copy()
                bindings.pop(name, None)
                if name in stable:
                    bindings[name] = stable[name]
                outputs.append(replace(row, bindings=bindings))
            paths.append(outputs)
        return list(islice((row for batch in zip_longest(*paths) for row in batch if row is not None), MAX_STATES))

    def pre_test_loop(self, frame, state, node):
        initializers, test, body = node[3:]
        entered = [state]
        for initializer in initializers:
            entered = [out for row in entered for out in self.statement(frame, row, initializer)][:MAX_STATES]
        results = []
        for initial in entered:
            # An unbounded loop contributes only its entry test and one body execution.
            checks = self.branch(frame, initial, test, loop=True) if test else [(initial, True)]
            for row, truth in checks:
                if row.exit or not truth:
                    results.append(row)
                    continue
                results.extend(self.first_iteration(frame, row, body))
        declared = {item[3] for item in initializers if item[0] == "let"}
        return [replace(row, bindings={**{key: value for key, value in row.bindings.items() if key not in declared},
                                       **{key: state.bindings[key] for key in declared & state.bindings.keys()}})
                for row in results[:MAX_STATES]]

    def each_loop(self, frame, state, node):
        parameter, iterable, body = node[3:]
        name, declared = parameter[3:]
        identity = frame.site(parameter)
        loop_element = (*frame.parents, identity)
        element = Value(f"`{name}`", identity, self.qualify(frame, declared),
                        identity=identity, loop_element=loop_element)
        label = self.code(frame, iterable)
        empty = Condition(f"the loop over {label} has no elements on entry", (frame.site(iterable), frame.site(parameter)))
        iteration = Condition(f"the loop over {label} binds its first element to `{name}`",
                              (frame.site(iterable), frame.site(parameter)))
        results = []
        for row, sequence in self.evaluate(frame, state, iterable):
            if row.exit or sequence.nullable is True:
                results.append(replace(row, exit=row.exit or "open"))
                continue
            receivers = {current: proof for current, proof in row.normal_receivers.items() if current != loop_element}
            entered = self.bind(replace(row, conditions=(*row.conditions, iteration), normal_receivers=receivers),
                                name, element)
            for outcome in self.first_iteration(frame, entered, body, loop_element):
                bindings = {key: value for key, value in outcome.bindings.items() if key != name}
                receivers = {current: proof for current, proof in outcome.normal_receivers.items() if current != loop_element}
                results.append(replace(outcome, bindings=bindings, normal_receivers=receivers))
            results.append(replace(row, conditions=(*row.conditions, empty)))
        return results[:MAX_STATES]

    def first_iteration(self, frame, state, body, loop_element=()):
        for outcome in self.statement(frame, state, body):
            outcome = self.iteration_outcome(frame, state, body, outcome, loop_element)
            exit_kind = None if outcome.exit == "break" else (
                outcome.exit if outcome.exit in {"return", "throw"} else "open")
            yield replace(outcome, exit=exit_kind)

    def iteration_outcome(self, frame, entered, body, outcome, loop_element):
        if not frame.parents:
            return outcome
        scope = IterationScope(loop_element, len(entered.conditions)) if loop_element else None
        if loop_element and outcome.exit == "return" and outcome.result.loop_element == loop_element:
            result = outcome.result
            return self.event(frame, outcome, "iteration-result",
                              f"The current iteration returns its element {result.label}.",
                              outcome.return_site, result.site, *result.flow, iteration=scope)
        if outcome.exit not in {None, "continue"}:
            return outcome
        decisions = [condition for condition in outcome.conditions[len(entered.conditions):] if condition.decision]
        if outcome.exit == "continue":
            return self.event(frame, outcome, "iteration-skip",
                              "The cited continue skips the rest of the current iteration.", outcome.return_site,
                              iteration=scope)
        if decisions and any(condition.value_flow for condition in decisions):
            last = body[3][-1] if body[0] == "block" and body[3] else body
            return self.event(frame, outcome, "iteration-skip",
                              "The current iteration reaches the loop body end without returning an element.",
                              decisions[-1].decision.site, frame.site(last), iteration=scope)
        return outcome

    def return_lineage(self, frame, state, node, expression, value):
        return_type = self.qualify(frame, frame.method[4])
        if not expression or not (scalar := self.scalar(replace(value, type=return_type or value.type))):
            return state
        local_decisions = tuple(item for item in state.conditions if item.value_flow == frame.site(frame.method))
        if not value.flow and not local_decisions:
            return state
        code = self.code(frame, expression)
        text = f"On this normal source path, the operation returns {scalar}"
        if code != scalar:
            text += f" through {code}"
        transferred = bool(value.flow) or any(item.transferred_result for item in local_decisions)
        opaque_calls = any(event.kind == "attempt" for event in state.events)
        return self.event(frame, state, "return-lineage", text + ".", frame.site(node), value.site, *value.flow,
                          transferred_result=transferred, opaque_calls=opaque_calls)

    def branch(self, frame, state, node, *, loop=False, enclosing_tests=()):
        kind, data = node[0], node[3:]
        if kind == "not":
            results = self.branch(frame, state, data[0], loop=loop, enclosing_tests=(*enclosing_tests, frame.site(node)))
            return [(row, not truth) for row, truth in results]
        if kind in {"and", "or"}:
            tests = (*enclosing_tests, frame.site(node))
            return [(branch, truth) for row, left in self.branch(frame, state, data[0], loop=loop, enclosing_tests=tests)
                    for branch, truth in (self.branch(frame, row, data[1], loop=loop, enclosing_tests=tests)
                                          if left == (kind == "and") else [(row, left)])][:MAX_STATES]
        result = []
        expression = self.code(frame, node)
        method = f"{frame.owner.name}.{frame.method[3]}"
        decision = Decision(expression, method, frame.site(node), frame.parents)
        for row, value in self.evaluate(frame, state, node):
            if row.exit:
                result.append((row, True))
                continue
            possible = (value.truth,) if value.truth is not None else (True, False)
            for truth in possible:
                evidence = value.flow if frame.value_flow else ()
                condition = Condition(f"{expression} is {'true' if truth else 'false'}",
                                      proof_sites(decision.site, *enclosing_tests, *evidence),
                                      value_flow=frame.site(frame.method) if evidence else None, decision=decision,
                                      transferred_result=value.transferred_result, truth=truth)
                optionals = row.optionals
                if value.test and value.truth is None:
                    address, present_when = value.test
                    optionals = {**optionals, address: "present" if truth == (present_when == "present") else "empty"}
                if evidence and self.value_flow:
                    role = "At the loop entry test, " if loop else "At the cited decision, "
                    text = (f"{role}{condition.text}." if value.truth is not None else
                            f"{role}{expression} uses the cited input and its result remains open.")
                    row = self.event(frame, row, "value-decision", text, *condition.cites)
                result.append((replace(row, conditions=(*row.conditions, condition), optionals=optionals), truth))
        return result[:MAX_STATES]

    def arguments(self, frame, state, nodes):
        rows = [(state, ())]
        for node in nodes:
            rows = [(branch, (*args, value)) for row, args in rows
                    for branch, value in self.evaluate(frame, row, node)][:MAX_STATES]
        return rows

    def evaluate(self, frame, state, node):
        if state.exit or node is None:
            return [(state, UNKNOWN)]
        if not self.tick():
            return [(replace(state, exit="open"), UNKNOWN)]
        kind, data = node[0], node[3:]
        site = frame.site(node)
        literal = Value(self.code(frame, node), site)
        if kind == "id":
            name = data[0]
            if name == "this":
                return [(state, frame.this)]
            if name in state.bindings:
                value = state.bindings[name]
                if value.loop_element and (proof := state.normal_receivers.get(value.loop_element)):
                    value = replace(value, nullable=False, flow=proof_sites(*value.flow, proof))
                return [(state, value)]
            return [(state, self.field_value(frame, frame.this, name, literal))]
        if kind == "literal":
            tag, *payload = data
            proof = self.publications.literal(frame.owner, node) if self.publications else None
            if tag == "INTEGRAL":
                type_name, number = payload
                return [(state, replace(literal, type=type_name, number=int(number),
                                        publication_operand=proof))]
            value = replace(literal, publication_operand=proof, truth=(tag == "TRUE") if tag in {"TRUE", "FALSE"} else None,
                            nullable=(tag == "NULL"),
                            string=payload[0] if payload and tag in {"EMPTY_STRING", "NONEMPTY_STRING"} else None,
                            type={"TRUE": "boolean", "FALSE": "boolean", "EMPTY_STRING": "java.lang.String",
                                  "NONEMPTY_STRING": "java.lang.String"}.get(tag, ""))
            return [(state, value)]
        if kind == "member":
            if self.publications and (proof := self.publications.class_operand(frame.owner, node, frame.method[12])):
                return [(state, replace(literal, publication_operand=proof))]
            return [(row, self.field_value(frame, receiver, data[1], literal))
                    for row, receiver in self.evaluate(frame, state, data[0])]
        if kind == "functional-reference":
            # Evaluating a bound qualifier can fail. The reference itself
            # carries no callback result or evidence that the method ran.
            formed = Condition(f"{literal.label} is formed normally", (site,))
            return [(replace(row, exit=row.exit or "open"), UNKNOWN) if row.exit or receiver.nullable is True else
                    (replace(row, conditions=(*row.conditions, formed)), replace(literal, nullable=False))
                    for row, receiver in self.evaluate(frame, state, data[0])]
        if kind == "choose":
            return [(row, value) for branch, truth in self.branch(frame, state, data[0])
                    for row, value in self.evaluate(frame, branch, data[1] if truth else data[2])][:MAX_STATES]
        if kind in {"eq", "neq", "ordered", "array"}:
            return [(row, self.comparison(kind, literal, args)) for row, args in self.arguments(frame, state, data)]
        if kind == "relation":
            return [(row, self.comparison(data[0], literal, args)) for row, args in self.arguments(frame, state, data[1:])]
        if kind == "signed":
            return [(row, self.signed(data[0], literal, value)) for row, value in self.evaluate(frame, state, data[1])]
        if kind in {"and", "or", "not"}:
            return [(row, replace(literal, type="boolean", truth=truth,
                                  flow=proof_sites(*(site for condition in row.conditions[len(state.conditions):]
                                                     if condition.value_flow for site in condition.cites))))
                    for row, truth in self.branch(frame, state, node)]
        if kind == "call":
            target, args = data
            receivers = self.receiver(frame, state, target)
            return [(row, value) for receiver_state, receiver, name, is_static in receivers
                    for branch, values in self.arguments(frame, receiver_state, args)
                    for row, value in self.call(frame, branch, node, receiver, name, is_static, args, values)][:MAX_STATES]
        if kind == "new":
            enclosing, type_node, args = data
            if enclosing:
                return [(replace(state, exit="open"), UNKNOWN)]
            return [(row, value) for branch, values in self.arguments(frame, state, args)
                    for row, value in self.construct(frame, branch, node, type_node[3], args, values)][:MAX_STATES]
        if kind in {"set", "mutate"}:
            return self.assign(frame, state, node, literal)
        return [(replace(state, exit="open"), UNKNOWN)]

    @staticmethod
    def comparison(kind, literal, values):
        if kind not in COMPARISONS:
            return literal
        left, right = values
        flow = proof_sites(*left.flow, *right.flow)
        logical = replace(literal, type="boolean", flow=flow,
                          transferred_result=left.transferred_result or right.transferred_result)
        if all(value.number is not None and value.type in INTEGRAL_TYPES for value in values):
            return replace(logical, truth=COMPARISONS[kind](left.number, right.number))
        if kind in {"eq", "neq"}:
            if True in (left.nullable, right.nullable) and None not in (left.nullable, right.nullable):
                equal = left.nullable == right.nullable
                return replace(logical, truth=equal == (kind == "eq"))
            if None not in (left.truth, right.truth) and "boolean" in {left.type, right.type}:
                return replace(logical, truth=(left.truth == right.truth) == (kind == "eq"))
        return logical

    @staticmethod
    def signed(operator_name, literal, value):
        if value.number is None or value.type not in INTEGRAL_TYPES:
            return literal
        type_name = "long" if value.type == "long" else "int"
        bits = 64 if type_name == "long" else 32
        number = value.number if operator_name == "UNARY_PLUS" else -value.number
        # Java evaluates integral negation in the promoted signed type.
        limit = 1 << (bits - 1)
        return replace(literal, type=type_name, number=(number + limit) % (limit * 2) - limit, flow=value.flow)

    def field_value(self, frame, receiver, name, literal):
        owner = self.units.lookup(frame.owner, receiver.type)
        fields = owner.fields(name) if owner else []
        if len(fields) != 1:
            return literal
        field = fields[0]
        address = owner.site(field)
        declared = self.units.qualify(owner, field[4])
        if receiver.exact == owner.qualified and (value := dict(receiver.final_fields).get(name)) is not None:
            return replace(value, type=declared or value.type,
                           provenance=(*value.provenance, address, literal.site))
        initial = field[6]
        exact = ""
        if "FINAL" in field[5] and initial and initial[0] == "new" and initial[3] is None:
            candidate = self.units.qualify(owner, initial[4][3])
            if candidate == declared:
                exact = candidate
        proof = owner.import_proof(field[4], declared)
        payload = (self.publications.payload_field(receiver.publication_operand, address, literal.label, literal.site)
                   if self.publications else None)
        return replace(literal, type=declared, exact=exact, site=address, provenance=proof, publication_operand=payload)

    def assign(self, frame, state, node, literal):
        kind, lhs, *remaining = (node[0], *node[3:])
        target = self.code(frame, lhs)
        local = lhs[0] == "id" and lhs[3] in state.bindings
        if local:
            rows = [(state, ())]
        elif lhs[0] in {"array", "member"}:
            rows = self.arguments(frame, state, lhs[3:4] if lhs[0] == "member" else lhs[3:])
        else:
            rows = [(state, ())]
        result = []
        for receiver_state, _ in rows:
            for row, value in self.evaluate(frame, receiver_state, remaining[0] if remaining else None):
                if row.exit:
                    result.append((row, UNKNOWN))
                    continue
                value = value if kind == "set" else literal
                if local:
                    if state.bindings[lhs[3]].type == OPTIONAL and not value.identity and remaining:
                        value = replace(value, identity=frame.site(remaining[0]))
                    previous = state.bindings[lhs[3]]
                    row = self.bind(row, lhs[3], value, previous.type, previous.type_sources)
                elif lhs[0] in {"array", "member"} or frame.owner.fields(lhs[3] if lhs[0] == "id" else ""):
                    if self.publications and kind == "set":
                        self.publications.handler_effect(frame, row, node, (value,), assignment=True)
                    definition = self.code(frame, node)
                    member = (f"the `{frame.owner.name}` constructor" if frame.method[3] == "<init>"
                              else f"`{frame.owner.name}.{frame.method[3]}`")
                    text = f"Writes {target} in {member} via {definition}."
                    if value.entries and value.label:
                        text += f" The assigned value comes from {value.label}."
                    anchor = frame.site(node)
                    role = "constructor-write" if frame.method[3] == "<init>" else "mutation"
                    row = self.event(frame, row, role, text, anchor, value.site, *value.provenance)
                    if kind == "set" and frame.method[3] == "<init>":
                        row = self.initialize_field(frame, row, lhs, value, anchor)
                result.append((row, value))
        return result

    @staticmethod
    def initialize_field(frame, state, lhs, value, site):
        if lhs[0] == "id":
            name = lhs[3]
        elif lhs[0] == "member" and lhs[3][0] == "id" and lhs[3][3] == "this":
            name = lhs[4]
        else:
            return state
        fields = frame.owner.fields(name)
        if len(fields) != 1 or "FINAL" not in fields[0][5] or "STATIC" in fields[0][5]:
            return state
        initialized = {**state.initialized, name: replace(value, provenance=(*value.provenance, site))}
        return replace(state, initialized=initialized)

    def receiver(self, frame, state, node):
        if node[0] == "id":
            return [(state, frame.this, node[3], False)]
        if node[0] != "member":
            return [(state, UNKNOWN, "", False)]
        base, name = node[3:]
        address = self.code(frame, base).removeprefix("`").removesuffix("`")
        head = address.partition(".")[0]
        if (head not in state.bindings and head not in {"this", "super"} and not frame.owner.fields(head)
                and (base[0] in {"id", "member"})):
            type_name = self.qualify(frame, address)
            if type_name == OPTIONAL or self.units.lookup(frame.owner, type_name):
                proof = frame.owner.import_proof(address, type_name) if type_name == OPTIONAL else ()
                return [(state, Value(type=type_name, site=frame.site(base), provenance=proof), name, True)]
        return [(row, value, name, False) for row, value in self.evaluate(frame, state, base)]

    def call(self, frame, state, node, receiver, name, is_static, args, values):
        if state.exit:
            return [(self.mark_bypass(frame, state, node), UNKNOWN)]
        site = frame.site(node)
        input_flow = proof_sites(*receiver.flow, *(proof for value in values for proof in value.flow))
        local = Value(self.code(frame, node), site, flow=input_flow)
        if receiver.nullable is True:
            return [(replace(state, exit="open"), UNKNOWN)]
        if not is_static and receiver.string is not None:
            if name == "isEmpty" and not values:
                return [(state, replace(local, type="boolean", truth=not receiver.string, flow=receiver.flow))]
            if name == "equals" and len(values) == 1 and (values[0].string is not None or values[0].nullable is True):
                return [(state, replace(local, type="boolean", truth=receiver.string == values[0].string,
                                        flow=input_flow))]
        if receiver.type == OPTIONAL:
            known = self.optional(frame, state, node, receiver, name, is_static, values, local)
            if known is not None:
                return known
        compiler_target = self.compiler_targets.get(site)
        if self.publications:
            operands = tuple(value.publication_operand for value in values)
            if name == "getResult" and not values and (
                    result := self.publications.bundle_result(frame, node, receiver.publication_operand, compiler_target)):
                return [(state, result)]
            if bound := self.publications.envelope_call(frame, node, receiver, name, values, compiler_target, local.label):
                operand, result_type = bound
                return [(state, replace(local, type=result_type, publication_operand=operand))]
            self.publications.handler_effect(frame, state, node, values)
            if name in {"publish", "publishById"} and (
                    submitted := self.publications.submit(frame, state, node, receiver, name, values, compiler_target)):
                state = replace(state, typed_submissions=(*state.typed_submissions, submitted))
            if name in {"of", "singletonList", "getName", "getEvents"}:
                projected = self.publications.call_operand(frame, state, node, receiver.publication_operand, name, operands, compiler_target)
                if projected:
                    local = replace(local, publication_operand=projected)
        compiler_owner, compiler_method = self.units.compiler_method(frame.owner, compiler_target)
        linked_owner, linked_method, link_fixed = self.linked_method(site, compiler_target, name)
        owner = compiler_owner or linked_owner or self.units.lookup(frame.owner, receiver.type)
        active = self.value_flow and frame.value_flow
        if owner and owner.record and not values and not owner.methods(name):
            if (component := dict(receiver.components).get(name)) is not None:
                return [(state, component)]
            components = [field for field in owner.fields(name) if "STATIC" not in field[5]]
            if active and len(components) == 1:
                field = components[0]
                declared = self.units.nominal_type(owner, field[4])
                evidence = proof_sites(site, owner.site(field), *owner.import_proof(field[4], declared))
                return [(state, replace(local, type=declared, type_sources=evidence))]
        inherited = ()
        receiver_owner = owner
        if active and owner and not is_static and not compiler_target and not linked_method and not owner.methods(name):
            declarations = self.units.inherited_declarations(owner, name, frame.owner)
            if declarations and len(declarations) == 1:
                owner, inherited = declarations[0]
        candidate = self.match(owner, name, len(values)) if owner else None
        if not candidate and owner and self.publications and not compiler_method and not linked_method:
            candidate = self.publications.generic_factory(owner, name, len(values))
        overload = (self.exact_overload(frame, owner, name, values)
                    if self.lookup_only and owner and not candidate and not compiler_target and not linked_method else None)
        direct = (self.source_signature(frame, owner, name, values)
                  if active and owner and not compiler_target else None)
        if compiler_target and owner and not compiler_target.startswith(f"method:class:{owner.qualified}#{name}("):
            candidate = None
        method = compiler_method or linked_method or direct or candidate or overload
        if method and is_static and "STATIC" not in method[5]:
            method = None
        if method and inherited and ("PRIVATE" in method[5]
                                     or (owner.node[4] == "INTERFACE" and "STATIC" in method[5])
                                     or (owner.package != receiver_owner.package and owner.node[4] != "INTERFACE"
                                         and not {"PUBLIC", "PROTECTED"}.intersection(method[5]))):
            method = None
        if method and len(values) != len(method[6]):
            method = None
        if active and not method and owner and (choices := owner.methods(name)):
            anchors = (declaration_anchor(owner, choice) for choice in choices[:3])
            state = self.open_value(frame, state, local, "The retained source leaves the selected declaration open.", *anchors)
        code = local.label
        mappings = [(self.code(frame, arg), value.label, value.site) for arg, value in zip(args, values)
                    if value.site and value.label and value.label not in {self.code(frame, arg), "the cited expression"}]
        if mappings:
            code += " (" + ", ".join(f"{argument} receives {origin}" for argument, origin, _ in mappings[:3]) + ")"
        origins = tuple(origin for _, _, origin in mappings)
        provenance = tuple(anchor for value in values if value.entries for anchor in value.provenance)
        if not method or method[8] is None or len(frame.parents) >= MAX_DEPTH:
            declared = method is not None and (method is compiler_method or (method is linked_method and link_fixed))
            source_contract = method is not None and not declared and self.compatible_contract(frame, owner, method, receiver, values)
            declaration_only = source_contract and bool(owner.node[11])
            return_type = (self.units.nominal_type(owner, method[4], method[12])
                           if declared or (source_contract and not declaration_only) else "")
            contract = None
            if not method and not is_static and name == "findById" and len(values) == 1:
                if compiler_target in CRUD_SIGNATURES:
                    contract = ()
                elif not compiler_target and (lookup := self.units.inherited_lookup(receiver.type, owner)) and lookup[0]:
                    contract = lookup[1]
            if contract is not None:
                return_type = OPTIONAL
                assumption = Condition(f"{local.label} returns an Optional under `CrudRepository.findById`",
                                       tuple(anchor for anchor in (site, receiver.site, *receiver.provenance, *contract) if anchor))
                state = replace(state, conditions=(*state.conditions, assumption))
            state = self.event(frame, state, "attempt", code, site, *origins, *provenance)
            references = tuple(frame.site(arg) for arg in args if arg[0] == "functional-reference")
            if references and not state.exit:
                normal = Condition(f"{local.label} returns normally", (site, *references),
                                   brief="the opaque call with the cited functional reference returns normally")
                state = replace(state, conditions=(*state.conditions, normal))
            for value in values:
                if value.constructed:
                    text = f"Passes the value constructed by {value.label} to {local.label}."
                    state = self.event(frame, state, "constructed-argument", text, site, value.site)
            conditions = state.conditions
            if source_contract:
                anchor = declaration_anchor(owner, method)
                premise = Condition(f"{local.label} resolves to the cited `{owner.qualified}.{method[3]}` signature",
                                    tuple(proof for proof in (site, anchor, receiver.site, *receiver.provenance,
                                                               *receiver.type_sources, *(value.site for value in values),
                                                               *(proof for value in values for proof in value.type_sources)) if proof))
                state = replace(state, conditions=(*state.conditions, premise))
            if method is not None and method[8] is None and (declared or source_contract):
                state = self.contract(frame, state, owner, method, local, declaration_only=declaration_only)
            if declaration_only:
                state = replace(state, conditions=conditions)
            if active and method and method[4] and method[4] != "void":
                reason = ("The retained declaration has no supported body." if not method[8]
                          else "The bounded call walk ends at this depth.")
                state = self.open_value(frame, state, local, reason, declaration_anchor(owner, method))
            types = (proof_sites(site, declaration_anchor(owner, method), *inherited)
                     if method and return_type and (declared or source_contract) else ())
            return [(state, replace(local, type=return_type, identity=site if return_type == OPTIONAL else None,
                                    provenance=contract or (), type_sources=types))]
        selected = method is compiler_method or (method is linked_method and link_fixed)
        dispatch_type = owner.qualified if (owner.record or "FINAL" in owner.node[5]
                      or {"STATIC", "PRIVATE", "FINAL"}.intersection(method[5])) else receiver.exact
        fixed = dispatch_type == owner.qualified
        follow = (active and (selected or method is direct) and method[9] == "DIRECT" and method[13] == "FIXED"
                  and all(self.units.qualify(owner, parameter[4], method[12]) for parameter in method[6]))
        if active and method[4] and method[4] != "void":
            if not follow:
                state = self.open_value(frame, state, local, "The retained source leaves the argument binding open.",
                                        declaration_anchor(owner, method))
            elif not fixed:
                state = self.open_value(frame, state, local, "The retained source leaves runtime dispatch open.",
                                        declaration_anchor(owner, method))
        premise = None
        if not (selected and fixed):
            premise = Condition(f"{local.label} resolves to `{owner.name}.{method[3]}` and enters its body",
                                proof_sites(site, declaration_anchor(owner, method), *inherited),
                                dispatch_candidate=not fixed, brief=f"{local.label} enters `{owner.name}.{method[3]}`")
        if premise:
            state = replace(state, conditions=(*state.conditions, premise))
        if follow and not is_static and "STATIC" not in method[5] and receiver.loop_element:
            state = replace(state, normal_receivers={**state.normal_receivers, receiver.loop_element: site})
        state = self.event(frame, state, "selected-call", code, site, declaration_anchor(owner, method), receiver.site, *origins, *provenance)
        if self.lookup_only and (method is overload or (selected and not candidate)):
            state = self.lookup_witness(frame, state, owner, method, local, args, values, receiver)
            # The witness describes the callee's return relation under its own
            # premises. It does not choose the actual return value for this call.
            return [(state, replace(local, type=self.units.qualify(owner, method[4])))]
        state = self.annotations(frame, state, owner, method, local, declaration_anchor(owner, method))
        child = Frame(owner, method, replace(receiver, type=owner.qualified), (*frame.parents, site), follow)
        bindings = self.bind_arguments(owner, method, values)
        nested = replace(state, bindings=bindings, initialized={})
        if follow:
            nested = self.value_arguments(frame, nested, child, site, args, values)
        rows = self.statement(child, nested, method[8])
        return_type = self.units.nominal_type(owner, method[4], method[12])
        return self.follow_results(frame, state, rows, local, owner, method, return_type, follow, inherited)

    def open_value(self, frame, state, invocation, reason, *sites):
        return self.event(frame, state, "value-boundary",
                          f"Value flow stops at {invocation.label}. {reason}", invocation.site, *sites)

    def value_arguments(self, frame, nested, child, call, args, values):
        bindings = dict(nested.bindings)
        conditions = list(nested.conditions)
        for arg, actual, parameter in zip(args, values, child.method[6]):
            scalar = self.scalar(actual)
            if not scalar and not actual.type_sources:
                continue
            name = parameter[3]
            argument = self.code(frame, arg)
            sites = proof_sites(call, child.site(parameter), frame.site(arg), actual.site,
                                *actual.flow, *actual.type_sources)
            value = scalar or f"the value of {argument}"
            mapping = f"`{child.owner.name}.{child.method[3]}` receives {value} for `{name}`"
            if scalar and argument not in {scalar, f"`{name}`"}:
                mapping += f" from {argument}"
            conditions.append(Condition(mapping, sites))
            bindings[name] = replace(bindings[name], flow=sites)
        return replace(nested, bindings=bindings, conditions=tuple(conditions))

    def follow_results(self, frame, initial, rows, invocation, owner, method, return_type, follow, inherited):
        results = []
        for row in rows:
            result = replace(invocation, type=return_type)
            if row.exit == "return":
                result = replace(row.result, type=return_type or row.result.type)
                if follow and return_type:
                    result = replace(result, type_sources=proof_sites(invocation.site, declaration_anchor(owner, method),
                                                                    row.return_site, *inherited, *result.type_sources))
                transferred = bool(self.scalar(result) or result.loop_element or return_type == "boolean")
                if follow and (transferred or result.flow):
                    evidence = proof_sites(invocation.site, declaration_anchor(owner, method), row.return_site,
                                           *result.flow, *result.type_sources)
                    result = replace(result, flow=evidence, transferred_result=transferred)
                elif self.value_flow and not follow and (self.scalar(result) or result.flow or result.loop_element):
                    result = replace(result, label=invocation.label, site=invocation.site,
                                     truth=None, number=None, string=None, flow=(),
                                     nullable=None if not result.constructed else result.nullable,
                                     loop_element=(), transferred_result=False)
            elif (follow and row.exit == "open" and return_type != "void"
                  and not any(event.kind == "value-boundary" for event in row.value_events[len(initial.value_events):])):
                row = self.open_value(frame, row, invocation, "The retained path reaches a bounded or unsupported part of the body.",
                                      declaration_anchor(owner, method))
            resumed = replace(row, bindings=initial.bindings, initialized=initial.initialized,
                              exit=None if row.exit == "return" else row.exit)
            results.append((resumed, result))
        return results

    def source_signature(self, frame, owner, name, values):
        # Generic or variable arity overloads require compiler selection.
        # The conditional source premise covers inherited declarations and runtime dispatch.
        if owner.node[4] not in {"CLASS", "RECORD", "ENUM", "INTERFACE"}:
            return None
        methods = owner.methods(name)
        if any(method[9] != "DIRECT" or method[13] != "FIXED" for method in methods):
            return None
        candidates = [method for method in methods if len(method[6]) == len(values)]
        if len(candidates) > 1 and any(p[5] != "DIRECT" for method in candidates for p in method[6]):
            return None
        actuals = tuple(self.units.comparable_type(frame.owner, value.type) for value in values)
        matching = []
        for method in candidates:
            declared = tuple(self.units.nominal_type(owner, p[4], method[12]) for p in method[6])
            if not all(declared):
                return None
            if all(actual == formal or (len(candidates) == 1 and not actual and value.nullable is True
                                       and formal not in DESCRIPTORS)
                   for actual, formal, value in zip(actuals, declared, values)):
                matching.append(method)
        return matching[0] if len(matching) == 1 else None

    def linked_method(self, call, compiler_target, name):
        if call not in self.selected:
            return None, None, False
        body, fixed = self.selected[call]
        owner, method = self.units.method_at(body)
        if not method or method[3] != name:
            return None, None, False
        if compiler_target and not compiler_target.startswith(f"method:class:{owner.qualified}#{name}("):
            return None, None, False
        return owner, method, fixed

    def exact_overload(self, frame, owner, name, values):
        methods = owner.methods(name)
        candidates = [method for method in methods if len(method[6]) == len(values)]
        if len(candidates) < 2 or any(not value.type for value in values):
            return None
        # Exact types win before conversions. Unknown, generic and potentially
        # variable-arity declarations leave the syntax-only choice open.
        if any(method[9] != "DIRECT" or any(not p[4] or p[4].endswith("[]") or p[5] != "DIRECT" for p in method[6])
               for method in methods):
            return None
        actuals = tuple(self.units.comparable_type(frame.owner, value.type) for value in values)
        matches = []
        for method in candidates:
            declared = tuple(self.units.qualify(owner, p[4], method[12]) for p in method[6])
            if not all(declared):
                return None
            if tuple(self.units.comparable_type(owner, name) for name in declared) == actuals:
                matches.append(method)
        return matches[0] if len(matches) == 1 else None

    def lookup_witness(self, frame, state, owner, method, call, args, values, receiver):
        anchor = declaration_anchor(owner, method)
        documents = self.documentation(owner, method)
        body = method[8]
        proof = [call.site, anchor, receiver.site, *receiver.type_sources]
        mappings = []
        for argument, actual, formal in zip(args, values, method[6]):
            parameter = self.units.code(owner.site(formal)) or f"`{formal[3]}`"
            argument_code = self.code(frame, argument)
            mapping = f"{parameter} receives {argument_code}"
            if initializer := self.lookup_initializer(frame, argument, actual):
                mapping += f" from {self.code(frame, initializer)}"
                proof.append(frame.site(initializer))
            mappings.append(mapping)
            proof.extend((frame.site(argument), *(actual.type_sources or (actual.site,))))
        text = f"In the cited overload for {call.label}, " + ", ".join(mappings) + "."
        role = "lookup-contract"
        setup = body[3][:-2]
        if (lookup := FILTERED_LOOKUP(body)) and all(
                statement[0] in {"let", "expr", "if", "assert", "empty"} and unchanged_guard(statement)
                for statement in setup):
            loop, fallback, routes = lookup
            element, iterable, _ = loop[3:]
            inputs = {parameter[3] for parameter in method[6]}
            returned = [tests for tests, found in routes if found]
            stable = all(unchanged_guard(test) for tests, _ in routes for test, _ in tests)
            if stable and all(any(lookup_comparison(test, holds, element[3], inputs) for test, holds in tests) for tests in returned):
                tests = [" and ".join(f"{self.units.code(owner.site(test)) or 'the cited test'} is {str(holds).lower()}"
                                      for test, holds in group) for group in returned]
                sequence = self.units.code(owner.site(iterable)) or "the cited iterable"
                text += (f" The loop over {sequence} returns the first element `{element[3]}` that reaches a return arm. "
                         "The return guards are " + " or ".join(tests) + ". The method returns `null` if traversal finishes without a match.")
                proof.extend((owner.site(loop), owner.site(fallback)))
                role = "filtered-lookup"
        if role == "lookup-contract" and not documents:
            return state
        if documents:
            comment, site = documents
            text += f" The declaration documents “{comment}”"
            proof.append(site)
        premise_text = f"preceding evaluations reach {call.label} and dispatch enters the cited overload"
        if role == "filtered-lookup" and setup:
            premise_text += " and reaches the loop"
        premise = Condition(premise_text, (call.site, anchor), lookup_scope=True)
        scope = tuple(condition for condition in state.conditions if condition.lookup_scope)
        scoped = replace(state, conditions=(*scope, premise))
        witnessed = self.event(frame, scoped, role, text, *proof)
        return replace(witnessed, conditions=state.conditions)

    @staticmethod
    def lookup_initializer(frame, argument, actual):
        """Keep an initializer only when it explains the lexical argument on every route."""
        if argument[0] != "id" or not actual.type_sources:
            return None
        declaration = actual.type_sources[0]
        if declaration.path != frame.owner.path or declaration.end > argument[1]:
            return None
        name = argument[3]
        initializer = None
        for node in syntax_nodes(frame.method[8]):
            if node[0] == "let" and frame.site(node) == declaration and node[3] == name:
                initializer = node[4]
            if (declaration.start < node[1] < argument[1] and node[0] in {"set", "mutate"}
                    and node[3][0] == "id" and node[3][3] == name):
                return None
        return initializer

    def documentation(self, owner, method):
        if comment := method[11]:
            site = owner.site(comment)
            raw = self.units.snapshot.read(site.path)[site.start:site.end].decode()
            text = " ".join(line.lstrip(" *") for line in raw.splitlines()).strip()
            if text and len(text) <= 275:
                return text, site
        return None

    def contract(self, frame, state, owner, method, call, *, declaration_only=False):
        anchor = declaration_anchor(owner, method)
        if document := self.documentation(owner, method):
            text, site = document
            bases = ()
            if declaration_only:
                signature = self.units.code(anchor) or f"`{method[3]}`"
                text = (f"Interface `{owner.qualified}` declares {signature} and documents its contract as “{text}” "
                        "Inherited overload resolution and runtime dispatch remain open.")
                bases = tuple(owner.site(parent) for parent in owner.node[11])
            else:
                text = f"The cited declaration for {call.label} describes its contract as “{text}”"
            state = self.event(frame, state, "contract", text, call.site, anchor, site, *bases)
        return self.annotations(frame, state, owner, method, call, anchor)

    def compatible_contract(self, frame, owner, method, receiver, values):
        if method[8] is not None or not receiver.site or receiver.type != owner.qualified or owner.node[4] != "INTERFACE":
            return False
        for parameter, value in zip(method[6], values):
            formal = self.units.qualify(owner, parameter[4], method[12])
            if not formal or parameter[5] != "DIRECT":
                return False
            if value.nullable is True and formal not in DESCRIPTORS:
                continue
            if (not value.type or self.units.comparable_type(owner, formal)
                    != self.units.comparable_type(frame.owner, value.type)):
                return False
        return True

    def annotations(self, frame, state, owner, method, call, declaration):
        for item in owner.node[6]:
            site = owner.site(item)
            if any(event.kind == "type-annotation" and event.cites[0] == site for event in state.events):
                continue
            if label := self.units.code(site):
                state = self.event(frame, state, "type-annotation", f"Type declaration `{owner.qualified}` carries {label}.",
                                   site, call.site)
        for item in method[7]:
            site = owner.site(item)
            if label := self.units.code(site):
                text = f"Method declaration `{owner.qualified}.{method[3]}` carries {label}."
                state = self.event(frame, state, "annotation", text, site, call.site, declaration)
        return state

    def bind_arguments(self, owner, method, actuals):
        declarations = ((parameter[3], self.units.qualify(owner, parameter[4], method[12])) for parameter in method[6])
        return {name: replace(actual, type=type_name or actual.type) for (name, type_name), actual in zip(declarations, actuals)}

    @staticmethod
    def match(owner, name, arity):
        # The emitted dispatch premise covers inherited choices. Overloads
        # declared here must identify one visible source candidate themselves.
        methods = owner.methods(name)
        if any(method[9] != "DIRECT" or any(not parameter[4] or parameter[4].endswith("[]") for parameter in method[6])
               for method in methods):
            return None
        candidates = [method for method in methods if len(method[6]) == arity]
        return candidates[0] if len(candidates) == 1 else None

    def optional(self, frame, state, node, receiver, name, is_static, values, local):
        if is_static and ((name == "empty" and not values) or (name in {"of", "ofNullable"} and len(values) == 1)):
            if name == "of" and values[0].nullable is True:
                return [(replace(state, exit="open"), UNKNOWN)]
            known = "empty" if name == "empty" or (name == "ofNullable" and values[0].nullable is True) else None
            if name == "of" or (name == "ofNullable" and values[0].nullable is False):
                known = "present"
            return [(state, replace(local, type=OPTIONAL, optional=known, nullable=False,
                                    identity=local.site, provenance=receiver.provenance))]
        if is_static or values:
            return None
        known = state.optionals.get(receiver.identity) or receiver.optional
        if name in {"isEmpty", "isPresent"}:
            expected = "empty" if name == "isEmpty" else "present"
            return [(state, replace(local, truth=known == expected if known else None,
                                    test=(receiver.identity, expected) if receiver.identity else None))]
        if name == "get":
            rows = []
            for presence in ((known,) if known else ("empty", "present")):
                branch = state
                if not known:
                    label = receiver.label or "the cited Optional"
                    proof = tuple(anchor for anchor in (receiver.site, *receiver.provenance, local.site) if anchor)
                    condition = Condition(f"{label} is {'empty' if presence == 'empty' else 'present'}", proof)
                    optionals = {**state.optionals, receiver.identity: presence} if receiver.identity else state.optionals
                    branch = replace(state, conditions=(*state.conditions, condition), optionals=optionals)
                if presence == "empty":
                    text = f"{local.label} throws `NoSuchElementException` for an empty `java.util.Optional`."
                    branch = self.event(frame, branch, "optional-throw", text, local.site, receiver.site, *receiver.provenance)
                    rows.append((replace(branch, exit="throw"), UNKNOWN))
                else:
                    branch = self.event(frame, branch, "attempt", local.label, local.site, receiver.site, *receiver.provenance)
                    rows.append((branch, replace(local, nullable=False)))
            return rows
        return None

    def mark_bypass(self, frame, state, node):
        if state.exit != "throw" or not node or node[0] != "call" or not state.events:
            return state
        failure = state.events[-1]
        if failure.kind != "optional-throw" or failure.bypass or frame.site(node) == failure.cites[0]:
            return state
        site = frame.site(node)
        text = f"{failure.text} This exit skips {self.code(frame, node)}."
        return replace(state, events=(*state.events[:-1], replace(failure, text=text, cites=(*failure.cites, site), bypass=site)))

    def construct(self, frame, state, node, type_name, args, values):
        if state.exit:
            return [(state, UNKNOWN)]
        site = frame.site(node)
        declared = self.qualify(frame, type_name)
        owner = self.units.lookup(frame.owner, declared)
        proof = (self.publications.construct(frame, node, type_name, values,
                                             self.compiler_targets.get(site)) if self.publications else None)
        value = Value(self.code(frame, node), site, declared, declared, nullable=False, constructed=True, arguments=values,
                      publication_operand=proof)
        state = self.shared_construction(frame, state, value, values)
        if not owner:
            return [(self.event(frame, state, "attempt", value.label, site), value)]
        constructors = owner.methods("<init>")
        if owner.record and not constructors:
            state, value = self.record_components(frame, state, owner, value, args, values)
        for field in owner.fields():
            initial = field[6]
            if "STATIC" in field[5] or not initial or initial[0] != "literal":
                continue
            if initializer := self.units.code(owner.site(initial)):
                state = self.event(frame, state, "construction", f"Construction of `{owner.name}` initializes field `{field[3]}` to {initializer} at its declaration.",
                                   site, owner.site(field))
        method = self.match(owner, "<init>", len(values))
        if not method or not method[8] or len(frame.parents) >= MAX_DEPTH or owner.node[8] != "DIRECT":
            return [(state, value)]
        premise = Condition(f"{value.label} resolves to the cited constructor", (site, declaration_anchor(owner, method)))
        state = replace(state, conditions=(*state.conditions, premise))
        constructor = Frame(owner, method, value, (*frame.parents, site), value_flow=False)
        bindings = self.bind_arguments(owner, method, values)
        rows = self.statement(constructor, replace(state, bindings=bindings, initialized={}), method[8])
        result = []
        for row in rows:
            built = replace(value, final_fields=tuple(row.initialized.items()))
            if owner.record and method[10] == "COMPACT" and row.exit is None:
                supplied = [row.bindings.get(parameter[3], UNKNOWN) for parameter in method[6]]
                row, built = self.record_components(frame, row, owner, value, args, supplied, compact=True)
            result.append((replace(row, bindings=state.bindings, initialized=state.initialized,
                                   exit=None if row.exit == "return" else row.exit), built))
        return result

    def shared_construction(self, frame, state, wrapper, values):
        inputs = {}
        for value in values:
            if not value.constructed:
                continue
            for argument in value.arguments:
                if argument.site and argument.label and not argument.constructed:
                    inputs.setdefault((argument.site, argument.label), {})[value.site] = value.label
        for (origin, label), constructions in inputs.items():
            if len(constructions) > 1:
                built = " and ".join(constructions.values())
                text = f"{built} both take the value from {label}. These constructed values are passed to {wrapper.label}."
                state = self.event(frame, state, "shared-construction", text, wrapper.site, origin, *constructions)
        return state

    def record_components(self, frame, state, owner, value, args, values, *, compact=False):
        components = [field for field in owner.fields() if "STATIC" not in field[5]]
        if len(components) != len(values):
            return state, value
        for component, arg, actual in zip(components, args, values):
            argument = self.code(frame, arg)
            target = f"record component `{component[3]}`"
            if compact:
                text = f"When construction by {value.label} completes normally, the compact constructor assigns {target}."
                if actual.label:
                    text += f" Its value comes from {actual.label}."
            else:
                text = f"Constructing {value.label} supplies {argument} to {target}."
                if actual.entries:
                    text += f" The component receives the value of {actual.label}."
            state = self.event(frame, state, "construction", text, value.site, owner.site(component), actual.site)
        proved = tuple((component[3], replace(actual, provenance=(*actual.provenance, value.site, owner.site(component))))
                       for component, actual in zip(components, values))
        return state, replace(value, components=proved)


def syntax_nodes(node):
    if isinstance(node, list) and node:
        if isinstance(node[0], str):
            yield node
            node = node[3:]
        for child in node:
            yield from syntax_nodes(child)


def unchanged_guard(node):
    return all(part[0] not in {"set", "mutate", "open"} for part in syntax_nodes(node))


def element_access(node, name):
    kind = node[0]
    if kind == "id":
        return node[3] == name
    if kind == "member":
        return element_access(node[3], name)
    return kind == "call" and not node[4] and node[3][0] == "member" and element_access(node[3], name)


def lookup_comparison(node, holds, element, parameters):
    kind, data = node[0], node[3:]
    if kind == "not":
        return lookup_comparison(data[0], not holds, element, parameters)
    if (kind == "and" and holds) or (kind == "or" and not holds):
        return any(lookup_comparison(part, holds, element, parameters) for part in data)
    if kind in {"eq", "neq"} and holds == (kind == "eq"):
        pair = data
    elif kind == "call" and holds and data[0][0] == "member" and data[0][4] == "equals":
        receiver, arguments = data[0][3], data[1]
        pair = (receiver, *arguments) if len(arguments) == 1 else arguments
        if len(arguments) != 1 and not (len(arguments) == 2 and receiver[0] in {"id", "member"}):
            return False
    else:
        return False
    left, right = pair
    return any(key[0] == "id" and key[3] in parameters and element_access(value, element)
               for key, value in ((left, right), (right, left)))


def obligation(process, context):
    """Match sealed declaration and compiler-call ranges to unit-local positions."""
    occurrences = context.get("_sourceOccurrences", {})
    local = {ref: (site["id"], occurrences[ref]) for site in context.get("sourceRanges", ())
             for ref in site.get("references", ()) if ref in occurrences
             and not (occurrences[ref].get("peerProcess") or occurrences[ref].get("peerService"))}
    bodies = {}
    for row in process.get("facts", ()):
        if row.get("kind") in {"BODY", "BODY_SOURCE"} and row.get("declaration"):
            candidates = bodies.setdefault(row["declaration"], {})
            for ref in row.get("sources", ()):
                if ref in local and local[ref][1]["file"].endswith(".java"):
                    identifier, source = local[ref]
                    candidates[identifier] = source
    root = bodies.get(process.get("root"), {})
    if len(root) != 1:
        return None, None, (), ()
    (root_id, source), = root.items()
    events = {row.get("occurrence"): row for row in process.get("facts", ())
              if row.get("event") == "CALL" and row.get("resolution") == "COMPILER_EXACT"}
    selected = []
    for row in process.get("facts", ()):
        event_id, callee_id = row.get("eventId"), row.get("targetMethodId")
        if not isinstance(event_id, str) or row.get("resolution") not in {"SAME_SCOPE_UNIQUE", "SAME_SCOPE_SELECTED_DECLARATION"}:
            continue
        callee = bodies.get(callee_id, {})
        target = events.get(event_id) or events.get(event_id.removeprefix("EVENT:"))
        if not target or target.get("declaration") != row.get("declaration") or not target.get("target") \
                or target["target"] != row.get("target") or len(callee) != 1:
            continue
        target_source = next(iter(callee.values()))
        for ref in target.get("sources", ()):
            if ref in local:
                selected.append((local[ref][1], target_source))
    targets = [(local[ref][1], event["target"]) for event in process.get("facts", ())
               if event.get("event") in {"CALL", "CONSTRUCT"} and event.get("resolution") == "COMPILER_EXACT"
               and isinstance(event.get("target"), str) and event["target"].startswith("method:class:")
               for ref in event.get("sources", ()) if ref in local]
    return root_id, source, selected, targets


def native_links(root, projection, ranges):
    if projection.get("root") != root:
        return
    fixed_calls = {}
    for outcome in projection.get("outcomes", ()):
        caller = outcome.get("caller") or {}
        if caller.get("binding") == "FIXED" and caller.get("calleeBody") and caller.get("callSource") in ranges:
            fixed_calls.setdefault(caller["calleeBody"], set()).add(caller["callSource"])
    for body in projection.get("calleeBodies", ()):
        declaration, binding = body.get("declaration"), body.get("binding")
        target = ranges.get(body.get("source"))
        if not declaration or declaration == root or not target:
            continue
        calls = fixed_calls.get(declaration, ()) if binding == "FIXED" else (body.get("callSource"),)
        for identifier in sorted(call for call in calls if call in ranges):
            if binding in {"FIXED", "SOURCE_CANDIDATE"}:
                yield ranges[identifier], target, binding == "FIXED"


def pinned_site(snapshot, source):
    if not source.get("text") or source.get("authority", "EXACT_SNAPSHOT_TEXT") != "EXACT_SNAPSHOT_TEXT":
        return None
    raw = snapshot.file_for(source)
    if not raw or (first := snapshot.match(raw, source)) is None:
        return None
    literal = source["text"].encode()
    return Site(source["file"], first + len(literal) - len(literal.lstrip()), first + len(literal.rstrip()))


def declaration_anchor(owner, method):
    boundary = method[8][1] if method[8] else method[2]
    return Site(owner.path, method[1], boundary)


def compact_sites(sites, *, covered):
    combined = []
    for site in sorted(set(sites), key=lambda row: (row.path, row.start, row.end)):
        if combined and combined[-1].path == site.path:
            earlier = combined[-1]
            if site.end <= earlier.end:
                continue
            if (site.start - earlier.end <= 45 and site.end - earlier.start <= 500
                    and covered(site.path, earlier.start, site.end)):
                combined[-1] = Site(site.path, earlier.start, site.end)
                continue
        combined.append(site)
    return combined


def source_location(snapshot, site):
    raw = snapshot.read(site.path)
    if raw is None:
        return site.path
    first = raw.rfind(b"\n", 0, site.start) + 1
    line = raw.count(b"\n", 0, first) + 1
    column = len(raw[first:site.start].decode()) + 1
    return f"{site.path}:{line}:{column}"


def displayed_conditions(snapshot, conditions, *, brief, context=()):
    clauses = dict.fromkeys((row.brief if brief and row.brief else row.text, row.decision) for row in conditions)
    repeated = defaultdict(dict)
    for decision in (row.decision for row in (*conditions, *context)):
        if decision:
            repeated[decision.expression][decision] = None
    scopes, call_sites = {}, []
    displayed = {decision for _, decision in clauses if decision}
    for group in repeated.values():
        if len(group) < 2:
            continue
        methods = Counter(decision.method for decision in group)
        sites = Counter((decision.method, decision.site) for decision in group)
        for decision in group:
            if decision not in displayed:
                continue
            label = f" in `{decision.method}`"
            if methods[decision.method] > 1:
                label += f" at {source_location(snapshot, decision.site)}"
            if sites[decision.method, decision.site] > 1 and decision.parents:
                label += " via " + " → ".join(source_location(snapshot, site) for site in decision.parents)
                call_sites.extend(decision.parents)
            scopes[decision] = label
    return " and ".join(text + scopes.get(decision, "") for text, decision in clauses), call_sites


def cite_fact(snapshot, event, root_id, prefix, text, kind, remaining, *, branch_conditions=()):
    flow = event.kind in VALUE_EVENT_KINDS
    condition, scope_sites = displayed_conditions(snapshot, event.conditions, brief=flow, context=branch_conditions)
    if not flow and len(condition) > 750:
        return None
    premises = (*event.conditions, *branch_conditions)
    required = [*event.cites, *event.parents, *prefix, *scope_sites, *(site for item in premises for site in item.cites)]
    summary = {"authority": "DECLARED_SOURCE_CONTRACT" if event.kind in {"contract", "lookup-contract"} else AUTHORITY,
               "kind": kind, "text": text, **({"condition": condition} if condition else {})}
    if flow:
        virtual = any(condition.dispatch_candidate for condition in premises)
        uncertainty = ("Applies if runtime dispatch enters the cited body and evaluation completes normally."
                       if virtual else "Applies to normal evaluation along the cited source path.")
        if event.opaque_calls:
            uncertainty += " An opaque call can still fail before this return."
        summary.update(authority="RETAINED_JAVA_VALUE_FLOW",
                       status="OPEN" if event.kind == "value-boundary" else "CONDITIONAL",
                       uncertainty="The value at this call remains open." if event.kind == "value-boundary" else uncertainty)
    spans = (compact_sites(required, covered=snapshot.covered) if hasattr(snapshot, "covered")
             else list(dict.fromkeys(required)))
    if len(json.dumps(summary, ensure_ascii=False).encode()) + 24 * len(spans) + 35 > remaining:
        return None
    coordinates = [(site.path, site.start, site.end) for site in spans]
    refs = (snapshot.cite_many(coordinates) if hasattr(snapshot, "cite_many")
            else [snapshot.cite(*span) for span in coordinates])
    if not refs or not all(refs):
        return None
    return {**summary, "sources": list(dict.fromkeys((root_id, *refs)))}


def ordered_effects(state):
    frames = {}
    for event in state.events:
        if event.depth and event.kind in {"attempt", "selected-call", "mutation", "constructor-write"}:
            frames.setdefault(event.parents, []).append(event)
    for chain in frames.values():
        if len(chain) < 2:
            continue
        included = [*chain[:4], *chain[-3:]] if len(chain) > 7 else chain
        sequence = " → ".join(event.text for event in included)
        yield replace(chain[-1], kind="ordered-effects", text=f"On a path that reaches these evaluations, their order includes {sequence}.",
                      cites=tuple(site for event in included for site in event.cites))


def facts(snapshot, projections, root_id):
    ranked = []
    seen = set()
    response_sites = {state.result.site for state in projections if state.exit == "return" and state.result.components}
    for state in projections:
        for ordered in ordered_effects(state):
            key = ordered.kind, ordered.text, ordered.cites, ordered.conditions
            if key not in seen:
                ranked.append((1, ordered, (), ordered.text))
                seen.add(key)
        calls = []
        terminal = next((event for event in reversed(state.events)
                         if event.kind in {"attempt", "selected-call", "mutation", "throw", "optional-throw"}), None)
        for event in state.events:
            if event.kind in {"attempt", "selected-call"}:
                if event is terminal and event.kind == "attempt" and event.depth:
                    text = f"The reached collaborator attempts {event.text}."
                    key = "invocation", event.text, event.cites, event.conditions
                    if key not in seen:
                        ranked.append((6, replace(event, kind="invocation"), (), text))
                        seen.add(key)
                calls.append(event)
                continue
            if event.kind in {"mutation", "constructor-write"} and not event.depth:
                continue
            key = (event.kind, event.cites[0]) if event.kind == "type-annotation" else (event.kind, event.text, event.cites, event.conditions)
            if key in seen:
                continue
            seen.add(key)
            prefix = [call for call in calls if call.cites[0] in event.parents or call.parents in {event.parents, event.parents[:-1]}]
            prefix = [*prefix[:1], *prefix[-4:]] if len(prefix) > 5 else prefix
            prefix = prefix if event.kind in {"mutation", "throw", "optional-throw"} else []
            text = event.text
            if prefix:
                sequence = " → ".join(call.text for call in prefix)
                text = f"On a path that reaches this point, the call order includes {sequence}. {text}"
            rank = {"optional-throw": 0, "mutation": 2, "throw": 2, "shared-construction": 3,
                    "constructed-result": 5, "construction": 6 if event.depth else 1,
                    "constructed-argument": 7, "contract": 0, "annotation": 4,
                    "constructor-write": 8, "type-annotation": 9}
            response_component = event.kind == "construction" and not event.depth and event.cites[0] in response_sites
            ranked.append((-1 if response_component else rank[event.kind], event,
                           tuple(site for call in prefix for site in call.cites), text))
    # Give returned record fields independent slots before recurring callee
    # evidence or variants from the same constructor.
    buckets = {}
    for row in sorted(ranked, key=lambda item: item[0]):
        level, event, _, _ = row
        scope = event.cites[:2] if level == -1 else ()
        if event.kind in {"ordered-effects", "contract"}:
            scope = event.parents
        buckets.setdefault((level, event.kind, scope), []).append(row)
    size = 0
    result = []
    covered = set()
    for _, event, prefix, text in (row for batch in zip_longest(*buckets.values()) for row in batch if row):
        if len(result) >= MAX_FACTS:
            break
        if event.kind in {"mutation", "constructor-write"} and (event.cites[0], event.conditions) in covered:
            continue
        kind = ("java-callee-path" if event.kind in {"mutation", "constructor-write", "throw", "optional-throw"}
                else f"java-{event.kind}")
        if row := cite_fact(snapshot, event, root_id, prefix, text, kind, MAX_FACT_BYTES - size):
            size += len(json.dumps(row, ensure_ascii=False).encode())
            result.append(row)
            if event.kind == "ordered-effects":
                covered.update((site, event.conditions) for site in event.cites)
    return result


def iteration_pairs(snapshot, events):
    forks = defaultdict(dict)
    for event in events:
        if not event.iteration:
            continue
        required = (*event.cites, *event.parents, *(site for row in event.conditions for site in row.cites))
        if hasattr(snapshot, "covered") and not all(snapshot.covered(site.path, site.start, site.end) for site in required):
            continue
        start = event.iteration.premises
        for index, condition in enumerate(event.conditions[start:], start):
            decision = condition.decision
            if decision and condition.truth is not None and decision.parents == event.parents:
                siblings = forks[event.iteration, decision, event.conditions[:index]]
                role = event.kind, condition.truth
                if role not in siblings or len(event.conditions) < len(siblings[role].conditions):
                    siblings[role] = event
    candidates = []
    for (_, _, prefix), siblings in forks.items():
        for truth in (True, False):
            returned = siblings.get(("iteration-result", truth))
            skipped = siblings.get(("iteration-skip", not truth))
            if returned and skipped:
                candidates.append(IterationPair(returned, skipped, len(prefix)))
    pairs, paired = [], set()
    # Prefer the closest witnessed fork for a given outcome. Earlier forks
    # describe less similar paths and can leave the actual sibling unpaired.
    for pair in sorted(candidates, key=lambda candidate: -candidate.fork):
        if pair.returned not in paired and pair.skipped not in paired:
            pairs.append(pair)
            paired.update((pair.returned, pair.skipped))
    return pairs, paired


def cite_iteration_pair(snapshot, pair, identifier, remaining):
    returned, skipped, fork = pair.returned, pair.skipped, pair.fork
    common = returned.conditions[:fork]
    branches = (*returned.conditions[fork:], *skipped.conditions[fork:])
    texts, sites = [], [*returned.cites, *skipped.cites]
    for branch in (returned, skipped):
        condition, origins = displayed_conditions(snapshot, branch.conditions[fork:], brief=True,
                                                   context=(*common, *branches))
        texts.append(f"When {condition}, {branch.text[0].lower()}{branch.text[1:]}")
        sites.extend(origins)
    pair_event = replace(returned, kind="iteration-alternatives", cites=proof_sites(*sites), conditions=common,
                         opaque_calls=returned.opaque_calls or skipped.opaque_calls)
    return cite_fact(snapshot, pair_event, identifier, (), " ".join(texts), "java-iteration-alternatives", remaining,
                     branch_conditions=branches)


def value_facts(snapshot, states, identifier):
    events = dict.fromkeys(event for state in states for event in state.value_events)
    pairs, paired = iteration_pairs(snapshot, events)
    groups = {kind: {} for kind in ("iteration-alternatives", "transferred-result", "iteration-result", "iteration-skip",
                                    "value-decision", "value-boundary", "return-lineage")}
    for pair in pairs:
        groups["iteration-alternatives"][pair] = {pair: None}
    for event in events:
        if event not in paired:
            kind = "transferred-result" if event.transferred_result else event.kind
            groups[kind].setdefault((event.cites[0], event.text), {})[event] = None
    rows = []
    remaining = MAX_VALUE_FACT_BYTES

    def admit(choice):
        nonlocal remaining
        if isinstance(choice, IterationPair):
            row = cite_iteration_pair(snapshot, choice, identifier, remaining)
        else:
            row = cite_fact(snapshot, choice, identifier, (), choice.text, f"java-{choice.kind}", remaining)
        if row:
            rows.append(row)
            remaining -= len(json.dumps(row, ensure_ascii=False).encode())
        return bool(row)

    variants = []
    # Preserve a different result, decision, or scope boundary before spending
    # the budget on multiple paths through the same source result.
    for batch in zip_longest(*(group.values() for group in groups.values())):
        for alternatives in (item for item in batch if item):
            if len(rows) == MAX_VALUE_FACTS:
                return rows
            ordered = list(alternatives)
            if len(ordered) > 1:
                ordered.sort(key=lambda item: sum(len(row.text) for row in item.conditions))
            for index, choice in enumerate(ordered):
                if admit(choice):
                    variants.extend(ordered[index + 1:])
                    break
    for event in variants:
        if len(rows) == MAX_VALUE_FACTS:
            break
        admit(event)
    return rows


def entry_frame(units, owner, method, parents=()):
    exact = owner.qualified if owner.record or "FINAL" in owner.node[5] else ""
    frame = Frame(owner, method, Value(type=owner.qualified, exact=exact), parents)
    bindings = {}
    for parameter in method[6]:
        name, declared = parameter[3:5]
        site = owner.site(parameter)
        type_name = units.qualify(owner, declared, method[12])
        proof = owner.import_proof(declared, type_name)
        bindings[name] = Value(f"`{name}`", site, type_name, identity=site, entries=(name,), provenance=proof)
    return frame, bindings


def lookup_events(projection, frame, initial, entry):
    states = projection.statement(frame, initial, frame.method[8])
    events = [event for state in states for event in state.events if event.kind in {"filtered-lookup", "lookup-contract"}]
    if events:
        return events
    # A replayed fixed native relationship is also an exact source seed if an
    # unsupported caller statement prevented reaching it through the unit walk.
    attempts = 0
    for call, (body, fixed) in projection.selected.items():
        if not fixed or call.path != entry.path or not entry.start <= call.start < call.end <= entry.end:
            continue
        owner, method = projection.units.method_at(body)
        if not method or not method[8] or not (label := projection.units.code(call)):
            continue
        anchor = declaration_anchor(owner, method)
        premise = Condition(f"control reaches {label} and dispatch enters `{owner.qualified}.{method[3]}`",
                            (call, anchor), lookup_scope=True)
        frame, bindings = entry_frame(projection.units, owner, method, (call,))
        states = projection.statement(frame, State(bindings=bindings, conditions=(premise,)), method[8])
        events.extend(event for state in states for event in state.events if event.kind in {"filtered-lookup", "lookup-contract"})
        attempts += 1
        if events or attempts >= MAX_LOOKUP_FACTS:
            break
    return events


def lookup_facts(snapshot, events, identifier):
    by_declaration = {}
    for event in events:
        identity = event.kind, event.cites[:2], event.text
        if previous := by_declaration.get(identity):
            shared = tuple(condition for condition in previous.conditions if condition in event.conditions)
            event = replace(previous, conditions=shared)
        by_declaration[identity] = event
    rows = []
    for event in by_declaration.values():
        if row := cite_fact(snapshot, event, identifier, (), event.text, f"java-{event.kind}", 2900):
            rows.append(row)
        if len(rows) >= MAX_LOOKUP_FACTS:
            break
    return rows


@contextmanager
def open_units(snapshot, *, dispatch=False):
    java = shutil.which("java")
    if not java:
        yield None
        return
    script = Path(__file__).with_name("clew_prepared_java_syntax.java")
    with subprocess.Popen([java, "--source", "21", str(script), "--dispatch-unit" if dispatch else "--unit"], stdin=subprocess.PIPE,
                          stdout=subprocess.PIPE, stderr=subprocess.DEVNULL) as parser:
        try:
            yield Units(snapshot, parser)
        finally:
            try:
                parser.kill()
            except ProcessLookupError:
                pass


def project_frame(units, owner, method, identifier, entry, selected_bodies, compiler_targets, *,
                  publication=True, value_flow=True, entry_kind=None, receiver_event=None):
    publications = (PUBLICATIONS["Publications"](units, entry_kind=entry_kind, receiver_event=receiver_event)
                    if publication else None)
    projection = Projection(units, selected_bodies, compiler_targets, publications=publications, value_flow=value_flow)
    frame, bindings = entry_frame(units, owner, method)
    if publications and receiver_event and len(method[6]) == 1:
        parameter = method[6][0]
        if operand := publications.envelope(owner, parameter):
            name = parameter[3]
            bindings[name] = replace(bindings[name], publication_operand=operand)
    initial = State(bindings=bindings)
    states = projection.statement(frame, initial, method[8])
    values = value_facts(units.snapshot, states, identifier) if value_flow else []
    original = facts(units.snapshot, states, identifier)
    lookups = Projection(units, selected_bodies, compiler_targets, lookup_only=True)
    events = lookup_events(lookups, frame, initial, entry)
    lookup = lookup_facts(units.snapshot, events, identifier)
    submissions = publications.facts(units.snapshot, identifier) if publications else []
    return [*submissions, *values, *original, *lookup]


def derive(process, context, snapshot, *, native=None, native_ranges=None):
    if not snapshot:
        return []
    identifier, source, selected, targets = obligation(process, context)
    if not identifier or not (entry := pinned_site(snapshot, source)):
        return []
    try:
        with open_units(snapshot) as units:
            if units is None:
                return []
            owner, method = units.method_at(entry)
            if method is None or not method[8]:
                return []
            known = {}
            links = [(invocation, declaration, True) for invocation, declaration in selected]
            links.extend(native_links(process.get("root"), native or {}, native_ranges or {}))
            for invocation, declaration, fixed in links:
                if (call := pinned_site(snapshot, invocation)) and (body := pinned_site(snapshot, declaration)):
                    candidates = known.setdefault(call, {})
                    candidates[body] = fixed or candidates.get(body, False)
            selected_bodies = {}
            for call, candidates in known.items():
                exact = [site for site, fixed in candidates.items() if fixed]
                if len(exact) == 1:
                    selected_bodies[call] = exact[0], True
                elif not exact and len(candidates) == 1:
                    selected_bodies[call] = next(iter(candidates)), False
            compiler_targets = {}
            for invocation, target in targets:
                if site := pinned_site(snapshot, invocation):
                    compiler_targets.setdefault(site, set()).add(target)
            unique_targets = {site: next(iter(values)) for site, values in compiler_targets.items() if len(values) == 1}
            entry_kind = process.get("entryKind")
            trigger = process.get("trigger") or {}
            config = trigger.get("configuration") or {}
            receiver_event = (config.get("eventType") if entry_kind == trigger.get("kind") == "JAVA_TYPED_DISPATCH"
                              and config.get("adapter") == PUBLICATIONS["ADAPTER"] else None)
            return project_frame(units, owner, method, identifier, entry, selected_bodies, unique_targets,
                                 entry_kind=entry_kind, receiver_event=receiver_event)
    except (OSError, ValueError, RecursionError, subprocess.SubprocessError):
        return []
