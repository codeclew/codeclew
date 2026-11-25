/* Compile selected retained snippets and project source-defined facts for reader authoring. */
const fs = require("node:fs");
const {createHash} = require("node:crypto");
const compilerPath = require.resolve(process.argv[2] || "typescript");
// Verify the official TypeScript module before loading it from the operator's dependency directory.
const compilerBytes = fs.statSync(compilerPath).size;
const compilerHash = compilerBytes <= 16 * 1024 * 1024
    && createHash("sha256").update(fs.readFileSync(compilerPath)).digest("hex");
if (compilerHash !== "3ae902c92cc44dace175c0e69e13a4b0899f6983c6121d76b9ab8dd5795e7675") {
    throw new Error("The prepared source projection requires the locked TypeScript compiler.");
}
const ts = require(compilerPath);
const sqlRequest = require("./clew_prepared_typescript_sql.cjs");
const directInventory = require("./clew_prepared_typescript_effects.cjs");
const request = JSON.parse(fs.readFileSync(0, "utf8"));
if (request.backgroundFiles) {
    const discover = require("./clew_prepared_typescript_background.cjs");
    process.stdout.write(JSON.stringify(discover(ts, request.backgroundFiles)));
    return;
}
const locations = new Map(), facts = [], seen = new Set();
const limit = 58;
const BODY_READS = new Set(["json", "text", "arrayBuffer", "blob", "formData", "bytes"]);
const priority = [
    "registered_callback", "registration_step", "transaction_step", "sql_request", "sql_columns", "sql_parameters", "sql_result_check", "sql_result_flow",
    "post_commit_call", "post_helper_call", "broker_replay_window", "broker_offset_call", "guarded_state_write",
    "conditional_repeat", "helper_outcome", "finally", "cleanup_call", "finally_resume", "finally_override", "status_body", "http_forward", "response_body_read", "exception_route", "exception_bypass",
    "http_request", "message_request", "request_after_commit_failure",
    "return_rejection", "await_bypass", "result_continuation", "iteration_guard", "early_exit", "direct_effect_inventory", "return", "throw", "argument_binding", "helper_result", "result_use", "call_arguments",
    "awaited_binding", "awaited_call", "retained_effect", "call_order", "callback_binding", "unawaited_call",
];
for (const excerpt of request.excerpts) {
    locations.set(excerpt.id, {excerpt, start: 0, end: excerpt.text.length});
    for (const anchor of excerpt.anchors) locations.set(anchor.id, {excerpt, ...anchor});
}

function parse(location, wrappers, select) {
    const {excerpt, start, end} = location;
    const text = excerpt.text.slice(start, end);
    if (text.length > 48_000) return undefined;
    const kind = ts.getScriptKindFromFileName(excerpt.file);
    for (const [prefix, suffix] of wrappers) {
        const file = ts.createSourceFile(excerpt.file, prefix + text + suffix,
            ts.ScriptTarget.Latest, true, kind);
        if (file.parseDiagnostics.length) continue;
        const node = select(file);
        if (node) return {file, node, excerpt, offset: start - prefix.length};
    }
}
// For a discovery page, enumerate a bounded set of callable source bodies
// inside the verified file. This does not bind a handler or a cross-file call.
const discovered = new Set();
if (request.fileRoot) {
    const location = locations.get(request.fileRoot);
    const bodies = location && parse(location, [["", ""]], file => file);
    function visit(node) {
        if (request.roots.length >= 8) return;
        const declaration = ts.isFunctionDeclaration(node) || ts.isMethodDeclaration(node) ||
            ts.isArrowFunction(node) || ts.isFunctionExpression(node);
        if (declaration && node.body && !node.asteriskToken) {
            const id = "file-body-" + discovered.size;
            discovered.add(id);
            locations.set(id, {excerpt: location.excerpt, start: node.getStart(bodies.file), end: node.getEnd()});
            request.roots.push(id);
            return;
        }
        ts.forEachChild(node, visit);
    }
    if (bodies) visit(bodies.file);
}
const plain = ["", ""], expressionWrapper = ["(", "\n)"], methodWrapper = ["class Retained {\n", "\n}"];
const single = statements => statements.length === 1 ? statements[0] : undefined;
const asyncFunction = node => node.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.AsyncKeyword);
const unwrap = node => {
    while (node && (ts.isParenthesizedExpression(node) || ts.isAsExpression(node)
            || ts.isNonNullExpression(node) || ts.isSatisfiesExpression?.(node))) node = node.expression;
    return node;
};
function bodyAt(id) {
    const location = locations.get(id);
    if (!location) return undefined;
    const selected = parse(location, [plain, expressionWrapper, methodWrapper], file => {
        const statement = single(file.statements);
        if (!statement) return undefined;
        let declaration = statement;
        if (ts.isVariableStatement(statement)) declaration = unwrap(single(statement.declarationList.declarations)?.initializer);
        if (ts.isExpressionStatement(statement)) declaration = unwrap(statement.expression);
        if (ts.isClassDeclaration(statement)) declaration = single(statement.members);
        return declaration && ts.isFunctionLike(declaration) && declaration.body && !declaration.asteriskToken
            ? declaration : undefined;
    });
    if (!selected) return undefined;
    // Build a checker for local lexical identity inside this excerpt. Retained
    // relationships, supplied separately, are the only cross-excerpt bindings.
    const options = {noLib: true, noResolve: true};
    const host = ts.createCompilerHost(options);
    host.getSourceFile = name => name === selected.file.fileName ? selected.file : undefined;
    host.fileExists = name => name === selected.file.fileName;
    host.readFile = name => host.getSourceFile(name)?.text;
    selected.checker = ts.createProgram([selected.file.fileName], options, host).getTypeChecker();
    return {...selected, id};
}

function addresses(scope, node) {
    const start = node.getStart(scope.file) + scope.offset, end = node.getEnd() + scope.offset;
    const candidates = scope.excerpt.anchors.filter(row => row.start <= start && end <= row.end);
    candidates.sort((a, b) => a.end - a.start - (b.end - b.start) || a.id.localeCompare(b.id));
    return candidates.map(row => row.id).concat(scope.excerpt.id);
}
function exact(scope, node) {
    const start = node.getStart(scope.file) + scope.offset, end = node.getEnd() + scope.offset;
    return addresses(scope, node).filter(id => {
        const location = locations.get(id);
        return location.start === start && location.end === end;
    });
}
const source = (scope, node) => addresses(scope, node)[0];
const short = (value, length = 88) => value.length <= length && !/[\r\n]/.test(value) ? value : undefined;
const {caption, callName, literal} = require("./clew_prepared_typescript_expressions.cjs")(ts, unwrap, short);
const predicates = require("./clew_prepared_typescript_predicates.cjs")(ts, unwrap, caption, short);
const queryResults = require("./clew_prepared_typescript_query_result.cjs")(ts, unwrap);
function truth(expression) {
    const node = unwrap(expression);
    if (node.kind === ts.SyntaxKind.TrueKeyword) return true;
    if (node.kind === ts.SyntaxKind.FalseKeyword || node.kind === ts.SyntaxKind.NullKeyword) return false;
    if (ts.isNumericLiteral(node)) return Number(node.text.replaceAll("_", "")) !== 0;
    if (ts.isStringLiteralLike(node)) return node.text.length !== 0;
    if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.ExclamationToken) {
        const known = truth(node.operand);
        if (known !== undefined) return !known;
    }
}
function emit(kind, text, sites, state, uncertainty, details = {}) {
    if (request.focus && !kind.startsWith("write_")) return;
    const guards = state?.guards || [];
    const sources = sites.filter(Boolean);
    if (!sources.length && !guards.some(guard => guard.source)) return;
    const row = {kind, text, sites: sources, guards, ...(uncertainty ? {uncertainty} : {}), ...details};
    const key = JSON.stringify(row);
    if (seen.has(key)) return;
    seen.add(key);
    facts.push(row);
}
const focus = request.focus && require("./clew_prepared_typescript_write.cjs")({
    ts, focus: request.focus, source, unwrap, callName, caption, emit,
});
const compatibility = request.focus?.topic && request.callback.active && request.callback.kind === "EACH_MESSAGE" &&
    require("./clew_prepared_typescript_message.cjs")({ts, unwrap, source, topic: request.focus.topic, resolve: linkedBody});
function evenly(rows, count) {
    if (rows.length <= count) return rows;
    return Array.from({length: count}, (_, index) => rows[Math.round(index * (rows.length - 1) / (count - 1))]);
}
function expressionAt(id) {
    const location = locations.get(id);
    return location && parse(location, [expressionWrapper], file => {
        const statement = single(file.statements);
        return statement && ts.isExpressionStatement(statement) ? unwrap(statement.expression) : undefined;
    })?.node;
}
function actionAt(id) {
    const location = locations.get(id);
    return location && parse(location, [plain, expressionWrapper], file => {
        const statement = single(file.statements);
        const expression = statement && ts.isExpressionStatement(statement) && unwrap(statement.expression);
        return expression && (ts.isCallExpression(expression) || ts.isAwaitExpression(expression)) ? expression : undefined;
    })?.node;
}
function manualConfiguration(options) {
    if (!options || !ts.isObjectLiteralExpression(options)) return false;
    const names = options.properties.map(property => property.name &&
        (ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)) && property.name.text);
    if (!names.every(Boolean) || names.filter(name => name === "autoCommit").length !== 1) return false;
    const mode = options.properties[names.indexOf("autoCommit")];
    return ts.isPropertyAssignment(mode) && unwrap(mode.initializer).kind === ts.SyntaxKind.FalseKeyword;
}
function enclosingManualConfiguration(id) {
    const location = locations.get(id);
    if (!location) return false;
    const {excerpt, start, end} = location;
    for (const [prefix, suffix] of [plain, expressionWrapper]) {
        const file = ts.createSourceFile(excerpt.file, prefix + excerpt.text + suffix, ts.ScriptTarget.Latest, true);
        if (file.parseDiagnostics.length) continue;
        function visit(node) {
            if (node.getStart(file) > start + prefix.length || node.end < end + prefix.length) return false;
            if (ts.isObjectLiteralExpression(node) && manualConfiguration(node) &&
                    node.properties.some(property => property.getStart(file) === start + prefix.length &&
                       property.end === end + prefix.length)) return true;
            return Boolean(ts.forEachChild(node, child => visit(child) ? true : undefined));
        }
        if (visit(file)) return true;
    }
    return false;
}
function setupGuards(guards) {
    return guards.map(({source, outcome}) => {
        const test = expressionAt(source);
        const structured = test && predicates.tree(test, () => source);
        return {text: (test && caption(test) || structured && predicates.summarize(structured) || "the cited condition") +
            " is " + outcome, source};
    });
}
const registration = request.setup.find(entry => entry.role === "Registration");
const registrationGuards = registration ? setupGuards(registration.guards) : [];
const manual = new Map();
if (registration && request.callback.active && request.callback.kind === "EACH_MESSAGE") {
    const action = actionAt(registration.action);
    const call = action && (ts.isAwaitExpression(action) ? unwrap(action.expression) : action);
    if (call && ts.isCallExpression(call) && ts.isPropertyAccessExpression(call.expression) &&
            call.expression.name.text === "run" && call.arguments.length === 1) {
        const config = manualConfiguration(unwrap(call.arguments[0])) ? registration.action :
            request.configurations.find(enclosingManualConfiguration);
        if (config) {
            for (const binding of request.offsetReceivers) {
                manual.set(binding.use, [...request.callback.sources, registration.action, config, binding.declaration, binding.consumer]);
            }
        }
    }
}
function replayWindow(effect, commit, sites, guards) {
    if (!effect.offset || !commit) return;
    const rejected = {text: "the awaited " + effect.place + " throws or rejects", source: effect.site};
    emit("broker_replay_window", "The registered KafkaJS consumer uses manual offsets. If the earlier database request committed " +
        "and the broker later delivers the same record again after this offset failure, the callback can run again.",
        [...effect.offset, ...sites, commit.site, effect.site], {guards: [...guards, ...registrationGuards, rejected]},
        "This requires activation and redelivery to the registered consumer. The effective broker offset, delivery and the callback's next path remain open.");
}
for (const entry of request.setup) {
    const action = actionAt(entry.action);
    const call = action && (ts.isAwaitExpression(action) ? unwrap(action.expression) : action);
    if (!call || !ts.isCallExpression(call)) continue;
    const guards = setupGuards(entry.guards);
    const wait = ts.isAwaitExpression(action) ? "awaits " : "calls ";
    const argumentsText = call.arguments.map(argument => caption(argument)).join(", ");
    const callee = callName(call);
    const label = short(callee + " with " + argumentsText, 218);
    emit("registration_step", "The " + entry.role.toLowerCase() + " " + wait +
        (argumentsText && label || callee) + ".", [entry.action], {guards},
        "Registration, scheduling and runtime activation remain open.");
}

const bodies = new Map(), bindings = new Map();
function load(id) {
    if (!bodies.has(id)) bodies.set(id, bodyAt(id));
    return bodies.get(id);
}
for (const link of request.calls) {
    if (!bindings.has(link.call)) bindings.set(link.call, new Set());
    bindings.get(link.call).add(link.target);
}
function linkedBody(scope, call) {
    const ids = exact(scope, call).filter(id => bindings.has(id));
    const alternatives = new Set(ids.flatMap(id => [...bindings.get(id)]));
    if (alternatives.size !== 1 || call.questionDotToken || call.arguments.some(ts.isSpreadElement)) return undefined;
    return load([...alternatives][0]);
}
const active = request.roots.map(id => ({id, guards: [], chain: [id]}));
const queued = new Set(request.roots);
const helperCalls = [], downstreamCalls = [], bodyOutcomes = new Map(), queryOutputs = [];
function schedule(id, guards, chain, writePath, message) {
    if (focus) {
        if (!chain.includes(id) && active.length < 20) active.push({id, guards, chain: [...chain, id], writePath, message});
        else if (compatibility) focus.incomplete();
        return;
    }
    if (!queued.has(id) && active.length < 20) {
        queued.add(id);
        active.push({id, guards});
    }
}
const callbacks = new Map();
for (const callback of request.callbacks) {
    if (callback.control) {
        if (!callbacks.has(callback.control)) callbacks.set(callback.control, []);
        callbacks.get(callback.control).push(callback);
        continue;
    }
    const [registrationSource, target] = callback.sources;
    const name = target && load(target)?.node.name?.getText();
    if (registrationSource && target && request.callback.active) {
        emit("registered_callback", "The retained configuration can use " +
            (name ? name + " as the message callback." : "this message callback."), callback.sources,
            {guards: [...registrationGuards, ...setupGuards(callback.guards)]},
            "This applies if the configuration reaches the registration and the runtime invokes the handler.");
    }
}
if (request.callback.kind === "INTERVAL" && request.callback.sources.length === 2) {
    emit("registered_callback", "The retained registration supplies this interval callback.", request.callback.sources,
        {guards: registrationGuards}, "Callback scheduling and invocation remain open.");
}
if (request.callback.active && request.callback.sources.length === 2) {
    for (const entry of active) {
        entry.guards = [{text: "the runtime invokes this registered callback", source: request.callback.sources[0]}];
    }
}
const visited = new Set();
function scan(scope, inherited) {
    let fuel = 320;
    let expressionFuel = 2400;
    const inventory = !focus && request.roots.includes(scope.id) && directInventory({ts, scope, source, callName, unwrap, emit});
    const initial = {guards: inherited.guards, last: undefined, values: new Map(), exits: [],
        ...(!focus ? {queryValues: new Map()} : {}),
        ...(focus ? {writePath: inherited.writePath || {branches: [], exits: []}, writeTransactions: new Map()} : {})};
    if (compatibility) {
        const previous = inherited.message;
        Object.assign(initial, {
            messageValues: compatibility.seeds(scope, previous?.args || (inherited.chain.length === 1 ? compatibility.root : [])),
            messageEvidence: previous?.evidence || [], messageCompared: previous?.compared,
            messageContradiction: previous?.contradiction, messageOpen: previous?.open, messageValidation: previous?.validation,
        });
    }
    const bypasses = new Map(), observedCalls = new Map();
    function retainBypass(exit, call, current) {
        if (!bypasses.has(exit)) bypasses.set(exit, {calls: new Map(), limited: false});
        const skipped = bypasses.get(exit);
        if (skipped.calls.size < 6 || skipped.calls.has(call)) skipped.calls.set(call, current);
        else skipped.limited = true;
    }
    const symbol = identifier => ts.isIdentifier(identifier) ? scope.checker.getSymbolAtLocation(identifier) : undefined;
    const query = !focus && queryResults.tracker(scope, symbol, source);
    const captured = new Set();
    function findCaptured(node, nested = false) {
        const inside = nested || ts.isFunctionLike(node) || ts.isClassLike(node);
        if (inside && ts.isIdentifier(node)) captured.add(symbol(node));
        ts.forEachChild(node, child => findCaptured(child, inside));
    }
    findCaptured(scope.node.body);
    if (compatibility) {
        for (const binding of initial.messageValues.keys()) if (captured.has(binding)) initial.messageValues.delete(binding);
    }
    const immutable = binding => binding.declarations?.[0]?.parent.flags & ts.NodeFlags.Const;
    function localBinding(node) {
        const value = symbol(node);
        const declaration = value?.declarations?.length === 1 && value.declarations[0];
        if (!declaration || !ts.isVariableDeclaration(declaration) || captured.has(value) && !immutable(value)) {
            return undefined;
        }
        let owner = declaration.parent;
        while (owner && !ts.isFunctionLike(owner)) owner = owner.parent;
        return owner === scope.node ? value : undefined;
    }
    function removeMutable(state) {
        return {...state, values: new Map([...state.values].filter(([binding]) => immutable(binding)))};
    }
    function bindMessage(state, binding, value) {
        if (!binding || captured.has(binding)) return state;
        const values = new Map(state.messageValues);
        if (value) values.set(binding, value);
        else values.delete(binding);
        return {...state, messageValues: values};
    }
    function changesLocal(node) {
        if (!node || ts.isFunctionLike(node) || ts.isClassLike(node)) return false;
        if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "eval") return true;
        if (ts.isBinaryExpression(node) && node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
                node.operatorToken.kind <= ts.SyntaxKind.LastAssignment) return true;
        if ((ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) &&
                [ts.SyntaxKind.PlusPlusToken, ts.SyntaxKind.MinusMinusToken].includes(node.operator)) return true;
        return Boolean(ts.forEachChild(node, child => changesLocal(child) ? true : undefined));
    }
    const empty = () => ({normal: [], exits: []});
    const useSite = (node, state) => {
        if (!node) return [];
        const values = new Set();
        function visit(item) {
            if (ts.isFunctionLike(item) || ts.isClassLike(item) || ts.isTypeNode(item)) return;
            if (ts.isConditionalExpression(item)) {
                visit(item.condition);
                return;
            }
            if (ts.isBinaryExpression(item) && [
                ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken, ts.SyntaxKind.QuestionQuestionToken,
                ts.SyntaxKind.AmpersandAmpersandEqualsToken, ts.SyntaxKind.BarBarEqualsToken, ts.SyntaxKind.QuestionQuestionEqualsToken,
            ].includes(item.operatorToken.kind)) {
                visit(item.left);
                return;
            }
            if (ts.isOptionalChain(item)) {
                visit(item.expression);
                return;
            }
            if (ts.isIdentifier(item) && !(ts.isPropertyAccessExpression(item.parent) && item.parent.name === item)
                    && !(ts.isPropertyAssignment(item.parent) && item.parent.name === item)) {
                const use = ts.isShorthandPropertyAssignment(item.parent)
                    ? scope.checker.getShorthandAssignmentValueSymbol(item.parent) : symbol(item);
                if (state.values.has(use)) values.add(state.values.get(use));
            }
            ts.forEachChild(item, visit);
        }
        visit(node);
        return [...values];
    };
    const describeFlow = (expression, label, site, state) => {
        for (const origin of useSite(expression, state)) {
            emit("result_use", label + " reads " + origin.name + ", which was bound to the " +
                (origin.awaited ? "awaited" : "returned") + " value of " + origin.call + ".",
                [...origin.sites, site], state);
        }
    };
    function resultFor(call, target, site, state, waited, binding, messageArgs) {
        if (!target) return;
        const name = callName(call);
        const guard = {text: name + " reaches the retained implementation", source: site};
        const guards = [...state.guards, guard];
        schedule(target.id, guards, inherited.chain, focus?.callPath(scope, call, state), compatibility ? {
            args: messageArgs, evidence: [...state.messageEvidence, site], compared: state.messageCompared,
            contradiction: state.messageContradiction, open: state.messageOpen, validation: state.messageValidation,
        } : undefined);
        const observed = {target: target.id, site, name, guards, awaited: waited, async: asyncFunction(target.node),
            destination: binding?.name};
        if (waited || !observed.async) helperCalls.push(observed);

        const args = [];
        for (let index = 0; index < call.arguments.length; index++) {
            const formal = target.node.parameters[index];
            if (!formal || formal.dotDotDotToken || !ts.isIdentifier(formal.name)) break;
            const actual = caption(call.arguments[index]);
            if (actual !== undefined) args.push(actual + " to " + formal.name.text);
        }
        if (args.length) {
            emit("argument_binding", name + " supplies " + args.join(", ") + ".", [site, target.id],
                {guards}, "The receiving implementation is source-selected.");
        }
        return observed;
    }
    const relevant = (call, statement) => {
        const requestSource = exact(scope, call).find(id => request.forwards[id]);
        if (requestSource) {
            const forward = request.forwards[requestSource];
            return {name: forward.method + " " + forward.target, kind: "http_forward", httpTarget: forward.target,
                sites: [requestSource, ...forward.proof], requestSource};
        }
        const ids = [...exact(scope, call), ...exact(scope, statement)];
        const operations = request.stages.filter(stage => stage.sources.some(id => ids.includes(id)));
        const operation = operations.length === 1 ? operations[0] : undefined;
        const name = callName(call);
        const first = call.arguments[0];
        const argument = first && literal(unwrap(first));
        return {name: argument ? name + " with " + argument : name, kind: operation ? "retained_effect" : "call",
            description: operation?.role === "INSERT" && operation.target ? "the insert into " + operation.target : undefined,
            stage: operation?.role, sites: operation?.sources || []};
    };
    function resolveConstant(expression, depth = 0) {
        const node = unwrap(expression);
        if (!node || !ts.isIdentifier(node) || depth >= 4) return node;
        const binding = localBinding(node);
        const initializer = binding && immutable(binding) && binding.declarations[0].initializer;
        return initializer ? resolveConstant(initializer, depth + 1) : node;
    }
    function resolveSelectConstant(expression, depth = 0) {
        const node = unwrap(expression);
        if (!node || !ts.isIdentifier(node) || depth >= 4) return node;
        const binding = localBinding(node);
        const declaration = binding && immutable(binding) && binding.declarations[0];
        if (declaration?.initializer && declaration.end <= node.getStart(scope.file)) {
            const value = resolveSelectConstant(declaration.initializer, depth + 1);
            if (value && ts.isStringLiteralLike(value)) return value;
        }
        return node;
    }
    function transport(call, state, awaited) {
        if (ts.isOptionalChain(call)) return;
        const site = source(scope, call);
        const occurrence = scope.id + ":" + call.getStart();
        const target = unwrap(call.expression);
        const args = call.arguments.map(unwrap);
        const members = object => {
            if (!object || !ts.isObjectLiteralExpression(object) ||
                object.properties.some(item => !ts.isPropertyAssignment(item) || ts.isComputedPropertyName(item.name))) return;
            const pairs = object.properties.map(item => [item.name.text, unwrap(item.initializer)]);
            if (new Set(pairs.map(pair => pair[0])).size === pairs.length) return new Map(pairs);
        };
        if (ts.isIdentifier(target) && target.text === "fetch" && !symbol(target) &&
            args.length <= 2 && args[0] && ts.isStringLiteralLike(args[0])) {
            const options = members(args[1]);
            if (args.length === 2 && !options) return;
            const method = options?.get("method");
            if (method && !ts.isStringLiteralLike(method)) return;
            const uri = args[0].text;
            emit("http_request", "The source " + (awaited ? "awaits" : "calls") + " the retained fetch expression.",
                [site], state, "The fetch implementation and runtime destination remain open.",
                {_edge: {kind: "HTTP", site, uri, occurrence, method: method?.text || "GET"}});
            return occurrence;
        } else if (ts.isPropertyAccessExpression(target) && target.name.text === "send" && args.length === 1) {
            const options = members(args[0]);
            const topic = options?.get("topic");
            if (!topic || !ts.isStringLiteralLike(topic) || !options.has("messages")) return;
            emit("message_request", "The source " + (awaited ? "awaits" : "calls") + " the retained message send expression.",
                [site], state, "The producer implementation and runtime broker binding remain open.",
                {_edge: {kind: "PUBLISH", site, occurrence, topic: topic.text}});
            return occurrence;
        }
    }
    function sqlFact(sql, call, site, state, awaited) {
        const name = callName(call);
        const sites = [site, source(scope, sql.literal)];
        const execution = "Execution, database identity and outcome depend on the runtime query binding.";
        if (sql.select) {
            const {table, tableAlias, selected, distinct, where, orderBy, limit, offset} = sql.select;
            const columns = selected.map(({column, alias}) => column + (alias ? " AS " + alias : "")).join(", ");
            const ordered = orderBy?.map(({expression, direction, nulls}) =>
                [expression, direction, nulls && "NULLS " + nulls].filter(Boolean).join(" ")).join(", ");
            const clauses = [where && "WHERE " + where, ordered && "ORDER BY " + ordered,
                limit && "LIMIT " + limit, offset && "OFFSET " + offset].filter(Boolean);
            const projection = (distinct ? "DISTINCT " : "") + columns + " FROM " + table + (tableAlias ? " AS " + tableAlias : "");
            emit("sql_request", (awaited ? "Awaits " : "Calls ") + name + " with a static SELECT of " + projection +
                (clauses.length ? " with " + clauses.join(", ") : "") + ".", sites, state, execution, {select: sql.select, _edge: {kind: "SELECT", site, table}});
        } else if (sql.kind !== "INSERT") {
            emit("transaction_step", (awaited ? "Awaits " : "Calls ") + name + " with the SQL command " + sql.label + ".",
                sites, state, execution, sql.table ? {_edge: {kind: sql.kind, site, table: sql.table}} : {});
            return;
        } else {
            const suffix = [sql.conflict, sql.predicate && "(" + sql.predicate + ")", sql.returning && "RETURNING " + sql.returning]
                .filter(Boolean).join(" ");
            emit("sql_request", (awaited ? "Awaits " : "Calls ") + name + " with " + sql.label + (suffix ? " and " + suffix : "") + ".",
                sites, state, execution, {_edge: {kind: "INSERT", site, table: sql.table}});
            if (sql.columns?.length || sql.conflictColumns?.length || sql.assignments?.length) {
                const insert = sql.columns?.length ? "The insert names columns " + sql.columns.join(", ") + "." : undefined;
                const target = sql.conflictColumns?.length ? " The conflict target names " + sql.conflictColumns.join(", ") + "." : "";
                const updates = sql.assignments?.length ? " The conflict update assigns " +
                    sql.assignments.map(({column, value}) => column + " = " + value).join(", ") + "." :
                    sql.conflictAction === "NOTHING" ? " The conflict alternative requests no update." : "";
                emit("sql_columns", (insert || "The SQL statement names a conflict alternative.") + target + updates, sites, state,
                    execution, {columns: {insert: sql.columns, values: sql.valuesByColumn, conflictTarget: sql.conflictColumns,
                        conflictAction: sql.conflictAction, assignments: sql.assignments, where: sql.predicate}});
            }
        }
        const actuals = [];
        for (const index of sql.parameters.slice(0, 10)) {
            const argument = sql.values[index - 1];
            const value = argument && caption(argument);
            if (value) actuals.push({text: "$" + index + " receives " + value, site: source(scope, argument)});
        }
        if (actuals.length) {
            emit("sql_parameters", "The query values array supplies these positions in the SQL request. " +
                actuals.map(argument => argument.text).join(", ") + ".", [...sites, ...actuals.map(argument => argument.site)],
                state, "The mapping applies under PostgreSQL-style positional binding.");
        }
    }
    function statusBody(call, site, state) {
        const callee = unwrap(call.expression);
        if (!ts.isPropertyAccessExpression(callee) || !["json", "send"].includes(callee.name.text)) return;
        const receiver = unwrap(callee.expression);
        if (!receiver || !ts.isCallExpression(receiver) || ts.isOptionalChain(receiver) || receiver.arguments.length !== 1) return;
        const method = unwrap(receiver.expression);
        if (!ts.isPropertyAccessExpression(method) || !["status", "code"].includes(method.name.text)) return;
        const code = unwrap(receiver.arguments[0]);
        if (!ts.isNumericLiteral(code) || !/^[1-5][0-9]{2}$/.test(code.getText()) || call.arguments.length > 1 ||
                call.arguments.some(ts.isSpreadElement)) return;
        const body = call.arguments.length && caption(call.arguments[0]);
        emit("status_body", callName(receiver) + " receives " + code.getText() + ". The returned receiver's " +
            callee.name.text + (body ? " receives " + body : call.arguments.length ? " receives the evaluated argument" :
                " is called without an argument") + ".", [source(scope, receiver), site], state,
            "The framework binding determines the actual HTTP status and response.");
        return Number(code.getText());
    }
    function orderedChildren(node) {
        if (ts.isFunctionLike(node) || ts.isClassLike(node) || ts.isTypeNode(node)) return [];
        if (ts.isOptionalChain(node) || ts.isConditionalExpression(node)) return [node.expression || node.condition];
        if (ts.isBinaryExpression(node)) {
            const op = node.operatorToken.kind;
            const left = truth(node.left);
            if (op === ts.SyntaxKind.QuestionQuestionToken || op === ts.SyntaxKind.QuestionQuestionEqualsToken ||
                    (op === ts.SyntaxKind.AmpersandAmpersandToken || op === ts.SyntaxKind.AmpersandAmpersandEqualsToken) && left !== true ||
                    (op === ts.SyntaxKind.BarBarToken || op === ts.SyntaxKind.BarBarEqualsToken) && left !== false) return [node.left];
            return [node.left, node.right];
        }
        if (ts.isPropertyAccessExpression(node)) return [node.expression];
        if (ts.isElementAccessExpression(node)) return [node.expression, node.argumentExpression];
        if (ts.isCallExpression(node) || ts.isNewExpression(node)) return [node.expression, ...(node.arguments || [])];
        const children = [];
        ts.forEachChild(node, child => { children.push(child); });
        return children;
    }
    function destination(catcher) {
        return catcher.kind === "finally" ? "the finally block" : "the local catch";
    }
    const bindingIds = new Map();
    function repeatSignature(call) {
        function stableTarget(expression) {
            const leaf = unwrap(expression);
            return ts.isIdentifier(leaf) || leaf.kind === ts.SyntaxKind.ThisKeyword ||
                ts.isPropertyAccessExpression(leaf) && !ts.isOptionalChain(leaf) && stableTarget(leaf.expression);
        }
        if (!stableTarget(call.expression) || call.arguments.some(ts.isSpreadElement)) return undefined;
        const text = short(call.getText(), 260);
        if (!text) return undefined;
        const bindings = [];
        function lexical(node) {
            if (ts.isIdentifier(node) && !(ts.isPropertyAccessExpression(node.parent) && node.parent.name === node)) {
                const binding = symbol(node);
                if (binding && !bindingIds.has(binding)) bindingIds.set(binding, bindingIds.size + 1);
                bindings.push(bindingIds.get(binding) || 0);
            }
            ts.forEachChild(node, lexical);
        }
        lexical(call);
        return JSON.stringify([text, bindings]);
    }
    function failureAt(catcher, site, name, waited, state, signature, occurrence) {
        if (!catcher) return undefined;
        const failure = name + (waited ? " throws during evaluation or the await rejects" : " throws before returning");
        const route = catcher.kind === "escape" ? "evaluation leaves this finally abruptly" :
            "control enters " + destination(catcher);
        emit("exception_route", "If " + failure + ", " + route + ".", [site, catcher.site], state, undefined,
            occurrence ? {_edgeCause: occurrence} : {});
        if (occurrence && state.commit) {
            emit("request_after_commit_failure", "The source reaches this failure after the awaited " +
                state.commit.name + " fulfills. The prior commit and the failing call take separate outcomes.",
                [state.commit.site, site, catcher.site], state,
                "Durability and server rollback depend on the earlier provider and transaction.",
                {_edgeCause: occurrence});
        }
        return {...state, last: undefined, exits: [], failed: signature && waited ? {signature, site} : undefined,
            ...(focus ? {writeTransactions: new Map()} : {}),
            ...(compatibility ? {messageValues: compatibility.dropRecords(state.messageValues)} : {}),
            errorArrival: waited ? {name, site, occurrence} : state.errorArrival,
            guards: [...state.guards, {text: failure, source: site}]};
    }
    function evaluate(expression, statement, state, catcher, ending, binding, depth = 0, evaluated = new Map()) {
        const node = unwrap(expression);
        if (!node) return {state, errors: [], evaluated};
        if (depth > 18 || --expressionFuel <= 0) {
            if (compatibility) focus.incomplete();
            inventory && inventory.gap(node);
            return {state, errors: [], evaluated};
        }
        if (depth === 0 && changesLocal(node)) {
            state = removeMutable(state);
            if (query) state = {...state, queryValues: new Map()};
            if (compatibility) state = {...state, messageValues: new Map()};
        }
        if (binding && state.values.has(binding)) {
            const values = new Map(state.values);
            values.delete(binding);
            state = {...state, values};
        }
        const awaited = ts.isAwaitExpression(node);
        const call = awaited ? unwrap(node.expression) : node;
        const errors = [];
        const evaluatedChildren = orderedChildren(call);
        if (inventory) {
            if (ts.isConditionalExpression(call) && [call.whenTrue, call.whenFalse].some(item => inventory.containsCall(item))) {
                inventory.gap(call);
            } else if (ts.isBinaryExpression(call) && evaluatedChildren.length === 1 && inventory.containsCall(call.right)) {
                const op = call.operatorToken.kind;
                const left = truth(call.left);
                const certainlySkipped = (op === ts.SyntaxKind.AmpersandAmpersandToken && left === false) ||
                    (op === ts.SyntaxKind.BarBarToken && left === true);
                if (!certainlySkipped) inventory.gap(call.right);
            } else if (ts.isCallExpression(call) && ts.isOptionalChain(call) && call.arguments.some(item => inventory.containsCall(item))) {
                inventory.gap(call);
            }
        }
        for (const child of evaluatedChildren) {
            const nested = evaluate(child, statement, state, catcher, undefined, undefined, depth + 1, evaluated);
            state = nested.state;
            errors.push(...nested.errors);
        }
        if (ts.isNewExpression(call)) {
            inventory && inventory.record(call, {label: caption(call) || "construction with " + callName(call)});
            const failure = failureAt(catcher, source(scope, call), "construction with " + callName(call), awaited, state);
            if (failure) errors.push(failure);
            if (query) state = query.escape(state, call.arguments || [], evaluated);
        }
        if (!ts.isCallExpression(call) || ts.isOptionalChain(call) ||
                ts.isPropertyAccessChain(call.expression) || ts.isElementAccessChain(call.expression)) {
            if (inventory && ts.isCallExpression(call)) inventory.record(call, {label: caption(call) || callName(call)});
            if (inventory && ts.isTaggedTemplateExpression(call)) inventory.record(call, {label: "template tagged by " +
                (caption(call.tag) || "the cited expression")});
            if (query) {
                // A skipped operand can still pass a tracked object to opaque code at runtime.
                if (ts.isConditionalExpression(call)) state = query.escape(state, [call.whenTrue, call.whenFalse], evaluated);
                if (ts.isBinaryExpression(call) && evaluatedChildren.length === 1) state = query.escape(state, [call.right], evaluated);
                if (ts.isCallExpression(call)) state = query.escape(state, [call.expression, ...call.arguments], evaluated);
                if (ts.isDeleteExpression(call)) state = query.escape(state, [call.expression], evaluated);
            }
            if (depth === 0) describeFlow(node, ending || "This expression", source(scope, node), state);
            if (binding) {
                const values = useSite(node, state);
                if ((ts.isIdentifier(node) || ts.isPropertyAccessExpression(node)) && values.length === 1) {
                    state = {...state, values: new Map([...state.values, [binding, values[0]]])};
                }
            }
            if (query && binding) {
                const value = !captured.has(binding) && query.read(node, state, evaluated);
                if (!value) state = query.escape(state, [node], evaluated);
                state = query.bind(state, binding, value, node);
            }
            if (compatibility && binding) {
                const value = compatibility.abstract(scope, node, state.messageValues);
                if (!value && (ts.isObjectLiteralExpression(node) || ts.isArrayLiteralExpression(node))) {
                    state = {...state, messageValues: compatibility.escapedValues(scope, [node], state.messageValues)};
                }
                state = bindMessage(state, binding, value);
            }
            return {state, errors, evaluated};
        }
        const site = source(scope, call);
        const target = linkedBody(scope, call);
        const callIds = exact(scope, call);
        for (const callback of callIds.flatMap(id => callbacks.get(id) || [])) {
            const name = callName(call);
            schedule(callback.body, [...state.guards,
                {text: name + " invokes this callback", source: site}], inherited.chain, focus?.callPath(scope, call, state));
            emit("callback_binding", name + " receives this callback.", [site, callback.body, ...callback.proof], state,
                "Callback invocation, completion and timing depend on the controlling code.");
        }
        const args = compatibility ? call.arguments.map(argument => compatibility.abstract(scope, argument, state.messageValues)) : undefined;
        const helper = resultFor(call, target, site, state, awaited, binding, args);
        const validated = compatibility && !awaited && target && compatibility.validate(target, args);
        const effect = relevant(call, depth ? call : statement);
        const sites = [site, ...effect.sites];
        const sql = sqlRequest(ts, call, resolveConstant, Boolean(focus)) ||
            query && effect.kind !== "http_forward" && sqlRequest.select(ts, call, resolveSelectConstant);
        if (focus) state = focus.call(scope, call, depth ? undefined : statement, sql, state, awaited, localBinding);
        const place = sql ? callName(call) + " with " + sql.label : effect.description || effect.name;
        inventory && inventory.record(call, {label: caption(call) || place, effect, target, sql});
        const argumentText = short(call.arguments.map(argument => caption(argument) || "an evaluated expression").join(", "), 208);
        const occurrence = !focus && transport(call, state, awaited);
        if (sql) sqlFact(sql, call, site, state, awaited);
        else if (awaited || effect.kind === "http_forward" || effect.kind === "retained_effect") {
            emit(awaited && effect.kind === "call" ? "awaited_call" : effect.kind,
                (awaited ? "The operation awaits " : "The operation calls ") + place + ".",
                sites, state, effect.kind === "http_forward" ? "The destination and response depend on runtime bindings." : undefined);
        }
        if (!sql && (awaited || depth || effect.kind === "http_forward") && argumentText) {
            emit("call_arguments", (awaited ? "The awaited " : "The ") + callName(call) + " call receives " + argumentText +
                " in source argument order.", [site, ...call.arguments.map(argument => source(scope, argument))], state);
        }
        for (const [index, argument] of call.arguments.entries()) {
            describeFlow(argument, "Argument " + (index + 1) + " to " + place, site, state);
        }
        const method = unwrap(call.expression);
        if (ts.isPropertyAccessExpression(method) && BODY_READS.has(method.name.text)) {
            const reply = state.values.get(symbol(unwrap(method.expression)));
            if (reply?.requestSource) {
                emit("response_body_read", "The operation " + (awaited ? "awaits " : "calls ") + method.name.text +
                    " on the value returned by " + reply.call + ".", [...reply.sites, site], state,
                    "Body decoding depends on the bound response API.", {requestSource: reply.requestSource, awaited: !!awaited});
            }
        }
        if (ts.isPropertyAccessExpression(method) || ts.isElementAccessExpression(method)) {
            describeFlow(method.expression, "The receiver of " + place, site, state);
        }
        const status = statusBody(call, site, state);
        if (query && ts.isPropertyAccessExpression(method) && ["json", "send"].includes(method.name.text) &&
                call.arguments.length && !call.arguments.some(ts.isSpreadElement)) {
            const reads = query.outputs(call.arguments[0], state, evaluated);
            if (reads.length) queryOutputs.push({reads, site, guards: state.guards,
                sink: {kind: "argument", call: callName(call), status}});
        }
        if (state.inCleanup && !depth) {
            emit("cleanup_call", "The finally block " + (awaited ? "awaits " : "calls ") + place + ".",
                [state.inCleanup, site], state);
        }
        const previous = state.last;
        if (!depth && !state.inCleanup) {
            for (const path of state.awaitExits || []) {
                const mode = path.mode === "return" ? "the cited catch returns before " :
                    path.mode === "throw" ? "the cited catch throws before " : "control leaves the invocation before ";
                emit("await_bypass", "If the await of " + path.name + " rejects, " + mode +
                    place + " is reached.", [path.site, ...path.sites, site], {guards: path.guards});
            }
            if (previous?.awaited) {
                for (const origin of previous.passed || []) {
                    emit("result_continuation", place + " is reached after " + previous.name +
                        " fulfills. That call receives " + origin.name + ", returned by " + origin.call + ".",
                        [...origin.sites, previous.site, site], state);
                }
            }
        }
        if (previous) {
            emit("call_order", place + " is reached after " + previous.name +
                (previous.awaited ? " fulfills." : " returns control."), [previous.site, site], state);
            if (previous.awaited && catcher && catcher.kind !== "escape" && previous.catcher === catcher) {
                emit("exception_bypass", "If " + previous.name + " throws or its await rejects, control reaches " + destination(catcher) +
                    " before " + place + ".", [previous.site, catcher.site, site], state);
            }
        }
        const current = {place, site, awaited};
        if (!focus) observedCalls.set(call, current);
        for (const exit of state.exits) retainBypass(exit, call, current);
        const separate = !sql && (effect.kind === "http_forward" || awaited && !depth && !state.inCleanup);
        const offset = separate && awaited && ts.isPropertyAccessExpression(method) &&
            method.name.text === "commitOffsets" && call.arguments.length === 1 && !ts.isSpreadElement(call.arguments[0])
            && exact(scope, unwrap(method.expression)).map(id => manual.get(id)).find(Boolean);
        const later = {place, site, guards: state.guards, offset};
        if (separate && state.helpers?.length) {
            downstreamCalls.push({...later, helpers: state.helpers});
        }
        if (offset) {
            emit("broker_offset_call", "The callback awaits the registered consumer's commitOffsets call.",
                [site, ...offset], state, "The broker determines whether the requested offsets advance. If the call fails, redelivery remains possible.");
            replayWindow(later, state.commit, [], state.guards);
        }
        if (separate && state.commit) {
            emit("post_commit_call", place + " is a separate call reached after the awaited " +
                state.commit.name + " fulfills.", [state.commit.site, site], state,
                "If the earlier request committed, this later call has its own outcome. The source establishes call order.");
        }
        const promise = effect.kind === "http_forward" || target && asyncFunction(target.node);
        if (promise && !awaited && ending !== "The returned expression" && !ts.isIfStatement(statement)) {
            emit("unawaited_call", "The source makes this call to " + place + " without awaiting it here.",
                [site], state, "A later local step can be reached while its result remains pending.");
        }
        const signature = awaited && (catcher || state.failed) ? repeatSignature(call) : undefined;
        if (signature && state.failed?.signature === signature && state.failed.site !== site) {
            emit("conditional_repeat", "This path explicitly calls " + place + " again after the earlier awaited call fails.",
                [state.failed.site, site], state, "The new call evaluates its operands again and takes its own runtime outcome.");
        }
        const failure = failureAt(catcher, site, place, awaited, state, signature, occurrence);
        if (failure) errors.push(failure);
        if (catcher?.kind === "catch" && !awaited && ending === "The returned expression" && (promise || !target)) {
            const resumed = state.cleanup ? " If finally allows the return to resume, its rejection propagates." : "";
            emit("return_rejection", "A later rejection of the promise returned from " + place +
                " does not enter this local catch." + resumed, [site, catcher.site], state,
                promise ? undefined : "This applies if the call returns a promise.", occurrence ? {_edgeCause: occurrence} : {});
        }
        const passed = [...new Set(call.arguments.flatMap(argument => useSite(argument, state)))].filter(origin => !origin.awaited);
        if (binding && (awaited || helper && !helper.async)) {
            const origin = {name: binding.name, call: place, awaited,
                sites: [...sites, helper?.target].filter(Boolean), sql: sql?.kind, requestSource: effect.requestSource};
            if (awaited) {
                emit("awaited_binding", binding.name + " receives the value from " + place + " if the await fulfills.", sites, state);
            }
            state = {...state, values: new Map([...state.values, [binding, origin]])};
        }
        if (query) {
            state = query.escape(state, [call.expression, ...call.arguments], evaluated);
            const origin = awaited && sql?.select ?
                {query: sql.select, call: callName(call), sites: [site, source(scope, sql.literal)]} :
                helper && (awaited || !helper.async) ? {helper, sites: [site, helper.target]} : undefined;
            const value = origin && {origin, path: [], sites: []};
            if (value) evaluated.set(node, value);
            if (binding) state = query.bind(state, binding, !captured.has(binding) && value, node);
        }
        const last = {site, name: place, awaited, catcher, passed};
        const commit = awaited && sql?.kind === "COMMIT" ? last : sql?.kind === "BEGIN" ? undefined : state.commit;
        const helpers = helper && awaited ? [...(state.helpers || []), helper].slice(-5) : state.helpers;
        const inheritedExits = depth || state.inCleanup ? state.awaitExits || [] : [];
        const awaitExits = awaited && !catcher ? [...inheritedExits, {name: place, site, sites: [], guards: state.guards}]
            .slice(-4) : inheritedExits;
        if (compatibility) {
            const receiver = (ts.isPropertyAccessExpression(method) || ts.isElementAccessExpression(method)) && method.expression;
            const values = receiver ? [receiver, ...call.arguments] : call.arguments;
            const messageValues = validated ? state.messageValues : compatibility.escapedValues(scope, values, state.messageValues);
            const value = validated || compatibility.abstract(scope, node, messageValues);
            state = {...state, messageValues, messageValidation: state.messageValidation || Boolean(validated),
                messageEvidence: validated ? [...new Set([...state.messageEvidence, site, target.id, ...validated.sites])] : state.messageEvidence};
            state = bindMessage(state, binding, value);
        }
        return {state: {...state, last, commit, helpers, awaitExits}, errors, evaluated};
    }
    function bounded(rows, limit) {
        if (compatibility && rows.length > limit) focus.incomplete();
        if (inventory && rows.length > limit) inventory.gap();
        return evenly(rows, limit);
    }
    function sequence(statements, states, catcher) {
        let normal = states, exits = [], errors = [];
        for (const statement of statements) {
            const next = [], results = normal.map(state => walk(statement, state, catcher));
            for (const result of results) {
                next.push(...result.normal);
                exits.push(...result.exits);
                errors.push(...(result.errors || []));
            }
            normal = bounded(next, 12);
            if (!normal.length || fuel <= 0) break;
        }
        return {normal, exits, errors};
    }
    function retainPredicate(expression, state, location, always = false) {
        if (!expression || !always && !predicates.permit(unwrap(expression))) return undefined;
        const predicate = predicates.tree(expression, node => source(scope, node));
        const description = predicates.summarize(predicate);
        emit("validation_predicate", "The " + location + " evaluates " + description + ".",
            predicates.sources(predicate), state,
            predicates.hasCallback(predicate) ?
                "The selected member implementation controls callback evaluation." : undefined, {predicate});
        return description;
    }
    function conditionState(expression, yes, state) {
        const condensed = caption(expression);
        const whole = condensed && !condensed.includes("…");
        const stated = retainPredicate(expression, state, "guard", !whole);
        const test = whole ? condensed : stated || condensed || "the cited condition";
        const site = source(scope, expression);
        const observed = compatibility ? compatibility.observe(scope, expression, state, yes) : state;
        const checked = {...observed, guards: [...state.guards, {text: test + " is " + yes, source: site,
            ...(focus ? {predicate: expression.getStart(scope.file), outcome: yes, body: scope.id} : {})}]};
        const checkedFields = new Map();
        function findFields(node) {
            if (ts.isPropertyAccessExpression(node) && !ts.isOptionalChain(node) &&
                    ["rowCount", "rows", "affectedRows"].includes(node.name.text)) {
                const base = unwrap(node.expression);
                const origin = ts.isIdentifier(base) && state.values.get(symbol(base));
                if (origin?.sql === "INSERT") checkedFields.set(origin, node);
            }
            for (const child of orderedChildren(node)) if (child) findFields(child);
        }
        findFields(expression);
        for (const [origin, member] of checkedFields) {
            emit("sql_result_check", "The branch tests " + test + " using " + member.name.text +
                " on the value received by " + origin.name + " from the awaited INSERT request.",
                [...origin.sites, site], checked, "The provider determines the result shape and what its values mean.");
        }
        return checked;
    }
    function returnContract(expression) {
        const node = unwrap(expression);
        if (!node) return {};
        if (ts.isObjectLiteralExpression(node) && node.properties.length <= 12) {
            const fields = new Map();
            for (const member of node.properties) {
                const shorthand = ts.isShorthandPropertyAssignment(member);
                if (!(shorthand || ts.isPropertyAssignment(member)) ||
                        !(ts.isIdentifier(member.name) || ts.isStringLiteral(member.name)) || fields.has(member.name.text)) return {};
                const value = caption(shorthand ? member.name : member.initializer);
                if (!value) return {};
                fields.set(member.name.text, {name: member.name.text, value, source: source(scope, member)});
            }
            return fields.size ? {returnedFields: [...fields.values()]} : {};
        }
        if (predicates.permit(node) || ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.ExclamationToken ||
                ts.isBinaryExpression(node) && [ts.SyntaxKind.EqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsToken,
                    ts.SyntaxKind.EqualsEqualsEqualsToken, ts.SyntaxKind.ExclamationEqualsEqualsToken,
                    ts.SyntaxKind.LessThanToken, ts.SyntaxKind.LessThanEqualsToken, ts.SyntaxKind.GreaterThanToken,
                    ts.SyntaxKind.GreaterThanEqualsToken, ts.SyntaxKind.InstanceOfKeyword, ts.SyntaxKind.InKeyword]
                    .includes(node.operatorToken.kind)) {
            return {predicate: predicates.tree(node, item => source(scope, item))};
        }
        return {};
    }
    function abrupt(statement, state, catcher) {
        const returning = ts.isReturnStatement(statement);
        const returnedTest = returning && retainPredicate(statement.expression, state, "return expression");
        const described = statement.expression && caption(statement.expression);
        const value = described && !described.includes("…") ? described : returnedTest || described;
        const site = source(scope, statement);
        const expression = evaluate(statement.expression, statement, state, catcher,
            returning ? "The returned expression" : "The thrown expression");
        const text = returning ? (value ? "Returns " + value : statement.expression ? "Returns the evaluated value" : "Returns without a value")
            : "Throws " + (value || "the evaluated value");
        const pending = state.cleanup ? " This " + (returning ? "return" : "exception") + " can remain pending through finally." : "";
        emit(returning ? "return" : "throw", text + "." + pending, [site, state.cleanup?.site], state, statement.expression ?
            "The outcome applies if evaluation of the operand completes normally." : undefined,
            state.errorArrival?.occurrence ? {_edgeCause: state.errorArrival.occurrence} : {});
        if (!returning && catcher) {
            const transfer = catcher.kind === "escape" ? "leaves this finally abruptly" : "enters " + destination(catcher);
            emit("exception_route", "This explicit throw " + transfer + ".", [site, catcher.site], state,
                "The operand must be evaluated for this value to be thrown.");
        }
        const contract = returning ? returnContract(statement.expression) : {};
        const reads = returning && query ? query.outputs(statement.expression, expression.state, expression.evaluated) : [];
        const exit = {kind: returning ? "return" : "throw", statement, state, text, site, value, contract, reads, resume: expression.state};
        return {normal: [], exits: [exit], errors: [...expression.errors,
            !returning && catcher && {...expression.state, last: undefined, exits: [],
                ...(focus ? {writeTransactions: new Map()} : {}), guards: [...state.guards,
                {text: "the explicit throw is reached", source: site}]}].filter(Boolean)};
    }
    function resolveFinally(block, result, enclosing, state) {
        const site = source(scope, block);
        emit("finally", "Control enters this finally when it leaves the try or catch. Normal cleanup resumes the pending outcome.",
            [site], state, "An abrupt cleanup can replace a pending return or exception.");
        const normal = [], exits = [], errors = [];
        const channels = [
            ["normal", result.normal.map(path => ({path}))],
            ["return", result.exits.filter(exit => exit.kind === "return").map(exit => ({path: exit.resume, exit}))],
            ["throw", (result.errors || []).map(path => ({path}))],
        ];
        for (const [outcome, paths] of channels) {
            const selected = compatibility && focus.precedesCleanup(scope, block)
                ? evenly(paths, 6) : bounded(paths, 6);
            for (const {path, exit} of selected) {
                const reason = {normal: "the try or catch completes normally", return: "a return is pending",
                    throw: "an exception is pending"}[outcome];
                const entered = {...path, cleanup: state.cleanup, inCleanup: site, exits: [],
                    guards: [...path.guards, {text: reason + " at finally", source: site}]};
                const cleaned = walk(block, entered, enclosing || {kind: "escape", site});
                for (const resumed of cleaned.normal) {
                    const continuation = {...resumed, inCleanup: state.inCleanup, exits: state.exits};
                    if (outcome === "normal") {
                        normal.push({...continuation, exits: [...path.exits, ...resumed.exits]});
                    } else if (outcome === "return") {
                        exits.push({...exit, resume: continuation});
                    } else {
                        if (enclosing) errors.push(continuation);
                        emit("finally_resume", "The pending exception continues after normal cleanup.", [site, resumed.last?.site], resumed);
                    }
                }
                for (const replaced of cleaned.exits) {
                    if (replaced.kind === "throw" && enclosing) continue;
                    exits.push(replaced);
                    if (outcome !== "normal") {
                        emit("finally_override", replaced.text + " from finally and can replace the pending " +
                            (outcome === "return" ? "return." : "exception."), [site, replaced.site], replaced.state);
                    }
                }
                if (enclosing) errors.push(...(cleaned.errors || []));
                if (outcome !== "normal" && cleaned.errors?.length) {
                    const failure = cleaned.errors[0];
                    emit("finally_override", "An abrupt cleanup transfers control from finally and can replace the pending " +
                        (outcome === "return" ? "return." : "exception."), [site], failure);
                }
            }
        }
        return {normal, exits, errors};
    }
    function protectedBlock(statement, state, enclosing) {
        const final = statement.finallyBlock;
        const handler = statement.catchClause;
        const cleanup = final && {kind: "finally", site: source(scope, final)};
        const handlerRoute = handler && {kind: "catch", site: source(scope, handler)};
        const next = cleanup || enclosing;
        const entered = cleanup ? {...state, cleanup} : state;
        const tried = walk(statement.tryBlock, entered, handlerRoute || next);
        let result = tried;
        if (handler) {
            const catchGuard = {text: "evaluation in the protected block enters the catch", source: handlerRoute.site,
                group: scope.id + ":" + handler.getStart()};
            const inputs = (tried.errors || []).map(input => ({...input, values: removeMutable(state).values,
                ...(query ? {queryValues: new Map()} : {}),
                guards: [...input.guards, catchGuard]}));
            if (inventory && !inputs.length && inventory.containsCall(handler.block)) inventory.gap(handler);
            const caught = sequence(handler.block.statements, bounded(inputs, 12), next);
            const terminated = !final ? caught.exits.flatMap(exit => {
                const arrival = exit.resume.errorArrival;
                if (!arrival || !(exit.kind === "return" || exit.kind === "throw" && !enclosing)) return [];
                return [{...arrival, mode: exit.kind, sites: [handlerRoute.site, exit.site], guards: exit.resume.guards}];
            }) : [];
            const continued = tried.normal.map(path => ({...path,
                awaitExits: [...(path.awaitExits || []), ...terminated].slice(-6)}));
            const recovered = caught.normal.map(path => ({...path, errorArrival: state.errorArrival}));
            result = {normal: [...continued, ...recovered], exits: [
                ...tried.exits.filter(exit => exit.kind !== "throw"), ...caught.exits], errors: caught.errors};
        }
        return final ? resolveFinally(final, result, enclosing, state) : result;
    }
    function forOf(statement, state, catcher) {
        const site = source(scope, statement);
        const collection = caption(statement.expression) || "the cited iterable";
        const evaluated = evaluate(statement.expression, statement, state, catcher, "The loop input");
        const declaration = ts.isVariableDeclarationList(statement.initializer) && single(statement.initializer.declarations);
        const start = evaluated.state;
        const outer = start.iterations || [];
        const binding = declaration && ts.isIdentifier(declaration.name) && (declaration.parent.flags & ts.NodeFlags.Const) &&
            symbol(declaration.name);
        const loop = binding && {binding, element: declaration.name.text, collection, site};
        // A prior iteration may change mutable bindings and the current element.
        const entered = {...removeMutable(start), iterations: loop ? [...outer, loop] : outer,
            ...(query ? {queryValues: new Map()} : {}),
            guards: [...start.guards, {text: (loop ? collection + " yields " + loop.element :
                "the cited iterable yields an element") + " for this iteration", source: site}]};
        if (compatibility) entered.messageValues = changesLocal(statement.statement)
            ? new Map() : compatibility.dropRecords(entered.messageValues);
        const body = walk(statement.statement, entered, catcher);
        const normal = {...removeMutable(start), ...(query ? {queryValues: new Map()} : {}), guards: [...start.guards,
            {text: "iteration over " + collection + " completes normally", source: site}]};
        if (compatibility) normal.messageValues = compatibility.dropRecords(normal.messageValues);
        return {normal: [normal], exits: body.exits.filter(exit => exit.kind === "return" || exit.kind === "throw"),
            errors: [...evaluated.errors, ...(body.errors || [])]};
    }
    function elementGuard(expression, state, yes, no) {
        for (const loop of state.iterations || []) {
            function reads(node) {
                if (ts.isFunctionLike(node) || ts.isTypeNode(node)) return false;
                if (ts.isIdentifier(node) && symbol(node) === loop.binding) return true;
                return Boolean(ts.forEachChild(node, child => reads(child) ? true : undefined));
            }
            if (!reads(expression)) continue;
            const predicate = predicates.tree(expression, node => source(scope, node));
            const describe = arm => {
                const paths = [...new Set(arm.exits.map(exit => exit.text))];
                if (arm.normal.length) paths.push("continues the loop body");
                return paths.length ? paths.join(" or ") : undefined;
            };
            const yesOutcome = describe(yes), noOutcome = describe(no);
            const text = "For a reached " + loop.element + " from " + loop.collection + ", the loop tests " +
                predicates.summarize(predicate) + "." +
                (yesOutcome ? " True branch: " + yesOutcome + "." : "") +
                (noOutcome ? " False branch: " + noOutcome + "." : "");
            emit("iteration_guard", text, [loop.site, ...predicates.sources(predicate),
                ...yes.exits.map(exit => exit.site), ...no.exits.map(exit => exit.site)], state, undefined,
                {element: loop.element, collection: loop.collection, predicate});
        }
    }
    function walk(statement, state, catcher) {
        if (--fuel <= 0) {
            if (compatibility) focus.incomplete();
            inventory && inventory.gap(statement);
            return empty();
        }
        if (ts.isBlock(statement)) return sequence(statement.statements, [state], catcher);
        if (ts.isFunctionDeclaration(statement) || ts.isEmptyStatement(statement)) return {normal: [state], exits: []};
        if (ts.isForOfStatement(statement)) return forOf(statement, state, catcher);
        if ((ts.isBreakStatement(statement) || ts.isContinueStatement(statement)) && !statement.label) {
            if (inventory && state.cleanup) inventory.gap(statement);
            const kind = ts.isBreakStatement(statement) ? "break" : "continue";
            const text = kind === "break" ? "Leaves the loop" : "Starts the next iteration";
            return {normal: [], exits: [{kind, text, state, site: source(scope, statement), resume: state}]};
        }
        if (ts.isIfStatement(statement)) {
            const predicate = unwrap(statement.expression);
            const evaluated = ts.isPrefixUnaryExpression(predicate) && predicate.operator === ts.SyntaxKind.ExclamationToken
                ? predicate.operand : predicate;
            const check = evaluate(evaluated, statement, state, catcher, "The condition");
            const value = truth(statement.expression);
            const yes = value === false ? empty() : walk(statement.thenStatement,
                conditionState(statement.expression, true, check.state), catcher);
            const noState = conditionState(statement.expression, false, check.state);
            const no = value === true ? empty() : statement.elseStatement
                ? walk(statement.elseStatement, noState, catcher) : {normal: [noState], exits: []};
            if (!focus && statement.elseStatement && value === undefined) {
                for (const [arm, exits, opposite] of [[statement.thenStatement, yes.exits, statement.elseStatement],
                    [statement.elseStatement, no.exits, statement.thenStatement]]) {
                    const terminal = ts.isBlock(arm) ? single(arm.statements) : arm;
                    const direct = exits.filter(exit => exit.kind === "return" && exit.statement === terminal);
                    if (!direct.length) continue;
                    for (const [call, current] of observedCalls) {
                        if (opposite.pos <= call.pos && call.end <= opposite.end) {
                            for (const exit of direct) retainBypass(exit, call, current);
                        }
                    }
                }
            }
            elementGuard(statement.expression, state, yes, no);
            const addExits = (normal, alternative) => normal.map(path => ({...path,
                exits: [...path.exits, ...alternative.exits.filter(exit => exit.kind === "return" || !catcher)]}));
            return {normal: [...addExits(yes.normal, no), ...addExits(no.normal, yes)],
                exits: [...yes.exits, ...no.exits], errors: [...check.errors, ...(yes.errors || []), ...(no.errors || [])]};
        }
        if (ts.isReturnStatement(statement) || ts.isThrowStatement(statement)) return abrupt(statement, state, catcher);
        if (ts.isTryStatement(statement)) return protectedBlock(statement, state, catcher);
        if (ts.isVariableStatement(statement)) {
            const errors = [];
            for (const declaration of statement.declarationList.declarations) {
                const binding = ts.isIdentifier(declaration.name) && localBinding(declaration.name);
                const evaluated = evaluate(declaration.initializer, statement, state, catcher, undefined, binding);
                state = evaluated.state;
                if (query && !binding && declaration.initializer) {
                    const pattern = declaration.name;
                    const fields = ts.isObjectBindingPattern(pattern) && (declaration.parent.flags & ts.NodeFlags.Const) &&
                        pattern.elements.every(member => !member.dotDotDotToken && !member.initializer &&
                            ts.isIdentifier(member.name) && !captured.has(symbol(member.name)) &&
                            queryResults.name(member.propertyName || member.name) !== undefined);
                    const value = fields && query.read(declaration.initializer, state, evaluated.evaluated);
                    if (value) {
                        for (const member of pattern.elements) {
                            const local = symbol(member.name);
                            if (local) {
                                state = query.bind(state, local, queryResults.bindField(value,
                                    queryResults.name(member.propertyName || member.name), source(scope, member)), member);
                            }
                        }
                    } else {
                        state = query.escape(state, [declaration.initializer], evaluated.evaluated);
                    }
                }
                if (compatibility && binding === false && ts.isObjectBindingPattern(declaration.name) &&
                        declaration.parent.flags & ts.NodeFlags.Const) {
                    const values = new Map(state.messageValues);
                    const input = compatibility.abstract(scope, declaration.initializer, values);
                    compatibility.bind(scope, values, declaration.name, input);
                    for (const item of captured) values.delete(item);
                    state = {...state, messageValues: values};
                }
                errors.push(...evaluated.errors);
            }
            return {normal: [state], exits: [], errors};
        }
        if (ts.isExpressionStatement(statement)) {
            const value = unwrap(statement.expression);
            const assignment = ts.isBinaryExpression(value) && value.operatorToken.kind === ts.SyntaxKind.EqualsToken
                && ts.isIdentifier(value.left);
            const binding = assignment && localBinding(value.left);
            const evaluated = evaluate(assignment ? value.right : value, statement, state, catcher, undefined,
                binding);
            // A formal can be assigned even though it is outside localBinding.
            let next = compatibility && assignment && !binding
                ? bindMessage(evaluated.state, symbol(value.left), undefined) : evaluated.state;
            if (query && assignment && !binding) next = query.escape(next, [value.right], evaluated.evaluated);
            if (request.callback.active && request.roots.includes(scope.id)) {
                const op = ts.isBinaryExpression(value) && value.operatorToken.kind;
                const target = op >= ts.SyntaxKind.FirstAssignment && op <= ts.SyntaxKind.LastAssignment &&
                    ![ts.SyntaxKind.AmpersandAmpersandEqualsToken, ts.SyntaxKind.BarBarEqualsToken,
                        ts.SyntaxKind.QuestionQuestionEqualsToken].includes(op) ? value.left :
                    (ts.isPrefixUnaryExpression(value) || ts.isPostfixUnaryExpression(value)) &&
                    [ts.SyntaxKind.PlusPlusToken, ts.SyntaxKind.MinusMinusToken].includes(value.operator) ? value.operand : undefined;
                if (target && (ts.isPropertyAccessExpression(target) || ts.isElementAccessExpression(target) ||
                        ts.isIdentifier(target) && !localBinding(target))) {
                    const site = source(scope, value);
                    const write = caption(value) || short(value.getText(scope.file), 160) || "the cited state assignment";
                    const preceding = next.last?.awaited ? next.last : undefined;
                    const text = "The callback reaches " + write +
                        (preceding ? " after " + preceding.name + " fulfills." : ".");
                    emit("guarded_state_write", text, [site, preceding?.site], next,
                        "Reaching the write depends on the cited control path and evaluation of its operands.");
                    for (const exit of next.awaitExits || []) {
                        emit("await_bypass", "If the await of " + exit.name + " rejects, control leaves this path before " + write + ".",
                            [exit.site, ...exit.sites, site], {guards: exit.guards});
                    }
                }
            }
            return {normal: [next], exits: [], errors: evaluated.errors};
        }
        if (compatibility) focus.incomplete();
        inventory && inventory.gap(statement);
        return empty();
    }
    function localOutcome(kind, site, value, state, contract = {}, reads = []) {
        if (reads.length && request.roots.includes(scope.id)) {
            queryOutputs.push({reads, site, guards: state.guards, sink: {kind: "return"}});
        }
        return {kind, site, value, guards: state.guards.slice(inherited.guards.length), commit: state.commit, contract, reads};
    }
    if (ts.isBlock(scope.node.body)) {
        const result = walk(scope.node.body, initial);
        const outcomes = result.exits.filter(exit => exit.kind === "return" || exit.kind === "throw")
            .map(exit => localOutcome(exit.kind, exit.site, exit.value, exit.resume, exit.contract, exit.reads));
        for (const normal of result.normal) {
            outcomes.push(localOutcome("normal", source(scope, scope.node.body), undefined, normal));
        }
        bodyOutcomes.set(scope.id, outcomes);
    } else {
        const {state, evaluated} = evaluate(scope.node.body, scope.node.body, initial, undefined, "The returned expression");
        const returnedTest = retainPredicate(scope.node.body, initial, "return expression");
        const described = caption(scope.node.body);
        const value = described && !described.includes("…") ? described : returnedTest || described;
        const site = source(scope, scope.node.body);
        if (value) emit("return", "Returns " + value + ".", [site], initial,
            "The outcome applies if the expression completes normally.");
        bodyOutcomes.set(scope.id, [localOutcome("return", site, value, state, returnContract(scope.node.body),
            query ? query.outputs(scope.node.body, state, evaluated) : [])]);
    }
    for (const [exit, skipped] of bypasses) {
        const calls = [...skipped.calls.values()];
        const descriptions = [...new Set(calls.map(item => (item.awaited ? "the await of " : "") + item.place))];
        emit("early_exit", exit.text + " before reaching " + descriptions.join(", ") + ".",
            [exit.site, ...calls.map(item => item.site)], exit.state,
            "The cited call sites are bypassed if the exit takes effect." + (skipped.limited ? " Further bypassed sites remain open." : ""),
            {scope: "CALLABLE_SOURCE_CONTINUATION", coverage: skipped.limited ? "PARTIAL" : "PROVEN_RETAINED_SITES"});
    }
    inventory && inventory.finish();
}
for (const entry of active) {
    const location = locations.get(entry.id);
    const body = load(entry.id);
    if (!location || !body) {
        if (compatibility) focus.incomplete();
        continue;
    }
    const key = JSON.stringify([location.excerpt.id, location.start, location.end]);
    if (!focus && visited.has(key)) continue;
    visited.add(key);
    scan(body, entry);
}
if (focus) {
    if (focus.ambiguous()) {
        facts.length = 0;
        emit("write_boundary", "The selected TypeScript source range matches more than one reachable call.", [request.focus.site]);
    } else {
        focus.finish(compatibility?.unsupported);
    }
    const compact = require("./clew_prepared_typescript_alternatives.cjs");
    process.stdout.write(JSON.stringify(compact(facts, [], 22, evenly, request.roots)));
    return;
}
for (const call of helperCalls) {
    for (const outcome of evenly(bodyOutcomes.get(call.target) || [], 8)) {
        if (outcome.kind === "normal") continue;
        const contract = outcome.contract;
        const sources = [call.site, call.target, outcome.site, ...(contract.predicate ? predicates.sources(contract.predicate) : []),
            ...(contract.returnedFields || []).map(field => field.source)];
        const state = {guards: [...call.guards, ...outcome.guards]};
        if (outcome.kind === "throw") {
            const transfer = call.async ? "the helper's returned promise rejects" : "the helper call throws";
            emit("helper_outcome", "If " + call.name + " reaches its explicit throw" +
                (outcome.value ? " of " + outcome.value : "") + ", " + transfer + ".", sources, state);
        } else if (call.destination && outcome.value) {
            emit("helper_result", "If " + call.name + " returns " + outcome.value +
                (call.awaited ? " and the await fulfills" : "") + ", the returned value reaches " + call.destination + ".",
                sources, state, undefined, contract);
        } else {
            const returned = outcome.value ? "returns " + outcome.value : "returns without a value";
            emit("helper_outcome", "The retained " + call.name + " implementation " + returned + " on this path.",
                sources, state, "This applies if evaluation reaches the source return and its operand completes normally.", contract);
        }
    }
}
for (const effect of downstreamCalls) {
    for (const helper of effect.helpers) {
        const paths = (bodyOutcomes.get(helper.target) || []).filter(path => path.kind !== "throw" && path.commit);
        for (const path of evenly(paths, 3)) {
            const returns = path.value ? "returning " + path.value : "completing normally";
            emit("post_helper_call", effect.place + " is reached after the await of " + helper.name +
                " fulfills. On the cited helper path, " + helper.name + " awaits " + path.commit.name + " before " + returns + ".",
                [helper.site, helper.target, path.commit.site, path.site, effect.site],
                {guards: [...new Set([...effect.guards, ...helper.guards, ...path.guards])]},
                "If the SQL request committed, the later call still has its own outcome. Runtime coordination remains open.");
            replayWindow(effect, path.commit, [helper.site, helper.target, path.site],
                [...new Set([...effect.guards, ...helper.guards, ...path.guards])]);
        }
    }
}
for (const {reads, sink, site, guards} of queryOutputs) {
    for (const read of reads) {
        for (const result of queryResults.resolve(read, bodyOutcomes)) {
            const path = queryResults.pathText(result.path);
            const output = queryResults.pathText(result.output);
            const target = sink.kind === "return" ? "The returned value" :
                "The argument to " + sink.call + (sink.status ? " after passing " + sink.status + " to the status method" : "");
            const value = path ? "the " + path + " member of the awaited value" : "the awaited value";
            emit("sql_result_flow", target + (output ? " supplies " + output + " from " : " is ") + value +
                " from " + result.call + " with the static SELECT from " + result.query.table + ".",
                [...result.sites, site], {guards: [...guards, ...result.guards]},
                "This applies when evaluation completes normally. The receiver binding, row values and wire representation remain open.",
                {result: {table: result.query.table, path: result.path, output: result.output, sink}});
        }
    }
}
const compact = require("./clew_prepared_typescript_alternatives.cjs");
const output = compact(facts, priority, limit, evenly, request.roots);
for (const fact of output) {
    fact.sources = [...new Set(fact.sources.map(id => discovered.has(id) ? request.fileRoot : id))];
    if (fact._edge && discovered.has(fact._edge.site)) delete fact._edge;
}
process.stdout.write(JSON.stringify(output));
