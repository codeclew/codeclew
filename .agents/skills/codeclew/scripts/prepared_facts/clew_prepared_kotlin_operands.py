"""Describe retained Kotlin call arguments and bounded literal SQL shapes."""

import json
from pathlib import Path
import re
import runpy


HERE = Path(__file__).parent
SOURCE = runpy.run_path(str(HERE / "clew_source_candidates.py"))
SQL = runpy.run_path(str(HERE / "clew_effect_storage.py"))
INTERPOLATION = re.compile(r"(?<!\\)\$(?:\{([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)\}|([A-Za-z_]\w*))")
RECEIVER = re.compile(r"[A-Za-z_]\w*(?:(?:\?\.|\.)[A-Za-z_]\w*)*\Z")
SQL_STRING = re.compile(r"'(?:''|[^'])*'|\"(?:\"\"|[^\"])*\"|`[^`]*`|\s+")
MODIFIERS = ("WHERE", "GROUP BY", "HAVING", "ORDER BY", "LIMIT", "OFFSET",
             "FOR UPDATE", "FOR SHARE", "ON CONFLICT", "RETURNING")


def code(value):
    if "`" in value:
        return "``" + value + "``"
    return "`" + value + "`"


def flat(tokens, text):
    """Flatten Kotlin layout without changing strings or joining token spellings."""
    if not tokens or any("\n" in token.text or "\r" in token.text for token in tokens):
        return None
    result = [tokens[0].text]
    for left, right in zip(tokens, tokens[1:]):
        if text[left.end:right.start]:
            result.append(" ")
        result.append(right.text)
    return "".join(result)


def sql_literal(tokens):
    if not tokens:
        return None
    token = tokens[0]
    suffix = tuple(item.text for item in tokens[1:])
    if suffix not in {(), (".", "trimIndent", "(", ")"), (".", "trimMargin", "(", ")")}:
        return None
    if token.text.startswith('"""') and token.text.endswith('"""'):
        text = token.text[3:-3]
        if suffix[1:2] == ("trimMargin",):
            text = "\n".join(re.sub(r"^\s*\|", "", line) for line in text.splitlines())
    elif token.text.startswith('"'):
        try:
            text = json.loads(token.text.replace(r"\$", "$"))
        except ValueError:
            return None
    else:
        return None
    return token, text


def collapse_sql(sql):
    return SQL_STRING.sub(lambda match: " " if match[0].isspace() else match[0], sql).strip()


def sql_shape(raw):
    """Read single-statement SQL operands. Keep Kotlin interpolation symbolic."""
    if len(raw) > 4096:
        return None
    symbols = {}

    def hide(match):
        key = f"__clew_kotlin_value_{len(symbols)}__"
        if key in raw:
            return match[0]
        symbols[key] = match[0]
        return key

    def visible(text):
        for key, spelling in symbols.items():
            text = text.replace(key, spelling)
        return collapse_sql(text)

    query = INTERPOLATION.sub(hide, raw).strip().removesuffix(";").strip()
    # Other interpolation would make it unclear which statement shape reaches the provider.
    if "${" in query or not re.match(r"(?i)^(SELECT|INSERT|UPDATE|DELETE)\b", query):
        return None
    try:
        top = list(SQL["top_level"](query))
    except ValueError:
        return None
    tables = SOURCE["sql_tables"](query, normalize=lambda name: collapse_sql(name))
    if len(tables) != 1 or any(word in {"UNION", "INTERSECT", "EXCEPT", "JOIN"} for word, _, _ in top):
        return None
    verb, table = tables[0]
    if verb == "SELECT":
        source = next((start for word, start, _ in top if word == "FROM"), None)
        if source is None or not query[len(verb):source].strip():
            return None
        items = [("selects", visible(query[len(verb):source]))]
    else:
        items = []
    clauses = []
    for index, (word, start, end) in enumerate(top):
        marker = next((key for key in MODIFIERS if key == word or key.startswith(word + " ")
                       and index + 1 < len(top) and top[index + 1][0] == key.split()[1]
                       and not query[end:top[index + 1][1]].strip()), None)
        if not marker:
            continue
        offset = end if marker == word else top[index + 1][2]
        clauses.append((marker, start, offset))
    conflict = next((index for index, item in enumerate(clauses) if item[0] == "ON CONFLICT"), None)
    if conflict is not None:
        clauses = [clause for index, clause in enumerate(clauses)
                   if index <= conflict or clause[0] == "RETURNING"]
    labels = {"WHERE": "filters by", "ORDER BY": "orders by", "LIMIT": "limits by", "OFFSET": "offsets by",
              "RETURNING": "requests returning", "ON CONFLICT": "specifies conflict handling",
              "FOR UPDATE": "requests", "FOR SHARE": "requests", "GROUP BY": "groups by", "HAVING": "filters groups by"}
    if verb in {"INSERT", "UPDATE"}:
        marker = "VALUES" if verb == "INSERT" else "SET"
        head = next(((start, end) for word, start, end in top if word == marker), None)
        if not head:
            return None
        start, end = head
        next_clause = next((point for _, point, _ in clauses if point > start), len(query))
        if verb == "INSERT":
            target = re.match(r"(?is)^INSERT(?:\s+OR\s+\w+)?\s+INTO\s+" + SOURCE["TABLE"], query)
            columns = query[target.end():start].strip() if target else ""
            if columns:
                items.append(("lists columns", visible(columns)))
            items.append(("supplies values", visible(query[end:next_clause])))
        else:
            items.append(("sets", visible(query[end:next_clause])))
    for index, (marker, start, end) in enumerate(clauses):
        next_start = clauses[index + 1][1] if index + 1 < len(clauses) else len(query)
        operand = visible(query[start:next_start] if marker in {"FOR UPDATE", "FOR SHARE"} else query[end:next_start])
        if operand:
            items.append((labels[marker], operand))
    return verb, visible(table), tuple(items)


def sql_sentence(shape):
    verb, table, details = shape
    clauses = [f"{label} {code(value)}" if len(value) <= 210 else f"{label} a longer cited expression"
               for label, value in details]
    return f"{verb} for {code(table)}" + (", " + ", ".join(clauses) if clauses else "")


def raw_sql(raw):
    """Retain source operands when structured SQL binding exceeds the grammar."""
    query = raw.strip().removesuffix(";").strip()
    if len(query) > 4096 or not (verb := re.match(r"(?i)^(SELECT|INSERT|UPDATE|DELETE)\b", query)):
        return None
    symbolic = INTERPOLATION.sub("__clew_source_symbol__", query)
    if "${" in symbolic:
        return None
    try:
        words = {word for word, _, _ in SQL["top_level"](symbolic)}
    except ValueError:
        return None
    operation = verb[1].upper()
    required = {"SELECT": {"FROM"}, "INSERT": {"INTO"}, "UPDATE": {"SET"}, "DELETE": {"FROM"}}
    if not required[operation] <= words:
        return None
    normalized = collapse_sql(query)
    if len(normalized) > 1400:
        return None
    tables = SOURCE["sql_tables"](query, normalize=collapse_sql)
    label = operation + (" for " + ", ".join(code(table) for _, table in tables[:3]) if tables else " literal")
    return label, code(normalized)
