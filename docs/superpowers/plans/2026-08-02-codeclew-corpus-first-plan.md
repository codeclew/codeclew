# Codeclew — план работ после Deep Research

## Метаданные

- **Режим:** `quick-plan`, planning/design-only.
- **Статус решения:** `GO_BUILD_CORPUS_FIRST`, `approved_by_human` запросом на
  планирование по результатам исследования.
- **Source of truth:**
  [`deep-research-codeclew-semantic-editing-results.md`](../../experiments/deep-research-codeclew-semantic-editing-results.md).
- **Постановка исследования:**
  [`deep-research-codeclew-semantic-editing.md`](../../experiments/deep-research-codeclew-semantic-editing.md).
- **Аудированный code baseline:**
  `7fc3e0d6c6e784a130245ef0e344535a146324c7`.
- **Цель:** доказать или опровергнуть способность общего semantic-goal binder
  существенно уменьшать model-owned работу на нейтральной multi-family
  выборке до реализации нового production transform.
- **Модель выполнения:** последовательные атомарные задачи; одна задача — один
  commit; следующий milestone начинается только после формального gate.
- **Параллелизм:** запрещён внутри одного generated seed или benchmark pair;
  допустим между независимыми seeds только после подтверждения
  детерминированности и изоляции ресурсов.

План не утверждает, что поведение уже реализовано или проверено. Runtime
evidence должно появляться по мере выполнения задач.

## Результат программы работ

На выходе должен быть получен один из трёх решений:

1. `GO_IMPLEMENT` — binder доказал достаточную applicability/correctness;
   выбранный family разрешено материализовать PSI-native операциями.
2. `NARROW_FEATURE_ONLY` — mechanism полезен для узкого family, но не
   подтверждает универсальную продуктовую гипотезу.
3. `STOP_NOT_PLAUSIBLE` — coverage/correctness/cost не оправдывают дальнейшую
   универсализацию.

## Что не входит в план до прохождения gate G1

- второй production transform;
- repository-specific recipes;
- перенос имён, полей или patches из закрытых benchmark repositories;
- сравнение по bytes вместо token telemetry;
- claim о победе над default или AST-index;
- PSI materialization нового family;
- оптимизация build/test времени, не подтверждённая профилем корпуса.

## Конвенции выполнения

1. Каждая задача начинается с чтения перечисленных `Read first` файлов.
2. Если предпосылка задачи не выполнена, worker записывает `BLOCKED:` в Status
   и не реализует обходной путь.
3. Failed, refused и retried runs не удаляются из run manifest.
4. Закрытые benchmark reports используются только с evidence label
   `ARTIFACT`.
5. Hidden seed/oracle не монтируется в agent worktree до окончания run.
6. Все новые форматы имеют `schema` и version; canonical serialization
   обязательна для digest и comparison.
7. Каждая задача завершается узким тестом и `git diff --check`.
8. Commit convention: `feat(corpus): TXX ...`, `feat(goal): TXX ...`,
   `test(goal): TXX ...`, `bench: TXX ...` или `docs: TXX ...`.
9. Product artifacts не обновляются: программа меняет внутреннюю инженерную
   платформу и benchmark evidence, но не пользовательские сценарии.

## Общие команды проверки

| Команда | Назначение |
| --- | --- |
| `cargo fmt --all -- --check` | Rust formatting |
| `cargo clippy --workspace --all-targets -- -D warnings` | Rust static checks |
| `cargo test --workspace` | полный Rust/Kotlin fixture contour |
| `./scripts/verify.sh` | полный репозиторный verification gate |
| `git diff --check` | whitespace/patch integrity |

## Milestone map

| Milestone | Задачи | Результат | Gate |
| --- | --- | --- | --- |
| M0 Measurement contract | T00–T01 | frozen definitions и schemas | G0 |
| M1 Corpus + population | T02–T07 | ≥36 tasks и независимые population weights | G1-Corpus |
| M2 Goal binding | T08–T14 | proof/refusal без source mutation | G1-Binder |
| M3 PSI materialization | T15–T19 | только при G1-Binder=PASS | G2 |
| M4 Paired benchmark | T20–T23 | честное default/AST/Codeclew решение | G3 |

Оценка до первого решения `GO/STOP` по binder: 3–4 инженерные недели. Полная
ветка с PSI materialization и paired benchmark: ещё 3–5 недель. Это порядок
величины, а не сроковое обязательство.

---

## T00. Зафиксировать gap register и decision contract

- **Status:** - [ ]
- **Goal:** превратить выводы исследования в machine-readable список gaps,
  decisions, thresholds и falsifiers относительно текущего HEAD.
- **Sources:** разделы 15–20 research results.
- **Depends on:** —.
- **Read first:**
  - `docs/experiments/deep-research-codeclew-semantic-editing-results.md`;
  - `benchmarks/reports/universal-task-surface-2026-08-02.json`;
  - `crates/sthread/src/task_context.rs`;
  - `crates/sthread/src/task_plan.rs`.
- **Modify:**
  - add `benchmarks/semantic-change/gap.json`;
  - add `benchmarks/semantic-change/decisions.json`;
  - add `benchmarks/semantic-change/README.md`;
  - add `scripts/validate-semantic-decisions.py`.
- **Product artifacts:** No product artifact update because this task records
  internal research and benchmark contracts only.
- **Steps:**
  1. Записать каждый найденный gap со статусом `done|partial|missing` и
     evidence path.
  2. Зафиксировать thresholds без округления и пояснить denominator каждой
     метрики.
  3. Зафиксировать решения `GO_BUILD_CORPUS_FIRST`, запрет production transform
     до G1 и три возможных финальных verdict.
  4. Добавить machine-check уникальности IDs и допустимых статусов.
- **Verify:**
  ```bash
  python3 scripts/validate-semantic-decisions.py \
    benchmarks/semantic-change/decisions.json \
    benchmarks/semantic-change/gap.json
  git diff --check
  ```
- **DoD:** все research gaps имеют ID/evidence/status; thresholds и falsifiers
  машиночитаемы; нет task vocabulary закрытых repos.

---

## T01. Определить telemetry и run artifact schemas

- **Status:** - [ ]
- **Goal:** сделать невозможным смешение tool wall, agent end-to-end, bytes и
  tokens.
- **Sources:** research sections 5 and 14.
- **Depends on:** T00.
- **Read first:**
  - `benchmarks/reports/maven-product-repo.json`;
  - `benchmarks/reports/agent-context-pim-migrator.json`;
  - `benchmarks/README.md`.
- **Modify:**
  - add `benchmarks/semantic-change/schema/run.schema.json`;
  - add `benchmarks/semantic-change/schema/telemetry.schema.json`;
  - add `benchmarks/semantic-change/schema/outcome.schema.json`;
  - add valid and invalid fixtures under
    `benchmarks/semantic-change/testdata/`;
  - add `scripts/validate-semantic-run.py`.
- **Product artifacts:** No product artifact update because this task defines
  internal measurement evidence.
- **Steps:**
  1. Ввести обязательные clock markers: task-visible, context-end,
     goal-valid, first-edit, apply-end, hidden-acceptance-end.
  2. Разделить worker/model/build/test duration.
  3. Хранить input/cached/output tokens и вычислять raw/noncached только
     валидатором.
  4. Требовать failed/refused/retried outcomes и exclusion reason.
  5. Reject report, если end-to-end заменён суммой tool durations или tokens
     отсутствуют, но заявлено token win.
  6. Хранить repository LOC/modules, model-visible context/proof/diagnostic
     bytes, navigation calls и `f_build=(compile+tests)/end-to-end`; вычислять
     scaling и speedup ceiling только валидатором.
- **Verify:**
  ```bash
  python3 scripts/validate-semantic-run.py \
    benchmarks/semantic-change/testdata/valid-run.json
  ! python3 scripts/validate-semantic-run.py \
    benchmarks/semantic-change/testdata/tool-wall-as-e2e.json
  ```
- **DoD:** schema принимает полный run, отклоняет misleading telemetry и
  сохраняет причины всех неуспешных исходов.

---

## T02. Создать отдельный deterministic corpus generator

- **Status:** - [ ]
- **Goal:** получить нейтральную, версионированную и воспроизводимую основу
  корпуса без закрытых repositories.
- **Sources:** research section 13.
- **Depends on:** T01.
- **Read first:**
  - `Cargo.toml`;
  - `fixtures/kotlin-2-1/`;
  - `fixtures/kotlin-maven/`;
  - `scripts/benchmark-corpus.sh`.
- **Modify:**
  - add workspace crate `crates/semantic-corpus/`;
  - update `Cargo.toml` workspace members;
  - add `benchmarks/semantic-change/schema/task-manifest.schema.json`.
- **Product artifacts:** No product artifact update because this task adds
  benchmark infrastructure only.
- **Steps:**
  1. Реализовать CLI `semantic-corpus generate --seed --family --build-system
     --output`.
  2. Генерировать Git-initialized Gradle/Kotlin 2.1 и Maven/Kotlin 2.3 projects
     с одним baseline commit.
  3. Отделить task-visible repository от controller-only manifest/oracle.
  4. Сделать output canonical и byte-identical для одинакового seed.
  5. Запретить запись generated caches в source tree.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus deterministic_generation
  run_a=$(mktemp -d /tmp/codeclew-corpus-a.XXXXXX)
  run_b=$(mktemp -d /tmp/codeclew-corpus-b.XXXXXX)
  cargo run -p semantic-corpus -- generate --seed 42 --family smoke \
    --build-system gradle --output "$run_a"
  cargo run -p semantic-corpus -- generate --seed 42 --family smoke \
    --build-system gradle --output "$run_b"
  diff -ru --exclude=.git "$run_a" "$run_b"
  ```
- **DoD:** одинаковый seed даёт идентичный project/manifest; Gradle и Maven
  baseline компилируются; task-visible tree не содержит oracle.

---

## T03. Реализовать hidden manifest и oracle isolation

- **Status:** - [ ]
- **Goal:** обеспечить blind acceptance при публичном generator code.
- **Sources:** research sections 13.4–13.5.
- **Depends on:** T02.
- **Read first:**
  - `crates/semantic-corpus/src/`;
  - `crates/sthread/src/canonical.rs`;
  - `crates/sthread/src/transaction.rs`.
- **Modify:**
  - extend `crates/semantic-corpus` with `seal`, `reveal`, `verify` commands;
  - add hidden/public manifest testdata;
  - add `scripts/check-corpus-isolation.sh`.
- **Product artifacts:** No product artifact update because this task protects
  benchmark validity.
- **Steps:**
  1. Public manifest содержит task text, base revision и family, но не oracle
     patch/hidden tests/seed material.
  2. Controller manifest содержит expected obligations, acceptable design
     classes, hidden tests и refusal reasons.
  3. До run публикуется digest controller manifest; содержимое раскрывается
     после завершения серии.
  4. Проверка isolation сканирует agent worktree, environment manifest и tool
     stdout на forbidden fields.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus hidden_manifest_is_not_agent_visible
  ./scripts/check-corpus-isolation.sh \
    benchmarks/semantic-change/testdata/isolated-run
  ```
- **DoD:** agent-visible tree не позволяет восстановить exact oracle; digest
  проверяется после reveal; leak test fail-closed.

---

## T04. Добавить structural variation и decoy engine

- **Status:** - [ ]
- **Goal:** исключить победу за счёт имён, форматирования или одного layout.
- **Sources:** research section 13.3.
- **Depends on:** T03.
- **Read first:**
  - `crates/semantic-corpus/src/`;
  - `crates/sthread/tests/metamorphic.rs`;
  - `fixtures/kotlin-control-flow/`.
- **Modify:**
  - add corpus variation model and generators;
  - add metamorphic corpus tests.
- **Product artifacts:** No product artifact update because this task only
  diversifies benchmark programs.
- **Steps:**
  1. Вариировать identifiers, packages, source order, comments и formatting.
  2. Добавить overloads, extensions, decoy symbols и unrelated modules.
  3. Вариировать nullability, inheritance и collection modality.
  4. Генерировать positive/ambiguous/must-refuse variants из одного semantic
     seed без сохранения одинакового textual patch.
  5. Для части seeds создать size strata с неизменной semantic obligation
     closure и 1x/10x repository padding через unrelated modules/decoys.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus variation_preserves_manifest_semantics
  cargo test -p semantic-corpus decoys_change_text_not_oracle
  ```
- **DoD:** минимум 10 structural dimensions; oracle obligations стабильны;
  target text/hash различаются между variants; size strata позволяют проверить,
  что model-visible context не растёт линейно с repository LOC.

---

## T05. Добавить три data-flow семейства корпуса

- **Status:** - [ ]
- **Goal:** покрыть wiring, signature propagation и DTO/event contract
  evolution без доменных имён прежних задач.
- **Sources:** research section 13.2 families 1–3.
- **Depends on:** T04.
- **Read first:** corpus schemas и variation engine.
- **Modify:** family generators, manifests, hidden tests and controller oracles
  under `crates/semantic-corpus`.
- **Product artifacts:** No product artifact update because this task adds
  internal benchmark families.
- **Steps:**
  1. Для каждого family создать Gradle и Maven positive variant.
  2. Добавить ambiguity с двумя type-compatible bindings.
  3. Добавить must-refuse boundary.
  4. Hidden tests проверяют declared contract, все branches и omission mutant.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus family_wiring
  cargo test -p semantic-corpus family_signature_propagation
  cargo test -p semantic-corpus family_contract_evolution
  ```
- **DoD:** по 6+ tasks на family; positive/ambiguous/refuse представлены;
  hidden tests падают на omission/wrong-branch mutants.

---

## T06. Добавить persistence и lifecycle семейства

- **Status:** - [ ]
- **Goal:** проверить границы, где K2 недостаточно без query/framework evidence.
- **Sources:** research section 13.2 families 4–5.
- **Depends on:** T04.
- **Read first:**
  - Maven fixture;
  - worker effect/type facts;
  - corpus manifest schema.
- **Modify:** persistence/nullability и configuration/annotation/lifecycle
  family generators/tests.
- **Product artifacts:** No product artifact update because this task adds
  refusal-sensitive benchmark families.
- **Steps:**
  1. Создать safe typed projection positives без repository vocabulary.
  2. Создать nullable mismatch и multiple-query ambiguity.
  3. Создать direct configuration positive и DI/reflection must-refuse.
  4. Добавить transaction/coroutine/lazy boundary variants.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus family_persistence_projection
  cargo test -p semantic-corpus family_configuration_lifecycle
  ```
- **DoD:** hidden oracle отличает nullable/public-contract ошибки; unsupported
  lifecycle cases явно помечены must-refuse.

---

## T07. Заморозить corpus v1 и независимо определить target population

- **Status:** - [ ]
- **Goal:** завершить минимум 36 generated задач и получить независимые веса
  целевой популяции до реализации binder.
- **Sources:** research sections 11, 13.1, 13.5 and mandatory question 12.
- **Depends on:** T05, T06.
- **Read first:** все family manifests и gap/decision contract.
- **Modify:**
  - add error/retry/resource family;
  - add test-only strengthening family;
  - add `benchmarks/semantic-change/corpus-v1.json`;
  - add `benchmarks/semantic-change/population-v1.json`;
  - add `benchmarks/semantic-change/ecological-sample-v1.json`;
  - add `benchmarks/semantic-change/population-labeling-protocol.json`;
  - add `scripts/verify-semantic-corpus.sh`;
  - add `scripts/validate-population-sample.py`.
- **Product artifacts:** No product artifact update because this task freezes
  an internal research corpus.
- **Steps:**
  1. Довести generated corpus до `>=36` tasks и `>=6` families; в каждом
     family обеспечить positive, ambiguous и must-refuse.
  2. До просмотра binder results preregister supported Kotlin/JVM contour,
     strata, sampling rule, exclusion rules и labeling taxonomy.
  3. Сформировать stratified sample из supported-scope Kotlin Benchmark и
     публичных Kotlin/JVM merged PR/issues, отдельно представив
     backend/service/library и baseline-favourable local syntactic tasks.
  4. Два независимых labeler классифицируют family, semantic obligations,
     expected default/AST sufficiency и supported/refuse status; расхождения
     проходят blind adjudication, agreement публикуется.
  5. Вывести `population-v1.json` weights только из этой выборки с provenance,
     revision/date и denominator. Public tasks не копировать в generator.
  6. Если source sample недоступен или double labeling не завершён, записать
     `POPULATION_UNAVAILABLE`: binder probe разрешён как exploratory, но
     universal applicability gate и `GO_IMPLEMENT` запрещены.
  7. Проверить forbidden vocabulary/structural duplication и создать
     canonical digests corpus, sample и population manifests.
- **Verify:**
  ```bash
  ./scripts/verify-semantic-corpus.sh \
    benchmarks/semantic-change/corpus-v1.json
  python3 scripts/validate-population-sample.py \
    benchmarks/semantic-change/ecological-sample-v1.json \
    benchmarks/semantic-change/population-v1.json
  cargo test -p semantic-corpus
  ```
- **DoD:** corpus frozen до binder code; hidden seed generation привязана к
  frozen revision; все baseline projects проходят compile/tests; population
  weights имеют independent sample/double-label evidence либо universal claim
  механически заблокирован статусом `POPULATION_UNAVAILABLE`.

---

## Gate G1-Corpus

Продолжать T08 в exploratory режиме только если:

- `>=36` tasks, `>=6` families;
- все families имеют positive/ambiguous/must-refuse;
- deterministic public generation и hidden isolation прошли;
- omission/wrong-placement mutants действительно ловятся hidden tests;
- нет закрытого task/repository vocabulary;
- generated corpus weights зафиксированы до получения binder results.

Для будущего universal G1 PASS дополнительно требуется ecological sample с
preregistered contour, public provenance, двумя независимыми labels и
adjudication evidence. Без него разрешён только exploratory binder run.

При fail исправлять только generator/oracle protocol; не писать binder под
частично сформированный corpus.

---

## T08. Ввести versioned Goal, Obligation и Proof schemas

- **Status:** - [ ]
- **Goal:** создать общий язык binding без source edits и transform-specific
  plan payload.
- **Sources:** research sections 8–10.
- **Depends on:** T07, Gate G1-Corpus PASS.
- **Read first:**
  - `crates/sthread/src/model.rs`;
  - `crates/sthread/src/task_plan.rs`;
  - `schemas/worker.proto`;
  - frozen corpus manifests.
- **Modify:**
  - add `crates/sthread/src/change_goal.rs`;
  - add `crates/sthread/src/change_graph.rs`;
  - export modules from `lib.rs`;
  - add JSON schemas under `benchmarks/semantic-change/schema/`.
- **Product artifacts:** No product artifact update because this task defines
  internal semantic IR.
- **Steps:**
  1. Определить typed constraints, obligations, dependencies и status.
  2. Определить `BOUND|AMBIGUOUS|REFUSED` proof result.
  3. Включить в proof обязательный `testOracleClass`:
     `DERIVED|PARAMETRIC|MODEL_AUTHORED|EXTERNAL`; unknown/external semantics
     остаются explicit undischarged obligation или refusal.
  4. Запретить file paths, source substitutions и occurrence counts в goal.
  5. Canonical hash связывает goal/proof с snapshot и corpus task.
- **Verify:**
  ```bash
  cargo test -p sthread change_goal
  cargo test -p sthread change_graph
  ```
- **DoD:** schema round-trip/canonicalization стабильны; invalid mixed
  goal/operations отклоняется; model-owned fields и oracle ownership
  перечислены явно.

---

## T09. Сделать evidence goal-wide и multi-root

- **Status:** - [ ]
- **Goal:** устранить зависимость `task-apply` и revalidation от первого Thread
  IR.
- **Sources:** research sections 2.2, 3.2 and risk register.
- **Depends on:** T08.
- **Read first:**
  - `task_context.rs::build`;
  - `main.rs::TaskApply`;
  - `transaction.rs::revalidate_semantic_read_set`;
  - `model.rs::ThreadIr` and `ReadFact`.
- **Modify:** model/evidence types, context builder, transaction revalidation,
  concurrency tests.
- **Product artifacts:** No product artifact update because this task fixes
  internal correctness scope.
- **Steps:**
  1. Ввести `TaskEvidence`/`GoalReadSet`, агрегирующий все required roots.
  2. Сохранять origin каждого ReadFact и boundary.
  3. Для obligations выбранного probe добавить interprocedural value-flow,
     resolved call/type/override, effect/purity и cross-call placement facts;
     lifecycle/transaction/coroutine, persistence-schema и test-trace gaps
     становятся typed boundaries, а не lexical assumptions.
  4. Revalidate union of required facts before commit.
  5. Удалить implicit first-thread behavior из goal path; legacy low-level path
     оставить versioned или мигрировать явно.
- **Verify:**
  ```bash
  cargo test -p sthread --test concurrency_matrix
  cargo test -p sthread goal_wide_read_set
  ```
- **DoD:** изменение второго required root вызывает stale/conflict; proof
  перечисляет все roots; каждая graph-dependent obligation имеет semantic fact
  либо explicit refusal boundary; legacy behavior не меняется молча.

---

## T10. Реализовать obligation closure и `COMPLETE_FOR`

- **Status:** - [ ]
- **Goal:** заменить глобальную эвристику полноты family-relative proof/refusal.
- **Sources:** research section 7.
- **Depends on:** T09.
- **Read first:**
  - task context selection/build;
  - graph completeness statuses;
  - frozen must-refuse manifests.
- **Modify:** change graph closure, task context boundaries, goal proof output,
  focused negative tests.
- **Product artifacts:** No product artifact update because this task adds an
  internal correctness gate.
- **Steps:**
  1. `COMPLETE_FOR(Family, Goal, Snapshot)` требует discharge всех obligations.
  2. Thread boundaries и omitted required surfaces автоматически запрещают
     `BOUND`.
  3. Fixed top-k становится budget: overflow → `PARTIAL_BUDGET`, не silent
     truncation.
  4. Реализовать 15 negative completeness cases из research.
- **Verify:**
  ```bash
  cargo test -p sthread complete_for
  cargo test -p sthread --test goal_binding must_refuse
  ```
- **DoD:** false `BOUND` отсутствует на public negative set;
  `missing_internal_calls` вычисляется или удалён; failure report указывает
  недоказанную obligation.

---

## T11. Реализовать общие binding primitives

- **Status:** - [ ]
- **Goal:** предоставить reusable semantic predicates вместо source-shape
  macros.
- **Sources:** research section 6 primitives list.
- **Depends on:** T10.
- **Read first:** graph/type/effect model and worker resolution protocol.
- **Modify:** `change_graph.rs`, new `goal_binding.rs`, tests over renamed/decoy
  fixtures.
- **Product artifacts:** No product artifact update because this task builds
  internal semantic reasoning.
- **Steps:**
  1. Реализовать `BindUnique`, `ResolveCallable`, `TypeAssignable`.
  2. Реализовать `IntroduceOnce`, dominance and multiplicity checks.
  3. Реализовать preserve constraints для order/cardinality/laziness,
     nullability/effects/ABI.
  4. Вывести oracle ownership из existing tests/contracts/transformation laws;
     business expected value пометить model-owned, отсутствие спецификации —
     `EXTERNAL` и structured refusal.
  5. Любой unknown boundary возвращает structured refusal.
- **Verify:**
  ```bash
  cargo test -p sthread goal_binding::primitives
  cargo test -p sthread --test metamorphic
  ```
- **DoD:** primitives не читают task names; decoys не меняют binding; ambiguity
  возвращает bounded choices, а не ranking winner.

---

## T12. Реализовать binder `MAP_EDGE_WITH_CONTEXT` без materialization

- **Status:** - [ ]
- **Goal:** проверить первый semantic family только на уровне proof/refusal.
- **Sources:** research section 10.
- **Depends on:** T11.
- **Read first:** MAP_EDGE goal schema, corpus family manifests, current
  `PROPAGATE_TYPED_FIELDS` code only as counterexample.
- **Modify:** goal binder and `crates/sthread/tests/goal_binding.rs`.
- **Product artifacts:** No product artifact update because no production edit
  capability is added.
- **Steps:**
  1. Bind producer `() -> C`, transformer `F(T,C)->T`, source/consumer edge.
  2. Prove evaluation once, placement, modality, types and effects.
  3. Bind behavioral oracle class and required expected values; отсутствие
     repository/task/external oracle — must-refuse до plan.
  4. Return no source, substitutions or EditIR.
  5. Добавить must-refuse для dual candidates, Flow/Sequence, transaction,
     suspend, identity, unknown effect and missing oracle.
- **Verify:**
  ```bash
  cargo test -p sthread --test goal_binding map_edge_with_context
  ```
- **DoD:** public positives bind correctly с explicit oracle class;
  ambiguous/refuse/no-oracle cases не получают plan; median proof payload
  измеряется отдельно от goal.

---

## T13. Перевести existing typed-field path на общий proof binder

- **Status:** - [ ]
- **Goal:** проверить, что constraint/obligation layer способен описать уже
  существующий family без добавления нового production behavior.
- **Sources:** current `task_plan.rs::expand_transient_transform` and research
  sections 3.2/6.
- **Depends on:** T11.
- **Read first:** `task_plan.rs`, Maven fixture tests, goal schemas.
- **Modify:** add binding adapter/proof tests; не менять physical expansion.
- **Product artifacts:** No product artifact update because this is an
  experimental proof adapter.
- **Steps:**
  1. Представить existing four-role path как Change Obligations.
  2. Отделить semantic binding от source-string materialization.
  3. Сравнить old applicability/refusal с proof result.
  4. Зафиксировать source-shape assumptions как undischarged materialization
     obligations.
- **Verify:**
  ```bash
  cargo test -p sthread task_plan
  cargo test -p sthread --test goal_binding propagate_typed_fields_proof
  ```
- **DoD:** semantic proof не использует JPQL/Kotlin substring parsing; existing
  expansion tests остаются зелёными; materialization не объявлена PSI-native.

---

## T14. Выполнить blind goal-binding-only experiment

- **Status:** - [ ]
- **Goal:** получить первое withheld evidence об applicability, correctness и
  model-owned goal size, а также paired cost evidence до редактирования кода.
- **Sources:** research sections 11 and 16–18, especially recommended commit 3.
- **Depends on:** T12, T13.
- **Read first:** frozen corpus digest, run schemas, binding CLI/tests.
- **Modify:**
  - add `scripts/run-goal-binding-benchmark.sh`;
  - add results under `benchmarks/reports/semantic-goal-binding-v1.json`;
  - add blind audit under
    `docs/experiments/semantic-goal-binding-v1-independent-audit.md`;
  - add experiment report in `docs/experiments/`.
- **Product artifacts:** No product artifact update because this task records
  internal benchmark evidence.
- **Steps:**
  1. Freeze binder/prompt/tools, затем generate withheld seeds.
  2. Запустить paired modes с одинаковыми task/base/model/effort/policy:
     default filesystem, AST-index и Codeclew. Каждый mode возвращает один
     versioned goal/obligation artifact; source mutation недоступна.
  3. Default и AST выполняют собственную localization; Codeclew получает только
     bounded semantic context. Один validator применяет одинаковую goal schema
     и hidden obligation comparison ко всем modes.
  4. Для Codeclew binder выдаёт `BOUND|AMBIGUOUS|REFUSED`; context budget
     `16–32 KiB`, а невозможность вместить obligation closure даёт
     `PARTIAL_BUDGET`, не silent truncation.
  5. Randomize mode order; измерить cold/warm, end-to-valid-goal, context bytes,
     goal/proof bytes, turns, raw/cached/output/noncached tokens, navigation
     calls, unresolved business choices и все failure reasons.
  6. На size strata проверить, что Codeclew context зависит от obligation
     closure, а не repository LOC; отдельно показать cohorts, где default/AST
     ожидаемо достаточны или быстрее.
  7. После закрытия серии независимый reviewer без run history проверяет hidden
     manifests, digests, exclusions, formulas, oracle ownership и отсутствие
     vocabulary leakage. До его разрешающего verdict G1 не вычисляется.
- **Verify:**
  ```bash
  ./scripts/run-goal-binding-benchmark.sh --corpus \
    benchmarks/semantic-change/corpus-v1.json \
    --modes default,ast-index,codeclew --verify-only
  python3 scripts/validate-semantic-run.py \
    benchmarks/reports/semantic-goal-binding-v1.json
  ```
- **DoD:** отчёт содержит все 36+ tasks × 3 modes; false-complete,
  must-refuse, paired cost и scaling посчитаны; отсутствующие tokens не
  заменены bytes; independent blind audit разрешает вычисление decision gate.

---

## Gate G1-Binder

### PASS к PSI materialization

- false complete = `0`;
- must-refuse accuracy = `100%`;
- correct binding `>=90%` applicable tasks;
- median goal `<=1 KiB`;
- median clarification turns `<=1`;
- no repository vocabulary;
- applicability `>=60%` целевой independently weighted sample;
- every `BOUND` has explicit oracle class and no undischarged required fact;
- independent G1 audit verdict = `ACCEPT`;
- ecological population status = `AVAILABLE` with double-label evidence.

### CONDITIONAL

Если correctness gates пройдены, но applicability `40–59.99%`, разрешена ровно
одна дополнительная binder-only итерация по наиболее частому **заранее
взвешенному** unsupported family. Corpus и weights не меняются. После неё G1
пересчитывается один раз.

### STOP/NARROW

- applicability `<40%` → `STOP_NOT_PLAUSIBLE` для универсальной линии;
- correctness/refusal gate fail → исправить semantic evidence/binder и
  повторить на новом withheld seed set, сохранив failed series;
- coverage остаётся `<60%` после conditional iteration →
  `NARROW_FEATURE_ONLY`, без universal claim.
- population status `UNAVAILABLE` или independent G1 audit не принят → G1 не
  может получить PASS; допускается только публикация exploratory results.

T15–T23 запрещены, пока G1 не имеет machine-readable `PASS`.

---

## T15. Спроектировать PSI-native semantic edit protocol

- **Status:** - [ ] CONDITIONAL ON G1-Binder PASS
- **Goal:** заменить textual source plan для выбранного family набором typed
  semantic operations.
- **Sources:** research section 12.
- **Depends on:** T14, Gate G1-Binder PASS.
- **Read first:** worker proto, EditOperation model, preview pipeline, winning
  binder proof shapes.
- **Modify:** `schemas/worker.proto`, `model.rs`, protocol docs, golden tests.
- **Product artifacts:** No product artifact update because this task evolves
  internal edit protocol.
- **Steps:**
  1. Добавить только primitives, необходимые proven family.
  2. Каждая operation содержит SymbolId/type/receiver/parameter/preconditions,
     не old/new source.
  3. Protocol version bump и explicit older-worker refusal.
  4. Define postconditions tied to discharged obligations.
- **Verify:**
  ```bash
  cargo test -p sthread semantic_edit_protocol
  cargo test -p sthread --test golden_language
  ```
- **DoD:** goal/proof детерминированно компилируются в semantic ops; textual
  substitution не требуется выбранному family; backward compatibility явная.

---

## T16. Реализовать PSI-native operations в Kotlin worker

- **Status:** - [ ] CONDITIONAL ON G1-Binder PASS
- **Goal:** materialize semantic ops через Kotlin PSI/K2 без Rust text
  normalizers.
- **Sources:** T15 protocol and research materialization safety section.
- **Depends on:** T15.
- **Read first:** `Worker.kt::applyEdit`, declaration/import rewrite code,
  K2 candidate validation.
- **Modify:** common Kotlin worker, version adapters if required, worker tests.
- **Product artifacts:** No product artifact update because this task changes
  internal compiler integration.
- **Steps:**
  1. Реализовать targeted PSI operations для selected family.
  2. Resolve imports/calls/arguments через K2 symbols.
  3. Reanalyze candidate и validate type/effect/order obligations.
  4. Strings/comments/decoys не должны быть edit targets.
- **Verify:**
  ```bash
  ./gradlew :workers:kotlin:test :workers:kotlin21:test :workers:kotlin23:test
  cargo test -p sthread --test kotlin21
  cargo test -p sthread --test maven
  ```
- **DoD:** PSI-native mutations проходят все version workers; negative binding
  не materializes; old text path не вызывается chosen operation.

---

## T17. Применить proof-level oracle policy и mutation gate

- **Status:** - [ ] CONDITIONAL ON G1-Binder PASS
- **Goal:** применить уже доказанный в T08–T12 oracle ownership к
  materialization и не считать self-confirming generated test evidence.
- **Sources:** research section 11.
- **Depends on:** T15.
- **Read first:** context test selection, validationPlan, hidden mutation
  manifests.
- **Modify:** corpus controller, test selection/materialization gate tests; goal
  proof schema не меняется после G1.
- **Product artifacts:** No product artifact update because this task hardens
  internal test evidence.
- **Steps:**
  1. Проверить proof hash и принять только уже классифицированный
     `DERIVED|PARAMETRIC|MODEL_AUTHORED`; `EXTERNAL` остаётся refusal.
  2. Для generated/strengthened tests запускать omission и wrong-placement
     mutants.
  3. Public declared contract проверять отдельно от concrete subtype.
  4. External/unknown oracle запрещает automatic accepted commit.
- **Verify:**
  ```bash
  cargo test -p semantic-corpus mutation_gate
  cargo test -p sthread test_oracle_classification
  ```
- **DoD:** self-confirming tests rejected; relevant mutants killed; model-owned
  expected values явно присутствуют в goal/receipt; T17 не может повысить
  `REFUSED/EXTERNAL` proof до materializable.

---

## T18. Подключить materialization одного proven family

- **Status:** - [ ] CONDITIONAL ON G1-Binder PASS
- **Goal:** end-to-end путь goal → proof → semantic EditIR → transaction для
  family с наибольшей preregistered coverage.
- **Sources:** G1 report and T15–T17.
- **Depends on:** T16, T17.
- **Read first:** exact G1 winning family report and all must-refuse outcomes.
- **Modify:** task apply route, plan/goal compiler, focused integration tests.
- **Product artifacts:** No product artifact update because this task adds an
  internal editing capability.
- **Steps:**
  1. Accept only goal schema/proof hash; low-level operations mixed with goal
     reject.
  2. Compile proof to PSI-native ops and preserve GoalReadSet.
  3. Receipt includes proof, discharged obligations and oracle class.
  4. Must-refuse cases stop before candidate creation.
- **Verify:**
  ```bash
  cargo test -p sthread --test semantic_change selected_family_end_to_end
  cargo test -p sthread --test semantic_change selected_family_must_refuse
  ```
- **DoD:** positive Gradle/Maven tasks commit cleanly; ambiguous/refuse tasks do
  not edit; no repository vocabulary or source-shape parser in new route.

---

## T19. Усилить concurrency, recovery и full corpus validation

- **Status:** - [ ] CONDITIONAL ON G1-Binder PASS
- **Goal:** доказать, что goal-wide transaction сохраняет существующие safety
  guarantees.
- **Sources:** transaction/correctness model and research risk register.
- **Depends on:** T18.
- **Read first:** concurrency matrix, transaction ledger/recovery, GoalReadSet.
- **Modify:** goal concurrency/recovery tests and semantic corpus transaction
  runner.
- **Product artifacts:** No product artifact update because this task validates
  internal transaction safety.
- **Steps:**
  1. Concurrent change любого required root invalidates commit.
  2. Unrelated change may replay without losing proof obligations.
  3. Crash before/after CAS recovers ref/index/ledger consistently.
  4. Run every applicable public corpus task; every refusal remains pre-edit.
- **Verify:**
  ```bash
  cargo test -p sthread --test concurrency_matrix
  cargo test -p sthread --test semantic_goal_concurrency
  cargo test --workspace
  ./scripts/verify.sh
  ```
- **DoD:** Gate G2 report has zero incorrect commits; GoalReadSet revalidates all
  roots; full workspace green.

---

## Gate G2 — Materialization correctness

Разрешить comparative benchmark только если:

- correctness не ниже oracle baseline;
- false commit = `0`;
- must-refuse pre-edit = `100%`;
- all applicable public/withheld tasks pass hidden acceptance;
- no textual edit path for selected family;
- goal-wide concurrency/recovery tests pass;
- full `cargo test --workspace` and `./scripts/verify.sh` pass.

---

## T20. Расширить ранний paired harness до полного end-to-end

- **Status:** - [ ] CONDITIONAL ON G2 PASS
- **Goal:** переиспользовать T14 harness для полных accepted commits default,
  AST-index и Codeclew с одинаковыми задачами и telemetry contract.
- **Sources:** research section 14, T01 schemas and T14 paired artifacts.
- **Depends on:** T19, Gate G2 PASS.
- **Read first:** T14 harness/audit, benchmark scripts/reports and frozen corpus
  controller.
- **Modify:** agent run manifests, orchestration scripts, telemetry ingest and
  validation.
- **Product artifacts:** No product artifact update because this task adds
  internal benchmarking infrastructure.
- **Steps:**
  1. Extend, не дублировать T14 mode adapters and telemetry ingestion.
  2. Same task/base/model/effort/system policy per pair.
  3. Randomize mode order; separate cold/warm.
  4. Agent never sees hidden controller manifest.
  5. Native token events mandatory for token ranking.
  6. Record all failures and exact time origin.
- **Verify:**
  ```bash
  ./scripts/benchmark-semantic-change.sh --dry-run \
    --corpus benchmarks/semantic-change/corpus-v1.json
  ```
- **DoD:** dry run creates valid E2E manifests for all modes using the already
  audited T14 adapters; mode-specific tools isolated; no solution leakage;
  timestamps/tokens validate.

---

## T21. Выполнить paired comparative series

- **Status:** - [ ] CONDITIONAL ON G2 PASS
- **Goal:** получить correctness-first end-to-end evidence на withheld corpus.
- **Sources:** G2 artifact, paired protocol, population weights.
- **Depends on:** T20.
- **Read first:** run protocol and exclusion rules.
- **Modify:** immutable run artifacts only; no worker/corpus edits during series.
- **Product artifacts:** No product artifact update because this task executes
  a frozen benchmark.
- **Steps:**
  1. Freeze Codeclew, generator, prompts and tool versions.
  2. Run all modes on independent worktrees with randomized order.
  3. Hidden verifier judges commit before performance ranking.
  4. Reveal oracle manifests only after final run.
  5. If infra failure matches preregistered exclusion, rerun once and retain both
     records.
- **Verify:**
  ```bash
  ./scripts/benchmark-semantic-change.sh --verify-results \
    benchmarks/runs/semantic-change-v1
  ```
- **DoD:** every task/mode has outcome; no missing telemetry in ranked runs;
  rejected patches excluded from performance winner but retained in totals.

---

## T22. Рассчитать статистику и провести blind acceptance audit

- **Status:** - [ ] CONDITIONAL ON G2 PASS
- **Goal:** исключить cherry-picking и сформировать paired confidence evidence.
- **Sources:** T21 raw run artifacts.
- **Depends on:** T21.
- **Read first:** all run manifests and hidden verdicts; no prior summary.
- **Modify:** statistics script, machine-readable report, independent audit
  report.
- **Product artifacts:** No product artifact update because this task validates
  benchmark evidence.
- **Steps:**
  1. Compute all-run and applicable-only views.
  2. Compute paired median, accepted win rate, bootstrap 95% CI and family
     weighted estimates.
  3. Report cold/warm separately.
  4. Для каждого run и cohort вычислить `f_build`, theoretical maximum speedup,
     model/discovery/materialization shares и impacted-test contribution.
  5. На repository size strata проверить context/output scaling; отдельно
     опубликовать local-syntactic, AST-sufficient, semantic-applicable и
     build-dominated cohorts без скрытия baseline wins.
  6. Independent reviewer verifies samples, formulas, exclusions and absence of
     repository vocabulary.
- **Verify:**
  ```bash
  python3 scripts/analyze-semantic-benchmark.py \
    benchmarks/runs/semantic-change-v1 \
    --output benchmarks/reports/semantic-change-v1.json
  python3 scripts/validate-semantic-run.py \
    benchmarks/reports/semantic-change-v1.json
  ```
- **DoD:** formulas reproducible; audit verdict recorded; every claim maps to
  run IDs; `f_build` ceiling и repository scaling проверены; historical closed
  runs not mixed into sample.

---

## T23. Зафиксировать финальное product/architecture decision

- **Status:** - [ ] CONDITIONAL ON G2 PASS
- **Goal:** принять `GO_IMPLEMENT`, `NARROW_FEATURE_ONLY` или
  `STOP_NOT_PLAUSIBLE` по preregistered thresholds.
- **Sources:** G1, G2 and paired benchmark reports.
- **Depends on:** T22.
- **Read first:** research results, `decisions.json`, all final reports and audit.
- **Modify:**
  - update `benchmarks/semantic-change/decisions.json`;
  - add final experiment document;
  - update architecture ADR only if decision is GO/NARROW.
- **Product artifacts:** No product artifact update because this task records an
  engineering product decision, not user-visible flow.
- **Steps:**
  1. Evaluate each threshold with exact numerator/denominator/CI.
  2. List falsifiers and unsupported families.
  3. If GO, define next family/materialization backlog without changing corpus
     retroactively.
  4. If NARROW/STOP, preserve useful transaction/index platform and remove
     universal performance claim from roadmap.
  5. Без AVAILABLE ecological population, accepted independent audits или
     native token telemetry итог не может быть `GO_IMPLEMENT`; недоступная
     token telemetry не превращается автоматически в STOP, но token-win claim
     остаётся `UNAVAILABLE`.
- **Verify:**
  ```bash
  python3 scripts/validate-semantic-decisions.py \
    benchmarks/semantic-change/decisions.json
  git diff --check
  ```
- **DoD:** decision is mechanically derived from frozen thresholds; confidence
  and limitations explicit; no work remains implicitly authorized beyond the
  verdict.

---

## Финальный чек плана

```bash
plan=docs/superpowers/plans/2026-08-02-codeclew-corpus-first-plan.md
grep -nE '^- \*\*Status:\*\* - \[ \]' "$plan" || true
```

План считается исполненным только после T23 либо после зафиксированного
`STOP/NARROW` на Gate G1. Conditional tasks не отмечаются выполненными при
ранней остановке: вместо этого в их Status записывается
`SKIPPED: <gate verdict>`.

## Известные пробелы покрытия

- План не гарантирует доступ к provider-level token telemetry; без неё token
  ranking должен быть `UNAVAILABLE`, а не оценочным.
- External ecological validation зависит от доступности публичных Kotlin
  issues/PRs и двух независимых labelers; T07 блокирует universal claim, если
  выборку или double labeling нельзя получить.
- JPQL/query semantics не становятся PSI-native автоматически и могут остаться
  отдельным отказным boundary.
- Android, KMP, scripts, reflection-heavy frameworks и arbitrary compiler
  plugins остаются вне текущего supported contour.
- Формальная верификация business oracle невозможна без внешней спецификации.
- Build/test dominated tasks могут не пройти speed threshold даже при успешном
  goal binding; это корректный отрицательный результат.

## Self-review данного planning artifact

Это self-review, **не независимая верификация**. Проверено:

- corpus создаётся и замораживается до binder;
- production materialization заблокирована G1;
- G1 отделяет correctness/applicability от speed;
- paired goal-binding comparison выполняется до materialization, а полный E2E
  benchmark начинается только после PSI-native correctness G2;
- исторические закрытые repos не являются runtime dependency;
- failed/refused runs не теряются;
- token claims невозможны без native telemetry;
- есть явный путь к `GO`, `NARROW` и `STOP`.
