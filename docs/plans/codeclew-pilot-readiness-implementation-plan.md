# Codeclew — план достижения pilot-ready состояния

## Метаданные

- **Источник продуктовой правды:** [README.md](../../README.md), зелёный usable
  baseline `44e82496da759518b539b70b944633a1f6965cc6`,
  [codeclew-usable-first-plan.md](codeclew-usable-first-plan.md).
- **Канон реализации:** `crates/clew/src/main.rs`,
  `bootstrap/clew_bootstrap.py`, `bootstrap/runtime_components.json`,
  `scripts/usability-smoke.py`, `.github/workflows/ci.yml`.
- **Approval source:** пользователь 2026-08-23 явно попросил подробный план,
  независимую проверку и немедленное последовательное исполнение в режиме цели.
- **Оркестратор:** выполняет первую задачу со `Status: - [ ]`, проверяет DoD,
  меняет статус на `- [x]` и только затем переходит дальше.
- **Параллелизм:** реализация последовательна. Независимый review плана и
  финальный review выполняются отдельным агентом с чистой ролью проверяющего.
- **Stop-loss:** разрешены ровно три локальных product E2E invocation: facade
  smoke в T00, трёхкейсный pilot в T02 и финальный `ci-verify` в T05. Каждый
  invocation может сделать один cold prime собственного private state; ни один
  не повторяется ради диагностики. До T05 выполняются только targeted checks.
  После push разрешён ровно один GitHub RELEASE run; его падение оставляет цель
  незавершённой и не разрешает второй push/run в рамках этого плана.

## Результат milestone

Codeclew пригоден для ограниченного командного пилота на Kotlin 2.4/Gradle:

1. Агент использует публичный двухфазный `change`-интерфейс вместо ручной
   оркестрации `session/context/plan/task-run`.
2. Production capsule содержит только доказанный K24 worker; preview workers не
   увеличивают cold start и не могут случайно стать runtime prerequisite.
3. Один pilot runner выполняет три разных изменения на чистых репозиториях,
   переиспользуя один runtime, и выдаёт обезличенный агрегат.
4. PR CI проверяет узкие контракты и один product smoke; scheduled/manual
   qualification проверяет трёхкейсный pilot и строгий warm audit.
5. README честно называет границу pilot-ready и следующий release milestone.

Это не general-availability release. Подписанные prebuilt capsule, Maven,
Android/KMP, K21/K23, multi-compilation performance и внешний agent benchmark
не входят в milestone.

## Поддержанный публичный flow

```text
./clew change open ...              -> sessionId + contextId + bounded context
agent создаёт immutable plan
./clew change prepare ...           -> planId + run
./clew change status --run ...      -> bounded candidate/status
./clew change publish ...           -> explicit publication
./clew change recover ...           -> exceptional recovery
```

Низкоуровневые команды не удаляются: это внутренний протокол и диагностический
escape hatch. Facade не создаёт новую change-базу, locator или schema; `sessionId`,
`contextId`, `planId` и `runId` остаются единственной authority.

## Общие команды проверки

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --locked -p clew --bin clew 'tests::' -- --test-threads=1
python3 -I -S bootstrap/test_clew_bootstrap.py
python3 -I -S scripts/check_repository_privacy.py --pre-commit
```

## Известные пробелы покрытия

- Нет подписанного portable installer/prebuilt capsule; решение принимается по
  данным пилота, а не проектируется заранее.
- Pilot использует три контролируемых K24 Gradle изменения. Он проверяет
  transaction/runtime, но не качество произвольной генерации plan внешним LLM.
- Нет Maven, Android/KMP, K21/K23 и multi-compilation product claims.
- Нет pen-test и формальной верификации ledger/CAS.
- macOS/Linux scheduled qualification подтверждает поддержанные платформы, но
  не все JDK vendors и filesystem/network configurations.

## T00. Двухфазный публичный `change` vertical slice

- **Status:** - [x]
- **Goal:** Агент получает context и запускает подготовку изменения двумя
  публичными командами, не воспроизводя вручную внутреннюю state machine.
- **Sources:** `README.md:20-155`, `crates/clew/src/main.rs:20-400`,
  `scripts/usability-smoke.py:90-330`.
- **Depends on:** —.
- **Read first:**
  - `crates/clew/src/main.rs:20-220` — Clap surface и аргументы;
  - `crates/clew/src/main.rs:240-405` — session/context/plan dispatch;
  - `scripts/usability-smoke.py:90-330` — acceptance-bearing flow.
- **Modify:**
  - `crates/clew/src/main.rs` — `change open|prepare|status|publish|recover`;
  - unit tests в `crates/clew/src/main.rs`;
  - `scripts/usability-smoke.py` — основной happy path через facade;
  - `README.md` — facade как основной flow, low-level API как advanced.
- **Product artifacts:** `README.md` меняет основной пользовательский flow.
  Отдельного scenario baseline в репозитории нет; новые baseline/cards/DOT не
  создаются, потому что README и executable smoke являются текущей канонической
  поверхностью данного developer tool.
- **Steps:**
  1. Добавить `Command::Change` и пять thin subcommands. Переиспользовать
     существующие функции; не создавать новую persisted authority.
  2. `change open` принимает аргументы session open плюс `--intent`, повторяемый
     `--term` и `--max-roots`; возвращает schema `codeclew-change-open/1.0`,
     session и bounded context.
     Если context creation падает после session open, facade под тем же session
     admission выполняет `abort` и `gc`. Если полная компенсация невозможна,
     typed error получает recovery authority (`transactionId=sessionId`) и
     retryable recovery code; orphan session никогда не скрывается от клиента.
  3. `change prepare` принимает `--session`, `--context`, `--plan`; безопасно
     читает/валидирует plan и вызывает idempotent start, возвращая schema
     `codeclew-change-prepare/1.0`, `planId` и текущий run projection.
  4. `status`, `publish`, `recover` являются typed wrappers существующих
     операций и не ослабляют conditional/recovery checks.
  5. Добавить parser/dispatch tests: обязательные аргументы, removed ambiguity,
     conditional flags и совпадение low-level/facade результатов.
  6. Перевести единственный usability smoke на `change` flow; close/gc оставить
     operational session-командами.
- **Verify:**
  ```bash
  cargo fmt --all --check && \
  cargo test --locked -p clew --bin clew 'tests::' -- --test-threads=1 && \
  python3 -I -S scripts/usability-smoke.py
  ```
- **DoD:**
  - основной README flow содержит только `change open`, `change prepare`,
    `change status`, `change publish` и exceptional `change recover`;
  - smoke публикует ровно один commit/two files через facade;
  - conditional approval остаётся exact digest/obligation-bound;
  - fault-injection test доказывает cleanup post-open failure, а failure самой
    компенсации возвращает session-bound typed recovery error;
  - stdout остаётся bounded и не содержит private paths;
  - low-level protocol продолжает проходить существующие tests.

---

## T01. K24-only production capsule

- **Status:** - [x]
- **Goal:** Cold/warm runtime собирает и арендует только доказанный Kotlin 2.4
  worker, исключая preview workers из стоимости и authority пилота.
- **Sources:** `README.md:9-27`, `bootstrap/runtime_components.json`,
  `bootstrap/clew_bootstrap.py:1330-1510`, финальный RELEASE smoke run
  `32607078475`.
- **Depends on:** T00.
- **Read first:**
  - `bootstrap/runtime_components.json` целиком;
  - `bootstrap/test_clew_bootstrap.py` tests, проверяющие component IDs;
  - `crates/clew/src/worker.rs` mapping compiler version → runtime worker.
- **Modify:**
  - `bootstrap/runtime_components.json` — оставить `clew` и `kotlin24`;
  - `bootstrap/test_clew_bootstrap.py` — exact registry/capsule expectations;
  - `crates/clew/src/worker.rs` — admission поддерживает только Kotlin 2.4;
  - component input closure: core `clew` не хеширует общий `workers/`, K24
    component хеширует только свои authoritative worker inputs;
  - связанные qualification fixtures, только если они проверяют default
    production registry;
  - `README.md` — K21/K23 остаются source-level research, не packaged runtime.
- **Product artifacts:** `README.md` сужает packaged capability до реально
  поддержанного K24. Другие продуктовые артефакты не существуют.
- **Steps:**
  1. Удалить K21/K23 entries из default registry, не добавляя profile switch.
     Удалить общий `workers/` из input closure core component; точные K24 пути
     остаются в K24 component.
  2. Отклонять любой non-K24 project до поиска/сборки worker distribution.
     Обновить exact tests и preflight expectations на `[clew, kotlin24]`.
  3. Проверить, что runtime manifest содержит `workerIds=[kotlin24]`, а Gradle
     build plan не включает K21/K23 tasks.
  4. Mutation test меняет non-K24-only source и доказывает неизменность
     production runtime/component key; изменение K24 source либо реально
     используемого K24 shared source обязано изменить key.
  5. Не удалять worker sources в этой задаче: отсутствие runtime references —
     достаточная граница, широкая чистка не даёт pilot outcome.
- **Verify:**
  ```bash
  python3 -I -S bootstrap/test_clew_bootstrap.py && \
  python3 -I -S scripts/check_repository_privacy.py --pre-commit && \
  ./clew --bootstrap-component-preflight
  ```
- **DoD:**
  - preflight возвращает ровно `clew,kotlin24`;
  - default cold build не запускает K21/K23 Gradle tasks;
  - non-K24-only source bytes не входят в production runtime authority; K24 и
    реально компилируемые им shared source bytes входят;
  - non-K24 project получает early typed unsupported error;
  - K24 RELEASE manifest verification остаётся fail-closed;
  - bootstrap tests не требуют cache seed/copy; facade smoke не повторяется в
    T01 и сохраняет уже полученное в T00 доказательство.

---

## T02. Трёхкейсный pilot runner

- **Status:** - [x]
- **Goal:** Один воспроизводимый запуск доказывает три разных K24 изменения на
  чистых репозиториях и выдаёт пригодный для сравнения обезличенный результат.
- **Sources:** `scripts/usability-smoke.py`, `fixtures/kotlin-basic`, публичный
  `change` flow из T00.
- **Depends on:** T00, T01.
- **Read first:**
  - `scripts/usability-smoke.py` целиком;
  - `fixtures/kotlin-basic/src/main/kotlin/com/acme/Samples.kt`;
  - `crates/clew/src/context_v2.rs` public bounded projection.
- **Modify:**
  - `scripts/pilot.py` — новый bounded runner;
  - `scripts/test_pilot.py` — unit tests агрегата/limits/failure behavior;
  - при необходимости маленькие plan builders внутри `scripts/pilot.py`, без
    отдельной framework/schema;
  - `scripts/usability-smoke.py` — переиспользовать общие безопасные helpers
    только если это уменьшает дублирование без изменения smoke claim.
- **Product artifacts:** No product artifact update because pilot execution does
  not change the user-visible flow defined by T00; T04 documents operation and
  interpretation after the runner exists.
- **Steps:**
  1. Runner создаёт один private `CODECLEW_HOME`, выполняет один cold prime,
     затем три fresh committed
     copies `fixtures/kotlin-basic`; абсолютные пути не выводятся.
  2. Cases: boundary behavior + new test; классификация edge-case + new test;
     method behavior change + new test. Каждый plan использует contentRef из
     своего context, exact replacement и exact expected file set.
  3. Для каждого case выполнить native baseline test, `change open`,
     `change prepare`, idempotent повтор, poll bounded status, strict refusal,
     conditional publish, повтор publish, native post-test, close/gc.
  4. Измерять только монотонные stage durations: native baseline, open,
     prepare-to-ready, publish, total. Не сохранять command, path, source text,
     stdout/stderr или пользовательский intent.
  5. stdout — один canonical JSON `codeclew-pilot/1.0`: 3 case IDs,
     pass/fail/errorCode, durations, runtimeMode, aggregate counts. При первом
     failure runner прекращает следующие cases и возвращает non-zero.
  6. Unit tests подменяют subprocess boundary и проверяют bounds, redaction,
     fail-fast и canonical order без запуска E2E.
- **Verify:**
  ```bash
  python3 -I -S scripts/test_pilot.py && \
  python3 -I -S scripts/pilot.py
  ```
- **DoD:**
  - pilot с одним cold prime завершает 3/3 cases без ручной очистки;
  - каждый case публикует один commit и только ожидаемые два файла;
  - один runtime переиспользуется; второй и третий cases не строят capsule;
  - aggregate JSON не содержит абсолютных путей или исходный код;
  - failure возвращает typed errorCode и не продолжает следующие cases.

---

## T03. Дешёвый PR gate и отдельная qualification lane

- **Status:** - [x]
- **Goal:** Обычные изменения получают быстрый сигнал без трёхкейсного E2E;
  warm/platform/pilot доказательства выполняются отдельно и не замедляют PR.
- **Sources:** `.github/workflows/ci.yml`, `scripts/ci-verify.sh`,
  `bootstrap/clew_bootstrap.py:2982-3155`, T02.
- **Depends on:** T01, T02.
- **Read first:**
  - `.github/workflows/ci.yml` и `scripts/ci-verify.sh` целиком;
  - `warm_audit_payload` и `--bootstrap-warm-audit`;
  - последний зелёный CI run `32607078475`.
- **Modify:**
  - `.github/workflows/ci.yml` — оставить current targeted tests + one smoke;
  - `.github/workflows/qualification.yml` — manual/scheduled matrix
    `ubuntu-latest, macos-latest`;
  - `scripts/qualification/pilot-readiness.sh` — prime once, strict warm audit,
    T02 pilot;
  - узкие Python/shell tests существующего gate style.
- **Product artifacts:** No product artifact update because CI routing changes
  assurance cadence, not the supported command flow or product decision.
- **Steps:**
  1. PR workflow не получает новый pilot run; один usability smoke остаётся
     единственным E2E acceptance.
  2. Qualification workflow запускается `workflow_dispatch` и weekly schedule,
     использует JDK 21 и pinned Rust на Linux/macOS.
  3. Qualification prime выполняется один раз, затем
     `./clew --bootstrap-warm-audit`; assert `PASSED`, `processRuns=0`,
     `digestFileCalls=0`, no cold toolchain/capsule build.
  4. После warm gate запустить T02 pilot. Не добавлять K21/K23/Maven arms.
  5. Обновить deprecated setup actions только в затронутых workflow, если
     доступная major версия совместима; это warning cleanup, не новый gate.
- **Verify:**
  ```bash
  sh -n scripts/qualification/pilot-readiness.sh && \
  python3 -I -S scripts/test_gate_safety.py
  ```
- **DoD:**
  - PR CI содержит ровно один product E2E;
  - qualification имеет только manual+weekly triggers и две supported OS;
  - warm audit fail-closed проверяет нулевые process/digest counters;
  - pilot failure блокирует только qualification claim, не unrelated PR CI.

---

## T04. Операторский контракт пилота и release decision gate

- **Status:** - [x]
- **Goal:** Команда может повторяемо использовать поддержанный contour и после
  20 реальных изменений принять evidence-based решение о prebuilt release.
- **Sources:** T00–T03, `README.md`, `docs/plans/codeclew-usable-first-plan.md`.
- **Depends on:** T03.
- **Read first:**
  - фактический stdout T02 pilot;
  - README supported contour и recovery/conditional sections;
  - error codes в `crates/clew/src/error.rs`.
- **Modify:**
  - `docs/pilot/README.md` — runbook, supported scope, feature-branch policy,
    recovery, anonymized recording fields, stop conditions;
  - `docs/pilot/case-template.json` — только case ID, project class, outcome,
    durations, runtime mode, error code; без repo/path/intent/source;
  - `README.md` — ссылка на pilot runbook и pilot-ready wording.
  - `scripts/check_repository_privacy.py` — запрещённый subtree для заполненных
    evidence;
  - `scripts/test_check_repository_privacy.py` — exact path-rule self-test;
  - `.gitignore` — удобный ignore, не являющийся security boundary.
- **Product artifacts:** `README.md` и `docs/pilot/README.md` фиксируют текущий
  operator flow. `case-template.json` является приватно заполняемым шаблоном,
  не доказательством и не report; заполненные cases запрещено коммитить.
- **Steps:**
  1. Зафиксировать обязательную feature-branch/clean-worktree политику пилота.
  2. Описать 20-case exit criteria: ≥95% prepare без ручной очистки, 100% no
     source mutation before publish, idempotent retry, typed failure/recovery,
     отсутствие private data.
  3. Решение о signed prebuilt capsule разрешено только после 20 completed
     cases. Failure ниже порога выбирает top typed blocker, а не расширение
     языка/runtime.
  4. Документировать три уровня checks: PR, qualification, future release.
  5. Заполненные evidence хранятся только вне repository. Зарезервировать
     `docs/pilot/results/` как forbidden prefix в scanner и ignore; scanner
     обязан отклонять путь даже при force-add. Committed template проходит.
- **Verify:**
  ```bash
  python3 -I -S scripts/test_check_repository_privacy.py && \
  python3 -I -S scripts/check_repository_privacy.py --pre-commit && \
  rg -n '20|95%|feature|qualification|release' docs/pilot/README.md README.md
  ```
- **DoD:**
  - новый оператор проходит supported flow без чтения research scripts;
  - заполненные pilot cases не могут попасть в Git;
  - критерий release decision численный и не допускает ручного повышения
    `UNSURE` до `VERIFIED`;
  - preview contours явно не входят в pilot claim.

---

## T05. Финальная проверка, публикация и один GitHub run

- **Status:** - [x]
- **Goal:** Опубликовать один coherent pilot-ready revision с независимым
  verdict и зелёным Linux RELEASE smoke.
- **Sources:** T00–T04, текущий `origin/main`, privacy policy.
- **Depends on:** T04.
- **Read first:**
  - полный diff T00–T04;
  - `scripts/ci-verify.sh`;
  - `.github/workflows/ci.yml` и qualification workflow.
- **Modify:**
  - только blocking fixes независимого review;
  - этот план — статусы T00–T05;
  - никаких новых feature/qualification arms.
- **Product artifacts:** No product artifact update because T05 verifies and
  publishes the already documented T00–T04 outcome.
- **Steps:**
  1. Независимый агент проверяет facade authority, K24-only registry, pilot
     redaction/fail-fast, warm gate locality и docs; максимум один targeted
     repair без нового full review.
  2. Выполнить fmt/clippy, targeted unit/bootstrap/pilot unit и privacy, затем
     ровно один финальный `./scripts/ci-verify.sh` как третий local product E2E.
  3. Не повторять ни smoke, ни pilot, ни `ci-verify` ради диагностики.
  4. Commit generic identity, lease-safe push, найти GitHub run exact SHA.
  5. Наблюдать единственный CI до terminal. При failure не исправлять и не
     перепушивать в рамках этого плана: зафиксировать точную причину и оставить
     T05/цель незавершёнными.
- **Verify:**
  ```bash
  test -z "$(git status --porcelain=v1 --untracked-files=all)" && \
  test "$(git rev-parse HEAD)" = "$(git ls-remote origin refs/heads/main | cut -f1)" && \
  gh run list --commit "$(git rev-parse HEAD)" --workflow ci.yml \
    --json databaseId,headSha,status,conclusion,url
  ```
- **DoD:**
  - независимый verdict `PASS` без critical/major;
  - local targeted checks зелёные;
  - exact GitHub SHA зелёный и smoke сообщает `runtimeMode=RELEASE`;
  - worktree чистый, local/remote HEAD совпадают;
  - все T00–T05 отмечены `[x]`, либо цель честно оставлена active/blocked с
    точным remaining task — не объявлена завершённой частично.

## Финальный чек

```bash
unchecked=$(grep -cE '^- \*\*Status:\*\* - \[ \]' docs/plans/codeclew-pilot-readiness-implementation-plan.md)
test "$unchecked" = "0" && echo PLAN-COMPLETE || {
  echo "outstanding tasks:"
  grep -nE '^- \*\*Status:\*\* - \[ \]' docs/plans/codeclew-pilot-readiness-implementation-plan.md
}
```
