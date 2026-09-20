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
