(()=>{
 const ru=document.documentElement.lang==='ru',label=(en,ruText)=>ru?ruText:en;
 const header=document.querySelector('.reader-nav');
 if(header){const measure=()=>document.documentElement.style.setProperty('--reader-header-height',`${header.getBoundingClientRect().height}px`);measure();if(typeof ResizeObserver==='function')new ResizeObserver(measure).observe(header);}
 const main=document.querySelector('main.reader-guide');
 if(main&&!main.classList.contains('catalog-page')){
  const headings=[...main.querySelectorAll('h2')];
  if(headings.length>2){
   const layout=document.createElement('div');layout.className='guide-layout';main.before(layout);
   const nav=document.createElement('nav');nav.className='guide-toc';nav.setAttribute('aria-label',label('On this page','На этой странице'));
   const contents=document.createElement('details');contents.className='guide-local-navigation';contents.open=!window.matchMedia('(max-width:760px)').matches;
   const title=document.createElement('summary');title.textContent=label('On this page','На этой странице');contents.append(title);
   headings.forEach((h,i)=>{h.id=h.id||`guide-section-${i+1}`;const a=document.createElement('a');a.href=`#${h.id}`;a.textContent=h.textContent;a.addEventListener('click',()=>{if(window.matchMedia('(max-width:760px)').matches)contents.open=false;setTimeout(()=>{h.tabIndex=-1;h.scrollIntoView({block:'start'});h.focus({preventScroll:true});},0);});contents.append(a);});
   const active=()=>contents.querySelectorAll('a').forEach(a=>{if(a.getAttribute('href')===location.hash)a.setAttribute('aria-current','location');else a.removeAttribute('aria-current');});window.addEventListener('hashchange',active);active();nav.append(contents);layout.append(nav,main);
  }
 }
 // Keep the exact title ahead of status; long descriptions follow the status.
 // Moving nodes preserves text, source controls and delegated event handlers.
 const live=document.getElementById('scenario-content'),lead=document.getElementById('reader-lead');
 let topicContextApplied=false;
 function focusTopicContext(){
  if(topicContextApplied)return;
  const params=new URLSearchParams(location.search||''),topic=params.get('topic'),direction=params.get('direction');
  if(!topic||!['incoming','outgoing'].includes(direction))return;
  const target=document.getElementById(`topic-${direction}-${topic}`);if(!target)return;
  topicContextApplied=true;const focus=()=>{target.scrollIntoView({block:'start'});target.focus({preventScroll:true});};if(typeof requestAnimationFrame==='function')requestAnimationFrame(focus);else focus();
 }
 function alignLead(){
  if(!live||!lead)return;
  if(live.hidden||live.querySelector(':scope > .question-reader')){lead.replaceChildren();return;}
  const children=[...live.children],index=children.findIndex(child=>child.classList.contains('breadcrumb'));
  if(index<0||children.slice(0,index).some(child=>!child.matches('.question-return')))return;
  const first=children[index];
  const nodes=[];
  for(const child of children){
   if(child===first||child.matches('.question-return,h2,.scenario-heading'))nodes.push(child);else break;
  }
  if(nodes.length<index+2)return;
  lead.replaceChildren(...nodes);
  const old=lead.querySelector('h2');
  if(old){const heading=document.createElement('h1');for(const attr of old.attributes)heading.setAttribute(attr.name,attr.value);heading.innerHTML=old.innerHTML;if(location.hash==='#section-overview')heading.textContent=first.textContent;old.replaceWith(heading);}
  focusTopicContext();
 }
 if(live&&lead){alignLead();new MutationObserver(alignLead).observe(live,{childList:true,attributes:true,attributeFilter:['hidden']});}
 const project=document.querySelector('.sidebar .project'),local=document.getElementById('reader-navigation');
 if(project&&local&&!document.body.classList.contains('question-profile')){
  const details=document.createElement('details'),summary=document.createElement('summary');details.className='reader-identity';summary.textContent=label('Page identity and revisions','Идентификатор страницы и ревизии');details.append(summary,project);local.append(details);
 }
 const data=document.getElementById('catalog-data');
 if(data){const rows=JSON.parse(data.textContent),q=document.getElementById('catalog-query'),kind=document.getElementById('catalog-kind'),list=document.getElementById('catalog-results'),status=document.getElementById('catalog-status'),prev=document.getElementById('catalog-prev'),next=document.getElementById('catalog-next');let page=0;
  function readState(){const params=new URLSearchParams(location.search);q.value=params.get('q')||'';kind.value=params.get('kind')||'';const requested=Number(params.get('page'));page=Number.isSafeInteger(requested)&&requested>0?requested-1:0;}
  function syncUrl(push=false){const params=new URLSearchParams(location.search);if(q.value.trim())params.set('q',q.value.trim());else params.delete('q');if(kind.value)params.set('kind',kind.value);else params.delete('kind');if(page>0)params.set('page',String(page+1));else params.delete('page');const search=params.toString(),url=`${location.pathname}${search?`?${search}`:''}${location.hash||''}`;if(push)history.pushState(history.state,'',url);else history.replaceState(history.state,'',url);}
  function render(){const query=q.value.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);const matched=rows.filter(r=>{const terms=Array.isArray(r.searchText)?r.searchText.join(' '):(r.searchText||'');const haystack=`${r.title} ${r.displayTitle||''} ${r.id} ${r.kind} ${r.context||''} ${r.summary||''} ${terms}`.toLocaleLowerCase();return(!kind.value||r.kind===kind.value)&&query.every(term=>haystack.includes(term));});const pages=Math.max(1,Math.ceil(matched.length/20));page=Math.max(0,Math.min(page,pages-1));list.replaceChildren();matched.slice(page*20,page*20+20).forEach(r=>{const li=document.createElement('li'),a=document.createElement('a'),meta=document.createElement('small');const displayTitle=typeof r.displayTitle==='string'&&r.displayTitle.trim()?r.displayTitle:null;a.href=r.href;a.textContent=displayTitle||r.title;const types={'Selected declarations':label('Selected declarations','Выбранные объявления'),'Selected question':label('Selected question','Выбранный вопрос'),'Diagnostic question':label('Diagnostic question','Диагностический вопрос'),'Examined callable':label('Examined callable','Просмотренный вызываемый код'),Endpoint:label('Endpoint','Точка входа'),Service:label('Service','Сервис'),Process:label('Process','Процесс'),Dataflow:label('Dataflow','Движение данных'),'Data model':label('Data model','Модель данных'),API:'API',Entrypoint:label('Entry point','Точка входа'),'Source declaration':label('Source declaration','Объявление в исходном коде'),Entity:label('Entity','Сущность')};const coverage={authored:label('Description present','Описание есть'),inventory:label('No description','Без описания'),unavailable:label('Evidence or translation gap','Есть пробелы')};meta.className='catalog-meta';meta.textContent=[types[r.kind]||r.kind,r.context,displayTitle?null:r.id,coverage[r.coverage]].filter(Boolean).join(' · ');li.append(a,meta);if(r.summary){const detail=document.createElement('small');detail.className='catalog-detail';detail.textContent=Array.isArray(r.summary)?r.summary.join(' · '):r.summary;li.append(detail);}if(Array.isArray(r.relatedLinks)&&r.relatedLinks.length){const related=document.createElement('small');related.className='catalog-detail';r.relatedLinks.forEach((entry,i)=>{const link=document.createElement('a');link.href=entry.href;link.textContent=entry.title;related.append(link);if(i<r.relatedLinks.length-1)related.append(document.createTextNode(' · '));});li.append(related);}if(displayTitle){const identity=document.createElement('details'),heading=document.createElement('summary');identity.className='catalog-identity';heading.textContent=label('Exact identity','Точный идентификатор');identity.append(heading);for(const [name,value] of [[label('Title','Заголовок'),r.title],[label('ID','Идентификатор'),r.id]]){const row=document.createElement('small'),code=document.createElement('code');row.append(document.createTextNode(`${name}: `));code.textContent=value;row.append(code);identity.append(row);}li.append(identity);}list.append(li);});status.textContent=matched.length?(ru?`Найдено: ${matched.length} · страница ${page+1} из ${pages}`:`${matched.length} results · page ${page+1} of ${pages}`):label('No matching results. Try another search or type.','Совпадений нет. Попробуйте другой запрос или тип.');prev.disabled=page===0;next.disabled=page>=pages-1;}
  q.addEventListener('input',()=>{page=0;render();syncUrl();});kind.addEventListener('change',()=>{page=0;render();syncUrl(true);});prev.addEventListener('click',()=>{page--;render();syncUrl(true);});next.addEventListener('click',()=>{page++;render();syncUrl(true);});
  if(typeof addEventListener==='function')addEventListener('popstate',()=>{readState();render();syncUrl();});
  readState();render();syncUrl();
  for(const id of ['catalog-processes']){const fallback=document.getElementById(id);if(fallback)fallback.hidden=true;}
  for(const id of ['catalog-controls','catalog-pager']){const controls=document.getElementById(id);if(controls)controls.hidden=false;}

 }
 const inCatalog=/\/(services|scenarios|data-model)\//.test(location.pathname)||/\/(catalog|data-model)\.html$/.test(location.pathname);
 document.querySelectorAll('.reader-nav [data-reader-destination]').forEach(a=>{if(new URL(a.href).pathname===location.pathname||(a.dataset.readerDestination==='catalog'&&inCatalog))a.setAttribute('aria-current','page');});
})();
