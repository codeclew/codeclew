Техническое задание для ИИ-агента

Semantic Thread Platform: Kotlin MVP

> Дополнение от 2026-08-01: после выполнения исходной Gradle-вертикали поддерживаемая область расширена полной single-module Maven Kotlin/JVM вертикалью. Maven project inspection, version-pinned worker selection, semantic index/context и detached-worktree compile/test/commit подчиняются тем же требованиям к fingerprint, fail-closed поведению и транзакционной безопасности. Подробный дизайн: `docs/superpowers/specs/2026-08-01-maven-end-to-end-design.md`.

1. Роль агента

Ты выступаешь как senior software architect и senior systems engineer с опытом в:

• Rust;
• Kotlin/JVM;
• Kotlin K2 Analysis API;
• Kotlin PSI и FIR;
• компиляторах и статическом анализе;
• program slicing, CFG, SSA, def-use и control dependencies;
• инкрементальных индексах;
• Git internals;
• оптимистичных транзакциях и MVCC;
• проектировании CLI, IPC и versioned protocols.

Твоя задача — создать первый работающий прототип многокязычной платформы семантического анализа и безопасного изменения исходного кода. Первым поддерживаемым языком является Kotlin.

Не ограничивайся созданием архитектурного каркаса. Результатом должна стать исполнимая вертикальная версия, которую можно запустить на тестовом Kotlin/JVM Gradle-проекте.

────────

2. Цель проекта

Создать систему, которая позволяет ИИ-агенту работать не со строками и файлами целиком, а с ограниченными вычислительными нитями — семантическими срезами программы.

Система должна уметь:

1. загрузить структуру Kotlin/JVM Gradle-проекта;
2. построить индекс файлов, declarations, symbols, references и calls;
3. выбрать seed — функцию или выражение;
4. построить локальную вычислительную нить вокруг seed;
5. представить её в собственном versioned Thread IR;
6. принять от агента структурную операцию изменения;
7. найти исходный PSI-узел по устойчивому semantic anchor;
8. применить изменение к изолированной копии исходника;
9. проверить синтаксис, типы, symbol bindings и diagnostics;
10. построить preview diff;
11. применить изменение в отдельном Git worktree;
12. запустить Gradle compilation и тесты;
13. атомарно зафиксировать изменение либо оставить репозиторий неизменным;
14. обнаруживать устаревшие данные и конфликты при параллельной работе агентов.

Основная пользовательская история:

```text
Kotlin repository
    → select function or expression
    → build semantic thread
    → produce structured edit
    → validate edit
    → preview diff
    → commit atomically or reject
```

────────

3. Зафиксированное архитектурное решение

Система должна быть полиглотной.

3.1. Универсальное ядро

Реализовать на Rust:

• canonical IR;
• compact graph representation;
• repository index;
• CFG normalization;
• dominators и post-dominators;
• SSA;
• def-use;
• control dependencies;
• program slicing;
• semantic anchors;
• ReadSet и WriteSet;
• MVCC и conflict detection;
• Git worktree orchestration;
• transaction ledger;
• CLI и основной daemon;
• worker lifecycle и IPC.

3.2. Kotlin-адаптер

Реализовать как отдельный долгоживущий процесс на Kotlin/JVM:

• загрузка Kotlin compilation context;
• Kotlin PSI;
• K2 Analysis API;
• разрешение symbols, types, calls и receivers;
• Kotlin-specific diagnostics;
• извлечение FIR CFG через изолированный version-pinned adapter;
• поиск PSI targets;
• создание replacement PSI через KtPsiFactory;
• применение edit к PSI-копии;
• ограниченное форматирование;
• выдача изменённых source bytes.

3.3. Протокол

Использовать Protocol Buffers.

Для MVP IPC реализовать через:

```text
length-prefixed Protobuf messages over stdin/stdout
```

Не использовать JNI между Rust и Kotlin.

3.4. Источник истины

Единственный источник истины:

```text
Git revision + exact source bytes + project model fingerprint
```

Thread IR, индекс, PSI, FIR и внутренние compiler objects являются производными представлениями.

────────

4. Базовая конфигурация MVP

Зафиксировать следующую конфигурацию:

```text
Language: Kotlin/JVM
Kotlin compiler baseline: 2.4.10
JDK: 21
Build system: Gradle Wrapper
Source files: .kt
Repository: Git
Core: Rust stable, pinned in rust-toolchain.toml
Storage: SQLite in WAL mode + content-addressed binary blobs
IPC: Protocol Buffers over framed stdin/stdout
```

Kotlin compiler version должна быть конфигурируемой, но первый worker реализуется и тестируется для одной точно зафиксированной версии.

────────

5. Термины

5.1. Вычислительная нить

Вычислительная нить — ограниченный статический срез программы:

```text
Thread(R, Seed, Policy) = boundedClosure(Seed, SemanticGraph(R), Policy)
```

Она включает:

• definitions и uses;
• управляющие predicates;
• calls и returns;
• arguments и parameters;
• receivers;
• local variables;
• property reads и writes;
• captures;
• exceptions;
• summaries внешних вызовов;
• границы неполноты анализа.

5.2. Semantic graph

Граф содержит как минимум следующие edge kinds:

```text
AST_CHILD
CFG_NORMAL
CFG_TRUE
CFG_FALSE
CFG_EXCEPTION
CFG_BACK
DEF_USE
PHI_INPUT
CONTROL_DEP
CALL
RETURN
ARG_PARAM
RECEIVER
CAPTURE
TYPE
READ_STATE
WRITE_STATE
THROW
SUSPEND
```

5.3. Thread IR

Thread IR — immutable, versioned, language-neutral представление выбранной нити.

Thread IR не должен использоваться для обратной печати Kotlin-кода.

5.4. Edit IR

Edit IR — набор структурных операций над source-backed nodes.

Агент не должен передавать целиком переписанные файлы, если операция может быть описана более узко.

5.5. Semantic anchor

Semantic anchor — составной идентификатор source node, устойчивый к сдвигам строк и локальным изменениям соседнего кода.

────────

6. Scope первой вертикальной версии

6.1. Обязательно реализовать

• Kotlin/JVM Gradle project inspection;
• один выбранный compilation unit;
• индекс Kotlin-файлов и declarations;
• поиск функции по FQN;
• поиск expression по file + offset с дальнейшим преобразованием в semantic anchor;
• local CFG для функции;
• SSA локальных переменных;
• def-use;
• control dependencies;
• backward slice;
• forward slice;
• bounded bidirectional slice;
• Thread IR;
• ReadSet;
• ReplaceExpression;
• ReplaceFunctionBody;
• preview diff;
• syntax validation;
• K2 diagnostics validation;
• type validation;
• protected binding validation;
• detached Git worktree;
• Gradle compile validation;
• запуск явно заданного набора тестов;
• atomic commit через compare-and-swap целевого Git ref;
• transaction ledger;
• stale/conflict detection;
• canonical JSON для отладки;
• Protobuf для IPC.

6.2. Поддерживаемые Kotlin-конструкции

```text
regular functions
member functions
top-level functions
function parameters
local val/var
assignments
if
when
while
do-while
for
break
continue
return
throw
try/catch/finally
ordinary calls
extension calls
named arguments
default arguments
safe calls
Elvis
short-circuit && and ||
property reads/writes on this
lambda captures — консервативно
suspend calls — как effect/boundary
```

6.3. Non-goals MVP

Не реализовывать в первой версии:

```text
Android project model
Kotlin Multiplatform
.kts scripts
expect/actual
Compose-specific semantics
полную поддержку произвольных compiler plugins
reflection resolution
полный points-to analysis
точное моделирование coroutine state machine
RenameSymbol
ChangeSignature
MoveDeclaration
ExtractFunction
InlineFunction
глобальный interprocedural PDG
MCP server
IDE plugin
Java source analysis
Python/TypeScript/C++ adapters
```

Unsupported cases должны возвращаться явно, а не молча игнорироваться.

────────

7. Архитектура системы

```text
┌──────────────────────────────────────────────┐
│               semanticd — Rust               │
│                                              │
│ project snapshots     repository index       │
│ canonical IR          compact graph          │
│ SSA / def-use         thread slicer          │
│ semantic anchors      ReadSet / WriteSet      │
│ MVCC                  transaction ledger     │
│ Git worktrees         worker supervisor      │
└──────────────────────┬───────────────────────┘
                       │ Protobuf IPC
                       ▼
┌──────────────────────────────────────────────┐
│       kotlin-worker-2.4.10 — Kotlin/JVM       │
│                                              │
│ Gradle/Kotlin model    Kotlin PSI             │
│ K2 Analysis API       FIR CFG adapter         │
│ target resolution     PSI edit-on-copy        │
│ diagnostics           bounded formatting     │
└──────────────────────────────────────────────┘
```

7.1. Жёсткие границы

Rust core не должен импортировать:

```text
PsiElement
KtExpression
KtFile
KaSession
KaSymbol
KaType
FirElement
FirControlFlowGraph
```

Kotlin worker не должен:

• напрямую изменять целевую Git-ветку;
• хранить authoritative repository state;
• выполнять собственную логику Git commit;
• возвращать compiler-native objects;
• принимать решение о финальном semantic conflict.

────────

8. Предлагаемая структура репозитория

```text
semantic-thread/
  Cargo.toml
  rust-toolchain.toml
  README.md

  schemas/
    worker.proto
    semantic_facts.proto
    local_cfg.proto
    thread_ir.proto
    edit_ir.proto
    transaction.proto

  crates/
    core-model/
    graph/
    index/
    slicer/
    anchors/
    transaction/
    git-store/
    worker-runtime/
    storage/
    cli/
    semanticd/

  workers/
    kotlin/
      settings.gradle.kts
      build.gradle.kts
      worker-protocol/
      kotlin-project-model/
      kotlin-analysis/
      kotlin-fir-adapter/
      kotlin-edit/
      kotlin-worker-main/

  fixtures/
    kotlin-basic/
    kotlin-control-flow/
    kotlin-calls/
    kotlin-concurrency/

  tests/
    protocol-conformance/
    graph-golden/
    slicing-golden/
    edit-golden/
    transaction-golden/

  benchmarks/
    corpus/
    runner/
    reports/

  docs/
    architecture.md
    correctness-model.md
    protocol.md
    progress.md
    decisions/
```

Допускается небольшое изменение структуры, но архитектурные границы должны сохраняться.

────────

9. Worker protocol

9.1. Общий envelope

Создать versioned messages:

```protobuf
message WorkerRequest {
  uint64 request_id = 1;
  ProtocolVersion protocol_version = 2;
  SnapshotId snapshot = 3;

  oneof payload {
    OpenProjectRequest open_project = 10;
    IndexFilesRequest index_files = 11;
    ResolveSymbolRequest resolve_symbol = 12;
    ResolveExpressionRequest resolve_expression = 13;
    BuildLocalGraphRequest build_local_graph = 14;
    ApplyEditRequest apply_edit = 15;
    ValidateCandidateRequest validate_candidate = 16;
    ShutdownRequest shutdown = 17;
  }
}
```

```protobuf
message WorkerResponse {
  uint64 request_id = 1;
  ProtocolVersion protocol_version = 2;

  oneof payload {
    WorkerCapabilities capabilities = 10;
    OpenProjectResponse open_project = 11;
    IndexFilesResponse index_files = 12;
    ResolveSymbolResponse resolve_symbol = 13;
    ResolveExpressionResponse resolve_expression = 14;
    BuildLocalGraphResponse build_local_graph = 15;
    ApplyEditResponse apply_edit = 16;
    ValidateCandidateResponse validate_candidate = 17;
    WorkerError error = 18;
  }
}
```

9.2. Capability negotiation

Worker при старте обязан сообщать:

```text
language
worker version
compiler version
protocol versions
supported operations
supported language features
unsupported features
```

Пример:

```text
kotlin.resolve.symbols
kotlin.resolve.calls
kotlin.resolve.types
kotlin.cfg.local
kotlin.edit.replace_expression
kotlin.edit.replace_function_body
kotlin.validate.copied_file
```

9.3. Правила протокола

• все messages должны иметь schema version;
• новые optional fields должны быть backward compatible;
• unknown enum value не должен приводить к silent fallback;
• большие source bytes передавать по content hash и blob reference;
• не выполнять RPC на каждый AST node;
• поддерживать batch requests;
• worker должен быть долгоживущим;
• worker crash не должен повреждать core state.

────────

10. Project Model

Rust core запускает Kotlin worker и передаёт путь к репозиторию и выбранный Gradle compilation.

Kotlin worker должен извлечь и нормализовать:

```text
project path
module identity
source set
source roots
generated source roots
compile classpath
friend paths
language version
API version
JVM target
free compiler arguments
opt-ins
compiler plugins
JDK home
compile task
test tasks
```

Создать:

```text
ProjectModelHash = hash(canonical normalized model)
```

Project Model считается изменившимся при изменении:

```text
settings.gradle(.kts)
build.gradle(.kts)
gradle.properties
version catalogs
Gradle Wrapper
buildSrc
convention plugins
dependencies
generated source configuration
compiler options
compiler plugins
```

При изменении Project Model все semantic transactions старого snapshot должны становиться stale.

────────

11. Repository Index

11.1. File facts

Для каждого файла хранить:

```text
FileId
module
source set
normalized relative path
content hash
package
imports
declaration IDs
line-ending mode
BOM presence
```

11.2. Declaration facts

```text
DeclarationId
SymbolId
kind
containing declaration
source origin
signature text hash
body text hash
ABI hash
semantic summary hash
```

11.3. Semantic facts

```text
symbols
types
references
resolved calls
receivers
argument-to-parameter mappings
inheritance
overrides
local CFG cache
function summaries
diagnostics
```

11.4. Инвалидация

Различать:

```text
BodyHash
SourceSignatureHash
AbiHash
SemanticSummaryHash
```

Правила:

|Изменение              |Инвалидация                          |
|-----------------------|-------------------------------------|
|body only              |local CFG, SSA, references, summary  |
|summary unchanged      |callers не инвалидировать            |
|signature changed      |callsites, overrides, implementations|
|ABI changed            |downstream modules                   |
|imports/package changed|semantic facts файла                 |
|classpath changed      |весь compilation semantic cache      |
|compiler plugin changed|весь compilation semantic cache      |
|project model changed  |все transactions snapshot stale      |

────────

12. SymbolId и NodeAnchor

12.1. SymbolId

Создать стабильную language-neutral модель:

```text
module
sourceSet
package
containingDeclarations
declarationName
declarationKind
typeParameterArity
receiverTypes
contextReceiverTypes
parameterTypes
returnType
suspendFlag
```

Для JVM дополнительно хранить descriptor, но не использовать его как единственный source-level ID.

12.2. NodeAnchor

```text
ownerSymbolId
syntaxKind
normalizedTokenHash
ancestorPathHash
localOrdinal
leftContextHash
rightContextHash
exactTextHash
rangeHint
```

Порядок повторного разрешения:

1. найти owner по SymbolId;
2. ограничить поиск owner subtree;
3. отфильтровать по syntax kind;
4. проверить normalized token hash;
5. проверить ancestor path;
6. проверить соседний контекст;
7. проверить preconditions.

Допустимый результат:

```text
0 matches  → STALE_TARGET
1 match    → target resolved
>1 matches → AMBIGUOUS_TARGET
```

Автоматический выбор «самого похожего» target запрещён.

────────

13. Local CFG

Kotlin worker должен экспортировать version-neutral Local CFG DTO.

Минимальная модель:

```text
entry node
exit node
source-backed nodes
synthetic nodes
normal edges
true/false branch edges
exception edges
back edges
subgraph references
source origins
```

Поддержать:

```text
if
when
loops
break/continue
return
throw
try/catch/finally
safe call
Elvis
short-circuit operators
```

FIR является внутренней реализационной деталью Kotlin worker.

При невозможности корректно экспортировать construct вернуть:

```text
UNSUPPORTED_CONTROL_FLOW
```

или boundary node, но не строить заведомо неполный граф без маркировки.

────────

14. SSA, def-use и control dependencies

Реализовать в Rust.

14.1. SSA

Для local variables и parameters:

1. построить dominator tree;
2. вычислить dominance frontier;
3. разместить PHI nodes;
4. выполнить SSA renaming;
5. построить DEF_USE edges.

14.2. Control dependencies

1. построить post-dominator tree;
2. вычислить post-dominance frontier;
3. создать CONTROL_DEP edges.

14.3. Memory abstraction MVP

```text
LOCAL(variable)
THIS_PROPERTY(propertySymbol)
OBJECT_PROPERTY(allocationSite, propertySymbol)
STATIC_PROPERTY(propertySymbol)
UNKNOWN_HEAP
```

При неразрешимом aliasing объединять зависимость с UNKNOWN_HEAP.

Допустима консервативная избыточность. Недопустим пропуск потенциальной зависимости без boundary.

────────

15. Thread slicing

15.1. Slice policy

```json
{
  "direction": "BOTH",
  "includeEdges": [
    "DEF_USE",
    "CONTROL_DEP",
    "CALL",
    "RETURN",
    "ARG_PARAM",
    "RECEIVER",
    "CAPTURE",
    "READ_STATE",
    "WRITE_STATE"
  ],
  "maxNodes": 200,
  "maxFiles": 20,
  "maxCallDepth": 0,
  "maxDispatchTargets": 8,
  "deadlineMs": 2000
}
```

В первой вертикали maxCallDepth = 0: calls представляются opaque summaries.

15.2. Результат

Thread IR должен содержать:

```text
snapshot metadata
seed
policy
completeness status
nodes
edges
editable units
external summaries
boundaries
ReadSet
validation plan
```

15.3. Completeness

Поддержать статусы:

```text
COMPLETE_SUPPORTED_SUBSET
PARTIAL_BUDGET
PARTIAL_UNSUPPORTED_FEATURE
PARTIAL_EXTERNAL_BOUNDARY
PARTIAL_DYNAMIC_DISPATCH
FAILED
```

Нельзя возвращать COMPLETE_SUPPORTED_SUBSET, если:

• превышен budget;
• встречена неподдерживаемая конструкция;
• потерян source origin;
• unresolved call влияет на seed;
• CFG incomplete;
• analysis diagnostics делают semantics недостоверной.

────────

16. Thread IR

Минимальный JSON-эквивалент:

```json
{
  "schema": "semantic-thread/0.1",
  "threadId": "thread:...",
  "snapshot": {
    "baseRevision": "git:...",
    "projectModelHash": "sha256:...",
    "compilerVersion": "2.4.10"
  },
  "seed": {
    "kind": "EXPRESSION",
    "anchor": "anchor:..."
  },
  "policy": {},
  "completeness": {
    "status": "COMPLETE_SUPPORTED_SUBSET",
    "boundaries": []
  },
  "nodes": [],
  "edges": [],
  "editableUnits": [],
  "externalSummaries": [],
  "readSet": [],
  "validationPlan": []
}
```

16.1. Source-backed node

```json
{
  "id": "node:...",
  "kind": "CALL",
  "origin": {
    "fileId": "...",
    "ownerSymbol": "symbol:...",
    "syntaxKind": "KtCallExpression",
    "rangeHint": [842, 875],
    "exactTextHash": "sha256:...",
    "normalizedTokenHash": "sha256:...",
    "ancestorPathHash": "sha256:...",
    "leftContextHash": "sha256:...",
    "rightContextHash": "sha256:..."
  },
  "symbol": "symbol:...",
  "type": "type:...",
  "sourceText": "discountService.apply(price)",
  "editable": true,
  "attributes": {
    "effect": "READ_STATE"
  }
}
```

16.2. Synthetic node

```text
PHI
CALL_SUMMARY
UNKNOWN_EFFECT
DYNAMIC_DISPATCH_BOUNDARY
REFLECTION_BOUNDARY
EXTERNAL_STATE
EXCEPTION_EXIT
```

Synthetic nodes всегда:

```json
{
  "editable": false
}
```

────────

17. Edit IR

Агент должен передавать structured operations.

17.1. MVP operations

```text
REPLACE_EXPRESSION
REPLACE_FUNCTION_BODY
```

Дополнительно допускается реализовать:

```text
ADD_IMPORT
REMOVE_IMPORT
```

17.2. Пример

```json
{
  "schema": "semantic-edit/0.1",
  "threadId": "thread:...",
  "baseRevision": "git:...",
  "operations": [
    {
      "opId": "op:1",
      "kind": "REPLACE_EXPRESSION",
      "target": "anchor:...",
      "preconditions": {
        "ownerSignatureHash": "sha256:...",
        "nodeTextHash": "sha256:...",
        "scopeBindingsHash": "sha256:...",
        "expectedType": "com.acme.Money"
      },
      "replacement": {
        "kotlin": "price.multiply(discount.factor)"
      },
      "postconditions": {
        "typeAssignableTo": "com.acme.Money",
        "mustNotIntroduceEffects": [
          "WRITE_STATE",
          "IO",
          "SUSPEND"
        ]
      }
    }
  ]
}
```

────────

18. Применение изменений

18.1. Preview pipeline

```text
1. Check base snapshot
2. Resolve semantic anchor
3. Verify preconditions
4. Create detached PSI copy
5. Parse replacement in the correct Kotlin context
6. Apply operation to copy
7. Run K2 analysis on candidate
8. Compare diagnostics
9. Compare protected bindings
10. Validate replacement type
11. Calculate effect delta
12. Produce exact diff
13. Build ActualWriteSet
14. Return preview report
```

18.2. Commit pipeline

```text
1. Recheck current target ref
2. Create detached Git worktree
3. Apply validated candidate bytes
4. Preserve line endings and BOM
5. Format only bounded touched range
6. Run affected Gradle compile task
7. Run configured tests
8. Recheck current target ref
9. If changed, perform semantic rebase
10. Repeat validation after rebase
11. Create candidate commit
12. Atomically update target ref using compare-and-swap
13. Append ledger record
14. Incrementally update repository index
```

При любой ошибке исходная ветка должна остаться неизменной.

────────

19. Protected semantic bindings

До изменения сохранить:

```text
reference anchor → SymbolId
call anchor → selected callable SymbolId
expression anchor → inferred type
receiver anchor → receiver symbol/type
callee summary hash
project model hash
```

После изменения:

• references вне разрешённого WriteSet должны разрешаться в те же symbols;
• выбранный overload не должен меняться незаметно;
• новые diagnostics запрещены, кроме явного allowlist;
• type resolution не должен ухудшаться;
• изменение effect summary должно быть явно отражено;
• change в прочитанном callee summary делает transaction stale.

────────

20. ReadSet и WriteSet

20.1. ReadSet

ReadSet включает не только файлы:

```text
source node hashes
owner signature hashes
resolved symbols
resolved call targets
expression types
callee summary hashes
inheritance facts
compiler options
classpath hash
project model hash
diagnostics relied upon
```

Всё, что попало в Thread IR и могло повлиять на решение агента, считается прочитанным.

20.2. WriteSet

```text
target anchors
changed bodies
changed signatures
changed imports
changed summaries
changed ABI
changed effects
```

Сравнивать:

```text
ExpectedWriteSet
ActualWriteSet
```

ActualWriteSet должен быть подмножеством разрешённого scope.

────────

21. Параллельная работа и MVCC

Каждая transaction стартует на immutable snapshot:

```json
{
  "txId": "tx:...",
  "actorId": "agent:...",
  "baseRevision": "git:...",
  "baseIndexSnapshot": "index:...",
  "projectModelHash": "sha256:..."
}
```

Не использовать долгие глобальные locks.

21.1. Commit algorithm

```text
if currentHead == baseRevision:
    validate
    candidateCommit
    CAS update ref
else:
    compare ReadSet and WriteSet
    resolve anchors against currentHead
    replay Edit IR
    rebuild affected slice
    validate again
    candidateCommit
    CAS update ref
```

21.2. Конфликты

```text
WW conflict:
    transaction пишет semantic fact, изменённый другой transaction

RW conflict:
    transaction читала semantic fact, изменённый другой transaction

Project conflict:
    изменился project model, classpath или compiler configuration
```

21.3. Обязательные правила

|Ситуация                                              |Результат                      |
|------------------------------------------------------|-------------------------------|
|изменились только offsets/whitespace                  |replay допустим                |
|изменён другой независимый symbol                     |replay + validation            |
|оба агента меняют один expression                     |hard conflict                  |
|signature vs body той же функции                      |conflict                       |
|callee summary, использованный другим агентом, изменён|`STALE_REQUIRES_RESLICE`       |
|project model изменён                                 |все transactions snapshot stale|
|два insert в один statement list                      |по умолчанию conflict          |
|два агента добавляют одинаковый import                |idempotent merge               |

────────

22. Transaction ledger

Реализовать append-only ledger.

22.1. Состояния

```text
CREATED
SLICED
EDIT_PREVIEWED
VALIDATING
VALIDATED
REBASING
COMMITTING
COMMITTED
CONFLICTED
STALE_REQUIRES_RESLICE
VALIDATION_FAILED
ABORTED
```

22.2. Запись

```json
{
  "txId": "tx:...",
  "actorId": "agent:...",
  "intent": "...",
  "baseRevision": "git:...",
  "baseIndexSnapshot": "index:...",
  "status": "COMMITTED",
  "operations": ["op:1"],
  "expectedWriteSetHash": "sha256:...",
  "actualWriteSetHash": "sha256:...",
  "validationEvidence": [],
  "candidateCommit": "git:...",
  "finalCommit": "git:..."
}
```

Candidate commit должен содержать trailers:

```text
Semantic-Transaction-Id: tx:...
Semantic-Base-Revision: ...
Semantic-Edit-Hash: ...
```

После crash система должна уметь восстановить итоговый статус transaction.

────────

23. CLI

Реализовать команды:

```bash
sthread project inspect \
  --repo <path>

sthread index \
  --repo <path> \
  --compilation :app/main

sthread resolve symbol \
  --repo <path> \
  --symbol com.acme.OrderService.placeOrder

sthread resolve expression \
  --repo <path> \
  --file src/main/kotlin/com/acme/OrderService.kt \
  --offset 842

sthread cfg \
  --repo <path> \
  --symbol com.acme.OrderService.placeOrder

sthread slice \
  --repo <path> \
  --symbol com.acme.OrderService.placeOrder \
  --direction both \
  --max-nodes 200 \
  --output thread.json

sthread edit preview \
  --repo <path> \
  --thread thread.json \
  --operations edit.json \
  --output preview.json

sthread tx validate \
  --repo <path> \
  --transaction tx.json

sthread tx commit \
  --repo <path> \
  --transaction tx.json \
  --target-ref refs/heads/main

sthread tx inspect \
  --transaction-id tx:...
```

CLI требования:

• --json режим;
• стабильные exit codes;
• stdout содержит только machine-readable output;
• warnings и logs направлять в stderr;
• детерминированная сортировка collections;
• ошибки должны иметь typed code;
• никакого source mutation в preview commands.

────────

24. Этапы реализации

Работай последовательно. Не переходи к следующему этапу, пока не пройдены acceptance gates текущего.

Этап 0. Bootstrap

Создать:

• monorepo;
• Rust workspace;
• Kotlin Gradle composite/build;
• Protobuf generation для Rust и Kotlin;
• framed IPC library;
• minimal worker supervisor;
• sthread doctor;
• CI для Rust tests, Kotlin tests и protocol compatibility.

Gate: Rust CLI запускает Kotlin worker, выполняет handshake и корректно завершает его.

Этап 1. Project Model

Реализовать:

```bash
sthread project inspect
```

Gate: повторный запуск на одном snapshot даёт тот же canonical JSON и ProjectModelHash.

Этап 2. Declaration index

Реализовать:

• file scan;
• hashing;
• declaration extraction;
• SymbolId;
• SQLite persistence;
• incremental update одного файла.

Gate: последовательное и параллельное индексирование дают одинаковый index hash.

Этап 3. K2 semantic facts

Реализовать:

• symbol resolution;
• expression types;
• selected calls;
• receivers;
• argument-to-parameter mapping;
• diagnostics export.

Gate: overloads, extension calls, named/default arguments проходят golden tests.

Этап 4. FIR CFG adapter

Реализовать local CFG export.

Gate: golden fixtures для if, when, loops, try/finally, return, throw, safe call и Elvis совпадают с ожидаемыми графами.

Этап 5. Rust graph analysis

Реализовать:

• graph normalization;
• dominators;
• post-dominators;
• SSA;
• PHI;
• def-use;
• control dependencies.

Gate: все graph-golden tests проходят; порядок nodes не влияет на canonical output.

Этап 6. Thread slicer

Реализовать:

• backward;
• forward;
• both;
• budgets;
• boundaries;
• Thread IR;
• ReadSet.

Gate: для hand-authored fixtures slice содержит все ожидаемые data/control dependencies.

Этап 7. Edit preview

Реализовать:

• ReplaceExpression;
• ReplaceFunctionBody;
• semantic anchor resolution;
• PSI copy;
• K2 candidate validation;
• exact diff;
• ActualWriteSet.

Gate: invalid identifier, ambiguous overload и type mismatch отклоняются; валидная замена проходит.

Этап 8. Transaction commit

Реализовать:

• worktree;
• source byte application;
• compile/test validation;
• candidate commit;
• CAS ref update;
• ledger;
• cleanup.

Gate: failed transaction не меняет target branch; valid transaction создаёт commit с evidence.

Этап 9. Parallel transactions

Реализовать:

• semantic ReadSet comparison;
• semantic rebase;
• stale detection;
• WW/RW conflicts;
• crash recovery.

Gate: обязательная concurrency matrix проходит полностью.

Этап 10. Benchmarks и документация

Реализовать benchmark runner и итоговый отчёт.

────────

25. Критерии корректности

|ID |Инвариант              |Критерий приёмки                                                    |
|---|-----------------------|--------------------------------------------------------------------|
|C0 |Project model корректен|source roots, classpath, options и tasks соответствуют fixture build|
|C1 |Детерминизм            |snapshot + seed + policy дают canonical-identical Thread IR         |
|C2 |Синтаксис              |изменённые `.kt` не содержат parse errors                           |
|C3 |No-op fidelity         |no-op оставляет source bytes неизменными                            |
|C4 |Минимальность          |неизменённые files/ranges byte-identical                            |
|C5 |Anchor safety          |target разрешается ровно в один node                                |
|C6 |Binding safety         |protected references разрешаются в прежние symbols                  |
|C7 |Type correctness       |replacement type совместим с expected type                          |
|C8 |Slice soundness        |golden dependencies не теряются                                     |
|C9 |Explicit incompleteness|budget/unsupported cases всегда маркированы                         |
|C10|Effect control         |неожиданные read/write/throw/suspend changes отклоняются            |
|C11|Build correctness      |affected Gradle compilation проходит                                |
|C12|ABI control            |защищённый public ABI не меняется неявно                            |
|C13|Behavioral evidence    |configured impacted/module tests проходят                           |
|C14|Atomicity              |failed transaction не изменяет branch/worktree/index snapshot       |
|C15|Snapshot isolation     |stale read приводит к conflict/reslice                              |
|C16|Serializable conflicts |non-commuting edits конфликтуют                                     |
|C17|Provenance             |commit восстанавливается до intent/edit/validation/base             |
|C18|Version isolation      |Rust core не зависит от Kotlin/FIR internals                        |

────────

26. Обязательные тесты

26.1. Golden language fixtures

Создать fixtures для:

```text
if/else
when
while/do-while/for
break/continue
try/catch/finally
throw
short-circuit expressions
safe calls
Elvis
local assignments
PHI after branches
PHI in loops
properties
extensions
overloads
named/default arguments
lambda captures
returns
suspend call boundary
Java library call boundary
```

26.2. Slice fixture

Для кода:

```kotlin
fun total(base: Int, premium: Boolean): Int {
    var value = base
    if (premium) {
        value *= 2
    }
    return value
}
```

Backward slice от return value обязан содержать:

```text
parameter base
initial definition value
premium predicate
conditional assignment
PHI
return
```

26.3. Metamorphic tests

```text
whitespace change preserves SymbolId
comment insertion preserves unaffected anchor
neighbor statement insertion preserves unaffected anchor
IR serialize → deserialize → serialize is canonical-identical
no-op edit is byte-identical
failed validation changes no Git ref
AddImport twice equals AddImport once
same-target writes conflict
changed read dependency invalidates transaction
parallel independent edits commute
```

26.4. Concurrency matrix

```text
A and B edit different functions:
    both orders produce canonical-equivalent result

A and B edit same expression:
    one commits, another conflicts

A changes callee semantic summary:
    caller transaction becomes stale

A changes only callee formatting:
    caller transaction may replay

A and B add same import:
    result contains one import

A changes project model:
    all previous semantic transactions become stale
```

────────

27. Производительность

Reference environment:

```text
8 CPU cores
32 GB RAM
NVMe
warm OS cache
warm Gradle daemon
```

Стартовые SLO:

|Операция                                      |Цель           |
|----------------------------------------------|--------------:|
|cold syntax/declaration index, 100k Kotlin LOC|≤ 20 секунд    |
|warm reindex одного файла                     |p95 ≤ 300 мс   |
|resolve symbol                                |p95 ≤ 150 мс   |
|local CFG + SSA, function ≤ 300 LOC           |p95 ≤ 500 мс   |
|local thread extraction                       |p95 ≤ 800 мс   |
|bounded slice ≤200 nodes                      |p95 ≤ 2 секунды|
|edit preview после готового slice             |p95 ≤ 700 мс   |
|anchor resolution                             |p95 ≤ 100 мс   |
|canonical IR serialization                    |p95 ≤ 100 мс   |

Gradle compile и tests измерять отдельно.

При превышении deadline вернуть PARTIAL_BUDGET, не продолжать неограниченный анализ.

Обязательно профилировать отдельно:

```text
worker startup
IPC
serialization
PSI parse
K2 analysis
FIR extraction
Rust graph construction
SSA
slicing
edit validation
Gradle validation
```

Не оптимизировать IPC до появления измерений, подтверждающих, что он является bottleneck.

────────

28. Форматирование и сохранение исходников

Обязательные правила:

• no-op transaction byte-identical;
• неизменённые файлы byte-identical;
• неизменённые ranges вне formatting window byte-identical;
• сохранять line endings;
• сохранять BOM;
• сохранять comments;
• запрещено глобальное reformat;
• запрещён глобальный optimize imports;
• formatting window должен быть явно указан в validation report;
• AddImport должен быть idempotent и детерминирован.

────────

29. Typed errors

Минимальный набор:

```text
UNSUPPORTED_KOTLIN_VERSION
UNSUPPORTED_PROJECT_CONFIGURATION
PROJECT_MODEL_CHANGED
WORKER_PROTOCOL_MISMATCH
WORKER_CRASHED
SYMBOL_NOT_FOUND
AMBIGUOUS_SYMBOL
EXPRESSION_NOT_FOUND
STALE_TARGET
AMBIGUOUS_TARGET
PRECONDITION_FAILED
UNSUPPORTED_CONTROL_FLOW
INCOMPLETE_SEMANTIC_ANALYSIS
SLICE_BUDGET_EXCEEDED
REPLACEMENT_PARSE_ERROR
TYPE_MISMATCH
BINDING_CHANGED
NEW_DIAGNOSTICS
EFFECT_CHANGED
WRITESET_EXCEEDED
COMPILE_FAILED
TEST_FAILED
ABI_CHANGED
RW_CONFLICT
WW_CONFLICT
STALE_REQUIRES_RESLICE
REF_COMPARE_AND_SWAP_FAILED
TRANSACTION_RECOVERY_REQUIRED
```

Ошибка должна содержать:

```text
code
human-readable message
transaction ID
snapshot ID
relevant anchors/symbols
evidence references
retryability
```

────────

30. Observability

Добавить structured logs и metrics:

```text
request duration
worker startup duration
worker memory
cache hit rate
files parsed
semantic facts extracted
CFG nodes
slice nodes
slice boundary count
anchor resolution attempts
validation failures by category
transaction conflicts by category
Gradle validation duration
orphan worktrees
```

Нельзя логировать полные source files по умолчанию.

────────

31. Обязательные ADR

Создать:

```text
ADR-001-polyglot-worker-architecture.md
ADR-002-source-of-truth.md
ADR-003-kotlin-worker-version-isolation.md
ADR-004-thread-ir-and-edit-ir.md
ADR-005-semantic-anchor.md
ADR-006-slicing-completeness.md
ADR-007-transaction-and-conflict-model.md
ADR-008-validation-policy.md
ADR-009-storage-and-cache-invalidation.md
```

Каждый ADR:

```text
Context
Decision
Alternatives considered
Consequences
Failure modes
Compatibility implications
```

────────

32. Запрещённые shortcuts

Запрещено:

• хранить PSI/FIR/Ka objects в индексе;
• использовать offsets как единственный node ID;
• использовать regex для semantic edits;
• переписывать файл целиком для локальной замены без доказанной необходимости;
• применять unified diff как semantic transaction;
• автоматически выбирать неоднозначный target;
• молча пропускать unsupported constructs;
• объявлять truncated slice полным;
• выполнять глобальное форматирование;
• изменять target branch до завершения validation;
• скрывать новые compiler diagnostics;
• пропускать validation после semantic rebase;
• связывать Rust core с конкретной Kotlin compiler version;
• выполнять отдельный RPC для каждого AST node;
• перезапускать JVM worker для каждого запроса;
• считать успешный compile достаточным доказательством корректности;
• начинать MCP или IDE integration до появления детерминированного CLI.

────────

33. Definition of Done

Первая вертикальная версия считается завершённой, когда:

1. sthread project inspect корректно загружает fixture Gradle Kotlin/JVM project.
2. sthread index строит детерминированный incremental index.
3. sthread resolve symbol разрешает функцию, calls и types.
4. sthread cfg выдаёт canonical Local CFG.
5. Rust строит SSA, def-use и control dependencies.
6. sthread slice выдаёт корректный Thread IR.
7. ReplaceExpression применяется к PSI-копии.
8. Invalid replacement отклоняется до source mutation.
9. Valid replacement создаёт минимальный preview diff.
10. Commit выполняется в isolated worktree.
11. Gradle compile и configured tests запускаются автоматически.
12. Failed validation не меняет target branch.
13. Valid transaction создаёт Git commit с transaction trailers.
14. Concurrent same-target edit обнаруживается как conflict.
15. Изменение прочитанного semantic summary приводит к reslice.
16. Worker crash не повреждает repository index и ledger.
17. Все correctness, metamorphic и concurrency tests проходят.
18. Все результаты воспроизводятся из clean checkout одной командой.

────────

34. Итоговые артефакты

Подготовить:

Код

• Rust core;
• Kotlin worker;
• Protobuf schemas;
• CLI;
• test fixtures;
• benchmark runner;
• CI configuration.

Документация

• README.md с быстрым запуском;
• docs/architecture.md;
• docs/correctness-model.md;
• docs/protocol.md;
• docs/progress.md;
• ADRs;
• описание известных ограничений;
• описание threat/failure model.

Демонстрация

Один воспроизводимый сценарий:

```text
1. открыть fixture repository;
2. выбрать функцию;
3. построить thread;
4. показать Thread IR;
5. заменить expression;
6. показать preview;
7. пройти validation;
8. создать commit;
9. запустить параллельную конфликтующую transaction;
10. получить semantic conflict без повреждения ветки.
```

Итоговый отчёт

Создать docs/final-report.md:

```text
что реализовано
какие gates пройдены
какие конструкции Kotlin поддерживаются
какие ограничения остались
результаты correctness tests
результаты performance benchmarks
известные риски Kotlin K2/FIR
предлагаемый следующий этап
готовность архитектуры к TypeScript worker
```

────────

35. Правила работы агента

1. Сначала изучи доступные API на фактически зафиксированной версии Kotlin.
2. Не придумывай методы Analysis API или FIR, которых нет в используемой версии.
3. Если compiler API недоступен или нестабилен, зафиксируй это в ADR и изолируй workaround в Kotlin worker.
4. Каждое архитектурное допущение покрывай тестом либо явно маркируй как непроверенное.
5. Поддерживай docs/progress.md после каждого этапа.
6. Не переходи к сложному interprocedural analysis до прохождения локальной вертикали.
7. Делай небольшие, логически целостные commits.
8. Не оставляй failing tests в основной ветке.
9. Не снижай correctness gates ради демонстрации happy path.
10. В сомнительной ситуации выбирай fail-closed поведение.

────────

36. Приоритеты

При конфликте требований использовать следующий порядок:

```text
1. Корректность и отсутствие повреждения исходников
2. Явное обнаружение неполноты и конфликтов
3. Детерминизм и воспроизводимость
4. Архитектурная изоляция языкового worker
5. Минимальность diff
6. Производительность
7. Полнота поддерживаемых Kotlin-конструкций
8. Удобство CLI
```

Главный принцип проекта:

> Thread IR является семантическим представлением. Edit IR выражает намерение. Source snapshot остаётся источником истины. Transaction coordinator является границей безопасности.
