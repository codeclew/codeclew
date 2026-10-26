'use strict';
// Execute the shipped reader, not a parallel renderer. The minimal DOM captures
// its generated HTML and event dispatch; browser geometry is tested separately.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const iconAssets=Object.fromEntries(['arrow-right','database','license'].map(name=>[name,fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/tabler/',name==='license'?'LICENSE':name+'.svg'),'utf8')]));
const localeScript='var CLEW_RU_MESSAGES='+fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/ru.json'),'utf8')+';\n'+fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/locale.js'),'utf8')+'\n';
const script=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/markdown.js'),'utf8')+'\n'+localeScript+'var QUESTION_ICONS='+JSON.stringify(iconAssets)+';\n'+fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/app.js'),'utf8')+'\n'+fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/question-reader.js'),'utf8');
const fragment=text=>({id:text,text,dependencyIds:['dep'],sourceIds:['same-source']});

test('field inspector uses exact model and field identities and returns focus to its canonical row',()=>{
 for(const language of ['en','ru']){
  const data=fieldInspectorFixture(language),before=JSON.stringify(data),r=load(data);r.run('showEntry("section-entities")');
  const frame=r.e(r.run('domainRecordsKey("order")'));
  r.click({domainModel:'order',domainFieldSelect:'current-id'});
  assert.match(frame.outerHTML,/CURRENT_ONLY_USE/);assert.doesNotMatch(frame.outerHTML,/ORIGIN_ONLY_USE/);
  assert.match(frame.outerHTML,/CurrentRow.id/);assert.doesNotMatch(frame.outerHTML,/OriginDto.sourceId/);
  assert.equal((frame.outerHTML.match(/data-domain-field="current-id"/g)||[]).length,1);
  assert.match(frame.outerHTML,/field-glossary" hidden/);assert.ok(r.e(r.run('domainFieldKey("order","current-id")+"-detail"')).focused);
  assert.equal(r.e('source-panel').hidden,true);
  r.click({domainModel:'order',domainFieldReturn:'current-id'});assert.doesNotMatch(frame.outerHTML,/field-glossary" hidden/);
  assert.ok(r.e(r.run('domainFieldKey("order","current-id")+"-control"')).focused);
  assert.match(frame.outerHTML,/ORIGIN_ONLY_USE/);assert.equal(JSON.stringify(data),before);
 }
});

test('field inspector preserves absent usage and representations as explicit local gaps',()=>{
 const data=fieldInspectorFixture(),before=JSON.stringify(data),r=load(data);r.run('showEntry("section-entities")');r.click({domainModel:'order',domainFieldSelect:'unexamined'});
 const html=r.e(r.run('domainRecordsKey("order")')).outerHTML;
 assert.match(html,/No usage is recorded in this scope/);assert.match(html,/No representation mapping is recorded for this field/);
 assert.match(html,/data-domain-field-return="unexamined"/);assert.equal(JSON.stringify(data),before);
});

test('field inspector refuses ambiguous IDs and does not join fields with overlapping names',()=>{
 const data=fieldInspectorFixture(),before=JSON.stringify(data),r=load(data);r.run('showEntry("section-entities")');
 r.click({domainModel:'order',domainFieldSelect:'origin-id'});assert.match(r.e(r.run('domainRecordsKey("order")')).outerHTML,/ORIGIN_ONLY_USE/);assert.doesNotMatch(r.e(r.run('domainRecordsKey("order")')).outerHTML,/CURRENT_ONLY_USE/);
 r.click({domainModel:'other-record',domainFieldSelect:'current-id'});assert.match(r.e(r.run('domainRecordsKey("other-record")')).outerHTML,/OTHER_MODEL_ONLY_USE/);
 assert.equal(r.run('domainFieldSelections.get("order")'),'origin-id');assert.equal(r.run('domainFieldSelections.get("other-record")'),'current-id');
 assert.equal(r.run('inspectDomainField("ambiguous","same-id")'),false);
 const ambiguous=r.run('modelFieldRecords(D.readerModels.find(m=>m.id==="ambiguous"))');assert.doesNotMatch(ambiguous,/data-domain-field-select/);assert.match(ambiguous,/ambiguous or missing identity/);
 assert.equal(JSON.stringify(data),before);
 const duplicated=fieldInspectorFixture();duplicated.readerModels.push(structuredClone(duplicated.readerModels[0]));
 const copy=load(duplicated);assert.doesNotMatch(copy.run('modelFieldRecords(D.readerModels[0])'),/data-domain-field-select|id="domain-records/);
});

test('polish route heading has one transport identity and preserves a distinct authored title',()=>{
 for(const language of ['en','ru']){
  const data=httpNavigationFixture();data.language=language;const before=JSON.stringify(data),r=load(data);
  const plain=r.run('operationIdentity(entries.find(e=>e.id==="health"))');
  assert.equal((plain.match(/<code>\/health<\/code>/g)||[]).length,1);assert.equal((plain.match(/>GET<\/span>/g)||[]).length,1);
  assert.doesNotMatch(plain,/<h2>GET \/health/);assert.match(plain,/data-copy-http-path="health"/);
  const named=r.run('operationIdentity(entries.find(e=>e.id==="named"))');assert.match(named,/<h2>List named places<\/h2>/);assert.equal((named.match(/<code>\/places\/named<\/code>/g)||[]).length,1);
  assert.equal(JSON.stringify(data),before);
 }
});

test('polish section navigation and search recovery retain current selection and input focus',()=>{
 for(const language of ['en','ru']){
  const data=fixture();data.language=language;const before=JSON.stringify(data),r=load(data);
  const overview=r.e('scenario-nav').innerHTML.match(/<button[^>]*data-entry="section-overview"[^>]*>[\s\S]*?<\/button>/)[0];
  assert.match(overview,/aria-current="page"/);assert.doesNotMatch(overview,/nav-icon|SECTION|РАЗДЕЛ/);
  assert.equal(r.e('clear-search').hidden,true);r.e('search').value='no-such-route';r.run('nav()');
  assert.equal(r.e('clear-search').hidden,false);assert.match(r.e('scenario-nav').innerHTML,/data-clear-search/);
  r.click({clearSearch:''});assert.equal(r.e('search').value,'');assert.equal(r.document.activeElement,r.e('search'));assert.equal(r.e('clear-search').hidden,true);
  assert.match(r.e('scenario-nav').innerHTML,/aria-current="page"/);assert.equal(JSON.stringify(data),before);
 }
});

test('polish unknown source status stays distinct from stale and actual update failures',()=>{
 for(const language of ['en','ru'])for(const freshness of ['UNVERIFIED','STALE']){
  const data=fixture();data.language=language;data.sectionState={freshness,verification:'UNASSESSED',reasons:[{reason:'SOURCE_MISSING'}]};
  const before=JSON.stringify(data),r=load(data),html=r.e('freshness-status').innerHTML;
  assert.match(html,new RegExp('freshness-'+freshness.toLowerCase()));assert.doesNotMatch(html,/freshness-error/);assert.match(html,/SOURCE_MISSING/);assert.equal(JSON.stringify(data),before);
 }
 const data=fixture();data.updateFailures={sample:{error:'Synthetic source read failed'}};const before=JSON.stringify(data),r=load(data),html=r.e('freshness-status').innerHTML;
 assert.match(html,/freshness-error/);assert.match(html,/Update failed/);assert.match(html,/Recent update gaps/);assert.match(html,/Synthetic source read failed/);assert.equal(JSON.stringify(data),before);
});

test('polish exact HTTP path copy reports success and honest failure without guessing paths',async()=>{
 for(const language of ['en','ru']){
  const data=httpNavigationFixture();data.language=language;data.catalogue.push({id:'unknown-path',kind:'HTTP_ENDPOINT',symbol:'unknown',trigger:{methods:['GET'],paths:[]},sourceIds:[]});
  const before=JSON.stringify(data),r=load(data);r.run('globalThis.copied=[];navigator.clipboard.writeText=async value=>copied.push(value)');
  assert.equal(await r.run('copyHttpPath({dataset:{copyHttpPath:"refresh",pathIndex:"0"}})'),true);assert.equal(r.run('copied[0]'),'/places/{id}/refresh');
  assert.equal(r.e('http-copy-status').textContent,language==='ru'?'Путь скопирован':'Path copied');
  r.run('navigator.clipboard.writeText=async()=>{throw new Error("denied")}');assert.equal(await r.run('copyHttpPath({dataset:{copyHttpPath:"health",pathIndex:"0"}})'),false);
  assert.match(r.e('http-copy-status').textContent,/\/health/);assert.doesNotMatch(r.e('http-copy-status').textContent,/Path copied|Путь скопирован/);
  assert.equal(await r.run('copyHttpPath({dataset:{copyHttpPath:"unknown-path",pathIndex:"0"}})'),false);
  assert.doesNotMatch(r.run('operationIdentity(entries.find(e=>e.id==="unknown-path"))'),/data-copy-http-path/);
  r.run('showEntry("refresh")');assert.equal(r.e('http-copy-status').textContent,'');assert.equal(JSON.stringify(data),before);
 }
});

test('polish compact external summaries preserve all grouped operations keys and profile links',()=>{
 const data=taskReaderFixture(),before=JSON.stringify(data),r=load(data),html=r.run('groupedEgress()');
 assert.match(html,/egress-config-keys/);assert.match(html,/class="egress-chevron" aria-hidden="true"/);assert.match(html,/provider.endpoint/);assert.match(html,/external-services\/provider.html/);assert.match(html,/external-services\/renewal.html/);assert.match(html,/external-services\/store.html/);
 assert.match(html,/Compute selected result/);assert.match(html,/Compute renewal result/);assert.match(html,/Attempt result write/);assert.equal(JSON.stringify(data),before);
});

test('polish path controls retain multiple exact paths and ignore an obsolete copy completion',async()=>{
 const data=httpNavigationFixture(),route=data.catalogue.find(e=>e.id==='health');route.trigger.paths=['/health','/health/{detail}'];
 const before=JSON.stringify(data),r=load(data),html=r.run('operationIdentity(entries.find(e=>e.id==="health"))');
 assert.equal((html.match(/data-copy-http-path="health"/g)||[]).length,2);
 r.run('globalThis.copied=[];navigator.clipboard.writeText=async value=>copied.push(value)');
 await r.run('copyHttpPath({dataset:{copyHttpPath:"health",pathIndex:"1"}})');assert.equal(r.run('copied[0]'),'/health/{detail}');
 r.run('navigator.clipboard.writeText=()=>new Promise(resolve=>globalThis.completeCopy=resolve)');
 const pending=r.run('copyHttpPath({dataset:{copyHttpPath:"health",pathIndex:"0"}})');r.run('showEntry("refresh");completeCopy()');await pending;assert.equal(r.e('http-copy-status').textContent,'');
 route.trigger.pathResolution='UNRESOLVED';const unknown=load(data);assert.doesNotMatch(unknown.run('operationIdentity(entries.find(e=>e.id==="health"))'),/data-copy-http-path/);delete route.trigger.pathResolution;assert.equal(JSON.stringify(data),before);
});

test('HTTP navigation shows transport methods once and retains exact routes and meaningful titles',()=>{
 for(const mobile of [false,true]){
  const data=httpNavigationFixture(),before=JSON.stringify(data),reader=load(data,mobile);
  const row=id=>reader.e('scenario-nav').innerHTML.match(new RegExp('<button[^>]*data-entry="'+id+'"[^>]*>[\\s\\S]*?<\\/button>'))?.[0];
  const text=id=>row(id).replace(/<[^>]+>/g,' ').replace(/\s+/g,' ').trim();
  for(const [id,expected] of [
   ['health','GET /health'],['create-place','POST /places'],['list-places','GET /places'],
   ['openapi','GET /openapi.json'],['refresh','POST /places/{id}/refresh'],
   ['named','GET /places/named · List named places'],['unprefixed','GET /places/history'],
   ['multiple','GET / POST /places/{id} · Read or replace place'],
   ['natural-title','GET /health/report · Get health report']
  ]){
   assert.equal(text(id),expected);
   assert.doesNotMatch(row(id),/aria-hidden="true"|aria-label=/);
  }
  assert.equal(text('local'),'CODE GET values from local cache');
  assert.equal(JSON.stringify(data),before);
 }
});

test('HTTP navigation preserves search matches, entry selection and original headings',()=>{
 for(const mobile of [false,true]){
  const data=httpNavigationFixture(),before=JSON.stringify(data),reader=load(data,mobile);
  for(const [query,id] of [['GET /health','health'],['refresh','refresh'],['POST / GET','multiple'],['List named places','named']]){
   reader.e('search').value=query;reader.run('nav()');
   assert.ok(reader.e('scenario-nav').innerHTML.includes('data-entry="'+id+'"'));
  }
  reader.e('search').value='refresh';reader.run('nav()');
  assert.doesNotMatch(reader.e('scenario-nav').innerHTML,/data-entry="health"/);
  reader.click({entry:'refresh'});
  assert.equal(reader.run('current.id'),'refresh');
  assert.match(reader.e('scenario-content').innerHTML,/<h2 class="operation-label route-heading">/);
  assert.equal((reader.e('scenario-content').innerHTML.match(/<code>\/places\/\{id\}\/refresh<\/code>/g)||[]).length,1);
  assert.equal(data.operations.find(op=>op.id==='refresh').title,'POST /places/{id}/refresh');
  assert.equal(JSON.stringify(data),before);
 }
});

test('HTTP navigation handles declared routes and incomplete methods without stripping ordinary prose',()=>{
 const data=httpNavigationFixture(),seed=data.operations[0];
 const rows=[
  ['declared','DECLARED_OPENAPI',['GET'],'/declared','GET /declared','/declared'],
  ['unknown','HTTP_ENDPOINT',[],'/unknown','Inspect unknown transport','Inspect unknown transport'],
  ['legacy','HTTP_ENDPOINT',[null],'','Original retained title','Original retained title'],
  ['custom','HTTP_ENDPOINT',['M-SEARCH'],'/discovery','M-SEARCH /discovery','/discovery'],
  ['word','HTTP_ENDPOINT',['GET'],'/status','GETTING status','/status · GETTING status']
 ];
 for(const [id,kind,methods,route,title] of rows){
  data.catalogue.push({id,kind,symbol:title,trigger:{methods,paths:route?[route]:[]},sourceIds:[]});
  data.operations.push({...seed,id,title});
 }
 const before=JSON.stringify(data),reader=load(data);
 for(const [id,_kind,_methods,_route,_title,expected] of rows)assert.equal(reader.run('navigationLabel(entries.find(e=>e.id==='+JSON.stringify(id)+'))'),expected);
 assert.equal(JSON.stringify(data),before);
});

function httpNavigationFixture(){
 const data=fixture(),seed=data.operations[0];
 const rows=[
  ['health',['GET'],['/health'],'GET /health'],
  ['create-place',['POST'],['/places'],'POST /places'],
  ['list-places',['GET'],['/places'],'GET /places'],
  ['openapi',['GET'],['/openapi.json'],'GET /openapi.json'],
  ['refresh',['POST'],['/places/{id}/refresh'],'POST /places/{id}/refresh'],
  ['named',['GET'],['/places/named'],'List named places'],
  ['unprefixed',['GET'],['/places/history'],'/places/history'],
  ['multiple',['GET','POST'],['/places/{id}'],'POST / GET /places/{id} · Read or replace place'],
  ['natural-title',['GET'],['/health/report'],'Get health report'],
  ['local',[],[],'GET values from local cache']
 ];
 data.title='Synthetic HTTP navigation fixture';data.subtitle='Synthetic routes; no private source or runtime behavior is represented.';
 data.catalogue=rows.map(([id,methods,paths,title])=>({id,symbol:id==='local'?'Cache.read':title,kind:id==='local'?'SOURCE_DECLARATION':'HTTP_ENDPOINT',trigger:{methods,paths},sourceIds:[]}));
 data.operations=rows.map(([id,_methods,_paths,title])=>({...seed,id,title,summary:fragment('Synthetic navigation example.'),visuals:[]}));
 data.sections=[];data.boundaryInventory={publicBoundaries:data.catalogue.filter(e=>e.kind==='HTTP_ENDPOINT'),gaps:[]};
 return data;
}

test('service egress keeps logical storage and other-trigger boundaries with local participant links',()=>{
 const data=fixture();data.subject='service:orders';
 data.declaredRelationships={nodes:[{id:'external-service:store',label:'Logical reservation storage'},{id:'external-service:sdk',label:'Inference SDK'}],edges:[
  {id:'guarded-save',from:'service:orders',to:'external-service:store',title:'Attempt guarded save',transport:{kind:'logical'}},
  {id:'other-trigger',from:'service:orders',to:'external-service:sdk',title:'Other trigger SDK call',transport:{kind:'logical'}},
  {id:'foreign',from:'service:other',to:'external-service:sdk',title:'Foreign service call',transport:{kind:'logical'}}]};
 const before=JSON.stringify(data),reader=load(data),html=reader.run('serviceDeclaredEgress()');
 assert.match(html,/Logical reservation storage/);assert.match(html,/Other trigger SDK call/);
 assert.match(html,/href="\.\.\/external-services\/store\.html"/);assert.match(html,/Declaration and source checks/);
 assert.doesNotMatch(html,/Foreign service call/);assert.equal(JSON.stringify(data),before);
 assert.match(reader.run('groupedEgress()'),/Attempt guarded save/);
});

test('egress labels remain escaped and same-destination callsites are retained',()=>{
 const data=fixture();data.subject='service:orders';
 data.declaredRelationships={nodes:[{id:'external-service:provider',label:'Provider <unsafe>'}],edges:[1,2].map(n=>({id:'call-'+n,from:'service:orders',to:'external-service:provider',title:'Call '+n,transport:{kind:'http'}}))};
 const html=load(data).run('serviceDeclaredEgress()');assert.match(html,/Call 1/);assert.match(html,/Call 2/);
 assert.match(html,/Provider &lt;unsafe&gt;/);assert.doesNotMatch(html,/<unsafe>/);
});

test('proof prefers the recorded explicit claim source without changing evidence order or bindings',()=>{
 const data=fixture(),operation=data.operations[0],event={id:'helper-call',kind:'note',text:'Read helper result',dependencyIds:[],sourceIds:[]};operation.events=[event];
 data.sources={'a-annotation':source('@Component'), 'z-body':{...source('fun helper() { return result }'),startLine:15,endLine:25}};
 operation.summary.sourceIds=['a-annotation','z-body'];event.sourceIds=['a-annotation','z-body'];data.operationSources[operation.id]=data.sources;
 data.preferredSourceIds={[operation.id]:{[event.id]:['z-body'],[operation.summary.id]:['z-body']}};
 const before=JSON.stringify(data),reader=load(data);reader.run("showEntry('section-responsibilities')");
 reader.click({event:event.id});assert.match(reader.e('source-code').innerHTML,/fun helper/);
 assert.equal(reader.e('source-select').value,'z-body');
 assert.ok(reader.e('source-select').innerHTML.indexOf('a-annotation')<reader.e('source-select').innerHTML.indexOf('z-body'));
 reader.click({sources:'a-annotation z-body',sourceClaim:operation.summary.id});
 assert.equal(reader.e('source-select').value,'z-body');assert.match(reader.e('source-code').innerHTML,/fun helper/);
 reader.e('source-select').onchange({target:{value:'a-annotation'}});assert.match(reader.e('source-code').innerHTML,/@Component/);
 assert.equal(JSON.stringify(data),before);
});

test('Russian readable step proof keeps its operation and claim binding and opens explicit body',()=>{
 const data=fixture();data.language='ru';const op=data.operations[0];
 data.sources={'annotation':source('@Service'),'body':source('fun helper() { return client.getForEntity(url) }')};
 data.operationSources[op.id]=data.sources;op.events=[{id:'helper-call',kind:'note',text:'Helper call',sourceIds:['annotation','body'],dependencyIds:[]}];
 data.preferredSourceIds={[op.id]:{'helper-call':['body']}};
 const reader=load(data);reader.run("showEntry('section-responsibilities')");
 const html=reader.run('stepEvidence(D.operations[0],D.operations[0].events[0])');
 assert.match(html,/class="source-link"/);assert.match(html,/data-source-operation="section-responsibilities"/);assert.match(html,/data-source-claim="helper-call"/);
 const dataset=Object.fromEntries([...html.matchAll(/data-([a-z-]+)="([^"]*)"/g)].map(([,key,value])=>[key.replace(/-([a-z])/g,(_,c)=>c.toUpperCase()),value]));
 reader.click(dataset);assert.equal(reader.e('source-select').value,'body');assert.match(reader.e('source-code').innerHTML,/getForEntity/);
});

test('legacy, foreign and current-snapshot proof preferences do not invent a source match',()=>{
 const data=fixture();data.sources={'a':source('FIRST'),'b':source('SECOND')};
 data.preferredSourceIds={[data.operations[0].id]:{'claim':['foreign','b']}};
 data.operationSources[data.operations[0].id]=data.sources;const reader=load(data);reader.run("showEntry('section-responsibilities')");reader.click({sources:'a b',sourceClaim:'claim'});assert.match(reader.e('source-code').innerHTML,/SECOND/);
 reader.click({sources:'a b',sourceClaim:'unknown'});assert.match(reader.e('source-code').innerHTML,/FIRST/);
 reader.click({sources:'a b',sourceClaim:'claim'},true);assert.match(reader.e('source-code').innerHTML,/FIRST/);
 const legacy=load({...data,preferredSourceIds:undefined});legacy.click({sources:'a b',sourceClaim:'claim'});assert.match(legacy.e('source-code').innerHTML,/FIRST/);
});

test('branch rows keep authored prefixes once and preserve the original event text',()=>{
 for(const language of ['en','ru']){
  const data=fixture();data.language=language;const before=JSON.stringify(data),reader=load(data);
  for(const [kind,text,prefixed] of [['alt','If quantity is positive',true],['alt','Если количество положительное',true],['alt','quantity is positive',false],['alt','ifEnabled is true',false],['else','Otherwise return the unchanged value',true],['else','Иначе вернуть исходное значение',true]]){
   const event={kind,text};
   const rendered=JSON.parse(reader.run(`JSON.stringify(pseudocodeEvent(D.operations[0],${JSON.stringify(event)}))`));
   assert.equal(rendered.keyword,prefixed?'':reader.run(`t(${JSON.stringify(kind==='alt'?'If':'Otherwise')})`));
   assert.equal(rendered.text,text+':');
  }
  for(const text of ['Otherwise','Иначе']){
   const rendered=JSON.parse(reader.run(`JSON.stringify(pseudocodeEvent(D.operations[0],{kind:'else',text:${JSON.stringify(text)}}))`));
   assert.equal(rendered.text,'');assert.equal(rendered.keyword,reader.run("t('Otherwise')"));
  }
  assert.equal(JSON.stringify(data),before);
 }
});

test('Russian related-documentation and ownership labels are complete owned sentences',()=>{
 const data=fixture();data.language='ru';const reader=load(data);
 const html=reader.run('chromeHtml`<h3>Related documentation</h3><p>Service data and domain context</p><h3>Domain entities and their relations</h3><p>${esc(t("The service data description above is separate from domain ownership."))} ${esc(t("No domain ownership declarations have been recorded."))}</p>`');
 assert.match(html,/Связанная документация/);assert.match(html,/Данные сервиса и предметная область/);
 assert.match(html,/Предметные сущности и связи с сервисом/);assert.match(html,/Сведения о владельцах сущностей предметной области не заявлены/);
 assert.doesNotMatch(html,/domain|ownership|Related|Service data/);
});

test('reading scope distinguishes described public operations from internal inventory and selection',()=>{
 const data=fixture(),seed=data.operations[0];
 data.operations=[{...seed,id:'public-1'},{...seed,id:'internal-1'}];
 data.catalogue=['public-1','public-2','public-3','internal-1','internal-2'].map(id=>({id,kind:'CALLABLE',symbol:id,trigger:{methods:[],paths:[]},sourceIds:[]}));
 data.boundaryInventory={publicBoundaries:data.catalogue.filter(e=>e.id.startsWith('public-')),internalCallableCount:2,gaps:[]};
 const before=JSON.stringify(data),reader=load(data),html=reader.run('readingScopeSummary()');
 assert.match(html,/<b>1<\/b> documented public operations/);assert.match(html,/<b>3<\/b> discovered public boundaries/);assert.match(html,/<b>2<\/b> internal callable evidence records/);
 assert.doesNotMatch(html,/explicit gaps|<b>5<\/b> discovered/);assert.equal(JSON.stringify(data),before);
 const russian=load({...data,language:'ru'}).run('readingScopeSummary()');assert.match(russian,/описанных публичных операций/);assert.match(russian,/найденных публичных интерфейсов/);assert.doesNotMatch(russian,/public|boundaries|callable/);
 data.selectedBoundaryInventory={publicBoundaries:data.catalogue.filter(e=>e.id==='public-2'),gaps:[]};
 const selected=load(data).run('readingScopeSummary()');
 assert.match(selected,/<b>0<\/b> documented public operations/);assert.match(selected,/<b>1<\/b> discovered public boundaries/);assert.doesNotMatch(selected,/internal callable/);
 const legacy=fixture();delete legacy.boundaryInventory;
 assert.match(load(legacy).run('readingScopeSummary()'),/source and API inventory records/);
});

test('overview service profile renders safe Markdown blocks and keeps the section evidence owner',()=>{
 const data=fixture(),seed=data.operations[0];
 const text='Original field | Readable name | Comment\n--- | --- | ---\n`id` | Identifier | [Payload](../services/sample.html#section-entities)\n\n<script>unsafe()</script>\n\n[Unsafe](javascript:alert(1))';
 const entity={...seed,id:'section-entities',title:'Domain entities',summary:fragment(text),visuals:[]};
 data.operations=[entity];data.sections=[{id:'section-overview',title:'Overview',gap:'Awaiting overview'},{id:entity.id,title:entity.title,content:entity}];
 data.operationSources={[entity.id]:{'same-source':source('ACCEPTED ENTITY SOURCE')}};
 data.operationStates={[entity.id]:{freshness:'STALE',verification:'UNASSESSED'}};
 const before=JSON.stringify(data),reader=load(data),html=reader.e('scenario-content').innerHTML;
 assert.match(html,/id="profile-section-entities"/);assert.match(html,/class="profile-summary clew-markdown"/);
 assert.match(html,/<table\b/);assert.match(html,/<th[^>]*>Original field<\/th>/);assert.match(html,/<code>id<\/code>/);
 assert.match(html,/href="\.\.\/services\/sample\.html#section-entities"/);assert.match(html,/&lt;script&gt;unsafe\(\)&lt;\/script&gt;/);
 assert.doesNotMatch(html,/<script>|href="javascript:/);assert.doesNotMatch(html,/<p class="profile-summary">/);
 assert.match(html,/data-source-operation="section-entities"/);assert.match(html,/STALE/);assert.match(html,/UNASSESSED/);
 reader.click({sources:'same-source',sourceOperation:'section-entities'});
 assert.match(reader.e('source-code').innerHTML,/ACCEPTED ENTITY SOURCE/);assert.doesNotMatch(reader.e('source-code').innerHTML,/CURRENT SOURCE/);
 assert.equal(JSON.stringify(data),before);
});

test('shared locale translates owned template chunks without touching matching authored or schema text',()=>{
 const data=fixture();data.language='ru';const reader=load(data);
 const output=reader.run('chromeHtml`<h2>Overview</h2><p>${esc("Overview")}</p><pre>${esc(JSON.stringify({description:"Overview",enum:["required","UNKNOWN"]}))}</pre>`');
 assert.match(output,/<h2>Обзор<\/h2>/);
 assert.match(output,/<p>Overview<\/p>/);
 assert.match(output,/&quot;description&quot;:&quot;Overview&quot;/);
 assert.match(output,/&quot;enum&quot;:\[&quot;required&quot;,&quot;UNKNOWN&quot;\]/);
 assert.equal(reader.run('chromeHtml`<code>in</code>`'),'<code>in</code>');
 assert.equal(data.operations[0].summary.text,fixture().operations[0].summary.text);
});
const source=text=>({service:'sample',file:'Worker.kt',startLine:1,endLine:1,text,revision:'abc',authority:'RETAINED_SOURCE_NOT_REVERIFIED'});
function fixture(){
 const flow={schema:1,generator:{id:'sample',version:'1'},id:'dispatch',kind:'execution-flow',title:'Dispatch work',purpose:fragment('Explain selected dispatch'),scope:fragment('The local handler only'),limitations:['Delivery is not observed.'],nodes:[{id:'start',meaning:fragment('Receive request')},{id:'select',meaning:fragment('Choose handler')},{id:'done',meaning:fragment('Return result')}],edges:[{id:'e1',from:'start',to:'select',meaning:fragment('After validation')},{id:'e2',from:'select',to:'done',meaning:fragment('When handler returns')}],parent:null,hitPolicy:null,rules:[],afterSelection:null};
 const decision={...flow,id:'rules',kind:'decision-table',title:'Select handler',parent:{artifact:'dispatch',node:'select'},nodes:[],edges:[],hitPolicy:'FIRST',policyExplanation:fragment('A return exits the inspected branch.'),rules:[{condition:fragment('The kind is supported'),outcome:fragment('Call the selected handler')}],afterSelection:fragment('Handler failure propagates separately.')};
 const op={id:'section-responsibilities',title:'Responsibilities',summary:fragment('Documented responsibilities'),participants:[],events:[],explanation:[],findings:[],boundaries:[],interfaceContracts:[],visuals:[flow,decision]};
 return {subject:'service:sample',title:'Sample',subtitle:'Example',catalogue:[],contracts:[],operations:[op],sections:[{id:'section-overview',title:'Overview',gap:'Awaiting overview'},{id:op.id,title:op.title,content:op}],notes:[],sources:{'same-source':source('CURRENT SOURCE')},operationSources:{[op.id]:{'same-source':source('ACCEPTED SOURCE')}},operationStates:{[op.id]:{freshness:'STALE',verification:'UNASSESSED'}},sectionState:{freshness:'CURRENT',verification:'VERIFIED'},sourceAuthorities:{},revisions:{sample:'abc'},boundaries:[],interactions:[],gaps:{},coverage:{},boundaryInventory:{publicBoundaries:[],gaps:[]}};
}
function entitySectionFixture(){
 const data=fixture(),seed=data.operations[0].visuals;
 const flow={...seed[0],id:'entity-map',title:'Order identity map',purpose:fragment('Explain declared ownership'),scope:fragment('One synthetic domain identity'),nodes:[{id:'declared',meaning:fragment('Order identity declared')},{id:'reviewed',meaning:fragment('Ownership criteria reviewed')},{id:'accepted',meaning:fragment('Ownership claim documented')}],edges:[{id:'review',from:'declared',to:'reviewed',meaning:fragment('The declaration is reviewed')},{id:'document',from:'reviewed',to:'accepted',meaning:fragment('Evidence supports the documented claim')}]};
 const decision={...seed[1],id:'ownership-rule',title:'Ownership criteria',purpose:fragment('Summarize the declared relation criteria'),scope:fragment('The synthetic sample service only'),limitations:['Other service ownership is outside this synthetic fixture.'],parent:{artifact:'entity-map',node:'reviewed'},policyExplanation:fragment('Apply only the criteria accepted with this section.'),rules:[{condition:fragment('A relation names the sample service'),outcome:fragment('Show the declared ownership claim')}],afterSelection:fragment('This view does not infer ownership for other entities.')};
 const owner={...data.operations[0],id:'section-entities',title:'Domain entities',summary:fragment('Synthetic accepted entity description'),visuals:[flow,decision]};
 data.title='Synthetic entity reader fixture';data.subtitle='Synthetic browser fixture. All source record bytes below are synthetic.';
 data.operations.push(owner);data.sections.push({id:owner.id,title:owner.title,content:owner});
 data.entities=[{sourceIds:['same-source'],normalized:{entity:{title:'Synthetic Order',description:'Synthetic entity declaration for reader testing.',id:'synthetic-order',relations:[{service:'sample',kind:'owned',origin:'human',confidence:'declared',rationale:'Synthetic relation for reader testing.',representations:['SyntheticOrderRow']}],limitations:['This declaration does not describe a real service.']},missingDependencies:[]}}];
 data.sources['same-source']=source('// SYNTHETIC CURRENT SOURCE RECORD: current entity declaration bytes.');
 data.operationSources[owner.id]={'same-source':source('// SYNTHETIC ACCEPTED SOURCE RECORD: section visual evidence bytes.')};
 data.operationStates[owner.id]={freshness:'STALE',verification:'UNASSESSED'};
 return data;
}
function behaviorFixture(){
 const data=fixture(),events=[
  {id:'for-each',kind:'loop',text:'For each supplied reservation',sourceIds:['loop-source']},
  {id:'positive',kind:'alt',text:'the quantity is positive',sourceIds:['guard-source']},
  {id:'save',kind:'message',text:'Apply <script>alert(1)</script>',from:'service',to:'store',sourceIds:['save-source']},
  {id:'already-closed',kind:'alt',text:'the reservation is already closed',sourceIds:['rejected&guard']},
  {id:'conflict',kind:'return',text:'the conflict response',from:'service',to:'caller',sourceIds:['return-source']},
  {id:'inner-else',kind:'else',text:'Otherwise',sourceIds:['rejected&guard']},
  {id:'continue',kind:'note',text:'Keep the existing value',sourceIds:['continue-source']},
  {id:'inner-end',kind:'end',text:'',sourceIds:['rejected&guard']},
  {id:'optional-audit',kind:'opt',text:'an audit note is supplied',sourceIds:['optional-source']},
  {id:'audit-note',kind:'note',text:'Attach the audit note',sourceIds:['audit-source']},
  {id:'optional-end',kind:'end',text:'',sourceIds:['optional-source']},
  {id:'outer-else',kind:'else',text:'Otherwise',sourceIds:['guard-source']},
  {id:'skip',kind:'note',text:'Leave the input unchanged',sourceIds:['skip-source']},
 {id:'outer-end',kind:'end',text:'',sourceIds:['guard-source']},
  {id:'loop-end',kind:'end',text:'',sourceIds:['loop-source']},
  {id:'result',kind:'return',text:'the accepted result',from:'service',to:'caller',sourceIds:['result-source']}
 ];
 const deepGroups=Array.from({length:9},(_,index)=>({id:`deep-loop-${index}`,kind:'loop',text:`nested group ${index+1}`,sourceIds:['loop-source']}));
 events.splice(events.length-1,0,...deepGroups,{id:'deep-optional',kind:'opt',text:'a provider result is available',sourceIds:['optional-source']},{id:'deep-declared',kind:'declared',text:'Provider submission remains a declaration',from:'service',to:'store',interaction:'declared:provider',sourceIds:['declared-source']},{id:'deep-optional-end',kind:'end',text:'',sourceIds:['optional-source']},...deepGroups.map((_,index)=>({id:`deep-end-${index}`,kind:'end',text:'',sourceIds:['loop-source']})));
 const operation={id:'reserve-fixture',title:'Reserve request',summary:{...fragment('Processes a reservation request.'),sourceIds:['summary-source']},participants:[{id:'caller',label:'Caller',service:null},{id:'service',label:'Reservation service',service:'sample'},{id:'store',label:'Reservation store',service:'sample'}],events,explanation:[{id:'same-step',text:'Apply <script>alert(1)</script>',sourceIds:['save-source'],detail:false},{id:'extra-commentary',text:'Additional authored explanation',sourceIds:['save-source'],detail:true}],findings:[],boundaries:['Runtime execution was not observed.'],interfaceContracts:[],visuals:[],overviewDiagram:null};
 const sourceIds=[...new Set([...operation.summary.sourceIds,...events.flatMap(event=>event.sourceIds)])];
 data.subject='scenario:renderer-fixture';data.title='Renderer fixture';data.subtitle='Synthetic reader fixture. No native field documentation was generated.';data.catalogue=[{id:operation.id,symbol:'PUT /reserve',kind:'HTTP_ENDPOINT',trigger:{methods:['PUT'],paths:['/reserve']},sourceIds:[]}];data.operations=[operation];data.sections=[];data.sources=Object.fromEntries(sourceIds.map(id=>[id,source(`Fixture source for ${id}`)]));data.operationSources={[operation.id]:data.sources};data.operationStates={[operation.id]:{freshness:'UNVERIFIED',verification:'UNASSESSED'}};
 return data;
}
function renderReaderFixture(data){
 const template=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/template.html'),'utf8');
 const style=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/style.css'),'utf8')+'\n'+fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/typography.css'),'utf8');
 const analysis=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/analysis.js'),'utf8');
 const payload=JSON.stringify(data).replace(/</g,'\\u003c').replace(/\u2028/g,'\\u2028').replace(/\u2029/g,'\\u2029');
 const html=template.replace('/*__STYLE__*/',()=>style).replace('/*__SCRIPT__*/',()=>script).replace('/*__ANALYSIS_SCRIPT__*/',()=>analysis).replace('__DOCUMENT_DATA__',()=>payload);
 if(!html.includes(script)||!html.includes(style)||!html.includes(analysis)||html.includes('__DOCUMENT_DATA__'))throw new Error('renderer fixture did not embed the exact shipped reader assets');
 return html;
}
function renderBehaviorFixture(data){return renderReaderFixture(data);}
if(process.env.CODECLEW_HTTP_NAV_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_HTTP_NAV_FIXTURE_OUT);fs.mkdirSync(path.dirname(output),{recursive:true});fs.writeFileSync(output,renderReaderFixture(httpNavigationFixture()));
}
function boundaryLayoutFixture(language='en'){
 const data=behaviorFixture(),op=data.operations[0],russian=language==='ru';data.language=language;
 const label=russian?'Внешняя система согласования синтетических резервирований и уведомлений':'External synthetic reservation approval and notification system';
 op.participants=[{id:'service',label:russian?'Сервис синтетического резервирования':'Synthetic reservation service',service:'sample'},{id:'provider',label,service:null,externalService:'provider'}];
 op.events=[{id:'request',kind:'declared-boundary',text:russian?'Отправить объявленный запрос на резервирование. Порядок выполнения на принимающей стороне и доставка не подтверждены сохранёнными данными.':'Send the declared reservation request. Receiver execution order and successful delivery are not established by the retained evidence.',from:'service',to:'provider',interaction:'request',sourceIds:[],dependencyIds:['interaction:request']}];
 data.operationExternalCalls={[op.id]:[{eventId:'request',interactionId:'request',to:label,title:'Synthetic reservation',declaration:{origin:'human',rationale:'Independent synthetic layout fixture.'},transport:{kind:'http'},declarationDigest:'synthetic-declaration',snapshotContextDigest:'synthetic-snapshot',authority:'USER_DECLARATION_NOT_RUNTIME_PROOF',retained:true,targetChanged:false}]};
 return data;
}
if(process.env.CODECLEW_BOUNDARY_LAYOUT_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_BOUNDARY_LAYOUT_FIXTURE_OUT);
 fs.mkdirSync(path.dirname(output),{recursive:true});
 fs.writeFileSync(output,renderReaderFixture(boundaryLayoutFixture(process.env.CODECLEW_BOUNDARY_LAYOUT_LANGUAGE||'en')));
}
if(process.env.CODECLEW_READER_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_READER_FIXTURE_OUT);
 fs.mkdirSync(path.dirname(output),{recursive:true});
 fs.writeFileSync(output,renderBehaviorFixture(behaviorFixture()));
}
if(process.env.CODECLEW_ENTITY_SECTION_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_ENTITY_SECTION_FIXTURE_OUT);
 fs.mkdirSync(path.dirname(output),{recursive:true});
 fs.writeFileSync(output,renderReaderFixture(entitySectionFixture()));
}
function processFixture(svgAvailable=false,lifecycleName='changeTaskStatus'){
 const data=fixture();data.subject='scenario:checkout';data.catalogue=[];
 const operation={id:'approve-request',title:'Approve request',summary:fragment('Accepted process operation details'),participants:[],events:[],explanation:[],findings:[],boundaries:[],interfaceContracts:[],visuals:[],dataflow:null,assessment:null,overviewDiagram:null,documentationLanguage:null};
 const overview={...operation,id:'process-overview',title:'Process overview',summary:fragment('Accepted process overview')};
 data.operations.push(overview,operation);
 data.process={definition:{title:'Checkout process',process:{scope:'One checkout',trigger:'A checkout request arrives',participants:['orders'],objects:[],outcomes:['Checkout accepted']}},authority:'Accepted process definition',targetChanged:false,targetGaps:[],linkedSubviews:[]};
 data.stateDiagram='scenario-checkout-states';data.stateDiagramSvg=svgAvailable;
 data.activityTransitions='scenario-checkout-activity-transitions';data.activityTransitionsSvg=svgAvailable;
 data.lifecycleOperations=[{name:lifecycleName,tree:'Entry: updateStatus()\n  return result',origin:'source',diagramStem:'scenario-checkout-lifecycle-8a41c7d0301b2f9c',svgAvailable}];
 return data;
}
function processOutline(svgAvailable=false){
 return {status:'STATIC_SOURCE_OUTLINE',authority:'STATIC_SOURCE_STRUCTURE_NOT_REVIEWED',causal:true,origin:'source',diagramStem:'scenario-checkout-process-outline-91ab42',pumlAvailable:true,svgAvailable,sourceIds:['checkout-root'],root:{service:'orders',scope:':main',symbol:'method:class:example.CheckoutController#checkout()Ljava/lang/String;',observation:'orders:symbol:checkout',observationDigest:'flow-digest',sourceIds:['checkout-root'],sourceRecordDigests:{'checkout-root':'source-record-digest'},candidates:[]},tree:'Entry: method:class:example.CheckoutController#checkout()Ljava/lang/String;\n[D] if (!hasPositiveQuantity(request)) then\n  return invalid()\n[W] reservations.save(request)\nreturn inventory.reserve(request)'};
}
function load(data=fixture(),mobile=false){
 const elements=new Map(),listeners={},navigationHistory=[],opened=[],browserLocation={hash:''};
 let testDocument;
 const media={matches:mobile,addEventListener(){}};
 function element(id){if(!elements.has(id))elements.set(id,{id,value:'',hidden:id==='source-panel',innerHTML:'',textContent:'',isConnected:true,tabIndex:0,attributes:{},children:[],classList:{add(){},remove(){},toggle(){}},focus(){this.focused=true;testDocument.activeElement=this;},setAttribute(name,value){this.attributes[name]=value;},removeAttribute(name){delete this.attributes[name];},contains(node){return node===this||this.children.includes(node);},querySelectorAll(){return this.children;},closest(){return null;},matches(){return false;},getClientRects(){return this.hidden?[]:[{}];},scrollIntoView(){this.scrolled=true;},insertAdjacentHTML(_,html){this.innerHTML=html+this.innerHTML;},addEventListener(){}});return elements.get(id);}
 element('document-data').textContent=JSON.stringify(data);
 testDocument={documentElement:{lang:"en"},getElementById:element,addEventListener:(name,fn)=>listeners[name]=fn,querySelectorAll:()=>[],querySelector:()=>null,body:{children:[],classList:{add(){},remove(){},toggle(){}}},activeElement:null};
 const context=vm.createContext({document:testDocument,URLSearchParams,location:browserLocation,history:{replaceState(_,__,hash){browserLocation.hash=hash;},pushState(_,__,hash){browserLocation.hash=hash;navigationHistory.push(hash);}},window:{open(...args){opened.push(args);},addEventListener(){},scrollTo(){},matchMedia:()=>media,getComputedStyle:element=>({visibility:element.visibility||'visible'})},navigator:{clipboard:{writeText:async()=>{}}}});
 vm.runInContext(script,context);
 return {data,e:element,document:testDocument,media,navigationHistory,opened,run:code=>vm.runInContext(code,context),key(key,shiftKey=false){const event={key,shiftKey,target:testDocument.activeElement,preventDefault(){this.prevented=true;}};listeners.keydown(event);return event.prevented===true;},click(dataset,currentSources=false){listeners.click({target:{closest:()=>({dataset,classList:{add(){}},hasAttribute:name=>name==='data-current-sources'&&currentSources})},preventDefault(){}});}};
}
function importedObservationFixture(value){
 const data=fixture(),text=JSON.stringify(value,null,2),id='assessment-observation';
 data.notes=[{id,association:{title:'original-observation.json',classification:'mixed',targets:[],period:value.nodeVersion||value.startedAt||'recorded label',service:'sample'},associationDigest:'association-digest',original:{status:'CAPTURED',digest:'original-digest',text},snapshotLabel:'Original imported text, not a verified fact.',assessmentSubject:data.subject,assessment:{id,title:'A bounded assessment of the recorded exercise',summary:fragment('Exact assessment prose.\nIts limitations remain visible.'),assessment:{outcome:'CONSISTENT',period:value.nodeVersion||value.startedAt||'recorded label',noteDigest:'original-digest',associationDigest:'association-digest'}},missingTargets:[]}];
 data.operationStates[id]={freshness:'SOURCE_BOUND',verification:'UNASSESSED'};
 return data;
}
test('recognized imported observations keep historical results separate from assessment and collapsed originals',()=>{
 const value={schema:'clew-weather-runtime-observation/1.0',id:'synthetic-replay',origin:'LOCAL_TEST_OBSERVATION',compilerEvidence:false,status:'PASS',sourceCommit:'a'.repeat(40),startedAt:'2026-10-13T01:19:20Z',cases:[{case:'forecast-before-place',pass:true},{case:'<script>unsafe</script>',pass:false}]};
 const data=importedObservationFixture(value),before=JSON.stringify(data),r=load(data);r.run("showEntry('assessment-observation')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Historical runtime observation/);assert.match(html,/1 of 2 replay cases reported passing/);
 assert.match(html,/Imported, unverified record/);assert.match(html,/Assessment conclusion: CONSISTENT/);
 assert.ok(html.indexOf('Separate agent assessment')<html.indexOf('<summary>Original record'));
 assert.match(html,/<details class="note-record-details"><summary>Original record/);
 assert.match(html,/Exact assessment prose\.\nIts limitations remain visible\./);
 assert.doesNotMatch(html,/<script>unsafe/);assert.match(html,/&lt;script&gt;unsafe/);
 assert.equal(JSON.stringify(data),before);assert.equal(r.run('D.notes[0].original.text'),data.notes[0].original.text);
});
test('Node observation labels its version correctly and arbitrary note JSON has no runtime-result projection',()=>{
 const value={schema:'clew-weather-runtime-observation/1.0',id:'synthetic-validator',origin:'LOCAL_TEST_OBSERVATION',compilerEvidence:false,status:'PASSED',repositoryRevision:'b'.repeat(40),nodeVersion:'v24.21.0',exerciseResult:{status:'PASSED',assertions:6,runtime:'v24.21.0'}};
 const r=load(importedObservationFixture(value));r.run("showEntry('assessment-observation')");
 assert.match(r.e('scenario-content').innerHTML,/Node version: <code>v24.21.0/);
 assert.match(r.e('scenario-content').innerHTML,/Node version assessed: v24.21.0/);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/Period: v24/);
 const other=load(importedObservationFixture({...value,schema:'arbitrary-json/1.0'}));other.run("showEntry('assessment-observation')");
 assert.doesNotMatch(other.e('scenario-content').innerHTML,/Historical runtime observation|Reported result:/);
 assert.match(other.e('scenario-content').innerHTML,/Original record/);
});
test('default service overview exposes the first native graph and all visual navigation',()=>{
 const r=load(),html=r.e('scenario-content').innerHTML;
 assert.match(html,/<svg class="artifact-svg"/);
 assert.doesNotMatch(html,/behavior-pseudocode/);
 assert.match(html,/After validation/);
 assert.match(html,/Meaning review: <b>UNASSESSED/);
 assert.match(html,/Source freshness: <b>STALE/);
 assert.match(r.e('scenario-nav').innerHTML,/Dispatch work/);
 assert.match(r.e('scenario-nav').innerHTML,/Select handler/);
 assert.equal(r.run('inventoryEntries().length'),0);
});
test('Kafka boundaries keep local publication separate from linked continuations without upgrading evidence',()=>{
 const r=load(),from={service:'producer',selector:{language:'kotlin',owner:'class:Producer',name:'publish',parameterTypes:[]}},to={service:'consumer',selector:{language:'typescript',owner:'package:',name:'consume',parameterTypes:[]}};
 const interaction={id:'event-route',origin:'agent-proposal',runtime:'UNKNOWN',declaration:{id:'event-route',from,to,transport:{kind:'kafka',topic:'event.changed'},declaration:{origin:'agent-proposal',rationale:'A declared mapping, not observed delivery.'}},from:{sourceIds:['publisher-source'],candidates:['producer-symbol']},to:{sourceIds:['consumer-source'],candidates:['consumer-symbol']},callSite:{sourceIds:['call-source'],candidates:['call-observation']},topic:{caller:{status:'UNRESOLVED'},receiver:{status:'UNRESOLVED'}},boundaries:['UNACCEPTED_AGENT_PROPOSAL']};
 const publisher={id:'publisher',title:'Publish pending events',href:'../scenarios/publisher.html#process-overview',root:{service:'producer',selector:{...from.selector,name:'runLoop'}},interactions:[interaction.id],steps:[{id:'send',blocks:[{text:'Send to event.changed.',sourceIds:['publisher-source'],dependencyIds:[]}]},{id:'commit',blocks:[{text:'Mark the outbox row and commit.',sourceIds:['publisher-source'],dependencyIds:[]}]}]};
 const consumer={id:'consumer',title:'Consume events',href:'../scenarios/consumer.html#process-overview',root:to,interactions:[interaction.id],steps:[{id:'receive',blocks:[{text:'Receive event.changed.',sourceIds:['consumer-source'],dependencyIds:[]}]}]};
 r.e('reader-context').textContent=JSON.stringify({processLinks:[publisher,consumer],serviceLabels:{producer:'Producer',consumer:'Consumer'}});
 r.run(`D.interactions=${JSON.stringify([interaction])};D.subject='scenario:save';D.process={definition:{id:'save',root:{service:'front',selector:{name:'save'}},interactions:['event-route']}};`);
 assert.equal(r.run("processTopicSection('send')"),'');
 const related=r.run('relatedTopicContinuations()');
 assert.match(related,/Related asynchronous continuation/);assert.match(related,/not additional publishing steps/);
 assert.match(related,/publisher\.html\?fromProcess=save/);assert.doesNotMatch(related,/Outgoing Kafka topic/);
 r.run(`D.subject='scenario:publisher';D.process={definition:${JSON.stringify({id:'publisher',root:publisher.root,interactions:[interaction.id]})}};`);
 const before=r.run('JSON.stringify(D)'),local=r.run("processTopicSection('send')");
 assert.match(local,/Outgoing Kafka topic/);assert.match(local,/event.changed/);assert.match(local,/UNRESOLVED/);assert.match(local,/UNKNOWN/);assert.match(local,/runtime delivery has not been verified/);
 assert.match(local,/consumer\.html\?fromProcess=publisher/);assert.equal(r.run("processTopicSection('commit')"),'');
 assert.equal(r.run('JSON.stringify(D)'),before);
 r.run("D.subject='service:consumer';D.process=null;");
 assert.match(r.run("serviceTopicBoundaries('incoming')"),/id="topic-incoming-event.changed"/);
 assert.equal(r.run("serviceTopicBoundaries('outgoing')"),'');
});
function longSequenceFixture(count=160){
 const data=boundaryLayoutFixture('en'),op=data.operations[0];
 op.participants=[{id:'service',label:'Synthetic caller',service:'sample'},...Array.from({length:10},(_,i)=>({id:'provider-'+i,label:'Synthetic port '+i,externalService:'provider-'+i}))];
 const event=(id,kind,text,from='service',to=null)=>({id,kind,text,from,to,interaction:kind.startsWith('declared-boundary')?'synthetic-boundary':null,sourceIds:['stream-source'],dependencyIds:['stream-dependency']});
 const pairs=Math.floor((count-3)/2),split=Math.floor(pairs/2);op.events=[event('stream-guard','alt','Caller condition')];
 for(let i=0;i<pairs;i++){
  if(i===split)op.events.push(event('stream-else','else','Caller alternative'));
  const remote='provider-'+(i%10);
  op.events.push(event('stream-request-'+i,'declared-boundary','Request '+i,'service',remote),event('stream-return-'+i,'declared-boundary-return','Normal return '+i,remote,'service'));
 }
 if(op.events.length<count-1)op.events.push(event('stream-note','note','Source-only detail retained outside the arrow projection'));
 op.events.push(event('stream-end','end',''));
 data.operationExternalCalls={[op.id]:[]};data.operationSources={[op.id]:{'stream-source':{...Object.values(data.sources)[0],id:'stream-source',text:'OWNED_SYNTHETIC_STREAM_SOURCE'}}};
 return data;
}
if(process.env.CODECLEW_LONG_SEQUENCE_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_LONG_SEQUENCE_FIXTURE_OUT);fs.mkdirSync(path.dirname(output),{recursive:true});fs.writeFileSync(output,renderReaderFixture(longSequenceFixture()));
}
function longHeaderSequenceFixture(language='en'){
 const data=longSequenceFixture();data.language=language;
 data.operations[0].participants=data.operations[0].participants.map((participant,index)=>({...participant,
  label:index===0?'Synthetic request coordination service':index%2?'External reservation verification adapter':'Synthetic account reconciliation provider',
  ...(participant.externalService?{externalService:'synthetic-external-reservation-destination-'+index+'-identifier'}:{})
 }));
 return data;
}
if(process.env.CODECLEW_SEQUENCE_HEADER_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_SEQUENCE_HEADER_FIXTURE_OUT);fs.mkdirSync(path.dirname(output),{recursive:true});
 fs.writeFileSync(output,renderReaderFixture(longHeaderSequenceFixture(process.env.CODECLEW_SEQUENCE_HEADER_FIXTURE_LANGUAGE||'en')));
}
test('long participant headers wrap readable titles IDs and localized status with separate baselines',()=>{
 for(const language of ['en','ru'])for(const width of [390,1280]){
  const data=longHeaderSequenceFixture(language),before=JSON.stringify(data),reader=load(data);
  const layout=JSON.parse(reader.run(`JSON.stringify(sequenceLayout(D.operations[0],${width}))`));
  assert.equal(layout.actors.length,11);assert.equal(layout.actorLineHeight,18);
  for(const actor of layout.actors){
   const original=data.operations[0].participants.find(p=>p.id===actor.id);
   assert.equal(actor.titleLines.join(' '),original.label);
   const technical=original.externalService||original.service;
   assert.equal(actor.technicalLines.join('').replaceAll(' ',''),technical.replaceAll(' ',''));
   const titleBottom=actor.titleY+(actor.titleLines.length-1)*layout.actorLineHeight;
   const technicalBottom=actor.technicalY+(actor.technicalLines.length-1)*layout.actorLineHeight;
   assert.ok(actor.technicalY-titleBottom>=24);
   assert.ok(layout.headerBottom-technicalBottom>=12);
  }
  assert.ok(layout.rows[0].y>layout.headerBottom);
  assert.equal(reader.run('JSON.stringify(D)'),before);
 }
});
test('long admitted sequences retain explicit arrows guards sources and immutable data at 65 160 and 512 events',()=>{
 for(const count of [65,160,512])for(const mobile of [false,true]){
  const data=longSequenceFixture(count),op=data.operations[0],before=JSON.stringify(data),r=load(data,mobile);
  assert.equal(op.events.length,count);
  const html=r.run('overviewDiagram(D.operations[0])');assert.match(html,/class="sequence-svg"/);assert.doesNotMatch(html,/bounded overview|NaN|Infinity/);
  for(const e of op.events.filter(e=>e.kind.startsWith('declared-boundary')||e.kind==='alt'||e.kind==='else')){
   const attribute=e.kind.startsWith('declared-boundary')?'data-boundary-declaration="'+r.run(`boundaryEvidenceId(D.operations[0],${JSON.stringify(e)})`):'data-event="'+e.id;
   assert.ok(html.includes(attribute+'"'),e.id);
  }
  for(const width of [390,1440]){
   const layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(D.operations[0],${width}))`));assert.equal(layout.W,Math.max(width,11*240+32));assert.equal(layout.actors.length,11);assert.ok(layout.actors.every(actor=>actor.width>=100));assert.ok(Number.isFinite(layout.H)&&layout.H>0);
   assert.ok(layout.actors.every(a=>layout.xs[a.id]>=0&&layout.xs[a.id]<=layout.W));
   assert.equal(layout.rows.filter(row=>row.e.kind.startsWith('declared-boundary')).length,op.events.filter(e=>e.kind.startsWith('declared-boundary')).length);
  }
  r.click({event:'stream-guard',sequenceEvent:'true'});assert.match(r.e('source-code').innerHTML,/OWNED_SYNTHETIC_STREAM_SOURCE/);
  assert.equal(r.run('JSON.stringify(D)'),before);
 }
});
test('external destination wording does not assert an address and logical ports do not request wire addresses',()=>{
 for(const kind of ['http','logical'])for(const addresses of [[],[{url:'https://example.invalid/owned',source:'synthetic',environment:'test'}]]){
  if(kind==='logical'&&addresses.length)continue;
  const data=boundaryLayoutFixture('ru');data.operationExternalCalls[data.operations[0].id][0].transport={kind};data.operationExternalCalls[data.operations[0].id][0].addresses=addresses;
  const r=load(data),html=r.run('externalCallDetails(D.operations[0],D.operations[0].events[0])');
  assert.match(html,/Получатель задан декларацией/);assert.doesNotMatch(html,/Адрес задан в декларации/);
  if(kind==='logical'){assert.match(html,/Физический транспорт и адрес этого логического порта неизвестны/);assert.doesNotMatch(html,/Задайте addresses/);}
  else if(addresses.length){assert.match(html,/https:\/\/example\.invalid\/owned/);assert.doesNotMatch(html,/Адрес не указан/);}
  else assert.match(html,/Адрес для этой связи не сохранён/);
 }
});
test('documented events render as localized, evidence-linked pseudocode while the sequence stays available',()=>{
 const data=behaviorFixture(),en=load(data),html=en.e('scenario-content').innerHTML;
 assert.equal((html.match(/class="diagram-card behavior-pseudocode"/g)||[]).length,1);
 assert.match(html,/If<\/code><div class="pseudocode-text clew-markdown"><p>the quantity is positive:/);
 assert.match(html,/Loop<\/code><div class="pseudocode-text clew-markdown"><p>For each supplied reservation:/);
 assert.match(html,/Reservation service → Reservation store: Apply &lt;script&gt;alert\(1\)&lt;\/script&gt;/);
 assert.doesNotMatch(html,/<script>alert\(1\)<\/script>/);
 assert.match(html,/Otherwise/);
 assert.match(html,/When<\/code><div class="pseudocode-text clew-markdown"><p>an audit note is supplied:/);
 assert.match(html,/Return<\/code><div class="pseudocode-text clew-markdown"><p>Reservation service → Caller: the accepted result/);
 assert.match(html,/style="--indent:40px;--indent-mobile:26px"/);
 assert.match(html,/style="--indent:200px;--indent-mobile:130px"><code class="pseudocode-keyword">Declared interaction<\/code><div class="pseudocode-text clew-markdown"><p>\(Reservation service → Reservation store\) Provider submission remains a declaration/);
 assert.match(html,/<div class="behavior-pseudocode-line" role="listitem" style="--indent:0px;--indent-mobile:0px"><code class="pseudocode-keyword">Return<\/code><div class="pseudocode-text clew-markdown"><p>Reservation service → Caller: the accepted result/);
 assert.match(html,/data-sources="rejected&amp;guard" data-source-operation="reserve-fixture"/);
 assert.match(html,/<details class="sequence-details"><summary>Sequence diagram<\/summary>/);
 assert.match(html,/class="sequence-svg"/);
 assert.doesNotMatch(html,/<details class="sequence-details" open/);
 assert.match(html,/<details class="implementation-detail"><summary>Implementation details and source commentary<\/summary>/);
 assert.doesNotMatch(html,/<details class="implementation-detail" open/);
 assert.match(html,/Additional authored explanation/);
 const sourceIndex=html.indexOf('the quantity is positive'),returnIndex=html.indexOf('the conflict response'),otherwiseIndex=html.indexOf('the accepted result');
 assert.ok(sourceIndex<returnIndex&&returnIndex<otherwiseIndex);
 en.click({sources:'rejected&guard',sourceOperation:'reserve-fixture'});
 assert.match(en.e('source-code').innerHTML,/Fixture source for rejected&amp;guard/);

 const ru=load({...data,language:'ru'}),ruHtml=ru.e('scenario-content').innerHTML;
 assert.match(ruHtml,/Описание поведения/);
 assert.match(ruHtml,/>Если<\/code>/);
 assert.match(ruHtml,/>Цикл<\/code>/);
 assert.match(ruHtml,/>Иначе<\/code>/);
 assert.match(ruHtml,/>При условии<\/code>/);
 assert.match(ruHtml,/Подтверждение шага/);
 assert.match(ruHtml,/<summary>Диаграмма последовательности<\/summary>/);
});
function externalCallFixture(){
 const data=behaviorFixture(),operation=data.operations[0],event=operation.events.find(event=>event.kind==='message');
 data.subject='service:sample';
 const outgoing={id:'contract-outgoing',sourceIds:['outgoing-contract'],normalized:{method:'POST',path:'/external-reservations',sourceMapping:'EXPLICIT_DECLARATION',boundaries:[],parameters:[],operation:{requestBody:{required:true,content:{'application/json':{schema:{type:'object',properties:{quantity:{type:'integer'}},required:['quantity']}}}},responses:{201:{description:'External reservation created',content:{'application/json':{schema:{type:'object',properties:{reservationId:{type:'string'}}}}}}}},servers:[{url:'https://contract-server.example.invalid'}],security:[],securitySchemes:{}}};
 const incoming={...outgoing,id:'contract-incoming',normalized:{...outgoing.normalized,path:'/incoming-orders',entrypoint:operation.id,operation:{responses:{200:{description:'Incoming caller operation'}}}}};
 data.contracts=[incoming];data.operationContracts={[operation.id]:[incoming]};
 data.operationSources[operation.id]={...data.operationSources[operation.id],'outgoing-contract':source('SAVED EXTERNAL CONTRACT')};
 data.sources['outgoing-contract']=source('CURRENT DIFFERENT CONTRACT');
 data.operationExternalCalls={[operation.id]:[{eventId:event.id,interactionId:'inventory-reservation',to:'inventory',title:'Reserve external inventory',addresses:[{url:'https://inventory.example.invalid/v1',environment:'staging',source:'Deployment inventory'}],declaration:{origin:'human',rationale:'The operator declared this outgoing binding.'},applicability:{environments:['staging']},bindingStatus:'EXPLICIT_INTERACTION',contractStatus:'EXPLICIT_SAVED_CONTRACT',contract:outgoing,snapshotContextDigest:'saved-snapshot',retained:false,targetChanged:false}]};
 return data;
}
test('ordinary message step displays a declared address and its exact outgoing saved contract',()=>{
 const data=externalCallFixture(),r=load(data),html=r.e('scenario-content').innerHTML,operation=data.operations[0];
 assert.match(html,/external-call-details/);assert.match(html,/https:\/\/inventory\.example\.invalid\/v1/);
 assert.match(html,/Environment: staging/);assert.match(html,/Provided from: Deployment inventory/);
 assert.match(html,/<summary>Contract · POST \/external-reservations<\/summary>/);
 assert.match(html,/quantity/);assert.match(html,/External reservation created/);assert.match(html,/reservationId/);
 assert.doesNotMatch(html,/Incoming caller operation/);assert.doesNotMatch(html,/href="https:\/\/inventory/);
 r.click({sources:'outgoing-contract',sourceOperation:operation.id});
 assert.match(r.e('source-code').innerHTML,/SAVED EXTERNAL CONTRACT/);assert.doesNotMatch(r.e('source-code').innerHTML,/CURRENT DIFFERENT CONTRACT/);
 r.click({tab:'contract'});assert.match(r.e('scenario-content').innerHTML,/Incoming caller operation/);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/External reservation created/);
 const ru=load({...data,language:'ru'});assert.match(ru.e('scenario-content').innerHTML,/Внешний сервис/);assert.match(ru.e('scenario-content').innerHTML,/<summary>Контракт · POST/);
});
test('sequence SVG omits long Notes while readable behavior retains nested note-only outcomes and their sources',()=>{
 const data=fixture(),op=data.operations[0];
 const longNote='The common envelope requires an object, string id, version 1, a place with id oslo, helsinki, or lisbon, string name, and finite latitude and longitude. The forecast branch requires updatedAt and fetchedAt accepted by isTimestamp, mode fixture or live, and a periods array of length 1 through 6. Each period requires a valid time, finite temperatureC, precipitationMm null or finite and nonnegative, and a string symbol. isTimestamp accepts a UTC ISO-like value with seconds and optional fractional seconds, and also requires Date.parse to be finite.';
 op.participants=[{id:'caller',label:'Caller'},{id:'dashboard',label:'dashboard'}];
 const event=(id,kind,text)=>({id,kind,text,from:'caller',to:'dashboard',sourceIds:['same-source'],dependencyIds:['dep']});
 op.events=[event('message','message','Consume forecast'),event('envelope','note',longNote),event('outer','alt','Valid envelope'),event('inner','opt','Existing card'),event('note-only','note','A newer stored timestamp leaves the card unchanged.'),event('inner-end','end',''),event('otherwise','else','Rejected envelope'),event('rejection-note','note','A rejection-write or rejection-path offset-commit failure propagates.'),event('outer-end','end',''),event('result','return','Return result')];
 const original=JSON.stringify(op),r=load(data),svg=r.run('diagram(D.operations[0])');
 assert.doesNotMatch(svg,/The common envelope requires|newer stored timestamp|rejection-write/);
 assert.doesNotMatch(svg,/data-event="(?:envelope|note-only|rejection-note)"/);
 for(const id of ['message','result'])assert.ok(svg.includes(`data-event="${id}"`));
 assert.doesNotMatch(svg,/data-event="(?:outer|inner|otherwise)"/);
 assert.equal((svg.match(/class="step-number"/g)||[]).length,2);
 assert.match(svg,/>2<\/text>/);
 const behavior=r.run('behaviorPseudocode(D.operations[0])');
 assert.ok(behavior.includes(longNote));assert.match(behavior,/newer stored timestamp|rejection-write/);
 assert.equal(r.run('JSON.stringify(D.operations[0])'),original);
 r.click({sources:'same-source',sourceOperation:op.id});
 assert.match(r.e('source-code').innerHTML,/ACCEPTED SOURCE/);
});

test('sequence projection prunes empty nested arms and retains surviving else guards without changing accepted evidence',()=>{
 const data=fixture(),op=data.operations[0];
 const event=(id,kind,text)=>({id,kind,text,from:'caller',to:'dashboard',sourceIds:['same-source'],dependencyIds:['dep-'+id]});
 op.participants=[{id:'caller',label:'Caller'},{id:'dashboard',label:'Dashboard'}];
 op.events=[event('guard','alt','A duplicate was already processed'),event('empty-loop','loop','For every duplicate'),event('empty-inner','opt','A stored timestamp is newer'),event('note','note','Keep the current value'),event('inner-end','end',''),event('loop-end','end',''),event('surviving-else','else','For a new event'),event('write','message','Apply the forecast'),event('outer-end','end','')];
 const before=JSON.stringify(data),r=load(data),projected=JSON.parse(r.run('JSON.stringify(sequenceEvents(D.operations[0]))'));
 assert.deepEqual(projected.map(e=>e.id),['surviving-else','write','outer-end']);
 assert.equal(projected[0].kind,'alt');assert.equal(projected[0].text,'Condition not met: A duplicate was already processed · Selected branch: For a new event');
 assert.deepEqual(projected[1],op.events.find(e=>e.id==='write'));
 const svg=r.run('diagram(D.operations[0])');assert.doesNotMatch(svg,/data-event="(?:guard|empty-loop|empty-inner|note)"/);assert.match(svg,/data-event="surviving-else"/);
 const behavior=r.run('behaviorPseudocode(D.operations[0])');for(const text of ['A duplicate was already processed','A stored timestamp is newer','Keep the current value','For a new event','Apply the forecast'])assert.ok(behavior.includes(text));
 assert.equal(JSON.stringify(data),before);assert.equal(r.run('JSON.stringify(D.operations[0])'),JSON.stringify(op));
 r.click({sources:'same-source',sourceOperation:op.id});assert.match(r.e('source-code').innerHTML,/ACCEPTED SOURCE/);
 const both={...op,events:[event('both','alt','First condition'),event('left','return','First result'),event('else','else','Other condition'),event('right','declared','Second result'),event('end','end','')]};
 assert.deepEqual(JSON.parse(r.run(`JSON.stringify(sequenceEvents(${JSON.stringify(both)}))`)),both.events);
 const emptyElse={...op,events:[event('first','alt','First condition'),event('left','message','First result'),event('empty-else','else','Other condition'),event('empty-note','note','No action'),event('end','end','')]};
 assert.deepEqual(JSON.parse(r.run(`JSON.stringify(sequenceEvents(${JSON.stringify(emptyElse)}))`)).map(e=>e.id),['first','left','end']);
});

test('surviving SVG else guard opens combined guard and else evidence without changing the accepted stream',()=>{
 const data=fixture(),op=data.operations[0];
 op.participants=[{id:'caller',label:'Caller'},{id:'store',label:'Store'}];
 op.events=[
  {id:'guard',kind:'alt',text:'The event was already processed',sourceIds:['guard-source'],dependencyIds:['guard-dependency']},
  {id:'note',kind:'note',text:'Return the previous result',sourceIds:['note-source'],dependencyIds:[]},
  {id:'else',kind:'else',text:'Otherwise',sourceIds:['else-source'],dependencyIds:['else-dependency']},
  {id:'write',kind:'message',text:'Apply the event',from:'caller',to:'store',sourceIds:['write-source'],dependencyIds:[]},
  {id:'end',kind:'end',text:'',sourceIds:[],dependencyIds:[]}
 ];
 data.operationSources[op.id]={...data.operationSources[op.id],'guard-source':source('GUARD_SOURCE'),'else-source':source('ELSE_SOURCE'),'note-source':source('NOTE_SOURCE'),'write-source':source('WRITE_SOURCE')};
 const before=JSON.stringify(data),r=load(data),projected=JSON.parse(r.run('JSON.stringify(sequenceEvents(D.operations[0]))'));
 r.run('showEntry(D.operations[0].id)');
 assert.deepEqual(projected[0].sourceIds,['guard-source','else-source']);assert.deepEqual(projected[0].dependencyIds,['guard-dependency','else-dependency']);
 assert.match(r.run('diagram(D.operations[0])'),/data-event="else" data-sequence-event="true"/);
 r.click({event:'else',sequenceEvent:'true'});
 assert.match(r.e('source-code').innerHTML,/GUARD_SOURCE/);
 assert.match(r.e('source-select').innerHTML,/value="guard-source"/);assert.match(r.e('source-select').innerHTML,/value="else-source"/);assert.doesNotMatch(r.e('source-select').innerHTML,/note-source/);
 r.e('source-select').onchange({target:{value:'else-source'}});assert.match(r.e('source-code').innerHTML,/ELSE_SOURCE/);
 assert.ok(r.e('source-title').textContent.includes('The event was already processed'));
 assert.equal(JSON.stringify(data),before);assert.equal(r.run('JSON.stringify(D.operations[0])'),JSON.stringify(op));
 const behavior=r.run('behaviorPseudocode(D.operations[0])');assert.match(behavior,/Return the previous result/);assert.match(behavior,/The event was already processed/);
});

test('sequence participants follow only visible arrows in their original order, including self, return and declared messages',()=>{
 const data=fixture(),op=data.operations[0],r=load(data);
 const event=(id,kind,text,from,to)=>({id,kind,text,from,to,sourceIds:['same-source'],dependencyIds:['dep-'+id]});
 op.participants=[{id:'caller',label:'Unused caller'},{id:'second',label:'Second'},{id:'notes',label:'Notes only'},{id:'first',label:'First'}];
 op.events=[event('guard','alt','Original condition','caller','notes'),event('note','note','No visible action','notes','caller'),event('alternative','else','Otherwise','caller','notes'),event('self','message','Self message','second','second'),event('reply','return','Return result','second','first'),event('declared','declared','Declared transfer','first','second'),event('end','end','')];
 const original=JSON.stringify(op),layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(op)}))`));
 assert.deepEqual(layout.actors.map(a=>a.id),['second','first']);assert.deepEqual(Object.keys(layout.xs),['second','first']);assert.equal(layout.W,800);
 const svg=r.run(`diagram(${JSON.stringify(op)})`);assert.doesNotMatch(svg,/Unused caller|Notes only|NaN|Infinity/);for(const id of ['self','reply','declared'])assert.ok(svg.includes(`data-event="${id}"`));
 assert.equal((svg.match(/class="lifeline"/g)||[]).length,2);assert.equal(JSON.stringify(op),original);
 const self={...op,events:[event('only','message','Self message','second','second')]};assert.deepEqual(JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(self)}))`)).actors.map(a=>a.id),['second']);assert.doesNotMatch(r.run(`diagram(${JSON.stringify(self)})`),/NaN|Infinity/);
});

test('notes-only sequence keeps readable behavior and original participants without an empty SVG scene',()=>{
 const data=fixture(),op=data.operations[0],r=load(data);
 op.overviewDiagram=null;op.participants=[{id:'caller',label:'Caller'}];
 op.events=[{id:'guard',kind:'alt',text:'Original condition',sourceIds:['same-source']},{id:'note',kind:'note',text:'Keep the current value',from:'caller',to:'caller',sourceIds:['same-source']},{id:'end',kind:'end',text:'',sourceIds:[]}];
 const original=JSON.stringify(op),layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(op)}))`));
 assert.deepEqual(layout.actors,[]);assert.deepEqual(layout.xs,{});assert.deepEqual(layout.rows,[]);assert.ok(Number.isFinite(layout.W)&&Number.isFinite(layout.H));
 assert.equal(r.run(`diagram(${JSON.stringify(op)})`),'');const html=r.run(`sequence({id:${JSON.stringify(op.id)}},${JSON.stringify(op)})`);
 assert.doesNotMatch(html,/sequence-svg|overview-scroll|sequence-details|NaN|Infinity/);assert.match(html,/Keep the current value/);assert.match(html,/Original condition/);assert.match(html,/data-sources="same-source"/);
 assert.equal(JSON.stringify(op),original);assert.equal(r.run(`diagram(${JSON.stringify({...op,participants:[],events:[]})})`),'');
});

test('sequence layout keeps readable lanes and pans within narrow containers without shrinking typography',()=>{
 const data=fixture(),r=load(data),op={...data.operations[0],participants:[{id:'caller',label:'Unused caller'},{id:'dashboard',label:'Dashboard service with a long name'},{id:'kafka',label:'Kafka topic weather.forecast-updated'},{id:'db',label:'Dashboard PostgreSQL'}],events:[]};
 const event=(id,kind,text,from,to)=>({id,kind,text,from,to,sourceIds:['same-source'],dependencyIds:['dep']});
 op.events=[event('guard','alt','The inbox insert reports an existing event id.'),event('empty','note','Return DUPLICATE'),event('else','else','Otherwise'),event('apply','message','For a new forecast event, insert the card from event.place when no card exists; update only when the incoming timestamp is strictly newer.','dashboard','db'),event('self','message','Self-message labels remain complete at the rightmost participant, including a-long-unbroken-identifier-with-unicode-π🙂.','db','db'),event('reply','return','Return the saved result after commit.','db','kafka'),event('declared','declared','Declared delivery remains unverified.','kafka','dashboard'),event('end','end','')];
 const original=JSON.stringify(op);
 for(const width of [224,280,320,375,560,610,1280]){
  const layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(op)},${width}))`));assert.equal(layout.W,Math.max(width,3*240+32));assert.deepEqual(layout.actors.map(a=>a.id),['dashboard','kafka','db']);
  for(const actor of layout.actors){assert.ok(layout.xs[actor.id]-actor.width/2>=0);assert.ok(layout.xs[actor.id]+actor.width/2<=layout.W);assert.equal(actor.titleLines.join('').replace(/ /g,''),actor.label.replace(/ /g,''));}
  for(const row of layout.rows.filter(row=>['message','return','declared'].includes(row.e.kind))){assert.equal(row.lines.join('').replace(/ /g,''),row.e.text.replace(/ /g,''));assert.ok(row.mid-row.labelWidth/2>=0);assert.ok(row.mid+row.labelWidth/2<=layout.W);assert.ok(row.arrowOffset>(row.lines.length-1)*14+10);if(row.e.kind==='declared'){assert.equal(row.captionLines.join('').replace(/ /g,''),'Declared service interaction'.replace(/ /g,''));assert.ok(row.h>row.captionOffset+(row.captionLines.length-1)*14);}if(row.e.from===row.e.to){const end=layout.xs[row.e.from]+row.direction*row.loopWidth;assert.ok(end>=0&&end<=layout.W);}}
  const svg=r.run(`diagram(${JSON.stringify(op)},${width})`);assert.doesNotMatch(svg,/NaN|Infinity/);assert.ok(svg.includes(`min-width:${layout.W}px`));assert.ok(svg.includes(`viewBox="0 0 ${layout.W} `));assert.equal((svg.match(/data-event=/g)||[]).length,5);
 }
 assert.equal(JSON.stringify(op),original);
});

test('sequence reflow skips hidden disclosure widths and updates on reveal and subsequent resize',()=>{
 const data=behaviorFixture(),r=load(data),original=JSON.stringify(data.operations[0]);
 const setup=`globalThis.reflowWidth=0;globalThis.reflowResult='';globalThis.reflowParent={get clientWidth(){return reflowWidth;}};globalThis.reflowSvg={parentElement:reflowParent,getAttribute(name){return name==='data-sequence-operation'?D.operations[0].id:'800';},set outerHTML(value){reflowResult=value;}};document.querySelectorAll=()=>[reflowSvg];window.getComputedStyle=()=>({paddingLeft:'8px',paddingRight:'8px',fontFamily:'sans-serif'});`;
 r.run(setup);r.run('reflowSequences()');assert.equal(r.run('reflowResult'),'');
 r.run('reflowWidth=610;reflowSequences()');assert.match(r.run('reflowResult'),/data-layout-width="594"/);
 r.run('reflowWidth=320;scheduleSequenceReflow()');assert.match(r.run('reflowResult'),/data-layout-width="304"/);
 r.run('reflowWidth=1280;scheduleSequenceReflow()');assert.match(r.run('reflowResult'),/data-layout-width="1264"/);
 assert.equal(r.run('JSON.stringify(D.operations[0])'),original);
});

test('sequence layout places arrows below complete Weather labels and bounds participant titles',()=>{
 const op={"id": "dashboard-forecast-updated-consumer", "title": "Dashboard consumes forecast-updated events", "participants": [{"id": "caller", "label": "Caller", "service": null}, {"id": "service-dashboard", "label": "dashboard", "service": "dashboard"}, {"id": "dashboard-kafka", "label": "Kafka topic weather.forecast-updated", "service": null}, {"id": "dashboard-db", "label": "Dashboard PostgreSQL", "service": null}], "events": [{"dependencyIds": ["dashboard:symbol:ac3aadb76be6fea3e790cb85"], "from": "service-dashboard", "id": "claim-ebeef3c264171b01cd39", "kind": "message", "sourceIds": ["dashboard-ac3aadb76be6fea3e790"], "text": "For a new forecast event, insert the card from event.place when no card exists, without checking for a prior place-requested event; for an existing card, update its place fields and forecast only when stored updated_at is null or the incoming timestamp is strictly newer.", "to": "dashboard-db"}, {"dependencyIds": ["dashboard:flow:59f03a0653cc7a3798b8ceaf", "dashboard:flow:a190efb9bca6603f1d2abc58", "dashboard:symbol:021fcd0ff7baa53bd3c51930"], "from": "service-dashboard", "id": "claim-0b1e97374ab832082b1d", "kind": "message", "sourceIds": ["dashboard-021fcd0ff7baa53bd3c5", "dashboard-59f03a0653cc7a3798b8", "dashboard-a190efb9bca6603f1d2a"], "text": "On JSON parse or validation failure, including an unrecognized topic that throws UNKNOWN_TOPIC, insert topic, partition, offset, and reason INVALID_EVENT into rejected_messages; ignore a duplicate rejection row.", "to": "dashboard-db"}, {"dependencyIds": ["dashboard:flow:049a97e649d7feb212e6c67f", "dashboard:flow:654eeebd7e0618b0c6dd58f9", "dashboard:flow:a190efb9bca6603f1d2abc58"], "from": "service-dashboard", "id": "claim-2bc228fbe1d00118816c", "kind": "message", "sourceIds": ["dashboard-049a97e649d7feb212e6", "dashboard-654eeebd7e0618b0c6dd", "dashboard-a190efb9bca6603f1d2a"], "text": "After the rejection row is written, commit this partition's offset plus one and return.", "to": "dashboard-kafka"}]};
 const data=fixture(),r=load(data),original=JSON.stringify(op);
 const layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(op)}))`));
 assert.equal(layout.rows.length,3);
 for(const row of layout.rows){assert.ok(row.lines.length>1);assert.equal(row.lines.join('').replace(/ /g,''),row.e.text.replace(/ /g,''));assert.ok(row.arrowOffset>(row.lines.length-1)*14+10);assert.ok(row.h>row.arrowOffset+20);}
 const kafka=layout.actors.find(p=>p.id==='dashboard-kafka');
 assert.ok(kafka.titleLines.length>1);assert.ok(kafka.titleLines.every(line=>Array.from(line).length<=32));
 assert.equal(kafka.titleLines.join('').replace(/ /g,''),kafka.label.replace(/ /g,''));
 assert.ok(layout.rows[0].y>layout.headerBottom);
 assert.equal(JSON.stringify(op),original);
 const event=(id,kind,text)=>({id,kind,text,from:'caller',to:'caller',sourceIds:[],dependencyIds:[]});
 const nested={...op,events:[event('outer','alt','outer '.repeat(70)),event('inner','opt','inner '.repeat(70)),event('self','message','self '.repeat(70)),event('inner-end','end',''),event('else','else','otherwise '.repeat(70)),event('declared','declared','declared '.repeat(70)),event('outer-end','end','')]};
 const n=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(nested)}))`));
 assert.deepEqual(n.frames.map(f=>f.depth),[0,1]);
 for(const row of n.rows){if(row.lines.length)assert.ok(row.h>row.lines.length*14);if(['message','declared'].includes(row.e.kind)){assert.ok(row.h>row.arrowOffset+17);assert.ok(row.arrowOffset>(row.lines.length-1)*14+10);}}
 const declared=n.rows.find(row=>row.e.kind==='declared');assert.ok(declared.captionOffset>declared.arrowOffset+17);assert.ok(declared.h>declared.captionOffset);
 const svg=r.run(`diagram(${JSON.stringify(nested)})`);for(const id of ['outer','inner','self','else','declared'])assert.ok(svg.includes(`data-event="${id}"`));
});
test('mapped contract anchors open the existing handler contract without duplicate rows or retained-source substitution',()=>{
 const data=externalCallFixture(),operation=data.operations[0],current=data.contracts[0],sourceId='same-contract-source';
 data.catalogue=[{id:operation.id,symbol:'savePlace',kind:'HTTP_ENDPOINT',trigger:{methods:['POST'],paths:['/api/places']},sourceIds:['same-source']}];
 data.sections=[{id:'section-overview',title:'Overview',gap:'No overview'}];
 current.sourceIds=[sourceId];current.normalized.sourceMapping='SOURCE_ROUTE_MATCH_ONLY';
 current.normalized.path='/api/places';current.normalized.operation={responses:{200:{description:'SELECTED CONTRACT'}}};
 data.sources[sourceId]=source('ORIGINAL PUBLICATION CONTRACT SOURCE');
 data.sources['cross-service-source']=source('ORIGINAL CROSS SERVICE SOURCE');
 data.selectedSources={[sourceId]:source('SELECTED CONTRACT SOURCE')};
 data.operationSources[operation.id][sourceId]=source('RETAINED CONTRACT SOURCE');
 data.operationContracts[operation.id]=[{...current,normalized:{...current.normalized,entrypoint:null,sourceMapping:'NO_SOURCE_ROUTE_MATCH'}}];
 operation.interfaceContracts=[{id:'retained-description',kind:'http',title:'Retained authored description',rows:[{label:'Request',value:'RETAINED AUTHOR PROSE',sourceIds:['same-source']}],boundaries:[]}];
 const r=load(data);
 assert.equal(r.run(`entries.some(e=>e.id===${JSON.stringify(current.id)})`),false);
 r.run(`location.hash='#'+${JSON.stringify(current.id)};routeReaderHash()`);
 assert.equal(r.run('current.id'),operation.id);assert.equal(r.run('tab'),'contract');
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Complete operation contract/);assert.match(html,/SOURCE_ROUTE_MATCH_ONLY/);
 assert.match(html,/RETAINED AUTHOR PROSE/);assert.match(html,/SELECTED CONTRACT/);
 assert.match(html,/data-current-sources/);
 r.click({sources:sourceId,sourceOperation:operation.id},true);
 assert.match(r.e('source-code').innerHTML,/SELECTED CONTRACT SOURCE/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/RETAINED CONTRACT SOURCE/);
 r.click({sources:sourceId,sourceOperation:operation.id});
 assert.match(r.e('source-code').innerHTML,/RETAINED CONTRACT SOURCE/);
 r.e('scenario-content').hidden=true;
 r.run("source(['cross-service-source'],'Retained cross-service evidence')");
 assert.match(r.e('source-code').innerHTML,/ORIGINAL CROSS SERVICE SOURCE/);
 r.run(`showEntry(${JSON.stringify(operation.id)})`);assert.equal(r.run('tab'),'sequence');
 r.run("location.hash='#unknown-contract';routeReaderHash()");assert.equal(r.run('current.id'),'section-overview');
});
test('mapped contract navigation refuses ambiguous captured contract identities',()=>{
 const data=externalCallFixture(),contract=data.contracts[0],operation=data.operations[0];
 data.catalogue=[{id:operation.id,symbol:'handler',kind:'HTTP_ENDPOINT',trigger:{methods:['POST'],paths:['/api/places']},sourceIds:[]}];
 data.sections=[{id:'section-overview',title:'Overview',gap:'No overview'}];
 contract.normalized.sourceMapping='SOURCE_ROUTE_MATCH_ONLY';
 data.contracts.push({...contract,normalized:{...contract.normalized,entrypoint:'unknown-handler'}});
 const r=load(data);r.run(`location.hash='#'+${JSON.stringify(contract.id)};routeReaderHash()`);
 assert.equal(r.run('current.id'),'section-overview');
});
test('full Pages keeps selected revisions separate from retained explanation coverage',()=>{
 const data=fixture();data.selectedSnapshot='selected-source-snapshot';data.frozenPublication='frozen-publication';
 data.revisions={sample:'retained-revision'};data.coverage={sample:'OLD_PARTIAL'};
 data.selectedEvidence={authority:'SELECTED_SNAPSHOT_NOT_RUNTIME_PROOF',revisions:{sample:'selected-revision'},services:{sample:{coverage:'SYNTAX',boundaries:['SELECTED_BOUNDARY']}}};
 const r=load(data);
 assert.match(r.e('revision-links').innerHTML,/selected-rev/);assert.doesNotMatch(r.e('revision-links').innerHTML,/retained-rev/);
 assert.match(r.e('freshness-status').innerHTML,/Selected source snapshot/);
 assert.match(r.e('freshness-status').innerHTML,/retained-revision/);
 r.run('coverage()');const html=r.e('coverage-view').innerHTML;
 assert.match(html,/selected-revision/);assert.match(html,/retained-revision/);
 assert.match(html,/SYNTAX/);assert.match(html,/OLD_PARTIAL/);assert.match(html,/SELECTED_BOUNDARY/);
 assert.equal(data.revisions.sample,'retained-revision');
 r.run('D.sectionState=null;freshness(null)');
 assert.match(r.e('freshness-status').innerHTML,/Selected source snapshot/);
 assert.match(r.e('freshness-status').innerHTML,/retained-revision/);
 assert.match(r.e('freshness-status').innerHTML,/Legacy publication/);
});
test('external call missing and multiple declarations remain explicit; old pages do not infer bindings',()=>{
 const data=externalCallFixture(),operation=data.operations[0],row=data.operationExternalCalls[operation.id][0];
 delete row.addresses[0].environment;assert.match(load(data).e('scenario-content').innerHTML,/Environment: unspecified/);
 row.addresses=[];row.contract=null;row.contractStatus='REFERENCE_UNAVAILABLE';row.bindingStatus='MULTIPLE_DECLARED_INTERACTIONS';
 let html=load(data).e('scenario-content').innerHTML;
 assert.match(html,/No address is retained/);assert.match(html,/Remote wire contract is not established/);assert.match(html,/REFERENCE_UNAVAILABLE/);assert.doesNotMatch(html,/Set contractReference|CONTRACT_OPERATION ID/);assert.match(html,/Multiple declarations match/);
 assert.doesNotMatch(html,/Incoming caller operation/);
 row.contractStatus='AMBIGUOUS_RECEIVER_CONTRACT';
 html=load(data).e('scenario-content').innerHTML;assert.match(html,/AMBIGUOUS_RECEIVER_CONTRACT/);assert.doesNotMatch(html,/Incoming caller operation/);
 row.contractStatus='NOT_BOUND';row.targetChanged=true;row.retained=true;
 html=load(data).e('scenario-content').innerHTML;assert.match(html,/Remote wire contract is not established/);assert.match(html,/earlier accepted operation/);
 delete data.operationExternalCalls;
 html=load(data).e('scenario-content').innerHTML;assert.doesNotMatch(html,/external-call-details/);assert.doesNotMatch(html,/inventory\.example/);
});
test('external destination and contract strings remain inert escaped data',()=>{
 const data=externalCallFixture(),row=data.operationExternalCalls[data.operations[0].id][0];
 row.addresses=[{url:'javascript:alert(1)',environment:'<img src=x onerror=alert(1)>',source:'"<script>attack()</script>'},{url:'data:text/html,<script>attack()</script>',environment:'test',source:'User declaration'}];
 row.title='<script>attack()</script>';row.contract.normalized.operation.responses['201'].description='<img src=x onerror=attack()>';
 const html=load(data).e('scenario-content').innerHTML;
 assert.match(html,/javascript:alert\(1\)/);assert.match(html,/&lt;script&gt;attack\(\)&lt;\/script&gt;/);
 assert.doesNotMatch(html,/<script>|<img src=x|href="(?:javascript:|data:)/);
});
test('ordinary step echoes display once while canonical explanations remain intact',()=>{
 const data=behaviorFixture(),operation=data.operations[0];
 for(const event of operation.events)event.dependencyIds=['documented-flow'];
 operation.explanation=operation.events.filter(event=>event.kind!=='end').map(event=>({id:`paragraph-${event.id}`,text:event.text,eventIds:[event.id],dependencyIds:[...event.dependencyIds],sourceIds:[...event.sourceIds],detail:false}));
 operation.explanation.push({...operation.explanation[0],id:'expanded-outcome',text:'The prepared result completes this request.'});
 const canonical=JSON.stringify(operation),r=load(data),html=r.e('scenario-content').innerHTML;
 assert.match(html,/class="diagram-card behavior-pseudocode"/);
 assert.match(html,/For each supplied reservation:/);
 assert.doesNotMatch(html,/<p>For each supplied reservation<\/p>/);
 assert.doesNotMatch(html,/<p>the accepted result<\/p>/);
 assert.match(html,/<p>The prepared result completes this request\.<\/p>/);
 assert.equal(r.run('JSON.stringify(D.operations[0])'),canonical);
 const noDisplay=r.run('explanation(D.operations[0])');
 assert.match(noDisplay,/<p>For each supplied reservation<\/p>/);
});
test('sequence preserves an otherwise explanation when only the localized keyword is displayed',()=>{
 const data=behaviorFixture(),operation=data.operations[0],event=operation.events.find(event=>event.id==='inner-else');
 event.dependencyIds=['alternative-flow'];
 operation.explanation=[{id:'alternative-explanation',text:event.text,eventIds:[event.id],dependencyIds:[...event.dependencyIds],sourceIds:[...event.sourceIds],detail:false}];
 const canonical=JSON.stringify(operation),en=load(data);
 assert.match(en.e('scenario-content').innerHTML,/<code class="pseudocode-keyword">Otherwise<\/code><div class="pseudocode-text clew-markdown"><\/div>/);
 assert.match(en.e('scenario-content').innerHTML,/<p>Otherwise<\/p>/);
 assert.equal(en.run('JSON.stringify(D.operations[0])'),canonical);
 const ru=load({...data,language:'ru'});
 assert.match(ru.e('scenario-content').innerHTML,/<code class="pseudocode-keyword">Иначе<\/code><div class="pseudocode-text clew-markdown"><\/div>/);
 assert.match(ru.e('scenario-content').innerHTML,/<p>Otherwise<\/p>/);
});
test('redundant-copy filtering preserves distinct evidence, thematic, authored, detail and unresolved context',()=>{
 const data=behaviorFixture(),operation=data.operations[0],event=operation.events[0];
 event.dependencyIds=['loop-flow'];
 const echo={id:'copied-loop',text:event.text,eventIds:[event.id],dependencyIds:[...event.dependencyIds],sourceIds:[...event.sourceIds],detail:false};
 operation.explanation=[echo,
  {...echo,id:'extra-dependency',dependencyIds:[...echo.dependencyIds,'extra-dependency']},
  {...echo,id:'extra-source',sourceIds:[...echo.sourceIds,'extra-source']},
  {...echo,id:'thematic',eventIds:[event.id,operation.events[1].id]},
  {...echo,id:'unresolved',eventIds:['unrendered-event']},
  {...echo,id:'missing-metadata',dependencyIds:undefined},
  {...echo,id:'authored',authorship:{author:'Synthetic editor',sourceSnapshot:'synthetic-snapshot',editDigest:'synthetic-edit'}},
  {...echo,id:'detail',detail:true},
 ];
 const canonical=JSON.stringify(operation),r=load(data),displayed='D.operations[0].events.filter(event=>event.kind!=="end")';
 const prose=r.run(`explanation(D.operations[0],false,${displayed})`),detail=r.run(`explanation(D.operations[0],true,${displayed})`);
 assert.equal((prose.match(/<p>For each supplied reservation<\/p>/g)||[]).length,6);
 assert.match(prose,/data-sources="loop-source extra-source"/);
 assert.match(prose,/User documentation by Synthetic editor/);
 assert.equal((detail.match(/<p>For each supplied reservation<\/p>/g)||[]).length,1);
 assert.equal(r.run('JSON.stringify(D.operations[0])'),canonical);
 // An overview still has displayed original labels in the HTML behavior view.
 operation.overviewDiagram={nodes:[{id:'summary',text:'Bounded overview',participant:'service',column:0,row:0,eventIds:[event.id]}],edges:[]};
 operation.explanation=[echo];
 const overview=load(data).e('scenario-content').innerHTML;
 assert.match(overview,/For each supplied reservation:/);
 assert.doesNotMatch(overview,/<p>For each supplied reservation<\/p>/);
});
test('meaning review labels identify model approval in English and Russian',()=>{
 const data=fixture();
 data.sectionState.verification='VERIFIED';
 data.operationStates['section-responsibilities']={freshness:'CURRENT',verification:'VERIFIED_WITH_LIMITATIONS'};
 const en=load(data),enBanner=en.e('freshness-status').innerHTML,enPage=en.e('scenario-content').innerHTML;
 assert.match(enBanner,/<strong>CURRENT<\/strong>/);
 assert.match(enBanner,/Meaning review: Model approved/);
 assert.match(enPage,/Meaning review: <b>Model approved with limitations<\/b>/);
 assert.match(enBanner,/Model review approves an interpretation; evidence links do not prove every statement in the prose\./);
 assert.equal(en.data.sectionState.verification,'VERIFIED');
 const ru=load({...data,language:'ru'}),ruBanner=ru.e('freshness-status').innerHTML,ruPage=ru.e('scenario-content').innerHTML;
 assert.match(ruBanner,/<strong>Актуально<\/strong>/);
 assert.match(ruBanner,/Проверка смысла: Одобрено моделью/);
 assert.match(ruPage,/Проверка смысла: <b>Одобрено моделью с оговорками<\/b>/);
 assert.match(ruBanner,/Модельное одобрение — это оценка интерпретации; ссылки на источники не доказывают каждое утверждение в тексте\./);
 assert.equal(ru.data.sectionState.verification,'VERIFIED');
 const unassessedData=fixture();unassessedData.sectionState.verification='UNASSESSED';
 assert.match(load(unassessedData).e('freshness-status').innerHTML,/Meaning review: UNASSESSED/);
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
 assert.doesNotMatch(html,/Explore internal processes/);
 assert.doesNotMatch(r.e('scenario-nav').innerHTML,/data-entry="process-catalog"/);
});
test('process catalog lists typed visuals without requiring an HTTP selection',()=>{
 const data=fixture();data.savedProcesses=[{id:'saved',title:'Saved workflow',href:'../scenarios/saved.html',status:'AUTHORED'}];
 const r=load(data);r.run("showEntry('process-catalog')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Processes and diagrams · 2/);
 assert.match(html,/<svg class="artifact-svg"/);
 assert.match(html,/<th>Selected result/);
});

test('process pages keep PlantUML downloads and text when SVG rendering is unavailable',()=>{
 const data=processFixture(false),r=load(data);r.run("showEntry('process-overview')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Declared process-state schema/);
 assert.match(html,/States and transitions come from the captured process-state schema/);
 assert.match(html,/Static source bindings do not show runtime execution/);
 assert.match(html,/PlantUML SVG preview is unavailable/);
 assert.match(html,/Entry: updateStatus/);
 assert.match(html,/Source syntax summary/);
 assert.match(html,/href="\.\.\/diagrams\/scenario-checkout-lifecycle-8a41c7d0301b2f9c\.puml"/);
 assert.doesNotMatch(html,/<img[^>]+scenario-checkout-(?:states|activity-transitions|lifecycle)/);
 assert.match(r.e('scenario-nav').innerHTML,/data-entry="approve-request"/);
 r.run("showEntry('approve-request')");
 assert.match(r.e('scenario-content').innerHTML,/Accepted process operation details/);
});

test('process SVG previews render only for returned assets and authored details override matching lifecycle output',()=>{
 const data=processFixture(true,'approve-request'),r=load(data);r.run("showEntry('process-overview')");
 let html=r.e('scenario-content').innerHTML;
 assert.equal((html.match(/class="diagram-img"/g)||[]).length,2);
 assert.doesNotMatch(html,/PlantUML SVG preview is unavailable/);
 assert.doesNotMatch(html,/Entry: updateStatus/);
 assert.doesNotMatch(html,/scenario-checkout-lifecycle-8a41c7d0301b2f9c\.puml/);
 assert.match(r.e('scenario-nav').innerHTML,/data-entry="approve-request"/);
 r.run("showEntry('approve-request')");
 assert.match(r.e('scenario-content').innerHTML,/Accepted process operation details/);
});

test('ordinary saved process exposes its exact-root outline without a state schema and opens retained source',()=>{
 const data=processFixture(false);delete data.stateDiagram;delete data.stateDiagramSvg;delete data.activityTransitions;delete data.activityTransitionsSvg;delete data.lifecycleOperations;
 data.processOutline=processOutline(false);data.processOutlineSources={'checkout-root':source('public String checkout(Request request) { return inventory.reserve(request); }')};
 const r=load(data);r.run("showEntry('process-overview')");let html=r.e('scenario-content').innerHTML;
 assert.doesNotMatch(html,/Declared process-state schema|Activity on transitions/);
 assert.match(html,/Static source outline/);assert.match(html,/hasPositiveQuantity/);assert.match(html,/return inventory\.reserve/);
 assert.match(html,/PlantUML SVG preview is unavailable/);assert.match(html,/scenario-checkout-process-outline-91ab42\.puml/);
 assert.match(html,/Exact selected root and retained source bindings/);assert.match(html,/checkout-root/);
 r.click({sources:'checkout-root',sourceOperation:'process-outline'});
 assert.match(r.e('source-code').innerHTML,/public String checkout/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/CURRENT SOURCE/);
 const svgData=processFixture(false);delete svgData.stateDiagram;delete svgData.stateDiagramSvg;delete svgData.activityTransitions;delete svgData.activityTransitionsSvg;delete svgData.lifecycleOperations;
 svgData.processOutline=processOutline(true);svgData.processOutlineSources={'checkout-root':source('SVG ROOT SOURCE')};
 const svg=load(svgData);svg.run("showEntry('process-overview')");
 assert.match(svg.e('scenario-content').innerHTML,/src="\.\.\/diagrams\/scenario-checkout-process-outline-91ab42\.svg"/);
 assert.doesNotMatch(svg.e('scenario-content').innerHTML,/PlantUML SVG preview is unavailable/);
});

test('ambiguous exact process root is visible as a gap instead of a guessed outline',()=>{
 const data=processFixture(false);delete data.stateDiagram;delete data.activityTransitions;delete data.lifecycleOperations;
 data.processOutline={status:'GAP',gap:'PROCESS_ROOT_SELECTOR_AMBIGUOUS',root:{service:'orders',scope:null,candidates:[{scope:':main',observation:'orders:symbol:main'},{scope:':test',observation:'orders:symbol:test'}]}};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/No qualified local source outline is available/);assert.match(html,/PROCESS_ROOT_SELECTOR_AMBIGUOUS/);
 assert.match(html,/:main/);assert.match(html,/:test/);assert.doesNotMatch(html,/scenario-checkout-process-outline-.*\.puml/);
});

test('authored overview visuals precede a separate unreviewed local outline',()=>{
 const data=processFixture(false),overview=data.operations.find(operation=>operation.id==='process-overview');
 overview.visuals=[fixture().operations[0].visuals[0]];data.operationStates['process-overview']={freshness:'CURRENT',verification:'VERIFIED'};
 data.processOutline=processOutline(false);data.processOutlineSources={'checkout-root':source('EXACT CHECKOUT SOURCE')};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 const authored=html.indexOf('Authored process overview visuals'),generated=html.indexOf('Static source outline');
 assert.ok(authored>=0&&generated>authored,html);
 assert.match(html,/Dispatch work/);assert.match(html,/Meaning review: <b>Model approved/);
 assert.match(html,/not an authored or reviewed explanation/);
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
 assert.doesNotMatch(html,/accepted description of external boundaries|creates no entities|No outgoing calls|No Kafka|No cron/);
 assert.match(html,/Thread-to-call links and complete outgoing coverage are not inferred/);
});
test('read and stored-copy entity relations do not acquire ownership headings or changed provenance',()=>{
 for(const language of ['en','ru']){
  const data=entitySectionFixture();data.language=language;
  data.operations.find(o=>o.id==='section-entities').visuals=[];
  data.entities[0].normalized.entity.relations=['read','stored-copy'].map(kind=>({service:'sample',kind,origin:'agent-proposal',confidence:'declared',rationale:'Synthetic non-ownership relation.',representations:[]}));
  const before=JSON.stringify(data),r=load(data);r.run("showEntry('section-entities')");
  const html=r.e('scenario-content').innerHTML;
  assert.match(html,language==='ru'?/<h3>Предметные объекты<\/h3>/:/<h3>Domain catalogue<\/h3>/);
  assert.doesNotMatch(html,/<h3>Recorded domain ownership<\/h3>|<h3>Зафиксированные владельцы сущностей<\/h3>/);
  assert.match(html,language==='ru'?/Читает/:/unknown · read/);assert.match(html,language==='ru'?/Работает с копией/:/unknown · stored-copy/);
  assert.match(html,/agent-proposal/);
  assert.equal(JSON.stringify(data),before);
 }
});
test('entity section shows only its accepted local visuals while the process gallery still includes all owners',()=>{
 const data=entitySectionFixture();data.savedProcesses=[{id:'saved',title:'Saved workflow',href:'../scenarios/saved.html',status:'AUTHORED'}];
 const r=load(data);r.run("showEntry('section-entities')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Domain catalogue/);assert.match(html,/Synthetic Order/);
 assert.match(html,/<h3>Entity views<\/h3>/);assert.match(html,/Order identity map/);assert.match(html,/Ownership criteria/);
 assert.doesNotMatch(html,/Dispatch work|Select handler/);
 assert.equal((html.match(/class="visual-artifact"/g)||[]).length,2);
 assert.ok(html.indexOf('<h3>Synthetic Order</h3>')<html.indexOf('<h3>Entity views</h3>'));
 assert.match(html,/Source freshness: <b>STALE<\/b> · Meaning review: <b>UNASSESSED/);
 assert.match(html,/Accepted with Domain entities \(section-entities\)/);assert.match(html,/&quot;freshness&quot;: &quot;STALE&quot;/);assert.match(html,/&quot;verification&quot;: &quot;UNASSESSED&quot;/);
 r.run("showEntry('process-catalog')");
 const gallery=r.e('scenario-content').innerHTML;
 assert.match(gallery,/Processes and diagrams · 4/);assert.match(gallery,/Dispatch work/);assert.match(gallery,/Order identity map/);
});
test('entity section heading is translated in Russian while accepted visual content remains authored',()=>{
 const data=entitySectionFixture();data.language='ru';
 const r=load(data);r.run("showEntry('section-entities')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/<h3>Схемы сущностей<\/h3>/);assert.match(html,/Order identity map/);
 assert.match(html,/Актуальность кода: <b>Устарело<\/b>/);assert.match(html,/Проверка смысла: <b>Смысл не проверен<\/b>/);
 assert.doesNotMatch(html,/<h3>Entity views<\/h3>|Source freshness:|Meaning review:/);
});
test('entity declaration current source and section visual accepted source stay distinct for a reused source ID',()=>{
 const r=load(entitySectionFixture());r.run("showEntry('section-entities')");
 r.click({sources:'same-source'},true);
 assert.match(r.e('source-code').innerHTML,/SYNTHETIC CURRENT SOURCE RECORD/);
 r.click({sources:'same-source',sourceOperation:'section-entities'});
 assert.match(r.e('source-code').innerHTML,/SYNTHETIC ACCEPTED SOURCE RECORD/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/SYNTHETIC CURRENT SOURCE RECORD/);
});
test('missing entity section owner shows its gap and a translation gap suppresses accepted entity visuals',()=>{
 const missing=fixture();missing.sections.push({id:'section-entities',title:'Domain entities',gap:'No accepted entity section.'});
 const missingReader=load(missing);missingReader.run("showEntry('section-entities')");
 assert.match(missingReader.e('scenario-content').innerHTML,/No accepted entity section/);
 assert.doesNotMatch(missingReader.e('scenario-content').innerHTML,/Entity views|visual-artifact|Synthetic Order/);

 const translated=entitySectionFixture();translated.language='ru';translated.requestedDocumentationLanguage='ru';
 translated.translationGaps={'section-entities':{requestedLanguage:'ru',availableLanguage:'en',href:'../history/service.html'}};
 const original=JSON.stringify(translated),reader=load(translated);reader.run("showEntry('section-entities')");
 const html=reader.e('scenario-content').innerHTML;
 assert.match(html,/Перевод ещё не подготовлен/);assert.match(html,/Английская версия/);
 assert.doesNotMatch(html,/Synthetic Order|Order identity map|Ownership criteria|Схемы сущностей/);
 assert.equal(reader.run('JSON.stringify(publication)'),original);
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
 assert.doesNotMatch(r.e('scenario-nav').innerHTML,/РАЗДЕЛ/);assert.match(r.e('scenario-nav').innerHTML,/Обзор/);
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
 r.run('catalogue()');assert.match(r.e('catalogue-view').innerHTML,/Каталог исходного кода и API/);assert.match(r.e('catalogue-view').innerHTML,/POST/);assert.match(r.e('catalogue-view').innerHTML,/\/tasks\/\{taskType\}/);
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
test('full Pages readable-original policy keeps mismatched descriptions and visuals with their original state',()=>{
 const data=entitySectionFixture();data.language='ru';data.requestedDocumentationLanguage='ru';
 data.proseDisplayPolicy='READABLE_ORIGINALS';data.translationComplete=false;
 data.translationGaps={'section-entities':{requestedLanguage:'ru',availableLanguage:'en',originalPath:'services/sample.operation-section-entities.original.json'}};
 const original=JSON.stringify(data),r=load(data);r.run("showEntry('section-entities')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Synthetic Order|Order identity map/);
 assert.doesNotMatch(html,/They are hidden here until|Они скрыты/);
 assert.equal(r.run('D.operations.length'),data.operations.length);
 assert.equal(r.run('JSON.stringify(publication)'),original);
 assert.equal(r.e('document-data').textContent,original);
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
  nodes['catalog-data'].textContent=JSON.stringify([
   {kind:'Service',id:'orderAPI',title:'Authored title',href:'services/orderAPI.html',context:'Orders service',summary:'Inventory import hit',searchText:['order-manager'],coverage:'authored'},
   {kind:'Process',id:'p',title:'ProcessTitle',href:'scenarios/p.html',context:'Checkout process',summary:'Exports orders',searchText:['checkout-steps'],coverage:'inventory'},
   {kind:'Entity',id:'order',title:'Order',href:'services/orderAPI.html#section-entities',context:'Orders service',summary:['OrderRecord'],searchText:['order-record'],coverage:'unavailable'}
  ]);
  const location={search:'?q=order-manager%20hit&kind=Service&campaign=qa',pathname:'/catalog.html',hash:''};
  const history={state:{marker:'keep'},lastUrl:null,pushState(state,title,url){this.replaceState(state,title,url);},replaceState(state,_title,url){assert.equal(state.marker,'keep');this.lastUrl=url;const parsed=new URL(url,'https://codex.test');location.search=parsed.search;location.hash=parsed.hash;}};
  const document={documentElement:{lang:language},querySelector(){return null;},querySelectorAll(){return [];},getElementById(key){return nodes[key];},createElement:element};
  vm.runInNewContext(reader,{document,URL,URLSearchParams,location,history});
  assert.equal(nodes['catalog-results'].children.length,1);
  assert.equal(nodes['catalog-query'].value,'order-manager hit');
  assert.equal(nodes['catalog-kind'].value,'Service');
  assert.equal(nodes['catalog-results'].children[0].children.length,3);
  assert.equal(nodes['catalog-results'].children[0].children[0].textContent,'Authored title');
  assert.equal(nodes['catalog-results'].children[0].children[0].href,'services/orderAPI.html');
  assert.equal(nodes['catalog-results'].children[0].children[1].textContent,language==='ru'?'Сервис · Orders service · orderAPI · Описание есть':'Service · Orders service · orderAPI · Description present');
  nodes['catalog-query'].value='inventory hit';nodes['catalog-query'].listeners.input();assert.equal(nodes['catalog-results'].children.length,1);
  let url=new URL(history.lastUrl,'https://codex.test');assert.equal(url.searchParams.get('q'),'inventory hit');assert.equal(url.searchParams.get('kind'),'Service');assert.equal(url.searchParams.get('campaign'),'qa');
  nodes['catalog-query'].value='';nodes['catalog-query'].listeners.input();assert.equal(nodes['catalog-results'].children.length,1);
  nodes['catalog-kind'].value='';nodes['catalog-kind'].listeners.change();assert.equal(nodes['catalog-results'].children.length,3);
  assert.match(nodes['catalog-results'].children[1].children[1].textContent,language==='ru'?/Без описания/:/No description/);
  assert.match(nodes['catalog-results'].children[2].children[1].textContent,language==='ru'?/Есть пробелы/:/Evidence or translation gap/);
  url=new URL(history.lastUrl,'https://codex.test');assert.equal(url.searchParams.get('campaign'),'qa');assert.equal(url.searchParams.has('kind'),false);
  nodes['catalog-kind'].value='Service';nodes['catalog-kind'].listeners.change();assert.equal(nodes['catalog-results'].children.length,1);
  nodes['catalog-query'].value='absent';nodes['catalog-query'].listeners.input();assert.equal(nodes['catalog-results'].children.length,0);
  assert.match(nodes['catalog-status'].textContent,language==='ru'?/Совпадений нет/:/No matching results/);
 }
});

test('user-authored paragraph exposes unverified original code after a source update',()=>{
 const data=fixture(),op=data.operations[0];
 op.explanation=[{id:'human-paragraph',text:'The editor maintains this explanation.',sourceIds:['same-source'],detail:false,authorship:{authority:'USER_DOCUMENTATION',author:'Fixture <editor>',meaningReview:'UNASSESSED',contextRole:'RETAINED_UNVERIFIED_CONTEXT',sourceSnapshot:'snapshot-original',editDigest:'edit-original',sourceRefs:{'same-source':'original-record'},dependencyRefs:{dep:'original-observation'}}}];
 const key=data.subject+'/'+op.id+'/human-paragraph';
 data.fragmentSources={[key]:{'same-source':source('ORIGINAL PARAGRAPH SOURCE')}};
 data.fragmentStates={[key]:{freshness:'STALE',verification:'UNASSESSED'}};
 const r=load(data),html=r.run('explanation(D.operations[0])');
 assert.match(html,/User documentation by Fixture &lt;editor&gt;/);
 assert.match(html,/Meaning review: UNASSESSED/);
 assert.match(html,/Originally linked code \(unverified context\)/);
 assert.doesNotMatch(html,/Supporting code/);
 assert.match(html,/data-source-operation="section-responsibilities"/);
 r.click({sources:'same-source',sourceOperation:op.id,sourceFragment:key});
 assert.match(r.e('source-code').innerHTML,/ORIGINAL PARAGRAPH SOURCE/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/CURRENT SOURCE/);
 assert.match(r.e('source-foot').innerHTML,/Linked code does not verify the narrative meaning/);
 op.explanation[0].sourceIds=[];
 const empty=load(data).run('explanation(D.operations[0])');
 assert.doesNotMatch(empty,/data-sources=/);
 assert.match(empty,/User documentation by/);
});

test('mixed operation routes protected and generated paragraphs with the same SOURCE id independently',()=>{
 const data=fixture(),op=data.operations[0],key=data.subject+'/'+op.id+'/protected';
 op.explanation=[{id:'protected',text:'Protected user explanation.',sourceIds:['same-source'],detail:false,authorship:{authority:'USER_DOCUMENTATION',author:'Fixture editor',meaningReview:'UNASSESSED',contextRole:'RETAINED_UNVERIFIED_CONTEXT',sourceSnapshot:'old-snapshot',editDigest:'old-edit'}}];
 data.operationSources[op.id]['same-source']=source('NEW GENERATED CODE');
 data.fragmentSources={[key]:{'same-source':source('OLD USER CONTEXT CODE')}};
 data.fragmentStates={[key]:{freshness:'STALE',verification:'UNASSESSED'}};
 const r=load(data),html=r.run('explanation(D.operations[0])');
 assert.match(html,/Source context freshness: STALE/);
 assert.match(html,/data-source-fragment="service:sample\/section-responsibilities\/protected"/);
 r.click({sources:'same-source',sourceOperation:op.id});
 assert.match(r.e('source-code').innerHTML,/NEW GENERATED CODE/);
 r.click({sources:'same-source',sourceOperation:op.id,sourceFragment:key});
 assert.match(r.e('source-code').innerHTML,/OLD USER CONTEXT CODE/);
 assert.doesNotMatch(r.e('source-code').innerHTML,/NEW GENERATED CODE/);
 const trigger=r.e('protected-source-trigger');
 r.run("document.activeElement=document.getElementById('protected-source-trigger');source(['same-source'],'Missing protected paragraph',false,D.operations[0].id,D.subject+'/'+D.operations[0].id+'/protected-missing')");
 assert.equal(r.e('source-code').innerHTML,'');
 assert.match(r.e('source-foot').textContent,/Selected fragment source context is unavailable/);
 assert.equal(r.e('source-panel').hidden,false);
 assert.equal(r.e('source-title').textContent,'Missing protected paragraph');
 assert.equal(r.e('close-source').focused,true);
 assert.equal(trigger.focused,undefined);
 r.e('close-source').onclick();
 assert.equal(r.e('source-panel').hidden,true);
 assert.equal(trigger.focused,true,'closing a gap restores its source trigger focus');
 r.click({sources:'same-source missing-source',sourceOperation:op.id,sourceFragment:key});
 assert.equal(r.e('source-code').innerHTML,'','an incomplete exact fragment never displays a subset as complete context');
 assert.equal(r.e('source-select').innerHTML,'');
 assert.match(r.e('source-foot').textContent,/Selected fragment source context is unavailable.*Missing sources: missing-source/);
 r.click({sources:'same-source missing-source',sourceOperation:op.id});
 assert.equal(r.e('source-code').innerHTML,'');
 assert.match(r.e('source-foot').textContent,/Selected source context is incomplete.*Missing sources: missing-source/);
 r.click({sources:'same-source',sourceOperation:'wrong-owner',sourceFragment:key});
 assert.equal(r.e('source-code').innerHTML,'');
 r.click({sources:'same-source',sourceOperation:op.id});
 assert.match(r.e('source-code').innerHTML,/NEW GENERATED CODE/);
});

test('public mixed publication exposes original paragraph bytes and current generated bytes', {skip:!process.env.CODECLEW_MIXED_READER_DATA},()=>{
 const data=JSON.parse(fs.readFileSync(process.env.CODECLEW_MIXED_READER_DATA,'utf8'));
 const op=data.operations.find(o=>o.explanation.some(p=>p.authorship));
 assert.ok(op);
 const paragraph=op.explanation.find(p=>p.authorship),key=data.subject+'/'+op.id+'/'+paragraph.id;
 const id=paragraph.sourceIds.find(id=>data.fragmentSources?.[key]?.[id]&&data.operationSources?.[op.id]?.[id]);
 assert.ok(id,'one original logical SOURCE id exists in both contexts');
 const oldCode=data.fragmentSources[key][id].text,newCode=data.operationSources[op.id][id].text;
 assert.notEqual(oldCode,newCode);
 assert.equal(data.fragmentStates[key].freshness,'STALE');
 const r=load(data);
 r.click({sources:id,sourceOperation:op.id});
 assert.match(r.e('source-code').innerHTML,/\+ 1/);
 r.click({sources:id,sourceOperation:op.id,sourceFragment:key});
 assert.doesNotMatch(r.e('source-code').innerHTML,/\+ 1/);
 assert.match(r.e('source-code').innerHTML,/return normalize\(quantity\);/);
 const html=r.run('explanation(D.operations.find(o=>o.explanation.some(p=>p.authorship)))');
 assert.match(html,/Source context freshness: STALE/);
 assert.match(html,/Meaning review: UNASSESSED/);
 r.click({sources:id,sourceOperation:op.id,sourceFragment:key+'-missing'});
 assert.equal(r.e('source-code').innerHTML,'');
 assert.match(r.e('source-foot').textContent,/Selected fragment source context is unavailable/);
});

test('manual context selection retains text attribution and distinguishes freshness from review',()=>{
 const data=fixture(),op=data.operations[0],key=data.subject+'/'+op.id+'/maintained';
 op.explanation=[{id:'maintained',text:'Preserved user text.',sourceIds:['same-source'],detail:false,authorship:{authority:'USER_DOCUMENTATION',author:'Text <author>',meaningReview:'UNASSESSED',contextRole:'RETAINED_UNVERIFIED_CONTEXT',sourceSnapshot:'new-snapshot',editDigest:'original-text-edit',contextMigration:{editor:'Context <editor>',contextReview:'UNASSESSED',previousSourceSnapshot:'old-snapshot',previousContextDigest:'old-context',instructionDigest:'context-instruction'}}}];
 data.fragmentSources={[key]:{'same-source':source('EXPLICIT CURRENT CONTEXT')}};
 data.fragmentStates={[key]:{freshness:'CURRENT',verification:'UNASSESSED'}};
 const r=load(data),html=r.run('explanation(D.operations[0])');
 assert.match(html,/User documentation by Text &lt;author&gt;/);
 assert.match(html,/Context selected by Context &lt;editor&gt;/);
 assert.match(html,/Context review: UNASSESSED/);
 assert.match(html,/Current freshness does not establish semantic review/);
 assert.match(html,/Explicitly selected code \(unverified context\)/);
 assert.doesNotMatch(html,/Originally linked code/);
 r.click({sources:'same-source',sourceOperation:op.id,sourceFragment:key});
 assert.match(r.e('source-code').innerHTML,/EXPLICIT CURRENT CONTEXT/);
 data.fragmentStates[key].freshness='STALE';
 assert.match(load(data).run('explanation(D.operations[0])'),/Source context freshness: STALE/);
});

test('native catalogue reuses bounded filtering and restores query kind and page on reload and Back',()=>{
 const reader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 const element=(tagName='')=>({tagName,children:[],value:'',textContent:'',listeners:{},replaceChildren(){this.children=[];},append(...nodes){this.children.push(...nodes);},addEventListener(kind,fn){this.listeners[kind]=fn;}});
 const nodes=Object.fromEntries(['catalog-data','catalog-query','catalog-kind','catalog-results','catalog-status','catalog-prev','catalog-next','catalog-controls','catalog-pager','catalog-processes'].map(key=>[key,element()]));
 const rows=Array.from({length:45},(_,i)=>({kind:'Examined callable',id:`callable-${i}`,title:`method:class:ChildWorker#prepare(Task${i})Request`,displayTitle:'ChildWorker.prepare',href:`source-calls.html#body-${i}`,context:'linked · :/main',summary:'Examined source, not runtime impact',searchText:['prepare','parent-a','parent-b'],relatedLinks:[{title:'Examined by: parent-a',href:'parent-a-overview.html'},{title:'Examined by: parent-b',href:'parent-b-overview.html'}]}));
 rows.push({kind:'Diagnostic question',id:'question',title:'Why is delivery absent?',href:'child-diagnostic.html',summary:'Supplied question, not an approved answer'});
 nodes['catalog-data'].textContent=JSON.stringify(rows);
 const location={pathname:'/index.html',search:'?q=prepare&kind=Examined+callable&page=2&campaign=owned',hash:'#catalog-title'};
 const stack=[`${location.pathname}${location.search}${location.hash}`];let index=0,popstate;
 const setUrl=url=>{const parsed=new URL(url,'https://codex.test');location.search=parsed.search;location.hash=parsed.hash;};
 const history={state:{original:'keep'},replaceState(state,_title,url){assert.equal(state.original,'keep');stack[index]=url;setUrl(url);},pushState(state,_title,url){assert.equal(state.original,'keep');stack.splice(index+1);stack.push(url);index++;setUrl(url);}};
 const document={documentElement:{lang:'en'},querySelector(){return null;},querySelectorAll(){return [];},getElementById:key=>nodes[key],createElement:element,createTextNode:text=>({textContent:text})};
 vm.runInNewContext(reader,{document,URL,URLSearchParams,location,history,addEventListener(kind,fn){assert.equal(kind,'popstate');popstate=fn;}});
 const list=nodes['catalog-results'];
 assert.equal(list.children.length,20);assert.equal(list.children[0].children[0].href,'source-calls.html#body-20');
 assert.equal(list.children[0].children[3].children[0].href,'parent-a-overview.html');
 assert.equal(list.children[0].children[3].children[2].href,'parent-b-overview.html');
 const preview=list.children[0];assert.equal(preview.children[0].textContent,'ChildWorker.prepare');
 assert.equal(preview.children[1].textContent,'Examined callable · linked · :/main');
 assert.equal(preview.children[2].textContent,'Examined source, not runtime impact');
 const exact=preview.children[4];assert.equal(exact.tagName,'details');assert.equal(exact.className,'catalog-identity');assert.equal(Object.hasOwn(exact,'open'),false);
 assert.equal(exact.children[0].tagName,'summary');assert.equal(exact.children[0].textContent,'Exact identity');
 assert.equal(exact.children[1].children[1].tagName,'code');assert.equal(exact.children[1].children[1].textContent,rows[20].title);
 assert.equal(exact.children[2].children[1].textContent,rows[20].id);
 assert.equal(new Set(list.children.map(li=>li.children[0].href)).size,20);
 assert.equal(nodes['catalog-controls'].hidden,false);assert.equal(nodes['catalog-pager'].hidden,false);assert.equal(nodes['catalog-processes'].hidden,true);
 nodes['catalog-next'].listeners.click();assert.equal(list.children.length,5);assert.equal(new URL(stack[index],'https://codex.test').searchParams.get('page'),'3');
 index--;setUrl(stack[index]);popstate();assert.equal(list.children.length,20);assert.equal(list.children[0].children[0].href,'source-calls.html#body-20');
 nodes['catalog-kind'].value='Diagnostic question';nodes['catalog-query'].value='';nodes['catalog-kind'].listeners.change();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,'child-diagnostic.html');assert.match(list.children[0].children[2].textContent,/not an approved answer/);
 index--;setUrl(stack[index]);popstate();assert.equal(nodes['catalog-query'].value,'prepare');assert.equal(nodes['catalog-kind'].value,'Examined callable');assert.equal(list.children[0].children[0].href,'source-calls.html#body-20');
 location.search='?q=prepare&kind=Examined+callable&page=999999999999';popstate();assert.equal(list.children.length,5);assert.equal(new URL(stack[index],'https://codex.test').searchParams.get('page'),'3');
 location.search='?q=absent';popstate();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,'child-diagnostic.html');
 location.search=`?q=${encodeURIComponent(rows[31].title)}`;popstate();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,rows[31].href);assert.equal(list.children[0].children[4].children[1].children[1].textContent,rows[31].title);
 location.search='?q=callable-31';popstate();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,rows[31].href);
 location.search='?q=ChildWorker.prepare&page=2';popstate();assert.equal(list.children.length,20);assert.equal(list.children[0].children[0].href,rows[20].href);
 location.search='?q=unmatched-fixture-query-9f7b&page=2';popstate();assert.equal(list.children.length,0);assert.match(nodes['catalog-status'].textContent,/No matching results/);assert.equal(new URL(stack[index],'https://codex.test').searchParams.has('page'),false);
});


test('native catalogue synthetic 200-service 2000-process DOM pagination filtering and history',()=>{
 // SYNTHETIC_DOM_ONLY_NOT_NATIVE_2000_PROCESS: renderer-shaped rows exercise
 // the shipped reader only, not native capture/projection caps or usefulness.
 const reader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 const hash=value=>require('node:crypto').createHash('sha256').update(value).digest('hex');
 const canonical=(kind,service,scope,symbol)=>`${kind}-${hash(JSON.stringify([service,scope,symbol]))}`;
 const rows=[];
 for(let s=0;s<200;s++){
  const service=`svc${String(s).padStart(3,'0')}`,scope=':/main',context=`${service} · ${scope}`;
  const processes=Array.from({length:10},(_,p)=>`${service}-parent-${String(p).padStart(2,'0')}`);
  for(const id of processes)rows.push({id,title:id,kind:'Process',href:`${id}-overview.html`,context,summary:'Selected source process; runtime activation and business meaning unverified.',searchText:[service,scope,'method:class:Parent#submit()V','method:class:ChildWorker#run()V'],relatedLinks:[]});
  const endpointSymbol='method:class:Parent#submit()V',endpointId=canonical('endpoint',service,scope,endpointSymbol);
  rows.push({id:endpointId,title:endpointSymbol,displayTitle:'Parent.submit',kind:'Endpoint',href:`${processes[0]}-endpoint.html`,context,summary:'Explicitly selected compiler declaration; no inferred HTTP route or runtime activation.',searchText:[service,scope,endpointSymbol],relatedLinks:processes.map(id=>({href:`${id}-overview.html`,title:`Selected process: ${id}`}))});
  const symbol='method:class:ChildWorker#prepare(Task)Request',id=canonical('callable',service,scope,symbol);
  rows.push({id,title:symbol,displayTitle:'ChildWorker.prepare',kind:'Examined callable',href:`source-calls.html#ref-${hash(id)}`,context,summary:'Retained examined source body; reverse links are documentation context, not runtime impact. Call frontiers remain local gaps.',searchText:[service,scope,symbol,...processes.slice(0,2)],relatedLinks:processes.slice(0,2).map(parent=>({href:`${parent}-overview.html`,title:`Examined by: ${parent}`}))});
  rows.push({id:`diagnostic-${processes[0]}`,title:'Why is delivery absent?',kind:'Diagnostic question',href:`${processes[0]}-diagnostic.html`,context,summary:'Supplied diagnostic question; source conditions describe possible reasons, not an approved answer or observed incident.',searchText:[processes[0],service,scope,endpointSymbol],relatedLinks:[]});
 }
 rows.sort((a,b)=>{for(const key of ['kind','title','id']){if(a[key]<b[key])return -1;if(a[key]>b[key])return 1;}return 0;});
 assert.equal(rows.length,2600);assert.equal(new Set(rows.filter(r=>r.kind==='Process').map(r=>r.context)).size,200);
 const element=()=>({children:[],value:'',textContent:'',listeners:{},replaceChildren(){this.children=[];},append(...nodes){this.children.push(...nodes);},addEventListener(kind,fn){this.listeners[kind]=fn;}});
 function loadCatalogue(initialUrl){
  const nodes=Object.fromEntries(['catalog-data','catalog-query','catalog-kind','catalog-results','catalog-status','catalog-prev','catalog-next','catalog-controls','catalog-pager','catalog-processes'].map(key=>[key,element()]));
  nodes['catalog-data'].textContent=JSON.stringify(rows);
  const location={pathname:'/index.html',search:'',hash:''},stack=[initialUrl];let index=0,popstate;
  const setUrl=url=>{const parsed=new URL(url,'https://codex.test');location.search=parsed.search;location.hash=parsed.hash;};setUrl(initialUrl);
  const history={state:{marker:'preserve'},replaceState(state,_title,url){assert.equal(state.marker,'preserve');stack[index]=url;setUrl(url);},pushState(state,_title,url){assert.equal(state.marker,'preserve');stack.splice(index+1);stack.push(url);index++;setUrl(url);}};
  const document={documentElement:{lang:'en'},querySelector(){return null;},querySelectorAll(){return [];},getElementById:key=>nodes[key],createElement:element,createTextNode:text=>({textContent:text})};
  vm.runInNewContext(reader,{document,URL,URLSearchParams,location,history,addEventListener(kind,fn){assert.equal(kind,'popstate');popstate=fn;}});
  return {nodes,url:()=>stack[index],back(){assert.ok(index>0);setUrl(stack[--index]);popstate();},restore(url){setUrl(url);popstate();},links:()=>nodes['catalog-results'].children.map(li=>li.children[0].href)};
 }
 const processRows=rows.filter(r=>r.kind==='Process');
 const r=loadCatalogue('/index.html?kind=Process&campaign=synthetic#catalog-title'),n=r.nodes,seen=[];
 for(let page=1;page<=100;page++){
  assert.equal(n['catalog-results'].children.length,20);
  assert.equal(n['catalog-status'].textContent,`2000 results · page ${page} of 100`);
  assert.deepEqual(r.links(),processRows.slice((page-1)*20,page*20).map(row=>row.href));seen.push(...r.links());
  assert.equal(n['catalog-prev'].disabled,page===1);assert.equal(n['catalog-next'].disabled,page===100);
  if(page<100)n['catalog-next'].listeners.click();
 }
 assert.equal(new Set(seen).size,2000);assert.equal(seen[1999],'svc199-parent-09-overview.html');
 n['catalog-prev'].listeners.click();assert.deepEqual(r.links(),processRows.slice(1960,1980).map(row=>row.href));
 r.back();assert.deepEqual(r.links(),processRows.slice(1980).map(row=>row.href));
 const reload=loadCatalogue(r.url());assert.deepEqual(reload.links(),r.links());assert.equal(reload.nodes['catalog-kind'].value,'Process');assert.equal(reload.nodes['catalog-status'].textContent,'2000 results · page 100 of 100');
 n['catalog-query'].value='svc199';n['catalog-query'].listeners.input();assert.deepEqual(r.links(),processRows.filter(row=>row.context.startsWith('svc199 ')).map(row=>row.href));assert.equal(n['catalog-status'].textContent,'10 results · page 1 of 1');
 n['catalog-kind'].value='Examined callable';n['catalog-kind'].listeners.change();
 const helper=rows.find(row=>row.kind==='Examined callable'&&row.context.startsWith('svc199 '));
 assert.deepEqual(r.links(),[helper.href]);const related=n['catalog-results'].children[0].children[3].children;
 assert.equal(related[0].href,'svc199-parent-00-overview.html');assert.equal(related[0].textContent,'Examined by: svc199-parent-00');assert.equal(related[2].href,'svc199-parent-01-overview.html');
 assert.equal(n['catalog-results'].children[0].children[0].textContent,'ChildWorker.prepare');
 assert.equal(n['catalog-results'].children[0].children[4].children[2].children[1].textContent,helper.id);
 assert.notEqual(helper.id,rows.find(row=>row.kind==='Examined callable'&&row.context.startsWith('svc198 ')).id);
 const helperReload=loadCatalogue(r.url());assert.deepEqual(helperReload.links(),[helper.href]);assert.equal(helperReload.nodes['catalog-query'].value,'svc199');assert.equal(helperReload.nodes['catalog-kind'].value,'Examined callable');
 r.back();assert.equal(n['catalog-kind'].value,'Process');assert.equal(n['catalog-query'].value,'svc199');assert.equal(r.links().length,10);
 n['catalog-query'].value='svc199 prepare';n['catalog-query'].listeners.input();assert.equal(r.links().length,0);assert.match(n['catalog-status'].textContent,/No matching results/);
 n['catalog-kind'].value='Examined callable';n['catalog-kind'].listeners.change();assert.deepEqual(r.links(),[helper.href]);
 n['catalog-query'].value='delivery absent';n['catalog-query'].listeners.input();n['catalog-kind'].value='Diagnostic question';n['catalog-kind'].listeners.change();assert.equal(n['catalog-status'].textContent,'200 results · page 1 of 10');assert.match(n['catalog-results'].children[0].children[2].textContent,/not an approved answer/);
 r.restore('/index.html?page=999999&campaign=synthetic#catalog-title');assert.equal(n['catalog-status'].textContent,'2600 results · page 130 of 130');assert.deepEqual(r.links(),rows.slice(2580).map(row=>row.href));assert.equal(n['catalog-next'].disabled,true);
 const lastReload=loadCatalogue(r.url());assert.deepEqual(lastReload.links(),rows.slice(2580).map(row=>row.href));
 const url=new URL(r.url(),'https://codex.test');assert.equal(url.searchParams.get('page'),'130');assert.equal(url.searchParams.get('campaign'),'synthetic');assert.equal(url.hash,'#catalog-title');
});

test('pinned-only freshness presents a saved snapshot without hiding the original authority',()=>{
 const data=fixture();data.sectionState={freshness:'UNVERIFIED',verification:'VERIFIED_WITH_LIMITATIONS',contentRevisions:{sample:'abc'},targetRevisions:{sample:'abc'},reasons:[{reason:'PINNED_SNAPSHOT_NOT_REVERIFIED',snapshot:'saved'}]};
 const original=JSON.stringify(data.sectionState),r=load(data),html=r.e('freshness-status').innerHTML;
 assert.match(html,/freshness-snapshot/);assert.match(html,/Saved source snapshot/);assert.match(html,/Current checkout freshness was not rechecked/);
 assert.match(html,/^<details[^>]+><summary>Source · UNVERIFIED<\/summary>/);assert.doesNotMatch(html,/^<details[^>]*\bopen(?:\s|=|>)/);
 assert.match(html,/Source freshness: <b>UNVERIFIED<\/b>/);assert.match(html,/PINNED_SNAPSHOT_NOT_REVERIFIED/);assert.match(html,/Model approved with limitations/);
 assert.doesNotMatch(html,/freshness-unverified|retained with a gap/);assert.equal(JSON.stringify(r.data.sectionState),original);
 const ru=load({...data,language:'ru'}).e('freshness-status').innerHTML;assert.match(ru,/Сохранённая версия исходного кода/);assert.match(ru,/Не проверено/);
 for(const reasons of [[],[{reason:'SOURCE_MISSING'}],[{reason:'PINNED_SNAPSHOT_NOT_REVERIFIED'},{reason:'SOURCE_MISSING'}]]){
  const actual=load({...data,sectionState:{...data.sectionState,reasons}}).e('freshness-status').innerHTML;
  assert.match(actual,/freshness-unverified/);assert.doesNotMatch(actual,/freshness-snapshot/);assert.match(actual,/^<details /,"unknown state is explicit in compact disclosure");assert.match(actual,/<strong>UNVERIFIED<\/strong>/);
 }
 assert.match(load({...data,sectionState:{...data.sectionState,freshness:'STALE'}}).e('freshness-status').innerHTML,/freshness-stale/);
});
test('unexplained source declarations stay discoverable with readable labels and exact identities',()=>{
 const data=fixture(),symbol='source:workers/Main.kt/package:example.worker/processRequest/114e9ee8cc58ecb97b3e';
 data.catalogue=[{id:'candidate',symbol,kind:'SOURCE_CALLABLE',trigger:{methods:['CODE'],paths:[]},sourceIds:[]}];
 const r=load(data),nav=r.e('scenario-nav').innerHTML;
 assert.match(nav,/<details class="pending-navigation" ><summary>Awaiting explanation · 1<\/summary>/);
 assert.match(nav,/processRequest\(\) · Main.kt/);assert.ok(nav.includes(symbol));
 r.e('search').value='114e9ee8';r.run('nav()');assert.match(r.e('scenario-nav').innerHTML,/<details class="pending-navigation" open>/);
 r.click({entry:'candidate'});assert.match(r.e('scenario-content').innerHTML,/processRequest\(\) · Main.kt/);assert.equal(r.run('current.id'),'candidate');
 r.run('catalogue()');assert.match(r.e('catalogue-view').innerHTML,/<summary>Exact identity<\/summary>/);assert.ok(r.e('catalogue-view').innerHTML.includes(symbol));
 assert.equal(r.data.catalogue[0].symbol,symbol);
 assert.equal(r.run("sourceLabel('source:workers/Main.kt/class:example.Nested/string/114e9ee8cc58ecb97b3e')"),'Nested.string() · Main.kt');
 assert.equal(r.run("sourceLabel('source:workers/Main.kt/class:example.Other/string/114e9ee8cc58ecb97b3e')"),'Other.string() · Main.kt');
});
test('mobile navigation starts collapsed and closes after selecting a section',()=>{
 const r=load(fixture(),true);assert.equal(r.e('reader-navigation').open,false);
 r.e('reader-navigation').open=true;r.click({entry:'section-responsibilities'});
 assert.equal(r.e('reader-navigation').open,false);assert.equal(r.e('content').focused,true);assert.equal(r.run('current.id'),'section-responsibilities');
 r.e('reader-navigation').open=true;r.e('catalogue-button').onclick();
 assert.equal(r.e('reader-navigation').open,false);assert.equal(r.e('catalogue-view').hidden,false);assert.equal(r.e('content').focused,true);
 assert.equal(load().e('reader-navigation').open,true);
});

test('reader section navigation records browser history while tabs keep the selected location',()=>{
 const r=load();
 assert.equal(r.navigationHistory.length,0);
 r.click({entry:'section-responsibilities'});assert.deepEqual(r.navigationHistory,['#section-responsibilities']);
 r.click({entry:'section-responsibilities'});assert.equal(r.navigationHistory.length,1);
 r.click({entry:'section-overview'});assert.deepEqual(r.navigationHistory,['#section-responsibilities','#section-overview']);
 r.run("showEntry('section-responsibilities','contract')");assert.equal(r.navigationHistory.length,2);
});

test('mobile source panel cycles visible enabled controls and Escape restores its initiating focus',()=>{
 const r=load(fixture(),true),panel=r.e('source-panel'),trigger=r.e('source-trigger'),background=r.e('background'),alreadyInert=r.e('already-inert');
 alreadyInert.inert=true;r.document.body.children=[background,alreadyInert,panel];
 const close=r.e('close-source'),link=r.e('source-url'),select=r.e('source-select'),copy=r.e('copy-source'),code=r.e('source-code'),binding=r.e('source-binding');
 const hidden=r.e('hidden-control'),disabled=r.e('disabled-control'),untabbable=r.e('untabbable-control'),invisible=r.e('invisible-control');
 hidden.hidden=true;disabled.disabled=true;untabbable.tabIndex=-1;invisible.visibility='hidden';
 panel.children=[close,link,select,copy,code,binding,hidden,disabled,untabbable,invisible];
 trigger.focus();r.click({sources:'same-source'});
 assert.equal(r.document.activeElement,close);assert.equal(panel.attributes.role,'dialog');assert.equal(panel.attributes['aria-modal'],'true');assert.equal(panel.attributes['aria-labelledby'],'source-title');assert.equal(background.inert,true);
 assert.equal(r.key('Tab',true),true);assert.equal(r.document.activeElement,binding);
 assert.equal(r.key('Tab'),true);assert.equal(r.document.activeElement,close);
 select.focus();assert.equal(r.key('Tab'),false,'the browser advances between interior controls');
 background.focus();assert.equal(r.key('Tab'),true);assert.equal(r.document.activeElement,close,'out-of-panel focus is recovered');
 assert.equal(r.key('Escape'),true);assert.equal(panel.hidden,true);assert.equal(r.document.activeElement,trigger);assert.equal(background.inert,undefined);assert.equal(alreadyInert.inert,true);assert.equal(panel.attributes.role,undefined);assert.equal(panel.attributes['aria-modal'],undefined);
 background.focus();assert.equal(r.key('Escape'),false);assert.equal(r.document.activeElement,background,'Escape outside a closed panel does not restore an old trigger');
});

test('source panel becomes modal only at mobile widths and releases background on resize or source gaps',()=>{
 const r=load(),panel=r.e('source-panel'),background=r.e('background'),trigger=r.e('source-trigger');
 panel.children=[r.e('close-source'),r.e('source-code')];r.document.body.children=[background,panel];trigger.focus();r.click({sources:'same-source'});
 assert.equal(panel.attributes.role,undefined);assert.equal(background.inert,undefined);
 r.e('source-code').focus();assert.equal(r.key('Tab'),false,'desktop side-panel permits keyboard access to the document');
 r.media.matches=true;background.focus();r.run('navigationViewport()');assert.equal(panel.attributes['aria-modal'],'true');assert.equal(r.document.activeElement,r.e('close-source'));assert.equal(background.inert,true);
 r.media.matches=false;r.run('navigationViewport()');assert.equal(panel.attributes.role,undefined);assert.equal(background.inert,undefined);assert.equal(r.key('Tab',true),false);
 r.media.matches=true;r.run('navigationViewport()');r.click({sources:'missing-source'});assert.equal(panel.hidden,true);assert.equal(background.inert,undefined);assert.equal(panel.attributes['aria-modal'],undefined);
});

test('declared process keeps one collapsed source section, exact line links and inert source text',()=>{
 const data=processFixture(false);data.process.definition.summary='A declared checkout path';
 data.processThread={status:'SOURCE_BOUND_DECLARATION',scope:'One bounded source path',rootMethod:'create',methodOrder:['create'],methods:{create:{label:'Order creation',source:{...source('fun create() {\n if (ready) save() else throw Error("<script>")\n return order\n}'),startLine:10,endLine:13}}},steps:[{id:'create',title:'Create <order>',text:'Save only when ready',method:'create',startLine:11,endLine:12,children:[{id:'return',title:'Return',text:'Return the order',method:'create',startLine:12,endLine:12,children:[]}]}],boundaries:['Delivery is unresolved']};
 data.processOutline={status:'GAP',gap:'FLOW_AUTHORITY_NOT_QUALIFIED'};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
 assert.match(html,/<details id="thread-source" class="thread-source">/);
 assert.match(html,/href="#thread-source-create-L11" data-thread-end="12"/);
 assert.match(html,/id="thread-source-create-L12" data-line="12"/);
 assert.match(html,/Create &lt;order&gt;/);assert.match(html,/Error\(&quot;&lt;script&gt;&quot;\)/);
 assert.doesNotMatch(html,/<script>/);
 assert.match(html,/<details class="process-supporting"><summary>Additional diagrams and source outlines/);
 assert.match(html,/Declared interpretation bound to exact retained source/);
});
test('shared source anchors reveal parents, focus the exact line, select a range and preserve navigation hash',()=>{
 const data=processFixture(false);data.processThread={status:'SOURCE_BOUND_DECLARATION',steps:[],methods:{},methodOrder:[]};
 const r=load(data);r.run("showEntry('process-overview')");
 const details=r.e('thread-source');details.tagName='DETAILS';details.open=false;
 const method=r.e('method');method.tagName='SECTION';method.parentElement=details;
 const line=r.e('thread-source-create-L10'),next=r.e('thread-source-create-L11');
 line.dataset={line:'10'};next.dataset={line:'11'};line.parentElement=method;line.closest=()=>method;
 const selected=[];line.classList.add=()=>selected.push(10);next.classList.add=()=>selected.push(11);
 method.querySelectorAll=()=>[line,next];
 r.run("revealThreadAnchor('thread-source-create-L10',11)");
 assert.equal(details.open,true);assert.equal(line.focused,true);assert.equal(line.scrolled,true);assert.deepEqual(selected,[10,11]);
 r.run("location.hash='#thread-source-create-L10';routeReaderHash()");
 assert.equal(r.run('location.hash'),'#thread-source-create-L10');
});

test('source navigation restores the process after catalogue or coverage and never drops out-of-thread evidence',()=>{
 const data=processFixture(false);data.processThread={status:'SOURCE_BOUND_DECLARATION',steps:[],methods:{create:{label:'Selected creation',source:{...source('fun create() = Order()'),id:'created-source',startLine:10}}},methodOrder:['create']};
 const r=load(data);r.run("showEntry('process-overview')");
 r.e('scenario-content').querySelector=()=>({});
 for(const view of ['catalogue()', 'coverage()']){
  r.run(view);assert.equal(r.e('scenario-content').hidden,true);
  r.run("location.hash='#thread-source-create-L10';routeReaderHash()");
  assert.equal(r.e('scenario-content').hidden,false);assert.equal(r.e('catalogue-view').hidden,true);assert.equal(r.e('coverage-view').hidden,true);
  assert.equal(r.run('location.hash'),'#thread-source-create-L10');
 }
 const links=r.run("threadEvidenceLinks(['created-source','checkout-root'])");
 assert.match(links,/href="#thread-source-create-L10"/);assert.match(links,/data-sources="checkout-root"/);assert.match(links,/Retained source outside the selected thread/);
});


function declaredThreadFixture(){
 const data=processFixture(false);data.operations=data.operations.filter(o=>o.id!=='process-overview');
 data.process.definition.summary='Create the request and return its representation';
 data.boundaries=['UNRELATED_PUBLICATION_DIAGNOSTIC'];
 data.processThread={status:'SOURCE_BOUND_DECLARATION',scope:'Two selected methods',rootMethod:'client',methodOrder:['client','receiver'],methods:{client:{label:'Create client',source:{...source('fun create() = client.send()'),id:'client-source',service:'caller',startLine:10,endLine:10}},receiver:{label:'Declared receiver',source:{...source('fun accept() = save()'),id:'receiver-source',service:'provider',startLine:20,endLine:20}}},steps:[{id:'send',title:'Submit request',text:'Call the client',method:'client',startLine:10,endLine:10,target:'receiver',children:[{id:'receive',kind:'DECISION',title:'Check request',text:'When request is valid',method:'receiver',startLine:20,endLine:20,children:[]}]}],boundaries:['Configuration and delivery are unresolved']};
 return data;
}
test('process reader distinguishes declared connections, reading order and scoped limits',()=>{
 const r=load(declaredThreadFixture());r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Create the request and return its representation/);
 assert.match(html,/Summary from the process declaration; no reviewed overview is implied/);
 assert.match(html,/Declared service connections/);assert.match(html,/caller → provider/);
 assert.match(html,/href="#thread-step-send"/);assert.match(html,/Delivery unverified/);
 assert.match(html,/Nesting groups explanations; it does not prove execution order or branch exclusivity/);
 assert.match(html,/<ul class="thread-steps">/);assert.doesNotMatch(html,/<ol class="thread-steps">/);
 assert.match(html,/thread-child-preview">Check request/);assert.match(html,/>Decision<\/span>/);
 assert.match(html,/<details class="process-captured"><summary>Declared scope, goals and outcomes/);
 assert.match(html,/Declared goals and outcomes are not proven program results/);
 assert.match(html,/<details class="process-publication-limits"><summary>Publication evidence details \(1\)/);
 assert.match(html,/UNRELATED_PUBLICATION_DIAGNOSTIC/);assert.match(html,/Configuration and delivery are unresolved/);
 assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
});
test('process chrome is localized and source and authored content stay inert',()=>{
 const data=declaredThreadFixture();data.language='ru';data.processThread.steps[0].title='<script>unsafe</script>';
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Объявленные связи сервисов/);assert.match(html,/Доставка не подтверждена/);
 assert.match(html,/Порядок|порядок исполнения/);assert.match(html,/&lt;script&gt;unsafe&lt;\/script&gt;/);
 assert.doesNotMatch(html,/<script>unsafe/);assert.match(html,/fun create\(\) = client.send\(\)/);
});
test('selected interaction declaration remains visible without an inferred cross-service step target',()=>{
 const data=declaredThreadFixture();delete data.processThread.steps[0].target;
 data.process.definition.interactions=['declared-send'];
 data.interactions=[{id:'declared-send',origin:'human',runtime:'UNKNOWN',contractStatus:'NOT_ASSESSED',callSite:{status:'MISSING'},from:{status:'SOURCE_MATCH',sourceIds:['client-source']},to:{status:'SOURCE_MATCH',sourceIds:['receiver-source']},destination:{status:'UNRESOLVED'},boundaries:['RUNTIME_UNKNOWN'],declaration:{title:'Declared submit',from:{service:'caller'},to:{service:'provider'},transport:{kind:'http',method:'POST',destinationConfigKey:'provider.base-url'}}},{id:'unselected',declaration:{title:'Unrelated transport',from:{service:'other'},to:{service:'provider'}}}];
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Declared submit/);assert.match(html,/Declared transport: http · POST/);
 assert.match(html,/provider.base-url/);assert.match(html,/callSite.*MISSING/s);
 assert.match(html,/href="#thread-source-client-L10"/);assert.match(html,/href="#thread-source-receiver-L20"/);
 assert.doesNotMatch(html,/Unrelated transport/);
 assert.match(html,/Delivery unverified/);
});
test('retained process case preserves selected source bytes and actual declared interactions', {skip:!process.env.CODECLEW_PROCESS_CASE_DIR},()=>{
 const folder=process.env.CODECLEW_PROCESS_CASE_DIR;
 const files=fs.readdirSync(folder).filter(file=>file.endsWith('.json'));
 let checked=0;
 for(const file of files){
  const data=JSON.parse(fs.readFileSync(path.join(folder,file),'utf8'));
  if(!data.processThread)continue;
  checked++;
  const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
  assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
  const escape=text=>text.replace(/[&<>"']/g,ch=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
  for(const id of data.processThread.methodOrder){
   const record=data.processThread.methods[id].source;
   for(const [index,line] of record.text.split('\n').entries()){
    assert.ok(html.includes(`id="thread-source-${id}-L${record.startLine+index}" data-line="${record.startLine+index}" tabindex="-1">${escape(line)}</span>`),`${file}: selected method ${id}, line ${record.startLine+index}`);
   }
  }
  for(const interaction of data.interactions||[]){
   if(!data.process.definition.interactions.includes(interaction.id)||!interaction.declaration)continue;
   assert.ok(html.includes(escape(interaction.declaration.title)),`${file}: declared interaction disappeared`);
   assert.match(html,/thread-connection-status/);
  }
  assert.match(html,/process-publication-limits/);
 }
 assert.ok(checked>0,'at least one retained process thread must be checked');
});

test('explicit first-match arms and selected effects remain separate and source linked',()=>{
 const data=declaredThreadFixture(),row=data.processThread.steps[0];delete row.children;
 const step=(id,title)=>({id,title,text:title,method:'client',startLine:10,endLine:10});
 row.branchGroup={mode:'FIRST_MATCH',arms:[{id:'failed',condition:'failedChunks > 0',guard:{method:'client',startLine:10,endLine:10},steps:[step('fail','Set Task.status to FAILED')]},{id:'error-rows',condition:'failedRows > 0',guard:{method:'client',startLine:10,endLine:10},steps:[{...step('errors','Select result status'),branchGroup:{mode:'IF_ELSE',arms:[{id:'missing-url',condition:'resultUrl.isNullOrBlank()',guard:{method:'client',startLine:10,endLine:10},steps:[step('wait-file','Assign IN_PROGRESS')]},{id:'result-url',condition:null,steps:[step('errors-file','Assign COMPLETED_WITH_ERRORS')]}]}}]},{id:'default-complete',condition:null,steps:[step('complete','Assign COMPLETED')]}]};
 row.effects=[{id:'save-call',kind:'STORAGE_CALL',label:'Invoke save <script>unsafe</script>',citation:{method:'client',startLine:10,endLine:10}}];
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/First matching arm/);assert.match(html,/One guarded arm or its default/);
 assert.match(html,/id="thread-step-failed"/);assert.match(html,/id="thread-step-missing-url"/);
 assert.match(html,/Selected declared effects/);assert.match(html,/calls do not prove commit or delivery/);
 assert.match(html,/Invoke save &lt;script&gt;unsafe&lt;\/script&gt;/);assert.doesNotMatch(html,/<script>unsafe/);
 assert.match(html,/href="#thread-source-client-L10" data-thread-end="10"/);
 assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
});
test('extraction packet keeps unknown outcomes and budget cuts visible without changing thread authority',()=>{
 const data=declaredThreadFixture();data.processExtraction={status:'TRUNCATED',methods:[{id:'root'}],events:[{id:'guard',detail:{kind:'IF',condition:'ready <script>unsafe</script>'},sourceIds:['client-source']}],query:{outcomeEventIds:['missing']},outcomes:[{eventId:'missing',discovery:'NOT_FOUND_UNKNOWN'}],frontier:[{category:'BUDGET',code:'METHOD_BUDGET'}],interpretationPlan:{actualModelCalls:0}};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Extracted evidence envelope/);assert.match(html,/METHOD_BUDGET/);assert.match(html,/NOT_FOUND_UNKNOWN/);
 assert.match(html,/ready &lt;script&gt;unsafe&lt;\/script&gt;/);assert.match(html,/runtime effects are not established/);
 assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
});

function contractThreadFixture(){
 const data=declaredThreadFixture(),thread=data.processThread;
 thread.processId='checkout';
 thread.serviceFragments=[{id:'accept',entryMethod:'receiver',rootStep:'receive'}];
 const evidence={route:{service:'provider',sourceId:'receiver-source',revision:'frozen-revision',textDigest:'sha256:'+ 'a'.repeat(64),startLine:20,endLine:20,source:thread.methods.receiver.source}};
 thread.boundaryContracts=[{id:'submit',callStep:'send',receiverFragment:'accept',transport:'HTTP',operation:{text:'POST baseDomain + basePath + RESOURCE_PATH',evidenceRefs:['route']},request:[{text:'Required request type; optional message defaults null',evidenceRefs:['route']}],response:[{text:'Return only after save returns',evidenceRefs:['route']}],failures:[{text:'A synchronous send failure skips caller continuation',evidenceRefs:['route']}],guarantees:[{text:'Commit and delivery are unresolved',missingEvidence:'Transaction activation and broker acknowledgement capture'}],evidence}];
 return data;
}
test('contracts reveal the exact receiving subtree and reciprocal service fragment address',()=>{
 const r=load(contractThreadFixture());r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/href="#thread-step-receive"/);
 assert.match(html,/\.\.\/services\/provider.html#service-fragment-checkout:accept/);
 assert.match(html,/thread-contract-submit/);assert.match(html,/POST baseDomain \+ basePath \+ RESOURCE_PATH/);
 assert.match(html,/Transaction activation and broker acknowledgement capture/);
 assert.equal((html.match(/<details id="thread-source"/g)||[]).length,1);
 assert.doesNotMatch(html,/runtime delivery (is |was )proven/);
});
test('service fragment selection and source reload preserve the owner without guessing an entrypoint',()=>{
 const data=contractThreadFixture(),thread=data.processThread;
 data.subject='service:provider';data.process=null;delete data.processThread;
 data.serviceFragments=[{id:'service-fragment-checkout:accept',fragmentId:'accept',processId:'checkout',title:'Checkout',entryMethod:'receiver',thread,steps:thread.steps[0].children,returnHref:'../scenarios/checkout.html#thread-step-receive'}];
 const r=load(data,true);r.run("showEntry('service-fragment-checkout:accept')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/href="\.\.\/scenarios\/checkout.html#thread-step-receive"/);
 assert.match(html,/href="#service-fragment-checkout:accept~thread-source-receiver-L20"/);
 assert.doesNotMatch(html,/data-thread-method="client"/);
 assert.equal((html.match(/<details id="thread-source"/g)||[]).length,1);
 r.run("location.hash='#service-fragment-checkout:accept~thread-source-receiver-L20'; routeReaderHash()");
 assert.equal(r.run('current.id'),'service-fragment-checkout:accept');
 assert.equal(r.run('location.hash'),'#service-fragment-checkout:accept~thread-source-receiver-L20');
 assert.equal(r.e('thread-source-receiver-L20').focused,true);
 assert.equal(r.run('inventoryEntries().some(e=>e.kind===\"SERVICE_FRAGMENT\")'),false);
});
test('receiving fragment opens its exact authored service operation without copying its body',()=>{
 const data=contractThreadFixture(),thread=data.processThread;
 data.subject='service:provider';data.process=null;delete data.processThread;
 thread.methods.receiver.entrypoint='receiver-operation';
 const operation={...data.operations[0],id:'receiver-operation',title:'Handle incoming request',summary:fragment('Retained repository and outbox behavior'),visuals:[]};
 data.operations.push(operation);
 data.catalogue.push({id:operation.id,symbol:operation.title,kind:'FUNCTION',trigger:{},sourceIds:[]});
 data.serviceFragments=[{id:'service-fragment-checkout:accept',fragmentId:'accept',processId:'checkout',title:'Checkout',entryMethod:'receiver',thread,steps:thread.steps[0].children,returnHref:'../scenarios/checkout.html#thread-step-receive'}];
 const r=load(data,true);r.run("showEntry('service-fragment-checkout:accept')");
 let html=r.e('scenario-content').innerHTML;
 assert.match(html,/href="#receiver-operation">Open full service operation: Handle incoming request/);
 assert.doesNotMatch(html,/Retained repository and outbox behavior/);
 r.run("location.hash='#receiver-operation';routeReaderHash()");
 assert.match(r.e('scenario-content').innerHTML,/Retained repository and outbox behavior/);
 data.operations=data.operations.filter(op=>op.id!==operation.id);
 const absent=load(data,true);absent.run("showEntry('service-fragment-checkout:accept')");
 assert.doesNotMatch(absent.e('scenario-content').innerHTML,/Open full service operation/);
});
test('contract source strings are inert and message frontier never invents a consumer',()=>{
 const data=contractThreadFixture(),c=data.processThread.boundaryContracts[0];c.receiverFragment=null;c.transport='KAFKA';c.operation.text='<script>execute()</script>';c.guarantees[0].missingEvidence='<img onerror="execute()">';
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/&lt;script&gt;execute\(\)&lt;\/script&gt;/);assert.match(html,/&lt;img onerror=/);
 assert.match(html,/Receiver capture is missing; no consumer is inferred/);
 assert.doesNotMatch(html,/<script>execute|<img onerror/);
});
test('Russian contract chrome retains authored field and path bytes',()=>{
 const data=contractThreadFixture();data.language='ru';const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Развернуть принимающую точку входа здесь/);assert.match(html,/Сценарии сбоя/);assert.match(html,/POST baseDomain \+ basePath \+ RESOURCE_PATH/);
});
test('fragment address separates process and fragment identity without hyphen collisions',()=>{
 const a=contractThreadFixture(),b=contractThreadFixture();
 a.processThread.processId='a-b';a.processThread.serviceFragments[0].id='c';a.processThread.boundaryContracts[0].receiverFragment='c';
 b.processThread.processId='a';b.processThread.serviceFragments[0].id='b-c';b.processThread.boundaryContracts[0].receiverFragment='b-c';
 const x=load(a),y=load(b);x.run("showEntry('process-overview')");y.run("showEntry('process-overview')");
 assert.match(x.e('scenario-content').innerHTML,/#service-fragment-a-b:c/);
 assert.match(y.e('scenario-content').innerHTML,/#service-fragment-a:b-c/);
});


test('frozen process reference opens the exact existing page without duplicating its narrative',()=>{
 const data=processFixture(false);
 data.processThread={status:'SOURCE_BOUND_DECLARATION',scope:'One source call and one existing service explanation',rootMethod:'client',methodOrder:['client'],methods:{client:{label:'Send request',source:{...source('fun send() = post()'),startLine:10,endLine:10}}},steps:[{id:'post',title:'Send request',text:'Declared receiving service',method:'client',startLine:10,endLine:10,processPage:'server'}],boundaries:[],processPages:{server:{title:'Existing process',service:'server',revision:'revision-server',href:'file:///existing%20publication/processes/accepted.html',proof:{pageDigest:'page-hash',exportDigest:'export-hash'},internalNarrativeGeneration:false}}};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/href="file:\/\/\/existing%20publication\/processes\/accepted.html"/);
 assert.match(html,/Existing explanation; its internal narrative is not regenerated/);
 assert.match(html,/revision-server/);assert.match(html,/page-hash/);assert.match(html,/export-hash/);
 assert.equal((html.match(/id="thread-source"/g)||[]).length,1);
 assert.equal((html.match(/fun send\(\)/g)||[]).length,1);
 data.processThread.processPages.server.href='javascript:alert(1)';
 const forged=load(data);forged.run("showEntry('process-overview')");
 assert.doesNotMatch(forged.e('scenario-content').innerHTML,/href="javascript:/);
});

test('internal discovery is absent by default even when internal candidates exist',()=>{
 const data=fixture();data.processCandidates={internal:[{name:'Helper candidate'}],summary:{internalCandidateCount:8}};
 const r=load(data);r.run("showEntry('section-overview')");
 assert.equal(r.run("entries.some(e=>e.id==='process-catalog')"),false);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/data-entry="process-catalog"|Internal processes/);
});
test('saved processes remain navigable without showing speculative internal discovery',()=>{
 const data=fixture();data.savedProcesses=[{id:'checkout',title:'Checkout workflow',href:'../scenarios/checkout.html',status:'AUTHORED',trigger:'Request received'}];
 data.processCandidates={internal:[{name:'Helper candidate'}],summary:{internalCandidateCount:8}};
 const r=load(data);r.run("showEntry('process-catalog')");
 assert.match(r.e('scenario-content').innerHTML,/Checkout workflow/);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/Internal candidates|Helper candidate/);
});
test('internal discovery is available only by explicit reader option and real candidates',()=>{
 const data=fixture();data.readerOptions={showInternalProcesses:true};
 data.processCandidates={internal:[{name:'Candidate',root:{status:'MISSING'},gaps:[],sourceIds:[],localCallTargetCount:1,controlEventCount:0}],summary:{internalCandidateCount:1}};
 const r=load(data);assert.equal(r.run("entries.some(e=>e.id==='process-catalog')"),true);
 r.run("showEntry('process-catalog')");assert.match(r.e('scenario-content').innerHTML,/Internal candidates/);
 const empty=fixture();empty.readerOptions={showInternalProcesses:true};const e=load(empty);
 assert.equal(e.run("entries.some(e=>e.id==='process-catalog')"),false);
});
test('source helpers remain accessible in inventory but do not demand separate navigation narratives',()=>{
 const data=fixture();data.catalogue=[{id:'http',kind:'HTTP_ENDPOINT',symbol:'Controller.accept',trigger:{methods:['POST'],paths:['/accept']},sourceIds:[]},{id:'helper',kind:'SOURCE_DECLARATION',symbol:'Mapper.convert',trigger:{},sourceIds:[]}];data.publicBoundaryIds=['http'];
 const r=load(data);assert.equal(r.run("entries.some(e=>e.id==='http')"),true);assert.equal(r.run("entries.some(e=>e.id==='helper')"),false);
 assert.equal(r.run('inventoryEntries().length'),2);r.run("catalogue();showEntry('helper')");
 assert.equal(r.run('current.id'),'helper');assert.match(r.e('scenario-content').innerHTML,/No separate narrative is required/);
});


test('process call bindings preserve compiler parameters, unknown transfer and inert expressions',()=>{
 const data=declaredThreadFixture();
 data.processExtraction={status:'COLLECTED_WITH_BOUNDARIES',methods:[{id:'root',symbol:'Caller.start',callBindings:[{callerMethodId:'root',targetIdentity:'Api.pick(String)',targetMethodId:'callee',targetStatus:'SAME_SCOPE_UNIQUE',sourceIds:['client-source'],argumentStatus:'COMPILER_PARAMETER_MAPPING',arguments:[{expression:'text = "<script>unsafe</script>"',parameter:'text',parameterIndex:1},{expression:'count = 7',parameter:'count',parameterIndex:0}],omittedDefaultParameterIndices:[2],defaultValues:'UNKNOWN',receiver:{type:'Api'},returnTransfer:{declaredType:'String'}}]},{id:'callee',symbol:'Api.pick(String)',sourceIds:['receiver-source']}],events:[],query:{},outcomes:[],frontier:[]};
 const r=load(data);r.run("showEntry('process-overview')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Compiler-selected calls and parameter bindings/);
 assert.match(html,/Api.pick\(String\)/);
 assert.match(html,/Receiving declaration/);
 assert.match(html,/Receiver type/);
 assert.match(html,/Declared return type/);
 assert.match(html,/&lt;script&gt;unsafe&lt;\/script&gt;/);
 assert.match(html,/Receiver object and runtime dispatch are unknown/);
 assert.match(html,/Return and exception transfer remain unproven/);
 assert.match(html,/Omitted default parameter indices/);
 data.language='ru';const ru=load(data);ru.run("showEntry('process-overview')");
 assert.match(ru.e('scenario-content').innerHTML,/Выбранные компилятором вызовы и параметры/);
});

function questionFixture(){
 const d=behaviorFixture();
 d.operations.push({...d.operations[0],id:'process-overview',title:'Saved wrapper'});
 d.catalogue.unshift({id:'process-overview',kind:'PROCESS',trigger:{methods:[]},symbol:'saved wrapper'});
 const block={title:'Supply a city',text:'Editorial <script>unsafe()</script>',evidence:[{kind:'RETAINED_CLAIM',title:'Original account',fragment:{text:'Original <img onerror=unsafe()>',id:'saved-claim'},sources:{old:source('ACCEPTED SOURCE')},state:{freshness:'STALE',verification:'UNASSESSED'}}],links:[{subject:'service:places',operation:'section-entities',href:'../services/places.html#section-entities',label:'Object context'}]};
 d.questionReader={title:'Save a city',purpose:'Follow the bounded save',authority:'EDITORIAL_READING_GUIDE_NOT_ACCEPTED_PROSE',sections:[{id:'story',title:'What happens?',blocks:[block]},{id:'confidence',title:'Source and confidence',blocks:[block]}],conclusion:{...block,title:'Local result'}};
 return d;
}
test('sequential reading retains the shared global header and search',()=>{
 const d=questionFixture();d.questionReader.presentation='SEQUENTIAL';d.questionReader.sections[0].number=1;
 const r=load(d);
 r.run("globalThis.sharedHeader={innerHTML:'shared navigation and global search',dataset:{}};document.querySelector=selector=>selector==='.reader-nav'?sharedHeader:null;sequentialNav()");
 assert.equal(r.run('sharedHeader.innerHTML'),'shared navigation and global search');
});
test('canonical model is primary without Data Flow and retains exact process return and legacy context',()=>{
 const d=questionFixture(),original=JSON.stringify({operations:d.operations,guide:d.questionReader});
 d.canonicalEntityLinks={'question-story':{places:[{id:'place',title:'Place',href:'../data-model/place.html'}]}};
 const r=load(d);r.run("location.search='?field=place.id';showEntry('process-overview')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/place.html\?fromProcess=renderer-fixture&amp;fromAnchor=question-story&amp;fromField=place.id/);
 assert.match(html,/Place · data model/);
 assert.match(html,/class="question-technical-link"[^>]+places.html\?fromProcess=renderer-fixture&amp;fromQuestion=story#section-entities/);
 assert.match(html,/Technical context/);
 assert.match(html,/title="Object context"/);
 assert.equal(JSON.stringify({operations:d.operations,guide:d.questionReader}),original);
 assert.equal(d.processDataFlow,undefined);
 const other=r.run("questionLink(D.questionReader.sections[1].blocks[0].links[0],'confidence')");
 assert.doesNotMatch(other,/data-model/);
 assert.match(other,/fromQuestion=confidence/);
});
test('explicit model reference binds a process question without replacing existing descriptions',()=>{
 const d=questionFixture(),before=JSON.stringify(d.questionReader);
 d.modelReferences=[{id:'lookup',title:'City lookup',anchor:'question-story',href:'../data-model/lookup.html',stages:['Lookup']}];
 const r=load(d),html=r.run("questionModelReferences('question-story')");
 assert.match(html,/City lookup/);
 assert.match(html,/fromProcess=renderer-fixture&amp;fromAnchor=question-story/);
 assert.equal(r.run("questionModelReferences('question-confidence')"),'');
 assert.equal(JSON.stringify(d.questionReader),before);
});
test('optional question profile has one main account, exact evidence and safe object return route',()=>{
 const d=questionFixture(),before=JSON.stringify(d.operations),r=load(d);
 r.run("showEntry('process-overview')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/question-reader/);assert.match(html,/What happens\?/);assert.match(html,/Local result/);
 assert.match(html,/Editorial &lt;script&gt;unsafe\(\)&lt;\/script&gt;/);
 assert.match(html,/Original &lt;img onerror=unsafe\(\)&gt;/);
 assert.match(html,/ACCEPTED SOURCE/);assert.match(html,/Full retained descriptions, notes and original views/);
 assert.match(html,/places.html\?fromProcess=renderer-fixture&amp;fromQuestion=story#section-entities/);
 assert.equal(JSON.stringify(d.operations),before);
 assert.equal(r.e('catalogue-button').hidden,true);
 assert.match(r.e('scenario-nav').innerHTML,/href="#question-story"/);
 r.run("revealQuestion('question-confidence')");
 assert.equal(r.document.activeElement.id,'question-confidence');assert.equal(r.e('question-confidence').scrolled,true);
 r.run("showEntry('reserve-fixture')");
 assert.match(r.e('scenario-content').innerHTML,/Back to the questions/);
});
test('question navigation ignores unknown destinations and preserves exact source line disclosure',()=>{
 const r=load(questionFixture());
 assert.equal(r.run("revealQuestion('question-missing')"),false);
 const html=r.run("questionSource({service:'dashboard',file:'compose.yaml',startLine:3,endLine:5,text:'before\\nPLACES_URL: http://places:8080\\nafter',revision:'revision-1234567',id:'config',textDigest:'digest'},4,4)");
 assert.match(html,/compose.yaml:4–4/);assert.match(html,/PLACES_URL: http:\/\/places:8080/);assert.doesNotMatch(html,/>before|after<\/pre>/);
});

test('active question uses every section after long evidence expansion and scroll',()=>{
 const r=load(questionFixture());r.run("showEntry('process-overview')");
 r.e('question-story').getBoundingClientRect=()=>({top:-902,bottom:-642,height:260});
 r.e('question-confidence').getBoundingClientRect=()=>({top:-602,bottom:520,height:1122});
 r.run("questionUpdateLocation()");assert.equal(r.e('question-select').value,'question-confidence');
 r.e('question-story').getBoundingClientRect=()=>({top:88,bottom:500,height:412});
 r.e('question-confidence').getBoundingClientRect=()=>({top:540,bottom:900,height:360});
 r.run("questionUpdateLocation()");assert.equal(r.e('question-select').value,'question-story');
});

test('a foreign-language original remains readable with stable step anchors and selected-locale controls',()=>{
 for(const available of ['en',null]){
  const d=questionFixture(),block=d.questionReader.sections[0].blocks[0];d.language='ru';
  d.questionReader={...d.questionReader,presentation:'SEQUENTIAL',language:available,sections:[
   {id:'request',title:'Forward request',number:1,blocks:[{...block,links:[]}]},
   {id:'save',title:'Save place',number:2,blocks:[block]},
   {id:'apply',title:'Apply response',number:3,blocks:[block]},
   {id:'confidence',title:'Original sources',blocks:[block]}]};
  d.questionReaderTranslationGap={requestedLanguage:'ru',availableLanguage:available,originalPath:'scenarios/example.reading-guide.original.json'};
  const before=JSON.stringify(d),r=load(d);r.run("showEntry('process-overview')");
  const html=r.e('scenario-content').innerHTML;
  for(const id of ['question-request','question-save','question-apply','question-confidence'])assert.ok(html.includes('id="'+id+'"'),id);
  for(const title of ['Forward request','Save place','Apply response'])assert.ok(html.includes(title),'original prose preserved');
  assert.match(html,/Исходный план чтения/);
  assert.match(html,/На этой странице/);assert.match(html,/Процессы/);
  assert.doesNotMatch(html,/\/ Weather \//,'owned breadcrumb must not invent a project');
  assert.match(html,/fromQuestion=save#section-entities/,'original technical cross-link preserved');
  r.run("revealQuestion('question-apply')");assert.equal(r.document.activeElement.id,'question-apply');
  assert.equal(JSON.stringify(d),before,'display policy never changes records or provenance');
 }
});

test('sequential profile keeps exceptions beside their steps and original descriptions on demand',()=>{
 const d=questionFixture(),block=d.questionReader.sections[0].blocks[0];
 d.questionReader={...d.questionReader,presentation:'SEQUENTIAL',language:'en',sections:[{id:'request',title:'Forward request',navTitle:'Request',number:1,icon:'arrow-right',route:'POST /api/places → POST /places',blocks:[{...block,links:[]},{...block,id:'exchange-error',navTitle:'Exchange failure · 503',tone:'ERROR',title:'Exchange failure',text:'503 leaves commit unknown'}]},{id:'save',title:'Save place',navTitle:'Save',number:2,icon:'database',blocks:[block]},{id:'confidence',title:'Original descriptions and source',blocks:[block]}]};
 const original=JSON.stringify(d.operations),r=load(d);r.run("showEntry('process-overview')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/sequential-reader/);assert.equal((html.match(/class="question-section sequential-step"/g)||[]).length,2);
 assert.ok(html.indexOf('id="question-exchange-error"')>html.indexOf('id="question-request"'));
 assert.ok(html.indexOf('id="question-exchange-error"')<html.indexOf('id="question-save"'));
 assert.equal((html.match(/id="question-exchange-error"/g)||[]).length,1);
 assert.match(html,/POST \/api\/places → POST \/places/);assert.equal((html.match(/class="sequential-icon"/g)||[]).length,2);assert.ok(html.includes(iconAssets['arrow-right']));assert.ok(html.includes(iconAssets.database));
 assert.match(html,/fromQuestion=save#section-entities/);
 assert.match(html,/Full retained descriptions, notes and original views/);assert.equal(JSON.stringify(d.operations),original);
 assert.match(html,/Editorial reading guide over retained source evidence/);assert.doesNotMatch(html,/Russian translation/);
 assert.match(r.e('scenario-nav').innerHTML,/Exchange failure · 503/);
 r.run("revealQuestion('question-exchange-error')");assert.equal(r.document.activeElement.id,'question-exchange-error');
});
test('active exception follows the nested viewport region rather than its parent step',()=>{
 const d=questionFixture(),block=d.questionReader.sections[0].blocks[0];
 d.questionReader.sections[0].blocks.push({...block,id:'exchange-error',tone:'ERROR'});
 const r=load(d);r.run("showEntry('process-overview')");
 r.e('question-story').getBoundingClientRect=()=>({top:-600,bottom:520,height:1120});
 r.e('question-exchange-error').getBoundingClientRect=()=>({top:-20,bottom:400,height:420});
 r.e('question-confidence').getBoundingClientRect=()=>({top:600,bottom:900,height:300});
 r.run('questionUpdateLocation()');assert.equal(r.e('question-select').value,'question-exchange-error');
});

test('explicit question selection survives scroll clamping until intentional user scrolling',()=>{
 const d=questionFixture(),block=d.questionReader.sections[0].blocks[0];
 d.questionReader.sections[0].blocks.push({...block,id:'unsupported-city',tone:'WARNING'});
 const r=load(d);r.run("showEntry('process-overview')");
 r.e('question-story').getBoundingClientRect=()=>({top:-100,bottom:500,height:600});
 r.e('question-unsupported-city').getBoundingClientRect=()=>({top:315,bottom:400,height:85});
 r.e('question-confidence').getBoundingClientRect=()=>({top:600,bottom:900,height:300});
 r.run("revealQuestion('question-unsupported-city');questionUpdateLocation()");
 assert.equal(r.e('question-select').value,'question-unsupported-city');
 assert.equal(r.document.activeElement.id,'question-unsupported-city');
 r.run('questionReleaseLocation();questionUpdateLocation()');
 assert.equal(r.e('question-select').value,'question-story');
 r.e('question-story').getBoundingClientRect=()=>({top:-700,bottom:500,height:1200});
 r.e('question-unsupported-city').getBoundingClientRect=()=>({top:-200,bottom:400,height:600});
 r.run('questionUpdateLocation()');assert.equal(r.e('question-select').value,'question-unsupported-city');
});
test('sequential inline code remains inert while retaining readable technical identifiers',()=>{
 const r=load(questionFixture());
 const html=r.run("sequentialText('Use `cityId` and `<img src=x onerror=alert(1)>` safely')");
 assert.match(html,/<code>cityId<\/code>/);assert.match(html,/&lt;img src=x onerror=alert\(1\)&gt;/);assert.doesNotMatch(html,/<img /);
});

function ingressFixture(){
 const data=fixture();data.sections.push({id:'section-ingress',title:'Entry points'});
 data.operations.push({...data.operations[0],id:'section-ingress',summary:fragment('Full accepted route description remains available.'),visuals:[]});
 const source=(id,method,path)=>({id,kind:'HTTP_ENDPOINT',symbol:id,trigger:{methods:[method],paths:[path],pathResolution:'DERIVED'},sourceIds:['same-source']});
 const declared=(id,method,path,entrypoint)=>({id,sourceIds:['same-source'],normalized:{method,path,entrypoint,sourceMapping:'SOURCE_ROUTE_MATCH_ONLY',declaredSource:'openapi.json',parameters:[],boundaries:[],operation:{responses:{'200':{description:'OK'},'500':{description:'Framework error'}}}}});
 data.boundaryInventory.publicBoundaries=[source('get-health','GET','/health'),source('post-item','POST','/items/{id}')];
 data.catalogue=data.boundaryInventory.publicBoundaries;
 data.contracts=[declared('health-contract','GET','/health','get-health'),declared('post-contract','POST','/items/{itemId}','post-item')];
 return {data,source,declared};
}
test('ingress joins routes once with separate behavior and declaration access, retaining original prose',()=>{
 const {data}=ingressFixture(),r=load(data);r.run("showEntry('section-ingress')");
 const html=r.e('scenario-content').innerHTML;
 assert.equal((html.match(/class="ingress-route"/g)||[]).length,2);
 assert.equal((html.match(/Route agrees/g)||[]).length,2);
 assert.match(html,/data-entry="post-item">Behavior/);assert.match(html,/data-entry="post-contract" data-tab="contract">Contract/);
 assert.match(html,/Full accepted entry-point description/);assert.match(html,/Full accepted route description remains available/);
 assert.match(html,/Status codes, body\/schema compatibility and runtime enforcement have not been verified/);
 assert.doesNotMatch(html,/ingress-warning|Discovered public boundaries|Declared OpenAPI operations/);
 r.click({entry:'post-contract',tab:'contract'});assert.match(r.e('scenario-content').innerHTML,/Framework error/);
 r.click({sources:'same-source'},true);assert.match(r.e('source-code').innerHTML,/CURRENT SOURCE/);
});
test('ingress warns about route differences without claiming an implementation is absent',()=>{
 const {data,source}=ingressFixture();data.boundaryInventory.publicBoundaries.push(source('new','DELETE','/items/{id}'));
 data.contracts[0].normalized.path='/Health';
 const r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/role="status"/);assert.match(html,/No matching OpenAPI declaration/);
 assert.match(html,/No source match in selected evidence/);assert.match(html,/does not prove that an implementation is absent/);
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows.filter(r=>r.status==='SOURCE_ONLY').length"),2);
});
test('ingress keeps HTTP methods, case, trailing slash and complex route semantics distinct',()=>{
 const {data,source,declared}=ingressFixture();
 data.boundaryInventory.publicBoundaries=[source('get','GET','/items/{id}'),source('post','POST','/items/{id}'),source('slash','GET','/items/'),source('constraint','GET','/items/{id:int}'),source('optional','GET','/items/{id?}'),source('catch-all','GET','/items/{*path}'),source('version','GET','/v1/items')];
 data.boundaryInventory.publicBoundaries.at(-1).trigger.apiVersions=['1'];
 data.contracts=[declared('plain','GET','/items/{other}','get'),declared('no-slash','GET','/items',null)];
 const r=load(data);
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows.filter(r=>r.status==='ROUTE_AGREES').length"),1);
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows.filter(r=>r.status==='UNCERTAIN').length"),4);
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows.find(r=>r.method==='POST').status"),'SOURCE_ONLY');
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows.find(r=>r.path==='/items/').status"),'SOURCE_ONLY');
});
test('ingress distinguishes ambiguous matches, conflicting source binding and unavailable OpenAPI',()=>{
 const {data,source}=ingressFixture();data.boundaryInventory.publicBoundaries.push(source('second-health','GET','/health'));
 data.contracts[1].normalized.entrypoint='other-target';let r=load(data);
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows[0].status"),'AMBIGUOUS');
 assert.equal(r.run("ingressRows(D.boundaryInventory,D.contracts).rows[1].status"),'BINDING_CONFLICT');
 data.contracts=[];data.boundaryInventory.publicBoundaries.push({id:'job',kind:'SCHEDULED_JOB',symbol:'nightly refresh',trigger:{methods:['CRON'],paths:[]},sourceIds:[]});
 r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Not compared · no OpenAPI operations available/);assert.doesNotMatch(html,/ingress-warning/);
 assert.match(html,/Other public boundaries · 1/);assert.match(html,/nightly refresh/);
});
function retainedHttpFixture(){
 const {data}=ingressFixture();data.selectedSnapshot='snapshot';data.selectedEvidence={revisions:{sample:'abc'}};
 data.contracts[0].digest='health-digest';data.contracts[1].digest='post-digest';
 data.contractComparison={schema:'codeclew-http-comparison-view/1.0',service:'sample',revision:'abc',snapshot:'snapshot',bindingStatus:'MATCHES_PINNED_SELECTED_SNAPSHOT',authority:'EXTERNAL_RECORDED_LOCAL_PROBE',scope:'Local harness; storage substituted.',validator:'external Draft 2020-12 validator',limitations:['No deployed runtime or database proof.'],sources:{'same-source':source('BOUND SOURCE')},artifacts:{probe:{digest:'sha256:probe',text:'</pre><script>unsafe()</script>'}},packetDigest:'sha256:packet',entries:[
  {entrypoint:'get-health',contractId:'health-contract',contractDigest:'health-digest',method:'GET',path:'/health',coverage:'HTTP not exercised; source inspected only.',cases:[],findings:[]},
  {entrypoint:'post-item',contractId:'post-contract',contractDigest:'post-digest',method:'POST',path:'/items/{itemId}',coverage:'Two observed cases; database failures not exercised.',cases:[
   {id:'extra',request:{body:'{"extra":true}'},response:{status:200,contentType:'application/json'},statusDeclared:true,mediaAssessment:'declared',bodyAssessment:'matches-selected-schema',requestAssessment:'outside-schema',comparedSchemaMedia:'application/json'},
   {id:'invalid',request:{body:'{}'},response:{status:400,contentType:'application/problem+json'},declaredMedia:['application/json'],statusDeclared:true,mediaAssessment:'undeclared-media',bodyAssessment:'matches-selected-schema',requestAssessment:'outside-schema',comparedSchemaMedia:'application/json'}],findings:[{kind:'BROADER_REQUEST_ACCEPTANCE',caseIds:['extra']},{kind:'UNDECLARED_RESPONSE_MEDIA',caseIds:['invalid']}]}
 ]};return data;
}
test('retained HTTP comparison displays bounded route findings, separate status/media/schema and complete provenance',()=>{
 const data=retainedHttpFixture(),r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
 assert.equal((html.match(/class="ingress-http-finding"/g)||[]).length,2);
 assert.equal((html.match(/Broader request acceptance/g)||[]).length,1);
 assert.match(html,/HTTP path not exercised/);assert.match(html,/Two observed cases; database failures not exercised/);
 assert.match(html,/Undeclared response media type/);assert.match(html,/Observed: <code>400 application\/problem\+json/);assert.match(html,/Declared for this status: <code>application\/json/);assert.match(html,/Recorded request/);assert.match(html,/400 · declared/);assert.match(html,/Matches selected schema/);
 assert.match(html,/does not show failure for a contract-valid request/);assert.match(html,/does not rerun that validator/);
 assert.match(html,/Raw recorded requests and responses/);assert.match(html,/http-contract-comparison.json/);
 assert.match(html,/No deployed runtime or database proof/);assert.match(html,/Exact selected source bindings/);
 assert.match(html,/&lt;script&gt;unsafe/);assert.doesNotMatch(html,/<script>unsafe/);
 assert.match(html,/Full accepted route description remains available/);
});
test('Russian retained HTTP comparison localizes computed findings while preserving probes and source bindings',()=>{
 const data=retainedHttpFixture();data.language='ru';
 data.contractComparison.scope='Source fragments';
 data.contractComparison.entries[1].coverage='Recorded request';
 data.contractComparison.entries[1].cases[0].request.body='{"value":"Snapshot"}';
 data.contractComparison.entries[1].cases[0].requestErrors=[{message:'External schema check'}];
 const before=JSON.stringify(data),r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Приём запросов за пределами схемы/);
 assert.match(html,/не доказывает сбой для запроса, соответствующего контракту/);
 assert.match(html,/Необъявленный тип содержимого ответа/);
 assert.doesNotMatch(html,/>Broader request acceptance<|>Undeclared response media type<|>Cases, evidence and limits</);
 for(const original of ['Source fragments','Recorded request','External schema check','Snapshot','application/json','application/problem+json','sha256:packet','sha256:probe'])assert.ok(html.includes(original),original);
 assert.match(html,/400 · объявлен/);assert.match(html,/Соответствует выбранной схеме/);
 assert.match(html,/&lt;script&gt;unsafe/);assert.doesNotMatch(html,/<script>unsafe/);
 assert.equal(r.run('JSON.stringify(publication)'),before);
 assert.equal(r.e('document-data').textContent,before);
});

test('retained HTTP comparison refuses stale snapshot, revision and integrity status without rendering findings',()=>{
 for(const [key,value] of [['snapshot','other'],['revision','other'],['bindingStatus','UNKNOWN']]){
  const data=retainedHttpFixture();data.contractComparison[key]=value;const r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
  assert.match(html,/findings are not applied/);assert.doesNotMatch(html,/class="ingress-http-finding"/);assert.doesNotMatch(html,/Broader request acceptance/);
 }
});
test('retained HTTP comparison never attaches findings to a mismatched or ambiguous ingress binding',()=>{
 for(const change of [d=>d.contracts[1].digest='new',d=>d.contractComparison.entries[1].entrypoint='other',d=>d.boundaryInventory.publicBoundaries.push({...d.boundaryInventory.publicBoundaries[1],id:'another-handler'})]){
  const data=retainedHttpFixture();change(data);const r=load(data);r.run("showEntry('section-ingress')");const html=r.e('scenario-content').innerHTML;
  assert.match(html,/No applicable recorded HTTP checks/);assert.doesNotMatch(html,/class="ingress-http-finding"/);
 }
});

test('selected route inventory supersedes an empty frozen inventory without rewriting retained evidence',()=>{
 const {data}=ingressFixture();
 data.selectedBoundaryInventory=structuredClone(data.boundaryInventory);
 data.selectedPublicBoundaryIds=data.catalogue.map(entry=>entry.id);
 data.boundaryInventory={publicBoundaries:[],gaps:[]};data.publicBoundaryIds=[];
 const r=load(data);r.run("showEntry('section-ingress')");
 assert.equal((r.e('scenario-content').innerHTML.match(/Route agrees/g)||[]).length,2);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/No source match in selected evidence/);
 assert.equal(r.run('D.boundaryInventory.publicBoundaries.length'),0);
 assert.equal(r.run('publicBoundaryIds.has("post-item")'),true);
});
test('an explicitly empty selected inventory never falls back to historical routes',()=>{
 const {data}=ingressFixture();data.selectedBoundaryInventory={publicBoundaries:[],gaps:[]};data.selectedPublicBoundaryIds=[];
 const r=load(data);r.run("showEntry('section-ingress')");
 assert.equal((r.e('scenario-content').innerHTML.match(/No source match in selected evidence/g)||[]).length,2);
 assert.doesNotMatch(r.e('scenario-content').innerHTML,/Route agrees/);
 assert.equal(r.run('publicBoundaryIds.size'),0);
});
test('operation links expose retained processes and existing data descriptions without inventing ownership',()=>{
 const {data}=ingressFixture();
 data.entryProcesses={'post-item':[{title:'Saved item journey',href:'../scenarios/item.html#process-overview',state:{freshness:'STALE',verification:'UNASSESSED'}}]};
 data.sections.push({id:'section-entities',title:'Domain entities'});
 data.operations.push({...data.operations[0],id:'post-item',summary:fragment('Original operation description'),visuals:[]},
  {...data.operations[0],id:'section-entities',summary:fragment('The item row stores an identifier and a value.'),visuals:[]});
 const original=JSON.stringify(data.operations),r=load(data);
 r.run("showEntry('section-ingress')");assert.match(r.e('scenario-content').innerHTML,/Saved process: Saved item journey/);
 r.click({entry:'post-item'});let html=r.e('scenario-content').innerHTML;
 assert.match(html,/Saved process: Saved item journey/);assert.match(html,/Service data and domain context/);assert.match(html,/Retained explanation/);
 r.click({entry:'section-entities'});html=r.e('scenario-content').innerHTML;
 assert.match(html,/The item row stores an identifier and a value/);
 assert.match(html,/Domain catalogue/);assert.match(html,/No structured business-object catalogue has been authored/);
 assert.doesNotMatch(html,/No domain entity identities have been declared/);
 assert.equal(r.run('JSON.stringify(D.operations)'),original);
});

test('OpenAPI literal and enum constraints never invent an object type',()=>{
 const ui=load();
 const html=ui.run(`fields({type:'object',required:['status'],properties:{status:{const:'QUEUED'},mode:{enum:['fast','safe']},empty:{},disabled:{const:false},nil:{const:null}}})`);
 assert.match(html,/const: &quot;QUEUED&quot;/);assert.match(html,/enum: \[&quot;fast&quot;,&quot;safe&quot;\]/);
 assert.match(html,/const: false/);assert.match(html,/const: null/);
 assert.match(html,/<code>status<\/code><\/td><td>Not declared<\/td><td class="required">required/);
 assert.equal(ui.run('type({})'),'Not declared');assert.equal(ui.run('type({$ref:"#/missing"})'),'#/missing');
 assert.match(ui.run('fields(false)'),/No value allowed/);
});
test('response JSON location respects operation path, escaped keys and unsupported input',()=>{
 const ui=load(),text=JSON.stringify({paths:{'/other':{post:{responses:{202:{description:'wrong'}}}},'/work/{id}':{post:{responses:{202:{description:'right',content:{'application/json':{schema:{properties:{status:{const:'QUEUED'}}}}}}}}}}},null,2);
 const location=ui.run(`jsonSourceLocation(${JSON.stringify(text)},['paths','/work/{id}','post','responses','202'])`);
 assert.equal(JSON.parse(text.slice(location.start,location.end)).description,'right');assert.ok(location.start>text.indexOf('wrong'));
 assert.equal(ui.run(`jsonSourceLocation('paths: yaml',['paths'])`),null);
 assert.equal(ui.run(`jsonSourceLocation('{"a":{},"a":{}}',['a'])`),null);
 assert.equal(ui.run(`jsonSourceLocation('{"a":{}}',['missing'])`),null);
});
test('response source opens its exact line span while preserving the complete retained bytes',()=>{
 const data=fixture(),text=JSON.stringify({openapi:'3.1.0',paths:{'/work':{post:{responses:{202:{description:'Queued',content:{'application/json':{schema:{properties:{status:{const:'QUEUED'}}}}}}}}}}},null,2);
 const record={...source(text),file:'contract.json',startLine:10,endLine:10+text.split('\n').length-1,url:'https://example.test/blob/abc/contract.json#L10-L99',textDigest:'retained-digest',evidenceDigest:'retained-evidence'};
 data.selectedSources={'contract-source':record};const before=JSON.stringify(data.selectedSources),ui=load(data);
 ui.run(`source(['contract-source'],'Response contract',true,'', '',['paths','/work','post','responses','202'])`);
 const location=ui.run(`jsonSourceLocation(${JSON.stringify(text)},['paths','/work','post','responses','202'])`),line=10+text.slice(0,location.start).split('\n').length-1;
 assert.match(ui.e('source-meta').innerHTML,new RegExp('#L'+line+'-L'));assert.match(ui.e('source-meta').innerHTML,/Selected response/);
 assert.match(ui.e('source-code').innerHTML,/openapi/);assert.match(ui.e('source-code').innerHTML,/QUEUED/);
 assert.equal(JSON.stringify(data.selectedSources),before);assert.match(ui.e('source-foot').innerHTML,/retained-digest/);
});

test('remote process sources open exact repository ranges without empty local code panes',()=>{
 const data=declaredThreadFixture();
 const revision='a'.repeat(40);
 for(const [id,method] of Object.entries(data.processThread.methods)){
  const record=method.source;record.text='';record.revision=revision;
  record.url=id==='client'?`https://github.com/example/client/blob/${revision}/src/Client.kt#L10`:`https://git.example:8443/gitlab/team/provider/-/blob/${revision}/src/Provider.kt#L20`;
 }
 data.processThread.steps[0].endLine=12;
 const reader=load(data);reader.run("showEntry('process-overview')");
 const html=reader.e('scenario-content').innerHTML;
 assert.match(html,new RegExp(`href="https://github.com/example/client/blob/${revision}/src/Client.kt#L10-L12"`));
 assert.match(html,new RegExp(`href="https://git.example:8443/gitlab/team/provider/-/blob/${revision}/src/Provider.kt#L20"`));
 assert.doesNotMatch(html,/href="#thread-source-[^"]*-L|class="thread-code"|class="thread-code-line"/);
 assert.match(html,/Source text is omitted from this export/);
 assert.equal(reader.run("threadSourceHref('receiver',20,23)"),`https://git.example:8443/gitlab/team/provider/-/blob/${revision}/src/Provider.kt#L20-23`);
});


test('remote single source opens the repository and multiple sources show links without copy controls',()=>{
 const data=fixture();data.sourceExport={mode:'remote'};
 data.sources['same-source']={...source(''),url:'https://github.com/example/repo/blob/'+ 'a'.repeat(40)+'/A.kt#L1-L3'};
 data.sources['other']={...source(''),file:'B.kt',url:'https://git.example/group/repo/-/blob/'+ 'b'.repeat(40)+'/B.kt#L4-9'};
 const r=load(data);r.e('source-select').parentElement=r.e('source-toolbar');
 r.click({sources:'same-source'});assert.deepEqual(r.opened,[[data.sources['same-source'].url,'_blank','noopener,noreferrer']]);assert.equal(r.e('source-panel').hidden,true);
 r.click({sources:'same-source other'});assert.equal(r.e('source-panel').hidden,false);assert.equal(r.e('source-code').hidden,true);assert.equal(r.e('source-toolbar').hidden,true);
 assert.match(r.e('source-meta').innerHTML,/B.kt#L4-9/);assert.match(r.e('source-foot').textContent,/Source text is omitted/);
});

test('bound interface card shows complete declaration apart from unchanged code checks and moves row sources on demand',()=>{
 const data=fixture(),op=data.operations[0];
 op.interfaceContracts=[{id:'checks',title:'Message checks',kind:'kafka',boundaries:[],rows:[{id:'row',label:'Validator rule',value:'Code accepts finite latitude; geographic bounds are not established here.',sourceIds:['same-source'],dependencyIds:['dep']}]}];
 const original=JSON.stringify(op.interfaceContracts);
 data.interfaceContractDeclarations={[op.id]:{checks:{id:'message',title:'Declared message',branchPointer:'/oneOf/1',source:{file:'events.schema.json',revision:'abc',url:'https://example.test/repo/blob/abc/events.schema.json',text:'EXACT SCHEMA'},declaration:{nodes:[
  {field:'$',type:'object',presence:'Root schema',nullability:'Excluded',constraints:{additionalProperties:false}},
  {field:'periods',type:'array',presence:'Required in parent',nullability:'Excluded',constraints:{minItems:1,maxItems:6}},
  {field:'periods[].rain',type:'["number","null"]',presence:'Required in parent',nullability:'Allowed',description:'Complete <literal> description',constraints:{minimum:0}}
 ],frontier:[],schema:{type:'object'}}}}};
 const r=load(data),html=r.run("interfaceContracts({id:'section-responsibilities'})");
 assert.equal((html.match(/class="contract-declaration-link"/g)||[]).length,1);
 assert.ok(!html.includes('<th>Evidence</th>'));
 assert.match(html,/periods\[\]\.rain/);assert.match(html,/Required in parent/);assert.match(html,/Allowed/);
 assert.match(html,/maxItems<\/code>: 6/);assert.match(html,/Complete &lt;literal&gt; description/);
 assert.ok(html.indexOf('Declared message')<html.indexOf('Source-derived context and checks'));
 const visible=html.split('<details class="contract-raw contract-evidence">')[0];assert.ok(!visible.includes('data-sources='));
 assert.match(html,/Detailed source bindings for the original descriptions/);assert.match(html,/data-sources="same-source"/);
 assert.equal(JSON.stringify(op.interfaceContracts),original);
 delete data.interfaceContractDeclarations;const fallback=load(data).run("interfaceContracts({id:'section-responsibilities'})");
 assert.match(fallback,/No declaration is bound/);assert.ok(!fallback.includes('inline-contract-table'));
 assert.match(fallback,/Code accepts finite latitude/);
});

test('payload behavior descriptions never count as schemas or erase known local roles',()=>{
 const data=fixture(),op=data.operations[0];data.language='ru';
 op.interfaceContracts=[{id:'local-result',title:'Local result',kind:'payload',rows:[{id:'result',label:'Return',value:'Construct a local result; HTTP serialization is not established.',sourceIds:[]}],boundaries:[]}];
 const before=JSON.stringify(data),r=load(data),html=r.run("interfaceContracts({id:'section-responsibilities'})");
 assert.match(html,/Описания данных и декларации · 1/);assert.match(html,/Описание данных/);
 assert.doesNotMatch(html,/1 schemas|1 схем|Схема данных/);assert.match(html,/Construct a local result/);
 assert.equal(JSON.stringify(data),before);
 assert.equal(r.run("t('These documentation mappings do not establish business ownership.')"),'Эти описания связей данных не определяют владельца в бизнес-системе.');
});

test('new inline declaration chrome localizes while schema values stay literal',()=>{
 const data=fixture();data.language='ru';const r=load(data);
 for(const key of ['Presence','Required in parent','Optional in parent','Each array item','Root schema','Excluded','Allowed','Not established'])assert.notEqual(r.run('t('+JSON.stringify(key)+')'),key,key);
 const c={id:'schema',title:'Authored schema title',branchPointer:'/oneOf/0',source:{file:'schema.json',revision:'r1',url:'https://example.test/schema'},declaration:{nodes:[{field:'mode',type:'string',presence:'Required in parent',nullability:'Excluded',constraints:{enum:['Allowed','Excluded']}}],frontier:[],schema:{}}};
 const html=r.run('inlineContractDeclaration('+JSON.stringify(c)+',"saved")');
 assert.match(html,/Обязательно в родительском объекте/);assert.match(html,/Не допускается/);
 assert.match(html,/\[&quot;Allowed&quot;,&quot;Excluded&quot;\]/);assert.match(html,/Authored schema title/);
});

test('service section body renders complete Markdown tables without sequence events or rewriting evidence',()=>{
 const data=fixture(),op=data.operations[0];
 op.explanation=[{id:'section-body',text:'| Field | Readable name | Description |\n| --- | --- | --- |\n| `cityId` | City identifier | Retained supported input. |\n| forecast | Latest forecast | May be absent. |',eventIds:[],dependencyIds:['retained-scope'],sourceIds:['same-source'],detail:false}];
 const before=JSON.stringify(data),reader=load(data);reader.run("showEntry('section-responsibilities')");
 const html=reader.e('scenario-content').innerHTML;
 assert.match(html,/<table class="clew-md-table/);assert.match(html,/<code>cityId<\/code>/);
 assert.match(html,/City identifier/);assert.match(html,/May be absent\./);
 assert.equal((html.match(/<td/g)||[]).length,6);assert.equal(JSON.stringify(data),before);
});

test('declared external boundary stays visible without source calls and links its participant page',()=>{
 const data=behaviorFixture(),op=data.operations[0];
 op.participants=[{id:'service',label:'Owned',service:'sample'},{id:'provider',label:'Provider <script>',service:null,externalService:'provider'}];
 op.events=[{id:'declared-request',kind:'declared-boundary',text:'Declared HTTP request',from:'service',to:'provider',interaction:'request',sourceIds:[],dependencyIds:['interaction:request']},{id:'declared-response',kind:'declared-boundary-return',text:'Explicit declared response',from:'provider',to:'service',interaction:'request',sourceIds:[],dependencyIds:['interaction:request']}];
 data.declaredRelationships={edges:[{id:'request',origin:'agent-proposal'}]};
 data.operationExternalCalls={[op.id]:op.events.map(e=>({eventId:e.id,interactionId:'request',declaration:{origin:'agent-proposal'},transport:{kind:'http'},declarationDigest:'declared-snapshot',snapshotContextDigest:'snapshot',authority:'USER_DECLARATION_NOT_RUNTIME_PROOF',retained:false,targetChanged:false}))};
 const retained=JSON.stringify(op),reader=load(data),svg=reader.run('diagram(D.operations[0],375)');
 assert.match(svg,/href="\.\.\/external-services\/provider\.html"/);
 assert.match(svg,/href="\.\.\/services\/sample\.html"/);
 assert.match(svg.replace(/<[^>]*>/g,' ').replace(/\s+/g,' '),/Declared boundary; runtime unverified/);
 assert.match(svg,/agent-proposal/);
 assert.match(svg,/Provider &lt;script&gt;/);
 assert.doesNotMatch(svg,/<script>/);
 assert.equal((svg.match(/class="event-arrow event-return"/g)||[]).length,2);
 assert.equal(JSON.stringify(data.operations[0]),retained);
 assert.match(svg,/data-boundary-declaration="boundary-evidence-reserve-fixture-declared-request"/);
 assert.match(reader.e('scenario-content').innerHTML,/Declared relationship evidence/);
 reader.click({boundaryDeclaration:'boundary-evidence-reserve-fixture-declared-request'});
 assert.equal(reader.e('boundary-evidence-reserve-fixture-declared-request').open,true);
 assert.equal(reader.e('boundary-evidence-reserve-fixture-declared-request').scrolled,true);
 assert.equal(reader.e('source-panel').hidden,true);
 assert.match(reader.run('pseudocodeEvent(D.operations[0],D.operations[0].events[0]).keyword'),/runtime unverified/);
});


test('Kafka external destination uses its registered page and never a fabricated service ingress',()=>{
 const data=fixture();data.externalServices={provider:{title:'External <provider>'}};const reader=load(data);
 const i={id:'event',origin:'human',runtime:'UNKNOWN',declaration:{external:true,externalService:'provider',from:{service:'sample'},to:{service:'provider'},transport:{kind:'kafka',topic:'requests'}}};
 const expression='topicBoundaryCard('+JSON.stringify([i])+',{direction:"outgoing"})';
 const html=reader.run(expression);assert.match(html,/href="\.\.\/external-services\/provider\.html"/);assert.doesNotMatch(html,/href="\.\.\/services\/provider\.html/);assert.match(html,/External &lt;provider&gt;/);assert.match(html,/Receiver source not captured/);
 reader.run('D.externalServices={}');const absent=reader.run(expression);assert.doesNotMatch(absent,/href="\.\.\/services\/provider\.html/);assert.doesNotMatch(absent,/href="\.\.\/external-services/);assert.match(absent,/Receiver source not captured/);
});


test('retained boundary origin and topic come from operation bindings after catalogue change or removal',()=>{
 const data=behaviorFixture(),op=data.operations[0],event={id:'old-event',kind:'declared-boundary',from:'service',to:'provider',interaction:'event',text:'Saved declared event',sourceIds:[],dependencyIds:['interaction:event']};op.events=[event];
 data.operationExternalCalls={[op.id]:[{eventId:'old-event',interactionId:'event',declaration:{origin:'human',rationale:'Saved declaration'},transport:{kind:'kafka',topic:'request-events'},declarationDigest:'old-digest',snapshotContextDigest:'old-context',retained:true,targetChanged:true}]};
 data.declaredRelationships={edges:[{id:'event',origin:'agent-proposal',transport:{kind:'kafka',topic:'request-events-v2'}}]};const r=load(data);
 assert.match(r.run('boundaryCaption(D.operations[0],D.operations[0].events[0])'),/human/);
 let evidence=r.run('boundaryEvidence(D.operations[0],D.operations[0].events[0])');assert.match(evidence,/old-digest/);assert.match(evidence,/request-events/);assert.match(evidence,/catalogue changed/);assert.doesNotMatch(evidence,/agent-proposal|request-events-v2/);
 r.run('D.declaredRelationships.edges=[]');assert.match(r.run('boundaryCaption(D.operations[0],D.operations[0].events[0])'),/human/);assert.match(r.run('boundaryEvidence(D.operations[0],D.operations[0].events[0])'),/old-digest/);
 r.run("D.operationExternalCalls[D.operations[0].id][0].declaration.origin='agent-proposal';D.declaredRelationships.edges=[{id:'event',origin:'human'}]");assert.match(r.run('boundaryCaption(D.operations[0],D.operations[0].events[0])'),/agent-proposal/);
});

test('explicit HTTP boundary response reuses only unambiguous same-operation binding and never current graph',()=>{
 const data=behaviorFixture(),op=data.operations[0];op.events=[{id:'response',kind:'declared-boundary-return',interaction:'request',from:'provider',to:'service',text:'Declared response',sourceIds:[],dependencyIds:['interaction:request']}];
 data.operationExternalCalls={[op.id]:[{eventId:'old-request',interactionId:'request',declaration:{origin:'agent-proposal'},transport:{kind:'http',path:'/old'},declarationDigest:'old-request-digest',snapshotContextDigest:'old-context',retained:true,targetChanged:true}]};data.declaredRelationships={edges:[{id:'request',origin:'human',transport:{kind:'http',path:'/new'}}]};const r=load(data);
 let evidence=r.run('boundaryEvidence(D.operations[0],D.operations[0].events[0])');assert.match(evidence,/SAME_OPERATION_INTERACTION/);assert.match(evidence,/old-request-digest/);assert.match(evidence,/\/old/);assert.doesNotMatch(evidence,/human|\/new/);assert.match(r.run('boundaryCaption(D.operations[0],D.operations[0].events[0])'),/agent-proposal/);
 r.run('D.operationExternalCalls[D.operations[0].id]=[]');evidence=r.run('boundaryEvidence(D.operations[0],D.operations[0].events[0])');assert.match(evidence,/No declaration binding was retained/);assert.doesNotMatch(evidence,/human|\/new|old-request-digest/);assert.match(r.run('boundaryCaption(D.operations[0],D.operations[0].events[0])'),/Provenance unavailable/);
});


test('bound OpenAPI interface card renders operations and nested schema fields without assuming root nodes',()=>{
 const data=fixture(),op=data.operations[0];
 op.interfaceContracts=[{id:'http',title:'HTTP',kind:'http',boundaries:[],rows:[]}];
 const declaration={kind:'OPENAPI_DOCUMENT',operations:[{method:'POST',path:'/cities/{id}',operationId:'save',parts:[{title:'Request body',required:true,declaration:{nodes:[{field:'cityId',type:'string',presence:'Required in parent',nullability:'Excluded',constraints:{enum:['oslo']}}]}},{title:'Response · 503',description:'Unavailable <script>'}]}],frontier:[],schema:{openapi:'3.1.0'}};
 data.interfaceContractDeclarations={[op.id]:{http:{id:'http',title:'HTTP declaration',source:{file:'openapi.json',revision:'r1'},declaration}}};
 const original=JSON.stringify(data),r=load(data),body=r.run('interfaceContracts({id:"section-responsibilities"})');
 assert.match(body,/POST \/cities\/\{id\}/);assert.match(body,/<code>cityId<\/code>/);assert.match(body,/Response · 503/);assert.match(body,/Unavailable &lt;script&gt;/);assert.doesNotMatch(body,/<script>/);
 assert.equal(JSON.stringify(data),original);
});


test('data-flow SVG and table show honest absence without rewriting legacy associations',()=>{
 const data=fixture(),before=JSON.stringify(data);
 const graph={definitionDigest:'view-digest',nodes:[
  {id:'domain',entity:'entity:order',kind:'domain',service:'sample',representation:'entity:order',meaning:fragment('Order')},
  {id:'trace',kind:'field',service:'sample',representation:'request.traceToken',meaning:fragment('Technical trace token')},
  {id:'context',entity:null,kind:'function',service:'sample',representation:'TraceContext',meaning:fragment('Technical context')}
 ],edges:[]};
 data.subject='scenario:trace-lineage';data.view={definition:{id:'trace-lineage',title:'Technical trace lineage',view:{scope:'Preserve technical values without claiming a business identity.',inputObjects:['entity:order'],human:{},limitations:[]}},definitionDigest:'view-digest',module:{limitations:[]},relatedComponents:[]};
 data.operations=[{...data.operations[0],id:'entity-dataflow',dataflow:graph}];data.sections=[];
 const original=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow')");
 const html=r.e('scenario-content').innerHTML,svg=r.run('dataflowDiagram(D.operations[0].dataflow)');
 assert.match(svg,/>entity:order<\/text>/);assert.equal((svg.match(/No domain association established/g)||[]).length,4);
 assert.match(html,/Choose an input, output or field/);r.run("selectFlow('node','trace')");assert.match(r.e('scenario-content').innerHTML,/No domain association established/);assert.equal(r.e('source-panel').hidden,true);
 assert.doesNotMatch(svg,/undefined| · null/);assert.equal(JSON.stringify(data),original);assert.notEqual(original,before);
});

function taskReaderFixture(language='en',view='domain'){
 const data=entitySectionFixture();data.language=language;
 const claim=text=>({text,evidence:[{kind:'SOURCE',service:'sample',sourceId:'same-source',textDigest:'owned',startLine:1,endLine:3}]});
 data.readerModels=[{id:'order',entity:'entity:synthetic-order',title:'Selected result',fieldCoverage:{status:'observed-fields',scope:'Only selected field uses; incoming DTO definition is not retained.',claim:claim('Partial observed fields')},processLinks:[{subject:'service:sample',anchor:'section-entities'}],fields:[{id:'value',path:'result.value',originalName:claim('value'),readableName:claim('Итоговое значение'),meaning:claim('Computed selected result'),representations:[{label:'Result.value',typeLabel:'Decimal',mapping:claim('Exact result expression')}],graphBindings:[{subject:'scenario:lineage',node:'result',claim:claim('Explicit field mapping')}],usage:[{stage:'Compute result',producer:'request',consumer:'response',condition:'Only when enabled',inputs:['request.amount','request.factor'],assessment:'observed-static-use',coveredScope:'Selected computation',claim:claim('Multiply amount by factor; use default factor one when absent'),processes:['scenario:lineage']}]},{id:'unknown',path:'request.unexamined',meaning:claim('Not investigated'),representations:[],usage:[]}]}];
 data.externalServices={provider:{title:'Provider selection profile',category:'system',identity:{group:'provider',title:'Provider',profile:'selection configuration',status:'proposed',declaration:{origin:'agent-proposal',rationale:'Explicit synthetic identity hypothesis'}}},renewal:{title:'Provider renewal profile',category:'system',identity:{group:'provider',title:'Provider',profile:'renewal configuration',status:'proposed',declaration:{origin:'agent-proposal',rationale:'Same hypothesis, distinct original profile'}}},store:{title:'Logical result storage',category:'storage'}};
 data.declaredRelationships={nodes:Object.keys(data.externalServices).map(id=>({id:'external-service:'+id,label:data.externalServices[id].title})),edges:[{id:'compute',from:'service:sample',to:'external-service:provider',title:'Compute selected result',transport:{kind:'http',method:'POST',path:'/compute',destinationConfigKey:'provider.endpoint'}},{id:'renew',from:'service:sample',to:'external-service:renewal',title:'Compute renewal result',transport:{kind:'http',path:'/renew'}},{id:'store',from:'service:sample',to:'external-service:store',title:'Attempt result write',transport:{kind:'logical'}},{id:'local',from:'service:sample',to:'service:sample',title:'Local self call',transport:{kind:'http'}}]};
 if(!data.sections.some(s=>s.id==='section-egress'))data.sections.push({id:'section-egress',title:'Outgoing systems and storage',gap:'Retained original egress description'});
 if(view==='flow'){
  const graph={definitionDigest:'view-digest',nodes:[{id:'input',readerRole:'input',kind:'dto',service:'sample',representation:'request',meaning:fragment('Request values')},{id:'rule',readerRole:'stage',kind:'function',service:'sample',representation:'compute',meaning:fragment('Compute selected value')},{id:'result',readerRole:'output',kind:'field',service:'sample',representation:'result.value',meaning:fragment('Selected result value')},{id:'isolated',kind:'domain',entity:'entity:order',service:'sample',representation:'entity:order',meaning:{...fragment('Declared order'),sourceIds:[]}}],edges:[{id:'input-rule',from:'input',to:'rule',kind:'read',authority:'SOURCE_INTERPRETATION',meaning:fragment('Use amount and factor'),uncertainty:''},{id:'rule-result',from:'rule',to:'result',kind:'transform',authority:'SOURCE_INTERPRETATION',meaning:fragment('Multiply amount by factor'),uncertainty:'Conditional on enabled computation'}]};
  data.subject='scenario:lineage';data.sections=[];data.view={definition:{id:'lineage',title:'Selected calculation lineage',view:{scope:'Owned calculation',inputObjects:['entity:order'],human:{},limitations:[]}},definitionDigest:'view-digest',module:{limitations:[]},relatedComponents:[]};data.operations=[{...data.operations[0],id:'entity-dataflow',title:'Selected calculation lineage',visuals:[],dataflow:graph}];
 }
 if(view==='process'){const behavior=behaviorFixture();behavior.language=language;for(const [index,event] of behavior.operations[0].events.entries())if(event.kind!=='end')event.reader={overview:index<5,label:fragment(event.kind==='declared'?'Compute result':event.kind==='return'?'Return prepared result':event.kind==='alt'?'When validation succeeds':'Inspect selected branch'),execution:event.kind==='declared'?{mode:'parallel-await',meaning:fragment('Wait for all required calculations')}:undefined};return behavior;}
 return data;
}
function polishFixture(language='en',state='unknown'){
 const data=taskReaderFixture(language),routes=httpNavigationFixture();
 data.title=language==='ru'?'Синтетический сервис мест':'Synthetic Places service';
 data.subtitle=language==='ru'?'Тест интерфейса; это не документация реального сервиса.':'Synthetic interface fixture, not documentation of a real service.';
 data.catalogue=routes.catalogue;data.operations.push(...routes.operations);
 if(language==='ru'){
  data.operations.find(o=>o.id==='named').title='Просмотр именованных мест';
  data.operations.find(o=>o.id==='natural-title').title='Получить отчёт о состоянии';
 }
 data.catalogue.push({id:'unknown-path',kind:'HTTP_ENDPOINT',symbol:'unresolvedHandler',trigger:{methods:['GET'],paths:[]},sourceIds:[]});
 data.operations.push({...routes.operations[0],id:'unknown-path',title:language==='ru'?'Путь не определён':'Path not established'});
 data.boundaryInventory={publicBoundaries:data.catalogue.filter(e=>e.kind==='HTTP_ENDPOINT'),gaps:[]};
 data.externalServices.queue={title:language==='ru'?'Очередь обновлений':'Refresh queue',category:'queue'};
 data.declaredRelationships.edges.push({id:'queue-send',from:'service:sample',to:'external-service:queue',title:'Publish refresh request',transport:{kind:'kafka',topic:'places.refresh.requested',destinationConfigKey:'places.refresh.messaging.destination.configuration.key'}});
 data.declaredRelationships.edges[0].transport.destinationConfigKey='places.providers.selection.remote.baseUrl';
 const freshness=state==='stale'?'STALE':'UNVERIFIED';
 const status={freshness,verification:'UNASSESSED',contentRevisions:{sample:'a'.repeat(40)},targetRevisions:{sample:'b'.repeat(40)},reasons:[{reason:state==='stale'?'SOURCE_CHANGED':'SOURCE_MISSING',service:'sample'}]};
 data.sectionState=status;data.operationStates=Object.fromEntries(data.operations.map(o=>[o.id,status]));
 if(state==='error')data.updateFailures={sample:{error:'Synthetic source read failed',scope:'Selected source check'}};
 return data;
}
function fieldInspectorFixture(language='en'){
 const data=polishFixture(language),model=data.readerModels[0],seed=model.fields[0];
 const claim=text=>({text,evidence:[{kind:'SOURCE',service:'sample',sourceId:'same-source',textDigest:'owned',startLine:1,endLine:3}]});
 const identifier=language==='ru'?'Идентификатор':'Identifier';
 const field=(id,path,meaning,label,use)=>({...structuredClone(seed),id,path,graphBindings:[],readableName:claim(identifier),meaning:claim(meaning),representations:label?[{label,typeLabel:'String',mapping:claim('Explicit synthetic representation mapping.')}]:[],usage:use?[{...structuredClone(seed.usage[0]),representation:label,inputs:[path],kinds:['forward'],stage:'Prepare synthetic payload',consumer:'receiver payload',coveredScope:'Synthetic field mapping only',claim:claim(use)}]:[]});
 model.title=language==='ru'?'Текущая запись':'Current record';
 model.fields=[field('current-id','record.id','Identifier on the current record.','CurrentRow.id','CURRENT_ONLY_USE: forward the current identifier unchanged.'),field('origin-id','request.sourceId','Identifier retained from the original request.','OriginDto.sourceId','ORIGIN_ONLY_USE: retain the source identifier separately.'),{...field('unexamined','record.unexamined','No use or representation has been recorded here.',null,null),readableName:claim(language==='ru'?'Неисследованное поле':'Unexamined field')},{...field('long-label','record.longLabel','A long label used for responsive layout checks.','CurrentRow.longLabel','LONG_LABEL_USE: keep the selected label.'),readableName:claim(language==='ru'?'Очень длинное читаемое имя поля для проверки переноса на узком экране':'A deliberately long readable field name for narrow screen wrapping')}];
 data.readerModels.push({...structuredClone(model),id:'other-record',entity:undefined,title:language==='ru'?'Другая локальная запись':'Other local record',fields:[field('current-id','other.id','An unrelated explicit model field.','OtherRow.id','OTHER_MODEL_ONLY_USE: use the unrelated model field.')]});
 data.readerModels.push({...structuredClone(model),id:'ambiguous',entity:undefined,title:language==='ru'?'Неоднозначная запись':'Ambiguous record',fields:[field('same-id','first.id','First retained duplicate record.','First.id','FIRST_DUPLICATE_USE'),field('same-id','second.id','Second retained duplicate record.','Second.id','SECOND_DUPLICATE_USE')]});
 return data;
}
if(process.env.CODECLEW_FIELD_INSPECTOR_FIXTURE_OUT){
 const out=path.resolve(process.env.CODECLEW_FIELD_INSPECTOR_FIXTURE_OUT);fs.mkdirSync(out,{recursive:true});
 const sharedStyle=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.css'),'utf8'),sharedReader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 for(const language of ['en','ru'])fs.writeFileSync(path.join(out,'fields-'+language+'.html'),renderReaderFixture(fieldInspectorFixture(language)).replace('</style>',()=>sharedStyle+'</style>').replace('</body>',()=>'<script>'+sharedReader+'</script></body>'));
}
if(process.env.CODECLEW_POLISH_FIXTURE_OUT){
 const out=path.resolve(process.env.CODECLEW_POLISH_FIXTURE_OUT);fs.mkdirSync(out,{recursive:true});
 const sharedStyle=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.css'),'utf8');
 const sharedReader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');
 for(const language of ['en','ru'])for(const state of ['unknown','stale','error']){
  const html=renderReaderFixture(polishFixture(language,state)).replace('</style>',()=>sharedStyle+'</style>').replace('</body>',()=>'<script>'+sharedReader+'</script></body>');
  fs.writeFileSync(path.join(out,state+'-'+language+'.html'),html);
 }
}
if(process.env.CODECLEW_TASK_READER_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_TASK_READER_FIXTURE_OUT);fs.mkdirSync(output,{recursive:true});
 for(const language of ['en','ru'])for(const view of ['domain','flow','process'])fs.writeFileSync(path.join(output,view+'-'+language+'.html'),renderReaderFixture(taskReaderFixture(language,view)));
}
test('domain catalogue leads preserved prose, binds only explicit models and labels partial coverage',()=>{
 const data=taskReaderFixture(),before=JSON.stringify(data),r=load(data);r.run("showEntry('section-entities')");const html=r.e('scenario-content').innerHTML;
 assert.ok(html.indexOf('domain-catalogue')<html.indexOf('original-service-description'));
 assert.match(html,/Only selected field uses/);assert.match(html,/result.value/);assert.match(html,/Итоговое значение/);assert.match(html,/Unrecorded actions are unknown/);
 assert.doesNotMatch(html,/grouped-egress|declared-relationships/);assert.equal(JSON.stringify(data),before);
 data.readerModels[0].entity=undefined;const other=load(data);other.run("showEntry('section-entities')");assert.match(other.e('scenario-content').innerHTML,/Computed selected result/);assert.match(other.e('scenario-content').innerHTML,/These documentation mappings do not establish business ownership/);data.readerModels[0].processLinks=[];assert.doesNotMatch(load(data).run('domainCatalogue()'),/Computed selected result/);
});
test('representation catalogue shares generic ownership limits once outside object cards',()=>{
 const data=taskReaderFixture();data.entities=[];data.readerModels[0].entity=undefined;
 data.readerModels.push({...structuredClone(data.readerModels[0]),id:'other-record',title:'Other local result'});
 const r=load(data),html=r.run('domainCatalogue()');
 assert.equal((html.match(/class="domain-data-scope"/g)||[]).length,1);
 assert.equal((html.match(/These documentation mappings do not establish business ownership/g)||[]).length,1);
 for(const card of html.split('class="domain-data-object"').slice(1))assert.doesNotMatch(card,/Business ownership|Service role and ownership|documentation mappings do not establish/);
 assert.match(html,/Other local result/);assert.match(html,/Computed selected result/);
});
test('egress groups profiles as explicit hypothesis, rejects grouping without rewriting IDs and hides self calls',()=>{
 const data=taskReaderFixture(),before=JSON.stringify(data),r=load(data);r.run("showEntry('section-egress')");let html=r.e('scenario-content').innerHTML;
 assert.ok(html.indexOf('grouped-egress')<html.indexOf('original-service-description'));assert.equal((html.match(/<h4>Provider<\/h4>/g)||[]).length,1);assert.match(html,/Identity hypothesis/);assert.match(html,/provider.endpoint/);assert.match(html,/POST <code>\/compute/);assert.match(html,/Logical result storage/);assert.doesNotMatch(html,/Local self call/);assert.equal(JSON.stringify(data),before);
 data.externalServices.renewal.identity.status='rejected';const rejected=load(data);assert.equal(rejected.run('egressGroups().length'),3);assert.equal(data.externalServices.renewal.identity.profile,'renewal configuration');
});
test('DataFlow defaults to selectable overview and selection focuses context without opening source',()=>{
 const data=taskReaderFixture('en','flow'),before=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow')");let html=r.e('scenario-content').innerHTML;
 assert.doesNotMatch(html,/<svg|Representations and source bindings/);assert.match(html,/data-flow-select="isolated"/);assert.doesNotMatch(html.slice(html.indexOf('flow-overview'),html.indexOf('flow-all-nodes')),/Declared order/);const noRoles=taskReaderFixture('en','flow');for(const node of noRoles.operations[0].dataflow.nodes)delete node.readerRole;const plain=load(noRoles);plain.run("showEntry('entity-dataflow')");assert.match(plain.e('scenario-content').innerHTML,/Objects and values/);assert.doesNotMatch(plain.e('scenario-content').innerHTML,/<h3>Inputs<\/h3>|<h3>Outputs and destinations<\/h3>/);
 r.click({flowSelect:'result'});html=r.e('scenario-content').innerHTML;assert.equal(r.e('source-panel').hidden,true);assert.match(html,/Multiply amount by factor/);assert.match(html,/Open source/);assert.doesNotMatch(html,/Use amount and factor/);
 r.click({flowExpand:''});assert.match(r.e('scenario-content').innerHTML,/Use amount and factor/);
 r.click({flowField:'order/value'});html=r.e('scenario-content').innerHTML;assert.match(html,/request.amount/);assert.match(html,/default factor one/);assert.match(html,/Only when enabled/);assert.match(html,/Field mappings are explicit claims/);
 r.click({flowField:'order/unknown'});assert.doesNotMatch(r.e('scenario-content').innerHTML,/Field use has not been investigated/);assert.doesNotMatch(r.e('scenario-content').innerHTML,/Not used in/);
 r.run("selectFlow('node','isolated')");assert.equal(r.e('source-panel').hidden,true);assert.match(r.e('scenario-content').innerHTML,/Declared order/);r.run("selectFlow('reset')");assert.doesNotMatch(r.e('scenario-content').innerHTML,/<svg/);assert.equal(r.run('JSON.stringify(D)'),before);
});
test('compact process uses authored overview and independently evidenced execution without stripping original',()=>{
 const data=behaviorFixture(),op=data.operations[0],event=op.events.find(e=>e.kind==='declared');event.reader={overview:true,label:fragment('Compute required result'),execution:{mode:'parallel-await',meaning:fragment('Wait for required results')}};
 const before=JSON.stringify(data),r=load(data),html=r.run('compactProcess(D.operations[0])');assert.match(html,/Compute required result/);assert.match(html,/parallel-await/);assert.match(html,/Wait for required results/);assert.match(html,new RegExp(event.text.replace(/[.*+?^${}()|[\]\\]/g,'\\$&')));assert.equal(r.run('JSON.stringify(D)'),before);
});

test('compact boundary label opens its own source without promoting declaration evidence',()=>{
 const data=boundaryLayoutFixture(),op=data.operations[0],event=op.events[0];
 event.reader={overview:true,label:{id:'boundary-label',text:'Request reservation',dependencyIds:['caller'],sourceIds:['label-source']}};
 data.operationSources[op.id]={'label-source':source('Response submit() { return client.call(); }')};
 const before=JSON.stringify(data),r=load(data);r.run(`showEntry(${JSON.stringify(op.id)})`);
 const html=r.run('compactProcess(D.operations[0])');
 assert.match(html,/Label evidence/);assert.match(html,/Declared relationship evidence/);
 const tag=html.match(/<button[^>]*data-source-claim="boundary-label"[^>]*>/)[0];
 const dataset=Object.fromEntries([...tag.matchAll(/data-([a-z-]+)="([^"]*)"/g)].map(([,key,value])=>[key.replace(/-([a-z])/g,(_,c)=>c.toUpperCase()),value]));
 r.click(dataset);assert.equal(r.e('source-select').value,'label-source');
 assert.match(r.e('source-code').innerHTML,/client\.call/);assert.equal(JSON.stringify(data),before);
 assert.deepEqual(event.sourceIds,[]);assert.deepEqual(event.dependencyIds,['interaction:request']);
});

test('compact domain rows disclose original meaning and exact fields before usage without rewriting data',()=>{
 const data=taskReaderFixture('ru'),before=JSON.stringify(data),r=load(data),html=r.run('domainCatalogue()');
 assert.match(html,/<details class="domain-object"><summary>/);assert.match(html,/domain-object-name/);assert.match(html,/domain-code-term/);assert.match(html,/domain-object-comment/);
 assert.ok(html.indexOf('field-glossary')<html.indexOf('class="field-usage"'));
 assert.match(html,/<details class="field-coverage"><summary>Только найденные обращения к полям/);assert.match(html,/Partial observed fields/);assert.match(html,/Only selected field uses/);
 assert.equal(JSON.stringify(data),before);
});
test('selected field leads with input rule output and keeps definition cautions and object context secondary',()=>{
 const data=taskReaderFixture('ru','flow'),before=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow');selectFlow('field','order/value')");const html=r.e('scenario-content').innerHTML;
 assert.ok(html.indexOf('request.amount')<html.indexOf('class="field-definition"'));
 assert.ok(html.indexOf('Multiply amount by factor')<html.indexOf('class="flow-object-context"'));
 assert.match(html,/<details class="flow-object-context"><summary>Контекст объекта целиком/);
 assert.match(html,/<details class="field-use-evidence">/);assert.match(html,/Only selected field uses/);assert.match(html,/Partial observed fields/);
 assert.equal(JSON.stringify(data),before);
});
test('process detail shows original once and moves separate execution interpretation and proofs into one disclosure',()=>{
 const data=behaviorFixture(),event=data.operations[0].events[0];event.reader={overview:true,label:fragment('Validate request'),execution:{mode:'synchronous',meaning:fragment(event.text)}};
 const before=JSON.stringify(data),r=load(data),html=r.run('compactProcess(D.operations[0])');
 assert.equal(html.replace(/<[^>]*>/g,'').split(event.text).length-1,1);assert.match(html,/class="process-step-evidence"/);assert.ok(html.indexOf('process-step-evidence')<html.indexOf('Label evidence'));
 event.reader.execution.meaning.text='A separate timing explanation';const other=load(data).run('compactProcess(D.operations[0])');assert.ok(other.indexOf('process-step-evidence')<other.indexOf('A separate timing explanation'));assert.equal(JSON.stringify(data.operations[0].events[0].text),JSON.stringify(JSON.parse(before).operations[0].events[0].text));
});
test('sequence uses compact labels while retaining surviving negative guards and unbroken ordinary code tokens',()=>{
 const data=behaviorFixture();data.language='ru';const op=data.operations[0];
 op.events=[{id:'guard',kind:'alt',text:'Original supplied version guard',reader:{label:fragment('Версия задана')},sourceIds:['summary-source']},{id:'note',kind:'note',text:'Retain version',sourceIds:['summary-source']},{id:'else',kind:'else',text:'Otherwise',reader:{label:fragment('Получить активную версию')},sourceIds:['summary-source']},{id:'call',kind:'message',text:'Original long meaning retained in source inspection',reader:{label:fragment('POST SyntheticSelectionRequest calculateRequiredResult /api/versions/active')},from:'caller',to:'service',sourceIds:['summary-source']},{id:'end',kind:'end',text:'',sourceIds:[]}];
 const before=JSON.stringify(data),r=load(data),layout=JSON.parse(r.run('JSON.stringify(sequenceLayout(D.operations[0],390))'));
 const guard=layout.rows[0].lines.join(' ');assert.match(guard,/Условие не выполнено: Версия задана/);assert.match(guard,/Выбранная ветвь: Получить активную версию/);
 const call=layout.rows.find(row=>row.e.id==='call');assert.ok(call.lines.some(line=>line.includes('SyntheticSelectionRequest')));assert.ok(call.lines.some(line=>line.includes('calculateRequiredResult')));
 assert.match(r.run('diagram(D.operations[0],390)'),/Original long meaning retained/);assert.equal(JSON.stringify(data),before);
});
test('Kafka Russian chrome localizes proposed delivery and keeps uncertainty with retained declaration evidence',()=>{
 const data=fixture();data.language='ru';const r=load(data),row={id:'synthetic-event',origin:'agent-proposal',runtime:'UNKNOWN',topic:'synthetic.result',declaration:{from:{service:'sample',selector:{name:'send'}},to:{service:'provider'},external:true,externalService:'provider',transport:{kind:'kafka',topic:'synthetic.result',purpose:'Retain a synthetic result'}}};
 const html=r.run(`topicBoundaryCard(${JSON.stringify([row])},{direction:'outgoing'})`);
 assert.match(html,/Сохранённые исходники и контекст декларации/);assert.match(html,/Предлагаемая передача/);assert.match(html,/доставка во время исполнения не проверена/);assert.doesNotMatch(html,/Proposed handoff|runtime delivery|declaration context/);assert.ok(html.indexOf('technical-evidence')<html.indexOf('доставка во время исполнения'));
});
function compactUxFixture(language,view){
 if(view==='kafka'){
  const data=taskReaderFixture(language);data.interactions=[{id:'synthetic-result',origin:'agent-proposal',runtime:'UNKNOWN',topic:'synthetic.result',declaration:{from:{service:'sample',selector:{name:'publishResult'}},to:{service:'provider'},external:true,externalService:'provider',transport:{kind:'kafka',topic:'synthetic.result',purpose:language==='ru'?'Передать наблюдаемый результат. Доставка и ключ не установлены.':'Transfer an observed result. Delivery and key are unknown.'}}}];return data;
 }
 if(view==='process'){
  const data=taskReaderFixture(language,view),op=data.operations[0],ru=language==='ru';
  op.participants=[{id:'caller',label:ru?'Вызывающая сторона':'Caller'},{id:'service',label:ru?'Обработка заявки':'Request processing',service:'sample'},{id:'provider',label:ru?'Внешний расчёт':'External calculation',externalService:'synthetic-pricing-provider'},{id:'audit',label:ru?'Фоновый аудит':'Background audit',externalService:'synthetic-audit-store'}];
  const event=(id,kind,text,from,to,label,mode)=>({id,kind,text,from,to,sourceIds:['summary-source'],dependencyIds:[],...(kind==='end'?{}:{reader:{overview:true,label:{...fragment(label||text),sourceIds:['summary-source']},...(mode?{execution:{mode,meaning:{...fragment(text),sourceIds:['summary-source']}}}:{})}})});
  op.events=[event('incoming','message',ru?'Принять SyntheticSelectionRequest и проверить обязательные входные значения.':'Accept SyntheticSelectionRequest and validate required inputs.','caller','service',ru?'Принять SyntheticSelectionRequest':'Accept SyntheticSelectionRequest'),event('guard','alt',ru?'Версия указана во входе':'Version supplied',null,null),event('note','note',ru?'Сохранить входную версию':'Keep supplied version','service','service'),event('alternative','else',ru?'Версия не указана':'Version not supplied',null,null),event('resolve','message',ru?'Вызвать calculateRequiredResult(request) для выбранной версии.':'Call calculateRequiredResult(request) for the selected version.','service','service','calculateRequiredResult(request)','synchronous'),event('end','end','',null,null),event('required','declared',ru?'Дождаться обязательных результатов расчёта перед формированием ответа.':'Wait for required calculation results before constructing the response.','service','provider',ru?'Обязательные расчёты':'Required calculations','parallel-await'),event('background','declared',ru?'Запустить аудит в фоне, не ожидая его результата перед ответом.':'Start audit in the background without awaiting its result before the response.','service','audit',ru?'Фоновый аудит':'Background audit','background'),event('return','return',ru?'Вернуть подготовленный результат после обязательных расчётов.':'Return the prepared result after required calculations.','service','caller',ru?'Вернуть результат':'Return result')];return data;
 }
 if(view!=='domain')return taskReaderFixture(language,view);
 const data=taskReaderFixture(language),original=data.entities[0],model=data.readerModels[0];
 data.entities=Array.from({length:7},(_,i)=>{const d=JSON.parse(JSON.stringify(original));d.id='entity:synthetic-domain-'+i;d.normalized.entity.id='synthetic-domain-'+i;d.normalized.entity.title=(language==='ru'?'Результат обработки ':'Processing result ')+(i+1);d.normalized.entity.description=language==='ru'?'Наблюдаемые значения результата и условия их использования сервисом. Полное определение внешнего типа не установлено.':'Observed result values and their service usage conditions. The full external type definition is not established.';for(const r of d.normalized.entity.relations)r.representations=['SyntheticCalculationResult — result.value → response.value','SyntheticResponse'];return d;});
 data.readerModels=data.entities.map((d,i)=>({...JSON.parse(JSON.stringify(model)),id:'model-'+i,entity:d.id,title:d.normalized.entity.title}));return data;
}
if(process.env.CODECLEW_COMPACT_UX_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_COMPACT_UX_FIXTURE_OUT);fs.mkdirSync(output,{recursive:true});
 for(const language of ['en','ru'])for(const view of ['domain','flow','process','kafka'])fs.writeFileSync(path.join(output,view+'-'+language+'.html'),renderReaderFixture(compactUxFixture(language,view)));
}

function boundedFlowFixture(language='en'){
 const data=taskReaderFixture(language,'flow'),g=data.operations[0].dataflow;data.readerModels=[];
 const node=(id,label,role)=>({id,kind:'dto',service:'sample',representation:id==='request'?'SyntheticRequest':id,meaning:fragment(label),...(role?{readerRole:role}:{})});
 g.nodes=[node('request',language==='ru'?'Исходная заявка':'Input request','input')];g.edges=[];
 for(let i=0;i<13;i++){
  const id=i===12?'combined':'value-'+i;g.nodes.push(node(id,(language==='ru'?'Производное значение ':'Derived value ')+i));g.nodes.push(node('next-'+i,(language==='ru'?'Следующее значение ':'Next value ')+i));
  g.edges.push({id:'request-'+i,from:'request',to:id,kind:'transform',authority:'SOURCE_INTERPRETATION',meaning:fragment(i===12?'Combine contributor inputs':'Derive value '+i),uncertainty:''});
  g.edges.push({id:'next-edge-'+i,from:id,to:'next-'+i,kind:'transform',authority:'SOURCE_INTERPRETATION',meaning:fragment('Use derived value '+i),uncertainty:''});
 }
 for(let i=0;i<6;i++){g.nodes.push(node('contributor-'+i,'Contributor '+i));g.edges.push({id:'contribution-'+i,from:'contributor-'+i,to:'combined',kind:'transform',authority:'SOURCE_INTERPRETATION',meaning:fragment('Combine contributor inputs'),uncertainty:''});}
 return data;
}
test('bounded high-degree connection view retains every branch and stays bounded after one further hop',()=>{
 const data=boundedFlowFixture(),before=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow');selectFlow('node','request');selectFlow('direction','outputs')");
 const originalFocused=JSON.parse(r.run('JSON.stringify(focusedFlow(D.operations[0].dataflow))'));assert.equal(originalFocused.edges.length,13);
 let html=r.e('scenario-content').innerHTML;assert.match(html,/Choose a connection/);assert.match(html,/1\/13/);assert.match(html,/data-shown-nodes="2"/);assert.doesNotMatch(html,/class="dataflow-scroll"/);
 for(const edge of originalFocused.edges){r.run(`selectFlow('connection',${JSON.stringify(edge.id)})`);html=r.e('scenario-content').innerHTML;assert.match(html,new RegExp('data-flow-select="'+edge.to+'"'));assert.equal(r.e('source-panel').hidden,true);assert.ok([...html.matchAll(/data-shown-nodes="(\d+)"/g)].every(m=>Number(m[1])<=4));}
 r.run("selectFlow('expand')");html=r.e('scenario-content').innerHTML;assert.match(html,/next-edge-12/);assert.ok([...html.matchAll(/data-shown-nodes="(\d+)"/g)].every(m=>Number(m[1])<=4));
 assert.equal(JSON.stringify(data),before);
});
test('bounded multi-input view pages contributors without dropping their rule evidence or inventing a path',()=>{
 const data=boundedFlowFixture(),before=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow');selectFlow('node','combined')");
 let html=r.e('scenario-content').innerHTML;assert.match(html,/Contributor inputs/);assert.match(html,/1–3 \/ 7/);assert.match(html,/data-shown-nodes="4"/);assert.match(html,/Other contributors remain part/);
 r.run("selectFlow('input-page',2)");html=r.e('scenario-content').innerHTML;assert.match(html,/7–7 \/ 7/);assert.match(html,/data-shown-nodes="2"/);assert.equal(r.e('source-panel').hidden,true);
 const selfData=JSON.parse(JSON.stringify(data));selfData.operations[0].dataflow.edges.push({id:'self',from:'combined',to:'combined',kind:'read',authority:'SOURCE_INTERPRETATION',meaning:fragment('Read same object'),uncertainty:''});const self=load(selfData);self.run("showEntry('entity-dataflow');selectFlow('node','combined');selectFlow('connection','self')");assert.match(self.e('scenario-content').innerHTML,/returns to the same object/);
 assert.equal(JSON.stringify(data),before);
});
test('mobile sequence trace preserves all non-end events, route names, guards and explicit execution modes',()=>{
 const data=compactUxFixture('ru','process'),before=JSON.stringify(data),r=load(data,true),op=data.operations[0],html=r.run('sequenceTrace(D.operations[0])');
 assert.deepEqual([...html.matchAll(/data-trace-event="([^"]+)"/g)].map(m=>m[1]),op.events.filter(e=>e.kind!=='end').map(e=>e.id));
 for(const actor of op.participants)assert.ok(html.includes(actor.label));assert.match(html,/Вызов/);assert.match(html,/Возврат/);assert.match(html,/Условие не выполнено/);assert.match(html,/Параллельно с ожиданием/);assert.match(html,/Без ожидания/);
 const adaptive=r.run("adaptiveSequence(D.operations[0],'<div>FULL_DIAGRAM</div>')");assert.match(adaptive,/data-presentation="narrow"/);assert.doesNotMatch(adaptive,/<div class="sequence-trace-panel" hidden|<details class="full-sequence-diagram" open/);assert.match(adaptive,/Полная диаграмма последовательности/);assert.equal(JSON.stringify(data),before);
 const wide=load(data).run("adaptiveSequence(D.operations[0],'<div>FULL_DIAGRAM</div>')");assert.match(wide,/<div class="sequence-trace-panel" hidden>/);assert.match(wide,/<details class="full-sequence-diagram" open>/);
});
if(process.env.CODECLEW_BOUNDED_FLOW_TRACE_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_BOUNDED_FLOW_TRACE_FIXTURE_OUT);fs.mkdirSync(output,{recursive:true});
 for(const language of ['en','ru']){fs.writeFileSync(path.join(output,'flow-'+language+'.html'),renderReaderFixture(boundedFlowFixture(language)));fs.writeFileSync(path.join(output,'trace-'+language+'.html'),renderReaderFixture(compactUxFixture(language,'process')));}
}
test('Russian bounded graph controls retain stable identities and all branches remain interactive',()=>{
 const data=boundedFlowFixture('ru'),before=JSON.stringify(data),r=load(data);r.run("showEntry('entity-dataflow');selectFlow('node','request')");
 let html=r.e('scenario-content').innerHTML;assert.match(html,/id="dfcpicker" data-dfcpicker/);assert.match(html,/class="dfcgraph"/);assert.match(html,/data-dfcshift="1"/);
 r.click({dfcshift:'1'});html=r.e('scenario-content').innerHTML;assert.match(html,/2\/13/);assert.match(html,/id="dfcdiagram"/);assert.equal(r.e('source-panel').hidden,true);assert.equal(JSON.stringify(data),before);
});

function traceConditionFixture(language='en'){
 const data=compactUxFixture(language,'process'),op=data.operations[0],ru=language==='ru';
 const e=(id,kind,text,mode)=>({id,kind,text,sourceIds:['summary-source'],dependencyIds:[],...(kind==='end'?{}:{reader:{label:fragment(text),...(mode?{execution:{mode,meaning:fragment(text)}}:{})}}),...(['message','declared','return'].includes(kind)?{from:kind==='return'?'provider':'service',to:kind==='return'?'service':'provider'}:{})});
 op.events=[
  e('outer','alt',ru?'Есть блокирующие входы':'Blocking inputs present'),e('outer-else','else',ru?'Блокирующих входов нет':'No blocking inputs'),
  e('version','alt',ru?'Активная версия отсутствует':'Active version missing'),e('version-else','else',ru?'Активная версия задана':'Active version supplied'),
  e('required-guard','loop','required=true: parallel; awaitAll before response','parallel-await'),e('required-call','declared','Call required calculation','parallel-await'),e('required-end','end',''),
  e('formula-first','message','callFormula(saveResult=false)','synchronous'),e('formula-first-result','alt','First result != null'),e('formula-second','message','callFormula(saveResult=true by default)','synchronous'),
  e('formula-second-result','alt','Second result != null'),e('optional-guard','loop','required=false: background without await/join','background'),e('optional-call','declared','Call optional calculation','background'),e('nested-return','return','Declared local response'),
  e('optional-end','end',''),e('second-end','end',''),e('first-end','end',''),e('version-end','end',''),e('outer-end','end','')
 ];return data;
}
test('mobile trace keeps only the nearest guard primary and preserves full negative ancestry in closed details',()=>{
 for(const language of ['en','ru']){
  const data=traceConditionFixture(language),before=JSON.stringify(data),r=load(data,true),html=r.run('sequenceTrace(D.operations[0])');
  const card=id=>html.split(`data-trace-event="${id}"`)[1].split('<li class="sequence-trace-step')[0];
  const required=card('required-call'),optional=card('optional-call'),nestedReturn=card('nested-return');
  const ancestors=language==='ru'?['Условие не выполнено: Есть блокирующие входы · Блокирующих входов нет','Условие не выполнено: Активная версия отсутствует · Активная версия задана']:['Condition not met: Blocking inputs present · No blocking inputs','Condition not met: Active version missing · Active version supplied'];
  for(const [content,local,total] of [[required,'required=true: parallel; awaitAll before response',3],[optional,'required=false: background without await/join',5],[nestedReturn,'required=false: background without await/join',5]]){
   const primary=content.match(/<p class="sequence-trace-conditions">([\s\S]*?)<\/p>/)[1];assert.ok(primary.includes(local));assert.doesNotMatch(primary,/Blocking inputs|Блокирующих|Active version|Активная версия|First result|Second result/);
   const inherited=content.match(/<details class="sequence-trace-ancestry">[\s\S]*?<\/details>/)[0];assert.deepEqual([...inherited.matchAll(/<li>([\s\S]*?)<\/li>/g)].map(m=>m[1]),[...ancestors,...(total===5?['First result != null','Second result != null']:[]),local]);assert.ok(content.indexOf(inherited)+inherited.length<=content.indexOf('<details class="sequence-trace-evidence">'));assert.match(content,/<details class="sequence-trace-ancestry"><summary>/);assert.doesNotMatch(content,/<details class="sequence-trace-ancestry" open/);assert.ok(content.includes((language==='ru'?'Все условия':'All conditions')+` (${total})`));assert.ok(content.includes(language==='ru'?'Условие не выполнено: Есть блокирующие входы':'Condition not met: Blocking inputs present'));assert.ok(content.includes(language==='ru'?'Активная версия задана':'Active version supplied'));
  }
  const second=card('formula-second');assert.match(second,/<p class="sequence-trace-conditions"><b>[^<]+<\/b> First result != null<\/p>/);assert.ok(second.includes('callFormula(saveResult=true by default)'));assert.ok(html.indexOf('data-trace-event="formula-first"')<html.indexOf('data-trace-event="formula-second"'));
  assert.deepEqual([...html.matchAll(/data-trace-event="([^"]+)"/g)].map(m=>m[1]),data.operations[0].events.filter(e=>e.kind!=='end').map(e=>e.id));assert.equal(JSON.stringify(data),before);assert.equal(r.e('source-panel').hidden,true);
 }
});
if(process.env.CODECLEW_TRACE_CONDITIONS_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_TRACE_CONDITIONS_FIXTURE_OUT);fs.mkdirSync(output,{recursive:true});
 for(const language of ['en','ru'])fs.writeFileSync(path.join(output,'trace-'+language+'.html'),renderReaderFixture(traceConditionFixture(language)));
}

function semanticDomainFixture(language='en'){
 const data=taskReaderFixture(language),model=data.readerModels[0],base=model.fields[0],claim=text=>({...base.meaning,text});
 data.entities[0].normalized.entity.title=language==='ru'?'Транспортное средство':'Vehicle';
 data.entities[0].normalized.entity.description='An independently constructed business concept; DTOs are representations.';
 data.entities[0].normalized.entity.relations[0].confidence='uncertain';
 data.entities[0].normalized.entity.relations[0].rationale='Proposed customer association; the observed read does not establish global customer identity.';
 model.title=language==='ru'?'Транспортное средство':'Vehicle';
 const year={...base,id:'manufacture-year',path:'request.year',readableName:claim(language==='ru'?'Год изготовления':'Manufacture year'),meaning:claim('Calendar year in which the vehicle was manufactured.'),representations:[{...base.representations[0],label:'AssetInput.manufactureYear',mapping:claim('Definition identifies manufacture year; the quote mapper copies the value unchanged.')},{...base.representations[0],label:'VehicleWire.year',mapping:claim('Same manufacture-year attribute in the quote mapping; no shared object identity or values across unrelated requests are asserted.')}],usage:[{...base.usage[0],stage:'Quote',representation:'AssetInput.manufactureYear',claim:claim('Read the manufacture year for quote calculation.')},{...base.usage[0],stage:'Renewal',representation:'VehicleWire.year',claim:claim('Read the same attribute in the separately scoped renewal request.')}]};
 const age={...base,id:'age',path:'calculation.age',readableName:claim(language==='ru'?'Возраст':'Age'),meaning:claim('Derived age at valuation; distinct from manufacture year.'),representations:[{...base.representations[0],label:'RiskFactors.age',mapping:claim('Valuation year minus manufacture year; this is a derived output, not an alias.')}],usage:[]};
 const reporting={...base,id:'reporting-year',path:'request.year',readableName:claim(language==='ru'?'Отчётный год':'Reporting year'),meaning:claim('Reporting period of a different operation, not manufacture year.'),representations:[{...base.representations[0],label:'ReportInput.year',mapping:claim('Independent reporting operation; identical path spelling does not establish equivalence.')}],usage:[]};
 const make={...base,id:'make',path:'request.make',readableName:claim(language==='ru'?'Марка':'Make'),meaning:claim('Vehicle manufacturer identity, distinct from model.'),representations:[{...base.representations[0],label:'AssetInput.manufacturer',mapping:claim('Definition and quote mapper identify the same manufacturer attribute.')},{...base.representations[0],label:'VehicleWire.make',mapping:claim('Alternate representation of manufacturer in the renewal mapping.')}],usage:[{...base.usage[0],representation:'AssetInput.manufacturer',claim:claim('The quote mapper copies manufacturer unchanged.')},{...base.usage[0],representation:'VehicleWire.make',claim:claim('The renewal uses the separate request value; object identity is not inferred.')}]};
 data.entities[0].normalized.entity.relations.push({...data.entities[0].normalized.entity.relations[0],representations:['LegacyInput.year'],rationale:'Whether LegacyInput.year is manufacture year remains unresolved: the field definition and mapper are missing. Path spelling is insufficient; no accepted field alias is asserted.'});
 model.fields=[year,make,age,reporting];return data;
}
test('semantic domain lists canonical attributes first and keeps scoped DTO mappings and repeated uses secondary',()=>{
 for(const language of ['en','ru']){
  const data=semanticDomainFixture(language),before=JSON.stringify(data),r=load(data),html=r.run('domainCatalogue()'),summary=html.split('<details class="domain-object"><summary>')[1].split('</summary>')[0];
  assert.ok(summary.includes(language==='ru'?'Транспортное средство':'Vehicle'));assert.doesNotMatch(summary,/domain-code-term|<code>|Hypothesis|Гипотеза/);
  const table=html.match(/<div class="table-wrap field-glossary">[\s\S]*?<\/table>/)[0];assert.equal((table.match(/data-domain-field=/g)||[]).length,4);assert.equal((table.match(/data-domain-field="manufacture-year"/g)||[]).length,1);assert.doesNotMatch(table,/request\.year|VehicleWire|AssetInput/);assert.ok(table.includes(language==='ru'?'Год изготовления':'Manufacture year'));assert.ok(table.includes(language==='ru'?'Возраст':'Age'));assert.ok(table.includes(language==='ru'?'Отчётный год':'Reporting year'));
  assert.equal((table.match(/data-domain-field="make"/g)||[]).length,1);assert.ok(html.includes('AssetInput.manufacturer'));assert.ok(html.includes('VehicleWire.make'));assert.doesNotMatch(table,/LegacyInput/);assert.ok(html.includes('Whether LegacyInput.year is manufacture year remains unresolved'));assert.ok(html.includes('VehicleWire.year'));assert.ok(html.includes('AssetInput.manufactureYear'));assert.ok(html.includes('no shared object identity'));assert.ok(html.includes('Quote'));assert.ok(html.includes('Renewal'));assert.ok(html.indexOf('field-representations')<html.indexOf('Same manufacture-year attribute'));
  assert.match(html,/<details class="domain-interpretations">/);assert.ok(html.indexOf('domain-interpretations')<html.indexOf('Proposed customer association'));assert.equal(JSON.stringify(data),before);
 }
});
test('timing is a plain nonfocusable annotation while step disclosure remains independently actionable',()=>{
 const data=compactUxFixture('en','process'),r=load(data),html=r.run('sequenceTrace(D.operations[0])');
 const timing=html.match(/<span class="execution-mode"[^>]*>[^<]*<\/span>/g);assert.ok(timing.length>=3);assert.ok(timing.every(tag=>!/(tabindex|role=|data-|<button)/.test(tag)));assert.match(html,/<details class="sequence-trace-evidence"><summary>/);
});
if(process.env.CODECLEW_SEMANTIC_READER_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_SEMANTIC_READER_FIXTURE_OUT);fs.mkdirSync(output,{recursive:true});for(const language of ['en','ru'])fs.writeFileSync(path.join(output,'domain-'+language+'.html'),renderReaderFixture(semanticDomainFixture(language)));
}

function semanticPayloadFixture(language='en'){
 const data=compactUxFixture(language,'process'),op=data.operations[0],claim=fragment('Selected source establishes a request field and an observed return. Complete wire schema and the generated message definition are not retained.');
 const view=(id,component,direction,fields,status)=>({id,title:id==='request'?'Price request':id==='response'?'Price response':id==='error'?'Remote failure response':'Produced Avro value',operation:op.id,subject:data.subject,component,direction,format:id==='message'?'avro':'json',status,documentationLanguage:language,coverage:{status:fields.length?'observed-fields':'unknown',scope:fields.length?'Named source representation; wire presence remains unknown.':'Complete generated definition unavailable in the selected source.',claim},fields,authority:'SOURCE_DERIVED_REPRESENTATION',wireSchema:'NOT_ESTABLISHED_BY_TYPE_OR_MAPPING',runtime:'UNVERIFIED'});
 const year={model:'vehicle',field:'manufacture-year',path:'manufactureYear',representation:'AssetInput.manufactureYear',typeLabel:'integer',requiredness:'Constructor parameter; wire presence unknown',nullability:'Nullable source property; wire nullability unknown',meaning:fragment('Calendar year of manufacture.'),mapping:fragment('Named request property; no serialization rule is inferred.')};
 data.payloadViews=[view('request','request-body','incoming',[year]),view('response','response-body','outgoing',[{...year,model:'quote',field:'premium',path:'premium',representation:'PriceResponse.premium',nullability:'Non-null type; wire property presence separate',meaning:fragment('Quoted price.')}],'200'),view('error','error-response-body','outgoing',[],'502'),view('message','message-value','outgoing',[])];return data;
}
function compactPayloadFixture(language='en'){
 const data=semanticPayloadFixture(language),view=data.payloadViews[0],ru=language==='ru';
 view.title=ru?'Локальные измерения':'Local measurements';view.direction='local';view.format='Source representation';
 view.coverage.scope=ru?'Четыре выбранных значения из преобразования; не схема обмена.':'Four selected mapper values; not a wire schema.';
 view.fields=Array.from({length:4},(_,i)=>({model:'measurement',field:'value-'+i,path:'request.values['+i+']',representation:'Input.value'+i,
  readableName:fragment(ru?'Измерение '+(i+1):'Measurement '+(i+1)),
  meaning:fragment(ru?'Значение для выбранного измерения '+(i+1)+'.':'Value for the selected measurement '+(i+1)+'.'),
  mapping:fragment(ru?'Передаётся соответствующее значение входного объекта.':'Forwards the corresponding input value.'),
  typeLabel:i===3?'Decimal':(ru?'Определение импортированного типа отсутствует в выбранном исходном коде; конкретный тип значения не установлен.':'The imported definition is absent from the selected source; the concrete value type has not been established.'),
  requiredness:i===3?(ru?'Нужно при включённом расчёте.':'Required when calculation is enabled.'):(ru?'Наличие свойства при сериализации не установлено; обращение к значению не доказывает обязательность свойства.':'Wire property presence has not been established; accessing a value does not establish required property presence.'),
  nullability:i===3?(ru?'Null в реализации запрещён; правила обмена не установлены.':'Non-null implementation type; wire rules are not established.'):(ru?'Nullability неизвестного типа не установлена; правила сериализации не следуют из вызова преобразования.':'The missing type nullability is not established; mapper invocation does not establish serialization rules.'),
  relatedModel:i===0?'nested-measurement':null}));
 data.payloadViews=[view];return data;
}
test('source payload views separate incoming request response error and unknown Avro shape without inventing wire rules',()=>{
 const data=semanticPayloadFixture(),original=JSON.stringify(data),r=load(data),op=data.operations[0],html=r.run(`contract({id:${JSON.stringify(op.id)}})`);
 for(const title of ['Price request','Price response','Remote failure response','Produced Avro value'])assert.ok(html.includes(title));assert.match(html,/request-body|Request body/);assert.match(html,/502/);assert.match(html,/manufactureYear/);assert.match(html,/Constructor parameter; wire presence unknown/);assert.match(html,/Nullable source property; wire nullability unknown/);assert.match(html,/Field definition unavailable/);assert.match(html,/No empty or complete payload schema is asserted/);assert.doesNotMatch(html,/No body declared|Interface contract is not documented/);assert.match(html,/data-label="Representation nullability"/);assert.equal((html.match(/class="cpvlegend"/g)||[]).length,1);assert.equal((html.match(/Source-derived representation shape/g)||[]).length,1);assert.ok(html.indexOf('class="cpvlegend"')<html.indexOf('Source-derived representation shape'));assert.equal(JSON.stringify(data),original);
 r.run(`location.hash=${JSON.stringify('#cpv-'+op.id+'-request')};routeReaderHash()`);assert.equal(r.run('tab'),'contract');assert.ok(r.e('scenario-content').innerHTML.includes('Price request'));assert.equal(r.e('source-panel').hidden,true);
});
test('source payload nested type links use only explicit projected identities',()=>{
 const data=semanticPayloadFixture();data.payloadViews[0].fields[0].relatedModel='nested-record';
 const before=JSON.stringify(data),r=load(data),html=r.run(`sourcePayloadViews({id:${JSON.stringify(data.operations[0].id)}})`);
 assert.match(html,/href="\.\.\/data-model\/nested-record.html#fields-and-representations"/);
 assert.match(html,/href="\.\.\/data-model\/vehicle.html#field-manufacture-year"/);
 assert.equal(JSON.stringify(data),before);
 delete data.payloadViews[0].fields[0].relatedModel;
 assert.doesNotMatch(load(data).run(`sourcePayloadViews({id:${JSON.stringify(data.operations[0].id)}})`),/nested-record/);
});
test('payload table leads with the sourced readable name and links its exact canonical field',()=>{
 const data=semanticPayloadFixture('ru'),field=data.payloadViews[0].fields[0];
 field.model='measurement';field.field='value';field.path='request.measuredValue';
 field.readableName=fragment('Измеренное значение <literal>');field.originalName=fragment('Input.measuredValue');
 const before=JSON.stringify(data),html=load(data).run(`sourcePayloadViews({id:${JSON.stringify(data.operations[0].id)}})`);
 const firstRow=html.split('<tbody>')[1].split('</tr>')[0];
 assert.match(firstRow,/href="\.\.\/data-model\/measurement.html#field-value"><strong>Измеренное значение &lt;literal&gt;<\/strong><\/a>/);
 assert.ok(firstRow.indexOf('Измеренное значение')<firstRow.indexOf('request.measuredValue'));
 assert.match(firstRow,/<code>request.measuredValue<\/code>/);assert.doesNotMatch(firstRow,/<literal>/);
 assert.equal(JSON.stringify(data),before);
 delete field.readableName;
 const fallback=load(data).run(`sourcePayloadViews({id:${JSON.stringify(data.operations[0].id)}})`);
 assert.match(fallback,/href="\.\.\/data-model\/measurement.html#field-value"><strong>request.measuredValue<\/strong><\/a>/);
 assert.doesNotMatch(fallback,/Измеренное значение/);
});
test('payload overview keeps meanings and mappings primary with all literal type constraints in one closed disclosure',()=>{
 const data=compactPayloadFixture('ru'),before=JSON.stringify(data),r=load(data),html=r.run(`sourcePayloadViews({id:${JSON.stringify(data.operations[0].id)}})`);
 const split=html.indexOf('<details class="cpv-representation-details">'),primary=html.slice(0,split),technical=html.slice(split);
 assert.ok(split>0);assert.equal((html.match(/class="cpv-representation-details"/g)||[]).length,1);
 assert.match(technical,/<summary>Типы, обязательность и null<\/summary>/);assert.doesNotMatch(technical,/cpv-representation-details" open/);
 assert.equal((primary.match(/<th scope="col">/g)||[]).length,2);assert.match(primary,/Значение и преобразование/);
 for(const field of data.payloadViews[0].fields){
  assert.ok(primary.includes(field.readableName.text));assert.ok(primary.includes(field.path));
  assert.ok(primary.includes(field.meaning.text));assert.ok(primary.includes(field.mapping.text));
  for(const key of ['typeLabel','requiredness','nullability']){assert.ok(!primary.includes(field[key]),key);assert.ok(technical.includes(field[key]),key);}
  assert.ok(primary.includes('../data-model/'+field.model+'.html#field-'+field.field));
 }
 assert.match(technical,/href="\.\.\/data-model\/nested-measurement.html#fields-and-representations"/);
 assert.equal(JSON.stringify(data),before);
});
if(process.env.CODECLEW_SEMANTIC_READER_FIXTURE_OUT){const output=path.resolve(process.env.CODECLEW_SEMANTIC_READER_FIXTURE_OUT);for(const language of ['en','ru']){fs.writeFileSync(path.join(output,'payload-'+language+'.html'),renderReaderFixture(semanticPayloadFixture(language)));fs.writeFileSync(path.join(output,'payload-compact-'+language+'.html'),renderReaderFixture(compactPayloadFixture(language)));}}

test('profile payload deep link opens an actual zero-event service egress catalogue without inventing a process',()=>{
 const data=fixture(),op={...data.operations[0],id:'section-egress',title:'Outgoing messages',summary:fragment('Retained outgoing message catalogue'),events:[],visuals:[]};
 data.operations=[op];data.sections=[{id:'section-overview',title:'Overview',gap:'Overview not authored'},{id:op.id,title:op.title,content:op}];
 data.operationSources={[op.id]:{'same-source':source('fun publish(input: Mapped) { sender.send(GeneratedValue(input.value)) }')}};data.operationStates={[op.id]:{freshness:'CURRENT',verification:'UNASSESSED'}};
 data.externalServices={observer:{title:'Observation transport',category:'queue'}};
 data.declaredRelationships={nodes:[{id:'external-service:observer',label:'Observation transport'}],edges:[{id:'publish-observation',from:data.subject,to:'external-service:observer',title:'Publish observation',transport:{kind:'kafka',topic:'observation.created'}}]};
 data.payloadViews=[{id:'observation-value',operation:op.id,subject:data.subject,interaction:'publish-observation',title:'Published observation value',component:'message-value',direction:'outgoing',format:'avro',coverage:{status:'observed-fields',scope:'Mapped.value only; imported Avro definition unavailable',claim:fragment('Named mapper access only')},fields:[{model:'record',field:'value',path:'input.value (mapper access)',representation:'Mapped.value',typeLabel:'String',requiredness:'Wire presence unknown',nullability:'Wire nullability unknown',meaning:fragment('Value supplied to the mapper'),mapping:fragment('Named mapper access')}],authority:'SOURCE_DERIVED_REPRESENTATION',runtime:'UNVERIFIED'}];
 const original=JSON.stringify(data),r=load(data),anchor='cpv-section-egress-observation-value';
 r.run(`location.hash=${JSON.stringify('#'+anchor)};routeReaderHash()`);
 const html=r.e('scenario-content').innerHTML;
 assert.equal(r.run('current.id'),'section-egress');assert.equal(r.run('location.hash'),'#'+anchor);assert.ok(html.includes('id="'+anchor+'"'));
 assert.ok(html.indexOf('id="'+anchor+'"')<html.indexOf('class="original-service-description"'));
 assert.match(html,/Mapped.value only; imported Avro definition unavailable/);assert.match(html,/input.value \(mapper access\)/);
 assert.doesNotMatch(html,/class="process-thread"|class="sequence-trace"/);assert.equal(r.e('source-panel').hidden,true);assert.equal(JSON.stringify(data),original);
});


test('domain fallback keeps authored meaning primary without inventing objects or ownership',()=>{
 const data=taskReaderFixture();data.entities=[];data.readerModels=[];
 const op=data.operations.find(o=>o.id==='section-entities');op.summary=fragment('Measured samples describe a calibration session; record ownership is unknown.');
 const original=JSON.stringify(data),r=load(data);r.run("showEntry('section-entities')");const html=r.e('scenario-content').innerHTML;
 assert.ok(html.indexOf('Measured samples describe')<html.indexOf('original-service-description'));
 assert.match(html,/No structured business-object catalogue has been authored/);assert.doesNotMatch(html,/No domain ownership declarations|class="domain-object"/);assert.equal(JSON.stringify(data),original);
});
test('artifact branch labels occupy separate lanes and retain full guard meaning',()=>{
 const data=taskReaderFixture(),r=load(data),claim=text=>({text,sourceIds:[]});
 const visual={id:'branch',kind:'flow',title:'Calibration decisions',nodes:['start','reject','continue'].map(id=>({id,meaning:claim(id)})),edges:[{id:'rejection',from:'start',to:'reject',meaning:claim('If calibration is rejected, return the rejection result before preparing any message')},{id:'continuation',from:'start',to:'continue',meaning:claim('If calibration is accepted, prepare a message and continue with the supported submission path')}]};
 const html=r.run(`visualGraph(D.operations[0],${JSON.stringify(visual)})`),groups=[...html.matchAll(/<g data-artifact-edge="[^"]+">([\s\S]*?)<\/g>/g)];assert.equal(groups.length,2);
 const ranges=groups.map(g=>[...g[1].matchAll(/<text [^>]*y="([0-9.]+)"[^>]*>([^<]*)<\/text>/g)].map(m=>({y:Number(m[1]),text:m[2]})));
 assert.ok(Math.max(...ranges[0].map(x=>x.y))+15<Math.min(...ranges[1].map(x=>x.y)));
 for(let i=0;i<2;i++)assert.equal(ranges[i].map(x=>x.text).join(' '),`${i+1}. ${visual.edges[i].meaning.text}`);assert.doesNotMatch(groups.map(g=>g[1]).join(''),/…/);
});

test('reader lead follows section to nested visual heading and back without stale H1',()=>{
 const reader=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/reader.js'),'utf8');let changed;
 function node(tag,cls='',text=''){
  const n={tagName:tag.toUpperCase(),className:cls,children:[],parent:null,hidden:false,attributes:[],innerHTML:text,_text:text};
  n.classList={contains:value=>n.className.split(' ').includes(value)};
  n.matches=selector=>selector.split(',').some(part=>part.trim().startsWith('.')?n.classList.contains(part.trim().slice(1)):n.tagName===part.trim().toUpperCase());
  n.querySelector=selector=>{if(selector===':scope > .question-reader')return n.children.find(c=>c.matches('.question-reader'))||null;for(const child of n.children){if(child.matches(selector))return child;const nested=child.querySelector(selector);if(nested)return nested;}return null;};
  n.replaceChildren=(...children)=>{for(const child of n.children)child.parent=null;n.children=[];for(const child of children){if(child.parent)child.parent.children=child.parent.children.filter(c=>c!==child);child.parent=n;n.children.push(child);}};
  n.replaceWith=other=>{const parent=n.parent,index=parent.children.indexOf(n);parent.children[index]=other;other.parent=parent;n.parent=null;};
  n.setAttribute=()=>{};Object.defineProperty(n,'textContent',{get:()=>n._text||n.innerHTML||n.children.map(c=>c.textContent).join(''),set:value=>{n._text=value;n.innerHTML=value;}});return n;
 }
 const live=node('div'),lead=node('div'),location={hash:'#section-entities',search:'',pathname:'/services/calibration.html'};
 const showSection=()=>live.replaceChildren(node('div','breadcrumb','Calibration service'),node('h2','','Business objects'),node('section','','Retained meaning'));
 showSection();const document={documentElement:{lang:'en'},querySelector:()=>null,querySelectorAll:()=>[],getElementById:id=>({'scenario-content':live,'reader-lead':lead}[id]||null),createElement:tag=>node(tag)};
 vm.runInNewContext(reader,{document,URL,URLSearchParams,location,MutationObserver:class{constructor(fn){changed=fn;}observe(){}}});
 assert.equal(lead.querySelector('h1').textContent,'Business objects');
 const visual=node('article','visual-artifact'),visualTitle=node('h3','','Calibration decision flow');visual.replaceChildren(visualTitle,node('div','','Full retained graph'));
 live.replaceChildren(node('div','breadcrumb','Calibration service · Processes'),visual);location.hash='#visual-calibration';changed();
 assert.equal(lead.querySelector('h1').textContent,'Calibration decision flow');assert.equal(visual.querySelector('h3').textContent,'Calibration decision flow');assert.ok(live.children.includes(visual));
 showSection();location.hash='#section-entities';changed();assert.equal(lead.querySelector('h1').textContent,'Business objects');
 live.replaceChildren(node('div','question-reader'));changed();assert.equal(lead.children.length,0);
});

function reviewFollowupFixture(language='en'){
 const d=fixture();d.language=language;d.title=language==='ru'?'Синтетический reader для ревью':'Synthetic reader review';d.subtitle='Synthetic data only; development renderer preview, not an official package.';
 const op={...d.operations[0],id:'submit-record',title:'Prepare and publish a record',visuals:[],participants:[{id:'app',label:'Reading service',service:'sample'},{id:'archive',label:'Archive provider',externalService:'archive'},{id:'unregistered',label:'Unregistered destination'}],events:[
  {id:'save',kind:'declared-boundary',interaction:'save-record',from:'app',to:'archive',text:'ArchiveGatewayWithAnIntentionallyLongSourceName.submitEnvelope(RecordEnvelopeWithAnIntentionallyLongTypeName)',sourceIds:[],dependencyIds:[]},
  {id:'unresolved',kind:'declared-boundary',interaction:'missing-profile',from:'app',to:'unregistered',text:'The destination is declared; no source or profile is retained.',sourceIds:[],dependencyIds:[]}],explanation:[]};
 d.operations=[op];d.catalogue=[{id:op.id,symbol:'RecordMapper.submit',kind:'SOURCE_DECLARATION',trigger:{},sourceIds:['same-source']}];d.sections=[];
 d.operationSources={[op.id]:d.sources};d.operationStates={[op.id]:{freshness:'CURRENT',verification:'UNASSESSED'}};
 d.externalServices={archive:{title:'Archive provider',category:'storage'}};
 d.payloadViews=[{id:'event',subject:d.subject,operation:op.id,interaction:'record-published',title:language==='ru'?'Наблюдаемые данные публикации':'Observed publication data',component:'message-value',direction:'outgoing',format:'Observed imported constructor; wire schema unavailable',coverage:{status:'unknown',scope:'The generated schema is not retained; only the source constructor call is described.',claim:fragment('Observed constructor call only; no field schema or message key is inferred.')},fields:[],constructorArguments:Array.from({length:12},(_,i)=>({position:i+1,expression:i===3?'input.flags.enabled ? 1 : 0':'input.part'+(i+1),claim:fragment(i===3?'A local Boolean read produces a 0/1 constructor argument.':'The source expression is passed at this constructor position.')}))}];
 d.readerModels=[{id:'reading',title:'Reading',fields:[{id:'factor',path:'input.flags.enabled',readableName:fragment('Calculation flag'),meaning:fragment('The local flag contributes a numeric factor.'),representations:[{label:'Input.enabled',typeLabel:'Boolean',requiredness:'UNPROVEN_PRESENCE',nullability:'UNPROVEN_NULLABILITY',mapping:fragment('The selected mapper reads the flag.')}],usage:[{stage:'RecordMapper',consumer:'RecordMapper',producer:'Input',representation:'Input.enabled',condition:'BOILERPLATE_CONDITION',unresolved:'BOILERPLATE_UNKNOWN',contract:'',inputs:[],kinds:['read','calculate-or-decide'],assessment:'observed-static-use',coveredScope:'Selected source method',claim:fragment('Reads the flag and calculates a numeric factor locally.'),presentation:{uncertainty:fragment('The active runtime flag value is not established.')}}]}]}];
 const selectedSource='final class RecordMapper {\n  ImportedEvent map(Input input) {\n    return new ImportedEvent('+d.payloadViews[0].constructorArguments.map(a=>a.expression).join(', ')+');\n  }\n}';
 d.sources={'same-source':{...source(selectedSource),file:'src/RecordMapper.java',endLine:5,textDigest:'sha256:'+require('node:crypto').createHash('sha256').update(selectedSource).digest('hex')}};d.operationSources={[op.id]:d.sources};
 d.interactions=[{id:'record-published',origin:'human',runtime:'UNKNOWN',declaration:{external:true,externalService:'archive',from:{service:'sample'},to:{service:'archive'},transport:{kind:'kafka',topic:'synthetic.record.calculated.with.a.deliberately.long.topic.name.v1'}}}];
 d.payloadViews[0].constructorArguments.forEach(a=>a.claim.evidence=[{kind:'SELECTED_SOURCE',source:{id:'same-source',...d.sources['same-source']},startLine:1,endLine:5}]);
 d.operationExternalCalls={[op.id]:[{eventId:'save',interactionId:'save-record',to:'archive',title:'Archive declared record',transport:{kind:'logical'},addresses:[],contract:null,contractStatus:'REFERENCE_UNAVAILABLE',declaration:{origin:'human',rationale:'Synthetic declared destination.'},snapshotContextDigest:'synthetic'}]};
 return d;
}

test('review usage omits empty inputs and boilerplate presentation but retains original machine values',()=>{
 const d=reviewFollowupFixture(),before=JSON.stringify(d),r=load(d),html=r.run('modelFieldRecords(D.readerModels[0],D.subject,true)');
 const primary=html.slice(0,html.indexOf('<details class="field-use-evidence"'));
 assert.doesNotMatch(primary,/Input<|Not established|BOILERPLATE|→|RecordMapper · RecordMapper/);
 assert.match(primary,/Calculates in/);assert.match(primary,/Reads the flag and calculates/);assert.match(primary,/active runtime flag/);
 assert.match(html,/BOILERPLATE_CONDITION/);assert.match(html,/observed-static-use/);assert.equal(JSON.stringify(d),before);
 d.readerModels[0].fields[0].usage[0].presentation.condition=fragment('Only when the selected source branch is entered.');
 assert.match(load(d).run('modelFieldRecords(D.readerModels[0],D.subject,true)'),/Only when the selected source branch is entered/);
 d.readerModels[0].fields[0].usage[0].kinds=['forward'];assert.match(load(d).run('modelFieldRecords(D.readerModels[0],D.subject,true)'),/Forwards to/);
});

test('review requiredness is scoped by an explicit bound representation claim rather than usage',()=>{
 const d=semanticPayloadFixture(),op=d.operations[0];d.payloadViews[0].fields[0].requiredness='LEGACY_REQUIRED_UNPROVEN';
 let html=load(d).run(`sourcePayloadViews({id:${JSON.stringify(op.id)}})`),primary=html.slice(0,html.indexOf('<details class="contract-raw"'));
 assert.doesNotMatch(primary,/LEGACY_REQUIRED_UNPROVEN/);assert.match(html,/LEGACY_REQUIRED_UNPROVEN/);
 d.payloadViews[0].fields[0].requirednessClaim=fragment('Required only in the selected DTO declaration.');
 html=load(d).run(`sourcePayloadViews({id:${JSON.stringify(op.id)}})`);assert.match(html,/Required only in the selected DTO declaration/);
});

test('review unknown Kafka schema shows12 source argument positions and links the topic to the exact payload',()=>{
 const d=reviewFollowupFixture(),before=JSON.stringify(d),r=load(d),html=r.run('sourcePayloadViews(D.operations[0])');
 assert.match(html,/Observed constructor arguments \(12\)<\/h4>/);assert.match(html,/\(12\)/);assert.match(html,/input.flags.enabled \? 1 : 0/);
 assert.equal((html.match(/data-label="Position"/g)||[]).length,12);assert.doesNotMatch(html,/data-label="Representation presence"/);
 assert.match(html,/not schema field names, message keys or requiredness/);
 d.sources['unrelated-topic-mention']=source(d.interactions[0].declaration.transport.topic);
 const topic=load(d).run('topicBoundaryCard(D.interactions,{direction:"outgoing"})');
 assert.match(topic,/data-sources="same-source"/);assert.doesNotMatch(topic,/unrelated-topic-mention/);
 delete d.sources['unrelated-topic-mention'];
 assert.match(topic,/href="#cpv-submit-record-event"/);assert.match(topic,/Receiver source not captured/);assert.equal(JSON.stringify(d),before);
 for(const arg of d.payloadViews[0].constructorArguments)arg.claim.evidence[0].source.revision='historical-revision';
 assert.doesNotMatch(load(d).run('topicBoundaryCard(D.interactions,{direction:"outgoing"})'),/data-sources="same-source"/);
 for(const arg of d.payloadViews[0].constructorArguments)arg.claim.evidence[0].source.revision=d.sources['same-source'].revision;
 r.run('location.hash="#cpv-submit-record-event";routeReaderHash()');assert.equal(r.run('tab'),'contract');assert.ok(r.e('scenario-content').innerHTML.includes('input.part12'));
});

test('review connection cards link a registered profile or disclose a declaration even with no snippets',()=>{
 const d=reviewFollowupFixture(),before=JSON.stringify(d),html=load(d).run('interactionOverview(D.operations[0])');
 assert.match(html,/<a class="interaction-link" href="\.\.\/external-services\/archive.html"/);
 assert.match(html,/<details class="interaction-disclosure"><summary class="interaction-link">/);
 assert.match(html,/View connection/);assert.doesNotMatch(html,/data-event=/);assert.equal(JSON.stringify(d),before);
 const row=d.operationExternalCalls[d.operations[0].id][0];row.contractStatus='FOREIGN_RECEIVER_CONTRACT';
 const details=load(d).run('externalCallDetails(D.operations[0],D.operations[0].events[0])');
 assert.doesNotMatch(details,/Set contractReference|CONTRACT_OPERATION/);assert.match(details,/FOREIGN_RECEIVER_CONTRACT/);assert.match(details,/Remote wire contract is not established/);
});

test('outgoing contract fallback links only the exact interaction payload within a shared operation',()=>{
 const d=reviewFollowupFixture(),op=d.operations[0],row=d.operationExternalCalls[op.id][0];
 d.payloadViews=[
  {...d.payloadViews[0],id:'matching',title:'Exact archived payload',interaction:row.interactionId},
  {...d.payloadViews[0],id:'foreign',title:'Other call payload',interaction:'other-interaction'},
  {...d.payloadViews[0],id:'local',title:'Local calculation payload',interaction:undefined}
 ];
 for(const stale of [false,true]){
  row.targetChanged=stale;const html=load(d).run('externalCallDetails(D.operations[0],D.operations[0].events[0])');
  assert.match(html,/href="#cpv-submit-record-matching"/);
  assert.doesNotMatch(html,/Other call payload|Local calculation payload|#cpv-submit-record-foreign|#cpv-submit-record-local/);
 } row.interactionId=undefined;assert.doesNotMatch(load(d).run('externalCallDetails(D.operations[0],D.operations[0].events[0])'),/href="#cpv-/);
});

if(process.env.CODECLEW_REVIEW_FOLLOWUP_FIXTURE_OUT){
 const out=path.resolve(process.env.CODECLEW_REVIEW_FOLLOWUP_FIXTURE_OUT);for(const dir of ['services','external-services','data-model'])fs.mkdirSync(path.join(out,dir),{recursive:true});
 for(const language of ['en','ru']){
  const d=reviewFollowupFixture(language),reader=load(d),extra=reader.run('modelFieldRecords(D.readerModels[0],D.subject,true)')+reader.run('topicBoundaryCard(D.interactions,{direction:"outgoing"})');
  // Same shipped renderer and CSS as a full native page; only the inputs are synthetic.
  let html=renderReaderFixture(d);html=html.replace('<script type="application/json" id="document-data"',`<section id="review-field-and-topic" style="margin:24px;max-width:1000px">${extra}</section><script type="application/json" id="document-data"`);
  fs.writeFileSync(path.join(out,'services',language==='en'?'sample.html':'sample-ru.html'),html);
 }
 fs.writeFileSync(path.join(out,'index.html'),'<!doctype html><meta charset="utf-8"><title>Synthetic reader review</title><h1>Synthetic renderer review</h1><p>Development renderer only; no official package or private content.</p><a href="services/sample.html#submit-record">English</a> · <a href="services/sample-ru.html#submit-record">Русский</a>');
 fs.writeFileSync(path.join(out,'external-services/archive.html'),'<!doctype html><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Archive provider</title><h1>Archive provider</h1><p>Synthetic declared external profile.</p><p><a href="../services/sample.html#cpv-submit-record-event">Observed publication data</a></p>');
}
