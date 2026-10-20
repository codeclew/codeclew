// Opt-in editorial reading profile. All original publication records stay in D.
function questionOriginalLanguageNotice(){
 const gap=D.questionReaderTranslationGap;if(!gap)return '';
 const available=gap.availableLanguage==='en'?t('English'):gap.availableLanguage==='ru'?t('Russian'):t('Unknown language');
 return chromeHtml`<p class="question-original-language" role="note">${esc(t('Original reading guide'))} · ${esc(available)} · ${esc(t('Translation pending'))}</p>`;
}
function questionAnchor(id){return 'question-'+id;}
function questionText(en,ru){return D.language==='ru'?ru:en;}
function questionDestinations(){const rows=(D.questionReader?.sections||[]).flatMap(section=>[{...section,title:section.navTitle||section.title},...section.blocks.filter(block=>block.id).map(block=>({...block,title:block.navTitle||block.title,exception:true}))]);if(relatedTopicInteractions().length){const at=rows.findIndex(row=>row.id==='confidence');rows.splice(at<0?rows.length:at,0,{id:'async-continuations',title:questionText('Related asynchronous continuations','Связанные асинхронные продолжения'),blocks:[]});}return rows;}
function questionSection(id){return questionDestinations().find(section=>questionAnchor(section.id)===id);}
function questionNav(){
 const guide=D.questionReader;if(!guide)return false;if(guide.presentation==='SEQUENTIAL')return sequentialNav();
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
 if(!source.text&&/^https:\/\//.test(source.url||'')){const base=source.url.split('#')[0],url=base+'#L'+start+(base.includes('/-/blob/')?'-':'-L')+end;return chromeHtml`<p class="question-code"><a href="${esc(url)}" target="_blank" rel="noopener noreferrer">${esc(source.service)} · ${esc(source.file)}:${esc(start)}–${esc(end)} ↗</a> · <code>${esc(source.revision)}</code></p>`;}
 const lines=(source.text||'').split('\n');
 const text=lines.slice(Math.max(0,start-source.startLine),end-source.startLine+1).join('\n');
 return chromeHtml`<details class="question-code"><summary>${esc(source.service)} · ${esc(source.file)}:${esc(start)}–${esc(end)} · ${esc((source.revision||'').slice(0,12))}</summary><pre tabindex="0" aria-label="Exact retained source fragment">${esc(text)}</pre><details><summary>${esc(questionText('Exact source binding','Точная привязка к исходнику'))}</summary><pre>${pretty({sourceId:source.id,revision:source.revision,textDigest:source.textDigest,evidenceDigest:source.evidenceDigest,authority:source.authority,range:{startLine:start,endLine:end},occurrence:source.occurrence})}</pre></details></details>`;
}
function questionEvidence(block){
 return chromeHtml`<details class="question-evidence"><summary>${esc(questionText('Evidence and scope','Исходники и границы'))}</summary><p class="question-editorial">${esc(questionText('This short reading text is editorial guidance. Original wording, source and review state follow.','Это редакционное изложение. Ниже сохранены исходный текст, его источники и статус проверки.'))}</p>${block.evidence.map(e=>e.kind==='RETAINED_CLAIM'?chromeHtml`<article><p><b>${esc(e.title)}</b> · ${esc(statusLabel(e.state?.freshness||'UNVERIFIED'))} · ${esc(meaningReviewLabel(e.state?.verification))}</p><blockquote>${esc(e.fragment.text)}</blockquote>${Object.values(e.sources||{}).map(s=>questionSource(s)).join('')}<details><summary>${esc(questionText('Retained claim and context','Сохранённое утверждение и его контекст'))}</summary><pre>${pretty({subject:e.subject,operation:e.operation,fragment:e.fragment,state:e.state,sourceContext:e.sourceContext})}</pre></details></article>`:chromeHtml`<article><p>${esc(questionText('Selected saved source · not runtime proof','Конфигурация из выбранного снимка · не подтверждает runtime'))}</p>${questionSource(e.source,e.startLine,e.endLine)}</article>`).join('')}</details>`;
}
function questionLink(link,section){
 let href=link.href;
 if(link.subject.startsWith('service:')&&link.operation==='section-entities'){
  const [path,hash]=href.split('#');href=path+'?fromProcess='+encodeURIComponent(D.subject.slice(9))+'&fromQuestion='+encodeURIComponent(section)+(D.questionReader?.language==='ru'?'&readerLanguage=ru':'')+'#'+hash;
  const models=D.canonicalEntityLinks?.[questionAnchor(section)]?.[link.subject.slice(8)];
  if(models?.length){
   const field=new URLSearchParams(location.search||'').get('field');
   const query='?fromProcess='+encodeURIComponent(D.subject.slice(9))+'&fromAnchor='+encodeURIComponent(questionAnchor(section))+(field?'&fromField='+encodeURIComponent(field):'');
   return chromeHtml`<span class="question-model-links">${models.map(model=>chromeHtml`<a data-dm-context data-dm-return-anchor="${esc(questionAnchor(section))}" href="${esc(model.href+query)}">${esc(model.title)} · ${esc(questionText('data model','модель данных'))}</a>`).join(' · ')} <a class="question-technical-link" href="${esc(href)}" title="${esc(link.label)}">${esc(questionText('Technical context','Технический контекст'))}</a></span>`;
  }
 }
 return chromeHtml`<a href="${esc(href)}">${esc(link.label)}</a>`;
}
function questionModelReferences(anchor){
 const links=(D.modelReferences||[]).filter(link=>link.anchor===anchor);
 if(!links.length)return '';
 const query='?fromProcess='+encodeURIComponent(D.subject.slice(9))+'&fromAnchor='+encodeURIComponent(anchor);
 return chromeHtml`<p class="question-context-links">${links.map(link=>chromeHtml`<a data-dm-context data-dm-return-anchor="${esc(anchor)}" href="${esc(link.href+query)}">${esc(link.title)}</a>`).join(' · ')}</p>`;
}
function questionBlock(block,section,index,story=false){
 return chromeHtml`<article class="question-block ${story?'question-story-row':''}">${story?chromeHtml`<span class="question-step-number" aria-hidden="true">${index+1}</span>`:''}<div><h3>${esc(block.title)}</h3><p>${esc(block.text)}</p>${block.links?.length?chromeHtml`<p class="question-context-links">${block.links.map(link=>questionLink(link,section)).join(' · ')}</p>`:''}${questionEvidence(block)}</div></article>`;
}
function showQuestionReader(){
 const guide=D.questionReader;if(guide.presentation==='SEQUENTIAL'){showSequentialReader();return;}questionStatus(D.operationStates?.['process-overview']||D.sectionState);
 const selector=chromeHtml`<label class="question-mobile-nav">Read a question<select id="question-select" aria-label="Read a question">${guide.sections.map(s=>chromeHtml`<option value="${esc(questionAnchor(s.id))}">${esc(s.title)}</option>`).join('')}</select></label>`;
 const sections=guide.sections.map((s,index)=>chromeHtml`${s.id==='confidence'?relatedTopicContinuations():''}<section id="${esc(questionAnchor(s.id))}" tabindex="-1" class="question-section"><h2>${esc(s.title)}</h2>${questionOriginalLanguageNotice()}${s.blocks.map((b,n)=>questionBlock(b,s.id,n,index===0)+(n===0?processTopicSection(s.id):'')).join('')}${index===0?chromeHtml`<aside class="question-local-result"><h3>${esc(guide.conclusion.title)}</h3><p>${esc(guide.conclusion.text)}</p>${questionEvidence(guide.conclusion)}</aside>`:''}${s.id==='confidence'?questionOriginals():''}</section>`).join('');
 $('scenario-content').innerHTML=chromeHtml`<div class="question-reader"><header class="question-heading"><h1>${esc(guide.title)}</h1><p>${esc(guide.purpose)}</p></header>${selector}${sections}<p class="question-end">End of this bounded account. Separate continuation views keep their own evidence and unknowns.</p></div>`;
 $('scenario-content').querySelector?.('#question-select')?.addEventListener('change',event=>{const hash='#'+event.target.value;history.pushState(null,'',hash);revealQuestion(event.target.value);});
 nav();questionObserve();document.title=guide.title+' · Codeclew';
}
function questionOriginals(){
 return chromeHtml`<details class="question-originals"><summary>${esc(questionText('Full retained descriptions, notes and original views','Полные исходные описания, Notes и представления'))}</summary><p>${esc(questionText('These original accounts retain their independent source scopes and statuses. They have not been rewritten or reaccepted by this reading guide.','Исходные описания сохраняют собственную область источников и статусы. Это изложение не переписывает их и не означает повторного одобрения.'))}</p>${D.operations.filter(op=>op.id!=='process-overview').map(op=>chromeHtml`<p><a href="#${esc(op.id)}">${esc(op.title)}</a></p>`).join('')}<details><summary>${esc(questionText('Original overview and its evidence','Исходный обзор и его доказательства'))}</summary>${D.operations.filter(op=>op.id==='process-overview').map(op=>chromeHtml`<p>${esc(op.summary.text)}</p><pre>${pretty(op)}</pre>`).join('')}</details>${D.processThread?chromeHtml`<details><summary>${esc(questionText('Selected controller methods and declared wrapper steps','Выбранные методы контроллеров и объявленные шаги'))}</summary>${processThreadCard(D.processThread)}</details>`:''}${(D.notes||[]).map(note=>chromeHtml`<details><summary>${esc(questionText('Retained note','Сохранённая заметка'))}</summary><pre>${pretty(note)}</pre></details>`).join('')}<details><summary>${esc(questionText('Full publication/source context and freshness','Полный контекст публикации, исходников и актуальности'))}</summary><pre>${pretty({publication:D.frozenPublication,selectedSnapshot:D.selectedSnapshot,selectedEvidence:D.selectedEvidence,revisions:D.revisions,sourceAuthorities:D.sourceAuthorities,operationStates:D.operationStates,sectionState:D.sectionState,process:D.process,boundaries:D.boundaries,questionReaderAuthority:D.questionReader.authority})}</pre></details></details>`;
}
function revealQuestion(id){
 const section=questionSection(id);if(!section)return false;
 if(!$('scenario-content').querySelector?.('.question-reader')||$('scenario-content').hidden||!$('coverage-view').hidden||!$('catalogue-view').hidden)showEntry('process-overview');
 const target=$(id);if(!target)return false;questionSelectedAnchor=id;
 let ancestor=target.parentElement;while(ancestor){if(ancestor.tagName==='DETAILS')ancestor.open=true;ancestor=ancestor.parentElement;}
 if(D.questionReader?.presentation==='SEQUENTIAL'&&id==='question-confidence'){const detail=target.querySelector?.('.sequential-provenance');if(detail)detail.open=true;}
 if('scrollRestoration' in history)history.scrollRestoration='manual';
 target.scrollIntoView({block:'start',behavior:'instant'});target.focus({preventScroll:true});questionActive(id);return true;
}
function questionActive(id){
 document.querySelectorAll('.question-link').forEach(link=>{if(link.getAttribute('href')==='#'+id)link.setAttribute('aria-current','location');else link.removeAttribute('aria-current');});
 const select=$('question-select');if(select)select.value=id;
}
var questionObserver,questionLocationPending,questionSelectedAnchor;
function questionUpdateLocation(){
 if(!D.questionReader||current?.id!=='process-overview'||$('scenario-content').hidden)return;
 if(questionSelectedAnchor&&questionSection(questionSelectedAnchor)){questionActive(questionSelectedAnchor);return;}
 const header=document.querySelector('.reader-nav')?.getBoundingClientRect?.();
 const menu=document.querySelector('.question-mobile-nav')?.getBoundingClientRect?.();
 const readingLine=Math.max(0,header?.bottom||0,menu?.bottom||0)+24;
 const sections=questionDestinations().map(section=>$(questionAnchor(section.id))).filter(element=>element?.getBoundingClientRect).map(element=>({element,rect:element.getBoundingClientRect()})).filter(row=>row.rect.height>0);
 // Recompute every section: an observer callback contains changed entries only.
 // Expanding evidence can move a long section's heading far above the viewport.
 const selected=[...sections].reverse().find(row=>row.rect.top<=readingLine&&row.rect.bottom>readingLine)||sections.find(row=>row.rect.top>readingLine)||sections.at(-1);
 if(selected)questionActive(selected.element.id);
}
function questionScheduleLocation(){
 if(questionLocationPending)return;
 questionLocationPending=true;
 const update=()=>{questionLocationPending=false;questionUpdateLocation();};
 if(typeof requestAnimationFrame==='function')requestAnimationFrame(update);else update();
}
function questionObserve(){
 questionObserver?.disconnect();
 if(typeof IntersectionObserver!=='undefined'){
  questionObserver=new IntersectionObserver(questionScheduleLocation,{rootMargin:'-100px 0px -55% 0px'});
  document.querySelectorAll('.question-section,.sequential-callout').forEach(section=>questionObserver.observe(section));
 }
 questionScheduleLocation();
}
function questionReleaseLocation(){questionSelectedAnchor=null;questionScheduleLocation();}
window.addEventListener('wheel',questionReleaseLocation,{passive:true});
window.addEventListener('touchmove',questionReleaseLocation,{passive:true});
window.addEventListener('keydown',event=>{if(['ArrowDown','ArrowUp','PageDown','PageUp','Home','End',' '].includes(event.key)&&!event.target?.matches?.('input,textarea,select,button'))questionReleaseLocation();});
window.addEventListener('pointerdown',event=>{if(event.clientX>=document.documentElement.clientWidth-16)questionReleaseLocation();},{passive:true});
window.addEventListener('scroll',questionScheduleLocation,{passive:true});
window.addEventListener('resize',questionScheduleLocation);
document.addEventListener('toggle',questionScheduleLocation,true);
function questionServiceReturn(){
 if(typeof URLSearchParams==='undefined')return;
 const params=new URLSearchParams(location.search||''),process=params.get('fromProcess'),question=params.get('fromQuestion');
 if(!/^[a-zA-Z0-9_-]{1,160}$/.test(process||'')||!/^[a-zA-Z0-9_-]{1,80}$/.test(question||''))return;
 if(!$('scenario-content').querySelector?.('.question-return'))$('scenario-content').insertAdjacentHTML('afterbegin',chromeHtml`<p class="question-return"><a href="../scenarios/${esc(process)}.html#question-${esc(question)}">${params.get('readerLanguage')==='ru'?'Вернуться к шагу процесса':'Return to process step'}</a></p>`);
}
function questionHistoryAlign(){
 const id=location.hash.slice(1);
 if(!questionSection(id))return;
 if(typeof requestAnimationFrame==='function')requestAnimationFrame(()=>revealQuestion(id));else revealQuestion(id);
}
window.addEventListener('pageshow',questionHistoryAlign);window.addEventListener('popstate',questionHistoryAlign);

// Sequential editorial presentation of the same retained evidence contract.
function sequentialIcon(name){return name&&QUESTION_ICONS[name]?chromeHtml`<span class="sequential-icon" aria-hidden="true">${QUESTION_ICONS[name]}</span>`:'';}
function sequentialNavigationRows(){return questionDestinations().map(destination=>({id:destination.id,title:destination.title,exception:destination.exception,reference:!destination.number&&!destination.exception}));}
function sequentialNav(){
 const guide=D.questionReader,reading=current?.id==='process-overview';
 document.body.classList.add('question-profile','sequential-profile');document.body.classList.toggle('sequential-reading',reading);
 document.documentElement.lang=reading?guide.language:(D.language||'en');
 $('freshness-status').hidden=reading;
 $('project-title').textContent=questionText('On this page','На этой странице');$('project-subtitle').hidden=true;$('project-kind').hidden=true;$('revision-links').hidden=true;$('catalogue-button').hidden=true;
 const rows=sequentialNavigationRows(),steps=rows.filter(row=>!row.exception&&!row.reference),exceptions=rows.filter(row=>row.exception),reference=rows.filter(row=>row.reference);
 const item=row=>chromeHtml`<a class="question-link" href="#${esc(questionAnchor(row.id))}">${esc(row.title)}</a>`;
 const entitySection=guide.sections.find(section=>section.blocks.some(block=>block.links?.some(link=>link.operation==='section-entities')));
 const entity=entitySection?.blocks.flatMap(block=>block.links||[]).find(link=>link.operation==='section-entities');
 const canonicalEntity=entity&&D.canonicalEntityLinks?.[questionAnchor(entitySection.id)]?.[entity.subject.slice(8)]?.length;
 $('scenario-nav').innerHTML=steps.map(item).join('')+(exceptions.length?chromeHtml`<div class="sequential-nav-group"><p>${esc(questionText('Exceptions','Исключения'))}</p>${exceptions.map(item).join('')}</div>`:'')+(entity?chromeHtml`<div class="sequential-nav-group sequential-entity">${questionLink(entity,entitySection.id)}<small>${esc(canonicalEntity?questionText('Data model and technical context','Модель данных и технический контекст'):questionText('Fields and service context','Поля и контекст сервиса'))}</small></div>`:'')+chromeHtml`<div class="sequential-nav-group">${reference.map(item).join('')}<small>${esc(questionText('Saved source · review required','Сохранённые исходники · требуется проверка'))}<small>${esc(statusLabel(D.operationStates?.['process-overview']?.freshness||'UNVERIFIED'))} · ${esc(D.operationStates?.['process-overview']?.verification||'UNASSESSED')}</small></div>`;
 if(!reading&&!$('scenario-content').querySelector?.('.question-return'))$('scenario-content').insertAdjacentHTML('afterbegin',chromeHtml`<p class="question-return"><a href="#question-confidence">${esc(questionText('Return to the process and sources','Вернуться к процессу и источникам'))}</a></p>`);
 return true;
}
function sequentialText(text){return String(text).split(/(`[^`\n]+`)/g).map(part=>part.startsWith('`')&&part.endsWith('`')?chromeHtml`<code>${esc(part.slice(1,-1))}</code>`:esc(part)).join('');}
function sequentialBlock(block,section){
 const links=block.links?.length?chromeHtml` <span class="sequential-inline-links">${block.links.map(link=>questionLink(link,section)).join(' · ')}</span>`:'';
 const content=chromeHtml`<p>${sequentialText(block.text)}${links}</p>`;
 if(block.disclosure)return chromeHtml`<details class="sequential-contract"><summary>${esc(block.disclosure)}${block.preview?chromeHtml`<span>${esc(block.preview)}</span>`:''}</summary>${content}${questionEvidence(block)}</details>`;
 if(block.tone&&block.tone!=='NORMAL')return chromeHtml`<aside${block.id?chromeHtml` id="${esc(questionAnchor(block.id))}" tabindex="-1"`:''} class="sequential-callout sequential-${esc(block.tone.toLowerCase())}"><h3>${esc(block.title)}</h3>${content}</aside>`;
 return content;
}
function showSequentialReader(){
 const guide=D.questionReader,steps=guide.sections.filter(section=>section.number),reference=guide.sections.find(section=>!section.number),state=D.operationStates?.['process-overview']||D.sectionState;
 const rows=sequentialNavigationRows();
 const selector=chromeHtml`<label class="question-mobile-nav">${esc(questionText('On this page','На этой странице'))}<select id="question-select" aria-label="${esc(questionText('On this page','На этой странице'))}">${rows.map(row=>chromeHtml`<option value="${esc(questionAnchor(row.id))}">${esc(row.title)}</option>`).join('')}</select></label>`;
 const content=steps.map(section=>chromeHtml`<section id="${esc(questionAnchor(section.id))}" tabindex="-1" class="question-section sequential-step"><span class="sequential-number" aria-hidden="true">${esc(section.number)}.</span><div><h2>${esc(section.title)}${sequentialIcon(section.icon)}</h2>${questionOriginalLanguageNotice()}${section.blocks.map((block,index)=>sequentialBlock(block,section.id)+(index===0?questionModelReferences(questionAnchor(section.id))+processTopicSection(section.id):'')+(index===0&&section.route?chromeHtml`<p class="sequential-route"><code>${esc(section.route)}</code></p>`:'')).join('')}</div></section>`).join('');
 const sources=chromeHtml`<section id="${esc(questionAnchor(reference.id))}" tabindex="-1" class="question-section sequential-source-section"><details class="sequential-provenance"><summary>${esc(reference.title)}</summary>${questionOriginalLanguageNotice()}<p>${esc(questionText('Editorial reading guide over retained source evidence. Original wording remains unchanged.','Редакционный план чтения по сохранённым исходным данным. Исходные формулировки не изменены.'))}</p><p>${esc(statusLabel(state?.freshness||'UNVERIFIED'))} · ${esc(meaningReviewLabel(state?.verification))}. ${esc(questionText('This is source interpretation, not an observed distributed run.','Это интерпретация исходников, а не наблюдение распределённого выполнения.'))}</p>${reference.blocks.map(block=>sequentialBlock(block,reference.id)).join('')}${steps.map(section=>chromeHtml`<details class="sequential-step-evidence"><summary>${esc(section.number)}. ${esc(section.title)}</summary>${section.blocks.map(block=>questionEvidence(block)).join('')}</details>`).join('')}${questionOriginals()}<details><summary>${esc(questionText('Icon licenses','Лицензии значков'))}</summary><pre>${esc(QUESTION_ICONS.license)}</pre></details></details></section>`;
 $('scenario-content').innerHTML=chromeHtml`<div class="question-reader sequential-reader"><p class="sequential-breadcrumb"><a href="../catalog.html">${esc(questionText('Processes','Процессы'))}</a><span> / </span>${esc(guide.title)}</p><header class="question-heading"><h1>${esc(guide.title)}</h1><p>${esc(guide.purpose)}</p></header>${selector}${content}${relatedTopicContinuations()}${sources}</div>`;
 $('scenario-content').querySelector?.('#question-select')?.addEventListener('change',event=>{history.pushState(null,'','#'+event.target.value);revealQuestion(event.target.value);});
 nav();questionObserve();document.title=guide.title+' · Codeclew';
}
