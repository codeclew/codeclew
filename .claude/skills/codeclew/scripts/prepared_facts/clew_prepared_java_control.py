"""Locate statements that a preceding Java exit can bypass."""


def following_sites(statements, *, limit):
    sites = list(lexical_sites(statements))
    if len(sites) <= limit:
        return sites

    # Keep a witness from each region before using the remaining slots for
    # additional sites. One busy try body should not hide its finally block.
    first = {}
    for index, (_, region, _) in enumerate(sites):
        first.setdefault(region[-1:], index)
    selected = list(first.values())[:limit]
    for index in range(len(sites)):
        if len(selected) == limit:
            break
        if index not in selected:
            selected.append(index)
    return [sites[index] for index in sorted(selected)]


def lexical_sites(statements, region=(), conditional=False):
    for statement in statements:
        kind, parts = statement[0], statement[3:]
        if kind == "block":
            yield from lexical_sites(parts[0], region, conditional)
        elif kind == "if":
            condition, yes, no = parts
            yield from direct_site(condition, region, conditional)
            for arm in (yes, no):
                if arm:
                    yield from lexical_sites([arm], region, True)
        elif kind == "try":
            body, handlers, final, _ = parts
            yield from lexical_sites([body], (*region, "the try body"), conditional)
            for handler in handlers:
                yield from lexical_sites([handler[4]], (*region, "a catch block"), True)
            if final:
                yield from lexical_sites([final], (*region, "the finally block"), conditional)
        elif kind == "foreach":
            _, iterable, body = parts
            yield from direct_site(iterable, region, conditional)
            yield from lexical_sites([body], (*region, "the loop body"), True)
        elif kind in {"let", "expr", "return", "throw"}:
            yield from direct_site(parts[-1], region, conditional)

        if ends_scope(statement):
            break


def ends_scope(node):
    if not node:
        return False
    kind, parts = node[0], node[3:]
    if kind in {"return", "throw", "break", "continue"}:
        return True
    if kind == "block":
        return any(ends_scope(child) for child in parts[0])
    if kind == "if":
        return ends_scope(parts[1]) and ends_scope(parts[2])
    # The region analysis does not establish which exception a try handles or
    # whether an enclosing labelled construct changes where execution resumes.
    return False


def direct_site(expression, region, conditional):
    if not expression:
        return
    kind, parts = expression[0], expression[3:]
    if kind in {"call", "new", "set"}:
        yield expression, region, conditional
    elif kind in {"not", "eq", "neq", "ordered", "array", "and", "or", "choose"}:
        for index, operand in enumerate(parts):
            guarded = conditional or (index > 0 and kind in {"and", "or", "choose"})
            yield from direct_site(operand, region, guarded)


def site_label(text, region, conditional):
    if region:
        location = " within ".join(reversed(region))
        return f"{text} ({'conditional site' if conditional else 'site'} in {location})"
    return f"{text} (conditional site)" if conditional else text


def filtered_lookup(block):
    """Recognize a loop with only item-return and next-iteration routes."""
    if block[0] != "block" or len(block[3]) < 2:
        return None
    loop, fallback = block[3][-2:]
    if (loop[0] != "foreach" or fallback[0] != "return" or not fallback[3]
            or fallback[3][0] != "literal" or fallback[3][3] != "NULL"):
        return None
    variable, _, body = loop[3:]
    routes = []
    pending = [((body,), ())]
    for _ in range(24):
        if not pending:
            break
        statements, tests = pending.pop()
        if not statements:
            routes.append((tests, False))
            continue
        first, remaining = statements[0], statements[1:]
        kind, parts = first[0], first[3:]
        if kind == "block":
            pending.append(((*parts[0], *remaining), tests))
        elif kind == "if":
            predicate, yes, no = parts
            otherwise = (no, *remaining) if no else remaining
            if predicate[0] == "literal" and predicate[3] in {"TRUE", "FALSE"}:
                branch = (yes, *remaining) if predicate[3] == "TRUE" else otherwise
                pending.append((branch, tests))
            else:
                pending.append((otherwise, (*tests, (predicate, False))))
                pending.append(((yes, *remaining), (*tests, (predicate, True))))
        elif kind == "empty":
            pending.append((remaining, tests))
        elif kind == "continue" and parts[0] is None:
            routes.append((tests, False))
        elif kind == "return" and parts[0] and parts[0][0] == "id" and parts[0][3] == variable[3]:
            routes.append((tests, True))
        else:
            return None
    if pending or not 2 <= len(routes) <= 5 or {found for _, found in routes} != {True, False}:
        return None
    return loop, fallback, routes


def lookup_branches(node):
    """Locate adjacent lookup, null split, and shared continuation sites."""
    if not node:
        return
    kind, parts = node[0], node[3:]
    if kind == "block":
        statements = [item for item in parts[0] if item[0] != "empty"]
        for at, statement in enumerate(statements):
            if (0 < at < len(statements) - 1 and statement[0] == "if"
                    and (lookup := statements[at - 1])[0] == "let"
                    and lookup[4] and lookup[4][0] == "call"
                    and null_test(statement[3], lookup[3])):
                yes, no = (straight_arm(arm) for arm in statement[4:])
                continuation = straight_prefix(statements[at + 1:])
                if has_action(yes) and has_action(no) and has_action(continuation):
                    setup = list(reversed(straight_prefix(reversed(statements[max(0, at - 4):at - 1]),
                                                        setup=True)))
                    yield lookup, statement[3], setup, yes, no, continuation
            yield from lookup_branches(statement)
            if ends_scope(statement):
                break
    elif kind == "if":
        for arm in parts[1:]:
            yield from lookup_branches(arm)
    elif kind == "try":
        body, handlers, final, resources = parts
        if not final and not resources:
            yield from lookup_branches(body)
            for handler in handlers:
                yield from lookup_branches(handler[4])


def null_test(condition, name):
    while condition[0] == "not":
        condition = condition[3]
    if condition[0] not in {"eq", "neq"}:
        return False
    left, right = condition[3:]
    return any(variable[0] == "id" and variable[3] == name and value[0] == "literal" and value[3] == "NULL"
               for variable, value in ((left, right), (right, left)))


def straight_arm(arm):
    if not arm:
        return ()
    statements = [item for item in (arm[3] if arm[0] == "block" else (arm,)) if item[0] != "empty"]
    return straight_prefix(statements, complete=True)


def has_action(nodes):
    return any(node[0] != "let" or node[4] and node[4][0] in {"call", "new", "set", "mutate"}
               for node in nodes)


def straight_prefix(statements, *, setup=False, complete=False):
    actions = []
    for statement in statements:
        kind, parts = statement[0], statement[3:]
        if kind == "let" or (setup and kind == "assert"):
            actions.append(statement)
        elif (kind == "expr" or (kind == "return" and not setup and not complete)) and parts[0] and parts[0][0] in {
                "call", "new", "set", "mutate"}:
            actions.append(parts[0])
            if kind == "return":
                break
        else:
            return () if complete else actions
    return actions


def straight_method(block):
    """Select complete straight-line methods with a nontrivial evaluation sequence."""
    if block[0] != "block":
        return ()
    statements = tuple(item for item in block[3] if item[0] != "empty")
    if not 2 <= len(statements) <= 12:
        return ()
    end = len(statements) - (statements[-1][0] == "return")
    if any(node[0] not in {"let", "expr"} for node in statements[:end]):
        return ()
    actions = straight_prefix(statements[:end], complete=True)
    if len(actions) != end or not has_action(actions):
        return ()
    return statements
