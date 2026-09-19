---
name: debug-codeclew-maven-admission
description: Diagnostic procedure for Codeclew Maven admission failures — capture real build output, identify read-only worktree vs JDK vs plugin issues
source: auto-skill
extracted_at: '2026-09-14T15:55:00.000Z'
---

# Debug Codeclew Maven Admission Failures

Use this procedure when `clew docs check` or `clew doctor task` returns
`BUILD_COMMAND_FAILED` for a Java/Maven service. The failure is **rarely a JDK issue** —
Codeclew correctly inherits `JAVA_HOME` from its own process environment. The most common
root cause is a project plugin that writes to `src/main/java` in-place while Codeclew
runs Maven inside a **read-only session worktree**.

## Step 1 — Capture real Maven output

The installed `clew` (e.g. `~/.local/bin/clew`) does not support `--debug-output`.
Use the research checkout instead:

```bash
mkdir -p /tmp/clew-debug && chmod 700 /tmp/clew-debug

cd ~/repo/research/codeclew
export JAVA_HOME=/path/to/jdk-17
export PATH=$JAVA_HOME/bin:/opt/homebrew/Cellar/maven/3.9.16/libexec/bin:$PATH

./clew docs check \
  --root ~/repo/kasko/arch-kasko \
  --debug-output /tmp/clew-debug \
  > /tmp/clew-check.log 2>&1
```

After completion, read the captured files:

```bash
ls -la /tmp/clew-debug/
# Look for maven-*.stdout — this is the real Maven output
cat /tmp/clew-debug/maven-*.stdout | tail -40
```

The `stdout` file contains the exact Maven error (not the sanitized version shown
in `clew docs check` output).

## Step 2 — Identify the failure category

Check the last lines of `maven-*.stdout` for one of these patterns:

### A. `Permission denied` on a `src/main/java/...` file

Illustrative excerpt with the write-failure message translated into English:

```
[ERROR] Failed to execute goal ru.tins:java-code-transform-plugin:1.0.6:mask-fields-annotate
on project common: ... Failed to write file:
.../repo/common/src/main/java/.../DealHashPost.java (Permission denied)
```

**Cause:** Codeclew runs Maven in a session worktree (`~/.cache/codeclew/v2/attempts/<id>/repo`)
where source files are `read-only` (mode `0550`). The `java-code-transform-plugin` (from
kasko-utils) rewrites Java sources in-place → `Permission denied`.

**Resolution:** Use the `source-syntax` profile instead of `java-17plus-maven-read-only`.
The source-syntax profile parses Java grammar without running Maven at all, bypassing
the read-only worktree entirely. Update the service catalog:

```json
{
  "schema": "codeclew-documentation-service/1.0",
  "id": "service-name",
  "title": "Service Name",
  "repositoryId": "service-name",
  "repository": "https://gitlab.tcsbank.ru/kasko/service-name.git",
  "language": "java",
  "profile": "source-syntax",
  "targetRef": "master",
  "source": {
    "roots": ["src", "pom.xml"],
    "dialect": "17"
  }
}
```

### B. `NoSuchFieldError: JCTree$JCImport ... qualid`

```
[ERROR] Fatal error compiling: java.lang.NoSuchFieldError: Class com.sun.tools.javac.tree.JCTree$JCImport does not have member field 'com.sun.tools.javac.tree.JCTree qualid'
```

**Cause:** Lombok 1.18.22 is incompatible with JDK 21. The project declares `java.version=17`
but the build runs on JDK 21.

**Resolution:** Run Codeclew with `JAVA_HOME` pointing to JDK 17:

```bash
export JAVA_HOME="$(/usr/libexec/java_home -v 17)"
```

### C. `Error while writing to classpath file`

```
[ERROR] Failed to execute goal org.apache.maven.plugins:maven-dependency-plugin:3.8.1:build-classpath
on project task-router-leads: Error while writing to classpath file
'<worktree>/target/codeclew-classpath.txt'
```

**Cause:** The root aggregator POM (`packaging=pom`) is included in the reactor by `-am`.
`build-classpath` tries to write `target/codeclew-classpath.txt` in the root module's
`target/` directory, which doesn't exist in a fresh worktree.

**Resolution:** This is a known Maven dependency-plugin quirk with reactor builds.
If the error only appears on the root aggregator but not on the actual submodule,
it may be safe to ignore if the submodule itself compiles. However, Codeclew treats
any `BUILD_COMMAND_FAILED` as admission failure — use `source-syntax` profile as workaround.

### D. `BUILD_MODEL_OUTPUT_LIMIT` / `BUILD_COMMAND_FAILED` with no clear error

**Cause:** The Maven build succeeds but stdout exceeds 4 MB limit, or the build
fails for an unrelated reason (missing credentials, unreachable Artifactory, etc.).

**Resolution:** Read the full `maven-*.stdout` file — it may contain the actual error
hidden among many lines of OpenAPI generator output. Look for `[ERROR]` lines.

## Step 3 — Verify JDK is correct

If the error is JDK-related (Lombok, compiler version mismatch), confirm the JDK:

```bash
# Check what Codeclew process sees
CLEWPID=$(pgrep -f 'bin/clew docs check' | head -1)
ps eww $CLEWPID | tr ' ' '\n' | grep -E '^JAVA_HOME=|^PATH='

# Manual verification
JAVA_HOME=/path/to/jdk-17
cd ~/repo/kasko/<service>
/bin/sh mvnw -B -q compile dependency:build-classpath \
  -DskipTests -Dstyle.color=never \
  -Dmdep.outputFile=target/codeclew-classpath.txt \
  -Dmdep.regenerateFile=true \
  -Dmdep.includeScope=compile \
  -f pom.xml -pl <submodule> -am
```

If this succeeds but `clew docs check` fails, the issue is the read-only worktree
(category A above), not JDK.

## Step 4 — If source-syntax profile works

After switching to `source-syntax`:

```bash
cd ~/repo/research/codeclew
export JAVA_HOME=/path/to/jdk-17

./clew docs check \
  --root ~/repo/kasko/arch-kasko \
  --debug-output /tmp/clew-debug \
  > /tmp/clew-check.log 2>&1
```

Check that the service no longer appears in `unresolved`. The output will show
`SOURCE_MATCH` (unique lexical declaration) and may include
`SYNTAX`, `ORDER_LEXICAL_ONLY` qualifiers — these are expected for source-syntax.

## CODEDEBUG pipeline tracing

`clew docs check` emits `CODEDEBUG` markers to stderr during the seal run.
These markers let you track progress in real-time without waiting for the
entire run to finish:

```bash
# Follow CODEDEBUG markers as the seal run progresses
grep CODEDEBUG /tmp/task6-seal-stderr.log | tail -40
```

### Marker sequence (what each stage means)

```
doctor START/OK <service>          — prerequisites validated
extract_java_model START           — about to run Maven
maven EFFECTIVE_POM START/OK       — effective POM extracted
maven COMPILE_CLASSPATH START/OK   — classpath built
maven RELEASE OK                   — model release succeeded
extract_java_model OK/ERR          — full model extraction result
transformed_java_source_digests OK — source digests computed
capture_session START/OK/ERR       — evidence capture pipeline
capture_session reading sources    — reading committed sources
contracts OK                       — OpenAPI contracts processed
modules OK                         — documentation modules processed
cache write OK                     — cache persisted
```

### Interpreting failures from CODEDEBUG

- `extract_java_model ERR BUILD_COMMAND_FAILED` — Maven exited non-zero.
  The error message includes reproduction guidance (`-B -q compile`,
  `-f <pom>`, `-pl <module> -am`). Read the captured `maven-*.stdout`
  from `--debug-output` for the real error.

- `capture_session ERR BUILD_COMMAND_FAILED` — Same root cause as above,
  surfaced at the capture stage. The extract failure propagates.

- `capture_session OK (0 sources, N observations)` — Capture succeeded
  but no sources were read (expected when the service has no `source.roots`
  or when using `java-17plus-maven-read-only` without source roots).

- `capture_session OK (N sources, M observations)` — Full capture succeeded.

### Monitoring a running seal

The seal process runs as a child of the wrapper command. Check liveness:

```bash
# Check if clew is still running
ps -o pid,etime= -p $(pgrep -f 'bin/clew docs check' | head -1) 2>/dev/null \
  || echo "SEAL PROCESS GONE"

# Check final result
python3 -c "import json;d=json.load(open('/tmp/task6-seal.json'));print(json.dumps(d,indent=2))"
```

The final seal output (`/tmp/task6-seal.json`) has schema
`codeclew-documentation-check/1.0` with:
- `status`: `UNRESOLVED` (any service has unresolved issues) or `OK`
- `services.<id>.coverage`: `FULL` / `PARTIAL` / `NONE`
- `services.<id>.entrypoints`: count of HTTP endpoints
- `services.<id>.boundaries`: list of boundary conditions
- `unresolved.<id>`: present only for failed services, with `reason` and
  `nextAction` containing the actionable error message

## Troubleshooting notes

- **Session worktrees are ephemeral.** They get GC'd after the session ends. If you
  need to reproduce a worktree-specific issue, create your own detached worktree:
  ```bash
  git worktree add --detach /tmp/test-repo master
  chmod 0550 /tmp/test-repo  # simulate read-only
  ```

- **`--debug-output` requires mode 0700 and caller ownership.** Create the directory
  before running: `mkdir -p /tmp/clew-debug && chmod 700 /tmp/clew-debug`.

- **The installed `clew` does not support `--debug-output`.** Always use the research
  checkout (`~/repo/research/codeclew/./clew`) for diagnostic captures.

- **`codeclew.yaml` with `maven.settings`** only affects which `settings.xml` Maven
  uses — it does not change the worktree behavior or bypass read-only restrictions.

- **`doctor task` passes** even when `docs check` fails — `doctor task` validates
  prerequisites but does not run the Maven build. Only `docs check` triggers the
  actual compilation and classpath extraction.

- **Seal runs process tasks sequentially.** If an early task fails (e.g. `task-manager`
  with BUILD_COMMAND_FAILED), subsequent tasks (e.g. `task-router-leads`) still run
  and may succeed. The final seal output aggregates all task results.
