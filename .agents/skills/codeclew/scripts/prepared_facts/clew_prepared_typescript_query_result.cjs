/* Project exact JavaScript value paths from static SELECT results to local outputs. */
module.exports = function queryResults(ts, unwrap) {
    const name = node => (ts.isIdentifier(node) || ts.isStringLiteral(node) || ts.isNumericLiteral(node)) &&
        node.text.length <= 88 ? node.text : undefined;
    const pathText = path => path.map(part => /^[A-Za-z_$][\w$]*$/.test(part) ? "." + part :
        "[" + (/^(0|[1-9]\d*)$/.test(part) ? part : JSON.stringify(part)) + "]").join("");
    const extend = (read, part, site) => ({
        ...read, path: [...read.path, part], sites: [...read.sites, site],
    });

    function tracker(scope, symbol, source) {
        const site = node => source(scope, node);
        function read(expression, state, evaluated, depth = 0) {
            const node = unwrap(expression);
            if (!node || depth > 8) return undefined;
            if (evaluated.has(node)) return evaluated.get(node);
            if (ts.isIdentifier(node)) {
                const binding = ts.isShorthandPropertyAssignment(node.parent)
                    ? scope.checker.getShorthandAssignmentValueSymbol(node.parent) : symbol(node);
                return state.queryValues.get(binding);
            }
            if (ts.isOptionalChain(node)) return undefined;
            let part;
            if (ts.isPropertyAccessExpression(node) && !ts.isPrivateIdentifier(node.name)) part = name(node.name);
            if (ts.isElementAccessExpression(node)) {
                const index = unwrap(node.argumentExpression);
                if (index && (ts.isStringLiteral(index) || ts.isNumericLiteral(index))) part = name(index);
            }
            const base = part !== undefined && read(node.expression, state, evaluated, depth + 1);
            return base && base.path.length < 8 ? extend(base, part, site(node)) : undefined;
        }
        function bind(state, binding, value, declaration) {
            const values = new Map(state.queryValues);
            if (value) values.set(binding, {...value, sites: [...value.sites, site(declaration)]});
            else values.delete(binding);
            return {...state, queryValues: values};
        }
        function escape(state, expressions, evaluated) {
            if (!state.queryValues.size) return state;
            const escaped = new Set();
            function visit(node) {
                if (ts.isFunctionLike(node) || ts.isClassLike(node) || ts.isTypeNode(node)) return;
                const value = read(node, state, evaluated);
                if (value) escaped.add(value.origin);
                ts.forEachChild(node, visit);
            }
            for (const expression of expressions) if (expression) visit(expression);
            if (!escaped.size) return state;
            return {...state, queryValues: new Map([...state.queryValues].filter(([, value]) => !escaped.has(value.origin)))};
        }
        function outputs(expression, state, evaluated, output = []) {
            const node = unwrap(expression);
            if (!node || output.length >= 5) return [];
            const value = read(node, state, evaluated);
            if (value) return [{...value, sites: [...value.sites, site(node)], output}];
            if (ts.isObjectLiteralExpression(node) && node.properties.length <= 12) {
                const properties = new Map();
                for (const property of node.properties) {
                    if (!(ts.isPropertyAssignment(property) || ts.isShorthandPropertyAssignment(property))) return [];
                    const key = name(property.name);
                    if (key === undefined || key === "__proto__" || properties.has(key)) return [];
                    properties.set(key, ts.isShorthandPropertyAssignment(property) ? property.name : property.initializer);
                }
                return [...properties].flatMap(([key, field]) => outputs(field, state, evaluated, [...output, key]));
            }
            if (ts.isArrayLiteralExpression(node) && node.elements.length <= 12 && !node.elements.some(ts.isSpreadElement)) {
                return node.elements.flatMap((item, index) => outputs(item, state, evaluated, [...output, String(index)]));
            }
            return [];
        }
        return {read, bind, escape, outputs};
    }

    function resolve(value, outcomes, chain = []) {
        const {origin, path, output, sites} = value;
        if (origin.query) return [{query: origin.query, call: origin.call, sites: [...origin.sites, ...sites], path, output, guards: []}];
        const helper = origin.helper;
        if (chain.length >= 4 || chain.includes(helper.target)) return [];
        const resolved = [];
        for (const exit of outcomes.get(helper.target) || []) {
            for (const returned of exit.reads || []) {
                const field = returned.output;
                const common = Math.min(field.length, path.length);
                if (field.slice(0, common).some((part, index) => part !== path[index])) continue;
                const selected = {...returned, path: [...returned.path, ...path.slice(common)],
                    output: [...output, ...field.slice(common)]};
                for (const reached of resolve(selected, outcomes, [...chain, helper.target])) {
                    const guards = exit.guards.map(guard => ({...guard, text: "in " + helper.name + ", " + guard.text}));
                    resolved.push({...reached, sites: [...origin.sites, exit.site, ...sites, ...reached.sites],
                        guards: [...guards, ...reached.guards]});
                    if (resolved.length === 8) return resolved;
                }
            }
        }
        return resolved;
    }
    return {tracker, bindField: extend, name, pathText, resolve};
};
