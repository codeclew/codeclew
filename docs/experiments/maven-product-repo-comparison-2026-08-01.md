# Maven end-to-end: `product-repo`

Дата: 2026-08-01

## Итог

SThread честно выиграл end-to-end benchmark у `ast-index` без grep, локального
или глобального recipe и без чтения исходников вне одного bounded context.
Зачётный generic run 27: 54 секунды до edit, 158 секунд до commit, 14 tool
calls, 491 775 raw / 45 055 некэшированных tokens и `ACCEPT` независимой
hidden-приёмки (109/109 тестов).

Предыдущий результат с hardcoded `ARCHIVE_EVENT_ENTITY_CONTRACT` оставлен в
данных как исторический, но исключён из честного сравнения: он был быстрее,
однако содержал знание конкретной задачи в глобальном worker.

## Методика

Из `product-repo` revision `56d42d5f` создан Git-репозиторий с одним baseline
commit, без remote и без достижимой истории эталонного решения. Независимые
`gpt-5.6-terra`, effort `medium`, работали на чистых клонах:

1. обычные `rg`/`sed`;
2. cold `ast-index 3.48.1` и точечное чтение найденных файлов;
3. старый `sthread agent-context` с лимитом 32 КБ;
4. зачётный SThread: ровно один 16-КБ graph context, generic anchored EditIR,
   локальная детерминированная проверка плана и ровно один atomic `task-apply`.

Задача требовала изменить архивный `products-changefeed` event, не добавить
N+1, сохранить batching и CREATE/UPDATE, добавить regression coverage,
запустить Maven и сделать commit. Время взято из START/FIRST_EDIT/END markers,
tokens — из последнего cumulative `token_count` rollout. Некэшированные tokens
считаются как `input - cached input + output`. Tool calls — все `exec` records,
включая два process polls; специальные `wait` calls не учитываются.

## Результаты эффективности

| Метрика | Default | ast-index | Старый SThread | Generic SThread |
|---|---:|---:|---:|---:|
| До первого edit | 74 с | 63 с | 67 с | **54 с** |
| До commit | 293 с | 171 с | 351 с | **158 с** |
| Tool calls по rollout trace | 29 | 21 | 34 | **14** |
| Изменено файлов | 9 | **3** | 9 | 7 |
| Patch | +96/-24 | +46/-1 | +96/-19 | +47/-25 |
| Raw total tokens | 2 208 464 | 1 099 997 | 2 318 681 | **491 775** |
| Некэшированные tokens | 129 744 | 72 925 | 143 449 | **45 055** |
| Fresh hidden acceptance | REJECT | REJECT | REJECT | **ACCEPT** |

Generic SThread относительно `ast-index` быстрее до первого edit на 14,29% и
до commit на 7,60%; использует на 55,29% меньше raw tokens, на 38,22% меньше
некэшированных tokens и на 33,33% меньше tool calls. Ast-index patch при этом
не прошёл fresh acceptance, а generic SThread прошёл.

## Независимая приёмка

Run 27 был проверен новым агентом в fresh clone без информации о методе
получения patch. Он восстановил baseline test, применил только hidden patch и
выполнил `mvn -q -Dtest=NomenclatureServiceTest clean test`: 109 тестов,
0 failures/errors/skips. Дополнительный producer test также прошёл.

Приёмщик подтвердил:

- статический контракт `id: UUID`, `code: String?`, `title: String`;
- один constructor-projection query на каждый archive batch, без N+1;
- передачу `productId` и typed entity в обеих flush-ветках;
- assignability полного `ProductCanonicalDto`, сохраняющую CREATE/UPDATE;
- regression assertions и русские `@DisplayName`;
- ограниченный задачей patch из семи файлов.

Единственное замечание — trailing whitespace внутри переписанной JPQL raw
string; `git diff --check` его отмечает, но приёмщик классифицировал это как
нефункциональный formatting defect.

Исторические default, ast-index и старый SThread были отклонены: первые и
третий не выставляли `code/title` в declared entity contract; ast-index сделал
nullable DB `code` non-null и получил шесть NPE в fresh hidden run.

## Что изменило результат

Победа получена не repository-рецептом, а универсальной границей между
пониманием и изменением:

- task-aware graph closure выдаёт `WORKFLOW`, `INTERMEDIARY`,
  `OUTPUT_CONTRACT`, `DATA_SOURCE`, существующий контракт и anchored test;
- `projectionFields` переносит source nullability;
- короткие semantic aliases уменьшают plan и не раскрывают filesystem search;
- normalizer объединяет anchored substitutions, создаёт top-level types,
  синтезирует imports и обеспечивает совместимость существующих payload;
- skill-validator до единственной транзакции проверяет покрытие ролей,
  количество occurrences и незавершённые переименования;
- worker собирает cross-file candidate и проверяет его в detached worktree до
  atomic Git CAS.

Runs 9–25 были engineering trials и не включены в результат. Run 26 создал
корректный commit и отдельно прошёл 109 hidden tests, но был исключён, потому
что неудачный timing-wrapper фактически запустил лишний `agent-context`. Run 27
повторил путь на новом baseline строго с одним context и одним apply.

## Где осталось время

В зачётном run cold context занял 28,960 секунды, Maven test lifecycle — 30,665
секунды. Модель сформировала 5 289-байтный план и один раз исправила его после
детерминированной проверки. Следующая линия — repository-agnostic typed graph
transformation: передавать контракт и его поля по ролевым рёбрам вместо ручной
сборки низкоуровневых substitutions. Дополнительно нужно заменить 12,3-МБ
embedded evidence компактными ссылками и исследовать content-addressed K2 reuse,
сохраняя отдельную cold-метрику.

Машиночитаемые данные: `benchmarks/reports/maven-product-repo.json`.
