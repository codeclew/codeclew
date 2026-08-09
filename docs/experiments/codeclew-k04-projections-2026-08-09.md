# K04 — bounded L0–L5 projections

Дата: 9 августа 2026 года

Базовый revision: `99141ea`

Итог ветви: `SUCCESS + NARROW_PROJECTIONS`

Независимый verdict: `ACCEPT`

## Человеко-читаемый вывод

Codeclew теперь умеет без filesystem search построить из Kotlin 2.1 Thread IR
ограниченную проекцию выбранного уровня L0–L5. Upper-level факт принимается
только при наличии точного нисходящего пути через каждый соседний уровень до
L0. Каждый L0 содержит file/range, непустой source text и совпадающий SHA-256;
результат привязан к composite snapshot и полностью воспроизводимому query.

Vertical kind нельзя назначить произвольно: adapter принимает только exact
compiler/IR facts для control, data, journey, state, effect, failure, config,
test-evidence и change. Неизвестный kind, decoy, пропущенный anchor, shortcut
между уровнями, другой kind evidence или неполный provenance дают отказ.

`maxNodes` и `maxBytes` ограничивают фактически печатаемый pretty JSON вместе с
newline. Если даже error envelope не помещается, CLI возвращает ненулевой exit
и пустой stdout. Усечение имеет snapshot/query-bound expansion handle.

## Исполняемые доказательства

- `cargo test -p clew projection --lib`: 19/19.
- `cargo test -p clew --test projection_cli`: 2/2 на Kotlin 2.1.21.
- `cargo test --workspace --all-targets`: зелёный полный regression до последних
  узких fail-closed поправок; после них повторены затронутые suites и compile.
- Реальный L5 запрос `com.acme.applyAdaptive`: `COMPLETE`, 1 node, 1 evidence
  path длиной 6, 2203 bytes из 32768, 1268 ms в одиночном локальном запуске.
- Тот же запрос с `maxBytes=1`: ненулевой exit, stdout 0 bytes.
- 10× disconnected irrelevant padding не меняет projection fingerprint.

Пять последовательных независимых adversarial проверок дали четыре `REJECT` и
финальный `ACCEPT`. Отказы обнаружили и заставили устранить: переставленные
уровни, caller-selected kinds, compact-vs-stdout budget gap, неполный replay,
direct L5→L0 shortcut, decoy predicates, unanchored/token-only facts, substring
test classification и L0 без проверяемого source text.

## Граница доказанного

Это не доказывает, что контекст достаточен для большинства задач, что H03/H04
истинны или что Codeclew быстрее default/ast-indexer. Поддержанный contour
намеренно fail-closed: все Thread IR nodes должны иметь exact source anchors.
Applicability, token/time reduction и task completeness проверяются следующими
узлами корпуса, binder/materialization и paired benchmark.
