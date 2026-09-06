#!/usr/bin/env python3
"""Verify pinned source bindings and render the nav-query documentation graph.

The agent authors claims from retained Codeclew evidence. This script checks
their byte bindings, not their semantic truth. It never reads private CAS state.
Graphviz is needed only to regenerate the checked-in SVG; --check needs Python.
"""

import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import subprocess
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parents[1]
DATA = ROOT / "site/evidence/nav-query.json"
START = "<!-- NAV_QUERY_GRAPH -->"
END = "<!-- /NAV_QUERY_GRAPH -->"


def verify(data):
    revision = data["repositoryRevision"]
    assert re.fullmatch(r"[a-f0-9]{40}", revision), "expected an immutable Git revision"
    claims = {claim["id"]: claim for claim in data["claims"]}
    assert len(claims) == len(data["claims"]), "duplicate claim IDs"
    blobs = {}
    for claim in claims.values():
        assert claim["narrativeAuthority"] == "AGENT_INFERRED"
        assert claim["evidence"], f"missing evidence: {claim['id']}"
        for source in claim["evidence"]:
            path = source["file"]
            assert not Path(path).is_absolute() and ".." not in Path(path).parts
            if path not in blobs:
                blobs[path] = subprocess.check_output(
                    ["git", "show", f"{revision}:{path}"], cwd=ROOT
                )
            blob = blobs[path]
            assert "sha256:" + hashlib.sha256(b"codeclew-cas/v2\0" + b"codeclew-repository-input-blob/2.0\0" + blob).hexdigest() == source["fileDigest"]
            lines = blob.decode("utf-8").splitlines()
            start, end = source["startLine"], source["endLine"]
            assert 1 <= start <= end <= len(lines)
            text = "\n".join(lines[start - 1:end])
            assert text == source["text"], f"source mismatch: {claim['id']} {path}:{start}"
            assert "sha256:" + hashlib.sha256(text.encode()).hexdigest() == source["textDigest"]
            assert source["authority"] == "EXACT_SNAPSHOT_TEXT"
            assert source["url"] == f"{data['repository']}/blob/{revision}/{path}#L{start}-L{end}"
            assert re.fullmatch(r"sha256:[a-f0-9]{64}", source["evidenceDigest"])
    nodes = {node["id"] for node in data["diagram"]["nodes"]}
    assert nodes <= claims.keys()
    for edge in data["diagram"]["edges"]:
        assert edge["from"] in nodes and edge["to"] in nodes
        assert edge["claimId"] in claims
        assert edge["authority"] == "AGENT_INFERRED_STATIC_FLOW"
    return claims


def diagram_sources(data):
    quote = json.dumps
    dot = [
        "digraph nav_query {",
        'graph [bgcolor="transparent", rankdir=TB, pad="0.3", nodesep="0.4", ranksep="0.48"];',
        'node [shape=box, style="rounded,filled", fillcolor="#151b16", color="#52654b", fontcolor="#f2f4ef", fontname="Arial", fontsize=15, margin="0.2,0.16"];',
        'edge [color="#77876c", fontcolor="#bdc8b6", fontname="Arial", fontsize=11, arrowsize=0.65, style=dashed];',
    ]
    mermaid = ["flowchart TD", "  %% Agent-interpreted static flow; no resolved Rust call graph."]
    for node in data["diagram"]["nodes"]:
        id = node["id"]
        shape = "diamond" if id == "decision" else "box"
        color = "#e0ae65" if id in {"abstain", "failure"} else "#91b774"
        dot.append(f'{id} [label={quote(node["label"])}, shape={shape}, color="{color}", id="node-{id}", URL="https://codeclew.github.io/codeclew/nav-query.html#claim-{id}", tooltip={quote(id)}];')
        label = node["label"].replace("\n", "<br/>")
        mermaid.append(f'  {id}["{label}"]')
    dot.append("{rank=same; supported; abstain;}")
    for index, edge in enumerate(data["diagram"]["edges"]):
        dot.append(f'{edge["from"]} -> {edge["to"]} [label={quote(edge["label"])}, id="edge-{index}", URL="https://codeclew.github.io/codeclew/nav-query.html#claim-{edge["claimId"]}", tooltip={quote(edge["authority"])}];')
        mermaid.append(f'  {edge["from"]} -. "{edge["label"]} · claim:{edge["claimId"]}" .-> {edge["to"]}')
    dot.append("}")
    return "\n".join(dot) + "\n", "\n".join(mermaid) + "\n"


def render_svg(dot, claims):
    raw = subprocess.check_output(["dot", "-Tsvg"], input=dot.encode())
    ET.register_namespace("", "http://www.w3.org/2000/svg")
    ET.register_namespace("xlink", "http://www.w3.org/1999/xlink")
    svg = ET.fromstring(raw)
    svg.attrib.pop("width", None)
    svg.attrib.pop("height", None)
    svg.set("aria-labelledby", "flow-title flow-description")
    ns = "{http://www.w3.org/2000/svg}"
    title = ET.Element(ns + "title", id="flow-title")
    title.text = "How nav query moves from a request to bounded source evidence"
    description = ET.Element(ns + "desc", id="flow-description")
    description.text = "Select a step or arrow to inspect its supporting code. Task readiness precedes context creation. The decision branches into SUPPORTED or ABSTAIN. Errors leave the main path. Dashed arrows are agent-interpreted static flow, not runtime observations."
    svg.insert(0, description)
    svg.insert(0, title)
    for link in svg.iter(ns + "a"):
        href = link.get("{http://www.w3.org/1999/xlink}href", "")
        id = href.rsplit("#claim-", 1)[-1]
        assert id in claims
        link.set("data-claim", id)
        link.set("aria-label", claims[id]["title"] + ": inspect source evidence")
        link.set("tabindex", "0")
    return ET.tostring(svg, encoding="unicode")


def render_claims(claims):
    escape = html.escape
    articles = []
    for claim in claims.values():
        sources = []
        for source in claim["evidence"]:
            label = f"{source['file']}:{source['startLine']}–{source['endLine']}"
            sources.append(
                '<div class="source-record">'
                f'<a href="{escape(source["url"])}">{escape(label)} ↗</a>'
                '<p class="source-authority">EXACT SNAPSHOT TEXT · RETRIEVED BY CODECLEW</p>'
                f'<pre><code>{escape(source["text"])}</code></pre>'
                '<details><summary>Digests and evidence binding</summary>'
                f'<p>Fragment: {escape(source["textDigest"])}<br>'
                f'File: {escape(source["fileDigest"])}<br>'
                f'Context: {escape(source["contextId"])}<br>'
                f'Evidence: {escape(source["evidenceDigest"])}</p></details></div>'
            )
        articles.append(
            f'<article class="claim-panel" id="claim-{claim["id"]}">'
            f'<p class="claim-id">claim:{claim["id"]}</p>'
            f'<h2>{escape(claim["title"])}</h2><p>{escape(claim["summary"])}</p>'
            f'<p class="mechanism">{escape(claim["mechanism"])}</p>'
            '<div class="claim-boundary"><b>Evidence boundary</b>'
            f'<p>{escape(claim["boundary"])}</p></div>'
            '<details class="claim-evidence"><summary>Inspect supporting code</summary>'
            + "".join(sources) + '</details></article>'
        )
    return "\n".join(articles)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    data = json.loads(DATA.read_text())
    claims = verify(data)
    dot, mermaid = diagram_sources(data)
    directory = ROOT / "site/diagrams"
    page = ROOT / "site/nav-query.html"
    if args.check:
        assert (directory / "nav-query.dot").read_text() == dot
        assert (directory / "nav-query.mmd").read_text() == mermaid
        svg = (directory / "nav-query.svg").read_text()
        ET.fromstring(svg)
        assert svg in page.read_text(), "page diagram is stale"
        assert render_claims(claims) in page.read_text(), "page claims are stale"
        print(f"PASS: {len(claims)} claims, pinned source digests and rendered graph bindings")
        return
    directory.mkdir(exist_ok=True)
    svg = render_svg(dot, claims)
    (directory / "nav-query.dot").write_text(dot)
    (directory / "nav-query.mmd").write_text(mermaid)
    (directory / "nav-query.svg").write_text(svg)
    content = page.read_text()
    prefix, remainder = content.split(START, 1)
    _, suffix = remainder.split(END, 1)
    content = prefix + START + "\n" + svg + "\n" + END + suffix
    claim_start, claim_end = "<!-- NAV_QUERY_CLAIMS -->", "<!-- /NAV_QUERY_CLAIMS -->"
    prefix, remainder = content.split(claim_start, 1)
    _, suffix = remainder.split(claim_end, 1)
    page.write_text(prefix + claim_start + "\n" + render_claims(claims) + "\n" + claim_end + suffix)
    print(f"Rendered {len(data['diagram']['nodes'])} nodes and {len(data['diagram']['edges'])} evidence-bound arrows")


if __name__ == "__main__":
    main()
