/* Find source-bound registrations before selecting callback control-flow roots. */
const path = require("node:path").posix;

module.exports = function discover(ts, files) {
    const admitted = new Map(Object.entries(files).map(([file, text]) =>
        [file, ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true)]));
    const usable = new Map([...admitted].filter(([, source]) => !source.parseDiagnostics.length &&
        !source.isDeclarationFile));
    const fileName = name => path.normalize(name).replace(/^\/+/, "");
    const options = {noLib: true};
    const host = ts.createCompilerHost(options);
    host.getCurrentDirectory = () => "/";
    host.getSourceFile = name => usable.get(fileName(name));
    host.fileExists = name => usable.has(fileName(name));
    host.readFile = name => host.getSourceFile(name)?.text;
    host.resolveModuleNames = (names, containing) => names.map(name => {
        if (!name.startsWith("./") && !name.startsWith("../")) return undefined;
        const base = fileName(path.join(path.dirname(containing), name));
        const extensions = [".ts", ".tsx", ".js", ".jsx", ".mts", ".cts", ".mjs", ".cjs"];
        const suffix = path.extname(base);
        const stems = extensions.includes(suffix) ? [base.slice(0, -suffix.length)] : [base, base + "/index"];
        const candidates = new Set([base, ...stems.flatMap(stem => extensions.map(ext => stem + ext))]
            .filter(file => admitted.has(file)));
        if (candidates.size !== 1) return undefined;
        const file = [...candidates][0];
        if (usable.has(file)) return {resolvedFileName: file, extension: path.extname(file)};
    });
    const checker = ts.createProgram([...usable.keys()], options, host).getTypeChecker();
    const registrations = [];
    const subscriptions = new Map();
    const assigned = new Set(), propertyWrites = [], replaced = new Map();

    function declaration(symbol, follow = false) {
        if (!symbol) return undefined;
        const items = symbol.declarations || [];
        if (items.length !== 1 || !usable.has(items[0].getSourceFile().fileName)) return undefined;
        const item = items[0];
        if (!follow || !(symbol.flags & ts.SymbolFlags.Alias)) return item;
        if (item.isTypeOnly || (ts.isImportSpecifier(item) && item.parent.parent.isTypeOnly)) return undefined;
        return declaration(checker.getAliasedSymbol(symbol));
    }
    const binding = (node, follow) => ts.isIdentifier(node) && declaration(checker.getSymbolAtLocation(node), follow);
    const constant = item => item && !assigned.has(item) && ts.isVariableDeclaration(item) &&
        item.parent.flags & ts.NodeFlags.Const ? item.initializer : undefined;
    const hasBody = node => node && ts.isFunctionLike(node) && node.body && !node.asteriskToken;

    function recordTarget(node) {
        if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isNonNullExpression(node)) {
            recordTarget(node.expression);
        } else if (ts.isIdentifier(node)) {
            const item = binding(node, true);
            if (item) assigned.add(item);
        } else if (ts.isPropertyAccessExpression(node)) {
            propertyWrites.push([node.expression, node.name.text]);
        } else if (ts.isElementAccessExpression(node)) {
            const key = node.argumentExpression;
            propertyWrites.push([node.expression, key && ts.isStringLiteralLike(key) ? key.text : "*"]);
        } else if (ts.isArrayLiteralExpression(node)) {
            for (const item of node.elements) recordTarget(ts.isSpreadElement(item) ? item.expression : item);
        } else if (ts.isObjectLiteralExpression(node)) {
            for (const item of node.properties) {
                if (ts.isShorthandPropertyAssignment(item)) recordTarget(item.name);
                else if (ts.isPropertyAssignment(item)) recordTarget(item.initializer);
                else if (ts.isSpreadAssignment(item)) recordTarget(item.expression);
            }
        } else if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken) {
            recordTarget(node.left);
        }
    }
    function collectWrites(node) {
        if (ts.isBinaryExpression(node) && node.operatorToken.kind >= ts.SyntaxKind.FirstAssignment &&
                node.operatorToken.kind <= ts.SyntaxKind.LastAssignment) {
            recordTarget(node.left);
        } else if ((ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) &&
                [ts.SyntaxKind.PlusPlusToken, ts.SyntaxKind.MinusMinusToken].includes(node.operator)) {
            recordTarget(node.operand);
        } else if (ts.isDeleteExpression(node)) {
            recordTarget(node.expression);
        } else if ((ts.isForOfStatement(node) || ts.isForInStatement(node)) && !ts.isVariableDeclarationList(node.initializer)) {
            recordTarget(node.initializer);
        }
        ts.forEachChild(node, collectWrites);
    }
    for (const source of usable.values()) collectWrites(source);

    function objectOrigin(node, depth = 0) {
        if (!node || depth >= 4) return undefined;
        const item = binding(node, true);
        const value = constant(item);
        if (value) return ts.isIdentifier(value) ? objectOrigin(value, depth + 1) : item;
    }
    for (const [receiver, name] of propertyWrites) {
        const owner = objectOrigin(receiver);
        if (!owner) continue;
        const names = replaced.get(owner) || new Set();
        names.add(name);
        replaced.set(owner, names);
    }
    const isReplaced = (owner, name) => replaced.get(owner)?.has(name) || replaced.get(owner)?.has("*");

    function selectedCallable(item, depth) {
        if (!item || assigned.has(item)) return undefined;
        const initializer = constant(item);
        if (initializer && ts.isIdentifier(initializer)) return callable(initializer, depth + 1);
        const target = initializer || item;
        return hasBody(target) ? target : undefined;
    }
    function callable(node, depth = 0) {
        if (!node || depth >= 4) return undefined;
        if (ts.isArrowFunction(node) || ts.isFunctionExpression(node)) return hasBody(node) && node;
        return selectedCallable(binding(node, true), depth);
    }
    function imported(node, name, modules) {
        const item = binding(node);
        return item && !assigned.has(item) && ts.isImportSpecifier(item) && !item.isTypeOnly && !item.parent.parent.isTypeOnly &&
            (item.propertyName || item.name).text === name &&
            modules.includes(item.parent.parent.parent.moduleSpecifier.text);
    }
    function member(node, name) {
        return ts.isCallExpression(node) && !node.questionDotToken &&
            ts.isPropertyAccessExpression(node.expression) && !node.expression.questionDotToken &&
            node.expression.name.text === name && node.expression.expression;
    }
    function component(node, name, method) {
        const item = objectOrigin(node);
        if (!item || isReplaced(item, method)) return undefined;
        const receiver = member(item.initializer, name);
        const kafkaOwner = receiver && objectOrigin(receiver);
        const kafka = kafkaOwner && !isReplaced(kafkaOwner, name) && kafkaOwner.initializer;
        if (kafka && ts.isNewExpression(kafka) && imported(kafka.expression, "Kafka", ["kafkajs"])) return {item, kafka};
    }
    const consumer = (node, method) => component(node, "consumer", method)?.item;
    function properties(node) {
        if (!node || !ts.isObjectLiteralExpression(node)) return undefined;
        const result = new Map();
        for (const item of node.properties) {
            if (!item.name || !(ts.isIdentifier(item.name) || ts.isStringLiteral(item.name)) ||
                    result.has(item.name.text)) return undefined;
            result.set(item.name.text, item);
        }
        return result;
    }
    function range(node) {
        const source = node.getSourceFile();
        const start = node.getStart(source);
        return {file: source.fileName, start: Buffer.byteLength(source.text.slice(0, start)),
            end: Buffer.byteLength(source.text.slice(0, node.getEnd()))};
    }
    function caption(node) {
        const named = node.name || (ts.isVariableDeclaration(node.parent) && node.parent.name);
        return named && ts.isIdentifier(named) ? named.text : undefined;
    }
    function callbackOf(property) {
        if (!property) return undefined;
        if (ts.isMethodDeclaration(property) && hasBody(property)) return property;
        if (ts.isPropertyAssignment(property)) return callable(property.initializer);
        if (ts.isShorthandPropertyAssignment(property)) {
            return selectedCallable(declaration(checker.getShorthandAssignmentValueSymbol(property), true), 0);
        }
    }
    function context(root, receiver) {
        const visited = new Set([root]), helpers = [], calls = [], offsets = [], publications = [];
        function walk(node) {
            if (ts.isFunctionLike(node)) return;
            if (ts.isCallExpression(node) && !node.questionDotToken && !node.arguments.some(ts.isSpreadElement)) {
                const target = callable(node.expression);
                if (target && (visited.has(target) || visited.size < 8) && calls.length < 24) {
                    calls.push({site: range(node), target: range(target)});
                    if (!visited.has(target)) {
                        visited.add(target);
                        helpers.push(range(target));
                        walk(target.body);
                    }
                }
                const producer = component(member(node, "send"), "producer", "send");
                if (producer && publications.length < 8) {
                    publications.push({call: range(node), producer: range(producer.item), client: range(producer.kafka)});
                }
                const use = member(node, "commitOffsets");
                if (use && receiver && consumer(use, "commitOffsets") === receiver && offsets.length < 8) {
                    offsets.push({use: range(use), call: range(node), declaration: range(receiver),
                        consumer: range(receiver.initializer)});
                }
            }
            ts.forEachChild(node, walk);
        }
        walk(root.body);
        return {helpers, calls, offsets, publications};
    }
    function visit(node) {
        const subscription = consumer(member(node, "subscribe"), "subscribe");
        if (subscription) {
            const subscribed = subscriptions.get(subscription) || [];
            if (subscribed.length < 9) subscribed.push(node);
            subscriptions.set(subscription, subscribed);
        }
        const runner = consumer(member(node, "run"), "run");
        if (runner && node.arguments.length === 1 && registrations.length < 16) {
            const options = properties(node.arguments[0]);
            const handler = callbackOf(options?.get("eachMessage"));
            if (handler) registrations.push({node, handler, runner, options, kind: "EACH_MESSAGE"});
        }
        if (ts.isCallExpression(node) && !node.questionDotToken && ts.isIdentifier(node.expression) &&
                node.arguments.length === 2 && ts.isNumericLiteral(node.arguments[1]) &&
                Number(node.arguments[1].text) > 0 && registrations.length < 16 &&
                (imported(node.expression, "setInterval", ["node:timers", "timers"]) ||
                    node.expression.text === "setInterval" && !checker.getSymbolAtLocation(node.expression))) {
            const handler = callable(node.arguments[0]);
            if (handler) registrations.push({node, handler, kind: "INTERVAL"});
        }
        ts.forEachChild(node, visit);
    }
    for (const source of usable.values()) visit(source);
    function setupStatement(call, requireAwait) {
        let expression = call;
        if (ts.isAwaitExpression(expression.parent)) expression = expression.parent;
        else if (requireAwait) return;
        const statement = expression.parent;
        if (ts.isExpressionStatement(statement) &&
            (ts.isBlock(statement.parent) || ts.isSourceFile(statement.parent))) return statement;
    }
    function topicScalar(node, evidence, depth = 0) {
        if (!node || depth >= 4) return;
        if (ts.isStringLiteralLike(node)) {
            if (/^[A-Za-z0-9._-]{1,249}$/.test(node.text) && node.text !== "." && node.text !== "..") return node.text;
            return;
        }
        if (ts.isParenthesizedExpression(node) || ts.isAsExpression(node) || ts.isSatisfiesExpression(node)) {
            return topicScalar(node.expression, evidence, depth);
        }
        const item = binding(node, true);
        if (!item || item.getSourceFile() === node.getSourceFile() && item.end > node.getStart()) return;
        const result = topicScalar(constant(item), evidence, depth + 1);
        if (result) evidence.push(range(item));
        return result;
    }
    function topicNames(call) {
        if (call.arguments.length !== 1) return;
        const options = properties(call.arguments[0]);
        const single = options?.get("topic"), multiple = options?.get("topics");
        if (Boolean(single) === Boolean(multiple)) return;
        const item = single || multiple;
        if (!ts.isPropertyAssignment(item)) return;
        const initializer = item.initializer;
        if (multiple && !ts.isArrayLiteralExpression(initializer)) return;
        const entries = single ? [initializer] : initializer.elements;
        if (!entries.length || entries.length > 8) return;
        const evidence = [];
        const topics = entries.map(entry => topicScalar(entry, evidence));
        if (topics.every(Boolean)) return {topics, evidence};
    }
    function subscriptionSetup(run, runner) {
        const calls = subscriptions.get(runner) || [];
        const registration = setupStatement(run, false);
        const topics = new Set(), proof = [], topicSources = [];
        let open = calls.length > 8;
        for (const call of calls.slice(0, 8)) {
            const preceding = setupStatement(call, true);
            const names = registration && preceding?.parent === registration.parent &&
                preceding.end <= registration.getStart() && topicNames(call);
            if (!names || topics.size + names.topics.length > 8) {
                open = true;
                continue;
            }
            for (const topic of names.topics) topics.add(topic);
            proof.push(range(call));
            topicSources.push(...names.evidence);
        }
        return {topics: [...topics], subscriptions: proof, topicSources, topicCoverageOpen: open || !topics.size};
    }
    return registrations.map(({node, handler, runner, options, kind}) => {
        const autoCommit = options?.get("autoCommit");
        return {kind, name: caption(handler) || (kind === "INTERVAL" ? "Interval callback" : "KafkaJS eachMessage"),
            registration: range(node), callback: range(handler),
            ...(kind === "INTERVAL" ? {interval: node.arguments[1].text} :
                {...subscriptionSetup(node, runner), manualOffsets: Boolean(autoCommit && ts.isPropertyAssignment(autoCommit)
                    && autoCommit.initializer.kind === ts.SyntaxKind.FalseKeyword)}),
            ...context(handler, runner)};
    });
};
