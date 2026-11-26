// A finite presentation of an authored sequence and its checked source excerpts.
import {htmlIds} from './navigation.mjs';
const escape=s=>s.replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('"','&quot;');
function need(ok,path,message){if(!ok)throw new Error(`Invalid sequence ${path}: ${message}`)}
function object(v,path,keys){need(v&&typeof v==='object'&&!Array.isArray(v),path,'expected object');need(Object.keys(v).every(k=>keys.includes(k)),path,'unknown property')}
function text(v,path){need(typeof v==='string'&&v.trim().length>0,path,'expected nonempty text');return v}
function target(v,path){
 text(v,path);need(/^[^/\\:?#\u0000-\u0020]+\.md(?:#[^\s]+)?$/u.test(v),path,'expected flat Markdown page with optional anchor');
 return v.replace(/\.md(?=#|$)/,'.html');
}
function catalogRef(item,path){
 if(item.catalogRef===undefined)return;
 need(typeof item.catalogRef==='string'&&/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(item.catalogRef),path,'invalid canonical external ID');
 need(item.target===undefined,path,'choose target or catalogRef');
}

export function validateSequence(v){
 object(v,'/',['title','participants','steps']);text(v.title,'/title');
 need(Array.isArray(v.participants)&&v.participants.length>=2,'/participants','expected at least two participants');
 const ids=new Set();
 v.participants.forEach((p,i)=>{const path='/participants/'+i;object(p,path,['id','title','target','catalogRef','local']);text(p.id,path+'/id');need(!ids.has(p.id),path+'/id','duplicate participant');ids.add(p.id);text(p.title,path+'/title');if(p.target!==undefined)target(p.target,path+'/target');catalogRef(p,path);need(p.local===undefined||typeof p.local==='boolean',path,'expected local boolean')});
 need(Array.isArray(v.steps)&&v.steps.length>0,'/steps','expected nonempty steps');
 v.steps.forEach((s,i)=>{const path='/steps/'+i;object(s,path,['from','to','label','kind','target','condition','catalogRef','independent']);need(ids.has(s.from),path+'/from','unknown participant');need(ids.has(s.to),path+'/to','unknown participant');text(s.label,path+'/label');need(['call','return','async','await','note'].includes(s.kind),path+'/kind','expected call, return, async, await or note');if(s.target!==undefined)target(s.target,path+'/target');if(s.condition!==undefined)text(s.condition,path+'/condition');catalogRef(s,path);need(s.independent===undefined||s.independent===true&&['note','async'].includes(s.kind),path,'invalid independent start')});
 return v;
}

function stepText(label,position,hasOpening){
 const ordinal=/^\s*(?:(?:step|\u0448\u0430\u0433)\s+([1-9][0-9]*)\s*[.):–—-]|([1-9][0-9]*)[.)])\s+(?=\S)/iu.exec(label);
 if(!ordinal)return label;
 const number=Number(ordinal[1]??ordinal[2]);
 // The assembled diagram may start with an entry step before the authored steps.
 return number===position||hasOpening&&number===position-1?label.slice(ordinal[0].length):label;
}

const english={
 call:'Call',return:'Return',async:'Start without waiting',await:'Await completion',
 note:'Local step',steps:'Steps',participants:'Participants',diagram:'Diagram; scroll horizontally when needed',
 arrows:{call:'Call',return:'Return',async:'Hand off',await:'Wait',note:'Work'},
 read:'Read',insert:'Insert',update:'Update',open:'Open connection',transaction:'Begin transaction',commit:'Commit',awaitCommit:'Wait for commit',
 requiredWait:'The successful result on this path requires this wait to complete. If it fails, the response is open.',
 phaseOrder:'Order relative to the earlier phase is open.',
 visibility:'The reader can see an earlier write when it becomes visible and the same record is selected.',
 results:{202:'Accepted',404:'Not found'},start:'Start',request:'Request',independent:'Independent start',conditional:'Conditional',relation:'Related step',
};

const typeDeclaration=/^\s*(?:[\w$]+ +)*(?:class|interface|object|record|struct) +([\w$]+)/gm;
const callableDeclaration=/^[ \t]*(?:(?:export +)?(?:async +)?function +|(?:async +)?def +|(?:[\w$]+ +)*fun +|(?:public|private|protected|internal|static) +(?:[\w$.<>,?\[\]]+ +)+)([\w$]+)\s*\(/gm;
const namespaceDeclaration=/^[ \t]*(?:namespace|package) +([\w$.]+)\b/gm;

function sourceFiles(sources){
 return sources.flatMap((group,i)=>{
  if(!/\.(?:cs|java|kt|kts|tsx?|jsx?|py)$/i.test(group.file))return [];
  const declarations=pattern=>new Set(group.ranges.flatMap(({text})=>[...text.matchAll(pattern)].map(match=>match[1])));
  return [{group,i,stem:group.file.split('/').at(-1).split('.')[0],types:declarations(typeDeclaration),
   callables:declarations(callableDeclaration),namespaces:declarations(namespaceDeclaration)}];
 });
}

function actorSources(actor, files){
 const title=/^([\w$]+(?:\.[\w$]+)*)(?:\(\))?$/.exec(actor.title);
 if(!title || actor.catalogRef)return [];
 const parts=title[1].split('.'), end=parts.pop(), root=parts.pop(), space=parts.join('.');
 return files.flatMap(file=>{
  if(space&&!file.namespaces.has(space))return [];
  if(root)return file.callables.has(end)&&(file.types.has(root)||file.stem===root&&!file.types.size)
   ?[{...file,member:end}]:[];
  if(file.types.has(end))return [{...file}];
  return file.callables.has(end)?[{...file,member:end}]:[];
 });
}

function systemLanes(participants, opening, systems, names, files){
 const lanes=[], bySystem=new Map(), index=new Map(), known=new Set(), frameworks=new Map();
 const owner=participants.find(p=>p.id===opening.to);
 const owningService=systems.get(owner.target?.split('#')[0]);
 function record(map,name,service){
  if(!map.has(name))map.set(name,service);
  else if(map.get(name)!==service)map.set(name,null);
 }
 for(const {group:{service,ranges}} of files){
  const system=names.get(service);
  if(!system)continue;
  for(const {text} of ranges){
   if(/\bWebApplication\.CreateBuilder\(/.test(text))record(frameworks,'ASP.NET Core',system);
  }
 }
 const role=/ +(?:host|service|listener|consumer|worker|\u0445\u043e\u0441\u0442|\u0441\u0435\u0440\u0432\u0438\u0441|\u043e\u0431\u0440\u0430\u0431\u043e\u0442\u0447\u0438\u043a)$/iu;
 for(const actor of participants){
  const local=actor.local && actor.id!==opening.from;
  const candidate=local?owner:actor;
  const bound=systems.get(candidate.target?.split('#')[0]);
  const named=names.get(actor.title.replace(role,'').toLowerCase());
  const owners=new Set(actorSources(actor,files).map(({group})=>names.get(group.service)));
  const declared=owners.size===1&&owners.values().next().value;
  const runtime=frameworks.get(actor.title);
  const identified=bound||(!actor.target&&!actor.catalogRef&&(declared||named||(runtime===owningService&&runtime)));
  const service=actor.id!==opening.from && identified;
  const key=service?'service:'+service.id:candidate.catalogRef?'external:'+candidate.catalogRef:'actor:'+candidate.id;
  if(!bySystem.has(key)){
   bySystem.set(key,lanes.length);
   lanes.push(service||candidate);
  }
  const lane=bySystem.get(key);
  index.set(actor.id,lane);
  if(service)known.add(lane);
 }
 return {lanes,index,known,frameworks};
}


function project(steps,index,known,participants,independentConditions){
 const groups=[];
 for(const step of steps){
  const from=index.get(step.from),to=index.get(step.to);
  const registration=step.kind==='note'&&step.from!==step.to&&from===to
      && /\b(?:AddHostedService|ScheduledExecutorService)\b/.test(step.label)
      && step.label.includes(participants.get(step.to).title);
  const independent=step.independent===true||registration|| (step.kind==='note' && step.from!==step.to
      && independentConditions.includes(step.condition));
  const prior=groups.at(-1);
  const same=prior&&prior.from===from&&prior.to===to&&prior.condition===step.condition&&!independent&&!prior.independent;
  const localNotes=same&&step.kind==='note'&&prior.kind==='note'&&step.from===step.to&&prior.local;
  const internal=from===to&&known.has(from)&&step.kind!=='async'&&!independent
      && (step.kind!=='return'||step.from!==steps[0].to);
  const waiting=same&&step.kind==='await'&&prior.kind==='call'
      && prior.details[0].from===step.from&&prior.details[0].to===step.to;
  const opening=same&&groups.length===1&&step.kind==='call'&&prior.lead.target?.endsWith('#handler')
      &&step.from===prior.lead.from&&step.to===prior.lead.to;
  if(localNotes||waiting||opening||same&&internal&&prior.internal){
   prior.details.push(step);
  }else{
   groups.push({from,to,kind:step.kind,condition:step.condition,local:step.from===step.to,
                independent,internal,lead:step,details:[step]});
  }
 }
 const exchanges=[];
 const touches=(work,exchange)=>exchange&&['call','await'].includes(exchange.kind)&&!exchange.internal
  &&!exchange.independent&&work.condition===exchange.condition
  &&(work.from===exchange.from||work.from===exchange.to);
 for(const [index,work] of groups.entries()){
  const previous=exchanges.at(-1),next=groups[index+1];
  if(work.internal&&work.kind!=='await'&&touches(work,previous))previous.details.push(...work.details);
  else if(work.internal&&work.kind!=='await'&&touches(work,next))next.details.unshift(...work.details);
  else exchanges.push(work);
 }
 return exchanges;
}

function actionLabel(step, words, recipient, frameworks){
 const {kind,label}=step;
 if(kind==='return'){
  const explicit=/(?:\breturns?|\baction) `?HTTP ([1-5][0-9]{2})\b/i.exec(label)?.[1];
  const factory=frameworks.get(recipient.title)&&/\breturn `(NotFound|Accepted)\(/.exec(label)?.[1];
  const status=explicit||({NotFound:'404',Accepted:'202'})[factory];
  if(status)return [status,words.results[status]].filter(Boolean).join(' ');
 }
 if(kind==='call'||kind==='await'){
  if(/\bCommitAsync\(|^Await commit\b/i.test(label))return kind==='await'?words.awaitCommit:words.commit;
  if(kind==='call'&&/\bOpenAsync\(/.test(label))return words.open;
  if(kind==='call'&&/\bBeginTransactionAsync\(/.test(label))return words.transaction;
  const verb=/(?:^|`)\s*(SELECT|INSERT|UPDATE)\b/i.exec(label)?.[1].toUpperCase();
  if(verb)return words[{SELECT:'read',INSERT:'insert',UPDATE:'update'}[verb]];
 }
 return words.arrows[kind];
}

const normalized=text=>text.trim().replace(/\s+/g,' ');
const sqlVerb=/^(SELECT|INSERT|UPDATE|DELETE)\b/i;

function sourceOperation(label){
 const spans=[...label.matchAll(/`([^`]{1,240})`/g)].map(match=>match[1]);
 const statements=spans.filter(span=>sqlVerb.test(span));
 const selection=/^select\b/i.test(label)&&spans.find(span=>/^[\w.]+$/.test(span));
 const qualifier=selection&&spans.find(span=>/^(?:WHERE|ORDER BY|FOR UPDATE)\b/i.test(span));
 if(statements.length||selection){
  if(statements.length>1||!statements.length&&!qualifier)return null;
  const expected=statements[0]?.trim();
  return code=>[...code.matchAll(/(["'`])((?:SELECT|INSERT|UPDATE|DELETE)\b[^\r\n]*?)\1/gi)]
   .filter(match=>{
    if(expected)return match[2].trim()===expected;
    const sql=normalized(match[2]);
    return /^SELECT\b/i.test(sql)&&/\bFROM\s+([\w.]+)/i.exec(sql)?.[1]===selection
     &&sql.endsWith(' '+normalized(qualifier));
   }).map(match=>match.index);
 }
 const calls=spans.filter(span=>/^[\w$.]+\([^;\n]*\)$/.test(span));
 if(calls.length!==1)return null;
 const call=calls[0];
 return code=>{
  const matches=[];
  for(let offset=code.indexOf(call);offset!==-1;offset=code.indexOf(call,offset+call.length)){
   const line=code.slice(code.lastIndexOf('\n',offset-1)+1,code.indexOf('\n',offset)===-1?code.length:code.indexOf('\n',offset));
   if(/[\w$."'\\`]/.test(code[offset-1]||'')||/["'`]/.test(code[offset+call.length]||'')
      ||[...line.matchAll(callableDeclaration)].some(match=>call.startsWith(match[1]+'(')))continue;
   matches.push(offset);
  }
  return matches;
 };
}

function sourceMarker(files, step, actor, service){
 if(!service||!['call','await'].includes(step.kind))return '';
 const operation=sourceOperation(step.label);
 if(!operation)return '';
 const scope=actorSources(actor,files);
 if(scope.length!==1||scope[0].group.service!==service)return '';
 const [{group,i,member,types}]=scope;
 if(types.size>1)return '';
 // A bound footer can contain broader or competing ranges. Link only when
 // the sending scope and the operation locate one unambiguous excerpt.
 const matches=group.ranges.flatMap(({text},j)=>{
  const start=text.split('\n').find(line=>line.trim()&&!/^\s*(?:\[|@)/.test(line))||'';
  if(member&&[...start.matchAll(callableDeclaration)][0]?.[1]!==member)return [];
  return operation(text.replace(/^[ \t]*(?:\/\/|#|\/\*|\*).*$/gm,'')).map(()=>j);
 });
 return matches.length===1?`<span data-clew-step-source="${i}-${matches[0]}"></span>`:'';
}

function phaseEvidence(phases, words, participant, frameworks, owner){
 const notes=new Map(), writes=new Map(), visibility=new Set();
 for(const [index,phase] of phases.entries()){
  const steps=phase.flatMap(group=>group.details.map(step=>({step,group})));
  for(const [position,{step,group}] of steps.entries()){
   if(step.kind==='await'){
    const call=/^Await commit completion before `([\w$]+)` completes\b/.exec(step.label)?.[1];
    const waiter=call&&steps.slice(position+1).find(({step:s})=>s.kind==='await'&&s.from===owner
       &&s.label.includes('`'+call+'`'));
    if(waiter&&steps.slice(steps.indexOf(waiter)+1).some(({step:s})=>s.from===owner&&s.kind==='return'
        &&s.condition&&s.condition===waiter.step.condition
        &&/^2[0-9]{2}(?: |$)/.test(actionLabel(s,words,participant.get(s.to),frameworks)))){
     notes.set(group.number,words.requiredWait);
    }
   }
   if(index&&/^(?:select|read)\b/i.test(step.label)&&['call','await'].includes(step.kind)
       &&[...writes.get(group.to)||[]].some(table=>step.label.includes('`'+table+'`'))){
    visibility.add(index);
   }
  }
  for(const {step,group} of steps){
   const table=['call','await'].includes(step.kind)&&/`INSERT INTO ([\w.]+)\b/i.exec(step.label)?.[1];
   if(table){
    if(!writes.has(group.to))writes.set(group.to,new Set());
    writes.get(group.to).add(table);
   }
  }
 }
 return {notes,visibility};
}

const link=(href,label,ref)=>ref!==undefined?`<a data-clew-catalog-ref="${escape(ref)}">${escape(label)}</a>`:href===undefined?escape(label):`<a href="${escape(target(href,'/target'))}">${escape(label)}</a>`;

// The bundled v0.15.6 drawing routine also accepts projected labels and detail rows.
export function drawSequence(v,words=english,{offset=0,label,detail}={}){
 const stepLabel=label??((s,number)=>number+'. '+link(s.target,s.label,s.catalogRef));
 const actors=v.participants.map(p=>link(p.target,p.title,p.catalogRef)).join(' · ');
 const n=v.participants.length,index=id=>v.participants.findIndex(p=>p.id===id),title=id=>v.participants[index(id)].title;
 const lifelines=`<div class="sequence-lifelines" aria-hidden="true">${v.participants.map(()=>'<span></span>').join('')}</div>`;
 const rows=v.steps.map((s,i)=>{
  const from=index(s.from),to=index(s.to);let left=(Math.min(from,to)+.5)/n*100,width=Math.abs(from-to)/n*100;
  const self=from===to;if(self){left=from/n*100;width=100/n;}
  return `<div class="sequence-row"><div class="sequence-message ${s.kind}${from>to?' reverse':''}${self?' self':''}" role="group" aria-label="${escape(title(s.from))} → ${escape(title(s.to))}" style="margin-left:${left}%;width:${width}%"><span>${stepLabel(s,offset+i+1)}${(['async','await'].includes(s.kind)||(self&&s.kind==='note'))?`<small>${escape(words[s.kind])}</small>`:''}${s.condition?`<small>${escape(s.condition)}</small>`:''}</span>${self&&s.kind!=='note'?'<i class="sequence-loop" aria-hidden="true"></i>':''}</div></div>`;
 }).join('');
 const trace=v.steps.map((s,i)=>detail?detail(s,offset+i+1):`<li><span class="sequence-direction">${escape(title(s.from))} → ${escape(title(s.to))} · ${escape(words[s.kind])}</span>${link(s.target,s.label,s.catalogRef)}${s.condition?`<p>${escape(s.condition)}</p>`:''}</li>`).join('');
 return `<figure class="clew-sequence"><figcaption>${escape(v.title)}</figcaption><p class="sequence-participants">${escape(words.participants)}: ${actors}</p><p class="sequence-hint">${escape(words.diagram)}</p><div class="sequence-scroll" tabindex="0" role="region" aria-label="${escape(words.diagram)}"><div class="sequence-diagram" role="group" aria-label="${escape(v.title)}" style="min-width:${Math.max(560,n*150)}px"><div class="sequence-actors">${v.participants.map(p=>`<span>${link(p.target,p.title,p.catalogRef)}</span>`).join('')}</div><div class="sequence-body">${lifelines}${rows}</div></div></div><details class="sequence-text" open><summary>${escape(words.steps)}</summary><ol${offset?` start="${offset+1}"`:''}>${trace}</ol></details></figure>\n`;
}

export function renderSequence(value,localWords,entryLabel=(_target,text)=>text,
  {id,systems=new Map(),names=new Map(),sources=[],independentConditions=[]}={}){
 const v=validateSequence(value);
 need(id===undefined||typeof id==='string'&&/^[\w-]+$/.test(id),'/','invalid figure identity');
 const words={...english,...localWords};
 const first=v.steps[0];
 const openingTarget=v.participants.find(p=>p.id===first.to)?.target;
 const hasOpening=first.from==='caller'&&first.from!==first.to&&['call','async'].includes(first.kind)&&openingTarget?.endsWith('#handler');
 const caption=hasOpening?entryLabel(openingTarget,v.title):v.title;
 const participants=v.participants.map(p=>({...p,title:entryLabel(p.target,p.title)}));
 const steps=v.steps.map((s,i)=>{
  const displayTarget=s.target??(i===0&&hasOpening?openingTarget:undefined);
  return {...s,label:stepText(entryLabel(displayTarget,s.label),i+1,hasOpening)};
 });
 const participant=new Map(participants.map(p=>[p.id,p]));
 const actor=p=>link(p.target,p.title,p.catalogRef);
 const files=sourceFiles(sources);
 const {lanes,index,known,frameworks}=systemLanes(participants,first,systems,names,files);
 const groups=project(steps,index,known,participant,independentConditions);
 const phases=[[]];
 for(const [offset,group] of groups.entries()){
  if(group.independent&&offset)phases.push([]);
  phases.at(-1).push({...group,number:offset+1});
 }
 const {notes,visibility}=phaseEvidence(phases,words,participant,frameworks,first.to);
 return phases.map((phase,phaseIndex)=>{
  const present=new Set(phase.flatMap(group=>group.independent?[group.to]:[group.from,group.to]));
  const actors=[...present].sort((a,b)=>a-b).map(i=>({...lanes[i],id:i}));
  const exchanges=phase.map(({from,to,kind,condition,independent,local,internal,number,details,lead})=>{
   const association=kind==='note'&&!local&&!internal;
   const title=independent?words.start:association?words.relation:number===1&&hasOpening
    ?(kind==='call'?words.request:words.start):internal&&kind!=='await'?words.arrows.note:actionLabel(lead,words,participant.get(lead.to),frameworks);
   const label=kind==='call'&&details.some(step=>step.kind==='await')?title+' · '+words.arrows.await:title;
   return {from:independent||association?to:from,to,kind:independent||association||internal?'note':kind,
    label,condition:condition?words.conditional:undefined,details,independent};
  });
  const qualifier=phaseIndex?`<p>${escape(words.phaseOrder)}${visibility.has(phaseIndex)?' '+escape(words.visibility):''}</p>\n`:'';
  const diagram={title:phaseIndex?words.independent:caption,participants:actors,steps:exchanges};
  return qualifier+drawSequence(diagram,words,{
   offset:phase[0].number-1,
   label:(step,number)=>{
    const text=number+'. '+escape(step.label);
    return id?`<a href="#${id}-step-${number}-detail">${text}</a>`:text;
   },
   detail:({details,independent},number)=>{
    const statements=details.map((s,position)=>{
     const from=participant.get(s.from),to=participant.get(s.to);
     const type=independent?words.independent:s.kind==='note'&&from!==to?words.relation:words[s.kind];
     const direction=actor(from)+(from===to?'':(s.kind==='note'||independent?' · ':' → ')+actor(to));
     const anchor=id&&position===0?` id="${id}-step-${number}-detail" tabindex="-1"`:'';
     return `<div class="sequence-detail"><span class="sequence-direction"${anchor}>${direction} · ${escape(type)}</span>${link(s.target,s.label,s.catalogRef)}${sourceMarker(files,s,from,known.has(index.get(s.from))?lanes[index.get(s.from)].id:null)}${s.condition?`<p>${escape(s.condition)}</p>`:''}</div>`;
    }).join('');
    const note=notes.get(number);
    return `<li${id?` id="${id}-step-${number}"`:''}>${statements}${note?`<p>${escape(note)}</p>`:''}</li>`;
   },
  });
 }).join('');
}

export function resolveCatalogSequenceLinks(sequences,pages,model){
 const ids=new Set(sequences.flatMap(v=>[...v.participants,...v.steps]).filter(x=>x.catalogRef!==undefined).map(x=>x.catalogRef));
 if(!ids.size)return;
 const records=new Map((model?.semantic?.records||[]).map(r=>[r.id,r]));
 for(const id of ids)need(records.get(id)?.kind==='external','/catalogRef','explicit canonical external required: '+id);
 for(const page of pages)page.html=page.html.replace(/<a data-clew-catalog-ref="([A-Za-z0-9._-]+)">([\s\S]*?)<\/a>/g,(_tag,id,label)=>{
  need(ids.has(id),'/catalogRef','reference is not an authored sequence selection');const to=records.get(id).to;
  return to.problem?`<span class="route-link-unavailable">${label} — ${escape(to.problem.note)}</span>`:`<a href="${escape(to.href)}">${label}</a>`;
 });
}

// Validate authored destinations against the actual rendered pages, including
// bound step headings. The existing full export checker checks them again.
export function checkSequenceTargets(sequences,pages){
 const byFile=new Map(pages.map(p=>[p.file,p.html]));
 for(const v of sequences)for(const item of [...v.participants,...v.steps]){
  if(item.target===undefined)continue;
  const split=item.target.indexOf('#'),file=split<0?item.target:item.target.slice(0,split);let anchor;
  try{anchor=decodeURIComponent(item.target.slice(split+1))}catch{need(false,item.target,'invalid anchor encoding')}
  need(byFile.has(file),item.target,'page does not exist');
  if(split<0)continue;
  const ids=htmlIds(byFile.get(file));
  need(ids.filter(id=>id===anchor).length===1,item.target,'anchor missing or ambiguous');
 }
}
