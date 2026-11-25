"""Project one bounded, result-controlled local continuation from retained Kotlin syntax."""

from dataclasses import dataclass
import re


MAX_STEPS = 6
LITERAL = re.compile(r'-?\d[\d_]*(?:[lL])?|"[^"$]*"|true|false|null')


@dataclass(frozen=True)
class Step:
    site: object
    catches: tuple = ()


@dataclass(frozen=True)
class Validation:
    parameters: set
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


class Projection:
    def __init__(self, body, syntax):
        self.body = body
        self.unit = body.unit
        self.syntax = syntax
        self.span = syntax["token_site"]
        self.words = self.unit.words
        self.parameters = {parameter.name for parameter in body.function.params}

    def names(self, tokens):
        """Use ordinary parameter references in their original, unshadowed prefix."""
        if any(token.text in {"{", "->", "::"} for token in tokens):
            return set()
        return {token.text for index, token in enumerate(tokens) if token.text in self.parameters
                and (not index or tokens[index - 1].text not in {".", "?.", "@"})
                and (index + 1 == len(tokens) or tokens[index + 1].text != "=")}

    def validation(self, statement):
        hard = self.body.hard_guard(statement)
        if hard and hard.predicate and (names := self.names(self.tokens(hard.predicate.site))):
            assumption = ("the selectors resolve to " + " and ".join(f"`kotlin.{name}`" for name in hard.bindings)
                          if hard.bindings else "")
            return Validation(names, hard.failure.text, hard.true, hard.false, f"throws `{hard.exception}`",
                              tuple(note.site for note in hard.proof), assumption)
        if (statement.kind == "if" and not statement.alternative and len(statement.body) == 1
                and statement.body[0].kind == "throw" and (names := self.names(statement.expression))):
            site, exit_ = self.span(statement.expression), statement.body[0].site
            if (predicate := self.literal(site)) and (terminal := self.literal(exit_)):
                return Validation(names, f"the check {predicate}", f"{predicate} is false", f"{predicate} is true",
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

    def action(self, statement, *, continues=False):
        if statement.kind == "expression":
            return self.call(statement.expression)
        if statement.kind != "try" or len(statement.body) != 1 or not statement.handlers or statement.cleanup:
            return None
        operation = self.action(statement.body[0], continues=continues)
        if operation is None or operation.catches:
            return None
        variants = []
        for _, handler, suite in statement.handlers:
            if not suite and not continues:
                variants.append((handler, None))
            elif (len(suite) == 1 and suite[0].kind == "continue" and not suite[0].expression):
                variants.append((handler, suite[0].site))
            else:
                return None
        return Step(operation.site, tuple(variants))

    def assignment(self, statement, declared=None):
        if statement.kind != "expression":
            return None
        tokens = statement.expression
        words = self.syntax["strings"](tokens)
        if declared and words[:2] == (declared, "=") and len(words) > 2:
            return declared, tokens[2:]
        if len(words) >= 4 and words[0] in {"val", "var"} and self.syntax["name"](words[1]):
            split = next((index for index, piece in enumerate(words[2:], 2) if piece == "="), None)
            if split is not None and (split == 2 or words[2] == ":"):
                return words[1], tokens[split + 1:]
        return None

    def supplier(self, tokens):
        if operation := self.call(tokens):
            return operation
        if tokens and tokens[0].text == "try":
            first = self.unit.token_indices[tokens[0].start]
            statement, stop = self.unit.statement(first, first + len(tokens), 0)
            if stop == first + len(tokens):
                return self.action(statement, continues=True)
        return None

    def equality(self, tokens):
        first = self.unit.token_indices[tokens[0].start] if tokens else None
        if first is None:
            return None
        stop = first + len(tokens)
        while stop - first >= 3 and self.words[first] == "(" and self.unit.pairs.get(first) == stop - 1:
            first, stop = first + 1, stop - 1
        common = self.unit.container[first]
        operators = [index for index in range(first, stop) if self.unit.container[index] == common
                     and self.words[index] in {"==", "!="}]
        if len(operators) != 1:
            return None
        middle, = operators
        return tuple(self.unit.tokens[first:middle]), tuple(self.unit.tokens[middle + 1:stop])

    def expected(self, tokens):
        words = self.syntax["strings"](tokens)
        if len(words) == 1 and LITERAL.fullmatch(words[0]):
            return True
        return (bool(words) and len(words) % 2 == 1 and all(self.syntax["name"](word) for word in words[::2])
                and all(word == "." for word in words[1::2]))

    def result_test(self, tokens, name=None):
        if not (sides := self.equality(tokens)):
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

    def actions(self, statements):
        if len(statements) > MAX_STEPS:
            return None
        operations = tuple(self.action(statement) for statement in statements)
        return operations if all(operations) else None

    def top_test(self, loop):
        if result := self.result_test(loop.expression):
            supplier, comparison = result
            if (actions := self.actions(loop.body)) is not None:
                return Loop(supplier, comparison, actions, "header", True)
        if (self.syntax["strings"](loop.expression) != ("true",) or len(loop.body) < 2
                or not (binding := self.assignment(loop.body[0]))):
            return None
        name, tokens = binding
        gate = loop.body[1]
        if (gate.kind != "if" or gate.alternative or len(gate.body) != 1
                or gate.body[0].kind != "break" or gate.body[0].expression):
            return None
        if ((supplier := self.supplier(tokens)) and (result := self.result_test(gate.expression, name))
                and (actions := self.actions(loop.body[2:])) is not None):
            return Loop(supplier, result[1], actions, "body", False, gate.body[0].site, loop.body[0].site)
        return None

    def tail_test(self, start, end, declared=None):
        suite, at = self.unit.block(start + 1, end, 0)
        if not suite or len(suite) > MAX_STEPS + 1 or self.words[at:at + 2] != ("while", "("):
            return None, at
        close = self.unit.pairs[at + 1]
        predicate = self.unit.tokens[at + 2:close]
        for index, statement in enumerate(suite):
            if not (binding := self.assignment(statement, declared)):
                continue
            name, tokens = binding
            if not ((supplier := self.call(tokens)) and (result := self.result_test(predicate, name))):
                continue
            # A continue before this binding could reach the test without this iteration's result.
            before, after = self.actions(suite[:index]), self.actions(suite[index + 1:])
            if before is not None and after is not None and not any(exit_ for step in before for _, exit_ in step.catches):
                return Loop(supplier, result[1], after, "tail", True, binding=statement.site, prefix=before), close + 1
        return None, close + 1

    def describe(self, loop, validations, suffix, terminal, declaration):
        steps = (*loop.prefix, loop.supplier, *loop.actions, *suffix)
        sites = [*(site for validation in validations for site in validation.sites), loop.comparison]
        sites.extend(site for site in (declaration, loop.binding, loop.stop, terminal) if site)
        sites.extend(step.site for step in steps)
        sites.extend(site for step in steps for handler, exit_ in step.catches for site in (handler, exit_) if site)
        literals = {site: self.literal(site) for site in sites}
        if not all(literals.values()):
            return None
        assumptions = list(dict.fromkeys(row.assumption for row in validations if row.assumption))
        passing = " and ".join([*assumptions, *(row.passes for row in validations)])
        parameters = ", ".join(f"`{name}`" for name in sorted(set().union(*(row.parameters for row in validations))))
        parts = [f"The leading validations of {parameters} are " + " then ".join(row.guard for row in validations) + "."]
        if loop.prefix:
            parts.append("After these checks, each entered iteration reaches " + " then ".join(literals[step.site] for step in loop.prefix)
                         + f" before {literals[loop.supplier.site]} supplies the loop result.")
        else:
            parts.append(f"After these checks complete normally, {literals[loop.supplier.site]} supplies the loop result.")
        for row in validations:
            failure = " and ".join(filter(None, (row.assumption, row.rejects)))
            parts.append(f"If source reaches {row.guard} and {failure}, it {row.failure} before the loop starts, after its arguments finish evaluating.")
        order = " then ".join(literals[step.site] for step in loop.actions)
        comparison = literals[loop.comparison]
        if loop.position == "tail":
            before = f"The normal iteration reaches {order} before the comparison. " if order else ""
            parts.append(before + f"The comparison {comparison} is tested after the body. If true, source starts another iteration.")
        elif loop.continue_if:
            next_ = f"the body reaches {order}, then the next test reevaluates the supplier" if order else "the next test reevaluates the supplier"
            parts.append(f"The comparison {comparison} is tested before the body. On the normal true continuation, {next_}.")
        else:
            parts.append(f"If {comparison} is true, {literals[loop.stop]} exits the loop. If false, source "
                         + (f"reaches {order}, then starts another iteration." if order else "starts another iteration."))
        if loop.continue_if:
            parts.append(f"If {comparison} evaluates false, the loop exits normally.")
        after = [*(literals[step.site] for step in suffix), *([literals[terminal]] if terminal else [])]
        if after:
            parts.append("After the normal exit, source reaches " + " then ".join(after)
                         + " if prior evaluations return normally.")
        skipped = f" bypasses {order} in that iteration and" if order else ""
        parts.append(f"A failure escaping the supplier{skipped} prevents the normal loop exit on that path. "
                     "A failure escaping a body action skips the later calls in that iteration and the next test.")
        for step in steps:
            for handler, exit_ in step.catches:
                if exit_:
                    resume = ("evaluates the bottom comparison before another possible iteration" if loop.position == "tail"
                              else "continues at the next loop iteration or header test")
                    consequence = f"{literals[exit_]} skips the remaining body and {resume}"
                else:
                    consequence = ("source resumes the after-loop continuation if the handler completes" if step in suffix
                                   else "source resumes at the next action or loop test if the handler completes")
                parts.append(f"If a failure from {literals[step.site]} selects {literals[handler]}, {consequence}.")
        return " ".join(parts), passing, tuple(dict.fromkeys(sites))

    def project(self):
        function = self.body.function
        if function.expression_body or len(self.parameters) < 2:
            return None
        start, end = function.body + 1, function.end
        validations = []
        declaration = None
        for _ in range(MAX_STEPS + 1):
            if start == end or self.words[start] in {"while", "do"}:
                break
            statement, after = self.unit.statement(start, end, 0)
            if validation := self.validation(statement):
                validations.append(validation)
                start = after + (self.words[after] == ";")
                continue
            words = self.syntax["strings"](statement.expression)
            if len(words) > 3 and words[0] == "var" and words[2] == ":" and self.expected(statement.expression[3:]):
                start = after + (self.words[after] == ";")
                if self.words[start] == "do" and self.syntax["name"](words[1]):
                    declaration = words[1], statement.site
                    break
            return None
        if len(validations) < 2 or len(set().union(*(row.parameters for row in validations))) < 2 or start >= end:
            return None
        if self.words[start] == "while":
            statement, after = self.unit.statement(start, end, 0)
            loop = self.top_test(statement) if statement.kind == "while" else None
        elif self.words[start] == "do":
            loop, after = self.tail_test(start, end, declaration[0] if declaration else None)
        else:
            return None
        if loop:
            remaining = self.unit.suite(after, end)
            terminal = None
            if remaining and remaining[-1].kind == "return":
                returned = remaining[-1]
                if returned.expression and returned.expression[0].text == "@":
                    return None
                remaining, terminal = remaining[:-1], returned.site
            suffix = self.actions(remaining)
            if suffix is not None and not any(exit_ for step in suffix for _, exit_ in step.catches):
                return self.describe(loop, validations, suffix, terminal, declaration[1] if declaration else None)
        return None


def project(body, syntax):
    try:
        return Projection(body, syntax).project()
    except syntax["Unsupported"]:
        return None
