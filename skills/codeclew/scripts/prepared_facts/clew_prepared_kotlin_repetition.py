"""Project one bounded, result-controlled local continuation from retained Kotlin syntax."""

from dataclasses import dataclass, replace
import re


MAX_STEPS = 6
MAX_STATEMENTS = 96
MAX_LOOPS = 4
COMPARISONS = {"==", "!=", "===", "!==", "<", "<=", ">", ">="}
LITERAL = re.compile(r'-?\d[\d_]*(?:[lL])?|"[^"$]*"|true|false|null')


@dataclass(frozen=True)
class Step:
    site: object
    catches: tuple = ()
    outcome: bool = True


@dataclass(frozen=True)
class Continuation:
    site: object
    text: str
    passes_lambda: bool = False


@dataclass(frozen=True)
class Local:
    site: object
    initializer: tuple = ()
    mutable: bool = False


@dataclass(frozen=True)
class Fact:
    kind: str
    text: str
    sites: tuple
    condition: str | None = None


@dataclass(frozen=True)
class Validation:
    guard: str
    passes: str
    rejects: str
    failure: str
    sites: tuple
    assumption: str = ""


@dataclass(frozen=True)
class Loop:
    supplier: Step
    comparison: object
    actions: tuple
    position: str
    continue_if: bool
    stop: object = None
    binding: object = None
    prefix: tuple = ()
    result: str | None = None
    suite: tuple = ()


class Projection:
    def __init__(self, body, syntax, controls, external_sites):
        self.body = body
        self.unit = body.unit
        self.syntax = syntax
        self.span = syntax["token_site"]
        self.words = self.unit.words
        self.parameters = {parameter.name: parameter for parameter in body.function.params}
        self.controls = controls(body, '', ())
        self.external_sites = external_sites
        self.locals = {}

    def checks_input(self, tokens):
        """Accept only visible inputs to the leading check."""
        if any(token.text in {"{", "->", "::"} for token in tokens):
            return False
        return any((token.text in self.parameters or token.text in self.locals)
                   and (not index or tokens[index - 1].text not in {".", "?.", "@"})
                   and (index + 1 == len(tokens) or tokens[index + 1].text != "=")
                   or token.text == "this" and index + 2 < len(tokens) and tokens[index + 1].text == "."
                   for index, token in enumerate(tokens))

    def validation(self, statement):
        hard = self.body.hard_guard(statement)
        if hard and hard.predicate and self.checks_input(self.tokens(hard.predicate.site)):
            assumption = ("the selectors resolve to " + " and ".join(f"`kotlin.{name}`" for name in hard.bindings)
                          if hard.bindings else "")
            return Validation(hard.failure.text, hard.true, hard.false, f"throws `{hard.exception}`",
                              tuple(note.site for note in hard.proof), assumption)
        if (statement.kind == "if" and not statement.alternative and len(statement.body) == 1
                and statement.body[0].kind == "throw" and self.checks_input(statement.expression)):
            site, exit_ = self.span(statement.expression), statement.body[0].site
            if (predicate := self.literal(site)) and (terminal := self.literal(exit_)):
                return Validation(f"the check {predicate}", f"{predicate} is false", f"{predicate} is true",
                                  f"reaches {terminal}", (site, exit_))
        return None

    def literal(self, site):
        note = self.body.note(site)
        return note.text if note else None

    def tokens(self, site):
        return self.unit.tokens[self.unit.token_indices[site.start]:self.unit.token_ends[site.end] + 1]

    def call(self, tokens):
        if not tokens:
            return None
        first = self.unit.token_indices[tokens[0].start]
        last = first + len(tokens)
        parts = self.body.invocation_stages(first, last)
        if (parts and parts[-1].site == self.span(tokens) and all(part.member and not part.callbacks for part in parts)
                and not any(token.text in {"{", "->", "?.", "return", "throw", "break", "continue", "::"} for token in tokens)):
            return Step(parts[-1].site)
        return None

    def trailing_lambda(self, tokens, start):
        """Keep the call header and its lambda boundary without replaying the argument body."""
        if not tokens or tokens[-1].text != "}":
            return None
        first = self.unit.token_indices[tokens[0].start]
        stop = first + len(tokens)
        parts = self.body.invocation_stages(first, stop)
        if (not parts or parts[-1].site != self.span(tokens) or len(parts[-1].callbacks) != 1
                or any(not part.member or part.callbacks for part in parts[:-1]) or not parts[-1].member):
            return None
        callback, = parts[-1].callbacks
        if callback.end != parts[-1].site.end:
            return None
        opening = self.unit.token_indices[callback.start]
        prefix = self.unit.tokens[first:opening]
        if not prefix or any(token.text in {"{", "->", "return", "throw", "break", "continue", "::"} for token in prefix):
            return None
        start_ = self.unit.token_indices[start]
        if not (label := self.literal(self.unit.site(start_, opening))):
            return None
        selector = self.unit.token_indices[parts[-1].selector.start]
        safe = selector > first and self.words[selector - 1] == "?."
        if self.words[opening] != "{":
            opening += 2  # The parser already verified a trailing lambda label.
        header = self.unit.site(start_, opening + 1)
        condition = ("when the safe-call receiver checks pass and call arguments evaluate normally" if safe
                     else "when the receiver and arguments evaluate normally")
        return Continuation(header, f"{label} with a trailing lambda argument {condition}", passes_lambda=True)

    def action(self, statement, *, binding=False, locals_=None):
        if statement.kind == "expression":
            if operation := self.call(statement.expression):
                return operation
            if not binding and self.accumulation(statement, locals_ or {}):
                return Step(statement.site)
            return None
        if statement.kind != "try" or len(statement.body) != 1 or not statement.handlers or statement.cleanup:
            return None
        operation = self.action(statement.body[0], binding=binding, locals_=locals_)
        if operation is None or operation.catches:
            return None
        variants = []
        for _, handler, suite in statement.handlers:
            if not suite and not binding:
                variants.append((handler, "resume", None))
            elif (len(suite) == 1 and suite[0].kind == "continue" and not suite[0].expression):
                variants.append((handler, "continue", suite[0].site))
            elif len(suite) == 1 and suite[0].kind == "throw":
                variants.append((handler, "throw", suite[0].site))
            else:
                return None
        return Step(operation.site, tuple(variants))

    def declaration(self, statement):
        tokens = statement.expression
        words = self.syntax["strings"](tokens)
        if statement.kind != "expression" or len(words) < 3 or words[0] not in {"val", "var"} or not self.syntax["name"](words[1]):
            return None
        first = self.unit.token_indices[tokens[0].start]
        direct = self.unit.container[first]
        split = next((index for index, word in enumerate(words[2:], 2) if word == "="
                      and self.unit.container[first + index] == direct), len(words))
        if split == 2 or words[2] == ":" and split > 3:
            return words[1], Local(statement.site, tokens[split + 1:], words[0] == "var")
        return None

    def assignment(self, statement):
        if local := self.declaration(statement):
            name, declaration = local
            return (name, declaration.initializer) if declaration.initializer else None
        tokens = statement.expression
        words = self.syntax["strings"](tokens)
        if statement.kind == "expression" and len(words) > 2 and words[1] == "=" and (local := self.locals.get(words[0])) and local.mutable:
            return words[0], tokens[2:]
        return None

    def supplier(self, tokens):
        if operation := self.call(tokens):
            return operation
        if tokens and tokens[0].text == "try":
            first = self.unit.token_indices[tokens[0].start]
            statement, stop = self.unit.statement(first, first + len(tokens), 0)
            if stop == first + len(tokens):
                return self.action(statement, binding=True)
        if self.expected(tokens) and (tokens[0].text in self.locals or tokens[0].text in self.parameters
                                      or tokens[0].text == "this" and len(tokens) >= 3):
            return Step(self.span(tokens), outcome=False)
        return None

    def accumulation(self, statement, locals_):
        if statement.kind != "expression":
            return None
        tokens = statement.expression
        words = self.syntax["strings"](tokens)
        if len(words) > 2 and (local := locals_.get(words[0])) and local.initializer:
            if local.mutable and words[1] == "+=":
                return words[0], local, tokens[2:]
            if local.mutable and len(words) > 4 and words[1:4] == ("=", words[0], "+"):
                return words[0], local, tokens[4:]
            if words[1:3] in {(".", "add"), (".", "addAll")}:
                initial = "".join(self.syntax["strings"](local.initializer)).split("(", 1)[0].split("<", 1)[0]
                if initial in {"kotlin.collections.mutableListOf", "kotlin.collections.arrayListOf"}:
                    if invocation := self.syntax["invocation"](tokens):
                        _, args = invocation
                        if len(args) == 1:
                            return words[0], local, args[0]
        return None

    def has_continue(self, steps):
        return any(mode == "continue" for step in steps for _, mode, _ in step.catches)

    def comparison_parts(self, tokens):
        first = self.unit.token_indices[tokens[0].start] if tokens else None
        if first is None:
            return None
        stop = first + len(tokens)
        while stop - first >= 3 and self.words[first] == "(" and self.unit.pairs.get(first) == stop - 1:
            first, stop = first + 1, stop - 1
        common = self.unit.container[first]
        operators = [index for index in range(first, stop) if self.unit.container[index] == common
                     and self.words[index] in COMPARISONS]
        if len(operators) != 1:
            return None
        middle, = operators
        return tuple(self.unit.tokens[first:middle]), tuple(self.unit.tokens[middle + 1:stop])

    def expected(self, tokens):
        words = self.syntax["strings"](tokens)
        if len(words) == 1 and LITERAL.fullmatch(words[0]):
            return True
        if len(words) == 2 and words[0] == "-" and words[1].isdigit():
            return True
        return (bool(words) and len(words) % 2 == 1 and all(self.syntax["name"](word) or index == 0 and word == "this"
                        for index, word in enumerate(words[::2]))
                and all(word == "." for word in words[1::2]))

    def result_test(self, tokens, name=None):
        if not (sides := self.comparison_parts(tokens)):
            return None
        matches = []
        for supplied, expected in (sides, sides[::-1]):
            if not self.expected(expected) or name and self.syntax["strings"](expected) == (name,):
                continue
            if name is None:
                if supplier := self.call(supplied):
                    matches.append((supplier, self.span(tokens)))
            elif supplied and supplied[0].text == name and self.expected(supplied):
                matches.append((None, self.span(tokens)))
        return matches[0] if len(matches) == 1 else None

    def thrown_arm(self, statement):
        if (statement.kind == "if" and not statement.alternative and len(statement.body) == 1
                and statement.body[0].kind == "throw" and self.syntax["strings"](statement.expression) not in {("true",), ("false",)}):
            return self.span(statement.expression), statement.body[0].site
        return None

    def actions(self, statements):
        if len(statements) > MAX_STEPS:
            return None
        result = []
        for statement in statements:
            if self.thrown_arm(statement) or (statement.kind == "if" and not statement.alternative
                                              and self.syntax["strings"](statement.expression) == ("false",)):
                continue
            if not (step := self.action(statement, locals_=self.locals)):
                return None
            result.append(step)
        return tuple(result)

    def reassignments(self, statements):
        changed = set()
        for statement in statements:
            words = self.syntax["strings"](self.tokens(statement.site))
            for word, following in zip(words, words[1:]):
                if following in {"=", "+=", "-=", "*=", "/=", "%=", "++", "--"} and self.syntax["name"](word):
                    changed.add(word)
                if word in {"val", "var", "++", "--"} and self.syntax["name"](following):
                    changed.add(following)
        return changed

    def resumable(self, statements):
        return self.controls.shape(statements, "CALLABLE").resumes

    def reached_steps(self, statements):
        for statement in statements:
            yield statement
            if not self.controls.statement_shape(statement, "CALLABLE").resumes:
                return

    def top_test(self, loop):
        if result := self.result_test(loop.expression):
            if not self.resumable(loop.body):
                return None
            supplier, comparison = result
            return Loop(supplier, comparison, self.actions(loop.body) or (), "header", True, suite=loop.body)
        if (self.syntax["strings"](loop.expression) != ("true",) or len(loop.body) < 2
                or not (binding := self.assignment(loop.body[0]))):
            return None
        name, tokens = binding
        gate = loop.body[1]
        if (gate.kind != "if" or gate.alternative or len(gate.body) != 1
                or gate.body[0].kind != "break" or gate.body[0].expression):
            return None
        if (supplier := self.supplier(tokens)) and (result := self.result_test(gate.expression, name)) and self.resumable(loop.body[2:]):
            return Loop(supplier, result[1], self.actions(loop.body[2:]) or (), "body", False,
                        gate.body[0].site, loop.body[0].site, result=name, suite=loop.body[2:])
        return None

    def tail_test(self, start, end):
        suite, at = self.unit.block(start + 1, end, 0)
        if self.words[at:at + 2] != ("while", "("):
            return None, at
        close = self.unit.pairs[at + 1]
        if not suite or len(suite) > MAX_STEPS + 1:
            return None, close + 1
        predicate = self.unit.tokens[at + 2:close]
        for index, statement in enumerate(suite):
            if not (binding := self.assignment(statement)):
                continue
            name, tokens = binding
            if not ((supplier := self.supplier(tokens)) and (result := self.result_test(predicate, name))):
                continue
            # A bottom test can only be tied to this iteration's value when the prefix reaches its binding.
            before, after = self.actions(suite[:index]), self.actions(suite[index + 1:])
            if (before is None or self.has_continue(before) or self.has_continue((supplier,))
                    or name in self.reassignments(suite[index + 1:]) or not self.resumable(suite[index + 1:])):
                continue
            return Loop(supplier, result[1], after or (), "tail", True, binding=statement.site,
                        prefix=before, result=name, suite=(*suite[:index], *suite[index + 1:])), close + 1
        return None, close + 1

    def fact(self, kind, text, sites, condition=None):
        return Fact(kind, text, tuple(dict.fromkeys(site for site in sites if site)), condition)

    def core(self, loop, suffix):
        supplier, comparison = self.literal(loop.supplier.site), self.literal(loop.comparison)
        if not supplier or not comparison:
            return None
        phases = [self.literal(step.site) for step in loop.actions]
        before = [self.literal(step.site) for step in loop.prefix]
        after = [step.text for step in suffix if not step.passes_lambda]
        stop = self.literal(loop.stop) if loop.stop else None
        if not all([*phases, *before, *after]) or loop.stop and not stop:
            return None
        origin = f"evaluates {supplier}" if loop.supplier.outcome else f"reads {supplier}"
        prefix = ("the normal iteration reaches " + " then ".join(before) + " before it " if before else "source ")
        parts = [f"If control enters this loop, {prefix}{origin} for the selected result."]
        actions = " then ".join(phases)
        if loop.position == "tail":
            if actions:
                parts.append(f"On normal continuation, {actions} follows the result before the bottom test.")
            parts.append(f"When the bottom comparison {comparison} is reached, source starts another iteration if it evaluates true.")
        elif loop.continue_if:
            continuation = f" source reaches {actions} on normal continuation before testing again." if actions else " source can enter the body."
            parts.append(f"The header comparison is {comparison}. If true,{continuation}")
        else:
            parts.append(f"If {comparison} is true, {stop} exits the loop.")
            continuation = f" source reaches {actions} before the next iteration on the normal path." if actions else " source can reach the next iteration."
            parts.append("If the comparison is false," + continuation)
        if loop.continue_if:
            parts.append(f"If {comparison} evaluates false, the loop exits normally.")
        if after:
            parts.append("The normal after-loop continuation reaches " + " then ".join(after) + ".")
        if suffix and suffix[-1].passes_lambda:
            opening = ("If that sequence completes normally, source then reaches " if after
                       else "The normal after-loop continuation reaches ")
            parts.extend((opening + suffix[-1].text + ".", "Whether the lambda runs depends on the callee."))
        sites = [loop.supplier.site, loop.comparison, loop.stop, loop.binding,
                 *(step.site for step in (*loop.prefix, *loop.actions, *suffix))]
        return self.fact("RESULT_CONTROLLED_LOOP", " ".join(parts), sites)

    def source(self, loop, locals_):
        tokens = self.tokens(loop.supplier.site)
        name = tokens[0].text
        literal = self.literal(loop.supplier.site)
        target = "the loop's selected result"
        sites = [loop.supplier.site, loop.binding, loop.comparison]
        if loop.supplier.site in self.external_sites:
            text = f"The retained external call in {literal} supplies {target} when its evaluation succeeds."
        elif name in locals_:
            local = locals_[name]
            sites.append(local.site)
            text = f"Source takes {target} through the local `{name}` at {literal}."
            if not local.mutable and local.initializer and (initial := self.literal(self.span(local.initializer))):
                text += f" Its retained initializer is {initial}."
        elif name in self.parameters:
            parameter = self.parameters[name]
            sites.append(parameter.site)
            text = f"Source takes {target} through the declared parameter `{name}` at {literal}."
        elif name == "this" and len(tokens) >= 3 and tokens[1].text == ".":
            text = f"Source takes {target} through the member on `this` at {literal}."
        elif loop.supplier.outcome:
            text = f"The normal return of {literal} supplies {target}."
        else:
            return None
        return self.fact("LOOP_RESULT_SOURCE", text, sites)

    def accumulations(self, loop, locals_):
        if not loop.result:
            return
        locals_ = locals_.copy()
        for statement in self.reached_steps(loop.suite):
            if loop.result in self.reassignments((statement,)):
                return
            if (shadow := self.declaration(statement)) and shadow[0] in locals_:
                locals_.pop(shadow[0])
            if not (update := self.accumulation(statement, locals_)):
                continue
            name, local, operand = update
            if not operand or operand[0].text != loop.result or not (self.expected(operand) or self.call(operand)):
                continue
            if (site := self.literal(statement.site)) and (declaration := self.literal(local.site)):
                yield self.fact("LOOP_LOCAL_ACCUMULATION", f"The loop uses the selected result in {site} on the local accumulator `{name}`. "
                                f"The local is declared by {declaration} before the loop.",
                                (local.site, statement.site, loop.supplier.site, loop.binding, loop.comparison))

    def failures(self, loop, suffix):
        after = bool(suffix)
        for step in (loop.supplier, *loop.prefix, *loop.actions):
            for handler, mode, exit_ in step.catches:
                call, clause = self.literal(step.site), self.literal(handler)
                transfer = self.literal(exit_) if exit_ else None
                if not call or not clause or exit_ and not transfer:
                    continue
                if mode == "continue":
                    next_ = "the bottom comparison" if loop.position == "tail" else "the next iteration or header test"
                    text = f"{transfer} skips the remaining body and goes to {next_}."
                elif mode == "throw":
                    text = f"{transfer} exits this continuation when the exception escapes."
                else:
                    text = "the source resumes at the next action or test when the handler completes."
                prefix = f"If a failure from {call} selects {clause}"
                yield self.fact("LOOP_FAILURE_PATH", f"{prefix}, {text}",
                                (step.site, handler, exit_, loop.comparison), prefix)
        for statement in self.reached_steps(loop.suite):
            if branch := self.thrown_arm(statement):
                predicate, exit_ = map(self.literal, branch)
                if predicate and exit_:
                    condition = f"{predicate} is true"
                    tail = " and the normal after-loop continuation" if after else ""
                    yield self.fact("LOOP_FAILURE_PATH", f"If this decision is reached and {condition}, {exit_} skips the remaining iteration{tail} "
                                    "once the thrown value evaluates and the exception escapes.",
                                    (*branch, loop.comparison, *(step.site for step in suffix)), condition)

    def validations(self, checks, loop):
        for check in checks:
            passes = " and ".join(filter(None, (check.assumption, check.passes)))
            rejects = " and ".join(filter(None, (check.assumption, check.rejects)))
            yield self.fact("LOOP_ENTRY_VALIDATION", f"Reaching the loop through its leading validation {check.guard} requires {passes}. "
                            f"If that validation is reached and {rejects}, it {check.failure} before the loop starts, after its arguments finish evaluating.",
                            (*check.sites, loop.comparison), passes)

    def scope_tail(self, statements):
        steps = []
        for statement in statements[:MAX_STEPS]:
            if statement.kind == "return" and (not statement.expression or statement.expression[0].text != "@"):
                if callback_call := self.trailing_lambda(statement.expression, statement.site.start):
                    steps.append(callback_call)
                elif text := self.literal(statement.site):
                    steps.append(Continuation(statement.site, text))
                break
            if statement.kind != "expression":
                break
            if callback_call := self.trailing_lambda(statement.expression, statement.site.start):
                steps.append(callback_call)
                # A callback contract is needed to reason about continuation past the call.
                break
            if not (operation := self.call(statement.expression)) or not (text := self.literal(operation.site)):
                break
            steps.append(Continuation(operation.site, text))
        return tuple(steps)

    def project(self):
        function = self.body.function
        if function.expression_body:
            return []
        at, end = function.body + 1, function.end
        checks, facts = [], []
        pending = None
        loops = 0
        for _ in range(MAX_STATEMENTS):
            if at >= end:
                break
            if self.words[at] == ";":
                at += 1
                continue
            if self.words[at] == "do":
                loop, at = self.tail_test(at, end)
                statement = None
            else:
                statement, at = self.unit.statement(at, end, 0)
                loop = self.top_test(statement) if statement.kind == "while" else None
            if pending is not None:
                earlier, earlier_checks, nearby, locals_ = pending
                if statement is not None and not loop and len(nearby) < MAX_STEPS:
                    nearby.append(statement)
                if statement is None or loop or len(nearby) == MAX_STEPS:
                    self.emit(earlier, earlier_checks, nearby, locals_, facts)
                    pending = None
            if loop:
                if loops == MAX_LOOPS:
                    break
                pending = loop, tuple(checks), [], self.locals.copy()
                loops += 1
            if statement is None:
                continue
            if statement.kind in {"while", "for"}:
                if (statement.kind == "while" and self.syntax["strings"](statement.expression) == ("true",)
                        and not any("break" in self.syntax["strings"](self.tokens(item.site)) for item in statement.body)):
                    break
                continue
            hard = self.body.hard_guard(statement)
            if (not self.controls.statement_shape(statement, "CALLABLE").resumes or
                    hard and hard.verified and hard.predicate and self.syntax["strings"](self.tokens(hard.predicate.site)) == ("false",)):
                break
            if check := self.validation(statement):
                if len(checks) < MAX_STEPS:
                    checks.append(check)
            else:
                # A rebind clears the initializer proof without losing the local declaration.
                for name in self.reassignments((statement,)):
                    if local := self.locals.get(name):
                        self.locals[name] = replace(local, initializer=())
                if local := self.declaration(statement):
                    name, value = local
                    self.locals[name] = value
        if pending is not None:
            self.emit(*pending, facts)
        return facts

    def emit(self, loop, checks, nearby, locals_, facts):
        suffix = self.scope_tail(nearby)
        if core := self.core(loop, suffix):
            facts.append(core)
            if result := self.source(loop, locals_):
                facts.append(result)
            facts.extend(self.accumulations(loop, locals_))
            facts.extend(self.failures(loop, suffix))
            facts.extend(self.validations(checks, loop))


def project(body, syntax, controls, external_sites=()):
    try:
        return Projection(body, syntax, controls, external_sites).project()
    except syntax["Unsupported"]:
        return []
