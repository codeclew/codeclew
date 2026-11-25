"""Project bounded Java call witnesses from the operation's pinned source."""

from base64 import b64encode
from contextlib import contextmanager
from dataclasses import dataclass, field, replace
import json
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
MAX_FACTS = 16
MAX_FACT_BYTES = 6500
MAX_LOOKUP_FACTS = 2
FILTERED_LOOKUP = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_control.py")))["filtered_lookup"]
PUBLICATIONS = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_publication.py")))


@dataclass(frozen=True)
class Site:
    path: str
    start: int
    end: int


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
JAVA_LANG = {"String", "Object", "Boolean", "Byte", "Character", "Short", "Integer", "Long", "Float", "Double", "Void"}


@dataclass(frozen=True)
class Condition:
    text: str
    cites: tuple[Site, ...]
    lookup_scope: bool = False


@dataclass(frozen=True)
class Event:
    kind: str
    text: str
    cites: tuple[Site, ...]
    depth: int
    parents: tuple[Site, ...]
    conditions: tuple[Condition, ...]
    bypass: Site | None = None


@dataclass(frozen=True)
class State:
    bindings: dict = field(default_factory=dict)
    events: tuple[Event, ...] = ()
    conditions: tuple[Condition, ...] = ()
    optionals: dict = field(default_factory=dict)
    initialized: dict = field(default_factory=dict)
    exit: str | None = None
    result: Value = UNKNOWN


@dataclass(frozen=True)
class Frame:
    owner: Type
    method: list
    this: Value
    parents: tuple[Site, ...] = ()

    def site(self, node):
        return self.owner.site(node)


class Units:
    def __init__(self, snapshot, parser):
        self.snapshot = snapshot
        self.parser = parser
        self.units = {}
        self.reads = 0
        self.searches = {}

    def read(self, path):
        if path not in self.units:
            self.units[path] = ()
            if (len(self.units) <= MAX_LOOKUPS and self.reads < MAX_FILES
                    and (data := self.snapshot.read(path)) and len(data) <= MAX_FILE_BYTES):
                self.reads += 1
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

    def lookup(self, owner, name):
        if not name or name == OPTIONAL or name in DESCRIPTORS or not all(part.isidentifier() for part in name.split(".")):
            return None
        key = owner.path, name
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
                       for entry in self.read(path) if entry.qualified == name}
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
        short = name.rpartition(".")[2]
        local = ".".join(part for part in (owner.package, short) if part)
        if (name == local and short in JAVA_LANG and not any(entry.endswith(".*") for entry in owner.imports)
                and not self.lookup(owner, name)
                and not (hasattr(self.snapshot, "type_paths") and self.snapshot.type_paths(name))
                and getattr(self.snapshot, "complete_tree", True)):
            return "java.lang." + short
        return name

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


class Projection:
    def __init__(self, units, selected, compiler_targets, *, lookup_only=False, publications=None):
        self.units = units
        self.selected = selected
        self.compiler_targets = compiler_targets
        self.lookup_only = lookup_only
        self.publications = publications
        self.steps = MAX_STEPS

    def code(self, frame, node):
        return self.units.code(frame.site(node)) or "the cited expression"

    def qualify(self, frame, name):
        return self.units.qualify(frame.owner, name, frame.method[12])

    def bind(self, state, name, value, declared_type="", type_sources=()):
        if declared_type:
            value = replace(value, type=declared_type, type_sources=type_sources)
        return replace(state, bindings={**state.bindings, name: value})

    def event(self, frame, state, kind, text, *sites):
        if len(state.events) >= 24:
            return replace(state, exit="open")
        event = Event(kind, text, tuple(site for site in sites if site), len(frame.parents), frame.parents, state.conditions)
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
        if kind == "try" and self.lookup_only:
            body, _, _, resources = data
            if not resources:
                entry = Site(frame.owner.path, node[1], node[1] + len(b"try"))
                premise = Condition("control enters the cited try body", (entry,), lookup_scope=True)
                entered = replace(state, conditions=(*state.conditions, premise))
                return [replace(row, exit="open") for row in self.statement(frame, entered, body)]
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
                if kind == "throw" and frame.parents and not row.exit:
                    row = self.event(frame, row, "throw", f"Executes {self.code(frame, node)} in `{frame.owner.name}.{frame.method[3]}`.",
                                     frame.site(node), value.site)
                if kind == "return" and frame.parents and value.constructed and not row.exit:
                    name = f"`{frame.owner.name}.{frame.method[3]}`"
                    row = self.event(frame, row, "constructed-result", f"{name} returns the value constructed by {value.label}.",
                                     frame.site(node), value.site)
                rows.append(row if row.exit else replace(row, exit=kind, result=value))
            return rows
        if kind == "expr":
            return [row for row, _ in self.evaluate(frame, state, data[0])]
        if kind == "empty":
            return [state]
        return [replace(state, exit="open")]

    def branch(self, frame, state, node):
        kind, data = node[0], node[3:]
        if kind == "not":
            return [(row, not truth) for row, truth in self.branch(frame, state, data[0])]
        if kind in {"and", "or"}:
            return [(branch, truth) for row, left in self.branch(frame, state, data[0])
                    for branch, truth in (self.branch(frame, row, data[1])
                                          if left == (kind == "and") else [(row, left)])][:MAX_STATES]
        result = []
        for row, value in self.evaluate(frame, state, node):
            if row.exit:
                result.append((row, True))
                continue
            possible = (value.truth,) if value.truth is not None else (True, False)
            for truth in possible:
                condition = Condition(f"{self.code(frame, node)} is {'true' if truth else 'false'}", (frame.site(node),))
                optionals = row.optionals
                if value.test and value.truth is None:
                    address, present_when = value.test
                    optionals = {**optionals, address: "present" if truth == (present_when == "present") else "empty"}
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
                return [(state, state.bindings[name])]
            return [(state, self.field_value(frame, frame.this, name, literal))]
        if kind == "literal":
            tag = data[0]
            proof = self.publications.literal(frame.owner, node) if self.publications else None
            value = replace(literal, publication_operand=proof, truth=(tag == "TRUE") if tag in {"TRUE", "FALSE"} else None,
                            nullable=(tag == "NULL"), type={"TRUE": "boolean", "FALSE": "boolean",
                                                           "EMPTY_STRING": "java.lang.String", "NONEMPTY_STRING": "java.lang.String"}.get(tag, ""))
            return [(state, value)]
        if kind == "member":
            if self.publications and (proof := self.publications.class_operand(frame.owner, node, frame.method[12])):
                return [(state, replace(literal, publication_operand=proof))]
            return [(row, self.field_value(frame, receiver, data[1], literal))
                    for row, receiver in self.evaluate(frame, state, data[0])]
        if kind == "choose":
            return [(row, value) for branch, truth in self.branch(frame, state, data[0])
                    for row, value in self.evaluate(frame, branch, data[1] if truth else data[2])][:MAX_STATES]
        if kind in {"eq", "neq", "ordered", "array"}:
            return [(row, self.comparison(kind, literal, args)) for row, args in self.arguments(frame, state, data)]
        if kind in {"and", "or", "not"}:
            return [(row, replace(literal, truth=truth)) for row, truth in self.branch(frame, state, node)]
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
        if kind in {"eq", "neq"}:
            left, right = values
            if True in (left.nullable, right.nullable) and None not in (left.nullable, right.nullable):
                equal = left.nullable == right.nullable
                return replace(literal, truth=equal == (kind == "eq"))
            if None not in (left.truth, right.truth):
                return replace(literal, truth=(left.truth == right.truth) == (kind == "eq"))
        return literal

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
        return replace(literal, type=declared, exact=exact, site=address, provenance=proof)

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
        local = Value(self.code(frame, node), site)
        if receiver.nullable is True:
            return [(replace(state, exit="open"), UNKNOWN)]
        if receiver.type == OPTIONAL:
            known = self.optional(frame, state, node, receiver, name, is_static, values, local)
            if known is not None:
                return known
        compiler_target = self.compiler_targets.get(site)
        if self.publications:
            operands = tuple(value.publication_operand for value in values)
            if name in {"publish", "publishById"}:
                self.publications.submit(frame, state, node, receiver, name, operands, compiler_target)
            if name in {"of", "singletonList", "getName", "getEvents"}:
                projected = self.publications.call_operand(frame, state, node, receiver.publication_operand, name, operands, compiler_target)
                if projected:
                    local = replace(local, publication_operand=projected)
        compiler_owner, compiler_method = self.units.compiler_method(frame.owner, compiler_target)
        linked_owner, linked_method, fixed = self.linked_method(site, compiler_target, name)
        owner = compiler_owner or linked_owner or self.units.lookup(frame.owner, receiver.type)
        if owner and owner.record and not values and receiver.components and name not in {m[3] for m in owner.methods()}:
            if (component := dict(receiver.components).get(name)) is not None:
                return [(state, component)]
        candidate = self.match(owner, name, len(values)) if owner else None
        overload = (self.exact_overload(frame, owner, name, values)
                    if self.lookup_only and owner and not candidate and not compiler_target and not linked_method else None)
        if compiler_target and owner and not compiler_target.startswith(f"method:class:{owner.qualified}#{name}("):
            candidate = None
        method = compiler_method or linked_method or candidate or overload
        if method and is_static and "STATIC" not in method[5]:
            method = None
        if method and len(values) != len(method[6]):
            method = None
        code = local.label
        mappings = [(self.code(frame, arg), value.label, value.site) for arg, value in zip(args, values)
                    if value.site and value.label and value.label not in {self.code(frame, arg), "the cited expression"}]
        if mappings:
            code += " (" + ", ".join(f"{argument} receives {origin}" for argument, origin, _ in mappings[:3]) + ")"
        origins = tuple(origin for _, _, origin in mappings)
        provenance = tuple(anchor for value in values if value.entries for anchor in value.provenance)
        if not method or method[8] is None or len(frame.parents) >= MAX_DEPTH:
            declared = method is not None and (method is compiler_method or (method is linked_method and fixed))
            source_contract = method is not None and not declared and self.compatible_contract(frame, owner, method, receiver, values)
            declaration_only = source_contract and bool(owner.node[11])
            return_type = self.units.qualify(owner, method[4]) if declared or (source_contract and not declaration_only) else ""
            contract = None
            if not method and not is_static and name == "findById" and len(values) == 1:
                if compiler_target in CRUD_SIGNATURES:
                    contract = ()
                elif not compiler_target and (inherited := self.units.inherited_lookup(receiver.type, owner)) and inherited[0]:
                    contract = inherited[1]
            if contract is not None:
                return_type = OPTIONAL
                assumption = Condition(f"{local.label} returns an Optional under `CrudRepository.findById`",
                                       tuple(anchor for anchor in (site, receiver.site, *receiver.provenance, *contract) if anchor))
                state = replace(state, conditions=(*state.conditions, assumption))
            state = self.event(frame, state, "attempt", code, site, *origins, *provenance)
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
            return [(state, replace(local, type=return_type, identity=site if return_type == OPTIONAL else None,
                                    provenance=contract or ()))]
        selected = method is compiler_method or (method is linked_method and fixed)
        dispatch_type = owner.qualified if (owner.record or "FINAL" in owner.node[5]
                      or {"STATIC", "PRIVATE", "FINAL"}.intersection(method[5])) else receiver.exact
        premise = None
        if not (selected and dispatch_type == owner.qualified):
            premise = Condition(f"{local.label} resolves to `{owner.name}.{method[3]}` and enters its body",
                                (site, declaration_anchor(owner, method)))
        if premise:
            state = replace(state, conditions=(*state.conditions, premise))
        state = self.event(frame, state, "selected-call", code, site, declaration_anchor(owner, method), receiver.site, *origins, *provenance)
        if self.lookup_only and (method is overload or (selected and not candidate)):
            state = self.lookup_witness(frame, state, owner, method, local, args, values, receiver)
            # The witness describes the callee's return relation under its own
            # premises. It does not choose the actual return value for this call.
            return [(state, replace(local, type=self.units.qualify(owner, method[4])))]
        state = self.annotations(frame, state, owner, method, local, declaration_anchor(owner, method))
        child = Frame(owner, method, replace(receiver, type=owner.qualified), (*frame.parents, site))
        bindings = self.bind_arguments(owner, method, values)
        nested = replace(state, bindings=bindings, initialized={})
        rows = self.statement(child, nested, method[8])
        return_type = self.units.qualify(owner, method[4])
        return [(replace(row, bindings=state.bindings, initialized=state.initialized, exit=None if row.exit == "return" else row.exit),
                 replace(row.result, type=return_type or row.result.type) if row.exit == "return" else replace(local, type=return_type))
                for row in rows]

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
        proof = (self.publications.construct(frame, node, type_name, tuple(value.publication_operand for value in values),
                                             self.compiler_targets.get(site)) if self.publications else None)
        type_name = self.qualify(frame, type_name)
        value = Value(self.code(frame, node), site, type_name, type_name, nullable=False, constructed=True, arguments=values,
                      publication_operand=proof)
        state = self.shared_construction(frame, state, value, values)
        owner = self.units.lookup(frame.owner, type_name)
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
        constructor = Frame(owner, method, value, (*frame.parents, site))
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


def compact_sites(sites):
    combined = []
    for site in sorted(set(sites), key=lambda row: (row.path, row.start, row.end)):
        if combined and combined[-1].path == site.path:
            earlier = combined[-1]
            if site.end <= earlier.end:
                continue
            if site.start - earlier.end <= 45 and site.end - earlier.start <= 500:
                combined[-1] = Site(site.path, earlier.start, site.end)
                continue
        combined.append(site)
    return combined


def cite_fact(snapshot, event, root_id, prefix, text, kind, remaining):
    condition = " and ".join(dict.fromkeys(row.text for row in event.conditions))
    if len(condition) > 750:
        return None
    required = [*event.cites, *event.parents, *prefix,
                *(site for item in event.conditions for site in item.cites)]
    summary = {"authority": "DECLARED_SOURCE_CONTRACT" if event.kind in {"contract", "lookup-contract"} else AUTHORITY,
               "kind": kind, "text": text, **({"condition": condition} if condition else {})}
    spans = compact_sites(required) if hasattr(snapshot, "cite_many") else list(dict.fromkeys(required))
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
                    "constructed-argument": 7, "contract": 7, "annotation": 4,
                    "constructor-write": 8, "type-annotation": 9}
            ranked.append((rank[event.kind], event, tuple(site for call in prefix for site in call.cites), text))
    # Give distinct evidence roles a turn before spending the rest of the
    # shared source budget on repeated branches or constructor writes.
    buckets = {}
    for row in sorted(ranked, key=lambda item: item[0]):
        level, event, _, _ = row
        buckets.setdefault((level, event.kind), []).append(row)
    first = [rows[0] for rows in buckets.values()]
    more = [row for rows in buckets.values() for row in rows[1:]]
    size = 0
    result = []
    covered = set()
    for _, event, prefix, text in (*first, *more):
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


def project_frame(units, owner, method, identifier, entry, selected_bodies, compiler_targets, *, publication=True):
    selected_publication = any(PUBLICATIONS["member_identity"](target)[0] in (
        PUBLICATIONS["DIRECT"], PUBLICATIONS["AGGREGATE"], PUBLICATIONS["PORT"]) for target in compiler_targets.values())
    enabled = publication and (selected_publication or re.search(rb"\b(?:publish|publishById)\s*\(",
                                                                 units.snapshot.read(owner.path) or b""))
    publications = PUBLICATIONS["Publications"](units) if enabled else None
    projection = Projection(units, selected_bodies, compiler_targets, publications=publications)
    frame, bindings = entry_frame(units, owner, method)
    initial = State(bindings=bindings)
    states = projection.statement(frame, initial, method[8])
    submissions = publications.facts(units.snapshot, identifier) if publications else []
    original = facts(units.snapshot, states, identifier)
    lookups = Projection(units, selected_bodies, compiler_targets, lookup_only=True)
    events = lookup_events(lookups, frame, initial, entry)
    return [*submissions, *original, *lookup_facts(units.snapshot, events, identifier)]


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
            return project_frame(units, owner, method, identifier, entry, selected_bodies, unique_targets)
    except (OSError, ValueError, RecursionError, subprocess.SubprocessError):
        return []
