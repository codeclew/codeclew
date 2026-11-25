"""Retained Kotlin source parsing reused by the bounded fact projector."""

from __future__ import annotations

from collections import Counter, defaultdict
from dataclasses import dataclass
import re


NAME = re.compile(r"[A-Za-z_][A-Za-z_0-9]*\Z")
TEMPLATE_NAME = re.compile(r"(?:[^\W\d]|_)\w*|`[^`\n]+`")
TOKEN = re.compile(
    r'\s+|//[^\n]*|/\*|'
    r"'(?:[^'\\\n]|\\.)*'|`[^`\n]+`|[A-Za-z_][A-Za-z_0-9]*|"
    r'[0-9]+(?:\.[0-9]+)?[A-Za-z]*|===|!==|==|!=|<=|>=|\?\.|\?:|!!|&&|\|\||'
    r'\+\+|--|\+=|-=|\*=|/=|%=|::|->|[^\s]'
)
COMMENT = re.compile(r"/\*|\*/")
PAIR = {"(": ")", "[": "]", "{": "}"}
MODIFIERS = {"private", "internal", "public", "protected", "final", "open", "abstract", "sealed",
             "override", "lateinit", "const", "inline", "suspend", "tailrec", "operator", "infix",
             "external", "actual", "expect"}
MEMBER = {"fun", "class", "interface", "object", "val", "var", "init", "constructor"}
COMPARISONS = {"==", "!=", "===", "!==", ">", "<", ">=", "<="}
JDBC = {
    "java.sql.Connection": {"rollback": "ROLLBACK", "commit": "COMMIT"},
    "java.sql.Statement": {"executeUpdate": "TRANSACTION_DEPENDENT", "executeLargeUpdate": "TRANSACTION_DEPENDENT"},
    "java.sql.PreparedStatement": {"executeUpdate": "TRANSACTION_DEPENDENT",
                                   "executeLargeUpdate": "TRANSACTION_DEPENDENT"},
}
REMOTE = {"java.net.http.HttpClient": {"send"}, "java.net.HttpURLConnection": {"connect"}}
EXACT_RESOLUTIONS = {"SAME_SCOPE_UNIQUE", "SAME_SCOPE_SELECTED_DECLARATION"}
COMPILER_EDGES = {"RETAINED_K2_COMPILER_SELECTED_SYMBOL_NOT_RUNTIME_DISPATCH",
                  "JAVAC_SELECTED_DECLARATION_NOT_CONTROL_FLOW"}
SYNTAX_AUTHORITY = "RETAINED_KOTLIN_SYNTAX_CANDIDATE"
UNIQUE_SYNTAX = "UNIQUE_SYNTAX_CANDIDATE"
HTTP_CLIENT = "java.net.http.HttpClient"
HTTP_CLIENT_BUILDER_METHODS = frozenset({
    "authenticator", "connectTimeout", "cookieHandler", "executor", "followRedirects", "localAddress",
    "priority", "proxy", "sslContext", "sslParameters", "version",
})

MAP_FACTORIES = {
    "kotlin.collections.mutableMapOf": "kotlin.collections.MutableMap",
    "kotlin.collections.hashMapOf": "java.util.HashMap",
    "kotlin.collections.linkedMapOf": "java.util.LinkedHashMap",
}
MAP_CONSTRUCTORS = {
    "java.util.HashMap", "java.util.LinkedHashMap", "java.util.TreeMap",
    "java.util.concurrent.ConcurrentHashMap",
}
MAP_TYPES = MAP_CONSTRUCTORS | set(MAP_FACTORIES.values())
FACTORY_RETURNS = {
    **MAP_FACTORIES,
    "java.net.http.HttpClient.newHttpClient": "java.net.http.HttpClient",
    "java.sql.DriverManager.getConnection": "java.sql.Connection",
}
DEFAULT_SYMBOLS = {
    "Throwable": "kotlin.Throwable",
    "Nothing": "kotlin.Nothing",
    "MutableMap": "kotlin.collections.MutableMap",
    "HashMap": "java.util.HashMap",
    "LinkedHashMap": "java.util.LinkedHashMap",
    **{qualified.rsplit(".", 1)[1]: qualified for qualified in MAP_FACTORIES},
}
KNOWN_INITIALIZERS = MAP_CONSTRUCTORS | FACTORY_RETURNS.keys()


class Unsupported(ValueError):
    pass


class EmptyTokenSpan(IndexError):
    """A source span was requested for an empty token sequence."""


@dataclass(frozen=True)
class Token:
    text: str
    start: int
    end: int
    file: str = ""


@dataclass(frozen=True)
class Site:
    start: int
    end: int
    file: str = ""


@dataclass(frozen=True)
class Owner:
    name: str
    start: int
    body: int
    end: int


@dataclass(frozen=True)
class Function:
    name: str
    start: int
    body: int
    end: int
    params: tuple
    modifiers: frozenset
    expression_body: bool = False
    file: str = ""
    returns: str = ""
    never_returns: bool = False


@dataclass(frozen=True)
class ParameterDecl:
    name: str
    type: str
    site: Site
    default: tuple = ()
    vararg: bool = False


@dataclass(frozen=True)
class Field:
    name: str
    site: Site
    type: str
    storage: str
    stable: bool
    inferred_type: bool = False


@dataclass(frozen=True)
class Statement:
    kind: str
    site: Site
    expression: tuple = ()
    body: tuple = ()
    alternative: tuple = ()
    handlers: tuple = ()
    cleanup: tuple = ()


@dataclass(frozen=True)
class Call:
    name: str
    site: Site
    receiver: str
    member: str
    args: tuple
    contract: str
    receiver_kind: str = ""
    receiver_site: Site | None = None
    type: str = ""
    receiver_declaration: Site | None = None


def interpolation(text, start, file, depth):
    expected, tokens = ["}"], []
    for token in lex(text, file, start, depth):
        word = token.text
        if word in PAIR:
            expected.append(PAIR[word])
        elif word in PAIR.values():
            if word != expected.pop():
                break
            if not expected and tokens:
                return token.end, tuple(tokens)
        tokens.append(token)
        if not expected:
            break
    raise Unsupported()


def quoted_string(text, start, file="", depth=0):
    if depth > 12:
        raise Unsupported()
    delimiter = '"""' if text.startswith('"""', start) else '"'
    raw = len(delimiter) == 3
    at, substitutions = start + len(delimiter), []
    while at < len(text):
        if text.startswith(delimiter, at):
            at += len(delimiter)
            while raw and text[at:at + 1] == '"':
                at += 1
            return at, tuple(substitutions)
        word = text[at]
        if not raw and word == "\\":
            if text[at + 1:at + 2] in {"", "\n", "\r"}:
                break
            at += 2
            continue
        if not raw and word in {"\n", "\r"}:
            break
        if word == "$":
            if text[at + 1:at + 2] == "{":
                at, expression = interpolation(text, at + 2, file, depth + 1)
                substitutions.append(expression)
                continue
            if match := TEMPLATE_NAME.match(text, at + 1):
                substitutions.append((Token(match[0], match.start(), match.end(), file),))
                at = match.end()
                continue
        at += 1
    raise Unsupported()


def lex(text, file="", start=0, depth=0):
    at = start
    while at < len(text):
        if text[at] == '"':
            end, _ = quoted_string(text, at, file, depth)
            yield Token(text[at:end], at, end, file)
            at = end
            continue
        match = TOKEN.match(text, at)
        if match is None:
            raise Unsupported()
        word = match[0]
        at = match.end()
        if word == "/*":
            depth = 1
            while depth:
                comment = COMMENT.search(text, at)
                if not comment:
                    raise Unsupported()
                depth += 1 if comment[0] == "/*" else -1
                at = comment.end()
        elif not (word.isspace() or word.startswith("//")):
            yield Token(word, match.start(), at, file)


def strings(tokens):
    return tuple(token.text for token in tokens)


def name(word):
    return bool(NAME.fullmatch(word)) and word not in {"this", "null", "true", "false", "super"}


def literal(word):
    return (word in {"true", "false", "null"}
            or bool(re.fullmatch(r"[0-9]+[LuU]*|[0-9]+\.[0-9]+[fF]?", word))
            or len(word) >= 2 and word[0] in {'"', "'"} and word[-1] == word[0] and "$" not in word)


def groups(tokens, split=",", types=False):
    start, depth = 0, 0
    for index, token in enumerate(tokens):
        if token.text in PAIR or types and token.text == "<":
            depth += 1
        if token.text in PAIR.values() or types and token.text == ">":
            depth -= 1
        if token.text == split and not depth:
            yield tokens[start:index]
            start = index + 1
    if start < len(tokens):
        yield tokens[start:]


def invocation(tokens):
    """Return the callee and positional source chunks of one complete call."""
    words = strings(tokens)
    if not words or words[-1] != ")" or "(" not in words:
        return None
    opening = words.index("(")
    end = words.index("<") if "<" in words[:opening] and words[opening - 1] == ">" else opening
    callee = tokens[:end]
    if not callee or not name(callee[-1].text):
        return None
    depth = 0
    for index, word in enumerate(words[opening:], opening):
        depth += (word == "(") - (word == ")")
        if depth == 0 and index != len(words) - 1:
            return None
    return callee, tuple(groups(tokens[opening + 1:-1]))


def named_argument(tokens):
    words = strings(tokens)
    if len(words) > 2 and name(words[0]) and words[1] == "=":
        return words[0], tokens[2:]
    return "", tokens


def type_end(words, start, depth=0):
    """Consume the type position of a cast or type test."""
    if depth > 8 or start == len(words) or not name(words[start]):
        return None
    end = start + 1
    while end + 1 < len(words) and words[end] == "." and name(words[end + 1]):
        end += 2
    if words[end:end + 1] == ("<",):
        end += 1
        while end < len(words):
            if words[end] in {"in", "out"}:
                end += 1
            if words[end:end + 1] == ("*",):
                end += 1
            else:
                end = type_end(words, end, depth + 1)
                if end is None:
                    return None
            separator = words[end:end + 1]
            end += 1
            if separator == (">",):
                break
            if separator != (",",):
                return None
        else:
            return None
    return end + (words[end:end + 1] == ("?",))


class Kotlin:
    def __init__(self, text, file=""):
        self.text = text
        self.file = file
        self.tokens = list(lex(text, file))
        self.words = strings(self.tokens) + ("",)
        self.token_indices = {token.start: index for index, token in enumerate(self.tokens)}
        self.token_ends = {token.end: index for index, token in enumerate(self.tokens)}
        self.pairs, self.scope, self.container = {}, {}, {}
        stack, braces = [], []
        for index, word in enumerate(self.words[:-1]):
            self.scope[index] = braces[-1] if braces else None
            self.container[index] = stack[-1] if stack else None
            if word in PAIR:
                stack.append(index)
                if word == "{":
                    braces.append(index)
            elif word in PAIR.values():
                if not stack or PAIR[self.words[stack[-1]]] != word:
                    raise Unsupported()
                self.pairs[stack.pop()] = index
                if word == "}":
                    braces.pop()
        if stack:
            raise Unsupported()
        headers = re.finditer(r"(?m)^[ \t]*(package|import)[ \t]+([\w.]+\*?)(?:[ \t]+as[ \t]+(\w+))?", text)
        self.package = ""
        self.imports, self.aliases = defaultdict(set), defaultdict(set)
        self.stars = set()
        for match in headers:
            if (index := self.token_indices.get(match.start(1))) is None or self.container[index] is not None:
                continue
            header, qualified, alias = match.groups()
            if header == "package":
                self.package = qualified
            elif qualified.endswith(".*"):
                self.stars.add(qualified[:-2])
            else:
                self.imports[alias or qualified.rsplit(".", 1)[-1]].add(qualified)
        for index, word in enumerate(self.words[:-1]):
            if word == "typealias" and self.scope[index] is None and name(self.words[index + 1]):
                stop = self.until(index + 2, len(self.tokens), {";", *MEMBER, "typealias"})
                header = self.words[index + 2:stop]
                if "=" in header:
                    self.aliases[self.words[index + 1]].add("".join(header[header.index("=") + 1:]))
        self.owners = [owner for index, word in enumerate(self.words[:-1])
                       if word in {"class", "object", "interface"} and (owner := self.owner(index))]
        self.class_names = defaultdict(set)
        self.constructors = Counter()
        for index, word in enumerate(self.words[:-1]):
            if word in {"class", "object", "interface"} and name(self.words[index + 1]):
                enclosing = next((owner for owner in self.owners if owner.body == self.scope[index]), None)
                if enclosing or self.scope[index] is None:
                    prefix = self.owner_name(enclosing)
                    short = self.words[index + 1]
                    qualified = ".".join(filter(None, (prefix, short)))
                    self.class_names[short].add(qualified)
                    start, modifiers = self.modifiers(index)
                    if (word == "class" and "abstract" not in modifiers
                            and not (start and self.words[start - 1] in {"enum", "annotation"})):
                        self.constructors[qualified] += 1
        self.member_cache = {}
        self.capture_cache = {}
        self.local_extensions = set()
        self.value_names = defaultdict(set)
        for index, word in enumerate(self.words[:-1]):
            scope = self.scope[index]
            if word in {"fun", "val", "var"}:
                self.value_names[scope].update(declared_names(self, index + 1))
                if scope is not None:
                    end = self.until(index + 1, len(self.tokens), {"(", ":", "=", "{", "}", ";"})
                    if end > index + 3 and self.words[end - 2] == ".":
                        self.local_extensions.add(self.words[end - 1])
        self.inherited_scopes = set()
        for owner in self.owners:
            cursor = owner.start + 2
            while cursor < owner.body:
                if self.words[cursor] == ":":
                    self.inherited_scopes.add(owner.body)
                    break
                cursor = self.pairs.get(cursor, cursor) + 1
        qualified_names = [self.package, *(item for names in self.imports.values() for item in names), *self.stars]
        self.namespace_prefixes = {tuple(parts[:2]) for item in qualified_names
                                   if len(parts := item.split(".")) >= 2}

    def site(self, start, end):
        return Site(self.tokens[start].start, self.tokens[end - 1].end, self.file)

    def owner_name(self, owner):
        names = []
        while owner:
            names.append(owner.name)
            parent = self.scope[owner.start]
            owner = next((outer for outer in self.owners if outer.body == parent), None)
        return ".".join(filter(None, (self.package, *reversed(names))))

    def function_site(self, function):
        return self.site(function.start, function.end + 1)

    def excerpt(self, site):
        return self.text[site.start:site.end]

    def line_break(self, index):
        return "\n" in self.text[self.tokens[index - 1].end:self.tokens[index].start]

    def until(self, start, end, stops, multiline=False):
        continuation = {".", "?.", "?:", "=", "&&", "||", ",", "+", "-", "*", "/", "as", "is", "in",
                        "==", "!=", "===", "!==", "->"}
        index = start
        while index < end:
            word = self.words[index]
            if word in stops or (not multiline and index > start and self.line_break(index)
                                 and word not in continuation and self.words[index - 1] not in continuation):
                break
            index = self.pairs.get(index, index) + 1
        return index

    def modifiers(self, index):
        result = set()
        while index and self.words[index - 1] in MODIFIERS:
            index -= 1
            result.add(self.words[index])
        return index, frozenset(result)

    def owner(self, index):
        following = self.words[index + 1]
        if not name(following):
            return None
        body = self.until(index + 2, len(self.tokens), {"{", "}", "=", ";", *MEMBER}, multiline=True)
        if self.words[body] == "{" and self.scope[body] == self.scope[index]:
            return Owner(following, index, body, self.pairs[body])
        return None

    def qualify(self, spelling, seen=frozenset()):
        if not re.fullmatch(r"[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*", spelling):
            return ""
        head, dot, suffix = spelling.partition(".")
        if head in seen:
            return ""
        aliases = self.aliases.get(head, set())
        imports = self.imports.get(head, set())
        classes = self.class_names.get(head, set())
        if sum(map(len, (aliases, imports, classes))) > 1:
            return ""
        if aliases:
            actual = self.qualify(next(iter(aliases)).split("<", 1)[0].rstrip("?"), seen | {head})
        elif imports or classes:
            actual = next(iter(imports or classes))
        elif dot:
            return spelling
        elif self.stars:
            candidates = {f"{package}.{head}" for package in self.stars}
            if len(candidates) != 1 or not candidates <= KNOWN_INITIALIZERS | self.constructors.keys():
                return ""
            actual = next(iter(candidates))
        else:
            actual = DEFAULT_SYMBOLS.get(head, ".".join(filter(None, (self.package, head))))
        return actual + dot + suffix if actual else ""

    def declared_type(self, tokens):
        return self.qualify("".join(strings(tokens)).rstrip("?").split("<", 1)[0])

    def parameter_type_arguments(self, start, end):
        """Match angle pairs without letting a comparison consume the next declaration."""
        openings, pairs = [], {}
        index = start
        while index < end:
            word = self.words[index]
            if word == "<":
                openings.append(index)
            elif word == ">" and openings:
                pairs[openings.pop()] = index
            elif word in {":", "="}:
                openings.clear()
            index = self.pairs.get(index, index) + 1
        return pairs

    def parameters(self, start, end):
        angles = self.parameter_type_arguments(start, end)
        result = []
        first, colon, default, vararg = start, None, None, False
        index = start
        while index <= end:
            word = self.words[index]
            if index == end or word == ",":
                limit = default if default is not None else index
                if colon is not None and colon > first and name(self.words[colon - 1]):
                    declaration = self.tokens[colon - 1:limit]
                    result.append(ParameterDecl(
                        self.words[colon - 1], self.declared_type(declaration[2:]), token_site(declaration),
                        tuple(self.tokens[limit + 1:index]), vararg,
                    ))
                first, colon, default, vararg = index + 1, None, None, False
            elif default is None:
                if word == "=":
                    default = index
                elif colon is None:
                    # Annotation use-site targets put their own colon before the parameter name.
                    if word == ":" and not (index >= first + 2 and self.words[index - 2] == "@"):
                        colon = index
                    elif (word == "vararg" and self.words[index - 1] not in {"@", "."}
                          and self.words[index + 1] != ":"):
                        vararg = True
            index = self.pairs.get(index, angles.get(index, index)) + 1
        return tuple(result)

    def function(self, index, end):
        start, modifiers = self.modifiers(index)
        params = index + 2
        if not name(self.words[index + 1]) or self.words[params] != "(":
            return None
        close = self.pairs[params]
        body = self.until(close + 1, end, {"{", "=", ";", "}", *MEMBER}, multiline=True)
        kind = self.words[body]
        if kind not in {"{", "="} or self.scope[body] != self.scope[index] or "abstract" in modifiers:
            return None
        if kind == "=":
            if body + 1 >= end:
                return None
            try:
                _, after = self.statement(body + 1, end, 0)
            except Unsupported:
                return None
            close_body = after - 1
        else:
            close_body = self.pairs[body]
        result = self.tokens[close + 2:body] if self.words[close + 1] == ":" else ()
        returns = self.declared_type(result)
        never_returns = returns == "kotlin.Nothing" and strings(result) in {("Nothing",), ("kotlin", ".", "Nothing")}
        return Function(self.words[index + 1], start, body, close_body,
                        self.parameters(params + 1, close), modifiers, kind == "=", self.file, returns, never_returns)

    def http_client_builder(self, initializer, owner):
        """Select the known factory, builder steps and terminal build from source syntax."""
        words = strings(initializer)
        if "(" not in words:
            return ""
        first = self.token_indices[initializer[0].start]
        opening = words.index("(")
        factory = self.qualify("".join(words[:opening]))
        if factory != HTTP_CLIENT + ".newBuilder" or words[opening + 1:opening + 2] != (")",):
            return ""
        head = words[0]
        if any(self.scope[index] in {owner.body, None} and self.words[index + 1] == head
               for index, word in enumerate(self.words[:-1]) if word in {"val", "var", "fun"}):
            return ""
        # A constructor parameter can also shadow the package/type qualifier.
        primary = next((index for index in range(owner.start + 2, owner.body)
                        if self.words[index] in {"(", ":"}), None)
        if primary is not None and self.words[primary] == "(" and any(
                parameter.name == head for parameter in self.parameters(primary + 1, self.pairs[primary])):
            return ""
        position = opening + 2
        while position < len(words):
            if words[position] != "." or words[position + 2:position + 3] != ("(",):
                return ""
            method, arguments = words[position + 1], position + 2
            end = self.pairs.get(first + arguments, -1) - first
            if not arguments < end < len(words):
                return ""
            args = tuple(groups(initializer[arguments + 1:end]))
            if method == "build" and not args and end == len(words) - 1:
                return HTTP_CLIENT
            if method not in HTTP_CLIENT_BUILDER_METHODS or len(args) != 1 or not args[0]:
                return ""
            position = end + 1
        return ""

    def inferred_type(self, initializer, owner):
        if not (call := invocation(initializer)):
            return self.http_client_builder(initializer, owner), False
        callee, args = call
        spelling = "".join(strings(callee))
        qualified = self.qualify(spelling)
        if not qualified:
            return "", False
        head = spelling.split(".", 1)[0]
        scope = {owner.body, None}
        if any(self.scope[index] in scope and self.words[index + 1] == head
               for index, word in enumerate(self.words[:-1]) if word in {"val", "var"}):
            return "", False
        functions = [index for index, word in enumerate(self.words[:-1])
                     if word == "fun" and self.words[index + 1] == head and self.scope[index] in scope]
        if functions:
            if "." not in spelling and not self.stars and not self.imports.get(spelling) and len(functions) == 1:
                if (method := self.function(functions[0], len(self.tokens))) and self.fixed_dispatch(method):
                    if len(method.params) == len(args) and all(not param.vararg for param in method.params):
                        return method.returns, False
            return "", False
        if qualified in KNOWN_INITIALIZERS:
            inferred = FACTORY_RETURNS.get(qualified, qualified)
            return inferred, inferred in MAP_TYPES
        if self.constructors[qualified] == 1:
            return qualified, False
        return "", False

    def members(self, owner):
        functions, fields = {}, {}
        if owner is None:
            return functions, fields
        if owner in self.member_cache:
            return self.member_cache[owner]
        opening = next((index for index in range(owner.start + 2, owner.body)
                        if self.words[index] in {"(", ":"}), None)
        if opening is not None and self.words[opening] == "(":
            for argument in groups(tuple(self.tokens[opening + 1:self.pairs[opening]]), types=True):
                words = strings(argument)
                if words[:2] == ("private", "val") and len(words) >= 5 and words[3] == ":":
                    end = words.index("=") if "=" in words else len(words)
                    field_name = words[2]
                    if name(field_name):
                        fields[field_name] = Field(field_name, token_site(argument), self.declared_type(argument[4:end]), "", True)
        for index in range(owner.body + 1, owner.end):
            if self.container[index] != owner.body:
                continue
            kind = self.words[index]
            if kind == "fun" and name(self.words[index + 1]):
                functions.setdefault(self.words[index + 1], []).append(self.function(index, owner.end))
            if kind not in {"val", "var"} or not name(self.words[index + 1]):
                continue
            start, modifiers = self.modifiers(index)
            field_name = self.words[index + 1]
            if "private" not in modifiers or modifiers & {"open", "override", "abstract", "lateinit"}:
                continue
            end = self.until(index + 2, owner.end, {";", "}", *MEMBER})
            words = self.words[index + 2:end]
            if not words or set(words) & {"by", "get", "set", "->"}:
                continue
            following = end + (self.words[end] == ";")
            while self.words[following] == "@":
                following += 1
                while following < owner.end and (
                        self.words[following] in {".", ":"} or name(self.words[following])
                        and not self.line_break(following)
                        and self.words[following] not in MODIFIERS | MEMBER | {"get", "set"}):
                    following += 1
                if self.words[following] == "(":
                    following = self.pairs[following] + 1
            if self.words[following] in {"get", "set"}:
                continue
            equal = words.index("=") if "=" in words else len(words)
            declared = self.declared_type(self.tokens[index + 3:index + 2 + equal]) if words[0] == ":" else ""
            initializer = tuple(self.tokens[index + 3 + equal:end])
            inferred, standard_map = self.inferred_type(initializer, owner)
            standard_map = standard_map and (not declared or declared in MAP_TYPES)
            storage = ("MAP" if kind == "val" and standard_map
                       else "INSTANCE_FIELD" if kind == "var" else "")
            declaration_end = end if inferred else index + 2 + equal
            fields[field_name] = Field(field_name, self.site(start, declaration_end), declared or inferred, storage,
                                       kind == "val" and "this" not in strings(initializer), bool(inferred and not declared))
        self.member_cache[owner] = functions, fields
        return functions, fields

    def enclosing_owner(self, function):
        scope = self.scope[function.body]
        return next((owner for owner in self.owners if owner.body == scope), None)

    def fixed_dispatch(self, method):
        modifiers = method.modifiers
        if modifiers & {"final", "private"} or not modifiers & {"open", "abstract", "override"}:
            return True
        owner = self.enclosing_owner(method)
        if owner and not modifiers & {"open", "abstract"}:
            _, owner_modifiers = self.modifiers(owner.start)
            return self.words[owner.start] != "interface" and not owner_modifiers & {"open", "abstract", "sealed"}
        return False

    def locate(self, sealed):
        """Locate a unique sealed excerpt at its pinned lines."""
        lines = self.text.split("\n")
        start, end = sealed["startLine"], sealed["endLine"]
        if not 1 <= start <= end <= len(lines):
            return None
        if type(sealed.get("startByte")) is int and type(sealed.get("endByte")) is int:
            raw = self.text.encode()
            first, last = sealed["startByte"], sealed["endByte"]
            if (0 <= first < last <= len(raw) and raw[first:last] == sealed["text"].encode()
                    and raw.count(b"\n", 0, first) + 1 == start and raw.count(b"\n", 0, last - 1) + 1 == end):
                at = len(raw[:first].decode())
                return Site(at, at + len(sealed["text"]), self.file)
            return None
        base = sum(len(line) + 1 for line in lines[:start - 1])
        region = "\n".join(lines[start - 1:end])
        fragment = sealed["text"]
        if fragment.endswith("\n"):
            region += "\n" if end < len(lines) else ""
        found = region.find(fragment)
        if found < 0 or region.find(fragment, found + 1) >= 0:
            return None
        return Site(base + found, base + found + len(fragment), self.file)

    def select(self, sealed):
        """Match a sealed body to a unique enclosing named function."""
        site = self.locate(sealed)
        return self.select_site(site) if site else None

    def select_site(self, site):
        lower, upper = site.start, site.end
        excerpt = self.text[lower:upper]
        first = upper - len(excerpt.lstrip())
        last = lower + len(excerpt.rstrip())
        contained, exact_bodies = [], []
        for index, token in enumerate(self.tokens):
            if token.start >= upper:
                break
            if token.text != "fun" or not (function := self.function(index, len(self.tokens))):
                continue
            end = self.tokens[function.end].end
            if lower <= token.start and end <= upper:
                contained.append(function)
            # A retained full body can omit the function declaration.
            body = function.body + function.expression_body
            if self.tokens[body].start == first and end == last:
                exact_bodies.append(function)
        if exact_bodies:
            return exact_bodies[0] if len(exact_bodies) == 1 else None
        outermost = [function for function in contained if not any(
            other.body < function.body < other.end for other in contained)]
        return outermost[0] if len(outermost) == 1 else None

    def suite(self, start, end, depth=0):
        if depth > 12:
            raise Unsupported()
        result = []
        while start < end:
            if self.words[start] == ";":
                start += 1
                continue
            statement, start = self.statement(start, end, depth)
            result.append(statement)
        return tuple(result)

    def block(self, start, end, depth):
        if start >= end:
            raise Unsupported()
        if self.words[start] == "{":
            close = self.pairs[start]
            try:
                suite = self.suite(start + 1, close, depth)
            except Unsupported:
                suite = (Statement("unsupported", self.site(start, close + 1)),)
            return suite, close + 1
        item, close = self.statement(start, end, depth)
        return (item,), close

    def statement(self, start, end, depth):
        if depth > 12:
            raise Unsupported()
        word = self.words[start]
        after = start + 1
        if word in {"if", "for", "while"} and self.words[after] == "(":
            close = self.pairs[after]
            body, next_ = self.block(close + 1, end, depth + 1)
            alternative = ()
            if word == "if" and self.words[next_] == "else":
                alternative, next_ = self.block(next_ + 1, end, depth + 1)
            return Statement(word, self.site(start, next_), tuple(self.tokens[after + 1:close]),
                             body, alternative), next_
        if word == "try" and self.words[after] == "{":
            body, next_ = self.block(after, end, depth + 1)
            handlers, cleanup = [], ()
            while self.words[next_] == "catch" and self.words[next_ + 1] == "(":
                close = self.pairs[next_ + 1]
                params = self.parameters(next_ + 2, close)
                if len(params) != 1 or self.words[close + 1] != "{":
                    raise Unsupported()
                handler, following = self.block(close + 1, end, depth + 1)
                handlers.append((params[0], self.site(next_, close + 1), handler))
                next_ = following
            if self.words[next_] == "finally" and self.words[next_ + 1] == "{":
                cleanup, next_ = self.block(next_ + 1, end, depth + 1)
            return Statement("try", self.site(start, next_), body=body, handlers=tuple(handlers),
                             cleanup=cleanup), next_
        stop = self.until(start, end, {";", "else", "catch", "finally"})
        if stop <= start:
            raise Unsupported()
        expression_start = (after if word in {"return", "throw"} else
                            next((self.token_indices[group[0].start] for group in
                                  tuple(groups(tuple(self.tokens[start:stop]), "="))[1:] if group), None))
        if expression_start is not None and expression_start < stop and self.words[expression_start] in {"if", "try"}:
            _, stop = self.statement(expression_start, end, depth + 1)
        kind = word if word in {"return", "throw", "break", "continue"} else "expression"
        expression = tuple(self.tokens[start + (kind != "expression"):stop])
        return Statement(kind, self.site(start, stop), expression), stop


@dataclass(frozen=True)
class Callback:
    host: Function
    site: Site
    handoff: Site
    start: int
    end: int
    label: str
    params: tuple


def declared_names(source, index):
    word = source.words[index]
    if name(word):
        return frozenset({word})
    if word == "(":
        components = groups(source.tokens[index + 1:source.pairs[index]], types=True)
        return frozenset(part[0].text.strip("`") for part in components if part)
    if word.startswith("`"):
        return frozenset({word[1:-1]})
    return frozenset()


def token_site(tokens):
    if not tokens:
        raise EmptyTokenSpan("Source span requires at least one token.")
    return Site(tokens[0].start, tokens[-1].end, tokens[0].file)


def encloses(outer, inner):
    return outer.file == inner.file and outer.start <= inner.start < inner.end <= outer.end


def callback_body(source, site, handoff, host):
    opening = source.token_indices.get(site.start)
    end = source.token_ends.get(site.end)
    start = source.token_indices.get(handoff.start)
    if opening is None or end is None or start is None:
        return None
    label = ""
    if source.words[opening + 1:opening + 3] == ("@", "{") and name(source.words[opening]):
        label, opening = source.words[opening], opening + 2
    elif opening >= start + 2 and source.words[opening - 1] == "@" and name(source.words[opening - 2]):
        label = source.words[opening - 2]
    if source.words[opening] != "{" or source.pairs.get(opening) != end:
        return None
    head = source.words[start:opening]
    callee = head[:next((index for index, word in enumerate(head) if word in {"(", "<"}), len(head))]
    if not label and callee and name(callee[-1]):
        label = callee[-1]
    first = opening + 1
    params = (("it", ""),)
    arrow = next((index for index in range(first, end)
                  if source.container[index] == opening and source.words[index] == "->"), None)
    if arrow is not None:
        params = []
        for tokens in groups(source.tokens[first:arrow], types=True):
            words = strings(tokens)
            if not words or not name(words[0]) or len(words) > 1 and (words[1] != ":" or len(words) < 3):
                return None
            params.append((words[0], source.declared_type(tokens[2:])))
        first = arrow + 1
    return Callback(host, site, handoff, first, end, label, tuple(params))
