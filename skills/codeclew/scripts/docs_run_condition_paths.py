"""Check the same source-selected path in the reader claims and trace."""
from collections import Counter, defaultdict
import re


TOKENS = re.compile(r"""(?:"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*')|[\w$]+|&&|\|\||==|!=|\S""")


def code(value):
    return tuple(TOKENS.findall(value.replace('`', '')))


def occurrences(tokens, action):
    return (index for index in range(len(tokens) - len(action) + 1)
            if tokens[index:index + len(action)] == action)


SKIP = re.compile(r'(?:skip\w*|bypass\w*|without|before|no|not|never|\u043d\u0435|\u0431\u0435\u0437|\u0434\u043e|\u043f\u0440\u043e\u043f\u0443\u0441\w*)', re.I)


def named(text, action, skipped):
    for index in occurrences(text, action):
        before = text[max(0, index - 5):index]
        after = text[index + len(action):index + len(action) + 3]
        if not skipped or any(SKIP.fullmatch(word) for word in (*before, *after)):
            return True
    return False


def usable(case):
    condition, actions = case['condition'], case['actions']
    return (0 < len(condition) <= 180 and 0 < len(actions) <= 3
            and all(0 < len(action) <= 90 for action in actions)
            and all(not set(item) & {'\n', '\r', '`'} for item in (condition, *actions)))


BRANCH_FACTS = {
    'early-exit', 'hard-exit', 'guarded-effects', 'ordered-effects', 'write-request', 'write-lineage', 'write-skip',
    'java-guarded-outcome', 'java-guarded-effects', 'java-ordered-effects', 'http-outcome', 'http-local-completion',
    'conditional-no-write', 'lookup-guard-return', 'guard-return-bypass', 'status-body', 'return-rejection',
    'response-decode-failure-outcome', 'await-bypass', 'exception-bypass', 'helper-outcome',
}
SPANS = re.compile(r'`([^`\r\n]+)`')
PREDICATE = re.compile(r'`([^`]+)`(?: is (true|false|null|non-null))?')


def premise(value):
    def operand(match):
        expression, test = match.groups()
        if test == 'false':
            return f'!({expression})'
        if test in ('null', 'non-null'):
            return f"({expression}) {'==' if test == 'null' else '!='} null"
        return expression
    return PREDICATE.sub(operand, value)


def proposition_case(fact, language):
    """Use an individually qualified path without inventing its complement."""
    kind = fact['kind'].lower().replace('_', '-')
    if kind not in BRANCH_FACTS or fact.get('status') not in (None, 'SUPPORTED', 'SOURCE_QUALIFIED_CANDIDATE'):
        return None, None
    text = fact['text']
    condition = fact.get('condition')
    scope = re.search(r'(?:^In | in )`([A-Za-z_$][\w$]*)`', text)
    selected_helper = re.search(r'in the retained body selected if `(?:this\.)?([A-Za-z_$][\w$]*)\(', text)
    if kind == 'guarded-effects':
        # The lexical arm supplies its own decision, even if the delivered
        # proposition also carries an outer call premise.
        arm = re.search(r' dispatches to `[^`]+` and (.+?), the (?:selected|opposite) arm', text)
        arm = arm or re.search(r'in `[^`]+` and (.+?), the (?:selected|opposite) arm', text)
        if arm:
            condition = arm[1]
    elif kind == 'ordered-effects' and (order := re.search(r', when (.+?), the normal source sequence is ', text)):
        condition = order[1]
    elif kind in ('early-exit', 'hard-exit'):
        match = re.match(r'If execution reaches .+? in `[^`]+` and (.+?), (?:the |it )', text)
        if match and (not condition or condition.startswith('If control reaches')):
            condition = match[1]
    if not condition:
        return None, 'NO_PREDICATE'
    if language != 'en' and re.search(r'\b[a-zA-Z]{2,}\b', PREDICATE.sub('', condition).replace(' and ', '')):
        return None, 'PREMISE_REQUIRES_TRANSLATION'
    scope = scope or selected_helper
    effect_text = text.split(', the selected arm contains the source sites for ', 1)[-1]
    parts = re.split(r'(?i)\b(?:bypasses|is skipped, including) ', effect_text, maxsplit=1)
    effect_text, skip_text = parts[0], parts[1] if len(parts) > 1 else ''
    known = set(SPANS.findall(condition)) | ({scope[1]} if scope else set())
    effects = [item for item in SPANS.findall(effect_text) if item not in known and len(item) <= 90]
    skipped = [item for item in SPANS.findall(skip_text) if item not in known and len(item) <= 90][:1]
    status = fact.get('_httpStatus')
    if type(status) is int and 100 <= status < 600:
        effects = [f'HTTP {status}']
    elif kind == 'early-exit':
        effects = [item for item in effects if item.startswith(('return', 'throw', 'break', 'continue'))][:1]
    elif kind == 'hard-exit':
        thrown = re.search(r'throws `([^`]+)`', effect_text)
        effects = [thrown[1]] if thrown else []
    elif kind == 'guarded-effects' and skipped:
        effects = []
    elif kind == 'write-lineage':
        value = re.search(r'receives `([^`]+)`', effect_text)
        write = re.search(r' at `([^`]+)`', effect_text)
        effects = [value[1], write[1]] if value and write else []
    elif kind.startswith('write-'):
        effects = effects[:1]
    else:
        effects = effects[-1:]
    actions = list(dict.fromkeys([*effects, *skipped]))
    if not actions:
        return None, 'NO_CONCRETE_EFFECT'
    return {'condition': premise(condition),
            'actions': actions, 'partial': True, **({'scope': scope[1]} if scope else {}),
            **({'skip': skipped} if skipped else {})}, None


def select(facts, aliases, *, visible=None, compact=None, language=None):
    facts = list(facts)
    groups, blocked = defaultdict(dict), defaultdict(dict)
    for fact in facts:
        for case in fact.get('_conditionCases', ()):
            group, key = case['group'], (case['condition'], tuple(case['actions']))
            if visible is not None and id(fact) not in visible:
                blocked[group][key] = 'PACKET_LIMIT'
                continue
            sources = fact['sources']
            if not sources or not all(handle in aliases for handle in sources):
                blocked[group][key] = 'UNAVAILABLE_SOURCE'
                continue
            cites = compact(sources) if compact else [aliases[handle] for handle in sources]
            if not usable(case) or len(cites) > 12:
                blocked[group][key] = 'CASE_LIMIT'
                continue
            rows = groups[group]
            if key not in rows or len(cites) < len(rows[key]['cites']):
                rows[key] = {**case, 'cites': cites}
    result, omitted, admitted = [], Counter(), 0
    for group in dict.fromkeys([*groups, *blocked]):
        rows = groups[group]
        missing = blocked[group].keys() - rows.keys()
        if missing:
            omitted[min(blocked[group][key] for key in missing)] += 1
            continue
        if len({condition for condition, _ in rows}) < 2:
            omitted['INCOMPLETE_DECISION'] += 1
            continue
        if len(rows) > 5 or len(result) + len(rows) > 10:
            omitted['CASE_LIMIT'] += 1
            continue
        admitted += 1
        result.extend({'decision': admitted, **{key: value for key, value in case.items() if key != 'group'}}
                      for case in rows.values())
    if not result and language:
        candidates = {}
        for fact in facts:
            path, reason = proposition_case(fact, language)
            if reason:
                omitted[reason] += 1
            if not path:
                continue
            sources = fact['sources']
            if visible is not None and id(fact) not in visible:
                omitted['PACKET_LIMIT'] += 1
            elif not sources or not all(handle in aliases for handle in sources):
                omitted['UNAVAILABLE_SOURCE'] += 1
            else:
                cites = compact(sources) if compact else [aliases[handle] for handle in sources]
                if not usable(path) or len(cites) > 12:
                    omitted['CASE_LIMIT'] += 1
                    continue
                key = path.get('scope'), path['condition'], tuple(path['actions']), tuple(path.get('skip', ()))
                kind = fact['kind'].lower()
                priority = (0 if kind in ('early_exit', 'hard_exit', 'http-outcome') else
                            1 if kind in ('guarded_effects', 'write_lineage') and not path.get('skip') else 2)
                if key not in candidates or len(cites) < len(candidates[key][1]['cites']):
                    candidates[key] = priority, {**path, 'cites': cites}
        for _, path in sorted(candidates.values(), key=lambda row: row[0])[:4]:
            admitted += 1
            result.append({**path, 'decision': admitted})
        if len(candidates) > 4:
            omitted['CASE_LIMIT'] += len(candidates) - 4
        if not (result or omitted):
            omitted['NO_QUALIFIED_BRANCH'] += 1
    return result, {'decisions': admitted, 'paths': len(result), 'omissions': dict(omitted)}


def check(review, value):
    paths = review.slot.get('conditionPaths')
    if not paths:
        return
    claims = []
    for row in value['claims']:
        if row['assessment'] == 'SUPPORTED':
            for paragraph in re.split(r'\n\s*\n|\n\s*(?=[-*] |\d+[.)] )', row['text']):
                spans = {code(text) for text in re.findall(r'`([^`\n]+)`', paragraph)}
                claims.append((set(row['cites']), spans, code(paragraph)))
    steps = [(code(row['condition'] or ''), row['kind'], code(row['label']))
             for row in value['steps']]
    requests, exits = {}, defaultdict(list)
    for path in paths:
        condition = code(path['condition'])
        actions = {code(action) for action in path['actions']}
        skipped = {code(action) for action in path.get('skip', ())}
        if path.get('scope'):
            actions.add(code(path['scope']))
        if not any({condition, *actions} <= spans and set(path['cites']) <= cites
                   and all(named(text, action, True) for action in skipped) for cites, spans, text in claims):
            review.issue('PAGE_SOURCE_PATH_MISSING', '/claims',
                         'Explain this source path in one supported paragraph. Put its condition and each action in separate code spans.',
                         sourcePath=path)
        matches = [index for index, (guard, kind, label) in enumerate(steps)
                   if condition == guard and (not path.get('stepKind') or kind == path['stepKind'])
                   and all(named(label, action, action in skipped) for action in actions)]
        if not matches:
            review.issue('PAGE_SOURCE_PATH_MISSING', '/steps',
                         'Put this exact path condition and its actions on a sequence step of the required kind.',
                         sourcePath=path)
        elif path.get('stepKind') == 'call':
            requests[path['decision']] = matches[0]
        elif skipped:
            exits[path['decision']].append((matches[0], path))
    for decision, request in requests.items():
        for exit_index, path in exits[decision]:
            if exit_index > request:
                review.issue('PAGE_SOURCE_PATH_MISSING', '/steps',
                             'Place this early exit before the outbound request it skips.', sourcePath=path)
