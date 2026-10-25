# Bounded evidence-only repair

Use `scripts/repair_evidence.py` only when the full proposal already states an
exact delivered FLOW RETURN text and its existing `meaning.evidence` is missing
that FLOW reference. The helper changes those arrays alone. It uses the admitted
installed launcher, rereads the exact references, preserves raw pages/receipts,
submits the original to obtain actual diagnostics, then submits and collects the
entire candidate. It never calls a model, captures source or publishes.

Choose one existing caller-owned output root. Keep the base and intent there;
resolve every orchestration input from it, including any nested run directory.
The helper resolves relative base, intent and output paths from `--input-root`,
not the shell's current directory, and refuses paths outside that root.

```sh
python3 -I -S <skill-folder>/scripts/repair_evidence.py \
  --launcher /absolute/installed/clew --root /absolute/documentation-root \
  --input-root /absolute/assigned-output-root \
  --base proposal.json --intent repair-intent.json --output evidence-repair-01
```

The output directory must be new. A repeated or stale input is not accepted
because a reference already appears in it. There is no automatic rebase or
existing-result approval shortcut. Original input bytes and every native failure
remain in the new directory; native state and host files are separate writes.

Supply this helper-specific intent, with real values from that exact saved Work.
These placeholders are not native evidence. `baseDigest` is SHA-256 of the exact
base file bytes, prefixed with `sha256:`. `recordDigest` is SHA-256 of the complete
native row's `record` serialized as UTF-8 JSON with sorted keys, compact separators
and unescaped Unicode; it is distinct from any inner observation digest.

```json
{
  "schema": "codeclew-evidence-only-repair-intent/1.0",
  "work": "EXACT_WORK_ID",
  "snapshot": "EXACT_SNAPSHOT",
  "baseDigest": "sha256:EXACT_FILE_HASH",
  "targets": [{
    "operation": 0,
    "stepPath": [{"list": "steps", "index": 0}, {"list": "children", "index": 0}],
    "expectedEvidence": ["EXISTING_REFERENCE"],
    "expectedText": "return null",
    "reference": "DELIVERED_RETURN_REFERENCE",
    "recordId": "EXACT_NATIVE_FLOW_ID",
    "recordDigest": "sha256:EXACT_RECORD_HASH",
    "mapping": "VERBATIM_FLOW_RETURN"
  }]
}
```

At most eight exact target arrays are admitted. The step must already be a return;
its text must equal the delivered RETURN fact verbatim. The old evidence array,
Work, snapshot, full record identity and actual native FLOW diagnostic are checked.
A generic coverage message alone does not choose either target. Text, guards,
children/otherwise, order, uncertainties and all other fields stay unchanged.
Syntax authority stays syntax authority, including unresolved runtime boundaries.

This first implementation deliberately stops at paraphrased, ambiguous or
non-RETURN mappings with a concrete `NEEDS_CONTEXT` reason. Read the returned
FLOW's `sourceReferences`, enclosing branch and relevant receiver/downstream
bodies through the existing Work route; retain whole branches and returns.
Use the existing author/reviewer route when meaning needs repair. Do not shorten
an isolated author payload or treat this helper as a semantic repair engine.

`NATIVE_CANDIDATE_RECORDED` means the whole candidate was checked, including
unrelated operations. Inspect `nativeStatus` and all collected diagnostics:
`NEEDS_REPAIR` still blocks publication. `READY_WITH_LIMITATIONS` and `UNASSESSED`
remain separate from independent meaning acceptance. The full host archive is
not proof that a model received every page; record actual model-delivered context
separately. Zero repair model calls excludes neither acquisition nor review cost.
