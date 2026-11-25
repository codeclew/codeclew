/* Keep source Boolean structure and the test passed to a higher-order check. */
module.exports = function predicates(ts, unwrap, caption, short) {
    function normalized(expression) {
        const value = caption(expression);
        if (value && !value.includes("…")) return value;
        const text = expression.getText();
        if (text.length > 8_000 || ts.isFunctionLike(expression)) return value;
        const file = expression.getSourceFile();
        const tokens = [];
        let truncated = false, includesStatement = false;
        function collect(node) {
            if (node.kind === ts.SyntaxKind.SemicolonToken) includesStatement = true;
            if (tokens.length >= 140) {
                truncated = true;
                return;
            }
            const children = node.getChildren(file);
            if (children.length) {
                for (const child of children) collect(child);
            } else {
                const start = node.getStart(file);
                if (start < node.end) tokens.push(file.text.slice(start, node.end));
            }
        }
        collect(expression);
        if (includesStatement) return value;
        const adjoining = new Set([".", "?.", "[", "("]);
        const suffix = new Set([".", "?.", "[", "]", "(", ")", ","]);
        function join(parts) {
            return parts.map((token, index) => (index && !adjoining.has(parts[index - 1]) &&
                !suffix.has(token) ? " " : "") + token).join("");
        }
        const compact = join(tokens);
        if (!truncated && short(compact, 230)) return compact;
        const first = [], last = [];
        let front = 0, back = tokens.length - 1;
        while (front <= back && short(join([...first, tokens[front]]), 155)) first.push(tokens[front++]);
        while (back >= front && short(join([tokens[back], ...last]), 65)) last.unshift(tokens[back--]);
        return [join(first), "…", join(last), truncated && "…"].filter(Boolean).join(" ");
    }
    function permit(node) {
        if (!node) return false;
        if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.ExclamationToken) return permit(unwrap(node.operand));
        if (ts.isBinaryExpression(node) && [ts.SyntaxKind.AmpersandAmpersandToken, ts.SyntaxKind.BarBarToken]
                .includes(node.operatorToken.kind)) return true;
        return Boolean(quantifier(node));
    }
    function quantifier(node) {
        if (!ts.isCallExpression(node) || ts.isOptionalChain(node) || !ts.isPropertyAccessExpression(node.expression) ||
                !["every", "some"].includes(node.expression.name.text) || node.arguments.length > 2) return undefined;
        const check = unwrap(node.arguments[0]);
        if (check && ts.isIdentifier(check)) {
            return {body: check, receiver: node.expression.expression, method: node.expression.name.text, parameters: [], reference: true};
        }
        if (!check || !(ts.isArrowFunction(check) || ts.isFunctionExpression(check) && !check.asteriskToken) ||
                check.modifiers?.some(item => item.kind === ts.SyntaxKind.AsyncKeyword) ||
                !check.parameters.length || check.parameters.some(parameter =>
                    !ts.isIdentifier(parameter.name) || parameter.dotDotDotToken || parameter.initializer)) return undefined;
        const block = check.body;
        const statement = ts.isBlock(block) && block.statements.length === 1 && block.statements[0];
        const body = ts.isBlock(block) ? statement && ts.isReturnStatement(statement) && statement.expression : block;
        if (!body) return undefined;
        return {body, receiver: node.expression.expression, method: node.expression.name.text,
            parameters: check.parameters.map(parameter => parameter.name.text)};
    }
    function tree(expression, sourceFor) {
        let count = 0;
        function walk(inner, depth) {
            const node = unwrap(inner);
            const source = sourceFor(node);
            if (++count > 52 || depth >= 12) return {kind: "test", expression: normalized(node) || "additional cited expression", source};
            if (ts.isBinaryExpression(node)) {
                const operator = node.operatorToken.kind;
                if (operator === ts.SyntaxKind.AmpersandAmpersandToken || operator === ts.SyntaxKind.BarBarToken) {
                    const kind = operator === ts.SyntaxKind.AmpersandAmpersandToken ? "all" : "any";
                    const pending = [node.right, node.left], children = [];
                    let examined = 0;
                    while (pending.length && examined++ < 90) {
                        const term = unwrap(pending.pop());
                        if (ts.isBinaryExpression(term) && term.operatorToken.kind === operator) {
                            pending.push(term.right, term.left);
                        } else {
                            children.push(walk(term, depth + 1));
                        }
                    }
                    if (pending.length) children.push({kind: "test", expression: "additional cited clauses", source, partial: true});
                    return {kind, children, source};
                }
            }
            if (ts.isPrefixUnaryExpression(node) && node.operator === ts.SyntaxKind.ExclamationToken) {
                return {kind: "not", children: [walk(node.operand, depth + 1)], source};
            }
            const higher = quantifier(node);
            if (higher) {
                const {method, receiver, parameters, body, reference} = higher;
                return {kind: "callback", method, receiver: normalized(receiver), parameters,
                    ...(reference ? {callback: body.text} : {}), children: [walk(body, depth + 1)], source};
            }
            return {kind: "test", expression: normalized(node) || "cited source expression", source};
        }
        return walk(expression, 0);
    }
    function describe(node) {
        if (node.kind === "test") return node.expression;
        if (node.kind === "not") return "not (" + describe(node.children[0]) + ")";
        if (node.kind === "callback") {
            const callback = describe(node.children[0]);
            const name = (node.receiver || "the cited receiver") + "." + node.method;
            return node.callback ? name + " uses " + node.callback + " as its test" :
                name + " tests " + node.parameters.join(", ") + " with (" + callback + ")";
        }
        const operator = node.kind === "all" ? " and " : " or ";
        return "(" + node.children.map(describe).join(operator) + ")";
    }
    function summarize(node) {
        const full = describe(node);
        if (full.length <= 525) return full;
        if (node.kind === "test") return node.expression;
        if (node.kind === "callback") return (node.receiver || "the cited receiver") + "." + node.method + " with the cited callback test";
        if (node.kind === "not") return "not (" + summarize(node.children[0]) + ")";
        const examples = [];
        for (const child of node.children) {
            const detail = describe(child);
            if (short([...examples, detail].join(", "), 230)) examples.push(detail);
        }
        return (node.kind === "all" ? "all of " : "any of ") + node.children.length +
            " cited clauses" + (examples.length ? ", including " + examples.join(", ") : "");
    }
    function sources(node) {
        return [node.source, ...(node.children || []).flatMap(sources)];
    }
    const hasCallback = node => node.kind === "callback" || (node.children || []).some(hasCallback);
    return {permit, tree, summarize, sources, hasCallback};
};
