# Codeclew documents its shipped starter

This guide explains the real `first-document.py` shipped with Codeclew 0.13.18.
Its source is fixed at release commit
`b810c1aa5c9765f3056f763a8367c57de070e7ba`, not a synthetic task-promotion fixture.
The Python source-syntax profile retains exact committed text; it does not
resolve imports, CLI internals, dynamic dispatch or runtime effects.

## Prepare your own task

Use Python 3.11+, Git and the [installed 0.13.18 release](https://github.com/codeclew/codeclew/releases/tag/v0.13.18).
Resolve the installed `clew` launcher once. Keep the documentation directory
separate from the checkout. These commands prepare inputs for your author:

```sh
git clone --branch v0.13.18 https://github.com/codeclew/codeclew.git codeclew-example
git -C codeclew-example switch -c read-example
checkout="$PWD/codeclew-example"
launcher="$(command -v clew)"
"$launcher" docs init --root ./architecture --title "Codeclew documentation starter"
python3 -I -S architecture/examples/first-document.py --clew "$launcher" capture \
  --root ./architecture --repo "$checkout" --service clew-starter \
  --title "Prepare a first document" \
  --repository https://github.com/codeclew/codeclew \
  --language python --dialect 3.11 \
  --source-root crates/clew/assets/documentation/examples/first-document.py \
  --audience "Developers using the shipped first-document.py starter"
```

The repository registration uses the canonical web URL. The clone command uses
the Git URL. Capture selects the checkout's committed HEAD and declared roots;
later local edits are not part of that saved Check.

The result names the Work, snapshot, packet and authoring directory. Read the
complete packet and all source parts. Ask your author to explain the inputs,
capture/read alternatives, prepared files and where preparation stops. The
starter creates an incomplete proposal template; fill its summary and evidence
with actual returned Work references. A visual, if useful, must bind every node
and connection to genuine source evidence. Our [authored proposal](./proposal.json)
is an example; its references belong to the recorded Work, so do not copy them
into a different Work without checking the returned references.

## Inspect and publish the explanation

```sh
"$launcher" docs proposal submit --root ./architecture \
  --work RETURNED_WORK --input RETURNED_DIRECTORY/proposal.json
"$launcher" docs proposal publish --root ./architecture \
  --proposal RETURNED_PROPOSAL --unassessed
```

Inspect the actual native result before publication. It must be
`READY_FOR_REVIEW` or `READY_WITH_LIMITATIONS`, with no structural diagnostics.
A process exit of zero is not enough: `NEEDS_REPAIR` is a refusal to proceed.
Open the returned frozen reader bundle; rendering an older Check can replay its
older authored baseline. `--unassessed` publishes a source-bound explanation
without inventing a reviewer approval.

For a question against saved source, use the returned snapshot:

```sh
python3 -I -S architecture/examples/first-document.py --clew "$launcher" read \
  --root ./architecture --service clew-starter --snapshot RETURNED_SNAPSHOT \
  --question "What does the starter prepare, and what still needs authoring?"
```

Read mode does not acquire new source or publish an explanation.

## What was verified for this published guide

The [verification record](./record.json) binds the actual release, source bytes,
Work, proposal and native publication. All retained source parts were compared
with the exact released Git bytes, and the claims and diagram were checked
against those parts. The published native files are copied without edits.

The accepted author was a Sol 6.1 High subagent with a fresh context containing
only the recorded package. This is a bounded author-input record, not an OS
sandbox or native isolated-review receipt. A prior coordinator-authored Work
honestly registered broader untracked reads; native submission returned
`NEEDS_REPAIR` / `INCOMPLETE_INFLUENCE`, and that Work was not published. The
successful author Work reused the same saved Check and recorded its own reads.

Native meaning remains `UNASSESSED`, runtime behavior remains `UNKNOWN`, and
unwritten sections remain explicit gaps. Manual source-correctness verification
is separate from native meaning review. Capture and preparation made no model
calls; the accepted prose and visual were authored by the subagent afterward.
This case documents one shipped module, not the entire Codeclew application.
