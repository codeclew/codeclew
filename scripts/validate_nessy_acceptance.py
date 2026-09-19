#!/usr/bin/env python3
"""Strict, source-owned supplemental acceptance checker for Nessy runs.

Validates a file-backed Claude <-> Nessy run (plan/status/report/events) against
the installed execute-claude-plan reference schemas, then applies cross-artifact
semantic acceptance checks that the minimal installed validator does not cover:

* full schema conformance (types, required/extra, enums, consts, lengths,
  patterns, integer bounds, RFC3339 timestamps, $ref/$defs, if/then/else);
  any schema keyword outside the supported set fails the run;
* safe run path, matching runId across every artifact;
* terminal state consistency across status/report/events;
* exact ordered report step IDs matching the plan;
* per-step unmet criteria empty on completed;
* every declared plan verification command appears in the report verification
  with a numeric exit code and a nonempty output summary, and command events
  correspond to those verification entries / step commands;
* report filesInspected/filesChanged present per step and top-level
  filesChanged present, with no empty output summaries.

This is a read-only supplemental checker. It never rewrites run artifacts and it
retains the installed validator's core checks.
"""

from __future__ import annotations

import datetime as _dt
import json
import re
import sys
from pathlib import Path
from typing import Any

RUN_ID_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,63}$")
TERMINAL_STATUSES = {"completed", "failed", "input-required", "canceled"}
STEP_STATUSES = {"completed", "failed", "skipped"}

# The schema keywords this validator understands. Any other keyword used by an
# installed schema (or a fixture) is reported as unsupported rather than
# silently ignored.
SUPPORTED_KEYWORDS = {
    "$schema",
    "$ref",
    "$defs",
    "title",
    "description",
    "type",
    "properties",
    "required",
    "additionalProperties",
    "items",
    "enum",
    "const",
    "pattern",
    "minLength",
    "maxLength",
    "minItems",
    "maxItems",
    "minimum",
    "maximum",
    "format",
    "allOf",
    "oneOf",
    "if",
    "then",
    "else",
}


class ValidationError(Exception):
    pass


class SchemaFailure(Exception):
    pass


def errors() -> list[str]:
    return []


class MiniSchema:
    """Hand-rolled JSON Schema validator for the keywords the run schemas use."""

    def __init__(self, schema: dict[str, Any], errors: list[str]) -> None:
        self.schema = schema
        self.errors = errors
        self.unsupported: list[str] = []

    def _check_keywords(self, node: dict[str, Any], where: str) -> None:
        # Unknown schema keywords are a syntax error regardless of the instance
        # branch that ends up matching. Raising here propagates from conditional
        # and oneOf/allOf sub-validators without relying on instance branches.
        for key in node:
            if key not in SUPPORTED_KEYWORDS:
                raise SchemaFailure(f"{where}: unsupported schema keyword '{key}'")

    def validate(
        self, instance: Any, node: dict[str, Any], path: str, defs: dict[str, Any]
    ) -> None:
        self._check_keywords(node, path or "$")
        if "$ref" in node:
            ref = node["$ref"]
            if not ref.startswith("#/$defs/"):
                raise SchemaFailure(f"{path}: unsupported $ref '{ref}'")
            target = defs.get(ref[len("#/$defs/") :])
            if target is None:
                raise SchemaFailure(f"{path}: unresolved $ref '{ref}'")
            self.validate(instance, target, path, defs)
            return
        if "$defs" in node:
            for name, sub in node["$defs"].items():
                self._check_keywords(sub, f"{path}.$defs.{name}")
        node_type = node.get("type")
        if isinstance(node_type, list):
            for single in node_type:
                if _instance_matches_type(instance, single):
                    node_type = single
                    break
            else:
                self.errors.append(f"{path}: value does not match any of types {node_type}")
                return
        if node_type == "object":
            self._validate_object(instance, node, path, defs)
        elif node_type == "array":
            self._validate_array(instance, node, path, defs)
        elif node_type == "string":
            self._validate_string(instance, node, path)
        elif node_type == "integer":
            self._validate_integer(instance, node, path)
        elif node_type == "boolean":
            if not isinstance(instance, bool):
                self.errors.append(f"{path}: expected boolean, got {type(instance).__name__}")
        elif node_type == "null":
            if instance is not None:
                self.errors.append(f"{path}: expected null")
        elif node_type is None and isinstance(instance, dict) and any(
            key in node for key in ("properties", "required", "additionalProperties")
        ):
            # The installed event.schema.json uses object assertions inside
            # conditionals without an explicit "type". Evaluate properties and
            # required on dict instances even when the type keyword is omitted.
            self._validate_object(instance, node, path, defs)
        elif node_type is not None:
            raise SchemaFailure(f"{path}: unsupported type '{node_type}'")
        if "enum" in node and instance not in node["enum"]:
            self.errors.append(f"{path}: value {instance!r} not in enum {node['enum']}")
        if "const" in node and instance != node["const"]:
            self.errors.append(f"{path}: value {instance!r} != const {node['const']!r}")
        if "allOf" in node:
            for index, sub in enumerate(node["allOf"]):
                self.validate(instance, sub, f"{path}.allOf[{index}]", defs)
        if "oneOf" in node:
            matches = 0
            for index, sub in enumerate(node["oneOf"]):
                probe: list[str] = []
                sub_validator = MiniSchema(sub, probe)
                try:
                    sub_validator.validate(instance, sub, f"{path}.oneOf[{index}]", defs)
                except SchemaFailure:
                    raise
                if not probe:
                    matches += 1
            if matches != 1:
                self.errors.append(f"{path}: oneOf must match exactly one variant ({matches})")
        if "if" in node:
            head: list[str] = []
            head_v = MiniSchema(node["if"], head)
            head_v.validate(instance, node["if"], f"{path}.if", defs)
            condition_passed = not head
            branch = node.get("then" if condition_passed else "else", {})
            if branch:
                self.validate(instance, branch, f"{path}.branch", defs)

    def _validate_object(
        self, instance: Any, node: dict[str, Any], path: str, defs: dict[str, Any]
    ) -> None:
        if not isinstance(instance, dict):
            self.errors.append(f"{path}: expected object, got {type(instance).__name__}")
            return
        properties = node.get("properties", {})
        for key, sub in properties.items():
            if key in instance:
                self.validate(instance[key], sub, f"{path}.{key}", defs)
            elif key in node.get("required", []):
                self.errors.append(f"{path}: missing required property '{key}'")
        if node.get("additionalProperties") is False:
            allowed = set(properties)
            for key in instance:
                if key not in allowed:
                    self.errors.append(f"{path}: unexpected property '{key}'")
        for key in node.get("required", []):
            if key not in instance:
                self.errors.append(f"{path}: missing required property '{key}'")

    def _validate_array(
        self, instance: Any, node: dict[str, Any], path: str, defs: dict[str, Any]
    ) -> None:
        if not isinstance(instance, list):
            self.errors.append(f"{path}: expected array, got {type(instance).__name__}")
            return
        min_items = node.get("minItems")
        max_items = node.get("maxItems")
        if min_items is not None and len(instance) < min_items:
            self.errors.append(f"{path}: fewer than {min_items} items")
        if max_items is not None and len(instance) > max_items:
            self.errors.append(f"{path}: more than {max_items} items")
        items = node.get("items")
        if items:
            for index, item in enumerate(instance):
                self.validate(item, items, f"{path}[{index}]", defs)

    def _validate_string(self, instance: Any, node: dict[str, Any], path: str) -> None:
        if not isinstance(instance, str):
            self.errors.append(f"{path}: expected string, got {type(instance).__name__}")
            return
        min_length = node.get("minLength")
        max_length = node.get("maxLength")
        if min_length is not None and len(instance) < min_length:
            self.errors.append(f"{path}: string shorter than {min_length}")
        if max_length is not None and len(instance) > max_length:
            self.errors.append(f"{path}: string longer than {max_length}")
        pattern = node.get("pattern")
        if pattern is not None and not re.search(pattern, instance):
            self.errors.append(f"{path}: string does not match pattern {pattern}")
        fmt = node.get("format")
        if fmt == "date-time":
            if not _valid_rfc3339(instance):
                self.errors.append(f"{path}: '{instance}' is not a valid RFC3339 timestamp")
        elif fmt is not None:
            raise SchemaFailure(f"{path}: unsupported format '{fmt}'")

    def _validate_integer(self, instance: Any, node: dict[str, Any], path: str) -> None:
        if isinstance(instance, bool) or not isinstance(instance, int):
            self.errors.append(f"{path}: expected integer, got {type(instance).__name__}")
            return
        minimum = node.get("minimum")
        maximum = node.get("maximum")
        if minimum is not None and instance < minimum:
            self.errors.append(f"{path}: {instance} < minimum {minimum}")
        if maximum is not None and instance > maximum:
            self.errors.append(f"{path}: {instance} > maximum {maximum}")

    def finish(self) -> None:
        # Unsupported schema keywords are raised immediately by _check_keywords,
        # so finish() is a no-op retained for API compatibility.
        return


def _instance_matches_type(instance: Any, name: str) -> bool:
    if name == "object":
        return isinstance(instance, dict)
    if name == "array":
        return isinstance(instance, list)
    if name == "string":
        return isinstance(instance, str)
    if name == "integer":
        return isinstance(instance, int) and not isinstance(instance, bool)
    if name == "boolean":
        return isinstance(instance, bool)
    if name == "null":
        return instance is None
    return False


def _valid_rfc3339(value: str) -> bool:
    try:
        parsed = _dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError:
        return False
    return parsed.tzinfo is not None


def load_json(path: Path, errors: list[str]) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        errors.append(f"missing required artifact: {path.name}")
        return None
    except (OSError, json.JSONDecodeError) as exc:
        errors.append(f"invalid JSON in {path.name}: {exc}")
        return None
    if not isinstance(value, dict):
        errors.append(f"{path.name} must contain a JSON object")
        return None
    return value


def resolve_run(argument: str) -> Path:
    nessy_root = Path(__file__).resolve().parents[1] / ".nessy"
    runs_root = (nessy_root / "a2a-runs").resolve()
    candidate = Path(argument)
    if candidate.is_absolute() or "/" in argument or argument in {".", ".."}:
        run_dir = candidate.resolve()
    else:
        if not RUN_ID_RE.fullmatch(argument):
            raise ValueError("run ID must match ^[a-z0-9][a-z0-9-]{0,63}$")
        run_dir = (runs_root / argument).resolve()
    try:
        run_dir.relative_to(runs_root)
    except ValueError as exc:
        raise ValueError("run directory must be inside .nessy/a2a-runs") from exc
    if run_dir == runs_root:
        raise ValueError("run directory must not be .nessy/a2a-runs itself")
    return run_dir


def schema_root() -> Path:
    return Path(__file__).resolve().parents[1] / ".nessy" / "skills" / "execute-claude-plan" / "references"


def validate_schema(artifact: dict[str, Any], schema: dict[str, Any], name: str, errors: list[str]) -> None:
    validator = MiniSchema(schema, errors)
    try:
        validator.validate(artifact, schema, "$", schema.get("$defs", {}))
        validator.finish()
    except SchemaFailure as exc:
        errors.append(f"{name}: schema validation aborted: {exc}")


def validate_run_path(run_dir: Path, errors: list[str]) -> None:
    runs_root = Path(__file__).resolve().parents[1] / ".nessy" / "a2a-runs"
    try:
        run_dir.resolve().relative_to(runs_root.resolve())
    except ValueError:
        errors.append(f"run directory escapes .nessy/a2a-runs: {run_dir}")


def validate_cross(
    plan: dict[str, Any],
    status: dict[str, Any],
    report: dict[str, Any],
    events: list[dict[str, Any]],
    errors: list[str],
) -> None:
    run_id = plan.get("runId")
    if not isinstance(run_id, str) or not RUN_ID_RE.fullmatch(run_id):
        errors.append("plan.runId must match ^[a-z0-9][a-z0-9-]{0,63}$")
        return
    for name, artifact in (("status", status), ("report", report)):
        if artifact.get("runId") != run_id:
            errors.append(f"{name}.runId does not match plan.runId")
    if status.get("status") != report.get("status"):
        errors.append("status.status must match report.status")
    # Event run IDs must match the plan.
    for index, event in enumerate(events):
        if event.get("runId") != run_id:
            errors.append(f"events[{index}] runId does not match plan.runId")
    # Balanced, ordered lifecycle: started first, one terminal event last, and
    # ordered step-start/step-end and (if present) verification-start/end.
    if not events:
        errors.append("events must not be empty")
        return
    if events[0].get("event") != "started":
        errors.append("first event must be started")
    terminal = [i for i, e in enumerate(events) if e.get("event") in TERMINAL_STATUSES]
    if len(terminal) != 1:
        errors.append("events must have exactly one terminal event")
        return
    if terminal[0] != len(events) - 1:
        errors.append("terminal event must be last")
    if events[terminal[0]].get("event") != report.get("status"):
        errors.append("terminal event must match report.status")
    step_starts = [e for e in events if e.get("event") == "step-start"]
    step_ends = [e for e in events if e.get("event") == "step-end"]
    if len(step_starts) != len(step_ends):
        errors.append("step-start and step-end counts must be balanced")
    else:
        for pair in zip(step_starts, step_ends):
            if pair[0].get("step") != pair[1].get("step"):
                errors.append("step-start/step-end are not balanced in order")
                break
    verification_starts = [i for i, e in enumerate(events) if e.get("event") == "verification-start"]
    verification_ends = [i for i, e in enumerate(events) if e.get("event") == "verification-end"]
    if bool(verification_starts) != bool(verification_ends):
        errors.append("verification-start and verification-end must both be present or both absent")
    elif verification_starts and (
        verification_starts[0] > verification_ends[0] or verification_ends[0] > terminal[0]
    ):
        errors.append("verification lifecycle must be ordered before the terminal event")
    # Exact ordered step IDs.
    plan_steps = [s.get("id") for s in plan.get("steps", []) if isinstance(s, dict)]
    report_steps = [s.get("id") for s in report.get("steps", []) if isinstance(s, dict)]
    if report_steps != plan_steps:
        errors.append("report step IDs and order must exactly match plan step IDs")
    # Per-step unmet criteria + files fields (completed only requires nonempty
    # filesInspected; failed steps may legitimately have inspected nothing).
    if report.get("status") == "completed":
        for step in report.get("steps", []):
            if step.get("status") != "completed":
                errors.append(f"completed report has non-completed step {step.get('id')}")
            if step.get("unmetCriteria"):
                errors.append(f"completed step {step.get('id')} has unmet criteria")
            if not isinstance(step.get("filesInspected"), list) or not step.get("filesInspected"):
                errors.append(f"step {step.get('id')} missing nonempty filesInspected")
            if not isinstance(step.get("filesChanged"), list):
                errors.append(f"step {step.get('id')} missing filesChanged")
            if not isinstance(step.get("summary"), str) or not step.get("summary"):
                errors.append(f"step {step.get('id')} missing summary")
        if not isinstance(report.get("filesChanged"), list):
            errors.append("top-level filesChanged must be an array")
        if report.get("unmetAcceptanceCriteria"):
            errors.append("completed report has unmet acceptance criteria")
        if report.get("blockingQuestion") is not None:
            errors.append("completed report must have blockingQuestion: null")
    # Verification command coverage. A completed report must carry the exact
    # ordered declared verification list with all zero exit codes. A failed or
    # input-required report may truthfully list only the commands actually
    # executed; their correspondence is still validated, without inventing
    # missing results.
    declared = plan.get("verificationCommands") or []
    verification = report.get("verification") or []
    actual = [v.get("command") for v in verification]
    if report.get("status") == "completed":
        if actual != declared:
            errors.append(
                "completed report verification commands must exactly match plan.verificationCommands in order"
            )
        for v in verification:
            if not isinstance(v.get("exitCode"), int) or isinstance(v.get("exitCode"), bool):
                errors.append("verification entry missing integer exitCode")
            elif v.get("exitCode") != 0:
                errors.append(
                    f"verification entry exitCode {v.get('exitCode')} must be 0 on completed"
                )
            if not isinstance(v.get("outputSummary"), str) or not v.get("outputSummary"):
                errors.append("verification entry missing nonempty outputSummary")
    else:
        for command in actual:
            if command not in declared:
                errors.append(f"verification entry not declared in plan: {command}")
        for v in verification:
            if not isinstance(v.get("exitCode"), int) or isinstance(v.get("exitCode"), bool):
                errors.append("verification entry missing integer exitCode")
            if not isinstance(v.get("outputSummary"), str) or not v.get("outputSummary"):
                errors.append("verification entry missing nonempty outputSummary")
    # Event/exit-code correspondence: each present verification entry must have a
    # matching phase=verification command event with the same exit code.
    command_events = [e for e in events if e.get("event") == "command"]
    for v in verification:
        matches = [
            e
            for e in command_events
            if e.get("phase") == "verification" and e.get("command") == v.get("command")
        ]
        if not matches or matches[-1].get("exitCode") != v.get("exitCode"):
            errors.append(
                f"verification command event/exit-code mismatch for: {v.get('command')}"
            )


def load_events(path: Path, errors: list[str]) -> list[dict[str, Any]]:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except FileNotFoundError:
        errors.append("missing required artifact: events.jsonl")
        return []
    events: list[dict[str, Any]] = []
    for number, line in enumerate(lines, 1):
        try:
            event = json.loads(line)
        except json.JSONDecodeError as exc:
            errors.append(f"events.jsonl line {number} is invalid JSON: {exc}")
            continue
        if not isinstance(event, dict):
            errors.append(f"events.jsonl line {number} must be a JSON object")
            continue
        events.append(event)
    if not events:
        errors.append("events.jsonl is empty")
    return events


def parse_args(args: list[str]) -> tuple[str | None, bool]:
    """Accept a positional run ID/path, a --run <id> alias, and an explicit
    --require-completed gate. Returns (run_argument, require_completed)."""
    run_argument: str | None = None
    require_completed = False
    index = 0
    while index < len(args):
        token = args[index]
        if token == "--require-completed":
            require_completed = True
        elif token == "--run":
            index += 1
            if index >= len(args):
                raise ValueError("--run requires an argument")
            if run_argument is not None:
                raise ValueError("multiple run arguments")
            run_argument = args[index]
        elif token.startswith("--"):
            raise ValueError(f"unknown option: {token}")
        else:
            if run_argument is not None:
                raise ValueError("multiple run arguments")
            run_argument = token
        index += 1
    if run_argument is None:
        raise ValueError("a run ID or path is required")
    return run_argument, require_completed


def main() -> int:
    try:
        run_argument, require_completed = parse_args(sys.argv[1:])
    except ValueError as exc:
        print(
            f"usage: {Path(sys.argv[0]).name} <run-id-or-path> [--run <id>] [--require-completed]",
            file=sys.stderr,
        )
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2
    try:
        run_dir = resolve_run(run_argument)
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2
    errors: list[str] = []
    validate_run_path(run_dir, errors)
    plan = load_json(run_dir / "plan.json", errors)
    status = load_json(run_dir / "status.json", errors)
    report = load_json(run_dir / "report.json", errors)
    if plan is None or status is None or report is None:
        for item in errors:
            print(f"ERROR: {item}", file=sys.stderr)
        return 1
    events = load_events(run_dir / "events.jsonl", errors)
    for name in ("plan", "status", "report"):
        schema = load_json(schema_root() / f"{name}.schema.json", errors)
        if schema is not None:
            validate_schema({"plan": plan, "status": status, "report": report}[name], schema, name, errors)
    schema = load_json(schema_root() / "event.schema.json", errors)
    if schema is not None:
        for index, event in enumerate(events):
            validate_schema(event, schema, f"events[{index}]", errors)
    validate_cross(plan, status, report, events, errors)
    if errors:
        for item in errors:
            print(f"ERROR: {item}", file=sys.stderr)
        return 1
    if require_completed and report.get("status") != "completed":
        print(
            f"NOT COMPLETED: {run_dir} status={report.get('status')}",
            file=sys.stderr,
        )
        return 3
    print(f"VALID: {run_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())