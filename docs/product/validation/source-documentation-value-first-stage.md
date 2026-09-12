# First-stage documentation value results

Date: 2026-09-12. Status: M0-M2 engineering comparison complete; planner not qualified.

## Result

All 15 answers passed parent source review of the frozen obligations: 19 required
facts per arm, no observed critical miss or false-exact claim. On these five
related fixture questions, installed Codeclew 0.7.1 required 2.56 times the
logical model tokens of disciplined native reading. Manually selected original
source in the initial prompt used 68.6% fewer downstream model tokens than native
reading. This is an oracle feasibility result, not an automatic product saving.

| Task | Native | Current Codeclew | Oracle source | Quality, all arms |
|---|---:|---:|---:|---|
| E1: Kafka topic/key | 50,022 | 102,663 | 16,925 | PASS |
| E2: import quantity paths | 51,263 | 161,774 | 17,112 | PASS |
| E3: ambiguous `receive` | 50,267 | 92,491 | 17,083 | PASS |
| E4: checkout client path | 71,380 | 148,083 | 17,323 | PASS |
| E5: declared two-service checkout | 56,163 | 210,275 | 19,261 | PASS |
| **Total** | **279,095** | **715,286** | **87,704** | **5/5 per arm** |

Tokens are actual reported cumulative input plus output, including cached input.
Each run's per-round usage reconciles with its final client totals. No model
token count was estimated from source or tool-output bytes.

| Additional measurement | Native | Current Codeclew | Oracle source |
|---|---:|---:|---:|
| Input tokens | 273,052 | 706,915 | 83,460 |
| Cached input, included above | 162,432 | 508,672 | 24,320 |
| Output tokens | 6,043 | 8,371 | 4,244 |
| Model rounds | 16 | 24 | 5 |
| Shell commands | 27 | 34 | 0 |
| Reported tool-output bytes | 28,922 | 681,796 | 0 |
| Model-arm elapsed time | 241.789 s | 330.084 s | 157.702 s |

Oracle source was supplied in the initial prompt and charged in actual input;
zero tool-output bytes does not mean no source. Command count and model-round
count differ because independent commands can be batched. Monetary billing is
unavailable, so these values are not reported as actual monetary savings.

## What the current documentation path established

The three source repositories were admitted and captured by installed RELEASE
0.7.1 before the current arms. Capture took 18.898 seconds with warmed dependency
caches; installation, registration and binding are separate setup costs.
Preparation scripts made no model calls. The current arms then used retained
documentation context and allowed native continuation for missing evidence.

- Orders and inventory each had one discovered HTTP entrypoint with COMPLETE
  reported extraction coverage on these fixtures.
- Kotlin 1.9.25 had two discovered entrypoints (HTTP and Kafka) with PARTIAL
  coverage. K2 2.4.10 analysis retained explicit compiler/language/API differences,
  unresolved or unsupported flows, and runtime-activation boundaries.
- Local source and supported compiler/framework observations were usable for
  domain explanations. The orders-to-inventory link remained human-declared,
  with runtime destination, delivery and persistence unproven.
- No baseline narrative existed yet, so the initial check's freshness was
  UNRESOLVED with MISSING_BASELINE despite successful source capture. A check
  exit code of 3 here does not mean compiler admission failed.
- The model correctly distinguished the in-memory `lastReservation` assignment
  from durable persistence and did not claim an executed end-to-end test.

The checked source snapshots remained unchanged. These observations establish
fixture behavior of the existing source/context path; they do not by themselves
qualify all supported frameworks or prove arbitrary generated prose.

## Observed delivery gaps

The selected Kotlin listener context omitted the other, unannotated `receive`
declaration; the current arm needed native source to answer E3. The selected Java
operation context omitted constructor/field context needed for E4's destination
configuration explanation. E5 included repeated large context responses and
native continuation. No command failed, but successful retrieval did not always
deliver the complete task evidence in one response.

The full documentation workflow and rich context are part of the current arm.
The comparison therefore does not isolate compiler facts from instruction,
projection, pagination or orchestration cost. It does show that the complete
current workflow was more expensive on every selected question at accepted
answer quality. Repeated large payloads and extra model rounds are concrete
targets for a bounded projection improvement; causal attribution is not claimed.

## Decision for the next work

1. Proceed with shared source/syntax documentation for Python, Java without a
   working build, and Kotlin 1.9 without K2. Availability is independently useful
   and should not be marketed as a proven token saving.
2. Preserve one documentation model and versioned source bindings, qualify
   conservative freshness, then add optional semantic enrichment and bounded
   change dossiers.
3. Prioritize M3 compact evidence delivery using the existing source and
   verification machinery. Avoid repeating broad source/context payloads.
4. Defer M4-M6 general planning/selection/routing. Five related fixture questions
   with one repetition and parent grading do not meet the repeatable independent
   qualification needed to promote that investment. Preserve the earlier
   automatic-preloading regression rather than replacing it with this oracle win.

## Limits and reproduction

The [frozen protocol](source-documentation-value-protocol.md) records task strata,
arms, model configuration, acceptance and accounting. The [machine-readable results](source-documentation-value-results.json)
contain each run and aggregate arithmetic. Private frozen repositories, prompts,
answers, per-round usage and preparation records remain outside the repository.

The first client preflight was rejected before a model answer because Codex CLI
0.147.0 was too old for the selected model; reported usage is unavailable. All
measured arms used CLI 0.153.4 and the same `gpt-6-astra` / high configuration.
An initial catalogue-add attempt also exposed the need for its current input
digest; its failure remains in preparation records. No unfavorable model answer
was replaced, and no source builds overlapped measured model arms.

Manual oracle selection and experiment design were performed by the parent
assistant outside measured arms and are not separately metered. The oracle
numbers are downstream feasibility measurements, not the end-to-end cost of an
automatic product. This small, correlated corpus does not establish a
population-level confidence bound, a general 30% product saving, or independent
quality non-inferiority. The next report must add executed documentation
availability, freshness, publication and example-rendering results.
