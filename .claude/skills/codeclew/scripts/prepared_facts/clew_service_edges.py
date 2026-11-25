"""Join bounded Work propositions by explicit retained protocol identities."""

from collections import defaultdict, deque
from itertools import islice
from urllib.parse import urlsplit
import re


ENTRIES = object()
MAX_MATCHES = 4
MAX_SITES = 8
MAX_LINKS = 16
MAX_COLLECTED_LINKS = 48
MAX_WRITE_SITES = 16
MAX_NESTED_VISITS = 24
MAX_CHAIN_SOURCES = 36
POLLERS = {"HOSTED_BACKGROUND_WORKER", "SCHEDULED_JOB"}
HTTP_METHODS = {"GET", "POST", "PUT", "DELETE", "PATCH", "HEAD", "OPTIONS", "TRACE"}
TOPIC = re.compile(r"[A-Za-z0-9._-]{1,249}")
TABLE = re.compile(r'[A-Za-z_][\w$]*(?:\.[A-Za-z_][\w$]*){0,2}', re.ASCII)
PARAMETER = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}|:[A-Za-z_][A-Za-z0-9_]*")
KAFKA_ADAPTER = "SPRING_KAFKA_LITERAL_TOPIC/1.0"
HTTP_ADAPTERS = {"SPRING_REST_TEMPLATE_LITERAL_SUFFIX/1.0", "SPRING_REST_CLIENT_URI/2.0"}
PEER_RESPONSES = {"http-outcome", "http-commit-before-response", "status_body"}
CALLER_PATHS = {"java-guarded-outcome", "java-ordered-effects", "http-outcome", "error-path",
                "effect-order", "handled-exception-continuation", "return_rejection", "await_bypass",
                "exception_route", "request_after_commit_failure", "response_body_read", "status_body",
                "result_continuation", "return", "throw", "EARLY_EXIT", "HARD_EXIT",
                "GUARDED_EFFECTS", "ORDERED_EFFECTS"}
HTTP_OPEN = ("Deployment destination, activation and wire mapping remain unknown. An application exception "
             "does not establish the status or exception observed by the client.")
MESSAGE_OPEN = ("Broker or adapter binding, handler activation, delivery, acknowledgement, ordering and "
                "payload compatibility remain unknown.")
STORAGE_OPEN = ("Database and schema binding, key or row identity, commit, visibility and timing remain unknown.")


def topic(value):
    return value if isinstance(value, str) and TOPIC.fullmatch(value) and value not in {".", ".."} else None


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


def record(requirement, process, context, facts, exact):
    """Retain only small indexes needed after all ordinary projections complete."""
    sources = {ref: {**exact[row["id"]], "handle": row["id"]}
               for row in context["sourceRanges"] for ref in row["references"] if row["id"] in exact}
    trigger = process.get("trigger") or {}
    subscription_refs = [*trigger.get("subscriptions", ()), *trigger.get("subscriptionSources", ())]
    trigger_refs = tuple(filter(None, (trigger.get("registration"), trigger.get("callback"), *subscription_refs)))
    for ref in trigger_refs:
        if ref in sources or ref not in process["sources"]:
            continue
        witness = process["sources"][ref]
        if handle := context["_bindWitness"](witness):
            sources[ref] = {**witness, "handle": handle}
    registration = cite_refs(sources, [trigger.get("registration"), *subscription_refs])
    if not registration:
        registration = cite_refs(sources, process["entrySources"],
                                 source_ids=set(requirement.get("registrationSourceIds", ())) or None)
    registered = registration[:MAX_SITES]
    topic_proven = trigger.get("authority") != "KAFKAJS_RETAINED_SOURCE" or (
        bool(trigger.get("subscriptions")) and len(registration) <= MAX_SITES
        and all(ref in sources for ref in subscription_refs))
    body = cite_refs(sources, [ref for fact in process["facts"]
                              if fact["kind"] == "BODY" and fact.get("declaration") == process["root"]
                              for ref in fact["sources"]])[:1]
    callbacks = cite_refs(sources, [trigger.get("callback")])
    calls, publishes, reads, writes = {}, {}, {}, {}

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

    write_focus = defaultdict(lambda: defaultdict(list))
    for fact in facts:
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
        elif link["kind"] in {"SELECT", "INSERT", "UPDATE", "DELETE"}:
            table = link.get("table")
            # Quoted identifiers preserve spelling. Conservatively leave the
            # dialect-dependent equivalence of alternate spellings open.
            if isinstance(table, str) and TABLE.fullmatch(table):
                index = reads if link["kind"] == "SELECT" else writes
                keep(index, (face["occurrence"] or site, table, link["kind"], face.get("condition")),
                     {**face, "table": table, "verb": link["kind"]}, MAX_WRITE_SITES)

    registration_witnesses = []
    for ref in trigger_refs:
        if ref in sources and sources[ref]["handle"] in registered + callbacks:
            source = sources[ref]
            registration_witnesses.append({"cite": source["handle"], **{key: source[key] for key in (
                "file", "revision", "startLine", "endLine", "startByte", "endByte", "textDigest") if key in source}})
    return {"requirement": requirement, "registration": registered, "body": callbacks or body,
            "registrationWitnesses": registration_witnesses, "topicProven": topic_proven,
            "calls": list(calls.values()), "publishes": list(publishes.values()),
            "reads": list(reads.values()), "writes": [{**write, "focus": write_focus.get(write["occurrence"], {})}
                                                     for write in writes.values()],
            "responses": [fact for fact in facts if fact["kind"] in PEER_RESPONSES][:MAX_MATCHES],
            "commits": [fact for fact in facts if fact["kind"] == "http-commit-before-response"][:2],
            "exits": list(source_exits(process, sources))[:MAX_MATCHES],
            "paths": [fact for fact in facts if fact["kind"] in CALLER_PATHS]}


def project(requirements, records):
    """Index each protocol once and bound per-page evidence fan-out."""
    routes = Routes(requirements)
    subscriptions, readers, reader_services = defaultdict(list), defaultdict(dict), defaultdict(dict)
    for row in requirements:
        for name in topics(row):
            subscriptions[name].append(row)
    for identity, row in records.items():
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

    def emit(owner, kind, text, uncertainty, *faces, status="SOURCE_QUALIFIED_CANDIDATE"):
        if len(result[owner]) >= MAX_COLLECTED_LINKS:
            return
        if not (sources := list(dict.fromkeys(ref for face in faces for ref in face["sources"]))):
            return
        conditions = list(dict.fromkeys(face["condition"] for face in faces if face.get("condition")))
        key = (kind, text, tuple(sources), tuple(conditions), status)
        if key in seen[owner]:
            return
        seen[owner].add(key)
        witnesses = [w for face in faces for w in face.get("witnesses", ())]
        result[owner].append({"kind": kind, "authority": "RETAINED_SOURCE_EDGE", "status": status,
                              "text": text, "uncertainty": uncertainty, "sources": sources,
                              **({"condition": " ".join(conditions)} if conditions else {}),
                              **({"witnesses": witnesses} if witnesses else {})})

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

    def exact_write(write, topic):
        rows = focused_rows(write, topic)
        for kind in ("write_request", "sql-write", "write_guard", "write_lineage"):
            if row := next((item for item in rows if item["kind"] == kind), None):
                return {**write, "sources": list(dict.fromkeys([*write["sources"], *row["sources"]])),
                        "witnesses": [*write.get("witnesses", ()), *row.get("witnesses", ())],
                        **({"condition": row["condition"]} if row.get("condition") else {})}
        return write

    def add_focused(owner, write, topic, message, qualifier, *proof):
        details = focused_rows(write, topic)
        for kind in ("write_lineage", "write_guard", "write_commit", "transactional-write-completion"):
            if detail := next((row for row in details if row["kind"] == kind), None):
                emit(owner, "message-peer-write-detail", message + " " + detail["text"],
                     qualifier + " " + detail.get("uncertainty", ""), *proof, detail)

    def representative_writes(writer, topic=None):
        groups = defaultdict(list)
        for write in records[writer]["writes"]:
            if incompatible_write(write, topic):
                groups[3].append(write)
                continue
            table = write["table"]
            rank = (0 if next(local_readers(writer, table, ("http",)), None) else
                    1 if write["verb"] != "DELETE" and any(records[reader]["publishes"]
                                                          for reader, _ in local_readers(writer, table, ("poll",))) else 2)
            groups[rank].append(write)
        groups[2].sort(key=lambda effect: not effect.get("condition"))
        selected = [group[0] for _, group in sorted(groups.items()) if group]
        if len(selected) < 4:
            selected.extend(islice((write for _, group in sorted(groups.items()) for write in group[1:]),
                                   4 - len(selected)))
        return selected

    def further_outbox(owner, writer, write, topic, visited, evidence):
        queue = deque([(writer, write, topic, visited, evidence)])
        while queue:
            writer, raw_write, topic, visited, evidence = queue.popleft()
            if raw_write["verb"] == "DELETE":
                continue
            write = exact_write(raw_write, topic)
            table = write["table"]
            publishers = list(islice((row for row in local_readers(writer, table, ("poll",))
                                      if records[row[0]]["publishes"]), MAX_MATCHES))
            for publisher, read in publishers:
                poller = records[publisher]
                title = poller["requirement"]["page"]
                frontier = (write, registration(poller), read)
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
                for publication in poller["publishes"][:2]:
                    subscribers = qualified_subscribers(publisher, publication["topic"])
                    if not subscribers:
                        emit(owner, "further-outbox-boundary", "The next publication has no bounded set of proven active callback candidates.",
                             MESSAGE_OPEN, *evidence, *frontier, publication, status="UNKNOWN")
                    for subscriber in subscribers:
                        target = subscriber["id"]
                        if nested_visits[owner] >= MAX_NESTED_VISITS:
                            emit(owner, "further-outbox-limit-boundary",
                                 f"More continuation remains beyond the prepared visit limit from the cited poll root in `{title}`.",
                                 MESSAGE_OPEN + " " + STORAGE_OPEN, *frontier, publication, status="PREPARATION_LIMIT")
                            return
                        nested_visits[owner] += 1
                        continuation = (*evidence, *frontier, publication, registration(records[target]))
                        if target in visited or target == writer:
                            emit(owner, "further-outbox-boundary", "The next publication can revisit a previously considered callback.",
                                 MESSAGE_OPEN, *continuation, status="UNKNOWN")
                            continue
                        next_topic = publication["topic"]
                        prior_topic = f"After delivery on `{topic}`, " if topic else ""
                        prefix = (prior_topic + f"if the cited callback write becomes visible and the registered poller in `{title}` "
                                  f"selects the same key, it can reach the publication on `{next_topic}`. ")
                        qualifier = STORAGE_OPEN + " " + MESSAGE_OPEN + " Row-to-payload lineage remains unknown."
                        proof = (*continuation, callback(records[target]))
                        if len({ref for face in proof for ref in face["sources"]}) > MAX_CHAIN_SOURCES:
                            emit(owner, "further-outbox-limit-boundary",
                                 f"More continuation remains beyond the source limit from the cited poll root in `{title}`.",
                                 qualifier, *frontier, publication, status="PREPARATION_LIMIT")
                            return
                        emit(owner, "further-subscriber-outbox", prefix
                             + f"`{subscriber['page']}` has the cited registration for that topic.", qualifier, *proof)
                        for effect in representative_writes(target, next_topic):
                            if conflict := incompatible_write(effect, next_topic):
                                emit(owner, "message-peer-write-boundary", prefix + conflict["text"],
                                     qualifier, *proof, conflict, status="INCOMPATIBLE_IN_RETAINED_PATHS")
                                continue
                            selected = exact_write(effect, next_topic)
                            emit(owner, "further-subscriber-effect",
                                 prefix + f"If delivery reaches the cited callback in `{subscriber['page']}`, it can reach this write. "
                                 + selected["text"], qualifier, *proof, selected)
                            add_focused(owner, effect, next_topic, prefix, qualifier, *proof)
                            readbacks(owner, target, selected, *proof, prefix=prefix, uncertainty=qualifier, nested=True)
                            queue.append((target, effect, next_topic, visited | {writer, publisher, target}, proof))

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
            if len(requester_links[peer["id"]]) < MAX_MATCHES:
                requester_links[peer["id"]].append((owner, call, cited[0], boundary))
            emit(owner, "http-peer", f"The request at {label} has a route candidate in {name}.", boundary, call, cited[0])
            emit(peer["id"], "http-peer", f"`{service}` has a retained request matching this source route at {label}.",
                 boundary, call, cited[0])
            for outcome in target["responses"] or target["exits"]:
                emit(owner, "http-peer-outcome", predicate + " " + outcome["text"],
                     boundary + " " + outcome.get("uncertainty", ""), call, cited[0], outcome)
            if not target["responses"] and not target["exits"]:
                emit(owner, "http-peer-outcome-boundary", f"An observable handler return or exception for {name} remains open.",
                     boundary, call, cited[0], status="UNKNOWN")
            local_paths = [row for row in local["paths"] if continuation(row, call)]
            decoding = [row for row in local_paths if row.get("_edgePhase") == "RESPONSE_DECODING"]
            failure = next((row for row in decoding if row["kind"] == "exception_route"), None)
            if failure and target["commits"]:
                terminal = next((row for kind in ("status_body", "return", "throw")
                                 for row in decoding if row["kind"] == kind), failure)
                if decode_read := next((row for row in decoding if row["kind"] == "response_body_read"), None):
                    for commit in target["commits"]:
                        emit(owner, "http-failure-after-peer-commit",
                             f"If the matched handler in {name} takes its cited commit-before-response path, "
                             f"the caller can still fail while decoding the resulting response. {terminal['text']}",
                             boundary + " The upstream write may already be committed. The caller failure does not "
                             "establish upstream rollback, durability or the wire status.",
                             call, cited[0], commit, decode_read, failure, terminal)
            for outcome in local_paths[:MAX_MATCHES]:
                emit(owner, "http-caller-continuation", outcome["text"],
                     boundary + " Persistence and rollback of any earlier effects remain unknown. "
                     + outcome.get("uncertainty", ""), call, cited[0], outcome)

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
                    selected = exact_write(effect, name)
                    emit(owner, "message-peer-effect", f"If the subscribed callback in {title} reaches this write, "
                         + selected["text"], boundary + " " + STORAGE_OPEN, *proof, selected)
                    add_focused(owner, effect, name, f"The selected callback in {title} has this retained write path.",
                                boundary, *proof)
                    readbacks(owner, peer, selected, *proof, prefix="After the subscribed callback reaches the write, ",
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
                if not target["publishes"]:
                    emit(owner, "storage-message-boundary", f"The registered reader in `{name}` has no qualified "
                         "publication for this retained storage path.", MESSAGE_OPEN + " " + STORAGE_OPEN,
                         write, registration(target), read, status="UNKNOWN")
                for publication in target["publishes"][:2]:
                    subscribers = subscriptions.get(publication["topic"], ())
                    if len(subscribers) > MAX_MATCHES:
                        emit(owner, "storage-message-boundary", f"Multiple retained subscriptions follow the "
                             f"publication candidate in `{name}`.", MESSAGE_OPEN, write, registration(target), read,
                             publication, status="AMBIGUOUS")
                        continue
                    matches = qualified_subscribers(reader, publication["topic"])
                    if not matches:
                        emit(owner, "storage-message-boundary", f"A source-qualified subscriber callback is "
                             f"unavailable for the publication candidate in `{name}`.", MESSAGE_OPEN + " " + STORAGE_OPEN,
                             write, registration(target), read, publication, status="UNKNOWN")
                    for subscriber in matches:
                        consumer = records[subscriber["id"]]
                        prefix = f"If the registered reader in `{name}` selects this row and reaches publication, "
                        boundary = " ".join(filter(None, (STORAGE_OPEN, MESSAGE_OPEN, publication.get("uncertainty"),
                                                           "Row-to-payload lineage remains unknown.")))
                        evidence = (write, registration(target), read, publication,
                                    registration(consumer), callback(consumer))
                        emit(owner, "storage-message-continuation", prefix
                             + f"`{subscriber['page']}` declares the same topic `{publication['topic']}`.",
                             boundary, *evidence)
                        for effect in representative_writes(subscriber["id"], publication["topic"]):
                            if conflict := incompatible_write(effect, publication["topic"]):
                                emit(owner, "message-peer-write-boundary", prefix + conflict["text"], boundary,
                                     *evidence, conflict, status="INCOMPATIBLE_IN_RETAINED_PATHS")
                                continue
                            selected = exact_write(effect, publication["topic"])
                            emit(owner, "storage-message-effect", prefix
                                 + f"delivery to `{subscriber['page']}` can reach the cited write path. " + selected["text"],
                                 boundary, *evidence, selected)
                            add_focused(owner, effect, publication["topic"], prefix, boundary, *evidence)
                            readbacks(owner, subscriber["id"], selected, *evidence,
                                      prefix=prefix + f"and delivery reaches `{subscriber['page']}`, ",
                                      uncertainty=boundary)
                            further_outbox(owner, subscriber["id"], effect, publication["topic"],
                                           {owner, reader, subscriber["id"]}, evidence)

    # Attach a bounded prefix of the selected handler's storage continuation to
    # the caller's view. The source-qualified route remains a prerequisite.
    followups = ("further-subscriber-readback", "further-subscriber-effect", "further-subscriber-outbox",
                 "message-peer-write-detail", "message-peer-write-boundary", "message-storage-reader",
                 "storage-message-effect", "storage-message-continuation",
                 "storage-message-boundary", "storage-reader", "storage-peer-boundary")
    priority = {kind: index for index, kind in enumerate(followups)}
    for handler, requesters in requester_links.items():
        rows = sorted((row for row in result[handler] if row["kind"] in priority),
                      key=lambda row: (priority[row["kind"]], -len(row["sources"])))[:3]
        name = records[handler]["requirement"]["page"]
        for requester, call, registration_proof, boundary in requesters:
            for row in rows:
                kind = "http-" + row["kind"]
                emit(requester, kind, f"If the request reaches `{name}`, the handler has this retained continuation. "
                     + row["text"], boundary + " " + row["uncertainty"], call, registration_proof, row,
                     status=row["status"])
    preferred = ("http-failure-after-peer-commit", "http-further-subscriber-readback", "further-subscriber-readback",
                 "message-storage-reader", "http-message-storage-reader", "further-subscriber-outbox",
                 "message-peer-write-detail", "message-peer-write-boundary", "message-peer-effect", "storage-message-effect",
                 "further-outbox-limit-boundary", "further-outbox-boundary")
    for owner, rows in result.items():
        if len(rows) <= MAX_LINKS:
            continue
        groups = defaultdict(list)
        for row in rows:
            groups[row["kind"]].append(row)
        deepest = {"further-subscriber-readback", "http-further-subscriber-readback"}
        sampled = [max(groups[kind], key=lambda row: len(row["sources"])) if kind in deepest else groups[kind][0]
                   for kind in preferred if groups[kind]]
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
