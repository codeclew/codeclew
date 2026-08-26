#!/usr/bin/env python3
"""End-to-end explainable-documentation acceptance smoke for Kotlin fixtures."""

from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
from typing import Any


ROOT = Path(__file__).resolve().parent.parent
FIXTURE = ROOT / "fixtures" / "kotlin-explanation"
DETAILS = ("summary", "scenario", "technical", "evidence", "compiler")
TERMINAL = {
    "READY_TO_PUBLISH",
    "READY_TO_PUBLISH_CONDITIONAL",
    "VALIDATED_CONDITIONAL",
    "FAILED",
    "WORKTREE_RECOVERY_REQUIRED",
    "CANCELLED",
}


def command(
    arguments: list[str],
    *,
    cwd: Path,
    environment: dict[str, str],
    check: bool = True,
) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        check=False,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if check and completed.returncode != 0:
        raise AssertionError(
            f"command {Path(arguments[0]).name} failed with {completed.returncode}: "
            f"{completed.stdout[-3000:]} {completed.stderr[-3000:]}"
        )
    return completed


def clew(
    arguments: list[str],
    *,
    environment: dict[str, str],
    check: bool = True,
) -> tuple[dict[str, Any], int]:
    started = time.monotonic()
    completed = command(
        [str(ROOT / "clew"), *arguments],
        cwd=ROOT,
        environment=environment,
        check=check,
    )
    elapsed_ms = round((time.monotonic() - started) * 1000)
    try:
        value = json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise AssertionError("clew stdout is not one JSON object") from error
    return value, elapsed_ms


def git(repository: Path, *arguments: str) -> str:
    completed = subprocess.run(
        ["git", *arguments],
        cwd=repository,
        check=True,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    return completed.stdout.strip()


def canonical_file(path: Path, value: object) -> None:
    path.write_text(
        json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")),
        encoding="utf-8",
    )


def prepare_repository(source: Path, destination: Path) -> None:
    shutil.copytree(source, destination)
    os.chmod(destination / "gradlew", 0o755)
    git(destination, "init", "-q", "-b", "main")
    git(destination, "config", "user.name", "Codeclew Explanation Smoke")
    git(destination, "config", "user.email", "explanation-smoke@localhost")
    git(destination, "add", ".")
    git(destination, "commit", "-q", "-m", "fixture baseline")


def open_session(repository: Path, environment: dict[str, str]) -> tuple[str, int]:
    opened, elapsed = clew(
        [
            "session",
            "open",
            "--repo",
            str(repository),
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":/main",
        ],
        environment=environment,
    )
    return str(opened["session"]["sessionId"]), elapsed


def exact_root(context: dict[str, Any], member: str, contains: str) -> str:
    matches = context["context"]["matches"]
    roots = {
        str(row["payload"]["symbolIdentity"])
        for row in matches
        if row.get("memberAlias") == member
        and isinstance(row.get("payload"), dict)
        and contains in str(row["payload"].get("symbolIdentity", ""))
        and row["payload"].get("declarationKind") == "FUNCTION"
    }
    if len(roots) != 1:
        raise AssertionError(f"expected one exact root containing {contains!r}, got {sorted(roots)}")
    return roots.pop()


def open_thread(
    product_session: str,
    outbox_session: str,
    environment: dict[str, str],
) -> str:
    opened, _ = clew(
        [
            "thread",
            "open",
            "--member",
            f"product={product_session}",
            "--member",
            f"outbox={outbox_session}",
            "--service-alias",
            "product=product-service",
            "--service-alias",
            "outbox=outbox-worker",
        ],
        environment=environment,
    )
    return str(opened["thread"]["threadId"])


def build_snapshot(
    product_session: str,
    outbox_session: str,
    environment: dict[str, str],
) -> dict[str, Any]:
    thread_id = open_thread(product_session, outbox_session, environment)
    context_arguments = [
        "thread",
        "context",
        "--thread",
        thread_id,
        "--intent",
        "explain everything that happens while saving a product with transactional outbox",
        "--term",
        "saveProduct",
        "--term",
        "ProductRepository",
        "--term",
        "OutboxRepository",
        "--term",
        "OutboxDispatcher",
        "--max-roots",
        "16",
    ]
    context, context_ms = clew(context_arguments, environment=environment)
    root = exact_root(context, "product", "ProductService.saveProduct")
    callables, callables_ms = clew(
        [
            "thread",
            "callables",
            "--thread",
            thread_id,
            "--context",
            str(context["contextId"]),
            "--task-id",
            "save-product",
            "--pair-id",
            "product-outbox",
            "--provider",
            "outbox",
            "--consumer",
            "product",
            "--term",
            root,
            "--term",
            "ProductRepository.save",
            "--term",
            "OutboxRepository.save",
        ],
        environment=environment,
    )
    flow, flow_ms = clew(
        [
            "thread",
            "flow",
            "--thread",
            thread_id,
            "--fact-set",
            str(callables["factSetId"]),
            "--pair-id",
            "product-outbox",
            "--member",
            "product",
            "--root-kind",
            "full-symbol",
            "--root",
            root,
            "--direction",
            "downstream",
            "--max-depth",
            "32",
        ],
        environment=environment,
    )
    if flow["sliceIncluded"] is True:
        binding = flow["flow"]
    else:
        assert flow.get("claimBindingIncluded") is True, (
            "fixture flow must expose a bounded exact claim-binding projection"
        )
        binding = flow["claimBinding"]
    return {
        "threadId": thread_id,
        "context": context,
        "contextArguments": context_arguments,
        "callables": callables,
        "flowResult": flow,
        "flow": binding,
        "root": root,
        "timings": {
            "contextMs": context_ms,
            "callablesMs": callables_ms,
            "flowMs": flow_ms,
        },
    }


def bind_claims(template: dict[str, Any], flow: dict[str, Any]) -> dict[str, Any]:
    nodes = {str(node["nodeId"]): node for node in flow["nodes"]}
    roots = [
        node
        for node in nodes.values()
        if template["rootContains"] in str(node["symbolIdentity"])
    ]
    if len(roots) != 1:
        raise AssertionError("claim template root selector is not unique")
    root = roots[0]

    def fact_ids(value: dict[str, Any]) -> set[str]:
        return {str(support["factId"]) for support in value.get("supportRefs", [])}

    def relevant_boundaries(
        kind: str,
        subjects: set[str],
        support_fact_ids: set[str],
    ) -> list[str]:
        selected = []
        for boundary in flow["boundaries"]:
            code = str(boundary["code"])
            if code == "VERIFY_CONTROL_FLOW_ORDER" and kind != "ORDERED_BEFORE":
                continue
            if code == "DECLARED_TOPOLOGY_HANDOFF" and kind != "COMPONENT_HANDOFF":
                continue
            if (
                str(boundary["subject"]) in subjects
                or fact_ids(boundary) & support_fact_ids
            ):
                selected.append(str(boundary["boundaryId"]))
        return sorted(selected)

    bound: list[dict[str, Any]] = []
    for item in template["claims"]:
        kind = str(item["kind"])
        if kind == "CALL_EXISTS":
            candidates = []
            for edge in flow["edges"]:
                target = nodes[str(edge["targetNodeId"])]
                if (
                    edge["sourceNodeId"] == root["nodeId"]
                    and edge["relationKind"] == "CALLS"
                    and str(item["targetContains"]) in str(target["symbolIdentity"])
                ):
                    candidates.append((edge, target))
            if len(candidates) != 1:
                observed = sorted(
                    (
                        str(edge["relationKind"]),
                        str(nodes[str(edge["targetNodeId"])]["symbolIdentity"]),
                    )
                    for edge in flow["edges"]
                    if edge["sourceNodeId"] == root["nodeId"]
                )
                raise AssertionError(
                    f"claim selector {item['targetContains']!r} matched {len(candidates)}; "
                    f"root edges were {observed}"
                )
            edge, target = candidates[0]
            predicate = {
                "kind": "CALL_EXISTS",
                "object": target["symbolIdentity"],
                "subject": root["symbolIdentity"],
            }
            support_refs = [edge["edgeId"]]
            boundary_refs = relevant_boundaries(
                kind,
                {str(root["symbolIdentity"]), str(target["symbolIdentity"])},
                fact_ids(edge),
            )
        elif kind == "NARRATIVE_SUMMARY":
            predicate = {"kind": kind, "subject": root["symbolIdentity"]}
            support_refs = [root["nodeId"]]
            boundary_refs = relevant_boundaries(
                kind,
                {str(root["symbolIdentity"])},
                fact_ids(root),
            )
            if "boundaryCode" in item:
                selected = [
                    boundary["boundaryId"]
                    for boundary in flow["boundaries"]
                    if boundary["code"] == item["boundaryCode"]
                ]
                if not selected:
                    raise AssertionError(f"missing requested boundary {item['boundaryCode']}")
                support_refs = []
                boundary_refs = sorted(set(boundary_refs) | set(selected))
        else:
            raise AssertionError(f"unsupported claim template kind {kind}")
        claim = {
            "localId": item["localId"],
            "locale": template["locale"],
            "predicate": predicate,
            "supportRefs": support_refs,
            "text": item["text"],
        }
        if boundary_refs:
            claim["boundaryRefs"] = sorted(boundary_refs)
        bound.append(claim)
    return {
        "claims": bound,
        "flowId": flow["flowId"],
        "schema": "codeclew-explanation-claim-input/0.1",
    }


def render_and_verify(
    snapshot: dict[str, Any],
    claims_path: Path,
    environment: dict[str, str],
) -> tuple[str, dict[str, Any], dict[str, int]]:
    explanation, explain_ms = clew(
        [
            "thread",
            "explain",
            "--thread",
            snapshot["threadId"],
            "--flow",
            snapshot["flow"]["flowId"],
            "--claims",
            str(claims_path),
        ],
        environment=environment,
    )
    explanation_id = str(explanation["explanationId"])
    renders: dict[str, dict[str, Any]] = {}
    render_ms: dict[str, int] = {}
    for detail in DETAILS:
        rendered, elapsed = clew(
            [
                "thread",
                "render",
                "--thread",
                snapshot["threadId"],
                "--explanation",
                explanation_id,
                "--detail",
                detail,
                "--format",
                "json",
            ],
            environment=environment,
        )
        renders[detail] = rendered
        render_ms[detail] = elapsed
    identities = {
        (value["explanationId"], value["flowId"], value["semanticDigest"])
        for value in renders.values()
    }
    assert len(identities) == 1, "semantic zoom levels changed explanation identity"
    technical = renders["technical"]
    compiler = renders["compiler"]
    authority = {claim["claimId"]: claim["authority"] for claim in technical["claims"]}
    for detail in ("evidence", "compiler"):
        assert {
            claim["claimId"]: claim["authority"] for claim in renders[detail]["claims"]
        } == authority
    texts = {claim["text"]: claim["authority"] for claim in compiler["claims"]}
    assert texts[
        "The code places product and outbox saves in one method, but atomicity is not proven without framework transaction evidence."
    ] == "UNKNOWN"
    for text in (
        "ProductService invokes the outbox repository save operation.",
        "ProductService invokes the product repository save operation.",
    ):
        assert texts[text] == "COMPILER_PROVEN"

    flow = snapshot["flow"]
    nodes = {node["nodeId"]: node for node in flow["nodes"]}
    claim_edges = [
        edge
        for edge in flow["edges"]
        if edge["relationKind"] == "CALLS"
        and any(
            selector in nodes[edge["targetNodeId"]]["symbolIdentity"]
            for selector in ("OutboxRepository.save", "ProductRepository.save")
        )
    ]
    assert len(claim_edges) == 2
    edge_fact_ids = {
        support["factId"] for edge in claim_edges for support in edge["supportRefs"]
    }
    compiler_support = compiler["compilerSupport"]
    anchored = [
        support
        for support in compiler_support
        if support["factId"] in edge_fact_ids and support.get("source")
    ]
    assert anchored, "compiler drilldown lost exact call source support"
    for support in anchored:
        source = support["source"]
        assert source["path"].endswith("ProductScenario.kt")
        assert source["start"] < source["end"]
        assert source["contentRef"]["digest"].startswith("sha256:")
    return explanation_id, renders, {"explainMs": explain_ms, **render_ms}


def source_row(context: dict[str, Any], marker: str) -> dict[str, Any]:
    rows = [
        row
        for row in context["context"]["sources"]
        if marker in str(row.get("text", ""))
    ]
    if len(rows) != 1:
        raise AssertionError(f"expected one managed source containing {marker!r}")
    return rows[0]


def managed_replace(
    repository: Path,
    old_text: str,
    new_text: str,
    operation_id: str,
    environment: dict[str, str],
    plan_path: Path,
) -> tuple[str, int]:
    opened, open_ms = clew(
        [
            "change",
            "open",
            "--repo",
            str(repository),
            "--target-ref",
            "main",
            "--language",
            "kotlin",
            "--compilation",
            ":/main",
            "--intent",
            operation_id,
            "--term",
            "saveProduct",
            "--max-roots",
            "8",
        ],
        environment=environment,
    )
    session = str(opened["session"]["sessionId"])
    context = opened["context"]
    try:
        source = source_row(context, old_text)
    except AssertionError:
        roots = {
            str(row["payload"]["symbolIdentity"])
            for row in context["context"].get("matches", [])
            if isinstance(row.get("payload"), dict)
            and "ProductService.saveProduct" in str(row["payload"].get("symbolIdentity", ""))
            and row["payload"].get("declarationKind") == "FUNCTION"
        }
        if len(roots) != 1:
            raise AssertionError("managed change could not select one exact saveProduct root")
        context, _ = clew(
            [
                "context",
                "expand",
                "--session",
                session,
                "--from",
                str(context["contextId"]),
                "--term",
                roots.pop(),
                "--max-roots",
                "1",
            ],
            environment=environment,
        )
        source = source_row(context, old_text)
    plan = {
        "operations": [
            {
                "kind": "REPLACE_TEXT",
                "newText": new_text,
                "oldText": old_text,
                "opId": operation_id,
                "target": {
                    "contentRef": source["contentRef"],
                    "fileId": source["fileId"],
                },
            }
        ],
        "schema": "codeclew-task-plan/2.0",
        "validation": [{"args": ["test", "--no-daemon", "--quiet"], "launcher": "GRADLE"}],
    }
    canonical_file(plan_path, plan)
    prepared, _ = clew(
        [
            "change",
            "prepare",
            "--session",
            session,
            "--context",
            str(context["contextId"]),
            "--plan",
            str(plan_path),
        ],
        environment=environment,
    )
    run = str(prepared["run"]["runId"])
    deadline = time.monotonic() + 240
    while True:
        status, _ = clew(["change", "status", "--run", run], environment=environment)
        run_status = str(status["run"]["status"])
        if run_status in TERMINAL:
            break
        if time.monotonic() >= deadline:
            raise AssertionError("managed fixture change did not reach a publishable state")
        time.sleep(0.2)
    assert run_status == "READY_TO_PUBLISH_CONDITIONAL", status
    candidate = status["candidate"]
    publish = [
        "change",
        "publish",
        "--session",
        session,
        "--run",
        run,
        "--allow-conditional",
        "--prepared-authority-digest",
        str(candidate["preparedAuthorityDigest"]),
    ]
    for obligation in candidate["qualifiedObligations"]:
        publish.extend(["--acknowledge-obligation", str(obligation["approvalId"])])
    published, _ = clew(publish, environment=environment)
    assert published["run"]["status"] == "PUBLISHED_CONDITIONAL"
    return session, open_ms


def freshness(
    old: dict[str, Any],
    explanation_id: str,
    against: dict[str, Any],
    environment: dict[str, str],
) -> tuple[dict[str, Any], int]:
    return clew(
        [
            "thread",
            "explanation-status",
            "--thread",
            old["threadId"],
            "--explanation",
            explanation_id,
            "--against-thread",
            against["threadId"],
            "--against-fact-set",
            str(against["callables"]["factSetId"]),
            "--against-flow",
            str(against["flow"]["flowId"]),
            "--member-correspondence",
            "product=product",
            "--member-correspondence",
            "outbox=outbox",
        ],
        environment=environment,
    )


def main() -> int:
    template = json.loads((FIXTURE / "claims.json").read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix=".codeclew-explanation-smoke.", dir=ROOT.parent) as tmp:
        temporary = Path(tmp).resolve()
        product = temporary / "product-service"
        outbox = temporary / "outbox-worker"
        state = temporary / "state"
        state.mkdir(mode=0o700)
        prepare_repository(FIXTURE / "product-service", product)
        prepare_repository(FIXTURE / "outbox-worker", outbox)
        environment = dict(os.environ)
        environment["CODECLEW_HOME"] = str(state)
        command(
            [str(product / "gradlew"), "test", "--no-daemon", "--quiet"],
            cwd=product,
            environment=environment,
        )
        command(
            [str(outbox / "gradlew"), "test", "--no-daemon", "--quiet"],
            cwd=outbox,
            environment=environment,
        )

        product_session, product_open_ms = open_session(product, environment)
        outbox_session, outbox_open_ms = open_session(outbox, environment)
        base = build_snapshot(product_session, outbox_session, environment)
        claims = bind_claims(template, base["flow"])
        claims_path = temporary / "bound-claims.json"
        canonical_file(claims_path, claims)
        explanation_id, renders, render_timings = render_and_verify(
            base, claims_path, environment
        )

        warm_context, warm_context_ms = clew(
            base["contextArguments"], environment=environment
        )
        assert warm_context["contextId"] == base["context"]["contextId"]
        assert warm_context["context"]["members"] == base["context"]["context"]["members"]
        repeated_callables, retained_callables_ms = clew(
            [
                "thread",
                "callables",
                "--thread",
                base["threadId"],
                "--context",
                str(base["context"]["contextId"]),
                "--task-id",
                "save-product",
                "--pair-id",
                "product-outbox",
                "--provider",
                "outbox",
                "--consumer",
                "product",
                "--term",
                base["root"],
                "--term",
                "ProductRepository.save",
                "--term",
                "OutboxRepository.save",
            ],
            environment=environment,
        )
        assert repeated_callables["factSetId"] == base["callables"]["factSetId"]

        offset_change_session, offset_change_open_ms = managed_replace(
            product,
            "    @Transactional\n",
            "\n    @Transactional\n",
            "offset-only-before-transaction-annotation",
            environment,
            temporary / "offset-plan.json",
        )
        offset_session, offset_session_open_ms = open_session(product, environment)
        offset = build_snapshot(offset_session, outbox_session, environment)
        offset_status, offset_freshness_ms = freshness(
            base, explanation_id, offset, environment
        )
        assert offset_status["freshness"]["status"] == "CURRENT", offset_status

        relation_change_session, relation_change_open_ms = managed_replace(
            product,
            '        outbox.save(OutboxEvent("product-saved", product.sku))\n',
            "",
            "remove-outbox-save-relation",
            environment,
            temporary / "relation-plan.json",
        )
        changed_session, changed_session_open_ms = open_session(product, environment)
        changed = build_snapshot(changed_session, outbox_session, environment)
        changed_status, changed_freshness_ms = freshness(
            base, explanation_id, changed, environment
        )
        assert changed_status["freshness"]["status"] == "PARTIALLY_STALE", changed_status
        assert changed_status["freshness"]["affectedClaims"]
        assert changed_status["freshness"]["unaffectedClaimIds"]

        print(
            json.dumps(
                {
                    "authority": {
                        claim["text"]: claim["authority"]
                        for claim in renders["compiler"]["claims"]
                    },
                    "cold": {
                        "outboxSessionOpenMs": outbox_open_ms,
                        "productSessionOpenMs": product_open_ms,
                        **base["timings"],
                    },
                    "explanationId": explanation_id,
                    "flowId": base["flow"]["flowId"],
                    "freshness": {
                        "offsetOnly": offset_status["freshness"]["status"],
                        "relationRemoved": changed_status["freshness"]["status"],
                    },
                    "managedChanges": {
                        "offsetOpenMs": offset_change_open_ms,
                        "offsetSessionOpenMs": offset_session_open_ms,
                        "relationOpenMs": relation_change_open_ms,
                        "relationSessionOpenMs": changed_session_open_ms,
                    },
                    "renderMs": render_timings,
                    "schema": "codeclew-explanation-smoke/1.0",
                    "status": "PASSED",
                    "warm": {
                        "contextMs": warm_context_ms,
                        "retainedCallablesMs": retained_callables_ms,
                        "offsetFreshnessMs": offset_freshness_ms,
                        "changedFreshnessMs": changed_freshness_ms,
                    },
                },
                ensure_ascii=False,
                sort_keys=True,
                separators=(",", ":"),
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
