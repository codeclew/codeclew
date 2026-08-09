# Верификация оптимизированного базового этапа Codeclew

Дата: 9 августа 2026 года

Проверяемый документ:
`docs/superpowers/plans/2026-08-09-codeclew-optimized-research-foundation-plan.md`

Итоговый SHA-256:
`80f2b7308c0e4eb51c6376931591dc389d0c08e6d7dc75a4ab757b7395506a34`

Итоговый независимый вердикт: `ACCEPT`

## 1. Объём проверки

Независимый агент проверял не стилистику, а исполнимость successor-plan и
сохранность доказательного содержания принятого R01:

- корректность carry-forward и полной old-to-new migration equivalence;
- единственность canonical source of truth;
- точность outcome/branch/edge matrix;
- разделение shared и node-specific budget digests;
- достижимость B03 probes без решений, изобретаемых после approval;
- stop-loss, retry, advisory metrics и GF0 semantics;
- различение raw receipt SHA и canonical receipt digest scope;
- арифметику execution-only и all-in budgets.

Historical refs проверены локально:

```text
old plan  83933d98913af3c4b016f674f73b76af3cfe4db190e30294ebb469d6d6cd6f93
R01 file  7c3af3ec0d2390727502c38ef9ea9733c7e42f2d35c056940b5c2444f18bfe46
GF report 6bcfdb3dd50d502b9326a835b3fbfd85252e3cd76a7200dceb55fae928011be3
```

## 2. Найденные дефекты и исправления

Первый полный независимый аудит вернул `REPAIR_REQUIRED` и обнаружил пять
классов дефектов:

1. GB ошибочно мог требовать равенства разных node budget digests.
2. B01 semantic sample не доказывал сохранность всех старых edges, evidence
   classes, provenance и falsifiers.
3. Не были зафиксированы exact schemas/controller paths и B03 probes.
4. Stop-loss, gap-closure, GF/GF0, retry denominator и advisory metrics имели
   неоднозначные формулировки.
5. Receipt digest scope был описан недостаточно точно.

Исправления:

- GB сравнивает общий `plan/source/model/topology/budget-policy` digest, а
  каждый `budgetDigest` — только со строкой своего узла;
- B01 обязан построить полный migration manifest, а пять risk records являются
  дополнительным semantic audit, не выборочной заменой equivalence proof;
- P10 задаёт exact schemas, paths, controller command и exit semantics;
- outcome/branch combinations развернуты в отдельные exact строки;
- B03 имеет frozen fixture/test paths, отдельные команды и accept predicates;
- half-budget stop-loss применяется только к B01–B03;
- GF0 является компактным current-run terminal decision и лишь ссылается на
  исторический GF;
- raw file SHA и canonical digest явно разделены областью digest scope.

## 3. Delta-only повторная проверка

Полный аудит не повторялся. Агент получал только изменённые invariants.

| Раунд | Charged tool calls | Результат | Новая фактура |
| --- | ---: | --- | --- |
| Full audit | `16` | `REPAIR_REQUIRED` | Пять классов блокирующих дефектов |
| Delta 1 | `4` | `REJECT` | Неверный B01 path, неполные B03 commands, две wording ambiguities |
| Delta 2 | `3` | `REJECT` | Idempotency ошибочно приписана не тому concurrency test |
| Delta 3 | `1` | `ACCEPT` | Оба exact tests и predicates согласованы |

Всего независимая проверка использовала `24` calls. После первого аудита
повторные проверки использовали `8` calls вместо повторения traversal. Каждая
из них либо нашла новый исполнимый дефект, либо закрыла конкретный changed
invariant; церемониальных раундов без evidence delta не было.

## 4. Проверка бюджетов

Механически пересчитаны заявленные пределы:

```text
execution-only: 140k noncached / 26k output / 100 calls / 85 min critical path
P10 + first run: 160k noncached / 30k output / 115 calls / 105 min
vs old 400k / 190 calls:
  execution-only reduction = 65% tokens / 47% calls
  all-in reduction         = 60% tokens / 39% calls
  B01 call reduction       = 72%
```

Native token telemetry для этой planning session не экспортирована, поэтому
фактические tokens проверки имеют значение `UNAVAILABLE`; bytes как замена не
использовались.

## 5. Итог и граница вердикта

`ACCEPT` означает:

- документ внутренне непротиворечив;
- он сохраняет принятую фактуру R01 без повторного полного исследования;
- оркестратор может материализовать P10 без изобретения branch semantics или
  B03 fixtures;
- базовый этап имеет проверяемую причину быть дешевле старого foundation path.

`ACCEPT` не является approval на выполнение B01 и не утверждает, что
гипотезы OF1–OF6 уже доказаны. Следующий разрешённый шаг — материализовать и
независимо негативно проверить P10. После этого пользователь одобряет exact
plan/sidecar digests в A10.
