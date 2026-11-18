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

Use this interface only for nonsecret values. Trusted Gradle plugins, build
scripts, and the same user's process inspection tools can see the actual
arguments. Codeclew does not copy the declaration into receipts or default
support reports. Gradle code controls what it writes into build outputs and
retained source files. Use a checkout you trust and check its outputs before
sharing them. A property can supply a version when the build supports an
override. A plugin that requires a real Git tag can still fail.
