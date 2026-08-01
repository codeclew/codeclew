# Эксперимент: поиск и изменение Kotlin-кода агентом

Дата: 2026-08-01

## Постановка

Три независимых агента `gpt-5.6-terra` получили один и тот же исходный commit
`be34782d0e6045e0e1427c712fff4fdd1f11f62e` репозитория `pim-migrator` в трёх
изолированных Git worktree. Агенты не могли читать worktree друг друга.

Задача: подключить уже существующие `readAdaptiveSettingsFromEnv` и
`applyAdaptive` к CLI `Main`, чтобы документированные `ADAPTIVE_*` overrides
применялись ко всем загруженным YAML jobs, и добавить три Docker-независимых
unit-теста. Каждый агент должен был сделать один commit.

Режимы:

1. обычные инструменты Codex (`rg`, `sed`, чтение файлов);
2. `ast-index 3.50.0` как основной инструмент поиска и навигации;
3. `sthread 0.1.0` как основной инструмент семантического поиска, slicing/CFG и
   попытки структурного edit preview.

## Результаты

| Метрика | Default | ast-index | sthread |
|---|---:|---:|---:|
| Commit | `bad7a37d5c1b0292c1010d0452a7c8950ff7e93d` | `288b37607f1c95a45c56078bf296fbaa824497ba` | `a766c134083ce9c23c551819363c6a38cfbe3a6a` |
| Время до commit | 149 с | 159 с | 185 с |
| Команд поиска/чтения до первого edit | 5 | 14 | 37 |
| Изменённые файлы | 2 | 2 | 2 |
| Размер patch | +101/-1 | +79/-1 | +101/-1 |
| Targeted `cleanTest` | PASS | PASS | PASS |
| Raw total tokens | 431 909 | 583 954 | 773 304 |
| Input tokens | 427 115 | 578 039 | 765 891 |
| Cached input tokens | 381 952 | 549 632 | 709 120 |
| Output tokens | 4 794 | 5 915 | 7 413 |
| Некэшированные tokens | 49 957 | 34 322 | 64 184 |

`Некэшированные tokens = input_tokens - cached_input_tokens + output_tokens`.
`reasoning_output_tokens` не прибавляются отдельно, потому что входят в
`output_tokens`. Значения взяты из последнего накопительного `token_count`
отдельного rollout каждого subagent.

Относительно default:

- `ast-index`: +6,7% времени, +35,2% raw total tokens, но -31,3%
  некэшированных tokens;
- `sthread`: +24,2% времени, +79,1% raw total tokens и +28,5%
  некэшированных tokens.

## Проверка корректности

Во всех worktree команда

```text
./gradlew cleanTest test \
  --tests io.ladadigit.pim.migrator.engine.AdaptiveBatchSettingsTest \
  --no-daemon
```

завершилась успешно. Production patch вариантов `ast-index` и `sthread`
совпадает байт-в-байт; default отличается только расположением `map`, поведение
эквивалентно. Все варианты читают env settings один раз и применяют их ко всем
jobs перед `runAll`.

Полный `./gradlew cleanTest test --no-daemon` блокируется существующим
`BatchMigratorTest`: Testcontainers не находит Docker. Первичный отчёт default
агента ошибочно назвал обычный `./gradlew test` успешным из-за Gradle
up-to-date после filtered run; принудительный `cleanTest` воспроизвёл тот же
инфраструктурный сбой, который корректно указали два других агента.

## Наблюдения по режимам

### Default

Самый быстрый режим и минимальный raw token total для репозитория из 11
индексируемых файлов. Пять широких поисковых/read-команд оказались достаточны.
Тестовый patch больше, чем у `ast-index`, но функционально полный. Слабое место
этого прогона — неверная интерпретация закэшированного полного тестового task.

### ast-index

Cold rebuild занял менее 0,2 с и создал индекс из 11 файлов, 219 symbols и 647
references (0,31 MB). Агент обошёлся без fallback на текстовый поиск, прочитал
меньше исходных файлов и создал самый компактный test patch. Для маленького
репозитория 14 индексных запросов не окупились по wall time или raw total, но
компактные ответы дали минимальный объём некэшированных tokens.

### sthread

`project inspect` занял 15 696 мс, index — 862 мс, slice — 731 мс, CFG — 548
мс. CFG и локальная нить для `applyAdaptive` были полезными, но интеграция с
реальным проектом оказалась частичной:

- Kotlin scripting compiler plugin проекта 2.1.21 несовместим с compiler
  worker 2.4.10, поэтому анализ помечен `PARTIAL_UNSUPPORTED_FEATURE`;
- короткий FQN/name lookup extension-функции возвращал `SYMBOL_NOT_FOUND`, а
  рабочий CFG потребовал canonical SymbolId из slice;
- test source root не попал в project inspection, поэтому понадобился ручной
  просмотр тестов;
- edit preview не сформировал применимую операцию и завершился
  `INVALID_INPUT`; production edit сделан через `apply_patch`;
- после работы осталось 2,8 MB untracked `.semantic-thread` cache.

Из-за этих ограничений режим потребовал 37 команд и дал худшие wall time и
token usage в данном эксперименте.

## Вывод

Для небольшой Kotlin repo и локальной двухфайловой правки default-инструменты
эффективнее по времени и raw total tokens. `ast-index` пока не ускоряет такой
малый кейс, но уже снижает некэшированный контекст и даёт точную структурную
навигацию; ожидаемое преимущество на большом или multi-module репозитории этим
одиночным прогоном не доказано. `sthread` в текущей версии не конкурентен для
проекта с Kotlin 2.1.21: прежде чем повторять эксперимент, нужны совместимость
compiler plugins, индекс test roots, более простой symbol lookup и рабочий путь
от semantic thread к структурному edit.

Это один repo, одна задача и по одному агенту на режим. Результат показывает
направление, а не статистически устойчивый benchmark; следующий полезный шаг —
повторить набор задач на среднем multi-module Kotlin repo с ротацией агентов и
не менее чем пятью прогонами на режим.
