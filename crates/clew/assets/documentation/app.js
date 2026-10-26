'use strict';
const publication=JSON.parse(document.getElementById('document-data').textContent);

// Explicit publication locale. The English source strings are the English dictionary.
// Translate only trusted renderer literals, never interpolated author text or code.
const language=publication.language==='ru'?'ru':'en';
const {t,chromeHtml}=clewLocale(language,CLEW_RU_MESSAGES);
function statusLabel(value){return t(value);}
function meaningReviewLabel(value){return value==='VERIFIED'?t('Model approved'):value==='VERIFIED_WITH_LIMITATIONS'?t('Model approved with limitations'):statusLabel(value);}
const explicitLanguage=['en','ru'].includes(publication.requestedDocumentationLanguage);
const translationGaps=explicitLanguage&&publication.proseDisplayPolicy!=='READABLE_ORIGINALS'?(publication.translationGaps||{}):{};
const sectionTitles={'section-overview':language==='ru'?'Обзор':'Overview','section-responsibilities':t('Responsibilities'),'section-entities':t('Domain entities'),'section-ingress':t('Entry points'),'section-egress':t('External calls and storage')};
// Keep the embedded publication immutable: accepted digests refer to its exact content.
// The normal publication view excludes mismatched prose; full Pages retains
// readable originals with separate language notices and unchanged review states.
const D={...publication,operations:publication.operations.filter(op=>!translationGaps[op.id]),sections:(publication.sections||[]).map(section=>translationGaps[section.id]?{...section,title:sectionTitles[section.id]||section.id,content:null,gap:t('The description is not yet available in the selected language.')}:section)};

const $=id=>document.getElementById(id);
const esc=v=>String(v??'').replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
function translationNotice(id){
 const gap=translationGaps[id];if(!gap)return '';
 const href=typeof gap.href==='string'&&!/^(?:[a-z][a-z0-9+.-]*:|\/\/)/i.test(gap.href)?gap.href:null;
 const label=gap.availableLanguage==='en'?t('English version'):gap.availableLanguage==='ru'?t('Russian version'):t('Original version');
 return `<aside class="gap-card translation-gap" data-translation-gap="${esc(id)}"><h3>${esc(t('Translation pending'))}</h3><p>${esc(t('The description is not yet available in the selected language.'))}</p><p>${esc(t('The accepted text and diagrams belong to another language or have no recorded language. They are hidden here until a matching version is accepted.'))}</p>${href?`<p><a href="${esc(href)}">${esc(label)} ↗</a></p>`:''}</aside>`;
}
const prose=v=>typeof clewMarkdown==='object'?clewMarkdown.render(v,{tableLabel:t('Table')}):'<p>'+esc(v)+'</p>';
const pretty=v=>esc(JSON.stringify(v,null,2));
const sourceRecords=()=>sourceFragment?(sourceFragment.startsWith(D.subject+'/'+sourceOperation+'/')?(D.fragmentSources?.[sourceFragment]||{}):{}):sourceScope==='current'?(D.selectedSources||D.sources):sourceOperation==='process-outline'?(D.processOutlineSources||{}):sourceOperation?(D.operationSources?.[sourceOperation]||{}):!$('scenario-content').hidden&&(D.operationSources?.[sourceOperation||current?.visualOwner||current?.id])||D.sources;
const button=(ids,label=t('Source'),currentSnapshot=false,owner='',sourcePath=[],claim='')=>ids?.length?chromeHtml`<button class="source-link" ${currentSnapshot?'data-current-sources':''} ${owner?`data-source-operation="${esc(owner)}"`:''} data-sources="${esc(ids.join(' '))}" ${claim?`data-source-claim="${esc(claim)}"`:""} ${sourcePath.length?`data-source-path="${esc(JSON.stringify(sourcePath))}"`:''}>${esc(label)} ↗</button>`:'';
const claimButton=(claim,label=t('Source'),currentSnapshot=false,owner='',sourcePath=[])=>button(claim?.sourceIds,label,currentSnapshot,owner,sourcePath,claim?.id||'');
// Typed artifacts are interpretations bound to their containing accepted operation.
// IDs encode each code point so user identifiers cannot collide or inject markup.
const visualToken=value=>Array.from(String(value),c=>c.codePointAt(0).toString(16)).join('_');
const visualKey=(owner,id)=>chromeHtml`visual-${visualToken(owner)}--${visualToken(id)}`;
const visualRows=()=>D.operations.flatMap(owner=>(owner.visuals||[]).map(visual=>({owner,visual})));
const visualKind=kind=>({'execution-flow':t('Execution flow'),'dependency-map':t('Dependency map'),'decision-table':t('Decision table')}[kind]||t('Visual artifact'));
const visualNodeKey=(owner,visual,node)=>chromeHtml`${visualKey(owner,visual)}-node-${visualToken(node)}`;
const visualLink=(owner,visual,text,node)=>chromeHtml`<a href="#${visualKey(owner,visual)}" data-visual-target="${visualKey(owner,visual)}" ${node?chromeHtml`data-visual-node-target="${visualNodeKey(owner,visual,node)}"`:''}>${esc(text)}</a>`;
function visualFragment(owner,fragment){
 if(!fragment)return '';
 return chromeHtml`<span>${esc(fragment.text)}</span>${fragment.sourceIds?.length?chromeHtml` <button class="source-link" data-sources="${esc(fragment.sourceIds.join(' '))}" data-source-operation="${esc(owner.id)}">Supporting code ↗</button>`:''}`;
}
function visualStatus(owner){
 const state=D.operationStates?.[owner.id];
 return chromeHtml`<p class="visual-status">Source freshness: <b>${esc(statusLabel(state?.freshness||'UNVERIFIED'))}</b> · Meaning review: <b>${esc(meaningReviewLabel(state?.verification||'UNASSESSED'))}</b></p>`;
}
function visualGraph(owner,visual){
 const nodes=visual.nodes||[],edges=visual.edges||[],byId=new Map(nodes.map(n=>[n.id,n]));
 if(!nodes.length)return chromeHtml`<p class="empty-note">No graph nodes were supplied.</p>`;
 // Rank acyclic portions by their actual edges, not declaration order. Cycles
 // remain directed edges; placement itself never claims execution order.
 const incoming=new Map(nodes.map(n=>[n.id,0])),rank=new Map();
 for(const edge of edges)if(byId.has(edge.from)&&byId.has(edge.to)&&edge.from!==edge.to)incoming.set(edge.to,incoming.get(edge.to)+1);
 const queue=nodes.filter(n=>incoming.get(n.id)===0).map(n=>n.id);
 for(const id of queue)rank.set(id,0);
 for(let i=0;i<queue.length;i++)for(const edge of edges.filter(e=>e.from===queue[i]&&e.to!==e.from&&byId.has(e.to))){rank.set(edge.to,Math.max(rank.get(edge.to)||0,rank.get(edge.from)+1));incoming.set(edge.to,incoming.get(edge.to)-1);if(incoming.get(edge.to)===0)queue.push(edge.to);}
 let tail=Math.max(-1,...queue.map(id=>rank.get(id)))+1;
 for(const node of nodes)if(!queue.includes(node.id))rank.set(node.id,tail++);
 const rows=new Map();for(const node of nodes){const r=rank.get(node.id);if(!rows.has(r))rows.set(r,[]);rows.get(r).push(node);}
 const width=Math.max(760,Math.max(...Array.from(rows.values(),r=>r.length))*320+140),positions=new Map(),edgeLabels=new Map();let y=24;
 for(const [rowRank,row] of [...rows].sort((a,b)=>a[0]-b[0])){
  const height=Math.max(...row.map(n=>Math.max(88,wrap(n.meaning.text,32).length*18+44+(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===n.id).length*18)));
  row.forEach((node,i)=>positions.set(node.id,{x:(width-row.length*320)/2+i*320+20,y,w:280,h:height}));
  let gap=26;
  edges.forEach((edge,i)=>{if(rank.get(edge.from)!==rowRank||!(rank.get(edge.to)>rowRank))return;const lines=sequenceWrap(`${i+1}. ${edge.meaning.text}`,30);edgeLabels.set(i,{bus:y+height+gap,lines});gap+=lines.length*15+24;});
  y+=height+Math.max(135,gap+25);
 }
 const marker=visualKey(owner.id,visual.id)+'-arrow';
 let svg=chromeHtml`<svg class="artifact-svg" viewBox="0 0 ${width} ${y}" style="min-width:${Math.min(width,900)}px" role="img" aria-label="${esc(visualKind(visual.kind)+': '+visual.title)}"><title>${esc(visual.title)}</title><desc>Arrows carry the documented relationships. Node position alone does not establish order. A complete node and connection table follows.</desc><defs><marker id="${marker}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#376d83"/></marker></defs>`;
 edges.forEach((edge,i)=>{
  const a=positions.get(edge.from),b=positions.get(edge.to);if(!a||!b)return;
  let path,x,y;
  if(b.y>a.y){const x1=a.x+a.w/2,x2=b.x+b.w/2,bus=edgeLabels.get(i)?.bus??a.y+a.h+35;path=chromeHtml`M${x1} ${a.y+a.h} V${bus} H${x2} V${b.y-3}`;x=(x1+x2)/2;y=bus+16;}
  else{const lane=width-15-(i%4)*16;path=chromeHtml`M${a.x+a.w} ${a.y+a.h/2} H${lane} V${b.y+22} H${b.x+b.w+3}`;x=lane-10;y=(a.y+a.h/2+b.y+22)/2;}
  const shown=edgeLabels.get(i)?.lines||sequenceWrap(`${i+1}. ${edge.meaning.text}`,30);
  svg+=chromeHtml`<g data-artifact-edge="${esc(edge.id)}"><title>${esc(edge.meaning.text)}</title><path d="${path}" class="artifact-edge" marker-end="url(#${marker})"/>${shown.map((line,j)=>chromeHtml`<text class="artifact-edge-label" x="${x}" y="${y+j*15}" text-anchor="${b.y>a.y?'middle':'end'}">${esc(line)}</text>`).join('')}</g>`;
 });
 for(const node of nodes){
  const p=positions.get(node.id),decisions=(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===node.id),nodeId=visualNodeKey(owner.id,visual.id,node.id);
  svg+=chromeHtml`<g id="${nodeId}" tabindex="-1"><title>${esc(node.meaning.text)}</title><rect x="${p.x}" y="${p.y}" width="${p.w}" height="${p.h}" rx="9"/>${wrap(node.meaning.text,32).map((line,i)=>chromeHtml`<text x="${p.x+14}" y="${p.y+25+i*18}">${esc(line)}</text>`).join('')}${decisions.map((decision,i)=>chromeHtml`<a href="#${visualKey(owner.id,decision.id)}" data-visual-target="${visualKey(owner.id,decision.id)}"><title>${esc(decision.title)}</title><text class="artifact-decision-link" x="${p.x+14}" y="${p.y+p.h-12-(decisions.length-1-i)*18}">${esc(decision.title.length>31?decision.title.slice(0,30)+'…':decision.title)} →</text></a>`).join('')}</g>`;
 }
 return chromeHtml`<div class="artifact-graph-scroll">${svg}</svg></div><p class="flow-note">${visual.kind==='dependency-map'?t('Arrows show dependencies, not execution order or runtime calls.'):t('Follow the arrows and their conditions. Placement alone does not establish order; this is a source-based interpretation, not an observed runtime trace.')}</p><details class="artifact-text"><summary>Complete diagram as text and supporting code</summary><h4>Nodes</h4><ul>${nodes.map(n=>chromeHtml`<li>${visualFragment(owner,n.meaning)}${(owner.visuals||[]).filter(v=>v.parent?.artifact===visual.id&&v.parent.node===n.id).map(v=>' · '+visualLink(owner.id,v.id,v.title)).join('')}</li>`).join('')}</ul><h4>Directed connections</h4><ol>${edges.map(e=>chromeHtml`<li><p><b>${esc(byId.get(e.from)?.meaning.text||e.from)} → ${esc(byId.get(e.to)?.meaning.text||e.to)}</b></p>${visualFragment(owner,e.meaning)}</li>`).join('')}</ol></details>`;
}
function visualCard(owner,visual){
 const parent=visual.parent&&(owner.visuals||[]).find(v=>v.id===visual.parent.artifact),node=parent?.nodes?.find(n=>n.id===visual.parent.node);
 const body=visual.kind==='decision-table'?chromeHtml`${parent?chromeHtml`<p class="visual-parent">Used at ${visualLink(owner.id,parent.id,node?.meaning.text||parent.title,visual.parent.node)} in ${esc(parent.title)}.</p>`:chromeHtml`<p class="visual-parent"><b>Local decision; placement in the wider process is not established.</b></p>`}<p class="decision-policy">${visual.hitPolicy==='FIRST'?t('Choose the first matching row, top to bottom. Later matching rows do not contribute another result.'):visual.hitPolicy==='UNIQUE'?t('At most one row may match. Multiple matching rows mean the documented decision rules conflict.'):t('The rule matching policy has not been established. Do not infer priority or exclusivity from row order.')}</p>${visual.policyExplanation?chromeHtml`<p><b>Policy interpretation:</b> ${visualFragment(owner,visual.policyExplanation)}</p>`:''}<p class="flow-note">A row selects a result; this table does not establish when its resulting actions execute.</p><details><summary>Decision notation</summary><p>Hit policy: <b>${esc(visual.hitPolicy||t('unspecified'))}</b>. This is an authored policy interpretation, not a formally verified or executable DMN model.</p></details><div class="table-wrap"><table><thead><tr><th>Rule</th><th>Condition</th><th>Selected result</th></tr></thead><tbody>${(visual.rules||[]).map((r,i)=>chromeHtml`<tr><td>${i+1}</td><td>${visualFragment(owner,r.condition)}</td><td>${visualFragment(owner,r.outcome)}</td></tr>`).join('')}</tbody></table></div>${visual.afterSelection?chromeHtml`<section class="visual-after-selection"><h4>After selection: execution and outcomes</h4><p>${visualFragment(owner,visual.afterSelection)}</p></section>`:''}`:visualGraph(owner,visual);
 return chromeHtml`<article class="visual-artifact" id="${visualKey(owner.id,visual.id)}"><div class="eyebrow">${esc(visualKind(visual.kind))}</div><h3>${esc(visual.title)}</h3>${visualStatus(owner)}<p><b>Why this view:</b> ${visualFragment(owner,visual.purpose)}</p><p><b>Scope:</b> ${visualFragment(owner,visual.scope)}</p>${body}${visual.limitations?.length?chromeHtml`<div class="visual-limits"><h4>Limits of this view</h4><ul>${visual.limitations.map(v=>chromeHtml`<li>${esc(v)}</li>`).join('')}</ul></div>`:''}<details class="technical-evidence"><summary>Artifact binding and producer</summary><p>Accepted with ${esc(owner.title)} (${esc(owner.id)}). Freshness and meaning review belong to this containing accepted version.</p><pre>${pretty({schema:visual.schema,generator:visual.generator,artifact:visual.id,operation:owner.id,state:D.operationStates?.[owner.id]||null})}</pre></details></article>`;
}
function visualGallery(){const rows=visualRows();return rows.length?chromeHtml`<section class="visual-gallery"><h2>Processes and diagrams · ${rows.length}</h2><p>Selected source-based views. Each view records its own purpose, scope and limitations; these are not an exhaustive list of service behavior.</p>${rows.map(({owner,visual})=>visualCard(owner,visual)).join('')}</section>`:'';}
function visualPreview(){const rows=visualRows(),first=rows.find(r=>r.visual.kind!=='decision-table')||rows[0];return first?chromeHtml`<section class="visual-preview"><h3>Processes and diagrams</h3><nav aria-label="Processes and diagrams">${rows.map(({owner,visual})=>visualLink(owner.id,visual.id,visual.title)).join('')}</nav>${visualCard(first.owner,first.visual)}</section>`:'';}

function serviceInventory(){return D.selectedBoundaryInventory??D.boundaryInventory??{};}
function readerEvidenceLegend(){
 return chromeHtml`<details class="reader-evidence-legend"><summary>${esc(t('Source interpretation'))} · ${esc(t('Evidence details'))}</summary><p>${esc(t('Static source interpretation; runtime execution was not observed.'))}</p><p>${esc(t('Unknown is not absence. Hypotheses and conditional behavior remain explicit.'))}</p></details>`;
}
function modelFieldCoverage(model){
 const coverage=model.fieldCoverage;
 return chromeHtml`<details class="field-coverage"><summary>${esc(t(coverage?.status||'Field coverage unknown'))}</summary>${coverage?chromeHtml`<p>${esc(coverage.scope)}</p><div class="clew-markdown">${prose(coverage.claim?.text||'')}</div>`:''}<a href="../data-model/${esc(model.id)}.html">${esc(t('Model details and sources'))} ↗</a></details>`;
}
function fieldBusinessName(field){return field.readableName?.text||field.path;}
function modelFieldRecords(model,subject=D.subject,expandedUsage=false){
 const fields=model.fields||[],table=chromeHtml`<div class="table-wrap field-glossary"><table><thead><tr><th>${esc(t('Business name'))}</th><th>${esc(t('Meaning'))}</th></tr></thead><tbody>${fields.map(field=>chromeHtml`<tr data-domain-field="${esc(field.id)}"><td data-label="${esc(t('Business name'))}"><strong>${esc(fieldBusinessName(field))}</strong></td><td data-label="${esc(t('Meaning'))}">${esc(field.meaning?.text)}</td></tr>`).join('')}</tbody></table></div>`;
 const representations=chromeHtml`<details class="field-representations"><summary>${esc(t('Representation and source details'))}</summary>${fields.map(field=>chromeHtml`<section><h4>${esc(fieldBusinessName(field))}</h4><p><code>${esc(field.path)}</code></p>${(field.representations||[]).map(r=>chromeHtml`<p><code>${esc(r.label)}</code> · ${esc(r.typeLabel)} · ${esc(r.requiredness||'')} · ${esc(r.nullability||'')}</p><div class="clew-markdown">${prose(r.mapping?.text||'')}</div>`).join('')}</section>`).join('')}<a href="../data-model/${esc(model.id)}.html">${esc(t('Model details and sources'))} ↗</a></details>`;
 const usage=fields.map(field=>{
  const uses=(field.usage||[]).filter(u=>!subject?.startsWith('scenario:')||!u.processes?.length||u.processes.includes(subject));
  return chromeHtml`<article class="reader-field"><h4>${esc(fieldBusinessName(field))}</h4>${uses.length?uses.map(u=>chromeHtml`<div class="reader-transformation"><p><b>${esc(t('Input'))}:</b> ${(u.inputs||[]).map(x=>chromeHtml`<code>${esc(x)}</code>`).join(' + ')||esc(t('Not established'))}</p><p><b>${esc(t('Rule and condition'))}:</b> ${esc(u.claim?.text)}<br>${esc(u.condition)}</p><p><b>${esc(t('Output and destination'))}:</b> ${esc(fieldBusinessName(field))} → ${esc(u.consumer)} · ${esc(u.stage)}</p><details class="field-use-evidence"><summary>${esc(t('Evidence and scope'))}</summary><p><code>${esc(u.representation)}</code></p><p>${esc(t(u.assessment))} · ${esc(u.coveredScope)}${u.unresolved?chromeHtml` · ${esc(u.unresolved)}`:''}</p><p>${(u.processes||[]).map(process=>chromeHtml`<a href="../scenarios/${esc(process.replace(/^scenario:/,''))}.html#entity-dataflow">${esc(t('Process'))} ↗</a>`).join(' · ')}</p><a href="../data-model/${esc(model.id)}.html">${esc(t('Model details and sources'))} ↗</a></details></div>`).join(''):chromeHtml`<p>${esc(t('Field use has not been investigated in this scope.'))}</p>`}</article>`;
 }).join('');
 return expandedUsage?chromeHtml`${usage}<details class="field-definition"><summary>${esc(t('Field definition and representations'))}</summary>${table}${representations}</details>`:chromeHtml`${table}<details class="field-usage"><summary>${esc(t('Usage and transformations'))}</summary>${usage}</details>${representations}`;
}
function domainCatalogue(){
 const entities=(D.entities||[]).filter(d=>d.normalized.entity.relations.some(r=>r.service===D.subject.slice(8))),models=D.readerModels||[];
 const unbound=models.filter(m=>(m.processLinks||[]).some(l=>l.subject===D.subject&&l.anchor==='section-entities')&&!entities.some(d=>m.entity==='entity:'+d.normalized.entity.id));
 const operation=D.operations.find(o=>o.id==='section-entities');
 const description=!entities.length&&!unbound.length&&operation?.summary?.text?chromeHtml`<section class="domain-description"><h4>${esc(t('Service data description'))}</h4><div class="clew-markdown">${prose(operation.summary.text)}</div></section>`:'';
 const documented=unbound.length?chromeHtml`<section class="domain-data-objects"><h4>${esc(t('Documented data objects'))}</h4><details class="domain-data-scope"><summary>${esc(t('Evidence and scope'))}</summary><p>${esc(t('These documentation mappings do not establish business ownership.'))}</p></details>${unbound.map(m=>chromeHtml`<details class="domain-data-object"><summary>${esc(m.title)}</summary><div class="clew-markdown">${prose(m.purpose?.text||'')}</div>${modelFieldCoverage(m)}${modelFieldRecords(m)}</details>`).join('')}</section>`:'';
 return chromeHtml`<section class="domain-catalogue" aria-label="${esc(t('Domain catalogue'))}"><h3>${esc(t('Domain catalogue'))}</h3>${entities.length?chromeHtml`<div class="domain-column-labels" aria-hidden="true"><span>${esc(t('Domain concept and service role'))}</span><span>${esc(t('Meaning'))}</span><span>${esc(t('Details'))}</span></div>${entities.map(d=>{
  const entity=d.normalized.entity,relations=entity.relations.filter(r=>r.service===D.subject.slice(8)),bound=models.filter(m=>m.entity==='entity:'+entity.id),representations=[...new Set(relations.flatMap(r=>r.representations||[]))],roles=[...new Set(relations.map(r=>t(r.kind)))],proposed=relations.filter(r=>r.confidence==='uncertain');
  return chromeHtml`<details class="domain-object"><summary><span class="domain-object-name"><strong>${esc(entity.title)}</strong><small>${roles.map(esc).join(' · ')}</small></span><span class="domain-object-comment">${esc(entity.description)}</span><span class="domain-open-label" title="${esc(t('Fields and service usage'))}">${esc(t('Fields'))}${proposed.length?chromeHtml`<small>${esc(t('Proposed relationships'))} · ${proposed.length}</small>`:''}</span></summary><div class="domain-object-detail">${proposed.length?chromeHtml`<details class="domain-interpretations"><summary>${esc(t('Proposed relationships'))} · ${proposed.length}</summary>${proposed.map(r=>chromeHtml`<p><b>${esc(t(r.role||'unknown'))} · ${esc(t(r.kind))}</b>: ${esc(r.rationale)}</p>`).join('')}<p>${esc(t('Review the specific relationship; its confidence does not change separate observed source facts.'))}</p></details>`:''}${bound.length?bound.map(m=>chromeHtml`<section><h4>${esc(m.title)}</h4>${modelFieldCoverage(m)}${modelFieldRecords(m)}<p><a href="../data-model/${esc(m.id)}.html">${esc(t('Model details and sources'))} ↗</a></p></section>`).join(''):chromeHtml`<p>${esc(t('No evidence-bound field glossary is associated with this concept yet.'))}</p>`}${relatedViews('entity:'+entity.id)}<details class="technical-evidence"><summary>${esc(t('Identity and evidence'))}</summary><div class="clew-markdown">${prose(entity.description)}</div><ul class="domain-code-term">${representations.map(r=>chromeHtml`<li><code>${esc(r)}</code></li>`).join('')}</ul><h4>${esc(t('Service actions and scope'))}</h4>${relations.map(r=>chromeHtml`<p><b>${esc(t(r.role||'unknown'))} · ${esc(t(r.kind))}${r.confidence==='uncertain'?' · '+esc(t('Proposed relationship')):''}</b>: ${esc(r.rationale)}</p>`).join('')}<p>${esc(t('Unrecorded actions are unknown, not confirmed absent.'))}</p><p>${entity.limitations.map(esc).join(' · ')}</p>${claimButton(d,t('Entity evidence'),true)}<pre>${pretty({identity:entity.id,relations})}</pre>${relatedNotes('entity:'+entity.id)}</details></div></details>`;
 }).join('')}`:chromeHtml`<p>${esc(t('No structured business-object catalogue has been authored.'))}</p>`}${documented}${description}</section>`;
}
function egressGroups(){
 const origin=D.subject?.startsWith('service:')?D.subject:'',edges=(D.declaredRelationships?.edges||[]).filter(e=>e.from===origin&&e.to!==origin),external=D.externalServices||{},nodes=new Map((D.declaredRelationships?.nodes||[]).map(n=>[n.id,n]));
 const ids=new Set([...edges.map(e=>e.to),...Object.keys(external).map(id=>'external-service:'+id)]),groups=new Map();
 for(const destination of ids){
  const id=destination.replace(/^external-service:/,''),record=external[id],identity=record?.identity,groupKey=identity&&identity.status!=='rejected'?'identity:'+identity.status+':'+identity.group:destination;
  if(!groups.has(groupKey))groups.set(groupKey,{title:identity&&identity.status!=='rejected'?identity.title:record?.title||nodes.get(destination)?.label||id,category:record?.category||'unknown',identity:identity&&identity.status!=='rejected'?identity:null,profiles:[],edges:[]});
  const group=groups.get(groupKey);group.profiles.push({id,destination,record});group.edges.push(...edges.filter(e=>e.to===destination));
 }
 return [...groups.values()];
}
function groupedEgress(){
 const groups=egressGroups(),categories=['system','storage','queue','configuration','unknown'];
 return chromeHtml`<section class="grouped-egress" aria-label="${esc(t('Outgoing systems and storage'))}"><h3>${esc(t('Outgoing systems and storage'))}</h3>${categories.map(category=>{
  const rows=groups.filter(g=>g.category===category);return rows.length?chromeHtml`<section><h3>${esc(t({'system':'Systems and APIs','storage':'Storage','queue':'Queues and topics','configuration':'Configuration-only profiles','unknown':'Unresolved destination category'}[category]))}</h3>${rows.map(group=>chromeHtml`<details class="egress-system"><summary><h4>${esc(group.title)}</h4>${[...new Set(group.edges.map(e=>e.transport?.destinationConfigKey).filter(Boolean))].map(key=>chromeHtml` <code>${esc(key)}</code>`).join(' · ')}</summary>${group.identity?chromeHtml`<p class="identity-hypothesis"><b>${esc(t(group.identity.status==='proposed'?'Identity hypothesis':group.identity.status))}</b>: ${esc(group.identity.declaration.rationale)}</p>`:''}${group.profiles.map(p=>chromeHtml`<p><a href="../${p.record?'external-services':'services'}/${esc(p.id)}.html">${esc(p.record?.title||p.id)}</a>${p.record?.identity?chromeHtml` · ${esc(p.record.identity.profile)}`:''}</p>`).join('')}${group.edges.length?chromeHtml`<ul class="egress-operations">${group.edges.map(edge=>chromeHtml`<li><b>${esc(edge.title)}</b><p>${esc(edge.transport?.method||'')} <code>${esc(edge.transport?.path||edge.transport?.topic||'')}</code> · ${esc(t(edge.transport?.kind||'unknown'))}</p><p><b>${esc(t('Configuration key'))}:</b> <code>${esc(edge.transport?.destinationConfigKey||t('Not established'))}</code></p>${(edge.addresses||[]).map(a=>chromeHtml`<p><code>${esc(a.url)}</code> · ${esc(a.source)} · ${esc(a.environment||'')}</p>`).join('')}<details><summary>${esc(t('Source and configuration evidence'))}</summary><pre>${pretty(edge)}</pre></details></li>`).join('')}</ul>`:chromeHtml`<p>${esc(t('Configuration/profile only; no outgoing call is established here.'))}</p>`}</details>`).join('')}</section>`:'';
 }).join('')}</section>`;
}
let flowSelection=null,flowHops=1,flowDirection='both',flowFieldSelection=null,flowConnectionSelection=null,flowInputPage=null;
function focusedFlow(g){
 const selected=g.nodes.find(n=>n.id===flowSelection),ids=new Set(selected?[selected.id]:[]);
 for(let hop=0;hop<flowHops;hop++){
  const current=new Set(ids);for(const edge of g.edges){if(flowDirection!=='outputs'&&current.has(edge.to))ids.add(edge.from);if(flowDirection!=='inputs'&&current.has(edge.from))ids.add(edge.to);}
 }
 return {nodes:g.nodes.filter(n=>ids.has(n.id)),edges:g.edges.filter(e=>ids.has(e.from)&&ids.has(e.to)&&(flowDirection==='both'||flowDirection==='inputs'&&ids.has(e.to)||flowDirection==='outputs'&&ids.has(e.from)))};
}
function flowConnectionGroups(g,complete=g){
 const key=edge=>JSON.stringify([edge.to,edge.kind,edge.meaning.text,edge.uncertainty,edge.authority]),selected=new Set(g.edges.map(key)),groups=new Map();
 for(const edge of complete.edges){const group=key(edge);if(!selected.has(group))continue;if(!groups.has(group))groups.set(group,[]);groups.get(group).push(edge);}
 return [...groups.values()];
}
function flowBranchControls(groups,index,nodes){
 if(!groups.length)return '';
 const caption=edges=>[...new Set(edges.map(e=>nodes.get(e.from)?.meaning.text||e.from))].join(' + ')+' → '+(nodes.get(edges[0].to)?.meaning.text||edges[0].to);
 return chromeHtml`<div class="dfccontrols"><label for="dfcpicker">${esc(t('Choose a connection'))} · ${index+1}/${groups.length}</label><select id="dfcpicker" data-dfcpicker>${groups.map((edges,i)=>chromeHtml`<option value="${esc(edges[0].id)}" ${i===index?'selected':''}>${i+1}. ${esc(caption(edges))}</option>`).join('')}</select><div><button id="dfcprevious" data-dfcshift="-1" ${index===0?'disabled':''}>${esc(t('Previous connection'))}</button><button id="dfcnext" data-dfcshift="1" ${index===groups.length-1?'disabled':''}>${esc(t('Next connection'))}</button></div></div>`;
}
function flowBranchGraph(o,selected,edges){
 const nodes=new Map(o.dataflow.nodes.map(n=>[n.id,n]));
 const card=node=>chromeHtml`<button class="dfcnode" data-flow-select="${esc(node.id)}" aria-pressed="${node.id===selected.id}"><strong>${esc(node.meaning.text)}</strong><code>${esc(node.representation)}</code><small>${esc(t(node.kind))} · ${esc(node.service)}</small></button>`;
 if(!edges?.length)return chromeHtml`<div class="dfcgraph" data-shown-nodes="1">${card(selected)}</div>`;
 const edge=edges[0],inputs=[...new Set(edges.map(e=>e.from))],pageSize=window.matchMedia?.('(max-width:700px)')?.matches?1:3,pages=Math.ceil(inputs.length/pageSize),automatic=Math.max(0,Math.floor(inputs.indexOf(selected.id)/pageSize)),page=Math.max(0,Math.min(pages-1,flowInputPage??automatic)),shown=inputs.slice(page*pageSize,page*pageSize+pageSize),target=nodes.get(edge.to);
 return chromeHtml`<section class="dfcview"><p class="flow-root-context"><b>${esc(t('Selected object'))}:</b> ${esc(selected.meaning.text)} · <code>${esc(selected.representation)}</code></p>${pages>1?chromeHtml`<div class="flow-input-pages"><p>${esc(t('Contributor inputs'))}: ${page*pageSize+1}–${Math.min(inputs.length,(page+1)*pageSize)} / ${inputs.length}. ${esc(t('Other contributors remain part of this same rule.'))}</p><button data-flow-input-page="${page-1}" ${page===0?'disabled':''}>${esc(t('Previous inputs'))}</button><button data-flow-input-page="${page+1}" ${page===pages-1?'disabled':''}>${esc(t('Next inputs'))}</button></div>`:''}<div class="dfcgraph" role="group" aria-label="${esc(t('Selected connection'))}" data-shown-nodes="${shown.length+1}"><div class="dfcinputs">${shown.map(id=>card(nodes.get(id))).join('')}</div><div class="dfcarrow"><span aria-hidden="true">→</span><small>${esc(t(edge.authority==='UNKNOWN'?'Unknown candidate':edge.authority==='DECLARED_TRANSFER'?'Declared transfer':edge.kind))}</small></div><div class="dfcoutput">${card(target)}</div></div>${inputs.includes(edge.to)?chromeHtml`<p>${esc(t('This connection returns to the same object.'))}</p>`:''}</section>`;
}
function flowReader(o){
 const g=o?.dataflow;if(!g)return chromeHtml`<p>${esc(t('No evidence-bound graph has been accepted.'))}</p>`;
 const nodes=new Map(g.nodes.map(n=>[n.id,n])),selected=nodes.get(flowSelection),focused=focusedFlow(g),models=D.readerModels||[],selectedField=models.flatMap(m=>(m.fields||[]).map(f=>({model:m,field:f}))).find(x=>x.model.id+'/'+x.field.id===flowFieldSelection);
 const hasRoles=g.nodes.some(n=>n.readerRole),endpoints=kind=>g.nodes.filter(n=>n.readerRole===kind);
 const nodeButtons=rows=>rows.map(n=>chromeHtml`<button type="button" data-flow-select="${esc(n.id)}" aria-pressed="${n.id===flowSelection}">${esc(n.meaning.text)} <code>${esc(n.representation)}</code></button>`).join('');
 const groups=flowConnectionGroups(focused,g),branchIndex=Math.max(0,groups.findIndex(edges=>edges[0].id===flowConnectionSelection)),active=groups[branchIndex];
 return chromeHtml`<section class="flow-reader"><h2>${esc(D.view?.definition.title||o.title)}</h2><div class="flow-overview">${hasRoles?chromeHtml`<section><h3>${esc(t('Inputs'))}</h3>${nodeButtons(endpoints('input'))}</section><section><h3>${esc(t('Outputs and destinations'))}</h3>${nodeButtons(endpoints('output'))}</section>${endpoints('stage').length?chromeHtml`<section><h3>${esc(t('Transformations'))}</h3>${nodeButtons(endpoints('stage'))}</section>`:''}${endpoints('side-effect').length?chromeHtml`<section><h3>${esc(t('Conditional side effects'))}</h3>${nodeButtons(endpoints('side-effect'))}</section>`:''}`:chromeHtml`<section><h3>${esc(t('Objects and values'))}</h3>${nodeButtons(g.nodes)}</section>`}</div><details class="flow-all-nodes"><summary>${esc(t('Choose an object or transformation'))} (${g.nodes.length})</summary><div class="flow-node-controls">${nodeButtons(g.nodes)}</div></details>${models.length?chromeHtml`<details class="flow-field-selector"><summary>${esc(t('Choose a field'))}</summary>${models.map(m=>chromeHtml`<h4>${esc(m.title)}</h4>${(m.fields||[]).map(f=>chromeHtml`<button data-flow-field="${esc(m.id+'/'+f.id)}" aria-pressed="${flowFieldSelection===m.id+'/'+f.id}">${esc(fieldBusinessName(f))}</button>`).join('')}`).join('')}</details>`:''}<div class="flow-toolbar"><button data-flow-reset>${esc(t('Reset'))}</button>${selected?chromeHtml`<button data-flow-direction="inputs">${esc(t('Inputs'))}</button><button data-flow-direction="outputs">${esc(t('Uses'))}</button><button data-flow-direction="both">${esc(t('Both directions'))}</button><button data-flow-expand>${esc(t('Expand one step'))}</button>`:''}</div><section id="flow-selection-detail" tabindex="-1" aria-live="polite">${selectedField?chromeHtml`<h3>${esc(selectedField.model.title)} · ${esc(fieldBusinessName(selectedField.field))}</h3>${modelFieldCoverage(selectedField.model)}${modelFieldRecords({...selectedField.model,fields:[selectedField.field]},D.subject,true)}<details class="field-mapping-limits"><summary>${esc(t('Evidence and mapping limits'))}</summary><p>${esc(t('Field mappings are explicit claims; object links do not establish field-level causality.'))}</p><a href="../data-model/${esc(selectedField.model.id)}.html">${esc(t('Model details and sources'))} ↗</a></details>`:''}${selected?chromeHtml`${selectedField?chromeHtml`<details class="flow-object-context"><summary>${esc(t('Whole object context'))}</summary>`:''}<h3>${esc(selected.meaning.text)}</h3><p><code>${esc(selected.representation)}</code></p><p>${esc(t('Incoming → selected object → outgoing'))}</p>${claimButton(selected.meaning,t('Open source'))}${flowBranchControls(groups,branchIndex,nodes)}<details id="dfcdiagram" class="flow-diagram"><summary>${esc(t('Focused diagram'))}</summary>${flowBranchGraph(o,selected,active)}</details>${(active?[active]:[]).map(edges=>{const edge=edges[0];return chromeHtml`<article class="flow-transformation"><h4>${[...new Set(edges.map(e=>nodes.get(e.from)?.meaning.text||e.from))].map(esc).join(' + ')} → ${esc(nodes.get(edge.to)?.meaning.text||edge.to)}</h4><p>${esc(t(edge.kind))} · ${esc(t(edge.authority))}</p><div class="clew-markdown">${prose(edge.meaning.text)}</div>${edge.uncertainty?chromeHtml`<p>${esc(edge.uncertainty)}</p>`:''}${edges.map(e=>claimButton(e.meaning,t('Open source')+': '+(nodes.get(e.from)?.representation||''))).join(' ')}</article>`;}).join('')}<details><summary>${esc(t('Representation and source details'))}</summary><ul>${focused.nodes.map(n=>chromeHtml`<li>${esc(n.meaning.text)} · <code>${esc(n.representation)}</code> · ${esc(domainAssociationLabel(n))} ${claimButton(n.meaning,t('Open source'))}</li>`).join('')}</ul></details>${selectedField?'</details>':''}`:!selectedField?chromeHtml`<p>${esc(t('Choose an input, output or field to inspect its direct context.'))}</p>`:''}</section></section>`;
}
function selectFlow(action,value){
 const owner=operation(current);if(!owner?.dataflow)return;
 const diagramOpen=$('dfcdiagram')?.open,objectOpen=document.querySelector?.('.flow-object-context')?.open;
 const groups=flowConnectionGroups(focusedFlow(owner.dataflow),owner.dataflow);
 if(action==='node'){flowSelection=value;flowFieldSelection=null;flowHops=1;flowDirection='both';flowConnectionSelection=null;flowInputPage=null;}
 if(action==='field'){
  flowFieldSelection=value;flowSelection=null;flowHops=1;flowDirection='both';flowConnectionSelection=null;flowInputPage=null;
  for(const m of D.readerModels||[])for(const f of m.fields||[])if(m.id+'/'+f.id===value)flowSelection=(f.graphBindings||[]).find(b=>b.subject===D.subject)?.node||null;
 }
 if(action==='expand')flowHops=Math.min(owner.dataflow.nodes.length,flowHops+1);
 if(action==='direction'){flowDirection=value;flowInputPage=null;}
 if(action==='connection'){flowConnectionSelection=groups.find(edges=>edges[0].id===value)?.[0].id||null;flowInputPage=null;}
 if(action==='connection-shift'){const index=Math.max(0,groups.findIndex(edges=>edges[0].id===flowConnectionSelection));flowConnectionSelection=groups[Math.max(0,Math.min(groups.length-1,index+Number(value)))]?.[0].id||null;flowInputPage=null;}
 if(action==='input-page')flowInputPage=Math.max(0,Number(value)||0);
 if(action==='reset'){flowSelection=null;flowFieldSelection=null;flowHops=1;flowDirection='both';flowConnectionSelection=null;flowInputPage=null;}
 $('scenario-content').innerHTML=flowReader(owner);
 const branchAction=['connection','connection-shift','input-page'].includes(action);
 if(diagramOpen||branchAction)$('dfcdiagram').open=true;
 if(objectOpen){const context=document.querySelector?.('.flow-object-context');if(context)context.open=true;}
 if(branchAction){const target=action==='connection-shift'?(Number(value)>0?'dfcnext':'dfcprevious'):'dfcpicker';const control=$(target);if(control&&!control.disabled)control.focus?.();else $('dfcpicker')?.focus?.();}
 else{$('flow-selection-detail')?.focus?.();$('flow-selection-detail')?.scrollIntoView?.({block:'start'});}
}
document.addEventListener('change',event=>{if(event.target?.dataset?.dfcpicker!==undefined)selectFlow('connection',event.target.value);});
function executionBadge(event){return event.reader?.execution?chromeHtml`<span class="execution-mode">${esc(t(event.reader.execution.mode))}</span>`:'';}
function compactProcess(o){
 const events=(o.events||[]).filter(e=>e.kind!=='end'),overview=events.filter(e=>e.reader?.overview),steps=overview.length?overview:events.filter(e=>e.reader);
 if(!steps.length)return chromeHtml`<section class="compact-process">${readerEvidenceLegend()}<p>${esc(t('Detailed retained algorithm is available below; no compact overview has been authored.'))}</p></section>`;
 return chromeHtml`<section class="compact-process"><h3>${esc(t('Process overview'))}</h3>${readerEvidenceLegend()}<ol>${steps.map(event=>chromeHtml`<li><details><summary>${esc(event.reader?.label?.text||event.text)} ${executionBadge(event)}</summary><div class="clew-markdown">${prose(event.text)}</div><details class="process-step-evidence"><summary>${esc(t('Evidence and execution details'))}</summary>${event.reader?.execution&&event.reader.execution.meaning.text.trim()!==event.text.trim()?chromeHtml`<p>${esc(event.reader.execution.meaning.text)}</p>`:''}<div class="process-proof-links">${claimButton(event.reader?.label,t('Label evidence'))}${event.reader?.execution?claimButton(event.reader.execution.meaning,t('Execution evidence')):''}</div>${stepEvidence(o,event)}${externalCallDetails(o,event)}</details></details></li>`).join('')}</ol></section>`;
}
function serviceDeclaredEgress(){
 if(!D.subject?.startsWith('service:'))return '';
 const graph=D.declaredRelationships||{},origin='service:'+D.subject.slice(8);
 const edges=(graph.edges||[]).filter(e=>e.from===origin);
 if(!edges.length)return '';
 const nodes=new Map((graph.nodes||[]).map(n=>[n.id,n]));
 return chromeHtml`<section class="declared-egress"><h3>Registered outgoing boundaries</h3><p>Declarations for the selected service, including logical storage and SDK calls. Runtime activation, transport delivery and remote outcomes remain unverified.</p><ul>${edges.map(e=>{
  const node=nodes.get(e.to),external=e.to?.startsWith('external-service:'),id=String(e.to||'').split(':').slice(1).join(':');
  const href='../'+(external?'external-services/':'services/')+encodeURIComponent(id)+'.html';
  return chromeHtml`<li><a href="${esc(href)}">${esc(node?.label||id)}</a>: ${esc(e.title)} · ${esc(e.transport?.kind||'unknown')}<details><summary>Declaration and source checks</summary><pre>${pretty(e)}</pre></details></li>`;
 }).join('')}</ul></section>`;
}
const serviceSectionOrder=['section-overview','section-responsibilities','section-entities','section-ingress','section-egress'];
function profileSummary(owner,accepted){
 return chromeHtml`<div class="profile-summary clew-markdown">${prose(accepted.text)}</div>${button(accepted.sourceIds,t('Supporting code'),false,owner.id)}`;
}
function serviceProfile(){
 if(!D.subject.startsWith('service:'))return '';
 const definitions=[['section-responsibilities',t('Responsibilities')],['section-entities',t('Domain entities')],['section-ingress',t('Entry points')],['section-egress',t('External calls and storage')]];
 return chromeHtml`<section class="service-profile" aria-label="Service profile">${definitions.map(([id,title])=>{
  const section=(D.sections||[]).find(s=>s.id===id),owner=D.operations.find(o=>o.id===id),accepted=owner?.summary;
  let detail='';
  if(id==='section-entities'){
   const entities=(D.entities||[]).filter(d=>d.normalized.entity.relations.some(r=>r.service===D.subject.slice(8)));
   detail=chromeHtml`<p class="profile-note">DTOs and implementation classes alone do not establish entity ownership or creation.</p>${entities.length?chromeHtml`<p>Declared domain identities: ${entities.map(d=>esc(d.normalized.entity.title)).join(', ')}.</p>`:chromeHtml`<p class="profile-gap">Entity ownership and creation roles have no explicit domain declarations in this publication.</p>`}`;
  }
  if(id==='section-ingress'){
   const known=serviceInventory().publicBoundaries||[];
   detail=known.length?chromeHtml`<ul class="profile-entry-list">${known.map(entry=>chromeHtml`<li><button data-entry="${esc(entry.id)}">${esc((entry.trigger.methods||[]).join(' / '))} ${esc((entry.trigger.paths||[]).join(' · ')||entry.symbol)}</button><span>${D.operations.some(o=>o.id===entry.id)?t('Behavior narrative available'):t('Behavior narrative not yet accepted')}</span></li>`).join('')}</ul>`:chromeHtml`<p class="profile-gap">No public entries are recorded in this publication. This does not establish that the service has no entry points.</p>`;
   detail+=chromeHtml`<details class="profile-discovery"><summary>Discovery scope</summary><p>The inventory is bounded by retained analyzer evidence. Per-transport completeness for HTTP, messaging, schedules and CLI is not established here; absent entries are not proof of absence.</p></details>`;
  }
  if(id==='section-egress')detail=chromeHtml`<p><button data-entry="section-egress">${esc(t('Outgoing systems and storage'))} (${egressGroups().length}) →</button></p>`+chromeHtml`<p class="profile-note">Thread-to-call links and complete outgoing coverage are not inferred from a dependency list.</p>`;
  return chromeHtml`<article class="profile-section" id="profile-${esc(id)}"><h3>${esc(title)}</h3>${translationGaps[id]?translationNotice(id):accepted?chromeHtml`${profileSummary(owner,accepted)}${visualStatus(owner)}${owner.boundaries?.length?chromeHtml`<details class="profile-discovery"><summary>Scope and limitations</summary><ul>${owner.boundaries.map(text=>chromeHtml`<li>${esc(text)}</li>`).join('')}</ul></details>`:''}`:chromeHtml`<p class="profile-gap">${esc(section?.gap||t('No source-bound section summary has been accepted in this publication.'))}</p>`}${detail}${section?chromeHtml`<p><button class="source-link" data-entry="${esc(id)}">Open ${esc(title.toLowerCase())} details →</button></p>`:''}</article>`;
 }).join('')}</section>`;
}

const sourceCatalogue=D.catalogue.length?[...D.catalogue]:publication.operations.filter(o=>!o.id.startsWith('section-')&&!o.id.startsWith('assessment-')&&o.id!=='process-overview'&&!o.dataflow).map(o=>({id:o.id,symbol:translationGaps[o.id]?o.id:o.title,kind:'SCENARIO',trigger:{methods:['FLOW'],paths:[]},sourceIds:[]}));
const boundaryIds=D.selectedPublicBoundaryIds??D.publicBoundaryIds;
const publicBoundaryIds=boundaryIds?new Set(boundaryIds):null;
const entries=sourceCatalogue.filter(e=>!publicBoundaryIds||publicBoundaryIds.has(e.id)||D.operations.some(o=>o.id===e.id));
const showInternalProcesses=D.readerOptions?.showInternalProcesses===true;
const processCatalogVisible=D.subject.startsWith('service:')&&((D.savedProcesses||[]).length>0||showInternalProcesses&&((D.processCandidates?.internal||[]).length>0||(D.processCandidates?.summary?.internalCandidateCount||0)>0));
if(D.process){for(const operation of D.operations.filter(o=>!o.id.startsWith('section-')&&!o.id.startsWith('assessment-')&&o.id!=='process-overview'&&!o.dataflow)){if(!entries.some(entry=>entry.id===operation.id))entries.push({id:operation.id,symbol:translationGaps[operation.id]?operation.id:operation.title,kind:'SCENARIO',trigger:{methods:['FLOW'],paths:[]},sourceIds:[]});}}
if(D.subject.startsWith('service:'))entries.push(...D.contracts.filter(c=>!c.normalized.entrypoint).map(c=>({id:c.id,symbol:t('Declared OpenAPI · ')+c.normalized.declaredSource,kind:'DECLARED_OPENAPI',trigger:{methods:[c.normalized.method],paths:[c.normalized.path]},sourceIds:c.sourceIds})));
entries.unshift(...[...(D.sections||[])].sort((a,b)=>{const rank=id=>serviceSectionOrder.indexOf(id)<0?serviceSectionOrder.length:serviceSectionOrder.indexOf(id);return rank(a.id)-rank(b.id);}).map(s=>({id:s.id,symbol:s.title,kind:'SECTION',trigger:{methods:['SECTION'],paths:[]},sourceIds:s.content?.summary.sourceIds||[]})));
entries.push(...(D.notes||[]).map(n=>({id:n.id,symbol:n.association.title,kind:'NOTE',searchText:JSON.stringify(n.association)+' '+(n.original.text||''),trigger:{methods:['NOTE'],paths:[]},sourceIds:[]})));
entries.push(...(D.serviceFragments||[]).map(f=>({id:f.id,symbol:f.title+' · '+f.fragmentId,kind:'SERVICE_FRAGMENT',trigger:{methods:['FRAGMENT'],paths:[]},sourceIds:[]})));
if(D.process){entries.unshift({id:'process-overview',symbol:t('Process overview'),kind:'PROCESS',trigger:{methods:['PROCESS'],paths:[]},sourceIds:[]});}
if(D.view){entries.unshift({id:'entity-dataflow',symbol:t('Entity data flow'),kind:'VIEW',trigger:{methods:['VIEW'],paths:[]},sourceIds:[]});if(!entries.some(e=>e.id===D.view.definition.id))entries.push({id:D.view.definition.id,symbol:t('Source flow scope'),kind:'SCENARIO',trigger:{methods:['FLOW'],paths:[]},sourceIds:[]});}
if(processCatalogVisible)entries.splice((D.sections||[]).length,0,{id:'process-catalog',symbol:t(showInternalProcesses?'Internal processes':'Saved processes'),kind:'PROCESS_CATALOG',trigger:{methods:['PROCESS'],paths:[]},sourceIds:[]});
entries.splice((D.sections||[]).length+(D.subject.startsWith('service:')?1:0),0,...visualRows().map(({owner,visual})=>({id:visualKey(owner.id,visual.id),visualOwner:owner.id,visualId:visual.id,symbol:visual.title,kind:'VISUAL',searchText:visual.purpose?.text+' '+visual.scope?.text,trigger:{methods:[{'execution-flow':'FLOW','dependency-map':'MAP','decision-table':'RULES'}[visual.kind]||'VIEW'],paths:[]},sourceIds:[]})));
const inventoryEntries=()=>sourceCatalogue.concat(entries.filter(e=>e.kind==='DECLARED_OPENAPI')).filter(e=>!['SERVICE_FRAGMENT','SECTION','NOTE','PROCESS','VIEW','PROCESS_CATALOG','VISUAL'].includes(e.kind));
const documentedCount=()=>D.operations.filter(o=>!o.id.startsWith('section-')&&!o.id.startsWith('assessment-')&&o.id!=='process-overview'&&!o.dataflow).length;
function readingScopeSummary(){
 const inventory=serviceInventory();
 if(Array.isArray(inventory.publicBoundaries)){
  const described=inventory.publicBoundaries.filter(e=>D.operations.some(o=>o.id===e.id)).length;
  const internal=inventory.internalCallableCount??inventory.internalCallables?.length;
  return chromeHtml`<span><b>${described}</b> ${esc(t('documented public operations'))}</span><span><b>${inventory.publicBoundaries.length}</b> ${esc(t('discovered public boundaries'))}</span>${Number.isInteger(internal)?chromeHtml`<span><b>${internal}</b> ${esc(t('internal callable evidence records'))}</span>`:''}`;
 }
 return chromeHtml`<span><b>${documentedCount()}</b> documented operations</span><span><b>${inventoryEntries().length}</b> source and API inventory records</span>`;
}
let sourceScope='',sourceOperation='',sourceFragment='',sourcePath=[];
let current=entries[0],tab='sequence',sourceId='',lastFocus=null,contractId='',payloadId='';
const operation=e=>D.operations.find(o=>o.id===(e.visualOwner||e.id));
function sourceLabel(symbol){
 const match=String(symbol||'').match(/^source:(.+)\/(package|class):([^/]+)\/([^/]+)\/[a-f0-9]{20}$/);
 return match?`${match[2]==='class'?match[3].split('.').pop()+'.':''}${match[4]}() · ${match[1].split('/').pop()}`:symbol;
}
const label=e=>(e.kind==='SECTION'?sectionTitles[e.id]:['NOTE','VISUAL','SERVICE_FRAGMENT'].includes(e.kind)?e.symbol:operation(e)?.title)||e.trigger.paths?.join(' · ')||sourceLabel(e.symbol)||e.id;
function plantumlPreview(stem,available,alt){return available?chromeHtml`<div class="diagram-img-wrap"><img class="diagram-img" src="../diagrams/${esc(stem)}.svg" alt="${esc(alt)}"></div>`:chromeHtml`<p class="renderer-unavailable">${esc(t('PlantUML SVG preview is unavailable; the .puml file and text steps remain available.'))}</p>`;}
function threadEvidenceLinks(sourceIds){
 const thread=D.processThread;if(!thread)return button(sourceIds,t('Source'),true);
 const ids=(thread.methodOrder||[]).filter(id=>(sourceIds||[]).includes(thread.methods[id].source.id));
 const selected=new Set(ids.map(id=>thread.methods[id].source.id));const outside=(sourceIds||[]).filter(id=>!selected.has(id));
 return ids.map(id=>chromeHtml`<a class="source-link" href="${esc(threadSourceHref(id,thread.methods[id].source.startLine,thread.methods[id].source.endLine,thread))}">${esc(t('Source'))} · ${esc(thread.methods[id].label)}</a>`).join(' ')+(outside.length?' '+button(outside,t('Retained source outside the selected thread'),true):'');
}
function threadAnchorHref(id){return '#'+(current?.kind==='SERVICE_FRAGMENT'?current.id+'~':'')+id;}
function remoteSourceHref(record,start=record?.startLine,end=record?.endLine){
 if(record?.text!==''||!/^https:\/\//.test(record.url||''))return '';
 return record.url.split('#')[0]+'#L'+start+(end!==start?(record.url.includes('/-/blob/')?'-':'-L')+end:'');
}
function threadSourceHref(method,line,end=line,thread=D.processThread){return remoteSourceHref(thread?.methods[method]?.source,line,end)||threadAnchorHref('thread-source-'+method+'-L'+line);}
function processCallBindingsCard(packet){
 const bindings=(packet.methods||[]).flatMap(m=>Array.isArray(m.callBindings)?m.callBindings:[]);
 if(!bindings.length)return '';
 const methods=new Map((packet.methods||[]).map(m=>[m.id,m]));
 const receiver=b=>b.targetMethodId?(methods.has(b.targetMethodId)?chromeHtml`<p>${esc(t('Receiving declaration'))}: ${esc(methods.get(b.targetMethodId).symbol||b.targetMethodId)} ${threadEvidenceLinks(methods.get(b.targetMethodId).sourceIds||[])}</p>`:chromeHtml`<p>${esc(t('Receiving declaration is outside the retained envelope.'))}</p>`):'';
 return chromeHtml`<details class="process-call-bindings"><summary>${esc(t('Compiler-selected calls and parameter bindings'))} (${bindings.length})</summary><p>${esc(t('Argument expressions are not evaluated values.'))}</p>${bindings.map(b=>chromeHtml`<article><h4>${esc(methods.get(b.callerMethodId)?.symbol||b.callerMethodId)} → ${esc(b.targetIdentity)}</h4><p>${esc(t('Compiler-selected target'))}: ${esc(b.targetStatus)} ${threadEvidenceLinks(b.sourceIds||[])}</p>${receiver(b)}${b.argumentStatus==='COMPILER_PARAMETER_MAPPING'?chromeHtml`<ul>${(b.arguments||[]).map(a=>chromeHtml`<li><code>${esc(a.expression)}</code> → <code>${esc(a.parameter)}</code> [${esc(a.parameterIndex)}]</li>`).join('')}</ul>`:chromeHtml`<p>${esc(t('Parameter mapping is unavailable.'))}</p>`}${b.omittedDefaultParameterIndices?.length?chromeHtml`<p>${esc(t('Omitted default parameter indices'))}: ${b.omittedDefaultParameterIndices.map(esc).join(', ')} · ${esc(b.defaultValues)}</p>`:''}${b.receiver?.type?chromeHtml`<p>${esc(t('Receiver type'))}: ${esc(b.receiver.type)}</p>`:''}<p>${esc(t('Receiver object and runtime dispatch are unknown.'))}</p>${b.returnTransfer?.declaredType?chromeHtml`<p>${esc(t('Declared return type'))}: ${esc(b.returnTransfer.declaredType)}</p>`:''}<p>${esc(t('Return and exception transfer remain unproven.'))}</p></article>`).join('')}</details>`;
}
function processExtractionCard(packet){
 if(!packet)return '';
 const events=packet.events||[],kinds=new Set(['IF','TRY','LOOP','RETURN','THROW','FINALLY']);
 const selected=new Set((packet.query?.outcomeEventIds)||[]);
 const rows=events.filter(e=>selected.has(e.id)||kinds.has(e.detail?.kind));
 return chromeHtml`<section class="process-extraction"><h3>${esc(t('Extracted evidence envelope'))}</h3><p>${esc(t('Potential static behavior; business intent and runtime effects are not established.'))}</p><p>${esc(packet.status)} · ${packet.methods?.length||0} ${esc(t('retained method envelopes'))} · ${events.length} ${esc(t('retained events'))}</p><ul>${rows.map(e=>chromeHtml`<li><strong>${esc(e.detail?.kind||'UNKNOWN')}</strong> ${esc(e.detail?.condition||e.detail?.target||e.id)} ${threadEvidenceLinks(e.sourceIds||[])}</li>`).join('')}</ul>${processCallBindingsCard(packet)}<details><summary>${esc(t('Extraction frontiers and provenance'))} (${packet.frontier?.length||0})</summary><pre>${pretty({snapshot:packet.snapshot,packetDigest:packet.packetDigest,policy:packet.policy,query:packet.query,outcomes:packet.outcomes,frontier:packet.frontier,interpretationPlan:packet.interpretationPlan})}</pre></details></section>`;
}
function boundaryContractsCard(thread){
 const claims=rows=>'<ul>'+rows.map(claim=>chromeHtml`<li>${esc(claim.text)} ${claim.missingEvidence?chromeHtml`<strong class="thread-boundary-badge">${esc(t('Unresolved'))}: ${esc(claim.missingEvidence)}</strong>`:''} ${(claim.evidenceRefs||[]).map(id=>{const e=activeContract.evidence[id];return chromeHtml`<span class="thread-provenance">${esc(e.service)} · ${esc(e.source?.file||e.file||e.sourceId)}:${e.startLine}–${e.endLine} · ${esc(e.revision)} ${e.source?button([e.sourceId],t('Source'),true):(/^https?:\/\//.test(e.url||'')?chromeHtml`<a href="${esc(e.url)}">${esc(t('Source'))}</a>`:'')}</span>`;}).join(' ')}</li>`).join('')+'</ul>';
 let activeContract;
 return (thread.boundaryContracts||[]).map(contract=>{
  activeContract=contract;
  const fragment=(thread.serviceFragments||[]).find(f=>f.id===contract.receiverFragment);
  const receiver=fragment?thread.methods[fragment.entryMethod]:null;
  const href=fragment?`../services/${receiver.source.service}.html#service-fragment-${thread.processId}:${fragment.id}`:null;
  return chromeHtml`<section id="thread-contract-${esc(contract.id)}" tabindex="-1" class="gap-card"><h4>${esc(contract.transport)} · ${esc(contract.id)}</h4><p>${esc(t('Declared source interpretation; wire compatibility and runtime delivery are not proven.'))}</p>${claims([contract.operation])}${[['Request',contract.request],['Response and continuation',contract.response],['Failure paths',contract.failures],['Guarantees and limits',contract.guarantees]].map(([title,rows])=>chromeHtml`<h5>${esc(t(title))}</h5>${claims(rows||[])}`).join('')}<p><a href="#thread-step-${esc(contract.callStep)}">${esc(t('Call site'))}</a>${href?chromeHtml` · <a href="${esc(href)}">${esc(t('Open receiving service fragment'))}: ${esc(receiver.label)}</a> · <a href="#thread-step-${esc(fragment.rootStep)}">${esc(t('Expand receiving entrypoint here'))}</a>`:chromeHtml` · ${esc(t('Receiver capture is missing; no consumer is inferred.'))}`}</p></section>`;
 }).join('');
}
function serviceFragmentCard(fragment){
 const original=fragment.thread,used=new Set();
 function collect(rows){for(const row of rows||[]){used.add(row.method);if(row.target)used.add(row.target);for(const effect of row.effects||[])used.add(effect.citation.method);for(const arm of row.branchGroup?.arms||[]){if(arm.guard)used.add(arm.guard.method);collect(arm.steps);}collect(row.children);}}
 collect(fragment.steps);
 const thread={...original,rootMethod:fragment.entryMethod,steps:fragment.steps,methodOrder:original.methodOrder.filter(id=>used.has(id)),boundaryContracts:[],serviceFragments:[]};
 const entrypoint=original.methods[fragment.entryMethod]?.entrypoint;
 const operation=D.operations.find(op=>op.id===entrypoint);
 const operationLink=operation?chromeHtml` · <a href="#${esc(operation.id)}">${esc(t('Open full service operation'))}: ${esc(operation.title)}</a>`:'';
 return chromeHtml`<section id="${esc(fragment.id)}" tabindex="-1"><h2>${esc(fragment.title)} · ${esc(fragment.fragmentId)}</h2><p>${esc(t('Receiving service fragment from the same captured process; runtime delivery is unverified.'))}</p><p><a href="${esc(fragment.returnHref)}">${esc(t('Return to process at this fragment'))}</a>${operationLink}</p>${processThreadCard(thread).replace(/href="#(thread-[^"]+)"/g,(_,id)=>`href="${esc(threadAnchorHref(id))}"`)}</section>`;
}
function processThreadCard(thread){
 if(!thread||thread.status!=='SOURCE_BOUND_DECLARATION')return '';
 const methods=thread.methods||{},order=thread.methodOrder||Object.keys(methods);
 const kinds={STEP:'Step',ACTION:'Action',DECISION:'Decision',ALTERNATIVE:'Alternative',ERROR:'Error path',BOUNDARY:'Evidence boundary'};
 function steps(rows){return '<ul class="thread-steps">'+rows.map(row=>{
  const method=methods[row.method],target=row.target?methods[row.target]:null;
  const crossService=target&&method?.source.service!==target.source.service;
  const citation=chromeHtml`<a class="source-link thread-citation" href="${esc(threadSourceHref(row.method,row.startLine,row.endLine,thread))}" data-thread-end="${row.endLine}">${esc(t('Source'))} · ${esc(method?.source.service||'')}:${row.startLine}${row.endLine!==row.startLine?'–'+row.endLine:''}</a>`;
  const receiving=(thread.serviceFragments||[]).find(f=>f.entryMethod===row.target);
  const targetLink=receiving?chromeHtml` <a class="thread-target" href="#thread-step-${esc(receiving.rootStep)}">${esc(t('Expand receiving entrypoint here'))}: ${esc(target.label)}</a> <a href="../services/${esc(target.source.service)}.html#service-fragment-${esc(thread.processId)}:${esc(receiving.id)}">${esc(t('Open receiving service fragment'))}</a>`:target?chromeHtml` <a class="thread-target" href="${esc(threadSourceHref(row.target,target.source.startLine,target.source.endLine,thread))}">${esc(t(crossService?'Declared service connection':'Declared target'))}: ${esc(target.label)}</a>`:'';
  const linked=row.processPage?thread.processPages?.[row.processPage]:null;
  const pageLink=linked&&/^file:\/\/\//.test(linked.href||'')?chromeHtml`<aside class="thread-process-page"><p><a href="${esc(linked.href)}">${esc(t('Open existing service process'))}: ${esc(linked.title)}</a></p><p>${esc(linked.service)} · ${esc(t('Source revision'))}: <code>${esc(linked.revision)}</code></p><details><summary>${esc(t('Exact page binding'))}</summary><p>${esc(t('Existing explanation; its internal narrative is not regenerated.'))}</p><p>${esc(linked.proof?.pageDigest||'')} · ${esc(linked.proof?.exportDigest||linked.proof?.publicationDigest||'')}</p></details></aside>`:'';
  const preview=(row.children||[]).map(child=>child.title).join(' · ');
  const cite=c=>chromeHtml`<a class="source-link thread-citation" href="${esc(threadSourceHref(c.method,c.startLine,c.endLine,thread))}" data-thread-end="${c.endLine}">${esc(t('Source'))} · ${esc(methods[c.method]?.source.service||'')}:${c.startLine}–${c.endLine}</a>`;
  const effectKinds={STATE_ASSIGNMENT:'State assignment',STORAGE_CALL:'Storage call',MESSAGE_SEND:'Message send',EXTERNAL_CALL:'External call',RETURN:'Return',THROW:'Throw',TRANSACTION_SCOPE:'Transaction scope'};
  const effects=row.effects?.length?chromeHtml`<div class="thread-effects"><h5>${esc(t('Selected declared effects'))}</h5><ul>${row.effects.map(effect=>chromeHtml`<li><span>${esc(t(effectKinds[effect.kind]||effect.kind))}</span> ${esc(effect.label)} ${cite(effect.citation)}</li>`).join('')}</ul><p>${esc(t('Effect labels are declarations; calls do not prove commit or delivery.'))}</p></div>`:'';
  const branchModes={IF_ELSE:'One guarded arm or its default',FIRST_MATCH:'First matching arm',INDEPENDENT:'Independent conditions'};
  const branch=row.branchGroup?chromeHtml`<section class="thread-branches"><p class="thread-branch-mode">${esc(t(branchModes[row.branchGroup.mode]))} · ${esc(t('Declared control interpretation'))}</p><div class="thread-branch-arms">${row.branchGroup.arms.map(arm=>chromeHtml`<section id="thread-step-${esc(arm.id)}" tabindex="-1"><h5>${esc(arm.condition??t('Otherwise'))}</h5>${arm.guard?cite(arm.guard):''}${steps(arm.steps||[])}</section>`).join('')}</div></section>`:'';

  return chromeHtml`<li id="thread-step-${esc(row.id)}" tabindex="-1"><article><div class="thread-step-meta"><span>${esc(method?.source.service||'')}</span><span>${esc(t(kinds[row.kind||'STEP']||'Step'))}</span>${crossService?chromeHtml`<span class="thread-boundary-badge">${esc(t('Delivery unverified'))}</span>`:''}</div><h4>${esc(row.title)}</h4><p>${esc(row.text)}</p><p class="thread-citations">${citation}${targetLink}</p>${pageLink}${effects}${branch}${row.children?.length?chromeHtml`<details class="thread-children"><summary>${esc(t('Detailed steps'))} (${row.children.length})<span class="thread-child-preview">${esc(preview)}</span></summary>${steps(row.children)}</details>`:''}</article></li>`;
 }).join('')+'</ul>';}
 const connections=[];
 function collect(rows){for(const row of rows||[]){const a=methods[row.method],b=methods[row.target];if(a&&b&&a.source.service!==b.source.service)connections.push({row,from:a.source.service,to:b.source.service});collect(row.children);for(const arm of row.branchGroup?.arms||[])collect(arm.steps);}}
 collect(thread.steps);
 const selectedInteractions=new Set(D.process?.definition.interactions||[]);
 const declarations=(D.interactions||[]).filter(interaction=>selectedInteractions.has(interaction.id)&&interaction.declaration?.from?.service&&interaction.declaration?.to?.service);
 const declarationCards=declarations.map(interaction=>{
  const d=interaction.declaration,transport=d.transport||{};
  const sourceIds=[...new Set([...(interaction.from?.sourceIds||[]),...(interaction.to?.sourceIds||[])])];
  return chromeHtml`<li><span>${esc(d.from.service)} → ${esc(d.to.service)}</span><p>${esc(d.title||interaction.id)}</p><p class="thread-connection-status">${esc(t('Declared transport'))}: ${esc([transport.kind,transport.method,transport.path,transport.topic].filter(Boolean).join(' · ')||t('unspecified'))} · ${esc(t('Delivery unverified'))}</p>${transport.destinationConfigKey?chromeHtml`<p>${esc(t('Declared configuration key'))}: <code>${esc(transport.destinationConfigKey)}</code></p>`:''}${threadEvidenceLinks(sourceIds)}<details><summary>${esc(t('Connection evidence and unresolved checks'))}</summary><pre>${pretty({id:interaction.id,origin:interaction.origin,from:interaction.from?.status,to:interaction.to?.status,callSite:interaction.callSite?.status,destination:interaction.destination,runtime:interaction.runtime,contractStatus:interaction.contractStatus,boundaries:interaction.boundaries||[]})}</pre></details></li>`;
 }).join('');
 const transfers=connections.length||declarations.length?chromeHtml`<section class="thread-connections"><h3>${esc(t('Declared service connections'))}</h3><p>${esc(t('These source-linked connections do not prove routing or delivery.'))}</p><ul>${declarationCards}${connections.map(({row,from,to})=>chromeHtml`<li><span>${esc(from)} → ${esc(to)}</span> <a href="#thread-step-${esc(row.id)}">${esc(row.title)}</a><p>${esc(t('Explicit step target; transport is not inferred.'))}</p></li>`).join('')}</ul></section>`:'';
 const source=order.map(id=>{
  const method=methods[id],record=method.source,lines=record.text.split('\n');
  return chromeHtml`<section class="thread-method" data-thread-method="${esc(id)}"><h4>${esc(method.label)}</h4><p class="thread-provenance">${esc(record.service)} · ${/^https?:\/\//.test(record.url||'')?chromeHtml`<a href="${esc(record.url)}">${esc(record.file)}:${record.startLine}–${record.endLine}</a>`:chromeHtml`${esc(record.file)}:${record.startLine}–${record.endLine}`}<br>${esc(t('Source revision'))}: <code>${esc(record.revision)}</code></p>${record.text?chromeHtml`<pre class="thread-code"><code>${lines.map((line,index)=>chromeHtml`<span class="thread-code-line" id="thread-source-${esc(id)}-L${record.startLine+index}" data-line="${record.startLine+index}" tabindex="-1">${esc(line)}</span>${index<lines.length-1?'\n':''}`).join('')}</code></pre>`:chromeHtml`<p>${esc(t('Source text is omitted from this export. Open the exact retained revision in the repository.'))}</p>`}<details class="thread-binding"><summary>${esc(t('Exact source binding'))}</summary><p>${esc(record.authority)} · ${esc(record.id)} · ${esc(record.textDigest)}</p></details></section>`;
 }).join('');
 return chromeHtml`<section class="process-thread"><nav class="thread-jump-links" aria-label="${esc(t('Process sections'))}"><a href="#thread-steps">${esc(t('Process steps'))}</a><a href="#thread-source">${esc(t('Source'))} (${order.length})</a><a href="#thread-limits">${esc(t('Evidence limits'))}</a></nav>${transfers}${boundaryContractsCard(thread)}<h3 id="thread-steps" tabindex="-1">${esc(t('Process steps'))}</h3><p class="thread-reading-note">${esc(t('Cards follow the declared reading order. Nesting groups explanations; it does not prove execution order or branch exclusivity.'))}</p>${steps(thread.steps||[])}<section id="thread-limits" tabindex="-1" class="thread-boundaries"><h3>${esc(t('Scope and unresolved boundaries'))}</h3><p>${esc(thread.scope)}</p>${thread.boundaries?.length?chromeHtml`<ul>${thread.boundaries.map(row=>chromeHtml`<li>${esc(row)}</li>`).join('')}</ul>`:chromeHtml`<p>${esc(t('No boundaries were declared; completeness is not established.'))}</p>`}</section><details id="thread-source" class="thread-source"><summary>${esc(t('Relevant source for the whole thread'))} (${order.length})</summary><p>${esc(t(order.some(id=>!methods[id].source.text)?'Source text is omitted from this export. Open the exact retained revision in the repository.':'Complete selected methods are shown here. Selection does not establish a complete program slice.'))}</p>${source}</details></section>`;
}
function isThreadAnchor(id){return ['thread-source','thread-steps','thread-limits'].includes(id)||id.startsWith('thread-source-')||id.startsWith('thread-step-')||id.startsWith('thread-contract-');}
function revealThreadAnchor(id,endLine){
 const target=$(id);if(!target)return;
 let parent=target.parentElement;while(parent){if(parent.tagName==='DETAILS')parent.open=true;parent=parent.parentElement;}
 document.querySelectorAll('.thread-code-line.source-selected').forEach(line=>line.classList.remove('source-selected'));
 if(id.startsWith('thread-source-')){
  const start=Number(target.dataset?.line),end=Number(endLine)||start;
  const method=target.closest('.thread-method');
  method?.querySelectorAll('.thread-code-line').forEach(line=>{const n=Number(line.dataset.line);if(n>=start&&n<=end)line.classList.add('source-selected');});
 }
 target.scrollIntoView({block:'start',behavior:'instant'});target.focus({preventScroll:true});
}
function routeReaderHash(){
 const raw=location.hash.slice(1),split=raw.indexOf('~');
 if(split>0){const owner=raw.slice(0,split),anchor=raw.slice(split+1);if((D.serviceFragments||[]).some(f=>f.id===owner)&&isThreadAnchor(anchor)){showEntry(owner);history.replaceState(null,'',(location.search||'')+'#'+raw);revealThreadAnchor(anchor);return;}}
 const payloadView=(D.payloadViews||[]).find(v=>'cpv-'+v.operation+'-'+v.id===raw);
 if(payloadView){showEntry(payloadView.operation,'contract');history.replaceState(null,'',(location.search||'')+'#'+raw);$(raw)?.scrollIntoView?.({block:'start'});return;}
 const id=raw;
 if(questionSection(id)){revealQuestion(id);return;}
 const fragment=(D.serviceFragments||[]).find(f=>f.id===id);
 if(fragment){showEntry(id);revealThreadAnchor(id);return;}
 if(isThreadAnchor(id)&&current?.kind==='SERVICE_FRAGMENT'){revealThreadAnchor(id);return;}
 if(isThreadAnchor(id)&&D.processThread){
  if(current?.id!=='process-overview'||$('scenario-content').hidden||!$('catalogue-view').hidden||!$('coverage-view').hidden||!$('scenario-content').querySelector?.('.process-thread'))showEntry('process-overview');
  history.replaceState(null,'',(location.search||'')+'#'+id);revealThreadAnchor(id);return;
 }
 if(location.hash!=='#content')showEntry(id);
}

function processOutlineCard(outline){
 if(!outline)return '';
 if(outline.status!=='STATIC_SOURCE_OUTLINE'){
  const changed=outline.gap==='PROCESS_EVIDENCE_CHANGED_REVIEW_REQUIRED';
  return chromeHtml`<section class="gap-card process-outline-gap"><h3>${esc(t('Static source outline'))}</h3><p>${esc(changed?t('The retained outline no longer matches the selected source evidence.'):t('No qualified local source outline is available.'))}</p><p><code>${esc(outline.gap||'PROCESS_OUTLINE_UNAVAILABLE')}</code></p><details><summary>${esc(t('Exact selected root and retained source bindings'))}</summary><pre class="note-original">${pretty(outline.root||{})}</pre></details></section>`;
 }
 const ids=outline.sourceIds||[];
 const sourceButton=ids.length?chromeHtml`<button class="source-link" data-sources="${esc(ids.join(' '))}" data-source-operation="process-outline">${esc(t('Root method source'))} ↗</button>`:'';
 if(D.sourceExport?.mode==='remote')return chromeHtml`<section class="gap-card"><h3>${esc(t('Static source outline'))}</h3><p>${esc(t('Source text is omitted from this export. Open the exact retained revision in the repository.'))}</p>${sourceButton}</section>`;
 const causal=outline.causal===true;
 return chromeHtml`<div class="diagram-card process-outline"><div class="section-label"><b>${esc(t('Static source outline'))}</b><span>${esc(causal?t('Static source interpretation; runtime execution was not observed.'):t('Control flow not established; evidence gap only.'))}</span></div>${plantumlPreview(outline.diagramStem,outline.svgAvailable,t('Static source outline'))}<pre class="tree">${esc(outline.tree)}</pre><div class="diagram-footer"><span>${esc(t('This generated source outline is not an authored or reviewed explanation.'))}</span>${outline.pumlAvailable?chromeHtml`<a download href="../diagrams/${esc(outline.diagramStem)}.puml">PlantUML ↓</a>`:''}</div>${sourceButton}<details><summary>${esc(t('Exact selected root and retained source bindings'))}</summary><pre class="note-original">${pretty({diagramStem:outline.diagramStem,pumlAvailable:outline.pumlAvailable,svgAvailable:outline.svgAvailable,root:outline.root||{}})}</pre></details></div>`;
}
function relatedViews(entity){const rows=(D.relatedViews||[]).filter(v=>!entity||v.inputObjects.includes(entity));return rows.length?chromeHtml`<section class="gap-card"><h3>Saved entity views</h3>${rows.map(v=>chromeHtml`<p><a href="../scenarios/${esc(v.id)}.html#entity-dataflow">${esc(v.title)} ↗</a></p>`).join('')}</section>`:'';}
function domainAssociationLabel(n,limit=Infinity){const value=typeof n.entity==='string'&&n.entity?n.entity:t('No domain association established');return value.length>limit?value.slice(0,limit-3)+'…':value;}
function dataflowDiagram(g,layout={}){
 const positions=Object.fromEntries(g.nodes.map((n,i)=>[n.id,{column:layout[n.id]?.column??i%3,row:layout[n.id]?.row??Math.floor(i/3)}]));
 const W=Math.max(900,(Math.max(...Object.values(positions).map(p=>p.column))+1)*410),H=Math.max(250,(Math.max(...Object.values(positions).map(p=>p.row))+1)*280);
 const point=id=>({x:30+positions[id].column*410,y:45+positions[id].row*280});
 let svg=chromeHtml`<svg viewBox="0 0 ${W} ${H}" style="min-width:${W}px" role="group" aria-label="Focused entity data flow; matching text records follow"><defs><marker id="flow-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#608365"/></marker></defs>`;
 for(const e of g.edges){
  const a=point(e.from),b=point(e.to),dx=b.x-a.x,dy=b.y-a.y,length=Math.hypot(dx,dy);let path,mx,my;
  if(length===0){path=chromeHtml`M${a.x+255} ${a.y+42} C${a.x+350} ${a.y-25},${a.x+350} ${a.y+172},${a.x+255} ${a.y+105}`;mx=a.x+326;my=a.y+75;}
  else if(dy===0&&Math.abs(dx)>410){mx=(a.x+b.x)/2+125;my=a.y+205;path=chromeHtml`M${a.x+125} ${a.y+146} L${a.x+125} ${my} L${b.x+125} ${my} L${b.x+125} ${b.y+146}`;}
  else {const k=Math.min(125/Math.abs(dx||.0001),70/Math.abs(dy||.0001)),x1=a.x+125+dx*k+dx/length*6,y1=a.y+70+dy*k+dy/length*6,x2=b.x+125-dx*k-dx/length*6,y2=b.y+70-dy*k-dy/length*6;path=chromeHtml`M${x1} ${y1} L${x2} ${y2}`;mx=(x1+x2)/2;my=(y1+y2)/2;}
  const label=e.authority==='UNKNOWN'?t('Unknown candidate'):e.authority==='DECLARED_TRANSFER'?t('Declared transfer'):t(e.kind);
  svg+=chromeHtml`<a href="#entity-dataflow" data-view-edge="${esc(e.id)}" aria-label="${esc(label+': '+e.meaning.text)}"><title>${esc(e.meaning.text)}</title><path d="${path}" fill="none" stroke="transparent" stroke-width="18"/><path d="${path}" fill="none" stroke="#608365" stroke-width="2" ${e.authority==='UNKNOWN'?'stroke-dasharray="6 5"':''} marker-end="url(#flow-arrow)"/><rect x="${mx-74}" y="${my-30}" width="148" height="24" rx="4" fill="#fcfdfb"/><text x="${mx}" y="${my-14}" text-anchor="middle" fill="#425c47" font-size="11">${esc(label)}</text></a>`;
 }
 for(const n of g.nodes){
  const {x,y}=point(n.id),lines=wrap(n.meaning.text,29),shown=lines.slice(0,4),linked=true;if(lines.length>4)shown[3]=shown[3].slice(0,26)+'…';
  svg+=chromeHtml`<${linked?'a':'g'} ${linked?chromeHtml`href="#entity-dataflow" data-view-node="${esc(n.id)}"`:''} aria-label="${esc(t(n.kind)+': '+n.meaning.text)}"><title>${esc(t(n.kind)+' · '+n.service+' · '+domainAssociationLabel(n))}</title><rect x="${x}" y="${y}" width="250" height="140" rx="8" fill="${n.kind==='domain'?'#eff4dc':'#f2f6ef'}" stroke="#d0ddc7"/><text x="${x+14}" y="${y+22}" fill="#6f7e6b" font-size="10">${esc(t(n.kind).toUpperCase()+' · '+n.service.slice(0,25))}</text>${shown.map((t,i)=>chromeHtml`<text x="${x+14}" y="${y+46+i*16}" fill="#263d2d" font-size="12">${esc(t)}</text>`).join('')}<text x="${x+14}" y="${y+124}" fill="#6f7e6b" font-size="9">${esc(domainAssociationLabel(n,38))}</text></${linked?'a':'g'}>`;
 }
 return svg+chromeHtml`</svg>`;
}
function relatedNotes(target){const rows=(D.notes||[]).filter(n=>n.association.targets.includes(target));return rows.length?chromeHtml`<section class="gap-card"><h3>Related notes</h3>${rows.map(n=>chromeHtml`<p><button data-entry="${esc(n.id)}">${esc(n.association.title)}</button> · ${esc(n.association.classification)}</p>`).join('')}</section>`:'';}
const method=e=>e.kind.startsWith('SOURCE_')?'CODE':(e.trigger.methods||[e.kind]).join(' / ');
const chip=m=>chromeHtml`<span class="http-method ${esc(m.toLowerCase())}">${esc(t(m))}</span>`;
function nav(){
 questionServiceReturn();if(questionNav())return;
 const q=$('search').value.toLowerCase().trim();
 const matches=entries.filter(e=>[label(e),e.id,e.symbol,JSON.stringify(e.trigger),e.searchText||''].join(' ').toLowerCase().includes(q));
 const inventory=new Set(inventoryEntries());
 const pending=e=>inventory.has(e)&&!operation(e);
 const row=e=>chromeHtml`<button class="scenario-link" data-entry="${esc(e.id)}" title="${esc(e.symbol||e.id)}" ${current?.id===e.id?'aria-current="page"':''}><span class="nav-icon">${esc(t(method(e)))}</span><span>${esc(label(e))}</span>${pending(e)?chromeHtml`<span class="nav-issue" title="Explicit documentation gap"></span>`:''}</button>`;
 const unexplained=matches.filter(pending);
 $('scenario-nav').innerHTML=matches.filter(e=>!pending(e)).map(row).join('')+(unexplained.length?chromeHtml`<details class="pending-navigation" ${q||unexplained.some(e=>e.id===current?.id)?'open':''}><summary>Awaiting explanation · ${unexplained.length}</summary>${unexplained.map(row).join('')}</details>`:'')||chromeHtml`<p class="no-results">No matching operations</p>`;
}
function wrap(text,max=28){const out=[''];for(const word of text.split(' ')){const i=out.length-1;if(out[i]&&out[i].length+word.length+1>max)out.push(word);else out[i]+=(out[i]?' ':'')+word;}return out;}
function sequenceWrap(text,max){
 const chunks=String(text??'').split(' ').flatMap(word=>{const points=Array.from(word),parts=[];for(let i=0;i<points.length;i+=max)parts.push(points.slice(i,i+max).join(''));return parts.length?parts:[''];});
 return wrap(chunks.join(' '),max);
}
// Project only visible sequence actions. The accepted event stream remains
// intact for readable behavior, source inspection and diagram exports.
function isDeclaredBoundary(e){return e.kind==='declared-boundary'||e.kind==='declared-boundary-return';}
function boundaryEvidenceId(o,e){return 'boundary-evidence-'+o.id+'-'+e.id;}
function boundaryBinding(o,e){
 const rows=(D.operationExternalCalls?.[o.id]||[]).filter(row=>row.interactionId===e.interaction),exact=rows.filter(row=>row.eventId===e.id),candidates=exact.length?exact:rows;
 if(!candidates.length||candidates.some(row=>!row.declarationDigest||!row.snapshotContextDigest))return null;
 const identities=new Set(candidates.map(row=>JSON.stringify([row.declarationDigest,row.snapshotContextDigest,row.declaration,row.transport])));
 return identities.size===1?{row:candidates[0],mode:exact.length?'EXACT_EVENT':'SAME_OPERATION_INTERACTION'}:null;
}
function boundaryEvidence(o,e){
 const binding=boundaryBinding(o,e),row=binding?.row,warning=!row?t('No declaration binding was retained for this event. The current catalogue cannot replace its provenance.'):row.targetChanged?t('The catalogue changed. These details belong to the saved operation.'):t('These details belong to the saved operation.');
 const evidence=row?{event:{id:e.id,kind:e.kind,from:e.from,to:e.to},bindingMode:binding.mode,bindingEventId:row.eventId,interactionId:row.interactionId,from:row.from,to:row.to,declaration:row.declaration,transport:row.transport,declarationDigest:row.declarationDigest,snapshotContextDigest:row.snapshotContextDigest,interactionSourceChecks:row.sourceChecks||{status:'NOT_RETAINED'},authority:row.authority,retained:row.retained,targetChanged:row.targetChanged}:null;
 return chromeHtml`<details id="${esc(boundaryEvidenceId(o,e))}" class="boundary-evidence"><summary>${esc(t('Declared relationship evidence'))}</summary><p>${esc(boundaryCaption(o,e))}</p><p class="schema-description">${esc(warning)}</p>${binding?.mode==='SAME_OPERATION_INTERACTION'?chromeHtml`<p>${esc(t("The event uses the same operation's saved interaction binding; no event-specific binding was retained."))}</p>`:''}${evidence?chromeHtml`<pre class="note-original">${pretty(evidence)}</pre>`:''}</details>`;
}
function sequenceEventLabel(e){return (e.sequenceGuardLabel||e.reader?.label?.text||e.text)+(e.reader?.execution?' · '+t(e.reader.execution.mode):'');}
function boundaryCaption(o,e){const binding=boundaryBinding(o,e);return `${t('Declared boundary; runtime unverified')} · ${binding?.row.declaration?.origin||t('Provenance unavailable')}`;}
function participantHref(p){return p.externalService?'../external-services/'+encodeURIComponent(p.externalService)+'.html':p.service?'../services/'+encodeURIComponent(p.service)+'.html':'';}
function sequenceEvents(o){
 const root=[],stack=[{nodes:root}];
 for(const e of o.events){
  const current=stack[stack.length-1];
  if(['alt','loop','opt'].includes(e.kind)){
   const arm={marker:e,nodes:[]},frame={arms:[arm],end:null};
   current.nodes.push(frame);stack.push({frame,nodes:arm.nodes});
  }else if(e.kind==='else'&&current.frame?.arms[0].marker.kind==='alt'){
   const arm={marker:e,nodes:[]};current.frame.arms.push(arm);current.nodes=arm.nodes;
  }else if(e.kind==='end'&&stack.length>1){current.frame.end=e;stack.pop();}
  else if(e.kind!=='note'){current.nodes.push({event:e});}
 }
 const project=nodes=>nodes.flatMap(node=>{
  if(node.event)return [node.event];
  const arms=node.arms.map((arm,index)=>({...arm,index,events:project(arm.nodes)})).filter(arm=>arm.events.length);
  if(!arms.length)return [];
  const first=arms[0];
  // A surviving alternative stays conditional on earlier branches not being
  // taken. Keep their authored guards and make that negative condition explicit.
  const marker=first.index===0?first.marker:{...first.marker,kind:'alt',text:[`${t(first.index===1?'Condition not met':'Earlier branches not taken')}: ${node.arms.slice(0,first.index).map(arm=>arm.marker.text).join('; ')}`,first.marker.text.toLowerCase()==='otherwise'?'':`${t('Selected branch')}: ${first.marker.text}`].filter(Boolean).join(' · '),sourceIds:[...new Set(node.arms.slice(0,first.index+1).flatMap(arm=>arm.marker.sourceIds||[]))],dependencyIds:[...new Set(node.arms.slice(0,first.index+1).flatMap(arm=>arm.marker.dependencyIds||[]))]};
  if(first.index>0&&node.arms.slice(0,first.index+1).some(arm=>arm.marker.reader?.label))marker.sequenceGuardLabel=[`${t(first.index===1?'Condition not met':'Earlier branches not taken')}: ${node.arms.slice(0,first.index).map(arm=>arm.marker.reader?.label?.text||arm.marker.text).join('; ')}`,`${t('Selected branch')}: ${first.marker.reader?.label?.text||first.marker.text}`].join(' · ');
  return [marker,...first.events,...arms.slice(1).flatMap(arm=>[arm.marker,...arm.events]),...(node.end?[node.end]:[])];
 });
 return project(root);
}
let sequenceMeasureContext;
function sequenceLines(text,width,font,maxChars=Infinity,mono=false){
 if(sequenceMeasureContext===undefined)sequenceMeasureContext=document.createElement?.('canvas')?.getContext?.('2d')||null;
 const style=window.getComputedStyle?.(document.body),family=mono?(style?.getPropertyValue?.('--mono')?.trim()||'monospace'):(style?.getPropertyValue?.('--clew-sans')?.trim()||style?.fontFamily||'sans-serif');
 if(sequenceMeasureContext)sequenceMeasureContext.font=font+' '+family;
 const measure=value=>sequenceMeasureContext?sequenceMeasureContext.measureText(value).width:Array.from(value).length*(font.includes('8px')?4.9:6.8);
 const fits=value=>Array.from(value).length<=maxChars&&measure(value)<=Math.max(12,width);
 const lines=[];let line='';
 for(const word of String(text??'').match(/\s+|[^\s/.,;(){}]+|[/.,;(){}]/gu)||['']){
  if(/^\s+$/.test(word)){if(line&&!line.endsWith(' '))line+=' ';continue;}
  if(fits(line+word)){line+=word;continue;}
  if(line){lines.push(line.trimEnd());line='';}
  if(fits(word)){line=word;continue;}
  for(const point of Array.from(word)){if(line&&!fits(line+point)){lines.push(line);line='';}line+=point;}
 }
 if(line||!lines.length)lines.push(line.trimEnd());
 return lines;
}
function sequenceLayout(o,availableWidth=800){
 const events=sequenceEvents(o),referenced=new Set(events.filter(e=>['message','return','declared','declared-boundary','declared-boundary-return'].includes(e.kind)).flatMap(e=>[e.from,e.to]));
 const participants=o.participants.filter(p=>referenced.has(p.id));
 const W=Math.max(160,Math.floor(availableWidth),participants.length*240+32),padding=16,gap=8,actorWidth=Math.min(224,Math.max(24,(W-padding*2-gap*Math.max(0,participants.length-1))/Math.max(1,participants.length)));
 const first=padding+actorWidth/2,last=W-padding-actorWidth/2,xs=Object.fromEntries(participants.map((p,i)=>[p.id,participants.length===1?W/2:first+i*(last-first)/Math.max(1,participants.length-1)]));
 // Match the shared SVG typography; measured wrapping and rendered baselines
 // must use the same readable font size, including destination IDs and status.
 const actorLineHeight=18,titleY=29;
 const actors=participants.map(p=>{
  const titleLines=sequenceLines(p.label,actorWidth-16,'600 14px'),technicalLines=sequenceLines(p.externalService?p.externalService:p.service||t('External participant'),actorWidth-16,'14px');
  return {...p,width:actorWidth,titleLines,technicalLines,titleY,technicalY:titleY+titleLines.length*actorLineHeight+6};
 });
 const headerBottom=Math.max(57,...actors.map(p=>p.technicalY+(p.technicalLines.length-1)*actorLineHeight+12)),headerHeight=headerBottom-9;
 let y=headerBottom+37,stack=[],frames=[],rows=[];
 for(const e of events){
  const depth=stack.length,branch=['alt','else','loop','opt'].includes(e.kind),inset=Math.min(Math.max(0,depth-(e.kind==='else'?1:0))*12,Math.max(0,(W-160)/4)),left=24+inset;
  const a=xs[e.from]??W/2,b=xs[e.to]??W/2,self=a===b&&!branch&&e.kind!=='end',direction=W-padding-a>=a-padding?1:-1;
  const selfSpace=direction>0?W-padding-a:a-padding,selfWidth=Math.max(12,Math.min(180,selfSpace-8)),loopWidth=Math.max(8,Math.min(85,selfSpace-8));
  const labelWidth=self?selfWidth:Math.max(12,Math.abs(a-b)-16),mid=self?a+direction*selfWidth/2:(a+b)/2;
  const lines=e.kind==='end'?[]:sequenceLines(branch?`${t(e.kind)} · ${sequenceEventLabel(e)}`:sequenceEventLabel(e),branch?W-left*2-12:labelWidth,branch?'600 14px':'14px');
  const arrowOffset=Math.max(39,(lines.length-1)*18+20),arrowBottom=arrowOffset+(a===b?17:0),captionOffset=arrowBottom+18;
  const captionLines=(e.kind==='declared'||isDeclaredBoundary(e))?sequenceLines(isDeclaredBoundary(e)?t(e.kind==='declared-boundary-return'?'Local boundary return':'Declared call'):t('Declared service interaction'),labelWidth,'14px'):[];
  const h=e.kind==='end'?18:branch?Math.max(40,lines.length*18+14):Math.max(72,((e.kind==='declared'||isDeclaredBoundary(e))?captionOffset+Math.max(0,captionLines.length-1)*18:arrowBottom)+28);
  rows.push({e,y,h,depth,left,lines,arrowOffset,captionOffset,captionLines,mid,labelWidth,direction,loopWidth});
  if(['alt','loop','opt'].includes(e.kind))stack.push({y,depth});
  if(e.kind==='end'){const frame=stack.pop();if(frame)frames.push({...frame,end:y+h-5});}
  y+=h;
 }
 return {W,H:y+25,xs,actors,actorLineHeight,headerHeight,headerBottom,frames:frames.sort((a,b)=>a.depth-b.depth),rows};
}
function diagram(o,availableWidth){
 const {W,H,xs,actors,actorLineHeight,headerHeight,headerBottom,frames,rows}=sequenceLayout(o,availableWidth);
 if(!actors.length||!rows.some(({e})=>['message','return','declared','declared-boundary','declared-boundary-return'].includes(e.kind)))return '';
 let svg=chromeHtml`<svg class="sequence-svg" data-sequence-operation="${esc(o.id)}" data-layout-width="${Math.max(160,Math.floor(availableWidth??800))}" style="width:100%;min-width:${W}px;height:auto" viewBox="0 0 ${W} ${H}" role="group" aria-label="Sequence diagram: ${esc(o.title)}"><defs><marker id="arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="6" markerHeight="6" orient="auto-start-reverse"><path d="M0 0 L8 4 L0 8Z" fill="#608365"/></marker></defs>`;
 svg+=frames.map(f=>{const inset=16+Math.min(f.depth*12,Math.max(0,(W-160)/4));return chromeHtml`<rect x="${inset}" y="${f.y-8}" width="${W-inset*2}" height="${f.end-f.y+8}" rx="4" fill="#fdfbf6" stroke="#e3d9c5"/>`;}).join('');
 svg+=actors.map(p=>chromeHtml`${participantHref(p)?chromeHtml`<a href="${esc(participantHref(p))}" aria-label="${esc(p.label)}: documentation"><title>${esc(p.label)}: documentation</title>`:''}<line class="lifeline" x1="${xs[p.id]}" x2="${xs[p.id]}" y1="${headerBottom+4}" y2="${H-8}"/><rect x="${xs[p.id]-p.width/2}" y="9" width="${p.width}" height="${headerHeight}" rx="6" fill="#f1f6ed" stroke="#d6e1d0"/>${p.titleLines.map((line,i)=>chromeHtml`<text class="actor-label" text-anchor="middle" x="${xs[p.id]}" y="${p.titleY+i*actorLineHeight}">${esc(line)}</text>`).join('')}${p.technicalLines.map((line,i)=>chromeHtml`<text class="actor-technical" text-anchor="middle" x="${xs[p.id]}" y="${p.technicalY+i*actorLineHeight}">${esc(line)}</text>`).join('')}${participantHref(p)?'</a>':''}`).join('');
 let number=0;
 for(const {e,y,h,left,lines,arrowOffset,captionOffset,captionLines,mid,labelWidth,direction,loopWidth} of rows){if(e.kind==='end')continue;number++;let inside='';
 if(['alt','else','loop','opt'].includes(e.kind)){
  inside=chromeHtml`${e.kind==='else'?chromeHtml`<line x1="${left-12}" y1="${y-8}" x2="${W-left+12}" y2="${y-8}" stroke="#ddd2bd" stroke-dasharray="4 3"/>`:''}<rect class="hit-area" x="${left-5}" y="${y-5}" width="${W-left*2+10}" height="${h-9}" rx="3" fill="transparent"/>`+lines.map((line,i)=>chromeHtml`<text class="branch-title" x="${left}" y="${y+13+i*18}">${esc(line)}</text>`).join('');
 }else{
  const a=xs[e.from],b=xs[e.to],start=a===b?Math.min(a,a+direction*labelWidth):Math.min(a,b),width=a===b?labelWidth:Math.abs(a-b),arrowY=y+arrowOffset;
  inside=chromeHtml`<rect class="hit-area" x="${Math.max(0,start-12)}" y="${y-14}" width="${Math.min(W-Math.max(0,start-12),width+24)}" height="${h+5}" rx="4" fill="transparent"/><text class="step-number" x="${Math.max(4,Math.min(a,b)-12)}" y="${arrowY}">${number}</text>`+lines.map((line,i)=>chromeHtml`<text text-anchor="middle" x="${mid}" y="${y+i*18}">${esc(line)}</text>`).join('');
  const dash=['return','declared','declared-boundary','declared-boundary-return'].includes(e.kind)?'event-return':'';
  inside+=a===b?chromeHtml`<path class="event-arrow ${dash}" d="M${a} ${arrowY} h${direction*loopWidth} v17 h${-direction*loopWidth}" marker-end="url(#arrow)"/>`:chromeHtml`<line class="event-arrow ${dash}" x1="${a}" y1="${arrowY}" x2="${b}" y2="${arrowY}" marker-end="url(#arrow)"/>`;
  if((e.kind==='declared'||isDeclaredBoundary(e)))inside+=captionLines.map((line,i)=>chromeHtml`<text class="branch-title" text-anchor="middle" x="${mid}" y="${y+captionOffset+i*18}">${esc(line)}</text>`).join('');
 }
 svg+=isDeclaredBoundary(e)?chromeHtml`<a href="#${esc(boundaryEvidenceId(o,e))}" data-boundary-declaration="${esc(boundaryEvidenceId(o,e))}" aria-label="${esc(e.text)}: ${esc(t('Declared relationship evidence'))}"><title>${esc(boundaryCaption(o,e))}</title>${inside}</a>`:chromeHtml`<a href="#${esc(o.id)}" data-event="${esc(e.id)}" data-sequence-event="true" aria-label="${esc(e.text)}: inspect source"><title>${esc(e.text)}</title>${inside}</a>`;
 }return svg+chromeHtml`</svg>`;
}
let sequenceReflowPending=false,sequenceContainers=[];
function scheduleSequenceReflow(){
 if(sequenceReflowPending)return;sequenceReflowPending=true;
 const run=()=>{sequenceReflowPending=false;reflowSequences();};
 if(typeof requestAnimationFrame==='function')requestAnimationFrame(run);else run();
}
const sequenceResizeObserver=typeof ResizeObserver==='function'?new ResizeObserver(scheduleSequenceReflow):null;
function reflowSequences(){
 updateSequencePresentation();
 const svgs=Array.from(document.querySelectorAll('.sequence-svg')),containers=svgs.map(svg=>svg.parentElement).filter(Boolean);
 for(const parent of sequenceContainers)if(!containers.includes(parent))sequenceResizeObserver?.unobserve(parent);
 for(const parent of containers)if(!sequenceContainers.includes(parent))sequenceResizeObserver?.observe(parent);
 sequenceContainers=containers;
 for(const svg of svgs){
  const parent=svg.parentElement;if(!parent)continue;
  const style=window.getComputedStyle?.(parent),width=Math.floor((parent.clientWidth||0)-(parseFloat(style?.paddingLeft)||0)-(parseFloat(style?.paddingRight)||0));
  if(width<160||Number(svg.getAttribute('data-layout-width'))===width)continue;
  const owner=D.operations.find(o=>o.id===svg.getAttribute('data-sequence-operation'));if(owner)svg.outerHTML=diagram(owner,width);
 }
}
if(typeof MutationObserver==='function')new MutationObserver(scheduleSequenceReflow).observe($('scenario-content'),{childList:true,subtree:true});
window.addEventListener('resize',scheduleSequenceReflow);
document.addEventListener('toggle',scheduleSequenceReflow,true);
function pseudocodeEvent(o,event){
 const participant=id=>o.participants.find(p=>p.id===id)?.label||id;
 const route=event.from&&event.to?`${participant(event.from)} → ${participant(event.to)}: `:'';
 const text=event.text||'';
 const startsWithKeyword=keyword=>[keyword,CLEW_RU_MESSAGES[keyword]].filter(Boolean).some(label=>{
  const value=text.trimStart().toLowerCase(),prefix=label.toLowerCase();
  return value===prefix||value.startsWith(prefix)&&/^[\s:]/u.test(value.slice(prefix.length));
 });
 switch(event.kind){
  case 'alt':return {keyword:startsWithKeyword('If')?'':t('If'),text:text?text+':':''};
  case 'else':return text&&["otherwise",String(CLEW_RU_MESSAGES.Otherwise).toLowerCase()].includes(text.trim().toLowerCase())?{keyword:t('Otherwise'),text:''}:{keyword:startsWithKeyword('Otherwise')?'':t('Otherwise'),text:text?text+':':''};
  case 'loop':return {keyword:t('Loop'),text:text?text+':':''};
  case 'opt':return {keyword:t('When'),text:text?text+':':''};
  case 'return':return {keyword:t('Return'),text:route+text};
  case 'message':return {keyword:t('Message'),text:route+text};
  case 'declared-boundary':case 'declared-boundary-return':return {keyword:boundaryCaption(o,event),text:route+text};
  case 'declared':return {keyword:t('Declared interaction'),text:(event.from&&event.to?`(${participant(event.from)} → ${participant(event.to)}) `:'')+text};
  case 'note':return {keyword:t('Note'),text};
  default:return {keyword:'',text};
 }
}
function stepEvidence(owner,event){
 if(isDeclaredBoundary(event))return boundaryEvidence(owner,event);
 return event.sourceIds?.length?chromeHtml`<button class="source-link" data-sources="${esc(event.sourceIds.join(' '))}" data-source-operation="${esc(owner.id)}" data-source-claim="${esc(event.id)}">${esc(t('Step evidence'))} ↗</button>`:'';
}
function externalCallDetails(owner,event){
 const rows=(D.operationExternalCalls?.[owner.id]||[]).filter(row=>row.eventId===event.id);
 return rows.map(row=>chromeHtml`<section class="external-call-details" aria-label="${esc(t('External call details'))}"><strong>${esc(t('External service'))}: ${esc(row.to)} · ${esc(row.title)}</strong>${row.bindingStatus==='MULTIPLE_DECLARED_INTERACTIONS'?chromeHtml`<p class="finding-strip">${esc(t('Multiple declarations match this step; no destination was selected automatically.'))}</p>`:''}${row.targetChanged?chromeHtml`<p class="finding-strip">${esc(t('The declaration or contract changed. These retained details belong to the earlier accepted operation.'))}</p>`:''}${(row.addresses||[]).length?row.addresses.map(address=>chromeHtml`<p class="external-address"><code>${esc(address.url)}</code><br><small>${esc(t('Environment'))}: ${esc(address.environment||t('unspecified'))} · ${esc(t('Provided from'))}: ${esc(address.source)}</small></p>`).join(''):chromeHtml`<p class="empty-note">${esc(t(row.transport?.kind==='logical'?'Physical transport and address are unknown for this logical port.':'Address not provided. Set addresses in the interaction declaration.'))}</p>`}<p class="schema-description">${esc(t('Declared destination; runtime binding is unverified.'))}</p><details class="external-call-contract"><summary>${esc(t('Contract'))}${row.contract?' · '+esc(row.contract.normalized.method)+' '+esc(row.contract.normalized.path):''}</summary>${row.contract?contractRows([row.contract],owner.id):chromeHtml`<p class="empty-note">${esc(t(row.contractStatus==='REFERENCE_UNAVAILABLE'?'The referenced contract is unavailable in this saved snapshot.':row.contractStatus==='AMBIGUOUS_RECEIVER_CONTRACT'?'Multiple saved receiver contracts match. No contract was selected.':'No outgoing contract is bound.'))} ${esc(t('Set contractReference to an exact saved CONTRACT_OPERATION ID in the interaction declaration.'))}</p>`}</details><details class="external-call-provenance"><summary>${esc(t('Declaration and provenance'))}</summary><p>${esc(t('Origin'))}: ${esc(row.declaration?.origin)}<br>${esc(row.declaration?.rationale)}</p><p>${esc(t('Declared environments'))}: ${esc((row.applicability?.environments||[]).join(', ')||t('unspecified'))}</p><code>${esc(row.interactionId)}</code><p>${esc(t('Saved declaration context'))}: <code>${esc(row.snapshotContextDigest)}</code></p></details></section>`).join('');
}
function behaviorPseudocode(o){
 const events=o.events||[];
 if(!events.some(event=>event.kind!=='end'))return '';
 const rows=[],stack=[];let depth=0;
 for(const event of events){
  if(event.kind==='end'){
   const frame=stack.pop();if(frame)depth=frame.depth;
   continue;
  }
  if(event.kind==='else'){
   const frame=stack[stack.length-1];if(frame?.kind==='alt')depth=frame.depth;
   rows.push({event,line:pseudocodeEvent(o,event),depth});
   if(frame?.kind==='alt')depth=frame.depth+1;
   continue;
  }
  rows.push({event,line:pseudocodeEvent(o,event),depth});
  if(['alt','loop','opt'].includes(event.kind)){stack.push({kind:event.kind,depth});depth++;}
 }
 if(!rows.length)return '';
 return compactProcess(o)+chromeHtml`<details class="full-process-algorithm"><summary>${esc(t('Full algorithm and source commentary'))}</summary><section class="diagram-card behavior-pseudocode" aria-label="${esc(t('Readable behavior'))}"><div class="section-label"><b>${esc(t('Readable behavior'))}</b><span>${esc(t('Projection of documented step narratives; not executable code or an observed runtime trace.'))}</span></div><div class="behavior-pseudocode-lines" role="list">${rows.map(({event,line,depth})=>chromeHtml`<div class="behavior-pseudocode-line" role="listitem" style="--indent:${depth*20}px;--indent-mobile:${depth*13}px"><code class="pseudocode-keyword">${esc(line.keyword)}</code><div class="pseudocode-text clew-markdown">${prose(line.text)}</div>${stepEvidence(o,event)}${externalCallDetails(o,event)}</div>`).join('')}</div></section></details>`;
}
function redundantDisplayedExplanation(p,displayedEvents){
 if(p.detail||p.authorship||!p.text||!Array.isArray(p.eventIds)||p.eventIds.length!==1)return false;
 const linked=displayedEvents.filter(event=>event.id===p.eventIds[0]);
 if(linked.length!==1||linked[0].kind==='end'||linked[0].text!==p.text)return false;
 const sameReferences=(a,b)=>Array.isArray(a)&&Array.isArray(b)&&a.every(id=>typeof id==='string')&&b.every(id=>typeof id==='string')&&new Set(a).size===new Set(b).size&&a.every(id=>b.includes(id));
 return sameReferences(p.dependencyIds,linked[0].dependencyIds)&&sameReferences(p.sourceIds,linked[0].sourceIds);
}
function explanation(o,detail=false,displayedEvents=[]){
 const paragraphs=new Map();
 for(const p of (o.explanation||[]).filter(p=>!!p.detail===detail)){
  if(redundantDisplayedExplanation(p,displayedEvents))continue;
  const key=JSON.stringify([p.text,p.eventIds||null,p.dependencyIds||null,p.sourceIds||null,p.authorship||null]);
  const previous=paragraphs.get(key);
  if(previous)previous.sourceIds=[...new Set([...previous.sourceIds,...p.sourceIds])];
  else paragraphs.set(key,{...p,sourceIds:[...p.sourceIds]});
 }
 return paragraphs.size?chromeHtml`<section class="operation-explanation" aria-label="${detail?t('Implementation commentary'):t('What happens')}">${detail?'':chromeHtml`<h3>What happens</h3>`}${[...paragraphs.values()].map(p=>chromeHtml`<div class="explanation-paragraph"><div class="clew-markdown">${prose(p.text)}</div>${p.authorship?chromeHtml`<p class="schema-description">User documentation by ${esc(p.authorship.author)}. Meaning review: UNASSESSED. ${p.authorship.contextMigration?chromeHtml`Context selected by ${esc(p.authorship.contextMigration.editor)}. Context review: UNASSESSED. Explicitly selected code remains unverified context. Current freshness does not establish semantic review.`:'Originally linked code is retained unverified context.'} Source context freshness: ${esc(D.fragmentStates?.[D.subject+'/'+o.id+'/'+p.id]?.freshness||'UNVERIFIED')}.</p>`:''}<div class="explanation-evidence">${p.authorship?(p.sourceIds.length?chromeHtml`<button class="source-link" data-sources="${esc(p.sourceIds.join(' '))}" data-source-operation="${esc(o.id)}" data-source-fragment="${esc(D.subject+'/'+o.id+'/'+p.id)}">${p.authorship.contextMigration?'Explicitly selected code (unverified context)':'Originally linked code (unverified context)'} ↗</button>`:''):claimButton(p,t('Supporting code'))}</div></div>`).join('')}</section>`:'';
}
function interactionOverview(o){
 const seen=new Set(),links=o.events.filter(e=>(e.kind==='declared'||isDeclaredBoundary(e))&&!seen.has(e.interaction)&&seen.add(e.interaction));
 if(!links.length)return '';
 const name=id=>o.participants.find(p=>p.id===id)?.label||id;
 return chromeHtml`<section class="interaction-overview" aria-label="Service links"><h3>Service links</h3><p class="schema-description">Declared connections in this scenario. Conditions are explained below; this map does not imply synchronous execution.</p><div class="interaction-links">${links.map(e=>chromeHtml`<button class="interaction-link" data-event="${esc(e.id)}"><span>${esc(name(e.from))}</span><span class="connection-arrow" aria-hidden="true">→</span><span>${esc(name(e.to))}</span><small>${esc(e.text)}</small></button>`).join('')}</div></section>`;
}
function overviewDiagram(o){
 const d=o.overviewDiagram;
 if(!d)return diagram(o);
 const W=(Math.max(...d.nodes.map(n=>n.column))+1)*280+20,H=(Math.max(...d.nodes.map(n=>n.row))+1)*160+30;
 const pos=Object.fromEntries(d.nodes.map(n=>[n.id,{x:150+n.column*280,y:90+n.row*160}]));
 let svg=chromeHtml`<svg class="overview-svg" viewBox="0 0 ${W} ${H}" role="group" aria-label="Overview diagram: ${esc(o.title)}"><defs><marker id="overview-arrow" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="7" markerHeight="7" orient="auto"><path d="M0 0 L8 4 L0 8Z" fill="#64806a"/></marker></defs>`;
 for(const e of d.edges){const a=pos[e.from],b=pos[e.to],dx=b.x-a.x,dy=b.y-a.y;let x1=a.x,y1=a.y,x2=b.x,y2=b.y;
 if(Math.abs(dx)>=Math.abs(dy)){x1+=Math.sign(dx)*108;x2-=Math.sign(dx)*108;}else{y1+=Math.sign(dy)*48;y2-=Math.sign(dy)*48;}
 const mx=(x1+x2)/2,my=(y1+y2)/2,vertical=Math.abs(dx)<Math.abs(dy),tx=vertical?mx+12:mx,ty=vertical?my:my-12;
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-overview-edge="${esc(e.id)}" aria-label="${esc(e.text||t('Transition'))}: inspect source"><title>${esc(e.text||t('Transition'))}</title><path d="M${x1} ${y1} L${x2} ${y2}" fill="none" stroke="#64806a" stroke-width="2" marker-end="url(#overview-arrow)"/><text class="overview-edge-label" text-anchor="${vertical?'start':'middle'}" x="${tx}" y="${ty}">${esc(e.text)}</text></a>`;
 }
 for(const n of d.nodes){const p=pos[n.id],actor=o.participants.find(a=>a.id===n.participant),lines=wrap(n.text,25);
 svg+=chromeHtml`<a href="#${esc(o.id)}" data-overview-node="${esc(n.id)}" aria-label="${esc(n.text)}: inspect source"><title>${esc(n.text)}</title><rect x="${p.x-108}" y="${p.y-48}" width="216" height="96" rx="10" fill="#f3f7ef" stroke="#cad8c3"/>${lines.map((line,i)=>chromeHtml`<text class="overview-node-label" text-anchor="middle" x="${p.x}" y="${p.y-23+i*17}">${esc(line)}</text>`).join('')}<text class="overview-actor" text-anchor="middle" x="${p.x}" y="${p.y+34}">${esc(actor?.service||actor?.label||'')}</text></a>`;
 }
 return svg+chromeHtml`</svg>`;
}
function entryProcessLinks(id){
 return (D.entryProcesses?.[id]||[]).map(process=>chromeHtml`<p><a href="${esc(process.href)}">Saved process: ${esc(process.title)} →</a><br><span class="small-label">Retained explanation · ${esc(statusLabel(process.state?.freshness||'UNVERIFIED'))} · ${esc(meaningReviewLabel(process.state?.verification||'UNASSESSED'))}</span></p>`).join('');
}
function operationContext(e){
 if(!D.subject.startsWith('service:'))return '';
 const processes=entryProcessLinks(e.id),data=(D.sections||[]).some(section=>section.id==='section-entities');
 if(!processes&&!data)return '';
 return chromeHtml`<section class="gap-card operation-context"><h3>Related documentation</h3>${processes}${data?chromeHtml`<p><button class="source-link" data-entry="section-entities">Service data and domain context →</button></p>`:''}</section>`;
}
function sequenceTrace(o){
 const actors=new Map(o.participants.map(p=>[p.id,p.label||p.service||p.externalService||p.id])),guards=[];let number=0;
 const rows=[];
 for(const event of o.events||[]){
  if(event.kind==='end'){guards.pop();continue;}
  const branch=['alt','else','opt','loop'].includes(event.kind),title=event.reader?.label?.text||event.text;
  if(event.kind==='else'&&guards.length){const guard=guards[guards.length-1];guard.label=t('Condition not met')+': '+guard.original+' · '+title;}
  const conditions=guards.map(g=>g.label);
  const kind=event.kind==='message'?'Call':event.kind==='return'?'Return':event.kind==='note'?'Note':event.kind==='declared-boundary-return'?'Local boundary return':event.kind==='declared-boundary'||event.kind==='declared'?'Declared call':event.kind;
  const proof=isDeclaredBoundary(event)?boundaryEvidence(o,event).replace(`id="${esc(boundaryEvidenceId(o,event))}"`,`id="trace-${esc(boundaryEvidenceId(o,event))}"`):stepEvidence(o,event);
  rows.push(chromeHtml`<li class="sequence-trace-step ${branch?'sequence-trace-guard':''}" data-trace-event="${esc(event.id)}"><div class="sequence-trace-kind">${++number}. ${esc(t(kind))} ${executionBadge(event)}</div>${!branch&&event.from&&event.to?chromeHtml`<div class="sequence-trace-route"><strong>${esc(actors.get(event.from)||event.from)}</strong><span aria-hidden="true">→</span><strong>${esc(actors.get(event.to)||event.to)}</strong></div>`:''}<h4>${esc(title)}</h4>${!branch&&conditions.length?chromeHtml`<p class="sequence-trace-conditions"><b>${esc(t('Conditions'))}:</b> ${esc(conditions[conditions.length-1])}</p>${conditions.length>1?chromeHtml`<details class="sequence-trace-ancestry"><summary>${esc(t('All conditions'))} (${conditions.length})</summary><ul>${conditions.map(condition=>chromeHtml`<li>${esc(condition)}</li>`).join('')}</ul></details>`:''}`:''}<details class="sequence-trace-evidence"><summary>${esc(t('Step evidence'))}</summary>${title!==event.text?chromeHtml`<div class="clew-markdown">${prose(event.text)}</div>`:''}${event.reader?.execution&&event.reader.execution.meaning.text!==event.text?chromeHtml`<p>${esc(event.reader.execution.meaning.text)}</p>`:''}${claimButton(event.reader?.label,t('Label evidence'))}${event.reader?.execution?claimButton(event.reader.execution.meaning,t('Execution evidence')):''}${proof}</details></li>`);
  if(['alt','opt','loop'].includes(event.kind))guards.push({original:title,label:title});
 }
 return chromeHtml`<section class="sequence-trace" aria-label="${esc(t('Step-by-step trace'))}"><p>${esc(t('Sender → step → receiver'))}</p><ol>${rows.join('')}</ol></section>`;
}
function adaptiveSequence(o,diagramMarkup){
 const narrow=window.matchMedia?.('(max-width:700px)')?.matches===true;
 return chromeHtml`<div class="sequence-adaptive" data-presentation="${narrow?'narrow':'wide'}"><div class="sequence-trace-panel" ${narrow?'':'hidden'}>${sequenceTrace(o)}</div><details class="full-sequence-diagram" ${narrow?'':'open'}><summary ${narrow?'':'hidden'}>${esc(t('Full sequence diagram'))}</summary>${diagramMarkup}</details></div>`;
}
function updateSequencePresentation(){
 const mode=window.matchMedia?.('(max-width:700px)')?.matches?'narrow':'wide';
 for(const root of document.querySelectorAll('.sequence-adaptive')){
  if(!root.querySelector||!root.dataset||root.dataset.presentation===mode)continue;
  const panel=root.querySelector('.sequence-trace-panel'),diagram=root.querySelector('.full-sequence-diagram'),summary=diagram?.querySelector('summary');
  if(panel)panel.hidden=mode!=='narrow';if(summary)summary.hidden=mode!=='narrow';if(diagram)diagram.open=mode==='wide';root.dataset.presentation=mode;
 }
}
function sequence(e,o){if(!o)return chromeHtml`<p class="empty-note">${esc(D.gaps[e.id]||t('Behavior documentation has not been authored for this entrypoint.'))}</p>`;
 const state=D.stateDiagram?chromeHtml`<div class="diagram-card"><div class="section-label"><b>${esc(t('State diagram'))}</b><span>${esc(t('Declared process-state schema'))} · ${esc(t('Static source bindings do not show runtime execution.'))}</span></div>${plantumlPreview(D.stateDiagram,D.stateDiagramSvg,'State diagram')}<div class="diagram-footer"><span>${esc(t('States and transitions come from the captured process-state schema.'))} scenarios/${esc(D.subject.split(':').pop())}-states.yaml</span><a download href="../diagrams/${esc(D.stateDiagram)}.puml">PlantUML ↓</a></div></div>`:'';
 const pseudocode=behaviorPseudocode(o);
 const displayedEvents=pseudocode?(o.events||[]).filter(event=>event.kind!=='end'&&typeof event.text==='string'&&event.text.length>0&&pseudocodeEvent(o,event).text.includes(event.text)):[];
 const diagramMarkup=overviewDiagram(o);
 const sequenceFlow=diagramMarkup?chromeHtml`<div class="diagram-card"><div class="section-label"><b>Scenario overview</b><span>Select a node or connection to inspect its source</span></div><p class="sequence-pan-hint">${esc(t('Scroll horizontally to see linked participants.'))}${o.participants.some(p=>p.externalService)?' '+esc(t('External participants have no captured source.')):''}</p><div class="overview-scroll" tabindex="0" role="region" aria-label="${esc(t('Sequence diagram'))}">${diagramMarkup}</div><div class="diagram-footer"><span>Source-based interpretation; declared links do not prove runtime delivery.</span><a download href="../diagrams/${esc(D.subject.replace(':','-'))}-${esc(o.id)}.mmd">Mermaid ↓</a></div></div>`:'';
 const accessible=o.overviewDiagram?chromeHtml`<details class="steps-accessible"><summary>Diagram as text</summary><ul>${o.overviewDiagram.nodes.map(n=>chromeHtml`<li><button data-overview-node="${esc(n.id)}">${esc(n.text)}</button></li>`).join('')}</ul><ul>${o.overviewDiagram.edges.map(e=>chromeHtml`<li>${esc(o.overviewDiagram.nodes.find(n=>n.id===e.from).text)} → ${esc(o.overviewDiagram.nodes.find(n=>n.id===e.to).text)}${e.text?' · '+esc(e.text):''}</li>`).join('')}</ul></details>`:'';
 const adaptive=sequenceFlow?adaptiveSequence(o,sequenceFlow):'';
 const flow=!sequenceFlow&&!accessible?'':pseudocode?chromeHtml`<details class="sequence-details"><summary>${esc(t(o.overviewDiagram?'Scenario overview':'Sequence diagram'))}</summary>${adaptive}${accessible}</details>`:chromeHtml`${adaptive}${accessible}`;
 const narrativeDetails=pseudocode?chromeHtml`<details class="implementation-detail"><summary>Implementation details and source commentary</summary>${explanation(o,false,displayedEvents)}${explanation(o,true)}</details>`:explanation(o);
 const implementationDetails=!pseudocode?chromeHtml`<details class="implementation-detail"><summary>Implementation details and source commentary</summary>${explanation(o,true)}</details>`:'';
 return chromeHtml`${operationContext(e)}${(o.visuals||[]).map(v=>visualCard(o,v)).join('')}${pseudocode}${state}${flow}${narrativeDetails}${o.boundaries.length?chromeHtml`<details class="technical-evidence"><summary>Scope and evidence boundaries</summary><ul>${o.boundaries.map(b=>chromeHtml`<li>${esc(b)}</li>`).join('')}</ul></details>`:''}${implementationDetails}<details class="technical-evidence"><summary>Declared service links</summary>${interactionOverview(o)||chromeHtml`<p>No cross-service connections selected.</p>`}</details>`;}
function inlineDeclarationBody(d){
 if(d.kind==='OPENAPI_DOCUMENT')return (d.operations||[]).map(operation=>chromeHtml`<section class="contract-operation"><h5><code>${esc(operation.method)} ${esc(operation.path)}</code></h5>${['operationId','summary','description'].map(key=>operation[key]?chromeHtml`<p>${esc(operation[key])}</p>`:'').join('')}${(operation.parts||[]).map(part=>chromeHtml`<h6>${esc(part.title)}</h6>${typeof part.required==='boolean'?chromeHtml`<p>${esc(t('Required'))}: ${esc(String(part.required))}</p>`:''}${part.description?chromeHtml`<p>${esc(part.description)}</p>`:''}${part.reference?chromeHtml`<p>${esc(t('Reference'))}: <code>${esc(part.reference)}</code></p>`:''}${part.declaration?inlineDeclarationBody(part.declaration):''}`).join('')}</section>`).join('');
 const headers=['Field','Type','Presence','Accepts null','Declared constraints / description'];
 return chromeHtml`<div class="table-wrap"><table class="inline-contract-table"><thead><tr>${headers.map(h=>chromeHtml`<th scope="col">${esc(t(h))}</th>`).join('')}</tr></thead><tbody>${(d.nodes||[]).map(n=>chromeHtml`<tr><td data-label="${esc(t(headers[0]))}"><code>${esc(n.field)}</code></td><td data-label="${esc(t(headers[1]))}">${esc(t(n.type))}</td><td data-label="${esc(t(headers[2]))}">${esc(t(n.presence))}</td><td data-label="${esc(t(headers[3]))}">${esc(t(n.nullability))}</td><td data-label="${esc(t(headers[4]))}">${n.description?chromeHtml`<p>${esc(n.description)}</p>`:''}${Object.entries(n.constraints).map(([key,value])=>chromeHtml`<p><code>${esc(key)}</code>: ${esc(JSON.stringify(value))}</p>`).join('')}</td></tr>`).join('')}</tbody></table></div>`;
}
function inlineContractDeclaration(contract,owner){
 const d=contract.declaration,source=contract.source,id='contract-source-'+owner+'-'+contract.id;
 const remote=/^https?:\/\//i.test(source?.url||'');
 return chromeHtml`<section class="contract-declaration"><h4>${esc(t('Declared contract'))}: ${esc(contract.title)}</h4><p><a class="contract-declaration-link" href="${esc(remote?source.url:'#'+id)}" ${remote?'':chromeHtml`data-inline-declaration="${esc(id)}"`}>${esc(t('Contract declaration'))}</a></p><p class="schema-description">${esc(t('Presence is relative to the parent. Child requirements apply when their parent and the selected branch are present.'))}</p>${inlineDeclarationBody(d)}${d.frontier?.length?chromeHtml`<aside class="finding-strip"><p>${esc(t('Some schema rules are retained without expansion; inspect the exact schema below.'))} ${d.frontier.map(esc).join(' · ')}</p></aside>`:''}<details class="contract-raw" id="${esc(id)}"><summary>${esc(t('Exact schema and declaration provenance'))}</summary><p>${esc(t('Retained declaration; validator behavior and runtime conformance are separate.'))}</p><pre>${pretty({source,branchPointer:contract.branchPointer,declaration:d})}</pre></details></section>`;
}
function declaredContractLinks(anchor='',operationId=''){
 const declarations=D.interfaceContractDeclarations||{},seen=new Set(),contracts=[];
 for(const [operation,items] of Object.entries(declarations)){
  if(operationId&&operation!==operationId)continue;
  for(const contract of Object.values(items||{}))if(contract?.id&&!seen.has(contract.id)){seen.add(contract.id);contracts.push(contract);}
 }
 if(!contracts.length)return '';
 const process=D.subject?.startsWith('scenario:')?D.subject.slice(9):'';
 return chromeHtml`<nav class="declared-contract-links" aria-label="${esc(t('Declared contracts'))}">${contracts.map(contract=>{
  const query=process?'?'+new URLSearchParams({fromProcess:process,fromAnchor:anchor||activeId}).toString():'';
  return chromeHtml`<p><a href="../contracts/${esc(contract.id)}.html${esc(query)}">${esc(t('Declared contract'))}: ${esc(contract.title)}</a></p>`;
 }).join('')}</nav>`;
}
function serviceModelFieldTables(){
 const models=D.serviceModelLinks||[];if(!models.length)return '';
 return chromeHtml`<section class="service-field-models">${models.map(model=>chromeHtml`<article class="service-field-model"><h3><a href="${esc(model.href)}">${esc(model.title)}</a></h3>${model.fieldTableHtml?chromeHtml`${model.fieldTableHtml}<details class="contract-raw"><summary>${esc(t('Field descriptions and provenance'))}</summary><p>${esc(t('These explicit documentation mappings do not establish domain ownership.'))}</p><pre>${pretty({kind:model.kind,authority:model.authority,review:model.review,documentationLanguage:model.documentationLanguage,provenance:model.provenance})}</pre></details>`:chromeHtml`<p><a href="${esc(model.href)}">${esc(t('Field descriptions'))}</a></p>`}</article>`).join('')}</section>`;
}
function interfaceContracts(e){
 const cards=operation(e)?.interfaceContracts||[];
 if(!cards.length)return '';
 const boundaries=cards.filter(c=>c.kind!=='payload'),payloads=cards.filter(c=>c.kind==='payload');
 const chosenPayload=payloads.find(c=>c.id===payloadId)||payloads[0];
 const render=c=>{
  const declared=D.interfaceContractDeclarations?.[e.id]?.[c.id];
  return chromeHtml`<section class="contract-card"><h3><span class="contract-kind">${esc(c.kind.toUpperCase())}</span> ${esc(c.title)}</h3>${declared?inlineContractDeclaration(declared,e.id):chromeHtml`<p class="schema-description">${esc(t('No declaration is bound to this card. The description below is source-derived behavior, not a declared schema.'))}</p>`}<section class="contract-checks"><h4>${esc(t('Source-derived context and checks'))}</h4><p>${esc(t('The original descriptions are retained separately from the declaration. Their sources do not establish schema enforcement or runtime delivery.'))}</p><div class="table-wrap"><table><thead><tr><th>Contract element</th><th>Value / behavior</th></tr></thead><tbody>${c.rows.map(r=>chromeHtml`<tr><td><code>${esc(r.label)}</code></td><td class="contract-value">${esc(r.value)}</td></tr>`).join('')}</tbody></table></div></section>${c.boundaries.length?chromeHtml`<p class="flow-note">${c.boundaries.map(esc).join(' ')}</p>`:''}<details class="contract-raw contract-evidence"><summary>${esc(t('Detailed source bindings for the original descriptions'))}</summary>${c.rows.map(r=>chromeHtml`<p><b>${esc(r.label)}</b> ${button(r.sourceIds)}</p><pre>${pretty({id:r.id,sourceIds:r.sourceIds,dependencyIds:r.dependencyIds})}</pre>`).join('')}</details></section>`;
 };
 const select=(values,chosen,name,label)=>chromeHtml`<label class="contract-selector">${label} · ${values.length}<select data-contract-select="${name}" aria-label="${label}">${values.map(c=>chromeHtml`<option value="${esc(c.id)}" ${c.id===chosen.id?'selected':''}>${esc(c.title)}</option>`).join('')}</select></label>`;
 return chromeHtml`<p class="schema-description">Explicitly bound declarations and source-derived interface descriptions are shown separately. Neither proves deployed wire compatibility.</p>${boundaries.map(render).join('')}${chosenPayload?chromeHtml`<details class="payload-library"><summary>${esc(t('Payload descriptions and declarations'))} · ${payloads.length}</summary>${select(payloads,chosenPayload,'payload',t('Payload description'))}${render(chosenPayload)}</details>`:''}`;
}
function type(s){if(s===undefined||s===null)return 'Not declared';if(s===false)return 'No value allowed (false schema)';if(s===true)return 'Any value (true schema)';if(s.$ref)return esc(s.$ref);if(s.type==='array')return chromeHtml`array&lt;${type(s.items)}&gt;`;return esc(s.type||(s.allOf?'allOf':s.oneOf?'oneOf':s.anyOf?'anyOf':'Not declared'))+(s.format?chromeHtml` <span class="optional">${esc(s.format)}</span>`:'');}
function constraints(s){return Object.entries(s||{}).filter(([k])=>['const','format','enum','pattern','minLength','maxLength','minimum','maximum','exclusiveMinimum','exclusiveMaximum','minItems','maxItems','uniqueItems','nullable','default','readOnly','writeOnly','additionalProperties'].includes(k)).map(([k,v])=>chromeHtml`${esc(k)}: ${esc(k==='const'||typeof v==='object'?JSON.stringify(v):v)}`).join(chromeHtml`<br>`)||'—';}
function fields(s,depth=0){if(s===undefined||s===null)return chromeHtml`<span class="optional">No schema declared</span>`;if(depth>6)return chromeHtml`<span class="optional">See the complete schema below for deeper fields.</span>`;
 let html='';for(const kind of ['allOf','oneOf','anyOf'])if(s[kind])html+=chromeHtml`<p class="schema-description">${kind} composition (alternatives are preserved)</p>`+s[kind].map((p,i)=>chromeHtml`<details class="contract-raw"><summary>${kind} ${i+1}</summary>${fields(p,depth+1)}</details>`).join('');
 if(s.type==='array')return html+chromeHtml`<p class="schema-description">${type(s)} · ${constraints(s)}</p>${fields(s.items,depth+1)}`;
 const properties=Object.entries(s.properties||{});if(!properties.length)return html||chromeHtml`<p class="schema-description">${type(s)} · ${constraints(s)}</p>`;
 return html+chromeHtml`<div class="table-wrap"><table><thead><tr><th>Field</th><th>Type</th><th>Required</th><th>Constraints / description</th></tr></thead><tbody>${properties.map(([name,p])=>chromeHtml`<tr><td><code>${esc(name)}</code></td><td>${type(p)}${p.properties||p.items||p.allOf||p.oneOf||p.anyOf?chromeHtml`<details class="contract-raw"><summary>Nested fields</summary>${fields(p,depth+1)}</details>`:''}</td><td class="${s.required?.includes(name)?'required':'optional'}">${s.required?.includes(name)?t('required'):t('optional')}</td><td>${constraints(p)}${p.description?chromeHtml`<div class="schema-description">${esc(p.description)}</div>`:''}</td></tr>`).join('')}</tbody></table></div>`;
}
function contentBodies(content){return Object.entries(content||{}).map(([media,value])=>chromeHtml`<p class="schema-description"><code>${esc(media)}</code></p>${fields(value.schema)}${value.example!==undefined?chromeHtml`<pre class="example-block">${pretty(value.example)}</pre><p class="example-caption">Declared OpenAPI example; not an observed request.</p>`:''}${value.examples?chromeHtml`<details class="contract-raw"><summary>Declared examples</summary><pre>${pretty(value.examples)}</pre></details>`:''}`).join('')||chromeHtml`<p class="empty-note">No body declared.</p>`;}
function sourcePayloadViews(e){
 const owner=operation(e),views=(D.payloadViews||[]).filter(v=>v.operation===(owner?.id||e.id));
 if(!views.length)return '';
 const legend=chromeHtml`<details class="cpvlegend"><summary>${esc(t('How to read these data shapes'))}</summary><p>${esc(t('Source-derived representation shape. Type nullability and observed mappings do not establish wire-property presence, full schema or runtime conformance.'))}</p><p>${esc(t('Field definition unavailable in selected evidence. No empty or complete payload schema is asserted.'))}</p></details>`;
 const fieldLabel=field=>chromeHtml`<a href="../data-model/${esc(field.model)}.html#field-${esc(field.field)}"><strong>${esc(fieldBusinessName(field))}</strong></a>${field.readableName?.text?chromeHtml`<div class="cpv-field-path"><code>${esc(field.path)}</code></div>`:''}`;
 const fieldType=field=>field.relatedModel?chromeHtml`<a href="${esc('../data-model/'+field.relatedModel+'.html#fields-and-representations')}">${esc(field.typeLabel)} ↗</a>`:esc(field.typeLabel);
 const fieldMeaning=field=>chromeHtml`<div class="clew-markdown">${prose(field.meaning?.text||'')}</div>${field.mapping?.text&&field.mapping.text!==field.meaning?.text?chromeHtml`<div class="cpv-field-mapping"><b>${esc(t('Representation mapping'))}:</b><div class="clew-markdown">${prose(field.mapping.text)}</div></div>`:''}`;
 return legend+views.map(view=>{
  const coverage=view.coverage||{},coverageLabel=({'complete-definition':'Complete retained type definition','observed-fields':'Observed fields only','unknown':'Field definition unavailable'})[coverage.status]||'Field definition unavailable',languageGap=view.documentationLanguage&&D.language&&view.documentationLanguage!==D.language;
  const fields=view.fields||[];
  const overview=fields.length?chromeHtml`<div class="table-wrap"><table class="inline-contract-table cpv-field-overview"><thead><tr>${['Field or observed path','Meaning and mapping'].map(key=>chromeHtml`<th scope="col">${esc(t(key))}</th>`).join('')}</tr></thead><tbody>${fields.map(field=>chromeHtml`<tr><td data-label="${esc(t('Field or observed path'))}">${fieldLabel(field)}</td><td data-label="${esc(t('Meaning and mapping'))}">${fieldMeaning(field)}</td></tr>`).join('')}</tbody></table></div><details class="cpv-representation-details"><summary>${esc(t('Types, requiredness and null'))}</summary><div class="table-wrap"><table class="inline-contract-table"><thead><tr>${['Field or observed path','Type','Representation presence','Representation nullability'].map(key=>chromeHtml`<th scope="col">${esc(t(key))}</th>`).join('')}</tr></thead><tbody>${fields.map(field=>chromeHtml`<tr><td data-label="${esc(t('Field or observed path'))}"><code>${esc(field.path)}</code><div><code>${esc(field.representation)}</code></div></td><td data-label="${esc(t('Type'))}">${fieldType(field)}</td><td data-label="${esc(t('Representation presence'))}">${esc(field.requiredness)}</td><td data-label="${esc(t('Representation nullability'))}">${esc(field.nullability)}</td></tr>`).join('')}</tbody></table></div></details>`:'';
  return chromeHtml`<section class="contract-card cpv" id="cpv-${esc(view.operation)}-${esc(view.id)}"><h3>${esc(view.title)}</h3>${languageGap?chromeHtml`<p class="finding-strip">${esc(t('Translation required'))}: ${esc(view.documentationLanguage)}</p>`:''}<p><b>${esc(t(view.component))}</b> · ${esc(t(view.direction))}${view.status?chromeHtml` · ${esc(view.status)}`:''} · <code>${esc(view.format)}</code></p><p class="cpvcoverage"><b>${esc(t(coverageLabel))}</b> · ${esc(coverage.scope||'')}</p>${overview}<details class="contract-raw"><summary>${esc(t('Payload mapping and source evidence'))}</summary><div class="clew-markdown">${prose(coverage.claim?.text||'')}</div>${fields.map(field=>chromeHtml`<p><code>${esc(field.representation)}</code> · ${esc(field.mapping?.text)} · <a href="../data-model/${esc(field.model)}.html#field-${esc(field.field)}">${esc(t('Model details and sources'))} ↗</a></p>`).join('')}<pre>${pretty({authority:view.authority,runtime:view.runtime,coverage:coverage.claim})}</pre></details></section>`;
 }).join('');
}
function contract(e){const authored=sourcePayloadViews(e)+interfaceContracts(e),matches=c=>D.subject.startsWith('scenario:')||c.normalized.entrypoint===e.id||c.id===e.id;
 const contracts=D.contracts.filter(matches),retained=(D.operationContracts?.[e.id]||[]).filter(matches);
 if(contracts.length){const older=retained.filter(c=>!contracts.some(current=>current.id===c.id&&JSON.stringify(current.normalized)===JSON.stringify(c.normalized)));return authored+contractRows(contracts,e.id,true)+(older.length?chromeHtml`<details class="contract-raw"><summary>Contract retained with authored context</summary>${contractRows(older,e.id)}</details>`:'');}
 if(retained.length)return authored+contractRows(retained,e.id);
 return authored||chromeHtml`<p class="empty-note">Interface contract is not documented for this operation. Neither an unambiguous OpenAPI operation nor a source-derived contract was supplied.</p>${claimButton(e,t('Entrypoint source'))}`;
}
function contractRows(contracts,owner='',currentSnapshot=false){
 return contracts.map(c=>{const v=c.normalized,o=v.operation;return chromeHtml`<div class="contract-intro"><div><h3>${esc(v.method)} ${esc(v.path)}</h3><p>${currentSnapshot?'Declared OpenAPI contract from the selected source snapshot.':'Saved declared OpenAPI contract.'} ${esc(v.sourceMapping)}. Runtime enforcement is unverified.</p></div>${claimButton(c,t('Original contract'),currentSnapshot,owner)}</div>${v.boundaries.length?chromeHtml`<p class="finding-strip">${v.boundaries.map(esc).join(' · ')}</p>`:''}<section class="contract-section"><h3>Request parameters</h3>${v.parameters.length?chromeHtml`<div class="table-wrap"><table><thead><tr><th>Parameter</th><th>Location / type</th><th>Required</th><th>Constraints</th></tr></thead><tbody>${v.parameters.map(p=>chromeHtml`<tr><td><code>${esc(p.name)}</code><br>${esc(p.description)}</td><td>${esc(p.in)} / ${type(p.schema)}</td><td>${p.required?t('required'):t('optional')}</td><td>${constraints(p.schema)}</td></tr>`).join('')}</tbody></table></div>`:chromeHtml`<p class="empty-note">No parameters declared.</p>`}</section><section class="contract-section"><h3>Request body</h3><p class="schema-description">${o.requestBody?.required?t('Required body'):t('Optional or absent body')}</p>${contentBodies(o.requestBody?.content)}</section><section class="contract-section"><h3>Responses</h3>${Object.entries(o.responses||{}).map(([status,r])=>chromeHtml`<details class="contract-raw" ${status.startsWith('2')?'open':''}><summary><b class="response-status">${esc(status)}</b> · ${esc(r.description)}</summary>${contentBodies(r.content)}${r.headers?chromeHtml`<h4>Headers</h4><pre>${pretty(r.headers)}</pre>`:''}${claimButton(c,t('Response contract'),currentSnapshot,owner,['paths',v.path,v.method.toLowerCase(),'responses',status])}</details>`).join('')}</section><section class="contract-section"><h3>Declared access and servers</h3><p class="schema-description">Static declarations do not prove that access checks or server bindings are active.</p><pre class="example-block">${pretty({security:v.security,securitySchemes:v.securitySchemes,servers:v.servers})}</pre></section><details class="contract-raw"><summary>Complete operation contract</summary><pre>${pretty(o)}</pre></details>`;}).join('');
}
function findings(o){return o?.findings.length?chromeHtml`<div class="findings-list">${o.findings.map(f=>chromeHtml`<article class="finding-card"><div class="eyebrow">SOURCE-BASED OBSERVATION</div><p>${esc(f.text)}</p><div class="finding-actions">${claimButton(f,t('Compare evidence'))}</div></article>`).join('')}</div><p class="finding-note">Static findings are separate from observed runtime failures.</p>`:chromeHtml`<p class="empty-note">No findings recorded within the documented scope.</p>`;}
function evidenceContexts(){
 if(!D.selectedSnapshot)return '';
 return chromeHtml`<section class="evidence-contexts"><h3>Evidence contexts</h3><p>Selected source snapshot: <code>${esc(D.selectedSnapshot)}</code></p><details><summary>Selected source revisions, coverage and boundaries</summary><pre>${pretty(D.selectedEvidence||{})}</pre></details><details><summary>Retained explanation source context</summary><pre>${pretty({publication:D.frozenPublication,revisions:D.revisions,coverage:D.coverage,extractor:D.extractor})}</pre></details><p>The selected source facts and retained explanations have separate contexts. A selected snapshot does not reverify the retained explanations or runtime behavior.</p></section>`;
}
function freshness(state=D.sectionState){
 const el=$('freshness-status');if(!el)return;
 if(!state){el.innerHTML=chromeHtml`<p class="freshness-banner">Legacy publication: meaning review has not been assessed.</p><details class="freshness-details"><summary>Source and review details</summary>${evidenceContexts()}</details>`;return;}
 const revisions=values=>Object.entries(values||{}).map(([id,sha])=>chromeHtml`${esc(id)}: ${sha?esc(sha.slice(0,12)):t('unavailable')}`).join(' · ');
 const retainedServices=Object.entries(D.sourceAuthorities||{}).filter(([id,authority])=>authority==='RETAINED_SOURCE_NOT_REVERIFIED'&&Object.hasOwn(D.revisions||{},id)).map(([id])=>id);
 const retainedNotice=retainedServices.length?chromeHtml`<p>Saved evidence reused for ${retainedServices.map(esc).join(', ')}. These sources were not rechecked during the selected-service capture.</p>`:'';
 const pinned=state.freshness==='UNVERIFIED'&&state.reasons?.length>0&&state.reasons.every(reason=>reason.reason==='PINNED_SNAPSHOT_NOT_REVERIFIED');
 const message=pinned?t('Current checkout freshness was not rechecked for this publication.'):state.freshness==='CURRENT'?t('Recorded source dependencies are current.'):state.freshness==='STALE'?t('Source changed. This retained explanation needs review.'):t('Source status could not be established. This explanation is retained with a gap.');
 const metadata=chromeHtml`${evidenceContexts()}<p>Source freshness: <b>${esc(statusLabel(state.freshness))}</b> · Meaning review: ${esc(meaningReviewLabel(state.verification))}</p>${retainedNotice}<p>Content: ${Object.keys(state.mixedRevisions||{}).length?t('Mixed source versions; inspect each operation.'):revisions(state.contentRevisions)}<br>Target: ${revisions(state.targetRevisions)}</p><p>${esc(t('Model review approves an interpretation; evidence links do not prove every statement in the prose.'))}</p>${state.reasons?.length?chromeHtml`<details><summary>What needs attention</summary><pre>${pretty(state.reasons)}</pre></details>`:''}`;
 const compact=true;
 if(compact){el.innerHTML=chromeHtml`<details class="freshness-banner freshness-details freshness-${pinned?'snapshot':esc(state.freshness.toLowerCase())}"><summary>${esc(statusLabel(state.freshness))} · ${esc(t('Source and review details'))}</summary><p><strong>${esc(pinned?t('Saved source snapshot'):statusLabel(state.freshness))}</strong> · ${message}</p>${metadata}</details>`;return;}
 el.innerHTML=chromeHtml`<div class="freshness-banner freshness-${pinned?'snapshot':esc(state.freshness.toLowerCase())}"><p class="freshness-summary"><strong>${esc(pinned?t('Saved source snapshot'):statusLabel(state.freshness))}</strong><span>Meaning review: ${esc(meaningReviewLabel(state.verification))}</span></p><p>${message}</p><details class="freshness-details"><summary>Source and review details</summary>${metadata}</details>${Object.keys(D.updateFailures||{}).length?chromeHtml`<details><summary>Recent update gaps</summary><pre>${pretty(D.updateFailures)}</pre></details>`:''}</div>`;
}
function mappedContractEntry(id){
 const matches=D.contracts.filter(c=>c.id===id&&c.normalized.sourceMapping==='SOURCE_ROUTE_MATCH_ONLY'&&typeof c.normalized.entrypoint==='string');
 if(matches.length!==1)return null;
 const target=matches[0].normalized.entrypoint,visible=entries.filter(e=>e.id===target);
 if(visible.length===1)return visible[0];
 if(visible.length)return null;
 const retained=sourceCatalogue.filter(e=>e.id===target);
 return retained.length===1?retained[0]:null;
}
// Compare only explicit HTTP method/route facts; preserve both evidence owners.
function ingressPath(path,trigger){
 if(typeof path!=='string'||!path.startsWith('/')||/[?*+():\[\]]/.test(path))return null;
 if(trigger&&(trigger.apiVersions?.length||trigger.versionedPaths||/UNRESOLVED|RUNTIME|UNKNOWN|UNSUPPORTED/.test(trigger.pathResolution||'')))return null;
 const parts=path.split('/');
 if(parts.some(part=>/[{}]/.test(part)&&!/^\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(part)))return null;
 return parts.map(part=>/^\{[A-Za-z_][A-Za-z0-9_]*\}$/.test(part)?'{}':part).join('/');
}
function ingressRows(inventory,contracts){
 const groups=new Map(),other=[];let unresolved=0;
 const add=(kind,item,method,path,trigger)=>{
  const route=ingressPath(path,trigger),verb=typeof method==='string'&&/^[A-Za-z]+$/.test(method)?method.toUpperCase():null;
  const key=route!==null&&verb?verb+' '+route:kind+':unknown:'+unresolved++;
  if(!groups.has(key))groups.set(key,{method:verb,path:path||'Unresolved route',sources:[],contracts:[],comparable:route!==null&&!!verb});
  const row=groups.get(key),items=kind==='source'?row.sources:row.contracts;
  if(!items.some(existing=>existing.id===item.id))items.push(item);
 };
 for(const boundary of inventory.publicBoundaries||[]){
  if(boundary.kind!=='HTTP_ENDPOINT'){other.push(boundary);continue;}
  const trigger=boundary.trigger||{},methods=trigger.methods?.length?trigger.methods:[null],paths=trigger.paths?.length?trigger.paths:[null];
  for(const method of methods)for(const path of paths)add('source',boundary,method,path,trigger);
 }
 for(const contract of contracts)add('contract',contract,contract.normalized.method,contract.normalized.path,null);
 return {other,rows:[...groups.values()].map(row=>{
  let status;
  if(!contracts.length)status='NOT_COMPARED';
  else if(!row.comparable)status='UNCERTAIN';
  else if(row.sources.length>1||row.contracts.length>1)status='AMBIGUOUS';
  else if(!row.sources.length)status='DECLARED_ONLY';
  else if(!row.contracts.length)status='SOURCE_ONLY';
  else if(row.contracts[0].normalized.entrypoint&&row.contracts[0].normalized.entrypoint!==row.sources[0].id)status='BINDING_CONFLICT';
  else status='ROUTE_AGREES';
  return {...row,status};
 })};
}
// These records are external observations admitted by native Pages, not a live
// conformance result. Keep status/media and external JSON-schema checks separate.
function ingressHttpCurrent(){
 const c=D.contractComparison;
 return c?.schema==='codeclew-http-comparison-view/1.0'&&c.bindingStatus==='MATCHES_PINNED_SELECTED_SNAPSHOT'&&c.snapshot===D.selectedSnapshot&&c.revision===D.selectedEvidence?.revisions?.[c.service]&&D.subject==='service:'+c.service;
}
function ingressHttpFindingExample(finding,entry){
 const cases=entry.cases.filter(c=>finding.caseIds.includes(c.id));
 if(finding.kind==='UNDECLARED_RESPONSE_MEDIA'){
  const seen=new Set();
  return cases.map(c=>{const observed=String(c.response.contentType).split(';')[0].trim(),expected=(c.declaredMedia||[]).join(', ')||t('No media specified'),key=c.response.status+' '+observed+' '+expected;if(seen.has(key))return '';seen.add(key);return chromeHtml`<p>Observed: <code>${esc(c.response.status)} ${esc(observed)}</code>. Declared for this status: <code>${esc(expected)}</code>.</p>`;}).join('');
 }
 if(finding.kind==='BROADER_REQUEST_ACCEPTANCE')return cases.slice(0,2).map(c=>{const body=String(c.request.body??''),error=c.requestErrors?.[0]?.message;return chromeHtml`<p>${esc(t(body.length>256?'Recorded request excerpt':'Recorded request'))}: <code>${esc(body.slice(0,256))}</code> → <code>${esc(c.response.status)} ${esc(String(c.response.contentType).split(';')[0])}</code>.</p>${error?chromeHtml`<p>External schema check: ${esc(String(error).slice(0,256))}</p>`:''}`;}).join('');
 return '';
}
function ingressHttpEntry(row){
 if(!D.contractComparison)return '';
 if(!ingressHttpCurrent())return chromeHtml`<p class="ingress-http ingress-needs-review">Retained HTTP comparison does not match this selected snapshot. Its findings are not applied.</p>`;
 const matches=D.contractComparison.entries.filter(e=>row.status==='ROUTE_AGREES'&&row.sources.length===1&&row.contracts.length===1&&e.entrypoint===row.sources[0].id&&e.contractId===row.contracts[0].id&&e.contractDigest===row.contracts[0].digest&&e.method===row.method&&e.path===row.contracts[0].normalized.path);
 if(matches.length!==1)return chromeHtml`<p class="ingress-http empty-note">No applicable recorded HTTP checks for this entry.</p>`;
 const entry=matches[0],findings={
  BROADER_REQUEST_ACCEPTANCE:['Broader request acceptance','A recorded request outside the declared schema received a successful response. This shows broader acceptance; it does not show failure for a contract-valid request.'],
  UNDECLARED_RESPONSE_MEDIA:['Undeclared response media type','Recorded responses use a media type absent from the selected status declaration. Status-code and body-schema results are separate below.'],
  UNDECLARED_RESPONSE_STATUS:['Undeclared response status','A recorded response status is absent from the selected contract.'],
  REPORTED_BODY_SCHEMA_DIFFERENCE:['Reported body-schema difference','The retained external validator reports a difference against its selected response schema.']
 },requestLabels={'within-schema':'Within schema','outside-schema':'Outside schema','undeclared-media':'Undeclared request media','no-body-declared':'No body declared','not-checked':'Not checked'},bodyLabels={'matches-selected-schema':'Matches selected schema','violates-selected-schema':'Differs from selected schema','not-specified':'No schema specified','not-checked':'Not checked'};
 return chromeHtml`<div class="ingress-http"><p class="ingress-http-coverage"><strong>${entry.cases.length?t('Recorded HTTP checks')+' · '+entry.cases.length:t('HTTP path not exercised')}</strong> ${esc(entry.coverage)}</p>${entry.findings.filter(f=>findings[f.kind]).map(f=>chromeHtml`<aside class="ingress-http-finding" role="status"><h5>${esc(t(findings[f.kind][0]))}</h5><p>${esc(t(findings[f.kind][1]))}</p>${ingressHttpFindingExample(f,entry)}<p class="small-label">Cases: ${f.caseIds.map(esc).join(', ')}</p></aside>`).join('')}<details class="ingress-http-details"><summary>Cases, evidence and limits</summary><p>${esc(D.contractComparison.scope)}</p><p>JSON-schema results are retained external assessments by ${esc(D.contractComparison.validator)}; Pages verifies packet integrity and selected source/contract bindings, and does not rerun that validator.</p>${entry.cases.length?chromeHtml`<div class="table-wrap"><table><thead><tr><th>${esc(t('Retained case'))}</th><th>Request schema</th><th>Status code</th><th>Response media</th><th>JSON body · external check</th></tr></thead><tbody>${entry.cases.map(c=>chromeHtml`<tr><td>${esc(c.id)}</td><td>${esc(t(requestLabels[c.requestAssessment]||'Unknown'))}</td><td>${esc(c.response.status)} · ${esc(t(c.statusDeclared?'Declared':'Undeclared').toLowerCase())}</td><td>${esc(c.response.contentType)} · ${esc(c.mediaAssessment)}</td><td>${esc(t(bodyLabels[c.bodyAssessment]||'Unknown'))}${c.comparedSchemaMedia?chromeHtml`<br>Schema: ${esc(c.comparedSchemaMedia)}`:''}</td></tr>`).join('')}</tbody></table></div><details><summary>Raw recorded requests and responses</summary>${entry.cases.map(c=>chromeHtml`<h5>${esc(c.id)}</h5><pre>${pretty(c)}</pre>`).join('')}</details>`:chromeHtml`<p>No HTTP observations were retained for this entry. Source inspection is not runtime verification.</p>`}<p>Contract binding: <code>${esc(entry.contractId)}</code><br><code>${esc(entry.contractDigest)}</code></p>${button(Object.keys(D.contractComparison.sources),t('Bound selected source'),true)}<p><a href="../http-contract-comparison.json">Retained comparison packet and provenance</a></p></details></div>`;
}
function ingressHttpProvenance(){
 if(!ingressHttpCurrent())return '';
 const c=D.contractComparison;
 return chromeHtml`<details class="ingress-http-provenance"><summary>HTTP comparison scope and retained provenance</summary><p>${esc(c.scope)}</p><ul>${c.limitations.map(v=>chromeHtml`<li>${esc(v)}</li>`).join('')}</ul><p>Authority: ${esc(c.authority)}. Binding: ${esc(c.bindingStatus)}. This is a pinned comparison; the current checkout and deployed service were not rechecked.</p><p>Source revision: <code>${esc(c.revision)}</code><br>Selected snapshot: <code>${esc(c.snapshot)}</code><br>Packet digest: <code>${esc(c.packetDigest)}</code></p><details><summary>Exact selected source bindings</summary><pre>${pretty({sources:c.sources,artifactBindings:c.artifactBindings})}</pre></details><p>Probe files below are retained external evidence. Files outside the selected source records have integrity checks, not compiler or snapshot-source authority.</p>${Object.entries(c.artifacts).map(([id,a])=>chromeHtml`<details><summary>${esc(id)} · ${esc(a.digest)}</summary>${a.sourceExport?.mode==='remote'?chromeHtml`<p>Source attachment omitted from this export.</p>${(c.artifactBindings||[]).filter(b=>b.artifact===id).map(b=>{const s=c.sources?.[b.sourceId];return s?.url?chromeHtml`<p><a href="${esc(s.url)}" target="_blank" rel="noopener noreferrer">${esc(s.file)}:${s.startLine}–${s.endLine} ↗</a></p>`:'';}).join('')}`:chromeHtml`<pre>${esc(a.text)}</pre>`}</details>`).join('')}</details>`;
}
function ingressComparison(inventory,contracts){
 const comparison=ingressRows(inventory,contracts),warnings=comparison.rows.filter(row=>!['ROUTE_AGREES','NOT_COMPARED'].includes(row.status));
 const status={ROUTE_AGREES:'Route agrees',NOT_COMPARED:'Not compared · no OpenAPI operations available',UNCERTAIN:'Route comparison uncertain',AMBIGUOUS:'Ambiguous route match',DECLARED_ONLY:'No source match in selected evidence',SOURCE_ONLY:'No matching OpenAPI declaration',BINDING_CONFLICT:'Route agrees, source binding conflicts'};
 return chromeHtml`<section class="ingress-comparison"><h3>HTTP entry points · ${comparison.rows.length}</h3><p class="ingress-scope">${esc(t(ingressHttpCurrent()?'Method and route comparison with separately retained local HTTP checks for bound entries. Exhaustive compatibility and deployed runtime enforcement have not been verified.':'Method and route comparison only. Status codes, body/schema compatibility and runtime enforcement have not been verified.'))} Discovery is bounded; an unmatched declaration does not prove that an implementation is absent.</p>${warnings.length?chromeHtml`<p class="finding-strip ingress-warning" role="status">${esc(t('Route comparisons needing review:'))} ${warnings.length}. ${esc(t('Inspect the separate source and OpenAPI evidence below.'))}</p>`:''}<div class="ingress-routes" role="list">${comparison.rows.map(row=>chromeHtml`<article class="ingress-route" role="listitem"><div><h4><code>${esc(row.method||'HTTP')} ${esc(row.path)}</code></h4><p class="ingress-status ${!['ROUTE_AGREES','NOT_COMPARED'].includes(row.status)?'ingress-needs-review':''}">${esc(t(status[row.status]))}</p></div><div class="ingress-links"><div><span>Source handler</span>${row.sources.length?row.sources.map(v=>chromeHtml`<button class="source-link" data-entry="${esc(v.id)}">Behavior</button> ${claimButton(v,t('Source'),true)}${entryProcessLinks(v.id)}`).join(' · '):chromeHtml`<p>Not matched in this evidence</p>`}</div><div><span>OpenAPI declaration</span>${row.contracts.length?row.contracts.map(c=>chromeHtml`<button class="source-link" data-entry="${esc(c.id)}" data-tab="contract">Contract</button> ${claimButton(c,t('Declaration'),true)}`).join(' · '):chromeHtml`<p>${esc(t(contracts.length?'Not matched':'Not available'))}</p>`}</div></div>${ingressHttpEntry(row)}</article>`).join('')}</div>${ingressHttpProvenance()}${comparison.other.length?chromeHtml`<h3>Other public boundaries · ${comparison.other.length}</h3>${comparison.other.map(v=>chromeHtml`<p><button class="source-link" data-entry="${esc(v.id)}">${esc(label(v))}</button> ${claimButton(v,t('Source'),true)}</p>`).join('')}`:''}</section>`;
}

function kafkaInteractions(){return (D.interactions||[]).filter(i=>i.declaration?.transport?.kind==='kafka'&&typeof i.declaration.transport.topic==='string');}
function topicReadingSteps(guide){return (guide?.sections||[]).filter(s=>s.number).map(s=>({id:s.id,blocks:s.blocks.map(b=>({text:b.text,sourceIds:b.evidence.flatMap(e=>e.fragment?.sourceIds||[]),dependencyIds:b.evidence.flatMap(e=>e.fragment?.dependencyIds||[])}))}));}
let topicContextText,topicContextValue;
function topicContext(){const text=$('reader-context')?.textContent||'{}';if(text!==topicContextText){topicContextText=text;try{topicContextValue=JSON.parse(text);}catch{topicContextValue={};}}return topicContextValue||{};}
function topicNavigation(){const rows=topicContext().processLinks;return Array.isArray(rows)?rows:[];}
function topicServiceName(service){const title=topicContext().serviceLabels?.[service];return typeof title==='string'?title:service;}
function currentTopicView(){return topicNavigation().find(v=>v.id===D.process?.definition?.id)||{root:D.process?.definition?.root,interactions:D.process?.definition?.interactions||[],steps:topicReadingSteps(D.questionReader)};}
function sameTopicEndpoint(a,b){return a?.service===b?.service&&a?.selector&&b?.selector&&['language','name','owner'].every(k=>a.selector[k]===b.selector[k])&&JSON.stringify(a.selector.parameterTypes||[])===JSON.stringify(b.selector.parameterTypes||[]);}
function topicReadingStep(view,interaction,side){
 const topic=interaction.declaration.transport.topic,sourceIds=new Set(interaction[side]?.sourceIds||[]),dependencies=new Set([...(interaction[side]?.candidates||[]),...(side==='from'?interaction.callSite?.candidates||[]:[])]);
 return (view.steps||[]).find(step=>step.blocks.some(b=>String(b.text).includes(topic)&&((b.dependencyIds||[]).some(id=>dependencies.has(id))||(b.sourceIds||[]).some(id=>sourceIds.has(id)))));
}
function topicViewDirection(view,interaction){
 if(!view?.root||!view.interactions?.includes(interaction.id))return null;
 for(const [side,direction] of [['from','outgoing'],['to','incoming']]){
  const endpoint=interaction.declaration[side];
  if(sameTopicEndpoint(view.root,endpoint)||view.root.service===endpoint?.service&&topicReadingStep(view,interaction,side))return direction;
 }
 return null;
}
function topicViewLinks(interaction,side,question){return topicNavigation().filter(v=>v.id!==D.process?.definition?.id&&topicViewDirection(v,interaction)===(side==='from'?'outgoing':'incoming')).map(v=>{let href=v.href;if(question&&D.subject.startsWith('scenario:')){const [path,hash]=href.split('#'),params=new URLSearchParams({fromProcess:D.subject.slice(9),fromQuestion:question});href=path+'?'+params+'#'+hash;}return chromeHtml`<a href="${esc(href)}">${esc(v.title)}</a>`;}).join(' · ');}
function topicServiceHref(service,direction,topic,question){
 const params=new URLSearchParams();if(topic){params.set('topic',topic);params.set('direction',direction);}
 if(question&&D.subject.startsWith('scenario:')){params.set('fromProcess',D.subject.slice(9));params.set('fromQuestion',question);}
 const query=params.toString();return `../services/${encodeURIComponent(service)}.html${query?'?'+query:''}#section-${direction==='incoming'?'ingress':'egress'}`;
}
function topicBoundaryCard(interactions,{direction='related',question=null,anchor=false}={}){
 const first=interactions[0],topic=first.declaration.transport.topic,from=first.declaration.from;
 const endpoint=(i,side)=>{const value=i.declaration[side],flow=side==='from'?'outgoing':'incoming',external=side==='to'&&i.declaration.external===true,registered=external&&i.declaration.externalService===value.service&&D.externalServices?.[value.service],name=registered?.title||topicServiceName(value.service),href=external?(registered?'../external-services/'+encodeURIComponent(value.service)+'.html':''):topicServiceHref(value.service,flow,topic,question);return chromeHtml`${href?chromeHtml`<a href="${esc(href)}">${esc(name)}</a>`:chromeHtml`<span>${esc(name)}</span>`}<span class="small-label">${esc(external?t('Source not captured'):value.selector?.name||t('Declared endpoint'))}</span>`;};
 const receiverRows=[...new Map(interactions.map(i=>[`${i.declaration.to.service}:${i.declaration.to.selector?.name}`,i])).values()];
 const sources=[...new Set(interactions.flatMap(i=>[...(i.from?.sourceIds||[]),...(i.to?.sourceIds||[]),...(i.callSite?.sourceIds||[])]))].filter(id=>D.sources?.[id]);
 const literalSources=Object.entries(D.sources||{}).filter(([,s])=>typeof s.text==='string'&&s.text.includes(topic)).map(([id])=>id).slice(0,12);
 const assessments=(D.notes||[]).filter(n=>n.assessment&&interactions.some(i=>n.association.service===i.declaration.to.service));
 const publisherLinks=topicViewLinks(first,'from',question),consumerLinks=interactions.map(i=>topicViewLinks(i,'to',question)).filter(Boolean);
 const heading=t(direction==='incoming'?'Incoming Kafka topic':direction==='outgoing'?'Outgoing Kafka topic':'Related asynchronous continuation');
 return chromeHtml`<article class="topic-boundary"${anchor?chromeHtml` id="topic-${esc(direction)}-${esc(topic)}" tabindex="-1"`:''}><h3>${esc(heading)} · <code>${esc(topic)}</code></h3><div class="topic-path" aria-label="${esc(t('Declared asynchronous handoff'))}"><div>${endpoint(first,'from')}</div><span aria-hidden="true">→</span><div><code>${esc(topic)}</code><span class="small-label">${esc(t('Kafka topic'))} · ${esc(t('asynchronous'))}</span></div><span aria-hidden="true">→</span><div>${receiverRows.map(i=>chromeHtml`<div>${endpoint(i,'to')}</div>`).join('')}</div></div><p><b>${esc(t('Purpose'))}:</b> ${esc(first.declaration.transport.purpose||t('Not established'))} · <b>${esc(t('Message key'))}:</b> <code>${esc(first.declaration.transport.messageKey||t('Not established'))}</code></p>${publisherLinks?chromeHtml`<p>${esc(t('Saved publisher view'))}: ${publisherLinks}</p>`:''}${consumerLinks.length?chromeHtml`<p>${esc(t('Saved consumer views'))}: ${[...new Set(consumerLinks)].join(' · ')}</p>`:''}${assessments.length?chromeHtml`<p>${esc(t('Related service assessments'))}: ${assessments.map(n=>chromeHtml`<a href="../services/${esc(n.association.service)}.html#${esc(n.id)}">${esc(n.assessment.title||n.association.title)}</a>`).join(' · ')}</p>`:''}<details class="technical-evidence"><summary>${esc(t('Retained source and declaration context'))}</summary><p>${esc(t(first.origin==='agent-proposal'?'Proposed handoff':'Declared handoff'))}; ${esc(t(first.runtime==='UNKNOWN'?'runtime delivery has not been verified.':'see the separately retained runtime scope.'))}</p><p class="topic-type-note">${esc(t("Topic names and the payload's event type are separate values."))}</p>${button([...new Set([...sources,...literalSources])],t('Selected source'),true)}<p>${esc(t('These references retain their own revisions. A source match does not upgrade the declared topic mapping or demonstrate delivery.'))}</p><pre>${pretty(interactions.map(i=>({interaction:i.id,origin:i.origin,from:i.declaration.from,to:i.declaration.to,topic:i.topic,runtime:i.runtime,rationale:i.declaration.declaration?.rationale,boundaries:i.boundaries})))}</pre></details></article>`;
}
function groupedTopicCards(rows,options){const groups=new Map();for(const i of rows){const key=i.declaration.transport.topic+'|'+i.declaration.from.service+'|'+i.declaration.from.selector?.name;if(!groups.has(key))groups.set(key,[]);groups.get(key).push(i);}return [...groups.values()].map(g=>topicBoundaryCard(g,options)).join('');}
function serviceTopicBoundaries(direction){
 if(!D.subject.startsWith('service:'))return '';
 const service=D.subject.slice(8),side=direction==='incoming'?'to':'from',rows=kafkaInteractions().filter(i=>i.declaration[side]?.service===service);
 return rows.length?chromeHtml`<section class="service-topic-boundaries"><h2>${esc(t(direction==='incoming'?'Incoming Kafka topics':'Outgoing Kafka topics'))}</h2>${groupedTopicCards(rows,{direction,anchor:true})}</section>`:'';
}
function processTopicSection(section){
 const view=currentTopicView(),rows=kafkaInteractions().filter(i=>{const direction=topicViewDirection(view,i);if(!direction)return false;const matched=topicReadingStep(view,i,direction==='outgoing'?'from':'to');return matched?.id===section||!matched&&view.steps?.[0]?.id===section;});
 return ['incoming','outgoing'].map(direction=>groupedTopicCards(rows.filter(i=>topicViewDirection(view,i)===direction),{direction,question:section})).join('');
}
function relatedTopicInteractions(){const view=currentTopicView();return D.process?.definition?kafkaInteractions().filter(i=>view.interactions?.includes(i.id)&&!topicViewDirection(view,i)):[];}
function relatedTopicContinuations(){
 const rows=relatedTopicInteractions();
 return rows.length?chromeHtml`<section id="question-async-continuations" tabindex="-1" class="question-section related-topic-continuations"><h2>Related asynchronous continuations</h2><p>These declared handoffs belong to the linked continuation views. They are not additional publishing steps in the local account above.</p>${groupedTopicCards(rows,{question:'async-continuations'})}</section>`:'';
}
function runtimeObservation(note){
 const text=note.original?.text;if(typeof text!=='string'||text.length>65536)return null;
 let value;try{value=JSON.parse(text);}catch{return null;}
 if(!value||value.schema!=='clew-weather-runtime-observation/1.0'||value.origin!=='LOCAL_TEST_OBSERVATION'||value.compilerEvidence!==false||typeof value.id!=='string'||!value.id.length||value.id.length>160)return null;
 const revision=value.sourceCommit||value.repositoryRevision;if(!/^[a-f0-9]{40}$/.test(revision||''))return null;
 if(!['PASS','PASSED','FAIL','FAILED'].includes(value.status))return null;
 if(Array.isArray(value.cases)&&value.cases.length<=100&&value.cases.every(row=>typeof row?.case==='string'&&row.case.length<=160&&typeof row.pass==='boolean')&&typeof value.startedAt==='string'&&value.startedAt.length<=80){
  return {kind:'replay',status:value.status,revision,recordedAt:value.startedAt,cases:value.cases.map(row=>({name:row.case,passed:row.pass})),passed:value.cases.filter(row=>row.pass).length,count:value.cases.length};
 }
 if(typeof value.nodeVersion==='string'&&value.nodeVersion.length<=32&&/^v\d+\.\d+\.\d+$/.test(value.nodeVersion)&&Number.isSafeInteger(value.exerciseResult?.assertions)&&value.exerciseResult.assertions>=0&&value.exerciseResult.assertions<=10000&&value.exerciseResult.runtime===value.nodeVersion){
  return {kind:'node',status:value.status,revision,nodeVersion:value.nodeVersion,count:value.exerciseResult.assertions};
 }
 return null;
}
function runtimeObservationCard(observation){
 if(!observation)return '';
 const detail=observation.kind==='replay'?chromeHtml`<p>${observation.passed} of ${observation.count} replay cases reported passing · Recorded at ${esc(observation.recordedAt)}</p><details><summary>Recorded replay cases</summary><div class="table-wrap"><table><thead><tr><th>Case</th><th>Recorded result</th></tr></thead><tbody>${observation.cases.map(row=>chromeHtml`<tr><td>${esc(row.name)}</td><td>${row.passed?'Passed':'Failed'}</td></tr>`).join('')}</tbody></table></div></details>`:chromeHtml`<p>${observation.count} assertions recorded · Node version: <code>${esc(observation.nodeVersion)}</code></p>`;
 return chromeHtml`<section class="note-observation"><h2>Historical runtime observation</h2><p><b>Reported result: ${esc(observation.status)}</b> · Repository revision <code>${esc(observation.revision)}</code></p>${detail}<p>Imported, unverified record. This page does not report a current execution, compiler verification, or a general runtime guarantee.</p></section>`;
}
function showEntry(id,nextTab='sequence'){const mapped=mappedContractEntry(id);if(mapped){id=mapped.id;nextTab='contract';}freshness();current=entries.find(e=>e.id===id)||sourceCatalogue.find(e=>e.id===id)||entries.find(e=>e.kind==='SECTION')||entries.find(e=>operation(e))||entries[0];tab=nextTab==='sequence'&&current?.kind==='DECLARED_OPENAPI'?'contract':nextTab;closeSource(false);$('catalogue-view').hidden=true;$('coverage-view').hidden=true;$('scenario-content').hidden=false;
 if(!current){$('scenario-content').innerHTML=chromeHtml`<h2>No entrypoints available</h2><p class="empty-note">Inspect coverage and the service evidence gaps.</p>`;nav();return;}
 const e=current,o=operation(e);
 if(D.questionReader&&e.id==='process-overview'){showQuestionReader();return;}
 freshness(D.operationStates?.[e.visualOwner||e.id]||D.sectionState);
 if(translationGaps[e.id]&&e.id!=='section-overview'){ $('scenario-content').innerHTML=`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(sectionTitles[e.id]||e.trigger.paths?.join(' · ')||e.symbol||e.id)}</h2>${translationNotice(e.id)}`;nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);return; }
 if(e.kind==='SERVICE_FRAGMENT'){
  const fragment=D.serviceFragments.find(f=>f.id===e.id);
  freshness(D.operationStates?.[e.id]||{freshness:'UNVERIFIED',verification:'UNASSESSED'});
  $('scenario-content').innerHTML=serviceFragmentCard(fragment);nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=fragment.title+' · '+D.title;return;
 }
 if(e.kind==='VISUAL'){
  const visual=o.visuals.find(v=>v.id===e.visualId);
  freshness(D.operationStates?.[o.id]||{freshness:'UNVERIFIED',verification:'UNASSESSED'});
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)} · Processes and diagrams</div>${visualCard(o,visual)}`;
  nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=visual.title+' · '+D.title;return;
 }
 if(e.kind==='NOTE'){
  const n=D.notes.find(n=>n.id===e.id),a=n.assessment?.assessment,state=D.operationStates?.[e.id];
  if(a)$('freshness-status').insertAdjacentHTML('afterbegin',chromeHtml`<p class="small-label">Status of the generated assessment · original note retains human/imported authority</p>`);
  else $('freshness-status').innerHTML=chromeHtml`<p class="freshness-banner">Assessment: UNASSESSED. Original note retains human/imported authority.</p>`;
  const match=a&&a.noteDigest===n.original.digest&&a.associationDigest===n.associationDigest&&n.targetChanged!==true;
  const observation=runtimeObservation(n),title=n.assessment?.title||n.association.title;
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)} · Imported notes and assessments</div><h2>${esc(title)}</h2><p class="small-label">Original record: ${esc(n.association.title)} · ${esc(n.association.classification)}${observation?.kind==='node'?'':chromeHtml` · Recorded label: ${esc(n.association.period)}`}</p>${n.targetChanged?chromeHtml`<p class="finding-strip">This retained note snapshot or its association has changed since publication. The retained assessment is stale.</p>`:''}${runtimeObservationCard(observation)}<article class="clew-markdown">${prose((n.original.text||'').replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/,''))}</article><p><a href="../pages/${esc(n.association.id)}.html">Open protected human page</a> · <a download href="../pages/${esc(n.association.id)}.original.md">Download exact original Markdown</a></p><section class="note-assessment"><h2>Separate agent assessment</h2>${a?chromeHtml`<p><b>Assessment conclusion: ${esc(a.outcome)}</b> · ${observation?.kind==='node'?'Node version assessed':'Observation label assessed'}: ${esc(a.period)}</p><p class="small-label">Input binding: ${match?t('matches displayed capture'):statusLabel('STALE')} · Meaning review: ${esc(meaningReviewLabel(state?.verification||'UNASSESSED'))} · Freshness: ${esc(statusLabel(state?.freshness||'UNVERIFIED'))}</p><p class="assessment-prose">${esc(n.assessment.summary.text)}</p>${claimButton(n.assessment.summary,t('Assessment evidence'))}${a.proposedCorrection?chromeHtml`<h3>Proposed correction · original unchanged</h3><p>${esc(a.proposedCorrection.text)}</p>${claimButton(a.proposedCorrection,t('Correction evidence'))}`:''}`:chromeHtml`<p>No assessment has been accepted for this note on this page.</p>${n.assessmentSubject!==D.subject?chromeHtml`<p><a href="../services/${esc(n.association.service)}.html#${esc(n.id)}">Open the assessment service</a></p>`:''}`}<p>Classification and original text remain human/imported declarations. Evidence tracking covers captured inputs; it cannot establish every implicit claim in arbitrary prose.</p></section><details class="note-record-details"><summary>Original record · ${esc(n.association.title)}</summary><p>${esc(n.snapshotLabel)}</p><p>Captured text digest: <code>${esc(n.original.digest)}</code></p><pre class="note-original" tabindex="0">${esc(n.original.text||n.original.reason||n.original.status)}</pre></details><details class="note-record-details"><summary>Original metadata and associations</summary><pre class="note-original" tabindex="0">${pretty(n.association)}</pre></details>${n.missingTargets?.length?chromeHtml`<p class="empty-note">Unresolved targets: ${n.missingTargets.map(esc).join(', ')}</p>`:''}`;
  nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=title+' · '+D.title;return;
 }
 if(e.kind==='VIEW'){
  const v=D.view,g=o?.dataflow,d=v.definition.view,h=d.human||{};
  const annotationRows=Object.entries(h.annotations||{});
  $('scenario-content').innerHTML=flowReader(o);
  nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=t('Entity data flow')+' · '+D.title;return;
 }
 if(e.kind==='PROCESS_CATALOG'){
  const catalog=D.processCandidates,summary=catalog?.summary||{},internal=showInternalProcesses?(catalog?.internal||[]):[],saved=D.savedProcesses||[];
  const cards=internal.map(candidate=>chromeHtml`<article class="gap-card"><h3>${esc(candidate.owner||t('Callable'))} · ${esc(candidate.name||candidate.symbol)}</h3><p><b>${candidate.status==='NEEDS_EVIDENCE'?t('Needs evidence'):t('Awaiting explanation')}</b> · ${esc(candidate.scope||t('unscoped source'))}</p><p>${candidate.lexicalCallSiteCount?candidate.lexicalCallSiteCount+t(' lexical call sites (targets unresolved)'):candidate.localCallTargetCount+t(' distinct local call targets')} · ${candidate.controlEventCount} control-flow events</p><p>No business interpretation has been accepted for this candidate.</p>${claimButton(candidate,t('Retained method source'),true)}${candidate.gaps.length?chromeHtml`<details><summary>Missing or limited evidence</summary><ul>${candidate.gaps.map(gap=>chromeHtml`<li>${esc(gap)}</li>`).join('')}</ul></details>`:''}<details><summary>Exact root and discovery reasons</summary><pre>${pretty({declaration:candidate.id,scope:candidate.scope,symbol:candidate.symbol,reasons:candidate.reasons})}</pre></details></article>`).join('');
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(t(showInternalProcesses?'Internal processes':'Saved processes'))}</h2>${visualGallery()}<p>Saved process explanations and structural candidates from retained code evidence.</p><h3>Saved processes · ${saved.length}</h3>${saved.length?saved.map(process=>chromeHtml`<article class="gap-card"><h3><a href="${esc(process.href)}">${esc(process.title)} ↗</a></h3><p>${process.status==='AUTHORED'?t('Explanation available'):t('Awaiting explanation')}</p>${process.state?chromeHtml`<p>Source freshness: ${esc(statusLabel(process.state.freshness))} · Meaning review: ${esc(meaningReviewLabel(process.state.verification))}</p>`:''}<p>Declared trigger: ${esc(process.trigger)}</p></article>`).join(''):chromeHtml`<p class="empty-note">No internal process has been selected for maintained documentation yet.</p>`}${showInternalProcesses?chromeHtml`<h3>Internal candidates · ${summary.internalCandidateCount??t('unknown')}</h3><p>These methods contain local calls and control flow. They are candidates for explanation, not an exhaustive list of business processes.</p>${cards||chromeHtml`<p class="empty-note">No internal candidates could be nominated from this evidence. This does not establish that the service has no internal processes. Select an exact method when discovery is incomplete.</p>`}${catalog?.omittedInternal?chromeHtml`<p>${catalog.omittedInternal} additional internal candidates are available in the retained catalogue.</p>`:''}<p>Separate trigger candidates: ${summary.triggerCandidateCount??t('unknown')}. Methods with missing or limited flow: ${summary.flowUnavailableCount??t('unknown')}.</p><details><summary>Browse the full retained catalogue</summary><p>This command reads saved evidence and does not invoke capture or a model.</p><pre>clew docs process candidates --root &lt;documentation-root&gt; --service ${esc(D.subject.slice(8))} --lane internal</pre><p>Use the returned cursor for subsequent pages; use --lane trigger to inspect trigger candidates separately. An explicit --snapshot selects a saved version.</p></details><details><summary>Discovery limits</summary><pre>${pretty(summary)}</pre></details>`:''}`;
  nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=t(showInternalProcesses?'Internal processes':'Saved processes')+' · '+D.title;return;
 }
 if(e.kind==='PROCESS'){
  const p=D.process,v=p.definition.process;
  const state=D.stateDiagram?chromeHtml`<div class="diagram-card"><div class="section-label"><b>${esc(t('State diagram'))}</b><span>${esc(t('Declared process-state schema'))} · ${esc(t('Static source bindings do not show runtime execution.'))}</span></div>${plantumlPreview(D.stateDiagram,D.stateDiagramSvg,'State diagram')}<div class="diagram-footer"><span>${esc(t('States and transitions come from the captured process-state schema.'))} scenarios/${esc(D.subject.split(':').pop())}-states.yaml</span><a download href="../diagrams/${esc(D.stateDiagram)}.puml">PlantUML ↓</a></div></div>`:'';
  const transitions=D.activityTransitions?chromeHtml`<div class="diagram-card"><div class="section-label"><b>Activity on transitions</b><span>Lifecycle operations and the state each one moves the process into</span></div>${plantumlPreview(D.activityTransitions,D.activityTransitionsSvg,'Activity on transitions')}<div class="diagram-footer"><span>${esc(t('One branch per declared lifecycle operation; this is not an observed runtime trace.'))} scenarios/${esc(D.subject.split(':').pop())}-states.yaml</span><a download href="../diagrams/${esc(D.activityTransitions)}.puml">PlantUML ↓</a></div></div>`:'';
  const authoredIds=new Set(D.operations.map(operation=>operation.id));
  const detailed=(D.lifecycleOperations||[]).filter(l=>!authoredIds.has(l.name)).map(l=>D.sourceExport?.mode==='remote'?chromeHtml`<section class="gap-card"><h4>${esc(l.name)}</h4><p>${esc(t('Source text is omitted from this export. Open the exact retained revision in the repository.'))}</p></section>`:chromeHtml`<div class="diagram-card"><div class="section-label"><b>${esc(l.name)}</b><span>${esc(t('Detailed process steps'))} · ${esc(t(l.origin==='source'?'Source syntax summary':'Retained structured FLOW'))}</span></div>${plantumlPreview(l.diagramStem,l.svgAvailable,l.name)}<pre class="tree">${esc(l.tree)}</pre><div class="diagram-footer"><span>${esc(t('Static source interpretation; runtime execution was not observed.'))}</span><a download href="../diagrams/${esc(l.diagramStem)}.puml">PlantUML ↓</a></div></div>`).join('');
  const overviewOwner=D.operations.find(operation=>operation.id==='process-overview');
  const authoredOverviewVisuals=overviewOwner?.visuals?.length?chromeHtml`<section class="visual-gallery process-overview-visuals"><h3>${esc(t('Authored process overview visuals'))}</h3>${overviewOwner.visuals.map(visual=>visualCard(overviewOwner,visual)).join('')}</section>`:'';
  const processOutline=processOutlineCard(D.processOutline);
  const thread=D.processThread?.status==='SOURCE_BOUND_DECLARATION'?D.processThread:null;
  const summary=o?.summary.text||p.definition.summary||D.gaps[e.id]||t('No overview has been accepted.');
  const services=thread?[...new Set((thread.methodOrder||Object.keys(thread.methods)).map(id=>thread.methods[id].source.service))]:v.participants;
  const limits=[...new Set([...D.boundaries,...(o?.boundaries||[]),...(p.targetGaps||[])])];
  const captured=chromeHtml`<details class="process-captured"><summary>${esc(t('Declared scope, goals and outcomes'))}</summary><p>${esc(v.scope)}</p>${v.objects.length?chromeHtml`<p><b>Domain objects:</b> ${v.objects.map(esc).join(', ')}</p>`:''}<ul>${v.outcomes.map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul><p>${esc(t('Declared goals and outcomes are not proven program results.'))}</p><p>${esc(p.authority)}</p></details>`;
  const children=p.linkedSubviews.length?chromeHtml`<section><h3>Linked child views · captured versions</h3>${p.linkedSubviews.map(c=>chromeHtml`<article class="gap-card"><h4>${c.gap==='LINKED_PROCESS_MISSING'?esc(c.child):chromeHtml`<a href="${esc(c.href)}#process-overview">${esc(c.child)} ↗</a>`}</h4><p>${esc(c.gap||meaningReviewLabel(c.accepted.verification))}</p>${c.accepted?chromeHtml`<p>${esc(c.accepted.summary.text)}</p><p>${[...c.accepted.limitations,...c.accepted.boundaries].map(esc).join(' · ')}</p><details><summary>Accepted child version and influence</summary><pre class="note-original">${pretty(c.accepted)}</pre></details>`:chromeHtml`<p>No current accepted explanation is available for composition.</p>`}</article>`).join('')}</section>`:'';
  const supporting=(thread?'':state)+transitions+authoredOverviewVisuals+processOutline+detailed+children;
  $('scenario-content').innerHTML=chromeHtml`<div class="process-view"><header class="process-introduction"><div class="eyebrow">${esc(t(thread?'Declared source-bound process':'SAVED PROCESS'))}</div><h2>${esc(D.process.definition.title)}</h2><p class="summary">${esc(summary)}</p><p class="process-summary-authority">${esc(t(o?'Accepted overview; its source freshness and meaning review are separate.':'Summary from the process declaration; no reviewed overview is implied.'))}</p>${claimButton(o?.summary,t('Overview evidence'))}${declaredContractLinks('process-overview')}<dl class="process-facts"><div><dt>${esc(t('Trigger'))}</dt><dd>${esc(v.trigger)}</dd></div><div><dt>${esc(t('Participants'))}</dt><dd>${services.map(esc).join(' · ')}</dd></div>${thread?chromeHtml`<div><dt>${esc(t('Selected source'))}</dt><dd>${thread.methodOrder.length} ${esc(t('complete methods'))} · ${esc(t('Runtime execution was not observed.'))}</dd></div>`:''}</dl>${thread?chromeHtml`<p class="thread-authority">${esc(t('Declared interpretation bound to exact retained source; call targets and runtime delivery are not established.'))}</p>`:''}${p.targetChanged?chromeHtml`<p class="finding-strip">This captured definition or a linked child has changed. Retained explanations need review.</p>`:''}${captured}</header>${thread?state+processThreadCard(thread):supporting}${processExtractionCard(D.processExtraction)}${thread&&supporting?chromeHtml`<details class="process-supporting"><summary>${esc(t('Additional diagrams and source outlines'))}</summary>${supporting}</details>`:''}${limits.length?chromeHtml`<details class="process-publication-limits"><summary>${esc(t('Publication evidence details'))} (${limits.length})</summary><p>${esc(t('These diagnostics describe retained publication evidence; they are not a list of bugs or additional process steps.'))}</p><ul>${limits.map(x=>chromeHtml`<li>${esc(x)}</li>`).join('')}</ul></details>`:''}${relatedNotes(D.subject)}</div>`;
  nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=t('Process overview')+' · '+D.title;return;
 }
 if(e.kind==='SECTION'){
  const section=D.sections.find(s=>s.id===e.id),inventory=serviceInventory(),entities=(D.entities||[]).filter(d=>d.normalized.entity.relations.some(r=>r.service===D.subject.slice(8)));
  let detail='';
  if(e.id==='section-entities')detail=domainCatalogue();
  if(e.id==='section-ingress')detail=ingressComparison(inventory,D.contracts);
  if(e.id==='section-overview')detail=serviceProfile()+(processCatalogVisible?chromeHtml`<section class="profile-processes"><h3>${esc(t(showInternalProcesses?'Internal processes and selected diagrams':'Saved processes'))}</h3>${visualRows().length?'':chromeHtml`<p class="profile-gap">No typed process diagram has been accepted in this publication.</p>`}<p>Browse maintained processes and structural candidates with their authoring status. A local fragment is not automatically an entry-to-outgoing-call scenario.</p><p><button class="source-link" data-entry="process-catalog">${esc(t(showInternalProcesses?'Explore internal processes →':'Open saved processes →'))}</button></p></section>`:'')+visualPreview()+chromeHtml`<details class="profile-discovery"><summary>Source inventory and coverage</summary><div class="coverage-grid"><div class="metric"><b>${(inventory.publicBoundaries||[]).length}</b><p>discovered public boundaries</p></div><div class="metric"><b>${inventory.internalCallableCount??(inventory.internalCallables||[]).length}</b><p>internal callable evidence records</p></div></div><p>Internal callable inventory does not establish public API exposure.</p></details>`;
  if(e.id==='section-ingress'||e.id==='section-overview')detail=serviceTopicBoundaries('incoming')+detail;
  if(e.id==='section-egress')detail=groupedEgress()+serviceTopicBoundaries('outgoing')+detail;
  detail+=sourcePayloadViews(e);

  if(['section-entities','section-egress'].includes(e.id)){
   $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(label(e))}</h2>${detail}${readerEvidenceLegend()}<details class="original-service-description"><summary>${esc(t('Original description and provenance'))}</summary><div class="clew-markdown">${prose(o?.summary.text||section.gap)}</div>${o?explanation(o):''}${o?.visuals?.length?chromeHtml`<section class="visual-gallery entity-section-visuals"><h3>${esc(t('Entity views'))}</h3>${o.visuals.map(v=>visualCard(o,v)).join('')}</section>`:''}${claimButton(o?.summary,t('Supporting source'))}${relatedNotes(D.subject+'/'+e.id)}<pre>${pretty(o?.boundaries||[])}</pre></details>`;window.clewMarkdown?.enhance(document);nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);return;
  }
  $('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb">${esc(D.title)}</div><h2>${esc(label(e))}</h2>${e.id==='section-entities'&&(D.serviceModelLinks||[]).some(m=>m.fieldTableHtml)?chromeHtml`<details class="original-service-description"><summary>${esc(t('Original service data description and provenance'))}</summary><div class="summary clew-markdown">${prose(o?.summary.text||section.gap)}</div>${claimButton(o?.summary,t('Supporting source'))}</details>`:e.id==='section-ingress'&&o?chromeHtml`<details class="ingress-original"><summary>Full accepted entry-point description</summary><p class="summary">${esc(o.summary.text)}</p>${claimButton(o.summary,t('Supporting source'))}</details>`:chromeHtml`<div class="summary clew-markdown">${translationGaps[e.id]?'':prose(o?.summary.text||section.gap)}</div>${translationNotice(e.id)}${claimButton(o?.summary,t('Supporting source'))}`}${relatedNotes(D.subject+'/'+e.id)}${relatedNotes(D.subject)}${o?explanation(o):''}${detail}<div class="gap-card"><h3>Evidence limits</h3><ul>${[...(inventory.gaps||[]).map(g=>({'PUBLIC_BOUNDARY_INVENTORY_IS_BOUNDED_BY_SELECTED_MODULES':t('Public boundary discovery covers the selected evidence modules.'),'DYNAMIC_REGISTRATION_AND_RUNTIME_ACTIVATION_UNVERIFIED':t('Dynamic registration and runtime activation have not been verified.'),'SOURCE_EVIDENCE_UNAVAILABLE':t('Source evidence is unavailable.')}[g]||g)),...(o?.boundaries||[])].map(v=>chromeHtml`<li>${esc(v)}</li>`).join('')}</ul></div>`;
  window.clewMarkdown?.enhance(document);nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=section.title+' · '+D.title;return;
 }
$('scenario-content').innerHTML=chromeHtml`<div class="breadcrumb"><span>${esc(D.title)}</span><div class="scope-summary">${readingScopeSummary()}</div></div><div class="scenario-heading"><div><div class="operation-label">${chip(method(e))}<code>${esc(e.trigger.paths?.join(' · ')||'')}</code></div><h2>${esc(label(e))}</h2></div>${button(o?.summary.sourceIds||e.sourceIds,t('Source'))}</div><p class="summary">${esc(o?.summary.text||D.gaps[e.id]||(e.kind==='DECLARED_OPENAPI'?t('Declared API operation; source behavior is not established.'):t(publicBoundaryIds&&!publicBoundaryIds.has(e.id)?'Source callable retained for use inside a process. No separate narrative is required.':'This entrypoint has no behavior narrative yet.')))}</p>${o?.findings.length?chromeHtml`<div class="finding-strip"><span class="finding-symbol">!</span><b>${esc(o.findings[0].text)}</b><button data-tab="findings">Inspect →</button></div>`:''}<div class="tabs" role="tablist" aria-label="Operation view">${[['sequence',t('What happens')],['contract',t('Contract')],['findings',t('Findings')]].map(([id,label])=>chromeHtml`<button role="tab" id="tab-${id}" data-tab="${id}" aria-controls="tab-content" aria-selected="${tab===id}">${label}${id==='findings'?chromeHtml`<span class="count">${o?.findings.length||0}</span>`:''}</button>`).join('')}</div><div id="tab-content" role="tabpanel" aria-labelledby="tab-${tab}">${tab==='sequence'?declaredContractLinks(e.id,e.id)+sequence(e,o):tab==='contract'?contract(e):findings(o)}</div>${relatedNotes(D.subject)}`;
 nav();history.replaceState(null,'',(location.search||'')+'#'+e.id);document.title=label(e)+' · '+D.title;
}
function catalogue(){freshness();closeSource(false);$('scenario-content').hidden=true;$('coverage-view').hidden=true;$('catalogue-view').hidden=false;$('catalogue-view').innerHTML=chromeHtml`<div class="catalogue-heading"><div class="eyebrow">SOURCE AND DECLARED SCOPE</div><h2>Source and API inventory · ${inventoryEntries().length}</h2><div class="scope-summary">${readingScopeSummary()}</div><p>Internal callable inventory does not establish public API exposure.</p></div><div class="table-wrap"><table><thead><tr><th>Trigger</th><th>Operation</th><th>Coverage</th></tr></thead><tbody>${inventoryEntries().map(e=>chromeHtml`<tr class="catalogue-row"><td>${chip(method(e))}</td><td><button data-entry="${esc(e.id)}">${esc(label(e))}</button><details class="inventory-identity"><summary>Exact identity</summary><code>${esc(e.symbol)}</code></details></td><td>${e.kind==='DECLARED_OPENAPI'?t('Declared contract; source mapping unresolved'):translationGaps[e.id]?t('Translation pending'):operation(e)?t('Documented'):esc(D.gaps[e.id]||t('Awaiting authoring'))}</td></tr>`).join('')}</tbody></table></div>`;}
function coverage(){freshness();closeSource(false);$('scenario-content').hidden=true;$('catalogue-view').hidden=true;$('coverage-view').hidden=false;$('coverage-view').innerHTML=evidenceContexts()+chromeHtml`<div class="coverage-heading"><div class="eyebrow">COVERAGE AND AUTHORITY</div><h2>What the sources support</h2><p>Freshness is scoped to the recorded dependencies. Runtime routing and wire compatibility remain unverified.</p></div><div class="coverage-grid"><div class="metric"><b>${inventoryEntries().length}</b><p>source and API inventory records</p></div><div class="metric"><b>${documentedCount()}</b><p>documented operations</p></div><div class="metric"><b>${Object.keys(D.sources).length}</b><p>retained source fragments</p></div></div>${D.boundaries.map(b=>chromeHtml`<div class="gap-card"><p>${esc(b)}</p></div>`).join('')}<div class="gap-card"><h3>Declared service interactions</h3>${D.interactions.length?D.interactions.map(i=>chromeHtml`<p>${esc(i.id)} · origin: ${esc(i.origin)} · runtime: ${esc(i.runtime)}</p><details class="technical-evidence"><summary>Checks and boundaries</summary><pre>${pretty(i)}</pre></details>`).join(''):chromeHtml`<p>No service interactions selected for this page.</p>`}</div><details class="technical-evidence"><summary>Revision vector and evidence scope</summary><pre>${pretty({revisions:D.revisions,coverage:D.coverage,extractor:D.extractor,renderer:D.renderer})}</pre></details>`;}
const sourceBackground=new Map();
function sourceViewport(){
 const panel=$('source-panel'),modal=!panel.hidden&&navigationMedia?.matches;
 if(modal){
  panel.setAttribute('role','dialog');panel.setAttribute('aria-modal','true');panel.setAttribute('aria-labelledby','source-title');
  for(const element of document.body.children){if(element===panel)continue;if(!sourceBackground.has(element))sourceBackground.set(element,element.inert);element.inert=true;}
  if(!panel.contains(document.activeElement))$('close-source').focus();
 }else{
  panel.removeAttribute('role');panel.removeAttribute('aria-modal');panel.removeAttribute('aria-labelledby');
  for(const [element,inert] of sourceBackground)element.inert=inert;sourceBackground.clear();
 }
}
function sourceKeyboard(e){
 const panel=$('source-panel');if(panel.hidden)return false;
 if(e.key==='Escape'){e.preventDefault();if($('source-panel').hidden&&(flowSelection||flowFieldSelection)){selectFlow('reset');}else closeSource();return true;}
 if(e.key!=='Tab'||!navigationMedia?.matches)return false;
 const controls=[...panel.querySelectorAll('a[href],button,input,select,textarea,summary,[tabindex]')].filter(element=>element.tabIndex>=0&&!element.disabled&&!element.closest('[hidden],[inert]')&&element.getClientRects().length&&window.getComputedStyle(element).visibility!=='hidden');
 const first=controls[0],last=controls.at(-1),active=document.activeElement;
 if(!controls.includes(active)||(e.shiftKey&&active===first)||(!e.shiftKey&&active===last)){e.preventDefault();((e.shiftKey?last:first)||$('close-source')).focus();return true;}
 return false;
}
function openSource(title){lastFocus=document.activeElement;$('source-title').textContent=title;$('source-panel').hidden=false;document.body.classList.add('source-open');sourceViewport();$('close-source').focus();}
// Locate an exact response declaration inside retained JSON without changing
// its bytes or source identity. Unsupported/ambiguous syntax stays full-source.
function jsonSourceLocation(text,wanted){
 if(!wanted?.length||text.length>2*1024*1024)return null;
 try{
  JSON.parse(text);let at=0,found=null;
  const ws=()=>{while(/\s/.test(text[at]||'')&&at<text.length)at++;};
  const string=()=>{const start=at++;while(at<text.length){const c=text[at++];if(c==='\\'){at++;continue;}if(c==='"')return JSON.parse(text.slice(start,at));}throw Error('string');};
  const node=(path,depth)=>{
   if(depth>64)throw Error('depth');ws();const start=at;
   if(text[at]==='{'){
    at++;ws();const keys=new Set();while(text[at]!=='}'){
     const key=string();if(keys.has(key))throw Error('duplicate');keys.add(key);ws();at++;node([...path,key],depth+1);ws();if(text[at]!==',')break;at++;ws();
    }at++;
   }else if(text[at]==='['){at++;ws();let index=0;while(text[at]!==']'){node([...path,String(index++)],depth+1);ws();if(text[at]!==',')break;at++;ws();}at++;}
   else if(text[at]==='"')string();
   else while(at<text.length&&!/[\s,}\]]/.test(text[at]))at++;
   if(path.length===wanted.length&&path.every((key,index)=>key===wanted[index]))found={start,end:at};
  };
  node([],0);return found;
 }catch{return null;}
}
function source(ids,title=t('Source evidence'),currentSnapshot=false,owner='',fragment='',path=[],claim=''){sourcePath=path;sourceScope=currentSnapshot?'current':'';sourceOperation=owner;sourceFragment=fragment;const records=sourceRecords(),valid=ids.filter(id=>records[id]),missing=ids.filter(id=>!records[id]);if(!valid.length||missing.length){sourceId='';closeSource(false);$('source-select').innerHTML='';$('source-meta').innerHTML='';$('source-code').innerHTML='';$('copy-status').textContent='';if(!fragment&&!valid.length)return;const message=fragment?t('Selected fragment source context is unavailable.'):t('Selected source context is incomplete.');$('source-foot').textContent=message+(missing.length?' '+t('Missing sources: ')+missing.join(', '):'');openSource(title);return;}if(D.sourceExport?.mode==='remote'){const links=valid.map(id=>records[id]);if(links.length===1&&/^https:\/\//.test(links[0].url||'')){closeSource(false);window.open(links[0].url,'_blank','noopener,noreferrer');return;}sourceId='';$('source-meta').innerHTML='<ul>'+links.map(s=>chromeHtml`<li><a href="${esc(s.url)}" target="_blank" rel="noopener noreferrer">${esc(s.service)} · ${esc(s.file)}:${s.startLine}–${s.endLine}</a> · ${esc(s.revision)}</li>`).join('')+'</ul>';$('source-code').hidden=true;if($('source-select').parentElement)$('source-select').parentElement.hidden=true;$('source-foot').textContent=t('Source text is omitted from this export. Open the exact retained revision in the repository.');openSource(title);return;}const declared=!currentSnapshot&&!fragment?D.preferredSourceIds?.[owner||current?.visualOwner||current?.id]?.[claim]:null;const preferred=Array.isArray(declared)?declared:[];sourceId=preferred.find(id=>valid.includes(id))||valid[0];$('source-select').innerHTML=valid.map(id=>chromeHtml`<option value="${esc(id)}">${esc(records[id].service+' / '+records[id].file+':'+records[id].startLine)}</option>`).join('');$('source-select').value=sourceId;sourceContent();openSource(title);}
function sourceContent(){
 $('source-code').hidden=false;if($('source-select').parentElement)$('source-select').parentElement.hidden=false;
 const s=sourceRecords()[sourceId],location=jsonSourceLocation(s.text,sourcePath);
 const span=location?{start:s.startLine+s.text.slice(0,location.start).split('\n').length-1,end:s.startLine+s.text.slice(0,location.end).split('\n').length-1}:null;
 const web=/^https?:\/\//.test(s.url||'')?s.url:null;
 const url=span&&web?web.split('#')[0]+'#L'+span.start+(web.includes('/-/blob/')?'-':'-L')+span.end:web;
 $('source-meta').innerHTML=chromeHtml`${url?chromeHtml`<a href="${esc(url)}" target="_blank" rel="noopener noreferrer">${esc(s.file)}:${span?span.start:s.startLine}–${span?span.end:s.endLine} ↗</a>`:esc(s.file)}<br>${esc(s.service)} · ${esc(s.revision.slice(0,12))} · ${esc(s.authority)}${sourcePath.length?chromeHtml`<p>${span?'Selected response within the full retained contract.':'Response location unavailable; full retained source is shown.'}</p>`:''}`;
 $('source-code').innerHTML=s.text.split('\n').map((line,i)=>chromeHtml`<span data-source-line="${s.startLine+i}" class="code-line ${!span||(s.startLine+i>=span.start&&s.startLine+i<=span.end)?'highlight':''}"><span class="line-number">${s.startLine+i}</span><code>${esc(line)||' '}</code></span>`).join('');
 $('source-code').scrollTop=0;
 $('source-foot').innerHTML=chromeHtml`<p>The highlighted fragment is exact retained source. Linked code does not verify the narrative meaning.</p><details><summary>Source binding</summary>Full retained span: ${s.startLine}–${s.endLine}<br>Text: ${esc(s.textDigest)}<br>Evidence: ${esc(s.evidenceDigest)}</details>`;
 $('copy-status').textContent='';
 if(span){const reveal=()=>document.querySelector('[data-source-line="'+span.start+'"]')?.scrollIntoView({block:'start'});if(typeof requestAnimationFrame==='function')requestAnimationFrame(reveal);else reveal();}
}
function closeSource(focus=true){const wasOpen=!$('source-panel').hidden;$('source-panel').hidden=true;sourceViewport();document.body.classList.remove('source-open');document.querySelectorAll('.selected[data-event]').forEach(e=>e.classList.remove('selected'));if(wasOpen&&focus&&lastFocus?.isConnected)lastFocus.focus();}
document.addEventListener('click',e=>{const declaration=e.target.closest('[data-inline-declaration]');if(declaration?.dataset?.inlineDeclaration&&!e.ctrlKey&&!e.metaKey&&!e.shiftKey&&!e.altKey){e.preventDefault();const target=$(declaration.dataset.inlineDeclaration);if(target){target.open=true;target.scrollIntoView({block:'start'});target.querySelector('summary')?.focus();}return;}const boundaryLink=e.target.closest('[data-boundary-declaration]');if(boundaryLink?.dataset?.boundaryDeclaration){e.preventDefault();const details=$(boundaryLink.dataset.boundaryDeclaration);if(details){details.open=true;details.scrollIntoView({block:'center'});details.querySelector?.('summary')?.focus?.();}return;}const questionLink=e.target.closest('a[href]');const questionHref=questionLink?.getAttribute?.('href')||'';if(questionHref.startsWith('#')&&questionSection(questionHref.slice(1))){e.preventDefault();if(location.hash!==questionHref)history.pushState(null,'',questionHref);revealQuestion(questionHref.slice(1));return;}const threadLink=e.target.closest('a[href]');const threadHref=threadLink?.getAttribute?.('href')||'';const anchor=threadHref.slice(1).split('~').at(-1);if(threadHref.startsWith('#')&&isThreadAnchor(anchor)&&(D.processThread||current?.kind==='SERVICE_FRAGMENT')){e.preventDefault();if(location.hash!==threadHref)history.pushState(null,'',threadHref);revealThreadAnchor(anchor,threadLink.dataset.threadEnd);return;}const el=e.target.closest('[data-flow-select],[data-flow-field],[data-flow-reset],[data-dfcshift],[data-flow-input-page],[data-flow-expand],[data-flow-direction],[data-visual-target],[data-entry],[data-tab],[data-sources],[data-event],[data-overview-node],[data-overview-edge],[data-view-node],[data-view-edge]');if(!el)return;e.preventDefault();if(el.dataset.dfcshift!==undefined){selectFlow('connection-shift',el.dataset.dfcshift);return;}if(el.dataset.flowInputPage!==undefined){selectFlow('input-page',el.dataset.flowInputPage);return;}if(el.dataset.flowSelect){selectFlow('node',el.dataset.flowSelect);return;}if(el.dataset.flowField){selectFlow('field',el.dataset.flowField);return;}if(el.dataset.flowReset!==undefined){selectFlow('reset');return;}if(el.dataset.flowExpand!==undefined){selectFlow('expand');return;}if(el.dataset.flowDirection){selectFlow('direction',el.dataset.flowDirection);return;}if(el.dataset.entry||el.dataset.visualTarget)readerSelection();if(el.dataset.visualTarget){navigateEntry(el.dataset.visualTarget);if(el.dataset.visualNodeTarget){const node=$(el.dataset.visualNodeTarget);node?.scrollIntoView({block:'center'});node?.focus();}else window.scrollTo(0,0);}else if(el.dataset.entry){navigateEntry(el.dataset.entry,el.dataset.tab||'sequence');window.scrollTo(0,0);}else if(el.dataset.tab)showEntry(current.id,el.dataset.tab);else if(el.dataset.sources)source(el.dataset.sources.split(' '),t('Source evidence'),el.hasAttribute('data-current-sources'),el.dataset.sourceOperation||'',el.dataset.sourceFragment||'',el.dataset.sourcePath?JSON.parse(el.dataset.sourcePath):[],el.dataset.sourceClaim||'');else if(el.dataset.viewNode||el.dataset.viewEdge){const g=operation(current)?.dataflow,item=el.dataset.viewNode?g?.nodes.find(n=>n.id===el.dataset.viewNode):g?.edges.find(e=>e.id===el.dataset.viewEdge);if(item){if(el.dataset.viewNode)selectFlow('node',item.id);else {flowSelection=item.to;selectFlow('direction','both');}}}else if(el.dataset.overviewNode||el.dataset.overviewEdge){const o=operation(current),d=o.overviewDiagram,item=el.dataset.overviewNode?d.nodes.find(n=>n.id===el.dataset.overviewNode):d.edges.find(n=>n.id===el.dataset.overviewEdge);if(item){source([...new Set(o.events.filter(e=>item.eventIds.includes(e.id)).flatMap(e=>e.sourceIds))],item.text);}}else if(el.dataset.event){const owner=operation(current),event=(el.dataset.sequenceEvent&&owner?sequenceEvents(owner):owner?.events)?.find(e=>e.id===el.dataset.event);if(event){source(event.sourceIds,event.text,false,'','',[],event.id);el.classList.add('selected');}}});
document.addEventListener('change',e=>{const kind=e.target.dataset.contractSelect;if(!kind)return;if(kind==='boundary')contractId=e.target.value;else payloadId=e.target.value;showEntry(current.id,'contract');if(kind==='payload')document.querySelector('.payload-library').open=true;document.querySelector(chromeHtml`[data-contract-select="${kind}"]`)?.focus();});
document.addEventListener('keydown',e=>{if(e.key==='Escape'&&$('source-panel').hidden&&(flowSelection||flowFieldSelection)){e.preventDefault();selectFlow('reset');return;}if(sourceKeyboard(e))return;if(e.target.matches('[role=tab]')&&['ArrowLeft','ArrowRight'].includes(e.key)){e.preventDefault();const tabs=['sequence','contract','findings'];const next=tabs[(tabs.indexOf(tab)+(e.key==='ArrowRight'?1:2))%3];showEntry(current.id,next);$('tab-'+next).focus();}});
$('search').addEventListener('input',nav);$('source-select').onchange=e=>{sourceId=e.target.value;sourceContent();};$('close-source').onclick=()=>closeSource();$('catalogue-button').onclick=()=>{catalogue();readerSelection();};$('coverage-button').onclick=()=>{coverage();readerSelection();};
$('copy-source').onclick=async()=>{try{await navigator.clipboard.writeText(sourceRecords()[sourceId].text);$('copy-status').textContent=t('Source copied');}catch{$('copy-status').textContent=t('Select the source text and copy it manually.');}};
$('project-kind').textContent=D.subject.startsWith('service:')?t('SERVICE'):t('INTERACTION SCENARIO');$('project-title').textContent=D.title;$('project-subtitle').textContent=D.subtitle;$('catalogue-count').textContent=inventoryEntries().length;$('revision-links').innerHTML=Object.entries(D.selectedEvidence?.revisions||D.revisions).map(([id,revision])=>chromeHtml`<div class="small-label">${esc(id)} · ${esc(revision.slice(0,12))}</div>`).join('');
const navigationMedia=window.matchMedia?.('(max-width:760px)');
function navigateEntry(id,nextTab='sequence'){if(location.hash!=='#'+id)history.pushState(null,'',(location.search||'')+'#'+id);showEntry(id,nextTab);}
function readerSelection(){if(navigationMedia?.matches)$('reader-navigation').open=false;$('content').focus();}
function navigationViewport(){const navigation=$('reader-navigation');if(navigation&&navigationMedia)navigation.open=!navigationMedia.matches;sourceViewport();}
navigationViewport();navigationMedia?.addEventListener?.('change',navigationViewport);
window.addEventListener('hashchange',routeReaderHash);routeReaderHash();scheduleSequenceReflow();
