#!/usr/bin/env python3
"""Connected source view from supported immutable Work reads; never run authors.

Containment and exact source are retained independently of execution semantics.
Full records and native receipts stay local. Only explicitly selected callables
are acquired, and their original bodies appear once in the author view.
"""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

E = runpy.run_path(str(Path(__file__).with_name('evidence_packet.py')))
Packet, BoundSource, Stop, encode = (E[k] for k in ('Packet', 'BoundSource', 'Stop', 'encode'))


def require(condition, code):
    if not condition:
        raise Stop(code, exit_code=2)


def source_range(record, source):
    """Use existing native coordinate bindings, never infer offsets from text."""
    raw = source['text'].encode('utf-8')
    require(source['textDigest'] == 'sha256:' + hashlib.sha256(raw).hexdigest(), 'SOURCE_TEXT_DIGEST_MISMATCH')
    occurrence = source.get('occurrence')
    if isinstance(occurrence, dict):
        binding = occurrence
        authority = 'SOURCE_OCCURRENCE'
    else:
        binding = record['normalized'].get('outlineOwnerSource', {})
        require(binding.get('sourceStatus') == 'SOURCE_RETAINED'
                and binding.get('sourceId') == source.get('id')
                and binding.get('sourceDigest') == source['textDigest']
                and binding.get('evidenceDigest') == source.get('evidenceDigest')
                and all(binding.get(k) == source.get(k) for k in ('file', 'startLine', 'endLine')),
                'MISSING_EXACT_OWNER_SOURCE_BINDING')
        first = record['normalized']['documentation']['sourceStructure']['nodes'][0]['sourceSpan']
        require(binding.get('fullCompilationSourceDigest') == first.get('fullCompilationSourceDigest'), 'OWNER_SOURCE_FILE_DIGEST_MISMATCH')
        authority = 'COMPILER_BOUND_OUTLINE_OWNER_SOURCE'
    start, end = binding.get('startByte', binding.get('byteStart')), binding.get('endByte', binding.get('byteEnd'))
    require(type(start) is int and type(end) is int and end - start == len(raw), 'MISSING_SOURCE_BYTE_OCCURRENCE')
    return start, end, authority


def project(records):
    """Pure projection, also used by focused neutral tests. No source parser."""
    lines = ['Source containment only; callback invocation, execution order, exception dispatch and effect success are not established.',
             'Selected callables only. Unselected callees/configuration/resources are not expanded; full provenance and unknowns: metadata.json.']
    metadata = {'schema': 'codeclew-connected-computation/1.0', 'callables': [],
                'omissions': ['Unselected declarations, configuration and resource bodies; reachable through the retained Work snapshot.'],
                'execution': 'NOT_ASSERTED', 'wholeProgramComplete': False}
    bodies, identities = {}, {}
    for index, item in enumerate(records, 1):
        record, source = item['record'], item['source']
        norm = record['normalized']; doc = norm.get('documentation', {})
        structure = doc.get('sourceStructure', {})
        require(structure.get('schema') == 'codeclew-kotlin-source-structure/1.0' and
                structure.get('authority') == 'SOURCE_CONTAINMENT_ONLY' and structure.get('execution') == 'NOT_ASSERTED',
                'MISSING_CONNECTED_SOURCE_STRUCTURE')
        raw = source['text'].encode('utf-8')
        start, end, coordinate_authority = source_range(record, source)
        # Normalized capture also retains the descriptor independently of the
        # original worker documentationOwner envelope.
        nodes = structure.get('nodes', [])
        require(isinstance(nodes, list) and nodes, 'EMPTY_CONNECTED_SOURCE_STRUCTURE')
        first_span = nodes[0].get('sourceSpan', {})
        owner_start, owner_end = first_span.get('ownerByteStart'), first_span.get('ownerByteEnd')
        require(type(owner_start) is int and type(owner_end) is int and start <= owner_start < owner_end <= end,
                'SOURCE_DOES_NOT_COVER_OWNER')
        def span_text(span):
            require(span.get('coordinateDomain') == 'ORIGINAL_UTF8_BYTES' and span.get('file') == source['file']
                    and span.get('ownerSymbolIdentity') == first_span.get('ownerSymbolIdentity')
                    and span.get('fullCompilationSourceDigest') == first_span.get('fullCompilationSourceDigest')
                    and span.get('compilationScope') == first_span.get('compilationScope')
                    and span.get('ownerByteStart') == owner_start and span.get('ownerByteEnd') == owner_end,
                    'STRUCTURE_SOURCE_IDENTITY_MISMATCH')
            a, b = span.get('byteStart'), span.get('byteEnd')
            require(type(a) is int and type(b) is int and owner_start <= a < b <= owner_end, 'STRUCTURE_RANGE_OUTSIDE_OWNER')
            return raw[a-start:b-start].decode('utf-8')
        for position, node in enumerate(nodes):
            require(node.get('id') == position and (node.get('parent') is None or
                    type(node['parent']) is int and 0 <= node['parent'] < position), 'INVALID_STRUCTURE_PARENT')
            span_text(node['sourceSpan'])
            if node['parent'] is not None:
                outer = nodes[node['parent']]['sourceSpan']; inner = node['sourceSpan']
                require(outer['byteStart'] <= inner['byteStart'] < inner['byteEnd'] <= outer['byteEnd'], 'INVALID_STRUCTURE_CONTAINMENT')
        key = 'c' + str(index)
        identity = (source['revision'], source['file'], start, end, source['textDigest'])
        text_key = bodies.setdefault(identity, key)
        identities[first_span['ownerSymbolIdentity']] = key
        lines.append(f'\n{key} {record["symbol"]} ({source["file"]}:{source["startLine"]}-{source["endLine"]})')
        if text_key == key:
            lines.extend(['```kotlin', source['text'], '```'])
        else:
            lines.append('Source: ' + text_key)
        controls = []
        for node in nodes:
            kind = node['kind']
            if kind in {'KtIfExpression', 'KtWhenExpression', 'KtWhenEntry', 'KtTryExpression', 'KtCatchClause',
                        'KtForExpression', 'KtWhileExpression', 'KtDoWhileExpression', 'KtLambdaExpression',
                        'KtReturnExpression', 'KtThrowExpression', 'KtBreakExpression', 'KtContinueExpression'} or node['origin'] != 'SOURCE_DECLARED':
                controls.append(node['id'])
        displayed = set(controls)
        # Structural bodies/conditions are references into the one source body.
        displayed.update(n['id'] for n in nodes if n['parent'] in controls and n['role'] != 'CHILD')
        def locator(span):
            return f'{span["byteStart"]}:{span["byteEnd"]}'
        for node in nodes:
            if node['id'] not in displayed:
                continue
            parent = node['parent']
            # Keep nearest displayed owner; full direct parent stays in metadata.
            while parent is not None and parent not in displayed:
                parent = nodes[parent]['parent']
            lines.append(f'{key}.n{node["id"]} {node["kind"].removeprefix("Kt").removesuffix("Expression")} '
                         f'{node["role"]} parent={"-" if parent is None else "n"+str(parent)} '
                         f'bytes={locator(node["sourceSpan"])} origin={node["origin"]}')
        typed = []
        for ordinal, event in enumerate(doc.get('events', [])):
            if 'kafka' in event:
                fact = {'eventPointer': f'/normalized/documentation/events/{ordinal}', 'sourceStructureId': event.get('sourceStructureId'), 'kafka': event['kafka']}
                typed.append(fact)
                lines.append(f'{key}.event{ordinal} declared-send {json.dumps(event["kafka"], ensure_ascii=False, separators=(",", ":"))}; delivery unknown')
        semantics = doc.get('semanticInputs')
        semantic_status = 'MISSING'
        if isinstance(semantics, dict) and isinstance(semantics.get('calls'), list) and isinstance(semantics.get('returns'), list):
            semantic_status = 'RETAINED'
            for ordinal, returned in enumerate(semantics['returns']):
                target = returned.get('targetRegion', {})
                target_ref = 'UNRESOLVED_LOCAL_TARGET'
                if returned.get('resolution') == 'COMPILER_EXACT':
                    if target.get('kind') == 'NAMED_FUNCTION' and target.get('ownerSymbolIdentity') == first_span['ownerSymbolIdentity']:
                        target_ref = key
                    else:
                        matches = [n['id'] for n in nodes if n['kind'] in {'KtLambdaExpression', 'KtNamedFunction', 'KtFunctionLiteral'}
                                   and n['sourceSpan']['byteStart'] == target.get('byteStart') and n['sourceSpan']['byteEnd'] == target.get('byteEnd')]
                        if len(matches) == 1:
                            target_ref = key + '.n' + str(matches[0])
                lines.append(f'{key}.return{ordinal} bytes={returned.get("byteStart")}:{returned.get("byteEnd")} '
                             f'target={target_ref} kind={target.get("kind", "UNKNOWN")} resolution={returned.get("resolution", "UNKNOWN")} '
                             f'nonLocal={returned.get("nonLocal", "UNKNOWN")}; exact target identity in metadata.json')
        else:
            lines.append('GAP: SEMANTIC_INPUTS_NOT_RETAINED; call/return target linkage is unavailable')
        unknowns = sorted(set(doc.get('boundaries', [])) | set(semantics.get('boundaries', []) if isinstance(semantics, dict) else []))
        lines.append('Boundaries: ' + ', '.join(unknowns or ['none reported; execution still not asserted']))
        if not structure.get('complete'):
            lines.append(f'GAP: {structure.get("omittedScans")} omitted traversals; exact gap ranges in metadata.json')
        metadata['callables'].append({**item, 'localId': key, 'structureNodesDisplayed': sorted(displayed),
                                      'typedBoundaries': typed, 'sourceTextKey': text_key, 'semanticInputsStatus': semantic_status, 'sourceCoordinateAuthority': coordinate_authority})
    # Exact compiler targets connect selected shared callee bodies; this does not
    # inline callbacks, infer receiver identity or broaden any equivalence proof.
    for index, item in enumerate(records, 1):
        semantics = item['record']['normalized']['documentation'].get('semanticInputs')
        calls = semantics.get('calls', []) if isinstance(semantics, dict) else []
        for ordinal, call in enumerate(calls):
            target = call.get('targetIdentity')
            if isinstance(target, dict):
                target = target.get('symbolIdentity')
            if call.get('resolution') == 'COMPILER_EXACT' and target in identities:
                lines.append(f'c{index}.call{ordinal} compiler target -> {identities[target]} (shared source body; receiver identity/execution not established)')
    return '\n'.join(lines) + '\n', metadata


def return_target(returned, nodes, owners):
    """Existing exact compiler region matching, shared by source and review."""
    target = returned.get('targetRegion', {})
    if returned.get('resolution') != 'COMPILER_EXACT':
        return None, None
    if target.get('kind') == 'NAMED_FUNCTION' and target.get('ownerSymbolIdentity') in owners:
        choices = owners[target['ownerSymbolIdentity']]
        return (choices[0] if len(choices) == 1 else None), None
    matches = [n for n in nodes if n['kind'] in {'KtLambdaExpression', 'KtNamedFunction', 'KtFunctionLiteral'}
               and n['sourceSpan']['byteStart'] == target.get('byteStart')
               and n['sourceSpan']['byteEnd'] == target.get('byteEnd')]
    return None, matches[0] if len(matches) == 1 else None


def compact_project(records):
    """Keep source syntax; inline comments carry only useful compiler links.

    Source bytes, SQL and configuration are unchanged apart from explicit
    display comments. Original source and every insertion stay in metadata.
    No token parser, semantic substitution or library interpretation is used.
    """
    _, metadata = project(records)
    metadata['authorViewFormat'] = 'COMPACT_SOURCE_WITH_BOUND_MARKERS/1.0'
    labels = {c['localId']: chr(65+i) for i, c in enumerate(metadata['callables'])}
    owners = {}
    for c in metadata['callables']:
        identity = c['record']['normalized']['documentation']['sourceStructure']['nodes'][0]['sourceSpan']['ownerSymbolIdentity']
        owners.setdefault(identity, []).append(labels[c['localId']])
    lines = ['Selected source. Letter headings name callables; /*→X*/ links a call or return to X; /*a1*/ names a callback. Execution and effect success are unproved.']
    gaps, unknown, kafka_facts, anchors = [], 0, [], {}
    seen_bodies = {}
    callback_number = 0
    for c in metadata['callables']:
        label = labels[c['localId']]
        record, source = c['record'], c['source']
        doc = record['normalized']['documentation']; structure = doc['sourceStructure']; nodes = structure['nodes']
        source_raw = source['text'].encode('utf-8'); source_start, source_end, _ = source_range(record, source)
        owner_span = nodes[0]['sourceSpan']; start, end = owner_span['ownerByteStart'], owner_span['ownerByteEnd']
        raw = source_raw[start-source_start:end-source_start]
        semantics = doc.get('semanticInputs')
        markers, callbacks = [], {}
        anchors[label] = {'recordId': record['id'], 'recordDigest': record.get('digest'), 'source': {k:v for k,v in source.items() if k != 'text'}, 'byteStart': start, 'byteEnd': end}
        def marker(offset, text, pointer, target=None):
            require(type(offset) is int and start <= offset < end, 'MARKER_OUTSIDE_BOUND_SOURCE')
            raw[:offset-start].decode('utf-8')
            markers.append({'start': offset-start, 'end': offset-start, 'replacement': text,
                            'pointer': pointer, 'target': target})
        if c['semanticInputsStatus'] == 'MISSING':
            gaps.append(label + ': call/return bindings unavailable')
        else:
            unknown += sum(call.get('resolution') != 'COMPILER_EXACT' for call in semantics['calls'])
            for i, returned in enumerate(semantics['returns']):
                target_label, node = return_target(returned, nodes, owners)
                if node is not None:
                    if node['id'] not in callbacks:
                        callback_number += 1; callback = 'a' + str(callback_number)
                        callbacks[node['id']] = callback
                        anchors[callback] = {'callable': label, 'nodeId': node['id'], 'sourceSpan': node['sourceSpan']}
                        marker(node['sourceSpan']['byteStart'], '/*'+callback+'*/', '/sourceStructure/nodes/'+str(node['id']), callback)
                    target_label = callbacks[node['id']]
                marker(returned.get('byteStart'), '/*→'+(target_label or '?')+'*/', '/semanticInputs/returns/'+str(i), target_label)
                if target_label is None: gaps.append(label + ': unresolved return target')
            for i, call in enumerate(semantics['calls']):
                choices = owners.get(call.get('targetIdentity'), [])
                if call.get('resolution') == 'COMPILER_EXACT' and len(choices) == 1:
                    marker(call['byteStart'], '/*→'+choices[0]+'*/', '/semanticInputs/calls/'+str(i), choices[0])
        for fact in c['typedBoundaries']:
            event = doc['events'][int(fact['eventPointer'].rsplit('/',1)[1])]
            marker(event['sourceSpan']['byteStart'], '/*Kafka*/', fact['eventPointer'], 'Kafka')
            kafka_facts.append(fact['kafka'])
        if not structure['complete']: gaps.append(label + ': structure budget gap; original source retained')
        edits = markers
        edits.sort(key=lambda e:(e['start'],e['end'],e['replacement']))
        chunks, cursor, displayed_edits, rendered_bytes = [], 0, [], 0
        for edit in edits:
            a,b = edit['start'],edit['end']; require(cursor <= a <= b <= len(raw), 'OVERLAPPING_DISPLAY_EDITS')
            before = raw[cursor:a]; replacement=edit['replacement'].encode('utf-8')
            chunks.extend([before,replacement]);rendered_bytes+=len(before)
            displayed_edits.append({**edit,'renderedStart':rendered_bytes,'renderedEnd':rendered_bytes+len(replacement),'original':raw[a:b].decode('utf-8')})
            rendered_bytes+=len(replacement);cursor=b
        chunks.append(raw[cursor:]);rendered=b''.join(chunks)
        # Reverse the exact edits, not a global comment/whitespace replacement.
        restored=rendered
        for edit in reversed(displayed_edits):
            restored=restored[:edit['renderedStart']]+edit['original'].encode()+restored[edit['renderedEnd']:]
        require(restored == raw, 'SOURCE_DISPLAY_RECONSTRUCTION_FAILED')
        c.update(compactLabel=label, displayEdits=displayed_edits, displaySourceRange={'byteStart':start,'byteEnd':end}, displayOriginalSha256=hashlib.sha256(raw).hexdigest(), displayedSourceSha256=hashlib.sha256(rendered).hexdigest())
        identity=(source['revision'],source['file'],start,end,source['textDigest'])
        lines.append('\n'+label+' · '+source['file'].rsplit('/',1)[-1])
        if identity in seen_bodies:
            lines.append('Same source as '+seen_bodies[identity])
        else:
            seen_bodies[identity]=label;lines.extend(['```kotlin',rendered.decode('utf-8'),'```'])
    lines.append('\nOther callees/configuration/resources are outside this selection.')
    if unknown: lines.append(str(unknown)+' call mappings unresolved; original calls are retained.')
    if kafka_facts:
        topics=sorted({f.get('topic',f.get('topicExpression','unknown')) for f in kafka_facts})
        lines.append('Kafka: '+', '.join(topics)+'; declared send, delivery unknown.')
    if gaps: lines.append('Gaps: '+'; '.join(gaps)+'.')
    metadata['compactAnchors']=anchors
    metadata['compactGaps']=gaps
    return '\n'.join(lines)+'\n',metadata


def review_project(saved, entrypoints=(), contexts=()):
    """Review saved native structure, never infer execution or semantic duties.

    Page inputs are (origin key, native page). Contexts additionally carry the
    root selected by the caller-trusted saved request receipt, not by a name rule.
    All output evidence points into the untouched saved inputs.
    """
    require(saved.get('schema') == 'codeclew-connected-computation/1.0', 'INVALID_SAVED_COMPUTATION')
    records = saved.get('callables')
    require(isinstance(records, list) and 1 <= len(records) <= 16, 'EXPLICIT_BOUNDED_SELECTION_REQUIRED')
    if saved.get('service'):
        require(all(c['record'].get('service') == saved['service'] and c['source'].get('service') == saved['service'] for c in records), 'SAVED_REVIEW_SERVICE_MISMATCH')
    _, validated = project(records)  # Existing source/digest/span/parent validation.
    labels = {c['record']['id']: chr(65+i) for i, c in enumerate(records)}
    require(len(labels) == len(records), 'DUPLICATE_OR_INVALID_SELECTION')
    owners = {}
    for c in records:
        owner = c['record']['normalized']['documentation']['sourceStructure']['nodes'][0]['sourceSpan']['ownerSymbolIdentity']
        owners.setdefault(owner, []).append(labels[c['record']['id']])
    rows, gaps, kinds, gap_evidence = [], [], {}, []
    sidecar = {'schema': 'codeclew-structural-review/1.0', 'rows': rows, 'gaps': gaps, 'gapEvidence': gap_evidence,
               'execution': 'NOT_ASSERTED', 'semanticCoverageAssessed': False, 'wholeProgramComplete': False,
               'originsMerged': False, 'callables': [], 'omitted': {}, 'contextRoots': []}
    controls = {'KtIfExpression', 'KtWhenExpression', 'KtWhenEntry', 'KtForExpression', 'KtWhileExpression',
                'KtDoWhileExpression', 'KtTryExpression', 'KtCatchClause', 'KtLambdaExpression', 'KtNamedFunction'}
    all_kinds, shown_kinds = {}, {}
    def emit(identity, kind, text, evidence, **details):
        rows.append({'id': identity, 'kind': kind, 'text': text, 'evidence': evidence, **details})
        kinds[kind] = kinds.get(kind, 0) + 1
    def point(origin, pointer):
        return {'input': origin, 'pointer': pointer}
    for ci, (item, checked) in enumerate(zip(records, validated['callables'])):
        record, source = item['record'], item['source']; label = labels[record['id']]
        require(item.get('compactLabel', label) == label, 'SAVED_LABEL_MISMATCH')
        doc = record['normalized']['documentation']; structure = doc['sourceStructure']; nodes = structure['nodes']
        raw = source['text'].encode(); start, end, _ = source_range(record, source)
        base = f'/callables/{ci}/record/normalized/documentation/'
        def evidence(pointer): return point('metadata', base + pointer)
        def node_point(n): return evidence('sourceStructure/nodes/' + str(n['id']))
        def ref(n): return label + '.n' + str(n['id'])
        def source_text(span):
            a, b = span.get('byteStart'), span.get('byteEnd')
            require(type(a) is int and type(b) is int and start <= a < b <= end, 'REVIEW_RANGE_OUTSIDE_SOURCE')
            return raw[a-start:b-start].decode('utf-8')
        def location(span):
            source_text(span)
            first = source['startLine'] + raw[:span['byteStart']-start].count(b'\n')
            last = source['startLine'] + raw[:span['byteEnd']-start-1].count(b'\n')
            return f'{first}-{last}' if first != last else str(first)
        def body(n): return ref(n) + '[' + location(n['sourceSpan']) + ']'
        def children(n, role): return [x for x in nodes if x['parent'] == n['id'] and x['role'] == role]
        def slot(n, role):
            values = children(n, role)
            return ','.join(body(x) for x in values) if values else 'not recorded'
        def condition(n, role):
            return ', '.join(source_text(x['sourceSpan']) for x in children(n, role)) or 'not recorded'
        def context(n):
            parent = n['parent']
            while parent is not None and nodes[parent]['kind'] not in controls: parent = nodes[parent]['parent']
            return ' in ' + ref(nodes[parent]) if parent is not None else ''
        emit(label, 'callable', label+' '+record['normalized'].get('name', record['id'])+' body '+body(nodes[0]),
             [point('metadata',f'/callables/{ci}/record'), point('metadata',f'/callables/{ci}/source')])
        sidecar['callables'].append({'label': label, 'recordId': record['id'], 'source': {k:v for k,v in source.items() if k != 'text'},
            'nativeLocatorAsRetained': item.get('dependencyArtifact'), 'nativeRecordPointer': f'/callables/{ci}/record',
            'sourceStructurePointer': base+'sourceStructure'})
        conditions = []
        sem = doc.get('semanticInputs'); semantic = sem if isinstance(sem, dict) else {}
        identity_fields = ('file', 'compilationScope', 'ownerSymbolIdentity', 'fullCompilationSourceDigest',
                           'ownerByteStart', 'ownerByteEnd', 'coordinateDomain')
        require(all(k not in semantic or semantic[k] == nodes[0]['sourceSpan'].get(k) for k in identity_fields), 'REVIEW_SEMANTIC_SCOPE_MISMATCH')
        if semantic and not all(k in semantic for k in identity_fields):
            semantic = {}
            gaps.append(label+': compiler identity metadata incomplete; links unasserted')
        for node in nodes:
            kind = node['kind']; all_kinds[kind] = all_kinds.get(kind, 0) + 1
            pointers = [node_point(node)]
            text = None
            if kind == 'KtIfExpression':
                text = 'if '+condition(node, 'CONDITION')+' => then '+slot(node, 'THEN')+'; else '+slot(node, 'ELSE')
                conditions.extend((ref(node), n) for n in children(node, 'CONDITION'))
            elif kind == 'KtWhenExpression': text = 'when '+condition(node, 'SUBJECT')+' => '+slot(node, 'WHEN_ENTRY')
            elif kind == 'KtWhenEntry':
                value = condition(node, 'CONDITION')
                native_else = [i for i,e in enumerate(doc.get('events', [])) if e.get('sourceStructureId') == node['id'] and e.get('kind') == 'ELSE']
                if value == 'not recorded' and native_else:
                    value = 'else (native event)'
                    pointers.extend(evidence('events/'+str(i)) for i in native_else)
                text = 'case '+value+' => '+slot(node, 'WHEN_BODY')
                conditions.extend((ref(node), n) for n in children(node, 'CONDITION'))
            elif kind in {'KtForExpression', 'KtWhileExpression', 'KtDoWhileExpression'}:
                keyword = {'KtForExpression':'for', 'KtWhileExpression':'while', 'KtDoWhileExpression':'do-while'}[kind]
                text = keyword+' '+condition(node, 'LOOP_RANGE' if kind == 'KtForExpression' else 'CONDITION')+' => '+slot(node, 'LOOP_BODY')
            elif kind == 'KtTryExpression':
                text = 'try '+slot(node, 'TRY_BODY')+'; catch '+slot(node, 'CATCH')+'; finally '+slot(node, 'FINALLY_BODY')
            elif kind == 'KtCatchClause': text = 'catch '+condition(node, 'CATCH_PARAMETER')+' => '+slot(node, 'CATCH_BODY')
            elif kind == 'KtLambdaExpression': text = 'callback body '+slot(node, 'CALLBACK_BODY')
            elif kind == 'KtNamedFunction': text = 'local function; execution not expanded'
            elif kind in {'KtReturnExpression', 'KtThrowExpression', 'KtBreakExpression', 'KtContinueExpression'}:
                target = 'unestablished'
                if kind == 'KtReturnExpression':
                    returned = [(i,r) for i,r in enumerate(semantic.get('returns', []))
                                if r.get('byteStart') == node['sourceSpan']['byteStart'] and r.get('byteEnd') == node['sourceSpan']['byteEnd']]
                    if len(returned) == 1:
                        ri, returned = returned[0]; target_label, target_node = return_target(returned, nodes, owners)
                        target = target_label or (ref(target_node) if target_node is not None else 'unestablished')
                        pointers.append(evidence('semanticInputs/returns/'+str(ri)))
                text = kind.removeprefix('Kt').removesuffix('Expression').lower()+' => '+target
            if text is not None:
                child_nodes = [n for n in nodes if n['parent'] == node['id'] and n['role'] != 'CHILD']
                pointers.extend(node_point(n) for n in child_nodes)
                emit(ref(node), 'structure', body(node)+context(node)+' '+text, pointers,
                     sourceSpan=node['sourceSpan'], origin=node['origin'], parent=node['parent'])
                shown_kinds[kind] = shown_kinds.get(kind, 0) + 1
        for event_index, event in enumerate(doc.get('events', [])):
            node_id = event.get('sourceStructureId')
            if event.get('kind') == 'RETURN' and type(node_id) is int and 0 <= node_id < len(nodes) and nodes[node_id]['kind'] != 'KtReturnExpression':
                emit(label+'.event'+str(event_index), 'source-result', label+' result expression '+body(nodes[node_id])+'; compiler return target not retained',
                     [evidence('events/'+str(event_index)), node_point(nodes[node_id])])
        if not isinstance(sem, dict): gaps.append(label+': compiler call/variable/return metadata missing')
        else:
            if 'variables' not in semantic: gaps.append(label+': compiler variable metadata not retained; lexical links unasserted')
            variables = semantic.get('variables', [])
            for vi, var in enumerate(variables):
                if var.get('kind') != 'VARIABLE_DECLARATION' or var.get('resolution') != 'COMPILER_EXACT': continue
                init = var.get('initializerSpan', {})
                for call_index, call in enumerate(semantic.get('calls', [])):
                    if (call.get('resolution') != 'COMPILER_EXACT' or call.get('byteStart') != init.get('byteStart')
                            or call.get('byteEnd') != init.get('byteEnd') or type(init.get('byteStart')) is not int): continue
                    uses = [(i,v,key,n) for i,v in enumerate(variables) for key,n in conditions
                            if v.get('kind') == 'VARIABLE_ACCESS' and v.get('resolution') == 'COMPILER_EXACT'
                            and v.get('variableIdentity') == var.get('variableIdentity') and var.get('variableIdentity')
                            and n['sourceSpan']['byteStart'] <= v['byteStart'] < v['byteEnd'] <= n['sourceSpan']['byteEnd']]
                    if not uses: continue
                    writes = [(i,v) for i,v in enumerate(variables) if v.get('variableIdentity') == var['variableIdentity']
                              and v.get('accessMode') in {'WRITE', 'READ_WRITE'}]
                    keys = list(dict.fromkeys(key for _,_,key,_ in uses))
                    text = label+'.call'+str(call_index)+' '+source_text(call)+'; '+var['name']+' read in '+','.join(keys)
                    if writes: text += '; writes retained'
                    emit(label+'.call'+str(call_index), 'lexical', text,
                         [evidence('semanticInputs/calls/'+str(call_index)), evidence('semanticInputs/variables/'+str(vi)),
                          *[evidence('semanticInputs/variables/'+str(i)) for i,_,_,_ in uses],
                          *[evidence('semanticInputs/variables/'+str(i)) for i,_ in writes]],
                         linkAuthority='SAME_COMPILER_VARIABLE_NOT_REACHING_VALUE', variableIdentity=var['variableIdentity'],
                         writes=writes, receiverObjectIdentity=call.get('receiverObjectIdentity', 'UNKNOWN'))
            unresolved = sum(c.get('resolution') != 'COMPILER_EXACT' for c in semantic.get('calls', []))
            if unresolved: gaps.append(f'{label}: {unresolved} unresolved call mappings; source retained')
        annotations = record['normalized'].get('jvmAnnotations', {})
        require('declaration' not in annotations or annotations['declaration'] == nodes[0]['sourceSpan']['ownerSymbolIdentity'], 'REVIEW_ANNOTATION_OWNER_MISMATCH')
        for ai, callable_ in enumerate(annotations.get('callables', [])):
            method = callable_.get('method', {})
            if method.get('identity') != nodes[0]['sourceSpan']['ownerSymbolIdentity']: continue
            for aj, annotation in enumerate(method.get('annotations', [])):
                if annotation.get('origin', {}).get('kind') != 'SOURCE': continue
                def argument(value):
                    if value.get('kind') in {'CONSTANT', 'ENUM', 'CLASS'}: return value.get('value', value)
                    if value.get('kind') == 'ARRAY': return [argument(v) for v in value.get('values', [])]
                    return {'unsupported': value.get('kind', 'unknown')}
                args = {k:argument(v) for k,v in annotation.get('arguments', {}).items()}
                emit(f'{label}.annotation{ai}.{aj}', 'annotation', label+' @'+annotation['typeName'].rsplit('.',1)[-1]+' '+json.dumps(args,ensure_ascii=False,separators=(',',':')),
                     [point('metadata',f'/callables/{ci}/record/normalized/jvmAnnotations/callables/{ai}/method/annotations/{aj}')],
                     owner=label, annotation=annotation['typeName'], declaredFields=args,
                     sourceOwnerPointer=f'/callables/{ci}/source', effectiveConfiguration='NOT_ASSERTED')
        for fact in checked['typedBoundaries']:
            index = int(fact['eventPointer'].rsplit('/',1)[-1])
            topic = fact['kafka'].get('topic', fact['kafka'].get('topicExpression', 'unresolved'))
            emit(label+'.event'+str(index), 'typed-boundary', label+' declared send topic='+json.dumps(topic,ensure_ascii=False),
                 [evidence('events/'+str(index))], typedFact=fact['kafka'])
        if not structure['complete']:
            gaps.append(f"{label}: structure scan gap ({structure['omittedScans']}); exact gaps retained")
            gap_evidence.append({'callable': label, 'evidence': [evidence('sourceStructure/gaps'), evidence('sourceStructure/omittedScans')]})
        sidecar['omitted'][label] = {'structureGaps': structure.get('gaps', []), 'boundaries': doc.get('boundaries', []),
                                    'semanticBoundaries': semantic.get('boundaries', [])}
        legend_boundaries = {'CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED', 'EXCEPTION_TYPES_AND_DISPATCH_NOT_ESTABLISHED'}
        if any(c.get('resolution') != 'COMPILER_EXACT' for c in semantic.get('calls', [])):
            legend_boundaries.add('DOCUMENTATION_CALL_TARGET_UNRESOLVED')
        other_boundaries = sorted((set(doc.get('boundaries', [])) | set(semantic.get('boundaries', []))) - legend_boundaries)
        if other_boundaries:
            gaps.append(label+': '+', '.join(other_boundaries))
            gap_evidence.append({'callable':label,'evidence':[evidence('boundaries')]})
    def page_scope(page):
        require(page.get('schema') == 'codeclew-documentation-work-page/1.0', 'INVALID_NATIVE_REVIEW_PAGE')
        require(all(isinstance(saved.get(k), str) and saved[k] and page.get(k) == saved[k] for k in ('work', 'snapshot')), 'REVIEW_PAGE_SCOPE_MISMATCH')
        if saved.get('service'):
            require(page.get('subject') == 'service:'+saved['service'], 'REVIEW_PAGE_SERVICE_MISMATCH')
    for origin, page in entrypoints:
        page_scope(page)
        for i, item in enumerate(page['items']):
            native = item.get('record', {}); ids = [v for v in native.get('rootDeclarations', []) if v in labels]
            if not ids: continue
            require(len(ids) == 1, 'AMBIGUOUS_TRIGGER_OWNER')
            selected = next(c for c in records if c['record']['id'] == ids[0])
            require(selected['source'].get('id') in native.get('sourceIds', []), 'TRIGGER_OWNER_SOURCE_MISMATCH')
            entry = native.get('entrypoint', {})
            if entry.get('sourceIds') is not None:
                require(selected['source'].get('id') in entry['sourceIds'], 'TRIGGER_OWNER_SOURCE_MISMATCH')
            if entry.get('dependencyIds') is not None:
                require(ids[0] in entry['dependencyIds'], 'TRIGGER_DECLARATION_MISMATCH')
            if entry.get('symbol') is not None:
                require(entry['symbol'] == selected['record']['symbol'], 'TRIGGER_DECLARATION_MISMATCH')
            trigger = entry.get('trigger', {})
            label = labels[ids[0]]
            displayed = {k:v for k,v in trigger.items() if k not in {'authority', 'pathResolution'} and v is not None}
            emit(label+'.'+origin+'.trigger'+str(i), 'trigger', label+' declared trigger '+json.dumps(displayed,ensure_ascii=False,separators=(',',':')),
                 [point(origin,f'/items/{i}/record')], owner=label, declaredFields=displayed, runtimeRegistration='NOT_ESTABLISHED')
    # One declaration's annotation/trigger values may coincide. Retain every
    # qualified field name and both evidence origins; share only equal JSON values.
    for label in labels.values():
        config = [r for r in rows if r.get('owner') == label and r['kind'] in {'annotation', 'trigger'}]
        if {r['kind'] for r in config} != {'annotation', 'trigger'}: continue
        facts = []
        prefixes = ['@'+r['annotation'].rsplit('.',1)[-1] if r['kind'] == 'annotation' else 'trigger' for r in config]
        for ordinal, (r, prefix) in enumerate(zip(config, prefixes)):
            if prefixes.count(prefix) > 1: prefix += '#'+str(ordinal+1)
            for key, value in r['declaredFields'].items():
                entries = value.items() if r['kind'] == 'trigger' and key == 'configuration' else [(key, value)]
                facts.extend((prefix+'.'+k, v) for k,v in entries)
        groups = {}
        for name, value in facts:
            group = groups.setdefault(encode(value), {'names': [], 'value': value})
            if name not in group['names']: group['names'].append(name)
        if not any(len(g['names']) > 1 for g in groups.values()): continue
        fields = []
        for group in groups.values():
            names = group['names']; leaves = {n.rsplit('.',1)[-1] for n in names}
            name = next(iter(leaves)) if len(names) > 1 and len(leaves) == 1 else '/'.join(names)
            fields.append(name+'='+json.dumps(group['value'],ensure_ascii=False,separators=(',',':')))
        text = label+' declared '+','.join(dict.fromkeys('@'+r['annotation'].rsplit('.',1)[-1] for r in config if r['kind'] == 'annotation'))+' + trigger: '+'; '.join(fields)
        position = rows.index(config[0]); rows[:] = [r for r in rows if r not in config]
        merged = {'id': label+'.config', 'kind': 'configuration', 'text': text,
                  'evidence': [p for r in config for p in r['evidence']], 'contributors': config,
                  'qualifiedFields': facts, 'effectiveConfiguration': 'NOT_ASSERTED'}
        rows.insert(position, merged)
    frontier_counts, frontier_evidence, context_pages, budget_counts = {}, {}, {}, {}
    for origin, page, root in contexts:
        page_scope(page); require(root in labels, 'CONTEXT_ROOT_NOT_SELECTED')
        label = labels[root]; sidecar['contextRoots'].append(root)
        counts = frontier_counts.setdefault(label, {})
        frontier_evidence.setdefault(label, []).append(point(origin,'/items'))
        context_pages.setdefault(label, []).append(page)
        for i, item in enumerate(page['items']):
            frontier = item.get('record', {}).get('frontier')
            if not isinstance(frontier, dict): continue
            kind = frontier.get('kind', 'UNKNOWN'); counts[kind] = counts.get(kind, 0) + 1
            frontier_evidence.setdefault(label, []).append(point(origin,f'/items/{i}/record/frontier'))
            if kind == 'BUDGET':
                target = labels.get(frontier.get('methodId'), 'unselected target')
                budget_counts[label] = budget_counts.get(label, 0) + 1
                emit(f'{label}.budget{budget_counts[label]}', 'frontier', f"{label} frontier {frontier.get('code', 'BUDGET')} at {target}; OPEN even if body is selected",
                     [point(origin,f'/items/{i}/record/frontier')], rootAssociation='SAVED_REQUEST_RECEIPT', semanticClosure='UNKNOWN')
        if page.get('omitted'): gaps.append(label+': native context omissions retained')
    for label, counts in sorted(frontier_counts.items()):
        pages = context_pages[label]; signature = [(p.get('membershipDigest'), p.get('total'), p.get('contextDigest')) for p in pages]
        require(all(s == signature[0] for s in signature), 'CONTEXT_PAGE_MEMBERSHIP_MISMATCH')
        ids = [i.get('id') for p in pages for i in p['items']]
        require(len(ids) == len(set(ids)), 'DUPLICATE_CONTEXT_ITEMS')
        if signature[0][1] != len(ids) or not any(p.get('nextCursor') is None for p in pages):
            gaps.append(label+': context continuation/count incomplete or unspecified')
        emit(label+'.frontiers', 'frontier-summary', label+' native frontiers '+json.dumps(counts,separators=(',',':')), frontier_evidence.get(label, []), semanticClosure='UNKNOWN')
    if not contexts: gaps.append('Thread context/frontiers not supplied; body structure is not thread closure')
    kinds = {kind:sum(r['kind'] == kind for r in rows) for kind in dict.fromkeys(r['kind'] for r in rows)}
    sidecar.update(kinds=kinds, nativeNodeKinds=all_kinds, displayedNodeKinds=shown_kinds,
                   omittedNodeKinds={k:v for k,v in all_kinds.items() if k not in shown_kinds})
    text = ['Structural self-review. A/B/... follow the saved callable order; nN is a native node, [L-L] original lines.',
            'in = lexical containment. Callback invocation/count/order, exception dispatch and effect success are unproved.',
            'Variable links are same-symbol reads, not reaching values. Declared configuration is not deployed state; frontier summaries are not closure proofs.']
    text.extend(row['text'] for row in rows)
    text.extend('Gap: '+gap for gap in dict.fromkeys(gaps))
    text.append('Review reminders only; no whole-program or meaning-coverage proof. Full origins/records/spans remain in the saved sidecars.')
    return '\n'.join(text)+'\n', sidecar


def acquire(args):
    packet = Packet(args)
    coverage, _ = packet.call('coverage', ['docs', 'context', '--root', args.root, '--service', args.service,
                                        '--snapshot', args.snapshot, '--format', 'compact', '--limit', '1'])
    packet.validate_page(coverage, 'codeclew-docs-page/1.0')
    rows = [r for r in coverage['items'] if r.get('kind') == 'COVERAGE' and r.get('id') == args.service]
    require(len(rows) == 1, 'MISSING_SERVICE_REVISION')
    selections = json.loads(args.selection.read_bytes())
    require(isinstance(selections, list) and 1 <= len(selections) <= 16, 'EXPLICIT_BOUNDED_SELECTION_REQUIRED')
    records, seen, cache = [], set(), {}
    for selection in selections:
        require(isinstance(selection, dict) and set(selection) <= {'declarationId', 'sourceId', 'sourceReference'}, 'INVALID_SELECTION')
        identity = selection.get('declarationId')
        require(isinstance(identity, str) and identity not in seen, 'DUPLICATE_OR_INVALID_SELECTION')
        seen.add(identity)
        record, raw = packet.dependency(identity)
        require(record.get('id') == identity and record.get('service') == args.service and record.get('kind') == 'SYMBOL', 'DEPENDENCY_SCOPE_MISMATCH')
        references = packet.navigation(record)
        source_ids = record.get('sourceIds', [])
        sid = selection.get('sourceId') or (source_ids[0] if len(source_ids) == 1 else None)
        reference = selection.get('sourceReference') or (references[0] if len(references) == 1 else None)
        require(sid in source_ids and reference in references, 'EXPLICIT_SOURCE_BINDING_REQUIRED')
        key = (sid, reference)
        if key not in cache:
            reader = BoundSource.__new__(BoundSource)
            reader.args = args; reader.work = args.work; reader.parts = []
            reader.binding = {'revision': rows[0]['revision']}
            reader.call = packet.call
            cache[key] = reader.source(sid, reference)
        records.append({'record': record, 'source': cache[key], 'dependencyArtifact': raw})
    text, metadata = (project if getattr(args, "view_format", "compact") == "connected" else compact_project)(records)
    metadata.update(work=args.work, snapshot=args.snapshot, service=args.service, coverage=coverage,
                    selected=selections, nativeReceipts=packet.receipts)
    (args.output_dir / 'author.txt').write_text(text, encoding='utf-8')
    (args.output_dir / 'metadata.json').write_text(encode(metadata), encoding='utf-8')
    # Direct comparison contains exactly the same selected source regions once.
    direct, used = [], set()
    for item in records:
        source = item['source']; start, end, _ = source_range(item['record'], source)
        source_text = source['text']
        if getattr(args, 'view_format', 'compact') == 'compact':
            span = item['record']['normalized']['documentation']['sourceStructure']['nodes'][0]['sourceSpan']
            a,b = span['ownerByteStart'],span['ownerByteEnd']
            source_text = source_text.encode()[a-start:b-start].decode('utf-8');start,end=a,b
        identity = (source['revision'], source['file'], start, end, hashlib.sha256(source_text.encode()).hexdigest())
        if identity not in used:
            used.add(identity); direct.append(source_text)
    (args.output_dir / 'direct-source.txt').write_text('\n'.join(direct) + '\n', encoding='utf-8')
    measurement = {'authorBytes': len(text.encode()), 'localMetadataBytes': (args.output_dir / 'metadata.json').stat().st_size,
                   'directSourceBytes': (args.output_dir / 'direct-source.txt').stat().st_size,
                   'tokenCounts': 'UNMEASURED: use the same explicit tokenizer on author.txt and direct-source.txt; bytes are not tokens',
                   'nativeCommands': len(packet.receipts), 'nativeSeconds': sum(r['elapsedSeconds'] for r in packet.receipts),
                   'qualityOrTokenWinClaimed': False}
    (args.output_dir / 'measurement.json').write_text(encode(measurement), encoding='utf-8')
    return measurement


def read_review_json(path):
    path = Path(path)
    require(path.is_file() and path.stat().st_size <= 64 * 1024 * 1024, 'REVIEW_INPUT_UNAVAILABLE_OR_OVERSIZED')
    return json.loads(path.read_bytes())


def review_saved(args):
    """Local saved-artifact rendering; never launch Work reads or acquire source."""
    saved = read_review_json(args.input); raw = args.input.read_bytes()
    require(isinstance(saved, dict), 'INVALID_SAVED_COMPUTATION')
    inputs = {'metadata': (args.input, raw)}; pages, contexts = [], []
    for i, path in enumerate(args.entrypoints):
        key = 'entrypoints'+str(i); inputs[key] = (path, path.read_bytes())
        pages.append((key, read_review_json(path)))
    for i, path in enumerate(args.context_receipt):
        receipt = read_review_json(path); argv = receipt['command']
        require(receipt.get('exitCode') == 0 and argv.count('--input') == 1, 'INVALID_CONTEXT_RECEIPT')
        require(argv.count('--work') == 1 and argv[argv.index('--work')+1] == saved.get('work'), 'CONTEXT_RECEIPT_WORK_MISMATCH')
        request_path = Path(argv[argv.index('--input')+1])
        require(request_path.resolve().parent == path.resolve().parent, 'CONTEXT_REQUEST_OUTSIDE_RECEIPT')
        name = receipt['stdout']; require(Path(name).name == name, 'INVALID_CONTEXT_STDOUT_PATH')
        stdout = path.parent/name; data = stdout.read_bytes()
        require(hashlib.sha256(data).hexdigest() == receipt['stdoutSha256'], 'CONTEXT_STDOUT_DIGEST_MISMATCH')
        request = read_review_json(request_path); query = request['query']
        require(query.get('kind') == 'THREAD_CONTEXT', 'INVALID_CONTEXT_QUERY')
        key = 'context'+str(i); inputs[key] = (stdout, data)
        inputs[key+'-request'] = (request_path, request_path.read_bytes())
        inputs[key+'-receipt'] = (path, path.read_bytes())
        contexts.append((key, read_review_json(stdout), query.get('symbolContains')))
    text, review = review_project(saved, pages, contexts)
    require(not args.output_dir.exists(), 'REVIEW_OUTPUT_EXISTS')
    args.output_dir.mkdir(parents=True)
    review['inputs'] = {}
    for key, (path, data) in inputs.items():
        require(path.read_bytes() == data, 'REVIEW_INPUT_CHANGED')
        name = key+'.json'; (args.output_dir/name).write_bytes(data)
        review['inputs'][key] = {'original': str(path.absolute()), 'copy': name, 'sha256': hashlib.sha256(data).hexdigest()}
    (args.output_dir/'review.txt').write_text(text, encoding='utf-8')
    (args.output_dir/'review.json').write_text(encode(review), encoding='utf-8')
    result = {'status': 'SAVED_STRUCTURAL_REVIEW_READY', 'review': str(args.output_dir/'review.txt'),
              'sidecar': str(args.output_dir/'review.json'), 'reviewBytes': len(text.encode()),
              'rows': len(review['rows']), 'kinds': review['kinds'], 'nativeCommands': 0,
              'semanticCoverageAssessed': False}
    (args.output_dir/'result.json').write_text(encode(result), encoding='utf-8')
    return text


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    if argv and argv[0] == 'review-saved':
        parser = argparse.ArgumentParser(description='Render optional structural self-review from saved admitted metadata only')
        parser.add_argument('--input', type=Path, required=True)
        parser.add_argument('--entrypoints', type=Path, action='append', default=[])
        parser.add_argument('--context-receipt', type=Path, action='append', default=[], help='Saved successful native THREAD_CONTEXT read receipt; includes its original request and hashed stdout')
        parser.add_argument('--output-dir', type=Path, required=True)
        args = parser.parse_args(argv[1:])
        try:
            print(review_saved(args), end='')
            return 0
        except (Stop, OSError, ValueError, KeyError, TypeError, IndexError) as error:
            print(encode({'status': error.status if isinstance(error, Stop) else 'INVALID_SAVED_REVIEW', 'details': str(error)}), end='')
            return 2
    parser = argparse.ArgumentParser(description=__doc__)
    for flag in ('clew', 'root', 'service', 'snapshot', 'work'):
        parser.add_argument('--' + flag, required=True)
    parser.add_argument('--view-format', choices=('compact', 'connected'), default='compact')
    parser.add_argument('--selection', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args(argv)
    args.output_dir.mkdir(parents=True, exist_ok=False)
    try:
        print(encode(acquire(args)), end='')
    except (Stop, ValueError, KeyError, TypeError, UnicodeError) as error:
        value = {'status': error.status if isinstance(error, Stop) else 'INVALID_COMPUTATION_INPUT', 'details': str(error)}
        (args.output_dir / 'failure.json').write_text(encode(value), encoding='utf-8')
        print(encode(value), end='')
        return 2
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
