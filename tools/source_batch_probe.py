#!/usr/bin/env python3
"""Owned Java SOURCE fixture: measure supported helper preparation, never private input."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import runpy
import subprocess
import sys
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('clew','skill-dir','output-dir'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--files',type=int,default=1)
    parser.add_argument('--methods',type=int,default=38)
    parser.add_argument('--page-items',type=int,choices=range(1,101),default=20)
    parser.add_argument('--baseline',action='store_true',help='Also read the same representative evidence through legacy parts')
    parser.add_argument('--retained-operations',type=int,default=0,help='Publish owned retained narrative context before preparing Work (0..16)')
    args=parser.parse_args()
    if not 1<=args.files<=308 or not 1<=args.methods<=128 or args.baseline and args.files*args.methods>128:
        parser.error('owned fixture bounds: 1..308 files, 1..128 methods; legacy baseline at most128 methods')
    if not 0 <= args.retained_operations <= min(16,args.files*args.methods):
        parser.error('retained operations must be0..16 and fit the owned callable count')
    out=args.output_dir.resolve();out.mkdir(parents=True,exist_ok=False)
    source=out/'owned-source';source.mkdir();docs=out/'docs';receipts=[]
    def call(label,argv,accepted=(0,)):
        started=time.monotonic();r=subprocess.run([str(v) for v in argv],capture_output=True)
        (out/(label+'.stdout')).write_bytes(r.stdout);(out/(label+'.stderr')).write_bytes(r.stderr)
        receipts.append(dict(label=label,argv=[str(v) for v in argv],exitCode=r.returncode,seconds=time.monotonic()-started))
        (out/'commands.json').write_text(json.dumps(receipts,indent=2)+'\n')
        if r.returncode not in accepted:raise RuntimeError(label+': '+r.stderr.decode(errors='replace')+r.stdout.decode(errors='replace'))
        return json.loads(r.stdout) if r.stdout.startswith(b'{') else None
    for n in range(args.files):
        prefix='' if args.retained_operations else 'if (value < 0) throw new IllegalArgumentException("owned"); '
        text='public class Owned'+str(n)+' {\n'+''.join('  public int method'+str(i)+'(int value) { '+prefix+'return value + '+str(i)+'; }\n' for i in range(args.methods))+'}\n'
        (source/('Owned'+str(n)+'.java')).write_text(text)
    call('git-init',['git','init','-q',source]);call('git-remote',['git','-C',source,'remote','add','origin','https://example.invalid/owned-source-batch'])
    call('git-add',['git','-C',source,'add','.']);call('git-commit',['git','-C',source,'-c','user.name=Owned Fixture','-c','user.email=fixture@example.invalid','commit','-qm','Owned public SOURCE fixture'])
    def native(label,arguments,accepted=(0,)):
        return call(label,[args.clew,'docs',*arguments,'--root',docs],accepted)
    native('init',['init'])
    service=out/'service.json';service.write_text(json.dumps(dict(schema='codeclew-documentation-service/1.0',id='owned',title='Owned SOURCE fixture',repositoryId='owned',repository='https://example.invalid/owned-source-batch',language='java',profile='source-syntax',targetRef='HEAD',source={'roots':['.'],'dialect':'17'})))
    baseline=native('list',['service','list'])
    native('add',['service','add','--input',service,'--expected-input-digest',baseline['inputDigest']])
    native('bind',['bind','--service','owned','--repo',source])
    checked=native('check',['check'],accepted=(0,3,4))
    if args.retained_operations:
        context=native('context',['context','--service','owned','--snapshot',checked['snapshot'],'--format','raw','--limit','100'])
        entries=[r['record'] for r in context['items'] if r['kind']=='ENTRYPOINT']
        assert len(entries)>=args.retained_operations
        operations=[]
        for index,entry in enumerate(entries[:args.retained_operations]):
            detail=native('entry-context-'+str(index),['context','--service','owned','--entrypoint',entry['id'],'--snapshot',checked['snapshot'],'--format','raw','--limit','100'])
            flows=[r['record'] for r in detail['items'] if r['kind']=='DEPENDENCY' and r['record']['kind']=='FLOW']
            assert flows
            events=[dict(id='step-'+str(n),kind='note',text='The owned source describes this flow.',**{'from':None,'to':None},dependencyIds=[flow['id']],sourceIds=flow['sourceIds']) for n,flow in enumerate(flows)]
            evidence={'dependencyIds':sorted(set(entry['dependencyIds'])|{f['id'] for f in flows}),
                      'sourceIds':sorted(set(entry['sourceIds'])|{s for f in flows for s in f['sourceIds']})}
            operations.append(dict(id=entry['id'],title='Owned retained operation',
                summary=dict(id='summary',text='The owned method returns a value.',**evidence),
                participants=[dict(id='caller',label='Caller',service=None),dict(id='service',label='Owned',service='owned')],
                events=events,
                explanation=[dict(id='paragraph-'+str(n),text='Owned retained context. '+('Context stays retained. '*300),eventIds=[event['id'] for event in events],**evidence) for n in range(4)],
                boundaries=['Owned syntax-only fixture.']))
        narrative=out/'narrative.json';narrative.write_text(json.dumps(dict(schema='codeclew-documentation-narrative/1.3',subject='service:owned',contextDigest=checked['contextDigest'],operations=operations,gaps={e['id']:'Not authored in this owned fixture.' for e in entries[args.retained_operations:]})))
        publication=native('publish-retained',['render','--snapshot',checked['snapshot'],'--input',narrative,'--publish'])
        assert not publication['updateFailures'] and publication['documentedOperations']==args.retained_operations,publication
    request=out/'work.json';request.write_text(json.dumps(dict(schema='codeclew-documentation-work-request/1.0',audience='Owned batch transport measurement',maxItems=args.page_items,maxBytes=49152)))
    work=native('prepare',['work','prepare','--subject','service:owned','--input',request])
    common=[sys.executable,'-I','-S',args.skill_dir/'scripts/evidence_packet.py','--clew',args.clew,'--root',docs,'--service','owned','--snapshot',work['snapshot']]
    call('discover',[*common,'discover-files','--work',work['work'],'--output-dir',out/'discovery'])
    discovery=json.loads((out/'discovery/packet.json').read_text());selection=out/'files.json'
    selection.write_text(json.dumps([{'file':row['file']} for row in discovery['choices']]))
    metrics={}
    for label,extra in ([('legacy',['--source-batch-size','1','--native-batch-size','1'])] if args.baseline else [])+[('batch',[])]:
        call(label,[*common,'read-files','--packet',out/'discovery/packet.json','--selection',selection,'--output-format','metadata','--output-dir',out/label,*extra])
        artifact=out/label;packet=json.loads((artifact/'packet.json').read_text());native_receipts=json.loads((artifact/'receipts.json').read_text())
        started=time.monotonic();module=runpy.run_path(str(args.skill_dir/'scripts/author_citations.py'));module['load'](artifact/'citations.json');validation=time.monotonic()-started
        assert packet['status']=='SELECTED_SOURCES_READY' and len(packet['outcomes'])==sum(r['sourceRecords'] for r in discovery['choices'])
        phases={}; stdout_bytes=0; source_pages=0; context_rows=0
        for receipt in native_receipts:
            command=receipt['command'];value=json.loads(Path(command[command.index('--input')+1]).read_text())
            raw=(artifact/receipt['stdout']).read_bytes();stdout_bytes+=len(raw);native_value=json.loads(raw)
            if command[3]=='read-batch':
                for result in native_value['results']:
                    selection=result['selection']; page=result.get('page',{})
                    if selection.get('projection')=='SOURCE_ROWS':
                        assert page.get('selectionProjection')=='SOURCE_ROWS'
                        source_pages+=1
                        context_rows+=sum(r['kind'] in ('RETAINED_OPERATION','RETAINED_GAPS') for r in page.get('items',[])+page.get('omitted',[]))

            phase='parts' if command[3]=='read-part' else 'metadata' if value.get('query') or value.get('selections') and all('query' in v for v in value['selections']) else 'sourceRows'
            bucket=phases.setdefault(phase,dict(commands=0,seconds=0));bucket['commands']+=1;bucket['seconds']+=receipt['elapsedSeconds']
        assert context_rows==0
        metrics[label]=dict(nativeStdoutBytes=stdout_bytes,sourceRowPages=source_pages,sourceContextRowsDelivered=context_rows,seconds=receipts[-1]['seconds'],nativeCommands=len(native_receipts),nativeSeconds=sum(r['elapsedSeconds'] for r in native_receipts),nativePhases=phases,validationSeconds=validation,sourceRecords=len(packet['outcomes']),sourceParts=sum(len(o.get('sourceParts',[])) for o in packet['outcomes']),textBytes=sum(len(t.encode()) for t in packet['texts'].values()),exactBindingDigest=hashlib.sha256(json.dumps(sorted([(o['sourceBinding']['id'],o['sourceBinding']['reference'],o['source'],packet['texts'][o['textKey']]) for o in packet['outcomes']],key=lambda v:v[0]),sort_keys=True).encode()).hexdigest())
    if args.baseline:assert metrics['legacy']['exactBindingDigest']==metrics['batch']['exactBindingDigest']
    result=dict(status='PASS',ownedFixture=True,files=args.files,methodsPerFile=args.methods,pageItems=args.page_items,retainedOperations=args.retained_operations,work=work['work'],snapshot=work['snapshot'],metrics=metrics,discoverySeconds=next(r['seconds'] for r in receipts if r['label']=='discover'))
    (out/'RESULT.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))


if __name__=='__main__':main()
