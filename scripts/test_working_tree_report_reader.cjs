'use strict';
// Execute the shipped report script with DOM selection/event behavior. Geometry
// belongs to browser qualification; this fixture contains synthetic evidence.
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');
const template=fs.readFileSync(process.env.CODECLEW_WORKING_TREE_REPORT_TEMPLATE||path.join(__dirname,'../crates/clew/src/working_tree_report.html'),'utf8');
const script=template.match(/<script>\s*([\s\S]*?)<\/script>/)[1];
function fixture(){
 const declaration=(name,compilation)=>({symbol:'synthetic/'+name+'#()V',compilation,source:{file:name.startsWith('PriceTest')?'src/test/PriceTest.kt':'src/main/Pricing.kt'}});
 const node=(id,name,compilation,role)=>({nodeId:id,role,entrypoints:[],before:declaration(name,compilation),after:declaration(name,compilation)});
 const nodes=[node('main-price','Pricing.price',':/main','CHANGED_DECLARATION'),node('test-price','Pricing.price',':/test','CHANGED_DECLARATION'),node('main-consumer','Pricing.labelConsumer',':/main','DIRECT_CONSUMER'),node('test-consumer','Pricing.labelConsumer',':/test','DIRECT_CONSUMER'),node('price-test','PriceTest.priceCheck',':/test','DIRECT_CONSUMER')];
 const edges=[{edgeId:'main-call',ownerNode:'main-consumer',targetNode:'main-price',kind:'CALL',presence:'AFTER'},{edgeId:'test-call',ownerNode:'test-consumer',targetNode:'test-price',kind:'CALL',presence:'AFTER'}];
 const preview=(id,side)=>({text:'Synthetic '+side+' source for '+id,anchor:{file:id+'.kt'},start:0,end:30,nextOffset:null});
 return {graph:{nodes,edges,candidates:[{nodeId:'main-consumer',changedNodeId:'main-price'},{nodeId:'test-consumer',changedNodeId:'test-price'}],testScope:'SELECTED_TEST_COMPILATION',omittedNodeCount:0,omittedEdgeCount:0,unresolvedRelationCount:0,obligations:[],boundaries:[]},claims:[...nodes.map(n=>({claimId:n.nodeId,statement:'Synthetic declaration claim for '+n.nodeId})),...edges.map(e=>({claimId:e.edgeId,statement:'Synthetic relation claim for '+e.edgeId,freshness:'CURRENT'}))],sources:Object.fromEntries(nodes.map(n=>[n.nodeId,{before:preview(n.nodeId,'before'),after:preview(n.nodeId,'after')}])),files:[],counts:{files:0,declarations:2,omittedFiles:0,omittedDeclarations:0},coverage:{status:'BOUNDED',compilations:[':/main',':/test'],beforeAnalysis:'SYNTAX',afterAnalysis:'SYNTAX',comparability:'COMPARABLE',obligations:[]},comparisonId:'synthetic-comparison',baseRevision:'synthetic-base',beforeSnapshot:'synthetic-before',afterSnapshot:'synthetic-after'};
}
function load(data=fixture(),width=1200){
 const all=[];
 class Element{
  constructor(tag,id=''){this.tagName=tag;this.id=id;this.children=[];this.dataset={};this.attributes={};this.listeners={};this.className='';this.textContent='';this.disabled=false;this._value='';this._selected='';all.push(this);this.classList={contains:name=>this.className.split(/\s+/).includes(name),add:name=>{if(!this.classList.contains(name))this.className=(this.className+' '+name).trim()},remove:name=>{this.className=this.className.split(/\s+/).filter(c=>c!==name).join(' ')}};}
  get options(){return this.children.filter(child=>child.tagName==='option');}
  get value(){return this.tagName==='select'?(this.options.find(option=>option.value===this._selected)?.value||''):this._value;}
  set value(value){if(this.tagName==='select')this._selected=String(value);else this._value=String(value);}
  get selectedOptions(){return this.options.filter(option=>option.value===this.value);}
  append(...children){for(const child of children){this.children.push(child);child.parentElement=this;if(this.tagName==='select'&&this.options.length===1)this._selected=child.value;}}
  replaceChildren(...children){this.children=[];this._selected='';this.append(...children);}
  setAttribute(name,value){this.attributes[name]=String(value);if(name==='class')this.className=String(value);if(name==='data-id')this.dataset.id=String(value);}
  getAttribute(name){return this.attributes[name];}
  getComputedTextLength(){return Array.from(this.textContent).reduce((width,char)=>width+(char==='W'?12:7),0);}
  addEventListener(type,listener){(this.listeners[type]??=[]).push(listener);}
  dispatch(type,extra={}){const event={type,target:this,defaultPrevented:false,preventDefault(){this.defaultPrevented=true},...extra};for(const listener of this.listeners[type]||[])listener(event);return event;}
  scrollIntoView(){this.scrolled=true;}
 }
 const roots=new Map([...template.matchAll(/\bid="([^"]+)"/g)].map(match=>[match[1],new Element(['nodeSelect','edgeSelect','fileSelect'].includes(match[1])?'select':'div',match[1])]));
 roots.get('change-data').textContent=JSON.stringify(data);
 const document={getElementById:id=>roots.get(id),createElement:tag=>new Element(tag),createElementNS:(_,tag)=>new Element(tag),querySelectorAll:query=>all.filter(e=>query==='.selected'?e.classList.contains('selected'):query==='[data-id]'&&e.dataset.id!==undefined)};
 const context=vm.createContext({document,window:{innerWidth:width},URL,Blob,setTimeout});
 vm.runInContext(script,context);
 return {data,retained:()=>vm.runInContext('JSON.stringify(data)',context),e:id=>roots.get(id),filter:query=>{roots.get('nodeSearch').value=query;roots.get('nodeSearch').dispatch('input');},choose:(select,id)=>{roots.get(select).value=id;roots.get(select).dispatch('change');},graph:id=>all.find(e=>e.dataset.id===id),selected:()=>all.filter(e=>e.classList.contains('selected')).map(e=>e.dataset.id)};
}
if(process.env.CODECLEW_WORKING_TREE_READER_FIXTURE_OUT){
 const output=path.resolve(process.env.CODECLEW_WORKING_TREE_READER_FIXTURE_OUT);fs.mkdirSync(path.dirname(output),{recursive:true});
 const payload=JSON.stringify(fixture()).replace(/</g,'\\u003c').replace(/\u2028/g,'\\u2028').replace(/\u2029/g,'\\u2029');
 fs.writeFileSync(output,template.replace('__CODECLEW_CHANGE_DATA__',()=>payload));
}
test('filter changes selection and source details together, preserves a match, and clears empty/reset states',()=>{
 const r=load(),original=r.retained();
 r.choose('nodeSelect','main-consumer');
 r.filter('PriceTest');
 assert.equal(r.e('nodeSelect').value,'price-test');
 assert.equal(r.e('selectionTitle').textContent,'PriceTest.priceCheck · :/test');
 assert.match(r.e('claimText').textContent,/price-test/);
 assert.match(r.e('beforeCode').textContent,/price-test/);
 assert.match(r.e('afterCode').textContent,/price-test/);
 assert.deepEqual(r.selected(),['price-test']);
 r.filter('');r.choose('nodeSelect','test-price');r.filter('Pricing');
 assert.equal(r.e('nodeSelect').value,'test-price');
 assert.equal(r.e('selectionTitle').textContent,'Pricing.price · :/test');
 r.filter(':/test');
 assert.deepEqual(r.e('nodeSelect').options.filter(o=>o.value).map(o=>o.value),['test-price','test-consumer','price-test']);
 assert.equal(r.e('nodeSelect').value,'test-price');
 r.filter('does-not-exist');
 assert.equal(r.e('nodeSelect').value,'');assert.equal(r.e('nodeSelect').disabled,true);
 assert.equal(r.e('selectionTitle').textContent,'No matching declarations');
 for(const id of ['beforeCode','afterCode','beforeAnchor','afterAnchor','claimJson','selectionMeta'])assert.equal(r.e(id).textContent,'',id);
 assert.deepEqual(r.selected(),[]);
 r.filter('');
 assert.equal(r.e('nodeSelect').disabled,false);assert.equal(r.e('nodeSelect').value,'main-price');
 assert.equal(r.e('selectionTitle').textContent,'Pricing.price · :/main');
 assert.equal(r.retained(),original,'filtering must not mutate retained evidence');
});
test('main/test declarations and relations have distinct visible and accessible identities',()=>{
 const r=load(),options=r.e('nodeSelect').options.filter(o=>o.value);
 assert.equal(options.length,r.data.graph.nodes.length);assert.equal(new Set(options.map(o=>o.textContent)).size,options.length);
 for(const id of ['main-price','test-price','main-consumer','test-consumer']){
  const compilation=id.startsWith('main')?':/main':':/test',graph=r.graph(id);
  assert.ok(options.find(o=>o.value===id).textContent.includes(compilation));
  assert.ok(graph.getAttribute('aria-label').includes(compilation));
  assert.ok(graph.children.find(e=>e.tagName==='text'&&e.textContent.includes(compilation)));
 }
 const relations=r.e('edgeSelect').options.filter(o=>o.value);
 assert.equal(new Set(relations.map(o=>o.textContent)).size,relations.length);
 for(const id of ['main-call','test-call']){
  const compilation=id==='main-call'?':/main':':/test';
  assert.equal(relations.find(o=>o.value===id).textContent.split(compilation).length,3);
  assert.equal(r.graph(id).getAttribute('aria-label').split(compilation).length,3);
 }
 r.graph('test-call').dispatch('click');
 assert.equal(r.e('edgeSelect').value,'test-call');assert.equal(r.e('nodeSelect').value,'');
 assert.equal(r.e('selectionTitle').textContent,'Pricing.labelConsumer · :/test → Pricing.price · :/test');
 assert.match(r.e('beforeCode').textContent,/test-consumer/);assert.deepEqual(r.selected(),['test-call']);
 r.choose('edgeSelect','');assert.equal(r.e('claimJson').textContent,'');assert.deepEqual(r.selected(),[]);
});
test('graph selection outside a filter restores a matching dropdown and resets relation selection',()=>{
 const r=load();r.filter('PriceTest');r.graph('main-consumer').dispatch('keydown',{key:'Enter'});
 assert.equal(r.e('nodeSearch').value,'');assert.equal(r.e('nodeSelect').value,'main-consumer');
 assert.equal(r.e('selectionTitle').textContent,'Pricing.labelConsumer · :/main');
 r.choose('edgeSelect','test-call');r.filter('PriceTest');
 assert.equal(r.e('nodeSelect').value,'price-test');assert.equal(r.e('edgeSelect').value,'');
 assert.equal(r.e('selectionTitle').textContent,'PriceTest.priceCheck · :/test');
});
test('a retained graph with no declarations exposes no stale source or claim selection',()=>{
 const data=fixture();data.graph.nodes=[];data.graph.edges=[];data.graph.candidates=[];data.claims=[];data.sources={};
 const r=load(data);assert.equal(r.e('nodeSelect').disabled,true);assert.equal(r.e('nodeSelect').value,'');
 assert.equal(r.e('selectionTitle').textContent,'No retained declarations');
 assert.equal(r.e('beforeCode').textContent,'');assert.equal(r.e('claimJson').textContent,'');
 assert.match(r.e('verification').textContent,/not a proof of no impact/);
});

test('Enter and Space activate graph buttons like click and reveal mobile details',()=>{
 const state=r=>({node:r.e('nodeSelect').value,edge:r.e('edgeSelect').value,query:r.e('nodeSearch').value,title:r.e('selectionTitle').textContent,before:r.e('beforeCode').textContent,after:r.e('afterCode').textContent,claim:r.e('claimJson').textContent,selected:r.selected()});
 for(const id of ['main-consumer','test-call'])for(const key of ['Enter',' ']){
  const clicked=load(fixture(),600),keyboard=load(fixture(),600);clicked.filter('PriceTest');keyboard.filter('PriceTest');
  clicked.graph(id).dispatch('click');const event=keyboard.graph(id).dispatch('keydown',{key});
  assert.equal(event.defaultPrevented,true);assert.deepEqual(state(keyboard),state(clicked));
  assert.equal(keyboard.e('selectionTitle').scrolled,true);
 }
 const r=load(),before=state(r),event=r.graph('test-call').dispatch('keydown',{key:'ArrowDown'});
 assert.equal(event.defaultPrevented,false);assert.deepEqual(state(r),before);
});
test('long compilation labels fit the graph while full retained identity remains available',()=>{
 const compilation='sln:fixtures/csharp-project-basic/Orders.slnx',data=fixture();
 data.graph.nodes[0].before.compilation=compilation;data.graph.nodes[0].after.compilation=compilation;
 const r=load(data),original=r.retained(),group=r.graph('main-price'),texts=group.children.filter(e=>e.tagName==='text'),compact=texts[1];
 assert.ok(compact.textContent.endsWith('…'));assert.ok(compact.getComputedTextLength()<=210);
 assert.ok(group.children.find(e=>e.tagName==='title').textContent.includes(compilation));
 assert.ok(group.getAttribute('aria-label').includes(compilation));
 assert.ok(r.e('nodeSelect').options.find(o=>o.value==='main-price').textContent.includes(compilation));
 r.choose('nodeSelect','main-price');assert.equal(r.e('selectionTitle').textContent,'Pricing.price · '+compilation);
 assert.equal(r.retained(),original,'display compaction must not alter retained identity or evidence');
 // Width, rather than character count, bounds even unusually wide labels.
 const wide=fixture();wide.graph.nodes[0].before.compilation='W'.repeat(24);wide.graph.nodes[0].after.compilation='W'.repeat(24);
 const wideGraph=load(wide).graph('main-price'),wideText=wideGraph.children.filter(e=>e.tagName==='text')[1];
 assert.ok(wideText.getComputedTextLength()<=210);assert.ok(wideText.textContent.endsWith('…'));
 const short=load();for(const id of ['main-price','test-price'])assert.ok(short.graph(id).children.filter(e=>e.tagName==='text')[1].textContent.includes(id==='main-price'?':/main':':/test'));
});
