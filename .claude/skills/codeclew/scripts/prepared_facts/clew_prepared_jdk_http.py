"""Resolve retained JDK HTTP requests for the process map."""

from pathlib import Path
import re
import runpy
from urllib.parse import urlsplit


SYNTAX = runpy.run_path(str(Path(__file__).with_name("clew_kotlin_syntax.py")))
Kotlin, Unsupported, groups, invocation, name, strings = (
    SYNTAX[key] for key in ("Kotlin", "Unsupported", "groups", "invocation", "name", "strings")
)
CLIENT = "java.net.http.HttpClient"
REQUEST = "java.net.http.HttpRequest"
URI = "java.net.URI"
VERBS = {"GET": 0, "DELETE": 0, "POST": 1, "PUT": 1}
METHODS = set(VERBS) | {"PATCH", "HEAD", "OPTIONS", "TRACE"}
OPTIONS = {"header": 2, "setHeader": 2, "timeout": 1, "version": 1, "expectContinue": 1}
MAX_SOURCE = 128 * 1024
MAX_REQUESTS = 16
JVM_FILES = (".kt", ".kts", ".java")


def absolute_url(parts):
    """Allow dynamic query values only after a literal query delimiter."""
    prefix = ""
    for part in parts:
        if len(part) != 1 or not (token := part[0].text).startswith('"'):
            return None
        text = token[3:-3] if token.startswith('"""') else token[1:-1]
        literal, query, _ = text.partition("?")
        if any(char in literal for char in ('$', '\\', '#', '"')) or any(char.isspace() for char in literal):
            return None
        prefix += literal
        if query:
            break
    try:
        url = urlsplit(prefix)
        if (url.scheme in {"http", "https"} and url.hostname
                and re.fullmatch(r"[a-zA-Z0-9.:-]+", url.netloc)
                and (url.port is None or 0 < url.port <= 65535)
                and len(prefix) <= 512):
            return url.scheme + "://" + url.netloc, url.path or "/"
    except ValueError:
        return None


class Method:
    def __init__(self, unit, site, java):
        self.unit, self.words, self.java = unit, unit.words, java
        self.openings = {end: start for start, end in unit.pairs.items()}
        self.kotlin = None if java else unit.select_site(site)
        if self.kotlin:
            self.name = self.kotlin.name
            self.start, self.end = self.kotlin.body + 1, self.kotlin.end + self.kotlin.expression_body
            self.params = {parameter.name: parameter.type for parameter in self.kotlin.params}
            owner = unit.enclosing_owner(self.kotlin)
            self.fields = {name: field.type for name, field in unit.members(owner)[1].items() if field.stable}
        elif java:
            first = next(i for i, token in enumerate(unit.tokens) if token.start >= site.start)
            last = next((i for i, token in enumerate(unit.tokens) if token.end > site.end), len(unit.tokens))
            opening = self.openings[last - 1]
            if opening < first or self.words[opening] != "{":
                raise Unsupported()
            self.start, self.end = opening + 1, last - 1
            close = next((i for i in reversed(range(first, opening)) if self.words[i] == ")"), None)
            left = self.openings.get(close, -1)
            self.name = self.words[left - 1] if left > first else None
            parameters = groups(unit.tokens[left + 1:close]) if left >= first else ()
            self.params = {arg[-1].text: unit.declared_type(arg[:-1]) for arg in parameters if len(arg) > 1}
            parent = unit.scope[opening]
            fields = range(parent + 1, unit.pairs[parent]) if parent is not None else ()
            self.fields = {slot[0]: CLIENT for i in fields if unit.container[i] == parent
                           and self.words[i] == "final" and (slot := self.java_decl(i + 1)) and slot[2] == CLIENT}
        else:
            raise Unsupported()
        self.locals = {}
        for index in range(self.start, self.end):
            if slot := self.declaration(index):
                self.locals.setdefault(slot[0], []).append((index, *slot[1:]))

    def java_decl(self, index):
        words = self.words
        if not name(words[index]) or words[index - 1] in {".", "="}:
            return None
        stop = index + 1
        while words[stop] == "." and name(words[stop + 1]):
            stop += 2
        if (name(words[stop]) and words[stop + 1] == "="
                and words[index] not in {"return", "throw", "new", "final"}):
            end = self.unit.until(stop + 2, len(self.unit.tokens), {";", "}"}, multiline=True)
            return words[stop], self.unit.tokens[stop + 2:end], self.unit.declared_type(self.unit.tokens[index:stop])

    def declaration(self, index):
        words = self.words
        if self.java:
            return self.java_decl(index)
        variable = words[index + 1]
        if words[index] not in {"val", "var"} or not name(variable):
            return None
        end = self.unit.until(index + 2, self.end, {";", "}"})
        equal = next((i for i in range(index + 2, end) if words[i] == "="), None)
        typ = self.unit.declared_type(self.unit.tokens[index + 3:equal]) if equal and words[index + 2] == ":" else ""
        return variable, self.unit.tokens[equal + 1:end] if equal is not None and words[index] == "val" else [], typ

    def type(self, tokens, expected):
        words = strings(tokens)
        if not words or self.unit.qualify("".join(words)) != expected:
            return False
        head = words[0]
        return (not any(head in scope for scope in (self.params, self.fields, self.locals))
                and not any(word in {"class", "object", "interface", "typealias", "val", "var", "fun"} and next_ == head
                            for word, next_ in zip(self.words, self.words[1:])))

    def stable(self, tokens, *, builder=False):
        if len(tokens) != 1:
            return ()
        name = tokens[0].text
        candidates = self.locals.get(name, ())
        if len(candidates) != 1:
            return ()
        index, expression, declared = candidates[0]
        use = self.unit.token_indices[tokens[0].start]
        scope = self.unit.scope[use]
        while scope is not None and scope != self.unit.scope[index]:
            scope = self.unit.scope[scope]
        if not expression or scope != self.unit.scope[index] or use <= index:
            return ()
        if not builder:
            return expression if self.words[self.start:self.end].count(name) == 2 else ()
        typed = self.words[index] != "var" if self.java else self.words[index + 2] == ":"
        if typed and declared != REQUEST + ".Builder":
            return ()
        lhs = self.unit.token_indices[expression[0].start] - 2
        end = self.unit.token_ends[expression[-1].end]
        return expression if self.builder_uses(name, lhs, end, use) else ()

    @staticmethod
    def builder_option(member, args):
        return (len(args) >= 2 and len(args) % 2 == 0 if member == "headers"
                else len(args) == OPTIONS.get(member, -1))

    def builder_uses(self, name, declaration, initializer_end, use):
        """Require references to be discarded options on the same builder."""
        spellings = {name} if self.java else {name, "`" + name + "`"}
        # Reject possible uses hidden in Kotlin string interpolation.
        if not self.java and any(word.startswith('"') and '$' in word and re.search(r"\b" + name + r"\b", word)
                                 for word in self.words[self.start:self.end]):
            return False
        references = {index for index in range(self.start, self.end) if self.words[index] in spellings}
        references.difference_update((declaration, use))
        scopes = {self.unit.scope[index] for index in references}
        # An enclosing initializer can fold the whole if/try into one statement.
        pending = [statement for scope in scopes for statement in self.unit.suite(scope + 1, self.unit.pairs[scope])]
        while pending and references:
            statement = pending.pop()
            pending.extend(statement.body + statement.alternative)
            if statement.kind != "expression" or not (tokens := statement.expression):
                continue
            first = self.unit.token_indices[tokens[0].start]
            last = self.unit.token_ends[tokens[-1].end]
            if first in references and initializer_end < first <= last < use and (chain := self.chain(tokens)):
                receiver, parts = chain
                if strings(receiver) == (name,) and all(self.builder_option(*part) for part in parts):
                    references.remove(first)
        return not references

    def url(self, expression):
        call = invocation(expression)
        if not call:
            return None
        callee, args = call
        if len(args) != 1 or strings(callee[-2:]) != (".", "create") or not self.type(callee[:-2], URI):
            return None
        value = args[0]
        if len(value) == 1 and not value[0].text.startswith('"'):
            value = self.stable(value)
        return absolute_url(groups(value, "+"))

    def chain(self, expression):
        if not expression:
            return ()
        first = self.unit.token_indices[expression[0].start]
        last = self.unit.token_ends[expression[-1].end]
        parts = []
        while last > first and self.words[last] == ")":
            opening = self.openings[last]
            dot = opening - 2
            if dot <= first or self.words[dot] != ".":
                break
            parts.append((self.words[dot + 1], tuple(groups(self.unit.tokens[opening + 1:last]))))
            if self.words[dot - 1] != ")":
                return self.unit.tokens[first:dot], parts[::-1]
            last = dot - 1
        return ()

    def client(self, expression):
        prefix = strings(expression)
        if len(prefix) == 3 and prefix[:2] == ("this", "."):
            return self.fields.get(prefix[-1]) == CLIENT
        if len(prefix) == 1:
            name = prefix[0]
            if local := self.locals.get(name):
                if len(local) != 1:
                    return False
                if local[0][2] == CLIENT:
                    return True
                expression = self.stable(expression)
            else:
                return (self.params.get(name) or self.fields.get(name)) == CLIENT
        chain = self.chain(expression)
        if not chain:
            return False
        owner, parts = chain
        return self.type(owner, CLIENT) and (parts == [("newHttpClient", ())] or
            parts[0] == ("newBuilder", ()) and parts[-1] == ("build", ())
            and all(member in SYNTAX["HTTP_CLIENT_BUILDER_METHODS"] and len(args) == 1 for member, args in parts[1:-1]))

    def request(self, expression, *, built=True, depth=0):
        if depth >= 6 or not expression:
            return None
        if len(expression) == 1:
            value = self.stable(expression, builder=not built)
            return self.request(value, built=built, depth=depth + 1)
        chain = self.chain(expression)
        if not chain:
            return None
        owner, parts = chain
        first, arguments = parts[0]
        if self.type(owner, REQUEST) and first == "newBuilder" and len(arguments) <= 1:
            origin = self.url(arguments[0]) if arguments else None
            if arguments and not origin:
                return None
            method, parts = None, parts[1:]
        elif len(owner) == 1 and (prior := self.request(owner, built=False, depth=depth + 1)):
            origin, method = prior
        else:
            return None
        if built:
            if parts[-1:] != [("build", ())]:
                return None
            parts.pop()
        for member, args in parts:
            if member == "uri" and not origin and len(args) == 1:
                if not (origin := self.url(args[0])):
                    return None
            elif member in VERBS and not method and len(args) == VERBS[member]:
                method = member
            elif member == "method" and not method and len(args) == 2 and len(args[0]) == 1:
                token = args[0][0].text
                method = token[1:-1] if token.startswith('"') and token.endswith('"') else None
                if method not in METHODS:
                    return None
            elif not self.builder_option(member, args):
                return None
        return (origin, method) if not built or origin and method else None

    def early_return_guard(self, send):
        if self.unit.container[send] != self.start - 1 or self.kotlin and self.kotlin.expression_body:
            return None, []
        conditions, failures = [], []
        for row in self.unit.suite(self.start, self.end):
            if row.site.end >= self.unit.tokens[send].start:
                break
            if (row.kind != "if" or row.alternative or len(row.body) != 1 or row.body[0].kind != "return"
                    or not row.expression or row.body[0].expression[:1] and row.body[0].expression[0].text == "@"):
                continue
            tokens = row.expression
            start = self.unit.token_indices[tokens[0].start]
            parent = self.unit.container[start]
            # An OR of early exit checks is a short-circuit AND of prerequisites.
            splits = [i for i, token in enumerate(tokens) if token.text == "||" and self.unit.container[start + i] == parent]
            effect = self.unit.excerpt(row.body[0].site).strip().removesuffix(";")
            for i, j in zip([-1, *splits], [*splits, len(tokens)], strict=True):
                part = tokens[i + 1:j]
                if not part:
                    return None, []
                negative = self.unit.text[part[0].start:part[-1].end]
                if len(part) == 2 and part[0].text == "!" and name(part[1].text):
                    positive = part[1].text
                elif len(part) == 1 and name(part[0].text):
                    positive = "!" + negative
                else:
                    positive = "!(" + negative + ")"
                    if len(part) > 1:
                        negative = "(" + negative + ")"
                failures.append((" && ".join([*conditions, negative]), effect))
                conditions.append(positive)
                if len(conditions) > 4:
                    return None, []
        if not conditions:
            return None, []
        condition = " && ".join(conditions)
        group = f"http:{self.unit.file}:{self.unit.tokens[send].start}"
        cases = [dict(group=group, condition=condition, actions=["send"], stepKind="call",
                      meaning="proceed to the outbound request if the remaining evaluations complete normally")]
        cases.extend(dict(group=group, condition=test, actions=[effect, "send"], skip=["send"],
                          stepKind="return", meaning="return before making the outbound request")
                     for test, effect in failures)
        return condition, cases

    def sends(self):
        for i in range(self.start + 2, self.end):
            if self.words[i - 1:i + 2] != (".", "send", "("):
                continue
            stop = self.unit.pairs[i + 1]
            receivers = [self.unit.tokens[i - 2:i - 1]]
            if self.words[i - 4:i - 2] == ("this", "."):
                receivers = [self.unit.tokens[i - 4:i - 1]]
            elif self.words[i - 2] == ")":
                receivers = [self.unit.tokens[start:i - 1] for start in range(max(self.start, i - 64), i - 2)
                             if name(self.words[start]) and self.words[start - 1] != "."]
            args = tuple(groups(self.unit.tokens[i + 2:stop]))
            if len(args) != 2 or not any(self.client(receiver) for receiver in receivers):
                continue
            if result := self.request(args[0]):
                try:
                    guard, cases = self.early_return_guard(i)
                except Unsupported:
                    guard, cases = None, []
                yield self.unit.site(i, stop + 1), result, guard, cases


def project(process, context, reached, units):
    bind = context["_bindWitness"]
    bodies = reached["body_index"](process)
    if not any(body["file"].endswith(JVM_FILES) for body in bodies.values()):
        return []
    targets, _ = reached["reached"](process, bodies, bind, context["_sourceCallSelections"])
    if not targets:
        return []
    root_cite = targets[process["root"]][1]
    records = []
    for body, body_cite, trail, _ in targets.values():
        file, revision = body["file"], body["revision"]
        if not file.endswith(JVM_FILES):
            continue
        key = reached["key"](body)
        if key not in units:
            native = context["_nativeFiles"].get(file, {})
            text = native.get("text", "")
            if native.get("revision") != revision or native.get("textDigest") != reached["digest"](text.encode()):
                text = context["_loadPinnedFile"](file, revision)
            units[key] = None
            if text and "java.net.http" in text and len(text.encode()) <= MAX_SOURCE:
                try:
                    units[key] = Kotlin(text, file)
                except Unsupported:
                    pass
        if not (unit := units[key]) or not (source := unit.locate(body)):
            continue
        try:
            method = Method(unit, source, file.endswith(".java"))
            for site, ((origin, path), verb), guard, cases in method.sends():
                if len(records) == MAX_REQUESTS:
                    return records
                raw = unit.excerpt(site)
                prefix = unit.text[:site.start]
                first, start = prefix.count("\n") + 1, len(prefix.encode())
                witness = dict(body, text=raw, textDigest=reached["digest"](raw.encode()), startLine=first,
                               endLine=first + raw.count("\n"), startByte=start, endByte=start + len(raw.encode()))
                if cite := bind(witness):
                    records.append({"method": verb, "endpoint": origin + path, "path": path,
                                    "site": [file, first, start],
                                    "sources": list(dict.fromkeys([root_cite, body_cite, cite, *(handle for _, handle in trail)])),
                                    "condition": None if trail else guard,
                                    **({"_conditionCases": [dict(row, scope=method.name) for row in cases]
                                        if trail and method.name else cases} if cases else {})})
        except (IndexError, KeyError, StopIteration, Unsupported):
            continue
    return records
