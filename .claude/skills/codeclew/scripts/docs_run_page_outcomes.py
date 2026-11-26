"""Project selected, qualified operation outcomes from the immutable author packet."""
from collections import defaultdict
from html import escape
from itertools import zip_longest
import re

# Priority follows the relationship carried by the proposition. Reuse only
# established projector kinds whose text describes an outcome or continuation.
OUTCOME_GROUPS = (
    ('http-failure-after-peer-commit', 'http-storage-message-effect', 'storage-message-effect',
     'http-further-subscriber-effect', 'further-subscriber-effect', 'further-subscriber-readback',
     'http-further-subscriber-readback', 'http-local-response-independent-poll', 'local-response-independent-poll',
     'message-storage-reader', 'storage-message-continuation', 'further-subscriber-outbox'),
    ('worker-reselection', 'possible-repeat-after-mark-failure', 'possible-later-selection'),
    ('handled-exception-continuation', 'loop-failure-path', 'message-publication-continuation', 'http-caller-continuation'),
    ('http-peer-conditional-no-write', 'http-peer-return-bypass', 'guard-return-bypass', 'conditional-no-write', 'await-bypass'),
    ('http-peer-commit-before-response', 'http-peer-guarded-return', 'message-peer-effect',
     'result-controlled-loop', 'early-exit', 'hard-exit'),
)
PRIORITY = {kind: group for group, kinds in enumerate(OUTCOME_GROUPS) for kind in kinds}
MAX_OUTCOMES = 8
MAX_OUTCOME_BYTES = 2500
MAX_OUTCOME_SOURCES = 36
MAX_OUTCOME_SOURCE_BYTES = 256 * 1024
CODE_SPAN = re.compile(r'(?<!`)(`+)(?!`)(.+?)(?<!`)\1(?!`)', re.DOTALL)


def select(packet, requirement, source_bytes):
    """Deduplicate exact statements, preserving complete citation sets and scope."""
    seen = {}
    for identity, scope in packet['facts'].items():
        for fact in scope.get('facts', ()):
            if identity != requirement and requirement not in fact.get('alsoFor', ()):
                continue
            kind = fact['kind'].lower().replace('_', '-')
            group = PRIORITY.get(kind)
            if group is None or fact.get('status') not in (None, 'SUPPORTED', 'SOURCE_QUALIFIED_CANDIDATE'):
                continue
            if fact.get('status') == 'SOURCE_QUALIFIED_CANDIDATE' and not fact.get('uncertainty'):
                continue
            sources = list(dict.fromkeys(fact['sources']))
            if not 0 < len(sources) <= MAX_OUTCOME_SOURCES or not all(ref in source_bytes for ref in sources):
                continue
            if sum(source_bytes[ref] for ref in sources) > MAX_OUTCOME_SOURCE_BYTES:
                continue
            wording = {key: fact[key] for key in ('text', 'condition', 'uncertainty') if fact.get(key)}
            if sum(len(text.encode()) for text in wording.values()) > MAX_OUTCOME_BYTES:
                continue
            key = tuple(wording.get(field, '').strip() for field in ('text', 'condition', 'uncertainty'))
            if key not in seen:
                seen[key] = (group, {**wording, 'cites': sources})
            else:
                previous_group, row = seen[key]
                merged = list(dict.fromkeys(row['cites'] + sources))
                if (len(merged) <= MAX_OUTCOME_SOURCES
                        and sum(source_bytes[ref] for ref in merged) <= MAX_OUTCOME_SOURCE_BYTES):
                    row['cites'] = merged
                seen[key] = (min(group, previous_group), row)

    groups = defaultdict(list)
    for group, row in seen.values():
        groups[group].append(row)
    ranked = [row for batch in zip_longest(*(groups[group] for group in sorted(groups)))
              for row in batch if row is not None]
    selected, covered, remaining = [], set(), MAX_OUTCOME_SOURCE_BYTES
    for row in ranked:
        new = set(row['cites']) - covered
        cost = sum(source_bytes[ref] for ref in new)
        if cost > remaining:
            continue
        selected.append(row)
        remaining -= cost
        covered.update(new)
        if len(selected) == MAX_OUTCOMES:
            break
    return selected


def inline(text):
    """Retain projector code spans, without rendering other embedded markup."""
    def literal(value):
        # The citation binder reserves the literal cite: prefix in Markdown.
        return escape(value, quote=False).replace('cite:', 'cite&#58;')
    pieces, offset = [], 0
    for match in CODE_SPAN.finditer(text):
        pieces.extend((literal(text[offset:match.start()]), '<code>' + literal(match[2]) + '</code>'))
        offset = match.end()
    pieces.append(literal(text[offset:]))
    return ''.join(pieces)


def stated_in_author_text(row, claims):
    wording = tuple(row[field] for field in ('text', 'condition', 'uncertainty') if row.get(field))
    expected = ' '.join(' '.join(wording).split())
    return any(claim['assessment'] == 'SUPPORTED'
               and ' '.join(claim['text'].split()) == expected
               and set(row['cites']) <= set(claim['cites']) for claim in claims)


def append(page, review, slot, claims, labels, cite):
    outcomes = [row for row in slot.get('sourceOutcomes', ()) if not stated_in_author_text(row, claims)]
    if not outcomes:
        return
    title, condition, uncertainty = (escape(labels[key]) for key in ('title', 'condition', 'boundary'))
    page['fragments'].append({'text': '\n<details class="operation-outcomes">\n<summary>' + title + '</summary>\n<ul>\n'})
    accounts = review['claims'][page['page']]
    for index, row in enumerate(outcomes, 1):
        identity = 'source-outcome-' + str(index)
        paragraphs = ['<p>' + inline(row['text']) + '</p>']
        for field, label in (('condition', condition), ('uncertainty', uncertainty)):
            if row.get(field):
                paragraphs.append('<p><strong>' + label + '</strong> · ' + inline(row[field]) + '</p>')
        paragraphs.append('<span data-clew-outcome-source="' + identity + '"></span>')
        page['fragments'].append({'id': identity, 'text': '<li>' + ''.join(paragraphs) + '</li>\n',
                                  'evidence': cite(row['cites'], slot)})
        accounts[identity] = 'SUPPORTED'
    page['fragments'].append({'text': '</ul>\n</details>\n\n'})
