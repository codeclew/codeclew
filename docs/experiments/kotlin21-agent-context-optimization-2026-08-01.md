# Kotlin 2.1 и компактный контекст агента

Дата: 2026-08-01

Это продолжение эксперимента на `pim-migrator` с Kotlin 2.1.21. Цель — убрать
конкретные причины проигрыша режима `sthread`: несовместимый compiler worker,
нерабочий lookup extension-функции и необходимость вручную искать тесты.

## Изменения

- добавлен отдельный version-pinned worker Kotlin 2.1.21;
- Gradle project model теперь сообщает фактическую версию compiler plugin;
- Rust-клиент автоматически выбирает worker 2.1.21 или 2.4.10 до K2-анализа;
- обычные `applyAdaptive` и `io.private-product.pim.migrator.engine.applyAdaptive`
  разрешают extension-функцию; неоднозначный short name возвращает ошибку;
- команда `sthread context` за один запуск строит индекс и semantic slice,
  выдаёт компактный edit-ready JSON и перечисляет тесты с текстовой ссылкой на
  seed.

## Smoke-замер на `pim-migrator`

Команда:

```text
sthread context --repo <pim-migrator> --symbol applyAdaptive --max-nodes 40
```

получила `COMPLETE_SUPPORTED_SUBSET` на Kotlin 2.1.21 и нашла
`AdaptiveBatchSettingsTest.kt`. Измеренный cold run без `.semantic-thread`
занял 8,42 с (`request_completed.durationMs = 8418`), warm run — 2,48 с
(`request_completed.durationMs = 2058`). После удаления повторяющихся source и
anchor полей ответ занял 6 033 байта против 16 740 байт полного `slice` для того
же seed: на 64,0% меньше байт. Это размер
tool output, а не полный token usage rollout; корректное сравнение agent tokens
требует повторного независимого Terra-прогона.

Отдельный fixture с serialization compiler plugin подтверждает, что K2/FIR
анализ Kotlin 2.1.21 проходит без прежнего `IncompatibleClassChangeError`.

## Повторный Terra-прогон

Независимый `gpt-5.6-terra` повторил исходную задачу на commit `be34782d` и
создал commit `0e5dd5b`. Корректность улучшилась, но общая эффективность — нет:

| Метрика | sthread v1 | sthread v2 | Изменение |
|---|---:|---:|---:|
| Время до commit | 185 с | 185 с | 0% |
| Команд до первого edit | 37 | 20 | -45,9% |
| Tool roundtrips | 19 | 29 | +52,6% |
| Raw total tokens | 773 304 | 1 276 799 | +65,1% |
| Output tokens | 7 413 | 8 670 | +17,0% |
| Некэшированные tokens | 64 184 | 76 159 | +18,7% |

Модель, reasoning effort, client version и задача совпадали. V2 устранил
несовместимость Kotlin compiler plugin и сократил число shell-команд, но агент
сделал больше отдельных tool turns. Два больших `context` ответа достигли
40-КБ лимита stdout каждый, а шесть запросов ушли на неуспешный lookup классов,
synthetic `MainKt` и угаданных FQN. Поэтому локальное уменьшение ответа для
`applyAdaptive` не перешло в экономию полного rollout.

Следующая оптимизация должна быть направлена не на FIR/K2, а на agent protocol:
summary-only stdout при наличии `--output`, единый discovery/search для
функций и классов, компактное представление больших slices и объединение
нескольких lookup в один roundtrip.
