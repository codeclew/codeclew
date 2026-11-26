/* Specialize an admitted callback only through the retained topic binding and validator. */
module.exports = function ({ts, unwrap, source, caption, topic, resolve}) {
    const UNKNOWN = Symbol("unknown");
    const literal = value => ({kind: "literal", value});
    const incomingTopic = {...literal(topic), message: true};
    const record = (facts = new Map(), origin = Symbol("decoded value")) => ({kind: "record", facts, origin});
    const envelope = {kind: "envelope"};
    const escapedEnvelope = {kind: "envelope"};
    const unknownTopic = {kind: "unknown"};
    const isRecord = value => value?.kind === "record";
    const symbol = (scope, node) => scope.checker.getSymbolAtLocation(node);
    const operators = ts.SyntaxKind;
    const strict = new Set([operators.EqualsEqualsEqualsToken, operators.ExclamationEqualsEqualsToken]);
    const comparisons = new Set([...strict, operators.LessThanToken, operators.LessThanEqualsToken,
        operators.GreaterThanToken, operators.GreaterThanEqualsToken]);
    const pureBuiltins = new Map([["Array", new Set(["isArray"])],
        ["Number", new Set(["isFinite", "isInteger", "isNaN", "isSafeInteger"])]]);
    const unsupported = new Map();

    function constant(node, scope) {
        if (ts.isStringLiteralLike(node)) return literal(node.text);
        if (ts.isNumericLiteral(node)) return literal(Number(node.text.replaceAll("_", "")));
        if (node.kind === operators.TrueKeyword || node.kind === operators.FalseKeyword) return literal(node.kind === operators.TrueKeyword);
        if (node.kind === operators.NullKeyword) return literal(null);
        if (node.kind === operators.RegularExpressionLiteral) return {kind: "regexp", text: node.text};
        if (ts.isIdentifier(node) && node.text === "undefined" && !symbol(scope, node)?.declarations?.length) {
            return literal(undefined);
        }
    }

    function operand(scope, expression, values, validation) {
        const node = unwrap(expression);
        if (!node) return undefined;
        const value = constant(node, scope);
        if (value) return value;
        if (ts.isIdentifier(node)) {
            const bound = values.get(symbol(scope, node));
            return validation && bound?.kind === "path" ? {...bound, facts: validation} : bound;
        }
        if (ts.isPropertyAccessExpression(node) && !ts.isOptionalChain(node)) {
            const base = operand(scope, node.expression, values, validation);
            if (base === envelope && node.name.text === "topic") return incomingTopic;
            if (base === escapedEnvelope && node.name.text === "topic") return unknownTopic;
            if (isRecord(base) || base?.kind === "path") {
                const path = [...(base.path || []), node.name.text];
                if (path.length <= 6) return {kind: "path", path, facts: validation || base.facts, origin: base.origin};
            }
        }
        return undefined;
    }

    function abstract(scope, expression, values) {
        const node = unwrap(expression);
        if (!node) return undefined;
        const inherited = operand(scope, node, values);
        if (inherited) return inherited;
        if (ts.isCallExpression(node) && !ts.isOptionalChain(node) && node.arguments.length === 1) {
            const method = unwrap(node.expression);
            if (ts.isPropertyAccessExpression(method) && method.name.text === "parse" &&
                    ts.isIdentifier(method.expression) && method.expression.text === "JSON" && !symbol(scope, method.expression)) {
                // Conditional on the ordinary JSON parser, with no reviver.
                return record();
            }
        }
        return undefined;
    }

    const property = member => {
        const name = member.propertyName || member.name;
        return ts.isIdentifier(name) || ts.isStringLiteralLike(name) ? name.text : undefined;
    };

    function bind(scope, values, name, value) {
        if (!value) return;
        if (ts.isIdentifier(name)) {
            const binding = symbol(scope, name);
            if (binding) values.set(binding, value);
        } else if (value.kind === "envelope" && ts.isObjectBindingPattern(name)) {
            for (const member of name.elements) {
                if (!member.dotDotDotToken && !member.initializer && property(member) === "topic" && ts.isIdentifier(member.name)) {
                    const binding = symbol(scope, member.name);
                    if (binding) values.set(binding, value === envelope ? incomingTopic : unknownTopic);
                }
            }
        }
    }

    function seeds(scope, args) {
        const values = new Map();
        scope.node.parameters.forEach((parameter, index) => {
            if (!parameter.dotDotDotToken && !parameter.initializer) bind(scope, values, parameter.name, args[index]);
        });
        return values;
    }

    function exact(value) {
        if (value?.kind === "literal") return value.value;
        const fact = fieldFact(value);
        return fact && Object.hasOwn(fact, "equal") ? fact.equal : UNKNOWN;
    }

    function fieldFact(value) {
        return value?.kind === "path" ? value.facts.get(JSON.stringify(value.path)) : undefined;
    }

    function bool(value) {
        const actual = exact(value);
        if (actual !== UNKNOWN) return Boolean(actual);
        return fieldFact(value)?.truth ?? UNKNOWN;
    }

    // Only these plain operations support a discriminator proof. Calls,
    // assignments, getters of a non-retained receiver and optional operators
    // do not supply additional field facts.
    function test(scope, expression, values, facts, depth = 0) {
        const node = unwrap(expression);
        if (!node || depth > 14) return UNKNOWN;
        if (ts.isPrefixUnaryExpression(node) && node.operator === operators.ExclamationToken) {
            const value = test(scope, node.operand, values, facts, depth + 1);
            return value === UNKNOWN ? UNKNOWN : !value;
        }
        if (ts.isBinaryExpression(node)) {
            const op = node.operatorToken.kind;
            if (op === operators.AmpersandAmpersandToken || op === operators.BarBarToken) {
                const a = test(scope, node.left, values, facts, depth + 1);
                if (a === (op === operators.BarBarToken)) return a;
                const b = test(scope, node.right, values, facts, depth + 1);
                if (a !== UNKNOWN || b === (op === operators.BarBarToken)) return b;
                return UNKNOWN;
            }
            if (strict.has(op)) {
                const a = exact(operand(scope, node.left, values, facts));
                const b = exact(operand(scope, node.right, values, facts));
                return a === UNKNOWN || b === UNKNOWN ? UNKNOWN : (a === b) === (op === operators.EqualsEqualsEqualsToken);
            }
        }
        return bool(operand(scope, node, values, facts));
    }

    function provenTypes(scope, expression, outcome, values, known) {
        const node = unwrap(expression);
        if (ts.isPrefixUnaryExpression(node) && node.operator === operators.ExclamationToken) {
            return provenTypes(scope, node.operand, !outcome, values, known);
        }
        if (!ts.isBinaryExpression(node)) return known;
        const operation = node.operatorToken.kind;
        if (operation === operators.AmpersandAmpersandToken && outcome || operation === operators.BarBarToken && !outcome) {
            const left = provenTypes(scope, node.left, outcome, values, known);
            return provenTypes(scope, node.right, outcome, values, left);
        }
        if (!strict.has(operation) || outcome !== (operation === operators.EqualsEqualsEqualsToken)) return known;
        const left = unwrap(node.left), right = unwrap(node.right);
        const [typeOf, label] = ts.isTypeOfExpression(left) ? [left, right] : [right, left];
        if (!ts.isTypeOfExpression(typeOf) || !ts.isStringLiteralLike(label)) return known;
        const input = operand(scope, typeOf.expression, values);
        return input?.kind === "path" ? new Map([...known, [JSON.stringify(input.path), label.text]]) : known;
    }

    function predicateBody(target, args, known, proof, stack, depth) {
        if (!target || stack.has(target.id) || stack.size === 4 || target.node.asteriskToken ||
                target.node.parameters.length !== args.length ||
                target.node.parameters.some(parameter => parameter.dotDotDotToken || parameter.initializer) ||
                target.node.modifiers?.some(modifier => modifier.kind === operators.AsyncKeyword)) return false;
        const values = seeds(target, args);
        const next = new Set([...stack, target.id]);
        function returned(node, types) {
            const statement = ts.isBlock(node) && node.statements.length === 1 ? node.statements[0] : node;
            return ts.isReturnStatement(statement) && Boolean(statement.expression) &&
                safeCondition(target, statement.expression, values, proof, types, next, depth + 1);
        }
        const body = target.node.body;
        if (!ts.isBlock(body)) return safeCondition(target, body, values, proof, known, next, depth + 1);
        const statements = [...body.statements];
        while (statements.length && ts.isVariableStatement(statements[0])) {
            const declarationList = statements.shift().declarationList;
            if (!(declarationList.flags & ts.NodeFlags.Const)) return false;
            for (const declaration of declarationList.declarations) {
                const value = operand(target, declaration.initializer, values);
                if (!value || !ts.isIdentifier(declaration.name)) return false;
                bind(target, values, declaration.name, value);
            }
        }
        if (statements.length === 1) return returned(statements[0], known);
        if (statements.length !== 2 || !ts.isIfStatement(statements[0]) || statements[0].elseStatement) return false;
        const [guard, result] = statements;
        return safeCondition(target, guard.expression, values, proof, known, next, depth + 1) &&
            returned(guard.thenStatement, provenTypes(target, guard.expression, true, values, known)) &&
            returned(result, provenTypes(target, guard.expression, false, values, known));
    }

    function safeCondition(scope, expression, values, proof = new Set(), known = new Map(), stack = new Set(), depth = 0) {
        const node = unwrap(expression);
        if (!node || depth > 14) return false;
        if (operand(scope, node, values)) return true;
        if (ts.isTypeOfExpression(node) || ts.isPrefixUnaryExpression(node) && node.operator === operators.ExclamationToken) {
            return safeCondition(scope, node.expression || node.operand, values, proof, known, stack, depth + 1);
        }
        if (ts.isCallExpression(node) && !ts.isOptionalChain(node) && node.arguments.length === 1) {
            const method = unwrap(node.expression);
            const receiver = ts.isPropertyAccessExpression(method) && unwrap(method.expression);
            const builtin = receiver && ts.isIdentifier(receiver) && !symbol(scope, receiver)?.declarations?.length
                ? receiver.text : undefined;
            const methodName = receiver && method.name.text;
            const argument = node.arguments[0];
            if (builtin === "Date" && methodName === "parse") {
                const value = operand(scope, argument, values);
                return value?.kind === "literal" && typeof value.value === "string" ||
                    value?.kind === "path" && known.get(JSON.stringify(value.path)) === "string";
            }
            const literalArray = receiver && ts.isArrayLiteralExpression(receiver) && methodName === "includes" &&
                receiver.elements.length <= 16 && receiver.elements.every(element => constant(element, scope)?.kind === "literal");
            const regexp = receiver && methodName === "test" && operand(scope, receiver, values);
            const pureRegexp = regexp?.kind === "regexp" && !/[gy]/.test(regexp.text.slice(regexp.text.lastIndexOf("/") + 1));
            if (literalArray || pureRegexp || pureBuiltins.get(builtin)?.has(methodName)) {
                return safeCondition(scope, argument, values, proof, known, stack, depth + 1);
            }
            const target = resolve(scope, node);
            const actual = operand(scope, argument, values);
            if (!actual || !predicateBody(target, [actual], known, proof, stack, depth + 1)) return false;
            proof.add(source(scope, node));
            proof.add(target.id);
            return true;
        }
        if (!ts.isBinaryExpression(node)) return false;
        const op = node.operatorToken.kind;
        const shortCircuit = op === operators.AmpersandAmpersandToken || op === operators.BarBarToken;
        return (shortCircuit || comparisons.has(op)) &&
            safeCondition(scope, node.left, values, proof, known, stack, depth + 1) &&
            safeCondition(scope, node.right, values, proof,
                shortCircuit ? provenTypes(scope, node.left, op === operators.AmpersandAmpersandToken, values, known) : known,
                stack, depth + 1);
    }

    function refine(scope, expression, truth, values, facts, depth = 0) {
        const node = unwrap(expression);
        if (depth > 14) return facts;
        const known = test(scope, node, values, facts);
        if (known !== UNKNOWN && known !== truth) return undefined;
        if (ts.isPrefixUnaryExpression(node) && node.operator === operators.ExclamationToken) {
            return refine(scope, node.operand, !truth, values, facts, depth + 1);
        }
        if (ts.isBinaryExpression(node)) {
            const op = node.operatorToken.kind;
            if (op === operators.AmpersandAmpersandToken && truth || op === operators.BarBarToken && !truth) {
                const left = refine(scope, node.left, truth, values, facts, depth + 1);
                return left && refine(scope, node.right, truth, values, left, depth + 1);
            }
            if (strict.has(op) && truth === (op === operators.EqualsEqualsEqualsToken)) {
                const a = operand(scope, node.left, values, facts), b = operand(scope, node.right, values, facts);
                const [member, value] = a?.kind === "path" ? [a, b] : [b, a];
                if (member?.kind === "path" && value?.kind === "literal") {
                    return new Map([...facts, [JSON.stringify(member.path), {...fieldFact(member), equal: value.value}]]);
                }
                const types = provenTypes(scope, node, truth, values, new Map());
                if (types.size) return new Map([...facts, ...[...types].map(([path, type]) => [path, {...facts.get(path), type}])]);
            }
            return facts;
        }
        const member = operand(scope, node, values, facts);
        return member?.kind === "path" ? new Map([...facts, [JSON.stringify(member.path), {...fieldFact(member), truth}]]) : facts;
    }

    function validate(scope, args) {
        if (!scope || !args.some(isRecord) || args.filter(isRecord).length !== 1 ||
                !args.includes(incomingTopic) || !ts.isBlock(scope.node.body) ||
                scope.node.parameters.length !== args.length ||
                scope.node.parameters.some(parameter => parameter.dotDotDotToken || parameter.initializer) ||
                scope.node.modifiers?.some(modifier => modifier.kind === operators.AsyncKeyword)) return undefined;
        const initial = seeds(scope, args);
        const successful = [];
        let fuel = 80, open = false;
        function guarded(expression, values, proof) {
            const supported = safeCondition(scope, expression, values, proof);
            if (!supported) unsupported.set("VALIDATOR_GUARD_OPEN", source(scope, expression));
            return supported;
        }
        function constants(statement, input) {
            if (!(statement.declarationList.flags & ts.NodeFlags.Const)) return undefined;
            const values = new Map(input);
            for (const declaration of statement.declarationList.declarations) {
                if (!ts.isIdentifier(declaration.name) || !declaration.initializer) return undefined;
                const value = operand(scope, declaration.initializer, values);
                if (!value || value.kind === "envelope") return undefined;
                bind(scope, values, declaration.name, value);
            }
            return values;
        }
        function checkLoop(statement, values, proof, depth = 0) {
            if (--fuel < 0 || depth > 8) return false;
            if (ts.isBlock(statement)) {
                for (const child of statement.statements) {
                    if (ts.isVariableStatement(child)) {
                        values = constants(child, values);
                        if (!values) return false;
                    } else if (!checkLoop(child, values, proof, depth + 1)) {
                        return false;
                    }
                }
                return true;
            }
            if (ts.isThrowStatement(statement) || ts.isEmptyStatement(statement)) return true;
            if (ts.isIfStatement(statement)) {
                return guarded(statement.expression, values, proof) &&
                    checkLoop(statement.thenStatement, values, proof, depth + 1) &&
                    (!statement.elseStatement || checkLoop(statement.elseStatement, values, proof, depth + 1));
            }
            if (!ts.isForOfStatement(statement) || statement.awaitModifier ||
                    !ts.isVariableDeclarationList(statement.initializer) ||
                    !(statement.initializer.flags & ts.NodeFlags.Const) || statement.initializer.declarations.length !== 1) return false;
            const member = operand(scope, statement.expression, values);
            const declaration = statement.initializer.declarations[0];
            if (member?.kind !== "path" || !ts.isIdentifier(declaration.name) || declaration.initializer) return false;
            const entered = new Map(values);
            bind(scope, entered, declaration.name, record());
            return checkLoop(statement.statement, entered, proof, depth + 1);
        }
        function walk(statement, state) {
            if (--fuel < 0) {
                open = true;
                return [];
            }
            if (ts.isBlock(statement)) {
                let paths = [state];
                for (const child of statement.statements) paths = paths.flatMap(path => walk(child, path));
                return paths.map(path => ({...path, values: state.values}));
            }
            if (ts.isVariableStatement(statement)) {
                const values = constants(statement, state.values);
                if (values) return [{...state, values}];
                open = true;
                return [];
            }
            const proof = new Set();
            if (ts.isIfStatement(statement) && guarded(statement.expression, state.values, proof)) {
                return [true, false].flatMap(truth => {
                    const facts = refine(scope, statement.expression, truth, state.values, state.facts);
                    if (!facts) return [];
                    const path = {...state, facts, sites: [...state.sites, source(scope, statement.expression), ...proof]};
                    const branch = truth ? statement.thenStatement : statement.elseStatement;
                    return branch ? walk(branch, path) : [path];
                });
            }
            // A read-only validation loop can reject a member. Any normal
            // exit preserves the outer discriminator and field constraints.
            if (ts.isForOfStatement(statement) && checkLoop(statement, state.values, proof)) {
                return [{...state, sites: [...state.sites, source(scope, statement), ...proof]}];
            }
            if (ts.isThrowStatement(statement)) return [];
            if (ts.isReturnStatement(statement) && isRecord(operand(scope, statement.expression, state.values))) {
                successful.push({...state, sites: [...state.sites, source(scope, statement)]});
                return [];
            }
            open = true;
            return [];
        }
        const input = args.find(isRecord);
        if (walk(scope.node.body, {values: initial, facts: input.facts, sites: []}).length || open || !successful.length) return undefined;
        const equal = (left, right) => left && right && left.truth === right.truth &&
            left.type === right.type &&
            Object.hasOwn(left, "equal") === Object.hasOwn(right, "equal") && left.equal === right.equal;
        const common = new Map([...successful[0].facts].filter(([field, fact]) =>
            successful.every(path => equal(fact, path.facts.get(field)))));
        return common.size ? {...record(common, input.origin), sites: [...new Set(successful.flatMap(path => path.sites))]} : undefined;
    }

    function observe(scope, expression, state, outcome) {
        function usesMessage(node) {
            const selected = operand(scope, node, state.messageValues);
            return selected === incomingTopic || selected === unknownTopic || selected?.kind === "envelope" ||
                isRecord(selected) || selected?.kind === "path" ||
                Boolean(ts.forEachChild(node, child => usesMessage(child) ? true : undefined));
        }
        if (!usesMessage(expression)) return state;
        const known = safeCondition(scope, expression, state.messageValues)
            ? test(scope, expression, state.messageValues) : UNKNOWN;
        if (known === UNKNOWN) return {...state, messageOpen: true};
        const site = source(scope, expression);
        const evidence = [...new Set([...state.messageEvidence, site])];
        const validation = state.messageValidation && test(scope, expression, dropRecords(state.messageValues)) !== known;
        const predicate = unwrap(expression);
        const values = validation && ts.isBinaryExpression(predicate) && strict.has(predicate.operatorToken.kind)
            ? [predicate.left, predicate.right].flatMap(node => {
                const value = operand(scope, node, state.messageValues);
                const fact = fieldFact(value);
                const name = caption(node);
                return fact && Object.hasOwn(fact, "equal") && name
                    ? [{name, value: JSON.stringify(fact.equal) ?? "undefined"}] : [];
            }) : [];
        const check = {guard: caption(expression), actual: known, required: outcome, validation, values};
        return {...state, messageEvidence: evidence,
            messageAgreement: known === outcome && (validation || !state.messageAgreement) ? check : state.messageAgreement,
            messageContradiction: state.messageContradiction || (known !== outcome ? check : undefined)};
    }

    function dropRecords(values) {
        return new Map([...values].filter(([, value]) => !isRecord(value) && value?.kind !== "path"));
    }

    function escapedValues(scope, expressions, values) {
        const escaped = new Map();
        let envelopeEscaped = false;
        function visit(expression) {
            const node = unwrap(expression);
            if (!node) return;
            const value = operand(scope, node, values);
            if (value === envelope) {
                envelopeEscaped = true;
                return;
            }
            if (value === incomingTopic || (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) &&
                    operand(scope, node.expression, values) === envelope) return;
            if (isRecord(value) || value?.kind === "path") {
                const observed = exact(value);
                const immutable = observed !== UNKNOWN && (observed === null || typeof observed !== "object") ||
                    ["string", "number", "boolean", "undefined"].includes(fieldFact(value)?.type);
                if (!immutable) {
                    if (!escaped.has(value.origin)) escaped.set(value.origin, []);
                    escaped.get(value.origin).push(value.path || []);
                }
                return;
            }
            ts.forEachChild(node, visit);
        }
        for (const expression of expressions) visit(expression);
        if (!escaped.size && !envelopeEscaped) return values;
        return new Map([...values].map(([binding, value]) => {
            if (value === envelope && envelopeEscaped) return [binding, escapedEnvelope];
            const paths = escaped.get(value.origin);
            if (!paths) return [binding, value];
            // An ordinary JSON decode is a tree. An escaped child can change
            // its own descendants, without changing its parent's sibling fields.
            const facts = new Map([...value.facts].filter(([key]) => {
                const field = JSON.parse(key);
                return !paths.some(path => path.length < field.length && path.every((name, index) => name === field[index]));
            }));
            return [binding, {...value, facts}];
        }));
    }

    return {root: [envelope], seeds, bind, abstract, validate, observe, dropRecords, escapedValues, unsupported};
};
