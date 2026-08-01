# Maven end-to-end: `product-repo`

Дата: 2026-08-01

## Итог

Maven/Kotlin 2.3 вертикаль реализована и независимо принята, но в агентском
benchmark SThread пока не выигрывает. По чистой эффективности патча лучший
результат показал `ast-index`; по корректности победителя нет — слепой
приёмщик отклонил все три патча после fresh hidden tests.

Это важное разделение: быстрый commit не считается победой, если он не проходит
чистую приёмку.

## Методика

Из `product-repo` revision `56d42d5f` создан новый Git-репозиторий с одним
baseline commit, без remote и без достижимой истории эталонного решения. Три
независимых `gpt-5.6-terra`, effort `medium`, работали в отдельных worktree:

1. обычные `rg`/`sed`;
2. cold `ast-index 3.48.1`;
3. ровно один bounded `sthread agent-context` с лимитом 32 КБ.

Каждый агент должен был изменить архивный `products-changefeed` event, не
добавить N+1, сохранить batching и CREATE/UPDATE, запустить Maven compile/tests
и сделать один commit. Время взято из START/FIRST_EDIT/END markers, tokens — из
последнего cumulative `token_count` rollout. Некэшированные tokens считаются
как `input - cached input + output`.

## Результаты эффективности

| Метрика | Default | ast-index | SThread |
|---|---:|---:|---:|
| До первого edit | 74 с | **63 с** | 67 с |
| До commit | 293 с | **171 с** | 351 с |
| Tool calls по rollout trace | 29 | **21** | 34 |
| Изменено файлов | 9 | **3** | 9 |
| Patch | +96/-24 | **+46/-1** | +96/-19 |
| Raw total tokens | 2 208 464 | **1 099 997** | 2 318 681 |
| Некэшированные tokens | 129 744 | **72 925** | 143 449 |
| Goal `tokensUsed` | 127 254 | **69 884** | 140 055 |
| Fresh hidden acceptance | REJECT | REJECT | REJECT |

Ast-index относительно default быстрее до commit на 41,64%, использует на
50,19% меньше raw tokens и на 43,79% меньше некэшированных tokens. SThread
начал edit на 7 секунд раньше default, но завершил commit на 19,8% позже,
использовал на 4,99% больше raw tokens и на 10,56% больше некэшированных.

Относительно ast-index SThread затратил на 105,26% больше времени до commit,
на 110,79% больше raw tokens и на 96,71% больше некэшированных tokens.

## Независимая приёмка

Приёмщик получил только baseline, обезличенные A/B/C patches и fresh evidence.
Он не знал, каким методом создан каждый вариант.

- Default: `REJECT`. Общий тип `ProductModifyEntity` объявляет только `id`,
  поэтому `event.entity.code/title` не компилируются. Concrete captor в тесте
  скрыл неполноту публичного контракта.
- SThread: `REJECT` по той же причине: `ProductChangeEntity` объявляет только
  `id`, хотя runtime subtype содержит остальные поля.
- Ast-index: `REJECT`. Он повторно загружает полный canonical DTO одним batch
  query, но `ProductCanonicalProjection.code` ошибочно non-null. На чистом
  worktree 6 из 109 archive-тестов падают с NPE для nullable DB code. Заявленный
  агентом PASS оказался результатом stale incremental build state.

Все варианты сохранили русские `@DisplayName`; default и SThread использовали
один projection query на batch, ast-index не создал N+1, но добавил второй
batch query и полный payload вместо минимальной archive entity.

## Почему проиграл SThread

Cold `agent-context` занял 21,567 с — лишь 6,1% от полного 351-секундного
workflow. Следовательно, ускорение самого индекса не решит проигрыш.

Полный evidence показывает структурную проблему выдачи:

- для `ProductService` вернулся 2400-байтный class-head, а не тело `archive`;
- repository projection и его nullability не попали в пакет;
- 12 test entries содержали первые 1200 байт файлов, а не snippets около
  archive assertions;
- declared field contract `entity.id/code/title` не был замкнут транзитивно;
- `tee` не сохранил canonical stdout, потому что `.semantic-thread` создавался
  CLI одновременно; повтор запрещался протоколом benchmark.

Пакет формально имел `COMPLETE`, но был неполон относительно задачи. Агенту
пришлось сделать 34 tool calls и он всё равно проверил concrete subtype вместо
статического event contract.

## Выбранная следующая оптимизация

Следующая линия — не микрооптимизация cold K2, а **acceptance-driven contract
closure и task-aware ranking**:

1. Если query terms соединяются вызовом, поднимать конкретный member
   (`ProductService.archive`) и его repository calls выше class declaration.
2. Транзитивно выдавать поля и nullability объявленных DTO/interface contracts.
3. Для тестов возвращать anchored snippets около релевантных методов, а не
   префиксы файлов.
4. Добавить `--output` для атомарной записи bounded canonical context.
5. Завершать агентский путь clean detached-worktree validation через уже
   существующую semantic transaction vertical.

Целевой следующий gate: корректный hidden acceptance, не более 16 tool calls,
не более 180 секунд до commit и не более 75 000 некэшированных tokens. Только
после прохождения correctness gate имеет смысл повторно сравнивать победителя.

Машиночитаемые данные: `benchmarks/reports/maven-product-repo.json`.
