# Maven end-to-end: `product-repo`

Дата: 2026-08-01

## Итог

Clew выиграл end-to-end benchmark у `ast-index` и default workflow без
grep, чтения исходников вне одного bounded context, локального recipe или
заранее зашитого решения. Зачётный run 34: 50 секунд до edit, 97 секунд до
commit, 11 измеряемых tool calls, 374 756 raw / 37 348 некэшированных tokens и
`ACCEPT` независимой hidden-приёмки — 109 тестов без failures/errors.

Победа получена за счёт transient change graph. Он строится из evidence
текущего запуска и компилирует 398-байтную semantic goal в полный EditIR.
Модель задаёт только два новых имени типов и import; data source, collection,
loop item, identity/payload bindings, test binding, occurrences и substitutions
выводит worker из полного resolved graph.

Исторический hardcoded recipe-run оставлен в данных, но дисквалифицирован.
Run 27 также исключён задним числом: он прошёл hidden tests и пороги, однако
строгий аудит обнаружил task-shaped vocabulary в глобальном worker/tests.

## Методика

Из `product-repo` revision `56d42d5f` подготовлен Git baseline без достижимой
истории эталонного решения. Независимые `gpt-5.6-terra`, effort `medium`,
работали на fresh clones:

1. обычные `rg`/`sed`;
2. cold `ast-index 3.48.1` и чтение найденных файлов;
3. старый 32-КБ `clew agent-context`;
4. Clew transient graph: один 16-КБ context, goal v0.4, один validator и
   один atomic `task-apply`.

Задача требовала сохранить `productId`, добавить статически типизированный
`entity {id, code, title}` для `DELETED`, не добавить N+1, сохранить batching и
CREATE/UPDATE, добавить regression coverage, запустить Maven и сделать commit.

Время взято из START/FIRST_EDIT/END markers. Tokens — из последнего cumulative
`token_count` rollout; noncached = `input - cachedInput + output`. Tool calls —
11 `exec`-вызовов; два orchestration `wait` указаны отдельно и, как в исходной
методике, не входят в сравнимый счётчик.

## Результаты

| Метрика | Default | ast-index | Старый Clew | Transient Clew |
|---|---:|---:|---:|---:|
| До первого edit | 74 с | 63 с | 67 с | **50 с** |
| До commit | 293 с | 171 с | 351 с | **97 с** |
| Сравнимые tool calls | 29 | 21 | 34 | **11** |
| Все calls с orchestration wait | — | — | — | 13 |
| Изменено файлов | 9 | **3** | 9 | 7 |
| Patch | +96/-24 | +46/-1 | +96/-19 | +39/-19 |
| Raw total tokens | 2 208 464 | 1 099 997 | 2 318 681 | **374 756** |
| Некэшированные tokens | 129 744 | 72 925 | 143 449 | **37 348** |
| Fresh hidden acceptance | REJECT | REJECT | REJECT | **ACCEPT** |

Относительно `ast-index` transient Clew быстрее до edit на 20,63%, до
commit на 43,27%, использует на 65,93% меньше raw и на 48,79% меньше
некэшированных tokens, а также на 47,62% меньше сравнимых tool calls.

## Независимая приёмка

Новый агент создал fresh clone candidate `dca765dc`, восстановил baseline test,
применил скрытый patch и выполнил
`mvn -q -Dtest=NomenclatureServiceTest clean test`: 109 тестов, 0 failures,
0 errors, 0 skipped.

Аудит подтвердил:

- контракт `id: UUID`, `code: String?`, `title: String`;
- сохранение persistence nullability;
- один constructor-projection query на archive batch, без repository calls в
  loop;
- `productId` и typed entity в обеих flush-ветках;
- `ProductCanonicalDto` остаётся assignable для CREATE/UPDATE;
- regression matcher полей и русский `@DisplayName`;
- task-scoped patch и чистый исходный candidate worktree.

Нефункциональное замечание: прежние пробелы JPQL raw string приводят к
`git diff --check` warning на изменённой строке.

## За счёт чего получен выигрыш

Clew сначала изолирует поверхность один раз:

`WORKFLOW → DATA_SOURCE → INTERMEDIARY → OUTPUT_CONTRACT`, плюс существующий
assignable contract, projection fields с nullability и anchored regression
test. Compact context остаётся меньше 16 КБ; полный evidence используется
worker'ом, но не пересылается модели повторно.

Затем transient compiler:

- проверяет единственный resolved path и отсутствие коллизий новых типов;
- выводит текущие method/collection/loop/sink/test bindings;
- строит общий typed contract и bulk projection;
- протягивает тип через все роли и обе workflow-ветки;
- формирует точные substitutions/occurrences и regression matcher;
- передаёт полный EditIR существующей detached Maven/Git транзакции.

Поэтому модель больше не тратит время и tokens на 5–11 КБ ручных замен. Цена
построения transient структуры полностью входит в 97 секунд benchmark; это не
сохранённый repository recipe.

## Неудачные итерации и следующая линия

Runs 31–33 были честными engineering trials. Они fail-closed показали, какие
данные ошибочно оставались model-owned: имя существующего contract, sink и
identity bindings, затем test binding. Последовательное перенесение этих связей
в worker уменьшило raw tokens 447 692 → 376 398 → 330 111; v0.4 завершился с
374 756 tokens и корректным commit.

Теперь основной пол — 22,985 секунды cold context и 39,831 секунды Maven.
Следующие оптимизации: компактные evidence references вместо 12,3-МБ snapshot,
content-addressed reuse K2 с отдельной cold-метрикой и проверка transient
подхода на переименованных Maven/Gradle fixtures и других change families.

Машиночитаемые данные: `benchmarks/reports/maven-product-repo.json`.
