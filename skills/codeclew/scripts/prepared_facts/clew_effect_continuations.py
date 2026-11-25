"""Bounded effect traces over a retained compiler control-flow projection."""

from __future__ import annotations

from collections import defaultdict, deque
from dataclasses import dataclass, field, replace
from hashlib import sha256
import json
import re


SCHEMA = "clew-reader-effect-continuations/1"
MAX_FILE_BYTES = 128 * 1024
MAX_FILES = 12
MAX_METHODS = 20
MAX_PATHS = 40
MAX_VISITS = 720
MAX_COVERAGE_PATHS = 18
MAX_IMPLICIT_INTERRUPTS = 2
HTTP_SUMMARY_VISITS = 1200
METHOD_SUMMARY_VISITS = 160
METHOD_NORMAL_RETURNS = 8
MAX_CALL_DEPTH = 5
MAX_EVENTS = 60
NULL_DISPATCH = "NULL_ACCEPTING_NON_INSTANCE_DISPATCH_REQUIRED"
LIST_SELECTION_OPEN = "LIST_SELECTION_CARDINALITY_OR_BINDING_OPEN"
SDK_IMPORTS = "ASSUMED_SDK_GLOBAL_IMPORTS"
NOMINAL_FRAMEWORK = "CONDITIONAL_NOMINAL_FRAMEWORK_MEMBER"
MAX_TRANSFERS = 120
MAX_HELPER_MODELS = 8
MAX_HELPER_BLOCKS = 32
MAX_SKIPPED = 12
MAX_COMPOSED_EFFECT_SITES = 120
LOCAL_CALLABLES = MAX_HELPER_MODELS + 1
LOCAL_PATHS = 6
LOCAL_VISITS = 240
LOCAL_OUTPUT_PATHS = 24
TRANSACTION_METHODS = {"BeginTransaction", "BeginTransactionAsync", "Commit", "CommitAsync", "Rollback", "RollbackAsync"}
EFFECTS = {"CALL", "AWAIT", "WRITE", "CONSTRUCT"}
NATIVE_TYPE = r"\[*(?:[ZBSCIJFD]|L[A-Za-z0-9_/$`]+;)"
NATIVE_TOKEN = "LSystem/Threading/CancellationToken;"


def digest(data):
    return "sha256:" + sha256(data).hexdigest()


def eligible(process, evidence):
    """Use callable anchors from the sealed operation, excluding peer ranges."""
    result = {}
    root = process.get("root")
    for fact in process.get("facts", ()):
        if (fact.get("kind") == "BODY" or fact.get("event") == "WORKER_HELPER_SOURCE" or
                fact.get("kind") == "BODY_SOURCE" and fact.get("authority") in {
                    "COMPILER_EXACT_CALLBACK_TARGET", "SAME_OWNER_CANDIDATE"}):
            for ref in fact.get("sources", ()):
                row = evidence.get(ref, {})
                if row.get("file", "").endswith(".cs") and not row.get("peerService") and not row.get("peerProcess"):
                    result[ref] = (fact.get("declaration"), root is not None and fact.get("declaration") == root)
    return result


def locate(blob, source):
    """Match the exact sealed UTF-8 bytes on the sealed lines, with unique placement."""
    needle = source["text"].encode()
    if not needle or blob.count(b"\n") < source["startLine"] - 1:
        return None
    lines = blob.split(b"\n")
    first, last = source["startLine"], source["endLine"]
    if not 1 <= first <= last <= len(lines):
        return None
    start = sum(len(line) + 1 for line in lines[:first - 1])
    end = sum(len(line) + 1 for line in lines[:last])
    options = []
    index = blob.find(needle, start, end)
    while index != -1 and index < start + len(lines[first - 1]) + 1:
        limit = index + len(needle)
        if blob[:limit - 1].count(b"\n") + 1 == last:
            options.append((index, limit))
        index = blob.find(needle, index + 1, end)
    return options[0] if len(options) == 1 else None


def derive(process, evidence, grouped, repo, project, load_source, storage, sql_tables, commands, *, site_texts=None, supporting=None):
    selections = eligible(process, evidence)
    cs = any(row["file"].endswith(".cs") and not row.get("peerService") for row in evidence.values())
    if not cs:
        return None
    if not selections or not any(is_root for _, is_root in selections.values()):
        return report("OPEN", ["RETAINED_ROOT_BODY_UNAVAILABLE"])
    if repo is None:
        return report("OPEN", ["PINNED_CSHARP_REPOSITORY_REQUIRED"])

    refs = {ref: name for name, source in grouped.items() for ref in source["references"]}
    roots_first = sorted(selections, key=lambda ref: (not selections[ref][1], refs[ref], ref))
    revisions = {evidence[ref]["revision"] for ref in roots_first if selections[ref][1]}
    if len(revisions) != 1:
        return report("OPEN", ["RETAINED_ROOT_REVISION_AMBIGUOUS"])
    revision = next(iter(revisions))
    raw, files, requested, anchors, gaps = {}, {}, {}, {}, set()
    for ref in roots_first:
        source = evidence[ref]
        file = source["file"]
        if source["revision"] != revision:
            gaps.add("OTHER_REVISION_NOT_COMPOSED")
            continue
        if file not in raw:
            if len(raw) >= MAX_FILES:
                gaps.add("COMPILATION_FILE_BUDGET")
                continue
            text = load_source(repo, source)
            raw[file] = text.encode() if text is not None else b""
            if 0 < len(raw[file]) <= MAX_FILE_BYTES:
                files[file] = text
        if file not in files:
            gaps.add("COMPILATION_SOURCE_BUDGET")
            continue
        pair = locate(raw[file], source)
        if pair is None:
            raise ValueError("The pinned Git blob differs from the sealed C# callable bytes.")
        key = (file, *pair)
        previous = requested.get(key)
        if previous is not None:
            if selections[ref][0] != selections[previous["reference"]][0]:
                return report("OPEN", ["SEALED_CALLABLE_IDENTITY_AMBIGUOUS"])
            continue
        if len(requested) >= MAX_METHODS:
            gaps.add("RETAINED_CALLABLE_BUDGET")
            continue
        requested[key] = {"reference": ref, "file": file, "start": pair[0], "end": pair[1]}
        anchors[ref] = (refs[ref], source, pair)
    if not requested:
        return report("OPEN", sorted(gaps | {"RETAINED_CALLABLE_SOURCE_UNAVAILABLE"}))
    is_http = process.get("entryKind") in {"HTTP_ENDPOINT", "SOURCE_HTTP_MAPPING"}
    allow_sdk_imports = is_http or process.get("entryKind") == "HOSTED_BACKGROUND_WORKER"
    result = project({"files": {**(supporting or {}), **files}, "selections": list(requested.values()),
                      **({"allowSdkImports": True} if allow_sdk_imports else {})})
    if result is None:
        return report("OPEN", sorted(gaps | {"ROSLYN_PROJECTOR_UNAVAILABLE"}))
    functions = {}
    for ref, graph in result.items():
        if ref in anchors and graph and graph.get("schema") == "clew-effect-cfg/1":
            citation, source, interval = anchors[ref]
            functions[ref] = Function(graph, citation, source, raw[source["file"]], interval)
    roots = [ref for ref in functions if selections[ref][1]]
    if len(roots) != 1:
        return report("OPEN", sorted(gaps | {"ROSLYN_ROOT_CFG_UNAVAILABLE_OR_AMBIGUOUS"}))
    if len(functions) != len(requested):
        gaps.add("SOME_RETAINED_CALLABLE_CFGS_UNAVAILABLE")
    roles = storage["source_roles"](process, evidence, refs, functions, locate, sql_tables)
    root = functions[roots[0]]
    signature = process.get("entrySymbol")
    tokens = sealed_token_parameters(root, signature)
    hosts = hosted_methods(process, functions, roots[0])
    projection = Trace(functions, gaps, roles, storage, sql_tables, commands).trace(roots[0], signature, tokens, hosts, is_http)
    if site_texts is not None:
        predicates = {f"{fn.citation}/if{block['id']}" for fn in functions.values() for block in fn.graph["blocks"]
                      if (decision := block["decision"]) and decision.get("sourcePredicate")}
        for step in projection.get("steps", ()):
            if step["kind"] == "CONDITION" and step["id"] not in predicates:
                continue
            site = step["site"]
            # Trace sites are validated against the pinned UTF-8 bytes by Function.site.
            site_texts[step["id"]] = raw[site["file"]][site["startByte"]:site["endByte"]].decode()
    return projection


def hosted_methods(process, functions, root):
    if process.get("entryKind") != "HOSTED_BACKGROUND_WORKER":
        return ()
    trigger = process.get("trigger") or {}
    if trigger.get("authority") not in {"COMPILER_REGISTRATION_AND_OVERRIDE", "TOP_LEVEL_FRAMEWORK_SOURCE_AND_COMPILER_OVERRIDE"}:
        return (root,)
    signature = process.get("entrySymbol") or ""
    name = trigger.get("name")
    return tuple(ref for ref, fn in functions.items() if ref == root or
                 signature.startswith(fn.graph["nativeOwner"] + "(") or
                 name == fn.graph["symbol"].split("(", 1)[0].removeprefix("M:"))


def sealed_token_parameters(root, signature):
    """Recover only by-value token slots from a corroborated native root identity."""
    graph = root.graph
    owner = graph["nativeOwner"]
    if not isinstance(signature, str) or len(signature) > 512 or not signature.startswith(owner + "("):
        return {}
    site = graph["site"]
    if not root.interval[0] <= site["start"] < site["end"] <= root.interval[1]:
        return {}
    matched = re.fullmatch(rf"\(((?:{NATIVE_TYPE})*)\)(V|{NATIVE_TYPE})", signature[len(owner):])
    if not matched:
        return {}
    descriptors = re.findall(NATIVE_TYPE, matched[1])
    parameters = graph["parameters"]
    if (len(descriptors) != len(parameters) or graph["returnDescriptor"] not in {None, matched[2]}
            or any(parameter["descriptor"] not in {None, kind} for parameter, kind in zip(parameters, descriptors))):
        return {}
    return {parameter["id"]: {"authority": "SEALED_NATIVE_METHOD_PARAMETER", "parameter": parameter["id"],
                              "type": "System.Threading.CancellationToken", "resolution": "NOMINAL_NATIVE_DESCRIPTOR",
                              "sources": [root.citation]}
            for parameter, kind in zip(parameters, descriptors)
            if kind == NATIVE_TOKEN and parameter["unboundToken"]}


def report(status, boundaries, **contents):
    return {
        "schema": SCHEMA, "authority": "ROSLYN_PINNED_SOURCE_PROJECTION",
        "status": status, "boundaries": boundaries,
        "execution": "CONDITIONAL_SOURCE_PATHS",
        "deployment": "OPEN",
        **contents,
    }


@dataclass
class Function:
    graph: dict
    citation: str
    source: dict
    blob: bytes
    interval: tuple

    def site(self, raw):
        start, end = raw["start"], raw["end"]
        if not self.interval[0] <= start < end <= self.interval[1]:
            raise ValueError("A Roslyn CFG site lies outside its sealed callable bytes.")
        text = self.blob[start:end]
        first = self.blob[:start].count(b"\n") + 1
        last = self.blob[:end - 1].count(b"\n") + 1
        if (first, last) != (raw["first"], raw["last"]):
            raise ValueError("Roslyn CFG lines differ from the sealed callable bytes.")
        return {"file": self.source["file"], "revision": self.source["revision"],
                "startByte": start, "endByte": end, "startLine": first, "endLine": last,
                "textDigest": digest(text)}


@dataclass(frozen=True)
class State:
    sequence: tuple = ()
    conditions: tuple = ()
    writes: tuple = ()
    memory: dict = field(default_factory=dict)
    fresh_lists: dict = field(default_factory=dict)
    projections: dict = field(default_factory=dict)
    data_fields: dict = field(default_factory=dict)
    event_base: int = 0
    uncertainties: tuple = ()
    transfers: int = 0
    modeled_faults: int = 0
    nonnull: frozenset = frozenset()
    null: frozenset = frozenset()


@dataclass(frozen=True)
class Frame:
    owner: tuple | None
    parameters: dict = field(default_factory=dict)
    temporaries: dict = field(default_factory=dict)
    cancellation_parameters: dict = field(default_factory=dict)
    selector_item: tuple | None = None


@dataclass(frozen=True)
class Exit:
    state: State
    kind: str
    value: tuple | None = None
    exception_types: tuple = ()


@dataclass(frozen=True)
class EntrySeed:
    state: State
    frame: Frame
    call: str
    authority: str


def literal(value):
    return "literal", value


def symbol(value, frame):
    """Preserve object identity and selector provenance through local aliases."""
    if value is None:
        return None
    kind = value["kind"]
    if kind == "literal":
        return literal(value["value"])
    if kind == "lexical":
        return "unbound-name", value["name"]
    if kind == "lexical-member":
        return "unbound-member", value["name"], symbol(value["owner"], frame)
    if kind == "instance":
        return frame.owner
    if kind == "tuple":
        return "tuple", tuple(symbol(item, frame) for item in value["items"])
    if kind == "record":
        return "record", tuple((item["name"], symbol(item["value"], frame)) for item in value["fields"])
    if kind == "tuple-slot":
        owner = symbol(value["owner"], frame)
        if owner and owner[0] == "tuple" and 0 <= value["index"] < len(owner[1]):
            return owner[1][value["index"]]
        return None
    if kind == "json-root":
        owner = symbol(value["owner"], frame)
        return (kind, owner) if owner is not None else None
    if kind == "json-key":
        owner, key = symbol(value["owner"], frame), symbol(value["key"], frame)
        return (kind, owner, key) if owner is not None and key is not None else None
    if kind == "record-member":
        owner = symbol(value["owner"], frame)
        if owner and owner[0] == "record":
            return dict(owner[1]).get(value["name"])
        return None
    if kind == "parameter":
        return frame.parameters.get(value["id"])
    if kind == "selector-item":
        return frame.selector_item
    if kind == "captured-parameter":
        if value["scope"] and frame.owner is not None:
            return "captured-parameter", value["scope"], value["id"], value["name"], frame.owner
        return None
    if kind == "ado-member":
        owner = symbol(value["owner"], frame)
        return ("ado-member", value["name"], owner, value["typed"]) if owner is not None else None
    if kind == "object-initializer-member":
        owner = symbol(value["owner"], frame)
        return (kind, value["name"], owner) if owner is not None else None
    if kind == "ado-index":
        owner, selector = symbol(value["owner"], frame), symbol(value["selector"], frame)
        return ("ado-index", owner, selector, value["typed"]) if owner is not None and selector is not None else None
    if kind in {"local", "capture", "result"}:
        return frame.temporaries.get((kind, value["id"]))
    if kind == "member" and value["id"]:
        if not value["direct"]:
            return ("open-member", value["id"], symbol(value["owner"], frame))
        owner = ("static",) if value["shared"] else symbol(value["owner"], frame)
        return ("member", value["id"], owner) if owner is not None else None
    if kind == "not":
        operand = symbol(value["value"], frame)
        return ("not", operand) if operand is not None else None
    if kind in {"equal", "different"}:
        left, right = symbol(value["left"], frame), symbol(value["right"], frame)
        return (kind, left, right) if left is not None and right is not None else None
    return None


def data_value(value, fields, depth=0):
    if not value or depth >= 8:
        return value
    kind, *items = value
    if kind == "member":
        identity, owner = items
        owner = data_value(owner, fields, depth + 1)
        if owner and owner[0] == "record":
            return dict(owner[1]).get(identity.rsplit(".", 1)[-1])
        member = (kind, identity, owner)
        return data_value(fields[member], fields, depth + 1) if member in fields else member
    if kind in {"not", "equal", "different"}:
        children = tuple(data_value(item, fields, depth + 1) for item in items)
        return (kind, *children) if all(child is not None for child in children) else None
    if kind == "record":
        return kind, tuple((name, data_value(item, fields, depth + 1)) for name, item in items[0])
    return value


def selector_term(value):
    if not value or value[0] in {"open-member", "unbound-member", "unbound-name", "captured-parameter"}:
        return False
    if value[0] in {"not", "equal", "different"}:
        return all(selector_term(item) for item in value[1:])
    if value[0] == "record":
        return all(selector_term(item) for _, item in value[1])
    if value[0] == "tuple":
        return all(selector_term(item) for item in value[1])
    if value[0] == "member":
        return selector_term(value[2])
    return True


def collection_items(source, state, depth=0):
    if depth >= 4:
        return None
    if source in state.fresh_lists:
        return state.fresh_lists[source]
    if source in state.projections:
        original, selector, frame = state.projections[source]
        items = collection_items(original, state, depth + 1)
        if items is not None:
            projected = tuple(data_value(symbol(selector, replace(frame, selector_item=item)), state.data_fields)
                              for item in items)
            return projected if all(selector_term(item) for item in projected) else None
    return None


def observed(value, memory):
    if value is None:
        return None
    kind, *items = value
    if kind == "literal":
        return value
    if kind == "member":
        return memory.get(value)
    if kind == "not":
        operand = observed(items[0], memory)
        return literal(not operand[1]) if operand and type(operand[1]) is bool else None
    if kind in {"equal", "different"}:
        if (items[0] == literal(None) and items[1][0] in {"tuple", "record"} or
                items[1] == literal(None) and items[0][0] in {"tuple", "record"}):
            return literal(kind == "different")
        left, right = (observed(item, memory) for item in items)
        if left is not None and right is not None:
            same = type(left[1]) is type(right[1]) and left == right
            return literal(same == (kind == "equal"))
    return None


def escapes(value, resource):
    if value == resource:
        return True
    if value and value[0] == "tuple":
        return any(escapes(item, resource) for item in value[1])
    if value and value[0] == "record":
        return any(escapes(item, resource) for _, item in value[1])
    return False


def forget_lists(state, values):
    if not values:
        return state
    return replace(state, fresh_lists={resource: items for resource, items in state.fresh_lists.items()
                                      if not any(escapes(value, resource) for value in values)},
                   data_fields={member: item for member, item in state.data_fields.items()
                                if not any(escapes(value, member[2]) for value in values)})


def eligible_guard(value, truth):
    if not value:
        return None
    if value[0] == "not":
        return eligible_guard(value[1], not truth)
    if value[0] == "member" and value[2][0] in {"parameter", "entry-receiver", "static"}:
        return value, truth
    if value[0] in {"equal", "different"}:
        left, right = value[1:]
        if left[0] == "literal":
            left, right = right, left
        if right[0] == "literal" and type(right[1]) is bool:
            return eligible_guard(left, right[1] == (truth == (value[0] == "equal")))
    return None


def stable_reference(value):
    return value and value[0] in {"allocation", "call-result", "entry-receiver"}


def null_test(value, truth):
    if not value:
        return None, False
    if value[0] == "not":
        return null_test(value[1], not truth)
    if value[0] in {"equal", "different"}:
        left, right = value[1:]
        if left == literal(None):
            left, right = right, left
        if right == literal(None) and stable_reference(left):
            return left, truth == (value[0] == "equal")
    return None, False


class Trace:
    def __init__(self, functions, boundaries, roles, storage, sql_tables, commands):
        self.functions = functions
        self.roles = roles
        self.storage = storage
        self.sql_tables = sql_tables
        self.commands = commands
        occurrences = defaultdict(list)
        for ref, fn in functions.items():
            occurrences[(fn.source["file"], fn.graph["declarationStart"], fn.graph["symbol"])].append(ref)
        self.bindings = {key: refs[0] for key, refs in occurrences.items() if len(refs) == 1}
        self.boundaries = boundaries
        self.steps = {}
        self.visits = 0
        self.coverage = False
        self.compose_candidates = True
        self.local_only = False
        self.entry_seeds = {}
        self.visit_limit = MAX_VISITS
        self.focal = False
        self.focus_call = None
        self.path_limit = MAX_PATHS
        self.summary_queue = None
        self.summary_visits = 0

    def step(self, fn, identity, kind, site, **metadata):
        key = f"{fn.citation}/{identity}"
        if key not in self.steps:
            self.steps[key] = {"id": key, "kind": kind, "site": fn.site(site), "sources": [fn.citation], **metadata}
        else:
            for name, value in metadata.items():
                self.steps[key].setdefault(name, value)
        return key

    @staticmethod
    def record(state, key, outcome, **details):
        return replace(state, sequence=(*state.sequence, {"step": key, "outcome": outcome, **details}))

    def open(self, state, reason):
        self.boundaries.add(reason)
        return Exit(replace(state, uncertainties=(*state.uncertainties, reason)), "OPEN")

    def enter(self, ref, state, frame, stack):
        if ref in stack or len(stack) >= MAX_CALL_DEPTH:
            return [self.open(state, "CALL_CYCLE_OR_DEPTH")]
        if self.functions[ref].graph.get("sdkImports"):
            self.boundaries.add(SDK_IMPORTS)
            if SDK_IMPORTS not in state.uncertainties:
                state = replace(state, uncertainties=(*state.uncertainties, SDK_IMPORTS))
        return self.block(ref, 0, 0, state, frame, (*stack, ref), (), {})

    def normal_returns(self, ref, state, frame, stack):
        """Instantiate bounded normal returns in a caller's symbolic frame.

        Each method advances its CFG independently of the caller's alternative
        and exception-path quota. Resume the caller only from a whole normal
        exit, carrying ordered resource events, values, and branch premises.
        """
        continuation = self.summary_queue
        self.summary_queue = deque()
        returns, seen, visits = [], set(), 0
        try:
            self.enter(ref, state, frame, stack)
            while self.summary_queue and visits < METHOD_SUMMARY_VISITS and self.summary_visits < HTTP_SUMMARY_VISITS:
                position = self.summary_queue.popleft()
                visits += 1
                self.summary_visits += 1
                for outcome in self.advance_block(*position):
                    if outcome.kind not in {"NORMAL_EXIT", "RETURNED"}:
                        continue
                    key = self.summary_identity(state, outcome)
                    if key not in seen:
                        seen.add(key)
                        returns.append(outcome)
                if len(returns) >= METHOD_NORMAL_RETURNS:
                    break
            if self.summary_queue:
                self.boundaries.add("NORMAL_RETURN_SUMMARY_BUDGET")
        finally:
            self.summary_queue = continuation
        return returns[:METHOD_NORMAL_RETURNS]

    def summary_identity(self, start, result):
        state = result.state
        effects = [(event.get("call") or event["step"], event["outcome"], event.get("receiver"))
                   for event in state.sequence[len(start.sequence):]
                   if (step := self.steps.get(event.get("call") or event["step"], {})).get("spelling") in TRANSACTION_METHODS
                   or self.commands["completion_boundary"](step.get("role"), step.get("spelling"), step.get("dispatch"))]
        return json.dumps([result.kind, result.value, state.conditions[len(start.conditions):], effects, state.uncertainties],
                          sort_keys=True)

    def block(self, ref, index, offset, state, frame, stack, pending, visits):
        if self.summary_queue is not None:
            self.summary_queue.append((ref, index, offset, state, frame, stack, pending, visits))
            return []
        return self.advance_block(ref, index, offset, state, frame, stack, pending, visits)

    def advance_block(self, ref, index, offset, state, frame, stack, pending, visits):
        self.visits += 1
        if self.summary_queue is None and (
                self.visits > self.visit_limit or len(state.sequence) - state.event_base >= MAX_EVENTS or
                state.transfers >= MAX_TRANSFERS):
            return [self.open(state, "TRACE_BUDGET")]
        state = replace(state, transfers=state.transfers + 1)
        fn = self.functions[ref]
        block = fn.graph["blocks"][index]
        if not block["reachable"]:
            return [self.open(state, "COMPILER_BLOCK_UNREACHABLE")]
        if offset == 0:
            if visits.get(index, 0) >= 2:
                return [self.open(state, "LOOP_CONTINUATION_BUDGET")]
            visits = {**visits, index: visits.get(index, 0) + 1}
        if block["kind"] == "Exit":
            return [Exit(state, "NORMAL_EXIT")]
        if offset == len(block["steps"]):
            return self.edge(ref, block, state, frame, stack, pending, visits)
        return self.operation(ref, block, offset, state, frame, stack, pending, visits)

    def bounded(self, exits):
        result = []
        for batch in exits:
            result.extend(batch)
            if len(result) >= self.path_limit:
                self.boundaries.add("PATH_ENUMERATION_BUDGET")
                return result[:self.path_limit]
        return result

    def completions(self, state, normal, exceptional, priority=False):
        if self.coverage or self.focal and (not priority or state.modeled_faults) or state.modeled_faults >= MAX_IMPLICIT_INTERRUPTS:
            return normal()
        fault = replace(state, modeled_faults=state.modeled_faults + 1)
        return self.bounded(action() for action in (normal, lambda: exceptional(fault)))

    def resolve(self, operation):
        binding = operation.get("binding") or operation.get("candidate")
        if not binding or any(arg["byRef"] for arg in operation["arguments"]):
            return None, None
        target = self.bindings.get((binding["file"], binding["start"], binding["symbol"]))
        if target:
            authority = "ROSLYN_STATIC_BINDING" if operation.get("binding") else "CONDITIONAL_SOURCE_CANDIDATE"
            return target, authority
        return None, None

    def operation(self, ref, block, offset, state, frame, stack, pending, visits):
        fn = self.functions[ref]
        operation = block["steps"][offset]
        kind = operation["kind"]
        identity = operation["id"]

        def advance(work, current=frame, skip=1):
            return self.block(ref, block["id"], offset + skip, work, current, stack, pending, visits)

        def fault(work, types=()):
            return self.exception(ref, block, work, frame, stack, pending, visits, types)

        def bind(target, value, current=frame):
            return replace(current, temporaries={**current.temporaries, target: value})

        def key(**details):
            return self.step(fn, f"op{identity}", kind, operation["site"], **details)

        if kind == "BIND":
            value = symbol(operation["value"], frame)
            return advance(state, bind(("capture", operation["capture"]), value))
        if kind == "ACCESS":
            value = symbol(operation["value"], frame)
            step = key()
            result = ("access-result", step, len(state.sequence))
            accessed = self.record(state, step, "INDEX_VALUE_REACHED", result=result, value=value)
            return advance(accessed, bind(("result", identity), result))
        if kind == "WRITE":
            target, value = operation["target"], symbol(operation["value"], frame)
            if target and target["kind"] in {"local", "parameter"}:
                if target["kind"] == "parameter":
                    current = replace(frame, parameters={**frame.parameters, target["id"]: value})
                else:
                    current = bind(("local", target["id"]), value)
                return advance(self.record(state, key(), "LOCAL_BINDING", value=value,
                                           **({"name": target["name"]} if target.get("name") else {})), current)
            resource = symbol(target, frame)
            state = forget_lists(state, (value,))
            if resource and resource[0] in {"ado-member", "ado-index"}:
                configured = self.record(state, key(), "RESOURCE_CONFIGURATION_REACHED", resource=resource, value=value)
                return advance(configured)
            if resource and resource[0] == "object-initializer-member":
                configured = self.record(state, key(), "CANDIDATE_INITIALIZER_REACHED", resource=resource, value=value)
                return advance(configured)
            if resource and resource[0] == "member":
                member = operation.get("initializerMember")
                step = key(**({"initializerMember": member} if member else {}))
                resolved = observed(value, state.memory)
                written = self.record(state, step, "STATE_WRITE_REACHED", resource=resource, value=resolved,
                                      **({"sourceValue": value} if member and value is not None else {}))
                fields = state.data_fields
                if resource[2][0] == "allocation":
                    fields = {**fields, resource: value}
                written = replace(written, memory={**state.memory, resource: resolved}, data_fields=fields,
                                  writes=(*state.writes, (step, resource, resolved)))
                return advance(written)
            self.boundaries.add("WRITE_TARGET_OR_SETTER_OPEN")
            return advance(self.record(replace(state, memory={}, fresh_lists={}, data_fields={}), key(), "WRITE_TARGET_OPEN"))
        if kind == "OPEN":
            self.boundaries.add(operation["reason"])
            opaque = self.record(replace(state, memory={}, fresh_lists={}, data_fields={}, uncertainties=(*state.uncertainties, operation["reason"])),
                                 key(), "COMPLETION_OPEN")
            return self.completions(opaque, lambda: advance(opaque), fault)
        if kind == "CONSTRUCT":
            constructed = ("allocation", fn.citation, identity, len(state.sequence))
            step = key(symbol=operation["symbol"], command=operation.get("command"), unresolved=operation.get("unresolved"),
                       parameter=operation.get("parameter"),
                       **{name: operation[name] for name in ("httpResult", "objectType", "recordFields", "resourceFamily", "emptyList")
                          if operation.get(name)})
            arguments = argument_values(operation.get("arguments", ()), frame)
            work = forget_lists(replace(state, memory={}), tuple(arg["value"] for arg in arguments))
            lists = {**work.fresh_lists, constructed: ()} if operation.get("emptyList") else work.fresh_lists
            record = {item["name"]: item["id"] for item in operation.get("recordFields", ()) if item["id"]}
            fields = {**work.data_fields, **{("member", record[arg["name"]], constructed): arg["value"]
                                            for arg in arguments if arg.get("name") in record}}
            return self.completions(
                work, lambda: advance(self.record(replace(work, nonnull=work.nonnull | {constructed}, fresh_lists=lists,
                                                          data_fields=fields),
                                                  step, "CONSTRUCTED", result=constructed, arguments=arguments),
                                      bind(("result", identity), constructed)),
                lambda exceptional: fault(self.record(exceptional, step, "CONSTRUCTOR_EXCEPTION")))
        if kind == "CALL":
            return self.call(ref, block, offset, operation, state, frame, stack, pending, visits, advance, fault, key, bind)
        if kind == "AWAIT":
            step = key()
            # Direct call/await pairs are processed together by call().
            work = replace(state, memory={})
            awaited = symbol(operation["value"], frame)
            details = {"awaited": awaited}
            if awaited and awaited[0] == "call-result" and self.steps.get(awaited[1], {}).get("taskLike"):
                details["call"] = awaited[1]
            return self.completions(
                work, lambda: advance(self.record(work, step, "RESUMED", **details)),
                lambda exceptional: fault(self.record(exceptional, step, "FAULT_OR_CANCELLATION", **details)))
        return [self.open(state, "UNSUPPORTED_EFFECT_OPERATION")]

    def call(self, ref, block, offset, call, state, frame, stack, pending, visits, advance, fault, key, bind):
        fn = self.functions[ref]
        evidence = self.roles.get((ref, call["id"]), {})
        role = call["role"] or evidence.get("role")
        nominal_publication = role is None and call.get("nominalPublication")
        kafka_position = call.get("kafkaTopicPosition")
        if nominal_publication or kafka_position is not None:
            role = "PUBLICATION_CANDIDATE"
        receiver = data_value(symbol(call["receiver"], frame), state.data_fields)
        actuals = {arg["id"]: symbol(arg["value"], frame) for arg in call["arguments"] if arg["id"] is not None}
        resource = receiver if role != "PERSISTENCE_REQUEST" else actuals.get(0)
        target, binding = self.resolve(call)
        if any(arg["byRef"] for arg in call["arguments"]):
            self.boundaries.add("BY_REF_ARGUMENT_EFFECTS_OPEN")
        step = key(symbol=call["symbol"], spelling=call.get("spelling"), role=role, dispatch=call["dispatch"],
                   binding=binding, taskLike=call.get("taskLike", False),
                   roleAuthority=(NOMINAL_FRAMEWORK if call.get("nominalFramework") else "ROSLYN_FRAMEWORK_DECLARATION")
                   if call["role"] else "CONDITIONAL_NOMINAL_BROKER_API" if nominal_publication else
                   "ROSLYN_PACKAGE_DECLARATION" if kafka_position is not None else evidence.get("roleAuthority"),
                   **{name: call[name] for name in ("dapperCandidate", "candidateEffectFamily") if call.get(name)},
                   **{key: evidence[key] for key in ("sql", "sourceSql", "topic") if key in evidence},
                   **({"kafkaTopicPosition": kafka_position} if kafka_position is not None else {}),
                   **{key: call[key] for key in ("httpResult", "receiverFamily", "resultFamily", "valueFactory", "implicit", "nullDefault", "directEffectFamily")
                      if call.get(key)},
                   **({"predicateSite": fn.site(call["predicateSite"])} if call.get("predicateSite") else {}),
                   boundSources=[self.functions[target].citation] if target else [])
        self.steps[step]["sources"] = list(dict.fromkeys([fn.citation, *evidence.get("sources", ())]))
        if role == "AWAITER_CONFIGURATION":
            return advance(state, bind(("result", call["id"]), receiver))
        operands = block["steps"][offset + 1:offset + 3]
        awaited, skip = None, 1
        previous = call["id"]
        for candidate in operands:
            operand = {"kind": "result", "id": previous}
            if candidate["kind"] == "CALL" and candidate["role"] == "AWAITER_CONFIGURATION" and candidate["receiver"] == operand:
                previous = candidate["id"]
                skip += 1
            elif candidate["kind"] == "AWAIT" and candidate["value"] == operand:
                awaited = self.step(fn, f"op{candidate['id']}", "AWAIT", candidate["site"], call=step)
                skip += 1
                break
            else:
                break

        if call.get("dynamic"):
            self.boundaries.add("DYNAMIC_CALL_COMPLETION_OPEN")
            state = replace(state, uncertainties=(*state.uncertainties, "DYNAMIC_CALL_COMPLETION_OPEN"))
        if call.get("nominalFramework") and NOMINAL_FRAMEWORK not in state.uncertainties:
            self.boundaries.add(NOMINAL_FRAMEWORK)
            state = replace(state, uncertainties=(*state.uncertainties, NOMINAL_FRAMEWORK))
        result = ("call-result", step, len(state.sequence))
        data = {"resource": resource, "receiver": receiver, "arguments": argument_values(call["arguments"], frame)}
        start = self.record(state, step, "INVOKED", result=result, **data)
        normal = start
        if call.get("instanceCall") and call.get("symbol") and receiver:
            if receiver == literal(None) or receiver in state.null:
                return fault(self.record(start, step, "CALL_EXCEPTION", call=step, **data),
                             ("System.NullReferenceException", "System.SystemException", "System.Exception", "object"))
            if stable_reference(receiver):
                normal = replace(start, nonnull=start.nonnull | {receiver})

        can_compose = target and (call["isAsync"] if awaited else not call["taskLike"])
        if can_compose and not self.local_only:
            candidate_start = normal
            if binding == "CONDITIONAL_SOURCE_CANDIDATE":
                candidate_start = replace(normal, uncertainties=(*normal.uncertainties, binding))
                self.boundaries.add(binding)
            target_parameters = self.functions[target].graph["parameters"]
            unbound = {parameter["id"] for parameter in target_parameters if parameter["unboundToken"]}
            tokens = {arg["position"]: arg["cancellationTokenEvidence"] for arg in data["arguments"]
                      if arg["position"] in unbound and arg.get("cancellationTokenEvidence")}
            inner = Frame(receiver, actuals, cancellation_parameters=tokens)
            if self.coverage:
                self.entry_seeds.setdefault(target, EntrySeed(candidate_start, inner, step, binding))

        if can_compose and not self.local_only and (self.compose_candidates or binding != "CONDITIONAL_SOURCE_CANDIDATE"):
            exits = (self.normal_returns(target, candidate_start, inner, stack) if self.summary_queue is not None else
                     self.enter(target, candidate_start, inner, stack))

            def continue_after(result):
                if result.kind in {"NORMAL_EXIT", "RETURNED"}:
                    outcome = "RESUMED" if awaited else ("AWAITABLE_RETURNED" if call.get("taskLike") else "RETURNED")
                    work = self.record(result.state, awaited or step, outcome, call=step,
                                       composed=binding, **({"returnedValue": result.value} if result.value is not None else {}),
                                       **({"returnedFamily": self.functions[target].graph["returnFamily"]}
                                          if self.functions[target].graph.get("returnFamily") else {}))
                    result_id = block["steps"][offset + skip - 1]["id"] if awaited else call["id"]
                    return advance(work, bind(("result", result_id), result.value), skip)
                if result.kind == "THREW":
                    work = self.record(result.state, awaited or step, "FAULT_OR_CANCELLATION" if awaited else "CALL_EXCEPTION",
                                       call=step, composed=binding)
                    return fault(work, result.exception_types)
                return [result]

            composed = self.bounded(continue_after(result) for result in exits)
            if binding != "CONDITIONAL_SOURCE_CANDIDATE" or self.summary_queue is not None:
                return composed
            if self.coverage and composed:
                # Reserve the successful coverage pass for joinable source
                # paths. Exploration still reports the unmatched runtime arm.
                return composed
        else:
            composed = []
        if target and not can_compose:
            self.boundaries.add("ASYNC_HELPER_SCHEDULING_OPEN")
            start = replace(start, uncertainties=(*start.uncertainties, "ASYNC_HELPER_SCHEDULING_OPEN"))
        if call["inSource"] or call["dispatch"] == "OPEN":
            self.boundaries.add("CALL_BODY_OR_RUNTIME_DISPATCH_OPEN")
        # Opaque calls may change tracked state through aliases. Previous branch
        # decisions remain historical observations and are never reinstated here.
        list_update = role in {"LIST_ADD", "LIST_CLEAR"} and receiver in state.fresh_lists
        factory = call.get("valueFactory")
        list_source = actuals.get(0) or receiver
        selections = None
        if factory == "LINQ_FIRST_OR_DEFAULT":
            selections = self.select(fn, call, normal, frame, list_source)
        projection = factory == "LINQ_SELECT" and call.get("selector") and list_source is not None
        work = replace(start, memory={})
        if not (list_update or selections is not None or projection):
            work = forget_lists(work, (receiver, *actuals.values()))
        result_id = block["steps"][offset + skip - 1]["id"] if awaited else call["id"]
        if call.get("spelling") == "FirstOrDefault" and selections is None:
            self.boundaries.add(LIST_SELECTION_OPEN)
            work = replace(work, uncertainties=(*work.uncertainties, LIST_SELECTION_OPEN))

        def resume(current, returned, **details):
            outcome = "RESUMED" if awaited else ("AWAITABLE_RETURNED" if call.get("taskLike") else "RETURNED")
            if opaque_member(self.steps[step], receiver) and (receiver == literal(None) or receiver in current.null):
                current = self.null_dispatch(current)
                details["dispatchPremise"] = NULL_DISPATCH
            if target or call["inSource"]:
                details["unexpandedSource"] = True
            completed = self.record(current, awaited or step, outcome, call=step, **data, **details)
            return advance(completed, bind(("result", result_id), returned), skip)

        def complete():
            if selections is not None:
                return self.bounded(resume(current, value, **details) for current, value, details in selections)
            resumed = replace(work, nonnull=normal.nonnull)
            if list_update:
                items = (*resumed.fresh_lists[receiver], actuals.get(0)) if role == "LIST_ADD" else ()
                resumed = replace(resumed, fresh_lists={**resumed.fresh_lists, receiver: items})
            if projection:
                resumed = replace(resumed, projections={**resumed.projections,
                                                         result: (list_source, call["selector"], frame)},
                                  nonnull=resumed.nonnull | {result})
            return resume(resumed, result)

        def interrupt(failed):
            failed = self.record(failed, awaited or step, "FAULT_OR_CANCELLATION" if awaited else "CALL_EXCEPTION",
                                 call=step, **data)
            return fault(failed)

        priority = (step == self.focus_call if self.focus_call else
                    self.commands["completion_boundary"](role, call.get("spelling"), call["dispatch"]))
        return self.bounded(action() for action in (lambda: composed,
                                                    lambda: self.completions(work, complete, interrupt, priority)))

    def select(self, fn, call, state, frame, source):
        items = collection_items(source, state)
        if items is None:
            return None
        default = literal(None) if call.get("nullDefault") else None
        if not items:
            return [(state, default, {"emptyListDefault": True})] if default else None
        if not call.get("hasSelector"):
            return [(state, items[0], {"selectedValue": items[0]})] if items[0] is not None else None
        if not call.get("selector") or not call.get("predicateSite"):
            return None
        for index, item in enumerate(items):
            if item is None:
                return None
            test = data_value(symbol(call["selector"], replace(frame, selector_item=item)), state.data_fields)
            known = observed(test, state.memory)
            if known == literal(False):
                continue
            if known == literal(True):
                return [(state, item, {"selectedValue": item})]
            # Keep a last unproven comparison as a premise. Ambiguous earlier
            # matches could choose a different element and are left open.
            if not selector_term(test) or index != len(items) - 1 or default is None:
                return None
            key = self.step(fn, f"select{call['id']}", "CONDITION", call["predicateSite"],
                            sourcePredicate=True, selectionPredicate=True)
            choices = []
            for truth, value in ((True, item), (False, default)):
                if any(prior["operand"] == test and prior["value"] != truth for prior in state.conditions):
                    continue
                condition = {"step": key, "value": truth, "operand": test, "guardReturn": False, "guard": None}
                branch = replace(self.record(state, key, "TRUE" if truth else "FALSE"),
                                 conditions=(*state.conditions, condition))
                choices.append((branch, value, {"selectedValue": value}))
            return choices
        return [(state, default, {"selectedValue": default})] if default else None

    def edge(self, ref, block, state, frame, stack, pending, visits):
        fn = self.functions[ref]
        decision = block["decision"]

        def follow(edge, current, returned=None):
            if edge is None:
                return [self.open(current, "CFG_EDGE_UNAVAILABLE")]
            kind = edge["kind"]
            if kind in {"Throw", "Rethrow"}:
                if decision:
                    step = self.step(fn, f"throw{block['id']}", "THROW", decision["site"],
                                     explicit=decision.get("explicitThrow", False))
                    current = self.record(current, step, "THROWN")
                elif kind == "Rethrow":
                    self.boundaries.add("RETHROW_EXCEPTION_TYPE_OPEN")
                return self.exception(ref, block, current, frame, stack, pending, visits, tuple(block["throwType"]))
            if kind == "StructuredExceptionHandling" and pending:
                return self.route(ref, pending[-1][1], current, frame, stack, pending[:-1], visits)
            if kind in {"Regular", "Return"}:
                actions = tuple(("finally", cleanup) for cleanup in edge["cleanups"])
                if returned and (edge["target"] is None or fn.graph["blocks"][edge["target"]]["kind"] != "Exit"):
                    returned = None
                if returned:
                    step = self.step(fn, f"guard-return{block['id']}/{returned['start']}", "RETURN", returned)
                    current = self.record(current, step, "GUARD_RETURN")
                if kind == "Return" or returned:
                    value = symbol(decision["value"], frame) if decision and not returned else None
                    if kind == "Return" and decision:
                        step = self.step(fn, f"return{block['id']}", "RETURN", decision["site"])
                        current = self.record(current, step, "RETURN", **({"value": value} if value is not None else {}))
                    exit_ = ("exit", Exit(current, "RETURNED", value=value))
                    actions = (*actions, exit_)
                else:
                    actions = (*actions, ("block", edge["target"]))
                return self.route(ref, actions, current, frame, stack, pending, visits)
            return [self.open(current, "UNSUPPORTED_CFG_EDGE_" + kind.upper())]

        if block["branch"] == "None":
            return follow(block["next"], state)
        if not decision:
            return [self.open(state, "CFG_CONDITION_UNAVAILABLE")]
        condition = data_value(symbol(decision["value"], frame), state.data_fields)
        known = observed(condition, state.memory)
        step = self.condition_step(fn, block)

        def choice(truth, edge):
            if known and type(known[1]) is bool and known[1] is not truth:
                return []
            guard = block["guard"]
            returned = guard["whenTrue" if truth else "whenFalse"] if guard else None
            item = {"step": step, "value": truth, "operand": condition, "guardReturn": returned is not None,
                    "guard": eligible_guard(condition, truth)}
            checked, is_null = null_test(condition, truth)
            if checked and (is_null and checked in state.nonnull or not is_null and checked in state.null):
                return []
            details = {"modeledOperandValue": known[1]} if known and type(known[1]) is bool else {}
            state_for_choice = state
            if checked and is_null:
                completed = [event["step"] for event in state.sequence
                             if event.get("receiver") == checked and event["outcome"] in {"RESUMED", "RETURNED", "AWAITABLE_RETURNED"}
                             and opaque_member(self.steps.get(event.get("call"), {}), checked)]
                if completed:
                    state_for_choice = self.null_dispatch(state)
                    details = {**details, "dispatchPremise": NULL_DISPATCH, "receiver": checked,
                               "receiverCallCompletions": list(dict.fromkeys(completed))}
            current = replace(self.record(state_for_choice, step, "TRUE" if truth else "FALSE", **details),
                              conditions=(*state.conditions, item))
            if checked:
                current = (replace(current, null=current.null | {checked}) if is_null
                           else replace(current, nonnull=current.nonnull | {checked}))
            return follow(edge, current, returned)

        on_true = block["branch"] == "WhenTrue"
        choices = ((on_true, block["conditional"]), (not on_true, block["next"]))
        return self.bounded(choice(truth, edge) for truth, edge in choices)

    def condition_step(self, fn, block):
        decision = block["decision"]
        return self.step(fn, f"if{block['id']}", "CONDITION", decision["site"],
                         **({"sourcePredicate": True} if decision.get("sourcePredicate") else {}),
                         **({"loopTest": decision["loopTest"]} if decision.get("loopTest") else {}))

    def null_dispatch(self, state):
        self.boundaries.add(NULL_DISPATCH)
        return replace(state, uncertainties=(*state.uncertainties, NULL_DISPATCH))

    def route(self, ref, actions, state, frame, stack, pending, visits):
        kind, item = actions[0]
        if kind == "exit":
            return [replace(item, state=state)]
        if kind == "open":
            return [self.open(state, item)]
        if kind == "catch":
            region = self.functions[ref].graph["regions"][item]
            state = self.region_entry(ref, region, state, "CATCH")
            return self.block(ref, region["entry"], 0, state, frame, stack, pending, visits)
        if kind == "finally":
            region = self.functions[ref].graph["regions"][item]
            state = self.region_entry(ref, region, state, "FINALLY")
            return self.block(ref, region["entry"], 0, state, frame, stack, (*pending, (item, actions[1:])), visits)
        if item is None:
            return [self.open(state, "CFG_DESTINATION_UNAVAILABLE")]
        return self.block(ref, item, 0, state, frame, stack, pending, visits)

    def region_entry(self, ref, region, state, kind):
        fn = self.functions[ref]
        if not region["site"]:
            return state
        key = self.step(fn, f"region{region['id']}", kind, region["site"], exceptionType=region["type"])
        return self.record(state, key, "ENTERED")

    def exception(self, ref, block, state, frame, stack, pending, visits, types):
        """Build possible dispatches. Roslyn CFG edges omit exceptional transfer."""
        regions = self.functions[ref].graph["regions"]
        child = regions[block["region"]]
        actions, outcomes, abandoned = (), [], set()
        matched = False
        while child["parent"] is not None and not matched:
            parent = regions[child["parent"]]
            siblings = [r for r in regions if r["parent"] == parent["id"]]
            active = tuple(entry for entry in pending if entry[0] not in abandoned)
            if parent["kind"] == "TryAndCatch" and child["kind"] == "Try":
                for handler in siblings:
                    if handler["kind"] not in {"Catch", "FilterAndHandler"}:
                        continue
                    type_ = handler["type"]
                    if types and type_ not in {None, *types}:
                        continue
                    if handler["kind"] == "FilterAndHandler":
                        outcomes.append(((*actions, ("open", "CATCH_FILTER_COMPLETION_OPEN")), active))
                        continue
                    outcomes.append(((*actions, ("catch", handler["id"])), active))
                    if types or type_ in {None, "object", "System.Exception"}:
                        matched = True
                        break
            if parent["kind"] == "TryAndFinally":
                cleanup = next((r["id"] for r in siblings if r["kind"] == "Finally"), None)
                if child["kind"] == "Try" and cleanup is not None:
                    actions = (*actions, ("finally", cleanup))
                if child["kind"] == "Finally":
                    abandoned.add(child["id"])
            child = parent
        if not matched:
            active = tuple(entry for entry in pending if entry[0] not in abandoned)
            outcomes.append(((*actions, ("exit", Exit(state, "THREW", exception_types=types))), active))
        return self.bounded(self.route(ref, path, state, frame, stack, active, visits) for path, active in outcomes)

    def trace(self, ref, signature, tokens, hosts, is_http=False):
        fn = self.functions[ref]
        parameters = {parameter["id"]: ("parameter", parameter["id"], parameter["name"])
                      for parameter in fn.graph["parameters"]}
        frame = Frame(("entry-receiver",), parameters, cancellation_parameters=tokens)
        self.coverage, self.path_limit = True, MAX_COVERAGE_PATHS
        summarized = self.normal_returns(ref, State(), frame, ()) if is_http else ()
        self.visits = 0
        normal = self.enter(ref, State(), frame, ())
        independent = []
        if any(step.get("binding") == "CONDITIONAL_SOURCE_CANDIDATE" for step in self.steps.values()):
            self.compose_candidates, self.path_limit, self.visits = False, 8, 0
            independent = self.enter(ref, State(), frame, ())
            self.compose_candidates = True
        has_commands = any(self.commands["completion_boundary"](op.get("role"), op.get("spelling"), op.get("dispatch"))
                           for fn in self.functions.values() for block in fn.graph["blocks"] for op in block["steps"])
        priority = []
        if has_commands:
            self.coverage, self.focal, self.path_limit, self.visits = False, True, 18, 0
            priority = self.enter(ref, State(), frame, ())
        self.coverage, self.focal, self.path_limit, self.visits = False, False, MAX_PATHS, 0
        explored = self.enter(ref, State(), frame, ())
        path_rows, known = [], set()
        summarized_ids = {id(outcome) for outcome in summarized}
        for result in (*summarized, *normal, *independent, *priority, *explored):
            state = result.state
            sequence = list(state.sequence)
            if not sequence or not any(self.steps[item["step"]]["kind"] in {*EFFECTS, "CONDITION", "THROW", "RETURN"}
                                       for item in sequence):
                continue
            identity = json.dumps((result.kind, state.sequence, state.conditions, state.uncertainties), sort_keys=True)
            if identity in known:
                continue
            known.add(identity)
            key = len(path_rows) + 1
            if key > MAX_PATHS:
                self.boundaries.add("PATH_ENUMERATION_BUDGET")
                break
            row = {"id": f"P{key:02d}", "outcome": result.kind, "sequence": sequence,
                   "conditions": [{key: value for key, value in item.items() if key != "guard"}
                                  for item in state.conditions],
                   **({"returnValue": result.value} if result.value is not None else {}),
                   **({"proofAuthority": "COMPOSED_NORMAL_RETURNS"} if id(result) in summarized_ids else {}),
                   "open": list(dict.fromkeys(state.uncertainties))}
            path_rows.append((row, state))
        paths = self.skipped(path_rows)
        helper_models, helper_steps = self.helper_models(ref)
        guard_returns = self.structural_guard_returns(fn.citation, helper_models)
        inventory = self.direct_effect_inventory(ref)
        composed_inventory = self.composed_effect_inventory(ref)
        guard_controls = self.structural_guard_controls(fn.citation, helper_models, hosts)
        host_controls = self.hosted_controls(hosts)
        caught = self.catch_continuations(fn.citation, helper_models, hosts)
        guarded_writes, catch_returns = self.structural_outcomes(fn.citation, helper_models, hosts)
        executions, command_durable, transactions = self.commands["analyze"](
            paths, self.steps, self.sql_tables, self.storage, fn.citation if is_http else None)
        local_paths, local_executions = self.callable_paths(ref, helper_models, paths, executions, worker=bool(hosts))
        if (not guard_returns and not inventory and not composed_inventory and not executions and len(self.functions) == 1 and
                not guard_controls and not host_controls
                and not local_paths and not caught and not guarded_writes and not catch_returns and not any(step.get("sourceSql") for step in self.steps.values())
                and not any(step["kind"] in {"AWAIT", "THROW", "RETURN"} for step in self.steps.values())):
            return report("NO_RETAINED_AWAIT_OR_ABRUPT_EXIT", sorted(self.boundaries))
        guard_steps = {key for fact in guard_returns for key in (fact["condition"], fact["return"], *fact["bypasses"])}
        guard_steps.update(self.steps[key]["guardAwaitStep"] for key in tuple(guard_steps) if self.steps[key].get("guardAwaitStep"))
        guard_steps.update(key for fact in guard_controls for key in (fact["condition"], fact["control"]))
        guard_steps.update(key for fact in host_controls for key in fact["sites"].values())
        guard_steps.update(key for fact in caught for key in fact["evidenceSteps"])
        guard_steps.update(key for fact in (*guarded_writes, *catch_returns) for key in fact["sites"].values())
        used = {item["step"] for row in (*paths, *local_paths) for item in row["sequence"]} | helper_steps | guard_steps
        if inventory:
            used.update(item["step"] for item in inventory["sites"])
        if composed_inventory:
            for scope in composed_inventory["scopes"]:
                used.update(scope["entrySites"])
                for item in scope["sites"]:
                    used.add(item["step"])
                    used.update(guard["condition"] for guard in item.get("witness", ()))
        used.update(row["scope"]["call"] for row in local_paths if row["scope"].get("call"))
        used.update(key for row in local_paths for key in row["scope"].get("openSites", ()))
        interruptions, repeated = self.interruptions(paths, executions), self.repeated(path_rows)
        durable = self.storage["durable_candidates"](paths, self.steps, 12)
        durable.extend(self.local_durable_paths(local_paths, durable))
        compact_paths, events = compact_paths_and_events([*paths, *local_paths])
        return report("PROJECTED" if paths else "OPEN", sorted(self.boundaries),
                      coverage="BOUNDED_RETAINED_SOURCE_ONLY",
                      assumptions=["The compiler projection uses selected pinned files and SDK framework metadata.",
                                   "Opaque operations and implicit exceptions may change outcomes.",
                                   *(["Some framework bindings assume the SDK's generated global imports."]
                                     if SDK_IMPORTS in self.boundaries else []),
                                   "An API return or await resume records source completion. External delivery, durability and rollback remain open.",
                                   *(["A matching sealed native signature can supply a nominal parameter type. "
                                      "Imports outside the selected source and assembly resolution remain open."] if tokens else [])],
                      root={"symbol": fn.graph["symbol"], "sources": [fn.citation],
                            "inputs": [self.root_input(fn, parameter) for parameter in fn.graph["parameters"]],
                            **({"sealedNativeSignature": signature} if tokens else {})},
                      steps=[step for key, step in self.steps.items() if key in used],
                      pathEncoding="Follow lastEvent through sequenceEvents.previous, then reverse.",
                      paths=compact_paths[:len(paths)], sequenceEvents=events, retainedHelpers=helper_models,
                      **({"boundedCallablePaths": compact_paths[len(paths):]} if local_paths else {}),
                      **({"structuralGuardReturns": guard_returns} if guard_returns else {}),
                      **({"directEffectInventory": inventory} if inventory else {}),
                      **({"composedEffectInventory": composed_inventory} if composed_inventory else {}),
                      **({"structuralGuardControls": guard_controls} if guard_controls else {}),
                      **({"hostedLoopStructure": host_controls} if host_controls else {}),
                      **({"catchContinuations": caught} if caught else {}),
                      **({"awaitedGuardWrites": guarded_writes} if guarded_writes else {}),
                      **({"catchReturns": catch_returns} if catch_returns else {}),
                      postCompletionInterruptions=interruptions, openAwaitOrdering=self.open_await_ordering(paths),
                      **({"relationalCommandPaths": executions} if executions else {}),
                      **({"httpTransactionOutcomes" if is_http else "transactionalWriteCompletions": transactions}
                         if transactions else {}),
                      **({"boundedCallableCommands": local_executions} if local_executions else {}),
                      possibleRepeatedEffects=[*repeated, *durable, *command_durable])

    def callable_paths(self, root, helpers, paths, executions, *, worker):
        """Spend separate small coverage budgets on selected callables and their rooted worker outcomes."""
        fallback = self.boundaries.intersection({"TRACE_BUDGET", "PATH_ENUMERATION_BUDGET", "CALL_CYCLE_OR_DEPTH"})
        if not fallback and not worker:
            return [], []
        retained = {self.functions[root].citation, *(helper["id"] for helper in helpers)}
        normal = {path["id"] for path in paths if path["outcome"] in {"RETURNED", "NORMAL_EXIT"} and
                  NULL_DISPATCH not in path["open"] and LIST_SELECTION_OPEN not in path["open"]}
        visible = {cmd["call"] for group in executions for cmd in group["commands"]
                   if group["path"] in normal and cmd["outcome"] in {"RETURNED", "RESUMED"}}
        visible.update(event["step"] for path in paths for event in path["sequence"]
                       if event["outcome"] == "INVOKED" and self.steps[event["step"]].get("kafkaTopicPosition") is not None)
        visible.update(event["call"] for path in paths if path["id"] in normal
                       for event in path["sequence"] if event.get("call") and
                       self.steps[event["call"]].get("spelling") == "FirstOrDefault")
        local = Trace(self.functions, set(), self.roles, self.storage, self.sql_tables, self.commands)
        local.steps = self.steps
        local.coverage, local.local_only, local.path_limit, local.visit_limit = True, True, LOCAL_PATHS, LOCAL_VISITS
        round_robin = []
        for ref, fn in self.functions.items():
            if fn.citation not in retained:
                continue
            operations = [op for block in fn.graph["blocks"] if block["reachable"] for op in block["steps"] if op["kind"] == "CALL"]
            wanted = {f"{fn.citation}/op{op['id']}" for op in operations if op.get("spelling") in {
                "ExecuteReader", "ExecuteReaderAsync", "ExecuteScalar", "ExecuteScalarAsync", "ExecuteNonQuery",
                "ExecuteNonQueryAsync", "FirstOrDefault", *TRANSACTION_METHODS} or
                op.get("kafkaTopicPosition") is not None or self.roles.get((ref, op["id"]), {}).get("sourceSql")}
            seed = self.entry_seeds.get(ref)
            commit_sites = [f"{fn.citation}/op{op['id']}" for op in operations
                            if op.get("spelling") in {"Commit", "CommitAsync"}]
            publication_sites = [f"{fn.citation}/op{op['id']}" for op in operations
                                 if op.get("nominalPublication") or op.get("kafkaTopicPosition") is not None]
            rooted = worker and seed and commit_sites and publication_sites
            if (not fallback and not rooted) or ref == root and not (wanted - visible):
                continue
            if len(round_robin) == LOCAL_CALLABLES:
                break
            scope = {"symbol": fn.graph["symbol"], "sources": [fn.citation], "prefixEvents": 0,
                     "authority": "RETAINED_CALLABLE_DECLARATION"}
            frame = Frame(("entry-receiver",), {parameter["id"]: ("parameter", parameter["id"], parameter["name"])
                                                 for parameter in fn.graph["parameters"]})
            native = self.local_witnesses(local, ref, State(), frame, scope, wanted, keep_normal=True) if fallback else []
            if fallback and not native:
                open_sites = [self.step(fn, f"op{op['id']}", "CALL", op["site"], spelling=op.get("spelling"))
                              for op in operations if f"{fn.citation}/op{op['id']}" in wanted][:4]
                native = [({"outcome": "OPEN", "sequence": [], "conditions": [], "sources": [fn.citation],
                            "scope": {**scope, "openSites": open_sites}, "open": ["CALLABLE_EFFECT_UNRESOLVED"]}, [])]
            elif native and native[0][0]["scope"].get("unresolved"):
                native[0][0]["scope"]["openSites"] = [self.step(fn, f"op{op['id']}", "CALL", op["site"], spelling=op.get("spelling"))
                                                       for op in operations if f"{fn.citation}/op{op['id']}" in wanted][:4]
            variants = []
            if seed:
                state = replace(seed.state, event_base=len(seed.state.sequence), transfers=0, modeled_faults=0)
                seeded = {**scope, "call": seed.call, "authority": seed.authority, "prefixEvents": state.event_base}
                if rooted:
                    seeded = {**seeded, "rootedWorker": True}
                    local.local_only = False
                variants.extend(self.local_witnesses(local, ref, state, seed.frame, seeded, wanted))
                if rooted:
                    local.coverage, local.focal = False, True
                    for site in [*commit_sites, *publication_sites][:4]:
                        local.focus_call = site
                        witnesses = self.local_witnesses(local, ref, state, seed.frame, seeded, wanted)
                        variants.extend(witnesses[:2])
                    local.coverage, local.focal, local.local_only, local.focus_call = True, False, True, None
            witnesses = [*variants, *native] if rooted or not native else [native[0], *variants, *native[1:]]
            round_robin.append(iter(witnesses))
        results, groups = [], []
        while round_robin and len(results) < LOCAL_OUTPUT_PATHS:
            remaining = []
            for variants in round_robin:
                if (item := next(variants, None)) is None:
                    continue
                row, commands = item
                row["id"] = f"L{len(results) + 1:02d}"
                results.append(row)
                if commands:
                    groups.append({"path": row["id"], "commands": commands})
                remaining.append(variants)
                if len(results) == LOCAL_OUTPUT_PATHS:
                    break
            round_robin = remaining
        if results:
            self.boundaries.add("CALLABLE_SCOPE_REPLAY_OUTER_JOIN_OPEN")
        return results, groups

    def local_durable_paths(self, paths, existing):
        """Compare alternatives only inside the same retained callback invocation prefix."""
        groups = defaultdict(list)
        for path in paths:
            scope = path["scope"]
            if scope.get("rootedWorker"):
                prefix = json.dumps(path["sequence"][:scope["prefixEvents"]], sort_keys=True)
                groups[scope["call"], scope["symbol"], prefix].append(path)
        signature = ("select", "externalCall", "interruption", "matchingWrite", "matchingCommit")
        seen = {tuple(row[key] for key in signature) for row in existing}
        derived = []
        for group in groups.values():
            for row in self.storage["durable_candidates"](group, self.steps, 8):
                key = tuple(row[field] for field in signature)
                if key not in seen:
                    seen.add(key)
                    derived.append(row)
            if len(derived) >= 8:
                break
        return derived[:8]

    def local_witnesses(self, local, ref, state, frame, scope, wanted, *, keep_normal=False):
        local.visits = 0
        results, seen = [], set()
        first_normal = None
        for exit_ in local.enter(ref, state, frame, ()):
            current = exit_.state
            suffix = current.sequence[scope["prefixEvents"]:]
            if NULL_DISPATCH in current.uncertainties:
                continue
            if local.focus_call and not any(event.get("call") == local.focus_call and
                                            event["outcome"] in {"FAULT_OR_CANCELLATION", "CALL_EXCEPTION"} for event in suffix):
                continue
            row = {"outcome": exit_.kind, "sequence": list(current.sequence), "scope": scope, "sources": scope["sources"],
                   "conditions": [{key: value for key, value in item.items() if key != "guard"} for item in current.conditions],
                   "open": list(dict.fromkeys(current.uncertainties))}
            if keep_normal and first_normal is None and exit_.kind in {"RETURNED", "NORMAL_EXIT"}:
                first_normal = {**row, "scope": {**scope, "unresolved": True}}
            if not wanted.intersection(event.get("call") or event["step"] for event in suffix):
                continue
            commands = self.commands["project_callable"](row, self.steps, self.sql_tables, self.storage)
            material = tuple(dict.fromkeys(event.get("call") or event["step"] for event in suffix if
                event.get("call") in wanted and event["outcome"] in {"RETURNED", "RESUMED"} or
                event["step"] in wanted and event["outcome"] == "INVOKED" and
                (self.steps[event["step"]].get("sourceSql") or self.steps[event["step"]].get("kafkaTopicPosition") is not None)))
            if not commands and not any(self.steps[key].get("spelling") == "FirstOrDefault" or
                                        self.steps[key].get("spelling") in TRANSACTION_METHODS or
                                        self.steps[key].get("sourceSql") or
                                        self.steps[key].get("kafkaTopicPosition") is not None for key in material):
                continue
            identity = exit_.kind, material, tuple((command["call"], command["outcome"]) for command in commands)
            if identity not in seen:
                seen.add(identity)
                results.append((row, commands))
        if first_normal and not any(row["outcome"] in {"NORMAL_EXIT", "RETURNED"} for row, _ in results):
            results.insert(0, (first_normal, []))
        return sorted(results, key=lambda item: item[0]["outcome"] not in {"NORMAL_EXIT", "RETURNED"})

    def structural_guard_returns(self, root_citation, helpers):
        """Prove local bypasses independently of trace completion or call binding."""
        selected = {root_citation, *(helper["id"] for helper in helpers)}
        facts = []
        for fn in self.functions.values():
            if fn.citation not in selected:
                continue
            blocks = fn.graph["blocks"]
            for block in blocks:
                if not block["reachable"] or not block["guard"] or not block["decision"].get("sourcePredicate"):
                    continue
                value = block["decision"].get("value") or {}
                if value.get("kind") == "literal" and isinstance(value.get("value"), bool):
                    continue
                on_true = block["branch"] == "WhenTrue"
                true_edge, false_edge = block["conditional"], block["next"]
                if not on_true:
                    true_edge, false_edge = false_edge, true_edge
                for truth, exiting, continuing in ((True, true_edge, false_edge), (False, false_edge, true_edge)):
                    returned = block["guard"]["whenTrue" if truth else "whenFalse"]
                    if not returned or not self.return_edge(blocks, exiting, returned):
                        continue
                    bypasses, bounded = self.continuing_sites(fn, block, continuing)
                    condition = self.condition_step(fn, block)
                    exit_step = self.step(fn, f"guard-return{block['id']}/{returned['start']}", "RETURN", returned)
                    facts.append({"condition": condition, "when": truth, "return": exit_step,
                                  "bypasses": bypasses, "bypassCoverage": "PARTIAL" if bounded else "CFG_SUFFIX",
                                  "sources": [fn.citation]})
        return facts

    @staticmethod
    def return_edge(blocks, edge, returned):
        if not edge or edge["kind"] not in {"Regular", "Return"} or edge["target"] is None:
            return False
        target = blocks[edge["target"]]
        if target["kind"] == "Exit":
            return True
        # A return operand is often lowered to a separate block, after evaluation of the guard.
        decision, successor = target["decision"], target["next"]
        return (target["reachable"] and not target["conditional"] and decision and decision["site"] == returned and
                successor and successor["kind"] == "Return" and
                successor["target"] is not None and blocks[successor["target"]]["kind"] == "Exit")

    @staticmethod
    def root_input(fn, parameter):
        row = {key: parameter[key] for key in ("id", "name")}
        site = parameter.get("site")
        if site and fn.interval[0] <= site["start"] < site["end"] <= fn.interval[1] and parameter.get("input"):
            row.update(parameter["input"], site=fn.site(site))
        return row

    def hosted_controls(self, hosts):
        facts = []
        for ref in hosts:
            fn = self.functions[ref]
            for index, row in enumerate(fn.graph.get("loopStructure", ())):
                sites = {name: self.step(fn, f"host{index}/{name}", "HOST_LOOP", row[name])
                         for name in ("site", "test", "exception", "filter", "token") if row.get(name)}
                sites.update({f"action{offset}": self.step(fn, f"host{index}/action{offset}", "HOST_LOOP", action)
                              for offset, action in enumerate(row.get("actions", ()))})
                facts.append({"kind": row["kind"], "loop": row["loop"], "sites": sites,
                              "host": fn.graph["symbol"], "sources": [fn.citation],
                              **({"conditional": True} if row.get("conditional") else {}),
                              **({"milliseconds": row["milliseconds"]} if row.get("milliseconds") is not None else {})})
        return facts

    def catch_continuations(self, root, helpers, hosts):
        selected = {root, *(item["id"] for item in helpers), *(self.functions[ref].citation for ref in hosts)}
        facts = []
        for fn in self.functions.values():
            if fn.citation not in selected:
                continue
            for index, raw in enumerate(fn.graph.get("catchContinuations", ())):
                def site(name, anchor):
                    return self.step(fn, f"caught{index}/{name}", "CATCH_CONTINUATION", anchor)

                caught = {name: site(name, raw[name]) for name in ("site", "clause", "filter", "loopEntryTest") if raw[name]}
                guards = [{"step": site(f"guard{offset}", check["site"]), "when": check["when"]}
                          for offset, check in enumerate(raw["tests"])]
                actions = [site(f"action{offset}", anchor) for offset, anchor in enumerate(raw["actions"])]
                awaits = [site(f"await{offset}", anchor) for offset, anchor in enumerate(raw["awaits"])]
                facts.append({"host": fn.graph["symbol"], "catch": caught, "guards": guards, "actions": actions,
                              "awaits": awaits, "sources": [fn.citation],
                              "authority": "ROSLYN_CATCH_ENDPOINT_AND_SOURCE_SEQUENCE",
                              "evidenceSteps": [*caught.values(), *(guard["step"] for guard in guards), *actions, *awaits],
                              **({"loop": raw["loop"]} if raw["loop"] else {})})
        return facts

    def structural_outcomes(self, root, helpers, hosts):
        selected = {root, *(item["id"] for item in helpers), *(self.functions[ref].citation for ref in hosts)}
        writes, returns = [], []
        for fn in self.functions.values():
            if fn.citation not in selected:
                continue
            calls = {(op["site"]["start"], op["site"]["end"]): op for block in fn.graph["blocks"]
                     if block["reachable"] for op in block["steps"] if op["kind"] == "CALL"}
            for index, row in enumerate(fn.graph.get("awaitedGuardWrites", ())):
                sites = {name: self.step(fn, f"guarded-write{index}/{name}", "AWAITED_GUARD_WRITE", row[name])
                         for name in ("condition", "awaitSite", "write")}
                refs = [fn.citation]
                if (raw := row.get("callSite")) and (call := calls.get((raw["start"], raw["end"]))):
                    target, binding = self.resolve(call)
                    if target and binding == "ROSLYN_STATIC_BINDING":
                        refs.append(self.functions[target].citation)
                writes.append({"host": fn.graph["symbol"], "when": row["when"], "sites": sites, "sources": refs})
            for index, row in enumerate(fn.graph.get("catchReturns", ())):
                sites = {name: self.step(fn, f"catch-return{index}/{name}", "CATCH_RETURN", row[name])
                         for name in ("handler", "returned", "exception", "filter") if row[name]}
                returns.append({"host": fn.graph["symbol"], "sites": sites, "sources": [fn.citation]})
        return writes, returns

    def structural_guard_controls(self, root_citation, helpers, hosts):
        selected = {root_citation, *(helper["id"] for helper in helpers), *(self.functions[ref].citation for ref in hosts)}
        facts = []
        for fn in self.functions.values():
            if fn.citation not in selected:
                continue
            for block in fn.graph["blocks"]:
                if not block["reachable"] or not block["guard"] or not block["decision"].get("sourcePredicate"):
                    continue
                for truth, field in ((True, "trueControl"), (False, "falseControl")):
                    if not (action := block["guard"].get(field)):
                        continue
                    guard = self.condition_step(fn, block)
                    control = self.step(fn, f"control{block['id']}/{action['site']['start']}", "CONTROL", action["site"])
                    facts.append({"condition": guard, "when": truth, "control": control,
                                  "kind": action["kind"], "target": action["target"], "sources": [fn.citation]})
        return facts

    def continuing_sites(self, fn, guard, edge):
        """Retain ordinary callable sites reachable on the continuing arm."""
        reachable, seen, queue = [], {guard["id"]}, [edge]
        cutoff = guard["decision"]["site"]["end"]
        bounded = False
        for edge in queue:
            if not edge or edge["kind"] != "Regular" or edge["target"] is None:
                continue
            if edge["cleanups"]:
                # The destination is the ordinary continuation after any cleanup. Do not list cleanup sites as bypassed.
                bounded = True
            target = edge["target"]
            if target in seen:
                continue
            if len(seen) > MAX_HELPER_BLOCKS:
                bounded = True
                break
            seen.add(target)
            block = fn.graph["blocks"][target]
            if not block["reachable"]:
                continue
            decorated = {op["id"]: (op.get("receiver") or {}).get("id") for op in block["steps"]
                         if op.get("role") == "AWAITER_CONFIGURATION"}
            awaiting = {(op.get("value") or {}).get("id"): op for op in block["steps"]
                        if op["kind"] == "AWAIT" and (op.get("value") or {}).get("kind") == "result"}
            awaiting.update({decorated[key]: awaiting[key] for key in decorated.keys() & awaiting.keys()})
            for operation in block["steps"]:
                kind = operation["kind"]
                if operation["site"]["start"] < cutoff:
                    continue
                if kind == "OPEN":
                    bounded = True
                metadata = {}
                if kind == "CALL":
                    if operation.get("role") == "AWAITER_CONFIGURATION":
                        continue
                    if operation.get("implicit"):
                        continue
                    metadata = {key: operation.get(key) for key in ("spelling", "symbol")}
                    if await_site := awaiting.get(operation["id"]):
                        metadata["guardAwaitStep"] = self.step(fn, f"op{await_site['id']}", "AWAIT", await_site["site"])
                elif kind != "WRITE" or (operation.get("target") or {}).get("kind") != "member":
                    continue
                key = self.step(fn, f"op{operation['id']}", kind, operation["site"])
                for name, value in metadata.items():
                    self.steps[key].setdefault(name, value)
                reachable.append(key)
                if len(reachable) > MAX_SKIPPED:
                    return reachable[:MAX_SKIPPED], True
            queue.extend((block["next"], block["conditional"]))
        return reachable, bounded

    def direct_effect_inventory(self, ref):
        """Classify reachable direct calls without treating callee execution as direct work."""
        fn = self.functions[ref]
        sites = []
        limited = projection_open = False
        for block in fn.graph["blocks"]:
            if not block["reachable"]:
                continue
            for op in block["steps"]:
                if op["kind"] not in {"CALL", "CONSTRUCT", "OPEN"} or op.get("implicit"):
                    continue
                if op["kind"] == "OPEN":
                    projection_open = True
                family = op.get("directEffectFamily")
                if family == "local" or op.get("httpResult") or op.get("responseConstruction"):
                    continue
                evidence = self.roles.get((ref, op["id"]), {})
                if family:
                    classification = "BOUND_API"
                elif op.get("binding"):
                    classification = "SOURCE_CALL"
                else:
                    classification = "OPEN"
                    family = {"PUBLICATION_CANDIDATE": "broker", "SQL_READ_CANDIDATE": "database",
                              "SQL_WRITE_CANDIDATE": "database"}.get(evidence.get("role"))
                    if evidence.get("sourceSql"):
                        family = "database"
                if len(sites) >= 24:
                    limited = True
                    continue
                key = self.step(fn, f"op{op['id']}", op["kind"], op["site"],
                                spelling=op.get("spelling"), symbol=op.get("symbol"))
                self.steps[key]["sources"] = list(dict.fromkeys([*self.steps[key]["sources"], *evidence.get("sources", ())]))
                sites.append({"step": key, "classification": classification, **({"family": family} if family else {})})
        if not sites and not any(block["reachable"] and block["kind"] == "Exit" for block in fn.graph["blocks"]):
            return None
        complete = not (limited or projection_open)
        closed = complete and not any(row["classification"] == "OPEN" for row in sites)
        return {"sites": sites, "status": "CLOSED_DIRECT_SCOPE" if closed else "OPEN",
                "siteCoverage": "COMPLETE" if complete else "PARTIAL", "limited": limited,
                "sdkImports": fn.graph.get("sdkImports", False), "sources": [fn.citation]}

    def inventory_witnesses(self, fn):
        """Single ordinary CFG witness per reachable block, independent of runtime replay."""
        witnesses = {0: ()}
        queue = deque([0])
        while queue:
            block = fn.graph["blocks"][queue.popleft()]
            for conditional, edge in ((False, block["next"]), (True, block["conditional"])):
                if not edge or (target := edge["target"]) is None or target in witnesses:
                    continue
                if not fn.graph["blocks"][target]["reachable"]:
                    continue
                prefix = witnesses[block["id"]]
                if (block["decision"] or {}).get("sourcePredicate"):
                    key = self.condition_step(fn, block)
                    prefix = (*prefix, {"condition": key, "when": conditional == (block["branch"] == "WhenTrue")})
                witnesses[target] = prefix
                queue.append(target)
        return witnesses

    def composed_effect_inventory(self, root):
        """Enumerate explicit sites across selected reachable declarations, even when replay is open."""
        queue, entries, scopes = deque([root]), {root: []}, []
        remaining = MAX_COMPOSED_EFFECT_SITES
        while queue:
            ref = queue.popleft()
            fn = self.functions[ref]
            witnesses = self.inventory_witnesses(fn)
            scope = {"id": fn.citation, "symbol": fn.graph["symbol"], "entrySites": entries[ref],
                     "siteCoverage": "COMPLETE" if fn.graph.get("explicitCallSitesComplete") else "PARTIAL", "sites": []}
            scopes.append(scope)
            for block in fn.graph["blocks"]:
                if not block["reachable"]:
                    continue
                for op in block["steps"]:
                    kind = op["kind"]
                    if kind not in {"CALL", "CONSTRUCT", "OPEN"} or op.get("implicit"):
                        continue
                    family = op.get("directEffectFamily")
                    if family == "local":
                        continue
                    if remaining == 0:
                        scope["siteCoverage"] = "PARTIAL"
                        continue
                    remaining -= 1
                    key = self.step(fn, f"op{op['id']}", kind, op["site"], spelling=op.get("spelling"), symbol=op.get("symbol"))
                    item = {"step": key, "block": block["id"], "region": block["region"],
                            "classification": "OPAQUE", "witness": witnesses.get(block["id"], ())[-3:]}
                    scope["sites"].append(item)
                    binding = op.get("binding") or op.get("candidate") or op.get("inventoryTarget")
                    if binding:
                        target = self.bindings.get((binding["file"], binding["start"], binding["symbol"]))
                        if target:
                            item.update(classification="SELECTED_SOURCE", callee=self.functions[target].citation,
                                        dispatch="STATIC" if op.get("binding") else "CONDITIONAL")
                            if target not in entries:
                                entries[target] = [*entries[ref], key]
                                queue.append(target)
                        else:
                            item["classification"] = "UNSELECTED_PROJECT_SOURCE"
                            scope["siteCoverage"] = "PARTIAL"
                    elif op.get("httpResult") or op.get("responseConstruction"):
                        item["classification"] = "RESPONSE_CONSTRUCTION"
                    elif family:
                        item.update(classification="BOUND_API", family=family)
                    elif kind == "OPEN":
                        item["classification"] = "OPEN_EVALUATION"
                    else:
                        evidence = self.roles.get((ref, op["id"]), {})
                        candidate = op.get("candidateEffectFamily") or {
                            "PUBLICATION_CANDIDATE": "broker", "SQL_READ_CANDIDATE": "database",
                            "SQL_WRITE_CANDIDATE": "database"}.get(evidence.get("role"))
                        if not candidate and evidence.get("sourceSql"):
                            candidate = "database"
                        if candidate:
                            item.update(classification="CANDIDATE_API", family=candidate)
                            self.steps[key]["sources"] = list(dict.fromkeys([*self.steps[key]["sources"], *evidence.get("sources", ())]))
                        elif op.get("inSource") and op.get("dispatch") != "OPEN":
                            item["classification"] = "UNSELECTED_PROJECT_SOURCE"
                            scope["siteCoverage"] = "PARTIAL"
        if len(scopes) == 1 and not any(item["classification"] == "UNSELECTED_PROJECT_SOURCE" for item in scopes[0]["sites"]):
            return None
        complete = all(scope["siteCoverage"] == "COMPLETE" for scope in scopes)
        return {"siteCoverage": "COMPLETE" if complete else "PARTIAL", "scopes": scopes}

    def helper_models(self, root):
        """Retain a bounded local CFG for reachable selected helper candidates."""
        queue, seen, models, sites = [root], {root}, [], set()
        for ref in queue:
            fn = self.functions[ref]
            blocks = []
            for block in fn.graph["blocks"]:
                if not block["reachable"]:
                    continue
                operations = []
                for op in block["steps"]:
                    if op["kind"] == "CALL":
                        target, binding = self.resolve(op)
                        if target and target not in seen:
                            queue.append(target)
                            seen.add(target)
                    if ref == root or len(blocks) >= MAX_HELPER_BLOCKS:
                        continue
                    key = self.step(fn, f"op{op['id']}", op["kind"], op["site"])
                    sites.add(key)
                    details = {k: v for k, v in op.items() if k not in {"id", "site", "kind", "binding", "candidate"}
                               and v is not None}
                    if op["kind"] == "CALL" and target:
                        details["helper"] = self.functions[target].citation
                        details["binding"] = binding
                    operations.append({"step": key, **details})
                if ref == root or len(blocks) >= MAX_HELPER_BLOCKS:
                    continue
                entry = {"id": block["id"], "region": block["region"], "kind": block["kind"], "operations": operations}
                if block["decision"]:
                    condition = self.step(fn, f"decision{block['id']}", "CFG_DECISION", block["decision"]["site"])
                    sites.add(condition)
                    entry["decision"] = {"step": condition, "value": block["decision"]["value"]}
                for field in ("branch", "next", "conditional", "throwType"):
                    if block[field]:
                        entry[field] = block[field]
                blocks.append(entry)
            if ref == root:
                continue
            if len(fn.graph["blocks"]) > MAX_HELPER_BLOCKS:
                self.boundaries.add("HELPER_CFG_BLOCK_BUDGET")
            models.append({"id": fn.citation, "symbol": fn.graph["symbol"], "sources": [fn.citation],
                           "blocks": blocks, "regions": [
                               {key: value for key, value in region.items() if key != "site" and value is not None}
                               for region in fn.graph["regions"]]})
            if len(models) >= MAX_HELPER_MODELS:
                if len(queue) > len(models) + 1:
                    self.boundaries.add("HELPER_CFG_COUNT_BUDGET")
                break
        return models, sites

    def skipped(self, paths):
        rows = []
        for row, _ in paths:
            related = {}
            reached = {event["step"] for event in row["sequence"]}
            if row["outcome"] != "OPEN" and not row["open"]:
                for other, _ in paths:
                    if row is other or other["outcome"] == "OPEN" or other["open"]:
                        continue
                    left = [(e["step"], e["outcome"]) for e in row["sequence"]]
                    right = [(e["step"], e["outcome"]) for e in other["sequence"]]
                    different = next(((a, b) for a, b in zip(left, right) if a != b), None)
                    if not different or different[0][0] != different[1][0]:
                        continue
                    pivot = different[0][0]
                    if self.steps[pivot]["kind"] not in {"CONDITION", "CALL", "AWAIT", "CONSTRUCT"}:
                        continue
                    for step, _ in right:
                        if step not in reached and self.steps[step]["kind"] in EFFECTS and not self.steps[step].get("implicit"):
                            related.setdefault((pivot, step), other["id"])
            if len(related) > MAX_SKIPPED:
                self.boundaries.add("SKIPPED_EFFECT_BUDGET")
            row["skippedOnThisPath"] = [{"after": pivot, "step": step, "alternative": path}
                                        for (pivot, step), path in list(related.items())[:MAX_SKIPPED]]
            row["sources"] = list(dict.fromkeys(source for step in [*reached, *(step for _, step in related)]
                                               for source in self.steps[step]["sources"]))
            row["sources"].sort()
            rows.append(row)
        return rows

    def interruptions(self, paths, command_paths):
        results = []
        command_writes = {(path["path"], event["completion"]): event for path in command_paths
                          for event in path["commands"] if event["kind"] == "WRITE"}
        for path in paths:
            completed = []
            for event in path["sequence"]:
                call = self.steps.get(event.get("call"), {})
                if call.get("role") in self.storage["EXTERNAL"] and event["outcome"] in {"RESUMED", "RETURNED"}:
                    completed.append((call["id"], event["step"]))
                if not completed or event["outcome"] not in {"FAULT_OR_CANCELLATION", "CALL_EXCEPTION", "THROWN"}:
                    continue
                skipped = [entry["step"] for entry in path["skippedOnThisPath"] if entry["after"] == event["step"]]
                command = command_writes.get((path["id"], event["step"]))
                results.append({
                    "path": path["id"], "completedExternalRequests": [
                        {"call": request, "completedAt": completion, "role": self.steps[request]["role"],
                         "authority": self.steps[request]["roleAuthority"]} for request, completion in completed],
                    "interruption": event["step"], "interruptionOutcome": event["outcome"],
                    "persistence": {
                        "PERSISTENCE_REQUEST": "ATTEMPT_INTERRUPTED",
                        "SQL_WRITE_CANDIDATE": "SQL_WRITE_CANDIDATE_INTERRUPTED",
                        "TRANSACTION_COMMIT_REQUEST": "COMMIT_ATTEMPT_INTERRUPTED",
                    }.get(call.get("role"), "COMMAND_WRITE_ATTEMPT_INTERRUPTED" if command else "OPEN"),
                    "skippedOnThisPath": list(dict.fromkeys(skipped)),
                    "sources": sorted({source for key in [event["step"], *skipped, *(request for request, _ in completed)]
                                       for source in self.steps[key]["sources"]}),
                })
                if len(results) >= MAX_PATHS:
                    self.boundaries.add("INTERRUPTION_FACT_BUDGET")
                    return results
        return results

    def open_await_ordering(self, paths):
        result = {}
        for path in paths:
            prior = None
            for event in path["sequence"]:
                call = self.steps.get(event.get("call"), {})
                if event["outcome"] == "RESUMED" and call.get("role") is None:
                    prior = event
                if not prior or event["outcome"] not in self.storage["INTERRUPTED"]:
                    continue
                key = prior["step"], event["step"]
                result.setdefault(key, {
                    "path": path["id"], "awaitResume": key[0], "laterInterruption": key[1],
                    "priorCall": prior.get("call"), "interruptedCall": event.get("call"),
                    "externalEffect": "OPEN", "durableEligibility": "OPEN",
                    "sources": sorted({source for step in key for source in self.steps[step]["sources"]}),
                })
                if len(result) >= 10:
                    return list(result.values())
        return list(result.values())

    def repeated(self, paths):
        """Contrast a same-state guard and a deferred write across modeled outcomes."""
        candidates = {}
        for failure, failed in paths:
            if failure["outcome"] == "OPEN" or failed.uncertainties:
                continue
            events = failure["sequence"]
            for index, event in enumerate(events):
                if event["outcome"] not in {"FAULT_OR_CANCELLATION", "CALL_EXCEPTION", "THROWN"}:
                    continue
                for effect in events[:index]:
                    original = self.steps.get(effect.get("call"), {})
                    if original.get("role") != "EXTERNAL_REQUEST" or effect["outcome"] not in {"RESUMED", "RETURNED"}:
                        continue
                    for condition in failed.conditions:
                        guard = condition["guard"]
                        if not guard or guard_reached_after(events[:index], condition["step"], effect["step"]):
                            continue
                        resource, admitted = guard
                        # Require an actual opposite guard-return arm that skips
                        # this call, rather than any preceding state condition.
                        if not any(
                                alternate["outcome"] == "RETURNED" and not before.uncertainties
                                and any(item["step"] == condition["step"] and item["guardReturn"]
                                        and item["guard"] == (resource, not admitted)
                                        for item in before.conditions)
                                and original["id"] not in {event["step"] for event in alternate["sequence"]}
                                for alternate, before in paths):
                            continue
                        if any(write[1] == resource for write in failed.writes):
                            continue
                        prefix = events[:index]
                        for normal, success in paths:
                            if normal["outcome"] not in {"NORMAL_EXIT", "RETURNED"} or success.uncertainties:
                                continue
                            other = normal["sequence"]
                            if prefix != other[:index]:
                                continue
                            written = next((item for item in reversed(success.writes) if item[1] == resource), None)
                            if not written or written[2] != literal(not admitted) or written[0] in {e["step"] for e in other[:index]}:
                                continue
                            steps = (condition["step"], original["id"], effect["step"], event["step"], written[0])
                            candidates.setdefault(steps, {
                                "kind": "POSSIBLE_REPEATED_REQUEST", "guard": steps[0],
                                "externalCall": steps[1], "completedAt": steps[2], "interruption": steps[3],
                                "guardWriteNotReached": steps[4], "guardResource": resource,
                                "failurePath": failure["id"], "comparisonPath": normal["id"],
                                "condition": "A subsequent invocation independently reaches the same call with the same guard resource still admitting it.",
                                "sources": sorted({source for key in steps for source in self.steps[key]["sources"]}),
                            })
        return list(candidates.values())


def compact_paths_and_events(paths):
    """Share trace prefixes while keeping each alternative independently reconstructable."""
    events, identities, compact = [], {}, []
    for path in paths:
        previous = None
        for event in path["sequence"]:
            key = previous, json.dumps(event, sort_keys=True)
            if key not in identities:
                identity = len(events) + 1
                identities[key] = identity
                events.append({"id": identity, **({"previous": previous} if previous else {}), **event})
            previous = identities[key]
        compact.append({**{key: value for key, value in path.items() if key != "sequence"}, "lastEvent": previous})
    return compact, events


def opaque_member(step, receiver):
    return receiver is not None and step.get("dispatch") == "OPEN" and not step.get("symbol")


def argument_values(arguments, frame):
    values = []
    for arg in arguments:
        if arg["id"] is None:
            continue
        evidence = frame.cancellation_parameters.get(arg.get("unboundTokenInput")) if not arg.get("byRef") else None
        values.append({"position": arg["id"], "value": symbol(arg["value"], frame),
                       **({"name": arg["name"]} if arg.get("name") else {}),
                       **{key: True for key in ("cancellationToken", "byRef") if arg.get(key)},
                       **({"cancellationToken": True, "cancellationTokenEvidence": evidence} if evidence else {})})
    return values


def guard_reached_after(events, guard, effect):
    sites = [event["step"] for event in events]
    return guard not in sites or effect not in sites or sites.index(guard) >= sites.index(effect)
