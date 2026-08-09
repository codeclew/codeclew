# Codeclew P10: результат исполнения оптимизированного bootstrap

Дата: 9 августа 2026 года

Plan digest:
`80f2b7308c0e4eb51c6376931591dc389d0c08e6d7dc75a4ab757b7395506a34`

Итог: `REJECTED_BUDGET_AND_CORRECTNESS`

Eligible next edges: `[]`

## 1. Что было создано

P10 материализовал пять предусмотренных планом sidecars:

- `schemas/evidence/foundation-approval-v1.schema.json`;
- `schemas/evidence/foundation-packet-v1.schema.json`;
- `schemas/evidence/foundation-receipt-v1.schema.json`;
- `docs/superpowers/plans/codeclew-optimized-foundation-manifests-v1.json`;
- `scripts/verify-foundation-node.sh`.

Ruby, Python, network runtime и host/RSA attestation не используются.
Controller основан на Bash + jq 1.7, разделяет raw SHA и canonical digest и
публикует result через временный файл.

После одного delta-repair его self-test показывает:

```json
{"schemaVersion":"codeclew-foundation-self-test/1","status":"PASS","positiveCases":8,"negativeCases":17,"total":25,"passed":25,"canonicalScope":"JQ_1_7_SORTED_COMPACT_INTEGER_JSON"}
```

Зафиксированы:

- четыре sidecar digests, включая controller;
- exact `22` outcome rows и четыре node budgets;
- семь B03 execution probes;
- четыре GB success combinations;
- packet/receipt raw и canonical identity;
- producer/verifier agent/session independence;
- token/call formulas и отдельные retry refs;
- publication failure как exit `3`.

Controller raw SHA после repair:
`59c6ce0a26f37c8e21c7a56507d3fe37afdab0815c9cede38c9da23b3818a1a4`.

Frozen plan-contract digest:
`cd50b780bdf5a0ea04f2ae48fa3a44523743760f43268fd3ecd7d8cb2eb2f5a3`.

## 2. Независимые проверки

Первый свежий verifier вернул `REPAIR_REQUIRED` после `10` calls. Он выявил:

1. controller не был связан approval digest;
2. instance schemas проверялись частично;
3. exact budgets/outcome rows не были frozen относительно плана;
4. GB не проверял B02/B03 parents;
5. B03 execution manifest отсутствовал;
6. retry ancestry не имел content-addressed proof;
7. оставались TOCTOU/publication/independence пробелы.

После delta-repair self-test вырос с `13/13` до `25/25`. Повторный независимый
verifier использовал `8` calls и вернул `REJECT` по трём остаточным обходам:

1. `HIGH`: remaining retry calls self-asserted; не доказаны
   `remaining = ceiling - initial` и `initial + retry <= ceiling`, token/wall
   remaining budgets отсутствуют;
2. `HIGH`: B02 `SUCCESS+NONE` допускается при
   `nativeTokenTelemetryAvailable=false`, поэтому token-win claim может не быть
   запрещён;
3. `MEDIUM`: повторный TOCTOU recheck покрывает только `artifactRefs`, но не
   parent/retry refs и основные plan/approval/manifest/packet/receipt bytes.

Следовательно, self-test является необходимым, но недостаточным evidence.
`A10` и `B01` не открыты.

## 3. Фактическая стоимость

Реконструкция charged calls до записи этого отчёта:

| Участок | Calls |
| --- | ---: |
| Root orchestration, reads, validation, waits и discarded patch | `30` |
| Initial conventions scout | `8` |
| P10 producer | `16` |
| Fresh verifier | `10` |
| Delta-repair producer | `34` |
| Final delta verifier | `8` |
| **Итого до отчёта** | **`106`** |

Плановый P10 ceiling был `15` calls. Фактический pre-report расход выше него
в `7.1` раза. Даже если исключить root preflight, producer/verifier work равен
`76` calls, то есть выше ceiling в `5.1` раза.

Goal telemetry snapshot перед отчётом:

```text
tokensUsed = 100278
timeUsedSeconds = 3174
```

Это общий счётчик goal runtime, а не native noncached-token telemetry.
Input/cached/output/noncached decomposition недоступна и имеет статус
`UNAVAILABLE`; bytes как замена не использовались.

## 4. Эффективность гейта

Verdict: `STOP`.

Независимый гейт полезен: оба прохода нашли реальные способы открыть неверный
edge, которые positive self-test не обнаруживал. Однако P10 оказался
архитектурно перегружен: planning bootstrap фактически реализует существенную
часть будущих B02 runtime contracts, GB join и retry accounting до A10.

Это объясняет стоимость лучше, чем «неудачный worker»:

```text
P10 одновременно пытается доказать
  package integrity
  + полный instance-schema runtime
  + experiment telemetry policy
  + retry ledger
  + GB parent join
  + atomic publication
```

Такой узел не является минимальным bootstrap и не может обоснованно иметь
ceiling `15 calls`.

## 5. Необходимая развилка

Продолжать repair текущего P10 нельзя: это будет второй содержательный retry и
изменение budget после наблюдения outcome.

Рекомендуемая amendment-линия — `P10-lite`:

1. Оставить в P10 только plan/sidecar/current-task-event integrity,
   controller self-binding, exact static plan-contract digest и small mutation
   suite.
2. Перенести полный packet/receipt instance contract, telemetry coupling и
   retry accounting в B02, где они являются прямым evidence delta.
3. Перенести проверку B02/B03 parent matrix в GB, а в P10 проверять только
   статическую полноту join table.
4. Сохранить сегодняшние sidecars как экспериментальный prototype/evidence,
   но не использовать их для открытия edge.
5. Установить новый P10-lite ceiling только после dry-run на frozen fixtures,
   не задним числом для текущего результата.

Это изменение DAG contract и требует явного human approval. До него цель
остаётся незавершённой; ни один research или implementation edge не открыт.
