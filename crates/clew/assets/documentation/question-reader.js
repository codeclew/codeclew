// Opt-in editorial reading profile. All original publication records stay in D.
function questionAnchor(id){return 'question-'+id;}
function questionSection(id){return D.questionReader?.sections.find(section=>questionAnchor(section.id)===id);}
function questionNav(){
 const guide=D.questionReader;if(!guide)return false;
 document.body.classList.add('question-profile');
 $('project-title').textContent=guide.title;$('project-subtitle').textContent='A bounded source-based account';
 $('revision-links').hidden=true;$('catalogue-button').hidden=true;
 const query=$('search').value.toLowerCase().trim();
 $('scenario-nav').innerHTML=guide.sections.filter(s=>(s.title+' '+s.blocks.map(b=>b.title+' '+b.text).join(' ')).toLowerCase().includes(query)).map(s=>chromeHtml`<a class="scenario-link question-link" href="#${esc(questionAnchor(s.id))}">${esc(s.title)}</a>`).join('');
 if(current?.id!=='process-overview'&&!$('scenario-content').querySelector?.('.question-return'))$('scenario-content').insertAdjacentHTML('afterbegin',chromeHtml`<p class="question-return"><a href="#question-confidence">Back to the questions and source context</a></p>`);
 return true;
}
function questionStatus(state){
 $('freshness-status').innerHTML=chromeHtml`<div class="question-status"><span><b>${esc(statusLabel(state?.freshness||'UNVERIFIED'))}</b> · Meaning review: ${esc(meaningReviewLabel(state?.verification))}</span><a href="#question-confidence">Source and confidence</a><small>Retained explanations keep their review needs; this view does not establish a completed distributed run.</small></div>`;
}
function questionSource(source,start=source.startLine,end=source.endLine){
 const lines=(source.text||'').split('\n');
 const text=lines.slice(Math.max(0,start-source.startLine),end-source.startLine+1).join('\n');
 return chromeHtml`<details class="question-code"><summary>${esc(source.service)} · ${esc(source.file)}:${esc(start)}–${esc(end)} · ${esc((source.revision||'').slice(0,12))}</summary><pre tabindex="0" aria-label="Exact retained source fragment">${esc(text)}</pre><details><summary>Exact source binding</summary><pre>${pretty({sourceId:source.id,revision:source.revision,textDigest:source.textDigest,evidenceDigest:source.evidenceDigest,authority:source.authority,range:{startLine:start,endLine:end},occurrence:source.occurrence})}</pre></details></details>`;
}
function questionEvidence(block){
 return chromeHtml`<details class="question-evidence"><summary>Evidence and scope</summary><p class="question-editorial">This short reading text is editorial guidance. Original wording, source and review state follow.</p>${block.evidence.map(e=>e.kind==='RETAINED_CLAIM'?chromeHtml`<article><p><b>${esc(e.title)}</b> · ${esc(statusLabel(e.state?.freshness||'UNVERIFIED'))} · ${esc(meaningReviewLabel(e.state?.verification))}</p><blockquote>${esc(e.fragment.text)}</blockquote>${Object.values(e.sources||{}).map(s=>questionSource(s)).join('')}<details><summary>Retained claim and context</summary><pre>${pretty({subject:e.subject,operation:e.operation,fragment:e.fragment,state:e.state,sourceContext:e.sourceContext})}</pre></details></article>`:chromeHtml`<article><p>Selected saved source configuration · not runtime proof</p>${questionSource(e.source,e.startLine,e.endLine)}</article>`).join('')}</details>`;
}
function questionLink(link,section){
 let href=link.href;
 if(link.subject.startsWith('service:')&&link.operation==='section-entities'){
  const [path,hash]=href.split('#');href=path+'?fromProcess='+encodeURIComponent(D.subject.slice(9))+'&fromQuestion='+encodeURIComponent(section)+'#'+hash;
 }
 return chromeHtml`<a href="${esc(href)}">${esc(link.label)}</a>`;
}
function questionBlock(block,section,index,story=false){
 return chromeHtml`<article class="question-block ${story?'question-story-row':''}">${story?chromeHtml`<span class="question-step-number" aria-hidden="true">${index+1}</span>`:''}<div><h3>${esc(block.title)}</h3><p>${esc(block.text)}</p>${block.links?.length?chromeHtml`<p class="question-context-links">${block.links.map(link=>questionLink(link,section)).join(' · ')}</p>`:''}${questionEvidence(block)}</div></article>`;
}
function showQuestionReader(){
 const guide=D.questionReader;questionStatus(D.operationStates?.['process-overview']||D.sectionState);
 const selector=chromeHtml`<label class="question-mobile-nav">Read a question<select id="question-select" aria-label="Read a question">${guide.sections.map(s=>chromeHtml`<option value="${esc(questionAnchor(s.id))}">${esc(s.title)}</option>`).join('')}</select></label>`;
 const sections=guide.sections.map((s,index)=>chromeHtml`<section id="${esc(questionAnchor(s.id))}" tabindex="-1" class="question-section"><h2>${esc(s.title)}</h2>${s.blocks.map((b,n)=>questionBlock(b,s.id,n,index===0)).join('')}${index===0?chromeHtml`<aside class="question-local-result"><h3>${esc(guide.conclusion.title)}</h3><p>${esc(guide.conclusion.text)}</p>${questionEvidence(guide.conclusion)}</aside>`:''}${s.id==='confidence'?questionOriginals():''}</section>`).join('');
 $('scenario-content').innerHTML=chromeHtml`<div class="question-reader"><header class="question-heading"><h1>${esc(guide.title)}</h1><p>${esc(guide.purpose)}</p></header>${selector}${sections}<p class="question-end">End of this bounded account. Separate continuation views keep their own evidence and unknowns.</p></div>`;
 $('scenario-content').querySelector?.('#question-select')?.addEventListener('change',event=>{const hash='#'+event.target.value;history.pushState(null,'',hash);revealQuestion(event.target.value);});
 nav();questionObserve();document.title=guide.title+' · Codeclew';
}
function questionOriginals(){
 return chromeHtml`<details class="question-originals"><summary>Full retained descriptions, notes and original views</summary><p>These original accounts retain their independent source scopes and statuses. They have not been rewritten or reaccepted by this reading guide.</p>${D.operations.filter(op=>op.id!=='process-overview').map(op=>chromeHtml`<p><a href="#${esc(op.id)}">${esc(op.title)}</a></p>`).join('')}<details><summary>Original overview and its evidence</summary>${D.operations.filter(op=>op.id==='process-overview').map(op=>chromeHtml`<p>${esc(op.summary.text)}</p><pre>${pretty(op)}</pre>`).join('')}</details>${D.processThread?chromeHtml`<details><summary>Selected controller methods and declared wrapper steps</summary>${processThreadCard(D.processThread)}</details>`:''}${(D.notes||[]).map(note=>chromeHtml`<details><summary>Retained note</summary><pre>${pretty(note)}</pre></details>`).join('')}<details><summary>Full publication/source context and freshness</summary><pre>${pretty({publication:D.frozenPublication,selectedSnapshot:D.selectedSnapshot,selectedEvidence:D.selectedEvidence,revisions:D.revisions,sourceAuthorities:D.sourceAuthorities,operationStates:D.operationStates,sectionState:D.sectionState,process:D.process,boundaries:D.boundaries,questionReaderAuthority:D.questionReader.authority})}</pre></details></details>`;
}
function revealQuestion(id){
 const section=questionSection(id);if(!section)return false;
 if(!$('scenario-content').querySelector?.('.question-reader')||$('scenario-content').hidden||!$('coverage-view').hidden||!$('catalogue-view').hidden)showEntry('process-overview');
 const target=$(id);if(!target)return false;
 if('scrollRestoration' in history)history.scrollRestoration='manual';
 target.scrollIntoView({block:'start',behavior:'instant'});target.focus({preventScroll:true});questionActive(id);return true;
}
function questionActive(id){
 document.querySelectorAll('.question-link').forEach(link=>{if(link.getAttribute('href')==='#'+id)link.setAttribute('aria-current','location');else link.removeAttribute('aria-current');});
 const select=$('question-select');if(select)select.value=id;
}
var questionObserver;
function questionObserve(){
 questionObserver?.disconnect();
 if(typeof IntersectionObserver==='undefined')return;
 questionObserver=new IntersectionObserver(rows=>{const row=rows.filter(r=>r.isIntersecting).sort((a,b)=>a.boundingClientRect.top-b.boundingClientRect.top)[0];if(row)questionActive(row.target.id);},{rootMargin:'-100px 0px -55% 0px'});
 document.querySelectorAll('.question-section').forEach(section=>questionObserver.observe(section));
}
function questionServiceReturn(){
 if(typeof URLSearchParams==='undefined')return;
 const params=new URLSearchParams(location.search||''),process=params.get('fromProcess'),question=params.get('fromQuestion');
 if(!/^[a-zA-Z0-9_-]{1,160}$/.test(process||'')||!/^[a-zA-Z0-9_-]{1,80}$/.test(question||''))return;
 if(!$('scenario-content').querySelector?.('.question-return'))$('scenario-content').insertAdjacentHTML('afterbegin',chromeHtml`<p class="question-return"><a href="../scenarios/${esc(process)}.html#question-${esc(question)}">Return to the originating question</a></p>`);
}
function questionHistoryAlign(){
 const id=location.hash.slice(1);
 if(!questionSection(id))return;
 if(typeof requestAnimationFrame==='function')requestAnimationFrame(()=>revealQuestion(id));else revealQuestion(id);
}
window.addEventListener('pageshow',questionHistoryAlign);window.addEventListener('popstate',questionHistoryAlign);
