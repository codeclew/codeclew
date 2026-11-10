# Read retained static configuration files

JVM compiler captures can retain versioned YAML and properties files from the
selected source sets' standard resource directories. These files carry
`STATIC_CONFIGURATION_FILE_ONLY:<path>` boundaries. They are exact snapshot
text, without configuration resolution, environment values or compiler semantics.

Use an existing service Work and its frozen snapshot. A SOURCE_SCOPE is not
required for the separate retained resource query:

```json
{"query":{"kind":"RETAINED_RESOURCE","projection":"NAVIGATION","fileContains":"application"}}
```

Save the selection as JSON and pass it to `clew docs work read --root DOCS
--work WORK --input SELECTION.json`. Omit `fileContains` to discover all retained
static files; use `file` for one exact returned path. Follow the returned
`fullRecordReference` with `{"projection":"SOURCE_ROWS","references":["RETURNED_REF"]}`.
Alternatively, `RETAINED_RESOURCE` with `projection: "RAW"` returns the original
SOURCE row directly. Drain cursors and recover oversized omissions with the
existing `docs work read-part` route before citing the text.

Navigation metadata is not citable content. Source identity, revision, text,
digest and occurrence remain those of the original capture. No new capture,
repository read, SOURCE_SCOPE, dependency or setting value is created. Empty
results establish only that this Work has no matching retained resource. The
ordinary `SOURCE` query continues to require its registered SOURCE_SCOPE.
