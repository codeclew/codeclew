"""Project closed switch expressions and their local guarded continuations."""

from dataclasses import dataclass
import json
import re


NAME = r"[A-Za-z_][A-Za-z0-9_]*"
SCALAR = r'"[^"\\\x00-\x1f]{0,64}"|-?(?:0|[1-9][0-9]{0,8})|true|false|null'
TYPE = rf"(?:global::)?{NAME}(?:\.{NAME}){{0,5}}"
ARM = re.compile(rf"\s*(?P<keys>_|(?:{SCALAR})(?:\s+or\s+(?:{SCALAR}))*)\s*=>\s*")
VALUE = re.compile(SCALAR)
ARM_END = re.compile(r"\s*(,|$)")
LOOKUP_PREDICATE = re.compile(rf"({NAME})\s*=>\s*({NAME})\s*==\s*({NAME})")
NULL_CHECK = re.compile(rf"({NAME})\s*(is\s+not\s+|is\s+|==\s*|!=\s*)null")
STRING_COLLECTION = re.compile(
    r"string\[\]|System\.Collections\.Generic\.(?:List|IList|IEnumerable|IReadOnlyList|ICollection|IReadOnlyCollection)<string\??>")
FORMAL_TYPES = {"System.String": str, "System.Int32": int, "System.Boolean": bool}
PATH_PREMISES = {"LIST_SELECTION_CARDINALITY_OR_BINDING_OPEN", "ASSUMED_SDK_GLOBAL_IMPORTS"}
INTERRUPTIONS = {"CALL_EXCEPTION", "FAULT_OR_CANCELLATION", "CONSTRUCTOR_EXCEPTION", "THROWN"}
SWITCH_FAILURE = "M:System.Runtime.CompilerServices.SwitchExpressionException.#ctor"


@dataclass(frozen=True)
class Construction:
    text: str
    step: str
    object_type: str


@dataclass(frozen=True)
class Mapping:
    call: str
    invocation: int
    returned: str
    actual: str
    formal: str
    arms: tuple
    fallback: object

    def values(self):
        return (*(value for _, value in self.arms), self.fallback)

    def constructors(self):
        return tuple(value.step for value in self.values() if isinstance(value, Construction))


@dataclass(frozen=True)
class Guard:
    mapping: Mapping
    index: int
    truth: bool
    tests_missing: bool
    bindings: tuple
    lookup: str | None
    predicate: str | None
    constructed: bool


def switch_expression(text, constructors):
    if not text or not (match := re.fullmatch(rf"\s*(?:return\s+)?({NAME})\s+switch\s*\{{(.*)\}}\s*;?\s*", text, re.DOTALL)):
        return None
    formal, source = match.groups()
    cursor, arms, keys = 0, [], set()
    while len(arms) < 8 and (arm := ARM.match(source, cursor)):
        pattern, = arm.groups()
        start = arm.end()
        offset = len(text[:match.start(2) + start].encode())
        if value := constructors.get(offset):
            if not source.startswith(value.text, start):
                return None
            end = start + len(value.text)
        elif scalar := VALUE.match(source, start):
            value, end = json.loads(scalar.group()), scalar.end()
        else:
            return None
        if not (tail := ARM_END.match(source, end)):
            return None
        comma, = tail.groups()
        cursor = tail.end()
        if pattern == "_":
            return (formal, tuple(arms), value) if arms and not source[cursor:].strip() else None
        selected = tuple(json.loads(key.group()) for key in VALUE.finditer(pattern))
        identities = {(type(key), key) for key in selected}
        if len(identities) != len(selected) or identities & keys or not comma:
            return None
        keys.update(identities)
        arms.append((selected, value))
    return None


def literal(facts, key):
    step = facts.steps[key]
    return facts.sources.literal(step["site"], step["sources"])


def actual_visible(value):
    if value[0] in {"parameter", "literal"}:
        return True
    return value[0] == "member" and value[2] and actual_visible(value[2])


def snapshot_value(value):
    if isinstance(value, (tuple, list)):
        return tuple(snapshot_value(item) for item in value)
    return value


def contains(outer, inner):
    return (inner and (outer["file"], outer["revision"]) == (inner["file"], inner["revision"])
            and outer["startByte"] <= inner["startByte"] < inner["endByte"] <= outer["endByte"])


def constructor_spans(facts, returned, operations):
    parent = facts.steps[returned]["site"]
    constructors = {}
    for operation in operations:
        key = operation["step"]
        step = facts.steps[key]
        if (step["kind"] != "CONSTRUCT" or step.get("unresolved") or not step.get("objectType")
                or ".#ctor" not in (step.get("symbol") or "") or not contains(parent, site := step["site"])
                or not (text := literal(facts, key)) or not re.match(r"new(?=\s|\()", text)):
            continue
        offset = site["startByte"] - parent["startByte"]
        if offset in constructors:
            return {}
        constructors[offset] = Construction(text, key, step["objectType"])
    return constructors


def confined_operations(facts, operations, mapping):
    # Argument evaluation may call other code. Its effect is outside the mapping proof.
    parent = facts.steps[mapping.returned]["site"]
    spans = [facts.steps[key]["site"] for key in mapping.constructors()]
    for operation in operations:
        key = operation["step"]
        step = facts.steps[key]
        if step["kind"] == "BIND" or any(contains(span, step["site"]) for span in spans):
            continue
        if (operation.get("symbol") != SWITCH_FAILURE or not contains(parent, step["site"])
                or not re.match(rf"{NAME}\s+switch\b", literal(facts, key) or "")):
            return False
    return True


def retained_mapping(facts, helpers, events, completed, value_text):
    event = events[completed]
    call = event["step"]
    if (not completed or event["outcome"] != "RETURNED" or event.get("composed") != "ROSLYN_STATIC_BINDING"
            or call.split("/", 1)[0] not in facts.root_refs):
        return None
    step = facts.steps[call]
    helper_ids = step.get("boundSources", ())
    helper = helpers.get(helper_ids[0]) if len(helper_ids) == 1 else None
    if not helper or not (type_ := FORMAL_TYPES.get(helper["symbol"].rpartition("(")[2].removesuffix(")"))):
        return None
    returned = events[completed - 1]["step"]
    if facts.steps[returned]["kind"] != "RETURN" or not returned.startswith(helper["id"] + "/"):
        return None
    operations = [operation for block in helper["blocks"] for operation in block["operations"]]
    source = literal(facts, returned)
    if not source or not (parsed := switch_expression(source, constructor_spans(facts, returned, operations))):
        return None
    formal, arms, fallback = parsed
    if any(key is not None and type(key) is not type_ or key is None and type_ is not str for keys, _ in arms for key in keys):
        return None
    inputs = [operation["value"] for operation in operations if isinstance(operation.get("value"), dict)
              and operation["value"]["kind"] == "parameter" and "capture" in operation]
    if not inputs or any(item["name"] != formal or item["id"] != 0 for item in inputs):
        return None
    entry = next((position for position in range(completed - 1, -1, -1) if events[position]["step"] == call), None)
    invoked = events[entry] if entry is not None else {}
    arguments = invoked.get("arguments", ()) if invoked.get("outcome") == "INVOKED" else ()
    if len(arguments) != 1 or arguments[0].get("name") != formal or arguments[0].get("position") != 0:
        return None
    actual = arguments[0].get("value")
    if not actual or not actual_visible(actual) or (shown := value_text(actual)) is None:
        return None
    mapping = Mapping(call, invoked["id"], returned, shown, formal, arms, fallback)
    if not confined_operations(facts, operations, mapping):
        return None
    result = snapshot_value(event.get("returnedValue"))
    if result and result[0] == "literal" and any(
            type(result[1]) is type(value) and result[1] == value for value in mapping.values()):
        return mapping
    if result and result[0] == "allocation" and any(
            item["step"] in mapping.constructors() and item["outcome"] == "CONSTRUCTED"
            and snapshot_value(item.get("result")) == result for item in events[entry + 1:completed - 1]):
        return mapping
    return None


def local_assignment(facts, event, expression):
    name = event.get("name")
    if not name or not re.fullmatch(NAME, name) or event["outcome"] != "LOCAL_BINDING":
        return None
    source = literal(facts, event["step"])
    rhs = literal(facts, expression)
    if not source or not rhs:
        return None
    if re.fullmatch(rf"(?:{TYPE}\??\s+)?{name}\s*=\s*{re.escape(rhs)}", source):
        return name
    return None


def lookup_predicate(facts, event, mapped, mapping):
    step = facts.steps[event["step"]]
    if (event["outcome"] != "INVOKED" or step.get("valueFactory") != "LINQ_FIRST_OR_DEFAULT"
            or not step.get("nullDefault") or len(event.get("arguments", ())) != 2
            or any(type(value) not in {str, type(None)} for _, value in mapping.arms)
            or type(mapping.fallback) not in {str, type(None)}):
        return None
    source = next((argument.get("value") for argument in event["arguments"] if argument.get("name") == "source"), None)
    if not source or source[0] != "parameter" or not any(
            item["id"] == source[1] and item["name"] == source[2] and STRING_COLLECTION.fullmatch(item.get("type") or "")
            for item in facts.root.get("inputs", ())):
        return None
    text = facts.sources.text(step.get("predicateSite"), step["sources"])
    pattern = LOOKUP_PREDICATE.fullmatch(text) if text else None
    if pattern and pattern[1] != mapped and {pattern[2], pattern[3]} == {pattern[1], mapped}:
        return text
    return None


def guarded_result(facts, events, completed, mapping, path):
    index = completed + 1
    if index >= len(events) or not (local := local_assignment(facts, events[index], mapping.call)):
        return None
    bindings = [events[index]["step"]]
    value = snapshot_value(events[index].get("value"))
    if value != snapshot_value(events[completed]["returnedValue"]):
        return None
    index += 1
    if index >= len(events):
        return None
    lookup, predicate = None, None
    if facts.steps[events[index]["step"]]["kind"] != "CONDITION":
        predicate = lookup_predicate(facts, events[index], local, mapping)
        if not predicate or index + 3 >= len(events):
            return None
        lookup = events[index]["step"]
        finished, result = events[index + 1:index + 3]
        if finished.get("call") != lookup or finished["outcome"] != "RETURNED":
            return None
        if not (local := local_assignment(facts, result, lookup)):
            return None
        bindings.append(result["step"])
        value = snapshot_value(result.get("value"))
        index += 3
        if not value or value[:2] != ("call-result", lookup) and value != ("literal", None):
            return None
    condition = events[index]
    key, outcome = condition["step"], condition["outcome"]
    if outcome not in {"TRUE", "FALSE"} or facts.steps[key]["kind"] != "CONDITION":
        return None
    text = facts.text(key)
    check = NULL_CHECK.fullmatch(text) if text else None
    if not check or check[1] != local:
        return None
    negated = check[2].split() in [["is", "not"], ["!="]]
    operator = "different" if negated else "equal"
    comparisons = [snapshot_value(row.get("operand")) for row in path["conditions"] if row["step"] == key]
    if comparisons not in [[(operator, value, ("literal", None))], [(operator, ("literal", None), value)]]:
        return None
    truth = outcome == "TRUE"
    return Guard(mapping, index, truth, truth != negated, tuple(bindings), lookup, predicate, value[0] == "allocation")


def selected_effects(facts, path, trace, index, skipped):
    actions = []
    for at, _, keys in facts.actions(path, trace):
        if at <= index:
            continue
        key = next((key for key in keys if key not in skipped and facts.steps[key].get("symbol")
                    and facts.steps[key].get("roleAuthority") == "ROSLYN_FRAMEWORK_DECLARATION"
                    and facts.steps[key]["kind"] == "CALL"), None)
        if key and (text := facts.text(key)):
            actions.append((text, keys))
        if len(actions) == 3:
            break
    return actions


def interrupted(facts, events):
    return any(event["outcome"] in INTERRUPTIONS or facts.steps[event["step"]]["kind"] == "CATCH" for event in events)


def mapping_text(facts, mapping, code):
    def render(value):
        if isinstance(value, Construction):
            return code(value.text) + " (constructs " + code(value.object_type) + ")"
        return code(json.dumps(value, ensure_ascii=False))

    cases = [" or ".join(render(key) for key in keys) + " → " + render(value) for keys, value in mapping.arms]
    text = (f"{facts.name(mapping.call)} binds {code(mapping.actual)} to the switch input {code(mapping.formal)}. "
            "The retained switch cases are " + ", ".join(cases) + f". The default result is {render(mapping.fallback)}.")
    if mapping.constructors():
        text += (" Each constructed arm yields a value if its arguments evaluate and its constructor completes normally."
                 " Effects inside construction expressions remain open.")
    return text


def project(facts, traces, is_http, code, value_text):
    helpers = {item["id"]: item for item in facts.graph.get("retainedHelpers", ())}
    if not helpers:
        return
    guarded = []
    for path_id, trace in traces.items():
        path = facts.paths[path_id]
        if path["outcome"] != "RETURNED" or not set(path["open"]) <= PATH_PREMISES:
            continue
        for index in range(len(trace)):
            if not (mapping := retained_mapping(facts, helpers, trace, index, value_text)):
                continue
            proof = [mapping.call, mapping.returned, *mapping.constructors()]
            facts.add_path("finite-mapping", mapping_text(facts, mapping, code), path, trace, proof)
            if guard := guarded_result(facts, trace, index, mapping, path):
                guarded.append((guard, path, trace))
    for guard, path, trace in guarded:
        if not guard.tests_missing:
            continue
        index = guard.index
        key = trace[index]["step"]
        root_return = facts.root_return(path, trace, shallow=True)
        if not root_return or interrupted(facts, trace[index + 1:]):
            continue
        label, returns = root_return
        return_step = returns[-1]
        if not is_http:
            label, returns = "a return from the selected callable", [return_step]
        if facts.steps[return_step]["site"]["startByte"] <= facts.steps[key]["site"]["endByte"]:
            continue
        mapping = guard.mapping
        proof = [mapping.call, mapping.returned, *mapping.constructors(), *guard.bindings, key, *returns]
        text = f"The value from {facts.name(mapping.call)} is checked for null"
        if guard.lookup:
            text = (f"The value from {facts.name(mapping.call)} supplies the string equality lookup "
                    + code(guard.predicate))
            proof.append(guard.lookup)
        outcome = f"the handler returns an {label}" if label.startswith("HTTP ") else "the callable returns"
        condition = code(facts.text(key)) + (" is true" if guard.truth else " is false")
        text += f". When {condition}, {outcome}."
        reached = {event["step"] for event in trace}
        alternate, skipped, constructed = [], [], False
        for other, continuing, normal in guarded:
            if (other.tests_missing or other.mapping != mapping or other.lookup != guard.lookup or other.bindings != guard.bindings
                    or normal[other.index]["step"] != key or not (finished := facts.root_return(continuing, normal, shallow=True))
                    or interrupted(facts, normal[other.index + 1:])
                    or return_step == finished[1][-1] or guard.lookup and trace[:index] != normal[:other.index]):
                continue
            skipped = selected_effects(facts, continuing, normal, other.index, reached)
            if skipped:
                alternate.append(continuing["id"])
                constructed = other.constructed
                break
        if skipped:
            text += (" This return bypasses " + ", ".join(code(label) for label, _ in skipped)
                     + f", which the normal continuation can reach when {code(facts.text(key))} "
                     + ("is false" if guard.truth else "is true")
                     + (" after the selected constructor completes normally." if constructed else "."))
            proof.extend(key for _, steps in skipped for key in steps)
        if guard.lookup:
            text += " The collection contents remain open."
        guards = facts.path_condition(trace)
        facts.add_path("lookup-guard-return", text, path, trace, [*proof, *(step for _, step in guards)], alternate,
                       condition=" and ".join(dict.fromkeys([*(label for label, _ in guards), condition])),
                       **({"_httpStatus": facts.http_status(returns)} if is_http else {}))
