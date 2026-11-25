/* Keep coherent exceptional alternatives and reserve room for distinct outcomes. */
const concrete = new Set(["transaction_step", "sql_request", "sql_columns", "sql_result_check", "sql_result_flow", "post_commit_call", "post_helper_call", "broker_replay_window",
    "response_decode_failure_outcome", "conditional_repeat", "await_bypass", "result_continuation", "iteration_guard", "status_body", "response_body_read", "cleanup_call", "call_arguments", "helper_result", "helper_outcome", "return", "early_exit"]);

function rendered(sources, clauses) {
    const condition = clauses.length ? clauses.join(" and ") : undefined;
    return {sources: [...new Set(sources.filter(Boolean))], ...(condition ? {condition} : {})};
}

function alternatives(rows) {
    const groups = new Map();
    for (const row of rows) {
        const {guards, ...fact} = row;
        const catchScopes = guards.filter(guard => guard.group).map(guard => guard.group);
        const key = catchScopes.length ? JSON.stringify([fact, catchScopes]) : row;
        if (!groups.has(key)) groups.set(key, []);
        groups.get(key).push(row);
    }
    return [...groups.values()].map(group => {
        const [{sites, guards, ...fact}] = group;
        if (group.length === 1) return {...fact, ...rendered([...sites, ...guards.map(guard => guard.source)],
            guards.map(guard => guard.text))};
        const id = guard => JSON.stringify([guard.source, guard.text, guard.group]);
        const selected = group.slice(0, 5);
        const common = guards.filter(guard => selected.every(row => row.guards.some(other => id(other) === id(guard))));
        const shared = new Set(common.map(id));
        const cases = selected.map(row => row.guards.filter(guard => !shared.has(id(guard))));
        const different = [...new Set(cases.map(path => path.map(guard => guard.text).join(" and ")))];
        const clauses = common.map(guard => guard.text);
        // An empty residual is already covered by the common conjuncts.
        if (different.length && different.every(Boolean)) {
            clauses.push(different.length > 1 ? "(" + different.map(path => "(" + path + ")").join(" or ") + ")" : different[0]);
        }
        const sources = [...sites, ...common.map(guard => guard.source), ...cases.flat().map(guard => guard.source)];
        return {...fact, ...rendered(sources, clauses)};
    });
}

module.exports = function compact(rows, priority, maximum, evenly, roots) {
    const projected = alternatives(rows);
    const entries = new Map();
    const predicates = new Map();
    const facts = projected.filter(fact => {
        if (fact.kind === "registered_callback" || fact.kind === "registration_step") {
            const key = JSON.stringify(fact);
            if (!entries.has(key)) entries.set(key, fact);
            return false;
        }
        if (fact.kind !== "validation_predicate") return true;
        const key = JSON.stringify(fact.predicate);
        if (!predicates.has(key)) predicates.set(key, fact);
        return false;
    });
    const predicateMaximum = 16;
    const guards = evenly([...predicates.values()], predicateMaximum);
    function finish(behavior) {
        const remaining = facts.length - behavior.length;
        const remainingPredicates = predicates.size - guards.length;
        const output = [...entries.values(), ...behavior, ...guards];
        if (remaining || remainingPredicates) {
            const source = roots[0] || output[0]?.sources[0];
            if (source) {
                output.push({kind: "analysis_boundary",
                    text: "This TypeScript projection limits behavior facts to " + maximum +
                        " and validation predicates to " + predicateMaximum +
                        ". It omits " + remaining + " additional behavior facts and " + remainingPredicates + " validation predicates.",
                    sources: [source]});
            }
        }
        return output;
    }
    if (facts.length <= maximum) return finish(facts);
    const kinds = new Map();
    for (const fact of facts) {
        if (!kinds.has(fact.kind)) kinds.set(fact.kind, new Map());
        const cases = kinds.get(fact.kind);
        const condition = fact.condition || "";
        if (!cases.has(condition)) cases.set(condition, []);
        cases.get(condition).push(fact);
    }
    const queues = [...new Set([...priority, ...kinds.keys()])].flatMap(kind => {
        const entries = [...(kinds.get(kind)?.values() || [])];
        return entries.length ? [entries] : [];
    });
    const catchReturns = facts.filter(fact => fact.kind === "return" &&
        fact.condition?.includes("evaluation in the protected block enters the catch"));
    const chosen = new Set(evenly(catchReturns, 2));
    for (const kind of priority.filter(value => concrete.has(value))) {
        const byText = new Map();
        for (const fact of facts) {
            const key = fact.requestSource ? JSON.stringify([fact.text, fact.requestSource]) : fact.text;
            if (fact.kind !== kind || byText.has(key)) continue;
            if (kind === "call_arguments" && !fact.text.startsWith("The awaited ")) continue;
            byText.set(key, fact);
        }
        const count = ["transaction_step", "sql_request", "status_body", "call_arguments"].includes(kind) ? 4 : 2;
        for (const fact of evenly([...byText.values()], count)) chosen.add(fact);
    }
    while (chosen.size < maximum) {
        for (const cases of queues) {
            if (!cases.length) continue;
            const first = cases.shift();
            chosen.add(first.shift());
            if (first.length) cases.push(first);
            if (chosen.size === maximum) break;
        }
    }
    return finish(facts.filter(fact => chosen.has(fact)));
};
