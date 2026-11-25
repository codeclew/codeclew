// Navigation declarations only. No source acquisition, graph traversal or remote fetch.
import {draftTarget,localTarget,validateDraftNavigation} from './navigation.mjs';
export const REGISTRY_BYTES=1024*1024;
const fail=(ok,path,message,details)=>{if(!ok)throw Object.assign(Error(`Invalid docset routes ${path}: ${message}`),{path,details})};
function shape(v,keys,path,optional=[]){
 fail(v&&typeof v==='object'&&!Array.isArray(v),path,'expected object');
 fail(Object.keys(v).every(k=>keys.includes(k))&&keys.filter(k=>!optional.includes(k)).every(k=>Object.hasOwn(v,k)),path,'unknown or missing property');
}
const text=(v,p,max=512)=>fail(typeof v==='string'&&v.trim().length>0&&v.length<=max,p,'expected bounded nonempty text');
const id=(v,p)=>fail(typeof v==='string'&&/^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$/.test(v),p,'invalid ID');
const list=(v,p,min=0,max=128)=>{fail(Array.isArray(v)&&v.length>=min&&v.length<=max,p,'array bound');return v};
function unique(rows,path){const m=new Map();for(const row of rows){id(row.id,path);fail(!m.has(row.id),path,'duplicate ID');m.set(row.id,row)}return m}
function baseURL(raw,path){
 text(raw,path,2048);const match=raw.match(/^https?:\/\/[^/?#\\\s]+(\/[^?#]*)$/i);
 fail(match&&/^\/(?:[A-Za-z0-9._~-]+\/)*$/.test(match[1])&&!match[1].split('/').some(x=>x==='.'||x==='..'),path,'use an explicit HTTP(S) directory, unencoded safe path, no traversal/query/fragment');
 let url;try{url=new URL(raw)}catch{fail(false,path,'invalid URL')}
 fail(!url.username&&!url.password&&!url.search&&!url.hash,path,'credentials/query/fragment forbidden');return url.href;
}
export const serviceKey=a=>`${a.docset}/${a.service}`;
export function validateDocsetRoutes(value,nav,docset,version){
 return validateRegistry(value,nav,docset,version,localTarget);
}

// Before rendering, check ownership and declared start identity against authored destinations.
// Anchor existence and uniqueness are still verified against rendered HTML.
export function validateDraftRouteCatalog(value,navigation,files,docset,version){
 const nav=validateDraftNavigation(navigation,files);
 return validateRegistry(value,nav,docset,version,draftTarget);
}

function validateRegistry(value,nav,docset,version,resolve){
 const semanticVersion=value?.schema==='codeclew-cross-docset-routes/1.2';
 shape(value,['schema','revision','catalog','docsets','externalActors','routes',...(semanticVersion?['semanticCatalog']:[])],'/',['catalog','semanticCatalog']);fail(semanticVersion||value.schema==='codeclew-cross-docset-routes/1.1','/schema','unsupported schema');id(value.revision,'/revision');fail(Buffer.byteLength(JSON.stringify(value),'utf8')<=REGISTRY_BYTES,'/','input exceeds 1 MiB');
 const sets=unique(list(value.docsets,'/docsets',1,64),'/docsets');
 const services=new Map(),bases=[];
 for(const [i,d] of value.docsets.entries()){
  const p=`/docsets/${i}`;shape(d,['id','title','version','baseURL','services'],p);text(d.title,p,160);id(d.version,p+'/version');
  const base=baseURL(d.baseURL,p+'/baseURL');
  fail(!bases.some(b=>new URL(b).origin===new URL(base).origin&&(b.startsWith(base)||base.startsWith(b))),p,'publication namespaces overlap');bases.push(base);
  unique(list(d.services,p+'/services',0,64),p+'/services');
  for(const s of d.services){shape(s,['id','title','overview'],p+'/services');text(s.title,p,160);services.set(serviceKey({docset:d.id,service:s.id}),{...s,docset:d.id,baseURL:base,version:d.version})}
 }
 fail(sets.has(docset),'/docset','current docset not declared');fail(sets.get(docset).version===version,'/version','current docset version differs');
 const local=sets.get(docset);fail(local.services.length===nav.services.length&&local.services.every(s=>nav.services.some(n=>n.id===s.id)),'/services','local service inventory differs');
 function navigationOwner(file){
  if(!nav.owners.has(file))return null;
  const service=nav.owners.get(file),i=nav.services.findIndex(s=>s.id===service);
  const path=i<0?'/services':`/services/${i}/pages/${nav.services[i].pages.indexOf(file)}`;
  return {file:'navigation.json',path,service,target:file};
 }
 const actors=unique(list(value.externalActors,'/externalActors',0,64),'/externalActors');
 for(const a of actors.values()){shape(a,['id','title',...(semanticVersion?['canonical']:[])],'/externalActors',['canonical']);text(a.title,'/externalActors',160)}
 function actor(a,p,recipient=false){
  fail(a&&typeof a==='object',p,'expected actor');
  if(a.kind==='service'){shape(a,['kind','docset','service'],p);id(a.docset,p+'/docset');id(a.service,p+'/service');fail(services.has(serviceKey(a)),p,'unknown service namespace')}
  else if(a.kind==='external'){shape(a,['kind','id'],p);fail(actors.has(a.id),p,'unknown external actor')}
  else if(recipient&&a.kind==='unknown'){shape(a,['kind','reason'],p);text(a.reason,p)}
  else if(recipient&&a.kind==='none')shape(a,['kind'],p);
  else fail(false,p,'unsupported actor kind');
 }
 function destination(t,p,owner,route){
  shape(t,['docset','page','anchor','problem'],p,['anchor','problem']);fail(sets.has(t.docset),p,'unknown destination docset');
  fail(typeof t.page==='string'&&t.page.length>=6&&t.page.length<=255&&t.page.normalize('NFC')===t.page&&/^[^/\\:%?#\s\u0000-\u001f\u007f]+\.html$/u.test(t.page),p,'use a raw flat HTML filename');
  if(t.anchor!==undefined)fail(typeof t.anchor==='string'&&t.anchor.length>0&&t.anchor.length<=512&&t.anchor.normalize('NFC')===t.anchor&&!/[/\\%#?\s\u0000-\u001f\u007f]/u.test(t.anchor),p,'use a raw anchor');
  if(t.problem!==undefined){shape(t.problem,['kind','note','reportedBy','observedVersion'],p+'/problem',['observedVersion']);fail(['reported-missing','reported-renamed','reported-version-drift'].includes(t.problem.kind),p,'unsupported problem');text(t.problem.note,p);text(t.problem.reportedBy,p,160);if(t.problem.observedVersion!==undefined)id(t.problem.observedVersion,p)}
  if(owner&&t.docset!==owner.docset)fail(false,p,'service destination has foreign ownership',{route,destination:t,owner});
  if(t.docset===docset){
   fail(!t.problem,p,'resolve the local target problem');const target=t.page.slice(0,-5)+'.md'+(t.anchor?'#'+encodeURIComponent(t.anchor):'');
   let to;try{to=resolve(target,p,nav.byFile)}catch(error){throw Object.assign(error,{file:'routes.json',details:{route,destination:t}})}
   if(owner&&nav.owners.get(to.file)!==owner.service){
    fail(false,p,'local destination not owned by its service',{route,destination:t,owner,conflict:navigationOwner(to.file),
     nextAction:'Use a target on a page owned by the named service in navigation.json.'});
   }
   return {...to,kind:'LOCAL_DOCSET',docset};
  }
  const d=sets.get(t.docset);return {kind:'REMOTE_DOCSET',docset:t.docset,href:baseURL(d.baseURL,p)+encodeURIComponent(t.page)+(t.anchor?'#'+encodeURIComponent(t.anchor):''),problem:t.problem};
 }
 const resolvedServices=[];
 for(const s of services.values()){
  const a={kind:'service',docset:s.docset,service:s.id};const to=destination(s.overview,'/overview',a);
  if(s.docset===docset){const n=nav.services.find(x=>x.id===s.id);fail(!to.fragment&&to.file===n.overview.file,'/overview','local overview differs')}
  resolvedServices.push({...s,to});
 }
 unique(list(value.routes,'/routes',0,128),'/routes');
 const canonical=new Set(),canonicalOwners=new Map();
 const legacyClaim=(route,index,key='target')=>({file:'navigation.json',path:`/routes/${index}/${key}`,route:route.id,target:route.target});
 function claimCanonical(key,path,claim,message,nextAction){
  if(canonicalOwners.has(key))fail(false,path,message,{...claim,conflict:canonicalOwners.get(key),nextAction});
  canonicalOwners.set(key,{...claim,file:'routes.json',path});
 }
 const resolved=[];
 for(const [ri,r] of value.routes.entries()){
  const p=`/routes/${ri}`;shape(r,['id','version','title','narrative','origin','occurrences','branches','outcomes','coverage'],p);id(r.version,p);text(r.title,p,160);
  const claim={route:r.id,destination:r.narrative},narrativePath=p+'/narrative';
  const idCollision=nav.routes.findIndex(x=>x.id===r.id);
  if(idCollision>=0)fail(false,p+'/id','legacy route ID collision',{...claim,conflict:legacyClaim(nav.routes[idCollision],idCollision,'id'),
   nextAction:'Declare each catalog route in routes.json and remove any duplicate declaration from navigation.json.'});
  const to=destination(r.narrative,narrativePath,null,r.id),key=r.narrative.docset+'/'+r.narrative.page;
  claimCanonical(key,narrativePath,claim,'duplicate canonical narrative page',
   'Give each distinct route its own narrative page. Different anchors on the same page share one canonical owner.');
  if(to.kind==='LOCAL_DOCSET'){
   const index=nav.routes.findIndex(x=>x.to.file===to.file);
   const conflict=navigationOwner(to.file)??(index<0?null:legacyClaim(nav.routes[index],index));
   if(conflict)fail(false,narrativePath,'shared narrative ownership collision',{...claim,conflict,
    nextAction:'Use a dedicated narrative page whose ownership is declared in routes.json alone.'});
   canonical.add(to.file);
  }
  const occurrences=unique(list(r.occurrences,p+'/occurrences',0,128),p+'/occurrences');const occurrenceRows=[];
  for(const [i,o] of r.occurrences.entries()){
   const path=`${p}/occurrences/${i}`;shape(o,['id','actor','role','target'],path);actor(o.actor,path+'/actor');text(o.role,path+'/role');
   occurrenceRows.push({...o,to:destination(o.target,path+'/target',o.actor.kind==='service'?o.actor:null,r.id)});
  }
  fail(r.origin&&typeof r.origin==='object',p+'/origin','expected tagged origin');
  if(r.origin.status==='known'){
   shape(r.origin,['status','occurrence','kind','title','summary'],p+'/origin');const o=occurrences.get(r.origin.occurrence);
   if(o?.actor.kind!=='service')fail(false,p+'/origin/occurrence','origin must name a service occurrence',{route:r.id,occurrence:r.origin.occurrence,
    nextAction:'Use the ID of a documented service occurrence, or provide an unknown origin with a source-bounded reason.'});
   fail(['http','kafka','scheduled','background','manual','other'].includes(r.origin.kind),p+'/origin/kind','invalid trigger kind');text(r.origin.title,p,160);text(r.origin.summary,p);
   if(o.actor.docset===docset){
    const serviceIndex=nav.services.findIndex(x=>x.id===o.actor.service),{starts}=nav.services[serviceIndex];
    const occurrenceIndex=occurrenceRows.findIndex(x=>x.id===o.id),target=occurrenceRows[occurrenceIndex].to;
    if(!starts.some(s=>s.kind===r.origin.kind&&s.to.file===target.file&&s.to.fragment===target.fragment)){
     const choices=starts.map((s,index)=>({path:`/services/${serviceIndex}/starts/${index}`,kind:s.kind,target:s.target,file:s.to.file}));
     const onPage=choices.filter(s=>s.file===target.file);
     fail(false,p+'/origin','local origin is not a documented start',{route:r.id,occurrence:o.id,owner:o.actor,kind:r.origin.kind,
      destination:o.target,occurrencePath:`${p}/occurrences/${occurrenceIndex}/target`,
      navigation:{file:'navigation.json',path:`/services/${serviceIndex}/starts`,starts:(onPage.length?onPage:choices).slice(0,8).map(({path,kind,target})=>({path,kind,target}))},
      nextAction:'Match the occurrence page and anchor and the origin kind to a documented start owned by that service.'});
    }
   }
  }else{shape(r.origin,['status','reason'],p+'/origin');fail(r.origin.status==='unknown',p,'invalid origin status');text(r.origin.reason,p)}
  const branches=unique(list(r.branches,p+'/branches',0,64),p+'/branches');
  for(const b of branches.values()){shape(b,['id','title','condition','target'],p+'/branch');text(b.title,p,160);text(b.condition,p);destination(b.target,p+'/branch/target')}
  unique(list(r.outcomes,p+'/outcomes',0,128),p+'/outcomes');
  for(const o of r.outcomes){
   shape(o,['id','branch','kind','title','target','internal','end'],p+'/outcome');fail(branches.has(o.branch),p,'unknown branch');fail(['normal','error','handoff','unknown'].includes(o.kind),p,'invalid outcome kind');text(o.title,p,160);destination(o.target,p+'/outcome/target');
   shape(o.internal,['coverage','occurrences'],p+'/internal');fail(['complete-for-this-outcome','partial','unknown'].includes(o.internal.coverage),p,'invalid internal coverage');const ids=list(o.internal.occurrences,p+'/internal',0,128);fail(new Set(ids).size===ids.length&&ids.every(x=>occurrences.has(x)),p,'duplicate or unknown internal occurrence');fail(r.origin.status!=='known'||!ids.includes(r.origin.occurrence),p,'origin occurrence is not internal');
   fail(o.end&&typeof o.end==='object',p+'/end','expected tagged end');
   if(o.end.status==='known'){shape(o.end,['status','ownerOccurrence','boundary','recipient'],p+'/end');fail(occurrences.has(o.end.ownerOccurrence)&&!ids.includes(o.end.ownerOccurrence),p,'unknown or internal terminal occurrence');fail(['return','publish','handoff','stop','other'].includes(o.end.boundary),p,'invalid end boundary');actor(o.end.recipient,p,true)}
   else{shape(o.end,['status','reason'],p+'/end');fail(o.end.status==='unknown',p,'invalid end status');text(o.end.reason,p)}
  }
  shape(r.coverage,['outcomeInventory','scope','reference'],p+'/coverage');fail(['complete-for-declared-scope','partial','unknown'].includes(r.coverage.outcomeInventory),p,'invalid inventory coverage');fail(r.coverage.outcomeInventory!=='complete-for-declared-scope'||r.outcomes.length>0,p,'complete inventory requires outcomes');text(r.coverage.scope,p);destination(r.coverage.reference,p+'/scope');
  resolved.push({...r,to,occurrences:occurrenceRows});
 }
 let catalog=null;if(value.catalog!==undefined){catalog=destination(value.catalog,'/catalog');if(catalog.kind==='LOCAL_DOCSET')fail(!nav.owners.has(catalog.file),'/catalog','catalog cannot be service-owned')}
 let semantic=null;
 if(value.semanticCatalog!==undefined){
  const raw=value.semanticCatalog;shape(raw,['id','revision','records'],'/semanticCatalog');id(raw.id,'/semanticCatalog/id');id(raw.revision,'/semanticCatalog/revision');
  const records=unique(list(raw.records,'/semanticCatalog/records',0,128),'/semanticCatalog/records');
  const aliases=new Set(),resolvedRecords=[];
  const owner=(v,p)=>{shape(v,['docset','service'],p);fail(services.has(serviceKey(v)),p,'unknown service namespace')};
  for(const r of records.values()){
   const p='/semanticCatalog/'+r.id;shape(r,['id','kind','title','target','aliases','uses','scope','represents','resolvedService'],p,['scope','represents','resolvedService']);
   fail(['external','concept','local','representation'].includes(r.kind),p,'unsupported semantic kind');text(r.title,p,160);
   fail((r.kind==='local'||r.kind==='representation')===Object.hasOwn(r,'scope'),p,'local and representation records require explicit service scope');
   if(r.scope)owner(r.scope,p+'/scope');
   if(r.kind==='representation')fail(records.has(r.represents)&&records.get(r.represents).kind==='concept',p,'representation must reference a concept');
   else fail(!Object.hasOwn(r,'represents'),p,'only a representation has represents');
   const to=destination(r.target,p+'/target',r.scope),key=r.target.docset+'/'+r.target.page;
   claimCanonical(key,p+'/target',{record:r.id,destination:r.target},'duplicate canonical page','Give each canonical record or route its own page.');
   if(to.kind==='LOCAL_DOCSET'){
    fail(!nav.routes.some(route=>route.to.file===to.file),p,'semantic page cannot be a process narrative');
    fail(r.scope?nav.owners.get(to.file)===r.scope.service:!nav.owners.has(to.file),p,'shared catalog page cannot be service-owned');canonical.add(to.file);
   }
   for(const a of list(r.aliases,p+'/aliases',0,128)){
    shape(a,['input','docset','localId','service','work','reviewRun'],p+'/alias',['service','work','reviewRun']);id(a.input,p);id(a.docset,p);text(a.localId,p,512);fail(sets.has(a.docset),p,'unknown alias docset');
    if(a.service!==undefined)owner({docset:a.docset,service:a.service},p+'/alias');
    if(a.work!==undefined||a.reviewRun!==undefined)fail(['concept','local','representation'].includes(r.kind)&&typeof a.service==='string'&&/^[0-9a-f]{64}$/.test(a.work)&&/^[0-9a-f]{32}$/.test(a.reviewRun),p,'saved glossary alias requires exact Work/review/service scope');
    const k=JSON.stringify([a.work!==undefined?'glossary':r.kind==='external'?'external':r.kind==='representation'?'representation':'term',a.input,a.localId,a.service??null,a.work??null,a.reviewRun??null]);
    fail(!aliases.has(k),p,'local alias maps to several canonical records');aliases.add(k);
   }
   const uses=[],seenUses=new Set();
   for(const u of list(r.uses,p+'/uses',0,256)){
    shape(u,['docset','service','title','target','process'],p+'/use',['process']);owner({docset:u.docset,service:u.service},p+'/use');text(u.title,p,160);
    const target=destination(u.target,p+'/use/target',u),usageKey=JSON.stringify([u.docset,u.service,target.href]);fail(!seenUses.has(usageKey),p,'duplicate local use destination');seenUses.add(usageKey);
    const process=u.process===undefined?null:destination(u.process,p+'/use/process');
    uses.push({...u,to:target,processTo:process});
   }
   if(r.scope)fail(uses.every(u=>u.docset===r.scope.docset&&u.service===r.scope.service),p,'scoped term has foreign uses');
   let record={...r,to,uses};
   if(r.resolvedService!==undefined){
    const target=r.resolvedService,path=p+'/resolvedService';
    shape(target,['docset','service'],path);
    fail(r.kind==='external'&&r.aliases.some(a=>a.docset===target.docset&&a.localId===target.service),path,'expected an exact external alias');
    const service=resolvedServices.find(s=>s.docset===target.docset&&s.id===target.service);
    fail(service,path,'unknown promoted service');
    record={...record,to:service.to,supplementalTo:to};
   }
   resolvedRecords.push(record);
  }
  semantic={...raw,records:resolvedRecords};
 }
 for(const a of actors.values())if(a.canonical!==undefined){
  fail(semantic?.records.some(r=>r.id===a.canonical&&r.kind==='external'),'/externalActors','canonical actor must name an explicit external system');
 }
 return {value,docset,version,services:resolvedServices,routes:resolved,canonical,catalog,...(semantic?{semantic}: {})};
}

// Self-contained so exactly the same pure evaluator runs in Node and the reader.
export function filterRoutes(registry,query={}){
 if(!query||typeof query!=='object'||Array.isArray(query)||Object.keys(query).some(k=>!['start','end','intermediate','outcomeKinds'].includes(k)))throw Error('Unknown filter property');
 const key=a=>{if(typeof a==='string')return a;if(!a||typeof a!=='object'||Array.isArray(a)||a.kind!=='service'||typeof a.docset!=='string'||typeof a.service!=='string'||Object.keys(a).some(k=>!['kind','docset','service'].includes(k)))throw Error('Expected service filter identity');return a.docset+'/'+a.service};
 const available=new Set(registry.docsets.flatMap(d=>d.services.map(s=>d.id+'/'+s.id)));
 const selected={};for(const field of ['start','end','intermediate']){const values=Object.hasOwn(query,field)?query[field]:[];if(!Array.isArray(values)||values.some(v=>!available.has(key(v))))throw Error('Unknown filter service namespace');selected[field]=new Set(values.map(key))}
 const kinds=Object.hasOwn(query,'outcomeKinds')?query.outcomeKinds:[];if(!Array.isArray(kinds)||kinds.some(k=>!['normal','error','handoff','unknown'].includes(k)))throw Error('Unknown outcome filter');
 const test=(actor,set)=>!set.size?1:actor?.kind==='service'?(set.has(key(actor))?1:-1):actor?.kind==='external'?-1:0;
 const name=n=>n===1?'MATCH':n===-1?'NO_MATCH':'UNCLASSIFIED';
 return registry.routes.map(route=>{
  const occurrences=new Map(route.occurrences.map(o=>[o.id,o]));const origin=route.origin.status==='known'?occurrences.get(route.origin.occurrence)?.actor:null;
  const start=test(origin,selected.start),matches=[],unknown=[];
  if(!selected.end.size&&!selected.intermediate.size&&!kinds.length)return {id:route.id,status:name(start),matches,unknown};
  let result=-1;
  for(const outcome of route.outcomes){
   const end=outcome.end.status==='known'?test(occurrences.get(outcome.end.ownerOccurrence)?.actor,selected.end):selected.end.size?0:1;
   let middle=1;if(selected.intermediate.size)middle=outcome.internal.occurrences.some(id=>test(occurrences.get(id)?.actor,selected.intermediate)===1)?1:outcome.internal.coverage==='complete-for-this-outcome'?-1:0;
   const kind=!kinds.length||kinds.includes(outcome.kind)?1:outcome.kind==='unknown'?0:-1;
   const state=Math.min(start,end,middle,kind);result=Math.max(result,state);const witness={branch:outcome.branch,outcome:outcome.id};if(state===1)matches.push(witness);if(state===0)unknown.push(witness);
  }
  if(result!==1&&route.coverage.outcomeInventory!=='complete-for-declared-scope'&&start!==-1)result=0;
  return {id:route.id,status:name(result),matches,unknown};
 });
}
