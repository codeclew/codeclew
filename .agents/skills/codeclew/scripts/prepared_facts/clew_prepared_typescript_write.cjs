/* Relate one retained write occurrence to paths through the existing TypeScript solver. */
module.exports = function ({ts, focus, source, unwrap, callName, caption, emit}) {
    const arrivals = [], skips = new Set();
    let coverageOpen = false;
    const where = (scope, node) => [node.getStart(scope.file) + scope.offset, node.getEnd() + scope.offset];
    const branchKey = guard => JSON.stringify([guard.body, guard.predicate, guard.outcome]);
    const clauses = state => state.guards.filter(guard => guard.predicate !== undefined);

    function precedesCleanup(scope, cleanup) {
        const selected = focus.spans[scope.excerpt.id];
        const parent = cleanup.parent;
        if (!selected || !ts.isTryStatement(parent) || parent.finallyBlock !== cleanup) return false;
        return [parent.tryBlock, parent.catchClause?.block].filter(Boolean).some(block => {
            const [start, end] = where(scope, block);
            return start <= selected[0] && selected[1] <= end;
        });
    }

    function match(scope, call, statement, sql) {
        const span = focus.spans[scope.excerpt.id];
        if (!span) return false;
        const [first, last] = span;
        const exact = node => node && where(scope, node).every((point, index) => point === span[index]);
        if (exact(call) || statement && (ts.isExpressionStatement(statement) || ts.isReturnStatement(statement)
                || ts.isVariableStatement(statement) && statement.declarationList.declarations.length === 1) && exact(statement)) return true;
        if (!sql?.write) return false;
        const [start, end] = where(scope, sql.literal);
        return start <= first && last <= end;
    }

    function branchSkips(scope, call) {
        const skips = [];
        for (let child = call; child !== scope.node && child.parent; child = child.parent) {
            const parent = child.parent;
            if (!ts.isIfStatement(parent) || child === parent.expression) continue;
            const test = caption(parent.expression);
            if (!test) continue;
            const outcome = child !== parent.thenStatement;
            skips.push({key: branchKey({body: scope.id, predicate: parent.expression.getStart(scope.file), outcome}),
                text: "When " + test + " is " + outcome + ", this branch bypasses the selected write.",
                sources: [source(scope, parent.expression)]});
        }
        return skips;
    }

    function callPath(scope, call, state) {
        return {
            branches: [...state.writePath.branches, ...branchSkips(scope, call)],
            exits: [...state.writePath.exits, ...state.exits],
        };
    }

    function client(expression, localBinding, seen = new Set()) {
        const node = unwrap(expression);
        if (!node || !ts.isIdentifier(node)) return undefined;
        const binding = localBinding(node);
        if (!binding || seen.has(binding)) return undefined;
        const declaration = binding.declarations[0];
        if (!(declaration.parent.flags & ts.NodeFlags.Const) || !declaration.initializer) return undefined;
        seen.add(binding);
        const initializer = unwrap(declaration.initializer);
        return ts.isIdentifier(initializer) ? client(initializer, localBinding, seen) :
            ts.isAwaitExpression(initializer) && ts.isCallExpression(unwrap(initializer.expression)) ? binding : undefined;
    }

    function transaction(scope, call, sql, state, awaited, localBinding, selected) {
        if (state.messageContradiction) return state.writeTransactions;
        const method = call.expression;
        const receiver = ts.isPropertyAccessExpression(method) && client(method.expression, localBinding);
        const transactions = new Map(state.writeTransactions);
        for (const argument of call.arguments) {
            const escaped = client(argument, localBinding);
            if (escaped) transactions.delete(escaped);
        }
        if (!receiver) return transactions;
        const site = source(scope, call);
        if (method.name.text !== "query" || !sql || !awaited) {
            transactions.delete(receiver);
        } else if (sql.kind === "BEGIN") {
            transactions.set(receiver, {begin: site});
        } else if (sql.kind === "ROLLBACK" || sql.kind === "COMMIT") {
            const previous = transactions.get(receiver);
            if (sql.kind === "COMMIT" && previous?.write) {
                emit("write_commit", "On this normal path, the selected write's await fulfills before source awaits COMMIT on the same client binding.",
                    [focus.site, previous.begin, previous.write, site], state,
                    "The change can become visible if the requests use one database transaction session, the SQL changes a row and COMMIT succeeds.");
            }
            transactions.delete(receiver);
        } else if (selected && transactions.has(receiver)) {
            transactions.set(receiver, {...transactions.get(receiver), write: site});
        }
        return transactions;
    }

    function sqlDetails(scope, sql, state, sources) {
        const description = [];
        if (sql.columns?.length) description.push("The insert names " + sql.columns.join(", ") + ".");
        if (sql.conflictAction === "NOTHING") description.push("ON CONFLICT DO NOTHING skips a conflicting row.");
        if (sql.conflictAction === "UPDATE") {
            const target = sql.conflictColumns?.length ? " on " + sql.conflictColumns.join(", ") : "";
            const assignments = sql.assignments?.map(({column, value}) => column + " = " + value).join(", ");
            description.push("ON CONFLICT" + target + " DO UPDATE" + (assignments ? " assigns " + assignments : "") + ".");
            if (sql.predicate) description.push("A conflicting row is updated only where " + sql.predicate + " is true.");
        }
        if (sql.returning) description.push("RETURNING requests " + sql.returning + ".");
        const actuals = sql.parameters.slice(0, 8).flatMap(index => {
            const argument = sql.values[index - 1];
            const value = argument && caption(argument);
            return value ? [{text: "$" + index + " receives " + value, site: source(scope, argument)}] : [];
        });
        if (actuals.length) description.push(actuals.map(argument => argument.text).join(", ") + ".");
        if (description.length) emit("write_sql", description.join(" "), [...sources, ...actuals.map(arg => arg.site)], state);
    }

    function call(scope, call, statement, sql, state, awaited, localBinding) {
        const selected = match(scope, call, statement, sql) && (!sql || sql.write);
        if (selected) {
            const site = source(scope, call);
            const path = callPath(scope, call, state);
            arrivals.push({state, path, identity: JSON.stringify([scope.excerpt.id, where(scope, call)])});
            if (!state.messageContradiction) {
                const evidence = [focus.site, site, sql && source(scope, sql.literal)];
                const request = sql ? callName(call) + " with " + sql.label : callName(call);
                emit("write_request", (awaited ? "The callback path awaits " : "The callback path calls ") + request + ".",
                    evidence, state, "The request's outcome depends on the runtime binding and data.");
                if (sql) sqlDetails(scope, sql, state, evidence);
            }
        }
        return {...state, writeReached: state.writeReached || selected,
            writeTransactions: transaction(scope, call, sql, state, awaited, localBinding, selected)};
    }

    function finish(validationBoundaries) {
        if (!arrivals.length) {
            emit("write_boundary", "The admitted TypeScript path does not establish a reachable call at this exact write occurrence.", [focus.site]);
            return;
        }
        const candidates = arrivals.filter(({state}) => !state.messageContradiction);
        if (!candidates.length) {
            if (coverageOpen) {
                emit("write_boundary", "The bounded callback paths leave this write's incoming message compatibility open.", [focus.site]);
            } else {
                const sourceProof = arrivals.every(({state}) => state.messageValidation)
                    ? "The incoming topic and retained normal-return validation contradict" : "The retained incoming topic contradicts";
                emit("write_compatibility", sourceProof + " the branch needed by this write.",
                    [focus.site, ...arrivals.flatMap(({state}) => state.messageEvidence)], undefined, undefined,
                    {compatibility: "INCOMPATIBLE"});
            }
            return;
        }
        if (!coverageOpen && candidates.every(({state}) => state.messageCompared && !state.messageOpen)) {
            emit("write_compatibility", "The selected write has a callback path compatible with the retained incoming message discriminator.",
                [focus.site, ...candidates.flatMap(({state}) => state.messageEvidence)], undefined, undefined,
                {compatibility: "COMPATIBLE"});
        } else if (validationBoundaries?.size) {
            emit("write_boundary", "The retained validator has a guard whose read-only evaluation remains open.",
                [focus.site, ...validationBoundaries.values()], undefined, undefined,
                {diagnostic: [...validationBoundaries.keys()]});
        }
        const reached = new Set(candidates.flatMap(({state}) => clauses(state).map(branchKey)));
        const common = clauses(candidates[0].state).filter(guard => candidates.every(({state}) =>
            clauses(state).some(candidate => branchKey(candidate) === branchKey(guard))));
        if (common.length) {
            emit("write_guard", "The cited callback paths reach the selected write when " + common.map(guard => guard.text).join(" and ") + ".",
                [focus.site, ...common.map(guard => guard.source)]);
        }
        for (const {path} of candidates) {
            for (const skip of path.branches) {
                if (reached.has(skip.key) || skips.has(skip.key)) continue;
                skips.add(skip.key);
                emit("write_skip", skip.text, [focus.site, ...skip.sources]);
            }
            for (const exit of path.exits) {
                const terminal = clauses(exit.state);
                if (!terminal.length || exit.resume.writeReached) continue;
                const condition = terminal.map(guard => guard.text).join(" and ");
                const text = "When " + condition + ", " + exit.text.charAt(0).toLowerCase() + exit.text.slice(1) +
                    " before reaching the selected write.";
                emit("write_skip", text, [focus.site, exit.site, ...terminal.map(guard => guard.source)]);
            }
        }
    }
    return {call, callPath, finish, precedesCleanup, incomplete: () => { coverageOpen = true; },
        ambiguous: () => new Set(arrivals.map(row => row.identity)).size > 1};
};
