#!/usr/bin/env python3
"""Verify a flat deterministic HTML export against its complete checked pages.

No Markdown interpretation, semantic approval, acquisition or renderer changes.
The explicit caller-trusted renderer executes with the caller's permissions.
"""
import argparse
from html.parser import HTMLParser
import json
from pathlib import Path
import runpy
import subprocess
import sys
from urllib.parse import quote, unquote, urlsplit
from xml.etree import ElementTree

M = runpy.run_path(str(Path(__file__).with_name("page_citations.py")))
R = runpy.run_path(str(Path(__file__).with_name("reader_markdown_repair.py")),
                 init_globals={"CITATIONS": M["C"]})
Stop, encode, require, read, sha = (M[k] for k in ("Stop", "encode", "require", "read", "sha"))
NATIVE_ENGINE_ARGS = []
FAVICON = "favicon.svg"


def configure_engine(clew=None, root=None, components=None, component_set_digest=None, evidence_inputs=None):
    global NATIVE_ENGINE_ARGS
    M["configure_engine"](clew, root, components, component_set_digest, evidence_inputs)
    NATIVE_ENGINE_ARGS = ([] if clew is None else ["--clew", str(clew),
        "--evidence-inputs" if evidence_inputs is not None else "--root",
        str(Path(evidence_inputs).absolute()) if evidence_inputs is not None else str(root)])
    if components is not None:
        NATIVE_ENGINE_ARGS.extend(["--components", str(Path(components).absolute()), "--component-set-digest", component_set_digest])


def inventory(root, suffix, *, only_pages=False):
    root = Path(root)
    require(root.is_dir() and not root.is_symlink(), "INVALID_EXPORT_DIRECTORY", path=str(root))
    found = set()
    for path in root.iterdir():
        if path.name == ".DS_Store":
            continue
        if only_pages:
            require(path.is_file() and not path.is_symlink() and (path.suffix == suffix or path.name == FAVICON),
                    "UNSUPPORTED_EXPORT_LAYOUT", path=str(path))
        if path.suffix == suffix:
            require(path.is_file() and not path.is_symlink(), "UNSAFE_PAGE_ARTIFACT", artifact=path.name)
            found.add(path.name)
            require(len(found) <= M["MAX_PAGES"], "PAGE_BYTES_LIMIT")
    return found


class Links(HTMLParser):
    """Inspect rendered targets only; never approximate Markdown or text meaning."""
    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.targets, self.links, self.favicons = set(), [], []

    def handle_starttag(self, tag, attrs):
        require(len(attrs) == len({key for key, _ in attrs}), "AMBIGUOUS_EXPORT_ATTRIBUTES")
        values = dict(attrs)
        require(tag != "base", "UNSUPPORTED_EXPORT_BASE_URL")
        for key in ("id", "name") if tag == "a" else ("id",):
            target = values.get(key)
            if not target or key == "name" and target == values.get("id"):
                continue
            require(target not in self.targets, "AMBIGUOUS_EXPORT_ANCHOR", anchor=target)
            self.targets.add(target)
        for key in ("href", "src"):
            if key in values:
                require(values[key] is not None, "INVALID_EXPORT_LINK")
                if (tag == "link" and key == "href" and "icon" in (values.get("rel") or "").lower().split()
                        and values.get("type") == "image/svg+xml" and values[key] == FAVICON):
                    self.favicons.append(FAVICON)
                    continue
                self.links.append(values[key])
        require("srcset" not in values, "UNSUPPORTED_EXPORT_ASSET")


def check_links(pages, favicon=None):
    documents = {}
    for name, raw in pages.items():
        document = Links(); document.feed(raw.decode("utf-8")); document.close()
        documents[name] = document
    uses_favicon = any(document.favicons for document in documents.values())
    require(not uses_favicon or favicon is not None, "EXPORT_FAVICON_MISSING")
    require(favicon is None or uses_favicon, "UNREFERENCED_EXPORT_FAVICON")
    if favicon is not None:
        try:
            icon = ElementTree.fromstring(favicon)
        except ElementTree.ParseError:
            require(False, "INVALID_EXPORT_FAVICON")
        require(icon.tag == "{http://www.w3.org/2000/svg}svg", "INVALID_EXPORT_FAVICON")
    count = 0
    for name, document in documents.items():
        for href in document.links:
            require(not any(ord(c) < 32 for c in href) and "\\" not in href,
                    "INVALID_EXPORT_LINK", page=name, href=href)
            parsed = urlsplit(href)
            if parsed.scheme or parsed.netloc:
                require(parsed.scheme.lower() in ("http", "https", "mailto", "tel") or
                        not parsed.scheme and bool(parsed.netloc), "UNSUPPORTED_EXPORT_LINK", page=name, href=href)
                continue
            target = unquote(parsed.path, errors="strict") or name
            require(target in documents, "EXPORT_LINK_TARGET_MISSING_OR_UNSUPPORTED", page=name, href=href)
            fragment = unquote(parsed.fragment, errors="strict")
            require(not fragment or fragment in documents[target].targets,
                    "EXPORT_FRAGMENT_MISSING", page=name, href=href)
            count += 1
    return count


class SearchHTML(Links):
    """Inspect the packaged catalog's actual controls, data and card links."""
    def __init__(self):
        super().__init__()
        self.stack, self.catalogs, self.scripts, self.sequences = [], [], [], []

    def handle_starttag(self, tag, attrs):
        super().handle_starttag(tag, attrs)
        node = {"tag": tag, "attrs": dict(attrs), "children": [], "text": ""}
        if self.stack:
            self.stack[-1]["children"].append(node)
        if tag == "section" and "route-catalog" in (dict(attrs).get("class") or "").split():
            self.catalogs.append(node)
        if tag == "script":
            self.scripts.append(node)
        if tag == "figure" and "clew-sequence" in (dict(attrs).get("class") or "").split():
            self.sequences.append(node)
        if tag not in {"area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"}:
            self.stack.append(node)

    def handle_endtag(self, tag):
        for i in range(len(self.stack) - 1, -1, -1):
            if self.stack[i]["tag"] == tag:
                del self.stack[i:]
                break

    def handle_data(self, data):
        if self.stack:
            self.stack[-1]["text"] += data


def check_sequences(pages, argv):
    """Opt-in packaged diagram/trace presence, using declared reader destinations.

    No Markdown guessing, endpoint inference, semantic or browser assessment.
    """
    def option(flag, required=True):
        count = argv.count(flag)
        require(count == 1 if required else count <= 1, "SEQUENCE_CONFIGURATION_MISSING", option=flag)
        if not count:
            return None
        require(argv.index(flag) + 1 < len(argv), "SEQUENCE_CONFIGURATION_MISSING", option=flag)
        return argv[argv.index(flag) + 1]

    path = Path(option("--navigation"))
    raw = read(path.parent, path.name, 1024 * 1024)
    navigation = json.loads(raw)
    require(navigation.get("schema") == "codeclew-markdown-navigation/1", "SEQUENCE_NAVIGATION_INVALID")
    required_pages = set()

    def destination(target, suffix):
        name = target.split("#", 1)[0]
        require(name and Path(name).name == name and name.endswith(suffix)
                and not any(c in name for c in "\\:?"), "SEQUENCE_TARGET_INVALID", target=target)
        html = str(Path(name).with_suffix(".html"))
        require(html in pages, "SEQUENCE_TARGET_MISSING", page=html)
        required_pages.add(html)

    for service in navigation["services"]:
        for start in service["starts"]:
            if start["kind"] == "http":
                destination(start["target"], ".md")
    for route in navigation["routes"]:
        destination(route["target"], ".md")
    registry_digest = None
    registry_path = option("--docset-routes", required=False)
    if registry_path is not None:
        registry_path = Path(registry_path)
        registry_raw = read(registry_path.parent, registry_path.name, 1024 * 1024)
        registry = json.loads(registry_raw)
        require(registry.get("schema") in {"codeclew-cross-docset-routes/1.1", "codeclew-cross-docset-routes/1.2"},
                "SEQUENCE_REGISTRY_INVALID")
        docset = option("--docset")
        for route in registry["routes"]:
            target = route["narrative"]
            if target["docset"] == docset and not target.get("problem"):
                destination(target["page"], ".html")
        registry_digest = sha(registry_raw)
    require(required_pages, "SEQUENCE_REQUIRED_PAGES_EMPTY")

    def descendants(node):
        for child in node["children"]:
            yield child
            yield from descendants(child)

    def has_class(node, value):
        return value in (node["attrs"].get("class") or "").split()

    counts = {}
    for name in sorted(required_pages):
        document = SearchHTML(); document.feed(pages[name].decode("utf-8")); document.close()
        require(document.sequences, "REQUIRED_SEQUENCE_MISSING", page=name,
                action="Author a clew-sequence fence for this entry/process; a heading or numbered list is not a diagram.")
        for figure in document.sequences:
            nodes = list(descendants(figure))
            diagrams = [n for n in nodes if has_class(n, "sequence-diagram")]
            traces = [n for n in nodes if n["tag"] == "details" and has_class(n, "sequence-text")]
            require(len(diagrams) == len(traces) == 1, "REQUIRED_SEQUENCE_INCOMPLETE", page=name)
            rows = [n for n in descendants(diagrams[0]) if has_class(n, "sequence-row")]
            lists = [n for n in descendants(traces[0]) if n["tag"] == "ol"]
            steps = [] if len(lists) != 1 else [n for n in lists[0]["children"] if n["tag"] == "li"]
            require(rows and len(rows) == len(steps)
                    and all(any(n["text"].strip() for n in descendants(row)) for row in rows)
                    and all(step["text"].strip() or any(n["text"].strip() for n in descendants(step)) for step in steps),
                    "REQUIRED_SEQUENCE_INCOMPLETE", page=name)
        counts[name] = len(document.sequences)
    return {"status": "SEQUENCE_STRUCTURE_CHECKED", "pages": counts, "navigationDigest": sha(raw),
            "registryDigest": registry_digest, "semanticCoverageAssessed": False, "browserBehaviorVerified": False}


def check_process_search(pages, argv):
    """Opt-in structural guard for skill-owned multi-service process search.

    No network fetch, semantic coverage or browser/JavaScript execution claim.
    Legacy and caller-selected renderers remain outside this explicit contract.
    """
    def option(flag):
        require(argv.count(flag) == 1 and argv.index(flag) + 1 < len(argv),
                "PROCESS_SEARCH_CONFIGURATION_MISSING", option=flag,
                action="Configure packaged docset routes, a local catalog, --docset and --docset-version; service links alone are not process search.")
        return argv[argv.index(flag) + 1]

    registry_path = Path(option("--docset-routes"))
    docset, version = option("--docset"), option("--docset-version")
    raw = read(registry_path.parent, registry_path.name, 1024 * 1024)
    registry = json.loads(raw)
    require(registry.get("schema") in {"codeclew-cross-docset-routes/1.1", "codeclew-cross-docset-routes/1.2"},
            "PROCESS_SEARCH_REGISTRY_INVALID")
    sets = registry["docsets"]
    require(any(d["id"] == docset and d["version"] == version for d in sets), "PROCESS_SEARCH_VERSION_MISMATCH")
    services = {d["id"] + "/" + s["id"] for d in sets for s in d["services"]}
    routes = registry["routes"]
    require(len(services) > 1 and routes, "PROCESS_SEARCH_EMPTY", action="Declare the evidenced processes and participating services; preserve unknown roles.")
    catalog = registry.get("catalog", {})
    require(catalog.get("docset") == docset, "PROCESS_SEARCH_LOCAL_CATALOG_REQUIRED",
            action="Check the catalog's own export. A remote peer link is only a declaration, never an availability or functional PASS.")
    name = catalog.get("page")
    require(name in pages, "PROCESS_SEARCH_CATALOG_MISSING", page=name)
    documents = {}
    for page, body in pages.items():
        parsed = SearchHTML(); parsed.feed(body.decode("utf-8")); parsed.close()
        documents[page] = parsed
    document = documents[name]
    require(len(document.catalogs) == 1, "PROCESS_SEARCH_CATALOG_MISSING", page=name)
    root = document.catalogs[0]

    def descendants(node):
        yield node
        for child in node["children"]:
            yield from descendants(child)

    nodes = list(descendants(root))
    require("hidden" not in root["attrs"], "PROCESS_SEARCH_CONTROLS_MISSING")
    forms = [n for n in nodes if n["tag"] == "form" and "route-filters" in (n["attrs"].get("class") or "").split()]
    require(len(forms) == 1, "PROCESS_SEARCH_CONTROLS_MISSING")

    def enabled(node):
        if "hidden" not in node["attrs"] and "disabled" not in node["attrs"]:
            yield node
            for child in node["children"]:
                yield from enabled(child)

    form_nodes = {id(n) for n in descendants(forms[0])}
    enabled_inputs = [n for n in enabled(root) if id(n) in form_nodes and n["tag"] == "input"]
    for field in ("start", "end", "intermediate"):
        controls = [n for n in enabled_inputs if n["attrs"].get("name") == field]
        require(len(controls) == len(services) and {n["attrs"].get("value") for n in controls} == services
                and all(n["attrs"].get("type") == "checkbox" and "disabled" not in n["attrs"] for n in controls),
                "PROCESS_SEARCH_CONTROLS_MISSING", field=field)
    outcomes = [n for n in enabled_inputs if n["attrs"].get("name") == "outcomeKinds"]
    require(len(outcomes) == 4 and {n["attrs"].get("value") for n in outcomes}
            == {"normal", "error", "handoff", "unknown"}
            and all(n["attrs"].get("type") == "checkbox" for n in outcomes),
            "PROCESS_SEARCH_CONTROLS_MISSING", field="outcomeKinds")
    unknown = [n for n in enabled_inputs if n["attrs"].get("name") == "includeUnknown"]
    require(len(unknown) == 1 and unknown[0]["attrs"].get("type") == "checkbox"
            and "checked" in unknown[0]["attrs"],
            "PROCESS_SEARCH_CONTROLS_MISSING", field="includeUnknown")
    prefix = (root["attrs"].get("id") or "").removesuffix("-catalog")
    payloads = [s for s in document.scripts if s["attrs"].get("id") == prefix + "-data" and s["attrs"].get("type") == "application/json"]
    require(prefix and len(payloads) == 1, "PROCESS_SEARCH_DATA_MISSING")
    payload = json.loads(payloads[0]["text"])
    require(payload.get("registry") == registry, "PROCESS_SEARCH_REGISTRY_MISMATCH")
    require(any(s["attrs"].get("type") != "application/json" and json.dumps(prefix) in s["text"]
                and "catalogClient" in s["text"] for s in document.scripts), "PROCESS_SEARCH_CLIENT_MISSING")
    cards = [n for n in nodes if n["tag"] == "article" and "route-card" in (n["attrs"].get("class") or "").split()]
    require(len(cards) == len(routes) and {n["attrs"].get("data-route-id") for n in cards} == {r["id"] for r in routes},
            "PROCESS_SEARCH_ROUTES_MISSING")
    by_id = {n["attrs"]["data-route-id"]: n for n in cards}

    def href(target):
        path = quote(target["page"], safe="-_.!~*'()")
        if target.get("anchor"):
            path += "#" + quote(target["anchor"], safe="-_.!~*'()")
        if target["docset"] == docset:
            require(not target.get("problem") and target["page"] in documents
                    and (not target.get("anchor") or target["anchor"] in documents[target["page"]].targets),
                    "PROCESS_SEARCH_TARGET_MISSING", target=target)
        else:
            base = next(d["baseURL"] for d in sets if d["id"] == target["docset"])
            path = base + path
        return path

    catalog_links = {n["attrs"].get("href") for n in nodes if n["tag"] == "a"}
    for d in sets:
        for service in d["services"]:
            if not service["overview"].get("problem"):
                require(href(service["overview"]) in catalog_links, "PROCESS_SEARCH_SERVICE_LINK_MISSING")
    for route in routes:
        links = {n["attrs"].get("href") for n in descendants(by_id[route["id"]]) if n["tag"] == "a"}
        for target in [route["narrative"], *(o["target"] for o in route["occurrences"])]:
            if not target.get("problem"):
                require(href(target) in links, "PROCESS_SEARCH_ROUTE_LINK_MISSING", route=route["id"])
    return {"status": "PROCESS_SEARCH_STRUCTURE_CHECKED", "catalog": name, "registryDigest": sha(raw),
            "services": len(services), "routes": len(routes), "remoteTargetsVerified": False,
            "browserBehaviorVerified": False, "semanticCoverageAssessed": False}


def verify(manifests, index_path, authored, exported, renderer, output, *, require_process_search=False, require_sequences=False):
    authored, exported, output = map(Path, (authored, exported, output))
    index_path = Path(index_path)
    require(not output.exists() and not output.is_symlink(), "OUTPUT_ALREADY_EXISTS")
    # The check must never create its receipts inside an input/evidence tree.
    for root in (authored, exported, index_path.parent, *(Path(p).parent for p in manifests)):
        require(not output.resolve().is_relative_to(root.resolve()), "CHECK_OUTPUT_OVERLAPS_INPUT")
    output.mkdir(parents=True)
    index_raw = read(index_path.parent, index_path.name)
    native = M["check"](manifests, index_path)
    (output / "native-check.json").write_text(encode(native), encoding="utf-8")
    require(read(index_path.parent, index_path.name) == index_raw, "EXPORT_INPUT_CHANGED")
    index = json.loads(index_raw)
    names = {entry["page"] for entry in index["pages"]}
    require(all(Path(name).name == name for name in names), "UNSUPPORTED_EXPORT_LAYOUT")
    require(inventory(authored, ".md") == names, "AUTHORED_PAGE_INVENTORY_MISMATCH")
    expected_names = {str(Path(name).with_suffix(".html")) for name in names}
    require(inventory(exported, ".html", only_pages=True) == expected_names, "EXPORT_PAGE_INVENTORY_MISMATCH")
    favicon = read(exported, FAVICON) if (exported / FAVICON).exists() else None
    source = output / "input"; source.mkdir()
    outer, bound, actual = {}, {}, {}
    for entry in index["pages"]:
        name = entry["page"]
        outer[name] = read(authored, name)
        original = read(index_path.parent, entry["inputMarkdown"])
        body = read(index_path.parent, entry["markdown"])
        require(sha(original) == entry["inputMarkdownDigest"] and sha(body) == entry["markdownDigest"],
                "EXPORT_INPUT_CHANGED", page=name)
        require(outer[name] == original, "AUTHORED_MARKDOWN_MISMATCH", page=name)
        bound[name] = body
        (source / name).write_bytes(body)
        html = str(Path(name).with_suffix(".html")); actual[html] = read(exported, html)
    require(sum(map(len, bound.values())) <= M["MAX_TOTAL"] and
            sum(map(len, actual.values())) <= M["MAX_TOTAL"], "PAGE_BYTES_LIMIT")
    M["component_call"]("validate_html", index.get("components"), actual)
    command_raw = Path(renderer).read_bytes()
    argv = R["renderer_command"](renderer)
    (output / "renderer.original.json").write_bytes(command_raw)
    expected = output / "expected"
    receipt = R["run_renderer"](argv, source.resolve(), expected.resolve(), output, "expected")
    require(inventory(expected, ".html", only_pages=True) == expected_names, "RENDERED_PAGE_INVENTORY_MISMATCH")
    expected_favicon = read(expected, FAVICON) if (expected / FAVICON).exists() else None
    require(expected_favicon == favicon, "EXPORTED_FAVICON_MISMATCH")
    for name, body in bound.items():
        require(read(source, name) == body and read(authored, name) == outer[name], "EXPORT_INPUT_CHANGED", page=name)
    for name, raw in actual.items():
        require(read(expected, name) == raw, "EXPORTED_HTML_MISMATCH", page=name)
        require(read(exported, name) == raw, "EXPORT_INPUT_CHANGED", page=name)
    require(inventory(authored, ".md") == names and
            inventory(exported, ".html", only_pages=True) == expected_names and
            read(index_path.parent, index_path.name) == index_raw and
            Path(renderer).read_bytes() == command_raw, "EXPORT_INPUT_CHANGED")
    require((read(exported, FAVICON) if (exported / FAVICON).exists() else None) == favicon,
            "EXPORT_INPUT_CHANGED")
    links = check_links(actual, favicon)
    search = check_process_search(actual, argv) if require_process_search else None
    sequences = check_sequences(actual, argv) if require_sequences else None
    return {"schema": "codeclew-reader-export-check/1.0", "status": "EXPORT_CHECKED",
        **({"processSearch": search} if search is not None else {}),
        **({"sequences": sequences} if sequences is not None else {}),
        "pages": len(names), "claims": native["claims"], "internalLinks": links,
        "nativeCheck": "native-check.json", "indexDigest": index["indexDigest"],
        "rendererCommandDigest": sha(command_raw), "rendererReceipt": receipt,
        "authoredDigests": {name: sha(raw) for name, raw in outer.items()},
        "exportDigests": {name: sha(raw) for name, raw in actual.items()},
        "exportAssetDigests": {FAVICON: sha(favicon)} if favicon is not None else {},
        "semanticTruthAssessed": False, "runtimeVerified": False,
        "nativePublicationApproval": False, "sourceFallbackAllowed": False,
        "limits": "Exact full HTML bytes from the explicit trusted renderer over complete checked Markdown, plus byte-checked local favicon.svg referenced as a typed SVG icon. Other local assets, nested/root-relative links, base URLs and srcset unsupported. External URL availability, renderer faithfulness, CSS/browser visibility and semantic completeness unassessed. Renderer must be deterministic and independent of input/output locations; keep inputs stable during check. Caller-trusted argv is not sandboxed."}


def checked_command(argv, receipts, label, expected):
    """Run an existing CLI operation and retain its actual complete receipt."""
    record = {"argv": list(map(str, argv)), "exitCode": None}
    stdout, stderr = b"", b""
    try:
        completed = subprocess.run(argv, capture_output=True)
        stdout, stderr = completed.stdout, completed.stderr
        record["exitCode"] = completed.returncode
    except OSError as error:
        record["error"] = str(error)
    for suffix, raw in (("stdout", stdout), ("stderr", stderr), ("command.json", encode(record).encode())):
        (receipts / (label + "." + suffix)).write_bytes(raw)
    try:
        value = json.loads(stdout)
    except (ValueError, UnicodeDecodeError):
        value = None
    require(record["exitCode"] == 0, "FINALIZATION_STAGE_FAILED", stage=label, exitCode=record["exitCode"],
            nativeStatus=value.get("status") if isinstance(value, dict) else None)
    require(isinstance(value, dict) and value.get("status") == expected,
            "FINALIZATION_STAGE_NOT_CHECKED", stage=label)
    return value


def finalize(compose_root, manifests, renderer, output, *, previous=None, repositories=None,
             source_footers=False, source_views=(), require_process_search=False, require_sequences=False):
    """Complete reviewed pages using existing bind, renderer and full check."""
    compose_root, renderer, output = map(Path, (compose_root, renderer, output))
    require(source_footers or not source_views, "SOURCE_VIEW_REQUIRES_FOOTERS")
    require(not output.exists() and not output.is_symlink(), "OUTPUT_ALREADY_EXISTS")
    require(output.parent.is_dir() and not output.parent.is_symlink(), "INVALID_COMPOSE_PARENT")
    require(compose_root.is_dir() and not compose_root.is_symlink(), "INVALID_COMPOSE_DIRECTORY")
    compose_root, renderer, output = compose_root.resolve(), renderer.resolve(), output.resolve()
    manifests = [Path(p).absolute() for p in manifests]
    for source in (compose_root, *(p.parent for p in manifests), *((Path(previous).absolute().parent,) if previous else ())):
        require(not output.is_relative_to(source.resolve()), "FINALIZATION_OUTPUT_OVERLAPS_INPUT")
    output.mkdir(); receipts = output / "receipts"; receipts.mkdir()
    state = {"status": "FINALIZATION_FAILED", "root": str(output), "inputCompose": str(compose_root),
        "composeRoot": "compose", "original": "compose/original.json", "authoredRoot": "compose/markdown",
        "renderedRoot": "html", "sourcesJson": "compose/markdown/sources.json",
        "manifests": list(map(str, manifests)), "rendererCommand": "renderer.json",
        "receipts": "receipts", "exportCheck": "check/result.json", "result": "result.json",
        "semanticTruthAssessed": False, "semanticReviewRequired": True, "visualReviewRequired": True,
        "nativePublicationApproval": False, "sourceFallbackAllowed": False}
    stage = "prepare"
    try:
        original = read(compose_root, "original.json", M["MAX_MAP"])
        pages, generated, _ = M["compose_pages"](M["original_value"](original))
        request = read(compose_root, "request.json")
        require(M["original_value"](request) == generated, "COMPOSE_PACKAGE_CHANGED")
        require(all(Path(name).name == name for name in pages), "UNSUPPORTED_EXPORT_LAYOUT")
        require(inventory(compose_root / "markdown", ".md") == set(pages), "AUTHORED_PAGE_INVENTORY_MISMATCH")
        require(all(read(compose_root / "markdown", name) == raw for name, raw in pages.items()), "COMPOSE_PACKAGE_CHANGED")
        renderer_raw = renderer_input_raw = read(renderer.parent, renderer.name)
        argv = R["renderer_command"](renderer)
        require(not source_footers or '--sources' not in argv, "SOURCE_FOOTERS_ALREADY_CONFIGURED")
        if source_footers:
            (output / 'renderer.request.json').write_bytes(renderer_input_raw)
        authored = output / state["authoredRoot"]; authored.mkdir(parents=True)
        (output / state["original"]).write_bytes(original)
        (output / state["composeRoot"] / "request.json").write_bytes(request)
        for name, raw in pages.items():
            (authored / name).write_bytes(raw)
        command_path = output / state["rendererCommand"]; command_path.write_bytes(renderer_raw)
        (receipts / "prepare.json").write_text(encode({"originalSHA256": sha(original), "requestSHA256": sha(request),
            "markdownDigests": {name: sha(raw) for name, raw in pages.items()}, "rendererCommandDigest": sha(renderer_raw),
            "previousIndex": str(Path(previous).absolute()) if previous else None,
            "repositories": str(Path(repositories).absolute()) if repositories else None}), encoding="utf-8")
        scripts = Path(__file__).resolve().parent
        bind_argv = [sys.executable, "-I", "-S", str(scripts / "page_citations.py"), "bind",
            "--markdown-root", str(authored), "--request", str(output / state["composeRoot"] / "request.json"),
            "--output-dir", str(authored / ".codeclew/bound")]
        manifest_args = [arg for path in manifests for arg in ("--manifest", str(path))]
        bind_argv.extend(manifest_args)
        bind_argv.extend(NATIVE_ENGINE_ARGS)
        for flag, path in (("--previous-index", previous), ("--repositories", repositories)):
            if path is not None: bind_argv.extend([flag, str(Path(path).absolute())])
        stage = "bind"
        bound = checked_command(bind_argv, receipts, stage, "PAGE_MAPS_BOUND")
        index = Path(bound["index"])
        require(index.is_file() and index == authored / ".codeclew/bound/index.json", "FINALIZATION_INDEX_PATH_MISMATCH")
        state["checkedIndex"] = index.relative_to(output).as_posix()
        pointer = {"checkedIndex": index.relative_to(authored).as_posix()}
        with (authored / "sources.json").open("x", encoding="utf-8") as stream:
            stream.write(encode(pointer))
        (receipts / "sources-pointer.json").write_text(encode(pointer), encoding="utf-8")
        bundle_raw = None
        if source_footers:
            stage = 'source-footers'
            bundle = output / 'source-footers.json'
            source_argv = [sys.executable, '-I', '-S', str(scripts / 'reader_sources.py'),
                           '--index', str(index), *manifest_args, '--output', str(bundle)]
            if NATIVE_ENGINE_ARGS[:1] == ['--clew']:
                source_argv.extend(NATIVE_ENGINE_ARGS[:4])
            for view in source_views:
                source_argv.extend(['--view', str(Path(view).resolve())])
            checked_command(source_argv, receipts, stage, 'SOURCE_FOOTERS_MATERIALIZED')
            bundle_raw = bundle.read_bytes()
            state['sourceFooterBundle'] = bundle.name
            argv = [*argv, '--sources', str(bundle)]
            renderer_raw = encode(argv).encode('utf-8')
            command_path.write_bytes(renderer_raw)
        stage = "render"
        try:
            R["run_renderer"](argv, index.parent / "markdown", output / state["renderedRoot"], receipts, stage)
        finally:
            for suffix in ("stdout", "stderr"):
                path = receipts / (stage + ".render." + suffix)
                if not path.exists(): path.write_bytes(b"")
        require(read(output, state["rendererCommand"]) == renderer_raw, "EXPORT_INPUT_CHANGED")
        if require_process_search:
            stage = "process-search"
            html_root = output / state["renderedRoot"]
            state["processSearch"] = check_process_search(
                {p.name: read(html_root, p.name) for p in html_root.glob("*.html")}, argv)
        if require_sequences:
            stage = "sequences"
            html_root = output / state["renderedRoot"]
            state["sequences"] = check_sequences(
                {p.name: read(html_root, p.name) for p in html_root.glob("*.html")}, argv)
        stage = "export-check"
        checked = checked_command([sys.executable, "-I", "-S", str(scripts / "reader_export_check.py"),
            *manifest_args, *NATIVE_ENGINE_ARGS, "--index", str(index), "--authored-root", str(authored),
            "--rendered-root", str(output / state["renderedRoot"]), "--renderer-command", str(command_path),
            "--output-dir", str(output / "check"),
            *(["--require-process-search"] if require_process_search else []),
            *(["--require-sequences"] if require_sequences else [])], receipts, stage, "EXPORT_CHECKED")
        require(M["json_file"](output / state["exportCheck"]) == checked, "FINALIZATION_CHECK_RESULT_MISMATCH")
        require(not require_process_search or checked.get("processSearch") == state["processSearch"],
                "PROCESS_SEARCH_INPUT_CHANGED")
        require(not require_sequences or checked.get("sequences") == state["sequences"], "SEQUENCE_INPUT_CHANGED")
        # The explicit trusted renderer must not silently change the reviewed
        # original/outer inputs while producing its first render.
        require(read(compose_root, "original.json", M["MAX_MAP"]) == original
                and read(compose_root, "request.json") == request and read(renderer.parent, renderer.name) == renderer_input_raw
                and read(output / state["composeRoot"], "original.json", M["MAX_MAP"]) == original
                and read(output / state["composeRoot"], "request.json") == request
                and read(output, state["rendererCommand"]) == renderer_raw
                and M["json_file"](authored / "sources.json") == pointer
                and (bundle_raw is None or bundle.read_bytes() == bundle_raw)
                and all(read(compose_root / "markdown", name) == raw for name, raw in pages.items()), "EXPORT_INPUT_CHANGED")
        state.update(status="CHECKED_PAGES_FINALIZED", exportStatus=checked["status"], pages=checked["pages"], claims=checked["claims"])
    except (Stop, OSError, ValueError, TypeError, KeyError, IndexError) as error:
        state.update(stage=stage, stageStatus=error.status if isinstance(error, Stop) else "INVALID_FINALIZATION_INPUT",
                     details=error.details if isinstance(error, Stop) else str(error))
    (output / "result.json").write_text(encode(state), encoding="utf-8")
    return state


def main(argv=None):
    arguments = list(sys.argv[1:] if argv is None else argv)
    finalizing = bool(arguments and arguments[0] == "finalize")
    parser = argparse.ArgumentParser(description=__doc__, epilog="Use finalize --compose-root for complete reviewed compose pages.")
    parser.add_argument("--manifest", type=Path, action="append", required=True)
    parser.add_argument("--clew", type=Path, help="Caller-trusted launcher for SOURCE_NATIVE validation")
    parser.add_argument("--root", type=Path, help="Root owning the prepared native evidenceSet")
    parser.add_argument("--evidence-inputs", type=Path, help="Caller-trusted manifest identities and owning roots")
    parser.add_argument("--components", type=Path, help="Mandatory pre-author component ticket")
    parser.add_argument("--component-set-digest", help="Exact independently selected ticket ID")
    parser.add_argument("--require-process-search", action="store_true",
                        help="Require the packaged multi-service catalog's actual Starts/Includes controls, registry, routes and links; no browser/remote availability claim")
    parser.add_argument("--require-sequences", action="store_true",
                        help="Require packaged diagrams and nonempty matching-length text traces on navigation HTTP/shared-route pages and local docset narrative pages; no semantic/browser claim")
    for name in (("compose-root", "renderer-command", "output-dir") if finalizing else
                 ("index", "authored-root", "rendered-root", "renderer-command", "output-dir")):
        parser.add_argument("--" + name, type=Path, required=True)
    if finalizing:
        parser.add_argument("--previous-index", type=Path)
        parser.add_argument("--repositories", type=Path)
        parser.add_argument("--source-footers", action="store_true", help="Materialize local source footers for a compatible renderer")
        parser.add_argument("--source-view", type=Path, action="append", default=[], help="Reuse an existing supported source view/union")
    args = parser.parse_args(arguments[1:] if finalizing else arguments)
    existed = args.output_dir.exists() or args.output_dir.is_symlink()
    try:
        configure_engine(args.clew, args.root, args.components, args.component_set_digest, args.evidence_inputs)
        if finalizing:
            result = finalize(args.compose_root, args.manifest, args.renderer_command, args.output_dir,
                              previous=args.previous_index, repositories=args.repositories,
                              source_footers=args.source_footers, source_views=args.source_view,
                              require_process_search=args.require_process_search, require_sequences=args.require_sequences)
            code = 0 if result["status"] == "CHECKED_PAGES_FINALIZED" else 2
        else:
            result = verify(args.manifest, args.index, args.authored_root, args.rendered_root,
                            args.renderer_command, args.output_dir, require_process_search=args.require_process_search,
                            require_sequences=args.require_sequences)
            code = 0
    except (Stop, OSError, ValueError, TypeError, KeyError, IndexError) as error:
        result = {"schema": "codeclew-reader-export-check/1.0",
            "status": error.status if isinstance(error, Stop) else "INVALID_EXPORT_CHECK_INPUT",
            "details": error.details if isinstance(error, Stop) else str(error),
            "semanticTruthAssessed": False, "sourceFallbackAllowed": False}
        code = 2
    if not finalizing and not existed and args.output_dir.is_dir():
        (args.output_dir / "result.json").write_text(encode(result), encoding="utf-8")
    print(encode(result), end="")
    return code


if __name__ == "__main__":
    raise SystemExit(main())
