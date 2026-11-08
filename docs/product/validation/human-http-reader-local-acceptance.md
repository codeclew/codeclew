# Human HTTP reader: local acceptance handoff

This slice extends the packaged Markdown route from 0.14.7. It does not change
the runtime, native publication schemas or official installed release. Full CI,
merge, publication and a new release are outside this local acceptance scope.

## Selected architecture

- `skills/codeclew/references/http-reader-closure.md`: same-author bounded
  route/helper/effects/outcomes review, consequential reference reads, endpoint
  inventory/pages, business objects versus representations and egress closure.
- `skills/codeclew/examples/markdown-renderer`: existing marked renderer and
  service starts/navigation. `sequence.mjs` renders explicit authored JSON into
  finite HTML/CSS lifelines/arrows and an equivalent linked text trace. No new
  dependencies, application execution, graph engine or inferred semantics.
- The `clew-sequence` contract is documented in that renderer's README. `call`
  makes no wait claim; `async`, `await`, `return` and local `note` remain distinct.
  Conditions apply only to their own row. Participant/step local links are
  optional but validated when present; an external actor needs a local authored
  description, while a client needs no fictitious page.
- Existing compose fragments bind each substantive JSON row to exact delivered
  evidence. The original Markdown, source maps and retained provenance survive
  finalize. Renderer validation assesses structure/links, never semantic truth.

## Apply separately from the official installation

Pin the accepted installed launcher at 0.14.7. Select the candidate's canonical
`skills/codeclew` directory explicitly as `CODECLEW_SKILL_DIR`; retain its exact
commit and package digest as an unreleased helper/skill delta. Do not run upgrade,
global skill installation or a source runtime. Copy only the renderer into a
new owned tooling directory and install its pinned dependency there:

```sh
cp -R "$CODECLEW_SKILL_DIR/examples/markdown-renderer" /path/to/new-renderer
npm ci --ignore-scripts --no-audit --no-fund --prefix /path/to/new-renderer
npm test --prefix /path/to/new-renderer
```

Use that copy in the explicit renderer argv JSON and use the selected candidate
helper paths for compose/bind/finalize with the already admitted launcher and
retained root/manifests. Keep the ordinary native SOURCE engine arguments where
required by the existing evidence kind. No new acquisition is needed. A successful
export check remains mechanical, not independent content/UX acceptance.

## Reproduce the public owned fixture

From the candidate checkout, with Python 3.11+ and Node 20+:

```sh
python3 -I -S scripts/test_human_http_reader.py \
  --renderer /path/to/new-renderer --output-dir /path/to/new-fixture-run
```

The test creates a synthetic SOURCE_SYNTAX Work receipt fixture using
`scripts/fixtures/human-http-reader/Intake.java`; it performs no real capture or
compiler verification. Real compose, bind, renderer and finalize/export helpers
then produce six pages with forty bound claims and `EXPORT_CHECKED`. The test
checks citation URL expansion, per-step map preservation, distinct endpoint
starts, local links, diagram output and absence of invented await steps. All
receipts and the initial original survive under the chosen output directory.

Serve its `final/html` locally and independently answer:

1. Which handler starts POST v1 and PUT v2? Does each inventory/start link open
   its own page and branch of the shared process, with a route back?
2. Which version validates positive quantity? Which inputs reach the Stock API,
   through which helper, under which flag key/default/provider call chain?
3. What exactly does the test predicate read? Which entry can send the logical
   topic, what payload does it send, and is its returned stage awaited?
4. Which immediate throws prevent the result? What is known about later stage
   failure, broker ACK/retry and deployment addresses? Unknown physical settings
   must not erase the known topic, key/default or logical operation.
5. Do the object fields have readable meanings with exact identifiers and a
   separate representation section? Does every sequence integration appear in
   the catalogue with a functioning local description link?
6. At 1280 and 390 px, inspect the default plus middle/end steps. On narrow
   screens, sender/message/receiver/condition must remain one readable block,
   with no page overflow or required diagram panning. Long self-step labels must
   not collapse to half a participant column. Open secondary source details and
   preserve original text/maps; a screenshot alone does not establish meaning.

Focused checks: renderer `npm test`; `scripts/test_human_http_reader.py` above;
`python3 -I -S scripts/test_agent_skill.py`, `scripts/test_page_compose.py`,
`scripts/test_reader_export_check.py`, `scripts/test_reader_plan_preflight.py`,
`scripts/check_english_content.py`, and `scripts/check_repository_privacy.py`.
The canonical skill and both discovery copies must remain byte-identical.
