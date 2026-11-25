"""Connect a scheduled iteration to a narrowly bound heap-state continuation."""

from dataclasses import dataclass, replace


MAX_DEPTH = 8
MAX_STATEMENTS = 160
MAX_FACTS = 16
KAFKA = "org.springframework.kafka.core.KafkaTemplate"


@dataclass(frozen=True)
class Key:
    loop: object
    properties: tuple


@dataclass(frozen=True)
class Access:
    receiver: str
    key: Key
    site: object


@dataclass(frozen=True)
class Flow:
    evidence: tuple = ()
    conditions: tuple = ()
    reads: tuple = ()
    writes: tuple = ()
    invalidated: frozenset = frozenset()
    transaction: bool = False


def project(unit, root, syntax, protocols, bind, native_calls):
    """Recognize ordered source effects without assuming scheduler or transaction activation."""
    owner = unit.enclosing_owner(root)
    if owner is None or root.expression_body:
        return []
    projector = ScheduledState(unit, syntax, protocols, bind, owner, native_calls)
    try:
        body = unit.suite(root.body + 1, root.end)
        projector.walk(body, {param.name: None for param in root.params}, Flow(),
                       (root.name,), 0)
    except syntax["Unsupported"]:
        return []
    return list(projector.rows.values())


class ScheduledState:
    def __init__(self, unit, syntax, protocols, bind, owner, native_calls):
        self.unit, self.bind, self.native_calls = unit, bind, native_calls
        self.functions, self.fields = unit.members(owner)
        self.name, self.site, self.words = (syntax[key] for key in ("name", "token_site", "strings"))
        self.invoke, self.named, self.split = (syntax[key] for key in ("invocation", "named_argument", "groups"))
        self.callback = syntax["callback_body"]
        self.make_call = protocols["jvm_target"]
        self.foreach = protocols["FOREACH"]
        self.transactions, self.futures = protocols["TRANSACTIONS"], protocols["FUTURES"]
        self.remote = syntax["REMOTE"]
        self.rows, self.steps = {}, MAX_STATEMENTS

    def phrase(self, site):
        return " ".join(self.unit.excerpt(site).split())[:160].replace('`', "")

    def add(self, kind, text, flow, *sites, uncertainty=None):
        if len(self.rows) >= MAX_FACTS:
            return
        proof = tuple(dict.fromkeys((*flow.evidence, *sites)))
        if not all(refs := [self.bind(site) for site in proof]):
            return
        conditions = " and ".join(dict.fromkeys(flow.conditions))
        row = {"kind": kind, "text": text, "authority": "BOUNDED_KOTLIN_SCHEDULED_STATE",
               "sources": list(dict.fromkeys(refs))}
        if conditions:
            row["condition"] = conditions
        if uncertainty:
            row["uncertainty"] = uncertainty
        self.rows.setdefault((kind, text, conditions, tuple(refs)), row)

    def value(self, tokens, env):
        words = self.words(tokens)
        if not words or not (origin := env.get(words[0])):
            return None
        tail = words[1:]
        if len(tail) % 2 or any(op != "." or not self.name(field)
                               for op, field in zip(tail[::2], tail[1::2])):
            return None
        return Key(origin.loop, (*origin.properties, *tail[1::2]))

    def receiver(self, labels, env):
        if len(labels) == 1 and labels[0] not in env:
            return self.fields.get(labels[0])
        if len(labels) == 3 and labels[:2] == ("this", "."):
            return self.fields.get(labels[2])
        return None

    def map_part(self, tokens, env):
        words = self.words(tokens)
        if "[" not in words or words[-1:] != ("]",):
            return None
        opening = words.index("[")
        receiver = self.receiver(words[:opening], env)
        if receiver and receiver.storage == "MAP" and (key := self.value(tokens[opening + 1:-1], env)):
            return Access(receiver.name, key, self.site(tokens))
        return None

    def call(self, tokens):
        if not tokens or "(" not in self.words(tokens):
            return None
        index = self.unit.token_indices[tokens[0].start]
        words = self.words(tokens)
        end = self.unit.pairs.get(index + words.index("("), -1) - index + 1
        if not 0 < end <= len(tokens):
            return None
        if parsed := self.invoke(tokens[:end]):
            callee, args = parsed
            return self.words(callee), args, self.site(tokens[:end]), end
        return None

    def receiver_call(self, tokens, env, names):
        if not (call := self.call(tokens)):
            return None
        labels, args, site, end = call
        if labels[-1] not in names or labels[-2:-1] != (".",):
            return None
        if field := self.receiver(labels[:-2], env):
            return field, labels[-1], args, site, end
        return None

    def access(self, tokens, env, *, write=False):
        if indexed := self.map_part(tokens, env):
            return indexed
        names = {"put", "set"} if write else {"get"}
        if call := self.receiver_call(tokens, env, names):
            field, _, args, site, end = call
            if (field.storage == "MAP" and len(args) == (2 if write else 1) and end == len(tokens)
                    and (key := self.value(args[0], env))):
                return Access(field.name, key, site)
        return None

    def native(self, site):
        return tuple((target, kind) for source, target, kind in self.native_calls
                     if site.file == source.file and site.start <= source.start < source.end <= site.end)

    def external(self, tokens, env):
        if not (call := self.call(tokens)):
            return None
        labels, _, site, end = call
        native = self.native(site)
        selected = any(kind in {"HTTP", "KAFKA"} or (api := self.make_call(target)) and (
            api[1] in self.remote.get(api[0], ()) or api[:2] == (KAFKA, "send") or
            api[0] in self.futures and api[1] == "get") for target, kind in native)
        evidence = ()
        kafka = any((api := self.make_call(target)) and api[:2] == (KAFKA, "send") for target, _ in native)
        if not selected and len(labels) > 2 and labels[-2] == "." and (field := self.receiver(labels[:-2], env)):
            member = labels[-1]
            kafka = field.type == KAFKA and member == "send"
            selected = field.stable and (
                member in self.remote.get(field.type, ()) or kafka or field.type in self.futures and member == "get")
            evidence = (field.site,)
        if not selected:
            return None
        tail = tokens[end:]
        if (kafka and self.words(tail[:3]) == (".", "get", "(")
                and (wait := self.call(tail[1:])) and wait[3] == len(tail) - 1):
            site = self.site(tokens)
        return site, evidence

    def foreach_loop(self, name, header, suite, env, flow, stack, depth):
        child = {**env, name: Key(header, ())}
        premise = "a scheduled iteration supplies " + '`' + name + '`'
        self.walk(suite, child, replace(flow, evidence=(*flow.evidence, header),
                                        conditions=(*flow.conditions, premise)), stack, depth + 1)

    def callbacks(self, statement, env, flow, stack, depth):
        tokens = statement.expression
        if not tokens or tokens[-1].text != "}":
            return
        start, last = self.unit.token_indices[tokens[0].start], self.unit.token_ends[tokens[-1].end]
        opening = next((index for index in range(start, last) if self.unit.pairs.get(index) == last), None)
        if opening is None:
            return
        prefix = tuple(self.unit.tokens[start:opening])
        direct = self.words(prefix)
        if direct[-2:] == ("(", ")"):
            direct = direct[:-2]
        if len(direct) < 3 or direct[-2] != ".":
            return
        callback = self.callback(self.unit, self.unit.site(opening, last + 1),
                                 statement.site, None)
        if not callback:
            return
        suite = self.unit.suite(callback.start, callback.end)
        if direct[-1] == "forEach" and len(callback.params) == 1 and any(
                self.foreach.match(target) for target, _ in self.native(statement.site)):
            self.foreach_loop(callback.params[0][0], statement.site, suite, env, flow, stack, depth)
            return
        if direct[-1] in {"execute", "executeWithoutResult"}:
            field = self.receiver(direct[:-2], env)
            if field and field.stable and field.type in self.transactions:
                self.walk(suite, {**env, **dict.fromkeys(name for name, _ in callback.params)},
                          replace(flow, transaction=True, evidence=(*flow.evidence, field.site, statement.site),
                                  conditions=(*flow.conditions, "the retained transaction template invokes its callback")),
                          stack, depth + 1)

    def helper(self, tokens, env, flow, stack, depth):
        if not (call := self.call(tokens)):
            return
        labels, args, site, end = call
        if end != len(tokens) or not (len(labels) == 1 or labels[:2] == ("this", ".") and len(labels) == 3):
            return
        name = labels[-1]
        if name in stack or (len(labels) == 1 and name in env):
            return
        choices = self.functions.get(name, ())
        if len(choices) != 1 or not (helper := choices[0]) or helper.expression_body or not self.unit.fixed_dispatch(helper):
            return
        if any(param.vararg for param in helper.params) or len(helper.params) != len(args):
            return
        values, parameter_sites = {}, []
        for param, arg in zip(helper.params, args):
            named, value = self.named(arg)
            if named and named != param.name:
                return
            values[param.name] = self.value(value, env)
            if values[param.name] is not None:
                parameter_sites.append((param, self.site(value)))
        if not parameter_sites:
            return
        child = replace(flow, evidence=(*flow.evidence, site, self.unit.function_site(helper)),
                        conditions=(*flow.conditions, "the call selects the retained same-owner implementation"))
        for param, actual in parameter_sites:
            self.add("SCHEDULED_ARGUMENT",
                     f"The source handoff passes {self.code(actual)} to the {param.name} parameter of {name}.",
                     child, actual, param.site,
                     uncertainty="The local call must be reached and the argument must evaluate normally.")
        suite = self.unit.suite(helper.body + 1, helper.end)
        self.walk(suite, values, replace(child, reads=(), writes=()), (*stack, name), depth + 1)

    def code(self, site):
        return '`' + self.phrase(site) + '`'

    def later(self, suite, index):
        if index + 1 < len(suite) and suite[index + 1].kind in {"expression", "return", "throw"}:
            return suite[index + 1].site
        return None

    def outcome(self, external, flow, following):
        call, evidence = external
        for write in flow.writes:
            field = self.fields[write.receiver]
            proof = (field.site, write.site, call, *evidence)
            self.add("HEAP_WRITE_BEFORE_CALL",
                     f"The source reaches the in-memory map write {self.code(write.site)} before {self.code(call)}.",
                     flow, *proof, uncertainty="Earlier operations and the map write must complete normally.")
            if following:
                self.add("HEAP_FAILURE_BYPASS",
                         f"If {self.code(call)} throws out of this source statement, the normal continuation to "
                         f"{self.code(following)} in the same block is bypassed.",
                         flow, *proof, following)
            for read in flow.reads:
                if ((read.receiver, read.key) != (write.receiver, write.key) or read.site.start >= write.site.start
                        or write.receiver in flow.invalidated):
                    continue
                rollback = " A database rollback does not undo this in-memory mutation." if flow.transaction else ""
                self.add("SAME_KEY_HEAP_RETRY",
                         f"A later scheduled iteration can read the value written to {self.code(write.site)} "
                         f"even if {self.code(call)} failed after the write." + rollback,
                         flow, *proof, read.site,
                         uncertainty="Requires the same receiver and evaluated key, the earlier write to have completed, "
                                     "and no intervening write, removal, clearing or replacement.")

    def cleanup_writes(self, suites, env):
        names = set()
        for suite in suites:
            for statement in suite:
                tokens = statement.expression
                words = self.words(tokens)
                for index, word in enumerate(words[:-1]):
                    if word in self.fields and word not in env and words[index + 1] in {".", "["}:
                        # Distinguish a known direct read from potential mutation or escape.
                        if words[index + 1:index + 3] not in {(".", "get"), (".", "containsKey")}:
                            names.add(word)
                names.update(self.cleanup_writes((statement.body, statement.alternative, statement.cleanup,
                                                  *(branch for _, _, branch in statement.handlers)), env))
        return names

    def with_write(self, flow, write):
        # A later direct write to this receiver can replace the tracked value.
        return replace(flow, writes=(*(item for item in flow.writes if item.receiver != write.receiver), write))

    def guard_read(self, tokens, env):
        split = next((word for word in self.words(tokens) if word in {"==", "!=", "<", ">", "<=", ">="}), None)
        if split is None or any(word in {"&&", "||", "?:"} for word in self.words(tokens)):
            return None
        parts = tuple(self.split(tokens, split))
        if len(parts) != 2:
            return None
        return next((read for part in parts if part and (read := self.access(part, env))), None)

    def branch(self, statement, env, flow, stack, depth):
        guard = self.site(statement.expression)
        if read := self.guard_read(statement.expression, env):
            flow = replace(flow, reads=(*flow.reads, read))
        continuing = []
        for suite, holds in ((statement.body, True), (statement.alternative, False)):
            condition = f"{self.code(guard)} is {str(holds).lower()}"
            selected = replace(flow, conditions=(*flow.conditions, condition), evidence=(*flow.evidence, guard))
            self.walk(suite, dict(env), selected, stack, depth + 1)
            if not suite or suite[-1].kind not in {"return", "throw", "continue", "break"}:
                continuing.append((suite, selected))
        if not continuing:
            return None
        if len(continuing) == 1 and not continuing[0][0]:
            return continuing[0][1]
        return replace(flow, reads=(), writes=())

    def walk(self, suite, env, flow, stack, depth):
        if depth > MAX_DEPTH:
            return
        for index, statement in enumerate(suite):
            self.steps -= 1
            if self.steps <= 0 or statement.kind in {"return", "throw", "break", "continue", "unsupported"}:
                return
            tokens, words = statement.expression, self.words(statement.expression)
            if statement.kind == "for" and len(words) > 2 and self.name(words[0]) and words[1] == "in":
                self.foreach_loop(words[0], self.site(tokens), statement.body, env, flow, stack, depth)
                continue
            if statement.kind == "try":
                hazard = self.cleanup_writes((statement.cleanup, *(branch for _, _, branch in statement.handlers)), env)
                self.walk(statement.body, dict(env), replace(flow, invalidated=flow.invalidated | hazard), stack, depth + 1)
                flow = replace(flow, reads=(), writes=(), invalidated=flow.invalidated | hazard)
                continue
            if statement.kind == "if":
                flow = self.branch(statement, env, flow, stack, depth)
                if flow is None:
                    return
                continue
            if statement.kind != "expression":
                return
            self.callbacks(statement, env, flow, stack, depth)
            assignment = tuple(self.split(tokens, "="))
            assigned = len(assignment) == 2 and all(assignment)
            evaluated = assignment[1] if assigned else tokens
            self.helper(evaluated, env, flow, stack, depth)
            if external := self.external(evaluated, env):
                self.outcome(external, flow, self.later(suite, index))
            if assigned:
                lhs, rhs = assignment
                labels = self.words(lhs)
                if labels[0] in {"val", "var"} and len(labels) > 1 and self.name(labels[1]):
                    env[labels[1]] = self.value(rhs, env) if labels[0] == "val" else None
                    if read := self.access(rhs, env):
                        flow = replace(flow, reads=(*flow.reads, read))
                elif write := self.access(lhs, env, write=True):
                    flow = self.with_write(flow, replace(write, site=statement.site))
                elif len(labels) == 1:
                    env[labels[0]] = None
            elif write := self.access(tokens, env, write=True):
                flow = self.with_write(flow, write)
            elif mutation := self.receiver_call(tokens, env, {"clear", "remove", "replace", "replaceAll", "putAll", "compute"}):
                field = mutation[0]
                if field.storage == "MAP":
                    flow = replace(flow, writes=tuple(row for row in flow.writes if row.receiver != field.name),
                                   invalidated=flow.invalidated | {field.name})
