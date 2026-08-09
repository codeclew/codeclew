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
3. `clew 0.1.0` как основной инструмент семантического поиска, slicing/CFG и
   попытки структурного edit preview.

После добавления Kotlin 2.1 worker и команды `context` четвёртый независимый
Terra-агент повторил режим `clew` на том же baseline как `clew v2`.

## Результаты

| Метрика | Default | ast-index | clew v1 | clew v2 |
|---|---:|---:|---:|---:|
| Commit | `bad7a37d` | `288b376` | `a766c13` | `0e5dd5b` |
| Время до commit | 149 с | 159 с | 185 с | 185 с |
| Команд поиска/чтения до первого edit | 5 | 14 | 37 | 20 |
| Tool roundtrips | — | — | 19 | 29 |
| Изменённые файлы | 2 | 2 | 2 | 2 |
| Размер patch | +101/-1 | +79/-1 | +101/-1 | +96/-1 |
| Targeted `cleanTest` | PASS | PASS | PASS | PASS |
| Raw total tokens | 431 909 | 583 954 | 773 304 | 1 276 799 |
| Input tokens | 427 115 | 578 039 | 765 891 | 1 268 129 |
| Cached input tokens | 381 952 | 549 632 | 709 120 | 1 200 640 |
| Output tokens | 4 794 | 5 915 | 7 413 | 8 670 |
| Некэшированные tokens | 49 957 | 34 322 | 64 184 | 76 159 |

`Некэшированные tokens = input_tokens - cached_input_tokens + output_tokens`.
`reasoning_output_tokens` не прибавляются отдельно, потому что входят в
`output_tokens`. Значения взяты из последнего накопительного `token_count`
отдельного rollout каждого subagent.

Относительно default:

- `ast-index`: +6,7% времени, +35,2% raw total tokens, но -31,3%
  некэшированных tokens;
- `clew`: +24,2% времени, +79,1% raw total tokens и +28,5%
  некэшированных tokens.
- `clew v2`: +24,2% времени, +195,6% raw total tokens и +52,5%
  некэшированных tokens.

## Проверка корректности

Во всех worktree команда

```text
./gradlew cleanTest test \
  --tests io.private-product.pim.migrator.engine.AdaptiveBatchSettingsTest \
  --no-daemon
```

завершилась успешно. Production patch вариантов `ast-index` и `clew`
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

### clew

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

### clew v2 после Kotlin 2.1 worker и `context`

Повторный независимый `gpt-5.6-terra` прогон выполнен на том же исходном
commit, с теми же model/effort (`gpt-5.6-terra`, medium), client version и
задачей. Kotlin 2.1.21 теперь анализируется корректно:

- `context applyAdaptive` завершился с `COMPLETE_SUPPORTED_SUBSET` за 8 374 мс;
- short-name lookup extension-функции сработал без canonical SymbolId;
- production patch и три Docker-независимых теста корректны;
- targeted `cleanTest` прошёл, полный suite остановился только на прежнем
  Testcontainers/Docker ограничении.

Число команд до первого edit снизилось с 37 до 20 (-45,9%), но wall time не
изменился: 185 секунд. Token usage вырос: raw total на 65,1%, некэшированные
tokens на 18,7%, output tokens на 17,0% относительно clew v1.

Трасса объясняет расхождение: v2 сделал 29 отдельных tool roundtrips против 19
у v1. Из 11 содержательных Clew-запросов шесть были неуспешными попытками
найти классы, synthetic `MainKt` или неверно угаданный FQN. Кроме того,
`context` для `main` и `loadJobsFromYaml` вернул в stdout по 40 КБ (ответы были
обрезаны tool limit), хотя одновременно записывал полный JSON через `--output`.
Совокупный размер tool outputs вырос лишь на 4,9%, но дополнительные model/tool
turns многократно переиспользовали большой cached context.

## Вывод

Для небольшой Kotlin repo и локальной двухфайловой правки default-инструменты
остаются эффективнее по времени и raw total tokens. `ast-index` пока не ускоряет
такой малый кейс, но уже снижает некэшированный контекст и даёт точную
структурную навигацию; ожидаемое преимущество на большом или multi-module
репозитории этим одиночным прогоном не доказано. `clew v2` устранил блокеры
Kotlin 2.1 и short-name lookup, но в этом прогоне всё ещё не конкурентен по
времени и tokens из-за лишних roundtrips и больших ответов.

Это один repo, одна задача и по одному агенту на режим. Результат показывает
направление, а не статистически устойчивый benchmark; следующий полезный шаг —
повторить набор задач на среднем multi-module Kotlin repo с ротацией агентов и
не менее чем пятью прогонами на режим. Для следующего Clew worker сначала
нужны единый discovery endpoint для функций и классов, summary-only stdout при
`--output`, дедупликация перекрывающихся `sourceText` в больших slices и меньше
отдельных agent/tool roundtrips.
