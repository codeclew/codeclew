"""Select explicit Java decision suffixes without enumerating their prefixes."""

MAX_REGIONS = 3


def regions(node, visible, guard, caught, *, premise=(), local_throw=False, prefix=()):
    if not node:
        return
    kind, parts = node[0], node[3:]
    if kind == "block":
        statements = parts[0]
        if start := decision_start(statements, prefix):
            first, suffix = start
            bound = visible | {row[3] for row in statements[:first] if row[0] == "let"}
            yield ["block", suffix[0][1], node[2], suffix], bound, premise, local_throw
        for child in statements:
            yield from regions(child, visible, guard, caught, premise=premise, local_throw=local_throw, prefix=prefix)
            if child[0] == "let":
                visible = visible | {child[3]}
            if exits(child):
                break
            prefix = (*prefix, child)
    elif kind == "if":
        condition, yes, no = parts
        for holds, arm in ((True, yes), (False, no)):
            yield from regions(arm, visible, guard, caught, premise=(*premise, guard(condition, holds)),
                               local_throw=local_throw, prefix=(*prefix, condition))
    elif kind == "try":
        body, handlers, final, resources = parts
        if final or resources:
            return
        yield from regions(body, visible, guard, caught, premise=premise, local_throw=bool(handlers) or local_throw,
                           prefix=prefix)
        for handler in handlers:
            parameter, body = handler[3:]
            yield from regions(body, visible | {parameter[3]}, guard, caught,
                               premise=(*premise, caught(parameter)), local_throw=local_throw, prefix=(*prefix, parts[0]))


def decision_start(statements, prefix):
    for index, row in enumerate(statements):
        if row[0] == "if" and any(exits(arm) for arm in row[4:]):
            suffix = statements[index:]
            alternatives = all(has_exit(arm) for arm in row[4:]) or any(has_exit(item) for item in suffix[1:])
            if alternatives and supported(suffix):
                return (index, suffix) if any(has_choice(item) for item in (*prefix, *statements[:index])) else None
        if exits(row):
            break
    return None


def nodes(node):
    if isinstance(node, list) and node:
        if isinstance(node[0], str):
            yield node
            node = node[3:]
        for child in node:
            yield from nodes(child)


def has_choice(statement):
    # The ordinary path pass already compacts optional arms that only attempt
    # side effects. Start a fresh suffix when the prefix also branches on values.
    for node in nodes(statement):
        if node[0] == "let" and any(item[0] in {"choose", "and", "or"} for item in nodes(node[4])):
            return True
        if node[0] == "if" and any(item[0] in {"set", "mutate"} and item[3][0] == "id"
                                   for arm in node[4:] for item in nodes(arm)):
            return True
    return False


def reads(node):
    return {item[3] for item in nodes(node) if item[0] == "id"}


def has_exit(node):
    if node is None:
        return False
    kind, parts = node[0], node[3:]
    if kind in {"return", "throw"}:
        return True
    if kind == "block":
        return any(has_exit(child) for child in parts[0])
    return kind == "if" and any(has_exit(arm) for arm in parts[1:] if arm)


def exits(node):
    if node is None:
        return False
    kind, parts = node[0], node[3:]
    if kind in {"return", "throw"}:
        return True
    if kind == "block":
        return any(exits(child) for child in parts[0])
    return kind == "if" and all(exits(arm) for arm in parts[1:])


def supported(statements):
    for statement in statements:
        kind, parts = statement[0], statement[3:]
        if kind == "block" and not supported(parts[0]):
            return False
        if kind == "if" and not supported(arm for arm in parts[1:] if arm):
            return False
        if kind not in {"block", "if", "let", "expr", "empty", "return", "throw"}:
            return False
    return True
