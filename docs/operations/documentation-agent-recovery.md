# Documentation agent recovery

Use the installed release and its actual `--help` output for target repositories.
This is a short recovery guide, not another admission or approval layer. The
`read-dependency-part` command below belongs to the accompanying product change;
older releases do not implement it. Confirm capabilities before using it.

## Keep stage and identity explicit

Retain one root, service, source revision, snapshot, Work ID and entrypoint.
A prepared Work is not a read context. A submitted proposal is not a published
operation. `NEEDS_REPAIR` is blocked, not done; a draft is still a draft. Only
publication plus inspection of its output establishes a documentation result.
Do not call unassessed publication reviewed or runtime-proven.

Use returned references and participant identities. Do not invent `client`,
service aliases, observation IDs or cursor offsets to satisfy a schema. For
substring discovery use `query.kind: SYMBOL`, `projection: NAVIGATION`, then
expand the returned exact references; discovery summaries are not citable reads.

## A required record exceeds the Work byte budget

Read every default Work page by following its returned `nextCursor`. Increasing
`maxItems` cannot make a single 180 KiB record fit a 48 KiB response. A different
profile is not a recovery action unless the product reports it compatible.

An oversized DEPENDENCY now includes `omitted[].recovery.input` and the command
`docs work read-dependency-part`. Save that exact input to a JSON file and run:

```sh
clew docs work read-dependency-part --root ROOT --work WORK --input part.json
```

Continue with each returned `nextCursor`, keeping the exact schema/kind/id,
until it is null. `text` is a canonical-JSON UTF-8 fragment; it is not a complete
JSON record until all parts are concatenated in byte order. The product records
and validates complete delivery against the immutable Work. A partial read,
missing page, altered digest or another Work/snapshot does not satisfy the gate.

SOURCE uses `docs work read-part` with its source reference. Retained authoring
uses `docs work read-retained-part` with its exact kind/id. Reading source files,
rendered JSON or `docs context` outside Work does not create these receipts.
Manual receipts alone do not establish delivery to an automatic author/reviewer
request. Never disable completeness checks to unblock an authoring run.

Retry `proposal submit` after the missing context or invalid references actually
change. Repeating the same submit or scanning the same omitted record does not
repair it. If the installed version has no supported read route, report the
exact blocking record and stop that publication; continue independent work.

## Callables are not automatically public entries

The source catalogue is evidence, not a to-do list to author every helper. New
rendering does not require separate narratives for source callables that the
existing classifier does not identify as public boundaries. Their source stays
available for selected process/thread descriptions and in the source inventory.
Explicitly authored helper operations remain supported.

`@RestController` alone does not prove that every method is mapped. For a route
inherited from an OpenAPI interface, retain the generated interface and its
mapping/override evidence before claiming an HTTP binding. Error handlers and
mappers belong to the relevant behavior, not automatically to independent public
starts. Missing routes remain a discovery limit, not proof of absent endpoints.

Do not narrow source roots merely to clean navigation: that can remove the
callee bodies needed for a complete explanation. Existing publication exclusions
are discoverable with:

```sh
clew docs endpoint list --root ROOT --service SERVICE --snapshot SNAPSHOT
clew docs endpoint exclude --root ROOT --service SERVICE --snapshot SNAPSHOT --endpoint RETURNED_ENDPOINT_ID
```

Exclusions change maintained publication selection; they do not delete captured
source or establish which methods are runtime entries. Do not blanket-exclude
unresolved controllers. Preserve the user's declared process scope.

## Reader discovery options

By default, saved processes stay navigable while speculative internal process
candidates are hidden. Opt in by saving `catalog/reader.json`:

```json
{"showInternalProcesses": true}
```

The internal discovery section requires actual candidates; empty sections are
not created. A saved process is not automatically end-to-end: keep its declared
scope and review status visible.

## Contracts and capture capabilities

Inspect the returned coverage, public-boundary count and contract-operation
count. An installed compile profile name is not evidence of compiler-qualified
capture; `coverage: SYNTAX` must retain its actual meaning. Check profile support
and source-selection configuration before another expensive capture.

When registered full OpenAPI files are retained but operation projection is
missing, inspect contract diagnostics and the installed recovery command:

```sh
clew docs refresh-contracts --root ROOT --snapshot SNAPSHOT --service SERVICE
```

Use its returned snapshot for subsequent Work/render. Do not mix immutable
snapshots or rewrite private CAS objects. Some versions already capture and
project contracts during `check`; do not assume refresh is universally required
or blame an OpenAPI minor version without evidence.

Routine command recovery, profiles, pagination and presentation choices should
not become repeated user questions. Ask about business meaning, scope or a real
tradeoff. Verify the output against the user's questions about inputs, flow,
effects, errors and outcomes; counts, placeholders and static PASS are not that
acceptance.

## Refresh agent instructions after an upgrade

The release bundles its skill, but upgrading the CLI does not automatically
replace personal or project skill copies. Run `clew skill install --agent codex`
or the selected client's equivalent. If it reports a different package, review
and back up local edits before using the same command with `--force`. For a
project-scoped installation, use `--project /absolute/target-repository` and
select the intended agent. Ask the working agent to reload the updated skill;
its existing context may still contain older instructions.

Existing documentation-root instructions are also preserved. `docs init` does
not overwrite `AGENTS.md` or an existing `AGENTS.codeclew-docs.md`; rerunning it
is not an instruction update. Generate a comparison copy in a new scratch root:

```sh
clew docs init --root /absolute/new-scratch-docs --title "Instruction comparison"
```

Compare its generated `AGENTS.md` with the maintained instructions in the real
documentation root, and merge only the applicable Codeclew guidance. Preserve
human notes, local authorization rules, authored narratives and immutable
history. In particular, remove an old requirement to author every source helper;
public boundaries and explicit user selection define publication scope. Do not
rewrite historical snapshots to make them appear generated by the new release.
