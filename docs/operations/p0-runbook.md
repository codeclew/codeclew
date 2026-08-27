# Codeclew P0: Installation and Operations

This document describes Codeclew's minimum production-ready operating model:
installing the distribution on a developer machine, connecting Codex or Claude,
handling changes in target repositories safely, and collecting investigation
material without disclosing source code.

P0 is a pilot operating model, not a general-availability commitment. Use the
installed `clew` command for the public macOS release and `./clew` from a pinned
checkout for source development. Running the binary directly from a runtime
capsule and editing `CODECLEW_HOME` contents manually are unsupported.

## 1. Supported scope

The machine-readable source of truth is `crates/clew/support-matrix.json` and is
returned by:

```bash
./clew capabilities
```

P0 support:

| Profile | Read | Change and publish |
|---|---:|---:|
| Kotlin 2.4.10, Gradle wrapper, one compilation, `PROJECT_NATIVE` | yes, K2 | yes, pilot |
| Kotlin 2.3.0, Maven, optional `kotlin23` pack | yes, preview | no |
| Python, Tree-sitter syntax | yes | yes, conditional pilot |
| Rust, bounded syntax | yes | yes, conditional pilot |
| Thread of 2–8 repositories | yes | no |

Python and Rust provide syntactic facts, not proven dynamic semantics. Their
changes require successful project-native validation plus explicit
acknowledgement of every reported obligation, and remain `UNSURE`. A
multi-repository thread does not turn declared topology into a proven
relationship and cannot be used as the source of a change plan.

Rust/Python pilot admission is reproduced by
`python3 -I -S scripts/language_mutation_pilot.py`. It requires 3/3 independent
cases per language; passing does not upgrade syntax evidence beyond `UNSURE`.

## 2. Installing on another machine

### Public macOS pilot

Install on Apple Silicon or Intel Mac with one command:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
```

The installer detects the architecture, downloads a prebuilt bundle from the
latest GitHub Release, verifies the published SHA-256 checksum, extracts the
archive while rejecting symlink and path-traversal entries, and atomically
updates the launcher. Codeclew is not compiled on the user's machine.

Files are installed to these locations by default:

- `~/.local/share/codeclew/releases/<version>-macos-<arch>-<profile>`;
- `~/.local/bin/clew`, an atomic link to the selected release.

Pin a version or override the directories with command-local variables:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | \
  CODECLEW_VERSION=v0.1.0 \
  CODECLEW_INSTALL_ROOT=/absolute/private/path/releases \
  CODECLEW_BIN_DIR=/absolute/private/path/bin sh
```

After installation:

```bash
clew capabilities --human
clew doctor --human
```

The default `core` profile downloads only Kotlin 2.4.10. Enable or remove the
Kotlin 2.3.0 read-only preview between tasks:

```bash
clew pack install kotlin23
clew pack list
clew pack remove kotlin23
```

Omit `--human` to receive canonical JSON for automation and retained baselines.

The public pilot verifies checksums but is not yet Apple-notarized. Releases are
built only on GitHub macOS runners for the matching architecture and contain a
sealed runtime seed, so Rust, Cargo, and Gradle are not required to install or
start Codeclew. Git, Python 3.11+, and JDK 21 remain external dependencies when
analyzing Kotlin projects.

### Source-build dependencies

Required:

- macOS or Linux;
- Git;
- Python 3.11 or newer;
- JDK 21;
- Rust/Cargo from `rust-toolchain.toml` (currently Rust 1.92.0);
- at least 6 GiB free on the `CODECLEW_HOME` volume for a cold build;
- Maven in `PATH` only for Maven projects without `./mvnw`.

A cold runtime-capsule build also needs either populated local dependency caches
or access to approved Cargo, Gradle, and Maven sources. An isolated prebuilt,
signed runtime distribution is outside P0; fully offline or fleet deployment
requires a separate release pipeline.

### Installing from a pinned source checkout for development

```bash
git clone <approved-codeclew-repository> /absolute/path/to/codeclew
cd /absolute/path/to/codeclew
git checkout <approved-commit-or-tag>
git status --short

export CODECLEW_ROOT=/absolute/path/to/codeclew
export CODECLEW_HOME=/absolute/private/path/codeclew-state
mkdir -p "$CODECLEW_HOME"
chmod 700 "$CODECLEW_HOME"

./clew --bootstrap-component-preflight
./clew capabilities --human
./clew doctor --human
```

The checkout must be unchanged and pinned to an approved commit or tag.
`CODECLEW_HOME` must be a physically normalized absolute path owned by the
current user with mode `0700`. Do not share one state root between Unix users or
place it in a synchronized directory. When `CODECLEW_HOME` is unset, Codeclew
uses the user's cache root.

The first normal launch builds and seals an immutable runtime capsule. A warm
launch verifies and reuses it. After deployment, run both commands again without
`--human` and retain the `capabilities` and `doctor` JSON as the baseline for
that machine; both responses intentionally omit paths and repository identity.

Run a separate check for the target repository:

```bash
./clew doctor \
  --repo /absolute/path/to/target-repository \
  --target-ref refs/heads/feature/codeclew-task
```

The command can exit with code 0 and `status: ACTION_REQUIRED`. Automation must
inspect the JSON and proceed only when every item with `required: true` has
`status: PASS`.

## 3. Connecting Codex and Claude

The Codeclew checkout includes project skills for both agents:

- Codex: `.agents/skills/codeclew/SKILL.md`;
- Claude: `.claude/skills/codeclew/SKILL.md`.

If the agent runs from the target repository, copy the appropriate skill into
that repository. Do not create a machine-specific symlink or commit an absolute
Codeclew path:

```bash
export CODECLEW_ROOT=/absolute/path/to/codeclew
export TARGET_REPO=/absolute/path/to/target-repository

install -d "$TARGET_REPO/.agents/skills/codeclew"
install -m 0644 \
  "$CODECLEW_ROOT/.agents/skills/codeclew/SKILL.md" \
  "$TARGET_REPO/.agents/skills/codeclew/SKILL.md"

install -d "$TARGET_REPO/.claude/skills/codeclew"
install -m 0644 \
  "$CODECLEW_ROOT/.claude/skills/codeclew/SKILL.md" \
  "$TARGET_REPO/.claude/skills/codeclew/SKILL.md"
```

Set `CODECLEW_ROOT` in the agent's local environment before starting it. The
skill requires `$CODECLEW_ROOT/clew`, runs `capabilities` and `doctor`, forbids
guessing the language or compilation, checks freshness, and prevents publication
without explicit user approval. Copy the skill again after updating Codeclew.
In a controlled environment, verify that its hash matches the approved version.

To test discovery, explicitly ask the agent to apply `codeclew` to a safe,
read-only task. A correctly configured agent reports admission results
(`capabilities` and `doctor`) before attempting to read the whole repository
with general shell commands. Explicitly naming the skill remains the fallback
when automatic selection does not activate it.

## 4. Standard Kotlin workflow

The target ref must point to the current `HEAD`, and the worktree must be clean.
Work on a dedicated feature branch; Codeclew must not publish directly to a
protected branch.

```bash
"$CODECLEW_ROOT/clew" change open \
  --repo /absolute/path/to/kotlin-repository \
  --target-ref refs/heads/feature/codeclew-task \
  --language kotlin \
  --compilation :app/main \
  --intent 'describe the change' \
  --term ImportantSymbol \
  --term ImportantBehavior
```

Save the `sessionId` and `contextId` from the JSON response. Prepare a private
edit plan and check freshness before isolated mutation starts:

```bash
"$CODECLEW_ROOT/clew" change check-freshness --session session:...

"$CODECLEW_ROOT/clew" change prepare \
  --session session:... \
  --context context:sha256:... \
  --plan /absolute/private/path/edit-plan.json

"$CODECLEW_ROOT/clew" change status --run run:...
```

Review the candidate diff, compile and test results, and every obligation.
`READY_TO_PUBLISH_CONDITIONAL` and `VALIDATED_CONDITIONAL` are not unconditional
approval: the remaining checks must be performed explicitly or accepted by the
change owner.

Immediately before publication, repeat `change check-freshness`. Then, only
after explicit user approval:

```bash
"$CODECLEW_ROOT/clew" change publish \
  --session session:... \
  --run run:...

"$CODECLEW_ROOT/clew" session close --session session:...
"$CODECLEW_ROOT/clew" session gc --session session:...
```

## 5. Frequently updated repositories

A session is bound to an exact runtime, source commit, target ref, and target
OID. A remote push does not change the local ref by itself, but a local branch
update, another developer's commit, or an uncommitted edit changes its state.
Codeclew neither rebases nor automatically carries an old plan forward.

`change check-freshness` returns:

| Status | Meaning | Action |
|---|---|---|
| `FRESH` | `HEAD`, target ref, and expected OID match; worktree is clean | continue |
| `DIRTY` | local changes exist | stop; the owner chooses commit, stash, or another worktree |
| `STALE` | `HEAD` or the ref moved away from session authority | close the old session and open a new one |
| `UNAVAILABLE` | repository, locator, or Git is unavailable | restore access; do not publish |
| `TERMINAL` | session was closed, aborted, or garbage-collected | open a new session if needed |

Runbook for `STALE`:

1. Do not publish or reuse the old edit plan.
2. Retain only the human-readable intent when useful; do not copy old semantic
   assertions as established facts.
3. Close the old session.
4. Update the target feature branch using the team's normal process.
5. Restore a clean worktree with `target ref == HEAD`.
6. Repeat `doctor`, `change open`, context creation, and plan creation.

Use the same runbook when publication hits a compare-and-swap conflict. This is
expected race protection, not a reason to force-move the ref.

## 6. Reading and conditionally changing Python and Rust

Python is read from tracked UTF-8 `.py` blobs at the exact base commit. Codeclew
does not execute Python, import modules, read `.env`, or install dependencies.

```bash
"$CODECLEW_ROOT/clew" session open \
  --repo /absolute/path/to/python-repository \
  --target-ref refs/heads/main \
  --language python \
  --compilation 'python:.#src'

"$CODECLEW_ROOT/clew" context create \
  --session session:... \
  --intent 'find the request normalization path' \
  --term normalize \
  --term Request
```

The import root must equal the source root or be its ancestor. Results remain
`PARTIAL/UNSURE`: verify framework wiring, runtime imports, types, and actual
call edges with the Python project's normal test suite.

Rust requires a regular root `Cargo.lock` and an exact target selector:

```bash
"$CODECLEW_ROOT/clew" session open \
  --repo /absolute/path/to/rust-repository \
  --target-ref refs/heads/main \
  --language rust \
  --compilation 'cargo:crates/example/Cargo.toml#example#lib#example'
```

The Rust contour makes no claim about name resolution, `cfg`, procedural
macros, or call edges. For mutation, open with `change open` using the same
language/compilation selectors, build the immutable plan only from returned
sources and facts, and use a language-native validator:

```json
{"launcher":"CARGO","args":["test","-p","example","focused_test"]}
{"launcher":"PYTHON","args":["-m","pytest","tests/test_focused.py","-q"]}
```

Python validation executes `python3 -m`; activate the project's virtual
environment or put it first on `PATH` before `change prepare`. Shell commands
and `python -c` are rejected. Rust plans accept only Cargo validation. For both
languages, inspect `change status`, review the exact bounded diff and successful
validation evidence, then publish only with all returned authorities:

```bash
"$CODECLEW_ROOT/clew" change prepare \
  --session session:... --context context:... --plan /private/plan.json
"$CODECLEW_ROOT/clew" change status --run run:...
"$CODECLEW_ROOT/clew" change publish \
  --session session:... --run run:... \
  --allow-conditional \
  --prepared-authority-digest sha256:... \
  --acknowledge-obligation context:sha256:... \
  --acknowledge-obligation candidate:sha256:...
```

The final status is `PUBLISHED_CONDITIONAL`; native validation and acknowledgement
do not turn syntax evidence into compiler-backed certainty.

## 7. Multi-repository threads

Open a separate session for every exact repository, language, and compilation.
One repository may have multiple analysis units. Then connect two to eight
sessions:

```bash
"$CODECLEW_ROOT/clew" thread open \
  --member provider=session:... \
  --member consumer=session:... \
  --service-alias provider=orders \
  --service-alias consumer=checkout

"$CODECLEW_ROOT/clew" thread context \
  --thread thread:... \
  --intent 'trace normalization across services' \
  --term normalize \
  --term Service
```

Qualified Kotlin members support `thread callables`, `thread impact`, and
conditional `thread validate`; see the README for complete examples. A thread
is read-only, does not own its member sessions, and cannot be used in a plan or
task run. Closing or garbage-collecting a thread does not close member sessions:

```bash
"$CODECLEW_ROOT/clew" thread close --thread thread:...
"$CODECLEW_ROOT/clew" thread gc --thread thread:...
```

When any repository changes, open a new session for that member and create a
new immutable thread. Never replace a member inside an old thread retroactively.

## 8. Errors and investigation material

Full stdout may contain source windows, diffs, symbols, arguments, identifiers,
and paths. It is useful for local investigation but is unsafe to send. Codeclew
never sends it automatically.

Create a private directory and capture stdout and stderr separately:

```bash
umask 077
INCIDENT_DIR=/absolute/private/path/codeclew-incident-$(date +%Y%m%d-%H%M%S)
mkdir -m 700 "$INCIDENT_DIR"

"$CODECLEW_ROOT/clew" <command> \
  >"$INCIDENT_DIR/result.json" \
  2>"$INCIDENT_DIR/completion.json" || true
chmod 600 "$INCIDENT_DIR/result.json" "$INCIDENT_DIR/completion.json"
```

For a core error or status, pass the local `result.json` to the allowlist-based
converter. For a bootstrap failure, provide a file containing exactly one
bootstrap error JSON object. If the broken installation cannot start the
summarizer, use a working installation of the same approved version on a
trusted machine:

```bash
"$CODECLEW_ROOT/clew" support summarize \
  --input "$INCIDENT_DIR/result.json" \
  >"$INCIDENT_DIR/shareable-summary.json"
```

The input must be a normalized absolute path to a non-symlink regular file,
owned by the current user, exactly mode `0600`, and no larger than 1 MiB.
Codeclew checks identity, size, and timestamps before and after reading, so a
racing modification fails closed.

The output is built only from an allowlist and has `status: SAFE_TO_SHARE`. It
contains the schema and stage, typed error code or terminal status,
retryability, remediation ID, and a digest of the sanitized summary itself. It
does not carry messages, source, diffs, symbols, arguments, repository content
digests, repository/session/run identity, or paths.

Attach only the following to a support request:

1. `shareable-summary.json`;
2. fresh `capabilities` JSON;
3. fresh `doctor` JSON, with or without `--repo`; both forms are path-free;
4. the approximate event time and reproducibility, without names, paths, or
   code fragments.

Do not attach raw stdout or stderr, plans, candidate diffs, CAS objects,
runtime/state directories, Git remotes, private symbol names, or command lines.
Originals remain on the developer machine under the team's retention policy and
are deleted after the investigation closes.

## 9. Common incident runbooks

### Worker crash

1. Inspect the typed error and `retryable` value.
2. For `WORKER_CRASHED`, retry once without changing authority.
3. If it repeats, stop, generate a safe summary, and retain raw artifacts
   locally.
4. Do not retry indefinitely or delete state before the investigator decides.

### `WORKTREE_RECOVERY_REQUIRED`

1. Do not edit the candidate worktree manually.
2. Run `change recover --session session:... --run run:...`.
3. Fetch `change status` again and follow the reported remediation.
4. If recovery repeatedly fails, preserve the incident locally and stop.

### `PROJECT_MODEL_CHANGED`

The session was created under another runtime/model authority. Run it from the
original pinned Codeclew checkout, or close it and create a new session with the
current version. Do not rewrite session JSON.

### Insufficient disk space

Free at least 6 GiB on the state volume, then repeat `doctor`. Remove only
terminal sessions and threads through their `gc` commands; manual object removal
can break authority. If state corruption is suspected, first retain a local
incident and stop writing.

### Dirty worktree

Codeclew never runs stash or reset. The owner chooses whether to commit, create
a separate Git worktree, or stash manually. Then repeat `doctor` and the
freshness check. The agent must not make this decision autonomously.

## 10. Updating Codeclew

### Installed macOS release

Run the updater between tasks:

```bash
clew upgrade
```

The command checks the latest GitHub Release first. If the installed version is
current, it exits without downloading the release bundle. If an update exists,
it preserves the current install root and launcher directory, downloads the
exact newer release, verifies its published SHA-256 checksum, and atomically
switches the launcher. Codeclew state, repositories, sessions, and threads are
not modified. The updater preserves the installed `core` or `kotlin23` profile.

Releases older than v0.1.3 do not contain the updater. Upgrade such an existing
installation once by rerunning the one-line installer:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
```

After that transition, use `clew upgrade` for later releases. If `clew` is not
on `PATH`, invoke the installed launcher directly, normally
`~/.local/bin/clew upgrade`.

### Source checkout

`./clew upgrade` deliberately refuses to update a source checkout. Update a
pinned checkout through Git, and only between tasks:

1. Complete or explicitly abort and close all active sessions and threads.
2. Retain the old version's `capabilities` and `doctor` baseline.
3. Switch to the new approved commit or tag with no local modifications.
4. Run bootstrap preflight, `capabilities`, `doctor`, and a pilot smoke case.
5. Refresh copied skills in target repositories.
6. Do not migrate or edit old session records. Open new sessions for the new
   runtime.

Rollback means using the old pinned checkout with its compatible runtime. The
shared state stores content-addressed capsules, but P0 does not promise that a
new CLI can continue an old session after runtime/model authority changes.

## 11. Adding a language or expanding a profile

Language support is not a single parser plug-in. The minimum safe path is:

1. Define the profile and its boundary in the support matrix. Start with
   `READ_ONLY_PREVIEW` and `mutation: false`.
2. Implement or select a `BuildModelProvider` for exact compilation authority.
3. Implement the `LanguageAdapter` handshake and generation facts with explicit
   completeness, certainty, and obligations.
4. Read only a sealed repository snapshot and publish canonical bounded facts
   to CAS. Do not scan the ambient filesystem or execute the project on the
   query path.
5. Add CLI language/compilation parsing, admission, and path/privacy tests.
6. Add a fixture corpus covering valid projects, ambiguity, parse/model failure,
   symlinks, dirty and untracked data, resource limits, and deterministic replay.
7. Prove read-only acceptance on real projects.
8. For mutation, separately implement plan validation, an isolated candidate,
   compile/test gates, effects/writeset/ABI checks, freshness/CAS publication,
   and recovery. Change the support matrix only after an independent mutation
   gate passes.

Current extension points are in `crates/clew/src/adapter_v2.rs`; references are
`kotlin_adapter_v2.rs`, `python_adapter_v2.rs`, and `rust_adapter_v2.rs`. The
Python and Rust project-model paths are in adjacent modules. The runtime-packaged
Kotlin worker lives under `workers/` and is registered through component
manifests. Every new compiler version is a new profile, not a silent replacement
for an existing one.

## 12. P0 acceptance checklist

A deployment is pilot-ready when:

- the checkout and support matrix are pinned to an approved commit;
- `capabilities` and all required `doctor` checks pass;
- the state root is private and not shared;
- the Codex or Claude skill is installed and verified with a read-only request;
- strict mutation is admitted only for the exact Kotlin P0 profile;
- Rust/Python mutation remains conditional on native validation and explicit
  acknowledgement of every `UNSURE` obligation;
- freshness is checked before preparation and publication;
- conditional obligations remain visible;
- publication requires explicit human approval;
- the incident workflow produces only a `SAFE_TO_SHARE` summary;
- the team can execute stale, recovery, upgrade, and disk-space runbooks.

A signed installer, centralized fleet management, automatic diagnostic upload,
cross-host shared state, semantic Rust/Python resolution, and multi-repository
publication remain intentionally outside P0.
