/* Describe a bounded literal SQL request without assuming an effective database. */
const token = /\s+|--[^\r\n]*|\/\*[\s\S]*?\*\/|'(?:[^']|'')*'|"(?:[^"]|"")*"|\$[1-9][0-9]*|[A-Za-z_][A-Za-z_0-9$]*|\d+(?:\.\d+)?|\:\:|<>|!=|>=|<=|[-+*/%=.,();<>?:]/y;

function tokenize(text) {
    if (text.length > 2600 || text.includes("\\") || /\/\*[\s\S]*?\/\*/.test(text)) return undefined;
    const items = [];
    let position = 0, depth = 0;
    while (position < text.length && items.length < 300) {
        token.lastIndex = position;
        const match = token.exec(text);
        if (!match) return undefined;
        const value = match[0];
        position = token.lastIndex;
        if (/^\s|^--|^\/\*/.test(value)) continue;
        if (value === ")" && --depth < 0) return undefined;
        items.push({value, key: value.toUpperCase(), depth, start: match.index, end: position});
        if (value === "(") depth++;
    }
    if (depth || position !== text.length) return undefined;
    if (items.at(-1)?.key === ";") items.pop();
    return items.some(item => item.key === ";") ? undefined : items;
}
const identifier = item => item && /^[A-Za-z_"]/.test(item.value);
const objectText = (text, tokens, start, end) => {
    if (start >= end) return undefined;
    const phrase = text.slice(tokens[start].start, tokens[end - 1].end).replace(
        /'(?:[^']|'')*'|"(?:[^"]|"")*"|\s+/g, part => /^\s/.test(part) ? " " : part);
    return phrase.length <= 160 ? phrase : undefined;
};

function split(tokens, first, last, depth) {
    const segments = [];
    let start = first;
    for (let index = first; index < last; index++) {
        if (tokens[index].key === "," && tokens[index].depth === depth) {
            if (start === index) return undefined;
            segments.push([start, index]);
            start = index + 1;
        }
    }
    if (start >= last) return undefined;
    segments.push([start, last]);
    return segments;
}

function columnsAt(tokens, first, last) {
    const columns = split(tokens, first, last, 1);
    if (!columns?.every(([start, end]) => end === start + 1 && identifier(tokens[start]))) return undefined;
    const seen = new Set(columns.map(([start]) => {
        const name = tokens[start].value;
        return name.startsWith('"') ? name.slice(1, -1).replaceAll('""', '"') : name.toLowerCase();
    }));
    return seen.size === columns.length ? columns.map(([start]) => tokens[start].value) : undefined;
}

function parameterPositions(tokens) {
    return [...new Set(tokens.filter(item => /^\$[1-9]\d*$/.test(item.value)).map(item => +item.value.slice(1)))];
}

function describe(text, additionalCommands) {
    const tokens = tokenize(text);
    if (!tokens?.length) return undefined;
    const key = index => tokens[index]?.key;
    const command = tokens.map(item => item.key).join(" ");
    if (/^(BEGIN|START TRANSACTION|BEGIN TRANSACTION|BEGIN WORK|COMMIT(?: TRANSACTION| WORK)?|ROLLBACK(?: TRANSACTION| WORK)?)$/.test(command)) {
        return {kind: key(0) === "START" ? "BEGIN" : key(0), label: command, parameters: []};
    }
    const operation = key(0);
    if (additionalCommands && operation === "SELECT") return {kind: "SELECT", label: "SELECT", parameters: []};
    const tableIndex = operation === "INSERT" && key(1) === "INTO" ? 2 :
        additionalCommands && operation === "UPDATE" ? 1 :
        additionalCommands && operation === "DELETE" && key(1) === "FROM" ? 2 : undefined;
    if (tableIndex === undefined || !identifier(tokens[tableIndex])) return undefined;
    let head = tableIndex + 1, table = tokens[tableIndex].value;
    while (key(head) === "." && identifier(tokens[head + 1]) && head < 7) {
        table += "." + tokens[head + 1].value;
        head += 2;
    }
    const parameters = parameterPositions(tokens);
    if (operation !== "INSERT") {
        if (operation === "UPDATE" && key(head) !== "SET") return undefined;
        const prefix = operation === "DELETE" ? "DELETE FROM " : "UPDATE ";
        return {kind: operation, write: true, label: prefix + table, table, parameters};
    }
    const closes = start => tokens.findIndex((item, index) => index > start && item.key === ")" && !item.depth);
    let columns;
    if (key(head) === "(") {
        const end = closes(head);
        if (end < 0) return undefined;
        columns = columnsAt(tokens, head + 1, end);
        head = end + 1;
    }
    if (!(key(head) === "VALUES" || key(head) === "DEFAULT" && key(head + 1) === "VALUES")) return undefined;
    const statement = {kind: "INSERT", write: true, label: "INSERT INTO " + table, table, columns, parameters};
    const indexOf = (from, ...words) => tokens.findIndex((item, index) => index >= from && !item.depth &&
        words.every((word, offset) => key(index + offset) === word && !tokens[index + offset].depth));
    const conflict = indexOf(head + 1, "ON", "CONFLICT");
    const returning = indexOf(head + 1, "RETURNING");
    const tail = returning < 0 ? tokens.length : returning;
    const valuesByColumn = [];
    if (columns && key(head) === "VALUES") {
        let rowStart = head + 1;
        while (valuesByColumn.length < 6 && key(rowStart) === "(") {
            const end = closes(rowStart);
            const values = end < 0 ? undefined : split(tokens, rowStart + 1, end, 1);
            if (!values || columns.length !== values.length) break;
            const expressions = values.map(([from, to]) => objectText(text, tokens, from, to));
            if (!expressions.every(Boolean)) break;
            valuesByColumn.push(Object.fromEntries(columns.map((name, index) => [name, expressions[index]])));
            if (key(end + 1) !== ",") break;
            rowStart = end + 2;
        }
    }
    if (valuesByColumn.length) statement.valuesByColumn = valuesByColumn;
    if (conflict >= 0) {
        if (conflict >= tail) return undefined;
        const doAt = indexOf(conflict + 2, "DO");
        if (doAt < 0 || doAt >= tail) return undefined;
        if (key(conflict + 2) === "(") {
            const end = closes(conflict + 2);
            if (end >= 0 && end < doAt) statement.conflictColumns = columnsAt(tokens, conflict + 3, end);
        }
        if (key(doAt + 1) === "NOTHING" && doAt + 2 === tail) {
            statement.conflictAction = "NOTHING";
            statement.conflict = "ON CONFLICT DO NOTHING";
        } else if (key(doAt + 1) === "UPDATE" && key(doAt + 2) === "SET") {
            const guard = indexOf(doAt + 3, "WHERE");
            const assignmentEnd = guard >= 0 && guard < tail ? guard : tail;
            const assignments = split(tokens, doAt + 3, assignmentEnd, 0);
            const parsed = assignments?.map(([first, last]) => identifier(tokens[first]) && key(first + 1) === "=" &&
                {column: tokens[first].value, value: objectText(text, tokens, first + 2, last)});
            if (parsed?.every(row => row?.value)) statement.assignments = parsed;
            statement.conflictAction = "UPDATE";
            statement.conflict = "ON CONFLICT DO UPDATE";
            if (guard >= 0 && guard < tail) {
                statement.conflict += " with a WHERE condition";
                statement.predicate = objectText(text, tokens, guard + 1, tail);
            }
        } else {
            return undefined;
        }
    }
    if (returning >= 0) statement.returning = objectText(text, tokens, returning + 1, tokens.length) || "the requested columns";
    return statement;
}

const selectKeywords = new Set(("ALL AND AS ASC BY DESC DISTINCT FETCH FIRST FOR FROM GROUP HAVING IN INTO IS JOIN " +
    "LAST LIMIT LOCK NOT NULL NULLS OFFSET OR ORDER RETURNING SELECT UNION INTERSECT EXCEPT WHERE WITH").split(" "));
const selectIdentifier = item => identifier(item) && item.value.length <= 88 &&
    !/[\r\n]/.test(item.value) && !selectKeywords.has(item.key);

// Accept a complete, single-table selection. Opaque SQL expressions and statement tails
// can conceal function calls, additional sources or writes, so they stay outside this shape.
function describeSelect(text) {
    const tokens = tokenize(text);
    if (!tokens?.length || tokens[0].key !== "SELECT") return undefined;
    let cursor = 1;
    const key = () => tokens[cursor]?.key;
    const take = word => {
        if (key() !== word) return false;
        cursor++;
        return true;
    };
    const name = () => selectIdentifier(tokens[cursor]) ? tokens[cursor++].value : undefined;
    function qualified(wildcard = false) {
        const parts = [];
        do {
            const part = wildcard && take("*") ? "*" : name();
            if (!part || parts.length === 3) return undefined;
            parts.push(part);
            if (part === "*") break;
        } while (take("."));
        return parts.join(".");
    }
    const operand = () => {
        const item = tokens[cursor];
        if (item && (/^(?:'|\d|\$[1-9]\d*$)/.test(item.value) || ["TRUE", "FALSE", "NULL"].includes(key()))) {
            return tokens[cursor++].value;
        }
        return qualified();
    };
    const comparison = () => {
        if (!operand()) return false;
        if (take("IS")) {
            take("NOT");
            return take("NULL") || take("TRUE") || take("FALSE");
        }
        if (!["=", "<>", "!=", ">", ">=", "<", "<="].includes(key())) return false;
        cursor++;
        return Boolean(operand());
    };
    const alias = () => {
        const explicit = take("AS");
        return name() || (explicit ? null : undefined);
    };
    const items = project => {
        const rows = [];
        do {
            const row = project();
            if (!row || rows.length >= 16) return undefined;
            rows.push(row);
        } while (take(","));
        return rows;
    };

    const distinct = take("DISTINCT");
    const selected = items(() => {
        const column = qualified(true);
        if (!column) return undefined;
        const named = alias();
        if (named === null || column.endsWith("*") && named) return undefined;
        return {column, alias: named};
    });
    if (!selected || !take("FROM")) return undefined;
    const table = qualified();
    if (!table) return undefined;
    const tableAlias = alias();
    if (tableAlias === null) return undefined;
    let where, orderBy, limit, offset;
    if (take("WHERE")) {
        const start = cursor;
        do {
            if (!comparison()) return undefined;
        } while (take("AND") || take("OR"));
        where = objectText(text, tokens, start, cursor);
        if (!where) return undefined;
    }
    if (take("ORDER")) {
        if (!take("BY")) return undefined;
        orderBy = items(() => {
            const expression = /^[1-9]\d*$/.test(key()) ? tokens[cursor++].value : qualified();
            if (!expression) return undefined;
            const direction = take("ASC") ? "ASC" : take("DESC") ? "DESC" : undefined;
            const nulls = take("NULLS") ? (take("FIRST") ? "FIRST" : take("LAST") ? "LAST" : null) : undefined;
            return nulls !== null && {expression, direction, nulls};
        });
        if (!orderBy) return undefined;
    }
    const count = () => /^(?:[0-9]+|\$[1-9]\d*)$/.test(key()) ? tokens[cursor++].value : undefined;
    if (take("LIMIT") && !(limit = count())) return undefined;
    if (take("OFFSET") && !(offset = count())) return undefined;
    if (cursor !== tokens.length) return undefined;
    return {kind: "SELECT", label: "SELECT FROM " + table, parameters: parameterPositions(tokens),
        select: {table, tableAlias, selected, distinct, where, orderBy, limit, offset}};
}

function requestParts(ts, call, resolve) {
    const callee = call.expression;
    if (!ts.isPropertyAccessExpression(callee) || !["query", "execute", "run"].includes(callee.name.text) ||
            !call.arguments.length || call.arguments.some(ts.isSpreadElement)) return undefined;
    let literal = resolve(call.arguments[0]), values = resolve(call.arguments[1]);
    if (literal && ts.isObjectLiteralExpression(literal)) {
        const fields = new Map();
        for (const property of literal.properties) {
            if (!ts.isPropertyAssignment(property) ||
                    !(ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)) ||
                    fields.has(property.name.text)) return undefined;
            fields.set(property.name.text, property.initializer);
        }
        if (call.arguments.length !== 1 || !fields.has("text")) return undefined;
        literal = resolve(fields.get("text"));
        values = resolve(fields.get("values"));
    }
    if (!literal || !ts.isStringLiteralLike(literal)) return undefined;
    return {literal, values: values && ts.isArrayLiteralExpression(values) &&
        values.elements.every(item => !ts.isOmittedExpression(item) && !ts.isSpreadElement(item)) ? values.elements : []};
}

module.exports = function sqlRequest(ts, call, resolve, additionalCommands) {
    const parts = requestParts(ts, call, resolve);
    const sql = parts && describe(parts.literal.text, additionalCommands);
    return sql && {...sql, ...parts};
};

module.exports.select = function selectRequest(ts, call, resolve) {
    if (call.arguments.length > 2) return undefined;
    const parts = requestParts(ts, call, resolve);
    const sql = parts && describeSelect(parts.literal.text);
    return sql && {...sql, ...parts};
};
