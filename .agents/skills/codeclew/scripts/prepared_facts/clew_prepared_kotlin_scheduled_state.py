"""Follow keyed heap state through retained scheduler and message callbacks."""

from dataclasses import dataclass, field, replace
import re


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
class MapAlias:
    members: frozenset[str]


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


def project(unit, root, syntax, protocols, hard_guards, bind, native_calls, entry_kind, *, selected_call=None):
    """Recognize ordered source effects without assuming scheduler or transaction activation."""
    owner = unit.enclosing_owner(root)
    if owner is None or root.expression_body:
        return []
    projector = ScheduledState(unit, syntax, protocols, hard_guards, bind, owner, native_calls, entry_kind, selected_call)
    try:
        body = unit.suite(root.body + 1, root.end)
        inputs = {param.name: Key(param.site, (), (param.site,)) if projector.listener else None for param in root.params}
        projector.walk(body, inputs, Flow(), (root,), 0)
    except syntax["Unsupported"]:
        return []
    return list(projector.rows.values())


class ScheduledState:
    def __init__(self, unit, syntax, protocols, hard_guards, bind, owner, native_calls, entry_kind, selected_call):
        self.unit, self.bind, self.native_calls = unit, bind, native_calls
        self.owner, self.selected_call = owner, selected_call
        self.listener = entry_kind == "KAFKA_LISTENER"
        self.functions, self.fields = unit.members(owner)
        self.maps = frozenset(field.name for field in self.fields.values() if field.storage == "MAP")
        self.name, self.literal, self.site, self.words = (
            syntax[key] for key in ("name", "literal", "token_site", "strings"))
        self.invoke, self.named, self.split = (syntax[key] for key in ("invocation", "named_argument", "groups"))
        self.callback = syntax["callback_body"]
        self.make_call = protocols["jvm_target"]
        self.foreach = protocols["FOREACH"]
        self.transactions, self.futures = protocols["TRANSACTIONS"], protocols["FUTURES"]
        self.remote = syntax["REMOTE"]
        self.unsupported = syntax["Unsupported"]
        self.hard_names, self.hard_pattern = hard_guards["EXCEPTIONS"], hard_guards["STDLIB"]
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
        authority = "BOUNDED_KOTLIN_LISTENER_STATE" if self.listener else "BOUNDED_KOTLIN_SCHEDULED_STATE"
        row = {"kind": kind, "text": text, "authority": authority,
               "sources": list(dict.fromkeys(refs))}
        if conditions:
            row["condition"] = conditions
        if uncertainty:
            row["uncertainty"] = uncertainty
        self.rows.setdefault((kind, text, conditions, tuple(refs)), row)

    def value(self, tokens, env):
        words = self.words(tokens)
        origin = env.get(words[0]) if words else None
        if not isinstance(origin, Key):
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

    def map_targets(self, labels, env):
        if len(labels) == 1 and isinstance(alias := env.get(labels[0]), MapAlias):
            return alias.members
        field = self.receiver(labels, env)
        return frozenset((field.name,)) if field and field.name in self.maps else frozenset()

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
        premise = ("the listener's loop supplies " if self.listener else "a scheduled iteration supplies ") + '`' + name + '`'
        self.walk(suite, child, replace(flow, evidence=(*flow.evidence, header),
                                        conditions=(*flow.conditions, premise)), stack, depth + 1)

    def callbacks(self, tokens, env, flow, stack, depth):
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
        handoff = self.site(tokens)
        callback = self.callback(self.unit, self.unit.site(opening, last + 1), handoff, None)
        if not callback:
            return
        suite = self.unit.suite(callback.start, callback.end)
        if direct[-1] == "forEach" and len(callback.params) == 1 and any(
                self.foreach.match(target) for target, _ in self.native(handoff)):
            self.foreach_loop(callback.params[0][0], handoff, suite, env, flow, stack, depth)
            return
        if direct[-1] in {"execute", "executeWithoutResult"}:
            field = self.receiver(direct[:-2], env)
            if field and field.stable and field.type in self.transactions:
                self.walk(suite, {**env, **dict.fromkeys(name for name, _ in callback.params)},
                          replace(flow, transaction=True, evidence=(*flow.evidence, field.site, handoff),
                                  conditions=(*flow.conditions, "the retained transaction template invokes its callback")),
                          stack, depth + 1)
        if direct[-1] == "also" and len(callback.params) == 1:
            selected = self.also_premise(handoff, env)
            if selected:
                origin = self.value(prefix[:-2], env)
                local = {**env, callback.params[0][0]: origin}
                self.walk(suite, local, replace(flow, evidence=(*flow.evidence, handoff),
                                               conditions=(*flow.conditions, selected)), stack, depth + 1)

    def helper(self, tokens, env, flow, stack, depth):
        if not (call := self.call(tokens)):
            return
        labels, args, site, end = call
        if end != len(tokens):
            return
        selected = self.unit.same_owner_callable(self.owner, labels, args, site, env)
        if selected is None:
            return
        helper, arguments = selected
        if helper in stack:
            return
        declaration = self.unit.function_site(helper)
        if self.selected_call:
            owner_site = self.unit.site(self.owner.start, self.owner.body + 1)
            self.selected_call(self.unit.function_site(stack[-1]), site, declaration, owner_site, flow)
        if helper.expression_body:
            return
        values, parameter_sites = {}, []
        for param, value in zip(helper.params, arguments):
            values[param.name] = self.value(value, env)
            if values[param.name] is not None:
                parameter_sites.append((param, self.site(value), values[param.name].origins))
        child = replace(flow, evidence=(*flow.evidence, site, self.unit.function_site(helper)),
                        conditions=(*flow.conditions, "the call selects the retained same-owner implementation"))
        for param, actual, origins in parameter_sites:
            self.add("LISTENER_ARGUMENT" if self.listener else "SCHEDULED_ARGUMENT",
                     f"The source handoff passes {self.code(actual)} to the {param.name} parameter of {helper.name}.",
                     child, actual, param.site, *origins,
                     uncertainty="The local call must be reached and the argument must evaluate normally.")
        suite = self.unit.suite(helper.body + 1, helper.end)
        self.walk(suite, values, replace(child, reads=(), writes=()), (*stack, helper), depth + 1)

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
                retry = "A later message-listener invocation" if self.listener else "A later scheduled iteration"
                self.add("SAME_KEY_HEAP_RETRY",
                         f"{retry} can read the value written to {self.code(write.site)} "
                         f"even if {self.code(call)} failed after the write." + rollback,
                         flow, *proof, read.site, *read.key.origins,
                         uncertainty="Requires the same receiver, equal derived keys, the earlier write to have completed, "
                                     "and no intervening write, removal, clearing or replacement. "
                                     "Accessor return values can vary across evaluations.")

    def invalidate(self, flow, receivers):
        if not receivers:
            return flow
        return replace(flow, reads=tuple(row for row in flow.reads if row.receiver not in receivers),
                       writes=tuple(row for row in flow.writes if row.receiver not in receivers),
                       invalidated=flow.invalidated | receivers)

    def bind_local(self, env, labels, rhs, *, site=None):
        name = labels[1]
        if targets := self.map_targets(self.words(rhs), env):
            env[name] = MapAlias(targets)
        else:
            value = self.value(rhs, env) if labels[0] == "val" else None
            env[name] = replace(value, origins=(*value.origins, site)) if value and site else value

    def map_effects(self, tokens, env):
        """Invalidate a map when it is mutated, escapes as a value, or reaches an opaque owner call."""
        if not tokens:
            return frozenset()
        words = self.words(tokens)
        first = self.unit.token_indices[tokens[0].start]
        names = set()
        for index, word in enumerate(words):
            previous = words[index - 1:index] if index else ()
            following = words[index + 1:index + 3]
            explicit = index >= 2 and words[index - 2:index] == ("this", ".")
            if word == "this" and following[:1] != (".",):
                names.update(self.maps)
            if previous in {(".",), ("?.",)} and not explicit:
                continue
            labels = ("this", ".", word) if explicit else (word,)
            if fields := self.map_targets(labels, env):
                if following[:1] == ("[",):
                    close = self.unit.pairs.get(first + index + 1, first + len(tokens)) - first
                    read = (words[close + 1:close + 2] not in {("=",), ("+=",), ("-=",), ("*=",), ("/=",),
                                                               ("%=",), ("++",), ("--",)} and
                            previous not in {("++",), ("--",)})
                else:
                    read = following in {(".", "get"), (".", "containsKey"), (".", "isEmpty"), (".", "size")}
                if not read:
                    names.update(fields)
            if following[:1] == ("(",) and word in self.functions and (explicit or word not in env):
                names.update(self.maps)
        return frozenset(names)

    def cleanup_writes(self, suites, env, rebindings=None):
        names = set()
        for suite in suites:
            local = dict(env)
            for statement in suite:
                tokens = statement.expression
                assignment = self.assignment(tokens)
                evaluated = assignment[1] if assignment else tokens
                if assignment:
                    lhs = self.words(assignment[0])
                    if lhs[:1] in {("val",), ("var",)} and len(lhs) > 1 and self.name(lhs[1]):
                        self.bind_local(local, lhs, evaluated)
                        if self.map_targets(self.words(evaluated), local):
                            continue
                    elif len(lhs) == 1 and lhs[0] in local:
                        targets = self.map_targets(self.words(evaluated), local)
                        if targets:
                            if rebindings is not None and lhs[0] in env:
                                rebindings[lhs[0]] = rebindings.get(lhs[0], frozenset()) | targets
                            local[lhs[0]] = self.merge_binding(local[lhs[0]], MapAlias(targets))
                            continue
                    else:
                        names.update(self.write_targets(assignment[0], local))
                        names.update(self.map_targets(lhs, local))
                names.update(self.map_effects(evaluated, local))
                names.update(self.cleanup_writes((statement.body, statement.alternative, statement.cleanup,
                                                  *(branch for _, _, branch in statement.handlers)), local, rebindings))
        return names

    def write_targets(self, tokens, env):
        words = self.words(tokens)
        return self.map_targets(words[:words.index("[")], env) if "[" in words else frozenset()

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

    def assignment(self, tokens):
        if not tokens:
            return None
        parts = tuple(self.split(tokens, "="))
        return parts if len(parts) == 2 and all(parts) else None

    def rhs_control(self, assignment):
        if assignment is None or assignment[1][0].text not in {"if", "try"}:
            return None
        tokens = assignment[1]
        first = self.unit.token_indices[tokens[0].start]
        end = first + len(tokens)
        try:
            node, after = self.unit.statement(first, end, 0)
        except self.unsupported:
            return None
        return node if after == end and node.kind in {"if", "try"} else None

    def local_also(self):
        for index, word in enumerate(self.unit.words[:-1]):
            if word in {"val", "var"} and self.unit.words[index + 1] == "also":
                return True
            if word == "fun":
                end = self.unit.until(index + 1, len(self.unit.tokens), {"(", "=", "{", "}"})
                if end > index + 1 and self.unit.words[end - 1] == "also":
                    return True
        return False

    def also_premise(self, site, env):
        targets = [target for target, _ in self.native(site)
                   if (api := self.make_call(target)) and api[1] == "also"]
        standard = r"(?:callable:kotlin/(?:StandardKt(?:__StandardKt)?[.])?also#jvm:|method:class:kotlin[.]StandardKt(?:__StandardKt)?#also\()"
        if targets:
            if any(not re.match(standard, target) for target in targets):
                return None
            return "Kotlin's also invokes its retained lambda"
        if ("also" in env or self.unit.stars or self.unit.imports.get("also", {"kotlin.also"}) != {"kotlin.also"}
                or self.local_also()):
            return None
        return "the suffix selects Kotlin's also and invokes its retained lambda"

    def decoding_result(self, tokens, env):
        """Tie a result identity to the reached initializer, with input provenance kept separate."""
        if not (call := self.call(tokens)):
            return None
        _, arguments, decoder, end = call
        inputs = [origin for argument in arguments if (origin := self.value(self.named(argument)[1], env))]
        if not inputs:
            return None
        tail = self.words(tokens[end:])
        evidence, premise = [decoder], f"{self.code(decoder)} completes normally"
        if tail:
            if tail[:3] != (".", "also", "{") or tail[-1] != "}":
                return None
            callback = self.unit.token_indices[tokens[end + 2].start]
            if self.unit.pairs.get(callback) != self.unit.token_ends[tokens[-1].end]:
                return None
            whole = self.site(tokens)
            if not (selected := self.also_premise(whole, env)):
                return None
            evidence.append(whole)
            premise += " and " + selected
            premise += " and that scope call and its invoked callback complete normally"
        return Key(decoder, (), tuple(dict.fromkeys((*evidence, *(site for origin in inputs for site in origin.origins))))), premise

    def listener_initializer(self, expression, local, env):
        if not self.listener or local.cleanup or not local.body or not local.handlers or any(
                not branch or not self.hard_stop(branch[-1], env) for _, _, branch in local.handlers):
            return None
        last = local.body[-1]
        if last.kind != "expression" or not (selected := self.decoding_result(last.expression, env)):
            return None
        origin, premise = selected
        return replace(origin, origins=(*origin.origins, expression.site)), premise

    def hard_atom(self, tokens, env):
        if not (call := self.call(tokens)):
            return None
        labels, args, site, end = call
        member = labels[-1]
        if member not in self.hard_names or (len(labels) > 1 and labels != ("kotlin", ".", member)):
            return None
        if end != len(tokens):
            first = self.unit.token_indices[tokens[0].start]
            if self.unit.words[first + end] != "{" or self.unit.pairs.get(first + end) != first + len(tokens) - 1:
                return None
            site = self.site(tokens)
        if len(args) not in ({1} if member == "error" else {1, 2}):
            return None
        native = [target for target, _ in self.native(site) if (api := self.make_call(target)) and api[1] == member]
        if native:
            if any(not (match := self.hard_pattern.match(target)) or match[1] != member for target in native):
                return None
        elif len(labels) == 1:
            if (member in env or member in self.fields or member in self.functions or self.unit.stars or
                    self.unit.imports.get(member, {f"kotlin.{member}"}) != {f"kotlin.{member}"} or
                    any(self.unit.words[index + 1] == member and word in {"fun", "val", "var", "class", "object"}
                        for index, word in enumerate(self.unit.words[:-1]))):
                return None
        elif "kotlin" in env or self.unit.imports.get("kotlin"):
            return None
        return member, self.site(args[0]), site

    def hard_stop(self, statement, env):
        if statement.kind in {"return", "throw", "continue", "break"}:
            return statement.site
        if statement.kind == "expression" and (hard := self.hard_atom(statement.expression, env)) and hard[0] == "error":
            return hard[2]
        return None

    def expression_guard(self, tokens, env, flow):
        paths = tuple(self.split(tokens, "?:"))
        candidate = paths[-1] if len(paths) <= 2 and all(paths) else ()
        hard = self.hard_atom(candidate, env)
        if not hard:
            return flow
        kind, condition, anchor = hard
        if len(paths) == 2 and kind == "error":
            condition = self.site(paths[0])
            premise = f"{self.code(condition)} yields a non-null value"
        elif len(paths) == 1 and kind in {"require", "check"}:
            premise = f"{self.code(condition)} passes the Kotlin {kind} guard"
        else:
            return None if kind == "error" and len(paths) == 1 else flow
        return replace(flow, conditions=(*flow.conditions, premise), evidence=(*flow.evidence, condition, anchor))

    def following_sites(self, suite, env, depth=0):
        if depth >= 3:
            return ()
        sites = []
        for item in suite[:6]:
            assignment = self.assignment(item.expression) if item.kind == "expression" else None
            inner = self.rhs_control(assignment) or item
            if inner.kind in {"if", "try"}:
                sites.extend(self.following_sites(inner.body, env, depth + 1))
                sites.extend(self.following_sites(inner.alternative, env, depth + 1))
            elif item.kind == "expression":
                evaluated = assignment[1] if assignment else item.expression
                if external := self.external(evaluated, env):
                    sites.append(external[0])
                elif assignment and self.access(assignment[0], env, write=True):
                    sites.append(item.site)
            if len(sites) >= 3 or self.hard_stop(item, env):
                break
        return tuple(sites[:3])

    def scoped(self, suite, env, flow, stack, depth):
        local, shadowed = dict(env), {}
        result = self.walk(suite, local, flow, stack, depth, shadowed=shadowed)
        local.update(shadowed)
        return result, local

    @staticmethod
    def merge_binding(left, right):
        if left == right and (not isinstance(left, Key) or left.origins == right.origins):
            return left
        aliases = frozenset(member for value in (left, right) if isinstance(value, MapAlias) for member in value.members)
        return MapAlias(aliases) if aliases else None

    def branch(self, statement, env, flow, stack, depth, following):
        guard = self.site(statement.expression)
        flow = self.invalidate(flow, self.map_effects(statement.expression, env))
        read = self.guard_read(statement.expression, env)
        if read:
            flow = replace(flow, reads=(*flow.reads, read))
        continuing = []
        for suite, holds in ((statement.body, True), (statement.alternative, False)):
            condition = f"{self.code(guard)} is {str(holds).lower()}"
            selected = replace(flow, conditions=(*flow.conditions, condition), evidence=(*flow.evidence, guard))
            result, local = self.scoped(suite, env, selected, stack, depth + 1)
            if result is not None:
                continuing.append((result, local))
                continue
            exit_site = self.hard_stop(suite[-1], env) if suite else None
            if exit_site and following and all(item.kind == "expression" for item in suite[:-1]):
                continuation = ", ".join(self.code(site) for site in following)
                kind = "HEAP_GUARD_EXIT" if read else "LISTENER_GUARD_EXIT" if self.listener else "SCHEDULED_GUARD_EXIT"
                subject = "The in-memory guard" if read else "The guard"
                proof = (self.fields[read.receiver].site, read.site, *read.key.origins) if read else ()
                self.add(kind, f"{subject} can reach {self.code(exit_site)} before the normal continuation to {continuation}.",
                         selected, *proof, exit_site, *following,
                         uncertainty="The selected arm and its preceding evaluations must reach the cited exit.")
        if not continuing:
            return None
        if len(continuing) == 1:
            result, local = continuing[0]
            env.update({name: local[name] for name in env})
            return result
        (left, left_env), (right, right_env) = continuing
        env.update({name: self.merge_binding(left_env[name], right_env[name]) for name in env})
        # Only the same reached source events support facts on both normal arms.
        intersect = {field: tuple(item for item in getattr(left, field) if item in getattr(right, field))
                     for field in ("reads", "writes", "conditions", "evidence")}
        return replace(flow, **intersect, invalidated=left.invalidated | right.invalidated,
                       last_external=left.last_external if left.last_external == right.last_external else None)

    def merge_possible_aliases(self, suites, env):
        rebindings = {}
        hazards = self.cleanup_writes(suites, env, rebindings)
        for name, members in rebindings.items():
            if name in env:
                env[name] = self.merge_binding(env[name], MapAlias(members))
        return hazards

    def walk(self, suite, env, flow, stack, depth, *, shadowed=None):
        if depth > MAX_DEPTH:
            return self.invalidate(flow, self.maps)
        for index, statement in enumerate(suite):
            self.steps -= 1
            if statement.kind in {"return", "throw", "break", "continue"}:
                if statement.kind == "return" and statement.expression:
                    self.helper(statement.expression, env, flow, stack, depth)
                return None
            if self.steps <= 0 or statement.kind == "unsupported":
                return self.invalidate(flow, self.maps)
            tokens, words = statement.expression, self.words(statement.expression)
            assignment = self.assignment(tokens) if statement.kind == "expression" else None
            local = self.rhs_control(assignment) or statement
            initialized = None
            if statement.kind == "for" and len(words) > 2 and self.name(words[0]) and words[1] == "in":
                self.foreach_loop(words[0], self.site(tokens), statement.body, env, flow, stack, depth)
                flow = self.invalidate(flow, self.merge_possible_aliases((statement.body,), env))
                continue
            if local.kind == "try":
                hazard = self.cleanup_writes((local.cleanup, *(branch for _, _, branch in local.handlers)), env)
                success, inside = self.scoped(local.body, env, replace(flow, invalidated=flow.invalidated | hazard),
                                              stack, depth + 1)
                if success is not None and assignment and (decoded := self.listener_initializer(statement, local, inside)):
                    initialized, premise = decoded
                    flow = replace(flow, conditions=(*flow.conditions, premise),
                                   evidence=(*flow.evidence, statement.site))
                self.merge_possible_aliases((local.body, local.cleanup, *(branch for _, _, branch in local.handlers)), env)
                flow = replace(flow, reads=(), writes=(), invalidated=flow.invalidated | hazard, last_external=None)
            if local.kind == "if":
                following = self.following_sites(suite[index + 1:], env) or tuple(
                    item.site for item in suite[index + 1:index + 3] if item.kind == "expression")
                flow = self.branch(local, env, flow, stack, depth, following)
                if flow is None:
                    return None
            if statement.kind in {"try", "if"}:
                continue
            if statement.kind != "expression":
                return self.invalidate(flow, self.maps)
            evaluated = assignment[1] if assignment else tokens
            self.callbacks(evaluated, env, flow, stack, depth)
            flow = self.expression_guard(evaluated, env, flow)
            if flow is None:
                return None
            self.helper(evaluated, env, flow, stack, depth)
            write = self.access(assignment[0] if assignment else tokens, env, write=True)
            labels = self.words(assignment[0]) if assignment else ()
            local_declaration = len(labels) > 1 and labels[0] in {"val", "var"} and self.name(labels[1])
            local_alias = local_declaration or len(labels) == 1 and labels[0] in env
            if local is statement and not (local_alias and self.map_targets(self.words(evaluated), env)):
                if write and not assignment:
                    direct = self.receiver_call(evaluated, env, {"put", "set"})
                    effects = set().union(*(self.map_effects(arg, env) for arg in direct[2])) if direct else set()
                else:
                    effects = self.map_effects(evaluated, env)
                flow = self.invalidate(flow, effects)
            if external := self.external(evaluated, env):
                self.outcome(external, flow, self.later(suite, index))
                flow = replace(flow, last_external=external)
            if write:
                flow = self.with_write(flow, replace(write, site=statement.site) if assignment else write)
            elif assignment:
                if local_declaration:
                    if shadowed is not None and labels[1] in env:
                        shadowed.setdefault(labels[1], env[labels[1]])
                    self.bind_local(env, labels, evaluated, site=statement.site)
                    if initialized:
                        env[labels[1]] = initialized
                    if read := self.access(evaluated, env):
                        flow = replace(flow, reads=(*flow.reads, read))
                elif len(labels) == 1 and labels[0] in env:
                    self.bind_local(env, ("var", labels[0]), evaluated)
                else:
                    flow = self.invalidate(flow, self.write_targets(assignment[0], env) | self.map_targets(labels, env))
        return flow
