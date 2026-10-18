//! Labels owned by the declared-profile interpreter, never authored input text.
use std::borrow::Cow;

pub(super) fn owned<'a>(value: &'a str, language: &str) -> Cow<'a, str> {
    if language != "ru" {
        return Cow::Borrowed(value);
    }
    let translated = match value {
        "Initialize context" => "Инициализация контекста",
        "Declared callback group" => "Объявленная группа обработчиков",
        "Declared initial operation queue" => "Объявленная начальная очередь операций",
        "Declared factory task type" => "Тип задачи, объявленный фабрикой",
        "Assign local data" => "Присваивание локальных данных",
        "Local if/else condition" => "Локальное условие if/else",
        "Conditionally select next operations" => "Условный выбор следующих операций",
        "Local helper or call" => "Локальный вспомогательный метод или вызов",
        "Return selected next operations" => "Возврат выбранных следующих операций",
        "Throw from local method" => "Исключение из локального метода",
        "Return from local method" => "Возврат из локального метода",
        "Entry and context initialization" => "Вход и инициализация контекста",
        "Declared callback stages" => "Объявленные этапы обработчиков",
        "Operation lookup bindings" => "Привязки поиска операций",
        "Local decisions and selected operations" => "Локальные решения и выбранные операции",
        "Data mappings and state effects" => "Преобразования данных и изменения состояния",
        "External boundaries and helper calls" => "Внешние границы и вспомогательные вызовы",
        "Exits and unresolved constructs" => "Выходы и неразобранные конструкции",
        "Construction source has Java parse errors; no DSL extraction accepted" => {
            "В исходном коде построения найдены ошибки синтаксиса Java; разбор DSL не принят"
        }
        "Inline builder initialization is missing or ambiguous" => {
            "Инициализация inline-построителя отсутствует или неоднозначна"
        }
        "Inline builder requires a standalone chain ending in build()" => {
            "Для inline-построителя требуется отдельная цепочка, завершающаяся build()"
        }
        "Conditional or nested inline builder expression is unsupported" => {
            "Условное или вложенное выражение inline-построителя не поддерживается"
        }
        "Unsupported inline builder chain call" => {
            "Вызов в цепочке inline-построителя не поддерживается"
        }
        "Unsupported inline builder callback expression" => {
            "Выражение обработчика inline-построителя не поддерживается"
        }
        "Conditional or nested construction is unsupported" => {
            "Условное или вложенное построение не поддерживается"
        }
        "Builder call is not bound to the declared root variable" => {
            "Вызов построителя не привязан к объявленной корневой переменной"
        }
        "Unsupported builder callback expression" => {
            "Выражение обработчика построителя не поддерживается"
        }
        "Root initialization is missing or multiple; chain activation remains unresolved" => {
            "Корневая инициализация отсутствует или неоднозначна; активация цепочки не установлена"
        }
        "Conditional or nested callback chain is unsupported" => {
            "Условная или вложенная цепочка обработчиков не поддерживается"
        }
        "Unsupported chain callback expression" => {
            "Выражение обработчика цепочки не поддерживается"
        }
        "Queue constructor is missing or ambiguous" => {
            "Конструктор очереди отсутствует или неоднозначен"
        }
        "Conditional or nested queue construction is unsupported" => {
            "Условное или вложенное построение очереди не поддерживается"
        }
        "Initial operation arguments are absent" => "Аргументы начальных операций отсутствуют",
        "Unsupported initial operation expression" => {
            "Выражение начальной операции не поддерживается"
        }
        "Registry source has parse errors" => "В исходном коде реестра найдены ошибки синтаксиса",
        "Conditional or nested registry binding is unsupported" => {
            "Условная или вложенная привязка реестра не поддерживается"
        }
        "Unsupported registry binding" => "Привязка реестра не поддерживается",
        "Unsupported registry key or callback" => "Ключ реестра или обработчик не поддерживается",
        "Factory source has parse errors" => "В исходном коде фабрики найдены ошибки синтаксиса",
        "Conditional or nested factory return is unsupported" => {
            "Условный или вложенный возврат из фабрики не поддерживается"
        }
        "Factory task type is not a supported constant return" => {
            "Тип задачи фабрики не задан поддерживаемым константным возвратом"
        }
        "Context source has parse errors" => "В исходном коде контекста найдены ошибки синтаксиса",
        "Unsupported local control construct; enclosed selections/effects are not interpreted" => {
            "Локальная управляющая конструкция не поддерживается; вложенные выборы и эффекты не интерпретируются"
        }
        "Short-circuit expression requires explicit evaluation semantics; enclosed calls are not interpreted" => {
            "Выражение с коротким замыканием требует явной семантики вычисления; вложенные вызовы не интерпретируются"
        }
        "Anonymous class body is outside the selected callback scope" => {
            "Тело анонимного класса находится вне выбранного обработчика"
        }
        "Control-dependent tail after a conditional exit; path is unresolved" => {
            "Продолжение зависит от условного выхода; путь не установлен"
        }
        "Missing decision condition" => "Условие решения отсутствует",
        "Unsupported next-operation expression" => "Выражение следующей операции не поддерживается",
        "Declaration order; callback group does not establish parallel execution" => {
            "Порядок объявлений; группа обработчиков не доказывает параллельное выполнение"
        }
        "Declared chain group; scheduler, exceptions and parallelism remain unresolved" => {
            "Объявленная группа цепочки; планирование, исключения и параллелизм не установлены"
        }
        "Constructor argument order only; dequeue and delivery semantics require engine evidence" => {
            "Установлен только порядок аргументов конструктора; извлечение из очереди и доставка требуют данных о движке"
        }
        "Duplicate registry key: replacement/activation semantics unresolved" => {
            "Повторяющийся ключ реестра: семантика замены и активации не установлена"
        }
        "Unordered operation lookup binding; not an execution sequence" => {
            "Неупорядоченная привязка поиска операций; порядок выполнения не установлен"
        }
        "Factory return declaration does not prove runtime registration or trigger activation" => {
            "Объявленный возврат фабрики не доказывает регистрацию или активацию триггера при выполнении"
        }
        "Assignment is an effect, not a condition" => {
            "Присваивание является эффектом, а не условием"
        }
        "Syntactic selection; activation, queue merge and repeat policy remain unresolved" => {
            "Синтаксический выбор; активация, объединение очередей и правила повторов не установлены"
        }
        "Outbound request is an opaque external decision boundary; local rules, destination configuration, delivery and response semantics are unresolved" => {
            "Исходящий запрос является непрозрачной внешней границей решения; локальные правила, настройка назначения, доставка и обработка ответа не установлены"
        }
        "Profile-declared effect classification; source proves the local call only" => {
            "Классификация эффекта задана профилем; исходный код подтверждает только локальный вызов"
        }
        "Target/effects/engine semantics not interpreted by this profile" => {
            "Назначение, эффекты и семантика движка не интерпретируются этим профилем"
        }
        "Return selection is not runtime execution or successful business completion" => {
            "Выбор при возврате не доказывает выполнение или успешное завершение бизнес-процесса"
        }
        "Local exit marker; overall process completion remains unresolved" => {
            "Локальная отметка выхода; завершение всего процесса не установлено"
        }
        "Overview phases are editorial groups; their order does not assert execution across categories" => {
            "Этапы обзора являются редакционными группами; их порядок не задаёт выполнение между категориями"
        }
        "Engine version is profile-declared or unresolved; framework scheduling, runtime activation, retries, ordering and delivery require separate evidence" => {
            "Версия движка задана профилем или не установлена; планирование, активация, повторы, порядок и доставка требуют отдельных доказательств"
        }
        "This static projection describes possible source structure, not incident causality or successful completion" => {
            "Эта статическая проекция описывает возможную структуру исходного кода, а не причину инцидента или успешное завершение"
        }
        _ => return Cow::Borrowed(value),
    };
    Cow::Borrowed(translated)
}

pub(super) fn choose<'a>(language: &str, en: &'a str, ru: &'a str) -> &'a str {
    if language == "ru" { ru } else { en }
}
