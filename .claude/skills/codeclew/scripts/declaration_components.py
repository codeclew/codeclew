#!/usr/bin/env python3
"""Consume mandatory native declaration components; never derive their cells."""
import hashlib
from html import escape
import json
import posixpath
from pathlib import Path
import subprocess
from urllib.parse import quote

ENGINE = None
EXPECTED = None
CACHED = None
NATIVE_COMMANDS = 0


def active():
    return EXPECTED is not None


class ComponentError(Exception):
    def __init__(self, status, **details):
        self.status, self.details = status, details


def require(ok, status, **details):
    if not ok:
        raise ComponentError(status, **details)


def sha(raw):
    return 'sha256:'+hashlib.sha256(raw).hexdigest()


def digest(value):
    return sha(json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode())


def configure(clew=None, root=None, ticket=None, expected=None):
    global ENGINE, EXPECTED, CACHED, NATIVE_COMMANDS
    ENGINE = (str(clew), str(root)) if clew is not None and root is not None else None
    EXPECTED = None; CACHED = None; NATIVE_COMMANDS = 0
    require((ticket is None) == (expected is None), 'COMPONENT_TICKET_AND_EXPECTED_DIGEST_REQUIRED')
    if ticket is not None:
        require(ENGINE is not None, 'COMPONENT_NATIVE_ENGINE_REQUIRED')
        path = Path(ticket)
        require(path.is_file() and not path.is_symlink() and path.stat().st_size <= 256*1024, 'INVALID_COMPONENT_TICKET')
        value = json.loads(path.read_bytes())
        require(value.get('schema') == 'codeclew-declaration-components-ticket/1.0'
                and value.get('id') == expected and value.get('reference', {}).get('digest') == expected,
                'EXPECTED_COMPONENT_SET_CHANGED')
        EXPECTED = {'path': str(path.absolute()), 'raw': path.read_bytes(), 'ticket': value, 'expected': expected}


def packet():
    global CACHED, NATIVE_COMMANDS
    require(EXPECTED is not None, 'EXPECTED_COMPONENT_SET_REQUIRED')
    require(Path(EXPECTED['path']).read_bytes() == EXPECTED['raw'], 'COMPONENT_TICKET_CHANGED')
    if CACHED is None:
        command = [ENGINE[0], 'docs', 'evidence', 'read-declarations', '--root', ENGINE[1], '--input', EXPECTED['path']]
        NATIVE_COMMANDS += 1
        process = subprocess.run(command, stdin=subprocess.DEVNULL, capture_output=True)
        require(process.returncode == 0, 'NATIVE_COMPONENT_REPLAY_FAILED', exitCode=process.returncode)
        require(len(process.stdout) <= 16*1024*1024, 'COMPONENT_PACKET_TOO_LARGE')
        value = json.loads(process.stdout)
        require(value.get('schema') == 'codeclew-declaration-components-read/1.0'
                and value.get('ticket') == EXPECTED['ticket'], 'NATIVE_COMPONENT_TICKET_CHANGED')
        value = value['packet']; components = value.get('components')
        require(value.get('schema') == 'codeclew-declaration-components/1.0'
                and isinstance(components, list) and 1 <= len(components) <= 32
                and len({c['id'] for c in components}) == len(components), 'INVALID_NATIVE_COMPONENT_SET')
        for c in components:
            require(isinstance(c.get('html'), str) and sha(c['html'].encode()) == c.get('htmlDigest'), 'COMPONENT_HTML_DIGEST_CHANGED')
        CACHED = value
    return CACHED


def native_commands():
    return NATIVE_COMMANDS


def binding():
    return {'ticket': EXPECTED['ticket'], 'packetDigest': digest(packet())}


def components():
    return {c['id']: c for c in packet()['components']}


def layout(pages):
    """One canonical placement for every independently selected component."""
    found = {}; references = []
    for page in pages:
        if EXPECTED is None and (not isinstance(page, dict) or not isinstance(page.get('fragments'), list)):
            continue
        require(isinstance(page, dict) and isinstance(page.get('fragments'), list), 'INVALID_COMPONENT_PAGE')
        for fragment in page['fragments']:
            if not isinstance(fragment, dict):
                continue
            if 'component' in fragment:
                require(set(fragment) == {'component'} and isinstance(fragment['component'], str), 'INVALID_COMPONENT_PLACEMENT')
                identity = fragment['component']
                require(identity not in found, 'DUPLICATE_COMPONENT_PLACEMENT', component=identity)
                found[identity] = page.get('page')
            if 'componentRef' in fragment:
                require(set(fragment) == {'componentRef', 'text'} and isinstance(fragment['componentRef'], str)
                        and isinstance(fragment['text'], str) and 0 < len(fragment['text']) <= 256
                        and '\n' not in fragment['text'] and '\r' not in fragment['text'], 'INVALID_COMPONENT_REFERENCE')
                references.append(fragment['componentRef'])
    if EXPECTED is None:
        require(not found and not references, 'EXPECTED_COMPONENT_SET_REQUIRED')
        return {}
    expected = components()
    require(set(found) == set(expected), 'REQUIRED_COMPONENT_INVENTORY_MISMATCH', missing=sorted(set(expected)-set(found)), unexpected=sorted(set(found)-set(expected)))
    require(set(references) <= set(expected), 'UNKNOWN_COMPONENT_REFERENCE')
    return found


def fragment(value, canonical, page):
    if 'component' in value:
        return components()[value['component']]['html']
    identity = value['componentRef']
    relative = posixpath.relpath(canonical[identity], posixpath.dirname(page) or '.')
    target = quote(relative, safe='/')+'#clew-component-'+identity
    label=escape(value['text']).replace('[','&#91;').replace(']','&#93;')
    return '<p><a href="'+escape(target, quote=True)+'">'+label+'</a></p>\n'


def validate_markdown(placement, pages):
    if EXPECTED is None:
        require(placement is None, 'EXPECTED_COMPONENT_SET_REQUIRED')
        return
    require(isinstance(placement, dict) and set(placement) == {'binding', 'placements'}
            and placement['binding'] == binding(), 'COMPONENT_BINDING_CHANGED')
    expected = components(); placed = placement['placements']
    require(isinstance(placed, list) and len(placed) == len(expected), 'REQUIRED_COMPONENT_INVENTORY_MISMATCH')
    seen = set()
    for item in placed:
        require(isinstance(item, dict) and set(item) == {'id', 'page', 'startByte', 'endByte', 'htmlDigest'}, 'INVALID_COMPONENT_RANGE')
        identity = item['id']; require(identity in expected and identity not in seen, 'COMPONENT_IDENTITY_CHANGED'); seen.add(identity)
        require(item['page'] in pages, 'COMPONENT_PAGE_MISSING', component=identity)
        raw = pages[item['page']]; html = expected[identity]['html'].encode(); a,b = item['startByte'],item['endByte']
        require(type(a) is int and type(b) is int and 0 <= a < b <= len(raw)
                and item['htmlDigest'] == sha(html) and raw[a:b] == html,
                'COMPONENT_CONTENT_CHANGED', component=identity, page=item['page'])
        require(sum(body.count(html) for body in pages.values()) == 1, 'COMPONENT_CANONICAL_LOCATION_CHANGED', component=identity)


def validate_html(placement, pages):
    if EXPECTED is None:
        require(placement is None, 'EXPECTED_COMPONENT_SET_REQUIRED')
        return
    require(placement['binding'] == binding(), 'COMPONENT_BINDING_CHANGED')
    expected = components()
    for item in placement['placements']:
        identity = item['id']; target = str(Path(item['page']).with_suffix('.html'))
        html = expected[identity]['html'].encode()
        require(target in pages and pages[target].count(html) == 1
                and sum(raw.count(html) for raw in pages.values()) == 1,
                'EXPORTED_COMPONENT_CHANGED', component=identity, page=target)


def sidecar(placement):
    return {'placement': placement, 'packet': packet()}
