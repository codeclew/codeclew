# Задача для Deep Research: универсальное семантическое изменение кода

Дата постановки: 2026-08-02

## Контекст

Codeclew должен помогать агенту быстро изолировать затронутую поверхность
Kotlin/JVM-задачи, сформировать изменение и применить его одной проверяемой
транзакцией. Целевое преимущество — существенно меньше полного end-to-end
времени, model/tool round trips и raw/noncached tokens, чем у обычного
grep/read workflow и навигации через AST-индекс.

Текущая реализация уже умеет:

- строить bounded task context из entrypoint, exact targets, ограниченного
  graph closure, контрактов и тестов;
- оставлять полный K2/index evidence локально и показывать модели ограниченную
  проекцию;
- отклонять неполный context;
- раскрывать короткий transient semantic goal `PROPAGATE_TYPED_FIELDS` в
  низкоуровневый EditIR;
- нормализовать план, синтезировать часть imports и targeted test routing;
- материализовать candidate, выполнить Gradle/Maven validation и атомарно
  опубликовать commit.

Однако победа не доказана как универсальная. В одном Maven-сценарии короткий
transient goal дал сильный результат, а в wiring/test-сценарии основное время
переместилось из discovery в model-authored low-level plan и test source.

## Доступные и недоступные данные

Исследователю доступны:

- все исходники текущего репозитория Codeclew;
- его Git-история;
- встроенные Gradle, Maven и Kotlin fixtures;
- unit, integration, metamorphic и concurrency tests;
- сохранённые benchmark JSON и документы в репозитории;
- публичные научные статьи, спецификации и исходники других инструментов.

Исследователю **недоступны**:

- `pim-migrator`, `product-repo` и любые другие прежние тестовые репозитории;
- временные worktree и файлы из `/private/tmp`;
- повторный запуск исторических закрытых benchmark-задач;
- полная новая token telemetry, если она не сохранена в репозитории.

Следствие: исторические отчёты по закрытым репозиториям можно использовать
только как наблюдения для генерации гипотез. Нельзя считать их независимо
воспроизводимым доказательством, восстанавливать отсутствующие tokens из bytes
или делать на их основании вывод о «большинстве задач».

## Цель исследования

Определить следующую архитектурную линию Codeclew, которая имеет проверяемую
причину сокращать model-owned работу, число model/tool turns, размер
накопленного transcript и полное время решения для широкой, заранее
определённой совокупности прикладных Kotlin-задач.

Исследование должно закончиться одним из трёх вердиктов:

1. `GO_IMPLEMENT` — архитектура достаточно формализована, а текущего evidence
   достаточно для реализации первого общего semantic-goal compiler.
2. `GO_BUILD_CORPUS_FIRST` — архитектура правдоподобна, но сначала нужен
   нейтральный генерируемый benchmark corpus и измерительный harness.
3. `STOP_NOT_PLAUSIBLE` — заявленное преимущество нельзя получить общей
   архитектурой Codeclew без repository/task-specific automation.

Вердикт должен быть фальсифицируемым: указать, какие наблюдения заставят его
изменить.

## Обязательный аудит исходников Codeclew

Сначала восстановить фактический pipeline по коду, а не по описаниям:

```text
task text
  -> agent-context / task_context::build
  -> bounded context + full evidence
  -> model-owned goal или low-level plan
  -> task_plan expansion + plan normalization
  -> EditIR
  -> semantic preview and validation
  -> Gradle/Maven tests
  -> Git transaction
```

Минимальный набор исходников для проверки:

- `crates/sthread/src/task_context.rs` — selection, requirements, roles,
  contracts, budgets и completeness;
- `crates/sthread/src/task_plan.rs` — текущий transient compiler и его
  structural assumptions;
- `crates/sthread/src/main.rs` — `agent-context`, `task-apply`, plan
  normalization, imports и test routing;
- `crates/sthread/src/agent_context.rs` — предыдущая модель context pack;
- `crates/sthread/src/graph.rs` и `crates/sthread/src/ir.rs` — доступная
  графовая и типовая информация;
- `crates/sthread/src/transaction.rs` и `crates/sthread/src/worker.rs` — границы
  validation и atomic apply;
- `workers/kotlin*/` — что реально можно доказать через PSI/K2;
- `crates/sthread/tests/` и `fixtures/` — текущая ширина тестового контура;
- `benchmarks/reports/*.json` — только сохранённые измерения и их ограничения;
- `docs/experiments/universal-task-surface-model-2026-08-02.md` — гипотезы и
  отрицательный итог предыдущей цели.

Для каждого этапа pipeline указать:

- вход и выход;
- какая информация полная, bounded, эвристическая или отсутствует;
- что вычисляет worker и что вынуждена решать модель;
- какие ошибки обнаруживаются до materialization, компиляцией и тестами;
- сколько потенциальных model turns создаёт этап;
- зависит ли объём model-visible данных от размера репозитория, размера
  task surface или размера textual patch.

## Основные исследовательские направления

### 1. Формальная модель стоимости

Построить модель полного решения, отдельно учитывающую:

```text
T_total = T_discovery_worker
        + T_model_context_reasoning
        + T_model_plan_generation
        + T_plan_validation_and_retries
        + T_materialization
        + T_compile_and_tests
        + T_repair_turns

Tokens_total = system/task input
             + cumulative transcript replay
             + model-visible context
             + plan/test output
             + diagnostics and repair turns
```

Модель должна объяснить:

- когда дорогой K2 context окупается относительно одного или нескольких `rg`;
- почему AST-index может уменьшить один ответ, но не обязательно уменьшает
  количество query/read/reasoning turns;
- почему bounded context сам по себе не гарантирует экономию, если модель
  воспроизводит source в низкоуровневом plan;
- какие переменные должны быть ограничены константой, task-surface size или
  semantic delta, чтобы преимущество сохранялось при росте репозитория;
- существует ли нижняя граница model-owned информации, без которой нельзя
  корректно выполнить изменение.

Не подменять tokens байтами. Bytes допустимы только как отдельная метрика
model-visible payload и plan size.

### 2. Граница между intent, goal, plan и EditIR

Определить минимальный model-owned артефакт. Сравнить как минимум четыре
варианта:

1. модель пишет точные textual substitutions;
2. модель выбирает один параметризованный transform kind;
3. модель задаёт postconditions и invariants, а worker решает constrained
   synthesis/search;
4. модель выбирает semantic graph delta, а worker компилирует его в PSI edits.

Для каждого варианта ответить:

- что остаётся универсальным, а что неизбежно превращается в recipe catalog;
- как растёт goal/plan при увеличении количества затронутых слоёв;
- какие ambiguities можно снять K2/PSI evidence, а какие требуют бизнес-решения;
- можно ли проверить goal до изменения исходников;
- какой proof object должен вернуть worker, чтобы модель не перечитывала code;
- как fail closed, если существует несколько корректных placements или
  implementations.

Отдельно решить, должен ли универсальный слой быть каталогом transform kinds
или небольшим constraint language над declarations, types, calls, effects,
data flow и behavioral obligations.

### 3. Полнота task surface

Проверить формулу:

```text
TaskSurface = EntryPoints
            union ExactTargets
            union BoundedGraphClosure
            union Contracts
            union Tests
```

Ответить:

- достаточны ли эти классы для прикладных изменений;
- нужны ли отдельные configuration, lifecycle, transaction, coroutine,
  persistence-schema, serialization и external-API boundaries;
- может ли `COMPLETE_TASK` иметь формально проверяемое значение или это всегда
  приближение;
- чем заменить фиксированные лимиты surfaces/contracts;
- как budget должен зависеть от обязательств задачи, а не от top-k ranking;
- какие negative completeness tests отсутствуют в текущем репозитории.

### 4. Роль графов

Установить, какие графы реально уменьшают model-owned reasoning:

- call graph;
- type/override/assignability graph;
- data-flow и def-use;
- control/effect graph;
- persistence projection flow;
- test-to-production coverage/trace graph;
- build/module/dependency graph.

Для каждого графа указать:

- какой класс решений он позволяет вывести локально;
- есть ли требуемые факты в текущем IR/evidence;
- какова цена построения и инкрементального обновления;
- что должно попасть в bounded context, а что оставаться локальным;
- какие ложные выводы возможны из-за reflection, DI, framework lifecycle,
  overloads, extension functions, generics, nullability и coroutines.

Исследовать не только retrieval graph, но и **change graph**: набор semantic
obligations, которые должны совместно измениться, чтобы типы, callers,
implementations, projections и tests остались согласованными.

### 5. Универсальный semantic-goal compiler

Использовать `PROPAGATE_TYPED_FIELDS` и предложенный `WIRE_TYPED_DECORATOR`
только как probes двух разных structural families, не как заранее выбранное
решение.

Для `WIRE_TYPED_DECORATOR` проверить возможность вывести без task vocabulary:

- единственный `config(): C`;
- единственный `T.decorate(C): T` или эквивалентную pure function;
- единственный source `List<T>` и совместимый downstream consumer;
- placement с однократным вычислением config и сохранением evaluation order;
- imports, overload resolution, nullability и collection kind;
- отсутствие опасных coroutine, transaction и lifecycle boundaries.

Затем определить, является ли это частным случаем более общего goal, например:

```text
Introduce value C once in region R
Transform each T produced at edge E with function F(T, C) -> T
Preserve consumer contract, effects and ordering
Require behavioral obligations O
```

Нужны:

- предлагаемая schema goal;
- статические invariants применимости;
- алгоритм binding goal к текущему graph evidence;
- алгоритм выбора PSI edit points;
- proof/failure report;
- граница model-owned значений;
- оценка goal bytes и количества model turns;
- минимум три контрпримера, на которых compiler обязан отказаться.

### 6. Test oracle и regression coverage

Это отдельная проблема, а не побочный этап edit compiler.

Исследовать:

- когда тест следует формально из transformation laws;
- когда можно безопасно создавать test inputs из data-class defaults,
  constructors и nullable fields;
- когда behavioral oracle отсутствует в коде и должен остаться model-owned;
- может ли worker выбирать и усиливать существующий тест вместо создания
  model-authored файла;
- можно ли использовать differential, metamorphic, property-based и mutation
  testing как замену hand-authored oracle;
- как отличить тест, проверяющий concrete subtype, от теста declared public
  contract;
- как не считать compile-only или self-confirming тест доказательством;
- какой минимальный test goal должен передать агент.

Результат должен классифицировать test changes как минимум на:

1. полностью выводимые worker;
2. parametrically generated с model-owned expected values;
3. требующие model-authored oracle;
4. неавтоматизируемые без внешней спецификации.

### 7. Безопасность materialization

Проверить, где текущий pipeline всё ещё использует text heuristics после
семантического анализа. В частности, оценить риски normalizers для extension
calls, imports и substitutions внутри strings/comments/nested expressions.

Ответить:

- какие операции должны стать PSI-native;
- какие preconditions обязаны включать resolved symbol/type/effect, а не только
  source fragment и occurrence count;
- можно ли сделать materialization детерминированной функцией goal + evidence;
- какие свойства должны перепроверяться после edit до build;
- какие классы ошибок сейчас ловит только compiler/test и создают дорогой
  repair turn.

### 8. Корпус без закрытых репозиториев

Спроектировать воспроизводимый benchmark corpus внутри Codeclew. Он не должен
копировать имена, доменные поля или точные patches прежних задач.

Предпочтительная конструкция:

- нейтральные Gradle и Maven project templates;
- генератор base revisions из seed;
- генератор task description, semantic oracle patch и hidden tests;
- семантически эквивалентные варианты имён, форматирования, package layout,
  module count, overloads и decoy symbols;
- варианты размера репозитория с реальными dependency/call paths, а не только
  дублированными строками;
- withheld seeds, генерируемые после заморозки worker;
- immutable manifest с family, difficulty, expected surface и forbidden
  shortcuts;
- независимый acceptance runner, который не раскрывает oracle агенту.

Корпус должен включать не менее пяти заранее определённых семейств:

1. wiring существующих producer/transformer/consumer;
2. type/signature propagation через несколько слоёв;
3. DTO/event/API contract evolution с несколькими branches;
4. persistence projection и nullability;
5. configuration/annotation/lifecycle change;
6. error/retry/resource-handling change;
7. test-only regression strengthening.

Исследователь должен определить, какие пять или более семейств действительно
представляют целевую популяцию «прикладных задач». Нельзя объявить большинство
по набору, специально составленному из применимых transforms. Желательно
обосновать распределение публичными исследованиями software changes или
случайной выборкой публичных Kotlin issues/commits, не перенося их исходники в
закрытый benchmark.

Для каждого family нужны positive, ambiguous и must-refuse cases.

### 9. Честное сравнение с default и AST-index

Спроектировать paired protocol с одинаковыми:

- task text и hidden acceptance;
- base revision;
- моделью, reasoning effort и системными инструкциями;
- cold/warm cache режимами;
- возможностью использовать build tools;
- time origin и завершением только после clean accepted commit.

Режимы:

1. default filesystem search/read/edit;
2. AST-index как основной navigation tool с разрешённым чтением найденного
   source;
3. Codeclew context + semantic goal + atomic apply.

Обязательные метрики:

- correctness и hidden acceptance прежде скорости;
- полное start-to-accepted-commit время;
- время worker, model reasoning/planning, build/test отдельно;
- model/tool calls и discovery calls;
- raw input, cached input, output и вычисленные noncached tokens;
- model-visible bytes, plan/goal bytes и diagnostics bytes;
- validator attempts, repair turns и failed applies;
- количество прочитанных source files и fallback-search;
- applicability/refusal rate;
- cold и warm measurements отдельно.

Не сравнивать сумму tool wall с agent end-to-end. Не исключать неудачные
запуски без заранее определённого правила и полного журнала причин.

## Вопросы, на которые нужен однозначный ответ

Финальный отчёт обязан ответить по номерам:

1. Где сейчас находится доминирующий model-owned bottleneck?
2. Какая часть этого bottleneck устранима локальным детерминированным worker?
3. Какой минимальный semantic goal достаточен для первого выбранного family?
4. Является ли goal language общим constraint language или каталогом macros?
5. Как предотвратить превращение macros в скрытые repository recipes?
6. Какие graph facts отсутствуют для компиляции goal в безопасный EditIR?
7. Можно ли формально определить `COMPLETE_TASK` для выбранного family?
8. Какие ambiguities требуют участия модели и сколько turns они добавляют?
9. Какие тестовые oracle можно вывести, а какие нельзя?
10. Какие текущие text heuristics необходимо заменить PSI-native edits до
    следующего benchmark?
11. Как будет построен withheld corpus без закрытых репозиториев?
12. Как определяется целевая популяция «большинства прикладных задач»?
13. Какой ожидается applicability rate выбранной архитектуры?
14. Почему ожидаемая экономия должна сохраняться при росте repository size?
15. Какой теоретический и измеримый upper bound у model-visible context и
    model-authored output?
16. При каких условиях default `rg` гарантированно останется быстрее?
17. В каких случаях AST-index принципиально достаточен и Codeclew не окупится?
18. Какие correctness gates нельзя обменивать на скорость или tokens?
19. Какой один следующий эксперимент имеет максимальную information gain?
20. Какой итоговый вердикт: `GO_IMPLEMENT`, `GO_BUILD_CORPUS_FIRST` или
    `STOP_NOT_PLAUSIBLE`?

## Требуемые deliverables

1. **Architecture map** фактического pipeline с file/line references.
2. **Bottleneck table**: worker time, model time, context, plan, validation и
   build; что известно, неизвестно и только предполагается.
3. **Cost model** для default, AST-index и Codeclew с условиями победы и
   проигрыша.
4. **Design alternatives matrix** минимум для textual plan, transform catalog,
   constraint synthesis и graph-delta compiler.
5. **Recommended architecture RFC**: schema, invariants, binding algorithm,
   proof/failure output и model/worker ownership.
6. **Test-oracle strategy** с чёткими границами автоматизации.
7. **Neutral corpus specification** и схема генератора внутри Codeclew.
8. **Benchmark protocol** с точными метриками и preregistered exclusions.
9. **Risk register**: overfitting, false completeness, framework semantics,
   unsafe text rewrite, token measurement и build-dominated tasks.
10. **Decision** с confidence level, falsifiers и минимальным планом следующих
    трёх implementation commits или рекомендацией не реализовывать transform.

## Предварительные критерии возобновления цели

Исследователь может предложить другие пороги, но обязан обосновать изменения.
До начала реализации рекомендуется зафиксировать:

- не менее 30 withheld задач и не менее 5 семейств;
- корректность Codeclew не ниже лучшего baseline;
- applicability не менее 60% целевой выборки, иначе нельзя говорить о
  большинстве задач;
- accepted win rate не менее 70% применимых задач против каждого baseline;
- снижение median full end-to-end не менее 30% против default и AST-index;
- снижение median noncached tokens не менее 30%, raw tokens не менее 40%;
- верхняя граница 95% confidence interval для парной разницы также должна
  показывать выигрыш, а не только point estimate;
- отсутствие task/repository vocabulary в worker, skill и corpus generator;
- must-refuse cases завершаются до edit и не создают candidate commit;
- все failed/retried runs остаются в итоговом отчёте.

Если архитектура покрывает только узкий structural family, результат может
быть полезным feature, но не считается выполнением универсальной цели.

## Дисциплина доказательств

Каждое существенное утверждение помечать одним из типов:

- `CODE` — непосредственно подтверждено текущими исходниками;
- `TEST` — подтверждено воспроизводимым тестом/fixture Codeclew;
- `ARTIFACT` — взято из сохранённого отчёта, но исходный repo недоступен;
- `LITERATURE` — подтверждено первичным внешним источником;
- `INFERENCE` — логический вывод из явно перечисленных предпосылок;
- `HYPOTHESIS` — требует будущего эксперимента.

Документация и benchmark summaries не являются истиной о реализации, пока
утверждение не сверено с кодом. Для внешнего исследования приоритетны papers,
официальные спецификации и исходники инструментов. Маркетинговые сравнения не
использовать как доказательство.

## Запреты

- Не проектировать решение под имена или поля прежних закрытых задач.
- Не предлагать repository-specific recipes как доказательство универсальности.
- Не считать два прежних кейса репрезентативной выборкой.
- Не оценивать tokens по JSON/source bytes.
- Не путать navigation speed, tool wall и полный agent end-to-end.
- Не считать successful compile достаточной behavioral acceptance.
- Не начинать реализацию нового transform до выбора архитектуры и определения
  falsification corpus.

## Ожидаемый формат финального ответа Deep Research

1. Краткий verdict и confidence.
2. Что доказано по исходникам Codeclew.
3. Что нельзя доказать без будущих запусков.
4. Ответы на 20 обязательных вопросов.
5. Рекомендуемая архитектура и её альтернативы.
6. Спецификация neutral withheld corpus.
7. Измерительный protocol и критерии победы.
8. Риски и falsifiers.
9. Следующий шаг с максимальной information gain.

Главный критерий качества исследования: после него команда должна понимать не
только **что реализовать**, но и **почему эта линия способна системно уменьшить
модельную работу**, на какой доле задач это ожидается и какой эксперимент
быстро опровергнет ошибочную гипотезу.
