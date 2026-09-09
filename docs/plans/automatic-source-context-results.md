# Automatic initial source context: first comparison

Status: implemented local candidate; token-saving gate failed. Measured on 2026-09-09.

Implementation: `b3801a1`. The full `scripts/ci-verify.sh` gate passed before measurement. The feature is not in the published 0.7.1 archives.

## Question and method

Does deterministic compiler-backed source preparation before the first model request reduce actual model tokens compared with a disciplined native agent at equal answer quality?

Two new source-analysis questions used the same frozen Kotlin worker repository, model (`gpt-6-astra`, high reasoning), task instructions, native-tool access, and 600-second deadline. Each arm ran once in a fresh model conversation, serially. Both arms were instructed to batch independent discovery and reads, avoid rereading available source, preserve evidence limits, and neither change source nor run builds/tests.

Corpus revision: `1a820e8e6f0eff83b4a8c7afb6e4e7d7a5b4b61f`. This was a previously seen self-hosting corpus, not an unseen third-party repository. The protocol and private acceptance oracle were recorded before agent runs; a pre-run clarification added the existing lifecycle-test evidence. No unfavorable arm was replaced.

- P1: `Proto.readFrame` / `writeFrame` framing boundaries and actual test assertions. Packet root: `Proto`.
- P2: `ProjectModelInventory` / `cachedProjectModel` identity, filesystem changes, RPC/extraction/cache-hit freshness, and actual test assertions. Packet roots: `ProjectModelInventory`, `cachedProjectModel`.

Preparation received exact identifiers from the questions, a compilation and profile; it received no implementation paths or answer oracle. The unchanged first policy selected whole declarations/files, exact K2 callers/callees within two hops, and filename-based test companions. Both packets were prepared before any measured model request. Native continuation remained available.

The parent reviewed all four final answers against source and the pre-run oracle. All were accepted: the framing answers correctly distinguished the test name from its two actual assertions; the inventory answers distinguished tracked inputs, capture points, nested RAW/CANONICAL behavior, and tested versus implementation-only cases. This was not blinded independent grading.

## Actual usage

Total tokens are reported input plus output, including cached input. Cached input is a subset of input and is shown separately. No token count was estimated from response bytes.

| Task | Arm | Input | Cached input | Output | Total | Native commands |
|---|---|---:|---:|---:|---:|---:|
| P1 | native | 83,027 | 77,696 | 1,393 | 84,420 | 3 |
| P1 | preloaded | 88,212 | 69,248 | 1,184 | 89,396 | 2 |
| P2 | native | 189,807 | 157,312 | 3,129 | 192,936 | 8 |
| P2 | preloaded | 251,897 | 203,776 | 3,652 | 255,549 | 11 |

Preloaded usage increased by 5.9% on P1 and 32.5% on P2. Combined usage increased from 277,356 to 344,945 tokens (24.4%). The median per-task native/preloaded ratio was 0.850, below the required 3.0. Noncached input also increased: 5,331 to 18,964 on P1, and 32,495 to 48,121 on P2. No monetary-cost equivalence is inferred from token counts.

All commands exited successfully. Each native arm recorded two TLS reconnect events; neither preloaded arm recorded one. All four completed without timeout. These events remain in the measured outcomes and limit wall-time comparisons.

## Preparation and elapsed time

| Task | Preparation | Model, native | Model, preloaded | Preparation + preloaded model |
|---|---:|---:|---:|---:|
| P1 | 141.856 s | 77.192 s | 46.832 s | 188.688 s |
| P2 | 145.133 s | 136.113 s | 133.594 s | 278.727 s |

Preparation made zero model calls and consumed zero model tokens. Its full prompt was charged through actual model input. P1 delivered 24,441 source bytes in 15 windows (37,106-byte prompt); P2 delivered 65,485 source bytes in 21 windows (88,581-byte prompt). Both reported partial coverage and explicit omissions.

Each fresh private state directory also constructed a local source runtime capsule: the Cargo stage took 65.058 / 66.798 seconds. CLI admission, analysis and source selection took 74.851 / 75.843 seconds. All runtime construction elapsed remains charged above; overlapping build-stage durations must not be added. These are local source-runtime first-use observations with warmed dependency caches, not installed-release startup measurements. Full CI completed before preparation and did not overlap the measured arms.

Development checks before the frozen comparison included an ambiguous-constructor selector failure, an incorrect raw-hash versus domain-separated CAS verification failure, and a successful preparation on a previously used descriptor question after those fixes. Those artifacts were retained as development evidence, not substituted for these two new questions.

## Interpretation and next bounded hypothesis

Automatic preparation is functional, but the current policy does not reduce token use on these questions. On P1, selecting the complete `Proto` object seeds calls from unrelated helper methods and expands callers; the differently named transport test still requires discovery. One fewer native command did not offset the larger initial prompt.

On P2, the packet nearly fills its 64 KiB source allowance but omits required RPC field updates and differently named lifecycle tests. Exact call adjacency does not establish field read/write lifecycle coverage. The agent then performs 11 native commands versus 8 for the baseline. These are observed selection gaps; their individual contribution to token growth was not isolated experimentally.

Keep the candidate optional. The next hypothesis is a smaller initial packet centered on exact task methods and relevant test discovery, with caller expansion and field lifecycle evidence supplied only when needed. Evaluate a changed policy on new questions; preserve this result. This two-question comparison establishes neither a general regression nor a general benefit, and says nothing about mutation tasks or retained follow-up turns.

See [candidate usage and evidence limits](../operations/initial-source-context.md) and [previous manual calibration](token-value-three-criteria.md).
