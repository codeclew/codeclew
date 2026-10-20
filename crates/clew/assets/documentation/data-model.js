// Authored field-use reader. No graph, scope or transfer is inferred at runtime.
(function(){
 'use strict';
 const readers=new Map();
 function reader(language){
  if(readers.has(language))return readers.get(language);
  const {t,chromeHtml}=clewLocale(language,CLEW_RU_MESSAGES);
 const labels={'validate':t('Validated'),'identify-or-route':t('Identity / routing'),'calculate-or-decide':t('Decision / calculation'),'display':t('Displayed'),'persist':t('Stored'),'forward':t('Forwarded'),'whole-object-serialization-or-comparison':t('Whole-object use')};
 const states={'observed-static-use':t('Seen in retained source'),'no-use-found-in-bounded-complete-scope':t('No direct use found in this scope'),'not-investigated':t('Not investigated'),'unresolved-dynamic-use':t('Dynamic use unresolved'),'stale-evidence':t('Stale evidence — use unknown')};
 const esc=value=>String(value??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
 const fields=data=>(data.models||[]).flatMap(model=>(model.fields||[]).map(field=>({model,field,key:model.id+'/'+field.id,label:model.title+'.'+field.path})));
 // The inventory is bounded to the models/contracts supplied by the renderer.
 function overviewRows(data,contract=''){
  const rows=fields(data).filter(r=>!contract||(r.field.representations||[]).some(rep=>rep.contract===contract));
  for(const c of (data.contracts||[]).filter(c=>!contract||c.id===contract))for(const leaf of c.leaves||[]){
   if(rows.some(r=>leafFor(r,c).some(l=>l.field===leaf.field)))continue;
   rows.push({key:'contract:'+c.id+'/'+leaf.field,label:c.title+' · '+leaf.field,contract:c,leaf,field:{usage:[],representations:[],meaning:{text:t('Contract leaf without a canonical field mapping in this documented scope.')}}});
  }
  return rows;
 }
 function inventoryLabel(row){return row.leaf?t('Unmapped · unknown'):row.field.selectedProcessField===false?t('Outside trace · unknown'):'';}
 function fullInventoryLabel(row){return row.leaf?t('Unmapped contract leaf · not investigated'):row.field.selectedProcessField===false?t('Outside investigated process scope · use unknown'):'';}
 function overviewHtml(data,rows,stages){
  const known=fields(data),traced=data.request?.fields?.length,selectedModels=new Set((data.request?.fields||[]).map(f=>f.model));
  const related=data.request?(data.models||[]).filter(m=>!selectedModels.has(m.id)).map(m=>chromeHtml`${esc(m.title)} · Model fields: ${(m.fields||[]).length}`).join(' · '):'';
  const full=chromeHtml`<p class="dm-inventory">Displayed rows: ${rows.length} · Stages: ${stages.length}. Known model fields / container paths: ${known.length} (${data.request?t('selected and explicitly related models'):t('this model scope')})${related?chromeHtml` (related inventory: ${related})`:''}${traced===undefined?'':chromeHtml` · Selected process fields: ${traced}`}. ${(data.contracts||[]).map(c=>chromeHtml`${esc(c.title)} · Contract leaves: ${(c.leaves||[]).length}`).join(' · ')}. Mapped leaves share model rows; unmapped leaves have separate rows. Required-leaf counts below are separate.</p><p class="dm-legend">Named usage keeps its stated category. Forwarding / whole-object use does not prove a named business read. “No direct use found in this scope” requires an explicit bounded finding. Missing evidence, outside scope, dynamic unresolved and stale remain unknown, never unused.</p><p class="dm-scroll-hint">Scroll within the table to compare stages. Select a cell for details; the overview filters stay unchanged.</p>`;
  const guide=data.request?.readingGuide;
  const points=guide?.points||[];
  return chromeHtml`<h3>Where each field is used</h3><p class="dm-purpose">${esc(guide?.purpose||t('Compare where fields are checked, used, stored or passed on. Choose View evidence to inspect the source behind one conclusion.'))}</p><p class="dm-limit">Static source interpretation, not a runtime trace or proof of delivery.</p>${points.length?chromeHtml`<ul class="dm-reading-notes">${points.map(point=>chromeHtml`<li>${esc(point.claim.text)}${(point.claim.evidence||[]).some(e=>e.state&&e.state.freshness!=='CURRENT')?chromeHtml(['<span class="dm-limit"> Retained evidence is not current; inspect its scope.</span>']):''}</li>`).join('')}</ul>`:''}<p class="dm-overview-key"><b>Field operation</b>: a named check, read or write. <b>Object handling</b>: no separate field read is established. <b>Unknown</b>: no conclusion here. A no-direct-use finding applies only to its stated scope. No field is proven safe to remove by this view.</p><details class="dm-scope-details"><summary>Scope, counts and evidence key</summary>${full}<p>${esc(data.scope||t('Only documented model/consumer scope is investigated.'))}</p>${points.map(point=>chromeHtml`<p>Reading note: ${esc(point.claim.text)}<br>Fields: ${point.fields.map(f=>esc(f.model+'/'+f.field)).join(', ')} · stages: ${point.stages.map(esc).join(', ')}</p>${evidence(point.claim)}`).join('')}</details>`;
 }
 function sourceHtml(source,start=source.startLine,end=source.endLine){
  const text=(source.text||'').split('\n').slice(Math.max(0,start-source.startLine),end-source.startLine+1).join('\n');
  const base=(source.url||'').split('#')[0],link=/^https?:\/\//.test(base)?chromeHtml`<a href="${esc(base)}#L${start}${base.includes('/-/blob/')?'-':'-L'}${end}">${esc(source.file)}:${start}–${end}</a>`:esc(source.file)+':'+start+'–'+end;
  return chromeHtml`<p>${link} · revision <code>${esc(source.revision)}</code></p>${source.text?chromeHtml`<pre tabindex="0">${esc(text)}</pre>`:''}<details><summary>Exact source binding</summary><pre>${esc(JSON.stringify({sourceId:source.id,textDigest:source.textDigest,authority:source.authority,startLine:start,endLine:end},null,2))}</pre></details>`;
 }
 function evidence(claim){
  return chromeHtml`<details class="dm-evidence"><summary>Evidence and scope</summary><p>Agent proposal · Not reviewed. Source interpretation does not establish runtime delivery.</p>${(claim?.evidence||[]).map(e=>e.kind==='SELECTED_SOURCE'?sourceHtml(e.source,e.startLine,e.endLine):chromeHtml`<blockquote>${esc(e.fragment?.text)}</blockquote>${e.state?chromeHtml`<p>Retained freshness: ${esc(t(e.state.freshness))} · meaning review: ${esc(t(e.state.verification))}</p>`:''}${Object.values(e.sources||{}).map(s=>sourceHtml(s)).join('')}`).join('')}</details>`;
 }
 function leafFor(row,contract){return (row.field.representations||[]).filter(r=>r.contract===contract.id).map(r=>(contract.leaves||[]).find(l=>l.field===r.field)).filter(Boolean);}
 function stats(data,contract,stage){
  const c=(data.contracts||[]).find(c=>c.id===contract);if(!c)return null;
  const leaves=c.leaves||[],required=leaves.filter(l=>l.requiredWhenBranchMatches),seen=new Set(),count=Object.fromEntries(Object.keys(labels).map(k=>[k,0]));let negative=0,unknown=0,stale=0;
  for(const row of fields(data)){
   const uses=row.field.usage.filter(u=>u.stage===stage&&u.contract===contract);
   for(const leaf of leafFor(row,c).filter(l=>l.requiredWhenBranchMatches)){
    if(seen.has(leaf.field))continue;
    if(uses.some(u=>u.assessment==='stale-evidence'))stale++;
    const observed=uses.filter(u=>u.assessment==='observed-static-use');
    const bounded=uses.some(u=>u.assessment==='no-use-found-in-bounded-complete-scope');
    if(observed.length||bounded){seen.add(leaf.field);if(bounded)negative++;for(const kind of Array.from(new Set(observed.flatMap(u=>u.kinds))))count[kind]++;}
   }
  }
  unknown=required.length-seen.size;
  return {contract:c.title,required:required.length,investigated:seen.size,unknown,stale,negative,categories:count,uninvestigated:required.filter(l=>!seen.has(l.field)).map(l=>l.field)};
 }
 function cellUses(row,stage,contract){return row.field.usage.filter(u=>u.stage===stage&&(!contract||u.contract===contract));}
 function cellLabel(uses){
  if(!uses.length)return t('Not investigated');
  return Array.from(new Set(uses.flatMap(u=>u.assessment==='observed-static-use'?u.kinds.map(k=>labels[k]||k):[states[u.assessment]||u.assessment]))).join(' · ');
 }
 function rowGroup(row){return row.leaf?t('Unmapped contract fields · ')+row.contract.title:row.model?.title||t('Fields');}
 function grouped(rows){const groups=new Map();for(const row of rows){const key=row.leaf?'contract:'+row.contract.id:'model:'+(row.model?.id||'fields');if(!groups.has(key))groups.set(key,{key,title:rowGroup(row),rows:[]});groups.get(key).rows.push(row);}const values=Array.from(groups.values());return values.map(group=>[group.title+(values.filter(other=>other.title===group.title).length>1?' ('+group.key+')':''),group.rows]);}
 const operations={'validate':t('Checks field'),'identify-or-route':t('Uses field as identity / selector'),'calculate-or-decide':t('Uses field in a decision / derivation'),'display':t('Displays field'),'persist':t('Stores field'),'forward':t('Passes field'),'whole-object-serialization-or-comparison':t('Object handling')};
 function readableUse(row,uses){
  if(row.leaf)return t('Unknown — no canonical field mapping');
  if(row.field.selectedProcessField===false)return t('Unknown — outside the selected investigation');
  if(!uses.length)return t('Unknown — not investigated');
  return Array.from(new Set(uses.flatMap(u=>u.assessment==='observed-static-use'?(u.kinds||[]).filter(k=>k!=='forward'||!u.kinds.includes('whole-object-serialization-or-comparison')).map(k=>operations[k]||k):[u.assessment==='no-use-found-in-bounded-complete-scope'?t('No direct use found in ')+(u.coveredScope||t('a bounded scope (scope not supplied)')):u.assessment==='stale-evidence'?t('Unknown — stale evidence'):u.assessment==='unresolved-dynamic-use'?t('Unknown — dynamic use unresolved'):t('Unknown — not investigated')]))).join(' · ');
 }
 function representationHtml(row,uses,conditions=true){return uses.filter(u=>u.assessment==='observed-static-use').map(u=>chromeHtml`<code>${esc(u.representation)}</code>${conditions&&u.condition?chromeHtml`<span class="dm-condition">When: ${esc(u.condition)}</span>`:''}`).join('');}
 function evidenceButton(row,stage,contract,content){const uses=stage?cellUses(row,stage,contract):row.field.usage.filter(u=>!contract||u.contract===contract),label=readableUse(row,uses);return chromeHtml`<button type="button" data-dm-cell data-field="${esc(row.key)}" data-stage="${esc(stage)}" aria-controls="dm-focused" aria-expanded="false" aria-label="${esc(t('View evidence: ')+row.label+', '+(stage||t('all selected stages'))+', '+label)}">${content||esc(label)+chromeHtml(['<span class="dm-evidence-hint">View evidence</span>'])}</button>`;}
 function matrixHtml(rows,stages,contract){
  const transposed=stages.length>Math.max(3,rows.length);
  const fieldButton=r=>chromeHtml`<button type="button" data-dm-cell data-field="${esc(r.key)}" data-stage="" aria-controls="dm-focused" aria-expanded="false" aria-label="${esc(t('View field details: ')+r.label)}">${esc(r.label)}</button>`;
  const cell=(r,stage,label)=>chromeHtml`<td data-stage-label="${esc(label)}">${evidenceButton(r,stage,contract)}${representationHtml(r,cellUses(r,stage,contract))}</td>`;
  const head=transposed?chromeHtml`<th scope="col">Consumer stage</th>${rows.map(r=>chromeHtml`<th scope="col">${fieldButton(r)}</th>`).join('')}`:chromeHtml`<th scope="col">Field</th>${stages.map(stage=>chromeHtml`<th scope="col">${esc(stage)}</th>`).join('')}`;
  const body=transposed?chromeHtml(['<tbody>'])+stages.map(stage=>chromeHtml`<tr><th scope="row">${esc(stage)}</th>${rows.map(r=>cell(r,stage,r.label)).join('')}</tr>`).join('')+'</tbody>':grouped(rows).map(([group,items])=>chromeHtml`<tbody><tr class="dm-group"><th scope="rowgroup" colspan="${stages.length+1}">${esc(group)}${items[0].leaf?chromeHtml(['<span>These paths have no canonical field mapping. Matching names do not establish the same identity.</span>']):''}</th></tr>`+items.map(r=>chromeHtml`<tr><th scope="row">${fieldButton(r)}</th>${stages.map(stage=>cell(r,stage,stage)).join('')}</tr>`).join('')+'</tbody>').join('');
  return chromeHtml`<div class="dm-matrix-scroll" tabindex="0" aria-label="Field usage by consumer"><table><caption>${transposed?t('One stage per row'):t('One field per row')} · View evidence opens the recorded statement and source</caption><thead><tr>${head}</tr></thead>${body}</table></div>`;
 }
 function compactHtml(rows,stages,contract){
  const fieldRow=r=>{const known=stages.filter(stage=>!r.leaf&&r.field.selectedProcessField!==false&&cellUses(r,stage,contract).length),unknown=stages.filter(stage=>!known.includes(stage));
   const conditions=known.flatMap(stage=>cellUses(r,stage,contract).filter(u=>u.condition).map(u=>chromeHtml`<dt>${esc(stage)}</dt><dd>${esc(u.condition)}</dd>`)).join('');
   return chromeHtml`<li class="dm-field-row"><h5>${evidenceButton(r,'',contract,esc(r.label))}</h5><ul>${known.map(stage=>chromeHtml`<li><strong>${esc(stage)}</strong> — ${esc(readableUse(r,cellUses(r,stage,contract)))}${representationHtml(r,cellUses(r,stage,contract),false)}${evidenceButton(r,stage,contract,t('View evidence'))}</li>`).join('')}${unknown.length?chromeHtml`<li>${esc(readableUse(r,[]))} at: ${unknown.map(stage=>chromeHtml`<span data-dm-stage-summary="${esc(stage)}" data-field="${esc(r.key)}">${esc(stage)}</span>`).join(' · ')}.</li>`:''}</ul>${conditions?chromeHtml`<details class="dm-row-conditions"><summary>Conditions for these stages</summary><dl>${conditions}</dl></details>`:''}</li>`;
  };
  return chromeHtml`<div class="dm-compact-overview">${grouped(rows).map(([group,items])=>chromeHtml`<section class="dm-compact-group"><h4>${esc(group)}</h4>${items[0].leaf?chromeHtml(['<p>These contract paths have no canonical field mapping. Matching names do not establish the same identity.</p>']):''}<ul class="dm-field-rows">${items.map(fieldRow).join('')}</ul></section>`).join('')}</div>`;
 }
 function contractContext(row,u,data){
  const c=(data.contracts||[]).find(c=>c.id===u.contract);if(!c)return esc(u.contract)+t(' · No exact contract leaf modeled for this representation.');
  const leaves=leafFor(row,c);
  return chromeHtml`${esc(c.title)} · branch <code>${esc(c.branchPointer)}</code>${leaves.map(l=>chromeHtml`<p><code>${esc(l.field)}</code> · <code>${esc(l.pointer)}</code><br>Required when branch matches: ${esc(l.requiredWhenBranchMatches)} · Accepts null: ${esc(l.nullable)}</p>`).join('')}<p>Origin contract constraints; receiving representation and condition remain separate.</p><details><summary>Exact contract schema and source</summary>${leaves.map(l=>chromeHtml`<pre>${esc(JSON.stringify(l.schema,null,2))}</pre>`).join('')}${sourceHtml(c.source)}</details>`;
 }
 function transferHtml(u,next,data){
  if(!next)return '';
  const edge=(data.request?.transfers||[]).find(e=>e.from===u.stage&&e.to===next.stage);
  if(!edge)return chromeHtml(['<div class="dm-transfer dm-transfer-unknown">↓ Reading order only · transfer not established</div>']);
  const label=edge.authority==='SOURCE_INTERPRETATION'?t('Source-interpreted transfer'):edge.authority==='DECLARED_TRANSFER'?t('Declared transfer · runtime unverified'):t('Unknown transfer');
  return chromeHtml`<div class="dm-transfer ${edge.authority==='UNKNOWN'?'dm-transfer-unknown':''}"><p>↓ ${esc(label)}</p><details><summary>Transfer condition and evidence</summary><p>${esc(edge.condition)}</p><p>${esc(edge.claim.text)}</p>${evidence(edge.claim)}</details></div>`;
 }
 function referenceLinks(stage,data){
  return (data.modelReferences||[]).filter(link=>link.stages?.includes(stage)).map(link=>chromeHtml`<p><a data-dm-context data-dm-return-anchor="${esc(link.anchor)}" href="${esc(data.modelHrefs?.[link.id]||link.href)}">${esc(link.title)}</a></p>`).join('');
 }
 function focused(row,uses,data){
  if(row.leaf)return chromeHtml`<h3 tabindex="-1" class="dm-flow-heading">${esc(row.label)}</h3><p>Unmapped contract leaf · not investigated. This is not zero usage.</p><p>Exact schema pointer: <code>${esc(row.leaf.pointer)}</code></p><pre>${esc(JSON.stringify(row.leaf.schema,null,2))}</pre>${row.contract.source?sourceHtml(row.contract.source):''}`;
  const canonical=data.modelHrefs?.[row.model.id]||`${data.request?'../data-model/':''}${row.model.id}.html`;
  return chromeHtml`<h3 tabindex="-1" class="dm-flow-heading">${esc(row.label)}</h3><p>${esc(row.field.meaning?.text)}</p>${inventoryLabel(row)?chromeHtml`<p>${esc(inventoryLabel(row))}. This is not zero usage.</p>`:''}<p><a data-dm-context href="${esc(canonical)}?field=${encodeURIComponent(row.field.id)}#field-${esc(row.field.id)}">Canonical field and representations</a></p><ol class="dm-flow-nodes">${uses.map((u,i)=>chromeHtml`<li class="dm-flow-node ${u.assessment==='observed-static-use'?'':'dm-uncertain'}"><h4>${i+1}. ${esc(u.stage)}</h4><p class="dm-badges">${esc(cellLabel([u]))}</p><p class="dm-representation"><code>${esc(u.representation)}</code></p><p>${esc(u.claim.text)}</p><details><summary>Contract, condition and unresolved scope</summary><dl><dt>Producer → consumer</dt><dd>${esc(u.producer)} → ${esc(u.consumer)}</dd><dt>Representation</dt><dd><code>${esc(u.representation)}</code></dd><dt>Contract branch / condition</dt><dd>${contractContext(row,u,data)}<p>${esc(u.condition)}</p></dd><dt>Covered scope</dt><dd>${esc(u.coveredScope)}</dd><dt>Unresolved</dt><dd>${esc(u.unresolved)}</dd></dl></details>${evidence(u.claim)}${transferHtml(u,uses[i+1],data)}</li>`).join('')||chromeHtml(['<li class="dm-uncertain">Not investigated in the selected contract/consumer scope. This is not zero usage.</li>'])}</ol><p>Stages summarize static source interpretation. Their order does not prove a completed runtime handoff. Whole-object use does not prove a named business read of every child.</p>`;
 }
 function mount(container,data){
  const rows=overviewRows(data);if(!rows.length)return;
  const query=new URLSearchParams(location.search||'');
  const requestedField=query.get('field')||(location.hash.startsWith('#field-')?location.hash.slice(7):'');
  let selected=(requestedField&&rows.find(r=>[r.key,r.field.id,r.field.path].some(k=>String(k||'').toLowerCase()===requestedField.toLowerCase())))||rows[0];
  const requestedStages=data.request?.stages;
  const stages=requestedStages||Array.from(new Set(rows.flatMap(r=>r.field.usage.map(u=>u.stage))));
  let stage=query.get('stage')||'';if(!stages.includes(stage))stage='';
  let detailStage=query.get('cellStage')||'';if(!stages.includes(detailStage))detailStage='';
  const contracts=data.contracts||[];
  const initialContract=query.get('contract')||'';
  container.innerHTML=chromeHtml`<section class="dm-overview" id="dm-overview" tabindex="-1"><div class="dm-overview-summary"></div><div class="dm-controls"><label>Contract<select class="dm-contract-select" aria-label="Contract scope"><option value="">All documented representations</option>${contracts.map(c=>chromeHtml`<option value="${esc(c.id)}">${esc(c.title)}</option>`).join('')}</select></label><label>Stage<select class="dm-stage-select" aria-label="Consumer stage"><option value="">All documented stages</option>${stages.map(s=>chromeHtml`<option value="${esc(s)}">${esc(s)}</option>`).join('')}</select></label></div><div class="dm-matrix"></div><p class="dm-comparison-end">End of field overview. Evidence opens separately below.</p><div class="dm-counts" aria-live="polite"></div></section><details class="dm-drilldown"><summary>Selected field details</summary><label>Field<select class="dm-field-select" aria-label="Field">${rows.map(r=>chromeHtml`<option value="${esc(r.key)}">${esc(r.label)}</option>`).join('')}</select></label><div class="dm-detail-nav"><button type="button" class="dm-return-overview">Back to field overview</button><span class="dm-selected-context" aria-live="polite"></span></div><section class="dm-focused" id="dm-focused" tabindex="-1" aria-live="polite"></section></details>`;
  const fieldSelect=container.querySelector('.dm-field-select'),contractSelect=container.querySelector('.dm-contract-select'),stageSelect=container.querySelector('.dm-stage-select'),matrix=container.querySelector('.dm-matrix'),flow=container.querySelector('.dm-focused');
  fieldSelect.value=selected.key;if(contracts.some(c=>c.id===initialContract))contractSelect.value=initialContract;
  function locationUpdate(){const url=new URL(location.href);url.searchParams.set('field',selected.key);if(detailStage)url.searchParams.set('cellStage',detailStage);else url.searchParams.delete('cellStage');if(stage)url.searchParams.set('stage',stage);else url.searchParams.delete('stage');if(contractSelect.value)url.searchParams.set('contract',contractSelect.value);else url.searchParams.delete('contract');history.replaceState(null,'',url);}
  function update(focus=false){
   fieldSelect.value=selected.key;stageSelect.value=stage;
   const uses=selected.field.usage.filter(u=>(!contractSelect.value||u.contract===contractSelect.value)&&(!(detailStage||stage)||u.stage===(detailStage||stage)));
   locationUpdate();flow.innerHTML=(detailStage?chromeHtml`<p>Selected stage: ${esc(detailStage)}</p>${referenceLinks(detailStage,data)}`:'')+focused(selected,uses,data);contextLinks(document,data);
   container.querySelectorAll('[data-dm-cell]').forEach(button=>{const active=button.dataset.field===selected.key&&button.dataset.stage===detailStage;button.setAttribute('aria-expanded',String(active&&(focus||container.querySelector('.dm-drilldown').open)));if(active)button.setAttribute('aria-current','true');else button.removeAttribute('aria-current');});
   container.querySelector('.dm-selected-context').textContent=selected.label+(detailStage?' · '+detailStage:t(' · all selected stages'));
   const summaryStages=stage?[stage]:stages;const summary=summaryStages.map(s=>stats(data,contractSelect.value,s)).map((s,i)=>s?chromeHtml`<details><summary>${esc(summaryStages[i])}: Required leaves investigated: ${s.investigated}/${s.required} · Unknown / not investigated: ${s.unknown}${s.stale?' · '+t('Stale evidence:')+' '+s.stale:''}</summary><p>${esc(s.contract)}. Categories overlap; array element leaves counted once.</p><ul>${Object.entries(s.categories).map(([k,n])=>chromeHtml`<li>${esc(labels[k])}: ${n}</li>`).join('')}</ul><p>Scoped negative findings: ${s.negative}. Uninvestigated leaves: ${esc(s.uninvestigated.join(', ')||t('none'))}.</p></details>`:'').join('');
   container.querySelector('.dm-counts').innerHTML=summary||chromeHtml(['<p>Select an exact contract branch for required-leaf counts. Counts describe this documented scope, not utilization or runtime frequency.</p>']);
   if(focus){container.querySelector('.dm-drilldown').open=true;flow.querySelector('h3')?.focus({preventScroll:true});}
  }
  function table(){const visibleStages=stage?[stage]:stages,visibleRows=overviewRows(data,contractSelect.value);
   container.querySelector('.dm-overview-summary').innerHTML=overviewHtml(data,visibleRows,visibleStages);
   matrix.innerHTML=chromeHtml`<div class="dm-wide-overview">${matrixHtml(visibleRows,visibleStages,contractSelect.value)}</div>${compactHtml(visibleRows,visibleStages,contractSelect.value)}`;
   matrix.querySelectorAll('[data-dm-cell]').forEach(button=>button.addEventListener('click',()=>{selected=rows.find(r=>r.key===button.dataset.field);detailStage=button.dataset.stage;update(true);container.querySelector('.dm-drilldown').scrollIntoView({block:'start'});}));
  }
  container.querySelector('.dm-return-overview').addEventListener('click',()=>{container.querySelector('.dm-drilldown').open=false;const button=Array.from(matrix.querySelectorAll('[data-dm-cell]')).find(b=>b.dataset.field===selected.key&&b.dataset.stage===detailStage&&(!b.getClientRects||b.getClientRects().length));if(button?.focus){button.scrollIntoView({block:'center',inline:'nearest'});button.focus({preventScroll:true});}else{const overview=container.querySelector('.dm-overview');overview.focus();overview.scrollIntoView({block:'start'});}update();});
  fieldSelect.addEventListener('change',()=>{selected=rows.find(r=>r.key===fieldSelect.value);detailStage='';update(true);});
  stageSelect.addEventListener('change',()=>{stage=stageSelect.value;detailStage='';table();update();});contractSelect.addEventListener('change',()=>{detailStage='';table();update();});table();update();
  return {selectField(id){const found=rows.find(r=>r.field.id===id||r.key===id);if(found){selected=found;detailStage='';update(true);container.querySelector('.dm-drilldown').scrollIntoView({block:'start'});}}};
 }
 function contextLinks(root,data){
  const q=new URLSearchParams(location.search||''),process=q.get('fromProcess')||data.request?.subject?.slice(9),anchor=data.request?(data.returnAnchor||data.request.anchor):q.get('fromAnchor');
  if(!/^[a-zA-Z0-9_-]{1,160}$/.test(process||'')||!/^[a-zA-Z0-9_-]{1,160}$/.test(anchor||''))return;
  root.querySelectorAll('[data-dm-context]').forEach(link=>{const url=new URL(link.getAttribute('href'),location.href);url.searchParams.set('fromProcess',process);url.searchParams.set('fromAnchor',link.getAttribute('data-dm-return-anchor')||anchor);const fromField=q.get('fromField')||(data.request?q.get('field'):null);if(fromField)url.searchParams.set('fromField',fromField);for(const name of ['stage','contract','cellStage']){const key='from'+name[0].toUpperCase()+name.slice(1),value=data.request?q.get(name):q.get(key);if(value)url.searchParams.set(key,value);else url.searchParams.delete(key);}link.setAttribute('href',url.pathname+url.search+url.hash);});
  const processPath=data.processHrefs?.[process]||'../scenarios/'+process+'.html';
  const returnQuery=new URLSearchParams({view:'process'});for(const name of ['field','stage','contract','cellStage']){const value=q.get('from'+name[0].toUpperCase()+name.slice(1));if(value)returnQuery.set(name,value);}
  const target=document.getElementById('dm-return');if(target)target.innerHTML=chromeHtml`<a href="${esc(processPath)}?${esc(returnQuery.toString())}#${esc(anchor)}">Return to the same process step</a>`;
 }
 function processMount(){
  if(typeof D==='undefined'||!D.processDataFlow)return;
  const data={...D.processDataFlow},root=document.querySelector('#scenario-content .question-reader');if(!root||root.querySelector('.dm-process-controls'))return;
  const all=[...root.querySelectorAll('.question-mobile-nav,.question-section')],panel=document.createElement('div');panel.id='dm-process-view';panel.setAttribute('role','tabpanel');panel.setAttribute('aria-labelledby','dm-tab-process');all.forEach(node=>panel.appendChild(node));root.appendChild(panel);
  const controls=document.createElement('div');controls.className='dm-process-controls';controls.innerHTML=chromeHtml(['<div role="tablist" aria-label="Process reading view"><button type="button" role="tab" id="dm-tab-process" aria-controls="dm-process-view" aria-selected="true">Process</button><button type="button" role="tab" id="dm-tab-flow" aria-controls="dm-process-flow" aria-selected="false" tabindex="-1">Data Flow</button></div>']);root.querySelector('header')?.after(controls);
  const flow=document.createElement('section');flow.id='dm-process-flow';flow.className='dm-page dm-process';flow.hidden=true;flow.setAttribute('role','tabpanel');flow.setAttribute('aria-labelledby','dm-tab-flow');flow.innerHTML=chromeHtml`<p class="dm-origin">Agent proposal · Not reviewed · <a href="#${esc(data.request.anchor)}" class="dm-back-process">Return to process step</a></p><div class="dm-process-usage"></div>`;root.appendChild(flow);
  const references=document.createElement('p');references.className='dm-relations';references.innerHTML=t('Data handled by this step: ')+(data.relatedModels||data.models).map(m=>chromeHtml`<a data-dm-context data-dm-return-anchor="${esc(data.request.anchor)}" href="../data-model/${esc(m.id)}.html?fromProcess=${encodeURIComponent(data.request.subject.slice(9))}&fromAnchor=${encodeURIComponent(data.request.anchor)}">${esc(m.title)}</a>`).join(' · ');const step=document.getElementById(data.request.anchor);(step?.querySelector(':scope > div')||step)?.appendChild(references);
  let mounted=false;const tabs=[...controls.querySelectorAll('[role=tab]')];
  function view(name,save=true){const on=name==='data-flow';if(on&&!panel.hidden){const anchor=location.hash.slice(1);data.returnAnchor=/^question-[a-zA-Z0-9_-]+$/.test(anchor)?anchor:(data.returnAnchor||data.request.anchor);flow.querySelector('.dm-back-process').setAttribute('href','#'+data.returnAnchor);} panel.hidden=on;flow.hidden=!on;tabs.forEach((tab,i)=>{tab.setAttribute('aria-selected',String(on?i===1:i===0));tab.tabIndex=(on?i===1:i===0)?0:-1;});if(on&&!mounted){mount(flow.querySelector('.dm-process-usage'),data);mounted=true;}contextLinks(document,data);if(save){const u=new URL(location.href);u.searchParams.set('view',on?'data-flow':'process');history.replaceState(null,'',u);}}
  tabs.forEach((tab,i)=>{tab.addEventListener('click',()=>view(i?'data-flow':'process'));tab.addEventListener('keydown',event=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(event.key)){event.preventDefault();event.stopPropagation();const next=event.key==='Home'?0:event.key==='End'?1:1-i;view(next?'data-flow':'process');tabs[next].focus();}});});
  flow.querySelector('.dm-back-process').addEventListener('click',()=>view('process'));
  controls.setAttribute('data-process-context',data.request.subject);
  root.dmSelectProcess=()=>view('process');
  view(new URLSearchParams(location.search||'').get('view')==='data-flow'?'data-flow':'process',false);
 }
 const api={stats,fields,overviewRows,overviewHtml,inventoryLabel,cellLabel,readableUse,matrixHtml,compactHtml,focused,referenceLinks,contextLinks,mount,processMount};
  readers.set(language,api);return api;
 }
 const rootLanguage=()=>document.documentElement?.lang==='ru'?'ru':'en';
 const base=reader(rootLanguage());
 const api=Object.fromEntries(Object.keys(base).map(name=>[name,(...args)=>{
  const supplied=args.find(arg=>arg&&typeof arg==='object'&&typeof arg.language==='string');
  const language=supplied?.language??rootLanguage();
  if(!['en','ru'].includes(language))throw new Error(clewLocale(rootLanguage(),CLEW_RU_MESSAGES).t('Documentation language must be en or ru'));
  return reader(language)[name](...args);
 }]));
 window.CodeclewDataModel=api;
 const {mount,contextLinks,processMount}=api;
 const node=document.getElementById('dm-data');
 if(node?.textContent?.trim()){
  const data=JSON.parse(node.textContent),container=document.getElementById('dm-usage'),handle=container?mount(container,data):null;
  contextLinks(document,data);document.querySelectorAll('[data-dm-field]').forEach(link=>link.addEventListener('click',event=>{
   if(!handle||event.button!==0||event.metaKey||event.ctrlKey||event.shiftKey||event.altKey)return;
   event.preventDefault();handle.selectField(link.dataset.dmField);
   if(location.hash!=='#field-usage')history.pushState(null,'','#field-usage');
  }));
 }
 if(typeof D!=='undefined'&&D.processDataFlow){
  const original=showSequentialReader;showSequentialReader=function(){original();processMount();};
  document.addEventListener('click',event=>{if(event.target.closest?.('.question-link'))document.querySelector('#scenario-content .question-reader')?.dmSelectProcess?.();},true);processMount();
 }
})();
