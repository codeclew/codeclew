# Указание: улучшить диагностику отклонения narrative

## Проблема (наблюдаемое поведение)

При `clew docs render --input narrative.json` narrative отклонялся полностью,
но CLI сообщал об этом неявно, маскируя корневую причину.

**Симптом у пользователя:** переданный narrative не применялся — в документации
оставался старый контент секций, а `inputDigest` в выводе не менялся. Прямой признак:
поле `input-0` в `updateFailures` со значением:

```json
{
  "nextAction": "documentation input violates its closed JSON/YAML schema",
  "reason": "INVALID_INPUT"
}
```

## Корневая причина

`crates/clew/src/documentation/cli.rs` в ветке `Command::Render` читает каждый input
через `store::read::<Narrative>(path, store::MAX_RECORD)`. При любой ошибке парсинга
оно записывает в `failures` **только** обобщённый `error.message` от
`store::read`, а сам narrative **не включается** в `narratives`.

Функция `store::read` (см. `crates/clew/src/documentation/store.rs:84`):

```rust
serde_yaml_ng::from_slice(&data)
    .map_err(|_| invalid("documentation input violates its closed JSON/YAML schema"))
```

Здесь детали ошибки сериализации (`serde_yaml_ng` Error) отбрасываются через `map_err(|_| ...)`.
В итоге пользователь видит общее «violates its closed JSON/YAML schema», но НЕ узнаёт:

1. **Какое поле неизвестно/лишнее** — в `struct Operation` и `struct Narrative`
   (`crates/clew/src/documentation/model.rs`) стоит `deny_unknown_fields`, поэтому
   любое лишнее поле (например, `interaction`, `operationId`) валит парсинг всего narrative.
2. **Какая операция (index/id) проблемная** — ошибка не локализуется.
3. **Что narrative отклонён целиком**, а render молча переиспользует предыдущий
   (retained/pinned) narrative.

### Конкретный воспроизводимый случай

Добавлены поля `"operationId"` и `"interaction"` в operation narrative 1.3.
Они отсутствуют в `struct Operation` (`model.rs`) — структура имеет
`#[serde(deny_unknown_fields)]`. Итог: **весь narrative** отклоняется, хотя
проблема — только в 2 лишних полях 3 операций.

## Что нужно исправить

### 1. Сохранять и показывать исходную ошибку сериализации

В `crates/clew/src/documentation/store.rs`, функция `read`:

```rust
serde_yaml_ng::from_slice(&data)
    .map_err(|_| invalid("documentation input violates its closed JSON/YAML schema"))
```

→ вернуть подробную причину (см. `serde_yaml_ng::Error` / `serde_json`):
- невалидные/неизвестные поля (`unknown field`), с именем поля;
- номер операции/строки, где возникла ошибка, если доступен.

Минимум — включить `error.to_string()` из сериализатора в текст `invalid(...)`,
не отбрасывая через `map_err(|_| ...)`.

### 2. Локализовать проблему на уровне операции

`deny_unknown_fields` на `Narrative`/`Operation` — «всё или ничего». Это допустимо,
но при отклонении всего narrative нужно указать, какая именно операция (по `id`
и/или индексу в `operations`) нарушает схему. Рассмотреть:
- отдельный проход валидации операций с человекочитаемым сообщением вида
  `operations[5] (id="...") : unknown field "interaction"`;
- либо завести отдельный diagnostic-блок в `updateFailures`, а не только
  общий `input-0`.

### 3. Явно предупреждать о «подмене» narrative

Когда часть/весь incoming narrative отклонена, а публикация продолжается на
предыдущем (retained/pinned) narrative — выводить явное предупреждение, что
входящий narrative не применён. Сейчас это определяется только косвенно по
стабильному `inputDigest` + старым секциям, что вводит в заблуждение.

## Где смотреть

- `crates/clew/src/documentation/cli.rs` — ветка `Command::Render` (сбор `failures`,
  `input-{index}`).
- `crates/clew/src/documentation/store.rs` — `read` (маскирование ошибки парсинга).
- `crates/clew/src/documentation/model.rs` — `struct Narrative`, `struct Operation`
  (поля, `deny_unknown_fields`).
- `crates/clew/src/documentation/render.rs` — `publish_language_with_mode` /
  `publish_internal` (как `failures` влияет на выбор narrative для публикации).

## Критерий приёмки

- `clew docs render --input narrative.json` с narrative, содержащим лишнее поле
  (`interaction`, `operationId`, и т.п.), сообщает **имя поля** и **id/индекс операции**.
- CLI явно предупреждает, если входящий narrative отклонён и вместо него
  публикуется предыдущий.
- Если narrative валиден — поведение не меняется (регрессий нет).