"""Project normal Java evaluations from exact retained method source."""

from base64 import b64encode
from collections import defaultdict
from dataclasses import dataclass, field, replace
from hashlib import sha256
from itertools import islice, zip_longest
import json
from pathlib import Path
import runpy
import shutil
import subprocess


AUTHORITY = "RETAINED_JAVAC_SYNTAX"
SAME_SCOPE = {"SAME_SCOPE_UNIQUE", "SAME_SCOPE_SELECTED_DECLARATION"}
MAX_BODY_BYTES = 24 * 1024
MAX_BODIES = 6
MAX_STATES = 20
MAX_STEPS = 6000
MAX_EVENTS = 20
MAX_FACTS = 18
MAX_OUTPUT_BYTES = 7600
MAX_VALUE_USES = 80
MAX_VALUE_VARIANTS = 4
MAX_VALUE_FACTS = 8
MAX_VALUE_BYTES = 2500
MAX_DECISION_FACTS = 9
MAX_DECISION_BYTES = 3600
MAX_PREDICATE_FACTS = 3
MAX_BYPASSED = 4
PRIMITIVES = {"boolean", "byte", "short", "int", "long", "char", "float", "double"}
CONTROL = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_control.py")))
DECISIONS = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_decisions.py")))


@dataclass(frozen=True)
class Range:
    identifier: str
    file: str
    revision: str
    line: int
    text: bytes
    offset: int | None = None
    end_line: int | None = None

    def local_span(self, other):
        if (self.file, self.revision) != (other.file, other.revision):
            return None
        if self.offset is None or other.offset is None:
            return None
        start = other.offset - self.offset
        end = start + len(other.text)
        if 0 <= start < end <= len(self.text) and self.text[start:end] == other.text:
            return start, end
        return None


@dataclass(frozen=True)
class Note:
    text: str
    span: tuple[int, int]
    references: tuple[str, ...] = ()


@dataclass(frozen=True)
class Effect:
    note: Note
    origin: tuple[Note, ...] = ()
    gates: tuple[Note, ...] = ()


@dataclass(frozen=True)
class Value:
    note: Note
    kind: str
    constant: str | None = None
    definition: Note | None = None
    gates: tuple[Note, ...] = ()


@dataclass(frozen=True)
class Use:
    role: str
    expression: Note
    enclosing: Note | None = None


@dataclass(frozen=True)
class State:
    conditions: tuple[Note, ...] = ()
    gates: tuple[Note, ...] = ()
    scope: Note | None = None
    effects: tuple[Effect, ...] = ()
    bindings: dict[str, Value] = field(default_factory=dict)
    locals: frozenset[str] = frozenset()
    omitted: bool = False
    arm_depth: int = 0
    arm_effects: int = 0
    loop_reached: bool = False
    watching: tuple[Use, ...] = ()

    def bind(self, name, value):
        bindings = dict(self.bindings)
        bindings.pop(name, None)
        if value:
            bindings[name] = value
        return replace(self, bindings=bindings)

    def declare(self, name, value):
        return replace(self.bind(name, value), locals=self.locals | {name})

    def close(self, names):
        return replace(self, bindings={name: value for name, value in self.bindings.items() if name not in names},
                       locals=self.locals - names)

    def effect(self, effect):
        effect = replace(effect, gates=self.gates)
        if len(self.effects) >= MAX_EVENTS:
            middle = MAX_EVENTS // 2
            effects = (*self.effects[:middle], *self.effects[middle + 1:], effect)
        else:
            effects = (*self.effects, effect)
        return replace(self, effects=effects, omitted=self.omitted or len(self.effects) == MAX_EVENTS,
                       arm_effects=self.arm_effects + bool(self.arm_depth))

    def condition(self, note):
        return replace(self, conditions=(*self.conditions, note))


@dataclass(frozen=True)
class Ending:
    state: State
    outcome: Note | None = None
    origin: Note | None = None
    skipped: tuple[Note, ...] = ()
    local_throw: bool = False
    optional_arm: bool = False


@dataclass(frozen=True)
class Context:
    future: tuple[Note, ...] = ()
    throw_future: tuple[Note, ...] = ()
    caught: bool = False
    iteration: bool = False


def joined(items):
    if len(items) <= 1:
        return "".join(items)
    return ", ".join(items[:-1]) + " and " + items[-1]


def compact(text, *, limit=125):
    if text and len(text) <= limit and not any(ch in text for ch in ("\r", "\n", "`", "](")):
        return f"`{text.strip()}`" if text.strip() else None
    return None


def span(node):
    return node[1], node[2]


def interleave(buckets):
    return (item for batch in zip_longest(*buckets) for item in batch if item is not None)


def descendants(parts):
    if not isinstance(parts, list) or not parts:
        return
    if isinstance(parts[0], str):
        yield parts
        parts = parts[3:]
    for item in parts:
        yield from descendants(item)


def local_writes(nodes):
    return {node[3][3] for node in nodes if node[0] in {"set", "mutate"} and node[3][0] == "id"}


def unknown_writes(nodes):
    return any(node[0] in {"assert", "boundary", "open"} or (node[0] == "try" and any(node[5:]))
               for node in nodes)


def witness_order(state):
    return state.arm_effects, state.omitted, len(state.effects), len(state.conditions)


@dataclass(eq=False, frozen=True)
class Proposition:
    detail: dict
    brief: dict | None = None
    exit_site: tuple[int, ...] | None = None


def ordered_endings(endings):
    by_exit = defaultdict(lambda: defaultdict(list))
    for row in endings:
        key = row.outcome.span if row.outcome else None
        effects = tuple(effect.note.text for effect in row.state.effects)
        by_exit[key][effects, row.outcome, row.origin].append(row)
    # Different lexical exits come first. Within one exit, prefer different
    # effects and value mappings over combinations that only vary the premise.
    ordered = []
    for variants in by_exit.values():
        buckets = [sorted(rows, key=lambda row: witness_order(row.state)) for rows in variants.values()]
        # Prefer a witnessed continuation without optional earlier effects.
        buckets.sort(key=lambda rows: witness_order(rows[0].state))
        ordered.append(list(interleave(buckets)))
    return interleave(ordered)


class Body:
    """Keep full branch states to preserve the value at each incoming edge of a join."""

    def __init__(self, source, parsed, exact, field_uses=None, predicate_uses=None, *, capture_decisions=False):
        self.source = source
        self.name, self.formals, self.block, self.signature = parsed
        self.parameters = frozenset(parameter[3] for parameter in self.formals)
        self.shared_inputs = {}
        self.exact = [(bounds, row.identifier) for row in exact
                      if (bounds := source.local_span(row))]
        self.endings = []
        self.iteration_endings = []
        self.optional_arms = {}
        self.steps = MAX_STEPS
        self.limited = False
        self.open_sites = {}
        self.call_conditions = {}
        self.field_uses = field_uses or {}
        self.predicate_uses = predicate_uses or {}
        self.predicate_calls = {}
        self.value_uses = {}
        regions = DECISIONS["regions"](self.block, self.parameters, self.guard, self.catch_condition)
        self.decision_regions = list(islice(regions, DECISIONS["MAX_REGIONS"])) if capture_decisions else []
        self.decision_inputs = {span(block[3][0]): (DECISIONS["reads"](block) & names, {})
                                for block, names, _, _ in self.decision_regions}

    def cite(self, notes, linked):
        refs = [*linked, self.source.identifier]
        for note in notes:
            start, end = note.span
            candidates = [(last - first, identifier) for (first, last), identifier in self.exact
                          if first <= start < end <= last]
            if candidates:
                refs.append(min(candidates)[1])
        extra = (reference for note in notes for reference in note.references)
        return list(dict.fromkeys([*list(dict.fromkeys(refs))[:7], *extra]))

    def code(self, node):
        if node:
            return compact(self.source.text[node[1]:node[2]].decode())

    def value(self, state, node):
        if node[0] == "id" and node[3] in state.locals:
            return state.bindings.get(node[3])
        if node[0] in {"id", "member"} and (field := self.field_uses.get(span(node))):
            initializer, constant, ref = field
            use = self.code(node) or "the cited field"
            definition = Note(f"the static constant declaration for {use}", span(node), (ref,))
            note = Note(compact(initializer) or "its cited initializer", span(node), (ref,))
            return Value(note, "constant", constant, definition)
        if node[0] in {"literal", "call", "new"} and (code := self.code(node)):
            constant = node[3] if node[0] == "literal" else "NONNULL" if node[0] == "new" else None
            return Value(Note(code, span(node)), node[0], constant)
        return None

    def origin(self, node):
        if code := self.code(node):
            return Value(Note(code, span(node)), "expression")

    def defined(self, value, node, name, gates=()):
        if value:
            label = self.code(node) or f"the cited binding for `{name}`"
            return replace(value, definition=Note(label, span(node), value.note.references),
                           gates=tuple(dict.fromkeys((*value.gates, *gates))))

    def observe(self, state, node, value):
        if not state.watching or not (code := self.code(node)):
            return state
        origin = (value.note, value.definition) if value and value.definition and code != value.note.text else None
        if origin is None and (node[0] != "id" or node[3] not in state.locals):
            return state
        read = Note(code, span(node))
        for use in reversed(state.watching):
            # Nested checks and argument evaluation keep their own lexical expression.
            if not use.expression.span[0] <= read.span[0] < read.span[1] <= use.expression.span[1]:
                continue
            key = use, read
            if key not in self.value_uses and len(self.value_uses) >= MAX_VALUE_USES:
                break
            definitions = self.value_uses.setdefault(key, {})
            if origin in definitions or origin is None or len(definitions) < MAX_VALUE_VARIANTS:
                supporting = definitions.get(origin, state.conditions)
                definitions[origin] = tuple(note for note in supporting if note in state.conditions)
            break
        return state

    def evaluated_use(self, state, node, role, enclosing=None):
        if not (code := self.code(node)):
            return self.evaluate(state, node)
        use = Use(role, Note(code, span(node)), enclosing)
        entered = replace(state, watching=(*state.watching, use))
        return [(replace(out, watching=state.watching), value) for out, value in self.evaluate(entered, node)]

    def actual(self, node, value):
        label = self.code(node) or "the cited argument expression"
        if (node[0] in {"id", "choose"} and value and value.kind != "caught"
                and not value.note.references and value.note.text != label):
            return f"{label} (from {value.note.text})", (value.note,)
        return label, ()

    def callable(self, node):
        if node[0] == "id":
            return self.code(node)
        if node[0] == "member":
            return self.code(node) or compact(node[4])

    def call_effect(self, state, node, target, arguments, actuals):
        label = self.callable(target) if node[0] == "call" else self.code(target)
        actuals = actuals[:4]
        mapped = any(origin for _, origin in actuals)
        if not mapped and (code := self.code(node)):
            text = code
        elif label:
            rendered = [self.code(arg) or "an expression" for arg in arguments[:4]]
            joined_args = ", ".join(item.strip("`") for item in rendered)
            if len(arguments) > 4:
                joined_args += ", …"
            prefix = "new " if node[0] == "new" else ""
            text = f"`{prefix}{label.strip('`')}({joined_args})`"
        else:
            text = "the cited invocation"
        if mapped:
            changes = [text for text, origin in actuals if origin]
            text += " with " + joined(changes)
        bounds = span(node)
        if node[0] == "call":
            self.observe_predicate(state, node, arguments, actuals)
            premises = (*([state.scope] if state.scope else []), *state.gates)
            existing = self.call_conditions.get(bounds, premises)
            self.call_conditions[bounds] = tuple(note for note in existing if note in premises)
        return Effect(Note(text, bounds), tuple(note for _, origin in actuals for note in origin))

    def bounded(self, states):
        if len(states) <= MAX_STATES:
            return states
        self.limited = True
        last = len(states) - 1
        kept = [index * last // (MAX_STATES - 1) for index in range(MAX_STATES)]

        def cost(index):
            item = states[index]
            return witness_order(item if isinstance(item, State) else item[0])

        simplest = min(range(len(states)), key=cost)
        if simplest not in kept:
            nearest = min(range(1, len(kept) - 1), key=lambda index: abs(kept[index] - simplest))
            kept[nearest] = simplest
        return [states[index] for index in sorted(kept)]

    def under_gate(self, state, condition, holds, evaluate):
        entered = replace(state, gates=(*state.gates, self.guard(condition, holds)))
        return [(replace(out, gates=state.gates), value) for out, value in evaluate(entered)]

    def evaluate_all(self, states, nodes):
        for node in nodes:
            states = self.bounded([out for state in states for out, _ in self.evaluate(state, node)])
        return states

    def evaluate(self, state, node):
        if node is None:
            return [(state, None)]
        self.steps -= 1
        if self.steps < 0:
            self.limited = True
            return []
        kind, parts = node[0], node[3:]
        if kind in {"id", "literal", "deferred"}:
            value = self.value(state, node)
            return [(self.observe(state, node, value) if kind == "id" else state, value)]
        if kind == "open":
            self.open_sites.setdefault("expression", span(node))
            note = Note("the cited expression completes normally", span(node))
            return [(replace(state.condition(note), bindings={}), None)]
        if kind == "member":
            value = self.value(state, node)
            return [(self.observe(result, node, value), value) for result, _ in self.evaluate(state, parts[0])]
        if kind in {"and", "or", "not"}:
            return [(out, None) for out, _ in self.test(state, node, guard=False)]
        if kind == "choose":
            results = []
            for before, holds in self.test(state, parts[0]):
                arm = parts[1] if holds else parts[2]
                origin = self.origin(arm)
                evaluated = self.under_gate(before, parts[0], holds, lambda entered: self.evaluate(entered, arm))
                results.extend((out, value or origin) for out, value in evaluated)
            return self.bounded(results)
        if kind in {"eq", "neq"}:
            return self.compare(state, node, *parts)
        if kind in {"ordered", "array"}:
            return [(result, None) for result in self.evaluate_all([state], parts)]
        if kind in {"set", "mutate"}:
            target, *operands = parts
            results = []
            # A simple assignment does not read the previous value of its target.
            address = target
            if kind == "set":
                address = target[3] if target[0] == "member" else None if target[0] == "id" else target
            for before, _ in self.evaluate(state, address):
                if kind == "set":
                    values = (self.evaluated_use(before, operands[0], "assignment", Note(code, span(node)))
                              if target[0] == "member" and (code := self.code(node))
                              else self.evaluate(before, operands[0]))
                    for out, value in values:
                        if target[0] == "id" and target[3] in out.locals:
                            origin = self.defined(value or self.origin(operands[0]), node, target[3], out.conditions)
                            out = out.bind(target[3], origin)
                        elif code := self.code(node):
                            label, origin = self.actual(operands[0], value)
                            shown = f"the assignment {code}" + (f" with {label}" if origin else "")
                            out = out.effect(Effect(Note(shown, span(node)), origin))
                        # Assignment conversions can change the result's Java type.
                        results.append((out, None))
                else:
                    for out in self.evaluate_all([before], operands):
                        results.append((out.bind(target[3], None) if target[0] == "id" else out, None))
            return results
        if kind in {"call", "new"}:
            if kind == "call":
                target, arguments = parts
                receiver = target[3] if target[0] == "member" else None
            else:
                receiver, target, arguments = parts
            return self.invoke(state, node, receiver, target, arguments)
        return []

    def compare(self, state, node, left, right):
        results = []
        for before, first in self.evaluate(state, left):
            for out, second in self.evaluate(before, right):
                known = {value.constant for value in (first, second) if value}
                equal = None
                if first and second and first.constant == second.constant == "NULL":
                    equal = True
                elif "NULL" in known and known & {"EMPTY_STRING", "NONEMPTY_STRING", "NONNULL"}:
                    equal = False
                elif first and second and known <= {"TRUE", "FALSE"}:
                    equal = first.constant == second.constant
                value = None
                if equal is not None and (origin := self.origin(node)):
                    holds = equal == (node[0] == "eq")
                    value = replace(origin, constant="TRUE" if holds else "FALSE")
                results.append((out, value))
        return self.bounded(results)

    def invoke(self, state, node, receiver, target, arguments):
        pending = [(out, value, ()) for out, value in self.evaluate(state, receiver)]
        label = self.callable(target) if node[0] == "call" else self.code(target)
        invocation = Note(label or "the cited invocation", span(node))
        for index, argument in enumerate(arguments, 1):
            # Capture each argument's origin before evaluating any later arguments.
            role = f"argument {index}"
            pending = self.bounded([
                (out, recv, (*actuals, self.actual(argument, value)))
                for before, recv, actuals in pending
                for out, value in self.evaluated_use(before, argument, role, invocation)])
        results = []
        for out, recv, actuals in pending:
            result = self.value(out, node)
            # A retained String literal identifies the actual receiver object.
            # For arbitrary receivers, a matching member name proves no behavior.
            if (result and node[0] == "call" and target[0] == "member" and target[4] == "isEmpty"
                    and not arguments and recv and recv.constant in {"EMPTY_STRING", "NONEMPTY_STRING"}):
                result = replace(result, constant="TRUE" if recv.constant == "EMPTY_STRING" else "FALSE")
            results.append((out.effect(self.call_effect(out, node, target, arguments, actuals)), result))
        return results

    def test(self, state, node, *, guard=True):
        kind, parts = node[0], node[3:]
        if kind == "not":
            return [(out, not holds) for out, holds in self.test(state, parts[0], guard=guard)]
        if kind in {"and", "or"}:
            proceed = kind == "and"
            return self.bounded([result for before, holds in self.test(state, parts[0], guard=guard)
                                 for result in (self.under_gate(before, parts[0], holds,
                                     lambda entered: self.test(entered, parts[1], guard=guard))
                                     if holds == proceed else [(before, holds)])])
        if kind == "choose":
            return self.bounded([result for before, holds in self.test(state, parts[0])
                                 for result in self.under_gate(before, parts[0], holds,
                                     lambda entered: self.test(entered, parts[1] if holds else parts[2], guard=guard))])
        results = []
        evaluated = self.evaluated_use(state, node, "guard") if guard else self.evaluate(state, node)
        for out, value in evaluated:
            known = value.constant if value and value.constant in {"TRUE", "FALSE"} else None
            options = (known == "TRUE",) if known else (True, False)
            for holds in options:
                expression = self.code(node) or "the cited condition"
                note = Note(f"{expression} is {str(holds).lower()}", span(node))
                results.append((out if kind == "literal" else out.condition(note), holds))
        return self.bounded(results)

    def later(self, statements):
        """Find bypassable evaluations on the following lexical sequence."""
        for expression, region, conditional in CONTROL["following_sites"](statements, limit=MAX_BYPASSED):
            if code := self.code(expression):
                text = CONTROL["site_label"](code, region, conditional)
                yield Note(text, span(expression))

    def location(self, node, statement):
        start, end = span(node)
        line = self.source.line + self.source.text[:start].count(b"\n")
        return Note(f"control reaches {statement} on line {line}", (start, end))

    def guard(self, condition, holds):
        while condition[0] == "not":
            condition, holds = condition[3], not holds
        return Note(f"{self.code(condition) or 'the cited check'} is {str(holds).lower()}", span(condition))

    def independent_arms(self, node, locals_):
        if node[0] != "if" or node[3][0] == "literal":
            return False
        items = list(descendants(node))
        if any(item[0] in {"return", "throw", "try", "foreach", "break", "continue", "assert", "boundary", "open"} for item in items):
            return False
        # Keep value-dependent joins in the ordinary path analysis. Compaction
        # only splits fallthrough arms that preserve the enclosing locals.
        if local_writes(items) & locals_:
            return False
        return any(item[0] in {"call", "new", "set", "mutate"}
                   for arm in node[4:] for item in descendants(arm))

    def observe_arm(self, before, after, condition, holds, bounds):
        if len(self.optional_arms) >= MAX_FACTS:
            self.limited = True
            return
        first, last = span(bounds)
        effects = []
        for effect in after.effects:
            start, end = effect.note.span
            if first <= start < end <= last:
                # The scoped arm does not inherit a particular incoming value.
                code = compact(self.source.text[start:end].decode()) or "the cited evaluation"
                effects.append(Effect(Note(code, (start, end))))
        if not effects:
            return
        guard = self.guard(condition, holds)
        premises = (*([after.scope] if after.scope else []), *after.conditions)
        local = [note for note in premises if first <= note.span[0] < note.span[1] <= last]
        guards = tuple(dict.fromkeys([*before.gates, guard, *local]))
        state = State(conditions=guards, scope=self.location(condition, "the check"), effects=tuple(effects))
        key = (guards, tuple(effects))
        self.optional_arms.setdefault(key, Ending(state, optional_arm=True))

    def invariants_after(self, invariants, node):
        items = list(descendants(node))
        if unknown_writes(items):
            invariants.clear()
            return
        for name in local_writes(items):
            invariants.pop(name, None)
        if node[0] != "let":
            return
        name, initializer = node[3:]
        invariants.pop(name, None)
        if initializer is not None and initializer[0] not in {"choose", "and", "or", "not", "set", "mutate"}:
            value = self.value(State(bindings=invariants, locals=frozenset(invariants)), initializer) or self.origin(initializer)
            if value:
                invariants[name] = self.defined(value, node, name)

    def common_effects(self, sources, gates):
        allowed = set(gates)
        indexes = [{effect.note.span: effect for effect in state.effects} for state in sources]
        common = []
        for bounds, effect in indexes[0].items():
            if any(bounds not in index or not allowed.issuperset(index[bounds].gates) for index in indexes):
                continue
            if effect.origin or not all(index[bounds] == effect for index in indexes):
                start, end = bounds
                code = compact(self.source.text[start:end].decode()) or "the cited evaluation"
                effect = Effect(Note(code, bounds), gates=effect.gates)
            common.append(effect)
        return tuple(common)

    def after_join(self, sources, next_statement, invariants):
        bindings = dict(invariants)
        gates, effects = (), ()
        if not self.limited:
            gates = tuple(gate for gate in sources[0].gates if all(gate in state.gates for state in sources))
            effects = self.common_effects(sources, gates)
            bindings.update({name: value for name, value in sources[0].bindings.items()
                             if set(gates).issuperset(value.gates)
                             and all(state.bindings.get(name) == value for state in sources)})
        # Sampled predecessors do not establish a common evaluation or gate.
        state = State(conditions=gates, gates=gates, scope=self.location(next_statement, "the continuation"),
                      effects=effects, bindings=bindings, locals=sources[0].locals, arm_depth=sources[0].arm_depth,
                      loop_reached=any(state.loop_reached for state in sources))
        return [state]

    def capture_inputs(self, node, sources):
        if region := self.decision_inputs.get(span(node)):
            names, inputs = region
            for state in sources:
                if state.scope and state.scope.span == span(node):
                    # An immediately preceding join already starts the ordinary
                    # projection at this same decision with the prefix scoped.
                    continue
                if len(inputs) >= MAX_STATES:
                    break
                bindings = {name: replace(state.bindings[name], note=Note(f"`{name}`", span(node)),
                                          definition=None, gates=())
                            for name in sorted(names & state.bindings.keys())}
                inputs.setdefault(tuple(bindings.items()), bindings)

    def suite(self, sources, node, context):
        if node[0] != "block":
            return self.bounded([out for state in sources for out in self.statement(state, node, context)])
        statements = node[3]
        # The enclosing scope resumes after a nested block. Java locals declared
        # inside it cannot replace an enclosing local or parameter.
        locals_ = {item[3] for item in statements if item[0] == "let"}
        invariants = dict(self.shared_inputs)
        for at, item in enumerate(statements):
            if not sources:
                break
            if self.decision_inputs:
                self.capture_inputs(item, sources)
            direct = tuple(self.later(statements[at + 1:]))
            inherited = replace(context, future=(*direct, *context.future)[:MAX_BYPASSED],
                                throw_future=(*direct, *context.throw_future)[:MAX_BYPASSED])
            optional = set() if at + 1 < len(statements) and self.independent_arms(item, sources[0].locals) else None
            sources = self.bounded([out for state in sources for out in self.statement(state, item, inherited, optional)])
            self.invariants_after(invariants, item)
            if optional == {True, False} and sources:
                sources = self.after_join(sources, statements[at + 1], invariants)
        return [state.close(locals_) for state in sources]

    def statement(self, state, node, context, optional=None):
        self.steps -= 1
        if self.steps < 0:
            self.limited = True
            return []
        kind, parts = node[0], node[3:]
        if kind == "empty":
            return [state]
        if kind == "block":
            return self.suite([state], node, context)
        if kind == "if":
            condition, yes, no = parts
            results = []
            for before, holds in self.test(state, condition):
                arm, other = (yes, no) if holds else (no, yes)
                gate = self.guard(condition, holds)
                dominated = (*before.gates, gate)
                entered = replace(before, gates=dominated, arm_depth=before.arm_depth + 1)
                outputs = self.suite([entered], arm, context) if arm else [entered]
                if optional is not None and outputs:
                    optional.add(holds)
                    if arm:
                        for out in outputs:
                            self.observe_arm(before, out, condition, holds, arm)
                gates = dominated if CONTROL["ends_scope"](other) else before.gates
                results.extend(replace(out, gates=gates, arm_depth=before.arm_depth) for out in outputs)
            return self.bounded(results)
        if kind == "let":
            name, expression = parts
            return [out.declare(name, self.defined(value or self.origin(expression), node, name, out.conditions))
                    for out, value in self.evaluate(state, expression)]
        if kind == "expr":
            return [out for out, _ in self.evaluate(state, parts[0])]
        if kind in {"return", "throw"}:
            self.exit(state, node, parts[0], kind, context)
            return []
        if kind == "foreach":
            return self.foreach(state, node, context)
        if kind in {"break", "continue"}:
            if context.iteration and parts[0] is None:
                text = "leaves the current loop" if kind == "break" else "skips the rest of the current iteration"
                self.iteration_endings.append(Ending(state, Note(text, span(node))))
            else:
                self.open_sites.setdefault(f"{kind} with an unresolved target", span(node))
            return []
        if kind == "try":
            body, handlers, final, resources = parts
            if final or resources:
                return self.open_statement(state, node, "try statement with resources or finally")
            normal = self.suite([state], body, replace(context, throw_future=(), caught=True))
            for handler in handlers:
                parameter, catch_body = handler[3:]
                label = self.code(parameter) or "the cited catch"
                condition = self.catch_condition(parameter)
                initial = replace(state.condition(condition), bindings={}, gates=(*state.gates, condition))
                name = parameter[3]
                entered = initial.declare(name, Value(Note(label, span(parameter)), "caught"))
                normal.extend(out.close({name}) for out in self.suite([entered], catch_body, context))
            return self.bounded([replace(out, gates=state.gates) for out in normal])
        if kind == "assert":
            return self.open_statement(state, node, "assert statement")
        if kind == "boundary":
            return self.open_statement(state, node, parts[0])
        return []

    def open_statement(self, state, node, description):
        self.open_sites.setdefault(description, span(node))
        note = Note(f"the cited {description} completes normally", span(node))
        return [replace(state.condition(note), bindings={})]

    def foreach(self, state, node, context):
        parameter, iterable, body = node[3:]
        name = parameter[3]
        sequence = self.code(iterable) or "the cited iterable"
        step = Note(f"the loop binds `{name}` to an element of {sequence}", span(parameter))
        finish = Note(f"the loop over {sequence} completes without returning from `{self.name}`", span(node))
        items = list(descendants(body))
        writes = state.locals if unknown_writes(items) else local_writes(items)
        completed = []
        for before, _ in self.evaluate(state, iterable):
            # A representative iteration can follow earlier writes. Preserve
            # only origins that the loop body cannot replace.
            previous = replace(before, loop_reached=True,
                               bindings={key: value for key, value in before.bindings.items() if key not in writes})
            entered = replace(previous.condition(step), gates=(*previous.gates, step)).declare(name, None)
            next_iteration = replace(context, iteration=True)
            for after in self.suite([entered], body, next_iteration):
                if after.conditions != entered.conditions or after.effects != entered.effects:
                    self.iteration_endings.append(Ending(after, Note("finishes the current iteration", span(body))))
            completed.append(replace(previous.condition(finish), gates=(*previous.gates, finish)))
        return self.bounded(completed)

    def exit(self, state, node, expression, kind, context):
        role = "return expression" if kind == "return" else "throw expression"
        for out, value in self.bounded(self.evaluated_use(state, expression, role)):
            code = self.code(expression)
            origin = (value.note if value and value.kind != "caught" and value.note.text != code
                      and not value.note.references else None)
            if kind == "return":
                outcome = f"returns {code or 'the evaluated value'}" if expression else "returns without a value"
                skipped = context.future
            else:
                verb = "rethrows" if value and value.kind == "caught" else "throws"
                outcome = f"{verb} {code or 'the evaluated exception'}"
                skipped = context.throw_future
            self.endings.append(Ending(out, Note(outcome, span(node)), origin, skipped,
                                       kind == "throw" and context.caught))

    def run(self, bindings=None):
        bindings = bindings or {}
        if bindings:
            items = list(descendants(self.block))
            if not unknown_writes(items):
                writes = local_writes(items)
                self.shared_inputs = {name: value for name, value in bindings.items() if name not in writes}
        completed = self.suite([State(locals=self.parameters, bindings=bindings)], self.block, Context())
        self.endings.extend(Ending(state) for state in completed)

    def lookup_fact(self):
        lookup = CONTROL["filtered_lookup"](self.block)
        if not lookup:
            return None
        loop, fallback, routes = lookup
        element, iterable, _ = loop[3:]
        sequence = self.code(iterable)
        if not sequence:
            return None
        name = f"`{element[3]}`"
        steps = []
        notes = [Note("", span(loop)), Note("", span(fallback))]
        for tests, found in routes:
            guards = []
            for predicate, holds in tests:
                if (any(part[0] in {"set", "mutate", "open"} for part in descendants(predicate))
                        or not (label := self.code(predicate))):
                    return None
                guards.append(f"{label} is {str(holds).lower()}")
                notes.append(Note("", span(predicate)))
            action = f"returns {name}" if found else "skips the rest of the current iteration"
            steps.append(f"When {joined(guards)}, it {action}.")
        text = (f"In `{self.name}`, the loop takes {name} from {sequence} and returns the first element "
                f"that reaches a return arm. {' '.join(steps)} If the loop finishes without returning an "
                "element, the method returns `null`.")
        if len(text) > 750:
            return None
        fact = {"authority": AUTHORITY, "kind": "java-filtered-lookup", "text": text,
                "condition": f"control reaches the loop in `{self.name}`", "sources": self.cite(notes, ()),
                "uncertainty": "Iterator traversal and predicate evaluation can fail. Collection contents and "
                               "which element reaches a return arm remain open."}
        return Proposition(fact, exit_site=(loop[1], loop[2], fallback[1]))

    def lookup_branches(self, linked, premise, binding):
        for lookup, check, setup, yes, no, suffix in CONTROL["lookup_branches"](self.block):
            call = lookup[4]
            reached = self.call_conditions.get(span(call))
            if reached is None or not (source := self.code(call)):
                continue
            actions = [*setup, *yes, *no, *suffix]
            if not 1 <= len(actions) <= 18 or any(self.code(action) is None for action in actions):
                continue
            yes_guard, no_guard = self.guard(check, True), self.guard(check, False)
            if not self.code(check):
                continue
            setup_steps = []
            for action in setup:
                code = self.code(action)
                if action[0] == "assert":
                    setup_steps.append(f"reaches {code}, which is checked when assertions are enabled")
                else:
                    setup_steps.append(f"evaluates {code}")
            setup_steps.append(f"binds `{lookup[3]}` to the result of {source}")
            scope = self.location(setup[0] if setup else lookup, "the lookup setup")
            condition = joined(list(dict.fromkeys([*([premise] if premise else []), scope.text,
                                                   *(note.text for note in reached)])))
            text = (f"In `{self.name}`, the method {', then '.join(setup_steps)}. "
                    f"When {yes_guard.text}, it evaluates {', then '.join(self.code(action) for action in yes)}. "
                    f"When {no_guard.text}, it evaluates {', then '.join(self.code(action) for action in no)}. "
                    f"After the selected arm completes normally, it evaluates "
                    f"{', then '.join(self.code(action) for action in suffix)}.")
            if len(text) > 1400 or len(condition) > 850:
                continue
            proof = [scope, Note(source, span(lookup)), yes_guard, no_guard, *reached,
                     *(Note("", span(action)) for action in actions)]
            fact = {"authority": AUTHORITY, "kind": "java-lookup-branches", "text": text,
                    "condition": condition, "sources": self.cite(proof, linked),
                    "uncertainty": "The cited evaluations can fail. Successful mutation and persistence remain open."}
            if binding:
                fact["binding"] = binding
            yield Proposition(fact, exit_site=(lookup[1], check[1], suffix[0][1]))

    def straight_helper(self, linked, binding):
        statements = CONTROL["straight_method"](self.block)
        if not statements:
            return None
        kinds = [node[0] for node in descendants(self.block)]
        if any(kind in {"and", "or", "choose", "open"} for kind in kinds):
            return None
        # Shorter effect sequences already fit in the ordinary path witness.
        if sum(kind in {"call", "new", "set", "mutate"} for kind in kinds) < 4:
            return None
        steps = []
        for statement in statements:
            kind, parts = statement[0], statement[3:]
            expression = parts[-1]
            code = self.code(expression)
            if expression is not None and not code:
                return None
            if kind == "let":
                name = f"`{parts[0]}`"
                steps.append(f"binds {name} to {code}" if code else f"declares {name}")
            elif kind == "expr":
                steps.append(f"evaluates {code}")
            else:
                steps.append(f"returns {code}" if code else "returns without a value")
        text = f"In `{self.name}`, the method {', then '.join(steps)}."
        if len(text) > 1200:
            return None
        declaration = "this candidate declaration" if binding == "SOURCE_CANDIDATE" else "the retained declaration"
        proof = [Note("", span(statement)) for statement in statements]
        fact = {"authority": AUTHORITY, "kind": "java-straight-helper", "text": text,
                "condition": f"a cited call enters {declaration} and the listed evaluations complete normally",
                "binding": binding, "sources": self.cite(proof, linked),
                "uncertainty": "The cited evaluations can fail. The external effects of the calls remain open."}
        return Proposition(fact, exit_site=span(self.block))

    def outcome_text(self, row, *, brief=False):
        actions = [item.note.text for item in row.state.effects]
        if brief and len(actions) > 3:
            actions = [actions[0], "intervening evaluations", *actions[-2:]]
        elif len(actions) > 6:
            actions = [*actions[:3], "intervening evaluations", *actions[-3:]]
        attempted = "attempts " + ", then ".join(actions) if actions else ""
        text = f"In `{self.name}`, the retained path "
        if attempted:
            text += attempted
            if row.outcome:
                text += " and then "
        if row.outcome:
            text += row.outcome.text
        if row.origin:
            text += f" using the value from {row.origin.text}"
        text += "."
        if row.skipped:
            scope = " in this try block" if row.local_throw else ""
            skipped = [note.text for note in row.skipped]
            text += f" This exit bypasses later {joined(skipped)}{scope}."
        return text

    def facts(self, linked=(), premise=None, binding=None, limit=MAX_FACTS):
        output = []
        rows = [*ordered_endings([*self.endings, *self.iteration_endings]), *self.optional_arms.values()]
        for row in rows:
            state, outcome = row.state, row.outcome
            if not outcome and (not state.effects or len(state.effects) == 1 and not (state.conditions or premise)):
                continue
            guards = [item.text for item in state.conditions]
            condition = joined(list(dict.fromkeys([*([premise] if premise else []),
                                                    *([state.scope.text] if state.scope else []), *guards])))
            if len(condition) > 850:
                self.limited = True
                continue
            uncertainty = "Prior evaluations can fail. Runtime feasibility, dispatch and persisted effects remain open."
            if any("catch for" in guard for guard in guards):
                uncertainty += " Which earlier evaluations inside the try completed remains open."
            if row.local_throw:
                uncertainty += " Which surrounding catch handles the throw remains open."
            if state.loop_reached:
                uncertainty += " Earlier loop iterations and the iteration count remain open."
            effects = [note for effect in state.effects[:3] + state.effects[-3:]
                       for note in (effect.note, *effect.origin)]
            proof = [*([state.scope] if state.scope else []), *state.conditions,
                     *([outcome] if outcome else []), *row.skipped,
                     *([row.origin] if row.origin else []), *effects]
            kind = "java-guarded-outcome" if outcome else "java-guarded-effects" if row.optional_arm else "java-ordered-effects"
            fact = {"authority": AUTHORITY, "kind": kind,
                    "text": self.outcome_text(row), "sources": self.cite(proof, linked), "uncertainty": uncertainty}
            if condition:
                fact["condition"] = condition
            if binding:
                fact["binding"] = binding
            brief = ({**fact, "text": self.outcome_text(row, brief=True)}
                     if len(state.effects) > 4 else None)
            exit_site = outcome.span if outcome else None if row.optional_arm else ()
            output.append(Proposition(fact, brief, exit_site))
            if len(output) == limit - bool(self.open_sites) - 1:
                self.limited |= len(rows) > len(output)
                break
        if self.open_sites and len(output) < limit:
            labels = joined(list(self.open_sites))
            text = f"The retained Java path leaves the behavior inside the cited {labels} open."
            notes = [Note(kind, bounds) for kind, bounds in self.open_sites.items()]
            fact = boundary("CONTROL_OR_EXPRESSION_OPEN", text, self.cite(notes, linked))
            if premise:
                fact.update(condition=premise, binding=binding)
            output.append(Proposition(fact))
        if self.limited and len(output) < limit:
            output.append(Proposition(boundary("PATH_LIMIT", "The retained Java path reached its local projection limit.",
                                              list(dict.fromkeys([*linked, self.source.identifier])))))
        return output

    def catch_condition(self, parameter):
        label = self.code(parameter) or "the cited catch"
        return Note(f"the catch for {label} is selected", span(parameter))

    def observe_predicate(self, state, node, arguments, actuals):
        bounds = span(node)
        if bounds not in self.predicate_uses:
            return
        conditions = tuple(dict.fromkeys([*state.gates, *(note for note in state.conditions
                            if note.text.startswith("the catch for "))]))
        if not conditions and not any(item.role == "guard" for item in state.watching):
            return
        if bounds in self.predicate_calls:
            _, _, previous, previous_conditions = self.predicate_calls[bounds]
            conditions = tuple(note for note in previous_conditions if note in conditions)
            actuals = tuple(current if current == earlier else (self.code(argument), ())
                            for argument, earlier, current in zip(arguments, previous, actuals))
        self.predicate_calls[bounds] = node, arguments, tuple(actuals), conditions

    def predicate_facts(self, snapshot, linked=(), premise=None, binding=None):
        for bounds, (node, arguments, actuals, conditions) in self.predicate_calls.items():
            filename, first, last, formals, steps, nominal, witnesses = self.predicate_uses[bounds]
            call = self.code(node)
            if not call or len(arguments) != len(formals) or len(formals) > 4:
                continue
            notes = [Note(call, bounds), *conditions, *(note for _, proof in actuals for note in proof)]
            mappings = [f"{label} to `{formal}`" for (label, _), formal in zip(actuals, formals) if label]
            if len(mappings) != len(formals):
                continue
            text = f"If {call} enters the retained predicate,"
            if mappings:
                text += f" it binds {joined(mappings)}."
            else:
                text += " it evaluates the cited body."
            if (body := predicate_body(steps)) is None:
                continue
            text += " " + body
            if nominal:
                text += f" The catch argument and formal both declare `{nominal}`."
            condition = joined([*([premise] if premise else []), *(note.text for note in conditions[-3:])])
            if len(text) > 650 or len(condition) > 500:
                continue
            references = [snapshot.cite(filename, low, high) for low, high in [(first, last), *witnesses]]
            if all(references):
                authority = "PINNED_JAVA_NOMINAL_CANDIDATE" if nominal else "PINNED_JAVAC_SELECTED_DECLARATION"
                uncertainty = "Dependency attribution and runtime completion remain open." if nominal else (
                    "The cited argument evaluation and method calls can fail.")
                row = {"kind": "java-local-predicate", "authority": authority, "text": text,
                       "sources": [*self.cite(notes, linked), *references], "uncertainty": uncertainty}
                if condition:
                    row["condition"] = condition
                if binding:
                    row["binding"] = binding
                yield row

    def decisions(self, exact):
        for block, names, premise, local_throw in self.decision_regions:
            check = block[3][0]
            _, inputs = self.decision_inputs[span(check)]
            if not inputs:
                continue
            decision = Body(self.source, (self.name, self.formals, block, self.signature), exact, self.field_uses)
            scope = self.location(check[3], "the decision")
            # Carry the abstract values witnessed at this gate. Earlier
            # normalization conditions, actual arguments and effects remain
            # evidence for the ordinary path, outside this local mapping.
            entry = [State(locals=names, conditions=premise, gates=premise, scope=scope, bindings=bindings)
                     for bindings in inputs.values()]
            decision.suite(entry, block, Context(caught=local_throw))
            if decision.open_sites:
                continue
            seen = set()
            groups = {}
            rows = []
            for row in ordered_endings(decision.endings):
                if row.outcome:
                    state = row.state
                    key = (row.outcome, row.origin, row.skipped, state.gates, state.scope, state.effects)
                    groups.setdefault(key, []).append(row)
            for choices in groups.values():
                row = choices[0]
                state = row.state
                common = [note for note in state.conditions if all(note in item.state.conditions for item in choices)]
                conditions = tuple(dict.fromkeys((*state.gates, *common)))
                condition = joined([scope.text, *dict.fromkeys(note.text for note in conditions)])
                if len(condition) > 460:
                    continue
                text = decision.outcome_text(row, brief=True)
                key = text, condition
                if key in seen:
                    continue
                seen.add(key)
                proof = [scope, *conditions, row.outcome, *row.skipped, *[effect.note for effect in state.effects]]
                uncertainty = "This mapping assumes the cited decision is reached with a retained input state."
                if row.local_throw:
                    uncertainty += " The surrounding catch selection remains open."
                rows.append({"kind": "java-local-decision", "authority": AUTHORITY, "text": text,
                             "condition": condition, "sources": self.cite(proof, ()), "uncertainty": uncertainty})
            if rows:
                yield rows

    def origin_facts(self, linked=(), premise=None, binding=None):
        for (use, read), variants in self.value_uses.items():
            shared = set.intersection(*(set(conditions) for conditions in variants.values()))
            rows = []
            for origin, conditions in variants.items():
                if origin is None:
                    continue
                value, definition = origin
                gates = [note for note in conditions if note not in shared][-2:]
                where = use.role
                proof = [use.expression, read, value, definition, *gates]
                if use.enclosing:
                    proof.append(use.enclosing)
                    where += f" to {use.enclosing.text}" if use.role.startswith("argument ") else f" {use.enclosing.text}"
                if use.role != "assignment" and use.expression.text != read.text:
                    where += f" ({use.expression.text})"
                if definition.references and definition.span == read.span:
                    site = "its static declaration"
                else:
                    line = self.source.line + self.source.text[:definition.span[0]].count(b"\n")
                    site = f"the binding on line {line}"
                line = self.source.line + self.source.text[:read.span[0]].count(b"\n")
                row = {"authority": AUTHORITY, "kind": "java-value-origin",
                       "text": f"In the {where}, {read.text} comes from {value.text} through {site} (read on line {line}).",
                       "sources": self.cite(proof, linked)}
                if len(variants) > 1 and not gates:
                    row["uncertainty"] = "The retained paths contain other reaching values for this read."
                if condition := joined([*([premise] if premise else []), *(note.text for note in gates)]):
                    row["condition"] = condition
                if binding:
                    row["binding"] = binding
                rows.append(row)
            if rows:
                priority = (not any(origin and origin[0].references for origin in variants), use.expression == read)
                yield priority, rows


def predicate_code(fragment):
    if "\n" in fragment or "\r" in fragment:
        # JDK syntax identifies the expression. Keep string contents exact
        # while folding source layout outside ordinary Java quoted literals.
        if any(marker in fragment for marker in ('"""', "/*", "//")):
            return None
        fragment = " ".join(line.strip() for line in fragment.splitlines())
    return compact(fragment, limit=210)


def predicate_body(steps):
    sentences = []
    guarded = False
    for kind, raw in steps:
        text = [predicate_code(fragment) for fragment in raw]
        if None in text:
            return None
        shown = text[0]
        if kind == "L":
            label = "On fallthrough it evaluates" if guarded else "It evaluates"
            sentences.append(f"{label} {shown}.")
        elif kind in {"T", "F"}:
            truth = "true" if kind == "T" else "false"
            label = "Otherwise, if" if guarded else "If"
            sentences.append(f"{label} {shown} is true, it returns `{truth}`.")
            guarded = True
        elif kind == "R":
            label = "On fallthrough it returns" if guarded else "It returns"
            sentences.append(f"{label} {shown}.")
        else:
            truth = "true" if kind == "A" else "false"
            sentences.append(f"It evaluates the right operand only when {shown} is {truth}.")
    return " ".join(sentences)


def source_range(identifier, source, *, checksum):
    file, revision, text = (source.get(key) for key in ("file", "revision", "text"))
    if (source.get("authority") != "EXACT_SNAPSHOT_TEXT" or source.get("textStatus") == "ADDRESS_ONLY"
            or not isinstance(file, str) or not file.endswith(".java")
            or not isinstance(revision, str) or not revision
            or not isinstance(text, str) or not text
            or type(source.get("startLine")) is not int):
        return None
    raw = text.encode()
    if (checksum or "textDigest" in source) and source.get("textDigest") != "sha256:" + sha256(raw).hexdigest():
        return None
    offset = None
    for low, high in (("startByte", "endByte"), ("fragmentStartByte", "fragmentEndByte")):
        first, last = source.get(low), source.get(high)
        if type(first) is type(last) is int and 0 <= first < last and last - first == len(raw):
            offset = first
            break
    end_line = source.get("endLine")
    return Range(identifier, file, revision, source["startLine"], raw, offset,
                 end_line if type(end_line) is int else None)


def author_ranges(process, context):
    by_ref = {}
    private = context.get("_sourceOccurrences", {})
    for site in context.get("sourceRanges", ()):
        for ref in site.get("references", ()):
            source = private.get(ref)
            metadata = process.get("sources", {}).get(ref)
            if not source or not metadata or source.get("peerProcess") or source.get("peerService"):
                continue
            if any(source.get(key) != metadata.get(key) for key in
                   ("file", "revision", "startLine", "endLine", "textDigest", "authority")):
                continue
            if row := source_range(site["id"], source, checksum=True):
                by_ref[ref] = row
    return by_ref


def method_bodies(process, ranges):
    candidates = defaultdict(dict)
    for row in process.get("facts", ()):
        if row.get("kind") not in {"BODY", "BODY_SOURCE"} or not row.get("declaration"):
            continue
        for ref in row.get("sources", ()):
            if source := ranges.get(ref):
                candidates[row["declaration"]][source.identifier] = source
    return {declaration: next(iter(sources.values())) for declaration, sources in candidates.items()
            if len(sources) == 1}


def boundary(status, text, refs):
    return {"authority": AUTHORITY, "kind": "java-scope-boundary", "status": status,
            "text": text, "sources": list(refs)}


def parse_batch(sources, *, timeout=8):
    java = shutil.which("java")
    if not java:
        return {}, "JDK_UNAVAILABLE"
    script = Path(__file__).with_name("clew_prepared_java_syntax.java")
    data = b"\n".join(b64encode(source.text) for source in sources.values()) + b"\n"
    try:
        result = subprocess.run([java, "--source", "21", str(script)], input=data,
                                capture_output=True, timeout=timeout, check=True)
        lines = result.stdout.splitlines()
        if len(lines) == len(sources):
            return {key: parsed for key, line in zip(sources, lines) if (parsed := json.loads(line))}, None
    except subprocess.TimeoutExpired:
        return {}, "PARSER_TIMEOUT"
    except (OSError, subprocess.SubprocessError, ValueError):
        pass
    return {}, "PARSER_UNAVAILABLE"


@dataclass(frozen=True)
class Link:
    target: str
    body: Range
    call: Range
    binding: str


def author_links(process, ranges, bodies, root):
    events = {row.get("occurrence"): row for row in process.get("facts", ())
              if row.get("declaration") == root and row.get("event") == "CALL"
              and row.get("resolution") == "COMPILER_EXACT"}
    for relation in process.get("facts", ()):
        caller, target, event_id = (relation.get(key) for key in ("declaration", "targetMethodId", "eventId"))
        if (caller != root or target == root or target not in bodies
                or not event_id or relation.get("resolution") not in SAME_SCOPE):
            continue
        event = events.get(event_id) or events.get(str(event_id).removeprefix("EVENT:"))
        if not event or not event.get("target") or event["target"] != relation.get("target"):
            continue
        for ref in event.get("sources", ()):
            if call := ranges.get(ref):
                yield Link(target, bodies[target], call, "COMPILER_SELECTED_DECLARATION")


def native_links(root, native, ranges):
    """Use only source IDs admitted by replay of the sealed native projection."""
    if native.get("root") != root:
        return
    calls = defaultdict(list)
    for outcome in native.get("outcomes", ()):
        caller = outcome.get("caller") or {}
        if caller.get("binding") == "FIXED" and caller.get("calleeBody") and caller.get("callSource") in ranges:
            calls[caller["calleeBody"]].append(caller["callSource"])
    for row in native.get("calleeBodies", ()):
        target = row.get("declaration")
        source = ranges.get(row.get("source"))
        if not target or target == root or not source or len(source.text) > MAX_BODY_BYTES:
            continue
        binding = row.get("binding")
        if binding == "SOURCE_CANDIDATE" and (call := ranges.get(row.get("callSource"))):
            yield Link(target, source, call, "SOURCE_CANDIDATE")
        elif binding == "FIXED":
            for identifier in dict.fromkeys(calls[target]):
                yield Link(target, source, ranges[identifier], "COMPILER_SELECTED_DECLARATION")


def linked_premise(link):
    call = compact(link.call.text.decode()) or "the cited call"
    if link.binding == "SOURCE_CANDIDATE":
        return f"control reaches {call} and dispatch enters this candidate body"
    return f"control reaches {call} and dispatch enters the retained declaration"


def collect_links(process, retained, bodies, root, root_body, native, native_ranges):
    seen = set()
    for link in (*author_links(process, retained, bodies, root), *native_links(root, native, native_ranges)):
        if link.body.revision != root_body.revision or link.call.revision != root_body.revision:
            continue
        if link.call.file != root_body.file:
            continue
        identity = (link.target, link.body.file, link.body.text, link.call.text, link.call.line, link.call.offset)
        if identity in seen:
            continue
        seen.add(identity)
        yield link


def invocation_span(root, call, invocations):
    if root.offset is not None and call.offset is not None:
        where = root.local_span(call)
        return where if where in invocations else None
    if (root.file, root.revision) != (call.file, call.revision) or call.end_line is None:
        return None

    # Older replayed ranges carry the exact excerpt and its line interval.
    # Search only parsed invocation nodes and require one matching occurrence.
    matches = []
    for start, end in invocations:
        if end - start != len(call.text) or root.text[start:end] != call.text:
            continue
        first = root.line + root.text.count(b"\n", 0, start)
        last = root.line + root.text.count(b"\n", 0, end - 1)
        if (first, last) == (call.line, call.end_line):
            matches.append((start, end))
    return matches[0] if len(matches) == 1 else None


def literal_inputs(entry, target, link, invocation):
    """Return positional constants, or None when invocation binding is open."""
    if not invocation or target.signature != "FIXED" or len(target.formals) != len(target.parameters):
        return None
    select, actuals = invocation[3:]
    if select[0] not in {"id", "member"} or select[-1] != target.name or len(actuals) != len(target.formals):
        return None
    bindings = {}
    for argument, parameter in zip(actuals, target.formals):
        name, declared_type = parameter[3:]
        if argument[0] != "literal":
            continue
        constant = argument[3]
        compatible = (
            constant in {"TRUE", "FALSE"} and declared_type == "boolean"
            or constant in {"EMPTY_STRING", "NONEMPTY_STRING"} and declared_type in {"String", "java.lang.String"}
            or constant == "NULL" and declared_type and declared_type not in PRIMITIVES
        )
        if not compatible or not (literal := entry.code(argument)):
            if link.binding == "SOURCE_CANDIDATE":
                return None
            continue
        reference = (link.call.identifier,)
        note = Note(literal, span(parameter), reference)
        declaration = target.code(parameter) or f"the declaration of `{name}`"
        bindings[name] = Value(note, "literal", constant, Note(declaration, span(parameter), reference))
    return bindings


def attribute(snapshot, sources, parsed):
    if not snapshot or not parsed:
        return {}, {}
    values = runpy.run_path(str(Path(__file__).with_name("clew_prepared_java_values.py")))
    qualifiers = {identifier: [name for node in descendants(syntax[2]) if node[0] == "member"
                               if (name := values["names"](node[3]))]
                  for identifier, syntax in parsed.items()}
    return values["attributed_bindings"](snapshot, {identifier: sources[identifier] for identifier in parsed}, qualifiers)


def derive(process, context, native=None, native_ranges=None, snapshot=None, *, prepared=None, declaration_only=False):
    root = process.get("root")
    if not root:
        return []
    retained = author_ranges(process, context)
    bodies = method_bodies(process, retained)
    root_body = bodies.get(root)
    if not root_body:
        return []
    if len(root_body.text) > MAX_BODY_BYTES:
        return [boundary("BODY_LIMIT", "The retained Java method exceeds the syntax projection size limit.",
                         [root_body.identifier])]
    native_exact = {identifier: found for identifier, source in (native_ranges or {}).items()
                    if (found := source_range(identifier, source, checksum=False))}
    links = list(collect_links(process, retained, bodies, root, root_body, native or {}, native_exact))
    candidates = [link for link in links if len(link.body.text) <= MAX_BODY_BYTES][:MAX_BODIES - 1]
    sources = {root_body.identifier: root_body, **{link.body.identifier: link.body for link in candidates}}
    if prepared is None:
        parsed, failure = parse_batch(sources, timeout=snapshot.remaining(8)) if hasattr(snapshot, "remaining") else parse_batch(sources)
        if failure:
            return [boundary(failure, "The retained Java method could not be projected with the local JDK syntax parser.",
                             [root_body.identifier])]
        fields, predicates = attribute(snapshot, sources, parsed)
    else:
        parsed, fields, predicates = prepared
    exact = set(retained.values())

    def project(identifier):
        if syntax := parsed.get(identifier):
            return Body(sources[identifier], syntax, exact, fields.get(identifier), predicates.get(identifier),
                        capture_decisions=identifier == root_body.identifier)
        return None

    entry = project(root_body.identifier)
    invocations = {}
    if entry:
        entry.run()
        root_facts = entry.facts()
        invocations = {span(node): node for node in descendants(entry.block) if node[0] == "call"}
        if lookup := entry.lookup_fact():
            root_facts.append(lookup)
        if declaration_only:
            root_facts.extend(islice(entry.lookup_branches((), None, None), 2))
    else:
        root_facts = [Proposition(boundary("BODY_SYNTAX_OPEN",
                                         "The retained Java excerpt does not establish a supported method body.",
                                         [root_body.identifier]))]
    truncated = len(links) > len(candidates) or (entry is not None and entry.limited)
    linked_facts = defaultdict(list)
    straight_helpers = {}
    origins = [*entry.origin_facts()] if entry else []
    seen_origins = set()
    predicate_groups = [entry.predicate_facts(snapshot)] if snapshot and entry else []
    for link in candidates:
        where = invocation_span(root_body, link.call, invocations)
        premise = linked_premise(link)
        if link.call.identifier in native_exact:
            # Native replay validates the source relationship separately from lexical flow.
            trail = (link.call.identifier,)
        elif entry:
            conditions = entry.call_conditions.get(where)
            if conditions is None:
                continue
            guards = [note.text for note in conditions[:3]]
            premise = joined([*guards, premise])
            trail = (root_body.identifier, link.call.identifier)
        else:
            continue
        binding = link.binding
        if target := project(link.body.identifier):
            bindings = literal_inputs(entry, target, link, invocations.get(where))
            is_candidate = binding == "SOURCE_CANDIDATE"
            if where is not None and (not is_candidate or bindings is not None):
                _, call_sites = straight_helpers.setdefault((link.body.identifier, binding), (target, []))
                if link.call.identifier not in call_sites:
                    call_sites.append(link.call.identifier)
            lookup = target.lookup_fact() if is_candidate or not bindings else None
            if lookup:
                linked_facts.setdefault(("lookup", link.body.identifier), [lookup])
            if is_candidate and bindings is None:
                # Publish declaration-local behavior when invocation binding is open.
                if lookup:
                    continue
                trail, premise, binding = (), None, None
            elif bindings:
                mapped = [f"`{name}` receives {value.note.text}" for name, value in bindings.items()]
                premise = joined([premise, *mapped])
            target.run(bindings)
            branch_trail, branch_premise = trail, premise
            if binding and link.call.identifier in native_exact and entry:
                guards = entry.call_conditions.get(where, ())[:3]
                if guards:
                    branch_trail = (*trail, root_body.identifier)
                    branch_premise = joined([*(note.text for note in guards), premise])
            branch_key = ("lookup-branches", link.body.identifier, link.call.identifier if binding else None)
            branches = list(islice(target.lookup_branches(branch_trail, branch_premise, binding), 2))
            if branches:
                linked_facts.setdefault(branch_key, branches)
            body_facts = linked_facts[link.body.identifier, binding]
            origin_key = link.call.identifier if bindings else (link.body.identifier, binding)
            if origin_key not in seen_origins:
                origins.extend(target.origin_facts(trail, premise, binding))
                if snapshot:
                    predicate_groups.append(target.predicate_facts(snapshot, trail, premise, binding))
                seen_origins.add(origin_key)
            if binding or not body_facts:
                body_facts.extend(target.facts(trail, premise, binding, limit=4))
            truncated |= target.limited
        else:
            linked_facts[link.body.identifier, binding].append(Proposition({
                **boundary("BODY_SYNTAX_OPEN", "The cited collaborator excerpt leaves its method syntax open.",
                           [*trail, link.body.identifier]), "condition": premise, "binding": binding,
            }))
    decisions = bounded_decisions(entry.decisions(set(retained.values()))) if entry else []
    predicate_facts = list(islice(interleave(predicate_groups), MAX_PREDICATE_FACTS)) if predicate_groups else []
    summaries = [[summary] for (_, binding), (body, calls) in straight_helpers.items()
                 if (summary := body.straight_helper(tuple(calls), binding))]
    return [*bounded_facts(root_facts, [*summaries, *linked_facts.values()], root_body.identifier, truncated),
            *decisions, *predicate_facts, *bounded_origins(origins)]


def partition_choices(root, linked):
    # Allocate one witness per root exit and retained collaborator before
    # spending the byte budget on another variant of the same source path.
    root_first, root_more, sites = [], [], set()
    for choice in root:
        if choice.exit_site not in sites:
            root_first.append(choice)
            sites.add(choice.exit_site)
        else:
            root_more.append(choice)
    helpers = [choices for choices in linked if choices]
    primary = [*root_first[:2], *(choices[0] for choices in helpers), *root_first[2:]]
    alternatives = interleave([root_more, *(choices[1:] for choices in helpers)])
    return primary, alternatives


def encoded_size(row):
    return len(json.dumps(row, ensure_ascii=False, separators=(",", ":")).encode()) + 1


def bounded_facts(root, linked, root_id, truncated):
    notice = boundary("PROJECTION_LIMIT", "The retained Java method projection omitted further paths or bodies.",
                      [root_id])
    primary, alternatives = partition_choices(root, linked)
    selected = {}
    remaining = MAX_OUTPUT_BYTES - 2 - encoded_size(notice)
    def allot(choices):
        nonlocal remaining, truncated
        for choice in choices:
            brief = choice.brief or choice.detail
            size = encoded_size(brief)
            if size > remaining or len(selected) >= MAX_FACTS - 1:
                truncated = True
                continue
            selected[choice] = brief
            remaining -= size

    allot(primary)
    for choice in primary:
        previous = selected.get(choice)
        if previous is None:
            continue
        cost = encoded_size(choice.detail) - encoded_size(previous)
        if cost <= remaining:
            selected[choice] = choice.detail
            remaining -= cost
    allot(alternatives)
    ordered = [*root, *(choice for group in linked for choice in group)]
    output = [selected[choice] for choice in ordered if choice in selected]
    if truncated and not any(row.get("status") == "PATH_LIMIT" for row in output):
        output.append(notice)
    return output



def bounded_origins(groups):
    rows = []
    remaining = MAX_VALUE_BYTES
    for _, group in sorted(groups, key=lambda item: item[0]):
        cost = sum(encoded_size(row) for row in group)
        if cost <= remaining and len(rows) + len(group) <= MAX_VALUE_FACTS:
            rows.extend(group)
            remaining -= cost
    return rows



def bounded_decisions(groups):
    rows = []
    remaining = MAX_DECISION_BYTES
    for row in interleave(groups):
        size = encoded_size(row)
        if len(rows) == MAX_DECISION_FACTS:
            break
        if size <= remaining:
            rows.append(row)
            remaining -= size
    return rows
