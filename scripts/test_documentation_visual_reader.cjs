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
 const style=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/style.css'),'utf8');
 const analysis=fs.readFileSync(path.join(__dirname,'../crates/clew/assets/documentation/analysis.js'),'utf8');
 const payload=JSON.stringify(data).replace(/</g,'\\u003c').replace(/\u2028/g,'\\u2028').replace(/\u2029/g,'\\u2029');
 const html=template.replace('/*__STYLE__*/',()=>style).replace('/*__SCRIPT__*/',()=>script).replace('/*__ANALYSIS_SCRIPT__*/',()=>analysis).replace('__DOCUMENT_DATA__',()=>payload);
 if(!html.includes(script)||!html.includes(style)||!html.includes(analysis)||html.includes('__DOCUMENT_DATA__'))throw new Error('renderer fixture did not embed the exact shipped reader assets');
 return html;
}
function renderBehaviorFixture(data){return renderReaderFixture(data);}
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

test('sequence layout wraps complete text inside supplied narrow and wide widths without shrinking typography',()=>{
 const data=fixture(),r=load(data),op={...data.operations[0],participants:[{id:'caller',label:'Unused caller'},{id:'dashboard',label:'Dashboard service with a long name'},{id:'kafka',label:'Kafka topic weather.forecast-updated'},{id:'db',label:'Dashboard PostgreSQL'}],events:[]};
 const event=(id,kind,text,from,to)=>({id,kind,text,from,to,sourceIds:['same-source'],dependencyIds:['dep']});
 op.events=[event('guard','alt','The inbox insert reports an existing event id.'),event('empty','note','Return DUPLICATE'),event('else','else','Otherwise'),event('apply','message','For a new forecast event, insert the card from event.place when no card exists; update only when the incoming timestamp is strictly newer.','dashboard','db'),event('self','message','Self-message labels remain complete at the rightmost participant, including a-long-unbroken-identifier-with-unicode-π🙂.','db','db'),event('reply','return','Return the saved result after commit.','db','kafka'),event('declared','declared','Declared delivery remains unverified.','kafka','dashboard'),event('end','end','')];
 const original=JSON.stringify(op);
 for(const width of [224,280,320,375,560,610,1280]){
  const layout=JSON.parse(r.run(`JSON.stringify(sequenceLayout(${JSON.stringify(op)},${width}))`));assert.equal(layout.W,width);assert.deepEqual(layout.actors.map(a=>a.id),['dashboard','kafka','db']);
  for(const actor of layout.actors){assert.ok(layout.xs[actor.id]-actor.width/2>=0);assert.ok(layout.xs[actor.id]+actor.width/2<=width);assert.equal(actor.titleLines.join('').replace(/ /g,''),actor.label.replace(/ /g,''));}
  for(const row of layout.rows.filter(row=>['message','return','declared'].includes(row.e.kind))){assert.equal(row.lines.join('').replace(/ /g,''),row.e.text.replace(/ /g,''));assert.ok(row.mid-row.labelWidth/2>=0);assert.ok(row.mid+row.labelWidth/2<=width);assert.ok(row.arrowOffset>(row.lines.length-1)*14+10);if(row.e.kind==='declared'){assert.equal(row.captionLines.join('').replace(/ /g,''),'Declared service interaction'.replace(/ /g,''));assert.ok(row.h>row.captionOffset+(row.captionLines.length-1)*14);}if(row.e.from===row.e.to){const end=layout.xs[row.e.from]+row.direction*row.loopWidth;assert.ok(end>=0&&end<=width);}}
  const svg=r.run(`diagram(${JSON.stringify(op)},${width})`);assert.doesNotMatch(svg,/min-width|NaN|Infinity/);assert.ok(svg.includes(`viewBox="0 0 ${width} `));assert.equal((svg.match(/data-event=/g)||[]).length,5);
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
 assert.ok(kafka.titleLines.length>1);assert.ok(kafka.titleLines.every(line=>Array.from(line).length<=20));
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
 assert.match(html,/Address not provided/);assert.match(html,/referenced contract is unavailable/);assert.match(html,/exact saved CONTRACT_OPERATION ID/);assert.match(html,/Multiple declarations match/);
 assert.doesNotMatch(html,/Incoming caller operation/);
 row.contractStatus='AMBIGUOUS_RECEIVER_CONTRACT';
 html=load(data).e('scenario-content').innerHTML;assert.match(html,/Multiple saved receiver contracts match/);assert.doesNotMatch(html,/Incoming caller operation/);
 row.contractStatus='NOT_BOUND';row.targetChanged=true;row.retained=true;
 html=load(data).e('scenario-content').innerHTML;assert.match(html,/No outgoing contract is bound/);assert.match(html,/earlier accepted operation/);
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
test('entity section shows only its accepted local visuals while the process gallery still includes all owners',()=>{
 const data=entitySectionFixture();data.savedProcesses=[{id:'saved',title:'Saved workflow',href:'../scenarios/saved.html',status:'AUTHORED'}];
 const r=load(data);r.run("showEntry('section-entities')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Recorded domain ownership/);assert.match(html,/Synthetic Order/);
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
 assert.match(html,/^<details[^>]+><summary>Source and review details<\/summary>/);assert.doesNotMatch(html,/^<details[^>]*\bopen(?:\s|=|>)/);
 assert.match(html,/Source freshness: <b>UNVERIFIED<\/b>/);assert.match(html,/PINNED_SNAPSHOT_NOT_REVERIFIED/);assert.match(html,/Model approved with limitations/);
 assert.doesNotMatch(html,/freshness-unverified|retained with a gap/);assert.equal(JSON.stringify(r.data.sectionState),original);
 const ru=load({...data,language:'ru'}).e('freshness-status').innerHTML;assert.match(ru,/Сохранённая версия исходного кода/);assert.match(ru,/Не проверено/);
 for(const reasons of [[],[{reason:'SOURCE_MISSING'}],[{reason:'PINNED_SNAPSHOT_NOT_REVERIFIED'},{reason:'SOURCE_MISSING'}]]){
  const actual=load({...data,sectionState:{...data.sectionState,reasons}}).e('freshness-status').innerHTML;
  assert.match(actual,/freshness-unverified/);assert.doesNotMatch(actual,/freshness-snapshot/);assert.match(actual,/^<div /,"actual unknown source state remains prominent");
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
 assert.match(html,/Recorded domain ownership/);assert.match(html,/No domain ownership declarations have been recorded/);
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
 const html=reader.run(expression);assert.match(html,/href="\.\.\/external-services\/provider\.html"/);assert.doesNotMatch(html,/href="\.\.\/services\/provider\.html/);assert.match(html,/External &lt;provider&gt;/);assert.match(html,/Source not captured/);
 reader.run('D.externalServices={}');const absent=reader.run(expression);assert.doesNotMatch(absent,/href="\.\.\/services\/provider\.html/);assert.doesNotMatch(absent,/href="\.\.\/external-services/);assert.match(absent,/Source not captured/);
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
