"""Engine-reloaded registered native bytes, distinct from Work SOURCE proof.

The caller supplies the trusted launcher/root on every consuming command.
Saved JSON/receipts are never engine attestation and never select an executable.
"""
from pathlib import Path
import hashlib
import json
import subprocess

# Supplied by author_citations; share its exception and canonical digest.
SCHEMA = 'codeclew-author-native-citations/1.0'
AUTHORITY = 'REGISTERED_NATIVE_BYTES_ONLY_NO_SEMANTIC_ANALYSIS'
_RELOADS = {}


def require(condition, status):
    if not condition:
        raise Stop(status, exit_code=2)


def read(path):
    path = Path(path)
    require(not path.is_symlink() and path.is_file() and path.stat().st_size <= 4 * 1024 * 1024,
            'INVALID_NATIVE_CITATION_ARTIFACT')
    raw = path.read_bytes()
    def unique(pairs):
        require(len(pairs) == len({key for key, _ in pairs}), 'DUPLICATE_NATIVE_JSON_KEY')
        return dict(pairs)
    return raw, json.loads(raw, object_pairs_hook=unique)


def native_records(packet):
    require(isinstance(packet, dict) and isinstance(packet.get('items'), list)
            and all(isinstance(i, dict) for i in packet['items']), 'INVALID_NATIVE_PACKET')
    rows = [i for i in packet['items'] if i.get('kind') == 'SOURCE_NATIVE']
    require(rows and len(rows) <= 32
            and all(isinstance(i.get('id'), str) and isinstance(i.get('record'), dict) for i in rows)
            and len({i['id'] for i in rows}) == len(rows),
            'INCOMPLETE_OR_DUPLICATE_NATIVE_RECORDS')
    return {i['id']: i for i in rows}


def reload(path, engine):
    require(engine is not None, 'NATIVE_ENGINE_VALIDATION_REQUIRED')
    clew, root = map(Path, engine)
    require(clew.is_absolute() and clew.is_file() and root.is_absolute() and root.is_dir(),
            'INVALID_TRUSTED_NATIVE_ENGINE')
    raw, supplied = read(path)
    require(isinstance(supplied, dict), 'INVALID_NATIVE_PACKET')
    key = (str(clew), str(root), hashlib.sha256(raw).hexdigest())
    if key in _RELOADS:
        return _RELOADS[key]
    ticket = supplied.get('evidenceSet', {})
    require(isinstance(ticket, dict) and ticket.get('schema') == 'codeclew-prepared-source-evidence/1.0'
            and supplied.get('snapshot') == ticket.get('snapshot')
            and isinstance(ticket.get('service'), str) and ticket['service'], 'INVALID_NATIVE_EVIDENCE_SET')
    receipts = []
    def invoke(args):
        argv = [str(clew), 'docs', *args, '--root', str(root)]
        completed = subprocess.run(argv, capture_output=True)
        receipts.append({'argv': argv, 'exitCode': completed.returncode,
                         'stdout': completed.stdout.decode('utf-8', errors='replace'),
                         'stderr': completed.stderr.decode('utf-8', errors='replace')})
        require(completed.returncode == 0, 'NATIVE_ENGINE_RELOAD_FAILED')
        return json.loads(completed.stdout)
    # Same immutable snapshot: engine reuses registered native objects, not Git.
    actual = invoke(['evidence', 'compare-source', '--input', str(Path(path).absolute()),
                     '--snapshot', ticket['snapshot'], '--include-context'])
    require(encode(actual.get('evidenceSet')) == encode(ticket) and actual.get('snapshot') == ticket['snapshot']
            and actual.get('comparison', {}).get('status') in ('UNCHANGED', 'UNKNOWN')
            and actual['comparison'].get('integrity') == 'BOTH_IMMUTABLE_SNAPSHOT_CLOSURES_ADMITTED'
            and actual['comparison'].get('baselineSnapshot') == ticket['snapshot']
            and actual['comparison'].get('targetSnapshot') == ticket['snapshot']
            and actual['comparison'].get('baselineBinding') == actual['comparison'].get('targetBinding'),
            'NATIVE_EVIDENCE_SET_MISMATCH')
    rows = native_records(actual)
    require(encode(native_records(supplied)) == encode(rows) and len(rows) == ticket.get('nativeFiles'),
            'NATIVE_ENGINE_RECORD_MISMATCH')
    require(not actual.get('registeredInputLimits', {}).get('native'), 'INCOMPLETE_NATIVE_INPUTS')
    binding = actual['comparison']['targetBinding']
    config = invoke(['service', 'show', '--id', ticket['service']])['record']
    analysis_config = {k: v for k, v in config.items() if k not in ('targetRef', 'sourceLinkTemplate', 'title')}
    require(digest(analysis_config) == binding.get('analysisConfigurationDigest'),
            'NATIVE_REPOSITORY_CONFIGURATION_MISMATCH')
    require(config.get('id') == ticket['service'] and config.get('repositoryId') and config.get('repository'),
            'NATIVE_REPOSITORY_BINDING_MISSING')
    for item in rows.values():
        record = item['record']; occurrence = record.get('occurrence', {})
        # One captured binding service per manifest. Separate packets support
        # other services; do not invent a cross-service repository identity.
        require(record.get('id') == item['id'] and record.get('service') == ticket['service']
                and record.get('bindingService') == ticket['service']
                and record.get('revision') == binding.get('revision')
                and record.get('authority') == AUTHORITY
                and record.get('textDigest') == 'sha256:' + hashlib.sha256(record['text'].encode('utf-8')).hexdigest()
                and occurrence.get('snapshot') == record['revision']
                and occurrence.get('startByte') == 0
                and occurrence.get('endByte') == len(record['text'].encode('utf-8')),
                'INVALID_NATIVE_RECORD_BINDING')
    # A compiler-selected file can have UNKNOWN whole-file coverage while the
    # separately registered native bytes are exact. Preserve that distinction.
    result = (supplied, rows, config, {**binding, 'reloadComparison': actual['comparison']}, receipts)
    _RELOADS[key] = result
    return result


def manifest_value(raw, packet, selection):
    value = {'schema': SCHEMA, 'prepared': 'prepared-native.json',
             'preparedDigest': 'sha256:' + hashlib.sha256(raw).hexdigest(),
             'evidenceSet': packet['evidenceSet'], 'selection': selection}
    return {**value, 'id': digest(value)}


def load(path, engine):
    path = Path(path)
    _, manifest = read(path)
    require(isinstance(manifest, dict) and manifest.get('schema') == SCHEMA and manifest.get('prepared') == 'prepared-native.json',
            'INVALID_NATIVE_CITATION_MANIFEST')
    prepared = path.parent / manifest['prepared']
    raw, packet = read(prepared)
    selection = manifest.get('selection')
    require(isinstance(selection, list) and selection and all(isinstance(i, str) for i in selection)
            and len(set(selection)) == len(selection)
            and manifest == manifest_value(raw, packet, selection), 'NATIVE_CITATION_MANIFEST_MISMATCH')
    _, rows, config, binding, _ = reload(prepared, engine)
    require(set(selection) <= rows.keys(), 'UNKNOWN_NATIVE_CITATION_SELECTION')
    outcomes, texts, entries = [], {}, {}
    tag = 'm' + manifest['id'].split(':')[1][:12]
    for index, selected in enumerate(selection, 1):
        record = rows[selected]['record']; source = {k: v for k, v in record.items() if k != 'text'}
        text_key = 'sha256:' + hashlib.sha256(record['text'].encode('utf-8')).hexdigest()
        texts[text_key] = record['text']
        provenance = {'evidenceSet': packet['evidenceSet'], 'repositoryId': config['repositoryId'],
                      'repository': config['repository'], 'binding': binding, 'kind': 'SOURCE_NATIVE'}
        outcome = {'source': source, 'sourceRecordDigest': digest(record), 'textKey': text_key,
                   'nativeBinding': provenance}
        outcomes.append(outcome)
        entries[tag + ':e' + str(index)] = {'manifestId': manifest['id'], 'source': source,
                                           'sourceRecordDigest': digest(record), 'nativeBinding': provenance}
    projected = {'kind': 'NATIVE', 'outcomes': outcomes, 'texts': texts,
                 'scope': {'service': packet['evidenceSet']['service'], 'snapshot': packet['snapshot']}}
    return manifest, projected, {}, entries


def project(packet, manifest, path, include_texts=True):
    tag = 'm' + manifest['id'].split(':')[1][:12]
    records, texts = [], {}
    for index, outcome in enumerate(packet['outcomes'], 1):
        source = outcome['source']; key = 't' + str(index)
        records.append({'cite': tag + ':e' + str(index),
                        'location': f'{source["service"]}:{source["file"]}:{source["startLine"]}-{source["endLine"]}',
                        'role': 'registered native file', 'authority': source['authority'],
                        'limits': 'l1', 'text': key})
        if include_texts:
            texts[key] = packet['texts'][outcome['textKey']]
    return {'schema': 'codeclew-author-read/1.0', 'status': 'SELECTED_NATIVE_SOURCES_READY',
            'manifest': {'path': str(path), 'id': manifest['id']}, 'records': records, 'texts': texts,
            'limitations': {'l1': ['Registered file bytes only; compiler semantics, execution and runtime success unproved.']},
            'coverageWarnings': [], 'failures': [], 'discoveryQueryComplete': False,
            'meaning': 'Explicit registered native files; original byte bindings retained. No Work handles or semantic/runtime approval.'}


def register(input_path, output, engine, selection=None):
    raw, _ = read(input_path)
    packet, rows, _, _, receipts = reload(input_path, engine)
    selection = list(rows) if selection is None else selection
    require(selection and len(set(selection)) == len(selection) and set(selection) <= rows.keys(),
            'UNKNOWN_NATIVE_CITATION_SELECTION')
    output = Path(output)
    require(not output.exists() and not output.is_symlink(), 'NATIVE_CITATION_OUTPUT_EXISTS')
    output.mkdir(parents=True)
    (output / 'prepared-native.json').write_bytes(raw)
    manifest = manifest_value(raw, packet, selection)
    path = output / 'citations.json'
    path.write_text(encode(manifest), encoding='utf-8')
    # Diagnostic only: consumers always reload using caller-supplied authority.
    (output / 'engine-reload.receipts.json').write_text(encode(receipts), encoding='utf-8')
    _, projected, _, _ = load(path, engine)
    return project(projected, manifest, path, include_texts=False)


def mapped_source(entry):
    binding = entry['nativeBinding']
    return {**entry['source'], 'evidenceKind': 'SOURCE_NATIVE',
            'originalUrl': entry['source'].get('url'),
            'sourceRecordDigest': entry['sourceRecordDigest'],
            'evidenceSet': binding['evidenceSet'], 'engineBinding': binding['binding'],
            'repository': binding['repository'], 'repositoryId': binding['repositoryId'],
            'bindingRepository': binding['repository'], 'linkMapping': 'ENGINE_RETAINED_NATIVE_LINK'}
