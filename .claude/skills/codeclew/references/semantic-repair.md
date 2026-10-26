# Independent semantic review and local repair

Use this route for an already submitted **manual Proposal** on the same immutable
Work. It adds an external reviewer handoff and guarded local assembly. It does
not call a model, infer source semantics, publish, or create native meaning-review
authority. Choose the existing `docs work review-draft` route for an eligible
saved operation draft and configured driver. An operation answer and a Proposal
are different carriers. Preserve an installed profile refusal; keep the manual
Proposal route on the admitted installed runtime.

`scripts/semantic_handoff.py` reuses `work_session.py`, `work_evidence.py` and
`collect_proposal.py`. The finite path is one initial independent review, at most
one addressed repair, one full native submit/collect, and one affected review.
There is no retry loop or provider fallback. A failed/uncertain outcome remains
saved. Another attempt requires an explicit new handoff and caller budget; never
hide the previous failure. Evidence-only RETURN append uses
[the narrower repair helper](evidence-repair.md).

## Prepare the review

All inputs and outputs stay under one absolute caller-assigned directory.
Relative paths resolve there, independently of shell cwd. Outputs must be fresh.
Start with the complete output of `work_session.py submit`: `input.original.json`,
`submit.command.json`, `submit.stdout`, and `submit-collection/` with every native
page and `items.json`. The native archive is caller-owned evidence, not a signed
attestation of execution. The helper verifies its binding, page identities,
cursor chain, counts and complete rows. It preserves every native diagnostic.
No raw source or full proposal needs to be printed into the conversation.

Create `request.json` in that directory (replace example identities and paths):

```json
{
  "schema": "codeclew-semantic-handoff-request/1.0",
  "work": "WORK",
  "workPage": "work-initial/page-0001.stdout",
  "nativeAttempt": "submit-before",
  "scope": "Explain one selected operation and its available local candidates; runtime dispatch and durable completion remain unproven.",
  "pages": ["work-initial/page-0001.stdout"],
  "collections": ["selected-source/read-collection"],
  "allowedPaths": ["/operations/0/summary", "/operations/0/steps/2"],
  "maxRepairs": 1,
  "author": {"id": "author-session", "model": "gpt-6-luna", "reasoning": "max"},
  "reviewer": {"id": "independent-reviewer-session", "model": "gpt-6-luna", "reasoning": "max"}
}
```

Choose the smallest allowed narrative subtrees with the caller. Existing
`summary`, `steps`, `explanation` and `contracts` subtrees can be replaced. A
specific existing root string at `/uncertainties/INDEX` can also be replaced when
explicitly allowed and addressed by the review. Its replacement must stay a
string; adding, deleting, reordering or replacing the uncertainties array is
unsupported. Other uncertainty strings remain exact. Root schema, operation
identities and unrelated operations are outside this route.
The author is explicitly Luna Max; the independent reviewer uses the caller's
selected model. Different IDs record the assignment; they do not prove process
isolation or which model really executed. Enforce isolation in the host session.

For a full-question semantic reader task, add optional `readerScope` to this
request with `path` and the exact earlier file `digest`, using the
[bounded original question/candidate inventory](semantic-reader-contracts.md).
Prepare binds that file in the existing tracked inputs and includes its original
question and source pins in review/repair/affected-review contexts. It retains
the same digest across native submission and publication/model phases; a changed
inventory blocks this handoff instead of silently narrowing it. This adds no
planner, model dispatch or semantic approval. Run the reader preflight on the
final assembled documents and explicit candidate dispositions before export.
Optional `readerScope.coverage: {"path":"reader-coverage.json", "digest":"FILE_SHA256"}`
also pins and carries an existing disposition map bound to that original scope;
the handoff does not treat these declared dispositions as validated semantics.

```sh
python3 -I -S SKILL/scripts/semantic_handoff.py prepare \
  --input-root OUTPUT_ROOT --input request.json --output review-01
```

Give a **separate reviewer** `review-01/review-context.json`. Initial full review
includes the exact candidate, SHA-256 binding, Work/snapshot, bounded scope,
complete native status/diagnostics and a bounded navigation view. Read the actual
SOURCE with `work_evidence.py`, not an author sketch. Follow SYMBOL
`sourceReferences`, unread bodies, constants, source order and relevant candidate
effects. An unknown call receiver does not make an available body unavailable.
Unknown runtime selection/order/completion can remain precisely scoped unknowns.

The navigation view can have `nextCursor`, truncated text and unread source parts.
Use its original page/record locations and the existing view's `--cursor`,
`--offset`, `--reference` or literal `--query`. Acquire missing evidence through
recorded `work_session.py read` on the same Work, following native SOURCE part
commands when necessary. Save acquired Work pages/parts in the assigned output
root. No new prepare or source capture is needed. Register those exact pages in
the review sidecar with their raw file SHA-256. Foreign Work/snapshot, conflicting
references, incomplete source text and changed bytes block repair.

## External review sidecar

Use `codeclew-semantic-handoff-review/1.0`. Copy `binding`, `reviewScope`, `reviewer`
and the SHA-256 of the exact review-context bytes into `contextDigest`. The
helper uses the existing native verdict vocabulary (`APPROVE`, `REJECT`,
`NEEDS_EVIDENCE`) and issue fields `severity`, `claim`, `reason`, `evidence`.
Additional JSON pointers address the manual Proposal; they do not become native
claim IDs. This sidecar is deliberately **not**
`codeclew-documentation-review/1.0`: manual external review cannot mint the native
dispatch evidence digest, claim coverage, reviewer driver provenance or approval.

```json
{
  "schema": "codeclew-semantic-handoff-review/1.0",
  "binding": {"copy": "the exact context binding object"},
  "contextDigest": "sha256:EXACT_CONTEXT_BYTES",
  "reviewScope": "FULL_PROPOSAL",
  "reviewer": {"id": "independent-reviewer-session", "model": "gpt-6-luna", "reasoning": "max"},
  "verdict": "REJECT",
  "assessedOperations": ["EXACT_ENTRYPOINT_IDS_FROM_assessOperations"],
  "acquiredPages": [{"path": "reviewer-read/page-0001.stdout", "digest": "sha256:EXACT_PAGE_BYTES"}],
  "inspected": [{"reference": "s12", "textDigest": "sha256:EXACT_SOURCE_TEXT"}],
  "issues": [{
    "severity": "ERROR", "claim": null,
    "path": "/operations/0/summary",
    "reason": "The available candidate body supports a guarded effect omitted by this account.",
    "evidence": ["s12"],
    "neededReads": []
  }],
  "limitations": ["Runtime candidate dispatch and durable completion remain unproven."]
}
```

Every cited source must have a matching `inspected` text digest. These are
acknowledgments, **not proof that a model read or understood the text**. A blocked
review can return `NEEDS_EVIDENCE` without inspected source, with an actionable
issue and `neededReads` containing native Selection objects. It ends the handoff
without a repair. A negative review needs an issue; approval cannot contain an
unresolved `ERROR`. Check all material claims and relevant open leads in the
declared scope; pointer/digest coverage cannot establish semantic completeness.
Initial APPROVE/REJECT must copy every `assessOperations` ID into
`assessedOperations` in order. An early NEEDS_EVIDENCE can refuse without claiming
that full coverage.

```sh
python3 -I -S SKILL/scripts/semantic_handoff.py repair-context \
  --input-root OUTPUT_ROOT --input review-01 \
  --review first-review.json --output repair-01
```

The helper retains the first raw review. On `REJECT`, it emits
`repair-context.json` with only affected old fragments, their issues and complete
selected source evidence. It excludes the full base and unrelated operations.
At most 32 nonoverlapping target subtrees and 256 KiB of repair context are
supported. Each source body is bounded to 64 KiB. If that is too large, narrow
the selected evidence and target scope explicitly; do not silently truncate it.

## Repair, assemble and validate

The context includes `nativeConstraints.stepMeaningText`: every step's
`meaning.text` must be **at most 1024 UTF-8 bytes**, recursively through
`children` and `otherwise`. This is a byte limit, not a character count: 512
copies of a two-byte character fit; adding one ASCII byte exceeds it. The shared
claim schema allows longer general claim text, but does not override the stricter
native sequence-event limit for steps. Preserve conditions, effects and evidence
when editing; never truncate automatically. Extended detail belongs only in
already permitted supporting targets, not a silently added fragment.

Give only that repair context to the assigned Luna Max author. It returns:

```json
{
  "schema": "codeclew-semantic-replacements/1.0",
  "binding": {"copy": "the exact repair-context binding object"},
  "contextDigest": "sha256:EXACT_REPAIR_CONTEXT_BYTES",
  "author": {"id": "author-session", "model": "gpt-6-luna", "reasoning": "max"},
  "replacements": [{
    "path": "/operations/0/summary",
    "old": {"text": "exact original text", "evidence": ["s1"]},
    "new": {"text": "corrected bounded account", "evidence": ["s1", "s12"]}
  }]
}
```

Return every addressed target once, with its complete exact old value. No other
path is allowed. Existing evidence reference occurrences cannot be removed;
new references must come from the selected complete source evidence. If correcting
an erroneous binding requires removing one, stop and request an explicitly
reviewed broader change outside this conservative helper. It never silently
rebases or drops bindings. Untouched JSON subtrees remain equal, including all
branches outside the targets. JSON serialization can change whitespace; original
full base bytes are preserved separately.

```sh
python3 -I -S SKILL/scripts/semantic_handoff.py apply \
  --input-root OUTPUT_ROOT --input repair-01 --patch replacements.json \
  --launcher /ABSOLUTE/INSTALLED/clew --root DOCUMENTATION_ROOT --output applied-01
```

All local guards run before submit. The helper assembles `input.original.json`
locally and saves it even if the known step-text preflight fails. Its
`local-preflight.json` checks every operation's steps, including unchanged steps
and nested branches. An oversized step returns `BLOCKED` with
`STEP_TEXT_UTF8_LIMIT_EXCEEDED`, the exact JSON pointer, `actualBytes` and
`limitBytes`. All oversized paths are listed in the report. No native command
is invoked and the single dispatch is not claimed. Correct the explicit input
in a fresh output directory; failed artifacts remain preserved.

This is a partial structural preflight. `PASS_KNOWN_STEP_TEXT_LIMIT` is neither
native READY nor meaning approval; other native schema and structural checks
remain necessary. Official 0.13.52 may report oversized step text as
`STRUCTURE_OR_COVERAGE_INVALID` / `unsafe or duplicate sequence fragment ID`
because its render check combines ID validity/uniqueness with this text limit.
That legacy message alone does not establish duplicate IDs. This helper provides
the precise local byte diagnostic without changing the admitted runtime.

After local guards pass, the helper submits that **full Proposal to the same
Work** and collects all native pages. It claims the one permitted dispatch before
invoking the CLI. A timeout
or failure consumes that attempt, preserving raw output with no automatic retry.
No model is invoked by this command; configured model execution remains the
host's responsibility. Do not replace the installed CLI with a source build.

Give the separate reviewer `applied-01/affected-review-context.json`. It contains
old/new targets, first issues, original limitations, necessary source and whole
native feedback. The reviewer must reassess every affected path and first issue.
Return the same review sidecar, now with `reviewScope: "AFFECTED_FRAGMENTS"`, its
new exact binding/context digest, all selected source acknowledgments, and
`assessedPaths` equal to the listed target paths in order. A repair that still
omits a supported effect or misorders a helper must stay `REJECT`.

```sh
python3 -I -S SKILL/scripts/semantic_handoff.py finish \
  --input-root OUTPUT_ROOT --input applied-01 \
  --review affected-review.json --output final-01
```

`REVIEWED_BOUNDED_CANDIDATE` requires native READY/READY_WITH_LIMITATIONS and
external affected APPROVE after initial full review. It is a local handoff
classification. `nativeMeaningReview` remains unchanged, typically UNASSESSED;
the helper never sets native PASS/VERIFIED or publishes. Native NEEDS_REPAIR,
external REJECT or NEEDS_EVIDENCE blocks acceptance. The final native proposal ID
must equal the origin/reviewer binding, and its complete identity, status,
meaning classification and diagnostics must equal the native result shown in
the exact affected review context. A mixed or changed archive blocks finish
even if its replacement pages are internally consistent and from the same Work.
This is bounded external
review, not universal completeness or runtime proof.

## Owned exercise and debug

Run the [owned exercise](../examples/semantic-repair/README.md) before real model
acceptance. It covers omission, source order and false unavailable literals with
scripted reviewers and a fake native transport. It is explicitly not real model
evidence. Installed native validation and real reviewer quality are separate
checks. Do not infer costs, token savings, reading or provider execution from it.

For `debug=true`, use a new caller-owned output root and keep request, initial
context, first review, repair context, replacements, full local base/candidate,
raw native stdout/stderr/commands and affected review. Those files are already
written by the normal commands. Add a short caller-owned decision note explaining
the failing guard or evidence gap; debug does not change validation, model choice,
publication permissions or the finite attempt count. Never put diagnostic
artifacts inside the installed skill folder or overwrite a failed stage.
