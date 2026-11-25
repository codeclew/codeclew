"""Recognize bounded Kotlin standard-library guard syntax."""

from dataclasses import dataclass
import re


EXCEPTIONS = {"error": "IllegalStateException", "check": "IllegalStateException", "require": "IllegalArgumentException"}
STDLIB = re.compile(
    r"(?:callable:kotlin/(?:PreconditionsKt(?:__\w+Kt)?\.|StandardKt(?:__\w+Kt)?\.)?|"
    r"method:class:kotlin\.(?:PreconditionsKt(?:__\w+Kt)?|StandardKt(?:__\w+Kt)?)#)"
    r"(error|require|check|takeIf)(?:#jvm:|\()"
)


@dataclass(frozen=True)
class HardGuard:
    failure: object
    proof: tuple
    bindings: tuple
    exception: str
    true: str = ""
    false: str = ""
    predicate: object = None

    @property
    def verified(self):
        return not self.bindings


def shadowed(body, names, invocation):
    if any(param.name in names for param in body.function.params):
        return True
    for site, (_, callback, _) in body.callback_scopes.items():
        if site.start <= invocation.selector.start < invocation.selector.end <= site.end:
            if any(name in names for name, _ in callback.params):
                return True
    words = body.unit.words
    for index, word in enumerate(words[:-1]):
        if word in {"val", "var", "class", "object"} and words[index + 1] in names:
            return True
        if word == "fun":
            # Include extension functions declared by the retained source.
            stop = body.unit.until(index + 1, len(body.unit.tokens), {"(", "=", "{", "}"})
            if stop > index + 1 and words[stop - 1] in names:
                return True
    return any(body.unit.class_names.get(name) or body.unit.aliases.get(name) for name in names)


def binding(body, invocation):
    member = invocation.member
    candidates = []
    for at, target in body.compiler_targets:
        if (at == invocation.site or at.start == invocation.selector.start and at.end <= invocation.selector.end):
            if re.search(r"[.#/]" + re.escape(member) + r"(?:#jvm:|\()", target):
                candidates.append((at, target))
    if candidates:
        if any(not (match := STDLIB.match(target)) or match[1] != member for _, target in candidates):
            return None
        calls = [body.text_note(at, "compiler-selected Kotlin call") for at, _ in candidates]
        return (), tuple(dict.fromkeys(calls))
    receiver = body.unit.excerpt(invocation.receiver) if invocation.receiver else ""
    if member == "takeIf":
        if not receiver or shadowed(body, {member}, invocation):
            return None
        return (member,), ()
    if receiver and receiver != "kotlin":
        return None
    names = {"kotlin"} if receiver else {member}
    if shadowed(body, names, invocation):
        return None
    if receiver == "kotlin" and not body.unit.imports.get("kotlin"):
        return (), ()
    imports = body.unit.imports.get(member, set())
    if imports:
        return ((), ()) if imports == {f"kotlin.{member}"} else None
    return (member,), ()


def argument(body, part):
    unit = body.unit
    at = unit.token_indices[part.selector.start] + 1
    if unit.words[at] != "(":
        return None
    end = unit.pairs[at]
    stop = next((index for index in range(at + 1, end)
                 if unit.words[index] == "," and unit.container[index] == at), end)
    start = at + 1 + (2 if unit.words[at + 2] == "=" else 0)
    if start < stop:
        return body.note(unit.site(start, stop))
    return None


def predicate(body, part):
    if len(part.callbacks) != 1:
        return None
    site = part.callbacks[0]
    return body.note(site)


def derive(body, statement):
    if statement.kind not in {"expression", "return"} or not statement.expression:
        return None
    unit = body.unit
    first = unit.token_indices[statement.expression[0].start]
    stop = first + len(statement.expression)
    container = unit.container[first]
    operators = [(at, unit.words[at]) for at in range(first, stop)
                 if unit.words[at] in {"=", "?:"} and unit.container[at] == container]
    pivots = [at for at, word in operators if word == "?:"]
    assignments = [at for at, word in operators if word == "="]
    if len(pivots) > 1 or len(assignments) > 1:
        return None
    start = assignments[0] + 1 if assignments else first
    pivot = pivots[0] if pivots else None
    if pivot is not None and pivot <= start:
        return None
    callee_at = pivot + 1 if pivot is not None else start
    parts = body.invocation_stages(callee_at, stop)
    if len(parts) != 1 or parts[0].member not in EXCEPTIONS:
        return None
    part, = parts
    if not (resolved := binding(body, part)):
        return None
    bindings, evidence = resolved
    call = body.note(part.site) or body.text_note(part.site, body.describe_call(part))
    exception = EXCEPTIONS[part.member]
    if part.member == "error" and part.arity != 1 or part.member != "error" and part.arity not in {1, 2}:
        return None
    if pivot is None:
        if part.member == "error":
            return HardGuard(call, (*evidence, call), bindings, exception)
        if assignments or statement.kind != "expression" or not (condition := argument(body, part)):
            return None
        return HardGuard(call, (*evidence, condition, call), bindings, exception,
                         condition.text + " is true", condition.text + " is false", condition)
    if part.member != "error":
        return None
    receiver = body.note(unit.site(start, pivot))
    if not receiver:
        return None
    proof = [receiver, *evidence, call]
    passes, fails = receiver.text + " is non-null", receiver.text + " is null"
    antecedents = body.invocation_stages(start, pivot)
    if antecedents and (filter_ := antecedents[-1]).member == "takeIf" and (resolved := binding(body, filter_)):
        if (test := predicate(body, filter_)) and (source := body.receiver_note(filter_.receiver)):
            member = unit.token_indices[filter_.selector.start]
            if unit.words[member - 1] == "?.":
                additional, target = resolved
                bindings = (*bindings, *additional)
                proof.extend((source, test, *target))
                passes = f"{source.text} is non-null and the `takeIf` predicate {test.text} accepts it"
                fails = f"{source.text} is null or the `takeIf` predicate {test.text} rejects it"
    return HardGuard(call, tuple(dict.fromkeys(proof)), tuple(dict.fromkeys(bindings)), exception, passes, fails)
