"""Derive concise C# author facts from sealed excerpts and bounded Roslyn paths."""

from collections import defaultdict
from hashlib import sha256
import json
from pathlib import Path
import re
import runpy


COMPLETED = {"RETURNED", "RESUMED", "CONSTRUCTED"}
TERMINAL = {"RETURNED", "NORMAL_EXIT", "THREW"}
INTERRUPTED = {"CALL_EXCEPTION": "throws", "FAULT_OR_CANCELLATION": "faults or is cancelled"}
LIST_SELECTION_OPEN = "LIST_SELECTION_CARDINALITY_OR_BINDING_OPEN"
EFFECT_ROLES = {
    "EXTERNAL_REQUEST", "DECLARED_EXTERNAL_CALL", "PUBLICATION_CANDIDATE", "PERSISTENCE_REQUEST",
    "TRANSACTION_BEGIN_REQUEST", "TRANSACTION_COMMIT_REQUEST", "TRANSACTION_ROLLBACK_REQUEST",
    "SQL_READ_CANDIDATE", "SQL_WRITE_CANDIDATE", "ADO_EXECUTE_READER", "ADO_EXECUTE_SCALAR", "ADO_EXECUTE_WRITE",
}
HTTP_CLIENT_METHODS = {"GetAsync": "GET", "GetStringAsync": "GET", "GetByteArrayAsync": "GET",
                       "GetStreamAsync": "GET", "PostAsync": "POST", "PutAsync": "PUT", "DeleteAsync": "DELETE",
                       "PatchAsync": "PATCH"}
DIRECT_EFFECT_FAMILIES = ("HTTP", "database", "broker", "file system", "process")
LIMIT = 32
MAPPINGS = runpy.run_path(str(Path(__file__).with_name("clew_prepared_managed_mappings.py")))["project"]


def code(text):
    text = text.replace("\r", "\\r").replace("\n", "\\n")
    ticks = re.findall(r"`+", text)
    if not ticks:
        return f"`{text}`"
    fence = "`" * (max(map(len, ticks)) + 1)
    return f"{fence} {text} {fence}"


def method(symbol):
    return symbol.split("(", 1)[0].removeprefix("M:").rsplit(".", 1)[-1].split("``", 1)[0]


def value_text(value, depth=0):
    if not value or depth > 3:
        return None
    kind, *data = value
    if kind == "literal":
        scalar = json.dumps(data[0], ensure_ascii=False)
        return scalar if len(scalar) <= 80 and "`" not in scalar else None
    if kind in {"parameter", "captured-parameter"}:
        return data[1] if kind == "parameter" else data[2]
    if kind in {"member", "open-member"}:
        receiver = value_text(data[1], depth + 1)
        name = data[0].rsplit(".", 1)[-1]
        if data[1] and data[1][0] in {"entry-receiver", "static"}:
            return name
        return f"{receiver}.{name}" if receiver else None
    if kind == "tuple":
        items = [value_text(item, depth + 1) for item in data[0]]
        if items and len(items) <= 4 and all(items):
            return "(" + ", ".join(items) + ")"
    if kind == "record" and 0 < len(data[0]) <= 8:
        fields = []
        for name, item in data[0]:
            field = value_text(item, depth + 1)
            fields.append(f"{name} = {field}" if field else name)
        return ", ".join(fields)
    return None


def null_literal(value):
    return value is not None and tuple(value) == ("literal", None)


def relates(value, target):
    return value == target or isinstance(value, (tuple, list)) and any(relates(item, target) for item in value)


class Sources:
    def __init__(self, context):
        self.rows = {row["id"]: row for row in context.get("sourceRanges", ())
                     if re.fullmatch(r"S[0-9]{3,}", row["id"]) and row["file"].endswith(".cs")}
        self.site_texts = {}
        self.lines = {}

    def text(self, site, refs):
        value = self.literal(site, refs)
        exact = " ".join(value.split()) if value else None
        return exact if exact and len(exact) <= 160 and "\x60" not in exact else None

    def literal(self, site, refs):
        if not site:
            return None
        key = site["file"], site["revision"], site["startByte"], site["endByte"], site["textDigest"], tuple(sorted(refs))
        if key not in self.site_texts:
            self.site_texts[key] = self.find(site, refs)
        return self.site_texts[key]

    def find(self, site, refs):
        length = site["endByte"] - site["startByte"]
        first, last = site["startLine"], site["endLine"]
        if not 0 < length <= 320:
            return None
        for ref in refs:
            row = self.rows.get(ref)
            if not row or row["file"] != site["file"] or row["revision"] != site["revision"] or not (
                    row["startLine"] <= first <= last <= row["endLine"]):
                continue
            row = self.rows.get(row["textIn"]) if "textIn" in row else row
            if not row or not isinstance(row.get("text"), str):
                continue
            if row["id"] not in self.lines:
                self.lines[row["id"]] = row["text"].encode().split(b"\n")
            lines = self.lines[row["id"]][first - row["startLine"]:last - row["startLine"] + 1]
            if not lines or len(lines[0]) > 4096:
                continue
            data = b"\n".join(lines)
            through = len(data) - len(lines[-1])
            matches = [data[start:start + length] for start in range(max(0, through + 1 - length),
                       min(len(lines[0]), len(data) - length + 1))
                       if sha256(data[start:start + length]).hexdigest() == site["textDigest"].removeprefix("sha256:")]
            if matches:
                return matches[0].decode()
        return None


class Facts:
    def __init__(self, context, graph):
        self.sources = Sources(context)
        self.graph = graph
        self.steps = {row["id"]: row for row in graph.get("steps", ())}
        self.events = {row["id"]: row for row in graph.get("sequenceEvents", ())}
        self.paths = {row["id"]: row for row in graph.get("paths", ())}
        self.local_paths = graph.get("boundedCallablePaths", ())
        self.local_by_id = {row["id"]: row for row in self.local_paths}
        self.root = graph.get("root", {})
        self.root_refs = set(self.root.get("sources", ()))
        self.helpers = {row["id"]: method(row["symbol"]) for row in graph.get("retainedHelpers", ())}
        self.commands = {row["path"]: row["commands"] for row in graph.get("relationalCommandPaths", ())}
        self.local_commands = {row["path"]: row["commands"] for row in graph.get("boundedCallableCommands", ())}
        self.transactions = defaultdict(list)
        for row in (*graph.get("httpTransactionOutcomes", ()), *graph.get("transactionalWriteCompletions", ())):
            self.transactions[row["path"]].append(row)
        self.rows = defaultdict(dict)

    def add(self, kind, text, steps=(), paths=(), sources=(), **details):
        keys = list(dict.fromkeys(key for key in steps if key in self.steps))
        refs = set(sources)
        for key in keys:
            refs.update(self.steps[key]["sources"])
        if not refs or not refs <= self.sources.rows.keys():
            return
        proof = {field: values for field, values in (("steps", keys), ("paths", list(dict.fromkeys(paths)))) if values}
        self.rows[kind].setdefault(text, {"kind": kind, "text": text, "sources": sorted(refs),
                                          **({"proof": proof} if proof else {}), **details})

    def add_path(self, kind, text, path, events, steps=(), alternate=(), sources=(), **details):
        candidates = {event["call"]: self.name(event["call"]) for event in events
                      if event.get("composed") == "CONDITIONAL_SOURCE_CANDIDATE"}
        if scope := path.get("scope"):
            call = scope.get("call")
            text += f" This replay ends within the retained callable {code(method(scope['symbol']))}. The outer continuation remains open."
            if call:
                steps = [call, *steps]
                if scope["authority"] == "CONDITIONAL_SOURCE_CANDIDATE":
                    candidates[call] = self.name(call)
        names = list(dict.fromkeys(name for name in candidates.values() if name))
        if names:
            text += " This path assumes unresolved calls select the retained source for " + ", ".join(names) + "."
        if "ASSUMED_SDK_GLOBAL_IMPORTS" in path["open"]:
            text += " Framework bindings on this path assume SDK global imports."
        nominal = [self.name(event["step"]) for event in events if event["outcome"] == "INVOKED" and
                   self.steps[event["step"]].get("roleAuthority") == "CONDITIONAL_NOMINAL_FRAMEWORK_MEMBER"]
        if nominal:
            text += " This path uses nominal framework binding for " + ", ".join(dict.fromkeys(nominal)) + "."
        sourced = (*sources, *(source for key in candidates for source in self.steps[key]["boundSources"]))
        self.add(kind, text, [*candidates, *steps], [path["id"], *alternate], sourced, **details)

    def commands_for(self, path):
        return (self.local_commands if path.get("scope") else self.commands).get(path["id"], ())

    def trace(self, path):
        steps, event = [], path["lastEvent"]
        while event:
            row = self.events[event]
            steps.append(row)
            event = row.get("previous")
        return steps[::-1]

    def text(self, key):
        step = self.steps[key]
        return self.sources.text(step["site"], step["sources"])

    def switch_input(self, key, event=None):
        step = self.steps[key]
        operand = event.get("switchInputValue") if event else None
        return (value_text(operand) or self.sources.text(step["switchTest"]["input"], step["sources"]))

    def condition(self, key, truth, event=None):
        step = self.steps[key]
        if switch := step.get("switchTest"):
            subject = self.switch_input(key, event)
            pattern = self.sources.text(switch["pattern"], step["sources"])
            if subject and pattern:
                return f"{code(subject)} {'matches' if truth else 'differs from'} {code(pattern)}"
        if step.get("sourcePredicate") and (text := self.text(key)):
            if self.steps[key].get("selectionPredicate"):
                return (f"the remaining candidate matches {code(text)}" if truth else
                        f"no row in the tracked collection matches {code(text)}")
            return f"{code(text)} is {'true' if truth else 'false'}"
        return None

    def source_call(self, key):
        step = self.steps[key]
        literal = self.sources.literal(step["site"], step["sources"])
        if literal and len(literal) <= 160:
            return code(literal)
        label = self.name(key)
        return code(label) if label else None

    def guard_bypasses(self):
        for row in self.graph.get("structuralGuardReturns", ()):
            condition, returned = row["condition"], row["return"]
            test = self.condition(condition, row["when"])
            if not test or not row["bypasses"]:
                continue
            scoped = condition.split("/", 1)[0]
            owner = method(self.root.get("symbol", "")) if scoped in self.root_refs else self.helpers.get(scoped)
            named = []
            proof = [condition, returned]
            ordered = sorted(row["bypasses"], key=lambda key: self.steps[key]["site"]["startByte"])
            for key in ordered:
                if label := self.source_call(key):
                    proof.append(key)
                    if await_step := self.steps[key].get("guardAwaitStep"):
                        proof.append(await_step)
                        label = "the await of " + label
                    named.append(label)
            if not named:
                continue
            outcome = self.text(returned)
            terminal = f"the early return {code(outcome)}" if outcome else "the early return"
            scope = f" in {code(owner)}" if owner else " in this callable"
            coverage = row.get("bypassCoverage", "PARTIAL")
            if len(named) != len(ordered):
                coverage = "PARTIAL"
            additional = " Further bypassed sites remain open." if coverage == "PARTIAL" else ""
            self.add("guard-return-bypass", f"When {test}{scope}, {terminal} bypasses " + ", ".join(named) +
                     " on the opposite source continuation if the return completes." + additional,
                     proof, sources=row["sources"], scope="CALLABLE_SOURCE_CONTINUATION", coverage=coverage)

    def effect_inventory(self):
        row = self.graph.get("directEffectInventory")
        if row is None:
            return
        labels, indirect, unresolved, candidates = defaultdict(list), [], [], set()
        for item in row["sites"]:
            name = self.source_call(item["step"])
            if item["classification"] == "BOUND_API":
                labels[item["family"]].append(name or "a cited call site")
            elif item["classification"] == "SOURCE_CALL":
                indirect.append(name or "a cited source call")
            else:
                label = name or "a cited operation"
                if item.get("family"):
                    candidates.add(item["family"])
                    label += f" (a {item['family']} candidate)"
                unresolved.append(label)
        found = list(labels)
        details = [f"{family} via {', '.join(dict.fromkeys(labels[family]))}" for family in DIRECT_EFFECT_FAMILIES if family in labels]
        scope = "In the selected callable body's compiler-reachable explicit calls and constructions"
        text = (f"{scope}, the retained bindings identify " + ", ".join(details) + "." if details else
                "The inventory examines the selected callable body's compiler-reachable explicit calls and constructions.")
        absent = []
        if row["status"] == "CLOSED_DIRECT_SCOPE":
            absent = [family for family in DIRECT_EFFECT_FAMILIES if family not in found]
            if absent:
                text += " This scope has no direct call in the " + ", ".join(absent) + " families."
        else:
            if unresolved:
                if row["siteCoverage"] == "COMPLETE":
                    text += " All explicit sites were examined within this scope."
                text += " Classification remains open at " + ", ".join(dict.fromkeys(unresolved)) + "."
            if row["limited"]:
                text += " Additional reachable sites exceed the inventory limit."
        if indirect:
            text += " Calls through " + ", ".join(dict.fromkeys(indirect)) + " have their own effect scope."
        if row.get("sdkImports"):
            text += " Framework bindings use the projected SDK global imports."
        self.add("direct-effect-inventory", text, [item["step"] for item in row["sites"]], sources=row["sources"],
                 scope="DIRECT_CALLS_IN_SELECTED_CALLABLE", coverage=row["status"], siteCoverage=row["siteCoverage"],
                 presentFamilies=found, candidateFamilies=sorted(candidates), absentFamilies=absent)

    def composed_effect_inventory(self):
        row = self.graph.get("composedEffectInventory")
        if row is None:
            return
        scopes = row["scopes"]
        labels, candidates, opaque, omitted, response = defaultdict(list), defaultdict(list), [], [], []
        proof = [scope["entrySites"][-1] for scope in scopes if scope["entrySites"]]
        for scope in scopes:
            for item in scope["sites"]:
                kind = item["classification"]
                if kind == "BOUND_API":
                    labels[item["family"]].append(item)
                elif kind == "CANDIDATE_API":
                    candidates[item["family"]].append(item)
                elif kind == "RESPONSE_CONSTRUCTION":
                    response.append(item)
                elif kind == "UNSELECTED_PROJECT_SOURCE":
                    omitted.append(item)
                elif kind in {"OPAQUE", "OPEN_EVALUATION"} or item.get("dispatch") == "CONDITIONAL":
                    opaque.append(item)

        def examples(items, limit):
            named = {}
            for item in items:
                step = item["step"]
                label = self.source_call(step)
                if label:
                    named.setdefault(label, step)
                if len(named) == limit:
                    break
            proof.extend(named.values())
            return ", ".join(named) or "cited source sites"

        symbols = [scope["symbol"].split("(", 1)[0].removeprefix("M:").replace(".#ctor", " constructor") for scope in scopes]
        text = ("The reachable direct call sites were examined from " + code(symbols[0]) + " through " +
                (", ".join(map(code, symbols[1:])) if len(symbols) > 1 else "its retained callable body") + ".")
        found, possible = list(labels), list(candidates)
        for family in DIRECT_EFFECT_FAMILIES:
            if family in labels:
                text += f" The reachable source includes {family} API sites such as {examples(labels[family], 3)}."
            if family in candidates:
                text += f" {family.capitalize()} SDK or call-shape candidates include {examples(candidates[family], 3)}."
        if response:
            text += f" {examples(response, 2)} construct the returned HTTP response."

        absent = []
        if row["siteCoverage"] == "COMPLETE":
            absent = [family for family in DIRECT_EFFECT_FAMILIES if family not in labels and family not in candidates]
            if "broker" in absent:
                text += " These enumerated reachable project bodies contain no explicit call to a supported inline broker client API."
            others = [family for family in absent if family != "broker"]
            if others:
                text += " No supported explicit call was identified in these bodies for " + ", ".join(others) + "."
        else:
            text += " The reachable project-source inventory remains partial."
        if omitted:
            text += f" Reachable project bodies are unavailable for {examples(omitted, 2)}."
        if opaque or candidates or labels:
            boundary = f" for {examples(opaque, 3)}" if opaque else ""
            text += f" Runtime dispatch and effects beyond the cited API or project bodies remain open{boundary}."

        witness = next((item for items in (*labels.values(), *candidates.values()) for item in items if item.get("witness")), None)
        if witness:
            guards = []
            for guard in witness["witness"]:
                if label := self.condition(guard["condition"], guard["when"]):
                    guards.append(label)
                    proof.append(guard["condition"])
            if guards:
                text += f" One retained CFG route to {examples([witness], 1)} takes the branch where " + " and ".join(guards) + "."

        self.add("composed-effect-inventory", text, proof, sources=[scope["id"] for scope in scopes],
                 scope="REACHABLE_SELECTED_PROJECT_BODIES", siteCoverage=row["siteCoverage"],
                 presentFamilies=found, candidateFamilies=possible, absentExplicitFamilies=absent,
                 runtimeCoverage="OPEN" if opaque or candidates or omitted or labels else "SELECTED_SOURCE_ONLY")

    def name(self, key, event=None, events=()):
        step = self.steps[key]
        label = step.get("spelling") or method(step.get("symbol") or "")
        if not label:
            return None
        receiver = value_text(event.get("receiver")) if event else None
        if not receiver and event and event.get("receiver"):
            owner = key.split("/", 1)[0]
            names = set()
            for item in reversed(events):
                name = item.get("name")
                if item["outcome"] != "LOCAL_BINDING" or not name or name in names or item["step"].split("/", 1)[0] != owner:
                    continue
                names.add(name)
                if item.get("value") == event["receiver"]:
                    receiver = name
                    break
        return code(f"{receiver}.{label}" if receiver else label)

    def transaction(self, value, start=None):
        if direct := value_text(value):
            return code(direct) if not null_literal(value) else None
        if value and value[0] == "call-result" and (start == value[1] or
                self.steps[value[1]].get("role") == "TRANSACTION_BEGIN_REQUEST" or
                self.steps[value[1]].get("receiverFamily") == "connection" and
                self.steps[value[1]].get("spelling") in {"BeginTransaction", "BeginTransactionAsync"}):
            return "the transaction from " + self.name(value[1])
        return None

    def allocation(self, value, events):
        created = next((item for item in events if item.get("result") == value and item["outcome"] == "CONSTRUCTED"), None)
        if not created:
            return None, {}
        step = self.steps[created["step"]]
        record = {item["name"]: item["id"] for item in step.get("recordFields", ())}
        fields = {arg["name"]: (arg["value"], created["step"], record[arg["name"]]) for arg in created["arguments"]
                  if arg.get("name") in record}
        for event in events:
            if event["outcome"] == "INVOKED" and (event.get("receiver") == value or
                    any(arg["value"] == value for arg in event.get("arguments", ()))):
                call = self.steps[event["step"]]
                if not call.get("binding") and call.get("role") != "LIST_ADD":
                    fields.clear()
            resource = event.get("resource")
            if event["outcome"] not in {"STATE_WRITE_REACHED", "CANDIDATE_INITIALIZER_REACHED"} or not resource or resource[2] != value:
                continue
            if event["outcome"] == "CANDIDATE_INITIALIZER_REACHED":
                fields[resource[1]] = event.get("value"), event["step"], None
            elif field := self.steps[event["step"]].get("initializerMember"):
                fields[field] = event.get("sourceValue"), event["step"], resource[1]
            else:
                fields.pop(resource[1].rsplit(".", 1)[-1], None)
        label = step.get("objectType")
        return ("candidate " + label if label and step.get("unresolved") else label), fields

    @staticmethod
    def completed_invocation(value, events):
        for index, item in enumerate(events):
            if item.get("result") != value or item["outcome"] != "INVOKED":
                continue
            completion = next((event for event in events[index + 1:] if event.get("call") == item["step"]), None)
            if completion and completion["outcome"] in {"RETURNED", "RESUMED"}:
                return item, completion["step"]
            break
        return None, None

    def json_binding_intact(self, owner, events, member=None):
        """Discard a JSON origin after a visible write or opaque escape."""
        seen = False
        for event in events:
            if event.get("result") == owner and event["outcome"] == "INVOKED":
                seen = True
                continue
            if not seen:
                continue
            if event["outcome"] in {"COMPLETION_OPEN", "WRITE_TARGET_OPEN"}:
                return False
            resource = event.get("resource")
            if (event["outcome"] == "STATE_WRITE_REACHED" and resource and resource[0] == "member"
                    and resource[2] == owner and (not member or resource[1] == member)):
                return False
            if event["outcome"] == "INVOKED" and self.steps[event["step"]].get("valueFactory") not in {
                    "JSON_PROPERTY", "JSON_VALUE"} and (event.get("receiver") == owner or
                    any(relates(argument["value"], owner) for argument in event.get("arguments", ()))):
                return False
        return seen

    def json_value(self, value, events, commands, proof, depth):
        if value[0] in {"json-root", "json-key"}:
            if value[0] == "json-root":
                return self.lineage(value[1], events, commands, proof, depth + 1)
            owner, key = value[1:]
            if key[0] != "literal" or not isinstance(key[1], str):
                return None
            if owner[0] == "call-result" and not self.json_binding_intact(owner, events):
                return None
            if label := self.lineage(owner, events, commands, proof, depth + 1):
                return f"JSON property {json.dumps(key[1], ensure_ascii=False)} in {label}"
            return None

        step = self.steps.get(value[1], {})
        factory = step.get("valueFactory")
        if factory not in {"JSON_PARSE", "JSON_DESERIALIZE", "JSON_PROPERTY", "JSON_VALUE", "JSON_SERIALIZE"}:
            return None
        invocation, completed = self.completed_invocation(value, events)
        if not invocation:
            return None
        upto = events.index(invocation)
        arguments = invocation["arguments"]
        if factory in {"JSON_PARSE", "JSON_DESERIALIZE", "JSON_SERIALIZE"}:
            argument = next((item["value"] for item in arguments if item["position"] == 0), None)
            operand = argument
        else:
            operand = invocation.get("receiver")
        attribute = {
            "JSON_PARSE": "JSON parsed from ",
            "JSON_SERIALIZE": "JSON from JsonSerializer.Serialize over ",
            "JSON_DESERIALIZE": "the object deserialized from ",
            "JSON_VALUE": "the value read from ",
        }.get(factory)
        if factory == "JSON_PROPERTY":
            key = next((item["value"] for item in arguments if item["position"] == 0), None)
            if not key or key[0] != "literal" or not isinstance(key[1], str):
                return None
            attribute = f"JSON property {json.dumps(key[1], ensure_ascii=False)} in "
        nested = []
        source = self.lineage(operand, events[:upto], commands, nested, depth + 1)
        if not source:
            return None
        proof.extend((*nested, step["id"], completed))
        return attribute + source

    def lineage(self, value, events, commands, proof, depth=0):
        if not value or depth > 8:
            return None
        kind, *data = value
        if kind in {"json-root", "json-key"}:
            return self.json_value(value, events, commands, proof, depth)
        if kind == "access-result":
            access = next((item for item in events if item.get("result") == value and
                           item["outcome"] == "INDEX_VALUE_REACHED"), None)
            if access and access.get("value") and access["value"][0] in {"json-root", "json-key"}:
                if label := self.json_value(access["value"], events[:events.index(access)], commands, proof, depth):
                    proof.append(access["step"])
                    return label
        if kind == "call-result":
            step = self.steps.get(data[0], {})
            if step.get("valueFactory") == "GUID_NEW":
                proof.append(data[0])
                return "Guid.NewGuid()"
            if label := self.json_value(value, events, commands, proof, depth):
                return label
            if step.get("valueFactory") == "LINQ_FIRST_OR_DEFAULT":
                proof.append(data[0])
                if predicate := self.sources.text(step.get("predicateSite"), step["sources"]):
                    return f"FirstOrDefault({predicate})"
                return "FirstOrDefault result"
            for command in commands:
                if command.get("returnValue") == value:
                    columns = command["sourceSql"]["projections"]
                    column = columns[0].get("column") if columns and command.get("executionRole") == "ADO_EXECUTE_SCALAR" else None
                    table = command["sourceSql"]["table"]
                    proof.extend(command["evidenceSteps"])
                    return f"the result from {table}.{column}" if column and column != "*" else f"the result from {table}"
                for read in command.get("columnReads", ()):
                    if read["value"] == value:
                        proof.extend([*command["evidenceSteps"], read["call"], read["completion"], read["rowCheck"]])
                        cast = f" cast as {read['cast']}" if read.get("cast") else ""
                        return f"the row value from {command['sourceSql']['table']}.{read['column']}{cast}"
        if kind == "member" and data[1] and data[1][0] == "call-result" and (
                self.steps.get(data[1][1], {}).get("valueFactory") == "JSON_DESERIALIZE" and
                self.json_binding_intact(data[1], events, data[0])):
            if label := self.lineage(data[1], events, commands, proof, depth + 1):
                return f"member {data[0].rsplit('.', 1)[-1]} of {label}"
        if kind in {"member", "open-member"} and data[1] and data[1][0] == "allocation":
            _, fields = self.allocation(data[1], events)
            name = data[0].rsplit(".", 1)[-1]
            if name in fields and fields[name][2] == data[0]:
                item, step, _ = fields[name]
                proof.append(step)
                return self.lineage(item, events, commands, proof, depth + 1)
        if kind == "record" and 0 < len(data[0]) <= 8:
            return self.field_lineage(dict(data[0]), events, commands, proof, depth)
        if kind == "allocation":
            type_name, fields = self.allocation(value, events)
            if type_name and fields:
                proof.extend(step for _, step, _ in fields.values())
                labels = self.field_lineage({name: item for name, (item, _, _) in fields.items()}, events, commands, proof, depth)
                return f"{type_name} initialized with {labels}"
        return value_text(value)

    def field_lineage(self, fields, events, commands, proof, depth):
        parts = []
        for name, item in list(fields.items())[:8]:
            value = self.lineage(item, events, commands, proof, depth + 1)
            if value and item and item[0] == "record":
                value = "{" + value + "}"
            parts.append(f"{name} = {value}" if value else name)
        return ", ".join(parts)

    def http(self, value, events, commands, *, shallow=False):
        if not value:
            return None
        if value[0] == "call-result":
            key = value[1]
        elif value[0] == "allocation":
            key = f"{value[1]}/op{value[2]}"
        else:
            return None
        if not (response := self.steps.get(key, {}).get("httpResult")):
            return None
        factory = next((i for i in range(len(events) - 1, -1, -1) if events[i].get("result") == value), None)
        if factory is None:
            return None
        argument = next((arg for arg in events[factory].get("arguments", ()) if arg["position"] == response["payload"]), None)
        proof = [key]
        payload = None
        if argument and not null_literal(argument["value"]):
            payload = value_text(argument["value"]) if shallow else self.lineage(argument["value"], events[:factory], commands, proof)
        suffix = f" with {code(payload)}" if payload else ""
        if payload and argument["value"][0] == "record":
            suffix = " with response fields " + code(payload)
        return f"HTTP {response['status']} result{suffix}", proof

    def root_return(self, path, events, *, shallow=False):
        if path["outcome"] != "RETURNED":
            return None
        returned = next((e["step"] for e in reversed(events) if self.steps[e["step"]]["kind"] == "RETURN"
                         and e["step"].split("/", 1)[0] in self.root_refs), None)
        if returned is None:
            return None
        if response := self.http(path.get("returnValue"), events, self.commands_for(path), shallow=shallow):
            return response[0], [*response[1], returned]
        return "a return from the handler", [returned]

    def path_condition(self, events):
        guards = {}
        for event in events:
            key, outcome = event["step"], event["outcome"]
            if outcome not in {"TRUE", "FALSE"} or self.steps[key].get("loopTest"):
                continue
            if description := self.condition(key, outcome == "TRUE", event):
                subject = self.switch_input(key, event) if self.steps[key].get("switchTest") else None
                guards.setdefault((key, subject), set()).add(description)
        return [(next(iter(descriptions)), key) for (key, _), descriptions in guards.items() if len(descriptions) == 1]

    def first_empty_reads(self, path, events):
        owners = set()
        for event in events:
            if event.get("emptyListDefault"):
                source = next((arg["value"] for arg in event["arguments"] if arg["position"] == 0), None)
                if source and source[0] == "allocation":
                    owners.add(source[1])
        guards = []
        for command in self.commands_for(path):
            checks = command.get("rowChecks", ())
            if command["call"].split("/", 1)[0] not in owners or len(checks) != 1 or checks[0]["hasRow"]:
                continue
            key = checks[0]["test"]
            condition = next((item for item in path["conditions"] if item["step"] == key), None)
            if condition and (text := self.condition(key, condition["value"])):
                guards.append((text, key))
        return guards

    @staticmethod
    def when(guards):
        return "When " + " and ".join(text for text, _ in guards) + ", this source path " if guards else "This source path "

    def entry(self, process, context, is_http):
        refs = [ref for ref in context.get("entrySources", ()) if ref in self.sources.rows]
        if not (refs and is_http):
            return
        routes = []
        for match in process.get("routes", ()):
            if not isinstance(match, (list, tuple)) or len(match) != 2:
                continue
            verb, route = match
            if (isinstance(verb, str) and re.fullmatch(r"[A-Z]{3,10}", verb) and isinstance(route, str) and
                    route.startswith("/") and len(route) <= 160):
                routes.append(verb + " " + route)
        if routes:
            self.add("http-entry", "The retained registration declares " + ", ".join(map(code, routes[:6])) + ".", sources=refs)
        for item in self.root.get("inputs", ()):
            description = item.get("type")
            if not description or not self.sources.text(item.get("site"), self.root_refs):
                continue
            name, binding = item["name"], item.get("binding")
            if binding == "services":
                statement = f"The handler declares {code(name)} of type {code(description)} from services."
            elif binding == "context":
                statement = f"The handler declares framework parameter {code(name)} of type {code(description)}."
            elif binding == "parameter group":
                statement = f"The handler declares the parameter group {code(name)} of type {code(description)}."
            elif binding:
                external = f" as {code(item['bindingName'])}" if item.get("bindingName") and item["bindingName"] != name else ""
                statement = f"The handler declares {code(name)} of type {code(description)} from the request {binding}{external}."
            else:
                statement = f"The handler declares parameter {code(name)} of type {code(description)}."
            self.add("input", statement, sources=self.root_refs)

    def returns(self, path, events, is_http):
        interrupted = any(event["outcome"] in {*INTERRUPTED, "THROWN", "CONSTRUCTOR_EXCEPTION"} for event in events)
        ambiguous = LIST_SELECTION_OPEN in path["open"]
        if (is_http and not interrupted and (returned := self.root_return(path, events, shallow=ambiguous)) and
                returned[0].startswith("HTTP ")):
            guards = [*self.first_empty_reads(path, events), *self.path_condition(events)]
            if ambiguous:
                guards = [(label, key) for label, key in guards if key.split("/", 1)[0] in self.root_refs]
                start = "When " + " and ".join(label for label, _ in guards) + ", the handler " if guards else "The handler "
                steps = [key for key in returned[1] if key.split("/", 1)[0] in self.root_refs]
                if len(steps) == len(returned[1]):
                    self.add("http-outcome", start + f"returns an {returned[0]}. The storage-to-selector lineage remains open.",
                             [*(key for _, key in guards), *steps])
            else:
                self.add_path("http-outcome", self.when(guards) + f"returns an {returned[0]}.", path, events,
                              [*(key for _, key in guards), *returned[1]])
        for index, event in enumerate(events):
            if event["outcome"] not in COMPLETED or "returnedValue" not in event:
                continue
            call = event["call"]
            symbol = self.steps[call]
            if not symbol.get("boundSources") or not (name := self.name(call)):
                continue
            value = event["returnedValue"]
            commands = self.commands_for(path)
            response = self.http(value, events[:index], commands)
            value_proof = []
            shown = response[0] if response else code(text) if (text := self.lineage(value, events[:index], commands, value_proof)) else None
            if shown is None:
                continue
            invoked = next((i for i in range(index - 1, -1, -1) if events[i]["step"] == call and events[i]["outcome"] == "INVOKED"), None)
            if invoked is None:
                continue
            inside = events[invoked + 1:index]
            guards = self.path_condition(inside)
            returning = [e["step"] for e in inside if self.steps[e["step"]]["kind"] == "RETURN"]
            description = "When " + " and ".join(text for text, _ in guards) + ", " if guards else ""
            qualifier = "the retained candidate " if symbol["binding"] == "CONDITIONAL_SOURCE_CANDIDATE" else ""
            if qualifier and not description:
                qualifier = qualifier.capitalize()
            self.add("helper-return", description + f"{qualifier}{name} returns {shown} to its caller.",
                     [call, event["step"], *(key for _, key in guards), *returning, *value_proof,
                      *(response[1] if response else ())], [path["id"]], symbol["boundSources"])

    @staticmethod
    def sql_label(sql):
        def clause(column, value):
            if value.startswith("not-"):
                return f"{column} IS NOT {value[4:]}"
            return f"{column} IS NULL" if value == "null" else f"{column} = {value}"

        verb, table = sql["verb"], sql["table"]
        article = "an" if verb in {"INSERT", "UPDATE"} else "a"
        parts = [f"{article} {code(verb)} on {code(table)}"]
        projected = []
        for field in sql["projections"]:
            column, name = field["column"], field["name"]
            if column and name and column != name:
                projected.append(f"{code(column)} as {code(name)}")
            elif column or name:
                projected.append(code(column or name))
        if projected:
            label = "RETURNING fields" if sql.get("returning") else "projected columns or aliases"
            parts.append(f"recognized {label} " + ", ".join(projected))
        if casts := [f"{code(field['sourceColumn'])} as {code(field['cast'])}" for field in sql["projections"] if field.get("cast")]:
            parts.append("source casts " + ", ".join(casts))
        if sql["assignment"]:
            parts.append("assignments " + ", ".join(code(f"{key} = {value}") for key, value in sql["assignment"].items()))
        elif sql.get("set"):
            parts.append("SET clause " + code(sql["set"]))
        if sql["where"]:
            parts.append("filters " + " and ".join(code(clause(key, value)) for key, value in sql["where"].items()))
        elif sql.get("filter"):
            parts.append("WHERE clause " + code(sql["filter"]))
        if sql.get("tail"):
            parts.append("selection clauses " + code(sql["tail"]))
        if conflict := sql.get("conflict"):
            keys = " on " + ", ".join(map(code, conflict["keys"])) if conflict["keys"] else ""
            if conflict["constraint"]:
                keys = " on constraint " + code(conflict["constraint"])
            action = " uses " + code("DO " + conflict["action"]) if conflict["action"] else ""
            parts.append(f"conflict handling{keys}{action}")
        return ", ".join(parts)

    def sql(self, path, events, focus=None):
        commands = {row.get("eventIndex"): row for row in self.commands_for(path)}
        prefix = path.get("scope", {}).get("prefixEvents", 0)
        invoked = {}
        for index, event in enumerate(events):
            key, outcome = event["step"], event["outcome"]
            if outcome == "INVOKED":
                invoked[key] = event
            else:
                key = event.get("call")
            if key is None or index < prefix or focus is not None and key != focus:
                continue
            command, step = commands.get(index), self.steps[key]
            sql = command.get("sourceSql") if command else step.get("sourceSql")
            if not sql or not (name := self.name(key, invoked.get(key), events[:index])):
                continue
            kind = "sql-read" if sql["verb"] == "SELECT" else "sql-write"
            statement, proof, refs = self.sql_label(sql), [key], ()
            if command:
                phase = ("resumes after" if outcome == "RESUMED" else "returns from" if outcome == "RETURNED" else
                         "starts" if outcome == "AWAITABLE_RETURNED" else "is interrupted during")
                if command["authority"] == "CONDITIONAL_COMMAND_API":
                    phase += " a candidate API call to"
                guards = self.path_condition(events[:index]) if focus is not None else ()
                text = self.when(guards) + f"{phase} {name} configured with {statement}"
                binding_proof = []
                bindings = [f"{code(item['name'])} from {code(value)}" for item in command["bindings"]
                            if (value := self.lineage(item["value"], events[:index], tuple(commands.values()), binding_proof)) is not None]
                if bindings:
                    text += ", binding " + ", ".join(bindings)
                if transaction := self.transaction(command["transaction"], command.get("transactionStart")):
                    text += " with " + transaction
                proof, refs = [*(key for _, key in guards), *command["evidenceSteps"], *binding_proof], command["sources"]
            elif outcome == "INVOKED":
                text = f"The source passes {statement} to {name}"
                args = {arg.get("name"): arg["value"] for arg in event.get("arguments", ()) if arg.get("name")}
                if transaction := self.transaction(args.get("transaction")):
                    text += " with " + transaction + " as its transaction argument"
            else:
                continue
            anchor = step["site"]
            write_range = {"host": step["sources"][0], "start": anchor["startByte"], "end": anchor["endByte"],
                           "units": "ABSOLUTE_BYTES", "textDigest": anchor["textDigest"]}
            self.add_path(kind, text + ".", path, events, proof, sources=refs,
                          _edge={"kind": sql["verb"], "table": sql["table"], "site": step["sources"][0],
                                 "occurrence": key},
                          **({"_writeRange": write_range} if kind == "sql-write" and focus is None else {}))
            if command:
                for check in command.get("rowChecks", ()):
                    if reader := self.name(check["readerCall"]):
                        result = "advances to a result row" if check["hasRow"] else "reports no further row"
                        self.add_path("sql-row", f"For the lookup on {code(sql['table'])}, {reader} {result} on this source path.",
                                      path, events, [*proof, check["readerCall"], check["completion"], check["test"]], sources=refs)
                if command.get("executionRole") == "ADO_EXECUTE_SCALAR" and (result := command.get("returnValue")):
                    reached = {event["step"] for event in events[index + 1:]}
                    for check in path["conditions"]:
                        operand = check.get("operand")
                        if (check["step"] in reached and relates(operand, result) and
                                (condition := self.condition(check["step"], check["value"]))):
                            self.add_path("sql-row", f"After {name} runs with the command for {code(sql['table'])}, "
                                          f"the source takes the branch where {condition} for the scalar result.",
                                          path, events, [*proof, check["step"]], sources=refs)

    def list_lookup(self, path, events):
        for event in events[path.get("scope", {}).get("prefixEvents", 0):]:
            if event["outcome"] != "RETURNED" or (call := event.get("call")) is None:
                continue
            step = self.steps[call]
            if step.get("spelling") != "FirstOrDefault":
                continue
            proof = [call, event["step"]]
            label = "The source calls `FirstOrDefault`" if step.get("valueFactory") == "LINQ_FIRST_OR_DEFAULT" else (
                "The retained call site names `FirstOrDefault`. Selector binding remains open")
            if predicate := self.sources.text(step.get("predicateSite"), step["sources"]):
                label += " with predicate " + code(predicate) if step.get("valueFactory") == "LINQ_FIRST_OR_DEFAULT" else (
                    ". The argument lambda is " + code(predicate))
            if event.get("emptyListDefault"):
                arg = next((arg["value"] for arg in event["arguments"] if arg["position"] == 0), None)
                if arg and arg[0] == "allocation":
                    proof.append(f"{arg[1]}/op{arg[2]}")
                label += ". The tracked list is empty at this call, so the result is `null` on this path"
                proof.extend(key for _, key in self.first_empty_reads(path, events))
            elif "selectedValue" in event and predicate:
                if null_literal(event["selectedValue"]):
                    label += ". No row in the tracked collection satisfies that predicate on this path"
                else:
                    label += ". This path selects the first tracked row that satisfies the predicate"
            self.add_path("lookup", label + ".", path, events, proof)

    def object_arguments(self, path, events):
        commands = self.commands_for(path)
        from_sql = {key for command in commands for key in command["evidenceSteps"]}
        if not from_sql:
            return
        prefix = path.get("scope", {}).get("prefixEvents", 0)
        for index, event in enumerate(events):
            if index < prefix or event["outcome"] != "INVOKED":
                continue
            for argument in event["arguments"]:
                value = argument["value"]
                if not value or value[0] not in {"allocation", "record", "call-result"}:
                    continue
                proof = []
                projected = self.lineage(value, events[:index], commands, proof)
                structured = value[0] in {"allocation", "record"} or self.steps.get(value[1], {}).get("valueFactory") in {
                    "JSON_SERIALIZE", "JSON_DESERIALIZE"}
                if structured and projected and from_sql.intersection(proof) and (
                        call := self.name(event["step"], event, events[:index])):
                    json_lineage = any((self.steps[key].get("valueFactory") or "").startswith("JSON_") for key in proof)
                    guards = self.path_condition(events[:index]) if json_lineage else ()
                    action = self.when(guards) if json_lineage else "The source "
                    self.add_path("argument-projection", action + f"passes {code(projected)} to {call}.",
                                  path, events, [*(key for _, key in guards), *proof, event["step"]])

    def actions(self, path, events):
        commands = {row.get("eventIndex"): row for row in self.commands_for(path)}
        invoked, actions = {}, []
        for i, event in enumerate(events):
            key, outcome = event["step"], event["outcome"]
            if outcome == "INVOKED":
                invoked[key] = event
            if outcome == "STATE_WRITE_REACHED" and (field := value_text(event["resource"])):
                value = value_text(event["value"])
                label = f"reaches the write of {code(value)} to {code(field)}" if value is not None else f"reaches a write to {code(field)}"
                actions.append((i, label, [key]))
            call = event.get("call")
            if call is None or outcome not in COMPLETED or not (step := self.steps[call]):
                continue
            command = commands.get(i)
            start = invoked.get(call, {})
            matching = next((cmd for index, cmd in commands.items() if index is not None and index < i and
                             cmd.get("transaction") is not None and cmd["transaction"] == start.get("receiver")), None)
            unresolved = not step.get("symbol") and step.get("dispatch") == "OPEN"
            candidate_tx = (unresolved and (matching or step.get("receiverFamily") == "transaction") and
                            step.get("spelling") in {"Commit", "CommitAsync", "Rollback", "RollbackAsync"})
            candidate_begin = (unresolved and step.get("receiverFamily") == "connection" and
                               step.get("spelling") in {"BeginTransaction", "BeginTransactionAsync"})
            if step.get("role") not in EFFECT_ROLES and not step.get("sourceSql") and not command and not candidate_tx:
                if not candidate_begin:
                    continue
            if not (name := self.name(call, start, events[:i])):
                continue
            sql = command.get("sourceSql") if command else step.get("sourceSql")
            resource = (f" for topic {code(step['topic'])}" if step.get("topic") else
                        f" for {code(sql['table'])}" if sql else "")
            if candidate_tx or step.get("role") in {"TRANSACTION_COMMIT_REQUEST", "TRANSACTION_ROLLBACK_REQUEST"}:
                if transaction := self.transaction(start.get("receiver"), matching.get("transactionStart") if matching else None):
                    resource = " on " + transaction
            elif step.get("role") == "EXTERNAL_REQUEST":
                uri = next((arg["value"] for arg in start.get("arguments", ()) if arg.get("name") == "requestUri"), None)
                if not null_literal(uri) and (address := value_text(uri)):
                    resource = f" targeting {code(address)}"
            elif step.get("role") == "PERSISTENCE_REQUEST":
                if destination := value_text(start.get("resource")):
                    resource = f" for {code(destination)}"
            completion = "resumes after awaiting" if outcome == "RESUMED" else "returns from"
            candidate = (candidate_begin or candidate_tx or step.get("roleAuthority") == "CONDITIONAL_NOMINAL_BROKER_API" or
                         command and command.get("authority") == "CONDITIONAL_COMMAND_API")
            witnesses = matching["evidenceSteps"] if candidate_tx and matching else command["evidenceSteps"] if command else ()
            prefix = "the candidate API call " if candidate else ""
            actions.append((i, f"{completion} {prefix}{name}{resource}", [*witnesses, call, key]))
        return actions

    def commit_before_response(self, path, events, returned, focus=None):
        for match in self.transactions[path["id"]]:
            http = "status" in match
            if http and (not returned or not returned[0].startswith("HTTP ")):
                continue
            if not http and focus is None:
                continue
            writes = match["writes"]
            if focus is not None and not any(write["call"] == focus for write in writes):
                continue
            pending = next((event for event in reversed(events[:match["commitIndex"]])
                            if event["step"] == match["commit"] and event["outcome"] == "INVOKED"), None)
            label = self.name(match["commit"], pending, events[:match["commitIndex"]])
            if not label:
                continue
            guards = self.path_condition(events)
            phase = "resumes after awaiting" if self.steps[match["completion"]]["kind"] == "AWAIT" else "returns from"
            summary = f"{phase} {label}"
            if http:
                summary += f" before returning an {returned[0]}"
            summary += "."
            if writes:
                statements = [write["sourceSql"] for write in writes]
                descriptions = list(dict.fromkeys(self.sql_label(sql) for sql in statements))
                summary = ("resumes after command execution for " + " and ".join(descriptions)
                           + " on the same transaction, then " + summary)
                if any((sql.get("conflict", {}).get("action") or "").upper() == "NOTHING" for sql in statements):
                    summary += " With `DO NOTHING`, a handled conflict can leave the existing row unchanged."
                summary += " Affected rows and durability remain open."
            if match["authority"] == "PROVIDER_COMMAND_AND_TRANSACTION_BINDING":
                summary += " This ordering assumes the provider binds the command and commit APIs to the tracked transaction."
            self.add_path("http-commit-before-response" if http else "transactional-write-completion",
                          self.when(guards) + summary, path, events,
                          [*(key for _, key in guards), *match["evidenceSteps"], *(returned[1] if http else ())],
                          authority=match["authority"],
                          **({"condition": " and ".join(text for text, _ in guards)} if guards else {}))

    def dispatches(self, path, events):
        scope = path.get("scope")
        if scope and not scope.get("call"):
            return
        prefix = scope["prefixEvents"] if scope else 0
        for index, event in enumerate(events):
            if index < prefix or event["outcome"] != "INVOKED":
                continue
            key = event["step"]
            step = self.steps[key]
            site = step["sources"][0]
            symbol = step.get("symbol") or ""
            if not (name := self.name(key)):
                continue
            uncertainty = None
            if step.get("directEffectFamily") == "HTTP" and symbol.startswith("M:System.Net.Http.HttpClient."):
                value = next((arg["value"] for arg in event.get("arguments", ()) if arg.get("name") == "requestUri"), None)
                verb = HTTP_CLIENT_METHODS.get(method(symbol))
                uri = value[1] if value and value[0] == "literal" and isinstance(value[1], str) else None
                edge = {"kind": "HTTP", "method": verb, "uri": uri, "site": site, "occurrence": key}
                label = "The source calls the framework HTTP client " + name + "."
            elif step.get("role") == "PUBLICATION_CANDIDATE":
                position = step.get("kafkaTopicPosition")
                topic = step.get("topic")
                if position is not None:
                    values = [argument["value"] for argument in event.get("arguments", ())
                              if argument["position"] == position and not argument.get("byRef")]
                    value = values[0] if len(values) == 1 else None
                    topic = value[1] if value and value[0] == "literal" and isinstance(value[1], str) else None
                if not isinstance(topic, str) or not re.fullmatch(r"[A-Za-z0-9._-]{1,249}", topic) or topic in {".", ".."}:
                    continue
                edge = {"kind": "PUBLISH", "topic": topic, "site": site, "occurrence": key}
                label = f"The source reaches {name} with Kafka topic {code(topic)}."
                if step.get("roleAuthority") == "CONDITIONAL_NOMINAL_BROKER_API":
                    uncertainty = "This assumes the retained declared receiver binds to the Confluent Kafka producer API."
            else:
                continue
            guards = self.path_condition(events[:index])
            self.add_path("protocol-request", self.when(guards) + label, path, events,
                          [*(key for _, key in guards), key], _edge=edge,
                          **({"condition": " and ".join(text for text, _ in guards)} if guards else {}),
                          **({"uncertainty": uncertainty} if uncertainty else {}))

    def effects(self, path, events, is_http):
        actions = self.actions(path, events)
        returned = self.root_return(path, events) if is_http else None
        self.commit_before_response(path, events, returned)
        if returned and not returned[0].startswith("HTTP "):
            returned = None
        interrupted = []
        command_interruptions = {row.get("eventIndex"): row for row in self.commands_for(path)}
        caught_http = returned and any(self.steps[e["step"]]["kind"] == "CATCH" for e in events)
        for index, event in enumerate(events):
            key, call, outcome = event["step"], event.get("call"), event["outcome"]
            if outcome == "THROWN":
                failure = "execution reaches an explicit throw" if self.steps[key].get("explicit") else "a modeled exception occurs"
                interrupted.append((index, failure, [key]))
            elif outcome in INTERRUPTED and call and (caught_http or self.steps[call].get("role") in EFFECT_ROLES or
                                                       self.steps[call].get("directEffectFamily") == "HTTP" or
                                                       self.steps[call].get("sourceSql") or index in command_interruptions
                                                       ) and (label := self.name(call)):
                command = command_interruptions.get(index, {})
                interrupted.append((index, f"{label} {INTERRUPTED[outcome]}", [call, key, *command.get("evidenceSteps", ())]))
        if (not interrupted and LIST_SELECTION_OPEN not in path["open"] and path["outcome"] in {"RETURNED", "NORMAL_EXIT"} and actions and
                (len(actions) >= 2 or returned)):
            trace = actions if len(actions) <= 6 else [*actions[:3], *actions[-3:]]
            labels = [label for _, label, _ in trace]
            if returned:
                labels.append("returns an " + returned[0])
            guards = [*self.first_empty_reads(path, events), *self.path_condition(events)]
            self.add_path("effect-order", self.when(guards) + ", then ".join(labels) + ".", path, events,
                          [*(key for _, key in guards), *(step for _, _, keys in trace for step in keys),
                           *(returned[1] if returned else ())])
        if interrupted:
            cut, failure, proof = interrupted[0]
            failed_call = events[cut].get("call")
            earlier = [row for row in actions if row[0] < cut]
            earlier = earlier if len(earlier) <= 2 else [earlier[0], earlier[-1]]
            prior = ", then ".join(row[1] for row in earlier)
            start = (f"After this source path {prior}, {failure}. The modeled path " if earlier
                     else f"If {failure}, the modeled path ")
            continuation = []
            for event in events[cut + 1:]:
                if self.steps[event["step"]]["kind"] in {"CATCH", "FINALLY"}:
                    continuation.append("enters " + self.steps[event["step"]]["kind"].lower())
                    proof.append(event["step"])
            if returned:
                continuation.append("returns an " + returned[0])
                proof.extend(returned[1])
            elif path["outcome"] == "THREW":
                continuation.append("exits by exception")
            if continuation:
                proof.extend(step for _, _, keys in earlier for step in keys)
                self.add_path("error-path", start + ", then ".join(continuation) + ".", path, events, proof,
                              **({"_edgeCause": failed_call} if failed_call else {}))

    def callable_order(self, path, events):
        prefix = path["scope"]["prefixEvents"]
        actions = [(label, keys) for index, label, keys in self.actions(path, events) if index >= prefix]
        if not actions:
            return
        sample = actions if len(actions) <= 6 else [*actions[:3], *actions[-3:]]
        guards = self.path_condition(events[prefix:])
        self.add_path("effect-order", self.when(guards) + ", then ".join(label for label, _ in sample) + ".",
                     path, events, [*(key for _, key in guards), *(step for _, keys in sample for step in keys)])

    def callable_scope(self, path):
        scope = path["scope"]
        if "openSites" not in scope:
            return
        sites = scope["openSites"]
        labels = list(dict.fromkeys(name for key in sites if (name := self.name(key))))
        statement = f"The retained callable {code(method(scope['symbol']))}"
        statement += " contains call sites named " + ", ".join(labels) + ". Their resource effects remain open." if labels else (
            " has a retained source body. Its additional resource effects remain open.")
        self.add("callable-scope", statement, sites, sources=scope["sources"])

    def skipped_writes(self, path, events, focus=None):
        if path["outcome"] not in TERMINAL:
            return
        guards = dict((key, label) for label, key in self.path_condition(events))
        omissions = list(path["skippedOnThisPath"])
        reached = {event["step"] for event in events}
        # Local bindings and parameter configuration can exhaust the generic
        # skipped-site budget before a candidate command's execution is seen.
        conditional = {"CONDITIONAL_SOURCE_CANDIDATE", "ASSUMED_SDK_GLOBAL_IMPORTS", "CONDITIONAL_NOMINAL_FRAMEWORK_MEMBER"}
        if set(path["open"]) <= conditional:
            for alternative, commands in self.commands.items():
                if alternative == path["id"] or not set(self.paths[alternative]["open"]) <= conditional:
                    continue
                candidates = [command["call"] for command in commands if command["kind"] == "WRITE" and
                              command["call"] not in reached]
                if not candidates:
                    continue
                other = self.trace(self.paths[alternative])
                divergence = next(((left, right) for left, right in zip(events, other)
                                   if (left["step"], left["outcome"]) != (right["step"], right["outcome"])), None)
                if divergence and divergence[0]["step"] == divergence[1]["step"]:
                    pivot = divergence[0]["step"]
                    omissions.extend({"step": step, "after": pivot, "alternative": alternative} for step in candidates)
        for omitted in omissions:
            key, pivot, alternative = omitted["step"], omitted["after"], omitted["alternative"]
            if focus is not None and key != focus:
                continue
            step = self.steps[key]
            command = next((row for row in self.commands.get(alternative, ()) if row["call"] == key), {})
            sql = command.get("sourceSql") or step.get("sourceSql")
            if not sql or sql["verb"] == "SELECT":
                continue
            if pivot in guards:
                prefix = "When " + guards[pivot] + ", this modeled path "
            else:
                cut = next((event for event in events if event["step"] == pivot and event["outcome"] in INTERRUPTED), None)
                if not cut or not (name := self.name(cut.get("call", pivot))):
                    continue
                prefix = f"If {name} {INTERRUPTED[cut['outcome']]}, this modeled path "
            self.add_path("conditional-no-write", prefix + f"bypasses the call configured for {self.sql_label(sql)}.",
                          path, events, [pivot, key, *command.get("evidenceSteps", ())], [alternative])

    def worker(self, path, events):
        loops, writes, calls, delays = {}, {}, {}, {}
        conditions = iter(path["conditions"])
        for event in events:
            key, outcome = event["step"], event["outcome"]
            step = self.steps[key]
            owner = key.split("/", 1)[0]
            if outcome == "INVOKED" and step.get("role") == "POLL_DELAY":
                calls[key] = event
            if outcome == "RESUMED" and (call := calls.get(event.get("call"))):
                milliseconds = next((arg["value"] for arg in call["arguments"] if arg.get("name") == "millisecondsDelay"), None)
                duration = f" of {milliseconds[1]} ms" if milliseconds and milliseconds[0] == "literal" and (
                    type(milliseconds[1]) is int and milliseconds[1] >= 0) else ""
                delays[owner] = [call["step"], key], duration
            if outcome == "STATE_WRITE_REACHED":
                writes[json.dumps(event["resource"])] = event, dict(loops)
            if outcome not in {"TRUE", "FALSE"}:
                continue
            observed = next(conditions)
            condition = self.condition(key, observed["value"])
            if not condition:
                continue
            if step.get("loopTest"):
                previous = loops.get(owner)
                loops[owner] = key, previous[1] + 1 if previous and previous[0] == key else 1
                if previous and previous[0] == key:
                    proof, duration = delays.pop(owner, ([], ""))
                    before = f"After an awaited `Task.Delay`{duration} resumes, the modeled worker path" if proof else "The modeled worker path"
                    self.add("worker-poll", f"{before} reaches the {step['loopTest']} test again. "
                             f"On that check, {condition}.", [*proof, key], [path["id"]])
                continue
            if owner not in loops or "modeledOperandValue" not in event:
                continue
            operand = observed.get("operand")
            while operand and operand[0] == "not":
                operand = operand[1]
            if (write := writes.get(json.dumps(operand))) and (prior := write[1].get(owner)):
                written = write[0]
                loop, iteration = loops[owner]
                if (prior[0] == loop and prior[1] < iteration and (field := value_text(operand)) and
                        (value := value_text(written["value"])) is not None):
                    self.add("worker-state", f"After a write of {code(value)} to {code(field)}, "
                             f"the modeled worker test on a later iteration observes that {condition}.",
                             [loop, written["step"], key], [path["id"]])

    def structural_outcomes(self):
        for item in self.graph.get("awaitedGuardWrites", ()):
            sites = item["sites"]
            test, awaited, write = (self.text(sites[name]) for name in ("condition", "awaitSite", "write"))
            if not all((test, awaited, write)):
                continue
            truth = "true" if item["when"] else "false"
            host = code(method(item["host"]))
            condition = f"{code(test)} evaluates to {truth}"
            self.add("guarded_state_write", f"In {host}, after {code(awaited)} resumes and {condition}, "
                     f"the selected arm contains the state write {code(write)}.", sites.values(), sources=item["sources"],
                     condition=condition, authority="ROSLYN_CONTROL_FLOW")
            self.add("await_bypass", f"If {code(awaited)} faults or is cancelled before the guard completes, "
                     f"this evaluation of the condition in {host} bypasses {code(write)}.", sites.values(),
                     sources=item["sources"], authority="ROSLYN_CONTROL_FLOW")
        for item in self.graph.get("catchReturns", ()):
            sites = item["sites"]
            caught = self.text(sites["exception"]) if "exception" in sites else None
            filter_ = self.text(sites["filter"]) if "filter" in sites else None
            subject = f"the catch for {code(caught)}" if caught else "the catch clause"
            if filter_:
                subject += " filtered by " + code(filter_)
            host = code(method(item["host"]))
            if returned := self.text(sites["returned"]):
                self.add("early_exit", f"If control enters {subject} in {host} and reaches {code(returned)}, "
                         "it takes the return path out of the callable through any enclosing cleanup.", sites.values(),
                         sources=item["sources"], authority="ROSLYN_CONTROL_FLOW")

    def catch_continuations(self):
        for item in self.graph.get("catchContinuations", ()):
            caught = item["catch"]
            type_ = self.text(caught["clause"]) if "clause" in caught else None
            filter_ = self.text(caught["filter"]) if "filter" in caught else None
            subject = "the catch for " + code(type_) if type_ else "the catch clause"
            if filter_:
                subject += " filtered by " + code(filter_)
            actions = [self.text(key) for key in item["actions"]]
            awaits = [self.text(key) for key in item["awaits"]]
            guards = [(self.text(check["step"]), check["when"]) for check in item["guards"]]
            if not all(awaits) or not all(text for text, _ in guards):
                continue
            conditions = [f"{subject} completes normally"]
            conditions.extend(f"{code(test)} is {'true' if truth else 'false'}" for test, truth in guards)
            if "loopEntryTest" in caught and (test := self.text(caught["loopEntryTest"])):
                conditions.append(f"the iteration entered on a true {code(test)} test")
            reached = f"the {item['loop']} loop reaches" if item.get("loop") else "the source reaches"
            sequence = " and, if that await resumes, reaches ".join(code(phrase) for phrase in awaits)
            text = f"In {code(method(item['host']))}, when {', and '.join(conditions)}, {reached} {sequence}."
            if actions and all(actions):
                text += " The catch starts by calling " + ", then ".join(code(action) for action in actions) + "."
            self.add("handled-exception-continuation", text, item["evidenceSteps"], sources=item["sources"],
                     condition=" and ".join(conditions), authority=item["authority"])

    def worker_controls(self):
        for item in self.graph.get("hostedLoopStructure", ()):
            sites, loop, host = item["sites"], item["loop"], code(method(item["host"]))
            test = self.text(sites["test"])
            if item["kind"] == "delay":
                token = self.text(sites["token"]) if "token" in sites else None
                label = " with second argument " if item.get("conditional") else " with cancellation token "
                cancellation = label + code(token) if token else ""
                duration = f" for {item['milliseconds']} ms" if "milliseconds" in item else ""
                checked = f", {code(test)}" if test else ""
                text = f"In {host}, the source awaits `Task.Delay`{duration}{cancellation} on a path back to the {loop} test{checked}."
                if item.get("conditional"):
                    text += " The delay interpretation depends on resolving the framework overload."
            else:
                caught = self.text(sites["exception"]) if "exception" in sites else None
                condition = self.text(sites["filter"]) if "filter" in sites else None
                subject = f"a catch for {code(caught)}" if caught else "a catch clause"
                if condition:
                    subject += " filtered by " + code(condition)
                outcome = {"catch-break": f"can reach `break` and exit its {loop} loop",
                           "catch-continue": f"can reach `continue` and proceed toward the next {loop} check",
                           "catch-fallthrough": "can finish normally and fall through"}[item["kind"]]
                actions = [code(action) for name, key in sites.items() if name.startswith("action") and (action := self.text(key))]
                if actions:
                    outcome = f"can reach {', then '.join(actions)} and finish normally"
                text = f"In {host}, {subject} {outcome}."
            self.add("worker-control" if item["kind"] != "delay" else "worker-poll", text, sites.values(), sources=item["sources"])
        for item in self.graph.get("structuralGuardControls", ()):
            key, action = item["condition"], item["control"]
            if not (condition := self.condition(key, item["when"])):
                continue
            target = item["target"]
            outcome = "continues with the next loop test or iteration" if item["kind"] == "Continue" else f"exits the {target}"
            self.add("worker-control", f"When {condition}, reaching the arm's {code(item['kind'].lower())} {outcome}.",
                     [key, action], sources=item["sources"])
        for item in self.graph.get("structuralGuardReturns", ()):
            key, returned = item["condition"], item["return"]
            if condition := self.condition(key, item["when"]):
                name = self.helpers.get(key.split("/", 1)[0]) or method(self.root.get("symbol", ""))
                self.add("worker-control", f"When {condition}, this arm returns from {code(name)}.",
                         [key, returned], sources=item["sources"])
        for item in self.graph.get("possibleRepeatedEffects", ()):
            if not (call := self.name(item["externalCall"])):
                continue
            if item["kind"] == "POSSIBLE_REPEATED_REQUEST":
                missed = item["guardWriteNotReached"]
                if write := self.text(missed):
                    self.add("worker-reselection", f"The interruption path does not reach {code(write)}. "
                             f"{call} can run again if a later poll reaches the same guard and the resource still admits it.",
                             [item[key] for key in ("guard", "externalCall", "completedAt", "interruption", "guardWriteNotReached")],
                             [item["failurePath"], item["comparisonPath"]], item["sources"])
                continue
            if item["kind"] != "POSSIBLE_DURABLE_RESELECTION":
                continue
            self.worker_delivery(item, call)

    def worker_delivery(self, item, call):
        table, column = code(item["selectTable"]), code(item["keyColumn"])
        topic = self.steps[item["externalCall"]].get("topic")
        action = f"{call} for topic {code(topic)}" if topic else call
        proof = item.get("evidenceSteps", [item[key] for key in ("select", "externalCall", "completedAt", "interruption",
                                                               "matchingWrite", "matchingCommit")])
        prefix, scope_proof = self.worker_delivery_scope(item["comparisonPath"])
        reached = self.source_call(item["externalCall"]) or action
        write = item.get("matchingWriteSql") or self.steps[item["matchingWrite"]].get("sourceSql")
        mark = self.sql_label(write) if write else f"the update by {column}"
        commit = self.name(item["matchingCommit"])
        guard = item.get("rowGuard")
        condition = self.condition(guard["step"], guard["when"]) if guard else None
        if item.get("selectedRowSuppliesArguments") and (item.get("publicationAwaitFailed") or item.get("matchingCommitFailed")):
            selection = f"When {condition}, the comparison path" if condition else "The comparison path"
            connection = next((name for key in item.get("connectionEvidence", ()) if self.steps[key].get("boundSources")
                               and (name := self.source_call(key))), None)
            handle = f" It uses the connection returned by {connection}." if connection else ""
            fields, lineage_proof = self.worker_publication_fields(item)
            data = " The send binds " + ", ".join(fields) + "." if fields else ""
            self.add("worker-row-delivery", prefix + f"{selection} supplies data from the selected {table} row to {reached}. "
                     f"After the send await resumes, it reaches {mark} for the same {column}, then {commit} on the matching transaction."
                     + handle + data,
                     [*scope_proof, *proof, *lineage_proof], [item["comparisonPath"]], item["sources"],
                     uncertainty="The provider and broker must bind the cited APIs and honor the projected arguments.",
                     **({"condition": condition} if condition else {}))
        if item.get("publicationAwaitFailed"):
            text = (f"If the await of {reached} faults or is cancelled, the broker may already have accepted the send. "
                    f"If a later poll selects the same still-eligible {column} from {table}, it can attempt that send again. "
                    f"The comparison path reaches {mark} and {commit} after the send await resumes.")
        elif item.get("matchingCommitFailed") and item.get("selectedRowSuppliesArguments"):
            text = (f"After {reached} completes and the source finishes {mark}, the matching {commit} can fail. "
                    f"If a later poll selects the same still-eligible {column} from {table}, it can attempt that send again.")
        else:
            text = (f"If a later poll selects the same still-eligible {column} from {table}, {action} can run again. "
                    "The comparison path reaches the corresponding write and commit.")
        self.add("worker-reselection", prefix + text, [*scope_proof, *proof],
                 [item["failurePath"], item["comparisonPath"]], item["sources"],
                 uncertainty="Broker delivery and row eligibility after an interruption depend on the provider, broker and transaction.")

    def worker_publication_fields(self, item):
        fields = item.get("publicationFields", ())
        path = self.local_by_id.get(item["comparisonPath"]) or self.paths.get(item["comparisonPath"])
        if not fields or not path:
            return [], []
        events = self.trace(path)
        invocation = next((index for index, event in enumerate(events)
                           if event["outcome"] == "INVOKED" and event.get("result") == item.get("publicationOccurrence")), None)
        if invocation is None:
            return [], []
        labels, proof = [], []
        for field in fields:
            if label := self.lineage(field["value"], events[:invocation], self.commands_for(path), proof):
                labels.append(f"{code(field['field'])} to {label}")
        return labels, proof

    def worker_delivery_scope(self, path_id):
        path = self.local_by_id.get(path_id)
        if not path:
            return "", []
        scope = path["scope"]
        prefix = self.trace(path)[:scope["prefixEvents"]]
        active_calls = []
        for event in prefix:
            key = event["step"]
            if event["outcome"] == "INVOKED" and self.steps[key].get("boundSources"):
                active_calls.append(key)
            elif event.get("call") in active_calls:
                active_calls.remove(event["call"])
        if scope["call"] not in active_calls:
            active_calls.append(scope["call"])
        names = [name for key in active_calls if (name := self.source_call(key))]
        callers = {key.split("/", 1)[0] for key in active_calls}
        latest = {}
        for event in reversed(prefix):
            key = event["step"]
            if len(latest) >= 3:
                break
            if (key not in latest and key.split("/", 1)[0] in callers and event["outcome"] in {"TRUE", "FALSE"}
                    and self.text(key) not in {"true", "false"}):
                if condition := self.condition(key, event["outcome"] == "TRUE"):
                    latest[key] = condition
        conditions = " and ".join(reversed(latest.values()))
        root = code(method(self.root.get("symbol", "")))
        worker = f"the modeled path from {root}"
        premise = f"When {conditions}, {worker}" if conditions else worker.capitalize()
        binding = " enters the retained candidate " if scope["authority"] == "CONDITIONAL_SOURCE_CANDIDATE" else " enters "
        helper = code(method(scope["symbol"]))
        via = " via " + " then ".join(names) if names else ""
        return f"{premise}{binding}{helper}{via}. ", [*active_calls, *latest]

    def output(self):
        order = ("http-entry", "input", "http-outcome", "finite-mapping", "lookup-guard-return", "guard-return-bypass", "direct-effect-inventory",
                 "helper-return", "sql-read", "sql-row", "lookup", "argument-projection", "protocol-request",
                 "sql-write", "conditional-no-write", "callable-scope", "worker-control", "worker-poll", "worker-state",
                 "worker-reselection", "worker-row-delivery", "guarded_state_write", "await_bypass", "early_exit", "handled-exception-continuation", "http-commit-before-response", "transactional-write-completion", "effect-order", "error-path")
        groups = [iter(self.rows[kind].values()) for kind in order]
        rows = []
        while groups and len(rows) < LIMIT:
            remaining = []
            for items in groups:
                if (row := next(items, None)) is not None:
                    rows.append(row)
                    remaining.append(items)
            groups = remaining
        return [*rows[:LIMIT], *self.rows["composed-effect-inventory"].values()]


def derive(process, context, graph):
    """Use exact source citations for compact facts without including the reviewer graph."""
    facts = Facts(context, graph or {})
    is_http = process.get("entryKind") in {"HTTP_ENDPOINT", "SOURCE_HTTP_MAPPING"}
    worker = process.get("entryKind") in {"HOSTED_BACKGROUND_WORKER", "SCHEDULED_JOB"}
    facts.entry(process, context, is_http)
    facts.guard_bypasses()
    facts.effect_inventory()
    facts.composed_effect_inventory()
    if worker:
        facts.worker_controls()
    facts.catch_continuations()
    facts.structural_outcomes()
    traces = {}
    for path in facts.paths.values():
        if "NULL_ACCEPTING_NON_INSTANCE_DISPATCH_REQUIRED" in path["open"]:
            continue
        events = facts.trace(path)
        traces[path["id"]] = events
        facts.returns(path, events, is_http)
        facts.list_lookup(path, events)
        facts.object_arguments(path, events)
        facts.sql(path, events)
        facts.dispatches(path, events)
        facts.effects(path, events, is_http)
        facts.skipped_writes(path, events)
        if worker:
            facts.worker(path, events)
    MAPPINGS(facts, traces, is_http, code, value_text)
    for path in facts.local_paths:
        if "NULL_ACCEPTING_NON_INSTANCE_DISPATCH_REQUIRED" in path["open"]:
            continue
        events = facts.trace(path)
        facts.callable_scope(path)
        facts.list_lookup(path, events)
        facts.object_arguments(path, events)
        facts.sql(path, events)
        facts.dispatches(path, events)
        facts.callable_order(path, events)
    return facts.output()


def derive_write(process, context, graph, site):
    """Read evidence for one externally selected write within the retained graph."""
    facts = Facts(context, graph or {})
    commands = (command for groups in (facts.commands, facts.local_commands) for rows in groups.values()
                for command in rows if command["kind"] == "WRITE")
    candidates = {command["call"] for command in commands}
    candidates.update(key for key, row in facts.steps.items() if row.get("sourceSql", {}).get("verb") not in {None, "SELECT"})

    def matches(key):
        anchor = facts.steps[key]["site"]
        if any(anchor[field] != site.get(field) for field in ("file", "revision")):
            return False
        if (anchor["startLine"], anchor["endLine"], anchor["textDigest"]) == (
                site.get("startLine"), site.get("endLine"), site.get("textDigest")):
            return True
        if all(isinstance(site.get(field), int) for field in ("startByte", "endByte")):
            return site["startByte"] <= anchor["startByte"] < anchor["endByte"] <= site["endByte"]
        return False

    selected = [key for key in candidates if matches(key)]
    if len(selected) != 1:
        return []
    focus = selected[0]
    for path in (*facts.paths.values(), *facts.local_paths):
        events = facts.trace(path)
        facts.sql(path, events, focus)
        if not path.get("scope"):
            facts.skipped_writes(path, events, focus)
            facts.commit_before_response(path, events, facts.root_return(path, events), focus)
    return [row for row in facts.output() if row["kind"] in {"sql-write", "conditional-no-write", "http-commit-before-response", "transactional-write-completion"}]
