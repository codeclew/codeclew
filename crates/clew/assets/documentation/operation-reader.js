'use strict';
const D=JSON.parse(document.getElementById('document-data').textContent);
const esc=value=>String(value??'').replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;').replace(/'/g,'&#39;');
const t=value=>value;
const chromeHtml=(parts,...values)=>parts.reduce((out,part,index)=>out+part+(index<values.length?values[index]:''),'');
function reveal(id){const target=document.getElementById(id);if(!target)return;for(let parent=target;parent;parent=parent.parentElement)if(parent.tagName==='DETAILS')parent.open=true;target.setAttribute('tabindex','-1');target.scrollIntoView({block:'start'});target.focus({preventScroll:true});}
function renderSequence(){const host=document.getElementById('sequence-host');if(!host||host.clientWidth<160)return;const op=D.operations.find(o=>o.id===D.readerPresentation.operationId);host.innerHTML=diagram(op,host.clientWidth);}
document.addEventListener('click',event=>{const anchor=event.target.closest('a[href]');if(!anchor)return;const op=D.operations.find(o=>o.id===D.readerPresentation.operationId);if(anchor.dataset.event){const row=sequenceEvents(op).find(e=>e.id===anchor.dataset.event);if(!row)return;event.preventDefault();const records=D.operationSources[op.id],ids=row.sourceIds.filter(id=>records[id]);document.getElementById('diagram-selected-evidence').innerHTML=ids.map(id=>`<a class="citation" href="#source-${esc(id)}">${esc(records[id].file)}:${records[id].startLine}–${records[id].endLine}</a>`).join(' ');if(ids.length){history.replaceState(null,'','#source-'+ids[0]);reveal('source-'+ids[0]);}return;}const href=anchor.getAttribute('href');if(href.startsWith('#')){event.preventDefault();history.pushState(null,'',href);reveal(decodeURIComponent(href.slice(1)));}});
document.addEventListener('toggle',renderSequence,true);window.addEventListener('resize',renderSequence);window.addEventListener('hashchange',()=>reveal(decodeURIComponent(location.hash.slice(1))));
if(typeof ResizeObserver==='function')new ResizeObserver(renderSequence).observe(document.getElementById('sequence-host'));
// Defer until shared sequence declarations have initialized.
queueMicrotask(()=>{renderSequence();if(location.hash)reveal(decodeURIComponent(location.hash.slice(1)));});
