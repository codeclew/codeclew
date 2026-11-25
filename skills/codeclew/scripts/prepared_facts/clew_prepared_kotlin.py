"""Project bounded Kotlin source facts before a reader draft is authored."""

from collections import defaultdict
from dataclasses import dataclass, replace
from hashlib import sha256
from pathlib import Path
import re
import runpy


HERE = Path(__file__).parent
FAILURE = runpy.run_path(str(HERE / "clew_kotlin_syntax.py"))
SCHEDULE = runpy.run_path(str(HERE / "clew_schedule_syntax.py"))
KAFKA_SEND = (
    "callable:org/springframework/kafka/core/KafkaTemplate.send#jvm:",
    "method:class:org.springframework.kafka.core.KafkaTemplate#send(",
)
FLOW = runpy.run_path(str(HERE / "clew_prepared_kotlin_flow.py"))
Controls, Action = FLOW["Controls"], FLOW["Action"]
OPERANDS = runpy.run_path(str(HERE / "clew_prepared_kotlin_operands.py"))
PROTOCOLS = runpy.run_path(str(HERE / "clew_prepared_kotlin_protocols.py"))["Projection"]
HARD_GUARD = runpy.run_path(str(HERE / "clew_prepared_kotlin_hard_exits.py"))["derive"]
WRITE_LINEAGE = runpy.run_path(str(HERE / "clew_prepared_kotlin_write_lineage.py"))["project"]
Kotlin, Site, Unsupported = (FAILURE[key] for key in ("Kotlin", "Site", "Unsupported"))
encloses, name, strings, token_site = (
    FAILURE[key] for key in ("encloses", "name", "strings", "token_site"))
SYNTAX = FAILURE["SYNTAX_AUTHORITY"]
MAX_BODY_BYTES = 24 * 1024
MAX_ROWS = 2400
MAX_BODIES = 8
MAX_CALLBACKS = 12
MAX_STATES = 32
MAX_STEPS = 240
MAX_ACTIONS = 6
MAX_FACTS = {
    "ENTRY": 2,
    "EARLY_EXIT": 48,
    "HARD_EXIT": None,
    "GUARDED_EFFECTS": 72,
    "ORDERED_EFFECTS": 16,
    "EXCEPTION_BOUNDARY": 10,
    "VALUE_LINEAGE": 18,
    "CALLBACK_BOUNDARY": 12,
    "CALL_BOUNDARY": 10,
    "SQL_LITERAL": 14,
    "BROKER_PUBLICATION": 8,
    "WAIT_TRANSACTION_BOUNDARY": 12,
    "TRANSACTION_BOUNDARY": 14,
    "POSSIBLE_REPEAT_AFTER_MARK_FAILURE": 8,
    "POSSIBLE_LATER_SELECTION": 8,
    "SCOPE_LIMIT": 1,
}
ASSIGNMENTS = {"=", "+=", "-=", "*=", "/=", "%="}
BOUNDARY = {"when", "do", "break", "continue", "class", "fun", "object", "typealias", "unsupported"}
SCOPING = {
    "also": "passes the receiver to the lambda and yields the original receiver",
    "let": "passes the receiver to the lambda and yields the lambda result",
    "apply": "uses the value as lambda receiver and yields that receiver",
    "run": "evaluates the lambda and yields its result",
    "with": "uses the value as lambda receiver and yields the lambda result",
}
SCOPING_TARGET = re.compile(
    r"(?:callable:kotlin/(?:StandardKt(?:__StandardKt)?\.)?|"
    r"method:class:kotlin\.StandardKt(?:__StandardKt)?#)"
    r"(also|let|apply|run|with)(?:#jvm:|\()"
)


def compact(value):
    # Keep string contents unchanged. Suppress long or multiline snippets.
    value = value.strip()
    if not value or "\n" in value or "\r" in value or "`" in value or len(value) > 105:
        return None
    return value


def quote(value):
    return f"`{value}`"


def join(labels):
    return ", ".join(labels[:-1]) + " and " + labels[-1] if len(labels) > 1 else labels[0]


def same_file(first, second):
    return (first["file"], first["revision"]) == (second["file"], second["revision"])


class Ranges:
    """Use the author context to rejoin exact native occurrences to author citations."""

    def __init__(self, process, context):
        self.native = process.get("sources", {})
        self.occurrences = context.get("_sourceOccurrences", {})
        self.rows = {row["id"]: row for row in context.get("sourceRanges", ())
                     if re.fullmatch(r"S[0-9]{3,}", row.get("id", "")) and not row.get("peerService")}
        self.by_ref = {}
        self.by_identity = defaultdict(list)
        self.texts = {}
        for ref, row in self.native.items():
            if row.get("authority") == "EXACT_SNAPSHOT_TEXT" and not row.get("peerService"):
                self.by_identity[self.identity(row)].append(ref)
        for key, row in self.rows.items():
            for ref in row.get("references", ()):
                if ref in self.by_identity[self.identity(row)]:
                    self.by_ref[ref] = key
            if "references" in row:
                continue
            # Compatibility with the original author_context shape.
            for ref in self.by_identity[self.identity(row)]:
                expected = self.native[ref]["textDigest"]
                if self.literal(key, ref, expected) is not None:
                    self.by_ref[ref] = key

    @staticmethod
    def identity(row):
        return tuple(row.get(key) for key in ("file", "revision", "startLine", "endLine"))

    def literal(self, key, ref=None, expected=None):
        row = self.rows[key]
        original = self.occurrences.get(ref)
        if original and self.identity(original) == self.identity(row):
            candidates = [original["text"]]
        elif "text" in row:
            candidates = [row["text"]]
        elif (parent := self.rows.get(row.get("textIn"))) and "text" in parent and same_file(row, parent):
            start, end = row["startLine"] - parent["startLine"], row["endLine"] - parent["startLine"]
            candidates = []
            if not 0 <= start <= end <= parent["endLine"] - parent["startLine"]:
                return None
            lines = parent["text"].split("\n")
            candidates.append("\n".join(lines[start:end + 1]))
            if ref and (source := self.native.get(ref)):
                positions = source.get("startByte"), source.get("endByte")
                if all(type(part) is int for part in positions):
                    for alias in self.by_identity[self.identity(parent)]:
                        original = self.native[alias]
                        if (not source.get("parentSourceId")
                                or original.get("parentSourceId") != source["parentSourceId"]
                                or type(base := original.get("startByte")) is not int):
                            continue
                        first, last = (part - base for part in positions)
                        raw = parent["text"].encode()
                        if 0 <= first < last <= len(raw):
                            try:
                                candidates.append(raw[first:last].decode())
                            except UnicodeDecodeError:
                                continue
        else:
            return None
        for value in candidates:
            for variant in (value, value + "\n"):
                if expected and "sha256:" + sha256(variant.encode()).hexdigest() == expected:
                    return variant
        return None

    def for_ref(self, ref):
        if ref not in self.texts:
            key = self.by_ref.get(ref)
            self.texts[ref] = self.literal(key, ref, self.native[ref].get("textDigest")) if key else None
        return self.texts[ref]

    def locate(self, ref, host, unit):
        item = self.native.get(ref)
        if not item or not same_file(host, item) or (text := self.for_ref(ref)) is None:
            return None
        if (host.get("parentSourceId") and host.get("parentSourceId") == item.get("parentSourceId")
                and all(type(row.get("startByte")) is int for row in (host, item))):
            start = item["startByte"] - host["startByte"]
            raw = unit.text.encode()
            if 0 <= start < len(raw) and raw[start:start + len(text.encode())] == text.encode():
                offset = len(raw[:start].decode())
                return Site(offset, offset + len(text), unit.file)
        return unit.locate({"text": text, "startLine": item["startLine"] - host["startLine"] + 1,
                            "endLine": item["endLine"] - host["startLine"] + 1})


@dataclass(frozen=True)
class Note:
    text: str
    site: Site
    citations: tuple
    negation: str = ""


@dataclass(frozen=True)
class Invocation:
    site: Site
    selector: Site
    receiver: Site | None
    member: str
    spelling: str
    arity: int = 0
    actuals: tuple = ()
    callbacks: tuple = ()


@dataclass(frozen=True)
class PathState:
    guards: tuple = ()
    effects: tuple = ()
    exit: tuple | None = None
    bindings: tuple = ()
    open: bool = False
    fragment: bool = False


class Body:
    def __init__(self, projector, ref, label, rows):
        text = projector.ranges.for_ref(ref)
        if text is None or len(text.encode()) > MAX_BODY_BYTES:
            raise Unsupported()
        self.projector = projector
        self.ref, self.label = ref, label
        self.ranges = projector.ranges
        source = self.ranges.native[ref]
        self.unit = Kotlin(text, source["file"])
        self.host = source
        scope = Site(0, len(text), self.unit.file)
        self.function = self.unit.select_site(scope)
        if self.function is None:
            raise Unsupported()
        self.scope = self.unit.function_site(self.function)
        self.sites = defaultdict(set)
        self.trailing_lambdas = {}
        self.compiler_targets = set()
        self.hard_guards = {}
        self.calls = {}
        self.call_parts = {}
        self.invocations = {}
        self.sql_operands = {}
        self.sql_verbs = {}
        self.sql_shapes = {}
        self.callback_scopes = {}
        self.callback_inline = set()
        self.chain_order = {}
        self.operand_regions = {}
        self.sparse_regions = set()
        self.rows = []
        for row in rows:
            positions = {item["role"]: at for item in row["sites"]
                         if (at := self.ranges.locate(item["reference"], self.host, self.unit))
                         and encloses(self.scope, at)}
            if positions:
                self.rows.append((row, positions))
                for item in row["sites"]:
                    if at := positions.get(item["role"]):
                        self.sites[at].add(self.ranges.by_ref[item["reference"]])

    def suite(self):
        if self.function.expression_body:
            statement, _ = self.unit.statement(self.function.body + 1, self.function.end + 1, 0)
            return (statement,)
        return self.unit.suite(self.function.body + 1, self.function.end)

    def note(self, site):
        text = compact(self.unit.excerpt(site))
        first, last = self.unit.token_indices.get(site.start), self.unit.token_ends.get(site.end)
        if not text and first is not None and last is not None:
            flat = OPERANDS["flat"](self.unit.tokens[first:last + 1], self.unit.text)
            if flat and len(flat) <= 290:
                text = flat
        return self.text_note(site, OPERANDS["code"](text)) if text else None

    def text_note(self, site, text):
        cites = tuple(sorted(self.sites.get(site, ()))) or (self.ranges.by_ref[self.ref],)
        return Note(text, site, cites)

    def arguments(self, site, arguments):
        visible = []
        for raw in FAILURE["groups"](arguments):
            parameter, value = FAILURE["named_argument"](raw)
            literal = OPERANDS["sql_literal"](value)
            if literal and (shape := OPERANDS["sql_shape"](literal[1])):
                sql = self.text_note(token_site((literal[0],)), OPERANDS["sql_sentence"](shape))
                self.sql_operands.setdefault((site, sql.site), sql)
                self.sql_verbs.setdefault((site, sql.site), shape[0])
                self.sql_shapes.setdefault((site, sql.site), shape)
                visible.append(shape[0] + " for " + OPERANDS["code"](shape[1]))
            elif literal and (query := OPERANDS["raw_sql"](literal[1])):
                sql = self.text_note(token_site((literal[0],)), query[1])
                self.sql_operands.setdefault((site, sql.site), sql)
                self.sql_verbs.setdefault((site, sql.site), query[0].split()[0])
                visible.append(query[0])
            elif (text := OPERANDS["flat"](value, self.unit.text)) and len(text) <= 105:
                visible.append(OPERANDS["code"](f"{parameter} = {text}" if parameter else text))
        return tuple(visible)

    def callback_site(self, first, after):
        opening = first
        if self.unit.words[first + 1:first + 3] == ("@", "{"):
            opening += 2
        if self.unit.words[opening] == "{" and self.unit.pairs.get(opening) == after - 1:
            return self.unit.site(first, after)
        return None

    def invocation_stages(self, first, stop):
        """Identify immediate selectors and receivers in one complete postfix chain."""
        words = self.unit.words
        if name(words[first]) or words[first] in {"this", "super"}:
            index, callee = first + 1, first
        elif words[first] == "(":
            index, callee = self.unit.pairs[first] + 1, None
        else:
            return ()
        receiver = None
        parts = []
        while index < stop:
            word = words[index]
            if word in {".", "?."} and index + 1 < stop and name(words[index + 1]):
                receiver = self.unit.site(first, index)
                callee = index + 1
                index += 2
                continue
            if word == "[" and index > first:
                index, callee = self.unit.pairs[index] + 1, None
                continue
            if word == "<" and callee is not None:
                parsed = FAILURE["type_end"](words, callee)
                if parsed is None or parsed >= stop or words[parsed] not in {"(", "{"}:
                    return ()
                index = parsed
                continue
            if word == "(":
                end = self.unit.pairs[index]
                if end >= stop:
                    return ()
                arguments = tuple(self.unit.tokens[index + 1:end])
                callbacks = []
                for argument in FAILURE["groups"](arguments):
                    _, value = FAILURE["named_argument"](argument)
                    if value:
                        a = self.unit.token_indices[value[0].start]
                        b = self.unit.token_ends[value[-1].end] + 1
                        if callback := self.callback_site(a, b):
                            callbacks.append(callback)
                if parts and callee is not None and parts[-1].selector.start == self.unit.tokens[callee].start:
                    # This postfix invocation applies to a previous result.
                    receiver, callee = parts[-1].site, None
                parts.append(self.invocation(first, end + 1, callee, receiver, arguments, tuple(callbacks)))
                index = end + 1
                continue
            labeled = name(word) and words[index + 1:index + 3] == ("@", "{")
            if word == "{" or labeled:
                opening = index + 2 if labeled else index
                end = self.unit.pairs[opening]
                if end >= stop:
                    return ()
                callback = self.callback_site(index, end + 1)
                if callback is None:
                    return ()
                prior = parts[-1] if parts else None
                if (prior and prior.site.end == self.unit.tokens[index - 1].end
                        and (callee is None or prior.selector.start == self.unit.tokens[callee].start)):
                    parts[-1] = replace(prior, site=self.unit.site(first, end + 1),
                                        selector=Site(prior.selector.start, callback.end, callback.file),
                                        callbacks=(*prior.callbacks, callback), arity=prior.arity + 1)
                else:
                    parts.append(self.invocation(first, end + 1, callee, receiver, (), (callback,)))
                index = end + 1
                continue
            return ()
        return tuple(parts)

    def invocation(self, first, after, callee, receiver, arguments, callbacks):
        site = self.unit.site(first, after)
        arity = sum(1 for _ in FAILURE["groups"](arguments))
        if callee is None:
            return Invocation(site, site, receiver, "", "the result invocation", arity,
                              self.arguments(site, arguments), callbacks)
        member = self.unit.words[callee]
        spelling = (OPERANDS["flat"](self.unit.tokens[first:callee + 1], self.unit.text)
                    if receiver is None or "(" not in self.unit.excerpt(receiver) else member)
        return Invocation(site, self.unit.site(callee, after), receiver, member, spelling or member, arity,
                          self.arguments(site, arguments), callbacks)

    @staticmethod
    def describe_call(part):
        label = OPERANDS["code"](part.spelling)
        if part.actuals:
            label += " with " + join(list(part.actuals))
        if part.callbacks:
            label += " and its callback" if part.actuals else " with its callback"
        return label

    def lexical_call(self, tokens):
        """Recognize standalone invocations and save their callback owners."""
        if not tokens:
            return None
        site = token_site(tokens)
        if site in self.calls:
            return self.calls[site]
        first = self.unit.token_indices[tokens[0].start]
        stop = self.unit.token_indices[tokens[-1].start] + 1
        parts = self.invocation_stages(first, stop)
        if not parts:
            return None
        for part in parts:
            previous = self.invocations.get(part.selector)
            if not previous or part.site.start <= previous.site.start:
                self.invocations[part.selector] = part
            for callback in part.callbacks:
                old = self.trailing_lambdas.get(callback)
                if not old or (old.selector.start == part.selector.start and part.site.start <= old.site.start):
                    self.trailing_lambdas[callback] = part
        descriptions = [self.describe_call(part) for part in parts]
        for previous, after in zip(parts, parts[1:]):
            before = (None if previous.callbacks else self.note(previous.site))
            before = before or self.text_note(previous.site, self.describe_call(previous))
            later = self.describe_call(after)
            if after.receiver == previous.site:
                relation = "supplies the receiver"
            elif after.receiver and (receiver := self.note(after.receiver)):
                relation = f"is evaluated while computing receiver {receiver.text}"
            else:
                continue
            order = f"{before.text} {relation} before {later} can be invoked."
            self.chain_order.setdefault((previous.site, after.site), (order, before, self.text_note(after.selector, later)))
        result = None if any(part.callbacks for part in parts) else self.note(site)
        result = result or self.text_note(site, " then ".join(descriptions))
        self.calls[site] = result
        self.call_parts[site] = parts
        return result

    def operand_facts(self, scope):
        def region(at):
            candidates = [site for site in self.operand_regions if encloses(site, at)]
            return self.operand_regions[min(candidates, key=lambda site: site.end - site.start)] if candidates else scope

        for key, query in self.sql_operands.items():
            call, _ = key
            owner = self.note(call) or next((note for site, note in self.calls.items()
                                             if encloses(site, call)), None)
            if owner:
                shape = self.sql_shapes.get(key)
                bound = shape is not None
                edge = ({"_edge": {"kind": shape[0], "table": shape[1], "site": owner.citations[0]}}
                        if bound and "__clew_kotlin_value_" not in shape[1] else {})
                self.projector.add("SQL_LITERAL", f"In {region(call)}, {owner.text} carries a literal {query.text}. "
                                   + ("Provider execution and storage outcome remain open." if bound else
                                      "SQL binding, provider execution and storage outcome remain open."),
                                   [owner, query], scope="SOURCE_SQL_OPERAND" if bound else "SQL_LITERAL_BINDING_OPEN", **edge)
        for text, earlier, later in self.chain_order.values():
            self.projector.add("ORDERED_EFFECTS", f"In {region(later.site)}, {text}", [earlier, later],
                               scope="CHAINED_CALL_RECEIVER")

    def action(self, statement):
        if statement.kind not in {"expression", "return", "throw"} or not statement.expression:
            return None
        tokens = statement.expression
        words = strings(tokens)
        first = self.unit.token_indices[tokens[0].start]
        direct = self.unit.container[first]
        split = next((index for index, word in enumerate(words) if word in ASSIGNMENTS
                      and self.unit.container[first + index] == direct), None)
        if split is not None and statement.kind == "expression":
            if words[0] in {"val", "var"}:
                return self.lexical_call(tokens[split + 1:])
            if not set(words[:split]) & {"if", "when", "try", "for", "while", "->"}:
                return self.note(statement.site)
        if any(row["kind"] == "ASSIGNMENT" and sites.get("assignment") == statement.site
               for row, sites in self.rows):
            return self.note(statement.site)
        return self.lexical_call(tokens)

    def values(self, statement, path, scope):
        tokens = statement.expression
        words = strings(tokens)
        bindings = dict(path.bindings)
        if words[:1] in {("val",), ("var",)}:
            if len(words) < 2 or not name(words[1]):
                return replace(path, bindings=())
            variable = words[1]
            bindings.pop(variable, None)
            if words[0] == "val" and "=" in words[2:]:
                start = words.index("=", 2) + 1
                # The statement declaration and local name are the only origins asserted here.
                # A getter, function result or mutable object referenced by the initializer stays open.
                if start < len(words) and (value := self.note(token_site(tokens[start:]))):
                    bindings[variable] = value
            return replace(path, bindings=tuple(bindings.items()))
        if len(words) > 1 and words[1] in ASSIGNMENTS:
            bindings.pop(words[0], None)
            expressions = (tokens[2:],)
        elif statement.kind == "return":
            expressions = (tokens,)
        elif (invocation := FAILURE["invocation"](tokens)) and self.lexical_call(tokens):
            callee, arguments = invocation
            expressions = tuple(FAILURE["named_argument"](argument)[1] for argument in arguments)
            if len(callee) >= 3 and callee[1].text in {".", "?."}:
                expressions = (callee[:1], *expressions)
        else:
            expressions = ()
        for expression in expressions:
            spelling = OPERANDS["flat"](expression, self.unit.text)
            if not spelling or not OPERANDS["RECEIVER"].fullmatch(spelling):
                continue
            variable = expression[0].text
            if not (origin := bindings.get(variable)):
                continue
            if use := self.note(statement.site):
                self.projector.add("VALUE_LINEAGE",
                                   f"In {scope}, {use.text} uses the local value `{variable}` "
                                   f"initialized from {origin.text}.",
                                   [origin, use], scope="LEXICAL_LOCAL_BINDING")
        return replace(path, bindings=tuple(bindings.items()))

    def continuation(self, suite, state, scope, callback=None, caught=False):
        states = [state]
        for statement in suite:
            if self.projector.steps <= 0:
                self.projector.limits.setdefault("NORMAL_ORDER_SCAN", (self.ranges.by_ref[self.ref],))
                return [replace(path, open=True) for path in states]
            terminal = [path for path in states if path.exit or path.open]
            active = [path for path in states if not path.exit and not path.open]
            if not active and any(path.open for path in terminal):
                # Preserve independently resolvable later source as conditional on reaching it.
                # Separate its order and skip set from the prefix that reached the unknown site.
                active = [PathState(fragment=True)]
            reached = terminal
            for path in active:
                if self.projector.steps <= 0:
                    reached.append(replace(path, open=True))
                    continue
                self.projector.steps -= 1
                reached.extend(self.step(statement, path, scope, callback, caught))
            if len(reached) > MAX_STATES:
                self.projector.limits.setdefault("NORMAL_ORDER_SCAN", (self.ranges.by_ref[self.ref],))
                self.projector.paths([replace(path, open=True) for path in reached[:MAX_STATES]], scope)
                states = [PathState(fragment=True)]
            else:
                states = reached
        return [replace(path, bindings=state.bindings) for path in states]

    def expression_control(self, statement):
        if not statement.expression:
            return None
        tokens = statement.expression
        words = strings(tokens)
        first = self.unit.token_indices[tokens[0].start]
        start = 0 if statement.kind == "return" else next((
            index + 1 for index, word in enumerate(words) if word in ASSIGNMENTS
            and self.unit.container[index + first] == self.unit.container[first]), len(words))
        if words[start:start + 1] not in {("if",), ("try",)}:
            return None
        try:
            nested, end = self.unit.statement(first + start, first + len(words), 0)
        except Unsupported:
            return None
        return nested if end == first + len(words) else None

    def elvis_exit(self, statement, path, scope, callback, caught):
        if statement.kind != "expression" or not statement.expression:
            return None
        tokens = statement.expression
        words = strings(tokens)
        first = self.unit.token_indices[tokens[0].start]
        parent = self.unit.container[first]
        operators = {word: index for index, word in reversed(list(enumerate(words)))
                     if word in {"?:", "="} and self.unit.container[first + index] == parent}
        pivot = operators.get("?:", len(words))
        start = operators.get("=", -1) + 1
        if start >= pivot or words[pivot + 1:pivot + 2] not in {("return",), ("throw",)}:
            return None
        if words[start] in {"if", "when", "try"} or "?:" in words[start:pivot]:
            return None
        lhs, control = tokens[start:pivot], token_site(tokens[start:pivot])
        if not (value := self.note(control)):
            return None
        try:
            abrupt, end = self.unit.statement(first + pivot + 1, first + len(tokens), 0)
        except Unsupported:
            return None
        if end != first + len(tokens):
            return None
        # The left-hand expression is evaluated once before the null split.
        if action := self.lexical_call(lhs):
            path = replace(path, effects=(*path.effects, action))
        guard = replace(value, text=f"{value.text} is null", negation=f"{value.text} is non-null")
        on_null = replace(path, guards=(*path.guards, (guard, True)))
        present = replace(path, guards=(*path.guards, (guard, False)))
        if words[:1] == ("val",) and start == 3 and name(words[1]):
            bindings = dict(present.bindings)
            bindings[words[1]] = value
            present = replace(present, bindings=tuple(bindings.items()))
        return [*self.step(abrupt, on_null, scope, callback, caught), present]

    @staticmethod
    def words(statement):
        return strings(statement.expression)

    @staticmethod
    def guard_text(guard, holds):
        return guard.text if holds else guard.negation or guard.text + " is false"

    def guard_bindings(self, statement, state, scope, guard):
        bindings = dict(state.bindings)
        previous = ""
        seen = set()
        for token in statement.expression:
            if (token.text in bindings and token.text not in seen and previous not in {".", "?.", "@"}
                    and "{" not in self.words(statement)):
                seen.add(token.text)
                origin = bindings[token.text]
                self.projector.add("VALUE_LINEAGE", f"In {scope}, the check {guard.text} uses local "
                                   f"{quote(token.text)}, initialized from {origin.text}.", [guard, origin],
                                   scope="LEXICAL_GUARD_BINDING")
            previous = token.text

    def decision(self, statement):
        if statement.kind != "if" or not statement.expression:
            return None
        if not (condition := self.note(token_site(statement.expression))):
            return None
        arms = ((True, statement.body), (False, statement.alternative))
        constant = self.words(statement)
        if constant in {("true",), ("false",)}:
            arms = tuple((arm, suite) for arm, suite in arms if arm == (constant == ("true",)))
        return condition, arms

    def hard_guard(self, statement):
        if statement.site not in self.hard_guards:
            self.hard_guards[statement.site] = HARD_GUARD(self, statement)
        return self.hard_guards[statement.site]

    def iteration(self, statement, callback, caught, scope=None, throw_future=()):
        loop = self.note(token_site(statement.expression)) if statement.expression else None
        if loop:
            label = "an iteration of " + loop.text + (f" in {scope}" if scope else "")
            self.operand_regions[statement.site] = label
            if statement.site not in self.sparse_regions:
                self.sparse_regions.add(statement.site)
                self.control(statement.body, label, loop.citations, "ITERATION", callback, caught, throw_future)
            return loop, label

    def potential_actions(self, suite, limit=4):
        shape = Controls(self, "", ()).shape(suite, "CALLABLE")
        return list(dict.fromkeys(action.note for action in shape.actions))[:limit]

    def handler_actions(self, suite, limit=3):
        actions = self.potential_actions(suite, limit)
        return [self.note(action.site) or action for action in actions]

    def protected_source(self, suite):
        """Pick a direct source expression and its possible normal successors."""
        controls = Controls(self, "", ())
        candidates = []
        for index, statement in enumerate(suite):
            if effect := self.action(statement):
                candidates.append((index, effect))
            if not controls.statement_shape(statement, "TRY_BODY").resumes:
                break
        if not candidates:
            return None, (), []
        selected = candidates[0]
        wait_site = None
        for candidate in candidates:
            effect = candidate[1]
            part = next((part for part in self.call_parts.get(effect.site, ())
                         if part.member == "await" or part.member == "get" and part.arity == 2
                         and any("TimeUnit." in value for value in part.actuals)), None)
            if part:
                selected = candidate
                wait_site = self.text_note(part.selector, self.describe_call(part))
                break
        index, earlier = selected
        after = (self.potential_actions(suite[index + 1:], limit=MAX_ACTIONS - 1)
                 if controls.statement_shape(suite[index], "TRY_BODY").resumes else [])
        return earlier, (wait_site,) if wait_site else (), after

    def control(self, suite, scope, proof=(), region="CALLABLE", callback=None, caught=False, throw_future=()):
        controls = Controls(self, scope, proof, throw_future)
        controls.visit(suite, region, callback=callback, catches=caught)
        if controls.flow_budget <= 0:
            self.projector.limits.setdefault("STRUCTURAL_SCAN", tuple(proof))

    def step(self, statement, path, scope, callback, caught):
        if hard := self.hard_guard(statement):
            if not hard.true:
                if hard.verified:
                    return [replace(path, exit=("CAUGHT_OPEN" if caught else "THROW", hard.failure))]
                return [replace(path, open=True)]
            guard = self.text_note(hard.failure.site, hard.true)
            tail = replace(path, guards=(*path.guards, (guard, True)))
            if statement.kind == "return" and callback is None:
                return [replace(tail, exit=("RETURN", self.note(statement.site) or hard.failure))]
            return [tail]
        if statement.kind == "if":
            if not (decision := self.decision(statement)):
                return [replace(path, open=True)]
            condition, arms = decision
            self.guard_bindings(statement, path, scope, condition)
            alternative = []
            for arm, suite in arms:
                selected = replace(path, guards=(*path.guards, (condition, arm)))
                alternative.extend(self.continuation(suite, selected, scope, callback, caught))
            return alternative
        if statement.kind == "try":
            return self.protected(statement, path, scope, callback, caught)
        if statement.kind in {"while", "for"}:
            # Analyze a local iteration separately. Its transfers cannot prove outer skips or order.
            if iteration := self.iteration(statement, callback, caught, scope):
                loop, label = iteration
                self.projector.paths(self.continuation(statement.body, PathState(), label, callback, caught),
                                     label, loop.citations)
            return [replace(path, open=True)]
        if nested := self.expression_control(statement):
            if callback and statement.kind == "return":
                return [replace(path, open=True)]
            reached = self.step(nested, path, scope, callback, caught)
            result = []
            for item in reached:
                if item.exit or item.open:
                    result.append(item)
                elif statement.kind == "return":
                    terminal = self.note(statement.site) or self.text_note(statement.site, "after evaluating the selected return expression")
                    result.append(replace(item, exit=("RETURN", terminal)))
                elif action := self.action(statement):
                    result.append(replace(item, effects=(*item.effects, action)))
                else:
                    result.append(item)
            return result
        if alternatives := self.elvis_exit(statement, path, scope, callback, caught):
            return alternatives
        words = strings(statement.expression)
        if statement.kind in BOUNDARY or words and (
                words[0] in BOUNDARY or words[:2] in {("return", "@"), ("suspend", "fun")}):
            return [replace(path, open=True)]
        if statement.expression and any(
                word in {"if", "when", "try", "throw", "return"}
                and self.unit.container[index] == self.unit.container[first]
                for first in (self.unit.token_indices[statement.expression[0].start],)
                for index, word in enumerate(words, first)):
            return [replace(path, open=True)]
        path = self.values(statement, path, scope)
        if action := self.action(statement):
            path = replace(path, effects=(*path.effects, action))
        if statement.kind in {"return", "throw"}:
            labeled = words[:1] == ("@",)
            if labeled and (not callback or words[1:2] != (callback,)):
                return [replace(path, open=True)]
            if statement.kind == "return" and callback and not labeled:
                return [replace(path, open=True)]
            if terminal := self.note(statement.site):
                kind = ("CAUGHT_OPEN" if caught and statement.kind == "throw" else
                        "CALLBACK_RETURN" if labeled else statement.kind.upper())
                return [replace(path, exit=(kind, terminal))]
            return [replace(path, open=True)]
        return [path]

    def protected(self, statement, path, scope, callback, caught):
        protected = self.continuation(statement.body, path, scope, callback, caught or bool(statement.handlers))
        earlier, selected, later = self.protected_source(statement.body) if statement.handlers else (None, (), [])
        for _, handler, suite in statement.handlers:
            note = self.note(handler)
            if not note:
                continue
            label = f"{note.text} in {scope}"
            self.projector.paths(self.continuation(suite, PathState(), label, callback, caught),
                                 label, note.citations)
            actions = self.handler_actions(suite)
            if earlier:
                if later:
                    description = (f"In {scope}, if evaluation at {earlier.text} throws and {note.text} handles it, "
                                   f"this try-body path bypasses {join([item.text for item in later])}.")
                else:
                    description = (f"In {scope}, {note.text} can handle an exception from evaluation at "
                                   f"{earlier.text} if it propagates to this handler and the catch is selected.")
                proof = [earlier, *selected, note, *later]
            else:
                description = f"In {scope}, {note.text} can handle a matching exception from its try body."
                proof = [note]
            if actions:
                description += " Possible handler actions include " + join([action.text for action in actions])
                description += ", each subject to its local conditions."
            self.projector.add("EXCEPTION_BOUNDARY", description, [*proof, *actions], scope="MATCHING_EXCEPTION")
        if not statement.cleanup:
            return [replace(item, open=True) if item.exit and item.exit[0] == "CAUGHT_OPEN" else item
                    for item in protected]
        cleanup = self.first_cleanup(statement)
        if cleanup:
            self.projector.add("EXCEPTION_BOUNDARY",
                               f"In {scope}, leaving the try/catch enters finally and can reach {cleanup.text}.",
                               [cleanup, self.text_note(statement.site, scope)], scope="LOCAL_UNWIND")
        result = []
        for item in protected:
            if item.open or item.exit and item.exit[0] == "CAUGHT_OPEN":
                result.append(replace(item, open=True))
                continue
            for after in self.continuation(statement.cleanup, replace(item, exit=None), scope, callback, caught):
                result.append(after if after.exit or after.open or after.fragment else replace(after, exit=item.exit))
        return result

    def first_cleanup(self, statement):
        first = statement.cleanup[0]
        return self.action(first) or self.note(first.site)

    def receiver_note(self, site):
        if site is None:
            return None
        preceding = next((part for part in self.invocations.values() if part.site == site and part.callbacks), None)
        if preceding:
            return self.text_note(site, self.describe_call(preceding))
        return self.note(site)

    def deferred(self, compiler, *, project=True):
        """Resolve each lambda's immediate source invocation before analyzing its body."""
        retained = defaultdict(set)
        inline = defaultdict(set)
        for row, sites in self.rows:
            if row["kind"] == "CALLBACK" and (callback := sites.get("callback")) and (
                    handoff := sites.get("argumentSite")) and encloses(handoff, callback):
                retained[callback].add(handoff)
        if compiler:
            events = defaultdict(list)
            for row in self.projector.process.get("facts", ()):
                if (row.get("kind") == "EVENT" and row.get("declaration") == compiler
                        and type(row.get("ordinal")) is int):
                    events[row["ordinal"]].append(row)
                    if (row.get("event") == "CALL" and row.get("resolution") == "COMPILER_EXACT"
                            and (match := SCOPING_TARGET.match(row.get("target") or ""))):
                        for call in self.event_sites(row):
                            inline[match[1]].add(call)
            for ordinal, entries in events.items():
                if len(entries) != 1 or entries[0].get("event") != "DEFERRED":
                    continue
                event = entries[0]
                parents = events.get(event.get("parentOrdinal"), ())
                if len(parents) != 1 or parents[0].get("event") != "CALL" or parents[0]["ordinal"] >= ordinal:
                    continue
                inner, outer = (self.event_sites(row) for row in (event, parents[0]))
                if len(inner) == len(outer) == 1 and encloses(outer[0], inner[0]):
                    retained[inner[0]].add(outer[0])
        for spans in retained.values():
            for span in sorted(spans, key=lambda site: (site.start, site.end)):
                first, last = self.unit.token_indices.get(span.start), self.unit.token_ends.get(span.end)
                if first is not None and last is not None:
                    self.lexical_call(tuple(self.unit.tokens[first:last + 1]))
        visited = set()
        while available := set(self.trailing_lambdas) - visited:
            if len(visited) >= MAX_CALLBACKS:
                self.projector.limits.setdefault("CALLBACK_SCOPE_SCAN", (self.ranges.by_ref[self.ref],))
                break
            site = min(available, key=lambda at: (at.start, at.end))
            visited.add(site)
            owner = self.trailing_lambdas[site]
            if not owner.member or not (callback := FAILURE["callback_body"](self.unit, site, owner.site, self.function)):
                continue
            first = self.unit.token_indices[site.start]
            if self.unit.words[first] == "{" and self.unit.words[first - 1:first] != ("@",):
                callback = replace(callback, label=owner.member)
            name_ = quote(owner.member)
            call = self.text_note(owner.selector, name_)
            body = self.text_note(site, "callback")
            receiver = self.receiver_note(owner.receiver)
            target = f"{name_} on receiver {receiver.text}" if receiver else name_
            evidence = [call, body, *([receiver] if receiver else ())]
            selected = [at for at in inline[owner.member] if at in {owner.site, owner.selector}]
            if len(selected) == 1:
                self.callback_inline.add(site)
                scope = f"the Kotlin scope lambda passed to {name_}"
                if project:
                    self.projector.add("CALLBACK_BOUNDARY",
                                   f"When invoked, {name_} {SCOPING[owner.member]}. "
                                   + (f"Its receiver expression is {receiver.text}." if receiver else ""),
                                   [*evidence, self.text_note(selected[0], "selected Kotlin call")],
                                   scope="COMPILER_SELECTED_KOTLIN_SCOPE")
            else:
                scope = f"the callback passed to {target}"
                if project:
                    self.projector.add("CALLBACK_BOUNDARY",
                                   f"Source passes a callback to {target}. Its invocation and timing depend on the callee.",
                                   evidence, scope="DEFERRED_BODY")
            self.operand_regions[site] = scope
            try:
                suite = self.unit.suite(callback.start, callback.end)
            except Unsupported:
                continue
            self.callback_scopes[site] = owner, callback, suite
            if not project:
                continue
            proof = tuple(dict.fromkeys(source for item in evidence for source in item.citations))
            self.control(suite, scope, proof, "CALLBACK", callback.label)
            self.projector.paths(self.continuation(suite, PathState(), scope, callback.label), scope, proof)

    def event_sites(self, row):
        sites = set()
        for ref in row.get("sources", ()):
            if (site := self.ranges.locate(ref, self.host, self.unit)) and encloses(self.scope, site):
                self.sites[site].add(self.ranges.by_ref[ref])
                sites.add(site)
        return sorted(sites, key=lambda site: (site.start, site.end))


class Projector:
    def __init__(self, process, context):
        self.process, self.context = process, context
        self.ranges = Ranges(process, context)
        self.facts = defaultdict(dict)
        self.limits = {}
        self.steps = MAX_STEPS

    def add(self, kind, text, notes=(), extra=(), *, identity=None, **metadata):
        sources = list(dict.fromkeys([*extra, *(source for note in notes for source in note.citations)]))
        key = identity if identity is not None else (text, tuple(sources))
        if not sources or key in self.facts[kind]:
            return
        limit = MAX_FACTS[kind]
        if limit is not None and len(self.facts[kind]) >= limit:
            self.limits.setdefault(kind, tuple(sources))
            return
        self.facts[kind][key] = {"kind": kind, "text": text, "sources": sources, **metadata}

    def paths(self, paths, scope, proof=()):
        possible = dict.fromkeys((effect.site, effect) for path in paths if not path.fragment for effect in path.effects)
        alternatives = tuple(effect for _, effect in possible)
        for path in paths:
            if len(path.guards) > 4:
                continue
            predicates = [Body.guard_text(guard, holds) for guard, holds in path.guards]
            conditions = [guard for guard, _ in path.guards]
            opening = f"In {scope}, if the later fragment is reached" if path.fragment else f"In {scope}"
            context = opening + (f", when {join(predicates)}" if predicates else "")
            effects = path.effects[:MAX_ACTIONS]
            reached = {effect.site for effect in path.effects}
            skipped = ([] if path.open or path.fragment else
                       [effect for effect in alternatives if effect.site not in reached][:MAX_ACTIONS])
            if path.exit and any(guard.negation for guard in conditions):
                kind, terminal = path.exit
                normal = "reaches callback exit" if kind == "CALLBACK_RETURN" else "reaches"
                tail = ""
                if skipped:
                    tail = " This path bypasses " + join([effect.text for effect in skipped]) + "."
                if kind == "CAUGHT_OPEN":
                    tail += " Local handling and any later continuation depend on the matching catch."
                if path.guards or skipped or kind in {"THROW", "CAUGHT_OPEN"}:
                    self.add("EARLY_EXIT", f"{context}, source {normal} {terminal.text}.{tail}",
                             [*conditions, terminal, *skipped], proof, scope="SOURCE_PATH")
            if len(effects) >= 2:
                self.add("ORDERED_EFFECTS",
                         f"{context}, the normal source sequence is "
                         + " → ".join(effect.text for effect in effects)
                         + ". This order describes the source continuation when prerequisite evaluations return.",
                         [*conditions, *effects], proof, scope="NORMAL_SOURCE_ORDER")

    def entry(self, body):
        source_ids = set(self.context.get("entrySources", ())) | {self.ranges.by_ref[body.ref]}
        trigger = self.context.get("trigger") or {}
        for key in sorted(source_ids):
            if not (row := self.ranges.rows.get(key)):
                continue
            if not row["file"].endswith((".kt", ".kts")) or row["revision"] != body.host["revision"]:
                continue
            text = row.get("text")
            if not text:
                text = next((value for ref, cite in self.ranges.by_ref.items() if cite == key
                             and (value := self.ranges.for_ref(ref))), None)
            if not text:
                continue
            if self.context.get("entryKind") == "SCHEDULED_JOB":
                modes = SCHEDULE["scheduled_arguments"](text)
                for mode, names in SCHEDULE["SCHEDULE_MODES"].items():
                    for attribute in names:
                        value = modes.get(attribute)
                        if not value or not (spelling := compact(value)):
                            continue
                        description = f"The scheduling declaration specifies {mode.lower()} {quote(spelling)}"
                        if (unit := modes.get("timeUnit")) and (unit := compact(unit)):
                            description += f" with {quote(unit)}"
                        if (zone := modes.get("zone")) and (zone := compact(zone)):
                            description += f" in zone {quote(zone)}"
                        self.add("ENTRY", description + ". Effective scheduling and activation remain open.",
                                 extra=[key], scope="DECLARED_SCHEDULE")
                if modes:
                    return
            if trigger.get("authority") == "RETAINED_DECLARATION_ANNOTATION":
                annotation = trigger.get("annotation", "")
                short = annotation.rsplit(".", 1)[-1]
                if short and re.search(r"(?<!\w)@" + re.escape(short) + r"\b", text):
                    self.add("ENTRY", f"The source declares {quote('@' + short)} on the entry. "
                             "Effective registration and delivery remain open.",
                             extra=[key], scope="DECLARED_ENTRY")
                    return

    def syntax_links(self, body):
        links = defaultdict(set)
        for row, sites in body.rows:
            if (row["kind"] == "CALL" and row["details"].get("resolution") == FAILURE["UNIQUE_SYNTAX"]
                    and row["details"].get("candidateCount") == 1 and (call := sites.get("call"))):
                target = next((site["reference"] for site in row["sites"] if site["role"] == "target"), None)
                if target and (source := self.ranges.native.get(target)) and source["revision"] == body.host["revision"]:
                    links[call].add((target, row["label"]))
        for row, sites in body.rows:
            kind = row["kind"]
            if (kind not in {"ARGUMENT", "DEFAULT_ARGUMENT"} or (call := sites.get("call")) not in links
                    or len(links[call]) != 1 or row["details"].get("resolution") != FAILURE["UNIQUE_SYNTAX"]):
                continue
            target, label = next(iter(links[call]))
            refs = {item["role"]: item["reference"] for item in row["sites"]}
            if row["details"].get("target") != label or refs.get("target", target) != target:
                continue
            role = "argument" if kind == "ARGUMENT" else "defaultValue"
            bindings = [refs.get(role), refs.get("parameter"), refs.get("target", target)]
            if not all(ref and self.ranges.for_ref(ref) is not None for ref in bindings):
                continue
            actual = compact(self.ranges.for_ref(refs[role]))
            member = compact(row["label"])
            occurrence = body.note(call)
            if actual and member and occurrence:
                description = (f"maps {quote(actual)} to parameter {quote(member)}"
                               if kind == "ARGUMENT" else
                               f"uses the declared default {quote(actual)} for parameter {quote(member)}")
                self.add("VALUE_LINEAGE", f"If {occurrence.text} selects {quote(label)}, it {description}.",
                         [occurrence], [self.ranges.by_ref[ref] for ref in bindings],
                         scope="SYNTAX_CALL_CANDIDATE")
        return [(target, label, body.note(call)) for call, values in links.items() if len(values) == 1
                for target, label in values if body.note(call)]

    @staticmethod
    def selected_role(target):
        kotlin = re.match(r"callable:(.+)/([^./]+)\.([^#.]+)#jvm:", target)
        java = re.match(r"method:class:([^#]+)#([^(]+)\(", target)
        if kotlin:
            owner = kotlin[1].replace("/", ".") + "." + kotlin[2]
            member = kotlin[3]
        elif java:
            owner, member = java.groups()
        else:
            return None
        jdbc = FAILURE["JDBC"].get(owner, {}).get(member)
        if jdbc == "COMMIT":
            return "requests a JDBC commit", "Durable completion remains open."
        if jdbc == "ROLLBACK":
            return "requests a JDBC rollback", "The database outcome remains open."
        if target.startswith(KAFKA_SEND):
            return "submits a Kafka publication request", "Delivery remains open."
        if owner in {"java.util.concurrent.Future", "java.util.concurrent.CompletableFuture"} and member == "get":
            timed = "(JLjava/util/concurrent/TimeUnit;)" in target
            statement = "waits for the selected future's result" + (" with the supplied timeout and unit" if timed else "")
            return statement, "Whether the wait succeeds remains open."
        if owner == "org.springframework.kafka.support.Acknowledgment" and member == "acknowledge":
            return "requests acknowledgement", "Broker state remains open."
        return None

    def compiler_links(self, body, declaration, native_bodies):
        rows = self.process.get("facts", ())
        for row in rows:
            target = row.get("target")
            if row.get("declaration") != declaration or not target or row.get("event") != "CALL":
                continue
            if row.get("kind") == "EVENT" and row.get("resolution") == "COMPILER_EXACT":
                if role := self.selected_role(target):
                    for site in body.event_sites(row):
                        if note := body.note(site):
                            statement, limit = role
                            self.add("CALL_BOUNDARY", f"When invoked, the compiler-selected call at {note.text} "
                                     f"{statement}. {limit}",
                                     [note], scope="COMPILER_SELECTED_CALL_SITE")
            if row.get("kind") != "CALL" or row.get("relationshipAuthority") not in FAILURE["COMPILER_EDGES"]:
                continue
            if row.get("resolution") not in FAILURE["EXACT_RESOLUTIONS"]:
                continue
            linked = row.get("targetMethodId")
            bodies = native_bodies[linked]
            if len(bodies) != 1:
                continue
            for site in body.event_sites(row):
                if note := body.note(site):
                    yield next(iter(bodies)), target, linked, note

    def declarations(self):
        rows = [row for row in self.process.get("entryKotlinSourceContext", ())
                if row.get("authority") == SYNTAX]
        retained = {site["reference"]: row["label"] for row in rows if row["kind"] == "BODY"
                    for site in row["sites"] if site["role"] == "declaration"}
        roots = {site["reference"]: row["label"] for row in rows
                 if row["kind"] == "BODY" and row["details"].get("root")
                 for site in row["sites"] if site["role"] == "declaration"}
        declared = defaultdict(set)
        for row in self.process.get("facts", ()):
            kind = row.get("kind")
            if kind == "BODY" or (kind == "BODY_SOURCE" and row.get("authority") == "COMPILER_EXACT_CALLBACK_TARGET"):
                declared[row.get("declaration")].update(ref for ref in row.get("sources", ()) if self.ranges.by_ref.get(ref)
                                                    and self.ranges.native[ref]["file"].endswith((".kt", ".kts")))
        compiler_root = self.process.get("root")
        if not roots and compiler_root:
            roots = {ref: self.process.get("entrySymbol") or self.context.get("title") or "the selected callable"
                     for ref in sorted(declared[compiler_root])}
        return rows, retained, roots, declared

    def derive(self):
        rows, retained, roots, declared = self.declarations()
        compiler_root = self.process.get("root")
        if len(roots) != 1 or len(rows) > MAX_ROWS:
            return []
        root, label = next(iter(roots.items()))
        pending = [(root, label, compiler_root, None)]
        visited = set()
        while pending and len(visited) < MAX_BODIES:
            ref, label, declaration, call = pending.pop(0)
            if ref in visited or ref not in self.ranges.by_ref:
                continue
            visited.add(ref)
            try:
                body = Body(self, ref, label, rows)
                suite = body.suite()
            except Unsupported:
                continue
            scope = quote(body.function.name) if ref == root else f"the retained body selected if {call.text} dispatches to {quote(label)}"
            proof = (self.ranges.by_ref[ref], *(call.citations if call else ()))
            if ref == root:
                self.entry(body)
            if declaration:
                for event in self.process.get("facts", ()):
                    if event.get("declaration") == declaration and event.get("kind") == "EVENT":
                        sites = body.event_sites(event)
                        if (event.get("event") == "CALL" and event.get("resolution") == "COMPILER_EXACT"
                                and (target := event.get("target"))):
                            body.compiler_targets.update((site, target) for site in sites)
            body.control(suite, scope, proof)
            self.paths(body.continuation(suite, PathState(), scope), scope, proof)
            body.deferred(declaration)
            PROTOCOLS(body, suite, scope, declaration).derive()
            body.operand_facts(scope)
            pending.extend((target, name_, None, site) for target, name_, site in self.syntax_links(body)
                           if target in retained)
            if declaration:
                pending.extend(self.compiler_links(body, declaration, declared))
        if any(ref not in visited for ref, _, _, _ in pending):
            self.limits.setdefault("LOCAL_BODY_SCAN", (self.ranges.by_ref[root],))
        if self.limits:
            reasons = ", ".join(kind.lower().replace("_", " ") for kind in self.limits)
            self.add("SCOPE_LIMIT", f"More source sites remain beyond the bounded projection for {reasons}.",
                     extra=list(dict.fromkeys(source for group in self.limits.values() for source in group))[:8],
                     scope="PROJECTION_BUDGET")
        return [fact for kind in MAX_FACTS for fact in self.facts[kind].values()]


def derive(process, context):
    """Return source-linked author facts over verified retained Kotlin source."""
    facts = Projector(process, context).derive()
    if any(fact["kind"] == "TRANSACTION_BOUNDARY" for fact in facts):
        facts.extend(FocusedWrite(process, context, None).derive())
    return facts


class FocusedWrite(Projector):
    """Join one write occurrence to retained paths through a callback and its helpers."""

    def __init__(self, process, context, site):
        super().__init__(process, context)
        self.site = site
        self.output = {}

    @staticmethod
    def span(row):
        for candidate in (row, row.get("occurrence") or {}):
            start, end = candidate.get("startByte"), candidate.get("endByte")
            if type(start) is type(end) is int and 0 <= start < end:
                return start, end
        return None

    def location(self, body):
        site, host = self.site, body.host
        if not same_file(site, host):
            return None
        container = self.span(host) or self.span(self.ranges.occurrences.get(body.ref, {}))
        selected = self.span(site)
        if selected is None:
            spans = {bounds for ref, identifier in self.ranges.by_ref.items() if identifier == site["id"]
                     and self.ranges.native[ref].get("textDigest") == site["textDigest"]
                     and (bounds := self.span(self.ranges.native[ref]) or self.span(self.ranges.occurrences.get(ref, {})))}
            if len(spans) == 1:
                selected = spans.pop()
        if not container or not selected:
            return None
        raw, text = body.unit.text.encode(), site["text"].encode()
        first, last = (byte - container[0] for byte in selected)
        if (container[1] - container[0] == len(raw) and 0 <= first < last <= len(raw) and raw[first:last] == text
                and host["startLine"] + raw[:first].count(b"\n") == site["startLine"]
                and host["startLine"] + raw[:last - 1].count(b"\n") == site["endLine"]):
            start = len(raw[:first].decode())
            literal = site["text"]
            trimmed = literal.strip()
            if not trimmed:
                return None
            start += len(literal) - len(literal.lstrip())
            return Site(start, start + len(trimmed), body.unit.file)
        return None

    def record(self, kind, text, trail=(), notes=(), **fields):
        anchor = (self.site["id"],) if self.site else ()
        sources = list(dict.fromkeys((*anchor, *trail, *(cite for note in notes for cite in note.citations))))
        key = (kind, text, tuple(sources), fields.get("condition"))
        self.output.setdefault(key, {"kind": kind, "text": text, "sources": sources, **fields})

    def links(self, body, declaration, retained, declared):
        candidates = defaultdict(set)
        for target, label, call in self.syntax_links(body):
            if target in retained:
                candidates[call.site].add((target, label, None, call))
        if declaration:
            for target, label, symbol, call in self.compiler_links(body, declaration, declared):
                candidates[call.site].add((target, label, symbol, call))
        for row in self.process.get("entryLinkedDefinitions", {}).get("rows", ()):
            if (row.get("kind") != "LOCAL_CONTINUATION" or row.get("stageAuthority") not in
                    {"COMPILER_SELECTED_HELPER", "STATIC_UNIQUE_HELPER_BINDING"}):
                continue
            steps = defaultdict(list)
            for item in row.get("steps", ()):
                steps[item["role"]].append(item.get("reference"))
            call, target = steps["Call site"], steps["Declared implementation"]
            if len(call) != 1 or len(target) != 1 or not self.ranges.for_ref(target[0]):
                continue
            if (at := self.ranges.locate(call[0], body.host, body.unit)) and encloses(body.scope, at):
                body.sites[at].add(self.ranges.by_ref[call[0]])
                if note := body.note(at):
                    candidates[at].add((target[0], "the retained helper", None, note))
        for at, matches in candidates.items():
            targets = {item[0] for item in matches}
            if len(targets) == 1:
                yield at, min(matches, key=lambda item: (item[2] is None, item[1]))

    @staticmethod
    def reached(body, path, site):
        return any(note.site == site or any(site in {part.site, part.selector} or (
            site.start == part.selector.start and encloses(part.selector, site))
            for part in body.call_parts.get(note.site, ())) for note in path.effects)

    @staticmethod
    def guards_before(path, site):
        return tuple((note, holds) for note, holds in path.guards if note.site.start < site.start)

    @staticmethod
    def conditions(guards):
        return join(list(dict.fromkeys(Body.guard_text(note, holds) for note, holds in guards))) if guards else None

    def follow(self, body, paths, at, trail, inherited):
        arriving = [path for path in paths if self.reached(body, path, at)]
        if not arriving:
            return [], ()
        first = self.guards_before(arriving[0], at)
        common = set(first).intersection(*(self.guards_before(path, at) for path in arriving[1:]))
        skips = []
        for path in paths:
            if path.open or path.exit and path.exit[0] == "CAUGHT_OPEN" or self.reached(body, path, at):
                continue
            guards = self.guards_before(path, at)
            if not any((note, not holds) in common for note, holds in guards):
                continue
            ending = path.exit[1] if path.exit else None
            premise = self.conditions((*inherited, *guards))
            action = f" reaches {ending.text} before the selected write" if ending else " bypasses the selected write on this path"
            notes = [*(note for note, _ in (*inherited, *guards)), *([ending] if ending else [])]
            skips.append((f"When {premise}, source{action}.", trail, notes))
        return [(path, (*inherited, *self.guards_before(path, at))) for path in arriving], tuple(skips)

    def mutation(self, body, paths, at):
        if at is None:
            return None
        candidates = {}
        for part in body.invocations.values():
            operands = [(key, verb) for key, verb in body.sql_verbs.items() if key[0] == part.site]
            if operands and any(verb not in {"INSERT", "UPDATE", "DELETE", "MERGE"} for _, verb in operands):
                continue
            sql = [body.sql_operands[key] for key, _ in operands]
            if at in {part.site, part.selector} or any(encloses(note.site, at) for note in sql):
                candidates[part.site] = (body.text_note(part.selector, body.describe_call(part)), sql)
        if not candidates and at not in body.call_parts and any(note.site == at for path in paths for note in path.effects):
            candidates[at] = (body.note(at), [])
        return next(iter(candidates.items())) if len(candidates) == 1 else None

    def structural_path(self, body, prefix, target, callback, trail, inherited):
        """Follow the direct lexical continuation when a prior region opens path state."""
        skips = []
        projector = self

        class TargetExits(Controls):
            def emit_arm(self, *args):
                pass

            def emit_hard_exit(self, *args):
                pass

            def exit(self, statement, guards, subsequent, alternatives, region, callback, catches):
                if (statement.kind not in {"return", "throw"} or statement.kind == "throw" and catches
                        or not any(item.note.site == target.site for item in subsequent)):
                    return
                words = self.body.words(statement)
                if statement.kind == "return" and (words[:1] == ("@",) or callback) and words[:2] != ("@", callback):
                    return
                ending = self.body.note(statement.site)
                premise = projector.conditions((*inherited, *guards))
                if ending and premise:
                    notes = [*(note for note, _ in (*inherited, *guards)), ending]
                    skips.append((f"If this check is reached and {premise}, source reaches {ending.text} before the selected write.",
                                  trail, notes))

        controls = TargetExits(body, "the retained write body", trail)
        guards = []
        for statement in prefix:
            if controls.flow_budget <= 0 or not controls.statement_shape(statement, "CALLABLE").resumes:
                return None
            if statement.kind == "while" and body.words(statement) == ("true",):
                return None
            nested = body.expression_control(statement) or statement
            if decision := body.decision(nested):
                condition, arms = decision
                continuing = [arm for arm, suite in arms if controls.shape(suite, "CALLABLE").resumes]
                if len(continuing) == 1:
                    guards.append((condition, continuing[0]))
        controls.visit(prefix, "CALLABLE", future=(Action(target, "CALLABLE"),), callback=callback)
        return tuple(guards), tuple(skips)

    @staticmethod
    def formation(body, prefix, action):
        first, last = body.unit.token_indices.get(action.start), body.unit.token_ends.get(action.end)
        if first is None or last is None or not (invocation := FAILURE["invocation"](body.unit.tokens[first:last + 1])):
            return []
        parameters = {value[0].text for argument in invocation[1]
                      for _, value in [FAILURE["named_argument"](argument)] if len(value) == 1 and name(value[0].text)}
        bindings = []
        for statement in reversed(prefix):
            words = body.words(statement)
            if len(words) < 4 or words[0] not in {"val", "var"} or words[1] not in parameters:
                continue
            variable = words[1]
            parameters.remove(variable)
            if words[0] == "val" and "=" in words[2:]:
                tokens = statement.expression[words.index("=", 2) + 1:]
                if tokens and (origin := body.note(token_site(tokens))):
                    bindings.append((variable, origin))
        return bindings

    def value_lineage(self, body, prefix, action, note, trail, guards, transaction, normal_prefix=None):
        selected = {guard: Body.guard_text(guard, holds) for guard, holds in guards}
        for value in WRITE_LINEAGE(body, prefix, action, Controls, FAILURE):
            if any(guard in selected and selected[guard] != text for text, guard in value.conditions):
                continue
            statements = ". ".join(text[0].upper() + text[1:] for text, _ in value.steps)
            premises = list(dict.fromkeys([*selected.values(), *([normal_prefix] if normal_prefix else ()),
                                          *(text for text, _ in value.conditions)]))
            proof = [*(guard for guard, _ in guards), *(guard for _, guard in value.conditions),
                     *(piece for _, links in value.steps for piece in links)]
            self.record("write_lineage", f"{statements} at {note.text}.", trail, [note, transaction, *proof],
                        condition=join(premises),
                        uncertainty="The cited path applies if its calls and evaluations complete normally and the database write joins this transaction.")

    def local_value_lineage(self, body, suite, trail, guards, transaction):
        if transaction is None:
            return
        for index, statement in enumerate(suite):
            if index >= 72 or len(self.output) >= 12:
                break
            if not (action := body.action(statement)):
                continue
            for part in body.call_parts.get(action.site, ()):
                if len(self.output) >= 12:
                    break
                if any(site == part.site and verb in {"INSERT", "UPDATE", "DELETE", "MERGE"}
                       for (site, _), verb in body.sql_verbs.items()):
                    note = body.text_note(part.selector, body.describe_call(part))
                    self.value_lineage(body, suite[:index], part.site, note, trail, guards, transaction)

    @staticmethod
    def loop_regions(body, suite, guards, controls, depth=0):
        """Select a possible iteration under the normal lexical prefix."""
        if depth > 8:
            return
        for statement in suite:
            if controls.flow_budget <= 0:
                return
            local = body.expression_control(statement) or statement
            if local.kind == "for" and local.expression:
                if note := body.note(token_site(local.expression)):
                    iteration = replace(note, text=f"source reaches an iteration of {note.text}")
                    yield local.body, (*guards, (iteration, True))
            elif decision := body.decision(local):
                note, arms = decision
                for holds, arm in arms:
                    yield from FocusedWrite.loop_regions(body, arm, (*guards, (note, holds)), controls, depth + 1)
                viable = [holds for holds, arm in arms if controls.shape(arm, "CALLABLE").resumes]
                if len(viable) == 1:
                    guards = (*guards, (note, viable[0]))
            elif local.kind == "try":
                yield from FocusedWrite.loop_regions(body, local.body, guards, controls, depth + 1)
            if not controls.statement_shape(statement, "CALLABLE").resumes:
                return

    def capture(self, body, suite, callback, paths, at, trail, inherited, transaction, bypasses):
        direct = next((index for index, statement in enumerate(suite) if at and encloses(statement.site, at)), None)
        exact = body.action(suite[direct]) if direct is not None else None
        probe = [PathState(effects=(exact,))] if exact else []
        if not (mutation := self.mutation(body, [*paths, *probe], at)):
            return False
        action, (note, sql) = mutation
        if note is None:
            return False
        reached, skips = self.follow(body, paths, action, trail, inherited)
        normal_prefix = None
        if not reached and probe and self.reached(body, probe[0], action):
            structural = self.structural_path(body, suite[:direct], exact, callback, trail, inherited)
            if structural:
                guards, skips = structural
                normal_prefix = "preceding evaluations and any loop iterations complete normally"
                normal_tail = Controls(body, "the retained write body", trail).shape(suite[direct + 1:], "CALLABLE").resumes
                reached = [(replace(probe[0], open=not normal_tail), (*inherited, *guards))]
        if reached:
            for text, sources, notes in (*bypasses, *skips):
                self.record("write_skip", text, sources, notes)
        direct_effect = exact and self.reached(body, probe[0], action)
        formation = self.formation(body, suite[:direct], action) if reached and direct_effect else []
        for path, guards in reached:
            clauses = [clause for clause in (self.conditions(guards), normal_prefix) if clause]
            premise = join(clauses) if clauses else None
            conditions = {"condition": premise} if premise else {}
            notes = [note, *(guard for guard, _ in guards)]
            self.record("write_request", f"The callback path reaches {note.text} at the selected write.", trail, notes, **conditions)
            if premise:
                self.record("write_guard", f"The cited callback path reaches the selected write when {premise}.", trail, notes)
            for variable, origin in formation:
                self.record("write_formation", f"The selected write receives {quote(variable)}, initialized from {origin.text}.",
                            trail, [*notes, origin], **conditions)
            if direct_effect and transaction:
                self.value_lineage(body, suite[:direct], action, note, trail, guards, transaction, normal_prefix)
            for detail in sql:
                self.record("write_sql", f"The SQL argument contains {detail.text}.", trail, [*notes, detail], **conditions)
            if transaction and self.normal(path):
                self.record("write_commit", "On this path, the selected write runs inside the transaction callback. "
                            "If the write participates in that transaction and the callback returns normally, the transaction template can complete.",
                            trail, [*notes, transaction], **conditions,
                            uncertainty="A separate read may see the change if the write changes a row, the commit succeeds, and the read observes it.")
        return bool(reached)

    @staticmethod
    def normal(path):
        return not path.open and (path.exit is None or path.exit[0] in {"RETURN", "CALLBACK_RETURN"})

    def derive(self):
        rows, retained, roots, declared = self.declarations()
        if len(roots) != 1 or len(rows) > MAX_ROWS:
            return []
        ref, label = next(iter(roots.items()))
        pending = [(ref, label, self.process.get("root"), (), (), (), None, ())]
        reached = False
        for _ in range(MAX_BODIES):
            if not pending or self.steps <= 0:
                break
            ref, label, declaration, ancestry, trail, guards, transaction, bypasses = pending.pop(0)
            if ref in ancestry or ref not in self.ranges.by_ref:
                continue
            try:
                body = Body(self, ref, label, rows)
                suite = body.suite()
            except Unsupported:
                continue
            at = self.location(body) if self.site else None
            proof = (*trail, self.ranges.by_ref[ref])
            links = list(self.links(body, declaration, retained, declared))
            protocols = PROTOCOLS(body, suite, label, declaration)
            protocols.selected()
            regions = [(suite, None, guards, proof, transaction, bypasses)]
            visited_callbacks = set()
            for suite, callback, inherited, trail, transaction, bypasses in regions:
                paths = [path for path in body.continuation(suite, PathState(), label, callback) if not path.fragment]
                if self.site:
                    reached |= self.capture(body, suite, callback, paths, at, trail, inherited, transaction, bypasses)
                else:
                    self.local_value_lineage(body, suite, trail, inherited, transaction)
                controls = Controls(body, label, trail)
                for inner, local in self.loop_regions(body, suite, inherited, controls):
                    if len(regions) >= MAX_CALLBACKS:
                        break
                    regions.append((inner, callback, local, trail, transaction, bypasses))
                for call, (target, name_, symbol, note) in links:
                    handoff = (*trail, *note.citations)
                    continuing, skipped = self.follow(body, paths, call, handoff, inherited)
                    for path, local in continuing:
                        pending.append((target, name_, symbol, (*ancestry, ref), (*trail, *note.citations), local,
                                        transaction if self.normal(path) else None, (*bypasses, *skipped)))
                body.deferred(declaration, project=False)
                for site, (owner, block, inner) in body.callback_scopes.items():
                    if site in visited_callbacks:
                        continue
                    binding = protocols.binding(owner)
                    opens_transaction = binding and binding.kind == "transaction"
                    if not opens_transaction and site not in body.callback_inline:
                        continue
                    continuing, skipped = self.follow(body, paths, owner.site, trail, inherited)
                    for path, local in continuing:
                        visited_callbacks.add(site)
                        note = binding.evidence if opens_transaction else transaction
                        regions.append((inner, block.label, local, trail, note if self.normal(path) else None, (*bypasses, *skipped)))
        if not self.site:
            return list(self.output.values())[:12]
        if not reached:
            return [{"kind": "write_boundary", "text": "The retained Kotlin callback does not establish a reachable call at this exact write occurrence.",
                     "sources": [self.site["id"]]}]
        order = {kind: index for index, kind in enumerate(("write_guard", "write_request", "write_commit", "write_lineage",
                                                         "write_formation", "write_sql", "write_skip"))}
        return sorted(self.output.values(), key=lambda row: order[row["kind"]])[:18]


def derive_write(process, context, graph, site):
    """Project the guards and normal source order of a verified Kotlin write site."""
    identifier, text = site.get("id"), site.get("text")
    if (not site.get("file", "").endswith((".kt", ".kts")) or not isinstance(text, str)
            or "sha256:" + sha256(text.encode()).hexdigest() != site.get("textDigest")
            or identifier not in {row["id"] for row in context.get("sourceRanges", ())}):
        return []
    return FocusedWrite(process, context, site).derive()
