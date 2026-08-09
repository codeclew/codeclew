# Универсальная модель task surface

Дата: 2026-08-02

## Результат проектирования

Clew планирует прикладное изменение не от одного «лучшего» символа, а от
обязательств задачи:

```text
TaskSurface = EntryPoints ∪ ExactTargets ∪ BoundedGraphClosure ∪ Contracts ∪ Tests
```

`EntryPoints`, точно названные declarations и запрошенная test surface являются
обязательными. Лимит `max-roots` ограничивает только дорогое K2-расширение
графа и не может вытеснить явно названную, но пока не связанную функцию.
Необязательный source графовой подсказки удаляется первым при нехватке stdout
budget; усечение обязательного source всегда переводит результат в
`PARTIAL_BUDGET`. Поэтому `COMPLETE_TASK` означает полноту поверхности
изменения, а не просто успешное завершение поиска.

Worker и skill не содержат имён репозиториев, declarations, полей или рецептов.
File alias, exact declaration, evidence term и запрос тестов распознаются общими
правилами. Полный индекс и K2 evidence остаются локальными; модель видит только
bounded projection и короткие стабильные anchors.

## Почему это должно экономить время и tokens

Пусть прикладная задача требует `k` последовательных решений «что читать
дальше», а суммарный показанный source равен `S`. В default workflow каждое
решение требует отдельного model/tool turn. В ast-index workflow индекс
сокращает один ответ, но модель всё ещё формулирует `q` запросов и отдельно
читает найденный source:

```text
default:   k model turns + k searches/reads, model-visible source grows with S
ast-index: q model turns + q index answers + r source reads
Clew:   1 context turn + 1 compact plan + 1 atomic apply, context <= B
```

Raw input tokens в агентном rollout повторно включают накопленный transcript.
Поэтому несколько малых ответов стоят не только сумму их размеров: каждый
следующий reasoning turn снова несёт предыдущие tool outputs. При фиксированном
`B` model-visible navigation у Clew ограничена сверху, а число discovery
turns постоянно для bounded task. Полный evidence используется worker локально
и не возвращается модели.

Для wall time важна не скорость literal search. Один `rg` почти всегда быстрее
cold semantic index. Выигрыш возникает, когда 7–23 секунд локальной компиляции
заменяют несколько model/tool round trips, ручное восстановление type flow,
создание большого patch и повторные build/fix циклы. Atomic apply также
объединяет materialization, targeted validation, commit и synchronization.

Следовательно, ожидаемое преимущество относится к большинству прикладных
задач, где затронуты минимум две из следующих границ: entrypoint, helper,
service/repository edge, typed contract, persistence projection и regression
test. Оно не обещается для тривиальной правки в заранее известной строке одного
файла, чисто текстовой массовой замены или задачи, чьё время целиком определяется
долгим build/deploy вне агента.

## Проверяемые предсказания

Универсальная архитектура считается подтверждённой, только если одновременно
выполняются следующие предсказания:

1. Exact target остаётся в context, даже если текущий call graph ещё не связан
   с ним.
2. Названный файл с `main` задаёт `WORKFLOW`; одноимённые `main` из других
   файлов не создают ложную полноту.
3. Required source никогда не урезается при `COMPLETE_TASK`; optional graph
   source может быть опущен с сохранением дуг и контрактов.
4. Один и тот же protocol работает на Gradle/Kotlin 2.1 и Maven/Kotlin 2.3 без
   build-specific model workflow.
5. Агент решает задачу без grep и чтения source вне одного bounded context,
   одним validated plan и одним atomic apply.
6. На задачах wiring/configuration и typed cross-layer propagation Clew
   уменьшает model/tool turns, wall time и raw/noncached tokens относительно
   зафиксированных default и ast-index baselines.

## Текущая эмпирика

На `pim-migrator` прежний requirements-blind selector давал ложный
`COMPLETE_TASK`: показывал две helper-функции, но не нужный `Main`, либо при
task intent выдавал `PARTIAL_BUDGET`. После разделения обязательной поверхности
пять fresh cold contexts дали 5/5 `COMPLETE_TASK`, одинаковые 11 516 bytes,
7 334–8 206 ms (среднее 7 563 ms). Каждый содержит `main`, обе disconnected
exact declarations, две execution edges, контракты `MigrationJob` и
`AdaptiveBatchSettings`, а также test template; все требования task audit
помечены как satisfied.

Историческая независимая серия на той же задаче показала против default:
34,8 против 59,1 секунды до edit, 56,6 против 149 секунд до commit, 179 636
против 431 909 raw tokens и 21 479 против 49 957 noncached tokens. Новая серия
с requirements-driven selector получила чистый commit: один context, две
попытки plan validator, один atomic apply и ни одного grep/source fallback.
Сумма локального tool wall составила 34,0 секунды (10,3 context + 23,7 apply),
но это **не end-to-end**: timestamps артефактов показывают ещё около 86 секунд
между context и валидным plan. Полный интервал от создания run directory до
receipt — примерно 138 секунд, то есть лишь немного лучше default 149 секунд и
не подтверждает существенную победу. Отдельный неудачный прогон выявил общий
класс ошибки с полностью квалифицированным вызовом Kotlin extension; worker
теперь локально канонизирует его в receiver syntax и синтезирует import до
компиляции.

На Maven `product-repo` transient task graph ранее дал 50 секунд до edit,
97 секунд до commit, 11 сравнимых calls, 374 756 raw и 37 348 noncached tokens
с независимым `ACCEPT` на 109 тестах. Ast-index потребовал 63/171 секунду,
21 call, 1 099 997 raw и 72 925 noncached tokens и был отклонён hidden tests.
Этот кейс проверяет другое семейство: типизированное изменение через data
source, intermediary, output contract и две workflow-ветки. Контроль текущего
selector на fresh checkout вернул `COMPLETE_TASK` размером 15 181 bytes,
сохранил все четыре роли, один `ProductCanonicalDto` contract и доступный
transient transform.

Новые orchestration-запуски не предоставляют сопоставимой cumulative token
telemetry, поэтому token-вывод опирается только на сохранённые Terra-метрики
тех же двух задач; размер нового model-visible context и число вызовов указаны
отдельно. Это ограничение не подменяется оценкой tokens из bytes.

Главный оставшийся bottleneck — уже не discovery, а model-authored low-level
plan: в зачётном новом прогоне он занял 2 985 bytes и две попытки validator.
Следующая линия имеет смысл только как общий semantic-goal compiler, который
выводит anchors, substitutions и import normalization локально из текущего
evidence. Дальнейшая настройка ранжирования surface этого bottleneck не решает.

## Критерий дальнейшей оптимизации

Если новый end-to-end проигрывает при полном context, сначала измеряются:

- model turns до plan;
- bytes обязательной и optional surface;
- validator attempts и размер model-authored plan;
- apply time отдельно от build time.

Transient graph добавляется только для повторяющейся структурной формы, где
модель всё ещё вручную переносит bindings через три и более роли. Он выводится
из evidence текущего запуска. Repository-specific автоматизация остаётся в
версируемом repository recipe и не может попасть в глобальный worker или skill.

## Итог цели: остановлена как недоказанная

Requirements-driven task surface, bounded projection и atomic apply являются
полезным и протестированным результатом. Однако заявленная существенная
end-to-end победа для большинства прикладных задач не доказана:

- Maven typed-propagation case выигрывает у default и ast-index по времени,
  calls и сохранённым Terra tokens, но это одна структурная форма;
- Gradle wiring/test case стабилен по context и zero-grep, однако единственный
  успешный новый transaction потребовал около 138 секунд end-to-end против
  default 149 секунд;
- новый минимальный plan был `VALID` с первой попытки менее чем за 51 секунду
  от старта, но benchmark invalid: apply получил SHA вместо branch ref и не
  создал candidate; следующий независимый run ошибся в compilation ID и также
  остановился до plan. Эти прогоны исключены, а не достроены вручную;
- сопоставимой новой cumulative token telemetry нет.

Независимый архитектурный анализ подтвердил, что PIM не использует
`PROPAGATE_TYPED_FIELDS`: его bottleneck — model-authored test oracle и
низкоуровневый plan. Возможный новый `WIRE_TYPED_DECORATOR` может быть
fail-closed и малым, но покрывает только узкую нормальную форму и без корпуса
не даёт основания говорить о «большинстве» задач.

## Постановка для deep research

> На заранее зафиксированном корпусе не менее 30 изолированных Kotlin Gradle и
> Maven задач определить семейства структурных изменений, для которых текущий
> semantic evidence позволяет безопасно компилировать короткий model-owned goal
> в полный EditIR. Для кандидата `WIRE_TYPED_DECORATOR` формализовать и проверить
> K2/AST-инварианты: единственные `config(): C` и `T.decorate(C): T`, единственный
> `List<T>` source/consumer path, evaluation order, nullability, collection kind,
> overload/import ambiguity, coroutine и transaction boundaries. Отдельно
> измерить, когда test oracle выводим из pure data-class decorator laws, а когда
> обязан оставаться model-owned. До раскрытия результатов зафиксировать пороги
> coverage, false-positive/false-negative, hidden acceptance, complete-to-commit
> time, model turns, raw/noncached tokens и сравнить с default и ast-index.

К включению в универсальный worker допускается только transform, который на
withheld части корпуса проходит correctness gates и даёт заранее заданное
существенное сокращение end-to-end и tokens. До этого нельзя обобщать победу
Maven-кейса на большинство Kotlin/Spring задач.
