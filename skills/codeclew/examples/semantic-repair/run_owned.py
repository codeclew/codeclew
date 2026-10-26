#!/usr/bin/env python3
"""Synthetic transport exercise. Scripted reviews are NOT real model acceptance."""
import argparse
import copy
import json
from pathlib import Path
import runpy
import subprocess
import sys

SKILL = Path(__file__).resolve().parents[2]
H = runpy.run_path(str(SKILL / 'scripts/semantic_handoff.py'))
T = H['T']


def write(root, name, value):
    path = root / name
    path.parent.mkdir(parents=True, exist_ok=True)
    T['save'](path, value)
    return path


def page(rows, **changes):
    return {'schema': 'codeclew-documentation-work-page/1.0', 'work': 'owned-work', 'snapshot': 'sha256:owned-snapshot/123',
            'subject': 'service:sample', 'contextDigest': 'owned-context', 'inputDigest': 'owned-input',
            'membershipDigest': 'owned-members', 'receiptDigest': 'owned-read',
            'authority': 'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED',
            'influenceCoverage': 'RECORDED_READS_ONLY_EXECUTION_NOT_ATTESTED',
            'items': rows, 'total': len(rows), 'omitted': [], 'nextCursor': None, **changes}


def source(reference, name, text):
    return {'kind': 'SOURCE', 'id': name, 'reference': reference, 'record': {
        'id': name, 'service': 'sample', 'file': 'src/' + name + '.kt', 'startLine': 1, 'endLine': len(text.splitlines()),
        'revision': 'owned-revision', 'authority': 'EXACT_SNAPSHOT_TEXT', 'text': text,
        'textDigest': H['sha'](text.encode()), 'evidenceDigest': 'sha256:owned-evidence'}}


def native_page(identity='owned-proposal', status='READY_WITH_LIMITATIONS', diagnostics=()):
    return {'schema': 'codeclew-docs-page/1.0', 'reportSchema': 'codeclew-documentation-proposal-result/1.0',
            'proposal': identity, 'work': 'owned-work', 'status': status, 'meaningReview': 'UNASSESSED',
            'readDigest': 'owned-reads', 'narrativeDigest': None, 'items': list(diagnostics), 'total': len(diagnostics),
            'omitted': [], 'nextCursor': None}


def archive(root, candidate, attempt='native-before', status='READY_WITH_LIMITATIONS'):
    p = write(root, attempt + '/input.original.json', candidate)
    n = native_page(status=status)
    write(root, attempt + '/submit.command.json', {'command': ['owned-fake-clew', 'docs', 'proposal', 'submit', '--root', 'owned-docs', '--work', 'owned-work', '--input', str(p)], 'exitCode': 0})
    write(root, attempt + '/submit.stdout', n)
    directory = root / attempt / 'submit-collection'
    directory.mkdir()
    record = {'stdout': 'page-0001.stdout', 'stderr': 'page-0001.stderr', 'cursor': None, 'exitCode': 0}
    T['save'](directory / record['stdout'], n)
    T['save'](directory / 'items.json', n['items'])
    T['save'](directory / 'manifest.json', {'schema': 'codeclew-proposal-collection/1.0', 'status': 'COMPLETE',
        'identity': T['page_identity'](n, n['proposal']), 'terminalCursorObserved': True, 'nextCursor': None,
        'artifacts': {'pages': [record]}})


def setup(root, case):
    # These source strings and expectations are owned synthetic software fixtures.
    main = source('s-main', 'Handler', '''fun handle(request: Request): Reply {
    val selected = collector.collect(request, rates = true)
    val response = Reply.from(selected)
    response.audit = auditId(request)
    return response
}
''')
    if case == 'polarity':
        main = source('s-main', 'Handler', '''fun handle(amount: Int): String {
    if (amount < 0) return "negative"
    return "nonnegative"
}
''')
    candidate = source('s-candidate', 'Collector', '''fun collect(request: Request, rates: Boolean): Result {
    if (SAVE_ENABLED && rates) try { history.save(request) } catch (e: Exception) { log(e) }
    if (LOOKUP_ENABLED && !rates) history.read(request)
    return vehicleSearch.search(request)
}
''')
    literal = source('s-literal', 'Constants', 'const val AUTHORITY = "worker"\n')
    navigation = {'kind': 'DEPENDENCY', 'id': 'symbol-collector', 'reference': 'd-candidate', 'sourceReferences': ['s-candidate'],
        'record': {'id': 'symbol-collector', 'kind': 'SYMBOL', 'symbol': 'Collector.collect', 'sourceIds': ['Collector'], 'normalized': {'authority': 'SYNTAX'}}}
    write(root, 'work/initial.stdout', page([main, navigation, {**navigation, 'id': 'symbol-literal', 'reference': 'd-literal',
        'sourceReferences': ['s-literal'], 'record': {**navigation['record'], 'id': 'symbol-literal', 'symbol': 'Constants.AUTHORITY', 'sourceIds': ['Constants']}}]))
    write(root, 'reviewer-read/source.stdout', page([candidate, literal]))
    claim = lambda text, refs: {'text': text, 'evidence': refs}
    reply = {'kind': 'note', 'meaning': claim('Construct Reply from the selected result.', ['s-main'])}
    audit = {'kind': 'note', 'meaning': claim('Assign audit after constructing Reply.', ['s-main'])}
    operation = {'entrypoint': 'handle', 'title': 'Owned handler', 'summary': claim('Collector effects are unavailable.', ['s-main']),
                 'steps': [reply, audit], 'explanation': [], 'contracts': []}
    base = {'schema': 'codeclew-documentation-proposal/1.0', 'operations': [operation,
        {'entrypoint': 'unrelated', 'title': 'Untouched sibling', 'summary': claim('Preserve this exact sibling.', ['s-main']), 'steps': []}],
        'uncertainties': ['Runtime collector selection, ordering among candidates and durable save success are unproven.']}
    if case == 'omission':
        path, refs = '/operations/0/summary', ['s-main', 's-candidate']
        replacement = claim('The available candidate attempts history save when SAVE_ENABLED and rates, catches save failure, excludes history read for rates, and performs vehicle search. Runtime candidate dispatch and durable success remain unknown.', refs)
        reason = 'The available collector body has guarded save/catch and search effects omitted by the account.'
    elif case == 'order':
        path, refs = '/operations/0/steps', ['s-main']
        operation['steps'] = [audit, reply]
        replacement = [reply, audit]
        reason = 'Reply.from is called before response.audit assignment in the actual containing source.'
    elif case == 'polarity':
        negative = {'kind': 'return', 'from': 'sample', 'to': 'caller',
                    'meaning': claim('Return the negative result.', ['s-main'])}
        nonnegative = {'kind': 'return', 'from': 'sample', 'to': 'caller',
                       'meaning': claim('Return the nonnegative result.', ['s-main'])}
        group = {'kind': 'alt',
                 'meaning': claim('When amount is nonnegative return nonnegative; otherwise return negative.', ['s-main']),
                 'children': [negative], 'otherwise': [nonnegative]}
        operation['summary'] = claim('Choose the result from the sign of amount.', ['s-main'])
        operation['steps'] = [group]
        base['uncertainties'] = ['Runtime invocation is not established.']
        path, refs = '/operations/0/steps/0', ['s-main']
        replacement = copy.deepcopy(group)
        replacement['meaning'] = claim('Amount is negative.', refs)
        reason = 'The source-true negative path is the first child, but the header starts with the nonnegative path. The two child returns already match source; repair the predicate without swapping them.'
    elif case == 'literal':
        path, refs = '/operations/0/summary', ['s-main', 's-literal']
        operation['summary']['text'] = 'AUTHORITY is unknown in the selected evidence.'
        replacement = claim('AUTHORITY is the source literal worker; deployment use remains unproven.', refs)
        reason = 'The available declaration supplies the literal; runtime use is a separate unknown.'
        base['uncertainties'].insert(0, 'AUTHORITY is unavailable in the delivered source.')
    else:
        raise ValueError('Unknown owned exercise case')
    archive(root, base)
    request = {'schema': 'codeclew-semantic-handoff-request/1.0', 'work': 'owned-work', 'workPage': 'work/initial.stdout',
        'nativeAttempt': 'native-before', 'scope': 'One owned handler and available collector/literal candidates; no runtime dispatch or completion proof.',
        'pages': ['work/initial.stdout'], 'collections': [], 'allowedPaths': [path] + (['/uncertainties/0'] if case == 'literal' else []), 'maxRepairs': 1,
        'author': {'id': 'owned-author', 'model': 'gpt-6-luna', 'reasoning': 'max'},
        'reviewer': {'id': 'owned-independent-reviewer', 'model': 'gpt-6-luna', 'reasoning': 'max'}}
    write(root, 'request.json', request)
    return {'base': base, 'path': path, 'replacement': replacement, 'refs': refs, 'reason': reason, 'sources': [main, candidate, literal],
            'additionalTargets': [{'path': '/uncertainties/0', 'new': 'AUTHORITY is worker in the retained declaration; runtime use remains unproven.'}] if case == 'literal' else []}


def finding(root, fixture, context='handoff/review-context.json', affected=False, verdict='REJECT'):
    raw = (root / context).read_bytes(); value = json.loads(raw)
    result = {'schema': 'codeclew-semantic-handoff-review/1.0', 'binding': value['binding'], 'contextDigest': H['sha'](raw),
        'reviewScope': value['reviewScope'], 'reviewer': value['reviewer'], 'verdict': verdict,
        'inspected': [{'reference': row['reference'], 'textDigest': row['record']['textDigest']} for row in fixture['sources'] if row['reference'] in fixture['refs']],
        'issues': [] if verdict == 'APPROVE' else [{'severity': 'ERROR', 'claim': None, 'path': fixture['path'], 'reason': fixture['reason'], 'evidence': fixture['refs'], 'neededReads': []}],
        'limitations': ['Scripted synthetic reviewer, not a real model or native meaning attestation.']}
    if affected:
        result['assessedPaths'] = [target['path'] for target in value['targets']]
    else:
        if verdict != 'APPROVE':
            result['issues'].extend({**result['issues'][0], 'path': target['path']} for target in fixture.get('additionalTargets', []))
        result['assessedOperations'] = value['assessOperations']
        result['acquiredPages'] = [{'path': 'reviewer-read/source.stdout', 'digest': H['sha']((root / 'reviewer-read/source.stdout').read_bytes())}]
    return result


def fake_launcher(root):
    launcher = root / 'fake-clew'
    launcher.write_text(f'#!{sys.executable}\n' + '''import json,sys
from pathlib import Path
args=sys.argv[1:]
root=Path(__file__).parent
with (root/'native-calls.jsonl').open('a') as out:out.write(json.dumps(args)+'\\n')
cfg=json.loads((root/'native-response.json').read_text())
if args[:3]==['docs','proposal','submit']:
    assert json.loads(Path(args[args.index('--input')+1]).read_text())['schema']=='codeclew-documentation-proposal/1.0'
else: assert args[:3]==['docs','proposal','show']
print(json.dumps(cfg))
''')
    launcher.chmod(0o700)
    write(root, 'native-response.json', native_page('owned-repaired'))
    return launcher


def invoke(root, *args):
    process = subprocess.run([sys.executable, '-I', '-S', str(SKILL / 'scripts/semantic_handoff.py'), *map(str, args), '--input-root', str(root)], capture_output=True, text=True)
    if not process.stdout:
        raise RuntimeError(process.stderr)
    return process.returncode, json.loads(process.stdout)


def run(root, case):
    fixture = setup(root, case)
    code, prepared = invoke(root, 'prepare', '--input', 'request.json', '--output', 'handoff')
    assert code == 0, prepared
    write(root, 'first-review.json', finding(root, fixture))
    code, repair = invoke(root, 'repair-context', '--input', 'handoff', '--review', 'first-review.json', '--output', 'repair')
    assert code == 0, repair
    context_raw = (root / 'repair/repair-context.json').read_bytes();context = json.loads(context_raw)
    replacements = {fixture['path']: fixture['replacement'], **{t['path']: t['new'] for t in fixture.get('additionalTargets', [])}}
    patch = {'schema': 'codeclew-semantic-replacements/1.0', 'binding': context['binding'], 'contextDigest': H['sha'](context_raw),
        'author': context['author'], 'replacements': [{'path': target['path'], 'old': target['old'], 'new': replacements[target['path']]} for target in context['targets']]}
    write(root, 'patch.json', patch)
    launcher = fake_launcher(root)
    code, applied = invoke(root, 'apply', '--input', 'repair', '--patch', 'patch.json', '--output', 'applied', '--launcher', launcher, '--root', root / 'docs')
    assert code == 0, applied
    write(root, 'affected-review.json', finding(root, fixture, 'applied/affected-review-context.json', True, 'APPROVE'))
    code, final = invoke(root, 'finish', '--input', 'applied', '--review', 'affected-review.json', '--output', 'final')
    assert code == 0, final
    final['exerciseAuthority'] = 'SCRIPTED_OWNED_TRANSPORT_ONLY_NO_MODEL_CALLS'
    return final


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, required=True, help='New caller-owned output directory; never an installed skill folder')
    args = parser.parse_args()
    target = args.output.absolute()
    if target.is_relative_to(SKILL):
        parser.error('output must stay outside the installed skill')
    target.mkdir(parents=True, exist_ok=False)
    for case in ('omission', 'order', 'literal', 'polarity'):
        directory = target / case;directory.mkdir()
        print(json.dumps({'case': case, **run(directory, case)}))
