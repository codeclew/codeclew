'use strict';
const publication=JSON.parse(document.getElementById('document-data').textContent);

// Explicit publication locale. The English source strings are the English dictionary.
// Translate only trusted renderer literals, never interpolated author text or code.
const language=publication.language==='ru'?'ru':'en';
const RU_MESSAGES={
 "Source": "Исходный код",
 "Supporting code": "Код, подтверждающий описание",
 "Supporting source": "Подтверждающий код",
 "Source evidence": "Подтверждающий код",
 "Source freshness:": "Актуальность кода:",
 "Meaning review:": "Проверка смысла:",
 "Execution flow": "Порядок действий",
 "Dependency map": "Карта зависимостей",
 "Decision table": "Таблица решений",
 "Visual artifact": "Диаграмма или таблица",
 "No graph nodes were supplied.": "Узлы диаграммы не добавлены.",
 "Arrows carry the documented relationships. Node position alone does not establish order. A complete node and connection table follows.": "Стрелки показывают описанные связи. Положение узлов само по себе не задаёт порядок. Ниже приведён полный список узлов и связей.",
 "Arrows show dependencies, not execution order or runtime calls.": "Стрелки показывают зависимости. Они не задают порядок выполнения и не подтверждают вызовы при работе программы.",
 "Follow the arrows and their conditions. Placement alone does not establish order; this is a source-based interpretation, not an observed runtime trace.": "Следуйте стрелкам и условиям на них. Положение узлов не задаёт порядок. Это объяснение исходного кода; фактическое выполнение не наблюдалось.",
 "Complete diagram as text and supporting code": "Полное текстовое описание диаграммы и подтверждающий код",
 "Nodes": "Узлы",
 "Directed connections": "Направленные связи",
 "Used at": "Используется в узле",
 "in": "диаграммы",
 "Local decision; placement in the wider process is not established.": "Локальное решение; его место в общем процессе пока не установлено.",
 "Choose the first matching row, top to bottom. Later matching rows do not contribute another result.": "Строки проверяются сверху вниз. Выбирается первая подходящая строка; следующие строки уже не добавляют результат.",
 "At most one row may match. Multiple matching rows mean the documented decision rules conflict.": "Подходить может не более одной строки. Если подходят несколько, описанные правила противоречат друг другу.",
 "The rule matching policy has not been established. Do not infer priority or exclusivity from row order.": "Правило выбора строк не установлено. Порядок строк не доказывает приоритет или исключительность условий.",
 "Policy interpretation:": "Как понимается правило выбора:",
 "A row selects a result; this table does not establish when its resulting actions execute.": "Строка выбирает результат. Таблица не устанавливает, когда выполняются выбранные действия.",
 "Decision notation": "Обозначения таблицы решений",
 "Hit policy:": "Правило выбора строк:",
 ". This is an authored policy interpretation, not a formally verified or executable DMN model.": ". Это объяснение правила выбора, а не формально проверенная или исполняемая модель DMN.",
 "Rule": "Правило",
 "Condition": "Условие",
 "Selected result": "Выбранный результат",
 "After selection: execution and outcomes": "После выбора: выполнение и результаты",
 "Why this view:": "Для чего нужна эта схема:",
 "Scope:": "Область описания:",
 "Limits of this view": "Ограничения этой схемы",
 "Artifact binding and producer": "Связь с версией документации и генератор",
 "Accepted with": "Принято вместе с",
 "). Freshness and meaning review belong to this containing accepted version.": "). Актуальность и проверка смысла относятся к этой принятой версии раздела.",
 "Processes and diagrams": "Процессы и диаграммы",
 "Selected source-based views. Each view records its own purpose, scope and limitations; these are not an exhaustive list of service behavior.": "Выбранные схемы по исходному коду. Для каждой указаны назначение, область описания и ограничения. Это не полный список поведения сервиса.",
 "Responsibilities": "Задачи сервиса",
 "Domain entities": "Предметные сущности",
 "Entry points": "Точки входа",
 "External calls and storage": "Внешние вызовы и хранение",
 "Service profile": "Описание сервиса",
 "DTOs and implementation classes alone do not establish entity ownership or creation.": "Одного наличия DTO и классов недостаточно, чтобы определить, кто создаёт сущности и отвечает за них.",
 "Declared domain identities:": "Объявленные предметные сущности:",
 "Entity ownership and creation roles have no explicit domain declarations in this publication.": "В этой публикации явно не указано, кто создаёт предметные сущности и отвечает за них.",
 "Behavior narrative available": "Описание поведения доступно",
 "Behavior narrative not yet accepted": "Описание поведения ещё не принято",
 "No public entries are recorded in this publication. This does not establish that the service has no entry points.": "В этой публикации не записаны публичные точки входа. Это не доказывает, что у сервиса их нет.",
 "Discovery scope": "Область поиска",
 "The inventory is bounded by retained analyzer evidence. Per-transport completeness for HTTP, messaging, schedules and CLI is not established here; absent entries are not proof of absence.": "Список ограничен сохранёнными результатами анализатора. Полнота поиска для HTTP, сообщений, расписаний и CLI не установлена; отсутствие записи не доказывает отсутствие точки входа.",
 "This is the accepted description of external boundaries. Thread-to-call links and complete outgoing coverage are not inferred from a dependency list.": "Это принятое описание внешних взаимодействий. Список зависимостей сам по себе не устанавливает связь процесса с вызовом и полноту исходящих вызовов.",
 "Scope and limitations": "Область описания и ограничения",
 "No source-bound section summary has been accepted in this publication.": "Для этого раздела ещё не принято описание, связанное с исходным кодом.",
 "Open": "Открыть",
 " details →": ": подробнее →",
 "Declared OpenAPI · ": "Объявлено в OpenAPI · ",
 "Process overview": "Обзор процесса",
 "Detailed process flow": "Подробный порядок действий",
 "Entity data flow": "Поток данных сущностей",
 "Source flow scope": "Область анализа кода",
 "Internal processes": "Внутренние процессы",
 "Saved entity views": "Сохранённые схемы сущностей",
 "Static entity data flow; an accessible edge table follows": "Поток данных по исходному коду; ниже доступно текстовое описание связей",
 "Unknown candidate": "Неподтверждённая связь",
 "Declared transfer": "Объявленная передача",
 "Related human notes": "Связанные заметки людей",
 "Explicit documentation gap": "Явный пробел в документации",
 "No matching operations": "Подходящих операций нет",
 "Sequence diagram:": "Диаграмма последовательности:",
 "External participant": "Внешний участник",
 "Declared service interaction": "Объявленное взаимодействие сервисов",
 "inspect source": "открыть исходный код",
 "Implementation commentary": "Пояснения к реализации",
 "What happens": "Что происходит",
 "Service links": "Связи сервисов",
 "Declared connections in this scenario. Conditions are explained below; this map does not imply synchronous execution.": "Объявленные связи в этом сценарии. Условия описаны ниже; схема не подразумевает синхронное выполнение.",
 "A bounded overview diagram has not been authored. Source evidence remains available below.": "Обзорная диаграмма ещё не подготовлена. Подтверждающий код доступен ниже.",
 "Overview diagram:": "Обзорная диаграмма:",
 "Transition": "Переход",
 "Behavior documentation has not been authored for this entrypoint.": "Описание поведения для этой точки входа ещё не подготовлено.",
 "Scenario overview": "Обзор сценария",
 "Select a node or connection to inspect its source": "Выберите узел или связь, чтобы открыть исходный код",
 "Source-based interpretation; declared links do not prove runtime delivery.": "Объяснение исходного кода; объявленные связи не доказывают фактическую доставку.",
 "Diagram as text": "Текстовое описание диаграммы",
 "Scope and evidence boundaries": "Область описания и границы подтверждений",
 "Implementation details and source commentary": "Подробности реализации и пояснения к коду",
 "Declared service links": "Объявленные связи сервисов",
 "No cross-service connections selected.": "Межсервисные связи не выбраны.",
 "Contract element": "Элемент контракта",
 "Value / behavior": "Значение / поведение",
 "Evidence": "Подтверждения",
 "Source-derived interface descriptions authored from the retained code. These are separate from published OpenAPI contracts and do not prove deployed wire compatibility.": "Описания интерфейсов подготовлены по сохранённому коду. Они рассматриваются отдельно от опубликованных контрактов OpenAPI и не доказывают совместимость развёрнутых систем.",
 "Interface contract": "Контракт интерфейса",
 "Payload fields and nested types": "Поля данных и вложенные типы",
 "schemas": "схем",
 "Payload schema": "Схема данных",
 "No schema declared": "Схема не объявлена",
 "See the complete schema below for deeper fields.": "Более глубокие поля приведены в полной схеме ниже.",
 "composition (alternatives are preserved)": "композиция (альтернативы сохранены)",
 "Field": "Поле",
 "Type": "Тип",
 "Required": "Обязательность",
 "Constraints / description": "Ограничения / описание",
 "Nested fields": "Вложенные поля",
 "Declared OpenAPI example; not an observed request.": "Пример объявлен в OpenAPI; это не запись реального запроса.",
 "Declared examples": "Объявленные примеры",
 "No body declared.": "Тело не объявлено.",
 "Interface contract is not documented for this operation. Neither an unambiguous OpenAPI operation nor a source-derived contract was supplied.": "Контракт интерфейса для этой операции не описан. Не предоставлены ни однозначная операция OpenAPI, ни контракт по исходному коду.",
 "Entrypoint source": "Код точки входа",
 "Declared OpenAPI contract from the same revision.": "Объявленный контракт OpenAPI из той же ревизии.",
 ". Runtime enforcement is unverified.": ". Соблюдение контракта при выполнении не проверено.",
 "Original contract": "Исходный контракт",
 "Request parameters": "Параметры запроса",
 "Parameter": "Параметр",
 "Location / type": "Расположение / тип",
 "Constraints": "Ограничения",
 "No parameters declared.": "Параметры не объявлены.",
 "Request body": "Тело запроса",
 "Required body": "Обязательное тело",
 "Optional or absent body": "Необязательное или отсутствующее тело",
 "Responses": "Ответы",
 "Headers": "Заголовки",
 "Response contract": "Контракт ответа",
 "Declared access and servers": "Объявленные правила доступа и серверы",
 "Static declarations do not prove that access checks or server bindings are active.": "Объявления в коде не доказывают, что проверки доступа и привязки серверов действуют.",
 "Complete operation contract": "Полный контракт операции",
 "SOURCE-BASED OBSERVATION": "НАБЛЮДЕНИЕ ПО ИСХОДНОМУ КОДУ",
 "Compare evidence": "Сравнить подтверждения",
 "Static findings are separate from observed runtime failures.": "Выводы по исходному коду рассматриваются отдельно от наблюдавшихся сбоев.",
 "No findings recorded within the documented scope.": "В пределах описанной области замечания не записаны.",
 "Legacy publication: meaning review has not been assessed.": "В прежней публикации проверка смысла не проводилась.",
 "Saved evidence reused for": "Сохранённые подтверждения повторно использованы для",
 ". These sources were not rechecked during the selected-service capture.": ". Этот код не проверялся повторно при сборе данных выбранного сервиса.",
 "Recorded source dependencies are current.": "Записанные зависимости от кода актуальны.",
 "Source changed. This retained explanation needs review.": "Код изменился. Сохранённое объяснение требует проверки.",
 "Source status could not be established. This explanation is retained with a gap.": "Состояние кода установить не удалось. Объяснение сохранено с указанием пробела.",
 "Content:": "Описание:",
 "Mixed source versions; inspect each operation.": "Смешаны версии кода; проверьте каждую операцию.",
 "Target:": "Целевая версия:",
 "Recent update gaps": "Пробелы последнего обновления",
 "What needs attention": "Что требует внимания",
 "No entrypoints available": "Доступных точек входа нет",
 "Inspect coverage and the service evidence gaps.": "Проверьте полноту анализа и пробелы в подтверждениях по сервису.",
 "Status of the generated assessment · original note retains human/imported authority": "Состояние созданной оценки · исходная заметка остаётся записью человека или импортированным материалом",
 "Assessment: UNASSESSED. Original note retains human/imported authority.": "Оценка ещё не проводилась. Исходная заметка остаётся записью человека или импортированным материалом.",
 "HUMAN / IMPORTED NOTE": "ЗАМЕТКА ЧЕЛОВЕКА / ИМПОРТИРОВАННЫЙ МАТЕРИАЛ",
 "Period:": "Период:",
 "This retained note snapshot or its association has changed since publication. The retained assessment is stale.": "Снимок заметки или её связи изменились после публикации. Сохранённая оценка устарела.",
 "Original metadata and associations": "Исходные метаданные и связи",
 "Unresolved targets:": "Неустановленные объекты:",
 "Separate agent assessment": "Отдельная оценка агента",
 "Period assessed:": "Оценённый период:",
 "Input binding:": "Связь с входными данными:",
 "matches displayed capture": "соответствует показанному снимку",
 "Freshness:": "Актуальность:",
 "Assessment evidence": "Подтверждения оценки",
 "Proposed correction · original unchanged": "Предложенное исправление · оригинал не изменён",
 "Correction evidence": "Подтверждения исправления",
 "No assessment has been accepted for this note on this page.": "На этой странице для заметки ещё не принята оценка.",
 "Open the assessment service": "Открыть сервис с оценкой",
 "Classification and original text remain human/imported declarations. Evidence tracking covers captured inputs; it cannot establish every implicit claim in arbitrary prose.": "Классификация и исходный текст остаются записями человека или импортированным материалом. Учёт подтверждений охватывает сохранённые входные данные, но не устанавливает истинность каждого неявного утверждения в произвольном тексте.",
 "SAVED ENTITY VIEW": "СОХРАНЁННАЯ СХЕМА СУЩНОСТЕЙ",
 "Domain identities:": "Предметные сущности:",
 ". A DTO, message or table is an implementation representation; its domain mapping remains an interpretation.": ". DTO, сообщение или таблица — форма представления в реализации; связь с предметной сущностью остаётся интерпретацией.",
 "The definition or a linked component has changed. Retained graph content needs review.": "Описание или связанный компонент изменились. Сохранённую схему нужно проверить.",
 "No evidence-bound graph has been accepted.": "Схема с привязкой к подтверждениям ещё не принята.",
 "View evidence": "Подтверждения схемы",
 "Static data flow, not a runtime trace or universal taint analysis. Dashed candidates remain unknown; declared transfers do not establish routing or wire compatibility.": "Поток данных описан по коду, а не по наблюдению выполнения или полному анализу распространения данных. Пунктирные связи не подтверждены; объявленная передача не доказывает маршрутизацию и совместимость форматов.",
 "Representations and source bindings": "Представления и привязки к коду",
 "Node": "Узел",
 "Domain identity": "Предметная сущность",
 "Representation": "Представление",
 "Node source": "Код узла",
 "Explicit domain declaration": "Явное объявление предметной сущности",
 "Reads, transformations, writes and transfers": "Чтение, преобразование, запись и передача",
 "Edge evidence": "Подтверждения связи",
 "Protected human annotations": "Защищённые комментарии людей",
 "No human annotations have been added.": "Комментарии людей ещё не добавлены.",
 "Tags:": "Метки:",
 "Human metadata and layout": "Метаданные и расположение, заданные человеком",
 "Related accepted components": "Связанные принятые компоненты",
 "Evidence limits": "Границы подтверждений",
 "Callable": "Метод",
 "Needs evidence": "Нужны подтверждения",
 "Awaiting explanation": "Ожидает объяснения",
 "unscoped source": "область кода не указана",
 " lexical call sites (targets unresolved)": " мест вызова в тексте (цели не установлены)",
 " distinct local call targets": " различных целей локальных вызовов",
 "control-flow events": "событий управления выполнением",
 "No business interpretation has been accepted for this candidate.": "Для этого кандидата ещё не принято объяснение на уровне предметной области.",
 "Retained method source": "Сохранённый код метода",
 "Missing or limited evidence": "Отсутствующие или ограниченные подтверждения",
 "Exact root and discovery reasons": "Точная исходная точка и основания обнаружения",
 "Saved process explanations and structural candidates from retained code evidence.": "Сохранённые объяснения процессов и кандидаты, найденные по структуре сохранённого кода.",
 "Saved processes": "Сохранённые процессы",
 "Explanation available": "Объяснение доступно",
 "Declared trigger:": "Объявленное событие запуска:",
 "No internal process has been selected for maintained documentation yet.": "Для обновляемой документации ещё не выбран ни один внутренний процесс.",
 "Internal candidates": "Кандидаты во внутренние процессы",
 "These methods contain local calls and control flow. They are candidates for explanation, not an exhaustive list of business processes.": "Эти методы содержат локальные вызовы и управление выполнением. Это кандидаты для объяснения, а не полный список бизнес-процессов.",
 "No internal candidates could be nominated from this evidence. This does not establish that the service has no internal processes. Select an exact method when discovery is incomplete.": "По этим подтверждениям не удалось выделить внутренние процессы. Это не доказывает, что их нет. При неполном поиске выберите конкретный метод.",
 "additional internal candidates are available in the retained catalogue.": "дополнительных кандидатов доступны в сохранённом каталоге.",
 "Separate trigger candidates:": "Отдельные кандидаты в события запуска:",
 ". Methods with missing or limited flow:": ". Методы с отсутствующими или неполными данными о выполнении:",
 "Browse the full retained catalogue": "Открыть полный сохранённый каталог",
 "This command reads saved evidence and does not invoke capture or a model.": "Эта команда читает сохранённые данные без повторного сбора и вызова модели.",
 "Use the returned cursor for subsequent pages; use --lane trigger to inspect trigger candidates separately. An explicit --snapshot selects a saved version.": "Для следующих страниц используйте возвращённый курсор. Параметр --lane trigger отдельно показывает кандидатов в события запуска. Параметр --snapshot выбирает сохранённую версию.",
 "Discovery limits": "Ограничения поиска",
 "SAVED PROCESS": "СОХРАНЁННЫЙ ПРОЦЕСС",
 "This captured definition or a linked child has changed. Retained explanations need review.": "Сохранённое описание или связанный дочерний процесс изменились. Объяснения нужно проверить.",
 "Explicit requested scope": "Явно заданная область описания",
 "Trigger:": "Событие запуска:",
 "Participants:": "Участники:",
 "Domain objects:": "Предметные объекты:",
 "Requested outcomes": "Ожидаемые результаты",
 "These are human-requested scope and outcomes; source support is assessed separately.": "Эту область и результаты задал человек; подтверждение кодом оценивается отдельно.",
 "No overview has been accepted.": "Обзор ещё не принят.",
 "Overview evidence": "Подтверждения обзора",
 "Open detailed flow →": "Открыть подробный порядок действий →",
 "Linked child views · captured versions": "Связанные дочерние схемы · сохранённые версии",
 "Accepted child version and influence": "Принятая дочерняя версия и её зависимости",
 "No current accepted explanation is available for composition.": "Актуальное принятое объяснение для включения в общую схему отсутствует.",
 "No linked child views were selected.": "Связанные дочерние схемы не выбраны.",
 "Current entity declarations": "Текущие объявления сущностей",
 "Domain identity:": "Предметная сущность:",
 "Human declaration": "Объявлено человеком",
 "Agent proposal": "Предложено агентом",
 "Implementation representations": "Представления в реализации",
 "Entity evidence": "Подтверждения сущности",
 "Some declared evidence references are unavailable; inspect the entity record.": "Часть объявленных подтверждений недоступна; проверьте запись сущности.",
 "No domain entity identities have been declared. Classes and DTOs alone do not establish domain ownership.": "Предметные сущности не объявлены. Классы и DTO сами по себе не определяют, кто за них отвечает.",
 "Discovered public boundaries": "Найденные публичные интерфейсы",
 "Declared OpenAPI operations": "Операции, объявленные в OpenAPI",
 "Declaration": "Объявление",
 "Internal processes and selected diagrams": "Внутренние процессы и выбранные диаграммы",
 "No typed process diagram has been accepted in this publication.": "В этой публикации ещё не принята структурированная диаграмма процесса.",
 "Browse maintained processes and structural candidates with their authoring status. A local fragment is not automatically an entry-to-outgoing-call scenario.": "Просмотрите обновляемые процессы и найденных кандидатов вместе с состоянием их описания. Локальный фрагмент сам по себе не является сценарием от входа до внешнего вызова.",
 "Explore internal processes →": "Просмотреть внутренние процессы →",
 "Source inventory and coverage": "Список кода и полнота анализа",
 " discovered": " найдено",
 " documented": " описано",
 "entrypoints": "точек входа",
 "explicit gaps.": "явных пробелов.",
 "discovered public boundaries": "найденных публичных интерфейсов",
 "internal callable evidence records": "записей о внутренних методах",
 "Internal callable inventory does not establish public API exposure.": "Список внутренних методов не доказывает их доступность через публичный API.",
 "Public boundary discovery covers the selected evidence modules.": "Поиск публичных интерфейсов охватывает выбранные модули с подтверждениями.",
 "Dynamic registration and runtime activation have not been verified.": "Динамическая регистрация и фактическая активация не проверены.",
 "Source evidence is unavailable.": "Подтверждающий код недоступен.",
 "Declared API operation; source behavior is not established.": "Операция API объявлена; поведение по коду не установлено.",
 "This entrypoint has no behavior narrative yet.": "Для этой точки входа ещё нет описания поведения.",
 "Inspect →": "Подробнее →",
 "Operation view": "Представление операции",
 "Contract": "Контракт",
 "Findings": "Замечания",
 "SOURCE AND DECLARED SCOPE": "КОД И ОБЪЯВЛЕННАЯ ОБЛАСТЬ АНАЛИЗА",
 "Trigger": "Событие запуска",
 "Operation": "Операция",
 "Coverage": "Полнота анализа",
 "Declared contract; source mapping unresolved": "Контракт объявлен; связь с кодом не установлена",
 "Documented": "Описано",
 "Awaiting authoring": "Ожидает описания",
 "COVERAGE AND AUTHORITY": "ПОЛНОТА АНАЛИЗА И ОСНОВАНИЯ ВЫВОДОВ",
 "What the sources support": "Что подтверждено исходным кодом",
 "Freshness is scoped to the recorded dependencies. Runtime routing and wire compatibility remain unverified.": "Актуальность оценивается по записанным зависимостям. Фактическая маршрутизация и совместимость форматов не проверены.",
 "entrypoints in scope": "точек входа в области анализа",
 "documented operations": "описанных операций",
 "retained source fragments": "сохранённых фрагментов кода",
 "Declared service interactions": "Объявленные взаимодействия сервисов",
 "origin:": "происхождение:",
 "runtime:": "выполнение:",
 "Checks and boundaries": "Проверки и ограничения",
 "No service interactions selected for this page.": "Для этой страницы взаимодействия сервисов не выбраны.",
 "Revision vector and evidence scope": "Версии репозиториев и область подтверждений",
 "The highlighted fragment is exact retained source. The step narrative is an agent interpretation.": "Выделен точный сохранённый фрагмент кода. Описание шага — объяснение агента.",
 "Source binding": "Привязка к исходному коду",
 "Text:": "Текст:",
 "Evidence:": "Подтверждение:",
 "Source copied": "Исходный код скопирован",
 "Select the source text and copy it manually.": "Выделите исходный код и скопируйте его вручную.",
 "MICROSERVICE": "МИКРОСЕРВИС",
 "INTERACTION SCENARIO": "СЦЕНАРИЙ ВЗАИМОДЕЙСТВИЯ"
};
Object.assign(RU_MESSAGES,{
 'Translation pending':'Перевод ещё не подготовлен',
 'The description is not yet available in the selected language.':'Описание на выбранном языке ещё не подготовлено.',
 'The accepted text and diagrams belong to another language or have no recorded language. They are hidden here until a matching version is accepted.':'Принятые текст и диаграммы относятся к другому языку или их язык не указан. Здесь они скрыты до принятия версии на выбранном языке.',
 'English version':'Английская версия','Russian version':'Русская версия','Original version':'Исходная версия',
 'CURRENT':'Актуально','STALE':'Устарело','UNVERIFIED':'Не проверено','UNASSESSED':'Смысл не проверен','VERIFIED':'Проверено','VERIFIED_WITH_LIMITATIONS':'Проверено с ограничениями',
 'SECTION':'РАЗДЕЛ','NOTE':'ЗАМЕТКА','PROCESS':'ПРОЦЕСС','FLOW':'ПРОЦЕСС','MAP':'СВЯЗИ','RULES':'ПРАВИЛА','VIEW':'СХЕМА','CODE':'КОД',
 'required':'обязательно','optional':'необязательно','unknown':'неизвестно','unavailable':'недоступно','unspecified':'не указано','none':'нет',
 'alt':'условие','else':'иначе','loop':'цикл','opt':'необязательный шаг',
 'read':'чтение','transform':'преобразование','write':'запись','transfer':'передача','candidate':'кандидат',
 'domain':'предметная сущность','dto':'DTO','message':'сообщение','table':'таблица','field':'поле','function':'функция'
});
const I18N={en:Object.fromEntries(Object.keys(RU_MESSAGES).map(key=>[key,key])),ru:RU_MESSAGES};
const t=key=>I18N[language][key]??key;
const chromePattern=new RegExp('(?<![\\p{L}\\p{N}_])(?:'+Object.keys(RU_MESSAGES).filter(key=>!['required','optional','unknown','unavailable','unspecified','none','alt','else','loop','opt','read','transform','write','transfer','candidate','domain','dto','message','table','field','function','CURRENT','STALE','UNVERIFIED','UNASSESSED','VERIFIED','VERIFIED_WITH_LIMITATIONS','SECTION','NOTE','PROCESS','FLOW','MAP','RULES','VIEW','CODE'].includes(key)).sort((a,b)=>b.length-a.length).map(key=>key.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')).join('|')+')(?![\\p{L}\\p{N}_])','gu');
function statusLabel(value){return t(value);}
function chromeHtml(parts,...values){return parts.reduce((out,part,i)=>out+(language==='ru'?part.replace(chromePattern,key=>t(key)):part)+(i<values.length?values[i]:''),'');}
const explicitLanguage=['en','ru'].includes(publication.requestedDocumentationLanguage);
const translationGaps=explicitLanguage?(publication.translationGaps||{}):{};
const sectionTitles={'section-overview':language==='ru'?'Обзор':'Overview','section-responsibilities':t('Responsibilities'),'section-entities':t('Domain entities'),'section-ingress':t('Entry points'),'section-egress':t('External calls and storage')};
// Keep the embedded publication immutable: accepted digests refer to its exact content.
// The display projection excludes mismatched authored prose and visual interpretations.
const D={...publication,operations:publication.operations.filter(op=>!translationGaps[op.id]),sections:(publication.sections||[]).map(section=>translationGaps[section.id]?{...section,title:sectionTitles[section.id]||section.id,content:null,gap:t('The description is not yet available in the selected language.')}:section)};

const $=id=>document.getElementById(id);
const esc=v=>String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
function translationNotice(id){
 const gap=translationGaps[id];if(!gap)return '';
 const href=typeof gap.href==='string'&&!/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(gap.href)?gap.href:null;
 const label=gap.availableLanguage==='en'?t('English version'):gap.availableLanguage==='ru'?t('Russian version'):t('Original version');
 return `<aside class="gap-card translation-gap" data-translation-gap="${esc(id)}"><h3>${esc(t('Translation pending'))}</h3><p>${esc(t('The description is not yet available in the selected language.'))}</p><p>${esc(t('The accepted text and diagrams belong to another language or have no recorded language. They are hidden here until a matching version is accepted.'))}</p>${href?`<p><a href="${esc(href)}">${esc(label)} ↗</a></p>`:''}</aside>`;
}
const pretty=v=>esc(JSON.stringify(v,null,2));
const sourceRecords=()=>sourceScope==='current'?D.sources:sourceOperation?(D.operationSources?.[sourceOperation]||{}):!$('scenario-content').hidden&&(D.operationSources?.[sourceOperation||current?.visualOwner||current?.id])||D.sources;
const button=(ids,label=t('Source'),currentSnapshot=false)=>ids?.length?chromeHtml`<button class="source-link" ${currentSnapshot?'data-current-sources':''} data-sources="${esc(ids.join(' '))}">${esc(label)} ↗</button>`:'';
// Typed artifacts are interpretations bound to their containing accepted operation.
// IDs encode each code point so user identifiers cannot collide or inject markup.
const visualToken=value=>Array.from(String(value),c=>c.codePointAt(0).toString(16)).join('_');
const visualKey=(owner,id)=>chromeHtml`visual-${visualToken(owner)}--${visualToken(id)}`;
const visualRows=()=>D.operations.flatMap(owner=>(owner.visuals||[]).map(visual=>({owner,visual})));
const visualKind=kind=>({'execution-flow':t('Execution flow'),'dependency-map':t('Dependency map'),'decision-table':t('Decision table')}[kind]||t('Visual artifact'));
const visualNodeKey=(owner,visual,node)=>chromeHtml`${visualKey(owner,visual)}-node-${visualToken(node)}`;
const visualLink=(owner,visual,text,node)=>chromeHtml`<a href="#${visualKey(owner,visual)}" data-visual-target="${visualKey(owner,visual)}" ${node?chromeHtml`data-visual-node-target="${visualNodeKey(owner,visual,node)}"`:''}>${esc(text)}</a>`;
function visualFragment(owner,fragment){
 if(!fragment)return '';
 return chromeHtml`<span>${esc(fragment.text)}</span>${fragment.sourceIds?.length?chromeHtml` <button class="source-link" data-sources="${esc(fragment.sourceIds.join(' '))}" data-source-operation="${esc(owner.id)}">Supporting code ↗</button>`:''}`;
}
function visualStatus(owner){
 const state=D.operationStates?.[owner.id];
 return chromeHtml`<p class="visual-status">Source freshness: <b>${esc(statusLabel(state?.freshness||'UNVERIFIED'))}</b> · Meaning review: <b>${esc(statusLabel(state?.verification||'UNASSESSED'))}</b></p>`;
}
function visualGraph(owner,visual){
 const nodes=visual.nodes||[],edges=visual.edges||[],byId=new Map(nodes.map(n=>[n.id,n]));
 if(!nodes.length)return chromeHtml`<p class="empty-note">No graph nodes were supplied.</p>`;
 // Rank acyclic portions by their actual edges, not declaration order. Cycles
 // remain directed edges; placement itself never claims execution order.
 const incoming=new Map(nodes.map(n=>[n.id,0])),rank=new Map();
 for(const edge of edges)if(byId.has(edge.from)&&byId.has(edge.to)&&edge.from!==edge.to)incoming.set(edge.to,incoming.get(edge.to)+1);
 const queue=nodes.filter(n=>incoming.get(n.id)===0).map(n=>n.id);
 for(const id of queue)rank.set(id,0);
 for(let i=0;i<queue.length;i++)for(const edge of edges.filter(e=>e.from===queue[i]&&e.to!==e.from&&byId.has(e.to))){rank.set(edge.to,Math.max(rank.get(edge.to)||0,rank.get(edge.from)+1));incoming.set(edge.to,incoming.get(edge.to)-1);if(incoming.get(edge.to)===0)queue.push(edge.to);}
 let tail=Math.max(-1,...queue.map(id=>rank.get(id)))+1;
 for(const node of nodes)if(!queue.includes(node.id))rank.set(node.id,tail++);
 const rows=new Map();for(const node of nodes){const r=rank.get(node.id);if(!rows.has(r))rows.set(r,[]);rows.get(r).push(node);}
 const width=Math.max(760,Math.max(...Array.from(rows.values(),r=>r.length))*320+140),positions=new Map();let y=24;
 for(const [,row] of [...rows].sort((a,b)=>a[0]-b[0])){
  const height=Math.max(...row.map(n=>Math.max(88,wrap(n.meaning.text,32).length*18+44+(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===n.id).length*18)));
  row.forEach((node,i)=>positions.set(node.id,{x:(width-row.length*320)/2+i*320+20,y,w:280,h:height}));y+=height+135;
 }
 const marker=visualKey(owner.id,visual.id)+'-arrow';
 let svg=chromeHtml`<svg class="artifact-svg" viewBox="0 0 ${width} ${y}" style="min-width:${Math.min(width,900)}px" role="img" aria-label="${esc(visualKind(visual.kind)+': '+visual.title)}"><title>${esc(visual.title)}</title><desc>Arrows carry the documented relationships. Node position alone does not establish order. A complete node and connection table follows.</desc><defs><marker id="${marker}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#376d83"/></marker></defs>`;
 edges.forEach((edge,i)=>{
  const a=positions.get(edge.from),b=positions.get(edge.to);if(!a||!b)return;
  let path,x,y;
  if(b.y>a.y){const x1=a.x+a.w/2,x2=b.x+b.w/2,bus=a.y+a.h+35+(i%3)*22;path=chromeHtml`M${x1} ${a.y+a.h} V${bus} H${x2} V${b.y-3}`;x=(x1+x2)/2+10;y=bus+15;}
  else{const lane=width-15-(i%4)*16;path=chromeHtml`M${a.x+a.w} ${a.y+a.h/2} H${lane} V${b.y+22} H${b.x+b.w+3}`;x=lane-10;y=(a.y+a.h/2+b.y+22)/2;}
  const lines=wrap(chromeHtml`${i+1}. ${edge.meaning.text}`,36),shown=lines.slice(0,3);if(lines.length>3)shown[2]+='…';
  svg+=chromeHtml`<g><title>${esc(edge.meaning.text)}</title><path d="${path}" class="artifact-edge" marker-end="url(#${marker})"/>${shown.map((line,j)=>chromeHtml`<text class="artifact-edge-label" x="${x}" y="${y+j*15}" text-anchor="${b.y>a.y?'start':'end'}">${esc(line)}</text>`).join('')}</g>`;
 });
 for(const node of nodes){
  const p=positions.get(node.id),decisions=(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===node.id),nodeId=visualNodeKey(owner.id,visual.id,node.id);
  svg+=chromeHtml`<g id="${nodeId}" tabindex="-1"><title>${esc(node.meaning.text)}</title><rect x="${p.x}" y="${p.y}" width="${p.w}" height="${p.h}" rx="9"/>${wrap(node.meaning.text,32).map((line,i)=>chromeHtml`<text x="${p.x+14}" y="${p.y+25+i*18}">${esc(line)}</text>`).join('')}${decisions.map((decision,i)=>chromeHtml`<a href="#${visualKey(owner.id,decision.id)}" data-visual-target="${visualKey(owner.id,decision.id)}"><title>${esc(decision.title)}</title><text class="artifact-decision-link" x="${p.x+14}" y="${p.y+p.h-12-(decisions.length-1-i)*18}">${esc(decision.title.length>31?decision.title.slice(0,30)+'…':decision.title)} →</text></a>`).join('')}</g>`;
 }
 return chromeHtml`<div class="artifact-graph-scroll">${svg}</svg></div><p class="flow-note">${visual.kind==='dependency-map'?t('Arrows show dependencies, not execution order or runtime calls.'):t('Follow the arrows and their conditions. Placement alone does not establish order; this is a source-based interpretation, not an observed runtime trace.')}</p><details class="artifact-text"><summary>Complete diagram as text and supporting code</summary><h4>Nodes</h4><ul>${nodes.map(n=>chromeHtml`<li>${visualFragment(owner,n.meaning)}${(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===n.id).map(v=>' · '+visualLink(owner.id,v.id,v.title)).join('')}</li>`).join('')}</ul><h4>Directed connections</h4><ol>${edges.map(e=>chromeHtml`<li><p><b>${esc(byId.get(e.from)?.meaning.text||e.from)} → ${esc(byId.get(e.to)?.meaning.text||e.to)}</b></p>${visualFragment(owner,e.meaning)}</li>`).join('')}</ol></details>`;
}
function visualCard(owner,visual){
 const parent=visual.parent&&(owner.visuals||[]).find(v=>v.id===visual.parent.artifact),node=parent?.nodes?.find(n=>n.id===visual.parent.node);
 const body=visual.kind==='decision-table'?chromeHtml`${parent?chromeHtml`<p class="visual-parent">Used at ${visualLink(owner.id,parent.id,node?.meaning.text||parent.title,visual.parent.node)} in ${esc(parent.title)}.</p>`:chromeHtml`<p class="visual-parent"><b>Local decision; placement in the wider process is not established.</b></p>`}<p class="decision-policy">${visual.hitPolicy==='FIRST'?t('Choose the first matching row, top to bottom. Later matching rows do not contribute another result.'):visual.hitPolicy==='UNIQUE'?t('At most one row may match. Multiple matching rows mean the documented decision rules conflict.'):t('The rule matching policy has not been established. Do not infer priority or exclusivity from row order.')}</p>${visual.policyExplanation?chromeHtml`<p><b>Policy interpretation:</b> ${visualFragment(owner,visual.policyExplanation)}</p>`:''}<p class="flow-note">A row selects a result; this table does not establish when its resulting actions execute.</p><details><summary>Decision notation</summary><p>Hit policy: <b>${esc(visual.hitPolicy||t('unspecified'))}</b>. This is an authored policy interpretation, not a formally verified or executable DMN model.</p></details><div class="table-wrap"><table><thead><tr><th>Rule</th><th>Condition</th><th>Selected result</th></tr></thead><tbody>${(visual.rules||[]).map((r,i)=>chromeHtml`<tr><td>${i+1}</td><td>${visualFragment(owner,r.condition)}</td><td>${visualFragment(owner,r.outcome)}</td></tr>`).join('')}</tbody></table></div>${visual.afterSelection?chromeHtml`<section class="visual-after-selection"><h4>After selection: execution and outcomes</h4><p>${visualFragment(owner,visual.afterSelection)}</p></section>`:''}`:visualGraph(owner,visual);
 return chromeHtml`<article class="visual-artifact" id="${visualKey(owner.id,visual.id)}"><div class="eyebrow">${esc(visualKind(visual.kind))}</div><h3>${esc(visual.title)}</h3>${visualStatus(owner)}<p><b>Why this view:</b> ${visualFragment(owner,visual.purpose)}</p><p><b>Scope:</b> ${visualFragment(owner,visual.scope)}</p>${body}${visual.limitations?.length?chromeHtml`<div class="visual-limits"><h4>Limits of this view</h4><ul>${visual.limitations.map(v=>chromeHtml`<li>${esc(v)}</li>`).join('')}</ul></div>`:''}<details class="technical-evidence"><summary>Artifact binding and producer</summary><p>Accepted with ${esc(owner.title)} (${esc(owner.id)}). Freshness and meaning review belong to this containing accepted version.</p><pre>${pretty({schema:visual.schema,generator:visual.generator,artifact:visual.id,operation:owner.id,state:D.operationStates?.[owner.id]||null})}</pre></details></article>`;
}
function visualGallery(){const rows=visualRows();return rows.length?chromeHtml`<section class="visual-gallery"><h2>Processes and diagrams · ${rows.length}</h2><p>Selected source-based views. Each view records its own purpose, scope and limitations; these are not an exhaustive list of service behavior.</p>${rows.map(({owner,visual})=>visualCard(owner,visual)).join('')}</section>`:'';}
function visualPreview(){const rows=visualRows(),first=rows.find(r=>r.visual.kind!=='decision-table')||rows[0];return first?chromeHtml`<section class="visual-preview"><h3>Processes and diagrams</h3><nav aria-label="Processes and diagrams">${rows.map(({owner,visual})=>visualLink(owner.id,visual.id,visual.title)).join('')}</nav>${visualCard(first.owner,first.visual)}</section>`:'';}

const serviceSectionOrder=['section-overview','section-responsibilities','section-entities','section-ingress','section-egress'];
function serviceProfile(){
 if(!D.subject.startsWith('service:'))return '';
 const definitions=[['section-responsibilities',t('Responsibilities')],['section-entities',t('Domain entities')],['section-ingress',t('Entry points')],['section-egress',t('External calls and storage')]];
 return chromeHtml`<section class="service-profile" aria-label="Service profile">${definitions.map(([id,title])=>{
  const section=(D.sections||[]).find(s=>s.id===id),owner=D.operations.find(o=>o.id===id),accepted=owner?.summary;
  let detail='';
  if(id==='section-entities'){
   const entities=(D.entities||[]).filter(d=>d.normalized.entity.relations.some(r=>r.service===D.subject.slice(8)));
   detail=chromeHtml`<p class="profile-note">DTOs and implementation classes alone do not establish entity ownership or creation.</p>${entities.length?chromeHtml`<p>Declared domain identities: ${entities.map(d=>esc(d.normalized.entity.title)).join(', ')}.</p>`:chromeHtml`<p class="profile-gap">Entity ownership and creation roles have no explicit domain declarations in this publication.</p>`}`;
  }
  if(id==='section-ingress'){
   const known=D.boundaryInventory?.publicBoundaries||[];
   detail=known.length?chromeHtml`<ul class="profile-entry-list">${known.map(entry=>chromeHtml`<li><button data-entry="${esc(entry.id)}">${esc((entry.trigger.methods||[]).join(' / '))} ${esc((entry.trigger.paths||[]).join(' · ')||entry.symbol)}</button><span>${D.operations.some(o=>o.id===entry.id)?t('Behavior narrative available'):t('Behavior narrative not yet accepted')}</span></li>`).join('')}</ul>`:chromeHtml`<p class="profile-gap">No public entries are recorded in this publication. This does not establish that the service has no entry points.</p>`;
   detail+=chromeHtml`<details class="profile-discovery"><summary>Discovery scope</summary><p>The inventory is bounded by retained analyzer evidence. Per-transport completeness for HTTP, messaging, schedules and CLI is not established here; absent entries are not proof of absence.</p></details>`;
  }
  if(id==='section-egress')detail=chromeHtml`<p class="profile-note">This is the accepted description of external boundaries. Thread-to-call links and complete outgoing coverage are not inferred from a dependency list.</p>`;
  return chromeHtml`<article class="profile-section" id="profile-${esc(id)}"><h3>${esc(title)}</h3>${translationGaps[id]?translationNotice(id):accepted?chromeHtml`<p class="profile-summary">${visualFragment(owner,accepted)}</p>${visualStatus(owner)}${owner.boundaries?.length?chromeHtml`<details class="profile-discovery"><summary>Scope and limitations</summary><ul>${owner.boundaries.map(text=>chromeHtml`<li>${esc(text)}</li>`).join('')}</ul></details>`:''}`:chromeHtml`<p class="profile-gap">${esc(section?.gap||t('No source-bound section summary has been accepted in this publication.'))}</p>`}${detail}${section?chromeHtml`<p><button class="source-link" data-entry="${esc(id)}">Open ${esc(title.toLowerCase())} details →</button></p>`:''}</article>`;
 }).join('')}</section>`;
}

const entries=D.catalogue.length?[...D.catalogue]:publication.operations.filter(o=>!o.id.startsWith('section-')&&!o.id.startsWith('assessment-')&&o.id!=='process-overview'&&!o.dataflow&&!D.process).map(o=>({id:o.id,symbol:translationGaps[o.id]?o.id:o.title,kind:'SCENARIO',trigger:{methods:['FLOW'],paths:[]},sourceIds:[]}));
if(D.subject.startsWith('service:'))entries.push(...D.contracts.filter(c=>!c.normalized.entrypoint).map(c=>({id:c.id,symbol:t('Declared OpenAPI · ')+c.normalized.declaredSource,kind:'DECLARED_OPENAPI',trigger:{methods:[c.normalized.method],paths:[c.normalized.path]},sourceIds:c.sourceIds})));
entries.unshift(...[...(D.sections||[])].sort((a,b)=>{const rank=id=>serviceSectionOrder.indexOf(id)<0?serviceSectionOrder.length:serviceSectionOrder.indexOf(id);return rank(a.id)-rank(b.id);}).map(s=>({id:s.id,symbol:s.title,kind:'SECTION',trigger:{methods:['SECTION'],paths:[]},sourceIds:s.content?.summary.sourceIds||[]})));
entries.push(...(D.notes||[]).map(n=>({id:n.id,symbol:n.association.title,kind:'NOTE',searchText:JSON.stringify(n.association)+' '+(n.original.text||''),trigger:{methods:['NOTE'],paths:[]},sourceIds:[]})));
if(D.process){entries.unshift({id:'process-overview',symbol:t('Process overview'),kind:'PROCESS',trigger:{methods:['PROCESS'],paths:[]},sourceIds:[]});}
if(D.view){entries.unshift({id:'entity-dataflow',symbol:t('Entity data flow'),kind:'VIEW',trigger:{methods:['VIEW'],paths:[]},sourceIds:[]});if(!entries.some(e=>e.id===D.view.definition.id))entries.push({id:D.view.definition.id,symbol:t('Source flow scope'),kind:'SCENARIO',trigger:{methods:['FLOW'],paths:[]},sourceIds:[]});}
if(D.subject.startsWith('service:'))entries.splice((D.sections||[]).length,0,{id:'process-catalog',symbol:t('Internal processes'),kind:'PROCESS_CATALOG',trigger:{methods:['PROCESS'],paths:[]},sourceIds:[]});
entries.splice((D.sections||[]).length+(D.subject.startsWith('service:')?1:0),0,...visualRows().map(({owner,visual})=>({id:visualKey(owner.id,visual.id),visualOwner:owner.id,visualId:visual.id,symbol:visual.title,kind:'VISUAL',searchText:visual.purpose?.text+' '+visual.scope?.text,trigger:{methods:[{'execution-flow':'FLOW','dependency-map':'MAP','decision-table':'RULES'}[visual.kind]||'VIEW'],paths:[]},sourceIds:[]})));
const inventoryEntries=()=>entries.filter(e=>!['SECTION','NOTE','PROCESS','VIEW','PROCESS_CATALOG','VISUAL'].includes(e.kind));
const documentedCount=()=>D.operations.filter(o=>!o.id.startsWith('section-')&&!o.id.startsWith('assessment-')&&o.id!=='process-overview'&&!o.dataflow).length;
let sourceScope='',sourceOperation='';
let current=entries[0],tab='sequence',sourceId='',lastFocus=null,contractId='',payloadId='';
const operation=e=>D.operations.find(o=>o.id===(e.visualOwner||e.id));
const label=e=>(['NOTE','VISUAL'].includes(e.kind)?e.symbol:operation(e)?.title)||e.trigger.paths?.join(' · ')||e.symbol;
function relatedViews(entity){const rows=(D.relatedViews||[]).filter(v=>!entity||v.inputObjects.includes(entity));return rows.length?chromeHtml`<section class="gap-card"><h3>Saved entity views</h3>${rows.map(v=>chromeHtml`<p><a href="../scenarios/${esc(v.id)}.html#entity-dataflow">${esc(v.title)} ↗</a></p>`).join('')}</section>`:'';}
function dataflowDiagram(g,layout={}){
 const positions=Object.fromEntries(g.nodes.map((n,i)=>[n.id,{column:layout[n.id]?.column??i%3,row:layout[n.id]?.row??Math.floor(i/3)}]));
 const W=Math.max(900,(Math.max(...Object.values(positions).map(p=>p.column))+1)*410),H=Math.max(250,(Math.max(...Object.values(positions).map(p=>p.row))+1)*280);
 const point=id=>({x:30+positions[id].column*410,y:45+positions[id].row*280});
 let svg=chromeHtml`<svg viewBox="0 0 ${W} ${H}" style="min-width:${W}px" role="img" aria-label="Static entity data flow; an accessible edge table follows"><defs><marker id="flow-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#608365"/></marker></defs>`;
 for(const e of g.edges){
  const a=point(e.from),b=point(e.to),dx=b.x-a.x,dy=b.y-a.y,length=Math.hypot(dx,dy);let path,mx,my;
  if(length===0){path=chromeHtml`M${a.x+255} ${a.y+42} C${a.x+350} ${a.y-25},${a.x+350} ${a.y+172},${a.x+255} ${a.y+105}`;mx=a.x+326;my=a.y+75;}
  else if(dy===0&&Math.abs(dx)>410){mx=(a.x+b.x)/2+125;my=a.y+205;path=chromeHtml`M${a.x+125} ${a.y+146} L${a.x+125} ${my} L${b.x+125} ${my} L${b.x+125} ${b.y+146}`;}
  else {const k=Math.min(125/Math.abs(dx||.0001),70/Math.abs(dy||.0001)),x1=a.x+125+dx*k+dx/length*6,y1=a.y+70+dy*k+dy/length*6,x2=b.x+125-dx*k-dx/length*6,y2=b.y+70-dy*k-dy/length*6;path=chromeHtml`M${x1} ${y1} L${x2} ${y2}`;mx=(x1+x2)/2;my=(y1+y2)/2;}
  const label=e.authority==='UNKNOWN'?t('Unknown candidate'):e.authority==='DECLARED_TRANSFER'?t('Declared transfer'):t(e.kind);
  svg+=chromeHtml`<a href="#entity-dataflow" data-view-edge="${esc(e.id)}" aria-label="${esc(label+': '+e.meaning.text)}"><title>${esc(e.meaning.text)}</title><path d="${path}" fill="none" stroke="transparent" stroke-width="18"/><path d="${path}" fill="none" stroke="#608365" stroke-width="2" ${e.authority==='UNKNOWN'?'stroke-dasharray="6 5"':''} marker-end="url(#flow-arrow)"/><rect x="${mx-74}" y="${my-30}" width="148" height="24" rx="4" fill="#fcfdfb"/><text x="${mx}" y="${my-14}" text-anchor="middle" fill="#425c47" font-size="11">${esc(label)}</text></a>`;
 }
 for(const n of g.nodes){
  const {x,y}=point(n.id),lines=wrap(n.meaning.text,29),shown=lines.slice(0,4),linked=n.meaning.sourceIds.length>0;if(lines.length>4)shown[3]=shown[3].slice(0,26)+'…';
  svg+=chromeHtml`<${linked?'a':'g'} ${linked?chromeHtml`href="#entity-dataflow" data-view-node="${esc(n.id)}"`:''} aria-label="${esc(t(n.kind)+': '+n.meaning.text)}"><title>${esc(t(n.kind)+' · '+n.service+' · '+n.entity)}</title><rect x="${x}" y="${y}" width="250" height="140" rx="8" fill="${n.kind==='domain'?'#eff4dc':'#f2f6ef'}" stroke="#d0ddc7"/><text x="${x+14}" y="${y+22}" fill="#6f7e6b" font-size="10">${esc(t(n.kind).toUpperCase()+' · '+n.service.slice(0,25))}</text>${shown.map((t,i)=>chromeHtml`<text x="${x+14}" y="${y+46+i*16}" fill="#263d2d" font-size="12">${esc(t)}</text>`).join('')}<text x="${x+14}" y="${y+124}" fill="#6f7e6b" font-size="9">${esc(n.entity.length>38?n.entity.slice(0,35)+'…':n.entity)}</text></${linked?'a':'g'}>`;
 }
 return svg+chromeHtml`</svg>`;
}
function relatedNotes(target){const rows=(D.notes||[]).filter(n=>n.association.targets.includes(target));return rows.length?chromeHtml`<section class="gap-card"><h3>Related human notes</h3>${rows.map(n=>chromeHtml`<p><button data-entry="${esc(n.id)}">${esc(n.association.title)}</button> · ${esc(n.association.classification)}</p>`).join('')}</section>`:'';}
const method=e=>e.kind.startsWith('SOURCE_')?'CODE':(e.trigger.methods||[e.kind]).join(' / ');
const chip=m=>chromeHtml`<span class="http-method ${esc(m.toLowerCase())}">${esc(t(m))}</span>`;
function nav(){const q=$('search').value.toLowerCase().trim();$('scenario-nav').innerHTML=entries.filter(e=>[label(e),e.symbol,JSON.stringify(e.trigger),e.searchText||''].join(' ').toLowerCase().includes(q)).map(e=>chromeHtml`<button class="scenario-link" data-entry="${esc(e.id)}" ${current?.id===e.id?'aria-current="page"':''}><span class="nav-icon">${esc(t(method(e)))}</span><span>${esc(label(e))}</span>${!operation(e)?chromeHtml`<span class="nav-issue" title="Explicit documentation gap"></span>`:''}</button>`).join('')||chromeHtml`<p class="no-results">No matching operations</p>`;}
function wrap(text,max=28){const out=[''];for(const word of text.split(' ')){const i=out.length-1;if(out[i]&&out[i].length+word.length+1>max)out.push(word);else out[i]+=(out[i]?' ':'')+word;}return out;}
function diagram(o){
 const W=Math.max(800,(o.participants.length-1)*215+240);const xs=Object.fromEntries(o.participants.map((p,i)=>[p.id,85+i*(W-325)/Math.max(1,o.participants.length-1)]));let y=94,stack=[],frames=[],rows=[];
 for(const e of o.events){const h=['alt','else','loop','opt'].includes(e.kind)?40:e.kind==='end'?18:e.kind==='note'?65:72;rows.push({e,y,h,depth:stack.length});if(e.kind==='alt'||e.kind==='loop'||e.kind==='opt')stack.push({y,depth:stack.length});if(e.kind==='end'){const f=stack.pop();frames.push({...f,end:y+h-5});}y+=h;}
 const H=y+25;
 let svg=chromeHtml`<svg class="sequence-svg" style="min-width:${W}px" viewBox="0 0 ${W} ${H}" role="group" aria-label="Sequence diagram: ${esc(o.title)}"><defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0 L8 4 L0 8Z" fill="#608365"/></marker></defs>`;
 svg+=frames.map(f=>chromeHtml`<rect x="${20+f.depth*12}" y="${f.y-8}" width="${W-40-f.depth*24}" height="${f.end-f.y+8}" rx="4" fill="#fdfbf6" stroke="#e3d9c5"/>`).join('');
 svg+=o.participants.map(p=>chromeHtml`<line class="lifeline" x1="${xs[p.id]}" x2="${xs[p.id]}" y1="61" y2="${H-8}"/><rect x="${xs[p.id]-76}" y="9" width="152" height="48" rx="6" fill="#f1f6ed" stroke="#d6e1d0"/><text class="actor-label" text-anchor="middle" x="${xs[p.id]}" y="29">${esc(p.label)}</text><text class="actor-technical" text-anchor="middle" x="${xs[p.id]}" y="44">${esc(p.service||t('External participant'))}</text>`).join('');
 let number=0;
 for(const {e,y,h,depth} of rows){if(e.kind==='end')continue;number++;let inside='';
 if(['alt','else','loop','opt'].includes(e.kind)){const left=32+Math.max(0,depth-(e.kind==='else'?1:0))*12;inside=chromeHtml`${e.kind==='else'?chromeHtml`<line x1="${left-12}" y1="${y-8}" x2="${W-left+12}" y2="${y-8}" stroke="#ddd2bd" stroke-dasharray="4 3"/>`:''}<rect class="hit-area" x="${left-5}" y="${y-5}" width="${W-left*2+10}" height="31" rx="3" fill="transparent"/><text class="branch-title" x="${left}" y="${y+13}">${esc(t(e.kind))} · ${esc(e.text)}</text>`;}
 else if(e.kind==='note'){const x=xs[e.from]??W/2;inside=chromeHtml`<rect class="hit-area" x="${x-120}" y="${y-8}" width="240" height="55" rx="4" fill="#f5f2e5"/>`+wrap(e.text,33).map((line,i)=>chromeHtml`<text x="${x-110}" y="${y+9+i*14}">${esc(line)}</text>`).join('');}
 else{const a=xs[e.from],b=xs[e.to],left=Math.min(a,b),width=Math.max(100,Math.abs(a-b)),mid=a===b?a+50:(a+b)/2;
 inside=chromeHtml`<rect class="hit-area" x="${left-17}" y="${y-14}" width="${width+34}" height="${h-5}" rx="4" fill="transparent"/><text class="step-number" x="${left-12}" y="${y+39}">${number}</text>`+wrap(e.text,Math.max(23,Math.floor(width/6))).map((line,i)=>chromeHtml`<text text-anchor="middle" x="${mid}" y="${y+i*14}">${esc(line)}</text>`).join('');
 const dash=['return','declared'].includes(e.kind)?'event-return':'';
 inside+=a===b?chromeHtml`<path class="event-arrow ${dash}" d="M${a} ${y+28} h85 v17 h-85" marker-end="url(#arrow)"/>`:chromeHtml`<line class="event-arrow ${dash}" x1="${a}" y1="${y+39}" x2="${b}" y2="${y+39}" marker-end="url(#arrow)"/>`;
 if(e.kind==='declared')inside+=chromeHtml`<text class="branch-title" text-anchor="middle" x="${mid}" y="${y+57}">Declared service interaction</text>`;
 }
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-event="${esc(e.id)}" aria-label="${esc(e.text)}: inspect source"><title>${esc(e.text)}</title>${inside}</a>`;
 }return svg+chromeHtml`</svg>`;
}
function explanation(o,detail=false){
 const paragraphs=new Map();
 for(const p of (o.explanation||[]).filter(p=>!!p.detail===detail)){
  const previous=paragraphs.get(p.text);
  if(previous)previous.sourceIds=[...new Set([...previous.sourceIds,...p.sourceIds])];
  else paragraphs.set(p.text,{...p,sourceIds:[...p.sourceIds]});
 }
 return paragraphs.size?chromeHtml`<section class="operation-explanation" aria-label="${detail?t('Implementation commentary'):t('What happens')}">${detail?'':chromeHtml`<h3>What happens</h3>`}${[...paragraphs.values()].map(p=>chromeHtml`<div class="explanation-paragraph"><p>${esc(p.text)}</p><div class="explanation-evidence">${button(p.sourceIds,t('Supporting code'))}</div></div>`).join('')}</section>`:'';
}
function interactionOverview(o){
 const seen=new Set(),links=o.events.filter(e=>e.kind==='declared'&&!seen.has(e.interaction)&&seen.add(e.interaction));
 if(!links.length)return '';
 const name=id=>o.participants.find(p=>p.id===id)?.label||id;
 return chromeHtml`<section class="interaction-overview" aria-label="Service links"><h3>Service links</h3><p class="schema-description">Declared connections in this scenario. Conditions are explained below; this map does not imply synchronous execution.</p><div class="interaction-links">${links.map(e=>chromeHtml`<button class="interaction-link" data-event="${esc(e.id)}"><span>${esc(name(e.from))}</span><span class="connection-arrow" aria-hidden="true">→</span><span>${esc(name(e.to))}</span><small>${esc(e.text)}</small></button>`).join('')}</div></section>`;
}
function overviewDiagram(o){
 const d=o.overviewDiagram;
 if(!d)return o.events.length<=64?diagram(o):chromeHtml`<p class="empty-note">A bounded overview diagram has not been authored. Source evidence remains available below.</p>`;
 const W=(Math.max(...d.nodes.map(n=>n.column))+1)*280+20,H=(Math.max(...d.nodes.map(n=>n.row))+1)*160+30;
 const pos=Object.fromEntries(d.nodes.map(n=>[n.id,{x:150+n.column*280,y:90+n.row*160}]));
 let svg=chromeHtml`<svg class="overview-svg" viewBox="0 0 ${W} ${H}" role="group" aria-label="Overview diagram: ${esc(o.title)}"><defs><marker id="overview-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#64806a"/></marker></defs>`;
 for(const e of d.edges){const a=pos[e.from],b=pos[e.to],dx=b.x-a.x,dy=b.y-a.y;let x1=a.x,y1=a.y,x2=b.x,y2=b.y;
 if(Math.abs(dx)>=Math.abs(dy)){x1+=Math.sign(dx)*108;x2-=Math.sign(dx)*108;}else{y1+=Math.sign(dy)*48;y2-=Math.sign(dy)*48;}
 const mx=(x1+x2)/2,my=(y1+y2)/2,vertical=Math.abs(dx)<Math.abs(dy),tx=vertical?mx+12:mx,ty=vertical?my:my-12;
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-overview-edge="${esc(e.id)}" aria-label="${esc(e.text||t('Transition'))}: inspect source"><title>${esc(e.text||t('Transition'))}</title><path d="M${x1} ${y1} L${x2} ${y2}" fill="none" stroke="#64806a" stroke-width="2" marker-end="url(#overview-arrow)"/><text class="overview-edge-label" text-anchor="${vertical?'start':'middle'}" x="${tx}" y="${ty}">${esc(e.text)}</text></a>`;
 }
 for(const n of d.nodes){const p=pos[n.id],actor=o.participants.find(a=>a.id===n.participant),lines=wrap(n.text,25);
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-overview-node="${esc(n.id)}" aria-label="${esc(n.text)}: inspect source"><title>${esc(n.text)}</title><rect x="${p.x-108}" y="${p.y-48}" width="216" height="96" rx="10" fill="#f3f7ef" stroke="#cad8c3"/>${lines.map((line,i)=>chromeHtml`<text class="overview-node-label" text-anchor="middle" x="${p.x}" y="${p.y-23+i*17}">${esc(line)}</text>`).join('')}<text class="overview-actor" text-anchor="middle" x="${p.x}" y="${p.y+34}">${esc(actor?.service||actor?.label||'')}</text></a>`;
 }
 return svg+chromeHtml`</svg>`;
}
function sequence(e,o){if(!o)return chromeHtml`<p class="empty-note">${esc(D.gaps[e.id]||t('Behavior documentation has not been authored for this entrypoint.'))}</p>`;
 const state=D.stateDiagram?chromeHtml`<div class="diagram-card"><div class="section-label"><b>State diagram</b><span>Declarative process states · transitions bound to source evidence</span></div><div class="diagram-img-wrap"><img class="diagram-img" src="../diagrams/${esc(D.stateDiagram)}.svg" alt="State diagram"></div><div class="diagram-footer"><span>States and transitions declared in scenarios/${esc(D.subject.split(':').pop())}-states.yaml</span><a download href="../diagrams/${esc(D.stateDiagram)}.puml">PlantUML ↓</a></div></div>`:'';
 const flow=chromeHtml`<div class="diagram-card"><div class="section-label"><b>Scenario overview</b><span>Select a node or connection to inspect its source</span></div><div class="overview-scroll">${overviewDiagram(o)}</div><div class="diagram-footer"><span>Source-based interpretation; declared links do not prove runtime delivery.</span><a download href="../diagrams/${esc(D.subject.replace(':','-'))}-${esc(o.id)}.mmd">Mermaid ↓</a></div></div>`;
 const accessible=o.overviewDiagram?chromeHtml`<details class="steps-accessible"><summary>Diagram as text</summary><ul>${o.overviewDiagram.nodes.map(n=>chromeHtml`<li><button data-overview-node="${esc(n.id)}">${esc(n.text)}</button></li>`).join('')}</ul><ul>${o.overviewDiagram.edges.map(e=>chromeHtml`<li>${esc(o.overviewDiagram.nodes.find(n=>n.id===e.from).text)} → ${esc(o.overviewDiagram.nodes.find(n=>n.id===e.to).text)}${e.text?' · '+esc(e.text):''}</li>`).join('')}</ul></details>`:'';
 return chromeHtml`${(o.visuals||[]).map(v=>visualCard(o,v)).join('')}${state}${flow}${accessible}${explanation(o)}${o.boundaries.length?chromeHtml`<details class="technical-evidence"><summary>Scope and evidence boundaries</summary><ul>${o.boundaries.map(b=>chromeHtml`<li>${esc(b)}</li>`).join('')}</ul></details>`:''}<details class="implementation-detail"><summary>Implementation details and source commentary</summary>${explanation(o,true)}</details><details class="technical-evidence"><summary>Declared service links</summary>${interactionOverview(o)||chromeHtml`<p>No cross-service connections selected.</p>`}</details>`;}
function interfaceContracts(e){
 const cards=operation(e)?.interfaceContracts||[];
 if(!cards.length)return '';
 const boundaries=cards.filter(c=>c.kind!=='payload'),payloads=cards.filter(c=>c.kind==='payload');
 const chosenPayload=payloads.find(c=>c.id===payloadId)||payloads[0];
 const render=c=>chromeHtml`<section class="contract-card"><h3><span class="contract-kind">${esc(c.kind.toUpperCase())}</span> ${esc(c.title)}</h3><div class="table-wrap"><table><thead><tr><th>Contract element</th><th>Value / behavior</th><th>Evidence</th></tr></thead><tbody>${c.rows.map(r=>chromeHtml`<tr><td><code>${esc(r.label)}</code></td><td class="contract-value">${esc(r.value)}</td><td>${button(r.sourceIds)}</td></tr>`).join('')}</tbody></table></div>${c.boundaries.length?chromeHtml`<p class="flow-note">${c.boundaries.map(esc).join(' ')}</p>`:''}</section>`;
 const select=(values,chosen,name,label)=>chromeHtml`<label class="contract-selector">${label} · ${values.length}<select data-contract-select="${name}" aria-label="${label}">${values.map(c=>chromeHtml`<option value="${esc(c.id)}" ${c.id===chosen.id?'selected':''}>${esc(c.title)}</option>`).join('')}</select></label>`;
 return chromeHtml`<p class="schema-description">Source-derived interface descriptions authored from the retained code. These are separate from published OpenAPI contracts and do not prove deployed wire compatibility.</p>${boundaries.map(render).join('')}${chosenPayload?chromeHtml`<details class="payload-library"><summary>Payload fields and nested types · ${payloads.length} schemas</summary>${select(payloads,chosenPayload,'payload',t('Payload schema'))}${render(chosenPayload)}</details>`:''}`;
}
function type(s){if(!s)return '—';if(s.$ref)return esc(s.$ref);if(s.type==='array')return chromeHtml`array&lt;${type(s.items)}&gt;`;return esc(s.type||(s.allOf?'allOf':s.oneOf?'oneOf':s.anyOf?'anyOf':'object'))+(s.format?chromeHtml` <span class="optional">${esc(s.format)}</span>`:'');}
function constraints(s){return Object.entries(s||{}).filter(([k])=>['format','enum','pattern','minLength','maxLength','minimum','maximum','exclusiveMinimum','exclusiveMaximum','minItems','maxItems','uniqueItems','nullable','default','readOnly','writeOnly','additionalProperties'].includes(k)).map(([k,v])=>chromeHtml`${esc(k)}: ${esc(typeof v==='object'?JSON.stringify(v):v)}`).join(chromeHtml`<br>`)||'—';}
function fields(s,depth=0){if(!s)return chromeHtml`<span class="optional">No schema declared</span>`;if(depth>6)return chromeHtml`<span class="optional">See the complete schema below for deeper fields.</span>`;
 let html='';for(const kind of ['allOf','oneOf','anyOf'])if(s[kind])html+=chromeHtml`<p class="schema-description">${kind} composition (alternatives are preserved)</p>`+s[kind].map((p,i)=>chromeHtml`<details class="contract-raw"><summary>${kind} ${i+1}</summary>${fields(p,depth+1)}</details>`).join('');
 if(s.type==='array')return html+chromeHtml`<p class="schema-description">${type(s)} · ${constraints(s)}</p>${fields(s.items,depth+1)}`;
 const properties=Object.entries(s.properties||{});if(!properties.length)return html||chromeHtml`<p class="schema-description">${type(s)} · ${constraints(s)}</p>`;
 return html+chromeHtml`<div class="table-wrap"><table><thead><tr><th>Field</th><th>Type</th><th>Required</th><th>Constraints / description</th></tr></thead><tbody>${properties.map(([name,p])=>chromeHtml`<tr><td><code>${esc(name)}</code></td><td>${type(p)}${p.properties||p.items||p.allOf||p.oneOf||p.anyOf?chromeHtml`<details class="contract-raw"><summary>Nested fields</summary>${fields(p,depth+1)}</details>`:''}</td><td class="${s.required?.includes(name)?'required':'optional'}">${s.required?.includes(name)?t('required'):t('optional')}</td><td>${constraints(p)}${p.description?chromeHtml`<div class="schema-description">${esc(p.description)}</div>`:''}</td></tr>`).join('')}</tbody></table></div>`;
}
function contentBodies(content){return Object.entries(content||{}).map(([media,value])=>chromeHtml`<p class="schema-description"><code>${esc(media)}</code></p>${fields(value.schema)}${value.example!==undefined?chromeHtml`<pre class="example-block">${pretty(value.example)}</pre><p class="example-caption">Declared OpenAPI example; not an observed request.</p>`:''}${value.examples?chromeHtml`<details class="contract-raw"><summary>Declared examples</summary><pre>${pretty(value.examples)}</pre></details>`:''}`).join('')||chromeHtml`<p class="empty-note">No body declared.</p>`;}
function contract(e){const authored=interfaceContracts(e);const contracts=(D.operationContracts?.[e.id]||D.contracts).filter(c=>D.subject.startsWith('scenario:')||c.normalized.entrypoint===e.id||c.id===e.id);
 if(!contracts.length)return authored||chromeHtml`<p class="empty-note">Interface contract is not documented for this operation. Neither an unambiguous OpenAPI operation nor a source-derived contract was supplied.</p>${button(e.sourceIds,t('Entrypoint source'))}`;
 return authored+contracts.map(c=>{const v=c.normalized,o=v.operation;return chromeHtml`<div class="contract-intro"><div><h3>${esc(v.method)} ${esc(v.path)}</h3><p>Declared OpenAPI contract from the same revision. ${esc(v.sourceMapping)}. Runtime enforcement is unverified.</p></div>${button(c.sourceIds,t('Original contract'))}</div>${v.boundaries.length?chromeHtml`<p class="finding-strip">${v.boundaries.map(esc).join(' · ')}</p>`:''}<section class="contract-section"><h3>Request parameters</h3>${v.parameters.length?chromeHtml`<div class="table-wrap"><table><thead><tr><th>Parameter</th><th>Location / type</th><th>Required</th><th>Constraints</th></tr></thead><tbody>${v.parameters.map(p=>chromeHtml`<tr><td><code>${esc(p.name)}</code><br>${esc(p.description)}</td><td>${esc(p.in)} / ${type(p.schema)}</td><td>${p.required?t('required'):t('optional')}</td><td>${constraints(p.schema)}</td></tr>`).join('')}</tbody></table></div>`:chromeHtml`<p class="empty-note">No parameters declared.</p>`}</section><section class="contract-section"><h3>Request body</h3><p class="schema-description">${o.requestBody?.required?t('Required body'):t('Optional or absent body')}</p>${contentBodies(o.requestBody?.content)}</section><section class="contract-section"><h3>Responses</h3>${Object.entries(o.responses||{}).map(([status,r])=>chromeHtml`<details class="contract-raw" ${status.startsWith('2')?'open':''}><summary><b class="response-status">${esc(status)}</b> · ${esc(r.description)}</summary>${contentBodies(r.content)}${r.headers?chromeHtml`<h4>Headers</h4><pre>${pretty(r.headers)}</pre>`:''}${button(c.sourceIds,t('Response contract'))}</details>`).join('')}</section><section class="contract-section"><h3>Declared access and servers</h3><p class="schema-description">Static declarations do not prove that access checks or server bindings are active.</p><pre class="example-block">${pretty({security:v.security,securitySchemes:v.securitySchemes,servers:v.servers})}</pre></section><details class="contract-raw"><summary>Complete operation contract</summary><pre>${pretty(o)}</pre></details>`;}).join('');
}
function findings(o){return o?.findings.length?chromeHtml`<div class="findings-list">${o.findings.map(f=>chromeHtml`<article class="finding-card"><div class="eyebrow">SOURCE-BASED OBSERVATION</div><p>${esc(f.text)}</p><div class="finding-actions">${button(f.sourceIds,t('Compare evidence'))}</div></article>`).join('')}</div><p class="finding-note">Static findings are separate from observed runtime failures.</p>`:chromeHtml`<p class="empty-note">No findings recorded within the documented scope.</p>`;}
function freshness(state=D.sectionState){
 const el=$('freshness-status');if(!el)return;
 if(!state){el.innerHTML=chromeHtml`<p class="freshness-banner">Legacy publication: meaning review has not been assessed.</p>`;return;}
 const revisions=values=>Object.entries(values||{}).map(([id,sha])=>chromeHtml`${esc(id)}: ${sha?esc(sha.slice(0,12)):t('unavailable')}`).join(' · ');
 const retainedServices=Object.entries(D.sourceAuthorities||{}).filter(([id,authority])=>authority==='RETAINED_SOURCE_NOT_REVERIFIED'&&Object.hasOwn(D.revisions||{},id)).map(([id])=>id);
 const retainedNotice=retainedServices.length?chromeHtml`<p>Saved evidence reused for ${retainedServices.map(esc).join(', ')}. These sources were not rechecked during the selected-service capture.</p>`:'';
 const message=state.freshness==='CURRENT'?t('Recorded source dependencies are current.'):state.freshness==='STALE'?t('Source changed. This retained explanation needs review.'):t('Source status could not be established. This explanation is retained with a gap.');
 el.innerHTML=chromeHtml`<div class="freshness-banner freshness-${esc(state.freshness.toLowerCase())}"><strong>${esc(statusLabel(state.freshness))}</strong><p>${message}</p>${retainedNotice}<p>Content: ${Object.keys(state.mixedRevisions||{}).length?t('Mixed source versions; inspect each operation.'):revisions(state.contentRevisions)}<br>Target: ${revisions(state.targetRevisions)}</p><p>Meaning review: ${esc(statusLabel(state.verification))}</p>${Object.keys(D.updateFailures||{}).length?chromeHtml`<details><summary>Recent update gaps</summary><pre>${pretty(D.updateFailures)}</pre></details>`:''}${state.reasons?.length?chromeHtml`<details><summary>What needs attention</summary><pre>${pretty(state.reasons)}</pre></details>`:''}</div>`;
}
function showEntry(id,nextTab='sequence'){freshness();current=entries.find(e=>e.id===id)||entries.find(e=>e.kind==='SECTION')||entries.find(e=>operation(e))||entries[0];tab=nextTab==='sequence'&&current?.kind==='DECLARED_OPENAPI'?'contract':nextTab;closeSource(false);$('catalogue-view').hidden=true;$('coverage-view').hidden=true;$('scenario-content').hidden=false;
 if(!current){$('scenario-content').innerHTML=chromeHtml`<h2>No entrypoints available</h2><p class="empty-note">Inspect coverage and the service evidence gaps.</p>`;nav();return;}
 const e=current,o=operation(e);freshness(D.operationStates?.[e.visualOwner||e.id]||D.sectionState);
 if(translationGaps[e.id]&&e.id!=='section-overview'){ $('scenario-content').innerHTML=`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(sectionTitles[e.id]||e.trigger.paths?.join(' · ')||e.symbol||e.id)}</h2>${translationNotice(e.id)}`;nav();history.replaceState(null,'','#'+e.id);return; }
 if(e.kind==='VISUAL'){
  const visual=o.visuals.find(v=>v.id===e.visualId);
  freshness(D.operationStates?.[o.id]||{freshness:'UNVERIFIED',verification:'UNASSESSED'});
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)} · Processes and diagrams</div>${visualCard(o,visual)}`;
  nav();history.replaceState(null,'','#'+e.id);document.title=visual.title+' · '+D.title;return;
 }
 if(e.kind==='NOTE'){
  const n=D.notes.find(n=>n.id===e.id),a=n.assessment?.assessment,state=D.operationStates?.[e.id];
  if(a)$('freshness-status').insertAdjacentHTML('afterbegin',chromeHtml`<p class="small-label">Status of the generated assessment · original note retains human/imported authority</p>`);
  else $('freshness-status').innerHTML=chromeHtml`<p class="freshness-banner">Assessment: UNASSESSED. Original note retains human/imported authority.</p>`;
  const match=a&&a.noteDigest===n.original.digest&&a.associationDigest===n.associationDigest&&n.targetChanged!==true;
  $('scenario-content').innerHTML=chromeHtml`<div class="eyebrow">HUMAN / IMPORTED NOTE</div><h2>${esc(n.association.title)}</h2><p>${esc(n.association.classification)} · Period: ${esc(n.association.period)}</p><p>${esc(n.snapshotLabel)}</p>${n.targetChanged?chromeHtml`<p class="finding-strip">This retained note snapshot or its association has changed since publication. The retained assessment is stale.</p>`:''}<pre class="note-original">${esc(n.original.text||n.original.reason||n.original.status)}</pre><details><summary>Original metadata and associations</summary><pre class="note-original">${pretty(n.association)}</pre></details>${n.missingTargets?.length?chromeHtml`<p class="empty-note">Unresolved targets: ${n.missingTargets.map(esc).join(', ')}</p>`:''}<section class="gap-card"><h3>Separate agent assessment</h3>${a?chromeHtml`<p><b>${esc(a.outcome)}</b> · Period assessed: ${esc(a.period)}</p><p>Input binding: ${match?t('matches displayed capture'):statusLabel('STALE')} · Meaning review: ${esc(statusLabel(state?.verification||'UNASSESSED'))} · Freshness: ${esc(statusLabel(state?.freshness||'UNVERIFIED'))}</p><p>${esc(n.assessment.summary.text)}</p>${button(n.assessment.summary.sourceIds,t('Assessment evidence'))}${a.proposedCorrection?chromeHtml`<h4>Proposed correction · original unchanged</h4><p>${esc(a.proposedCorrection.text)}</p>${button(a.proposedCorrection.sourceIds,t('Correction evidence'))}`:''}`:chromeHtml`<p>No assessment has been accepted for this note on this page.</p>${n.assessmentSubject!==D.subject?chromeHtml`<p><a href="../services/${esc(n.association.service)}.html#${esc(n.id)}">Open the assessment service</a></p>`:''}`}<p>Classification and original text remain human/imported declarations. Evidence tracking covers captured inputs; it cannot establish every implicit claim in arbitrary prose.</p></section>`;
  nav();history.replaceState(null,'','#'+e.id);document.title=n.association.title+' · '+D.title;return;
 }
 if(e.kind==='VIEW'){
  const v=D.view,g=o?.dataflow,d=v.definition.view,h=d.human||{};
  const annotationRows=Object.entries(h.annotations||{});
  $('scenario-content').innerHTML=chromeHtml`<div class="process-view"><div class="eyebrow">SAVED ENTITY VIEW</div><h2>${esc(v.definition.title)}</h2><p>${esc(d.scope)}</p><p>Domain identities: ${d.inputObjects.map(esc).join(', ')}. A DTO, message or table is an implementation representation; its domain mapping remains an interpretation.</p>${v.targetChanged||g&&g.definitionDigest!==v.definitionDigest?chromeHtml`<p class="finding-strip">The definition or a linked component has changed. Retained graph content needs review.</p>`:''}<p class="summary">${esc(o?.summary.text||D.gaps[e.id]||t('No evidence-bound graph has been accepted.'))}</p>${button(o?.summary.sourceIds,t('View evidence'))}<p class="empty-note">Static data flow, not a runtime trace or universal taint analysis. Dashed candidates remain unknown; declared transfers do not establish routing or wire compatibility.</p>${g?chromeHtml`<div class="sequence-scroll dataflow-scroll">${dataflowDiagram(g,h.layout)}</div><h3>Representations and source bindings</h3><div class="table-wrap"><table><thead><tr><th>Node</th><th>Domain identity</th><th>Representation</th><th>Evidence</th></tr></thead><tbody>${g.nodes.map(n=>chromeHtml`<tr><td>${esc(n.meaning.text)}</td><td>${esc(n.entity)}</td><td>${esc(t(n.kind))} · ${esc(n.service)}<br><code>${esc(n.representation)}</code></td><td>${button(n.meaning.sourceIds,t('Node source'))||t('Explicit domain declaration')}</td></tr>`).join('')}</tbody></table></div><h3>Reads, transformations, writes and transfers</h3>${g.edges.map(edge=>chromeHtml`<article class="gap-card"><h4>${esc(edge.from)} → ${esc(edge.to)} · ${esc(t(edge.kind))}</h4><p><b>${esc(edge.authority)}</b> · ${esc(edge.matchBasis)}</p><p>${esc(edge.meaning.text)}</p>${edge.uncertainty?chromeHtml`<p class="empty-note">${esc(edge.uncertainty)}</p>`:''}${button(edge.meaning.sourceIds,t('Edge evidence'))}</article>`).join('')}`:''}<section class="gap-card"><h3>Protected human annotations</h3>${annotationRows.length?annotationRows.map(([id,text])=>chromeHtml`<p><b>${esc(id)}</b></p><pre class="note-original">${esc(text)}</pre>`).join(''):chromeHtml`<p>No human annotations have been added.</p>`}<p>Tags: ${(h.tags||[]).map(esc).join(', ')||t('none')}</p><details><summary>Human metadata and layout</summary><pre class="note-original">${pretty(h)}</pre></details></section>${v.relatedComponents.length?chromeHtml`<section class="gap-card"><h3>Related accepted components</h3>${v.relatedComponents.map(c=>chromeHtml`<p><a href="${esc(c.href)}">${esc(c.child)} ↗</a> · ${esc(c.gap||c.accepted.verification)}</p>`).join('')}</section>`:''}${relatedNotes('view:'+v.definition.id)}${relatedNotes(D.subject)}<section class="gap-card"><h3>Evidence limits</h3><ul>${[...new Set([...d.limitations,...v.module.limitations,...D.boundaries,...(o?.boundaries||[])])].map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul></section></div>`;
  nav();history.replaceState(null,'','#'+e.id);document.title=t('Entity data flow')+' · '+D.title;return;
 }
 if(e.kind==='PROCESS_CATALOG'){
  const catalog=D.processCandidates,summary=catalog?.summary||{},internal=catalog?.internal||[],saved=D.savedProcesses||[];
  const cards=internal.map(candidate=>chromeHtml`<article class="gap-card"><h3>${esc(candidate.owner||t('Callable'))} · ${esc(candidate.name||candidate.symbol)}</h3><p><b>${candidate.status==='NEEDS_EVIDENCE'?t('Needs evidence'):t('Awaiting explanation')}</b> · ${esc(candidate.scope||t('unscoped source'))}</p><p>${candidate.lexicalCallSiteCount?candidate.lexicalCallSiteCount+t(' lexical call sites (targets unresolved)'):candidate.localCallTargetCount+t(' distinct local call targets')} · ${candidate.controlEventCount} control-flow events</p><p>No business interpretation has been accepted for this candidate.</p>${button(candidate.sourceIds,t('Retained method source'),true)}${candidate.gaps.length?chromeHtml`<details><summary>Missing or limited evidence</summary><ul>${candidate.gaps.map(gap=>chromeHtml`<li>${esc(gap)}</li>`).join('')}</ul></details>`:''}<details><summary>Exact root and discovery reasons</summary><pre>${pretty({declaration:candidate.id,scope:candidate.scope,symbol:candidate.symbol,reasons:candidate.reasons})}</pre></details></article>`).join('');
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)}</div><h2>Internal processes</h2>${visualGallery()}<p>Saved process explanations and structural candidates from retained code evidence.</p><h3>Saved processes · ${saved.length}</h3>${saved.length?saved.map(process=>chromeHtml`<article class="gap-card"><h3><a href="${esc(process.href)}">${esc(process.title)} ↗</a></h3><p>${process.status==='AUTHORED'?t('Explanation available'):t('Awaiting explanation')}</p>${process.state?chromeHtml`<p>Source freshness: ${esc(statusLabel(process.state.freshness))} · Meaning review: ${esc(statusLabel(process.state.verification))}</p>`:''}<p>Declared trigger: ${esc(process.trigger)}</p></article>`).join(''):chromeHtml`<p class="empty-note">No internal process has been selected for maintained documentation yet.</p>`}<h3>Internal candidates · ${summary.internalCandidateCount??t('unknown')}</h3><p>These methods contain local calls and control flow. They are candidates for explanation, not an exhaustive list of business processes.</p>${cards||chromeHtml`<p class="empty-note">No internal candidates could be nominated from this evidence. This does not establish that the service has no internal processes. Select an exact method when discovery is incomplete.</p>`}${catalog?.omittedInternal?chromeHtml`<p>${catalog.omittedInternal} additional internal candidates are available in the retained catalogue.</p>`:''}<p>Separate trigger candidates: ${summary.triggerCandidateCount??t('unknown')}. Methods with missing or limited flow: ${summary.flowUnavailableCount??t('unknown')}.</p><details><summary>Browse the full retained catalogue</summary><p>This command reads saved evidence and does not invoke capture or a model.</p><pre>clew docs process candidates --root &lt;documentation-root&gt; --service ${esc(D.subject.slice(8))} --lane internal</pre><p>Use the returned cursor for subsequent pages; use --lane trigger to inspect trigger candidates separately. An explicit --snapshot selects a saved version.</p></details><details><summary>Discovery limits</summary><pre>${pretty(summary)}</pre></details>`;
  nav();history.replaceState(null,'','#'+e.id);document.title=t('Internal processes')+' · '+D.title;return;
 }
 if(e.kind==='PROCESS'){
  const p=D.process,v=p.definition.process;
  const state=D.stateDiagram?chromeHtml`<div class="diagram-card"><div class="section-label"><b>State diagram</b><span>Declarative process states · transitions bound to source evidence</span></div><div class="diagram-img-wrap"><img class="diagram-img" src="../diagrams/${esc(D.stateDiagram)}.svg" alt="State diagram"></div><div class="diagram-footer"><span>States and transitions declared in scenarios/${esc(D.subject.split(':').pop())}-states.yaml</span><a download href="../diagrams/${esc(D.stateDiagram)}.puml">PlantUML ↓</a></div></div>`:'';
  const detailed=(D.lifecycleOperations||[]).map(l=>chromeHtml`<div class="diagram-card"><div class="section-label"><b>${esc(l.name)}</b><span>Подробный порядок действий · pseudocode из retained flow</span></div><pre class="tree">${esc(l.tree)}</pre><div class="diagram-footer"><span>Сгенерировано из самого глубокого retained flow для ${esc(l.name)}</span><a download href="../diagrams/${esc(D.subject.replace(':','-'))}-${esc(l.name)}.puml">PlantUML ↓</a></div></div>`).join('');
  $('scenario-content').innerHTML=chromeHtml`<div class="process-view"><div class="eyebrow">SAVED PROCESS</div><h2>${esc(D.process.definition.title)}</h2><p>${esc(p.authority)}</p>${p.targetChanged?chromeHtml`<p class="finding-strip">This captured definition or a linked child has changed. Retained explanations need review.</p>`:''}${state}<section class="gap-card"><h3>Explicit requested scope</h3><p>${esc(v.scope)}</p><p><b>Trigger:</b> ${esc(v.trigger)}</p><p><b>Participants:</b> ${v.participants.map(esc).join(', ')}</p>${v.objects.length?chromeHtml`<p><b>Domain objects:</b> ${v.objects.map(esc).join(', ')}</p>`:''}<h4>Requested outcomes</h4><ul>${v.outcomes.map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul><p>These are human-requested scope and outcomes; source support is assessed separately.</p></section><h3>Process overview</h3><p class="summary">${esc(o?.summary.text||D.gaps[e.id]||t('No overview has been accepted.'))}</p>${button(o?.summary.sourceIds,t('Overview evidence'))}${detailed}<h3>Linked child views · captured versions</h3>${p.linkedSubviews.length?p.linkedSubviews.map(c=>chromeHtml`<article class="gap-card"><h4>${c.gap==='LINKED_PROCESS_MISSING'?esc(c.child):chromeHtml`<a href="${esc(c.href)}#process-overview">${esc(c.child)} ↗</a>`}</h4><p>${esc(c.gap||c.accepted.verification)}</p>${c.accepted?chromeHtml`<p>${esc(c.accepted.summary.text)}</p><p>${[...c.accepted.limitations,...c.accepted.boundaries].map(esc).join(' · ')}</p><details><summary>Accepted child version and influence</summary><pre class="note-original">${pretty(c.accepted)}</pre></details>`:chromeHtml`<p>No current accepted explanation is available for composition.</p>`}</article>`).join(''):chromeHtml`<p>No linked child views were selected.</p>`}<section class="gap-card"><h3>Evidence limits</h3><ul>${[...new Set([...D.boundaries,...(o?.boundaries||[]),...(p.targetGaps||[])])].map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul></section>${relatedNotes(D.subject)}</div>`;
  nav();history.replaceState(null,'','#'+e.id);document.title=t('Process overview')+' · '+D.title;return;
 }
 if(e.kind==='SECTION'){
  const section=D.sections.find(s=>s.id===e.id),inventory=D.boundaryInventory||{},entities=(D.entities||[]).filter(d=>d.normalized.entity.relations.some(r=>r.service===D.subject.slice(8)));
  let detail='';
  if(e.id==='section-entities')detail=chromeHtml`<h3>Current entity declarations</h3>`+ (entities.length?entities.map(d=>{const v=d.normalized.entity;return chromeHtml`<article class="gap-card"><h3>${esc(v.title)}</h3><p>${esc(v.description)}</p><p>Domain identity: ${esc(v.id)}</p>${v.relations.map(r=>chromeHtml`<p><b>${esc(r.service)} · ${esc(r.kind)}</b> · ${r.origin==='human'?t('Human declaration'):t('Agent proposal')} · ${esc(r.confidence)}<br>${esc(r.rationale)}</p>${r.representations.length?chromeHtml`<details><summary>Implementation representations</summary><ul>${r.representations.map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul></details>`:''}`).join('')}<p>${v.limitations.map(esc).join(' · ')}</p>${button(d.sourceIds,t('Entity evidence'),true)}${relatedNotes('entity:'+v.id)}${relatedViews('entity:'+v.id)}${d.normalized.missingDependencies.length?chromeHtml`<p class="empty-note">Some declared evidence references are unavailable; inspect the entity record.</p>`:''}</article>`;}).join(''):chromeHtml`<p class="empty-note">No domain entity identities have been declared. Classes and DTOs alone do not establish domain ownership.</p>`);
  if(e.id==='section-ingress')detail=chromeHtml`<h3>Discovered public boundaries · ${(inventory.publicBoundaries||[]).length}</h3>${(inventory.publicBoundaries||[]).map(v=>chromeHtml`<p><button data-entry="${esc(v.id)}">${esc(label(v))}</button> ${button(v.sourceIds,t('Source'),true)}</p>`).join('')}<h3>Declared OpenAPI operations · ${D.contracts.length}</h3>${D.contracts.map(c=>chromeHtml`<p><button data-entry="${esc(c.normalized.entrypoint||c.id)}" ${c.normalized.entrypoint?'data-tab="contract"':''}>${esc(c.normalized.method)} ${esc(c.normalized.path)}</button> ${button(c.sourceIds,t('Declaration'),true)}</p>`).join('')}`;
  if(e.id==='section-overview')detail=serviceProfile()+chromeHtml`<section class="profile-processes"><h3>Internal processes and selected diagrams</h3>${visualRows().length?'':chromeHtml`<p class="profile-gap">No typed process diagram has been accepted in this publication.</p>`}<p>Browse maintained processes and structural candidates with their authoring status. A local fragment is not automatically an entry-to-outgoing-call scenario.</p><p><button class="source-link" data-entry="process-catalog">Explore internal processes →</button></p></section>`+visualPreview()+chromeHtml`<details class="profile-discovery"><summary>Source inventory and coverage</summary><div class="coverage-grid"><div class="metric"><b>${(inventory.publicBoundaries||[]).length}</b><p>discovered public boundaries</p></div><div class="metric"><b>${inventory.internalCallableCount??(inventory.internalCallables||[]).length}</b><p>internal callable evidence records</p></div></div><p>Internal callable inventory does not establish public API exposure.</p></details>`;
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(section.title)}</h2><p class="summary">${translationGaps[e.id]?'':esc(o?.summary.text||section.gap)}</p>${translationNotice(e.id)}${button(o?.summary.sourceIds,t('Supporting source'))}${relatedNotes(D.subject+'/'+e.id)}${relatedNotes(D.subject)}${detail}<div class="gap-card"><h3>Evidence limits</h3><ul>${[...(inventory.gaps||[]).map(g=>({'PUBLIC_BOUNDARY_INVENTORY_IS_BOUNDED_BY_SELECTED_MODULES':t('Public boundary discovery covers the selected evidence modules.'),'DYNAMIC_REGISTRATION_AND_RUNTIME_ACTIVATION_UNVERIFIED':t('Dynamic registration and runtime activation have not been verified.'),'SOURCE_EVIDENCE_UNAVAILABLE':t('Source evidence is unavailable.')}[g]||g)),...(o?.boundaries||[])].map(v=>chromeHtml`<li>${esc(v)}</li>`).join('')}</ul></div>`;
  nav();history.replaceState(null,'','#'+e.id);document.title=section.title+' · '+D.title;return;
 }
$('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb"><span>${esc(D.title)}</span><div class="scope-summary"><span><b>${documentedCount()}</b> documented</span><span><b>${inventoryEntries().length}</b> discovered</span></div></div><div class="scenario-heading"><div><div class="operation-label">${chip(method(e))}<code>${esc(e.trigger.paths?.join(' · ')||'')}</code></div><h2>${esc(label(e))}</h2></div>${button(o?.summary.sourceIds||e.sourceIds,t('Source'))}</div><p class="summary">${esc(o?.summary.text||D.gaps[e.id]||(e.kind==='DECLARED_OPENAPI'?t('Declared API operation; source behavior is not established.'):t('This entrypoint has no behavior narrative yet.')))}</p>${o?.findings.length?chromeHtml`<div class="finding-strip"><span class="finding-symbol">!</span><b>${esc(o.findings[0].text)}</b><button data-tab="findings">Inspect →</button></div>`:''}<div class="tabs" role="tablist" aria-label="Operation view">${[['sequence',t('What happens')],['contract',t('Contract')],['findings',t('Findings')]].map(([id,label])=>chromeHtml`<button role="tab" id="tab-${id}" data-tab="${id}" aria-controls="tab-content" aria-selected="${tab===id}">${label}${id==='findings'?chromeHtml`<span class="count">${o?.findings.length||0}</span>`:''}</button>`).join('')}</div><div id="tab-content" role="tabpanel" aria-labelledby="tab-${tab}">${tab==='sequence'?sequence(e,o):tab==='contract'?contract(e):findings(o)}</div>${relatedNotes(D.subject)}`;
 nav();history.replaceState(null,'','#'+e.id);document.title=label(e)+' · '+D.title;
}
function catalogue(){freshness();closeSource(false);$('scenario-content').hidden=true;$('coverage-view').hidden=true;$('catalogue-view').hidden=false;$('catalogue-view').innerHTML=chromeHtml`<div class="catalogue-heading"><div class="eyebrow">SOURCE AND DECLARED SCOPE</div><h2>${inventoryEntries().length} entrypoints</h2><p>${documentedCount()} documented; ${inventoryEntries().length-documentedCount()} explicit gaps.</p></div><div class="table-wrap"><table><thead><tr><th>Trigger</th><th>Operation</th><th>Coverage</th></tr></thead><tbody>${inventoryEntries().map(e=>chromeHtml`<tr class="catalogue-row"><td>${chip(method(e))}</td><td><button data-entry="${esc(e.id)}">${esc(label(e))}</button><br><code>${esc(e.symbol)}</code></td><td>${e.kind==='DECLARED_OPENAPI'?t('Declared contract; source mapping unresolved'):translationGaps[e.id]?t('Translation pending'):operation(e)?t('Documented'):esc(D.gaps[e.id]||t('Awaiting authoring'))}</td></tr>`).join('')}</tbody></table></div>`;}
function coverage(){freshness();closeSource(false);$('scenario-content').hidden=true;$('catalogue-view').hidden=true;$('coverage-view').hidden=false;$('coverage-view').innerHTML=chromeHtml`<div class="coverage-heading"><div class="eyebrow">COVERAGE AND AUTHORITY</div><h2>What the sources support</h2><p>Freshness is scoped to the recorded dependencies. Runtime routing and wire compatibility remain unverified.</p></div><div class="coverage-grid"><div class="metric"><b>${inventoryEntries().length}</b><p>entrypoints in scope</p></div><div class="metric"><b>${documentedCount()}</b><p>documented operations</p></div><div class="metric"><b>${Object.keys(D.sources).length}</b><p>retained source fragments</p></div></div>${D.boundaries.map(b=>chromeHtml`<div class="gap-card"><p>${esc(b)}</p></div>`).join('')}<div class="gap-card"><h3>Declared service interactions</h3>${D.interactions.length?D.interactions.map(i=>chromeHtml`<p>${esc(i.id)} · origin: ${esc(i.origin)} · runtime: ${esc(i.runtime)}</p><details class="technical-evidence"><summary>Checks and boundaries</summary><pre>${pretty(i)}</pre></details>`).join(''):chromeHtml`<p>No service interactions selected for this page.</p>`}</div><details class="technical-evidence"><summary>Revision vector and evidence scope</summary><pre>${pretty({revisions:D.revisions,coverage:D.coverage,extractor:D.extractor,renderer:D.renderer})}</pre></details>`;}
function source(ids,title=t('Source evidence'),currentSnapshot=false,owner=''){sourceScope=currentSnapshot?'current':'';sourceOperation=owner;const valid=ids.filter(id=>sourceRecords()[id]);if(!valid.length)return;lastFocus=document.activeElement;sourceId=valid[0];$('source-select').innerHTML=valid.map(id=>chromeHtml`<option value="${esc(id)}">${esc(sourceRecords()[id].service+' / '+sourceRecords()[id].file+':'+sourceRecords()[id].startLine)}</option>`).join('');$('source-title').textContent=title;sourceContent();$('source-panel').hidden=false;document.body.classList.add('source-open');$('close-source').focus();}
function sourceContent(){const s=sourceRecords()[sourceId];$('source-meta').innerHTML=chromeHtml`${s.url?chromeHtml`<a href="${esc(s.url)}" target="_blank" rel="noopener noreferrer">${esc(s.file)}:${s.startLine}–${s.endLine} ↗</a>`:esc(s.file)}<br>${esc(s.service)} · ${esc(s.revision.slice(0,12))} · ${esc(s.authority)}`;$('source-code').innerHTML=s.text.split('\n').map((line,i)=>chromeHtml`<span class="code-line highlight"><span class="line-number">${s.startLine+i}</span><code>${esc(line)||' '}</code></span>`).join('');$('source-code').scrollTop=0;$('source-foot').innerHTML=chromeHtml`<p>The highlighted fragment is exact retained source. The step narrative is an agent interpretation.</p><details><summary>Source binding</summary>Text: ${esc(s.textDigest)}<br>Evidence: ${esc(s.evidenceDigest)}</details>`;$('copy-status').textContent='';}
function closeSource(focus=true){$('source-panel').hidden=true;document.body.classList.remove('source-open');document.querySelectorAll('.selected[data-event]').forEach(e=>e.classList.remove('selected'));if(focus&&lastFocus?.isConnected)lastFocus.focus();}
document.addEventListener('click',e=>{const el=e.target.closest('[data-visual-target],[data-entry],[data-tab],[data-sources],[data-event],[data-overview-node],[data-overview-edge],[data-view-node],[data-view-edge]');if(!el)return;e.preventDefault();if(el.dataset.visualTarget){showEntry(el.dataset.visualTarget);if(el.dataset.visualNodeTarget){const node=$(el.dataset.visualNodeTarget);node?.scrollIntoView({block:'center'});node?.focus();}else window.scrollTo(0,0);}else if(el.dataset.entry){showEntry(el.dataset.entry,el.dataset.tab||'sequence');window.scrollTo(0,0);}else if(el.dataset.tab)showEntry(current.id,el.dataset.tab);else if(el.dataset.sources)source(el.dataset.sources.split(' '),t('Source evidence'),el.hasAttribute('data-current-sources'),el.dataset.sourceOperation||'');else if(el.dataset.viewNode||el.dataset.viewEdge){const g=operation(current)?.dataflow,item=el.dataset.viewNode?g?.nodes.find(n=>n.id===el.dataset.viewNode):g?.edges.find(e=>e.id===el.dataset.viewEdge);if(item)source(item.meaning.sourceIds,item.meaning.text);}else if(el.dataset.overviewNode||el.dataset.overviewEdge){const o=operation(current),d=o.overviewDiagram,item=el.dataset.overviewNode?d.nodes.find(n=>n.id===el.dataset.overviewNode):d.edges.find(n=>n.id===el.dataset.overviewEdge);if(item){source([...new Set(o.events.filter(e=>item.eventIds.includes(e.id)).flatMap(e=>e.sourceIds))],item.text);}}else if(el.dataset.event){const event=operation(current)?.events.find(e=>e.id===el.dataset.event);if(event){source(event.sourceIds,event.text);el.classList.add('selected');}}});
document.addEventListener('change',e=>{const kind=e.target.dataset.contractSelect;if(!kind)return;if(kind==='boundary')contractId=e.target.value;else payloadId=e.target.value;showEntry(current.id,'contract');if(kind==='payload')document.querySelector('.payload-library').open=true;document.querySelector(chromeHtml`[data-contract-select="${kind}"]`)?.focus();});
document.addEventListener('keydown',e=>{if(e.key==='Escape')closeSource();if(e.target.matches('[role=tab]')&&['ArrowLeft','ArrowRight'].includes(e.key)){e.preventDefault();const tabs=['sequence','contract','findings'];const next=tabs[(tabs.indexOf(tab)+(e.key==='ArrowRight'?1:2))%3];showEntry(current.id,next);$('tab-'+next).focus();}});
$('search').addEventListener('input',nav);$('source-select').onchange=e=>{sourceId=e.target.value;sourceContent();};$('close-source').onclick=()=>closeSource();$('catalogue-button').onclick=catalogue;$('coverage-button').onclick=coverage;
$('copy-source').onclick=async()=>{try{await navigator.clipboard.writeText(sourceRecords()[sourceId].text);$('copy-status').textContent=t('Source copied');}catch{$('copy-status').textContent=t('Select the source text and copy it manually.');}};
$('project-kind').textContent=D.subject.startsWith('service:')?t('MICROSERVICE'):t('INTERACTION SCENARIO');$('project-title').textContent=D.title;$('project-subtitle').textContent=D.subtitle;$('catalogue-count').textContent=inventoryEntries().length;$('revision-links').innerHTML=Object.entries(D.revisions).map(([id,revision])=>chromeHtml`<div class="small-label">${esc(id)} · ${esc(revision.slice(0,12))}</div>`).join('');
window.addEventListener('hashchange',()=>{if(location.hash!=='#content')showEntry(location.hash.slice(1));});showEntry(location.hash.slice(1));
