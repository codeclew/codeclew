"""Prove closure of a normal HTTP return on one retained Roslyn path."""


BINDING_ASSUMPTIONS = {"ASSUMED_SDK_GLOBAL_IMPORTS", "CONDITIONAL_SOURCE_CANDIDATE",
                       "CONDITIONAL_NOMINAL_FRAMEWORK_MEMBER"}
COMPOSED = {"ROSLYN_STATIC_BINDING", "CONDITIONAL_SOURCE_CANDIDATE"}
OPEN_EVENTS = {"THROWN", "CALL_EXCEPTION", "FAULT_OR_CANCELLATION", "CONSTRUCTOR_EXCEPTION", "COMPLETION_OPEN",
               "WRITE_TARGET_OPEN", "CANDIDATE_INITIALIZER_REACHED", "AWAITABLE_RETURNED"}
COMPLETIONS = {"RETURNED", "RESUMED"}
PROVIDER_ROLES = {"ADO_CREATE_COMMAND", "ADO_CREATE_DATA_SOURCE_COMMAND", "ADO_CREATE_PARAMETER", "ADO_ADD_PARAMETER",
                  "ADO_READER_READ", "ADO_READER_NEXT_RESULT", "ADO_READER_ORDINAL", "ADO_READER_VALUE",
                  "TRANSACTION_BEGIN_REQUEST"}
EXERCISED_COMMANDS = {"ADO_EXECUTE_WRITE", "ADO_EXECUTE_SCALAR", "ADO_EXECUTE_READER"}
PROVIDER_FACTORY = {"OpenConnection": "connection", "OpenConnectionAsync": "connection", "CreateCommand": "command",
                    "BeginTransaction": "transaction", "BeginTransactionAsync": "transaction", "CreateParameter": "parameter"}
RESOURCE_CALLS = {"connection": {"Open", "OpenAsync", "Close", "CloseAsync", "CreateCommand", "BeginTransaction", "BeginTransactionAsync"},
                  "data source": {"OpenConnection", "OpenConnectionAsync", "CreateCommand"},
                  "command": {"CreateParameter"},
                  "reader": {"Read", "ReadAsync", "NextResult", "NextResultAsync", "GetOrdinal", "GetValue", "GetFieldValue",
                             "GetFieldValueAsync", "GetString", "GetGuid", "GetBoolean", "GetByte", "GetInt16", "GetInt32",
                             "GetInt64", "GetDecimal", "GetDouble", "GetFloat"}}
DISPOSAL = {"Dispose", "DisposeAsync"}
MAX_WRITES = 4


class ClosedReturn:
    def __init__(self, facts, path):
        self.facts, self.steps = facts, facts.steps
        commands = facts.commands_for(path)
        self.commands = {row["eventIndex"]: row for row in commands}
        self.read_calls = {check["test"] for row in commands for check in row.get("rowChecks", ())}
        self.commits = {row["commitIndex"]: row for row in facts.transactions[path["id"]] if "status" in row}
        self.pending, self.resources = {}, {}
        self.proof, self.writes, self.reader_loops = [], [], set()
        self.awaited = False

    def construction(self, step, event):
        symbol = step.get("symbol") or ""
        if (step.get("httpResult") or step.get("transparentRecord")
                or step.get("emptyList") and symbol.startswith("M:System.Collections.Generic.List")):
            return True
        if family := step.get("resourceFamily"):
            if step.get("candidateEffectFamily") == "database" or symbol.startswith("M:System.Data.Common.Db"):
                self.resources[event["result"]] = family
                return True
        return False

    def provider(self, step, invoked, index):
        name, role = step.get("spelling"), step.get("role")
        symbol = step.get("symbol") or ""
        receiver = invoked.get("receiver")
        tracked = self.resources.get(receiver)
        typed = step.get("directEffectFamily") == "database"
        nominal = step.get("candidateEffectFamily") == "database"
        if (commit := self.commits.get(index)) and commit["commit"] == step["id"]:
            return True
        if command := self.commands.get(index):
            if not (role in EXERCISED_COMMANDS and typed or nominal or tracked == "command"):
                return False
            if command["kind"] == "SELECT" and (self.reader_loops or self.writes):
                # A fresh query during iteration or after a write can be an observation dependency.
                return False
            family = "reader" if command["executionRole"] == "ADO_EXECUTE_READER" else None
            if family:
                self.resources[invoked["result"]] = family
            if command["kind"] == "WRITE":
                self.writes.append(command)
            return True
        if (typed and (role in PROVIDER_ROLES or name in {"Open", "OpenAsync", "Close", "CloseAsync",
                                                          "OpenConnection", "OpenConnectionAsync"})
                or nominal and name in RESOURCE_CALLS.get(step.get("receiverFamily"), ())
                or tracked and name in RESOURCE_CALLS.get(tracked, ())):
            if family := PROVIDER_FACTORY.get(name) or step.get("resultFamily"):
                self.resources[invoked["result"]] = family
            return True
        if (name in {"Add", "AddWithValue"} and receiver and receiver[:2] == ("ado-member", "Parameters") and
                self.resources.get(receiver[2]) == "command" and (not symbol or typed or nominal)):
            return True
        return name in DISPOSAL and (
            tracked or step.get("receiverFamily") and symbol.startswith(("M:System.IDisposable.",
                "M:System.IAsyncDisposable.", "M:System.Data.Common.Db")))

    def call(self, step, invocation, done, index):
        if done.get("composed") in COMPOSED:
            # The helper's effects have gone through the same closure check before this resume.
            return True
        if step.get("httpResult") or step.get("valueFactory") == "GUID_NEW" or step.get("localJsonSerialization"):
            return True
        role, factory = step.get("role"), step.get("valueFactory")
        if role in {"LIST_ADD", "LIST_CLEAR"} and step.get("directEffectFamily") == "local":
            return True
        if factory == "LINQ_FIRST_OR_DEFAULT" and ("selectedValue" in done or done.get("emptyListDefault")):
            return True
        if factory == "LINQ_SELECT" and done.get("localProjection"):
            return True
        if (step.get("symbol") or "").startswith("M:System.Threading.Tasks.Task.FromResult``1("):
            return True
        return self.provider(step, invocation, index)

    def event(self, index, event):
        key, outcome = event["step"], event["outcome"]
        step = self.steps[key]
        if outcome in OPEN_EVENTS or event.get("unexpandedSource") or step["kind"] == "CATCH":
            return False
        if step.get("loopTest") and outcome in {"TRUE", "FALSE"}:
            if key not in self.read_calls:
                return False
            if outcome == "TRUE":
                self.reader_loops.add(key)
            else:
                self.reader_loops.discard(key)
        if outcome == "INVOKED":
            if key in self.pending or any(arg.get("byRef") for arg in event.get("arguments", ())):
                return False
            self.pending[key] = event
        elif outcome == "CONSTRUCTED":
            if not self.construction(step, event):
                return False
            self.proof.append(key)
        elif outcome in COMPLETIONS and (step["kind"] == "AWAIT" or event.get("call")):
            call = event.get("call")
            invocation = self.pending.pop(call, None)
            if invocation is None or not self.call(self.steps[call], invocation, event, index):
                return False
            self.proof.extend((call, key))
            self.awaited |= outcome == "RESUMED"
        return True

    def completed_writes(self):
        committed = {(row["call"], row["sourceSql"]["table"], row["sourceSql"]["verb"])
                     for match in self.commits.values() for row in match["writes"]}
        proof = []
        for row in self.writes:
            sql = row["sourceSql"]
            identity = row["call"], sql["table"], sql["verb"]
            if row["transaction"] is None or identity in committed:
                proof.append({"occurrence": identity[0], "table": identity[1], "verb": identity[2]})
                self.proof.extend(row["evidenceSteps"])
        return proof[:MAX_WRITES]


def project(facts, path, events):
    if set(path["open"]) - BINDING_ASSUMPTIONS or path.get("scope"):
        return
    returned = facts.root_return(path, events, shallow=True)
    status = facts.http_status(returned[1]) if returned else None
    if type(status) is not int or not 200 <= status < 300:
        return
    closed = ClosedReturn(facts, path)
    if not all(closed.event(index, event) for index, event in enumerate(events)) or closed.pending or not closed.awaited:
        return
    if not (writes := closed.completed_writes()):
        return
    guards = facts.path_condition(events)
    marker = {"status": status, "writeOccurrences": [row["occurrence"] for row in writes], "writes": writes}
    facts.add_path("http-local-completion", facts.when(guards) + f"finishes its cited local calls and returns an HTTP {status} result.",
                   path, events, [*(key for _, key in guards), *closed.proof, *returned[1]],
                   authority="RETAINED_CLOSED_HTTP_CONTINUATION", _httpCompletion=marker,
                   uncertainty="The proof assumes the selected providers perform the cited local operations. Provider hooks, "
                               "durability, response transmission and downstream observation order remain open.",
                   **({"condition": " and ".join(text for text, _ in guards)} if guards else {}))
