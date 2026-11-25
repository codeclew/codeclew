"""Join bounded Work propositions by explicit retained protocol identities."""

from collections import defaultdict, deque
from itertools import chain, islice
from urllib.parse import urlsplit
import re


ENTRIES = object()
MAX_MATCHES = 4
MAX_SITES = 8
MAX_TYPED_REGISTRATION_SITES = 24
MAX_LINKS = 16
MAX_WRITE_SITES = 16
MAX_HTTP_WRITES = 2
MAX_NESTED_VISITS = 24
MAX_CHAIN_SOURCES = 36
POLLERS = {"HOSTED_BACKGROUND_WORKER", "SCHEDULED_JOB"}
HTTP_METHODS = {"GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE"}
TOPIC = re.compile(r"[A-Za-z0-9._-]{1,249}")
JAVA_CHANNEL = re.compile(r"[A-Za-z0-9._$-]{1,249}", re.ASCII)
JAVA_EVENT_CLASS = re.compile(r"[A-Za-z_$][\w$]*(?:\.[A-Za-z_$][\w$]*)+", re.ASCII)
JAVA_TYPED_ADAPTER = "EVENTUATE_TRAM_DOMAIN_EVENT"
JAVA_DISPATCH_AUTHORITY = "RETAINED_JAVA_DISPATCHER_WIRING"
TABLE = re.compile(r'[A-Za-z_][\w$]*(?:\.[A-Za-z_][\w$]*){0,2}', re.ASCII)
PARAMETER = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}|:[A-Za-z_][A-Za-z0-9_]*")
KAFKA_ADAPTER = "SPRING_KAFKA_LITERAL_TOPIC/1.0"
HTTP_ADAPTERS = {"SPRING_REST_TEMPLATE_LITERAL_SUFFIX/1.0", "SPRING_REST_CLIENT_URI/2.0"}
PEER_RESPONSES = {"http-outcome", "status_body"}
OUTBOX_WRITE_FACETS = ("further-subscriber-write-lineage", "further-subscriber-write-bypass")
PEER_FACETS = (("lookup-guard-return", "http-peer-guarded-return"),
               ("guard-return-bypass", "http-peer-return-bypass"),
               ("sql_result_flow", "http-peer-value-lineage"),
               ("argument-projection", "http-peer-value-lineage"),
               ("sql-row", "http-peer-value-lineage"),
               ("lookup", "http-peer-value-lineage"),
               ("finite-mapping", "http-peer-value-lineage"),
               ("helper-return", "http-peer-helper-result"),
               ("awaited-list-selection", "http-peer-list-selection"))
PEER_CONTRACTS = {"openapi-declared-response": "http-peer-declared-response",
                  "openapi-overlay-boundary": "http-peer-declaration-boundary",
                  "openapi-response-boundary": "http-peer-declaration-boundary"}
CALLER_PATHS = {"java-guarded-outcome", "java-ordered-effects", "http-outcome", "error-path",
                "effect-order", "handled-exception-continuation", "return_rejection", "await_bypass",
                "exception_route", "request_after_commit_failure", "response_body_read", "status_body",
                "response_decode_failure_outcome",
                "result_continuation", "return", "throw", "EARLY_EXIT", "HARD_EXIT",
                "GUARDED_EFFECTS", "ORDERED_EFFECTS"}
HTTP_OPEN = ("Deployment destination, activation and wire mapping remain unknown. An application exception "
             "does not establish the status or exception observed by the client.")
MESSAGE_OPEN = ("Broker or adapter binding, handler activation, delivery, acknowledgement, ordering and "
                "payload compatibility remain unknown.")
STORAGE_OPEN = ("Database and schema binding, key or row identity, commit, visibility and timing remain unknown.")
JAVA_DISPATCH_OPEN = ("Deployment destination, dispatcher activation, adapter event mapping, delivery, acknowledgement "
                      "and callback completion remain open.")
ASYNC_OPEN = "Any wait for downstream completion and its order relative to an original HTTP response remain open."


def topic(value):
    return value if isinstance(value, str) and TOPIC.fullmatch(value) and value not in {".", ".."} else None


def typed_route(configuration):
    adapter, channel, event = (configuration.get(key) for key in ("adapter", "channel", "eventType"))
    if (adapter == JAVA_TYPED_ADAPTER and isinstance(channel, str) and channel not in {".", ".."}
            and JAVA_CHANNEL.fullmatch(channel) and isinstance(event, str) and JAVA_EVENT_CLASS.fullmatch(event)):
        return adapter, channel, event
    return None


def topics(requirement):
    if requirement.get("entryKind") != "KAFKA_LISTENER":
        return ()
    config = (requirement.get("trigger") or {}).get("configuration") or {}
    values = config.get("topics", ())
    if isinstance(values, str):
        values = (values,)
    return tuple(dict.fromkeys(value for item in values if (value := topic(item)))) if isinstance(values, (list, tuple)) else ()


def request_key(method, path):
    if method not in HTTP_METHODS or not isinstance(path, str) or not 0 < len(path) <= 512:
        return None
    if not path.startswith("/") or path.startswith("//") or "#" in path or any(c.isspace() for c in path):
        return None
    path = path.partition("?")[0]
    parts = tuple(None if PARAMETER.fullmatch(part) else part for part in path[1:].split("/"))
    if any(part is not None and (part in {".", ".."} or any(c in part for c in "{}*\\%")) for part in parts):
        return None
    return method, parts


def uri_path(value):
    if not isinstance(value, str) or any(c.isspace() for c in value):
        return None
    if value.startswith("/"):
        return value
    try:
        url = urlsplit(value)
        if (url.scheme in {"http", "https"} and url.hostname and url.username is None and url.password is None
                and (url.port is None or 0 < url.port <= 65535)):
            return url.path or "/"
    except ValueError:
        return None
    return None


class Routes:
    def __init__(self, requirements):
        self.rows = defaultdict(dict)
        for row in requirements:
            if row.get("entryKind") != "HTTP_ENDPOINT":
                continue
            if key := request_key(row.get("method"), row.get("path")):
                method, pieces = key
                branch = self.rows[method, len(pieces)]
                for piece in pieces:
                    branch = branch.setdefault(piece, {})
                branch.setdefault(ENTRIES, []).append(row)

    def matching(self, key):
        method, pieces = key
        branches = [(self.rows.get((method, len(pieces)), {}), False)]
        for piece in pieces:
            next_branches = []
            for branch, depends_on_value in branches:
                keys = branch.keys() if piece is None else (piece, None)
                for key in keys:
                    if key is not ENTRIES and key in branch:
                        next_branches.append((branch[key], depends_on_value or piece is None and key is not None))
                    if len(next_branches) > MAX_MATCHES * MAX_MATCHES:
                        return [], True
            branches = next_branches
        whole = list(islice((row for branch, depends in branches if not depends
                            for row in branch.get(ENTRIES, ())), MAX_MATCHES + 1))
        if not whole:
            return [], False
        literals = list(islice((row for branch, depends in branches if depends
                               for row in branch.get(ENTRIES, ())), MAX_MATCHES + 1))
        found = whole + literals
        return found[:MAX_MATCHES], len(found) > MAX_MATCHES


def cite_refs(sources, refs, *, source_ids=None):
    return list(dict.fromkeys(sources[ref]["handle"] for ref in refs if ref in sources
                              and (source_ids is None or sources[ref].get("id") in source_ids)))


def stored_face(fact):
    return {"sources": fact["sources"], "witnesses": fact.get("witnesses", ()),
            **({"condition": fact["condition"]} if fact.get("condition") else {}),
            "text": fact["text"], "uncertainty": fact.get("uncertainty", "")}


def source_exits(process, sources):
    seen = set()
    for event in process["facts"]:
        if (event.get("declaration") != process["root"] or event.get("kind") != "EVENT"
                or event.get("event") not in {"RETURN", "THROW"}):
            continue
        for ref in event["sources"]:
            if ref not in sources:
                continue
            source = sources[ref]
            literal = " ".join(source["text"].split())
            if len(literal) > 150 or not re.match(r"^(?:return|throw)\b", literal) or "`" in literal:
                continue
            key = source["handle"], literal
            if key not in seen:
                seen.add(key)
                yield {"sources": [source["handle"]],
                       "text": f"The handler contains the source exit `{literal}`.",
                       "uncertainty": "Reaching the exit and completing its operand evaluation are conditional."}


def response_representatives(rows):
    groups, status_rows = defaultdict(list), {}
    for row in rows:
        status = row.get("_httpStatus")
        family = status // 100 if type(status) is int and 100 <= status <= 599 else None
        groups[family].append(row)
        if family:
            status_rows.setdefault(status, row)
    priority = (4, 5, 2, 3, 1, None)
    selected, seen = [], set()
    for row in chain((groups[family][0] for family in priority if groups[family]),
                     status_rows.values(), (row for family in priority for row in groups[family])):
        if id(row) not in seen:
            seen.add(id(row))
            selected.append(row)
    return selected


def write_coordinates(site):
    if not site:
        return None
    first, last = site.get("startByte"), site.get("endByte")
    identity = tuple(site.get(key) for key in ("file", "revision", "textDigest"))
    if type(first) is type(last) is int and 0 <= first < last and all(identity):
        return (*identity, first, last)
    return None


def record(requirement, process, context, facts, exact):
    """Retain only small indexes needed after all ordinary projections complete."""
    sources = {ref: {**exact[row["id"]], "handle": row["id"]}
               for row in context["sourceRanges"] for ref in row["references"] if row["id"] in exact}
    trigger = process.get("trigger") or {}
    subscription_refs = [*trigger.get("subscriptions", ()), *trigger.get("subscriptionSources", ())]
    assembly = trigger.get("assemblySources", ()) if trigger.get("kind") == "JAVA_TYPED_DISPATCH" else ()
    trigger_refs = tuple(dict.fromkeys(filter(None, (trigger.get("registration"), trigger.get("callback"),
                                                    *subscription_refs, *assembly))))
    for ref in trigger_refs:
        if ref in sources or ref not in process["sources"]:
            continue
        witness = process["sources"][ref]
        if handle := context["_bindWitness"](witness):
            sources[ref] = {**witness, "handle": handle}
    registration = cite_refs(sources, [trigger.get("registration"), *subscription_refs, *assembly])
    if not registration:
        registration = cite_refs(sources, process["entrySources"],
                                 source_ids=set(requirement.get("registrationSourceIds", ())) or None)
    registration_limit = MAX_TYPED_REGISTRATION_SITES if trigger.get("kind") == "JAVA_TYPED_DISPATCH" else MAX_SITES
    registered = registration[:registration_limit]
    topic_proven = trigger.get("authority") != "KAFKAJS_RETAINED_SOURCE" or (
        bool(trigger.get("subscriptions")) and len(registration) <= MAX_SITES
        and all(ref in sources for ref in subscription_refs))
    body = cite_refs(sources, [ref for fact in process["facts"]
                              if fact["kind"] == "BODY" and fact.get("declaration") == process["root"]
                              for ref in fact["sources"]])[:1]
    callbacks = cite_refs(sources, [trigger.get("callback")])
    typed_registration = None
    if (requirement.get("entryKind") == trigger.get("kind") == "JAVA_TYPED_DISPATCH"
            and trigger.get("authority") == JAVA_DISPATCH_AUTHORITY
            and trigger.get("registration") in sources and callbacks
            and all(ref in sources for ref in assembly) and len(registration) <= registration_limit):
        typed_registration = typed_route(trigger.get("configuration") or {})
    calls, publishes, typed_publishes, reads, writes = {}, {}, {}, {}, {}

    def keep(index, key, face, limit=MAX_SITES):
        if len(index) < limit:
            index.setdefault(key, face)

    for event in process["facts"]:
        if event.get("event") != "CALL" or event.get("kind") != "EVENT" or event.get("resolution") != "COMPILER_EXACT":
            continue
        if not (refs := cite_refs(sources, event.get("sources", ()))):
            continue
        http, kafka = event.get("http") or {}, event.get("kafka") or {}
        face = {"sources": refs, "site": refs[0]}
        if http.get("adapter") in HTTP_ADAPTERS:
            key = request_key(http.get("method"), http.get("path"))
            keep(calls, (refs[0], key), {**face, "key": key, "path": http.get("path"), "precise": True})
        if kafka.get("adapter") == KAFKA_ADAPTER:
            value = topic(kafka.get("topic"))
            keep(publishes, (refs[0], value), {**face, "topic": value, "precise": True})

    write_focus, poll_paths = defaultdict(lambda: defaultdict(list)), defaultdict(dict)
    write_statements, write_shapes, write_sites = defaultdict(list), defaultdict(set), defaultdict(set)
    response_facts, completions = defaultdict(list), defaultdict(list)
    for fact in facts:
        response_facts[fact["kind"]].append(fact)
        if fact["kind"] == "http-local-completion" and (proof := fact.get("_httpCompletion")):
            status = proof.get("status")
            if type(status) is int and 200 <= status < 300:
                for write in proof.get("writes", ()):
                    identity = write.get("occurrence"), write.get("table"), write.get("verb")
                    if all(isinstance(part, str) and part for part in identity):
                        completions[identity].append(fact)
        if (pair := fact.get("_pollPublication")) and pair.get("readOccurrence") and pair.get("publicationOccurrence"):
            poll_paths[pair["readOccurrence"]].setdefault(pair["publicationOccurrence"], stored_face(fact))
        if (focus := fact.get("_writeFocus")) and focus.get("occurrence"):
            write_focus[focus["occurrence"]][focus.get("topic")].append(fact)
        if not (link := fact.get("_edge")):
            continue
        site = link["site"]
        face = {**stored_face(fact), "site": site,
                "occurrence": link.get("occurrence"), "precise": not link.get("occurrence")}
        if link["kind"] == "HTTP":
            path = uri_path(link.get("uri"))
            key = request_key(link.get("method"), path)
            if key and None in key[1] and not link.get("encodedSegment"):
                key = None
            keep(calls, (face["occurrence"] or site, key), {**face, "key": key, "path": path})
        elif link["kind"] == "PUBLISH":
            value = topic(link.get("topic"))
            keep(publishes, (face["occurrence"] or site, value), {**face, "topic": value})
        elif link["kind"] == "JAVA_TYPED_PUBLISH" and link.get("adapter") == JAVA_TYPED_ADAPTER:
            route = typed_route(link)
            keep(typed_publishes, (face["occurrence"] or site, route, face.get("condition")),
                 {**face, "route": route, "compiler": fact["authority"] == "COMPILER_SELECTED_JAVA_PUBLICATION"})
        elif link["kind"] in {"SELECT", "INSERT", "UPDATE", "DELETE"}:
            table = link.get("table")
            # Quoted identifiers preserve spelling. Conservatively leave the
            # dialect-dependent equivalence of alternate spellings open.
            if isinstance(table, str) and TABLE.fullmatch(table):
                identity = (face["occurrence"] or site, table, link["kind"])
                indexed = {**face, "table": table, "verb": link["kind"]}
                index = reads if link["kind"] == "SELECT" else writes
                slot = (*identity, face.get("condition"))
                if index is writes:
                    indexed["writeSite"] = fact.get("_writeSite")
                    write_shapes[face["occurrence"]].add((table, link["kind"]))
                    if coordinate := write_coordinates(indexed["writeSite"]):
                        write_sites[face["occurrence"]].add(coordinate)
                        earlier = writes.get(slot)
                        if earlier and not earlier.get("writeSite") and earlier["site"] == indexed["site"]:
                            writes[slot] = {**earlier, "writeSite": indexed["writeSite"]}
                    write_statements[identity].append({**indexed, "bound": fact.get("_writeBinding") is True})
                keep(index, slot, indexed, MAX_WRITE_SITES)

    registration_witnesses = []
    for ref in trigger_refs:
        if ref in sources and sources[ref]["handle"] in registered + callbacks:
            source = sources[ref]
            registration_witnesses.append({"cite": source["handle"], **{key: source[key] for key in (
                "file", "revision", "startLine", "endLine", "startByte", "endByte", "textDigest") if key in source}})
    return {"requirement": requirement, "registration": registered, "body": callbacks or body,
            "registrationWitnesses": registration_witnesses, "topicProven": topic_proven,
            "calls": list(calls.values()), "publishes": list(publishes.values()), "pollPaths": dict(poll_paths),
            "typedPublishes": list(typed_publishes.values()), "typedRegistration": typed_registration,
            "reads": list(reads.values()), "writes": [
                {**write, "focus": write_focus.get(write["occurrence"], {}),
                 "uniqueWrite": len(write_shapes[write["occurrence"]]) == 1 and len(write_sites[write["occurrence"]]) <= 1}
                for write in writes.values()],
            "writeStatements": dict(write_statements), "completions": dict(completions),
            "responses": response_representatives(fact for fact in facts if fact["kind"] in PEER_RESPONSES),
            "facets": [(target, response_representatives(response_facts[kind]))
                       for kind, target in PEER_FACETS if response_facts[kind]],
            "commits": response_facts["http-commit-before-response"],
            "bypasses": response_facts["conditional-no-write"],
            "exits": list(source_exits(process, sources))[:MAX_MATCHES],
            "paths": [fact for fact in facts if fact["kind"] in CALLER_PATHS]}


def project(requirements, records, *, contracts=None, isolated=False):
    """Index each protocol once and bound per-page evidence fan-out."""
    routes = Routes(requirements)
    contracts = contracts or {}
    subscriptions, typed_dispatchers = defaultdict(list), defaultdict(list)
    readers, reader_services = defaultdict(dict), defaultdict(dict)
    for row in requirements:
        for name in topics(row):
            subscriptions[name].append(row)
    for identity, row in records.items():
        if route := row["typedRegistration"]:
            typed_dispatchers[route].append(identity)
        for read in row["reads"]:
            if row["body"] and row["registration"]:
                service, table = row["requirement"]["service"], read["table"]
                kind = row["requirement"].get("entryKind")
                family = ("http" if kind == "HTTP_ENDPOINT" else "poll" if kind in POLLERS else "other")
                group = readers[service, table].setdefault(family, [])
                if len(group) < MAX_WRITE_SITES:
                    group.append((identity, read))
                reader_services[table].setdefault(service, read)
    result, seen, requester_links = defaultdict(list), defaultdict(set), defaultdict(list)
    nested_visits = defaultdict(int)
    first_outbox_hops, guarded_terminal_effects = defaultdict(list), {}

    def emit(owner, kind, text, uncertainty, *faces, status="SOURCE_QUALIFIED_CANDIDATE",
             authority="RETAINED_SOURCE_EDGE"):
        if not (sources := list(dict.fromkeys(ref for face in faces for ref in face["sources"]))):
            return
        conditions = list(dict.fromkeys(face["condition"] for face in faces if face.get("condition")))
        witnesses = {tuple(sorted(w.items())): w for face in faces for w in face.get("witnesses", ())}
        key = (kind, text, tuple(sorted(sources)), tuple(conditions), status, authority, uncertainty,
               frozenset(witnesses))
        if key in seen[owner]:
            return
        seen[owner].add(key)
        row = {"kind": kind, "authority": authority, "status": status,
               "text": text, "uncertainty": uncertainty, "sources": sources,
               **({"condition": " ".join(conditions)} if conditions else {}),
               **({"witnesses": list(witnesses.values())} if witnesses else {})}
        result[owner].append(row)
        return row

    def registration(row):
        return {"sources": row["registration"], "witnesses": [
            witness for witness in row["registrationWitnesses"] if witness["cite"] in row["registration"]]}

    def callback(row):
        return {"sources": row["body"], "witnesses": [
            witness for witness in row["registrationWitnesses"] if witness["cite"] in row["body"]]}

    def continuation(row, call):
        if not call["precise"]:
            return row.get("_edgeCause") == call["occurrence"]
        return call["site"] in row["sources"]

    def local_readers(writer, table, families=("http", "poll", "other")):
        service = records[writer]["requirement"]["service"]
        indexed = readers[service, table]
        return ((reader, read) for family in families for reader, read in indexed.get(family, ()) if reader != writer)

    def poll_publications(poller, read):
        paths = poller["pollPaths"].get(read["occurrence"], {})
        return [(publication, paths[identity]) for publication in poller["publishes"]
                if (identity := publication["occurrence"]) and identity in paths][:2]

    def entry_kind(owner):
        return records[owner]["requirement"].get("entryKind")

    def readbacks(owner, writer_id, effect, *evidence, prefix, uncertainty, nested=False):
        table = effect["table"]
        matched = list(islice(local_readers(writer_id, table, ("http",)), MAX_MATCHES))
        if not matched and not nested:
            matched = list(islice(local_readers(writer_id, table), MAX_MATCHES))
        for reader, read in matched:
            observed = records[reader]
            name = observed["requirement"]["page"]
            observed_outcome = ("can observe the row's absence once the deletion becomes visible and while it remains deleted"
                                if effect["verb"] == "DELETE" else
                                "can see the affected row after the change becomes visible")
            emit(owner, "further-subscriber-readback" if nested else "message-storage-reader", prefix
                 + f"the retained read in `{name}` {observed_outcome} if it uses the same database and row key in `{table}`.",
                 uncertainty, *evidence, effect, registration(observed), read)

    def qualified_subscribers(publisher, topic):
        candidates = subscriptions.get(topic, ())
        if len(candidates) > MAX_MATCHES:
            return []
        return [row for row in candidates if row["id"] != publisher and row["id"] in records
                and records[row["id"]]["registration"] and records[row["id"]]["topicProven"] and records[row["id"]]["body"]
                and ((row.get("trigger") or {}).get("configuration") or {}).get("autoStartup") not in (False, "false")]

    def focused_rows(write, topic):
        # A topic-specific callback path is kept separate from other subscriptions.
        focus = write["focus"]
        return focus.get(topic, focus.get(None, ()))

    def incompatible_write(write, topic):
        rows = focused_rows(write, topic)
        verdicts = {row["_writeFocus"].get("compatibility") for row in rows} - {None}
        if verdicts == {"INCOMPATIBLE"}:
            return next(row for row in rows if row["_writeFocus"].get("compatibility") == "INCOMPATIBLE")
        return None

    def selected_writes(write, topic):
        rows = focused_rows(write, topic)
        guard = write.get("condition")
        for kind in ("write_request", "sql-write", "write_guard", "write_lineage"):
            candidates = [item for item in rows if item["kind"] == kind
                          and item["_writeFocus"].get("compatibility") != "INCOMPATIBLE"]
            independent = write["uniqueWrite"] and kind in {"write_request", "sql-write"}
            variants = {}
            for row in candidates:
                premise = row.get("condition")
                if premise != guard and not (independent and premise):
                    continue
                if premise in variants:
                    continue
                if independent and premise != guard:
                    # This replay is its own reached path at the exact slot.
                    # Do not append conditions or bindings from another path.
                    variants[premise] = {
                        **write, "text": row["text"], "condition": premise,
                        "sources": list(dict.fromkeys([write["site"], *row["sources"]])),
                        "witnesses": row.get("witnesses", ()),
                        "uncertainty": " ".join(filter(None, (write["uncertainty"], row.get("uncertainty")))),
                    }
                else:
                    variants[premise] = {
                        **write, "sources": list(dict.fromkeys([*write["sources"], *row["sources"]])),
                        "witnesses": [*write.get("witnesses", ()), *row.get("witnesses", ())],
                        **({"condition": premise} if premise else {}),
                    }
            if variants and (independent or len(variants) == 1):
                return variants.values() if isolated else [next(iter(variants.values()))]
        return (write,)

    def add_focused(owner, write, topic, message, qualifier, *proof):
        details = focused_rows(write, topic)
        for kind in ("write_lineage", "write_guard", "write_commit", "transactional-write-completion"):
            if detail := next((row for row in details if row["kind"] == kind
                               and (not write.get("condition") or not row.get("condition")
                                    or row["condition"] == write["condition"])), None):
                emit(owner, "message-peer-write-detail", message + " " + detail["text"],
                     qualifier + " " + detail.get("uncertainty", ""), *proof, detail)

    def outbox_write_facets(owner, write, topic, writer_page, poller, read, qualifier, *upstream):
        site = write.get("writeSite")
        if not write["uniqueWrite"] or not (coordinate := write_coordinates(site)):
            return
        details = defaultdict(list)
        for row in focused_rows(write, topic):
            if (write_coordinates(row.get("_writeSite")) == coordinate
                    and row["_writeFocus"].get("compatibility") != "INCOMPATIBLE"):
                details[row["kind"]].append(row)
        topic_scope = f" on `{topic}`" if topic else ""
        scope = (f"If delivery{topic_scope} reaches `{writer_page}`, this path concerns its cited "
                 f"{write['verb']} on `{write['table']}`. The separate poller `{poller['requirement']['page']}` "
                 "can select the row if that write is reached and becomes visible. ")
        # Carry the relation's witnesses but leave the selected write and the
        # later poll guards out of each independent lineage or bypass path.
        anchor = {"sources": [site["cite"]], "witnesses": [site]}
        association = (anchor, registration(poller), {**read, "condition": None})
        groups = (details["write_lineage"] or details["write_formation"],
                  [*details["write_skip"], *details["conditional-no-write"]])
        for kind, candidates in zip(OUTBOX_WRITE_FACETS, groups, strict=True):
            alternatives = {}
            for row in candidates:
                alternatives.setdefault((row["text"], row.get("condition")), row)
            for detail in islice(alternatives.values(), MAX_MATCHES):
                uncertainty = " ".join(filter(None, (qualifier, detail.get("uncertainty"))))
                emit(owner, kind, scope + detail["text"], uncertainty, *upstream, *association, detail)

    def http_writes(target):
        groups = []
        for variants in target["writeStatements"].values():
            candidates = sorted(variants, key=lambda row: (not row["bound"], not row.get("condition")))
            groups.append(candidates)
        groups.sort(key=lambda rows: (not rows[0]["bound"], not rows[0].get("condition")))
        # First preserve distinct call sites. Use a spare slot for a distinct
        # guarded binding from the same SQL call site.
        yield from chain((rows[0] for rows in groups), (row for rows in groups for row in rows[1:]))

    def representative_writes(writer, topic=None):
        groups = defaultdict(list)
        for write in records[writer]["writes"]:
            if incompatible_write(write, topic):
                groups[3].append(write)
                continue
            table = write["table"]
            rank = (0 if next(local_readers(writer, table, ("http",)), None) else
                    1 if write["verb"] != "DELETE" and any(poll_publications(records[reader], read)
                                                          for reader, read in local_readers(writer, table, ("poll",))) else 2)
            groups[rank].append(write)
        groups[2].sort(key=lambda effect: not effect.get("condition"))
        selected = [group[0] for _, group in sorted(groups.items()) if group]
        if len(selected) < 4:
            selected.extend(islice((write for _, group in sorted(groups.items()) for write in group[1:]),
                                   4 - len(selected)))
        variants = {}
        for write in selected:
            for branch in selected_writes(write, topic):
                key = branch["occurrence"] or branch["site"], branch["table"], branch["verb"], branch.get("condition")
                variants.setdefault(key, branch)
        return variants.values()

    def further_outbox(owner, writer, write, topic, visited, evidence):
        queue = deque([(writer, write, topic, visited, evidence, True)])
        while queue:
            writer, write, topic, visited, evidence, initial = queue.popleft()
            if write["verb"] == "DELETE":
                continue
            table = write["table"]
            candidates = [(publisher, read, poll_publications(records[publisher], read))
                          for publisher, read in local_readers(writer, table, ("poll",))]
            candidates.sort(key=lambda row: not row[2])
            for publisher, read, paired in candidates[:MAX_MATCHES]:
                poller = records[publisher]
                title = poller["requirement"]["page"]
                frontier = (write, registration(poller), {**read, "condition": None})
                if not paired:
                    emit(owner, "further-outbox-boundary",
                         f"The cited query in `{title}` has no retained row-to-publication continuation.",
                         STORAGE_OPEN + " " + MESSAGE_OPEN, *frontier, status="UNKNOWN")
                    continue
                proof_sources = {ref for face in (*evidence, *frontier) for ref in face["sources"]}
                if publisher in visited:
                    emit(owner, "further-outbox-boundary", f"The next poll can revisit the cited root in `{title}`.",
                         STORAGE_OPEN + " " + MESSAGE_OPEN, *frontier, status="UNKNOWN")
                    continue
                if nested_visits[owner] >= MAX_NESTED_VISITS or len(proof_sources) >= MAX_CHAIN_SOURCES:
                    emit(owner, "further-outbox-limit-boundary",
                         f"More continuation remains beyond the prepared path limit at the cited poll root in `{title}`.",
                         STORAGE_OPEN + " " + MESSAGE_OPEN, *frontier, status="PREPARATION_LIMIT")
                    return
                for publication, selection in paired:
                    publication = {**publication, "condition": None}
                    subscribers = qualified_subscribers(publisher, publication["topic"])
                    if not subscribers:
                        emit(owner, "further-outbox-boundary", "The next publication has no bounded set of proven active callback candidates.",
                             MESSAGE_OPEN, *evidence, *frontier, publication, selection, status="UNKNOWN")
                    for subscriber in subscribers:
                        target = subscriber["id"]
                        if nested_visits[owner] >= MAX_NESTED_VISITS:
                            emit(owner, "further-outbox-limit-boundary",
                                 f"More continuation remains beyond the prepared visit limit from the cited poll root in `{title}`.",
                                 MESSAGE_OPEN + " " + STORAGE_OPEN, *frontier, publication, selection,
                                 status="PREPARATION_LIMIT")
                            return
                        nested_visits[owner] += 1
                        continuation = (*evidence, *frontier, publication, selection, registration(records[target]))
                        if target in visited or target == writer:
                            emit(owner, "further-outbox-boundary", "The next publication can revisit a previously considered callback.",
                                 MESSAGE_OPEN, *continuation, status="UNKNOWN")
                            continue
                        next_topic = publication["topic"]
                        prior_topic = f"After delivery on `{topic}`, " if topic else ""
                        writer_page = records[writer]["requirement"]["page"]
                        prefix = (prior_topic + f"if the cited {write['verb']} on `{table}` in `{writer_page}` becomes visible "
                                  f"and the independently registered poller in `{title}` selects the same key, "
                                  f"it can reach the publication on `{next_topic}`. ")
                        qualifier = (STORAGE_OPEN + " " + MESSAGE_OPEN + " Other payload fields and wire mapping remain open. "
                                     + selection["uncertainty"] + " " + ASYNC_OPEN)
                        proof = (*continuation, callback(records[target]))
                        if len({ref for face in proof for ref in face["sources"]}) > MAX_CHAIN_SOURCES:
                            emit(owner, "further-outbox-limit-boundary",
                                 f"More continuation remains beyond the source limit from the cited poll root in `{title}`.",
                                 qualifier, *frontier, publication, status="PREPARATION_LIMIT")
                            return
                        hop = emit(owner, "further-subscriber-outbox", prefix
                                   + selection["text"] + f" `{subscriber['page']}` has the cited registration for that topic.",
                                   qualifier, *proof)
                        if hop and initial:
                            first_outbox_hops[owner].append(hop)
                            outbox_write_facets(owner, write, topic, writer_page, poller, read, qualifier, *evidence)
                        for effect in representative_writes(target, next_topic):
                            if conflict := incompatible_write(effect, next_topic):
                                emit(owner, "message-peer-write-boundary", prefix + conflict["text"],
                                     qualifier, *proof, conflict, status="INCOMPATIBLE_IN_RETAINED_PATHS")
                                continue
                            emit(owner, "further-subscriber-effect",
                                 prefix + f"If delivery reaches the cited callback in `{subscriber['page']}`, it can reach this write. "
                                 + effect["text"], qualifier, *proof, effect)
                            add_focused(owner, effect, next_topic, prefix, qualifier, *proof)
                            readbacks(owner, target, effect, *proof, prefix=prefix, uncertainty=qualifier, nested=True)
                            queue.append((target, effect, next_topic, visited | {writer, publisher, target}, proof, False))

    for owner, local in records.items():
        service = local["requirement"]["service"]
        for call in local["calls"]:
            matching, overflow = routes.matching(call["key"]) if call["key"] else ([], False)
            boundary = " ".join(filter(None, (HTTP_OPEN, call.get("uncertainty"))))
            peers = [entry for entry in matching if entry["service"] != service]
            cited = [registration(records[row["id"]]) for row in peers if row["id"] in records]
            if overflow or len(matching) > 1:
                emit(owner, "http-peer-boundary", "The request route has multiple retained handler candidates.",
                     boundary, call, *cited, status="AMBIGUOUS")
                continue
            if not peers or not cited or not cited[0]["sources"]:
                if not matching or peers:
                    emit(owner, "http-peer-boundary", "A source-qualified peer handler is unavailable for this retained request.",
                         boundary, call, status="UNKNOWN")
                continue
            peer = peers[0]
            target = records[peer["id"]]
            method = call["key"][0]
            label = f"`{method} {call['path'].partition('?')[0]}`"
            name = f"`{peer['service']}` (`{peer['page']}`)"
            predicate = (f"If the deployed request reaches the matching handler in {name}, "
                         "the following are separately retained source outcomes.")
            requester_links[peer["id"]].append((owner, call, cited[0], boundary))
            emit(owner, "http-peer", f"The request at {label} has a route candidate in {name}.", boundary, call, cited[0])
            emit(peer["id"], "http-peer", f"`{service}` has a retained request matching this source route at {label}.",
                 boundary, call, cited[0])
            for commit in (target["commits"] if isolated else target["commits"][:2]):
                emit(owner, "http-peer-commit-before-response", predicate + " " + commit["text"],
                     boundary + " " + commit.get("uncertainty", ""), call, cited[0], commit)
            for bypass in (target["bypasses"] if isolated else target["bypasses"][:2]):
                emit(owner, "http-peer-conditional-no-write", predicate + " " + bypass["text"],
                     boundary + " " + bypass.get("uncertainty", ""), call, cited[0], bypass)
            for declaration in contracts.get(peer["id"], ()):
                kind = PEER_CONTRACTS[declaration["kind"]]
                emit(owner, kind, f"The matching source route in {name} has this declaration evidence. "
                     + declaration["text"], boundary + " " + declaration["uncertainty"],
                     call, cited[0], declaration, status=declaration["status"], authority=declaration["authority"])
            for kind, facets in target["facets"]:
                for facet in (facets if isolated else facets[:1]):
                    emit(owner, kind, predicate + " " + facet["text"],
                         boundary + " " + facet.get("uncertainty", ""), call, cited[0], facet)
            for write in islice(http_writes(target), None if isolated else MAX_HTTP_WRITES):
                scope = (f"If the request reaches the matched handler in {name}, it can take "
                         f"the cited {write['verb']} path on `{write['table']}`. ")
                qualifier = " ".join(filter(None, (boundary, STORAGE_OPEN, write["uncertainty"])))
                emit(owner, "http-peer-sql-write", scope + write["text"], qualifier, call, cited[0], write)
                focus = next((item["focus"] for item in target["writes"]
                              if item["occurrence"] == write["occurrence"] and item["site"] == write["site"]), {})
                for kind in ("write_lineage", "write_sql"):
                    details = (row for row in focus.get(None, ()) if row["kind"] == kind
                               and (not write.get("condition") or row.get("condition") == write["condition"]))
                    for detail in (details if isolated else islice(details, 1)):
                        emit(owner, "http-peer-write-binding", scope + detail["text"],
                             qualifier + " " + detail.get("uncertainty", ""), call, cited[0], write, detail)
            outcomes = target["responses"] or target["exits"]
            for outcome in (outcomes if isolated else outcomes[:MAX_MATCHES]):
                emit(owner, "http-peer-outcome", predicate + " " + outcome["text"],
                     boundary + " " + outcome.get("uncertainty", ""), call, cited[0], outcome)
            if not (target["responses"] or target["commits"] or target["exits"]):
                emit(owner, "http-peer-outcome-boundary", f"An observable handler return or exception for {name} remains open.",
                     boundary, call, cited[0], status="UNKNOWN")
            local_paths = [row for row in local["paths"] if continuation(row, call)]
            decoding = (row for row in local_paths if row["kind"] == "response_decode_failure_outcome"
                        and row.get("_edgePhase") == "RESPONSE_DECODING")
            # Extra peer commit paths remain separate facts. Do not manufacture
            # a product of unrelated response decoders and callee commit paths.
            for outcome in (decoding if isolated else islice(decoding, MAX_MATCHES)):
                for commit in target["commits"][:1]:
                    emit(owner, "http-failure-after-peer-commit",
                         f"If the matched handler in {name} takes its cited commit-before-response path, "
                         f"the caller can still fail while decoding the resulting response. {outcome['text']}",
                         boundary + " The upstream write may already be committed. The caller failure does not "
                         "establish upstream rollback, durability or the wire status. " + outcome.get("uncertainty", ""),
                         call, cited[0], commit, outcome)
            for outcome in (local_paths if isolated else local_paths[:MAX_MATCHES]):
                if outcome["kind"] == "response_decode_failure_outcome" and target["commits"]:
                    continue
                emit(owner, "http-caller-continuation", outcome["text"],
                     boundary + " Persistence and rollback of any earlier effects remain unknown. "
                     + outcome.get("uncertainty", ""), call, cited[0], outcome)

        for publication in local["typedPublishes"]:
            boundary = " ".join(filter(None, (JAVA_DISPATCH_OPEN, publication.get("uncertainty"))))
            route = publication["route"]
            candidates = [peer for peer in typed_dispatchers.get(route, ()) if peer != owner]
            if not candidates or len(candidates) > MAX_MATCHES:
                text = ("Multiple retained Java dispatcher candidates match this protocol, channel and event class."
                        if candidates else "A source-qualified Java dispatcher for this publication remains open.")
                emit(owner, "typed-message-peer-boundary", publication["text"] + " " + text, boundary, publication,
                     status="AMBIGUOUS" if candidates else "UNKNOWN")
                continue
            _, channel, event = route
            basis = ("The compiler selected the publication call." if publication["compiler"]
                     else "The retained publication receiver and operands corroborate the library call.")
            for peer in candidates:
                receiver = records[peer]
                target = receiver["requirement"]
                title = f"`{target['service']}` (`{target['page']}`)"
                evidence = (publication, registration(receiver), callback(receiver))
                emit(owner, "typed-message-peer", basis + f" Channel `{channel}` and qualified event `{event}` "
                     f"match the retained dispatcher registration in {title}.",
                     boundary, *evidence)
                emit(peer, "typed-message-peer", f"`{service}` has a retained submission for channel `{channel}` "
                     f"and qualified event `{event}`. " + basis,
                     boundary, *evidence)

        for publication in local["publishes"]:
            name = publication["topic"]
            matching = subscriptions.get(name, ())
            boundary = " ".join(filter(None, (MESSAGE_OPEN, publication.get("uncertainty"))))
            label = f"topic `{name}`" if name else "a topic that remains open"
            if not matching or len(matching) > MAX_MATCHES:
                text = (f"The publication on {label} has multiple retained subscriber candidates." if matching else
                        f"The publication names {label}. A source-qualified subscriber is unavailable.")
                emit(owner, "message-peer-boundary", text, boundary, publication,
                     status="AMBIGUOUS" if matching else "UNKNOWN")
                continue
            eligible = [entry for entry in matching if entry["id"] != owner and entry["id"] in records
                        and records[entry["id"]]["registration"] and records[entry["id"]]["topicProven"]]
            if not eligible:
                emit(owner, "message-peer-boundary", f"The publication names {label}. A source-qualified subscriber is unavailable.",
                     boundary, publication, status="UNKNOWN")
                continue
            for entry in eligible:
                peer = entry["id"]
                target = records[peer]
                config = (entry.get("trigger") or {}).get("configuration") or {}
                disabled = config.get("autoStartup") in (False, "false")
                result_status = "DISABLED_IN_SOURCE" if disabled else "SOURCE_QUALIFIED_CANDIDATE"
                title = f"`{entry['service']}` (`{entry['page']}`)"
                text = (f"The publication and subscription in {title} name {label}."
                        + (" The retained declaration disables automatic startup." if disabled else
                           " The retained callback is eligible if the subscription runs and delivery reaches it."
                           if target["body"] else " An exact callback body is unavailable."))
                emit(owner, "message-peer", text, boundary,
                     publication, registration(target), callback(target), status=result_status)
                emit(peer, "message-peer", f"`{service}` has a retained publication on {label}.", boundary,
                     publication, registration(target), callback(target), status=result_status)
                if disabled or not target["body"]:
                    continue
                for outcome in [row for row in local["paths"] if continuation(row, publication)][:2]:
                    emit(owner, "message-publication-continuation", outcome["text"],
                         boundary + " Persistence and rollback of earlier effects remain unknown. "
                         + outcome.get("uncertainty", ""), publication, registration(target), outcome)
                proof = (publication, registration(target), callback(target))
                for effect in representative_writes(peer, name):
                    if conflict := incompatible_write(effect, name):
                        emit(owner, "message-peer-write-boundary", conflict["text"], boundary, *proof, conflict,
                             status="INCOMPATIBLE_IN_RETAINED_PATHS")
                        continue
                    emit(owner, "message-peer-effect", f"If the subscribed callback in {title} reaches this write, "
                         + effect["text"], boundary + " " + STORAGE_OPEN, *proof, effect)
                    add_focused(owner, effect, name, f"The selected callback in {title} has this retained write path.",
                                boundary, *proof)
                    readbacks(owner, peer, effect, *proof, prefix="After the subscribed callback reaches the write, ",
                              uncertainty=boundary + " " + STORAGE_OPEN)
                    further_outbox(owner, peer, effect, name, {owner, peer}, proof)

        for write in local["writes"]:
            table = write["table"]
            other_services = list(islice((read for owner_service, read in reader_services[table].items()
                                         if owner_service != service), MAX_MATCHES))
            if other_services:
                emit(owner, "storage-peer-boundary", f"Other services also name `{table}` in a read. "
                     "A shared database or schema binding needs independent evidence.",
                     STORAGE_OPEN, write, *other_services, status="UNKNOWN")
            matches = list(islice(local_readers(owner, table), MAX_MATCHES))
            pollers = list(islice(local_readers(owner, table, ("poll",)), MAX_MATCHES))
            for reader, read in [*matches, *(row for row in pollers if row not in matches)]:
                target = records[reader]
                name = target["requirement"]["page"]
                emit(owner, "storage-reader", f"The `{write['verb']}` and the retained query in `{name}` "
                     f"name `{write['table']}`. The reader is eligible if both operations use the same database and row key.",
                     STORAGE_OPEN, write, registration(target), read)
                if write["verb"] == "DELETE" or entry_kind(reader) not in POLLERS:
                    continue
                if not (paired := poll_publications(target, read)):
                    emit(owner, "storage-message-boundary", f"The cited query in `{name}` has no retained "
                         "row-to-publication continuation.", MESSAGE_OPEN + " " + STORAGE_OPEN,
                         write, registration(target), read, status="UNKNOWN")
                read = {**read, "condition": None}
                for publication, selection in paired:
                    publication = {**publication, "condition": None}
                    emit(owner, "storage-poll-activation",
                         f"The cited write names the same table as an independently registered poller in `{name}`. "
                         + selection["text"], STORAGE_OPEN + " " + MESSAGE_OPEN + " " + ASYNC_OPEN,
                         write, registration(target), read, publication, selection)
                    for completion in local["completions"].get((write["occurrence"], table, write["verb"]), ()):
                        emit(owner, "local-response-independent-poll", completion["text"] +
                             f" The related poller in `{name}` has its own registration and can publish if it selects the visible row. "
                             "The cited local response path can complete independently of that polling and delivery.",
                             " ".join(filter(None, (completion.get("uncertainty"), STORAGE_OPEN, MESSAGE_OPEN,
                                                    "Timing relative to the local HTTP response remains open."))),
                             completion, registration(target), read, publication, {**selection, "condition": None})
                    subscribers = subscriptions.get(publication["topic"], ())
                    if len(subscribers) > MAX_MATCHES:
                        emit(owner, "storage-message-boundary", f"Multiple retained subscriptions follow the "
                             f"publication candidate in `{name}`.", MESSAGE_OPEN, write, registration(target), read,
                             publication, selection, status="AMBIGUOUS")
                        continue
                    matches = qualified_subscribers(reader, publication["topic"])
                    if not matches:
                        emit(owner, "storage-message-boundary", f"A source-qualified subscriber callback is "
                             f"unavailable for the publication candidate in `{name}`.", MESSAGE_OPEN + " " + STORAGE_OPEN,
                             write, registration(target), read, publication, selection, status="UNKNOWN")
                    for subscriber in matches:
                        consumer = records[subscriber["id"]]
                        prefix = f"If the registered reader in `{name}` selects this row and reaches publication, "
                        boundary = " ".join(filter(None, (STORAGE_OPEN, MESSAGE_OPEN, publication.get("uncertainty"),
                                                           selection["uncertainty"], ASYNC_OPEN,
                                                           "Other payload fields and wire mapping remain open.")))
                        evidence = (write, registration(target), read, publication, selection,
                                    registration(consumer), callback(consumer))
                        emit(owner, "storage-message-continuation", prefix + selection["text"] +
                             f" `{subscriber['page']}` declares the same topic `{publication['topic']}`.",
                             boundary, *evidence)
                        for effect in representative_writes(subscriber["id"], publication["topic"]):
                            if conflict := incompatible_write(effect, publication["topic"]):
                                emit(owner, "message-peer-write-boundary", prefix + conflict["text"], boundary,
                                     *evidence, conflict, status="INCOMPATIBLE_IN_RETAINED_PATHS")
                                continue
                            reached = emit(owner, "storage-message-effect", prefix
                                           + f"delivery to `{subscriber['page']}` can reach the cited write path. " + effect["text"],
                                           boundary, *evidence, effect)
                            if (reached and owner not in guarded_terminal_effects and effect.get("condition")
                                    and not next(local_readers(subscriber["id"], effect["table"]), None)):
                                guarded_terminal_effects[owner] = reached
                            add_focused(owner, effect, publication["topic"], prefix, boundary, *evidence)
                            readbacks(owner, subscriber["id"], effect, *evidence,
                                      prefix=prefix + f"and delivery reaches `{subscriber['page']}`, ",
                                      uncertainty=boundary)
                            further_outbox(owner, subscriber["id"], effect, publication["topic"],
                                           {owner, reader, subscriber["id"]}, evidence)

    # Attach a bounded prefix of the selected handler's storage continuation to
    # the caller's view. The source-qualified route remains a prerequisite.
    followups = ("further-subscriber-readback", "further-subscriber-effect",
                 "message-peer-write-detail", "message-peer-write-boundary", "message-storage-reader",
                 "storage-message-effect", "storage-message-continuation",
                 "storage-message-boundary", "storage-reader", "storage-peer-boundary")
    priority = {kind: index for index, kind in enumerate(followups)}
    for handler, requesters in requester_links.items():
        rows = sorted((row for row in result[handler] if row["kind"] in priority),
                      key=lambda row: (priority[row["kind"]], -len(row["sources"])))
        by_kind = {}
        for row in rows:
            by_kind.setdefault(row["kind"], row)
        activation = next((item for item in result[handler] if item["kind"] == "storage-poll-activation"), None)
        completion = (item for item in result[handler] if item["kind"] == "local-response-independent-poll")
        initial = first_outbox_hops[handler] if isolated else first_outbox_hops[handler][:1]
        facets = []
        for kind in OUTBOX_WRITE_FACETS:
            candidates = (row for row in result[handler] if row["kind"] == kind)
            facets.extend(candidates if isolated else islice(candidates, 1))
        rows = [*([activation] if activation else ()), *initial, *facets,
                *(completion if isolated else islice(completion, 1)), *islice(by_kind.values(), 3)]
        name = records[handler]["requirement"]["page"]
        for requester, call, registration_proof, boundary in requesters:
            for row in rows:
                kind = "http-" + row["kind"]
                emit(requester, kind, f"If the request reaches `{name}`, the handler has this retained continuation. "
                     + row["text"], boundary + " " + row["uncertainty"], call, registration_proof, row,
                     status=row["status"])
    if isolated:
        return result
    preferred = ("http-failure-after-peer-commit", "http-peer-commit-before-response", "http-peer-conditional-no-write",
                 "http-peer-sql-write", "http-peer-write-binding", "http-peer-guarded-return",
                 "http-peer-return-bypass", "http-peer-declared-response",
                 "http-peer-declaration-boundary", "http-peer-outcome", "http-peer-value-lineage",
                 "http-local-response-independent-poll", "local-response-independent-poll",
                 "http-storage-poll-activation", "http-further-subscriber-readback", "http-further-subscriber-outbox",
                 "further-subscriber-readback",
                 "message-storage-reader", "http-message-storage-reader", "storage-poll-activation", "further-subscriber-outbox",
                 "message-peer-write-detail", "message-peer-write-boundary", "message-peer-effect", "storage-message-effect",
                 "typed-message-peer", "typed-message-peer-boundary",
                 "further-outbox-limit-boundary", "further-outbox-boundary")
    for owner, rows in result.items():
        if len(rows) <= MAX_LINKS:
            continue
        groups = defaultdict(list)
        for row in rows:
            groups[row["kind"]].append(row)
        deepest = {"further-subscriber-readback", "http-further-subscriber-readback"}
        sampled = []
        for kind in preferred:
            candidates = groups[kind]
            if not candidates:
                continue
            if kind in deepest:
                sampled.append(max(candidates, key=lambda row: len(row["sources"])))
            elif kind == "storage-message-effect" and owner in guarded_terminal_effects:
                sampled.append(guarded_terminal_effects[owner])
            else:
                sampled.extend(candidates[:MAX_HTTP_WRITES if kind == "http-peer-sql-write" else 1])
        sampled = sampled[:MAX_LINKS - 1]
        for row in rows:
            if len(sampled) >= MAX_LINKS - 1:
                break
            if row not in sampled:
                sampled.append(row)
        result[owner] = [*sampled, {
            "kind": "continuation-coverage-boundary", "authority": "RETAINED_SOURCE_EDGE", "status": "UNKNOWN",
            "text": "Additional source continuation candidates remain beyond the prepared page limit.",
            "sources": sampled[0]["sources"],
        }]
    return result
