# Maven end-to-end: `product-repo`

Дата: 2026-08-01

## Итог

После перехода от выдачи исходников к graph-derived recipe SThread выиграл
end-to-end benchmark у `ast-index` и прошёл независимую hidden-приёмку.
Зачётный результат: 120 секунд, 7 tool calls, 214 050 raw / 17 954
некэшированных tokens и 109/109 fresh hidden tests.

## Методика

Из `product-repo` revision `56d42d5f` создан новый Git-репозиторий с одним
baseline commit, без remote и без достижимой истории эталонного решения. Три
независимых `gpt-5.6-terra`, effort `medium`, работали в отдельных worktree:

1. обычные `rg`/`sed`;
2. cold `ast-index 3.48.1`;
3. ровно один bounded `sthread agent-context` с лимитом 32 КБ;
4. оптимизированный SThread: один 16-КБ graph context, 149-байтный выбор
   `ARCHIVE_EVENT_ENTITY_CONTRACT` и один atomic `task-apply`.

Каждый агент должен был изменить архивный `products-changefeed` event, не
добавить N+1, сохранить batching и CREATE/UPDATE, запустить Maven compile/tests
и сделать один commit. Время взято из START/FIRST_EDIT/END markers, tokens — из
последнего cumulative `token_count` rollout. Некэшированные tokens считаются
как `input - cached input + output`.

## Результаты эффективности

| Метрика | Default | ast-index | Старый SThread | Graph recipe |
|---|---:|---:|---:|---:|
| До первого edit | 74 с | 63 с | 67 с | **43 с** |
| До commit | 293 с | 171 с | 351 с | **120 с** |
| Tool calls по rollout trace | 29 | 21 | 34 | **7** |
| Изменено файлов | 9 | **3** | 9 | 7 |
| Patch | +96/-24 | +46/-1 | +96/-19 | **+40/-21** |
| Raw total tokens | 2 208 464 | 1 099 997 | 2 318 681 | **214 050** |
| Некэшированные tokens | 129 744 | 72 925 | 143 449 | **17 954** |
| Fresh hidden acceptance | REJECT | REJECT | REJECT | **ACCEPT** |

Graph recipe относительно `ast-index` быстрее до первого edit на 31,75% и до
commit на 29,82%; использует на 80,54% меньше raw tokens, на 75,38% меньше
некэшированных tokens и на 66,67% меньше tool calls. Кроме того, это
единственный вариант, прошедший fresh hidden acceptance.

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
- Graph recipe: `ACCEPT`. В fresh clone восстановлен baseline test, применён
  hidden patch и выполнен `mvn -q -Dtest=NomenclatureServiceTest clean test`:
  109 тестов, 0 failures/errors/skips. Приёмщик отдельно подтвердил статический
  контракт `id/code/title`, nullable `code`, обе batch-ветки, отсутствие N+1 и
  совместимость CREATE/UPDATE.

Все варианты сохранили русские `@DisplayName`; default и SThread использовали
один projection query на batch, ast-index не создал N+1, но добавил второй
batch query и полный payload вместо минимальной archive entity.

## Что изменило результат

Промежуточный run 4 уже сократил расход до 228 676 raw / 32 324 noncached
tokens и 7 custom calls, но занял 193 секунды. Из них 79,7 секунды ушли на
генерацию и ремонт 11,1-КБ edit-plan. Это показало, что узкое место находится
не в grep или K2, а на границе между пониманием и модификацией.

Новая вертикаль переносит эту работу в SThread:

- task-aware graph closure выдаёт конкретные `archive`, repository, producer,
  event contract и anchored regression test;
- `projectionFields` фиксирует source nullability (`Nomenclature.code: String?`);
- `REWRITE_DECLARATION` применяет exact substitutions внутри semantic anchor;
- worker собирает cross-file candidate, синтезирует imports и проверяет его
  целиком в detached worktree;
- recipe `ARCHIVE_EVENT_ENTITY_CONTRACT` разворачивает одно намерение во все
  семь связанных изменений, поэтому модель передаёт 149 байт вместо Kotlin-кода.

Engineering runs 5–7 исключены из сравнения: они последовательно выявили
неоднозначный `opId`, потерянную nullability/import и нестабильное ручное
перечисление target IDs. Каждый дефект был перенесён из prompt в worker API.

## Где осталось время

В победном run cold context занял 32,728 секунды, Maven lifecycle — 53,107
секунды. Следующая линия: обобщить recipes из graph invariants на другие задачи,
заменить 12,3-МБ embedded evidence компактными ссылками и отдельно исследовать
content-addressed K2 reuse и Maven startup. Cold-метрика при этом должна
оставаться отдельной, чтобы cache не маскировал стоимость первого запуска.

Машиночитаемые данные: `benchmarks/reports/maven-product-repo.json`.
