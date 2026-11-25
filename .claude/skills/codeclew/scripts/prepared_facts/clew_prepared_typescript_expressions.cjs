/* Label retained expressions while preserving their operand and call structure. */
module.exports = function expressions(ts, unwrap, short) {
    const bound = value => short(value, 270);
    function literal(node) {
        if (ts.isStringLiteralLike(node)) return short(JSON.stringify(node.text), 88);
        if (ts.isNumericLiteral(node) || ts.isBigIntLiteral(node)) return short(node.getText(), 36);
        if (ts.isRegularExpressionLiteral(node)) return short(node.getText(), 230);
        if ([ts.SyntaxKind.ThisKeyword, ts.SyntaxKind.SuperKeyword, ts.SyntaxKind.TrueKeyword,
            ts.SyntaxKind.FalseKeyword, ts.SyntaxKind.NullKeyword].includes(node.kind)) return node.getText();
    }
    function caption(expression, depth = 0) {
        if (!expression || depth > 14) return undefined;
        const inner = value => caption(value, depth + 1);
        if (ts.isParenthesizedExpression(expression)) {
            const value = inner(expression.expression);
            return value && bound("(" + value + ")");
        }
        const node = unwrap(expression);
        const value = literal(node);
        if (value) return value;
        if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) return short(node.text, 110);
        if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
            const receiver = inner(node.expression);
            if (!receiver) return undefined;
            const optional = node.questionDotToken ? "?." : "";
            if (ts.isPropertyAccessExpression(node)) return bound(receiver + (optional || ".") + node.name.text);
            const index = inner(node.argumentExpression);
            return index && bound(receiver + optional + "[" + index + "]");
        }
        if (ts.isCallExpression(node) || ts.isNewExpression(node)) {
            const name = inner(node.expression);
            if (!name) return undefined;
            const head = (ts.isNewExpression(node) ? "new " : "") + name + (node.questionDotToken ? "?." : "");
            const items = (node.arguments || []).map(inner);
            const formatted = items.every(Boolean) && bound(head + "(" + items.join(", ") + ")");
            return formatted || bound(head + "(…)");
        }
        if (ts.isObjectLiteralExpression(node)) {
            const fields = node.properties.slice(0, 8).map(property => {
                if (ts.isShorthandPropertyAssignment(property)) return property.name.text;
                if (!ts.isPropertyAssignment(property)) return undefined;
                const key = ts.isIdentifier(property.name) ? property.name.text : inner(property.name);
                const field = inner(property.initializer);
                return key && (field ? key + " set to " + field : key);
            });
            const names = fields.length && fields.every(Boolean) && bound("an object with " + fields.join(", "));
            return names || "an object";
        }
        if (ts.isArrayLiteralExpression(node)) {
            const members = node.elements.length <= 8 && node.elements.map(inner);
            return members?.length && members.every(Boolean) && bound("[" + members.join(", ") + "]") || "an array";
        }
        if (ts.isAwaitExpression(node) || ts.isTypeOfExpression(node) || ts.isVoidExpression(node)) {
            const operand = inner(node.expression);
            const keyword = ts.isAwaitExpression(node) ? "the awaited value of" : ts.isTypeOfExpression(node) ? "typeof" : "void";
            return operand && bound(keyword + " " + operand);
        }
        if (ts.isPrefixUnaryExpression(node) || ts.isPostfixUnaryExpression(node)) {
            const operand = inner(node.operand);
            const operator = ts.tokenToString(node.operator);
            if (!operand || !operator) return undefined;
            return bound(ts.isPrefixUnaryExpression(node) ? operator + operand : operand + operator);
        }
        if (ts.isBinaryExpression(node)) {
            const left = inner(node.left), right = inner(node.right);
            const operator = ts.tokenToString(node.operatorToken.kind);
            return left && right && operator && bound(left + " " + operator + " " + right);
        }
        if (ts.isConditionalExpression(node)) {
            const condition = inner(node.condition), yes = inner(node.whenTrue), no = inner(node.whenFalse);
            return condition && yes && no && bound(condition + " ? " + yes + " : " + no);
        }
    }
    function callName(node) {
        const callee = unwrap(node.expression);
        return caption(callee) || "the callable expression";
    }
    return {caption, callName, literal};
};
