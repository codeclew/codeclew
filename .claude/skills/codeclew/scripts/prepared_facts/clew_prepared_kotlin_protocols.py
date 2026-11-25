"""Join Kotlin waits and transaction callbacks to local failure continuations."""

from dataclasses import dataclass, replace
from pathlib import Path, PurePosixPath
import re
import runpy


SQL_TERMS = runpy.run_path(str(Path(__file__).with_name("clew_prepared_kotlin_operands.py")))["sql_terms"]
TRANSACTIONS = {"org.springframework.transaction.support.TransactionTemplate",
                "org.springframework.transaction.support.TransactionOperations"}
FUTURES = {"java.util.concurrent.Future", "java.util.concurrent.CompletableFuture"}
ACK = "org.springframework.kafka.support.Acknowledgment"
KAFKA = {"org.springframework.kafka.core.KafkaTemplate", "org.springframework.kafka.core.KafkaOperations"}
TYPES = TRANSACTIONS | FUTURES | KAFKA | {ACK}
TOPIC_OPERAND = re.compile(r'"([A-Za-z0-9._-]{1,249})"')
SHORT_TYPES = {type_.rsplit(".", 1)[-1]: type_ for type_ in TYPES}
EXTRACT_ITEM = r"(?:\.\s*first\s*\(\s*\)|\[\s*[0-9]+\s*\])"
TIME_UNIT = re.compile(r"(?:java\.util\.concurrent\.)?TimeUnit\.(?:NANOSECONDS|MICROSECONDS|MILLISECONDS|SECONDS|MINUTES|HOURS|DAYS)\Z")
TIMED = "(JLjava/util/concurrent/TimeUnit;)"
TYPE_NAME = re.compile(r"(?:this\.)?([A-Za-z_]\w*)\Z")
FOREACH = re.compile(r"(?:callable:kotlin/collections/(?:CollectionsKt(?:__\w+Kt)?\.)?forEach#jvm:|"
                     r"method:class:java\.lang\.Iterable#forEach\()")
SQL_MARKER = r"(?:NULL|TRUE|FALSE|-?\d+|'(?:''|[^'])*')"
SQL_MARK_TIMESTAMP = re.compile(r"(?:CURRENT_TIMESTAMP(?:\(\d*\))?|NOW\(\))\Z", re.I)
TRANSFERS = {"return", "throw", "break", "continue"}


def inside(inner, outer):
    return (inner.file == outer.file and outer.start <= inner.start < inner.end <= outer.end)


def jvm_target(target):
    java = re.match(r"method:class:([^#]+)#([^(]+)(\(.*)", target)
    kotlin = re.match(r"callable:(.+)/([^./]+)\.([^#.]+)#jvm:(\(.*)", target)
    if java:
        return java.groups()
    if kotlin:
        package, owner, member, descriptor = kotlin.groups()
        return package.replace("/", ".") + "." + owner, member, descriptor
    return None


def contract(owner, member, descriptor, arity):
    if owner in TRANSACTIONS and member in {"execute", "executeWithoutResult"}:
        return "transaction"
    if owner in FUTURES and member == "get" and (descriptor.startswith(TIMED) or not descriptor and arity == 2):
        return "timed_wait"
    if owner in FUTURES and member == "get" and descriptor.startswith("()"):
        return "wait"
    if owner == "java.util.concurrent.CompletableFuture" and member == "join" and descriptor.startswith("()"):
        return "wait"
    if owner == ACK and member == "acknowledge":
        return "acknowledgment"
    if owner in KAFKA and member == "send" and (descriptor.startswith("(Ljava/lang/String;") or not descriptor and arity >= 2):
        return "publication"
    return None


@dataclass(frozen=True)
class Binding:
    invocation: object
    kind: str
    evidence: object
    authority: str
    condition: str = ""


@dataclass(frozen=True)
class DeclaredReceiver:
    type: str
    citations: tuple
    authority: str
    condition: str


@dataclass(frozen=True)
class Frame:
    statement: object
    suite: tuple
    index: int
    loops: tuple
    protected: tuple
    callback: object
    guards: tuple


@dataclass(frozen=True)
class Query:
    part: object
    frame: Frame
    note: object
    verb: str
    table: str
    details: tuple
    occurrence: str


class Projection:
    def __init__(self, body, root, scope, declaration):
        self.body = body
        self.root = root
        self.scope = scope
        self.declaration = declaration
        self.frames = []
        self.native = []
        self.callback_iterations = {}
        self.selection_iterations = {}
        self.catches = []
        self.bindings = []
        self.queries = []

    def note(self, part):
        return self.body.text_note(part.selector, self.body.describe_call(part))

    def selected(self):
        if not self.declaration:
            return
        for event in self.body.projector.process.get("facts", ()):
            if (event.get("kind") == "EVENT" and event.get("event") == "CALL"
                    and event.get("declaration") == self.declaration and event.get("resolution") == "COMPILER_EXACT"
                    and (target := event.get("target"))):
                self.native.extend((site, target) for site in self.body.event_sites(event))

    @staticmethod
    def occurrence(part, site):
        # A bare selector can be sealed separately. An inner call with the same
        # name has a different start and cannot lend its type to this selector.
        return site in {part.site, part.selector} or (
            site.start == part.selector.start and inside(site, part.selector))

    @staticmethod
    def declared_spelling(text, member, field=False):
        prefix = r"(?:(?:private|protected|internal|public|override|final|open)\s+)*(?:val|var)\s+" if field else ""
        pattern = prefix + re.escape(member) + r"\s*:\s*([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)"
        match = re.match(pattern + r"(?=\s|[<?=,)]|$)", text.strip())
        return match[1] if match else None

    def candidate(self, literal, member, citations, unbound=False):
        if not literal:
            return None
        declared = self.body.unit.qualify(literal)
        if declared in TYPES and not unbound:
            return DeclaredReceiver(declared, citations, "DECLARED_RECEIVER_CANDIDATE", "")
        unit = self.body.unit
        if declared not in TYPES and (unit.imports.get(literal) or unit.aliases.get(literal)
                                      or unit.class_names.get(literal)):
            return None
        target = declared if declared in TYPES else SHORT_TYPES.get(literal)
        if not target:
            return None
        prefix = f"the retained `{member}: {literal}` is the receiver binding and " if unbound else ""
        return DeclaredReceiver(target, citations, "DECLARED_TYPE_BINDING_OPEN",
                                f"{prefix}`{literal}` denotes `{target}`")

    def mentioned_field(self, part, member):
        fields = set()
        for row, sites in self.body.rows:
            details = row.get("details", {})
            if (row["kind"] != "FIELD" or not (use := sites.get("use")) or not inside(use, part.receiver)
                    or details.get("member") not in {None, member}):
                continue
            reference = next((item["reference"] for item in row["sites"] if item["role"] == "declaration"), None)
            if not reference or (text := self.body.ranges.for_ref(reference)) is None:
                continue
            literal = self.declared_spelling(text, member, field=True)
            if not literal:
                declared = details.get("declaredType") or details.get("initializerType") or ""
                literal = declared.rstrip("?").split("<", 1)[0].strip()
            if candidate := self.candidate(literal, member, (self.body.ranges.by_ref[reference],),
                                           unbound=details.get("conditionalReceiver", False)):
                fields.add(candidate)
        return fields

    def retained_field(self, member):
        owner = self.body.unit.enclosing_owner(self.body.function)
        if owner:
            owner_name = owner.name
        else:
            symbol = (self.body.projector.process.get("entrySymbol") or "") if self.declaration else ""
            api = jvm_target(symbol)
            owner_name = api[0].rsplit(".", 1)[-1] if api else self.body.label.rpartition(".")[0].rsplit(".", 1)[-1]
        if owner_name != PurePosixPath(self.body.host["file"]).stem:
            return ()
        rows = {cite: ref for ref, cite in self.body.ranges.by_ref.items()
                if (source := self.body.ranges.native[ref])["file"] == self.body.host["file"]
                and source["revision"] == self.body.host["revision"]}
        matches = set()
        for cite, ref in rows.items():
            if ref == self.body.ref or (text := self.body.ranges.for_ref(ref)) is None or len(text) > 380:
                continue
            if literal := self.declared_spelling(text, member, field=True):
                matches.add((literal, cite))
        if len(matches) == 1:
            literal, cite = next(iter(matches))
            if candidate := self.candidate(literal, member, (cite,), unbound=True):
                return (candidate,)
        return ()

    def typed_receiver(self, part):
        if not part.receiver or not (receiver := TYPE_NAME.fullmatch(self.body.unit.excerpt(part.receiver).strip())):
            return ()
        member = receiver[1]
        qualified = receiver[0].startswith("this.")
        if not qualified:
            # Do not assign a retained declaration to a shadowing local or lambda binding.
            if any(token.text in {"val", "var"} and self.body.unit.words[index + 1] == member
                   for index, token in enumerate(self.body.unit.tokens)):
                return ()
            for site, (_, callback, _) in self.body.callback_scopes.items():
                if inside(part.selector, site) and any(name == member for name, _ in callback.params):
                    return ()
            parameters = [param for param in self.body.function.params if param.name == member]
            if parameters:
                result = []
                for param in parameters:
                    literal = self.declared_spelling(self.body.unit.excerpt(param.site), member)
                    proof = self.body.text_note(param.site, "declared parameter")
                    if candidate := self.candidate(literal, member, proof.citations):
                        result.append(candidate)
                return tuple(result)
        fields = self.mentioned_field(part, member)
        if fields:
            return tuple(fields)
        if qualified and any(inside(part.selector, site) for site in self.body.callback_scopes):
            return ()  # The lambda's this receiver cannot be inferred from the host.
        return self.retained_field(member)

    def open_chained_wait(self, part):
        if part.member != "get" or part.arity != 2 or not part.receiver:
            return None
        if not any(at.site == part.receiver for at in self.body.invocations.values()):
            return None
        if not any(TIME_UNIT.fullmatch(value.strip("`").split(" = ")[-1]) for value in part.actuals):
            return None
        if receiver := self.body.receiver_note(part.receiver):
            proof = self.body.text_note(part.selector, "timed get source site")
            evidence = replace(proof, citations=tuple(dict.fromkeys((*proof.citations, *receiver.citations))))
            return Binding(part, "timed_wait", evidence, "CHAIN_RECEIVER_TYPE_OPEN",
                           f"{receiver.text} supplies a Java Future receiver for this `get` call")
        return None

    def binding(self, part):
        selected = [(api, site) for site, target in self.native if self.occurrence(part, site)
                    and (api := jvm_target(target)) and api[1] == part.member]
        apis = {api for api, _ in selected}
        if apis:
            if len(apis) != 1 or not (kind := contract(*next(iter(apis)), part.arity)):
                return None
            site = min((site for _, site in selected), key=lambda site: (site.end - site.start, site.start))
            return Binding(part, kind, self.body.text_note(site, "compiler-selected call"), "COMPILER_SELECTED")
        typed = self.typed_receiver(part)
        if len(typed) == 1 and (kind := contract(typed[0].type, part.member, "", part.arity)):
            note = self.body.receiver_note(part.receiver)
            evidence = self.body.text_note(part.selector, "declared receiver")
            sources = tuple(dict.fromkeys((*evidence.citations, *typed[0].citations, *(note.citations if note else ()))))
            return Binding(part, kind, replace(evidence, citations=sources), typed[0].authority, typed[0].condition)
        return self.open_chained_wait(part)

    def scan(self, suite, loops=(), protected=(), callback=None, depth=0, guards=()):
        if depth > 16:
            return
        for index, statement in enumerate(suite):
            frame = Frame(statement, suite, index, loops, protected, callback, guards)
            self.frames.append(frame)
            local = self.body.expression_control(statement) or statement
            if local.kind == "try":
                self.scan(local.body, loops, (*protected, (frame, "TRY")), callback, depth + 1, guards)
                for _, handler, child in local.handlers:
                    self.catches.append((frame, local, handler, child))
                    self.scan(child, loops, (*protected, (frame, "CATCH")), callback, depth + 1, guards)
                self.scan(local.cleanup, loops, (*protected, (frame, "FINALLY")), callback, depth + 1, guards)
            else:
                inner = (*loops, local) if local.kind in {"for", "while"} else loops
                guard = None
                if local.kind == "if" and local.expression:
                    first, last = local.expression[0], local.expression[-1]
                    site = type(local.site)(first.start, last.end, local.site.file)
                    guard = self.body.note(site)
                yes = (*guards, (guard, True)) if guard else guards
                no = (*guards, (guard, False)) if guard else guards
                self.scan(local.body, inner, protected, callback, depth + 1, yes)
                self.scan(local.alternative, inner, protected, callback, depth + 1, no)

    def straight_path(self, suite):
        """Keep source text for the straight-line steps and first transfer."""
        actions = []
        for statement in suite:
            if statement.kind in {"return", "throw", "break", "continue"}:
                return actions, statement
            action = self.body.action(statement)
            if not action:
                return None
            actions.append(self.body.note(action.site) or action)
        return actions, None

    def initializer_catch(self, frame, protected, handler, handling):
        if (self.body.words(frame.statement)[:1] not in {("val",), ("var",)}
                or not handling or not (stop := handling[1]) or stop.kind != "return"):
            return
        words = self.body.words(stop)
        if frame.callback:
            _, callback, _ = self.body.callback_scopes[frame.callback]
            if words[:2] != ("@", callback.label):
                return
        elif words[:1] == ("@",):
            return
        terminal = self.body.note(stop.site)
        after = self.body.potential_actions(frame.suite[frame.index + 1:])
        direct = [note for item in protected.body if (note := self.body.action(item))][:4]
        if not terminal or not after or not direct:
            return
        scope = self.body.operand_regions.get(frame.callback, self.scope)
        text = (f"In {scope}, {handler.text} can handle a matching exception from the try initializer, "
                f"including failure while evaluating {', '.join(note.text for note in direct)}.")
        callbacks = []
        for action in direct:
            for part in self.body.call_parts.get(action.site, ()):
                for site in part.callbacks:
                    if not (entry := self.body.callback_scopes.get(site)):
                        continue
                    nested = self.straight_path(entry[2])
                    if not nested or not nested[0]:
                        continue
                    owner, operations = self.note(part), nested[0][:3]
                    callbacks.extend((owner, *operations))
                    text += (f" If {owner.text} runs the callback, a matching failure from "
                             f"{', '.join(note.text for note in operations)} can reach {handler.text} "
                             "if it escapes the callback and its invoking call.")
        actions = handling[0][:5]
        order = " → ".join(note.text for note in (*actions, terminal))
        skipped = ", ".join(note.text for note in after)
        text += (f" On the handler path, {order} can run in order if prior evaluations finish. "
                 f"If the return takes effect, it bypasses the normal continuation that can reach {skipped}.")
        self.body.projector.add("EXCEPTION_BOUNDARY", text,
                                [*direct, *callbacks, handler, *actions, terminal, *after],
                                scope="INITIALIZER_CATCH_RETURN")

    def iteration_setup(self, frame, expression):
        start = self.body.unit.token_indices.get(expression.start)
        end = self.body.unit.token_ends.get(expression.end)
        if start is None or end is None:
            return None
        tokens = self.body.unit.tokens[start:end + 1]
        if len(tokens) != 1:
            note = self.body.lexical_call(tokens)
            return (note, frame) if note else None
        name = tokens[0].text
        for previous in reversed(frame.suite[:frame.index]):
            words = self.body.words(previous)
            if words[:2] == ("val", name):
                note = self.body.action(previous)
                before = next((item for item in self.frames if item.statement == previous and item.suite == frame.suite), None)
                return (note, before) if note and before else None
            if words[:2] == ("var", name):
                return None
        return None

    def iteration_catch(self, frame, protected, handler, handling, iteration, setup, *, candidate=False):
        prefix = "If this `forEach` invokes the callback for each item, " if candidate else ""
        text = prefix + f"{handler.text} is inside one iteration of {iteration.text}."
        evidence = [handler, iteration]
        if not frame.loops and (observed := self.selection_iterations.get(frame.callback)):
            evidence.append(observed[1])
        if setup:
            source, before = setup
            text += f" A failure escaping {source.text} during selection occurs before entry to the iteration and its handler."
            evidence.append(source)
            enclosing = next((local for region, area in reversed(before.protected) if area == "TRY"
                              and (local := self.body.expression_control(region.statement) or region.statement).handlers), None)
            if enclosing:
                handlers = [note for _, at, _ in enclosing.handlers if (note := self.body.note(at))]
                if handlers:
                    text += f" At selection, a matching exception can reach {', '.join(note.text for note in handlers)}."
                    evidence.extend(handlers)
        operations = [note for item in protected.body if (note := self.body.action(item))][:3]
        if operations:
            text += (f" Within an item, a failure from {', '.join(note.text for note in operations)} "
                     f"can reach {handler.text} if it escapes to this try and the handler matches.")
            evidence.extend(operations)
        if handling:
            actions, stop = handling
            if actions:
                actions = actions[:5]
                text += " The matching handler can reach " + " → ".join(note.text for note in actions) + " in source order."
                evidence.extend(actions)
            if not stop:
                text += (" If the handler and the rest of the current iteration complete normally, "
                         "control can reach the next one when another item is available.")
            else:
                words = self.body.words(stop)
                local_continue = stop.kind == "continue" and not words and frame.loops
                local_return = (stop.kind == "return" and not frame.loops and frame.callback
                                and words[:2] == ("@", self.body.callback_scopes[frame.callback][1].label))
                if (local_continue or local_return) and (note := self.body.note(stop.site)):
                    text += f" If {note.text} takes effect, control can reach the next iteration when another item is available."
                    evidence.append(note)
        self.body.projector.add("EXCEPTION_BOUNDARY", text, evidence, scope="ITERATION_CATCH")

    def catch_region(self, frame, protected, handler, suite):
        note = self.body.note(handler)
        if not note:
            return
        handling = self.straight_path(suite)
        self.initializer_catch(frame, protected, note, handling)
        if frame.loops:
            loop = frame.loops[-1]
            iterator = next((item for item in self.frames if item.statement.site == loop.site), None)
            values = self.body.words(loop)
            setup = None
            if iterator and loop.kind == "for" and "in" in values and values[-1] != "in":
                tokens = loop.expression[values.index("in") + 1:]
                at = type(loop.site)(tokens[0].start, tokens[-1].end, loop.site.file)
                setup = self.iteration_setup(iterator, at)
            header = type(loop.site)(loop.expression[0].start, loop.expression[-1].end, loop.site.file)
            iteration = self.body.note(header) or self.body.text_note(loop.site, "the loop")
            self.iteration_catch(frame, protected, note, handling, iteration, setup)
            return
        if frame.callback:
            owner = self.body.callback_scopes[frame.callback][0]
            observed = self.selection_iterations.get(frame.callback)
            selected = self.callback_iterations.get(frame.callback)
            iterator = self.frame(owner) if owner.member == "forEach" and owner.receiver else None
            setup = self.iteration_setup(iterator, owner.receiver) if iterator else None
            if selected or observed or setup:
                iteration = self.note(owner)
                self.iteration_catch(frame, protected, note, handling, iteration, setup, candidate=not selected)
                return
        loop = next((item for item in protected.body if item.kind in {"for", "while"}), None)
        if loop is None:
            operations = [*((part, None) for part in self.callback_iterations.values()),
                          *self.selection_iterations.values()]
            observed = next(((part, selection) for part, selection in operations
                             if any(inside(part.selector, item.site) for item in protected.body)), None)
            if observed:
                part, selection = observed
                prefix = "If this `forEach` iterates the selected records, " if selection else ""
                self.body.projector.add("EXCEPTION_BOUNDARY", prefix + f"{note.text} surrounds {self.note(part).text}. "
                                        "A matching exception that propagates from an element callback through "
                                        "the iteration call can reach this handler.",
                                        [note, self.note(part), *([selection] if selection else ())],
                                        scope="OUTER_ITERATION_CATCH")
            return
        self.body.projector.add("EXCEPTION_BOUNDARY", f"{note.text} surrounds the iteration. "
                                "An exception that escapes the iteration can reach this handler and leaves that loop.",
                                [note, self.body.text_note(loop.site, "iteration")], scope="OUTER_ITERATION_CATCH")

    def frame(self, part):
        expected = [site for site in self.body.trailing_lambdas if inside(part.selector, site)]
        callback = min(expected, key=lambda site: site.end - site.start) if expected else None
        candidates = [frame for frame in self.frames if frame.callback == callback
                      and inside(part.selector, frame.statement.site)]
        return min(candidates, key=lambda item: item.statement.site.end - item.statement.site.start, default=None)

    def later(self, first, binding):
        return [entry for entry in self.bindings if entry.kind == binding
                and (frame := self.frame(entry.invocation)) and self.later_in_suite(first, frame)]

    @staticmethod
    def later_in_suite(first, last):
        return (last.suite == first.suite and last.index > first.index and first.statement.kind not in TRANSFERS
                and not any(item.kind in TRANSFERS for item in last.suite[first.index + 1:last.index]))

    def following(self, frame, limit=4):
        following = []
        for statement in frame.suite[frame.index + 1:]:
            if statement.kind in {"if", "try", "for", "while", "break", "continue", "return", "throw"}:
                break
            if effect := self.body.action(statement):
                following.append(effect)
                if len(following) == limit:
                    break
        return following

    def direct_effects(self, wait, frame):
        extra = [self.note(part) for part in self.body.invocations.values()
                 if part.receiver and inside(wait.invocation.selector, part.receiver)
                 and inside(part.selector, frame.statement.site)][:2]
        return [*extra, *self.following(frame)]

    @staticmethod
    def assumptions(*bindings):
        conditions = list(dict.fromkeys(binding.condition for binding in bindings if binding.condition))
        if conditions:
            return "If " + " and ".join(conditions) + ", "
        if any(binding.authority == "DECLARED_RECEIVER_CANDIDATE" for binding in bindings):
            return "If the declared receiver candidates are selected, "
        return ""

    def failure(self, wait, wait_frame, transaction, tx_frame):
        wait_note, tx_note = self.note(wait.invocation), self.note(transaction.invocation)
        direct = self.direct_effects(wait, wait_frame)
        evidence = [wait_note, tx_note, wait.evidence, transaction.evidence]
        timeout = ""
        if direct:
            effects = ", ".join(note.text for note in direct)
            timeout += f"If the wait throws out of its source statement, normal continuation to {effects} in that block stops. "
        local_handlers = []
        for frame, transfer in ((wait_frame, "propagates within the callback to"),
                                (tx_frame, f"escapes the callback and {tx_note.text} to")):
            handlers = []
            for protected, role in reversed(frame.protected):
                if role != "TRY":
                    continue
                statement = self.body.expression_control(protected.statement) or protected.statement
                for _, handler, suite in statement.handlers:
                    if note := self.body.note(handler):
                        handlers.append((note, suite))
                if handlers:
                    break
            for note, suite in handlers[:2]:
                local_handlers.append(note)
                actions = self.body.handler_actions(suite)
                if actions:
                    timeout += (f"If the wait failure {transfer} {note.text} and that catch is selected, "
                                f"the handler can reach {', '.join(action.text for action in actions)} where evaluated. ")
                    local_handlers.extend(actions)
                else:
                    timeout += f"The wait failure can reach {note.text} if it {transfer} that handler and the catch is selected. "
            if handlers and frame is tx_frame and (skipped := self.following(tx_frame, limit=3)):
                timeout += ("When the failure escapes the template call, the normal continuation to "
                            f"{', '.join(action.text for action in skipped)} in the protected block is bypassed. ")
                local_handlers.extend(skipped)
        attribution = self.assumptions(transaction, wait)
        text = (f"{attribution}{wait_note.text} is a timed wait in the transaction callback of {tx_note.text}. ")
        text += timeout + "If the failure escapes the callback and template, the template call does not return normally. "
        text += "The caller's normal continuation follows the template's return. Transaction participation and durable outcome remain open."
        self.body.projector.add("WAIT_TRANSACTION_BOUNDARY", text, [*evidence, *direct, *local_handlers],
                                scope="TIMED_WAIT_IN_TRANSACTION_CALLBACK",
                                authority=[wait.authority, transaction.authority])
        self.ack_edges(transaction, tx_frame, wait, wait_note)

    def ack_edges(self, transaction, frame, origin=None, origin_note=None):
        kind = "WAIT_TRANSACTION_BOUNDARY" if origin else "TRANSACTION_BOUNDARY"
        tx = self.note(transaction.invocation)
        source = f"failure from {origin_note.text}" if origin_note else "a callback failure"
        bindings = [transaction, *([origin] if origin else ())]
        proof = [tx, transaction.evidence, *([origin_note, origin.evidence] if origin else ())]
        for ack in self.later(frame, "acknowledgment"):
            note = self.note(ack.invocation)
            opening = self.assumptions(*bindings, ack)
            evidence = [*proof, note, ack.evidence]
            if not origin:
                subject = "the source" if opening else "The source"
                self.body.projector.add(kind, opening + f"{subject} reaches {note.text} on the normal continuation "
                                        f"after {tx.text} returns. Acknowledgement outcome remains open.",
                                        evidence, scope="POST_TRANSACTION_ACK")
            subject = source if opening else source[0].upper() + source[1:]
            self.body.projector.add(kind, opening + f"{subject} bypasses the normal continuation to "
                                    f"{note.text} in the same block when it escapes {tx.text}.",
                                    evidence, scope="PRE_ACK_FAILURE")
        for protected, region in reversed(frame.protected):
            if region != "TRY":
                continue
            local = self.body.expression_control(protected.statement) or protected.statement
            handlers = [note for _, handler, _ in local.handlers if (note := self.body.note(handler))]
            if not handlers:
                continue
            for ack in self.later(protected, "acknowledgment"):
                note = self.note(ack.invocation)
                self.body.projector.add(kind, self.assumptions(*bindings, ack)
                                        + f"{note.text} follows the surrounding try/catch. If a matching handler catches "
                                          f"failure escaping {tx.text} and control reaches the normal continuation, "
                                          "the acknowledgement remains reachable.",
                                        [*proof, *handlers, note, ack.evidence], scope="POST_CATCH_ACK")

    def validation_catches(self, transaction, frame, callback_site, callback):
        tx = self.note(transaction.invocation)
        for protected in self.frames:
            if protected.callback != callback_site or protected.statement.kind == "return":
                continue
            local = self.body.expression_control(protected.statement) or protected.statement
            if local.kind != "try" or not local.handlers:
                continue
            after = self.body.potential_actions(protected.suite[protected.index + 1:])
            if not after:
                continue
            direct = self.body.potential_actions(local.body)
            nested = []
            for site, (owner, _, _) in self.body.callback_scopes.items():
                if not any(inside(owner.selector, statement.site) for statement in local.body):
                    continue
                for part in self.body.invocations.values():
                    if inside(part.selector, site) and (at := self.frame(part)) and at.callback == site:
                        nested.append(self.note(part))
            operations = list(dict.fromkeys((*direct, *nested)))[:6]
            if not operations:
                continue
            for _, handler, suite in local.handlers:
                stop = next((item for item in suite if item.kind in {"return", "throw", "break", "continue"}), None)
                if not stop or stop.kind != "return" or self.body.words(stop)[:2] != ("@", callback.label):
                    continue
                catch = self.body.note(handler)
                terminal = self.body.note(stop.site)
                if not catch or not terminal:
                    continue
                names = ", ".join(note.text for note in operations)
                skipped = ", ".join(note.text for note in after)
                opening = self.assumptions(transaction)
                region = "try initializer" if self.body.expression_control(protected.statement) else "try body"
                initial = f"{opening}in" if opening else "In"
                text = (f"{initial} this {region}, a failure from {names} reaches {catch.text} if it propagates "
                        f"out of the operation or invoked nested callback and matches the handler. "
                        f"Reaching {terminal.text} exits the transaction callback before {skipped}.")
                handling = self.body.potential_actions(suite[:suite.index(stop)], limit=2)
                if handling:
                    text += " The matching handler can reach " + ", ".join(note.text for note in handling) + " before that return."
                evidence = [tx, transaction.evidence, *operations, catch, *handling, terminal, *after]
                self.body.projector.add("TRANSACTION_BOUNDARY", text, evidence, scope="CALLBACK_CATCH_RETURN")
                for ack in self.later(frame, "acknowledgment"):
                    note = self.note(ack.invocation)
                    self.body.projector.add("TRANSACTION_BOUNDARY", self.assumptions(transaction, ack)
                                            + f"{terminal.text} bypasses {skipped} in the callback. The later source can "
                                              f"reach {note.text} if {tx.text} returns normally and the continuation is reached.",
                                            [*evidence, note, ack.evidence], scope="AFTER_CALLBACK_CATCH_RETURN_ACK")

    def selected_iteration_callbacks(self):
        for part in self.body.invocations.values():
            if part.member != "forEach" or not part.receiver or len(part.callbacks) != 1:
                continue
            frame = self.frame(part)
            if not frame:
                continue
            receiver = self.body.unit.excerpt(part.receiver).strip()
            matches = []
            for (site, _), note in self.body.sql_operands.items():
                if not note.text.startswith(("SELECT ", "`SELECT ")):
                    continue
                if inside(site, part.receiver):
                    matches.append(note)
                    continue
                if not re.fullmatch(r"[A-Za-z_]\w*", receiver):
                    continue
                for statement in frame.suite[:frame.index]:
                    if inside(site, statement.site) and self.body.words(statement)[:3] == ("val", receiver, "="):
                        matches.append(note)
            if len(matches) == 1 and part.callbacks[0] not in self.callback_iterations:
                self.selection_iterations[part.callbacks[0]] = part, matches[0]

    def transaction_iteration(self, binding, frame, tx):
        selection = self.selection_iterations.get(frame.callback)
        if selection:
            iterator, query = selection
            prefix = self.assumptions(binding) or "When the transaction binding is selected, "
            self.body.projector.add("TRANSACTION_BOUNDARY", prefix
                                    + f"source places the {query.text} request before {self.note(iterator).text}. If this "
                                      f"`forEach` iterates the selected records, {tx.text} runs within a reached element "
                                      "callback. Selection occurs outside that per-item transaction callback.",
                                    [binding.evidence, tx, self.note(iterator), query], scope="TRANSACTION_IN_ITERATION")
        elif frame.loops:
            loop = frame.loops[-1]
            first, last = loop.expression[0], loop.expression[-1]
            span = type(loop.site)(first.start, last.end, loop.site.file)
            if note := self.body.note(span):
                prefix = self.assumptions(binding) or "When the transaction binding is selected, "
                heading = ("The source evaluates the iterable before entering the loop."
                           if loop.kind == "for" else "The source tests the loop condition before entering the iteration.")
                self.body.projector.add("TRANSACTION_BOUNDARY", prefix
                                        + f"source enters an iteration of {note.text} before reaching {tx.text}. "
                                        + heading,
                                        [binding.evidence, tx, note], scope="TRANSACTION_IN_ITERATION")

    def transaction(self, binding):
        frame = self.frame(binding.invocation)
        sites = [site for site in binding.invocation.callbacks if site in self.body.callback_scopes]
        if not frame or not sites:
            return
        tx = self.note(binding.invocation)
        self.body.projector.add("TRANSACTION_BOUNDARY", self.assumptions(binding)
                                + f"{tx.text} evaluates the transaction callback before the template can return normally. "
                                  "A failure escaping the template interrupts that normal continuation. Transaction "
                                  "participation and durable outcome remain open.",
                                [tx, binding.evidence, *(self.body.text_note(site, "transaction callback") for site in sites)],
                                scope="TRANSACTION_CALLBACK_COMPLETION", authority=binding.authority)
        self.ack_edges(binding, frame)
        self.transaction_iteration(binding, frame, tx)
        for site in sites:
            _, callback, _ = self.body.callback_scopes[site]
            self.validation_catches(binding, frame, site, callback)

    def collect_queries(self):
        for key, (verb, table, details) in self.body.sql_shapes.items():
            call, _ = key
            candidates = [part for part in self.body.invocations.values()
                          if part.site.start == call.start and inside(call, part.site)
                          and part.selector.start < call.end]
            if candidates:
                part = min(candidates, key=lambda item: item.site.end)
                if (frame := self.frame(part)) and (owner := self.body.sql_owner(call)):
                    self.queries.append(Query(part, frame, self.body.sql_operands[key], verb, table, details,
                                              self.body.sql_occurrence(owner)))

    def selected_loop(self, wait, tx):
        loops = wait.loops or tx.loops
        if not loops or loops[-1].kind != "for":
            return None
        loop = loops[-1]
        loop_tokens = [token.text for token in loop.expression]
        if len(loop_tokens) != 3 or loop_tokens[1] != "in":
            return None
        variable, _, collection = loop_tokens
        if not all(re.fullmatch(r"[A-Za-z_]\w*", label) for label in (variable, collection)):
            return None
        frame = next((item for item in self.frames if item.statement.site == loop.site), None)
        return (frame, variable, collection) if frame else None

    def item_reads(self, wait, tx):
        selected = self.selected_loop(wait, tx)
        if selected:
            loop, item, collection = selected
            for query in self.queries:
                if (query.verb == "SELECT" and self.later_in_suite(query.frame, loop)
                        and query.part.site.end == query.frame.statement.site.end
                        and self.body.words(query.frame.statement)[:3] == ("val", collection, "=")):
                    statement = loop.statement
                    first, last = statement.expression[0], statement.expression[-1]
                    header = type(statement.site)(first.start, last.end, statement.site.file)
                    yield query, item, self.body.text_note(header, "loop item binding"), ""
        for site in self.callback_ancestry(tx.callback):
            if (observed := self.selection_iterations.get(site)):
                owner, selection = observed
                condition = "this `forEach` passes the selected records to its callback"
            elif (owner := self.callback_iterations.get(site)):
                selection, condition = None, ""
            else:
                continue
            callback = self.body.callback_scopes[site][1]
            if len(callback.params) != 1 or not owner.receiver or not (frame := self.frame(owner)):
                continue
            receiver = self.body.unit.excerpt(owner.receiver).strip()
            for query in self.queries:
                if query.verb != "SELECT" or selection and query.note != selection:
                    continue
                direct = query.part.site == owner.receiver
                bound = (self.later_in_suite(query.frame, frame) and query.part.site.end == query.frame.statement.site.end
                         and self.body.words(query.frame.statement)[:3] == ("val", receiver, "="))
                if direct or bound:
                    header = type(site)(site.start, self.body.unit.tokens[callback.start - 1].end, site.file)
                    yield query, callback.params[0][0], self.body.text_note(header, "element callback binding"), condition
        for anchor in dict.fromkeys((wait, tx)):
            for index, statement in enumerate(anchor.suite[:anchor.index]):
                words = self.body.words(statement)
                if len(words) < 4 or words[0] != "val" or words[2] != "=" or not re.fullmatch(r"[A-Za-z_]\w*", words[1]):
                    continue
                item = words[1]
                for query in self.queries:
                    if query.verb != "SELECT" or query.frame.suite != anchor.suite:
                        continue
                    if query.frame.index == index:
                        suffix = self.body.unit.text[query.part.site.end:statement.site.end]
                        if re.fullmatch(EXTRACT_ITEM, suffix.strip()):
                            yield query, item, self.body.text_note(statement.site, "selected item binding"), ""
                    elif (query.frame.index < index and self.later_in_suite(query.frame, anchor)
                          and query.part.site.end == query.frame.statement.site.end):
                        source = self.body.words(query.frame.statement)
                        if len(source) < 4 or source[0] != "val" or source[2] != "=":
                            continue
                        initializer = self.body.unit.text[statement.expression[3].start:statement.site.end]
                        if re.fullmatch(re.escape(source[1]) + r"\s*" + EXTRACT_ITEM, initializer.strip()):
                            yield query, item, self.body.text_note(statement.site, "selected item binding"), ""

    def unshadowed_item(self, item, proof, part):
        for index, token in enumerate(self.body.unit.tokens):
            if (proof.site.end <= token.start < part.selector.start and token.text in {"val", "var"}
                    and self.body.unit.words[index + 1] == item):
                return False
        return all(site.start == proof.site.start or not inside(part.selector, site)
                   or all(name != item for name, _ in callback.params)
                   for site, (_, callback, _) in self.body.callback_scopes.items())

    def matching_key(self, read, write, item):
        selected = next((value for label, value in read.details if label == "selects"), "")
        predicate = next((value for label, value in write.details if label == "filters by"), "")
        if not selected or not predicate:
            return None
        columns = {field.strip() for field in selected.split(",")}
        candidates = set()
        for term in SQL_TERMS(predicate, "AND", blocked="OR"):
            key = re.fullmatch(r"(?:[A-Za-z_]\w*\.)?([A-Za-z_]\w*)\s*=\s*(?::([A-Za-z_]\w*)|(\?))", term.strip())
            if not key or key[1] not in columns and "*" not in columns:
                continue
            access = rf'{re.escape(item)}(?:\.{re.escape(key[1])}|\s*\[\s*"{re.escape(key[1])}"\s*\])'
            if key[2]:
                named = rf'"{re.escape(key[2])}"\s+to\b'
                if sum(len(re.findall(named, actual)) for actual in write.part.actuals) != 1:
                    continue
                bound = rf'{named}\s+({access})(?=\s*[,)]|$)'
                candidates.update((key[1], match[1]) for actual in write.part.actuals if (match := re.search(bound, actual)))
            elif sum(value.count("?") for _, value in write.details) == 1:
                candidates.update((key[1], actual.strip("`")) for actual in write.part.actuals
                                  if re.fullmatch(access, actual.strip("`")))
        return next(iter(candidates)) if len(candidates) == 1 else None

    @staticmethod
    def eligibility_mark(filters, write):
        assignments = {}
        for setting in SQL_TERMS(dict(write.details).get("sets", ""), ","):
            if parts := re.fullmatch(r"([A-Za-z_]\w*)\s*=\s*(.+)", setting):
                column = parts[1].lower()
                if column in assignments:
                    return None
                assignments[column] = parts[2]
        for term in SQL_TERMS(filters, "AND", blocked="OR"):
            predicate = re.fullmatch(rf"(?:[A-Za-z_]\w*\.)?([A-Za-z_]\w*)\s*(IS\s+NULL|=\s*{SQL_MARKER})", term, re.I)
            if not predicate or not (value := assignments.get(predicate[1].lower())):
                continue
            if predicate[2].upper().startswith("IS"):
                if ((re.fullmatch(SQL_MARKER, value, re.I) and value.upper() != "NULL")
                        or SQL_MARK_TIMESTAMP.fullmatch(value)):
                    return term
            else:
                previous = predicate[2][1:].strip()
                # Same-kind literals prevent implying changes under implicit SQL casts.
                if (previous.upper(), value.upper()) in {("TRUE", "FALSE"), ("FALSE", "TRUE")}:
                    return term
                integer = r"-?\d{1,18}"
                if re.fullmatch(integer, previous) and re.fullmatch(integer, value) and int(previous) != int(value):
                    return term
                label = r"'[A-Za-z_][A-Za-z_0-9]*'"
                if re.fullmatch(label, previous) and re.fullmatch(label, value) and previous.lower() != value.lower():
                    return term
        return None

    def publication_result(self, part, frame, wait, publications):
        if not part.receiver:
            return None
        timed = part.arity == 2 and any(TIME_UNIT.fullmatch(value.strip("`")) for value in part.actuals)
        if not wait and (part.member != "get" or not (part.arity == 0 or timed)):
            return None
        if not wait and any(self.occurrence(part, site) for site, _ in self.native):
            return None
        receiver = self.body.unit.excerpt(part.receiver)
        local = re.fullmatch(r"[A-Za-z_]\w*", receiver)
        matches = []
        for publication in publications:
            origin = publication.invocation
            if not (source := self.frame(origin)):
                continue
            if source == frame and origin.site == part.receiver:
                matches.append((publication, None))
            elif local and self.later_in_suite(source, frame) and origin.site.end == source.statement.site.end:
                words = self.body.words(source.statement)
                if (words[:3] == ("val", receiver, "=") and source.statement.expression[3].start == origin.site.start
                        and not any(self.body.words(statement)[:2] in {("val", receiver), ("var", receiver)}
                                    for statement in source.suite[source.index + 1:frame.index])):
                    matches.append((publication, self.body.text_note(source.statement.site, "immutable send result")))
        if len(matches) != 1:
            return None
        publication, alias = matches[0]
        if wait is None or wait.authority == "CHAIN_RECEIVER_TYPE_OPEN":
            wait = Binding(part, "timed_wait" if timed else "wait", self.body.text_note(part.selector, "publication future wait"),
                           publication.authority, "the send result selects a Java Future `get` overload")
        return wait, publication, alias

    def repeat_after_mark(self, read, write, item_note, key, guard, result, transaction, iteration):
        wait, publication, alias = result
        source, waited = self.note(publication.invocation), self.note(wait.invocation)
        tx = self.note(transaction.invocation)
        column, actual = key
        if iteration:
            wait = replace(wait, condition=" and ".join(filter(None, (wait.condition, iteration))))
        heading = self.assumptions(transaction, publication, wait)
        first = "the" if heading else "The"
        text = (f"{heading}{first} callback of {tx.text} can reach {source.text} and wait for that send result at {waited.text}. "
                f"If the wait returns normally, it can reach the separate mark {write.note.text}, using `{actual}` for the key. "
                f"If the mark fails, or the participating transaction fails to commit the mark after the callback completes, "
                f"a later poll can reach another publication request for the same `{column}` if the same row remains unmarked, "
                f"still satisfies `{guard}` and the other selection conditions, and is selected again. "
                "The provider must bind the key and apply the mark as written. Broker acceptance and consumer completion remain open.")
        callback = next(site for site in transaction.invocation.callbacks if inside(wait.invocation.selector, site))
        self.body.projector.add("POSSIBLE_REPEAT_AFTER_MARK_FAILURE", text,
                                [self.note(read.part), read.note, item_note, source, publication.evidence,
                                 *([alias] if alias else ()), waited, wait.evidence, self.note(write.part), write.note,
                                 tx, transaction.evidence, self.body.text_note(callback, "transaction callback")],
                                scope="POLL_REPEAT_IF_MARK_NOT_PERSISTED")

    def reentry(self, wait, wait_frame, transaction, tx_frame, publication):
        writes = [query for query in self.queries if query.verb in {"UPDATE", "DELETE"}
                  and self.later_in_suite(wait_frame, query.frame)]
        for read, item, alias, iteration in self.item_reads(wait_frame, tx_frame):
            filters = next((value for label, value in read.details if label == "filters by"), "")
            filter_text = f"the same predicate `{filters}`" if filters and "`" not in filters else "the same selection predicates"
            for write in writes:
                if (read.table != write.table or not self.unshadowed_item(item, alias, write.part)
                        or not (key := self.matching_key(read, write, item))):
                    continue
                if publication and (guard := self.eligibility_mark(filters, write)):
                    self.repeat_after_mark(read, write, alias, key, guard, publication, transaction, iteration)
                    continue
                if wait is None or iteration:
                    continue
                column, actual = key
                wait_note, tx_note = self.note(wait.invocation), self.note(transaction.invocation)
                self.body.projector.add("POSSIBLE_LATER_SELECTION",
                                        self.assumptions(transaction, wait)
                                        + f"the normal continuation after {wait_note.text} can reach {write.note.text}. "
                                          f"Its key comes from `{actual}`. If that value preserves the selected "
                                          f"`{column}` and a failed write leaves the record eligible, a later poll can select "
                                          f"that key while {filter_text} still holds. This depends on matching provider "
                                          "bindings and persisted eligibility.",
                                        [self.note(read.part), read.note, alias, self.note(write.part), write.note,
                                         wait_note, tx_note, wait.evidence, transaction.evidence],
                                        scope="POLL_ELIGIBILITY_CONDITIONAL")

    def callback_ancestry(self, site):
        while site:
            yield site
            if site not in self.callback_iterations and site not in self.body.callback_inline:
                return
            if not (retained := self.body.callback_scopes.get(site)):
                return
            owner = self.frame(retained[0])
            site = owner.callback if owner else None

    def row_operand(self, tokens, item, selected, publication, frames):
        if not tokens or not self.unshadowed_item(item, selected, publication):
            return ()
        unit = self.body.unit
        location = type(publication.site)(tokens[0].start, tokens[-1].end, publication.site.file)
        operands = unit.excerpt(location)
        cast = r'(?:\s+as\s+[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)?'
        path = r'(?:(?:\s*\.\s*[A-Za-z_]\w*)|(?:\s*\[\s*"[^"\\]+"\s*\]))*'
        proof = []
        upper = publication.selector.start
        for _ in range(4):
            if re.fullmatch(re.escape(item) + path + cast, operands):
                return [*proof, value] if (value := self.body.note(location)) else ()
            if not (name := re.fullmatch(r'([A-Za-z_]\w*)' + cast, operands)):
                return ()
            candidates = {}
            for frame in frames:
                for statement in frame.suite[:frame.index]:
                    words = self.body.words(statement)
                    if (selected.site.start <= statement.site.start < upper and words[:1] in {("val",), ("var",)}
                            and words[1:2] == (name[1],)):
                        candidates[statement.site] = statement
            if len(candidates) != 1:
                return ()
            statement, = candidates.values()
            if self.body.words(statement)[:3] != ("val", name[1], "="):
                return ()
            initializer = statement.expression[3:]
            if (not initializer or not (binding := self.body.note(statement.site))
                    or not self.unshadowed_item(name[1], binding, publication)
                    or not (use := self.body.note(location))):
                return ()
            proof.extend((use, binding))
            upper = statement.site.start
            location = type(location)(initializer[0].start, initializer[-1].end, location.file)
            operands = unit.excerpt(location)
        return ()

    def selected_publication(self, binding, frame, arguments, occurrence, literal):
        part = binding.invocation
        guards = list(frame.guards)
        frames = [frame]
        callback = frame.callback
        while callback:
            owner = self.body.callback_scopes[callback][0]
            outer = self.frame(owner)
            if not outer or not (callback in self.callback_iterations or callback in self.selection_iterations
                                 or callback in self.body.callback_inline
                                 or any(item.kind == "transaction" and item.invocation == owner for item in self.bindings)):
                return
            frames.append(outer)
            guards.extend(outer.guards)
            callback = outer.callback
        callbacks = {frame.callback for frame in frames}
        frames.extend(parent for parent in self.frames if parent.callback in callbacks
                      and inside(part.selector, parent.statement.site))
        if any(region != "TRY" for at in frames for _, region in at.protected):
            return
        selections = dict.fromkeys(selection for at in frames for selection in self.item_reads(frame, at))
        for query, item, selected, iteration in selections:
            operands = [proof for argument in arguments if (proof := self.row_operand(argument, item, selected, part, frames))]
            if not operands:
                continue
            guards = list(dict.fromkeys(guards))
            conditions = [binding.condition, iteration, *(self.body.guard_text(guard, taken) for guard, taken in guards)]
            values = " and ".join(dict.fromkeys(proof[0].text for proof in operands))
            self.body.projector.add(
                "POLL_ROW_PUBLICATION", f"The selected row from {query.note.text} supplies {values} to "
                f"{self.note(part).text} on the retained continuation.",
                [self.note(query.part), query.note, selected, binding.evidence, literal,
                 *(note for proof in operands for note in proof), *(guard for guard, _ in guards)],
                authority=binding.authority, condition=" and ".join(filter(None, conditions)),
                uncertainty="Selection, API binding and broker delivery remain conditional.",
                _pollPublication={"readOccurrence": query.occurrence, "publicationOccurrence": occurrence})

    def publication(self, binding):
        part = binding.invocation
        if not (frame := self.frame(part)):
            return
        unit = self.body.unit
        first = unit.token_indices[part.selector.start]
        stop = unit.token_ends[part.selector.end] + 1
        opening = next((index for index in range(first + 1, stop) if unit.words[index] == "("), None)
        if opening is None:
            return
        closing = unit.pairs[opening]
        operands = unit.tokens[opening + 1:closing]
        # A direct String overload starts with one literal followed by a comma.
        if (len(operands) < 2 or operands[1].text != "," or not (match := TOPIC_OPERAND.fullmatch(operands[0].text))
                or match[1] in {".", ".."}):
            return
        argument = operands[0]
        literal = self.body.text_note(type(part.selector)(argument.start, argument.end, part.selector.file),
                                      "topic operand")
        prefix = self.assumptions(binding)
        occurrence = f"{self.body.ranges.by_ref[self.body.ref]}:{part.selector.start}:{part.selector.end}"
        self.body.projector.add(
            "BROKER_PUBLICATION", (prefix + "the source" if prefix else "The source")
            + f" submits a Kafka publication request for topic `{match[1]}` if {self.note(part).text} is reached.",
            [binding.evidence, literal], authority=binding.authority,
            **({"condition": binding.condition} if binding.condition else {}),
            uncertainty=("The retained declared receiver is assumed to select the Kafka producer API. "
                         if binding.authority != "COMPILER_SELECTED" else "")
            + "Broker mapping, acknowledgement and delivery remain open.",
            _edge={"kind": "PUBLISH", "topic": match[1], "site": binding.evidence.citations[0],
                   "occurrence": occurrence})
        self.selected_publication(binding, frame, tuple(self.body.operand_groups(operands))[1:], occurrence, literal)

    def derive(self):
        self.selected()
        for part in self.body.invocations.values():
            if found := self.binding(part):
                self.bindings.append(found)
            if any(self.occurrence(part, site) and FOREACH.match(target) for site, target in self.native):
                for callback in part.callbacks:
                    self.callback_iterations[callback] = part
        self.scan(self.root)
        for site, (_, _, suite) in self.body.callback_scopes.items():
            self.scan(suite, callback=site)
        self.collect_queries()
        self.selected_iteration_callbacks()
        publications = [entry for entry in self.bindings if entry.kind == "publication"]
        for publication in publications:
            self.publication(publication)
        for protected, local, handler, suite in self.catches:
            self.catch_region(protected, local, handler, suite)
        transactions = [entry for entry in self.bindings if entry.kind == "transaction"]
        for transaction in transactions:
            self.transaction(transaction)
        if transactions:
            waits = {entry.invocation: entry for entry in self.bindings if entry.kind in {"wait", "timed_wait"}}
            for part in self.body.invocations.values():
                if part.member not in {"get", "join"}:
                    continue
                frame = self.frame(part)
                if frame is None or frame.callback is None:
                    continue
                wait = waits.get(part)
                publication = self.publication_result(part, frame, wait, publications)
                selected_wait = publication[0] if publication else wait
                timed = selected_wait if selected_wait and selected_wait.kind == "timed_wait" else None
                if not timed and not publication:
                    continue
                ancestors = set(self.callback_ancestry(frame.callback))
                for transaction in transactions:
                    if ancestors.intersection(transaction.invocation.callbacks) and (tx_frame := self.frame(transaction.invocation)):
                        if timed:
                            self.failure(timed, frame, transaction, tx_frame)
                        self.reentry(timed, frame, transaction, tx_frame, publication)
