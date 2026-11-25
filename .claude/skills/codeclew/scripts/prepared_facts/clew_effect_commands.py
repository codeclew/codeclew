"""Project ADO-style relational command and row lineage along retained paths."""

from dataclasses import dataclass, field


LEXICAL_ROLES = {
    "CreateCommand": "ADO_CREATE_COMMAND",
    "CreateParameter": "ADO_CREATE_PARAMETER",
    "BeginTransaction": "TRANSACTION_BEGIN_REQUEST",
    "BeginTransactionAsync": "TRANSACTION_BEGIN_REQUEST",
    "ExecuteReader": "ADO_EXECUTE_READER",
    "ExecuteReaderAsync": "ADO_EXECUTE_READER",
    "ExecuteScalar": "ADO_EXECUTE_SCALAR",
    "ExecuteScalarAsync": "ADO_EXECUTE_SCALAR",
    "ExecuteNonQuery": "ADO_EXECUTE_WRITE",
    "ExecuteNonQueryAsync": "ADO_EXECUTE_WRITE",
    "Add": "ADO_ADD_PARAMETER",
    "AddWithValue": "ADO_ADD_WITH_VALUE",
    "Read": "ADO_READER_READ",
    "ReadAsync": "ADO_READER_READ",
    "NextResult": "ADO_READER_NEXT_RESULT",
    "NextResultAsync": "ADO_READER_NEXT_RESULT",
    "GetOrdinal": "ADO_READER_ORDINAL",
    **{name: "ADO_READER_VALUE" for name in (
        "GetValue", "GetFieldValue", "GetFieldValueAsync", "GetString", "GetGuid", "GetBoolean",
        "GetInt16", "GetInt32", "GetInt64", "GetDecimal", "GetDouble", "GetFloat",
    )},
}
TYPED = "ROSLYN_ADO_DECLARATION"
CANDIDATE = "CONDITIONAL_COMMAND_API"
MAX_COMMAND_PATHS = 18
MAX_COMMAND_FACTS = 12


def completion_boundary(role, spelling, dispatch):
    return role in {"ADO_EXECUTE_WRITE", "TRANSACTION_COMMIT_REQUEST"} or (
        dispatch == "OPEN" and spelling in {"ExecuteNonQuery", "ExecuteNonQueryAsync", "Commit", "CommitAsync"})


def literal(value):
    return value[1] if value and value[0] == "literal" else None


def tracked(value):
    return value and value[0] not in {"unbound-name", "unbound-member", "open-member", "literal"}


def argument(event, position):
    return next((arg["value"] for arg in event.get("arguments", ()) if arg["position"] == position), None)


def constructor_argument(event, position, names):
    args = event.get("arguments", ())
    named = [arg["value"] for arg in args if arg.get("name", "").lower() in names]
    if len(named) == 1:
        return named[0]
    return next((arg["value"] for arg in args if arg["position"] == position and not arg.get("name")), None)


def property_receiver(value, name):
    return value[2] if value and value[0] == "ado-member" and value[1] == name else None


def source_list(steps, step_ids):
    return sorted({source for step in step_ids if step in steps for source in steps[step]["sources"]})


def display_column(name):
    return name.strip('"[]`')


@dataclass
class Parameter:
    name: str | None = None
    value: tuple | None = None
    sites: list = field(default_factory=list)


@dataclass
class Command:
    connection: tuple | None
    typed: bool
    sites: list
    transaction: tuple | None = None
    sql: str | None = None
    valid: bool = True
    text_mode: bool = True
    parameters: list = field(default_factory=list)

    def parameter(self, selector):
        if type(selector) is int and 0 <= selector < len(self.parameters):
            return self.parameters[selector]
        if isinstance(selector, str):
            matches = [p for p in self.parameters if p.name and p.name.lstrip("@:") == selector.lstrip("@:")]
            return matches[0] if len(matches) == 1 else None
        return None


@dataclass
class Reader:
    query: dict
    current: dict | None = None
    latest: tuple | None = None


def row_test(value, truth):
    if value and value[0] == "not":
        return row_test(value[1], not truth)
    if value and value[0] in {"equal", "different"}:
        left, right = value[1:]
        if left[0] == "literal":
            left, right = right, left
        if right[0] == "literal" and type(right[1]) is bool:
            return row_test(left, right[1] == (truth == (value[0] == "equal")))
    return value, truth


class CommandPath:
    def __init__(self, steps, storage, sql_tables):
        self.steps = steps
        self.storage = storage
        self.sql_tables = sql_tables
        self.commands = {}
        self.parameters = {}
        self.transactions = {}
        self.readers = {}
        self.read_tests = {}
        self.column_ordinals = {}
        self.values = {}
        self.indexes = {}
        self.calls = {}
        self.executions = []
        self.resource_families = {}

    def project(self, path):
        conditions = iter(path["conditions"])
        for index, event in enumerate(path["sequence"]):
            step = self.steps[event["step"]]
            outcome = event["outcome"]
            if outcome == "INVOKED":
                self.calls[step["id"]] = event
                if step.get("receiverFamily") and tracked(event.get("receiver")):
                    self.resource_families[event["receiver"]] = step["receiverFamily"], step["id"]
            elif outcome == "CONSTRUCTED":
                self.construct(event, step)
                if step.get("resourceFamily"):
                    self.resource_families[event["result"]] = step["resourceFamily"], step["id"]
            elif outcome == "RESOURCE_CONFIGURATION_REACHED":
                self.assign(event["resource"], event["value"], event["step"])
            elif outcome in {"TRUE", "FALSE"}:
                condition = next(conditions)
                operand, has_row = row_test(condition["operand"], condition["value"])
                if (prior := self.read_tests.get(operand)) and prior[0].latest == operand:
                    reader, completion = prior
                    reader.current = {"index": index, "source": event["step"], "advancedAt": completion} if has_row else None
                    reader.query.setdefault("rowChecks", []).append({"test": event["step"], "readerCall": operand[1],
                                                                      "completion": completion, "hasRow": has_row})
            elif outcome == "INDEX_VALUE_REACHED" and event.get("value"):
                self.indexes[event["result"]] = event["value"]
                if value := self.read_index(event["value"], event["step"]):
                    self.values[event["result"]] = value
            elif (call := self.calls.get(event.get("call"))) and outcome in {
                    "RESUMED", "RETURNED", "CALL_EXCEPTION", "FAULT_OR_CANCELLATION", "AWAITABLE_RETURNED"}:
                self.complete(self.steps[call["step"]], call, event, index)
        return self.executions

    def construct(self, event, step):
        args = event.get("arguments", ())
        if not (step.get("command") or step.get("unresolved") or step.get("parameter")) or len(args) > 3:
            return
        if not args:
            self.commands[event["result"]] = Command(None, bool(step.get("command")), [step["id"]])
            self.parameters[event["result"]] = Parameter(sites=[step["id"]])
            return
        sql = literal(constructor_argument(event, 0, {"cmdtext", "commandtext", "sql", "parametername", "name"}))
        if not isinstance(sql, str):
            return
        if not self.storage["author_sql_contract"](sql):
            if len(args) <= 2 and (step.get("parameter") or step.get("unresolved")):
                scalar = argument(event, 1)
                second = args[1] if len(args) == 2 else {}
                if second.get("name") in {None, "value"}:
                    self.parameters[event["result"]] = Parameter(sql, scalar, [step["id"]])
            return
        connection = constructor_argument(event, 1, {"connection"})
        transaction = constructor_argument(event, 2, {"transaction"})
        if connection is not None and not tracked(connection) or transaction is not None and not tracked(transaction):
            return
        self.commands[event["result"]] = Command(connection, bool(step.get("command")), [step["id"]], transaction, sql)

    def set_parameter(self, parameter, member, value, site):
        parameter.sites.append(site)
        if member == "ParameterName":
            name = literal(value)
            parameter.name = name if isinstance(name, str) else None
        else:
            parameter.value = value

    def assign(self, target, value, site):
        if target[0] != "ado-member":
            return
        _, name, owner, typed = target
        family = self.resource_families.get(owner)
        if owner not in self.commands and family and family[0] == "command" and name in {
                "CommandText", "Connection", "Transaction", "CommandType"}:
            self.commands[owner] = Command(None, False, [family[1]])
        if (command := self.commands.get(owner)) and name in {"CommandText", "Connection", "Transaction", "CommandType"}:
            command.sites.append(site)
            command.typed = command.typed and typed
            if name == "CommandText":
                command.sql = literal(value)
            elif name == "Connection":
                command.connection = value
            elif name == "Transaction":
                command.transaction = value
            else:
                command.text_mode = literal(value) == 1
            return
        parameter = self.parameters.get(owner)
        owner = self.indexes.get(owner, owner)
        if owner[0] == "ado-index" and (command := self.commands.get(property_receiver(owner[1], "Parameters"))):
            parameter = command.parameter(literal(owner[2]))
        if parameter is not None and name in {"Value", "ParameterName"}:
            self.set_parameter(parameter, name, value, site)

    def column(self, reader_value, selector):
        known = self.column_ordinals.get(selector)
        if known and known[0] == reader_value:
            selector = known[1]
        key = literal(selector)
        reader = self.readers.get(reader_value)
        if reader is None or reader.current is None:
            return None
        fields = reader.query["sourceSql"]["projections"]
        if type(key) is int and 0 <= key < len(fields) and fields[key]["column"] != "*":
            return fields[key]
        if isinstance(key, str):
            if any(field["name"] is None for field in fields):
                return None
            names = [field for field in fields if display_column(field["name"]) == key]
            if len(names) == 1:
                return names[0]
        return None

    def row_value(self, reader_value, selector, site):
        if (selected := self.column(reader_value, selector)) and (column := selected["column"] or selected.get("sourceColumn")):
            reader = self.readers[reader_value]
            return {"reader": reader_value, "column": column, "row": reader.current, "source": site,
                    **({"cast": selected["cast"]} if selected.get("cast") else {})}
        return None

    def read_index(self, value, site):
        return self.row_value(value[1], value[2], site) if value and value[0] == "ado-index" else None

    def complete(self, step, call, event, index):
        role = step.get("role")
        typed = role is not None
        if not typed and step.get("dispatch") == "OPEN" and not step.get("symbol"):
            role = LEXICAL_ROLES.get(step.get("spelling"))
        receiver, result = call["receiver"], call["result"]
        finished = event["outcome"] in {"RESUMED", "RETURNED"}
        if finished and event.get("composed"):
            if event.get("returnedFamily") and tracked(event.get("returnedValue")):
                self.resource_families[event["returnedValue"]] = event["returnedFamily"], step["id"]
            return
        if finished and step.get("resultFamily"):
            self.resource_families[result] = step["resultFamily"], step["id"]
        if role in {"ADO_CREATE_COMMAND", "ADO_CREATE_DATA_SOURCE_COMMAND"} and finished and tracked(receiver):
            sql = literal(constructor_argument(call, 0, {"cmdtext", "commandtext", "sql"}))
            connection = receiver if role == "ADO_CREATE_COMMAND" else None
            self.commands[result] = Command(connection, typed, [step["id"]], sql=sql)
        elif role == "TRANSACTION_BEGIN_REQUEST" and finished and tracked(receiver):
            self.transactions[result] = (receiver, typed, step["id"])
        elif role == "ADO_CREATE_PARAMETER" and finished and receiver in self.commands:
            self.parameters[result] = Parameter(sites=[step["id"]])
        elif role in {"ADO_ADD_PARAMETER", "ADO_ADD_WITH_VALUE"} and finished:
            self.add_parameter(role, call, step["id"], typed)
        elif role in {"ADO_EXECUTE_READER", "ADO_EXECUTE_SCALAR", "ADO_EXECUTE_WRITE"} and receiver in self.commands:
            self.execute(role, receiver, result, event, index, typed)
        elif receiver in self.readers and role == "ADO_READER_READ":
            reader = self.readers[receiver]
            reader.current = None
            reader.latest = result if finished else None
            if finished:
                self.read_tests[result] = (reader, event["step"])
        elif receiver in self.readers and role == "ADO_READER_NEXT_RESULT":
            self.readers.pop(receiver, None)
        elif receiver in self.readers and role == "ADO_READER_ORDINAL" and finished:
            self.column_ordinals[result] = (receiver, argument(call, 0))
        elif receiver in self.readers and role == "ADO_READER_VALUE" and finished:
            if value := self.row_value(receiver, argument(call, 0), event["step"]):
                self.values[result] = value
                self.readers[receiver].query.setdefault("columnReads", []).append({
                    "value": result, "column": value["column"], "call": step["id"], "completion": event["step"],
                    "rowCheck": value["row"]["source"], "rowIndex": value["row"]["index"],
                    **({"cast": value["cast"]} if value.get("cast") else {}),
                })
        elif not role or role in {"LIST_ADD"}:
            for value in [receiver, *(arg["value"] for arg in call.get("arguments", ()))]:
                if value in self.commands:
                    self.commands[value].valid = False
                if value in self.readers:
                    reader = self.readers[value]
                    reader.current = reader.latest = None

    def add_parameter(self, role, call, site, typed):
        owner = property_receiver(call["receiver"], "Parameters")
        if (command := self.commands.get(owner)) is None:
            return
        command.typed = command.typed and typed
        value = argument(call, 0)
        if parameter := self.parameters.get(value):
            parameter.sites.append(site)
            command.parameters.append(parameter)
            return
        arguments = call.get("arguments", ())
        if role == "ADO_ADD_WITH_VALUE" and len(arguments) == 1:
            command.parameters.append(Parameter(value=value, sites=[site]))
            return
        if isinstance(name := literal(value), str):
            assigned = argument(call, 1) if role == "ADO_ADD_WITH_VALUE" else None
            added = Parameter(name, assigned, [site])
            self.parameters[call["result"]] = added
            command.parameters.append(added)
        else:
            command.valid = False

    def execute(self, role, receiver, result, event, index, typed):
        command = self.commands[receiver]
        if not command.valid or not command.text_mode or not isinstance(command.sql, str):
            return
        sql = self.storage["sql_contract"](command.sql, self.sql_tables) if tracked(command.connection) else None
        source_sql = self.storage["author_sql_contract"](command.sql)
        if not source_sql:
            return
        read = source_sql["verb"] == "SELECT"
        if (read and role == "ADO_EXECUTE_WRITE" or not read and role != "ADO_EXECUTE_WRITE" and
                not source_sql.get("returning")):
            return
        sites = [*command.sites, event["call"], event["step"]]
        begin = self.transactions.get(command.transaction)
        if begin:
            if tracked(command.connection) and begin[0] != command.connection:
                return
            sites.append(begin[2])
        for owner in (command.connection, command.transaction):
            if family := self.resource_families.get(owner):
                sites.append(family[1])
        selector = None if sql is None else sql["parameter"] if sql["parameter"] is not None else sql["parameterPosition"]
        parameter = command.parameter(selector) if selector is not None else None
        if parameter:
            sites.extend(parameter.sites)
        key = parameter.value if parameter else None
        row = self.values.get(key) or self.read_index(key, sites[-1])
        if row:
            sites.extend((row["source"], row["row"]["source"], row["row"]["advancedAt"]))
        bindings = []
        for name, position in self.storage["command_parameters"](source_sql):
            if bound := command.parameter(position):
                bindings.append({"name": name, "value": bound.value})
                sites.extend(bound.sites)
        row_record = {
            "kind": "SELECT" if read else "WRITE", "call": event["call"], "completion": event["step"],
            "outcome": event["outcome"], "index": index, "command": receiver, "connection": command.connection,
            "transaction": command.transaction, "sql": sql, "parameter": selector, "key": key,
            "rowKey": row, "authority": TYPED if typed and command.typed else CANDIDATE,
            "sourceSql": source_sql, "bindings": bindings, "executionRole": role,
            **({"returnValue": result} if event["outcome"] in {"RESUMED", "RETURNED"} else {}),
            **({"transactionStart": begin[2]} if begin else {}),
            **({"connectionFamily": self.resource_families[command.connection][0]} if command.connection in self.resource_families else {}),
            "evidenceSteps": list(dict.fromkeys(sites)),
        }
        self.executions.append(row_record)
        if role == "ADO_EXECUTE_READER" and event["outcome"] in {"RESUMED", "RETURNED"}:
            self.readers[result] = Reader(row_record)
            row_record["reader"] = result


def transitioned(select, write):
    sql, mutation = select["sql"], write["sql"]
    if (select["connection"], select["transaction"], sql["table"]) != (
            write["connection"], write["transaction"], mutation["table"]):
        return False
    if not tracked(select["transaction"]) or sql["column"] not in {None, mutation["column"]}:
        return False
    if any(sql["predicate"].get(name) != value for name, value in mutation["predicate"].items()):
        return False
    return mutation["verb"] == "DELETE" or any(
        name in mutation["assignment"] and value != mutation["assignment"][name]
        and (not value.startswith("not-") or value[4:] == mutation["assignment"][name])
        for name, value in sql["predicate"].items())


def prefix_gaps(path, index):
    gaps = []
    if any(event["outcome"] in {"COMPLETION_OPEN", "WRITE_TARGET_OPEN"} for event in path["sequence"][:index + 1]):
        gaps.append("UNRESOLVED_OPERATION_BEFORE_WITNESS")
    if "CONDITIONAL_SOURCE_CANDIDATE" in path["open"]:
        gaps.append("CONDITIONAL_SOURCE_CANDIDATE")
    return gaps


def selected_row_argument(value, origins, completed, steps, depth=0):
    """Link a reader column, or an immutable JSON projection of it, to the reached send."""
    if not value or depth > 8:
        return None
    if value in origins:
        source = origins[value]
        return [source["call"], source["completion"]]
    if value[0] == "json-root":
        return selected_row_argument(value[1], origins, completed, steps, depth + 1)
    if value not in completed:
        return None
    call, done = completed[value]
    if value[0] == "access-result" and (parent := call.get("value")) and parent[0] == "json-root":
        proof = selected_row_argument(parent, origins, completed, steps, depth + 1)
        return [*proof, call["step"]] if proof else None
    if value[0] != "call-result":
        return None
    factory = steps[call["step"]].get("valueFactory")
    if factory == "JSON_PARSE" and (steps[call["step"]].get("symbol") or "").startswith("M:System.Text.Json.JsonDocument.Parse("):
        parent = argument(call, 0)
    elif factory == "JSON_VALUE" or factory == "JSON_PROPERTY" and isinstance(literal(argument(call, 0)), str):
        parent = call.get("receiver")
    else:
        return None
    proof = selected_row_argument(parent, origins, completed, steps, depth + 1)
    return [*proof, call["step"], done["step"]] if proof else None


def reader_publication(read, row, invocation, sequence, calls, steps, storage, before):
    origins = {column["value"]: column for column in read.get("columnReads", ())
               if column["rowIndex"] == row["row"]["index"]}
    completed = {started["result"]: (started, done) for i, _, done, started in calls
                 if i < before and done["outcome"] in storage["COMPLETED"]}
    completed.update({event["result"]: (event, event) for event in sequence if event["outcome"] == "INDEX_VALUE_REACHED"})
    arguments, fields = storage["publication_values"](invocation, sequence)
    evidence, data = [], []
    for label, value, site in (*arguments, *fields):
        if proof := selected_row_argument(value, origins, completed, steps):
            evidence.extend(proof)
            if site:
                evidence.append(site)
            if label:
                data.append({"field": label, "value": value})
    return evidence, data


def command_candidates(paths, records, steps, storage):
    result = {}
    path_calls = {path["id"]: [(index, call, event, invocation)
                              for index, call, event, invocation in storage["event_calls"](path["sequence"], steps)
                              if index >= path.get("scope", {}).get("prefixEvents", 0)] for path in paths}
    for path in paths:
        calls = path_calls[path["id"]]
        eligible = [cmd for cmd in records[path["id"]] if cmd["kind"] == "SELECT" and cmd["sql"] and cmd.get("reader")]
        if not eligible:
            continue
        effects = [(index, call, event, invocation) for index, call, event, invocation in calls if
                   call["role"] in storage["EXTERNAL"] and (event["outcome"] in storage["COMPLETED"] or
                   call["role"] == "PUBLICATION_CANDIDATE" and event["outcome"] == "FAULT_OR_CANCELLATION")]
        for index, effect, observed, sent in effects:
            seq = path["sequence"]
            publication = effect["role"] == "PUBLICATION_CANDIDATE"
            ack_failed = observed["outcome"] == "FAULT_OR_CANCELLATION"
            interruptions = [(index, observed)] if ack_failed else enumerate(seq[index + 1:], index + 1)
            prefix_end = index if ack_failed else index + 1
            for cut, interruption in interruptions:
                if interruption["outcome"] not in storage["INTERRUPTED"]:
                    continue
                for normal in paths:
                    route = normal["sequence"]
                    if seq[:prefix_end] != route[:prefix_end]:
                        continue
                    if ack_failed and not (len(route) > index and route[index]["step"] == observed["step"] and
                                           route[index]["outcome"] == "RESUMED"):
                        continue
                    completed = route[index]
                    writes = [cmd for cmd in records[normal["id"]] if cmd["kind"] == "WRITE" and cmd["sql"]
                              and cmd["index"] > index and cmd["outcome"] in storage["COMPLETED"] and cmd["rowKey"]]
                    for write in writes:
                        row = write["rowKey"]
                        if row.get("cast") or row["row"]["index"] >= index or row["column"] != write["sql"]["column"]:
                            continue
                        read = next((cmd for cmd in eligible if cmd["reader"] == row["reader"] and cmd["index"] < index), None)
                        if read is None or not transitioned(read, write):
                            continue
                        event_index = row["row"]["index"]
                        payload, data = reader_publication(read, row, sent, seq[event_index:index], calls, steps, storage, index)
                        if publication and not payload:
                            continue
                        tx = write["transaction"]
                        if any(commit_matches(call, invocation, tx, storage) and event["outcome"] in storage["COMPLETED"]
                               for _, call, event, invocation in calls):
                            continue
                        for at, commit, outcome, invocation in path_calls[normal["id"]]:
                            if at <= write["index"] or outcome["outcome"] not in storage["COMPLETED"]:
                                continue
                            binding = commit_matches(commit, invocation, tx, storage)
                            if not binding or any(null_dispatch_required(candidate, read, write) for candidate in (path, normal)):
                                continue
                            authority, signature = binding
                            commit_failed = (cut == at and interruption.get("call") == commit["id"] and
                                             interruption["step"] == outcome["step"] and seq[:cut] == route[:cut])
                            connection = storage["returned_connection"](steps, route[:read["index"]], read["connection"])
                            begin = storage["transaction_begin"](path_calls[normal["id"]], read["connection"], tx,
                                                                  read["index"], connection)
                            if publication and (ack_failed or commit_failed) and not begin:
                                continue
                            key = (read["call"], effect["id"], interruption["step"], write["call"], commit["id"])
                            proof = [*connection, *begin, *read["evidenceSteps"], *write["evidenceSteps"], *payload,
                                     effect["id"], completed["step"], interruption["step"], commit["id"], outcome["step"]]
                            argument = storage["commit_argument_evidence"](invocation)
                            sources = set(source_list(steps, proof))
                            if argument:
                                sources.update(argument["sources"])
                            open_prefix = sorted(set(prefix_gaps(path, cut) + prefix_gaps(normal, at)))
                            if path["outcome"] == "OPEN":
                                open_prefix.append("LATER_INTERRUPTED_PATH_OPEN")
                            result.setdefault(key, {
                                "kind": "POSSIBLE_DURABLE_RESELECTION", "select": read["call"],
                                "selectTable": read["sql"]["table"], "keyColumn": row["column"],
                                "keyBinding": "READER_ROW_COLUMN_TO_COMMAND_PARAMETER",
                                "keyRead": row["source"], "rowAvailableAt": row["row"]["source"],
                                "rowGuard": {"step": row["row"]["source"], "when": seq[event_index]["outcome"] == "TRUE"},
                                "parameter": write["parameter"], "receiver": write["connection"], "transaction": tx,
                                "externalCall": effect["id"], "effectAuthority": effect["roleAuthority"],
                                "completedAt": completed["step"], "interruption": interruption["step"],
                                "matchingWrite": write["call"], "matchingWriteSql": write["sourceSql"],
                                "matchingCommit": commit["id"], "matchingCommitFailed": publication and commit_failed,
                                "publicationAwaitFailed": ack_failed,
                                "commitAuthority": authority, "commitSignature": signature,
                                "selectedRowSuppliesArguments": bool(payload), "publicationFields": data,
                                "publicationOccurrence": sent["result"],
                                **({"connectionEvidence": connection} if connection else {}),
                                **({"commitArgumentEvidence": argument} if argument else {}),
                                "commandAuthority": TYPED if read["authority"] == write["authority"] == TYPED else CANDIDATE,
                                "failurePath": path["id"], "comparisonPath": normal["id"],
                                "comparisonExtent": "THROUGH_MATCHING_COMMIT", "comparisonEnd": outcome["step"],
                                "openPremises": open_prefix,
                                "reentry": "IF_LATER_SELECT_ENCOUNTERS_THE_SAME_ELIGIBLE_KEY",
                                "evidenceSteps": list(dict.fromkeys(proof)), "sources": sorted(sources),
                                "premises": [
                                    "The selected command and provider honor the projected SQL and parameter binding.",
                                    "The reader column and write predicate address the same eligible record.",
                                    "The interrupted path leaves the record eligible when the select runs again.",
                                    "Runtime delivery, commit, rollback and later activation remain open.",
                                ],
                            })
                            if len(result) >= MAX_COMMAND_FACTS:
                                return list(result.values())
    return list(result.values())


def commit_matches(call, invocation, transaction, storage):
    binding = storage["commit_operation"](call, invocation)
    return binding[1:] if binding and binding[0] == transaction else None


def null_dispatch_required(path, read, write):
    handles = {read["reader"], read["command"], write["command"], write["connection"], write["transaction"]}
    return any(event.get("dispatchPremise") == "NULL_ACCEPTING_NON_INSTANCE_DISPATCH_REQUIRED"
               and event.get("receiver") in handles for event in path["sequence"])


def public_commands(records, steps):
    return [{**{key: value for key, value in command.items() if key != "index"}, "eventIndex": command["index"],
             "sources": source_list(steps, command["evidenceSteps"])} for command in records]


def project_callable(path, steps, sql_tables, storage):
    records = CommandPath(steps, storage, sql_tables).project(path)
    return public_commands((command for command in records if command["index"] >= path["scope"]["prefixEvents"]), steps)


def completed_call(call, event):
    expected = "RESUMED" if call.get("taskLike") or (call.get("spelling") or "").endswith("Async") else "RETURNED"
    return event["outcome"] == expected


def transaction_outcomes(path, records, steps, storage, http_root):
    """Join a completed write to its own transaction on a normal source path."""
    if path["outcome"] not in {"RETURNED", "NORMAL_EXIT"} or set(path["open"]) - {
            "CONDITIONAL_SOURCE_CANDIDATE", "ASSUMED_SDK_GLOBAL_IMPORTS", "CONDITIONAL_NOMINAL_FRAMEWORK_MEMBER"}:
        return []
    sequence = path["sequence"]
    if any(event["outcome"] in {"THROWN", "CALL_EXCEPTION", "FAULT_OR_CANCELLATION", "CONSTRUCTOR_EXCEPTION"} or
           steps[event["step"]]["kind"] == "CATCH" or event.get("unexpandedSource") for event in sequence):
        return []
    result, returned = None, None
    return_index = len(sequence)
    status = None
    if http_root:
        value = path.get("returnValue")
        if not value:
            return []
        result = value[1] if value[0] == "call-result" else f"{value[1]}/op{value[2]}" if value[0] == "allocation" else None
        if not (status := steps.get(result, {}).get("httpResult")):
            return []
        exit_site = next(((i, event["step"]) for i, event in reversed(list(enumerate(sequence)))
                          if steps[event["step"]]["kind"] == "RETURN" and event["step"].startswith(http_root + "/")), None)
        if exit_site is None:
            return []
        return_index, returned = exit_site
    calls = list(storage["event_calls"](sequence, steps))
    writes = [record for record in records if record["kind"] == "WRITE"]
    effect_calls = [event for event in sequence if event["outcome"] == "INVOKED" and
                    ((call := steps[event["step"]]).get("role") in {"ADO_EXECUTE_WRITE", "SQL_WRITE_CANDIDATE"} or
                     call.get("sourceSql", {}).get("verb") not in {None, "SELECT"} or
                     call.get("spelling") in {"ExecuteNonQuery", "ExecuteNonQueryAsync"})]
    matches = []
    for index, commit, event, invocation in calls:
        if index >= return_index or not completed_call(commit, event):
            continue
        operation = storage["commit_operation"](commit, invocation)
        if not operation:
            continue
        tx, authority, signature = operation
        paired = [row for row in writes if row["transaction"] == tx and row.get("transactionStart")]
        if (writes or effect_calls or not http_root) and (not paired or any(row["index"] >= index or
                row["outcome"] not in {"RETURNED", "RESUMED"} for row in paired)):
            continue
        begin = next(((at, call, done, invoked) for at, call, done, invoked in calls
                      if invoked.get("result") == tx and completed_call(call, done) and
                      (call.get("role") == "TRANSACTION_BEGIN_REQUEST" or call.get("spelling") in
                       {"BeginTransaction", "BeginTransactionAsync"} and not call.get("symbol"))), None)
        if begin:
            began, begin_call, started, receiver = begin
            connection = receiver.get("receiver")
            if not tracked(connection) or began >= index or any(row.get("connection") != connection or
                    row["index"] <= began for row in paired):
                continue
        elif paired or authority != "ROSLYN_FRAMEWORK_DECLARATION":
            continue

        if any(at < return_index and invoked.get("receiver") == tx and
               (call.get("role") == "TRANSACTION_ROLLBACK_REQUEST" or
                call.get("spelling") in {"Rollback", "RollbackAsync"} and not call.get("symbol"))
               for at, call, _, invoked in calls):
            continue
        # An unexpanded method may change the transaction or connection when
        # passed either handle. Identified ADO configuration and completed
        # source summaries retain their effects in this same sequence.
        if begin and any(began < at < index and not done.get("composed") and
                         not call.get("role") and not call.get("implicit") and
                         call.get("spelling") not in LEXICAL_ROLES and
                         {tx, connection}.intersection([invoked.get("receiver"),
                                                        *(arg["value"] for arg in invoked.get("arguments", ()))])
                         for at, call, done, invoked in calls):
            continue
        proof = [*(site for row in paired for site in row["evidenceSteps"]), commit["id"], event["step"]]
        if result and returned:
            proof.extend((result, returned))
        if begin:
            proof.extend((begin_call["id"], started["step"]))
        if authority != "ROSLYN_FRAMEWORK_DECLARATION" or any(row["authority"] == CANDIDATE for row in paired):
            premise = "PROVIDER_COMMAND_AND_TRANSACTION_BINDING"
        else:
            premise = "SOURCE_COMPLETION"
        matches.append({"path": path["id"], "commit": commit["id"], "completion": event["step"],
                        "commitIndex": index, "transaction": tx, "commitAuthority": authority,
                        "commitSignature": signature, "authority": premise,
                        "writes": public_commands(paired, steps), "evidenceSteps": list(dict.fromkeys(proof)),
                        "sources": source_list(steps, proof),
                        **({"begin": begin_call["id"], "connection": connection} if begin else {}),
                        **({"status": status["status"], "return": returned, "response": result} if status else {})})
    return matches


def analyze(paths, steps, sql_tables, storage, http_root=None):
    records = {path["id"]: CommandPath(steps, storage, sql_tables).project(path)
               for path in paths}
    candidates = command_candidates(paths, records, steps, storage)
    transactions = [match for path in paths for match in transaction_outcomes(path, records[path["id"]], steps, storage, http_root)]
    witnessed = {fact[key] for fact in candidates for key in ("failurePath", "comparisonPath")}
    witnessed.update(match["path"] for match in transactions)
    ordered = sorted(paths, key=lambda path: path["id"] not in witnessed)
    groups = [{"path": path["id"], "commands": public_commands(records[path["id"]], steps)}
        for path in ordered if records[path["id"]]]
    return groups[:MAX_COMMAND_PATHS], candidates, transactions[:MAX_COMMAND_PATHS]
