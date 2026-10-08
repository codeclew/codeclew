# Указание: поддержка OpenAPI 3.1.x в контрактах сервисов

## Проблема (наблюдаемое поведение)

Сервис объявляет контракт `src/main/resources/openapi/openapi.yml` с `openapi: 3.1.2`.
При `docs render`/`docs check` контракт не интерпретируется:

- в статусе сервиса виден boundary `UNSUPPORTED_CONTRACT_VERSION:src/main/resources/openapi/openapi.yml`;
- операции `CONTRACT_OPERATION` не создаются, `contract-scope` (CONTRACT_SCOPE) остаётся без контрактов;
- render отдаёт `contracts:[]` и «Операции, объявленные в OpenAPI · 0», хотя операции в контракте есть.

Пользователь при этом не видит причину — контракт «молча» игнорируется.

## Корневая причина

`crates/clew/src/documentation/contracts.rs` жёстко ограничивает поддержку версий:

```rust
pub const TESTED_VERSIONS: &[&str] = &["3.0.0", "3.0.3"];
```

В `contracts.rs`:
- `capture`/`import` (строка ~145-160): версия из `document["openapi"]` сверяется с `TESTED_VERSIONS`;
  при отсутствии совпадения статус = `UNSUPPORTED_CONTRACT_VERSION`, контракт **не добавляется** в `evidence.contracts`;
- `enrich` (строка ~299-306): `if !document["openapi"].as_str().is_some_and(|v| TESTED_VERSIONS.contains(&v)) { continue; }` —
  операции `CONTRACT_OPERATION` для неподдерживаемых версий не генерируются.

В `crates/clew/src/documentation/modules.rs:228` список версий берётся из того же `TESTED_VERSIONS`:

```rust
openapi["testedVersions"] = json!(super::contracts::TESTED_VERSIONS);
```

Поэтому достаточно обновить один источник.

## Реальный контракт, который нужно поддержать

`motor-deal-service`: `openapi: 3.1.2`, 5343 строки, структура стандартная
(`info`, `servers`, `security`, `tags`, `paths`, `components`). 3.1-специфичные
признаки (`webhooks`, `type: null`, `unevaluatedProperties`, `patternProperties`)
**не используются** — контракт совместим с 3.0.3, отличается только строкой версии.
`examples:` присутствует, но он валиден и в 3.0.3.

## Что нужно исправить

### Минимально (добавить конкретную версию)

В `crates/clew/src/documentation/contracts.rs:17`:

```rust
pub const TESTED_VERSIONS: &[&str] = &["3.0.0", "3.0.3", "3.1.0", "3.1.1", "3.1.2"];
```

### Правильнее (поддержка семейства 3.1.x)

Вместо точечного списка — совместимость по мажор/минор, например добавить
вспомогательную проверку `supported_openapi_version(&str)`, которая допускает
`3.0.x` и `3.1.x` (а не только тестовые 3.0.0/3.0.3), и использовать её в обоих
местах (`capture`/`import` и `enrich`). Это уберёт хрупкость, когда каждый патч
3.1 нужно заносить в список.

Важно сохранить поведение для 3.1-специфичных фич:
- если контракт использует `webhooks`, `type: null`, `2020-12` и т.п. — он структурно
  отличается от 3.0; такие случаи должны по-прежнему оставаться в явном ограничении,
  а не молча игнорироваться;
- `Resolver` (`contracts.rs`) уже резолвит `$ref`/фрагменты — 3.1-совместимый парсинг
  путей/параметров/security не требует новой логики для описанного выше контракта.

## Диагностика (полезно добавить в движок)

Пользователю не видно, почему контракт игнорируется. Рекомендуется:
- в `render`/`check` выводе, когда контракт отклонён из-за версии, указывать явно
  `UNSUPPORTED_CONTRACT_VERSION:<path> (openapi=<ver>; supported=3.0.x, 3.1.x)`;
- не прятать этот boundary в общий статус, а показывать как причину `contracts:[]`.

## Критерий приёмки

- `motor-deal-service` с `openapi: 3.1.2` захватывается: в `evidence.contracts`
  появляется контракт, `enrich` создаёт `CONTRACT_OPERATION` для каждого `paths.*.*`;
- render отдаёт ненулевые операции OpenAPI-контракта (исчезает «Операции, объявленные в OpenAPI · 0»);
- контракты 3.0.x не регрессируют;
- контракты с 3.1-специфичными фичами (если появятся) остаются явно ограниченными, а не молча пропадают.