# A first useful source document with Clew 0.13.11

Question: **What does `--check` verify, and when does it invoke Graphviz?**

This case uses the official installed Clew 0.13.11 and the public `v0.13.11` source revision `d91dbec1164e0601d47c221ab97c504033ce858e`. It selects the complete `scripts/build_cli_documentation.py` file. No application build, Graphviz installation, configured provider or paid driver is needed to capture this Python source and author the overview with the current agent.

The initial answer is in the first immutable native publication. The current reader also includes a clearly labeled caller-owned local maintenance exercise. That follow-up commit is not part of the released Codeclew application. See `record.json` for the actual runtime, Work, publication and preservation observations; `saved-source-answer.md` answers the source question against the original release capture.

## Prepare and publish one overview

Use Git, Python 3.11+ and installed `clew` 0.13.11. Choose a new local documentation root; do not write into this static example.

```sh
clew --version
git clone https://github.com/codeclew/codeclew.git release-checkout
git -C release-checkout checkout --detach v0.13.11
docs="$PWD/first-document"
repo="$PWD/release-checkout"
clew docs init --root "$docs" --title 'What --check verifies and when Graphviz runs'
python3 -I -S "$docs/examples/first-document.py" capture \
  --root "$docs" --repo "$repo" --service cli-documentation \
  --title 'CLI documentation generator' \
  --repository https://github.com/codeclew/codeclew \
  --language python --dialect 3.11 --source-root scripts/build_cli_documentation.py
```

The starter prints its actual `output`, `packet`, `work` and `snapshot`. The packet contains coherent authoring context and the complete selected retained source. Give that packet to the current agent with `authoring-request.md`. Edit its provided `proposal.json`; use the actual operation and supporting SOURCE references from this Work. Inspect `check.json`: a first missing baseline can accompany captured source; an unresolved source is not a successful capture.

Set these variables from the actual returned values:

```sh
packet_dir='ACTUAL_OUTPUT_DIRECTORY'
work='ACTUAL_WORK'
original_snapshot='ACTUAL_SNAPSHOT'
clew docs proposal submit --root "$docs" --work "$work" --input "$packet_dir/proposal.json"
```

Inspect the returned structure, limitations and proposed overview. This case returned `READY_WITH_LIMITATIONS`. Set `proposal` to its returned proposal ID, then explicitly publish locally:

```sh
proposal='ACTUAL_PROPOSAL_ID'
clew docs proposal publish --root "$docs" --proposal "$proposal" --unassessed
```

Open `$docs/docs/index.html` and its source citation. This case produced one useful overview, `PARTIAL` publication, `UNASSESSED` meaning, `UNKNOWN` runtime and visible unwritten gaps. Local publication is not separate meaning review or verification of a production generator run.

## Ask against saved source

```sh
python3 -I -S "$docs/examples/first-document.py" read \
  --root "$docs" --service cli-documentation --snapshot "$original_snapshot" \
  --question 'What does --check verify, and when does it invoke Graphviz?'
```

Read this new packet and answer separately. Do not submit or publish merely to ask a source question. The observed helper trace contains Work preparation/read/read-part stages, no acquisition or publication stages, and the current reader bytes did not change.

## Preserve a note and another section

From this reproduction directory, import the included owner note using the current input digest from `clew docs service list --root "$docs"`:

```sh
input_digest='ACTUAL_CURRENT_INPUT_DIGEST'
clew docs note import --root "$docs" --input inputs/owner-note-association.json \
  --source inputs/owner-note.md --expected-input-digest "$input_digest"
clew docs recompose --root "$docs" --snapshot "$original_snapshot"
```

Use the returned recomposed snapshot with the small case helper:

```sh
derived_snapshot='ACTUAL_RECOMPOSED_SNAPSHOT'
python3 -I -S prepare-responsibilities.py --root "$docs" --snapshot "$derived_snapshot"
```

Read that complete packet. Author only the responsibilities target using its actual section and SOURCE references; explain byte-binding checks, static presentation and the meaning/runtime limits. Submit and publish with the same supported commands and `--unassessed`. Save `clew docs section show --root "$docs" --service cli-documentation --id section-responsibilities`, the exact owner note bytes, and hashes of every file in the two existing generated bundles before updating.

## Caller-owned local maintenance exercise

Create a separate clone at the same released source revision. This exercise changes the saved diagram check to reject XML whose root is not an SVG element. The focused test uses synthetic minimal retained artifacts; it forbids subprocess calls and checks that saved artifact bytes remain unchanged. It does not run the real public-site generator.

```sh
git clone https://github.com/codeclew/codeclew.git exercise-checkout
git -C exercise-checkout checkout --detach v0.13.11
exercise="$PWD/exercise-checkout"
case_dir="$PWD"
python3 -I -S maintenance-test.py "$exercise/scripts/build_cli_documentation.py"
# Expected: the non-SVG rejection test fails on released source.
git -C "$exercise" apply --check "$case_dir/maintenance-exercise.patch"
git -C "$exercise" apply "$case_dir/maintenance-exercise.patch"
python3 -I -S maintenance-test.py "$exercise/scripts/build_cli_documentation.py"
# Expected: both tests pass after the exercise change.
git -C "$exercise" diff -- scripts/build_cli_documentation.py
git -C "$exercise" add scripts/build_cli_documentation.py
git -C "$exercise" -c user.name='Codeclew Maintainers' \
  -c user.email='maintainers@codeclew.invalid' commit \
  -m 'exercise: reject a non-SVG root in the saved diagram check'
git -C "$exercise" rev-parse HEAD
```

The new commit differs for each reproduction. Read `clew docs service show --root "$docs" --id cli-documentation`; save its record as an update input. Change **only** `targetRef` to the actual local exercise commit and `sourceLinkTemplate` to a static source route you will serve:

```text
https://YOUR_STATIC_HOST/exercise-source/{revision}/{file}.html
```

This example uses `https://codeclew.github.io/codeclew/examples/current-workflow/reproduce/exercise-source/{revision}/{file}.html`. A fresh local commit is not on GitHub: do not give it an invented GitHub blob URL. Preserve repository/service identity, source roots, language/dialect and profile. Use the current returned `inputDigest` to register the update, then bind and capture the separate clone:

```sh
input_digest='ACTUAL_SERVICE_SHOW_INPUT_DIGEST'
clew docs service add --root "$docs" --input updated-service.json --expected-input-digest "$input_digest"
clew docs bind --root "$docs" --service cli-documentation --repo "$exercise"
clew docs check --root "$docs" --service cli-documentation
```

Use the new snapshot with the shipped starter:

```sh
changed_snapshot='ACTUAL_CHANGED_SNAPSHOT'
python3 -I -S "$docs/examples/first-document.py" read \
  --root "$docs" --service cli-documentation --snapshot "$changed_snapshot" \
  --question 'Preserve the existing overview and explain the local exercise change: --check now rejects a non-SVG XML root. Keep the responsibilities section and owner note unchanged.'
```

Read the retained overview and complete new source. Preserve the initial summary as an exact prefix; append only the local change and cite this new Work's SOURCE references. Preserve any existing steps or visuals. This case has none.

Generate the exact escaped source mirror/raw-source pair at the configured route using the actual new packet:

```sh
python3 -I -S render-exercise-source.py --clone "$exercise" \
  --revision ACTUAL_LOCAL_COMMIT --release-revision d91dbec1164e0601d47c221ab97c504033ce858e \
  --packet-dir ACTUAL_UPDATED_PACKET --output-root exercise-source
```

The helper verifies committed blob bytes against native retained text, digests and ranges. Submit and explicitly publish **only** the updated overview with `--unassessed`. Compare the responsibilities `content` JSON, owner note bytes and prior bundle file hashes with the saved baseline. Retained unrelated prose is not automatically refreshed or semantically reverified when source changes. Serve the complete new native docs tree and the exact exercise source mirror together; do not hand-edit immutable native outputs.

## Publication files and checks

All 100 native docs files were copied byte for byte. The ordinary reader routes, current bundle navigation, source mirror anchors and pinned released source ranges were checked locally; browser and deployment checks are separate. Each bundle also retains `root-overview.html`, the native root restore payload: its relative links assume restoration to the docs root, so it is not a directly navigable bundle reader. No reader links to that payload.

## Create an explicit release checkpoint

After the overview update, this case used the saved exercise snapshot to add one checkpoint to released history:

```sh
clew docs render --root "$docs" --snapshot "$changed_snapshot" --language en --publish
```

This native command does not recapture source or run a provider. It created a new bundle with `released: true`, ordinal 1, and one entry in History. The three earlier working bundles remain `released: false`; all 71 of their files are unchanged. The checkpoint keeps both authored sections, the imported owner note, `UNASSESSED` accepted meaning, `UNKNOWN` runtime and nine explicit gaps. Release is a retention checkpoint, not a meaning review. See the exact native result and integrity inspection in `observed/release-checkpoint-render.json` and `observed/history-show-checkpoint.json`.

The checkpoint was emitted by the official installed 0.13.11. The source UX fix that explains an empty release history is separate and is not claimed to be present in these installed-release output bytes.

## Reader refresh with installed Clew 0.13.13

The same saved snapshot was rendered through official installed Clew 0.13.13 in RELEASE mode. All 95 existing bundle files and caller notes remained byte-identical. History now says **Released documentation snapshots**. The current bundle is `0a9a2804909f09fe7b37eba66f0411d7d7f71aea955bdb4f4c6157e3042b240b`. The refresh performs no new source acquisition or model review; the source, authoring and original checkpoint remain the recorded 0.13.11 case. Read `rendererRefresh01313` in `record.json` and the separate exact native render/history responses under `observed/`.
