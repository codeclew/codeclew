"""Connect a scheduled iteration to a narrowly bound heap-state continuation."""

from dataclasses import dataclass, field, replace


MAX_DEPTH = 8
MAX_STATEMENTS = 160
MAX_FACTS = 24
MAX_KEY_PARTS = 6
KAFKA = "org.springframework.kafka.core.KafkaTemplate"


@dataclass(frozen=True)
class Key:
    loop: object
    projections: tuple
    origins: tuple = field(default=(), compare=False)


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
    last_external: tuple | None = None


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
        self.name, self.literal, self.site, self.words = (
            syntax[key] for key in ("name", "literal", "token_site", "strings"))
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
        parts = list(origin.projections)
        index = 1
        while index < len(words) and len(parts) < MAX_KEY_PARTS:
            word = words[index]
            if word == "[" and words[index + 2:index + 3] == ("]",) and self.literal(words[index + 1]):
                parts.append(("index", words[index + 1]))
                index += 3
            elif word in {".", "?."} and index + 1 < len(words) and self.name(words[index + 1]):
                member = words[index + 1]
                index += 2
                literals = None
                if words[index:index + 1] == ("(",):
                    first = self.unit.token_indices[tokens[0].start]
                    closing = self.unit.pairs.get(first + index, -1) - first
                    if not index < closing < len(tokens):
                        return None
                    operands = tuple(self.split(tokens[index + 1:closing]))
                    if len(operands) > 2 or any(len(item) != 1 or not self.literal(item[0].text) for item in operands):
                        return None
                    literals = tuple(item[0].text for item in operands)
                    index = closing + 1
                parts.append((word, member, literals))
            else:
                return None
        if index == len(words):
            sites = (*origin.origins, self.site(tokens)) if len(words) > 1 else origin.origins
            return Key(origin.loop, tuple(parts), sites)
        return None

    def receiver(self, labels, env):
        if len(labels) == 1 and labels[0] not in env:
            return self.fields.get(labels[0])
        if len(labels) == 3 and labels[:2] == ("this", "."):
            return self.fields.get(labels[2])
        return None

    def map_part(self, tokens, env, allow_suffix):
        words = self.words(tokens)
        if "[" not in words:
            return None
        opening = words.index("[")
        first = self.unit.token_indices[tokens[0].start]
        closing = self.unit.pairs.get(first + opening, -1) - first
        if not opening < closing < len(words) or (
                closing + 1 < len(words) and (not allow_suffix or words[closing + 1] not in {".", "?."})):
            return None
        receiver = self.receiver(words[:opening], env)
        if receiver and receiver.storage == "MAP" and (key := self.value(tokens[opening + 1:closing], env)):
            return Access(receiver.name, key, self.site(tokens[:closing + 1]))
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

    def access(self, tokens, env, *, write=False, allow_suffix=False):
        if indexed := self.map_part(tokens, env, allow_suffix):
            return indexed
        names = {"put", "set"} if write else {"get"}
        if call := self.receiver_call(tokens, env, names):
            field, _, args, site, end = call
            if (field.storage == "MAP" and len(args) == (2 if write else 1)
                    and (end == len(tokens) or allow_suffix and tokens[end].text in {".", "?."})
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
        if index + 1 < len(suite) and suite[index + 1].kind != "unsupported":
            return suite[index + 1].site
        return None

    def outcome(self, external, flow, following):
        call, evidence = external
        for write in flow.writes:
            field = self.fields[write.receiver]
            proof = (field.site, write.site, *write.key.origins, call, *evidence)
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
                rollback = " This heap mutation has its own lifetime through database rollback." if flow.transaction else ""
                self.add("SAME_KEY_HEAP_RETRY",
                         f"A later scheduled iteration can read the value written to {self.code(write.site)} "
                         f"even if {self.code(call)} failed after the write." + rollback,
                         flow, *proof, read.site, *read.key.origins,
                         uncertainty="Requires the same receiver, equal derived keys, the earlier write to have completed, "
                                     "and no intervening write, removal, clearing or replacement. "
                                     "Accessor return values can vary across evaluations.")

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
        if flow.last_external:
            call, evidence = flow.last_external
            self.add("HEAP_WRITE_AFTER_CALL",
                     f"After {self.code(call)} returns normally, the source continuation reaches "
                     f"the in-memory write {self.code(write.site)}.",
                     flow, call, *evidence, self.fields[write.receiver].site, write.site, *write.key.origins)
        # A later direct write to this receiver can replace the tracked value.
        return replace(flow, writes=(*(item for item in flow.writes if item.receiver != write.receiver), write))

    def guard_read(self, tokens, env):
        if not tokens:
            return None
        first = self.unit.token_indices[tokens[0].start]
        container = self.unit.container[first]
        operators = [(index, token.text) for index, token in enumerate(tokens)
                     if self.unit.container[first + index] == container
                     and token.text in {"==", "!=", "<", ">", "<=", ">=", "&&", "||", "?:"}]
        if len(operators) != 1 or operators[0][1] in {"&&", "||", "?:"}:
            return None
        index, _ = operators[0]
        parts = (tokens[:index], tokens[index + 1:])
        return next((read for part in parts if part and (read := self.access(part, env, allow_suffix=True))), None)

    def branch(self, statement, env, flow, stack, depth, following):
        guard = self.site(statement.expression)
        read = self.guard_read(statement.expression, env)
        if read:
            flow = replace(flow, reads=(*flow.reads, read))
        continuing = []
        for suite, holds in ((statement.body, True), (statement.alternative, False)):
            condition = f"{self.code(guard)} is {str(holds).lower()}"
            selected = replace(flow, conditions=(*flow.conditions, condition), evidence=(*flow.evidence, guard))
            self.walk(suite, dict(env), selected, stack, depth + 1)
            if not suite or suite[-1].kind not in {"return", "throw", "continue", "break"}:
                continuing.append((suite, selected))
            elif read and following and suite[-1].kind in {"return", "throw"} and all(
                    item.kind == "expression" for item in suite[:-1]):
                exit_site = suite[-1].site
                continuation = ", ".join(self.code(site) for site in following)
                self.add("HEAP_GUARD_EXIT",
                         f"The in-memory guard can reach {self.code(exit_site)} before "
                         f"the normal continuation to {continuation}.",
                         selected, self.fields[read.receiver].site, read.site, *read.key.origins, exit_site, *following,
                         uncertainty="The selected arm and its preceding evaluations must reach the cited exit.")
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
                following = tuple(item.site for item in suite[index + 1:index + 4] if item.kind == "expression")
                flow = self.branch(statement, env, flow, stack, depth, following)
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
                flow = replace(flow, last_external=external)
            if assigned:
                lhs, rhs = assignment
                labels = self.words(lhs)
                if labels[0] in {"val", "var"} and len(labels) > 1 and self.name(labels[1]):
                    value = self.value(rhs, env) if labels[0] == "val" else None
                    env[labels[1]] = replace(value, origins=(*value.origins, statement.site)) if value else None
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
