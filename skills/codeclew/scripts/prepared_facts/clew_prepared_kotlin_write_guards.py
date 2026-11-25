"""Compare only source-bound, stable Kotlin values in a focused write path."""

from collections import defaultdict
import re


HTTP_CLIENT = "java.net.http.HttpClient"
HTTP_RESPONSE = "java.net.http.HttpResponse"
INT = "kotlin.Int"
INTEGER = re.compile(r"-?(?:0|[1-9]\d{0,9})\Z")


class GuardModel:
    def __init__(self, body, syntax, captured=None):
        self.body, self.syntax = body, syntax
        self.unit, self.method, self.origin = body.unit, body.function, 0
        if captured and (site := captured.locate(body.host)) and (method := captured.select_site(site)):
            self.unit, self.method, self.origin = captured, method, site.start
        unit, method = self.unit, self.method
        self.locals = defaultdict(list)
        self.parameters = {param.name: param for param in method.params}
        self.fields = unit.members(unit.enclosing_owner(method))[1]
        self.shadows = set()
        for index in range(method.body + 1, method.end):
            if unit.words[index] in {"for", "catch"} and unit.words[index + 1] == "(":
                self.shadows.update(syntax["declared_names"](unit, index + 2))
            elif unit.words[index] in {"val", "var"}:
                if unit.words[index + 1] == "(":
                    self.shadows.update(syntax["declared_names"](unit, index + 1))
                else:
                    self.locals[unit.words[index + 1]].append(index)
        self.cache = {}

    @staticmethod
    def number(words):
        if len(words) == 1 or len(words) == 2 and words[0] == "-":
            literal = "".join(words)
            if INTEGER.fullmatch(literal) and -(2 ** 31) <= (value := int(literal)) < 2 ** 31:
                return value
        return None

    def declared_type(self, tokens):
        unit = self.unit
        words = self.syntax["strings"](tokens)
        if words in {("Int",), ("Int", "?")} and not any(
                bindings.get("Int") for bindings in (unit.class_names, unit.imports, unit.aliases)):
            return INT
        return unit.declared_type(tokens)

    def stable_binding(self, name, index, depth=0):
        if depth >= 5 or name == "it" or name in self.shadows:
            return None
        unit = self.unit
        position = unit.tokens[index].start - self.origin
        if any(site.start <= position < site.end and any(param == name for param, _ in callback.params)
               for site, (_, callback, _) in self.body.callback_scopes.items()):
            return None
        scopes = set()
        enclosing = unit.scope[index]
        while enclosing is not None:
            scopes.add(enclosing)
            enclosing = unit.scope[enclosing]
        local = [decl for decl in self.locals.get(name, ()) if decl < index and unit.scope[decl] in scopes]
        if local:
            decl = local[-1]
            if unit.words[decl] != "val":
                return None
            try:
                statement, _ = unit.statement(decl, self.method.end, 0)
            except self.syntax["Unsupported"]:
                return None
            tokens = statement.expression
            equal = next((i for i, token in enumerate(tokens) if token.text == "="), None)
            if equal is None or equal < 2 or equal + 1 == len(tokens):
                return None
            kind = (self.declared_type(tokens[3:equal]) if tokens[2].text == ":"
                    else self.result_type(tokens[equal + 1:], depth + 1))
            return kind, decl
        if parameter := self.parameters.get(name):
            if parameter.vararg:
                return None
            first = unit.token_indices[parameter.site.start]
            end = unit.token_ends[parameter.site.end] + 1
            return self.declared_type(unit.tokens[first + 2:end]), first
        if (field := self.fields.get(name)) and field.stable:
            return field.type, unit.token_indices[field.site.start]
        return None

    def result_type(self, tokens, depth):
        if self.number(self.syntax["strings"](tokens)) is not None:
            return INT
        if call := self.syntax["invocation"](tokens):
            callee, args = call
            if len(callee) == 3 and callee[1].text == ".":
                index = self.unit.token_indices[callee[0].start]
                if binding := self.stable_binding(callee[0].text, index, depth):
                    if (binding[0], callee[2].text, len(args)) == (HTTP_CLIENT, "send", 2):
                        return HTTP_RESPONSE
                    if (binding[0], callee[2].text, len(args)) == (HTTP_RESPONSE, "statusCode", 0):
                        return INT
        return ""

    def atoms(self, note, holds):
        key = (note, holds)
        if key not in self.cache:
            self.cache[key] = tuple(self.comparisons(note, holds))
        return self.cache[key]

    def comparisons(self, note, holds):
        unit = self.body.unit
        first, last = unit.token_indices.get(note.site.start), unit.token_ends.get(note.site.end)
        if first is None or last is None or self.body.note(note.site) != note:
            return
        for left, operator, right in self.terms(unit, first, last + 1, holds):
            for value, tested in ((left, right), (right, left)):
                if not tested or (number := self.number(self.syntax["strings"](value))) is None:
                    continue
                words = self.syntax["strings"](tested)
                name = words[0]
                if not self.syntax["name"](name):
                    continue
                index = self.unit.token_indices.get(self.origin + tested[0].start)
                binding = self.stable_binding(name, index) if index is not None else None
                if len(words) == 1 and binding and binding[0] == INT:
                    yield (binding[1], INT), None, number, holds == (operator == "==")
                elif len(words) >= 5 and words[1] == "." and words[3:] == ("(", ")"):
                    identity = (binding[1], HTTP_RESPONSE) if binding and binding[0] == HTTP_RESPONSE and words[2] == "statusCode" else None
                    yield identity, None if identity else words, number, holds == (operator == "==")

    def terms(self, unit, first, end, holds, depth=0):
        if depth >= 16:
            return
        while unit.words[first] == "(" and unit.pairs[first] == end - 1:
            first, end = first + 1, end - 1
        delimiters = []
        index = first
        while index < end:
            if unit.words[index] in {"&&", "||", "==", "!="}:
                delimiters.append(index)
            index = unit.pairs.get(index, index) + 1
        logical = [i for i in delimiters if unit.words[i] in {"&&", "||"}]
        if logical:
            connective = "||" if any(unit.words[i] == "||" for i in logical) else "&&"
            if holds == (connective == "&&"):
                splits = [i for i in logical if unit.words[i] == connective]
                for start, stop in zip((first, *(i + 1 for i in splits)), (*splits, end)):
                    if start < stop:
                        yield from self.terms(unit, start, stop, holds, depth + 1)
        elif len(delimiters) == 1:
            i = delimiters[0]
            yield unit.tokens[first:i], unit.words[i], unit.tokens[i + 1:end]

    def feasible(self, guards):
        selected, excluded = {}, defaultdict(set)
        for note, holds in guards:
            for identity, _, number, equality in self.atoms(note, holds):
                if identity is None:
                    continue
                if equality:
                    if selected.setdefault(identity, number) != number or number in excluded[identity]:
                        return False
                elif selected.get(identity) == number:
                    return False
                else:
                    excluded[identity].add(number)
        return True

    def labels(self, guards):
        calls = defaultdict(set)
        for note, holds in guards:
            for _, unresolved, _, _ in self.atoms(note, holds):
                if unresolved:
                    calls[unresolved].add(note)
        repeated = set().union(*(notes for notes in calls.values() if len(notes) > 1))
        for note, holds in guards:
            text = self.body.guard_text(note, holds)
            if note in repeated:
                line = self.body.host["startLine"] + self.body.unit.text.count("\n", 0, note.site.start)
                text = f"at line {line}, {text}"
            yield text
