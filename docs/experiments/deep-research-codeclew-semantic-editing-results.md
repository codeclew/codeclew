# Codeclew: исследование универсального семантического изменения кода

Дата: 2 августа 2026 года
Репозиторий: `example-user/codeclew`
Аудированный revision: `7fc3e0d6c6e784a130245ef0e344535a146324c7`
Итоговый вердикт: `GO_BUILD_CORPUS_FIRST`
Уверенность: `0,88`

---

## 0. Резюме решения

Codeclew уже является не концептом, а работающим Kotlin/JVM-прототипом с
сильным инфраструктурным основанием:

- Rust-ядро хранит индекс, строит локальные SSA/def-use/control-dependency
  graphs, формирует Thread IR и управляет транзакциями;
- version-pinned Kotlin/JVM workers используют PSI, K2/FIR и Gradle/Maven
  project model;
- изменения применяются через semantic anchors, проходят preview,
  K2-проверки, detached worktree, compile/tests и Git compare-and-swap;
- индекс публикуется отдельно и атомарно, предусмотрены rollback и recovery.

Однако существующее evidence не доказывает универсальное преимущество
Codeclew. На текущем HEAD:

1. `COMPLETE_TASK` является эвристической полнотой task-context, а не
   доказанной полнотой относительно конкретного класса изменений.
2. Task surface выбирается по фиксированным top-k лимитам и
   лексическим/структурным эвристикам.
3. `task-apply` берёт первый Thread IR, хотя task context может быть построен из
   нескольких semantic roots.
4. Единственный semantic-goal compiler — `PROPAGATE_TYPED_FIELDS`; он уменьшает
   model-authored output, но компилируется через семейство source-shape
   assumptions и текстовых разборов Kotlin/JPQL/test code.
5. Физический `REWRITE_DECLARATION` остаётся exact/regex text substitution
   внутри найденной PSI-declaration, а не PSI-native semantic mutation.
6. Успешный transient benchmark относится к одной задаче на одном закрытом
   репозитории после нескольких итераций настройки на той же задаче.
7. Нет neutral withheld corpus, по которому можно оценить applicability,
   false-completeness, must-refuse accuracy и win rate на заранее
   зафиксированной популяции задач.

Поэтому:

> **Не следует сейчас добавлять второй production transform. Сначала следует
> построить нейтральный withheld corpus, формализовать family-relative
> completeness и провести слепой goal-binding experiment без materialization.**

`STOP_NOT_PLAUSIBLE` также не обоснован: существующий accepted benchmark и
кодовая архитектура показывают, что компактный semantic goal способен убрать
значительную часть model-owned localization/plan work. Нужно проверить,
является ли это общим механизмом или эффектом одного структурного семейства.

---

## 1. Дисциплина доказательств и ограничения исследования

В отчёте используются метки:

- `CODE` — непосредственно подтверждено исходниками на audited revision.
- `TEST` — подтверждено тестом, заново запущенным в рамках исследования.
- `ARTIFACT` — подтверждено сохранённым JSON, документом или Git history, но
  исходный benchmark repository/rollout недоступен.
- `LITERATURE` — подтверждено первичным внешним источником.
- `INFERENCE` — логический вывод из перечисленных предпосылок.
- `HYPOTHESIS` — требует будущего эксперимента.

### Ограничения

`CODE` Аудит выполнен по public GitHub source на revision `7fc3e0d...`.

`TEST` Новых test-run evidence в исследовании нет. GitHub не показывает CI
statuses для audited commit, а локальное клонирование из execution environment
невозможно из-за отсутствия сетевого DNS-доступа. Наличие тестов и
`scripts/verify.sh` подтверждает тестовый контур как код, но заявления
документации о прохождении тестов классифицируются как `ARTIFACT`, а не как
заново воспроизведённый `TEST`.

`ARTIFACT` Исторические `pim-migrator` и `product-repo` benchmarks используются
только как наблюдения. Закрытые repositories, worktrees и исходные rollout
transcripts недоступны.

`INFERENCE` Отсутствие нового test execution снижает уверенность в деталях
реализации, но не меняет основной verdict: даже при принятии всех сохранённых
benchmark results нет multi-family withheld evidence.

---

## 2. Architecture map фактического pipeline

### 2.1. Верхнеуровневый поток

```text
Task text + explicit terms
        |
        v
OpenProject / project model
        |
        v
Syntax-only declaration index
        |
        v
task_context::select
  lexical requirements
  exact targets
  ranked roots
        |
        v
K2 ResolveSymbol for roots/follow-ups
        |
        v
FIR/PSI local graph -> Rust SSA/def-use/control deps
        |
        v
Thread IR(s) + full evidence
        |
        v
task_context::build
  edit surfaces
  contracts
  execution path
  projection fields
  tests
  bounded stdout
        |
        v
Model-owned artifact
  A. compact semantic goal
  or
  B. low-level operation plan
        |
        v
task_plan expansion + main.rs normalization
  targets
  substitutions
  imports
  overrides
  tests
        |
        v
EditIR
        |
        v
semantic preview
  target re-resolution
  PSI parse
  K2 diagnostics/types/bindings/effects
  WriteSet/ABI checks
        |
        v
detached worktree
  Gradle/Maven compile
  configured tests
        |
        v
candidate commit
        |
        v
semantic replay if HEAD moved
        |
        v
Git ref CAS + staged index publication + ledger
```

### 2.2. Этапы с ownership и полнотой

| Этап | Кодовая точка | Вход → выход | Характер информации | Worker/Core | Model turns и ошибки |
| --- | --- | --- | --- | --- | --- |
| Project model | `workers/kotlin/.../Worker.kt`, `inspect`, `gradleModel`, Maven extractor | repo + compilation → source roots, classpath, compiler options, tasks, hash | Полная только для поддержанного Gradle/Maven contour | Kotlin worker | Без модели. Unsupported project fail-closed |
| Syntax index | `main.rs::AgentContext`, `IndexFiles`; `index.rs` | project → declaration facts + persistent snapshot | Repository-wide syntax; semantic enrichment on demand | Kotlin worker + Rust/SQLite | Без модели. Hash/invalidation errors до planning |
| Task selection | `task_context.rs::select`, `root_symbols`, `followup_symbols` | task text/terms + catalog → requirements, roots, exact candidates | **Эвристическая**: tokens, names, source text, ranking | Rust core | Может сэкономить queries, но false selection обнаруживается поздно |
| Semantic resolution | `ResolveSymbol`, `BuildLocalGraph` | roots → K2 facts, anchors, calls, CFG | Полная для выбранных declarations и поддержанного worker contour | Kotlin worker | Без модели. Ambiguous/missing symbol fail-closed |
| Graph enrichment | `graph.rs::enrich` | local CFG → AST/type/effect, SSA, def-use, control dep | Локальная функция; calls — summary boundaries | Rust core | Без модели. External/unsupported boundaries в Thread IR |
| Thread slicing | `graph.rs::slice` | seed + policy → Thread IR + ReadSet | Bounded; explicit partial status для graph budget/external calls | Rust core | Без модели |
| Task surface | `task_context.rs::build` | selection + resolutions + threads → surfaces/contracts/tests/context | **Эвристическая**, top-k bounded; отдельная от Thread completeness | Rust core | Плохая surface closure заставляет модель дочитывать или ошибаться |
| Goal/plan | model | context → semantic goal или low-level plan | Полностью model-owned бизнес-решения; объём зависит от abstraction level | Model | Главный варьируемый источник tokens/turns |
| Goal compilation | `task_plan.rs::expand_transient_transform` | `PROPAGATE_TYPED_FIELDS` → operations | Детерминированно только при узком source shape | Rust core, но с source-text heuristics | Fail/refusal при нестандартной форме |
| Plan normalization | `main.rs` helpers | operations → anchored EditIR | Частично semantic, частично text-normalization | Rust core | Ошибки могут проявиться в preview/build |
| Preview | `transaction.rs::preview`; Kotlin `applyEdit` | EditIR + snapshot → candidates, diff, diagnostics, WriteSet | Сильная structural/semantic проверка | Rust + Kotlin worker | Repair turn при reject |
| Build/tests | `transaction.rs::validate_worktree` | candidate files → compile/test evidence | Реальная compiler/test validation | External Gradle/Maven | Дорогой этап; tests не гарантируют полный behavioral oracle |
| Commit | `transaction.rs::commit` | validated candidate → Git commit/index/ledger | Snapshot isolation, CAS, recovery | Rust core | Без модели при отсутствии conflict |
| Rebase | `preview_for_commit`, `revalidate_semantic_read_set` | moved HEAD → rebuilt first-thread ReadSet + replay | Сильнее line merge, но ограничено сохранённым Thread ReadSet | Rust + Kotlin worker | Conflict/reslice вместо silent merge |

### Ключевое наблюдение

`CODE` Semantic transaction subsystem существенно сильнее task-selection и
goal-compilation layers. У него есть detached worktree, semantic replay,
ReadSet comparison, compile/tests, commit trailers, target-ref CAS, staged
index and rollback/recovery.

`INFERENCE` Поэтому главный риск Codeclew сегодня — не «сломать Git при
применении», а:

1. выбрать полный task surface;
2. сформулировать достаточно общий, но компактный goal;
3. доказать applicability/refusal;
4. скомпилировать goal в semantic edits без source-shape recipes;
5. получить корректный behavioral oracle.

---

## 3. Что фактически доказано исходниками

### 3.1. Сильные стороны

#### Version-isolated compiler integration

`CODE` Kotlin 2.1.21, 2.3.0 и 2.4.10 workers изолированы, protocol versioned,
compiler objects не пересекают process boundary.

#### Persistent project/index model

`CODE` RepositoryIndex хранит file/declaration facts в SQLite/WAL,
content-addressed source blobs, project/classpath/compiler-options hashes и
typed invalidations. Новая index database строится в staging file и
публикуется atomic rename.

#### Local semantic graph

`CODE` Kotlin worker экспортирует K2/FIR-validated local CFG,
call/type/receiver/effect facts и anchors. Rust добавляет SSA, `PHI_INPUT`,
`DEF_USE` и `CONTROL_DEP`.

#### Explicit Thread boundaries

`CODE` `graph::slice` маркирует `PARTIAL_BUDGET`,
`PARTIAL_UNSUPPORTED_FEATURE` и `PARTIAL_EXTERNAL_BOUNDARY`. External call при
`maxCallDepth=0` не объявляется полным.

#### Anchor safety

`CODE` Edit target определяется owner SymbolId, syntax kind, token hash, exact
hash и contextual tie-breakers. Ноль targets → stale; несколько → ambiguous.
Worker не выбирает «самый похожий» target.

#### Semantic preview

`CODE` Для expression/body changes worker сравнивает K2 diagnostics, protected
semantic facts, target/replacement types, effects, signature/body/ABI/summary
deltas.

#### Transactionality

`CODE` Commit выполняется в detached worktree. После compile/tests создаётся
candidate commit. Target ref меняется через compare-and-swap. Index publication
происходит через staged database; при ошибке ref откатывается. Ledger может
восстановить outcome по Git trailers.

### 3.2. Что не доказано или доказано только частично

#### `COMPLETE_TASK` не равно semantic completeness

`CODE` В `task_context::build` статус `COMPLETE_TASK` определяется отсутствием
локального массива task-context boundaries. В него не включается
`ThreadIr.completeness`.

`CODE` Переменная `missing_internal_calls` прямо установлена в `0`, поэтому
boundary `UNRESOLVED_INTERNAL_CALLS` фактически недостижим.

`CODE` `TaskApply` проверяет `COMPLETE_TASK`, затем берёт только первый Thread IR
из evidence.

`INFERENCE` Task context может быть объявлен полным при частичном local slice,
external call boundary или неполном ReadSet относительно других edit surfaces.
Это не доказывает наличие конкретного false commit, но доказывает, что текущий
статус не является формальным `COMPLETE_FOR(goal)`.

#### Task surface сильно зависит от heuristics

`CODE` Role assignment использует entrypoint/exact/root status,
`is_contract_source`, наличие `@Query` и ranked call declarations. Tests
выбираются по task needles. Жёсткие лимиты:

```text
edit surfaces: 4
contracts: 2
tests: 1
execution edges: 4
source per surface: 4200 bytes
```

`INFERENCE` Эти пределы контролируют payload, но сами по себе не доказывают
closure of obligations.

#### Goal compiler только один и structural-family specific

`CODE` `task_plan.rs` поддерживает единственный kind:
`PROPAGATE_TYPED_FIELDS`.

`CODE` Он ожидает ровно по одному `WORKFLOW`, `INTERMEDIARY`,
`OUTPUT_CONTRACT`, `DATA_SOURCE`, один contract, один test и projection fields.

`CODE` Он разбирает source strings для:

- class header shape `\n) {\n`;
- JPQL `SELECT ... FROM`;
- `List<T>` return type;
- `val x = method(...)`;
- `collection.forEach { item ->`;
- named argument shape;
- Mockito-style `anyOrNull()`;
- test `.forEach { expected -> ... }`.

`INFERENCE` Это generic относительно имён repository, но не generic
относительно program structure/framework conventions. Это family compiler, а
не универсальный semantic-goal compiler.

#### Materialization не полностью PSI-native

`CODE` `REWRITE_DECLARATION` ищет declaration через PSI, но внутри declaration
выполняет exact/regex substitutions по тексту, затем reparses result.

`CODE` Imports перестраиваются через textual source assembly. `main.rs` также
нормализует extension calls, imports и contract overrides до worker apply.

`INFERENCE` Parser+compile validation уменьшают риск, но не устраняют semantic
ambiguity до candidate construction и могут создавать дорогой repair turn.

---

## 4. Bottleneck table

| Компонент | Что известно | Evidence type | Текущий вывод |
| --- | --- | --- | --- |
| Worker startup | ~349 ms p95 в сохранённом benchmark | `ARTIFACT` | Не является главным bottleneck |
| Cold semantic index, fixture | ~1.57 s p95 | `ARTIFACT` | Дешёв относительно model rollout |
| 100k LOC syntax index | ~8.14 s | `ARTIFACT` | Приемлем как cold repository cost |
| Local CFG+SSA | ~41 ms p95 | `ARTIFACT` | Graph algorithms не доминируют |
| Preview | ~37 ms p95 на fixture | `ARTIFACT` | Сам EditIR preview дешёв на малом project |
| Compile/test fixture | ~2.56 s + 2.63 s | `ARTIFACT` | На production project может доминировать |
| Real Maven context | ~30.4 s в accepted transient run | `ARTIFACT` | Cold K2/project context уже значим |
| Real Maven transaction | ~46.4 s | `ARTIFACT` | Build/test/materialization значимы |
| Model low-level plan | 2,985 B и 2 validator attempts в PIM surface experiment | `ARTIFACT` | Bounded context не гарантирует малую model-owned работу |
| Transient goal | 398 B, один validator attempt | `ARTIFACT` | Semantic goal может существенно сократить output/retry |
| Test source/oracle | В PIM wiring scenario осталось model-owned | `ARTIFACT` + `CODE` | Не решается одной navigation architecture |
| Exact model-time decomposition | Не сохранена полностью для всех historical runs | — | Нельзя назвать точный процент model bottleneck |
| Applicability across task families | Нет neutral corpus | — | Неизвестна |

### Ответ о доминирующем bottleneck

`INFERENCE` В общем pipeline доминирующий model-owned bottleneck сегодня — не
raw discovery, а переход:

```text
bounded context
    -> выбор semantic obligations
    -> low-level multi-file plan
    -> behavioral test oracle
    -> repair after validation
```

В одной задаче `PROPAGATE_TYPED_FIELDS` переносит большую часть binding/plan
work в deterministic compiler, и bottleneck смещается в cold project context +
build/tests. Но нет основания утверждать, что этот сдвиг произойдёт на
большинстве задач.

---

## 5. Формальная модель стоимости

### 5.1. Общая модель

```text
T_total =
    T_project_model
  + T_discovery_worker
  + T_model_context_reasoning
  + T_model_goal_or_plan
  + T_goal_binding
  + T_plan_validation
  + T_materialization
  + T_compile
  + T_tests
  + T_repair_turns
  + T_commit
```

```text
Tokens_total =
    Σ(system + task)
  + Σ(model-visible context at turn i)
  + Σ(prior transcript replay at turn i)
  + Σ(tool diagnostics at turn i)
  + model-authored goal/plan/test output
```

```text
Noncached_tokens =
    Σ(raw input_i - cached input_i + output_i)
```

Bytes нужно учитывать отдельно:

```text
Visible_bytes =
    navigation payload
  + source snippets
  + goal/plan bytes
  + diagnostics bytes
```

Bytes не являются корректной заменой token telemetry.

### 5.2. Default filesystem workflow

```text
T_default =
    q_rg * T_rg
  + q_read * T_read
  + q_turn * T_model_turn
  + T_patch_generation
  + T_build/tests
  + T_repairs
```

Особенность: каждый дополнительный turn может повторно включать значительную
часть transcript. Даже дешёвый `rg` может быть дорогим end-to-end, если требует
последовательности:

```text
search -> read -> infer -> search dependency -> read test -> patch -> repair
```

### 5.3. AST-index workflow

```text
T_ast =
    T_ast_index
  + q_ast * (T_query + T_model_query_decision)
  + q_read * T_source_read
  + T_patch_generation
  + T_build/tests
  + T_repairs
```

AST index уменьшает стоимость и payload отдельного navigation query, но не
обязательно:

- уменьшает число model decisions;
- закрывает data/control/effect dependencies;
- определяет полный change surface;
- генерирует coherent multi-file patch;
- выводит behavioral oracle.

### 5.4. Codeclew workflow

```text
T_codeclew =
    T_project_model
  + T_semantic_index
  + T_task_surface
  + T_model_goal
  + T_goal_compile
  + T_semantic_preview
  + T_build/tests
  + T_repairs
  + T_atomic_commit
```

Codeclew выигрывает, когда:

```text
avoided(
    repeated search/read turns
  + transcript replay
  + model-authored source
  + failed placements/imports
  + repair iterations
)
>
added(
    cold project/K2 analysis
  + task-surface construction
  + goal binding/proof
)
```

### 5.5. Условия масштабируемого выигрыша

Преимущество сохраняется при росте repository size только если:

```text
|model context| = O(|task obligations| + |task surfaces|)
```

а не `O(|repository|)`, и:

```text
|model output| = O(|irreducible business choices| + |semantic ambiguities|)
```

а не `O(|textual patch|)`.

Рекомендуемые верхние границы как hypothesis:

```text
model-visible task context: 16–32 KiB
semantic goal: <= 1 KiB для typical applicable task
clarification turns: 0 или 1
goal candidates before refusal: bounded constant
```

Это не correctness criteria: при нехватке бюджета система должна вернуть
`PARTIAL/REFUSE`, а не обрезать обязательную информацию.

### 5.6. Build-dominated tasks

Пусть:

```text
f_build = (T_compile + T_tests) / T_total
```

Даже идеальное устранение всей model/discovery части даёт:

```text
max relative speedup <= 1 - f_build
```

Если build/tests занимают 75% end-to-end, добиться 30% общего сокращения
невозможно без:

- более точного impacted-test routing;
- incremental build reuse;
- parallel validation;
- pre-build static refusal;
- уменьшения repair builds.

### 5.7. Нижняя граница model-owned информации

`INFERENCE` Worker не может корректно вывести информацию, которой нет в
repository/task specification:

- новый публичный термин или имя, если допустимы несколько;
- business expected value;
- предпочтение между несколькими behaviorally valid designs;
- внешнюю policy;
- intended compatibility trade-off;
- behavioral oracle, отсутствующий в code/contracts/tests.

Следовательно, цель не «убрать модель», а свести model-owned artifact к этой
irreducible information.

---

## 6. Intent, goal, plan и EditIR: альтернативы

| Вариант | Model-owned artifact | Универсальность | Output/turns | Проверяемость до edit | Главный риск |
| --- | --- | --- | --- | --- | --- |
| 1. Textual substitutions | old/new source, occurrences, files | Высокая номинально | `O(patch text)` | Слабая | fragile placement, large output, repair turns |
| 2. Transform catalog | kind + parameters | Хорошая в известных families | Очень малая | Сильная | recipe explosion и overfitting |
| 3. Typed constraints/postconditions | obligations + business values | Лучшая целевая модель | `O(choices + ambiguities)` | Сильная через binder/proof | сложность solver и family completeness |
| 4. Model-authored graph delta | semantic nodes/edges | Очень выразительная | `O(semantic delta)` | Средняя | model должен понимать unstable internal graph; неверные graph IDs |

### Рекомендация

Нужен гибрид:

> **Небольшой типизированный constraint language над semantic obligations +
> ограниченный набор сертифицированных compiler strategies.**

Не следует предоставлять модели unrestricted graph editor. Graph delta должен
быть внутренним результатом binder.

Не следует делать flat macro catalog. Каждый family compiler должен быть
композицией общих primitives:

```text
BindUnique
ResolveCallable
TypeAssignable
IntroduceOnce
MapEdge
PropagateType
PreserveOrder
PreserveCardinality
PreserveLaziness
PreserveEffects
PreserveNullability
PreserveABI
RequireOracle
MustRefuseOnBoundary
```

Transform family допустим как reusable strategy, если:

1. его schema не содержит repository/task vocabulary;
2. applicability определяется semantic predicates;
3. bindings выводятся из compiler evidence;
4. ambiguity приводит к explicit choice/refusal;
5. каждый application возвращает proof object;
6. он проходит generated renaming/layout/decoy cases;
7. withheld seeds созданы после freeze compiler.

---

## 7. Полнота task surface

### 7.1. Текущая формула

```text
TaskSurface =
    EntryPoints
  ∪ ExactTargets
  ∪ BoundedGraphClosure
  ∪ Contracts
  ∪ Tests
```

Она полезна как retrieval skeleton, но недостаточна как correctness definition.

### 7.2. Недостающие boundary classes

Для прикладных Kotlin/JVM задач нужно явно моделировать:

```text
Build/module/classpath boundaries
Configuration and feature-flag boundaries
Dependency-injection and framework lifecycle
Transaction boundaries
Coroutine/suspend/dispatcher boundaries
Lazy/eager collection and Flow boundaries
Persistence query/schema/projection boundaries
Serialization and external API compatibility
Error/retry/resource lifecycle
Generated code and compiler plugins
Test-to-production coverage/trace
External behavioral specification
```

### 7.3. Новое определение полноты

```text
COMPLETE_FOR(Family F, Goal G, Snapshot S)
```

выполнено только если:

1. все obligations семейства F материализованы;
2. для каждого obligation есть semantic evidence;
3. все required bindings уникальны либо model choice зафиксирован;
4. unsupported boundaries отсутствуют или разрешены policy;
5. все planned edit points uniquely anchorable;
6. effect/order/cardinality/nullability/lifecycle constraints доказаны;
7. test-oracle class определён;
8. concurrent revalidation ReadSet покрывает все dependencies goal;
9. evidence не усечено по обязательным surfaces;
10. validation plan покрывает change graph.

Фиксированные top-k лимиты остаются performance budgets, но не входят в
доказательство полноты:

```text
closure > budget -> PARTIAL_BUDGET
```

### 7.4. Negative completeness tests, которых не хватает

Минимальный набор:

1. unresolved internal call, влияющий на goal;
2. две одинаково правдоподобные test surfaces;
3. multiple config producers/transformers;
4. omitted fifth required edit surface;
5. external call с unknown effect;
6. reflection/DI-created implementation;
7. transaction boundary между producer и consumer;
8. suspend/lazy Flow, где placement меняет timing;
9. persistence query field без schema evidence;
10. serialization/API branch outside selected graph;
11. source truncation внутри required declaration;
12. execution-path truncation;
13. second Thread IR dependency changes during concurrent edit;
14. decoy declaration with matching lexical terms;
15. test passes even when transformation omitted.

---

## 8. Роль графов и отсутствующий change graph

| Graph | Что позволяет вывести | Текущее состояние | Цена/риск |
| --- | --- | --- | --- |
| Call graph | producer/consumer chain, affected callers | K2 resolved calls для selected roots; summaries | Dynamic dispatch/DI/reflection partial |
| Type/assignability/override | compatible transformer, signature propagation | Types, signatures, inheritance facts частично есть | Conservative overload/generics |
| Local CFG | placement, branches, loops, order | FIR/structured fallback есть | Local only |
| Def-use/SSA | value dependencies | Есть внутри function | Не crossing call boundaries |
| Control dependency | governing predicates | Есть локально | Exceptions/coroutines/framework paths ограничены |
| Effect graph | purity/state/throw/suspend constraints | Coarse effect nodes; unknown calls conservative | Недостаточно для strong purity proof |
| Persistence projection | query → DTO fields → consumers | Отдельного graph нет | Сейчас source parsing JPQL |
| Test-production trace | impacted tests, oracle relationship | Лексический test search | Coverage/trace отсутствуют |
| Build/module graph | compilation and invalidation | Project model есть | Не включён полноценно в task obligation closure |
| Change graph | какие semantic obligations должны измениться совместно | Отсутствует как first-class artifact | Ключевой следующий слой |

### Change graph

Предлагается first-class IR:

```text
ChangeObligation {
    id
    kind
    subject symbols/types/edges
    preconditions
    postconditions
    dependsOn[]
    evidence[]
    dischargeStatus
}
```

Типы obligations:

```text
CREATE_TYPE
CHANGE_DECLARED_TYPE
PROPAGATE_RETURN_TYPE
REWIRE_CALL
INTRODUCE_VALUE_ONCE
MAP_VALUE_EDGE
PRESERVE_ORDER
PRESERVE_CARDINALITY
PRESERVE_EFFECTS
PRESERVE_NULLABILITY
PRESERVE_API
UPDATE_PERSISTENCE_PROJECTION
UPDATE_SERIALIZATION_BRANCH
PROVIDE_BEHAVIORAL_ORACLE
SELECT_TESTS
```

Task surface должен строиться как closure по change obligations, а не как
список top-ranked snippets.

---

## 9. Рекомендуемая архитектура semantic-goal compiler

### 9.1. Слои

```text
Task text
    |
    v
Task intent extractor
    |
    v
Goal template candidate(s)
    |
    v
Obligation binder
    |
    +-- unique -> proof
    +-- multiple -> one bounded model choice
    +-- unsupported -> refusal
    |
    v
Change Graph
    |
    v
Certified strategy compiler
    |
    v
PSI-native EditIR
    |
    v
Preview / build / tests / transaction
```

### 9.2. Model-owned данные

Только:

```text
family/goal selection if not uniquely inferred
new business names
selection among explicitly presented alternatives
business expected values
external policy
behavioral oracle not present in repository
```

### 9.3. Worker-owned данные

```text
symbols and overloads
types and receivers
argument-to-parameter mappings
producer/consumer edges
placement and dominance
imports
anchors/occurrences
collection kind and laziness
effects
nullability
order/cardinality
test routing
PSI mutations
proof/failure report
```

### 9.4. Proof object

```json
{
  "schema": "semantic-goal-proof/0.1",
  "goalId": "goal:...",
  "snapshot": "...",
  "status": "BOUND | AMBIGUOUS | REFUSED",
  "bindings": [],
  "obligations": [
    {
      "id": "o1",
      "kind": "INTRODUCE_ONCE",
      "evidence": [],
      "status": "PROVED"
    }
  ],
  "placementProof": {
    "dominates": [],
    "evaluationCount": "ONCE"
  },
  "typeProof": [],
  "effectProof": [],
  "orderAndCardinalityProof": [],
  "boundaries": [],
  "ambiguities": [],
  "plannedSemanticOperations": [],
  "testOracleClass": "DERIVED | PARAMETRIC | MODEL_AUTHORED | EXTERNAL_SPEC",
  "refusalCode": null
}
```

Модель не должна перечитывать source после successful proof. Failure report
должен объяснять:

- какая obligation не доказана;
- какие candidates найдены;
- какой минимальный choice нужен;
- почему auto-placement запрещён.

---

## 10. Первый новый probe: `MAP_EDGE_WITH_CONTEXT`

`WIRE_TYPED_DECORATOR` следует рассматривать как частный случай:

```text
Introduce value C exactly once in region R
Transform each T on producer -> consumer edge E with F(T, C) -> T
Preserve order, cardinality, laziness, consumer contract and allowed effects
Require a behavioral oracle
```

### 10.1. Goal schema

```json
{
  "schema": "semantic-goal/0.1",
  "family": "MAP_EDGE_WITH_CONTEXT",
  "baseRevision": "<snapshot>",
  "intent": {
    "elementType": "T",
    "contextType": "C",
    "transformResultType": "T",
    "contextEvaluation": "ONCE_PER_REGION",
    "preserve": [
      "ORDER",
      "CARDINALITY",
      "LAZINESS",
      "CONSUMER_CONTRACT",
      "EFFECTS",
      "NULLABILITY"
    ]
  },
  "choices": {
    "newNames": {},
    "selectedBinding": null,
    "behavioralOracle": null
  }
}
```

Большинство symbols должно быть `AUTO`, а не передаваться моделью.

### 10.2. Applicability invariants

1. Единственный compatible context producer `() -> C`.
2. Единственный compatible transformer:
   - `(T, C) -> T`, или
   - `T.decorate(C): T`, или
   - equivalent statically resolved pure call.
3. Единственный producer-to-consumer value edge.
4. Placement dominates все transformed uses.
5. Context вычисляется ровно один раз в разрешённом region.
6. Transform применяется ровно один раз к каждому element.
7. Order/cardinality/laziness не меняются.
8. Consumer принимает `T` после transform.
9. Nullability compatible.
10. Нет unsupported lifecycle/transaction/coroutine boundary.
11. Effects transform входят в allowlist.
12. Test-oracle class известен.

### 10.3. Binding algorithm

1. Построить candidates по type signatures, не по names.
2. Разрешить overload/receiver/extension через K2.
3. Найти value-flow edge от source collection/producer до consumer.
4. Проверить collection modality: eager list, sequence, flow, callback.
5. Вычислить dominators/post-dominators для placement.
6. Проверить evaluation count `C`.
7. Проверить effect summary transformer.
8. Построить candidate Change Graph.
9. Если candidate один — proof.
10. Если candidates 2–N — вернуть bounded ambiguity.
11. Если unsupported boundary — refuse.
12. После choice скомпилировать PSI-native edits.

### 10.4. Must-refuse cases

1. Два compatible config producers или transformers без task preference.
2. Transformer пишет state, throws/suspends или зависит от invocation order, а
   goal требует preserve effects.
3. Source — lazy `Sequence`/`Flow`, а proposed placement делает вычисление
   eager.
4. Между source и consumer находится transaction/lifecycle boundary.
5. Consumer использует object identity/alias до transform.
6. DI/reflection скрывает actual call target.
7. Transform может менять cardinality.
8. Нет behavioral oracle и task не задаёт expected relation.

---

## 11. Test oracle strategy

Исследования test-oracle problem показывают, что desired behavior часто нельзя
вывести только из implementation. Contracts, models и metamorphic relations
помогают, но последним источником может оставаться человек/модель с domain
knowledge.

### 11.1. Полностью выводимый worker oracle

Условия:

- existing test уже задаёт expected outcome;
- change должен сохранять известный algebraic/property relation;
- можно усилить existing assertion;
- mutation check показывает, что тест падает при omission/wrong placement.

Worker может:

- выбрать existing test;
- добавить assertion на уже существующий expected object;
- сгенерировать differential check old/new path;
- использовать transformation law.

### 11.2. Parametric generation

Worker выводит:

- fixture construction;
- input values из constructors/defaults/nullability;
- call sequence;
- assertion shape.

Модель задаёт только:

- expected business values;
- выбранный branch;
- tolerance/policy.

### 11.3. Model-authored oracle

Нужен, если:

- несколько externally valid behaviors;
- expected result не кодирован;
- изменение добавляет business rule;
- issue text содержит behavior, которого нет в repository.

### 11.4. External specification required

Автоматизация должна отказаться, если:

- task text двусмыслен;
- отсутствуют tests/contracts/examples;
- expected behavior зависит от внешней системы/policy;
- generated test будет self-confirming.

### Дополнительные gates

Generated/selected test должен:

1. падать на relevant mutant — например, transform omitted;
2. падать при wrong placement/order;
3. не проверять только concrete subtype, если task обещает public contract;
4. не считаться доказательством, если он compile-only;
5. не использовать implementation value как expected value без независимого
   relation.

Metamorphic/property/differential testing полезны, только если relation уже
известен. Они не создают business semantics из ничего.

---

## 12. Materialization safety

### 12.1. Что уже сделано правильно

- unique semantic anchors;
- PSI parse replacement;
- K2 before/after analysis;
- protected binding checks;
- type/effect/diagnostic checks;
- exact candidate source hashes;
- isolated worktree;
- compile/tests;
- CAS commit.

### 12.2. Что заменить до следующего end-to-end benchmark

Текущие text heuristics:

1. `REWRITE_DECLARATION` exact/regex substitutions.
2. JPQL `SELECT/FROM` substring parser.
3. Kotlin signature parsing через `split_once`.
4. `forEach`/named-argument parsing через strings.
5. test matcher `anyOrNull()` и lambda binding inference.
6. import source reconstruction.
7. extension-call and override normalizers в Rust CLI.

PSI-/semantic-native operations:

```text
CHANGE_DECLARED_TYPE
CHANGE_RETURN_TYPE
ADD_SUPERTYPE
REPLACE_RESOLVED_CALL
REPLACE_ARGUMENT
INSERT_NAMED_ARGUMENT
INTRODUCE_LOCAL_ONCE
MAP_COLLECTION_EDGE
CREATE_TOP_LEVEL_DECLARATION
ADD_RESOLVED_IMPORT
REPLACE_TEST_MATCHER
ADD_ASSERTION
```

Для persistence query нужен отдельный query parser и schema evidence. Kotlin
PSI не может доказать корректность JPQL string literal.

Обязательные preconditions:

```text
resolved SymbolId
selected callable
dispatch/extension receiver
argument-to-parameter mapping
expression/declared type
nullability
effect summary
multiplicity
dominator placement
collection modality
lifecycle/transaction region
```

Postconditions до build:

```text
protected references unchanged
expected call path exists
type propagation complete
effect delta allowed
order/cardinality/laziness preserved
public ABI policy satisfied
all new symbols uniquely imported
no new compiler diagnostics
all Change Graph obligations discharged
```

---

## 13. Neutral withheld corpus

### 13.1. Два слоя оценки

#### A. Codeclew Semantic Change Corpus

Назначение: проверить compiler invariants, applicability, refusal и
generalization.

Минимум:

```text
36 задач
>= 6 structural families
для каждой family:
  positive
  ambiguous
  must-refuse
```

Каждый task генерируется из seed после freeze worker.

#### B. External ecological validation

Назначение: проверить, насколько families соответствуют реальным задачам.

Использовать:

- supported-scope subset официального Kotlin Benchmark;
- stratified sample публичных Kotlin/JVM PR/issues;
- отдельную выборку backend/service/library repositories.

Официальный Kotlin Benchmark содержит 105 задач из восьми repositories, но 43
относятся к ktlint и 28 к detekt — 71/105, то есть около 67,6%, приходится на
два developer-tooling projects. JetBrains прямо планирует расширять ecosystem
coverage и метрики. Поэтому dataset полезен, но не должен единолично определять
«большинство прикладных задач».

### 13.2. Семейства

1. producer–transformer–consumer wiring;
2. type/signature propagation через несколько layers;
3. DTO/event/API contract evolution с branches;
4. persistence projection и nullability;
5. configuration/annotation/lifecycle;
6. error/retry/resource handling;
7. test-only regression strengthening.

### 13.3. Variations

```text
identifier renaming
package/layout changes
single/multi module
Gradle/Maven
overloads and extensions
nullable/non-null
data class/interface/sealed hierarchy
decoy symbols
formatting/comments
eager List vs Sequence vs Flow
direct construction vs DI
transaction/suspend boundaries
different test frameworks
```

### 13.4. Manifest

```json
{
  "taskId": "...",
  "seed": "...",
  "family": "...",
  "difficulty": "...",
  "baseRevision": "...",
  "taskText": "...",
  "expectedObligations": [],
  "expectedSurface": [],
  "acceptableDesignClasses": [],
  "mustRefuse": false,
  "refusalReasons": [],
  "oraclePatch": "<hidden>",
  "hiddenTests": [],
  "forbiddenVocabulary": [],
  "forbiddenShortcuts": []
}
```

### 13.5. Anti-overfitting

- generator и worker не содержат task/repository names;
- withheld seeds создаются после freeze;
- names/package/layout randomized;
- acceptance runner скрыт от agent;
- corpus includes structural counterexamples;
- all failed/refused/retried runs retained;
- public external tasks never copied into generated corpus;
- skill/system instructions common across modes and contain no answer hints.

---

## 14. Paired benchmark protocol

### 14.1. Modes

1. Default filesystem search/read/edit.
2. AST-index for navigation, source reads allowed.
3. Codeclew context + semantic goal + atomic apply.

### 14.2. Controlled variables

```text
same task text
same base revision
same model/version
same reasoning effort
same system instructions
same build/test access
same machine resources
independent clean worktrees
randomized run order
cold and warm cache runs
```

### 14.3. Clock

```text
start:
  task becomes visible to agent

end:
  clean accepted commit passes hidden verification
```

Не использовать sum of tool wall как substitute for end-to-end.

### 14.4. Обязательные metrics

```text
hidden acceptance
compile/test correctness
time to first correct edit
time to accepted commit
worker/model/build/test durations
model/tool calls
navigation/discovery calls
raw input tokens
cached input tokens
output tokens
noncached tokens
model-visible bytes
goal/plan bytes
diagnostic bytes
validator attempts
repair turns
failed applies
files read
files changed
fallback search count
applicability/refusal
cold/warm
```

### 14.5. Exclusions

Preregister only:

- infrastructure failure before task delivery;
- unavailable model/API;
- corrupted fixture;
- verifier crash unrelated to patch.

Incorrect patch, timeout, refusal, extra edit, failed build и agent crash
остаются в результатах.

### 14.6. Statistical decision

Для paired tasks:

- report paired median difference;
- bootstrap 95% confidence interval;
- accepted win rate;
- correctness rate;
- applicability and refusal precision;
- family-weighted estimates;
- all-run and applicable-only views.

Скорость сравнивать только вместе с correctness. Fast rejected run не является
победой.

---

## 15. Ответы на 20 обязательных вопросов

### 1. Где сейчас находится доминирующий model-owned bottleneck?

В преобразовании bounded context в coherent multi-file change: выборе
obligations, формировании low-level plan, создании test oracle и repair turns.
Discovery уже может быть сильно сокращён `agent-context`, но это не гарантирует
компактный patch plan.

Evidence: `CODE`, `ARTIFACT`, `INFERENCE`.

### 2. Какая часть bottleneck устранима deterministic worker?

Symbol/role binding, compatible call selection, placement, dominance, type
flow, imports, occurrences, PSI operations, nullability/effect/order/cardinality
checks, test routing и proof/refusal. Неустранимы business choices и
отсутствующий behavioral oracle.

### 3. Какой минимальный semantic goal достаточен для первого выбранного family?

`MAP_EDGE_WITH_CONTEXT`: типы `T/C`, obligation «вычислить `C` один раз;
применить `F(T,C)->T` на единственном value edge; сохранить
order/cardinality/laziness/effects/consumer contract», плюс только новые
business names/oracle при необходимости.

### 4. Goal language — constraint language или каталог macros?

Небольшой typed constraint language с certified family strategies. Не
unrestricted synthesis и не flat macro catalog.

### 5. Как не превратить macros в repository recipes?

Запрет repository/task vocabulary; applicability только по semantic predicates;
randomized renaming/layout/decoys; withheld seeds после freeze; proof object;
must-refuse cases; external ecological validation.

### 6. Какие graph facts отсутствуют?

Interprocedural value flow, precise call/effect/purity summaries, cross-call
dominance/placement, lifecycle/transaction/coroutine regions, persistence
projection/schema graph, test-production trace/coverage, explicit change
obligations.

### 7. Можно ли формально определить `COMPLETE_TASK`?

Глобальный текущий статус — нет. Можно определить только
`COMPLETE_FOR(family, goal, snapshot)` через discharge всех family obligations.
Текущий `COMPLETE_TASK` не включает Thread completeness и использует
эвристические surfaces.

### 8. Какие ambiguities требуют модели?

Несколько compatible producers/transformers/placements, несколько behaviorally
valid designs, новые public names, business expected values, policy trade-offs.
Система должна либо доказать unique binding, либо сформировать один bounded
clarification turn, либо refuse.

### 9. Какие test oracle можно вывести?

Existing-oracle strengthening и известные transformation laws — полностью;
fixture/scaffold — parametrically; business expected behavior — model-authored;
отсутствие specification — external-spec required/refusal.

### 10. Какие text heuristics заменить PSI-native edits?

`REWRITE_DECLARATION`, Kotlin signature/loop/named-arg parsing,
import/extension/override normalizers, test matcher rewrite. JPQL требует
отдельного parser/schema layer.

### 11. Как построить withheld corpus?

Generated neutral Gradle/Maven templates, immutable hidden manifest, randomized
names/layout/modules/decoys, positive/ambiguous/refuse variants, seeds generated
after freeze, independent verifier, no old repository vocabulary.

### 12. Как определить популяцию «большинства прикладных задач»?

Заранее сформировать stratified random sample публичных Kotlin/JVM issues/merged
PRs в supported contour, вручную/двойной разметкой классифицировать families и
взвесить generated corpus по независимому distribution. Handpicked
transform-applicable set недостаточен.

### 13. Какой ожидается applicability rate?

Сейчас надёжно неизвестен. Доказан один production structural family. Порог
60% можно preregister как критерий универсальной линии, но нельзя выдавать его
за текущую оценку.

### 14. Почему экономия должна сохраняться при росте repository size?

Только если repository-wide index инкрементален, model-visible context зависит
от obligation closure, а goal — от semantic delta/business choices. Если model
получает source или пишет textual patch, преимущество не масштабируется.

### 15. Каков upper bound model-visible context/output?

Текущий context имеет byte budgets, но они не являются semantic bound.
Предлагаемый experimental bound: 16–32 KiB context и `<=1 KiB` typical goal;
превышение обязательной closure → partial/refusal. Это hypothesis, подлежащая
corpus measurement.

### 16. Когда default `rg` гарантированно быстрее?

Exact known literal/symbol, один-два файла, local syntactic edit, нет
cross-layer obligations, no ambiguous tests, cold Codeclew state, либо
build/test доминируют total time.

### 17. Когда AST-index достаточен?

Navigation/localization, deterministic rename/refactoring, local
type-independent edits и задачи без data/control/effect/lifecycle/test closure.
Тогда semantic transaction premium может не окупиться.

### 18. Какие gates нельзя обменивать на скорость?

Hidden correctness, unique target, fail-closed ambiguity, no new diagnostics,
protected bindings/types/effects/ABI policy, must-refuse accuracy, complete
telemetry, isolated validation и atomic commit.

### 19. Какой эксперимент имеет максимальную information gain?

Blind, withheld goal-binding-only experiment до materialization: минимум 30
задач/5 families; model выдаёт только typed goal; binder возвращает
proof/ambiguity/refusal; результат сравнивается с hidden obligation manifest.

### 20. Итоговый verdict?

`GO_BUILD_CORPUS_FIRST`, confidence `0.88`.

---

## 16. Следующий эксперимент с максимальной information gain

### Goal-binding-only trial

#### Почему не end-to-end сразу

Materialization/build/model variance скроют главный вопрос:

> Может ли общий goal language правильно и компактно связать task intent с
> semantic change obligations без repository-specific recipe?

#### Протокол

1. Freeze task-context implementation, goal schema and binder.
2. Generate `>=30` withheld tasks across `>=5` families.
3. Agent получает task text + bounded context.
4. Agent возвращает только semantic goal; source edits запрещены.
5. Binder возвращает:
   - `BOUND + proof`;
   - `AMBIGUOUS + choices`;
   - `REFUSED + reason`.
6. Hidden runner сравнивает:
   - obligations;
   - bindings;
   - expected surface;
   - refusal decision.

#### Метрики

```text
false COMPLETE = 0
must-refuse accuracy = 100%
binding precision
binding recall
applicability
goal bytes
model turns
unresolved business ambiguities
family-specific failure modes
```

#### Gate для materialization stage

Переходить к PSI edit compiler только если:

- false-complete = 0;
- must-refuse = 100%;
- applicability `>=60%` target sample;
- correct binding `>=90%` applicable tasks либо обоснованный более строгий
  threshold;
- median goal `<=1 KiB`;
- median clarification turns `<=1`;
- no repository vocabulary.

---

## 17. Следующие три implementation commits

### Commit 1

```text
bench: add neutral semantic-change corpus generator and hidden manifest verifier
```

Содержит:

- families;
- seed generator;
- positive/ambiguous/refuse variants;
- immutable manifests;
- independent acceptance runner;
- telemetry schema.

### Commit 2

```text
feat(experimental): add typed goal schema, obligation closure and proof/refusal binder
```

Важно:

- без source mutation;
- `COMPLETE_FOR`;
- Change Graph;
- proof object;
- bounded ambiguity;
- no second production transform.

### Commit 3

```text
bench: add paired goal-binding and default/ast/codeclew telemetry harness
```

Содержит:

- randomized paired runs;
- cold/warm;
- token telemetry;
- all-run retention;
- bootstrap CI;
- applicability/refusal report.

---

## 18. Decision thresholds и falsifiers

### Изменить verdict на `GO_IMPLEMENT`, если

На preregistered corpus:

- `>=30` withheld tasks;
- `>=5` families;
- correctness Codeclew не ниже лучшего baseline;
- applicability `>=60%` target sample;
- accepted win rate `>=70%` applicable tasks против каждого baseline;
- median end-to-end reduction `>=30%`;
- median noncached token reduction `>=30%`;
- median raw token reduction `>=40%`;
- paired 95% CI остаётся в зоне выигрыша;
- false complete = 0;
- must-refuse = 100%;
- нет task/repository vocabulary;
- materialization PSI-native для выбранного family.

### Изменить verdict на `STOP_NOT_PLAUSIBLE`, если

После goal binder + neutral corpus:

- applicability `<40%` target population;
- false completeness остаётся ненулевой;
- correctness ниже baseline;
- большая часть applicable tasks всё равно требует low-level source plan;
- test oracle в большинстве задач остаётся полностью model-authored;
- end-to-end/token benefit исчезает после учёта cold context/build;
- generalization требует family-specific source recipes;
- must-refuse cases регулярно проходят до candidate commit.

Даже при `STOP_NOT_PLAUSIBLE` отдельные narrow transforms могут остаться
полезными features. Это будет отказ от универсальной продуктовой гипотезы, а не
от всей платформы.

---

## 19. Risk register

| Риск | Вероятность | Ущерб | Мера |
| --- | ---: | ---: | --- |
| Overfitting на product-repo task | Высокая | Критический | Withheld seeds, multi-family corpus |
| False `COMPLETE_TASK` | Высокая | Критический | `COMPLETE_FOR`, obligation proof |
| Text rewrite ловит wrong fragment | Средняя | Высокий | PSI-native operations |
| Framework lifecycle невидим | Высокая | Высокий | Explicit boundaries + refusal |
| Coroutine/laziness semantics | Средняя | Высокий | Modality/effect regions |
| Test self-confirms patch | Высокая | Высокий | Mutation gate |
| Token claims по bytes | Средняя | Высокий | Native token telemetry |
| Build dominates total time | Высокая | Средний | Separate profile, impacted tests |
| Benchmark population skew | Высокая | Высокий | Independent stratified sampling |
| Goal language слишком общий | Средняя | Высокий | Certified strategies |
| Goal catalog превращается в recipes | Высокая | Критический | Structural predicates and anti-vocabulary tests |
| Multi-root concurrent staleness | Средняя | Высокий | Goal-wide ReadSet, not first thread only |
| K2/FIR version instability | Высокая | Средний | Existing version-isolated workers |

---

## 20. Финальная рекомендация

Codeclew следует развивать не как «AST-index с редактированием» и не как набор
быстрых macros.

Целевой продуктовый объект:

```text
Task-specific semantic change transaction
```

Он должен содержать:

```text
goal
obligation closure
proof/refusal
change graph
PSI-native edits
validation evidence
snapshot/read-write set
atomic commit
```

Ключевой критерий успеха:

> Модель сообщает только намерение и irreducible business decisions; worker
> доказывает bindings, строит change graph, выбирает placements, материализует
> PSI edits и отказывается при неполноте.

Текущий репозиторий уже содержит значительную часть нижней половины этой
архитектуры — compiler integration, graph facts, preview и transactionality. Не
доказана верхняя половина: универсальная task obligation closure, goal binding,
test oracle ownership и applicability across real task population.

Поэтому решение:

`GO_BUILD_CORPUS_FIRST`

Не реализовывать новый production transform до завершения goal-binding-only
withheld experiment.

---

## Приложение A. Основные code references

Все ссылки относятся к revision
`7fc3e0d6c6e784a130245ef0e344535a146324c7`. Диапазоны помечены `≈`, когда
функция занимает длинный блок и точной единицей навигации является её имя.

| Область | Файл / диапазон / функция |
| --- | --- |
| CLI project/index/context pipeline | `crates/sthread/src/main.rs`, ≈250–405: `Command::Project`, `Command::Index`, `Command::AgentContext` |
| Goal/plan to transaction | `crates/sthread/src/main.rs`, ≈405–535: `Command::TaskApply` |
| Context limits и root selection | `crates/sthread/src/task_context.rs`, ≈1–240: constants, `TaskContextSelection::root_symbols`, `followup_symbols` |
| Task-surface assembly | `crates/sthread/src/task_context.rs`, ≈620–940: `build` |
| `COMPLETE_TASK` и bounded/full evidence | `crates/sthread/src/task_context.rs`, ≈880–980 |
| Surface/contract compaction | `crates/sthread/src/task_context.rs`, ≈950–1120 |
| Existing semantic-goal compiler | `crates/sthread/src/task_plan.rs`, ≈1–240: `expand_transient_transform` |
| Resolved-path check | `crates/sthread/src/task_plan.rs`, ≈260–325: `verify_resolved_path` |
| Source-shape parsers | `crates/sthread/src/task_plan.rs`, ≈325–580: JPQL/signature/loop/test helpers |
| IR | `crates/sthread/src/model.rs`: `ThreadIr`, `ReadFact`, `EditIr`, `EditOperation`, `Transaction` |
| Graph enrichment | `crates/sthread/src/graph.rs`, ≈1–340: `enrich_profiled`, type/call/effect edges |
| Slicing and Thread ReadSet | `crates/sthread/src/graph.rs`, ≈660–880: `slice` |
| Preview and write-set validation | `crates/sthread/src/transaction.rs`, ≈1–520 |
| Semantic replay/worktree/CAS | `crates/sthread/src/transaction.rs`, ≈500–1150 |
| Ledger recovery/index rollback | `crates/sthread/src/transaction.rs`, ≈1150–end |
| Persistent index | `crates/sthread/src/index.rs`: `RepositoryIndex`, `stage_update`, `update_from_root` |
| Worker protocol | `schemas/worker.proto`; `crates/sthread/src/worker.rs` |
| Kotlin project/K2 environment | `workers/kotlin/src/main/kotlin/dev/semanticthread/worker/Worker.kt`, ≈1–500 |
| Symbol resolution/anchors/local graph | `Worker.kt`, ≈500–1100 |
| PSI expression/body apply | `Worker.kt`, ≈1100–1320: `applyEdit` |
| Declaration text rewrite/import edit | `Worker.kt`, ≈1300–1570: `applyDeclarationRewrite`, `applyImportEdit` |
| Verification script | `scripts/verify.sh` |
| Performance artifacts | `benchmarks/reports/latest.json`, `corpus-100k.json` |
| Closed benchmark artifacts | `benchmarks/reports/maven-product-repo.json`, `agent-context-pim-migrator.json` |
| Previous negative conclusion | `docs/experiments/universal-task-surface-model-2026-08-02.md` |

## Приложение B. Внешние источники

1. JetBrains. *Introducing the Kotlin Benchmark for AI Coding Agents*, 2026.
2. Kotlin Benchmark. *Methodology and Outlook*, 2026.
3. Jimenez et al. *SWE-bench: Can Language Models Resolve Real-World GitHub
   Issues?* arXiv:2310.06770.
4. Xia et al. *Agentless: Demystifying LLM-based Software Engineering Agents.*
   arXiv:2407.01489.
5. Horwitz, Reps, Binkley. *Interprocedural Slicing Using Dependence Graphs.*
   ACM TOPLAS 12(1), 1990, DOI 10.1145/77606.77608.
6. Barr et al. *The Oracle Problem in Software Testing: A Survey.* IEEE TSE
   41(5), DOI 10.1109/TSE.2014.2372785.
7. Chen et al. *Metamorphic Testing for Cybersecurity.* IEEE Computer, DOI
   10.1109/MC.2016.176.
8. Kotlin Analysis API documentation: fundamentals, references/calls,
   in-memory file analysis.
