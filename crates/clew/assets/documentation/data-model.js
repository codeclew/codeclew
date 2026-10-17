// Authored field-use reader. No graph, scope or transfer is inferred at runtime.
(function(){
 'use strict';
 const labels={'validate':'Validated','identify-or-route':'Identity / routing','calculate-or-decide':'Decision / calculation','display':'Displayed','persist':'Stored','forward':'Forwarded','whole-object-serialization-or-comparison':'Whole-object use'};
 const states={'observed-static-use':'Seen in retained source','no-use-found-in-bounded-complete-scope':'No direct use found in this scope','not-investigated':'Not investigated','unresolved-dynamic-use':'Dynamic use unresolved','stale-evidence':'Stale evidence — use unknown'};
 const esc=value=>String(value??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
 const fields=data=>(data.models||[]).flatMap(model=>(model.fields||[]).map(field=>({model,field,key:model.id+'/'+field.id,label:model.title+'.'+field.path})));
 // The inventory is bounded to the models/contracts supplied by the renderer.
 function overviewRows(data,contract=''){
  const rows=fields(data).filter(r=>!contract||(r.field.representations||[]).some(rep=>rep.contract===contract));
  for(const c of (data.contracts||[]).filter(c=>!contract||c.id===contract))for(const leaf of c.leaves||[]){
   if(rows.some(r=>leafFor(r,c).some(l=>l.field===leaf.field)))continue;
   rows.push({key:'contract:'+c.id+'/'+leaf.field,label:c.title+' · '+leaf.field,contract:c,leaf,field:{usage:[],representations:[],meaning:{text:'Contract leaf without a canonical field mapping in this documented scope.'}}});
  }
  return rows;
 }
 function inventoryLabel(row){return row.leaf?'Unmapped · unknown':row.field.selectedProcessField===false?'Outside trace · unknown':'';}
 function fullInventoryLabel(row){return row.leaf?'Unmapped contract leaf · not investigated':row.field.selectedProcessField===false?'Outside investigated process scope · use unknown':'';}
 function overviewHtml(data,rows,stages){
  const known=fields(data),traced=data.request?.fields?.length,selectedModels=new Set((data.request?.fields||[]).map(f=>f.model));
  const related=data.request?(data.models||[]).filter(m=>!selectedModels.has(m.id)).map(m=>`${esc(m.title)}: ${(m.fields||[]).length} fields`).join(' · '):'';
  const full=`<p class="dm-inventory">${rows.length} displayed rows · ${stages.length} stages. ${known.length} known model fields / container paths in ${data.request?'selected and explicitly related models':'this model scope'}${related?` (related inventory: ${related})`:''}${traced===undefined?'':` · ${traced} selected process fields`}. ${(data.contracts||[]).map(c=>`${esc(c.title)}: ${(c.leaves||[]).length} contract leaves`).join(' · ')}. Mapped leaves share model rows; unmapped leaves have separate rows. Required-leaf counts below are separate.</p><p class="dm-legend">Named usage keeps its stated category. Forwarding / whole-object use does not prove a named business read. “No direct use found in this scope” requires an explicit bounded finding. Missing evidence, outside scope, dynamic unresolved and stale remain unknown, never unused.</p><p class="dm-scroll-hint">Scroll within the table to compare stages. Select a cell for details; the overview filters stay unchanged.</p>`;
  return `<h3>All fields by stage</h3><p class="dm-overview-counts">${rows.length} rows · ${stages.length} stages · ${known.length} model paths${traced===undefined?'':` · ${traced} traced fields`}</p><p class="dm-overview-key">Named use · Whole-object / forwarded · Scoped negative · Unknown (outside trace, missing, dynamic or stale). Unknown ≠ unused.</p><details class="dm-scope-details"><summary>Scope and evidence key</summary><p>${esc(data.request?.title||'Field-use scope')}</p>${full}<p>${esc(data.scope||'Only documented model/consumer scope is investigated.')}</p></details>`;
 }
 function sourceHtml(source,start=source.startLine,end=source.endLine){
  const text=(source.text||'').split('\n').slice(Math.max(0,start-source.startLine),end-source.startLine+1).join('\n');
  const base=(source.url||'').split('#')[0],link=/^https?:\/\//.test(base)?`<a href="${esc(base)}#L${start}-L${end}">${esc(source.file)}:${start}–${end}</a>`:esc(source.file)+':'+start+'–'+end;
  return `<p>${link} · revision <code>${esc(source.revision)}</code></p><pre tabindex="0">${esc(text)}</pre><details><summary>Exact source binding</summary><pre>${esc(JSON.stringify({sourceId:source.id,textDigest:source.textDigest,authority:source.authority,startLine:start,endLine:end},null,2))}</pre></details>`;
 }
 function evidence(claim){
  return `<details class="dm-evidence"><summary>Evidence and scope</summary><p>Agent proposal · Not reviewed. Source interpretation does not establish runtime delivery.</p>${(claim?.evidence||[]).map(e=>e.kind==='SELECTED_SOURCE'?sourceHtml(e.source,e.startLine,e.endLine):`<blockquote>${esc(e.fragment?.text)}</blockquote>${e.state?`<p>Retained freshness: ${esc(e.state.freshness)} · meaning review: ${esc(e.state.verification)}</p>`:''}${Object.values(e.sources||{}).map(s=>sourceHtml(s)).join('')}`).join('')}</details>`;
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
  if(!uses.length)return 'Not investigated';
  return Array.from(new Set(uses.flatMap(u=>u.assessment==='observed-static-use'?u.kinds.map(k=>labels[k]||k):[states[u.assessment]||u.assessment]))).join(' · ');
 }
 function matrixHtml(rows,stages,contract){
  const transposed=stages.length>Math.max(3,rows.length);
  const fieldButton=r=>`<button type="button" data-dm-cell data-field="${esc(r.key)}" data-stage="">${esc(r.label)}</button>`;
  const cell=(r,stage,label)=>`<td data-stage-label="${esc(label)}"><button type="button" data-dm-cell data-field="${esc(r.key)}" data-stage="${esc(stage)}" aria-label="${esc(r.label+' · '+stage+' · '+(fullInventoryLabel(r)||cellLabel(cellUses(r,stage,contract))))}">${esc(inventoryLabel(r)||cellLabel(cellUses(r,stage,contract)))}</button></td>`;
  const head=transposed?`<th scope="col">Consumer stage</th>${rows.map(r=>`<th scope="col">${fieldButton(r)}</th>`).join('')}`:`<th scope="col">Field</th>${stages.map(stage=>`<th scope="col">${esc(stage)}</th>`).join('')}`;
  const body=transposed?stages.map(stage=>`<tr><th scope="row">${esc(stage)}</th>${rows.map(r=>cell(r,stage,r.label)).join('')}</tr>`).join(''):rows.map(r=>`<tr><th scope="row">${fieldButton(r)}</th>${stages.map(stage=>cell(r,stage,stage)).join('')}</tr>`).join('');
  return `<div class="dm-matrix-scroll" tabindex="0" aria-label="Field usage by consumer"><table><caption>${transposed?'One stage per row':'One field per row'} · scroll to compare · select a cell for evidence</caption><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div>`;
 }
 function contractContext(row,u,data){
  const c=(data.contracts||[]).find(c=>c.id===u.contract);if(!c)return esc(u.contract)+' · No exact contract leaf modeled for this representation.';
  const leaves=leafFor(row,c);
  return `${esc(c.title)} · branch <code>${esc(c.branchPointer)}</code>${leaves.map(l=>`<p><code>${esc(l.field)}</code> · <code>${esc(l.pointer)}</code><br>Required when branch matches: ${esc(l.requiredWhenBranchMatches)} · Accepts null: ${esc(l.nullable)}</p>`).join('')}<p>Origin contract constraints; receiving representation and condition remain separate.</p><details><summary>Exact contract schema and source</summary>${leaves.map(l=>`<pre>${esc(JSON.stringify(l.schema,null,2))}</pre>`).join('')}${sourceHtml(c.source)}</details>`;
 }
 function transferHtml(u,next,data){
  if(!next)return '';
  const edge=(data.request?.transfers||[]).find(e=>e.from===u.stage&&e.to===next.stage);
  if(!edge)return '<div class="dm-transfer dm-transfer-unknown">↓ Reading order only · transfer not established</div>';
  const label=edge.authority==='SOURCE_INTERPRETATION'?'Source-interpreted transfer':edge.authority==='DECLARED_TRANSFER'?'Declared transfer · runtime unverified':'Unknown transfer';
  return `<div class="dm-transfer ${edge.authority==='UNKNOWN'?'dm-transfer-unknown':''}"><p>↓ ${esc(label)}</p><details><summary>Transfer condition and evidence</summary><p>${esc(edge.condition)}</p><p>${esc(edge.claim.text)}</p>${evidence(edge.claim)}</details></div>`;
 }
 function focused(row,uses,data){
  if(row.leaf)return `<h3 tabindex="-1" class="dm-flow-heading">${esc(row.label)}</h3><p>Unmapped contract leaf · not investigated. This is not zero usage.</p><p>Exact schema pointer: <code>${esc(row.leaf.pointer)}</code></p><pre>${esc(JSON.stringify(row.leaf.schema,null,2))}</pre>${row.contract.source?sourceHtml(row.contract.source):''}`;
  const canonical=data.modelHrefs?.[row.model.id]||`${data.request?'../data-model/':''}${row.model.id}.html`;
  return `<h3 tabindex="-1" class="dm-flow-heading">${esc(row.label)}</h3><p>${esc(row.field.meaning?.text)}</p>${inventoryLabel(row)?`<p>${esc(inventoryLabel(row))}. This is not zero usage.</p>`:''}<p><a data-dm-context href="${esc(canonical)}?field=${encodeURIComponent(row.field.id)}#field-${esc(row.field.id)}">Canonical field and representations</a></p><ol class="dm-flow-nodes">${uses.map((u,i)=>`<li class="dm-flow-node ${u.assessment==='observed-static-use'?'':'dm-uncertain'}"><h4>${i+1}. ${esc(u.stage)}</h4><p class="dm-badges">${esc(cellLabel([u]))}</p><p class="dm-representation"><code>${esc(u.representation)}</code></p><p>${esc(u.claim.text)}</p><details><summary>Contract, condition and unresolved scope</summary><dl><dt>Producer → consumer</dt><dd>${esc(u.producer)} → ${esc(u.consumer)}</dd><dt>Representation</dt><dd><code>${esc(u.representation)}</code></dd><dt>Contract branch / condition</dt><dd>${contractContext(row,u,data)}<p>${esc(u.condition)}</p></dd><dt>Covered scope</dt><dd>${esc(u.coveredScope)}</dd><dt>Unresolved</dt><dd>${esc(u.unresolved)}</dd></dl></details>${evidence(u.claim)}${transferHtml(u,uses[i+1],data)}</li>`).join('')||'<li class="dm-uncertain">Not investigated in the selected contract/consumer scope. This is not zero usage.</li>'}</ol><p>Stages summarize static source interpretation. Their order does not prove a completed runtime handoff. Whole-object use does not prove a named business read of every child.</p>`;
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
  container.innerHTML=`<section class="dm-overview" id="dm-overview" tabindex="-1"><div class="dm-overview-summary"></div><div class="dm-controls"><label>Contract<select class="dm-contract-select" aria-label="Contract scope"><option value="">All documented representations</option>${contracts.map(c=>`<option value="${esc(c.id)}">${esc(c.title)}</option>`).join('')}</select></label><label>Stage<select class="dm-stage-select" aria-label="Consumer stage"><option value="">All documented stages</option>${stages.map(s=>`<option value="${esc(s)}">${esc(s)}</option>`).join('')}</select></label></div><div class="dm-matrix"></div><div class="dm-counts" aria-live="polite"></div></section><details class="dm-drilldown"><summary>Selected field details</summary><label>Field<select class="dm-field-select" aria-label="Field">${rows.map(r=>`<option value="${esc(r.key)}">${esc(r.label)}</option>`).join('')}</select></label><p><button type="button" class="dm-return-overview">Return to all-fields overview</button></p><section class="dm-focused" id="dm-focused" tabindex="-1" aria-live="polite"></section></details>`;
  const fieldSelect=container.querySelector('.dm-field-select'),contractSelect=container.querySelector('.dm-contract-select'),stageSelect=container.querySelector('.dm-stage-select'),matrix=container.querySelector('.dm-matrix'),flow=container.querySelector('.dm-focused');
  fieldSelect.value=selected.key;if(contracts.some(c=>c.id===initialContract))contractSelect.value=initialContract;
  function locationUpdate(){const url=new URL(location.href);url.searchParams.set('field',selected.key);if(detailStage)url.searchParams.set('cellStage',detailStage);else url.searchParams.delete('cellStage');if(stage)url.searchParams.set('stage',stage);else url.searchParams.delete('stage');if(contractSelect.value)url.searchParams.set('contract',contractSelect.value);else url.searchParams.delete('contract');history.replaceState(null,'',url);}
  function update(focus=false){
   fieldSelect.value=selected.key;stageSelect.value=stage;
   const uses=selected.field.usage.filter(u=>(!contractSelect.value||u.contract===contractSelect.value)&&(!(detailStage||stage)||u.stage===(detailStage||stage)));
   locationUpdate();flow.innerHTML=(detailStage?`<p>Selected stage: ${esc(detailStage)}</p>`:'')+focused(selected,uses,data);contextLinks(document,data);
   container.querySelectorAll('[data-dm-cell]').forEach(button=>button.setAttribute('aria-pressed',String(button.dataset.field===selected.key&&(!detailStage||button.dataset.stage===detailStage))));
   const summaryStages=stage?[stage]:stages;const summary=summaryStages.map(s=>stats(data,contractSelect.value,s)).map((s,i)=>s?`<details><summary>${esc(summaryStages[i])}: ${s.investigated}/${s.required} required leaves investigated · ${s.unknown} unknown/not investigated${s.stale?' · '+s.stale+' stale':''}</summary><p>${esc(s.contract)}. Categories overlap; array element leaves counted once.</p><ul>${Object.entries(s.categories).map(([k,n])=>`<li>${esc(labels[k])}: ${n}</li>`).join('')}</ul><p>Scoped negative findings: ${s.negative}. Uninvestigated leaves: ${esc(s.uninvestigated.join(', ')||'none')}.</p></details>`:'').join('');
   container.querySelector('.dm-counts').innerHTML=summary||'<p>Select an exact contract branch for required-leaf counts. Counts describe this documented scope, not utilization or runtime frequency.</p>';
   if(focus){container.querySelector('.dm-drilldown').open=true;flow.querySelector('h3')?.focus({preventScroll:true});}
  }
  function table(){const visibleStages=stage?[stage]:stages,visibleRows=overviewRows(data,contractSelect.value);
   container.querySelector('.dm-overview-summary').innerHTML=overviewHtml(data,visibleRows,visibleStages);
   matrix.innerHTML=matrixHtml(visibleRows,visibleStages,contractSelect.value);
   matrix.querySelectorAll('[data-dm-cell]').forEach(button=>button.addEventListener('click',()=>{selected=rows.find(r=>r.key===button.dataset.field);detailStage=button.dataset.stage;update(true);container.querySelector('.dm-drilldown').scrollIntoView({block:'start'});}));
  }
  container.querySelector('.dm-return-overview').addEventListener('click',()=>{const overview=container.querySelector('.dm-overview');overview.focus();overview.scrollIntoView({block:'start'});});
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
  const target=document.getElementById('dm-return');if(target)target.innerHTML=`<a href="${esc(processPath)}?${esc(returnQuery.toString())}#${esc(anchor)}">Return to the same process step</a>`;
 }
 function processMount(){
  if(typeof D==='undefined'||!D.processDataFlow)return;
  const data={...D.processDataFlow},root=document.querySelector('#scenario-content .question-reader');if(!root||root.querySelector('.dm-process-controls'))return;
  const all=[...root.querySelectorAll('.question-mobile-nav,.question-section')],panel=document.createElement('div');panel.id='dm-process-view';panel.setAttribute('role','tabpanel');panel.setAttribute('aria-labelledby','dm-tab-process');all.forEach(node=>panel.appendChild(node));root.appendChild(panel);
  const controls=document.createElement('div');controls.className='dm-process-controls';controls.innerHTML='<div role="tablist" aria-label="Process reading view"><button type="button" role="tab" id="dm-tab-process" aria-controls="dm-process-view" aria-selected="true">Process</button><button type="button" role="tab" id="dm-tab-flow" aria-controls="dm-process-flow" aria-selected="false" tabindex="-1">Data Flow</button></div>';root.querySelector('header')?.after(controls);
  const flow=document.createElement('section');flow.id='dm-process-flow';flow.className='dm-page dm-process';flow.hidden=true;flow.setAttribute('role','tabpanel');flow.setAttribute('aria-labelledby','dm-tab-flow');flow.innerHTML=`<p class="dm-origin">Agent proposal · Not reviewed · <a href="#${esc(data.request.anchor)}" class="dm-back-process">Return to process step</a></p><div class="dm-process-usage"></div>`;root.appendChild(flow);
  const references=document.createElement('p');references.className='dm-relations';references.innerHTML='Data handled by this step: '+(data.relatedModels||data.models).map(m=>`<a data-dm-context data-dm-return-anchor="${esc(data.request.anchor)}" href="../data-model/${esc(m.id)}.html?fromProcess=${encodeURIComponent(data.request.subject.slice(9))}&fromAnchor=${encodeURIComponent(data.request.anchor)}">${esc(m.title)}</a>`).join(' · ');const step=document.getElementById(data.request.anchor);(step?.querySelector(':scope > div')||step)?.appendChild(references);
  let mounted=false;const tabs=[...controls.querySelectorAll('[role=tab]')];
  function view(name,save=true){const on=name==='data-flow';if(on&&!panel.hidden){const anchor=location.hash.slice(1);data.returnAnchor=/^question-[a-zA-Z0-9_-]+$/.test(anchor)?anchor:(data.returnAnchor||data.request.anchor);flow.querySelector('.dm-back-process').setAttribute('href','#'+data.returnAnchor);} panel.hidden=on;flow.hidden=!on;tabs.forEach((tab,i)=>{tab.setAttribute('aria-selected',String(on?i===1:i===0));tab.tabIndex=(on?i===1:i===0)?0:-1;});if(on&&!mounted){mount(flow.querySelector('.dm-process-usage'),data);mounted=true;}contextLinks(document,data);if(save){const u=new URL(location.href);u.searchParams.set('view',on?'data-flow':'process');history.replaceState(null,'',u);}}
  tabs.forEach((tab,i)=>{tab.addEventListener('click',()=>view(i?'data-flow':'process'));tab.addEventListener('keydown',event=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(event.key)){event.preventDefault();event.stopPropagation();const next=event.key==='Home'?0:event.key==='End'?1:1-i;view(next?'data-flow':'process');tabs[next].focus();}});});
  flow.querySelector('.dm-back-process').addEventListener('click',()=>view('process'));
  controls.setAttribute('data-process-context',data.request.subject);
  root.dmSelectProcess=()=>view('process');
  view(new URLSearchParams(location.search||'').get('view')==='data-flow'?'data-flow':'process',false);
 }
 const api={stats,fields,overviewRows,overviewHtml,inventoryLabel,cellLabel,matrixHtml,focused,mount,processMount};window.CodeclewDataModel=api;
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
