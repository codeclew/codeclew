/* Bound the inventory to explicit calls and constructions in one retained callable body. */
const families = ["HTTP", "database", "broker", "file system", "process"];
const stageFamilies = {INSERT: "database", READ: "database", DELETE: "database", PUBLISH: "broker"};
const limit = 24;

module.exports = function inventory({ts, scope, source, callName, unwrap, emit}) {
    const calls = new Map(), gaps = new Set();
    let truncated = false;

    function gap(node) {
        if (gaps.size < limit) gaps.add(source(scope, node || scope.node.body));
    }

    function containsCall(node, depth = 0) {
        if (!node || ts.isFunctionLike(node) || ts.isTypeNode(node)) return false;
        if (depth > 18 || ts.isCallExpression(node) || ts.isNewExpression(node) || ts.isTaggedTemplateExpression(node)) return true;
        return Boolean(ts.forEachChild(node, child => containsCall(child, depth + 1) ? true : undefined));
    }

    function record(node, {label = callName(node), effect, target, sql} = {}) {
        if (calls.size >= limit && !calls.has(node)) {
            truncated = true;
            return;
        }
        const forward = effect?.httpTarget;
        const family = forward && (!/^[a-z][a-z0-9+.-]*:/i.test(forward) || /^https?:/i.test(forward))
            ? "HTTP" : stageFamilies[effect?.stage];
        const callee = unwrap(node.expression);
        const fetchSpelling = ts.isCallExpression(node) && ts.isIdentifier(callee) && callee.text === "fetch";
        const classification = family ? "classified" : target ? "source" : "open";
        const previous = calls.get(node);
        calls.set(node, {
            label, family, classification, fetchSpelling, candidate: !family && !target && (sql?.label || previous?.candidate),
            sites: [source(scope, node), ...(effect?.sites || []), target?.id],
        });
    }

    function finish() {
        const complete = !gaps.size && !truncated;
        const rows = [...calls.values()];
        const found = families.filter(family => rows.some(row => row.family === family));
        const unknown = rows.filter(row => row.classification === "open");
        const closed = complete && !unknown.length;
        const absent = closed ? families.filter(family => !found.includes(family)) : [];
        const names = items => [...new Set(items)].join(", ");
        const details = found.map(family => family + " via " + names(rows.filter(row => row.family === family).map(row => row.label)));
        const scope = "the selected callable body's potentially reachable explicit calls and constructions";
        let text = details.length ? "In " + scope + ", the retained source identifies " + details.join(", ") + "." :
            "The inventory examines " + scope + ".";
        if (found.includes("HTTP")) text += " The standard fetch binding and destination remain runtime-dependent.";
        if (absent.length) text += " This scope has no direct call in the " + absent.join(", ") + " families.";
        if (unknown.length) {
            text += " Classification remains open at " + names(unknown.map(row => row.label +
                (row.candidate ? " (with " + row.candidate + " as a SQL argument)" : ""))) + ".";
            if (complete && !rows.some(row => row.fetchSpelling)) {
                text += " The enumerated sites contain no separate call spelled `fetch(...)`.";
            }
        }
        if (!complete) text += " Additional direct call sites may occur in source the projection did not cover.";
        const helpers = rows.filter(row => row.classification === "source");
        if (helpers.length) text += " Calls through " + names(helpers.map(row => row.label)) + " have their own effect scope.";
        emit("direct_effect_inventory", text, [scope.id, ...rows.flatMap(row => row.sites), ...gaps], undefined, undefined, {
            scope: "DIRECT_CALLS_IN_SELECTED_CALLABLE", coverage: closed ? "CLOSED_DIRECT_SCOPE" : "OPEN",
            siteCoverage: complete ? "COMPLETE" : "PARTIAL", presentFamilies: found,
            candidateFamilies: rows.some(row => row.candidate) ? ["database"] : [], absentFamilies: absent,
        });
    }

    return {containsCall, finish, gap, record};
};
