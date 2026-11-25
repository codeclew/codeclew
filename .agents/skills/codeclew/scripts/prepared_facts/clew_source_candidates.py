"""Project literal continuation candidates from already retained source ranges."""

from collections import defaultdict
from hashlib import sha256
import re

TOKEN = re.compile(
    r'//[^\n]*|/\*[\s\S]*?\*/|\$?@?"{3}[\s\S]*?"{3}'
    r'|\$@"(?:""|[^"])*"|@"(?:""|[^"])*"|\$?"(?:\\[\s\S]|[^"\\])*"'
    r"|\x27(?:\\[\s\S]|[^\x27\\])*\x27|`(?:\\[\s\S]|[^`\\])*`"
    r'|[A-Za-z_$][\w$]*|\?\.|[^\s]', re.ASCII,
)
IDENT = r'(?:[A-Za-z_][\w$]*|"[^"]+"|`[^`]+`|\[[^\]]+\])'
TABLE = IDENT + r'(?:\s*\.\s*' + IDENT + r')*'
SQL_TOKEN = re.compile(r"'(?:''|[^'])*'|--[^\n]*|/\*[\s\S]*?\*/|" + IDENT + r"|[().,]|[^\s]", re.ASCII)
TOPIC = re.compile(r'[A-Za-z0-9][A-Za-z0-9._-]{0,248}\Z', re.ASCII)
BODY_BYTES = 5 * 1024
BODY_LINES = 72
PACKET_BODY_BYTES = 20 * 1024
PACKET_BODY_RANGES = 8


def literal(token):
    text = token.group()
    if text.startswith(('"""', '@"')) or text[:1] in {'"', "'", '`'} and len(text) > 1:
        if text.startswith('"""'):
            value = text[3:-3]
        elif text.startswith('@"'):
            value = text[2:-1].replace('""', '"')
        else:
            value = text[1:-1]
        if '${' not in value:
            return value.replace(r'\n', '\n').replace(r'\"', '"').replace(r"\'", "'")
    return None


def table_name(name):
    return ".".join(part.group().strip('"`[]').casefold() for part in re.finditer(IDENT, name, re.ASCII))


def top_level_sql(sql):
    # Track parentheses and skip quoted SQL values. JSON paths, nested
    # SELECTs, and FROM inside values cannot stand in for the outer table.
    depth = 0
    for token in SQL_TOKEN.finditer(sql):
        word = token.group()
        if word == "(":
            depth += 1
        elif word == ")":
            depth -= 1
        elif depth == 0 and not word.startswith(("'", "--", "/*")):
            yield token


def sql_tables(value, normalize=table_name):
    sql = re.sub(r"\A(?:\s+|--[^\n]*(?:\n|$)|/\*[\s\S]*?\*/)*", "", value)
    prefix = re.match(r"(?is)(INSERT(?:\s+OR\s+\w+)?\s+INTO|UPDATE|DELETE\s+FROM)\s+(" + TABLE + ")", sql)
    if prefix:
        kind = prefix[1].split()[0].upper()
        name = normalize(prefix[2])
        if kind == "UPDATE":
            # UPDATE alias SET ... FROM schema.table AS alias.
            for token in top_level_sql(sql[prefix.end():]):
                if token.group().upper() != "FROM":
                    continue
                from_clause = sql[prefix.end() + token.end():]
                aliases = re.findall(r'(?i)(?:^|\bJOIN\s+|,)\s*(' + TABLE + r')\s+(?:AS\s+)?(' + IDENT + ')',
                                     from_clause)
                matched = {normalize(table) for table, alias in aliases if normalize(alias) == name}
                if len(matched) == 1:
                    name = next(iter(matched))
                break
        return [(kind, name)]
    if not re.match(r"SELECT\b", sql, re.I):
        return []
    result = []
    for token in top_level_sql(sql):
        if token.group().upper() in {"FROM", "JOIN"}:
            # A subquery/table-valued function needs its own interpretation.
            table = re.match(r"\s+(" + TABLE + r")(?![\w$.])\s*(?!\s*\()", sql[token.end():])
            if table:
                result.append(("SELECT", normalize(table[1])))
    return list(dict.fromkeys(result))


def close_group(tokens, start, max_tokens=160):
    # Argument and annotation scanning is deliberately local.
    opening = {"(": ")", "[": "]", "{": "}"}
    expect = []
    for end in range(start, min(len(tokens), start + max_tokens)):
        word = tokens[end].group()
        if word in opening:
            expect.append(opening[word])
        elif word in {")", "]", "}"}:
            if not expect or expect.pop() != word:
                return None
            if not expect:
                return end
    return None


def topic_at(tokens, index):
    if index >= len(tokens):
        return None
    value = literal(tokens[index])
    return value if value and TOPIC.fullmatch(value) else None


def member_topics(tokens, start, end, method):
    words = [token.group() for token in tokens]
    if words[start:start + 2] in (["topic", ":"], ["topic", "="]):
        start += 2
    if topic := topic_at(tokens, start):
        return [topic]
    if method in {"send", "sendAsync", "ProduceAsync", "Produce"}:
        if words[start] == "{":
            return [value for index in range(start + 1, end - 1) if words[index:index + 2] == ["topic", ":"]
                    and (value := topic_at(tokens, index + 2))]
        if words[start:start + 2] == ["new", "ProducerRecord"]:
            opening = next((index for index in range(start + 2, end) if words[index] == "("), None)
            topic = topic_at(tokens, opening + 1) if opening is not None else None
            return [topic] if topic else []
        return []
    for index in range(start, end):
        if words[index] in {"topic", "topics"} and words[index + 1] in {":", "="}:
            bound = index + 2
            if topic := topic_at(tokens, bound):
                return [topic]
            if words[bound] in {"[", "{", "("} and (close := close_group(tokens, bound)):
                return [topic for index in range(bound + 1, close) if (topic := topic_at(tokens, index))]
    if words[start:start + 4] in (["List", ".", "of", "("], ["Arrays", ".", "asList", "("],
                                   ["Collections", ".", "singletonList", "("]):
        if close := close_group(tokens, start + 3):
            return [topic for index in range(start + 4, close) if (topic := topic_at(tokens, index))]
    return []


def anchors(text):
    tokens = [token for token in TOKEN.finditer(text) if not token.group().startswith(("//", "/*"))]
    for i, token in enumerate(tokens):
        value = literal(token)
        if value:
            for verb, table in sql_tables(value):
                yield verb, table, token.start(), token.end()
        if i < 1 or i + 2 >= len(tokens):
            continue
        name = token.group()
        receiver = tokens[i - 1].group()
        annotation = i - 1
        if name == "KafkaListener":
            while annotation > 1 and tokens[annotation].group() == ".":
                annotation -= 2
        subscribed = name == "KafkaListener" and tokens[annotation].group() == "@"
        member = receiver in {".", "?."}
        method = (member and name in {"send", "sendAsync", "ProduceAsync", "Produce", "subscribe"}
                  or name in {"ProduceAsync", "Produce"})
        if not (subscribed or method) or tokens[i + 1].group() != "(":
            continue
        end = close_group(tokens, i + 1)
        if end is None:
            continue
        start = token.start() if not subscribed else tokens[annotation].start()
        if method and member and i > 1:
            start = tokens[i - 2].start()
        for topic in dict.fromkeys(member_topics(tokens, i + 2, end, name)):
            yield "SUBSCRIBE" if subscribed or name == "subscribe" else "PUBLISH", topic, start, tokens[end].end()


def snippet(source, start, end, digest):
    text = source["text"]
    first = text.rfind("\n", 0, start) + 1
    last = text.find("\n", end)
    if last < 0:
        last = len(text)
    if last - first > 1500:
        first, last = start, end
    selection = text[first:last]
    if len(selection.encode()) > 2500:
        return None
    start_line = source["startLine"] + text[:first].count("\n")
    result = {
        **source, "startLine": start_line, "endLine": start_line + selection.count("\n"),
        "text": selection, "textDigest": digest(selection), "parentSourceId": source.get("id"),
        "parentSourceDigest": source["textDigest"],
        "fragmentStartByte": len(text[:first].encode()), "fragmentEndByte": len(text[:last].encode()),
        "selection": "LITERAL_ANCHOR_IN_RETAINED_SOURCE",
        "_anchorInFragment": [len(text[first:start].encode()), len(text[first:end].encode())],
    }
    if (span := byte_range(source)) is not None:
        result["startByte"] = span[0] + result["fragmentStartByte"]
        result["endByte"] = span[0] + result["fragmentEndByte"]
    return result


def byte_range(source):
    width = len(source["text"].encode())
    for location in (source, source.get("occurrence") or {}):
        start, end = location.get("startByte"), location.get("endByte")
        if type(start) is int and type(end) is int and end - start == width:
            return start, end
    return None


def anchor_range(source):
    if "_anchorInFragment" not in source:
        return source
    start, end = source["_anchorInFragment"]
    raw = source["text"].encode()
    projection = raw[start:end].decode()
    first = source["startLine"] + raw[:start].count(b"\n")
    result = {**source, "text": projection, "startLine": first, "endLine": first + projection.count("\n")}
    if (span := byte_range(source)) is not None:
        result.update(startByte=span[0] + start, endByte=span[0] + end)
    return result


def contains(outer, inner):
    if any(outer[key] != inner[key] for key in ("file", "revision")):
        return False
    if not outer["startLine"] <= inner["startLine"] <= inner["endLine"] <= outer["endLine"]:
        return False
    left, right = byte_range(outer), byte_range(inner)
    if left is not None and right is not None:
        return (left[0] <= right[0] <= right[1] <= left[1]
                and outer["text"].encode()[right[0] - left[0]:right[1] - left[0]] == inner["text"].encode())
    lines = outer["text"].split("\n")
    window = "\n".join(lines[inner["startLine"] - outer["startLine"]:inner["endLine"] - outer["startLine"] + 1])
    return window.count(inner["text"].rstrip("\n")) == 1


def native_csharp_body(source, fact):
    if (not source["file"].endswith(".cs") or fact.get("_bodyStatus") is not None
            or fact.get("_bodyAuthority") != "RETAINED_COMPILER_DECLARATION_NOT_RUNTIME_DISPATCH"
            or source.get("id") not in fact.get("_bodySourceIds", [])):
        return False
    tokens = [token for token in TOKEN.finditer(source["text"]) if not token.group().startswith(("//", "/*"))]
    words = [token.group() for token in tokens]
    # Roslyn's retained declaration is complete. A block or expression
    # implementation distinguishes it from a signature-only declaration.
    for i, word in enumerate(words):
        if word in {"{", ";", "="}:
            break
        if word == "(" and (end := close_group(tokens, i)) is not None and end + 1 < len(words):
            if words[end + 1] == "{" and words[-1] == "}" or words[end + 1:end + 3] == ["=", ">"] and words[-1] == ";":
                return True
    return False


def body_index(process, exact_source):
    bodies = {}
    for fact in process["facts"]:
        for ref in fact["sources"]:
            source = process["sources"].get(ref, {})
            if fact.get("declaration") and exact_source(source) and (
                    fact.get("kind") == "BODY" and (
                        fact.get("_bodyStatus") == "RETAINED_BODY" or native_csharp_body(source, fact))
                    or fact.get("authority") == "COMPILER_EXACT_CALLBACK_TARGET"
                    or fact.get("event") == "WORKER_HELPER_SOURCE"):
                bodies[ref] = fact["declaration"]
    bodies.update((ref, ref) for ref in process.get("_retainedBodyRefs", []))
    return {ref: declaration for ref, declaration in bodies.items()
            if exact_source(process["sources"].get(ref, {})) and not process["sources"][ref].get("peerProcess")}


def address(process, ref):
    source = process["sources"][ref]
    return {"service": source.get("peerService") or process["service"], **{key: source[key] for key in (
        "file", "revision", "startLine", "endLine", "textDigest",
    )}}


def unique_body(process, refs):
    unique = {tuple(address(process, ref).values()) + (byte_range(process["sources"][ref]),): ref for ref in refs}
    return next(iter(unique.values())) if len(unique) == 1 else None


def immediate_body(process, bodies, anchor):
    sources = process["sources"]
    occurrence = anchor_range(sources[anchor])
    enclosing = [ref for ref in bodies if contains(sources[ref], occurrence)]
    smallest = [ref for ref in enclosing if not any(
        len(sources[other]["text"].encode()) < len(sources[ref]["text"].encode())
        and contains(sources[ref], sources[other]) for other in enclosing)]
    return unique_body(process, smallest), "ENCLOSING_BODY_AMBIGUOUS" if smallest else "ENCLOSING_BODY_NOT_RETAINED"


CONTEXT_BOUNDARIES = {
    "ENCLOSING_BODY_AMBIGUOUS": "The retained ranges do not identify one directly enclosing method.",
    "ENCLOSING_BODY_NOT_RETAINED": "The anchor is retained, and its complete enclosing method is unavailable.",
    "CALLBACK_BODY_NOT_RETAINED": "The subscription is retained, and a complete selected callback body is unavailable.",
    "CALLBACK_BODY_AMBIGUOUS": "The retained ranges identify more than one callback body.",
    "HELPER_BINDING_NOT_UNIQUE": "The retained call does not establish one local helper for this effect.",
    "HELPER_BODY_NOT_RETAINED": "The helper is selected at this call site, and its complete body is unavailable.",
    "BODY_RANGE_LIMIT": "The complete retained body exceeds 5 KiB or 72 lines.",
    "PACKET_BODY_BUDGET": "The packet's eight-body or 20 KiB source context budget is exhausted.",
}


class BodyBudget:
    def __init__(self):
        self.ranges = set()
        self.bytes = 0

    def context(self, owner, process, use_source, role, confidence, ref, missing, sites=()):
        record = {"role": role, "confidence": confidence if ref else "UNRESOLVED",
                  "anchorReferences": [use_source(owner, process, site) for site in sites]}
        code = missing
        if ref:
            record["address"] = address(process, ref)
            key = tuple(record["address"].values())
            source = process["sources"][ref]
            size = len(source["text"].encode())
            if size > BODY_BYTES or source["endLine"] - source["startLine"] + 1 > BODY_LINES:
                code = "BODY_RANGE_LIMIT"
            elif key not in self.ranges and (len(self.ranges) >= PACKET_BODY_RANGES or self.bytes + size > PACKET_BODY_BYTES):
                code = "PACKET_BODY_BUDGET"
            else:
                if key not in self.ranges:
                    self.ranges.add(key)
                    self.bytes += size
                return {**record, "status": "RETAINED", "sourceReference": use_source(owner, process, ref)}
        return {**record, "status": "BOUNDARY", "boundary": {"code": code, "message": CONTEXT_BOUNDARIES[code]}}


def effect_helper(process, bodies, callback, effect):
    sources = process["sources"]
    occurrence = anchor_range(sources[effect])
    root = bodies.get(callback)
    related = {}
    for edge in process.get("_bodyBindings", []):
        if not root or edge["caller"] != root or edge["caller"] == edge["target"]:
            continue
        sites = [ref for ref in edge["siteReferences"] if ref in sources and contains(sources[callback], sources[ref])]
        if not sites:
            continue
        targets = [ref for ref, declaration in bodies.items() if declaration == edge["target"]]
        if (any(contains(sources[site], occurrence) or contains(occurrence, sources[site]) for site in sites)
                or any(contains(sources[target], occurrence) for target in targets)):
            entry = related.setdefault(edge["target"], {"refs": [], "sites": [], "confidence": edge["confidence"]})
            entry["refs"].extend(targets)
            entry["sites"].extend(sites)
    if len(related) == 1:
        selected = next(iter(related.values()))
        target = unique_body(process, selected["refs"])
        missing = "HELPER_BINDING_NOT_UNIQUE" if selected["refs"] else "HELPER_BODY_NOT_RETAINED"
        return target, selected["confidence"], missing, list(dict.fromkeys(selected["sites"]))[:4]
    if related or any(fact.get("event") == "CALL" and any(
            site in sources and (contains(sources[site], occurrence) or contains(occurrence, sources[site]))
            for site in fact["sources"]) for fact in process["facts"]):
        return None, "UNRESOLVED", "HELPER_BINDING_NOT_UNIQUE", []
    return None


def selected_refs(process):
    groups = {"BODY", "BODY_SOURCE", "SOURCE_LOCATION", "CALLBACK_SOURCE_LOCATION", "BUDGET_SOURCE_LOCATION"}
    direct = {ref for fact in process["facts"]
              if fact.get("kind") in groups or fact.get("event") in {"CALL", "WRITE", "SUBSCRIPTION"}
              for ref in fact["sources"]}
    trigger = process["trigger"]
    direct.update(ref for ref in (trigger.get("registration"), trigger.get("callback")) if ref)
    direct.update(process.get("entrySources", []))
    direct.update(process.get("_retainedBodyRefs", []))
    return direct


def collect(process, exact_source, digest):
    result = defaultdict(list)
    seen = set()
    sources = process["sources"]
    refs = [ref for ref in selected_refs(process) if exact_source(sources.get(ref, {}))
            and not sources[ref].get("peerProcess")]
    refs.sort(key=lambda ref: (sources[ref]["file"], -len(sources[ref]["text"]), ref))
    for ref in refs[:64]:
        source = sources[ref]
        if source["file"].rsplit(".", 1)[-1] not in {"cs", "java", "kt", "kts", "ts", "js", "tsx", "jsx", "mjs", "mts"}:
            continue
        text = source["text"]
        if len(text.encode()) > 160_000:
            continue
        for kind, subject, start, end in anchors(text):
            fragment = snippet(source, start, end, digest)
            if not fragment:
                continue
            anchor_line = source["startLine"] + text[:start].count("\n")
            identity = (kind, subject, source["file"], anchor_line, digest(text[start:end]))
            if identity in seen:
                continue
            seen.add(identity)
            key = "flow-literal-" + sha256(repr(identity).encode()).hexdigest()[:16]
            sources[key] = fragment
            result[kind].append((subject, key))
            if len(seen) >= 40:
                return result
    return result


def add_static_candidates(processes, exact_source, digest, use_source, body_refs):
    records = {process["id"]: collect(process, exact_source, digest) for process in processes}
    bodies = {process["id"]: body_index(process, exact_source) for process in processes}
    budget = {process["id"]: BodyBudget() for process in processes}
    for process in processes:
        process["_candidateBodyBudget"] = budget[process["id"]]
    topics = defaultdict(list)
    workers = defaultdict(list)

    def registration(process):
        trigger = process["trigger"]
        if trigger.get("registrationReferences"):
            return trigger["registrationReferences"]
        if trigger.get("registration"):
            return [trigger["registration"]]
        if process.get("entryKind") in {"SCHEDULED_JOB", "HOSTED_BACKGROUND_WORKER"}:
            return process.get("entrySources", [])
        return []

    def stage(owner, process, kind, refs, confidence, *contexts):
        anchors = [use_source(owner, process, ref) for ref in dict.fromkeys(refs) if ref in process["sources"]]
        context = [budget[owner["id"]].context(owner, process, use_source, *spec) for spec in contexts]
        attached = [ref for item in context for ref in item["anchorReferences"]]
        attached += [item["sourceReference"] for item in context if item["status"] == "RETAINED"]
        return {"kind": kind, "process": process["id"], "service": process["service"],
                "confidence": confidence, "anchorReferences": anchors, "bodyContext": context,
                "sourceReferences": list(dict.fromkeys(anchors + attached))}

    def publication_stage(owner, process, kind, ref, confidence):
        body, missing = immediate_body(process, bodies[process["id"]], ref)
        return stage(owner, process, kind, [ref], confidence,
                     ("ENCLOSING_PUBLISHER_METHOD", "EXACT_ENCLOSING_SOURCE", body, missing))

    def subscriptions(owner, topic):
        receivers = []
        for subscriber, subscription in topics[topic][:8]:
            receivers.append(stage(owner, subscriber, "SUBSCRIPTION_CANDIDATE", [subscription], "MATCHING_TOPIC_LITERAL"))
            trigger = subscriber["trigger"]
            jvm = subscriber.get("entryKind") == "KAFKA_LISTENER"
            javascript = trigger.get("kind") == "EACH_MESSAGE"
            if not jvm and not javascript:
                continue
            candidates = body_refs(subscriber) if jvm else [trigger.get("callback")]
            retained = bodies[subscriber["id"]]
            candidates = [ref for ref in candidates if ref in retained]
            callback = unique_body(subscriber, candidates)
            missing = "CALLBACK_BODY_AMBIGUOUS" if candidates else "CALLBACK_BODY_NOT_RETAINED"
            callback_anchor = [trigger.get("registration") or subscription]
            receivers.append(stage(owner, subscriber, "SUBSCRIBER_CALLBACK_CANDIDATE", callback_anchor,
                                   "RETAINED_CALLBACK_SELECTION",
                                   ("CALLBACK_METHOD", "EXACT_SELECTED_CALLBACK_SOURCE", callback, missing)))
            effects = [ref for kind in ("INSERT", "UPDATE", "DELETE") for _, ref in records[subscriber["id"]][kind]]
            effects += [ref for fact in subscriber["facts"] if fact.get("event") == "WRITE"
                        or fact.get("kind") == "DATA_EFFECT" for ref in fact["sources"]]
            if effects:
                effect_refs = [ref for ref in dict.fromkeys(effects) if ref in subscriber["sources"]][:4]
                possible = (effect_helper(subscriber, retained, callback, effect) for effect in effect_refs)
                contexts = list(dict.fromkeys(("EFFECT_HELPER_METHOD", confidence, ref, reason, tuple(sites))
                                              for ref, confidence, reason, sites in (result for result in possible if result)))
                receivers.append(stage(owner, subscriber, "SUBSCRIBER_EFFECT_CANDIDATE",
                                       effect_refs, "RETAINED_EFFECT_SITE", *contexts))
        return receivers

    def thread(table, topic, stages, receivers, cleanup, boundary, authority):
        return {"authority": authority, "table": table, "topic": topic, "status": "BOUNDARY",
                "runtime": "UNKNOWN", "boundary": boundary, "stages": stages,
                "receiverCandidates": receivers, "publisherAfterSend": [], "cleanupCandidates": cleanup}

    for process in processes:
        observed = records[process["id"]]
        for topic, ref in observed["SUBSCRIBE"]:
            topics[topic].append((process, ref))
        if registration(process) and observed["PUBLISH"]:
            for table, ref in observed["SELECT"]:
                workers[table].append((process, ref))
    for owner in processes:
        # Keep a more precise adapter's existing source-bound result.
        if any(item["authority"] == "RETAINED_STATIC_BINDINGS" for item in owner.get("outboxThreads", [])):
            continue
        observed = records[owner["id"]]
        threads = []
        for table, writer in observed["INSERT"] + observed["UPDATE"]:
            matches = sorted(workers[table], key=lambda row: (row[0]["service"] != owner["service"], row[0]["id"], row[1]))
            for worker, reader in matches[:4]:
                if worker["id"] == owner["id"]:
                    continue
                found = records[worker["id"]]
                for topic, send in found["PUBLISH"][:4]:
                    stages = [
                        stage(owner, owner, "SQL_WRITE_CANDIDATE", [writer], "RETAINED_LITERAL"),
                        stage(owner, worker, "PUBLISHER_REGISTRATION", registration(worker), "RETAINED_REGISTRATION"),
                        publication_stage(owner, worker, "SQL_READ_CANDIDATE", reader, "MATCHING_TABLE_NAME"),
                        publication_stage(owner, worker, "CHANNEL_PUBLICATION_CANDIDATE", send, "RETAINED_LITERAL"),
                    ]
                    receivers = subscriptions(owner, topic)
                    cleanup = [stage(owner, worker, "SQL_CHANGE_CANDIDATE", [ref], "MATCHING_TABLE_NAME")
                               for kind in ("UPDATE", "DELETE") for candidate, ref in found[kind] if candidate == table]
                    boundary = ("Literal table and topic matches provide source candidates. "
                                "Database identity, row and payload lineage, activation, delivery and effect order remain open.")
                    if not receivers:
                        boundary += " A matching retained subscription is unavailable."
                    threads.append(thread(table, topic, stages, receivers, cleanup[:4], boundary,
                                          "RETAINED_LITERAL_CONTINUATION_CANDIDATES"))
                    if len(threads) >= 6:
                        break
                if len(threads) >= 6:
                    break
            if len(threads) >= 6:
                break
        if not threads and not owner.get("outboxThreads") and registration(owner):
            for topic, send in observed["PUBLISH"][:4]:
                stages = [stage(owner, owner, "PUBLISHER_REGISTRATION", registration(owner), "RETAINED_REGISTRATION")]
                stages += [publication_stage(owner, owner, "SQL_READ_CANDIDATE", ref, "RETAINED_LITERAL")
                           for _, ref in observed["SELECT"][:3]]
                stages.append(publication_stage(owner, owner, "CHANNEL_PUBLICATION_CANDIDATE", send, "RETAINED_LITERAL"))
                threads.append(thread(None, topic, stages, subscriptions(owner, topic), [],
                                      "Storage lineage, broker binding, activation and delivery remain open.",
                                      "RETAINED_LITERAL_PUBLICATION_PREFIX"))
        if not threads and not owner.get("outboxThreads") and owner.get("entryKind") != "KAFKA_LISTENER":
            for table, writer in (observed["INSERT"] + observed["UPDATE"])[:3]:
                stages = [stage(owner, owner, "SQL_WRITE_CANDIDATE", [writer], "RETAINED_LITERAL")]
                threads.append(thread(table, None, stages, [], [],
                                      "A retained SQL write is visible. Its continuation and runtime effects remain open.",
                                      "RETAINED_SQL_PREFIX_CANDIDATE"))
        if threads:
            owner.setdefault("outboxThreads", []).extend(threads)
            owner["boundaries"].append("Outbox continuation candidates use literal source anchors.")
    for owner in processes:
        used = {ref for item in owner.get("outboxThreads", [])
                for key in ("stages", "receiverCandidates", "publisherAfterSend", "cleanupCandidates")
                for part in item.get(key, []) for ref in part["sourceReferences"]}
        for ref in list(owner["sources"]):
            if "flow-literal-" in ref and ref not in used:
                del owner["sources"][ref]
