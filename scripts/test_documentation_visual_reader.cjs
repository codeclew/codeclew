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
function load(data=fixture()){
 const elements=new Map(),listeners={};
 function element(id){if(!elements.has(id))elements.set(id,{id,value:'',hidden:false,innerHTML:'',textContent:'',isConnected:true,classList:{add(){},remove(){}},focus(){this.focused=true;},scrollIntoView(){this.scrolled=true;},insertAdjacentHTML(_,html){this.innerHTML=html+this.innerHTML;},addEventListener(){}});return elements.get(id);}
 element('document-data').textContent=JSON.stringify(data);
 const context=vm.createContext({document:{getElementById:element,addEventListener:(name,fn)=>listeners[name]=fn,querySelectorAll:()=>[],querySelector:()=>null,body:{classList:{add(){},remove(){}}},activeElement:null},location:{hash:''},history:{replaceState(){}},window:{addEventListener(){},scrollTo(){}},navigator:{clipboard:{writeText:async()=>{}}}});
 vm.runInContext(script,context);
 return {data,e:element,run:code=>vm.runInContext(code,context),click(dataset,currentSources=false){listeners.click({target:{closest:()=>({dataset,hasAttribute:name=>name==='data-current-sources'&&currentSources})},preventDefault(){}});}};
}
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
test('documented events render as localized, evidence-linked pseudocode while the sequence stays available',()=>{
 const data=behaviorFixture(),en=load(data),html=en.e('scenario-content').innerHTML;
 assert.equal((html.match(/class="diagram-card behavior-pseudocode"/g)||[]).length,1);
 assert.match(html,/If<\/code><span class="pseudocode-text">the quantity is positive:/);
 assert.match(html,/Loop<\/code><span class="pseudocode-text">For each supplied reservation:/);
 assert.match(html,/Reservation service → Reservation store: Apply &lt;script&gt;alert\(1\)&lt;\/script&gt;/);
 assert.doesNotMatch(html,/<script>alert\(1\)<\/script>/);
 assert.match(html,/Otherwise/);
 assert.match(html,/When<\/code><span class="pseudocode-text">an audit note is supplied:/);
 assert.match(html,/Return<\/code><span class="pseudocode-text">Reservation service → Caller: the accepted result/);
 assert.match(html,/style="--indent:40px;--indent-mobile:26px"/);
 assert.match(html,/style="--indent:200px;--indent-mobile:130px"><code class="pseudocode-keyword">Declared interaction<\/code><span class="pseudocode-text">\(Reservation service → Reservation store\) Provider submission remains a declaration/);
 assert.match(html,/<div class="behavior-pseudocode-line" role="listitem" style="--indent:0px;--indent-mobile:0px"><code class="pseudocode-keyword">Return<\/code><span class="pseudocode-text">Reservation service → Caller: the accepted result/);
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
 assert.match(html,/Explore internal processes/);
});
test('process catalog lists typed visuals without requiring an HTTP selection',()=>{
 const r=load();r.run("showEntry('process-catalog')");
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
 assert.doesNotMatch(html,/creates no entities|No outgoing calls|No Kafka|No cron/);
});
test('entity section shows only its accepted local visuals while the process gallery still includes all owners',()=>{
 const r=load(entitySectionFixture());r.run("showEntry('section-entities')");
 const html=r.e('scenario-content').innerHTML;
 assert.match(html,/Current entity declarations/);assert.match(html,/Synthetic Order/);
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
 const element=()=>({children:[],value:'',textContent:'',listeners:{},replaceChildren(){this.children=[];},append(...nodes){this.children.push(...nodes);},addEventListener(kind,fn){this.listeners[kind]=fn;}});
 const nodes=Object.fromEntries(['catalog-data','catalog-query','catalog-kind','catalog-results','catalog-status','catalog-prev','catalog-next','catalog-controls','catalog-pager','catalog-processes'].map(key=>[key,element()]));
 const rows=Array.from({length:45},(_,i)=>({kind:'Examined callable',id:`callable-${i}`,title:`ChildWorker.prepare ${i}`,href:`source-calls.html#body-${i}`,context:'linked · :/main',summary:'Examined source, not runtime impact',searchText:['prepare','parent-a','parent-b'],relatedLinks:[{title:'Examined by: parent-a',href:'parent-a-overview.html'},{title:'Examined by: parent-b',href:'parent-b-overview.html'}]}));
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
 assert.equal(nodes['catalog-controls'].hidden,false);assert.equal(nodes['catalog-pager'].hidden,false);assert.equal(nodes['catalog-processes'].hidden,true);
 nodes['catalog-next'].listeners.click();assert.equal(list.children.length,5);assert.equal(new URL(stack[index],'https://codex.test').searchParams.get('page'),'3');
 index--;setUrl(stack[index]);popstate();assert.equal(list.children.length,20);assert.equal(list.children[0].children[0].href,'source-calls.html#body-20');
 nodes['catalog-kind'].value='Diagnostic question';nodes['catalog-query'].value='';nodes['catalog-kind'].listeners.change();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,'child-diagnostic.html');assert.match(list.children[0].children[2].textContent,/not an approved answer/);
 index--;setUrl(stack[index]);popstate();assert.equal(nodes['catalog-query'].value,'prepare');assert.equal(nodes['catalog-kind'].value,'Examined callable');assert.equal(list.children[0].children[0].href,'source-calls.html#body-20');
 location.search='?q=prepare&kind=Examined+callable&page=999999999999';popstate();assert.equal(list.children.length,5);assert.equal(new URL(stack[index],'https://codex.test').searchParams.get('page'),'3');
 location.search='?q=absent';popstate();assert.equal(list.children.length,1);assert.equal(list.children[0].children[0].href,'child-diagnostic.html');
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
  rows.push({id:endpointId,title:endpointSymbol,kind:'Endpoint',href:`${processes[0]}-endpoint.html`,context,summary:'Explicitly selected compiler declaration; no inferred HTTP route or runtime activation.',searchText:[service,scope,endpointSymbol],relatedLinks:processes.map(id=>({href:`${id}-overview.html`,title:`Selected process: ${id}`}))});
  const symbol='method:class:ChildWorker#prepare(Task)Request',id=canonical('callable',service,scope,symbol);
  rows.push({id,title:symbol,kind:'Examined callable',href:`source-calls.html#ref-${hash(id)}`,context,summary:'Retained examined source body; reverse links are documentation context, not runtime impact. Call frontiers remain local gaps.',searchText:[service,scope,symbol,...processes.slice(0,2)],relatedLinks:processes.slice(0,2).map(parent=>({href:`${parent}-overview.html`,title:`Examined by: ${parent}`}))});
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
