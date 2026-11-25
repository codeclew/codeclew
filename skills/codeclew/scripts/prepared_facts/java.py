"""Project retained Java syntax and local declaration witnesses for docs-run."""
from collections import defaultdict
from pathlib import Path
import runpy
import subprocess


# Original accepted projection: both syntax and compilation-unit engines.
IMPLEMENTATION_REVISION = 'a8f16ddc5983178324910ff685d425f80ab2797b'
DIRECTORY = Path(__file__).resolve().parent
JAVA = runpy.run_path(str(DIRECTORY / 'clew_prepared_java.py'))
CHAIN = runpy.run_path(str(DIRECTORY / 'clew_java_chain.py'))
SOURCE = runpy.run_path(str(DIRECTORY / 'clew_reader_java_sources.py'))
MAX_DISCOVERY_METHODS = 24


def snapshot_for(process, context):
    return SOURCE['Snapshot'](
        process, context['_sourceOccurrences'], context.get('_repository'),
        revision=context.get('_revision'),
        work_files=context.get('_nativeFiles', {}), citation_sources=context.get('_citationSources', {}),
        deadline=context.get('_deadline'),
    )


def discover(process, context):
    """Find declaration-local witnesses inside the selected Work file scope."""
    file = context.get('file')
    if not SOURCE['java_path'](file):
        return [], {}
    selected = [ref for ref, row in context['_sourceOccurrences'].items() if row['file'] == file]
    root = {**process, 'root': file, 'facts': [{'kind': 'BODY', 'declaration': file, 'sources': selected}]}
    snapshot = snapshot_for(root, context)
    citations = [site['id'] for site in context['sourceRanges']
                 if any(ref in snapshot.retained for ref in site['references'])]
    if not citations:
        return [], {}
    if not snapshot.read(file):
        return [source_boundary(citations[0])], {}
    try:
        with CHAIN['open_units'](snapshot) as units:
            if units is None:
                return [parser_boundary(citations[0])], {}
            owners = units.read(file)
            if not owners:
                return [parser_boundary(citations[0])], {}
            declarations = []
            limited = False
            for owner in owners:
                for method in owner.methods():
                    if not method[8] or method[3] == '<init>':
                        continue
                    if len(declarations) >= MAX_DISCOVERY_METHODS:
                        limited = True
                        break
                    if ref := snapshot.cite(file, method[1], method[2], declaration=True):
                        declarations.append((owner, method, ref))
                if limited:
                    break
            ranges = {ref: JAVA['source_range'](ref, snapshot.ranges[ref], checksum=True)
                      for _, _, ref in declarations}
            parsed, failure = JAVA['parse_batch'](ranges, timeout=snapshot.remaining(8)) if ranges else ({}, None)
            fields, predicates = JAVA['attribute'](snapshot, ranges, parsed)
            groups = defaultdict(list)
            for owner, method, ref in declarations:
                if snapshot.remaining(8) <= 0:
                    limited = True
                    break
                source = snapshot.ranges[ref]
                declaration = f'{owner.qualified}.{method[3]}'
                synthetic = {**root, 'sources': {ref: source},
                             'facts': [{'kind': 'BODY', 'declaration': file, 'sources': [ref]}]}
                source_context = {**context, 'sourceRanges': [{'id': ref, 'references': [ref]}],
                                  '_sourceOccurrences': {ref: source}}
                if ref in parsed:
                    rows = JAVA['derive'](synthetic, source_context, snapshot=snapshot,
                                          prepared=(parsed, fields, predicates), declaration_only=True)
                else:
                    rows = []
                rows.extend(CHAIN['project_frame'](units, owner, method, ref, owner.site(method), {}, {}))
                for row in rows:
                    subject = f'within the cited source declaration `{declaration}`'
                    condition = row.get('condition')
                    row['condition'] = f'{subject} and {condition}' if condition else subject
                    groups[ref].append(row)
            rows = list(JAVA['interleave'](groups.values()))
            if failure:
                rows.append(parser_boundary(citations[0], failure))
            if limited:
                rows.append(JAVA['boundary']('PROJECTION_LIMIT', 'Java declaration projection reached its preparation limit.',
                                              [citations[0]]))
    except (OSError, ValueError, RecursionError, subprocess.SubprocessError):
        return [parser_boundary(citations[0])], {}
    if snapshot.missing_authority:
        rows.append(source_boundary(citations[0]))
    return rows, cited_ranges(rows, snapshot)


def source_boundary(handle):
    return JAVA['boundary']('SNAPSHOT_SOURCE_UNAVAILABLE',
                            'Some Java witnesses require source ranges outside the retained Work citations.', [handle])


def parser_boundary(handle, status='PARSER_UNAVAILABLE'):
    return JAVA['boundary'](status, 'The retained Java file could not be fully projected with the local JDK syntax parser.', [handle])


def cited_ranges(rows, snapshot):
    cited = {ref for row in rows for ref in row['sources']}
    return {ref: source for ref, source in snapshot.ranges.items() if ref in cited}


def project(process, context, *, node):
    if not process.get('root'):
        return discover(process, context)
    snapshot = snapshot_for(process, context)
    sealed = {**context, '_sourceOccurrences': snapshot.retained}
    body = JAVA['method_bodies'](process, JAVA['author_ranges'](process, sealed)).get(process['root'])
    if body is None:
        return [], {}
    rows = JAVA['derive'](process, sealed, snapshot=snapshot)
    rows.extend(CHAIN['derive'](process, sealed, snapshot))
    if snapshot.missing_authority or not snapshot.read(body.file):
        rows.append(source_boundary(body.identifier))
    return rows, cited_ranges(rows, snapshot)
