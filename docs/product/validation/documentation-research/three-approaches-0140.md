# Three final documentation approaches before 0.14.0

Status: research closed after the explicitly bounded three approaches. Product release qualification remains separate. These results do not establish automatic completeness, optimal token cost, or production compatibility.

## Scope and method

The public Weather system consists of Places, Forecasts, and Dashboard. The reader task covered HTTP/OpenAPI, Kafka, scheduled jobs, domain objects, external participants and both ends of service connections. Detailed field-level DataFlow and GraphQL were excluded. Retained Places/Forecasts source revision was c289b28be4819eddf29d59a7c12f178559ec5229; Dashboard was 7e602ace02c04006fb7b2a2f5ecd88ba3c4005c6. The admitted experimental product was ab6f8677116af9cc35b343a4be35e17902fd2401, tree ee5573a719809252091f8f6026effc9f9ea109b8. It was not an official 0.14.0 package.

Authors used gpt-6-luna/max. They could revise their own drafts before READY. Two independent gpt-6.1-sol/high critics evaluated each final output only after it was frozen; no critic-driven author correction followed. Each critic froze answers to 48 reader questions before consulting source evidence. Rendered pages were actually inspected. Required gate: all 48 material requirements, no material false claims, clarity at least 18/20 and navigation at least 11/12 for each critic, plus visual acceptance. Source identities could remain in sidecars rather than reader prose. Natural equivalent explanations counted; prescribed wording and repeated negative disclaimers were not required.

Token metric is input minus cached input plus output; reasoning tokens are already included in output. All author responses, including failed calls and self-revision, are counted. Critic cost is separate. Root research, deterministic preparation and build/capture work are not represented by author token totals. This was not a randomized or replicated causal comparison. Whole-system costs are not directly comparable to earlier Places-only trials or a differently scoped Direct baseline. Direct was not rerun.

## Outcomes

| Approach | Disposition | Author net tokens | Terminal completeness | Clarity | Navigation |
| --- | --- | ---: | --- | --- | --- |
| 1. Operational SQL projection | Technical NO-GO before authoring | 0 | Not measured | Not measured | Not measured |
| 2. Sequential same-model service contexts plus final integration | Quality gate failed | 740,075 | Both critics 43/48 | 18/20 and 17/20 | Both 10/12 |
| 3. Original evidence plus bounded runtime observations | Quality gate failed | 276,821 | Both critics 29/48; root adjudication 27/48 | 17/20 and 16/20 | 8/12 and 9/12 |

Approach 1 needed new literal extraction, SQL interpretation/dialect boundaries and source mapping to produce reliable operational explanations. It was rejected as disproportionate to the bounded release horizon. This was not an empirical author failure or proof that the technique is impossible.

Approach 2 used four fresh contexts of the same model: Forecasts 131,648; Places 195,495; Dashboard 159,034; integration 253,898 net tokens. Ten local pages became 12 final pages. All Places requirements passed, but Forecasts refresh/publish settings, connection timeout, backoff details and Dashboard startup readiness remained incomplete. Dense prose and weak section navigation also missed the gate. The result does not establish that partitioning itself caused an improvement. Critic cost was 300,542 net tokens.

Approach 3 supplied an immutable observation record from an isolated public fixture-mode deployment: 19 HTTP requests and two database snapshots showed local completion followed by a forecast and Dashboard projection. Two Kafka-console sampling attempts timed out and captured no messages; this is not evidence of empty topics. The runtime was removed after the observations. The author produced five pages. Missing explanations included startup, selection/locking, send/commit replay windows, timeout/backoff details, validation boundaries and browser behavior. Bounded happy-path observations did not ensure those source-supported alternatives survived into prose. Critic cost was 277,658 net tokens.

The third approach's critics disagreed on five requirement classifications. Root retained their original verdicts and adjudicated only those disagreements against frozen prose and source. The final count is 27 complete, 19 partial and two contradictory requirements. The two source-semantic defects are downward rounding stated for RoundingMode.DOWN (which truncates toward zero), and an unqualified three-city GET restriction where the enum belongs to the OpenAPI declaration rather than the handler. A Java semantic discriminator confirmed the rounding distinction; it was not a new author trial or a live-provider test. The retained fixture observation itself is not claimed to demonstrate a negative-coordinate failure.

## Product evidence and limits

Both authored outputs passed compose and checked export. The 12-page result had 70 source-bound claims and 201 checked internal links. Missing optional public source URL mappings did not mean lost evidence: 202 claim-to-source records remained, with no empty source list. The five-page result had 32 bound claims and 47 checked internal links. Both outputs passed visual layout inspection; semantic completeness and navigation still failed. Mechanical binding proves selected-source association and literal placement, not entailment or human comprehension.

The independently accepted product slices remain useful: retained-source acquisition and portable authoring workflow, genuine Work source citations, full C# source bodies, native-file bindings with freshness refusal, C# source regions, compose diagnostics, and protected OpenAPI/JSON Schema/DDL declaration components. Their focused technical acceptance must be re-established on the exact official package where affected. They do not justify an automatic 10/10 documentation claim. No operational SQL engine, runtime-wire proof, generalized ownership inference or new author model is included by these experiments.

## Stop decision

No fourth approach, additional author correction, or repeated Direct run follows this cycle. Publish 0.14.0 only after required exact-commit CI, official platform packaging, independent package acceptance and public asset verification. Preserve the failed outputs and measurements as research evidence. Future research must distinguish source availability, authored meaning loss and presentation defects; it must not treat successful composition, source citations or a happy-path runtime observation as a completeness oracle.
