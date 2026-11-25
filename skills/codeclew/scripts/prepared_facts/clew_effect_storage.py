"""Join selected effect sites and a bounded keyed SQL contract to effect traces."""

import re


PUBLICATION = {"CHANNEL_PUBLICATION", "CHANNEL_PUBLICATION_CANDIDATE"}
EXTERNAL = {"EXTERNAL_REQUEST", "PUBLICATION_CANDIDATE", "DECLARED_EXTERNAL_CALL"}
INTERRUPTED = {"FAULT_OR_CANCELLATION", "CALL_EXCEPTION", "THROWN"}
COMPLETED = {"RESUMED", "RETURNED"}
SQL_ID = r'(?:[A-Za-z_][A-Za-z_0-9]*|"[A-Za-z_][A-Za-z_0-9]*"|\[[A-Za-z_][A-Za-z_0-9]*\]|`[A-Za-z_][A-Za-z_0-9]*`)'
SQL_TABLE = rf"{SQL_ID}(?:\.{SQL_ID})*"
SQL_COLUMN = rf"(?:{SQL_ID}\.)?{SQL_ID}"
SQL_PARAM = r"(?:[@:][A-Za-z_][A-Za-z_0-9]*|\$[1-9][0-9]*|\?)"
SQL_LITERAL = r"(?:true|false|null|-?[0-9]+)"
SQL_VALUE = rf"(?:{SQL_PARAM}|{SQL_LITERAL})"
SQL_ALIAS = rf"(?:\s+(?:AS\s+)?(?P<alias>{SQL_ID}))?"
ASSIGN = re.compile(rf"({SQL_COLUMN})\s*=\s*({SQL_VALUE})", re.I)
TEST = re.compile(rf"(NOT\s+)?({SQL_COLUMN})(?:\s+IS\s+(NOT\s+)?(TRUE|FALSE|NULL))?", re.I)
FIELD = re.compile(rf"({SQL_COLUMN})(?:\s+(?:AS\s+)?({SQL_ID}))?", re.I)
CAST_FIELD = re.compile(rf"(?:(?P<colonColumn>{SQL_COLUMN})\s*::\s*(?P<colonType>{SQL_ID})|"
                        rf"CAST\(\s*(?P<castColumn>{SQL_COLUMN})\s+AS\s+(?P<castType>{SQL_ID})\s*\))"
                        rf"(?:\s+(?:AS\s+)?(?P<alias>{SQL_ID}))?", re.I)
ORDER_FIELD = rf"{SQL_COLUMN}(?:\s+(?:ASC|DESC))?(?:\s+NULLS\s+(?:FIRST|LAST))?"
TAIL = (
    rf"(?:\s+ORDER\s+BY\s+{ORDER_FIELD}(?:\s*,\s*{ORDER_FIELD})*)?"
    r"(?:\s+LIMIT\s+[0-9]+)?"
    r"(?:\s+FOR\s+(?:UPDATE|SHARE)(?:\s+(?:SKIP\s+LOCKED|NOWAIT))?)?"
)
SELECT = re.compile(rf"SELECT\s+(?P<columns>.+?)\s+FROM\s+(?P<table>{SQL_TABLE}){SQL_ALIAS}\s+WHERE\s+(?P<where>.+?){TAIL}", re.I | re.S)
UPDATE = re.compile(rf"UPDATE\s+(?P<table>{SQL_TABLE}){SQL_ALIAS}\s+SET\s+(?P<assignment>.+?)\s+WHERE\s+(?P<where>.+)", re.I | re.S)
DELETE = re.compile(rf"DELETE\s+FROM\s+(?P<table>{SQL_TABLE}){SQL_ALIAS}\s+WHERE\s+(?P<where>.+)", re.I | re.S)
CAST_PARAMETER = re.compile(rf"({SQL_PARAM})\s*::\s*({SQL_TABLE})|"
                            rf"CAST\s*\(\s*({SQL_PARAM})\s+AS\s+({SQL_TABLE})\s*\)", re.I)
AUTHOR_VALUE = rf"(?:{SQL_VALUE}|{CAST_PARAMETER.pattern}|N?'(?:[^']|'')*'|CURRENT_TIMESTAMP|DEFAULT|NOW\(\)|EXCLUDED\.{SQL_ID})"
SQL_FIELDS = rf"{SQL_ID}(?:\s*,\s*{SQL_ID})*"
AUTHOR_SELECT = re.compile(rf"SELECT\s+(?P<columns>.+?)\s+FROM\s+(?P<table>{SQL_TABLE}){SQL_ALIAS}"
                           rf"(?:\s+WHERE\s+(?P<where>.+?))?{TAIL}", re.I | re.S)
INSERT = re.compile(rf"INSERT\s+INTO\s+(?P<table>{SQL_TABLE})\s*\((?P<fields>{SQL_FIELDS})\)"
                    rf"\s*VALUES\s*\((?P<values>{AUTHOR_VALUE}(?:\s*,\s*{AUTHOR_VALUE})*)\)"
                    rf"(?:\s+ON\s+CONFLICT(?:\s*\((?P<conflictKeys>{SQL_FIELDS})\)"
                    rf"|\s+ON\s+CONSTRAINT\s+(?P<constraint>{SQL_ID}))?\s+DO\s+"
                    r"(?P<conflict>NOTHING|UPDATE\s+SET\s+.+))?", re.I | re.S)


def identifier(name):
    # Keep quoted tokens distinct from unquoted tokens.
    return name.lower() if name[0].isascii() and name[0].isalpha() or name[0] == "_" else name


def column(name, qualifier):
    parts = name.split(".")
    if len(parts) > 1 and identifier(parts[0]) != qualifier:
        return None
    return identifier(parts[-1])


def predicate(text, qualifier, assignment=False):
    parts = {}
    for raw in re.split(r"," if assignment else r"\s+AND\s+", text, flags=re.I):
        clause = raw.strip()
        if clause.startswith("(") and clause.endswith(")") and not assignment:
            clause = clause[1:-1].strip()
        if found := ASSIGN.fullmatch(clause):
            name, value = found.groups()
            if not re.fullmatch(SQL_PARAM, value):
                value = value.lower()
                if value == "null" and not assignment:
                    return None
        elif not assignment and (found := TEST.fullmatch(clause)):
            leading_not, name, trailing_not, tested = found.groups()
            if leading_not and tested:
                return None
            value = (tested or ("false" if leading_not else "true")).lower()
            if trailing_not:
                # IS NOT [TRUE|FALSE] also accepts NULL.
                value = "not-" + value
        else:
            return None
        resolved = column(name, qualifier)
        if resolved is None or resolved in parts:
            return None
        parts[resolved] = value
    return parts


def top_level(text):
    """Yield words and commas outside SQL strings, quotes and parenthesized expressions."""
    depth, quote, offset = 0, None, 0
    while offset < len(text):
        char = text[offset]
        if quote:
            if char == quote:
                if text[offset + 1:offset + 2] == quote:
                    offset += 2
                    continue
                quote = None
        elif char in {"'", '"', "`", "["}:
            quote = "]" if char == "[" else char
        elif text[offset:offset + 2] in {"--", "/*"} or char == ";":
            raise ValueError
        elif char == "(":
            depth += 1
        elif char == ")":
            depth -= 1
            if depth < 0:
                raise ValueError
        elif depth == 0 and char == ",":
            yield ",", offset, offset + 1
        elif depth == 0 and (char.isalpha() or char == "_"):
            word = re.match(r"[A-Za-z_][A-Za-z_0-9]*", text[offset:])
            if word:
                end = offset + len(word[0])
                yield word[0].upper(), offset, end
                offset = end
                continue
        offset += 1
    if quote is not None or depth:
        raise ValueError


def select_parts(statement):
    prefix = re.match(r"SELECT\s+", statement, re.I)
    if prefix is None:
        return statement, None
    try:
        start = next((start for word, start, _ in top_level(statement) if word == "FROM"), None)
    except ValueError:
        return statement, None
    if start is not None:
        return "SELECT projection " + statement[start:], statement[prefix.end():start]
    return statement, None


def opaque_projection(field):
    tokens = list(top_level(field))
    aliases = [end for word, _, end in tokens if word == "AS"]
    alias = field[aliases[-1]:].strip() if aliases else ""
    return {"column": None, "name": identifier(alias) if re.fullmatch(SQL_ID, alias) else None}


def projections(text, qualifier, *, retain_cast=False):
    if text is None:
        return None
    if text.strip() == "*" or text.strip().endswith(".*") and identifier(text.strip()[:-2]) == qualifier:
        return [{"column": "*", "name": "*"}]
    try:
        ends = [start for word, start, _ in top_level(text) if word == ","] + [len(text)]
        result, start = [], 0
        for end in ends:
            field = text[start:end].strip()
            start = end + 1
            if not field or field == "*" or re.search(r"\.\s*\*(?:\s+AS\s+.*)?$", field, re.I):
                return None
            found = FIELD.fullmatch(field)
            if found and (resolved := column(found[1], qualifier)) is not None:
                result.append({"column": resolved, "name": identifier(found[2]) if found[2] else resolved})
            elif retain_cast and (cast := CAST_FIELD.fullmatch(field)) and (
                    resolved := column(cast["colonColumn"] or cast["castColumn"], qualifier)) is not None:
                result.append({"column": None, "name": identifier(cast["alias"]) if cast["alias"] else None,
                               "sourceColumn": resolved, "cast": identifier(cast["colonType"] or cast["castType"])})
            else:
                result.append(opaque_projection(field))
        return result
    except ValueError:
        return None


def sql_contract(sql, tables):
    """Recognize a single-table select or mutation with simple state predicates."""
    statement = sql.strip().removesuffix(";").strip()
    if len(statement) > 1024 or len(tables(statement)) != 1:
        return None
    structural, selected_fields = select_parts(statement)
    for verb, grammar in (("SELECT", SELECT), ("UPDATE", UPDATE), ("DELETE", DELETE)):
        match = grammar.fullmatch(structural)
        if not match:
            continue
        table = ".".join(identifier(part) for part in match["table"].split("."))
        qualifier = identifier(match["alias"]) if match["alias"] else table.rsplit(".", 1)[-1]
        where = predicate(match["where"], qualifier)
        if not where:
            return None
        keys = [(name, value) for name, value in where.items() if re.fullmatch(SQL_PARAM, value)]
        assigned = predicate(match["assignment"], qualifier, assignment=True) if verb == "UPDATE" else {}
        selected = projections(selected_fields, qualifier) if verb == "SELECT" else []
        if (len(keys) > 1 or not keys and verb != "SELECT" or assigned is None or selected is None
                or any(re.fullmatch(SQL_PARAM, value) for value in assigned.values())):
            return None
        key_column, marker = keys[0] if keys else (None, None)
        return {"verb": verb, "table": table, "column": key_column,
                "parameter": marker[1:] if marker and marker[0] in "@:" else None,
                "parameterPosition": (int(marker[1:]) - 1 if marker.startswith("$") else 0) if marker and marker[0] in "$?" else None,
                "columns": [field["column"] for field in selected], "projections": selected,
                "predicate": {name: value for name, value in where.items() if name != key_column},
                "assignment": assigned}
    return None


def author_sql_contract(sql):
    """Describe simple SQL configuration without broadening the durability contract."""
    statement = sql.strip().removesuffix(";").strip()
    if len(statement) > 1024:
        return None
    try:
        tokens = list(top_level(statement))
    except ValueError:
        return None
    returned = None
    if tokens and tokens[0][0] in {"INSERT", "UPDATE", "DELETE"}:
        split = next(((start, end) for word, start, end in tokens if word == "RETURNING"), None)
        if split:
            returned = statement[split[1]:].strip()
            statement = statement[:split[0]].rstrip()
    structural, selected_fields = select_parts(statement)
    for verb, grammar in (("SELECT", AUTHOR_SELECT), ("UPDATE", UPDATE), ("DELETE", DELETE), ("INSERT", INSERT)):
        if not (match := grammar.fullmatch(structural)):
            continue
        table = ".".join(identifier(part) for part in match["table"].split("."))
        table_name = table.rsplit(".", 1)[-1]
        qualifier = identifier(match["alias"]) if verb != "INSERT" and match["alias"] else table_name
        selected = projections(selected_fields if verb == "SELECT" else returned, qualifier, retain_cast=True) if (
            verb == "SELECT" or returned is not None) else []
        if selected is None:
            return None
        result = {"verb": verb, "table": table, "where": {}, "assignment": {},
                  "columns": [field["column"] for field in selected if field["column"]], "projections": selected}
        if returned is not None:
            result["returning"] = True
        if verb == "INSERT":
            fields = [identifier(item.strip()) for item in match["fields"].split(",")]
            values = split_sql_commas(match["values"])
            if len(set(fields)) != len(fields) or len(fields) != len(values):
                return None
            result["assignment"] = dict(zip(fields, values))
            casts = {}
            for field, value in result["assignment"].items():
                if cast := CAST_PARAMETER.fullmatch(value):
                    postfix_parameter, postfix_type, standard_parameter, standard_type = cast.groups()
                    casts[field] = {"parameter": postfix_parameter or standard_parameter,
                                    "type": postfix_type or standard_type}
            if casts:
                result["assignmentCasts"] = casts
            if match["conflict"]:
                result["conflict"] = {"action": short_clause(match["conflict"]),
                                      "keys": split_sql_commas(match["conflictKeys"]) if match["conflictKeys"] else [],
                                      "constraint": match["constraint"]}
            return result
        if match["where"]:
            result["where"] = predicate(match["where"], qualifier) or {}
            if not result["where"]:
                result["filter"] = short_clause(match["where"])
        if verb == "UPDATE":
            result["assignment"] = predicate(match["assignment"], qualifier, assignment=True) or {}
            if not result["assignment"]:
                result["set"] = short_clause(match["assignment"])
        if verb == "SELECT":
            tail_start = next((start for start in (match.end("where"), match.end("alias"), match.end("table")) if start >= 0), None)
            if tail := short_clause(structural[tail_start:]):
                result["tail"] = tail
        return result
    return None


def split_sql_commas(text):
    commas = [start for word, start, _ in top_level(text) if word == ","]
    return [text[start + 1:end].strip() for start, end in zip([-1, *commas], [*commas, len(text)])]


def short_clause(text):
    text = text.strip()
    return text if len(text) <= 260 else None


def clause_parameters(text):
    # Top-level words omit quoted strings and nested expressions. Keep markers
    # attested by this scan and leave hidden markers unassociated.
    for token, start, end in top_level(text or ""):
        if token == "," or start < 1 or text[start - 1] not in "@:" or start >= 2 and text[start - 2] == ":":
            continue
        yield text[start - 1:end]


def command_parameters(source_sql):
    casts = source_sql.get("assignmentCasts", {})
    assignments = (casts[field]["parameter"] if field in casts else value
                   for field, value in source_sql["assignment"].items())
    markers = [value for value in (*source_sql["where"].values(), *assignments) if re.fullmatch(SQL_PARAM, value)]
    markers.extend(value for text in (source_sql.get("filter"), source_sql.get("set"),
                                      source_sql.get("conflict", {}).get("action")) for value in clause_parameters(text))
    # Indexing a mixed placeholder list would require the provider's rewrite rules.
    if casts and len({marker[0] if marker[0] in "$?" else "named" for marker in markers}) > 1:
        return
    for value in dict.fromkeys(markers):
        if value.startswith(("@", ":")):
            yield value, value[1:]
        elif value.startswith("$"):
            yield value, int(value[1:]) - 1
        elif markers.count(value) == 1:
            yield value, 0


def source_roles(process, evidence, citations, functions, locate, sql_tables):
    """Use unique byte containment to attach sealed call and topic evidence."""
    calls = [(ref, fn, op) for ref, fn in functions.items()
             for block in fn.graph["blocks"] for op in block["steps"] if op["kind"] == "CALL"]
    result = {}

    def attach(refs, role, authority, topic=None):
        for source_ref in refs:
            source = evidence.get(source_ref, {})
            if not source or source.get("peerProcess") or source.get("peerService"):
                continue
            matched = []
            for ref, fn, op in calls:
                if source["file"] != fn.source["file"] or source["revision"] != fn.source["revision"]:
                    continue
                interval = locate(fn.blob, source)
                if not interval:
                    continue
                start, end = op["site"]["start"], op["site"]["end"]
                if not (start <= interval[0] < interval[1] <= end or interval[0] <= start < end <= interval[1]):
                    continue
                if topic is not None and not any((arg.get("value") or {}).get("value") == topic
                                                 for arg in op["arguments"]):
                    continue
                matched.append((ref, op["id"]))
            if len(matched) == 1:
                result.setdefault(matched[0], {"role": role, "roleAuthority": authority, "sources": [citations[source_ref]],
                                                **({"topic": topic} if topic is not None else {})})

    for relation in process.get("connections", ()):
        if relation.get("direction") == "outgoing":
            attach(relation.get("sourceReferences", ()), "DECLARED_EXTERNAL_CALL",
                   relation.get("authority", "SEALED_OUTGOING_CALL"))
    for thread in process.get("outboxThreads", ()):
        for stage in thread.get("stages", ()):
            if stage.get("process") == process["id"] and stage.get("kind") in PUBLICATION and thread.get("topic"):
                attach(stage.get("anchorReferences") or stage.get("sourceReferences", ()),
                       "PUBLICATION_CANDIDATE", stage["kind"], thread["topic"])

    for ref, fn, op in calls:
        if op["role"] or (ref, op["id"]) in result or op.get("binding") or op.get("spelling") == "CreateCommand":
            continue
        contracts = []
        for arg in op["arguments"]:
            value = arg.get("value") or {}
            if value.get("kind") == "literal" and isinstance(value.get("value"), str):
                if source_sql := author_sql_contract(value["value"]):
                    contracts.append((source_sql, sql_contract(value["value"], sql_tables)))
        if len(contracts) == 1:
            source_sql, contract = contracts[0]
            result[(ref, op["id"])] = {"sourceSql": source_sql, "sources": [fn.citation]}
            if contract:
                result[(ref, op["id"])].update(
                    role="SQL_READ_CANDIDATE" if contract["verb"] == "SELECT" else "SQL_WRITE_CANDIDATE",
                    roleAuthority="RETAINED_SQL_CALL_ARGUMENT", sql=contract)
    return result


def event_calls(sequence, steps):
    invocations = {}
    for index, event in enumerate(sequence):
        key = event["step"]
        if event["outcome"] == "INVOKED":
            invocations[key] = event
        if call := steps.get(event.get("call")):
            yield index, call, event, invocations.get(call["id"], event)


def tracked(value):
    return value and value[0] not in {"unbound-name", "unbound-member", "open-member", "literal"}


def bound_argument(event, name):
    values = [argument["value"] for argument in event.get("arguments", ()) if argument.get("name") == name]
    if len(values) == 1 and tracked(values[0]):
        return values[0]
    return None


def sql_argument(step, event, name):
    """Project an explicitly bound SQL parameter, including a retained Dapper parameter record."""
    if name is None:
        return None
    if not step.get("dapperCandidate"):
        return bound_argument(event, name)
    args = event.get("arguments", ())
    supplied = [arg["value"] for arg in args if arg.get("name") == "param" or
                not arg.get("name") and arg["position"] == 1]
    if len(supplied) != 1 or not supplied[0] or supplied[0][0] != "record":
        return None
    matches = [value for field, value in supplied[0][1] if field.casefold() == name.casefold()]
    return matches[0] if len(matches) == 1 and tracked(matches[0]) else None


def positional_key(event, transaction):
    candidates = [(argument["position"], argument["value"]) for argument in event.get("arguments", ())
                  if not argument.get("name") and tracked(argument["value"]) and argument["value"] != transaction]
    return candidates[0] if len(candidates) == 1 else (None, None)


def transaction_argument(event, transaction):
    named = bound_argument(event, "transaction")
    if named is not None:
        return "NAMED" if named == transaction else None
    values = [arg for arg in event.get("arguments", ()) if not arg.get("name") and arg["value"] == transaction]
    return "POSITIONAL" if len(values) == 1 else None


def contains_result(value, call):
    return isinstance(value, tuple) and (value[:2] == ("call-result", call) or
                                         any(contains_result(part, call) for part in value))


def projected_key(key, select, write, read):
    if key is None or key[0] != "member" or not contains_result(key[2], read["id"]):
        return False
    column = write["column"]
    names = [field["name"] for field in select["projections"] if field["column"] == column]
    if select["columns"] == ["*"]:
        names = [column]
    member = key[1].rsplit(".", 1)[-1].casefold()
    return len(names) == 1 and member == names[0].strip('"[]`').casefold()


def sql_pair(read, read_inv, write, write_inv, transaction, commit_authority):
    select, mutate = read["sql"], write["sql"]
    if (select["table"] != mutate["table"] or select["column"] not in {None, mutate["column"]}
            or mutate["parameterPosition"] is not None or select["parameterPosition"] is not None):
        return None
    receiver = read_inv.get("receiver")
    if not tracked(receiver) or receiver != write_inv.get("receiver"):
        return None
    roles = transaction_argument(read_inv, transaction), transaction_argument(write_inv, transaction)
    if None in roles or commit_authority == "CONDITIONAL_TRANSACTION_COMPLETION" and roles != ("NAMED", "NAMED"):
        return None
    if any(select["predicate"].get(name) != value for name, value in mutate["predicate"].items()):
        return None
    if mutate["verb"] != "DELETE" and not any(
            name in mutate["assignment"] and mutate["assignment"][name] != value
            for name, value in select["predicate"].items()):
        return None
    key = sql_argument(write, write_inv, mutate["parameter"])
    if select["parameter"] is None:
        if key is None and not write.get("dapperCandidate"):
            _, key = positional_key(write_inv, transaction)
        return (receiver, key, "SELECTED_RECORD_MEMBER") if projected_key(key, select, mutate, read) else None
    if key is not None and key == sql_argument(read, read_inv, select["parameter"]):
        return receiver, key, "NAMED"
    if read.get("dapperCandidate") or write.get("dapperCandidate"):
        return None
    read_position, prior = positional_key(read_inv, transaction)
    write_position, key = positional_key(write_inv, transaction)
    if key is not None and key == prior and read_position == write_position:
        return receiver, key, "POSITIONAL"
    return None


def commit_operation(step, invoked):
    receiver = invoked.get("receiver")
    if not tracked(receiver):
        return None
    args = invoked.get("arguments", ())
    token = len(args) == 1 and args[0].get("cancellationToken") and not args[0].get("byRef")
    if step["role"] == "TRANSACTION_COMMIT_REQUEST":
        return receiver, "ROSLYN_FRAMEWORK_DECLARATION", "FRAMEWORK_CANCELLATION_TOKEN" if token else "FRAMEWORK_PARAMETERLESS"
    if step["role"] is not None or step["dispatch"] != "OPEN" or step["symbol"]:
        return None
    if step["spelling"] == "CommitAsync" and token:
        return receiver, "CONDITIONAL_TRANSACTION_COMPLETION", "CANCELLATION_TOKEN_ARGUMENT_PROVIDER_SIGNATURE_OPEN"
    if step["spelling"] in {"Commit", "CommitAsync"} and not args:
        return receiver, "CONDITIONAL_TRANSACTION_COMPLETION", "PROVIDER_SIGNATURE_OPEN"
    return None


def commit_argument_evidence(invocation):
    arguments = invocation.get("arguments", ())
    return arguments[0].get("cancellationTokenEvidence") if len(arguments) == 1 else None


def uses_value(value, origin):
    return value == origin or isinstance(value, tuple) and any(uses_value(part, origin) for part in value)


def read_gate(path, comparisons, row, effect, after_read, effect_index):
    sequence = path["sequence"]
    for index in range(after_read + 1, effect_index):
        event = sequence[index]
        if event["outcome"] not in {"TRUE", "FALSE"}:
            continue
        truth = event["outcome"] == "TRUE"
        if not any(condition["step"] == event["step"] and condition["value"] == truth and
                   uses_value(condition["operand"], row) for condition in path["conditions"]):
            continue
        for other in comparisons:
            route = other["sequence"]
            if (other["outcome"] in {"RETURNED", "NORMAL_EXIT"} and len(route) > index
                    and route[:index] == sequence[:index] and route[index]["step"] == event["step"]
                    and route[index]["outcome"] != event["outcome"]
                    and not any(item["step"] == effect for item in route[index:])):
                return {"step": event["step"], "when": truth}
    return None


def row_origin(value, row):
    if value == row:
        return True
    if not value or not isinstance(value, tuple):
        return False
    if value[0] == "member":
        return row_origin(value[2], row)
    if value[0] == "tuple":
        return any(row_origin(item, row) for item in value[1])
    if value[0] == "record":
        return any(row_origin(item, row) for _, item in value[1])
    return False


def publication_values(invocation, sequence):
    """Read evaluated arguments and the latest fields of a non-escaped inline object."""
    values = [arg["value"] for arg in invocation.get("arguments", ())]
    allocations = {value for value in values if value and value[0] == "allocation"}
    fields = {}
    for event in sequence:
        if event["outcome"] == "INVOKED":
            if event.get("result") == invocation.get("result") and event["step"] == invocation["step"]:
                break
            escaped = {event.get("receiver"), *(item["value"] for item in event.get("arguments", ()))} & allocations
            if escaped:
                fields = {key: value for key, value in fields.items() if key[0] not in escaped}
        if event["outcome"] in {"COMPLETION_OPEN", "WRITE_TARGET_OPEN"}:
            fields.clear()
        if event["outcome"] not in {"STATE_WRITE_REACHED", "CANDIDATE_INITIALIZER_REACHED", "RESOURCE_CONFIGURATION_REACHED"}:
            continue
        target = event["resource"]
        if len(target) >= 3 and target[0] in {"member", "object-initializer-member", "ado-member"} and target[2] in allocations:
            name = target[1].rsplit(".", 1)[-1]
            fields[target[2], name] = (name, event.get("sourceValue", event["value"]), event["step"])
    return [(None, value, None) for value in values], list(fields.values())


def publication_arguments(invocation, sequence, row):
    direct, fields = publication_values(invocation, sequence)
    proof = [site for _, value, site in fields if row_origin(value, row)]
    return proof if proof or any(row_origin(value, row) for _, value, _ in direct) else None


def returned_connection(steps, sequence, receiver):
    """Keep the selected connection origin, including allocation followed by a completed open."""
    if not receiver:
        return []
    returns = [(index, event) for index, event in enumerate(sequence)
               if event.get("composed") and event.get("returnedValue") == receiver]
    if receiver[0] == "call-result":
        origin = steps.get(receiver[1], {})
        if origin.get("candidateEffectFamily") != "database" or origin.get("spelling") not in {
                "OpenConnection", "OpenConnectionAsync"}:
            return []
        return [origin["id"], *(key for _, event in returns for key in (event["call"], event["step"]))]
    if receiver[0] != "allocation" or not returns:
        return []
    allocated = next(((index, event["step"]) for index, event in enumerate(sequence)
                      if event["outcome"] == "CONSTRUCTED" and event.get("result") == receiver), None)
    if not allocated:
        return []
    start, construction = allocated
    origin = steps[construction]
    if origin.get("resourceFamily") != "connection" and origin.get("candidateEffectFamily") != "database":
        return []
    opened = None
    for index, call, completed, invoked in event_calls(sequence, steps):
        if index <= start or invoked.get("receiver") != receiver:
            continue
        if call.get("spelling") in {"Close", "CloseAsync", "Dispose", "DisposeAsync"}:
            opened = None
        elif (call.get("spelling"), completed["outcome"]) in {("Open", "RETURNED"), ("OpenAsync", "RESUMED")} and (
                call.get("receiverFamily") == "connection" or call.get("candidateEffectFamily") == "database"):
            opened = index, call["id"], completed["step"]
    if opened:
        handoff = [key for index, event in returns if index > opened[0] for key in (event["call"], event["step"])]
        if handoff:
            return [construction, *opened[1:], *handoff]
    return []


def transaction_begin(calls, receiver, transaction, before, returned):
    for index, operation, completion, invocation in calls:
        if (index >= before or completion["outcome"] not in COMPLETED or invocation["result"] != transaction or
                invocation["receiver"] != receiver):
            continue
        if operation["role"] == "TRANSACTION_BEGIN_REQUEST" or (
                operation["spelling"] in {"BeginTransaction", "BeginTransactionAsync"} and
                (operation.get("candidateEffectFamily") == "database" or
                 returned and operation.get("dispatch") == "OPEN" and not operation.get("symbol"))):
            return [operation["id"], completion["step"]]
    return []


def durable_candidates(paths, steps, budget):
    """Use a common path prefix and the matching write and commit to qualify repeat delivery."""
    found = {}
    routes = [(path, [(i, call, event, invocation) for i, call, event, invocation in event_calls(path["sequence"], steps)
                      if i >= path.get("scope", {}).get("prefixEvents", 0)]) for path in paths
              if "NULL_ACCEPTING_NON_INSTANCE_DISPATCH_REQUIRED" not in path["open"]]
    for path, calls in routes:
        sequence = path["sequence"]
        prefix_events = path.get("scope", {}).get("prefixEvents", 0)
        interruptions = [(index, event) for index, event in enumerate(sequence) if index >= prefix_events and
                         event["outcome"] in INTERRUPTED]
        for cut, interruption in interruptions:
            for j, external, event, invocation in calls:
                ack_failure = (j == cut and event["outcome"] == "FAULT_OR_CANCELLATION" and
                               external["role"] == "PUBLICATION_CANDIDATE")
                if external["role"] not in EXTERNAL or not (j < cut and event["outcome"] in COMPLETED or ack_failure):
                    continue
                prefix_end = j if ack_failure else j + 1
                for i, read, read_event, read_inv in calls:
                    if (i >= j or read["role"] != "SQL_READ_CANDIDATE" or read_event["outcome"] not in COMPLETED or
                            not (guard := read_gate(path, paths, read_inv["result"], external["id"], i, j))):
                        continue
                    payload = publication_arguments(invocation, sequence[i:j], read_inv["result"])
                    if payload is None and (ack_failure or path.get("scope", {}).get("rootedWorker")):
                        continue
                    for normal, later in routes:
                        route = normal["sequence"]
                        if route[:prefix_end] != sequence[:prefix_end]:
                            continue
                        if ack_failure and not (len(route) > j and route[j]["step"] == event["step"] and
                                                route[j]["outcome"] == "RESUMED"):
                            continue
                        for write_index, write, write_event, write_inv in later:
                            if write_index <= j or write["role"] != "SQL_WRITE_CANDIDATE" or write_event["outcome"] not in COMPLETED:
                                continue
                            for k, commit, done, invoked in later:
                                if k <= write_index or done["outcome"] not in COMPLETED or not (binding := commit_operation(commit, invoked)):
                                    continue
                                tx, authority, signature = binding
                                if not (matched := sql_pair(read, read_inv, write, write_inv, tx, authority)):
                                    continue
                                if any(mark["outcome"] in COMPLETED and (observed := commit_operation(action, applied))
                                       and observed[0] == tx for _, action, mark, applied in calls):
                                    continue
                                receiver, key, key_mode = matched
                                if key_mode == "SELECTED_RECORD_MEMBER" and key[2] != read_inv["result"]:
                                    continue
                                commit_failure = (interruption.get("call") == commit["id"] and k == cut and
                                                  route[:cut] == sequence[:cut] and done["step"] == interruption["step"])
                                publication = external["role"] == "PUBLICATION_CANDIDATE"
                                connection_proof = returned_connection(steps, route[:i], receiver)
                                begin = transaction_begin(later, receiver, tx, i, connection_proof)
                                if (ack_failure or commit_failure and publication) and not begin:
                                    continue
                                fact = (read["id"], external["id"], interruption["step"], write["id"], commit["id"], ack_failure)
                                callers = [item["step"] for item in route[:i] if item["outcome"] == "INVOKED" and
                                           read["id"].split("/", 1)[0] in steps[item["step"]].get("boundSources", ())]
                                proof = list(dict.fromkeys([*callers, *connection_proof, *begin, guard["step"], *fact[:5],
                                                           *(payload or ()), route[j]["step"], write_event["step"], done["step"]]))
                                token = commit_argument_evidence(invoked)
                                sources = {source for step in proof for source in steps[step]["sources"]}
                                if token:
                                    sources.update(token["sources"])
                                found.setdefault(fact, {
                                    "kind": "POSSIBLE_DURABLE_RESELECTION", "select": read["id"],
                                    "selectTable": read["sql"]["table"], "keyColumn": write["sql"]["column"],
                                    "key": key, "keyBinding": key_mode, "receiver": receiver, "transaction": tx,
                                    "externalCall": external["id"], "effectAuthority": external["roleAuthority"],
                                    "completedAt": route[j]["step"], "interruption": interruption["step"],
                                    "matchingWrite": write["id"], "matchingCommit": commit["id"],
                                    "commitAuthority": authority, "commitSignature": signature,
                                    "publicationAwaitFailed": ack_failure, "matchingCommitFailed": commit_failure,
                                    "selectedRowSuppliesArguments": payload is not None, "rowGuard": guard,
                                    **({"connectionEvidence": connection_proof} if connection_proof else {}),
                                    "failurePath": path["id"], "comparisonPath": normal["id"],
                                    "evidenceSteps": proof,
                                    "reentry": "IF_LATER_INVOCATION_REACHES_SELECT",
                                    "premises": ["The provider executes the retained SQL with the projected key and transaction arguments.",
                                                 "A later select encounters that same still-eligible row.",
                                                 "Runtime delivery, durability and rollback remain open."],
                                    **({"commitArgumentEvidence": token} if token else {}),
                                    "sources": sorted(sources),
                                })
                                if len(found) >= budget:
                                    return list(found.values())
    return list(found.values())
