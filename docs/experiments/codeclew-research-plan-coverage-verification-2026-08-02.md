# Codeclew: верификация покрытия Deep Research планом работ

Дата: 2 августа 2026 года

Итоговый вердикт независимого агента: `COVERED`

Verifier: fresh-context agent `plan_research_coverage_audit`.

## Объект проверки

- Исследование:
  [`deep-research-codeclew-semantic-editing-results.md`](deep-research-codeclew-semantic-editing-results.md).
- План:
  [`2026-08-02-codeclew-corpus-first-plan.md`](../superpowers/plans/2026-08-02-codeclew-corpus-first-plan.md).
- Аудированный research baseline:
  `7fc3e0d6c6e784a130245ef0e344535a146324c7`.

Проверка отвечает только на вопрос, покрывает ли план выводы и обязательные
эксперименты исследования. Она не подтверждает, что задачи плана уже
реализованы или что Codeclew уже выигрывает у baseline.

## Метод

Отдельный агент без истории авторства полностью прочитал исследование и план и
проверил:

- architecture gaps и границы доказанной полноты;
- bottleneck/cost model, время и token telemetry;
- goal language, Change Graph, `COMPLETE_FOR` и graph facts;
- `MAP_EDGE_WITH_CONTEXT`, test oracle и PSI-native materialization;
- neutral corpus, ecological population и anti-overfitting;
- blind goal-binding и paired default/AST-index/Codeclew benchmarks;
- все 20 обязательных вопросов;
- три рекомендованных implementation commits;
- decision thresholds, falsifiers и risk register;
- зависимости задач, gates и запрет преждевременной materialization.

Дополнительно машинно проверены наличие T00–T23, обязательных полей каждой
задачи, разрешимость `Depends on`, локальные Markdown-ссылки и отсутствие
форматных дефектов.

## Результаты проходов

### Проход 1 — `NOT_COVERED`

Независимый агент нашёл четыре блокирующих пробела:

1. External ecological validation была только известным ограничением, но не
   исполнимой задачей.
2. G1 мог разрешить materialization без независимого blind audit.
3. Paired goal-binding harness был отложен после materialization, вопреки
   рекомендованному третьему commit исследования.
4. `f_build`, speedup ceiling и repository-size scaling не были обязательной
   частью анализа.

Исправления внесены в T01, T04, T07, T14, T20, T22, G1-Corpus и G1-Binder.

### Проход 2 — `NOT_COVERED`

Четыре исходных пробела были подтверждены как устранённые. Обнаружена одна
новая циклическая зависимость: G1 требовал explicit test-oracle class, но её
реализация оставалась в условной T17 после G1.

Исправление:

- schema oracle ownership перенесена в T08;
- вывод класса и refusal при отсутствии спецификации — в T11;
- no-oracle must-refuse для `MAP_EDGE_WITH_CONTEXT` — в T12;
- в T17 оставлены только materialization-time enforcement и mutation gate.

### Проход 3 — `COVERED`

Независимый агент подтвердил:

> Proof-level oracle cycle устранён. Оставшиеся существенные области
> fail-closed и проверяемы. Новых блокирующих противоречий не обнаружено.

## Матрица покрытия разделов исследования

| Раздел исследования | Задачи и gates плана | Статус |
| --- | --- | --- |
| 0–3. Verdict, pipeline и доказанные gaps | T00, T08–T10, T13, T19 | Покрыто |
| 4–5. Bottleneck и cost model | T01, T14, T20–T22 | Покрыто |
| 6. Typed constraints вместо macros/recipes | T08, T11–T13 | Покрыто |
| 7. `COMPLETE_FOR` и negative completeness | T09–T10, G1-Binder | Покрыто |
| 8–9. Graph facts, Change Graph и proof architecture | T08–T11 | Покрыто |
| 10. `MAP_EDGE_WITH_CONTEXT` | T12, T14 | Покрыто |
| 11. Test oracle ownership | T08, T11–T12, T17 | Покрыто |
| 12. PSI-native materialization safety | T15–T19, G2 | Покрыто и gated |
| 13. Neutral corpus и ecological validation | T02–T07, G1-Corpus | Покрыто |
| 14. Paired benchmark protocol | T01, T14, T20–T22 | Покрыто |
| 15. Ответы на 20 обязательных вопросов | Отдельная матрица ниже | Покрыто |
| 16. Maximum-information experiment | T14, G1-Binder | Покрыто |
| 17. Три первых commits | T02–T07; T08–T13; T14 | Покрыто с более атомарной декомпозицией |
| 18. Thresholds и falsifiers | T00, G1-Binder, G2, T23 | Покрыто |
| 19. Risk register | T01–T22 и fail-closed gates | Покрыто |
| 20. `GO_BUILD_CORPUS_FIRST` | Порядок M0–M4 | Покрыто |

## Покрытие 20 обязательных вопросов

| № | Тема | Покрытие плана |
| ---: | --- | --- |
| 1 | Model-owned bottleneck | T01, T14, T22 |
| 2 | Что детерминировать worker-ом | T09–T12, T15–T17 |
| 3 | Минимальный goal | T08, T12 |
| 4 | Constraint language или macros | T08, T11 |
| 5 | Защита от repository recipes | T03–T04, T08, T11, T14 |
| 6 | Недостающие graph facts | T09–T11 |
| 7 | Формальный `COMPLETE_FOR` | T10, G1-Binder |
| 8 | Model-owned ambiguity | T08, T11–T12 |
| 9 | Test oracle ownership | T08, T11–T12, T17 |
| 10 | Замена text heuristics | T15–T16 |
| 11 | Withheld corpus | T02–T07 |
| 12 | Популяция прикладных задач | T07, G1-Corpus |
| 13 | Applicability rate | T14, G1-Binder |
| 14 | Масштабирование по размеру repository | T04, T14, T22 |
| 15 | Bounds context/output | T01, T10, T14 |
| 16 | Где default быстрее | T07, T14, T22 |
| 17 | Где AST-index достаточен | T07, T14, T22 |
| 18 | Необмениваемые safety gates | G1-Binder, G2, T21–T23 |
| 19 | Experiment maximum information gain | T14 |
| 20 | Финальный verdict | T23 |

## Покрытие рисков

- Overfitting, vocabulary leakage и population skew: T03–T07 и blind audits.
- False completeness, graph boundaries и multi-root staleness: T09–T10, T19.
- Text rewrite и K2-version instability: T15–T16, G2.
- Lifecycle, coroutine, laziness и unknown effects: T06, T09, T11–T12.
- Self-confirming tests и отсутствующий business oracle: T08, T11–T12, T17.
- Подмена tokens bytes и build-dominated результаты: T01, T14, T22.
- Recipe explosion: T04, T08, T11, anti-vocabulary checks T14/T22.

## Ограничения вердикта

- `COVERED` означает полноту planning coverage, а не успешность будущих runs.
- Если public ecological sample, double labeling или provider token telemetry
  недоступны, план обязан блокировать `GO_IMPLEMENT`, а не заполнять пробел
  оценочными данными.
- Android, KMP, reflection-heavy frameworks, arbitrary compiler plugins и
  внешние business specifications остаются вне заявленного supported contour
  либо приводят к explicit refusal.
- Любое изменение research thresholds или supported population делает этот
  coverage verdict устаревшим и требует нового независимого прохода.

## Финальный вывод

План в текущей версии полностью покрывает исследование и сохраняет его главный
порядок доказательства:

```text
neutral corpus + independent population
    -> proof/refusal binder
    -> blind paired goal-binding audit
    -> G1
    -> PSI-native materialization одного proven family
    -> G2
    -> paired accepted-commit benchmark
    -> GO / NARROW / STOP
```

Новый production transform не может быть реализован до machine-readable G1
PASS, а универсальная победа не может быть заявлена без independent ecological,
correctness, timing и token evidence.
