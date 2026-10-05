# Saved-edit analysis with installed Clew 0.13.11; reader qualification with 0.13.13

`observations.json` is an author-derived, safe public record of the observed case and later reader qualification. It is not native CLI output. The unchanged raw stdout/stderr, exact command records and native HTML reports are retained privately. Source fragments are from this public fixture. The original analysis used installed Clew 0.13.11. The later 0.13.13 qualification read that retained comparison; it did not repeat analysis or execute tests.

Use an installed `clew`, JDK 21, Git and the fixture's Gradle wrapper. `setup.py` copies the public `fixtures/kotlin-basic` wrapper from a Clew source checkout; it does not use that checkout's Clew launcher or run analysis/tests. Choose a new output directory:

```sh
python3 -I -S setup.py --checkout /path/to/clew-source --output /path/to/new-pricing-fixture
export JAVA_HOME=/path/to/jdk-21
export PATH="$JAVA_HOME/bin:$PATH"
fixture=/path/to/new-pricing-fixture
clew --version
clew doctor repository --repo "$fixture" --working-tree
```

The fixture has HEAD `price() = 1`, staged `2`, and saved `3`, plus a signature change, changed caller, comment, added/deleted files and a moved declaration. Keep these layers intact. The recorded doctor returned the exact `kotlin-2.4.10-gradle-single` profile and compilations `:/main` and `:/test`; use the ready contour reported for your recreated fixture.

```sh
clew change inspect --repo "$fixture" --target-ref refs/heads/main   --language kotlin --profile kotlin-2.4.10-gradle-single   --compilation :/main --working-tree --base HEAD
clew change inspect --repo "$fixture" --target-ref refs/heads/main   --language kotlin --profile kotlin-2.4.10-gradle-single   --compilation :/main --compilation :/test --working-tree --base HEAD
```

Use the comparison ID returned by the second command in these commands:

```sh
clew change graph --comparison RETURNED_COMPARISON
clew change source --comparison RETURNED_COMPARISON --file src/main/kotlin/Price.kt --side before
clew change source --comparison RETURNED_COMPARISON --file src/main/kotlin/Price.kt --side after
clew change source --comparison RETURNED_COMPARISON --file src/test/kotlin/PriceTest.kt --side after
clew change render --comparison RETURNED_COMPARISON --output /path/to/new-report.html
```

Main-only analysis excludes tests. Adding `:/test` records the compiler-resolved `PriceTest.priceCheck -> Pricing.price` relation, while retaining partial coverage and unresolved scopes. It does not execute tests or prove test sufficiency. The native report distinguishes direct consequences from runtime failures.

The recorded Gradle failure is a **separate earlier caller-project check** using JDK 21 and `./gradlew --no-daemon check`: main and test source compiled; one test failed with expected `1`, actual `3`. That command was not repeated or caused by Clew during the installed case. Clew's call edge identifies a concrete verification candidate; the separate native test execution supplies the observed failure.

After reviewing the retained comparison, change only saved `stable(): Int = 9` to `stable(): Int = 10`, without staging it. Then:

```sh
clew change check-freshness --comparison RETURNED_COMPARISON
clew change source --comparison RETURNED_COMPARISON --file src/main/kotlin/Price.kt --side after
```

The recorded result was `LIVE_CHANGED` with `retainedEvidenceValid: true`. The old source still returned `stable() = 9` and `price() = 3`; it did not refresh to the new saved `10`. No new semantic analysis or test execution was performed after this delta. A new comparison is required if current edits are needed.

Original 0.13.11 latency was a limitation of this case: sequential graph/source/render/freshness requests took roughly 55–117 seconds on shared default state. Overlapping earlier requests took longer. These are observed wall times, not a controlled performance benchmark. The extra supported storage dry-run failed with `INVALID_INPUT: run ledger contains an invalid transition`; it supplied no CAS object counts or byte totals. No cleanup or retry was performed during the original 0.13.11 case.

Native CAS object digests use domain separation and differ from raw source SHA-256. Both are labeled separately in the public observations.

The original 0.13.11 interactive report is an **exact native HTML report**, distinct from the author-derived observations. Its original and public raw-file SHA-256 are both `sha256:31f653810ff17e28b5c6a63259a0943ef1be76b46c4c7caae254ce343fb49fc7` (313566 bytes). Native schema: `codeclew-change-explanation/1.0`; narrative authority: `DETERMINISTIC_EVIDENCE_LABELS`. It binds `:/main` and `:/test`, declares partial coverage and `testsExecuted: false`, and labels live status `NOT_CHECKED_BY_OFFLINE_RENDER`. The separately recorded freshness result applies to the later intentional edit; the offline report was not rewritten or refreshed.

## Installed 0.13.13 retained-reader qualification

Official installed Clew 0.13.13 in `RELEASE` mode read the original retained main-and-test comparison on 2026-10-05. The full returned test-source JSON and embedded report evidence JSON were exactly equal to the installed 0.13.11 responses. Freshness still returned `LIVE_CHANGED`, `liveSnapshotMatches: false` and `retainedEvidenceValid: true`. There was no new semantic analysis, test execution or repeat of the earlier caller-project Gradle check.

Sequential calls on existing shared default state took the following observed wall times:

| Retained request | Original 0.13.11 | Installed 0.13.13 |
| --- | ---: | ---: |
| Test source after | 110.854 s | 5.677 s |
| Main-and-test report | 112.380 s | 5.650 s |
| Freshness after saved edit | 117.058 s | 5.790 s |

These are observations on existing shared state, not an isolated or cold benchmark, and establish no global latency guarantee. Graph and main-source requests were not repeated. The original timings, failed storage dry-run and separate earlier test-failure record remain unchanged in `observations.json`.

[Open the 0.13.13 native reader](report-0.13.13.html) for the original retained evidence. Its renderer is 0.13.13; its analyzed source and evidence remain from 0.13.11. The offline report still labels live status `NOT_CHECKED_BY_OFFLINE_RENDER`; the separately recorded freshness result does not rewrite it. The exact native HTML copy is 315830 bytes, with original and public raw-file SHA-256 `sha256:dccedb59c7bb3b17c0783351df5a7d09ac15f7522f268f74a573dfe25cd8578e`. Native schema and narrative authority are unchanged. Whitelisted runtime identity, exact-equality outcomes, freshness and full observed timings are in `installedReaderQualification`; private command arguments and filesystem paths are excluded.

[The original 0.13.11 native report](report.html) remains byte-identical, with its original SHA-256 and metadata preserved in `nativeReport`. Reader selection and mobile layout changes do not relabel the original analysis or alter its source, coverage or test claims.

A separate supported 0.13.13 storage GC dry-run also exited `2` with non-retryable `INVALID_INPUT`. Its improved diagnostic identifies a logical run and the invalid transition `WorktreeRecoveryRequired -> Cancelled` at sequence `3`; the public record omits the run ID. The inherited ledger remains invalid and unrepaired. Root cause is `UNPROVEN`; no cleanup, recovery or private-object edit was performed, and CAS counts/bytes remain `UNKNOWN`. This followup is recorded separately from the original unchanged `storageMetadata`.
