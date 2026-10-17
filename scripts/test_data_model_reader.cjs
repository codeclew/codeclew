'use strict';
const {test}=require('node:test');const assert=require('node:assert/strict');const fs=require('node:fs');const vm=require('node:vm');
const script=fs.readFileSync('crates/clew/assets/documentation/data-model.js','utf8');
function api(){const context={window:{},document:{getElementById:()=>null},URLSearchParams};vm.runInNewContext(script,context);return context.window.CodeclewDataModel;}
function sample(){return {contracts:[{id:'event',title:'Event v1',leaves:['a','b','c'].map(field=>({field,requiredWhenBranchMatches:true}))}],models:[{id:'entity',title:'Entity',fields:[{id:'a',path:'a',representations:[{contract:'event',field:'a'}],usage:[{stage:'Store',contract:'event',assessment:'observed-static-use',kinds:['persist','forward']}]},{id:'b',path:'b',representations:[{contract:'event',field:'b'}],usage:[{stage:'Store',contract:'event',assessment:'stale-evidence',kinds:['display']}]}]}]};}
test('required-leaf counts keep unknown and stale separate from observed uses',()=>{const s=api().stats(sample(),'event','Store');assert.equal(s.required,3);assert.equal(s.investigated,1);assert.equal(s.unknown,2);assert.equal(s.stale,1);assert.equal(s.categories.persist,1);assert.equal(s.categories.forward,1);assert.equal(s.categories.display,0);});
test('whole-object use is not promoted to direct display or business decisions',()=>{const d=sample();d.models[0].fields[0].usage[0].kinds=['whole-object-serialization-or-comparison'];const s=api().stats(d,'event','Store');assert.equal(s.categories['whole-object-serialization-or-comparison'],1);assert.equal(s.categories.display,0);assert.equal(s.categories['calculate-or-decide'],0);});
test('scoped negative findings remain investigated without becoming observed use',()=>{const d=sample();d.models[0].fields[0].usage[0]={stage:'UI',contract:'event',assessment:'no-use-found-in-bounded-complete-scope',kinds:[]};const s=api().stats(d,'event','UI');assert.equal(s.negative,1);assert.equal(s.investigated,1);assert.equal(s.unknown,2);assert.equal(s.categories.display,0);assert.equal(api().cellLabel([]),'Not investigated');assert.match(api().cellLabel([{assessment:'stale-evidence'}]),/unknown/);});
test('an unconfigured reader mounts no Data Flow controls',()=>{let writes=0;const context={window:{},document:{getElementById:()=>null,addEventListener:()=>writes++},URLSearchParams,D:{}};vm.runInNewContext(script,context);assert.equal(writes,0);});

test('many consumer stages transpose into rows without dropping field-stage cells',()=>{const a=api(),rows=Array.from({length:4},(_,i)=>({key:'place/'+i,label:'Place.field'+i,field:{usage:[]}})),stages=Array.from({length:12},(_,i)=>'Consumer stage '+i),html=a.matrixHtml(rows,stages,'event');assert.equal((html.match(/scope="col"/g)||[]).length,5);assert.equal((html.match(/scope="row"/g)||[]).length,12);assert.equal((html.match(/<td /g)||[]).length,48);for(const stage of stages)for(const row of rows)assert.ok(html.includes('data-field="'+row.key+'" data-stage="'+stage+'"'));});
test('a process with many fields keeps its short consumer scope in columns',()=>{const a=api(),rows=Array.from({length:11},(_,i)=>({key:'model/'+i,label:'Model.field'+i,field:{usage:[]}})),stages=['Receive','Validate','Store'],html=a.matrixHtml(rows,stages,'event');assert.equal((html.match(/scope="col"/g)||[]).length,4);assert.equal((html.match(/scope="row"/g)||[]).length,11);assert.equal((html.match(/<td /g)||[]).length,33);assert.ok(html.includes('data-stage-label="Store"'));});

test('field changes synchronize existing canonical links while preserving each process question',()=>{
 const location={href:'http://localhost/scenarios/example.html?stage=Store&contract=event&cellStage=Store#question-apply',search:'?stage=Store&contract=event&cellStage=Store',hash:'#question-apply'};
 const attrs={href:'../data-model/entity.html?fromProcess=example&fromAnchor=question-save','data-dm-return-anchor':'question-save'};
 const link={getAttribute:name=>attrs[name],setAttribute:(name,value)=>attrs[name]=value};
 const document={getElementById:()=>null,querySelectorAll:()=>[link]};
 const history={replaceState(_,__,next){const u=new URL(next,location.href);location.href=u.href;location.search=u.search;location.hash=u.hash;}};
 const context={window:{},document,location,history,URL,URLSearchParams};vm.runInNewContext(script,context);
 const nodes={};
 for(const selector of ['.dm-field-select','.dm-contract-select','.dm-stage-select','.dm-matrix','.dm-focused','.dm-counts','.dm-overview-summary','.dm-return-overview','.dm-drilldown','.dm-overview']){
  nodes[selector]={value:'',innerHTML:'',events:{},addEventListener(name,fn){this.events[name]=fn;},querySelectorAll:()=>[],querySelector:()=>null};
 }
 const container={innerHTML:'',querySelector:selector=>nodes[selector],querySelectorAll:()=>[]};
 const data=sample();for(const f of data.models[0].fields)f.usage=[];
 data.request={subject:'scenario:example',anchor:'question-apply',stages:['Store']};
 context.window.CodeclewDataModel.mount(container,data);
 let url=new URL(attrs.href,location.href);
 assert.equal(url.searchParams.get('fromField'),'entity/a');
 assert.equal(url.searchParams.get('fromStage'),'Store');assert.equal(url.searchParams.get('fromContract'),'event');assert.equal(url.searchParams.get('fromCellStage'),'Store');
 assert.equal(url.searchParams.get('fromAnchor'),'question-save');
 nodes['.dm-field-select'].value='entity/b';nodes['.dm-field-select'].events.change();
 url=new URL(attrs.href,location.href);
 assert.equal(url.searchParams.get('fromField'),'entity/b');
 assert.equal(url.searchParams.get('fromStage'),'Store');assert.equal(url.searchParams.get('fromContract'),'event');assert.equal(url.searchParams.get('fromCellStage'),null);
 assert.equal(url.searchParams.get('fromAnchor'),'question-save');
 assert.equal(new URL(location.href).searchParams.get('field'),'entity/b');
});

const scaling=JSON.parse(fs.readFileSync('fixtures/documentation-field-overview/twenty-fields.json','utf8'));
test('bounded inventory preserves twenty fields, omitted fields, unmapped leaves and every evidence state',()=>{
 const a=api(),before=JSON.stringify(scaling),rows=a.overviewRows(scaling),html=a.matrixHtml(rows,scaling.request.stages,'fixture');
 assert.equal(rows.length,21);assert.match(html,/aria-label="Fixture.field01 · Receive · Displayed"/);assert.equal((html.match(/<td /g)||[]).length,63);
 for(const label of ['Displayed','Whole-object use','No direct use found in this scope','Not investigated','Dynamic use unresolved','Stale evidence — use unknown','Outside investigated process scope · use unknown','Unmapped contract leaf · not investigated'])assert.ok(html.includes(label),label);
 assert.match(a.overviewHtml(scaling,rows,scaling.request.stages),/20 known model fields \/ container paths.*18 selected process fields/);
 assert.match(a.overviewHtml(scaling,rows,scaling.request.stages),/21 contract leaves/);
 assert.equal(JSON.stringify(scaling),before,'inventory must not mutate evidence');
 const selected=a.overviewRows(scaling,'fixture');assert.equal(selected.length,21);
 assert.doesNotMatch(html,/role="grid"/);assert.equal((html.match(/<button type="button"/g)||[]).length,84,'native keyboard buttons for field and cell details');
});

function mounted(data){
 const location={href:'http://localhost/process.html?view=data-flow',search:'?view=data-flow',hash:''},history={replaceState(_,__,next){const u=new URL(next,location.href);location.href=u.href;location.search=u.search;}};
 const nodes={};let focusCount=0;
 for(const selector of ['.dm-field-select','.dm-contract-select','.dm-stage-select','.dm-matrix','.dm-focused','.dm-counts','.dm-overview-summary','.dm-return-overview','.dm-drilldown','.dm-overview'])nodes[selector]={value:'',innerHTML:'',events:{},addEventListener(n,fn){this.events[n]=fn;},focus(){focusCount++;},scrollIntoView(){this.scrolled=true;},querySelector(){return {focus(){focusCount++;}};},querySelectorAll(){return [];}};
 let cachedHtml,cachedButtons;
 nodes['.dm-matrix'].querySelectorAll=()=>{if(cachedHtml===nodes['.dm-matrix'].innerHTML)return cachedButtons;cachedHtml=nodes['.dm-matrix'].innerHTML;return cachedButtons=Array.from(nodes['.dm-matrix'].innerHTML.matchAll(/<button type="button" data-dm-cell data-field="([^"]*)" data-stage="([^"]*)"/g),m=>({dataset:{field:m[1],stage:m[2]},events:{},addEventListener(n,fn){this.events[n]=fn;}}));};
 const container={innerHTML:'',querySelector:s=>nodes[s],querySelectorAll:()=>[]};
 const context={window:{},document:{getElementById:()=>null,querySelectorAll:()=>[]},location,history,URL,URLSearchParams};vm.runInNewContext(script,context);context.window.CodeclewDataModel.mount(container,data);
 return {nodes,container,location,focusCount:()=>focusCount};
}
test('default overview precedes optional trail and cell drilldown retains overview columns and filters',()=>{
 const {nodes,container,location,focusCount}=mounted(scaling);
 assert.ok(container.innerHTML.indexOf('dm-overview')<container.innerHTML.indexOf('dm-focused'));
 assert.match(container.innerHTML,/<details class="dm-drilldown">/);
 const original=nodes['.dm-matrix'].innerHTML;
 const cell=nodes['.dm-matrix'].querySelectorAll().find(b=>b.dataset.stage==='Store');cell.events.click();
 assert.equal(nodes['.dm-matrix'].innerHTML,original);assert.equal(nodes['.dm-stage-select'].value,'');
 assert.equal(new URL(location.href).searchParams.get('cellStage'),'Store');assert.equal(new URL(location.href).searchParams.get('stage'),null);
 assert.equal(nodes['.dm-drilldown'].open,true);assert.equal(nodes['.dm-drilldown'].scrolled,true,'detail navigation stays visible above focused heading');assert.equal(nodes['.dm-focused'].scrolled,undefined);assert.ok(focusCount()>0,'detail heading receives keyboard-visible focus');
 nodes['.dm-return-overview'].events.click();assert.ok(focusCount()>1,'return control focuses overview');
 nodes['.dm-stage-select'].value='Validate';nodes['.dm-stage-select'].events.change();
 assert.equal((nodes['.dm-matrix'].innerHTML.match(/<td /g)||[]).length,21);assert.equal(new URL(location.href).searchParams.get('stage'),'Validate');
 assert.equal(new URL(location.href).searchParams.get('cellStage'),null);
});

test('a sole contract does not silently hide related model fields on fresh overview entry',()=>{
 const data=JSON.parse(JSON.stringify(scaling));
 data.models.push({id:'related',title:'Related model',fields:[{id:'outside',path:'outside',selectedProcessField:false,meaning:{text:'Outside selected trace.'},representations:[],usage:[]}]});
 const {nodes}=mounted(data);
 assert.equal(nodes['.dm-contract-select'].value,'');
 assert.match(nodes['.dm-matrix'].innerHTML,/Related model.outside/);
 assert.match(nodes['.dm-overview-summary'].innerHTML,/22 displayed rows/);
 assert.match(nodes['.dm-overview-summary'].innerHTML,/related inventory: Related model: 1 fields/);
 nodes['.dm-contract-select'].value='fixture';nodes['.dm-contract-select'].events.change();
 assert.doesNotMatch(nodes['.dm-matrix'].innerHTML,/Related model.outside/);
 assert.match(nodes['.dm-overview-summary'].innerHTML,/21 displayed rows/);
});

test('overview keeps denominator explanations in a closed disclosure and concise visible counts',()=>{
 const a=api(),html=a.overviewHtml(scaling,a.overviewRows(scaling),scaling.request.stages),visible=html.split('<details class="dm-scope-details">')[0];
 assert.match(visible,/21 rows · 3 stages · 20 model paths · 18 traced fields/);
 assert.match(visible,/Unknown ≠ unused/);
 assert.doesNotMatch(visible,/Fixture contract|Required-leaf counts|Known model|Mapped leaves/);
 assert.match(html,/<details class="dm-scope-details"><summary>Scope and evidence key<\/summary>/);
 assert.match(html,/Required-leaf counts below are separate/);
});
