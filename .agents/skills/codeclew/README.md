# Prepared SOURCE answers

Read this file only for a supplied docs root, service, exact selector and saved
snapshot or answer/evidence. Do not load the full managed-code or service guide.
Use absolute task paths or an explicit working directory; a docs workspace need
not be the source Git checkout. Resolve the installed `clew` once when needed.

Choose before acquiring context:

- **Verified unchanged:** reuse the saved answer, evidence and citations. Add a
  short checked/unchanged annotation naming the verified scope. No context fetch,
  refresh, packet dump or full answer rewrite. Read the saved answer or small
  saved projection only if needed; do not print old JSON packets again.
- **Current snapshot supplied:** reuse its local packet, or fetch missing context
  once against that exact snapshot. No installation, registration, bind, doctor,
  check or refresh. The docs command performs its own admission.
- **Relevant changes:** use the supplied current snapshot if ready; otherwise
  refresh the selected registered service once at the explicitly requested ref.
  Update affected answer clauses. Refreshing a caller does not reverify receivers.
- **Unrelated changes:** separate repository/provenance change from behavior.
  Reuse interpretation only when relevant-input checks justify it. Keep the older
  cited revision unless actual returned bindings establish newer provenance.

Relevant inputs include the registered SOURCE scope and membership, called
helpers, receivers, configuration, registered generated inputs, contracts, and
external/reference inputs used by the answer. Equal handler text is insufficient.
A changed callee/config/generated input requires affected evidence refresh or an
explicit missing-input limit. Unobserved dependencies remain unknown. Operator
checks do not upgrade `PINNED_SNAPSHOT_NOT_REVERIFIED`, coverage or meaning review.

## Bounded evidence, full packet local

Use the bundled helper without reading its implementation. Replace placeholders
with supplied paths/handles. It never decides freshness or edits service records.

```sh
# Existing packet: local read only, no clew invocation.
python3 -I -S SKILL/scripts/prepared_source.py read --input PACKET.json

# Missing packet, ready snapshot: one CLI call; OUTPUT must be a new private file.
python3 -I -S SKILL/scripts/prepared_source.py fetch --clew CLEW --root DOCS --snapshot SNAPSHOT --service SERVICE --symbol EXACT_ID --output OUTPUT.json
```

The view preserves packet authority, coverage, revisions/digests, source bindings,
unresolved/omitted evidence and `nextCursor`. Full stdout and private `.stderr`
are saved locally on fetch, including failures; existing files are never replaced.
Do not `cat`, pretty-print or truncate raw packets as a substitute for the view.
Default stdout is at most **12,000 UTF-8 bytes**, with up to three SOURCE records
and four related handles. SOURCE_ALIAS bodies and normalized catalogues stay
local. Nested record limits remain visible. Local projection omissions are
separate from the original packet's `omitted` evidence.

`textDelivery.complete=false` means a source excerpt is partial; use its exact
`--source ID --text-offset N` with `read` to continue locally. `--text-chars`,
`--max-sources`, `--max-handles` and `--max-bytes` (1,024..65,536) bound the view.
`--text-chars 0 --max-handles 0` gives metadata without repeating source text.
Only drill `--source` / `--dependency` through `clew docs context`, or follow the
original cursor, when required evidence is missing. Keep each result local and
project it. A helper error delivers no evidence; inspect its local packet/log,
request fewer sources or a larger bounded view rather than suppressing limits.

For authorized refresh, replace `--snapshot SNAPSHOT` with `--refresh`. These are
alternatives. Refresh uses the registered ref; it does not select a new one. If
that ref must change, batch `docs service show --id SERVICE`, changing only the
full record's `targetRef`, `docs service add --expected-input-digest DIGEST`, and
the focused refresh in one deterministic invocation. Preserve compare-and-swap;
stop on conflict. Do not precede refresh with a separate check/read sequence.

Keep syntax, declared, unresolved and external-call limits explicit. A successful
`CHECKED` capture and `MISSING_BASELINE` publication freshness are separate facts;
missing publication is not capture failure and needs no invented publication.
Report an answer delta or a short unchanged annotation, not a replay of earlier
answers, packets or error history. Full evidence remains private unless transfer
to a specific recipient is authorized; the projection is not a public-safe report.
