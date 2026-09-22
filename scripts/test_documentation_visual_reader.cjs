'use strict';
// Execute the shipped reader, not a parallel renderer. The minimal DOM captures
// its generated HTML and event dispatch; browser geometry is tested separately.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const script=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/app.js'),'utf8');
const fragment=text=>({id:text,text,dependencyIds:['dep'],sourceIds:['same-source']});
const source=text=>({service:'sample',file:'Worker.kt',startLine:1,endLine:1,text,revision:'abc',authority:'RETAINED_SOURCE_NOT_REVERIFIED'});
function fixture(){
 const flow={schema:1,generator:{id:'sample',version:'1'},id:'dispatch',kind:'execution-flow',title:'Dispatch work',purpose:fragment('Explain selected dispatch'),scope:fragment('The local handler only'),limitations:['Delivery is not observed.'],nodes:[{id:'start',meaning:fragment('Receive request')},{id:'select',meaning:fragment('Choose handler')},{id:'done',meaning:fragment('Return result')}],edges:[{id:'e1',from:'start',to:'select',meaning:fragment('After validation')},{id:'e2',from:'select',to:'done',meaning:fragment('When handler returns')}],parent:null,hitPolicy:null,rules:[],afterSelection:null};
 const decision={...flow,id:'rules',kind:'decision-table',title:'Select handler',parent:{artifact:'dispatch',node:'select'},nodes:[],edges:[],hitPolicy:'FIRST',policyExplanation:fragment('A return exits the inspected branch.'),rules:[{condition:fragment('The kind is supported'),outcome:fragment('Call the selected handler')}],afterSelection:fragment('Handler failure propagates separately.')};
 const op={id:'section-responsibilities',title:'Responsibilities',summary:fragment('Documented responsibilities'),participants:[],events:[],explanation:[],findings:[],boundaries:[],interfaceContracts:[],visuals:[flow,decision]};
 return {subject:'service:sample',title:'Sample',subtitle:'Example',catalogue:[],contracts:[],operations:[op],sections:[{id:'section-overview',title:'Overview',gap:'Awaiting overview'},{id:op.id,title:op.title,content:op}],notes:[],sources:{'same-source':source('CURRENT SOURCE')},operationSources:{[op.id]:{'same-source':source('ACCEPTED SOURCE')}},operationStates:{[op.id]:{freshness:'STALE',verification:'UNASSESSED'}},sectionState:{freshness:'CURRENT',verification:'VERIFIED'},sourceAuthorities:{},revisions:{sample:'abc'},boundaries:[],interactions:[],gaps:{},coverage:{},boundaryInventory:{publicBoundaries:[],gaps:[]}};
}
function load(data=fixture()){
 const elements=new Map(),listeners={};
 function element(id){if(!elements.has(id))elements.set(id,{id,value:'',hidden:false,innerHTML:'',textContent:'',isConnected:true,classList:{add(){},remove(){}},focus(){this.focused=true;},scrollIntoView(){this.scrolled=true;},insertAdjacentHTML(_,html){this.innerHTML=html+this.innerHTML;},addEventListener(){}});return elements.get(id);}
 element('document-data').textContent=JSON.stringify(data);
 const context=vm.createContext({document:{getElementById:element,addEventListener:(name,fn)=>listeners[name]=fn,querySelectorAll:()=>[],querySelector:()=>null,body:{classList:{add(){},remove(){}}},activeElement:null},location:{hash:''},history:{replaceState(){}},window:{addEventListener(){},scrollTo(){}},navigator:{clipboard:{writeText:async()=>{}}}});
 vm.runInContext(script,context);
 return {data,e:element,run:code=>vm.runInContext(code,context),click(dataset){listeners.click({target:{closest:()=>({dataset,hasAttribute:()=>false})},preventDefault(){}});}};
}
test('default service overview exposes the first native graph and all visual navigation',()=>{
 const r=load(),html=r.e('scenario-content').innerHTML;
 assert.match(html,/<svg class="artifact-svg"/);
 assert.match(html,/After validation/);
 assert.match(html,/Meaning review: <b>UNASSESSED/);
 assert.match(html,/Source freshness: <b>STALE/);
 assert.match(r.e('scenario-nav').innerHTML,/Dispatch work/);
 assert.match(r.e('scenario-nav').innerHTML,/Select handler/);
 assert.equal(r.run('inventoryEntries().length'),0);
});
test('decision and parent node links are reciprocal and preserve owner authority',()=>{
 const r=load(),key=r.run("visualKey('section-responsibilities','rules')");
 r.click({visualTarget:key});
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Choose the first matching row, top to bottom/);
 assert.match(html,/A return exits the inspected branch/);
 assert.match(html,/After selection: execution and outcomes/);
 assert.match(html,/Handler failure propagates separately/);
 assert.match(r.e('freshness-status').innerHTML,/<strong>STALE/);
 const parent=r.run("visualKey('section-responsibilities','dispatch')"),node=r.run("visualNodeKey('section-responsibilities','dispatch','select')");
 assert.ok(html.includes(`data-visual-target="${parent}"`));
 assert.ok(html.includes(`data-visual-node-target="${node}"`));
 r.click({visualTarget:parent,visualNodeTarget:node});
 assert.ok(r.e(node).focused&&r.e(node).scrolled);
 assert.ok(r.e('scenario-content').innerHTML.includes(`data-visual-target="${key}"`));
});
test('visual source buttons on overview use accepted owner sources, not current snapshot',()=>{
 const r=load();
 r.click({sources:'same-source',sourceOperation:'section-responsibilities'});
 assert.match(r.e('source-code').innerHTML,/ACCEPTED SOURCE/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/CURRENT SOURCE/);
});
test('unknown policy and unplaced decisions make uncertainty explicit',()=>{
 const data=fixture(),v=data.operations[0].visuals[1];v.parent=null;v.hitPolicy='UNKNOWN';
 delete data.operationStates['section-responsibilities'];
 const r=load(data);r.run("showEntry(visualKey('section-responsibilities','rules'))");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Local decision; placement in the wider process is not established/);
 assert.match(html,/Do not infer priority or exclusivity from row order/);
 assert.match(html,/Source freshness: <b>UNVERIFIED/);
 assert.match(r.e('freshness-status').innerHTML,/UNASSESSED/);
 assert.doesNotMatch(r.e('freshness-status').innerHTML,/<strong>CURRENT/);
});
test('artifact identifiers and text cannot inject markup or collide across owners',()=>{
 const r=load();
 assert.notEqual(r.run("visualKey('a--b','c')"),r.run("visualKey('a','b--c')"));
 assert.match(r.run("visualKey('<img onerror=x>','\" id=evil')"),/^visual-[0-9a-f_]+--[0-9a-f_]+$/);
 r.run(`D.operations[0].visuals[0].title='<img src=x onerror=alert(1)>';D.operations[0].visuals[0].nodes[0].meaning.text='<script>alert(1)</script>';showEntry('section-overview')`);
 const html=r.e('scenario-content').innerHTML;
 assert.doesNotMatch(html,/<img|<script>/);
 assert.match(html,/&lt;img/);
 assert.match(html,/&lt;script&gt;/);
});
test('dependency maps never imply execution order; cycles retain directed arrows',()=>{
 const data=fixture(),v=data.operations[0].visuals[0];v.kind='dependency-map';v.edges.push({id:'back',from:'done',to:'start',meaning:fragment('Depends on initial input')});
 const r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/Arrows show dependencies, not execution order/);
 assert.match(html,/Depends on initial input/);
 assert.doesNotMatch(html,/NaN|undefinedpx/);
 assert.equal((html.match(/class="artifact-edge"/g)||[]).length,3);
});
test('publications without visuals retain normal overview and operations',()=>{
 const data=fixture();delete data.operations[0].visuals;
 const r=load(data),html=r.e('scenario-content').innerHTML;
 assert.doesNotMatch(html,/artifact-svg|visual-preview/);
 assert.match(html,/Awaiting overview/);
 assert.match(html,/Explore internal processes/);
});
test('process catalog lists typed visuals without requiring an HTTP selection',()=>{
 const r=load();r.run("showEntry('process-catalog')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Processes and diagrams · 2/);
 assert.match(html,/<svg class="artifact-svg"/);
 assert.match(html,/<th>Selected result/);
});

test('missing accepted source map cannot fall back to unrelated current evidence',()=>{
 const data=fixture();delete data.operationSources['section-responsibilities'];
 const r=load(data);r.e('source-panel').hidden=true;
 r.click({sources:'same-source',sourceOperation:'section-responsibilities'});
 assert.equal(r.e('source-panel').hidden,true);
 assert.doesNotMatch(r.e('source-code').innerHTML,/CURRENT SOURCE/);
});
test('service overview surfaces accepted profile summaries before diagrams in reader-question order',()=>{
 const data=fixture();
 for(const [id,text] of [['section-entities','Tracks business concepts; ownership unknown'],['section-ingress','Accepts the selected request'],['section-egress','Requests processing from a partner']]){
  const owner={...data.operations[0],id,title:id,summary:fragment(text),visuals:[]};
  data.operations.push(owner);data.sections.push({id,title:id,content:owner});
  data.operationStates[id]={freshness:'CURRENT',verification:'UNASSESSED'};
  data.operationSources[id]={'same-source':source('SOURCE '+id)};
 }
 data.sections.reverse();
 const r=load(data),html=r.e('scenario-content').innerHTML;
 for(const text of ['Documented responsibilities','Tracks business concepts; ownership unknown','Accepts the selected request','Requests processing from a partner'])assert.ok(html.includes(text));
 const ids=['profile-section-responsibilities','profile-section-entities','profile-section-ingress','profile-section-egress','artifact-svg'];
 for(let i=1;i<ids.length;i++)assert.ok(html.indexOf(ids[i-1])<html.indexOf(ids[i]));
 assert.match(html,/DTOs and implementation classes alone do not establish entity ownership/);
 assert.match(html,/Per-transport completeness.*is not established here/);
 assert.ok(r.e('scenario-nav').innerHTML.indexOf('section-overview')<r.e('scenario-nav').innerHTML.indexOf('section-entities'));
 r.click({sources:'same-source',sourceOperation:'section-egress'});
 assert.match(r.e('source-code').innerHTML,/SOURCE section-egress/);
});
test('missing profile sections and discovered but unauthored entries remain explicit gaps',()=>{
 const data=fixture();data.boundaryInventory.publicBoundaries=[{id:'ingress',symbol:'handle',trigger:{methods:['POST'],paths:['/input']}}];
 data.catalogue=[{...data.boundaryInventory.publicBoundaries[0],kind:'HTTP_ENDPOINT'}];
 const r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/No source-bound section summary has been accepted/);
 assert.match(html,/POST \/input/);
 assert.match(html,/Behavior narrative not yet accepted/);
 assert.match(html,/Entity ownership and creation roles have no explicit domain declarations/);
 assert.doesNotMatch(html,/creates no entities|No outgoing calls|No Kafka|No cron/);
});

test('Russian locale translates reader chrome, policies and source controls while retaining authored text',async()=>{
 const data=fixture();data.language='ru';
 data.operations[0].visuals[0].nodes[0].meaning.text='Source /tasks/{taskType} OK_CODE';
 const r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/Задачи сервиса/);
 assert.match(html,/Предметные сущности/);
 assert.match(html,/Процессы и диаграммы/);
 assert.match(html,/Актуальность кода: <b>Устарело/);
 assert.match(html,/Проверка смысла: <b>Смысл не проверен/);
 assert.match(html,/Source \/tasks\/\{taskType\} OK_CODE/);
 assert.match(html,/Documented responsibilities/);
 assert.match(html,/Dispatch work/);
 assert.doesNotMatch(html,/Source freshness:|Meaning review:|Complete diagram as text/);
 assert.match(r.e('scenario-nav').innerHTML,/РАЗДЕЛ/);
 r.run("showEntry(visualKey('section-responsibilities','rules'))");
 const table=r.e('scenario-content').innerHTML;
 assert.match(table,/Строки проверяются сверху вниз/);
 assert.match(table,/Правило выбора строк: <b>FIRST/);
 assert.match(table,/<th>Условие<\/th><th>Выбранный результат/);
 assert.match(table,/После выбора: выполнение и результаты/);
 assert.match(table,/A return exits the inspected branch/);
 assert.match(table,/Handler failure propagates separately/);
 r.click({sources:'same-source',sourceOperation:'section-responsibilities'});
 assert.equal(r.e('source-title').textContent,'Подтверждающий код');
 assert.match(r.e('source-code').innerHTML,/ACCEPTED SOURCE/);
 assert.match(r.e('source-foot').innerHTML,/точный сохранённый фрагмент кода/);
 await r.e('copy-source').onclick();
 assert.equal(r.e('copy-status').textContent,'Исходный код скопирован');
});
test('Russian UNIQUE and UNKNOWN policies explain selection without translating rule identifiers',()=>{
 for(const [policy,expected] of [['UNIQUE',/не более одной строки/],['UNKNOWN',/Порядок строк не доказывает/]]){
  const data=fixture();data.language='ru';data.operations[0].visuals[1].hitPolicy=policy;data.operations[0].visuals[1].parent=null;
  const r=load(data);r.run("showEntry(visualKey('section-responsibilities','rules'))");
  assert.match(r.e('scenario-content').innerHTML,expected);
  assert.ok(r.e('scenario-content').innerHTML.includes(`<b>${policy}</b>`));
  assert.match(r.e('scenario-content').innerHTML,/Локальное решение; его место в общем процессе пока не установлено/);
 }
});
test('Russian catalogue, coverage and contract UI preserve paths, schema keys and authored descriptions',()=>{
 const data=fixture();data.language='ru';data.catalogue=[{id:'handler',symbol:'handleTask',kind:'HTTP_ENDPOINT',trigger:{methods:['POST'],paths:['/tasks/{taskType}']},sourceIds:[]}];
 const r=load(data);
 r.run('catalogue()');assert.match(r.e('catalogue-view').innerHTML,/точек входа/);assert.match(r.e('catalogue-view').innerHTML,/POST/);assert.match(r.e('catalogue-view').innerHTML,/\/tasks\/\{taskType\}/);
 r.run('coverage()');assert.match(r.e('coverage-view').innerHTML,/Что подтверждено исходным кодом/);
 const fields=r.run(`fields({type:'object',required:['task_id'],properties:{task_id:{type:'string',description:'Original description'},optionalKey:{type:'integer'}}})`);
 assert.match(fields,/<th>Поле<\/th><th>Тип/);assert.match(fields,/class="required">обязательно/);assert.match(fields,/class="optional">необязательно/);
 assert.match(fields,/task_id/);assert.match(fields,/optionalKey/);assert.match(fields,/Original description/);assert.match(fields,/string/);
});
test('explicit language gap suppresses foreign prose and visuals, keeps accepted model unchanged and offers original link',()=>{
 const data=fixture();data.language='ru';data.requestedDocumentationLanguage='ru';
 data.translationGaps={'section-responsibilities':{requestedLanguage:'ru',availableLanguage:'en',href:'../history/service.html'}};
 const original=JSON.stringify(data),r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/Перевод ещё не подготовлен/);assert.match(html,/Английская версия/);assert.match(html,/href="\.\.\/history\/service.html"/);
 assert.doesNotMatch(html,/Documented responsibilities|Dispatch work|Receive request|artifact-svg/);
 assert.doesNotMatch(r.e('scenario-nav').innerHTML,/Dispatch work|Select handler/);
 assert.equal(r.run('JSON.stringify(publication)'),original);
 assert.equal(r.e('document-data').textContent,original);
 r.run("showEntry('section-responsibilities')");
 assert.match(r.e('scenario-content').innerHTML,/Задачи сервиса/);assert.match(r.e('scenario-content').innerHTML,/Английская версия/);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/Documented responsibilities|Dispatch work/);
 r.run("showEntry('process-catalog')");assert.doesNotMatch(r.e('scenario-content').innerHTML,/Dispatch work|Select handler/);
});
test('unknown-language operation shows original-version gap; absent explicit request retains legacy content',()=>{
 const data=fixture();data.language='en';data.requestedDocumentationLanguage='en';
 data.translationGaps={'section-responsibilities':{requestedLanguage:'en',availableLanguage:null,href:'../history/original.html'}};
 let r=load(data);assert.match(r.e('scenario-content').innerHTML,/Original version/);assert.doesNotMatch(r.e('scenario-content').innerHTML,/Documented responsibilities/);
 delete data.requestedDocumentationLanguage;r=load(data);assert.match(r.e('scenario-content').innerHTML,/Documented responsibilities/);assert.doesNotMatch(r.e('scenario-content').innerHTML,/Translation pending/);
 data.language='fr';r=load(data);assert.match(r.e('scenario-content').innerHTML,/Source freshness/);
});
test('translation gap does not expose unsafe historical links or mismatched endpoint explanations',()=>{
 const data=fixture();data.language='ru';data.requestedDocumentationLanguage='ru';
 const op={...data.operations[0],id:'handler',title:'Foreign authored title',visuals:[]};data.operations.push(op);
 data.catalogue=[{id:'handler',symbol:'handleTask',kind:'HTTP_ENDPOINT',trigger:{methods:['POST'],paths:['/tasks/{taskType}']},sourceIds:[]}];
 data.translationGaps={handler:{requestedLanguage:'ru',availableLanguage:'en',href:'javascript:alert(1)'}};
 const r=load(data);r.run("showEntry('handler')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Перевод ещё не подготовлен/);assert.match(html,/\/tasks\/\{taskType\}/);
 assert.doesNotMatch(html,/javascript:|Foreign authored title|Documented responsibilities/);
});
test('overview translation gap preserves same-language sections and fallback operation navigation',()=>{
 const data=fixture();data.language='ru';data.requestedDocumentationLanguage='ru';
 const overview={...data.operations[0],id:'section-overview',title:'Foreign overview',summary:fragment('FOREIGN OVERVIEW'),visuals:[]};
 data.operations.push(overview);data.sections[0].content=overview;
 data.operations.push({...overview,id:'missing-operation',title:'FOREIGN OPERATION',summary:fragment('FOREIGN EXPLANATION')});
 data.translationGaps={'section-overview':{availableLanguage:'en',href:null},'missing-operation':{availableLanguage:'en',href:null}};
 const r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/Перевод ещё не подготовлен/);assert.match(html,/Documented responsibilities/);assert.doesNotMatch(html,/FOREIGN OVERVIEW/);
 assert.match(r.e('scenario-nav').innerHTML,/missing-operation/);assert.doesNotMatch(r.e('scenario-nav').innerHTML,/FOREIGN OPERATION/);
 r.run("showEntry('missing-operation')");assert.match(r.e('scenario-content').innerHTML,/Перевод ещё не подготовлен/);assert.doesNotMatch(r.e('scenario-content').innerHTML,/FOREIGN EXPLANATION/);
});
test('static chrome translation never rewrites interpolated text even when it equals a chrome key',()=>{
 const data=fixture();data.language='ru';const r=load(data);
 assert.equal(r.run('chromeHtml`<p>Source ${"Source"} ${"READ_SOURCE"} ${"/Source/in"}</p>`'),'<p>Исходный код Source READ_SOURCE /Source/in</p>');
 assert.equal(r.run('chromeHtml`<p>invisible Within SourceMapping</p>`'),'<p>invisible Within SourceMapping</p>');
});
test('reader tracks the header height without requiring ResizeObserver',()=>{
 const reader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 let height=66,update,observed,variable;
 const header={getBoundingClientRect:()=>({height})};
 const document={documentElement:{lang:'en',style:{setProperty:(name,value)=>{variable=[name,value];}}},querySelector:key=>key==='.reader-nav'?header:null,querySelectorAll:()=>[],getElementById:()=>null};
 vm.runInNewContext(reader,{document,URL,URLSearchParams,location:{pathname:'/guide.html',search:''},ResizeObserver:class{constructor(callback){update=callback;}observe(element){observed=element;}}});
 assert.equal(observed,header);
 assert.deepEqual(variable,['--reader-header-height','66px']);
 height=110;update();assert.deepEqual(variable,['--reader-header-height','110px']);
 vm.runInNewContext(reader,{document,URL,URLSearchParams,location:{pathname:'/guide.html',search:''}});
 assert.deepEqual(variable,['--reader-header-height','110px']);
});
test('shared catalogue localizes chrome and retains neutral filtering keys and authored titles',()=>{
 const reader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 const element=()=>({children:[],value:'',textContent:'',listeners:{},replaceChildren(){this.children=[];},append(...nodes){this.children.push(...nodes);},addEventListener(kind,fn){this.listeners[kind]=fn;}});
 for(const language of ['en','ru']){
  const nodes=Object.fromEntries(['catalog-data','catalog-query','catalog-kind','catalog-results','catalog-status','catalog-prev','catalog-next'].map(key=>[key,element()]));
  nodes['catalog-data'].textContent=JSON.stringify([{kind:'Service',id:'orderAPI',title:'Authored title',href:'services/orderAPI.html'},{kind:'Process',id:'p',title:'ProcessTitle',href:'scenarios/p.html'}]);
  const document={documentElement:{lang:language},querySelector(){return null;},querySelectorAll(){return [];},getElementById(key){return nodes[key];},createElement:element};
  vm.runInNewContext(reader,{document,URL,URLSearchParams,location:{search:'',pathname:'/catalog.html'}});
  assert.equal(nodes['catalog-results'].children.length,2);
  assert.equal(nodes['catalog-results'].children[0].children[0].textContent,'Authored title');
  assert.equal(nodes['catalog-results'].children[0].children[1].textContent,language==='ru'?'Сервис · orderAPI':'Service · orderAPI');
  nodes['catalog-kind'].value='Service';nodes['catalog-kind'].listeners.change();assert.equal(nodes['catalog-results'].children.length,1);
  nodes['catalog-query'].value='absent';nodes['catalog-query'].listeners.input();assert.equal(nodes['catalog-results'].children.length,0);
  assert.match(nodes['catalog-status'].textContent,language==='ru'?/Документы не найдены/:/No matching/);
 }
});
