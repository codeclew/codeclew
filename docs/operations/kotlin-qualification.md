# Kotlin qualification failures

Run one compatibility row with JDK 21 and retain its diagnostic artifacts:

```sh
python3 -I -S scripts/kotlin-compatibility-gate.py \
  --row k24-exact --full-lifecycle --output /tmp/k24-exact
```

The output directory contains the test log and normalized compiler facts. The
scheduled workflow retains these files for each attempted row on both hosts.
The gate compares the complete checked-in facts snapshot before comparing its
digest. An unexpected structural difference must be explained before updating
either artifact. Discovery mode does not qualify a changed golden.

The previous digest `51e4f7601614b17e8165c3d3b0b5e580ab402815f9fbab4fbd9e0e85cde96075`
is reproducible at `b666129b`. The snapshot at `540bf957` adds primary-constructor
storage bindings, backing-property storage, JVM annotation and hierarchy facts,
and three local compiler CFGs. File facts and declaration relations remain
unchanged. The new digest is
`a770f6c8566aecdf8a67dfedf4bc8a806da7782d2852338bb91e205fa5b6729f`.
These facts remain part of qualification; normalization does not discard them.

`RAW_SCHEMA_HASH` also covers failure while receiving a worker response, before
any semantic payload exists. A frame-size failure together with a worker exit
must be diagnosed at the process boundary. In particular, HotSpot's
`ExitOnOutOfMemoryError` can write termination text to protocol stdout and exit
with code 3. This is not a valid oversized compiler response. Source analysis
must remain unavailable when the response cannot be verified.

The worker heap ceiling is 2 GiB so the real worker-source dogfood compilation
can complete with its expanded compiler facts. Frame and blob limits, typed
response checks, and compiler-evidence validation remain in force. This does
not bound the memory of project-native Gradle or Maven children.

Retained working-tree comparison allows up to 131,072 relation facts, matching
its existing total generation-fact visit ceiling. The older 32,768-relation
ceiling rejected the self-hosting compilation after successful compiler
analysis. The 64 MiB semantic-payload and retained-report limits still apply;
exceeding a bound remains an explicit failure rather than a complete result.
