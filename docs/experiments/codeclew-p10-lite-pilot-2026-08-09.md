# Codeclew P10-lite: pilot

Дата: 9 августа 2026 года

Статус: `REJECTED_SEMANTIC_AND_BUDGET`

Base plan:
`80f2b7308c0e4eb51c6376931591dc389d0c08e6d7dc75a4ab757b7395506a34`

Amendment:
`docs/superpowers/plans/2026-08-09-codeclew-p10-lite-amendment.md`

## Результат

P10-lite корректно сузил ownership: он больше не заявляет packet/receipt,
telemetry, retry или GB runtime correctness и возвращает
`runtimeContractsAccepted:false`.

Внутренний self-test проходит `11/11`. Свежий независимый verifier использовал
`5` calls и вернул semantic `REJECT` по двум обходам:

1. `HIGH`: NORMAL current-task event допускает пустые `taskId` и `messageId`,
   после чего открывается `A10->B01`.
2. `MEDIUM`: approval фиксирует exact role set, но не exact `role -> path` и
   nested artifact keys. Path alias и лишнее nested field проходят.

Подтверждены base/amendment/report/prototype digests, controller self-binding,
six-role set, TEST_ONLY boundary, deferred ownership и exit `0/2/3/64`.

## Стоимость

До записи отчёта:

```text
root charged calls       12
independent verifier      5
all-team pilot calls     17
candidate ceiling        12
```

Следовательно, cost verdict всего pilot: `FAIL`. Verifier-side gate сам по
себе эффективен: `5` calls и около `2.18 s` subprocess wall.

Native input/cached/output/noncached telemetry недоступна; bytes не
использовались как замена.

## Вывод гейта

Gate verdict: `KEEP` для независимого verifier, `SIMPLIFY` для P10-lite
materialization.

Сравнение с full P10:

```text
full P10 pre-report calls  106
P10-lite pre-report calls   17
reduction                   84%
```

P10-lite существенно ближе к правильной границе ответственности, но текущая
версия не открывает A10/B01.

## Предлагаемая final delta

Отдельный preregistered `P10-lite-v2` должен менять только три инварианта:

1. `taskId` и `messageId` — непустые strings.
2. Каждый artifact имеет exact keys `role/path/rawFileSha256`.
3. Полный canonical set `role+path` равен manifest mapping; self-test содержит
   empty-event, path-alias и nested-extra-field mutations.

Никакие другие обязанности, budgets или edges не меняются. Рекомендуемый
ceiling final delta: `8` all-team calls, установлен до запуска. При превышении
или semantic REJECT amendment прекращается и цель получает формальный
process-overhead falsifier.

Эта final delta не запускалась: она требует human approval изменения
исполняемого контракта и нового ceiling.
