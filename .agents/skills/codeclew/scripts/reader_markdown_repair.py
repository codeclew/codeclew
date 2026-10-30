#!/usr/bin/env python3
"""Addressed external Markdown repair; existing citations and caller renderer.

Only explicit whole paragraph replacements. No semantic inference, models, source
acquisition or publication. Rendering must produce stable standalone HTML/main;
visibility checks cover this bounded structure, not browser-computed CSS.
"""
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import subprocess
from urllib.parse import urlsplit

C = globals()["CITATIONS"]
Stop, digest, encode = (C[k] for k in ("Stop", "digest", "encode"))


def sha(raw):return "sha256:" + hashlib.sha256(raw).hexdigest()


def require(condition, status, **details):
    if not condition:raise Stop(status, exit_code=2, **details)


def renderer_command(path):
    argv = json.loads(Path(path).read_bytes())
    require(isinstance(argv, list) and bool(argv) and all(isinstance(a, str) for a in argv)
            and argv.count('{input}') == 1 and argv.count('{output}') == 1, "INVALID_RENDERER_COMMAND")
    return argv


def run_renderer(argv, source, destination, receipts, label):
    """Caller-trusted argv only; preserve stdout, stderr and actual exit status."""
    command = [str(source) if a == '{input}' else str(destination) if a == '{output}' else a for a in argv]
    try:
        result = subprocess.run(command, capture_output=True)
    except OSError as error:
        (receipts / (label + '.render.command.json')).write_text(encode({'command': command, 'exitCode': None, 'error': str(error)}))
        raise Stop("RENDERER_UNAVAILABLE", exit_code=2, stage=label, message=str(error)) from error
    (receipts / (label + '.render.stdout')).write_bytes(result.stdout)
    (receipts / (label + '.render.stderr')).write_bytes(result.stderr)
    receipt = {'command': command, 'exitCode': result.returncode}
    (receipts / (label + '.render.command.json')).write_text(encode(receipt))
    require(result.returncode == 0, "RENDERER_FAILED", stage=label, exitCode=result.returncode)
    return receipt


def authored(claim):
    return {k:claim[k] for k in ("id", "text", "occurrence", "evidence") if k in claim}


def whole_paragraph(text):
    require(isinstance(text,str) and text.strip() == text and bool(text), "TARGET_MUST_BE_WHOLE_PARAGRAPH")
    require(not re.search(r'\r?\n[ \t]*\r?\n|[<>]|(^|\n)(?:\s|#{1,6} |[-+>] |\d+[.)] )',text),
            "TARGET_MUST_BE_WHOLE_PARAGRAPH")


def formatted(target):
    return any(re.search(r'[`*_\[\]\\|]',target[key]) for key in ('old','new'))


def addressed(base, previous, patch):
    require(isinstance(previous,dict), "INVALID_PREVIOUS_MAP")
    require(isinstance(patch,dict) and set(patch)=={"schema","baseMarkdownDigest","baseMapDigest","replacements"}
            and patch["schema"]=="codeclew-reader-repair/1.0", "INVALID_READER_REPAIR")
    require(patch["baseMarkdownDigest"]==sha(base.encode())==previous.get("markdownDigest")
            and patch["baseMapDigest"]==previous.get("mapDigest"), "STALE_REPAIR_BASE")
    require(previous.get("schema")=="codeclew-authored-sources/1.0"
            and previous.get("mapDigest")==digest({k:v for k,v in previous.items() if k!="mapDigest"}), "INVALID_PREVIOUS_MAP")
    # Raw HTML can create hidden ancestors/CSS outside the addressed paragraph.
    # Refuse it conservatively, even when a particular renderer would escape it.
    require(not re.search(r'<[!/?A-Za-z]',base), "UNSUPPORTED_MARKDOWN_RAW_HTML")
    changes=patch["replacements"]
    require(isinstance(changes,list) and bool(changes), "NO_ADDRESSED_REPLACEMENT")
    previous_claims={c["id"]:c for c in previous["claims"]}
    require(len(previous_claims)==len(previous["claims"]), "AMBIGUOUS_PREVIOUS_CLAIM")
    claims=[authored(c) for c in previous["claims"]];by_id={c["id"]:c for c in claims}
    targets=[];seen=set();rebind=[]
    for change in changes:
        require(isinstance(change,dict) and not set(change)-{"claim","old","new","evidence","rebind"}
                and {"claim","old","new"}<=set(change), "INVALID_ADDRESSED_REPLACEMENT")
        identity=change["claim"]
        require(isinstance(identity,str) and identity in by_id and identity not in seen, "UNKNOWN_OR_DUPLICATE_REPAIR_TARGET")
        seen.add(identity);old,new=change["old"],change["new"]
        whole_paragraph(old);whole_paragraph(new)
        require(old!=new and old not in new, "OLD_TARGET_LEFT_UNCHANGED")
        require(previous_claims[identity]["text"]==old, "PREVIOUS_CLAIM_TARGET_MISMATCH", claim=identity)
        # Reuse the established literal locator, rather than implementing another
        # claim presence check. The repair adds address/base/render constraints.
        position=C['location'](base,{"id":identity,"text":old})
        require(position==previous_claims[identity]["location"], "PREVIOUS_CLAIM_LOCATION_CHANGED")
        start=base.index(old);end=start+len(old)
        before,after=base[:start],base[end:]
        require(not before or re.search(r'\r?\n[ \t]*\r?\n$',before), "TARGET_MUST_BE_WHOLE_PARAGRAPH")
        require(not after or re.match(r'^(?:\r?\n[ \t]*\r?\n|\r?\n$)',after), "TARGET_MUST_BE_WHOLE_PARAGRAPH")
        target={"claim":identity,"old":old,"new":new,"startByte":position["startByte"],"endByte":position["endByte"]}
        targets.append(target);by_id[identity]["text"]=new;by_id[identity].pop("occurrence",None)
        if "evidence" in change:by_id[identity]["evidence"]=change["evidence"]
        require(type(change.get("rebind",False)) is bool, "INVALID_EXPLICIT_REBIND")
        if change.get("rebind"):rebind.append(identity)
    targets.sort(key=lambda t:t['startByte'])
    require(all(a['endByte']<=b['startByte'] for a,b in zip(targets,targets[1:])), "OVERLAPPING_REPAIR_TARGETS")
    raw=base.encode();parts=[];offset=0
    for target in targets:
        parts.extend([raw[offset:target['startByte']],target['new'].encode()]);offset=target['endByte']
    parts.append(raw[offset:]);candidate=b''.join(parts)
    return candidate,{"claims":claims,"replace":rebind},targets


class MainParagraphs(HTMLParser):
    """Record existing renderer output; do not interpret Markdown syntax."""
    def __init__(self,html):
        super().__init__(convert_charrefs=True);self.html=html;self.offsets=[0]
        self.offsets.extend(m.end() for m in re.finditer('\n',html));self.stack=[];self.main_count=0;self.paragraphs=[];self.current=None
    def position_offset(self):
        line,column=self.getpos();return self.offsets[line-1]+column
    def handle_starttag(self,tag,attrs):
        if tag in ('html','body'):
            require(not any(key in ('hidden','aria-hidden','style','class','id') for key,_ in attrs),
                    "UNSUPPORTED_RENDERED_VISIBILITY")
        require(tag not in ('script','iframe','template','noscript'),"UNSUPPORTED_RENDERED_ACTIVE_CONTENT")
        if tag=='main':
            self.main_count+=1;require(self.main_count==1 and 'main' not in self.stack,"UNSUPPORTED_RENDERED_MAIN")
            require(all(parent in ('html','body') for parent in self.stack),"UNSUPPORTED_RENDERED_MAIN_ANCESTORS")
            require(not attrs,"UNSUPPORTED_RENDERED_MAIN_ATTRIBUTES")
        if tag=='p' and self.stack and self.stack[-1]=='main':
            self.current={"start":self.position_offset(),"text":[],"inline":[],"unsafeInline":False,"attrs":attrs,"comment":False}
        elif self.current is not None:
            self.current['inline'].append(('start',tag,tuple(attrs)))
            safe=tag=='code' and not attrs and self.stack[-1] in ('p','a')
            if tag=='a':
                values=dict(attrs);href=values.get('href');parsed=urlsplit(href or '')
                safe=(len(values)==len(attrs) and set(values)<={'href','title'} and bool(href)
                      and not re.search(r'[\s\x00-\x1f\x7f\\]',href) and not href.startswith('//')
                      and (parsed.scheme in ('http','https') and bool(parsed.netloc) and parsed.username is None
                           or not parsed.scheme and ':' not in href)
                      and self.stack[-1]=='p')
            if not safe:self.current['unsafeInline']=True
        if tag not in ('area','base','br','col','embed','hr','img','input','link','meta','param','source','track','wbr'):
            self.stack.append(tag)
    def handle_endtag(self,tag):
        require(self.stack and self.stack[-1]==tag,"UNSUPPORTED_OR_MALFORMED_RENDERED_HTML",tag=tag)
        if tag=='p' and self.current is not None and len(self.stack)>1 and self.stack[-2]=='main':
            item=self.current;item['end']=self.html.index('>',self.position_offset())+1;item['text']=''.join(item['text'])
            self.paragraphs.append(item);self.current=None
        elif self.current is not None:self.current['inline'].append(('end',tag))
        self.stack.pop()
    def handle_data(self,data):
        if self.current is not None:
            self.current['text'].append(data);tokens=self.current['inline']
            if tokens and tokens[-1][0]=='text':tokens[-1]=('text',tokens[-1][1]+data)
            else:tokens.append(('text',data))
    def handle_comment(self,data):
        if self.current is not None:self.current['comment']=True


def normalize(text):return ' '.join(text.split())


def main_paragraphs(html):
    parser=MainParagraphs(html);parser.feed(html);parser.close()
    require(parser.main_count==1 and not parser.stack,"UNSUPPORTED_RENDERED_MAIN")
    return parser.paragraphs


def supported_row(row):
    require(not row['attrs'] and not row['unsafeInline'] and not row['comment'],"UNSUPPORTED_RENDERED_TARGET_LAYOUT")


def inline_elements(row,tag):
    elements=[];active=None
    for token in row['inline']:
        if token[:2]==('start',tag):active=[]
        if active is not None:active.append(token)
        if token==('end',tag) and active is not None:elements.append(active);active=None
    return elements


def target_profiles(directory,records):
    expected={}
    for record in records:
        rows=[]
        for side in ('old','new'):
            path=Path(directory)/record[side]['html'];raw=path.read_bytes()
            require(sha(raw)==record[side]['digest'],"RENDERED_TARGET_REFERENCE_CHANGED")
            paragraphs=main_paragraphs(raw.decode('utf-8'))
            require(len(paragraphs)==1,"TARGET_MUST_RENDER_AS_ONE_PARAGRAPH")
            supported_row(paragraphs[0]);rows.append(paragraphs[0])
        old,new=rows
        require(inline_elements(old,'a')==inline_elements(new,'a'),"EXISTING_INLINE_LINKS_CHANGED")
        # Existing code spans stay intact and ordered; a correction may add code.
        codes=iter(inline_elements(new,'code'))
        require(all(any(candidate==code for candidate in codes) for code in inline_elements(old,'code')),
                "EXISTING_INLINE_CODE_CHANGED")
        expected[record['claim']]=rows
    return expected


def rendered_check(before,after,targets,expected=None):
    parsed=[main_paragraphs(html) for html in (before,after)]
    old_rows,new_rows=parsed
    require(len(old_rows)==len(new_rows),"RENDERED_PARAGRAPH_SET_CHANGED")
    old_spans=[];new_spans=[];affected=[]
    for target in targets:
        profiles=(expected or {}).get(target['claim'])
        matches=[i for i,row in enumerate(old_rows) if (row['inline']==profiles[0]['inline'] if profiles
                 else normalize(row['text'])==normalize(target['old']))]
        require(len(matches)==1,"BASE_RENDERED_TARGET_NOT_UNIQUE_OR_VISIBLE",claim=target['claim'])
        index=matches[0];old_row,new_row=old_rows[index],new_rows[index]
        for row in (old_row,new_row):supported_row(row)
        require(new_row['inline']==profiles[1]['inline'] if profiles else normalize(new_row['text'])==normalize(target['new']),
                "RENDERED_TARGET_NOT_PROPAGATED",claim=target['claim'])
        marker='\x00REPAIR:'+target['claim']+'\x00'
        old_spans.append((old_row['start'],old_row['end'],marker));new_spans.append((new_row['start'],new_row['end'],marker))
        affected.append({"claim":target['claim'],"mainParagraphIndex":index,"visible":True})
    def masked(html,spans):
        for start,end,marker in sorted(spans,reverse=True):html=html[:start]+marker+html[end:]
        return html
    require(masked(before,old_spans)==masked(after,new_spans),"RENDERED_OUTSIDE_TARGET_CHANGED")
    return {"affected":affected,"unrelatedRenderedBytesPreserved":True,
        "scope":"Stable standalone HTML/main, direct paragraphs with text, inline code and unchanged links; structural visibility only, not browser-computed CSS or semantic review."}


def repair(manifests,base_directory,patch_path,renderer_path,output,*,mapping=None):
    output=Path(output);output.mkdir(parents=True,exist_ok=False)
    base_directory=Path(base_directory);base_raw=(base_directory/'index.md').read_bytes();base=base_raw.decode('utf-8')
    previous_raw=(base_directory/'sources.json').read_bytes();previous=json.loads(previous_raw);patch_raw=Path(patch_path).read_bytes()
    for name,raw in [('base.original.md',base_raw),('base.original.sources.json',previous_raw),('patch.original.json',patch_raw)]:
        (output/name).write_bytes(raw)
    candidate,request,targets=addressed(base,previous,json.loads(patch_raw))
    # Revalidate the previous source map against its exact native closure using
    # the existing binder. No source acquisition or separate trust protocol.
    base_claims=output/'base.claims.json';base_claims.write_text(encode({'claims':[authored(c) for c in previous['claims']]}))
    C['compile_map'](manifests,output/'base.original.md',base_claims,output/'base-check',mapping=mapping)
    validated=json.loads((output/'base-check/sources.json').read_text())
    require([{k:c[k] for k in ('id','text','evidence','sources','location')} for c in validated['claims']]
            ==[{k:c[k] for k in ('id','text','evidence','sources','location')} for c in previous['claims']],"PREVIOUS_BINDINGS_CHANGED")
    candidate_path=output/'candidate.md';candidate_path.write_bytes(candidate)
    request_path=output/'candidate.claims.json';request_path.write_text(encode(request))
    C['compile_map'](manifests,candidate_path,request_path,output/'candidate',mapping=mapping,previous=base_directory/'sources.json')
    require((output/'candidate/index.md').read_bytes()==candidate,"UNSUPPORTED_REPAIR_LINK_TRANSFORMATION")
    argv=renderer_command(renderer_path)
    (output/'renderer.original.json').write_bytes(Path(renderer_path).read_bytes())
    renderer_receipts=[];references=[]
    jobs=[('base-check','base-render','before'),('candidate','rendered','after')]
    for index,target in enumerate(targets):
        if not formatted(target):continue
        record={'claim':target['claim']}
        for side in ('old','new'):
            source=f'targets/{index}-{side}/input';destination=f'targets/{index}-{side}/rendered'
            (output/source).mkdir(parents=True);(output/source/'index.md').write_bytes(target[side].encode())
            jobs.append((source,destination,f'target-{index}-{side}'));record[side]={'html':destination+'/index.html'}
        references.append(record)
    for source,destination,label in jobs:
        renderer_receipts.append(run_renderer(argv, output/source, output/destination, output, label))
    for record in references:
        for side in ('old','new'):record[side]['digest']=sha((output/record[side]['html']).read_bytes())
    expected=target_profiles(output,references)
    before=(output/'base-render/index.html').read_bytes();after=(output/'rendered/index.html').read_bytes()
    checked=rendered_check(before.decode('utf-8'),after.decode('utf-8'),targets,expected)
    state={"schema":"codeclew-reader-repair-state/1.0","targets":targets,
        "baseMarkdownDigest":sha(base_raw),"previousMapDigest":previous['mapDigest'],"patchDigest":sha(patch_raw),
        "outputMarkdownDigest":sha(candidate),"outputMapDigest":json.loads((output/'candidate/sources.json').read_text())['mapDigest'],
        "beforeHtmlDigest":sha(before),"afterHtmlDigest":sha(after),"rendererReceipts":renderer_receipts,"renderedTargets":references,
        "manifests":[str(Path(m).resolve()) for m in manifests]}
    state['stateDigest']=digest(state);(output/'repair-state.json').write_text(encode(state))
    return {"status":"ADDRESSED_REPAIR_RENDERED","affectedClaims":[t['claim'] for t in targets],
        "markdown":str(output/'candidate/index.md'),"html":str(output/'rendered/index.html'),
        "unrelatedMarkdownBytesPreserved":True,**checked,"sourceAcquisitionCommandsIssued":0,
        "semanticTruthAssessed":False,"nativePublicationApproval":False,
        "rendererAuthority":"Caller-trusted existing renderer; argv execution, no shell. Renderer/CSS behavior beyond supported structure is not attested."}


def check(directory):
    directory=Path(directory)
    require((directory/'repair-state.json').is_file(),"MISSING_ADDRESSED_REPAIR_STATE")
    state=json.loads((directory/'repair-state.json').read_text())
    require(isinstance(state,dict),"INVALID_REPAIR_STATE")
    require(state.get('stateDigest')==digest({k:v for k,v in state.items() if k!='stateDigest'}),"REPAIR_STATE_CHANGED")
    base_raw=(directory/'base.original.md').read_bytes();patch_raw=(directory/'patch.original.json').read_bytes()
    previous=json.loads((directory/'base.original.sources.json').read_bytes())
    require(sha(base_raw)==state['baseMarkdownDigest'] and sha(patch_raw)==state['patchDigest'],"REPAIR_INPUT_CHANGED")
    expected,_,targets=addressed(base_raw.decode(),previous,json.loads(patch_raw))
    require(targets==state['targets'] and (directory/'candidate/index.md').read_bytes()==expected
            and sha(expected)==state['outputMarkdownDigest'],"UNADDRESSED_OR_OUT_OF_SCOPE_MARKDOWN_EDIT")
    current=json.loads((directory/'candidate/sources.json').read_text())
    require(current.get('mapDigest')==state['outputMapDigest']==digest({k:v for k,v in current.items() if k!='mapDigest'}),"REPAIR_BINDINGS_CHANGED")
    # Same validated manifests; do not reopen Work or reinterpret a new snapshot.
    for manifest in state['manifests']:C['load'](manifest)
    before=(directory/'base-render/index.html').read_bytes();after=(directory/'rendered/index.html').read_bytes()
    require(sha(before)==state['beforeHtmlDigest'] and sha(after)==state['afterHtmlDigest'],"RENDERED_ARTIFACT_CHANGED")
    return {"status":"ADDRESSED_REPAIR_RENDERED",**rendered_check(before.decode(),after.decode(),targets,
            target_profiles(directory,state.get('renderedTargets',[]))),
        "semanticTruthAssessed":False,"sourceAcquisitionCommandsIssued":0}
