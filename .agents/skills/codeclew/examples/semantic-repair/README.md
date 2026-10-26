# Owned semantic handoff exercise

Run with Python 3.11+ and a fresh absolute caller-owned output directory:

```sh
python3 -I -S SKILL/examples/semantic-repair/run_owned.py --output /ABSOLUTE/NEW/owned-run
```

The four cases model an omitted guarded candidate effect, reversed response
helper order, a source-known literal incorrectly described as unavailable, and
a whole-choice summary whose leading predicate contradicts its first child path.
Initial Work pages expose SOURCE and SYMBOL navigation. The scripted reviewer
adds a separately saved, same-Work source acquisition. Repair contexts contain
only addressed old fragments and necessary exact source. The helper assembles
the full candidate locally; an unchanged sibling operation and runtime/dispatch
uncertainties remain exact.

The literal case corrects both the operation claim and an explicitly allowed
existing root `/uncertainties/0` string. The unrelated runtime uncertainty at
index one stays exact; no uncertainty is added, deleted or reordered by repair.

The polarity case delivers the source condition and both child paths as one
bounded group. Its first child and alternative return already agree with source;
only the header meaning changes. Both child arrays, evidence references and the
unrelated operation remain exact. Compare a source-true and a source-false path
with the proposed header. This is an owned transport/repair counterexample, not
a natural-language predicate classifier or evidence that a model passed it.

The exercise uses a **fake CLI transport and scripted review/replacements**.
It does not invoke Luna, another model, the installed runtime, source capture or
publication. Its positive classifications demonstrate mechanical lifecycle and
carrier behavior only. An independent real model review can still reject any
candidate. Test expectations are software-fixture controls, not a claim that a
model found the defects.

Each case retains `handoff/`, `repair/first-review.original.json`,
`repair/repair-context.json`, `applied/` with the full candidate and raw native
shapes, and `final/result.json`. Native status stays UNASSESSED while the external
scripted meaning verdict is recorded separately. `native-calls.jsonl` shows one
full submit and one proposal-page read. No prepare or publish is performed.

The repository regression suite adds stale base, foreign Work/snapshot,
missing/ambiguous evidence, altered acquisition, wrong reviewer, disallowed
targets, dropped references, insufficient evidence, native failure and single
attempt negatives, plus same-Work wrong-proposal/mixed metadata at finish and
invalid uncertainty indices/types.

The suite also checks native step-text boundaries at 1024/1025 UTF-8 bytes with
ASCII, two-byte characters and four-byte emoji. Oversized nested or unchanged
steps produce exact JSON-pointer/byte diagnostics before any native dispatch;
the complete assembled input is saved and the single attempt remains unclaimed.

Run the suite with:

```sh
python3 -I -S scripts/test_semantic_handoff.py
```

For debug, retain this same fresh output directory and inspect the original
context, addressed old/new values and first failed guard. This mode has the same
mechanical outcome; it never changes a verdict or grants a retry. For actual
documentation, follow [the manual handoff route](../../references/semantic-repair.md)
with the installed admitted runtime and separately assigned real author/reviewer.
