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
from urllib.parse import unquote, urlsplit

M = runpy.run_path(str(Path(__file__).with_name("page_citations.py")))
R = runpy.run_path(str(Path(__file__).with_name("reader_markdown_repair.py")),
                 init_globals={"CITATIONS": M["C"]})
Stop, encode, require, read, sha = (M[k] for k in ("Stop", "encode", "require", "read", "sha"))
NATIVE_ENGINE_ARGS = []


def configure_engine(clew=None, root=None, components=None, component_set_digest=None):
    global NATIVE_ENGINE_ARGS
    M["configure_engine"](clew, root, components, component_set_digest)
    NATIVE_ENGINE_ARGS = [] if clew is None else ["--clew", str(clew), "--root", str(root)]
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
            require(path.is_file() and not path.is_symlink() and path.suffix == suffix,
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
        self.targets, self.links = set(), []

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
                self.links.append(values[key])
        require("srcset" not in values, "UNSUPPORTED_EXPORT_ASSET")


def check_links(pages):
    documents = {}
    for name, raw in pages.items():
        document = Links(); document.feed(raw.decode("utf-8")); document.close()
        documents[name] = document
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


def verify(manifests, index_path, authored, exported, renderer, output):
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
    for name, body in bound.items():
        require(read(source, name) == body and read(authored, name) == outer[name], "EXPORT_INPUT_CHANGED", page=name)
    for name, raw in actual.items():
        require(read(expected, name) == raw, "EXPORTED_HTML_MISMATCH", page=name)
        require(read(exported, name) == raw, "EXPORT_INPUT_CHANGED", page=name)
    require(inventory(authored, ".md") == names and
            inventory(exported, ".html", only_pages=True) == expected_names and
            read(index_path.parent, index_path.name) == index_raw and
            Path(renderer).read_bytes() == command_raw, "EXPORT_INPUT_CHANGED")
    links = check_links(actual)
    return {"schema": "codeclew-reader-export-check/1.0", "status": "EXPORT_CHECKED",
        "pages": len(names), "claims": native["claims"], "internalLinks": links,
        "nativeCheck": "native-check.json", "indexDigest": index["indexDigest"],
        "rendererCommandDigest": sha(command_raw), "rendererReceipt": receipt,
        "authoredDigests": {name: sha(raw) for name, raw in outer.items()},
        "exportDigests": {name: sha(raw) for name, raw in actual.items()},
        "semanticTruthAssessed": False, "runtimeVerified": False,
        "nativePublicationApproval": False, "sourceFallbackAllowed": False,
        "limits": "Exact full HTML bytes from the explicit trusted renderer over complete checked Markdown. Flat .md/.html pages only; local assets, nested/root-relative links, base URLs and srcset unsupported. External URL availability, renderer faithfulness, CSS/browser visibility and semantic completeness unassessed. Renderer must be deterministic and independent of input/output locations; keep inputs stable during check. Caller-trusted argv is not sandboxed."}


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


def finalize(compose_root, manifests, renderer, output, *, previous=None, repositories=None):
    """Complete reviewed pages using existing bind, renderer and full check."""
    compose_root, renderer, output = map(Path, (compose_root, renderer, output))
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
        renderer_raw = read(renderer.parent, renderer.name)
        argv = R["renderer_command"](renderer)
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
        stage = "render"
        try:
            R["run_renderer"](argv, index.parent / "markdown", output / state["renderedRoot"], receipts, stage)
        finally:
            for suffix in ("stdout", "stderr"):
                path = receipts / (stage + ".render." + suffix)
                if not path.exists(): path.write_bytes(b"")
        require(read(output, state["rendererCommand"]) == renderer_raw, "EXPORT_INPUT_CHANGED")
        stage = "export-check"
        checked = checked_command([sys.executable, "-I", "-S", str(scripts / "reader_export_check.py"),
            *manifest_args, *NATIVE_ENGINE_ARGS, "--index", str(index), "--authored-root", str(authored),
            "--rendered-root", str(output / state["renderedRoot"]), "--renderer-command", str(command_path),
            "--output-dir", str(output / "check")], receipts, stage, "EXPORT_CHECKED")
        require(M["json_file"](output / state["exportCheck"]) == checked, "FINALIZATION_CHECK_RESULT_MISMATCH")
        # The explicit trusted renderer must not silently change the reviewed
        # original/outer inputs while producing its first render.
        require(read(compose_root, "original.json", M["MAX_MAP"]) == original
                and read(compose_root, "request.json") == request and read(renderer.parent, renderer.name) == renderer_raw
                and read(output / state["composeRoot"], "original.json", M["MAX_MAP"]) == original
                and read(output / state["composeRoot"], "request.json") == request
                and read(output, state["rendererCommand"]) == renderer_raw
                and M["json_file"](authored / "sources.json") == pointer
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
    parser.add_argument("--components", type=Path, help="Mandatory pre-author component ticket")
    parser.add_argument("--component-set-digest", help="Exact independently selected ticket ID")
    for name in (("compose-root", "renderer-command", "output-dir") if finalizing else
                 ("index", "authored-root", "rendered-root", "renderer-command", "output-dir")):
        parser.add_argument("--" + name, type=Path, required=True)
    if finalizing:
        parser.add_argument("--previous-index", type=Path)
        parser.add_argument("--repositories", type=Path)
    args = parser.parse_args(arguments[1:] if finalizing else arguments)
    existed = args.output_dir.exists() or args.output_dir.is_symlink()
    try:
        configure_engine(args.clew, args.root, args.components, args.component_set_digest)
        if finalizing:
            result = finalize(args.compose_root, args.manifest, args.renderer_command, args.output_dir,
                              previous=args.previous_index, repositories=args.repositories)
            code = 0 if result["status"] == "CHECKED_PAGES_FINALIZED" else 2
        else:
            result = verify(args.manifest, args.index, args.authored_root, args.rendered_root,
                            args.renderer_command, args.output_dir)
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
