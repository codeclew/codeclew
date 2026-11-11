import {filterRoutes,serviceKey} from './docset-routes.mjs';
const esc=s=>String(s).replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('"','&quot;');
export const routeWords={catalog:'Services and processes',filters:'Filter processes',services:'Services',processes:'Processes',start:'Starts in',end:'Ends in',intermediate:'Internal service',kinds:'Outcome',normal:'Normal',error:'Error',handoff:'Handoff',unknown:'Unclassified',whole:'Whole process',ownPart:'This service’s part',origin:'Documented start',outcomes:'Described outcomes',scope:'Described scope',partial:'Some outcomes or roles remain unclassified',other:'Other documentation',external:'External actor',declared:'Navigation declarations; remote pages and anchors are not checked.',help:'Leave a filter empty for all services. Values within a filter use OR; filters combine with AND in one outcome.',include:'Include unclassified results',clear:'Clear filters',matches:'Matches',negative:'No match in the described scope',unclassified:'Needs classification',none:'No matching processes in the described scope.',uncertain:'No confirmed match; unclassified processes remain.',nojs:'Filters need JavaScript. All declared processes and links are shown.',recipient:'Recipient',noneRecipient:'None',unknownEnd:'End not classified',matchingOutcomes:'Matching outcomes',more:'more',uncertainMatches:'Uncertain for these filters',classificationGaps:'Processes with classification gaps'};
export const routeStyle=`.route-catalog,.route-context{margin:1.3rem 0}.route-service-list{display:flex;flex-wrap:wrap;gap:.5rem 1.4rem;list-style:none;padding:0}.route-card h3,.markdown-reader .reader-guide .route-card h3{margin:0 0 .7rem;font-size:1.05rem}.route-filter-panel{margin:1rem 0}.route-filters{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:.8rem;margin-top:1rem}.route-facet{min-width:0;border:1px solid #ccd5df;padding:.7rem}.route-facet legend{font-weight:600}.route-options{max-height:12rem;overflow:auto}.route-filters label{display:flex;align-items:flex-start;gap:.4rem;margin:.4rem 0;overflow-wrap:anywhere}.route-filters input{flex:none;margin-top:.3rem}.route-actions{grid-column:1/-1;display:flex;flex-wrap:wrap;align-items:center;gap:1rem}.route-cards{display:grid;gap:1rem}.route-card{border:1px solid #ccd5df;border-radius:.6rem;padding:1rem;min-width:0}.route-card [hidden],.route-card[hidden]{display:none}.route-state,.route-gap{font-size:.9em}.route-link-unavailable{color:#5d3740}.route-validation{font-size:.9em}.route-context ul,.route-card ul{padding-left:1.3rem}@media(max-width:850px){.route-filters{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:500px){.route-filters{grid-template-columns:1fr}}`;
function gap(r){return r.origin.status==='unknown'||r.coverage.outcomeInventory!=='complete-for-declared-scope'||r.outcomes.some(o=>o.kind==='unknown'||o.end.status==='unknown'||o.internal.coverage!=='complete-for-this-outcome')}
export function docsetView(model,file,words=routeWords,prefix='clew-route'){
 const registry=model.value,sets=new Map(registry.docsets.map(d=>[d.id,d])),services=new Map(model.services.map(s=>[s.docset+'/'+s.id,s]));
 const titleCounts=new Map();for(const s of model.services)titleCounts.set(s.title,(titleCounts.get(s.title)||0)+1);
 const serviceTitle=s=>s.title+(titleCounts.get(s.title)>1?' · '+sets.get(s.docset).title:'');
 function link(to,title){
  if(to.problem)return `<span class="route-link-unavailable">${esc(title)} — ${esc(to.problem.note)}</span>`;
  const remote=to.kind==='REMOTE_DOCSET';return `<a href="${esc(to.href)}"${remote?` data-docset="${esc(to.docset)}" aria-label="${esc(title)} (${esc(words.other)}: ${esc(sets.get(to.docset).title)})"`:` data-page="${esc(to.file)}" data-fragment="${esc(to.fragment)}"${to.file===file&&!to.fragment?' aria-current="page"':''}`}>${esc(title)}</a>`;
 }
 const actorTitle=a=>a.kind==='service'?serviceTitle(services.get(serviceKey(a))):a.kind==='external'?registry.externalActors.find(x=>x.id===a.id).title:a.kind==='none'?words.noneRecipient:words.unknown;
 const actorTarget=a=>a.kind==='external'?model.semantic?.records.find(r=>r.id===registry.externalActors.find(x=>x.id===a.id)?.canonical)?.to:null;
 const actorDisplay=a=>actorTarget(a)?link(actorTarget(a),actorTitle(a)):esc(actorTitle(a));
 const origin=r=>{if(r.origin.status!=='known')return esc(words.unknown);const o=r.occurrences.find(o=>o.id===r.origin.occurrence);return esc(words.triggerLabels?.[r.origin.kind]||r.origin.kind)+' · '+link(o.to,actorTitle(o.actor)+' · '+r.origin.title)};
 const roles=r=>r.occurrences.map(o=>`<li>${link(o.to,actorTitle(o.actor))} — ${esc(o.role)}${o.actor.kind==='service'?` · ${link(services.get(serviceKey(o.actor)).to,words.services)}`:` · ${actorTarget(o.actor)?link(actorTarget(o.actor),words.external):esc(words.external)}`}</li>`).join('');
 const ends=r=>r.outcomes.map(o=>{const owner=o.end.status==='known'?actorDisplay(r.occurrences.find(x=>x.id===o.end.ownerOccurrence).actor):esc(words.unknownEnd);return `<li>${esc(o.title)} (${esc(words[o.kind])}): ${owner}${o.end.status==='known'?` · ${esc(words.recipient)}: ${actorDisplay(o.end.recipient)}`:''}</li>`}).join('');
 const endSummary=r=>{
  const values=[...new Set(r.outcomes.map(o=>words[o.kind]+': '+(o.end.status==='known'?actorTitle(r.occurrences.find(x=>x.id===o.end.ownerOccurrence).actor):words.unknownEnd)))];
  return values.slice(0,4).map(esc).join('; ')+(values.length>4?' · +'+(values.length-4)+' '+esc(words.more):'')||esc(words.unknownEnd);
 };
 const localRoles=(r,service)=>r.occurrences.filter(o=>o.actor.kind==='service'&&o.actor.docset===model.docset&&o.actor.service===service);
 const relevant=r=>r.narrative.docset===model.docset||r.occurrences.some(o=>o.actor.kind==='service'&&o.actor.docset===model.docset);
 const semanticTitle=r=>r.title+(r.scope?' · '+serviceTitle(services.get(serviceKey(r.scope))):'');
 const extra={canonical:model.canonical,routes:model.routes.filter(relevant).map(r=>link(r.to,r.title)),participation:service=>model.routes.filter(r=>localRoles(r,service).length).map(r=>link(r.to,r.title))};
 if(model.catalog&&!(model.semantic&&!model.routes.length)&&!(model.catalog.kind==='LOCAL_DOCSET'&&model.catalog.file===file))extra.routes.unshift(link(model.catalog,words.catalog));
 let hub='';
 if(model.semantic){
  const title=words.sharedCatalog||'Shared catalog',usedBy=words.usedBy||'Used by',localDetails=words.localDetails||'Local details';
  const byId=new Map(model.semantic.records.map(r=>[r.id,r])),representations=new Map();
  for(const r of model.semantic.records)if(r.kind==='representation'){
   if(!representations.has(r.represents))representations.set(r.represents,[]);representations.get(r.represents).push(r);
  }
  extra.semanticTitle=title;extra.semanticPages=new Set(model.semantic.records.filter(r=>r.to.kind==='LOCAL_DOCSET').map(r=>r.to.file));
  extra.semanticCatalogHome=model.catalog?.kind==='LOCAL_DOCSET'?model.catalog.file:undefined;
  extra.semanticLinks=model.semantic.records.filter(r=>model.catalog?.docset===model.docset||r.target.docset===model.docset||r.uses.some(u=>u.docset===model.docset)).map(r=>link(r.to,semanticTitle(r)));
  if(!model.routes.length&&model.catalog?.kind==='REMOTE_DOCSET')extra.semanticLinks.unshift(link(model.catalog,title));
  for(const r of model.semantic.records){
   const own=r.to.kind==='LOCAL_DOCSET'&&r.to.file===file;
   const relevant=r.uses.filter(u=>u.to.kind==='LOCAL_DOCSET'&&u.to.file===file||u.docset===model.docset&&services.get(serviceKey(u)).to.file===file);
   if(own){
    const details=r.uses.map(u=>`<li>${link(u.to,serviceTitle(services.get(serviceKey(u)))+' · '+u.title)}${u.processTo?` · ${link(u.processTo,words.whole)}`:''}</li>`).join('');
    const related=r.kind==='representation'?`<p>${esc(words.representationOf||'Representation of')}: ${link(byId.get(r.represents).to,semanticTitle(byId.get(r.represents)))}</p>`:representations.has(r.id)?`<p>${esc(words.representations||'Representations')}: ${representations.get(r.id).map(x=>link(x.to,semanticTitle(x))).join('; ')}</p>`:'';
    hub+=`<section class="route-context semantic-context">${related}<h2>${esc(usedBy)}</h2>${details?`<ul>${details}</ul>`:`<p>${esc(words.noDeclaredUses||'No declared service uses in this collection.')}</p>`}</section>`;
   }else if(relevant.length){
    hub+=`<section class="route-context semantic-context"><h2>${link(r.to,semanticTitle(r))}</h2><ul>${relevant.map(u=>`<li>${link(u.to,localDetails+' · '+u.title)}</li>`).join('')}</ul></section>`;
   }
  }
 }
 for(const r of model.routes){
  const own=r.to.kind==='LOCAL_DOCSET'&&r.to.file===file;
  const participation=r.occurrences.filter(o=>o.to.kind==='LOCAL_DOCSET'&&o.to.file===file||o.actor.kind==='service'&&o.actor.docset===model.docset&&services.get(serviceKey(o.actor)).to.file===file);
  if(!own&&!participation.length)continue;
  hub+=`<section class="route-context"><h2>${link(r.to,r.title)}</h2><p>${esc(words.origin)}: ${origin(r)}</p>${own?`<ul>${roles(r)}</ul><h3>${esc(words.outcomes)}</h3><ul>${ends(r)}</ul>`:`<p>${link(r.to,words.whole)}</p><ul>${participation.map(o=>`<li>${link(o.to,words.ownPart)} — ${esc(o.role)}</li>`).join('')}</ul>`}${model.catalog?`<p>${link(model.catalog,words.catalog)}</p>`:''}</section>`;
 }
 let catalog='',script='';
 if(model.catalog?.kind==='LOCAL_DOCSET'&&model.catalog.file===file){
  const select=(field,label,options)=>`<fieldset class="route-facet"><legend>${esc(label)}</legend><div class="route-options">${options.map(([v,t])=>`<label><input type="checkbox" name="${field}" value="${esc(v)}"><span>${esc(t)}</span></label>`).join('')}</div></fieldset>`;
  const choices=model.services.map(s=>[s.docset+'/'+s.id,serviceTitle(s)]);
  catalog=`<section class="route-catalog" id="${prefix}-catalog"><h2>${esc(words.services)}</h2><ul class="route-service-list">${model.services.map(s=>`<li>${link(s.to,serviceTitle(s))}</li>`).join('')}</ul><h2>${esc(words.processes)}</h2>${model.routes.some(gap)?`<p>${esc(words.classificationGaps)}: ${model.routes.filter(gap).length}</p>`:''}<p id="${prefix}-help">${esc(words.help)}</p><details class="route-filter-panel" open><summary>${esc(words.filters)}</summary><form class="route-filters" aria-label="${esc(words.filters)}" aria-describedby="${prefix}-help">${select('start',words.start,choices)}${select('end',words.end,choices)}${select('intermediate',words.intermediate,choices)}${select('outcomeKinds',words.kinds,['normal','error','handoff','unknown'].map(k=>[k,words[k]]))}<div class="route-actions"><label class="route-include"><input type="checkbox" name="includeUnknown" checked>${esc(words.include)}</label><button type="reset">${esc(words.clear)}</button></div></form></details><p class="route-count" role="status" aria-live="polite"></p><p class="route-empty" hidden></p><noscript>${esc(words.nojs)}</noscript><div class="route-cards">${model.routes.map(r=>`<article class="route-card" data-route-id="${esc(r.id)}"><h3>${link(r.to,r.title)}</h3><p>${esc(words.origin)}: ${origin(r)}</p><p>${esc(words.end)}: ${endSummary(r)}</p>${gap(r)?`<p class="route-gap">${esc(words.partial)}</p>`:''}<p class="route-state"></p><p class="route-matches" hidden></p><details><summary>${esc(words.outcomes)}</summary><ul>${ends(r)}</ul><ul>${roles(r)}</ul><p>${esc(words.scope)}: ${esc(r.coverage.scope)}</p></details></article>`).join('')}</div><p class="route-validation">${esc(words.declared)}</p></section>`;
  const data=JSON.stringify({registry,words}).replaceAll('<','\\u003c').replaceAll('\u2028','\\u2028').replaceAll('\u2029','\\u2029');
  script=`<script type="application/json" id="${prefix}-data">${data}</script><script>(()=>{const evaluate=(${filterRoutes.toString()});(${catalogClient.toString()})(evaluate,${JSON.stringify(prefix)})})()</script>`;
  if(model.semantic){
   const group=(title,rows)=>rows.length?`<h2>${esc(title)}</h2><ul class="route-service-list">${rows.map(r=>`<li>${link(r.to,semanticTitle(r))}</li>`).join('')}</ul>`:'';
   const semantic=group(words.externalSystems||'External systems',model.semantic.records.filter(r=>r.kind==='external'))+group(words.terms||'Terms',model.semantic.records.filter(r=>r.kind!=='external'));
   if(!model.routes.length){
    catalog=`<section class="route-catalog" id="${prefix}-catalog"><h2>${esc(words.services)}</h2><ul class="route-service-list">${model.services.map(s=>`<li>${link(s.to,serviceTitle(s))}</li>`).join('')}</ul>${semantic}</section>`;script='';
   }else catalog+=`<section class="route-catalog semantic-catalog">${semantic}</section>`;
  }
 }
 return {extra,hub:hub+catalog,script};
}
function catalogClient(evaluate,prefix){
 const root=document.getElementById(prefix+'-catalog'),data=JSON.parse(document.getElementById(prefix+'-data').textContent),form=root.querySelector('form');
 const narrow=matchMedia('(max-width:700px)'),panel=root.querySelector('.route-filter-panel');const resize=()=>{panel.open=!narrow.matches};resize();narrow.addEventListener('change',resize);
 const cards=new Map([...root.querySelectorAll('[data-route-id]')].map(e=>[e.dataset.routeId,e]));
 const titles=new Map(data.registry.routes.map(r=>[r.id,new Map(r.outcomes.map(o=>[o.id,o.title]))]));
 function apply(){
  const query={};for(const field of ['start','end','intermediate','outcomeKinds'])query[field]=[...form.querySelectorAll('input[name="'+field+'"]:checked')].map(o=>o.value);
  const active=Object.values(query).some(values=>values.length);
  const results=evaluate(data.registry,query),showUnknown=form.elements.namedItem('includeUnknown').checked;
  let matches=0,unknown=0,negative=0,visible=0;
  for(const row of results){
   const card=cards.get(row.id);if(row.status==='MATCH')matches++;else if(row.status==='UNCLASSIFIED')unknown++;else negative++;
   card.hidden=row.status==='NO_MATCH'||row.status==='UNCLASSIFIED'&&!showUnknown;if(!card.hidden)visible++;
   card.querySelector('.route-state').hidden=!active;card.querySelector('.route-state').textContent=row.status==='MATCH'?data.words.matches:row.status==='UNCLASSIFIED'?data.words.unclassified:data.words.negative;
   const outcome=card.querySelector('.route-matches');outcome.hidden=!row.matches.length;outcome.textContent=row.matches.length?data.words.matchingOutcomes+': '+row.matches.map(m=>titles.get(row.id).get(m.outcome)).join('; '):'';
  }
  root.querySelector('.route-count').textContent=data.words.matches+': '+matches+'. '+data.words.uncertainMatches+': '+unknown+'. '+data.words.negative+': '+negative+'.';
  const empty=root.querySelector('.route-empty');empty.hidden=visible>0;empty.textContent=unknown?data.words.uncertain:data.words.none;
 }
 form.addEventListener('submit',event=>event.preventDefault());form.addEventListener('change',apply);form.addEventListener('reset',()=>setTimeout(apply,0));apply();
}
