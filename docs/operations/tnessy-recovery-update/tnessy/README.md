# tnessy — paired Claude + Nessy file-session skills

`tnessy` installs two complementary skills for a controlled, file-backed delegation workflow:

```text
Claude global skill                         Nessy workspace skill
/orchestrate-nessy-plan                     /execute-claude-plan
          │                                           │
          └──── plan.json → run artifacts ← report ──┘
```

Claude creates an immutable engineering plan. You explicitly run Nessy to execute it. Claude then validates Nessy’s machine-readable report before accepting the result.

The package deliberately does **not** start Nessy, make network calls, operate an A2A daemon, access secrets, or use a package manager.

## Requirements

The target Mac must have:

- `tclaude`;
- `nessy`;
- `python3` (standard library only);
- standard macOS shell tools, including `shasum`.

## Install

Clone/copy this repository to the target laptop, then run from `tnessy/`:

```bash
./install.sh install /absolute/path/to/workspace
```

The command installs:

```text
/absolute/path/to/workspace/.nessy/skills/execute-claude-plan/
~/.claude/skills/orchestrate-nessy-plan/
```

It also creates (only if needed) protocol documentation in:

```text
/absolute/path/to/workspace/.nessy/a2a-runs/
```

### Safe preview

```bash
./install.sh install /absolute/path/to/workspace --dry-run
```

### Reinstall and upgrade

If the same package version is already installed, the installer is a no-op. If a different or manually managed target exists, installation refuses with exit code `3` and changes nothing:

```text
REFUSED: target is unmanaged or has a different package version
```

Inspect the existing directory, then deliberately replace it with a recoverable timestamped backup:

```bash
./install.sh install /absolute/path/to/workspace --force
```

## Normal workflow

### 1. Create a delegated plan in Claude

Open `tclaude` anywhere and invoke the global skill:

```text
/orchestrate-nessy-plan /absolute/path/to/workspace <task description>
```

Claude investigates enough to create a bounded plan in:

```text
<workspace>/.nessy/a2a-runs/<run-id>/plan.json
```

Claude then prints a literal Nessy command. It does **not** run Nessy or make the implementation changes itself.

### 2. Explicitly execute the plan in Nessy

Start Nessy from the target workspace, then invoke:

```text
/execute-claude-plan <run-id>
```

Nessy validates the plan, executes only its bounded work, then writes:

```text
<workspace>/.nessy/a2a-runs/<run-id>/
├── plan.json       # Claude-owned, immutable
├── status.json     # Nessy-owned state snapshot
├── events.jsonl    # Nessy-owned append-only journal
└── report.json     # Nessy-owned terminal result
```

### 3. Let Claude validate the outcome

After Nessy returns its report path, invoke:

```text
/orchestrate-nessy-plan status /absolute/path/to/workspace <run-id>
```

Claude reads the artifacts, invokes the installed read-only protocol validator, compares results with the original plan, and reports one of:

- `completed`;
- `failed`;
- `input-required`;
- `canceled`.

A terminal report is never overwritten. A retry needs a **new** run ID and a new immutable plan containing `retryOfRunId`.

## Verify installation

```bash
./install.sh verify /absolute/path/to/workspace
```

This parses all installed reference JSON, compiles the installed Python validator, and confirms both skill locations and the rendered workspace boundary.

Validate a Nessy terminal run without running its plan:

```bash
python3 /absolute/path/to/workspace/.nessy/skills/execute-claude-plan/scripts/validate_run.py <run-id>
```

`VALID:` means only that the installed validator's implemented checks passed.
It is not an implementation-success signal, full JSON Schema certification, or
proof that mandatory commands and production scenarios ran. Check terminal status,
exact declared verification coverage, corresponding execution events and unmet
criteria separately. A structurally valid failed report is still failed.

## Planning and execution discipline

- Prefer small runs with named functions, existing fixtures and a first regression.
  Keep a separate mandatory native/public integration gate when local slices cannot
  prove the whole lifecycle. Correct successor prerequisites after a failed run.
- Implement and iterate before final verification. A test file the plan asks to
  create is not an external prerequisite. Do not spend a run on auxiliary process
  work and then abandon implementation merely because it is multi-file.
- Step commands support iteration; final `verificationCommands` are mandatory for
  completion. Unrun commands have no exit code. Preserve partial work and report
  concrete blockers or exhausted limits rather than inventing command results.
- Test fixtures use unique owned directories; setup must not remove a fixed existing
  run directory. Safety and permission boundaries remain unchanged.

Skill text changes do not by themselves upgrade `shared/scripts/validate_run.py`.
The reviewer must still perform the explicit plan/report/event comparison. Future
validator changes should add executable regression tests, not just stronger claims
in documentation.

## Uninstall

Default uninstall moves the managed skills into timestamped recoverable backups:

```bash
./install.sh uninstall /absolute/path/to/workspace
```

Use destructive removal only deliberately:

```bash
./install.sh uninstall /absolute/path/to/workspace --purge
```

An unmarked skill is never removed unless `--force` is specified.

**The installer never reads, modifies, or deletes any run directory under:**

```text
<workspace>/.nessy/a2a-runs/<run-id>/
```

## Protocol boundaries

Both skills prohibit, even when a plan asks for them:

- remote Git operations, PR/MR creation, review publication, and other external publication;
- deployment, package publication, database or external-service mutations;
- CI/CD and Dockerfile changes;
- access to `.env`, credentials, keys, keychains, or other secrets;
- destructive cleanup and force operations;
- network or MCP actions.

Nessy stops with `input-required` when it needs a prohibited action, unavailable permission, missing information, or a human decision. Claude surfaces that question and does not silently approve or alter the plan.

## Maintainer update procedure

1. Change files under `nessy-skill/`, `claude-skill/`, or `shared/`.
2. Bump `VERSION`.
3. Test in a disposable workspace using `--dry-run`, install, `verify`, and both skill-discovery checks.
4. Keep the executor’s `__TNESSY_WORKSPACE_ROOT__` placeholder exactly once: `install.sh` renders it to the selected target workspace.

No manifest needs regeneration in version 1.0.0; the marker uses the package version and installer static checks validate all JSON and Python source before copying.
