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
    return key[1].rsplit(".", 1)[-1].casefold() == column.casefold() and (
        select["columns"] == ["*"] or column in select["columns"])


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
    key = bound_argument(write_inv, mutate["parameter"])
    if select["parameter"] is None:
        if key is None:
            _, key = positional_key(write_inv, transaction)
        return (receiver, key, "SELECTED_RECORD_MEMBER") if projected_key(key, select, mutate, read) else None
    if key is not None and key == bound_argument(read_inv, select["parameter"]):
        return receiver, key, "NAMED"
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


def read_gates_effect(path, comparisons, read, effect, effect_index):
    for index, event in enumerate(path["sequence"][:effect_index]):
        if event["outcome"] not in {"TRUE", "FALSE"}:
            continue
        if not any(condition["step"] == event["step"] and contains_result(condition["operand"], read)
                   for condition in path["conditions"]):
            continue
        for other in comparisons:
            route = other["sequence"]
            if (other["outcome"] == "RETURNED" and not other["open"] and len(route) > index
                    and route[:index] == path["sequence"][:index] and route[index]["step"] == event["step"]
                    and route[index]["outcome"] != event["outcome"]
                    and not any(item["step"] == effect for item in route)):
                return True
    return False


def durable_candidates(paths, steps, budget):
    """An interrupted path can leave the prior selection eligible, under stated premises."""
    found = {}
    for path in paths:
        if path["outcome"] == "OPEN" or path["open"]:
            continue
        calls = list(event_calls(path["sequence"], steps))
        for cut, event in enumerate(path["sequence"]):
            if event["outcome"] not in INTERRUPTED:
                continue
            for i, read, read_event, read_inv in calls:
                if i >= cut or read["role"] != "SQL_READ_CANDIDATE" or read_event["outcome"] not in COMPLETED:
                    continue
                for j, external, external_event, _ in calls:
                    if not i < j < cut or external["role"] not in EXTERNAL or external_event["outcome"] not in COMPLETED:
                        continue
                    if not read_gates_effect(path, paths, read["id"], external["id"], j):
                        continue
                    for normal in paths:
                        if normal["outcome"] not in {"NORMAL_EXIT", "RETURNED"} or normal["open"]:
                            continue
                        if path["sequence"][:j + 1] != normal["sequence"][:j + 1]:
                            continue
                        later = list(event_calls(normal["sequence"], steps))
                        for write_index, write, write_event, write_inv in later:
                            if write_index <= j or write["role"] != "SQL_WRITE_CANDIDATE" or write_event["outcome"] not in COMPLETED:
                                continue
                            commits = [(operation, binding, invoke) for k, operation, completed, invoke in later
                                       if k > write_index and completed["outcome"] in COMPLETED
                                       and (binding := commit_operation(operation, invoke))]
                            for commit, (transaction, authority, signature), invocation in commits:
                                matched = sql_pair(read, read_inv, write, write_inv, transaction, authority)
                                if not matched or any(
                                        completed["outcome"] in COMPLETED and (observed := commit_operation(action, invoked))
                                        and observed[0] == transaction for _, action, completed, invoked in calls):
                                    continue
                                receiver, key, key_mode = matched
                                fact = (read["id"], external["id"], event["step"], write["id"], commit["id"])
                                reentered = any(n > cut and action["id"] == read["id"] and
                                                invocation.get("receiver") == receiver and
                                                bound_argument(invocation, read["sql"]["parameter"]) == key
                                                for n, action, _, invocation in calls)
                                proof = commit_argument_evidence(invocation)
                                sources = {source for step in fact for source in steps[step]["sources"]}
                                if proof:
                                    sources.update(proof["sources"])
                                found.setdefault(fact, {
                                    "kind": "POSSIBLE_DURABLE_RESELECTION",
                                    "select": read["id"], "selectTable": read["sql"]["table"],
                                    "keyColumn": write["sql"]["column"], "key": key, "keyBinding": key_mode,
                                    "receiver": receiver, "transaction": transaction,
                                    "externalCall": external["id"], "effectAuthority": external["roleAuthority"],
                                    "completedAt": external_event["step"], "interruption": event["step"],
                                    "matchingWrite": write["id"], "matchingCommit": commit["id"], "commitAuthority": authority,
                                    "commitSignature": signature,
                                    **({"commitArgumentEvidence": proof} if proof else {}),
                                    "failurePath": path["id"], "comparisonPath": normal["id"],
                                    "reentry": "READ_REVISITED_ON_MODELED_PATH" if reentered else "IF_LATER_INVOCATION_REACHES_SELECT",
                                    "premises": ["The provider executes the retained SQL with the projected key and transaction arguments.",
                                                 "The effective completion API is a commit on that transaction.",
                                                 "The absent successful commit leaves a row with that key eligible when the select runs again.",
                                                 "Repeated source invocation leaves runtime delivery, idempotency and rollback open."],
                                    "sources": sorted(sources),
                                })
                                if len(found) >= budget:
                                    return list(found.values())
    return list(found.values())
