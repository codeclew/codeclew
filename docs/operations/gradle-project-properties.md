# Set Gradle project properties for compiler capture

Put a `codeclew.yaml` in the checkout that owns the Gradle wrapper. For a docs
root with several services, use the checkout bound to each Gradle service.
Services bound to the same checkout share its declaration.

```yaml
version: 1
gradle:
  projectProps:
    version: '1.1'
    env: 'staging'
    profile: 'internal preview'
```

Run `clew docs check --root DOCS_ROOT --service SERVICE_ID` with the service's
Java or Kotlin compiler profile. Codeclew reads the local file before taking its
source snapshot. Each property becomes one `-Pkey=value` argument to the Gradle
model invocation. Kotlin uses the same declaration in project-native and sealed
external build state. Tasks that Gradle runs as dependencies of model extraction
see the same properties. A declared CLI property takes precedence over the same
key in `ORG_GRADLE_PROJECT_*`, `org.gradle.project.*`, and `gradle.properties`.
Other inherited properties still follow Gradle's rules for the selected mode.
A build script can explicitly assign a different project version or otherwise
ignore a property.

Use at most 32 entries. Keys start with an ASCII letter or digit and may also
contain `.`, `_`, and `-`, up to 128 characters. Reserved Gradle and Codeclew
controller namespaces and `kotlin.project.persistent.dir` are rejected. Values
must be YAML strings of at most 1,024 bytes without control characters. Quote
numeric and Boolean-looking values. Spaces and shell metacharacters are literal.

The Check reports the source, applied status and count for a declared set. To
use a different set, edit or remove the declaration and run `docs check` again.
The new capture reflects the change even when the checkout's commit has not
changed. An explicit older snapshot still reads the historical capture. Removing
a declaration restores the project's inherited behavior. Keep the file local
and untracked if its purpose is to configure Codeclew on this machine.

For an open session, `clew change check-freshness --session SESSION_ID` reports
when the declaration no longer matches. Open a new session to use the current
properties. Reordering keys or restoring the admitted values leaves the property
binding unchanged. Creating or expanding a context requires a matching
declaration. Explicit reads of retained source still use the saved context.

Use this interface only for nonsecret values. Trusted Gradle plugins, build
scripts, and the same user's process inspection tools can see the actual
arguments. Codeclew does not copy the declaration into receipts or default
support reports. Gradle code controls what it writes into build outputs and
retained source files. Use a checkout you trust and check its outputs before
sharing them. A property can supply a version when the build supports an
override. A plugin that requires a real Git tag can still fail.

## Supply local Git tags to Kotlin Gradle capture

When a trusted Kotlin Gradle build enumerates release tags during model
configuration, opt in from the bound checkout's local `codeclew.yaml`:

```yaml
version: 1
gradle:
  gitTags: true
```

Run `clew docs check --root DOCS_ROOT --service SERVICE_ID` with the service's
Kotlin Gradle compiler profile. The expected tags must already exist in the
bound checkout. The build's sort order, filtering, and limits still apply.
The setting also applies to Kotlin Gradle managed sessions for the checkout.

The isolated Git repository receives the complete local tag namespace with
original ref and object identities. This supports tag enumeration, default
Git version sorting such as `git tag --sort=-v:refname`, and peeling annotated
tags to their original commit. The sealed source checkout retains its synthetic
HEAD. History, original trees, ancestry, and `git describe` relative to the
original HEAD are outside this interface. Git version-sort overrides in the
bound checkout are rejected.

Codeclew admits up to 512 direct tag refs of at most 512 UTF-8 bytes each.
Tags can point to commits or through up to eight annotations to a commit.
Up to 1,024 unique tag and commit objects are admitted, with a 64 KiB limit
per object and 4 MiB total. Missing objects, unsupported targets, and oversized
inputs fail explicitly. Codeclew reads local refs and objects without fetching
or opening the bound Git directory to Gradle.

Successful Checks report the applied tag count. Their retained evidence
binds the capture to the selected source revision and one private tag input
identity. A same-commit tag change requires a new Check or managed session.
Explicit historical reads still use their saved evidence. Tag names and
annotation messages are available to the trusted build process. Check any
build outputs before sharing them.

## Select the compilation for compiler capture

For the `kotlin-jvm-gradle-analysis` profile, the service's compilation
identifies the Gradle project and Kotlin source set. For example, `:/main`
selects the root `:compileKotlin` task and `:payments/main` selects
`:payments:compileKotlin`. The Gradle wrapper, build scripts, applied
plugins and toolchain determine the actual compiler invocation. Codeclew
can observe the selected Kotlin task and schedule qualified Java generators
attached through `compileJava`.

Compiler capture requires source, generated inputs, the selected invocation
and a compiler observation for the selected revision. A prebuilt output
directory can provide dependencies where supported. Existing classfiles alone
cannot establish which inputs or compiler arguments produced them, or recover
exact source occurrences for a compiler Check.

## Run explicit preparation tasks before compiler capture

Use `gradle.prepareTasks` when the build requires a separate invocation of a
generator before compiling. For example, a service registers a generated
directory as a source root but its compile task has no dependency on the
generator.

```yaml
version: 1
gradle:
  projectProps:
    profile: 'preview'
  prepareTasks:
    - ':app:generateAvroJava'
    - ':app:prepareApiTypes'
```

Use up to 16 distinct absolute task paths. Codeclew runs the requested tasks
with the selected revision and the same Gradle properties as compiler capture,
inside the disposable source checkout. Gradle still controls the order between
preparation tasks via its dependency graph. Tasks that require the selected
compiler, or the paired `compileJava` in a Kotlin capture, can cause a cycle and
are refused. Configure the producer that creates the sources instead.

In the output of `clew docs check`, open
`services.<service>.gradlePrepareTasks`. Each compilation lists the requested
tasks and their declared outputs. Each observed regular file has a
repository-relative path, a SHA-256 digest and a selection status.
`SOURCE` denotes an exact source input. `CLASSPATH` denotes a file on the
classpath or under one of its admitted directories. `NOT_SELECTED` denotes
an observed output outside both selections. `CHANGED_AFTER_TASK` means the
observed bytes changed or disappeared before input selection. A task with
observed outputs but no selected files reports `NO_SELECTED_OUTPUTS`. A
skipped task with no observation reports `UNAVAILABLE`.

Output paths must stay inside the disposable checkout and use regular files
and directories. Codeclew refuses unsafe or oversized output observations.
Task failures stop the Check. Edit the local declaration and run a fresh
Check to add, remove or change tasks. Source-syntax Checks do not invoke
these tasks. The detailed inventory is in local Check output and retained
evidence. Default support reports include a fixed failure code when available
and omit task names, paths and Gradle properties.
