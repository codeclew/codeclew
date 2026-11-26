"""Slot contracts and evidence-preserving repair for isolated page authors."""
from copy import deepcopy
import re
from pathlib import Path
import runpy


CONDITION_PATHS = runpy.run_path(str(Path(__file__).with_name("docs_run_condition_paths.py")))


COMMON = ('slot', 'reviewed', 'disposition', 'limitations')
ROLES = {
    'GUIDE': ('title', 'claims'),
    'OPERATION': ('claims', 'participants', 'steps'),
    'CONTRACTS': ('claims', 'contracts'),
    'DISCOVERY': ('discovery', 'starts'),
    'PROCESS_MAP': ('claims', 'routes'),
}
ARRAYS = ('claims', 'participants', 'steps', 'starts', 'contracts', 'routes')
FIXED_ACTORS = ('caller', 'handler')
KINDS = ('call', 'await', 'return', 'async', 'note')
ROUTE_KINDS = (*KINDS, 'reference')
DISPOSITIONS = ('EXPLAINED', 'EXPLICIT_UNKNOWN')
MAX_ENUM_CITES = 400
MAX_ISSUES = 28
HEADING_LENGTH = 160
STEP_TEXT_LENGTH = 240
CLAIM_TEXT_LENGTH = 20000
ROUTE_PROSE_LENGTH = 512
CONTENT_ISSUES = {
    'PAGE_CLAIMS_REQUIRED', 'PAGE_CLAIM_INVALID', 'PAGE_RETURN_CLAIM_SUPPORT_REQUIRED',
    'PAGE_CONTRACT_FIELDS_REQUIRED', 'PAGE_DISCOVERY_RESULT_REQUIRED', 'PAGE_ROUTE_TRANSITION_REQUIRED',
    'PAGE_SOURCE_PATH_MISSING',
}
LOCATOR = re.compile(r'(S[0-9a-zA-Z]+|V[0-9]+)(?::([1-9][0-9]{0,7})(?:-([1-9][0-9]{0,7}))?|#L([1-9][0-9]{0,7})(?:-L?([1-9][0-9]{0,7}))?)')


def owned_fields(slot):
    if slot.get('ownedContentOnly'):
        return ('claims',) if slot['kind'] == 'GUIDE' else ('routes',)
    return ROLES[slot['kind']]


def schema_for_slot(base, slot):
    schema = deepcopy(base)
    kind = slot['kind']
    common = COMMON[:-1] if slot.get('ownedContentOnly') else COMMON
    own = owned_fields(slot)
    fields = (*common, *own)
    properties = {key: schema['properties'][key] for key in fields}
    properties['slot'] = {'type': 'string', 'enum': [slot['key']]}
    if 'title' in own:
        properties['title'] = {'type': 'string'}
    if kind == 'DISCOVERY':
        properties['discovery'] = {'type': 'string', 'enum': ['NO_ADDITIONAL_STARTS', 'ADDITIONAL_STARTS']}
    definitions = schema['$defs']
    used = {'claim'} if 'claims' in own or kind == 'DISCOVERY' else set()
    if kind in ('DISCOVERY', 'OPERATION'):
        used.update(('participant', 'step'))
    if kind == 'DISCOVERY':
        used.add('start')
        definitions['start']['properties']['startKind']['enum'] = ['call', 'async']
    if kind == 'CONTRACTS':
        used.add('contract')
    if kind == 'PROCESS_MAP':
        used.update(('route', 'step'))
        if slot.get('transitionRoutes'):
            definitions['step']['properties']['kind']['enum'] = list(ROUTE_KINDS)
        entries = slot.get('operationIds', ())
        if entries:
            entry = {'type': 'string', 'enum': list(entries)}
            route = definitions['route']['properties']
            route['origin'] = route['participants']['items']['properties']['entry'] = entry
            route['terminal'] = {'anyOf': [entry, {'type': 'null'}]}
            actors = {'type': 'string', 'enum': ['caller', *entries]}
            definitions['step']['properties'].update({'from': actors, 'to': actors})
    cite_options = list(slot['citations'])
    if 0 < len(cite_options) <= MAX_ENUM_CITES:
        used.add('citation')
        definitions['citation'] = {'type': 'string', 'enum': cite_options}
        for key in ('claim', 'route', 'contract'):
            definitions[key]['properties']['cites']['items'] = {'$ref': '#/$defs/citation'}
    claim = definitions['claim']
    if kind == 'GUIDE' and slot.get('ownedContentOnly'):
        claim['properties']['assessment']['enum'] = ['SUPPORTED']
    if slot['returns']:
        claim['properties']['returns']['items'] = {'type': 'string', 'enum': list(slot['returns'])}
    else:
        claim['required'].remove('returns')
        del claim['properties']['returns']
    return {'type': 'object', 'properties': properties, 'required': list(fields), 'additionalProperties': False,
            '$defs': {key: value for key, value in definitions.items() if key in used}}


def instructions_for_slot(instructions, kind):
    parts = re.split(r'\n(?=(?:GUIDE|OPERATION|CONTRACTS|DISCOVERY|PROCESS_MAP) \| )', instructions)
    return '\n'.join([parts[0].rstrip(), *(part for part in parts[1:] if part.startswith(kind + ' | '))]).strip()


def resolve_citation(slot, value):
    """Resolve only packet-admitted identity and a wholly visible line qualifier."""
    if not isinstance(value, str):
        return None, ()
    ref = value.strip()
    admitted = slot['citations']
    if ref in admitted:
        return ref, ()
    handles = [alias for alias, handle in admitted.items() if ref == handle]
    if len(handles) == 1:
        return handles[0], ()
    spans = slot.get('citationSpans', {})
    if found := LOCATOR.fullmatch(ref):
        label, first, last, markdown_first, markdown_last = found.groups()
        start = int(first or markdown_first)
        end = int(last or markdown_last or first or markdown_first)
        if end < start:
            return None, ()
        choices = [alias for alias, ranges in spans.items() if alias in admitted
                   and any((region == label or alias == label) and lower <= start <= end <= upper
                           for region, lower, upper in ranges)]
        if len(choices) == 1:
            return choices[0], ()
        visible_regions = {view for view, _, _ in spans.get(label, ())}
        if not choices and visible_regions:
            choices = [alias for alias, ranges in spans.items() if alias in admitted and
                       any(view in visible_regions and lower <= start <= end <= upper for view, lower, upper in ranges)]
        return None, tuple(choices)
    if re.fullmatch(r'V[0-9]+', ref) and (region := slot.get('sourceRegions', {}).get(ref)):
        start, end = region
        choices = [alias for alias, ranges in spans.items() if alias in admitted
                   and any(view == ref and lower <= start <= end <= upper for view, lower, upper in ranges)]
        return (choices[0], ()) if len(choices) == 1 else (None, tuple(choices))
    return None, ()


def pointer(value):
    return value[:100].replace('~', '~0').replace('/', '~1')


def safe_value(value):
    return value[:100] if isinstance(value, str) else value if value is None or isinstance(value, (bool, int)) else type(value).__name__


class Review:
    def __init__(self, slot):
        self.slot = slot
        self.issues = []
        self.count = 0
        self.return_paths = {}
        self.needs_source = False

    def issue(self, code, path, message, **details):
        self.count += 1
        self.needs_source |= code in CONTENT_ISSUES
        if len(self.issues) < MAX_ISSUES:
            self.issues.append({'code': code, 'path': path, 'message': message, **details})

    def shape(self, value, path, fields):
        if not isinstance(value, dict):
            self.issue('PAGE_OBJECT_REQUIRED', path, 'Supply an object with the listed fields.', fields=list(fields))
            return False
        missing, extra = [field for field in fields if field not in value], sorted(value.keys() - set(fields))
        for field in missing:
            self.issue('PAGE_FIELD_REQUIRED', path + '/' + field, 'Supply this field.')
        for field in extra:
            self.issue('PAGE_FIELD_UNKNOWN', path + '/' + pointer(field), 'Use the supplied response schema.')
        return not missing

    def choices(self, value, path, allowed, code, message):
        if value not in allowed:
            self.issue(code, path, message, received=safe_value(value), allowed=[safe_value(item) for item in allowed[:32]])
            return False
        return True

    def line(self, value, path, size, *, empty=False):
        if not (isinstance(value, str) and len(value) <= size and '\n' not in value and '\r' not in value
                and (empty or value.strip())):
            self.issue('PAGE_SINGLE_LINE_REQUIRED', path, 'Supply text on one line.', maxCharacters=size)

    def cites(self, row, path):
        values = row['cites']
        path += '/cites'
        if not isinstance(values, list) or not values:
            self.issue('PAGE_CITATION_REQUIRED', path, 'Add an alias from the packet citations keys for this claim.',
                       example=list(self.slot['citations'])[:1])
            return
        accepted = []
        for number, reference in enumerate(values):
            ref, choices = resolve_citation(self.slot, reference)
            if ref is None:
                self.issue('PAGE_CITATION_OUTSIDE_PACKET', f'{path}/{number}',
                           'Copy an S alias supporting the claim from the packet citations keys. Include line details in the claim text.',
                           received=safe_value(reference), matchingAliases=list(choices[:8]))
            else:
                accepted.append(ref)
        if len(accepted) == len(values):
            row['cites'] = list(dict.fromkeys(accepted))

    def claims(self, rows, path, *, supported_only=False):
        if not isinstance(rows, list) or not rows:
            self.issue('PAGE_CLAIMS_REQUIRED', path, 'Add at least one supported claim or source-bounded unknown.')
            return
        for number, claim in enumerate(rows):
            target = f'{path}/{number}'
            if isinstance(claim, dict) and not self.slot['returns']:
                claim.setdefault('returns', [])
            if not self.shape(claim, target, ('heading', 'text', 'cites', 'assessment', 'returns')):
                continue
            self.line(claim['heading'], target + '/heading', HEADING_LENGTH, empty=True)
            text = claim['text']
            if not isinstance(text, str) or not text.strip() or len(text) > CLAIM_TEXT_LENGTH or 'clew-sequence' in text:
                self.issue('PAGE_CLAIM_INVALID', target + '/text', 'Give reader Markdown. Put sequence steps in the steps array.')
            assessments = ('SUPPORTED',) if supported_only else ('SUPPORTED', 'EXPLICIT_UNKNOWN')
            self.choices(claim['assessment'], target + '/assessment', assessments,
                         'PAGE_CLAIM_ASSESSMENT_REQUIRED', 'Record whether the cited source supports this statement.')
            self.cites(claim, target)
            refs = claim['returns']
            if not isinstance(refs, list):
                self.issue('PAGE_LITERAL_RETURN_REVIEW_REQUIRED', target + '/returns', 'Supply an array of packet return IDs.')
                continue
            for index, reference in enumerate(refs):
                item = f'{target}/returns/{index}'
                if self.choices(reference, item, tuple(self.slot['returns']), 'PAGE_RETURN_REF_UNKNOWN',
                                'Use a key from the packet returns object.'):
                    if earlier := self.return_paths.get(reference):
                        self.issue('PAGE_RETURN_REF_DUPLICATED', item, 'Bind this return ID once.', earlierPath=earlier)
                    self.return_paths[reference] = item
                    if claim['assessment'] != 'SUPPORTED':
                        self.issue('PAGE_RETURN_CLAIM_SUPPORT_REQUIRED', target + '/assessment',
                                   'Bind the return ID to a supported claim that cites the retained declaration.')

    def trace(self, row, path, *, actors=FIXED_ACTORS, participants=True, route_references=False):
        available = list(actors)
        renamed = {}
        if participants:
            entries = row['participants']
            if not isinstance(entries, list) or len(entries) > 18:
                self.issue('PAGE_TRACE_PARTICIPANTS_REQUIRED', path + '/participants',
                           'Declare at most 18 additional actors. Use the fixed caller and handler IDs directly.')
            else:
                selected, seen = [], set()
                originals = {actor['id'] for actor in entries if isinstance(actor, dict) and isinstance(actor.get('id'), str)}
                proposed = [identity.lower().replace('_', '-') for identity in originals
                            if re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]{0,31}', identity)]
                for index, actor in enumerate(entries):
                    place = f'{path}/participants/{index}'
                    fields = ('id', 'title', 'local') if isinstance(actor, dict) and 'local' in actor else ('id', 'title')
                    if not self.shape(actor, place, fields):
                        continue
                    if 'local' in actor and type(actor['local']) is not bool:
                        self.issue('PAGE_TRACE_PARTICIPANT_INVALID', place + '/local',
                                   'Use true only for a participant within the same source-evidenced system.')
                    identity = actor['id']
                    self.line(actor['title'], place + '/title', 120)
                    original = identity
                    if isinstance(identity, str) and re.fullmatch(r'[A-Za-z][A-Za-z0-9_-]{0,31}', identity):
                        canonical = identity.lower().replace('_', '-')
                        if canonical != identity and canonical not in FIXED_ACTORS and proposed.count(canonical) == 1:
                            renamed[identity] = canonical
                            identity = actor['id'] = canonical
                    if not isinstance(identity, str) or not re.fullmatch(r'[a-z][a-z0-9-]{0,31}', identity) or original in seen:
                        self.issue('PAGE_TRACE_PARTICIPANT_INVALID', place + '/id',
                                   'Use a distinct short actor ID with lowercase letters, digits, or hyphens.',
                                   received=safe_value(identity))
                        continue
                    seen.add(original)
                    if identity not in FIXED_ACTORS:
                        available.append(identity)
                        selected.append(actor)
                row['participants'] = selected
        if not isinstance(row['steps'], list) or not 0 < len(row['steps']) <= 48:
            self.issue('PAGE_TRACE_STEPS_REQUIRED', path + '/steps', 'Provide 1 to 48 ordered steps after the fixed start.')
            return
        kinds = ROUTE_KINDS if route_references else KINDS
        for index, step in enumerate(row['steps']):
            place = f'{path}/steps/{index}'
            if not self.shape(step, place, ('from', 'to', 'kind', 'label', 'condition')):
                continue
            self.line(step['label'], place + '/label', STEP_TEXT_LENGTH,
                      empty=route_references and step['kind'] == 'reference')
            if isinstance(step['condition'], str) and not step['condition'].strip():
                step['condition'] = None
            if step['condition'] is not None:
                self.line(step['condition'], place + '/condition', STEP_TEXT_LENGTH)
            for key in ('from', 'to'):
                if isinstance(step[key], str) and step[key] in renamed:
                    step[key] = renamed[step[key]]
                self.choices(step[key], place + '/' + key, available, 'PAGE_TRACE_ACTOR_UNKNOWN',
                             'Use a fixed actor ID or an additional actor declared in this trace.')
            valid = self.choices(step['kind'], place + '/kind', kinds, 'PAGE_TRACE_STEP_KIND_INVALID',
                                 'Use the step kind that describes the evidenced action.')
            if valid and step['kind'] == 'reference' and (step['from'] == 'caller' or step['from'] != step['to']):
                self.issue('PAGE_TRACE_STEP_DIRECTION_INVALID', place + '/to',
                           'Use the same operation ID for both actors of an operation reference.')
            if valid and step['kind'] not in ('note', 'reference') and step['from'] == step['to']:
                self.issue('PAGE_TRACE_STEP_DIRECTION_INVALID', place + '/to',
                           'Use the actual receiving actor. Use note for a local action on one actor.')

    def contract(self, row, path):
        if not self.shape(row, path, ('scope', 'name', 'fields', 'cites')):
            return
        self.line(row['name'], path + '/name', 160)
        self.choices(row['scope'], path + '/scope', ('LOCAL', 'EXTERNAL'), 'PAGE_CONTRACT_INVALID',
                     'Choose the scope of the contract.')
        self.cites(row, path)
        if not isinstance(row['fields'], list) or not 0 < len(row['fields']) <= 80:
            self.issue('PAGE_CONTRACT_FIELDS_REQUIRED', path + '/fields',
                       'Supply 1 to 80 evidenced named fields. For a shape with no named fields, move its name and '
                       'known shape to a cited claim and omit this table.')
            return
        for index, field in enumerate(row['fields']):
            place = f'{path}/fields/{index}'
            if self.shape(field, place, ('name', 'type', 'meaning')):
                for key in ('name', 'type', 'meaning'):
                    self.line(field[key], place + '/' + key, 512)

    def route(self, row, path):
        fields = ('title', 'summary', 'origin', 'participants', 'condition', 'outcome', 'result',
                  'terminal', 'boundary', 'recipient', 'steps', 'cites')
        if not self.shape(row, path, fields):
            return
        self.line(row['title'], path + '/title', 160)
        for key in ('summary', 'condition', 'outcome', 'recipient'):
            self.line(row[key], path + '/' + key, ROUTE_PROSE_LENGTH)
        self.choices(row['result'], path + '/result', ('normal', 'error', 'handoff', 'unknown'),
                     'PAGE_ROUTE_OUTCOME_INVALID', 'Use the evidenced outcome kind.')
        self.choices(row['boundary'], path + '/boundary', ('return', 'publish', 'handoff', 'stop', 'other', None),
                     'PAGE_ROUTE_BOUNDARY_INVALID', 'Use an evidenced boundary or null for an unknown end.')
        entries = self.slot.get('operationIds')
        if entries is not None:
            self.choices(row['origin'], path + '/origin', entries, 'PAGE_ROUTE_ENTRY_OUTSIDE_PACKET',
                         'Copy the canonical operation ID from context.operations.')
        members = [row['origin']]
        if isinstance(row['participants'], list):
            for index, participant in enumerate(row['participants']):
                place = f'{path}/participants/{index}'
                if self.shape(participant, place, ('entry', 'role')):
                    identity = participant['entry']
                    if entries is not None:
                        self.choices(identity, place + '/entry', entries, 'PAGE_ROUTE_ENTRY_OUTSIDE_PACKET',
                                     'Copy the canonical operation ID from context.operations.')
                    if identity in members:
                        self.issue('PAGE_ROUTE_PARTICIPANT_DUPLICATED', place + '/entry',
                                   'Declare each occurrence once. The origin already participates.')
                    members.append(identity)
                    self.line(participant['role'], place + '/role', 160)
        else:
            self.issue('PAGE_ROUTE_PARTICIPANTS_REQUIRED', path + '/participants', 'Supply an array of peer operation IDs.')
        if (row['terminal'] is None) != (row['boundary'] is None) or row['terminal'] is not None and row['terminal'] not in members:
            self.issue('PAGE_ROUTE_TERMINAL_INVALID', path + '/terminal',
                       'Use an operation in this route with an evidenced boundary, or set terminal and boundary to null.',
                       allowed=[safe_value(value) for value in members])
        self.trace(row, path, actors=('caller', *members), participants=False,
                   route_references=self.slot.get('transitionRoutes', False))
        if self.slot.get('transitionRoutes') and isinstance(row['steps'], list) and not any(
                isinstance(step, dict) and step.get('from') in members and step.get('to') in members
                and step['from'] != step['to'] and step.get('kind') in ('call', 'await', 'return', 'async')
                for step in row['steps']):
            self.issue('PAGE_ROUTE_TRANSITION_REQUIRED', path + '/steps',
                       'A route needs a retained transition between two documented operations. Local behavior belongs on its operation page.')
        self.cites(row, path)

    def start(self, row, path):
        fields = ('kind', 'title', 'initiator', 'method', 'path', 'startLabel', 'startKind', 'disposition',
                  'claims', 'participants', 'steps')
        if not self.shape(row, path, fields):
            return
        self.choices(row['kind'], path + '/kind', ('HTTP', 'KAFKA', 'SCHEDULED', 'OTHER'),
                     'PAGE_DISCOVERED_START_INVALID', 'Select the trigger kind.')
        self.choices(row['startKind'], path + '/startKind', ('call', 'async'),
                     'PAGE_DISCOVERED_START_INVALID', 'Use call for a direct start or async for an asynchronous handoff.')
        self.choices(row['disposition'], path + '/disposition', DISPOSITIONS,
                     'PAGE_DISCOVERED_START_INVALID', 'Record the evidence disposition.')
        for key in ('title', 'initiator', 'startLabel'):
            self.line(row[key], path + '/' + key, 240)
        for key, limit in (('method', 16), ('path', 240)):
            if row['kind'] == 'HTTP':
                self.line(row[key], path + '/' + key, limit)
            elif row[key] is not None:
                self.issue('PAGE_DISCOVERED_START_INVALID', path + '/' + key, 'Use null for a trigger outside HTTP.')
        self.claims(row['claims'], path + '/claims')
        self.trace(row, path)

    def page(self, value):
        kind = self.slot['kind']
        own = owned_fields(self.slot)
        scoped = self.slot.get('ownedContentOnly')
        if isinstance(value, dict):
            if 'title' not in own and ('title' not in value or isinstance(value['title'], str)):
                value['title'] = None
            for field in ARRAYS:
                if field not in own:
                    value.setdefault(field, [])
            if kind != 'DISCOVERY':
                value.setdefault('discovery', None)
            if scoped:
                value.setdefault('limitations', [])
        fields = (*COMMON, *ARRAYS, 'title', 'discovery')
        if not self.shape(value, '', fields):
            return
        self.choices(value['slot'], '/slot', (self.slot['key'],), 'PAGE_WRONG_SLOT_OR_REVIEW_MISSING',
                     'Copy the slot ID from the packet.')
        if value['reviewed'] is not True:
            self.issue('PAGE_WRONG_SLOT_OR_REVIEW_MISSING', '/reviewed', 'Check the content against the retained source and set reviewed to true.')
        self.choices(value['disposition'], '/disposition', DISPOSITIONS, 'PAGE_DISPOSITION_REQUIRED',
                     'Record the evidence disposition for the slot.')
        if 'title' in own:
            self.line(value['title'], '/title', HEADING_LENGTH)
        elif value['title'] is not None:
            self.issue('PAGE_TITLE_INVALID', '/title', 'The runner supplies this page title.')
        if scoped and value['limitations']:
            self.issue('PAGE_FIELD_OUTSIDE_SLOT', '/limitations',
                       'Leave general limitations to the operation or route that owns the evidence.')
        elif isinstance(value['limitations'], list):
            for index, label in enumerate(value['limitations']):
                self.line(label, f'/limitations/{index}', 512)
        else:
            self.issue('PAGE_LIST_REQUIRED', '/limitations', 'Supply an array of source limitations.')
        for field in ARRAYS:
            if not isinstance(value[field], list):
                self.issue('PAGE_LIST_REQUIRED', '/' + field, 'Supply an array.')
            elif field not in own and value[field]:
                self.issue('PAGE_FIELD_OUTSIDE_SLOT', '/' + field,
                           f'This {kind} slot has no {field} destination. Put any essential evidence into a supported field.')
        if kind == 'DISCOVERY':
            if not (value['discovery'] == 'NO_ADDITIONAL_STARTS' and value['starts'] == []
                    or value['discovery'] == 'ADDITIONAL_STARTS' and isinstance(value['starts'], list) and value['starts']):
                self.issue('PAGE_DISCOVERY_RESULT_REQUIRED', '/discovery',
                           'Use NO_ADDITIONAL_STARTS with an empty starts list, or ADDITIONAL_STARTS with cited starts.')
            if isinstance(value['starts'], list):
                for index, row in enumerate(value['starts']):
                    self.start(row, f'/starts/{index}')
        else:
            if value['discovery'] is not None:
                self.issue('PAGE_FIELD_OUTSIDE_SLOT', '/discovery', f'This {kind} slot has no discovery destination.')
            if 'claims' in own and not (value['claims'] == [] and (self.slot.get('transitionRoutes') or scoped)):
                self.claims(value['claims'], '/claims', supported_only=bool(scoped))
        if kind == 'OPERATION':
            self.trace(value, '')
            if not self.issues:
                CONDITION_PATHS['check'](self, value)
        for field, validate in (('contracts', self.contract), ('routes', self.route)):
            if field in own and isinstance(value[field], list):
                for index, row in enumerate(value[field]):
                    validate(row, f'/{field}/{index}')
        missing = [ref for ref in self.slot['returns'] if ref not in self.return_paths]
        if missing:
            self.issue('PAGE_LITERAL_RETURN_REVIEW_REQUIRED', '/claims',
                       'Bind each missing return ID to its cited claim. Put the predicate and literal in consecutive inline code spans.',
                       missing=missing)


def review_response(slot, response, need):
    result = deepcopy(response)
    review = Review(slot)
    review.page(result)
    need(not review.issues, 'PAGE_RESPONSE_INVALID', slot=slot['key'], formatOnly=not review.needs_source,
         issues=review.issues, omittedIssues=review.count - len(review.issues))
    return result
