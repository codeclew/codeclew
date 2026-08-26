# Codeclew P0: установка и эксплуатация

Этот документ описывает минимальный промышленный контур Codeclew: как
развернуть исходный дистрибутив на машине разработчика, подключить Codex или
Claude, безопасно переживать изменения целевых репозиториев и собирать материал
для расследований без передачи исходного кода.

P0 является пилотным контуром, а не обещанием общей доступности. Для публичного
macOS release используется установленная команда `clew`; для разработки из
исходников — `./clew` из зафиксированного checkout. Прямой запуск бинарника из
runtime capsule и ручное редактирование `CODECLEW_HOME` не поддерживаются.

## 1. Что именно поддерживается

Машиночитаемая истина находится в
`crates/clew/support-matrix.json` и возвращается командой:

```bash
./clew capabilities
```

На P0:

| Профиль | Чтение | Изменение и публикация |
|---|---:|---:|
| Kotlin 2.4.10, Gradle wrapper, одна compilation, `PROJECT_NATIVE` | да, K2 | да, пилот |
| Kotlin 2.3.0, Maven | да, preview | нет |
| Python, Tree-sitter syntax | да, preview | нет |
| Rust, bounded syntax | да, preview | нет |
| Нить из 2–8 репозиториев | да | нет |

Python и Rust дают синтаксические факты, а не доказанную динамическую семантику.
Многорепозиторная нить не превращает объявленную топологию в доказанную связь и
не может быть источником плана изменения.

## 2. Установка на другую машину

### Публичный macOS pilot

Для Apple Silicon и Intel Mac установка выполняется одной строкой:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | sh
```

Installer определяет архитектуру, скачивает готовый bundle из последнего GitHub
Release, проверяет опубликованный SHA-256, безопасно распаковывает архив без
symlink/path traversal и атомарно обновляет launcher. Codeclew на машине
пользователя не компилируется.

По умолчанию файлы размещаются в:

- `~/.local/share/codeclew/releases/<version>-macos-<arch>`;
- `~/.local/bin/clew` — атомарная ссылка на выбранный release.

Можно закрепить версию и изменить каталоги локальными переменными:

```bash
curl -fsSL https://codeclew.github.io/codeclew/install.sh | \
  CODECLEW_VERSION=v0.1.0 \
  CODECLEW_INSTALL_ROOT=/absolute/private/path/releases \
  CODECLEW_BIN_DIR=/absolute/private/path/bin sh
```

После установки:

```bash
clew capabilities
clew doctor
```

Публичный pilot проверяет checksum, но пока не является Apple-notarized
поставкой. Release строится только на GitHub macOS runner соответствующей
архитектуры и содержит запечатанный runtime seed, поэтому Rust/Cargo/Gradle не
нужны для установки или старта самого Codeclew. Git, Python 3.11+ и JDK 21 для
анализа Kotlin-проектов остаются внешними зависимостями.

### Зависимости source build

Нужны:

- macOS или Linux;
- Git;
- Python 3.11 или новее;
- JDK 21;
- Rust/Cargo из `rust-toolchain.toml` (сейчас Rust 1.92.0);
- не менее 6 GiB свободного места на томе `CODECLEW_HOME` для холодной сборки;
- Maven в `PATH` только для Maven-проектов без `./mvnw`.

Холодная сборка runtime capsule также должна иметь доступ к уже заполненным
локальным dependency caches или к одобренным источникам Cargo, Gradle и Maven.
Изолированная поставка готового подписанного runtime не входит в P0; для
полностью offline/fleet-развёртывания нужен отдельный release pipeline.

### Установка из зафиксированного исходного checkout для разработки

```bash
git clone <approved-codeclew-repository> /absolute/path/to/codeclew
cd /absolute/path/to/codeclew
git checkout <approved-commit-or-tag>
git status --short

export CODECLEW_ROOT=/absolute/path/to/codeclew
export CODECLEW_HOME=/absolute/private/path/codeclew-state
mkdir -p "$CODECLEW_HOME"
chmod 700 "$CODECLEW_HOME"

./clew --bootstrap-component-preflight
./clew capabilities
./clew doctor
```

Checkout должен быть неизменённым и закреплённым за одобренным commit/tag.
`CODECLEW_HOME` должен быть физическим нормализованным абсолютным путём,
принадлежать текущему пользователю и иметь режим `0700`. Не используйте один
state root несколькими Unix-пользователями и не размещайте его в синхронизируемой
папке. Если `CODECLEW_HOME` не задан, используется пользовательский cache root.

Первый обычный запуск строит и запечатывает immutable runtime capsule. Тёплый
запуск проверяет и повторно использует его. После развертывания сохраните JSON
от `capabilities` и `doctor` как baseline конкретной машины; оба ответа намеренно
не содержат путей или идентичности репозитория.

Для целевого репозитория выполните отдельную проверку:

```bash
./clew doctor \
  --repo /absolute/path/to/target-repository \
  --target-ref refs/heads/feature/codeclew-task
```

Команда может завершиться с кодом 0 и `status: ACTION_REQUIRED`: автоматизация
обязана читать JSON и разрешать работу только когда все строки с
`required: true` имеют `status: PASS`.

## 3. Подключение Codex и Claude

В checkout Codeclew уже находятся project skills:

- Codex: `.agents/skills/codeclew/SKILL.md`;
- Claude: `.claude/skills/codeclew/SKILL.md`.

Если агент запускается из целевого репозитория, скопируйте соответствующий skill
в него. Не создавайте машинно-зависимый symlink и не записывайте абсолютный путь
Codeclew в Git:

```bash
export CODECLEW_ROOT=/absolute/path/to/codeclew
export TARGET_REPO=/absolute/path/to/target-repository

install -d "$TARGET_REPO/.agents/skills/codeclew"
install -m 0644 \
  "$CODECLEW_ROOT/.agents/skills/codeclew/SKILL.md" \
  "$TARGET_REPO/.agents/skills/codeclew/SKILL.md"

install -d "$TARGET_REPO/.claude/skills/codeclew"
install -m 0644 \
  "$CODECLEW_ROOT/.claude/skills/codeclew/SKILL.md" \
  "$TARGET_REPO/.claude/skills/codeclew/SKILL.md"
```

Перед запуском агента задайте `CODECLEW_ROOT` в его локальном окружении. Skill
требует использовать `$CODECLEW_ROOT/clew`, запускать `capabilities` и `doctor`,
не угадывать язык/compilation, проверять freshness и не публиковать результат
без явного разрешения пользователя. После обновления Codeclew скопируйте skill
повторно. В контролируемой среде полезно проверять совпадение его хеша с
одобренной версией.

Для проверки обнаружения попросите агента явно применить `codeclew` к безопасной
read-only задаче. Успешный агент сначала покажет результат admission
(`capabilities`/`doctor`), а не начнёт читать весь репозиторий обычными shell
командами. Явное указание имени skill остаётся аварийным способом, если
автоматический выбор не сработал.

## 4. Обычный Kotlin workflow

Целевой ref должен указывать на текущий `HEAD`, а worktree быть чистым. Работайте
на отдельной feature-ветке; Codeclew не должен публиковать прямо в защищённую
ветку.

```bash
"$CODECLEW_ROOT/clew" change open \
  --repo /absolute/path/to/kotlin-repository \
  --target-ref refs/heads/feature/codeclew-task \
  --language kotlin \
  --compilation :app/main \
  --intent 'описание изменения' \
  --term ImportantSymbol \
  --term ImportantBehavior
```

Сохраните `sessionId` и `contextId` из JSON. Подготовьте закрытый edit-plan и
перед началом изолированной мутации проверьте freshness:

```bash
"$CODECLEW_ROOT/clew" change check-freshness --session session:...

"$CODECLEW_ROOT/clew" change prepare \
  --session session:... \
  --context context:sha256:... \
  --plan /absolute/private/path/edit-plan.json

"$CODECLEW_ROOT/clew" change status --run run:...
```

Проверьте candidate diff, результаты compile/test и все obligations. Статусы
`READY_TO_PUBLISH_CONDITIONAL` и `VALIDATED_CONDITIONAL` не являются зелёным
сигналом: оставшиеся проверки должны быть явно выполнены или приняты владельцем
изменения.

Непосредственно перед публикацией повторите `change check-freshness`. Затем,
только после явного согласия пользователя:

```bash
"$CODECLEW_ROOT/clew" change publish \
  --session session:... \
  --run run:...

"$CODECLEW_ROOT/clew" session close --session session:...
"$CODECLEW_ROOT/clew" session gc --session session:...
```

## 5. Часто обновляемый репозиторий

Session привязан к точному runtime, исходному commit, target ref и target OID.
Внешний push сам по себе не меняет локальный ref; обновление локальной ветки,
commit другого разработчика или незакоммиченная правка меняют её состояние.
Codeclew не делает rebase и не переносит старый план автоматически.

`change check-freshness` возвращает:

| Статус | Смысл | Действие |
|---|---|---|
| `FRESH` | `HEAD`, target ref и ожидаемый OID совпадают, worktree чист | можно продолжать |
| `DIRTY` | есть локальные изменения | остановиться; владелец решает, commit/stash/другой worktree |
| `STALE` | `HEAD` или ref ушёл от session authority | закрыть старую session и открыть новую |
| `UNAVAILABLE` | репозиторий/locator/Git недоступен | восстановить доступ, не публиковать |
| `TERMINAL` | session закрыта/abort/gc | открыть новую при необходимости |

Ранбук для `STALE`:

1. Не публиковать и не переиспользовать старый edit-plan.
2. Сохранить только нужное человеку описание intent; не копировать старые
   semantic assertions как доказанные.
3. Закрыть старую session.
4. Обновить целевую feature-ветку обычным командным процессом команды.
5. Добиться чистого worktree и `target ref == HEAD`.
6. Повторить `doctor`, `change open`, построение context и plan.

Если публикация столкнулась с compare-and-swap, применяется тот же ранбук. Это
ожидаемая защита от гонки, а не повод принудительно двигать ref.

## 6. Чтение Python и Rust

Python читается из tracked UTF-8 `.py` blobs точного base commit. Codeclew не
запускает Python, не импортирует модули, не читает `.env` и не устанавливает
зависимости.

```bash
"$CODECLEW_ROOT/clew" session open \
  --repo /absolute/path/to/python-repository \
  --target-ref refs/heads/main \
  --language python \
  --compilation 'python:.#src'

"$CODECLEW_ROOT/clew" context create \
  --session session:... \
  --intent 'найти путь нормализации запроса' \
  --term normalize \
  --term Request
```

Import root должен совпадать с source root или быть его предком. Выход остаётся
`PARTIAL/UNSURE`: framework wiring, runtime imports, типы и реальные call edges
проверяются штатными тестами Python-проекта.

Для Rust нужен корневой обычный `Cargo.lock` и точный target selector:

```bash
"$CODECLEW_ROOT/clew" session open \
  --repo /absolute/path/to/rust-repository \
  --target-ref refs/heads/main \
  --language rust \
  --compilation 'cargo:crates/example/Cargo.toml#example#lib#example'
```

Rust preview не утверждает name resolution, `cfg`, procedural macro или call
edge. Python/Rust session нельзя передать в `change prepare`/publish.

## 7. Многорепозиторные нити

Откройте отдельную session для каждого точного repository/language/compilation.
Один и тот же репозиторий может иметь несколько analysis units. Затем свяжите
от двух до восьми sessions:

```bash
"$CODECLEW_ROOT/clew" thread open \
  --member provider=session:... \
  --member consumer=session:... \
  --service-alias provider=orders \
  --service-alias consumer=checkout

"$CODECLEW_ROOT/clew" thread context \
  --thread thread:... \
  --intent 'проследить нормализацию между сервисами' \
  --term normalize \
  --term Service
```

Для квалифицированных Kotlin members доступны `thread callables`,
`thread impact` и условная `thread validate`; полные примеры находятся в
README. Нить read-only, не владеет member sessions и не может использоваться в
plan/task-run. Закрытие и GC нити не закрывают member sessions:

```bash
"$CODECLEW_ROOT/clew" thread close --thread thread:...
"$CODECLEW_ROOT/clew" thread gc --thread thread:...
```

При обновлении хотя бы одного репозитория откройте новую session этого member и
новую immutable thread. Старую нить не «подменяют» новым member задним числом.

## 8. Ошибки и материал для расследования

Полный stdout может содержать source windows, diff, symbols, arguments, IDs и
пути. Он нужен для локального расследования, но не является безопасным для
отправки. Codeclew не отправляет его автоматически.

Создайте приватный каталог и захватите stdout/stderr отдельно:

```bash
umask 077
INCIDENT_DIR=/absolute/private/path/codeclew-incident-$(date +%Y%m%d-%H%M%S)
mkdir -m 700 "$INCIDENT_DIR"

"$CODECLEW_ROOT/clew" <command> \
  >"$INCIDENT_DIR/result.json" \
  2>"$INCIDENT_DIR/completion.json" || true
chmod 600 "$INCIDENT_DIR/result.json" "$INCIDENT_DIR/completion.json"
```

Для core error/status передайте локальный `result.json` в allowlist-конвертер.
Для bootstrap failure передайте файл, содержащий ровно один bootstrap error
JSON; если сломанная установка не запускает summarizer, используйте исправную
установку той же одобренной версии на доверенной машине:

```bash
"$CODECLEW_ROOT/clew" support summarize \
  --input "$INCIDENT_DIR/result.json" \
  >"$INCIDENT_DIR/shareable-summary.json"
```

Вход обязан быть нормализованным абсолютным путём, обычным файлом владельца с
режимом ровно `0600`, размером не более 1 MiB и не symlink. Чтение проверяет
identity/size/timestamps до и после, поэтому гонка закрывается отказом.

Выход строится только из allowlist и имеет `status: SAFE_TO_SHARE`. Он содержит
schema/stage, типизированный код ошибки или terminal status, retryability,
remediation ID и digest самой очищенной сводки. Он не переносит сообщения,
исходники, diff, symbols, arguments, repository content digests, repository/session/run
identity или пути.

В обращение можно приложить только:

1. `shareable-summary.json`;
2. свежий JSON `capabilities`;
3. свежий JSON `doctor` без `--repo` либо с repo-проверками — оба варианта
   path-free;
4. человеческое время события и повторяемость без имён/путей/фрагментов кода.

Не прикладывайте raw stdout/stderr, plan, candidate diff, CAS, runtime/state
каталоги, Git remote, названия закрытых symbols и командную строку. Оригиналы
остаются только на машине разработчика по политике retention команды и
удаляются после закрытия расследования.

## 9. Типовые аварийные ранбуки

### Worker crash

1. Проверить typed error и `retryable`.
2. Для `WORKER_CRASHED` повторить операцию один раз без смены authority.
3. При повторе остановиться, собрать safe summary и сохранить локальные raw
   artifacts.
4. Не делать бесконечный retry и не удалять state до решения расследующего.

### `WORKTREE_RECOVERY_REQUIRED`

1. Не изменять candidate worktree вручную.
2. Запустить `change recover --session session:... --run run:...`.
3. Снова получить `change status` и выполнить указанную remediation.
4. Если recovery повторно не завершается, сохранить incident и остановиться.

### `PROJECT_MODEL_CHANGED`

Session была создана другим runtime/model authority. Запустите её из исходного
зафиксированного checkout Codeclew или закройте и создайте новую session текущей
версией. Не переписывайте session JSON.

### Недостаток места

Освободите минимум 6 GiB на state volume, затем повторите `doctor`. Удаляйте
только завершённые sessions/threads их командами `gc`; ручное удаление объектов
может разрушить authority. Если state подозревается в повреждении, сначала
сохраните локальный incident и прекратите запись.

### Грязный worktree

Codeclew ничего не stash/reset. Владелец выбирает commit, отдельный Git worktree
или ручной stash. После этого повторяются `doctor` и freshness. Агент не должен
принимать это решение самостоятельно.

## 10. Обновление Codeclew

Обновляйте checkout только между задачами:

1. Завершите либо явно abort/close все активные sessions и threads.
2. Сохраните baseline `capabilities` и `doctor` старой версии.
3. Переключите checkout на новый одобренный commit/tag без локальных правок.
4. Выполните bootstrap preflight, `capabilities`, `doctor` и пилотный smoke case.
5. Обновите copies skills в целевых репозиториях.
6. Не мигрируйте и не редактируйте старые session records. Для нового runtime
   открывайте новые sessions.

Rollback означает запуск старого зафиксированного checkout с его совместимым
runtime. Общий state хранит capsules content-addressed, но P0 не обещает, что
новый CLI продолжит старую session при изменении runtime/model authority.

## 11. Как добавлять язык или расширять профиль

Поддержка языка не добавляется одним parser plugin. Минимальный безопасный путь:

1. Описать новый profile и его границу в support matrix; сначала
   `READ_ONLY_PREVIEW`, `mutation: false`.
2. Реализовать или выбрать `BuildModelProvider` для точной compilation authority.
3. Реализовать `LanguageAdapter` handshake и generation facts с явной
   completeness/certainty/obligations.
4. Читать только sealed repository snapshot и публиковать canonical bounded
   facts в CAS; не сканировать ambient filesystem и не запускать проект на
   query path.
5. Добавить CLI language/compilation parsing, admission и path/privacy tests.
6. Добавить fixture corpus: корректные проекты, ambiguity, parse/model failure,
   symlinks, dirty/untracked data, resource limits и deterministic replay.
7. Доказать read-only acceptance на реальных проектах.
8. Для mutation отдельно реализовать plan validation, isolated candidate,
   compile/test gates, effects/writeset/ABI checks, freshness/CAS publication и
   recovery. Только после независимого mutation gate менять support matrix.

Текущие точки расширения находятся в `crates/clew/src/adapter_v2.rs`;
референсы — `kotlin_adapter_v2.rs`, `python_adapter_v2.rs`,
`rust_adapter_v2.rs`, а project-model контуры Python/Rust находятся в соседних
модулях. Runtime-packaged Kotlin worker живёт в `workers/` и регистрируется через
component manifests. Любая новая версия компилятора является новым профилем, а
не молчаливой заменой существующего.

## 12. P0 acceptance checklist

Развёртывание готово к пилоту, когда:

- checkout и support matrix закреплены одобренным commit;
- `capabilities` и все required `doctor` checks проходят;
- state root приватный и не общий;
- Codex/Claude skill установлен и проверен read-only запросом;
- Kotlin mutation допускается только в точном P0 profile;
- перед prepare/publish проверяется freshness;
- conditional obligations не скрываются;
- публикация требует явного согласия человека;
- incident workflow выдаёт только `SAFE_TO_SHARE` summary;
- команда умеет выполнить stale, recovery, upgrade и disk-space runbooks.

Подписанный installer, централизованное fleet-управление, автоматическая
доставка диагностик, cross-host shared state, Python/Rust mutation и
многорепозиторная публикация намеренно остаются за границей P0.
