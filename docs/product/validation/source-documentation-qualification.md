# Source documentation implementation and bounded qualification

Date: 2026-09-12. Implementation revision: `a74ba0d880a03a4ef6aae7904421d3b16244e26a`.
Status: implemented and locally verified development change; not a published release
or a qualification of every research-plan acceptance case.

## Result and decisions

Python, Java with missing build dependencies, and Kotlin 1.9 without K2 now share
the existing documentation model through `source-syntax`. Source capture, authored
explanations, exact source links, freshness checks, optional compiler observations,
and atomic output publication are available through the normal `docs` commands.
The [operational guide](../../operations/source-documentation.md) documents the
interface and boundaries; [fixtures](../../../fixtures/durable-docs-source/README.md)
provide reproducible inputs.

The agreed sequence from PRs [#6](https://github.com/codeclew/codeclew/pull/6) and
[#7](https://github.com/codeclew/codeclew/pull/7) produced these outcomes:

| Stage | Executed result | Qualification limit |
|---|---|---|
| M0-M2 | Released 0.7.1, native, and manual oracle comparison; 15 accepted answers | Small synthetic corpus, parent grading |
| D0-D4 | Source admission, three grammars, common narratives, conservative freshness | Focused deterministic tests; broader mutation matrix is not claimed complete |
| D5 | Optional javac/K2 observations attached to the same source roots | No proven reduction in ambiguity or review fan-out |
| D6 | Before/after change dossier, retained claims/text, existing atomic renderer integration | Bounded payloads and conservative scope context; review remains necessary |
| M3 | Opt-in compact projection, direct qualified selection, exact drill-down | Small paired token effect; raw remains the default |
| M4-M6 | Deferred under the measurement condition | No general planner, routing or automatic oracle selector added |
| Qualification | Full local CI path, compiler acceptance, public CLI mutation test, rendered examples, ten paired model runs | No release qualification or population-level agent result |

The [first-stage result](source-documentation-value-first-stage.md) remains
unchanged: native 279,095, released Codeclew 715,286, manual oracle 87,704 logical
tokens. The oracle's 68.6% downstream reduction is not automatic product economics.
The historical automatic-preloading regression also remains part of the evidence.

## What documentation can now do

A registered committed Git scope can supply declarations, class/property/import
context, lexical decisions and calls, and exact retained snippets without executing
project code, a compiler, or dependency resolution. The baseline includes Kotlin
safe access, Elvis expressions, `when`, lambdas and suspend/extension declarations.
A source declaration match has syntax authority; it is not compiler call resolution.

One reviewed narrative can produce offline HTML, Markdown, Mermaid, sequence and
optional overview/state views. Existing source inspection controls expose exact
revision/file/line evidence. The CLI discovers callable declarations and lets an
author explicitly mark unimplemented explanations as gaps. It does not automatically
produce or verify arbitrary business prose.

Logical declaration identity is independent of its immutable snapshot occurrence.
Blank-line relocation can rebind links without rewriting the explanation. Scope
fingerprints conservatively cover helpers, configuration and new files. Every
source-based fragment watches each involved service's declared scope. This
protects against missing obvious influence while allowing over-invalidation within
that scope; arbitrary reads outside it are not tracked.

`docs changes` supplies old claims, affected view IDs, changed facts, before/after
snippets and focused expansion references. Older bundles without retained text
report unavailable history. Changed explanations still need review and a narrative
bound to the new context. All generated views publish as a single versioned bundle;
failed rendering preserves the old pointer and manual content.

Optional semantic providers attach only through unique equal-revision source
mappings. Provider loss leaves syntax readable and makes semantic dependants require
review. Unmapped/synthetic facts do not acquire invented links. Kotlin 1.9 syntax
needs no K2; optional K2 evidence retains the packaged analyzer's language/API 2.0
compatibility boundary. Source call edges are not silently upgraded by this overlay.

## Executed correctness and rendering evidence

Seven focused syntax tests passed, covering three-language extraction, exact Unicode
snippets, relocation, meaningful literal/Python nesting fingerprints, committed
scope isolation, helpers/configuration/additions, partial parse evidence, optional
semantic overlay, duplicate identity handling, and partial/empty catalogue freshness.
These are grouped test cases, not independent repositories or a complete count of
the proposed mutation matrix.

The public CLI no-build acceptance passed with Java, javac, Maven, Gradle, Kotlin,
Python, Cargo, curl, wget and Node blocked in its analysis PATH. It creates all three
fixture repositories, authors source-bound explanations and gaps, compares compact
and raw selected flow identities, publishes deterministically, rebinds relocation,
invalidates only the changed Python service after its helper changes, inspects the
old/new dossier, and checks failed publication/manual-content preservation.

Actual javac recovery passed on the same source roots after restoring the missing
type. The actual K2 acceptance also enriched syntax roots from its retained compiler
facts, preserved those roots, retained analyzer compatibility boundaries, and checked
readability after simulated provider loss. These are fixture-level provider checks,
not native Kotlin 1.9 execution or generalized cross-provider equivalence.

The full `scripts/ci-verify.sh` path passed across an initial run and a continuation
from its sole failure: the portable skill reference changed, so the embedded-skill
digest expectation needed updating. The already successful prefix was retained; the
remaining suffix passed after that test expectation correction. This covered static
checks, focused and existing documentation tests, JVM workers, managed CLI,
operations, bootstrap, privacy and the development usability smoke. No successful
expensive check was repeated solely to obtain another receipt.

An offline showcase was then built through the supported source launcher against
the implementation revision: three service pages, three authored `reserve`
operations and eight explicit authoring gaps. It includes domain prose, a source-bound
overview and detailed lexical events. Browser inspection confirmed readable output
and exact Kotlin source inspection. It is intentionally PARTIAL, not a complete
service manual. Showcase generation made no model API calls; the parent authored
and reviewed the explanations outside measured model arms.

## Paired compact projection experiment

The separate protocol was frozen after implementation, leaving the original study
untouched. Both arms used the same source-built implementation, retained compiler
capture, five original tasks, concise retrieval guide, native continuation,
`gpt-6-astra` / high, Codex CLI 0.153.4 and 300-second deadline. Only the requested
context format differed. Qualified declaration selection was available in both.
Order alternated by task, with one repetition. No build overlapped a model arm.

| Task | Raw tokens | Compact tokens | Answers |
|---|---:|---:|---|
| E1: Kafka topic/key | 60,500 | 60,438 | Both PASS |
| E2: import quantity paths | 108,550 | 90,573 | Both PASS |
| E3: ambiguous receive | 86,082 | 82,798 | Both PASS |
| E4: checkout client path | 112,571 | 116,080 | Both PASS |
| E5: declared two-service checkout | 123,014 | 118,643 | Both PASS |
| **Total** | **490,717** | **468,532** | **5/5 per arm** |

Parent source review accepted all 19 frozen obligations per arm, with no observed
critical miss or false-exact claim. Review was neither independent nor blinded.
Every corpus HEAD and working-tree status remained unchanged. Every per-round
usage sum reconciled with its final client totals.

| Measurement | Raw | Compact |
|---|---:|---:|
| Input tokens | 483,394 | 461,773 |
| Cached input, included above | 349,952 | 320,768 |
| Output tokens | 7,323 | 6,759 |
| Model rounds | 20 | 19 |
| Commands / failed commands | 20 / 1 | 17 / 0 |
| Reported tool-output bytes | 272,326 | 254,031 |
| Model-arm elapsed time | 293.003 s | 273.660 s |

Compact used 4.52% fewer logical tokens in this sample. E2 raw first requested the
unqualified class `ImportController` and received `INVALID_INPUT`; its failed call
and recovery remain charged. Excluding the entire E2 pair for descriptive sensitivity
only, the other four pairs differ by 1.10%. E4 compact used more tokens. This is weak,
mixed evidence for aggregate token savings, not a causal estimate of serialization
alone or a reason to promote a general planner. Native continuation still supplied
missing field, DTO or unannotated declaration context.

Source-launcher preparation took 54.57 seconds; warm compiler capture took 17.752
seconds. These were separate setup costs with zero scripted model calls. Parent
experiment design, oracle selection and review are not independently metered. Logical
tokens include cached input; monetary billing is unavailable. Tool bytes are not a
token or cost proxy. Historical native/oracle totals were not rerun and are not a
fresh paired comparison with the changed implementation. The paired study uses
compiler profiles; it does not substitute for a syntax/enriched/hybrid maintenance
study or cold-cost amortization across updates.

[Machine-readable paired results](source-documentation-projection-results.json)
contain protocol and answer digests, actual usage, quality decisions and arithmetic.
Raw prompts, responses, private paths and client session records remain outside the
repository. Five related synthetic questions and one repetition do not establish
confidence bounds or independent quality non-inferiority.

## Remaining boundaries

The feature documents committed files only. Capture budgets and parse errors are
explicit; unsupported files retain file-level evidence, and partial captures do not
become fresh. Renames, changed signatures and duplicates can require manual rebinding.
Conservative service watches do not prove universal influence closure or minimize
review. Lexical order does not establish callback, coroutine or network execution.
Engineer-declared service links do not prove deployment, delivery or persistence.

The broader research matrix, independent repository repetitions, maintenance-arm
economics and release qualification remain future qualification work. The current
change delivers availability and reviewable evidence without claiming those results.
