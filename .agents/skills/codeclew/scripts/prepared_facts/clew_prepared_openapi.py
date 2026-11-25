"""Project declared OpenAPI responses from retained files, without resolving runtime APIs."""

from collections import defaultdict
from bisect import bisect_right
from functools import lru_cache
import hashlib
import json
import re


MAX_FILES = 8
MAX_BYTES = 256 * 1024
MAX_DEPTH = 48
MAX_RESPONSES = 12
METHODS = {"GET", "PUT", "POST", "DELETE", "OPTIONS", "HEAD", "PATCH", "TRACE"}
STATUS = re.compile(r"[1-5](?:[0-9]{2}|XX)|default")
FIELD = re.compile(r"[A-Za-z_][A-Za-z0-9_.-]{0,71}")
SCHEMA_TYPES = {"object", "array", "string", "integer", "number", "boolean", "null"}
PARAMETER = re.compile(r"\{[A-Za-z_][A-Za-z0-9_]*\}|:[A-Za-z_][A-Za-z0-9_]*")
TOKEN = re.compile(r'"(?:[^"\\]|\\.)*"|[{}\[\]:,]|[^\s{}\[\]:,"]+')


def route(method, path, *, specification=False):
    if (method not in METHODS or not isinstance(path, str) or not 0 < len(path) <= 512
            or not path.startswith("/") or path.startswith("//") or re.search(r"[?#\\%*`\s]", path)):
        return None
    parts = path[1:].split("/")
    shape = []
    for part in parts:
        if PARAMETER.fullmatch(part) and (not specification or part.startswith("{")):
            shape.append(None)
        elif part in {".", ".."} or re.search(r"[{}]", part):
            return None
        else:
            shape.append(part)
    return method, tuple(shape)


def unique_object(properties):
    result = dict(properties)
    if len(result) != len(properties):
        raise ValueError("Duplicate JSON property")
    return result


def fail_constant(_):
    raise ValueError("Non-JSON value")


def sites(text):
    """Index positions of direct path operations in one pass over valid JSON."""
    tokens = iter(TOKEN.finditer(text))
    matches = {}

    def visit(token, path, depth):
        if depth > MAX_DEPTH:
            raise ValueError("OpenAPI JSON nesting exceeds preparation bound")
        if token[0] not in {"{", "["}:
            return token.end()
        opened, closed = ("{", "}") if token[0] == "{" else ("[", "]")
        current = next(tokens)
        while current[0] != closed:
            if opened == "{":
                key = json.loads(current[0]) if path is not None else None
                child = (*path, key) if path is not None and len(path) < 3 else None
                begin = current.start()
                next(tokens)  # colon
                end = visit(next(tokens), child, depth + 1)
                if child and len(child) == 3 and child[0] == "paths":
                    matches[child[1:]] = (begin, end)
            else:
                visit(current, None, depth + 1)
            current = next(tokens)
            if current[0] == ",":
                current = next(tokens)
        return current.end()

    visit(next(tokens), (), 0)
    return matches


@lru_cache(maxsize=MAX_FILES)
def json_declaration(text):
    if not isinstance(text, str) or not text.lstrip().startswith("{") or len(text.encode()) > MAX_BYTES:
        return None
    try:
        result = json.loads(text, object_pairs_hook=unique_object, parse_constant=fail_constant)
        if not (isinstance(result, dict) and isinstance(result.get("openapi"), str)
                and re.fullmatch(r"3\.(?:0|[1-9][0-9]{0,7})\.(?:0|[1-9][0-9]{0,7})", result["openapi"])
                and isinstance(result.get("paths"), dict)):
            return None
        return result, sites(text)
    except (ValueError, StopIteration, RecursionError):
        return None


def response_schema(schema, document, depth=0):
    if not isinstance(schema, dict) or depth > 2:
        return None
    if reference := schema.get("$ref"):
        if not isinstance(reference, str) or not (match := re.fullmatch(r"#/components/schemas/([A-Za-z_][\w.-]{0,71})", reference, re.ASCII)):
            return "a schema reference whose target remains open"
        name = match[1]
        components = document.get("components")
        schemas = components.get("schemas") if isinstance(components, dict) else None
        return f"schema `{name}`" if isinstance(schemas, dict) and name in schemas else (
            f"schema reference `{name}` whose retained target remains open")
    value = schema.get("type")
    if not isinstance(value, str) or value not in SCHEMA_TYPES:
        return None
    if value == "array":
        element = response_schema(schema.get("items"), document, depth + 1)
        return f"array of {element}" if element else "array with item schema open"
    fields = schema.get("properties")
    if value == "object" and isinstance(fields, dict):
        names = sorted(name for name in fields if FIELD.fullmatch(name))
        if names:
            listed = ", ".join(f"`{name}`" for name in names[:8])
            return "object with declared fields " + listed + (" and additional fields" if len(names) > 8 else "")
    return value


def document_responses(operation, document):
    responses = operation.get("responses")
    if not isinstance(responses, dict):
        return None
    statuses = sorted(key for key in responses if STATUS.fullmatch(key))
    if not statuses:
        return None
    details = []
    for status in statuses[:MAX_RESPONSES]:
        response = responses[status]
        content = response.get("content") if isinstance(response, dict) else None
        schemas = []
        if isinstance(content, dict):
            for name, media in sorted(content.items()):
                if (len(schemas) < 3 and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9.+-]{0,48}/[A-Za-z0-9*][A-Za-z0-9.+*-]{0,64}", name)
                        and isinstance(media, dict) and (shape := response_schema(media.get("schema"), document))):
                    schemas.append(f"`{name}` {shape}")
        elif isinstance(response, dict) and "$ref" in response:
            schemas.append("a response reference whose target remains open")
        details.append(f"`{status}`" + (" (" + ", ".join(schemas) + ")" if schemas else ""))
    return ", ".join(details) + (" and additional declared responses" if len(statuses) > MAX_RESPONSES else "")


def witness(source, span, offsets):
    text = source["text"]
    start, end = span
    first, last = len(text[:start].encode()), len(text[:end].encode())
    result = {"file": source["file"], "revision": source["revision"],
              "startLine": bisect_right(offsets, start) + 1, "endLine": bisect_right(offsets, end - 1) + 1,
              "textDigest": "sha256:" + hashlib.sha256(text[start:end].encode()).hexdigest()}
    origin = (source.get("occurrence") or {}).get("startByte")
    if type(origin) is int:
        result.update(startByte=origin + first, endByte=origin + last)
    return result


def admitted(source, service, revision):
    text = source.get("text")
    return (source.get("service") == service and source.get("revision") == revision
            and source.get("authority") in {"DECLARED_OPENAPI", "EXACT_SNAPSHOT_TEXT", "REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS"}
            and source.get("startLine") == 1 and isinstance(text, str) and 0 < len(text.encode()) <= MAX_BYTES
            and source.get("endLine") == len(text.split("\n")) - text.endswith("\n")
            and source.get("textDigest") == "sha256:" + hashlib.sha256(text.encode()).hexdigest()
            and not (source.get("occurrence") or {}).get("generatedOrigin"))


def project(service, revision, requirements, sources, operations):
    """Bind a unique declaration and endpoint of the same method and route shape."""
    by_ref, declarations, json_files = {}, defaultdict(dict), {}
    targets = defaultdict(list)
    for entry in requirements:
        if entry.get("entryKind") == "HTTP_ENDPOINT" and (key := route(entry.get("method"), entry.get("path"))):
            targets[key].append(entry)
    admitted_sources = {handle: source for handle, source in sources.items() if admitted(source, service, revision)}
    for handle, source in admitted_sources.items():
        for ref in source.get("references", ()):
            by_ref[ref] = handle
        if not (parsed := json_declaration(source["text"])):
            continue
        document, spans = parsed
        json_files[source["file"]] = document
        offsets = [match.end() for match in re.finditer("\n", source["text"])]
        for (path, method), span in spans.items():
            operation = document["paths"].get(path)
            key = route(method.upper(), path, specification=True) if method in {verb.lower() for verb in METHODS} else None
            if (key not in targets or not isinstance(operation, dict) or "$ref" in operation
                    or not isinstance(operation.get(method), dict)):
                continue
            response = document_responses(operation[method], document)
            declarations[key].setdefault((source["file"], path), {
                "method": method.upper(), "path": path, "response": response, "sources": [handle],
                "witnesses": [dict(cite=handle, **witness(source, span, offsets))],
            })

    for row in operations:
        record = row.get("record") or {}
        item = record.get("normalized") or {}
        if (record.get("kind") != "CONTRACT_OPERATION" or record.get("service") != service
                or item.get("schema") != "codeclew-documentation-contract-facts/1.0"
                or item.get("authority") != "DECLARED_OPENAPI" or item.get("runtimeEnforcement") != "UNVERIFIED"
                or not isinstance(item.get("operation"), dict)):
            continue
        refs = row.get("sourceReferences", ())
        if not refs or any(ref not in by_ref for ref in refs):
            continue
        handles = list(dict.fromkeys(by_ref[ref] for ref in refs))
        file = item.get("declaredSource")
        if not isinstance(file, str) or file.lower().endswith(".json") and file not in json_files:
            continue
        if file in json_files:
            path_item = json_files[file]["paths"].get(item.get("path"))
            if not isinstance(path_item, dict) or "$ref" not in path_item:
                # Direct retained JSON keys and statuses are already authoritative.
                continue
        if not any(admitted_sources[handle]["file"] == file
                   and admitted_sources[handle]["authority"] == "DECLARED_OPENAPI" for handle in handles):
            continue
        if key := route(item.get("method"), item.get("path"), specification=True):
            declarations[key].setdefault((file, item["path"]), {
                "method": item["method"], "path": item["path"], "response": document_responses(item["operation"], {}),
                "sources": handles, "entrypoint": item.get("entrypoint"),
                "ambiguous": item.get("sourceMapping") == "AMBIGUOUS_SOURCE_ROUTE_MATCH",
            })

    result = {}
    for key, entries in targets.items():
        candidates = list(declarations.get(key, {}).values())
        if not candidates:
            continue
        for entry in entries:
            if (len(entries) != 1 or len(candidates) != 1 or candidates[0].get("ambiguous")
                    or candidates[0].get("entrypoint") not in {None, entry["entrypoint"]}):
                handles = list(dict.fromkeys(handle for candidate in candidates[:4] for handle in candidate["sources"]))
                result[entry["id"]] = [{
                    "kind": "openapi-overlay-boundary", "authority": "DECLARED_OPENAPI", "status": "AMBIGUOUS",
                    "text": "More than one retained declaration or source handler can correspond to this method and route.",
                    "uncertainty": "The governing API declaration and runtime binding remain unknown.", "sources": handles,
                }]
                continue
            candidate = candidates[0]
            label = f"`{candidate['method']} {candidate['path']}`"
            outcome = candidate["response"]
            result[entry["id"]] = [{
                "kind": "openapi-declared-response" if outcome else "openapi-response-boundary",
                "authority": "DECLARED_OPENAPI", "status": "SOURCE_ROUTE_MATCH_ONLY",
                "text": f"The retained OpenAPI operation {label} declares responses {outcome}." if outcome
                        else f"The retained OpenAPI operation {label} has no supported response details in this projection.",
                "uncertainty": "Implementation-derived statuses and schemas are separate evidence. "
                               "Deployed routing, runtime enforcement and actual client outcomes remain unknown.",
                "sources": candidate["sources"],
                **({"witnesses": candidate["witnesses"]} if "witnesses" in candidate else {}),
            }]
    return result
