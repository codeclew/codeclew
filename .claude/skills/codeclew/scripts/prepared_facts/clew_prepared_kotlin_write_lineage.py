"""Trace conditional Kotlin values through local bindings to a selected write argument."""

from dataclasses import dataclass, replace


MAX_ALTERNATIVES = 8
MAX_NESTING = 8
MAX_STEPS = 72


@dataclass(frozen=True)
class Value:
    steps: tuple = ()
    conditions: tuple = ()
    alternative: bool = False

    def with_step(self, text, *notes):
        return replace(self, steps=(*self.steps, (text, notes)))

    def bind(self, name):
        text, notes = self.steps[-1]
        return replace(self, steps=(*self.steps[:-1], (f"`{name}` receives {text}", notes)))

    def under(self, conditions, *, alternative=False):
        return replace(self, conditions=tuple(dict.fromkeys((*self.conditions, *conditions))),
                       alternative=self.alternative or alternative)


@dataclass(frozen=True)
class Local:
    origins: tuple = ()
    fields: tuple = ()

    def write_field(self, key, values):
        fields = dict(self.fields)
        fields[key] = values
        return replace(self, fields=tuple(fields.items()))


def project(body, prefix, action, controls_type, failure):
    """Return independent source paths for each value that directly feeds the action."""
    return Projection(body, controls_type, failure).derive(prefix, action)


class Projection:
    def __init__(self, body, controls_type, failure):
        self.body, self.unit = body, body.unit
        self.controls = controls_type(body, "the selected write body", ())
        self.name, self.site, self.named = (failure[key] for key in ("name", "token_site", "named_argument"))
        self.invocation, self.unsupported = failure["invocation"], failure["Unsupported"]
        self.remaining = MAX_STEPS

    def note(self, tokens):
        return self.body.note(self.site(tokens)) if tokens else None

    def consume(self, tokens, values):
        """Join an identifier use, never a property selector, label, or nested lambda."""
        used = []
        braces = 0
        for index, token in enumerate(tokens):
            word = token.text
            if word == "{":
                braces += 1
            elif word == "}":
                braces -= 1
            previous = tokens[index - 1].text if index else None
            following = tokens[index + 1].text if index + 1 < len(tokens) else None
            if not braces and word in values and previous not in {".", "?.", "::", "@"} and following != "=":
                local = values[word]
                used.extend(local.origins)
                if following not in {".", "?.", "["}:
                    used.extend(origin for _, field in local.fields for origin in field)
        unique = tuple(dict.fromkeys(used))
        return tuple(origin for origin in unique if not any(
            len(origin.steps) < len(other.steps) and origin.steps == other.steps[:len(origin.steps)]
            and set(origin.conditions) <= set(other.conditions) for other in unique))[:MAX_ALTERNATIVES]

    def assignment(self, statement):
        tokens = statement.expression
        if not tokens:
            return None
        first = self.unit.token_indices[tokens[0].start]
        direct = self.unit.container[first]
        for index, token in enumerate(tokens):
            if token.text in {"=", "+=", "-=", "*=", "/=", "%="} and self.unit.container[first + index] == direct:
                return tokens[:index], token.text, tokens[index + 1:]
        return None

    def decision(self, statement):
        if decision := self.body.decision(statement):
            note, arms = decision
            continuations = [(holds, suite) for holds, suite in arms if self.controls.shape(suite, "CALLABLE").resumes]
            return note, continuations
        return None

    def evaluate(self, tokens, values, conditions, depth):
        if not tokens or depth >= MAX_NESTING:
            return ()
        if tokens[0].text == "if":
            first = self.unit.token_indices[tokens[0].start]
            try:
                branch, end = self.unit.statement(first, first + len(tokens), 0)
            except self.unsupported:
                return ()
            if end != first + len(tokens) or not (decision := self.body.decision(branch)):
                return ()
            result = []
            guard, arms = decision
            for holds, suite in arms:
                if not suite:
                    continue
                selected = (*conditions, (self.body.guard_text(guard, holds), guard))
                result.extend(self.yield_value(suite, values, selected, depth + 1))
            return tuple(dict.fromkeys(result))[:MAX_ALTERNATIVES]
        if value := self.note(tokens):
            return tuple(origin.under(conditions).with_step(value.text, value) for origin in
                         (self.consume(tokens, values) or (Value(),)))
        return ()

    def yield_value(self, suite, values, conditions, depth):
        observed, conditions = self.walk(suite[:-1], dict(values), conditions, depth)
        if observed is None:
            return ()
        last = suite[-1]
        if last.kind == "if":
            first, end = self.unit.token_indices[last.site.start], self.unit.token_ends[last.site.end]
            tokens = self.unit.tokens[first:end + 1]
        elif last.kind == "expression":
            tokens = last.expression
        else:
            return ()
        return tuple(origin.under(conditions, alternative=True) for origin in self.evaluate(tokens, observed, conditions, depth))

    def guard(self, hard, conditions):
        premise = hard.true if hard.verified else hard.failure.text + " returns normally"
        return (*conditions, (premise, hard.failure))

    def elvis(self, tokens):
        if not tokens:
            return None
        first = self.unit.token_indices[tokens[0].start]
        outer = self.unit.container[first]
        for index, token in enumerate(tokens):
            if token.text == "?:" and index > 0 and self.unit.container[first + index] == outer:
                return tokens[:index]
        return None

    def bind(self, values, assignment, conditions, depth, hard=None):
        left, operation, right = assignment
        labels = tuple(token.text for token in left)
        if labels[0] == "val" and len(labels) >= 2 and self.name(labels[1]) and operation == "=":
            if hard and hard.true and (lhs := self.elvis(right)) and (note := self.note(lhs)):
                # Only the non-null operand can supply the value without evaluating the hard exit.
                conditions = (*conditions, (f"{note.text} yields a non-null value", note))
                right = lhs
            binding = labels[1]
            values[binding] = Local(tuple(origin.bind(binding) for origin in self.evaluate(right, values, conditions, depth)))
        elif len(labels) == 1 or len(labels) >= 2 and labels[0] == "var":
            values[labels[0] if len(labels) == 1 else labels[1]] = Local()
        return conditions

    def accumulation(self, statement, active, owners):
        changed = self.assignment(statement)
        if changed and changed[1] == "+=" and len(changed[0]) == 1:
            name, operand = changed[0][0].text, changed[2]
        elif invocation := self.invocation(statement.expression):
            callee, args = invocation
            words = tuple(item.text for item in callee)
            if len(words) != 3 or words[1:] not in {(".", "add"), (".", "addAll")} or len(args) != 1:
                return None
            name, operand = words[0], self.named(args[0])[1]
        else:
            return None
        if name in owners and active.get(name) is owners[name]:
            return name, operand
        return None

    def loop(self, statement, values, conditions, depth):
        tokens = statement.expression
        if len(tokens) < 3 or not self.name(tokens[0].text) or tokens[1].text != "in":
            return
        name, iterable = tokens[0].text, tokens[2:]
        if not (note := self.note(iterable)):
            return
        origins = [origin for origin in self.consume(iterable, values) if origin.alternative]
        if not origins:
            return
        iteration = (*conditions, (f"the loop reaches an element of {note.text}", note))
        locals_ = {**values, name: Local(tuple(origin.under(iteration).with_step(f"`{name}` comes from {note.text}", note)
                                              for origin in origins))}
        changes = {}
        self.walk(statement.body, locals_, iteration, depth + 1, values, changes)
        for name, rows in changes.items():
            values[name] = Local(tuple(dict.fromkeys(rows))[:MAX_ALTERNATIVES])

    def update(self, statement, invocation, values, conditions):
        callee, arguments = invocation
        words = tuple(token.text for token in callee)
        if len(words) != 3 or words[0] not in values or words[1] != ".":
            return
        receiver, _, member = words
        if member in {"clear", "remove", "removeAll", "removeAt", "removeIf", "retainAll", "replace"}:
            values[receiver] = Local()
        elif member in {"set", "put"} and len(arguments) == 2:
            key, value = (self.named(argument)[1] for argument in arguments)
            if len(key) != 1 or not key[0].text.startswith('"') or "$" in key[0].text:
                values[receiver] = Local()
                return
            if (operand := self.note(value)) and (action := self.body.note(statement.site)):
                traces = tuple(origin.under(conditions).with_step(f"{operand.text} supplies {action.text}", operand, action)
                               for origin in self.consume(value, values) if origin.alternative)
                values[receiver] = values[receiver].write_field(key[0].text, traces)
            else:
                values[receiver] = Local()

    def walk(self, suite, values, conditions=(), depth=0, owners=None, changes=None):
        if depth >= MAX_NESTING:
            return None, conditions
        for statement in suite:
            self.remaining -= 1
            if self.remaining < 0 or statement.kind in {"return", "throw", "break", "continue", "unsupported"}:
                return None, conditions
            if owners is not None and (mutation := self.accumulation(statement, values, owners)):
                target, operand = mutation
                if (expression := self.note(operand)) and (action := self.body.note(statement.site)):
                    changes.setdefault(target, []).extend(
                        origin.under(conditions).with_step(f"{expression.text} supplies {action.text}", expression, action)
                        for origin in self.consume(operand, values) if origin.alternative)
                continue
            if statement.kind == "if":
                decision = self.decision(statement)
                if owners is not None and decision:
                    note, viable = decision
                    for holds, branch in viable:
                        selected = (*conditions, (self.body.guard_text(note, holds), note))
                        self.walk(branch, dict(values), selected, depth + 1, owners, changes)
                if decision and len(decision[1]) == 1:
                    note, ((holds, _),) = decision
                    conditions = (*conditions, (self.body.guard_text(note, holds), note))
                elif decision and not decision[1]:
                    return None, conditions
            elif statement.kind == "for" and owners is None:
                self.loop(statement, values, conditions, depth)
            else:
                hard = self.body.hard_guard(statement)
                if hard and not hard.true:
                    return None, conditions
                if (changed := self.assignment(statement)) and changed[0]:
                    conditions = self.bind(values, changed, conditions, depth, hard)
                elif hard:
                    lhs = self.elvis(statement.expression) if hard.true else None
                    if lhs and (note := self.note(lhs)):
                        conditions = (*conditions, (f"{note.text} yields a non-null value", note))
                    else:
                        conditions = self.guard(hard, conditions)
                elif invocation := self.invocation(statement.expression):
                    self.update(statement, invocation, values, conditions)
        return values, conditions

    def derive(self, prefix, action):
        first, last = self.unit.token_indices.get(action.start), self.unit.token_ends.get(action.end)
        if first is None or last is None or not (invocation := self.invocation(self.unit.tokens[first:last + 1])):
            return ()
        values, conditions = self.walk(prefix, {})
        if values is None:
            return ()
        result = []
        for argument in invocation[1]:
            _, tokens = self.named(argument)
            if not (note := self.note(tokens)):
                continue
            for origin in self.consume(tokens, values):
                if origin.alternative:
                    result.append(origin.under(conditions).with_step(f"the write receives {note.text}", note))
        return tuple(dict.fromkeys(result))[:MAX_ALTERNATIVES]
