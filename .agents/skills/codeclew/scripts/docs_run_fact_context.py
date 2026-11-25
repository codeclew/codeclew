"""Lossless, page-local sharing of repeated fact qualifiers and citation fields."""
from __future__ import annotations

from collections import defaultdict
from itertools import pairwise
import json
from os.path import commonprefix

FIELDS = {
    'condition': 'P', 'uncertainty': 'U', 'sources': 'S', 'lines': 'L',
    'authority': 'A', 'status': 'T',
}
CATEGORIES = ('facts', 'boundaries')
INSTRUCTIONS = (
    'Read each fact with the fields supplied by all of its contextRefs in factContext. '
    'Append conditionTail verbatim to the referenced condition when present. '
    'Use the resulting sources and lines as the citation aliases and line anchors for that fact.'
)


def compact_json(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':'))


def field_bytes(key, value):
    # Count the field with its structural separator inside the containing record.
    return len(compact_json({key: value}).encode()) - 1


def copy_facts(packet):
    scopes, facts = {}, []
    for identity, scoped in packet.get('facts', {}).items():
        copies = dict(scoped)
        for category in CATEGORIES:
            if category in scoped:
                copies[category] = [dict(fact) for fact in scoped[category]]
                facts.extend(copies[category])
        scopes[identity] = copies
    return scopes, facts


def premise_prefixes(facts):
    by_text = defaultdict(list)
    for fact in facts:
        if 'condition' in fact:
            by_text[fact['condition']].append(fact)
    prefixes = set()
    values = sorted(by_text)
    for left, right in pairwise(values):
        prefix = commonprefix((left, right))
        if prefix != left and not prefix.endswith(' '):
            prefix = prefix[:prefix.rfind(' ')] if ' ' in prefix else ''
        if prefix := prefix.rstrip():
            prefixes.add(prefix)
    groups = [(prefix, [fact for value in values if value.startswith(prefix) for fact in by_text[value]])
              for prefix in sorted(prefixes)]
    return sorted(groups, key=lambda group: len(group[0]) * (len(group[1]) - 1), reverse=True)


def compact_fact_context(packet):
    scopes, facts = copy_facts(packet)
    catalog, counts = {}, defaultdict(int)

    def replace(key, shared, rows, tail=None):
        ref = FIELDS[key] + str(counts[key] + 1)
        fragment = {key: shared}
        overhead = field_bytes(ref, fragment)
        reduction = -overhead
        for fact in rows:
            refs = fact.get('contextRefs', [])
            reduction += field_bytes(key, fact[key]) - field_bytes('contextRefs', [*refs, ref])
            if refs:
                reduction += field_bytes('contextRefs', refs)
            if tail and (suffix := tail(fact[key])):
                reduction -= field_bytes('conditionTail', suffix)
        if reduction <= 0:
            return
        counts[key] += 1
        catalog[ref] = fragment
        for fact in rows:
            value = fact.pop(key)
            fact.setdefault('contextRefs', []).append(ref)
            if tail and (suffix := tail(value)):
                fact['conditionTail'] = suffix

    for key in FIELDS:
        groups = defaultdict(list)
        for fact in facts:
            if key in fact:
                groups[compact_json(fact[key])].append(fact)
        for rows in groups.values():
            if len(rows) > 1:
                replace(key, rows[0][key], rows)
    for prefix, rows in premise_prefixes(facts):
        retained = [fact for fact in rows if 'condition' in fact]
        if len(retained) > 1:
            replace('condition', prefix, retained, tail=lambda value: value[len(prefix):])
    if catalog:
        return {**packet, 'facts': scopes, 'factContext': catalog}
    return packet


def expand_fact_context(packet):
    """Recover the canonical fact records from a plain or compact author packet."""
    if 'factContext' not in packet:
        return packet
    catalog = packet['factContext']
    scopes, facts = copy_facts(packet)
    for fact in facts:
        for ref in fact.pop('contextRefs', []):
            fragment = catalog[ref]
            if len(fragment) != 1 or any(key not in FIELDS or key in fact for key in fragment):
                raise ValueError('Invalid shared fact field')
            fact.update(fragment)
        if 'conditionTail' in fact:
            fact['condition'] += fact.pop('conditionTail')
    return {key: value if key != 'facts' else scopes for key, value in packet.items() if key != 'factContext'}


def author_prompt(instructions, packet):
    plain = compact_json(packet)
    compact = compact_fact_context(packet)
    if compact is not packet:
        text = compact_json(compact)
        if len(text.encode()) + len(INSTRUCTIONS.encode()) + 2 < len(plain.encode()):
            return instructions + '\n\n' + INSTRUCTIONS + '\n\n' + text + '\n'
    return instructions + '\n\n' + plain + '\n'
